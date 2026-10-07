# Blueprint: MOD-12 milestone 2: Spend guard

**Plan**: `.claude/plans/mod-12-m2-spend-guard.plan.md`. Its D1 to D11, T1 to T6, waves and "Verified
claims" table are binding (maintainer CONFIRM, 2026-10-07). Every file:line below was re-read on
`hr/MOD-12` at `961e4f40`.
**PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md` (M2 row, success metric "batch spend
overshoot").
**M1 seams this builds on**: `.claude/plans/mod-12-m1-unattended-runs.blueprint.md` §B.3 (queue
store surface), §B.4 (`admit`, `Engine::enqueue_in_batch`).
**When the tree and the plan disagree, the tree wins.** Each case is listed under §F **Plan
deviations** with its evidence. Where the plan leaves a detail open, this blueprint settles it and
says so. A **VERIFY** marker means the implementer must check the point before relying on it.

Conventions inherited unchanged (M1 blueprint header, re-checked):
- MSRV 1.98, edition 2024, workspace lints at `-D warnings`. `#![warn(missing_docs)]` in
  `htui-core`, `htui-store`, `htui-worker`: every new `pub` item carries a doc comment.
- Store traits are bound by path, never `use`d where `ReadStore`/`WriteStore` are visible
  (`crates/htui-core/src/store/worker.rs:5-8`). `WorkerHost`/`WorkerStore` methods are declared
  `fn … -> impl Future<Output = Result<T>> + Send`; implementors write `async fn`.
- Unmirrored reads are inherent on `MemStore`/`PgStore`, dispatched by `Backend`; `Backend::Offline`
  answers `orchestration_offline()` (`crates/htui-store/src/backend.rs:732-738` is the shape).
- `.sqlx` is regenerated inside `crates/htui-store` against a **migrated scratch DB** with
  `cargo sqlx prepare -- --all-targets --all-features` (`docs/hr-sandbox.md:196-210`). A literal
  that is byte-identical to an existing one (indentation included) reuses its entry
  (`pg/write.rs:1955-1957` says so for `set_box_probe_spec`).
- Every gate runs `--no-fail-fast`; grep the log for `SIGABRT|overflowed|test result: FAILED`.
  `htui` integration tests need `--all-features` and `--test-threads=1`.

---

## A. Per-file change table

| # | File | Task | What changes (and what must **not**) |
|---|---|---|---|
| 1 | `crates/htui-core/src/model/queue.rs` | T1 | `BatchStop`, `batch_budget`, `MIN_BUDGET_FOR_NEW_ATTEMPT`, `min_budget_micros` + unit tests (§B.1.1). `admission_*`/`free_slots` untouched |
| 2 | `crates/htui-core/src/model/mod.rs` | T1, T4 | T1: extend `pub use queue::{…}` (`:149`). T4: `pub mod queue_settings;` after `pub mod queue;` (`:96`) and its `pub use` |
| 3 | `crates/htui-core/src/store/mem.rs` | T1, T4 | T1: `MemStore::batch_spend`, `MemStore::run_batch_spend`, `State::batch_spend`, unit tests by the M1 block (`:14126`). T4: `queue_setting`/`set_queue_setting`/`clear_queue_setting` in `impl WriteStore for MemStore` (`:7856`) + `State` helpers |
| 4 | `crates/htui-core/src/store/worker.rs` | T1 | `WorkerHost::{batch_spend, project_settings}` after `close_drained_batch` (`:570-575`); `WorkerStore::run_batch_spend` after `cancel_command` (`:462`); `impl WorkerStore for MemStore` arm (`:659-946`); module-doc counts (`:15-20`) |
| 5 | `crates/htui-store/src/pg/read.rs` | T1 | `PgStore::batch_spend`, `PgStore::run_batch_spend` after `batch_cancelled_items` (`:2305-2325`). **No** project-settings read: it exists (`:1550-1558`, §F-1) |
| 6 | `crates/htui-store/src/backend.rs` | T1 | `Backend::batch_spend` after `batch_cancelled_items` (`:732-738`) |
| 7 | `crates/htui-store/src/worker.rs` | T1 | `WorkerStore::run_batch_spend` for `PgStore` (`:116-407`) and `Writer` (`:491-782`); `WorkerHost::{batch_spend, project_settings}` for `PgStore` (`:784-888`) and `Backend` (`:890-994`) |
| 8 | `crates/htui-worker/src/runtime.rs` | T1, T3 | T1: `Failing` (`:3693-3843`) gains the two `WorkerHost` methods. T3: `admit` (`:2115-2240`), two `Shared` fields (`:186-260`) + `assemble` (`:1185-1215`), the streak test list (`:3895-3904`) |
| 9 | `crates/htui-store/tests/pg_criteria.rs` | T1 | Two parity tests beside `queue_surface_answers_alike_on_both_stores` (`:7525`) |
| 10 | `crates/htui-store/.sqlx/` | T1, T4 | T1: +2 files (§B.1.3). T4: +2 files (box set/clear); every other T4 literal is reused byte for byte |
| 11 | `crates/htui-orch/src/select.rs` | T2 | Two `SelectInput` fields, two `SkipCause` variants, one rule, docs, tests (§B.2.1) |
| 12 | `crates/htui-orch/src/engine.rs` | T2 | `walk_candidates` batch figures (`:3373-3409`); D6 `Allowance` + `session_allowance` + `open_recorder` (`:6146-6181`); `session` (`:6093`), `candidate_live` (`:4391`), judge (`:5265-5279`) callers; spec `budget_micros` (`:6320`); two `SettleInput` sites (`:3743`, `:4449`); `enqueue_in_batch_fake` beside `claim_fake` (`:7314`); unit tests |
| 13 | `crates/htui-orch/src/gate.rs` | T2 | `StepFailure::CapBreached { batch }`, `SettleInput::cap_batch`, `settle`, tests (§B.2.3; §F-2) |
| 14 | `crates/htui-orch/src/recover.rs` | T2 | `cap_batch: None` in the one `SettleInput` literal (`:337-347`) (§F-2) |
| 15 | `crates/htui-orch/src/fake.rs` | T2 | `FakeOrchestrator::enqueue_in_batch` beside `claim` (`:2334`) (§F-2) |
| 16 | `crates/htui-orch/src/conformance.rs` | T2 | `Orchestrate::enqueue_in_batch` + impl; `spend_guard_case` frame; four `CASES`; pin 103 to 107 (`:7924-7961`) |
| 17 | `crates/htui-orch/tests/fake_conformance.rs` | T2 | `cases_len_is_pinned` 103 to 107 (`:13-23`) (§F-2) |
| 18 | `crates/htui-worker/tests/auto_queue.rs` | T3 | Costing transport, `mint_in`, project-cap helper, six tests (§C.3) |
| 19 | `crates/htui-core/src/model/queue_settings.rs` | T4 | CREATE: `QueueSetting`, validators, USD and window parse/format, tests (§B.4.1) |
| 20 | `crates/htui-core/src/store/traits.rs` | T4 | `QueueTarget`, `QueueToken`, `QueueStored`, two refusal fns, three `WriteStore` methods; `edit_box` doc amended (`:543-572`, "only writer" at `:553`) |
| 21 | `crates/htui-core/src/store/mod.rs` | T4 | Re-export the new trait-module names (`:16-35`) (§F-4) |
| 22 | `crates/htui-core/src/store/conformance.rs` | T4 | Five cases; `CASES` 159 to 164 |
| 23 | `crates/htui-store/tests/pg_conformance.rs` | T4 | `EXPECTED_CASES` 159 to 164 (`:33-42`) (§F-4) |
| 24 | `crates/htui-store/src/pg/write.rs` | T4 | The three methods in `impl WriteStore for PgStore`, after `setting` (`:4134-4139`) |
| 25 | `crates/htui-store/src/writer.rs` | T4 | Three delegating methods after `setting` (`:869-874`) |
| 26 | `crates/htui-agent/src/conformance.rs` | T4 | `UsageSpy` delegates (after `setting`, `:1080`) |
| 27 | `crates/htui-agent/tests/recorder.rs` | T4 | `SpyStore` delegates (after `setting`, `:806`) |
| 28 | `crates/htui/src/queue_settings.rs` | T5 | CREATE: serve module (§B.5.2) |
| 29 | `crates/htui/src/lib.rs` | T5 | `pub mod queue_settings;` between `provision` (`:34`) and `qdrant_settings_info` (`:36`) |
| 30 | `crates/htui/src/store_worker.rs` | T5 | Three requests after `WriteDocument` (`:1096-1105`), two replies before `Failed` (`:1648`), `name()` arms (after `:1261`), `try_serve` or-arm by the secrets arm (`:2170-2174`), name test |
| 31 | `crates/htui/src/ui/tabs/settings/queue.rs` | T5 | CREATE: `QueueSection` (§B.5.3) |
| 32 | `crates/htui/src/ui/tabs/settings/mod.rs` | T5 | `pub mod queue;` + `pub use queue::QueueSection;` + module-doc mention (`:2-8`, `:18-21`, `:39-46`) |
| 33 | `crates/htui/src/app/mod.rs` | T5 | Import (`:15-18`) and register last, after `SecretsSection` (`:78-79`) |
| 34 | `crates/htui/tests/queue_settings.rs` + `tests/snapshots/queue_settings__*.snap` | T5 | CREATE (§C.5) |
| 35 | `crates/htui/tests/settings.rs` | T5 | Strip test: import, tenth section, `9` to `10`, doc (`:1081-1112`). No strip `.snap` changes (§F-6) |
| 36 | `docs/ANA-2.md`, PRD, `HANDOFF.md` | T6 | §B.6 |

Waves are the plan's: T1, then T2 ∥ T3 ∥ T4, then T5, then T6. The file-list widening in §F
keeps every wave-2 pair disjoint: T2's extra files are all `htui-orch` (T3 and T4 touch none), T4's
extras are `store/mod.rs` and `pg_conformance.rs` (T2 and T3 touch neither).

---

## B. Interfaces and SQL, exactly

### B.1 T1: batch figures and the admission rule

#### B.1.1 `crates/htui-core/src/model/queue.rs` (append before `#[cfg(test)]`, `:105`)

```rust
/// `app_setting.min_budget_for_new_attempt` (OQ-6): unseeded; USD micros.
pub const MIN_BUDGET_FOR_NEW_ATTEMPT: &str = "min_budget_for_new_attempt";

/// OQ-6's reading of [`MIN_BUDGET_FOR_NEW_ATTEMPT`]: a positive integer, else `0`. A stray `0`, a
/// negative, a string or a float all read as `0`, as `htui-orch`'s `min_budget` reads it
/// (`engine.rs:7143-7152`), so the runner (MOD-12 M2 D4) and the walk agree on one number.
#[must_use]
pub fn min_budget_micros(app: &BTreeMap<String, Value>) -> i64 {
    app.get(MIN_BUDGET_FOR_NEW_ATTEMPT)
        .and_then(Value::as_i64)
        .filter(|micros| *micros > 0)
        .unwrap_or(0)
}

/// Why [`batch_budget`] admits no new attempt in a batch (MOD-12 M2 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStop {
    /// The batch has spent its project's `per_token_cap_batch` or more (rule 2's batch twin).
    CapReached {
        /// The batch's spend, USD micros.
        spent: i64,
        /// The cap compared against, USD micros.
        cap: i64,
    },
    /// What is left is below `min_budget_for_new_attempt` (rule 5's batch twin).
    Budget {
        /// `cap - spent`, USD micros.
        remaining: i64,
        /// The minimum a new attempt needs, USD micros.
        min: i64,
    },
}

impl core::fmt::Display for BatchStop {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CapReached { spent, cap } => {
                write!(f, "batch cap reached ({spent} of {cap} micros)")
            }
            Self::Budget { remaining, min } => {
                write!(f, "batch budget: {remaining} micros left, {min} required")
            }
        }
    }
}

/// MOD-12 M2 D3: the one admission rule the runner, the walk and (through its figures) the
/// recorder share, mirroring `select::walk`'s rule 2 and rule 5 exactly. Either figure unknown is
/// unbounded (`Ok(None)`, OQ-6). `spent >= cap` is [`BatchStop::CapReached`] (equality reaches it);
/// `cap - spent < min` is [`BatchStop::Budget`] (exactly the minimum is enough); otherwise
/// `Ok(Some(cap - spent))`.
///
/// # Errors
/// The [`BatchStop`] that refuses the attempt.
pub fn batch_budget(spent: Option<i64>, cap: Option<i64>, min: i64) -> Result<Option<i64>, BatchStop> {
    let (Some(spent), Some(cap)) = (spent, cap) else {
        return Ok(None);
    };
    if spent >= cap {
        return Err(BatchStop::CapReached { spent, cap });
    }
    let remaining = cap.saturating_sub(spent);
    if remaining < min {
        return Err(BatchStop::Budget { remaining, min });
    }
    Ok(Some(remaining))
}
```

