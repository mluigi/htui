# Blueprint: MOD-12 milestone 3: Queue overlay

**Plan**: `.claude/plans/mod-12-m3-queue-overlay.plan.md`. Its D1 to D10, T1 to T6 and waves are binding
(maintainer CONFIRM, 2026-10-07, L4: a stalled batch closes). Every file:line below was re-read on
`hr/MOD-12` at `925c9e9f`.
**PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md` (M3 row; metric "escalations visible").
**Seams this builds on**: M1 blueprint §B.3 (queue store surface), §B.4 (`admit`); M2 blueprint §B.3
(the spend gate inside `admit`), §B.5 (a TUI request family and its serve module).
**When the tree and the plan disagree, the tree wins.** Each case is in §F **Amendments** with its
evidence. Where the plan leaves a detail open, this blueprint settles it and says so. **VERIFY** marks
a point the implementer must check before relying on it.

Conventions inherited unchanged (M2 blueprint header, re-checked):
- MSRV 1.98, edition 2024, workspace lints at `-D warnings`. `#![warn(missing_docs)]` in `htui-core`,
  `htui-store`, `htui-worker`: every new `pub` item carries a doc comment.
- Unmirrored reads and writes are inherent on `MemStore`/`PgStore`, dispatched by `Backend`, whose
  `Offline` arm answers `orchestration_offline()` (`backend.rs:811-813`, `Unreachable(DATABASE_UNREACHABLE)`).
- `.sqlx` is regenerated inside `crates/htui-store` against a **migrated scratch DB**
  (`docs/hr-sandbox.md:196-210`): `CREATE DATABASE htui_sqlx`, `cargo sqlx migrate run --source
  migrations`, `cargo sqlx prepare -- --all-targets --all-features`. A byte-identical literal reuses
  its entry; prepare deletes entries no literal uses any more, and the deletion is committed too.
- `htui` integration tests need `--all-features` (testkit) and `--test-threads=1`; every gate runs
  `--no-fail-fast` and greps `SIGABRT|overflowed|test result: FAILED`.

---

## A. Per-file change table

| # | File | Task | What changes (and what must **not**) |
|---|---|---|---|
| 1 | `crates/htui-core/src/model/queue.rs` | T1 | Append before `#[cfg(test)]` (`:179`): `QueueMove`, `moved_order`, `QueueRunFact`, `QueueRow`, `Hold`, `Wait`, `Escalation`, `EntryState`, `LiveFacts`, `BatchFigures`, `QueueOverview`, `classify_entry`, their `Display`s; table tests. **Also** the `BatchClose::Drained` doc (`:20-21`, §F-3). `admission_order`, `batch_budget`, `free_slots` untouched |
| 2 | `crates/htui-core/src/model/mod.rs` | T1 | Extend `pub use queue::{…}` (`:150-153`) |
| 3 | `crates/htui-core/src/store/mem.rs` | T2, T3 | T2: `close_drained_batch` drops the entry clause (`:1074-1106`), its unit test flips (`:14664`, assertion `:14708`). T3: `State::queue_sorted` (D2 order), `queue_entries` sorts through it (`:964-988`), `queue_rows`, `move_queue_entry`; unit tests in the M1 block (`:14401+`) |
| 4 | `crates/htui-store/src/pg/write.rs` | T2, T3 | T2: the drain UPDATE (`:8041-8075`). T3: `move_queue_entry` after `dequeue_item` (`:7926-7939`) |
| 5 | `crates/htui-store/src/pg/read.rs` | T3 | `queue_entries` ORDER BY + doc (`:2224-2250`); `queue_rows` after it |
| 6 | `crates/htui-store/src/backend.rs` | T2, T3 | T2: `close_drained_batch` doc (`:707-708`, §F-2). T3: `queue_rows` after `queue_entries` (`:627-639`, doc amended to D2), `move_queue_entry` after `dequeue_item` (`:619`) |
| 7 | `crates/htui-store/tests/pg_criteria.rs` | T2, T3 | T2: restructure the drain half of `close_batch_cancels_its_queued_runs_and_the_drain_closes_only_its_batch` (`:8120`, `:8270-8320`). T3: amend `queue_surface_answers_alike_on_both_stores` (`:7665`, §F-5); three new parity tests |
| 8 | `crates/htui-store/.sqlx/` | T2, T3 | T2: +1 −1 (drain). T3: +4 −1 (`queue_entries` new literal, `queue_rows`, move lock, move update); the move's ordered read reuses the new `queue_entries` literal byte for byte |
| 9 | `crates/htui-worker/src/runtime.rs` | T2 | `admit` (`:2169-2346`) stall closes (§B.2.2), its doc (`:2149-2168`); `drain` (`:2350-2372`) takes a `Drain` cause; unit test `an_entry_whose_project_is_gone_is_not_admitted` (`:4102`) amended (§F-1) |
| 10 | `crates/htui-worker/tests/auto_queue.rs` | T2 | Six L4 tests (§C.2); three M1 tests amended (§F-1); one M2 doc (`:1539-1540`) |
| 11 | `crates/htui/src/queue_overview.rs` | T4 | CREATE: `overview(backend)` serve read + tests (§B.4.2) |
| 12 | `crates/htui/src/lib.rs` | T4 | `pub mod queue_overview;` between `qdrant_settings_info` (`:36`) and `queue_settings` (`:37`) |
| 13 | `crates/htui/src/store_worker.rs` | T4 | Two requests after `PauseQueue` (`:382`); `QUEUE_REQUEST_NAMES` 5 → 7 (`:94-102`); two `name()` arms (`:1175-1180`); `StoreReply::QueueOverview` after `QueueWritten` (`:1589-1594`); `QueueWrite::Moved` (`:1717-1745`); `QueueView.entries` doc (`:1706`); `try_serve` arms (`:2290-2296`); `serve_queue` arm (`:2341-2410`) and its doc; the names test (`:5126-5146`) |
| 14 | `crates/htui/src/ui/tabs/backlog/mod.rs` | T4 | Two wildcard-free `QueueWrite` matches: `on_queue_written` (`:477-489`), `queue_sentence` (`:1001-1033`); `queue_sentence_covers_every_write` (`:1069`) gains the two `Moved` rows. Nothing else; the `Q`/`P` sentences are unchanged (D10) |
| 15 | `crates/htui/src/ui/overlay/queue.rs` + `src/ui/overlay/snapshots/htui__ui__overlay__queue__tests__*.snap` | T5 | CREATE: `QueueOverlay` (§B.5.2) and its Bench tests |
| 16 | `crates/htui/src/ui/overlay/mod.rs` | T5 | `pub mod queue;` + `pub use queue::QueueOverlay;` |
| 17 | `crates/htui/src/ui/overlay/registry.rs` | T5 | `Overlay::refresh` default no-op after `on_reply` (`:51-52`) |
| 18 | `crates/htui/src/app/update.rs` | T5 | `on_tick` (`:126-138`) calls `refresh_top_overlay`; the new fn beside `refresh_active_tab` (`:141`); one unit test |
| 19 | `crates/htui/src/app/state.rs` | T5 | `is_offerable` (`:339-341`) gains `Act::Queue`. `every_global_act_is_fixed_mapped_or_offerable` (`:935`) covers it unchanged |
| 20 | `crates/htui/src/app/mod.rs` | T5 | Import (`:14`), register + offer after the waiting list (`:101-107`) |
| 21 | `crates/htui/src/keys/catalogue.rs` | T5 | `Act::Queue` after `Waiting` (`:108-109`); row after `waiting` (`:321`); test `ALL` (`:459-460`), `position` (`:506-507`, every later index +1), count 41 → 42 (`:557`) |
| 22 | `crates/htui/src/keys/hint.rs` | T5 | `bare` (`:216-218`) excludes `Act::Queue`; `FULL` (`:212-213`), `the_help_lines_follow_d8` (`:280-283`), `rows_break_between_entries_only` (`:326`) gain ` · Ctrl+q queue` (§F-6) |
| 23 | `crates/htui/src/keys/stack.rs` | T5 | `the_base_stack_resolves_…` (`:149`) gains `ctrl-q` → `[Act::Queue]` (§F-6) |
| 24 | `crates/htui/src/keys/print.rs` | T5 | `DEFAULT_HEAD` (`:92`) gains the `queue` line; `take(26)` → `take(27)` (`:100`); the round-trip test's `ctrl-q` → `ctrl-x` (`:119`, `:127`) (§F-6) |
| 25 | `crates/htui/tests/keys.rs` | T5 | `ctrl_c_quits_over_every_modal_overlay` (`:143`) gains `("ctrl-q", QueueOverlay::ID)`; import |
| 26 | `crates/htui/tests/keys_file.rs`, `crates/htui/tests/fixtures/keys/valid.toml` | T5 | `quit = ["ctrl-q"]` → `["ctrl-x"]` in the fixture and its three assertions (`:84`, `:346`, `:371`) (§F-6) |
| 27 | `crates/htui/tests/queue_overlay.rs` (+ `tests/snapshots/queue_overlay__*.snap`) | T5 | CREATE: end-to-end over `Harness` (§C.5) |
| 28 | `docs/ANA-2.md`, PRD, `HANDOFF.md`, `docs/decisions/mod/mod-12.md` | T6 | §B.6 |

**Waves are the plan's**: T1 ∥ T2, then T3, T4, T5, T6. The widening in §F keeps T1 ∥ T2 disjoint:
T1 = {`model/queue.rs`, `model/mod.rs`}; T2 = {`mem.rs`, `pg/write.rs`, `backend.rs`, `pg_criteria.rs`,
`.sqlx`, `runtime.rs`, `auto_queue.rs`}. `backend.rs` moves into T2 as well as T3 (T2 ∩ T3 was already
non-empty, so T3 stays after T2). Every T5 addition is an `htui` file no other task touches.

---

## B. Interfaces and SQL, exactly

### B.1 T1: the pure queue model (`crates/htui-core/src/model/queue.rs`)

Imports become:
```rust
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::{BoxSettings, DEFAULT_MAX_CONCURRENT_ITEMS};
use crate::model::ids::{BatchId, BoxId, ItemId, ProjectId, RunId, StepId, UserId};
use crate::model::item::{ItemSummary, Status};
use crate::model::quota::CapError;
use crate::model::run::{RunMode, RunStatus};
```
(**VERIFY** the module paths of `Status` (`model/item.rs:8`), `RunStatus`/`RunMode` (`model/run.rs`) and
`CapError` (`model/quota.rs:298`); adjust to whatever `model/mod.rs` re-exports if a path is private.)

`BatchClose::Drained`'s doc (`:20-21`) becomes: "Nothing was admissible and no auto run of the batch
was live: the queue emptied (M1 D3) or stalled (M3 D4, L4)." (doc only; §F-3).

