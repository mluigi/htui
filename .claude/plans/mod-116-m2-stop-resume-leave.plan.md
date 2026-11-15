# Plan: MOD-116 M2 — Stop, resume and leaving

**Item**: MOD-116 — Workspaces tab and workspace lifecycle
**Source PRD**: `.claude/prds/mod-116-workspaces-tab.prd.md`
**Selected Milestone**: 2 — Stop, resume and leaving
**Routing**: routed as PRD by `/handoff-run` (C2, C4); PRD gate answered (Q1-Q4, 2026-11-15). Sandbox run
`hr/MOD-116`, plan written at `db97562a` while the M1 review fix round (R1, brief
`.claude/plans/mod-116-m1-workspaces-tab.briefs/R1.md`) was editing `ui/tabs/workspaces/mod.rs`, `app/*`, `lib.rs`,
`state_file.rs`, `header.rs`, `testkit.rs`, `keys/hint.rs`: **line anchors in those files are approximate** (cite by
symbol); the fact-check runs after R1 lands, and T1 starts only after it. **(fact-check)** R1 landed
(`dc559952..f8037cd8`); every anchor below was re-checked at `5457063c` and is exact unless marked `~`.
**Requirements**: `R-ORCH-16` (stop the open workspace too; both stops ask; resumable), `R-TUI-14` (stopping and
resuming from the tab), `R-TUI-18` (the leave dialog lists every live workspace; stop gracefully first)
**Complexity**: Medium (one shared report module moved out of `stop_cmd.rs` with the CLI byte-identical; two
`StoreRequest`/`StoreReply` pairs served by `workspaces.rs`; a question mode and a Runs section on the tab; a stateful
leave dialog; three catalogue rows, one view default, one stack; no migration, no store trait change, no `.sqlx`)
**Status**: COMPLETE (2026-11-15; T1-T3 `266f3896`..`1692b8bb`, review fix round R1 `a3305744`..`5c8926a3`). Fact-checked at `5457063c` (see "Verified claims")

## Summary

