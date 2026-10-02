# Blueprint: MOD-37 milestone 2 - store and engine correctness

Input: `mod-37-store-engine.plan.md` (CONFIRMED 2026-10-02; open choices settled: build the park composite, R-31 recovery-side through `Unblock`, candidates keep the every-repo rule, R-32a re-deferred). Line numbers are from the tree at `ecd385d3`. Every symbol below was read in full.

## Plan claims corrected
1. **T2's read-back cannot live in the store conformance suite.** `phase_agents` and `resolve_graph` are inherent methods (`mem.rs:629`, `:662`; `pg/read.rs:1653`), on neither `ReadStore` nor `WriteStore`, and `run_case` is bound to `WriteStore` (`conformance.rs:179`). The conformance case reads the rows back through `delete_reach(...).phase_agents`, a `WriteStore` method. The per-backend read-back goes in `mem.rs` and in `pg_criteria.rs::inherent_orchestration_reads_answer_the_fixture`, which compares Postgres with MemStore.
2. **"The state a crash between `answer_gate(Rejected)` and `unpark` leaves" is not unique.** A review loop that escalates leaves the run `awaiting_approval`, the item `blocked` and the latest review `failed` + `rejected` (`gate.rs:1017-1046`). `Unblock`'s `FollowRun` then moves the item `blocked -> awaiting_approval` (`engine.rs:1607-1630`). The rows are then byte-for-byte the crash state, so no row predicate can tell them apart. Consequence, accepted and documented: `Unblock` on a followed escalation now answers `Resume`. The loop re-runs and, with nothing changed, escalates again: the run is `awaiting_approval`, the item `blocked`, there is one more escalation note, and no step is written. Today the same press answers `NotBlocked` ("parked at a gate"). Nothing pins the old answer (`runs_pg.rs:1171` stops after the first `Unblock`). **Maintainer: flag for the reviewer gate.**
3. **T5's rule as written breaks the empty scope.** "Every scope repo has a row **and** at least one has `after_hash`" is false for `run_scope = []` (an `any` over nothing). The demo fixture's vacuous `Finished` (`recover.rs:972-980`) would turn into `Reset`. The rule keeps `run_scope.is_empty() ||`.
4. **T3 is not only `WorkerStore`.** Every `WorkerStore` method is a `WriteStore` method forwarded by UFCS (`htui-core/src/store/worker.rs:1-13`, `htui-store/src/worker.rs:1-8`). So the ops go on `WriteStore` too, and `traits.rs`, `writer.rs` and both spies are in T3's set, not conditionally. "Fenced" also means `GateContext` must carry the walk's fence, which is built only in `engine.rs:5859`. See the lanes below.
5. **T4 also touches `htui-worker/src/views.rs`.** `verdicts` calls `unblock_enabled` (`views.rs:441`) and that signature changes. `hand_back_resume` (`engine.rs:2124`) is a third production reader of `resumable_park` and switches with the other two.
6. The pass's settle note today goes to an **item note** (`note_step`, `gate.rs:461`), not to `gate_note`. T3's pass op writes it to `gate_note` as well, as the plan asks, and the item note stays.

## Order and lanes
- **C0 (primary tree, before lane B forks):** `GateContext` gains `fence`. This hoists T3's only `engine.rs` edit out of T3, so the lanes stay file-disjoint.
- **Lane A (primary tree, Postgres + `.sqlx`):** T1, T2, T3, serial.
- **Lane B (worktree branched from C0, no Postgres):** T4, T5, T6, serial.
- Then lane B is merged into `hr/MOD-37`, the gates run on the merged tree, and the close-out commit follows.
- Worktree rules (memory): create the named branch first. Gortex `edit` writes to the **primary** checkout, so lane B edits with file tools on the worktree path. Expect about 10 GB of `target/` per worktree. Remove the worktree before `branch -d`.
- Implementers commit per group, and each commit is green on its own.

