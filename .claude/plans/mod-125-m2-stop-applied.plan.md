# Plan: MOD-125 M2 — Stop applied by the executor

**Source PRD**: `.claude/prds/mod-125-stop-and-resume-runs.prd.md`
**Selected Milestone**: 2 — Stop applied by the executor
**Complexity**: Large (one store read verb over five `WriteStore` implementors, an engine settle path, the
runtime's command poll and walk registry, the admission path, and the four M1 R1 deferrals; no migration)
**Status**: confirmed 2026-11-12 (fact-checked, table at the end; D11 (a) chosen by the maintainer). Routed PRD (continuing)
by `/handoff-run MOD-125`; ultracode accepted for implement and the review verify pass. Sandbox run
`hr/MOD-125`.

## Summary

M1 left a stopped workspace's record in the store and a fenced `stop_run` CAS that nobody calls. M2
makes the process that walks each run apply that record. The command poll that already applies a
cancel (TUI store loop and worker, every `COMMAND_POLL` = 1 s, both through `poll_once`) also lists
the running runs it leases in a stopped workspace. It interrupts each walk through the existing cancel
signal, graceful (up to `CANCEL_GRACE` for the turn's real end) or forced (no wait), then retakes the
lease and settles the run and its running steps as `stopped`, trees, branch and session banner kept.
Admission stops creating and counting runs for a stopped workspace, the walking commands are refused
there, and the four M1 review deferrals land.

## Decisions

- **D1 — The request is the `workspace_stop` row; no per-run command row.** The executor reads it
  through a new `WriteStore::pending_stops(owner) -> Vec<PendingStop { run, mode }>` (on `WriteStore`
  beside `pending_commands`, `traits.rs:2294`, forwarded by `WorkerStore` like `worker.rs:436`, so it is
  a conformance case on both stores): `workspace_stop` joined to `run` on
  `idx_run_workspace_status` (`0019:37`), `status = 'running'`, `lease_owner = owner`. No migration, no
  `run_command` kind.
  - Escalation is free: a forced upsert over a graceful one (M1 D4) shows on the next poll.
  - Runs whose lease lapsed are not listed: `adopt_runs` settles them `stopped` (M1 D6). A run leased
    on another box is that box's poll (MOD-41: a TUI on a worker box hands back, `Role::tails`,
    `runtime.rs:84`).
  - Rejected: a `run_command` kind `stop` per run (CHECK migration, rows missed for a run that starts
    running later, a separate escalation path).
- **D2 — Graceful and forced map onto the one cancel seam (settles the PRD open question).** There is
  one driver path: `Signal::Cancel{grace}` → relay `cancel` → `AgentSession::cancel(grace)`, which
  answers parked permissions `cancelled` (Q3), interrupts the turn (ACP `session/cancel` only when a
  turn is open, `acp/mod.rs:1785-1832`; CLI closes stdin and sends SIGINT to the group even at zero
  grace, `cli/mod.rs:1654-1684`), waits up to `grace` for the real stop reason, else synthesizes one
  `Done(Cancelled)` (only while a turn is open), **then SIGKILLs** (`acp/mod.rs:1418-1420`,
  `cli/mod.rs:1462-1464`). The recorder flushes on that `Done`, and `finish()` runs on the error path
  (`engine.rs:8906`). "Keep the session" means the agent's own persisted session plus the
  `session_started` banner's `session_id`; the process is not kept.
  - **Graceful** = `Cancel{CANCEL_GRACE}` (2 s), then the token drop at grace + **`STOP_SLACK`**.
  - **Forced** = `Cancel{ZERO}`, then the token drop at `STOP_SLACK`. The agent is SIGKILLed right
    after its interrupt, so it rarely persists the interrupted turn: M3 redoes it, or falls back to a
    fresh rerun with `resume_failed` (Q4).
  - **`STOP_SLACK` = 5 s**, not the cancel path's 1 s (`runtime.rs:783-796`): after the driver's grace
    the relay still kills, reaps, settles permission rows, flushes, `sync_step`s and `finish`es, and a
    token drop mid-flush loses rows whose `seq` was taken (`relay.rs:758-760`). `preempt_gracefully`
    already returns early once the walk ends, so a fast stop costs nothing. It takes the slack as a
    parameter; cancel keeps 1 s.
  - **Escalation** (forced over a graceful stop in flight): the relay does not re-read the control
    inside `cancel(2s)` (`engine.rs:10102-10110`), so escalation is the token drop at once. No
    `Done(Cancelled)` row is guaranteed on that path; the drop path's cleanup is `abandoned` (D3).
  - Rejected: a `Signal::Stop` variant; the driver behaves the same, the difference is the engine's
    settlement (D3).
- **D3 — `Engine::settle_stop(run) -> Result<bool>`, the `cancel_leased` shape
  (`engine.rs:2676-2712`) minus the terminal parts.** The preempted walk has already run `abandoned`
  (`engine.rs:2980-3001`: guards released, follow-ups closed, relay rows staled via
  `stale_dropped_requests_of`, **lease released**), and the walk task holds the run lock until then.
  So `settle_stop`, under the run lock inside `leased_window`:
  1. `take_lease` (works on a released lease; refused while another live owner holds it → `false`);
  2. `stop_run(run, owner)` (M1: `running → stopped`, running steps `stopped`, lease released);
  3. clears the run's `dead_walks` entry and guards if it had no live walk (D4).
  No `finish_run`, no `cleanup_walk` (trees and branch kept, M1 D2), item stays `in_progress`. One
  `info!` line `run {run} stopped ({mode})`; `cancel_leased` writes none, so there is no activity row
  to mirror and none is added. `false` (finished, cancelled, or the sweep took it first) writes
  nothing; the sweep's own settlement (M1 D6) converges on `stopped` too, so the race tests accept
  either writer. The walk's `PREEMPTED` refusal frame is published as on a cancel.
- **D4 — The runtime applies stops in the command poll.** `poll_once` (`runtime.rs:3161`, reached by
  the worker at `worker.rs:71` and the TUI store loop at `store_worker.rs:3697` →
  `run_worker.rs:155`) runs `pending_stops` after `pending_commands` and spawns `stop_walk(run, mode)`
  per row (as it spawns `cancel_run`, `:3187`):
  1. A new `Stopping` map in `Shared` keyed by run → mode (not on `Walks::Parent`, which `preempt`
     removes, `:812`) dedupes across polls: same or weaker mode → no-op; forced over graceful →
     `Walks::preempt` at once.
  2. `preempt_gracefully(run, grace(mode), STOP_SLACK)`.
  3. `engine.settle_stop(run)`; the map entry is removed when it returns.
  A pending cancel of the same run wins: `cancels_first` and `pending_commands` run first, and
  `stop_run`'s CAS then answers `false`. A run leased with no live walk (sweep adoption before the
  child is minted, or in `dead_walks`, `:2617`) goes straight to step 3.
