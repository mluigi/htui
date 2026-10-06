# Plan: MOD-78 — `command_run` lifecycle: lease check and cancel on session end

**Source**: HANDOFF.md MOD-78 (from MOD-11, `docs/decisions/mod/mod-11.md` I-3 and review L5)
**Route**: plan (C3 only, weak; maintainer accepted 2026-10-06)
**Complexity**: Medium
**Status**: done 2026-10-07 — `docs/decisions/mod/mod-78.md`

## Summary

`command_run` is the one htui tool whose writes are not fenced. A session whose walk has lost its
lease can queue a command and keep running it until the walk notices the loss at its next renewal.
Ending a session also only sets `Session.ended`, which is read at the *start* of a call, so an
in-flight call outlives its session. This plan adds two things:
1. **A lock-free lease read** (`lease_holds`), checked before the enqueue, after admission and on
   every heartbeat. A loss refuses the call or kills the child.
2. **A `CancellationToken` on `Session`** that replaces `ended: AtomicBool` and that `Served::call`
   selects against. An in-flight call then ends with its session: the row is cancelled and the
   process group killed, both through the drop paths that already exist.

## Requirements restated

- R1. Before `enqueue_command`, read the run's `lease_owner` with no row lock. If it is not the
  session's fence owner, answer `fenced: lease lost` and write nothing.
- R2. On each `COMMAND_HEARTBEAT` of a running command, read the lease the same way. If it is lost,
  kill the child, end the row as `cancelled`, and answer `fenced: lease lost`.
- R3. Dropping the `ToolLease` (or `McpHost::close`) ends every in-flight call of that session
  promptly. The call answers `session ended`, the row is `cancelled` (`Enqueued` drop) and the
  child's process group is killed (`run_shell` `kill_on_drop` + `GroupGuard`).
- R4. The docs (`docs/htui-mcp.md`) stop saying `command_run` is unfenced.

## Decisions (for CONFIRM)