```rust
/// MOD-12 M3 D3: which way [`moved_order`] (and the stores' `move_queue_entry`) moves an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMove {
    /// One place towards the head of the queue.
    Up,
    /// One place towards its tail.
    Down,
}

/// MOD-12 M3 D3: `order` with `item` swapped with its neighbour `to`; `None` when `item` is not
/// in `order`, or is already at that end. Both stores call it, so Mem and Pg move alike.
#[must_use]
pub fn moved_order(order: &[ItemId], item: ItemId, to: QueueMove) -> Option<Vec<ItemId>> {
    let at = order.iter().position(|id| *id == item)?;
    let other = match to {
        QueueMove::Up => at.checked_sub(1)?,
        QueueMove::Down => Some(at + 1).filter(|next| *next < order.len())?,
    };
    let mut moved = order.to_vec();
    moved.swap(at, other);
    Some(moved)
}

/// MOD-12 M3 D5, D6: the item's latest graph run, as [`QueueRow`] carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRunFact {
    /// `run.id`.
    pub id: RunId,
    /// `run.status`.
    pub status: RunStatus,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.target_box_id`.
    pub target_box_id: BoxId,
    /// That box's `hostname`; `None` when no box row answers (Mem only; Pg's FK guarantees one).
    pub target_hostname: Option<String>,
    /// `run.failure`.
    pub failure: Option<String>,
    /// The run's first step `awaiting_approval` by `(position, attempt, fanout_index)`: a parked
    /// gate. `None` for a judge park, which parks no step (`engine.rs` `park_selection`).
    pub parked_step: Option<StepId>,
}

/// MOD-12 M3 D5, D6: one queue entry with what the overlay classifies it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueRow {
    /// The `queue_entry` row (with its item's project).
    pub entry: QueueEntry,
    /// `item.key`, e.g. `FIX-3`.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status`.
    pub status: Status,
    /// `item.priority` (D2's tie-break; never written by a move).
    pub priority: i16,
    /// `item.created_at` (D2's second tie-break).
    pub created_at: DateTime<Utc>,
    /// The item's latest `kind = 'graph'` run by `(queued_at, id)`.
    pub latest_run: Option<QueueRunFact>,
    /// The body of the item's latest note by `(created_at, id)`, verbatim.
    pub latest_note: Option<String>,
    /// The keys of its live `blocked_by` targets that are not `done`/`closed`, in byte order.
    pub open_blockers: Vec<String>,
}

/// MOD-12 M3 D5: why a ready entry is not admitted this batch: the runner's own three refusals
/// in `admit` (M2 D4, review R1 L3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// [`batch_budget`] refused it.
    Budget(BatchStop),
    /// Its project's caps do not parse ([`ProjectCaps::from_settings`](crate::model::ProjectCaps)).
    BadCap(CapError),
    /// Its project read as absent (a delete racing the read).
    ProjectGone,
}

/// MOD-12 M3 D5: why an entry waits without needing a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// Open `blocked_by` targets, by key.
    BlockedBy(Vec<String>),
    /// A run of it was cancelled under the open batch (review H1).
    CancelledInBatch,
    /// No batch is open.
    Paused,
    /// Not ready for a reason the list above does not name: the item's status (§F-7).
    NotReady(Status),
}

/// MOD-12 M3 D5 (PRD hypothesis): why an entry needs a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Escalation {
    /// A hard gate parked a step of the run.
    HardGateParked {
        /// The parked run.
        run: RunId,
        /// The parked step.
        step: StepId,
    },
    /// A fan-out awaits a human selection: the run parked with no step parked.
    JudgeUndecided {
        /// The parked run.
        run: RunId,
        /// The item's latest note (the selection note, `engine.rs:4670`'s).
        note: Option<String>,
    },
    /// The review loop gave up: item `blocked`, run `awaiting_approval` (`gate.rs:1050`).
    ReviewLoopExhausted {
        /// The parked run.
        run: RunId,
        /// The item's latest note.
        note: Option<String>,
    },
    /// The item is `blocked` with no parked run (a walk refusal, or by hand).
    Blocked {
        /// Its latest run, if any (for `Enter`).
        run: Option<RunId>,
        /// The item's latest note.
        note: Option<String>,
    },
    /// The item is `failed`.
    Failed {
        /// Its latest run, if any (for `Enter`).
        run: Option<RunId>,
        /// That run's `failure`.
        failure: Option<String>,
    },
    /// This box lacks tags the item requires (`R-ORCH-10`).
    MissingTags(Vec<String>),
}

/// MOD-12 M3 D5: what one entry is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryState {
    /// A `queued` or `running` run targeted at this box.
    Running {
        /// The run.
        run: RunId,
        /// `Queued` (admitted, not claimed yet) or `Running`.
        status: RunStatus,
    },
    /// A `queued` or `running` run targeted at another box (MOD-43 owns choosing; M3 reports).
    Elsewhere {
        /// The run.
        run: RunId,
        /// The target box's hostname.
        hostname: Option<String>,
        /// `Queued` or `Running`.
        status: RunStatus,
    },
    /// Ready and admissible: the runner admits it at the next free slot.
    Next,
    /// Ready, but this batch will not admit it.
    Held(Hold),
    /// Not ready; nothing for a person to do.
    Waiting(Wait),
    /// A person is needed.
    Escalated(Escalation),
}

impl EntryState {
    /// Whether the overlay draws the row in the warning style.
    #[must_use]
    pub const fn is_escalated(&self) -> bool {
        matches!(self, Self::Escalated(_))
    }

    /// The run and step `Enter` reveals (D8): `(None, None)` reveals the item.
    #[must_use]
    pub fn reveal(&self) -> (Option<RunId>, Option<StepId>)
    // Running{run}|Elsewhere{run} → (Some(run), None); HardGateParked{run,step} → (Some(run), Some(step));
    // JudgeUndecided{run}|ReviewLoopExhausted{run} → (Some(run), None);
    // Blocked{run}|Failed{run} → (run, None); everything else → (None, None).
}

/// MOD-12 M3 D5: the per-box, per-batch facts [`classify_entry`] reads, composed live by the
/// serve module from the runner's own reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveFacts {
    /// No batch is open.
    pub paused: bool,
    /// `ready_items` over the rows' projects on this box.
    pub ready: HashSet<ItemId>,
    /// `batch_cancelled_items` of the open batch; empty when paused.
    pub cancelled: HashSet<ItemId>,
    /// `missing_tags` of each `open` row that is not ready; empty lists are left out.
    pub missing_tags: HashMap<ItemId, Vec<String>>,
    /// The batch's [`Hold`] per project; empty when paused.
    pub holds: HashMap<ProjectId, Hold>,
}

/// MOD-12 M3 D5: the open batch as the overlay's header shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFigures {
    /// `queue_batch.id`.
    pub id: BatchId,
    /// `queue_batch.opened_at`.
    pub opened_at: DateTime<Utc>,
    /// `batch_spend`, USD micros; `None` when no step reported a cost.
    pub spent: Option<i64>,
}

/// MOD-12 M3 D5, D7: one box's queue for the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueOverview {
    /// This box.
    pub box_id: BoxId,
    /// The open batch; `None` is paused.
    pub batch: Option<BatchFigures>,
    /// `running` runs on the box plus `queued` runs targeted at it (`free_slots`' inputs).
    pub slots_used: usize,
    /// [`admission_limit`].
    pub slots_limit: u32,
    /// The rows in D2 order, each classified.
    pub rows: Vec<(QueueRow, EntryState)>,
    /// The memory backend (`htui --demo`), whose runtime never admits (M1 review L3).
    pub demo: bool,
}

/// MOD-12 M3 D5: what `row` is doing on box `here`, given `live`. First match wins (§B.1.1).
#[must_use]
pub fn classify_entry(row: &QueueRow, here: BoxId, live: &LiveFacts) -> EntryState
```

`model/mod.rs:150-153` becomes (rustfmt wraps it):
`pub use queue::{BatchClose, BatchFigures, BatchStop, EntryState, Escalation, Hold, LiveFacts, MIN_BUDGET_FOR_NEW_ATTEMPT, QueueBatch, QueueEntry, QueueMove, QueueOverview, QueueRow, QueueRunFact, Wait, admission_limit, admission_order, batch_budget, classify_entry, free_slots, min_budget_micros, moved_order};`
No name clashes (checked: no other `Wait`, `Hold`, `EntryState`, `Escalation`, `QueueMove`, `QueueRow`,
`QueueRunFact`, `LiveFacts`, `QueueOverview`, `BatchFigures` in the workspace).

#### B.1.1 `classify_entry`: the decision list

`live_run` = `row.latest_run` when its status is `Queued` or `Running`. `parked_run` = `row.latest_run`
when its status is `AwaitingApproval`. `note` = `row.latest_note.clone()`.

| # | Condition (first match wins) | Result |
|---|---|---|
| 1 | `live_run` with `target_box_id == here` | `Running { run, status }` |
| 2 | `live_run` on another box | `Elsewhere { run, hostname: target_hostname, status }` |
| 3 | `row.status == Failed` | `Escalated(Failed { run: latest_run.id, failure: latest_run.failure })` |
| 4 | `row.status == Blocked` and `parked_run` | `Escalated(ReviewLoopExhausted { run, note })` |
| 5 | `row.status == Blocked` | `Escalated(Blocked { run: latest_run.id, note })` |
| 6 | `row.status == AwaitingApproval` and `parked_run` with `parked_step: Some(step)` | `Escalated(HardGateParked { run, step })` |
| 7 | `row.status == AwaitingApproval` and `parked_run` (no parked step) | `Escalated(JudgeUndecided { run, note })` |
| 8 | `live.missing_tags[item]` non-empty | `Escalated(MissingTags(tags))` |
| 9 | `row.open_blockers` non-empty | `Waiting(BlockedBy(keys))` |
| 10 | `live.cancelled` holds the item | `Waiting(CancelledInBatch)` |
| 11 | `live.ready` lacks the item | `Waiting(NotReady(row.status))` |
| 12 | `live.paused` | `Waiting(Paused)` |
| 13 | `live.holds[row.entry.project_id]` | `Held(hold)` |
| 14 | otherwise | `Next` |

So "a live run (here or elsewhere)" beats everything (rows 1-2), and `Held` is reached only by a ready,
uncancelled entry of an open batch (rows 11-13), exactly the runner's `admissible` complement. Row 11
catches what the plan's list leaves unnamed: an `awaiting_approval` item with no parked run, a `queued`
or `in_progress` item with no live run, a `done`/`closed` entry the runner has not pruned yet.

#### B.1.2 `Display` sentences (exact)

`note_line(n)` is the note's first line (`n.lines().next().unwrap_or("")`): the overlay draws one row
per entry, and the Runs pane (`Enter`) holds the whole note. Text is otherwise verbatim, never parsed.

| Value | Sentence |
|---|---|
| `Running { status: Queued, .. }` | `admitted, waiting to be claimed` |
| `Running { .. }` (any other status) | `running here` |
| `Elsewhere { status: Queued, hostname, .. }` | `queued for {host}` |
| `Elsewhere { hostname, .. }` | `running on {host}` (`host` = hostname, else `another box`) |
| `Next` | `next to run` |
| `Held(h)` | `held: {h}` |
| `Hold::Budget(stop)` | `{stop}` (`batch cap reached (600 of 500 micros)` / `batch budget: 100 micros left, 200 required`) |
| `Hold::BadCap(err)` | `{err}` (`CapError`'s own sentence, `quota.rs:298-312`) |
| `Hold::ProjectGone` | `its project is gone` |
| `Waiting(w)` | `{w}` |
| `Wait::BlockedBy(keys)` | `waiting on {keys joined by ", "}` |
| `Wait::CancelledInBatch` | `cancelled in this batch; the next batch runs it` |
| `Wait::Paused` | `queue paused` |
| `Wait::NotReady(s)` | `not ready: item is {s}` (`s.as_str()`, e.g. `in_progress`) |
| `Escalated(e)` | `{e}` |
| `HardGateParked` | `hard gate parked` |
| `JudgeUndecided { note: Some(n) }` / `None` | `judge undecided (last note: {note_line(n)})` / `judge undecided` |
| `ReviewLoopExhausted { note }` | `review loop exhausted (last note: {…})` / `review loop exhausted` |
| `Blocked { note }` | `blocked (last note: {…})` / `blocked` |
| `Failed { failure: Some(f) }` / `None` | `failed: {note_line(f)}` / `failed` |
| `MissingTags(tags)` | `missing tags: {tags joined by ", "}` |

A walk-time batch-cap block therefore reads `blocked (last note: no_candidate_agent: phase `research`; … (batch cap reached (… of … micros)))`, its own note, as D5 requires.

### B.2 T2: L4, a stalled batch closes

#### B.2.1 The store predicate

**Pg** (`pg/write.rs:8041-8075`), the statement becomes (one new `.sqlx` file, the old one deleted):
```sql
UPDATE queue_batch b
   SET closed_at = $2, closed_reason = 'drained'
 WHERE b.id = $1 AND b.closed_at IS NULL
   AND NOT EXISTS (
           SELECT 1 FROM run r
            WHERE r.batch_id = b.id
              AND r.status IN ('queued', 'running', 'awaiting_approval'))
