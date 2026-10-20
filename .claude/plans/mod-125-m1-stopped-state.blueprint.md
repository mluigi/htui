# Blueprint: MOD-125 M1 — Stopped state and workspace gate (T1–T5)

> Produced by `code-architect` 2026-10-18 from the fact-checked plan
> `.claude/plans/mod-125-m1-stopped-state.plan.md` (D1–D8), PRD
> `.claude/prds/mod-125-stop-and-resume-runs.prd.md`. Tree read at `9d7679fa` (branch `hr/MOD-125`).
> The plan stays the spec. §0 lists where the tree disagrees with it: rows marked **adopted** are
> implementation detail and need no call; rows marked **DV-n** change or contradict a plan decision
> and wait for the maintainer (§7). Until a DV is answered, implement the **recommended** branch;
> every DV names the exact lines to flip if the maintainer overrules it.

## 0. Plan claims contradicted or extended by the tree

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | D2's "every other SQL list gains `'stopped'`" names `pg/read.rs:1417` and `cache/read.rs:1480`, which are `PgStore::active_runs` and `CacheStore::active_runs`: the top-bar count D2 itself excludes. | `read.rs:1417` and `cache/read.rs:1480` both sit in `fn active_runs`; Mem twin `mem.rs:487` | **DV-1**: both keep three statuses. |
| F2 | One constant pins two queries. `ACTIVE_RUN_STATUSES` (`pg/read.rs:50`) is the literal of both `waiting_candidates` (`:1178` doc) and `active_runs`, pinned to `is_active` by `active_runs_literal_matches_run_status` (`:3292`). After D2 the two lists differ. | `read.rs:50,1178,3286-3300` | **adopted**: split into `LIVE_RUN_STATUSES` (4, `waiting_candidates`) and `TOP_BAR_RUN_STATUSES` (3, `active_runs`), each with its own pin (§T2.4). |
| F3 | D2 adds `'stopped'` at `pg/relay.rs:544` (`pending_commands`' lapsed-lease arm). A stopped run has `lease_owner = NULL`, so every command poll would pick up a cancel pending on it. `cancel_run` then hits `cancel_enabled`'s `Stopped` refusal, which is the "transient, retry next poll" arm (`runtime.rs:3122`), so it retries every poll for ever. The Mem twin (`mem.rs:7740`) is not in the plan either. | `relay.rs:540-547`, `runtime.rs:3118-3123` | **DV-2**: leave both unchanged in M1. |
| F4 | D2 lists no `htui-worker` site. `cancel_run` (`runtime.rs:3020,3034`) and `lost_to_a_claim` (`:2820`) treat any non-queued, non-terminal status as "executing". A `c` on a stopped run would write a durable `run_command` row that nobody applies, which breaks D2's "M1 shows no cancel that cannot work". | `runtime.rs:2816-2824, 3020-3037` | **adopted** (the plan's own intent): three edits (§4, rows 50–51). |
| F5 | MOD-77 D1 fixes the Postgres lock order as **step, then run** (`write.rs:203-240`, `step_fence`, `park_step`'s `FOR UPDATE OF s, r`). `stop_run` and D6's settlement are the first writers that update a run **and** its steps. Done run-first, they invert that order and can deadlock (`40P01`) against `park_step`, `upsert_step_tree` (`FOR NO KEY UPDATE OF s`) or a stalled former owner. | `write.rs:203-240, 6103` | **adopted**: `stop_run` locks steps before the run. The settlement locks steps first with `SKIP LOCKED` and defers a run whose running step another transaction holds (§T3.3). D6's "keep `FOR UPDATE SKIP LOCKED`" holds, and is extended to the steps. |
| F6 | `queue_item` has 63 positional call sites, most in T4-owned files (`htui-worker/src/runtime.rs` tests, `htui-worker/tests/auto_queue.rs`, `htui/src/{queue_overview,run_worker,store_worker}.rs`, `htui/tests/queue_overlay.rs`, `pg_criteria.rs`, `mem.rs` tests). A new argument added in T3 breaks the plan's "T3 ∥ T4 disjoint". | fork census, `mem.rs:951`, `write.rs:7892`, `backend.rs:618` | **DV-3**: move the `queue_item` **signature** change and its 63 sites into T2's fan-out (T2 already rewrites `queue_item`'s SELECT for `QueueEntry.workspace_id`). T3 keeps the request plumbing (`StoreRequest`, `serve_queue`, `on_queue`). |
| F7 | D7's `stop_run(run, owner, at)`: no column takes `at`. A stopped run gets no `finished_at` (D2), steps get none, and `updated_at` is the trigger's. `release_lease(run, owner)`, its closest neighbour, has no `at`. | `traits.rs:1209-1214` ("every writer that stamps a clock takes the instant from the caller"), `:1335-1358` | **DV-4**: drop `at`. |
| F8 | PRD metric "zero runs **admitted**, claimed or adopted in a stopped workspace". D5 leaves `admit` unchanged, so the queue runner still **creates** queued runs (up to `free_slots`) in a stopped workspace. They cannot start, but they are admitted in the queue's own vocabulary. | `runtime.rs:2280-2340`, plan D4 ("no `WorkerHost` verb in M1") | **DV-5**: accept for M1 and gate `admit` in M2. |
| F9 | `pg_conformance.rs:34` pins `const EXPECTED_CASES: usize = 164`; `tests/migrations.rs:103` pins the applied list `vec![1, …, 18]` plus a prose list; neither is in the plan. | — | **adopted**: T1 and T3 bump them. |
| F10 | `pg/demo.rs:442` (`load_demo`) inserts the fixture runs column by column. Fixture `Run`s that carry `workspace_id: Some(..)` round-trip only if it binds the column. | `run_and_steps_round_trip` (Pg and mirror) | **adopted**: fixtures use `Some(ids::WORKSPACE_PLATFORM)` (PROJECT_HTUI belongs only to PLATFORM), and `demo.rs` binds it (T2). |
| F11 | `enqueue_in_batch` has callers outside T4's file list: the `Orchestrate` trait (`htui-orch/src/conformance.rs:151`, impl `:283`, `run_in_batch` `:8013`), `FakeOrchestrator::enqueue_in_batch` (`htui-orch/src/fake.rs:2427`) and `enqueue_in_batch_fake` (`engine.rs:7638`). | — | **adopted**: `fake.rs` added to T4's files. |
| F12 | The plan names `Writer` as the extra `WorkerStore` implementor. `PgStore` implements `WorkerStore` too, in the same file (`htui-store/src/worker.rs:116`). The module doc counts methods ("60 methods: 14 … 6 … 25 writes", `htui-core/src/store/worker.rs:15-21`). | — | **adopted**: both impls in T3; the count becomes 62 (7 WriteStore reads, 26 writes). |
| F13 | `run_to_rest`, `select_stage`, `retire_slot`, `cursor` and the two style matches are confirmed. "Dim, like a parked state" is self-contradictory: parked is `theme.warning` (`runs.rs:1191-1203`, `execution_graph.rs:229-245`). | — | **adopted**: `theme.dim`, the plan's operative word. A stopped run asks for a deliberate resume, not an answer. |
| F14 | `create_run_records_its_workspace` needs no new verb. | — | **adopted**: it lands with T2 (TDD with the code it pins). The other nine T3 cases stay in T3. |
| F15 | ANA-2 `:673-676` says "`RunStatus::is_active()` stays `Queued \| Running \| AwaitingApproval` as shipped, and it is what `R-TUI-1`'s active-run count reads". That sentence becomes false under D2. | `docs/ANA-2.md:673-676` | **adopted**: T5 amends it too. |
| F16 | Mem must emulate the two new foreign keys, `ON DELETE SET NULL` on `run`/`queue_entry.workspace_id` and `CASCADE` on `workspace_stop`. Today `State::delete_workspace` (`mem.rs:4874`) clears only links and box paths. | — | **adopted**: T3 extends it, plus one conformance case. |

> **Deviation note on F5 (review round R1, 2026-10-19, finding M-1).** The settlement no longer
> locks steps before runs. `adopt_runs` locks its candidates `FOR UPDATE SKIP LOCKED`, reads the
> stop rows `FOR SHARE`, and only then, for the stopped candidates alone, reads their running steps
> plainly and locks them `FOR NO KEY UPDATE SKIP LOCKED`; a step the first read saw and the second
> skipped defers its run. Every run/step lock is `SKIP LOCKED` and every `UPDATE` touches only rows
> already held, so the sweep never waits on a run or step lock and MOD-77's step → run order is
> irrelevant to it; the only waiting lock (`FOR SHARE` on `workspace_stop`) is now taken holding no
> step lock. `stop_run` keeps steps first. Amended algorithm: §T3.3, `adopt_runs`.

---

## 1. Shared decisions for every task

- **Enum order.** `RunStatus::ALL` = `queued, running, awaiting_approval, stopped, done, failed, cancelled`.
  `StepStatus::ALL` = `pending, running, awaiting_approval, stopped, done, failed, cancelled, superseded`.
  `stopped` goes right after `awaiting_approval`, so the live statuses stay together. The CHECK
  lists in 0019 use the same order (`model/mod.rs:3-6`: variant order **is** CHECK order).
- **Lease after a stop.** `lease_owner = NULL`, `lease_expires_at = <store now>` and `lease_box_id`
  unchanged: `release_lease`'s rule (`traits.rs:1335-1358`). `executing_box_id` is unchanged too,
  because M3 resumes on the box that holds the CLI session files. Mem: `lease_owners.remove(&run)`,
  `lease_expires_at = Some(now)`.
- **A stopped run** gets no `finished_at`, keeps its item `in_progress`, and writes no item row.
- **No new `WorkerHost` verb**, no `Backend` verb and no mirror table for `workspace_stop` (D4).
- **Error taxonomy** as today: `NotFound { entity }` for the addressed row; `Constraint` for a
  foreign key (`references_no_row(column, id, table)` in Mem, `23503` through `map_sqlx` in Pg).

---

## T1 — Migration and mirror schema

### T1.1 `crates/htui-store/migrations/0019_stop_runs.sql` (probed, see T1.3)

```sql
-- 0019_stop_runs.sql - MOD-125 milestone 1 (plan D1, D3, D4). Forward-only (R-STO-5).
--
-- A run and a step can be 'stopped': halted by a workspace stop and resumable (R-ORCH-16). A
-- stopped run holds no slot (claim_run's slot counts 'running' only) but keeps its trees and its
-- branch, so it still refuses an overlapping scope (ANA-2 §4.7, invariant 6); it has no
-- finished_at and its item stays in_progress. 0001_init.sql's two status CHECKs are unnamed, so
-- Postgres named them run_status_check and run_step_status_check; both are dropped and re-added
-- under the same names with 'stopped' after 'awaiting_approval', the order RunStatus::ALL and
-- StepStatus::ALL follow.
--
-- run.workspace_id is the workspace a graph run was started from (PRD Q1): a workspace stop
-- stops only those runs. A graph run whose project belongs to exactly one workspace is backfilled
-- to it; a project in none or in several leaves its runs NULL, and no workspace stop ever touches
-- them. Chat runs stay NULL: a chat is driven by the person typing in it. ON DELETE SET NULL, so
-- a deleted workspace leaves its runs unscoped. The backfill fires trg_run_updated_at, which only
-- moves the cache cursor that the mirror rebuild below replaces anyway. queue_entry.workspace_id
-- is the workspace an item was queued from; the queue runner copies it onto the run it admits.
--
-- workspace_stop holds one row per stopped workspace (plan D4). Its presence is the gate that
-- claim_run and adopt_runs read; a resume deletes it. Not mirrored, like queue_batch: it is read
-- through the store. The mirror gains run.workspace_id in cache_migrations/0006_run_workspace.sql,
-- and schema_version becomes 19, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

ALTER TABLE run DROP CONSTRAINT run_status_check;
ALTER TABLE run ADD CONSTRAINT run_status_check CHECK (status IN
    ('queued','running','awaiting_approval','stopped','done','failed','cancelled'));

ALTER TABLE run_step DROP CONSTRAINT run_step_status_check;
ALTER TABLE run_step ADD CONSTRAINT run_step_status_check CHECK (status IN
    ('pending','running','awaiting_approval','stopped','done','failed','cancelled','superseded'));

ALTER TABLE run
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_run_workspace REFERENCES workspace(id) ON DELETE SET NULL;

CREATE INDEX idx_run_workspace_status ON run(workspace_id, status);

UPDATE run r
   SET workspace_id = wp.workspace_id
  FROM workspace_project wp
 WHERE wp.project_id = r.project_id
   AND r.kind = 'graph'
   AND NOT EXISTS (SELECT 1
                     FROM workspace_project other
                    WHERE other.project_id = r.project_id
                      AND other.workspace_id <> wp.workspace_id);

ALTER TABLE queue_entry
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_queue_entry_workspace REFERENCES workspace(id) ON DELETE SET NULL;

-- DV-6 (recommended): the same one-workspace rule for entries queued before 0019, so the run an
-- old entry admits is gated like a backfilled one. Drop this statement if the maintainer declines.
UPDATE queue_entry q
   SET workspace_id = wp.workspace_id
  FROM item i
  JOIN workspace_project wp ON wp.project_id = i.project_id
 WHERE i.id = q.item_id
   AND NOT EXISTS (SELECT 1
                     FROM workspace_project other
                    WHERE other.project_id = i.project_id
                      AND other.workspace_id <> wp.workspace_id);

CREATE TABLE workspace_stop (
    workspace_id UUID        PRIMARY KEY REFERENCES workspace(id) ON DELETE CASCADE,
    mode         TEXT        NOT NULL,
    requested_by UUID        NOT NULL REFERENCES app_user(id),
    requested_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT chk_workspace_stop_mode CHECK (mode IN ('graceful', 'forced'))
);

COMMENT ON COLUMN run.status IS
    'MOD-125 D1: stopped = halted by a workspace stop and resumable; live (not terminal), holds no '
    'slot, still overlaps, no finished_at.';
COMMENT ON COLUMN run_step.status IS
    'MOD-125 D1: stopped = interrupted by a workspace stop; resumes to running, or is superseded '
    'by a fresh rerun.';
COMMENT ON COLUMN run.workspace_id IS
    'MOD-125 D3: the workspace a graph run was started from; a workspace stop stops only these. '
    'NULL for chat runs and for runs whose project was in no or several workspaces at 0019.';
COMMENT ON COLUMN queue_entry.workspace_id IS
    'MOD-125 D3: the workspace the item was queued from; copied onto the run admit creates.';
COMMENT ON TABLE workspace_stop IS
    'MOD-125 D4: one row per stopped workspace; while it exists claim_run refuses and adopt_runs '
    'settles abandoned runs of the workspace as stopped. Resume deletes it. Not mirrored.';
COMMENT ON COLUMN workspace_stop.mode IS
    'MOD-125 D4: graceful (cancel the turn, keep the session) or forced (kill now); a forced stop '
    'is never downgraded by a later graceful one.';
COMMENT ON COLUMN workspace_stop.requested_by IS
    'MOD-125 D4: who asked for the stop (the escalating user, after an escalation).';
COMMENT ON COLUMN workspace_stop.requested_at IS
    'MOD-125 D4: the caller''s clock at the request (the escalation''s, after an escalation).';
```

`idx_run_lease` (`0003:50`, `WHERE status IN ('queued','running')`) is deliberately unchanged,
because a stopped run has no lease to scan.

### T1.2 `crates/htui-store/cache_migrations/0006_run_workspace.sql`

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-125 milestone 1 (plan D3): run.workspace_id, the workspace a graph run was
-- started from (migrations/0019_stop_runs.sql). TEXT, no foreign key, as 0001 mirrors every UUID.
-- The 'stopped' status needs nothing here: the mirror has no CHECKs (0001_mirror.sql). Neither
-- queue_entry.workspace_id nor workspace_stop is mirrored.
--
-- cache_meta.schema_version moves to 19 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run ADD COLUMN workspace_id TEXT;
```

### T1.3 Probe result (2026-10-18, scratch DB `mod125_probe` on `localhost:5439`, since dropped)

- `0001..0018` applied, then the exact SQL above. Before 0019 the DB was seeded with: a project in
  one workspace (graph run `running` and chat run `running`), a project in two workspaces (graph run
  `queued`), and a project in none (graph run `done`). **0019 applied.**
- The backfill set `workspace_id` only on the one-workspace graph run. The chat run, the
  two-workspace run and the no-workspace run stayed `NULL`.
- `\d run` shows `run_status_check` with `stopped` and `fk_run_workspace … ON DELETE SET NULL`.
  `\d run_step` shows `run_step_status_check` with `stopped`. `\d workspace_stop` shows the PK, both
  FKs and `chk_workspace_stop_mode`. `UPDATE run SET status='stopped'` succeeds, and
  `mode='paused'` is refused.
- The upsert of T3.4 and the settlement statements of T3.3 were run against the same DB, with the
  outcomes recorded there.
- Mirror: `0001..0005` + `0006_run_workspace.sql` applied through Python's `sqlite3`, and
  `run.workspace_id` is the last column.

### T1.4 Count updates (same commit)

- `crates/htui-store/tests/connect.rs:140,156,242`: `Pending(18)` → `Pending(19)`; "eighteen …
  through MOD-96's 0018_seed_permission_mode.sql" → "nineteen … through MOD-125's 0019_stop_runs.sql".
- `crates/htui-store/tests/migrations.rs`:
  - `:103`: `vec![1, …, 18]` → `…, 18, 19]`, and the prose list gains "MOD-125's `0019_stop_runs.sql`";
  - `:1461`, `:1562`: `Pending(19)`;
  - `:1571`, `:1594`: `MigrationsPending(19)`;
  - `:1746-1747`: `19`;
  - each message changes from "eighteen" to "nineteen". `rerun_0018` (`:1168`) is unchanged.

### T1 tests

| Test | Backend | Asserts |
|---|---|---|
| `migrations.rs::migration_0019_scopes_only_one_workspace_graph_runs` (new) | Pg | `common::bare_db()`, `MIGRATOR.run_to(18, …)` (the `migration_0018_pins_the_old_claude_row_once` staging), seed T1.3's four runs plus one queue entry, plain `run`; then `run.workspace_id`: one-workspace graph = W, chat / two-workspace / none = `NULL`; the entry of the one-workspace item = W (DV-6) |
| `migrations.rs::migration_0019_admits_stopped_and_refuses_an_unknown_stop_mode` (new) | Pg | `UPDATE run/run_step SET status='stopped'` ok; `workspace_stop.mode='paused'` → `23514` |
| existing `connect.rs`, `migrations.rs` count tests | Pg | Counts are 19 |

### T1 commits

1. `MOD-125 M1 T1: 0019_stop_runs.sql, the mirror's 0006 and the migration counts` (both SQL
   files, `connect.rs`, `migrations.rs` incl. the two new tests).

