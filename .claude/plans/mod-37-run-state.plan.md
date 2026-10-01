# Plan: MOD-37 milestone 1 - Run state and visibility

**Source PRD**: `.claude/prds/mod-37-orchestrator-hardening.prd.md`
**Selected Milestone**: 1 - Run state and visibility (R-3, R-40, R-41, R-44, R-51, R-53, T7)
**Complexity**: Medium
**Routing**: PRD path; ultracode for implement and review (maintainer-confirmed).

## Summary
A parked run shows its reason, the Runs pane learns when a step starts and when a command is waiting
on a live walk, a dropped `RunStream` failure reaches the status line, and a promoted chat's stream
no longer depends on the Chat tab's follow arriving first. R-44 and R-53 are proposed for
re-deferral with a reason, because the tree already marks and pins them and no fix is smaller than
a design choice. The run-lifecycle code now lives in `crates/htui-worker`, not `run_worker.rs`.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| `RunStepSummary` has no `gate_note`; `RunStep` has it | true | `htui-core/src/model/run.rs:235-236` is the only `gate_note` in that file |
| `run_step.gate_note` column exists in both schemas, so R-3 needs no migration | true | indexed at `htui-store/migrations/0001_init.sql:484` and `cache_migrations/0001_mirror.sql:116` |
| `FrameKind` has no step-start or waiting variant | true | `htui-worker/src/views.rs:147-166`: Subscribed, Started, SessionDone, Rested, Changed, Adopted, Error |
| `SessionSink` has only `after_done` | true | `htui-orch/src/engine.rs:201`, `:216`, `:6610`, `:9670` (the `started` at `:8335` is a test helper) |
| Seven commands take `lock_unless_cancelled` with no frame while waiting | true | `htui-worker/src/runtime.rs:1723, 1929, 2094, 2182, 2396, 2518, 2540` |
| `App::is_fresh` drops stale replies | true | `htui/src/app/state.rs:329` |
| `TAIL_WIDTH` is 24 and `tail` is cut with `…` | true | `htui/src/ui/tabs/backlog/detail/runs.rs:128`, `:1095` |
| HANDOFF's `run_worker.rs` citations for R-40/R-51 | false (moved) | logic is in `htui-worker/src/{runtime,views}.rs`; `htui/src/run_worker.rs` is the TUI adapter |
| `RunStepSummary` has three builders (Mem, Pg, cache mirror) plus fixtures | true per research | `store/mem.rs:~1264`, `pg/rows.rs` + `pg/read.rs:~330`, `cache/read.rs:~540`; fixtures in `closeout.rs:265` and `runs.rs` tests |
| R-3 changes a `query_as!` text, so one `.sqlx` entry is regenerated | true per research | `pg/read.rs` SELECT; needs the migrated scratch DB (`docs/hr-sandbox.md:196`) |
| R-53's window is already closed by D171/D184 | true per research | `runs.rs:378` re-requests actions with each `Runs`; engine re-checks (D184) |

Claims marked "per research" come from the grounding agent's file:line reading and are re-checked by
the implementer's first failing test.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `app/update.rs:605` | tests named as sentences; plan and decision IDs in doc comments |
| Errors | `agent_worker.rs:~938` | `failed(PROMOTE_STEP, &err)`; a dropped send is `let _ =` with a comment |
| Tests (store) | `htui-core/src/store/conformance.rs:12754, 12890` | `run_and_steps_round_trip` leg (g), field-by-field summary comparison |
| Tests (pane) | `runs.rs:3276-3325` | build a `RunFrame`, call `pane.on_reply` |
| Tests (worker) | `htui-worker/src/runtime.rs` frame assertions near `:2127` | assert frames through the publisher |
| Tests (chat) | `htui/tests/chat.rs:1688, 1728` | needs `--features testkit` |
| SQL | `pg/read.rs` | `query_as!` binds positionally: append the new column last |

## Files to Change and task file sets
| Task | Files (touched set) |
|---|---|
| T1 R-3 model and stores | `htui-core/src/model/run.rs`, `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs`, `htui-store/src/pg/rows.rs`, `htui-store/src/pg/read.rs`, `htui-store/src/cache/read.rs`, `.sqlx/query-*.json`, `htui-orch/src/closeout.rs`, `htui/src/ui/tabs/backlog/detail/runs.rs` (fixtures only) |
| T2 R-3 pane render | `htui/src/ui/tabs/backlog/detail/runs.rs` (+ its snapshots) |
| T3 R-40 step-start frame | `htui-orch/src/engine.rs`, `htui-orch/src/fake.rs` if it implements `SessionSink`, `htui-worker/src/views.rs`, `htui-worker/src/runtime.rs` (tests), `runs.rs` (`invalidates`) |
| T4 R-51 waiting frame | `htui-worker/src/runtime.rs`, `htui-worker/src/views.rs`, `runs.rs` |
| T5 R-41 surface `RunStream` errors | `runs.rs` |
| T6 T7 chat stream | `htui/src/agent_worker.rs`, `htui/src/ui/tabs/chat/mod.rs`, `htui/src/store_worker.rs`, `htui/src/testkit.rs`, `htui/tests/chat.rs` |

**Independence (intersection of the sets):**
- T1, T2, T3, T4, T5 all touch `runs.rs`; T3 and T4 also share `views.rs` and `runtime.rs`. They run **serial**, in the order T1, T2, T3, T4, T5.
- T6 shares no file with any other task, so it runs **in parallel** with that serial lane.
- Two lanes in total. Ultracode fans out lane 1 (serial stages) and T6; it does not parallelise inside lane 1.

