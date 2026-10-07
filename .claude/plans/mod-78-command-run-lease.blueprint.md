# Blueprint: MOD-78 — `command_run` lease check and session-end cancel

**Plan**: `.claude/plans/mod-78-command-run-lease.plan.md` (confirmed 2026-10-06)
**Author**: code-architect, against `aaf794a0`


I read the confirmed plan, then the full source of: `host.rs`, `tools/command.rs`, `tools/mod.rs`, `tests/tools_command.rs`, `run_shell` and `GroupGuard`, the lease and queue trait declarations, `StepFence`, the Mem `State` lease operations, `fence_holds`, `MemFault`, the Pg `refresh_lease`, `release_lease`, `item_by_key` and `beat_command`, every `item_by_key` forwarder, the conformance helpers, both case-count pins, and the migrations. Gortex answered (it is "INACTIVE" but works). Line numbers below are as of `aaf794a0`.

None of D1-D7 is wrong against the code. There are three real gaps (B-1, B-2, B-3) and one sequencing hazard (B-4).

### Design decisions (additions inside the plan)
- **Lease checks in `command.rs` go through one small `Copy` struct `Lease { run, fence, step }`.** `Lease::lost()` returns `store_error(StoreError::Fenced { step })`, so the string `"fenced: lease lost"` keeps one source of truth and `tools/mod.rs` is not touched.
- **D3(d) rate limit uses `tokio::time::Instant`.** The pre-enqueue read (a) counts as the first read, so the first read in the queue happens 10 s or more after `admit` starts.
- **D5 is extended to (b) and (d).** A failed read there does not stop the call, the same as on the heartbeat. Only the pre-enqueue read (a) refuses the call (B-6).
- **`Stopped` is passed out of the `stop` future through a `std::sync::OnceLock<Stopped>`.** It replaces the `AtomicBool`. A `Cell` would make the future `!Send`.

---

### 1. Store read (T1)