RETURNING b.id AS "id: BatchId", b.box_id AS "box_id: BoxId", b.opened_at,
          b.opened_by AS "opened_by: UserId", b.closed_at,
          b.closed_reason AS "closed_reason: BatchClose"
```
Doc: "MOD-12 D3 (review M1), M3 D4 (L4): the runner's close of exactly `batch`, only while it is open
and no run of its own is `queued`, `running` or `awaiting_approval`. Entries do not keep it open: the
runner calls it when nothing is admissible (an empty queue, or a stalled one). The re-check is the
UPDATE's own `WHERE`, so a resume that opened a new batch after the runner's reads is never closed by
it, and a run that committed before the statement keeps the batch open."

**Mem** (`mem.rs:1080-1106`): the closure drops the `queue_entries.values().any(…)` clause and the
now-unused `box_id` binding:
```rust
let row = state.queue_batches.get(&batch)?;
if row.closed_at.is_some()
    || state.run_batches.iter().filter(|(_, of)| **of == batch)
        .filter_map(|(run, _)| state.runs.get(run))
        .any(|row| row.status.is_active())
{
    return None;
}
```
`RunStatus::is_active` is `Queued | Running | AwaitingApproval` (`model/run.rs:54-56`), the SQL clause.
Same doc as Pg. **Backend** doc (`backend.rs:707-708`): "(open, no live run of its own; entries do not
count, M3 D4)". `WorkerHost::close_drained_batch`'s doc (`store/worker.rs:579`) names no predicate and
stays.

#### B.2.2 `admit` (`htui-worker/src/runtime.rs:2169-2346`): the control flow

Hard constraints: a tick with zero free slots never makes a stall close, and an enqueue refusal never
closes. Branches in order; **[CLOSE]** marks the only three places `drain` is called, **[RETURN]** a
return with the batch untouched:

1. `open_batch_of` → `Ok(None)`: `queue_reads_ok`, [RETURN]; `Err`: `queue_read_failed`, [RETURN].
2. `prune_finished_entries` `Err` → [RETURN].
3. `queue_entries` `Err` → [RETURN].
4. `entries.is_empty()` (`:2198`) → **[CLOSE 1]** `drain(ctx, &batch, Drain::Empty)`, return.
   *Unchanged M1 D3 drain, before any slot read* (§F-4).
5. `ready_items` `Err` → [RETURN]; `batch_cancelled_items` `Err` → [RETURN]; `ready` filtered (unchanged).
6. `let order = admission_order(&entries, &ready);` (`:2233`).
7. **Moved up** from `:2238-2251`, body unchanged: the `slots` block yields `(free, app)`; `Err` →
   `queue_read_failed("counting this box's free slots")`, [RETURN].
8. `free == 0` → `queue_reads_ok`, [RETURN]. **No close at zero free slots**, whatever `order` holds.
9. `order.is_empty()` (was `:2234`, before the slot read) → **[CLOSE 2]**
   `drain(ctx, &batch, Drain::Stalled)`, return. Nothing is ready: not ready, cancelled in this batch,
   or escalated.
10. `batch_spend` `Err` → [RETURN]; the caps loop (unchanged): a `project_settings` `Err` → [RETURN].
11. `queue_reads_ok()` (`:2306`, unchanged).
12. `admissible.is_empty()` (`:2307`) → **[CLOSE 3]** `drain(ctx, &batch, Drain::Stalled)`, return.
    The tick had free slots (step 8) and every ordered entry was stopped by `batch_budget`, a
    malformed cap or an absent project.
13. `Kit::read` `Err` (`:2310`) → warn, [RETURN] (no close: transient).
14. The enqueue loop is unchanged: `Ok` admits; `Err(Store(Constraint))` re-reads the open batch and
    [RETURN]s when it is gone; any other `Err` skips the entry. **No close anywhere in the loop**, even
    when every enqueue was refused: a refusal is transient (D4).

`drain` (`:2350-2372`) becomes:
```rust
/// Why `admit` closes its batch (MOD-12 M3 D4). Log text only: the store's predicate is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drain {
    /// No entry is left (M1 D3).
    Empty,
    /// Entries are left, but none is admissible this tick (L4).
    Stalled,
}

async fn drain<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: &TaskCtx<H, P>,
    batch: &htui_core::model::QueueBatch,
    why: Drain,
)
```
On `Ok(Some(_))`: `Empty` logs `tracing::info!(batch = %batch.id, "the queue drained")` (unchanged);
`Stalled` logs `tracing::info!(batch = %batch.id, "the queue stalled with nothing admissible; its batch closed")`.
`Ok(None)` (a live run of the batch, or a racing pause/resume) and `Err` behave as today.

`admit`'s doc (`:2149-2168`): replace "A batch whose every entry is stopped stays open and admits
nothing (no `drain`)" with "MOD-12 M3 D4 (L4): when the tick has a free slot and nothing is admissible
(no entry ready, or every ready entry stopped by the spend gate), the batch closes `drained` like an
empty one, unless a run of its own is live; a later `Q` then waits for `P`. A tick with no free slot,
a failed read and an enqueue refusal never close."

No new `WorkerHost` method, so `Failing` and the streak test's call list (`:4061-4070`) are unchanged
(the slot reads move above the order check, but their order relative to `ready_items`,
`batch_cancelled_items`, `batch_spend` and `project_settings` is the same).

### B.3 T3: store order, `queue_rows`, `move_queue_entry`

#### B.3.1 D2 order

**Pg `queue_entries`** (`pg/read.rs:2224-2250`), new literal (new `.sqlx`; the old one has no other
user — `queue_item`'s re-read is a different literal — so prepare deletes it):
```sql
SELECT e.item_id    AS "item_id: ItemId",
       i.project_id AS "project_id: ProjectId",
       e.box_id     AS "box_id: BoxId",
       e.position,
       e.queued_at,
       e.queued_by  AS "queued_by: UserId"
  FROM queue_entry e JOIN item i ON i.id = e.item_id
 WHERE e.box_id = $1
 ORDER BY e.position NULLS LAST, i.priority DESC, i.created_at, i.id
```
Doc: "MOD-12 D4, M3 D2: `box_id`'s queue entries in queue order, `position NULLS LAST, priority
DESC, created_at, id`: `admission_order`'s order over every entry, ready or not."

**Mem**: a `State` helper beside `open_batch_of`, used by `queue_entries`, `queue_rows` and
`move_queue_entry`:
```rust
/// MOD-12 M3 D2: `box_id`'s entries in queue order: `position` first (`None` last), then the
/// item's `priority DESC, created_at, id` (`ready_items`' order). `ItemId`'s `Ord` is uuid byte
/// order, Postgres' uuid order.
fn queue_sorted(&self, box_id: BoxId) -> Vec<QueueEntry> {
    let mut rows: Vec<QueueEntry> = self.queue_entries.values()
        .filter(|entry| entry.box_id == box_id).cloned().collect();
    rows.sort_by_cached_key(|entry| {
        let item = self.items.get(&entry.item_id);
        (
            entry.position.is_none(),
            entry.position,
            core::cmp::Reverse(item.map_or(0, |item| item.priority)),
            item.map(|item| item.created_at),
            entry.item_id,
        )
    });
    rows
}
```
`MemStore::queue_entries` becomes `Ok(self.read(|state| state.queue_sorted(box_id)))`, doc as Pg's.
`Backend::queue_entries` doc (`:627`) and `QueueView.entries` doc (`store_worker.rs:1706`, T4) follow.

#### B.3.2 `queue_rows`

```rust
/// MOD-12 M3 D6: `box_id`'s entries in queue order (D2), each with its item's key, title, status,
/// priority and `created_at`, its latest graph run (target hostname, parked step), its latest note
/// and its open `blocked_by` keys. One statement on Postgres.
/// # Errors
/// Whatever the driver reports, through [`map_sqlx`].
pub async fn queue_rows(&self, box_id: BoxId) -> Result<Vec<QueueRow>>
```
Pg (`pg/read.rs`, after `queue_entries`), `sqlx::query!` (the record is mapped by hand, `run_*`
columns into `QueueRunFact` when `run_id` is `Some`):
```sql
SELECT e.item_id         AS "item_id: ItemId",
       i.project_id      AS "project_id: ProjectId",
       e.box_id          AS "box_id: BoxId",
       e.position,
       e.queued_at,
       e.queued_by       AS "queued_by: UserId",
       i.key             AS "key!",
       i.title,
       i.status          AS "status: Status",
       i.priority,
       i.created_at,
       lr.id             AS "run_id?: RunId",
       lr.status         AS "run_status?: RunStatus",
       lr.mode           AS "run_mode?: RunMode",
       lr.target_box_id  AS "run_target?: BoxId",
       lr.hostname       AS "run_hostname?",
       lr.failure        AS "run_failure?",
       lr.parked_step    AS "run_parked_step?: StepId",
       ln.body           AS "note?",
       COALESCE(ob.keys, '{}'::text[]) AS "blockers!"
  FROM queue_entry e
  JOIN item i ON i.id = e.item_id
  LEFT JOIN LATERAL (
        SELECT r.id, r.status, r.mode, r.target_box_id, tb.hostname, r.failure,
               (SELECT s.id FROM run_step s
                 WHERE s.run_id = r.id AND s.status = 'awaiting_approval'
                 ORDER BY s.position, s.attempt, s.fanout_index
                 LIMIT 1) AS parked_step
          FROM run r
          LEFT JOIN box tb ON tb.id = r.target_box_id
         WHERE r.item_id = i.id AND r.kind = 'graph'
         ORDER BY r.queued_at DESC, r.id DESC
         LIMIT 1
       ) lr ON true
  LEFT JOIN LATERAL (
        SELECT n.body FROM item_note n
         WHERE n.item_id = i.id
         ORDER BY n.created_at DESC, n.id DESC
         LIMIT 1
       ) ln ON true
  LEFT JOIN LATERAL (
        SELECT array_agg(t.key ORDER BY t.key COLLATE "C") AS keys
          FROM item_link l JOIN item t ON t.id = l.to_item_id
         WHERE l.from_item_id = i.id AND l.kind = 'blocked_by'
           AND l.deleted_at IS NULL AND t.status NOT IN ('done', 'closed')
       ) ob ON true
 WHERE e.box_id = $1
 ORDER BY e.position NULLS LAST, i.priority DESC, i.created_at, i.id
