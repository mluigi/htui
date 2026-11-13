# Blueprint: MOD-125 M3, resuming through saved sessions

Companion to `mod-125-m3-resume.plan.md` (D1-D10, D5a settled). This file holds the seams the plan
leaves to the implementer. Each delta names its task; fold it into that task's brief. Line numbers
are at `edaab6b9` (`~` = within a few lines). **PLAN ERROR** marks where the plan is wrong against
the source.

## Cross-task contract (read by every brief)

- Stopped runs always come from `stop_run` (directly, or through `settle_stop` or a poll), never from
  a bare `transition_run` (D1). A stopped run with `workspace_id IS NULL` has no gate row, so
  `pending_resumes` lists it and the next poll resumes it (T1). A test that needs a run to stay
  stopped must stop it inside a workspace that has a `workspace_stop` row.
- Pins: orch `CASES` 234 → 244 (T3, ten rows) → 247 (T4, three); store 290 → 295 (five new).
- **Known red between T3 and T5:** `runtime.rs:5247` `a_cancel_of_a_stopped_run_is_refused_and_writes_no_row`
  and `run_worker.rs:4665` `a_polled_cancel_of_a_stopped_run_is_resolved_refused`. After T3 the
  engine cancels a stopped run on this box or on no box, so no refusal happens. T3's validation
  runs neither test, and T5 retires both. Every implementer leaves `cargo check` green.

## T1: store

**PLAN ERROR (D2):** the store has no `Lease` type. `take_lease` (`traits.rs:1624`) answers
`Result<bool>`, and `claim_run` answers `Claim` (`model/overlap.rs:131`). `resume_run` takes
`take_lease`'s shape. **Delta:** `cancel_stopped_run` also takes `at`. Every lease-less terminal
write takes the caller's clock (`transition_run(.., at)`, `finish_run(.., now)`), so the engine's
`TestClock` pins `finished_at`.

```rust
// WriteStore (traits.rs, beside stop_run :1705); WorkerStore forwards each one by path.
async fn pending_resumes(&self, box_id: BoxId) -> Result<Vec<RunId>>;            // (queued_at, id) order
async fn resume_run(&self, run: RunId, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<bool>;
async fn cancel_stopped_run(&self, run: RunId, box_id: Option<BoxId>, at: DateTime<Utc>) -> Result<bool>;
// WorkerStore only (worker.rs, the WriteStore method already exists at traits.rs:2025):
fn record_opening(&self, step: StepId, opening: StepOpening) -> impl Future<Output = Result<()>> + Send;
```
- `resume_run`:
  - `Constraint` for a ttl out of range comes before any read (`lease_ttl_micros`, as `take_lease`).
  - `NotFound { "run" }` comes first among the reads. `Ok(false)` writes nothing.
  - On success it sets `running`, `lease_owner`, `lease_box_id` and `lease_expires_at = store now + ttl`.
    No step moves. A NULL `workspace_id` passes the gate.
- `pending_resumes`: `status='stopped' AND executing_box_id=$box AND NOT EXISTS (workspace_stop
  WHERE workspace_id = run.workspace_id)`. A NULL workspace matches nothing, so the run is listed.
- `cancel_stopped_run`: `Ok(false)` = not `stopped`, or another box; `NotFound` is read first.
- `WorkerStore` doc header (`worker.rs:15-26`): 71 → 75 methods.
  - Forwards: core `worker.rs:737` (MemStore), htui-store `worker.rs:117` (PgStore) and `:540`
    (Writer), all by UFCS.
  - `UsageSpy` (`htui-agent/src/conformance.rs:781`) delegates. `SpyStore` (`tests/recorder.rs:469`)
    does what it does for `stop_run`.
- Cases:
  - New: `pending_resumes_lists_the_boxs_stopped_runs_once_the_workspace_resumes`,
    `resume_run_is_a_cas_that_takes_the_lease`,
    `a_stop_after_a_resume_leaves_a_leased_running_run_pending_stops_lists`,
    `cancel_stopped_run_settles_run_and_steps_cancelled`,
    `cancel_and_resume_of_one_stopped_run_race_on_one_cas`.
  - Renamed: `store/conformance.rs:8881` becomes
    `a_stopped_run_takes_a_cancel_the_box_lists_and_a_pending_cancel_holds_off_its_stop`, in
    `CASES`, `run_case`, and the pin messages (`mem_store.rs:37`, `pg_conformance.rs:63,70`).

## T2: promote helpers and the fake driver

