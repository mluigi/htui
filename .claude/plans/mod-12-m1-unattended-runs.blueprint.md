# Blueprint: MOD-12 milestone 1: Unattended runs

**Plan**: `.claude/plans/mod-12-m1-unattended-runs.plan.md`. The plan's D1 to D11, T0 to T6, its waves and
its "Verified claims" table are binding, and the maintainer confirmed them on 2026-10-06. Every one
of them was re-checked against the tree at `1a235d25` (`hr/MOD-12`).
**PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md` (D1 to D4, M1 row).
**Design**: `docs/ANA-2.md` §4.10 (`:1447-1465`, downgrade `:1487-1494`), §8 "MOD-12 build order"
(`:1915-1929`), and §12 criteria 22 to 28 (`:2251-2268`).
**When the tree and the plan disagree, the tree wins.** Each such case appears under
**Deviations from the plan** with its evidence. Where the plan leaves a detail open, this blueprint
settles it and says so. A **VERIFY** marker means the implementer must check the point.

Conventions inherited unchanged:
- MSRV 1.98 and edition 2024. Workspace lints are at `-D warnings`. `#![warn(missing_docs)]` is set in
  `htui-core`, `htui-store` and `htui-worker`, so every new `pub` item carries a doc comment.
- Store traits are bound by path and are never `use`d where `ReadStore`/`WriteStore` are visible
  (`crates/htui-core/src/store/worker.rs:5-8`). A `WorkerHost` method is declared
  `fn … -> impl Future<Output = Result<T>> + Send`, and implementors write `async fn`.
- Unmirrored tables use inherent methods on `MemStore`/`PgStore`, dispatched by `Backend`. `Backend`'s
  `Offline` arm answers `orchestration_offline()` (`crates/htui-store/src/backend.rs:618-620`,
  which is `StoreError::Unreachable(DATABASE_UNREACHABLE)`).
- The status line takes every message, success or failure, through `Action::Error(String)` (e.g.
  `crates/htui/src/ui/tabs/backlog/detail/graph.rs:231`). `App::on_reply` already prints every
  `StoreReply::Failed` as `"{request}: {message}"` (`crates/htui/src/app/update.rs:287-289`).
- `.sqlx` is regenerated from inside `crates/htui-store` against a **migrated scratch DB**, using
  `cargo sqlx prepare -- --all-features --all-targets`. The bare form deletes feature-gated entries.
  In the hr sandbox, the DSN is `postgres://postgres@localhost:5439/<scratch>` (see `docs/hr-sandbox.md`).
- Every gate runs with `--no-fail-fast`, and the log is grepped for `SIGABRT|overflowed`. The suite is green only
  under `--test-threads=1`, and `htui` integration tests need `--all-features`.

---

## A. Per-file change table

| # | File | Task | What changes (and what must **not**) |
|---|---|---|---|
| 1 | `crates/htui-store/migrations/0016_auto_queue.sql` | T0 | CREATE (§B.0) |
| 2 | `crates/htui-store/tests/migrations.rs` | T0 | Count literals 15→16 (6 sites), `TABLES` 42→44, `MOD12_COLUMN_COMMENTS` and its three `.chain` sites, comment count 46→58, five new tests (§C.0) |
| 3 | `crates/htui-store/tests/connect.rs` | T0 | `Pending(15)`/`15`/"fifteen" → 16/"sixteen" at `:140-143`, `:156-158`, `:242-244` |
| 4 | `crates/htui-store/src/pg/read.rs` | T1, T3 | T1: `ready_items` `ORDER BY` and doc (`:2052-2110`). T3: six queue reads after `queued_runs_on_box` (`:2202-2220`) |
| 5 | `crates/htui-core/src/store/mem.rs` | T1, T3 | T1: `ready_items` sort and doc (`:746-777`) and new unit tests. T3: three `State` fields, the `from_demo` literal, inherent queue methods, `create_run` batch check, `delete_project` retain, and unit tests |
| 6 | `crates/htui-store/tests/pg_criteria.rs` | T1, T3 | T1: order test. T3: parity test, batch round trip, concurrent `open_batch`. Plus `batch_id: None` at `:1081` |
| 7 | `crates/htui-store/.sqlx/` | T1, T3 | T1: `query-a22310d8…json` (ready_items) replaced. T3: `query-eb3ab1ed…json` (create_run INSERT) replaced, plus one new file per new query (§B.3.5) |
| 8 | `crates/htui/src/store_worker.rs` | T1, T5 | T1: two `ready_here` tests compare row sets, not order (§C.1). T5: requests, replies, `serve_queue`, the resume wake in the loop, and tests |
| 9 | `crates/htui-orch/src/graph.rs` | T2 | `effective_gate`, `snapshot_phase(.., mode)`, the call site, docs, and unit tests |
| 10 | `crates/htui-orch/src/conformance.rs` | T2 | `start_in`, `auto_mode_case` frame, three `CASES`, count 100→103 |
| 11 | `crates/htui-orch/tests/fake_conformance.rs` | T2 | `CASES.len()` pin 100→103 (`:15-25`) |
| 12 | `crates/htui-core/src/model/queue.rs` | T3 | CREATE (§B.3.1) |
| 13 | `crates/htui-core/src/model/ids.rs` | T3 | `BatchId` appended to `id_newtype!` (after `PersonaId`, `:128`) |
| 14 | `crates/htui-core/src/model/mod.rs` | T3 | `pub mod queue;` (between `quota` and `relay`, `:96-97`). Re-exports `BatchId` (`:122-126`) and `queue::{…}` |
| 15 | `crates/htui-core/src/model/run.rs` | T3 | `NewRun.batch_id` (`:349-376`) |
| 16 | `crates/htui-core/src/store/worker.rs` | T3 | Seven `WorkerHost` methods, imports, module-doc count (`:19-20`, `:503`) |
| 17 | `crates/htui-store/src/pg/write.rs` | T3 | `create_run` batch check plus the INSERT column (`:4108-4227`). Queue writes in a new `impl PgStore` block |
| 18 | `crates/htui-store/src/backend.rs` | T3 | Nine inherent dispatchers after `queued_runs_on_box` (`:577-589`) |
| 19 | `crates/htui-store/src/worker.rs` | T3 | The seven methods in both `WorkerHost` impls (`:707-785`, `:787-865`) |
| 20 | 16 `NewRun` literals in 12 files | T3 | `batch_id: None` (§B.3.6) |
| 21 | `crates/htui-orch/src/engine.rs` | T3, T4 | T3: `batch_id: None` at `:706`, `:8930`, `:12200`, `:12304`. T4: `enqueue_with` and `enqueue_in_batch` |
| 22 | `crates/htui-worker/src/runtime.rs` | T3, T4 | T3: `batch_id: None` at `:3602`. T4: `admit`, `spawn_sweep`, `wake_on_rest`, `Shared.sweep_again`, `sweep_once` edits |
| 23 | `crates/htui-worker/tests/auto_queue.rs` | T4 | CREATE: harness plus 14 tests (§C.4) |
| 24 | `crates/htui/src/ui/tabs/backlog/mod.rs` | T5 | `Q`/`P` arms, `QueueIntent`, `on_reply` arms, `queue_sentence` |
| 25 | `crates/htui/src/app/mod.rs` | T5 | Two help `Binding` rows after `N`/`e` (`:160-170`) |
| 26 | `crates/htui/tests/backlog.rs` | T3, T5 | T3: `batch_id: None` at `:979`. T5: seven tests |
| 27 | `crates/htui/src/run_worker.rs` | T3, T5 | T3: `batch_id: None` at `:1618`, `:1659`. T5: `resuming_the_queue_sweeps_at_once` |
| 28 | `docs/ANA-2.md`, PRD, `HANDOFF.md` | T6 | §B.6 |

Waves are unchanged: T0 ∥ T1 ∥ T2, then T3, then T4 ∥ T5, then T6. T1 now also touches
`crates/htui/src/store_worker.rs`, which no other wave-1 task touches. T5 is the only wave-3 task that
touches it. **T4 ∩ T5 stays ∅** because the D8 resume hook moves into T5 (deviation 8).

---

## B. Interfaces and SQL, exactly

### B.0 T0: `crates/htui-store/migrations/0016_auto_queue.sql`

Before committing, re-check `ls /host/htui/crates/htui-store/migrations` (plan risk row). On a
collision, renumber the file and bump every literal in §C.0.

```sql
-- 0016_auto_queue.sql - MOD-12 milestone 1 (plan D1, D2, D3, D4, D7). Forward-only (R-STO-5).
--
-- The auto-mode queue (MOD-12 PRD D1-D4, docs/ANA-2.md §4.10). queue_entry is one item's opt-in
-- membership of one box's queue: run.status 'queued' already means "a run exists" (create_run moves
-- the item open|failed -> queued), so membership needs a row of its own. An item is queued on at most
-- one box, and the entry goes with its item (ON DELETE CASCADE), which delete_project relies on.
-- queue_batch is one queue activation: a box's queue runs iff it has an open batch (plan D2). Pause
-- closes it 'paused', drain closes it 'drained' (plan D3), and resume opens a new one. The partial
-- unique index keeps at most one open batch per box. run.batch_id is the batch an auto run was
-- admitted under, NULL for manual and chat runs. Milestone 2 sums run_step.usage over it, and no
-- total is stored (ANA-2 §4.10). Nothing here is mirrored, but schema_version becomes 16, so each
-- box rebuilds its mirror once on first start. A headless worker never migrates: migrate from a
-- TUI first.

CREATE TABLE queue_batch (
    id            UUID        PRIMARY KEY,
    box_id        UUID        NOT NULL REFERENCES box(id),
    opened_at     TIMESTAMPTZ NOT NULL,
    opened_by     UUID        NOT NULL REFERENCES app_user(id),
    closed_at     TIMESTAMPTZ,
    closed_reason TEXT,
    CONSTRAINT chk_queue_batch_closed_reason CHECK (closed_reason IN ('paused', 'drained')),
    CONSTRAINT chk_queue_batch_closed CHECK ((closed_at IS NULL) = (closed_reason IS NULL))
);

CREATE UNIQUE INDEX uq_queue_batch_open ON queue_batch(box_id) WHERE closed_at IS NULL;

CREATE TABLE queue_entry (
    item_id   UUID        PRIMARY KEY REFERENCES item(id) ON DELETE CASCADE,
    box_id    UUID        NOT NULL REFERENCES box(id),
    position  INTEGER,
    queued_at TIMESTAMPTZ NOT NULL,
    queued_by UUID        NOT NULL REFERENCES app_user(id)
);

CREATE INDEX idx_queue_entry_box ON queue_entry(box_id);

ALTER TABLE run
    ADD COLUMN batch_id UUID NULL CONSTRAINT fk_run_batch REFERENCES queue_batch(id);

CREATE INDEX idx_run_batch ON run(batch_id) WHERE batch_id IS NOT NULL;

COMMENT ON COLUMN queue_batch.id IS
    'MOD-12 D1: client-minted UUIDv7; what run.batch_id references.';
COMMENT ON COLUMN queue_batch.box_id IS
    'MOD-12 D2: the box whose queue this activation is; at most one open batch per box.';
COMMENT ON COLUMN queue_batch.opened_at IS
    'MOD-12 D2: when the queue was resumed.';
COMMENT ON COLUMN queue_batch.opened_by IS
    'MOD-12 D2: who resumed it.';
COMMENT ON COLUMN queue_batch.closed_at IS
    'MOD-12 D2, D3: when the batch closed; NULL while the queue runs.';
COMMENT ON COLUMN queue_batch.closed_reason IS
    'MOD-12 D2, D3: paused, or drained (no entry left and no live auto run); NULL while open.';
COMMENT ON COLUMN queue_entry.item_id IS
    'MOD-12 D1: the queued item; queued on at most one box, and gone with the item.';
COMMENT ON COLUMN queue_entry.box_id IS
    'MOD-12 D1: the box whose queue holds the item; the local box at queue time (R-ORCH-12).';
COMMENT ON COLUMN queue_entry.position IS
    'MOD-12 D4: an explicit queue position (milestone 3 reorder); NULL sorts last.';
COMMENT ON COLUMN queue_entry.queued_at IS
    'MOD-12 D1: when the item was queued.';
COMMENT ON COLUMN queue_entry.queued_by IS
    'MOD-12 D1: who queued it.';
COMMENT ON COLUMN run.batch_id IS
    'MOD-12 D7: the queue_batch an auto run was admitted under; NULL for manual and chat runs.';
```