M1 shows every workspace's state; M2 makes the tab act on it. On a workspace's rows `s` reads the workspace once
more and asks **"Stop the runs in Graphics?"** in a kit question listing each live run (item key, status and box,
and per running step its phase, agent and what the stop does to it: "stops at its turn boundary; its session is
kept" or "stops at once; runs again on resume"), with the per-run wording `htui stop` prints for queued, parked and
stopped runs. Nothing to stop says **"Nothing is running in Graphics"** and writes nothing. `s` on a workspace
stopping gracefully asks the forced question; `Esc` there leaves the graceful stop going. `r` asks the resume question
(parity with `htui resume`). Every write goes through a new `StoreRequest` served by the store worker over the writer
(offline refused, said in the box), and every write re-reads the stop after the answer and refuses when someone
changed it meanwhile (MOD-125 M4 R1's rule), saying so. The open workspace stays `● open`, annotated `· stopping…` or
`· stopped · n held`. The overview gains a **Runs** section: each held run and how it resumes, and while stopping which
runs stop at their turn boundary and which at once. The key hints follow the state. The **leave dialog** lists every
live workspace (open one first) with running and waiting counts and offers **`S` stop gracefully first**: a graceful
stop on each, a settling line, a bounded wait (20 s, as `htui stop`), then it leaves. While this TUI is the executor
leaving still drops its walks (MOD-125 Q5); the dialog keeps saying work stops with it (MD6). The report, its lines and
its wording move from `stop_cmd.rs` to a shared `stop_report.rs`; `htui stop`/`htui resume` print byte for byte what
they print today.

## Shared decisions

- **D1 — One shared report module, the CLI byte-identical.** `crates/htui/src/stop_report.rs` (new, `pub mod` in
  `lib.rs`) takes from `stop_cmd.rs`, unchanged in behaviour: `STOP_WAIT`, `STOP_POLL`, `caps_map`, `Report`,
  `RunLine`, `RunResume` (with `words`, `resumes`), `RunLine::status_words`, `impl Display for Report`, the store-generic
  `report` body and `stopped_resume`. `stop_cmd.rs` keeps its public API so `tests/it/stop_cli_pg.rs` and every
  `stop_cmd` unit test compile **untouched**: `pub use crate::stop_report::{STOP_POLL, STOP_WAIT, caps_map, Report,
  RunLine, RunResume};`, `Context` unchanged, and `stop_cmd::report(store, ctx, workspace) -> Result<Report,
  StateExit>` a three-line wrapper (`stop_report::report(store, ctx.caps, &HashMap::new(), workspace.workspace_id,
  &workspace.slug).await.map_err(|err| store_exit(&err, &workspace.slug, "read"))`). The questions, `confirm`,
  `settle`, `resolve`, `store_exit`, `emit`, `say` stay in `stop_cmd.rs`; `unchanged` keeps its body but compares
  through `stop_report::same_stop` (D3). Pinned by the existing `stop_cmd` tests (assertions and
  `htui__stop_cmd__tests__report_stopping.snap` unchanged) and `stop_cli_pg::` on Postgres.
- **D2 — What a running run's steps do under a stop (new, shared).** `RunLine` gains `steps: Vec<LiveStep>`: for a
  `running` run, its `run_steps` with `status == Running` in row order (empty for every other status; the CLI never
  prints it). `LiveStep { phase: String, agent: Option<String>, pauses: bool }`: `agent` is the step agent's name from
  the `names` map `report` is given (`None` for a step without an agent or a name the map lacks); `pauses` =
  `step.agent_id` is `Some` and that agent's caps (`caps`, `DriverCaps::default()` when unknown, as `stopped_resume`
  does) have `resume`. Wording (`LiveStep::under(mode)`), `<who>` = `phase · agent` or `phase`:
  - graceful, pauses: `<who>: stops at its turn boundary; its session is kept`
  - graceful, not: `<who>: cannot pause between turns: stops at once; runs again on resume`
  - forced, pauses: `<who>: stops now, mid-turn; redoes that turn on resume`
  - forced, not: `<who>: stops now; runs again on resume`
  A running run with no running step (between steps): `stops before its next step` (graceful) / `stops now`
  (forced). Design source: findings §4.1 stopping bullets; `R-ORCH-16`; the CLI's question ("Each agent step stops at
  its turn boundary, checks stop at once").
- **D3 — The race rule and the writes (shared, store-generic).** In `stop_report.rs`:
  `same_stop(a, b)` compares `(mode, requested_at)` of two `Option<&WorkspaceStop>` (what `stop_cmd::unchanged` does
  today). `write_stop(store, workspace, mode, decided_on, user, at) -> Result<StopWrite, StoreError>` re-reads
  `workspace_stop`; differs from `decided_on` → `Changed` (nothing written); a row that already holds the mode or more
  (graceful over graceful, anything over forced) → `Already(row.mode)`; else `stop_workspace(…)` → `Written`.
  `write_resume(store, workspace, decided_on) -> Result<ResumeWrite, StoreError>`: re-read differs → `Changed`; no
  row → `NotStopped`; else `resume_workspace` (its `Ok(false)` — resumed in between — is still `Resumed`, as the CLI
  says). `at` is taken by the worker when it serves the request, i.e. after the answer (M4 R1). `decided_on` is the
  stop row read by the same overview read the question was built from (D6), or the counts row for the leave dialog
  (D9).
- **D4 — Two store requests, served by `workspaces.rs` through the writer.**
  `StoreRequest::StopWorkspaces { mode: StopMode, targets: Vec<StopTarget> }` (name `"stop_workspaces"`) →
  `StoreReply::StopsWritten { mode, outcomes: Vec<(WorkspaceId, StopWrite)> }` in target order;
  `StoreRequest::ResumeWorkspace { workspace: WorkspaceId, decided_on: Option<WorkspaceStop> }` (name
  `"resume_workspace"`) → `StoreReply::ResumeWritten { workspace, outcome: ResumeWrite }`. `StopTarget { workspace,
  decided_on: Option<WorkspaceStop> }` lives in `stop_report.rs`. One batched stop request because the shell drops a
  reply superseded by a newer request of the same name from the same origin (`App::on_reply`'s staleness gate,
  `update.rs:~541-546`): the leave dialog stops several workspaces in one request, the tab sends one target. Served in
  `workspaces::serve_with_home` (the M1 module, one or-arm in `try_serve` widened to the three requests): no writer →
  `Unreachable(DATABASE_UNREACHABLE)` before any write (`workspace_stop` is not mirrored); the user is
  `backend.this_user()`, the clock `Utc::now()`. A target that fails stops the batch with the error (the targets before
  it are written; the reply is a `Failed`, so the asker re-reads).
- **D5 — The overview carries the report.** `WorkspaceOverview` gains `report: Report` (D1/D2), read in
  `workspaces::overview` with `caps_map` and `agent_names` over `backend.agents()`, the slug from the tree's workspace
  row. The tab's detail gains a **Runs** section after `held runs` (heading `Runs`, `no runs` dim when empty): per
  `RunLine` a line `<item>  <status_words>` and under it, indented, its fate: a running run with a stop row → each
  `LiveStep::under(stop.mode)` (or D2's between-steps words); a running run without one → nothing; every other run →
  `RunResume::words(stopped)` with `stopped` = a stop row exists (the CLI's words: `starts when the workspace is
  resumed`, `its gate can be answered once resumed`, `implement reopens its agent session`, `cannot be resumed: cancel
  it from the Runs pane`, …). No run id (the CLI's 8 hex columns stay CLI-only).
- **D6 — The tab's stop and resume (question mode).** `s` (`workspaces.stop`) and `r` (`workspaces.resume`) on any
  row of a workspace mark a pending ask and force one overview read of that workspace (or wait for the one in
  flight); its reply decides, from `overview.report.stop` and the report's lines:
  - `s`, no stop row, no `queued`/`running`/`awaiting_approval` line → told `Nothing is running in <name>`;
  - `s`, no stop row, live lines → **graceful question**, title `Stop the runs in <name>?`, first line `Each agent
    step stops at its turn boundary, checks stop at once; nothing new starts until it is resumed.`, then every live
    line with its fate under `Graceful` (held `queued`/parked runs: `RunResume::words(true)`);
  - `s`, graceful row and a running line → **forced question**, title `Stop the runs in <name> now?`, first line
    `Every step still running is killed mid-turn and redoes its turn on resume.`, then the running lines under
    `Forced`; `Esc`/`n` writes nothing (the graceful stop goes on);
  - `s`, graceful row and no running line → told `Nothing is running in <name>: its graceful stop has settled`;
    `s`, forced row → told `<name> is already stopped (forced)`. (The CLI's `--force` over a settled graceful stop
    still escalates; the tab does not, findings §4.1 "s on a stopping workspace".)
  - `r`, no stop row → told `<name> is not stopped`; else **resume question**, title `Resume <name>?`, first line `Its
    stopped runs resume on the boxes that ran them and its queued runs can start.` (the CLI's), then every line with
    `RunResume::words(true)`.
  `confirm.yes` sends `StopWorkspaces { mode, targets: [StopTarget { workspace, decided_on: report.stop }] }` or
  `ResumeWorkspace { …, decided_on }` and returns to browsing; `confirm.no` closes. On `StopsWritten`/`ResumeWritten`:
  `Written`/`Resumed` → nothing told, `WorkspaceRunCounts` and a forced overview read are asked (the states move at
  once); `Already(m)` → told `<name> is already stopped (<m>)`; `NotStopped` → told `<name> is not stopped`; `Changed`
  → told `The stop of <name> changed while asking; nothing was stopped (resumed). Press s (r) again.` A `Failed` for
  `stop_workspaces`/`resume_workspace`/the pending overview → told `Stopping needs the database: <message>` (resp.
  `Resuming`), absorbed (`absorbs_failure`). A told box is the same kit question with one answer (`confirm.no`
  "close"). Moving the cursor to another workspace drops a pending ask. Keys: stack `views::WORKSPACES_CONFIRM` =
  `[Layer::view(Context::Workspaces, &[]), CONFIRM, MODAL]` (as `RUNS_CANCEL`, `views.rs:~1001`); `confirm.yes`
  answers `y` and, through a `VIEW_DEFAULTS` row `(Context::Workspaces, Act::ConfirmYes, &["enter"])`, `Enter`
  (findings §4.1: "Enter confirms, Esc cancels"); `confirm.no` `n`/`Esc`. Every other chord: `modal_rest`.
- **D7 — The open workspace's annotation (display only; `WorkspaceLifecycle` unchanged).** Wherever the tab draws a
  state (`overview::state_spans`), an `Open` state whose counts row has a stop row gets a dim suffix: running > 0 →
  ` · stopping…`; else ` · stopped` plus ` · <held> held` when `held() > 0`. So `● open · 2 running · stopping…`,
  `● open · stopped · 3 held`. No stop row: as M1. Offline (R1 F6: no states for the others), the open one keeps M1's
  `● open` without annotation. **(fact-check, orchestrator: M1 R1's remaining low finding)** The detail pane obeys
  the same rule, which M1 left out: `render_detail` builds its state with `WorkspaceLifecycle::of(open,
  &overview.counts)` (`ui/tabs/workspaces/mod.rs:496`) and never checks `ctx.top_bar.offline()`, so offline a
  non-open workspace's detail still shows the last overview's state as current. M2: offline, a non-open workspace's
  detail shows **no state line** and the open one's reads `● open` (as `lifecycle()`, `mod.rs:562-571`), with no D7
  annotation on either; the **Runs** section (D5) is left out, heading included, for every workspace (its fates are
  the last read's, not current); and D8's hint row is chosen from no counts row (M1's `o open · …`). Online, the
  detail's state and annotation read `overview.counts` (the overview read's own row), as today.
- **D8 — Hints follow the state.** The hint row (list pane, detail pane, `key_hint`) is chosen per the selected
  workspace's counts row: no stop row and `live() > 0` → `o open · s stop · j/k move · Enter fold`; a graceful row and
  `running > 0` → `o open · s force stop · r resume · …`; any other stop row → `o open · r resume · …`; no stop row and
  nothing live, or no counts → M1's `o open · …`. While a question is up `key_hint` is `None` (the box carries its
  keys), as `RunsTab::key_hint`. **(fact-check)** Offline the row is taken as absent (`counts_of(..)` filtered by
  `!ctx.top_bar.offline()`, as `lifecycle()` does), so the hints fall back to M1's.
- **D9 — The leave dialog (R-TUI-18).** `LeaveDialog` becomes stateful (`new()` keeps its signature). It asks for
  `StoreRequest::Workspaces` when pushed (names; its reply is addressed to it). Rows, under the unchanged `LIVE`
  sentence: the open workspace's row first, as today (`TopBarState::waiting`, shown when it is live, an edit is open
  or its counts are unknown); then one row per other workspace whose counts row reads `Background` or `Stopping`
  (`WorkspaceLifecycle::of(false, row)`), in the `Workspaces` reply's order: `◐ <running> working · ⚠
  <awaiting_approval> waiting[ · <queued> queued]   <name>[  ◌ stopping…]` (other workspaces' "waiting" counts their
  runs parked at a gate: the counts read carries no question rows; documented in the module doc). Before names land,
  those rows fold into one `◐ <running> working in <n> other workspaces`. `workspace_runs: None` → no other rows (the
  four existing snapshots do not move). **`S` (`leave.stop`) "stop gracefully first"** is offered (chip and a line
  `S stops them gracefully first, waits up to 20 s for their runs to stop, then leaves; each resumes later.`) only
  when the counts are known and some row (the open one included) has `live() > 0` without a stop row or `running > 0`
  with one. `S` sends one `StopWorkspaces { Graceful, targets }` (every row with `live() > 0` and no stop row,
  `decided_on: None`) — or, with none to write, goes straight to settling — and the box reads `Stopping <n>
  workspaces gracefully, then leaving…` / `waiting for <r> running runs to stop (<s> s left)`, chips `Enter leave now
  · Esc stay (the stops stand)`. The waited set: targets whose outcome is `Written`/`Already` plus rows already
  stopping; `Changed` ones are left as they are and counted in the line (`<k> changed meanwhile, left as they are`).
  Settling: on each `refresh` (once a second, `App::refresh_top_overlay`) the dialog asks `WorkspaceRunCounts` itself
  (so the answer is newer than the write); it leaves (`Action::Quit`) on the first answer where every waited row has
  `running == 0` or no stop row, or after `SETTLE_TICKS = STOP_WAIT.as_secs() / STOP_POLL.as_secs()` (20) refreshes.
  `leave.confirm` (`Enter`, second `ctrl-c`) leaves at once in every stage; `leave.stay` closes the dialog in every
  stage (written stops stand). A refused write (offline, `Failed`) shows `Could not stop: <message>` with `S` offered
  again (an overlay's refusal is also the status line's, `App::absorbed`). After the stops settle the in-process
  executor's walks are already `stopped`; leaving still runs `RunRuntime::shutdown` (MOD-125 Q5), and the `LIVE`
  sentence keeps promising nothing about work surviving (MD6). The module doc's "until MOD-116/MOD-125" paragraph
  is rewritten.
- **D10 — Keys** (every key a catalogue row; `tests/it/no_hardcoded_keys.rs`). New acts appended after
  `Act::ActivityFilter` (`position` 251) in the tests' `ALL` list (`catalogue.rs:3091`) and `position` (`:3349`):
  `WorkspacesStop` 252, `WorkspacesResume` 253 (T2), `LeaveStop` 254 (T3). **(fact-check)** Not so in the `Act` enum:
  there `ActivityFilter` (`:894`) is followed by 34 later variants up to `KindsEditor` (`:964`); the enum derives no
  `Ord`, so its order is free: declare `WorkspacesStop`/`WorkspacesResume` after `WorkspacesOpen` (`:614`) and
  `LeaveStop` after `LeaveStay` (`:829`), each with a doc line. **(fact-check)** `every_act_has_exactly_one_row` pins
  `CATALOGUE.len() == 252` (`:3387`): 254 after T2, 255 after T3. Rows in their context's block:
  `row(Act::WorkspacesStop, Workspaces, "stop", &["s"], "stop runs")` and `row(Act::WorkspacesResume, Workspaces,
  "resume", &["r"], "resume runs")` after `WorkspacesOpen` (`catalogue.rs:1801-1809`); `row(Act::LeaveStop, Leave,
  "stop", &["S"], "stop gracefully first")` after `LeaveStay` (`:2504`), no `.since(2)` (new rows, as M1's).
  `WORKSPACES_BROWSE`'s view layer lists the two new acts; `views::LEAVE`'s view layer lists `LeaveStop`.
  New stack `views::WORKSPACES_CONFIRM` (D6) with its `views.rs` stack-list entry beside `WORKSPACES_BROWSE`
  (`:1519`) and a `DECLARED` entry `("in a Workspaces question", views::WORKSPACES_CONFIRM)` **(fact-check) appended
  at the end**, after `("in the activity panel", views::ACTIVITY_BROWSE)` (`DECLARED[101]`), so it is
  `DECLARED[102]`: inserting it after `("in Workspaces", …)` (`stack.rs:277`) would move the pinned `DECLARED[83]`
  (leave dialog), `[84]` (Logs) and `[101]` (activity panel) and break the file's "appended so every first-reported
  phrase is unchanged" rule. `DECLARED.len()` 102 → 103 (`:482`), plus a `DECLARED[102]` pin with a `MOD-116 M2 D6`
  comment. No context is added, so no table list moves (`keys_file.rs`, `load.rs` `TABLES`). `docs/htui-keys.md` regenerated with `cargo run -q -p htui -- --default-keys
  --print-keys` (pinned by `keys_file::reference_page`).
- **D11 — Snapshots are reviewed, never blindly accepted** (M1 D13): each task runs `cargo insta test -p htui
  --all-features` then `cargo insta pending-snapshots` and checks every `.snap.new` against what its section says
  moves; anything else is a regression to fix.
- **D12 — Not in M2.** Forms, Hierarchy changes, `T` (M3); stopping one run; a post-exit "htui keeps running in the
  background: n run(s)" line (findings §3.6; no requirement asks for it); changing `htui stop`'s behaviour (it still
  writes a stop on an idle workspace and escalates a settled graceful stop).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| The report and its wording | `crates/htui/src/stop_cmd.rs:91-237` (`Report`, `RunLine`, `RunResume::words`, `Display`), `:467-561` (`report`, `stopped_resume`), `:642-667` (`unchanged`), tests `:721-1709` | moved verbatim; the CLI's tests are the parity pin |
| Resume forecast | `crates/htui-orch/src/promote.rs:148-186` (`StepResume`, `stopped_step_resume`); `DriverCaps.resume` `crates/htui-agent/src/driver.rs:367-394` | caps default when the agent is unknown (`stop_cmd.rs:547-551`) |
| Worker-assembled module, writer or `Unreachable` | `crates/htui/src/workspaces.rs:91-121` (`serve`, `serve_with_home`), `:123-` (`overview`); or-arm `store_worker.rs:~2752`; `this_user` in a worker write `hierarchy.rs:291` | one module, one or-arm; identity filled in the worker (`R-NF-3`) |
| Request / reply / name | `store_worker.rs:476-486` (`WorkspaceRunCounts`, `WorkspaceOverview`), names `:1464-1465`, replies `:1937-1947`, `name_arms_are_stable` `:6865` (fact-check: was `~6862`) | one variant, one name arm, one serve arm |
| Kit question in a tab | `ui/tabs/backlog/detail/runs.rs:~803-817` (`Mode::ConfirmCancel` through the mode's stack, `modal_rest`), `:~2476-2512` (`question`), `:~2252-2263` (render `Ask` over the content); `ui/kit/modal.rs:171` (`Ask`), `:439` (`modal_rest`) | the modal carries the keys; the pane draws none while it is up |
| Confirm stack + view default | `keys/views.rs:1001-1002` (`RUNS_CANCEL`), `:73` (`CONFIRM`), `:61` (`MODAL`); `catalogue.rs:2583-2641` (`VIEW_DEFAULTS`, Migration's `Y`/`N` `:2616-2617`; `Context::ALL` puts `Workspaces` just before `Migration`, so the new row goes before the `// migration_prompt.rs:92` comment at `:2615`) | view layer with no acts, `CONFIRM`, `MODAL` |
| Leave dialog | `ui/overlay/leave.rs` (`lines`, `counts_row`, `on_key`, the `Bench` tests); `Overlay::refresh` `ui/overlay/registry.rs` (`refresh`); `App::on_tick` `app/update.rs:~379-403` (`refresh_top_overlay` once a second) | answers resolve through `views::LEAVE`; chips name the chords in force |
| Tab reads and replies | `ui/tabs/workspaces/mod.rs` (`read_if_due`, `on_reply`, `absorbs_failure`, `HINT`), `overview.rs:30-38` (`state_spans`), `:112-200` (`workspace_lines`; fact-check: was `:108`) | one overview read in flight; refusals said in the pane |
| Harness | `testkit.rs:559` (`with_leave_dialog`), `:609` (`settle`), `:317` (`drive`), `:661` (`settle_refusing(request, message)`) (fact-check: R1 moved them by 3 from `:556`/`:606`/`:314`); `App::update(Action::Tick)` × 4 = one refresh (`TICKS_PER_REFRESH`, `update.rs:29`); `tests/it/workspaces_tab.rs`'s `open_over` (`:32`), `refresh` (`:43`), `demo` (`:89`, folder rows on the demo box) | no sleeps |

## Tasks

Order: **T1 → T2 → T3**, one serial lane, starting after M1 R1 has landed. T1 is the foundation. T2 and T3
both touch `keys/catalogue.rs`, `keys/views.rs`, `docs/htui-keys.md` and `README.md`. **(fact-check)** T1 and T2
share no file (T2 edits `ui/tabs/workspaces/`, not `src/workspaces.rs`); T2 builds on T1's `Report`, requests and
replies, so it stays after T1. Every task leaves `cargo check --workspace --all-targets --all-features` green and runs only its
crate-scoped checks. Integration tests are modules of `htui`'s one `it` binary (TOOL-15): run them with
`--all-features`. Grep every test log for `SIGABRT`. A hook blocks shell `grep` on bare symbol names: use the Gortex
tools for Rust source, and add `\b` (or filter with `python3 -I`) when grepping logs.

### T1: The shared report module and the stop/resume requests (foundation)

- **Files**:
  - `crates/htui/src/stop_report.rs` (CREATE), `crates/htui/src/lib.rs` (`pub mod stop_report;` beside `pub mod
    stop_cmd;`)
  - `crates/htui/src/stop_cmd.rs` (D1: moves out, re-exports, the `report` wrapper, `unchanged` via `same_stop`;
    its tests module untouched)
  - `crates/htui/src/workspaces.rs` (`WorkspaceOverview.report`; serve the two writes; unit tests)
  - `crates/htui/src/store_worker.rs` (two requests, two replies, two name arms, the or-arm, `name_arms_are_stable`)
  - Not expected to change: `crates/htui/tests/it/stop_cli_pg.rs`, `src/snapshots/htui__stop_cmd__tests__report_stopping.snap`,
    every `it__workspaces_tab__*` snapshot (no rendering changes in T1).
- **Types and signatures** (`stop_report.rs`; doc comments name MOD-116 M2 D1-D4):
  ```rust
  pub const STOP_WAIT: Duration;  pub const STOP_POLL: Duration;          // moved, values unchanged
  pub fn caps_map(agents: Vec<AgentSummary>) -> HashMap<AgentId, DriverCaps>;        // moved
  pub fn agent_names(agents: &[AgentSummary]) -> HashMap<AgentId, String>;           // new: summary.agent.name
  pub struct Report { pub slug: String, pub state: WorkspaceStopState,
                      pub stop: Option<WorkspaceStop>, pub lines: Vec<RunLine> }      // moved
  pub struct RunLine { pub item: String, pub run: RunId, pub status: RunStatus,
                       pub on_box: Option<String>, pub resume: RunResume,
                       pub steps: Vec<LiveStep> }                                      // + steps (D2)
  pub enum RunResume { Stopping, Held, AtGate, Steps(Vec<(String, StepResume)>), Continues, NoBox } // moved
  impl RunResume { pub fn words(&self, stopped: bool) -> String; pub(crate) const fn resumes(&self) -> bool; }
  impl RunLine { pub fn status_words(&self) -> String;
                 /// D2/D5: what `mode` does to this run: one line per live step, the between-steps words,
                 /// or `self.resume.words(true)` for a run that is not running.
                 pub fn under(&self, mode: StopMode) -> Vec<String>; }
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct LiveStep { pub phase: String, pub agent: Option<String>, pub pauses: bool }
  impl LiveStep { pub fn under(&self, mode: StopMode) -> String; }                    // D2's four sentences
  impl core::fmt::Display for Report                                                    // moved verbatim
  pub async fn report<S: ReadStore + WriteStore>(store: &S, caps: &HashMap<AgentId, DriverCaps>,
      names: &HashMap<AgentId, String>, workspace: WorkspaceId, slug: &str) -> Result<Report, StoreError>;
  pub fn same_stop(a: Option<&WorkspaceStop>, b: Option<&WorkspaceStop>) -> bool;      // (mode, requested_at)
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct StopTarget { pub workspace: WorkspaceId, pub decided_on: Option<WorkspaceStop> }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum StopWrite { Written, Already(StopMode), Changed }
  #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum ResumeWrite { Resumed, NotStopped, Changed }
  pub async fn write_stop<S: WriteStore>(store: &S, workspace: WorkspaceId, mode: StopMode,
      decided_on: Option<&WorkspaceStop>, user: UserId, at: DateTime<Utc>) -> Result<StopWrite, StoreError>;
  pub async fn write_resume<S: WriteStore>(store: &S, workspace: WorkspaceId,
      decided_on: Option<&WorkspaceStop>) -> Result<ResumeWrite, StoreError>;
  ```
  `report` is today's body with `ctx.caps` → `caps`, `workspace.workspace_id`/`.slug` → the arguments, `StateExit` →
  `StoreError`, plus, for a `Running` run, `steps` from `run_steps(run)` filtered to `StepStatus::Running` (D2).
  `stop_cmd.rs` keeps `Ask`, `Context` (field `caps: &HashMap<AgentId, DriverCaps>` unchanged), `run_stop`,
  `run_resume`, `stop_on`, `resume_on`, `report` (wrapper, D1), the four question fns, `confirm`, `settle`,
  `unchanged`, `resolve`, `store_exit`, `emit`, `say`; its `use` list shrinks to what remains.
  ```rust
  // store_worker.rs (D4)
  StoreRequest::StopWorkspaces { mode: StopMode, targets: Vec<StopTarget> },          // "stop_workspaces"
  StoreRequest::ResumeWorkspace { workspace: WorkspaceId, decided_on: Option<WorkspaceStop> }, // "resume_workspace"
  StoreReply::StopsWritten { mode: StopMode, outcomes: Vec<(WorkspaceId, StopWrite)> },
  StoreReply::ResumeWritten { workspace: WorkspaceId, outcome: ResumeWrite },
  // try_serve: the WorkspaceOverview or-arm becomes
  StoreRequest::WorkspaceOverview(..) | StoreRequest::StopWorkspaces { .. }
      | StoreRequest::ResumeWorkspace { .. } => workspaces::serve(backend, request).await?,
  // workspaces.rs
  pub struct WorkspaceOverview { …, /// D5: the workspace's stop report (`stop_report::report`).
                                 pub report: Report }
  ```
  `serve_with_home`: the "not this module's request" guard admits the three; the two writes take the writer (or
  `Unreachable(DATABASE_UNREACHABLE)`), `backend.this_user()` and `Utc::now()`, and loop the targets through
  `write_stop` / call `write_resume` (D3, D4). `overview` reads `backend.agents()` once, builds `caps_map` and
  `agent_names`, and calls `stop_report::report(&writer, …, workspace, &tree.workspace.slug)` (field name of the
  slug on the tree's workspace row: confirm in `hierarchy::HierarchySnapshot`).
- **Tests first**:
  - `stop_report.rs` unit tests (MemStore demo, helpers copied from `stop_cmd.rs`'s `Fixture::claimed` /
    `running_step` / `stop_run`, `:908-1028`):
    `same_stop_compares_mode_and_instant` (None/None equal; graceful vs forced differ; same mode, other instant
    differ; `requested_by` ignored);
    `the_live_step_words_follow_the_mode_and_the_caps` (the four D2 sentences, with and without an agent name, and
    the between-steps words through `RunLine::under`);
    `a_running_run_lists_its_running_steps` (a claimed run in Graphics with a running `implement` step on
    `ids::AGENT_CLAUDE_CLI` and a second running step with `agent_id: None`: `steps == [LiveStep { phase:
    "implement", agent: Some(<that agent's name from store.agents()>), pauses: <caps_for(that agent).resume> },
    LiveStep { phase, agent: None, pauses: false }]`; a queued and a stopped line have no steps);
    `write_stop_writes_once_never_downgrades_and_refuses_a_changed_stop` (None → `Written` with user and instant;
    graceful again → `Already(Graceful)`; forced over graceful with `decided_on` = that row → `Written`, row forced;
    graceful over forced → `Already(Forced)`; a `decided_on` that no longer matches → `Changed` and the row unchanged);
    `write_resume_deletes_only_the_stop_it_was_decided_on` (`Resumed`; `NotStopped` without a row; `Changed` when the
    row was replaced meanwhile, which stays).
  - `stop_cmd.rs`: no test edits. `cargo test -p htui --all-features --lib stop_cmd::` green with
    `report_stopping.snap` unchanged is the parity pin; plus one new test there,
    `the_cli_report_text_is_the_shared_display` (the `stop_cmd::report` wrapper's `to_string()` equals
    `stop_report::report(…, &HashMap::new(), …)`'s over the same fixture).
  - `workspaces.rs`: `the_overview_carries_the_stop_report` (Platform: three lines in `(queued_at, id)` order with
    `Held`, `Stopping`, `AtGate` for `RUN_2`, `RUN_4`, `RUN_5`; `report.stop == None`; Graphics: one `NoBox` line,
    `RUN_8`; fact-check: demo `queued_at` (2,8), (2,9), (2,10), `RUN_8` has no executing box); `a_stop_request_writes_each_target_by_this_user` (`StopWorkspaces { Graceful, [Platform, Graphics] }`
    → `StopsWritten` with two `Written`, both rows `requested_by == this_user()`); `a_stop_over_a_changed_stop_writes_nothing`
    (`decided_on: None` over an existing row → `Changed`); `a_resume_request_resumes_the_stop_it_was_decided_on`;
    `offline_stop_and_resume_are_refused_before_any_write` (as `offline_the_overview_is_refused`).
  - `store_worker.rs` `name_arms_are_stable`: `"stop_workspaces"`, `"resume_workspace"`.
- **Check**:
  ```bash
  # (fact-check) cargo test takes one positional filter ("unexpected argument" otherwise): the rest go after `--`
  cargo test -p htui --all-features --lib -- stop_report:: stop_cmd:: workspaces:: store_worker::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^stop_cli_pg::/) or test(/^workspaces_tab::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # expected: none (D11)
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```
  (`stop_cli_pg::` returns early with `testkit::SKIP` without `HTUI_TEST_DATABASE_URL`; run it with the sandbox's test
  DSN when one is set, else it is owed on the host.)

### T2: Stop and resume on the Workspaces tab, the Runs section, the annotation and the hints (after T1)

- **Files**:
  - `crates/htui/src/ui/tabs/workspaces/mod.rs` (the ask/question/told state, keys, replies, stacks, hints;
    **(fact-check)** `render_detail`'s offline rule, D7), `crates/htui/src/ui/tabs/workspaces/overview.rs` (Runs
    section D5, `state_spans` annotation D7; **(fact-check)** `workspace_lines` takes an optional state and whether
    to draw the Runs section, D7's offline rule),
    `crates/htui/src/ui/tabs/workspaces/question.rs` (CREATE: the question's lines from a `Report`, pure)
  - `crates/htui/src/keys/catalogue.rs` (two acts, two rows, `ALL`, `position`, one `VIEW_DEFAULTS` row),
    `crates/htui/src/keys/views.rs` (`WORKSPACES_BROWSE` lists the acts; `WORKSPACES_CONFIRM`; stack list; resolve
    tests), `crates/htui/src/keys/stack.rs` (`DECLARED` + len), `crates/htui/src/keys/hint.rs` (the
    `WORKSPACES_BROWSE` help-lines pin, R1 F7: `"Workspaces: o open workspace · s stop runs · r resume runs"`)
  - `crates/htui/src/ui/about.rs` (Workspaces actions gain `(WorkspacesStop, "stop")`, `(WorkspacesResume,
    "resume")` if `every_main_tab_has_a_one_sentence_purpose_and_actions` allows; else name why in the commit)
  - `crates/htui/tests/it/workspaces_tab.rs`, `crates/htui/tests/it/snapshots/it__workspaces_tab__*.snap`
  - `docs/htui-keys.md` (regenerated), `README.md` (the **1 Workspaces** row, `:~189`: "`o` opens one, `s` stops
    its runs, `r` resumes them"), `docs/htui-cli.md` (`:1-4`: one sentence: the Workspaces tab's `s`/`r` do the same
    from the TUI)
- **Types and signatures** (`ui/tabs/workspaces/mod.rs`, private):
  ```rust
  enum Asked { Stop, Resume }                                    // which key is waiting on a fresh overview
  enum Question {                                                // D6; drawn as a kit `Ask` over the tab body
      Stop { workspace: WorkspaceId, mode: StopMode, decided_on: Option<WorkspaceStop>, lines: Vec<Line<'static>> },
      Resume { workspace: WorkspaceId, decided_on: Option<WorkspaceStop>, lines: Vec<Line<'static>> },
      Told { title: String, text: String },
  }
  // WorkspacesTab gains: pending: Option<(WorkspaceId, Asked)>, question: Option<Question>
  ```
  `question.rs`: `pub(super) fn stop_lines(report: &Report, mode: StopMode, theme: &Theme) -> Vec<Line<'static>>`
  (the live lines, D6), `pub(super) fn resume_lines(report: &Report, theme: &Theme) -> Vec<Line<'static>>`, and
  `pub(super) fn run_lines(line: &RunLine, fate: &[String], theme: &Theme) -> Vec<Line<'static>>` (`<item>
  <status_words>` then indented fates), shared with `overview::workspace_lines`' Runs section. Constants for every
  sentence in D6 (titles, first lines, told texts).
  Behaviour:
  - `on_key`: with a `question`, resolve through `views::WORKSPACES_CONFIRM` as `RunsTab::modal_key` does
    (`ConfirmYes` on `Stop`/`Resume` → request (D6) and close; `ConfirmNo` → close; anything else `modal_rest`). Else
    `WORKSPACES_BROWSE`: `WorkspacesStop`/`WorkspacesResume` on a row whose workspace is listed → `pending = Some((ws,
    asked))` and a forced overview read (a variant of `read_if_due` that ignores `read_against`, or waits for the read
    in flight of the same workspace). A list motion onto another workspace clears `pending`.
  - `on_reply`: a `WorkspaceOverview` for the pending workspace builds the `Question` (D6) and clears `pending`;
    `StopsWritten`/`ResumeWritten` per D6 (on success request `StoreRequest::WorkspaceRunCounts` and a forced
    overview read); `Failed` for `"stop_workspaces"`, `"resume_workspace"`, and for `"workspace_overview"` while a
    pending ask exists → `Told` (D6). `absorbs_failure` adds `"stop_workspaces"` and `"resume_workspace"`.
  - `key_stack`: `WORKSPACES_CONFIRM` while a question is up, else `WORKSPACES_BROWSE`; `browse_stack` overridden to
    `WORKSPACES_BROWSE`. `key_hint`: `None` while a question is up; else D8's spec.
  - `render`: as today, then, with a question, `Ask::new(title).width(72).hint(&keys.hint(WORKSPACES_CONFIRM,
    spec))` over the body area with its lines (the `Told` box: its text, hint `confirm.no` "close"); the stop
    question's hint `Hint::One(ConfirmYes, "stop")`, `Hint::One(ConfirmNo, "cancel")` (forced: "keep the graceful
    stop"); resume `("resume", "cancel")`.
  - `overview::state_spans(state, counts: Option<&WorkspaceRunCounts>, theme)` gains the counts row for D7 (both
    callers pass it); `workspace_lines` appends the Runs section (D5) after `held runs`.
  - **(fact-check, orchestrator)** D7's offline rule in `render_detail` (`mod.rs:493-498` today): with
    `ctx.top_bar.offline()`, a non-open workspace gets no state (`workspace_lines(overview, None, …)`: no state line,
    the overview starts at `folder on this box`), the open one `Some(Open { running: 0 })` with `counts: None` (no
    annotation), and neither draws the Runs section; `group_row` already passes `None` counts offline through
    `counts_of(..).filter(!offline)`. `hint_for` is handed the same filtered row (D8). Online: `Some(&overview.counts)`
    for the detail, the top bar's row for the list, as specified above.
  - D8: `HINT` becomes four consts (`HINT_IDLE`, `HINT_STOP`, `HINT_FORCE`, `HINT_RESUME`) chosen by a `hint_for(row:
    Option<&WorkspaceRunCounts>)` helper; labels `"open"`, `"stop"`, `"force stop"`, `"resume"`, then M1's move/fold.
- **Keys** (D10, T2's half): `Act::WorkspacesStop` (252), `Act::WorkspacesResume` (253) appended to `ALL` and
  `position` (declared after `WorkspacesOpen` in the enum, D10 fact-check), `CATALOGUE.len()` pin 252 → 254
  (`catalogue.rs:3387`); rows after `WorkspacesOpen`; `VIEW_DEFAULTS` gains `(Context::Workspaces, Act::ConfirmYes,
  &["enter"])` at the `[workspaces]` context's place in `Context::ALL` order, with a comment citing
  `ui/tabs/workspaces/mod.rs` `on_key`; `WORKSPACES_BROWSE`'s view layer `&[WorkspacesOpen, WorkspacesStop,
  WorkspacesResume]`; `WORKSPACES_CONFIRM` (D6) + stack-list entry + `DECLARED` appended at the end (`[102]`, len
  103; D10 fact-check). If the loader's shadowing or
  clash validation objects to `enter` on `confirm.yes` under `WORKSPACES_CONFIRM` (it should not: no other layer there
  binds `enter`), report it rather than adding a `SHADOWING` entry.
- **Tests first**:
  - `views.rs`: `resolve(WORKSPACES_BROWSE, "s") == [WorkspacesStop]`, `"r"` → `[WorkspacesResume]`;
    `resolve(WORKSPACES_CONFIRM, "y")`/`"enter"` → `[ConfirmYes]`, `"esc"`/`"n"` → `[ConfirmNo]`, `"j"` → `[]`.
  - `overview.rs` unit tests (pure, over a hand-built `WorkspaceOverview`/`Report`): `the_open_state_carries_its_stop`
    (D7's two suffixes, none without a stop row); `the_runs_section_says_how_each_run_resumes` (a held queued run,
    a box-less stopped run, a `Steps` run, words as the CLI's); `while_stopping_the_runs_say_when_they_stop`
    (graceful row: one pausing and one non-pausing live step → the two D2 graceful sentences; forced row → the
    forced ones).
  - `crates/htui/tests/it/workspaces_tab.rs` (**(fact-check)** the module's own `open_over(demo().await, 100, 30)`
    (`Harness::over` a demo with folder rows on the demo box, so no frame prints this machine's home directory, which
    `Harness::demo()` would) and `refresh(&mut harness)` after a direct store write; demo: Graphics open with
    box-less `RUN_8`; Platform `RUN_2` queued, `RUN_4` running, `RUN_5` parked; cursor onto Platform with `j`s):
    `s_asks_to_stop_the_runs_listing_each` (snapshot `stop_question_100`; every live line of `stop_report::report`
    over the same store appears with its item key and its `under(Graceful)` words: the parity metric);
    `confirming_writes_a_graceful_stop_and_the_state_moves` (`y` → `workspace_stop(platform)` graceful by the demo
    user; after `settle` Platform reads `◌ stopping…`; `Enter` answers the same on a fresh store);
    `esc_on_the_stop_question_writes_nothing`;
    `s_on_a_stopping_workspace_asks_to_force_and_esc_keeps_the_graceful_stop` (snapshot `force_question_100`; `esc` →
    row still graceful; `s`, `y` → forced);
    `nothing_running_says_so_and_writes_nothing` (`s` on Graphics → snapshot `nothing_running_100`, no row);
    `a_forced_workspace_is_already_stopped`;
    `r_asks_then_resumes` (after a stop: snapshot `resume_question_100`, `y` → no row; Platform `◐ background`);
    `r_on_a_workspace_not_stopped_says_so`;
    `a_stop_changed_while_asking_is_refused_and_said` (open the question, write a forced stop to the store directly,
    `y` → the told box names the change; the row is the other one);
    `offline_the_stop_says_it_needs_the_database` (harness over an offline backend, or a store whose overview read is
    refused: told `Stopping needs the database: …`, nothing on the status line);
    `the_open_workspace_stops_too_and_reads_open_stopping` (stop Graphics after claiming a run there, or `o` onto
    Platform first: `● open · 1 running · stopping…`, snapshot `open_stopping_100`);
    `the_overview_lists_the_runs_and_how_each_resumes` (snapshot `overview_runs_100`, Platform under a graceful stop);
    `the_hints_follow_the_state` (the hint row reads `s stop`, then `s force stop · r resume`, then `r resume`, and
    on idle Graphics neither);
    **(fact-check, orchestrator)** `offline_a_non_open_workspace_s_detail_shows_no_state` (as M1's
    `offline_the_last_counts_are_not_shown_as_current`, `:605`: cursor `j`-ed onto Platform's group row and its
    overview read online, Platform under a graceful stop so the Runs section and the D7/D8 words show; then
    `harness.app().top_bar.store = "offline · 0s"`: the detail's rows hold no `◐`/`◌`/`○`/`background`/`stopping`,
    no `Runs` heading, and the hint row reads M1's `o open`, without `r resume`; back to `"memory"`, the state, the
    Runs section and `r resume` return) and `offline_the_open_workspace_s_detail_reads_open_only` (a graceful stop
    written on Graphics, `refresh`, cursor left on Graphics: the detail reads `● open · stopped · 1 held` with its
    Runs section; offline: `● open` with no annotation and no `Runs` heading). Do not move the cursor while offline:
    a move asks for an overview, which offline is refused and replaces the held one with "needs the database".
- **Snapshots** (D11): new `it__workspaces_tab__{stop_question,force_question,nothing_running,resume_question,
  open_stopping,overview_runs}_100`; moved: `overview_100` (the Runs section; the hint row where the cursor sits on
  a live workspace), and `list_100`/`list_80`/`project_100` and R1's new tab snapshots **only** in the hint row or the
  Runs section. Any full-shell snapshot that moves outside those two is a regression.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- ui::tabs::workspaces:: keys:: ui::about::   # (fact-check) filters after `--`
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_tab::/) or test(/^keys_file::/) or test(/^keys::/) or test(/^no_hardcoded_keys::/) or test(/^shell::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new (D11)
  cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T3: The leave dialog lists every live workspace and stops them first (after T2)

- **Files**:
  - `crates/htui/src/ui/overlay/leave.rs` (stateful dialog, D9; module doc; unit tests and its `Bench`),
    `crates/htui/src/ui/overlay/snapshots/htui__ui__overlay__leave__tests__*.snap` (new ones; the four existing must
    not move)
  - `crates/htui/src/keys/catalogue.rs` (`Act::LeaveStop` 254, its row, `ALL`, `position`),
    `crates/htui/src/keys/views.rs` (`LEAVE`'s view layer lists `LeaveStop`; resolve test)
  - `crates/htui/tests/it/leave_dialog.rs` (CREATE), `crates/htui/tests/it/main.rs` (`mod leave_dialog;`),
    `crates/htui/tests/it/snapshots/it__leave_dialog__*.snap` (CREATE)
  - `docs/htui-keys.md` (regenerated), `docs/htui-editor.md` (§ "The leave dialog", `:~212-230`: `S` and the settling
    box; the example block keeps today's text, which still holds with no other live workspace), `README.md`
    (`:~203`, the `q` row: "lists every live workspace; `Enter` leaves, `S` stops them gracefully first, `Esc` stays")
- **Types and signatures** (`leave.rs`):
  ```rust
  #[derive(Debug, Default)]
  pub struct LeaveDialog { names: Option<Vec<WorkspaceSummary>>, stage: Stage }
  #[derive(Debug, Default)]
  enum Stage {
      #[default] Asking,
      /// `S` sent the stop request; nothing answered yet.
      Writing { waited: Vec<WorkspaceId> },
      /// D9: waiting for `waited` to settle; `ticks` refreshes so far; `changed` targets left as they were;
      /// `running` the last count read.
      Settling { waited: Vec<WorkspaceId>, changed: usize, ticks: u64, running: u32 },
      /// The write was refused; `S` is offered again.
      Refused(String),
  }
  const SETTLE_TICKS: u64 = STOP_WAIT.as_secs() / STOP_POLL.as_secs();   // 20 (crate::stop_report)
  ```
  `new()` unchanged. `wants_requests` → `vec![StoreRequest::Workspaces]` (if an `app::pane`/`state` test pins that the
  leave dialog asks nothing when pushed, update that pin and name it in the commit). `on_key` over `views::LEAVE`:
  `LeaveConfirm` → `Action::Quit` in every stage; `LeaveStay` → `Close` in every stage; `LeaveStop` only in `Asking`/
  `Refused` when offered (D9): targets from `ctx.top_bar.workspace_runs` (every row with `live() > 0` and `stop:
  None`, `decided_on: None`), waited = targets plus rows with a stop and `running > 0`; with targets, request
  `StoreRequest::StopWorkspaces { mode: Graceful, targets }` → `Writing`; without, request `WorkspaceRunCounts` →
  `Settling`. Not offered → `Pass` (the modal swallows it). `on_reply`: `Workspaces(list)` → `names`;
  `StopsWritten { outcomes, .. }` → `Settling` (waited minus `Changed` ones, `changed` counted) and request
  `WorkspaceRunCounts`; `Failed` for `"stop_workspaces"` → `Refused(message)`; `WorkspaceRunCounts(rows)` while
  `Settling` → `running` = the waited rows' running sum; every waited row `running == 0` or `stop: None` (or gone) →
  `Action::Quit`. `refresh` while `Settling`: `ticks += 1`; `ticks >= SETTLE_TICKS` → `Action::Quit`; else request
  `WorkspaceRunCounts`. `render`/`lines` per D9 (rows, the `S` line, the settling and refused texts, chips through
  `keys.label(views::LEAVE, …)`).
- **Keys** (D10, T3's half): `Act::LeaveStop` 254 (declared after `LeaveStay` in the enum; `CATALOGUE.len()` pin
  254 → 255, D10 fact-check); `row(Act::LeaveStop, Leave, "stop", &["S"], "stop gracefully first")` after
  `LeaveStay`; `LEAVE = [Layer::view(Context::Leave, &[LeaveConfirm, LeaveStay, LeaveStop]), HELP]`.
  The loader's must-keep set does not gain it (leaving stays possible without it).
- **Tests first**:
  - **(fact-check)** Today `LeaveDialog` is a unit struct (`leave.rs:68`) whose `lines(ctx)` (`:85`) and
    `counts_row(ctx, counts)` (`:131`) take no `self`; two unit tests call `LeaveDialog::lines(&bench.ctx())` (`:436`, `:508`)
    (`without_a_workspace_name_the_counts_row_ends_at_the_counts`, `the_waiting_count_is_a_warning_only_when_someone_waits`)
    and `Bench::terminal` (`:332-337`) always draws a fresh `LeaveDialog::new()`. A stateful `lines(&self, ctx)` edits
    those two call sites to `LeaveDialog::new().lines(..)` (their assertions unchanged), and `Bench` gains a
    `render_of(&LeaveDialog)` (the existing `render()` keeps drawing `LeaveDialog::new()`, so the four snapshot tests
    stay untouched).
  - `leave.rs` unit tests (extend `Bench` with `workspace_runs` rows and a `names` reply):
    `every_live_workspace_has_a_row_the_open_one_first` (Graphics open counting (1, 0); Platform background (1
    running, 1 parked, 1 queued); Ops stopping (1 running, stop row); Idle (no runs) and Held (stop row, 0 running)
    absent → rows in that order with D9's words; snapshot `the_box_with_every_live_workspace`);
    `before_the_names_land_the_others_fold_into_one_row`;
    `stop_first_is_offered_only_when_something_can_stop` (no counts → no `S` chip and `S` passes; only stopped rows →
    none; a background row → `S  stop first` chip and the line; snapshot `the_box_offering_stop_first`);
    `s_writes_a_graceful_stop_for_every_unstopped_live_workspace` (the emitted request's targets: open and background
    ones, `decided_on: None`, not the stopping one);
    `settling_leaves_once_every_waited_workspace_has_no_running_run` (`StopsWritten` → a `WorkspaceRunCounts` request
    emitted; a counts reply with running left → no `Quit`, the line says `waiting for 2 running runs to stop`; one
    with none → `Quit`; snapshot `the_box_settling`);
    `settling_gives_up_after_the_bound` (20 `refresh` calls → `Quit` on the 20th, not before);
    `a_changed_target_is_left_as_it_is_and_said`; `a_refused_stop_says_so_and_offers_s_again`;
    `enter_and_ctrl_c_leave_at_once_and_esc_stays_in_every_stage`;
    `the_four_existing_snapshots_do_not_move` (no new code path is taken without counts: the existing four
    snapshot tests, unchanged, are the pin; do not edit them).
  - `views.rs`: `resolve(LEAVE, "S") == [LeaveStop]`, `"s"` → `[]`.
  - `crates/htui/tests/it/leave_dialog.rs` (`#![cfg(feature = "testkit")]`; `Harness::demo().size(100,
    30).with_leave_dialog()`; a helper ticks one refresh: four `app().update(Action::Tick)` then `settle`):
    `the_leave_dialog_lists_graphics_and_platform` (press `q` after the first counts read; snapshot `two_workspaces`);
    `s_stops_every_live_workspace_then_leaves_when_they_settle` (`q`, `S`: `workspace_stop(platform)` graceful;
    not quit while `RUN_4` runs; `finish_run(RUN_4, Done)` (as M1's `a_stopping_and_a_stopped_workspace_read_so`) and
    one refresh → `app().should_quit`);
    `s_leaves_after_the_bound_when_a_run_never_stops` (20 refreshes → quit; the stop row stays);
    `a_second_ctrl_c_while_settling_leaves_at_once`; `esc_while_settling_stays_and_the_stop_stands`.
- **Snapshots** (D11): new `htui__ui__overlay__leave__tests__{the_box_with_every_live_workspace,
  the_box_offering_stop_first,the_box_settling}` and `it__leave_dialog__two_workspaces`; nothing else moves (the four
  existing leave snapshots in particular).
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- ui::overlay::leave:: keys:: app::   # (fact-check) filters after `--`
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^leave_dialog::/) or test(/^keys_file::/) or test(/^keys::/) or test(/^no_hardcoded_keys::/) or test(/^shell::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new (D11)
  cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

## Validation (after T3, the merged tree)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh          # grep the log for SIGABRT
cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

A manual run in the sandbox (`cargo run -p htui -- --demo`): `w`, `j` onto Platform, `s` lists `RUN_2`, `RUN_4`,
`RUN_5` with their fates; `Enter` marks it `◌ stopping…`; `s` again asks the forced question, `Esc` keeps it graceful;
`r`, `y` returns it to `◐ background`; `q` lists Graphics and Platform and offers `S`.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The move changes `htui stop`'s output | Low | D1: `Display` moved verbatim; the `stop_cmd` tests and `report_stopping.snap` untouched; one equality test of the wrapper |
| The question box overflows with many runs | Medium | kit `Ask` cuts the question with `…` and keeps the hint (`fit_rows`); width 72; the overview's Runs section scrolls |
| The overview's extra reads (agents, steps per running run, events per stopped step) slow it on a large workspace | Low | read per event, one in flight (M1 D7); the CLI already pays the same reads |
| A stale preview: runs move between the read and the answer | Medium | the race rule guards the stop row (D3); run moves are what the stop applies to anyway; the box says "changed" when the row moved |
| The leave wait is mistaken for a hang | Low | the settling line counts the running runs and the seconds left; `Enter`/second `ctrl-c` leave at once |
| Leaving after a stop is still read as "work survives" | Low | `LIVE` unchanged (MD6); the `S` line says the runs resume later, not that they keep running |
| T2/T3 collide with a parallel item on `catalogue.rs`/`views.rs` | Medium | acts appended at the end, rows in their own blocks; positions named here |

## Acceptance

- [ ] T1-T3 complete; `htui stop`/`htui resume` print byte for byte as before (`stop_cmd::`, `stop_cli_pg::`)
- [ ] `s`, `s` again and `r` on the tab ask first, list each run with what happens to it, and write through the store
  worker; nothing to stop says so and writes nothing; a stop changed while asking is refused and said
- [ ] The open workspace stops too and reads `● open · … · stopping…` / `· stopped · n held`
- [ ] The overview lists each held run and how it resumes, and while stopping when each running step stops
- [ ] The hints follow the state; every key is a catalogue row; `docs/htui-keys.md` matches `--print-keys`
- [ ] The leave dialog lists every live workspace (open first) with its counts; `S` stops them gracefully, waits up to
  20 s and leaves; `Enter`/second `ctrl-c` leave at once; `Esc` stays
- [ ] Validation passes; patterns mirrored, not reinvented

## Owed on the host

- **Real use**: stop and resume a workspace with live handoff runs from the tab and compare each line with `htui
  stop <slug>`'s report; leave with `S` while this TUI walks a run and check it resumes on the next start.
- **Postgres**: `stop_cli_pg::` on the merged tree (the shared `report` over `PgStore`), and the gate with
  `HTUI_TEST_DATABASE_URL` set if the sandbox run had none; the live-Qdrant suites (sandbox limits).

## Open questions

None changes M2's scope. Settled here at the recommendation, named so the maintainer can override:

- **OQ1 — Does `r` confirm?** Yes, with `htui resume`'s sentence and the per-run resume lines (parity with the CLI;
  `R-ORCH-16` asks confirmation for both stops only, findings §4.1 is silent on `r`). The alternative, resuming at
  once, is a two-line change in T2.
- **OQ2 — The tab's `s` on an idle workspace writes nothing** ("Nothing is running in <name>", findings §4.1), while
  `htui stop` writes a stop there and escalates a settled graceful stop. The CLI is left as it is (D12).
- **OQ3 — Which keys answer the stop questions.** `y` and `Enter` confirm, `n` and `Esc` cancel (`confirm.yes` with a
  `[workspaces]` view default for `Enter`), so the questions answer like every other htui question and as findings
  §4.1 says.
- **OQ4 — The leave wait's end.** After 20 s the dialog leaves anyway (whatever still runs here is dropped as leaving
  does today; the stop rows stand), mirroring `htui stop`'s bound; it does not keep htui open until every run settles.

## Verified claims

Fact-checked at `5457063c` (M1 R1 landed) by reading the tree through Gortex (`read`/`search`) and Python line
scans, two scratch-crate probes under `/tmp` (cargo's one positional test filter; the `SETTLE_TICKS` const and a
`pub use` re-export seen by a child test module); no SQL claim, so no database. **58 claims: 46 true, 6 false (all
amended), 6 amended without being false** (gaps, exact anchors, the orchestrator's offline finding). The stray
`</content></invoke>` the draft ended with was removed.

| Claim | Verdict | Evidence |
|---|---|---|
| D1: `stop_cmd.rs:91-237` holds `Report`, `RunLine`, `RunResume` (`words`, `resumes`), `status_words`, `Display`; `:467-561` `report` + `stopped_resume`; `:642-667` `unchanged` | true | `Report` doc `:91`/struct `:93`, Display ends `:237`; `report` doc `:467`/fn `:473`, `stopped_resume` `:537-561`; `unchanged` `:646`, key `(mode, requested_at)` |
| D1: `STOP_WAIT` 20 s, `STOP_POLL` 1 s, `caps_map(Vec<AgentSummary>)` | true | `stop_cmd.rs:35`, `:38`, `:84` |
| D1: `stop_cmd::report(store, ctx, workspace) -> Result<Report, StateExit>` kept as a wrapper keeps every caller | true | callers `stop_on`, `resume_on`, `confirm` (`:374-664`) and `stop_cli_pg.rs:235`; errors mapped with `store_exit(.., "read")` today too |
| D1: the `stop_cmd` tests compile untouched with the re-exports | true | tests import `super::{Ask, Context, RunResume, STOP_POLL, STOP_WAIT, StateExit, caps_map, report, resume_on, stop_on, store_exit}` (`:723-726`); no `RunLine {` literal in the tests |
| D1: `htui__stop_cmd__tests__report_stopping.snap` is the parity pin | true | `the_report_says_how_each_run_resumes` `:1437`, snapshot `:1501` |
| D1: `stop_cli_pg.rs` uses `stop_cmd::{Ask, Context, STOP_POLL, STOP_WAIT}`, `caps_map`, `stop_on`, `report`, `resume_on` | true | `stop_cli_pg.rs:15`, `:37`, `:69`, `:235`, `:250` |
| T1: `pub mod stop_cmd;` in `lib.rs` | true | `lib.rs:50` |
| D2: `DriverCaps.resume: bool`; unknown agent reads `DriverCaps::default()`; promote anchors | true | `driver.rs:367`, `:379`; `stop_cmd.rs:546-551`; `promote.rs:148` (`StepResume`), `:174` (`stopped_step_resume`) |
| D2/T1: `summary.agent.name`; `ids::AGENT_CLAUDE_CLI` exists | true | `model/agent.rs:193-195`, `Agent.name` `:35`; `fixtures.rs:154` |
| D3: `stop_workspace` upserts and only escalates graceful → forced; `resume_workspace` `Ok(false)` when not stopped; all on `WriteStore` | true | `htui-core/src/store/traits.rs:1699-1732` |
| D3/D4: `WorkspaceStop` derives `Debug, Clone, PartialEq, Eq` with `mode`, `requested_by`, `requested_at` | true | `model/stop.rs:23-33` |
| D4: the shell drops a reply superseded by a newer request of the same kind from the same origin | true | `App::on_reply` `update.rs:541-547` (`is_fresh`); keyed `(origin, discriminant(request))`, `app/state.rs:1124-1149` (variant = name) |
| D4: `workspaces.rs` `serve` `:91`, `serve_with_home` `:100-121` (guard, writer or `Unreachable(DATABASE_UNREACHABLE)`), `overview` `:123` | true | read at HEAD; module doc: `workspace_stop` not mirrored |
| D4: `backend.this_user()` in a worker write; `backend.agents()` | true | `hierarchy.rs:291` (`.await?`); `store_worker.rs:2607` |
| D4: only `StoreRequest::name` and `try_serve` match the requests exhaustively; both enums derive `Debug, Clone` only | true | `store_worker.rs:160-161`, `:1605-1606`; one `impl StoreRequest` (`:1393`); `observe_reply` has `_ => {}` |
| D4: `store_worker.rs` requests `:476-486`, names `:1464-1465`, replies `:1937-1947`, or-arm `:2752` | true | read at HEAD |
| D5: the slug is on the tree's workspace row | true | `HierarchySnapshot.workspace: Workspace` (`hierarchy.rs:39-41`), `Workspace.slug` (`model/hierarchy.rs:19`): `tree.workspace.slug` |
| D5: `WorkspaceOverview` is built only in `workspaces.rs`; derives `PartialEq` (not `Eq`) | true | `workspaces.rs:26-27`, `:193`; no other `WorkspaceOverview {` literal |
| D5/T1: Platform's lines `RUN_2` Held, `RUN_4` Stopping, `RUN_5` AtGate in `(queued_at, id)` order; Graphics `RUN_8` NoBox | true | `fixtures.rs` runs: (2,8) queued, (2,9) running, (2,10) parked, `RUN_8` stopped with no executing box (`conformance.rs:12506`) |
| D6: `RUNS_CANCEL = [view(BacklogRuns, []), CONFIRM, MODAL]`; `CONFIRM` binds `y` / `n`,`esc`; `MODAL` only CONTROL/ALT/function keys | true | `views.rs:1001-1002`, `:73`, `:61`; `catalogue.rs:1247`, `:1251` |
| D6: kit question pattern | true | `runs.rs:804-818` (`modal_key`), `:2476-2512` (`question`), `:2252-2262` (render), `:2220-2227` (`key_hint` `None` under a question); `modal.rs:171` (`Ask`, `width`, `hint(&str)`), `:439` (`modal_rest`); `Keys::hint` returns `String` (`hint.rs:166`) |
| D6: the CLI's question sentences | true | `stop_cmd.rs:240-269` |
| D6/D10: no layer of `WORKSPACES_BROWSE` binds `s` or `r` | true | `views.rs:1223-1230`; `s`/`r` rows are only in Settings, Backlog Runs/Reqs, Picker, Common (`r` reload), none in List/Pane/Layout/History/Global |
| D7: `state_spans` and `workspace_lines` are `overview.rs`'s only state drawers; `group_row` takes `lifecycle()` | true | `overview.rs:30-38`, `:112-200`; `mod.rs:573-595` |
| D7 (orchestrator): offline, the detail pane still shows a non-open workspace's state | true (amended into D7, T2) | `render_detail` `mod.rs:493-498`: `WorkspaceLifecycle::of(open, &overview.counts)`, no `offline()` check; `lifecycle()` `:562-571` filters it |
| D8: `HINT` const; `Tab::key_hint(width, ctx)` and `browse_stack` exist | true | `mod.rs:64-68`; `ui/tabs/registry.rs:91`, `:100` |
| `TopBarState` after R1: `workspace`, `box_name`, `store`, `waiting`, `workspace_runs`, `background`; `offline()` | true | `app/state.rs:49-73`; `offline()` = `store.starts_with("offline")` |
| D9: the leave dialog's current structure | true | `leave.rs:68` unit struct, `new()`, `wants_requests` empty (`:177`), `on_key` over `views::LEAVE`, `on_reply` no-op; `LEAVE = [view(Leave, [LeaveConfirm, LeaveStay]), HELP]` `views.rs:728-731` |
| D9: the four leave snapshots; the `Bench` has no `workspace_runs` | true | `the_live_box`, `the_box_with_an_open_edit`, `the_box_with_nothing_live`, `the_box_before_the_first_count`; `Bench::new` uses `..TopBarState::default()` |
| D9: `Overlay::refresh` once a second through `App::refresh_top_overlay` | true | `registry.rs:54`; `update.rs:379-403`, `:415`; `TICKS_PER_REFRESH = 4` `:29` |
| D9: a pushed overlay's `wants_requests` go out under its origin; no app test pins the dialog asking nothing | true | `App::push_overlay` `app/state.rs:947-955`; no request-channel assertion in `app/state.rs`/`pane.rs` tests |
| D9: the dialog's own `WorkspaceRunCounts` also refreshes the top bar | true | `observe_reply` sets `workspace_runs` whatever the origin (`update.rs:630-635`) |
| D9: an overlay's refusal is the status line's | true | `App::absorbed`: `Origin::Overlay(_) => false` (`update.rs:592-615`) |
| D9: `SETTLE_TICKS = STOP_WAIT.as_secs() / STOP_POLL.as_secs()` is a const 20 | true | scratch-crate probe compiles and asserts 20 |
| D9: `RunRuntime::shutdown`; the module doc's "until MOD-116/MOD-125" | true | `htui-worker/src/runtime.rs:1779`; `leave.rs:7` |
| D10: `ActivityFilter` is `position` 251; `ALL` and `position` end there | true | `catalogue.rs:3091`, `:3349`; `CATALOGUE.len() == 252` |
| D10: rows `WorkspacesOpen` `:1801-1809`, `LeaveStay` `:2504`; `VIEW_DEFAULTS`, Migration's `Y`/`N` | true (made exact) | `catalogue.rs:1801-1809`, `:2504`, `:2583-2641`, `:2616-2617` |
| D10: stack list `:1519`; `DECLARED` "in Workspaces" `:277`, len 102 pinned `:482` | true | `views.rs:1519`; `stack.rs:277`, `:482` |
| T2: the R1 F7 help-lines pin | true | `hint.rs:750-754` |
| T2: `about.rs` Workspaces actions; the test does not bound them | true | `about.rs:93-96`; `every_main_tab_has_a_one_sentence_purpose_and_actions` `:200-216` |
| T2: `shell::the_help_box_on_the_workspaces_tab` survives `s`/`r` | true | it matches `line.contains("Workspaces: o open workspace")` (`shell.rs:368`), a prefix of the new line: no file to add |
| D10: `docs/htui-keys.md` pinned by `keys_file::reference_page`; regeneration command | true | `tests/it/keys_file.rs:762-783`; one ```` ```toml ```` block (`htui-keys.md:28-427`) |
| T2/T3 docs anchors | true | `README.md:189`, `:203`; `docs/htui-cli.md:1-4`; `docs/htui-editor.md:212-233` |
| T3: `tests/it/main.rs` holds the `mod` list | true | `main.rs:73` `mod workspaces_tab;` |
| T2 snapshots: `list_100`, `list_80`, `overview_100`, `project_100` and R1's `folded_100`; `testkit` `shell_empty` | true | `tests/it/snapshots/`; `shell_empty` has no workspace, so its hint row stays M1's |
| T3: `store.finish_run(RUN_4, Done, None, now)` as M1's case | true | `workspaces_tab.rs:169` |
| D10: new acts appended after `ActivityFilter` "in the `Act` enum" | **false → amended** | the enum continues to `KindsEditor` (`catalogue.rs:894-964`); no `Ord` derive: declared beside `WorkspacesOpen`/`LeaveStay` instead |
| D10: the `DECLARED` entry goes after `("in Workspaces", …)` | **false → amended** | would shift the pinned `DECLARED[83]`/`[84]`/`[101]` (`stack.rs:494`, `:496`, `:484`) and break the append rule (`:333`, `:336`): appended as `[102]` |
| T1-T3 checks: `cargo test … --lib a:: b:: c::` | **false → amended** | probe: `error: unexpected argument 'b::' found`; filters moved after `--` |
| T2 tests: `Harness::demo().size(100, 30).with_tab(…)` + `settle` | **false → amended** | M1's module uses `open_over(demo().await, …)` with folder rows (`workspaces_tab.rs:32`, `:89`); `Harness::demo()` (`testkit.rs:113`) has none and would print the host's home |
| Tasks: "T2 and T1 both touch `workspaces.rs`" | **false → amended** | T2's Files never list `src/workspaces.rs`; T1 and T2 share no file |
| Harness anchors `testkit.rs:556`/`:606`/`:314` | **false → amended** | `:559`, `:609`, `:317` after R1 |
| `CATALOGUE.len()` pin | amended (gap) | `every_act_has_exactly_one_row` asserts 252 (`catalogue.rs:3387`): 254 in T2, 255 in T3 |
| T3: the stateful dialog vs today's `lines(ctx)` and `Bench` | amended (gap) | `lines`/`counts_row` take no `self` (`leave.rs:85`, `:131`); called at `:436`, `:508`; `Bench::terminal` draws `LeaveDialog::new()` (`:332-337`) |
| `name_arms_are_stable` `~6862` | amended (exact) | `store_worker.rs:6865` |
| `workspace_lines` `:108-200` | amended (exact) | `overview.rs:112-200` |
| `VIEW_DEFAULTS` row's place | amended (exact) | `Context::ALL` has `Workspaces` right before `Migration`: before `catalogue.rs:2615` |
| Orchestrator: offline detail, annotations, Runs section, hints | amended into D7, D8, T2 | see the D7 row above; T2 gains two offline cases |