```rust
// htui_orch::promote
pub const STOP_RESUME_OPENING: &str = "This step was stopped and has been resumed. Your last turn \
    may have been cut off: continue the task, redoing any unfinished part of it.";
pub const STOP_REASON: &str = "the step was stopped and its session could not be reopened";
pub const fn falls_back(err: &htui_agent::DriverError) -> bool;                 // verbatim, agent_worker.rs:5453
pub fn resume_failed_notice(session_ref: &AgentSessionRef, reason: &str) -> OtherEvent; // verbatim, :5468
pub async fn record_opening<S: htui_core::store::WriteStore + ?Sized>(store: &S, step: StepId, opening: StepOpening);
```
- **Decision:** `record_opening` is bound on `WriteStore` (named by path), so T2 does not depend on
  T1. `Writer: WriteStore` (`writer.rs:375`), and `run_chat` calls `record_opening(&writer, ..)`. It
  `warn!`s and swallows the error, as `:5481` does. Delete the three private copies.
- The engine never calls it. Its module sees only `WorkerStore` (E0034, `worker.rs:8`). T3 adds
  `Engine::note_opening(step, opening)` over T1's `WorkerStore::record_opening`, with the same `warn!`.
- Fake driver (`htui-agent/src/fake.rs`):
  - `FakeDriver` gains `start_failures: Mutex<VecDeque<DriverError>>` and
    `pub fn failing_starts(self, errors: impl IntoIterator<Item = DriverError>) -> Self`.
  - `start` records the spec first. It then pops one queued failure and answers it **without**
    taking the script, so the next `start` on the **same** driver plays it. The engine calls `start`
    twice on one driver (`engine.rs:~9300`).
  - `FakeAdapter::fail_next_starts(&self, errors)` holds an `Arc<Mutex<VecDeque<_>>>` that `build`
    drains into its driver. `DriverError` is `Clone`.
  - The banner id is `spec.resume` when set, else `fake-{step}`. Update module rule 1 and the
    "deterministic" paragraph (`:1-52`), and the stale comment at `crates/htui/tests/chat.rs:1279`.

## T3: engine plumbing

```rust
/// What `drive_once` starts with (D5).
pub(crate) enum SessionOpening<'t> {
    Fresh(&'t str),                       // today: the caller already recorded the text
    Continue { text: String },            // D4 branch 2: drive_once records it with record_follow_up
    Resume { session_ref: AgentSessionRef, text: String, fallback: Option<String> },
}
/// What `open_recorder` opens.
pub(crate) enum RecorderStart<'p> { Prompt(&'p AssembledPrompt), Tail(Vec<SessionEvent>) }
```
- **Delta (D5 shape):**
  - `Fresh` carries its start text and replaces `drive_once`'s `text: &str` (`:9167`), so the start
    text has one source.
  - `fallback` is `Option`, as in `OpeningPath::Resume` (`:2210`): the handoff is best effort, and
    `None` fails the step as `run_chat`'s `(Err, None)` arm does.
- `drive_once(.., key, opening: &SessionOpening<'_>, cwd, ..)`. Its four callers (`:8933`, `:6290`,
  `:7025`, `:8016`) pass `Fresh`. `spec.resume = Some(session_ref)` for `Resume` (`:9289`).
- Start, resume and fallback: take the start result **before** `remasked_driver`'s `?`.
  - `Resume` `Ok`: `note_opening(Resumed)`, then `record_follow_up(text)`.
  - `Resume` `Err(e)`, when `falls_back(&e)` and a `fallback` exists:
    1. `note_opening(ResumeFailed)`.
    2. `record_notice(resume_failed_notice(..))`; a failed write is logged and the walk goes on.
    3. A second start with `resume: None` and the fallback text.
    4. On `Ok`, `record_follow_up(fallback)`.
  - `Continue` `Ok`: `record_follow_up(text)`, with no opening recorded.
  - Every follow-up is written after `start` and before the pump.
- `open_recorder(run, step, start: RecorderStart)`: `Prompt` is today's path. `Tail(events)` is
  `Recorder::continuing(.., None, events)` with the same fence and run cap and no `record_prompt*`.
  `session()` replaces `prompt` with `start: RecorderStart, opening: SessionOpening`.