**Trait**, `crates/htui-core/src/store/traits.rs`, immediately after `item_by_key` (line 1904), before the `MOD-11 M4` comment:
```rust
/// MOD-78 D1: whether `run` carries `fence`'s lease now: `run.lease_owner IS NOT DISTINCT FROM`
/// [`StepFence::owner`], the predicate every fenced write uses ([`StepFence`]), read with no row
/// lock (no `FOR SHARE`), so the answer may be stale by the time the caller acts on it. Owner
/// only, never the expiry (MOD-78 D2): an expired lease nobody has taken still holds, as it still
/// writes. `command_run` reads it before it queues, at admission, and on every heartbeat (MOD-78
/// D3), because the queue's own writes take no fence. A read on `WriteStore` by the
/// `command_runs` precedent: `run.lease_owner` is not a [`Run`] field, so no `ReadStore` read
/// answers it.
///
/// # Errors
/// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }` for an
/// unknown run; the backend's own failures.
async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool>;
```

**WorkerStore declaration**, `crates/htui-core/src/store/worker.rs`, after `item_by_key` (line 384):
```rust
/// [`WriteStore::lease_holds`].
fn lease_holds(&self, run: RunId, fence: StepFence) -> impl Future<Output = Result<bool>> + Send;
```
**MemStore forward**, same file, after line 807:
```rust
async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
    WriteStore::lease_holds(self, run, fence).await
}
```

**Mem**, `crates/htui-core/src/store/mem.rs`:
- New `MemFault::LeaseHolds` variant (line 122; see B-3). No code matches on `MemFault` exhaustively (checked with gortex usages). The enum's doc says "a write"; change it to "a call".
  ```rust
  /// [`WriteStore::lease_holds`] (MOD-78 D5): a read, so `command_run` can tell a store blip
  /// from a lost lease.
  LeaseHolds,
  ```
- `State` method next to `release_lease` (around line 4790):
  ```rust
  /// MOD-78 D1: [`State::fence_holds`]'s predicate on a run. `lease_owners` holds a run only
  /// while its lease names an owner, so a missing entry is Postgres's `NULL`.
  fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
      self.require_run(run)?;
      Ok(self.lease_owners.get(&run).copied() == fence.owner())
  }
  ```
- `WriteStore` impl, after `item_by_key` (line 7770):
  ```rust
  async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
      #[cfg(feature = "test-support")]
      self.check_fault(MemFault::LeaseHolds)?;
      self.read(|state| state.lease_holds(run, fence))
  }
  ```

**Pg**, `crates/htui-store/src/pg/write.rs`, after `item_by_key` (line 6872). The migrations confirm the names: table `run` (`0001_init.sql:447`) and column `lease_owner UUID` (`0003_orchestration.sql:42`).
```rust
/// MOD-78 D1: one plain read on the pool. There is no `FOR SHARE`: the answer is a snapshot
/// either way, and a lock would queue behind `claim_run`'s and the fenced writes' row locks.
/// The predicate is the fenced writes' `IS NOT DISTINCT FROM`.
///
/// # Errors
///
/// `NotFound { entity: "run" }` when no row matches; the backend's own failures.
async fn lease_holds(&self, run: RunId, fence: StepFence) -> Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT lease_owner IS NOT DISTINCT FROM $2 AS "holds!" FROM run WHERE id = $1"#,
        run.as_uuid(),
        fence.owner(),
    )
    .fetch_optional(&self.pool)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound { entity: "run", id: run.to_string() })
}
```
Without `"holds!"`, sqlx infers an expression column as nullable and returns `Option<Option<bool>>`. `$2` binds `Option<Uuid>`, as the fenced writes already do (line 1287).

**Forwarders.** Each goes right after its file's `item_by_key`. Every file already imports `RunId` and `StepFence`.
- `crates/htui-store/src/worker.rs:334` (PgStore) and `:671` (Writer): `WriteStore::lease_holds(self, run, fence).await`
- `crates/htui-store/src/writer.rs:1197`: `match self { Self::Memory(store) => store.lease_holds(run, fence).await, Self::Online(pg) => pg.lease_holds(run, fence).await }`
- `crates/htui-agent/src/conformance.rs:1295` (UsageSpy) and `crates/htui-agent/tests/recorder.rs:1019` (SpyStore): `async fn lease_holds(&self, run: RunId, fence: StepFence) -> StoreResult<bool> { self.inner.lease_holds(run, fence).await }`

**`.sqlx` entry.** Follow `docs/hr-sandbox.md:196-205`: create `htui_sqlx` on port 5439, run `cargo sqlx migrate run --source migrations`, then `cargo sqlx prepare -- --all-targets --all-features` from `crates/htui-store`.

### 2. Conformance case (T1)
- **Name:** `lease_holds_reads_the_owner`.
- **Placement:**
  - `CASES`: append last, after `"open_permissions_list_live_pending_item_requests"` (line 205).
  - `run_case`: a new arm before `other =>` (line 532).
  - The function: before `#[cfg(test)] mod tests` (line 18276), after `cancel_command_ends_a_queued_or_running_row`.
- **Helpers it uses (verified):**
  - `seam_clock() -> DateTime<Utc>` (4636)
  - `LEASE` (5 min)
  - `leased_step(case, store, a: Uuid, at) -> (RunId, StepId)` (6694): creates a run on `HTUI_ANA_2` and claims it for `a`.
  - `taken_by(case, store, run, a, b)` (6852): sets `a`'s lease to a zero TTL, then `b` takes it with `take_lease` for 14 min.
  - `ChatRunSpec::mint(project, box, user, agent, model)` and `start_chat_run(&chat)` (as at 7054).
- **Body:**
  1. `(run, _) = leased_step(CASE, store, a, at)`. Check: `Lease(a)` is true, `Lease(b)` false, `Unleased` false.
  2. `taken_by(CASE, store, run, a, b)`. Check: `Lease(a)` false, `Lease(b)` true.
  3. D2: `refresh_lease(run, b, TimeDelta::zero())`. `Lease(b)` is still true (expired but not taken).
  4. `release_lease(run, b)` returns true. Check: `Unleased` true, `Lease(b)` false.
  5. Chat run: `Unleased` true on `chat.run_id`, `Lease(a)` false.
  6. `lease_holds(RunId::new(), StepFence::Unleased)` matches `Err(StoreError::NotFound { entity: "run", .. })`.

### 3. host.rs (T2)

**Imports.** Drop `use std::sync::atomic::{AtomicBool, Ordering};` (line 13). Add `use tokio_util::sync::CancellationToken;`.

**`crates/htui-mcp/Cargo.toml`:**
```toml
# MOD-78 D6: `sync::CancellationToken` ends a session's in-flight calls with the session; the
# module is not behind a feature.
tokio-util = { workspace = true }
```
In `Cargo.lock`, only `htui-mcp`'s dependency list changes; `htui-agent` already locks `tokio-util`.

