# Blueprint: MOD-77 Pg step writers lock the step before the run

**Contract**: `.claude/plans/mod-77-step-lock-order.plan.md` (CONFIRMED). Decisions D1-D7 are fixed; this file turns
them into build-ordered instructions for **one serial implementer** on `hr/MOD-77`.
**Scope**: `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/pg/relay.rs`,
`crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/.sqlx/`. No migration, no trait change, `MemStore`
untouched, `pg_conformance.rs`'s `EXPECTED_CASES` stays 148.

## Architecture: step → run lock order for the fenced Pg step writers

### Design decisions (added beyond the plan)

- **B1: `StepLock` is a private `Copy` enum and `step_fence` gets a fourth argument.** No other signature changes.
  Every `step_fence` caller is in `write.rs` (`upsert_step_tree`, `record_commits`, `fenced_miss`; confirmed with
  the callers graph), so no `verify_change` fallout outside the file.
- **B2: `fenced_miss` uses `StepLock::Share`.** It runs on a pooled connection outside a transaction, so its lock
  lasts one statement. It only makes the diagnostic read wait for a writer that holds the step or the run to commit,
  and no `UPDATE` follows that could upgrade it. `Update` would conflict with every other share-locking writer of
  the step for no gain. `Share` keeps the step → run order, so this read can't join a cycle either.
- **B3: `upsert_step_tree` always takes `StepLock::Update`, even for an empty batch.** D1 says so. An empty batch
  could take `Share` (`!trees.is_empty()` is exactly "the `UPDATE` runs"), but that is a second branch for no
  observed benefit. Not taken. Recorded here so a reviewer doesn't re-derive it.
- **B4: `pass_step` re-checks `status = 'running'` in the outer `UPDATE` too.** The plan brackets it as optional. The
  CTE already holds the step, so the outer check can't change the answer. It stays because it makes the
  compare-and-set readable on its own line.
- **B5: `finish_chat_run` keeps its `closed_step == 0` guard** after D5's lock. A step we hold `FOR NO KEY UPDATE`
  can't be deleted (a delete needs `FOR UPDATE`), so the guard is unreachable. It is kept anyway for a minimal diff,
  and it is the statement's own contract.
- **B6: the test helper proves the writer waits on *the holder*, not on some lock.** It filters `pg_stat_activity`
  with `$holder_pid = ANY(pg_blocking_pids(pid))` as well as `wait_event_type = 'Lock'` and
  `datname = current_database()`. It also takes the `NOWAIT` result, rolls back and joins the writer **before** it
  asserts. A red run then fails on the `NOWAIT` assertion with the SQLSTATE in the message, and the writer is never
  left hanging on a dropped transaction.
- **B7: test SQL uses the runtime `sqlx::query` / `sqlx::query_scalar`, never the macros.** The test commit then
  compiles offline (`.cargo/config.toml` sets `SQLX_OFFLINE = "true"`) with no `.sqlx` change, and only source
  queries move in `.sqlx`.
- **B8: a shared fixture `leased_running_step(db)`** for eight of the nine tests, following the
  `a_step_document_racing_a_park_never_deadlocks` round body. That test is not refactored onto it, which is out of
  scope.

### Files to modify

| File | Changes | Priority |
|---|---|---|
| `crates/htui-store/tests/pg_criteria.rs` | 2 imports, `leased_running_step`, `assert_takes_the_step_first`, nine `*_takes_the_step_first` tests; later one stale doc line (§3.9) | 1 (red), 2 (doc) |
| `crates/htui-store/src/pg/write.rs` | `StepLock`, `step_fence`, `fenced_miss`, `step_scope` doc, `append_events`, `set_step_usage`, `set_step_prompt`, `finish_step`, `pass_step`, `upsert_step_tree`, `record_commits`, `finish_chat_run` | 2 |
| `crates/htui-store/src/pg/relay.rs` | module doc lines 8-10, `open_permission` fence SQL and its comment | 2 |
| `crates/htui-store/.sqlx/*.json` | regenerated: 7 entries removed, 9 added | 3 |
| `docs/decisions/mod/mod-77.md`, `docs/DECISIONS.md`, `HANDOFF.md` | close-out (plan Task 4) | 4 |

No files are created except the close-out write-up.

### Data flow (the invariant)

Each fenced step writer now acquires row locks in the order **`run_step` → `run`**, the order `park_step` /
`promote_step` already use (`FOR UPDATE OF s, r`). Where a writer later updates `run_step` in the same transaction or
statement, it takes the step lock as `FOR NO KEY UPDATE`, never `FOR SHARE` followed by an upgrade. While a writer
waits on a step, it holds nothing on that step's run. That is what the D6 tests observe through `NOWAIT`.

