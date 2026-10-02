# Plan: MOD-37 milestone 2 - Store and engine correctness

**Source PRD**: `.claude/prds/mod-37-orchestrator-hardening.prd.md`
**Selected Milestone**: 2 - Store and engine correctness (R-5, R-6, R-29, R-30, R-31 remainder, R-32)
**Complexity**: Medium-Large
**Routing**: PRD path, M2 planned on its own; ultracode for implement (maintainer-confirmed).

## Summary
A `never`/`on_failure` pass writes `gate_outcome = 'skipped'` as ANA-2 §4.2's gate table says, and
the gate park becomes one store transaction modelled on `promote_step`. Override graph copies carry
their `phase_agent` rows through a new `WriteStore` writer. MemStore truncates `queued_at` to
microseconds as Postgres does. `recover::classify` adopts a step that changed only some repos. A
crash between a rejection and its unpark gets a resume path through `Unblock`. D138's `part_way`
keeps an `Io` error as `Io`. D131's lost park detail (R-32a) is proposed for re-deferral, because
the reason was never persisted and recovering it changes an exact-matched `gate_note`.

## Verified claims
| Claim | Verdict | Evidence |
|---|---|---|
| The park is three separate compare-and-sets, step → run → item | true | `htui-orch/src/gate.rs:488-510` (`park`); doc `:364-367` names the composite as R-5 |
| `GateOutcome::Skipped` exists and the Pg CHECK allows `'skipped'` | true | `htui-core/src/model/run.rs:140-151`; `migrations/0001_init.sql:483`; the mirror column has no CHECK (`cache_migrations/0001_mirror.sql:116`) |
| A pass writes no `gate_outcome` | true | `gate.rs` `apply`, arm `(OnFailure \| Never, Ok)`: `note_step` + `move_step(Running → Done)`; doc `:368-371` (H-9) |
| `'skipped'` on a pass is the specified value, not a new meaning | true | `gate.rs:368` cites ANA-2 §4.2's row "done + skipped" |
| Production code reads `gate_outcome` only as `== Some(Rejected)` | true | `gate.rs:909`, `engine.rs:2737`; the only other non-test reader is the pane cell `runs.rs:1043` |
| A human `Skip` answer also writes `Skipped` | true | `engine.rs:860` (`GateAnswer::Skipped`); the two are indistinguishable in the row, as the spec's table has it |
| The orch conformance pins "skipped is not written" | true | `htui-orch/src/conformance.rs:2317-2322`; T3 flips it |
| `promote_step` is a one-transaction step/run/item template on Pg | true | `htui-store/src/pg/write.rs:5033` (`pool.begin`, `FOR UPDATE OF s, r`) |
| `WorkerStore` implementors | true | trait `htui-core/src/store/worker.rs:100`; `MemStore` `:471`; `PgStore` `htui-store/src/worker.rs:79`; `Writer` `:358` |
| `WriteStore` implementors, including two test spies | true | `mem.rs:6217`, `pg/write.rs:719`, `writer.rs:312`, `htui-agent/src/conformance.rs:743` (`UsageSpy`), `htui-agent/tests/recorder.rs:430` (`SpyStore`) |
| Nothing writes `phase_agent`; `override_graph` says so | true | `htui-orch/src/graph.rs:398-400`; only raw-SQL test inserts (`pg_criteria.rs:3122, 3936`) |
| `phase_agent` is not mirrored in the cache | true per research | `htui-store/src/backend.rs:424`; no table in `cache_migrations` |
| MemStore's `phase_agents` always answers empty | true per research | `mem.rs:629-632`; pinned divergence `pg_criteria.rs:3658-3666`, `mem.rs:11150` |
| MemStore stores `queued_at` raw | true | `mem.rs:4166` `queued_at: new.queued_at` |
| Both stores already order overlaps by `(queued_at, id)` | true per research | `mem.rs:4259`; `pg/write.rs:3946` |
| `TIMESTAMPTZ_DIGITS` is the existing truncation idiom | true per research | `model/run.rs:265`; `clock.rs:43` |
| `classify` needs `after_hash` on every scope repo | true | `htui-orch/src/recover.rs:249-253` |
| Capture leaves `after_hash = None` for an unchanged repo | true | `isolate.rs:172` doc; `isolate/real.rs:970-993` pushes a row per tree |
| `record_commits` is one transaction on Pg | true | `pg/write.rs:4856-4862` (`pool.begin`, fence check, per-row upsert) |
| A failing candidate's trees are captured and recorded too | true | `engine.rs:3978-3994` (`release_trees`); this bears on R-30's relaxed rule, see T5 |
| A crash between `answer_gate(Rejected)` and the unpark is not adopted | true per research | `answer_guarded` (`engine.rs:845-914`) then `unpark` (`:5877`); `adopt_runs` takes only `Running` runs |
| `Unblock`'s third case and the resume walk share `resumable_park` | true | `status.rs:313-318`; callers `command.rs:1180` (`unblock_enabled`), `engine.rs:3057`, `:2131` |
| A stuck rejected run can already be retried by hand while budget lasts | true | `retry_enabled` (`command.rs:729-755`) accepts `Failed`; the rejection's own tail never runs, and nothing helps once the budget is spent |
| `part_way` maps `Io` to `Git` | true | `htui-orch/src/isolate/real.rs:165`; doc `:151-152` says so |
| D131's lost detail is the reason text, never persisted | true per research | `engine.rs` `never_reset` (~2690) holds it in memory; `interrupt_step` stores the constant `NOT_RESET` (`:108`) |
| Sandbox Postgres is up | true | `pg_isready -h localhost -p 5439` |