## Gates (scope with `-p` while iterating; workspace before hand-off)
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p <crates> --all-features -- --test-threads=1
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/orch.log; grep -n SIGABRT /tmp/orch.log   # must print nothing
```
- Integration tests need `--all-features` (testkit) or they report 0 tests as ok. Check the count.
- Pg conformance and `pg_criteria` cases skip silently without a DB (`common::demo_db()` answers `None`). The red run must show the DB was reached. `HTUI_TEST_DATABASE_URL` is preset in the sandbox (`docs/hr-sandbox.md:177`).

**`.sqlx` (`docs/hr-sandbox.md:196-210`).** Prepare uses a scratch DB, which is not the test DSN's database.
```
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features
cargo sqlx prepare --check            # "potentially unused queries" is expected
```
Byte-identical query text reuses an existing entry (memory). The expected `.sqlx` diff is given per task. Anything else in `.sqlx` is a leak, so stop.

**Conformance registration (store suite).** Each new case needs four things:
- the name appended to `CASES` (`htui-core/src/store/conformance.rs:48`);
- an arm in `run_case`'s `match` (`:179`). `run_case_accepts_every_name_in_cases` catches drift between the two;
- the count in `htui-core/tests/mem_store.rs:37` (currently 119), with its message extended;
- `EXPECTED_CASES` in `htui-store/tests/pg_conformance.rs:21`, with its doc and message.

Counts: 119, then 120 after T1, 121 after T2, 123 after T3.

`every_cross_referenced_test_name_exists` (`conformance.rs:14513`) checks every backticked snake_case name with four or more underscores in a doc comment. A bare name must exist in `conformance.rs` or `mem.rs`. A `<file>.rs::<name>` must be in a file its table knows: `pg_criteria.rs` or `box_identity.rs`. Never cite a `gate.rs` or `graph.rs` test by name from these docs.

## C0 - `GateContext` carries the walk's fence (plumbing, no behaviour)
**Files:** `htui-orch/src/gate.rs`, `htui-orch/src/engine.rs`.
**Changes:**
- `GateContext` (`gate.rs:301`) gains `pub fence: StepFence` (doc: "the walk's lease, `StepFence::Lease(parts.owner)`, which the gate's pass and park write under (MOD-37 R-5)"). Its doc goes from "six things" to "seven". Import `htui_core::store::StepFence`.
- `Engine::gate_context` (`engine.rs:5859`, a `const fn`) sets `fence: StepFence::Lease(self.parts.owner)`.
- The 7 test constructions at `gate.rs:1518, 1553, 1604, 1677, 1721, 1801, 1862` use `fence: StepFence::Unleased`. Demo `RUN_3` has no lease (`mem.rs:335`), and `StepFence` is already imported in the tests (`gate.rs:1062`).

The field is unused until T3. That is fine for a `pub` field of a `pub` struct, so clippy stays quiet.
**Gate:** `cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`; `cargo test -p htui-orch --all-features --lib gate`.
**Commit:** `refactor(mod-37): GateContext carries the walk's StepFence`. **Create lane B's worktree from this commit.**

## T1 - R-29 MemStore truncates `queued_at` (lane A)
**Files:** `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`.
**Fix:** `State::create_run` (`mem.rs:4166`): `queued_at: new.queued_at.trunc_subsecs(TIMESTAMPTZ_DIGITS)`. Imports: `chrono::SubsecRound as _` and `crate::model::TIMESTAMPTZ_DIGITS`; neither is in `mem.rs` today. sqlx encodes `chrono` as whole microseconds, truncating toward zero, so this is what Postgres stores.
**Failing test first:** the case `queued_at_ties_inside_a_microsecond_break_on_id`, mirroring `claim_run_applies_the_isolation_and_path_rules` (`conformance.rs:4796`):
1. Create repo `core`. Set `base = seam_clock()`, which is on a whole microsecond.
2. Mint items A and B. Mint the two `RunId`s up front, with `low = min` and `high = max`. Queue `low` at `base + 500ns` (scope `core` isolated `src/`) and `high` at `base` (scope `core` isolated `docs/`). The lower id gets the later nanoseconds.
3. Queue C at `base + 1s` with `core` `RepoScope::default()`.
4. Assert `store.run(low).queued_at == base` and `store.run(high).queued_at == base`.
5. Claim `low` and `high` (both `Admitted`), then park both `Running -> AwaitingApproval`. This frees the two slots, as legs A and B do.
6. Assert `claim_run(C)` is `Claim::Overlaps { with: low, rule: OverlapRule::NotIsolated }`.

It is red on MemStore today: `with: high`, and the `queued_at` asserts fail. It is green on Pg.
**MemStore tests to fix in the same commit:** `queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order` (`mem.rs:9600`). Its `let early = Utc::now();` (`:9619`) is compared, untruncated, inside `expected` (`:9685`), so truncate it. Audited, with no `queued_at` equality, so expected unaffected:
- `mem.rs` tests' `graph_run` (`:9339`);
- `htui-orch/tests/review_loop.rs:70`;
- `htui/src/run_worker.rs:1416`;
- `htui-worker/src/runtime.rs:3487, 3788, 3857`.

Production callers pass a clock instant, which is already truncated (`clock.rs:42-44`).
**Gate:** `cargo test -p htui-core -p htui-store --all-features -- --test-threads=1`; `cargo test -p htui-worker -p htui --all-features -- --test-threads=1` (MemStore consumers).
**Commit:** `fix(mod-37): MemStore keeps queued_at to the microsecond, as Postgres does (R-29)`. No `.sqlx` change.
**Must NOT touch:** `claim_run`, `overlapping_runs` (already `(queued_at, id)`, `mem.rs:813, 4259`), and any production `queued_at` writer.

