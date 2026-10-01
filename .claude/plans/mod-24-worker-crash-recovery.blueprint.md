# Blueprint: MOD-24 - crash recovery of runs under the headless worker

Implements `.claude/plans/mod-24-worker-crash-recovery.plan.md` (CONFIRMED 2026-10-01, OQ-1..OQ-4
= (a); D1-D4 are not reopened). Base `hr/MOD-24` @ `5f25e97`. Every file:line below was read on
2026-10-01 (Gortex symbol reads plus `sed -n` ranges). Where the plan is wrong or incomplete, it is
under "Amendments to the plan" (end), with evidence, and the blueprint follows the amendment.

## Design decisions (blueprint-level, within the plan)

- **B1 - `kill_point` is always compiled; only its armed half is gated.** `pub mod kill_point;` is
  unconditional, its two types and four constants exist in every build, and `reached` is an empty
  `#[inline]` fn without `test-support`. The parser, the env read and the park live in a private
  `armed` submodule under `#[cfg(feature = "test-support")]`. So the three call sites compile in
  the production build with no `cfg` of their own.
- **B2 - the env is read once, never written.** A `OnceLock` holds the parsed spec. No test sets an
  env var in process (`unsafe_code = "forbid"`, and `set_var` is `unsafe` in edition 2024). The
  unit tests exercise the pure parser and the marker writer. The env-driven path is exercised by
  T4's children only.
- **B3 - a kill point that never gets killed ends the process itself.** The park is
  `park_timeout` up to `PARK_LIMIT` (300 s), then `std::process::exit(PARK_EXPIRED_EXIT = 86)`. A
  malformed spec exits `MISCONFIGURED_EXIT = 87` at the first call. An orphan cannot outlive the
  suite by more than five minutes, and a misconfigured case fails loudly with a distinct code
  instead of waiting out `PATIENCE`.
- **B4 - D3 is a new private fn `cancels_first`, called first in `adopt`.** It applies only rows
  whose run is `running` and not walked by this process. Parked and terminal runs are never
  adopted by the sweep, so no recovery races them, and they stay the poll's. It applies them
  through `cancel_run(.., Some(row_id))`, then **waits for the B-5 guard to clear** (a `Notify`
  fired by `Applying::drop`). A row the poll took first is therefore awaited, never applied twice,
  and never overtaken by `sweep_fenced`.
- **B5 - D3 respects D212's live-chat refusal.** `poll_commands_with` stores the `LiveChats` it
  was handed in `Shared` (`last_live`). `cancels_first` passes that set to `cancel_run`. The worker
  always passes `LiveChats::default()`. The TUI's sweep can no longer cancel under a chat the
  TUI's poll would have refused.
- **B6 - D4 rides on `Answer`, not on `answering`'s signature.** A new optional field
  `closes: Option<(Writer, ChatRunSpec)>` is set by a builder `Answer::closing`. `Answer::send`
  becomes `async` and closes before it sends. The other seven `answering` call sites and their
  `Answer`s are untouched.
- **B7 - T4 copies helpers from `worker_pg.rs`.** No `tests/common/` exists anywhere under
  `crates/`, and `worker_pg.rs` itself is "a minimal copy of `runs_pg.rs`'s stack" (its `//!`,
  blueprint F-29). The repo's convention is a self-contained file.
- **B8 - every child session commits.** Without a commit, a worktree capture records
  `after_hash = NULL`, and K4 would be classified `Reset` instead of `Finished` (amendment A-3).
  So each child session writes and commits `step-<step_id>.txt` into its tree before `Done`. That
  also makes reconcile's merges real, which gives K1's "stray file not landed" a positive control.

## Files to create

| File | Purpose | Task |
|---|---|---|
| `crates/htui-orch/src/kill_point.rs` | `KillPoint`, `Site`, `reached`, env/exit consts; gated `armed` (parse, mark, park); unit tests | T1 |
| `crates/htui/tests/worker_crash_pg.rs` | the child entry and K1-K5 against Postgres, real `GixIsolator`, real `SIGKILL` | T4 |

## Files to modify

| File | Change | Task |
|---|---|---|
| `crates/htui-orch/src/lib.rs` | `pub mod kill_point;` between `:36` and `:37`; one crate-doc sentence | T1 |
| `crates/htui-orch/src/engine.rs` | 2 call sites in `walk_live_step` (`:3466-3471`, `:3488-3492`); one `use` | T1 |
| `crates/htui-worker/src/runtime.rs` | `CommandPicked` site in `poll_once` (`:2476-2479`) | T1 |
| `crates/htui-worker/src/runtime.rs` | D3: `Shared.last_live` + `Shared.applied`, `Applying` notifies, `until_applied`, `cancels_first`, call in `adopt` | T2 |
| `crates/htui/src/run_worker.rs` | 4 D3 tests in a new section before `:3432` | T2 |
| `crates/htui-store/src/pg/write.rs` | `adopt_runs` SQL `AND kind = 'graph'` (`:4073-4074`), doc `:4049-4051` | T3 |
| `crates/htui-core/src/store/mem.rs` | `State::adopt_runs` filter (`:4334-4340`), doc `:4320-4321` | T3 |
| `crates/htui-core/src/store/traits.rs` | `WriteStore::adopt_runs` **doc only** (`:1139-1143`) | T3 (A-2) |
| `crates/htui-core/src/store/conformance.rs` | case `adopt_runs_never_leases_a_chat_run`: `CASES` (`:166`), `run_case` arm (`:~446`), fn | T3 |
| `crates/htui-core/tests/mem_store.rs` | pin `118` → `119` and its sentence (`:35-56`) | T3 (A-2) |
| `crates/htui-store/tests/pg_conformance.rs` | `EXPECTED_CASES` `118` → `119`, doc + message (`:17-31`) | T3 (A-2) |
| `crates/htui-store/.sqlx/` | delete `query-f65406bc…299` → `…604.json`, add the new hash (count stays 307) | T3 |
| `crates/htui-store/src/testkit.rs` | `fixture_box_store(url)` after `demo_db` (`:188`) | T4 |
| `crates/htui/src/agent_worker.rs` | `Answer.closes`, `Answer::closing`, async `send`, `answering`, `:1983`; 3 tests | T5 |
| `docs/htui-worker.md`, `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-24.md` | close-out | T6 |

---

## 1. T1 - `htui_orch::kill_point`

### 1.1 API (always compiled)

```rust
//! MOD-24 D1: where a crash test may stop a worker, at the walk's seams no test part reaches.
//!
//! [`reached`] is called at three sites. [`KillPoint::Documented`] is in the engine's single-step
//! walk, after the session sink wrote the step's output document and before verify.
//! [`KillPoint::Captured`] is in the same walk, after the step's `after` commits are recorded and
//! before its output is read and the step finished. [`KillPoint::CommandPicked`] is in
//! `htui-worker`'s command poll, once a pending row is this process's to apply and before its
//! cancel is spawned. Fan-out candidates and judges have none (plan D1).
//!
//! Without this crate's `test-support` feature, [`reached`] is an empty `#[inline]` function and
//! nothing reads the environment. The production `htui` binary never enables the feature.
//!
//! With the feature, the first call reads [`POINT_VAR`] once. Its grammar is
//! `<point>[@<phase>][#<attempt>]`, where `<point>` is a [`KillPoint::name`], and the optional
//! phase and attempt narrow the match to one step of a walk (for example `documented@prd#1`). A
//! matching call writes the marker file [`MARK_VAR`] names, written in full, synced and renamed
//! into place, so a reader never sees half of it. It then parks the calling thread until the
//! test's `SIGKILL`. A park still waiting after five minutes exits with [`PARK_EXPIRED_EXIT`]. A
//! malformed spec, or a spec with no marker path, exits with [`MISCONFIGURED_EXIT`] at the first
//! call. That half is a private module, `armed`, named in plain text: a link to it would be a
//! `broken_intra_doc_links` error in any build without the feature.

use htui_core::model::RunStep;

