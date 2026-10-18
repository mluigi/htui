# Plan: MOD-125 M1 — Stopped state and workspace gate

**Source PRD**: `.claude/prds/mod-125-stop-and-resume-runs.prd.md`
**Selected Milestone**: 1 — Stopped state and workspace gate
**Complexity**: Large (a store seam over five implementors plus a migration; a mechanical fan-out of
one new status and one new run column)
**Status**: fact-checked 2026-10-18 (verified claims at the end). Awaiting CONFIRM.

## Summary

Add a resumable `stopped` status to runs and steps, record on every run the workspace it was started
from, and add a durable per-workspace stop record. While a workspace's record exists, nothing new
starts there: `claim_run` refuses with a new `Claim::WorkspaceStopped` that writes nothing, and the
crash sweep settles an abandoned run of that workspace as `stopped` instead of adopting and retrying
it. M1 ships the state, the store verbs and the gate. Nobody issues a stop yet: the executors apply
stops in M2, resume comes in M3, and the CLI verbs in M4.

## Decisions

- **D1 — Status law.** `RunStatus::Stopped` (`"stopped"`): `running -> stopped`;
  `stopped -> running | cancelled | failed`. `StepStatus::Stopped`: `running -> stopped`;
  `stopped -> running | cancelled | failed | superseded`. `running` is resume, `superseded` is M3's
  fresh-rerun fallback, and `cancelled` is a cancel of a stopped run, wired in M3. `queued`, `pending`
  and `awaiting_approval` are never moved to `stopped`:
  - a queued run waits at the claim gate (D5);
  - a parked gate consumes nothing, and M2 refuses approving it while its workspace is stopped.
- **D2 — Live, not running (revised by the fact-check).** `stopped` joins `RunStatus::is_active`
  (`queued | running | awaiting_approval | stopped`), so `is_terminal` stays `!is_active` and its pin
  test (`run.rs:1012-1025`) still holds. "Active" now reads as "live" everywhere; the doc at
  `run.rs:52-56` says so. Most callers want exactly that, and the per-site verdicts are below. A stopped
  run:
  - **holds no slot**, because the slot counts `status = 'running'` only and is unchanged;
  - **still refuses an overlapping scope**, because it keeps its trees and branch (ANA-2 §4.7,
    invariant 6). `stopped` is added by hand to the overlap list `('running','awaiting_approval')`
    (`mem.rs:5418-5424`, `pg/write.rs:4847`);
  - keeps its item at `in_progress`;
  - gets no `finished_at` (`transition_run` stamps it only on terminal moves) and is not cleanup-enabled.

  Explicit exclusions:
  - the top bar's active-run count (`active_runs`, `mem.rs:487`, and its SQL twin), so the R-TUI-18
    counts stay "running and waiting";
  - `cancel_enabled` (`command.rs:983`), because a stopped run's cancel is wired in M3, so M1 shows no
    cancel that cannot work;
  - `take_lease` (`write.rs:5053`), because a stopped run is not leasable for commands.

  Every other hand-written SQL liveness list gains `'stopped'` after a per-site check: `pg/read.rs:1246,1417,2626`,
  `pg/write.rs:982,6178,6312,8196`, `cache/read.rs:1342,1480` and `pg/relay.rs:544`. The pin test
  `pg/read.rs:3292` moves with them. Each verdict is recorded in the commit message.
- **D3 — `run.workspace_id`.** `UUID NULL REFERENCES workspace(id) ON DELETE SET NULL`, mirrored, set
  by `create_run` from `NewRun.workspace_id` (`#[serde(default)]`). The PRD's Q1 is answered by
  plumbing:
  - `Command::StartRun` gains `workspace: Option<WorkspaceId>`, and the one production construction
    (`ui/tabs/backlog/detail/runs.rs:835`) sends `Some(ctx.scope.workspace_id)`;
  - `queue_item` gains a workspace that `queue_entry.workspace_id` records, sent by `on_queue`
    (`ui/tabs/backlog/mod.rs:507`) through `StoreRequest::QueueItem { item, workspace }`;
  - `admit` reads it per entry (a `workspace_of` map beside `project_of`, `runtime.rs:2286`).

  The migration backfills every run whose project belongs to exactly one workspace. A run whose
  project belongs to none or to several stays `NULL`, and a workspace stop never touches it.
  **Chat runs keep `NULL`** (`start_chat_run` is a separate verb): a chat is driven by the person
  typing in it, so MOD-125 does not stop it.