Claims marked "per research" come from the grounding agents' file:line reading and are re-checked by
the implementer's first failing test.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `recover.rs` tests (`classify_finished_by_every_after_hash`) | tests named as sentences; plan and decision IDs in doc comments |
| One-transaction write | `pg/write.rs:5033` `promote_step` | `pool.begin`, `SELECT … FOR UPDATE OF s, r`, CAS updates, `commit` |
| Fenced write | `pg/write.rs:4856` `record_commits` | `step_fence(&mut tx, step, fence)` first in the transaction |
| New writer | `create_phase` (`traits.rs:863`, `pg/write.rs:2704`, `mem.rs:2901`, `writer.rs:686`) | trait method, Mem state, Pg `query!`, `Writer` dispatch |
| Timestamps | `clock.rs:43` | `.trunc_subsecs(TIMESTAMPTZ_DIGITS)` |
| Errors | `isolate/copy.rs:308` | `io::Error::new(kind, text)` keeps the kind (`git.rs:1144` `is_lock_error` reads it) |
| Tests (store) | `htui-core/src/store/conformance.rs` `gate_answers_write_their_outcome` (`:9528`), `claim_run_applies_the_isolation_and_path_rules` (`:4796`) | one case, both stores via `run_case`; Mem via `htui-core/tests/mem_store.rs`, Pg via `htui-store/tests/pg_conformance.rs` |
| Tests (gate) | `gate.rs:1838` `a_stale_compare_and_set_in_the_gate_is_a_stale_write` | unit test on the gate CAS |
| Tests (engine) | `engine.rs:10347` and `walk_resumed_stops_on_a_stale_unpark` (`:12243`) | `Harness::new()` on MemStore, `harness.orch.restarted()` |
| Tests (recover) | `recover.rs:940-1130` | pure `#[test]` on hand-built rows (`running()`, `tree`, `commit`) |
| SQL | `pg/write.rs` | new or changed `query!` text means a `.sqlx` entry; migrated scratch DB (`docs/hr-sandbox.md`) |

