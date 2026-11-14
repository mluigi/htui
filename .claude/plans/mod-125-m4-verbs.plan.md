# Plan: MOD-125 M4 — The CLI verbs

**Item**: MOD-125 — Stopping and resuming runs
**Source PRD**: `.claude/prds/mod-125-stop-and-resume-runs.prd.md`
**Selected Milestone**: 4 — CLI verbs
**Routing**: routed as plan by `/handoff-run` (0 criteria fired); PRD continuing. Sandbox run `hr/MOD-125`,
plan written at `18df483b`.
**Requirements**: `R-ORCH-16`, `R-TUI-14`, `R-TUI-18`
**Complexity**: Medium (one migration with its pins, one store read on five `WriteStore` implementors, two pure
helpers, a new `htui stop` / `htui resume` module over the existing headless connect, docs)
**Status**: FACT-CHECKED at `18df483b` (see "Verified claims"; amendments marked "fact-check").

## Summary

M1-M3 built everything but the caller: `stop_workspace` writes the gate, the executors settle each run `stopped`
(M2), deleting the row through `resume_workspace` resumes them on their boxes (M3), and every stopped run can be
cancelled. Nothing in the product issues a stop or a resume. M4 adds `htui stop [--force] <workspace>` and
`htui resume <workspace>`: each resolves the workspace by slug over the headless connect `--set-qdrant-url` uses,
shows what it will touch, asks `[y/N]` (or takes `--yes`), writes the request, and prints one line per run of the
workspace saying where it is and how it resumes, under a header saying whether the workspace reads `stopping` or
`stopped`. M4 also ships the owed migration (`0036`, an index for `pending_resumes`; HANDOFF M2 owed (1)), whose
apply raises `htui_target_version` to `0.2.0` on any database still below it, so a `0.1.0` worker, which cannot
apply a stop, is refused there. (Fact-check: `0033`-`0035` already landed on `main` after the `0.2.0` bump, so a
database a `main` build migrated through `0035` already carries `0.2.0`; `0036` makes the rise certain on every
database, it is not the first.)

## Shared decisions

- **D1 — Verb syntax.** Two subcommands of `cli::Command` beside `Status`/`Waiting` (`cli.rs:167-194`):
  `htui stop [--force] [--yes|-y] <WORKSPACE>` and `htui resume [--yes|-y] <WORKSPACE>`. `<WORKSPACE>` is the
  workspace's **slug** (`WorkspaceSummary.slug`, `hierarchy.rs:303-312`), matched exactly as `--project` matches a
  project slug (`state_cmd.rs` `state_target`). No argument reads the environment but the global `--log`
  (pinned like `no_state_argument_reads_the_environment`, `cli.rs:821`); no TUI flag stands beside them
  (`args_conflicts_with_subcommands`, `cli.rs:12`).