## T2 - R-6 the `phase_agent` writer (lane A)
**Signature** (`traits.rs`, beside `create_phase` `:863`):
```rust
/// Inserts `agents` as `phase`'s candidate rows, all or nothing; an empty slice writes nothing and checks nothing.
async fn create_phase_agents(&self, phase: PhaseId, agents: &[PhaseAgent]) -> Result<()>;
```
- **Insert-only.** `override_graph` writes to fresh phases, and replace semantics would cost a second statement.
- **Refusals,** all `StoreError::Constraint`:
  - a row whose `phase_id != phase` (new helper `row_names_another_phase(table, row, phase)` beside `row_names_another_step` `traits.rs:2193`, exported in `store/mod.rs:16-32`);
  - a phase that names no row;
  - an agent that names no row;
  - a taken `(phase_id, position)`, against stored rows or within the batch.
- Rows may come in any order; readers sort by `position`.

**MemStore:**
- `State` gains `phase_agents: BTreeMap<(PhaseId, i32), PhaseAgent>`, the primary key `0001_init.sql:254-260`; the BTreeMap order is position order per phase. Initialise it empty where `State` is built from `DemoData` (`mem.rs:~335`); the fixture has no rows.
- `State::create_phase_agents` validates everything before the first insert, using `references_no_row` and `already_exists` (`traits.rs:2112, 2118`). `self.phase(phase)` (`mem.rs:2116`) and `self.agents` (`:189`) are the lookups.
- `MemStore` gets the `WriteStore` impl beside `create_phase` (`:6489`), through `self.write`.

**Readers fed:**
- `MemStore::phase_agents` (`mem.rs:629`): the rows of `phase`.
- `State::resolve_graph` (`:5146-5172`): `agents:` from the map, replacing `Vec::new()`.
- `State::project_reach` (`:3714`): count rows whose phase is in `phases`.
- `State::delete_project` (`:3916`): `self.phase_agents.retain(|(phase, _), _| !gone.phases.contains(phase))`.
- Docs at `:623-627, :3631, :5166`.

**Pg** (`pg/write.rs`, beside `create_phase` `:2704`): return `Ok(())` early on an empty slice, check the phase match in Rust, then one `query!`:
```sql
INSERT INTO phase_agent (phase_id, position, agent_id, model)
SELECT $1, t.position, t.agent_id, t.model
  FROM UNNEST($2::int4[], $3::uuid[], $4::text[]) AS t(position, agent_id, model)
```
`23503`/`23505` map to `Constraint` (`error.rs:17-48`).

**Writer dispatch:** `writer.rs` beside `create_phase` (`:686`), using `match self { Memory(store) => store.create_phase_agents(..), Online(pg) => pg.create_phase_agents(..) }`.

**Spies:** `UsageSpy` (`htui-agent/src/conformance.rs:~975`) and `SpyStore` (`htui-agent/tests/recorder.rs:~701`) forward with `self.inner.create_phase_agents(phase, agents).await`. The method is not on `WorkerStore`: `override_graph` keeps its `WriteStore` bound. Fix the doc at `htui-core/src/store/worker.rs:98` ("seven" becomes "eight extra methods").

**`override_graph` call site** (`graph.rs:~502-512`): after each `create_phase`, write
`store.create_phase_agents(id, &row.agents.iter().map(|agent| PhaseAgent { phase_id: id, ..agent.clone() }).collect::<Vec<_>>()).await?`
Then rewrite the doc at `graph.rs:398-400`: the clone now carries `phase_agent`, and re-override is still owed.

**Pinned divergence tests:**
- `mem.rs` `the_eleven_inherent_reads_answer_from_the_fixture` (`:11138`): the assertions at `:11148-11155` and `:11187-11190` stay true, since the fixture seeds no `phase_agent`. Only their messages change, to "the fixture seeds no phase_agent row".
- `pg_criteria.rs` `inherent_orchestration_reads_answer_the_fixture` (`:3670`):
  - replace the raw `sqlx::query!` seeding (`:3934-3946`) with `create_phase_agents(PHASE_HTUI_IMPLEMENT, [(1, AGY, "opus"), (0, CLAUDE, "sonnet")])` on **both** `db.store` and `mem`;
  - assert `phase_agents` is equal on the two stores and in position order, and that `resolve_graph`'s implement phase carries the same two on both;
  - rewrite the doc at `:3658-3669` ("no longer divergent").
- Leave the raw string `sqlx::query` insert in `every_cascade_table_loses_exactly_what_the_report_names` (`:3122`); it is no macro and has no `.sqlx` entry.

**Docs only:**
- `ResolvedPhase.agents` (`model/kind.rs:316`);
- `DeleteReach` (`traits.rs:2356-2362`);
- `PgStore::phase_agents` (`pg/read.rs:1647-1648`);
- `fake.rs:10-14` and `:923-925`;
- `TestSource` (`graph.rs:895-899`).