## Files to Change and task file sets
| Task | Files (touched set) |
|---|---|
| T1 R-29 `queued_at` | `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs` (+ case registration in `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs` if cases are listed there) |
| T2 R-6 `phase_agent` writer | `htui-core/src/store/traits.rs`, `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs` (+ registrations), `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `.sqlx/query-*.json`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`, `htui-orch/src/graph.rs`, `htui-store/tests/pg_criteria.rs` (pinned divergence) |
| T3 R-5 pass and park composites | `htui-core/src/store/worker.rs`, `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs` (+ registrations), `htui-store/src/worker.rs`, `htui-store/src/pg/write.rs` or a sibling, `.sqlx/query-*.json`, `htui-orch/src/gate.rs`, `htui-orch/src/conformance.rs`; `htui-core/src/store/traits.rs`, `htui-store/src/writer.rs` and the two spies only if the ops are also put on `WriteStore` |
| T4 R-31 resume after a rejection | `htui-orch/src/status.rs`, `htui-orch/src/command.rs`, `htui-orch/src/engine.rs` (code and tests) |
| T5 R-30 `classify` | `htui-orch/src/recover.rs` |
| T6 R-32b `part_way` | `htui-orch/src/isolate/real.rs` |

**Independence (intersection of the sets):**
- T1, T2 and T3 all touch `mem.rs` and `htui-core/src/store/conformance.rs`; T2 and T3 also share `pg/write.rs` and `.sqlx`. They run **serial** as lane A, in the order T1, T2, T3.
- T4 touches `engine.rs`, `command.rs` and `status.rs`. No other task touches those, as long as T4's tests stay in `engine.rs`/`command.rs` and not in `htui-orch/src/conformance.rs` (which T3 owns). T4 is lane B.
- T5 (`recover.rs`) and T6 (`isolate/real.rs`) share no file with anything. They join lane B serially, after T4, since each is a one-file change.
- Two lanes. Lane B needs no Postgres and no `.sqlx`. Lane B runs in its own worktree (disk: 135G free at planning time).

## Tasks
### Task 1: R-29 MemStore truncates `queued_at`
- **Action**: in `State::create_run` store `new.queued_at.trunc_subsecs(TIMESTAMPTZ_DIGITS)`, as Postgres does on write. Fix any MemStore-only test that compares a raw `Utc::now()` `queued_at` with the row it gets back.
- **Mirror**: `claim_run_applies_the_isolation_and_path_rules` (`conformance.rs:4796`).
- **Test first**: a conformance case with two overlapping runs 500 ns apart inside one microsecond, ids in the reverse order of the nanoseconds; `Overlaps.with` is the lower id on both stores. Red on MemStore before the fix.
- **Validate**: `cargo test -p htui-core -p htui-store --all-features -- --test-threads=1`.

### Task 2: R-6 a `phase_agent` writer, used by `override_graph`
- **Action**: add a `WriteStore` writer for a phase's agent rows (name and shape per the blueprint; it takes the new phase id and the rows in `position` order). MemStore keeps real `phase_agent` state, so `phase_agents`, `resolve_graph` and `project_reach` answer from it. Pg uses one `query!` (one `.sqlx` entry). `Writer` dispatches; both spies get the method. `override_graph` writes `row.agents`, re-keyed to the cloned phase, after `create_phase`. Update the doc at `graph.rs:398-400` and the pinned divergence tests.
- **Mirror**: `create_phase` across the four sites.
- **Test first**: a conformance case that writes agents to a phase and reads them back through `phase_agents` and `resolve_graph`. It is red on MemStore before the fix, and it is a compile failure on Pg. Then an orch test: overriding a graph whose phase has agents leaves the clone with the same agents.
- **Validate**: as T1, plus `cargo sqlx prepare --check -- --all-targets --all-features`.

