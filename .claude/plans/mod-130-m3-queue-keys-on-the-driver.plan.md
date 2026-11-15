# Plan: MOD-130 M3 — The queue keys on the driver

**Source PRD**: `.claude/prds/mod-130-execution-targets.prd.md`
**Selected Milestone**: 3 — The queue keys on the driver
**Complexity**: Medium
**Status**: in progress (planned 2026-11-15)
**Routing**: PRD path, ultracode for implement (C2, C4) (`/handoff-run` in the `hr/MOD-130` sandbox,
2026-11-15). The plan runs without a confirm gate (handoff-run step 3). No architect: M3 adds no module
seam. It re-keys four existing reads and one writer, and moves two checks to another box id.

## Open questions

Neither question blocks M3. For every target this build can run (`local` only, D1), both answers behave
the same. The plan goes ahead on the stated default. The maintainer may overrule either one, and Q1's
alternative costs one line plus the pins listed under it.

- **Q1 — Does a queued `local` run carry `executing_box_id = here` (D8 read literally), or `NULL`?**
  D8 says `enqueue` writes the driving box "for every kind but `enrolled`". The fixed constraint says "a
  run on the seeded local target is byte-for-byte today's run". The first risk row says "existing claim,
  worker and queue tests stay unedited except counts". These cannot all hold literally, because a queued
  local run has `executing_box_id = NULL` today. **Default (P1):** write the driver only when the work
  runs on a box other than the driver (a child box or an ssh machine). A local run keeps `NULL`, and the
  claim and poll key `COALESCE(executing_box_id, target_box_id)` still names this box. **Alternative:**
  `driver_of` answers `Some(here)` for `local` as well. That moves these pins:
  - `a_capability_refusal_at_claim_fails_the_run_by_name`'s "no box executed it" assertion
    (`crates/htui-orch/src/conformance.rs:4199`);
  - every engine-test assertion that a queued or failed-at-claim run has `executing_box_id == None`;
  - the `htui stop` report's `on_box` for queued runs (`crates/htui/src/stop_cmd.rs:521`), which then
    prints this box.
- **Q2 — Do the queue runner's tag filter and the Backlog's "missing tags" check the item's resolved
  target box (D9), or this box?** These are `ready_items`, `missing_tags` and `missing_tags_of`
  (`crates/htui-core/src/store/mem.rs:1095`, `:1132`, `:1152`, and their Pg twins). They run before any
  target is resolved. Resolving the cascade per item inside a store read would pull the engine's
  cascade inputs into the store: the `app` map, the graph pin and the scope rows. **Default (P8):** they
  stay on this box in M3. That is exact while only `local` runs. MOD-44, which makes the first non-local
  kind runnable, owns the switch. The engine's own enqueue check (P2) and the claim (P4) already use the
  target box.

## Summary

After M3 the queue, the claim and the worker poll ask **"who drives this run?"** instead of **"where does
it run?"**:

- **The driver is named.** `enqueue` writes `executing_box_id` = this box for a run whose work runs on
  another box, unless the kind is `enrolled` (D8, P1). The driver of any run is
  `COALESCE(executing_box_id, target_box_id)`, which is `Run::driver()`.
- **Claim and poll key on the driver.** `claim_run` admits a queued run only for its driver.
  `queued_runs_on_box` lists the queued runs a box drives. Slots and overlap stay counted on the
  claiming box, the driver (D9). The admit write is unchanged: `executing_box_id = lease_box_id = box`.
- **Tags and candidates check the box the work runs on** (D9): the claim-time tags (`R-ORCH-10`), the
  enqueue-time tags, and rung 3 of the candidate chain all read `target_box_id`.
- **The queue overlay's classification keys on the driver.** `classify_entry` answers `Running` for a
  run this box drives, even when its work runs on a child box. `Elsewhere` carries the **driving** box's
  hostname. The overlay wording itself is M4.
- **Enrolled waits for its own worker.** An `enrolled` run keeps `executing_box_id = NULL`, so its
  target box's worker drives it. That is today's rule and MOD-43's other-box dispatch.

A local run is unchanged: same columns, same claim, same slots, same tags (P1, Q1). No migration and no
cache migration (P10). Chat runs are untouched (D17).

## Shared decisions

Every task's brief carries this section.

### Decisions from the PRD gate (2026-11-14) that M3 implements

| D | What M3 does with it |
|---|---|
| D1 | Non-local kinds are still refused at enqueue (`RUNNABLE_KINDS`). M3's child-box behaviour is proven at the store with `create_run` and in engine tests that widen the runnable set (P7). |
| D8 | The driver is `COALESCE(executing_box_id, target_box_id)`. `enqueue` writes `executing_box_id` for a non-`enrolled` run whose work runs on another box (P1). Claim and poll key on the driver. No new column. |
| D9 | Tags (`R-ORCH-10`, at enqueue and at claim) and rung 3's agent candidates read `target_box_id`. Slots and overlap read the driver. |
| D10 | Unchanged: `run_start_refusal` refuses a target whose machine is not this box. A child of this box passes, and its run's driver is this box. |
| D17 | `start_chat_run` is untouched: a chat run is inserted at `running` with `executing_box_id = target_box_id = here`. |

### Decisions this plan takes

- **P1 — `driver_of`.** A new pure function in `crates/htui-core/src/model/target.rs`:

  ```rust
  /// The box that drives a run on `binding` started on `here`, as `NewRun.executing_box_id`
  /// (MOD-130 M3, D8, P1): `None` for `enrolled` (its own worker drives it) and whenever the work
  /// runs on `here` (local), where `COALESCE(executing_box_id, target_box_id)` already names
  /// `here`; else `Some(here)`.
  #[must_use]
  pub fn driver_of(binding: &TargetBinding, here: BoxId) -> Option<BoxId>;
  ```

  The body is `(binding.target.kind != TargetKind::Enrolled && runs_on(binding, here) != here).then_some(here)`.
  See Q1 for the literal-D8 alternative.