**Failing tests first:**
1. Conformance case `phase_agents_are_written_whole_and_counted`:
   - Write two rows out of order to `PHASE_HTUI_IMPLEMENT`; `delete_reach(DeleteTarget::Project(PROJECT_HTUI))?.phase_agents == 2`.
   - Each refusal (another phase's row, unknown phase, unknown agent, taken position, and a batch of one good row plus one taken) is `Constraint`, and the count stays 2.
   - An empty slice on an unknown phase is `Ok`.
   - The doc cross-refers `written_phase_agents_answer_every_reader` and `pg_criteria.rs::inherent_orchestration_reads_answer_the_fixture`.
   - It fails to compile first. Then it is red on MemStore (count 0) until `project_reach` counts.
2. `mem.rs` `written_phase_agents_answer_every_reader`: `phase_agents` is in position order, `resolve_graph` carries the rows, and `delete_project` takes them.
3. `graph.rs` `override_clone_carries_phase_agents`, beside `override_clone_carries_phase_attachments_and_marks_itself` (`:1588`):
   - write agents on `PHASE_HTUI_IMPLEMENT`;
   - `override_graph(&store, &TestSource::claude(&store), &item)` (`TestSource::resolve_graph` reads MemStore);
   - the cloned implement phase's `store.phase_agents(..)` equals the source rows re-keyed. Every other cloned phase has none.

**Behaviour check, MemStore-backed orch tests:** none changes. No fixture or orch path writes `phase_agent`: `override_graph` has no production caller. `FakeGraphSource` (`fake.rs:1014`) and `TestSource` (`graph.rs:928`) answer rung 1 from their own maps, and `impl GraphSource for MemStore` (`fake.rs:881`) still answers empty for the fixture.

**`.sqlx`:** one added (the `UNNEST` insert) and one deleted (pg_criteria's seeding insert).

**Gate:** T1's gate; `cargo test -p htui-orch --all-features --lib graph`; `cargo test -p htui-agent --all-features -- --test-threads=1` (the spies compile); `cargo sqlx prepare --check -- --all-targets --all-features`.

**Commits:**
- (a) `feat(mod-37): WriteStore::create_phase_agents; MemStore holds phase_agent (R-6)`: the store, the spies, both pinned tests, `.sqlx`.
- (b) `feat(mod-37): override_graph copies each phase's agents onto its clone (R-6)`: `graph.rs` and the `fake.rs` docs.

**Must NOT touch:** the cache and `Backend` (`phase_agent` is not mirrored, `backend.rs:424`), `GraphSource`, `WorkerStore`'s method list, and `graph::candidates`.

## T3 - R-5 the pass writes `skipped`; the park is one transaction (lane A)
**Placement:**
- Both ops go on `WriteStore` (`traits.rs`, beside `promote_step` `:1403`) **and** on `WorkerStore` (`htui-core/src/store/worker.rs`, the write block `:168-312`).
- They are forwarded by UFCS in the three `WorkerStore` impls: MemStore (`worker.rs:471`), PgStore (`htui-store/src/worker.rs:79`) and `Writer` (`:358`).
- They are dispatched in `writer.rs` (beside `promote_step` `:1037`) and forwarded in both spies (beside `promote_step` `conformance.rs:1195`, `recorder.rs:920`).
- Update the method counts in the `worker.rs` module doc (`:15-19`).

```rust
/// R-5 / ANA-2 §4.2 "done + skipped": `running -> done`, `gate_outcome = 'skipped'`, `gate_note` set when `note` is `Some`
/// (kept otherwise), the first `finished_at`. One statement, under `fence`. `Ok(false)`: the step is not `running`.
async fn pass_step(&self, fence: StepFence, step: StepId, note: Option<&str>, at: DateTime<Utc>) -> Result<bool>;
/// R-5, `promote_step`'s shape: step and run `running -> awaiting_approval`, item `in_progress -> awaiting_approval`, one transaction.
async fn park_step(&self, fence: StepFence, step: StepId) -> Result<ParkOutcome>;

/// What `park_step` found. A refusal writes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkOutcome { Parked, StepMoved, RunMoved }   // traits.rs beside StepFence :2327, exported in store/mod.rs
```
**Check order (both ops, both stores):** `NotFound { entity: "run_step" }`, then `Fenced { step }`, then the step's status, then (park only) the run's status. The order is load-bearing: the stale test's `RUN_3` is `done` *and* its step is `failed`, and it expects the step refusal.

The item keeps D17's reading inside the transaction. An item that is not `in_progress` is left alone and the answer is still `Parked`; a chat run (`item_id = NULL`) skips the item. `park_step` takes no instant: no timestamp moves on `running -> awaiting_approval` (`pg/write.rs:4377-4382`), and `updated_at` is the trigger's (`0001_init.sql:579`).

**Pg** (`pg/write.rs`, beside `promote_step` `:5033`). `pass_step`:
```sql
UPDATE run_step
   SET status = 'done', gate_outcome = 'skipped', gate_note = COALESCE($2, gate_note),
       finished_at = COALESCE(finished_at, $3)
 WHERE id = $1 AND status = 'running'
   AND EXISTS (SELECT 1 FROM run r WHERE r.id = run_step.run_id
                 AND r.lease_owner IS NOT DISTINCT FROM $4 FOR SHARE)
```
Bind `fence.owner()` as in `finish_step` (`:4425`). On zero rows, `Box::pin` a new helper `async fn fenced_miss(pool, step, fence) -> Result<bool>`. It acquires a connection, runs `step_fence(&mut conn, step, fence).await?` (`:193`), which answers `NotFound`, then `Fenced`, and reuses that query's `.sqlx` entry. If both pass, it answers `Ok(false)`. The boxing follows `fenced_or_missing`'s stack note (`:216-223`, `htui/tests/runs_pg.rs`).

`park_step`:
- `pool.begin()`.
- `SELECT s.status, r.id, r.status, r.item_id, r.lease_owner AS "lease_owner?" FROM run_step s JOIN run r ON r.id = s.run_id WHERE s.id = $1 FOR UPDATE OF s, r`.
- Decide in the order above. Return before any `UPDATE`; the dropped transaction rolls back.
- `UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1` (a new entry).
- The run and item `UPDATE`s **copied verbatim from `promote_step`** (`:5077-5097`), so their `.sqlx` entries are reused.
- `commit`, then answer `Parked`.

**MemStore** (one `&mut self` section each, beside `State::interrupt_step` `mem.rs:4562` and `State::promote_step` `:4905`; impls beside `MemStore::promote_step` `:6765`):
- `pass_step`: `steps.get_mut` (`NotFound`), `Self::fence_holds(&self.lease_owners, row, fence)?` (`:1557`, a disjoint borrow as in `finish_step` `:4545`), `status != Running` → `Ok(false)`. Otherwise set `Done`, `Some(Skipped)`, the note if `Some`, `finished_at.or(Some(at))`, `updated_at = now`.
- `park_step`: `require_step`, then fence, step status, `require_run` and its status, all **before** any write, then the three moves. The item move is through `self.transition` guarded by `status == InProgress`, exactly as `promote_step` (`:4934-4940`).

**Gate** (`gate.rs`):
- `apply`'s pass arm (`:394-398`) keeps `note_step`. It then runs `if !ctx.store.pass_step(ctx.fence, step.id, note.as_deref(), now).await? { return Err(stale_step(ctx.run.id, step.id, Running, Done)) }`.
- `park` (`:488-510`): `match Box::pin(ctx.store.park_step(ctx.fence, step.id)).await? { Parked => .., StepMoved => Err(stale_step(.., Running, AwaitingApproval)), RunMoved => Err(stale_run(ctx.run.id, Running, AwaitingApproval)) }`. The `Box::pin` keeps the Pg transaction's future out of `apply`'s frame.
- The `never` + rejected arm (`move_step` + `reject_step`) is unchanged.
- A `Fenced` answer propagates as `EngineError::Store(Fenced)`, which the walk already treats as a lost fence (`engine.rs:6196`, `:3369`).
- Docs: `apply` (`:360-375`) gets the new park/done rows (H-9 and H-10 closed by R-5; D96 stays for rows written before). Update `note_step` (`:449-458`) and `park` (`:487`).

**Assertions that flip** (orch `conformance.rs`; the compiler finds none):
- `live_run_ignores_a_gate_edit` `:1437-1445`: `research` and `verdict` move to `Some(GateOutcome::Skipped)`, and the message is rewritten.
- `a_never_gate_rejection_loops_the_review` `:2191-2195`: `(3,2)` moves to `Some(Skipped)`.
- `on_failure_passes_ok_and_parks_failed` `:2317-2322`: `(0,1)` moves to `Some(Skipped)`.

Unchanged, because they are parks, interrupts or human answers: `:1328` (`feat_walks_end_to_end`, all approved), `:1594`, `:1675`, `:2166`, `:2269`, `:4255`, `:4635`, `:5568`, `:5696`, `:5798`, `:6564`, `engine.rs:8078`, `gix_isolator.rs:2234`. The D96 case doc (`conformance.rs:4982-4983`) becomes "a park written before R-5's one-transaction writer". Its body stages the half park by hand and stays green.

**`a_stale_compare_and_set_in_the_gate_is_a_stale_write` (`gate.rs:1838`)** does **not** assert the partial state. Its last segment (`:1937-1948`) checks only the error, but its comment describes it ("whose step move lands but whose run move does not"). T3 rewrites that comment and adds `assert_eq!(status_of(&store, live.id).await, StepStatus::Running, "a refused park writes nothing")`. This is **the red gate-level test**: today the step is `AwaitingApproval`. Segments 1-4 keep their exact `StaleWrite` rows, given the check order above.

**Failing tests first:**
1. Conformance `pass_step_writes_done_and_skipped_under_the_fence`, using `leased_step` (`conformance.rs:5757`) plus `Pending -> Running`:
   - the pass answers `true`, and the row is `(Done, Some(Skipped), Some(note), Some(at))`;
   - a second pass answers `false` and the row is unchanged;
   - `None` keeps the note `NULL`;
   - a pending step answers `false`;
   - `Lease(b)` and `Unleased` on A's run are `Fenced`, with nothing written;
   - an unknown step is `NotFound`.
2. Conformance `park_step_moves_step_run_and_item_or_nothing`:
   - `Parked` moves all three;
   - the run moved first (`transition_run` to `AwaitingApproval`) gives `RunMoved`; the step stays `Running` and the item `InProgress`;
   - a step not running gives `StepMoved`, with nothing written;
   - the item moved first (`InProgress -> Blocked`) still gives `Parked` (D17), and the item stays `Blocked`;
   - a fenced park and an unknown step are refused as for the pass.
3. The gate assertion above.

**`.sqlx`:** three added: the pass `UPDATE`, the park `SELECT`, and the park step `UPDATE`.

**Gate:** T2's gate; the orch suite (`--no-fail-fast`, SIGABRT grep); `cargo test -p htui --all-features -- --test-threads=1` (it walks on Pg in `runs_pg.rs`, the debug-stack canary); `cargo insta test -p htui --all-features --check -- --test-threads=1` (the Runs pane's gate cell `runs.rs:1043` shows `skipped` for a pass; expect zero diffs).

**Commits:**
- (a) `feat(mod-37): fenced pass_step and park_step store ops (R-5)`: traits, worker, both stores, `Writer`, the spies, two cases, `.sqlx`.
- (b) `fix(mod-37): the gate passes with skipped and parks in one transaction (R-5)`: `gate.rs` and orch `conformance.rs`. The body states the red run.

**Must NOT touch:** `engine.rs` (C0 did the only edit), `transition_step`, `answer_gate`, `reject_step`, `recover_run`'s D96 branch, and `promote_step`.

## T4 - R-31 `Unblock` resumes a rejection a crash left parked (lane B)
**Predicate** (`status.rs`, beside `resumable_park` `:313`, which stays as is):
```rust
/// MOD-37 R-31: the `failed` + `rejected` latest step of a parked run, which `answer_gate(Rejected)` wrote and whose unpark a
/// crash cut off (also what a followed review-loop escalation looks like: see the blueprint, claim 2).
pub fn crashed_rejection<'s>(cursor: &Cursor, steps: &'s [RunStep]) -> Option<&'s RunStep>   // Cursor::Rest { status: Failed } whose step has gate_outcome == Some(Rejected)
/// Blueprint D196, widened by R-31: the one predicate every resume reader reads.
pub fn resumable(cursor: &Cursor, steps: &[RunStep]) -> bool { resumable_park(cursor) || crashed_rejection(cursor, steps).is_some() }
```
D196 holds. All three production readers move from `resumable_park` to `resumable` and none open-codes it:
- `unblock_enabled` through its callers;
- `walk_resumed_from` (`engine.rs:3057`);
- `hand_back_resume` (`engine.rs:2131`). Without it, `Tails::HandBack` would answer `Resume` and silently not unpark. After its unpark, the adopter's `recover_run` reaches D131's `settle_failed` on its own (`engine.rs:2453-2464`).

**`unblock_enabled`** (`command.rs:1169`): `runs: &[(Run, bool)]`. The bool is `status::resumable(&cursor, &steps)`, which no other predicate computes; the doc says so. Arm 3 becomes `(AwaitingApproval, Some((run, true)), _)`. Callers:
- `Engine::unblock_case` (`engine.rs:2312-2326`), which already reads the steps;
- `htui-worker/src/views.rs` `verdicts` (`:322-444`), which pushes `(run.clone(), resumable(&cursor(&snapshot, steps), steps))`.

The guard needs the steps and its callers already hold them; `Cursor` alone cannot see `gate_outcome`.

**`walk_resumed_from`** (`engine.rs:3051`):
1. Compute `let at = cursor(..)` once.
2. `if resumable(&at, &steps)`, unpark with D180's stale check, unchanged (`:3060-3066`).
3. Then, **before** the frontier reconcile, `if let Some(rejected) = crashed_rejection(&at, &steps)`:
   - re-read the run (now `running`);
   - `match Box::pin(self.settle_failed(&row, &snapshot, rejected)).await? { Some(rest) => return Ok(rest), None => return self.run_to_rest(run).await }`.

`settle_failed`'s first rule (`:2738-2740`) is `rejection` (`:935`): `finish_run(failed, Rejected)` + cleanup for a non-loopable phase, `review_loop` for a review. That is exactly what `answer_guarded`'s tail (`:892-894`) would have run. A rejection is never merged, so the frontier is skipped. The `Box::pin` is needed because `settle_failed` holds `admit`'s and `review_loop`'s futures, and `walk_resumed_from` sits under `resume`, under `Unblock`, under `every_case_name_dispatches`.

**Failing tests first:**
1. `status.rs` `a_rejection_a_crash_left_parked_is_resumable`, beside `only_a_crashed_command_s_park_is_resumable` (`:522`):
   - `Rest{Failed}` over a `rejected` step is resumable, and `crashed_rejection` names it;
   - the same step with `gate_outcome: None` (budget or interrupt) is not;
   - `Rest{AwaitingApproval}` is not.
   - The doc names the escalation twin.
2. `command.rs` `unblock_resumes_a_run_parked_over_a_crashed_rejection`, beside `unblock_names_its_three_cases_and_what_holds_the_item` (`:2117`), which is rewritten to `(run, bool)` pairs:
   - `resumable(..)` over hand-built rows feeds `unblock_enabled` and gives `Resume(run)`;
   - with `gate_outcome: None`, it gives `NotBlocked` with `run_waits_at_a_gate`.
3. `engine.rs` `a_crash_between_a_rejection_and_the_unpark_is_resumed_by_unblock`, beside `walk_resumed_stops_on_a_stale_unpark` (`:12243`):
   - `Harness::new()`, then `started(&harness)` (`:8356`, which parks `FEAT-3` at `prd`);
   - `harness.orch.store.answer_gate(prd, Rejected, Some("not like this"), now)`, which is the crash: run and item stay `awaiting_approval`;
   - `let other = harness.orch.restarted()`;
   - `super::dispatch_fake(&other, Command::Unblock { item: HTUI_FEAT_3 })` gives `Unblocked { case: Resume(run), rest: Some(Rest { run: Failed, position: Some(0), failure: Some(RunFailure::Rejected { phase: "prd" }) }) }`;
   - the run row is `Failed`, the item `Failed` (D7 mirror after the unpark), and the step is still `failed` + `rejected` with its note.
   - Red today: `NotBlocked` ("parked at a gate").

**Gate:** the orch suite with the SIGABRT grep; `cargo test -p htui-worker --all-features -- --test-threads=1`.
**Commit:** one, because the predicate and the walk must land together: `fix(mod-37): Unblock resumes a run a crash left parked over a rejection (R-31)`. Splitting it would let `Unblock` unpark onto a `running` run resting on a failed step, which is D131's stuck state.
**Must NOT touch:** `cursor`, `resumable_park`'s body, `answer_guarded`, `settle_failed`, `rejection`, `gate.rs`, the orch `conformance.rs` (lane A's), and `GateContext`.

