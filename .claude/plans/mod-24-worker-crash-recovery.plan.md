# MOD-24 - Crash recovery of runs under the headless worker (plan)

Base: `hr/MOD-24` @ f3c5b8b (main after the MOD-42 merge). Sandbox run (TOOL-7, `HR_SANDBOX=1`).
Requirements: `R-HIS-1`, `R-ORCH-11`. No migration expected (the next one stays `0012`).
**Status:** fact-checked (see Verified claims); **CONFIRMED by the maintainer 2026-10-01** with
OQ-1..OQ-4 as recommended (all (a)).

## Routing verdict

```
Item:      MOD-24 - Crash recovery of runs under the headless worker
Path:      plan
Criteria:  C1 ✗  C2 ✗ (a test-support kill-point hook, no public surface)  C3 ✗ (mostly settled)  C4 ✗   → 0 fired
Ultracode: not needed
Reasoning: the 2026-09-25 rescope leaves reset-and-retry (built in MOD-4 M6) plus its proof under the
           worker (MOD-41): kill points, a kill/restart end-to-end test, and MOD-53's chat-panic leftover.
```

Accepted by the maintainer 2026-10-01.

## What the code shows

1. **The step's stage order** (`crates/htui-orch/src/engine.rs` `walk_live_step`, `:3374-3540`):
   `prepare` → `upsert_step_tree` → `record_commits(before)` → prompt (`set_step_prompt`) → session
   (driver start, the recorder's flushes) → `sink.after_done` (**the output document**, in a test
   through `StepAuthor`; MOD-11's `document_write` in production) → verify → `isolator.capture` →
   `record_commits(after)` → `output_of` → `finish_step` → gate.
2. **What the sweep makes of each crash point** (`recover::classify`, `crates/htui-orch/src/recover.rs:236-269`):
   a step with its output document **and** (`finished_at` set **or** every repo's `after_hash`
   captured) is `Finished` and settles `done` without a session; anything else with resettable trees is
   `Reset`: `reset_interrupted` (`engine.rs:2622-2675`) resets the trees, fails the step `interrupted`,
   notes "retrying as attempt N+1" on the item, and admits the next attempt while `retry_limit` holds.
3. **ANA-27 T11's four kill points do not all exist in that order.** "After capture and before the
   output document" is unreachable: the document precedes capture. Mapped onto the real order, the
   four points are: **K1** session started, no driver event flushed (the `prompt` row is always
   flushed before the session, `record.rs:700/709`); **K2** after a flush (a `ToolCall` followed by
   a park lands its row; there is no idle flush, `record.rs:900/1041`); **K3** after the document,
   before capture (→ reset, the document notwithstanding); **K4** after capture, before
   `finish_step` (→ `Finished`, settled through `gate::apply`, so `done` on an ungated phase). K1-K3
   must reset-and-retry, K4 must settle `done`. This is the item's "mid-step" vs "after a step's
   artefacts are written" split, made exact. K3 holds only with a non-empty `repo_scope`: with none,
   `captured` is vacuously true and K3 is `Finished` too (`recover.rs:251`, test `:970`).
4. **There is no notification to kill before.** MOD-42 ships no `LISTEN`/`NOTIFY`
   (`docs/decisions/mod/mod-42.md:89`): a `run_command` row is applied only by the executor's command
   poll (`poll_once`, `crates/htui-worker/src/runtime.rs:2451-2485`, every `COMMAND_POLL` = 1 s).
   ANA-27's fifth point therefore becomes **K5**: the worker has read the pending cancel row and dies
   before applying it. `pending_commands` (`crates/htui-store/src/pg/relay.rs:429-460`) also returns
   rows of a lapsed-lease run on this box, so the restarted worker's poll is the backstop.
5. **A restarted worker races its own cancel.** The loop (`crates/htui-worker/src/worker.rs` `run`,
   biased `select!`) fires the sweep first, then the command poll, both at start. `adopt`
   (`runtime.rs:1831-1880`) resumes every `Next::Walk` run on its own task without looking at pending
   commands (`pending_commands` is read only in `poll_once`). The dead owner's lease hides the row
   from `pending_commands` until it lapses; then the sweep (every `WorkerConfig.poll`) usually adopts
   before the 1 s command poll, recovery runs, and the resumed walk goes on. A walk that reaches `done`
   first makes the poll's `cancel_run` resolve the row `refused` (`command.rs:941`,
   `runtime.rs:2436-2440`): the user's cancel is lost to the crash. **Deterministic** when the crash
   is K4 on the last ungated phase: `sweep_fenced` finishes the run inside recovery. (OQ-2.)
   A cancel applied to a free (lapsed) run instead takes the lease and cancels every unsettled step
   without recovery, resolving `applied` (`cancel_leased` → `take_lease`, `engine.rs:1934`).