---

## T2 — Model, law and the compile fan-out

### T2.1 `crates/htui-core/src/model/run.rs`

```rust
str_enum!(
    /// `run.status` (§5.8).
    RunStatus {
        /// Waiting to start.
        Queued => "queued",
        /// Executing.
        Running => "running",
        /// Stopped at a gate.
        AwaitingApproval => "awaiting_approval",
        /// Halted by a workspace stop (MOD-125 D1, `R-ORCH-16`): live and resumable. Holds no
        /// slot, still refuses an overlapping scope (it keeps its trees and branch), has no
        /// `finished_at`, and its item stays `in_progress`.
        Stopped => "stopped",
        /// Finished successfully.
        Done => "done",
        /// Finished unsuccessfully.
        Failed => "failed",
        /// Stopped by a human.
        Cancelled => "cancelled",
    }
);
```

- `is_active`: the body becomes `matches!(self, Self::Queued | Self::Running | Self::AwaitingApproval | Self::Stopped)`.
  Replace its doc with: "Whether the run is **live** (MOD-125 D2): `queued | running |
  awaiting_approval | stopped`. Every live run may still move; the complement is
  [`RunStatus::is_terminal`]. Not the top bar's count, which is [`RunStatus::on_top_bar`]."
- New, right after it:
  ```rust
  /// R-TUI-18's active-run count (MOD-125 D2): the live runs less `stopped`, so the top bar keeps
  /// counting "running and waiting". `active_runs` in every store filters by it.
  #[must_use]
  pub const fn on_top_bar(self) -> bool {
      self.is_active() && !matches!(self, Self::Stopped)
  }
  ```
- `can_move_to` (cite `docs/ANA-2.md:653-671`, which fixes D8's stale `:600-616`):
  `Running => matches!(to, AwaitingApproval | Done | Failed | Cancelled | Stopped)` and a new arm
  `Self::Stopped => matches!(to, Self::Running | Self::Cancelled | Self::Failed)`.
- `is_terminal`: the doc keeps "`done | failed | cancelled`". The body is unchanged.
- `StepStatus` gains `Stopped => "stopped"` after `AwaitingApproval`, with the doc "Interrupted by a
  workspace stop (MOD-125 D1); resumes to `running` (M3), or is superseded by a fresh rerun (M3's
  fallback), or is cancelled or failed with its run."
- `StepStatus::can_move_to` (cite `docs/ANA-2.md:683-710`):
  `Running => …| Self::Stopped`, and the new arm
  `Self::Stopped => matches!(to, Self::Running | Self::Cancelled | Self::Failed | Self::Superseded)`.
  `is_terminal` is unchanged (a stopped step is not terminal).
- `Run`: add `pub workspace_id: Option<WorkspaceId>` after `item_id`, documented
  "`run.workspace_id` (MOD-125 D3): the workspace a graph run was started from; `None` for a chat run,
  a run started without a scope, or one whose project was in no or several workspaces at 0019."
  Mark it `#[serde(default)]`, because `Run` is serialised into worker frames.