- **D4 — `workspace_stop` record.** One row per stopped workspace: `workspace_id` PK (FK, cascade),
  `mode` `graceful | forced`, `requested_by`, `requested_at`.
  - `stop_workspace` upserts: a graceful stop over a forced one stays forced, and a forced one over a
    graceful one escalates.
  - `resume_workspace` deletes the row and answers whether one existed.
  - The row's presence is the gate; "stopping" vs "stopped" is derived in M4.
  - It is not mirrored, like `queue_batch`, and is read through the store.
  - No `WorkerHost` verb in M1; M2 adds what the runtime needs.
- **D5 — Claim gate.** `claim_run` decides in this order:
  1. `NotFound` (run, then box);
  2. `NotClaimable`;
  3. **`WorkspaceStopped { workspace }`**: right after the `NotClaimable` return (`mem.rs:5380`,
     `pg/write.rs:4744`, where Pg already holds `FOR UPDATE` on the run and box);
  4. `MissingTags`;
  5. slot;
  6. overlap.

  It writes nothing, so a stopped workspace never fails a run for its tags. `Engine::claim`
  (`engine.rs:826-830`) sorts it with the transient refusals into `ClaimRefused`. The runtime re-queues
  the run, and every sweep's claim scan retries it with no back-off (`runtime.rs:2555,1997`), which is
  how queued runs start after resume. **Accepted cost:** one `claim_run` transaction and one debug line
  per queued run per sweep while stopped. M2 may add a pre-check if it shows up. `admit` is otherwise
  unchanged: its queued count already caps `free_slots`.
- **D6 — Sweep settlement.** `adopt_runs` does not adopt a lapsed-lease `running` graph run of a
  stopped workspace. In the same transaction it moves the run and its `running` steps to `stopped`,
  clears the lease, and leaves the run out of the answer.
  - Postgres needs an explicit transaction here: today it is one CTE on the pool (`write.rs:4972-5021`).
    Keep `FOR UPDATE SKIP LOCKED`.
  - The process is dead, so this is a forced stop, and M3's resume redoes the interrupted turn.
- **D7 — `stop_run` verb for M2.** `stop_run(run, owner, at) -> bool` is a compare-and-set
  `running -> stopped`:
  - it is fenced on `lease_owner = owner`;
  - in one transaction it moves the run's `running` steps to `stopped` and releases the lease;
  - `Ok(false)` means not ours or not running, and nothing is written then.

  It is on `WriteStore` and `WorkerStore`. M2 calls it.
- **D8 — Docs.** ANA-2's run table (`docs/ANA-2.md:653-671`) and step table (`:683-710`) gain the D1
  rows in the doc's `| from | event | guard | to | who |` format, under an italic
  `*Amended by MOD-125 M1, 2026-10-18:*` paragraph. MOD-24's "no mid-step resume" is noted as
  narrowed: crash recovery is unchanged, and explicit stops resume in M3. The stale line citations in
  `run.rs:57,99` are corrected.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Status enum + law | `crates/htui-core/src/model/run.rs:33-138` | `str_enum!` variant with doc; `can_move_to` `matches!` table; law tests `run.rs:986,1000`; CHECK-list pin `model/mod.rs:296-321` |