## Tasks
### Task 1: R-3 `gate_note` on `RunStepSummary` (model and the three builders)
- **Action**: add `gate_note: Option<String>` after `agent_name`; fill it in `MemStore`, `PgStore` (`StepRow::into_summary` and the SELECT, appended last) and the cache mirror; fix every fixture; regenerate the one `.sqlx` entry.
- **Mirror**: conformance leg (g) at `:12890`. **Test first**: extend leg (g) to compare `gate_note`; it is red before the fix.
- **Validate**: `cargo test -p htui-core -p htui-store --all-features -- --test-threads=1`; `cargo sqlx prepare --check`.

### Task 2: R-3 render the reason on a parked step
- **Action**: show `gate_note` for an `awaiting_approval` step in `step_lines`/`run_lines`, within the 43-column grid.
- **Design choice (maintainer)**: third line, or fold onto the run line (see Open choices). Do not also write `run.failure` (HANDOFF names only the summary route).
- **Test first**: a parked-step width test beside `runs.rs:1866`.
- **Validate**: `cargo insta test -p htui --all-features` and the pane tests.

### Task 3: R-40 step-start frame
- **Action**: add a defaulted no-op `started` to `SessionSink`; call it after each `Pending → Running` move (`engine.rs:3321-3344, 3897, 3937, 4476`); `ProgressSink` publishes a frame. Frame kind: reuse `Changed` (`invalidates` already re-reads) unless a new `StepStarted` is wanted.
- **Test first**: `htui-worker` frame assertion that a step start is published; engine sink test.
- **Validate**: `cargo test -p htui-orch -p htui-worker --no-fail-fast -- --test-threads=1`; grep SIGABRT (stack headroom).

### Task 4: R-51 waiting frame
- **Action**: in `lock_unless_cancelled`, try the lock first; on a miss publish a waiting frame, then await. The pane shows "waiting for the walk". `invalidates` returns false for it.
- **Test first**: a command issued during a live walk publishes the waiting frame before the walk rests.
- **Validate**: `htui-worker` and pane tests.

### Task 5: R-41 surface a dropped failure
- **Action**: `RunsTab::on_reply` emits the `FrameKind::Error` sentence as an `Action::Error` from the `RunStream` frame, which survives the `is_fresh` drop.
- **Test first**: mirror `a_failure_overtaken_by_a_newer_request_never_reaches_the_status_line` (`update.rs:692`) for the frame path.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### Task 6: T7 residual window
- **Action**: the stream must not depend on the Chat tab's follow arriving before a second `Orch` request. Option (b): move the stream to a non-`Orch` address inside the worker at accept time. Option (a): issue `ChatFollow` at dispatch.
- **Design choice (maintainer)**: (a) vs (b). The grounding agent recommends (b) but did not verify the worker has an address to move to; the implementer settles that first, as a spike, before writing code.
- **Test first**: two `Orch` requests between `ChatAccepted` and `ChatFollow` lose no frames.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1` (needs testkit).

## Proposed re-deferrals (need maintainer decision)
- **R-44**: `…` marks the cut and the width test already pins it. Any real fix is a layout design (drop the indent, abbreviate the model, add a line). Re-defer to the Runs pane's next layout change.
- **R-53**: the window is closed by D171/D184 (the pane re-reads, the engine re-checks). Close as "mitigated, no code change", with no red-then-green test.

## Open choices
1. R-3 rendering: third line per parked step vs the run line.
2. R-40 frame kind: reuse `Changed` vs new `StepStarted`.
3. T7: option (a) vs (b), pending the spike.
4. R-44 and R-53: re-defer as proposed?

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1
cargo sqlx prepare --check -- --all-targets --all-features   # after Task 1; migrated scratch DB
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| `htui-orch` tests near the 2 MiB stack | M | box large futures; `--no-fail-fast`, grep SIGABRT |
| `.sqlx` regeneration needs a migrated scratch DB | M | `docs/hr-sandbox.md:196` recipe; `psql -h localhost -p 5439` |
| `FrameKind` additions break exhaustive matches | M | `invalidates` (`runs.rs:620`) is exhaustive, so the compiler finds them |
| Snapshot churn in the Runs pane | M | `cargo insta` review, one task at a time |
| T7 option (b) has no usable worker address | M | spike first; fall back to (a) |

## Acceptance
- [ ] Every task's failing test was red before its fix
- [ ] Validation passes on the real tree, `--test-threads=1`
- [ ] Reviewer gate run over the full change set
- [ ] HANDOFF R-lines updated: closed, or re-deferred with a reason

---
*Status: DONE (2026-10-02), commits `19ca229`..`151c87e`.*

## As built
- **T1 to T6** landed as planned. T6 took option (a): the spike confirmed all three of the
  blueprint's checks against option (b).
- **R-44 and R-53** are re-deferred, with their reasons on the HANDOFF line.
- **Review** (rust-reviewer) found no BLOCKER and no HIGH, and raised four findings. Each was
  checked by an adversarial verifier.
  - **F1** (MEDIUM) was confirmed and fixed (`409aced`, `151c87e`): the waiting line now clears
    only on `Rested`, `Error` or `Adopted`, because a walk's own `Changed` and `SessionDone` frames
    arrive while the command is still queued.
  - **F2** was refuted: binds run in promotion order, and the slot is taken before every return.
  - **F3** was refuted: a stale slot is overwritten or corrected by the next follow.
  - **F4** (LOW, `ChatTab::followed` stale after a failed bind) is deferred as harmless, since the
    next `Promoted` overwrites it.