```
Tables and columns checked against `0001_init.sql`: `item_link(from_item_id, to_item_id, kind,
deleted_at)` (`:357-367`, index `idx_item_link_to`), `item_note(id, item_id, body, created_at)`
(`:374-383`, index `idx_item_note_item (item_id, created_at)`), `run(item_id, kind, mode, status,
target_box_id, queued_at, failure)` (`:447-464`, index `idx_run_item (item_id, queued_at DESC)`),
`run_step(run_id, position, attempt, fanout_index, status)` (`:472-495`, the `UNIQUE` index serves the
parked-step probe), `box(hostname)` (`:53-74`). The blocker clause is `ready_items`' (`:2096-2099`)
verbatim, so "open blocker" and "not ready for a blocker" are one rule. `COLLATE "C"` matches Mem's
byte order (`missing_tags` does the same, `:2132`). **VERIFY** the decode overrides
(`htui_core::model::` paths as `ready_items` spells them, `:2081-2091`) and that `i.key` needs `!`
here as there.

Mem (`impl MemStore`, after `queue_entries`), one `read` closure over `state.queue_sorted(box_id)`:
- `latest_run`: `state.runs.values().filter(|r| r.item_id == Some(item) && r.kind == RunKind::Graph).max_by_key(|r| (r.queued_at, r.id))`;
  hostname `state.boxes.get(&run.target_box_id).map(|b| b.hostname.clone())`; parked step
  `state.steps.values().filter(|s| s.run_id == run.id && s.status == StepStatus::AwaitingApproval).min_by_key(|s| (s.position, s.attempt, s.fanout_index)).map(|s| s.id)`.
- `latest_note`: `state.notes.iter().filter(|n| n.item_id == item).max_by_key(|n| (n.created_at, n.id)).map(|n| n.body.clone())`.
- `open_blockers`: `state.links` with `deleted_at.is_none() && kind == LinkKind::BlockedBy && from_item_id == item`,
  target `items.get(&to)` with `!status.is_terminal()` (`State::is_ready`'s clause, `:1447-1457`), its
  `key`; sorted (`String`'s `Ord` is byte order) and deduplicated.
- An entry whose item is gone is skipped (Pg's inner join).

**Backend** (after `queue_entries`): `Memory` → Mem, `Online { pg, .. }` → Pg, `Offline` →
`Err(orchestration_offline())`; doc in the shape of its neighbours.

#### B.3.3 `move_queue_entry`

```rust
/// MOD-12 M3 D3: moves `item` one place `to` in `box_id`'s queue, atomically. The first move of a
/// queue writes `position = 1..n` over every entry in the current D2 order, so an entry queued later
/// (`NULL`) goes after them. `false`, writing nothing, when `item` is not in `box_id`'s queue or is
/// already at that end. Never touches `item.priority`.
/// # Errors
/// Whatever the driver reports, through [`map_sqlx`] (Pg); never (Mem).
pub async fn move_queue_entry(&self, box_id: BoxId, item: ItemId, to: QueueMove) -> Result<bool>
```
**Pg** (`pg/write.rs`, after `dequeue_item`), one transaction, three statements:
1. Lock the box's entries (no `ORDER BY`: §F-12):
   ```sql
   SELECT item_id FROM queue_entry WHERE box_id = $1 FOR UPDATE
   ```
   `query_scalar!(…).fetch_all(&mut *tx)`, result discarded.
2. Re-read in D2 order with the **new `queue_entries` literal byte for byte** (B.3.1), on `&mut *tx`:
   under `READ COMMITTED` this statement's snapshot sees every commit that the lock waited for.
3. `let Some(order) = moved_order(&ids, item, to) else { return Ok(false) }` (the dropped `tx` rolls
   back), then:
   ```sql
   UPDATE queue_entry e
      SET position = v.position
     FROM UNNEST($2::uuid[], $3::int4[]) AS v(item_id, position)
    WHERE e.box_id = $1 AND e.item_id = v.item_id
   ```
   `$2` the moved order's uuids, `$3` `1..=n` as `i32` (`i32::try_from(index + 1)`; a queue past
   `i32::MAX` entries is not a case). `tx.commit()`, `Ok(true)`.
Two new `.sqlx` files (lock, update). A concurrent `queue_item` inserts `NULL`, which sorts after the
written positions; a concurrent `dequeue_item` waits on the row lock.

**Mem** (after `dequeue_item`, `:960`): one `write` closure: `let order: Vec<ItemId> =
state.queue_sorted(box_id).iter().map(|e| e.item_id).collect();`, `moved_order`, then each entry's
`position = Some(index + 1)`; `false` when `moved_order` is `None`.

**Backend**: dispatch as `queue_rows`, offline refusal.

### B.4 T4: requests and the serve module

#### B.4.1 `crates/htui/src/store_worker.rs`

```rust
/// [`StoreRequest::name`] of the seven queue requests (MOD-12 D9, M3 D7), in variant order: the
/// Backlog matches a queue request's `Failed` by these, and the queue overlay by its two.
pub const QUEUE_REQUEST_NAMES: [&str; 7] = [
    "queue_state",
    "queue_item",
    "dequeue_item",
    "resume_queue",
    "pause_queue",
    "queue_overview",
    "move_queue_entry",
];
```
Requests, after `PauseQueue` (`:382`):
```rust
    /// MOD-12 M3 D7: this box's queue with every entry's state, for the queue overlay. Answered
    /// with [`StoreReply::QueueOverview`]; offline `DATABASE_UNREACHABLE`.
    QueueOverview,
    /// MOD-12 M3 D3: move `item` one place `to` in this box's queue (`queue_entry.position`; the
    /// item's priority is untouched). Answered with [`StoreReply::QueueWritten`] (`Moved`).
    MoveQueueEntry {
        /// The item.
        item: ItemId,
        /// Which way.
        to: QueueMove,
    },
```
`name()` (`:1175-1180`): the comment becomes "The seven of `QUEUE_REQUEST_NAMES`, in that order
(MOD-12 D9, M3 D7)", plus `Self::QueueOverview => "queue_overview"`,
`Self::MoveQueueEntry { .. } => "move_queue_entry"`.

Reply, after `QueueWritten` (`:1589-1594`), boxed as `QueueSettings` is (`:1690`):
```rust
    /// Answer to [`StoreRequest::QueueOverview`] (MOD-12 M3 D7).
    QueueOverview(Box<QueueOverview>),
```
`QueueWrite` (`:1717-1745`) gains, last:
```rust
    /// MOD-12 M3 D3: a move; `moved` is `false` when the entry was already at that end or not in
    /// this box's queue, and nothing was written.
    Moved {
        /// Whether the order changed.
        moved: bool,
    },
```
`QueueView.entries` doc (`:1706`): "The queued items in queue order (M3 D2)". Imports: `QueueMove`,
`QueueOverview` from `htui_core::model`; `crate::queue_overview`.

`try_serve` (`:2290-2296`): the or-arm's comment says "six", and it gains
`| StoreRequest::MoveQueueEntry { .. }`; a new arm beside it:
```rust
        // MOD-12 M3 D7: the overlay's read composes the runner's own reads (`queue_overview`).
        StoreRequest::QueueOverview => {
            StoreReply::QueueOverview(Box::new(queue_overview::overview(backend).await?))
        }
```
`serve_queue` (`:2341-2410`) gains, before `other =>`:
```rust
        StoreRequest::MoveQueueEntry { item, to } => QueueWrite::Moved {
            moved: backend.move_queue_entry(box_id, *item, *to).await?,
        },
```
and its doc says "the six queue writes and reads". No other exhaustive `StoreRequest`/`StoreReply`
match exists outside this file (checked: M2's three variants touched only `name()` and `try_serve`).

**Backlog** (`backlog/mod.rs`): `on_queue_written` (`:477-489`): the second arm becomes
`QueueWrite::Resumed { .. } | QueueWrite::Paused { .. } | QueueWrite::Moved { .. } => String::new()`.
`queue_sentence` (`:1001-1033`) gains, last:
```rust
        QueueWrite::Moved { moved: true } => format!("queue reordered ({n} in queue)"),
        QueueWrite::Moved { moved: false } => "already at that end of the queue".to_owned(),
```
The Backlog never sends `MoveQueueEntry` and replies are addressed by origin, so neither arm fires in
practice; they exist for exhaustivity and keep every Backlog sentence unchanged (D7). Its
`Failed { request } if QUEUE_REQUEST_NAMES.contains(request)` arm (`:801`) only ever sees its own
origin's failures, so the two new names change nothing there.

#### B.4.2 `crates/htui/src/queue_overview.rs` (CREATE)

```rust
//! The queue overlay's read (MOD-12 M3 D7): one `QueueOverview` composed from the runner's own
//! reads (`ready_items`, `batch_cancelled_items`, `batch_spend`, `batch_budget`, `admission_limit`),
//! so the overlay says what `admit` would do. Classification is `htui_core::model::classify_entry`.

