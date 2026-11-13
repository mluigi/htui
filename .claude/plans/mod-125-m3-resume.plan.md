# Plan: MOD-125 M3 — Resume through saved sessions

**Source PRD**: `.claude/prds/mod-125-stop-and-resume-runs.prd.md`
**Selected Milestone**: 3 — Resume through saved sessions
**Complexity**: Large (three store verbs and a forward over five `WriteStore` implementors, one new `Isolator`
method, a resume path through the engine's walk for every step role, the session opening threaded into
`drive_once`, the runtime's poll, a cancel way out for every stopped run; no migration)
**Status**: fact-checked 2026-11-12 (six read-only agents: three grounding readers, three fact-checkers over the
first draft; falsified claims amended below), **confirmed by the maintainer 2026-11-13**. Routed plan (PRD continuing) by
`/handoff-run MOD-125`; ultracode recommended for implement; all stages inherit (Opus 5.5). Sandbox run
`hr/MOD-125`. **Complete 2026-11-13:** T1-T7 landed, review gate (rust-reviewer, two area instances) and one
fix round (F1 orch, F2 the rest; briefs beside T1-T7) applied; the `pending_resumes` index is deferred to M4's `0031`.

## Summary

M1 made `stopped` a resumable state and M2 settles a stopped workspace's runs there, trees, branch and session
banner kept. Nothing moves a stopped run back. M3 makes the resume request (deleting the `workspace_stop` row,
the existing `resume_workspace`) reach the box that walked each run: its command poll lists its stopped runs whose
workspace is no longer stopped, retakes each run's lease with a `stopped → running` CAS, and walks it. A stopped
agent step reopens its saved session on the same row and tree (the chat-promotion path: continuing recorder, the
redo turn recorded as a follow-up, `opening = resumed`), falling back to a fresh session on the same row with
`resume_failed` and the handoff prompt. Checks rerun because the step's walk runs on past the session. Every
stopped run also gets a cancel, including one that executes on no box (the demo's `RUN_8`). No production caller
issues a stop or a resume until M4's verbs; M3 is exercised through the store verbs in tests.

## Decisions

- **D1 — The resume request is the absence of the `workspace_stop` row; no new row.** `resume_workspace`
  (`traits.rs:1691`, both stores) already deletes it. A run that is `stopped`, has an `executing_box_id` and whose
  workspace has no `workspace_stop` row is owed a resume, durably: a box that is offline resumes it when it comes
  back. In production a run reaches `stopped` only through `stop_run` (`mem.rs:6791`, `pg/write.rs:7258`) or the
  sweep's settlement of a stopped workspace (`mem.rs:6709`, `pg/write.rs:6861`), and both keep
  `executing_box_id` (`pg/write.rs:7265,6870`). `settle_stop` does not re-read the gate, so a workspace resumed
  between `pending_stops` and the settle leaves a stopped run with no row: D1 resumes it at the next poll, which
  is the intent. The demo's `RUN_8` (stopped, no box, no row, `fixtures.rs:1741-1747`) is never listed: it can
  only be cancelled (D6). **Tests must reach `stopped` through `stop_workspace` + a poll or `stop_run`, never a
  bare `transition_run`**, or a poll will resume the run under them. No migration.
  - Rejected: a per-run `resume_requested` marker (a migration, and a second record to keep in step with the gate).
- **D2 — Store verbs** (on `WriteStore`, forwarded by `WorkerStore`; conformance cases on both stores; every
  implementor updated: MemStore, PgStore, `Writer`, `UsageSpy`, `SpyStore`; `WorkerStore` forwards on MemStore,
  PgStore and `Writer`):
  - `pending_resumes(box) -> Vec<RunId>`: `status = 'stopped' AND executing_box_id = box AND NOT EXISTS
    workspace_stop` for its `workspace_id`. No index serves it (`idx_run_lease` is partial on
    `queued|running`, `0003:50-51`); a scan is accepted, as `claim_run`'s box + `stopped` count already does
    (`pg/write.rs:6605`). An index would need a migration.
  - `resume_run(run, box, owner, ttl) -> Result<bool>` (`take_lease`'s shape; there is no `Lease` type — blueprint): one CAS `stopped → running` that takes the lease.
    Refused (`false`) when the run is not `stopped`, executes on another box, its workspace is stopped again, or a
    cancel of it is pending (the hold-off `stop_run` already has). **Pg order, as `claim_run` (`:6495-6506`),
    READ COMMITTED:** the run `FOR NO KEY UPDATE` (check status and box), then `workspace_stop FOR SHARE`
    (refuse if present), then the `run_command` cancel check, then the update. A stop committed after the gate
    read leaves a `running` run leased to `owner`, which `pending_stops` lists and M2 stops again. Steps are not
    touched (the engine moves them, D4).
  - `cancel_stopped_run(run, box: Option<BoxId>, at) -> Result<bool>` (the caller's clock, as `finish_run`): one CAS `stopped → cancelled` with `finished_at`;
    every step with `!status.is_terminal()` (`stopped`, `pending`, `awaiting_approval`, `failed`) `→ cancelled`
    (all legal, `run.rs:192-219`; `cancel_leased`'s rule, `engine.rs:2684-2692`); pending follow-ups refused
    `FOLLOW_UP_RUN_CANCELLED` and their `follow_up_window` rows closed **in the same transaction** (the store's
    `close_dropped_follow_ups` only writes for a lease owner, `relay.rs:1107-1141`, `mem.rs:10059-10076`, and the
    lease stays empty); a pending cancel row of the run resolved applied. `Some(b)` requires
    `executing_box_id = b`, `None` requires it `NULL`. Pg order: steps `FOR NO KEY UPDATE`, run, then
    `run_command` and `follow_up_window` (`stop_run`'s order, `:7224-7237`).
  - `request_cancel` accepts a stopped run (retire its refusal: `mem.rs:9697-9699`, `relay.rs:457-459`, doc
    `traits.rs:2299`), and `pending_commands`' box branch lists a cancel row of a `stopped` run on the box
    (`mem.rs:9734-9761`, `relay.rs:571-576`). The R1 hold-off (a pending cancel makes `stop_run` answer `false`)
    is kept. The sentence helper `cancel_of_a_stopped_run` (`traits.rs:3763`) stays until T5 drops its last
    users (`runtime.rs:27`, `run_worker.rs:4688`).
  - `WorkerStore::record_opening` forward (core `worker.rs:737`, htui-store `worker.rs:117,540`): the engine is
    bound to `WorkerStore` (`engine.rs:442,532`), which lacks it.
  - `select_fanout` (L-7, a backstop: the human path needs a parked run, `command.rs:929-964`, and D4 resumes
    stopped siblings first): losers gain `stopped` (`→ superseded`, legal `run.rs:207`); a stopped judge goes to
    **`superseded`**, not `done` (`Stopped → Done` is illegal, `run.rs:205-208`). Mem `mem.rs:7498-7519`, Pg
    `pg/write.rs:7743-7771`.
- **D3 — Engine entry `Engine::resume_stopped(run) -> Result<Option<Rest>>` (the rest frame is published as `resumed` does), the `claim` shape (`engine.rs:1366`).**
  `let sent = Instant::now()`; `resume_run(run, box, owner, ttl)`; on a lease,
  `walk_leased(run, self.lease_until(sent), self.run_to_rest(run))` (heartbeat and `release_after_walk`, as
  `claim` and `Engine::resume :4564`). A refused CAS writes nothing and answers `None`. One `info!` line
  `run {run} resumed`. Without a resume path a `running` run whose open step is `stopped` rests forever and is
  adopted as a crash in a loop (`pass :4250`, `recover_run :3437`), so D4 is required, not optional.
- **D4 — Stopped steps resume on their own row; three branches by what the row holds.** `stopped → running`
  first (legal, `run.rs:1765`); the row and attempt stay the same, so no retry budget is spent.
  1. **No session rows** (a forced drop before the session's recorder opened): re-walk from stage 1 on the same
     row; the tree is still at base, so `prepare` and the integrate pre-merge are safe to repeat (`isolate.rs:261-269`,
     a merged tip is `Contained`). The entries admit `Stopped` beside `Pending`: `walk_step` (`:5098`),
     `admit_planner` (`:6448`), `run_candidate` (`:6137`), `run_part` (`:6862`, after its `cancel_read`).
  2. **Rows but no `session_started` banner** (a graceful stop during prepare, pre-merge or prompt: the prompt row
     is written before `drive_once` reads the cancel, `:9178`) **or an agent without session caps:** a fresh
     session with the fallback text (D7) on a continuing recorder. Opening not recorded.
  3. **A banner and `DriverCaps.resume`:** reopen the session (D5).
  Branches 2 and 3 **never** call `prepare`, the pre-merge, `set_step_prompt`, `upsert_step_tree` or
  `record_commits(before)` again (`prepare` resets a moved or dirty worktree to its base, `isolate/real.rs:829-856`):
  `cwd` and `extra_dirs` are rebuilt from the step's `step_trees` with `chat_dirs` (as `Engine::opening`,
  `:2190`), and `Isolator::reacquire` (new, D5a) retakes any guard `settle_stop` released. After the session the
  step's walk runs on (verify writes a new `command_run`, capture writes a newer document that `output_of` reads,
  `finish_step` once), so checks rerun. The deadline restarts from the resume. A stop after `Done` costs one extra
  agent turn.
  - **Plain, integrate, planner** go through a new `Cursor::Resume(step)` from `node_state` (`walk.rs:98-104`)
    in a `running` run, scheduled by `plan_pass` instead of blocking (`schedule.rs:96-108`), with arms in
    `drive_cursor` (`:4310`), `schedule::candidate` (`schedule.rs:212-224`: weight, touches, `admits` for a planner),
    rule (e)'s input-wait list (`:175`, never holds a `Resume`) and `position_of` (`status.rs:667-676`).
    A stopped **planner** (it never has part rows: `admit_plan` writes them with the planner's `done` in one
    transaction, `:6469`) goes `stopped → running` and `admit_parts`, as recovery does (`:3448-3453`).
  - **Parts and candidates** resume inside `drive_parts` (`:6665-6671`) and `drive_group` (`:5838`), whose member
    filters admit `Stopped` beside `Pending`, so pending siblings run alongside and the round-end reset of the
    primary checkout still happens; `group_cursor` (`status.rs:736-749`) maps a stopped candidate to `Fan`
    (blueprint: "like a running one" would rest forever), and `parts_state` rule 1 (`walk.rs:~127-138`) stops
    resting the node on a stopped part so rule 5 yields `Cursor::Parts`; under `shared_serialized` stopped members
    run to completion before pending siblings start (a sibling's `prepare` resets the shared checkout), and `select_stage` (`:7247-7265`) admits a
    stopped judge beside a pending one.
  - **The judge** re-runs fresh on its own row, both calls, on a continuing recorder, no resume: its two calls
    are two `drive_once`s on one recorder (`judge_calls :7994`) and the latest banner may be call 1's. The
    verdict is read from the documents the re-run wrote, never an earlier call's (pinned by a test).
- **D5 — The opening, threaded into `drive_once` (`:9160`, the one spawn point; callers `session :8933`,
  `candidate_live :6290`, `part_live :7025`, `judge_calls :8016`).** A new
  `enum SessionOpening { Fresh, Continue { text }, Resume { session_ref, text, fallback } }`:
  - `Fresh` is today's path (`open_recorder :8967`: `Recorder::new` + `with_fence` + `with_run_cap` +
    `record_prompt_digesting`; `resume: None`).
  - `Continue` and `Resume` use `Recorder::continuing(.., tail)` (`record.rs:596`; `tail` = the step's
    `step_events`, as `agent_worker.rs:4692-4701`) with the same fence and run cap, and record the text with
    `record_follow_up` (`:823`), never `record_prompt*`.
  - `Resume` sets `SessionSpec.resume = Some(session_ref)`; on start `record_opening(Resumed)`. When `start`
    fails and `falls_back(&err)` (`Transport | Closed`): `record_opening(ResumeFailed)`, the `resume_failed`
    notice, then a second start with `resume: None` and `fallback` as the follow-up (Q4). Any other error fails
    the step as a first run's start error does.
  - Decided with `promote::opening_kind(caps_for(agent), events)` (`promote.rs:71`; `caps_for` through
    `self.parts.graphs.agent(..)`, `:2196`); `step_events(..).unwrap_or_default()` is authoritative on the walk's
    store. `Engine::opening` itself is not called (it needs a tree error path and the chat's texts).
  - `falls_back`, `resume_failed_notice` and `record_opening` move from `crates/htui/src/agent_worker.rs`
    (`:5453`, `:5468`, `:5481`) to `htui_orch::promote`; `record_opening` becomes generic over the store (it
    takes `&Writer` today; htui-orch never depends on htui-store). `run_chat` calls them there. The docs of
    `StepOpening` (`run.rs:257-268`) and `StepStatus::Stopped` (`run.rs:169-170`, "superseded by a fresh rerun")
    are widened to the same-row fallback.
- **D5a — `Isolator::reacquire(run, step, trees) -> IsolatorFuture<()>`.** Takes the guards `prepare` would hold
  for those trees (only `shared_serialized` takes one, `isolate/real.rs:684-686`; `local` has none — blueprint) and never resets, creates or labels; refuses
  a `shared_serialized` checkout whose `HEAD` is not the step's (another walk moved it) so the step fails rather
  than working on someone else's tree. `worktree`/`copy` hold no guard: `Ok`. Implementors: `isolate/real.rs`,
  htui-orch `fake.rs` (records the call), and any test isolator in `tests/gix_isolator.rs`.
- **D6 — A way out for every stopped run: cancel.** `cancel_enabled` (`command.rs:1073`) admits `Stopped`; its
  other caller, htui-worker `views.rs` `verdicts` (`:355`), then enables the cancel action (tests `:1657`,
  `:2538`). The engine's `cancel_run` (`:2626`) gets a `Stopped` arm beside `Queued` (`:2629`) that picks by
  `executing_box_id`: none → `cancel_stopped_run(None)`; this box → `cancel_stopped_run(Some(box))` then
  `cleanup_walk` (`:8274`, lease-less, as `cancel_queued :2651-2670`); another box → an "on another box" answer.
  No `close_finished_follow_ups` (D2 does it). Runtime: the rowless path's re-read guard (`runtime.rs:3225-3232`)
  lets `Stopped` through, `lost_to_a_claim` (`:2972-2983`) counts the engine's "another box" answer as lost so the
  durable path writes the row (its box check `:3260` answers `requested`), and the poll's refused arm
  (`:3311-3323`) and import (`:27`) go. A cancel and a resume race on the one `stopped` CAS, and `resume_run` holds
  off while a cancel is pending. A cancel stays allowed in a stopped workspace (M2 D7).
  - A stopped run whose box is gone cannot be resumed (its CLI sessions live there, PRD risk); M4 reports it as
    "cannot resume: cancel it". Moving it to another box is out of scope.
- **D7 — The fallback text is `handoff_opening` (`engine.rs:2243-2322`)** with a stop reason ("the step was
  stopped and its session could not be reopened"): the phase's task (`phase_spec`, non-strict), the handoff
  template, the transcript summary, the diff so far, the reason. It is what promotion's fallback sends, made for
  a fresh agent on a tree that already holds work. No prompt text is stored (`set_step_prompt` keeps the digest
  and trim, `traits.rs:771-777`; the recorder's prompt row is scrubbed) and re-running `assemble_prompt` would
  not be byte-identical. The resume text is `STOP_RESUME_OPENING` ("this step was stopped and has been resumed;
  your last turn may have been cut off; continue the task, redoing any unfinished part of it"), in `promote.rs`.
- **D8 — The runtime resumes in the command poll.** `poll_once` (`runtime.rs:3340`) runs `pending_resumes(box)`
  after `pending_stops`, **only when `role.executes(executor)`** (as `sweep_once`, `:2211`: the listing is per box,
  so a TUI and a worker on one box must not both act), and spawns `resume_walk(run)` per row, deduped by a
  run-keyed set with a drop guard (the `stopping`/`StopInFlight` shape, `:672-710`; `applying` is keyed by command
  row). `resume_walk` mirrors `resumed` (`:2730-2780`) minus adoption's `backoff_due`, `dead_walks.mark` and
  shutdown lease release: `walks.child(run)` before locking (so `stop_walk` and `cancel_run` reach it through
  `is_live`/`preempt_gracefully`), `lock_unless_cancelled`, `Kit::read`, `walked(&walk, engine.resume_stopped(run))`,
  `engine.abandoned(run)` on `None`/`Cancelled`. The sweep never adopts a stopped run (`adopt_runs` selects
  `running`, `mem.rs:6684`), and `resume_run` sets `running` and the lease in one CAS.
- **D9 — Test matrix (ANA-33 §9).** Orch conformance, stop (through `stop_run`) then resume: a plain step
  (opening `resumed`, one follow-up row, same row, attempt and tree, no second `prepare`), a failed resume
  (`resume_failed` row, fresh session, step done), no caps (fresh, no opening), no rows (re-walked), rows but no
  banner (fresh on the continuing recorder), the planner, a part with a queued sibling (both run, round-end reset
  kept), the pre-merge (integrate re-walked), integrate (resumed), a `shared_serialized` candidate (reacquired,
  not reset) and the judge (re-run, verdict from the re-run); cancels of a stopped run on its box and of a
  box-less one. The fake driver already records the spec (`fake.rs:94-118,197`), so tests assert the recorded
  `resume`; its banner uses `spec.resume` when set, and it gains a scripted start-failure queue (only a private
  wrapper exists, `agent_worker.rs:~6060-6200`). The **subgraph row waits for MOD-143** (only the
  `StepRole::Subgraph` string exists, `run.rs:1099`).
- **D10 — Live probe of the saved session (host).** `#[ignore]` cases: `crates/htui-agent/tests/cli_live.rs`
  (the harness interrupts with `Run::signal` `:406` and kills with `kill_tree` `:357`; case 3 already interrupts a
  long turn, case 2 resumes) — a SIGINT-interrupted and a killed turn, then `--resume` with the redo text,
  asserting the session opens and keeps the first turn; and an ACP twin in `tests/isolation_live.rs` beside
  `acp_isolated_on_resume` (`:1312`). Owed on the host with a scratch `CLAUDE_CONFIG_DIR` (memory); M3 ships
  without its result and the fallback covers a session that does not open.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Durable request poll | `pending_stops` + `poll_once` (`runtime.rs:3340-3427`), `StopInFlight` (`:672-710`) | box/owner-scoped listing, one spawned task per row, run-keyed dedupe with a drop guard |
| Walk task | `resumed` (`runtime.rs:2730-2780`) | `walks.child` → lock → `walked(engine…)` → `abandoned` |
| Engine walk on a fresh lease | `claim` (`engine.rs:1366`), `Engine::resume` (`:4554-4564`) | `sent`, store CAS with ttl, `walk_leased(lease_until(sent), run_to_rest)` |
| Resume with fallback | `run_chat` (`agent_worker.rs:4714-4812`) | start with `resume` → on `falls_back` record `ResumeFailed` + notice, start fresh |
| Continuing recorder | promotion's `Recorder::continuing` + `record_follow_up` (`agent_worker.rs:4692-4701`) | no second prompt row; seq and turn continue |
| Cancel without a lease | `cancel_queued` (`engine.rs:2651-2670`) | store CAS then `cleanup_walk` |
| Store CAS order | `claim_run` (`pg/write.rs:6495-6506`), `stop_run` (`:7205-7271`) | run lock, then `workspace_stop FOR SHARE`; steps before run |
| Errors | `StoreError::Constraint` sentences in `traits.rs` | sentence helpers |
| Store tests | M2's stop cases (`conformance.rs:270-287`), `a_cancel_racing_a_stop_waits_for_it_and_is_refused` (`pg_criteria.rs:8031`) | conformance on both stores; Pg race twins in `pg_criteria` |
| Orch tests | `stop_case` dispatcher (`htui-orch/src/conformance.rs:16926`), `settled_stopped :16950`, `settle_stop_fake`/`resume_fake` (`engine.rs:10542,10594`) | one dispatcher arm per matrix row |
| Runtime tests | `the_command_poll_stops_a_live_walk_gracefully` (`run_worker.rs:4138`), `the_store_loop_applies_a_workspace_stop_on_its_own` (`:4723`), `worker_pg.rs` `a_stop_from_another_box_settles` (`:1075`, `STOP_BOUND :1062`) | paused time without a real child, real time with one |

## Tasks

Every task leaves `cargo check --workspace --all-targets --all-features` green. Order:
**(T1 ∥ T2 ∥ T3a ∥ T6) → T3 → T4 → T5 → T7.** The blueprint `.claude/plans/mod-125-m3-resume.blueprint.md`
(architect, 2026-11-13) carries each task's signatures and seams; its cross-task contract and the task's section are
part of that task's brief. Orch `CASES` 234 → 244 (T3) → 247 (T4); store 290 → 295 (T1). T1, T2, T3a and T6 run in separate worktrees on named branches off
`hr/MOD-125` (file sets disjoint, fact-checked; T3a split out of T3 on 2026-11-13 to keep T3's brief under ~300 lines). Between T1 and T5 the runtime resolves a listed stopped-run cancel
as refused (`runtime.rs:3311-3323`); T5 removes that arm.

### T1: Store — `pending_resumes`, `resume_run`, `cancel_stopped_run`, cancel listing, `select_fanout`, forward
- **Action** (tests first): conformance cases (`CASES` entry + `run_case` arm each; pins 290 → 295 in
  `mem_store.rs:37` and `pg_conformance.rs:63`, and its message `:70`):
  `pending_resumes_lists_the_boxs_stopped_runs_once_the_workspace_resumes` (not while stopped; not another
  box's; not a box-less run), `resume_run_is_a_cas_that_takes_the_lease` (refused: not stopped, other box,
  re-stopped, cancel pending), `a_stop_after_a_resume_leaves_a_leased_running_run_pending_stops_lists`,
  `cancel_stopped_run_settles_run_and_steps_cancelled` (box and box-less; a failed step too; follow-ups refused
  `FOLLOW_UP_RUN_CANCELLED`; refused on a running run), `cancel_and_resume_of_one_stopped_run_race_on_one_cas`;
  the R1 case `a_stopped_run_takes_no_cancel_and_a_pending_cancel_holds_off_its_stop` (`conformance.rs:8881`)
  becomes `a_stopped_run_takes_a_cancel_the_box_lists_and_a_pending_cancel_holds_off_its_stop` (half 2 kept);
  `select_fanout_is_one_transaction` (`:13032`) and the Mem unit test (`mem.rs:16190`) extended with a stopped
  loser and judge. `pg_criteria`: `a_cancel_racing_a_stop_waits_for_it_and_is_refused` (`:8031`) reworked to the
  new answer; a Pg race twin of resume vs stop. Then D2 on every implementor. `cargo sqlx prepare` against a
  migrated scratch DB (memory).
- **Files**: `crates/htui-core/src/store/{traits,worker,mem,conformance}.rs`, `crates/htui-core/tests/mem_store.rs`,
  `crates/htui-store/src/{pg/write.rs, pg/relay.rs, worker.rs, writer.rs}`, `crates/htui-store/.sqlx/`,
  `crates/htui-store/tests/{pg_conformance,pg_criteria}.rs`, `crates/htui-agent/src/conformance.rs`,
  `crates/htui-agent/tests/recorder.rs`.
- **Validate**: `cargo test -p htui-core --all-features --test mem_store`; `cargo test -p htui-core --all-features --lib store::mem`;
  `cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1`.

### T2: Resume helpers and the fake driver
- **Action** (tests first): move `falls_back`, `resume_failed_notice`, `record_opening` (generic over the store)
  into `htui_orch::promote` with `STOP_RESUME_OPENING` and the stop reason (D5, D7); `run_chat` calls them there
  (its tests unchanged and green). The agent fake's banner uses `spec.resume` when set (module rule `fake.rs:51-52`
  updated; stale comment `crates/htui/tests/chat.rs:1279`), and it gains a scripted start-failure queue reachable
  from htui-orch conformance (fake unit tests). `StepOpening` and `StepStatus::Stopped` docs widened.
- **Files**: `crates/htui-orch/src/promote.rs`, `crates/htui/src/agent_worker.rs`, `crates/htui-agent/src/fake.rs`,
  `crates/htui-core/src/model/run.rs` (docs only), `crates/htui/tests/chat.rs` (comment only).
- **Validate**: `cargo test -p htui-agent --all-features --lib fake`; `cargo test -p htui-orch --all-features --lib promote`;
  `cargo test -p htui --all-features --bin htui agent_worker`.

### T3: Engine plumbing — resume entry, opening, reacquire, plain/integrate/planner, cancel (after T1, T2)
- **Action** (tests first, orch conformance; `Orchestrate` gains `resume_stopped` with a fake shim beside
  `resume_fake`): D9's plain, failed-resume, no-caps, no-rows, no-banner, planner, pre-merge, integrate and both
  cancel rows. Then D3; `Cursor::Resume` with its arms; `SessionOpening` through all four `drive_once` callers
  (`Fresh` everywhere but the new paths, compile-forced); `walk_live_step` split into a prepare-and-prompt half
  and a session-onward half that takes existing trees, calling T3a's `Isolator::reacquire`; D6's engine half
  and `views.rs`; D7.
- **Files**: `crates/htui-orch/src/{engine,walk,schedule,status,command,conformance,fake}.rs`,
  `crates/htui-orch/tests/fake_conformance.rs` (pin 234 → N
  at `conformance.rs:17139`, its message `:17231`, `fake_conformance.rs:18` and `:79`),
  `crates/htui-worker/src/views.rs`.
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast` (grep SIGABRT; memory: stack headroom);
  `cargo test -p htui-worker --all-features --lib views`.

### T3a: `Isolator::reacquire` (D5a; independent, wave 1)
- **Action** (tests first): the trait method, the real isolator (retake the `shared_serialized` slot guard, check
  `HEAD` against the step's base and foreign `htui/*` labels, refuse `moved_while_stopped` and release; `Ok` for
  `local`, `worktree`, `copy`), the gix label reader, the fake isolator's `Reacquire` call record, `reacquires()`
  and `refuse_reacquire(reason)` queue, and `StallAfterReconcile`'s forward. Real-isolator tests over a scratch
  repo (moved by a foreign label → refused and released; at base or own commits → guard held, `capture` releases).
- **Files**: `crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/{real,git}.rs`,
  `crates/htui-orch/src/fake.rs` (the fake isolator only), `crates/htui-orch/tests/gix_isolator.rs`.
- **Validate**: `cargo test -p htui-orch --all-features --lib isolate`; `cargo test -p htui-orch --all-features --test gix_isolator`.

### T4: Engine — parts, candidates, judge (after T3, same files)
- **Action** (tests first): D9's part-with-queued-sibling, `shared_serialized` candidate and judge rows; the
  `drive_parts`/`drive_group` member filters, `run_part`/`run_candidate` entries, `group_cursor`, `select_stage`,
  `part_live`/`candidate_live` openings with `reacquire`, the judge's fresh re-run.
- **Files**: `crates/htui-orch/src/{engine,status,walk,conformance}.rs`, `crates/htui-orch/tests/fake_conformance.rs`.
- **Validate**: as T3.

### T5: Runtime — resume in the poll, the cancel way out (after T4)
- **Action** (tests first; every stop through `stop_workspace` + a poll or `stop_run`, never a bare transition):
  `crates/htui/src/run_worker.rs` (paused time, beside M2's poll tests `:4109-4881`):
  `the_command_poll_resumes_a_stopped_run_once_its_workspace_resumes`, `a_resume_is_not_applied_twice`,
  `a_stopped_workspace_is_not_resumed`, `a_resumed_walk_can_be_stopped_again`,
  `only_the_executing_role_resumes_on_a_shared_box`, `the_store_loop_resumes_a_stopped_run_on_its_own`,
  `a_stopped_run_is_cancelled_on_its_box_and_a_boxless_one_at_once`, `a_stopped_run_on_another_box_gets_a_row`;
  retire `a_polled_cancel_of_a_stopped_run_is_resolved_refused` (`:4665`) and, in `runtime.rs`'s test module,
  `a_cancel_of_a_stopped_run_is_refused_and_writes_no_row` (`:5247`); `crates/htui/tests/worker_pg.rs` (real time):
  `a_stopped_worker_run_resumes_and_finishes` (graceful and forced, beside `:1075`);
  `crates/htui/tests/hierarchy.rs` (`:880-894`) and `hierarchy_pg.rs` (`:290-303`): cancel `RUN_8` through the
  verb, then `graphics` deletes. Then D8, D6's runtime half, and drop `cancel_of_a_stopped_run` and its imports.
- **Files**: `crates/htui-worker/src/runtime.rs`, `crates/htui/src/run_worker.rs`,
  `crates/htui/tests/{worker_pg,hierarchy,hierarchy_pg}.rs`, `crates/htui-core/src/store/traits.rs` (the helper
  removal only).
- **Validate**: `cargo test -p htui-worker --all-features --no-fail-fast`;
  `cargo test -p htui --all-features -- --test-threads=1`.

### T6: Live probes (independent; run on the host)
- **Action**: D10's `#[ignore]` cases. Compiles in the sandbox; the maintainer runs them on the host.
- **Files**: `crates/htui-agent/tests/{cli_live,isolation_live}.rs`.
- **Validate**: `cargo test -p htui-agent --all-features --test cli_live --test isolation_live --no-run`.

### T7: Docs (after T5)
- **Action**: `docs/ANA-2.md` run rows (`:707-711`; `:711` `stopped → failed` "resume impossible") and step rows
  (`:768-773`; `:770` `stopped → running` gains "or a fresh session on the same row", `:771` `stopped → superseded`
  "fresh rerun inserted" becomes `select_fanout`'s and recovery's only) amended under
  `*Amended by MOD-125 M3, <date>:*`; the MOD-24 narrowing (`docs/ANA-2.md:784-788`, record
  `docs/decisions/mod/mod-24.md:4-7`, cited `docs/ANA-38.md:110-112`) gets the resume mechanism; the PRD's open
  questions (session id source = the banner, D5; the `--resume` redo, D5/D7; the amendment wording, this task)
  ticked. `CONCEPTS.md:66-67` ("can always be resumed") is left for M4, which words the box-gone case.
- **Files**: `docs/ANA-2.md`, `docs/decisions/mod/mod-24.md`, `.claude/prds/mod-125-stop-and-resume-runs.prd.md`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
bash scripts/gate.sh                     # grep the log for SIGABRT
cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1
cargo sqlx prepare --check --workspace   # against a migrated scratch DB
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A killed CLI/ACP session does not reopen | Medium | Fresh session on the same row with `resume_failed` and the handoff text (Q4); D10 probes it on the host |
| The resumed agent redoes finished work or misses the interrupted turn | Medium | The redo text asks it to continue and redo unfinished parts; verify reruns |
| A resume touches a tree another walk moved (`shared_serialized`) | Low | `reacquire` refuses a checkout whose `HEAD` is not the step's; the step fails, never resets |
| A stop, a resume and a cancel race | Medium | One `stopped` CAS each; `claim_run`'s lock order; cancel hold-off; conformance and `pg_criteria` race cases |
| The judge's re-run reads an earlier call's verdict | Medium | Verdict read from the re-run's documents; a test pins it |
| A test stops a run with a bare transition and a poll resumes it | Medium | D1's rule in every brief; T5 retires the two existing ones |
| A run whose box is gone stays stopped | Low | Cancel always works (D6); M4 says so |
| Engine walks near the 2 MiB debug stack | Low | `--no-fail-fast`, grep SIGABRT (memory) |
| Live-Qdrant timeouts in the sandbox gate | High | Known; owed on host |

## Acceptance

- [ ] All tasks complete
- [ ] Every new conformance case passes on MemStore and PgStore
- [ ] A worker walk and a TUI walk each resume a graceful and a forced stop to `done`, on the same step rows
- [ ] Every stopped run, `RUN_8` included, can be cancelled
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- Note: no production caller issues `stop_workspace` or `resume_workspace` until M4 (`writer.rs:1133` and tests
  only); M4 also owes migration **`0031`** (`0030` is MOD-150's `link_kinds`), not the `0030` the HANDOFF names.

## Verified claims

Six read-only agents at `a05deeac`, 2026-11-12: three grounding readers (drivers and promotion, engine re-walk
paths, store and demo), then three fact-checkers over the first draft (store, engine, runtime/docs/independence).
Falsified claims were amended above before CONFIRM.

| Claim | Verdict | Evidence |
|---|---|---|
| Only `stop_run` and the sweep reach `stopped` in production | TRUE (+ `RUN_8` seed, + a settle after the gate lifts) → D1 | `mem.rs:6791,6709`; `pg/write.rs:7258,6861`; `fixtures.rs:1741-1747`; `engine.rs:2737-2757` |
| `executing_box_id` survives a stop | TRUE | `pg/write.rs:7265,6870`; `mem.rs:6790-6793` |
| An index serves `pending_resumes` | FALSE → D2 (scan accepted) | `0003_orchestration.sql:50-51` partial; `0019:37` needs a workspace |
| M2 D9 locks `workspace_stop` first; locking it serialises stop and resume | FALSE → D2 | `pg/write.rs:10868-10873` locks nothing; a share lock on an absent row is vacuous; `claim_run`'s order `:6495-6506` used |
| `cancel_stopped_run` cancels stopped/pending/awaiting steps | PARTLY → D2 | `failed` is non-terminal (`run.rs:192-219`); `!is_terminal()` as `cancel_leased` |
| Follow-ups refused `FOLLOW_UP_SESSION_ENDED`, by the engine after the cancel | FALSE → D2, D6 | `FOLLOW_UP_RUN_CANCELLED` (`engine.rs:2704`); owner-only store write (`relay.rs:1107-1141`) → same transaction |
| Listing a stopped run's cancel keeps the R1 hold-off | TRUE (+ resume hold-off added) | `stop_run` reads `run_command` directly; `relay.rs:571-576` |
| `select_fanout` lists and legality | TRUE (judge → superseded); reachable only as a backstop | `run.rs:205-208`; `command.rs:929-964` |
| T1 lists every implementor | TRUE for traits; FALSE for files → T1 | `pg_criteria.rs:8031` added; `WorkerStore::record_opening` forward added |
| CASES pins 290; latest migration 0030 | TRUE | `mem_store.rs:37`; `pg_conformance.rs:63,70`; `0030_link_kinds.sql` |
| D3 shape: run lock + `leased_window` + `run_to_rest` | FALSE → D3 | `leased_window :2999` has no heartbeat; `walk_leased` as `claim :1366` |
| A stopped step is read in `node_state`, `parts_state`, `group_cursor`, `plan_pass` | TRUE (+ three more arms) → D4 | `walk.rs:98-104,129-138`; `status.rs:736-749,667-676`; `schedule.rs:96-108,175,212-224` |
| A running run with only a stopped step hangs without D4 | TRUE | `pass :4250`; `recover_run :3437` |
| `Isolator::prepare` is safe to repeat on a step that did work | FALSE → D4, D5a | `isolate/real.rs:829-856` resets moved/dirty worktrees; `:676-736` in-place |
| The pre-merge is safe to repeat | TRUE before the session only → D4 branch 1 | `isolate.rs:261-269` |
| "No rows" = stopped before the session | PARTLY → D4 three branches | prompt row written before the cancel read (`:9178`) |
| Re-running verify/capture after a resume is harmless | TRUE (no `set_step_prompt`/`record_commits` again) | `views.rs:986`; `output_of :9709`; `verify :5620` |
| `drive_once` is the one spawn point with four callers | TRUE | `:8933,6290,7025,8016`; `resume: None :9289` |
| The judge's session can be resumed | FALSE → D4 | `judge_calls :7994` two calls, one recorder |
| The stored prompt can be re-sent | FALSE → D7 | `traits.rs:771-777` digest only; recorder row scrubbed; `handoff_opening :2243` used |
| `Engine::opening` fits a walk-time call | PARTLY → D5 | needs a tree, chat texts; `opening_kind :71` + `caps_for :2196` used directly |
| `cancel_enabled` change is engine-local | FALSE → D6, T3 | `htui-worker/src/views.rs:355` |
| `close_finished_follow_ups` after the lease-less cancel | FALSE → D6 | owner-only (`mem.rs:10066-10069`) |
| Entry moves in `candidate_live`/`part_live` | FALSE → D4 | `run_candidate :6137`, `run_part :6862` |
| Resume dedupe by `applying`; walk handed to `Walks`; TUI hands back via `tails` | FALSE → D8 | `applying` keyed by command row (`:624`); `walks.child` before lock (`:832`); `role.executes` (`:2211`) |
| The runtime cancels a box-less stopped run itself | PARTLY → D6 | choice in the engine's `cancel_run`; re-read guard `:3225-3232`; `lost_to_a_claim :2972` |
| Helpers move to htui-orch unchanged | PARTLY → D5, D2 | `record_opening` takes `&Writer`; engine bound to `WorkerStore` (`engine.rs:442,532`) |
| The fake ignores `spec.resume`; a scripted failure exists | PARTLY / FALSE → D9, T2 | spec recorded (`fake.rs:94-118,197`); banner id `fake-{step}` `:282`; no failure queue |
| ACP live harness for a resume twin in `acp_live.rs` | FALSE → D10 | `isolation_live.rs:1312` `acp_isolated_on_resume` |
| Runtime poll tests in `auto_queue.rs` | FALSE → T5 | `run_worker.rs:4109-4881`; `runtime.rs` test module `:4034-6747` |
| ANA-2 M1 rows match the same-row fallback | FALSE → T7 | `docs/ANA-2.md:711,770,771` |
| MOD-24's "no mid-step resume" location | `mod-24.md:4-7`, narrowed `ANA-2.md:784-788` | T7 |
| T1 ∥ T2 ∥ T6 independent | TRUE after the forward moves to T1 and `pg_criteria.rs` joins T1 | T1 = core store, htui-store, htui-agent spies; T2 = `promote.rs`, `agent_worker.rs`, agent `fake.rs`, `run.rs` docs, `chat.rs` comment; T6 = `cli_live.rs`, `isolation_live.rs`: disjoint |
| T3/T4 split workable | TRUE if T3 owns the cursor, the opening and the `walk_live_step` split | engine fact-check §11 |
