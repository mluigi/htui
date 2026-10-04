# Plan: MOD-77 Pg step writers lock the step before the run

**Source**: HANDOFF `MOD-77` (from MOD-11, `docs/decisions/mod/mod-11.md`; `R-HIS-1`, `R-ORCH-11`)
**Routed**: plan path via `/handoff-run` (0 criteria fired), accepted by the maintainer 2026-10-04. Sandbox run
`hr/MOD-77`.
**Complexity**: Small (Postgres store only: two source files, one test file, regenerated `.sqlx`; no migration, no
trait or signature change, `MemStore` untouched)
**Status**: plan, awaiting CONFIRM

## Summary

`park_step` and `promote_step` lock `FOR UPDATE OF s, r`, which is step then run. MOD-11's fenced writes follow the
same order through `step_scope`. The older fenced step writers lock the **run first** and only then the step, either
through `step_fence` (`FOR SHARE OF r`, followed by a foreign-key insert or an `UPDATE run_step`) or through an
`UPDATE run_step … WHERE EXISTS (… FOR SHARE)`. Against a park on the same step, that run → step order closes a cycle
and Postgres aborts one side with `40P01`. The fix moves every such writer to the step → run order. Writers that read
under a lock take `FOR SHARE OF s, r`. Writers that `UPDATE run_step` take `FOR NO KEY UPDATE OF s FOR SHARE OF r`,
so they never upgrade a share lock on the step.

One run → step path the item does not list, `finish_chat_run`, has to flip as well. Otherwise the fix opens a new
cycle between a chat's usage write and its close.

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: `step_fence` takes a lock mode.** A private `enum StepLock { Share, Update }` selects between two statements:
  `FOR SHARE OF s, r` (`record_commits`, and `pass_step`'s miss path through `fenced_miss`) and
  `FOR NO KEY UPDATE OF s FOR SHARE OF r` (`upsert_step_tree`, which updates `run_step.isolation_path` after the
  fence). There are two `query_scalar!` calls because the macro needs literal SQL. `NotFound`, then `Fenced`, stays
  the order. The step lock is taken before any `run_step` update in the same transaction, so a shared step lock is
  never upgraded (two upgraders would deadlock on each other).
- **D2: the four single-statement `UPDATE`s become one statement with a locking CTE.**
  `set_step_usage`, `set_step_prompt`, `finish_step` and `pass_step` become:
  ```sql
  WITH locked AS (
      SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id
       WHERE s.id = $1 [AND s.status = 'running']           -- pass_step only
         AND r.lease_owner IS NOT DISTINCT FROM $fence
         FOR NO KEY UPDATE OF s FOR SHARE OF r)
  UPDATE run_step SET … FROM locked
   WHERE run_step.id = locked.id [AND run_step.status = 'running']
  ```
  Each stays one round trip, and the miss path (`fenced_or_missing` / `fenced_miss`) is unchanged. **The clause
  order is load-bearing:** Postgres takes a joined row's locks in the order the locking clauses are written (probe
  below), so `OF s` must come first.
- **D3: `append_events`' `lease` CTE locks `FOR SHARE OF s, r`.** It writes no `run_step` column. The insert's
  foreign key takes `FOR KEY SHARE` on the step, which `FOR SHARE` already covers. The CTE is otherwise unchanged.
- **D4: the relay's `open_permission` fence locks `FOR SHARE OF s, r`.** It inserts into `step_permission`, with
  foreign keys to both rows, and updates only `step_permission`.
- **D5: `finish_chat_run` locks the step before it updates the run.** A first statement,
  `SELECT 1 FROM run_step WHERE id = $step FOR NO KEY UPDATE`, takes the step (or finds nothing). Then
  `UPDATE run` runs (0 rows → `NotFound { run }`, as today), then `NotFound { run_step }` if the first read found
  nothing, then `UPDATE run_step`. The conformance case `start_chat_run_mints_chat_rows` pins `NotFound { run }`
  first for an unknown pair, and D5 keeps that order.