/// The environment variable naming the point, and optionally the phase and attempt, to stop at.
pub const POINT_VAR: &str = "HTUI_TEST_KILL_POINT";
/// The environment variable naming the marker file a reached point writes.
pub const MARK_VAR: &str = "HTUI_TEST_KILL_MARK";
/// The exit code of a process whose parked kill point was never killed.
pub const PARK_EXPIRED_EXIT: i32 = 86;
/// The exit code of a process whose kill-point spec does not parse, or names no marker.
pub const MISCONFIGURED_EXIT: i32 = 87;

/// A place a crash test may stop the process at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillPoint {
    /// After the step's output document, before verify and capture (K3).
    Documented,
    /// After the step's `after` commits, before `output_of` and `finish_step` (K4).
    Captured,
    /// After the command poll picked a pending row, before its cancel is spawned (K5).
    CommandPicked,
}

impl KillPoint {
    /// The name [`POINT_VAR`] spells it with: `documented`, `captured`, `command_picked`.
    #[must_use]
    pub const fn name(self) -> &'static str { /* match */ }
}

/// The step a point is reached for; both `None` outside a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site<'a> {
    /// `run_step.phase_name`.
    pub phase: Option<&'a str>,
    /// `run_step.attempt`.
    pub attempt: Option<i32>,
}

impl Site<'static> {
    /// No step: the command poll's site.
    pub const NONE: Self = Self { phase: None, attempt: None };
}

impl<'a> Site<'a> {
    /// `step`'s phase and attempt.
    #[must_use]
    pub fn step(step: &'a RunStep) -> Self {
        Self { phase: Some(&step.phase_name), attempt: Some(step.attempt) }
    }
}

/// MOD-24 D1: a no-op unless `test-support` is on and [`POINT_VAR`] names this point and site.
#[inline]
pub fn reached(point: KillPoint, at: Site<'_>) {
    #[cfg(feature = "test-support")]
    armed::reached(point, at);
    #[cfg(not(feature = "test-support"))]
    let _ = (point, at);
}
```

### 1.2 The gated half (`#[cfg(feature = "test-support")] mod armed`)

- `pub(super) struct Spec { point: KillPoint, phase: Option<String>, attempt: Option<i32> }`
  (`Debug, PartialEq`).
- `pub(super) fn parse(text: &str) -> Result<Spec, String>`. The text is trimmed.
  `rsplit_once('#')` splits off the attempt, an `i32 >= 1`. Then `split_once('@')` splits off the
  phase, which must not be empty. The rest must equal one of the three `name()`s. Anything else is
  `Err("<why>: `<text>`")`.
- `impl Spec { pub(super) fn matches(&self, point: KillPoint, at: Site<'_>) -> bool }`. The point
  must be equal. If the spec has a phase, `at.phase == Some(phase)`. If it has an attempt,
  `at.attempt == Some(attempt)`. So a spec naming a phase never matches `Site::NONE`.
- `pub(super) fn mark(path: &Path, point: KillPoint) -> io::Result<()>`. It writes
  `point.name()` to `path.with_extension("tmp")`, calls `sync_all`, then `rename`s the file to
  `path`.
- `struct Armed { spec: Spec, mark: PathBuf }`, `static ARMED: OnceLock<Option<Armed>>`.
- `fn from_env() -> Option<Armed>`:
  - `POINT_VAR` unset (or not UTF-8) gives `None`.
  - A parse error, or `MARK_VAR` unset, does `eprintln!` and `exit(MISCONFIGURED_EXIT)`.
- `pub(super) fn reached(point, at)`:
  1. `let Some(armed) = ARMED.get_or_init(from_env) else { return }`.
  2. `if !armed.spec.matches(point, at) { return }`.
  3. `mark`. On an error, `eprintln!` and `exit(MISCONFIGURED_EXIT)`.
  4. Loop with `std::thread::park_timeout(left)` while
     `deadline.checked_duration_since(Instant::now())` is `Some`. This tolerates spurious wakeups.
  5. Then `exit(PARK_EXPIRED_EXIT)`.

  `const PARK_LIMIT: Duration = Duration::from_secs(300)`.

### 1.3 Call sites

1. **`Documented`**: `engine.rs:3466-3471`, inside the `if let Ok(done) = &result` block, right
   after `sink.after_done(..).await?;`. The doc is written only on `Ok`, so the point exists only
   there:
   ```rust
   // MOD-24 D1 (K3): the output document is written; the trees are not captured.
   crate::kill_point::reached(KillPoint::Documented, Site::step(step));
   ```
2. **`Captured`**: `engine.rs`, after the `record_commits(.., &after).await?;` that ends at
   `:3491`, and before `let output = self.output_of(..)` at `:3492`:
   ```rust
   // MOD-24 D1 (K4): the `after` commits are recorded; `finish_step` is not.
   crate::kill_point::reached(KillPoint::Captured, Site::step(step));
   ```
   Add `use crate::kill_point::{KillPoint, Site};` beside the `use crate::…` block (`:53`). Call
   `reached` by its path, so `unused_qualifications` stays quiet. The fan-out twin at
   `:4087`/`:4107` (`candidate_live`) gets **no** call (D1).
3. **`CommandPicked`**: `runtime.rs` `poll_once`, in `for row in rows`, after the
   `if ctx.shared.is_applying(row.id) { continue; }` at `:2476-2478`, and before
   `let task = ctx.unaddressed("cancel_run");` at `:2479`:
   ```rust
   // MOD-24 D1 (K5): this process will apply the row; nothing has applied it yet.
   htui_orch::kill_point::reached(KillPoint::CommandPicked, Site::NONE);
   ```
   Add `use htui_orch::kill_point::{KillPoint, Site};`.

**How `htui-worker` reaches it.** `htui-orch` is a normal dependency
(`crates/htui-worker/Cargo.toml:20`), so the module always resolves. The hook is armed only when
`htui-orch/test-support` is unified on. That is true for `htui`'s test targets
(`crates/htui/Cargo.toml:76`, a dev-dependency) and `htui-worker`'s own tests (`:33`). It is not
true for `cargo build -p htui --bin htui`. Note: during `cargo test`, the `htui` binary that cargo
builds for `CARGO_BIN_EXE_htui` **is** armed (H-3).

**Crate doc** (`lib.rs`, end of the `//!` paragraph before `:23`): "MOD-24 adds [`kill_point`],
the crash tests' hook at three seams of a walk and the command poll; without `test-support` it
compiles to nothing."

### 1.4 Unit tests (`#[cfg(all(test, feature = "test-support"))] mod tests`, inside `kill_point.rs`)

| Test | Asserts |
|---|---|
| `a_spec_names_a_point_and_optionally_a_phase_and_an_attempt` | `documented`, `captured@prd`, `command_picked`, `documented@prd#1`, `captured#2` parse to the expected `Spec` |
| `a_malformed_spec_is_refused` | `""`, `nap`, `documented@`, `documented@prd#0`, `documented@prd#x`, `captured@#1` are `Err` |
| `a_spec_fires_only_for_its_point_phase_and_attempt` | `documented@prd#1` matches `(Documented, prd, 1)`, not `(Captured, prd, 1)`, `(Documented, plan, 1)`, `(Documented, prd, 2)` or `Site::NONE` |
| `a_bare_point_matches_every_site_of_that_point` | `command_picked` matches `Site::NONE` and any step site |
| `a_reached_point_leaves_the_whole_marker_and_no_temp_file` | `mark(dir/m, Captured)`: `read_to_string == "captured"`, and `m.tmp` does not exist |
| `the_names_round_trip` | `parse(p.name())` is `p` for all three |

**Red:** the `parse`, `matches` and `mark` stubs answer `Err("todo")`, `false` and `Ok(())`, so
the tests fail on their assertions. The park is not unit-tested; T4 covers it.

### 1.5 T1 validate

