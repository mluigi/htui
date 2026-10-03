# Blueprint: MOD-37 milestone 4 - Deadline and sessions (ANA-27 T4, R-49, R-46)

**Contract**: `.claude/plans/mod-37-deadline-sessions.plan.md` (CONFIRMED 2026-10-03). Scope unchanged;
deviations are listed at the end, each with evidence.
**Order**: serial T1 → T2 → T3 → T4 (docs). Implementers commit per step (§ Commits).
**Line numbers** are from `8537da78`. The plan's line references have drifted: `walk_live_step` `:3417`,
`candidate_live` `:4074`, `judge_calls` `:5031`, `session` `:5769`, `drive_once` `:5870`,
`Engine::remaining` `:3711`, `on_run_unless_claimed` `:2315` (Promoted arm `:2370-2431`).

## Design decisions
- **D1 composition.** `drive_once` keeps today's `Box::pin(drive(..))` for `deadline == None`. For
  `Some(left)` it boxes one helper future, `drive_with_deadline`, that runs `drive` against a
  step-local `Control` and, in the same `select!`, a forwarder that never ends. The forwarder
  forwards **cancels only** from the run control and, when its timer fires, sends
  `Cancel { grace: RELAY_GRACE }` unless a cancel is already there. It reports `cut = true` only
  when its own cancel was the one it put in. `drive_once` reads the run control again after `drive`
  returns, so a run cancel wins over a deadline in the same instant, and wins during the drain too.
  `drive` borrows only the local control and the forwarder borrows only the run control, so there
  is no borrow conflict.
- **D1 return shape.** A small private `Driven { result, cut }` replaces the bare `SessionResult` as
  `drive_once`'s `Ok`. That is clearer than a `(SessionResult, bool)` tuple at three sites.
- **D1 no recorder row.** A cut does not write an htui `error` row. `drive`'s cancel path already
  drains and records the transport's own `done {cancelled}`, and answers parked requests (I-7).
  The reason is visible as the settle's item note "deadline elapsed", which `gate::apply`
  (`fan_out = 1`) and `fail_candidate` already write. `record_cap_breach` writes its row because a
  cap breach is the recorder's own decision, and nothing else would name it. A `tracing::warn!`
  marks the cut.
- **D1 verify.** `VerifyStage` gains `deadline_cut`. A cut session's verify gets `Some(ZERO)`
  (`unavailable`, nothing spawned) whatever the engine clock says. The plan's "verify gets
  `Some(ZERO)`" holds only when the engine clock and tokio agree (deviation 3).
- **D2 try form: `try_hold`, not a callback.** `Isolator` is used as `&dyn Isolator` / `Arc<dyn
  Isolator>`, so a generic `waiting: impl FnOnce()` parameter would make the trait
  non-dyn-compatible. It would need `Box<dyn FnOnce + Send>`, and the isolator would then publish
  a worker frame. A sync `try_hold` mirrors `RunLocks::try_lock`, the try half of
  `lock_announcing` (`runtime.rs:470-490`). It is deterministic, and keeps announcing in the
  worker. Polling `hold` once (`now_or_never`) was rejected because tokio's coop budget can make an
  uncontended `lock_owned` return `Pending`, which would publish a spurious `Waiting`.
- **D2 test placement.** The worker-level R-49 tests go in `crates/htui/tests/chat.rs`, not in
  `run_worker.rs` (deviation 2).
- **D3** as planned. `go_offline` becomes `#[must_use]`, so a future call site cannot drop the swap
  silently.

---

## T1 - Deadline timer (ANA-27 T4)

**Files**: `crates/htui-orch/src/engine.rs`, `crates/htui-orch/src/gate.rs`, `crates/htui-orch/src/recover.rs`.

### gate.rs
`SettleInput` (`:203`): new field after `deadline_seconds`:
```rust
    /// MOD-37 M4 D1: the step deadline's timer cut the session (`Engine::drive_once`). Settles
    /// `DeadlineElapsed` whatever `now` says: the timer runs on tokio's clock and `now` on the
    /// engine's, and the two need not agree at the boundary (`deadline_elapsed` is a strict `>`).
    pub deadline_cut: bool,
```
`settle` (`:239`): the one rule changes, and stays in place after `CapBreached`:
```rust
    if input.deadline_cut || deadline_elapsed(input) {
        return Settle::Failed(StepFailure::DeadlineElapsed);
    }
```
Literals: `ok_input` (`:1113`) and the nine test literals (`:1245, 1256, 1269, 1280, 1291, 1304, 1321,
1340, 1353`) take `deadline_cut: false`. The ones built with `..ok_input(..)` need nothing.

### recover.rs
`resettle` (`:337`): `deadline_cut: false,` (recovery never has a live timer). Update the doc at
`:321` to say "`deadline_seconds: None`, `deadline_cut: false`".

### engine.rs - imports
```rust
use htui_agent::record::{
    Control, RELAY_GRACE, RELAY_POLL, Recorder, Relay, RunCap, Signal, control_channel, drive,
};
use tokio::sync::watch;
```
(`htui_agent::record` re-exports both, `record.rs:123`. The workspace tokio has `macros`.)