- **`walk_live_step` split:**
  - **Prepare-and-prompt half.** `walk_live_step` keeps its name and covers `:5143-5280`: secrets,
    integrate stage, prepare, trees, premerge, `before`, prompt, `set_step_prompt`. It ends in
    `walk_live_session(.., LiveSession { trees, cwd, extra_dirs, integrate, live,
    start: Prompt(&prompt), opening: Fresh(&prompt.text) })`.
  - **Session-onward half.** `LiveSession<'p>` holds `{ trees: Vec<RunStepTree>, cwd: PathBuf,
    extra_dirs: Vec<PathBuf>, integrate: Option<IntegrateStage>, live: bool, start, opening }`.
    New `walk_live_session` covers persona → session → after_done → verify → capture →
    `record_commits(after)` → `unmerged_keys` → `output_of` → settle → `finish_step` → gate.
    `integrate_stage` (`:5420`) is read-only, so a resume recomputes it.
- **D4 routing.** `walk_step` (`:5084`) moves `from = step.status`. `Stopped` calls
  `resume_live_step(.., now)` inside the same `fail_hard` arm; `now` restarts the deadline.
  `resume_live_step` reads `events = step_events(step).unwrap_or_default()`:
  - Empty → `walk_live_step` (branch 1).
  - Otherwise, `reopen_trees` → `stopped_opening`, recompute `integrate` and `live`, then
    `walk_live_session(.., start: Tail(events), opening)`.
- Helpers, which T4 reuses:
  - `reopen_trees(run, step, phase) -> (Vec<RunStepTree>, PathBuf, Vec<PathBuf>)`: `step_trees`,
    then `isolator.reacquire`, then `chat_dirs(&trees, &repos)`.
  - **Delta (D4):** `chat_dirs` is `None` for an empty scope. Conformance runs one
    (`conformance.rs:2926`). In that case only, use `prepare(run, step, &[], phase.isolation,
    None).cwd`. With no tree there is nothing to reset (`real.rs:~720`, fake `fake.rs:~889`).
  - `stopped_opening(..)`:
    - caps come from `candidate_of`, then `self.parts.graphs.agent`, then `registry::caps_for`;
    - `promote::opening_kind(caps, &events)` decides: `Resume(r)` gives
      `Resume { r, STOP_RESUME_OPENING, fallback: handoff.ok() }`, and `Handoff` gives
      `Continue { text: handoff? }`.
  - **Delta (D7):** `handoff_opening` (`:2241`) gains `reason: Option<String>`. `Engine::opening`
    passes `None`; the resume path passes `Some(STOP_REASON)` (`run.failure` is NULL while stopped).
    The no-caps test edits the **agent row** that `caps_for` reads, not `FakeOrchestrator::with_caps`.
- **`Cursor::Resume(StepId)`** (status.rs `:383`). Arms:
  - `node_state` (walk.rs `:98`): `Stopped` → `Live(Resume)`.
  - **Missed site:** the test-only parity scan `cursor_by_position` (`status.rs:578`) needs the same
    arm, or the oracle diverges.
  - `position_of`: reads the row, as `Run` does. `resumable_park`: `false`.
  - `schedule::candidate`: weight and touches as `Run`; `admits` also matches `Resume(id)` on a
    planner row. Rule (e) never lists `Resume`. Rule 1 keeps `Stopped`.
  - `drive_cursor`: `Cursor::Run(s) | Cursor::Resume(s)` share the arm. `admit_planner` (`:6443`)
    and `walk_step` move `from = step.status`.
- **Delta (D3):** `resume_stopped` answers `Option<Rest>`, not `bool`, so the runtime can publish
  `FrameKind::Rested`, as `resumed` does (`runtime.rs:~2770`).
```rust
pub async fn resume_stopped(&self, run: RunId) -> Result<Option<Rest>, EngineError> {
    let sent = tokio::time::Instant::now();
    let ttl = self.lease_times().ttl;
    if !self.parts.store.resume_run(run, self.parts.box_id, self.parts.owner, ttl).await? { return Ok(None); }
    tracing::info!(%run, "run {run} resumed");
    Ok(Some(self.walk_leased(run, self.lease_until(sent), self.run_to_rest(run)).await?))
}
```
- Fakes:
  - `resume_stopped_fake` goes beside `resume_fake` (`engine.rs:10594`). Add `Orchestrate::resume_stopped`
    (trait `conformance.rs:~276`, impl block `:279`).
  - Add `refuse_resume(phase, attempt, slot: Option<(i32,u32)>, reason)` on `Orchestrate` and on
    `FakeOrchestrator`. It stores `ScriptKey → Vec<DriverError>`; `scripted_driver`'s `None` arm
    applies `failing_starts([Transport(reason)])`. `ScriptedStep` is unchanged, since it is built
    as a literal elsewhere.
  - Rows go in a `resume_case` dispatcher beside `stop_case` (`:16926`). Stop each run with M2's
    preempt + `settle_stop`, then call `orch.resume_stopped(run)`.