```bash
cargo test -p htui-orch --features test-support --lib kill_point
cargo check -p htui-orch && cargo build -p htui --bin htui                 # the no-op build
cargo doc -p htui-orch --no-deps && cargo doc -p htui-orch --no-deps --features test-support
cargo clippy -p htui-orch --all-targets -- -D warnings
cargo clippy -p htui-orch -p htui-worker --all-targets --all-features -- -D warnings
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/t1.log; grep -c SIGABRT /tmp/t1.log   # 0
cargo test -p htui-worker --all-features -- --test-threads=1
```
Commit: `feat(mod-24): kill-point hook for the crash tests`.

---

## 2. T2 - D3, commands before recovery (`crates/htui-worker/src/runtime.rs`)

### 2.1 State

- `Shared` (`:190-218`): after `applying` (`:216`), add:
  ```rust
  /// MOD-24 D3 (B5): the live chats the last command poll was handed; the sweep's own cancels
  /// honour them as the poll does (D212). Always empty for the worker.
  last_live: StdMutex<LiveChats>,
  /// MOD-24 D3: notified whenever an `Applying` guard drops, so the sweep can await a row the
  /// poll is applying.
  applied: tokio::sync::Notify,
  ```
  `assemble` (`:1084`): `last_live: StdMutex::default(), applied: Notify::new(),`. Extend the
  import at `:32` with `Notify`.
- `Applying` (`:488-500`): add `done: &'a Notify`. `Shared::applying` (`:468-478`) fills it with
  `&self.applied`. `Drop` removes the id and then calls `self.done.notify_waiters()`.
- A new `Shared` method, after `is_applying` (`:480-485`):
  ```rust
  /// MOD-24 D3: until no task of this process applies `id` (B-5's guard is free).
  async fn until_applied(&self, id: RunCommandId) {
      loop {
          let notified = std::pin::pin!(self.applied.notified());
          let mut notified = notified;
          notified.as_mut().enable();          // registered before the check: no lost wakeup
          if !self.is_applying(id) {
              return;
          }
          notified.await;
      }
  }
  ```
- `poll_commands_with` (`:1160`): as its first line, before `prune_tasks`, add
  `*self.shared.last_live.lock().unwrap_or_else(PoisonError::into_inner) = live.clone();`. It runs
  every tick, even when the in-flight check skips the poll.

### 2.2 `cancels_first` (new private fn after `adopt`, `:~1881`) and its call

In `adopt` (`:1831`), the first statement becomes `cancels_first(ctx).await;`. It runs before
`Kit::read` at `:1833`, because `cancel_run` builds its own kit and a failed kit read must not
skip the cancels.

```rust
/// MOD-24 D3 (OQ-2): before the sweep recovers anything, every pending cancel this process may
/// apply (`pending_commands(owner, box)`, B-4) whose run is `running` and not walked here is
/// applied, through the poll's own `cancel_run`, and then awaited under B-5's guard: a row the
/// poll took first is waited for, never applied twice. Such a run is cancelled without recovery
/// (`cancel_leased` takes the lapsed lease) instead of being walked on, or finished by the
/// recovery itself, which used to lose the user's cancel to the crash.
///
/// Left to the poll: a run this process walks (a graceful preempt), and parked or terminal runs
/// (the sweep never adopts them, so nothing races them). A run whose lease is live elsewhere is
/// not in `pending_commands` at all. Failures are logged as the poll's are: a read at `warn`, a
/// refused cancel at `debug` (its row stays pending, and recovery proceeds as before).
async fn cancels_first<H: htui_core::store::WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>) {
    let Some(writer) = ctx.host.writer() else { return };
    let box_id = match registered_box(&ctx.host).await {
        Ok(box_id) => box_id,
        Err(err) => return tracing::debug!(%err, "the sweep could not read this box for its cancels"),
    };
    let rows = match htui_core::store::WorkerStore::pending_commands(&writer, ctx.shared.owner, box_id).await {
        Ok(rows) => rows,
        Err(err) => return tracing::warn!(%err, "the sweep could not read the pending run commands"),
    };
    let live = ctx.shared.last_live.lock().unwrap_or_else(PoisonError::into_inner).clone();
    for row in rows {
        if ctx.shared.walks.is_live(row.run_id) {
            continue;
        }
        let running = matches!(
            htui_core::store::WorkerStore::run(&writer, row.run_id).await,
            Ok(Some(run)) if run.status == RunStatus::Running
        );
        if !running {
            continue;
        }
        Box::pin(cancel_run(ctx.unaddressed("cancel_run"), row.run_id, &live, Some(row.id))).await;
        // B-5: if the poll took the row first, `cancel_run` returned at once; its application ends here.
        ctx.shared.until_applied(row.id).await;
    }
}
```

Notes:
- It reuses `cancel_run(task, run, live, Some(row_id))` (`:2297`) unchanged. With
  `existing = Some`, it skips the queued/terminal branch and writes no second row. Its
  `refuse` logs at `debug` and never publishes (B-10). Its `applying` guard (`:2382`) is B-5's.
  Its success path is `resolve(Applied)` plus `ctx.done`.
- Each row gets a fresh `ctx.unaddressed(..)`, because `TaskCtx.tag` is a `OnceLock` per context.
- `Box::pin` keeps the sweep task's future small. It is spawned, so this is not a stack issue,
  but it follows the repo's habit for big engine futures.
- A dead walk of this process (its lease is ours, the walk is gone) is included. The sweep's
  pre-pass would re-adopt and recover it. `cancel_run` → `take_lease` → `renew_lease` removes it
  from `DeadWalks` and releases its guards (`engine.rs:1968-1988`).
- After the loop, `sweep_fenced` sees `cancelled` runs, which `adopt_runs` never selects
  (`status = 'running'`). A run whose cancel was refused transiently is recovered as today.
- I-1 is preserved: `adopt` is reached only past `sweep_once`'s
  `role.executes(&executor)` check (`:1795-1797`).
- Also update `adopt`'s doc (`:1829-1830`): "…first the pending cancels of free runs (MOD-24 D3),
  then every lapsed lease on the box adopted…".

### 2.3 Tests (`crates/htui/src/run_worker.rs`)

New section `// MOD-24 D3: commands before recovery`, inserted after
`a_polled_cancel_under_a_live_chat_stays_pending_and_silent` (`:3364-3425`) and before the
`B-20` test at `:3432`. Reused fixtures:
- `Fixture::new` / `Fixture::over` (`:601-615`), `stranded` (`:1342`), `requested` (`:2919`),
  `commands_of` (`:2909`), `swept` (`:2784`), `polled` (`:2987`), `set_executor` (`:2715`),
  `within` (`:749`), `only_run` (`:794`).
- `Play::Stall` / `Stall` (`:303-345`), `Sessions::push`, `testing::probe`,
  `MemFault::RefreshLease`, `OutputAuthor`.
- New import: `htui_core::store::StepFence`, `htui_core::model::{Gate, RunStepCommit}`.

| # | Test | Shape | Asserts | Red before D3 |
|---|---|---|---|---|
| 1 | `a_sweep_applies_a_pending_cancel_before_it_recovers_a_free_run` | `stranded(ANA_2)` (running, lapsed lease, no step); `set_executor(Worker)`; `runtime().with_role(Role::Worker)`; `requested`; `swept` | run `Cancelled`; `commands_of == [(id, Applied)]`; `fixture.steps(run).is_empty()` (no session, nothing recovered); `!probe.is_applying(id)` | the sweep adopts, `recover_run` answers `Walk`, the resumed walk creates step 0, which parks at its gate: the status is `AwaitingApproval`, not `Cancelled`, and the command is `Pending` |
| 2 | `a_cancel_beats_the_recovery_that_would_finish_the_run` (`#[tokio::test(start_paused = true)]`) | demo with every `GRAPH_HTUI_ANA` phase `gate = Gate::Never, gate_hard = false` (`data.phases`, then `Fixture::over(MemStore::from_demo(data))`); assert each has `fan_out == 1`; `lease_ttl_seconds = 30`; push `n-1` `Play::Done` then `Play::Stall`; `runtime.serve(start_run(ANA_2))` (Deferred, **no** settle); `within(stall.reached)`; then, as the walk's owner (`probe.owner()`): `write_document` (last phase's `output_kind`, `produced_by_step_id = last step`) and `record_commits(StepFence::Lease(owner), last, &[before rows with after_hash = Some("a"*40)])`; `set_fault(RefreshLease, true)`; wait until `probe.is_dead_walk(run)`; `settle`; clear the fault; `requested`; `swept` | run `Cancelled`; command `Applied`; the last step `Cancelled` (`cancel_leased`), not `Done`; no step `Done` at the last position | **deterministic**: the sweep classifies the last step `Finished` (document plus captured), `gate::apply` (ungated) lands `done`, and the run is `Done` with the command still `Pending` (fact 5) |
| 3 | `a_sweep_and_a_poll_in_one_tick_apply_one_cancel_once` | as 1, but `runtime.sweep(..)` then `runtime.poll_commands(.., &LiveChats::default())` back to back, then one `settle` | run `Cancelled`; exactly one row, `Applied`; `steps.is_empty()`; no task stuck | **B-5 guard, not TDD-red**: before D3 the outcome depends on the interleaving. Say so in its doc |
| 4 | `a_sweep_leaves_a_live_leases_cancel_to_its_holder` | `stranded`, then `take_lease(run, BOX, Uuid::now_v7(), days(1))` (a stranger holds it, live); `requested`; `swept` | the row is `Pending`; the run is `Running`; no step | **regression guard** (passes before and after): `pending_commands` excludes it, and so does `adopt_runs` |