- `NewRun`: add `#[serde(default)] pub workspace_id: Option<WorkspaceId>`, documented
  "`run.workspace_id` (MOD-125 D3); `None` is never gated by a workspace stop."
- Import `WorkspaceId`.
- Test consts: append `(Running, Stopped)`, `(Stopped, Running)`, `(Stopped, Cancelled)` and
  `(Stopped, Failed)` to `RUN_SANCTIONED`. Append `(Running, Stopped)`, `(Stopped, Running)`,
  `(Stopped, Cancelled)`, `(Stopped, Failed)` and `(Stopped, Superseded)` to `STEP_SANCTIONED`.
  Move both doc citations to `:653-671` / `:683-710` (D8).

### T2.2 `crates/htui-core/src/model/stop.rs` (new) and `model/mod.rs`

```rust
//! A workspace's stop record (MOD-125 D4; `0019_stop_runs.sql`). Not mirrored: read through the
//! store ([`WriteStore::workspace_stop`](crate::store::WriteStore::workspace_stop)).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{UserId, WorkspaceId};

str_enum!(
    /// `workspace_stop.mode` (MOD-125 D4, PRD gate decisions).
    StopMode {
        /// Cancel the current turn cleanly and keep the session; resume continues it.
        Graceful => "graceful",
        /// Kill now; resume reopens the session and redoes the interrupted turn.
        Forced => "forced",
    }
);

/// One `workspace_stop` row (MOD-125 D4): its presence is the gate. "Stopping" vs "stopped" is
/// derived from the workspace's runs (M4), never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStop {
    /// `workspace_stop.workspace_id`.
    pub workspace_id: WorkspaceId,
    /// `workspace_stop.mode`; only ever escalates `graceful -> forced`.
    pub mode: StopMode,
    /// `workspace_stop.requested_by`: the latest request that wrote the row.
    pub requested_by: UserId,
    /// `workspace_stop.requested_at`: the caller's clock, microsecond-truncated.
    pub requested_at: DateTime<Utc>,
}
```

In `model/mod.rs`, add `pub mod stop;` (alphabetical, after `skill_language`) and
`pub use stop::{StopMode, WorkspaceStop};`. The derive set matches `QueueBatch`.

### T2.3 `crates/htui-core/src/model/overlap.rs` and `queue.rs`

```rust
    /// MOD-125 D5: the run was started from a workspace that has a `workspace_stop` row. Writes
    /// nothing. Decided right after `NotClaimable` and before `MissingTags`, so a stopped
    /// workspace never fails a run for its tags. Transient: the run stays `queued`, and a claim
    /// after the resume admits it.
    WorkspaceStopped {
        /// The run's `workspace_id`.
        workspace: WorkspaceId,
    },
```

Place it after `NotClaimable`. The `Display` arm is
`Self::WorkspaceStopped { workspace } => write!(f, "workspace {workspace} is stopped")`, and
`EngineError::ClaimRefused` renders it as `claim refused: workspace <id> is stopped`. Update the
`Claim` type doc: "MOD-125 adds `WorkspaceStopped`, which writes nothing."

`QueueEntry` gains `pub workspace_id: Option<WorkspaceId>` after `box_id`, documented
"`queue_entry.workspace_id` (MOD-125 D3): the workspace the item was queued from, copied onto the
run `admit` creates." Import `WorkspaceId`.

### T2.4 Compile fan-out (exhaustive matches; no catch-alls)

| Site | New arm |
|---|---|
| `store/traits.rs:2154` `chat_step_status` | `RunStatus::Queued \| RunStatus::Running \| RunStatus::AwaitingApproval \| RunStatus::Stopped => None` |
| `htui-orch/src/engine.rs:820` `Engine::claim` | `claim @ (Claim::NotClaimable \| Claim::WorkspaceStopped { .. } \| Claim::SlotFull { .. } \| Claim::Overlaps { .. }) => return Err(EngineError::ClaimRefused { run, claim })`. Add "or the run's workspace is stopped (MOD-125 D5)" to the docs at `:781` and `command.rs:403-405`, and update the comment at `:799-802` ("four verdicts") |
| `engine.rs:3003` `run_to_rest` | Add `\| RunStatus::Stopped` to the `AwaitingApproval \| Done \| Failed \| Cancelled => return self.resting(&row).await` arm. `resting` (`:6858`) writes nothing and answers `Rest { run: Stopped, .. }`; `landed`/`walk_live_step` treat a non-terminal rest as parked: no cleanup |
| `engine.rs:4664` `select_stage` (judge status) | Add `\| StepStatus::Stopped` to the no-op `Running \| Done \| Superseded \| Cancelled => {}` arm |
| `htui-orch/src/gate.rs:1018` `retire_slot` | Add `\| StepStatus::Stopped` to the `Running \| Failed` arm (move to `Cancelled`); `superseded` stays reserved for M3's rerun |
| `htui-orch/src/status.rs:331` `cursor` | Add `\| StepStatus::Stopped` to `Running \| AwaitingApproval \| Failed => Cursor::Rest { step, status }` |
| `htui/src/ui/tabs/backlog/detail/runs.rs:1191` `run_style` | Add `\| RunStatus::Stopped` to the `Done` (dim) arm (F13) |
| `runs.rs:1202` `step_style` | Add `\| StepStatus::Stopped` to the `Done` (dim) arm |
| `runs/execution_graph.rs:229` `border` | Add `\| StepStatus::Stopped` to `Superseded \| Cancelled` (dim) |
| `execution_graph.rs:245` `text` | Add `\| StepStatus::Stopped` to `Superseded \| Cancelled` (dim) |

`run_status`/`step_status` (`runs.rs:1286`) use `other => other.as_str()`. `"stopped"` fits the 9-
and 10-column grids, so they need no change and no snapshot moves.

### T2.5 Field sites

- **`Run` literals (7):**
  - `fixtures.rs:1438,1458,1478`: `Some(ids::WORKSPACE_PLATFORM)`, F10;
  - `mem.rs:2695` (`start_chat_run`): `None`;
  - `mem.rs:5330` (`create_run`): `new.workspace_id`;
  - `cache/read.rs:236` (`run_of`): `opt_uuid_col::<WorkspaceId>("run.workspace_id", opt_text(row, "workspace_id")?.as_deref())?`;
  - `pg_criteria.rs:9953`: `None`.
- **`NewRun` literals (18):** `workspace_id: None`, except `engine.rs:745` (`enqueue_with`), which
  gets `workspace_id: workspace` in T4. Until then T2 writes `None` there. The 18 are:
  - `htui-agent/tests/relay.rs:133`, `htui-mcp/tests/tools_backlog.rs:77`, `htui-orch/tests/review_loop.rs:61`;
  - `conformance.rs:4871` (`new_run`), `mem.rs:12263`;
  - `engine.rs:745, 9664, 12937, 13042`;
  - `htui-store/tests/cache.rs:650`, `pg_criteria.rs:1083`;
  - `runtime.rs:4688`, `htui/src/queue_overview.rs:323`, `run_worker.rs:1619, 1661`;
  - `htui/tests/backlog.rs:978, 3310`, `htui/tests/waiting.rs:102`.
- **`QueueEntry` literals (4):**
  - `mem.rs:971` (`queue_item`): `workspace_id: workspace`, DV-3;
  - `model/queue.rs:609`: `None`;
  - `pg/read.rs:2395` (`queue_rows`): `row.workspace_id`;
  - `overlay/queue.rs:719`: `None`.
- **Pg `Run` lists:** in `query_as!(Run, …)` at `pg/read.rs:647,1224,2605` and `pg/write.rs:4601`
  (RETURNING) and `:4975`, add `workspace_id AS "workspace_id: WorkspaceId"` after `item_id`.
  `create_run`'s INSERT gains the column and `$11` = `new.workspace_id.map(WorkspaceId::as_uuid)`.
- **Mirror:**
  - `RUN_SELECT` (`cache/read.rs:230`) appends `workspace_id`;
  - `RUN_COLUMNS` and `refresh_run`'s SELECT (`cache/refresh.rs:1103,1134`) append `workspace_id`;
  - `refresh_run` binds `row.workspace_id.map(|id| id.to_string())` at the same position, because
    the binds are positional.
- **Pg `QueueEntry` lists:** the three byte-identical SELECTs (`pg/read.rs:2287`, `pg/write.rs:7926,8001`)
  add `e.workspace_id AS "workspace_id: WorkspaceId"` after `e.box_id`, kept byte-identical so they
  share one `.sqlx` entry. `queue_rows` (`read.rs:2395`) adds the column too.