`model/mod.rs:149` becomes
`pub use queue::{BatchClose, BatchStop, MIN_BUDGET_FOR_NEW_ATTEMPT, QueueBatch, QueueEntry, admission_limit, admission_order, batch_budget, free_slots, min_budget_micros};`
(rustfmt will wrap it).

#### B.1.2 Mem: `crates/htui-core/src/store/mem.rs`

Inherent, in the `impl MemStore` block after `batch_cancelled_items` (`:1148-1161`):

```rust
/// MOD-12 M2 D1: `Σ run_step.usage["cost_micros"]` over the steps of every run admitted under
/// `batch`; `None` when no step reports an integer cost. Computed, never stored (ANA-2 §4.10).
/// # Errors
/// Never; the signature matches `PgStore`'s so `Backend` can dispatch over both.
pub async fn batch_spend(&self, batch: BatchId) -> Result<Option<i64>> {
    Ok(self.read(|state| state.batch_spend(batch)))
}

/// MOD-12 M2 D5: the batch `run` was admitted under, with [`MemStore::batch_spend`] of it; `None`
/// for a manual or chat run, and for an unknown run.
/// # Errors
/// Never; the signature matches `PgStore`'s.
pub async fn run_batch_spend(&self, run: RunId) -> Result<Option<(BatchId, Option<i64>)>> {
    Ok(self.read(|state| {
        state.run_batches.get(&run).map(|batch| (*batch, state.batch_spend(*batch)))
    }))
}
```

`State` gains (beside the other `State` read helpers):

```rust
/// [`MemStore::batch_spend`]'s body: `as_i64` skips a non-integer cost, as `select::run_spend`
/// does, and Pg's text guard does (§B.1.3).
fn batch_spend(&self, batch: BatchId) -> Option<i64> {
    let runs: HashSet<RunId> = self
        .run_batches
        .iter()
        .filter(|(_, of)| **of == batch)
        .map(|(run, _)| *run)
        .collect();
    self.steps
        .values()
        .filter(|step| runs.contains(&step.run_id))
        .filter_map(|step| step.usage.as_ref()?.get("cost_micros")?.as_i64())
        .fold(None, |total, cost| Some(total.unwrap_or(0).saturating_add(cost)))
}
```

`run_batches` is the M1 field (`mem.rs:305`); `steps: HashMap<StepId, RunStep>` (`:268`).

#### B.1.3 Pg: `crates/htui-store/src/pg/read.rs` (after `batch_cancelled_items`, `:2305-2325`)

```rust
/// MOD-12 M2 D1: `Σ (run_step.usage->>'cost_micros')::bigint` over the runs admitted under
/// `batch`; `None` when no step reports an integer cost. The text guard keeps a non-integer cost
/// out of the sum (Mem's `as_i64` skips it) instead of raising `22P02` on the cast.
/// # Errors
/// Whatever the driver reports, through [`map_sqlx`].
pub async fn batch_spend(&self, batch: BatchId) -> Result<Option<i64>>
```
```sql
SELECT SUM((s.usage->>'cost_micros')::bigint)::bigint AS "spent"
  FROM run_step s
  JOIN run r ON r.id = s.run_id
 WHERE r.batch_id = $1
   AND (s.usage->>'cost_micros') ~ '^-?[0-9]+$'
```
`sqlx::query_scalar!(…, batch.as_uuid()).fetch_one(&self.pool)` gives `Option<i64>`. The outer
`::bigint` is required: `SUM(bigint)` is `numeric`, and the workspace `sqlx` has no decimal feature
(`Cargo.toml:74-76`). `idx_run_batch` (`0016_auto_queue.sql:41`) and the `UNIQUE (run_id, …)` index
of `run_step` (`0001_init.sql:494`) serve the join.

```rust
/// MOD-12 M2 D5: `run`'s `batch_id` and [`PgStore::batch_spend`] of it; `None` for a manual or
/// chat run (`batch_id IS NULL`) and for an unknown run. Two statements: the second is
/// `batch_spend`'s, so the two answers cannot drift.
/// # Errors
/// Whatever the driver reports, through [`map_sqlx`].
pub async fn run_batch_spend(&self, run: RunId) -> Result<Option<(BatchId, Option<i64>)>>
```
```sql
SELECT batch_id AS "batch_id: BatchId" FROM run WHERE id = $1
```
`query_scalar!` with `fetch_optional` gives `Option<Option<BatchId>>`; `.flatten()`, then
`batch_spend(b)`. Two new `.sqlx` files.

**Backend** (`backend.rs`, after `batch_cancelled_items` `:732-738`), same doc shape:
```rust
pub async fn batch_spend(&self, batch: BatchId) -> Result<Option<i64>> {
    match self {
        Self::Memory(store) => store.batch_spend(batch).await,
        Self::Online { pg, .. } => pg.batch_spend(batch).await,
        Self::Offline { .. } => Err(orchestration_offline()),
    }
}
```
`run_batch_spend` gets **no** `Backend` method: it is a `WorkerStore` read and `Backend` is no
`WorkerStore` (its writer is `Writer`).

#### B.1.4 Traits: `crates/htui-core/src/store/worker.rs`

`WorkerStore` (after `cancel_command`, `:462`):
```rust
    /// MOD-12 M2 D5: the batch `run` was admitted under and that batch's spend
    /// (`MemStore::run_batch_spend`); `None` for a manual or chat run. A `WorkerStore` read because
    /// [`Run`] carries no `batch_id` and the mirror is untouched (plan D5).
    fn run_batch_spend(
        &self,
        run: RunId,
    ) -> impl Future<Output = Result<Option<(BatchId, Option<i64>)>>> + Send;
```
Implementors (exactly three, re-checked with a text search for `WorkerStore for `):
- `MemStore` (`store/worker.rs:659`): `async fn run_batch_spend(&self, run: RunId) -> Result<…> { MemStore::run_batch_spend(self, run).await }`
- `PgStore` (`htui-store/src/worker.rs:116`): `PgStore::run_batch_spend(self, run).await`
- `Writer` (`:491`): `match self { Self::Memory(s) => s.run_batch_spend(run).await, Self::Online(pg) => pg.run_batch_spend(run).await }`

`UsageSpy` and `SpyStore` implement `WriteStore`/`RecorderStore` only, not `WorkerStore`.

`WorkerHost` (after `close_drained_batch`, `:570-574`):
```rust
    // -- MOD-12 M2 (plan D4): the runner's spend gate
    /// `Backend::batch_spend` (plan D1).
    fn batch_spend(&self, batch: BatchId) -> impl Future<Output = Result<Option<i64>>> + Send;
    /// `Backend::project_settings`: the project's `settings` blob, read live at admission
    /// (plan D2); `None` for an unknown project.
    fn project_settings(
        &self,
        project: ProjectId,
    ) -> impl Future<Output = Result<Option<Value>>> + Send;
```
Implementors: `PgStore` (`htui-store/src/worker.rs:784`) forwards `PgStore::batch_spend` /
`PgStore::project_settings`; `Backend` (`:890`) forwards `Backend::batch_spend` /
`Backend::project_settings` (`backend.rs:319-325`, exists); `Failing` (`runtime.rs:3725`):
```rust
        async fn batch_spend(&self, batch: BatchId) -> Result<Option<i64>> {
            self.check("batch_spend")?;
            WorkerHost::batch_spend(&self.inner, batch).await
        }
        async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
            self.check("project_settings")?;
            WorkerHost::project_settings(&self.inner, project).await
        }
```
Module doc (`worker.rs:15-20`): "`WorkerStore` is 59 methods: … and MOD-12 M2's `run_batch_spend`";
"… the 29th `close_drained_batch`; MOD-12 M2 adds two (`batch_spend`, `project_settings`), 31."

### B.2 T2: engine walk and session allowance

#### B.2.1 `crates/htui-orch/src/select.rs`

`SelectInput` (`:19-37`) gains, after `min_budget_micros`:
```rust
    /// MOD-12 M2 D5: the spend of the batch the run was admitted under (`run_batch_spend`), USD
    /// micros; `None` when unknown, and for a run in no batch.
    pub batch_spent_micros: Option<i64>,
    /// MOD-12 M2 D2, D5: `snapshot.settings.per_token_cap_batch` when the run is in a batch, else
    /// `None` (a manual run has no batch figure); `None` is unbounded.
    pub batch_cap_micros: Option<i64>,
```
`SkipCause` (`:41-60`) gains, after `Budget`:
```rust
    /// Rule 6 (MOD-12 M2 D3, D5): the batch has spent its cap ([`BatchStop::CapReached`]).
    BatchCapReached {
        /// The batch's spend, USD micros.
        spent: i64,
        /// `per_token_cap_batch`, USD micros.
        cap: i64,
    },
    /// Rule 7 (MOD-12 M2 D3, D5): the batch's remainder is below the minimum
    /// ([`BatchStop::Budget`]).
    BatchBudget {
        /// `cap - spent`, USD micros.
        remaining: i64,
        /// The minimum a new attempt needs, USD micros.
        min: i64,
    },
```
`Display` arms delegate so the sentence is defined once:
`Self::BatchCapReached { spent, cap } => BatchStop::CapReached { spent: *spent, cap: *cap }.fmt(f)`,
`Self::BatchBudget { remaining, min } => BatchStop::Budget { remaining: *remaining, min: *min }.fmt(f)`.
Import `htui_core::model::queue::{BatchStop, batch_budget}`.

`skip_cause` (`:160-194`): after rule 5's block, before `None`:
```rust
    // Rules 6 and 7 (MOD-12 M2 D3): the batch twins of rules 2 and 5, after the run's own so a
    // run-level cause keeps its sentence. Unknown is unbounded (OQ-6).
    if let Err(stop) = batch_budget(
        input.batch_spent_micros,
        input.batch_cap_micros,
        input.min_budget_micros,
    ) {
        return Some(match stop {
            BatchStop::CapReached { spent, cap } => SkipCause::BatchCapReached { spent, cap },
            BatchStop::Budget { remaining, min } => SkipCause::BatchBudget { remaining, min },
        });
    }
```
Docs: "five rules" becomes "seven" in the `SkipCause` doc (`:39`), the `skip_cause` doc (`:159`) and
the `walk` list (`:131-144`), adding
`6. batch spend and cap both known with spent >= cap → [`SkipCause::BatchCapReached`];`
`7. batch spend and cap both known with cap - spent < min_budget → [`SkipCause::BatchBudget`].`
The test helper `input` (`:329-344`) gains `batch_spent_micros: None, batch_cap_micros: None`; every
other test literal is a struct update (`..input(…)`/`..base`) and needs nothing.

#### B.2.2 `crates/htui-orch/src/engine.rs`

**`walk_candidates`** (`:3373-3409`): before `Ok(select::walk(…))`:
```rust
        // MOD-12 M2 D5: a batch run is walked against its batch too; a manual or chat run is not.
        let batch = self.parts.store.run_batch_spend(run.id).await?;
```
and in the literal:
```rust
            batch_spent_micros: batch.and_then(|(_, spent)| spent),
            batch_cap_micros: batch.and(snapshot.settings.per_token_cap_batch),
```
`min_budget_micros: min_budget(&self.parts.app)` stays. **Recommended**: `min_budget` (`:7147-7152`)
becomes `htui_core::model::queue::min_budget_micros(app)` and `MIN_BUDGET_KEY` (`:126`) is dropped, so
the runner and the walk read one function. The `stage_one` doc (`:3349-3359`) gains "and, for a run
admitted under a batch, the batch's spend against the snapshot's `per_token_cap_batch` (MOD-12 M2 D5)".
A walk with nothing eligible already takes `refuse_no_candidate` (`:3418-3463`): the item goes
`blocked` and the note is `no_candidate_agent: phase `<p>`; <agent> (batch cap reached (… of … micros))`
(the `Walk::summary` of the skip causes). No new code there.