## T5 - R-30 `classify` adopts a partial capture (lane B)
**Change** (`recover.rs:242-246`):
```rust
let row = |repo: &RepoId| commits.iter().find(|row| row.repo_id == *repo);
let changed = |repo: &RepoId| row(repo).is_some_and(|row| row.after_hash.is_some());
let captured = match kind {
    StepKind::Candidate => run_scope.iter().all(changed),                       // settled: the every-repo rule
    StepKind::Plain | StepKind::Judge => run_scope.iter().all(|repo| row(repo).is_some())
        && (run_scope.is_empty() || run_scope.iter().any(changed)),
};
```
Why one `Some` proves the batch: stage 2's `record_commits(before)` gives every scope repo a row (`engine.rs:3419`). Stage 5's capture batch lands in one transaction (`pg/write.rs:4865`), so one `after_hash` means it landed. All-`None` stays `Reset`: a capture that never landed and one that changed nothing look alike, and retrying is safe. Rewrite the doc at `:228-232`.

**Tests** (`recover.rs` tests, using `running()`/`tree`/`commit` `:861-880`):
- New `classify_finished_when_the_capture_changed_some_repos`: `(Some, None)` over `[first, second]` with output gives `Finished`. Red today.
- `classify_not_finished_with_one_repo_missing` (`:993`): drop the first case and its "capture died between them" comment. Keep "no row" and "stranger" as `Reset`, and add all-`None` as `Reset`.
- New `classify_a_candidate_keeps_the_every_repo_rule`: a fanned `implement` with `fan_out = 3` and a candidate at index 1. `(Some, None)` gives `(Candidate, Reset)`; `(Some, Some)` gives `Finished`.
- `classify_finished_by_every_after_hash`'s empty-scope half (`:972-980`) must stay green; this is the vacuous guard.