- **D6: tests prove lock order deterministically, not by a 50-round race.** For each writer, a raw transaction holds
  the step `FOR UPDATE` (the first half of a park). The writer is spawned and the test waits until a backend waits on
  a lock (`wait_for_the_take`'s loop). Then, still in the raw transaction, it runs
  `SELECT 1 FROM run WHERE id = $1 FOR UPDATE NOWAIT`. That statement **must succeed**: the writer holds nothing on
  the run. With today's shapes it fails with `55P03` (probed). Finally the raw transaction rolls back and the writer
  must land. One test per writer, with a shared helper.
- **D7: what is out of scope.** These do not lock the run and the step together, so they cannot close the cycle:
  the unfenced single-row step writers (`transition_step`, `interrupt_step`, `answer_gate`, `supersede_step`,
  `record_opening`), `select_fanout` (steps only, `ORDER BY id`), `record_command_run`/`enqueue_command` (foreign-key
  `KEY SHARE` only), and the lease writers (`take_lease`, `adopt_runs`, `release_lease`, `refresh_lease`, which touch
  the run only). `close_out` locks the item and then key-shares steps, but it refuses a live run before it reaches a
  step, and a park needs a live run. A multi-step `append_events` batch spanning two steps of one run could still
  cycle with a park on the second step. The trait permits such a batch, but the only production caller,
  `Recorder::flush`, writes one step per `Recorder` (`record.rs`, field `step`). That case is recorded in the
  write-up, not fixed.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Lock order, step first | `crates/htui-store/src/pg/write.rs` `step_scope` | `FOR SHARE OF s, r` plus the doc paragraph explaining the `park_step` order |
| Fenced write + miss | `write.rs` `set_step_usage` / `fenced_or_missing` | one statement, `rows_affected() == 1`, a boxed follow-up read on a miss |
| Lock-wait proof | `crates/htui-store/tests/pg_criteria.rs` `wait_for_the_take` | poll `pg_stat_activity.wait_event_type = 'Lock'`, bounded at 10 s |
| Leased running step fixture | `pg_criteria.rs` `a_step_document_racing_a_park_never_deadlocks` | `create_run(race_run(item))` → `claim_run` → `create_step` → `transition_step` to `Running` |
| Doc tone | `write.rs` `step_scope` doc | name the deadlock, the test that pins it, and why holding the step first resolves it |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-store/src/pg/write.rs` | UPDATE | D1, D2, D3, D5 + doc comments on each writer (`step_fence`, `step_scope`'s cross-reference) |
| `crates/htui-store/src/pg/relay.rs` | UPDATE | D4 + its module doc line 9 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | D6 tests (one helper + nine cases) |
| `crates/htui-store/.sqlx/*.json` | regenerate | eight query texts change (the old entries are removed, new ones added) |
| `docs/decisions/mod/mod-77.md`, `docs/DECISIONS.md` index, `HANDOFF.md` | CREATE/UPDATE | close-out (P2) |

## Tasks

All tasks touch `write.rs` and/or `pg_criteria.rs`, so they run **serially** with one implementer (file sets
intersect).

### Task 1: lock-order tests first (red)
- **Action**: in `pg_criteria.rs`, a helper `assert_takes_the_step_first(db, run, step, write)` per D6, plus one
  `#[tokio::test(flavor = "multi_thread")]` per writer: `append_events`, `set_step_usage`, `set_step_prompt`,
  `finish_step`, `pass_step`, `upsert_step_tree` (one-row batch, so the `isolation_path` update runs),
  `record_commits`, `open_permission`, and `finish_chat_run` (on a `start_chat_run` chat, the same probe). Each also asserts the write lands after the rollback. Run them and confirm that **all nine
  fail** with the `NOWAIT` refusal on today's code.
- **Mirror**: `wait_for_the_take`, `a_step_document_racing_a_park_never_deadlocks`' fixture.
- **Validate**: `cargo test -p htui-store --all-features --test pg_criteria takes_the_step_first -- --test-threads=1`
  → 9 failed (red).

### Task 2: the fix (green)
- **Action**: D1–D5 in `write.rs` and `relay.rs`; update each writer's doc ("`FOR SHARE` on the run for
  `append_events`' reason" becomes the step → run sentence), and the `step_fence` and `step_scope` docs, which now
  agree.
- **Validate**: Task 1's tests pass. The existing fence-race tests still pass
  (`a_lease_take_committed_mid_write_fences_it`, `…_mid_settle_fences_it`,
  `a_step_document_racing_a_park_never_deadlocks`), as does the Pg conformance suite.

### Task 3: `.sqlx`
- **Action**: regenerate per `docs/hr-sandbox.md` "Changing SQL queries in a run" (scratch `htui_sqlx` DB,
  `cargo sqlx prepare -- --all-targets --all-features`), then `cargo sqlx prepare --check`.
- **Validate**: `SQLX_OFFLINE=true cargo build -p htui-store --all-features`.

### Task 4: docs and close-out
- **Action**: `docs/decisions/mod/mod-77.md` (decisions D1–D7, the probe evidence, the carried multi-step batch
  case), the DECISIONS index row, and the HANDOFF line closed per `lifecycle.md` P2.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui-store --all-features -- --test-threads=1
cargo test -p htui-core --all-features
cargo sqlx prepare --check   # from crates/htui-store, DATABASE_URL=…/htui_sqlx
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Locking clauses written in the wrong order silently restore run-first | Medium | D6 tests fail on any writer that holds the run while waiting on the step; the control probe shows `FOR SHARE OF r FOR NO KEY UPDATE OF s` holds the run |
| A shared step lock later upgraded by an `UPDATE run_step` (self-made deadlock between two writers of one step) | Low | D1/D2: every writer that updates `run_step` takes `NO KEY UPDATE` up front |
| Writers of one step now serialise on its row (usage vs. settle) | Low | They come from one walk in sequence; today's `UPDATE` already takes the same row lock, only later |
| A new run → step path elsewhere forms a fresh cycle | Low | Swept: only `finish_chat_run` (D5); triggers on `run` / `run_step` are `set_updated_at` only |
| `.sqlx` drift between this branch and others in flight | Medium | Literal-query hashing: only the changed texts move; merge is a file add/remove, no content conflict |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `park_step` and `promote_step` lock `FOR UPDATE OF s, r` | true | `write.rs:5564`, `:5419` |
| `step_scope` locks `FOR SHARE OF s, r` | true | `write.rs:254` |
| `step_fence` locks `FOR SHARE OF r` only | true | `write.rs:202-222` |
| `set_step_usage`, `set_step_prompt`, `finish_step`, `pass_step` fence with `UPDATE run_step … EXISTS (… FOR SHARE)` | true | `write.rs:1207`, `:1968`, `:4807`, `:5519` |
| `upsert_step_tree` and `record_commits` fence through `step_fence` | true | `write.rs:5152`, `:5238` |
| `upsert_step_tree` then updates `run_step` (so its fence needs `NO KEY UPDATE OF s`) | true (plan amended: D1 lock mode) | `write.rs:5204` |
| `record_commits` only inserts (foreign-key `KEY SHARE` on the step) | true | `write.rs:5251-5265` |
| `append_events` locks `FOR SHARE OF r` in its `lease` CTE and only inserts | true | `write.rs:1147` |
| The relay's `open_permission` locks `FOR SHARE OF r` and inserts / updates `step_permission` only | true | `relay.rs:130-175` |
| Today's shapes hold the run while waiting on a held step (the cycle) | true | psql probe, PG 16.15 sandbox: `NOWAIT` on `run` fails (`could not obtain lock`) for the `EXISTS` update, `step_fence` + insert, `append_events` |
| The proposed shapes hold nothing on the run while waiting on the step | true | same probe: `NOWAIT` on `run` succeeds for `FOR SHARE OF s, r` + insert, the D2 CTE, the D3 CTE |
| A joined row's locks follow the clause order (`park_step` really is s → r) | true | probe with the run held: park's shape holds the step; control `FOR SHARE OF r FOR NO KEY UPDATE OF s` with the step held holds the run |
| The D2 CTE re-checks under `READ COMMITTED`: a take committed mid-write → 0 rows; a park committed mid-pass → 0 rows; an unrelated commit → 1 row | true | psql probe, the three cases |
| `finish_chat_run` updates `run` then `run_step` in one transaction (a new cycle against D2's usage write) | true (plan amended: D5) | `write.rs:1902-1935` |
| Conformance pins `NotFound { run }` for an unknown run + step to `finish_chat_run` | true | `htui-core/src/store/conformance.rs:1930-1936`; `mem.rs:2218` checks the run first |
| No other transaction touches run then step | true | every `pool.begin()` in `pg/*.rs` read: `claim_run`, `finish_run` (run → item), `select_fanout` (steps only), `close_out` (item, refuses a live run), `write_step_document`/`add_step_note`/`propose_link`/`withdraw_link` (`step_scope`), `enqueue_command`/`claim_command` (no run lock) |
| Triggers on `run` / `run_step` touch no other row | true | migrations: only `set_updated_at` triggers |
| `append_events`' only production caller writes one step per batch | true | `htui-agent/src/record.rs:1211`, `:1224` (`Recorder::flush`; `Recorder` holds one `step`) |
| `wait_for_the_take` and the race fixture exist to mirror | true | `pg_criteria.rs:5063-5091`, `:6299` |
| The sandbox `.sqlx` recipe exists | true | `docs/hr-sandbox.md:192-210` |
| Task independence | n/a | all four tasks share `write.rs` or `pg_criteria.rs`, so they run serially |

## Acceptance
- [ ] T1 red, then T2 green: nine lock-order tests
- [ ] Existing fence-race and conformance tests still pass
- [ ] `.sqlx` regenerated, `--check` clean
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