---

## 1. Exact source changes (`write.rs`, `relay.rs`)

The SQL below is **literal**. For `"…\`-continuation strings, Rust drops the newline and the next line's leading
whitespace, so the space before each `\` is load-bearing. The `.sqlx` hash is the literal string (memory
`sqlx-offline-hash-is-literal-query`).

### 1.1 `StepLock` (new, directly above `step_fence`, `write.rs:~196`)

```rust
/// MOD-77 plan D1: the lock [`step_fence`] takes on the step's row. The run's is always
/// `FOR SHARE`, and is always taken **after** the step's.
#[derive(Clone, Copy)]
enum StepLock {
    /// `FOR SHARE OF s, r`: the caller reads the step and inserts rows whose foreign key names
    /// it, and never updates it (`record_commits`, [`fenced_miss`]).
    Share,
    /// `FOR NO KEY UPDATE OF s FOR SHARE OF r`: the caller goes on to `UPDATE run_step`
    /// (`upsert_step_tree`'s `isolation_path`), so it takes the update lock up front. A share
    /// lock upgraded later would deadlock two such writers of one step against each other.
    Update,
}
```

### 1.2 `step_fence` (`write.rs:197-222`): signature and body

```rust
async fn step_fence(
    conn: &mut PgConnection,
    step: StepId,
    fence: StepFence,
    lock: StepLock,
) -> Result<()> {
    // Two statements, not one with a spliced clause: `query_scalar!` needs literal SQL.
    let owner = match lock {
        StepLock::Share => {
            sqlx::query_scalar!(
                r#"SELECT r.lease_owner AS "lease_owner?"
                     FROM run_step s JOIN run r ON r.id = s.run_id
                    WHERE s.id = $1
                      FOR SHARE OF s, r"#,
                step.as_uuid(),
            )
            .fetch_optional(conn)
            .await
        }
        StepLock::Update => {
            sqlx::query_scalar!(
                r#"SELECT r.lease_owner AS "lease_owner?"
                     FROM run_step s JOIN run r ON r.id = s.run_id
                    WHERE s.id = $1
                      FOR NO KEY UPDATE OF s FOR SHARE OF r"#,
                step.as_uuid(),
            )
            .fetch_optional(conn)
            .await
        }
    }
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound {
        entity: "run_step",
        id: step.to_string(),
    })?;
    if owner == fence.owner() {
        Ok(())
    } else {
        Err(StoreError::Fenced { step })
    }
}
```

`conn` is moved in both arms, and only one runs. Both arms yield `Result<Option<Option<Uuid>>, sqlx::Error>`, so
the tail chain is today's. Keep the `"lease_owner?"` override in both statements. Without it the join makes sqlx
infer `lease_owner` non-null and a `NULL` (chat or unleased run) decode would fail.

Doc comment (replace lines 197-201):

```rust
/// MOD-41 plan D1: `step` exists and its run carries `fence`'s lease, read under row locks on the
/// step and then the run, inside the caller's transaction, so an adoption cannot commit between
/// this check and the batch's writes. [`StoreError::NotFound`] first, then
/// [`StoreError::Fenced`]: `append_events`' order. The fenced twin of [`step_exists`], for
/// `upsert_step_tree`, `record_commits` and [`fenced_miss`]; `close_out` keeps the unfenced
/// check (blueprint B-1).
///
/// MOD-77 plan D1: the step is locked **before** the run, `park_step`'s `FOR UPDATE OF s, r`
/// order. MOD-41 locked the run alone (`FOR SHARE OF r`) and left the step to the writes that
/// follow: an insert's foreign key takes `FOR KEY SHARE` on it, and `upsert_step_tree`'s
/// `UPDATE run_step` a row lock. Against a park that held the step and waited for the run, that
/// closed a cycle and Postgres aborted one side (`40P01`). Holding the step first makes
/// whichever of the two reaches it first run to its commit. The clause order is load-bearing:
/// Postgres locks a joined row in the order the locking clauses are written. `lock` chooses the
/// step's mode ([`StepLock`]); `pg_criteria.rs`'s `upsert_step_tree_takes_the_step_first` and
/// `record_commits_takes_the_step_first` pin the order.
```

### 1.3 `step_scope` doc (`write.rs:233-245`): only the sentence that now disagrees

Replace:

```
/// It locks the **step** as well as the run (`FOR SHARE OF s, r`, the order of `park_step`'s
/// `FOR UPDATE OF s, r`), where `step_fence` locks the run alone. The writes that follow insert a
```

with:

```
/// It locks the **step** as well as the run (`FOR SHARE OF s, r`, the order of `park_step`'s
/// `FOR UPDATE OF s, r`, which [`step_fence`] has taken too since MOD-77). The writes that follow insert a
```

Re-wrap to 100 columns. The rest of the paragraph (the `40P01` story, the race test name) stays verbatim.

### 1.4 `fenced_miss` (`write.rs:292-300`)

Body: `step_fence(&mut conn, step, fence, StepLock::Share).await?;`. Append to its doc:

```
///
/// [`StepLock::Share`] (MOD-77 plan D1): it runs on a pooled connection outside any transaction,
/// so the lock lasts one statement and only makes this read wait for a writer that holds the step
/// or its run; no `UPDATE` follows that would need the step's update lock.
```

`fenced_or_missing` is unchanged: its `step_exists` read takes no lock.

### 1.5 `append_events` (D3, `write.rs:~1147`)

In the `lease` CTE, change exactly one line:

```
                   FOR SHARE OF r