**D6 allowance.** New items above `impl Engine` (or beside `min_budget`):
```rust
/// MOD-12 M2 D6: what one session may spend, and the batch when the batch's remainder was the
/// binding term (so a breach can name it, D6's last bullet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Allowance {
    /// The recorder's cap and the driver's `budget_micros`, USD micros, never below 0.
    micros: i64,
    /// `Some` only when the batch term is strictly smaller than the run term (or the only one).
    batch: Option<BatchId>,
}

/// MOD-12 M2 D6: `min(run_cap - run_spent, batch_cap - batch_spent)` over the caps that are known,
/// floored at 0; an unknown spend against a known cap reads as 0 (today's recorder starts every
/// session at zero, so a run with no costed step keeps its whole cap). No cap known: `None`, no cap
/// is set (unchanged). A tie names the run, so the shipped `cap breached` text is kept.
/// `batch` is `(id, per_token_cap_batch, batch_spend)`.
fn session_allowance(
    run_cap: Option<i64>,
    run_spent: Option<i64>,
    batch: Option<(BatchId, Option<i64>, Option<i64>)>,
) -> Option<Allowance>
```
Body: `run = run_cap.map(|cap| cap.saturating_sub(run_spent.unwrap_or(0)).max(0))`;
`batch_term = batch.and_then(|(id, cap, spent)| cap.map(|cap| (id, cap.saturating_sub(spent.unwrap_or(0)).max(0))))`;
then `match (run, batch_term) { (None, None) => None, (Some(r), None) => Some(Allowance { micros: r, batch: None }), (None, Some((id, b))) => Some(Allowance { micros: b, batch: Some(id) }), (Some(r), Some((id, b))) => Some(if b < r { Allowance { micros: b, batch: Some(id) } } else { Allowance { micros: r, batch: None } }) }`.

```rust
    /// MOD-12 M2 D6: this session's [`Allowance`]: the run term from `settings` (the live project
    /// settings `open_recorder` already reads for `per_token_cap_run`, unchanged) less the run's
    /// own spend so far, and, for a batch run, the batch term from the snapshot (plan D2). The
    /// snapshot is decoded only for a batch run.
    async fn allowance(
        &self,
        run: &Run,
        settings: &ProjectSettings,
    ) -> Result<Option<Allowance>, EngineError> {
        let steps = self.parts.store.run_steps(run.id).await?;
        let batch = match self.parts.store.run_batch_spend(run.id).await? {
            Some((id, spent)) => Some((id, Self::snapshot_of(run)?.settings.per_token_cap_batch, spent)),
            None => None,
        };
        Ok(session_allowance(settings.per_token_cap_run, select::run_spend(&steps), batch))
    }
```

**`open_recorder`** (`:6146-6181`) signature becomes
```rust
    async fn open_recorder(
        &self,
        run: &Run,
        step: &RunStep,
        prompt: &AssembledPrompt,
    ) -> Result<(Recorder<'a, S>, Option<BatchId>), EngineError>
```
and `:6164-6171` becomes
```rust
        let allowance = self.allowance(run, &settings).await?;
        if let Some(allowance) = allowance {
            recorder = recorder.with_run_cap(RunCap {
                micros: allowance.micros,
                grace: std::time::Duration::ZERO,
            });
        }
```
returning `Ok((recorder, allowance.and_then(|a| a.batch)))`. Doc: "with the run cap applied" becomes
"with D6's allowance applied (MOD-12 M2), and the batch when its remainder bound it".

**`drive_once`** spec (`:6320`): `budget_micros: recorder.run_cap().map(|cap| cap.micros),`.
`Recorder::run_cap` exists (`htui-agent/src/record.rs:655-657`), so the recorder and the driver read
one number, as `SessionSpec::budget_micros`'s doc requires (`htui-agent/src/driver.rs:298-302`, D83).
`settings` stays read in `drive_once` for `keep_raw_events`.

**Callers**:
- `session` (`:6093-6137`): `let (mut recorder, cap_batch) = self.open_recorder(…).await?;`, returning
  `Result<(Driven, Option<CapBreach>, Option<BatchId>), EngineError>`: `Ok((result, summary.cap_breach, cap_batch))`.
  Its one caller (`:3692-3711`) destructures the third element, and `SettleInput` at `:3743` gains
  `cap_batch,`.
- `candidate_live` (`:4391`): `let (mut recorder, cap_batch) = …;` and `SettleInput` at `:4449` gains `cap_batch,`.
- judge (`:5265-5279`): `let (mut recorder, cap_batch) = …;` and the text becomes
  `format!("{} ({breach:?})", gate::StepFailure::CapBreached { batch: cap_batch })`.

**Conformance plumbing** (beside `claim_fake`, `:7314-7325`, `#[cfg(feature = "test-support")]`):
```rust
/// [`Engine::enqueue_in_batch`] over a `FakeOrchestrator`'s parts (MOD-12 M2 conformance).
/// # Errors
/// [`Engine::enqueue`]'s.
pub async fn enqueue_in_batch_fake(
    orch: &crate::fake::FakeOrchestrator,
    item: ItemId,
    batch: BatchId,
) -> Result<RunId, EngineError>
```
The body is `claim_fake`'s with `engine.enqueue_in_batch(item, batch).await`.

#### B.2.3 `crates/htui-orch/src/gate.rs` (§F-2)

- `StepFailure::CapBreached` (`:122`) becomes
  ```rust
      /// `R-AGT-7`'s cap was reached and the recorder cancelled the turn. `batch` names the queue
      /// batch when its remainder was the binding term of the session's allowance (MOD-12 M2 D6).
      CapBreached {
          /// The binding batch, or `None` for the run's own cap.
          batch: Option<BatchId>,
      },
  ```
- `Display` (`:139`): `Self::CapBreached { batch: None } => f.write_str("cap breached")`,
  `Self::CapBreached { batch: Some(batch) } => write!(f, "cap breached (batch {batch})")`.
  `run_failure_text` then carries the batch text into `run.failure` unchanged in mechanism.
- `SettleInput` (`:203-235`) gains, after `cap_breach`:
  ```rust
      /// MOD-12 M2 D6: the batch whose remainder bound the session's allowance; read only with
      /// `cap_breach`.
      pub cap_batch: Option<BatchId>,
  ```
- `settle` (`:255-257`): `if input.cap_breach.is_some() { return Settle::Failed(StepFailure::CapBreached { batch: input.cap_batch }); }`
- Literals: `ok_input` (`:1117-1131`) gains `cap_batch: None`; `recover.rs:337-347` gains
  `cap_batch: None`. Every other gate test literal is `..ok_input(…)`. Test assertions at `:1317`,
  `:1345`, `:1422`, `:1436`, `:1437` spell `StepFailure::CapBreached { batch: None }`.

#### B.2.4 `crates/htui-orch/src/fake.rs` and `conformance.rs`

`FakeOrchestrator` (after `claim`, `:2334-2336`):
```rust
    /// `Engine::enqueue_in_batch` over the same parts (MOD-12 M2): an `auto` run of `item` under
    /// `batch`, left `queued` for [`claim`](Self::claim).
    /// # Errors
    /// Every [`EngineError`] the enqueue raises.
    pub async fn enqueue_in_batch(&self, item: ItemId, batch: BatchId) -> std::result::Result<RunId, EngineError> {
        crate::engine::enqueue_in_batch_fake(self, item, batch).await
    }
```
`Orchestrate` (`conformance.rs:77-222`) gains, after `claim` (`:143`):
```rust
    /// `Engine::enqueue_in_batch` (MOD-12 M2): the only way a case makes a batch run, which
    /// `Command::StartRun` never does. Claim it with [`claim`](Orchestrate::claim).
    /// # Errors
    /// Whatever the engine refuses with.
    async fn enqueue_in_batch(&self, item: ItemId, batch: BatchId) -> Result<RunId, EngineError>;
```
and `impl Orchestrate for FakeOrchestrator` (`:224`) forwards. `FakeOrchestrator` is the trait's
only implementor (text search `Orchestrate for `).

New frame, chained in `case()` (`:687-695`) as `.or_else(|| spend_guard_case(name, harness))`
before `.unwrap_or_else(…)`:
```rust
/// MOD-12 M2's four (plan D5, D6), boxed for [`case`]'s reason.
fn spend_guard_case<'a, H: CaseHarness>(
    name: &str,
    harness: &'a H,
) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>>
```
`CASES` (`:442-666`) gains, after `"a_manual_snapshot_keeps_its_gates"`:
```rust
    // MOD-12 M2 (plan D5, D6; PRD metric "batch spend overshoot"): a batch at its cap starts no
    // attempt, a batch run's session is cut at the batch's remainder, a manual run reads no batch
    // figure, and the run cap spans steps.
    "a_batch_at_its_cap_starts_no_attempt",
    "a_batch_run_session_is_capped_at_the_batch_remainder",
    "a_manual_run_ignores_batch_figures",
    "the_run_cap_spans_steps",
```
The `CASES` doc block gains "**Four for MOD-12 M2** (plan D5, D6): …". The count pin
(`:7924-7961`) becomes 107 with "…, and MOD-12 M2's four (the batch walk refusal, the batch
remainder, the manual run, the run cap across steps)"; `fake_conformance.rs:13-23` likewise.

### B.3 T3: the runner gate (D4)

#### B.3.1 `Shared` (`runtime.rs:186-260`) and `assemble` (`:1185-1215`)

```rust
    /// MOD-12 M2 D4: the batch whose first spend stop was said at `info`; later stops of it are
    /// `debug` (M3's overlay surfaces them).
    batch_stop_noted: StdMutex<Option<BatchId>>,
    /// MOD-12 M2 D4: the batch under which a malformed project cap was warned; later ones of it
    /// are `debug`.
    bad_cap_noted: StdMutex<Option<BatchId>>,
```
`assemble` initialises both with `StdMutex::new(None)` (the literal is exhaustive). Methods beside
`queue_read_failed` (`:291-309`):
```rust
    /// MOD-12 M2 D4: `item` was not admitted under `batch` for `stop`. The first stop of a batch
    /// is `info` ("the queue's batch reached its cap"); every later one `debug`. True when this
    /// one was the first.
    fn note_batch_stop(&self, batch: BatchId, item: ItemId, stop: &BatchStop) -> bool

    /// MOD-12 M2 D4: `project`'s `settings` hold a malformed cap, so its entries fail closed.
    /// Warned once per batch, `debug` after. True when this one warned.
    fn note_bad_cap(&self, batch: BatchId, project: ProjectId, err: &CapError) -> bool
```
Both lock their mutex, compare with `Some(batch)`, store `Some(batch)` when new, and log
(`tracing::info!(%batch, %item, %stop, "the queue's batch reached its cap")`;
`tracing::warn!(%batch, %project, %err, "a project's queue cap is malformed; its entries are not admitted")`).

#### B.3.2 `admit` (`runtime.rs:2115-2240`)