Notes:
- **`closed_reason IN (…)` with NULL is NULL, so the CHECK passes.** The pairing CHECK is what forbids a
  half-closed row.
- **`ON CONFLICT (box_id) WHERE closed_at IS NULL` infers `uq_queue_batch_open`** (§B.3.4). The index has
  to be a partial unique *index*, because Postgres has no partial unique constraint.
- **`idx_run_batch` is new; D1 is silent on it.** It serves `batch_runs`, the drain check, and M2's
  spend sum. Being partial, it costs nothing on manual and chat rows.
- **The comments in `MOD12_COLUMN_COMMENTS` must stay byte-for-byte equal to the strings above.** Each
  string is a single literal, so no line-continuation joins are involved.

### B.1 T1: queue order for `ready_items` (criterion 22, D4)

**Pg** (`crates/htui-store/src/pg/read.rs:2101`): one line changes.

```sql
             ORDER BY i.priority DESC, i.created_at, i.id
```

`$1` stays used by the `WHERE i.project_id = ANY($1)`. `ItemSummary` and its projection are
unchanged, because Pg orders by `created_at` without projecting it (D4). The doc paragraph at
`:2055-2057`, "The readiness conjunct and the ordering are `items`' own…", becomes: "The readiness
conjunct is `items`' own, copied…; the ordering is the queue's (MOD-12 D4, ANA-2 criterion 22):
`priority DESC, created_at`, the id breaking a tie, and **not** the Backlog's display order. The
Backlog's "ready here" view keeps display order through `items` (`htui::store_worker::read_items`)."

**Mem** (`crates/htui-core/src/store/mem.rs:758-777`): the rows are filtered as today, then sorted
inside the same `read` closure.

```rust
let mut rows: Vec<ItemSummary> = /* today's filter chain, collected */;
rows.sort_by_cached_key(|row| {
    let item = state.items.get(&row.id);
    (
        core::cmp::Reverse(item.map_or(row.priority, |item| item.priority)),
        item.map(|item| item.created_at),
        row.id,
    )
});
rows
```

`ItemId`'s `Ord` is uuid byte order, which is the same as Postgres's uuid order (`mem.rs:819-820` relies
on this). The doc sentence "Ordered as [`ReadStore::items`] orders." becomes "In queue order:
`priority DESC, created_at, id` (MOD-12 D4, ANA-2 criterion 22)."

**`.sqlx`**: `query-a22310d83599424233ef569d5d082dde7e1db6b07ee7f8220bb1962e87fb6c31.json` is
deleted, and prepare writes its replacement.

### B.2 T2: the auto-mode gate downgrade (criteria 23 and 24, D10)

`crates/htui-orch/src/graph.rs`:

```rust
/// ANA-2 §4.10's one line (`R-ORCH-6`, `R-ORCH-2`): in auto mode a phase whose gate is not hard is
/// snapshotted `never`, so its steps land `done` with `gate_outcome = skipped`; a hard gate, and
/// every gate of a manual run, is kept. Applied **only** here, at snapshot time (MOD-12 D10):
/// `run.mode` is fixed at insert, so no later edit can skip a gate a run has not reached.
#[must_use]
pub(crate) const fn effective_gate(mode: RunMode, gate: Gate, gate_hard: bool) -> Gate {
    if matches!(mode, RunMode::Auto) && !gate_hard { Gate::Never } else { gate }
}
```

The `snapshot_phase` signature (`:683-692`) gains `mode` **last**, and the existing
`#[expect(clippy::too_many_arguments)]` still covers it:

```rust
async fn snapshot_phase<G: GraphSource>(
    source: &G,
    phase: &StepGraphPhase,
    rung_one: &[PhaseAgent],
    position: i32,
    project: ProjectId,
    settings: &ProjectSettings,
    app: &BTreeMap<String, Value>,
    box_id: BoxId,
    mode: RunMode,
) -> std::result::Result<SnapshotPhase, ResolveError>
```

- `:727-729`: the stub comment and `gate_effective: phase.gate` become
  `gate_effective: effective_gate(mode, phase.gate, phase.gate_hard),`.
- `:339-348`: the single call site passes `mode` after `box_id`.
- `:295-297`: the `resolve` doc sentence is replaced with: "`mode` is recorded on the snapshot and
  decides `gate_effective`: in auto mode every phase whose gate is not hard is snapshotted `never`
  ([`effective_gate`], MOD-12 D10). `resume` re-resolves under the run's own mode
  (`engine.rs:3036-3044`), so the topology an auto run is compared against is downgraded the same
  way (H-3)."
- `:1169-1171` (`field_chains_walk_phase_project_app`): the comment becomes "`gate_effective`: a
  manual snapshot never downgrades." The assertion is unchanged.

No snapshot fixture changes, because every recorded snapshot and `feature_snapshot_topology_is_pinned`
(`:1100`) resolve in `RunMode::Manual`.

`crates/htui-orch/src/conformance.rs`:

```rust
/// `StartRun` on `item` in `mode` with no requested scope, unwrapped to its run id and rest.
async fn start_in<O: Orchestrate>(orch: &O, item: ItemId, mode: RunMode) -> (RunId, Rest)
```

`start` (`:1300-1315`) becomes `start_in(orch, item, RunMode::Manual).await`. Its doc stays.

```rust
/// MOD-12 M1's three (ANA-2 criteria 23, 24), boxed for [`case`]'s reason.
fn auto_mode_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>>
```

`case()` (`:680-687`) gains `.or_else(|| auto_mode_case(name, harness))` before
`.unwrap_or_else(|| earlier_case(..))`. Three names are appended to `CASES` after
`"judges_never_get_command_run"` (`:656`), with a comment:
`// MOD-12 M1 (plan D10; ANA-2 criteria 23, 24): an auto run skips soft gates, parks at hard ones,
// and a manual snapshot keeps every gate.`
The new names are `"auto_mode_skips_soft_gates"`, `"auto_mode_parks_at_a_hard_gate"` and
`"a_manual_snapshot_keeps_its_gates"`. The `CASES` doc block (`:434-437`) gains the paragraph
"**Three for MOD-12 M1** (plan D10): …".

### B.3 T3: the queue store surface

#### B.3.1 `crates/htui-core/src/model/queue.rs` (CREATE)

```rust
//! The auto-mode queue (MOD-12 PRD D1-D4; `0016_auto_queue.sql`): one item's membership of a
//! box's queue, the batch a queue activation is, and the pure rules the runner composes
//! (plan D4-D6). Neither table is mirrored, so every store read of them is inherent.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::{BoxSettings, DEFAULT_MAX_CONCURRENT_ITEMS};
use crate::model::ids::{BatchId, BoxId, ItemId, ProjectId, UserId};
use crate::model::item::ItemSummary;

str_enum!(
    /// `queue_batch.closed_reason` (MOD-12 D2, D3).
    BatchClose {
        /// `P` closed it: no new admission, running runs untouched.
        Paused => "paused",
        /// No entry was left and no auto run of the batch was live.
        Drained => "drained",
    }
);

/// One `queue_entry` row, with its item's project (joined, not a column): what the runner needs
/// to build `ready_items`' scope (MOD-12 D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// `queue_entry.item_id`.
    pub item_id: ItemId,
    /// `item.project_id` of that item.
    pub project_id: ProjectId,
    /// `queue_entry.box_id`.
    pub box_id: BoxId,
    /// `queue_entry.position`; `None` sorts last (D4). Milestone 1 writes only `None`.
    pub position: Option<i32>,
    /// `queue_entry.queued_at`.
    pub queued_at: DateTime<Utc>,
    /// `queue_entry.queued_by`.
    pub queued_by: UserId,
}

/// One `queue_batch` row (MOD-12 D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueBatch {
    /// `queue_batch.id`.
    pub id: BatchId,
    /// `queue_batch.box_id`.
    pub box_id: BoxId,
    /// `queue_batch.opened_at`.
    pub opened_at: DateTime<Utc>,
    /// `queue_batch.opened_by`.
    pub opened_by: UserId,
    /// `queue_batch.closed_at`; `None` while the queue runs.
    pub closed_at: Option<DateTime<Utc>>,
    /// `queue_batch.closed_reason`; `None` exactly when `closed_at` is.
    pub closed_reason: Option<BatchClose>,
}

/// MOD-12 D4, D5: the queued items `ready` holds, in queue order: an explicit `position` first,
/// ascending, then `ready`'s own order (`ready_items` is `priority DESC, created_at, id`). An entry
/// whose item `ready` does not hold is not ready and is left out.
#[must_use]
pub fn admission_order(entries: &[QueueEntry], ready: &[ItemSummary]) -> Vec<ItemId>

/// `R-ORCH-9`'s three rungs, as `claim_run` reads them (`pg/write.rs:4374-4389`,
/// `mem.rs:4338-4350`): the box's `settings.max_concurrent_items`, else `app_setting`'s, else
/// [`DEFAULT_MAX_CONCURRENT_ITEMS`]. A blob that does not decode falls through.
#[must_use]
pub fn admission_limit(box_settings: &Value, app: &BTreeMap<String, Value>) -> u32

/// MOD-12 D6: the runs the runner may still create: the limit less every `running` run on the
/// box and every `queued` run targeted at it. Never below zero.
#[must_use]
pub fn free_slots(limit: u32, running: usize, queued: usize) -> usize
```