5b. **Every sweep adopts live chat runs.** `adopt_runs` has no `kind` filter (`pg/write.rs:4065-4117`,
   `mem.rs:4322-4354`), and a chat run is `running` on its box with a `NULL` lease
   (`pg/write.rs:1726-1735`). The sweep leases it, `recover_run` fails at `snapshot_of` (no graph
   snapshot), and `unrecovered` (`engine.rs:2366-2377`) warns and releases the lease. While the sweep
   holds it, the chat's own writes, all under `StepFence::Unleased` ("refused on a run whose lease
   names an owner", `traits.rs:2326-2333`; `pg/write.rs:193-213`), are refused `Fenced`. It repeats
   every sweep: every TTL in the TUI, every 5 s on a worker sharing the box. (OQ-4.)
6. **A chat that panics mid-turn leaves its run open.** `answering` (`crates/htui/src/agent_worker.rs:3524-3550`)
   catches the unwind, drops the task, and sends `ChatFrame::Failed` then `Ended`, but
   `ChatBinding::close` → `close_run` → `finish_chat_run` (`:2041`, `:4096`) runs only on
   `run_chat`'s own exits (`:3610`, `:3800`). The `run(kind='chat')` row and its step stay `running`,
   counted as an active run for good.
7. **What exists to mirror.** MOD-4 M6's crash cases run in process over `MemStore`
   (`conformance.rs` `crash`, `FakeOrchestrator::stall_after_done`); `crates/htui-orch/tests/gix_isolator.rs`
   crashes over the real isolator (`StallAfterReconcile`). `crates/htui/tests/worker_pg.rs` runs
   `htui_worker::worker::run` in process over Postgres with test parts (`Parts::runtime_with`,
   `OutputAuthor`), and spawns the real binary with a `Reaped` guard. Nothing yet kills a process that
   holds a lease mid-walk. Aborting the in-process loop task would not be a crash either: `RunRuntime`
   has no `Drop`, dropped `JoinHandle`s detach the supervisors (`runtime.rs:1600`), and the walks go on
   heartbeating.

## Design

### D1. Kill points: a test-support hook where test parts cannot reach, test parts elsewhere

- **K1, K2: in the test's transport.** The child's scripted session starts, (K2: emits a flush-trigger
  event and the parent waits until its `session_event` row is in Postgres), then awaits forever. The
  parent kills it. No production code: the session is the test's own.
- **K3, K4, K5: `htui_orch::kill_point`**, a new module. `pub fn reached(point: KillPoint, at: Site)`
  is an empty `#[inline]` function unless `htui-orch`'s existing `test-support` feature is on. With it,
  it reads `HTUI_TEST_KILL_POINT` once (a point name plus the phase and attempt it fires for, so one
  step of a multi-step walk is chosen), and on a match writes a marker file named by
  `HTUI_TEST_KILL_MARK` (written and synced) and parks the calling thread forever, awaiting the
  parent's `SIGKILL`. Call sites:
  - `Documented`: `walk_live_step`, after `sink.after_done`, before verify.
  - `Captured`: `walk_live_step`, after `record_commits(after)`, before `output_of`/`finish_step`.
  - `CommandPicked`: `poll_once`, after `pending_commands` returned a row this process will apply,
    before `cancel_run` is spawned.
  The production `htui` binary never enables `test-support`, so the hook compiles to nothing there.
  Fan-out candidates (`candidate_live`) and judges get no call sites: they are not this item's walk.

### D2. The kill is a real process kill (OQ-1)