**`Session` field** (replaces `ended`, line 82):
```rust
/// MOD-78 D6 (I-6): cancelled when the lease drops, the host closes, or the last `McpHost` drops
/// (D7). `Served::call` races every call against it, so an in-flight call ends with its session.
pub(crate) cancel: CancellationToken,
```
- `has_ended()` becomes `self.cancel.is_cancelled()`.
- In `open`, the literal at line 396 becomes `cancel: CancellationToken::new()`.

**`Served::call`.** Keep the `has_ended()` fast path and the `advertised` check. Then:
```rust
let ctx = Ctx { session: &session, host, progress };
// MOD-78 D6: an end of the session wins over a ready answer. The dropped call undoes itself:
// `Enqueued` cancels its row, and `run_shell`'s `GroupGuard` kills the child.
tokio::select! {
    biased;
    () = session.cancel.cancelled() => Ok(session_ended()),
    result = tools::dispatch(&name, ctx, arguments) => Ok(match result { /* existing arms */ }),
}
```
Both branches borrow `session` immutably, which compiles.

**Lease closure** (lines 440-446). Before `ToolLease::new`, add `let cancel = session.cancel.clone();`. The closure calls `cancel.cancel();` and keeps the `weak.upgrade()` removal. It no longer captures `session`.

**`close()`** (line 456): `served.session.cancel.cancel();`

**`impl Drop for Inner<H>`:**
```rust
/// MOD-78 D7: the last `McpHost` is gone. New calls already end through `Bound`'s `Weak`; an
/// in-flight one holds its `Arc<Served>` and ends through the token.
impl<H: htui_core::store::WorkerHost> Drop for Inner<H> {
    fn drop(&mut self) {
        let sessions = self.sessions.get_mut().unwrap_or_else(PoisonError::into_inner);
        for served in sessions.values() {
            served.session.cancel.cancel();
        }
    }
}
```
This does not conflict with anything:
- The bounds match the struct's, so E0367 cannot arise.
- Nothing destructures or moves out of `Inner`; it only lives inside an `Arc`.
- `get_mut` takes no lock.
- `cancel()` needs no runtime. `open_outside_a_runtime_is_a_listener_error` drops a host outside a runtime and still works.
- Fields still drop after it in declaration order (host, sessions, listener, config).

**T2 tests (host.rs `mod tests`), real time.** The in-flight call is deterministic: a `command_run` that can never be admitted.

Helper `queued_call()` returns `(MemStore, McpHost<Backend>, ToolLease, JoinHandle<io::Result<CallResult>>)`:
1. `let store = MemStore::demo(); let host = McpHost::new(Backend::memory(store.clone()))`.
2. Hold the box's only `build` slot. `store.enqueue_command(NewCommandRun { run_step_id: ids::STEP_R2_PRD, box_id: ids::BOX, class: "build", status: Queued, queued_at: Utc::now(), .. })`, then `store.claim_command(held.id, Uuid::now_v7(), 1)` must return `Some`. With no `command_limits` set, the build limit is 1 (the precedent is `progress_ticks_while_queued`).
3. Open `host.open(ToolScope { run_id: ids::RUN_2, step_id: ids::STEP_R2_PRD, command_queue: true, ..scope(Transport::Acp) })`. Do not use the bare `scope()`; see B-5.
4. Get the client with `host.client(token)` and call `initialize()`.
5. `tokio::spawn(async move { client.call("command_run", json!({"class":"build","command":"true"})).await })`.
6. Poll `store.command_runs(ids::STEP_R2_PRD)` every 20 ms, for at most 5 s, until a row whose id is not `held.id` is `Queued`.

Import only `htui_core::store::WriteStore`; importing both store traits makes MemStore's method calls ambiguous (E0034).