| Additive migration | `migrations/0014_run_step_opening.sql`, `0016_auto_queue.sql` | Header naming item, forward-only (R-STO-5), mirror note, `COMMENT ON COLUMN`; CHECKs are `run_status_check` / `run_step_status_check` (verified in `pg_constraint`) |
| Mirror column | `cache_migrations/0005_run_step_opening.sql` | No CHECKs in the mirror; rebuild is automatic (`schema_version()` = max migration, `pg/mod.rs:723-726`; `cache/mod.rs:142`) |
| Claim verdict | `model/overlap.rs:127-169` | Doc'd variant; `Display` (`:172`); engine names every verdict (`engine.rs:826-830`, blueprint R-43) |
| Store verb | `WriteStore::claim_run` / `adopt_runs` (`store/traits.rs:1255-1330`) | Doc gives decision order, writes, errors; MemStore `State` fn + async wrapper (`mem.rs:5363,5505,8690,8711`); PgStore `pg/write.rs:4712,4972`; Writer `writer.rs:330` |
| Conformance | `store/conformance.rs` `CASES` (`:61`) + `run_case` (`:237`) | Generic `async fn x<S: WriteStore>(store: &S)`; run by `htui-core/tests/mem_store.rs:13` and `htui-store/tests/pg_conformance.rs:56` |
| Errors | `StoreError::Constraint` / `NotFound { entity }`; `legal_move` (`traits.rs:2211`) | Same taxonomy |

## Tasks

Every task leaves `cargo check --workspace --all-targets --all-features` green. That rule, plus the
file sets, sets the order: **T1 → T2 → (T3 ∥ T4) → done, with T5 any time.**

### T1: Migration and mirror schema
- **Action**:
  - Write `0019_stop_runs.sql`:
    - drop and re-add `run_status_check` and `run_step_status_check` with `'stopped'`;
    - add `run.workspace_id`, an index `(workspace_id, status)`, and the D3 backfill;
    - add `queue_entry.workspace_id`;
    - create `workspace_stop`;
    - add `COMMENT`s.
  - Write `cache_migrations/0006_run_workspace.sql` (`run.workspace_id`).
  - Update the hard-coded migration counts (`Pending(18)`, "eighteen") to 19.
- **Files**: `crates/htui-store/migrations/0019_stop_runs.sql`, `crates/htui-store/cache_migrations/0006_run_workspace.sql`,
  `crates/htui-store/tests/connect.rs` (`:140,156,242`), `crates/htui-store/tests/migrations.rs` (`:1461,1562,1571,1594,1747`).
- **Validate**: apply the migration to a scratch DB on `psql -h localhost -p 5439 -U postgres`; check
  that the backfill and the CHECKs are in place (`\d run`); run `cargo test -p htui-store --all-features --test migrations --test connect`.

### T2: Model, law and the compile fan-out (after T1)
- **Action**:
  - Test first: extend the law tables (`run.rs:986,1000`), `run_enums_match_check_lists` (`model/mod.rs:296`),
    and add `a_stopped_run_is_live_and_holds_no_slot` at the model level.
  - Add `Stopped` to both enums with D1's law and D2's `is_active`.
  - Add `Run.workspace_id` and `NewRun.workspace_id`, `QueueEntry.workspace_id`, `StopMode`,
    `WorkspaceStop` and `Claim::WorkspaceStopped` (with `Display`).
  - Fix every compile site, sorting `stopped` explicitly, with no catch-alls:
    - `traits.rs:2154` (`chat_step_status`: a chat is never stopped, so `None`);
    - `engine.rs:804` (the claim arm, `ClaimRefused`);
    - `engine.rs:3003` (`run_to_rest`: a stopped run rests like a parked one, no cleanup);
    - `engine.rs:4664` (`select_stage`);
    - `gate.rs:1018` (`retire_slot`);
    - `status.rs:331` (`cursor`);
    - `runs.rs:1192,1203` and `execution_graph.rs:233,245` (dim, like a parked state).
  - Add every `Run` / `NewRun` / `QueueEntry` field site: the literals in `mem.rs:2695,5330`,
    `cache/read.rs:236`, `fixtures.rs:1438,1458,1478`, `pg_criteria.rs:9953`, and the tests named by
    the compiler (`htui-agent/tests/relay.rs:133`, `htui-orch/tests/review_loop.rs:61`,
    `htui-mcp/tests/tools_backlog.rs:77`, `queue.rs:609`, `overlay/queue.rs:719`).
  - Add the `query_as!(Run, …)` SELECT/RETURNING lists (`pg/read.rs:647,1224,2605`, `pg/write.rs:4601,4975`),
    the mirror's `RUN_SELECT`, `run_of`, `RUN_COLUMNS` and `refresh_run` (`cache/read.rs:229-264`,
    `cache/refresh.rs:1103-1167`), and the `QueueEntry` reads (`pg/read.rs:2287,2395`, `pg/write.rs:7926,8001`).
  - `create_run` writes `workspace_id` in both stores.
  - Apply D2's liveness verdicts to every `is_active` / `is_terminal` caller and SQL list.
  - Run `cargo sqlx prepare` against a scratch DB migrated by T1.