- **`pg/demo.rs:442`** binds `run.workspace_id.map(WorkspaceId::as_uuid)` (F10).
- **`create_run`, both stores.** Mem: after the box check,
  `if let Some(workspace) = new.workspace_id && !self.workspaces.contains_key(&workspace)` returns
  `Constraint(references_no_row("run.workspace_id", workspace, "workspace"))`. Pg: the FK does the
  same (`23503` → `Constraint`). The `create_run` doc's "or on any foreign key" already covers it.
- **`queue_item` (DV-3, recommended in T2).** The new signature, identical on MemStore, PgStore and
  `Backend`:
  ```rust
  pub async fn queue_item(&self, item: ItemId, box_id: BoxId, workspace: Option<WorkspaceId>,
                          by: UserId, at: DateTime<Utc>) -> Result<QueueEntry>
  ```
  Add to the doc: "`workspace` (MOD-125 D3) is recorded on a new entry. An entry already stored
  comes back unchanged, its workspace included." The `# Errors` line becomes "`Constraint` for an
  unknown box, workspace or user".
  - Mem checks the workspace after the idempotent early return, beside the box check:
    `references_no_row("queue_entry.workspace_id", w, "workspace")`.
  - Pg: `INSERT … (item_id, box_id, position, queued_at, queued_by, workspace_id) VALUES ($1, $2, NULL, $3, $4, $5)`.
  - All 63 call sites pass `None`, except `serve_queue`, which is T3's.

`Command::StartRun` and the `enqueue*` signatures are T4's; T2 touches no other call site.

### T2.6 D2 liveness verdicts

Apply the §4 table. The rows marked "change" are T2's edits, and the SQL edits need a
`cargo sqlx prepare` (see Validation).

### T2 tests (first)

| Test | Backend | Asserts |
|---|---|---|
| `run.rs::the_run_status_table_sanctions_exactly_the_ana_2_pairs` (extended consts) | unit | D1's run rows |
| `run.rs::the_step_status_table_sanctions_exactly_the_ana_2_pairs` (extended) | unit | D1's step rows |
| `run.rs::a_terminal_run_status_is_one_that_reaches_nothing` (unchanged) | unit | still holds with `Stopped` live |
| `run.rs::a_terminal_step_status_is_done_cancelled_or_superseded` (unchanged) | unit | `Stopped` not terminal |
| `run.rs::a_stopped_run_is_live_and_holds_no_slot` (new) | unit | `Stopped.is_active()`, `!is_terminal()`, `!on_top_bar()`; `Running.on_top_bar()`; the slot half is pinned in T3 by `a_stopped_run_holds_no_slot_but_still_overlaps` |
| `model/mod.rs::run_enums_match_check_lists` | unit | the new CHECK orders of T1.1 |
| `model/mod.rs::stop_mode_matches_check_list` (new) | unit | `["graceful","forced"]` |
| `model/mod.rs::run_status_activity_matches_top_bar_rule` → **rename** `run_status_liveness_and_the_top_bar_rule` | unit | `is_active` = the four live statuses; `on_top_bar` = `queued \| running \| awaiting_approval` |
| `overlap.rs::claim_display_names_the_rule_and_the_holding_run` (extended) | unit | `WorkspaceStopped` → `"workspace <id> is stopped"`, `!is_admitted()` |
| `command.rs::the_named_refusals_say_what_went_wrong` (extended) | unit | `"claim refused: workspace <id> is stopped"` |
| `command.rs::cancel_is_enabled_exactly_where_the_run_table_allows_it` (`:1650`) | unit | `Stopped` refused, with the comment "MOD-125 D2: the law allows it; wired in M3" |
| `command.rs` cleanup test (`:2311`) | unit | `Stopped` refused |
| `pg/read.rs::live_runs_literal_matches_run_status` (renamed pin) | unit | `LIVE_RUN_STATUSES` = `is_active` filter |
| `pg/read.rs::top_bar_runs_literal_matches_run_status` (new pin) | unit | `TOP_BAR_RUN_STATUSES` = `on_top_bar` filter |
| conformance `create_run_records_its_workspace` (F14) | Mem + Pg | `create_run` with `Some(WORKSPACE_PLATFORM)` → returned row and `run()` carry it; `None` stays `None`; an unknown workspace → `Constraint`, no row |
| conformance `waiting_candidates`/`overlapping_runs`/`close_drained_batch` parity (existing suites) | Mem + Pg | still green with the SQL lists widened |
| `htui-worker` runtime test `a_cancel_of_a_stopped_run_is_refused_and_writes_no_row` (new) | Mem | Seed a run `running -> stopped` through `transition_run`, then `CancelRun`. Expect a refusal that names `stopped`, and `pending_commands` empty |
| `cache.rs::run_and_steps_round_trip` (existing) | Pg + mirror | fixtures' `workspace_id` round-trips |

Add the `CASES` entry and the `run_case` arm for the conformance case, and bump `EXPECTED_CASES`
to 165.

### T2 commits (each compiles)

1. `MOD-125 M1 T2: stopped run and step status, the law and the liveness verdicts`. Covers T2.1
   enums/law/`on_top_bar`, `model/mod.rs` tests, the T2.4 match arms, the §4 edits, the pin split
   and `.sqlx`.
2. `MOD-125 M1 T2: run.workspace_id and queue_entry.workspace_id end to end in the stores`. Covers
   the `Run`/`NewRun`/`QueueEntry` fields, the T2.5 sites, `queue_item`'s argument (DV-3), the
   conformance case, `.sqlx` and `demo.rs`.
3. `MOD-125 M1 T2: StopMode, WorkspaceStop and Claim::WorkspaceStopped`. Covers `model/stop.rs`,
   the `Claim` variant, `Display`, the `engine.rs:820` arm and the doc and test updates.

---

## T3 — Store verbs, gate and settlement

### T3.1 `WriteStore` (`store/traits.rs`, new block after `release_lease`)

```rust
    // ---- MOD-125 M1 (plan D4, D7): workspace stops ---------------------------------------------

    /// MOD-125 D4: records that `workspace` is stopped and answers the record as stored. An upsert
    /// on `workspace_stop`'s key, decided in this order: an unknown workspace is `NotFound`, an
    /// unknown `by` is `Constraint`; no row inserts one (`mode`, `requested_by = by`,
    /// `requested_at = at`); a `graceful` row asked `forced` **escalates**, all three columns
    /// taking this request's values; every other repeat (`graceful` over `graceful`, anything over
    /// `forced`) writes nothing and answers the stored row, so a forced stop is never downgraded
    /// and the first request's instant is kept. Moves no run: the executors apply the stop (M2).
    /// `at` is the caller's clock, microsecond-truncated.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "workspace" }`;
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for an unknown user.
    async fn stop_workspace(
        &self,
        workspace: WorkspaceId,
        mode: StopMode,
        by: UserId,
        at: DateTime<Utc>,
    ) -> Result<WorkspaceStop>;

    /// MOD-125 D4: the workspace's stop record; `None` when it is not stopped, or does not exist.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn workspace_stop(&self, workspace: WorkspaceId) -> Result<Option<WorkspaceStop>>;

    /// MOD-125 D4: deletes the workspace's stop record; `Ok(true)` when one existed, `Ok(false)`
    /// when the workspace was not stopped. Moves no run: a queued run starts at its next claim,
    /// and a stopped run is resumed by M3.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "workspace" }`, told
    /// apart from "not stopped" by one follow-up read.
    async fn resume_workspace(&self, workspace: WorkspaceId) -> Result<bool>;

    /// MOD-125 D7: a compare-and-set `running -> stopped` on a run whose `lease_owner` is `owner`.
    /// In one transaction every `running` step of the run moves to `stopped` and the lease is
    /// released as [`release_lease`](WriteStore::release_lease) releases it (`lease_owner = NULL`,
    /// `lease_expires_at = <store now>`, `lease_box_id` and `executing_box_id` kept). No
    /// `finished_at`, no item write. `Ok(false)` when the run is not `running` or not `owner`'s;
    /// nothing is written then. The caller has quiesced its walk (M2): a step it starts
    /// concurrently is not this verb's to stop.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, read first.
    async fn stop_run(&self, run: RunId, owner: Uuid) -> Result<bool>;