Recommended (B5), not plan-mandated: `a_sweep_does_not_cancel_under_a_chat_the_poll_refused`.
- Set it up as `a_polled_cancel_under_a_live_chat_stays_pending_and_silent` does, but on a
  `running` free run with a step id in `LiveChats`.
- `polled_watching(.., &LiveChats::of([step]))` stores the set; then `swept`.
- The row stays `Pending`.

Write it if constructing a `running` run with a step under a chat costs under ~40 lines. Otherwise
B5 is covered by review.

### 2.4 T2 validate

```bash
cargo test -p htui --features testkit --lib run_worker -- --test-threads=1
cargo test -p htui-worker --all-features -- --test-threads=1
cargo clippy -p htui-worker -p htui --all-targets --all-features -- -D warnings
```
Commit: `fix(mod-24): the sweep applies pending cancels before it recovers`.

---

## 3. T3 - D3b, the sweep adopts graph runs only

### 3.1 Postgres (`crates/htui-store/src/pg/write.rs:4065`)

In the `candidates` CTE, insert one line after `AND status = 'running'` (`:4073`). Keep the
indentation; the `.sqlx` hash is of the literal text:

```sql
                 WHERE executing_box_id = $1
                   AND status = 'running'
                   AND kind = 'graph'
                   AND (lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())
```
Doc (`:4049`): "every `running` **graph** run on the box … A chat run (`kind = 'chat'`, `running`
with no lease, `start_chat_run`) is never the sweep's: the engine cannot recover it, and leasing
it fenced the chat's own unleased writes until `unrecovered` gave it back (MOD-24 D3b)."

### 3.2 `MemStore` (`crates/htui-core/src/store/mem.rs:4322`)

In the `filter` (`:4335-4340`), add `&& row.kind == RunKind::Graph` after the status test.
`RunKind` is already imported (`:39`). Mirror the doc sentence at `:4320`.

### 3.3 Trait doc (`crates/htui-core/src/store/traits.rs:1139`)

"ANA-2 §4.9's sweep: every `running` run of `kind = 'graph'` whose `executing_box_id` is …; never
a chat run (MOD-24 D3b)." The signature is unchanged (A-2).

### 3.4 Conformance case

Name `adopt_runs_never_leases_a_chat_run`. Append it to `CASES` after
`"deleting_a_project_takes_its_relay_rows"` (`:166`), add its `run_case` arm before
`other => panic!` (`:~446`), and put the fn after `pending_commands_are_the_owners_and_the_boxs_free_runs`
(`:14182`):

```rust
/// MOD-24 D3b: the sweep adopts graph runs only. A chat run is `running` on its box with no lease,
/// and a sweep that leased it fenced the chat's unleased writes; a free graph run beside it is
/// still adopted.
async fn adopt_runs_never_leases_a_chat_run<S: WriteStore>(store: &S) {
    const CASE: &str = "adopt_runs_never_leases_a_chat_run";
    let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
    let at = seam_clock();
    let chat = ChatRunSpec::mint(ids::PROJECT_HTUI, ids::BOX, ids::USER, Some(ids::AGENT_CLAUDE),
                                 Some("sonnet".to_owned()));
    store.start_chat_run(&chat).await.expect(CASE);
    let graph = store.create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await.expect(CASE).id;
    assert_eq!(store.claim_run(graph, ids::BOX, x, at, TimeDelta::minutes(5)).await.expect(CASE),
               Claim::Admitted, "{CASE}: X claims the graph run");
    assert!(store.release_lease(graph, x).await.expect(CASE), "{CASE}: and frees it");
    let before = run_row(CASE, store, chat.run_id).await;

    let adopted = store.adopt_runs(ids::BOX, y, TimeDelta::minutes(10)).await.expect(CASE);
    assert_eq!(adopted.iter().map(|row| row.id).collect::<Vec<_>>(), vec![graph],
               "{CASE}: the free graph run, never the chat");
    assert_eq!(run_row(CASE, store, chat.run_id).await, before,
               "{CASE}: the chat run is untouched: no lease box, no expiry");
    assert_eq!(store.append_events(StepFence::Unleased, &[chat_event(chat.step_id, 0)])
                   .await.expect(CASE), 1,
               "{CASE}: the chat still writes unleased after the sweep");
}
```
**Red** on both stores: today's `adopted` is `[chat, graph]` (or the reverse, by `queued_at`), and
the unleased append is refused `Fenced`. Both are run by
`crates/htui-core/tests/mem_store.rs:12` `mem_store_conformance` (`run_all`, with a fresh
`MemStore::demo()` per case) and by `crates/htui-store/tests/pg_conformance.rs:36-46`
`pg_store_conformance`, which loops `CASES` over a fresh `demo_db()` per case
(`#[cfg(feature = "demo")]`, implied by `test-support`).

Pins: `mem_store.rs:35-37` becomes `119`, and the sentence gains ", and MOD-24's one for the
sweep's graph-only adoption (plan D3b)". In `pg_conformance.rs:21`, `EXPECTED_CASES = 119`, plus
its `//!` line and the message at `:29-30`.

### 3.5 Other callers

Every call of `adopt_runs`, by text search (the graph has no edges for it):
- `engine.rs:2233`, the sweep, which is the only production caller.
- Delegating wrappers: `htui-agent/src/conformance.rs:1095`, `tests/recorder.rs:820`,
  `htui-store/src/worker.rs:176`, `:455`, `writer.rs:874-875`, `htui-core/src/store/worker.rs:568`.
- Tests over graph runs only: `conformance.rs:5407-5477`, `:5727`, `:6684`, `:6801`, `:6844`;
  `mem.rs:10086-10140`; `engine.rs:8801`, `:10894`, `:10988`; `pg_criteria.rs:778-779`, `:1003`,
  `:1053`.

None relies on a chat being adopted, and no test expects `unrecovered`'s note on a chat run (no
test asserts a chat-snapshot recovery error).

`active_runs_on_box` (`pg/read.rs:1981`, `mem.rs:767`) still counts chats. It is the slot count,
and in `sweep_once` (`runtime.rs:1820-1824`) it only decides whether `adopt` runs at all. With a
live chat, `adopt` now runs, adopts nothing, and logs nothing. No change.

### 3.6 `.sqlx` (sandbox recipe, `docs/hr-sandbox.md:194-210`)

```bash
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx_mod24;" -c "CREATE DATABASE htui_sqlx_mod24;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod24
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features
cd ../.. && git status --short crates/htui-store/.sqlx      # exactly: D query-f65406bc…604.json, ?? query-<new>.json
ls crates/htui-store/.sqlx | wc -l                           # 307
SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE htui_sqlx_mod24;"
```
The test DSN database and this prepare database are different databases (project memory). Any
other `.sqlx` change in `git status` means drift. Stop and investigate; do not commit it.