Placement is the plan's: after `free_slots`, before the enqueue loop and before `Kit::read`, so a
fully stopped batch costs no `Kit`. The `slots` block (`:2184-2200`) returns `(free, app)`:
```rust
    let slots = async {
        let running = host.running_runs_on_box(box_id).await?;
        let queued = host.queued_runs_on_box(box_id).await?.len();
        let app = host.app_settings().await?;
        let free = free_slots(admission_limit(box_settings, &app), running, queued);
        StoreResult::Ok((free, app))
    };
    let (free, app) = match slots.await { Ok(pair) => pair, Err(err) => { /* unchanged */ } };
    if free == 0 {
        shared.queue_reads_ok();
        return;
    }
    // MOD-12 M2 D4: the batch's spend once per sweep, each entry's project caps live (D2), one
    // rule for runner, walk and recorder (D3). A stopped entry is skipped and the next tried: a
    // project without a cap may still admit. A malformed cap fails closed.
    let spent = match host.batch_spend(batch.id).await {
        Ok(spent) => spent,
        Err(err) => {
            shared.queue_read_failed("reading its batch's spend", &err);
            return;
        }
    };
    let min = min_budget_micros(&app);
    let project_of: HashMap<ItemId, ProjectId> =
        entries.iter().map(|entry| (entry.item_id, entry.project_id)).collect();
    let mut caps: HashMap<ProjectId, Result<ProjectCaps, CapError>> = HashMap::new();
    let mut admissible = Vec::with_capacity(order.len());
    for item in order {
        let Some(&project) = project_of.get(&item) else { continue };
        if !caps.contains_key(&project) {
            let settings = match host.project_settings(project).await {
                Ok(settings) => settings.unwrap_or(Value::Null),
                Err(err) => {
                    shared.queue_read_failed("reading a project's caps", &err);
                    return;
                }
            };
            caps.insert(project, ProjectCaps::from_settings(&settings));
        }
        match &caps[&project] {
            Err(err) => {
                shared.note_bad_cap(batch.id, project, err);
            }
            Ok(project_caps) => match batch_budget(spent, project_caps.batch_micros, min) {
                Ok(_) => admissible.push(item),
                Err(stop) => {
                    shared.note_batch_stop(batch.id, item, &stop);
                }
            },
        }
    }
    shared.queue_reads_ok();
    if admissible.is_empty() {
        return;
    }
```
The loop that follows iterates `admissible` instead of `order`. `ProjectCaps::from_settings`
(`quota.rs:322-345`) rejects either key, so a malformed `per_token_cap_run` also fails the entry
closed; that matches the walk, whose snapshot would decode the same blob to defaults
(`graph.rs:655-657`, D11's flagged `unwrap_or_default`). The batch stays **open** (D4); `drain` is
not called. Imports: `htui_core::model::{BatchStop, CapError, ProjectCaps, batch_budget, min_budget_micros}`
(`runtime.rs:18-22`). Doc of `admit` (`:2104-2114`) gains: "Before the `Kit`, the D3 spend gate
(MOD-12 M2 D4): …".

### B.4 T4: the queue-setting write surface

#### B.4.1 `crates/htui-core/src/model/queue_settings.rs` (CREATE)

The architect's call between a new file and a section of `queue.rs`: a new file, so T4's diff stays
off T1's `queue.rs` and the UI-facing parsers sit apart from the runner's arithmetic.

```rust
//! The keys `Settings > Queue` writes (MOD-12 M2 D8-D10): two `project.settings` caps, three
//! `app_setting` rows and one `box.settings` key, each with its own validator, plus the USD and
//! window text the editor types. Nothing here touches a store.

str_enum!(
    /// MOD-12 M2 D9: one queue key, by its stored name. Which targets take which key is
    /// [`QueueSetting::PROJECT_KEYS`] / [`APP_KEYS`](QueueSetting::APP_KEYS) /
    /// [`BOX_KEYS`](QueueSetting::BOX_KEYS).
    QueueSetting {
        /// `project.settings.per_token_cap_run`, USD micros.
        PerTokenCapRun => "per_token_cap_run",
        /// `project.settings.per_token_cap_batch`, USD micros.
        PerTokenCapBatch => "per_token_cap_batch",
        /// `app_setting.min_budget_for_new_attempt`, USD micros.
        MinBudgetForNewAttempt => "min_budget_for_new_attempt",
        /// `app_setting.max_concurrent_items` and `box.settings.max_concurrent_items`.
        MaxConcurrentItems => "max_concurrent_items",
        /// `app_setting.scheduler_window` (D10): stored, not enforced.
        SchedulerWindow => "scheduler_window",
    }
);

impl QueueSetting {
    /// The keys a project target takes, in row order.
    pub const PROJECT_KEYS: [Self; 2] = [Self::PerTokenCapRun, Self::PerTokenCapBatch];
    /// The keys the app target takes, in row order.
    pub const APP_KEYS: [Self; 3] = [Self::MinBudgetForNewAttempt, Self::MaxConcurrentItems, Self::SchedulerWindow];
    /// The keys a box target takes.
    pub const BOX_KEYS: [Self; 1] = [Self::MaxConcurrentItems];

    /// Whether the editor types and shows this key in USD (D8): the caps and the minimum.
    #[must_use]
    pub const fn is_money(self) -> bool

    /// D9's validator for a value about to be **set** (a clear needs none).
    /// # Errors
    /// The sentence the section and the store both show, verbatim.
    pub fn validate(self, value: &Value) -> Result<(), String>
}

/// D8: dollars as typed (`1.5`, `$1.50`, ` 0.000001 `) to USD micros.
/// # Errors
/// [`USD_NOT_A_NUMBER`], [`USD_TOO_PRECISE`] (more than six decimals) or [`USD_TOO_LARGE`].
pub fn parse_usd(text: &str) -> Result<i64, String>

/// D8: USD micros as shown: `$` and at least two decimals, the rest trimmed of trailing zeros
/// (`1_500_000` → `$1.50`, `1_234_567` → `$1.234567`, `0` → `$0.00`). `parse_usd` inverts it.
#[must_use]
pub fn format_usd(micros: i64) -> String

/// D10: `HH:MM-HH:MM` to `{"start":"HH:MM","end":"HH:MM"}`; `end < start` crosses midnight.
/// # Errors
/// [`WINDOW_SHAPE`], or [`WINDOW_EMPTY`] when start and end are the same minute.
pub fn parse_window(text: &str) -> Result<Value, String>

/// D10: a stored window as typed back, `None` for `null`, absent, or a shape it does not know.
#[must_use]
pub fn format_window(value: &Value) -> Option<String>
```

Validator rules and sentences (settled here):

| Key | Accepts | Refusal sentence |
|---|---|---|
| `PerTokenCapRun`, `PerTokenCapBatch` | integer ≥ 0 (`0` is a real cap) | `CapError { key: PER_TOKEN_CAP_RUN \| PER_TOKEN_CAP_BATCH, found: value.to_string() }.to_string()` (`quota.rs:298-312`), so `null` reads "…got null": a clear is `clear_queue_setting` |
| `MinBudgetForNewAttempt` | integer > 0 | `app_setting.min_budget_for_new_attempt must be a positive integer of USD micros, got {value}` |
| `MaxConcurrentItems` | integer in `1..=u32::MAX` (`BoxSettings` is `Option<u32>`, `box_.rs:330`) | `max_concurrent_items must be a whole number from 1 to 4294967295, got {value}` |
| `SchedulerWindow` | `null`, or an object with exactly `start`/`end`, each `HH:MM` (00-23, 00-59), different | `scheduler_window must be null or {"start":"HH:MM","end":"HH:MM"} with two different minutes, got {value}` |

Constants (each `pub const …: &str` with a doc):
`USD_NOT_A_NUMBER = "type dollars, like 1.50"`,
`USD_TOO_PRECISE = "at most six decimals: a micro-dollar is the smallest unit"`,
`USD_TOO_LARGE = "that is more dollars than htui can count"`,
`WINDOW_SHAPE = "type the window as HH:MM-HH:MM, like 22:00-06:00"`,
`WINDOW_EMPTY = "the window starts and ends on the same minute"`.
`parse_usd` refuses a sign, an exponent, and an empty string (the section never parses empty: empty
clears). `mod.rs`: `pub use queue_settings::{QueueSetting, format_usd, format_window, parse_usd, parse_window};`
plus the five constants.

#### B.4.2 `crates/htui-core/src/store/traits.rs` (beside `SettingRung`/`StoredSetting`, `:2995-3035`)

```rust
/// MOD-12 M2 D9: where a queue key is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTarget {
    /// One `app_setting` row, keyed by [`QueueSetting::as_str`].
    App,
    /// One key of a project's `settings`.
    Project(ProjectId),
    /// One key of a box's `settings` (this user's boxes only, `edit_box`'s reach).
    Box(BoxId),
}

/// MOD-12 M2 D9: the compare-and-set token a queue write presents and a read answers. Two kinds
/// because the box target's guard is `box.edit_version` (`edit_box`'s, D9) and the others' is
/// the row's `updated_at` (`set_setting`'s rungs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueToken {
    /// `app_setting.updated_at` or `project.updated_at`. `None` is "I expect no row": accepted on
    /// `App` set only, as `set_setting`'s `expected: None` (`:1107-1108`).
    Stamp(Option<DateTime<Utc>>),
    /// `box.edit_version`.
    EditVersion(i32),
}

/// MOD-12 M2 D9: one queue key as stored, with its token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueStored {
    /// The stored JSON; `None` when the target's row holds no value for the key.
    pub value: Option<Value>,
    /// The token the next write presents. After an `App` clear: `Stamp(None)`.
    pub token: QueueToken,
}

/// MOD-12 M2 D9: the sentence for `key` on a `target` that does not take it, or `None`.
#[must_use]
pub fn queue_target_refusal(key: QueueSetting, target: QueueTarget) -> Option<String>

/// MOD-12 M2 D9: the sentence for a token of the wrong kind for `target` (an `EditVersion` on
/// `App`/`Project`, a `Stamp` on `Box`, `Stamp(None)` on `Project`, or on any clear), or `None`.
#[must_use]
pub fn queue_token_refusal(key: QueueSetting, target: QueueTarget, token: QueueToken, clearing: bool) -> Option<String>
```
Sentences: `` `{key}` is not a {target_kind} setting `` (target kinds `app`, `project`, `box`) and
`` `{key}` on {target_kind} needs {its token}: {why} ``. `store/mod.rs:16-35` re-exports
`QueueStored, QueueTarget, QueueToken, queue_target_refusal, queue_token_refusal` (§F-4).

`WriteStore` gains, after `setting` (`:1148`), under a `// queue settings (MOD-12 M2 D8, D9)` header:
```rust
    /// One queue key on one target with its token (MOD-12 M2 D9; T5's read). `None` only when the
    /// target's row is absent (`App`: no `app_setting` row; `Project`: no such id; `Box`: no such
    /// id, or a box of another `app_user`); a present project or box without the key answers
    /// `Some` with `value: None`.
    /// # Errors
    /// `Constraint` with [`queue_target_refusal`]'s sentence.
    async fn queue_setting(&self, target: QueueTarget, key: QueueSetting) -> Result<Option<QueueStored>>;

    /// Writes one queue key (MOD-12 M2 D9). Precedence: [`queue_target_refusal`],
    /// [`queue_token_refusal`], [`QueueSetting::validate`] (before the compare-and-set, as
    /// `set_setting` validates first), then the row: `NotFound`, `Stale`, and last a non-object
    /// blob (`Constraint`). `App` upserts its row (`Stamp(None)` = "I expect no row"; `Stale` if one
    /// exists). `Project` merges the key into `project.settings` under CAS on `updated_at`,
    /// `set_setting`'s project rung exactly. `Box` merges it into `box.settings` under CAS on
    /// `edit_version`, bumping it as `edit_box` does, every other key (`executor`,
    /// `command_limits`, unknown ones) kept. A refusal writes nothing.
    /// # Errors
    /// As above: `Constraint` for the three refusals and for `BOX_SETTINGS_NOT_AN_OBJECT` or a
    /// project blob that is not an object; `NotFound` for an absent row (`app_setting` when a
    /// `Stamp(Some)` names a row that is gone, as `set_setting` answers).
    async fn set_queue_setting(&self, target: QueueTarget, key: QueueSetting, value: Value, expected: QueueToken) -> Result<CasOutcome<QueueStored>>;

    /// Removes one queue key (MOD-12 M2 D9): `DELETE` on `app_setting`, `settings - key` on the
    /// project (CAS on `updated_at`) or the box (CAS on `edit_version`, bumped). `Applied` carries
    /// `value: None`; on `App` its token is `Stamp(None)` (no row is left to carry one).
    /// Clearing an absent key of a present project or box applies.
    /// # Errors
    /// The refusals of [`set_queue_setting`](WriteStore::set_queue_setting) minus the validator;
    /// `NotFound` for an absent row.
    async fn clear_queue_setting(&self, target: QueueTarget, key: QueueSetting, expected: QueueToken) -> Result<CasOutcome<QueueStored>>;
```
The `edit_box` doc (`:552-557`) "It is `box.settings`' only writer" becomes "It and
[`set_queue_setting`](WriteStore::set_queue_setting)'s `Box` target (MOD-12 M2 D9) are
`box.settings`' only writers, both key by key under the same `edit_version` guard".

Implementors (five, re-checked with a text search for `WriteStore for `):
`MemStore` (`mem.rs:7856`), `PgStore` (`pg/write.rs:1008`), `Writer` (`writer.rs:329`, a
`match self { Memory, Online }` per method as `set_setting` at `:844-855`), `UsageSpy`
(`htui-agent/src/conformance.rs:751`) and `SpyStore` (`htui-agent/tests/recorder.rs:438`), both
`self.inner.<method>(…).await` as their `set_box_probe_spec` does (`:811-817`, `:518-524`).

#### B.4.3 Pg SQL (`pg/write.rs`, in `impl WriteStore for PgStore` after `setting`, `:4134-4139`)

**App target**: the three `App` statements of `set_setting`/`clear_setting` **byte for byte,
indentation included** (`:3934-3951` and `:4069-4072`), as `set_box_probe_spec` reuses them
(`:1952-1999`, with its "mis-indented on purpose" comment copied). Bind `$1 = key.as_str()`. A miss
re-reads with `stored_setting`'s `App` literal (`pg/read.rs:2985-2988`, byte for byte) and answers
`cas_miss`'s rule (`write.rs:97-108`): `Stale(QueueStored { value: Some(v), token: Stamp(Some(at)) })`
or `NotFound { entity: "app_setting", id: key }`.

**Project target**: `set_setting`'s project statements byte for byte (`:3976-3980` set, `:4087-4090`
clear) with `$2 = key.as_str()`; a `Stamp(None)` was refused earlier. A miss re-reads
`"SELECT settings, updated_at FROM project WHERE id = $1"` (`pg/read.rs:2998-3001`, byte for byte):
absent is `NotFound { entity: "project" }`, else `Stale` with `settings.get(key)` and `Stamp(Some(updated_at))`.