```

(DV-4: if the maintainer keeps D7's `at`, the signature becomes `stop_run(&self, run, owner,
at: DateTime<Utc>)`, with the sentence "`at` is accepted for M2's notes and stamps no column".)

Doc amendments:
- **`claim_run`** (`traits.rs:1255`): after "`NotClaimable` when …", insert "then
  [`Claim::WorkspaceStopped`] when the run's `workspace_id` names a workspace with a
  `workspace_stop` row (MOD-125 D5; nothing written, the run stays `queued`; a run with no
  workspace is never gated)". In "Every other answer writes nothing", `WorkspaceStopped` is one of
  them.
- **`adopt_runs`** (`:1280`): add "MOD-125 D6: a candidate whose `workspace_id` has a
  `workspace_stop` row is **settled, not adopted**. In the same transaction it moves `running ->
  stopped` with every `running` step, its lease is released as `release_lease` releases it, and it
  is left out of the answer. On Postgres a candidate whose running step another transaction holds
  is deferred to a later sweep, neither settled nor adopted (step-then-run lock order)."

### T3.2 `WorkerStore` (`store/worker.rs`)

Add under the "-- 6 WriteStore reads" comment:
`fn workspace_stop(&self, workspace: WorkspaceId) -> impl Future<Output = Result<Option<WorkspaceStop>>> + Send;`
documented [`WriteStore::workspace_stop`].

Add under writes:
`fn stop_run(&self, run: RunId, owner: Uuid) -> impl Future<Output = Result<bool>> + Send;`
documented [`WriteStore::stop_run`].

Implementors forward by UFCS:
- MemStore (`htui-core/src/store/worker.rs:682`);
- PgStore (`htui-store/src/worker.rs:116`);
- Writer (`:497`).

Module doc: "62 methods: 14 ReadStore reads, 7 WriteStore reads, 26 writes, …"; and "MOD-125 M1
adds `workspace_stop` and `stop_run`".

Writer (`writer.rs:330`), UsageSpy (`htui-agent/src/conformance.rs:751`) and SpyStore
(`htui-agent/tests/recorder.rs:439`) delegate each of the four `WriteStore` verbs in one line,
the way `release_lease` does.

### T3.3 Algorithms

**MemStore `State`**

- New field `workspace_stops: BTreeMap<WorkspaceId, WorkspaceStop>`, documented "`workspace_stop`
  (MOD-125 D4), by workspace". Initialise it in `from_demo` (`mem.rs:368-430`) with
  `BTreeMap::new()`.
- **`claim_run`** (`mem.rs:5363`): right after the `NotClaimable` return (`:5381`):
  ```rust
  // MOD-125 D5: after claimability, before the tags, so a stopped workspace never fails a run.
  if let Some(workspace) = claimed.workspace_id
      && self.workspace_stops.contains_key(&workspace)
  {
      return Ok(Claim::WorkspaceStopped { workspace });
  }
  ```
  Also widen the overlap `live` filter (`:5418-5424`) to
  `matches!(row.status, RunStatus::Running | RunStatus::AwaitingApproval | RunStatus::Stopped)`,
  per §4 row 14. The slot count stays `== Running`, and the comment at `:5411-5416` gains
  "a `stopped` run is in the overlap set and not the slot (MOD-125 D2)". (This edit is T2's, §4.)
- **`adopt_runs`** (`:5505`): the candidate filter and order are unchanged. In the loop:
  ```rust
  let settle = self.runs.get(&id).and_then(|row| row.workspace_id)
      .is_some_and(|workspace| self.workspace_stops.contains_key(&workspace));
  if settle {
      // MOD-125 D6: a stopped workspace's abandoned run is settled, not adopted.
      self.lease_owners.remove(&id);
      if let Some(row) = self.runs.get_mut(&id) {
          row.status = RunStatus::Stopped;
          row.lease_expires_at = Some(now);
          row.updated_at = now;
      }
      for step in self.steps.values_mut()
          .filter(|step| step.run_id == id && step.status == StepStatus::Running)
      {
          step.status = StepStatus::Stopped;
          step.updated_at = now;
      }
      continue;
  }
  // …today's adoption, unchanged
  ```
- **`stop_run(run, owner, now)`**:
  1. `require_run(run)?` gives `NotFound`;
  2. if `lease_owners.get(&run) != Some(&owner)` or the status is not `Running`, return `Ok(false)`;
  3. otherwise move the status to `Stopped`, set `lease_expires_at = Some(now)` and
     `updated_at = now`, `lease_owners.remove`, move every `Running` step of the run to `Stopped`
     with `updated_at = now`, and return `Ok(true)`.
- **`stop_workspace(workspace, mode, by, at, _now)`**:
  1. if `!workspaces.contains_key` → `NotFound { entity: "workspace" }`;
  2. `require_user(by, "workspace_stop.requested_by")?`;
  3. `let at = at.trunc_subsecs(TIMESTAMPTZ_DIGITS)`;
  4. match `workspace_stops.get(&workspace)`:
     - `None`: insert;
     - `Some(row)` if `row.mode == Graceful && mode == Forced`: overwrite `mode`, `requested_by`, `requested_at`;
     - `Some(_)`: keep.
  5. Return the stored clone.
- **`workspace_stop`** is `workspace_stops.get(&w).cloned()`.
- **`resume_workspace`**: if `!workspaces.contains_key` → `NotFound`; otherwise
  `Ok(workspace_stops.remove(&w).is_some())`.
- **`delete_workspace`** (F16): `delete_workspace(id, now)`. The wrapper reads `self.now()`, and
  the function also:
  - removes `workspace_stops[id]`;
  - sets `workspace_id = None` with `updated_at = now` on every run whose `workspace_id == Some(id)`
    (Pg's `SET NULL` fires the `updated_at` trigger);
  - sets `workspace_id = None` on every queue entry.
- The MemStore async wrappers follow `claim_run`'s shape (`let now = self.now(); self.write(|state| …)`).

**PgStore** (`pg/write.rs`)

- **`claim_run` gate** (`write.rs:4744`): the run read (`:4724-4730`) adds
  `workspace_id AS "workspace_id: WorkspaceId"`. Right after the `NotClaimable` return:
  ```rust
  // MOD-125 D5: run, then box, then the stop row. stop_workspace and resume_workspace lock only
  // workspace_stop, so no cycle. FOR SHARE makes a racing resume (DELETE) or escalation wait for
  // this decision; a stop inserted after this read is ordered after the claim, and M2's executor
  // stops the run it admitted.
  if let Some(workspace) = claimed.workspace_id {
      let stopped = sqlx::query_scalar!(
          r#"SELECT 1 AS "one!" FROM workspace_stop WHERE workspace_id = $1 FOR SHARE"#,
          workspace.as_uuid(),
      )
      .fetch_optional(&mut *tx).await.map_err(map_sqlx)?.is_some();
      if stopped {
          return Ok(Claim::WorkspaceStopped { workspace });
      }
  }
  ```
  The early return drops `tx`, so it rolls back, like `NotClaimable`. Update the method doc's
  `# Errors` / decision paragraph to match T3.1. The overlap list at `:4847` gains `'stopped'`, as
  §4 row 15 (T2).