**Gate:** `cargo test -p htui-orch --lib recover`, then the orch suite plus SIGABRT. `FakeIsolator::capture` (`fake.rs:535`) is what the crash cases record, so a case that relied on a partial capture reading `Reset` would surface there.
**Commit:** `fix(mod-37): classify adopts a step whose capture changed only some repos (R-30)`.

## T6 - R-32b `part_way` keeps `Io` (lane B)
**Change:** `real.rs:165` becomes `IsolateError::Io(io) => IsolateError::Io(std::io::Error::new(io.kind(), format!("{io}; {named}")))`. In the doc at `:151-152`, "an `Io` failure has no text of its own to extend and is carried as `Git`" becomes "an `Io` failure keeps its kind (`git::is_lock_error` reads it), with the text appended".
**Test first:** `part_way_keeps_an_io_error_as_io`, beside `already_reset_names_every_row_and_where_it_was` (`:4961`):
- `part_way(IsolateError::Io(io::Error::new(AlreadyExists, "held")), &[(repo, "/src/core".into(), "h1".into(), "b1".into())])` matches `Io(e)`, with `e.kind() == AlreadyExists` and `e.to_string()` ending in `already_reset(..)`;
- `crate::isolate::git::is_lock_error(&err)` is true (`git.rs:1144`);
- an empty `done` returns the error untouched.