**Box target** (two new `.sqlx` files):
```sql
UPDATE box
   SET settings     = settings || jsonb_build_object($3::text, $4::jsonb),
       edit_version = edit_version + 1
 WHERE id = $1 AND user_id = $5 AND edit_version = $2
   AND jsonb_typeof(settings) = 'object'
RETURNING settings, edit_version
```
```sql
UPDATE box
   SET settings     = settings - $3::text,
       edit_version = edit_version + 1
 WHERE id = $1 AND user_id = $4 AND edit_version = $2
   AND jsonb_typeof(settings) = 'object'
RETURNING settings, edit_version
```
`$1 = id`, `$2 = expected`, `$3 = key.as_str()`, `$4 = &value` (set) or `me` (clear), `$5 = me`
(`self.this_user()`, `edit_box`'s reach, `:1844`). `Applied(QueueStored { value: row.settings.get(key).cloned(), token: EditVersion(row.edit_version) })`.
A miss mirrors `edit_box`'s (`:1903-1918`): `self.box_row(id).await?.filter(|row| row.user_id == me)`;
`None` is `NotFound { entity: "box" }`; `Some(row)` with `row.edit_version == expected` and a
non-object blob is `Constraint(BOX_SETTINGS_NOT_AN_OBJECT)`; any other `Some(row)` is
`Stale(QueueStored { value: row.settings.get(key).cloned(), token: EditVersion(row.edit_version) })`.

**`queue_setting`** reads: `App` with `stored_setting`'s literal; `Project` with its project literal;
`Box` through `self.box_row(id)` filtered by `this_user()`. No new `.sqlx`.

#### B.4.4 Mem (`mem.rs`)

`MemStore`'s three methods take `this_user()` and `now()` **before** the write lock (`edit_box`'s
rule, `:7944-7948`), then call `State::{queue_setting, set_queue_setting, clear_queue_setting}`.
`State` uses `app_settings: BTreeMap<String, (Value, DateTime<Utc>)>` (`:243`) with
`State::set_setting`'s `App` match (`:4085-4117`), `State::set_setting`'s `Project` arm
(`:4118-4147`, including `settings_not_an_object`), and `State::edit_box`'s lookup and guards
(`:2467-2517`) for `Box`: `boxes.get(&id).filter(|row| Some(row.user_id) == user)`, then
`edit_version`, then `is_object`, then `insert`/`remove` the key, `edit_version += 1`,
`updated_at = now`.

### B.5 T5: `Settings > Queue`

#### B.5.1 `crates/htui/src/store_worker.rs`