### 3.7 T3 validate

```bash
cargo test -p htui-core --features test-support --test mem_store
cargo test -p htui-core --features test-support --lib store::mem
cargo test -p htui-store --features test-support --test pg_conformance -- --test-threads=1
cargo test -p htui-store --features test-support --test pg_criteria -- --test-threads=1
SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1   # the engine's sweep tests
```
Commit: `fix(mod-24): the sweep never leases a chat run`.

---

## 4. T4 - the kill test (`crates/htui/tests/worker_crash_pg.rs`)

### 4.1 `htui_store::testkit::fixture_box_store` (after `demo_db`, `testkit.rs:188`)

```rust
/// A second process's store on `url`, a [`demo_db`] database, registered as the fixture's box
/// `ids::BOX`: the box `db.store` stands for after `load_demo` repointed it (MOD-24 D2). A
/// `PgStore::connect(url, &db.identity)` would name the minted box `demo_db` deleted.
///
/// The fixture box belongs to the fixture user, the only `app_user` left and therefore the oldest,
/// and its row carries no fingerprint, so registration answers `Known` for any machine. The first
/// connect records this machine's fingerprint, and a later one from the same machine is `Known`
/// again. Registration also refreshes the row's display columns (`os_family`, `arch`). The
/// hostname is kept as the fixture's.
///
/// # Panics
/// When the headless connect fails, or registration answers any box but `ids::BOX`.
pub async fn fixture_box_store(url: &str) -> PgStore {
    let identity = Identity { box_id: htui_core::fixtures::ids::BOX, hostname: "DESKTOP-HTUI".to_owned() };
    let store = PgStore::connect_headless(url, &identity, crate::pg::CONNECT_TIMEOUT,
                                          crate::pg::PoolSize::WORKER_DEFAULT)
        .await
        .expect("the fixture box connects headless");
    assert_eq!(store.this_box(), htui_core::fixtures::ids::BOX,
               "registered as the fixture box, not a copy: {:?}", store.registration());
    store
}
```
Evidence:
- `connect_headless` is at `pg/mod.rs:285-327`; it runs the same `bootstrap` as `connect`.
- `seed_if_empty_as` (`:437-517`) is all `ON CONFLICT DO NOTHING`. It never undoes the case's TTL
  or the disabled fixture agents. It answers the oldest user.
- `register_box` is at `:546-640`; it is `copied` only on another user or a differing stored
  fingerprint.
- The fixture `boxes()` row is `fixtures.rs:462-485` (`user_id: ids::USER`, no fingerprint field).

### 4.2 File shape

`//!` doc: the plan's D2 in prose. It says that the file re-execs itself, that the kill is a real
`SIGKILL`, why not in process (fact 7), and that it is ungated and skips with `testkit::SKIP`.
Then `#![cfg(target_os = "linux")]`, as `worker_pg.rs`'s binary cases are (`Reaped`, `signal()`).

Constants:
```rust
const PATIENCE: Duration = Duration::from_secs(60);   // child start + four git-backed phases + a TTL
const CONFIG: WorkerConfig = WorkerConfig { poll: Duration::from_millis(50), box_beat: Duration::from_secs(1), grace: Duration::ZERO };
const TTL_SECONDS: i64 = 2;                           // H-9
const CHILD: &str = "crash_child";                    // the child entry's fn name; `--exact` needs it top-level
const DSN_VAR: &str = "HTUI_TEST_CRASH_DSN";          // TestDb.url; the child entry's switch
const REPO_VAR: &str = "HTUI_TEST_CRASH_REPO";        // the seeded primary repo's RepoId
const CHECKOUT_VAR: &str = "HTUI_TEST_CRASH_CHECKOUT";// the temporary git repository
const SCRATCH_VAR: &str = "HTUI_TEST_CRASH_SCRATCH";  // the scratch root, outside the checkout
const SCRIPT_VAR: &str = "HTUI_TEST_CRASH_SCRIPT";    // walks | stall | park
const TREE: &str = "htui";                            // RepoCheckout.name: the tree dir under a session's cwd
```
`htui_orch::kill_point::{POINT_VAR, MARK_VAR}` name the hook's own vars.

### 4.3 Parts (copied, then adapted)

- `OutputAuthor`: copied verbatim from `worker_pg.rs:84-104`. A `review` body approves.
- `Committing`, a session wrapper. It is the `Stalled`/`Graced` shape (`run_worker.rs:463-537`)
  over a `FakeDriver` session playing `Script::one_turn([Done(EndTurn)])`.
  - On its first `next_event` it writes `<cwd>/htui/step-<step_id>.txt` and runs
    `git -C <tree> add -A`, then
    `git -C <tree> -c user.name=htui-test -c user.email=test@localhost commit -q -m "step <id>"`.
    It asserts both succeed and that `<tree>/.git` exists, so a changed layout fails loudly (H-12).
  - Then it delegates.
  - The cwd and step come from `SessionSpec { cwd, step_id, .. }`, captured in the driver's
    `start`, as `ScriptedDriver::start` (`run_worker.rs:435-461`) does.
- `Commits`, a `TransportBuilder`: every session is `Committing`. Script `walks`.
- `StallsOnce(AtomicBool, PathBuf mark)`, script `stall` (K1).
  - The first session's first `next_event` writes `<cwd>/htui/stray.txt` (uncommitted).
  - It then writes the marker as the hook does (`<mark>.tmp`, `sync_all`, `rename`).
  - It then awaits `std::future::pending::<()>()`.
  - Later sessions are `Committing`.
- `ParksOnce`, script `park` (K2, K5): `worker_pg.rs:131-178`, except that later sessions are
  `Committing` and not `Walks`.

### 4.4 The child entry

```rust
/// MOD-24 D2: the worker a case kills. Returns at once unless a case spawned this binary with
/// `DSN_VAR` set; then it is `htui worker`'s loop over the case's database as the fixture box,
/// with the real isolator, until its stdin closes (or a SIGKILL).
#[tokio::test(flavor = "multi_thread")]
async fn crash_child() {
    let Ok(dsn) = std::env::var(DSN_VAR) else { return };
    let host = testkit::fixture_box_store(&dsn).await;
    let repo: RepoId = var(REPO_VAR).parse().expect("a RepoId");        // ids impl FromStr (model/ids.rs:54)
    let isolator = GixIsolator::new(IsolatorConfig {
        repos: BTreeMap::from([(repo, RepoCheckout { name: TREE.to_owned(), local_path: var(CHECKOUT_VAR).into(), is_primary: true })]),
        scratch_root: var(SCRATCH_VAR).into(),
        copy_exclude: Vec::new(),
        copy_max_total_bytes: 1 << 30,
        box_id: ids::BOX,
    }).expect("the scratch root is outside the checkout");
    let transport: Box<dyn TransportBuilder> = match var(SCRIPT_VAR).as_str() {
        "walks" => Box::new(Commits), "park" => Box::new(ParksOnce::default()),
        "stall" => Box::new(StallsOnce::new(var(MARK_VAR).into())), other => panic!("unknown script {other}"),
    };
    let mut factory = DriverFactory::new();
    factory.register("acp", transport);
    let runtime: RunRuntime<PgStore, Unaddressed> = RunRuntime::with_parts(
        Arc::new(isolator) as Arc<dyn Isolator>, Arc::new(FakeVerifier::new()) as Arc<dyn Verifier>, factory)
        .with_author(Arc::new(OutputAuthor)).with_role(Role::Worker);
    worker::run(host, runtime, CONFIG, stdin_closed()).await;
}
```
- `stdin_closed()` is `tokio::task::spawn_blocking`, reading `std::io::stdin()` until it returns
  0 bytes or an error. It never outlives the loop: it has returned by the time `run` resolves.
- `IsolatorConfig` fields are `isolate/real.rs:233-246`; `GixIsolator::new` is `:313`. Injected
  parts skip the isolator rebuild (`runtime.rs:363-365`).
- `with_parts`/`with_author`/`with_role` are at `runtime.rs:1049`, `:1253`, `:1221`.