Tests:
- (a) `an_in_flight_call_ends_when_its_lease_drops`: `drop(lease)`. Within `timeout(5 s)` the call answers `is_error` with `"session ended"`. Then poll until our row is `Cancelled` (the guard's cancel runs in a spawned task).
- (b) `an_in_flight_call_ends_when_the_host_closes`: same, with `host.close()`. Keep the lease alive until the end.
- (c) `an_in_flight_call_ends_with_the_last_host`: `let weak = Arc::downgrade(&host.inner); drop(host); assert!(weak.upgrade().is_none())`. Then the same assertions; drop the lease last.
- (d) The existing `client_refuses_an_unknown_token` and `dropping_the_last_host_ends_an_in_process_client` stay green unchanged.

All three fail on the current code: the call stays in `admit` for ever. They spawn no process. They stay correct after T3: `RUN_2` has no lease owner in the demo store, so `Unleased` holds.

### 4. command.rs (T3)
```rust
/// MOD-78 D1, D3: the lease the session's writes are fenced on, and the step a loss is
/// reported on.
#[derive(Debug, Clone, Copy)]
struct Lease { run: RunId, fence: StepFence, step: StepId }
impl Lease {
    fn of(scope: &ToolScope) -> Self { /* run_id, fence, step_id */ }
    /// MOD-78 D1: a lock-free read of whether the run still carries the fence's lease.
    async fn holds<S: WorkerStore>(self, store: &S) -> Result<bool, StoreError> { WorkerStore::lease_holds(store, self.run, self.fence).await }
    /// The answer every fenced tool gives (`store_error`).
    fn lost(self) -> ToolError { store_error(StoreError::Fenced { step: self.step }) }
}

/// MOD-78 D4: why [`beat`] returned, which is why [`run_shell`] stopped the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopped { Taken, LeaseLost }
```

The checks in `call`:
- **(a)** Just before `enqueue_command` (line 137): `Ok(true)` continues; `Ok(false)` returns `Err(lease.lost())` and writes nothing; `Err(e)` returns `Err(store_error(e))` (D5: `store unavailable: …`, or `not found: run …`).
- **(d)** `admit` gains a `lease: Lease` argument. Before the loop: `let mut read_at = tokio::time::Instant::now();`. At the top of each iteration: `if read_at.elapsed() >= COMMAND_HEARTBEAT { read_at = Instant::now(); if let Ok(false) = lease.holds(store).await { return Err(lease.lost()); } }`. A read error does not stop the wait. `claim_command` errors are unchanged. Because `Enqueued` is still armed, the return cancels the row.
- **(b)** After `admit(..)?`, before `run_shell`: `if let Ok(false) = lease.holds(&store).await { return Err(lease.lost()); }`. The row is `running`; the armed guard's `cancel_command` accepts that.
- **(c)** `beat` gains `lease: Lease` and returns `Stopped`. Each iteration:
  1. Sleep `COMMAND_HEARTBEAT`.
  2. `if let Ok(false) = beat_command(..) { return Stopped::Taken }`.
  3. `if let Ok(false) = lease.holds(store).await { return Stopped::LeaseLost }`. An `Err` is not a stop (D5).
  4. Tick.
  
  In `call`: `let stopped = OnceLock::new(); let beats = async { let _ = stopped.set(beat(..).await); };`. Remove the `AtomicBool`/`Ordering` import (line 16). `stopped.get().is_some()` is true exactly when `run_shell`'s race ended `Stopped`, as with the flag today.
- **D4 finish path.** Add this after `output` is computed and before the existing `finished` block:
  ```rust
  if stopped.get() == Some(&Stopped::LeaseLost) {
      let note = format!("[stopped: fenced: lease lost]\n{output}");
      if WorkerStore::finish_command(&store, id, claimant, CommandRunStatus::Cancelled, None, Some(note)).await.is_ok() {
          guard.disarm(); // Ok(false): the queue ended it first. Err: the armed guard cancels it.
      }
      return Err(lease.lost());
  }
  ```
  - The row ends `cancelled`, with `exit_code` NULL and output `[stopped: fenced: lease lost]\n` followed by the scrubbed tail (which may still carry the truncation marker).
  - The agent gets `{isError: true, text: "fenced: lease lost"}`.
  - `Stopped::Taken` takes the existing `stored_row` path unchanged.

**Paused time with a real child: do not use it.** Under `start_paused`, tokio advances the clock whenever every task is waiting. A task waiting for a child's exit or pipe counts as waiting, so `beat`'s 10 s sleep, `run_shell`'s `sleep(budget)` and `kill_within_grace`'s timers all fire at once in virtual time while the process runs on the wall clock. The result is that a run reports `TimedOut` almost immediately. So the tests are split (B-2):

*Unit tests in `command.rs`, `start_paused`, no child.* Setup: `MemStore::demo()`, then `claim_run(ids::RUN_2, ids::BOX, owner, Utc::now(), TimeDelta::minutes(5))` must return `Claim::Admitted` (precedent `recorder.rs:4280`). To take the lease away: `release_lease(ids::RUN_2, owner)` then `take_lease(ids::RUN_2, ids::BOX, stranger, TimeDelta::minutes(5))`, both true. Then `Lease { run: RUN_2, fence: Lease(owner), step: STEP_R2_PRD }`.
- `a_waiter_on_a_lost_lease_stops_within_two_heartbeats` (D3d):
  1. Hold the slot with another claimant, enqueue the waiter, take the lease.
  2. Pin `admit(..)`. `timeout(COMMAND_HEARTBEAT - 1ms, &mut admit)` must time out; this pins the rate limit.
  3. `timeout(COMMAND_HEARTBEAT * 2, admit)` must return `Err(ToolError("fenced: lease lost"))`.
- `a_beat_on_a_lost_lease_answers_lease_lost` (D3c): claim the row, take the lease, then `timeout(COMMAND_HEARTBEAT * 2, beat(..))` returns `Stopped::LeaseLost`.
- `a_failed_lease_read_does_not_stop_the_beat` (D5):
  1. `store.set_fault(MemFault::LeaseHolds, true)` (`use htui_core::store::mem::MemFault`).
  2. `timeout(COMMAND_HEARTBEAT * 5, &mut beat)` must time out.
  3. Switch the fault off and take the lease; the beat then returns `LeaseLost` within two heartbeats.
- The existing `a_stalled_client_does_not_stop_the_heartbeat` (553) and `..._admission_asks` (576) gain `Lease { run: ids::RUN_2, fence: StepFence::Unleased, step: ids::STEP_R2_PRD }`. The first also asserts `Stopped::Taken`.

*End-to-end tests in `crates/htui-mcp/tests/tools_command.rs`, real time.* They reuse `scope`, `open`, `rows`, `until_status`, `alive`, `until_gone`, `connect` and `queued` from that file, with scope `ToolScope { fence: StepFence::Lease(owner), ..scope(dir.path(), true) }`. The child check uses `pgrep` with a unique sleep length (the precedent uses `sleep 30.4217` to `30.4219`; use `30.4220` and up). A pid file adds nothing over that.
- `a_lost_lease_refuses_the_call_and_queues_nothing` (R1): claim, then take. The call answers `fenced: lease lost` and `rows()` is empty. Control: a scope fenced on `Lease(stranger)` runs `echo ok`.
- `a_lease_lost_while_running_kills_the_child` (R2): wait for `until_status(Running)` and `alive(P)`, then take the lease. The answer is fenced within 20 s. `until_gone(P)` succeeds. The row is `Cancelled` with output starting `[stopped: fenced: lease lost]\n` and `exit_code` None.
- `a_lease_lost_while_queued_cancels_the_row` (D3d): another session holds the build slot (as `progress_ticks_while_queued` does). Once our row is `Queued`, take the lease. The answer is fenced within 25 s, and our row becomes `Cancelled` (poll for it).
- (e) needs no new test. Every existing case in this file already runs `Unleased` on `RUN_2`, whose `lease_owner` is NULL (B-7).

### 5. Task file sets
| Task | Files |
|---|---|
| T1 | `crates/htui-core/src/store/{traits.rs, worker.rs, mem.rs, conformance.rs}`, `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/{worker.rs, writer.rs}`, `crates/htui-store/.sqlx/query-<hash>.json` (new), `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` |
| T2 | `crates/htui-mcp/Cargo.toml`, `Cargo.lock`, `crates/htui-mcp/src/host.rs` |
| T3 | `crates/htui-mcp/src/tools/command.rs`, `crates/htui-mcp/tests/tools_command.rs` |
| T4 | `docs/htui-mcp.md` |

T1∩T2 = ∅, T2∩T3 = ∅, and T1∩T3 = ∅. T3 compiles against T1's trait method and `MemFault::LeaseHolds`. Even with disjoint files, T2 and T3 must not run in parallel on one tree (B-4).

### 6. Hazards and deviations
- **B-1 (plan gap).** `crates/htui-core/tests/mem_store.rs:35-37` also pins `CASES.len() == 148`, with a long message that is not in the plan's file table. Bump it to 149 and append "MOD-78's one for `lease_holds` (plan D1, D2)". In `pg_conformance.rs`, change the doc comment (lines 25-30), the constant (line 31) and the message string "148 since MOD-69's…" (line 38).
- **B-2 (deviation: T3 test placement).**
  - Evidence: `tools/command.rs` has no end-to-end `call` test (its tests call `beat`/`admit` directly, lines 537-590). The process-level suite and its `pgrep` helpers live in `tests/tools_command.rs` (lines 110-130, `a_reaped_claim_stops_its_child`), on real time. Paused time cannot drive `run_shell`'s timers next to a real child (reasoning in section 4).
  - Fix: paused-time unit tests in `command.rs`, real-time end-to-end tests in `tests/tools_command.rs`. That adds one file to T3, still disjoint from T2.
- **B-3 (addition).** T3(d) needs a way to make `lease_holds` fail. `MemFault` (mem.rs:122) covers only writes. Fix: add `MemFault::LeaseHolds`, checked in `MemStore::lease_holds`, in T1 (mem.rs is already T1's file). `htui-mcp`'s dev-dependency enables `htui-core/test-support` (Cargo.toml:38).
- **B-4 (parallelism, hidden coupling).** T2 and T3 both build the `htui-mcp` crate. On a shared tree, a half-edited `host.rs` breaks T3's `cargo test -p htui-mcp`, and the reverse. Run T1 ∥ T2, then T3; or put T2 and T3 in separate worktrees. T1's crate-scoped gates do not compile `htui-mcp`, but a workspace clippy does.
- **B-5.** The `scope()` helper in host.rs tests (lines 581-596) uses `RunId::new()` and `StepId::new()`. After T3, a `command_run` on that scope answers `not found: run <id>` (D1's NotFound). T2's in-flight tests must use `ids::RUN_2` and `ids::STEP_R2_PRD`, as section 3 does.
- **B-6 (clarification of D5).** The plan defines a read error only for (a), which refuses, and the heartbeat, which does not stop. I propose (b) and (d) also do not stop: the claim just succeeded, and (c) catches a real loss within 10 s. `claim_command` errors in `admit` still answer `store_error` as they do today (command.rs, `admit`).
- **B-7 (T3(e) is already covered).** Production chat scopes set `command_queue: false` (`htui/src/agent_worker.rs:1106`, `:2287`). Only the engine's scope exposes `command_run`, with `Lease(parts.owner)` (`htui-orch/src/engine.rs:6352`). The demo store seeds no `lease_owners` (the only inserts are in `claim_run`, `adopt_runs`, `take_lease` and tests), so every existing `tools_command.rs` case is the Unleased regression test.
- **B-8 (E0034).** MemStore implements both `WriteStore` and `WorkerStore` with the same method names. Each test module must import exactly one: `command.rs` tests use `WorkerStore`; `tools_command.rs` and the new host.rs tests use `WriteStore`.
- **B-9 (T4 wording).** The D4 path writes the row after the lease is lost. So the troubleshooting entry "`fenced: lease lost` … Nothing was written" (`docs/htui-mcp.md:448-451`) needs a `command_run` qualifier as well as dropping "never answers this". Line 186 ("a call on a connection still open answers `session ended`") should also cover in-flight calls. The Scope bullet at lines 171-175 and the Liveness section at 292-305 change as the plan says.
- **B-10 (sqlx).** `query_scalar!` needs the `AS "holds!"` non-null override. Prepare with `--all-targets --all-features`, or test-only queries are deleted from `.sqlx` (`docs/hr-sandbox.md:207`).
- **B-11 (timing).** Because (a) counts as the first read, a queued call's first lease read is at 10 s or later. So T3's queued end-to-end test takes about 10 s of wall time, and the running test about 10 s plus the kill grace. The `a_reaped_claim_stops_its_child` precedent already allows 20 s.
- **B-12 (kept behaviour).** `Served::call` keeps `has_ended()` before the `advertised` check. An ended session must still answer `session ended` rather than `unknown tool` (the `dropping_the_last_host_…` test, host.rs:770-790).

### Build sequence
1. T1: write the conformance case (red), then the trait, Mem with `MemFault`, Pg with `.sqlx`, the forwarders, and the count bumps (B-1).
2. T2, in parallel with T1: the host tests (red), then the token, select, closure, `close()` and `Drop`.
3. T3, after T1 and T2: the unit and end-to-end tests (red), then `Lease`, `Stopped`, the four checks and the D4 path.
4. T4: the docs (B-9), then the full validation from the plan, including the featureless clippy and `--test-threads=1`.