`a_reset_that_fails_part_way_names_the_rows_already_reset` (`:4974`) asserts `"git: "` on a `Git` failure and is unaffected.
**Gate:** `cargo test -p htui-orch --lib isolate`. **Commit:** `fix(mod-37): part_way keeps an Io error as Io (R-32b)`.

## Touched-file sets and intersections
| Group | Set | Versus the plan |
|---|---|---|
| C0 | orch `gate.rs`, `engine.rs` (`gate_context` only) | new: hoisted from T3 |
| T1 | core `mem.rs`, `conformance.rs`, `tests/mem_store.rs`; store `tests/pg_conformance.rs` | same |
| T2 | core `traits.rs`, `mod.rs`, `mem.rs`, `conformance.rs`, `worker.rs` (doc), `model/kind.rs` (doc), `tests/mem_store.rs`; store `pg/write.rs`, `pg/read.rs` (doc), `writer.rs`, `tests/pg_conformance.rs`, `tests/pg_criteria.rs`, `.sqlx`; agent `conformance.rs`, `tests/recorder.rs`; orch `graph.rs`, `fake.rs` (doc) | adds `mod.rs`, `worker.rs`, `kind.rs`, `pg/read.rs`, `fake.rs` (docs or exports) |
| T3 | core `traits.rs`, `mod.rs`, `worker.rs`, `mem.rs`, `conformance.rs`, `tests/mem_store.rs`; store `pg/write.rs`, `worker.rs`, `writer.rs`, `tests/pg_conformance.rs`, `.sqlx`; agent spies; orch `gate.rs`, `conformance.rs` | WriteStore files unconditional; `engine.rs` moved to C0 |
| T4 | orch `status.rs`, `command.rs`, `engine.rs`; worker `views.rs` | adds `htui-worker/src/views.rs` |
| T5 | orch `recover.rs` | same |
| T6 | orch `isolate/real.rs` | same |