- **P2 — The order inside `enqueue_with` changes.** M2 P2 left this to M3. The new order is:
  1. `start_enabled`
  2. `run_workspace_for` (R1 L3)
  3. **the graph read** (`graph::read_graph`, P3), whose `graph.target_id` is the workflow level
  4. `cascade_target`
  5. `target_binding`
  6. `run_start_refusal(.., self.runnable)` → `refuse_target`
  7. `on = runs_on(&binding, here)`
  8. **missing tags against `on`** → `refuse_missing_tags`
  9. **`graph::resolve_with(.., graph, .., on)`** (rung 3 reads `on`'s `agent_box` rows) → `refuse_rung_four`
  10. `create_run(NewRun { target_box_id: on, executing_box_id: driver_of(&binding, here), target: Some(chosen), .. })`

  A target refusal now precedes the tag and rung-4 refusals. M2's risk row anticipated this. An item with
  no graph now gets `NoGraph` before its tags are checked. Tags against `on` equal tags against `here` for
  every local run, so `enqueue_refuses_missing_tags_before_resolving` stays green unedited.
- **P3 — The graph is read once (`crates/htui-orch/src/graph.rs`).** Split the read out of
  `resolve`/`resolve_or_refused` without changing either signature:
  - `pub(crate) async fn read_graph<G: GraphSource>(source: &G, item: &Item) -> Result<ResolvedGraph, ResolveError>`:
    `Box::pin(source.resolve_graph(item.id))`, `None` → `ResolveError::NoGraph(item.id)`.
  - `pub(crate) async fn resolve_or_refused_with(store, source, item, graph: ResolvedGraph, mode, app, requested_scope, box_id) -> Result<Resolution, ResolveError>`:
    today's body from the project read onwards.
  - `pub async fn resolve_with(store, source, item, graph: ResolvedGraph, mode, app, requested_scope, box_id) -> Result<Resolved, ResolveError>`:
    today's `resolve` body over `resolve_or_refused_with`.
  - `resolve` = `read_graph` + `resolve_with`; `resolve_or_refused` = `read_graph` + `resolve_or_refused_with`.

  Keep every `Box::pin` (the 2 MiB debug stack, MOD-138 blueprint §4). `Resolved.workflow_target` stays
  and equals the pre-read `graph.graph.target_id`. The engine reads the pin from the pre-read value
  (`ResolvedGraph.graph.target_id` is `Copy`) before moving the graph in. This gives one read, so the
  workflow level and the snapshot can never come from two different graph revisions.
- **P4 — `claim_run` (both stores).**
  - **Claimability:** `status = queued` and **`COALESCE(executing_box_id, target_box_id) = box_id`**
    (Mem: `claimed.driver() != box_id` → `NotClaimable`).
  - **Tags:** against **`target_box_id`**:
    - Mem: `self.missing_for(&row.required_tags, claimed.target_box_id)`.
    - Pg: the missing-tags query joins `box b ON b.id = r.target_box_id` instead of `b.id = $2`, and
      drops that bind.
  - **Unchanged:** the box `FOR UPDATE` read (the claiming box's settings, so the limit is the driver's);
    the slot count and the overlap set over `executing_box_id = box_id`; the admit write
    (`executing_box_id = lease_box_id = box_id`); the stop gate.
  - **Pg:** the `FOR UPDATE` run read also selects `executing_box_id AS "executing_box_id: BoxId"`.
  - **Docs:** update `Claim::NotClaimable` (`crates/htui-core/src/model/overlap.rs:134`) and the
    `WriteStore::claim_run` docs (`crates/htui-core/src/store/traits.rs:1571-1613`) to "not driven by
    `box`".
- **P5 — The queue facts are additive.** `QueueRunFact` (`crates/htui-core/src/model/queue.rs:210`)
  keeps `target_box_id`/`target_hostname` (M4's "runs on" needs them) and gains:

  ```rust
  /// `COALESCE(run.executing_box_id, run.target_box_id)`: the box that drives the run (MOD-130 M3, D8).
  pub driver_box_id: BoxId,
  /// That box's `hostname`; `None` when no box row answers (Mem only).
  pub driver_hostname: Option<String>,
  ```

  - `classify_entry` keys on `run.driver_box_id == here`, and `Elsewhere.hostname` is `driver_hostname`.
  - Update the `EntryState::Running`/`Elsewhere` docs:
    - `Running`: "driven by this box; its work may run on a child box";
    - `Elsewhere`: "driven by another box (MOD-43 owns choosing)";
    - `Elsewhere.hostname`: "the driving box's hostname".
  - The overlay sentence (`crates/htui/src/ui/overlay/queue.rs:303`) is **not** touched (M4).
- **P6 — `NewRun.executing_box_id`.** `#[serde(default)] pub executing_box_id: Option<BoxId>` on `NewRun`
  (`crates/htui-core/src/model/run.rs:509`), documented as "the driving box when it is not the target box
  (MOD-130 M3, D8, P1); `None` = the target box drives".
  - **Mem `create_run`:** stores it. A box id naming no row gives
    `StoreError::Constraint(references_no_row("run.executing_box_id", id, "box"))`, checked right after
    the `run.target_box_id` check (`mem.rs:6862`).
  - **Pg `create_run`:** binds it in place of the literal `NULL` (`pg/write.rs:8001`). A phantom box is
    the column's FK, `Constraint` through `map_sqlx`, the same as `target_box_id` (`:7853`).
  - **Lock order:** the extra FK takes `FOR KEY SHARE` on the driver box. `claim_run` takes run → box
    `FOR UPDATE` and never locks an item, so no new cycle appears (C21).
  - Add `Run::driver(&self) -> BoxId` (`self.executing_box_id.unwrap_or(self.target_box_id)`, `const fn`,
    doc MOD-130 M3 D8). Mem's `claim_run`, `queued_runs_on_box`, `latest_queue_run` and `run_hostname`
    use it.
- **P7 — Test seams for a runnable child-box target.**
  - `Engine` gains `runnable: &'static [TargetKind]`, set to `RUNNABLE_KINDS` in `Engine::new` (still a
    `const fn`). `enqueue_with` passes `self.runnable` to `run_start_refusal`. A
    `#[cfg(test)] pub(crate) fn with_runnable_kinds(mut self, kinds: &'static [TargetKind]) -> Self`
    lets engine tests use `TargetKind::ALL`. Production behaviour is unchanged.
  - `MemStore::set_target_probed(&self, id: TargetId, at: DateTime<Utc>) -> bool`, under
    `#[cfg(feature = "test-support")]`, sets `target.probed_at` the way no writer does until MOD-44/131.
    It mirrors `set_run_snapshot` (`mem.rs:1786`).
- **P8 — What stays on this box in M3** (Q2). Each item below is listed in the audit with its reason:
  - `ready_items`, `missing_tags` and `missing_tags_of` (the pre-enqueue filters, `LiveFacts.missing_tags`,
    the Backlog);
  - the walk-time `agent_boxes(self.parts.box_id)` selection read (`engine.rs:5518`: quota latches, not
    candidates; no child-box walk exists before MOD-44);
  - `take_integrations`, `adopt_runs`, `take_lease`, `pending_resumes`, `resume_run`.

  The last group are a driver's duties, and the driver is `executing_box_id` once claimed.
- **P9 — Enrolled.** `driver_of` answers `None`, so the run waits for its target box's worker. That is
  today's `target_box_id`-keyed behaviour, preserved by the `COALESCE`. `run_start_refusal` is unchanged:
  `NotRunnable` (MOD-47) precedes `BoundElsewhere`, so relaxing D10 for `enrolled` is MOD-47's.
- **P10 — No migration, no cache migration.**
  - D8 reuses `run.executing_box_id` (`0001_init.sql:456`).
  - No index covers `target_box_id` today; the poll's scan is accepted (R-6, `pg/read.rs:2651`). The
    `COALESCE` predicate costs the same.
  - The cache mirrors `executing_box_id` since `cache_migrations/0001_mirror.sql:106`.
  - No pin of `migrations::`/`connect::` moves.

### `executing_box_id` reader audit (D8; the PRD risk "a queued run with `executing_box_id` set reads as started somewhere")

Under P1, the only queued run with a non-`NULL` `executing_box_id` is one whose work runs on another box.
No production path writes such a run in this build (D1). Each reader was checked for when one exists.

| Reader | Where | Filter | Verdict |
|---|---|---|---|
| `active_runs_on_box` | `mem.rs:1188`, `pg/read.rs:2640` | `running`/`awaiting_approval` | unaffected: a queued run is never counted |
| `running_runs_on_box` | `mem.rs:1608`, `pg/read.rs:3012` | `running` | unaffected |
| `run_hostname` / `run_summaries` / Runs pane `box_hostname` | `mem.rs:2328`, `pg/read.rs:415-422`, `cache/read.rs:789-792` | `COALESCE(executing, target)` | already names the driver; correct under D8. M4 adds "runs on <target>" |
| `start_chat_run` | `mem.rs:3192`, `pg/write.rs:4549` | writer | unchanged (D17) |
| `create_run` | `mem.rs:6915`, `pg/write.rs:8001` | writer | **changes**: writes `NewRun.executing_box_id` (P6) |
| `claim_run` slot and overlap sets | `mem.rs:7007`, `pg/write.rs:8222`, `:8247` | `running`/`awaiting`/`stopped` | unaffected; stays on the driver (D9) |
| `claim_run` admit write | `mem.rs:7051`, `pg/write.rs:8274` | writer | unchanged; writes the driver it already holds |
| `adopt_runs` | `mem.rs:7102`, `pg/write.rs:8396` | `running` | unaffected |
| `take_lease` | `mem.rs:7158`, `pg/write.rs:8603`; engine `engine.rs:3368` | `running`/`awaiting` | unaffected |
| `pending_resumes` / `resume_run` / `cancel_stopped_run` | `mem.rs:7215`, `:7241`, `:7277`; `pg/write.rs:8975`, `:9023`, `:9099`, `:9120`; engine `cancel_stopped` `engine.rs:3033` | `stopped` | unaffected: no product path moves a queued run to `stopped` (C8) |
| `take_integrations` | `mem.rs:8794`, `pg/write.rs:10766` | records of finished runs | unaffected |
| `pending_commands` (relay) | `mem.rs:10952`, `pg/relay.rs:596` | terminal/`stopped`/lapsed live | a queued run never matches. A child-box run cancelled while queued with a stale pending row is now applied by its driver's poll; before, nothing applied it. Benign; a queued cancel is rowless anyway (C9) |
| worker cancel path | `htui-worker/src/runtime.rs:3398` | reached for non-queued runs | unaffected (queued → rowless CAS, `:3347-3366`) |
| `htui stop` report `on_box` | `crates/htui/src/stop_cmd.rs:491-523` | non-terminal runs | a queued child-box run prints its driver: correct (the driver holds it); M4 owns wording |
| Activity triggers | `0036_activity_event.sql:288-339` | `OLD.status`/`NEW.status` only | unaffected: no column read (C7) |
| Indexes | `0003_orchestration.sql:50`, `0037_stopped_run_index.sql:16` | — | no behaviour |
| Mirror, rows, demo | `cache_migrations/0001_mirror.sql:106`, `cache/refresh.rs:1268-1317`, `cache/read.rs:305-324`, `pg/rows.rs:60`, `pg/demo.rs:478` | copies | unaffected |
| Test fixtures | `fixtures.rs:1639-1711`, `crates/htui/src/queue_overview.rs:238` | — | unchanged; their driver equals their target |

Readers of `target_box_id` that **become driver-keyed**:
- Mem: `claim_run` (`mem.rs:6957`), `queued_runs_on_box` (`:1208`), `latest_queue_run` (`:6603-6617`).
- Pg: `claim_run` (`pg/write.rs:8105`, `:8133`), `queued_runs_on_box` (`pg/read.rs:2664`), `queue_rows`
  (`:2745-2761`).
- `classify_entry` (`queue.rs:475`).

Readers that **become target-keyed**: the claim tags (`mem.rs` `missing_for(.., box_id)`, `pg/write.rs`
`b.id = $2`); the engine's enqueue tags and rung 3 (`engine.rs:1471`, `:1483`).

### Conventions

- **Conformance pattern.** Each case is `async fn <name><S: WriteStore>(store: &S)` and starts with
  `const CASE: &str = "<name>";`. Register it twice: in `CASES` (`conformance.rs:120`) and as an arm of
  `run_case_inline`. Mirror `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks`
  (`conformance.rs:10481`) and `claim_run_admits_one_and_refuses_the_second` (`:9491`).
  - **A child box:** `create_target(NewTarget { kind: Docker, .. })`, then
    `attach_child_box(NewChildBox { parent: ids::BOX, target, .. })`.
  - **A second machine:** register one as `pg_criteria`'s `queue_rows_answer_alike_on_both_stores` does
    for `laptop` (`pg_criteria.rs:~13119`), or as an existing conformance case does.
  - **Tags:** set them with `edit_box(id, edit_version, BoxEdit { declared_tags: Some(..), .. })`. If
    `edit_box` refuses a container row, add a `test-support` seam beside `set_target_probed` and say so in
    the commit.
- **The case count is pinned in three places, all reading 346 before M3** (C13):
  `crates/htui-store/tests/it/pg_conformance.rs:78` (`EXPECTED_CASES`), its doc text at `:74-77`, the
  assert message at `:85`; and `crates/htui-core/tests/it/mem_store.rs:65` with its message. T1 adds 1
  and T2 adds 3, so the count becomes **350**. Extend both doc texts with one clause each.
- **Inherent queue reads have no conformance path.** `queued_runs_on_box` and `queue_rows` are on
  `MemStore`/`PgStore` (dispatched by `Backend` and `WorkerHost`), not on `WriteStore`. Test them as a
  `store::mem` unit test plus its `pg_criteria` twin, the existing pairing
  (`mem.rs:18268` ↔ `pg_criteria.rs:6350`).
- **`htui-orch`'s conformance suite is MemStore-only** (M2 C11). Engine tests go in `engine.rs`'s test
  module. Build the engine with `harness_engine!(harness.orch, engine)` (`engine.rs:14112`), then
  `let engine = engine.with_runnable_kinds(TargetKind::ALL);` where P7 needs it. Claim with
  `harness.orch.store.claim_run(..)`, not `engine.claim`, so no walk starts on a child box.
- **`.sqlx`.** Every changed `query!`/`query_as!` needs prepared data. In the hr sandbox (Postgres at
  `localhost:5439`, user `postgres`, trust auth; `docs/hr-sandbox.md` "Changing SQL queries in a run"):

  ```
  psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
  cd crates/htui-store
  export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations
  cargo sqlx prepare -- --all-targets --all-features     # keep both flags, or test-only queries are deleted
  cargo sqlx prepare --check -- --all-targets --all-features
  ```

  `prepare` rewrites the directory, so commit what it leaves: new entries in, stale ones out. A "potentially
  unused queries" warning from `--check` is expected. Queries that change:
  - T1: `create_run`'s `INSERT … RETURNING`;
  - T2: `claim_run`'s run read and its missing-tags query;
  - T3: `queued_runs_on_box` and `queue_rows`.
- **Tests.** Integration tests are modules of each crate's single `it` binary. Run one module with
  `cargo nextest run -p <crate> --all-features -E 'binary(it) and test(/^<module>::/)'`, and unit tests
  with `-E 'not binary(it) and test(/<filter>/)'`. `HTUI_TEST_DATABASE_URL` is exported in the sandbox.
  TDD: write each task's tests first and watch them fail for the right reason.
- **Regression bar.** Do not edit an existing claim, worker, queue or engine test's assertions. Only the
  count pins and mechanical literal additions (`executing_box_id: None`, the two `QueueRunFact` fields)
  move. A test that needs any other edit is a finding: stop and report it.
- **Lints.** `unsafe_code = "forbid"`, the workspace lints and the MSRV are unchanged. Every new pub item
  has a doc line citing `MOD-130 M3` and its D or P. Run
  `cargo clippy -p <crate> --all-targets --all-features -- -D warnings` and
  `cargo clippy -p <crate> -- -D warnings` (featureless) on every crate touched, and `cargo fmt`.
- **Never run the workspace gate.** Commit incrementally, one commit per green step, with the message
  `feat(<area>): MOD-130 M3 T<n> …` ending with the attribution line. Never stash; never push.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Pure helper beside the resolver | `runs_on`, `run_start_refusal` (`crates/htui-core/src/model/target.rs:239-281`) and their tests | `#[must_use]` free fn over `TargetBinding`; table-style unit tests in `target.rs`'s `mod tests` |
| Claim refusal ordering | `State::claim_run` (`mem.rs:6941-7060`), `PgStore::claim_run` (`pg/write.rs:8091-8293`) | claimability → stop → tags → slot → overlap → write, decided before any write |
| Claim conformance | `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` (`conformance.rs:10481`), `claim_run_checks_tags_after_claimability_and_before_the_slot` (`:10650`) | create a run with `NewRun`, claim, assert the verdict and the row |
| Inherent read with a Pg twin | `queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order` (`mem.rs:18268`, `pg_criteria.rs:6350`) | Mem unit test and `pg_criteria` test naming each other in their docs |
| Queue classification tests | `a_live_run_elsewhere_beats_everything` (`queue.rs:~960`), helper `run(status, target)` (`queue.rs:840`) | `QueueRow` fixtures through helpers, `classify_entry` asserted per state |
| Test-only store seam | `MemStore::set_run_snapshot` (`mem.rs:1786`) | `#[cfg(feature = "test-support")]`, "as no writer does", answers whether the row exists |
| Engine refusal tests | `enqueue_refuses_missing_tags_before_resolving` (`engine.rs:19292`), `enqueue_refuses_a_target_this_build_cannot_run` (`:19799`) | MemStore harness, assert the error, no run written, note and `blocked` |
| Rung-3 tests | `graph.rs:2901-2970` (`agent_box` row on the fixture box; `without_candidates`) | `upsert_agent_box` then resolve |

## Files to Change

| File | Task |
|---|---|
| `crates/htui-orch/src/graph.rs` | T0 |
| `crates/htui-core/src/model/{run,target,mod}.rs` | T1 |
| `crates/htui-core/src/store/{mem,conformance}.rs` | T1, T2, T3 (mem) |
| `crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/` | T1, T2 (`.sqlx` also T3) |
| every `NewRun { … }` literal (C14: 78 in 20 files; `htui-core`, `htui-store/tests`, `htui-orch` incl. `engine.rs`, `htui-agent/tests`, `htui-mcp/tests`, `htui-worker`, `htui`) | T1 |
| `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-core/tests/it/mem_store.rs` | T1, T2 |
| `crates/htui-core/src/model/overlap.rs`, `crates/htui-core/src/store/traits.rs` (docs) | T2 |
| `crates/htui-core/src/model/queue.rs`, `crates/htui-store/src/pg/read.rs`, `crates/htui-store/tests/it/pg_criteria.rs`, `crates/htui-core/src/store/worker.rs` (doc of `WorkerHost::queued_runs_on_box`) | T3 |
| `crates/htui-orch/src/engine.rs` (Engine field, `enqueue_with`, docs, tests) | T4 (and T1's literals) |

## Tasks

Two lanes, then a join:

- **Lane A:** T0, alone. Its file set `{graph.rs}` meets no other task's.
- **Lane B:** T1 → T2 → T3, serial. They share `mem.rs`, `.sqlx` and the conformance files.
- **T4** needs T0 (`read_graph`/`resolve_with`), T1 (`NewRun.executing_box_id`, `driver_of`,
  `set_target_probed`) and T2 (the driver-keyed claim its tests use). Its file set `{engine.rs}` is
  disjoint from T3's, so **T4 runs beside T3**.
- **T1 is `verify: foundation`.** It changes `NewRun` across every crate. T0, T2, T3 and T4 go to the
  review unverified.

### Task 0: The graph is read once (htui-orch, refactor)

**Depends on:** nothing. **Independent of T1–T3** (file set `{crates/htui-orch/src/graph.rs}`).
**Files:** `crates/htui-orch/src/graph.rs`.

Do P3: add `read_graph`, `resolve_or_refused_with` and `resolve_with`, and make `resolve` and
`resolve_or_refused` thin wrappers. Signatures of `resolve`/`resolve_or_refused` and every caller stay
unchanged (`engine.rs:1476`, `:5203`; `crates/htui/tests/it/graph_editor.rs:362`, `:1356`; graph tests
`:5523`, `:6208`, `:6592`).

Tests first, in `graph.rs`'s test module:
- `resolve_with_a_pre_read_graph_answers_as_resolve`: over the demo store, `resolve_with(.., read_graph(..)?, ..)`
  equals `resolve(..)` field for field (snapshot, repo scope, workflow target).
- `read_graph_refuses_an_item_with_no_graph`: `ResolveError::NoGraph(item)`, the same as `resolve` answers
  today.

**Check:**
- `cargo nextest run -p htui-orch --all-features -E 'not binary(it) and test(/graph::/)'`
- `cargo nextest run -p htui-orch --all-features -E 'test(/enqueue|resume/)'`
- clippy (both forms) and fmt on `htui-orch`.

### Task 1: Runs name their driver (`verify: foundation`)

**Depends on:** nothing. **Files:**
- `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/model/target.rs`, `crates/htui-core/src/model/mod.rs`;
- `crates/htui-core/src/store/mem.rs` (`create_run`, `set_target_probed`), `crates/htui-core/src/store/conformance.rs`;
- `crates/htui-store/src/pg/write.rs` (`create_run`), `crates/htui-store/.sqlx/`;
- `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-core/tests/it/mem_store.rs`;
- every `NewRun { … }` literal (C14).

1. **Model.**
   - Add `NewRun.executing_box_id` (P6) and `Run::driver()` (P6).
   - Add `driver_of` (P1) to `target.rs`, and re-export it from `model/mod.rs` beside `runs_on`.
     `model/mod.rs` is T1's file too.
   - Doc `Run.executing_box_id`: "the box that drives the run: written at enqueue when the work runs on
     another box (MOD-130 M3, D8), else at claim; `NULL` while a run waits for its target box's worker".
2. **Literals.** Add `executing_box_id: None` to every `NewRun { … }` literal, `enqueue_with`'s too (T4
   sets it). Let the compiler list them: `cargo check --workspace --all-targets --all-features`.
3. **Mem `create_run`** stores the field and refuses a phantom box (P6). **Mem `set_target_probed`** (P7).
4. **Pg `create_run`** binds the field (P6); prepare `.sqlx`.
5. **Conformance (+1, 346 → 347):** `create_run_records_the_driving_box`:
   - a `NewRun` whose `target_box_id` is a child box of `ids::BOX` and whose `executing_box_id` is
     `Some(ids::BOX)` reads back both; `run_summaries`' `box_hostname` is `ids::BOX`'s hostname;
   - `executing_box_id: None` reads back `None`;
   - a phantom `executing_box_id` is `StoreError::Constraint`, and `runs(item)` is unchanged.
   Move the three count pins.

Tests first, in `target.rs`'s `mod tests`: `driver_of_names_here_only_for_work_elsewhere`:
- local → `None`;
- docker on a child of `here` → `Some(here)`;
- ssh bound to a machine (any id) → `Some(here)`;
- enrolled bound to another machine → `None`.

In `run.rs`: `driver_is_the_executing_box_else_the_target`.

**Check:**
- `cargo nextest run -p htui-core --all-features -E 'not binary(it) and test(/model::(target|run)::/)'`
- `cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'`
- `cargo check --workspace --all-targets --all-features`, since the literals span every crate
- the sqlx `--check`, then clippy (both forms) and fmt on every crate touched.

### Task 2: The claim keys on the driver; tags check the box the work runs on

**Depends on:** T1. **Files:**
- `crates/htui-core/src/store/mem.rs` (`State::claim_run`), `crates/htui-core/src/store/conformance.rs`;
- `crates/htui-store/src/pg/write.rs` (`PgStore::claim_run` and its doc), `crates/htui-store/.sqlx/`;
- `crates/htui-core/src/model/overlap.rs` (doc), `crates/htui-core/src/store/traits.rs` (docs);
- `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-core/tests/it/mem_store.rs`.

Do P4 in both stores, in the same order: claimability, stop gate, tags, slot, overlap, write.

Tests first. Conformance cases (+3, 347 → 350):
- `claim_run_keys_on_the_driving_box`:
  - **Child-box run** (`target_box_id = child`, `executing_box_id = Some(BOX)`): claimed by `child` →
    `NotClaimable`, row unchanged. Claimed by `BOX` → `Admitted`; then `status = running`,
    `executing_box_id = lease_box_id = Some(BOX)`, `target_box_id = child`.
  - **MOD-43 shape** (`target_box_id = other machine`, `executing_box_id = None`): claimed by `BOX` →
    `NotClaimable`. Claimed by the other machine → `Admitted`. The enrolled-worker rule, P9.
- `claim_run_checks_tags_on_the_box_the_work_runs_on`:
  - the child declares `gpu`, `BOX` does not; an item requiring `gpu` on a child-box run is `Admitted`
    by `BOX`;
  - `BOX` declares `vulkan`, the child does not; an item requiring `vulkan` on a child-box run gives
    `MissingTags { missing: ["vulkan"] }`, the run `failed` with `missing_tags_failure`, and the item
    `blocked`.
- `claim_run_counts_a_child_box_run_on_the_driver`:
  - **Slot:** with `BOX`'s `max_concurrent_items = 1` and one local run `running` on `BOX`, a child-box
    run's claim by `BOX` → `SlotFull { running: 1, limit: 1 }`. Set the limit the way the existing slot
    case does.
  - **Overlap:** after admitting the child-box run, a local run sharing its repo scope gets
    `Overlaps { with: <child-box run> }`.

Move the three count pins to 350. Existing claim cases and the Mem unit tests
(`claim_run_refuses_an_overlapping_scope_and_a_full_box`, `a_run_with_no_item_is_never_refused_for_tags`)
stay unedited and green.

**Check:**
- `cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'`
- `cargo nextest run -p htui-core --all-features -E 'not binary(it) and test(/claim_run/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and (test(/^pg_conformance::/) or test(/claim_run/))'`
- `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^fake_conformance::/)'` (claim regression)
- the sqlx `--check`, then clippy (both forms) and fmt on `htui-core` and `htui-store`.

### Task 3: The poll and the queue overlay key on the driver

**Depends on:** T2. **Runs beside T4.** **Files:**
- `crates/htui-core/src/model/queue.rs`;
- `crates/htui-core/src/store/mem.rs` (`queued_runs_on_box`, `latest_queue_run`, unit tests),
  `crates/htui-core/src/store/worker.rs` (doc of `WorkerHost::queued_runs_on_box`);
- `crates/htui-store/src/pg/read.rs` (`queued_runs_on_box`, its doc at `:2650`, `queue_rows`),
  `crates/htui-store/.sqlx/`, `crates/htui-store/tests/it/pg_criteria.rs`.

1. **`queue.rs`.** Do P5: the new `QueueRunFact` fields, `classify_entry`, and the `EntryState` docs. The
   test helper `run(status, target)` (`:840`) sets `driver_box_id = target` and `driver_hostname` =
   `target_hostname`. Update the other literals: `mem.rs:6609`, `:22041`; `pg/read.rs:2810`.
2. **Mem.** `queued_runs_on_box` filters `row.driver() == box_id`. `latest_queue_run` fills
   `driver_box_id = run.driver()` and `driver_hostname` from that box.
3. **Pg.**
   - `queued_runs_on_box`: `WHERE COALESCE(executing_box_id, target_box_id) = $1 AND status = 'queued'`.
   - `queue_rows`' lateral adds `COALESCE(r.executing_box_id, r.target_box_id) AS driver_box_id` and
     `db.hostname AS driver_hostname` through `LEFT JOIN box db ON db.id = COALESCE(r.executing_box_id, r.target_box_id)`.
   - Prepare `.sqlx`.

Tests first:
- `queue.rs` unit tests:
  - `a_run_driven_here_on_a_child_box_is_running_here`: `driver_box_id = here`, `target_box_id = child`
    → `Running`, for `Queued` and `Running`;
  - `a_run_driven_elsewhere_names_the_driver`: `driver_box_id = other` → `Elsewhere { hostname: <driver's> }`,
    even when `target_box_id == here`.
- Mem unit `queued_runs_on_box_lists_the_runs_this_box_drives` and its `pg_criteria` twin of the same
  name, the two naming each other:
  - a child-box run (`executing_box_id = Some(BOX)`) is listed for `BOX`, not for the child;
  - a MOD-43-shape run (`target_box_id = other`, `None`) is listed for `other`, not for `BOX`;
  - a local run is listed for `BOX`.
- Mem unit `queue_rows_name_the_driving_box` and its `pg_criteria` twin: a queued child-box run's fact
  has `driver_box_id = BOX`, `driver_hostname = BOX`'s, `target_box_id = child`,
  `target_hostname = child`'s. Extend `queue_rows_answer_alike_on_both_stores` (`pg_criteria.rs:~13119`)
  to compare the two new fields.

Existing tests stay green unedited apart from the added literal fields. Among them:
`queue_rows_name_the_parked_step_and_the_target_hostname` (`mem.rs:22085`), `a_live_run_elsewhere_beats_everything`,
the htui `queue_overview` snapshots (C18).

**Check:**
- `cargo nextest run -p htui-core --all-features -E 'not binary(it) and test(/model::queue::|queued_runs_on_box|queue_rows/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_criteria::/) and test(/queued_runs_on_box|queue_rows/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'` (calls `queue_rows`, `pg_conformance.rs:433`)
- `cargo nextest run -p htui --all-features -E 'test(/queue_overview|overlay::queue|queue_state/)'`: no `.snap` may change
- `cargo nextest run -p htui-worker --all-features -E 'test(/queue|claim/)'`
- the sqlx `--check`, then clippy (both forms) and fmt on `htui-core`, `htui-store`.

### Task 4: `enqueue` names the driver and checks the target box

**Depends on:** T0, T1, T2. **Runs beside T3.** **Files:** `crates/htui-orch/src/engine.rs`.

1. **Engine field (P7).** Add `runnable` to `Engine`, set it in `new`, add `with_runnable_kinds`
   (`#[cfg(test)]`), and use it in `enqueue_with`.
2. **`enqueue_with` (P2).** Re-order as in P2. Use `graph::read_graph` once and `graph::resolve_with`
   with `on = runs_on(&binding, here)`. Check `missing_tags(item.id, on)`. Write
   `NewRun { target_box_id: on, executing_box_id: htui_core::model::driver_of(&binding, here), .. }`.
   Update the refusal-order doc on `Engine::enqueue` (`engine.rs:1380-1389`): target, then tags, then
   rung 4. Update the `missing_tags` comment ("before resolution, … the cheaper box-level fact") to name
   the target box.
3. **Tests first** (MemStore engine, `engine.rs`'s test module).
   - **Shared fixture:** a `docker` target attached to a child box of `harness.orch.box_id()`
     (`attach_child_box`), marked probed (`set_target_probed`), the harness box declaring `docker`
     (`edit_box`), and the project level set to the target (`set_project_target`).
   - `a_local_run_names_no_driver`: the default local enqueue → `executing_box_id == None`,
     `target_box_id == here` (P1, regression).
   - `enqueue_on_a_child_box_target_names_this_box_the_driver`: with `with_runnable_kinds(TargetKind::ALL)`
     and the fixture → `target_box_id = child`, `executing_box_id = Some(here)`, target and source
     recorded. `store.claim_run(run, here, ..)` → `Admitted`; `store.claim_run(run, child, ..)` before
     that → `NotClaimable`.
   - `enqueue_checks_tags_on_the_box_the_work_runs_on`:
     - the item requires a tag only the child declares → the run is written;
     - the item requires a tag only the harness box declares → `EngineError::MissingTags` naming it, no
       run, the item `blocked`.
   - `rung_three_reads_the_agents_of_the_box_the_work_runs_on`: with `without_candidates("prd")`:
     - a lone enabled `agent_box` row on the child → resolves;
     - on the harness box only → the rung-4 refusal.
   - `a_target_refusal_comes_before_the_tags_and_rung_four`: default runnable set, a docker target at the
     project level, the item lacking a tag and with no candidate → `EngineError::TargetRefused`
     (`NotRunnable`, MOD-44), the note the target's sentence.
   - M2's engine tests stay green unedited, `enqueue_refuses_missing_tags_before_resolving` included.

**Check:**
- `cargo nextest run -p htui-orch --all-features -E 'not binary(it) and test(/enqueue|target|rung|claim/)'`
- `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^fake_conformance::/)'`
- `cargo nextest run -p htui --all-features -E 'test(/run_worker|stop_cmd/)'` (local run paths)
- clippy (both forms) and fmt on `htui-orch`.

## Validation (main thread, once both lanes join)

```bash
bash scripts/gate.sh
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Re-keying claim and poll changes who picks up a local run | Low | A local run keeps `executing_box_id = NULL` (P1), so `COALESCE` is `target_box_id` = here, exactly today's key. The regression bar forbids editing an existing claim, worker or queue assertion |
| The `enqueue_with` re-order changes which refusal an item sees | Medium | Only items with a non-runnable target (now target first) or no graph (now `NoGraph` before tags). M2's risk row anticipated it. T4 pins the new order |
| `NewRun` literal churn (78 sites, 20 files) breaks the build mid-task | High | T1 does them mechanically in one commit and runs `cargo check --workspace --all-targets --all-features` |
| Pre-enqueue tag filters (`ready_items`, Backlog) disagree with the enqueue check once a non-local kind runs | Low now, Medium at MOD-44 | Q2, P8: exact while only `local` runs. HANDOFF's M3 phase note hands it to MOD-44 |
| `edit_box` refuses to set tags on a container row | Medium | A `test-support` seam beside `set_target_probed`, named in the commit |
| A child-box run's walk uses `self.parts.box_id` for paths, agent rows and command records | n/a in M3 | No child-box run can start before MOD-44 (D1). P8 lists the walk-time reads MOD-44 must revisit |

## Claims to verify

| # | Claim |
|---|---|
| C1 | No production path writes a non-`NULL` `executing_box_id` at `create_run` today (`mem.rs:6915`, `pg/write.rs:8001` literal `NULL`); only `claim_run` and `start_chat_run` write it. |
| C2 | Both stores' `claim_run` decide claimability on `target_box_id == box_id` (`mem.rs:6957`, `pg/write.rs:8133`). |
| C3 | The claim-time tag check reads the claiming box (`mem.rs` `missing_for(&row.required_tags, box_id)`; `pg/write.rs` `b.id = $2`). |
| C4 | The slot count and the overlap set filter on `executing_box_id = box_id` and on `running`/`awaiting_approval`/`stopped` status, never `queued` (`mem.rs:7007`, `pg/write.rs:8222`, `:8247`). |
| C5 | `queued_runs_on_box` is inherent on `MemStore`/`PgStore`, dispatched by `Backend` and `WorkerHost` (not on `WriteStore`/`WorkerStore`), keyed on `target_box_id` (`mem.rs:1208`, `pg/read.rs:2664`); M3 changes no trait signature, so no `Writer`, `UsageSpy` or `SpyStore` edit. |
| C6 | No index covers `run.target_box_id`; the poll is an accepted scan (`pg/read.rs:2651`). M3 needs no migration and no cache migration. |
| C7 | The `0036_activity_event.sql` run triggers fire on status transitions only and read no `executing_box_id`. |
| C8 | No product path moves a `queued` run to `stopped` (`stop_run` is a CAS on `running` with the lease owner); the only box-less stopped run is the fixture's `RUN_8`. |
| C9 | A queued run's cancel is rowless (`htui-worker/src/runtime.rs:3347-3366`), so the `pending_commands` executing-box branch never sees a queued run. |
| C10 | `QueueRunFact` is constructed at exactly five sites: `queue.rs:841`, `mem.rs:6609`, `mem.rs:22041`, `pg/read.rs:2810`, plus none in `crates/htui`. |
| C11 | In `graph::resolve`, `box_id` reaches only rung 3 (`agent_boxes(box_id)`, `graph.rs:2052`), and `resolve_or_refused` reads the graph first through `source.resolve_graph` (`graph.rs:665`). |
| C12 | `Engine` is built only through `Engine::new` (`engine.rs:1293`); `Engine` has the fields `parts` and `walks`. |
| C13 | `CASES` is 346, pinned at `pg_conformance.rs:78` (doc `:74-77`, message `:85`) and `mem_store.rs:65`. |
| C14 | `NewRun { … }` literals: 78 across 20 files (list in the T1 file set). |
| C15 | No writer sets `target.probed_at` in this build (MOD-44/131 write it), so engine tests need P7's seam. |
| C16 | The walk-time `agent_boxes(self.parts.box_id)` (`engine.rs:5518`) is the selection's agent rows, not the candidate chain. |
| C17 | `MemStore::create_run` checks `target_box_id` against `boxes` with `references_no_row` (`mem.rs:6862`); Pg relies on the FK through `map_sqlx`. `run.executing_box_id` has an FK to `box(id)` (`0001_init.sql:456`). |
| C18 | The htui `queue_overview` fixtures' runs have driver = target (`queue_overview.rs:237-238`), so no `.snap` moves. |
| C19 | `run_hostname` (`mem.rs:2328`) and the Pg/cache run summaries (`pg/read.rs:422`, `cache/read.rs:792`) already join `COALESCE(executing_box_id, target_box_id)`. |
| C20 | The `htui stop` report reads `executing_box_id` for every non-terminal run (`stop_cmd.rs:491-523`). |
| C21 | `PgStore::claim_run` locks the run, then the claiming box `FOR UPDATE`, and no item; `create_run` locks the item, then takes FK key-share on the boxes. The extra FK adds no cycle. |
| C22 | `edit_box` with `BoxEdit.declared_tags` accepts a `container` row (else T2/T4 add a seam). |
| C23 | Task independence: T0 ⊂ {`graph.rs`}; T4 ⊂ {`engine.rs`}; T3 ∩ T4 = ∅; T1, T2 and T3 share `mem.rs`, `.sqlx` and the conformance files and are serial. |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| C1 | pending | |
| C2 | pending | |
| C3 | pending | |
| C4 | pending | |
| C5 | pending | |
| C6 | pending | |
| C7 | pending | |
| C8 | pending | |
| C9 | pending | |
| C10 | pending | |
| C11 | pending | |
| C12 | pending | |
| C13 | pending | |
| C14 | pending | |
| C15 | pending | |
| C16 | pending | |
| C17 | pending | |
| C18 | pending | |
| C19 | pending | |
| C20 | pending | |
| C21 | pending | |
| C22 | pending | |
| C23 | pending | |

## Acceptance

- [ ] T0–T4 complete, each commit green on its own check
- [ ] No existing claim, worker, queue or engine assertion edited (only count pins and mechanical literal fields)
- [ ] Gate passes on the joined lanes
- [ ] Reviewer (`rust-reviewer`) findings applied or deferred with the maintainer
- [ ] PRD milestone 3 row updated; HANDOFF phase note written (Q1/Q2 outcome, and MOD-44's ownership of the
      pre-enqueue tag filters and the walk-time box reads); validator green
