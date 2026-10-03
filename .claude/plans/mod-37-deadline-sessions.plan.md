# Plan: MOD-37 milestone 4 - Deadline and sessions

**Source PRD**: `.claude/prds/mod-37-orchestrator-hardening.prd.md`
**Selected Milestone**: 4 - Deadline and sessions (ANA-27 §5.1 T4 deadline, R-49, R-46)
**Complexity**: Medium
**Routing**: PRD path, M4 planned on its own (maintainer-confirmed 2026-10-03); ultracode not needed.
**Status**: confirmed 2026-10-03 - implementation

## Summary
Three fixes, one per risk. **Deadline**: a step session (and each fan-out candidate's) runs under a
timer for what is left of `deadline_seconds`. When it fires, the walk cancels the session through
MOD-42's graceful path and settles the step as `DeadlineElapsed`, the way the run-cap breach already
settles as `CapBreached`. **R-49**: a promoted chat over a `shared_serialized` step takes the
`(box, repo)` guards again before its chat is bound and holds them until the chat task ends. If
another run holds them, it publishes `Waiting` and waits. **R-46**: when the TUI's store loop drops
`Online → Offline`, it preempts every live walk at once, so the walk is abandoned now and the
reconnect sweep adopts it, instead of running blind until the heartbeat fence (~80 s with defaults).

## Maintainer decisions (2026-10-03)
| Question | Decision |
|---|---|
| R-49: guard held by another run when a promotion attaches | **Wait** with a `FrameKind::Waiting` frame (R-51 precedent), cancellable like `lock_announcing` |
| R-46: when to preempt | **At once** on the `Online → Offline` swap; a short blip that the pool would have survived now ends the session, and the sweep adopts the run as a new attempt |
| Deadline scope | **Step sessions and fan-out candidates**; judge calls stay untimed (they have no `started_at` of their own) |

## Design

### D1 - Deadline: a step-local control, an explicit cut flag
- `Engine::drive_once` gains `deadline: Option<std::time::Duration>` (the time left, from the
  existing `Engine::remaining(phase, started_at, self.now())`, `engine.rs:3711`). `session` and
  `candidate_live` compute it; `judge_calls` passes `None`.
- With `Some(left)`, `drive_once` makes a step-local `control_channel()` (`relay.rs:95`) and
  drives `drive(..)` against it. Beside it runs a forwarder that copies the run control's
  signal into the local channel, and a `tokio::time::sleep(left)` that, on firing, sends
  `Signal::Cancel { grace: RELAY_GRACE }` and marks the session **cut**. Both are polled in one
  boxed `select!` with the drive future (stack headroom, `every_case_name_dispatches`). `None`
  keeps today's path byte-for-byte.
- `drive` then cancels gracefully (answers parked requests `cancelled`, `session.cancel(grace)`,
  bounded drain) and returns `Err(DriverError::Cancelled)`. `drive_once` maps that to:
  - `EngineError::Cancelled` when the **run** control is a cancel (unchanged MOD-42 behaviour; a
    real cancel wins over a deadline that fired in the same instant);
  - otherwise, when cut, to `Ok(Ok(DoneEvent { stop_reason: Cancelled, .. }))`, the
    `record::enforce_breach` shape (`htui-agent/src/record.rs:1910-1933`), plus `cut = true`.
- The cut flag travels up: `drive_once` → `session` / `candidate_live` → `SettleInput`, which
  gains `deadline_cut: bool`. `gate::settle` checks `input.deadline_cut || deadline_elapsed(input)`
  at the existing `DeadlineElapsed` rule (after `CapBreached`). The flag exists because the engine
  clock (`TestClock` in tests) and tokio time are independent: settle must not depend on the two
  agreeing at the boundary (`deadline_elapsed` is a strict `>`).
- What follows is unchanged: verify gets `Some(ZERO)` and answers `unavailable` without spawning,
  capture runs, `gate::apply` parks (`always`/`on_failure`) or retries/fails (`never`) as for any
  `DeadlineElapsed` today. `recover.rs`'s `SettleInput` passes `deadline_cut: false`.

### D2 - R-49: an owned hold on the shared guards
- New `Isolator` verb `hold(&self, trees: &[RunStepTree]) -> IsolatorFuture<'_, Hold>`, where
  `Hold` is a concrete newtype over `Vec<tokio::sync::OwnedMutexGuard<()>>`. It derives
  `Debug`, it is `Send + Sync`, and both isolators' guards are this type. It takes the `(box, repo)` guards of the rows whose mode is
  `SharedSerialized`, in sorted `RepoId` order as `GixIsolator::acquire` does
  (`isolate/real.rs:510-530`). It never records them in `held`, so `capture`, `release(run)` and
  `cleanup` cannot free a chat's guard. Rows of any other mode take nothing (an empty `Hold`).
  `local` stays unguarded, as it is for walks today.
- Implementations: `GixIsolator` (real), `FakeIsolator` (`fake.rs`, its one global `serial`
  mutex), and `StallAfterReconcile` (`tests/gix_isolator.rs`, delegates).
- `htui-worker` `on_run_unless_claimed`, `CommandOutcome::Promoted` arm (`runtime.rs:~2370`):
  after the `Promoted` answer and the `Rested` frame, read `kit.writer.step_trees(step)`, then
  take the hold. Try first: a free hold is taken silently. A busy one publishes
  `FrameKind::Waiting` once and selects on `walk.token.cancelled()`, `cancel_signalled`, and
  `kit.isolator.hold(..)`, as `Shared::lock_announcing` does (`runtime.rs:470-490`). The try
  needs a non-blocking form: either `hold` takes a `waiting: impl FnOnce()` callback or the trait
  also gets `try_hold`. The blueprint picks one.
- The `Hold` moves into `ChatEnd` (new field), and `ChatEnd` loses its unused `#[derive(Clone)]`
  (nothing clones it; `RunServed` derives only `Debug`). So `ChatEnd::after` drops it when the chat task
  returns (cancel, end, panic). The `Reply`/`Deferred` paths drop `ended`, and with it the hold.
- If the wait is cancelled (the run is cancelled or preempted while waiting), the chat is not
  bound and the requester gets a failure sentence ("the step was promoted, but its chat was not
  started: …"), mirroring the existing "could not be read to bind its chat" answer. A
  `step_trees` read failure answers the same way. The run guard stays held for the wait, so no
  other command moves the run between `Promoted` and the bind.

### D3 - R-46: preempt walks on the offline swap
- `RunRuntime::preempt_walks(&self)` calls `self.shared.walks.preempt_all()` (`runtime.rs:723`)
  and nothing else. It is unlike `forget_server`, which also bumps `server` (an isolator rebuild)
  and clears the queue, both wrong for the same server.
- `store_worker::go_offline` returns whether the backend actually went `Online → Offline`
  (`Backend::went_offline`'s bool). Its two loop call sites (`store_worker.rs:2428`, `:2528`) call
  `runs.preempt_walks()` when it did. The two test call sites ignore the bool.
- Each preempted walk ends as a `SetDsn` preempt does: `walked` → `None` → `engine.abandoned(run)`
  → `release_lease` fails offline → `dead_walks` → the `ConnEvent::Online` sweep adopts it
  (D175/D190). The requester of a live command gets `PREEMPTED`, as for a server switch.
- A `worker` box's runs are walked by `htui worker`, which has no offline swap; the TUI holds no
  walk for them, so `preempt_all` has nothing of theirs to cancel.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Cut session settles as a failure, not a cancel | `crates/htui-agent/src/record.rs:1910-1933` `enforce_breach` | cancel with grace, drain, return `Ok(DoneEvent{stop_reason: Cancelled})` |
| Boxed driving future | `crates/htui-orch/src/engine.rs` `drive_once` (`Box::pin(drive(..))`) | box any future that wraps the session |
| Remaining deadline | `engine.rs:3711` `Engine::remaining` | `None` = no deadline, `Some(ZERO)` = passed |
| Settle rule order | `crates/htui-orch/src/gate.rs:239-278` | first hit wins; `DeadlineElapsed` after `CapBreached` |
| Waiting announce | `crates/htui-worker/src/runtime.rs:470-490` `lock_announcing` | try, announce once, select on cancel |
| Ordered guard acquire | `crates/htui-orch/src/isolate/real.rs:510-530` `acquire` | sorted `RepoId`, `lock_owned` |
| Lifetime of a chat | `crates/htui-worker/src/address.rs:136-148` `ChatEnd::after` | the one end point of a promoted chat |
| Walk preempt | `crates/htui-worker/src/runtime.rs:1351-1359` `forget_server` | `walks.preempt_all()` |
| Errors | `EngineError`/`IsolateError` | the hold's failure is an `IsolateError`; the worker answers a sentence |
| Tests | `engine.rs` `mod tests` with `#[tokio::test(start_paused = true)]` and `SuspendingDriver`; `run_worker.rs` worker tests with `Stalled`; `isolate/real.rs` lock tests | one regression test per risk, red before the fix |

## Files to Change
| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-orch/src/engine.rs` | UPDATE | `drive_once` deadline arm + cut flag; `session`/`candidate_live` thread it; `SettleInput` sites; tests | T1 |
| `crates/htui-orch/src/gate.rs` | UPDATE | `SettleInput::deadline_cut`, settle rule; unit tests' literals | T1 |
| `crates/htui-orch/src/recover.rs` | UPDATE | `SettleInput { deadline_cut: false, .. }` | T1 |
| `crates/htui-orch/src/isolate.rs` | UPDATE | `Isolator::hold` (+ try form), `Hold` type | T2 |
| `crates/htui-orch/src/isolate/real.rs` | UPDATE | `GixIsolator::hold`; lock test | T2 |
| `crates/htui-orch/src/fake.rs` | UPDATE | `FakeIsolator::hold` | T2 |
| `crates/htui-orch/tests/gix_isolator.rs` | UPDATE | `StallAfterReconcile` delegates `hold` | T2 |
| `crates/htui-worker/src/address.rs` | UPDATE | `ChatEnd` owns the `Hold` | T2 |
| `crates/htui-worker/src/runtime.rs` | UPDATE | T2: hold in the `Promoted` arm. T3: `RunRuntime::preempt_walks` | T2, T3 |
| `crates/htui/src/store_worker.rs` | UPDATE | `go_offline` returns the swap; callers preempt; test | T3 |
| `crates/htui/src/run_worker.rs` | UPDATE | worker-level regression tests for R-49 and R-46 | T2, T3 |
| `HANDOFF.md`, PRD, `docs/decisions/mod/mod-37.md` (if present) | UPDATE | close-out | T4 |

## Task independence
| Task | Files | Intersects |
|---|---|---|
| T1 | engine.rs, gate.rs, recover.rs | none by file, but `htui-orch` is compiled by T2's and T3's crates |
| T2 | isolate.rs, real.rs, fake.rs, gix_isolator.rs, address.rs, runtime.rs, run_worker.rs | T3: runtime.rs, run_worker.rs |
| T3 | runtime.rs, store_worker.rs, run_worker.rs | T2 |

**Serial: T1 → T2 → T3.** T2 ∩ T3 is non-empty. T1 is file-disjoint, but on one shared tree its
half-edited `htui-orch` breaks T2's and T3's builds, and a worktree split (~10 GB of target per
worktree) is not worth it for three medium tasks.

## Tasks
### Task 1: Deadline timer (ANA-27 T4)
- **Tests first** (red on today's code):
  - `engine.rs`: `a_hung_session_is_cut_at_the_step_deadline_and_settles_deadline_elapsed`.
    `start_paused`, `step_deadline_seconds = 1`, a `SuspendingDriver`-style session that never
    sends `done`. The walk returns, the step is `failed`/parked with note "deadline elapsed",
    the session saw `cancel(grace)`, and no `EngineError::Cancelled` is raised.
  - `engine.rs`: the same for one fan-out candidate.
  - `engine.rs`: a run cancel that lands before the timer still ends as `Cancelled` (MOD-42
    unchanged).
  - `engine.rs`: a session that finishes before the deadline is not cut (the timer is dropped).
  - `gate.rs`: `deadline_cut` settles `DeadlineElapsed` even with `now` before the deadline, and
    ranks after `CapBreached`.
- **Action**: D1.
- **Validate**: `cargo nextest run -p htui-orch --all-features --no-fail-fast` (grep SIGABRT),
  `cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`.

### Task 2: R-49 - a promoted chat holds the shared guard
- **Tests first**:
  - `isolate/real.rs`: `hold_takes_the_shared_serialized_guard_until_dropped`: a `prepare` of
    another run on the same `(box, repo)` times out while the `Hold` lives and proceeds after the
    drop; `release(run)`/`cleanup` do not free it.
  - `isolate/real.rs`: a `worktree`/`copy`/`local` row's hold takes nothing.
  - `run_worker.rs`: a promotion of a `shared_serialized` step whose guard another run holds
    publishes `Waiting`, binds the chat once the guard is free, and holds it until the chat task
    ends. A second walk's `prepare` on the repo waits for the chat end.
  - `run_worker.rs`: a cancel while the promotion waits answers the failure sentence and binds
    nothing.
- **Action**: D2.
- **Validate**: `cargo nextest run -p htui-orch -p htui-worker -p htui --all-features
  --no-fail-fast`, clippy as above for the three crates.

### Task 3: R-46 - the offline swap preempts live walks
- **Tests first**:
  - `run_worker.rs`: `an_offline_swap_preempts_the_walk_and_the_sweep_adopts_it`, built from
    `a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after` (`run_worker.rs:2455`). After
    `preempt_walks` the session is dropped at once (not at the fence), and after reconnect the
    sweep walks attempt 2.
  - `store_worker.rs`: the swap call sites preempt only on a real `Online → Offline` (not on
    `Offline`/`Memory`). Extend `go_offline_drops_the_health_watch_on_a_backend_that_is_not_online`
    to assert the returned bool.
- **Action**: D3.
- **Validate**: `cargo nextest run -p htui-worker -p htui --all-features --no-fail-fast`.

### Task 4: Close-out
- HANDOFF MOD-37: phase 4 note, the deadline entry, R-46 and R-49 struck through as "closed by
  MOD-37 phase 4". PRD row 4 → complete. `.claude/plans/mod-4-orch-drive.plan.md:651` and drive
  blueprint §18's R-49 row get a pointer. Validator green.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features --no-fail-fast 2>&1 | tee /tmp/m4.log; grep -c SIGABRT /tmp/m4.log
# keyring fake is process-wide: confirm htui with --test-threads=1 if anything flakes
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
No SQL changes, so no `sqlx prepare`.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| The deadline `select!` grows the walk future past the 2 MiB debug test stack | M | box it; gate with `--no-fail-fast`, grep SIGABRT |
| A deadline and a run cancel firing together pick the wrong ending | L | the run control is read first; a test pins it |
| R-49 wait holds the run lock indefinitely if the other holder is a long human chat | M | the `Waiting` frame shows it and a cancel ends it; the pane's text "waiting for the walk" is approximate when the holder is another chat (accepted, LOW) |
| R-49 hold vs. the walk's own `acquire` deadlock | L | both take guards in sorted `RepoId` order; a hold never also takes a run lock of another run |
| R-46 kills a session on a brief blip | accepted | maintainer decision; adoption walks a new attempt |
| Paused tokio time and `TestClock` disagree in tests | M | the `deadline_cut` flag makes settle independent of the engine clock |

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| `Engine::remaining` returns `None`/`Some(ZERO)` as D1 uses it | true | `engine.rs:3711-3723` |
| `drive_once` has exactly three callers: `session`, `candidate_live`, `judge_calls` | true | `engine.rs:5788` (in `session`), `:4155` (in `candidate_live`, `:4074`), `:5054` (in `judge_calls`, `:5031`) |
| `session` has one caller, inside the walk that holds `started_at` | true | `engine.rs:3520`; `started_at` is passed to `verify` and `SettleInput` at `:3545`, `:3565` |
| `candidate_live` has its own `started_at` in scope at the session | true | `engine.rs:4196` (`verify`), `:4207` (`SettleInput`) |
| `control_channel`, `Signal`, `RELAY_GRACE` are reachable from `htui-orch` | true | `htui-agent/src/record.rs:123` re-exports them; `engine.rs:30` already imports `RELAY_GRACE` |
| A cancel through `drive` returns `Err(DriverError::Cancelled)`, which `drive_once` maps to `EngineError::Cancelled` | true | `relay.rs` `drive` doc ("Errors"); `engine.rs` `drive_once` match on `driven` |
| `enforce_breach` returns `Ok(DoneEvent{stop_reason: Cancelled})` after cancel + drain | true | `htui-agent/src/record.rs:1910-1933` |
| `DoneEvent` has the single field `stop_reason` (constructible in `drive_once`) | true | `htui-agent/src/event.rs:495-498` |
| Settle order: driver `Err` → `Stopped` → `CapBreached` → `DeadlineElapsed`, strict `>` | true | `gate.rs:239-290` |
| `SettleInput` is built in `gate.rs`, `engine.rs`, `recover.rs` only (13 literals) | true | repo-wide search, 13 hits in those three files |
| The engine already boxes `drive` for stack headroom | true | `engine.rs` `drive_once`: `Box::pin(drive(..))` with the `every_case_name_dispatches` comment |
| `SuspendingDriver` and paused-time engine tests exist to build T1's test on | true | `engine.rs:9083`; 31 `start_paused = true` in `engine.rs` |
| `GixIsolator::acquire` takes guards in sorted `RepoId` order with `lock_owned`, recorded in `held` | true | `isolate/real.rs:510-530` |
| `Isolator` has three implementations | true | `fake.rs:451`, `isolate/real.rs:1427`, `tests/gix_isolator.rs:2004`; none elsewhere in `crates/` |
| `Kit` carries the isolator and the writer in the `Promoted` arm | true | `runtime.rs:848-851`; `kit` is in scope in `on_run_unless_claimed` (`:2349`) |
| The run guard is held through the `Promoted` arm and dropped at the end | true | `runtime.rs:2340-2346` taken, `drop(guard)` after the match |
| `lock_announcing` is the try/announce/select precedent | true | `runtime.rs:470-490` |
| `ChatEnd` is built once and never cloned | true | built only at `runtime.rs:2415`; no `.clone()` of it in `crates/`; `RunServed` is `#[derive(Debug)]` only (`address.rs:97`) |
| `ChatEnd::after` is the chat's one end point; refusals drop `ended` | true | `address.rs:136-148`; `store_worker.rs:1965-1972`; `testkit.rs:193-207` passes it through |
| `testkit.rs` does not construct a `ChatEnd` | true | `testkit.rs` only destructures and calls `ended.after` |
| `OwnedMutexGuard<()>` is `Send + Sync + Debug` (toolchain) | true | tokio 1.53.1 (`Cargo.lock:6645`), `sync/mutex.rs:261` (`Sync`), `:1173` (`Debug`); `Send` is auto over `Arc<Mutex<()>>` |
| `walks.preempt_all` cancels every live walk's token | true | `runtime.rs:723-727` |
| Every walk entry point is a `walks.child`, so `preempt_all` reaches sweeps too | true | `runtime.rs:1785, 2068, 2232, 2340, 2558, 2686, 2712` |
| `walked` returns `None` on a cancelled token | true | `runtime.rs:1641-1646` |
| `Backend::went_offline` is `true` only for `Online → Offline` | true | `htui-store/src/backend.rs:200-217` |
| `go_offline` has two loop call sites and two test call sites; `runs` is in scope in the loop | true | `store_worker.rs:2428, 2528` (loop), `:3881, :4684` (tests); `mut runs: RunRuntime` at `:2024` |
| Only the TUI store loop swaps `Online → Offline` (`htui worker` does not) | true | `went_offline` called only from `store_worker.rs:2693` outside `backend.rs` tests |
| R-46 template and switch tests exist | true | `run_worker.rs:2455`, `:1984`; `Stalled` at `:497` |
| Task independence | T1 file-disjoint; T2 ∩ T3 = {`runtime.rs`, `run_worker.rs`} | file table above → serial T1 → T2 → T3 |

## Acceptance
- [ ] Each risk has a regression test that is red on the old code
- [ ] A stalled fake driver settles as `DeadlineElapsed` at the step deadline (PRD metric)
- [ ] fmt, clippy, workspace nextest green, no SIGABRT
- [ ] rust-reviewer run, findings applied or deferred with the maintainer
- [ ] HANDOFF/PRD updated, validator green