The intersections:
- Lane A (T1-T3) shares `mem.rs`, `conformance.rs`, `traits.rs`, `pg/write.rs`, `.sqlx` and the spies, so it stays serial, as planned.
- Lane B (T4-T6) and lane A share **no file once C0 is committed before the fork**. Without C0, T3 and T4 would both edit `engine.rs`.
- Hidden couplings to check on the merged tree:
  - T3 changes what a pass writes and T4/T5 change recovery, so run the full orch suite after the merge;
  - T4's `views.rs` change and T3's pane `skipped` cell both reach `htui` snapshots, so run `cargo insta test --check` on the merged tree.

## Stack headroom (`every_case_name_dispatches`, orch `conformance.rs:6829`, near 2 MiB)
- New awaits: T3's `park_step` gets `Box::pin` at `gate::park`, and the Pg pass miss branch is boxed in `PgStore::pass_step`. T4's `settle_failed` gets `Box::pin` in `walk_resumed_from`.
- The pass replaces one CAS future with one op future, so `apply` does not grow.
- T1, T2, T5 and T6 add no engine future.
- Every orch gate is run with `--no-fail-fast` and the SIGABRT grep. Clean up any orphaned test process after an abort (memory).

## Close-out (after the merge, one commit)
`docs(mod-37): M2 close-out, R-32a re-deferred`:
- HANDOFF R-lines: R-5, R-6, R-29, R-30, R-31 and R-32b closed. R-32a is re-deferred to whoever next changes the park's note format; the reason text was never persisted (`engine.rs` `never_reset`; `interrupt_step` stores `NOT_RESET`).
- Record claim 2's escalation re-run as a known, accepted behaviour (or a follow-up if the reviewer objects).
- PRD row update.

Run the reviewer gate over the full change set before this commit. Then run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.