- **D2 — Confirmation (PRD gate Q2: a prompt).** No Rust verb prompts today; the convention is `scripts/hr`'s
  `confirm` (`scripts/hr:154-160`): `--yes` answers it, and without a terminal a confirmation needs `--yes`.
  - The preview (D4's report as it stands, before the write) goes to **stdout**, the question to **stderr**, the
    answer is one line of **stdin**. `y` or `yes` (any case, trimmed) confirms; anything else, an empty line and
    EOF decline.
  - Without `--yes` and with stdin not a terminal (`std::io::IsTerminal`, as `lib.rs` `set_qdrant_key` reads it):
    refused **before any write**, `needs confirmation: pass --yes, or run it in a terminal; nothing was stopped`
    (`... nothing was resumed`).
  - Declined: refused, `not confirmed; nothing was stopped` (`... resumed`).
  - Questions (one line each, `<slug>` in backticks):
    graceful `Stop workspace `<slug>` gracefully? Each agent step stops at its turn boundary, checks stop at once;
    nothing new starts until `htui resume <slug>`. [y/N] `;
    forced `Stop workspace `<slug>` now? Every step is killed mid-turn and redoes its turn on resume; nothing new
    starts until `htui resume <slug>`. [y/N] `;
    escalation `Workspace `<slug>` is stopped gracefully; force it? Every step still running is killed mid-turn.
    [y/N] `;
    resume `Resume workspace `<slug>`? Its stopped runs resume on the boxes that ran them and its queued runs can
    start. [y/N] `.
  - A request that writes nothing never asks (D5, D6).
- **D3 — Endings and exit codes: `state_cmd::StateExit`** (`state_cmd.rs:51-72`), which `main` already maps
  (`main.rs` `exit_code`, `reports_to_sentry`): **0** done (written, or nothing to write and the state printed);
  **2** refused (no DSN, keyring, `box.toml`, schema or below-target connect, unknown slug, not confirmed, no
  terminal without `--yes`, a workspace deleted mid-verb: `NotFound`); **3** Postgres did not answer
  (`StoreError::Unreachable`, before or after the connect); **1** any other store failure. The connect is
  `qdrant_cmd::connect` (`qdrant_cmd.rs:205-213`: keyring DSN, `identity::load_or_mint`,
  `PgStore::connect_headless`, which never migrates and refuses below the target) with its `dsn_exit` and
  `failure_exit`, made `pub(crate)` and reused, not copied. `by` is `PgStore::this_user()` (`pg/mod.rs:781`).
- **D4 — The report** (plain text on stdout; no `--json` in M4, see OQ3). A header and one line per run of the
  workspace that is not terminal (T2's `workspace_live_runs`), in `(queued_at, id)` order:

  ```
  workspace graphics: stopped (graceful, requested 2026-11-14 10:02 UTC)
    GFX-3  0193a1b2  stopped on DESKTOP-HTUI   resumes: implement reopens its agent session
    GFX-4  0193a1c4  stopped on build-box      resumes: review starts a fresh session with the handoff
    GFX-5  0193a1d0  stopped, no box           cannot be resumed: cancel it from the Runs pane
    GFX-6  0193a1e9  running on build-box      stopping
    GFX-7  0193a1f3  queued                    starts when the workspace is resumed
    GFX-8  0193a201  awaiting approval         its gate can be answered once resumed
  ```

  - Header state: `WorkspaceStopState` (T2): `not stopped` (no `workspace_stop` row), `stopping` (a row and at
    least one `running` run), `stopped` (a row and no `running` run). Mode and `requested_at` from the row.
  - Item key from `ReadStore::item` (`traits.rs:126`), `-` for a run without an item; the run id's first 8 hex
    digits; the box's hostname from `WriteStore::boxes` (`traits.rs:654`; `BoxRecord.row.hostname`; it lists
    this user's boxes only), the id when no row names it.
  - A stopped run's resume, per stopped step (`run_steps`, `status == Stopped`), from T3's
    `stopped_step_resume`: `<phase> reopens its agent session` / `<phase> starts a fresh session with the
    handoff` / `<phase> runs again from its start`; several stopped steps (parts, candidates) are joined `; `;
    a stopped run with no stopped step: `continues from where it stopped`. Its checks always rerun, said once
    in a footer line (`checks rerun after each resumed step`), not per run.
  - A stopped run with no `executing_box_id` (M3 D6, the demo's `RUN_8`): `cannot be resumed: cancel it from
    the Runs pane`. A box that is offline is not detected (no liveness read): the line names the box, and the
    footer says each box resumes its runs at its next command poll.
  - No runs: `no runs in this workspace`.
- **D5 — `htui stop`.** Read `workspace_stop` first (`traits.rs:1699`):
  - **No row**: preview, ask (graceful or forced question), `stop_workspace(ws, mode, this_user, Utc::now())`
    (`traits.rs:1687`), then wait (D7), then the report.
  - **Graceful row, `--force`**: escalation (the store escalates, `traits.rs:1674-1680`): preview, the
    escalation question, write, wait, report.
  - **A repeat that writes nothing** (graceful over graceful, anything over forced: the store answers the stored
    row and writes nothing, M1 D4): no question, no write; the report, preceded by `workspace <slug> is already
    stopped (<mode>)` and, for a graceful request over a forced row, `a graceful stop does not downgrade a
    forced one`. Exit 0. This is how the state is read again after the wait ran out.
  - A row deleted or escalated by someone else between the read and the write is harmless: the upsert decides
    (M1 D4) and the report re-reads.
- **D6 — `htui resume`.** Read `workspace_stop`:
  - **A row**: preview, ask, `resume_workspace(ws)` (`traits.rs:1708`, `Ok(false)` = someone resumed it first:
    not an error), then the report under `resume requested: each box resumes its stopped runs at its next
    command poll`. No wait: a resume has no settle to wait for, and a box that is down resumes later (M3 D1).
  - **No row**: no question, no write; `workspace <slug> is not stopped` and the report, which still lists any
    stopped run (one its box has not resumed yet, or a box-less one to cancel). Exit 0.
- **D7 — `htui stop` waits, bounded.** After the write it re-reads every `STOP_POLL` = 1 s until the state reads
  `stopped` or `STOP_WAIT` = 20 s passed (graceful settles within `CANCEL_GRACE` 2 s + `STOP_SLACK` 5 s +
  one command poll, PRD metric "Stop takes seconds"; `htui-worker/src/runtime.rs:129,139`), printing nothing meanwhile but
  one stderr line `waiting for <n> running runs to stop…`. If the bound passes it prints the report as it
  stands (state `stopping`) and `still stopping; run htui stop <slug> again to see it settle`, exit 0. Both
  durations are fields of the verb's context so tests shrink them. Ctrl-C during the wait is harmless (the
  request is already durable). See OQ2.
- **D8 — Where the logic lives.** A new module `crates/htui/src/stop_cmd.rs` (beside `state_cmd.rs`,
  `qdrant_cmd.rs`), its core generic over `S: ReadStore + WriteStore` so unit tests run on `MemStore::demo()` and
  one integration module runs it on Postgres. Agent caps come in as a map built by the caller from
  `agents()` (inherent on `MemStore` `mem.rs:615` and `PgStore` `pg/read.rs:1794`) through
  `htui_agent::registry::caps_for` (`registry.rs:152`). So does the workspace list for slug resolution:
  `workspaces()` is inherent too (`mem.rs:569`, `pg/read.rs:1645`; on no `ReadStore`/`WriteStore`, only on
  `WorkerHost`, which `MemStore` does not implement), so the caller reads it and hands it in through the
  context (fact-check amendment). The two pure decisions live with the data they read:
  the workspace state in `htui_core::model::stop` (T2), the per-step resume beside `opening_kind` in
  `htui_orch::promote` (T3), so the verb and the engine's resume (`resume_live_step`,
  `engine.rs:5780-5834`; `stopped_opening`, `:5884-5932`) cannot drift unnoticed (T3's agreement checks).
- **D9 — Migration `0036_stopped_run_index.sql`** (not `0031`: `0031_command_log` is MOD-152's and the tree runs
  through `0035_requirement_keys`). One statement:
  `CREATE INDEX idx_run_stopped_box ON run (executing_box_id) WHERE status = 'stopped';`, the predicate and
  column of `pending_resumes` (`pg/write.rs:8048-8063`), byte-identical SQL unchanged, so no `.sqlx` change for
  it. No table, column or comment: `TABLES` (56) and the mirror's table count stay; no cache migration.
  `cache_meta.schema_version` moves to 36 through `PgStore::schema_version()`, so each box rebuilds its mirror
  once (as every migration since 0022 notes in its header). The header also records why it ships now: M2's owed
  (1) (review H1) asks M4 for a migration because `htui_target_version` rises only in `apply_migrations`
  (`pg/mod.rs:399-404`, run by the TUI only over a pending schema, `store_worker.rs:3472`), so the `0.2.0` bump
  alone refused no `0.1.0` worker. (Fact-check: `0033`, `0034` and `0035` landed on `main` after the bump
  `a05deeac`, so applying any of them with a `main` build already raised the target; `0036` is the migration M4
  owes, not the first since the bump. Say that in the header, not "first migration".) M1's L-6
  (`queue_entry.workspace_id` unindexed) is not folded in; it stays a note.
- **D10 — Owed and not M4.** `labels_between`'s reflog caveat (M3 owed (2): with reflogs off a resume is refused,
  and an unlabelled foreign move a later branch point marks is admitted) stays owed; not trivial, not M4. The
  `htui status` line for a stopped workspace is OQ1. MOD-116 builds the Workspaces tab on the store verbs, not on
  this CLI's output.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Write verb over the headless connect | `qdrant_cmd.rs:41-58` (`set_url`), `:113-128` (`set_url_on`), `:205-213` (`connect`) | `run` = connect, core `_on(&store, …)` answering the line(s), close, print; core tested over `testkit::demo_db()` |
| Endings | `state_cmd::StateExit` (`state_cmd.rs:51-72`), `qdrant_cmd::failure_exit`/`store_exit` (`:228-245`) | offline 3, refused 2, failed 1; `main` maps codes and keeps 2/3 out of Sentry |
| Subcommand wiring | `cli.rs:167-194` (`Command`), `lib.rs` `run` (`Status`/`Waiting`/`Claude` arms after `logging::init`) | one `if let Some(cli::Command::X(..))` arm returning `.map_err(anyhow::Error::from)` |
| CLI parse tests | `cli.rs:742-845` (`status_and_waiting_parse_their_three_flags`, `state_verbs_refuse_tui_flags_beside_them`, `no_state_argument_reads_the_environment`) | `Args::try_parse_from`, env check over `command.find_subcommand` |
| Confirmation | `scripts/hr:154-160` (`confirm`) | `--yes` skips; no terminal and no `--yes` refuses |
| Store read on both stores | `pending_resumes` (`traits.rs:1726-1736`; `mem.rs:11753`; `pg/write.rs:8048`) | trait doc, Mem filter + sort, Pg `query_as!` ordered `(queued_at, id)` |
| Full run row in Pg | `PgStore::run` (`pg/read.rs:756-786`) | the 19-column `query_as!(Run, …)` |
| Store tests | conformance `CASES` + `run_case_inline` arm (`conformance.rs:498`; `run_case` `:482` dispatches to it); `pending_resumes_lists_the_boxs_stopped_runs_once_the_workspace_resumes` (`conformance.rs:9688`) | one generic case, both stores |
| Migration tests | `the_persona_phase_index_exists` (`migrations.rs:1204-1225`), `applying_migrations_raises_the_target_and_never_lowers_it` (`:4494-4556`), `plant_target`/`target` (`:4327-4348`) | `pg_indexes.indexdef` verbatim; target as JSON |
| Pg CLI integration | `crates/htui/tests/it/state_cli_pg.rs` (module of `it`, `#![cfg(feature = "testkit")]`, `testkit::demo_db()`, never the keyring) | seam functions, not the binary |

## Tasks

Order: **(T1 ∥ T2 ∥ T3) → T4.** T1, T2 and T3 own disjoint file sets (intersect the **Files** lists) and can run
in separate worktrees on named branches off `hr/MOD-125`. T4 needs T2's read and state type and T3's helper; it
does not need T1. (Fact-check: the four **Files** sets intersect pairwise empty; only T2 touches `.sqlx/` and moves
the store `CASES` pins, only T4 touches `crates/htui/tests/it/main.rs` and adds snapshots
(`crates/htui/src/snapshots/htui__stop_cmd__*`), no task touches a `Cargo.toml`, and T3 adds no orch case. The one
cross-task reference is text: T2's two `pending_resumes` docs name T1's index.) Every task leaves `cargo check --workspace --all-targets --all-features` green. Conformance
`CASES` 324 → 325 (T2). Integration tests are modules of each crate's one `it` binary (TOOL-15); without
`--all-features` the `it` binary fails on its canary. Never run the workspace gate inside a task.

### T1: Migration `0036` — the `pending_resumes` index and the target rise (independent)

- **Files**:
  - `crates/htui-store/migrations/0036_stopped_run_index.sql` (CREATE)
  - `crates/htui-store/tests/it/migrations.rs`
  - `crates/htui-store/tests/it/connect.rs`
- **Tests first** (`crates/htui-store/tests/it/migrations.rs`):
  - `the_stopped_run_index_exists` (beside `the_persona_phase_index_exists`, `fresh_db`): `pg_indexes.indexdef`
    of `idx_run_stopped_box` equals Postgres's normalised text (expected
    `CREATE INDEX idx_run_stopped_box ON public.run USING btree (executing_box_id) WHERE (status = 'stopped'::text)`;
    fact-checked verbatim on the sandbox's Postgres 18.7; 16 has no server here, but
    `migration_0030_widens_link_kinds_and_backs_one_parent` pins the same `'<x>'::text` shape and
    `the_auto_queue_tables_and_column_exist` the same partial-index shape, both green on 16 before the test DB
    moved to 18, so pin the full text).
  - `migration_0036_raises_a_0_1_0_target_so_an_old_worker_is_refused` (`fresh_db`; review H1): plant the
    target `"0.1.0"` (`plant_target`); roll 0036 back by hand (`DROP INDEX idx_run_stopped_box`,
    `DELETE FROM _sqlx_migrations WHERE version = 36`); a TUI connect (as `connect_reports_pending_on_a_bare_database`
    does it) reports `MigrationState::Pending(1)` and `below_target() == None`; `apply_migrations` raises the
    target to `json!(HTUI_VERSION)`; assert `HTUI_VERSION` parses to a version above `0.1.0` (split on `.`;
    `0.2.0` today), i.e. a `0.1.0` headless process now meets `BelowTarget` (`pg/mod.rs` `headless_checks`
    compares `this_version() < target`); a headless connect at this build still connects. The doc comment says
    why: the target rises only in `apply_migrations`, which the TUI runs only over a pending schema, so a
    database is raised to `0.2.0` only by applying a migration with a `0.2.0` build (`0033`-`0035` did on `main`;
    `0036` does on any database still below).
- **Then** write the migration (D9; header in the `0035_requirement_keys.sql` style: MOD-125 M4, plan D9,
  forward-only R-STO-5, what it serves, the mirror note, the H1 reason, "No table (TABLES stays 56)").
- **Pins moved 35 → 36** (each assertion **and** its message, which names the last migration; "thirty-five" →
  "thirty-six", "through MOD-149's 0035_requirement_keys.sql" → "through MOD-125 M4's 0036_stopped_run_index.sql"):
  - `migrations.rs` `migrations_apply_on_a_clean_database` (`:111-196`): the version vector gains `36`, the
    migration list message gains `MOD-125 M4's 0036_stopped_run_index.sql`. `TABLES.len()` 56 and the table
    message unchanged.
  - `migrations.rs` `connect_reports_pending_on_a_bare_database` (`:4266-4267`).
  - `migrations.rs` `a_headless_connect_never_migrates` (`:4367-4368`, `:4376-4377`, `:4399-4400`).
  - `migrations.rs` `applying_migrations_raises_the_target_and_never_lowers_it` (`:4550-4552`, count of
    `_sqlx_migrations`).
  - `connect.rs` `start_reports_the_pending_count_over_a_bare_database` (`:140-141`, `:154-157`).
  - `connect.rs` `apply_migrations_then_persist_writes_a_minted_id_back` (`:241-243`).
  - Not moved (checked): `read_only_pg.rs` `a_pending_schema_is_refused_read_only` counts `MIGRATOR` itself;
    `store_worker.rs`'s `MigrationsPending(_, 5)` and `connection.rs`'s `3` are synthetic; the
    `it__connection__confirm.snap` snapshot shows the mirror's 23 tables, which an index does not move; no
    cache migration, so no `cache_00NN_applies_over_…` case.
- **Also**: the `TABLES` doc comment (`migrations.rs:24`) says "55 tables"; make it 56 (stale before M4).
- **`.sqlx`**: none (no query macro added or changed).
- **Check**:
  ```bash
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features \
    -E 'binary(it) and (test(/^migrations::/) or test(/^connect::/) or test(/^read_only_pg::/))'
  cargo clippy -p htui-store --all-targets --all-features -- -D warnings
  cargo fmt -p htui-store --check
  ```

### T2: Store read `workspace_live_runs` and the derived workspace state (independent)

- **Files**:
  - `crates/htui-core/src/model/stop.rs`, `crates/htui-core/src/model/mod.rs` (the `pub use stop::{…}` line,
    `:221`)
  - `crates/htui-core/src/store/traits.rs`, `crates/htui-core/src/store/mem.rs`,
    `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/it/mem_store.rs`
  - `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`,
    `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-store/.sqlx/` (one new entry)
  - `crates/htui-agent/src/conformance.rs` (`UsageSpy` forward), `crates/htui-agent/tests/it/recorder.rs`
    (`SpyStore` forward)
- **Types and signatures**:
  - `htui_core::model::stop`:
    ```rust
    /// How a workspace reads (MOD-125 M4 D4; R-TUI-14's "stopping, stopped"): derived from its stop row
    /// and its runs, never stored.
    pub enum WorkspaceStopState { NotStopped, Stopping, Stopped }
    impl WorkspaceStopState {
        /// `NotStopped` without a row; with one, `Stopping` while any run is `Running`, else `Stopped`.
        /// Queued runs are held and parked ones wait at their gate: neither keeps a workspace stopping.
        pub fn of(stop: Option<&WorkspaceStop>, runs: impl IntoIterator<Item = RunStatus>) -> Self;
    }
    impl core::fmt::Display for WorkspaceStopState // "not stopped" | "stopping" | "stopped"
    ```
    re-exported beside `WorkspaceStop` (`model/mod.rs:221`); `WorkspaceStop`'s doc (`stop.rs:20-21`, "derived
    from the workspace's runs (M4)") links it.
  - `WriteStore` (after `resume_workspace`, `traits.rs:1708`):
    ```rust
    /// MOD-125 M4 D4: every run whose `workspace_id` is `workspace` and whose status is not terminal
    /// (`queued`, `running`, `awaiting_approval`, `stopped`; `RunStatus::is_terminal`), in
    /// `(queued_at, id)` order, whole rows as [`ReadStore::run`]. Empty for an unknown workspace (a list
    /// read is total). Runs without a workspace never appear. Postgres serves it from
    /// `idx_run_workspace_status` (`0019_stop_runs.sql`).
    async fn workspace_live_runs(&self, workspace: WorkspaceId) -> Result<Vec<Run>>;
    ```
  - `pending_resumes`' doc (`traits.rs:1726-1733`): "No index serves the read; a scan of the box's runs is
    accepted" → "served by `idx_run_stopped_box` (`0036_stopped_run_index.sql`, M4)". The same sentence on
    `PgStore::pending_resumes` (`pg/write.rs:8040-8042`, "No index serves it (`idx_run_lease` is partial on
    `queued|running`)") changes with it. (T1 owns the SQL; the doc lines are here because this task owns
    `traits.rs` and `pg/write.rs`.)
  - Implementors: MemStore (`mem.rs`, filter `self.runs` on `workspace_id == Some(ws) && !status.is_terminal()`,
    sort `(queued_at, id)`, beside `workspace_stop` `:11740`); PgStore (`pg/write.rs` beside `workspace_stop`
    `:7901`, `query_as!(Run, …)` with `PgStore::run`'s 19 columns, `WHERE workspace_id = $1 AND status IN
    ('queued', 'running', 'awaiting_approval', 'stopped') ORDER BY queued_at, id`); `Writer` (`writer.rs`
    beside `:1135`), `UsageSpy` (`htui-agent/src/conformance.rs` beside `:1252`) and `SpyStore`
    (`htui-agent/tests/it/recorder.rs` beside `:978`) forward. No `WorkerStore` forward (only the CLI reads it).
- **Tests first**:
  - `model/stop.rs` unit tests: `a_workspace_without_a_stop_row_is_not_stopped` (whatever its runs);
    `a_stopped_workspace_with_a_running_run_is_stopping`; `queued_parked_and_stopped_runs_leave_it_stopped`
    (a row and `[Queued, AwaitingApproval, Stopped]` → `Stopped`; no runs → `Stopped`); the `Display` words.
  - Conformance case `workspace_live_runs_lists_the_workspaces_unfinished_runs` (`CASES` entry + `run_case_inline`
    arm, `conformance.rs`): two workspaces; in W a queued, a running, a stopped (reached through
    `stop_workspace` + a lease + `stop_run`, never a bare transition, M3 D1), a `done` and a `cancelled` run; one
    running run in the other workspace; one run with no workspace. Asserts W lists exactly its queued,
    running and stopped runs (plus an `awaiting_approval` one if the case can park one cheaply; otherwise the
    case's doc says the status filter covers it) in `(queued_at, id)` order, whole rows equal to `run(id)`;
    an unknown workspace answers empty.
  - Pins 324 → 325: `crates/htui-core/tests/it/mem_store.rs:43-44` (count and message) and
    `crates/htui-store/tests/it/pg_conformance.rs:66-70` (`EXPECTED_CASES`, its doc list and the message
    `:75-…`), each naming "MOD-125 M4's `workspace_live_runs` case".
- **`.sqlx`**: one new query → `cargo sqlx prepare` from `crates/htui-store` against a migrated scratch DB
  (`docs/hr-sandbox.md:227-236`: `CREATE DATABASE htui_sqlx`, `DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx`,
  `cargo sqlx migrate run --source migrations`, `cargo sqlx prepare -- --all-targets --all-features`); commit
  only the new `.sqlx/query-*.json`. The scratch DB may lack `0036` if T1 has not merged; the new query does
  not touch the index, so its entry is the same either way.
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

### T3: The per-step resume forecast, beside `opening_kind` (independent)

- **Files**:
  - `crates/htui-orch/src/promote.rs`
  - `crates/htui-orch/src/conformance.rs` (assertions added to four existing M3 cases; no new case, orch
    `CASES` count unchanged)
- **Types and signatures** (`htui_orch::promote`, after `opening_kind` `:136`):
  ```rust
  /// MOD-125 M4 D4: how a stopped step will resume (M3 D4), told before the resume runs.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum StepResume {
      /// A banner and `caps.resume`: the step's own session reopens (M3 D4 branch 3).
      Reopen(AgentSessionRef),
      /// Session rows but no banner, or no resume caps: a fresh session sent the handoff (branch 2).
      FreshWithHandoff,
      /// No session rows (branch 1), or a judge (re-run fresh, both calls, M3 D4): the step runs again.
      Rerun,
  }
  /// `step` is a `Stopped` row; `caps` its agent's (`DriverCaps::default()` for an unknown agent, as
  /// `stopped_opening` defaults them, `engine.rs:5895-5901`; the engine reads `step.agent_id`, the row being
  /// authority, `candidate_of` `engine.rs:10613`, and a row with no agent and session rows is an engine
  /// error there, so the forecast's default for `None` is its own choice); `events` its `step_events`.
  /// Judge (`fanout_index == -1`) → `Rerun`; empty `events` → `Rerun`; else `opening_kind(caps, events)`
  /// mapped `Resume(r)` → `Reopen(r)`, `Handoff` → `FreshWithHandoff`.
  #[must_use]
  pub fn stopped_step_resume(step: &RunStep, caps: DriverCaps, events: &[SessionEvent]) -> StepResume;
  ```
  Pure; the engine is **not** changed to call it (its branches are spread over `resume_live_step`,
  `resumed_member` and the judge path); the agreement checks below pin that the two agree.
- **Tests first**:
  - `promote.rs` unit tests: `a_judge_reruns_whatever_its_rows`, `a_step_with_no_rows_reruns`,
    `rows_without_a_banner_start_fresh_with_the_handoff`, `a_banner_without_resume_caps_starts_fresh`,
    `a_banner_with_resume_caps_reopens_that_session` (the banner's id; the latest banner by `seq` after a
    failed-resume fallback, as `banner` reads it). Build `SessionEvent` rows the way the existing
    `opening_kind` tests in this file do.
  - Agreement checks in `conformance.rs`, each computing the forecast from the stopped step's row, its
    `step_events` and the caps the case's agent was registered with, **before** `resume_stopped`, then asserting
    what the resume did:
    `a_stopped_step_resumes_its_own_session_on_its_row` (`:18310`) → `Reopen` with the pre-stop banner's id;
    `a_stopped_step_without_session_caps_opens_fresh_with_the_handoff` (`:18398`) → `FreshWithHandoff`;
    `a_stopped_step_with_no_session_rows_is_walked_again` (`:18450`) → `Rerun`;
    `a_stopped_judge_reruns_fresh_and_reads_the_reruns_verdict` (`:19063`) → `Rerun`;
    and `a_stopped_step_with_no_banner_opens_fresh_on_its_rows` (`:18476`) → `FreshWithHandoff` if it is
    cheap to add. These run on the fake (`fake_conformance`) and any other harness of the dispatcher.
- **Check**:
  ```bash
  cargo test -p htui-orch --all-features --lib promote::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^fake_conformance::/)'
  cargo clippy -p htui-orch --all-targets --all-features -- -D warnings
  cargo clippy -p htui-orch -- -D warnings
  cargo fmt -p htui-orch --check
  ```
  Grep the run for `SIGABRT` (memory: orch stack headroom).

### T4: `htui stop` / `htui resume` and their docs (after T2, T3)

- **Files**:
  - `crates/htui/src/cli.rs` (two `Command` variants, `StopArgs`, `ResumeArgs`, parse tests)
  - `crates/htui/src/stop_cmd.rs` (CREATE: the verbs and their unit tests)
  - `crates/htui/src/lib.rs` (`pub mod stop_cmd;` and two dispatch arms in `run`, after the `Waiting` arm)
  - `crates/htui/src/qdrant_cmd.rs` (`connect`, `dsn_exit`, `failure_exit` → `pub(crate)`; no behaviour change)
  - `crates/htui/tests/it/stop_cli_pg.rs` (CREATE), `crates/htui/tests/it/main.rs` (`mod stop_cli_pg;`)
  - `docs/htui-cli.md`, `CONCEPTS.md`
- **CLI** (`cli.rs`):
  ```rust
  /// Stop every run in a workspace (`R-ORCH-16`), after a confirmation: each agent step stops at its turn
  /// boundary and keeps its session, checks and htui's own steps stop at once; with `--force` every step is
  /// killed mid-turn (a graceful stop already in force is escalated). Nothing new starts in the workspace until
  /// `htui resume`. Waits up to 20 s for the runs to stop, then prints each run and how it will resume.
  Stop(StopArgs),
  /// Resume a stopped workspace, after a confirmation: each stopped run resumes on the box that ran it, at
  /// that box's next command poll; queued runs can start again. Prints each run and how it will resume.
  Resume(ResumeArgs),

  pub struct StopArgs {
      /// The workspace's slug.
      #[arg(value_name = "WORKSPACE")] pub workspace: String,
      /// Kill every step mid-turn instead of stopping at turn boundaries.
      #[arg(long)] pub force: bool,
      /// Answer the confirmation yes; without a terminal it is required.
      #[arg(long, short = 'y')] pub yes: bool,
  }
  pub struct ResumeArgs { /* workspace, yes: as above */ }
  ```
- **`stop_cmd.rs`** (signatures are guidance; names may change, the seams may not):
  ```rust
  pub const STOP_WAIT: Duration = Duration::from_secs(20);
  pub const STOP_POLL: Duration = Duration::from_secs(1);
  /// The confirmation seam (D2).
  pub struct Ask<'a> { pub yes: bool, pub interactive: bool,
                       pub input: &'a mut dyn std::io::BufRead, pub question: &'a mut dyn std::io::Write }
  /// What the cores need that `run_*` read from the process (test seam, as `state_cmd::Context`).
  pub struct Context<'a> { pub user: UserId, pub now: DateTime<Utc>,
                           pub caps: &'a HashMap<AgentId, DriverCaps>,
                           pub workspaces: &'a [WorkspaceSummary],   // inherent `workspaces()`, read by the caller
                           pub wait: Duration, pub poll: Duration }
  pub async fn run_stop(args: cli::StopArgs) -> Result<(), StateExit>;     // connect, stop_on, close
  pub async fn run_resume(args: cli::ResumeArgs) -> Result<(), StateExit>;
  pub async fn stop_on<S: ReadStore + WriteStore>(store: &S, ctx: &Context<'_>, slug: &str, mode: StopMode,
                                                   ask: &mut Ask<'_>, out: &mut dyn Write) -> Result<(), StateExit>;
  pub async fn resume_on<S: ReadStore + WriteStore>(store: &S, ctx: &Context<'_>, slug: &str,
                                                     ask: &mut Ask<'_>, out: &mut dyn Write) -> Result<(), StateExit>;
  /// D4's read, used before and after the write.
  pub async fn report<S: ReadStore + WriteStore>(store: &S, ctx: &Context<'_>, workspace: &WorkspaceSummary)
      -> Result<Report, StateExit>;
  pub struct Report { /* header: slug, WorkspaceStopState, Option<WorkspaceStop>; lines: Vec<RunLine> */ }
  pub enum RunResume { Stopping, Held, AtGate, Steps(Vec<(String, StepResume)>), Continues, NoBox }
  impl core::fmt::Display for Report  // D4's text, exactly
  ```
  `run_stop`/`run_resume`: `qdrant_cmd::connect()`, caps from `pg.agents()` through `caps_for`, `user =
  pg.this_user()`, `interactive = stdin().is_terminal()`, stdout locked for `out`, stderr for the question,
  then `pg.close()` whatever the core answered (as `set_url`). Slug resolution over `ctx.workspaces`, which
  `run_*` fills from the inherent `pg.workspaces()` (tests from `MemStore::workspaces()`); the unknown-slug
  refusal lists the known slugs. Store errors: `Unreachable` → `Offline` (`cannot reach Postgres:
  …`), `NotFound{workspace}` → `Refused` (`workspace <slug> no longer exists`), else `Failed` (`the workspace
  could not be stopped: …` / `resumed` / `read`).
- **Tests first**:
  - `cli.rs` tests: `stop_and_resume_parse_their_flags` (`stop x`, `stop --force -y x`, `resume --yes x`);
    `stop_and_resume_need_a_workspace_and_refuse_tui_flags` (`htui stop`, `htui --demo stop x`,
    `htui resume --force x` fail); `no_stop_or_resume_argument_reads_the_environment` (as `:821`);
    `log_goes_after_stop`.
  - `stop_cmd.rs` unit tests over `MemStore::demo()` plus runs made with the store verbs (stopped runs through
    `stop_workspace` + lease + `stop_run`; never a bare transition), paused time for the wait:
    `a_stop_asks_then_writes_graceful_and_reports_stopped` (no running run: state `stopped` at once, question
    text pinned, `workspace_stop.mode == Graceful`, `requested_by == ctx.user`);
    `force_writes_forced_and_escalates_a_graceful_stop` (escalation question, mode `Forced` after);
    `a_repeat_stop_writes_nothing_and_asks_nothing` (graceful over graceful, graceful over forced with the
    no-downgrade line; the stored `requested_at` unchanged);
    `declining_or_eof_writes_nothing_and_is_refused` (exit 2, `not confirmed; nothing was stopped`, no row);
    `no_terminal_without_yes_is_refused_before_any_write` (and `--yes` without a terminal writes);
    `an_unknown_slug_is_refused_naming_the_workspaces`;
    `a_running_run_keeps_it_stopping_until_the_wait_runs_out` (a leased `running` run nobody settles: report
    says `stopping` and `still stopping; run htui stop <slug> again…`, exit 0);
    `the_report_says_how_each_run_resumes` (one line each: reopen / fresh with handoff / rerun / no box /
    queued / awaiting approval / stopping, pinned with `insta::assert_snapshot!` on `Report`'s text, run ids
    and timestamps redacted as `state_cmd`'s snapshots do);
    `resume_asks_then_deletes_the_row` and `resume_of_a_workspace_not_stopped_asks_nothing_and_lists_stopped_runs`
    (the demo's `graphics` with `RUN_8`: `cannot be resumed: cancel it from the Runs pane`, exit 0);
    `store_failures_map_to_the_state_exits` (`Unreachable` 3, `NotFound` 2, `Backend` 1).
  - `crates/htui/tests/it/stop_cli_pg.rs` (`#![cfg(feature = "testkit")]`, `testkit::demo_db()`, the keyring
    never read; prints `testkit::SKIP` without `HTUI_TEST_DATABASE_URL`):
    `a_stop_and_a_resume_round_trip_on_postgres` (`stop_on` with `--yes` writes the row with the fixture
    user, the report reads `stopped`; `resume_on` deletes it; both through `PgStore`'s
    `workspace_live_runs`, `boxes`, `item` and `agents`); `a_forced_stop_escalates_on_postgres`.
- **Docs** (same task):
  - `docs/htui-cli.md`: the title and intro say the page also covers two writing verbs; a row each in "The
    commands and their flags"; a new section `## Stopping and resuming a workspace` after "Setting the Qdrant
    URL and API key" (D1-D7 in user terms: what graceful and forced do, the question and `--yes`, the report
    lines, the 20 s wait, exit codes, that each box resumes at its next command poll and a box-less stopped
    run must be cancelled); its TOC entry. "No writes" keeps saying "the three commands".
  - `CONCEPTS.md:66-67` (M3 T7 left it for M4; follow `.claude/rules/concept-docs.md`, bump `**Reviewed:**`):
    "Stopping a run saves its conversation and can always be resumed" → it resumes on the box that ran it, and
    a stopped run that box can no longer resume is cancelled instead [R-ORCH-16].
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib cli::tests
  cargo test -p htui --all-features --lib stop_cmd::
  cargo test -p htui --all-features --lib qdrant_cmd::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and test(/^stop_cli_pg::/)'
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  cargo insta test -p htui --all-features --lib --review=never   # new snapshots accepted deliberately, not blindly
  ```

## Validation (after T4, the merged tree)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
bash scripts/gate.sh                        # grep the log for SIGABRT
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

A manual smoke in the sandbox (no keyring there, memory): the verbs refuse with `keyring_unreadable` (exit 2);
the round trip is covered by `stop_cli_pg`. The real round trip on live runs is owed on the host.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The forecast drifts from what the engine does | Medium | T3's agreement checks in the M3 conformance cases |
| A pin missed for the new migration | Medium | T1's list from a search of `Pending(35)`, `thirty-five` and `0035_requirement_keys` across tests; the gate catches the rest |
| `pg_indexes.indexdef` differs between Postgres 16 and 18 | Low | pin 18's text (the test DB); fact-check the shape on 16 |
| The 20 s wait reads as a hang | Low | one stderr line while waiting; bound in the context, OQ2 |
| A `0.1.0` worker keeps running until the TUI applies `0036` | Certain (by design) | the target rises only when someone applies the migration; host note below |
| Prompt read with stdin piped from a script | Low | no terminal and no `--yes` refuses before any write |

## Acceptance

- [ ] T1-T4 complete; conformance 325 cases green on MemStore and PgStore
- [ ] `htui stop x`, `htui stop --force x`, `htui resume x` ask, write and report as D1-D7 say; a repeat writes and asks nothing
- [ ] Applying `0036` raises `htui_target_version` to this build's `0.2.0`
- [ ] Validation passes; patterns mirrored, not reinvented

## Owed on the host

- **After the merge**: reinstall the MCP relay and run `htui claude emit --force` on each box (the `0.2.0` crates
  refuse a `0.1.0` relay, `RELAY_VERSION`; M2 owed (1)). Start one `0.2.0` TUI against the shared database and
  apply `0036` there: that raises the target and refuses every `0.1.0` worker until it is upgraded.
- **Postgres gate on the merged tree** (Postgres 16 dev and 18 test), including `the_stopped_run_index_exists`
  on 16.
- **Live-Qdrant suites** (timeouts in the sandbox; memory).
- **M3's live probes**: `cli_live` cases 12-13 and `isolation_live` `reopen` (`--ignored`, scratch
  `CLAUDE_CONFIG_DIR`).
- **First real use of the verbs** (PRD evidence: real use is an assumption): `htui stop` and `htui stop --force`
  on a workspace with live handoff runs, then `htui resume`; check each report line against what the runs did.
- **Still owed, not M4**: `labels_between`'s reflog caveat (M3 owed (2)); M1 L-6 (`queue_entry.workspace_id`
  unindexed, a note).

## Open questions

**Answered by the maintainer, 2026-11-14:** OQ1 not in M4; OQ2 wait, bounded at 20 s with 1 s polls (D7 as
written); OQ3 not in M4. Fact-check follow-ups, taken at the recommendation: `0036` still ships (the index serves
`pending_resumes` on its own merits and M2 owed (1) asks for it); T1 also fixes the stale "55 tables" in the
`TABLES` doc comment (`migrations.rs:24`) to 56.

- **OQ1 — Should `htui status` show the stop state too?** The PRD's "the workspace's state reads as stopping or
  stopped" is met by the verbs' header and by a repeat `htui stop <slug>` (D5), which asks nothing. Adding a
  field to `status` (`docs/htui-cli.md`, additive, schema `version` stays 1) would let a status bar show it.
  **Recommendation: not in M4**; MOD-116's Workspaces tab and overview lines own the standing display, and
  `status` can gain the field when the Claude Code plugin or a status bar asks for it.
- **OQ2 — Should `htui stop` wait (D7), and how long?** Without a wait the report right after a stop shows every
  live run as `stopping` and says nothing about how they resume. **Recommendation: wait, bounded at 20 s with
  1 s polls**, exit 0 either way; no `--no-wait` flag until someone needs one.
- **OQ3 — `--json` for the two verbs?** `status`/`waiting` have one. **Recommendation: not in M4**; nothing
  consumes it yet, and adding it later is additive.

## Verified claims

Filled by the fact-check stage at `18df483b` (tree read through Gortex; Postgres 18.7 probed on the sandbox's
`localhost:5439` in a scratch database, dropped after; clap 4.6.6 and `IsTerminal` compile-probed in a scratch
crate under `/tmp`, removed after). Counts: 32 true, 1 false, 7 amended (40 rows).

| Claim | Verdict | Evidence |
|---|---|---|
| The highest migration on `hr/MOD-125` is `0035_requirement_keys.sql`; `0031` is MOD-152's `command_log`; M4's is `0036` | true | `ls migrations/`: `0001`…`0035_requirement_keys.sql`, no `0036`; `0031_command_log.sql` header "MOD-152 milestone 1" |
| `pending_resumes` (Pg) is `WHERE r.status = 'stopped' AND r.executing_box_id = $1 AND NOT EXISTS (workspace_stop …) ORDER BY r.queued_at, r.id` (`pg/write.rs:8048-8063`), and the partial index `run (executing_box_id) WHERE status = 'stopped'` serves it | true | source `pg/write.rs:8048-8063` verbatim; PG 18.7 scratch `EXPLAIN` (seqscan off): `Index Scan using idx_run_stopped_box … Index Cond: (executing_box_id = …)`, then `Sort` on `queued_at, id` |
| `idx_run_lease` (`0003:50`) and `idx_run_workspace_status` (`0019:37`) exist; neither serves `pending_resumes` well; the latter serves `workspace_live_runs` | true | `0003:50` `(executing_box_id, lease_expires_at) WHERE status IN ('queued','running')` (excludes `stopped`); `0019:37` `run(workspace_id, status)` |
| `htui_target_version` is raised only in `apply_migrations` (`pg/mod.rs:399-404`, `raise_target`); `headless_checks` refuses `this_version() < target` (`pg/mod.rs:986-1012`) | true | callers of `raise_target`: `apply_migrations` (`:402`) and `testkit::build_template` only; `apply_migrations`'s one product caller is the store worker's `ApplyMigrations` arm, guarded `held.is_some()` (a pending schema, `store_worker.rs:3472`); `headless_checks` `:986`, `this_version() < target` → `BelowTarget` |
| (D9, Summary) `0036` is "the first migration since every crate went `0.2.0`", so applying it is what refuses a `0.1.0` worker | **false** → amended in Summary, D9, T1 | the bump `5c621db4` reached `main` in `a05deeac` (2026-11-12 22:58); `0033_memory_capture` landed in `e96f2dcf` (11-13), `0034_item_artifact_link` in `6816c8f4` and `0035_requirement_keys` in `18df483b` (11-14), all after it. A database a `main` build migrated through `0035` already carries target `0.2.0`. `0036` still discharges M2 owed (1) (HANDOFF.md:1081-1083) and raises any database still below |
| `HTUI_VERSION` is `env!("CARGO_PKG_VERSION")` of htui-store, `0.2.0` today | true | `pg/mod.rs:59`; `crates/htui-store/Cargo.toml:3` `0.2.0` (all nine crates `0.2.0`) |
| `PgStore::schema_version()` is the highest embedded migration, so `0036` rebuilds each mirror once; no cache migration needed for an index | true | `pg/mod.rs:758` `MIGRATOR.iter().map(|m| m.version).max()`, "the number `cache_meta.schema_version` is compared against"; an index adds no mirrored column |
| Migration-count pins: `migrations.rs` `:111-196`, `:4266`, `:4367`, `:4376`, `:4399`, `:4550`; `connect.rs` `:140`, `:155`, `:241`; no others (`read_only_pg.rs:173` counts `MIGRATOR`) | true | repo-wide text search: `Pending(35)` 6 hits, `thirty-five` 9 hits, `0035_requirement_keys` in tests only these plus `migrations.rs:35,106` (the `TABLES` doc and list comment, which stay); `connect.rs:155` is `pending, 35`; `migrations.rs:4550-4552` the `_sqlx_migrations` count 35; `read_only_pg.rs:173` `MigrationsPending(embedded)`; `Some(35)`/`35 embedded` none |
| `TABLES.len()` is 56 and an index moves neither it nor `it__connection__confirm.snap` (23 mirrored tables) | true | `migrations.rs:172-174` asserts 56 (the `TABLES` doc comment at `:24` still says "55", a pre-existing stale line); the snapshot's "the 23 mirrored tables" counts mirror tables; `schema 3` in it is synthetic |
| `testkit::fresh_db` reaches `0036` automatically (template rebuilt from the embedded set) | true | `testkit.rs` `template_name` hashes every embedded migration's version, description, checksum and `HTUI_VERSION`; a new migration names a new template, built by `MIGRATOR.run` + `raise_target` |
| `pg_indexes.indexdef` for the new index reads the same on Postgres 16 and 18 | amended (T1) | PG 18.7 printed `CREATE INDEX idx_run_stopped_box ON public.run USING btree (executing_box_id) WHERE (status = 'stopped'::text)`, the plan's expected text exactly. No PG 16 server in the sandbox (client only); precedent: `migration_0030_…` pins `'<kind>'::text` and `the_auto_queue_…` a partial `WHERE (…)`, both green on 16 before the test DB moved to 18 (2026-11-14). Owed on the host anyway |
| `WriteStore` implementors: MemStore, PgStore, `Writer` (`writer.rs`), `UsageSpy` (`htui-agent/src/conformance.rs`), `SpyStore` (`htui-agent/tests/it/recorder.rs`); no others | true | text `WriteStore for`: `mem.rs:11164`, `pg/write.rs:2607`, `writer.rs:384`, `htui-agent/src/conformance.rs:784`, `recorder.rs:472`; `Backend` has none (`backend.rs:6`); forward anchors `writer.rs:1135`, `conformance.rs:1252`, `recorder.rs:978` are each `workspace_stop` |
| `stop_workspace` (`traits.rs:1687`), `workspace_stop` (`:1699`), `resume_workspace` (`:1708`) signatures and semantics (escalate graceful→forced, never downgrade, repeat writes nothing; `resume_workspace` `Ok(false)` when not stopped) | true | `traits.rs:1675-1708` docs: upsert, unknown workspace `NotFound`, unknown `by` `Constraint`, graceful asked forced escalates all three columns, every other repeat writes nothing; resume `Ok(false)` when not stopped, `NotFound` for an unknown workspace |
| `WorkspaceStop` doc says "Stopping vs stopped is derived from the workspace's runs (M4), never stored" (`model/stop.rs:20-21`); `model/mod.rs:221` re-exports `stop::{PendingStop, StopMode, WorkspaceStop}` | true | `stop.rs:20-21`; `mod.rs:221` |
| No store read lists a workspace's runs today | true | text `workspace_id = $1` in `htui-store/src`: only `delete_workspace_once`'s checks touch `run`; no trait or inherent list read filtered by workspace |
| `RunStatus::is_terminal` exists (`run.rs:99`) and `queued`, `running`, `awaiting_approval`, `stopped` are exactly the non-terminal run statuses | true | `run.rs:62` `is_active` = those four; `:99` `is_terminal = !is_active` |
| A stop leaves `queued` runs queued and `awaiting_approval` runs parked (M2 D7, D8); only `running` runs are settled `stopped` | true | `PendingStop` doc (`stop.rs:34-36`): "a `running` run it leases"; M2 plan D7 (walking commands, approving a parked run refused) and D8 (`admit` holds queued entries) |
| Conformance `CASES.len()` is 324, pinned at `mem_store.rs:44` and `pg_conformance.rs:70` (`EXPECTED_CASES`) | true | `mem_store.rs:43-44`; `pg_conformance.rs:69-70` (doc list ends "make it 324"), message `:77-…` |
| (Patterns, T2) the case's dispatch is a `run_case` arm | amended | the arms live in `run_case_inline` (`conformance.rs:498`; `pending_resumes_…` arm `:1082`); `run_case` (`:482`) dispatches to it |
| `promote::opening_kind(caps, events)` (`promote.rs:136`) and `banner` decide reopen vs handoff; `stopped_opening` (`engine.rs:5884-5932`) uses them with caps defaulted for a missing agent | amended (T3 doc) | `opening_kind` `:136`, `banner` just above it; `stopped_opening` `engine.rs:5895-5901` `.unwrap_or_default()` for an agent row not found. But the agent id is `step.agent_id` through `candidate_of` (`engine.rs:10613`), which **errors** on `None`; only an unknown agent defaults |
| `resume_live_step` (`engine.rs:5780-5834`) re-walks a step with no events; a stopped judge re-runs fresh (M3 D4) | true | `engine.rs:5787-5797` `events.is_empty()` → `walk_live_step`; `select_stage` `:7964` "a stopped judge re-runs fresh on its own row" → `run_judge(…, Some(judge))` |
| `RunStep.fanout_index == -1` is the judge; `RunStep.agent_id: Option<AgentId>` (`run.rs:327-397`) | true | `run.rs:336-338`, `:341-342`; `engine.rs:111` |
| `htui_agent::registry::caps_for(&Agent) -> DriverCaps` (`registry.rs:152`); `agents()` is inherent on MemStore (`mem.rs:615`) and PgStore (`pg/read.rs:1794`) returning `AgentSummary { agent, .. }` | true | `registry.rs:152`; `mem.rs:615` `pub async fn agents` in `impl MemStore`; `pg/read.rs:1794`; `summary.agent` used at `orch conformance.rs:18406`. `DriverCaps` is `Copy` (`driver.rs:366`) |
| (D8, T4) slug resolution "through `workspaces()`" inside the generic core `S: ReadStore + WriteStore` | amended (D8, T4 `Context`) | `workspaces()` is inherent (`mem.rs:569`, `pg/read.rs:1645`), on no store trait but `WorkerHost` (`htui-core/src/store/worker.rs:616`), which `MemStore` does not implement (`WorkerHost for`: PgStore, Backend, a test `Failing`). The caller passes the list in `Context`, as the caps |
| The four M3 orch cases T3 extends exist at the cited lines and can read the step's rows and agent caps before `resume_stopped` | true | `conformance.rs:18310`, `:18398`, `:18450`, `:18476`, `:19063`; each holds the stopped `RunStep` and calls `events_of` before `resumed_to_done`/`resume_stopped`; `orch.store().agents()` reads the agent rows (`:18406`), `caps_for` in reach (orch depends on htui-agent); harnesses: `Demo` in `tests/it/fake_conformance.rs:6` and the in-crate `Demo` (`:19121`) |
| `ReadStore::item` gives `Item.key`; `WriteStore::boxes` gives `BoxRecord` with a hostname | amended (D4) | `traits.rs:126`, `:654`; the hostname is `BoxRecord.row.hostname` (`box_.rs:129`), and `boxes()` lists "every box of this user" only |
| `PgStore::this_user()` (`pg/mod.rs:781`) is the `UserId` a headless connect bootstraps | true | `pg/mod.rs:780-783`; `connect_headless` (`:287-305`) runs `bootstrap`, which sets `this_user` |
| `qdrant_cmd::connect` (`qdrant_cmd.rs:205-213`) is the headless write connect (keyring DSN, `load_or_mint`, `connect_headless`, `PoolSize::TUI`); `dsn_exit`/`failure_exit` map its refusals | true | source as described; all three private `fn` today (T4 makes them `pub(crate)`); `failure_exit` already imported by its own tests |
| `state_cmd::StateExit` codes 3/2/1 and `main`'s `exit_code`/`reports_to_sentry` handle any `StateExit` from `lib::run` | true | `state_cmd.rs:51-72`; `main.rs:105-135` downcasts `StateExit`, `:148-162` keeps `Offline`/`Refused` out of Sentry; `status`/`waiting` exit 0/3/2/1 (`state_cmd.rs:1-6`) |
| `cli::Command` holds `Worker, Provision, Status, Waiting, Claude, Mcp`; `args_conflicts_with_subcommands = true`; no Rust verb prompts or has `--yes` | true | `cli.rs:166-191`, `:12`; text `[y/N]`, `pub yes: bool`: none; no `short =` anywhere in `crates/` |
| clap 4.6.6: `#[arg(long, short = 'y')]`, required positional, `args_conflicts_with_subcommands` and the global `--log` behave as D1 and T4's parse tests need | true | scratch-crate probe: `stop x`, `stop --force -y x`, `resume --yes x`, `stop x --log f` parse; `stop` → `MissingRequiredArgument`, `--demo stop x` → `ArgumentConflict`, `resume --force x` → `UnknownArgument` |
| `std::io::IsTerminal` on stdin, as `lib.rs` `set_qdrant_key` reads it | true | `lib.rs:165`, `:230` `std::io::IsTerminal::is_terminal(&std::io::stdin())`; compiled in the probe |
| sqlx offline: a new `query_as!` needs its `.sqlx` entry; the index does not change it | true | `.cargo/config.toml` `[env] SQLX_OFFLINE = "true"`; the entry hashes the query text and its describe, neither of which an index moves |
| `crates/htui/tests/it/main.rs` lists the `it` modules (`mod state_cli_pg;` at `:63`) | true | text search `mod state_cli_pg` → `main.rs:63` |
| `CANCEL_GRACE` 2 s and `STOP_SLACK` 5 s (`runtime.rs:117,139`) | amended (D7) | `htui-worker/src/runtime.rs:129` `CANCEL_GRACE` (not `:117`), `:139` `STOP_SLACK` |
| The demo's `RUN_8` in `graphics` is stopped with no box and no stop row | true | `fixtures.rs:1743-1749` `graph_run(RUN_8, WORKSPACE_GRAPHICS, …, Stopped)`, `graph_run` sets `executing_box_id: None` (`:1632`); no `WorkspaceStop` in `fixtures.rs`; slug `graphics` (`:717`) |
| `CONCEPTS.md:66-67` holds "can always be resumed [R-ORCH-16]" | true | `CONCEPTS.md:66-67`; `**Reviewed:** 2026-10-20` at `:4` |
| `docs/hr-sandbox.md:227-236` gives the `cargo sqlx prepare` recipe on `localhost:5439` | true | lines 227-237 |
| `scripts/hr:154-160` `confirm`: `--yes` skips, no terminal refuses | true | `scripts/hr:154-161` |
| (T2 doc) only `traits.rs` says no index serves `pending_resumes` | amended (T2) | `pg/write.rs:8040-8042` says it too ("No index serves it (`idx_run_lease` is partial …)"); T2 owns that file and updates both |