| # | Decision | Why |
|---|---|---|
| D1 | New store read `lease_holds(run: RunId, fence: StepFence) -> Result<bool>` on `WriteStore`, forwarded through `WorkerStore`. Postgres: `SELECT lease_owner IS NOT DISTINCT FROM $2 FROM run WHERE id = $1`, a plain read with no `FOR SHARE`. Mem: `lease_owners.get(&run).copied() == fence.owner()`. A missing run is `NotFound { entity: "run" }`. | `lease_owner` is deliberately not a `Run` field (`model/run.rs:216`), so no current read exposes it. This uses the same predicate as every fenced write (`IS NOT DISTINCT FROM`, `traits.rs` `StepFence` doc). Keeping it a separate read leaves the five queue methods, and their forwarders, unchanged. |
| D2 | The check is owner-only, not expiry. | Same as every other fence (`fence_holds`, mem.rs:1712). An expired lease that nobody has taken still lets its owner write. |
| D3 | When to check: (a) before the enqueue; (b) once after admission, before `run_shell`; (c) each running heartbeat, inside `beat`; (d) during the queued wait, at most once per `COMMAND_HEARTBEAT` (not on every 1 s ask). | The item asks for (a) and (c). Without (b), a call that waited in the queue would spawn its child for up to 10 s on a lost lease. Without (d), a waiter on a lost lease would sit in the queue until the walk notices. The rate limit in (d) caps the extra reads at the heartbeat cadence. |
| D4 | A lease loss mid-run kills the child (the `beat` future returns, which is `run_shell`'s `stop`). The call then runs `finish_command(id, claimant, Cancelled, None, Some("[stopped: fenced: lease lost]\n" + tail))` and answers the error `fenced: lease lost`. A loss before the run calls `cancel_command` (through the armed `Enqueued` guard) and answers the same error. | The answer matches the other fenced tools, so the agent sees one reason. The row keeps what the command printed, scrubbed as before. |
| D5 | A failed `lease_holds` read is not a stop on the heartbeat path (as with `beat_command`). It refuses the call before the enqueue (`store unavailable: …`). | A store blip must not kill a healthy build. A lasting outage already makes the row go stale and get reaped. |
| D6 | Cancel signal: `tokio_util::sync::CancellationToken`, which **replaces** `ended: AtomicBool` (`is_cancelled()` for `has_ended`, `cancel()` in the lease drop and `close()`). `Served::call` runs `tokio::select! { biased; () = token.cancelled() => Ok(session_ended()), r = dispatch => r }`. | One source of truth instead of a flag plus a signal. `tokio-util` is already a workspace dependency (`Cargo.toml:97`) and its `sync` module is not behind a feature flag. A `watch` would need a sender and a receiver per call for the same effect. |
| D7 | `Inner<H>` gets a `Drop` that cancels every remaining session's token. | Dropping the last `McpHost` already ends *new* calls through the `Weak` in `Bound`. An in-flight call holds `Arc<Served>`, so it needs the token to end with its host as well. |

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Store read on `WriteStore` | `traits.rs:1896-1903` `item_by_key` | Read on `WriteStore` "by the `command_runs` precedent", forwarded through `WorkerStore`, `Writer`, `UsageSpy`, `SpyStore` |
| Fence predicate | `pg/write.rs:1278` | `r.lease_owner IS NOT DISTINCT FROM $n` |
| Mem fence | `mem.rs:1707-1716` `State::fence_holds` | `lease_owners.get(&run).copied() == fence.owner()` |
| Tool errors | `tools/mod.rs:126` `store_error` | `StoreError::Fenced` → `"fenced: lease lost"`; reuse the string, do not add a variant |
| Drop-path cleanup | `tools/command.rs` `Enqueued` | Cancelled call → `cancel_command` in a contained task |
| Paused-time tests | `tools/command.rs` `a_stalled_client_does_not_stop_the_heartbeat` | `#[tokio::test(start_paused = true)]`, `MemStore::demo()`, `COMMAND_HEARTBEAT * n` bounds |
| Conformance cases | `conformance.rs:200` `CASES` + `run_case`; `htui-store/tests/pg_conformance.rs:31` `EXPECTED_CASES` | One named case per behaviour; bump the Pg count |

## Files to change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/store/traits.rs` | UPDATE: `WriteStore::lease_holds` | T1 |
| `crates/htui-core/src/store/worker.rs` | UPDATE: `WorkerStore::lease_holds` + MemStore forward | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE: `MemStore::lease_holds` | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE: case `lease_holds_reads_the_owner` + `CASES` | T1 |
| `crates/htui-store/src/pg/write.rs` | UPDATE: `PgStore::lease_holds` | T1 |
| `crates/htui-store/src/worker.rs` | UPDATE: PgStore + Writer `WorkerStore` forwards | T1 |
| `crates/htui-store/src/writer.rs` | UPDATE: `Writer` `WriteStore` forward | T1 |
| `crates/htui-store/.sqlx/query-<hash>.json` | CREATE: offline entry for the new SELECT | T1 |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE: `EXPECTED_CASES` 148 → 149 | T1 |
| `crates/htui-agent/src/conformance.rs` | UPDATE: `UsageSpy` forward | T1 |
| `crates/htui-agent/tests/recorder.rs` | UPDATE: `SpyStore` forward | T1 |
| `crates/htui-mcp/Cargo.toml`, `Cargo.lock` | UPDATE: `tokio-util = { workspace = true }` | T2 |
| `crates/htui-mcp/src/host.rs` | UPDATE: token replaces `ended`; `Served::call` select; `Inner` drop | T2 |
| `crates/htui-mcp/src/tools/command.rs` | UPDATE: D3/D4 checks; `beat` returns why it stopped | T3 |
| `docs/htui-mcp.md` | UPDATE: Scope bullet, Liveness bullets, `fenced: lease lost` troubleshooting | T4 |

## Tasks

### T1: `lease_holds` store read (TDD)
- **Action**: Write the conformance case first: claim a run under `owner` → `Lease(owner)` true,
  `Lease(other)` false, `Unleased` false. `take_lease` by `other` → `Lease(owner)` false. Released →
  `Unleased` true. A chat run (`lease_owner` NULL) → `Unleased` true. A missing run → `NotFound`.
  Then add the trait method, the Mem and Pg implementations, every forwarder, the `.sqlx` entry
  (prepare against a migrated scratch DB on `localhost:5439`) and the `EXPECTED_CASES` bump.
- **Mirror**: `item_by_key` (trait + forwarders), `fence_holds` (mem).
- **Validate**: `cargo test -p htui-core --all-features conformance`;
  `cargo test -p htui-store --all-features --test pg_conformance` (sandbox Postgres);
  `cargo sqlx prepare --check` equivalent per `docs/hr-sandbox.md`.

### T2: Session cancellation signal (TDD), independent of T1
- **Action**: Tests first, in `host.rs`:
  (a) an in-flight call (a test-only slow tool, or `command_run` of `sleep 600` on a demo step)
  answers `session ended` within a bound after `drop(lease)`;
  (b) the same after `host.close()`;
  (c) the same after dropping the last `McpHost` (D7);
  (d) the existing `client_refuses_an_unknown_token` and `dropping_the_last_host_…` tests stay green.
  Then: `ended: AtomicBool` → `cancel: CancellationToken`; `has_ended` → `is_cancelled`; the lease
  closure and `close()` call `cancel()`; `Served::call` uses a `biased` select; `impl Drop for Inner`.
- **Mirror**: the existing `session_ended()` answer; contained spawns.
- **Validate**: `cargo test -p htui-mcp`.

### T3: `command_run` lease checks (TDD), after T1
- **Action**: Tests first, in `tools/command.rs` (paused time, `MemStore::demo()`):
  (a) lease taken before the call → `fenced: lease lost` and no `command_run` row on the step;
  (b) lease taken while queued → the waiter stops within `COMMAND_HEARTBEAT * 2`, the row is
  `cancelled`, and the answer is fenced;
  (c) lease taken while running (`sleep 600`) → the child is dead (pid probe), the row is
  `cancelled` with the `[stopped: fenced: lease lost]` note, and the answer is fenced;
  (d) a `lease_holds` error mid-run does not stop the command (D5);
  (e) an `Unleased` chat scope on a lease-less run runs normally.
  Then: `beat` returns `enum Stopped { Taken, LeaseLost }`, `admit` checks once per heartbeat
  interval, and the post-admission check and the D4 finish path are added.
- **Mirror**: `beat` / `admit` / `Enqueued` as they are; `store_error`.
- **Validate**: `cargo test -p htui-mcp`.

### T4: Docs, after T2 and T3
- **Action**: `docs/htui-mcp.md`: the Scope bullet becomes "`command_run` is fenced too" (check
  before queueing, at admission, on every heartbeat; a loss kills the command and cancels the row).
  Liveness gains "a session that ends mid-call ends the call". The `fenced: lease lost`
  troubleshooting drops "`command_run` never answers this".
- **Validate**: validator (below); the doc's anchors resolve.

## Parallelism (file-set intersection)

T1 ∩ T2 = ∅ (T1: htui-core/htui-store/htui-agent; T2: `htui-mcp/Cargo.toml`, `Cargo.lock`,
`htui-mcp/src/host.rs`), so **T1 ∥ T2**. T3 needs T1's trait and touches only `tools/command.rs`
(∩ T2 = ∅), so it runs after T1, in parallel with T2 if T2 is still running. T4 runs last.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings                       # featureless gate
cargo test --workspace --all-features -- --test-threads=1     # grep SIGABRT; qdrant_live flakes re-run serially
cargo doc --workspace --no-deps --all-features                # RUSTDOCFLAGS=-D warnings
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| TOCTOU: the lease is lost just after the pre-enqueue read | High (inherent) | Bounded by D3(c): at most one `COMMAND_HEARTBEAT` (10 s) of a running child. That is far below the lease renewal window the item complains about. |
| Extra Pg reads per command | Low | One read per 10 s per running or queued call, by primary key. |
| A `select!` drop leaves the row `running` | Low | `Enqueued`'s drop cancels it; T2 test (a) asserts the row's status. |
| A child survives the drop on Windows | Low | `run_shell`'s `GroupGuard` (job object) is unchanged; Windows verification is MOD-16. |
| `.sqlx` offline drift | Medium | Prepare against a migrated scratch DB (memory note); `--check` in the gate. |
| A real-process test under paused time is flaky | Medium | Bound waits with `COMMAND_HEARTBEAT * n`; if auto-advance races the child, fall back to real time with one ~10 s test. |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `lease_owner` is not readable through `Run` or any non-test store method | TRUE | `model/run.rs:216` doc; only `tests/worker_pg.rs:470` / `worker_crash_pg.rs:707` helpers read it |
| Every fenced write uses `lease_owner IS NOT DISTINCT FROM` the fence owner | TRUE | `traits.rs` `StepFence` doc; `pg/write.rs:1278,2069,4910,5631`; `mem.rs:1716` |
| `enqueue_command` / `beat_command` have 12 / 11 definition sites (trait, WorkerStore, Mem×2-3, Pg×2, Writer×2, UsageSpy, SpyStore) | TRUE | gortex text search `fn enqueue_command` / `fn beat_command` |
| `ToolScope` carries `run_id` and `fence` | TRUE | `host.rs` tests `scope()` builds `run_id`, `fence: StepFence::Unleased` |
| `beat` returning ends `run_shell` and kills the group | TRUE | `verify.rs:537-543` doc: `stop` resolving kills the whole process group |
| Dropping the `run_shell` future kills the child | TRUE | `verify.rs:560` `kill_on_drop(true)`; `verify.rs:668` `impl Drop for GroupGuard` |
| A dropped call cancels its row | TRUE | `tools/command.rs` `Enqueued::drop` → `cancel_command` in a contained task |
| `finish_command` accepts `Cancelled` | TRUE | `traits.rs` doc "`running → status` (`done \| failed \| cancelled`)"; `command_finish_status` rejects only queued/running |
| `StoreError::Fenced` already maps to `fenced: lease lost` | TRUE | `tools/mod.rs:128` |
| `Session` is constructed only in `McpHost::open`; `ended` is read only by `has_ended` | TRUE | gortex text `command_slot:` (1 construction, host.rs:403); `ended` uses host.rs:82,100,396,430,443 |
| `tokio-util` is a workspace dep and `sync::CancellationToken` is not feature-gated | TRUE | `Cargo.toml:97`; `tokio-util-0.7.19/src/lib.rs:57` `pub mod sync;` unconditional; `sync/mod.rs:5` re-export |
| `htui-mcp` does not depend on `tokio-util` yet | TRUE | `crates/htui-mcp/Cargo.toml` |
| Pg conformance pins the case count | TRUE | `htui-store/tests/pg_conformance.rs:31` `EXPECTED_CASES: usize = 148` |
| `COMMAND_HEARTBEAT` = 10 s, stale after 30 s; default lease TTL 120 s | TRUE | `traits.rs:2601,2605`; `htui-orch/src/recover.rs:31` |
| T1 ∩ T2 and T2 ∩ T3 file sets are empty | TRUE | Files-to-change table above |

## Acceptance

- [ ] R1-R4 hold, each pinned by a test named in T1-T3
- [ ] Validation passes (featureless clippy included)
- [ ] `docs/htui-mcp.md` no longer calls `command_run` unfenced
- [ ] HANDOFF close-out + `docs/decisions/mod/mod-78.md` per `references/lifecycle.md`