- **`cancel_run` (`:2626`)** routes `RunStatus::Stopped` to a new `cancel_stopped(&row)`:
  - It reads `position` from `resting`, then picks by `executing_box_id`:
    - `None`: `cancel_stopped_run(None, now)`; no cleanup, there is no tree of ours.
    - This box: `cancel_stopped_run(Some(box), now)`, then `cleanup_walk`.
    - Another box: `Err(EngineError::RunStatus { run, status: Stopped, expected: "stopped on this box
      or on none" })`. Reusing the variant means T5 only flips `lost_to_a_claim`.
  - A `false` CAS means a resume or another cancel won. Re-read the run, run `cancel_enabled`, then
    `cancel_leased`, as `cancel_from_queued` does (`:~2636`).
  - `cancel_enabled` (`command.rs:1073`) drops `&& status != Stopped`. Update its doc, its test
    (`:2048-2058`), and `views.rs:355` with its tests `:1657,:2538`.
- **`Isolator::reacquire` (D5a).** **PLAN ERROR:** `local` takes no guard. Only `shared_serialized`
  calls `acquire` (`real.rs:684-686`, fake `:866-879`), so `local`, `worktree` and `copy` answer `Ok`.
```rust
fn reacquire<'a>(&'a self, run: RunId, step: StepId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()>;
```
  - Real implementation, in order:
    1. Collect the rows with `mode == SharedSerialized` and `acquire(run, step, &checkouts)`. The
       guards land in `held[(run, step)]` and `capture` releases them, as today.
    2. Check each row: (a) `git::is_ancestor(path, &base_ref, &head)`, where equal passes; (b) no
       `refs/heads/htui/<s>` label with `s != step` lies in `base_ref..HEAD`.
    3. A failure calls `release_step`, then returns
       `Refused(moved_while_stopped(path, head, base))`. The step fails and is never reset.
  - **Scope delta:** check (b) needs a gix label reader, so T3's files gain
    `crates/htui-orch/src/isolate/git.rs`.
  - Fake: `IsolatorCall::Reacquire(StepId)`. For a shared row it calls `release_step`, then
    `serial.lock_owned()`, then `held.insert`. Add `reacquires()` and a `refuse_reacquire(reason)`
    queue. `StallAfterReconcile` (`tests/gix_isolator.rs:2017`) forwards to `inner`.

## T4: parts, candidates, judge

- **PLAN ERROR (D4, `group_cursor`):** treating a stopped candidate "like a running one" returns
  `Rest`, and `drive_group` is reached only through `Cursor::Fan`, so the walk rests forever. Keep
  `Running` first, then `short || any(Pending) || any(Stopped) → Fan` (`status.rs:~743-746`). A
  stopped judge over settled candidates falls through to `Select`, which `select_stage` admits.
- **Missed in plan:** `parts_state` rule 1 (`walk.rs:~127-138`) rests the node on a stopped part, so
  `drive_parts` is never reached. Drop `Stopped` there so rule 5 yields `Cursor::Parts`, and update
  the rule doc. **Scope delta:** T4's files gain `crates/htui-orch/src/walk.rs`.
- **Member filters** in `drive_group` (`:5838`) and `drive_parts` (`:6665`):
  - Select `Pending | Stopped`, with stopped members first.
  - Under `SharedSerialized`, drive the stopped members to completion **before** any pending one
    starts. A pending sibling's slot `prepare` resets the shared checkout (and refuses on dirt).
    Without the ordering, tokio's FIFO guard and the futures' await counts would decide who goes
    first.
  - Other modes keep one `join_all` or stream.
- Every hard-coded `from: Pending` in the member paths becomes `step.status`: `run_candidate :6137`,
  `run_part :6862` (after its `cancel_read`), `drive_parts`' secrets loop `:~6688`, and
  `fail_group_before_a_token`.
- Group prompt: assembled from the first **pending** member, or the first member when none is
  pending. Only branch-1 members (no rows) use it.
- **`candidate_live` / `part_live`** reopen when `stage.step.status == Stopped` (the row before the
  move) and `events` is non-empty:
  - `reopen_trees`, then `*trees = Some(rows)` **at once**, so `release_trees` frees the guard on error.
  - Skip prepare, `upsert_step_tree`, `before` and `set_step_prompt`.
  - `open_recorder(Tail(events))`, then `drive_once(&stopped_opening(..))`.
  - `started_at = now` after the reacquire.