`admission_order` builds a `HashMap<ItemId, usize>` from `ready`'s indices, filters the entries, and
sorts by `(position.is_none(), position, rank)` with a stable sort. `free_slots` is
`usize::try_from(limit).unwrap_or(usize::MAX).saturating_sub(running.saturating_add(queued))`.
The file carries a `#[cfg(test)] mod tests` (§C.3).

`ids.rs` (`:126-129`): `/// \`queue_batch.id\` (MOD-12 plan D1): one queue activation of one box.\n BatchId,`
goes after `PersonaId`. `mod.rs`: `pub mod queue;` goes between `quota` and `relay` (it is textually after
`str_enum!`); `BatchId` joins the `ids` re-export; and a new
`pub use queue::{BatchClose, QueueBatch, QueueEntry, admission_limit, admission_order, free_slots};`
is added.

#### B.3.2 `NewRun.batch_id` (`crates/htui-core/src/model/run.rs:349-376`, D7)

```rust
    /// `run.batch_id` (MOD-12 D7): the queue batch an auto run was admitted under; `None` for a
    /// manual run. Not a [`Run`] field: only the runner and milestone 2's spend sum read it, through
    /// `batch_runs`.
    #[serde(default)]
    pub batch_id: Option<BatchId>,
```

The field goes last, after `queued_at`. `RunRow`/`Run` is unchanged (D7).

#### B.3.3 `create_run` writes and guards the batch (both stores)

**Pg** (`pg/write.rs:4108`). Inside the transaction, **after** the phantom-repo check (`:4165`) and
**before** the INSERT:

```rust
if let Some(batch) = new.batch_id {
    let open = sqlx::query_scalar!(
        r#"SELECT closed_at IS NULL AS "open!" FROM queue_batch WHERE id = $1 FOR SHARE"#,
        batch.as_uuid(),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound { entity: "queue_batch", id: batch.to_string() })?;
    if !open {
        return Err(StoreError::Constraint(format!(
            "queue_batch `{batch}` is closed: no run joins it"
        )));
    }
}
```

The INSERT (`:4170-4174`) becomes:

```sql
            INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id,
                             executing_box_id, graph_snapshot, started_by, queued_at, repo_scope,
                             batch_id)
            VALUES ($1, $2, $3, 'graph', $4, 'queued', $5, NULL, $6, $7, $8, $9::uuid[], $10)
```

The `RETURNING` list is unchanged. The new bind after `&scope` is `new.batch_id.map(BatchId::as_uuid)`.
`FOR SHARE` conflicts with `close_batch`'s UPDATE (FOR NO KEY UPDATE), so a pause and an admission
are serialised: a run never joins a batch that has already closed (H-6). The lock order is item (FOR
UPDATE, `:4121`), then batch; `close_batch` takes no item lock, so the two can never cycle. The doc's
`# Errors` gains "`NotFound { entity: "queue_batch" }` / `Constraint` for an unknown or closed batch."
`.sqlx`: `query-eb3ab1ed0aeb41e6174e244bcacd2ae57b0711d804c1960f034a01ebba399475.json` is deleted,
and two new files are added (the INSERT and the `FOR SHARE` read).