Requests, after `WriteDocument` (`:1096-1105`):
```rust
    /// MOD-12 M2 D8: the queue settings of this scope: the three `app_setting` keys with their
    /// tokens, each scope project's two caps, and this box's `max_concurrent_items` with the
    /// effective value. Answered with [`StoreReply::QueueSettings`]; offline `DATABASE_UNREACHABLE`.
    QueueSettings(Scope),
    /// MOD-12 M2 D9: `set_queue_setting`. Answered with [`StoreReply::QueueSettings`] when it
    /// applied, [`StoreReply::QueueSettingsStale`] when the token was spent.
    SetQueueSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Where the key is written.
        target: QueueTarget,
        /// Which key.
        key: QueueSetting,
        /// The validated JSON: micros for money, an integer, or a window object.
        value: Value,
        /// The token the editor opened on.
        expected: QueueToken,
    },
    /// MOD-12 M2 D9: `clear_queue_setting`, so a cap is unbounded and the box limit inherits.
    ClearQueueSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Where the key is cleared.
        target: QueueTarget,
        /// Which key.
        key: QueueSetting,
        /// The token the editor opened on.
        expected: QueueToken,
    },
```
`name()` (after `WriteDocument`'s arm, `:1261`): `// The three of `queue_settings::REQUEST_NAMES`, in that order (MOD-12 M2 D9).`
then `"queue_settings"`, `"set_queue_setting"`, `"clear_queue_setting"`.

Replies, before `Failed` (`:1648`):
```rust
    /// The scope's queue settings, freshly read: the answer to [`StoreRequest::QueueSettings`] and
    /// to every queue-setting write that applied (MOD-12 M2 D8).
    QueueSettings(Box<QueueSettingsSnapshot>),
    /// A queue-setting write missed its token: the settings as they are now, for the editor to
    /// reload against; it keeps its text and retries only on `Enter` (PRD D8's rule).
    QueueSettingsStale(Box<QueueSettingsSnapshot>),
```
`try_serve` (`:1973`), after the secrets arm (`:2170-2174`):
```rust
        // The three queue-setting requests of `Settings > Queue`, or-ed for the reason the arms
        // above are (MOD-15 M3 plan F-12; MOD-12 M2 D9).
        StoreRequest::QueueSettings(..)
        | StoreRequest::SetQueueSetting { .. }
        | StoreRequest::ClearQueueSetting { .. } => queue_settings::serve(backend, request).await?,
```
Imports: `htui_core::model::QueueSetting`, `htui_core::store::{QueueTarget, QueueToken}`,
`crate::queue_settings::{self, QueueSettingsSnapshot}`.

#### B.5.2 `crates/htui/src/queue_settings.rs` (CREATE, mirrors `prompt_settings.rs` `:1-262`)

```rust
//! The queue settings behind `Settings > Queue` (MOD-12 M2 D8-D10). One read per event, one reply
//! out, every write through `WriteStore::set_queue_setting`/`clear_queue_setting`. The section
//! renders the last snapshot and never patches a row into it. Residue, `prompt_settings`'s: a
//! re-read that fails after an applied write answers `Failed`.

/// One read of the scope (`PartialEq` only: [`Project`] derives no `Eq`).
#[derive(Debug, Clone, PartialEq)]
pub struct QueueSettingsSnapshot {
    /// [`QueueSetting::APP_KEYS`], in that order.
    pub app: Vec<QueueAppEntry>,
    /// Each scope project that still names a row, in scope order.
    pub projects: Vec<QueueProjectEntry>,
    /// This box; `None` before registration.
    pub this_box: Option<QueueBoxEntry>,
}

/// One `app_setting` key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAppEntry {
    /// The key.
    pub key: QueueSetting,
    /// The stored JSON; Postgres seeds `scheduler_window` as JSON `null` (`0003:139`), which
    /// reads as not set.
    pub value: Option<Value>,
    /// The row's token; `Stamp(None)` when there is no row.
    pub token: QueueToken,
}

/// One project's two caps.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueProjectEntry {
    /// The row (name, slug, `updated_at`).
    pub project: Project,
    /// `settings.per_token_cap_run`, as stored.
    pub run_cap: Option<Value>,
    /// `settings.per_token_cap_batch`, as stored.
    pub batch_cap: Option<Value>,
}

/// This box's `max_concurrent_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueBoxEntry {
    /// `box.id`.
    pub id: BoxId,
    /// `box.hostname`.
    pub hostname: String,
    /// `settings.max_concurrent_items`, as stored; `None` inherits.
    pub value: Option<Value>,
    /// `EditVersion(box.edit_version)`.
    pub token: QueueToken,
    /// `admission_limit(box.settings, app)`: what `claim_run` admits (D8).
    pub effective: u32,
}

impl QueueSettingsSnapshot {
    /// The app rows with a value, as `admission_limit` reads them.
    #[must_use]
    pub fn app_map(&self) -> BTreeMap<String, Value>
    /// `admission_limit(&json!({}), &self.app_map())`: the app default the box inherits.
    #[must_use]
    pub fn app_limit(&self) -> u32
}

/// The snapshot (`Backend` for `box_info`, `Writer` for every other read).
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(backend: &Backend, writer: &Writer, scope: &Scope) -> Result<QueueSettingsSnapshot>

/// Serves one queue-setting request, off the UI task. `Unreachable(DATABASE_UNREACHABLE)` on
/// `Backend::Offline` (no writer), the read included, as `prompt_settings::serve` (`:184-188`).
/// # Errors
/// The seam's, `Unreachable` offline, `Backend` for a foreign request.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>

/// `Applied` answers `QueueSettings`, `Stale` answers `QueueSettingsStale`, both re-read.
async fn cas<T>(backend: &Backend, writer: &Writer, scope: &Scope, outcome: &CasOutcome<T>) -> Result<StoreReply>

/// The three request names, in [`StoreRequest`] order.
pub const REQUEST_NAMES: [&str; 3] = ["queue_settings", "set_queue_setting", "clear_queue_setting"];
/// The read's name.
pub const READ_NAME: &str = REQUEST_NAMES[0];
```
`snapshot`: `writer.queue_setting(QueueTarget::App, key)` per `APP_KEYS`; per scope project
`writer.project(id)` (skip `None`) and `project.settings.get(key)`; `backend.box_info()` then
`writer.queue_setting(QueueTarget::Box(info.box_id), MaxConcurrentItems)`, with
`effective = admission_limit(&info.settings, &app_map)`.

#### B.5.3 `crates/htui/src/ui/tabs/settings/queue.rs` (CREATE)

Structure mirrors `PromptSection` (`settings/prompt.rs:56-260`: `Row`/`Target`/`Editor`/`Mode`/
`Notice`/`Reload`, hand-written `Debug` that never prints a buffer); registration, test file and
snapshot naming mirror Secrets (`7188c47d`).

```rust
/// `Settings > Queue` (MOD-12 M2 D8): the spend caps, the minimum, both concurrency levels and
/// the stored window.
#[derive(Debug, Default)]
pub struct QueueSection { /* snapshot, unavailable, cursor, mode, notice */ }

impl QueueSection {
    /// Stable identity.
    pub const ID: SectionId = SectionId("queue");
    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self
}
```
`title()` is `"Queue"` (strip 80 + 7 = 87 of 100 columns). `wants_requests(scope)` is
`vec![StoreRequest::QueueSettings(scope.clone())]`. `captures_input()` is `matches!(self.mode, Mode::Editing(_))`.

Rows, in order (headers are rows, as `PromptSection`'s `Row` makes them):

| Row | Label | Shown value |
|---|---|---|
| `ProjectHeader { p }` | `project {slug}` | |
| `Project { p, key: PerTokenCapRun }` | `run cap` | `format_usd`, or `unbounded` |
| `Project { p, key: PerTokenCapBatch }` | `batch cap` | `format_usd`, or `unbounded` |
| `AppHeader` | `all boxes` | |
| `App { i }` min budget | `min budget` | `format_usd`, or `none` |
| `App { i }` concurrency | `max concurrent` | the integer, or `{DEFAULT_MAX_CONCURRENT_ITEMS} (default)` |
| `App { i }` window | `window` | `format_window` or `not set`, always followed by ` · stored, not enforced` |
| `BoxHeader` | `this box ({hostname})` | |
| `Box` | `max concurrent` | the integer, or `inherit ({effective})` |

Below the rows, one help line: `UNKNOWN_COST = "a run whose agent reports no USD cost is never capped (unknown is unbounded)"`
(plan risk row 2). Constants: `HINT_BROWSE = "j/k · e edit · r reload"`,
`HINT_EDITING = "Enter save · Esc cancel · empty clears"` (`prompt.rs:68`, `:74`),
`NOT_READ = "queue settings not read yet"`, `UNAVAILABLE = "queue settings unavailable"`,
`NOT_A_VALUE_ROW = "`e` edits a value row"`, `NOTHING_SET = "nothing is set here"`,
`WINDOW_NOT_ENFORCED = "stored, not enforced"`.

Keys: `j`/`Down`, `k`/`Up` move over all rows (clamped); `e` (and `Enter`) on a value row opens the
editor prefilled with the shown value as typed (`1.50`, `3`, `22:00-06:00`; empty when unset); `r`
re-sends `wants_requests`; `Esc` drops a notice. In the editor, `Enter` submits, `Esc` cancels, and
every other key goes to the `TextField`.

Submit: empty text with nothing stored is `Notice::Info(NOTHING_SET)` and no request; empty with a
value is `ClearQueueSetting`. Otherwise the text is parsed: money keys with `parse_usd` to
`json!(micros)`, concurrency with `str::parse::<u32>` to `json!(n)`, the window with `parse_window`;
then `key.validate(&value)`. A parse or validator error is `Notice::Error(sentence)`, the editor
stays open, **no request is sent**. A valid value sends `SetQueueSetting` with the target
(`Project(project.id)`, `App`, `Box(box.id)`) and the opened token (`Stamp(Some(project.updated_at))`,
the app entry's token, the box entry's `EditVersion`).

Replies: `QueueSettings` adopts the snapshot, closes an editor whose write landed, clamps the cursor;
`QueueSettingsStale` adopts it, keeps the editor's text, retakes the token by `Reload` (`Gone` closes
with `DELETED_ELSEWHERE`, an `App` row now absent becomes `Stamp(None)`), and says `CHANGED_ELSEWHERE`
(`settings/mod.rs:102`) or `CHANGED_ELSEWHERE_CLOSED` with no editor; `Failed { request, message }`
with `request == READ_NAME` sets `unavailable` (`UNAVAILABLE: {message}`), any other of
`REQUEST_NAMES` is `Notice::Error(message)` with the editor kept. `on_scope_change` drops the snapshot
and the editor.

`settings/mod.rs`: `/// The queue settings section (MOD-12 M2).` `pub mod queue;` after `qdrant`
(`:20`), `pub use queue::QueueSection;` (`:46`), and the module doc's list gains "and MOD-12's queue
caps (`Queue`, milestone 2)". `app/mod.rs`: import, and after `:79`
`// Last (MOD-12 milestone 2, D8): appending moves no existing section's line.`
`Box::new(QueueSection::new()),`.

### B.6 T6: documents

- `docs/ANA-2.md` §4.10: "**As built (MOD-12 M2).** A batch's spend is `Σ run_step.usage.cost_micros`
  over its runs, computed when needed and never stored. The guard applies at three points with one
  rule (`batch_budget`): the runner admits no entry whose project's `per_token_cap_batch` the batch has
  reached, or whose remainder is below `min_budget_for_new_attempt`; the walk refuses a new attempt in
  a batch run on the same rule (`batch cap reached` / `batch budget` skip causes); and a session's
  recorder cap is `min(run cap − run spend, batch cap − batch spend)`, so a breach names the batch
  when the batch bound it. A batch spans projects: each run is guarded by its own project's cap
  against the whole batch's spend (D2). Overshoot is at most one in-flight attempt per running run of
  the batch (D7). Unknown cost is unbounded (OQ-6)." §5.4 near `:1634`: caps live in
  `project.settings`; the seeded `app_setting.per_token_cap_*` rows are unread; `scheduler_window` is
  `null` or `{"start":"HH:MM","end":"HH:MM"}` in the box's local time, `end < start` crossing midnight,
  stored and not enforced (D10).
- PRD: M2 row `complete`; the overshoot metric reads "≤ one in-flight attempt per running run of the
  batch above `per_token_cap_batch` (one attempt at `max_concurrent_items = 1`)" (D7).
- `HANDOFF.md`: P1 phase note; "Live coordinates": migrations unchanged (`0017`), `htui-orch` `CASES`
  107, store conformance `CASES` 164.
- File the CLEAN item of D11 (`Engine::project_settings`/`graph::project_settings`'
  `unwrap_or_default()` drop every setting when one is malformed; `engine.rs:6779-6781`,
  `graph.rs:655-657`). Mention also `SessionSpec::budget_micros`' doc (`htui-agent/src/driver.rs:298-302`),
  which still says `per_token_cap_run` and now carries D6's allowance.
- Validate with `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## C. Tests first (TDD), per task

Each red test lands in a commit only when it compiles; otherwise it lands with the smallest stub
(`todo!()`-free: a stub that returns the wrong answer, so the test fails rather than panics).

### C.1 T1

`model/queue.rs` tests (`:107+`):

| Test | Asserts |
|---|---|
| `batch_budget_is_unbounded_when_either_figure_is_unknown` | `(None, Some(1_000))`, `(Some(900), None)`, `(None, None)` with `min = 200` all `Ok(None)` |
| `batch_budget_is_reached_at_equality` | `(Some(500), Some(500), 0)` and `(Some(600), Some(500), 0)` are `Err(CapReached { spent, cap })` |
| `batch_budget_refuses_a_remainder_below_the_minimum` | `(Some(900), Some(1_000), 200)` is `Err(Budget { remaining: 100, min: 200 })` |
| `batch_budget_admits_exactly_the_minimum` | `(Some(800), Some(1_000), 200)` is `Ok(Some(200))`; `(Some(0), Some(0), 0)` is `CapReached` (`0` is a real cap) |
| `batch_stop_names_the_batch` | Display: `batch cap reached (500 of 500 micros)`, `batch budget: 100 micros left, 200 required` |
| `min_budget_micros_reads_a_positive_integer_else_zero` | `500` → 500; absent, `0`, `-5`, `"500"`, `1.5` → 0 |

`mem.rs` tests (in the M1 block after `delete_project_takes_queue_entries_and_run_batches`, `:14559`).
Steps get usage with `create_step` then `set_step_usage(StepFence::Unleased, step, json!(…), None)`
on an unclaimed run (`StepFence::Unleased` writes where `lease_owner` is NULL, `traits.rs:2878-2880`):

| Test | Asserts |
|---|---|
| `batch_spend_is_none_without_a_costed_step` | A batch run with no step, then a step with `{"input_tokens": 3}`: `batch_spend == None` |
| `batch_spend_sums_only_the_batch_runs` | Two runs in batch A (`batch_run`, `:14126`) with costs 700 and 250 plus `{"cost_micros": "x"}` and `1.5` rows; a run in batch B costing 1 000; a manual run costing 5 000: `batch_spend(A) == Some(950)`, `batch_spend(B) == Some(1_000)` |
| `run_batch_spend_is_none_for_a_manual_run` | The manual run is `None`; a run of batch A is `Some((A, Some(950)))`; an unknown `RunId` is `None` |

`pg_criteria.rs` (beside `:7525`, on `demo_db()` and `MemStore::demo()`, early return without
`HTUI_TEST_DATABASE_URL`):

| Test | Asserts |
|---|---|
| `batch_spend_answers_alike_on_both_stores` | The same fixture as the Mem test through both stores (the `batch_run` helper `:7495`): equal `batch_spend` for A, B and a batch with no costed step (`None`); the non-integer rows are skipped on Pg too (no `22P02`). PRD metric's Pg half: `SUM(run_step.usage)` |
| `run_batch_spend_answers_alike_on_both_stores` | Manual run `None`, batch run `Some((id, spend))` on both; ids differ per store, so compare `(spend, is_some)` and each store's own batch id |

Gates:
```bash
cargo test -p htui-core --all-features -- queue batch
cargo test -p htui-store --all-features --test pg_criteria -- --test-threads=1 batch
cargo build --workspace --all-features --all-targets
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
cargo clippy --workspace -- -D warnings
```

### C.2 T2

`select.rs` tests:

| Test | Asserts |
|---|---|
| `a_batch_at_its_cap_skips_every_candidate` | `batch_spent 500, batch_cap 500`: nothing eligible, every cause `BatchCapReached { spent: 500, cap: 500 }`, Display `batch cap reached (500 of 500 micros)`; `499` admits both |
| `a_short_batch_budget_skips_and_exactly_the_minimum_is_enough` | `min 200`: `900 of 1 000` is `BatchBudget { remaining: 100, min: 200 }` (`batch budget: 100 micros left, 200 required`); `800 of 1 000` eligible |
| `unknown_batch_figures_are_unbounded` | `(None, Some)`, `(Some, None)`, `(None, None)` with `min 200` all eligible |
| `the_run_rules_come_before_the_batch_rules` | Run at its cap and batch at its cap: the cause is rule 2's `Quota(CapReached)`; run budget short and batch at cap: rule 5's `Budget` |

`gate.rs` tests: `a_batch_bound_breach_names_the_batch` (`cap_batch: Some(b)` settles
`CapBreached { batch: Some(b) }`, Display `cap breached (batch {b})`, `run_failure_text` the same);
the existing five assertions updated (§B.2.3).

`engine.rs` unit tests (pure, beside the other `min_budget`-level tests):

| Test | Asserts |
|---|---|
| `session_allowance_takes_the_smaller_known_term` | run `1 000 − 600 = 400` vs batch `5 000 − 4 800 = 200` → `200, batch: Some(id)`; batch `5 000 − 100` → `400, batch: None` |
| `session_allowance_names_the_batch_only_when_it_binds` | equal terms name the run; batch alone (no run cap) names the batch |
| `session_allowance_floors_at_zero_and_reads_unknown_spend_as_zero` | over-spent terms are `0`; `run_cap 1 000, run_spent None` → `1 000`; no cap at all → `None` |

Conformance cases (`conformance.rs`, all `Box::pin`ned in `spend_guard_case`). Box and user are
`ids::BOX` and the store's `this_user()` (`fake.rs:1676-1695`). Caps are planted with
`orch.store().set_project_settings` **merged** into the current blob (read with
`project_settings`, as `engine.rs:8198-8212`'s `cap_feat_3` does: the hook replaces the whole blob).

| Case | Body and asserts |
|---|---|
| `a_batch_at_its_cap_starts_no_attempt` | `open_batch`; `script("research", 1, done_costing("r", 600))`; `enqueue_in_batch(HTUI_ANA_2, b)` then `claim`: research runs (auto skips its soft gate), verdict parks; `batch_spend(b) == Some(600)`. Then plant `per_token_cap_batch = 500`, mint a second ANA item (the `mint_feat` recipe, `:4257`, with `KIND_HTUI_ANA`), `enqueue_in_batch` and `claim`: `Rest { run: Failed, failure: NoCandidateAgent { phase: "research", detail } }` with `detail` containing `batch cap reached (600 of 500 micros)`; no step row for the second run; the item is `Blocked` with that note |
| `a_batch_run_session_is_capped_at_the_batch_remainder` | Plant `per_token_cap_batch = 1 000` first (the snapshot freezes it). `script("research", 1, done_costing(_, 600))`, `script("verdict", 1, done_costing(_, 600))`. `enqueue_in_batch(HTUI_ANA_2, b)`, `claim`. `spec_for(&SessionKey { phase: "verdict", attempt: 1, fanout_index: 0, call: 0 })` has `budget_micros == Some(400)`; the verdict step (attempt 1) is `Failed` with `gate_note == Some(format!("cap breached (batch {b})"))`; `batch_spend(b) == Some(1_200)`: the overshoot is at most the one attempt in flight (PRD metric, Mem half) |
| `a_manual_run_ignores_batch_figures` | A batch with spend 600 (as in the first case) and `per_token_cap_batch = 0` planted after it. `start(&orch, <a second ANA item>)` (Manual, `:1323`): it walks to its first park (a manual snapshot keeps every gate) with no `BatchCapReached` refusal; its research session's spec has `budget_micros == None`; `store.run_batch_spend(run) == None`; no step `CapBreached` |
| `the_run_cap_spans_steps` | `feat_3_gated(&orch, Gate::Never, \|_\| {})` (`:4770`); `per_token_cap_run = 1 000` merged; `script("prd", 1, done_costing(_, 600))`, `script("plan", 1, done_costing(_, 500))`. `plan`'s spec has `budget_micros == Some(400)`; `plan` attempt 1 is `Failed` with `gate_note == Some("cap breached")` (no batch). Before D6 its allowance was a fresh 1 000 and it passed |

**VERIFY**: the fake demo's ANA graph is `research` (soft) then `verdict` (hard) under the fake as
under `auto_queue.rs` (`:142`'s doc); if the fake's ANA phases differ, use `fan_research`-style
`repoint` to make it so, and keep the asserted positions.

Count pins: `cases_are_unique_and_counted` (`:7919-7962`) and `cases_len_is_pinned`
(`fake_conformance.rs:13-23`) 103 to 107.

Gates:
```bash
cargo test -p htui-orch --all-features -- --no-fail-fast 2>&1 | tee /tmp/m2-orch.log
grep -n -E 'SIGABRT|overflowed|FAILED' /tmp/m2-orch.log
RUST_MIN_STACK=1572864 cargo test -p htui-orch --all-features --lib every_case_name_dispatches   # headroom, optional
cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast run_worker runs_pg worker_pg chat 2>&1 | tee /tmp/m2-htui-walks.log
grep -n -E 'SIGABRT|overflowed|FAILED' /tmp/m2-htui-walks.log
```

### C.3 T3: `crates/htui-worker/tests/auto_queue.rs`

Harness additions (all in this file):
- `Hold` (`:178-196`) gains `cost: Option<i64>`; with `Some(n)` the script is
  `[Emit(Usage(UsageEvent { cost_micros: Some(n), ..UsageEvent::default() })), Emit(Done(EndTurn))]`,
  the shape of `ScriptedStep::done_costing` (`htui-orch/src/fake.rs:1151-1166`). `Parts::costing(n)`
  builds a runtime whose sessions each cost `n`; `Harness::costing(n)` is `Harness::open()` with it.
- `mint_in(store, project, kind, title, priority)`; `mint` (`:146-169`) calls it with
  `PROJECT_HTUI`/`KIND_HTUI_ANA`. Project B is `PROJECT_AGY` with `KIND_AGY_ANA`
  (`fixtures.rs:161`, `:179`). **VERIFY** that an agy ANA run resolves to the scripted agent under
  `seed` (`:98-131`) as the htui one does, and that the agy project has no repo that overlaps.
- `cap_batch(h, project, micros)`: merge `per_token_cap_batch` into
  `h.store.project_settings(project)` and write it with `MemStore::set_project_settings`
  (`mem.rs:667-675`, tests only; it replaces the whole blob).
- The minimum is planted with `MemStore::set_app_setting("min_budget_for_new_attempt", json!(m))`
  (`mem.rs:656-659`).

Spend is made by a run that is **already parked** when the cap is planted: its snapshot froze no
batch cap, so T2's D6 never cuts its sessions (H-6).

| # | Test | Asserts |
|---|---|---|
| a | `a_batch_at_a_projects_cap_admits_only_other_projects` | `Harness::costing(300)`. Queue A1 (htui), resume, sweep, settle: A1 parks at verdict, batch spend 600. `cap_batch(PROJECT_HTUI, 600)`. Queue A2 (htui) and B1 (agy), sweep, settle: A2 has no run, B1 has a run in `batch_runs(batch)` |
| b | `a_remainder_below_the_minimum_admits_nothing` | Spend 600 as in (a); `cap_batch(PROJECT_HTUI, 700)`, `set_app_setting(min, 200)`. Queue A2: no run after sweep and settle (100 left < 200) |
| c | `a_pause_and_a_resume_open_a_batch_that_admits_again` | After (a)'s stop: `close_batch(Paused)`, `resume`, sweep: A2 gets a run in the **new** batch (its spend starts `None`, unbounded). Assert admission only (H-6) |
| d | `a_malformed_cap_skips_its_project_and_admits_the_next` | `set_project_settings(PROJECT_HTUI, {… "per_token_cap_batch": "lots"})` with no spend. Queue A1 then B1: A1 has no run (fail closed), B1 has one, in the same sweep |
| e | `a_stopped_batch_stays_open_and_keeps_its_entries` | After (a)'s stop and a second sweep: `open_batch_of(BOX)` is the same batch, `closed_at` `None`, `queue_entries` still holds A2 |
| f | `a_manual_runs_spend_does_not_count_against_the_batch` | `Harness::costing(300)`. A manual `StartRun` on M (htui) through `serve_request` (the `a_manual_run_holds_off…` recipe, `:925`) parks at its first gate (manual keeps every gate), spending 300 outside any batch. `cap_batch(PROJECT_HTUI, 300)`; queue A1, resume, sweep: A1 has a run (batch spend is `None`) |

Runtime unit test (`runtime.rs:3883`): the call list gains `"batch_spend"` and `"project_settings"`
after `"app_settings"`; the fixture's entry (`HTUI_ANA_2`, two free slots) reaches both.

Gate: `cargo test -p htui-worker --all-features -- --test-threads=1 --no-fail-fast`.

### C.4 T4

`model/queue_settings.rs` tests:

| Test | Asserts |
|---|---|
| `queue_setting_texts_are_the_stored_keys` | `ALL` texts; `PROJECT_KEYS`, `APP_KEYS`, `BOX_KEYS` as §B.4.1 |
| `caps_take_a_non_negative_integer_and_say_cap_errors_sentence` | `0`, `1_500_000` ok; `-1`, `1.5`, `"1"`, `null` refused with `project.settings.per_token_cap_batch must be a non-negative integer of USD micros, got …` |
| `min_budget_takes_a_positive_integer` | `1` ok; `0`, `-1`, `"5"` refused |
| `max_concurrent_items_takes_at_least_one` | `1`, `u32::MAX` ok; `0`, `u32::MAX + 1`, `2.0` refused |
| `the_window_takes_hh_mm_pairs_and_may_cross_midnight` | `parse_window("22:00-06:00")` → `{"start":"22:00","end":"06:00"}`, validates; `24:00-01:00`, `9:00-10:00`, `10:00-10:00`, `10:00` refused; `null` validates; `format_window` inverts |
| `usd_parses_to_micros_and_formats_back` | `parse_usd("1.5") == Ok(1_500_000)`, `"$1.50"` too, `"0.000001"` → `1`; `"1.2345678"` → `USD_TOO_PRECISE`; `"-1"`, `"1e3"`, `"abc"` → `USD_NOT_A_NUMBER`; `format_usd(1_500_000) == "$1.50"`, `1_234_567` → `"$1.234567"`, `0` → `"$0.00"`; `parse_usd(&format_usd(m)) == Ok(m)` for a spread of `m` |

Conformance (`store/conformance.rs`, appended to `CASES` `:60-220`, dispatched in `run_case`). Each
case reads its token with `queue_setting` first: Postgres seeds `max_concurrent_items` and
`scheduler_window` app rows (`0003:127-140`) and the Mem demo seeds neither (H-8).

| Case | Asserts |
|---|---|
| `queue_setting_on_a_project_merges_under_cas_and_keeps_foreign_keys` | `PROJECT_HTUI`: set `PerTokenCapBatch = 1_500_000` with the project's `Stamp`: `Applied`, the read answers it, every other `project.settings` key survives; the old token is `Stale` and writes nothing; `Stamp(None)` and `EditVersion` are refused |
| `queue_setting_on_app_upserts_and_refuses_a_stale_token` | `MinBudgetForNewAttempt`: `Stamp(None)` inserts; a second `Stamp(None)` is `Stale` carrying the row; the fresh token updates; `MaxConcurrentItems` on `App` writes the app rung (whatever its seed) |
| `clear_queue_setting_removes_the_key_on_every_target` | Each target: clear answers `Applied { value: None }`; the read answers `value: None` (`App`: no row, token `Stamp(None)`); a project or box clear of an absent key applies; a token over an absent `app_setting` is `NotFound` |
| `queue_setting_on_a_box_merges_under_edit_version_and_keeps_executor` | `edit_box(BOX, 0, executor_edit(Worker))` first (`:9928` shape), then `set_queue_setting(Box(BOX), MaxConcurrentItems, 3, EditVersion(1))`: `Applied`, `edit_version` 2, blob `{"max_concurrent_items": 3, "executor": "worker"}`; `EditVersion(1)` again is `Stale`; an `edit_box` on the old version is `Stale` too (one guard) |
| `queue_setting_refuses_a_foreign_key_and_a_bad_value_before_the_row` | `PerTokenCapRun` on `App`, `SchedulerWindow` on `Box`: `Constraint` with the target sentence; `MaxConcurrentItems = 0` on `Box` with a **spent** token: `Constraint` (validator), not `Stale`; nothing written |

`CASES` 159 to 164; `pg_conformance.rs` `EXPECTED_CASES` 159 to 164 with its doc line
"and MOD-12 M2's five queue-setting cases (plan D9) make it 164".

Gates:
```bash
cargo test -p htui-core --all-features
cargo test -p htui-store --all-features -- --test-threads=1 --no-fail-fast
cargo test -p htui-agent --all-features
cargo build --workspace --all-features --all-targets
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
cargo clippy --workspace -- -D warnings
```

### C.5 T5: `crates/htui/tests/queue_settings.rs`

Mirrors `tests/prompt_settings.rs` (serve half `:41-560`, section half `:607+`: `demo()`,
`settings(reply)`, `stale(reply)`, `refusal(reply)`, `bench_with(backend)`, `feed`).

Serve half (over `Backend::memory(MemStore::demo())`):

| Test | Asserts |
|---|---|
| `queue_settings_names_are_stable` | `REQUEST_NAMES` equals the three `name()`s in order |
| `the_demo_snapshot_has_three_app_keys_each_scope_project_and_this_box` | `app` keys are `APP_KEYS`; the scope's projects; `this_box` is `ids::BOX` with `value: Some(2)` and `effective: 2` |
| `set_queue_setting_on_a_project_stores_micros_and_keeps_foreign_keys` | answers `QueueSettings`, the cap reads back, other keys survive |
| `clear_queue_setting_on_app_returns_the_entry_to_none` | |
| `a_spent_token_answers_queue_settings_stale` | |
| `the_box_limit_writes_box_settings_under_edit_version` | `this_box.token` moves to `EditVersion(1)` |
| `offline_refuses_all_three_by_name` | the `prompt_settings.rs:516` recipe: each answers `Failed` naming its request with `DATABASE_UNREACHABLE` |
| `serve_refuses_a_foreign_request_by_name` | |

Section half (`SectionBench`):

| Test | Asserts |
|---|---|
| `the_section_renders_three_groups_with_effective_values` | `render_section` holds `project htui`, `all boxes`, `this box (…)`, `unbounded`, `2 (default)` or the seeded value, `inherit (2)` after a clear; snapshot `queue_settings__demo` |
| `a_usd_entry_stores_micros` | `e` on `batch cap`, type `1.5`, `Enter`: the drained request is `SetQueueSetting { key: PerTokenCapBatch, value: json!(1_500_000), expected: Stamp(Some(updated_at)), .. }`; snapshot `queue_settings__editing` before `Enter` |
| `an_empty_field_clears_the_key` | the drained request is `ClearQueueSetting`; with nothing stored it is `NOTHING_SET` and no request |
| `an_invalid_value_shows_the_validators_sentence_and_writes_nothing` | `1.2345678` shows `USD_TOO_PRECISE`; `0` on `max concurrent` shows the validator sentence; nothing drained; snapshot `queue_settings__invalid` |
| `an_empty_box_limit_shows_the_inherited_value` | after a box clear reply, the row reads `inherit ({app_limit})` |
| `the_window_row_says_stored_not_enforced` | set and unset both carry `stored, not enforced` |
| `offline_the_section_says_the_server_is_needed` | a `Failed { request: "queue_settings", message: DATABASE_UNREACHABLE }` reply; snapshot `queue_settings__offline` |
| `a_cas_conflict_shows_changed_elsewhere` | a `QueueSettingsStale` with a moved token: the editor keeps its text, `CHANGED_ELSEWHERE` shows, the next `Enter` carries the new token; snapshot `queue_settings__changed_elsewhere` |
| `editing_captures_input` | `captures_input()` true while editing, false in browse |

`store_worker.rs` tests: `queue_settings_requests_are_named_as_request_names_lists_them` (the shape
of `queue_requests_are_named_as_queue_request_names_lists_them`, `:5121`).
`tests/settings.rs`: the strip test (`:1091-1112`) gains `Box::new(QueueSection::new())` and `10`.

Gates:
```bash
cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast
cargo insta test -p htui --all-features   # expect exactly five new queue_settings__*.snap, nothing else
```

---

## D. Commits per task

The trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage by explicit path.
Implementers commit incrementally (uncommitted subagent work dies with its session).

- **T1**
  - `test(mod-12): batch_budget and min_budget_micros pins (M2 D3)` (with the stub returning `Ok(None)`)
  - `feat(mod-12): batch_budget, BatchStop and min_budget_micros (M2 D3)`
  - `feat(mod-12): batch_spend and run_batch_spend on MemStore, PgStore, Backend; .sqlx (M2 D1, D5)`
  - `feat(mod-12): WorkerHost batch_spend/project_settings, WorkerStore run_batch_spend; parity tests`
- **T2**
  - `test(mod-12): batch skip rules in select::walk (M2 D3, D5)` then `feat(mod-12): SkipCause::BatchCapReached and BatchBudget; walk_candidates reads the batch`
  - `feat(mod-12): D6 session allowance - run cap spans steps, batch remainder caps a batch session; CapBreached names the batch`
  - `test(mod-12): four spend-guard conformance cases; CASES 107`
- **T3**
  - `test(mod-12): the runner's spend gate over Backend::Memory (M2 D4)` (costing harness + six tests)
  - `feat(mod-12): admit skips entries the batch's spend stops; once-per-batch notes (M2 D4)`
- **T4**
  - `feat(mod-12): QueueSetting, validators, USD and window text (M2 D8-D10)`
  - `feat(mod-12): WriteStore queue_setting/set_queue_setting/clear_queue_setting on MemStore; five conformance cases`
  - `feat(mod-12): queue-setting writes on PgStore, Writer and the agent spies; .sqlx`
- **T5**
  - `feat(mod-12): queue-setting StoreRequests and their serve module (M2 D9)`
  - `feat(mod-12): Settings > Queue section, registered last (M2 D8)`
  - `test(mod-12): Settings > Queue serve and section tests, five snapshots, strip of ten`
- **T6**: `docs(mod-12): M2 phase note - spend guard landed; PRD milestone 2 complete, D7 metric amended`

Full validation (plan §Validation, run by the orchestrator after T5 and again after T6):
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod12-m2.log
grep -n -E 'SIGABRT|overflowed|test result: FAILED' /tmp/mod12-m2.log
cargo doc --workspace --no-deps --all-features
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## E. Hazards (checked against the tree)

| # | Hazard | Evidence | Handling |
|---|---|---|---|
| H-1 | `htui-orch` debug-build stack headroom | Repo memory "htui-orch test stack headroom"; `case()` doc `:676-686`; `dispatch` boxes every arm (`engine.rs:625-630`) | The four cases sit in `spend_guard_case`, every arm `Box::pin`ned. `open_recorder` grows by two awaits; keep `allowance` a separate `async fn` (not inlined). Gate with `--no-fail-fast` + grep `SIGABRT\|overflowed`, including `htui`'s `run_worker`/`runs_pg`/`worker_pg`/`chat` walks; optional `RUST_MIN_STACK=1572864` before/after |
| H-2 | `.sqlx` against a migrated scratch DB | `docs/hr-sandbox.md:196-210`; repo memory "sqlx prepare needs a migrated scratch DB" (`HTUI_TEST_DATABASE_URL` is `…/postgres`, not the prepare DB) | `psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"`, `cargo sqlx migrate run --source migrations`, then prepare with `-- --all-targets --all-features`. T1 adds 2 files; T4 adds 2 (box). T4 runs after T1 is merged; re-run `--check` at collect |
| H-3 | Byte-identical literal reuse | `pg/write.rs:1955-1957` | T4's App and Project statements and both re-reads must be copied with their exact indentation; otherwise prepare adds files (harmless, but they must be committed). `cargo sqlx prepare --check` tells |
| H-4 | Integration tests silently run 0 tests | Repo memory "htui integration tests need testkit" | Every `htui`/`htui-worker`/`htui-store` gate uses `--all-features` |
| H-5 | Featureless clippy | Repo memory "featureless clippy gate"; `enqueue_in_batch_fake` and the fake method are `test-support`-gated | `cargo clippy --workspace -- -D warnings` at T1, T2, T4 and the end |
| H-6 | T2 ∥ T3 coupling | After T2, a batch run whose snapshot froze a batch cap gets a D6 allowance; the session that crosses the cap is cut by design | T3 makes spend with a run parked **before** the cap is planted (its snapshot froze none), and asserts admission, never the walk outcome, of a run admitted under a cap (§C.3 (c)) |
| H-7 | Keyring fake is process-wide | Repo memory "htui suite green is scheduling-dependent" | `--test-threads=1` for every `htui` gate |
| H-8 | Pg and Mem app seeds differ | `0003_orchestration.sql:127-140` seeds `max_concurrent_items = 2`, `scheduler_window = null`, `per_token_cap_* = null`; `MemStore::from_demo` starts `app_settings` empty (`mem.rs:392`) | Conformance and serve tests read the token with `queue_setting` before writing; the section shows JSON `null` as unset; never assert an app row's presence across stores |
| H-9 | `set_project_settings` replaces the blob | `mem.rs:660-675` | Merge into the current blob first (`engine.rs:8198-8212` recipe) in T2 cases and T3 tests |
| H-10 | `Backend::project_settings` offline answers the mirror | `backend.rs:319-325` | Harmless: `admit` returns at `open_batch_of`, which refuses offline, before any cap read |
| H-11 | `CASES` pins exist twice in `htui-orch`, and the store suite's count twice | `conformance.rs:7924`, `fake_conformance.rs:15`; `store/conformance.rs` `CASES` + `pg_conformance.rs:33` | T2 bumps both to 107; T4 bumps both to 164 |
| H-12 | `Shared`'s literal is exhaustive | `assemble` `runtime.rs:1185-1215` | Add both mutexes there or the build fails |
| H-13 | `budget_micros` is now the allowance | `htui-agent/src/cli/mod.rs:196` passes it only when `> 0` | An allowance of `0` (an in-flight race) sends no `--max-budget-usd`; the recorder still cancels at the first costed row (`record.rs:1636-1650`, `>=`) |
| H-14 | Fan-out siblings start together | Each candidate's `open_recorder` reads the batch and run spend at its own start | Accepted by D7: the bound is one in-flight attempt per running run; the PRD metric is amended in T6 |
| H-15 | The `edit_box` "only writer" doc becomes false | `traits.rs:552-557` | T4 amends it (§B.4.2); `Boxes` editors go `Stale` after a queue box write, which is the shared guard working |
| H-16 | `D11`'s malformed-settings drop | `engine.rs:6779-6781`, `graph.rs:655-657` | Not fixed here (D11); D4 fails closed on its own read; T6 files the CLEAN item |
| H-17 | Disk pressure kills the dev Postgres | Repo memory "Dev Postgres crash loop" | `df -h .` before long Pg gates; `cargo clean -p` a stale target if needed |

---

## F. Plan deviations

1. **The project-settings read already exists on all three stores.** The plan lists a new Pg
   "project settings read" for T1 (`pg/read.rs` row) and says `WorkerHost` "has no project read".
   The second is true; the first is not needed: `PgStore::project_settings`
   (`pg/read.rs:1550-1558`, `SELECT settings FROM project WHERE id = $1`), `MemStore::project_settings`
   (`mem.rs:525-527`) and `Backend::project_settings` (`backend.rs:319-325`) exist.
   `WorkerHost::project_settings` forwards to them and adds no SQL. Unlike the queue reads,
   `Backend`'s `Offline` arm answers from the mirror (H-10).
2. **T2's file set is wider than `select.rs`, `engine.rs` and `conformance.rs`.** D6's "the failure
   text naming the batch" is written by `gate::apply` from `StepFailure`'s `Display`
   (`gate.rs:139`, `:256`), so the batch has to reach `gate.rs` (`StepFailure::CapBreached { batch }`,
   `SettleInput::cap_batch`), and `recover.rs:337` builds a `SettleInput` literal. A case can make a
   batch run only through `Engine::enqueue_in_batch`, which no `Command` carries, so
   `Orchestrate`/`FakeOrchestrator` gain `enqueue_in_batch` (`fake.rs`). `tests/fake_conformance.rs`
   pins the count. All five files are `htui-orch`, which T3 and T4 never touch, so T2 ∥ T3 ∥ T4 holds.
3. **T1 also edits `model/mod.rs`** (re-exports). T1 runs before T4, which also edits it, so no
   wave conflict.
4. **T4 adds a third `WriteStore` method, `queue_setting` (a read with its token), plus
   `store/mod.rs` and `pg_conformance.rs`.** T5's snapshot needs the `app_setting` rows' tokens, and
   no read answers a token for an arbitrary key: `setting` takes the prompt registry's
   `SettingKey` (`traits.rs:1148`), which D9 rules out, and `Backend::app_settings` drops the token
   (`mem.rs:640-648`). `store/mod.rs:16-35` is where `SettingRung`/`StoredSetting` are exported;
   `pg_conformance.rs:33` pins the store suite's count (159).
5. **`expected` is a `QueueToken`, not a timestamp.** D9 names the three targets but not the token
   type. The box target's guard is `edit_version: i32` (`edit_box`, `traits.rs:571`), the other two
   are `updated_at`, so one enum carries both and a mismatched kind is refused before the row.
6. **No strip snapshot changes.** The plan expects "affected strip snapshots". Strip snapshots render
   only the sections their test registers (e.g. `hierarchy__demo.snap` shows `Agents  Hierarchy`),
   and `7188c47d` added Secrets without touching one. Only `tests/settings.rs`'s count moves
   (9 to 10, 87 of 100 columns).
7. **D6's run term reads the live project settings, as today.** `open_recorder` reads
   `per_token_cap_run` from the live project (`engine.rs:6152-6164`), not the snapshot the walk uses.
   D6 keeps that line's source and subtracts the run's spend; the batch term comes from the snapshot
   per D2. If the maintainer strikes D6's run half, `allowance`'s run term reverts to
   `settings.per_token_cap_run` with no subtraction, and `the_run_cap_spans_steps` goes.
8. **D1's SQL gains a cast and a guard.** `SUM(bigint)` is `numeric`, which the workspace `sqlx`
   cannot decode (no decimal feature), hence `::bigint`; and a non-integer `cost_micros` would raise
   `22P02` on the bare cast where Mem's `as_i64` skips it, hence the `~ '^-?[0-9]+$'` filter.
   The figure is D1's.
9. **Line drift in the plan's grounding.** `enqueue_in_batch` is `engine.rs:691-697` with its body
   in `enqueue_with` `:700-760` (plan: `:699-757`; the `batch_id: batch` write is at `:757` as the
   plan says). `Mem run_batches` is `mem.rs:305` (plan: `:304`). No decision changes.