The test re-executes **its own test binary** as the worker: `std::env::current_exe()` with
`--exact <child entry> --nocapture --test-threads=1` (a top-level fn's name alone, probed) and an env
var that turns the child entry (a `#[tokio::test(flavor = "multi_thread")]` that returns at once when
the variable is unset) into a worker. The child connects a `PgStore` to the case's throwaway database
(`TestDb.url`) **as the fixture box `ids::BOX`**: `demo_db` repoints its own store there in process
and deletes the minted box (`pg/demo.rs:658`), so `PgStore::connect(url, &db.identity)` would name a
box that no longer exists; the child needs the same repoint (a small `htui_store::testkit` helper
over what `demo_db` does). It runs `htui_worker::worker::run` (the loop the binary runs) over
`RunRuntime::with_parts(..).with_author(OutputAuthor).with_role(Role::Worker)` (`worker_pg.rs:405`):
the **real `GixIsolator`** (`IsolatorConfig { repos, scratch_root, .. }`, the repo map built by the
case: the seed's repo has no `repo_box_path` and no path on disk) over a temporary git repository
whose scratch root lies outside it (the tree must survive the kill as a box's filesystem does; a
`FakeIsolator` dies with the process), a fake verifier, the scripted transport. The FEAT graph is
ungated (`ungate_feat`) so a recovered `Finished` step lands `done`; its phases default to
`Worktree` isolation with `retry_limit = 1`, which admits attempt 2 (`may_attempt(n, l) = n <= l+1`).
The parent waits for the marker (never a sleep) and `Child::kill()`s it: `SIGKILL` (exit signal 9,
probed), no destructor, no lease release, no shutdown. It then starts a second child (a new process:
`RunRuntime` mints `owner = Uuid::now_v7()` per runtime, `runtime.rs:1067`) with no kill point, which
must wait out the dead lease and recover. The TTL is shortened per case by
`UPDATE app_setting SET value = '2'::jsonb WHERE key = 'lease_ttl_seconds'` (migration `0003` seeds
`120`; there is no Postgres setter; refresh = min(60 s, TTL/3)).

### D3. Commands before recovery (OQ-2)

`adopt` first reads `pending_commands(owner, box)` and applies every pending cancel of a run with a
free (lapsed or absent) lease, awaiting the poll's own `cancel_run` for that `RunCommandId` (under the
B-5 `is_applying` guard, so the poll never applies it twice), and only then calls `sweep_fenced`. A
free run with a pending cancel is therefore cancelled without recovery (`cancel_leased` takes the
lapsed lease and cancels every unsettled step) and never walked again: `cancelled`, command
`applied`, deterministically. Applying it after `sweep_fenced` would be too late (fact 5: recovery can
finish the run itself). No new store method and no `.sqlx` change.

### D3b. The sweep adopts graph runs only (OQ-4)

`adopt_runs` gains `AND kind = 'graph'` in Postgres and the same filter in `MemStore`, with a store
conformance case ("adopt_runs never leases a chat run", both stores). A chat run is never the
engine's to recover; the filter ends the sweep's lease on live chats and its per-sweep warning. The
changed query needs a regenerated `.sqlx` entry (`cargo sqlx prepare` against a migrated scratch
database, `docs/hr-sandbox.md`).

### D4. A chat task that panics closes its own run

The chat's `answering` call gets a closing step for `ChatBinding::Fresh`: on an unwind, the run and its
step are closed `failed` through `finish_chat_run` **before** the `Failed`/`Ended` frames, so a tab that
re-reads runs on `Ended` finds it closed. `ChatBinding::Promoted` closes nothing (MOD-4 D165: the
step's status is the engine's). The shape (a cleanup future passed to `answering`, or the chat's
`Answer` carrying it) is the architect's; the other six tasks' replies are unchanged.

## Not in scope

- Continuing an interrupted step in the agent's own session (`claude --resume`, ACP `session/load`):
  needs an ANA-2 §4.9 amendment; can be raised after MOD-37 (item text).
- Re-attaching a live agent across worker and TUI (MOD-46/MOD-47).
- A chat whose **TUI process** dies (not a panic): the chat run has no lease or owner to sweep by,
  and after D3b no sweep touches it. Stated in the write-up's "Left open" (OQ-3).
- A command already queued to a dead chat stays unanswered (MOD-53 note; the tab sees `Ended`).
- Kill points inside fan-out candidates and judges (D1).

## Files

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-orch/src/kill_point.rs` | CREATE | D1 hook, no-op without `test-support` | T1 |
| `crates/htui-orch/src/lib.rs` | UPDATE | `pub mod kill_point` (always present; its body is gated) | T1 |
| `crates/htui-orch/src/engine.rs` | UPDATE | `Documented`, `Captured` call sites in `walk_live_step` | T1 |
| `crates/htui-worker/src/runtime.rs` | UPDATE | `CommandPicked` call site (T1); D3 in `adopt` (T2) | T1, T2 |
| `crates/htui/src/run_worker.rs` | UPDATE | D3's test, beside `stranded` and the pending-cancel tests (`:1342`, `:3155-3364`) | T2 |
| `crates/htui-store/src/pg/write.rs` | UPDATE | D3b: `adopt_runs` filter | T3 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | D3b: `State::adopt_runs` filter | T3 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | D3b: one case, registered in `CASES` (116 → 117) | T3 |
| `crates/htui-store/.sqlx/` | UPDATE | D3b: the changed query's entry (one removed, one added) | T3 |
| `crates/htui-store/src/testkit.rs` | UPDATE | a connect-as-fixture-box helper for the child (D2) | T4 |
| `crates/htui/tests/worker_crash_pg.rs` | CREATE | D2: K1-K5 against Postgres | T4 |
| `crates/htui/src/agent_worker.rs` | UPDATE | D4 and its tests (MOD-53 section) | T5 |
| `docs/htui-worker.md` | UPDATE | what a crash costs on a worker box | T6 |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-24.md` | UPDATE/CREATE | close-out | T6 |

Not touched: migrations, `recover.rs`, the store traits, snapshots.

## Tasks

Tests first in every task (TDD per repo convention: each test is seen failing for the right reason).

- **T1 - Kill-point hook** {`kill_point.rs`, `htui-orch/src/lib.rs`, `engine.rs`, `runtime.rs`}.
  Unit tests in `kill_point.rs`: a spec naming another point, phase or attempt does not fire; a
  matching one writes the marker (the park is exercised by T4, not in process). Docs name the gated
  items in plain text, not intra-doc links (`broken_intra_doc_links = "deny"` without the feature).
  Validate: `cargo test -p htui-orch --features test-support kill_point`; `cargo check -p htui-orch`
  and `cargo build -p htui --bin htui` (no features) compile the no-op.
- **T2 - D3, commands before recovery** {`runtime.rs`, `run_worker.rs` tests}; after T1 (shares
  `runtime.rs`). Tests: a lapsed run with a pending cancel is cancelled by the sweep, its command
  `applied`, no session started, nothing recovered; the same with the crashed step being the last
  phase's captured one (the deterministic loss of fact 5). Seen failing first.
- **T3 - D3b, graph runs only** {`pg/write.rs`, `mem.rs`, `store/conformance.rs`, `.sqlx`}.
  Conformance case: a `running` chat run with a `NULL` lease on the box is not adopted, and a graph
  run beside it is (both stores). `cargo sqlx prepare` per `docs/hr-sandbox.md` against a migrated
  scratch database; only this task touches `.sqlx`. Validate: `cargo test -p htui-core --features
  test-support conformance`, `cargo test -p htui-store --features test-support` (Postgres), and
  `SQLX_OFFLINE=true cargo check -p htui-store`.
- **T4 - The kill test** {`worker_crash_pg.rs`, `htui-store/src/testkit.rs`}; after T1, T2. Five
  cases, each kill → restart on the same box (`ids::BOX`, executor `worker`), FEAT ungated, TTL 2 s:
  - **K1** session started, no driver event: attempt 1 `failed` with `gate_note = "interrupted"`, the
    item's "retrying as attempt 2" note, attempt 2 walked, run `done`; a file the session wrote into
    its worktree before the kill is in no tree the run lands.
  - **K2** after a flush (a `ToolCall`, then the park; the parent waits for its `session_event`
    row): as K1, and attempt 1 keeps its flushed rows (`R-HIS-1`).
  - **K3** after the document (`Documented` hook): as K1 although attempt 1's document exists;
    attempt 2 writes its own and the settle reads that one.
  - **K4** after capture (`Captured` hook): the step settles `done` with no attempt 2 and no new
    `session_event` row for it; the run walks on to `done`.
  - **K5** cancel picked, not applied: a client registered as another box (`another_box`) requests the
    cancel while the child's session is parked (live lease, so the row is queued, not applied by the
    requester); the child dies at `CommandPicked`; the restarted child ends the run `cancelled`, the
    command `applied`, and the run never reaches `done`.
  Every case asserts the dead child's exit signal is 9 and the lease is released at rest. Children are
  reaped on every exit path (`worker_pg.rs`'s `Reaped`); every wait is bounded by `PATIENCE`. The file
  is ungated like `worker_pg.rs` (no `htui::testkit` use) and skips with `testkit::SKIP` without
  `HTUI_TEST_DATABASE_URL`. Validate:
  `cargo test -p htui --features testkit --test worker_crash_pg -- --test-threads=1`.
- **T5 - D4, chat panic closes its run** {`agent_worker.rs`}; **independent of T1-T4** (disjoint
  set). `ChatRunSpec`'s `run_id` is copied out before `ChatBinding::Fresh(chat)` takes it
  (`:1983-1987`). Tests: a chat whose session panics **mid-turn** closes its run and step `failed`,
  then sends `Failed` and `Ended`; the existing start-panic case
  (`a_chat_that_panics_ends_its_stream_with_failed_then_ended`, `:10336`) also asserts the closed row;
  a promoted step's chat that panics closes nothing. Validate:
  `cargo test -p htui --features testkit --lib agent_worker`.
- **T6 - Docs and close-out**: `docs/htui-worker.md` (a crash costs at most the interrupted step;
  K1-K5 outcomes; a cancel survives a crash), then HANDOFF (pins: store `CASES` 117, `.sqlx` count)
  /DECISIONS/write-up per `.claude/rules/workflow-docs.md`, validator.

**Parallelism (file-set intersection):** T1 ∩ T2 = {`runtime.rs`} → serial. T3, T5 intersect nothing
else → parallel with T1. T4 needs T1 and T2 (hooks, D3) and T3 (a chat-free sweep is not needed by
T4, so T4 may start after T2). Waves: **W1** T1 ∥ T3 ∥ T5; **W2** T2; **W3** T4; **W4** T6.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check -p htui-orch && cargo build -p htui --bin htui          # the hook's no-op build
SQLX_OFFLINE=true cargo check -p htui-store
cargo test -p htui --features testkit --test worker_crash_pg -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep the log for SIGABRT
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A parked hook thread stalls the child before the parent sees the marker | Low | probed: marker seen within ~12 ms over ~240 cycles; written and synced before the park |
| Orphaned children outlive a failing case | Medium | `Reaped` on every child; `PATIENCE` bounds every wait |
| Five cases × lease TTL are slow | Low | TTL 2 s per case; the restarted child sweeps every `CONFIG.poll` (50 ms) |
| `test-support` reaches the production binary | Low | fact-checked: `cargo tree` normal edges show only `default`; `cargo build -p htui --bin htui` in the gate |
| The child cannot be the fixture box | Medium | T4's testkit helper mirrors `demo_db`'s repoint; asserted by the child's first beat on `ids::BOX` |
| D3 double-applies with the poll | Low | the B-5 `is_applying` guard; T2's test runs both arms |
| `.sqlx` drift from the D3b query | Low | only T3 touches `.sqlx`; `SQLX_OFFLINE=true cargo check` in the gate |
| `htui-orch`'s 2 MiB stack headroom | Low | the hook is a sync call holding nothing across an `.await` |

## Open questions (maintainer)

- **OQ-1 - How the worker is killed.** (a) **Recommended:** re-exec the test binary as the worker
  (D2): a real `SIGKILL` of a process holding a lease, with test parts. (b) Spawn the real `htui
  worker` binary: needs a test-parts mode inside the shipped binary, and production parts need a real
  agent. (c) In process, abort the loop task: not a crash, the walks keep heartbeating (fact 7).
- **OQ-2 - D3, a cancel survives a crash.** (a) **Recommended:** the adopter applies pending cancels
  of free runs before recovery. (b) Leave it; K5 then accepts `cancelled` or `refused`, documenting
  the loss instead of fixing it.
- **OQ-3 - A chat orphaned by a TUI crash.** (a) **Recommended:** out of scope, named in "Left open".
  (b) File a MOD item now.
- **OQ-4 - D3b, the sweep stops leasing chat runs.** (a) **Recommended:** filter `adopt_runs` to
  `kind = 'graph'` here (two stores, one conformance case, one `.sqlx` entry). (b) File it separately
  and leave this item to the worker tests.

## Verified claims

Step 3.5, 2026-10-01: two verifiers over the tree plus a toolchain probe (`/tmp/mod24-probe`, rustc
1.98.1, tokio 1.53.1); the session re-read every falsified or surprising claim.

| Claim | Verdict | Evidence |
|---|---|---|
| `walk_live_step` order: after_done → verify → capture → record_commits(after) → output_of → finish_step → gate | true | `engine.rs:3469`, `3474`, `3487`, `3490`, `3492`, `3506`, `3528` |
| ANA-27's "after capture and before the output document" kill point exists | **false** → amended (fact 3) | the document precedes capture |
| `classify`: doc + (finished or captured) → Finished, else Reset | partly → amended | `recover.rs:251`, `:261`; an empty `repo_scope` makes `captured` vacuous (test `:970`); Finished lands through `gate::apply`, so `done` only when ungated |
| `reset_interrupted` fails the step `interrupted`, notes, admits attempt+1 | true | `engine.rs:2622-2675`; `INTERRUPTED = "interrupted"` `:104`; `interrupt_step` sets `failed` `pg/write.rs:4465`; note only with an item |
| FEAT phases' `retry_limit` ≥ 2 | **false** → amended | all `retry_limit = 1`; `may_attempt(2, 1)` is true (`status.rs:27`), so attempt 2 is still admitted |
| The seed's repo is a real tree | **false** → amended (D2) | no `repo_box_path`, no path on disk; the case builds the repo map |
| No `LISTEN`/`NOTIFY` for `run_command` | true | no `pg_notify`/`NOTIFY`/`LISTEN`/`PgListener` under `crates/`; `mod-42.md:89` |
| `pending_commands` returns the owner's runs and free runs on the box | true | `pg/relay.rs:429-460`, `mem.rs:6039` |
| Worker loop: biased, sweep then commands, both at start; `COMMAND_POLL` 1 s outside `WorkerConfig` | true | `worker.rs:18`, `:64-75` |
| `adopt` resumes walks without reading pending commands | true | `runtime.rs:1831-1880`; only caller of `pending_commands` is `poll_once` `:2466` |
| A pre-crash cancel can lose to recovery | true (deterministic for K4 on the last phase) | `cancel_run` `runtime.rs:2297-2445`; `command.rs:941` |
| A cancel on a free run takes the lease and cancels without recovery | true | `cancel_leased` → `take_lease` `engine.rs:1934` |
| Aborting the loop task is a crash | **false** | no `Drop` on `RunRuntime`; detached supervisors `runtime.rs:1600` |
| Each runtime mints its own owner | true | `runtime.rs:1067` `Uuid::now_v7()` |
| Lease TTL is settable per case | true, by raw SQL | `recover.rs:23-71` (default 120, 1 s..1 year, refresh min(60, TTL/3)); seeded by `0003:135`; no PG setter |
| `with_parts`/`with_author`/`with_role(Role::Worker)` are public; injected parts skip the isolator rebuild | true | `runtime.rs:1049`, `:1253`, `:363-365`; `worker_pg.rs:405`; `IsolatorConfig` `isolate/real.rs:222-246` |
| A second `PgStore` from `db.identity` is the same box | **false** → amended (D2, T4 helper) | `demo_db` repoints to `ids::BOX` and deletes the minted box (`pg/demo.rs:658`) |
| A `ToolCall` then a park lands a `session_event` row; the prompt row precedes the session | true | `record.rs:900`, `:1041`, `:700/709`; no idle flush |
| `htui-orch/test-support` is off in the production binary, on for `htui` test targets | true | `cargo tree -e features,normal` shows only `default`; with dev edges `test-support`; `resolver = "3"` |
| A new `crates/htui/tests/*.rs` is auto-discovered; `worker_pg.rs` is ungated | true | no `[[test]]`/`autotests`; `runs_pg.rs:33` gated, `worker_pg.rs` not |
| Re-exec with `--exact <fn>` runs one test; `Child::kill()` gives signal 9 | true (probe) | 6 runs × 40 cycles |
| A sync marker-then-park inside a multi-thread tokio test is seen and killable | true (probe) | ~12 ms per cycle, ~240 cycles |
| The chat panic leaves its run open | true | `answering` `agent_worker.rs:3524-3550`; `close` only at `:3610`, `:3800`; mint at `:1894-1895` |
| The existing start-panic test checks the run row | **false** | `a_chat_that_panics_ends_its_stream_with_failed_then_ended` `:10336` checks frames only; T5 extends it |
| Nothing ever sweeps chat runs | **false** → new fact 5b, OQ-4 | `adopt_runs` has no `kind` filter; `unrecovered` leases and releases a live chat each sweep |
| `run.kind` is `graph` or `chat` | true | `0001_init.sql:451`; `model/mod.rs:265` |
| The next migration is `0012`; none is needed | true | highest `0011_permission_relay.sql` |
| Task independence | checked | file sets in the Files table; only T1 ∩ T2 = {`runtime.rs`} is non-empty |