**Mem** (`mem.rs:4472`). After the repo-scope loop (`:4508-4516`, which matches Pg's order), it runs the
same check against `self.queue_batches`, with the same error variants and entity. After
`self.runs.insert(..)` it does `if let Some(batch) = new.batch_id { self.run_batches.insert(row.id, batch); }`.

#### B.3.4 Inherent queue methods: identical on `MemStore`, `PgStore` and `Backend`

```rust
/// MOD-12 D1: queue `item` on `box_id`. Idempotent: an item already queued (on any box) answers
/// its stored entry unchanged. `at` is microsecond-truncated.
/// # Errors
/// `NotFound { entity: "item" }` for an unknown item; `Constraint` for an unknown box or user.
pub async fn queue_item(&self, item: ItemId, box_id: BoxId, by: UserId, at: DateTime<Utc>) -> Result<QueueEntry>;
/// MOD-12 D9: `item` leaves whatever queue holds it; `false` when none did. Never touches a run.
pub async fn dequeue_item(&self, item: ItemId) -> Result<bool>;
/// MOD-12 D4: `box_id`'s entries, `position NULLS LAST, queued_at, item_id`.
pub async fn queue_entries(&self, box_id: BoxId) -> Result<Vec<QueueEntry>>;
/// MOD-12 D2: resume — the open batch of `box_id`, opened now under a fresh `BatchId` unless one
/// is already open (idempotent; two racing resumes answer the same row).
pub async fn open_batch(&self, box_id: BoxId, by: UserId, at: DateTime<Utc>) -> Result<QueueBatch>;
/// MOD-12 D2: `box_id`'s open batch, if any: whether its queue runs.
pub async fn open_batch_of(&self, box_id: BoxId) -> Result<Option<QueueBatch>>;
/// MOD-12 D2, D3: close `box_id`'s open batch with `reason`; `None` when none was open.
pub async fn close_batch(&self, box_id: BoxId, reason: BatchClose, at: DateTime<Utc>) -> Result<Option<QueueBatch>>;
/// MOD-12 D3: drop `box_id`'s entries whose item is `done` or `closed`; how many went.
pub async fn prune_finished_entries(&self, box_id: BoxId) -> Result<u64>;
/// MOD-12 D3, D9: the runs admitted under `batch`, `(id, status)` by `(queued_at, id)`.
pub async fn batch_runs(&self, batch: BatchId) -> Result<Vec<(RunId, RunStatus)>>;
/// MOD-12 D6: `claim_run`'s slot count — `running` runs executing on `box_id`, **not**
/// `awaiting_approval` (that is `active_runs_on_box`).
pub async fn running_runs_on_box(&self, box_id: BoxId) -> Result<usize>;
```

`Backend` dispatches each method `Memory`→`MemStore`, `Online`→`PgStore`, `Offline`→`Err(orchestration_offline())`.
They are placed after `queued_runs_on_box` (`backend.rs:583-589`). The doc `# Errors` matches its neighbours.

**Pg SQL.** Reads go in `pg/read.rs` after `:2220`, and writes in a new `impl PgStore` block at the end of `pg/write.rs`.

`queue_entries`:
```sql
SELECT e.item_id    AS "item_id: ItemId",
       i.project_id AS "project_id: ProjectId",
       e.box_id     AS "box_id: BoxId",
       e.position,
       e.queued_at,
       e.queued_by  AS "queued_by: UserId"
  FROM queue_entry e JOIN item i ON i.id = e.item_id
 WHERE e.box_id = $1
 ORDER BY e.position NULLS LAST, e.queued_at, e.item_id
```

`queue_item` is one transaction with three statements. (1) `SELECT 1 AS "one!" FROM item WHERE id = $1`:
when it returns no row, the answer is `NotFound { entity: "item" }`. (2) The insert:
```sql
INSERT INTO queue_entry (item_id, box_id, position, queued_at, queued_by)
VALUES ($1, $2, NULL, $3, $4)
ON CONFLICT (item_id) DO NOTHING
```
(3) The entry is read back with the `queue_entries` projection `WHERE e.item_id = $1` (a distinct
query text, so it gets its own `.sqlx` file). An unknown box or user is a 23503 error, which
`map_sqlx` turns into `Constraint`.

`dequeue_item`: `DELETE FROM queue_entry WHERE item_id = $1`, returning `rows_affected() == 1`.

`open_batch`: a `BatchId::new()` is minted, then:
```sql
INSERT INTO queue_batch (id, box_id, opened_at, opened_by)
VALUES ($1, $2, $3, $4)
ON CONFLICT (box_id) WHERE closed_at IS NULL DO NOTHING
```
That is followed by `open_batch_of(box_id)`. If the result is `None` (a close slipped between the two
statements), the pair is retried once. A second `None` is `StoreError::Backend("queue_batch: no open
batch after open")`.

`open_batch_of`:
```sql
SELECT id            AS "id: BatchId",
       box_id        AS "box_id: BoxId",
       opened_at,
       opened_by     AS "opened_by: UserId",
       closed_at,
       closed_reason AS "closed_reason: BatchClose"
  FROM queue_batch
 WHERE box_id = $1 AND closed_at IS NULL
```

`close_batch`:
```sql
UPDATE queue_batch
   SET closed_at = $3, closed_reason = $2
 WHERE box_id = $1 AND closed_at IS NULL
RETURNING id AS "id: BatchId", box_id AS "box_id: BoxId", opened_at, opened_by AS "opened_by: UserId",
          closed_at, closed_reason AS "closed_reason: BatchClose"
```
Bind `$2` from `reason.as_str()`.

`prune_finished_entries`:
```sql
DELETE FROM queue_entry e
 USING item i
 WHERE e.item_id = i.id AND e.box_id = $1 AND i.status IN ('done', 'closed')
```

`batch_runs`:
```sql
SELECT id AS "id: RunId", status AS "status: RunStatus"
  FROM run
 WHERE batch_id = $1
 ORDER BY queued_at, id
```

`running_runs_on_box` reuses **claim_run's literal byte for byte** (`write.rs:4391-4392`), so no new
`.sqlx` file is needed (repo memory "sqlx offline hash = literal query"):
`sqlx::query_scalar!("SELECT COUNT(*) FROM run WHERE executing_box_id = $1 AND status = 'running'", box_id.as_uuid())`
gives an `Option<i64>`, which is mapped to `usize` with `unwrap_or(0)`.

**Mem.** Three `State` fields are added after `run_commands` (`mem.rs:281`), and the explicit `from_demo`
literal (`:375`) gets `BTreeMap::new()`/`HashMap::new()` for them:

```rust
    /// `queue_entry` (MOD-12 D1), by item: an item is queued on at most one box. The entry keeps
    /// its item's project, which never moves.
    queue_entries: BTreeMap<ItemId, QueueEntry>,
    /// `queue_batch` (MOD-12 D2), by id.
    queue_batches: BTreeMap<BatchId, QueueBatch>,
    /// `run.batch_id` (MOD-12 D7), beside `runs` as `lease_owners` is: not a `Run` field.
    run_batches: HashMap<RunId, BatchId>,
```

The methods go in the `impl MemStore` block beside `queued_runs_on_box` (`:819-835`). Every `at` gets
`.trunc_subsecs(TIMESTAMPTZ_DIGITS)`. `queue_item` checks the item (`require_item`), the box
(`references_no_row("queue_entry.box_id", ..)`) and the user (`require_user`). `open_batch` mints with
`BatchId::new()`. `batch_runs` sorts by `(queued_at, id)`. `running_runs_on_box` mirrors the
`claim_run` filter at `:4605-4614`. `State::delete_project` (`:4186`) gains
`self.queue_entries.retain(|item, _| !gone.items.contains(item));` and
`self.run_batches.retain(|run, _| !gone.runs.contains(run));`, which is the CASCADE parity.

#### B.3.5 `WorkerHost` (`crates/htui-core/src/store/worker.rs`): the runner's seven

Imports gain `BatchClose, BatchId, ItemSummary, QueueBatch, QueueEntry, Scope`. These go after
`queued_runs_on_box` (`:503`):

```rust
    // -- MOD-12 M1 (plan D3, D5, D6): the queue runner's reads and its one write
    /// `Backend::ready_items`: §7.4's ready items `box_id` can take, in queue order.
    fn ready_items(&self, scope: &Scope, box_id: BoxId)
        -> impl Future<Output = Result<Vec<ItemSummary>>> + Send;
    /// `Backend::running_runs_on_box`: `claim_run`'s slot count.
    fn running_runs_on_box(&self, box_id: BoxId) -> impl Future<Output = Result<usize>> + Send;
    /// `Backend::queue_entries`.
    fn queue_entries(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<QueueEntry>>> + Send;
    /// `Backend::open_batch_of`.
    fn open_batch_of(&self, box_id: BoxId)
        -> impl Future<Output = Result<Option<QueueBatch>>> + Send;
    /// `Backend::batch_runs`.
    fn batch_runs(&self, batch: BatchId)
        -> impl Future<Output = Result<Vec<(RunId, RunStatus)>>> + Send;
    /// `Backend::prune_finished_entries`.
    fn prune_finished_entries(&self, box_id: BoxId) -> impl Future<Output = Result<u64>> + Send;
    /// `Backend::close_batch` (the drain, D3).
    fn close_batch(&self, box_id: BoxId, reason: BatchClose, at: DateTime<Utc>)
        -> impl Future<Output = Result<Option<QueueBatch>>> + Send;
```

The module doc (`:19-20`) gains "; MOD-12 M1 adds seven (plan D3, D5, D6), the 29th
`close_batch`." Both impls in `crates/htui-store/src/worker.rs` forward by path:
`PgStore::ready_items(self, scope, box_id).await` and `Backend::ready_items(self, ..)`. No other
implementor exists (the plan's verified claim was re-checked).

#### B.3.6 The `NewRun` literals that get `batch_id: None` (16 sites in 12 files)

`crates/htui-agent/tests/relay.rs:120`; `crates/htui-core/src/store/conformance.rs:4607`
(`new_run`); `crates/htui-core/src/store/mem.rs:10616` (`graph_run`); `crates/htui-mcp/tests/tools_backlog.rs:77`;
`crates/htui-orch/src/engine.rs:706, 8930, 12200, 12304`; `crates/htui-orch/tests/review_loop.rs:61`;
`crates/htui-store/tests/cache.rs:650`; `crates/htui-store/tests/pg_criteria.rs:1081` (`race_run`),
`:5460` is a struct update, so it is excluded (see below); `crates/htui-worker/src/runtime.rs:3602`;
`crates/htui/src/run_worker.rs:1618, 1659`; `crates/htui/tests/backlog.rs:979`;
`crates/htui/tests/waiting.rs:94`.

Seven struct-update sites inherit the field and are **not** edited: `conformance.rs:4801, 5156,
5351`, `mem.rs:10933, 11166`, `pg_criteria.rs:4651, 5460`. The gate for this list is
`cargo build --workspace --all-features --all-targets`.

### B.4 T4: the runner

#### B.4.1 `Engine::enqueue_in_batch` (`crates/htui-orch/src/engine.rs:646-718`)

```rust
    /// §6.2's `run`/`queue` up to the claim (unchanged doc) …
    pub async fn enqueue(
        &self,
        item: ItemId,
        mode: htui_core::model::RunMode,
        repo_scope: Option<Vec<RepoId>>,
    ) -> Result<RunId, EngineError> {
        Box::pin(self.enqueue_with(item, mode, repo_scope, None)).await
    }

    /// MOD-12 D6, D7: [`Self::enqueue`] for the queue runner — an `auto` run over the item's
    /// default scope, recorded under `batch`. `create_run` refuses a closed or unknown batch.
    ///
    /// # Errors
    /// [`Self::enqueue`]'s.
    pub async fn enqueue_in_batch(&self, item: ItemId, batch: BatchId) -> Result<RunId, EngineError> {
        Box::pin(self.enqueue_with(item, RunMode::Auto, None, Some(batch))).await
    }

    /// The body `enqueue` had, with `batch_id` threaded into `NewRun`.
    async fn enqueue_with(
        &self,
        item: ItemId,
        mode: htui_core::model::RunMode,
        repo_scope: Option<Vec<RepoId>>,
        batch: Option<BatchId>,
    ) -> Result<RunId, EngineError>
```

The `NewRun` at `:706` gets `batch_id: batch`. The `Box::pin`s keep `start_run`'s and `run_request`'s frames
where they are (H-9). The public signature of `enqueue` is unchanged (D7).

#### B.4.2 `crates/htui-worker/src/runtime.rs`

**`Shared`** (`:185-240`) gains a field, and `assemble` (`:1147-1186`) initialises it with
`AtomicBool::new(false)`:
```rust
    /// MOD-12 D8: a sweep asked for while one ran; the running one sweeps once more before it ends.
    sweep_again: AtomicBool,
```

**`sweep_with`** (`:1211-1250`) becomes `self.shared.prune(); spawn_sweep(&self.shared, host, sink);`,
and today's body moves into:

```rust
/// D190's one-sweep-at-a-time task (MOD-12 D8: callable from a task's tail). A sweep asked for
/// while one runs is not dropped: `sweep_again` makes the running one go round once more.
fn spawn_sweep<H: htui_core::store::WorkerHost, P: ReplySink>(
    shared: &Arc<Shared<P>>,
    host: &H,
    sink: &P,
)
```

The body is today's, with two changes: on a failed `compare_exchange` it does
`shared.sweep_again.store(true, Ordering::SeqCst)` before returning, and the spawned task runs
`loop { sweep_once(ctx.clone()).await; if !ctx.shared.sweep_again.swap(false, Ordering::SeqCst) { break; } }`
under the existing `Swept` guard.

**`sweep_once`** (`:1918-1969`). The box row is read once and its settings are kept:
```rust
    let settings = match host.box_row(box_id).await {
        Ok(Some(row)) => row.settings,
        Ok(None) => return,
        Err(err) => { tracing::debug!(%err, "the sweep could not read this box's executor"); return; }
    };
    let executor = Executor::of(&settings);
```
The tail becomes:
```rust
    if claims {
        // MOD-12 D6: admission first, under the same guard, so the claim scan below claims and
        // walks what it created; there is no second admission path.
        Box::pin(admit(&ctx, box_id, &settings)).await;
        claim_scan(&ctx, box_id).await;
    }
```
The doc comment gains a sentence: "Between the adoption and the claim scan, the queue runner admits
this box's queued ready items (MOD-12 D6, [`admit`])."

**`admit`**:
```rust
/// MOD-12 D3, D5, D6: the queue runner, once per sweep of the box's executing process (I-1 is
/// `sweep_once`'s early return). Nothing without an open batch (D2). First the D3 prune: entries
/// whose item is `done` or `closed` go, and a batch left with no entry and no live run of its own
/// closes `drained`. Then `ready_items` over the entries' projects (D5), kept to the queued ones in
/// queue order (D4, [`admission_order`]), and up to [`free_slots`] of them enqueued `auto` under
/// the batch (D7). Every refusal is logged at `debug` and the next entry tried; one that writes a
/// note on its item (rung 4, missing tags) is visible there. A `Constraint` re-reads the open
/// batch and stops when it is no longer this one (a pause won the race, H-6). The `Kit` is read
/// only when something is to be admitted.
async fn admit<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: &TaskCtx<H, P>,
    box_id: BoxId,
    box_settings: &Value,
)
```

The steps, in order:
1. `open_batch_of(box_id)`: `Ok(None)` returns, and `Err` is logged at `debug` and returns.
2. `prune_finished_entries`: `Err` is logged at `debug` and returns.
3. `queue_entries`. When it is empty, `batch_runs(batch.id)` is read; if every status `is_terminal()`, it calls
   `close_batch(box_id, BatchClose::Drained, ctx.shared.clock.now())` and logs at `info`
   (`batch`, `"the queue drained"`). It then returns.
4. A scope is built:
   `Scope { workspace_id: WorkspaceId::default(), project_ids }` (distinct `project_id`s in entry
   order; see H-4). Then `ready_items(&scope, box_id)`, followed by `admission_order(&entries, &ready)`. If the
   order is empty, it returns.
5. `running_runs_on_box`, `queued_runs_on_box(..).len()` and `app_settings()`, then
   `free = free_slots(admission_limit(box_settings, &app), running, queued)`. If `free` is 0, it returns.
6. `Kit::read(&ctx.shared, &ctx.host, false)`, where `Err` is logged at `warn` and returns; then the `driver` closure and
   `kit.engine(&driver)` are built exactly as `adopt` builds them (`:1991-1999`).
7. For each `item` in the order, while `admitted < free` and `!ctx.shared.walks.closed()`:
   `engine.enqueue_in_batch(item, batch.id)`.
   - `Ok(run)`: `admitted += 1`, then
     `ctx.shared.publisher.publish(&RunFrame { item, run: Some(run), kind: FrameKind::Started })`
     and `tracing::info!(%run, %item, batch = %batch.id, "the queue admitted a run")`.
   - `Err(EngineError::Store(StoreError::Constraint(_)))`: logged at `debug`. `open_batch_of` is re-read,
     and if it is not `Some(b)` with `b.id == batch.id`, the function returns.
   - Any other `Err(err)` is logged with `tracing::debug!(%item, %err, "the queue skipped an entry")`.

**D8 wake on rest**:
```rust
/// MOD-12 D8: a walk of this process that rested may have freed a slot. When this box's queue
/// runs, a sweep is asked for now rather than at the next tick (the lease period, 120 s, on a
/// TUI box). Only a task whose run is read to be at rest (not `queued`, not `running`) asks, so
/// a refused claim, which leaves its run `queued`, never loops; nothing without the claim scan.
async fn wake_on_rest<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>)
```

The body runs these checks in order:
1. It returns at once unless `ctx.shared.claim_scan` is set.
2. It takes `ctx.tag.run.get()`, then calls `ctx.host.run(run)`. Any of `Ok(Some(Run { status: Running | Queued, .. }))`, `Ok(None)` or `Err(_)` returns.
3. It calls `registered_box`, then `open_batch_of`.
4. When that returns `Ok(Some(_))`, it calls `spawn_sweep(&ctx.shared, &ctx.host, &ctx.sink)`.

In the `spawn_supervised` tail (`:1781-1783`), the order becomes `ended.fetch_add` →
`wake_on_rest(&ctx).await` → `retry_claims(&ctx).await`. `retry_claims` and `testing::retry_claims`
keep their behaviour (`run_worker.rs:2105-2130` pins it).

**Admission and `Kit::read`.** `Kit::read(.., false)` costs seven host reads, plus a one-time parts build
on the first use in a process. The reads are `box_info`, `box_row`, `singletons`, `this_user`,
`app_settings`, `box_profile` and `agents` (`:918-986`), and `admit` pays for them only when there is an
open batch, a ready entry and a free slot. `kit.walking_refusal()` and `Tails::HandBack` do not matter
for admission. `admit` runs only when `role.executes(executor)` is true (`sweep_once`'s early
return), which means `(Tui, Tui)` or `(Worker, Worker)`. `Role::tails` is then always
`Tails::Walk` (`runtime.rs:78-85`), and `walking_refusal` needs `Executor::Other`, which
`executes` already refuses (`:69-75`). A TUI on a `worker` box never admits; that box's worker does.

### B.5 T5: Backlog `Q`/`P` (D9)

#### B.5.1 `crates/htui/src/store_worker.rs`

The new `StoreRequest` variants go after `SetToolPaths` (`:330-341`); their docs follow the `SetAgentOnBox` style:
```rust
    /// MOD-12 D9: this box's queue — its entries and whether it runs. Answered with
    /// [`StoreReply::Queue`]. Offline: `DATABASE_UNREACHABLE`.
    QueueState,
    /// MOD-12 D9: queue `item` on this box (idempotent). Answered with [`StoreReply::QueueWritten`].
    QueueItem {
        /// The item.
        item: ItemId,
    },
    /// MOD-12 D9: take `item` out of the queue; never cancels a run. [`StoreReply::QueueWritten`].
    DequeueItem {
        /// The item.
        item: ItemId,
    },
    /// MOD-12 D2, D9: resume — open a batch on this box (idempotent); the loop sweeps at once (D8).
    ResumeQueue,
    /// MOD-12 D2, D9: pause — close this box's open batch `paused`; running runs continue.
    PauseQueue,
```

`name()` (`:1012-1150`) gains `"queue_state"`, `"queue_item"`, `"dequeue_item"`, `"resume_queue"` and `"pause_queue"`.
A `pub const QUEUE_REQUEST_NAMES: [&str; 5]` with those five, in that order, is placed beside `LIST_DIR`.

The `StoreReply` variants go after `PersonaWritten`. The new types go just below `StoreReply`:
```rust
    /// Answer to [`StoreRequest::QueueState`].
    Queue(QueueView),
    /// Answer to the four queue writes: what was done, and the queue after it.
    QueueWritten {
        /// The write.
        write: QueueWrite,
        /// The queue after it.
        view: QueueView,
    },

/// MOD-12 D9: this box's queue as the Backlog needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueView {
    /// The queued items, `position NULLS LAST, queued_at, item_id`.
    pub entries: Vec<ItemId>,
    /// The open batch; `None` = paused (D2).
    pub open_batch: Option<BatchId>,
}

/// MOD-12 D9: what one queue write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueWrite {
    /// `item` is queued.
    Queued { item: ItemId },
    /// `item` left the queue; `was_queued` is `false` when it was not in it.
    Dequeued { item: ItemId, was_queued: bool },
    /// A batch is open; `already` when one was.
    Resumed { already: bool },
    /// The batch closed `paused` with `live` of its runs `queued` or `running`; `already` when
    /// none was open.
    Paused { live: usize, already: bool },
}
```
Each field carries a `///` doc.

The serve function goes beside `read_items` (`:2030`):
```rust
/// MOD-12 D9: the five queue requests over `backend`'s inherent queue methods. This box is
/// `box_info()`'s, the author `this_user()`, every time `Utc::now()` truncated to the microsecond.
async fn serve_queue(backend: &Backend, request: &StoreRequest) -> StoreResult<StoreReply>
```
Before anything, the box comes from `box_info()?.map(|i| i.box_id)` and fails with
`NotFound { entity: "box", id: "this box" }` (the `AnswerPermission` arm, `:1990-1998`).
`ResumeQueue` reads `open_batch_of` before `open_batch` to set `already`.
`PauseQueue`'s `live` counts `batch_runs(closed.id)` entries in `Queued | Running`.
Any other variant answers `StoreReply::Failed { request: request.name(), message: "not a queue request".into() }`.
The `try_serve` arm (wildcard-free match) is:
`StoreRequest::QueueState | StoreRequest::QueueItem { .. } | StoreRequest::DequeueItem { .. } | StoreRequest::ResumeQueue | StoreRequest::PauseQueue => serve_queue(backend, request).await?,`

**D8 resume wake.** This goes in the loop's `other =>` arm (`:2718-2735`), after `try_serve` and before the
`match served`:
```rust
if matches!(other, StoreRequest::ResumeQueue) && served.is_ok() {
    // MOD-12 D8: a resumed queue is admitted at the next sweep, which is now.
    runs.sweep(&backend, &tx);
}
```

#### B.5.2 `crates/htui/src/ui/tabs/backlog/mod.rs`

A tab field `queue_intent: Option<QueueIntent>` is added and initialised to `None` in `new()` (`:159-170`):
```rust
/// MOD-12 D9: the toggle `Q` or `P` asked for, decided by the `QueueState` read it is waiting on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum QueueIntent {
    /// `Q` on this item (its key, for the status line).
    Item { id: ItemId, key: String },
    /// `P`.
    Pause,
}
```

`on_key` (`:651-700`) gains two arms after `'e'`, both below the capture and modifier guards:
```rust
            // MOD-12 D9: `Q` toggles the cursor item's queue membership, `P` this box's queue.
            // Each reads the queue first; the reply decides the write (no cached state goes stale
            // when a worker drains the batch).
            KeyCode::Char('Q') => self.toggle_queued(ctx),
            KeyCode::Char('P') => {
                self.queue_intent = Some(QueueIntent::Pause);
                ctx.request(StoreRequest::QueueState);
            }
```

`toggle_queued` behaves as follows:
- With no item selected (`self.item()` is `None`, `:174`), it emits `Action::Error(NO_ITEM_TO_QUEUE)`.
- With an item whose status is `Done` or `Closed`, it emits
  `Action::Error(format!("{key} is {status}: nothing to queue"))` and sends nothing.
- Otherwise it sets the intent and requests `QueueState`.

`on_reply` (`:703`) gains three arms ahead of the list's:
- `StoreReply::Queue(view)` takes the intent and sends the write. For `Item`, the write is
  `DequeueItem` if `view.entries.contains(&id)` and `QueueItem` otherwise. For `Pause`, it is
  `PauseQueue` if `view.open_batch.is_some()` and `ResumeQueue` otherwise. With no intent, it is
  ignored.
- `StoreReply::QueueWritten { write, view }` emits `Action::Error(queue_sentence(write, key, view))`.
  The `key` comes from the intent it remembered, or failing that from `self.items`, or failing that
  from the id. The arm then clears the intent.
- `StoreReply::Failed { request, .. } if QUEUE_REQUEST_NAMES.contains(request)` clears the intent
  (App already printed the failure).

```rust
/// MOD-12 D9: the status line after a queue write.
fn queue_sentence(write: &QueueWrite, key: &str, view: &QueueView) -> String
```

| Write | Sentence |
|---|---|
| `Queued` | `queued {key} ({n} in queue, {running\|paused})` |
| `Dequeued { was_queued: true }` | `dequeued {key} ({n} in queue)` |
| `Dequeued { was_queued: false }` | `{key} was not queued` |
| `Resumed { already: false }` | `queue resumed` |
| `Resumed { already: true }` | `queue already running` |
| `Paused { already: true, .. }` | `queue already paused` |
| `Paused { live: 0, .. }` | `queue paused` |
| `Paused { live: 1, .. }` | `queue paused — 1 run still running` |
| `Paused { live: n, .. }` | `queue paused — {n} runs still running` |

Here `n` is `view.entries.len()`. The constant is `const NO_ITEM_TO_QUEUE: &str = "select an item to queue";`.

#### B.5.3 `crates/htui/src/app/mod.rs` (after `:160-170`)

```rust
    // MOD-12 D9: `Q` toggles the cursor item's queue membership and `P` this box's queue, in the
    // Backlog's own arms, which always consume the key; these rows are the help box's half.
    for (key, help) in [('Q', "queue / dequeue"), ('P', "pause / resume queue")] {
        app.keymap.bind(Binding { scope: KeyScope::Tab(BacklogTab::ID),
            key: KeyChord::new(KeyCode::Char(key), KeyModifiers::NONE),
            action: Action::Tab(TabAction::Focus(BacklogTab::ID)), help });
    }
```
**VERIFY**: check that `harness.key("Q")` reaches the tab as `Char('Q')`. The `N` row binds `NONE` for an
uppercase letter (`:163`), so binding Q the same way matches the precedent. No `.snap` contains a Backlog help row
(`grep -l 'open graph' **/*.snap` is empty), so no snapshot changes.

### B.6 T6: documents

- `docs/ANA-2.md` §4.10, after the downgrade paragraph (`:1494`), gains a forward note: "**As built (MOD-12
  M1).** Queue membership is a `queue_entry` row (an item queued on at most one box), and a queue
  activation is a `queue_batch` row whose `id` each auto run records in `run.batch_id`
  (`0016_auto_queue.sql`). A box's queue runs iff it has an open batch: pause closes it, and resume
  opens a new one. Queue order is `position NULLS LAST`, then `ready_items` order."
- PRD: the M1 row becomes `complete`, and the plan path is kept.
- `HANDOFF.md`: a phase note is added, and the "Live coordinates" line (`:50-53`) changes to say
  migrations run through `0016_auto_queue` (next `0017`, cache still `0005`), with `htui-orch` `CASES` 103.
- Validate with `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## C. Tests first (TDD), per task

### C.0 T0: `crates/htui-store/tests/migrations.rs` and `connect.rs`

Count literals:
- `migrations.rs:96`: `vec![1, …, 15, 16]`, and the message gains "and MOD-12's 0016_auto_queue.sql".
- `:1088-1089`, `:1189-1190`, `:1198-1199`, `:1221-1222`: 15 becomes 16 and "fifteen" becomes "sixteen";
  the text "through MOD-11's 0015_command_queue.sql" becomes "through MOD-12's 0016_auto_queue.sql".
- `:1373-1375`: likewise.
- `connect.rs:140-143`, `:156-158`, `:242-244`: likewise.

Two more literals the plan's grounding missed:
- `TABLES` (`:24-76`): append `// 0016_auto_queue.sql (MOD-12)` followed by `"queue_batch"` and
  `"queue_entry"`. Also update the doc ("The 44 tables …") and `:119-133` (42 becomes 44, with both
  messages naming "MOD-12's 0016_auto_queue.sql adds queue_batch and queue_entry").
- Comment set: add a `MOD12_COLUMN_COMMENTS: &[(&str, &str, &str)]` with the twelve texts of §B.0,
  verbatim, in migration order. Chain it after `MOD11_COLUMN_COMMENTS` at **all three** sites
  (`:620-627`, `:678-685`, `:695-702`). Then change "forty-six" to "fifty-eight" and "forty-seventh" to
  "fifty-ninth" (`:667-668`, `:708`), and add "MOD-12" to the assertion message list (`:641-642`).
  Without this, `the_ana_column_comments_are_present_and_verbatim` fails, because `run` is in its
  table set and `run.batch_id` is commented.

New tests. Each uses `common::fresh_db()`, except the cascade test, which uses `demo_db()`.

| Test | Asserts |
|---|---|
| `the_auto_queue_tables_and_column_exist` | `information_schema.columns` for both tables gives names, `data_type` and `is_nullable` as in §B.0. `run.batch_id` is `uuid`/`YES`. `pg_indexes` shows `uq_queue_batch_open` as `CREATE UNIQUE INDEX uq_queue_batch_open ON public.queue_batch USING btree (box_id) WHERE (closed_at IS NULL)`, plus `idx_queue_entry_box` and `idx_run_batch` (**VERIFY**: compare against the exact normalised text Postgres prints, then pin it) |
| `a_second_open_batch_on_one_box_is_refused` | Raw inserts of two open batches for the seeded box: the second error, through `htui_store::map_sqlx`, is `StoreError::Constraint` naming `uq_queue_batch_open`. After `UPDATE … closed_at = now(), closed_reason = 'paused'` the second insert lands, and two closed batches coexist |
| `a_batch_closes_with_its_reason_and_only_with_it` | `closed_at` without a reason, and a reason without `closed_at`, are both refused with `chk_queue_batch_closed`. `closed_reason = 'cancelled'` is refused with `chk_queue_batch_closed_reason` |
| `a_queue_entry_goes_with_its_item` | (`demo_db`) Insert a `queue_entry` for `HTUI_ANA_2` on `ids::BOX` by `ids::USER`. `WriteStore::delete_project(&db.store, PROJECT_HTUI)` succeeds, and `count(queue_entry) == 0` |
| `a_run_names_only_an_existing_batch` | (`demo_db`) `UPDATE run SET batch_id = <fresh uuid> WHERE id = RUN_2` is refused with `fk_run_batch`. With an inserted batch's id, the update lands |

### C.1 T1

| Test | File | Asserts |
|---|---|---|
| `ready_items_are_in_queue_order` (new) | `htui-core` `mem.rs` tests (by `:12588`) | Three items minted untagged in `PROJECT_HTUI` with priorities 0, 1, 0. The order under scope `[PROJECT_HTUI]` is `[p1, ANA_2, p0a, p0b]`. `ANA_2` is priority 0 with `created_at = demo_at(0, 1)`, which is earlier than the mints |
| `ready_items_break_a_created_at_tie_by_id` (new) | `mem.rs` tests | Over `MemStore::demo().with_clock(<fixed>)`, two mints share `created_at` and come back in `ItemId` order |
| `ready_items_order_by_priority_then_created_at` (new) | `pg_criteria.rs` | On `demo_db` and `MemStore::demo()`, mint the same three. Pg ids are in the expected order. After `UPDATE item SET created_at = $1` on two of them, Pg ties break by id. The Mem order of the shared rows matches |
| `inherent_orchestration_reads_answer_the_fixture` (existing, `:4121-4132`) | unchanged | Pg == Mem still holds, since both sort identically |
| `ready_here_equals_ready_items_for_this_box` (`store_worker.rs:3439`) | adjust | Compare **row sets**: sort both vectors by `id` before `assert_eq!`. Doc: "the same rows as `MemStore::ready_items` (which `pg_criteria.rs` pins against `PgStore`); `ready_here` keeps the Backlog's display order, `ready_items` is queue order (MOD-12 D4)". Add `assert_eq!(ready, display_order_of(ready))`: the rows are in the `items` order `items_of(…, ready: Some(true))` returned, filtered |
| `ready_here_with_an_unregistered_box_keeps_only_untagged_items` (`:3477`) | adjust | Same set comparison. Keep `ids_of(&ready) == [HTUI_ANA_2, AGY_FIX_1]`, which is display order |

**The two `ready_here` tests would pass by coincidence today.** The platform scope is `[htui, agy]`
(`fixtures.rs:692-710`), every ready demo item has priority 0, and `created_at = demo_at(0, n)` follows key
order (`fixtures.rs:1092-1118`). They are still reworded, because order equality between the two reads is not the contract.

### C.2 T2

| Test | Kind | Asserts |
|---|---|---|
| `auto_mode_downgrades_a_soft_gate_to_never` | `graph.rs` unit | `effective_gate(Auto, g, false) == Never` for every `g` in `Gate::ALL` |
| `auto_mode_keeps_a_hard_gate` | unit | `effective_gate(Auto, g, true) == g` |
| `manual_mode_keeps_every_gate` | unit | `effective_gate(Manual, g, h) == g` for every g and h |
| `an_auto_snapshot_downgrades_only_its_soft_phases` | `graph.rs` tokio | `resolve(.., RunMode::Auto, ..)` on `feat_1`: every phase with `gate_hard` keeps `gate`, every other phase is `Never`, and `snapshot.mode == Auto`. At least one phase of each kind exists |
| `auto_mode_skips_soft_gates` | case, criterion 23 first half | `feat_3_gated(&orch, Gate::Always, \|p\| p.gate_hard = false)`. `start_in(.., Auto)` rests `Done`. `run.mode == Auto`. Every snapshot phase is `(Always, Never)`. There are 4 steps, every one `Done` with `gate_outcome == Some(Skipped)`, and the item is `Done` |
| `auto_mode_parks_at_a_hard_gate` | case, criterion 23 second half | `feat_3_gated(&orch, Gate::Always, \|_\| {})` (the seeded `gate_hard` is kept). Auto start, then `answer(Approved)` while the rest parks. Parked positions == positions of snapshot phases with `gate_hard`, which must be non-empty. Soft steps are `Skipped` and hard steps `Approved`. The run ends `Done` |
| `a_manual_snapshot_keeps_its_gates` | case, criterion 24 | `feat_3_gated(Always, gate_hard=false)`, Manual start, parks at 0. Every phase has `gate_effective == Always`. `run.mode == Manual`. `approve(&orch, run, 1)` parks again at 1: no soft gate was skipped |
| `cases_are_unique_and_counted` / `cases_len_is_pinned` | count | 103. Both messages gain "and MOD-12 M1's three (criterion 23's two halves, criterion 24)" |

Stack headroom is covered by three things. The three cases sit in a frame of their own (`auto_mode_case`), every arm is
`Box::pin`ned, and the bodies use only existing helpers. That is the shape that took 85 arms off the
stack (`conformance.rs:888-892`) and that `command_queue_case` repeats. Gate T2 with
`cargo test -p htui-orch --all-features -- --no-fail-fast` and grep for `SIGABRT|overflowed`. Optionally measure
`RUST_MIN_STACK=1572864 cargo test -p htui-orch --all-features --lib every_case_name_dispatches`
before and after, following the repo memory on stack headroom.

### C.3 T3

`model/queue.rs` unit tests:

| Test | Asserts |
|---|---|
| `batch_close_matches_check_list` | `BatchClose::ALL` texts are `["paused", "drained"]` |
| `admission_order_puts_positions_first_then_ready_order` | Entries `[x(None), y(Some 2), z(Some 1)]` with ready `[x, y, z]` give `[z, y, x]` |
| `admission_order_drops_entries_that_are_not_ready` | An entry absent from `ready` is left out, and a ready item that is not queued is never added |
| `admission_limit_walks_box_then_app_then_default` | The box key wins. If it is absent or the blob is undecodable, `app["max_concurrent_items"]` wins. Otherwise `DEFAULT_MAX_CONCURRENT_ITEMS` applies |
| `free_slots_counts_running_and_queued_and_never_underflows` | `(2,1,0)→1`, `(2,1,1)→0`, `(1,3,0)→0` |

`mem.rs` unit tests:

| Test | Asserts |
|---|---|
| `queue_item_is_idempotent_and_dequeue_reports_membership` | The second `queue_item` answers the stored entry, `dequeue_item` gives `true` then `false`, and an unknown item gives `NotFound { entity: "item" }` |
| `open_batch_answers_the_open_one` | Two `open_batch` calls give the same id. `open_batch_of` returns `Some` until the close |
| `close_batch_records_its_reason_once` | `Some` with `closed_reason == Paused` and `closed_at == at`, then `None` |
| `prune_finished_entries_drops_done_and_closed_items_only` | ANA-1 (done) and ANA-2 (open) queued: `1` is removed and ANA-2 remains |
| `create_run_records_its_batch_and_refuses_a_closed_one` | `batch_runs` gives `[(run, Queued)]`. A closed batch is a `Constraint` and an unknown one `NotFound { entity: "queue_batch" }`, and in both cases nothing is written |
| `delete_project_takes_queue_entries_and_run_batches` | After `delete_project(PROJECT_HTUI)`, no entry and no run-batch pair remains |
| `running_runs_on_box_counts_running_only` | A claimed run counts 1. Once parked (`awaiting_approval`) it counts 0, while `active_runs_on_box` still counts 1 |

In `pg_criteria.rs` (new), all on `demo_db` plus `MemStore::demo()`:

| Test | Asserts |
|---|---|
| `queue_surface_answers_alike_on_both_stores` | The same calls with the same `at` give equal `QueueEntry` values, plus the same `queue_entries`, `dequeue_item` booleans, `prune_finished_entries` counts and `running_runs_on_box`. The batch shape (`box_id`, `opened_at`, `closed_reason`) matches; ids differ because each store mints its own. Error variants match for an unknown item and box |
| `create_run_round_trips_its_batch` | On both stores, `batch_runs(batch) == [(run, Queued)]`. A closed batch gives `Constraint` and an unknown one `NotFound { entity: "queue_batch" }` |
| `two_resumes_open_one_batch` | `tokio::join!` of `open_batch` on two `PgStore` handles gives equal ids, and `SELECT COUNT(*) … closed_at IS NULL` is 1 |
| `a_pause_and_an_admission_serialise_on_the_batch_row` | (Pg only) tx A runs `create_run` with a batch and, before commit, a concurrent `close_batch`. Either the run lands with the batch and the close commits after it, or `create_run` reports `Constraint` "closed". A run is never left in a batch that had already closed when the run was inserted |

Gates for T3:
- `cargo test -p htui-core --all-features`
- `cargo test -p htui-store --all-features -- --test-threads=1`
- `cargo sqlx prepare --check`, run inside `crates/htui-store` against the migrated scratch DB
- `cargo build --workspace --all-features --all-targets`
- `cargo clippy --workspace -- -D warnings`

### C.4 T4: `crates/htui-worker/tests/auto_queue.rs` (CREATE)

**The harness is self-contained.** No `htui-worker` test harness exists (`tests/` holds only `deps.rs`), and
`htui`'s fixture is private to `run_worker.rs`. Every dev-dep it needs is already present
(`crates/htui-worker/Cargo.toml` dev-deps): `htui-agent`'s `FakeDriver`/`Script` and
`TransportBuilder`, `htui-orch`'s `FakeIsolator`/`FakeVerifier`, `htui-store`'s `demo` + `test-support`
(`Backend::memory`, `testkit::demo_db`), and `tokio` `test-util`. It copies, trimmed, from
`crates/htui/src/run_worker.rs`:
- `scripted_row` (`:221-234`), `ready_on_box` (`:237-250`) and `seeded` (`:260-299`, which disables the fixture's agents,
  adds one `acp` row plus its `agent_box`, adds the htui primary repo, and cancels `RUN_2`).
- An `OutputAuthor` (`:628-650`).
- A `Hold` `TransportBuilder` whose sessions emit `Done` after acquiring one permit from a shared
  `Arc<tokio::sync::Semaphore>`. A `Harness::release(n)` adds permits, and `Harness::open()` adds
  `Semaphore::MAX_PERMITS / 2`.
- A `TestSink` implementing `htui_worker::ReplySink` with `Addr = u64` and `Subscriber = u64`, collecting
  `RunReply`s. `Unaddressed` cannot address a manual `StartRun`, since its `Addr` is `Infallible` (`address.rs:81`).
- A `runtime()`:
  `RunRuntime::<Backend, TestSink>::with_parts(FakeIsolator, FakeVerifier, factory).with_author(..).with_sweep_every(Duration::from_secs(3600))`.
  The claim scan is on by default.
- `mint_ana(store, title, priority) -> ItemId`, a `NewItem` in `PROJECT_HTUI`/`KIND_HTUI_ANA`. The analysis graph is
  `research` (soft) then `verdict` (hard), so an auto run walks `research` skipped and parks at
  `verdict`, holding no slot.
- `eventually(|| async { … })`, polling every 10 ms for up to 20 s.

The items are ANA because a review phase needs a parsed verdict, which `OutputAuthor` does not write.

| # | Test | Asserts |
|---|---|---|
| a | `a_paused_box_admits_nothing` | Two queued ready items and no open batch. After `sweep_with` and `settle`, no item has a run |
| b | `a_resumed_box_admits_in_queue_order_up_to_the_cap` | Hold sessions. Three items with priorities 0, 2, 1, queued in that order. `open_batch` then sweep, and `eventually` sees two runs, for p2 and p1 (D4), both `mode == Auto` with `batch_runs(batch)` holding exactly them. p0 has none while both hold (cap 2 on the demo box, `fixtures.rs:487`). Open, then `settle`: p0 gets a run through the D8 wake, with no second `sweep_with` |
| c | `a_queued_item_that_is_not_ready_waits_then_runs` | The entry's item is `Blocked`, so a sweep gives no run. `transition(Blocked → Open)`, sweep again, and it is admitted |
| d | `a_queued_item_missing_a_tag_waits_unblocked` | `required_tags = ["cuda"]` gives no run, the item stays `Open` with no new note, and the next entry is admitted in the same sweep (PRD D1) |
| d2 | `an_enqueue_refusal_is_skipped_and_the_next_entry_admitted` | The first entry's item is repointed (the conformance `repoint` recipe, `conformance.rs:1115-1158`) at a clone whose first phase names `template_name = "no-such-template"`, which makes `ResolveError::NoTemplate` at enqueue. The item has no run and stays `Open`, and the second entry gets a run in the same sweep |
| e | `pausing_stops_admission_and_leaves_running_runs_running` | Criterion 27, pause half. Cap is 2 and three items are queued. Hold, admit two, then `close_batch(Paused)` while both are running. Release and settle: both rest (`AwaitingApproval`) and their runs were never cancelled. The third has no run. `open_batch_of` is `None`, and the closed row's reason is `Paused` |
| f | `a_drained_batch_closes_drained` | One item queued and admitted, parks at `verdict`. `finish_run(Cancelled)`, then `transition(Open → Closed)` (**VERIFY** the legal path, `model/item.rs:46-60`). Sweep: the entry is pruned and the batch closes with `Drained` |
| f2 | `a_blocked_entry_keeps_the_batch_open` | An entry whose item is `Blocked`, and no live runs. After the sweep the batch is still open and the entry still present (D3) |
| g1 | `a_manual_run_holds_off_an_overlapping_auto_run` | Criterion 26, manual first. Hold. Manual `StartRun` on item M through `serve_request` with `TestSink`, and it is running. Queue A (same project, so primary repo scope) and sweep: A's run exists and stays `Queued` (the claim refused it as `Overlaps`). Release: M rests, then A is claimed and rests |
| g2 | `an_auto_run_holds_off_an_overlapping_manual_run` | Same, auto first. The manual `StartRun` reply is `Failed` containing "overlaps run", and its run is claimed after A rests |
| h | `two_runtimes_on_one_store_admit_each_item_once` | Two runtimes over one `Backend::memory(store)`, two queued items, one open batch, `sweep_with` on both, settle both. Each item has exactly one run (`store.runs(item).len() == 1`) |
| i | `a_tui_on_a_worker_box_admits_nothing` | `edit_box` sets `executor: "worker"`. A `Role::Tui` runtime sweeps and nothing is admitted. The same store under `.with_role(Role::Worker)` admits |
| k | `a_rested_walk_wakes_the_sweep` | Box cap 1 (`edit_box` sets `max_concurrent_items: 1`), two items queued, open batch. One `sweep_with`, then `settle`. Both items have runs, the second created after the first run's `verdict` park. Only the D8 wake can have admitted it, since the ticker is 3600 s and there is a single explicit sweep |
| pg-h | `two_runtimes_on_one_database_admit_each_item_once_pg` | (h) over `htui_store::testkit::demo_db()`, two `RunRuntime<PgStore, TestSink>`. It returns early without `HTUI_TEST_DATABASE_URL` |
| pg-g | `manual_and_auto_overlap_serialise_on_postgres` | (g1) over `demo_db` |

The Pg seeding is `seeded`'s writes issued through `WriteStore` on `db.store`. **VERIFY**:
`htui_core::fixtures::edit_agent`'s bound accepts `PgStore`; if not, inline `edit_agent`'s
`WriteStore::edit_agent` CAS.

The gates are:
- `cargo test -p htui-worker --all-features -- --test-threads=1`
- `cargo test -p htui-orch --all-features -- --no-fail-fast` (grep `SIGABRT`)
- `cargo test -p htui --all-features run_worker -- --test-threads=1`, which runs the existing runtime suite with the new tail

### C.5 T5

`crates/htui/tests/backlog.rs` uses `Harness::demo()`/`backlog_over(store)`. The cursor arrives on `ANA-1`
(done), and `TO_ANA_2 = 1` (`:93-96`). Every test then calls `drive_to_end()`.

| Test | Asserts |
|---|---|
| `q_queues_the_cursor_item_and_says_so` | On ANA-2, `Q`: `store.queue_entries(ids::BOX)` is `[ANA_2]` and the status is `queued ANA-2 (1 in queue, paused)` |
| `a_second_q_dequeues_it` | `Q`, `Q`: no entries, and the status is `dequeued ANA-2 (0 in queue)` |
| `p_resumes_then_pauses_the_queue` | `P`: `open_batch_of` is `Some`, status `queue resumed`. `P` again: `None`, with the closed row's reason `Paused`, status `queue paused` |
| `pausing_names_the_runs_still_running` | `store.open_batch(..)`, then `create_run(NewRun { batch_id: Some(b), item: HTUI_ANA_2, graph_snapshot: bare_snapshot(), .. })`. `P` gives `queue paused — 1 run still running` |
| `q_on_a_done_item_queues_nothing` | On ANA-1, `Q`: status `ANA-1 is done: nothing to queue`, and there are no entries |
| `offline_q_is_refused_with_the_database_sentence` | `offline_backlog` (`:2165-2187`), `Q` on ANA-2: the status contains `DATABASE_UNREACHABLE` |
| `q_and_p_are_on_the_backlog_help_line` | Mirroring `m_is_on_the_backlog_help_line` (`:267`), the help line contains `Q queue / dequeue` and `P pause / resume queue` |

In `crates/htui/src/store_worker.rs` tests:

| Test | Asserts |
|---|---|
| `queue_requests_are_named_as_queue_request_names_lists_them` | The five `name()`s equal `QUEUE_REQUEST_NAMES` (the `box_requests_are_named…` shape, `:4680`) |
| `queue_state_answers_the_box_queue` | `serve(&Backend::memory(demo), &QueueState)` gives `Queue(QueueView { entries: [], open_batch: None })`, and after `queue_item` it gives `[ANA_2]` |
| `pause_on_a_paused_box_says_already` | `PauseQueue` with no open batch gives `QueueWritten { write: Paused { already: true, live: 0 }, .. }` |

In `crates/htui/src/run_worker.rs` tests:

| Test | Asserts |
|---|---|
| `resuming_the_queue_sweeps_at_once` | D8, resume half. `Fixture::new()`, `runtime().with_sweep_every(3600 s)`, `Worker::spawn`. Then `store.queue_item(HTUI_ANA_2, ..)` and send `ResumeQueue`: the reply is `QueueWritten { Resumed { already: false } }`, and within `PATIENCE` ANA-2 has a run with `mode == Auto` that appears in `batch_runs` |

The gates are `cargo test -p htui --all-features -- --test-threads=1` and `cargo insta test -p htui --all-features`.
The latter is a full insta run, which must show **no** snapshot change.

---

## D. Commits per task

The commit trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Implementers commit incrementally.
A tests-first commit lands only when it compiles, and otherwise the red tests go in with the smallest stub.

- **T0**: `test(mod-12): pin 0016's tables, index, checks, cascade and the sixteen-migration counts`,
  then `feat(mod-12): migration 0016_auto_queue - queue_entry, queue_batch, run.batch_id`.
- **T1**: `test(mod-12): ready_items is in queue order on both stores (ANA-2 criterion 22)`, then
  `feat(mod-12): ready_items orders by priority DESC, created_at, id; .sqlx regenerated`.
- **T2**: `test(mod-12): auto-mode gate cases and effective_gate pins (criteria 23, 24)`, then
  `feat(mod-12): auto snapshots downgrade non-hard gates to never (D10)`.
- **T3**:
  - `feat(mod-12): BatchId, QueueEntry, QueueBatch and the admission rules`
  - `feat(mod-12): NewRun.batch_id; create_run records and guards the batch (D7)`
  - `feat(mod-12): queue store surface on MemStore, PgStore and Backend; .sqlx regenerated`
  - `feat(mod-12): WorkerHost gains the runner's seven queue methods`
  - `test(mod-12): queue surface parity and the batch round trip, Pg and Mem`
- **T4**:
  - `feat(mod-12): Engine::enqueue_in_batch`
  - `test(mod-12): the queue runner over MemStore and Postgres`
  - `feat(mod-12): sweep_once admits queued ready items, prunes and drains (D3, D5, D6)`
  - `feat(mod-12): a rested walk wakes the sweep when the queue runs (D8)`
- **T5**:
  - `feat(mod-12): queue StoreRequests and their serve arm; a resume sweeps at once (D8, D9)`
  - `feat(mod-12): Backlog Q and P with status lines and help rows (D9)`
  - `test(mod-12): Backlog queue keys, offline refusal and the resume wake`
- **T6**: `docs(mod-12): M1 phase note - unattended runs landed; PRD milestone 1 complete`.

---

## E. Hazards (checked against the tree)

| # | Hazard | Evidence | Handling |
|---|---|---|---|
| H-1 | Two count pins the plan did not list will go red at T0 | `migrations.rs:119-133` (`TABLES.len() == 42`, `present.len() == TABLES.len()+1`) and `:663-711` (exactly 46 commented columns over a table set that includes `run`) | §C.0 updates both. 44 tables and 58 comments |
| H-2 | The `htui-orch` `CASES` pin exists twice | `conformance.rs:7745-7788` and `crates/htui-orch/tests/fake_conformance.rs:15-25` | T2 bumps both to 103 |
| H-3 | Topology includes `gate_effective`, so an auto snapshot hashes differently from a manual one | `graph.rs:274-284` serialises the whole `SnapshotPhase`; `resume_window` re-resolves with `row.mode` (`engine.rs:3036-3044`) | They are consistent: a new auto run resumes clean. The demo `RUN_2` is `Auto` with a pre-MOD-12 snapshot (`fixtures.rs:1458-1476`). It would read as a topology mismatch on resume, but every test cancels or never resumes it (`engine.rs:11477` refuses it as `queued` before resolving), and `--demo` never claims it |
| H-4 | `Scope` documents "always one workspace" | `model/scope.rs:8-15` | The runner's scope is `Scope { workspace_id: WorkspaceId::default(), project_ids }`. Neither `ready_items` reads `workspace_id`: Pg binds only `project_uuids(scope)` (`read.rs:2074`) and Mem reads only `project_ids` (`mem.rs:1066-1083`). After T1 the order no longer reads project position. The `admit` doc says so; the plan's D5 already implies it |
| H-5 | `active_runs_on_box` is the wrong slot count for D6 | It counts `running` and `awaiting_approval` (`read.rs:2170-2190`, `mem.rs:798-817`), while `claim_run` counts `running` only (`write.rs:4391`, `mem.rs:4609-4618`) | The new `running_runs_on_box`. With the old read, two hard-gate parks would stall the queue |
| H-6 | A pause racing an admission | D2. Without a guard, a run created after the close would join the closed batch | `create_run` takes `FOR SHARE` on the batch row and refuses a closed one. `admit` re-reads the batch on `Constraint` and stops |
| H-7 | A wake loop through refused claims | A claim refused for `SlotFull` or `Overlaps` ends its task with the run `queued` (`runtime.rs:1856-1915`) | `wake_on_rest` wakes only on a run read at rest, and only with an open batch. The existing runtime tests (no batch) see no extra sweep, only one or two extra reads in the task tail |
| H-8 | A sweep asked for while one runs used to be dropped | `sweep_with` returns on `compare_exchange` failure (`runtime.rs:1215-1223`) | `sweep_again`. A wake landing between the loop's final `swap` and `Swept`'s drop was lost until the next tick; review L2 closes it: `Swept`'s drop frees the claim, then swaps `sweep_again` and calls `spawn_sweep` again (its CAS and close check keep one sweep at a time). That alone moved the gap to the losing caller (review G2): a wake whose CAS failed before the release but whose `sweep_again` store landed after the drop's swap was still lost; the loser now tries the CAS once more after publishing the flag, so the holder's later swap sees it or the loser sweeps itself. Documented, not tested (an interleaving across two threads) |
| H-9 | Debug-build stack headroom | Repo memory on stack headroom. `run_request` was 512 KiB before MOD-10's boxing | `enqueue`'s and `enqueue_in_batch`'s bodies are `Box::pin`ned, `sweep_once` awaits `Box::pin(admit(..))`, and the new cases sit in their own boxed frame. Gate the workspace with `--no-fail-fast` plus a `SIGABRT` grep, including `htui` `runs_pg`/`worker_pg`/`chat` |
| H-10 | `.sqlx` drift | `ready_items` (`query-a22310d8…`) and `create_run` (`query-eb3ab1ed…`) texts change, and eleven new query texts appear | Regenerate inside `crates/htui-store` with `-- --all-features --all-targets` against a migrated scratch DB, then `--check`. `running_runs_on_box` reuses `claim_run`'s literal and adds no file |
| H-11 | Admission spam | An enqueue that refuses without moving the item (e.g. `NoTemplate`) is retried at every sweep, every 5 s on a worker | Logged at `debug`. It takes no slot and never stops the next entry. M3's overlay lists it |
| H-12 | The `0016` number can collide on merge | Sibling hr runs. `/host/htui` ends at `0015` today | Re-check before T0 commits and again at collect. A renumber touches §C.0's literals and the HANDOFF line |
| H-13 | `queue_entry` and `queue_batch` FKs on `box`/`app_user` without cascade | `testkit::demo_db` deletes the seeded box and user (`testkit.rs:177-186`), and `migrations.rs:1892-1899` clears `box`/`app_user` | Both run before any queue row exists, so nothing blocks. No production path deletes a box or a user |
| H-14 | The `ready_here` view must keep display order | `read_items` composes `items()` (`store_worker.rs:2030-2047`) | It does not call `ready_items`, so the view is untouched. Only the two equality tests are reworded (§C.1) |
| H-15 | Mem's `State` literal is exhaustive | `from_demo` lists every field (`mem.rs:328-395`) | Add the three fields there, or the build fails |

---

## F. Deviations from the plan

1. **`NewRun` site count.** The plan's grounding says "27 sites in 13 files". The tree has **16** literal
   sites in **12** files that need `batch_id: None` (§B.3.6). Seven more use struct-update syntax
   (`..new_run(..)`, `..graph_run(..)`, `..race_run(..)`) and inherit the field, and
   `model/run.rs` constructs none. No decision changes.
2. **`WorkerHost` is narrowed to the runner's calls.** T3 lists every queue method on `WorkerHost`. The
   tree's rule is that each trait is "exactly its call sites" (`store/worker.rs:3-8`). `queue_item`,
   `dequeue_item` and `open_batch` are called only by the TUI's `serve_queue` through `Backend`, so they
   stay inherent. `WorkerHost` gains the seven methods the runtime calls (§B.3.5), and T4's tests drive
   `MemStore`/`PgStore` directly.
3. **`batch_has_live_runs` becomes `batch_runs`, returning `Vec<(RunId, RunStatus)>`.** `Run` carries no
   `batch_id` (D7), so this is the only read that can show batch membership. One read serves the D3 drain
   check, D9's "N runs still running", and T3's `create_run` round trip on both stores.
4. **A new read, `running_runs_on_box`.** D6's "running" has to be `claim_run`'s slot count. The
   existing `active_runs_on_box` also counts `awaiting_approval` (H-5).
5. **`QueueEntry` carries `project_id`.** This is a join, not a column. D5 builds `ready_items`' scope from
   "the entries' distinct projects", and the row alone does not have them.
6. **D11's Memory refusal does not hold in the tree.** `Backend::Memory` serves orchestration reads
   (`backend.rs:540-546`), and `--demo` starts runs. T4's tests run on `Backend::Memory` (plan T4), and
   T5's run on `Harness::demo()` (Memory), so Memory **serves** the queue requests. Only `Offline`
   refuses, with `DATABASE_UNREACHABLE`. In `htui --demo`, `Q`/`P` write rows but the demo runtime
   never admits, because it is built `without_claim_scan()` (`run_worker.rs:49-56`). That is the same
   reason the seeded `RUN_2` is never claimed there.
7. **T4 test (d) is restated.** "A missing-tags item is blocked with a note and the next item admitted"
   cannot happen through D5, because `ready_items` already excludes an item whose tags the box lacks
   (`read.rs:2094-2097`, `mem.rs:767-773`). Such an item waits in the queue, `open` and with no note
   (PRD D1). (d) asserts that instead. The enqueue-refusal path (log it, try the next entry) is covered
   by (d2), which uses a resolve refusal.
8. **D8's resume half moves to T5.** The wake on resume needs the TUI loop (`store_worker.rs:2718`), which is a
   T5 file. Putting it in T4 would break the plan's T4 ∩ T5 = ∅. T4 keeps the walk-end half inside
   the runtime (`wake_on_rest`), which also serves the headless worker (its 5 s poll is unchanged), and
   adds `sweep_again` so a wake during a sweep is not lost.
9. **T5 adds a read request, `QueueState`, and two replies, `Queue` and `QueueWritten`.** The plan
   leaves open how `Q`/`P` learn the current state. The tab reads it before every toggle rather than
   caching it, so a batch drained by a headless worker never leaves a stale `P`. D9's sentences are
   extended for dequeue and the "already" cases (§B.5.2).
10. **Two file lists are wider than the plan's.** T1 also edits `crates/htui/src/store_worker.rs` (the two
    `ready_here` tests, §C.1); the plan's Validate line already runs them. T2 also edits
    `crates/htui-orch/tests/fake_conformance.rs` (H-2). T0's `migrations.rs` changes extend past the
    count literals (H-1).
11. **`create_run` refuses a closed or unknown batch.** This extends D7 so that a pause racing an admission
    obeys D2 (H-6). The migration also adds `idx_run_batch`, which D1 does not mention (§B.0).
12. **A failing queue store call is warned once per streak (review M2).** §B.4 steps 1-2 log a failed
    read at `debug` and return. A read failing on every sweep then stops admission while the batch shows
    open, with nothing at the worker's default `info` filter. `admit`'s reads and prune and `drain`'s close
    now go through `Shared::queue_read_failed`: `warn` on the first failure of a streak, `debug` after it,
    and one `info` line when a sweep gets past them again. Per-entry refusals stay at `debug` (H-11).