- **D5 — Checks and `htui`'s own steps** stop when the token drops the walk (≤ grace + slack). The
  verifier (`ShellVerifier::spawn_and_wait`, `verify.rs:386,719-728`) uses tokio `kill_on_drop`, which
  kills only the group leader, so grandchildren of a dropped check (the binaries `cargo test` starts)
  survive today, for cancel as well. M2 wraps the verifier's child in the existing `GroupGuard`
  (`verify.rs:669`, today only `run_shell`'s) so a dropped check kills its group. M3 reruns them.
- **D6 — Divided steps (ANA-33 matrix row).** `stop_run` moves every `running` step, parts included,
  to `stopped`; `pending` parts stay `pending`. The matrix: a stop during the planner, during a part
  with a queued sibling, during the pre-merge, during integrate. The subgraph-node row waits for
  MOD-143 (not built).
- **D7 — Walking commands refused in a stopped workspace.** `Kit::refusal` is synchronous with no store
  access (`runtime.rs:1077-1086`), so the check goes in `on_run_unless_claimed` (its only caller,
  `:2888`) after the run lock: read `row.workspace_id` (`tag_run`) and `workspace_stop`, and refuse
  `AnswerGate`, `RetryStep`, `SelectFanout`, `AcceptArtifact` and `Unblock`'s resume case (`:3215`)
  with `"workspace <slug> is stopped: resume it first"`. This covers M1 D1's "refuse approving a parked
  gate". `StartRun` keeps M1's behaviour (`start_run` `:2747`: the claim answers
  `ClaimRefused{WorkspaceStopped}` and the run stays `queued`, `engine.rs:25530`). `CancelRun` is never
  refused.