### engine.rs - new private items (near `SessionResult`, `:267`)
```rust
/// What [`Engine::drive_once`] answers (MOD-37 M4 D1): the session's result, and whether the step
/// deadline cut it. `cut` is `true` only with `result == Ok(done {cancelled})` after the
/// deadline's own cancel, which no run cancel overtook.
#[derive(Debug)]
struct Driven {
    result: SessionResult,
    cut: bool,
}
```
Free functions (after `drive_once`'s impl block or beside `refused_over_finish`):
```rust
/// MOD-37 M4 D1: [`drive`] under the step deadline. `drive` runs against a step-local control;
/// [`forward_or_cut`] copies the run's cancel into it and, after `left`, sends the deadline's own.
/// Answers `drive`'s result and whether the deadline's cancel is the one `drive` saw.
async fn drive_with_deadline<S, R>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: &Relay<'_, R>,
    control: &mut Control,
    left: std::time::Duration,
) -> (SessionResult, bool)
where
    S: htui_core::store::RecorderStore,
    R: htui_core::store::RelayStore,
{
    let (local, mut step_control) = control_channel();
    // A cancel that reached the run during `driver.start` is already the step's.
    if control.signal().is_cancel() {
        local.send_replace(control.signal());
    }
    let mut cut = false;
    let driven = tokio::select! {
        biased;
        driven = drive(session, recorder, Some(relay), &mut step_control) => driven,
        never = forward_or_cut(control, &local, left, &mut cut) => match never {},
    };
    (driven, cut)
}

/// D1: forwards the run's cancel into `local`, and after `left` sends `Cancel { grace:
/// RELAY_GRACE }` unless a cancel is there already; `cut` says the deadline's was the one sent.
/// Never ends: [`drive_with_deadline`] drops it when `drive` answers.
async fn forward_or_cut(
    control: &mut Control,
    local: &watch::Sender<Signal>,
    left: std::time::Duration,
    cut: &mut bool,
) -> core::convert::Infallible {
    let mut timer = std::pin::pin!(tokio::time::sleep(left));
    // A finished `Sleep` polls `Ready` again, so the arm is disarmed after it fires (no spin).
    let mut armed = true;
    let cancel_once = |to: Signal| {
        local.send_if_modified(|seen| {
            if seen.is_cancel() {
                false
            } else {
                *seen = to;
                true
            }
        })
    };
    loop {
        tokio::select! {
            // The run first: a run cancel and the deadline in one instant end as the run's.
            biased;
            () = control.changed() => {
                let signal = control.signal();
                if signal.is_cancel() {
                    cancel_once(signal);
                }
            }
            () = &mut timer, if armed => {
                armed = false;
                *cut = cancel_once(Signal::Cancel { grace: RELAY_GRACE });
            }
        }
    }
}
```
`AgentSession` is `htui_agent::driver::AgentSession`. Name it with its full path or add it to the
`driver` import. If borrowck rejects reading `cut` after the `select!` (the forward future must be
dropped first; tokio's macro drops its futures with its block, so it should accept), use
`std::sync::atomic::AtomicBool`. Do not use `Cell`, because it is not `Sync`, so the walk future
would stop being `Send` (`a_dispatch_future_is_send`).

### engine.rs - `drive_once` (`:5870`)
New parameter `deadline: Option<std::time::Duration>`, between `extra_dirs` and `recorder`. The
return type becomes `Result<Driven, EngineError>`. Append to the `#[allow(clippy::too_many_arguments)]`
reason: "…and the step deadline's remainder (MOD-37 M4 D1)". Replace the `let driven = …` line and
the final `match`:
```rust
        // Boxed, as `pump` boxes it (`record.rs`): `drive`'s state machine inline would grow
        // every walk future past the debug test stack (`every_case_name_dispatches`). The
        // deadline's composite is boxed for the same reason (MOD-37 M4 D1).
        let (driven, cut) = match deadline {
            None => (
                Box::pin(drive(&mut *session, recorder, Some(&relay), &mut control)).await,
                false,
            ),
            Some(left) => {
                Box::pin(drive_with_deadline(&mut *session, recorder, &relay, &mut control, left))
                    .await
            }
        };
        // MOD-42 D4, D10: two answers leave the session result before settle can read them.
        match driven {
            // MOD-37 M4 D1: the deadline's own cancel, no run cancel behind it: a cut session,
            // which settles as `DeadlineElapsed` (the `enforce_breach` shape, `record.rs:1910`).
            Err(DriverError::Cancelled) if cut && !control.signal().is_cancel() => {
                tracing::warn!(run = %run.id, step = %step.id, "the step deadline cut the session");
                Ok(Driven {
                    result: Ok(DoneEvent { stop_reason: StopReason::Cancelled }),
                    cut: true,
                })
            }
            Err(DriverError::Cancelled) => Err(EngineError::Cancelled { run: run.id }),
            Err(fenced @ DriverError::Store(StoreError::Fenced { .. })) => {
                Err(EngineError::Driver(fenced))
            }
            result => Ok(Driven { result, cut: false }),
        }
```
(Keep the existing fence comment on its arm.) This gives the four required behaviours:
(a) A run cancel before or after the timer: `control.signal().is_cancel()` gives
`EngineError::Cancelled`.
(b) The deadline: `Ok(Driven { Ok(done{cancelled}), cut: true })`.
(c) `None` is today's code, unchanged.
(d) The composite sits behind one `Box`, so `drive_once`'s state machine holds two pointers.

### engine.rs - `session` (`:5769`)
New parameter `started_at: DateTime<Utc>` after `extra_dirs`; update the `#[allow]` reason. It
returns `Result<(Driven, Option<CapBreach>), EngineError>`. After `open_recorder`:
```rust
        // MOD-37 M4 D1: what is left of the step deadline as the session starts.
        let deadline = Self::remaining(phase, started_at, self.now());
```
Pass `deadline` to `drive_once`. The tail becomes `Ok((driven, summary.cap_breach))`.

### engine.rs - `walk_live_step` (`:3417`)
```rust
        let (Driven { result, cut: deadline_cut }, cap_breach) = self
            .session(run, step, phase, persona, &prompt, prepared.cwd, prepared.extra_dirs, started_at)
            .await?;
```
`VerifyStage { …, deadline_cut }` and `SettleInput { …, deadline_cut, … }`.

### engine.rs - `candidate_live` (`:4074`)
Right before `self.drive_once(..)` (after `open_recorder`):
`let deadline = Self::remaining(phase, started_at, self.now());`. Here `started_at` is the
candidate's own, taken after `prepare` (D48). Pass it, then:
`Ok(driven) => driven` → `let Driven { result, cut: deadline_cut } = driven;`. Thread
`deadline_cut` into `VerifyStage` and `SettleInput` as above.

### engine.rs - `judge_calls` (`:5031`)
Pass `None`. Then `let done = match self.drive_once(.., None, recorder).await?.result { … }`;
judge calls stay untimed (maintainer decision).

### engine.rs - `VerifyStage` (`:6435`) / `verify` (`:3638`)
```rust
    /// MOD-37 M4 D1: the step deadline cut the session; the verify gets `Some(ZERO)` at once.
    deadline_cut: bool,
```
In `verify`: destructure it, and
`remaining: if deadline_cut { Some(std::time::Duration::ZERO) } else { Self::remaining(phase, started_at, self.now()) },`.

### What does not change
`gate::apply`, `Landing`, `fail_candidate`, `run_candidate`'s error arm (a cut is `Ok`, so it
never reaches the cancel/fence arm), `relay.rs`, `record.rs`, `Recorder`.

### T1 regression tests
All of these go in `engine.rs`'s `mod tests::relay` (`:15640`). That module is on paused time,
with `walked` bounded at `CLIENT_LIMIT * 2` = 60 s, and has `parks`, `step_at`, `note_count`,
`assert_cancel_answered_once` and `start_feat_3`. Planting the deadline is the app rung, as in
`a_step_that_outlives_its_deadline_settles_failed` (`:8093`):
`harness.orch.store.set_app_setting("step_deadline_seconds", serde_json::json!(1))`.

| # | Name | Setup | Assertions | Red on old code? |
|---|---|---|---|---|
| 1 | `a_hung_session_is_cut_at_the_step_deadline_and_settles_deadline_elapsed` | `#[tokio::test(start_paused = true)]`; `harness.free_feat_3()`; deadline 1 s; `harness.orch.script("prd", 1, parks("the prd"))` (a parked stage-3 request nobody answers is a session that never sends `done`); `let t0 = tokio::time::Instant::now()`; `walked(harness.dispatch(start_feat_3()))` | `Ok(Started { run, rest })` with `rest.run == AwaitingApproval` (`prd` is `always`); the `prd` step is `AwaitingApproval`; `store.notes(FEAT_3)` has a body containing `"deadline elapsed"`; `assert_cancel_answered_once(&harness.orch, prd.id)` (the graceful cancel reached the session: I-7 row and relay row `cancelled`); `t0.elapsed() >= 1 s` and `< CLIENT_LIMIT` | **Red**: the parked session polls for ever, so `walked` panics "the walk ends within its bound" at 60 s of paused time. It also pins the cut flag: the `TestClock` never moves, so a timer without `deadline_cut` settles `Ok` (FakeOrchestrator's `after_done` writes the output) and the note assertion fails |
| 2 | `a_hung_fanout_candidate_is_cut_at_its_deadline_and_fails_alone` | the setup of `a_cancel_reaches_a_parked_fanout_candidate_and_settles_nothing` (`:16129`): `prd` `fan_out = 2`, `Gate::Never`; candidate 0 `parks("candidate 0")`, candidate 1 `done_with_output("candidate 1")`; deadline 1 s | walk `Ok(Started{..})`, not `Cancelled`; candidate 0 `Failed` with a note containing `"deadline elapsed"`; candidate 1 `Done`; a step at position 1 exists (the group resolved and the walk moved on); `assert_cancel_answered_once(.., first.id)` | **Red**: same hang, so `walked` panics at its bound |
| 3 | `a_run_cancel_before_the_deadline_still_ends_cancelled` | as `a_cancel_reaches_a_parked_single_step_and_settles_nothing` (`:16070`), plus deadline 5 s; `tokio::join!(walked(..), cancelling_client(..))` | `Err(EngineError::Cancelled { run })`; `prd` `Running`, `finished_at == None`; note count unchanged; `assert_cancel_answered_once` | **Pin, green on old code.** It keeps (a) under the new arm: an implementation that mapped any `Err(Cancelled)` with `Some(deadline)` to a cut fails it |
| 4 | `a_session_that_ends_inside_its_deadline_is_not_cut` | deadline 1 s; FEAT-3 freed, default scripts; walk; then `tokio::time::sleep(5 s)` | `rest.run == AwaitingApproval`; no note contains `"deadline elapsed"`; after the sleep, the run row and the note count are unchanged (the dropped timer did nothing) | **Pin, green on old code**, as the plan lists it |
| 5 | `gate.rs`: `a_cut_session_settles_deadline_elapsed_whatever_the_clock_says` | `ok_input(&Ok(done{cancelled}), Some(&doc))` with `deadline_cut: true`, `now == started_at` | `DeadlineElapsed`. With `cap_breach: Some(..)` too: `CapBreached`. With `driver: Err(Closed)` + cut: `Driver(..)`. With `deadline_cut: false`, same input: `Ok { note: None }` | **Red by compile** (no field); the three variants pin the rank |

The existing MOD-42 relay tests (`:16070, :16129, :16244, :16297, :16473`) and every walk test now
run the `Some(7200 s)` path. `graph.rs:29` defaults `deadline_seconds` to two hours, so they are
the regression net for (a) and (c).

**Gate T1**: `cargo nextest run -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/t1.log; grep -c SIGABRT /tmp/t1.log` (must be 0),
`cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`, `cargo clippy -p htui-orch --lib -- -D warnings`.

---

## T2 - R-49: a promoted chat holds the shared guard

**Files**: `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/lib.rs` (re-export, not in the plan's list),
`crates/htui-orch/src/isolate/real.rs`, `crates/htui-orch/src/fake.rs`, `crates/htui-orch/tests/gix_isolator.rs`,
`crates/htui-worker/src/address.rs`, `crates/htui-worker/src/runtime.rs`, `crates/htui/tests/chat.rs`.

### isolate.rs
```rust
/// MOD-37 M4 D2 (R-49): the `(box, repo)` guards a promoted chat holds over its step's
/// `shared_serialized` checkouts, from before the chat is bound until its task ends. Dropping it
/// frees them. An isolator never records it in its per-step table, so `capture`, `release` and
/// `cleanup` cannot free a chat's guard.
#[derive(Debug, Default)]
#[must_use = "a hold frees its guards the moment it is dropped"]
pub struct Hold(Vec<tokio::sync::OwnedMutexGuard<()>>);

impl Hold {
    /// Over `guards`, taken in `RepoId` order.
    pub(crate) const fn new(guards: Vec<tokio::sync::OwnedMutexGuard<()>>) -> Self {
        Self(guards)
    }

    /// Whether it holds nothing: no row it was asked for was `shared_serialized`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
```
Trait `Isolator`, after `release`:
```rust
    /// MOD-37 M4 D2 (R-49): the `(box, repo)` guard of every `shared_serialized` row of `trees`,
    /// in `RepoId` order as `prepare` takes them, waiting for each. Rows of any other mode take
    /// nothing, so a step with none answers an empty [`Hold`] at once.
    fn hold<'a>(&'a self, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Hold>;

    /// [`hold`](Isolator::hold) without waiting: `None` when any of its guards is held now, and
    /// then nothing is kept.
    fn try_hold(&self, trees: &[RunStepTree]) -> Option<Hold>;
```
(`Hold` is `Send + Sync + Debug`. tokio 1.53.1 `OwnedMutexGuard<()>`, plan-verified.)
**lib.rs**: add `Hold` to `pub use isolate::{…}`.

### real.rs (`GixIsolator`)
Private helpers in the inherent `impl`:
```rust
    /// D43's lock for `repo` on this box, minted on first use; the `std` lock is released before
    /// anyone awaits the `tokio` one (as in `acquire`).
    fn guard_of(&self, repo: RepoId) -> Arc<tokio::sync::Mutex<()>> { … }
```
`acquire` may switch to `guard_of` (pure extraction: same order, same `held` push). Free fn:
```rust
/// MOD-37 M4 D2: the repos of `trees`' `shared_serialized` rows, sorted and deduplicated (A-1).
fn serialized_repos(trees: &[RunStepTree]) -> Vec<RepoId>
```
Trait impl (`:1427`):
```rust
    fn hold<'a>(&'a self, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Hold> {
        Box::pin(async move {
            let mut guards = Vec::new();
            for repo in serialized_repos(trees) {
                guards.push(self.guard_of(repo).lock_owned().await);
            }
            Ok(Hold::new(guards))
        })
    }

    fn try_hold(&self, trees: &[RunStepTree]) -> Option<Hold> {
        let mut guards = Vec::new();
        for repo in serialized_repos(trees) {
            // A busy one drops every guard taken so far with `guards`.
            guards.push(self.guard_of(repo).try_lock_owned().ok()?);
        }
        Some(Hold::new(guards))
    }
```
`local` takes nothing, as for walks today.

### fake.rs (`FakeIsolator`, its one `serial` mutex, `:116`)
```rust
    fn hold<'a>(&'a self, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Hold> {
        Box::pin(async move {
            if !trees.iter().any(|tree| tree.mode == Isolation::SharedSerialized) {
                return Ok(Hold::default());
            }
            Ok(Hold::new(vec![Arc::clone(&self.serial).lock_owned().await]))
        })
    }

    fn try_hold(&self, trees: &[RunStepTree]) -> Option<Hold> {
        if !trees.iter().any(|tree| tree.mode == Isolation::SharedSerialized) {
            return Some(Hold::default());
        }
        Arc::clone(&self.serial).try_lock_owned().ok().map(|guard| Hold::new(vec![guard]))
    }
```
### tests/gix_isolator.rs (`StallAfterReconcile`, `:2004`)
Delegate both, `self.inner.hold(trees)` and `self.inner.try_hold(trees)`, and import `Hold`.

### address.rs (`ChatEnd`, `:122`)
Drop `Clone` from the derive (`#[derive(Debug)]`) and add the field:
```rust
    /// MOD-37 M4 D2 (R-49): the step's `shared_serialized` guards, held for the chat's whole life
    /// and dropped when [`after`](Self::after)'s task returns, before its `Changed` frame.
    pub(crate) hold: htui_orch::Hold,
```
`after` destructures so the hold drops before the frame:
```rust
        let Self { publisher, tag, hold } = self;
        task.await;
        drop(hold);
        if let Some(item) = tag.item.get() { publisher.publish(&RunFrame { … }); }
```
A panicking task unwinds the spawned `after` future, which drops `hold`. The `Served::Reply` and
`Served::Deferred` paths in `store_worker::on_run_served` (`:1955`) and `testkit::on_run_served`
(`:190`) drop `ended`, and with it the hold. No edit there.

### runtime.rs
Constant beside `PREEMPTED` (`:1021`):
```rust
/// MOD-37 M4 D2 (R-49): the head of a promotion's second answer when its chat was not started.
pub const CHAT_NOT_STARTED: &str = "the step was promoted, but its chat was not started";
```
New fn beside `on_run_unless_claimed`:
```rust
/// MOD-37 M4 D2 (R-49): the guards of `step`'s `shared_serialized` trees for its promoted chat,
/// taken under the run's lock. A free hold is taken silently; a busy one publishes `Waiting` once
/// and waits, cancellable as `Shared::lock_announcing` is, then publishes `Changed` so the pane
/// stops showing the wait. `Err` is the sentence the requester is answered with.
async fn chat_hold<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: &TaskCtx<H, P>,
    kit: &Kit<H>,
    walk: &WalkToken,
    run: RunId,
    step: StepId,
) -> Result<Hold, String> {
    let stopped = || format!("{CHAT_NOT_STARTED}: the wait for its checkout's guard was stopped");
    let mut signal = walk.signal.clone();
    if walk.token.is_cancelled() || signal.borrow().is_cancel() {
        return Err(stopped());
    }
    let trees = htui_core::store::WorkerStore::step_trees(&kit.writer, step)
        .await
        .map_err(|err| format!("{CHAT_NOT_STARTED}: its trees could not be read: {err}"))?;
    if let Some(hold) = kit.isolator.try_hold(&trees) {
        return Ok(hold);
    }
    ctx.publish(Some(run), FrameKind::Waiting);
    let held = tokio::select! {
        biased;
        () = walk.token.cancelled() => Err(stopped()),
        () = cancel_signalled(&mut signal) => Err(stopped()),
        hold = kit.isolator.hold(&trees) => hold.map_err(|err| format!("{CHAT_NOT_STARTED}: {err}")),
    };
    // R-51's pane shows "waiting" until an invalidating frame; nothing else publishes one
    // before the chat ends.
    ctx.publish(Some(run), FrameKind::Changed);
    held
}
```
The three exact failure sentences:
- `the step was promoted, but its chat was not started: the wait for its checkout's guard was stopped`
- `the step was promoted, but its chat was not started: its trees could not be read: <StoreError>`
- `the step was promoted, but its chat was not started: <IsolateError>` (unreachable for both isolators today)

`on_run_unless_claimed`, Promoted arm, `Ok(project)` (`:2405-2420`):
```rust
                    Ok(project) => match chat_hold(ctx, &kit, &walk, run, step).await {
                        Ok(hold) => {
                            let _ = ctx.shared.events.send(RunServed::Attach {
                                addr,
                                promoted: Box::new(Promoted { run, step, project, opening: *opening }),
                                ended: ChatEnd {
                                    publisher: Arc::new(ctx.shared.publisher.clone()),
                                    tag: Arc::clone(&ctx.tag),
                                    hold,
                                },
                            });
                        }
                        Err(message) => ctx.answer(RunReply::Failed { request: ctx.name, message }),
                    },
```
This keeps the order: `Promoted` answer → `Rested` frame → project read → (`step_trees` read →
hold) → `Attach`. The hold is taken only when a chat will be bound (`ctx.addr` is `Some` and the
project read succeeded). `guard` (the run lock) is still held, since it is dropped after the
`match`. `walk` is the task's own child token (`:2338`), already in scope. Imports: `Hold`,
`StepId`.

### T2 regression tests
| # | Name / file | Setup | Assertions | Red on old code? |
|---|---|---|---|---|
| 1 | `real.rs` `a_hold_takes_the_shared_serialized_guard_until_it_drops` | the `a_refused_shared_serialized_prepare_releases_what_it_took` (`:2055`) shape: one repo `core`; `prepare(run, step, &[core], SharedSerialized, None)`, rows from `prepared.trees`, `capture(step, &rows)` (the walk's guard goes) | `try_hold(&rows)` is `Some`, `!is_empty()`; a second `try_hold` is `None`; `timeout(200 ms, hold(&rows))` is `Err`; `release(run)` and `cleanup(run, &rows)` run; `timeout(200 ms, prepare(RunId::new(), StepId::new(), &[core], SharedSerialized, None))` is still `Err`; `drop(hold)`; `timeout(5 s, prepare(..))` `Ok(Ok(_))` | **Red by compile** (no verb). It also fails a hold implemented through `acquire`/`held`, which `release`/`cleanup` would free |
| 2 | `real.rs` `a_hold_over_rows_of_any_other_mode_takes_nothing` | rows for `core` with modes `Worktree`, `Copy`, `Local` (paths irrelevant); a live `shared_serialized` `prepare` of another step holds the guard | `try_hold(&rows)` is `Some` and `is_empty()`; `timeout(1 s, hold(&rows))` answers an empty hold | **Red by compile** |
| 3 | `fake.rs` `the_fake_hold_takes_its_serial_lock` (beside `the_fake_release_frees_its_serial_lock` `:2337`) | a `SharedSerialized` row | `try_hold` `Some`; `prepare(.., SharedSerialized, ..)` times out while it lives, then completes after the drop; a `Worktree` row gives an empty hold | **Red by compile** |
| 4 | `tests/chat.rs` `a_promoted_shared_serialized_chat_waits_for_its_guard_and_holds_it_to_its_end` | the raw-loop shape of `promoting_a_running_step_preempts_its_walk` (`:1773`). `graph_store()`, then project settings `default_isolation: "shared_serialized"` (read `project_settings(ids::PROJECT_HTUI)`, set the key, `set_project_settings`). A new `run_runtime_over(walks, isolator: Arc<FakeIsolator>)` that `run_runtime` calls with a fresh fake; the test keeps its `Arc`. `chat_runtime(Script::one_turn(vec![chunk("Taking over."), done()]))`. seq 1 `RunStream { item: HTUI_ANA_2 }`; seq 2 `start_run()` → `Started { rest: AwaitingApproval }`; `trees = store.step_trees(step.id)` (assert non-empty, all `SharedSerialized`); `let other = isolator.try_hold(&trees).expect(..)` stands in for another holder (deviation 1); seq 3 `PromoteStep { chat_open: false }` from `Origin::Tab(ChatTab::ID)` | at seq 3, `Orch(Promoted{step})`; a `RunStream` frame `Waiting` with `run == Some(run)` arrives; after a 300 ms sleep there is still no `ChatAccepted` at seq 3; `drop(other)` → `ChatAccepted { step_id == step.id }` at seq 3 and a `Changed` frame for the run; `timeout(300 ms, isolator.prepare(RunId::new(), StepId::new(), &[], SharedSerialized, None))` is `Err` (a second walk's `prepare` waits on the chat); seq 4 `ChatCancel { step_id }` → `within(20 s, isolator.prepare(..))` is `Ok`, then `isolator.release(that_run)` | **Red**: old code binds at once, so no `Waiting` frame arrives (the wait times out) and `ChatAccepted` comes while `other` is held |
| 5 | `tests/chat.rs` `a_cancel_while_a_promotion_waits_for_its_guard_binds_no_chat` | as 4 through the `Waiting` frame; then seq 4 `CancelRun { run }` from `Origin::App` | seq 3's second reply is `Failed { request: "promote_step", message == "the step was promoted, but its chat was not started: the wait for its checkout's guard was stopped" }`; no `ChatAccepted` at seq 3 within 300 ms; seq 4 answers `Orch(Done(Cancelled{..}))`; `drop(other)`, then `isolator.try_hold(&trees)` is `Some` (the failed promotion kept nothing) | **Red**: old code binds the chat (`ChatAccepted`), and the `Waiting` assertion fails first |

Why the cancel reaches the wait: `cancel_run` (`:2460`) finds the run live (the promotion's
`WalkToken`) and calls `preempt_gracefully`, which sends `Cancel` on the parent. `chat_hold`'s
`cancel_signalled` arm ends the wait, the promotion task drops its token and the run lock, and the
cancel proceeds.

**Gate T2**: `cargo nextest run -p htui-orch -p htui-worker -p htui --all-features --no-fail-fast` (grep SIGABRT);
confirm `htui` with `cargo test -p htui --all-features -- --test-threads=1` (the keyring fake is
process-wide). Check that `tests/chat.rs` ran a non-zero count. Clippy for the three crates
`--all-targets --all-features -D warnings`.

---

## T3 - R-46: the offline swap preempts live walks

**Files**: `crates/htui-worker/src/runtime.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/run_worker.rs`.

### runtime.rs - `RunRuntime` (beside `forget_server`, `:1351`)
```rust
    /// MOD-37 M4 D3 (R-46): every live walk of this process preempted at once, without
    /// forgetting the server: the isolator, the verifier and the claim queue stay (unlike
    /// [`Self::forget_server`]). The TUI's store loop calls it when its backend drops
    /// `Online → Offline`. Each walk ends through `walked`'s `None` → `abandoned` → a release
    /// that fails offline → the dead-walk set, and the next `Online` sweep adopts the run (D175,
    /// D190). A promotion waiting for its guard ends too (R-49's failure sentence).
    pub fn preempt_walks(&self) {
        self.shared.walks.preempt_all();
    }
```
### store_worker.rs
`go_offline` (`:2683`):
```rust
#[must_use = "an Online → Offline swap must preempt the live walks (R-46)"]
fn go_offline(…) -> bool {
    if let Some(previous) = refresher.take() { previous.abort(); }
    *health = None;
    if !backend.went_offline() {
        return false;
    }
    tracing::warn!(%why, "store unreachable; falling back to the mirror");
    true
}
```
The doc gains: "Whether the backend went `Online → Offline` now (`Backend::went_offline`), which the
loop answers by preempting every live walk (MOD-37 M4 D3)."
Loop call sites:
- `:2427-2429`: `if matches!(err, StoreError::Unreachable(_)) && go_offline(&mut backend, &mut refresher, &mut health, &err) { runs.preempt_walks(); }`
- `:2528`: `if go_offline(&mut backend, &mut refresher, &mut health, &err) { runs.preempt_walks(); }`

Test call sites (both backends are not `Online`):
- `:3881`: `assert!(!go_offline(..), "a memory backend never swaps");`
- `:4684`: `assert!(!go_offline(..), "{was} is not Online: no swap");`

### T3 regression tests
| # | Name / file | Setup | Assertions | Red on old code? |
|---|---|---|---|---|
| 1 | `run_worker.rs` `an_offline_swap_preempts_the_walk_and_the_sweep_adopts_it` | `#[tokio::test(start_paused = true)]`, built from `a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after` (`:2455`) with the direct-`serve` shape of `a_server_switch_preempts_every_walk_and_forgets_the_queue` (`~:1982`): `Play::Stall`, `runtime.serve(start_run(ANA_2))` → `Deferred`, `stall.reached`; `fixture.store.set_fault(MemFault::ReleaseLease, true)` (the server is gone); `let t0 = tokio::time::Instant::now(); runtime.preempt_walks();` | the requester gets `Failed { message == PREEMPTED }` within `PATIENCE`; `t0.elapsed() < 1 s` (at once, where the old fence is `ttl - refresh` = 80 s); `stall.dropped`; `testing::probe(&runtime).is_dead_walk(run)`; then fault off, `swept(&mut runtime, &fixture)`, `rests_at(run, AwaitingApproval)`, `step_at(run, 0).attempt == 2`, `!is_dead_walk(run)` | **Red by compile** (no `preempt_walks`) |
| 2 | `run_worker.rs` `an_offline_swap_keeps_the_server` | no walk; `testing::probe(&runtime).queue(Utc::now(), RunId::new())`; `runtime.preempt_walks()` | `queued()` still holds the entry; `isolator_builds()` unchanged | **Red by compile**. It pins the difference from `forget_server` |
| 3 | `store_worker.rs` `go_offline_reports_a_swap_only_from_online` | `Backend::Online { pg: PgStore::lazy("postgres://nobody:nothing@127.0.0.1:1/none", ..), cache }` (the `an_unreachable_read_drops_an_online_backend_onto_the_mirror` shape, `:3600`) | the first `go_offline` is `true` with a label starting `offline · `; a second call on the same backend is `false` | **Red by compile** (returns `()`) |
| 4 | `store_worker.rs` `go_offline_drops_the_health_watch_on_a_backend_that_is_not_online` (extended) | unchanged | `assert!(!go_offline(..))` for `demo()` and `Offline` | **Red by compile** |

The two loop call sites have no end-to-end test. A `Memory` backend never swaps, and an `Online`
lazy backend cannot host a walk, because its writer is unreachable. The `#[must_use]` and test 3
pin the bool. The wiring is two `if`s, reviewed by reading.

**Gate T3**: `cargo nextest run -p htui-worker -p htui --all-features --no-fail-fast`; then `htui` with `--test-threads=1`; clippy.

---

## T4 - Close-out
As in the plan, plus: the HANDOFF R-49 line names the residuals (hazards 10 and 11), and the
deviations below go into the MOD-37 write-up.

## Commits (each green, except the two marked red, which compile and fail as described)
1. `test(mod-37): step deadline regression tests (red: a hung session hangs the walk)`. These are
   T1 tests 1-4. They compile on the old code; 1 and 2 fail at the 60 s walk bound, and 3 and 4 are
   pins that pass. Say so in the body.
2. `feat(mod-37): cut a hung session at the step deadline (ANA-27 T4)`. Contains gate.rs (field,
   rule, test 5), recover.rs and engine.rs (`Driven`, `drive_with_deadline`, `forward_or_cut`,
   `drive_once`, `session`, `candidate_live`, `judge_calls`, `VerifyStage`).
3. `feat(mod-37): Isolator::hold and try_hold (R-49)`. Contains isolate.rs, lib.rs, real.rs (+2
   tests), fake.rs (+1 test) and gix_isolator.rs.
4. `test(mod-37): a promoted shared_serialized chat waits for its guard (red)`. Contains the
   `tests/chat.rs` tests 4 and 5 and `run_runtime_over`. They compile after commit 3 and fail on
   the missing `Waiting`.
5. `fix(mod-37): a promoted chat holds the shared_serialized guard to its end (R-49)`. Contains
   address.rs and runtime.rs (`CHAT_NOT_STARTED`, `chat_hold`, the Promoted arm).
6. `fix(mod-37): an offline swap preempts every live walk (R-46)`. Contains runtime.rs
   `preempt_walks`, store_worker.rs and the run_worker.rs tests 1 and 2. Its red is compile-only;
   say so in the body.
7. `docs(mod-37): M4 close-out`.

## Validation (the plan's)
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features --no-fail-fast 2>&1 | tee /tmp/m4.log; grep -c SIGABRT /tmp/m4.log   # 0
cargo test -p htui --all-features -- --test-threads=1    # keyring fake is process-wide
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
No SQL, no `.sqlx`.

## Hazards
1. **Stack.** `every_case_name_dispatches` (`conformance.rs:7246`) is near the 2 MiB debug stack.
   The composite sits behind `Box::pin`, so do not inline `drive_with_deadline`. `chat_hold`'s
   future sits in the spawned `on_run` task. If htui-worker tests SIGABRT, box it there too.
2. **Every step now runs the timer.** `graph.rs:29` defaults `deadline_seconds` to 7200 s, so the
   `None` path is in practice judge calls only. A paused-time test that idles a live session for
   more than 7200 virtual seconds would now be cut. A scan found none (the only 7200 s sleep is
   `concepts_worker.rs:957`, no walk). The full suite is the check.
3. **Two clocks.** `remaining` is measured on the engine `Clock` and the timer on tokio. `deadline_cut`
   makes settle independent of their agreement, and `VerifyStage.deadline_cut` does the same for
   verify. In production, wall-clock drift only moves the cut by milliseconds.
4. **Cancel against deadline.** The forwarder polls the run first (`biased`) and forwards cancels
   only. The deadline's cancel is sent only if no cancel is there. `drive_once` re-reads the run
   control after `drive`, so a run cancel that lands during the cut's drain still ends as
   `Cancelled`. `armed` stops a finished `Sleep` from spinning the loop.
5. **B-16 overlap.** A `stale`/`cancelled` relay row that ends `drive` after the timer fired settles
   as a cut (`DeadlineElapsed`) instead of `EngineError::Cancelled`. This needs both in one turn,
   and a superseded session's writes are fenced anyway. Accepted.
6. **Borrow of `cut`.** If `select!`'s futures outlive the read, switch to `AtomicBool`. Never use
   `Cell`, which would make the walk future non-`Send`.
7. **Lock order (R-49).** The order is run lock, then `(box, repo)` guards in `RepoId` order. That
   is the walk's order, and a walk never takes another run's lock. A hold never also takes a run
   lock.
8. **The wait holds the run lock.** Commands on the run queue behind the promotion with R-51's
   `Waiting`. A cancel, a preempt, a server switch or an offline swap end it.
9. **The pane's "waiting" line.** `chat_hold` publishes `Changed` after any announced wait.
   Without it, the Runs pane would say "waiting for the walk" for the chat's whole life, because
   `ChatEnd::after`'s `Changed` is the next invalidating frame. The text is approximate for a chat
   holder (accepted, LOW).
10. **The hold is per isolator instance.** A `GixIsolator` rebuild (repo map moved,
    `forget_server`) or a separate `htui worker` process does not see a chat's hold. Across
    processes, `claim_run`'s overlap admission covers it. Accepted, LOW; noted in HANDOFF.
11. **D185 race.** Two promotions of one `shared_serialized` step served before either binds: the
    second waits, holding the run lock, until the first chat ends, then binds a second chat. All
    run-moving commands are refused by `chat_free` meanwhile anyway. Accepted, LOW.
12. **`ChatEnd` loses `Clone`.** Nothing clones it (plan-verified). `RunServed` derives only
    `Debug`.
13. **`#[must_use]` on `go_offline`.** Its two test call sites must use the bool. Clippy
    `-D warnings` catches a miss.
14. **R-46 blips.** A short blip now ends sessions; the sweep walks a new attempt. This is the
    maintainer's decision. `preempt_all` also cancels claim, sweep and command tasks of every run
    (their requesters hear `PREEMPTED`), as `SetDsn` does.
15. **Test harness.** `crates/htui/tests/*.rs` run 0 tests without `--features testkit`; use
    `--all-features` and check the counts.

## Deviations from the plan (with evidence)
1. **"Another run holds the guard" cannot be staged with two runs.** `MemStore::claim_run`
   (`mem.rs:4420-4466`) refuses a claim that overlaps any `running` or `awaiting_approval` run of
   the box. Rule I (`overlap.rs` `overlaps`) fires whenever either run is not isolated in a shared
   repo, and `shared_serialized` is not isolated. So while a promoted step's run is non-terminal,
   no other run on that repo is admitted on this box. The realistic contenders are:
   - a same-run command queued in the window between `Promoted` and the bind, whose `chat_free`
     (`views.rs:457`) saw no live chat;
   - commands outside `moves_the_run` (`views.rs:468`);
   - a rebuilt isolator.

   The fix is unchanged. Test 4 stands in for the holder with a direct `try_hold` on the shared
   `FakeIsolator`.
2. **The worker tests go in `crates/htui/tests/chat.rs`, not `run_worker.rs`.** `run_worker.rs`'s
   `Worker::spawn` builds `AgentRuntime::new(DriverFactory::new())` (`run_worker.rs:~700`), so a
   promotion's bind fails at `driver_for` and `ended` (the hold) drops at once. "Holds it until the
   chat ends" is untestable there. `tests/chat.rs` already drives a bound promoted chat through the
   raw store loop (`promoting_a_running_step_preempts_its_walk`, `:1773`).
3. **"verify gets `Some(ZERO)`" needed the cut flag too.** `verify` computes `remaining` from the
   engine clock (`engine.rs` `verify`, `:3638`). With `TestClock` (or any clock that disagrees with tokio), a cut
   session would still spawn its verify. `VerifyStage.deadline_cut` makes the plan's statement true.
4. **The `Waiting` wait needs a closing `Changed` frame.** See hazard 9. The plan omitted it.
5. **`lib.rs`** gains the `Hold` re-export, which is missing from the plan's file table.
6. **T3's regressions are red by compile only.** The loop wiring cannot be driven end to end (see
   T3). The plan's "each risk has a regression test that is red on the old code" holds for T1
   (behavioural) and T2 (behavioural, tests 4 and 5). It holds for T3 only at compile level.
7. **Line references drifted** (see the header).