### 4.5 The parent side

- **`Stack`** (trimmed copy of `worker_pg.rs:282-529`): `db`, `_root`, `cache`, `backend`, `tui`,
  `seq`; `command`, `start`, `exit_tui`, `set_executor`, `ungate_feat`, `run_row`, `steps`,
  `lease_owner`, `released`, `rested`, `finish`. `seed` (`:220-276`) now **returns** the
  `RepoId` it creates.
- **`Case::new() -> Option<Case>`**:
  1. `skip_without_git!()` (`htui_orch::skip_without_git`, `isolate/git.rs:2068`) returns `None`
     with its reason when git is missing.
  2. `Stack::new()` returns `None` after `testkit::SKIP`.
  3. `ungate_feat()`; `set_executor(Executor::Worker)`.
  4. `UPDATE app_setting SET value = to_jsonb($1::integer) WHERE key = 'lease_ttl_seconds'`
     with `TTL_SECONDS` (`recover.rs:23`; seeded `120` by `0003`).
  5. `dir = tempdir()`. `checkout = dir/repo`, made with
     `htui_orch::isolate::git::testkit::repo_with_one_commit(&checkout)` (`git.rs:2168`).
     `scratch = dir/trees`. `mark = dir/killed`.
  6. `(run, rest) = stack.start(ids::HTUI_FEAT_3)`. Assert `rest.run == Queued` (a worker box
     only queues, plan D12).
  7. Assert `run_row(run).repo_scope == vec![repo]`. This is K3's precondition: a `None` scope
     resolves to the primary (`graph.rs:362`, test `:1484-1497`).
  8. `stack.exit_tui()`. The parent then holds no runtime that could sweep (H-8).
- **`spawn(case, n, script, point: Option<&str>) -> Reaped`**:
  - `Command::new(std::env::current_exe())` with args
    `["--exact", CHILD, "--nocapture", "--test-threads=1"]`.
  - `.env` the five `HTUI_TEST_CRASH_*` vars and `MARK_VAR`. `POINT_VAR` is set when `point` is
    `Some`, else `.env_remove(POINT_VAR)` (H-3).
  - Also `GIT_AUTHOR_NAME/EMAIL` and `GIT_COMMITTER_NAME/EMAIL` (H-13).
  - `stdin(piped)`; stdout and stderr both go to `dir/child-<n>.log` (`File` + `try_clone`).
- **`Reaped`**: copied from `worker_pg.rs:1078-1104` (kill and wait on drop).
- **`marked(child, case, n)`** polls every 20 ms until the marker exists.
  - It panics with the child's log if `try_wait()` shows the child exited. The message includes
    whether the code is `MISCONFIGURED_EXIT` or `PARK_EXPIRED_EXIT`.
  - It panics after `PATIENCE`.
- **`killed(child) -> ()`**: `kill()` then `wait()`. Assert
  `ExitStatusExt::signal(&status) == Some(9)` (probe: `/tmp/mod24-probe/tests/crash.rs:51-56`).
- **`stopped(child)`**: drop its stdin, then poll `try_wait` for up to `PATIENCE`. Assert exit
  code `0`: the child's test returned, so the restarted loop shut down cleanly.
- **Raw reads**:
  - `notes(item)`: `SELECT body FROM item_note WHERE item_id = $1` (`0001_init.sql:374-382`).
  - `produced_by(item, kind)`:
    `SELECT produced_by_step_id FROM document WHERE item_id = $1 AND kind = $2` (`:389-400`).
  - `command_statuses(run)`: copied from `worker_pg.rs:827-838`.
  - `events(step)`: `(seq, kind)` from `db.store.step_events`.
  - `landed(checkout)`: `git -C <checkout> ls-tree -r --name-only HEAD`.
- **`another_box`** and **`pending_request`**: copied from `worker_pg.rs:782-814`.
- **`prd(steps, attempt) -> Option<&RunStep>`**: `phase_name == "prd"` at that attempt.

Never a sleep before a kill: every kill follows a marker, or a DB row (K2), observed by a bounded
poll.

### 4.6 The five cases (each ends `stopped(second)`, then `stack.finish()`)

Common "interrupted then retried" assertions (K1-K3), read once the run is at rest
(`stack.rested(run)`):
- (a) `prd#1.status == Failed` and `gate_note == Some("interrupted")`. The const is private; this
  is `engine.rs:104`'s value.
- (b) Some `notes(FEAT_3)` entry contains `"(`prd` attempt 1) did not finish"` and
  `"retrying as attempt 2"` (`engine.rs:2659-2664`).
- (c) `prd#2.status == Done`.
- (d) `run.status == Done`, and every phase has a `Done` step.
- (e) `released(run)`.
- (f) `landed(checkout)` contains `step-<prd#2.id>.txt`. This positive control proves that a tree
  landed.