```
to
```
                   FOR SHARE OF s, r
```

Nothing else in the statement changes. Doc: in the paragraph "The fence is decided **inside** the statement…",
replace "`lease` reads the named steps' runs and share-locks them, so" with:

```
    /// … `lease` share-locks each named step and then its run (`FOR SHARE OF s, r`, MOD-77 plan
    /// D3: `park_step`'s order, so a park holding the step cannot deadlock against the insert's
    /// foreign-key lock on it), so a `take_lease` or `adopt_runs` either waits for this write or is
    /// seen by it; …
```

Then add one paragraph after it (the carried case, plan D7):

```
    /// A batch naming two steps of one run may still lock them in either order against a park of
    /// the second (MOD-77 plan D7); the only production caller, `Recorder::flush`, writes one
    /// step per batch.
```

### 1.6 `set_step_usage` (D2, `write.rs:~1200-1225`)

```rust
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET usage = $2, prompt_digest = COALESCE($3, prompt_digest) \
               FROM locked \
              WHERE run_step.id = locked.id",
            step.as_uuid(),
            usage,
            prompt_digest.as_deref(),
            fence.owner(),
        )
```

Argument order and the `rows_affected()` / boxed `fenced_or_missing` tail are unchanged. The collapsed text is:
`WITH locked AS ( SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id WHERE s.id = $1 AND r.lease_owner IS NOT
DISTINCT FROM $4 FOR NO KEY UPDATE OF s FOR SHARE OF r) UPDATE run_step SET usage = $2, prompt_digest = COALESCE($3,
prompt_digest) FROM locked WHERE run_step.id = locked.id`.

Doc: replace "`FOR SHARE` on the run for `append_events`' reason (MOD-40 blueprint B2)." with:

```
    /// … The fence is a locking CTE (MOD-77 plan D2): the step `FOR NO KEY UPDATE`, then its run
    /// `FOR SHARE`. The run lock is `append_events`' reason (MOD-40 blueprint B2). Holding the
    /// step first is `park_step`'s order, so a park on the same step cannot deadlock against this
    /// write (`40P01`, `pg_criteria.rs::set_step_usage_takes_the_step_first`), and the step's
    /// update lock is taken up front rather than upgraded. The clause order is load-bearing.
```

### 1.7 `set_step_prompt` (D2, `write.rs:~1966-1980`)

```rust
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step SET prompt_digest = $2, trim_record = $3 \
               FROM locked \
              WHERE run_step.id = locked.id",
            step.as_uuid(),
            digest,
            trim,
            fence.owner(),
        )
```

Doc: "Written only while the step's run carries `fence`'s lease: `set_step_usage`'s predicate, verbatim (MOD-41
plan D1), `FOR SHARE` on the run for `append_events`' reason." becomes:

```
    /// Written only while the step's run carries `fence`'s lease: `set_step_usage`'s locking CTE,
    /// verbatim (MOD-41 plan D1, MOD-77 plan D2), the step and then its run.
```

### 1.8 `finish_step` (D2, `write.rs:~4806-4823`)

```rust
        let updated = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 \
                    AND r.lease_owner IS NOT DISTINCT FROM $8 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET exit_code        = $2, \
                    usage            = COALESCE($3, usage), \
                    trim_record      = COALESCE($4, trim_record), \
                    verify_outcome   = $5, \
                    verify_exit_code = $6, \
                    finished_at      = $7 \
               FROM locked \
              WHERE run_step.id = locked.id",
            // eight arguments unchanged, in today's order
        )