- **Files**:
  - `crates/htui-core/src/{model/{run,overlap,queue,mod}.rs, model/stop.rs (new), fixtures.rs, store/traits.rs (chat_step_status only), store/mem.rs (field sites, liveness)}`
  - `crates/htui-store/src/{pg/read.rs, pg/write.rs, pg/relay.rs, cache/read.rs, cache/refresh.rs}`
  - `crates/htui-store/.sqlx/`
  - `crates/htui-store/tests/pg_criteria.rs`
  - `crates/htui-orch/src/{engine,gate,status,command}.rs` (match arms, `cancel_enabled`)
  - `crates/htui-orch/tests/review_loop.rs`
  - `crates/htui-agent/tests/relay.rs`
  - `crates/htui-mcp/tests/tools_backlog.rs`
  - `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `runs/execution_graph.rs`, `ui/overlay/queue.rs` (test)
- **Validate**: `cargo test -p htui-core --all-features`;
  `cargo check --workspace --all-targets --all-features`; and the Pg suite on a scratch DB.

### T3: Store verbs, gate and settlement (after T2; parallel with T4 in its own worktree)
- **Action**:
  - Conformance cases first (`CASES` entry + `run_case` arm each):
    - `stop_workspace_upserts_and_escalates_never_downgrades`
    - `resume_workspace_deletes_and_reports`
    - `claim_run_refuses_a_stopped_workspace_and_writes_nothing`
    - `claim_run_checks_the_stop_before_tags`
    - `claim_run_admits_after_resume`
    - `a_run_without_a_workspace_is_never_gated`
    - `adopt_runs_settles_a_stopped_workspaces_abandoned_run_as_stopped`
    - `stop_run_is_a_fenced_cas_on_running`
    - `a_stopped_run_holds_no_slot_but_still_overlaps`
    - `create_run_records_its_workspace`
  - Then add the verbs with full docs to `WriteStore` (`stop_workspace`, `resume_workspace`,
    `workspace_stop`, `stop_run`) and to `WorkerStore` (`stop_run`, `workspace_stop`).
  - Implement them on MemStore, PgStore, `Writer` (`writer.rs:330`, `worker.rs:497`), and the spies
    `UsageSpy` and `SpyStore`.
  - Change the claim gate and the adopt settlement in both stores; Pg gets an explicit transaction.
  - Add the `queue_item` workspace end to end: MemStore, PgStore, `Backend::queue_item` (`backend.rs:618`),
    `StoreRequest::QueueItem`, `serve_queue` (`store_worker.rs:2420`), `on_queue` (`backlog/mod.rs:507`).
    It is an inherent method, so it gets a MemStore unit test plus a Pg test, not a generic conformance case.
  - Run `cargo sqlx prepare`.
- **Files**:
  - `crates/htui-core/src/store/{traits,mem,worker,conformance}.rs`
  - `crates/htui-store/src/{pg/write.rs, writer.rs, worker.rs, backend.rs}`
  - `crates/htui-store/.sqlx/`
  - `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`
  - `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/backlog/mod.rs` (+ the `StoreRequest` definition file)
- **Validate**: `cargo test -p htui-core --all-features --test mem_store`;
  `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features --test pg_conformance`.

### T4: Engine and runtime workspace plumbing (after T2; parallel with T3 in its own worktree)
- **Action**:
  - Add `Command::StartRun { …, workspace: Option<WorkspaceId> }` and update the 74 constructions:
    1 production, 73 tests and test-support.
  - Name the new field at the destructuring sites `engine.rs:630` and `runtime.rs:2647`.
  - `enqueue` / `enqueue_in_batch` / `enqueue_with` take the workspace into `NewRun`.
  - `admit` passes `workspace_of[item]`.
  - `RunsTab::action` sends `Some(ctx.scope.workspace_id)`.
  - Tests:
    - an engine test: `StartRun` in a workspace records it on the run;
    - an engine test: a claim refused `WorkspaceStopped` leaves the run queued and writes nothing.
      This uses MemStore's existing `workspace_stop` seeding through T3's verb, **so this test lands
      after T3 merges**.
    - a runtime test: `admit` records the entry's workspace.
- **Files**:
  - `crates/htui-orch/src/{command,engine,conformance}.rs`, `crates/htui-orch/tests/gix_isolator.rs`
  - `crates/htui-worker/src/{runtime,views}.rs`, `crates/htui-worker/tests/auto_queue.rs`
  - `crates/htui/src/{run_worker.rs, ui/tabs/backlog/detail/runs.rs}`
  - `crates/htui/tests/{chat,runs_pg,worker_crash_pg,worker_pg}.rs`
- **Validate**: `cargo test -p htui-orch -p htui-worker --all-features --no-fail-fast` (grep SIGABRT);
  `cargo test -p htui --all-features -- --test-threads=1`.

### T5: Docs (any time)
- **Action**: D8.
- **Files**: `docs/ANA-2.md`.

## Independence (verified)

- **T1 → T2:** `.sqlx` prepare needs the new columns.
- **T2 → T3, T4:** both compile against T2's types.
- **T3 ∥ T4:** their file sets are disjoint.
  - T3 owns `htui-core/src/store/*`, `htui-store/src/{pg/write,writer,worker,backend}.rs`,
    `htui-agent` spies, `htui/src/store_worker.rs` and `backlog/mod.rs`.
  - T4 owns `htui-orch`, `htui-worker`, `htui/src/run_worker.rs`, `backlog/detail/runs.rs` and `htui/tests/*`.
  - T2 also edits `backlog/detail/runs.rs` (styles) and `htui-orch/src/engine.rs` (match arms), but T2
    lands first.
  - Run T3 and T4 in **separate worktrees**, each its own branch off T2, so neither compiles the
    other's half-edits. Merge T3 first, then T4, plus the one T4 test that needs T3's verb.
- **T5:** disjoint from everything.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep output for SIGABRT
cargo insta test --workspace --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A liveness site misjudged under D2 | Medium | Per-site verdicts listed in D2; conformance cases on slot vs overlap; verdicts in the commit message |
| Pg `adopt_runs` rewrite loses `SKIP LOCKED` or ordering | Medium | Existing adopt conformance cases plus the new settlement case on Pg |
| sqlx offline drift | Medium | `cargo sqlx prepare` on a scratch DB migrated by T1 (memory: the compose `htui` DB is empty) |
| Worktree disk (~10G target each) | Medium | `df -h .` before T3/T4; fall back to serial T3 then T4 on one tree |
| Engine tests near the debug stack limit | Low | `--no-fail-fast`, grep SIGABRT |
| A sweep claim per queued run per tick while stopped | Low | Accepted (D5); revisit in M2 |

## Acceptance

- [ ] All tasks complete
- [ ] Every new conformance case passes on MemStore and PgStore
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented

## Verified claims

Two fact-check agents read the tree at `41bf5c90`, plus a scratch DB and a compile probe on a /tmp copy, on 2026-10-18.

| Claim | Verdict | Evidence |
|---|---|---|
| `is_terminal` is `!is_active()` | TRUE | `run.rs:76-78`; pin test `run.rs:1012-1025` |
| `0001` status CHECKs are unnamed; names | TRUE | `pg_constraint`: `run_status_check`, `run_step_status_check`; no later migration touches them |
| The mirror carries `run`, `run_step`, `workspace`; has CHECKs | PARTLY | Those tables yes; `queue_entry` not mirrored; mirror has no CHECKs (`0001_mirror.sql:17`); so the "widen mirror CHECK" step was dropped |
| `schema_version` needs a bump | FALSE | It is the max migration version (`pg/mod.rs:723-726`); the step was dropped; the hard-coded counts in `tests/connect.rs` and `tests/migrations.rs` were added to T1 instead |
| `claim_run` order NotFound, NotClaimable, MissingTags, slot, overlap | TRUE | `mem.rs:5373-5442`, `pg/write.rs:4722-4859` |
| Overlap set = "non-terminal" | FALSE | It is the literal `('running','awaiting_approval')` (`mem.rs:5418`, `write.rs:4847`); D2 adds `stopped` by hand |
| Slot counts `running` only | TRUE | `mem.rs:5428`, `write.rs:4826` |
| `adopt_runs` is one transaction in Pg | FALSE | One CTE on the pool (`write.rs:5021`); D6 opens a transaction |
| `queue_item` is a trait verb | FALSE | Inherent on MemStore, PgStore and Backend; `StoreRequest::QueueItem` carries no workspace; T3 plumbs it end to end |
| WriteStore implementors = Mem, Pg, 2 spies | FALSE | Also `Writer` (`writer.rs:330`; WorkerStore `worker.rs:497`); added to T3 |
| `.sqlx` lives in `crates/htui-store/.sqlx`, `query!` macros | TRUE | 386 files; `SQLX_OFFLINE=true` (`.cargo/config.toml:6`) |
| Conformance runs over both stores | TRUE | `CASES` `:61`, `run_case` `:237`; `mem_store.rs:13`, `pg_conformance.rs:56` |
| `workspace_project` is many-to-many; `run.project_id` cascades | TRUE | `0001_init.sql:161`, `run_project_id_fkey` |
| `Command::StartRun` has one production construction with the scope at hand | TRUE | `runs.rs:835`, `ctx.scope.workspace_id` (`app/state.rs:82`); 73 test sites |
| `Engine::claim` names every `Claim` arm | TRUE | `engine.rs:826-830`; compile probe fails at `:804` |
| `admit` has the `QueueEntry` at the enqueue call | PARTLY | It loops over `Vec<ItemId>` (`runtime.rs:2337`); `workspace_of` map added |
| A refused claim retries on every sweep | TRUE | `claim_scan` `:2555`, `claim_queued` `:1997`, no back-off; cost accepted (D5) |
| The exhaustive match list | TRUE | Compiler-verified on a /tmp copy; list in T2; plus pin test `model/mod.rs:296-321` |
| No snapshot moves | TRUE | No `.snap` renders a `Run` row or iterates the enums |
| ANA-2 tables at `:600-654` | FALSE | `:653-671` (run), `:683-710` (step); `run.rs:57,99` cite the stale lines, fixed in D8 |
| Chat runs use a separate verb | TRUE | `start_chat_run` (`traits.rs:631`); `ChatRunSpec` at `run.rs:312` |
| T4 ∥ T5 (old numbering: Pg vs engine) independent | FALSE | Compile coupling through new trait verbs and `Backend::queue_item`; tasks regrouped so T3 ∥ T4 are disjoint and each compiles on T2 |

## Maintainer calls on the blueprint deviations

DV-1..DV-6 (blueprint) accepted as recommended, 2026-10-18: DV-1 the top-bar count lists stay three statuses; DV-2 `pg/relay.rs:544` and its MemStore twin unchanged in M1; DV-3 the `queue_item` signature change moves into T2; DV-4 `stop_run` takes no `at`; DV-5 the queue runner may still create queued runs in a stopped workspace (claim refuses them), gated in M2; DV-6 `queue_entry.workspace_id` is backfilled by the one-workspace rule. The blueprint is the implementation authority where it and this plan differ.