| Case (test name) | First child | Kill trigger | Extra assertions |
|---|---|---|---|
| **K1** `a_worker_killed_before_any_event_retries_the_step` | `stall`, no point | marker from `StallsOnce` (after `stray.txt`) | `killed` signal 9; common (a)-(f); `!landed(checkout).contains("stray.txt")`; `!checkout.join("stray.txt").exists()`; prd#2's tree (`db.store.step_trees(prd2)` path) has no `stray.txt`; `events(prd#1)` holds only the `Prompt` row (no driver event was flushed) |
| **K2** `a_worker_killed_after_a_flush_keeps_its_rows_and_retries` | `park`, no point | poll until prd#1 exists and `events(prd#1)` has a non-`Prompt` row (the `ToolCall`/request flush, `record.rs:896-901`) | read `at_kill = events(prd#1)` **after** the kill; common (a)-(f); `events(prd#1) == at_kill` at rest, non-empty beyond the prompt (`R-HIS-1`) |
| **K3** `a_worker_killed_after_the_document_retries_the_step` | `walks`, `documented@prd#1` | marker | before the restart: `produced_by(FEAT_3, "prd") == [prd#1]`; common (a)-(f); at rest `produced_by(FEAT_3, "prd")` as a set is `{prd#1, prd#2}` (attempt 2 wrote its own, and its step settled `Done` over it) |
| **K4** `a_worker_killed_after_capture_settles_the_step_done` | `walks`, `captured@prd#1` | marker | before the restart: `step_commits(prd#1)` has `after_hash.is_some()` (B8 precondition), and `at_kill = events(prd#1)`; at rest: `prd#1.status == Done`, no prd attempt 2 row, `events(prd#1) == at_kill` (no new session), no note contains `"retrying as attempt 2"`; run `Done`; `released`; `landed` contains `step-<prd#1.id>.txt` (recovery's reconcile merged it) |
| **K5** `a_cancel_picked_but_not_applied_survives_the_kill` | `park`, `command_picked` | after `pending_request(client, FEAT_3)` (parked, so the child's lease is live): `client.request_cancel(run, client.this_user(), client.this_box())` is `Inserted` (`another_box`, `worker_pg.rs:782`); then the marker | `killed` signal 9; at rest: `run.status == Cancelled`; `command_statuses(run) == [Applied]`; every step has `attempt == 1`, and none is `Done` (prd#1 `Cancelled`): no recovery ran (D3), so the run never reached `done`; `released` |

Every case also asserts `killed`'s signal 9 and `stopped`'s exit 0.

K5 runs with or without D3 winning the race against the poll. Once the dead lease lapses, the
restarted child's sweep (every 50 ms) reaches the row before its 1 s poll. Either path cancels a
free run without recovery. Before D3, the sweep would recover and walk prd#2, and the case fails
on `attempt == 1`. So K5 is red without T2.

### 4.7 T4 validate

```bash
cargo test -p htui --features testkit --test worker_crash_pg -- --test-threads=1
for i in 1 2 3; do cargo test -p htui --features testkit --test worker_crash_pg -- --test-threads=1 || break; done
pgrep -af crash_child            # empty: no orphan
ls /tmp | grep -c '^htui-test-'  # no new debris beyond TempDir's
cargo clippy -p htui -p htui-store --all-targets --all-features -- -D warnings
```
Commit: `test(mod-24): a real worker kill at five points, against Postgres`.

---

## 5. T5 - D4, a panicking chat closes its own run (`crates/htui/src/agent_worker.rs`)

### 5.1 Code

- `struct Answer` (`:3403-3407`) gets a new field:
  ```rust
  /// MOD-24 D4: a fresh chat's `run(kind='chat')` pair, closed `failed` before the last word is
  /// sent, so a tab that re-reads runs on `Ended` finds it closed. `None` for every other task, and
  /// for a promoted step's chat, which opened no run (MOD-4 D165).
  closes: Option<(Writer, ChatRunSpec)>,
  ```
  `Frames::answer` (`:3431-3437`) sets `closes: None`.
- New method `Answer::closing`:
  ```rust
  /// MOD-24 D4: this answer closes `chat`'s run first. Copies, not the task's state: the task
  /// is dropped before the answer is sent.
  fn closing(mut self, writer: Writer, chat: ChatRunSpec) -> Self { self.closes = Some((writer, chat)); self }
  ```
- `Answer::send` (`:3415-3426`) becomes `async fn send(self, message: String)`. First:
  `if let Some((writer, chat)) = &self.closes { close_run(writer, chat, RunStatus::Failed).await; }`,
  then the existing frame loop. `close_run` (`:4096-4103`) logs its own failure at `error`.
- `answering` (`:3524-3550`): `answer.send(message).await;`. The order stays: `drop(task)`, then
  the log, then close, then `Failed`, then `Ended`.
- The start site (`:1983`), before `ChatArgs { .., writer, binding: ChatBinding::Fresh(chat), .. }`
  moves both:
  ```rust
  let answer = frames.answer(chat_failed).closing(writer.clone(), chat.clone());
  ```
  The whole `ChatRunSpec` is copied, not only `run_id`: `finish_chat_run` needs the step id too
  (A-5). The promoted site (`:1015`) is unchanged, so a promoted chat closes nothing.
- Docs:
  - `LastWord`'s doc (`:3393-3397`): "carries no state of the task it outlives; a fresh chat's
    answer carries copies of the run's ids and a writer (MOD-24 D4)".
  - `chat_failed` (`:3492-3494`): "…after the chat's run is closed `failed` (MOD-24 D4)".
  - `answering` (`:3512-3517`): "on an unwind the task is dropped, a fresh chat's run is closed,
    and `answer` is sent".

### 5.2 Tests (beside `:10336`)

| Test | Fixture | Asserts | Red |
|---|---|---|---|
| extend `a_chat_that_panics_ends_its_stream_with_failed_then_ended` (`:10336`) | as today, but `Backend::memory(store.clone())`; `before = store.active_runs(&scope())` (`scope()` `:4598`) | adds `store.active_runs(&scope()) == before`, "the panicked chat's run is closed" | today `before + 1`: the pair is still `running` |
| new `a_chat_that_panics_mid_turn_closes_its_run_then_ends_its_stream` | `MemStore::demo()` + `fake_row` (`:4452`); factory `"cli/fake"` → `PanicsMidTurn` (new): a driver over `FakeDriver::new(agent.name.clone(), caps, Script::one_turn([Done(EndTurn)]))` whose `start` succeeds and whose session panics `"the adapter blew up mid-turn"` on its first `next_event` (after `start`, so `run_chat` is past its own start-failure close at `:3610`) | `active_runs == before`; frames end `[Failed{message contains "mid-turn"}, Ended{Cancelled}]` at seq 7 | `before + 1` |
| new `a_promoted_chat_that_panics_closes_nothing` | `a_promoted_chat_never_closes_a_run`'s shape (`:4840-4896`) with `PanicsMidTurn` registered and `runtime.attach_promoted(.., promote_addr(), promoted(..))` (`:4714`, `:4741`), `task.await` | `RUN_1` status and `finished_at` unchanged; `STEP_PLAN` status unchanged; `active_runs` unchanged; frames end `Failed`, `Ended` at seq 7 | **guard** (passes before and after): proves D4 left `Promoted` alone |

The close-before-frames order is structural (awaited before the send). The tests check the end
state; review checks the order.

### 5.3 T5 validate

```bash
cargo test -p htui --features testkit --lib agent_worker -- --test-threads=1
cargo clippy -p htui --all-targets --all-features -- -D warnings
```
Commit: `fix(mod-24): a chat that panics closes its run`.

---

## 6. T6 - docs and close-out

- `docs/htui-worker.md`: a section "What a crash costs".
  - A crash costs at most the interrupted step: it is reset and retried while `retry_limit`
    admits, or settled `done` when its document and capture both landed.
  - The K1-K5 outcomes table.
  - A cancel requested before or during a crash is applied before recovery.
  - The sweep never touches chat runs.
  - Left open: a TUI-process crash orphans its chat run (OQ-3).
- Then the write-up `docs/decisions/mod/mod-24.md`, `DECISIONS.md` and `HANDOFF.md`, per
  `.claude/rules/workflow-docs.md`.
  - Pins: store `CASES` 119; `.sqlx` count 307; the new test binary `worker_crash_pg`.
- Validate: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## 7. Build sequence

| Wave | Task | Files | Depends on |
|---|---|---|---|
| W1 | T1 | `htui-orch/src/kill_point.rs`, `htui-orch/src/lib.rs`, `htui-orch/src/engine.rs`, `htui-worker/src/runtime.rs` (`poll_once` only) | - |
| W1 | T3 | `pg/write.rs`, `store/mem.rs`, `store/traits.rs` (doc), `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`, `.sqlx/` | - |
| W1 | T5 | `htui/src/agent_worker.rs` | - |
| W2 | T2 | `htui-worker/src/runtime.rs`, `htui/src/run_worker.rs` | T1 (shares `runtime.rs`) |
| W3 | T4 | `htui-store/src/testkit.rs`, `htui/tests/worker_crash_pg.rs` | T1 (hooks), T2 (K5 is red without D3) |
| W4 | T6 | docs | all |

File-set intersections: T1 ∩ T2 = {`runtime.rs`}, and that is all. T3's two extra pin files and
the trait doc intersect nothing (A-2). Each task commits when it is done (project memory:
uncommitted subagent work dies with the session). Run `cargo fmt --all` before each commit.

**Final gates (main thread, after W4):**
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p htui-orch && cargo build -p htui --bin htui
SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features
cargo test -p htui --features testkit --test worker_crash_pg -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod24-gate.log; grep -c SIGABRT /tmp/mod24-gate.log
pgrep -af crash_child; df -h /
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

---

## 8. Hazards

- **H-1 Stack headroom (`htui-orch` `every_case_name_dispatches`, near 2 MiB).**
  - The two engine calls are sync, borrow `step`, and hold nothing across an `.await`, so the
    walk future does not grow.
  - `cancels_first` boxes `cancel_run`.
  - Gate: `--no-fail-fast -- --test-threads=1`, then grep `SIGABRT` (T1 and the final gate).
- **H-2 Orphan children.**
  - `Reaped` kills and reaps on every exit path, panics included.
  - The restarted child stops on stdin EOF, so a parent that dies closes the pipe.
  - A parked kill point exits by itself after 300 s (code 86).
  - Check `pgrep -af crash_child` after T4 and after the gate.
- **H-3 Feature unification.**
  - `cargo test` builds `htui-orch` once, with `test-support` (htui's dev-dependency). Every
    test-built artefact therefore carries the armed hook, including the `htui` binary that
    `worker_pg.rs` spawns. That spawn uses `env_clear()`, so it is safe.
  - A developer shell exporting `HTUI_TEST_KILL_POINT` would park any matching test process, so
    T4's children `env_remove` it when they have no point.
  - The production build is guarded by `cargo build -p htui --bin htui` in the gate.
- **H-4 Env read once, never written.** It is a `OnceLock`. Unit tests never touch the
  environment (B2), so one test binary's cases cannot leak a spec into each other.
- **H-5 `.sqlx` coupling.**
  - Only T3 touches `.sqlx`.
  - The hash is of the literal query text, so re-indenting the SQL changes it. A byte-identical
    query elsewhere shares an entry (project memory).
  - `prepare` needs `--all-targets --all-features`.
  - Check `git status` for exactly 1 deletion and 1 addition.
  - The prepare database is a scratch one, not the test DSN.
- **H-6 Three `CASES` pins:** `CASES` itself, `mem_store.rs:36` and `pg_conformance.rs:21`. Moving
  one alone fails the suite (A-2).
- **H-7 The keyring fake is process-wide.** `run_worker`'s D3 tests and T5's tests run under
  `--test-threads=1` (project memory). `worker_crash_pg` uses no keyring.
- **H-8 Two runtimes on one box under I-1.**
  - The parent's TUI runtime only queues the run and is shut down (`exit_tui`) before the first
    child starts. It never sweeps unless asked, and the box's executor is `worker` anyway.
  - Never start a second child before the first one is reaped. Each child mints its own owner
    (`runtime.rs:1067`), and two live workers on one box would race for adoption.
- **H-9 TTL 2 s versus the heartbeat's self-fence.**
  - The refresh is TTL/3 ≈ 667 ms, and the walk fences itself after about 1.33 s without a
    successful beat (`recover.rs:52-68`).
  - A loaded gate could fence the restarted child's own walk. The run still converges, because
    the next sweep adopts it, but K4's "no attempt 2" could be disturbed.
  - If T4 flakes on that, raise `TTL_SECONDS` to 3 (one const) and note it. Do not add sleeps.
- **H-10 Child registration.**
  - `fixture_box_store` relies on the fixture box's user being the oldest `app_user` and its
    stored fingerprint being NULL or this machine's.
  - It asserts `this_box == ids::BOX`, so a `Copied` registration fails at connect, not mid-case.
  - Registration rewrites the fixture row's `os_family` and `arch`. Nothing in the walk reads them.
- **H-11 A no-commit worktree capture is `after_hash = NULL`** (A-3). Child sessions must commit
  (B8), or K4 resets. The K4 precondition assertion catches a regression here.
- **H-12 A session's `cwd` is the step directory, not the tree.** It is `<scratch>/<run>/<step>/`
  (`real.rs:202-206`), and the tree is `<cwd>/htui`. `Committing` and `StallsOnce` assert
  `<tree>/.git` exists.
- **H-13 Git identity.**
  - Reconcile's `merge --no-ff` and `Committing`'s commit need an identity.
  - The children get `GIT_AUTHOR_*`/`GIT_COMMITTER_*` explicitly.
  - The sandbox sets one already, and CI may not.
- **H-14 `--exact crash_child`** only works for a top-level fn (probe). `CHILD` is a const shared
  by the spawn and the doc. A rename or a `mod` around it silently runs 0 tests, and the parent
  then fails on "marker never appeared" with exit 0 in the log.
- **H-15 Waits.** Every wait is a bounded poll of a marker or a row, never a sleep. Every poll
  checks `try_wait`, so a child that died early fails at once with its log.
- **H-16 D3 versus the poll (B-5).**
  - `until_applied` enables its `Notified` before re-checking, so no wakeup is lost.
  - `Applying::drop` notifies after removing the id.
  - A row the poll holds is awaited, and the poll's `cancel_run` takes its run lock in the same
    poll that took the guard. So `sweep_fenced`'s `try_lock` fence either sees the lock held or
    sees a cancelled run.
- **H-17 D212 under D3.** `cancels_first` must pass `last_live`, never `LiveChats::default()`, in
  the TUI. The worker's poll always stores an empty set (`worker.rs:71`).
- **H-18 D3b and the rest of the suite.**
  - No test relies on a chat being adopted (§3.5).
  - The TUI's chat tests in `store_worker`/`agent_worker` gain a quieter sweep, so run the full
    `htui` lib under `--test-threads=1` in the gate.
- **H-19 D4 double close.** If a panic ever followed `binding.close` in `run_chat` (today only
  `frames.ended` follows it, `:3800-3801`), `finish_chat_run` would rewrite `done` as `failed`.
  This is accepted and noted in the write-up.
- **H-20 Parking a tokio worker thread.**
  - On a one-CPU box the multi-thread runtime has one worker, and the park stalls the whole child.
    That is harmless, because the marker is written before the park.
  - The parked thread holds the run lock and no std mutex.
- **H-21 `start_paused` in D3 test 2.**
  - Clear `MemFault::RefreshLease` before the sweep.
  - Write the document and commits **before** setting the fault, while the walk's lease is live.
- **H-22 `testkit` memory.** `worker_crash_pg.rs` is ungated, like `worker_pg.rs`, so it runs
  without `--features testkit`. Still validate with the plan's `--features testkit`.
- **H-23 Disk.** Each case makes worktrees under its `TempDir`, and a killed child leaves its own,
  which go with the `TempDir`. Check `df -h /` before the gate (project memory: `target/` fills
  the disk and Postgres crash-loops).

---

## Amendments to the plan

- **A-1 The `CASES` count is 118, not 116.**
  - `crates/htui-core/tests/mem_store.rs:35-37` pins `118`, `pg_conformance.rs:21` has
    `EXPECTED_CASES: usize = 118`, and the list `conformance.rs:48-166` has 118 entries (counted).
  - The plan's "116 → 117" becomes **118 → 119**, and T6's HANDOFF pin is `CASES` 119.
- **A-2 T3's file set misses three files.**
  - `crates/htui-core/tests/mem_store.rs` and `crates/htui-store/tests/pg_conformance.rs` both
    pin `CASES.len()`. Without them, T3's own validation fails.
  - The `WriteStore::adopt_runs` contract text (`traits.rs:1139-1143`, "every `running` run") must
    say graph runs only.
  - So "Not touched: the store traits" becomes "no trait **signature** changes; one doc comment
    does".
  - None of the three intersects another task's set, so the waves are unchanged.
- **A-3 K4 needs a session that commits.** The plan's fact 3 says "K4 after capture, before
  `finish_step` (→ `Finished`)", and that holds only when capture recorded an `after_hash`:
  - `GixIsolator::capture_worktree` answers `Ok(None)` for a clean tree the step committed nothing
    to (`isolate/real.rs:1042-1064`).
  - `capture_rows` stores that as `after_hash: None` (`:983-988`).
  - `recover::classify` counts a repo as captured only with `after_hash.is_some()`
    (`recover.rs:246-250`).

  With `worker_pg.rs`'s no-op `Walks` sessions, K4 would `Reset` and retry. T4's child sessions
  therefore commit one file each (B8), and K4 asserts the precondition. The same fact makes K1's
  "landed" check meaningful, because the trees now carry commits.
- **A-4 A session's cwd is not its worktree.**
  - Under `worktree` isolation the session `cwd` is `<scratch>/<run>/<step>/`, the common parent
    (`real.rs:202-206`), and the repo's tree is `<cwd>/<repo name>`.
  - The plan's K1 says "a file the session wrote into its worktree". That is
    `<cwd>/htui/stray.txt`, and T4's parts say so explicitly.
- **A-5 T5 copies the whole `ChatRunSpec`, not just `run_id`.** `close_run` → `finish_chat_run`
  takes the run **and** step ids (`agent_worker.rs:4096-4103`, `traits.rs:592-598`). The spec is
  `Clone` (`model/run.rs:285-286`). The site is still `:1983-1987`.
- **A-6 D3, specified more tightly.** This does not contradict the plan; it makes "every pending
  cancel of a run with a free lease" exact.
  - `pending_commands` also returns this owner's runs and terminal runs (`pg/relay.rs:445-452`).
    D3 applies rows whose run is `running` and is not walked by this process; dead walks of this
    process are included.
  - Parked and terminal runs are left to the poll. The sweep never adopts them, so no recovery
    races them.
  - D3 needs two small `Shared` fields that the plan did not name: `last_live` (B5, D212) and the
    `applied` `Notify` that makes "awaiting the poll's own `cancel_run`" exact (B4).
  - There is still no new store method and no `.sqlx` change.