```

Doc: replace "`FOR SHARE` on the run for `append_events`' reason (MOD-40 blueprint B2)." with
"`set_step_usage`'s locking CTE (MOD-40 blueprint B2, MOD-77 plan D2): the step, then its run."

### 1.9 `pass_step` (D2, `write.rs:~5518-5530`)

```rust
        let moved = sqlx::query!(
            "WITH locked AS ( \
                 SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
                  WHERE s.id = $1 AND s.status = 'running' \
                    AND r.lease_owner IS NOT DISTINCT FROM $4 \
                    FOR NO KEY UPDATE OF s FOR SHARE OF r) \
             UPDATE run_step \
                SET status = 'done', gate_outcome = 'skipped', \
                    gate_note = COALESCE($2, gate_note), \
                    finished_at = COALESCE(finished_at, $3) \
               FROM locked \
              WHERE run_step.id = locked.id AND run_step.status = 'running'",
            step.as_uuid(),
            note,
            at,
            fence.owner(),
        )
```

The tail stays `Box::pin(fenced_miss(&self.pool, step, fence)).await`. Doc: "`FOR SHARE` on the run for
`finish_step`'s reason." becomes "`finish_step`'s locking CTE (MOD-77 plan D2), with `status = 'running'` in the
CTE, so a park committed first matches no row."

### 1.10 `upsert_step_tree` / `record_commits` (D1, `write.rs:5152`, `:5238`)

- `upsert_step_tree`: `step_fence(&mut tx, step, fence, StepLock::Update).await?;`. Doc: after "An empty slice
  still runs the existence and fence check (`step_fence`) and writes nothing." add "The fence takes the step's
  update lock ([`StepLock::Update`], MOD-77 plan D1), because this transaction may go on to update
  `run_step.isolation_path`. It is taken before the run's share lock, `park_step`'s order."
- `record_commits`: `step_fence(&mut tx, step, fence, StepLock::Share).await?;`. Doc: add "The fence share-locks
  the step and then its run ([`StepLock::Share`], MOD-77 plan D1). The inserts only key-share the step."

### 1.11 `finish_chat_run` (D5, `write.rs:1902-1935`): exact statement sequence

```rust
        let step_status = chat_step_status(status)
            .ok_or_else(|| StoreError::Constraint(not_a_terminal_status(status)))?;   // 0. unchanged, before the tx

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        // 1. MOD-77 plan D5: the step first, so this close and a usage write on the same chat
        //    lock in one order. Absence is remembered, not answered yet: an unknown run is
        //    `NotFound { run }` before an unknown step (conformance `start_chat_run_mints_chat_rows`).
        let step_found = sqlx::query_scalar!(
            "SELECT 1 FROM run_step WHERE id = $1 FOR NO KEY UPDATE",
            step.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .is_some();

        // 2. unchanged: `UPDATE run SET status = $2, finished_at = $3 WHERE id = $1`;
        //    0 rows -> Err(NotFound { entity: "run", .. })
        // 3. new:
        if !step_found {
            return Err(StoreError::NotFound { entity: "run_step", id: step.to_string() });
        }
        // 4. unchanged: `UPDATE run_step SET status = $2, finished_at = $3 WHERE id = $1`,
        //    with its `closed_step == 0` guard (B5)
        // 5. unchanged: tx.commit()
```

Each early `return` drops `tx`, which rolls back and releases the step lock. Error order for every input:
non-terminal status → `Constraint`; unknown run (any step) → `NotFound{run}`; known run + unknown step →
`NotFound{run_step}`. That matches today and `mem.rs:2218`. Doc: add a paragraph after the first:

```
    /// MOD-77 plan D5: the step is locked `FOR NO KEY UPDATE` before the run is updated. A chat's
    /// `set_step_usage` locks the step and then the run; closing the run first and the step second
    /// would close a cycle against it (`40P01`, `pg_criteria.rs::finish_chat_run_takes_the_step_first`).
    /// Errors keep their order: the run's `NotFound` before the step's.
```

### 1.12 `relay.rs` (D4)

`open_permission` (line ~130):

```rust
    // MOD-77 plan D4: the step and then its run are share-locked, `park_step`'s order, so an
    // adoption cannot commit between this check and the insert, and a park holding the step
    // cannot deadlock against the insert's foreign-key lock on it.
    let fence = sqlx::query!(
        r#"SELECT s.run_id, r.lease_owner AS "lease_owner?"
             FROM run_step s JOIN run r ON r.id = s.run_id
            WHERE s.id = $1
              FOR SHARE OF s, r"#,
        open.run_step_id.as_uuid(),
    )
```

Module doc lines 8-10:

```
//! `clock_timestamp()`, never a box clock (I-4). The only transaction is `open_permission`'s,
//! which share-locks the step and then its run (`FOR SHARE OF s, r`, MOD-77 plan D4: `park_step`'s
//! order) so an adoption cannot commit between its fence and its insert (MOD-41's `step_fence`
//! shape).
```

---

## 2. The D6 tests (`pg_criteria.rs`)

Place them as a new section directly after `a_step_document_racing_a_park_never_deadlocks` (before
`queued_build`), headed:

```rust
// ------------------------------------------------------------------------------------------------
// MOD-77 (plan D1-D6): every fenced step writer locks the step before its run, `park_step`'s order.
// ------------------------------------------------------------------------------------------------
```

**Imports**: add `ChatRunSpec` and `RunStepCommit` to the `use htui_core::model::{…}` list (rustfmt sorts them).
Everything else used is already imported (`OpenPermission`, `PermissionId`, `RelaySessionId`, `NewRepo`, `RepoId`,
`RunStepTree`, `Isolation`, `SessionEvent`, `EventKind`, `EventRole`, `StepOutcome`, `RunStatus`, `StepFence`,
`Claim`, `TimeDelta`, `ids`). `NewRunStep` / `StepStatus` go by full path, as in the race test.

**Skip**: every test opens with `let Some(db) = common::demo_db().await else { return; };`. With
`HTUI_TEST_DATABASE_URL` unset it prints `common::SKIP` and the test passes (it panics only when `CI` is set). Every
test ends with `db.drop_db().await;`. The file is `#![cfg(feature = "demo")]`, and `--all-features` covers it.

### 2.1 Fixture `leased_running_step`

```rust
/// MOD-77: a graph run on `HTUI_ANA_2` leased by a fresh owner, and one running step of it:
/// `a_step_document_racing_a_park_never_deadlocks`' round, once.
async fn leased_running_step(db: &common::TestDb) -> (RunId, StepId, uuid::Uuid)
```

Body: `owner = Uuid::now_v7()`; `run = db.store.create_run(race_run(ids::HTUI_ANA_2)).await…id`; assert
`claim_run(run, ids::BOX, owner, Utc::now(), TimeDelta::minutes(30)) == Claim::Admitted`; `step =
create_step(NewRunStep { id: StepId::new(), run_id: run, position: 0, attempt: 1, fanout_index: 0, phase_name:
"implement", agent_id: Some(ids::AGENT_CLAUDE), model: None })…id`; assert `transition_step(step, Pending, Running,
Utc::now())` is `true`. Return `(run, step, owner)`.

### 2.2 Helper `assert_takes_the_step_first` (exact signature and behaviour)

```rust
/// MOD-77 plan D6: `write` must wait on a held step while holding **nothing** on its run.
///
/// A raw transaction takes the step `FOR UPDATE`, the first half of a `park_step`. `write` is
/// spawned, and once a backend of this database waits on a lock the holder owns, the holder asks
/// for the run `FOR UPDATE NOWAIT`, the second half of the park. If `write` held the run while it
/// waited, that is `55P03`, and a real park would have closed a cycle with it (`40P01`). The
/// holder then rolls back and `write`'s answer is returned for the caller's own assertion.
async fn assert_takes_the_step_first<T: Send + 'static>(
    db: &common::TestDb,
    run: RunId,
    step: StepId,
    write: impl std::future::Future<Output = T> + Send + 'static,
) -> T
```

Behaviour, in order:

1. `let mut holder = db.pool.begin().await.expect("begin the holder");`
2. `let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *holder)…;`
3. `sqlx::query("SELECT 1 FROM run_step WHERE id = $1 FOR UPDATE").bind(step.as_uuid()).fetch_one(&mut *holder)`
   (`fetch_one`, so a fixture naming no step fails loudly here).
4. `let handle = tokio::spawn(write);`
5. Poll, bounded at 10 s with 20 ms sleeps (`wait_for_the_take`'s loop), on `db.pool`:
   ```sql
   SELECT count(*) FROM pg_stat_activity
    WHERE datname = current_database() AND wait_event_type = 'Lock'
      AND $1 = ANY(pg_blocking_pids(pid))
   ```
   bound to `holder_pid`, read as `i64`. Break on `> 0`. On timeout panic with "the write never waited on the held
   step". Then `assert!(!handle.is_finished(), "the write answered while the step was held")`.
6. `let nowait = sqlx::query("SELECT 1 FROM run WHERE id = $1 FOR UPDATE NOWAIT").bind(run.as_uuid())
   .fetch_one(&mut *holder).await;`. Keep the `Result`. Derive `code: Option<String>` from
   `nowait.as_ref().err().and_then(sqlx::Error::as_database_error).and_then(|e| e.code()).map(Cow::into_owned)`.
7. `holder.rollback().await.expect("roll back the holder");` (also valid after a `55P03`-aborted transaction).
8. `let answer = tokio::time::timeout(Duration::from_secs(10), handle).await.expect("the write lands once the step is
   free").expect("the write task must not panic");`
9. `assert!(nowait.is_ok(), "the write held the run while it waited on the step (SQLSTATE {code:?}); a park holding
   this step would deadlock against it");`
10. `answer`

**Why the waiter count is clean.** `db.pool` is the `PgStore`'s own pool, connected to `htui_test_<12 hex>`
(`testkit::bare_db`: `url = with_database(maint_url, name)`, `pool = connected.store.pool().clone()`). So
`current_database()` evaluated on it names this test's database alone. With `--test-threads=1` no other case of the
binary runs, and other binaries run after this one. Background pool connections (idle, `LISTEN`) never sit in
`wait_event_type = 'Lock'`. B6's `pg_blocking_pids` filter goes further: it counts only a backend blocked by the
holder, which can only be the writer.

**Pool headroom.** The holder, the writer's statement or transaction and the poll each use one connection of
`db.pool`. `an_uncommitted_take` already runs that same three-way use green.

### 2.3 The nine tests (each `#[tokio::test(flavor = "multi_thread")]`)

The names all contain `takes_the_step_first`, so the plan's filter selects exactly them. `fence =
StepFence::Lease(owner)` from `leased_running_step` unless stated. `store = db.store.clone()` is moved into the
`async move`.

| # | Test | Fixture extra | Write (inside `async move`) | Lands as | Why red today |
|---|---|---|---|---|---|
| 1 | `append_events_takes_the_step_first` | – | `store.append_events(fence, &[row]).await`; `row` = the `SessionEvent` of `a_lease_take_committed_mid_write_fences_it` with `run_step_id: step`, `seq: 0` | `Ok(1)` | `lease` CTE share-locks the run; the insert's FK `KEY SHARE` waits on the step |
| 2 | `set_step_usage_takes_the_step_first` | – | `store.set_step_usage(fence, step, json!({"input_tokens": 1}), None).await` | `Ok(())` | `EXISTS … FOR SHARE` locks the run, then the `UPDATE` waits on the step |
| 3 | `set_step_prompt_takes_the_step_first` | – | `store.set_step_prompt(fence, step, "sha256:mod-77", &trim).await`, `trim = json!({})` owned in the block | `Ok(())` | as 2 |
| 4 | `finish_step_takes_the_step_first` | – | `store.finish_step(fence, step, outcome).await`; `outcome` = the `StepOutcome` of `a_lease_take_committed_mid_settle_fences_it` (`verify_exit_code: Some(0)`, `finished_at: now`) | `Ok(())` | as 2 |
| 5 | `pass_step_takes_the_step_first` | – | `store.pass_step(fence, step, None, Utc::now()).await` | `Ok(true)` | as 2 |
| 6 | `upsert_step_tree_takes_the_step_first` | one repo via `create_repo(NewRepo { id: RepoId::new(), project_id: ids::PROJECT_HTUI, name: "mod-77", default_branch: "main", is_primary: true, remote_url: None })` | `store.upsert_step_tree(fence, step, &[tree]).await`; `tree = RunStepTree { run_step_id: step, repo_id, mode: Isolation::Worktree, path: "/srv/trees/mod-77", base_ref: "main", dirty: false }` | `Ok(())`, then `SELECT isolation_path FROM run_step WHERE id = $1` (runtime query) is `Some("/srv/trees/mod-77")` | `step_fence` locks the run only; the `run_step_tree` FK waits on the step |
| 7 | `record_commits_takes_the_step_first` | one repo, as 6 | `store.record_commits(fence, step, &[commit]).await`; `commit = RunStepCommit { run_step_id: step, repo_id, before_hash: "a".repeat(40), after_hash: None }` | `Ok(())` | as 6. **A non-empty batch is required**: with `&[]` today's code never waits and the test would fail on the waiter timeout, not on `55P03` |
| 8 | `open_permission_takes_the_step_first` | – | `store.open_permission(OpenPermission { id, run_id: run, run_step_id: step, session: RelaySessionId::new(), request_id: "r1", tool_call_id: None, summary: None, options: relay_options(), owner }).await` | `Ok(id)` | fence `FOR SHARE OF r`; the `step_permission` FK waits on the step |
| 9 | `finish_chat_run_takes_the_step_first` | **not** `leased_running_step`: `chat = ChatRunSpec::mint(ids::PROJECT_HTUI, ids::BOX, ids::USER, Some(ids::AGENT_CLAUDE), None)`; `db.store.start_chat_run(&chat)`; helper gets `(chat.run_id, chat.step_id)` | `store.finish_chat_run(chat.run_id, chat.step_id, RunStatus::Done, Utc::now()).await` | `Ok(())`, then `db.store.run(chat.run_id)` → `status == RunStatus::Done` | `UPDATE run` holds the run, then `UPDATE run_step` waits on the step |

Each test has a one-line doc in `wait_for_the_take`'s register, for example `/// MOD-77 plan D2: `set_step_usage`
waits on a held step holding nothing on its run, then lands.`

### 2.4 Existing tests that must stay green

- `pg_criteria.rs`: `a_lease_take_committed_mid_write_fences_it`, `a_lease_take_committed_mid_settle_fences_it`,
  `a_step_document_racing_a_park_never_deadlocks`, `upsert_step_tree_writes_the_primary_isolation_path`, and the
  relay cases `a_second_box_answers_and_the_executor_applies`, `a_second_box_cannot_answer_after_adoption_or_expiry`,
  `two_concurrent_answers_admit_one`, `relay_times_are_the_databases`,
  `deleting_a_project_holding_relay_rows_succeeds`, `an_unknown_actor_is_refused_before_the_status`.
- `pg_conformance.rs`: `case_list_matches_mem_store` (stays 148) and `pg_store_conformance`, in particular
  `start_chat_run_mints_chat_rows` (D5's error order), `set_step_usage_writes_usage_and_digest`,
  `pass_step_writes_done_and_skipped_under_the_fence`, `trees_and_commits_round_trip`,
  `a_missing_step_is_not_found_before_the_fence`, `prompt_tree_commits`, `an_unleased_fence_writes_a_chat_step`,
  `an_unleased_prompt_write_is_refused_on_a_leased_run`, `opening_a_permission_is_fenced_on_the_lease`,
  `opening_a_permission_stales_older_sessions_of_the_step`.
- `crates/htui/tests/runs_pg.rs` (the debug-build worker-stack guard for the boxed miss paths) and
  `crates/htui/tests/chat_usage_pg.rs` (chat usage then close, both now step-first). Remember `--features testkit`
  / `--all-features` for `htui`'s `tests/*.rs` (memory `htui-integration-tests-need-testkit`).

### 2.5 One stale test doc (fix commit)

`a_lease_take_committed_mid_settle_fences_it`'s doc (`pg_criteria.rs:~5142`): "`finish_step`'s fence is an `EXISTS
… FOR SHARE` sub-select rather than the append's CTE" becomes "`finish_step`'s fence is a locking CTE (`FOR NO KEY
UPDATE OF s FOR SHARE OF r`, MOD-77) rather than the append's `lease` CTE". The test body is unchanged. The lease take
holds the run, and the CTE now takes the free step and then waits on the run, so the wait and the `Fenced` re-check
behave as before (plan probe: a take committed mid-write → 0 rows).

---

## 3. Build sequence and commit boundaries

**Step 0 (no commit): environment.** `HTUI_TEST_DATABASE_URL` is set in the sandbox. Create the scratch prepare DB now
(`docs/hr-sandbox.md:192-210`): `psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"`, then from
`crates/htui-store` run `DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx migrate run --source
migrations`. The test DSN and the prepare DSN are different databases (memory `sqlx-prepare-needs-migrated-scratch-db`).
Check `df -h .` first: `target/` pressure crashes Postgres.

**Commit (a): tests, red.** Edit `pg_criteria.rs` per §2.1-2.3 only. It builds offline unchanged, because of B7.

```
cargo test -p htui-store --all-features --test pg_criteria takes_the_step_first -- --test-threads=1
```

Expect **9 failed**, and **each** failure must be the step-9 assertion carrying `SQLSTATE Some("55P03")`. A waiter
timeout or a panic elsewhere is a wrong red: fix the test, not the source. Record the nine failure lines in the
commit body. Commit: `test(mod-77): lock-order probes for the nine fenced step writers (red)`.

**Commit (b): the fix.** Apply §1.1-1.12 and §2.5. Offline builds can't see the new SQL yet, so build and test
online: `SQLX_OFFLINE=false DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo …` (the cargo `[env]`
entry does not override a set variable). Gates:

```
SQLX_OFFLINE=false DATABASE_URL=… cargo test -p htui-store --all-features -- --test-threads=1   # nine green + §2.4
SQLX_OFFLINE=false DATABASE_URL=… cargo test -p htui-core --all-features
SQLX_OFFLINE=false DATABASE_URL=… cargo test -p htui --all-features --test runs_pg --test chat_usage_pg -- --test-threads=1
```

Commit: `fix(mod-77): Pg step writers lock the step before the run`.

**Commit (c): `.sqlx`.** From `crates/htui-store` with the scratch `DATABASE_URL`: `cargo sqlx prepare --
--all-targets --all-features`, then `cargo sqlx prepare --check`. Expect 7 removals (`step_fence` `fbbebda2…`,
`set_step_usage` `364ef2e0…`, `pass_step` `3c6fb8fc…`, `open_permission` `5bc288ee…`, `append_events` `7c45bd99…`,
`finish_step` `d65bfff9…`, `set_step_prompt` `de01976e…`) and 9 additions (two `step_fence`, four CTEs,
`append_events`, `open_permission`, `finish_chat_run`'s step lock). Any other file changing in `.sqlx` means a stray
edit, so stop. Stage only `crates/htui-store/.sqlx/` with explicit paths, including the deletions
(`git add -A -- crates/htui-store/.sqlx` is path-scoped and acceptable; a bare `git add -A` is not). Then run the
full validation block of the plan, which is offline now:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
SQLX_OFFLINE=true cargo build -p htui-store --all-features
cargo test -p htui-store --all-features -- --test-threads=1
```

Commit: `chore(mod-77): regenerate .sqlx for the step-first queries`.

**Commit (d): close-out (plan Task 4).** `docs/decisions/mod/mod-77.md` (D1-D7 plus B1-B8 above, the probe evidence,
the red run's nine `55P03` lines, the carried multi-step `append_events` batch case), the `docs/DECISIONS.md` row and
the `HANDOFF.md` line per `lifecycle.md` P2. Then `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## 4. Hazards

1. **Commit (b) does not build offline on its own.** `.cargo/config.toml` sets `SQLX_OFFLINE = "true"`, so a checkout
   of (b) alone fails `query!` until (c) lands, and `git bisect` across (b) is broken. This is fine if (b) and (c)
   merge together. If the reviewer wants every commit buildable, fold (c) into (b). That changes plan Tasks 2/3 into
   one commit, not their content.
2. **The CTE must stay materialized.** Postgres does not inline a CTE carrying row marks: probed on the sandbox PG
   16, `EXPLAIN` shows `CTE locked -> LockRows -> Nested Loop` beneath `Update`. Do **not** add `NOT MATERIALIZED`.
   It would be refused or would fold the locks into the `UPDATE`'s own row marks, and the order guarantee would go.
3. **Clause order.** `FOR NO KEY UPDATE OF s FOR SHARE OF r` and `FOR SHARE OF s, r`: `s` first, always. The D6 tests
   catch a reversal. `rustfmt` doesn't touch string contents, but a "tidy-up" that alphabetises `OF r, s` silently
   restores run-first.
4. **`UPDATE … FROM locked` and sqlx.** `rows_affected()` counts updated target rows, and `locked` yields at most one
   row (`s.id` is the PK), so `== 1` keeps its meaning. No output columns, so there are no new type overrides. `$1` is
   typed by `s.id = $1` (uuid), `$4`/`$8` by `r.lease_owner IS NOT DISTINCT FROM`, and the `SET` parameters by their
   columns, as today. Unqualified column names in `SET` / `COALESCE` resolve to `run_step` because `locked` exposes
   only `id`. That is why `WHERE run_step.id = locked.id` must be qualified.
5. **The `"lease_owner?"` override** must survive in both `step_fence` statements and in `open_permission`. Dropping it
   makes sqlx type the joined column `Uuid`, and every chat or unleased step then fails to decode.
6. **Future size / the 2 MiB worker stack.** The miss paths stay `Box::pin(fenced_or_missing(..))` and
   `Box::pin(fenced_miss(..))` (`fenced_or_missing`'s doc: the engine nests settles deep enough to overflow a debug
   worker stack, `htui/tests/runs_pg.rs`). The CTE change does not grow the inline futures, because it is the same
   macro with the same arguments. `step_fence`'s two arms are alternative await states, so the layout is roughly
   their max. `finish_chat_run` gains one await on the agent-worker path, not the engine's. Do not inline
   `fenced_miss` into `pass_step` while touching it. Gate on `runs_pg` (and on htui-orch's `every_case_name_dispatches`
   if run, `--no-fail-fast`, grep `SIGABRT`; memory `htui-orch-test-stack-headroom`).
7. **Writers of one step now serialise earlier.** `set_step_usage` vs `finish_step` vs `upsert_step_tree` on one step
   used to meet only at the `UPDATE`. They now meet at the CTE/fence lock. They are sequential in one walk, so
   there's no behaviour change (plan risk table). `append_events` (`FOR SHARE OF s`) now also waits behind a held
   `NO KEY UPDATE` on its step, e.g. a usage write. Both are short single statements.
8. **Red must be the right red** (§3a). `record_commits` with an empty batch and `upsert_step_tree` with an empty batch
   do not wait at all on today's code. They would time out in the poll, not fail `NOWAIT`. The nine fixtures above
   all reach a step lock on today's code.
9. **Nothing in this change alters the plan's decisions.** The only plan-level adjustment worth a maintainer's look
   is hazard 1 (commit granularity).
