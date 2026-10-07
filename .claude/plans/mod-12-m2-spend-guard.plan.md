# Plan: MOD-12 M2 — Spend guard

**Source PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md`
**Selected Milestone**: 2 — Spend guard
**Complexity**: Medium–Large
**Status**: confirmed (maintainer, 2026-10-07) — implementation in progress
**Routing**: plan path (milestone of an existing PRD), ultracode not recommended (`/handoff-run`
verdict 2026-10-07, accepted). Waves below are decided by the file sets in "Files to Change".

## Summary

Each queue batch (M1's `queue_batch`) gets a spend figure, `SUM(run_step.usage->>'cost_micros')` over
its runs, computed when needed and never stored (ANA-2 §4.10). The guard applies at **three points**:
1. **The runner** admits no further item once the batch has hit the item's project cap, or what is
   left is below `min_budget_for_new_attempt`.
2. **The engine's candidate walk** refuses a new attempt inside a batch run on the same rule.
3. **The recorder** cancels a session once it has spent what the batch had left when the attempt
   started (R-AGT-7: "enforced by cancelling the session when exceeded").

A new `Settings > Queue` section edits the two project caps, `min_budget_for_new_attempt`, the
app-wide and per-box `max_concurrent_items`, and the stored-only `scheduler_window`.

## Grounding (read on `hr/MOD-12` at `da52b1db`; see Verified claims)

- **Batch identity exists; no migration.** `0016_auto_queue.sql` added `queue_batch` and
  `run.batch_id`. `NewRun.batch_id` (`crates/htui-core/src/model/run.rs:405`) is written by
  `Engine::enqueue_in_batch` (`engine.rs:699-757`). The read model `Run` (`run.rs:182`) has **no**
  `batch_id`, and the SQLite mirror's `run` table does not carry it. Migrations end at `0017` here and
  on `/host/htui`.
- **Cap keys.** `per_token_cap_run` and `per_token_cap_batch` are `project.settings` keys in USD
  micros (`model/quota.rs:276-281`, `ProjectCaps::from_settings` `:322-345`). Absent or `null` means
  unbounded, an integer ≥ 0 is the cap (`0` is a real cap), and anything else is a `CapError`. The
  snapshot freezes both (`graph.rs:359-360`, `SnapshotSettings`, `run.rs:634-639`). The
  `app_setting.per_token_cap_*` rows seeded `null` by `0003` (`:137-138`) have **no reader**.
  `min_budget_for_new_attempt` is an unseeded `app_setting` key. `min_budget()` (`engine.rs:7147`)
  reads it as a positive `i64`, else `0`.
- **Run-level guard today.**
  - `select::walk` rule 2 (`quota::available` → `CapReached`) and rule 5 (`Budget`), at
    `select.rs:160-194`, take `spent_micros = run_spend(run's steps)`,
    `cap_micros = snapshot.per_token_cap_run` and `min_budget`. Either figure unknown means unbounded
    (OQ-6).
  - The engine builds this input in `walk_candidates` (`engine.rs:3373-3412`). Every attempt and the
    judge's walk pass through it.
- **Recorder cap.**
  - `open_recorder` (`engine.rs:6150-6170`) passes `RunCap { micros: per_token_cap_run }`, and the
    driver spec gets `budget_micros: per_token_cap_run` (`:6320`).
  - `Recorder::check_cap` (`htui-agent/src/record.rs:1636-1650`) compares the recorder's **own**
    `usage.cost_micros`, which starts at zero per `Recorder::new` (`:529-566`). So the "run" cap is
    applied **per step session**: a run can spend up to about 2× its cap, because the attempt
    started at cap − 1 gets a full cap of its own.
  - A breach settles the step `StepFailure::CapBreached` (`gate.rs:122`, `:256`). Judges also report
    `CapBreached` (`engine.rs:5274`).
- **Runner.** `admit()` (`crates/htui-worker/src/runtime.rs:2115-2242`):
  - reads the open batch, prunes, reads the entries, `ready_items`, the batch's cancelled items, then
    `admission_order`;
  - computes free slots with `admission_limit` and `free_slots` (`model/queue.rs:86-96`);
  - enqueues up to `free` items through `engine.enqueue_in_batch`.

  `WorkerHost` (`store/worker.rs:426+`) has no project read and no batch-spend read. Its
  implementors: `PgStore`, `Backend` (`htui-store/src/worker.rs:784`, `:890`), and the test wrapper
  `Failing` (`runtime.rs:3725`).
- **Engine store.** `EngineParts.store: &S where S: WorkerStore` (`engine.rs:439-450`).
  `WorkerStore` implementors: `MemStore` (`store/worker.rs:659`), `PgStore`
  (`htui-store/src/worker.rs:116`), `Writer` (`:491`).
- **Settings.**
  - Sections implement `SettingsSection` (`crates/htui/src/ui/tabs/settings/mod.rs:134-169`) and are
    registered in `app/mod.rs:66-79`; the newest is Secrets (MOD-10, commit `7188c47d`).
  - The prompt registry `SettingKey` (`htui-core/src/prompt/settings.rs:157`, `ALL: [_; 11]`) cannot
    take these keys. `app_keys()` feeds `Defaults::as_rows` and the `0002` ten-row pin, and its
    integer kind has no "nullable".
  - `box.settings` has one writer, `WriteStore::edit_box` (`traits.rs:545-572`, CAS on
    `edit_version`), which merges `executor` only (`BoxEdit`, `model/box_.rs:163-171`).
  - There is no `StoreRequest` for an arbitrary `app_setting` key.
  - `WriteStore` implementors: `MemStore` (`mem.rs:7856`), `PgStore` (`pg/write.rs:1008`), `Writer`
    (`writer.rs:329`), `UsageSpy` (`htui-agent/src/conformance.rs:751`) and `SpyStore`
    (`htui-agent/tests/recorder.rs:438`).
- **`scheduler_window`** is seeded `null` (`0003:139`), typed "object, nullable, reserved unused in
  v1" (`docs/ANA-2.md:1634`). No inner shape is defined anywhere.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Inherent queue read → runtime | `batch_cancelled_items` (M1: Mem, Pg, `Backend`, `WorkerHost` + 2 impls + `Failing`) | Unmirrored → inherent on Pg/Mem, `Backend` offline → `orchestration_offline()`, exposed on `WorkerHost` |
| Pure admission arithmetic | `admission_limit` / `free_slots` (`model/queue.rs:86-120`, unit tests `:170+`) | A pure function in `model/queue.rs` with table tests; the runtime only feeds it |
| Walk skip rule | `SkipCause::Budget` + `min_budget_skips_only_when_cap_and_spend_are_known` (`select.rs:52`, `:609`) | A new `SkipCause` variant with `Display`, a unit test per rule, and "unknown is unbounded" |
| Cap parse | `ProjectCaps::from_settings` (`quota.rs:322`) | One parser; a `CapError` sentence surfaced verbatim |
| Settings section | Secrets (`7188c47d`: `settings/secrets.rs`, `settings/mod.rs`, `app/mod.rs:79`, `tests/secrets_settings.rs`, `tests/settings.rs` strip test, 6 snapshots) and its serve module (`9e008387`: `crates/htui/src/secrets_settings.rs`) | Section file + serve module + `StoreRequest` variants + registry line + `SectionBench` tests + insta |
| CAS write | `edit_box` (`traits.rs:545`, conformance `edit_box_*` `store/conformance.rs:9622-9928`) | Compare-and-set, foreign keys preserved, conformance case per rule |
| Pg/Mem parity | `inherent_orchestration_reads_answer_the_fixture` (`htui-store/tests/pg_criteria.rs`) | Same fixture through both stores, equal answers |
| Orchestrator conformance | `conformance.rs` `CASES` + `cases_are_unique_and_counted` | New case names, count bumped |

## Decisions (proposed; CONFIRM accepts or overrides)

- **D1 — Batch spend.** `batch_spend(batch) -> Option<i64>` is
  `SUM((usage->>'cost_micros')::bigint)` over the `run_step` rows of the runs with
  `run.batch_id = batch`. It is `None` when no step reports a cost. Pg and Mem only; `Backend`
  offline refuses, like every queue read. Nothing is stored (ANA-2 §4.10).
- **D2 — Which cap.** A batch spans projects, so a run is guarded by **its own project's**
  `per_token_cap_batch` against **the whole batch's** spend. The runner uses the item's
  `project.settings` value at admission, and the engine and recorder use the run's
  `snapshot.settings.per_token_cap_batch`, which is already frozen. There is no `app_setting`
  fallback: the seeded `app_setting.per_token_cap_*` rows stay unread, as today. *Alternative not
  taken:* a single box-wide or app-wide batch cap. That would need a reader for the dormant
  `app_setting` row, and two caps would apply to a run.
- **D3 — One admission rule, three callers.** A pure function in `model/queue.rs`,
  `batch_budget(spent: Option<i64>, cap: Option<i64>, min: i64) -> Result<Option<i64>, BatchStop>`,
  mirrors rule 2 and rule 5 exactly:
  - cap unknown or spend unknown → unbounded (`Ok(None)`), as OQ-6 has it;
  - `spent >= cap` → `BatchStop::CapReached { spent, cap }`;
  - `cap - spent < min` → `BatchStop::Budget { remaining, min }`;
  - otherwise `Ok(Some(remaining))`.

  `Display` names the batch: `batch cap reached (… of … micros)`,
  `batch budget: … micros left, … required`.
- **D4 — Runner (point 1).** In `admit()`, after `free_slots` and before the enqueue loop:
  - read `batch_spend(batch.id)` once per sweep;
  - for each ordered item, read its project's caps and apply D3;
  - an item stopped by D3 is skipped and the loop goes on to the next item, because another
    project's cap may still admit;
  - a malformed cap (`CapError`) **fails closed**: the item is skipped and the error logged at
    `warn` once per batch.

  When every remaining entry is stopped, the batch stays **open** and admits nothing. There is no
  new `closed_reason` (it would need a migration). A pause and a resume opens a fresh batch, the
  accepted consequence from the PRD. The first stop per batch is logged at `info` (`the queue's
  batch reached its cap`); M3's overlay surfaces it. The project read is one new `WorkerHost`
  method, `project_settings(project) -> Option<Value>` (or `ProjectCaps`), with the `batch_spend`
  read beside it.
- **D5 — Engine walk (point 2).** `SelectInput` gains the batch figures (`batch_spent_micros`,
  `batch_cap_micros`) and `walk` gains two rules after rule 5: `SkipCause::BatchCapReached` and
  `SkipCause::BatchBudget`, both from D3. `walk_candidates` fills them in when the run belongs to a
  batch. The engine learns that through one new `WorkerStore` read,
  `run_batch_spend(run) -> Option<(BatchId, Option<i64>)>`, which is `None` for a manual or chat
  run; it is `WorkerStore` because `Run` has no `batch_id` and the mirror is untouched. A walk with
  no eligible candidate takes the existing `refuse_no_candidate` path (the item goes `blocked`, with
  a note naming the batch cause), so it becomes an escalation for M3 with no new state.
- **D6 — Session allowance (point 3).** The `RunCap` and `budget_micros` handed to a session become
  `min(run_cap − run_spent, batch_cap − batch_spent)` over the figures that are known, floored at
  `0`. With no figure known, no cap is set, as today.
  - This fixes the per-step reading of `per_token_cap_run` noted in Grounding, so a run can no longer
    spend about 2× its cap. It sits on the same line of code and is called out so the maintainer can
    strike it (strike → run part stays `per_token_cap_run` as is).
  - A breach whose binding term was the batch settles `CapBreached` with the failure text naming the
    batch (`cap breached (batch …)`), so M3 can tell the two apart.
- **D7 — Overshoot bound.** Overshoot is at most the spend of the attempts already in flight when the
  batch reaches its cap. That is one per running run of the batch, so at most
  `max_concurrent_items` attempts, and each is cut off by D6 at what the batch had left when it
  started. The PRD metric "≤ one attempt above `per_token_cap_batch`" holds for
  `max_concurrent_items = 1` and is amended to "one in-flight attempt per running run of the batch".
  *Alternative not taken:* the recorder re-reading the batch sum on every usage row. That costs a
  `SUM` query per row, and the recorder has no batch handle.
- **D8 — `Settings > Queue` section.** A new section, registered last. Rows, by scope:
  - **this project** (follows the section's scope): `per_token_cap_run`, `per_token_cap_batch`;
  - **all boxes** (`app_setting`): `min_budget_for_new_attempt`, `max_concurrent_items` (default),
    `scheduler_window`;
  - **this box** (`box.settings`): `max_concurrent_items` (empty = inherit the app default; the
    effective value is shown, resolved by `admission_limit`).

  Money is typed and shown in USD (`1.50` → stored `1500000` micros, up to six decimals), never as
  raw micros. Empty clears a key, which means unbounded for a cap and inherit for the box limit.
  Offline, writes get the same "needs the server" refusal as other orchestration. The section text
  says the window is **stored, not enforced** (R-ORCH-13 is `later`).
- **D9 — Write surface.** One typed key enum `QueueSetting` in `htui-core`, with
  `{PerTokenCapRun, PerTokenCapBatch}` on project,
  `{MinBudgetForNewAttempt, MaxConcurrentItems, SchedulerWindow}` on app, and
  `{MaxConcurrentItems}` on box. Each has its own validator: `ProjectCaps`' rule for caps, positive
  for `min_budget`, ≥ 1 for concurrency, and D10 for the window. All writes go through two
  `WriteStore` methods, `set_queue_setting` and `clear_queue_setting`, with a
  `QueueTarget::{App, Project(id), Box(id)}`:
  - a project key merges into `project.settings` under CAS on `updated_at`, exactly the
    project-rung path of `set_setting`;
  - an app key upserts its `app_setting` row under the same `expected` rule;
  - a box key merges into `box.settings` under CAS on `edit_version` (bumping it, as `edit_box`
    does) and keeps every other key.

  **`BoxEdit` is not changed** (fact-check: about 35 construction sites in more than 20 files,
  about 23 of them spelling every field out, including T2's and T3's files). The prompt registry is
  **not** extended. `StoreRequest::SetQueueSetting` and `StoreRequest::ClearQueueSetting` are
  served from a new `crates/htui/src/queue_settings.rs`.
- **D10 — `scheduler_window` shape.** `null`, or `{"start":"HH:MM","end":"HH:MM"}` in the box's
  local time, with `end < start` meaning the window crosses midnight. The editor takes it typed as
  `22:00-06:00`. This is the minimum that R-ORCH-13 ("run the queue inside a time window on a chosen
  box") needs. It is stored under `app_setting` as ANA-2 §5.4 reserves; moving it per box is
  R-ORCH-13's call. Nothing reads it.
- **D11 — Not in M2:**
  - the overlay, the escalation list and the batch-cap state display (M3);
  - enforcing the window;
  - an `app_setting` fallback for the project caps;
  - repairing `Engine::project_settings`' `unwrap_or_default()`, which drops **every** project
    setting when one is malformed. It is pre-existing; D4 fails closed on its own read, and this is
    flagged for a CLEAN item at close-out.

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/queue.rs` | UPDATE (`batch_budget`, `BatchStop` + tests) | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE (`batch_spend`, `run_batch_spend`; T4: queue-setting writes, `edit_box` merge) | T1, T4 |
| `crates/htui-core/src/store/worker.rs` | UPDATE (`WorkerHost::batch_spend`, `::project_settings`; `WorkerStore::run_batch_spend` + Mem impl) | T1 |
| `crates/htui-store/src/pg/read.rs` | UPDATE (Pg `batch_spend`, `run_batch_spend`, project settings read) | T1 |
| `crates/htui-store/src/worker.rs` | UPDATE (`WorkerHost` for Pg/`Backend`, `WorkerStore` for Pg/`Writer`) | T1 |
| `crates/htui-store/src/backend.rs` | UPDATE (dispatch, offline refusal) | T1 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE (parity: batch spend, run batch spend) | T1 |
| `crates/htui-store/.sqlx/*` | CREATE (prepared queries) | T1, T4 |
| `crates/htui-worker/src/runtime.rs` (`Failing` wrapper) | UPDATE (new `WorkerHost` methods) | T1 |
| `crates/htui-orch/src/select.rs` | UPDATE (batch inputs, two rules, tests) | T2 |
| `crates/htui-orch/src/engine.rs` | UPDATE (`walk_candidates` batch figures, D6 allowance in `open_recorder` + spec, failure text) | T2 |
| `crates/htui-orch/src/conformance.rs` | UPDATE (batch cases, count) | T2 |
| `crates/htui-worker/src/runtime.rs` (`admit`) | UPDATE (D4 gate, once-per-batch logs) | T3 |
| `crates/htui-worker/tests/auto_queue.rs` | UPDATE (batch-cap admission tests) | T3 |
| `crates/htui-core/src/model/queue_settings.rs` (or a section of `queue.rs`, architect's call) | CREATE (`QueueSetting`, validators, USD parse/format, window parse) | T4 |
| `crates/htui-core/src/model/mod.rs` | UPDATE (export) | T4 |
| `crates/htui-core/src/store/traits.rs` | UPDATE (`set_queue_setting`, `clear_queue_setting`, `QueueTarget`) | T4 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE (queue-setting CAS cases per target) | T4 |
| `crates/htui-store/src/pg/write.rs` | UPDATE (Pg writes) | T4 |
| `crates/htui-store/src/writer.rs` | UPDATE (delegate) | T4 |
| `crates/htui-agent/src/conformance.rs` (`UsageSpy`), `crates/htui-agent/tests/recorder.rs` (`SpyStore`) | UPDATE (delegate / unimplemented) | T4 |
| `crates/htui/src/store_worker.rs` | UPDATE (`SetQueueSetting`, `ClearQueueSetting`, `QueueSettings` read; `name()` + routes) | T5 |
| `crates/htui/src/queue_settings.rs` | CREATE (serve module) | T5 |
| `crates/htui/src/lib.rs` (module line) | UPDATE | T5 |
| `crates/htui/src/ui/tabs/settings/queue.rs` | CREATE (section) | T5 |
| `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/app/mod.rs` | UPDATE (module, register) | T5 |
| `crates/htui/tests/queue_settings.rs` + `tests/snapshots/queue_settings__*.snap` | CREATE | T5 |
| `crates/htui/tests/settings.rs` (strip test) + affected strip snapshots | UPDATE | T5 |
| `docs/ANA-2.md` (§4.10 / §5.4 as-built notes), PRD milestone row + metric amendment (D7), HANDOFF phase note | UPDATE | T6 |

**Intersections that decide the waves:**
- T1 ∩ T4 = {`mem.rs`, `.sqlx`} → serial: T4 after T1.
- T1 ∩ T3 = {`runtime.rs`} (T1 only touches `Failing`) → serial.
- T1 ∩ T2 = ∅, but T2 needs T1's `WorkerStore::run_batch_spend` and `batch_budget` → after T1.
- T2 ∩ T3 = ∅, T2 ∩ T4 = ∅, T3 ∩ T4 = ∅ → **T2 ∥ T3 ∥ T4**.
- T5 ∩ T4 = ∅, but T5 needs T4's API → after T4.
- T5 ∩ T2 = T5 ∩ T3 = ∅.

## Tasks

### Wave 1

#### T1: Batch figures and the admission rule (TDD)
- **Action**: Tests first:
  - `batch_budget` table tests (unknown/unbounded, reached at equality, budget short, exactly the
    minimum is enough);
  - Mem unit tests for `batch_spend` (no costed rows → `None`; sums only the batch's runs; manual
    runs excluded) and `run_batch_spend` (manual run → `None`);
  - Pg/Mem parity in `pg_criteria.rs`.

  Then the pure function, the Mem and Pg reads, `Backend` dispatch with offline refusal, the
  `WorkerHost` and `WorkerStore` additions with every implementor (`Failing` included), and
  `.sqlx` against a migrated scratch DB.
- **Mirror**: `batch_cancelled_items` end to end; `admission_limit` tests.
- **Validate**: `cargo test -p htui-core --all-features queue`; `cargo test -p htui-store --all-features --test pg_criteria -- --test-threads=1`; `cargo build --workspace --all-features --all-targets`.

### Wave 2 (parallel: T2 ∥ T3 ∥ T4)

#### T2: Engine walk and session allowance (TDD)
- **Action**: Red tests:
  - `select.rs` units for `BatchCapReached` and `BatchBudget`, plus "unknown is unbounded";
  - conformance cases:
    - `a_batch_at_its_cap_starts_no_attempt`: the item goes `blocked` with a note naming the batch;
    - `a_batch_run_session_is_capped_at_the_batch_remainder`: the recorder breaches at the
      remainder, the step settles `CapBreached` and the text names the batch;
    - `a_manual_run_ignores_batch_figures`;
    - `the_run_cap_spans_steps` (D6's fix: a second step's allowance is cap − first step's spend).

  Then the code, with the `CASES` count bumped.
- **Mirror**: `min_budget_skips_only_when_cap_and_spend_are_known`; the existing `per_token_cap_run` engine tests (`engine.rs:8198-8260`).
- **Validate**: `cargo test -p htui-orch --all-features -- --no-fail-fast 2>&1 | tee /tmp/m2-orch.log; grep -n -E 'SIGABRT|FAILED' /tmp/m2-orch.log`.

#### T3: Runner spend gate (TDD)
- **Action**: Red tests in `auto_queue.rs` over `Backend::Memory`:
  - (a) a batch whose spend reached project A's cap admits no more A items but still admits a
    project-B item with no cap;
  - (b) remaining below `min_budget_for_new_attempt` admits nothing;
  - (c) pause and resume opens a fresh batch that admits again;
  - (d) a malformed cap skips that item (fail closed) and admits the next;
  - (e) the batch stays open while stopped and is not drained;
  - (f) a manual run's spend on the box does not count against the batch.

  Then the D4 gate in `admit()`.
- **Mirror**: M1's `auto_queue.rs` harness.
- **Validate**: `cargo test -p htui-worker --all-features -- --test-threads=1`.

#### T4: Queue-setting write surface (TDD)
- **Action**: Red tests:
  - `QueueSetting` validators: caps ≥ 0 or clear; `min_budget` > 0; concurrency ≥ 1; window
    `HH:MM-HH:MM`; USD parse/format round-trip (`1.5` ↔ `1500000`, more than six decimals refused);
  - conformance:
    - a project key merges under CAS and keeps foreign keys;
    - an app key upserts, and a stale `expected` is refused;
    - clear removes the key;
    - a box key merges into `box.settings` under CAS on `edit_version`, bumps it, and keeps
      `executor` and the other keys.

  Then the types, and the trait methods on all five `WriteStore` implementors.
- **Mirror**: `set_setting` project/app rungs; `edit_box_writes_the_executor_and_keeps_every_other_setting` for the box merge.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features -- --test-threads=1`; `cargo build --workspace --all-features --all-targets`.

### Wave 3

#### T5: `Settings > Queue` section (TDD)
- **Action**: Red tests in `tests/queue_settings.rs` (`SectionBench`):
  - renders the three groups with effective values;
  - USD entry stores micros;
  - empty clears;
  - an invalid value shows the validator's sentence and writes nothing;
  - box limit empty shows the inherited value;
  - the window row says "stored, not enforced";
  - offline refusal;
  - a CAS conflict shows `CHANGED_ELSEWHERE`;
  - snapshots.

  Serve-level tests drive a `Backend`. Then the requests, the serve module, the section, the
  registration, and the strip test update.
- **Mirror**: Secrets section (`7188c47d`) and serve module (`9e008387`).
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features` (expected changes: new `queue_settings__*` and the section strip only).

### Wave 4

#### T6: Documents and gates
- **Action**:
  - `docs/ANA-2.md`: as-built notes in §4.10 (three enforcement points, D2 and D7) and §5.4 (caps
    live in `project.settings`; the `app_setting` rows are unread; the window shape per D10);
  - PRD M2 row and the D7 metric amendment;
  - HANDOFF P1 phase note;
  - file the CLEAN item from D11.

  Full validation below.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless: catches test-support-only code
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod12-m2.log
grep -n -E 'SIGABRT|test result: FAILED' /tmp/mod12-m2.log
cargo doc --workspace --no-deps --all-features
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Overshoot beyond one attempt with concurrency > 1 | Certain (by design) | D7 bound, stated and tested; PRD metric amended |
| Agents that never report a USD cost are never batch-capped | Medium | "Unknown is unbounded" is the shipped OQ-6 rule; documented in the section help and ANA-2 note |
| D6 changes chat or graph runs that relied on the per-step reading | Low | Conformance `the_run_cap_spans_steps`; D6's run half can be struck at CONFIRM |
| `htui-orch` conformance stack headroom with new cases | Medium | Box large futures; `--no-fail-fast` + `SIGABRT` grep |
| `.sqlx` drift between T1 and T4 | Medium | Prepare against a migrated scratch DB after each task; T4 after T1 |
| Settings strip snapshot churn | Medium | Full `cargo insta test`; accept only the strip and the new snapshots |
| `WriteStore` addition misses an implementor behind a feature | Low | `--all-features --all-targets` build and featureless clippy |

## Acceptance

- [ ] A batch at its cap admits no further item of that project; a project without a cap still admits
- [ ] Remaining below `min_budget_for_new_attempt` admits nothing and starts no attempt
- [ ] A batch-run session is cancelled at the batch remainder, and the failure names the batch
- [ ] Manual runs are unaffected by batch figures
- [ ] `Settings > Queue` edits the caps (USD), `min_budget`, both `max_concurrent_items` levels and the stored window
- [ ] PRD success metric "batch spend overshoot" has a Mem conformance test and a Pg test over `SUM(run_step.usage)`
- [ ] Validation passes; snapshot changes limited to the new section and the strip

## Verified claims

Checked on `hr/MOD-12` at `da52b1db`. Falsified claims were amended before CONFIRM.

| Claim | Verdict | Evidence |
|---|---|---|
| No migration needed; batch identity exists (`queue_batch`, `run.batch_id`) | TRUE | `0016_auto_queue.sql`; `NewRun.batch_id` `run.rs:405`; written at `engine.rs:757` |
| Migrations end at `0017` here and on the host | TRUE | `ls` both `migrations/` dirs → `0017_follow_up.sql` last |
| `Run` read model has no `batch_id`; the mirror has none | TRUE | `Run` fields `run.rs:182-222`; `grep batch_i[d] cache_migrations/*.sql` → none; Mem keeps it beside `runs` (`mem.rs:304`) |
| `closed_reason` only admits `paused`/`drained` (a `capped` reason needs a migration) | TRUE | `0016_auto_queue.sql:22` |
| Caps are `project.settings` keys in micros; the snapshot freezes both | TRUE | `quota.rs:276-281`; `graph.rs:359-360`; `kind.rs:381-383` |
| `app_setting.per_token_cap_*` seeded and unread | TRUE | `0003_orchestration.sql:137-138`; no reader in `grep per_token_cap` outside `project.settings` paths |
| `min_budget_for_new_attempt` is unseeded; read as positive `i64` else `0` | TRUE | `0003` seed list (absent); `engine.rs:7143-7150` |
| Walk rule 2/rule 5 take run spend and run cap; unknown = unbounded | TRUE | `select.rs:160-194`; test `min_budget_skips_only_when_cap_and_spend_are_known` |
| Recorder cap compares its own per-session `usage`, starting at zero | TRUE | `record.rs:1636-1650` (`self.usage.cost_micros`); `Recorder::new` `usage: UsageTotals::default()` `:556`; engine uses `Recorder::new` `engine.rs:6154` |
| Run cap applied per step, so a run can spend about 2× its cap | TRUE (derived) | `RunCap{micros: per_token_cap_run}` per step (`engine.rs:6164`) + walk refuses only once `spent >= cap` |
| A breach settles `StepFailure::CapBreached` | TRUE | `gate.rs:122`, `:256` |
| Engine store is `WorkerStore`; 3 implementors | TRUE | `engine.rs:439-441`; `grep "impl.*WorkerStore for"` → `store/worker.rs:659`, `htui-store/src/worker.rs:116`, `:491` |
| `WorkerHost` has no project read and no batch-spend read | TRUE | method list `store/worker.rs:426-660` |
| `WorkerHost` implementors: Pg, `Backend`, test `Failing` | TRUE | `htui-store/src/worker.rs:784`, `:890`; `runtime.rs:3725` |
| `WriteStore` has 5 implementors | TRUE | `mem.rs:7856`, `pg/write.rs:1008`, `writer.rs:329`, `htui-agent/src/conformance.rs:751`, `htui-agent/tests/recorder.rs:438` |
| `set_setting` project rung merges under CAS on `project.updated_at`; app rung `expected: None` = no row | TRUE | `traits.rs:1107-1110` doc |
| Prompt registry cannot take the keys (pinned to `0002`'s ten rows, no nullable kind) | TRUE | `prompt/settings.rs:185-205`, `:222-224`, `SettingKind` `:297` |
| `box.settings` has one writer, `edit_box`, merging `executor` only | TRUE (agent) | `traits.rs:545-572`; `BoxEdit` `box_.rs:163-171` |
| **Adding a `BoxEdit` field is local to T4** | **FALSE → amended** | `grep "BoxEdit {"`: about 35 sites in more than 20 files, about 23 spelling every field (`engine.rs:14377`, `htui-orch/src/conformance.rs:4238`, `boxes.rs:578`, …); D9 now writes the box key through `set_queue_setting(Box)` |
| No Settings section edits the five keys today | TRUE (agent) | no hit in `crates/htui/src`, `crates/htui/tests` |
| Secrets section is the newest pattern (`7188c47d`, `9e008387`) | TRUE (agent) | `app/mod.rs:79`; `git show --stat` |
| `crates/htui/src/lib.rs` carries the serve-module lines | TRUE | `lib.rs:40` `pub mod secrets_settings;` |
| `scheduler_window` has no defined inner shape | TRUE | `docs/ANA-2.md:1634`; no code reader |
| `admit()` computes `free_slots` before the enqueue loop | TRUE | `runtime.rs:2185-2216` |
| Independence T2 ∥ T3 ∥ T4 (after amendment) | TRUE | T2 = {`select.rs`, `engine.rs`, `htui-orch/src/conformance.rs`}; T3 = {`runtime.rs` `admit`, `auto_queue.rs`}; T4 = {`queue_settings.rs` (core), `model/mod.rs`, `traits.rs`, `store/conformance.rs`, `mem.rs`, `pg/write.rs`, `writer.rs`, `.sqlx`, the two `htui-agent` spies} → pairwise ∅ |
| T1 before T2/T3/T4 | TRUE | T1 ∩ T4 = {`mem.rs`, `.sqlx`}; T1 ∩ T3 = {`runtime.rs`}; T2 consumes T1's API |