- **D8 — Admission (L-7 half, DV-5).** `admit` (`runtime.rs:2174-2353`) reads `workspace_stop` once per
  distinct workspace in its `workspace_of` map (`:2290`) and skips stopped workspaces' entries: they
  stay queued, untouched, and **do not count toward the `Stalled` drain** (`:2310,2386`), so a stop
  never closes a batch. `queued_runs_on_box` (inherent on MemStore `mem.rs:1058`, PgStore
  `pg/read.rs:2472`, forwarded by Backend and `WorkerHost`) excludes queued runs of a stopped
  workspace: its only callers are `admit` (`:2247`, free slots) and `claim_scan` (`:2569`), so those
  runs use no free slot and are not re-claimed every sweep (retires M1 D5's accepted cost). The TUI
  queue header's `slots_used` (`queue_overview.rs:97`) reads through `admit`'s count and drops them
  too. After resume both reappear and the claim gate's slot check still caps `running`.
- **D9 — M-3: `delete_workspace` refuses while a member-project run that recorded this workspace is
  `stopped`.** MOD-147 A1 already refuses on a live (incl. `stopped`) *scope-row* run
  (`pg/write.rs:10677-10700`, `mem.rs:5799-5815`); M2 adds `run.workspace_id = $1 AND
  status = 'stopped'` **excluding scope-row runs** (no double count), Pg locking the `workspace_stop`
  row `FOR UPDATE` first, answering `Constraint(workspace_has_stopped_run(slug, n))`. The shell already
  puts `{request}: {message}` on the status line; a test in `crates/htui/tests/hierarchy.rs` pins it
  (pattern `a_directory_gone_before_the_choice_is_refused_on_the_status_line`, `:3289`).
- **D10 — L-3: readable FK refusal.** One helper `workspace_gone(ws)` in `store/traits.rs` reading
  `"this workspace no longer exists: switch workspace"`; Pg `create_run` (`pg/write.rs:6342-6382`) and
  `queue_item` (`:10911-10925`) map `fk_run_workspace` / `fk_queue_entry_workspace` to it (the
  `phase_fk_refused` pattern, `:1210`); MemStore uses it in place of `references_no_row`
  (`mem.rs:6381,1110`). No test pins the current text.
- **D11 — L-5: raise the minimum version. Maintainer chose (a), 2026-11-12.** The mechanism is
  `app_setting.htui_target_version`, raised by `apply_migrations` to `htui-store`'s
  `CARGO_PKG_VERSION` (`pg/mod.rs:58,67,1104`); a headless process below it is refused
  (`BelowTarget`, `:1005`), a TUI warns. It has never moved; each of the nine crates declares its own
  `0.1.0`, no workspace version.
  - **(a, recommended)** every crate `0.1.0 → 0.2.0`: one meaning for "the version" (`htui --version`,
    the box record, ACP/MCP client and server info, user agents). Cost: the MCP relay handshake
    requires an exact `RELAY_VERSION` match (`htui-mcp/src/lib.rs:40`, `channel.rs:294`), so an
    installed relay from the old build is refused until reinstalled; demo boxes recorded at `0.1.0`
    read as needing a probe.
  - (b) `htui-store` alone: only the target moves; the relay is untouched; versions diverge
    (`--version` says 0.1.0, the box record 0.2.0).
  - (c) defer to M4 with the first `stop_workspace` caller: an old worker would silently ignore a stop
    issued from a new build.
- **D12 — Pending follow-ups of a stopped run are refused, as on a cancel.** A follow-up is applied
  inside the live session's window; when the driver returns `Cancelled` the relay refuses pending ones
  with `FOLLOW_UP_SESSION_CANCELLED` (`record/relay.rs:290-297`), and a dropped walk's `abandoned`
  refuses them with `FOLLOW_UP_SESSION_ENDED` (`engine.rs:3023`). M2 keeps that (the person re-sends
  after resume) and pins it with a test; nothing starts a follow-up on a stopped run.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Durable request poll | `WriteStore::pending_commands` (`traits.rs:2294`), Pg `pg/relay.rs:523-549`, `WorkerStore` forward `worker.rs:436` | Owner-fenced listing read by `poll_once`; conformance `pending_commands_are_the_owners_and_the_boxs_free_runs` (`conformance.rs:17973`) |
| Apply in the runtime | `cancel_run` (`runtime.rs:2992-3155`), `poll_once` (`:3161-3195`), `applying(id)` dedupe | preempt, then dispatch under the run lock; one spawned task per row |
| Engine settle | `cancel_leased` (`engine.rs:2676-2712`) | `take_lease` inside `leased_window`, the fenced write, release |
| Readable FK | `phase_fk_refused` (`pg/write.rs:1210`) | constraint name → sentence |
| Errors | `StoreError::Constraint`, `workspace_has_live_run` (`traits.rs:3679`) | sentence helpers in `traits.rs` |
| Store tests off-trait | `run_batch_spend_answers_alike_on_both_stores` (`pg_criteria.rs:8973`); `queued_runs_on_box` tests `mem.rs:15008`, `pg_criteria.rs:6232` | mem unit test + Pg criteria twin |
| Grace observation | `Graced` (`run_worker.rs:550`), `Watched` (`tests/relay.rs:444`), `Held` (`auto_queue.rs:320`) | the fake ignores grace (`fake.rs:614`); assert the grace the session received |
| Runtime tests | `the_workers_command_poll_applies_a_pending_cancel` (`run_worker.rs:3728`); `worker_pg.rs` `a_cancel_from_another_box_ends_a_live_worker_walk` (`:967`, `CANCEL_BOUND` `:796`); orch `a_cancel_during_a_part_leaves_the_queued_part_pending` (~`15331`) | paused time without a real child, real time with one |

## Tasks

Every task leaves `cargo check --workspace --all-targets --all-features` green. Order:
**(T1 ∥ T2 ∥ T4) → T3 → T5.** T1, T2 and T4 run in separate worktrees on named branches off
`hr/MOD-125`; merge T1, T2, T4, then T3 on the merged tree.

### T1: Store — `pending_stops`, the admission count, M-3, L-3
- **Action** (tests first):
  - conformance cases (`CASES` entry + `run_case` arm each; pins 277 → 282 in `mem_store.rs:37` and
    `pg_conformance.rs:59`):
    `pending_stops_lists_the_owners_running_runs_of_a_stopped_workspace` (graceful and forced; not
    another owner's; not queued/parked/stopped; not a run without a workspace),
    `pending_stops_shows_an_escalation`, `pending_stops_is_empty_after_resume`,
    `delete_workspace_refuses_while_a_member_run_of_it_is_stopped` (and passes once it is not),
    `create_run_naming_a_gone_workspace_reads_switch_workspace`;
  - mem unit + `pg_criteria` twins: `queued_runs_on_box_skips_a_stopped_workspaces_runs` (+ resume),
    `queue_item_naming_a_gone_workspace_reads_switch_workspace`;
  - `crates/htui/tests/hierarchy.rs`: `a_workspace_with_a_stopped_run_is_refused_on_the_status_line`.
  - Then `PendingStop` (model), `pending_stops` on `WriteStore` (MemStore, PgStore, `Writer`, the spies
    `UsageSpy` and `SpyStore`) forwarded by `WorkerStore`; D8's `queued_runs_on_box` filter; D9; D10.
  - `cargo sqlx prepare` against a migrated scratch DB (memory: the compose `htui` DB is empty).
- **Files**: `crates/htui-core/src/{model/stop.rs, store/{traits,worker,mem,conformance}.rs}`,
  `crates/htui-core/tests/mem_store.rs`,
  `crates/htui-store/src/{pg/read.rs, pg/write.rs, pg/relay.rs, worker.rs, writer.rs}`,
  `crates/htui-store/.sqlx/`, `crates/htui-store/tests/{pg_conformance,pg_criteria}.rs`,
  `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`,
  `crates/htui/tests/hierarchy.rs`.
- **Validate**: `cargo test -p htui-core --all-features --test mem_store`;
  `cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1`;
  `cargo test -p htui --all-features --test hierarchy`.

### T2: Engine — `settle_stop`, the verifier's group, the divided-step matrix
- **Action**: tests first (paused time, fake driver; grace asserted through a recording session):
  - `a_stop_retakes_the_released_lease_and_settles_run_and_step_stopped_keeping_the_trees`;
  - `a_stop_records_the_turns_cancelled_done` (every event before `Done(Cancelled)` has a row);
  - `a_stop_of_a_run_that_finished_or_was_cancelled_writes_nothing`;
  - `a_stop_while_another_owner_holds_the_lease_writes_nothing`;
  - `a_stopped_steps_pending_follow_up_is_refused_session_cancelled` (D12);
  - `a_dropped_check_kills_its_process_group` (verify.rs, real time, a child that forks);
  - orch conformance: stop during the planner, a part with a queued sibling, the pre-merge, integrate
    (D6); pin 230 → 234 in `conformance.rs:16929` and `tests/fake_conformance.rs:18`.
  - Then `Engine::settle_stop` (D3) and the `GroupGuard` in `ShellVerifier` (D5).
- **Files**: `crates/htui-orch/src/{engine.rs, conformance.rs, verify.rs}`,
  `crates/htui-orch/tests/fake_conformance.rs`.
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast` (grep SIGABRT; memory: stack headroom).

### T3: Runtime — apply stops, refuse walking commands, admission (after T1, T2)
- **Action**: tests first:
  - runtime (paused, `Held`/`Graced`): `the_command_poll_stops_a_live_walk_gracefully` (grace 2 s
    seen), `a_forced_stop_cancels_with_zero_grace`, `a_forced_poll_escalates_a_graceful_stop`
    (no `Done(Cancelled)` asserted), `a_second_poll_does_not_restop`, `a_pending_cancel_beats_a_stop`,
    `a_leased_run_with_no_walk_is_settled_directly`,
    `walking_commands_are_refused_in_a_stopped_workspace` (each of the five), `a_cancel_is_not`,
    `admit_skips_a_stopped_workspaces_entries_without_draining_the_batch`;
  - `crates/htui/src/run_worker.rs`: `the_store_loop_applies_a_workspace_stop_on_its_own`;
  - `crates/htui/tests/worker_pg.rs` (real time, `STOP_BOUND = CANCEL_GRACE + STOP_SLACK + COMMAND_POLL + 2s`):
    `a_stop_from_another_box_settles_a_live_worker_walk_stopped`, and the forced twin.
  - Then D4 (`Stopping` map, `poll_once`, `stop_walk`, `STOP_SLACK`, `preempt_gracefully`'s slack
    parameter), D7, D8's `admit` half.
- **Files**: `crates/htui-worker/src/{runtime.rs, worker.rs}`, `crates/htui-worker/tests/auto_queue.rs`,
  `crates/htui/src/run_worker.rs`, `crates/htui/tests/worker_pg.rs`.
- **Validate**: `cargo test -p htui-worker --all-features --no-fail-fast`;
  `cargo test -p htui --all-features -- --test-threads=1` (memory: scheduling-dependent suite).

### T4: Minimum version (D11 (a): every crate `0.1.0 → 0.2.0`)
- **Action**: the chosen bump and `Cargo.lock`. No literal `0.1.0` pin compares against the build
  version (fact-checked); tests that follow the build use the constant. With (a), check the 7
  `box_settings__*.snap` renders of the demo `0.1.0` box still pass (fixture value, unchanged) and
  `.claude/skills/htui-dev/tests/cargo.test.ts` (unread by the fact-check).
- **Files**: `crates/*/Cargo.toml`, `Cargo.lock` (and `.claude/skills/htui-dev/tests/cargo.test.ts` if it pins).
- **Validate**: `cargo test -p htui-store --all-features --test migrations --test read_only_pg -- --test-threads=1`;
  `cargo test -p htui --all-features --bin htui cli`.

### T5: Docs (after T3)
- **Action**: ANA-2's M1 rows that say "orchestrator (M2)" (`docs/ANA-2.md` run rows `:706-712`, step
  rows `:757-764`) get M2's mechanism under `*Amended by MOD-125 M2, 2026-11-12:*`; the PRD's
  "mapping onto ACP and the CLI adapter" open question is ticked with D2. `CONCEPTS.md` unchanged
  (`:67` "stopping a run saves its conversation and can always be resumed" still holds).
- **Files**: `docs/ANA-2.md`, `.claude/prds/mod-125-stop-and-resume-runs.prd.md`.

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
| A stop races a step finishing, a cancel or the sweep | Medium | `take_lease` + fenced CAS; cancel first in the poll; the sweep settles `stopped` too, so tests accept either writer |
| A forced stop kills before the agent persists the turn | High (by design) | M3 redoes the turn or reruns fresh with `resume_failed` (Q4) |
| The Claude CLI may not persist a session interrupted by SIGINT | Medium | M3's live probe; M2 records the cancelled turn either way |
| An escalated stop drops the walk before its `Done(Cancelled)` row | Medium | `abandoned` stales relay rows and refuses follow-ups; the gap is the unflushed buffer (PRD risk table) |
| A run stopped before its banner's first flush has no `session_started` row | Low | M3 treats it as not resumable (fresh rerun, `resume_failed`) |
| The relay handshake refuses an old installed relay after a full bump (D11 a) | Medium | Reinstall the relay; or choose D11 (b) |
| Worktree disk (~10 G each) | Low | 2.6 T free on the repo volume (2026-11-12) |
| Live-Qdrant timeouts in the sandbox gate | High | Known (memory); owed on host |

## Acceptance

- [ ] All tasks complete
- [ ] Every new conformance case passes on MemStore and PgStore
- [ ] A worker walk and a TUI walk each settle `stopped` on a graceful and a forced stop
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented

## Verified claims

Six read-only agents at `0e92cd47`, 2026-11-12: three grounding readers (cancel path, drivers and
recorder, store), then three fact-checkers over the first draft (runtime/engine, driver/recorder,
store/pins/independence). Falsified claims were amended above before CONFIRM.

| Claim | Verdict | Evidence |
|---|---|---|
| One driver cancel seam, interrupt then kill, no graceful/forced split | TRUE | `relay.rs:41-49,730`; `driver.rs:466-472`; `acp/mod.rs:1417-1421`; `cli/mod.rs:1454-1465` |
| Forced = `Cancel{ZERO}` still answers permissions, one `Done(Cancelled)`, recorded, killed; CLI still SIGINTs | TRUE (Done only while a turn is open) | `acp/mod.rs:1794,1530`; `cli/mod.rs:1456-1460,1665-1667,1836`; `record.rs:~1165`; `engine.rs:8906` |
| Graceful leaves every pre-`Done` event recorded | PARTLY → D2 | the 1 s slack can drop a mid-flush relay; `STOP_SLACK` = 5 s added |
| A forced poll can escalate a graceful stop through the signal | FALSE → D2 | relay does not re-read control in `cancel` (`engine.rs:10102-10110`); escalation is the token drop |
| The fake can distinguish graceful from forced | FALSE → Patterns | `fake.rs:614` ignores grace; tests assert the received grace (`Graced`, `Watched`) |
| The lease is still held after a preempted walk | FALSE → D3 | `abandoned` releases it (`engine.rs:2980-3001`, `pg/write.rs:6987-6994`); `settle_stop` retakes it like `cancel_leased` (`engine.rs:2679`) |
| A preempted walk is not a crash or retry | TRUE | `abandoned` + `PREEMPTED` refusal on every walk path (`runtime.rs:2795,2899,2066,2640,3242`); backoff only on non-cancel errors (`:2645`) |
| `Walks` can carry a stopping marker | FALSE → D4 | `preempt` removes the `Parent` (`:812`); a separate `Stopping` map |
| TUI and worker share `poll_once` | TRUE | `worker.rs:71`; `store_worker.rs:3697` → `run_worker.rs:155` → `runtime.rs:1355` |
| `Kit::refusal` is the place and knows the workspace | FALSE → D7 | sync, no store (`:1077-1086`); `StartRun` bypasses it (`:2747`); check moved to `on_run_unless_claimed`, `Unblock` added |
| `cancel_leased` writes an activity line | FALSE → D3 | no activity/log/`run_event` write in `engine.rs:2676-2712` |
| Pending follow-ups wait for resume | FALSE → D12 | refused `FOLLOW_UP_SESSION_CANCELLED` (`record/relay.rs:290-297`) or `_ENDED` (`engine.rs:3023`) |
| A run can be leased with no live walk | TRUE → D3/D4 | sweep adoption before the child; `dead_walks` (`:2617`); `settle_stop` clears it |
| The verifier's dropped check is killed with its group | FALSE → D5 | `kill_on_drop` on the leader only (`verify.rs:386,719-728`); `GroupGuard` only in `run_shell` (`:572,669`); D5 adds it |
| `admit` can skip per workspace; `queued_runs_on_box` callers | PARTLY → D8 | `workspace_of` (`:2290`); skipped entries must not drain the batch (`:2310,2386`); callers only `admit`, `claim_scan` (+ TUI header through `admit`) |
| `pending_stops` on `WorkerStore` only, as a conformance case | FALSE → D1 | conformance is generic over `WriteStore` (`conformance.rs:409`); `pending_commands` is `WriteStore` (`traits.rs:2294`); moved there, spies join T1 |
| `queued_runs_on_box` / `queue_item` as conformance cases | FALSE → T1 | inherent methods; mem unit + `pg_criteria` tests |
| `idx_run_workspace_status` on `(workspace_id, status)` | TRUE | `0019_stop_runs.sql:37`; no `lease_owner` index, so the query starts from `workspace_stop` |
| M-3 is unaddressed | PARTLY → D9 | scope-row runs (incl. `stopped`) already refused by MOD-147 A1; member-project runs not; no double count |
| No test pins the FK refusal text | TRUE | `conformance.rs:5709`, `mem.rs:17168`, `pg_criteria.rs:12087` assert `Constraint(_)` only |
| Minimum version mechanism | TRUE → D11 | `pg/mod.rs:58,67,1005,1104`; never raised; nine crates at `0.1.0`; relay exact-match `htui-mcp/src/lib.rs:40`, `channel.rs:294` |
| No literal `0.1.0` pin compares against the build | TRUE | pins are passed-in values or fixtures (`fixtures.rs:519`, 7 `box_settings__*.snap`); build-following tests use the constant |
| CONCEPTS.md invariant moves | FALSE | only `:67`, which still holds |
| CASES = 277, pins | TRUE | `conformance.rs:99`; `mem_store.rs:37`; `pg_conformance.rs:59` |
| Orch conformance count pins | TRUE (230) | `htui-orch/src/conformance.rs:16929`, `tests/fake_conformance.rs:18` → T2 |
| T1 ∥ T2 ∥ T4 independent | TRUE (after the file-list fixes) | T2 needs only M1's `stop_run`; T1 = core store, htui-store, htui-agent spies, `htui/tests/hierarchy.rs`; T2 = `htui-orch/{engine,conformance,verify}.rs`, `tests/fake_conformance.rs`; T4 = `Cargo.toml`s, `Cargo.lock`: disjoint. T3 (`htui-worker`, `htui/src/run_worker.rs`, `htui/tests/worker_pg.rs`) needs T1's `pending_stops` and T2's `settle_stop` |