- **`adopt_runs`** — **amended by review round R1 (2026-10-19, M-1; see the F5 deviation note):**
  step 1 below is deleted. The order is step 2 (candidates, `FOR UPDATE SKIP LOCKED`), step 3
  (stops, `FOR SHARE`), then, only when some candidate's workspace is stopped, (a) step 4's plain
  read and (b) the lock
  `SELECT id FROM run_step WHERE run_id = ANY($1::uuid[]) AND status = 'running' ORDER BY run_id, id FOR NO KEY UPDATE SKIP LOCKED`
  into the held set; a stopped candidate is deferred when (a) has a step (b) lacks, settled
  otherwise. (a) and (b) are not collapsed into one `SKIP LOCKED` read: that read cannot tell a
  held step from no step, and would settle a run whose step another transaction holds
  (`pg_criteria.rs::adopt_runs_defers_a_stopped_run_whose_step_is_held`). Steps 5–7 are unchanged.
  The original text follows.

  `adopt_runs` becomes an explicit transaction on READ COMMITTED (`self.pool.begin()`), seven
  statements. Each id list is empty-checked in Rust, so the common path with no stopped workspace
  runs only steps 2, 6 and 7.
  1. **Steps first** (F5; MOD-77's step → run order). The running steps of stopped-workspace
     candidates:
     ```sql
     SELECT s.id AS "step: StepId", s.run_id AS "run: RunId"
       FROM run_step s
       JOIN run r ON r.id = s.run_id
       JOIN workspace_stop ws ON ws.workspace_id = r.workspace_id
      WHERE r.executing_box_id = $1 AND r.status = 'running' AND r.kind = 'graph'
        AND (r.lease_expires_at IS NULL OR r.lease_expires_at <= clock_timestamp())
        AND r.lease_owner IS DISTINCT FROM $2
        AND s.status = 'running'
      ORDER BY s.run_id, s.id
        FOR NO KEY UPDATE OF s SKIP LOCKED
     ```
  2. **Candidates.** Today's predicate and order, now selected as rows:
     ```sql
     SELECT id AS "id: RunId", workspace_id AS "workspace_id: WorkspaceId"
       FROM run
      WHERE executing_box_id = $1 AND status = 'running' AND kind = 'graph'
        AND (lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())
        AND lease_owner IS DISTINCT FROM $2
      ORDER BY queued_at, id
        FOR UPDATE SKIP LOCKED
     ```
  3. **Stops held**, for the candidates' distinct non-null workspaces:
     ```sql
     SELECT workspace_id AS "workspace_id: WorkspaceId"
       FROM workspace_stop WHERE workspace_id = ANY($1::uuid[]) FOR SHARE
     ```
  4. **Running steps now**, read under the run locks, for the candidates in a stopped workspace:
     ```sql
     SELECT id AS "id: StepId", run_id AS "run_id: RunId"
       FROM run_step WHERE run_id = ANY($1::uuid[]) AND status = 'running'
     ```
     Partition the candidates in Rust:
     - **settle**: stopped workspace, and every running step from step 4 is among step 1's held ids;
     - **defer**: stopped workspace, with a running step another transaction holds. Logged with
       `tracing::debug!(run, "a stopped workspace's run is deferred: a step is locked")`;
     - **adopt**: every other candidate (workspace `NULL` or not stopped).
  5. **Settle**:
     ```sql
     UPDATE run_step SET status = 'stopped' WHERE id = ANY($1::uuid[]) AND status = 'running'
     ```
     ```sql
     UPDATE run SET status = 'stopped', lease_owner = NULL, lease_expires_at = clock_timestamp()
      WHERE id = ANY($1::uuid[]) AND status = 'running'
     ```
  6. **Adopt.** Today's CTE, with `candidates` replaced by the adopt ids:
     ```sql
     WITH swept AS (
         UPDATE run r
            SET lease_owner      = $2,
                lease_box_id     = $1,
                lease_expires_at = clock_timestamp() + $3::bigint * interval '1 microsecond'
          WHERE r.id = ANY($4::uuid[])
      RETURNING r.*
     )
     SELECT <the 19 Run columns, workspace_id included> FROM swept ORDER BY queued_at, id
     ```
  7. `tx.commit()`, then return step 6's rows. `SKIP LOCKED` is on every lock this verb takes on
     `run` and `run_step`, so two sweeps still never wait on each other (A-5), and none waits on a
     stalled former owner. The only waiting lock is `FOR SHARE` on `workspace_stop`, and only
     `stop_workspace`/`resume_workspace` hold that row, for one statement.

  Probed on the scratch DB: one candidate in a stopped workspace (one done step and one running
  step) and one unscoped candidate. The first ended `stopped`, with the lease owner cleared, the
  running step `stopped` and the done step untouched. The second was adopted and returned.

  Update the method doc: it is no longer "in one statement". It is one transaction, steps first,
  and the A-5 paragraph stands.

- **`stop_run`** (one transaction):
  1. `SELECT status AS "status: RunStatus", lease_owner AS "lease_owner?" FROM run WHERE id = $1`
     (unlocked). `None` → `NotFound { entity: "run" }`. If the owner is not `owner` or the status
     is not `Running` → `Ok(false)` (the early exit takes no lock).
  2. `SELECT id AS "id: StepId" FROM run_step WHERE run_id = $1 AND status = 'running' ORDER BY id FOR NO KEY UPDATE`.
     Steps first, MOD-77; this waits, since the run is ours.
  3. Re-check the run under its own lock:
     `SELECT status AS "status: RunStatus", lease_owner AS "lease_owner?" FROM run WHERE id = $1 FOR NO KEY UPDATE`.
     If it is no longer `running`/ours → `Ok(false)` (rollback).
  4. `UPDATE run_step SET status = 'stopped' WHERE id = ANY($1::uuid[]) AND status = 'running'`.
  5. `UPDATE run SET status = 'stopped', lease_owner = NULL, lease_expires_at = clock_timestamp() WHERE id = $1`.
  6. Commit, `Ok(true)`.

- **`stop_workspace`** (one transaction):
  1. `SELECT 1 AS "one!" FROM workspace WHERE id = $1` → `NotFound { entity: "workspace" }`.
  2. `SELECT 1 AS "one!" FROM app_user WHERE id = $1` → `Constraint(references_no_row("workspace_stop.requested_by", by, "app_user"))`.
     This read is explicit, because the no-op conflict path writes no row and so takes no FK
     check; without it Pg and Mem would disagree.
  3. The upsert (probed: graceful@1 → graceful@1 kept on a repeat → forced@3 escalates → graceful@4
     and forced@5 keep forced@3):
     ```sql
     INSERT INTO workspace_stop AS ws (workspace_id, mode, requested_by, requested_at)
     VALUES ($1, $2, $3, $4)
     ON CONFLICT (workspace_id) DO UPDATE
        SET mode         = 'forced',
            requested_by = EXCLUDED.requested_by,
            requested_at = EXCLUDED.requested_at
      WHERE ws.mode = 'graceful' AND EXCLUDED.mode = 'forced'
     RETURNING workspace_id AS "workspace_id: WorkspaceId", mode AS "mode: StopMode",
               requested_by AS "requested_by: UserId", requested_at
     ```
     Use `fetch_optional`. **No row comes back when the `WHERE` is false (probed)**, so on `None`
     read the row with `SELECT … FROM workspace_stop WHERE workspace_id = $1` (`fetch_one`).
     `ON CONFLICT DO UPDATE` locks the conflicting row even when its `WHERE` fails, so the row is
     stable in this transaction.
  4. Commit.

- **`workspace_stop`**: `SELECT … FROM workspace_stop WHERE workspace_id = $1` (`fetch_optional`,
  on the pool).
- **`resume_workspace`**: `DELETE FROM workspace_stop WHERE workspace_id = $1`. One row → `true`;
  zero → `SELECT 1 FROM workspace WHERE id = $1`, which is `false` when the workspace exists and
  `NotFound` when it does not (`refresh_lease`'s follow-up pattern).

### T3.4 `queue_item` request plumbing (`htui`)

- `StoreRequest::QueueItem { item: ItemId, workspace: WorkspaceId }` (`htui/src/store_worker.rs:373`),
  with the doc "the scope's workspace, recorded on the entry (MOD-125 D3)".
- `serve_queue` (`:2420`) passes `Some(*workspace)` to `backend.queue_item(*item, box_id, Some(*workspace), user, now())`.
- `on_queue` (`ui/tabs/backlog/mod.rs:507`) sends `StoreRequest::QueueItem { item: id, workspace: ctx.scope.workspace_id }`.
- Other sites:
  - `store_worker.rs:1210` (`{ .. }`) is unchanged;
  - `:5226`, `:5485` and `backlog/mod.rs:1590` add the field;
  - `backlog/mod.rs:1437` changes the pattern to `{ item, .. }`, or better asserts the workspace.

If DV-3 is declined, the `queue_item` signature change and its 63 call sites move here, and T3 and T4
must then run serially.

### T3 tests (conformance first; each = `CASES` entry + `run_case` arm; `EXPECTED_CASES` 165 → 175)

Every case runs on MemStore (`htui-core/tests/mem_store.rs`) and PgStore (`htui-store/tests/pg_conformance.rs`).

| Case | Asserts |
|---|---|
| `stop_workspace_upserts_and_escalates_never_downgrades` | graceful@t1 stored; graceful@t2 → unchanged t1; forced@t3 by U2 → forced/t3/U2; graceful@t4 and forced@t5 → forced/t3 kept; `workspace_stop` equals each answer; unknown workspace → `NotFound "workspace"`; unknown user → `Constraint`, no row |
| `resume_workspace_deletes_and_reports` | stopped → `true`, `workspace_stop` `None`; again → `false`; unknown workspace → `NotFound`; resume of a never-stopped workspace → `false` |
| `claim_run_refuses_a_stopped_workspace_and_writes_nothing` | run (W) queued, W stopped → `WorkspaceStopped { workspace: W }`; the run row equals its pre-claim snapshot (`queued`, no lease, no `executing_box_id`); item still `queued`; a **running** run of W claimed again is `NotClaimable` (order) |
| `claim_run_checks_the_stop_before_tags` | item requires a tag the box lacks, W stopped → `WorkspaceStopped`, run `queued`, item `queued` (no `MissingTags` write); resume → `MissingTags` |
| `claim_run_admits_after_resume` | stop, then `WorkspaceStopped`, then resume, then `Admitted`, with the lease and item `in_progress` as in `claim_run_admits_one_and_refuses_the_second` |
| `a_run_without_a_workspace_is_never_gated` | both fixture workspaces stopped; a `workspace_id: None` run → `Admitted`; after `release_lease`, `adopt_runs` adopts it |
| `adopt_runs_settles_a_stopped_workspaces_abandoned_run_as_stopped` | run A (W) and run B (`None`) claimed by owner X; A gets one `done` and one `running` step; stop W; `release_lease(A/B, X)`; `adopt_runs(BOX, Y)` answers `[B]` only; A is `stopped`, its `running` step `stopped`, its `done` step untouched, `refresh_lease(A, Y)` and `(A, X)` both `false`, `lease_expires_at` set, `finished_at` `None`, item `in_progress`; a second `adopt_runs` answers `[]` |
| `stop_run_is_a_fenced_cas_on_running` | wrong owner → `false`, nothing changed; right owner → `true`, run `stopped`, running steps `stopped`, `pending`/`done` steps untouched, `refresh_lease(owner)` `false`; again → `false`; a parked (`awaiting_approval`) run of the owner → `false`; unknown run → `NotFound "run"` |
| `a_stopped_run_holds_no_slot_but_still_overlaps` | default limit 2: A (scope R) and B (scope ∅) claimed; `stop_run(A)`; D (scope R) → `Overlaps { with: A, .. }`; C (scope ∅) → `Admitted` (A holds no slot) |
| `a_deleted_workspace_unscopes_its_runs_and_drops_its_stop` (F16) | W is a scratch workspace from `create_workspace`; a queued run with `workspace_id: Some(W)`; stop W; `delete_workspace(W)` → the run's `workspace_id` `None`, `workspace_stop(W)` `None`; the claim is then `Admitted` |

Inherent `queue_item` (not generic):

| Test | Backend | Asserts |
|---|---|---|
| `mem.rs::queue_item_records_its_workspace` | Mem | `Some(W)` stored and returned; a repeat with another workspace returns the stored entry unchanged; an unknown workspace → `Constraint` |
| `pg_criteria.rs::queue_item_records_its_workspace` | Pg | the same three |
| `pg_criteria.rs::adopt_runs_defers_a_stopped_run_whose_step_is_held` | Pg only | tx T holds `SELECT … FROM run_step WHERE id = s FOR UPDATE`; `adopt_runs` returns promptly (timeout 5 s), neither settling nor adopting that run; after T commits, the next `adopt_runs` settles it |
| `pg_criteria.rs::stop_run_takes_the_steps_first` | Pg only | a park-shaped tx holding `FOR UPDATE OF s, r` and `stop_run` from another connection: both finish, no `40P01` (the `upsert_step_tree_takes_the_step_first` shape) |
| `store_worker.rs::queue_item_records_the_scope_workspace` | Mem backend | `QueueItem { item, workspace }` → `queue_entries` shows `Some(workspace)` |
| `backlog/mod.rs` `on_queue` test (`:1437`) | unit | the request carries `ctx.scope.workspace_id` |

### T3 commits

1. `MOD-125 M1 T3: stop_workspace, workspace_stop, resume_workspace and stop_run`. Covers the trait
   verbs and docs, both stores, Writer, both WorkerStore impls, the spies, the cases
   `stop_workspace_upserts…`, `resume_workspace…`, `stop_run_is…`, `a_stopped_run_holds_no_slot…`,
   `stop_run_takes_the_steps_first`, `.sqlx` and `EXPECTED_CASES` = 169.
2. `MOD-125 M1 T3: the workspace gate in claim_run`. Covers the gate in both stores, the doc and
   the four claim cases (`EXPECTED_CASES` = 173).
3. `MOD-125 M1 T3: adopt_runs settles a stopped workspace's abandoned runs`. Covers both stores,
   Pg's transaction rewrite, the settle case, `adopt_runs_defers…`, `.sqlx` (`EXPECTED_CASES` = 174).
4. `MOD-125 M1 T3: a deleted workspace unscopes its runs in MemStore`. Covers the F16 edit and its
   case (`EXPECTED_CASES` = 175).
5. `MOD-125 M1 T3: the queue request records the scope's workspace`. Covers T3.4 and its tests
   (plus the inherent `queue_item` tests, if DV-3 is declined).

---

## T4 — Engine and runtime workspace plumbing

### T4.1 Signatures

- `Command::StartRun` (`htui-orch/src/command.rs:57`):
  ```rust
  StartRun {
      item: ItemId,
      mode: RunMode,
      repo_scope: Option<Vec<RepoId>>,
      /// MOD-125 D3: the workspace the run is started from, recorded as `run.workspace_id`;
      /// `None` (a test, an unscoped caller) is never gated by a workspace stop.
      workspace: Option<WorkspaceId>,
  },
  ```
- In `engine.rs`:
  - `pub async fn enqueue(&self, item: ItemId, mode: RunMode, repo_scope: Option<Vec<RepoId>>, workspace: Option<WorkspaceId>) -> Result<RunId, EngineError>` (`:675`);
  - `pub async fn enqueue_in_batch(&self, item: ItemId, batch: BatchId, workspace: Option<WorkspaceId>)` (`:689`);
  - `async fn enqueue_with(&self, item, mode, repo_scope, batch: Option<BatchId>, workspace: Option<WorkspaceId>)` (`:699`), which writes `workspace_id: workspace` at `:745`;
  - `start_run(item, mode, repo_scope, workspace)` (`:761`).
- Destructuring: `engine.rs:630` and `runtime.rs:2647` add `workspace`, and `runtime.rs` `start_run`
  (`:2720`) passes it to `engine.enqueue(…, workspace)` (`:2740`). `{ .. }` sites
  (`runtime.rs:1079,2677`, `views.rs:69`) are unchanged.
- `Orchestrate::enqueue_in_batch` (`htui-orch/src/conformance.rs:151`, impl `:283`, `run_in_batch`
  `:8013`), `FakeOrchestrator::enqueue_in_batch` (`fake.rs:2427`) and `enqueue_in_batch_fake`
  (`engine.rs:7638`) gain the same trailing `workspace: Option<WorkspaceId>`.
- `admit` (`runtime.rs` ~2280-2340): beside `project_of`, build
  `let workspace_of: HashMap<ItemId, Option<WorkspaceId>> = entries.iter().map(|e| (e.item_id, e.workspace_id)).collect();`
  and call `engine.enqueue_in_batch(item, batch.id, workspace_of.get(&item).copied().flatten())`.
- `RunsTab::action` (`runs.rs:835`): `Command::StartRun { item, mode: RunMode::Manual, repo_scope: None, workspace: Some(ctx.scope.workspace_id) }`.
- The other 73 constructions take `workspace: None`, except where a test asserts the workspace:
  - `engine.rs` (47), `htui-orch/src/conformance.rs` (13), `gix_isolator.rs` (2), `views.rs:1062`,
    `auto_queue.rs:604`;
  - `run_worker.rs:817,3045`, `runs.rs:3385` (asserts `Some(scope)`);
  - `htui/tests/chat.rs:1113`, `runs_pg.rs:648,1424,1494`, `worker_crash_pg.rs:618`, `worker_pg.rs:400`.
- `enqueue(...)` callers: `engine.rs` tests at 14510, 14522, 14596, 14692, 14761, 14800, 18770,
  21447, 21771 and 21868. They pass `None`.

### T4 tests

| Test | Backend | Asserts |
|---|---|---|
| `engine.rs::start_run_records_the_workspace_it_was_started_from` | Mem | `StartRun { workspace: Some(W) }`, then `run(id).workspace_id == Some(W)`; `None` stays `None` |
| `engine.rs::a_claim_refused_for_a_stopped_workspace_leaves_the_run_queued_and_writes_nothing` (**after T3 merges**) | Mem | Stop W through `stop_workspace`, then `StartRun { workspace: Some(W) }`. Expect `Err(ClaimRefused { claim: WorkspaceStopped { workspace: W } })`, the run `queued` with no lease, item `queued`, no note. After a resume, `claim(run)` → `Started` |
| `auto_queue.rs::admit_records_the_entrys_workspace_on_the_run` | Mem | Queue an item with `Some(W)` and open a batch; after the sweep, `run.workspace_id == Some(W)` |
| `runs.rs::shift_r_starts_a_run` (updated) | unit | the exact `Command` carries `workspace: Some(ctx.scope.workspace_id)` |
| `htui-orch` + `htui-worker` existing suites | Mem / Pg | unchanged behaviour (`--no-fail-fast`, grep SIGABRT) |

### T4 commits

1. `MOD-125 M1 T4: Command::StartRun and enqueue carry the run's workspace`. Covers the signatures,
   all 74 constructions, `fake.rs` and `Orchestrate`, the `RunsTab` production site and its test,
   and the engine records test.
2. `MOD-125 M1 T4: admit copies the queue entry's workspace onto the run`. Covers `workspace_of`
   and the `auto_queue.rs` test.
3. `MOD-125 M1 T4: a stopped workspace's claim leaves the run queued` (after the T3 merge). The
   engine test only.

---

## T5 — Docs (`docs/ANA-2.md`)

Under the run table (`:653-671`) and the step table (`:683-710`), add an italic paragraph
`*Amended by MOD-125 M1, 2026-10-18:*` followed by these rows in the `| from | event | guard | to | who |` format:

- **Run:**
  - `running | workspace stop applied | — | stopped | orchestrator (M2)`;
  - `running | lease expired, workspace stopped | recovery sweep (§4.9), D6 | stopped | recovery sweep`;
  - `stopped | resume | session reopened or rerun fresh | running | orchestrator (M3)`;
  - `stopped | cancel | — | cancelled | user (M3)`;
  - `stopped | resume impossible and no rerun | — | failed | orchestrator (M3)`.
- **Step:**
  - `running | workspace stop applied | — | stopped | orchestrator (M2)`;
  - `running | lease expired, workspace stopped | recovery sweep | stopped | recovery sweep`;
  - `stopped | resume | session reopened | running | orchestrator (M3)`;
  - `stopped | resume cannot reopen the session | fresh rerun inserted | superseded | orchestrator (M3)`;
  - `stopped | run cancelled | — | cancelled | orchestrator`;
  - `stopped | run failed | — | failed | orchestrator`.

Then:
- Amend `:673-676` (F15): "`RunStatus::is_active()` is the live set, `Queued | Running |
  AwaitingApproval | Stopped` since MOD-125; R-TUI-1's active-run count reads `on_top_bar()`, the
  live set less `stopped`."
- Note MOD-24's "no mid-step resume" as narrowed: crash recovery is unchanged, and explicit stops
  resume in M3.
- In `run.rs:57,99` and the test-const docs, the citations become `:653-671` / `:683-710` (done in T2.1).

Validate with `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

Commit: `MOD-125 M1 T5: ANA-2 run and step tables gain the stopped rows`.

---

## 4. D2 liveness verdicts — every site read at HEAD

"Live" = `is_active()` after D2. "none" means the site already answers right for a stopped run once
`is_active` includes it. Every edit in this table is T2's (commit 1); rows 14–15 are repeated in
T3.3 only for context.

| # | Site | Today | Change | Why |
|---|---|---|---|---|
| 1 | `model/mod.rs:426` `run_status_activity_matches_top_bar_rule` | pins the 3-status set | rename and extend (T2 tests) | `is_active` now means live; `on_top_bar` keeps three |
| 2 | `run.rs:1012` `a_terminal_run_status_is_one_that_reaches_nothing` | is_terminal ⇔ reaches nothing ⇔ !is_active | none | stopped reaches running/cancelled/failed |
| 3 | `mem.rs:487` `MemStore::active_runs` | `is_active()` | `run.status.on_top_bar()` | D2 exclusion (top bar) |
| 4 | `pg/read.rs:1417` `PgStore::active_runs` | 3-status literal | **none** (DV-1; plan says add) | twin of #3 |
| 5 | `cache/read.rs:1480` `CacheStore::active_runs` | 3-status literal | **none** (DV-1; plan says add) | twin of #3 |
| 6 | `pg/read.rs:50` `ACTIVE_RUN_STATUSES` + pin `:3292` | one constant, pinned to `is_active` | split into `LIVE_RUN_STATUSES` (4) and `TOP_BAR_RUN_STATUSES` (3), two pins; update the `:1178` doc | F2 |
| 7 | `mem.rs:2005` `waiting_candidates` | `is_active()` | none | live runs are candidates |
| 8 | `pg/read.rs:1246` `waiting_candidates` | 3 | add `'stopped'` | twin of #7 (plan ✓) |
| 9 | `cache/read.rs:1342` `waiting_candidates` | 3 | add `'stopped'` | twin of #7 (plan ✓) |
| 10 | `mem.rs:1198` `close_drained_batch` | `.any(is_active)` | none | a stopped auto run is still in flight; the batch must not drain |
| 11 | `pg/write.rs:8196` `close_drained_batch` | 3 | add `'stopped'` | twin (plan ✓) |
| 12 | `mem.rs:1318` `overlapping_runs` | `is_active()` | none | keeps trees and branch |
| 13 | `pg/read.rs:2626` `overlapping_runs` | 3 | add `'stopped'` | twin (plan ✓) |
| 14 | `mem.rs:5418-5424` `claim_run` overlap `live` | `Running \| AwaitingApproval` | add `\| RunStatus::Stopped` | D2 overlap; slot filters `== Running` afterwards |
| 15 | `pg/write.rs:4847` `claim_run` overlap read | `('running','awaiting_approval')` | add `'stopped'` | twin (plan ✓); slot `:4826` stays `= 'running'` |
| 16 | `mem.rs:918` / `pg/read.rs:2241` `active_runs_on_box` | `Running \| AwaitingApproval` | none | sweep's adopt trigger (`runtime.rs:2138`); a stopped run has nothing to adopt |
| 17 | `mem.rs:5557` / `pg/write.rs:5053` `take_lease`; `engine.rs:2106` re-read | `Running \| AwaitingApproval` | none (D2 exclusion) | not leasable; M3 needs its own path |
| 18 | `mem.rs:5679` / `pg/write.rs:5212` `transition_run` | `finished_at` on terminal only | none | stopped gets no `finished_at` |
| 19 | `mem.rs:5710` / `pg/write.rs:5278` `transition_step` | terminal = done/cancelled/superseded | none | stopped step not terminal |
| 20 | `mem.rs:6485` / `pg/write.rs:934` `finish_run` `!to.is_terminal()` | refuses non-terminal `to` | none | `finish_run(Stopped)` refused; stopping is `stop_run` |
| 21 | `mem.rs:6517` `finish_run` "other live run of the item" | `is_active()` | none | a stopped sibling keeps the item where it is |
| 22 | `pg/write.rs:982` `finish_run_on` live count | 3 | add `'stopped'` | twin of #21 (plan ✓) |
| 23 | `mem.rs` `fail_run` (`legal_move`) | law-driven | none | D1 adds `stopped -> failed` |
| 24 | `pg/write.rs:6178` `fail_run` | `('queued','running','awaiting_approval')` = the law's `-> failed` column | add `'stopped'` | D1 (plan ✓) |
| 25 | `mem.rs:6546` `close_out` live-run refusal | `is_active()` | none | stopped run holds trees and branch |
| 26 | `pg/write.rs:6312` `close_out` | 3 | add `'stopped'`, fix the rustdoc `:6289` | twin (plan ✓) |
| 27 | `mem.rs:6360` / `pg/write.rs:5978` `promote_step` | refuses only a terminal run | none in M1 (flag, §6 R5) | `promote_enabled`'s hand list (`command.rs:1125`) refuses a stopped run |
| 28 | `mem.rs:7634` `relay_view`; `pg/relay.rs:631,650`; `mem.rs:8062` `follow_up_views` | `!is_terminal()` / `NOT IN (done,failed,cancelled)` | none | already include stopped; Mem = Pg |
| 29 | `mem.rs:7740` / `pg/relay.rs:543-544` `pending_commands` | terminal arm + `('running','awaiting_approval')` lapsed arm | **none** (DV-2; plan says add at 544) | F3: adding it loops a refused cancel every poll; Mem twin unlisted |
| 30 | `command.rs:983` `cancel_enabled` | `is_active()` | `run.status.is_active() && run.status != RunStatus::Stopped`; rustdoc `:942-947` "the three live statuses but `stopped` (MOD-125 D2: wired in M3)" | D2 exclusion |
| 31 | `engine.rs:1836` `cancel_run` → `cancel_enabled`; `:1903` `cancel_leased` (`StepStatus::is_terminal`) | guard; cancels non-terminal steps | none | refuses stopped; `stopped -> cancelled` legal for M3 |
| 32 | `command.rs:1069` `retry_admitted`, `:1125` `promote_enabled` | `Running \| AwaitingApproval` | none | refusing both on a stopped run is right in M1 |
| 33 | `command.rs:1217` `unblock_enabled`; `engine.rs:2493` / `views.rs:473` `unblock_case` | over `is_active` runs | none | stopped run's item is `in_progress`; wording edge only |
| 34 | `command.rs:1253` `close_out_enabled` | first live run refuses | none | consistent with #25 |
| 35 | `command.rs:1271` `cleanup_enabled` | `is_terminal()` | none; doc `:1268` lists the four refused | stopped keeps its trees (D2) |
| 36 | `engine.rs:2784` `land_recovered`, `:2794` `landed`, `:3813` `walk_live_step` | cleanup and `Finished` only when terminal | none | `Rest { run: Stopped }` is parked |
| 37 | `conformance.rs:12156` `close_out_resolution_law` | skips items with a live run | none | stopped is live |
| 38 | `runtime.rs:2820` `lost_to_a_claim` | `status != Queued && !terminal` | `!matches!(*status, RunStatus::Queued \| RunStatus::Stopped) && !status.is_terminal()` | **plan missed (F4)** |
| 39 | `runtime.rs:3020` `cancel_run` rowless path | `Queued \|\| is_terminal()` | `\|\| row.status == RunStatus::Stopped` | F4: no durable row for a cancel nobody applies |
| 40 | `runtime.rs:3034` `cancel_run` re-read | `!= Queued && !terminal` | `&& row.status != RunStatus::Stopped` | F4, defensive twin of #39 |
| 41 | `runtime.rs:3118` terminal-refusal arm | `is_terminal()` | none | unreachable for stopped after #38-40 |
| 42 | `runtime.rs:1945` `wake_on_rest`, `:1980` `retry_claims` | `Running \| Queued` = not at rest | none | a stopped run is at rest and frees a slot: waking the sweep is right |
| 43 | `runtime.rs:2532` `cancels_first` | pending cancels on `Running` graph runs before adopt | none | a cancel still beats a stop |
| 44 | `views.rs:2094` (test) | `is_active() && == AwaitingApproval` | none | correct |
| 45 | `runs.rs:1836` `RunsTab::has_active_run` | `is_active()` → 5 s poll | none | polling a stopped run is wanted (another process may resume it) |
| 46 | `queue_overview.rs:240` `push_run` | `finished_at` iff terminal | none | correct |
| 47 | `model/queue.rs:451` `classify_entry` | `Queued \| Running` latest run | none in M1 | stopped falls to `NotReady`; M4/MOD-116 display gap |
| 48 | `0003` `idx_run_lease` partial index | `('queued','running')` | none | no lease to scan |
| 49 | `mem.rs:1560,5084,2203` | item `Status::is_terminal` | none | not run sites |

**Plan verdicts confirmed:** `pg/read.rs:1246,2626`, `pg/write.rs:982,4847,6178,6312,8196`,
`cache/read.rs:1342`, and the `cancel_enabled` and `take_lease` exclusions. **Plan verdicts that
look wrong:** `pg/read.rs:1417` and `cache/read.rs:1480` (DV-1), and `pg/relay.rs:544` (DV-2).
**Missed:** rows 38–40 (F4) and row 6 (F2).

---

## 5. Validation (gate on the real tree, `--test-threads=1`)

```bash
# sqlx data after T1 (columns) and after every T2/T3 commit that changes a query!:
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx_mod125"
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod125 cargo sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod125 cargo sqlx prepare -- --all-targets --all-features)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
cargo insta test --workspace --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE htui_sqlx_mod125"
```

`HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres` drives the Pg suites. The
compose `htui` DB is empty; the prepare DB above is separate.

---

## 6. Open risks (no plan decision involved)

| # | Risk | Mitigation |
|---|---|---|
| R1 | M2 must keep the lock order this blueprint sets. A `stop_workspace` that also writes runs would lock `workspace_stop` before run and step, the reverse of `claim_run` and `adopt_runs` (run/step, then `workspace_stop`). | Note for M2's plan: M2 stops runs through `stop_run`, in its own transaction, never inside `stop_workspace`'s. |
| R2 | `stop_run` stops only the steps running when it locks them. A step the caller starts at the same moment stays `running` under a `stopped` run. | Rustdoc puts the duty on the caller (M2 quiesces the walk first). M2 adds a test. |
| R3 | Pg `adopt_runs` defers a stopped-workspace run whose step is locked. A permanently stuck locker would leave it `running` with a lapsed lease indefinitely. | It is retried every sweep. A locker never outlives its transaction. Pinned by `adopt_runs_defers_a_stopped_run_whose_step_is_held`. |
| R4 | A cancel left `pending` on a run D6 settles as `stopped` stays pending until M3 (DV-2). | `cancels_first` applies cancels on `running` runs before adoption, which narrows the window. M3 applies the rest. |
| R5 | `promote_step` in both stores refuses only terminal runs. Only `promote_enabled` keeps a stopped run's failed step from promotion. | Low, because nothing stops a run in M1. M3 adds a store refusal when it wires `stopped`. |
| R6 | `take_lease` refuses `stopped`. M3's cancel and resume both start with a lease. | M3 decides between a widened take and a resume-specific claim. |
| R7 | The sweep claim per queued run per tick while stopped (D5's accepted cost). | As plan. |
| R8 | Worktree disk ~10 G each for T3 ∥ T4. | `df -h .` first; fall back to serial. |

---

## 7. Deviations needing a maintainer call

| DV | Plan says | Tree / problem | Recommendation | If overruled |
|---|---|---|---|---|
| **DV-1** | D2: `pg/read.rs:1417` and `cache/read.rs:1480` gain `'stopped'`. | Those are `active_runs`, the top-bar count D2 excludes (F1). | Keep three statuses in both; Mem `active_runs` uses `on_top_bar()`; split the pin (F2). | Add `'stopped'` to both, and make Mem `active_runs` use `is_active()`: the top bar then counts stopped runs, contradicting D2 and R-TUI-18. |
| **DV-2** | D2: `pg/relay.rs:544` gains `'stopped'`. | It would hand every poll a pending cancel on a stopped run that `cancel_enabled` refuses as transient: an endless retry. The Mem twin is unlisted (F3). | Leave both `pending_commands` twins unchanged in M1; M3 applies such cancels when it wires stopped-run cancel. | Add `'stopped'` there and at `mem.rs:7740`, **and** make `cancel_run` resolve a `Stopped` refusal as `refused` (a new arm before `runtime.rs:3122`). |
| **DV-3** | T3 plumbs `queue_item`'s workspace; T3 ∥ T4 disjoint. | 63 positional call sites, most in T4's files (F6). | Move the signature change and the 63 sites into T2's commit 2; T3 keeps `StoreRequest`/`serve_queue`/`on_queue`. | T3 then T4 serially on one tree (worktree parallelism lost). |
| **DV-4** | D7: `stop_run(run, owner, at) -> bool`. | No column takes `at`; `release_lease` has none (F7). | `stop_run(run, owner) -> Result<bool>`. | Keep `at`, documented as stamping nothing. |
| **DV-5** | D5: `admit` unchanged. | PRD metric "zero runs admitted … in a stopped workspace": the runner still creates queued runs there (F8). | Accept for M1 (they cap `free_slots` and cannot start); M2 adds a `WorkerHost` read and skips entries of a stopped workspace in `admit`. | Add `WorkerHost::workspace_stop` now (Backend + PgStore inherent reads) and filter `admissible`. That contradicts D4's "no `WorkerHost` verb in M1". |
| **DV-6** | D3 backfills `run.workspace_id` only. | Entries queued before 0019 would admit `NULL`-workspace runs that a stop never gates, even though their project sits in exactly one workspace. | Backfill `queue_entry.workspace_id` with the same rule (the marked statement in T1.1). | Delete that one `UPDATE queue_entry` statement. |