- **Judge:**
  - `select_stage` (`:~7256`): `Pending | Stopped` → `run_judge`, which moves `from = judge.status`
    (`:~7505`).
  - `judge_sessions` (`:7938`):
    - With non-empty `events`, open `Tail(events)` and pass `rerun = true`.
    - Prepare and `set_step_prompt` repeat as today; the judge has no tree, and its prompts are
      rebuilt from the same rows.
    - Before the calls, read `stale = output_of(item, &jp, judge.id)?.map(|d| d.id)`.
  - `judge_calls` (`:7994`) gains `rerun: bool, stale: Option<DocumentId>`:
    - Call 0 records its text with `record_follow_up` when `rerun`.
    - The acceptance guard becomes `Some(d) if Some(d.id) != stale && !documents.iter().any(..)`.
      Without `stale`, a re-run call 0 that writes nothing reads the stopped attempt's verdict as
      new (`output_of :9709` is newest-by-version).
    - Pinning test: re-run call 0 scripted with no output answers `MissingDocument { call: 0 }`.

## T5: runtime

- `Shared` gains `resuming: StdMutex<HashSet<RunId>>`.
  - `resume_turn(self: &Arc<Self>, run) -> Option<ResumeInFlight<P>>` inserts the run and drops the
    set's lock before it builds the guard (`applying`'s rule, `runtime.rs:~617-627`).
  - `ResumeInFlight { shared, run }` removes the run on drop, in the shape of `StopInFlight`
    (`:698-710`).
- `poll_once` (`:3340`), after the stops loop:
  1. Read the executor fresh, as `sweep_once` does (`:~2203-2212`: `host.box_row` →
     `Executor::of`). Return unless `ctx.shared.role.executes(&executor)`.
  2. Call `WorkerStore::pending_resumes(&writer, box_id)`.
  3. Skip a run that is in `cancelling` or `walks.is_live`, or whose `resume_turn` answers `None`.
  4. Spawn `resume_walk` on `ctx.unaddressed("resume_run")` with `spawn_supervised`. A read error is
     `warn!` and returns.
- `resume_walk` is `resumed` (`:2730`) minus `backoff_due`, `dead_walks.mark` and the shutdown
  release.
```rust
async fn resume_walk<H: WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId, _in_flight: ResumeInFlight<P>) {
    ctx.tag(run).await;
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else { return };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await { Ok(k) => k, Err(m) => return ctx.refuse(m) };
    let driver = |c: &SnapshotCandidate, _k: &SessionKey<'_>| kit.driver(c);
    let engine = kit.engine(&driver);
    match walked(&walk, engine.resume_stopped(run)).await {
        None | Some(Err(EngineError::Cancelled { .. })) => { engine.abandoned(run).await; ctx.refuse(PREEMPTED.to_owned()); }
        Some(Ok(None)) => {}                                    // CAS refused: nothing written
        Some(Ok(Some(rest))) => ctx.publish(Some(run), FrameKind::Rested(rest)),
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}
```
- **D6 runtime edits:**
  - `lost_to_a_claim` (`:2972`): `!matches!(*status, RunStatus::Queued) && !status.is_terminal()`, so
    `Stopped` counts as lost (only the other-box answer raises it).
  - The rowless pre-check (`:~3206-3209`) **keeps** `Stopped`, so a box-less or own-box stopped run
    is cancelled at once.
  - Only the post-`lost` re-read match (`:~3222-3229`) drops `&& row.status != Stopped`. The durable
    row is written, and `:3260` answers `requested`.
  - Delete the refused arm (`:3311-3323`), the import (`:27`), and the helper (`traits.rs:3763`). The
    store-side uses go in T1.

## Hidden coupling (not in the plan)

- T3 → T4: `reopen_trees`, `stopped_opening`, `note_opening`, `RecorderStart` and `SessionOpening`
  must be reusable `Engine` methods and types, not inlined into `resume_live_step`.
- T3 and T4 both append to `CASES` and `resume_case`. T4 moves the pin 244 → 247 in all four places
  (`conformance.rs:17139,:17231`, `fake_conformance.rs:18,:79`).
- T1 → T3: the `at` on `cancel_stopped_run`.
- T3 → T5: `resume_stopped`'s `Option<Rest>`, and the other-box `RunStatus { Stopped }` answer.
  Until T5, that answer gets today's rowless refusal, which is correct in the interim.
- T2 → T3: the orch harness reaches the failure queue through `scripted_driver` (`orch fake.rs:2933`,
  one `FakeDriver` per session key), not `FakeAdapter`; both build on `FakeDriver::failing_starts`.