/// [`StoreRequest::QueueOverview`]'s answer.
///
/// # Errors
/// `NotFound` "this box" before registration; offline, `Unreachable(DATABASE_UNREACHABLE)` from
/// `queue_rows`, the first queue read; otherwise whatever a read reports.
pub async fn overview(backend: &Backend) -> Result<QueueOverview>
```
Order of reads (an offline backend fails at step 2, before any other read):
1. `box_info()` → `info` (`NotFound { entity: "box", id: "this box" }` when `None`, `serve_queue`'s).
2. `rows = queue_rows(info.box_id)`.
3. `batch = open_batch_of(box)`; `cancelled = batch_cancelled_items(b.id)` when open, else empty.
4. `scope = Scope { workspace_id: WorkspaceId::default(), project_ids: <the rows' projects, first-seen order> }`
   (the runner's placeholder scope, `runtime.rs:2208-2211`); `ready = ready_items(&scope, box)` into a
   `HashSet`; **minus `cancelled`**, as `admit` filters it.
5. For each row with `status == Open` not in `ready`: `missing_tags(item, box)`; `NotFound` (a delete
   racing the read) reads as no tags; non-empty lists go into `LiveFacts::missing_tags`.
6. `app = app_settings()`; `slots_used = running_runs_on_box(box) + queued_runs_on_box(box).len()`;
   `slots_limit = admission_limit(&info.settings, &app)`.
7. When a batch is open: `spent = batch_spend(b.id)`, `min = min_budget_micros(&app)`; for each distinct
   project of the rows that are ready (after step 4): `project_settings(p)` → `None` is
   `Hold::ProjectGone`; `ProjectCaps::from_settings` `Err(e)` is `Hold::BadCap(e)`;
   `batch_budget(spent, caps.batch_micros, min)` `Err(stop)` is `Hold::Budget(stop)`; `Ok` is no hold.
8. `live = LiveFacts { paused: batch.is_none(), ready, cancelled, missing_tags, holds }`; each row is
   paired with `classify_entry(&row, box, &live)`.
9. `QueueOverview { box_id, batch: batch.map(|b| BatchFigures { id: b.id, opened_at: b.opened_at, spent }), slots_used, slots_limit, rows, demo: matches!(backend, Backend::Memory(_)) }`.

`missing_tags` is called once per non-ready `open` row (accepted N+1, D6).

### B.5 T5: the overlay and its wiring

#### B.5.1 Refresh hook

`registry.rs`, in `trait Overlay` after `on_reply` (`:51-52`):
```rust
    /// The shell's refresh tick reached this overlay, the top one (MOD-12 M3 D9): it may request a
    /// re-read through `ctx`. Every other overlay keeps this no-op.
    fn refresh(&mut self, _ctx: &mut Ctx<'_>) {}
```
`update.rs::on_tick` (`:126-138`), inside the refresh branch, after `self.refresh_active_tab();`:
`self.refresh_top_overlay();`. New, beside `refresh_active_tab` (`:141`), the same borrow split as
`on_key`'s overlay branch (`state.rs:725-756`):
```rust
    /// MOD-12 M3 D9: the top overlay's refresh hook, with a [`Ctx`] addressed as that overlay, then
    /// what it emitted drained. Overlays below the top are not ticked.
    fn refresh_top_overlay(&mut self) {
        let Some(id) = self.overlays.top().map(Overlay::id) else { return };
        let origin = Origin::Overlay(id);
        {
            let Self { scope, projects, top_bar, keymap, keys, theme, emit, overlays, .. } = self;
            let Some(top) = overlays.top_mut() else { return };
            let mut ctx = Ctx::new(scope, projects, top_bar, keymap, theme, origin.clone(), emit)
                .with_keys(keys);
            top.refresh(&mut ctx);
        }
        self.drain(&origin);
    }
```

#### B.5.2 `crates/htui/src/ui/overlay/queue.rs` (CREATE)

```rust
//! The queue overlay (MOD-12 M3 D8, `R-TUI-1`): this box's queue in order, each entry's state, and
//! the writes that steer it: reorder, pause/resume, dequeue. `Enter` reveals the row's run or item.

/// Lists this box's queue and steers it.
#[derive(Debug)]
pub struct QueueOverlay {
    /// The last `QueueOverview`; `None` before the first.
    overview: Option<QueueOverview>,
    /// The last failed read's message, shown instead of the rows.
    failure: Option<String>,
    /// The highlighted row's index, the fallback when the anchor's row is gone.
    cursor: usize,
    /// The highlighted row's item, re-found on every reply (a move or a re-sort keeps it).
    anchor: Option<ItemId>,
    /// A `QueueOverview` is in flight: `refresh` asks for none. `true` from `new`, because the
    /// shell sends `wants_requests`' read as it pushes the overlay.
    in_flight: bool,
}

impl QueueOverlay {
    /// The factory's and `Act::Queue`'s id.
    pub const ID: OverlayId = OverlayId("queue");
    /// Empty, its first read in flight.
    #[must_use]
    pub fn new() -> Self
}
```
Trait impl: `title()` `"Queue"`; `is_modal()` `true`; `wants_requests` →
`vec![StoreRequest::QueueOverview]`.

`on_reply`:
- `StoreReply::QueueOverview(view)` → store it, `failure = None`, `in_flight = false`, re-find the
  anchor (index of `anchor` in the rows, else `cursor.min(len - 1)`; the anchor is set to that row).
- `StoreReply::QueueWritten { .. }` → `ctx.request(StoreRequest::QueueOverview)`, `in_flight = true`.
- `StoreReply::Failed { request, message }` with `request == "queue_overview"` → `failure =
  Some(message)`, `in_flight = false`; with any other `QUEUE_REQUEST_NAMES` name → re-request the
  overview as above. (`App::on_reply` already put the failure on the status line.)

`refresh`: `if !self.in_flight { self.in_flight = true; ctx.request(StoreRequest::QueueOverview); }`.

`on_key` (matching `key.code`; `J`/`K`/`P`/`Q` are the capitals crossterm reports with Shift):

| Key | Effect | Returns |
|---|---|---|
| `j` / `Down` | cursor + 1 (clamped), anchor follows | `Consumed` |
| `k` / `Up` | cursor − 1 (saturating), anchor follows | `Consumed` |
| `J` | `MoveQueueEntry { item: anchor, to: QueueMove::Down }`; anchor kept on the item | `Consumed` |
| `K` | `MoveQueueEntry { item, to: QueueMove::Up }` | `Consumed` |
| `P` | `PauseQueue` when `overview.batch.is_some()`, else `ResumeQueue`; nothing before the first reply | `Consumed` |
| `Q` | `DequeueItem { item }` of the cursor row | `Consumed` |
| `Enter` | `Action::Overlay(OverlayAction::Close)` then `Action::Reveal(…)`: `(run, step) = state.reveal()`; `run.is_some()` → `RevealTarget::Step { item, key, run, step }`, else `RevealTarget::Item { id: item, key }` (`WaitingList::enter`'s order, `waiting_list.rs:115-126`) | `Consumed` |
| anything else (`Esc` included) | `Pass`: `Esc` reaches the wildcard close; the rest is swallowed by `is_modal` | `Pass` |

Keys on an empty or unread list do nothing and are `Consumed`. Lower-case `q` is **not** handled, so it
is swallowed (the overlay stack offers `Esc` and help only, `keys/stack.rs:76-79`): it never quits from
inside the overlay.

**Layout** (`WaitingList::render`'s sizing, `waiting_list.rs:305-332`: natural width, clamped to
`area.width - 4` and `area.height - 2`, centred, `Clear`, `Block` with ` Queue ` in `theme.title`):
```
{NO_CURSOR}{header}
(blank)
{marker}{key, padded}{GAP}{title, fit to its column}{GAP}{sentence, clipped}
… one line per row, windowed to keep the cursor drawn …
(blank)
{NO_CURSOR}j/k move · J/K reorder · P pause/resume · Q dequeue · Enter open · Esc close
```
Constants: `CURSOR = "> "`, `NO_CURSOR = "  "`, `GAP = "  "`, `CHROME = 3`, `TITLE_MAX = 32` (the title
column is `min(widest title, TITLE_MAX)` cells), `HINT` as the last line above,
`READING = "reading the queue"`, `EMPTY = "the queue is empty"`,
`UNAVAILABLE = "the queue is unavailable: "` (followed by the failure message, on the row line).

Header (exact; `·` is U+00B7; `HH:MM` is `opened_at.format("%H:%M")` in **UTC**, as
`settings/connection.rs:524-528` formats, so snapshots do not depend on the host's zone):
- open batch: `queue: running · batch since {HH:MM} · {spent} · {used}/{limit} slots`, `{spent}` =
  `format!("{} spent", format_usd(m))` for `Some(m)` (`$1.20 spent`), `no spend yet` for `None`;
- no batch: `queue: paused · {used}/{limit} slots`;
- `demo`: `queue: running · demo: nothing is admitted` or `queue: paused · demo: nothing is admitted`
  (the text of the Backlog's private `DEMO_NOTHING_ADMITTED`, `backlog/mod.rs:996`, copied as the
  overlay's own constant so `backlog/mod.rs` gains no T5 edit).

Row style: the selected row `theme.accent`, else an escalated row `theme.warning`, else `theme.base`.
The sentence is `state.to_string()` clipped with `cells::clip` (ends in `…`, `cells.rs:120-135`); the key
and title use `cells::fit`. No new width rule (MOD-81's).

#### B.5.3 Wiring `Act::Queue`

- `keys/catalogue.rs`: after `Waiting` (`:108-109`):
  `/// `global.queue`: open the queue overlay (offered by `register_all`).` `Queue,`
  and after the `waiting` row (`:321`): `row(Act::Queue, Global, "queue", &["ctrl-q"], "queue"),`
  with the block comment amended to "… waiting list (MOD-69 D7) and queue overlay (MOD-12 M3 D8)".
  Placed **after** `waiting`, so the status line, cut at 100 columns before `Ctrl+w waiting` in every
  snapshot, is unchanged (§E H-4). Tests: `ALL` gains `Act::Queue` after `Act::Waiting`; `position`
  gains `Act::Queue => 16` and every later arm +1 (`OverlayClose => 17` … `Dismiss => 41`);
  `CATALOGUE.len()` 41 → 42.
- `app/state.rs:339-341`: `matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)`.
- `app/mod.rs`: import `QueueOverlay` (`:14`); after the waiting list (`:101-107`):
  ```rust
      // MOD-12 M3 D8: the queue overlay, global `Ctrl+Q`. A chord, for `Ctrl+F`'s reason.
      app.overlay_factories
          .register(QueueOverlay::ID, || Box::new(QueueOverlay::new()));
      app.offer(Act::Queue, Action::Overlay(OverlayAction::Open(QueueOverlay::ID)));
  ```
- `keys/print.rs` `DEFAULT_HEAD` (`:92`), after the `waiting` line:
  `queue        = ["ctrl-q"]   # queue` (12-cell name column, 26-cell left column: the existing
  padding, computed). `take(26)` → `take(27)`.
- `keys/hint.rs`: `bare` → `!matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)`;
  `FULL` and the two help-line literals end `… · Ctrl+w waiting · Ctrl+q queue`; `rows(88)`'s second
  row becomes `"  ?/F1 help · w workspaces · Ctrl+f find · Ctrl+w waiting · Ctrl+q queue"` (72 cells,
  still two rows).
- Every `Act` site, checked with a text search for `Act::Waiting`: `catalogue.rs` (enum, row, `ALL`,
  `position`), `state.rs:340` (`is_offerable`), `app/mod.rs:105` (offer), `hint.rs:217` (`bare`),
  `stack.rs:149` (test). `action_for` (`state.rs:346-361`) has a wildcard arm: no change.

### B.6 T6: documents

- `docs/ANA-2.md` §4.10 (`:1448`), after the M2 as-built paragraph (`:1541`): **As built (MOD-12 M3,
  `.claude/plans/mod-12-m3-queue-overlay.plan.md`).** The overlay (`ctrl-q`); D2's one queue order;
  reorder writes `queue_entry.position` (first move materialises `1..n`); L4: a stalled batch closes
  `drained` (the predicate ignores entries; zero free slots, a read failure and an enqueue refusal
  never close; a parked run keeps it open), so a later `Q` waits for `P`; `0016`'s
  `closed_reason` column comment ("no entry left") is stale and stays, as migrations are forward-only.
- PRD: M3 row `complete`; under Open Questions a checked L4 entry ("Does a stalled batch close? Yes
  (maintainer, 2026-10-07) …").
- `HANDOFF.md:111`: MOD-12 done; `docs/decisions/mod/mod-12.md` (CREATE) per
  `.claude/rules/workflow-docs.md:27`, with D1-D4 of the PRD and L4.
- Validate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

### Data flow

Runner (every sweep of the executing process): `admit` → reads → `admission_order` → spend gate →
enqueue, or `drain(Empty|Stalled)` → `close_drained_batch` (store re-checks live runs).
Overlay: `ctrl-q` → `OverlayAction::Open(QueueOverlay::ID)` → `wants_requests` →
`StoreRequest::QueueOverview` → store worker `try_serve` → `queue_overview::overview` (Backend reads +
`classify_entry`) → `StoreReply::QueueOverview` → `QueueOverlay::on_reply` → `render`. Keys →
`MoveQueueEntry`/`PauseQueue`/`ResumeQueue`/`DequeueItem` → `serve_queue` → `QueueWritten` → the
overlay re-requests the overview. Every fourth tick → `App::on_tick` → `refresh_top_overlay` →
`QueueOverlay::refresh` (only when no read is in flight).

---

## C. Tests first (TDD), per task

A red test lands only when it compiles; otherwise with the smallest stub that returns a wrong answer
(no `todo!()`), so it fails rather than panics.

### C.1 T1 (`model/queue.rs` tests, after `:352`)

A `row(status)` helper builds a `QueueRow` over `entry(item, None)` (`:186`) with `key "FIX-1"`,
`latest_run: None`, no note, no blockers; `run(status, box)` a `QueueRunFact`; `facts()` a `LiveFacts`
with the item ready.

| Test | Asserts |
|---|---|
| `moved_order_swaps_with_the_neighbour` | `[a, b, c]`: `b` `Up` → `[b, a, c]`, `b` `Down` → `[a, c, b]` |
| `moved_order_is_none_at_an_end_or_off_the_list` | `a` `Up`, `c` `Down`, an unknown id: `None` |
| `a_live_run_here_reads_running_and_a_queued_one_admitted` | rows 1: `Running { status: Running }` → `running here`; `Queued` → `admitted, waiting to be claimed` |
| `a_live_run_elsewhere_beats_everything` | a `failed` item with an open blocker, missing tags and a hold, whose latest run is `Running` on box B: `Elsewhere { hostname: Some("b"), .. }` → `running on b`; `Queued` → `queued for b`; hostname `None` → `running on another box` |
| `a_failed_item_reads_failed_even_with_an_open_blocker` | row 3; `failed: boom` and `failed` |
| `a_blocked_item_with_a_parked_run_reads_review_loop_exhausted` | row 4; sentence `review loop exhausted (last note: review loop exhausted after 3 attempts)` |
| `a_blocked_item_without_a_parked_run_reads_blocked` | row 5, with and without a run and a note |
| `awaiting_approval_with_a_parked_step_reads_hard_gate_parked` | row 6 |
| `awaiting_approval_without_a_parked_step_reads_judge_undecided` | row 7; a note `fan-out `p` attempt 2 awaits selection: …` |
| `awaiting_approval_without_a_parked_run_is_not_ready` | `Waiting(NotReady(AwaitingApproval))` → `not ready: item is awaiting_approval` |
| `missing_tags_beat_an_open_blocker` | row 8 before 9; `missing tags: cuda, gpu` |
| `an_open_blocker_reads_waiting_on_its_keys` | `waiting on FEAT-1, FEAT-2` |
| `cancelled_in_batch_reads_waiting` | row 10 even when ready |
| `a_paused_queue_reads_paused_for_ready_entries_only` | ready + paused → `queue paused`; not ready + paused → `NotReady` |
| `held_only_for_ready_entries_of_an_open_batch` | a hold on the project: ready → `Held(Budget(..))` → `held: batch cap reached (600 of 500 micros)`; not ready → `NotReady`; `BadCap` → `held: ` + `CapError` text; `ProjectGone` → `held: its project is gone` |
| `a_ready_admissible_entry_reads_next` | `next to run` |
| `a_multi_line_note_shows_its_first_line` | `blocked (last note: first)` for `"first\nsecond"` |
| `reveal_names_the_run_and_the_parked_step` | `HardGateParked` → `(Some(run), Some(step))`; `Next` → `(None, None)`; `Failed { run: Some }` → `(Some, None)` |

Gate: `cargo test -p htui-core --all-features queue` and `cargo clippy -p htui-core --all-features --all-targets -- -D warnings`.

### C.2 T2

**`auto_queue.rs`** (new, after the M2 block, `:1610`), over `Backend::Memory`:

| # | Test | Asserts |
|---|---|---|
| a | `a_stalled_batch_closes_and_a_later_queue_waits_for_resume` | `Harness::open`; a `blocked` item queued; `resume`; sweep: `open_batch_of == None`, the entry kept, `batch_runs(batch)` empty. Then a ready item queued and swept: no run. `resume`, sweep: it has a run under the **new** batch |
| b | `a_parked_auto_run_keeps_a_stalled_batch_open` | A queued, resumed, swept: A parks at `verdict` (its item `awaiting_approval`, so `order` is empty: CLOSE 2 is reached); a second sweep: `open_batch_of` is still `batch` (the predicate's live run) |
| c | `a_batch_whose_every_entry_is_held_closes` | `spent_batch()` (`:1424`), then `finish_run(A1, Cancelled)` (no live run; A1 is now cancelled-in-batch), `cap_batch(PROJECT_HTUI, 600)`, queue A2 (htui), **one** sweep: A2 has no run, `open_batch_of == None`, A2's entry kept, `batch_spend(batch) == Some(600)` |
| d | `zero_free_slots_never_close_a_batch_with_admissible_entries` | `Harness::with_box_settings({"max_concurrent_items": 1})`, sessions held; a manual `StartRun` of M runs (the slot); X (ready) queued; `resume`; sweep: X has no run, `open_batch_of == Some(batch)`. `parts.open()`, settle: M parks (no longer `running`), the wake admits X under `batch` |
| d2 | `zero_free_slots_never_close_a_stalled_batch` | As (d) but X is `blocked`: after the first sweep `open_batch_of == Some(batch)` (step 8 returns before CLOSE 2). After M parks and the wake sweeps: `open_batch_of == None` |
| f | `an_enqueue_refusal_never_closes_the_batch` | Only entry `broken` (`repoint_at_no_template`, `:758`); `resume`; two sweeps: `broken` has no run, its status `open`, `open_batch_of == Some(batch)` |

Plan (e), "the empty-queue drain still closes", is the unchanged `a_drained_batch_closes_drained`
(`:888`) and `a_live_batch_run_keeps_the_drained_batch_open` (`:961`).

Amended in T2 (§F-1): `a_queued_item_that_is_not_ready_waits_then_runs` (`:705`),
`a_blocked_entry_keeps_the_batch_open` (`:1006`, renamed
`a_blocked_entry_keeps_its_place_and_its_stalled_batch_closes`),
`a_cancelled_auto_run_is_not_readmitted_in_its_batch` (`:1247`), the doc of
`a_stopped_batch_stays_open_and_keeps_its_entries` (`:1539-1540`: "stays open while A1's parked run
is live (M3 L4)"). Unchanged and still green, re-checked by reading: (b) `:648`, (d) `:735`, (d2) `:817`,
(e) `:835`, (g1) `:1045`, (g2) `:1104`, (h) `:1155`, (i) `:1179`, (k) `:1213`, (m) `:1316`, M2 (a)-(d),
(f), the Pg cases (`a_batch_overshoots_…_pg`, `:1825`, still passes: A1's failure stalls the batch
before A2 is queued, so A2 is not admitted, for a new reason).

**`runtime.rs`** unit: `an_entry_whose_project_is_gone_is_not_admitted` (`:4102`): after the first
`admit`, assert `store.open_batch_of(BOX) == None` ("an absent project stalls the batch: CLOSE 3"),
then `store.open_batch(BOX, USER, now)` before the second `admit`. The streak test (`:4047`) is
unchanged (§B.2.2).

**Store** (§F-1):
- `mem.rs` `close_drained_batch_closes_only_the_drained_batch_it_names` (`:14664`), renamed
  `close_drained_batch_closes_only_its_batch_and_ignores_entries`: after `queue_item(ANA_2)`, the
  "an entry keeps it open" assertion (`:14703-14709`) and the `dequeue_item` go; `create_run(batch_run(ANA_2, Some(second)))`
  with the entry present → `None` ("a live run of its own keeps it open"); `finish_run(Cancelled)` →
  closes `drained`; `queue_entries(BOX)` still `[ANA_2]`.
- `pg_criteria.rs` `close_batch_cancels_its_queued_runs_and_the_drain_closes_only_its_batch`
  (`:8120`): the `:8270-8282` pair ("an entry keeps it open") and the two `dequeue_item`s (`:8284-8285`)
  go; the live run is created with the entry present; after its cancel both stores close `drained`
  and `queue_entries` agree and still hold ANA-2.
- New `pg_criteria.rs` parity: `a_stalled_batch_closes_alike_on_both_stores`: an open batch with one
  entry and no run: `close_drained_batch` closes on both, `batch_shape` equal; with a `queued` batch
  run: `None` on both.

Gates:
```bash
set -o pipefail   # the cargo exit code, not tee's; a grep on an aborted log is silent
cargo test -p htui-worker --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m3-t2.log
grep -n -E 'SIGABRT|overflowed|test result: FAILED' /tmp/m3-t2.log
cargo test -p htui-core --all-features batch
cargo test -p htui-store --all-features --test pg_criteria -- --test-threads=1 batch drain
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
```

### C.3 T3

`mem.rs` (M1 block, `:14401+`; steps via `create_step`, notes via `add_note`, links via the
`create_link` write the M1/M4 tests use (**VERIFY** its name), a second box by pushing a `BoxRow` into
`fixtures::demo_data().boxes` and `MemStore::from_demo`):

| Test | Asserts |
|---|---|
| `queue_entries_sort_position_then_priority_then_created_at` | entries with priorities 0/2/1 queued in that order read `[p2, p1, p0]`; equal priorities read by `created_at` (ANA-1 before ANA-2); a positioned entry leads |
| `queue_entries_and_admission_order_agree` | `admission_order(&queue_entries, &ready_items)` equals `queue_entries` filtered to the ready ids (D2's pin) |
| `queue_rows_carry_the_latest_graph_run_note_and_open_blockers` | two runs, the later wins; a chat run is ignored; two notes, the later wins; `blocked_by` to an `in_progress` item listed, to a `done` one and a tombstoned edge not |
| `queue_rows_name_the_parked_step_and_the_target_hostname` | an `awaiting_approval` step is `parked_step`; a run targeted at box B carries B's hostname |
| `move_queue_entry_writes_every_position_and_swaps` | three unpositioned entries; `move(b, Up)` → `true`; `queue_entries` `[b, a, c]` with positions `1, 2, 3` |
| `move_queue_entry_at_an_end_or_off_the_queue_writes_nothing` | first `Up`, last `Down`, an unqueued item, an item queued on another box: `false`; every `position` still `None` |
| `an_entry_queued_after_a_move_goes_last` | after a move, `queue_item(d)` with the highest priority reads last |
| `move_queue_entry_never_touches_priority` | `item.priority` and `item.version` unchanged |

`pg_criteria.rs` (on `common::demo_db()` and `MemStore::demo()`, early return without the database):
- `queue_surface_answers_alike_on_both_stores` (`:7616`): `:7665` becomes
  `assert_eq!(entries[0].item_id, ids::HTUI_ANA_1, "priority, then created_at (M3 D2)");` (§F-5).
- `queue_entries_follow_one_order_on_both_stores`: the Mem fixture of the first table row through both.
- `queue_rows_answer_alike_on_both_stores`: the same planted runs, steps, notes and links on both;
  ids differ per store, so compare the rows with run and step ids mapped to their position in each
  store's own planting. A second box on Pg is a raw `INSERT INTO box …` on `db.pool` (the
  `auto_queue.rs:1840` style); **VERIFY** the box's NOT NULL columns (`0001_init.sql:53-74`).
- `move_queue_entry_answers_alike_on_both_stores`: the same three moves on both; equal
  `queue_entries` (positions included) and equal booleans.

Gates:
```bash
cargo test -p htui-core --all-features
cargo test -p htui-store --all-features --no-fail-fast -- --test-threads=1
cargo build --workspace --all-features --all-targets
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
cargo test -p htui-worker --all-features -- --test-threads=1   # the order change under the runner
```

### C.4 T4 (`queue_overview.rs` `#[cfg(test)] mod tests`, over `Backend::memory`)

Items are minted as `auto_queue.rs` mints them (`mint_in`, `:184`) and steered with `transition`,
`create_run`/`finish_run`, `create_step` and `add_note`; the reply is read through
`crate::store_worker::serve`.

| Test | Asserts |
|---|---|
| `the_overview_lists_the_queue_in_order_with_every_state` | one entry per state: running here, admitted (queued run), elsewhere (box B), next, waiting on a blocker, missing tags, failed, blocked, review loop exhausted, judge undecided, hard gate parked, not ready; rows in D2 order; each `EntryState` as expected |
| `a_batch_over_its_cap_marks_ready_rows_held` | a costed run's spend 600, `per_token_cap_batch` 500 on htui: the htui ready row `Held(Budget(CapReached { spent: 600, cap: 500 }))`, an agy ready row `Next` |
| `a_malformed_cap_reads_held` | `per_token_cap_batch: "lots"`: `Held(BadCap(_))` (D10's noted residual) |
| `a_paused_queue_marks_ready_rows_paused_and_has_no_batch` | `batch == None`, ready rows `Waiting(Paused)` |
| `next_is_exactly_what_admission_would_admit` | with a hold on one project, cancelled-in-batch and not-ready rows: the `Next` items equal `admission_order` minus the held projects |
| `the_header_figures_count_slots_and_spend` | `slots_used` = running + queued, `slots_limit` = the box's `max_concurrent_items`, `spent` = `batch_spend` |
| `memory_answers_demo` | `demo == true` |
| `move_queue_entry_answers_moved_with_the_new_order` | `MoveQueueEntry { item: second, to: Up }` → `QueueWritten { write: Moved { moved: true }, view }`, `view.entries` reordered |
| `moving_the_head_up_answers_moved_false` | `Moved { moved: false }`, order unchanged |
| `offline_both_requests_fail_by_their_names` | an offline `Backend` (the `prompt_settings.rs:516` recipe; **VERIFY**): `Failed { request: "queue_overview" | "move_queue_entry", message: DATABASE_UNREACHABLE }`, both names in `QUEUE_REQUEST_NAMES` |

`store_worker.rs`: `queue_requests_are_named_as_queue_request_names_lists_them` (`:5129`) gains the two
requests (seven, in order). `backlog/mod.rs`: `queue_sentence_covers_every_write` (`:1069`) gains
`Moved { moved: true }` → `queue reordered (2 in queue)` and `Moved { moved: false }` →
`already at that end of the queue`.

Gate: `cargo test -p htui --all-features --no-fail-fast queue -- --test-threads=1`.

### C.5 T5

`ui/overlay/queue.rs` tests: a `Bench` as `WaitingList`'s (`waiting_list.rs:349-411`: `Ctx` over
owned parts, `TestBackend(100, 30)`, symbols only) plus `feed(&mut overlay, overview)` that calls
`on_reply(&StoreReply::QueueOverview(Box::new(overview)))` and `drained()` over `emit`. Snapshots
(`insta::assert_snapshot!`) land as `src/ui/overlay/snapshots/htui__ui__overlay__queue__tests__{name}.snap`.

| Test | Asserts (snapshot) |
|---|---|
| `before_the_first_reply_it_says_reading` | `reading the queue` (`reading`) |
| `every_state_reads_its_sentence` | one row per `EntryState` and `Escalation`, §B.1.2's text (`states`) |
| `the_header_reads_a_running_batch` | `queue: running · batch since 14:02 · $1.20 spent · 1/2 slots` (`header_running`) |
| `the_header_reads_paused` | `queue: paused · 0/2 slots` (`header_paused`) |
| `the_header_reads_demo` | `queue: running · demo: nothing is admitted` (`header_demo`) |
| `a_failed_read_shows_the_unavailable_line` | `the queue is unavailable: <DATABASE_UNREACHABLE>` (`offline`) |
| `a_narrow_frame_clips_the_sentence_with_an_ellipsis` | `TestBackend(60, 20)`: the longest sentence ends in `…` (`narrow`) |
| `escalated_rows_use_the_warning_style` | the escalated row's cells carry `theme.warning`'s fg; the selected row `theme.accent` |
| `the_cursor_follows_its_item_across_a_reorder` | cursor on `b` (row 2); a reply with `b` first: cursor row 0, anchor `b` |
| `capital_j_and_k_request_a_move_of_the_cursor_row` | `J` → `Store(MoveQueueEntry { item: b, to: Down })`; `K` → `Up` |
| `p_pauses_a_running_queue_and_resumes_a_paused_one` | batch open → `PauseQueue`; none → `ResumeQueue`; before any reply → nothing |
| `capital_q_dequeues_the_cursor_row` | `DequeueItem { item }` |
| `enter_closes_then_reveals_the_run_or_the_item` | a `HardGateParked` row → `[Close, Reveal(Step { run: Some, step: Some })]`; a `Next` row → `[Close, Reveal(Item { .. })]` |
| `a_queue_write_reply_re_reads_the_overview` | `QueueWritten` and a `Failed { request: "pause_queue" }` each emit `Store(QueueOverview)` |
| `refresh_requests_only_when_nothing_is_in_flight` | fresh: `refresh` emits nothing (the open read is in flight); after a reply: one request; a second `refresh` before the reply: none |
| `esc_and_lower_q_pass` | `Esc` → `Pass`; `q` → `Pass` (swallowed by `is_modal`, never quits) |

`app/update.rs` test: `the_refresh_tick_refreshes_the_top_overlay_only`: two test overlays (the
`Popup` pattern, `:1093`) counting `refresh` calls; four `Action::Tick`s: the top one counted once, the
lower one zero.

`tests/queue_overlay.rs` (CREATE; testkit `Harness`, `register_all`, the `tests/waiting.rs` style):

| Test | Asserts |
|---|---|
| `ctrl_q_opens_the_queue_overlay_over_the_memory_backend` | two items queued in the `MemStore` before `Harness::over`; `key("ctrl-q")`, `drive_to_end`: top is `QueueOverlay::ID`; the frame holds both keys and `demo: nothing is admitted` |
| `a_reorder_round_trips_through_the_store` | `j`, `K`, `drive_to_end`: the store's `queue_entries` order and positions changed; the frame's row order matches |
| `p_resumes_and_pauses_from_the_overlay` | `P`: `open_batch_of` is `Some`; `P` again: `None`; header follows |
| `capital_q_dequeues_from_the_overlay` | the store no longer holds the entry; the row is gone |
| `the_overlay_re_reads_on_the_refresh_tick` | an item queued in the store behind the overlay's back; four `app().update(Action::Tick)` + `drive_to_end`: its row appears |

Key tables: `tests/keys.rs:143` gains `("ctrl-q", QueueOverlay::ID)`; `keys/stack.rs:149` gains
`assert_eq!(base(keys, "ctrl-q"), [Act::Queue]);`; `keys/hint.rs`, `keys/print.rs`,
`tests/keys_file.rs` + `valid.toml` as §B.5.3 and §F-6.

Gates:
```bash
set -o pipefail   # the cargo exit code, not tee's; a grep on an aborted log is silent
cargo test -p htui --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m3-t5.log
grep -n -E 'SIGABRT|overflowed|test result: FAILED' /tmp/m3-t5.log
cargo insta test -p htui --all-features   # exactly the new queue snapshots (8 unit + e2e if any), nothing else
cargo run -p htui -- --print-keys | sed -n '20,24p'   # the queue line under waiting
```

---

## D. Commits per task

The trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage by explicit path.
Implementers commit incrementally (uncommitted subagent work dies with its session).

- **T1**
  - `test(mod-12): classify_entry and moved_order pins (M3 D3, D5)` (stub `classify_entry` returns `Next`)
  - `feat(mod-12): the queue overlay's pure model - QueueRow, EntryState, Escalation, classify_entry (M3 D5)`
- **T2**
  - `test(mod-12): L4 - a stalled batch closes; amend the tests that pinned it open`
  - `feat(mod-12): close_drained_batch ignores entries; admit closes a stalled batch (M3 D4, L4); .sqlx`
- **T3**
  - `test(mod-12): D2 queue order, queue_rows facts, move_queue_entry (Mem units, parity)`
  - `feat(mod-12): queue_entries in D2 order; queue_rows and move_queue_entry on Mem, Pg, Backend; .sqlx`
- **T4**
  - `feat(mod-12): QueueOverview and MoveQueueEntry requests, QueueWrite::Moved, names 5 to 7 (M3 D7)`
  - `feat(mod-12): queue_overview serve module + tests`
- **T5**
  - `feat(mod-12): Overlay::refresh on the shell's refresh tick (M3 D9)`
  - `feat(mod-12): the queue overlay and its tests, snapshots (M3 D8)`
  - `feat(mod-12): global ctrl-q opens the queue overlay; key tables and fixtures follow`
- **T6**: `docs(mod-12): M3 as built - queue overlay, D2 order, L4 stall close; PRD complete; MOD-12 decision file`

Full validation (plan §Validation, run by the orchestrator after T5 and again after T6):
```bash
set -o pipefail   # the cargo exit code, not tee's
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod12-m3.log
grep -n -E 'SIGABRT|overflowed|test result: FAILED' /tmp/mod12-m3.log
cargo insta test -p htui --all-features
cargo doc --workspace --no-deps --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## E. Hazards (checked against the tree)

| # | Hazard | Evidence | Handling |
|---|---|---|---|
| H-1 | `htui-orch` stack headroom | Repo memory "htui-orch test stack headroom" | Irrelevant: M3 touches no `htui-orch` file. `htui-worker`'s and `htui`'s suites still walk the engine, so every gate keeps `--no-fail-fast` and the `SIGABRT\|overflowed` grep |
| H-2 | `.sqlx` against a migrated scratch DB | `docs/hr-sandbox.md:196-210`; repo memory "sqlx prepare needs a migrated scratch DB" (the test DSN is not the prepare DB) | T2: +1 −1. T3: +4 −1, and re-prepare **after** T2 is in (serial). The move's ordered read must be the new `queue_entries` literal byte for byte (indentation included) or prepare adds a file; `prepare --check` tells. Commit deletions with additions |
| H-3 | Mem/Pg parity of the new reads | D2 tie-break `i.id` (Pg) vs `entry.item_id` (Mem, the same uuid, `ItemId` `Ord` = uuid byte order); blockers `COLLATE "C"` vs `String` order; latest run `(queued_at, id)`, latest note `(created_at, id)`; `kind = 'graph'` both sides; timestamps microsecond-truncated | The three parity tests of §C.3; never compare minted ids across stores |
| H-4 | Snapshot churn | 37 snapshots render the global status line, every one cut before `Ctrl+w waiting` at 100 columns (`grep -rh 'Ctrl+f find' --include=*.snap`); no snapshot holds the help box's global line | The `queue` row goes **after** `waiting` (§B.5.3), so no existing `.snap` changes; `cargo insta test` must list only `htui__ui__overlay__queue__tests__*` and `queue_overlay__*` |
| H-5 | The Backlog's wildcard-free matches | `QueueWrite` at `backlog/mod.rs:477-489`, `:1001-1033`; `QUEUE_REQUEST_NAMES` is `[&str; 5]` (`store_worker.rs:96`) used at `:801`, `:1195` | T4 adds `Moved` to both matches and resizes the array to 7 in the same commit as the variants, or `htui` does not build |
| H-6 | Key-file collision | `load_str` runs `validate` (`keys/load.rs`, `validate.rs:15-60`): two actions on one chord in `[global]` are refused. `valid.toml` and `print.rs:119` bind `quit = ["ctrl-q"]` | T5 moves those to `ctrl-x` (unbound: no `"ctrl-x"` anywhere in `crates/htui`) (§F-6). Since R1 H1 a user's entry over a default it shares a table with is no longer refused (§F-18) |
| H-7 | Test infrastructure | Repo memories "htui integration tests need testkit", "htui suite green is scheduling-dependent", "featureless clippy gate" | `--all-features`, `--test-threads=1`; `cargo clippy --workspace -- -D warnings` at T2, T3, T5 and the end |
| H-8 | Resume over a stalled queue | `run_worker.rs:1784` (D8: the loop sweeps right after `ResumeQueue`) | A `P` over a queue with nothing admissible closes again at that sweep: the Backlog's `queue resumed` is followed by a paused queue on the next read. Accepted by D4 ("Q/P status lines unchanged"); T6 says so in ANA-2 |
| H-9 | A stall at zero free slots | §B.2.2 step 8 returns before CLOSE 2 | The batch stays open until a slot frees; the first tick with a free slot and nothing admissible closes it. In that window a newly queued ready item can be admitted without a new `P`. The cost of the hard constraint; §F-4 |
| H-10 | Racing writers | `close_drained_batch` re-checks live runs in its own `WHERE`; `enqueue_in_batch` on a closed batch is a `Constraint` (M1 H-6) | A close racing an admission either sees the run (no close) or wins (the enqueue is refused and `admit` returns at the re-read). No new lock |
| H-11 | `READ COMMITTED` + `ORDER BY … FOR UPDATE` | Postgres documents that such a select can return rows out of order after waiting on a lock | The move locks without `ORDER BY`, then re-reads in order (§B.3.3, §F-12) |
| H-12 | Time zone in snapshots | Header time | `format("%H:%M")` on the UTC `opened_at`, never local |
| H-13 | `0016`'s column comment is stale | `0016_auto_queue.sql` `closed_reason`: "drained (no entry left and no live auto run)" | Not edited (forward-only, D1 no migration); ANA-2's as-built note (T6) records the new meaning |
| H-14 | Done/closed entries before a prune | Only `admit` prunes (`runtime.rs:2187`) | The overview may show one as `not ready: item is done` until the next sweep; harmless |
| H-15 | `ctrl-q` flow control | crossterm raw mode clears `IXON` | The e2e test drives the chord; `keys.toml` rebinds (MOD-67) |
| H-16 | Disk pressure kills the dev Postgres | Repo memory "Dev Postgres crash loop" | `df -h .` before long Pg gates |

---

## F. Amendments (where the plan is wrong or silent against the tree)

1. **T2 breaks six existing tests the plan does not name; all are in T2's own files.** Each pins "a
   stalled batch stays open":
   - `auto_queue.rs` `a_queued_item_that_is_not_ready_waits_then_runs` (`:705`): its first sweep now
     closes the batch, so after the unblock it must `h.resume()` before the second sweep (and assert
     the close first).
   - `auto_queue.rs` `a_blocked_entry_keeps_the_batch_open` (`:1006`): the exact inverse of L4; it
     becomes `a_blocked_entry_keeps_its_place_and_its_stalled_batch_closes` (entry kept,
     `open_batch_of == None`).
   - `auto_queue.rs` `a_cancelled_auto_run_is_not_readmitted_in_its_batch` (`:1247`): after the cancel
     the sweep stalls the batch (the item is cancelled-in-batch), so the explicit
     `close_batch(Paused).expect("the batch was open")` (`:1290-1294`) panics; it becomes an assertion
     that the batch closed, then `resume`.
   - `runtime.rs` `an_entry_whose_project_is_gone_is_not_admitted` (`:4102`): the first `admit` now
     closes (CLOSE 3), so the second never reaches the engine; re-open the batch between them.
   - `mem.rs:14708` and `pg_criteria.rs:8275` assert "an entry keeps it open": the predicate's removed
     clause.
   M2's `a_stopped_batch_stays_open_and_keeps_its_entries` stays green only because `spent_batch`'s A1
   is parked (live); its doc is corrected. No file outside T2's set, so no wave change.
2. **T2 also edits `backend.rs`** (the `close_drained_batch` doc, `:707-708`, states the old
   predicate). The plan lists `backend.rs` under T3 only. T2 ∩ T3 was already non-empty (serial) and T1
   does not touch it, so **T1 ∥ T2 still holds**.
3. **`BatchClose::Drained`'s doc (`queue.rs:20-21`) states the old predicate** ("No entry was left
   and …"). It lives in T1's file, so this blueprint gives the doc edit to **T1**, not T2, to keep
   T1 ∥ T2 disjoint.
4. **D4's list and its "free slots = 0 never closes" bullet disagree for the empty queue; this
   blueprint reads the hard constraint as covering every close M3 adds.** Today `entries.is_empty()`
   drains before any slot read (`:2198`), and `order.is_empty()` returns before it (`:2234`). The
   slot read moves above the `order` check (§B.2.2 step 7), so neither new close fires at zero free
   slots; the M1 empty-queue drain is unchanged (T2(e) is its regression). If the maintainer wants
   the literal reading for the empty queue too, move the `entries.is_empty()` branch below step 8: one
   move, and every listed test still passes (their fixtures have free slots). H-9 is the cost either way.
5. **T3 breaks `queue_surface_answers_alike_on_both_stores`** (`pg_criteria.rs:7665`,
   `entries[0] == HTUI_ANA_2, "queued_at first"`). Under D2 all three fixture items have priority 0, so
   `created_at` decides (`demo_at(0, n)`, `fixtures.rs:1093-1118`): ANA-1 (n 0) first. In T3's file.
6. **T5's file set is wider than the plan's**, all `htui` files only T5 touches:
   - `keys/hint.rs`: `FULL` and two help-line literals end `Ctrl+w waiting` and would fail with a
     fourth offered global; `bare` names the offerable acts.
   - `keys/stack.rs:149`: the base-stack test lists the offered globals (an added assertion).
   - `tests/keys_file.rs` (`:84`, `:346`, `:371`) and `tests/fixtures/keys/valid.toml`: the "valid" key
     file binds `quit = ["ctrl-q"]`, which `validate` refuses once `queue` defaults to `ctrl-q`;
     `print.rs:119`/`:127` likewise. All move to `ctrl-x`. (As of T5; since R1 H1 such a file
     loads and the queue gives `ctrl-q` up, §F-18.)
   - `app/update.rs` gains a unit test as well as the hook.
7. **The model's types need four small additions to D5's sketch:**
   - `Held` carries a `Hold` (`Budget(BatchStop)`, `BadCap(CapError)`, `ProjectGone`), not a bare
     `BatchStop`: D10 says a malformed cap "reads `Held`", and `BatchStop` cannot carry one; the
     runner refuses an absent project too (M2 review R1 L3).
   - `Wait::NotReady(Status)` is the fallback for what the precedence list does not name (an
     `awaiting_approval` item with no parked run, a `queued`/`in_progress` item with no live run, an
     unpruned `done` entry). Without it `classify_entry` has no total answer.
   - `Elsewhere`, `Blocked` and `Failed` carry the run id: D8's `Enter` reveals "a run or escalation
     with a run", and a failed or walk-blocked item has one.
   - `BatchFigures` names the header's batch part; `moved_order` is the one swap both stores call.
8. **`StoreReply::QueueOverview` is boxed** (`Box<QueueOverview>`), as M2's `QueueSettings` is
   (`store_worker.rs:1690`), for `clippy::large_enum_variant`.
9. **The Backlog's "exhaustive `QueueWrite` match, if any" is two**: `on_queue_written`
   (`backlog/mod.rs:477-489`) and `queue_sentence` (`:1001-1033`), plus the table test (`:1069`).
10. **No existing snapshot changes** (H-4), provided the catalogue row follows `waiting`.
11. **Order docs follow D2** in T3 (`mem.rs:964`, `pg/read.rs:2224`, `backend.rs:627`) and T4
    (`QueueView.entries`, `store_worker.rs:1706`).
12. **The move locks, then reads.** D3 says "`SELECT … FOR UPDATE` in D2 order". With `ORDER BY`, a
    `READ COMMITTED` locking select that waited on a concurrent move can return rows in their
    pre-wait order, so the move would swap the wrong neighbour. The blueprint locks without `ORDER BY`
    and re-reads in D2 order in the same transaction, then writes the final permutation in one
    `UNNEST` update (equivalent to "write `1..n`, then swap"). D3 says an end move is `Ok(false)`;
    the blueprint settles that it **writes nothing** (no positions materialised).
13. **"Latest run" is the latest `kind = 'graph'` run.** A chat run attached to the item is not the
    queue's run (settled here; the plan is silent).
14. **T5 as built: three more files follow the new `[global]` action** (beyond §F-6). Every list of
    `[global]`'s actions gains `queue`: `keys/catalogue.rs` `the_global_block_is_in_status_line_order`,
    `keys/load.rs` `every_error_is_reported_and_sorted_by_line` (the `quitt` unknown-action report),
    and the same report through the binary in `tests/keys_file.rs` (`:117-120`, besides `:84`,
    `:346`, `:371`).
15. **T5 as built: a failed overview read drops the rows.** §B.5.2 sets only `failure`; the overlay
    also clears `overview`, so the keys over the unavailable line (`j`/`k`/`J`/`K`/`P`/`Q`/`Enter`)
    are consumed and do nothing, as on an unread list: a reorder, pause or dequeue never acts on rows
    the box is not showing. The next good read brings the rows back and the cursor re-finds its
    item. Pinned by `a_failed_read_shows_the_unavailable_line`. Before the first reply and while
    failed, the box shows no header: the reading or unavailable line, a blank, the hint.
16. **T5 as built: a failed read backs off, and a refused write keeps the status line.** `refresh`
    asks again after a failed read only every `REFRESHES_PER_RETRY` (10) refresh ticks (about 10 s),
    not every tick: offline every read is refused and `App::on_reply` would re-post the refusal to the
    status line each second. A good read ends the back-off. When a refused queue write's re-read
    fails too, the overlay emits the write's `"{request}: {message}"` again, so the status line names
    the write, not the read. Pinned by `a_failed_read_is_asked_again_only_every_tenth_refresh` and
    `a_refused_write_keeps_its_own_failure_on_the_status_line`.
17. **T5 as built: the header, the unavailable line and the hint are clipped to the box with an
    ellipsis** (`cells::clip`), besides the sentence column. The offline message is wider than a
    100-column box; the whole message is on the status line too. (The waiting list leaves its hint
    uncut; MOD-81 owns width rules.)
18. **Upgrade note for T6: `ctrl-q` is the queue's default** (amended by R1 H1, maintainer
    2026-10-08; T5 as built refused such a file). A `keys.toml` written before M3 that binds
    `ctrl-q` in `[global]` (e.g. `quit = ["ctrl-q"]`) still loads: the user's entry wins over an
    action of its own table left at its default. `global.queue` gives `ctrl-q` up and is unbound,
    htui prints a stderr notice (`htui: PATH:LINE: [global] quit = "ctrl-q" takes "ctrl-q" from
    global.queue, which is now unbound`) and starts, and `--print-keys` prints
    `queue = []` with the comment `# queue (unbound by quit)`. To keep the queue, list it with another chord
    (e.g. `queue = ["ctrl-x"]`). Two entries on `ctrl-q` (`quit` and `queue` both listed) are still
    refused (exit 2). T6's documents say so; `tests/keys_file.rs`
    `a_global_ctrl_q_binding_takes_ctrl_q_from_the_queue_with_a_notice`,
    `two_entries_on_ctrl_q_are_still_refused` and `a_quit_on_ctrl_q_quits_and_leaves_the_queue_unbound`
    pin it.
