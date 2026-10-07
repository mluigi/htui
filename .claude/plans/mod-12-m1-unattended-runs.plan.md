# Plan: MOD-12 M1 — Unattended runs

**Source PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md`
**Selected Milestone**: 1 — Unattended runs
**Complexity**: Large
**Status**: complete — merged on `hr/MOD-12` at `d19767df` (2026-10-07), review round included
**Execution**: ultracode accepted for implement and review (PRD header). Waves below are decided by
the file sets in "Files to Change", not by prose.

## Summary

The maintainer queues items from the Backlog (`Q`), resumes the box's queue (`P`), and the box's
executing process (online TUI or headless worker, whichever the box's executor names) admits the
queued items that are ready, in queue order, as `RunMode::Auto` runs, up to the box's slot cap. The
existing claim scan claims and walks them. Auto runs snapshot non-hard gates as `never`, so their
steps land `done` with `gate_outcome = skipped`; hard gates still park. Pausing (`P` again) closes
the batch: no new admissions, running runs untouched. One forward-only migration `0016` adds the
queue and batch tables and `run.batch_id`, so M2 (spend guard) needs no schema change.

## Grounding (read on `hr/MOD-12`, base `94c434de`; see Verified claims)

- `ready_items(scope, box_id)` is an **inherent** method on `PgStore` (`crates/htui-store/src/pg/read.rs:2070`),
  `MemStore` (`crates/htui-core/src/store/mem.rs:758`) and `Backend` (`crates/htui-store/src/backend.rs:540`,
  offline → `orchestration_offline()`), not a trait method. It has the full ANA-9 §7.4 predicate but
  orders `array_position($1, project_id), key_prefix, key_number`. Only tests call it.
- `ItemSummary` (`crates/htui-core/src/model/item.rs:183`) has `priority`, no `created_at`, and its
  SQL projections bind positionally ("Appended, never inserted"). `Item` (Mem state) has both.
- **`Status::Queued` already means "a run exists"**: `create_run` moves the item `open|failed → queued`
  under a status CAS (`crates/htui-store/src/pg/write.rs:4135`, `:4214`). Queue membership (PRD D1)
  therefore needs its own table; an item drops out of `ready_items` once its run is created.
- Gate downgrade stub: `graph::resolve` takes `mode` (`crates/htui-orch/src/graph.rs:300`) but
  `snapshot_phase` (`:683`, single call `:339`) does not; it writes `gate_effective: phase.gate`
  (`:729`). Readers already honour `gate_effective` (`gate.rs:394` → `pass_step` writes
  `GateOutcome::Skipped`; `select.rs:172`). Test `field_chains_walk_phase_project_app` asserts
  `gate_effective == gate` (`:1171`).
- Run creation: `Engine::enqueue(item, mode, repo_scope)` (`crates/htui-orch/src/engine.rs:659`):
  missing-tags refusal, `graph::resolve(.., mode, ..)`, `create_run(NewRun{ .., target_box_id: self.parts.box_id, .. })`.
  The only production `StartRun` is the Runs pane `R` key with `RunMode::Manual` (`runs.rs:661`).
- Runtime: `RunRuntime<H, P>` (`crates/htui-worker/src/runtime.rs:1096`) is shared by the TUI
  (`RunRuntime<Backend, TuiReplies>`, `crates/htui/src/run_worker.rs:39`) and the headless worker
  (`RunRuntime<PgStore, Unaddressed>`, `crates/htui-worker/src/worker.rs:52`). `sweep_once` (`:1918`)
  returns early unless `role.executes(executor)`, then adopts, then `claim_scan` when `claims`.
  `start_run` (`:2286`) builds the engine with `Kit::read` and calls `engine.enqueue`. Memory/demo
  builds use `without_claim_scan()` (`run_worker.rs:52-56`). Sweep cadence: headless 5 s
  (`worker.rs:14`); TUI = lease period, default 120 s (`runtime.rs:1077`).
- `WorkerHost` (`crates/htui-core/src/store/worker.rs:426`) has exactly two implementors: `PgStore`
  and `Backend` (`crates/htui-store/src/worker.rs:707`, `:787`).
- `NewRun` (`crates/htui-core/src/model/run.rs:351`, no `Default`) is built at 27 sites in 13 files.
- Migrations end at `0015_command_queue.sql` here and on the host tree (`/host/htui`). Count
  literals: `crates/htui-store/tests/migrations.rs:96,1088,1189,1198,1221,1373`,
  `crates/htui-store/tests/connect.rs:140,156,242`.
- Backlog left pane (`crates/htui/src/ui/tabs/backlog/mod.rs:634`) binds `j k g G l ] h [ m f F N e Enter`
  and forwards the rest to the detail pane; the Runs pane binds `a x r p s A o c T u R C 1-9 v + - =`.
  `Q` and `P` are free in both and globally (`keymap.rs` binds `q` only). Help rows are registered
  in `crates/htui/src/app/mod.rs:129-170`; the Backlog help line is asserted by
  `m_is_on_the_backlog_help_line` (`crates/htui/tests/backlog.rs:267`), not by a snapshot.
- The SQLite mirror has a `run` table (`cache_migrations/0001_mirror.sql:103`); queue tables and
  `run.batch_id` are not mirrored (ANA-10 withdrawn, PRD out of scope), so every queue request is
  refused offline like `ready_items`.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Migration | `crates/htui-store/migrations/0015_command_queue.sql` | Header `-- 00NN_name.sql - MOD-NN (plan Dn). Forward-only (R-STO-5).`, rationale comment, `COMMENT ON` per column |
| Inherent orchestration read | `queued_runs_on_box` (Mem `mem.rs:824`, Pg, `Backend` `backend.rs:583`, `WorkerHost` + 2 impls) | Unmirrored tables → inherent methods dispatched by `Backend`, exposed to the runtime via `WorkerHost` |
| Store write + TUI request | `SetAgentOnBox` (`store_worker.rs:314`, name `:1044`, route `:1899`, serve `agent_settings.rs:665`) | `StoreRequest` variant → serve arm → store method → re-read reply |
| Runtime engine use | `start_run` (`runtime.rs:2226`) | `Kit::read` → `kit.engine(&driver)` → `engine.enqueue` |
| Orchestrator conformance | `conformance.rs:438` `CASES`, `:7689` example, `cases_are_unique_and_counted` `:7745` | New case name in `CASES`, dispatched from a `*_case` group, count assertion bumped |
| Pg/Mem parity | `inherent_orchestration_reads_answer_the_fixture` (`crates/htui-store/tests/pg_criteria.rs:3928`) | Same fixture through both stores, equal answers |
| Key + help row | `app/mod.rs:129-170`, `tests/backlog.rs:267` | Raw `KeyCode` arm in the tab plus a `Binding` row for the help line |

## Decisions (proposed; CONFIRM accepts or overrides)

- **D1 — One migration, `0016_auto_queue.sql`, for all of MOD-12.** Tables:
  - `queue_batch(id UUID PK, box_id UUID NOT NULL REFERENCES box, opened_at TIMESTAMPTZ NOT NULL,
    opened_by UUID NOT NULL REFERENCES app_user, closed_at TIMESTAMPTZ NULL, closed_reason TEXT NULL
    CHECK (closed_reason IN ('paused','drained')), CHECK ((closed_at IS NULL) = (closed_reason IS NULL)))`
    plus a partial unique index `ON queue_batch(box_id) WHERE closed_at IS NULL` — at most one open
    batch per box, enforced by the database.
  - `queue_entry(item_id UUID PK REFERENCES item ON DELETE CASCADE, box_id UUID NOT NULL REFERENCES box,
    position INTEGER NULL, queued_at TIMESTAMPTZ NOT NULL, queued_by UUID NOT NULL REFERENCES app_user)`.
    An item is queued on at most one box (PK); `box_id` is the local box at `queue` time (R-ORCH-12
    v1 half: target stored, local execution only).
  - `ALTER TABLE run ADD COLUMN batch_id UUID NULL REFERENCES queue_batch(id)` — `NULL` for manual and
    chat runs. M2 sums `run_step.usage` over it; no stored total (ANA-2 §4.10).
  The number `0016` is re-checked against the host tree at merge (risk table).
- **D2 — The queue runs iff the box has an open batch.** Pause = close the open batch
  (`closed_reason = 'paused'`); resume = open a new one (PRD: pause ends the batch). No separate
  paused flag, so "paused" and "batch" can never disagree. A fresh box, or one whose batch drained,
  is paused: queueing an item never starts spending by itself — `P` does.
- **D3 — Drain closes the batch.** At a runner tick with an open batch, entries whose item is
  `done` or `closed` are removed; when no entry remains and no auto run of the batch is non-terminal,
  the batch closes with `closed_reason = 'drained'`. Items that are `failed`, `blocked` or
  `awaiting_approval` keep their entry and keep the batch open (they are M3's escalations).
- **D4 — Queue order is `position NULLS LAST`, then `ready_items`' order.** `queue` inserts
  `position = NULL`; M3's reorder writes explicit positions. `ready_items` is fixed to
  `ORDER BY i.priority DESC, i.created_at, i.id` (ANA-2 criterion 22); `ItemSummary` is **not**
  changed (Pg orders by the column without projecting it; Mem sorts on its internal `Item`).
- **D5 — The runner composes `ready_items`, it does not re-implement it.** Per tick: read the
  box's entries, call `ready_items(scope, box)` with `scope` = the entries' distinct projects, keep
  entries whose item is ready, sort by D4. One readiness predicate, tested once.
- **D6 — Admission lives in `sweep_once`, before `claim_scan`, under the same `claims` guard.**
  Only the box's executing process runs it (the existing `role.executes(executor)` early return).
  Free slots = `max_concurrent_items` − (running + queued runs targeted at the box). For each ready
  entry up to the free count: `engine.enqueue(item, RunMode::Auto, None)` stamped with the open
  batch; the claim scan that follows claims and walks it (no second admission path). An enqueue
  refusal (missing tags → item `blocked` + note, existing behaviour) is logged and the next entry
  tried. Two runners racing one item are serialised by `create_run`'s status CAS; the loser's error
  is expected and logged at `debug`.
- **D7 — `batch_id` reaches the run through `NewRun`.** `NewRun` gains `batch_id: Option<BatchId>`
  (all 27 sites get `batch_id: None`); `Engine` gains an `enqueue_in_batch(item, batch)` (or an
  equivalent parameter on a private helper) so `enqueue`'s public signature and its callers do not
  change. `RunRow` is unchanged in M1.
- **D8 — Admission latency on a TUI box.** The TUI sweep runs every lease period (120 s default).
  M1 also wakes the sweep when a walk ends and when the queue is resumed, so the next item is
  admitted within one sweep of a slot freeing, not up to two minutes later. Headless cadence (5 s)
  is unchanged.
- **D9 — Keys (Backlog left pane).** `Q` toggles the cursor item's queue membership (queue on the
  local box / dequeue); `P` toggles this box's queue (resume = open batch, pause = close it). Each
  answers with a status line (`queued MOD-7 (3 in queue, paused)`, `queue resumed`, `queue paused —
  2 runs still running`). Both get help rows. No list marker and no top-bar indicator in M1 — M3's
  overlay owns the queue view (avoids a snapshot sweep now and again in M3). Dequeue never cancels a
  run already created.
- **D10 — Gate downgrade at snapshot time only.** `snapshot_phase` takes `mode`;
  `gate_effective = if mode == Auto && !gate_hard { Never } else { gate }` (ANA-2 §4.10, criteria
  23–24). The `resolve` doc sentence saying `mode` does not move `gate_effective` is replaced.
- **D11 — Not in M1:** batch-cap accounting, Settings sections (M2); overlay, reorder, escalation
  list, off-box run reporting (M3). Memory/demo builds keep `without_claim_scan()`, so they never run
  the queue; `Q`/`P` there answer with the same "needs the server" refusal as other orchestration.

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-store/migrations/0016_auto_queue.sql` | CREATE | T0 |
| `crates/htui-store/tests/migrations.rs` | UPDATE (count 15→16, new-table assertions) | T0 |
| `crates/htui-store/tests/connect.rs` | UPDATE (`Pending(16)`) | T0 |
| `crates/htui-store/src/pg/read.rs` | UPDATE (`ready_items` order) | T1, T3 |
| `crates/htui-core/src/store/mem.rs` | UPDATE (`ready_items` order; queue state + methods) | T1, T3 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE (order assertions; queue parity) | T1, T3 |
| `crates/htui-store/.sqlx/*` | CREATE/DELETE (prepared queries) | T1, T3 |
| `crates/htui-orch/src/graph.rs` | UPDATE (downgrade + tests) | T2 |
| `crates/htui-orch/src/conformance.rs` | UPDATE (auto-mode cases, count) | T2 |
| `crates/htui-core/src/model/queue.rs` | CREATE (`BatchId`, `QueueEntry`, `QueueBatch`, `BatchClose`) | T3 |
| `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/model/ids.rs` (`BatchId`) | UPDATE | T3 |
| `crates/htui-core/src/model/run.rs` | UPDATE (`NewRun.batch_id`) | T3 |
| `crates/htui-core/src/store/worker.rs` | UPDATE (`WorkerHost` queue methods) | T3 |
| `crates/htui-store/src/pg/write.rs` | UPDATE (`create_run` writes `batch_id`; queue writes) | T3 |
| `crates/htui-store/src/backend.rs`, `crates/htui-store/src/worker.rs` | UPDATE (dispatch, `WorkerHost` impls) | T3 |
| 12 other `NewRun` sites (`htui-agent/tests/relay.rs`, `htui-mcp/tests/tools_backlog.rs`, `htui-core/src/store/conformance.rs`, `htui-orch/tests/review_loop.rs`, `htui-store/tests/cache.rs`, `htui/tests/waiting.rs`, `htui/src/run_worker.rs`, `htui/tests/backlog.rs`, `htui-worker/src/runtime.rs`, `htui-orch/src/engine.rs`, `htui-core/src/model/run.rs`, `mem.rs`) | UPDATE (`batch_id: None`) | T3 |
| `crates/htui-orch/src/engine.rs` | UPDATE (`enqueue_in_batch`) | T4 |
| `crates/htui-worker/src/runtime.rs` | UPDATE (admission step, drain, wake-ups) | T4 |
| `crates/htui-worker/tests/*` (new `auto_queue.rs`) | CREATE | T4 |
| `crates/htui/src/store_worker.rs` | UPDATE (`QueueItem`, `DequeueItem`, `ResumeQueue`, `PauseQueue`) | T5 |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE (`Q`, `P`) | T5 |
| `crates/htui/src/app/mod.rs` | UPDATE (help rows) | T5 |
| `crates/htui/tests/backlog.rs` | UPDATE (key + help-line tests) | T5 (after T3's `batch_id: None` edit) |
| `docs/ANA-2.md` (§4.10 forward note), PRD milestone row, HANDOFF phase note | UPDATE | T6 |

**Intersections that decide the waves:** T1 ∩ T3 = {`pg/read.rs`, `mem.rs`, `pg_criteria.rs`, `.sqlx`}
→ serial. T3 ∩ T4 = {`engine.rs`, `runtime.rs`} (T3 only adds `batch_id: None`) → serial. T3 ∩ T5 =
{`tests/backlog.rs`, `run_worker.rs` via `NewRun`} → serial. T4 ∩ T5 = ∅. T0, T1, T2 pairwise ∅.

## Tasks

### Wave 1 (parallel: T0 ∥ T1 ∥ T2)

#### T0: Migration `0016_auto_queue.sql` (TDD)
- **Action**: Tests first in `migrations.rs`: version list ends at 16; the three objects exist; the
  partial unique index refuses a second open batch on one box; `closed_at`/`closed_reason` pairing
  check; `queue_entry` cascades on item delete. Then the SQL per D1. Bump `Pending(15)` →
  `Pending(16)` and `MigrationsPending(15)` → `(16)` everywhere listed in Grounding.
- **Mirror**: `0015_command_queue.sql`; the existing migration tests.
- **Validate**: `cargo test -p htui-store --all-features --test migrations --test connect -- --test-threads=1` against `HTUI_TEST_DATABASE_URL`.

#### T1: `ready_items` in queue order (TDD)
- **Action**: Red test in `pg_criteria.rs` and the Mem test: two ready items in one project, the
  lower key with lower priority, plus a priority tie broken by `created_at` → order is
  `priority DESC, created_at, id` in both stores. Then change the Pg `ORDER BY` and the Mem sort;
  rewrite the doc comment; regenerate the `.sqlx` entry against a migrated scratch DB.
- **Mirror**: `inherent_orchestration_reads_answer_the_fixture`.
- **Validate**: `cargo test -p htui-store --all-features --test pg_criteria ready -- --test-threads=1`; `cargo test -p htui-core --all-features ready_items`; `cargo test -p htui --all-features ready_here`.

#### T2: Auto-mode gate downgrade (TDD)
- **Action**: Red tests: unit in `graph.rs` (Auto + `gate = always`, `gate_hard = false` → `Never`;
  Auto + `gate_hard` → unchanged; Manual → unchanged), and conformance cases
  `auto_mode_skips_soft_gates` (steps `done`, `gate_outcome = skipped`) and
  `auto_mode_parks_at_a_hard_gate` (criterion 23), `a_manual_snapshot_keeps_its_gates` (criterion
  24). Then thread `mode` into `snapshot_phase` and apply D10. Bump the `CASES` count.
- **Mirror**: `field_chains_walk_phase_project_app`; `judges_never_get_command_run` case layout.
- **Validate**: `cargo test -p htui-orch --all-features -- --no-fail-fast` (grep the log for `SIGABRT` — stack headroom).

### Wave 2 (serial)

#### T3: Queue store surface (TDD)
- **Action**: Model types in `model/queue.rs`. Inherent methods on `MemStore` and `PgStore`,
  dispatched by `Backend` (offline → `orchestration_offline()`), and exposed on `WorkerHost`:
  `queue_item(item, box, by, at)`, `dequeue_item(item)`, `queue_entries(box)`,
  `open_batch(box, by, at) -> QueueBatch` (idempotent: returns the open one), `close_batch(box, reason, at)`,
  `open_batch_of(box)`, `prune_finished_entries(box)`, `batch_has_live_runs(batch)`, and
  `ready_items` on `WorkerHost`. `NewRun.batch_id` written by both `create_run`s. Tests first: Mem
  unit tests per method; Pg/Mem parity for every method in `pg_criteria.rs` (one-open-batch refusal
  surfaces as a typed error, not a panic); `create_run` with a batch round-trips `run.batch_id`.
- **Mirror**: `queued_runs_on_box` end to end; `SetAgentOnBox` for the write shape.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features -- --test-threads=1`; `cargo sqlx prepare` check; `cargo build --workspace --all-features`.

### Wave 3 (parallel: T4 ∥ T5)

#### T4: The runner (TDD)
- **Action**: Red tests in a new `crates/htui-worker/tests/auto_queue.rs` over `Backend::Memory`
  with the claim scan on and a fake driver: (a) paused box admits nothing; (b) resumed box admits
  queued ready items in D4 order up to `max_concurrent_items`, all `RunMode::Auto` with the batch id;
  (c) a queued-but-blocked item is skipped and admitted after unblock; (d) a missing-tags item is
  blocked with a note and the next item admitted; (e) pause stops admission and leaves running runs
  running (criterion 27, second half); (f) drain closes the batch `drained`; (g) a manual run and an
  auto run that overlap serialise in either order (criterion 26); (h) two runtimes on one store
  admit each item once. Then `enqueue_in_batch` in `engine.rs`, the admission + prune/drain step in
  `sweep_once` (D6), and the D8 wake-ups. A Pg variant of (h) and (g) under `testkit`.
- **Mirror**: `start_run`'s `Kit::read` → engine use; existing runtime tests' harness.
- **Validate**: `cargo test -p htui-worker --all-features -- --test-threads=1`; `cargo test -p htui-orch --all-features -- --no-fail-fast`.

#### T5: Backlog `Q` / `P` (TDD)
- **Action**: Red tests in `tests/backlog.rs`: `Q` on an item sends `QueueItem` and a second `Q`
  sends `DequeueItem`; `P` sends `ResumeQueue` / `PauseQueue` by the current state; the status lines
  of D9; offline refusal; both on the Backlog help line. Then the `StoreRequest` variants (+ `name()`
  arms) and serve arms in `store_worker.rs`, the two key arms, and the help rows.
- **Mirror**: `SetAgentOnBox`; `m_is_on_the_backlog_help_line`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features` shows no snapshot change.

### Wave 4

#### T6: Documents and gates
- **Action**: Forward note in `docs/ANA-2.md` §4.10 (queue membership and batch row, PRD D1/D2);
  PRD milestone row; HANDOFF phase note (P1). Full gate run below; prototype batch per PRD.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless: catches test-support-only code
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod12-m1.log
grep -n -E 'SIGABRT|test result: FAILED' /tmp/mod12-m1.log
cargo doc --workspace --no-deps --all-features
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `0016` collides with a sibling run's migration on merge | Medium | Re-check `/host/htui` migrations before T0 commits and again at collect; renumber is a rename + count bump |
| Runner admits on a box whose executor is another process | Low | Admission sits after the existing `role.executes` early return; test with a `worker` executor + TUI role |
| Two runners double-admit one item | Low | `create_run`'s status CAS; test (h) over Mem and Pg |
| Overlap-refused auto runs pile up as `queued` | Medium | Free-slot count includes queued runs on the box (D6) |
| `htui-orch` conformance stack overflow from new cases | Medium | Box large futures; `--no-fail-fast` + `SIGABRT` grep |
| `NewRun` field addition misses a site behind a feature | Low | `cargo build --workspace --all-features --all-targets` and featureless clippy |
| Prepared-query cache drift | Medium | Prepare against a migrated scratch DB, not the empty compose DB |

## Acceptance

- [ ] ANA-2 criteria 22, 23, 24, 26 and 27 (pause half) each have a named passing test
- [ ] A queued, ready item on a resumed box becomes an `auto` run with the batch id and walks to the
      end with soft gates `skipped`; a hard gate parks
- [ ] Pause stops admission and closes the batch; running runs continue
- [ ] Validation passes; no snapshot changes
- [ ] Patterns mirrored, not reinvented

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `ready_items` is inherent on Pg/Mem/Backend, not on a trait | TRUE | grep `fn ready_items` → `pg/read.rs:2070`, `mem.rs:758`, `backend.rs:540` only |
| `ready_items` orders by display order | TRUE | `pg/read.rs` `ORDER BY array_position($1, i.project_id), i.key_prefix, i.key_number` |
| `ItemSummary` has no `created_at`; `Item` does | TRUE | `item.rs:183-210`; `item.rs:150` |
| `create_run` moves item to `queued` under a status CAS | TRUE | `pg/write.rs:4135` `legal_move(status, Status::Queued)`, `:4214` `UPDATE item SET status = 'queued' … AND status = $2` |
| `snapshot_phase` does not receive `mode`; one call site | TRUE | `graph.rs:339` only call; signature `:683` |
| Readers honour `gate_effective`, `Never` → `Skipped` | TRUE | `gate.rs:394`; `mem.rs:4975` |
| Shared `RunRuntime` for TUI and headless worker | TRUE | `run_worker.rs:39`, `worker.rs:52` |
| Admission point can reuse `role.executes` guard | TRUE | `sweep_once` early return, `runtime.rs` (read) |
| `start_run` builds engine via `Kit::read` then `enqueue` | TRUE | `runtime.rs:2286` (read) |
| `WorkerHost` has exactly two implementors | TRUE | grep `impl … WorkerHost for` → `htui-store/src/worker.rs:707,787` |
| `NewRun` has no `Default`; 27 sites / 13 files | TRUE | `run.rs:349` derives `Debug, Clone, PartialEq, Serialize, Deserialize`; grep count |
| Next migration number is `0016` (here and host) | TRUE | `ls` both migration dirs end at `0015_command_queue.sql` |
| Migration count literals at the listed lines | TRUE | grep `\b15\b` in `migrations.rs`, `connect.rs` |
| `Q`/`P` unbound on Backlog left pane, Runs pane and globally | TRUE | grep `KeyCode::Char('[QPqp]')` → only `q`/`p` hits; Runs key table `runs.rs:17-31` |
| Backlog help line asserted by test, not snapshot | TRUE (from grounding agent) | `tests/backlog.rs:267`; status bar shows global help only (`app/state.rs:754`) |
| TUI sweep cadence is the lease period (120 s default) | TRUE (from grounding agent) | `runtime.rs:1077`, `:1919-1925` |
| Queue tables unmirrored → offline refusal is consistent | TRUE | mirror tables in `cache_migrations/0001_mirror.sql` include `run`, no queue tables (new) |
| Task independence: T0/T1/T2 pairwise disjoint; T4 ∩ T5 = ∅ | TRUE | "Files to Change" intersections above |
| Task independence: T1/T3, T3/T4, T3/T5 intersect | TRUE → serial | same table |