### Task 3: R-5 the pass writes `skipped`; the park is one transaction
- **Action**:
  - **(a)** One fenced `WorkerStore` op for the pass. It moves the step `Running → Done` with `gate_note` and `gate_outcome = 'skipped'`, as one statement.
  - **(b)** One fenced `WorkerStore` op for the park, modelled on `promote_step`. In one transaction it moves the step `Running → AwaitingApproval`, the run `Running → AwaitingApproval`, and the item `InProgress → AwaitingApproval`. The item keeps D17's "already moved" reading, and a refused step or run move writes nothing.
  - `gate::apply` and `park` call the new ops. D96's recovery stays, for rows written before this change.
  - Flip `conformance.rs:2317-2322`, and update every orch assertion that expected `None` on a passed step.
  - Update the docs at `gate.rs:364-371`.
- **Test first**:
  - A store conformance case for each op: the pass writes `skipped`, and a park on a run another writer has moved leaves the step `Running`.
  - At gate level, a park whose run was moved first leaves the step unmoved. Today the step is left `AwaitingApproval`, so this is red. The existing `a_stale_compare_and_set_in_the_gate_is_a_stale_write` may pin the old partial state and is updated with it.
- **Validate**: as T2, plus `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1`, and grep the output for SIGABRT (stack headroom).

### Task 4: R-31 a rejected-then-crashed run can be resumed
- **Action**: a run that is `awaiting_approval` whose latest step is `failed` with `gate_outcome = rejected` (the state a crash between `answer_gate` and `unpark` leaves) becomes resumable. That means `Unblock`'s third case answers `Resume` for it. `walk_resumed_from` unparks first (keeping D180's stale-write check) and then runs the rejection tail as D131's `settle_failed` does. The predicate gets a sibling that reads the steps, so `resumable_park` and its D196 single-predicate rule stay one source of truth.
- **Test first**: an engine test that answers a gate `Rejected`, simulates the crash by leaving the run and item parked, restarts (`harness.orch.restarted()`), and checks that `Unblock` resumes the run and the rejection tail runs. Red before the fix (`NotBlocked`, "run waits at a gate"). Also a `command.rs` case beside `unblock_names_its_three_cases_and_what_holds_the_item`.
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1`; grep SIGABRT.

### Task 5: R-30 adopt a step that changed only some repos
- **Action**: `captured` holds when every scope repo has a commit row and at least one scope row has `after_hash`. `record_commits` writes the capture batch in one transaction, so one `Some` proves the batch landed. A step that changed nothing and crashed at K4 is still retried; it cannot be told apart from a K3 crash without a new marker, and that case stays safe. **Guard**: `release_trees` records a failing candidate's capture as well. The blueprint settles whether the relaxed rule applies to `StepKind::Candidate`. Until it does, candidates keep today's every-repo rule.
- **Test first**: `(Some, None)` over two scope repos with output present is `Finished`. Today it is `Reset`, so this is red. Rewrite `classify_not_finished_with_one_repo_missing`'s first case (its "capture died between them" comment describes a state one transaction cannot produce). Keep "no row" and "stranger repo" as `Reset`. Add all-`None` as `Reset`, and a candidate case.
- **Validate**: `cargo test -p htui-orch --lib recover`.

### Task 6: R-32b `part_way` keeps `Io` as `Io`
- **Action**: `IsolateError::Io(io) => IsolateError::Io(io::Error::new(io.kind(), format!("{io}; {named}")))`; fix the doc at `real.rs:151-152`.
- **Test first**: a pure `#[test]` on `part_way` with an `Io` error and one row done, mirroring `already_reset_names_every_row_and_where_it_was` (`real.rs:4961`). Red before the fix, because the variant is `Git`.
- **Validate**: `cargo test -p htui-orch --lib isolate`.

## Proposed re-deferral (needs maintainer decision)
- **R-32a** (D131's not-reset park loses its labelled detail): `labelled` is always empty on a refusal, so what is actually lost is the reason text. It lives only in memory in `never_reset` and is never persisted; `interrupt_step` stores the constant `NOT_RESET`. Recovering it needs either a new column or a change to `gate_note`, which is matched exactly in the engine (`engine.rs:2741`) and in four tests. Calling `reset` again to rebuild it is unsafe, because it can move trees. This is diagnostics only. Re-defer to whoever next touches the park's note format.

## Open choices (settled 2026-10-02: maintainer CONFIRM, all as recommended)
1. **R-5 transaction half**: build the park composite. The store fan-out already exists for the pass op, and `promote_step` is the template. Rejected: close only the `skipped` half and re-defer the composite (D96 already closes the crash half).
2. **R-31 fix side**: recovery-side through `Unblock`. It also rescues runs already stuck. Rejected: an `answer_gate`-plus-unpark store transaction, which closes the window but leaves existing stuck rows stuck.
3. **R-30 and candidates**: candidates keep today's every-repo rule. Rejected: let the blueprint prove the relaxed rule safe for them.
4. **R-32a**: re-deferred as proposed.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # grep SIGABRT
cargo sqlx prepare --check -- --all-targets --all-features   # after T2 and T3; migrated scratch DB
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| `htui-orch` tests near the 2 MiB stack | M | box large futures; `--no-fail-fast`, grep SIGABRT |
| `.sqlx` regeneration needs a migrated scratch DB | M | `docs/hr-sandbox.md` recipe; `psql -h localhost -p 5439` |
| T3 churns many orch assertions that expected `None` on a pass | H | the compiler doesn't find them; run the orch suite and fix each, one commit |
| Two lanes in the same repo couple through the build or `.sqlx` | M | lane B in its own worktree; it needs no `.sqlx`; final gate on the merged tree |
| MemStore truncation breaks a test comparing raw `Utc::now()` | M | T1 fixes them in its own commit |
| R-30's relaxed rule adopts a failing candidate | L | candidates keep the old rule unless the blueprint proves otherwise |

## Acceptance
- [ ] Every task's failing test was red before its fix
- [ ] Validation passes on the real tree, `--test-threads=1`
- [ ] Reviewer gate run over the full change set
- [ ] HANDOFF R-lines updated: closed, or re-deferred with a reason

---
*Status: DONE (2026-10-02), commits `1d1118f3`..`7df69d33`.*

## As built
- **C0** (`1d1118f3`): `GateContext` carries the walk's `StepFence`. It also touched
  `crates/htui-orch/tests/review_loop.rs`, which the blueprint missed.
- **Lane B** ran in a worktree and was merged as `1e50fa91`:
  - T5 / R-30 (`d4be38ec`);
  - T6 / R-32b (`29bcc594`);
  - T4 / R-31 (`8640d4d4`).
- **Lane A** ran on the primary tree:
  - T1 / R-29 (`19dc3af9`);
  - T2 / R-6 (`52f3e509`, `5bc4372f`, `de918596`);
  - T3 / R-5 (`2ece53cc`, `a2cf16f4`, `033625f6`).
- **R-31 side effect (blueprint claim 2), accepted by the maintainer:** `Unblock` on a followed
  review-loop escalation now resumes it and re-runs the loop. It is pinned by
  `unblock_on_a_followed_escalation_reruns_the_loop_and_escalates_again`.
- **R-32a** is re-deferred, with its reason on the HANDOFF line.
- **Review** (rust-reviewer): approve. There was no BLOCKER, HIGH or MEDIUM finding.
  - Applied: L1 (`e4f49308`, the hand-back of a crashed rejection), L2 (`020dfc85`, the worker
    `Unblock` verdict), L3 (`c02e7508`, the fence answered before the status on both stores) and
    N1 (`7df69d33`, MemStore's park is all-or-nothing).
  - Left: NIT 3 (`unblock_enabled`'s bare `bool`) and NIT 4 (the `Vec<String>` copy in the
    `phase_agent` insert).
  - Noted on HANDOFF: NIT 2 (chat runs keep their timestamps untruncated).
