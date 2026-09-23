# Blueprint: MOD-4 milestone 5, "two runs do not collide, and a crash is survivable"

**Status**: **ACCEPTED by the maintainer 2026-09-23.** Every finding F-A..F-W is accepted with the fix in its row. Every proposed change A-1..A-8 is accepted, so wherever this document says "under A-n" or "when A-n is accepted", that branch is the one to build. Consequences: A-8 supersedes F-C's rewrite (`an_interrupted_step_out_of_budget_parks` asserts `RetryStep` → `Retried`, and R-21 is closed); A-1, A-4 and A-5 mean R-24, R-22 and R-23 are not incurred; A-3 means T7/T8 assert exactly one reconcile per recovered winner. Implementation in progress.

**Plan**: `.claude/plans/mod-4-orch-lease.plan.md`, confirmed by the maintainer on 2026-09-23. It covers D79–D106, and OQ-1..OQ-11 take their adopted defaults. **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 5 (`:309`). Where PRD D1–D8 disagree with this blueprint, D1–D8 win. **Design authority**: the ANA-2 passages cited in the plan's header, chiefly §4.7 (`docs/ANA-2.md:1009-1113`) and §4.9 (`:1242-1340`).

**Verified at**: HEAD `d9dee1e` on branch `mod-4-m5`. `git diff --stat d854ff1 HEAD -- crates/` is empty: the one commit since the plan's base touches only `.claude/plans/`. Every `crates/` citation in the plan therefore still resolves, and I re-opened each one this blueprint relies on. **Line numbers are pre-edit.** A citation into a file that a task edits moves after that task's first commit.

**Graphify**: I read `graphify-out/GRAPH_REPORT.md` first. It was built from `3e346107` (`GRAPH_REPORT.md:12`) and has no `htui-orch` community. Its `PgStore`, `MemStore` and store-conformance hubs predate `claim_run`, `refresh_lease` and `adopt_runs`. So it gave orientation only. Every `htui-core`, `htui-store`, `htui-agent` and `htui-orch` fact below was read from the tree at `d9dee1e`.

**Scope**:
- Order: T1 runs alone. Then T3 ∥ T5, each in its own git worktree, merged T3 then T5. Then T2 → T4 → T6 → T7 → T8.
- New modules: `htui-core/src/model/overlap.rs`, `htui-orch/src/overlap.rs`, `htui-orch/src/recover.rs`.
- Seam changes: one changed `WriteStore` signature (`claim_run -> Result<Claim>`), two new writers (`take_lease`, `interrupt_step`) and one narrowed contract (`adopt_runs` never adopts the sweeper's own lease).
- Two new `Isolator` verbs.
- **No migration, no `schema_version` move, no `cache_migrations` file, no `.snap` change, no `Cargo.toml`/`Cargo.lock` change. `.sqlx` does change.**

**House style (carried)**:
- One named free function per refusal sentence, and `Display`-exact vocabularies.
- Every instant comes from `Clock`.
- No `std` guard is held across an `.await`, and no `gix::Repository` crosses one.
- The only `Command::new` calls under `crates/htui-orch/src/` are in `isolate/git.rs` and `verify.rs`.
- `#![warn(missing_docs)]`.
- Implementers commit incrementally with `git add <own paths>` only (never `-A`/`-a`, never `stash`). Every commit compiles.
- Every gate is verified with `--test-threads=1`.

---

## 0. Findings against the plan

**Blocker** means the plan, read literally, produces a case that cannot pass, a build that cannot compile, or a state that cannot be recovered. The fix column is what the implementer follows unless the maintainer overrules it.

| # | Blocker? | Plan says | Tree at `d9dee1e` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (two T7 cases, one T8 case) | T7's `a_crash_before_reconcile_is_reconciled_on_adoption` and `a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled` script `FakeIsolator::fail_reconcile` to stand in for the crash. | A failed `reconcile` goes `reconcile_done_step` → `park_run` (`engine.rs:2878-2887`), so the run becomes `awaiting_approval`. Both stores' `adopt_runs` adopt only `status = 'running'` (`mem.rs:3313`, `pg/write.rs:2663`). The run is never adopted, D97 never runs, and neither case can pass. | Simulate the crash with a reconcile that **never returns**. `FakeIsolator::stall_nth_reconcile(n)` makes the n-th `reconcile` call (1-based) await forever. It is driven by A-2's stall harness, and the old process's future is dropped there. T8's lost merge uses a test-local wrapper `Isolator` that delegates to `GixIsolator` and stalls **after** the real merge (§10). |
| **F-B** | **Blocker** (T8) | D100: a case polls `dispatch` once with `now_or_never` because "the fakes never suspend otherwise". | `FakeIsolator::prepare` does `tokio::task::yield_now().await` for `shared_serialized` (`fake.rs:308`). `GixIsolator` suspends at every `spawn_blocking` and every `git` child. Under T8, `now_or_never` returns `None` inside stage 2 and leaves a `running` step with **no tree rows**, which is not the stalled state any T8 case needs. | A-2: the stalling sink signals a `tokio::sync::Notify` (the `sync` feature is already on, `htui-orch/Cargo.toml`). The case races the dispatch future against `notified()` with `futures::future::select` and drops the dispatch future on `Either::Right`. This is mandatory for T8. For T7 it is A-2's proposal. If A-2 is declined, T7 keeps `now_or_never` and must avoid `shared_serialized` stalls (H-6). |
| **F-C** | **Blocker** (one T7 case) | `an_interrupted_step_out_of_budget_parks` says "`retry_limit = 0` … `RetryStep` then resumes it". D94: "`retry_enabled` accepts `failed`". | `retry_enabled` also requires `may_attempt(step.attempt + 1, phase.retry_limit)` (`command.rs:412-418`). With `retry_limit = 0` and `attempt = 1` that is `2 <= 1`, which is false, so the answer is `RetryExhausted`. The limit comes from the immutable snapshot, so no live edit raises it. | Rewrite the case to assert `RetryStep` → `Err(RetryExhausted { .. })`, and then `CancelRun` → `Cancelled` with the item back at `open`. Record **R-21** (an out-of-budget interruption has no resume verb until milestone 6). A-8 offers the alternative. D94's `RetryStep` claim does hold for D93's dirty-tree park, which parks with budget left. |
| **F-D** | Non-blocker | 18b: "admits attempt 2 whose `before_hash` equals attempt 1's". | `FakeIsolator::prepare` mints `fake:base:{tick}` per call (`fake.rs:325-327`). `restarted()` builds a fresh `FakeIsolator` whose counter starts again at 1. So attempt 2's base either differs, or equals attempt 1's by coincidence (both `fake:base:1`). An equality assertion there proves nothing. | T7's 18b asserts `resets() == 1` on the restarted isolator, one `interrupt_step`, and an attempt-2 row walked to `done`. Base equality is asserted only over real git (T8's `criterion_18_an_unfinished_worktree_step_is_retried_from_the_same_base`). |
| **F-E** | **Blocker** (T2 case) | `claim_run_applies_the_isolation_and_path_rules`: two isolated runs are admitted, then a third on `src/lib/` is `Overlaps { Paths }` … | The fixture box allows two concurrent runs (`fixtures.rs:433`: `max_concurrent_items: 2`). D83 decides `SlotFull` **before** `Overlaps`. After two admissions, the third reads `SlotFull`. Also, `new_run` stamps `seam_clock()` on every run (`conformance.rs:3792`), so `queued_at` ties and "names the first" rests on `RunId` ordering. | Park the two admitted runs (`transition_run(Running → AwaitingApproval)`) before the overlap legs. This also exercises criterion 16's "parked still overlaps, holds no slot". Give each run a distinct `queued_at` (`at + i s`). §6.5 gives the full leg order. |
| **F-F** | Non-blocker | `pg_criteria.rs::a_claim_and_a_take_do_not_both_win_a_parked_run`. | `claim_run` answers `NotClaimable` for any non-`queued` run before a lock matters (`mem.rs:3225`, `pg/write.rs:2503`). `take_lease` acts only on `running \| awaiting_approval`. A claim and a take can never race on one row, so the case would pass vacuously. | Replace it with `two_takes_of_one_released_lease_admit_one`: two pools, a parked run whose lease was released, and exactly one `true`. |
| **F-G** | **Blocker** (compile, T6) | T6's `lib.rs` re-exports `overlap::resolve`. | `lib.rs:45` already re-exports `graph::resolve` at the crate root, so a second `resolve` is `E0252`. | No root re-export. Callers write `htui_orch::overlap::resolve`. |
| **F-H** | Non-blocker | D99's trait verb is `release(run)`. | Both implementors already have an **inherent** `release(&self, step: StepId)` (`real.rs:494`, `fake.rs:241`). It is legal, because inherent methods win method-call resolution, but `self.release(x)` would then mean two different things in one file. | T3's first commit renames the inherent one to `release_step`. There are four call sites: `real.rs:1234`, `:1258`, `fake.rs:310`, `:360`. |
| **F-I** | **Blocker** (T3 fake) | D99: the fake's `release` "drops its single `serial` guard for the run's steps". | `FakeIsolator::held` is `BTreeMap<StepId, OwnedMutexGuard<()>>` (`fake.rs:92`), so there is no way to find a run's steps. | Re-key it `BTreeMap<(RunId, StepId), _>`. `prepare` already receives `run`. `release_step` removes by the step half and `release(run)` by the run half. |
| **F-J** | Non-blocker | T7's action: "the `Orchestrate` trait gains `sweep`, `claim`, `restarted`, `stall_after_done`". | T6's cases already call `Engine::claim` (15a) and `restarted()` (D87's two cases), and they need the new owner to assert ownership. `FakeOrchestrator::owner` exists (`fake.rs:1268`) but is not on the trait. | T6 adds `claim`, `restarted` and `owner` to `Orchestrate`. T7 adds `sweep` and `stall_after_done`. |
| **F-K** | Non-blocker | T7's file list omits `lib.rs`. | T7 creates `pub` `Adopted`, `Next` and `sweep_fake`, and milestone 6 reads them from the crate root, as it does `Resume` (`lib.rs:37-38`). | Add `crates/htui-orch/src/lib.rs` to T7's set. It is serial, so there is no conflict. |
| **F-L** | Non-blocker | T4 regenerates both JSON fixtures, and `tests/fixtures.rs` "asserts their `topology` equals `FEATURE_TOPOLOGY`'s pin". | `feature_with_verify_snapshot_matches` compares `topology` only (`tests/fixtures.rs`, last assertion), so that file decodes unchanged with `scope` defaulted. `FEATURE_TOPOLOGY` is private to `graph.rs`'s test module (`:746`), and an integration test cannot name it. | Regenerate **only** `feature.snapshot.json`: add the `"scope"` key and nothing else. `feature-with-verify.snapshot.json` stays byte-identical (C-3). `feature_snapshot_matches` already asserts the whole snapshot. Keep the `topology` line in the file unchanged and let `graph.rs:885`'s pin keep guarding it. |
| **F-M** | Non-blocker | D86: `walk_leased` wraps "the eight `run_to_rest` call sites". | Between `unpark` and `run_to_rest`, `answer_gate` runs `after_rejection` → `review_loop` or `reconcile_done_step` (`engine.rs:519-531`), and `select_fanout` runs `reconcile_done_step` (`:757-760`). Those writes land on a `running` run with no heartbeat. | `walk_leased` wraps the **whole post-unpark tail** of each command. That is a superset of the eight sites (§8.3). |
| **F-N** | **Blocker** (correctness) | D87: "the four `unpark` call sites take [the lease] before `unpark`". | Four commands write **before** `unpark`: `answer_gate` (`:510`), `retry_step` (`:636`), `retry_group`'s `retire_slot` (`:696`), and `select_fanout` (`:731`, note `:744`). If `take_lease` then answers `LeaseHeld`, the gate answer is already written, the run is still parked, and `answer_gate_enabled` refuses the now-`done` step forever. That is an unrecoverable state. | D108: `take_lease` runs after the pure guard (`*_enabled`) and before the command's **first** write. |
| **F-O** | Non-blocker | `stall_after_done(phase, attempt, write_output)`. | `an_interrupted_candidate_fails_alone` stalls candidate 1 of a 3-way group, and `an_interrupted_judge_parks_for_selection` stalls the judge (`research:judge`, index −1). A `(phase, attempt)` key cannot address either. | Key the stall by `ScriptKey`, `(phase, attempt, Option<(fanout_index, call)>)` (`fake.rs:1004`), exactly as `script_candidate` does. |
| **F-P** | Non-blocker | D98's order: adjudicate running steps (D90–D95), then D96, then D97. | When D91 re-settles a step to `Landing::Advance`, it has already run `reconcile_done_step`. D97 then finds the same step as the frontier and reconciles it again. That is idempotent (M3 H-3), but T8's "merges it once" would observe two `reconcile` calls. | A-3: compute the frontier on the rows **as adopted**, before any adjudication. |
| **F-Q** | Non-blocker | Carried table: "`engine.rs:1590-1595`'s doc is updated". | Other sentences promise things D98 does not do. A failed cleanup is "retried by milestone 5's sweep" (`engine.rs:2942`, `:2955` warn text, `real.rs:1355`), but the sweep adopts only `running` runs, so a terminal run's failed cleanup is never retried. `engine.rs:935-936` says the rest of the sweep "is not" cut here. `engine.rs:2851-2852` offers "milestone 5's sweep or milestone 6's". `gate.rs:935` says "R-3 for milestone 5". | T6 rewrites `engine.rs:935-936`, `:1588-1595`, `:2851-2852`, `:2942` and `:2955` to name milestone 6. T3 rewrites `real.rs:1355`. `gate.rs:935` is outside every file set; record it for close-out. Recorded as **R-25**. |
| **F-R** | Non-blocker | D90's commit half; T7's 18a "`record_commits` an `after_hash`". | The demo fixture seeds no repo (`conformance.rs:615-617`), so `run.repo_scope` is empty, there are no tree rows and nothing to put an `after_hash` on. | Every T7 case that reads the commit half or a tree row calls `primary_repo(&orch)` (`conformance.rs:618`) before `StartRun`. |
| **F-S** | Non-blocker | 18a: "an ungated phase is `done` and the run walks on to `done`". | Every seeded phase is `gate: Always`, as M4 F-E found. | The case repoints every phase of its graph to `Gate::Never` (`conformance.rs:455`'s `repoint`), and its gated leg keeps `prd` at `Always`. |
| **F-T** | Non-blocker | D92 labels `htui/<step_id>` through `git::create_branch`. | `create_branch` is `PreviousValue::MustNotExist` (`git.rs:1337-1348`). It tolerates a label already at the same target (`real.rs:895-897`; test `git.rs:1916-1930`) and refuses one elsewhere. `capture_in_place` writes that label for `shared_serialized` (`real.rs:893-905`). | D114: read `branch_target` first. Absent → create. At `HEAD` → report it. Elsewhere → the refusal `label_conflict: …`, with nothing written, and the engine takes D93's path. |
| **F-U** | Non-blocker | D85: `LeaseTimes::from_app` with fallbacks. | `graph::app_positive` (`graph.rs:488`) is private, and `graph.rs` is T2/T4's. | T5 writes its own two-line reader in `recover.rs` with the same rule (a positive `i64`, else silence) and does not touch `graph.rs`. |
| **F-V** | Non-blocker | D88's store leg "goes after the 'expired lease adopted' leg at `:4307-4316`". | After `:4316` the lease is `second_owner`'s. The cleanest point for "own lease not adopted, stranger's sweep adopts it" is after `:4330` ("and the sweeper holds it"). | Insert after `conformance.rs:4330` and before the no-box leg `:4331-4338` (§6.5). The `mem.rs` twin gets the same leg after `mem.rs:6164`. |
| **F-W** | Non-blocker | D98 gives no rule for one adopted run's recovery failing. | Returning `Err` on the first failure leaves the other adopted runs under this process's lease. D88 means this process's own sweeps never re-adopt them, so they stall until a restart. | A-4. |

---

## 0b. Architect's proposed changes

Each item departs from the plan's letter and goes to the maintainer before dispatch. The default column is what the implementer builds if the maintainer is silent. For A-2 and A-3 that is the proposal, because F-B and F-P make the plan's letter fail.

| # | Change | Reason | If declined |
|---|---|---|---|
| **A-1** | `Engine::resume` calls `take_lease` first. `Ok(false)` → `LeaseHeld`, and nothing is resolved or walked. | `resume` is the path milestone 6 runs for every adopted run, and it is also callable on any run. Without a take, `resume` on a `running` run whose lease a **live** stranger holds walks it concurrently until the first beat, up to `lease_refresh_seconds` = 60 s, while two processes write one run. For an adopted run the take is a renewal, and for a released parked run it is harmless: the walk rests at once and is released. | Recorded as **R-24**; `resume` stays as in D86. |
| **A-2** | **One stall harness for T7 and T8.** `FakeOrchestrator::stall_after_done(key, write_output) -> Arc<Notify>` and a helper `until_stalled(fut, &notify)`, which uses `futures::future::select` on two `Box::pin`ned futures and drops the dispatch future on `Right`. It replaces D100's `now_or_never`. `FakeIsolator::stall_nth_reconcile(n) -> Arc<Notify>` uses the same shape. | F-B makes this mandatory for T8. For T7 it removes the `shared_serialized` blind spot (H-6) and gives both binaries one crash model. There is no new dependency: `Notify` is `tokio`'s `sync` and `select` is `futures`'. | T7 keeps `now_or_never` with H-6's exclusion. T8 still uses the signal (F-B). |
| **A-3** | **D97 before D90–D95.** `recover_run` computes `recover::frontier` over the rows as `adopt_runs` found them, and only when no step is `running`. It then reconciles that frontier before any step is adjudicated. | A live `running` row at position > *p* already makes the frontier `None` under D97's own definition. So the frontier exists exactly when there is nothing to adjudicate, and the double reconcile of F-P disappears. That gives one reconcile call per recovered winner, which T8 asserts. | The plan's order; T7/T8 assert "at least once" and the merge count. |
| **A-4** | **A per-run sweep failure does not abort the sweep.** `Next::Error(String)` (the error's `Display`), a `tracing::warn!`, an `item_note` `sweep could not recover run <id>: <err>`, and the lease **released** (`refresh_lease(run, owner, now)`). The sweep then continues with the next run. | D88 forbids this process from re-adopting its own lease. Releasing makes the run adoptable by any **other** process at once. For this process, milestone 6 can call `resume` (under A-1). Aborting would strand every later run in the batch. | `sweep` returns the first `Err`; recorded as **R-22**. |
| **A-5** | **`adopt_runs` locks candidates with `FOR UPDATE SKIP LOCKED`**: `WITH candidates AS (SELECT id FROM run WHERE executing_box_id = $1 AND status = 'running' AND (lease_expires_at IS NULL OR lease_expires_at <= $3) AND lease_owner IS DISTINCT FROM $2 ORDER BY queued_at, id FOR UPDATE SKIP LOCKED), swept AS (UPDATE run r SET … FROM candidates c WHERE r.id = c.id RETURNING r.*) SELECT … FROM swept ORDER BY queued_at, id`. | T2 rewrites this query anyway (D88) and regenerates its `.sqlx` anyway. Today's plain `UPDATE` makes two concurrent sweeps block on each other's row locks, and correctness then rests on READ COMMITTED's re-check. Deterministic lock order plus `SKIP LOCKED` means two sweeps never wait on each other and cannot deadlock. That makes `two_sweeps_adopt_each_expired_run_once` stable by construction. | The plain `UPDATE` plus the D88 clause; recorded as **R-23**. |
| **A-6** | **A `local` tree whose `HEAD` moved is not reset.** `GixIsolator::reset` refuses a `local` row with `HEAD != base_ref` (`local_moved: <path> at <head>, before_hash <base>`), and the engine takes D93's path. `shared_serialized` keeps D92's label-then-reset. | In `local` mode the checkout is the maintainer's own working tree and branch. OQ-7's own reasoning ("edits made during the run may be the maintainer's") applies equally to **commits**. The label keeps them reachable, but `reset --hard` still moves the maintainer's branch backwards under them. `shared_serialized` checkouts are `htui`-managed. | D92 as written, for both modes. |
| **A-7** | `recover_run` runs inside `walk_leased`, so a stranger taking the lease mid-adjudication abandons the recovery like any walk. | "Every write under a lease runs under a heartbeat" becomes one rule. The sweep's writes include a `git reset --hard` and possibly a merge, each seconds long. The cost is one extra `select` per adopted run. | Recovery writes run unleased for their few seconds. |
| **A-8** | **A human `RetryStep` on an interrupted step ignores the budget.** `retry_enabled` admits a `failed` step whose `gate_note` starts with `interrupted` even when `may_attempt` is false (T6 edits `command.rs`). | D92 itself says an interruption "is not the agent's failed settle". The budget bounds automatic retries, and a human choosing to retry after a crash is not one. This closes R-21 without a new verb. | F-C's rewrite; R-21 stands until milestone 6. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum; each compiles) | Gate |
|---|---|---|---|
| T1 overlap predicate | htui-core | 2: (a) types, signatures with `todo!()` bodies, all tests (red); (b) bodies (green) | `cargo test -p htui-core --all-features --lib model::overlap -- --test-threads=1`; `cargo clippy -p htui-core --all-targets --all-features -- -D warnings`; `cargo build -p htui-orch --all-features` |
| T3 `reset`/`release` | htui-orch (worktree) | 4: (a) F-H rename (green); (b) trait verbs + `ResetReport` + stub impls + tests (red); (c) `GixIsolator` per mode; (d) `FakeIsolator` (F-I re-key, counters, scripting) | `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`; C-1 |
| T5 `recover.rs` | htui-orch (worktree) | 4: (a) module + `lib.rs` line and doc + `LeaseTimes` + `heartbeat`; (b) `classify`; (c) `resettle` + `verify_of`; (d) `frontier` | `cargo test -p htui-orch --all-features --lib -- recover:: --test-threads=1`; clippy as T3 |
| merge | — | T3 into `mod-4-m5`, then T5 | after **each** merge, on the real tree: `cargo test -p htui-orch --all-features -- --test-threads=1` and clippy |
| T2 the seam | htui-core, htui-store, htui-agent, htui-orch | 5: (a) `GraphSnapshot.scope` + five literals; (b) `claim_run -> Claim` + the predicate + 31 call sites + case 1 + pins 50 + `.sqlx`; (c) `take_lease` + case 2 + pins 51 + `.sqlx`; (d) `interrupt_step` + D88 + case 3 + pins 52 + `.sqlx`; (e) `pg_criteria` races | §6.7 |
| T4 resolution | htui-orch | 2: (a) `overlap.rs` + `lib.rs` line + tests; (b) `graph::resolve` wiring, `UnknownTouchedRepo`, `resolve_scope` removal, fixture JSON | `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; C-3 |
| T6 lease + claim | htui-orch | 4: (a) `command.rs` errors and bytes; (b) `LeaseTimes` wiring, `LEASE_SECONDS` removed; (c) `walk_leased`, `take_lease`, release, `claim`, `resume`, doc sentences; (d) harness, `Orchestrate`, six `CASES`, pins 42, re-exports | as T4, plus `cargo doc -p htui-orch --no-deps --all-features` |
| T7 the sweep | htui-orch | 4: (a) `RunFailure::Interrupted`; (b) stall and reconcile-stall harness + `Orchestrate`; (c) `Engine::sweep`, `recover_run`, `park_interrupted`, `sweep_fake`, `lib.rs`; (d) ten `CASES`, pins 52 | as T6, then the workspace gate (§15) |
| T8 real git | htui-orch tests | 2: (a) stalling sink, stalling isolator, second-process helper, criterion 12, 18-unfinished, shared reset; (b) 18-finished, lost merge | `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`, then the workspace gate |

Every task writes its tests first, and the first failing test is named in each section. A red commit must still **compile**. Unimplemented bodies are `todo!()`, and `clippy::todo` is not in `clippy::all`.

---

## 2. Wave shape and the compile-coupling contract (D105, D106)

**Disk first.** Two worktrees mean two `target/` directories, and project memory records `target/` as the cause of the dev-Postgres crash loop. `df -h /` read 82 GB free (82 % used) when this blueprint was written. Re-check before dispatching Wave A′.

| Task | Owns | May **not** |
|---|---|---|
| T1 | `crates/htui-core/src/model/overlap.rs` (new), `crates/htui-core/src/model/mod.rs` | write a prefix parser (D104); touch `model/run.rs` (T2 adds `GraphSnapshot.scope`); name `GraphSnapshot` |
| T3 | `crates/htui-orch/src/isolate.rs`, `isolate/real.rs`, `fake.rs` (`FakeIsolator` only) | touch `lib.rs` (D106: `ResetReport` is re-exported by T6); touch `isolate/git.rs` (no new verb, D99); call `reset`/`release` from `engine.rs` (T6/T7) |
| T5 | `crates/htui-orch/src/recover.rs` (new), `crates/htui-orch/src/lib.rs` (the `pub mod` line and the doc sentence at `:15-16` only) | name `RunScope`, `Claim`, `claim_run`, or a `GraphSnapshot` struct literal (D106; snapshots are decoded from JSON); touch `graph.rs` (F-U); call any store |

**Build coupling, re-checked:**
- **T1 → everything**: T1 adds a module and re-exports only. Nothing that compiles today stops compiling. That is why it runs alone and first, and why its gate includes `cargo build -p htui-orch`.
- **T3**: two trait methods with **no default body**. `grep -rn 'impl.*Isolator for' crates/` finds exactly `real.rs:1214` and `fake.rs:280`, both in T3's set. No caller exists until T6.
- **T5**: its only non-`recover` imports are `crate::gate::{settle, Settle, SettleInput}` (`gate.rs:239`), `crate::isolate::Clock`, `crate::verify::VERIFY_CLASS` (`verify.rs:42`), and `htui_core::model` rows that T2 does not change (`RunStep`, `RunStepTree`, `RunStepCommit`, `CommandRun`, `Document`, `SnapshotPhase`). T3 edits `isolate.rs` in the other worktree but does not change `Clock`.
- **`.sqlx`, `tests/fixtures/*.json`, `CASES` pins**: T2 only; T4 only; T2 (store) and T6/T7 (orch) only. Never concurrently.
- **`crates/htui`**: no task touches it. `grep -rn 'claim_run\|GraphSnapshot {' crates/htui/` is empty.

**Worktree rule.** A Wave A′ gate that fails in the other task's file is not yours to fix. After each merge, re-run the `htui-orch` gate on the real tree before starting the next task (project memory: `parallel-fanout-hidden-file-coupling.md`).

---

## 3. T1: the overlap predicate, pure (D79, D80, D83, D104)

**First failing test**: `rule_p_two_isolated_runs_overlap_on_intersecting_prefixes`.

### 3.1 `crates/htui-core/src/model/overlap.rs`

```rust
//! ANA-2 §4.7's overlap predicate (`docs/ANA-2.md:1059-1089`), pure, shared by both stores'
//! admission (plan D80). The prefixes are `prompt::excerpt::PathPrefix::prefix` strings (D104):
//! this module parses nothing.
use std::collections::BTreeMap;
use core::fmt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::model::{RepoId, RunId};

/// Plan D79: `GraphSnapshot.scope`. Keys equal `run.repo_scope`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunScope {
    #[serde(default)]
    pub repos: BTreeMap<RepoId, RepoScope>,
}

/// One repo of a [`RunScope`]. Every field defaults to the conservative value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoScope {
    /// Every phase is `worktree | copy` for this repo (`:1053-1055`).
    #[serde(default)] pub isolated: bool,
    /// Any phase is `local` (`:1046`).
    #[serde(default)] pub local: bool,
    /// `PathPrefix::prefix` values; **empty = unknown = the whole repo** (`:1072`).
    #[serde(default)] pub prefixes: Vec<String>,
}

impl RunScope {
    /// D80: every repo of `repo_scope`, `isolated = false`, `local = false`, `prefixes = []`,
    /// which is exactly the pre-milestone-5 "any shared repo overlaps".
    #[must_use] pub fn conservative(repo_scope: &[RepoId]) -> Self;
}

/// Rules L, I, P in §4.7's order (`:1063-1066`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlapRule { Local, NotIsolated, Paths }

/// §4.7's `overlaps(A, B)`: for each repo of the intersection in `RepoId` order, check L, then I,
/// then P, and return the first hit. `None` = no overlap.
#[must_use] pub fn overlaps(a: &RunScope, b: &RunScope) -> Option<OverlapRule>;

/// D80, D109: decode `snapshot["scope"]`. If it is absent, `null` or undecodable, the answer is
/// [`RunScope::conservative`]`(repo_scope)`. A decoded scope also gains a conservative entry for
/// every `repo_scope` repo it lacks (the safe direction).
#[must_use] pub fn scope_of(snapshot: &Value, repo_scope: &[RepoId]) -> RunScope;

/// Plan D83: `WriteStore::claim_run`'s verdict.
#[must_use = "a refused claim wrote nothing; the caller must act on why"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Admitted,
    /// Not `queued`, or `target_box_id != box`.
    NotClaimable,
    SlotFull { running: u64, limit: u32 },
    /// The first overlapping live run in `(queued_at, id)` order.
    Overlaps { with: RunId, rule: OverlapRule },
}
impl Claim { #[must_use] pub const fn is_admitted(&self) -> bool; }
```

`intersect(xs, ys)` is private: `xs.is_empty() || ys.is_empty() || xs.iter().any(|x| ys.iter().any(|y| x.as_bytes().starts_with(y.as_bytes()) || y.as_bytes().starts_with(x.as_bytes())))`. Bytes, no case folding (`excerpt.rs:84-89`'s rule).

**`Display`, byte-exact (D109):**

| Value | Bytes |
|---|---|
| `OverlapRule::Local` / `NotIsolated` / `Paths` | `local` / `not_isolated` / `paths` |
| `Claim::Admitted` | `admitted` |
| `Claim::NotClaimable` | `not claimable` |
| `Claim::SlotFull { running, limit }` | `box full ({running} of {limit} running)` |
| `Claim::Overlaps { with, rule }` | `overlaps run {with} ({rule})` |

### 3.2 `model/mod.rs`

Add `pub mod overlap;` in alphabetical order (after `note`) and `pub use overlap::{Claim, OverlapRule, RepoScope, RunScope};`. `overlaps` and `scope_of` stay path-qualified (`model::overlap::overlaps`), the way `quota::available` is re-exported by name: add them to the `pub use` line too, for the stores' convenience.

### 3.3 Tests (all pure, `#[cfg(test)] mod tests` in `overlap.rs`)

| Test | Asserts |
|---|---|
| `prefixes_come_from_the_excerpt_path_prefix` | Over `crate::prompt::excerpt::PathPrefix::parse(_, "htui").prefix`: `src/**/*.rs` → `src/`; `crates/htui-core/src/model/item.rs` whole; `**` → `""`; `src/{a,b}` → `src/`; `a?b` → `""`. The rule is stated in the doc comment. |
| `disjoint_repos_never_overlap` | Repos {r1} vs {r2}, both `local`, give `None`. |
| `rule_l_local_overlaps_even_when_the_other_is_isolated` | `Some(Local)`. |
| `rule_i_shared_serialized_overlaps_an_isolated_run_on_the_same_repo` | `isolated: false` vs `isolated: true`, disjoint prefixes, give `Some(NotIsolated)`. |
| `rule_p_two_isolated_runs_overlap_on_intersecting_prefixes` | `["src/"]` vs `["src/lib/"]` give `Some(Paths)`. |
| `two_isolated_runs_with_disjoint_prefixes_do_not_overlap` | `["src/"]` vs `["docs/"]` give `None` (criterion 15's parallel half). |
| `an_empty_prefix_list_overlaps_everything_in_its_repo` | `[]` vs `["docs/"]` give `Some(Paths)` (`:2126-2127`). |
| `a_double_star_is_the_same_as_no_declaration` | `[""]` vs `["docs/"]` give `Some(Paths)`. |
| `rules_are_reported_in_l_i_p_order` | In one repo that trips all three, `Local`. In one that trips I and P, `NotIsolated`. With two repos (r1 < r2), r1 trips P and r2 trips L, giving `Paths`: the per-repo loop returns at the first repo. |
| `scope_of_a_snapshot_without_scope_is_conservative` | `json!({"v":1})` with `[r]` gives `{r: default}`, which overlaps any scope holding `r` via `NotIsolated`. |
| `scope_of_an_undecodable_scope_is_conservative` | `{"scope": 7}` and `{"scope": {"repos": 3}}` give conservative. `{"scope": null}` gives conservative. |
| `scope_of_fills_a_repo_the_scope_forgot` (added, D109) | `{"scope":{"repos":{}}}` with `[r]` gives `{r: default}`. |
| `claim_display_names_the_rule_and_the_holding_run` | Every row of the table above. |

---

## 4. T3: `Isolator::reset` and `release` (D92, D93, D99)

**First failing test**: `reset_labels_then_resets_a_clean_shared_checkout`.

### 4.1 `isolate.rs`

```rust
/// Plan D92/D99: what [`Isolator::reset`] did. **`refused` non-empty ⇒ nothing was reset.**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// `(repo, head)`: `htui/<step>` names `head` after the call, written now or found there.
    pub labelled: Vec<(RepoId, String)>,
    /// `(repo, reason)`: `dirty_tree_not_reset: <path>` or `label_conflict: …` (D114).
    pub refused: Vec<(RepoId, String)>,
}

// in `trait Isolator`, after `cleanup`:
/// ANA-2 §4.9 `:1298` as plan D92 reads it. `worktree`/`copy` rows are never touched (OQ-8).
/// `shared_serialized`/`local` rows are checked **all first** (OQ-7: live dirt refuses), then
/// labelled when `HEAD` moved, then `reset --hard <base_ref>`.
fn reset<'a>(&'a self, step: StepId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ResetReport>;
/// D99: drop every guard any step of `run` holds and touch no tree; an abandoning walk's release.
fn release<'a>(&'a self, run: RunId) -> IsolatorFuture<'a, ()>;
```

Rewrite the trait doc's "four verbs" (`isolate.rs:116`) to read "ANA-2 §4.6's four verbs plus milestone 4's two reads and milestone 5's two recovery verbs".

### 4.2 `isolate/real.rs` (D114)

- **F-H**: `fn release(&self, step)` (`:494`) becomes `fn release_step`, and its call sites at `:1234` and `:1258` follow.
- New refusal: `#[must_use] pub fn label_conflict(label: &str, target: &str, head: &str) -> String` → `label_conflict: {label} names {target}, HEAD is {head}`. Under A-6, also `pub fn local_moved(path: &Path, head: &str, base: &str) -> String` → `local_moved: {path} at {head}, before_hash {base}`.
- `reset` algorithm:
  1. For every row with `mode ∈ {SharedSerialized, Local}`, reading `tree.path`:
     - Read `dirty = blocking(git::is_dirty(path))?` (`git.rs:1263`). If dirty → `refused.push((repo, dirty_tree_not_reset(path)))` and go to the next row.
     - Read `head = blocking(git::head(path))?`. If `head == tree.base_ref`, nothing is planned for the row.
     - Otherwise (A-6 accepted and `mode == Local`) → `refused.push(local_moved(..))`.
     - Otherwise read `label = blocking(git::branch_target(path, "htui/<step>"))?`:
       - `Some(t) if t != head` → `refused.push(label_conflict(..))`;
       - otherwise plan `(repo, path, head, create: label.is_none())`.
  2. If `refused` is non-empty → return the report. **No write has run.**
  3. For each planned row:
     - if `create`, run `git::with_retry("create branch", || blocking(create_branch(path, "htui/<step>", head)))` (the `capture_in_place` pattern, `real.rs:899-904`);
     - `labelled.push((repo, head))`;
     - `git::with_retry("reset --hard", || git.reset_hard(path, &tree.base_ref))` (`real.rs:1114`'s pattern), where `git = self.cli()?.clone()`.
  4. `worktree`/`copy` rows are skipped without reading their path, so a vanished tree is not an error.
- **No guard is taken.** Rule I makes a concurrent in-process holder of the same `(box, repo)` impossible (D80), and waiting on one inside the sweep could hang it.
- `release(run)` is `Box::pin(async move { self.release_run(run); Ok(()) })` (`:513`).
- The `real.rs:1355` doc sentence (F-Q) is rewritten: "…a tree nobody removes; no sweep retries a terminal run's cleanup (milestone 6)".

### 4.3 `fake.rs` (`FakeIsolator` only)

- **F-I**: `held: Mutex<BTreeMap<(RunId, StepId), OwnedMutexGuard<()>>>`.
  - `prepare` inserts `(run, step)`.
  - `release_step(step)` (renamed from `:241`, F-H) removes the entry whose step half matches.
  - `cleanup` still clears everything (`:455-458`).
- New fields: `reset_refusals: Mutex<VecDeque<String>>`, `resets: Mutex<u32>`, `releases: Mutex<u32>`.
- `pub fn script_reset_refusal(&self, reason: &str)`, `#[must_use] pub fn resets(&self) -> u32`, `#[must_use] pub fn releases(&self) -> u32`.
- `reset`: `resets += 1`. A queued refusal answers `ResetReport { refused: vec![(first row's repo, reason)], .. }`. Otherwise it answers `ResetReport::default()` and touches nothing.
- `release(run)`: `releases += 1`, then drop every `held` entry whose run half is `run`.

### 4.4 Tests

| Test | Where | Asserts |
|---|---|---|
| `reset_leaves_worktree_and_copy_trees_untouched` | `real.rs` [git] | A committed worktree tree and a copy: `HEAD`, branch and files are unchanged, the report is empty, and no `htui/` ref was added. |
| `reset_labels_then_resets_a_clean_shared_checkout` | [git] | An agent commit on the checked-out branch → `htui/<step>` names it, `HEAD == base_ref`, the commit is still reachable, and `labelled == [(repo, commit)]`. |
| `reset_finds_a_label_capture_already_wrote` (added, F-T) | [git] | A `capture` first (which writes the label), then `reset` → no error, `labelled` names it, and `HEAD == base_ref`. |
| `reset_refuses_a_label_that_names_another_commit` (added, F-T) | [git] | `htui/<step>` pre-created at `base_ref` while `HEAD` moved → `refused` holds `label_conflict: …`, and `HEAD` is unmoved. |
| `reset_of_a_shared_checkout_at_before_hash_writes_no_label` | [git] | The report is empty and there is no `htui/` ref. |
| `reset_refuses_a_live_dirty_local_checkout_and_touches_nothing` | [git-free: `local` needs no binary] | The edit survives byte-for-byte, `HEAD` is unmoved, there is no label, and `refused == [(repo, "dirty_tree_not_reset: <path>")]`. |
| `reset_is_all_or_nothing_across_repos` | [git] | Repo A is clean and moved, repo B is dirty → neither is reset or labelled. |
| `reset_never_reads_a_worktree_or_copy_path` (the plan's "vanished" case, scoped) | [git-free] | A `worktree` row whose path does not exist → `Ok(ResetReport::default())`. |
| `release_frees_a_shared_guard_without_removing_a_tree` | [git] | `prepare(run1, s1, shared)`, then `release(run1)`, then `prepare(run2, s2, shared)` on the same repo completes within `tokio::time::timeout(5 s)`. The checkout, and a worktree tree prepared for `run1`, still exist. |
| `the_fake_reset_is_scripted_and_empty_by_default` | `fake.rs` [fake] | Default is an empty report; a scripted refusal is consumed once; `resets()` counts both. |
| `the_fake_release_frees_its_serial_lock` | [fake] | A shared `prepare` for `(r1, s1)` holds the lock; `release(r1)`; a shared `prepare` for `(r2, s2)` completes within a timeout; `releases() == 1`. |

---

## 5. T5: `recover.rs`, pure (D85, D86, D90, D91, D97, D106)

**First failing test**: `heartbeat_returns_abandoned_on_a_zero_row_refresh`.

### 5.1 Surface (D116)

```rust
//! ANA-2 §4.9's lease heartbeat and the sweep's pure half (plan D85, D86, D90, D91, D97).
pub const LEASE_TTL_KEY: &str = "lease_ttl_seconds";          // `0003_orchestration.sql:135`
pub const LEASE_REFRESH_KEY: &str = "lease_refresh_seconds";  // `:136`
pub const DEFAULT_LEASE_TTL_SECONDS: i64 = 120;
pub const DEFAULT_LEASE_REFRESH_SECONDS: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseTimes { pub ttl: TimeDelta, pub refresh: std::time::Duration }
impl LeaseTimes {
    /// Positive `i64` per key, else the default; then `refresh >= ttl` → `ttl / 2`, in
    /// **milliseconds** so a 1 s TTL still beats at 500 ms.
    #[must_use] pub fn from_app(app: &BTreeMap<String, Value>) -> Self;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heartbeat { Abandoned }

/// Sleep `times.refresh` (`tokio::time::sleep`), then `refresh(clock.now() + times.ttl)`:
/// `Ok(true)` → loop; `Ok(false)` → `Abandoned`; `Err(e)` → `tracing::warn!` and loop (D86, D103).
/// Never returns otherwise.
pub async fn heartbeat<F, Fut, C>(mut refresh: F, clock: &C, times: LeaseTimes) -> Heartbeat
where F: FnMut(DateTime<Utc>) -> Fut, Fut: Future<Output = Result<bool, StoreError>>, C: Clock + ?Sized;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind { Plain, Candidate, Judge }
impl StepKind {
    /// `fanout_index == -1` → Judge; `phase.fan_out > 1` → Candidate; else Plain.
    #[must_use] pub const fn of(step: &RunStep, phase: &SnapshotPhase) -> Self;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adjudication {
    /// D90. `verify*` are what `finish_step` must write when `finished_at` is `NULL`.
    Finished { verify: Option<VerifyOutcome>, verify_exit_code: Option<i32> },
    /// D92: every row is resettable (vacuously, with no rows).
    Reset,
    /// D93: `(repo, path, base_ref)` of every row, for the note.
    NeverReset { trees: Vec<(RepoId, String, String)> },
}

/// D90/D92/D93 over one `running` step. Seven arguments, which is clippy's limit.
#[must_use]
pub fn classify(step: &RunStep, phase: &SnapshotPhase, run_scope: &[RepoId], trees: &[RunStepTree],
                commits: &[RunStepCommit], output_present: bool, command_runs: &[CommandRun])
    -> (StepKind, Adjudication);

/// D91: the step's verify from rows. `finished_at` set → the row's own columns. Otherwise the last
/// `command_run` whose `class == VERIFY_CLASS`: `Done` + `Some(0)` → `Pass`; `Done` + other →
/// `Fail`; `Failed` → `Unavailable`; `Queued`/`Running` or none → `None`.
#[must_use] pub fn verify_of(step: &RunStep, command_runs: &[CommandRun]) -> (Option<VerifyOutcome>, Option<i32>);

/// D91: `gate::settle` with `driver: Ok(DoneEvent { stop_reason: EndTurn })`, `cap_breach: None`,
/// `deadline_seconds: None`.
#[must_use]
pub fn resettle(output: Option<&Document>, verify: Option<VerifyOutcome>, is_review: bool,
                started_at: DateTime<Utc>, now: DateTime<Utc>) -> Settle;

/// D97 as redefined (C129): the latest-attempt `done` winner at the highest position `p` having one
/// (`selected == Some(true)`, or the one `fanout_index = 0` row of a `fan_out = 1` phase),
/// provided every row at a position > `p` is `Superseded | Cancelled`.
#[must_use] pub fn frontier(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Option<StepId>;
```

`classify`'s finished rule is `output_present && (step.finished_at.is_some() || run_scope.iter().all(|r| commits.iter().any(|c| c.repo_id == *r && c.after_hash.is_some())))`.

- Resettable: every row is `Worktree | Copy`, or `(SharedSerialized | Local) && !dirty`.
- A Judge is classified as well. The engine ignores the answer (D95, §9.3).

### 5.2 `lib.rs`

Add `pub mod recover;`. Rewrite `:15-16`: milestone 5 adds `overlap` (T4) and `recover`; `queue.rs` remains MOD-12's and is not created. **No re-export** (T6 adds them).

### 5.3 Tests (all in `recover.rs`; snapshots decoded from JSON; no store)

- **Lease times**:
  - `lease_times_read_the_two_app_settings`: `{ttl: 300, refresh: 90}` → 300 s / 90 s.
  - `lease_times_fall_back_to_120_and_60`: absent, `0`, `-5` and `"120"` all fall back.
  - `a_refresh_at_or_above_the_ttl_reads_as_half`: `{60, 60}` → 30 s; `{1, 5}` → 500 ms.
- **Heartbeat**, each `#[tokio::test(start_paused = true)]` with a scripted closure:
  - `heartbeat_refreshes_every_interval`: three `Ok(true)`s; after `timeout(185 s)` the closure ran 3 times and each `until == clock.now() + ttl`.
  - `heartbeat_returns_abandoned_on_a_zero_row_refresh`: `Ok(true)` then `Ok(false)` → `Abandoned` at t = 120 s.
  - `heartbeat_survives_a_store_error`: `Err`, `Err`, `Ok(false)` → `Abandoned` on the third beat.
- **`classify`**:
  - `classify_finished_by_finished_at`
  - `classify_finished_by_every_after_hash`
  - `classify_not_finished_without_the_document`
  - `classify_not_finished_with_one_repo_missing`
  - `classify_resettable_and_not_per_mode_and_dirty`: the 4 modes × `dirty` table.
  - `classify_a_candidate`
  - `classify_a_judge`
  - `classify_a_step_with_no_tree_is_vacuously_resettable`
- **`resettle` and `verify_of`**:
  - `verify_of_maps_command_runs_to_outcomes`: the four rows, a non-`verify` class ignored, `finished_at` wins.
  - `resettle_maps_command_runs_to_verify_outcomes`: `Fail` → `Settle::Failed(Verify…)`.
  - `resettle_reads_a_review_verdict`: `is_review` with a `request-changes` front matter → `Settle::Rejected`.
- **`frontier`**:
  - `frontier_is_the_highest_done_winner_without_a_successor`
  - `frontier_is_none_mid_position`: a `pending` row at *p* + 1.
  - `frontier_ignores_retired_rows_after_a_review_rejection`: implement attempt 2 `done` at *p* and the review attempt 1 `cancelled` at *p* + 1 → implement attempt 2 (C129).
  - `frontier_of_a_fan_out_is_the_selected_winner` (added): three `done` candidates, one `selected`.

---

## 6. T2: the seam (D79–D83, D87–D89, D110–D112)

### 6.1 Files and implementors

Files: `model/run.rs`, `store/traits.rs`, `store/mem.rs`, `store/conformance.rs`, `fixtures.rs`, `tests/mem_store.rs` (htui-core); `pg/write.rs`, `writer.rs`, `tests/pg_conformance.rs`, `tests/pg_criteria.rs`, `.sqlx/*` (htui-store); `src/conformance.rs`, `tests/recorder.rs` (htui-agent); `engine.rs`, `graph.rs` (htui-orch); `recover.rs` (a guard grep only).

**The five `WriteStore` implementors**, all of which change in commits (b)–(d):

| Implementor | Where |
|---|---|
| `MemStore` | `mem.rs:4180`; `claim_run` at `:4461`, `State::claim_run` at `:3209` |
| `PgStore` | `pg/write.rs:381`; `claim_run` at `:2464` |
| `Writer` | `writer.rs:287`; `claim_run` at `:673-685` (two arms: `Memory`, `Online`) |
| `UsageSpy<'_, S>` | `htui-agent/src/conformance.rs:673`; `claim_run` at `:927-937` |
| `SpyStore` | `htui-agent/tests/recorder.rs:353`; `claim_run` at `:621-631` |

### 6.2 Surface

```rust
// model/run.rs, GraphSnapshot (`:448-461`), after `settings`:
/// Plan D79: §4.7's resolved scope, written by `graph::resolve` at `StartRun`. **Not** hashed by
/// `topology` (which reads `phases[]` alone) and [`GraphSnapshot::V`] is not bumped for it; `None`
/// on every snapshot written before milestone 5, read conservatively by `overlap::scope_of`.
#[serde(default)]
pub scope: Option<crate::model::overlap::RunScope>,

// traits.rs
async fn claim_run(&self, run: RunId, box_id: BoxId, owner: Uuid, at: DateTime<Utc>,
                   lease_until: DateTime<Utc>) -> Result<Claim>;
/// D87: `lease_owner = owner, lease_box_id = box_id, lease_expires_at = until` WHERE status IN
/// ('running','awaiting_approval') AND executing_box_id = box_id AND (lease_owner = owner OR
/// lease_owner IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= now). `Ok(false)` = not
/// ours and live, or not takeable. NotFound { "run" } told apart by one follow-up read.
async fn take_lease(&self, run: RunId, box_id: BoxId, owner: Uuid, now: DateTime<Utc>,
                    until: DateTime<Utc>) -> Result<bool>;
/// D89: `running -> failed`, `gate_note = note`, `finished_at = COALESCE(finished_at, at)`,
/// `gate_outcome` untouched. `Ok(false)` when not `running`; NotFound { "run_step" }.
async fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>) -> Result<bool>;
```

- Placement: `take_lease` goes after `adopt_runs` (`traits.rs:741`), and `interrupt_step` after `finish_step` (`:786`).
- `claim_run`'s doc (`:691-710`) is rewritten around rules L/I/P, `Claim`, the decision order and `scope_of`'s conservative reading.
- `adopt_runs`' doc (`:727-734`) gains: "**never** a run whose `lease_owner` is `owner` (D88)".
- The `WriteStore` method count goes from 63 to 65.

### 6.3 Behaviour per store (D111)

**`MemStore`** (`State`, `mem.rs:3209-3333`):
- `claim_run` keeps its first checks: `require_run`, the box, and `NotClaimable`.
- The slot check becomes `SlotFull { running: rows(running), limit }` (the limit is read once into a local).
- Overlap: collect `live` sorted by `(queued_at, id)`. Build `scope_of(claimed.graph_snapshot.as_ref().unwrap_or(&Value::Null), &claimed.repo_scope)`, and the same for each live row after the prefilter `row.repo_scope ∩ claimed.repo_scope ≠ ∅`. The first `overlaps(..)` hit gives `Overlaps { with: row.id, rule }`.
- Otherwise the writes are unchanged and the answer is `Admitted`.
- `take_lease` and `interrupt_step` are new `State` methods, stamping `updated_at = now` like their siblings.
- `adopt_runs` adds `&& self.lease_owners.get(&row.id) != Some(&owner)`.

**`PgStore`** (`pg/write.rs`):
- `claim_run`'s first `SELECT` adds `graph_snapshot` (`:2474-2480`).
- The `EXISTS` query (`:2535-2547`) becomes a row query that keeps the box `FOR UPDATE` held:

  ```sql
  SELECT id AS "id: RunId", graph_snapshot, repo_scope AS "repo_scope: Vec<RepoId>"
    FROM run
   WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval')
     AND repo_scope && $2::uuid[]
   ORDER BY queued_at, id
  ```

  It is decided in Rust with `scope_of`/`overlaps`.
- `take_lease` is one `UPDATE`, plus the existing literal `SELECT 1 FROM run WHERE id = $1` (`:2617`) as its `NotFound` follow-up.
- `interrupt_step` is one `UPDATE run_step SET status = 'failed', gate_note = $2, finished_at = COALESCE(finished_at, $3) WHERE id = $1 AND status = 'running'`, plus `step_exists`'s literal `SELECT 1 FROM run_step WHERE id = $1` (`:157`) on zero rows.
- `adopt_runs` adds `AND lease_owner IS DISTINCT FROM $2` (or takes A-5's shape).

**`Writer`**: two new delegating arms each. **Spies**: forward.

**`engine.rs:450-457`** becomes

```rust
let claim = self.parts.store.claim_run(id, self.parts.box_id, self.parts.owner, now, lease).await?;
if !claim.is_admitted() {
    return Err(EngineError::ClaimRefused { run: id });
}
```

The reason is T6's. Use `is_admitted`, not `other =>`, which is an unused binding (H-9). **`graph.rs:338`** gains `scope: None` (T4 fills it).

### 6.4 Commits (D110)

- **(a)** `GraphSnapshot.scope` plus `scope: None` at the five literals: `graph.rs:338`, `pg_criteria.rs:691`, `fixtures.rs:1328`, `mem.rs:5794`, `conformance.rs:3761`. Green, no behaviour change. `feature_snapshot_matches` still passes because the JSON decodes `scope` as `None` and `resolve` writes `None`.
- **(b)** `claim_run -> Result<Claim>` with the **final** predicate. Scope-less runs read conservatively and answer exactly as today, with `rule: NotIsolated`. This commit covers all 31 call sites (§11), the `mem.rs:5948` twin, case 1 (first failing: `claim_run_applies_the_isolation_and_path_rules`), `CASES` 49 → 50 with both pins, and `.sqlx` (§12).
- **(c)** `take_lease` on all five implementors, case 2, pins 51, `.sqlx`.
- **(d)** `interrupt_step` on all five, case 3, pins 52; D88 clause on both stores plus the extended legs; `.sqlx`.
- **(e)** the `pg_criteria.rs` races (§6.5), plus `.sqlx` if a new `query!` literal is added.

### 6.5 Tests

| Test | Runs on | Asserts |
|---|---|---|
| **`claim_run_applies_the_isolation_and_path_rules`** (new `CASES` entry) | Mem + Pg | Repos `core` (primary) and `web`; nine minted items; each run's snapshot is `GraphSnapshot { scope: Some(..), ..run_snapshot() }` with `queued_at = at + i s` (F-E). Legs in order: **A** `core{isolated,["src/"]}` → `Admitted`; **B** `core{isolated,["docs/"]}` → `Admitted` (15, parallel); park A and B via `transition_run`; **C** `core{isolated,["src/lib/"]}` → `Overlaps{with:A, Paths}`, still `queued`, item still `queued`; **D** `core{isolated:false}` → `Overlaps{A, NotIsolated}`; **E** `core{local:true}` → `Overlaps{A, Local}`; **F** `core{isolated,[]}` → `Overlaps{A, Paths}`; **G** `web{isolated,["src/"]}` → `Admitted`; **H** `web{isolated,["docs/"]}` → `Admitted`; **I** `repo_scope = []` → `SlotFull{running:2, limit:2}`. The legs C–F hit parked runs and G–H find free slots beside them (criterion 16's two halves, `:2128-2130`). |
| **`take_lease_moves_only_our_own_or_an_expired_lease`** (new) | Mem + Pg | Run R is claimed by X until `at+5m`. `take(Y, now=at)` → `false` and the expiry is unchanged. `take(X, at, at+10m)` → `true`, expiry `at+10m`. `take(Y, at+11m, at+20m)` → `true`; `refresh_lease(R, X)` → `false`; `refresh_lease(R, Y)` → `true`. Park R, release with `refresh_lease(R, Y, at+11m)`, then `take(Z, at+11m, ..)` → `true`. An unclaimed `queued` run → `false`. A `finish_run`'d run → `false`. `take(R, BoxId::new(), ..)` → `false`. An unknown run → `NotFound{"run"}`. |
| **`interrupt_step_is_a_cas_on_running`** (new) | Mem + Pg | A `running` step → `true`: `failed`, `gate_note == Some("interrupted")`, `gate_outcome == None`, `finished_at == Some(at)`. A second call → `false`. A `running` step already carrying `finished_at` (via `finish_step`) keeps the earlier instant. `pending`, `awaiting_approval` and `done` steps → `false`, row-equal before and after. Unknown → `NotFound{"run_step"}`. |
| `claim_run_admits_one_and_refuses_the_second` (extended) | Mem + Pg | §11's mapping; same legs. |
| `lease_refresh_is_a_cas_on_owner` (extended, F-V) | Mem + Pg | After `:4330`: `adopt_runs(BOX, second_owner, swept + 1 s, later)` is empty ("a process never adopts its own lease, D88"); then `adopt_runs(BOX, first_owner, swept + 1 s, later)` gives `[run]`. |
| `mem.rs::claim_run_refuses_an_overlapping_scope_and_a_full_box` / `a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it` | Mem | Mapped per §11; the lease twin gets the D88 leg after `:6164`. |
| **`two_sweeps_adopt_each_expired_run_once`** (new, `pg_criteria.rs`) | Pg, two pools | Two runs are claimed by X with `until = at − 1 min`. `tokio::join!(a.adopt_runs(BOX, A, now, now+5m), b.adopt_runs(BOX, B, ..))`: both are `Ok`, sizes sum to 2, the id sets are disjoint, and the union is `{r1, r2}`. |
| **`two_takes_of_one_released_lease_admit_one`** (new, F-F) | Pg, two pools | A parked run whose lease was released → `join!(take(A), take(B))` gives exactly one `true`, and the stored expiry is the winner's. |

The mirror pattern is `admission_is_serialised_by_the_box_row_lock` (`pg_criteria.rs:585-664`), including its second `PgStore::connect`.

### 6.6 `recover.rs` guard

`grep -nE 'claim_run|RunScope|Claim|GraphSnapshot \{' crates/htui-orch/src/recover.rs` must be empty (D106). T2 edits nothing there.

### 6.7 Gate

```bash
cargo test -p htui-core --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1      # all 36 orch CASES unchanged
( cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
    cargo sqlx prepare --check -- --all-targets --all-features )  # §12
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The store suites print `skipped: HTUI_TEST_DATABASE_URL not set` and pass vacuously without the variable. Confirm that line is **absent**.

---

## 7. T4: resolving the scope at `StartRun` (D79, D81, D119)

**First failing test**: `a_qualified_glob_names_its_repo`.

### 7.1 `crates/htui-orch/src/overlap.rs`

```rust
//! ANA-2 §4.7's resolution half (plan D81): `touched_paths` + the project's repos + the snapshot's
//! per-phase isolation → `(run.repo_scope, RunScope)`. The predicate is `htui_core`'s (D80).
/// # Errors
/// [`ResolveError::UnknownTouchedRepo`]; [`ResolveError::EmptyScopeWithPrimary`] (D14, kept).
pub fn resolve(item: &Item, repos: &[Repo], phases: &[SnapshotPhase], requested: Option<&[RepoId]>)
    -> Result<(Vec<RepoId>, RunScope), ResolveError>;
```

Rules (D119):
1. `primary = repos.iter().find(|r| r.is_primary)`.
2. Each `touched_paths` entry goes through `PathPrefix::parse(entry, primary.map_or("", |r| &r.name))`:
   - if the parsed `repo` equals `primary`'s name, or the entry had no qualifier, it belongs to the primary;
   - a qualified slug maps to the `Repo` whose `name` equals it, and no match → `UnknownTouchedRepo { item, name }`;
   - with no primary, a bare glob maps nowhere and is dropped.
3. Derived repos (with `requested == None`): every repo a qualified entry names, plus the primary when any bare glob is declared or no glob at all. The result is `repo_scope`, in `RepoId` order.
4. `requested == Some(scope)`: `Some([])` with a primary → `EmptyScopeWithPrimary` (`graph.rs:251-255`'s rule). Otherwise `repo_scope = scope` **as given** (order kept), and only entries for repos in `scope` contribute prefixes. `UnknownTouchedRepo` still fires, because the name is wrong regardless.
5. For each repo in `repo_scope`:
   - `isolated = phases.iter().all(|p| matches!(p.isolation, Worktree | Copy))`;
   - `local = phases.iter().any(|p| p.isolation == Local)`;
   - `prefixes` = its entries' `prefix` strings, deduplicated and sorted; no entry means `[]`.
   The judge's synthesized phase is not a snapshot phase and runs with an empty scope, so it is not an input.
6. Empty `phases` (the store conformance's `run_snapshot`) → `isolated = true` vacuously. That is irrelevant in practice: graphs have phases.

### 7.2 `graph.rs`

- Add `ResolveError::UnknownTouchedRepo { item: ItemId, name: String }`, with `#[error("item {item} touches repo `{name}`, which the project does not carry (ANA-2 §4.7 `:1025`)")]`.
- In `resolve`, read `repos` **before** building the literal, call `overlap::resolve(item, &repos, &phases, requested_scope)?`, and set `scope: Some(scope)` in the `GraphSnapshot` literal (`:338-349`).
- Remove `resolve_scope` (`:235-260`) and its re-export at `lib.rs:45`. `grep -rn resolve_scope crates/` finds only `graph.rs:109` (doc), `:245` (fn), `:352` (call) and `lib.rs:45`, besides `real.rs`'s unrelated `self.resolve_scope`. Rewrite `Resolved.repo_scope`'s doc (`:109`) to "from [`crate::overlap::resolve`]".
- `lib.rs`: `pub mod overlap;` (alphabetical). No re-export (F-G).

### 7.3 Fixture

`tests/fixtures/feature.snapshot.json` gains exactly `"scope": { "repos": {} }` after `"settings"`: the demo project has no repo, so the derived scope is empty. **Nothing else in the file moves, `topology` included.** `feature-with-verify.snapshot.json` is untouched (F-L, C-3).

### 7.4 Tests

`overlap.rs` [pure]:
- `a_bare_glob_is_the_primary_repo`
- `a_qualified_glob_names_its_repo`
- `an_unknown_repo_name_is_refused`
- `no_declaration_is_the_whole_primary_repo`: `prefixes == []`.
- `a_requested_scope_is_honoured_and_its_paths_are_filtered`
- `every_isolated_phase_makes_the_repo_isolated`
- `one_local_phase_makes_the_repo_local`
- `one_shared_phase_makes_it_not_isolated`
- `a_project_without_repos_resolves_to_nothing` (added): `[]`, `RunScope::default()`.

`graph.rs` [Mem]:
- `resolve_writes_the_scope_into_the_snapshot`: a repo `core` plus `touched_paths = ["src/**"]` gives `scope.repos[core] == {isolated: true, local: false, prefixes: ["src/"]}` under the seeded `worktree` default.
- `feature_snapshot_topology_is_pinned`: unchanged (`:885`).
- `empty_scope_with_primary_is_refused`: unchanged in meaning (`:1288`).

Gate: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; C-3.

---

## 8. T6: the engine holds its lease, and overlap reaches the walk (D83–D88, D107, D108)

### 8.1 `command.rs` (commit a, D120)

```rust
/// Plan D83: why `claim_run` refused. The run stays `queued`; `Engine::claim` re-attempts (D84).
#[error("claim refused: {claim}")]
ClaimRefused { run: RunId, claim: Claim },
/// D86: a zero-row refresh; the walk was dropped where it stood.
#[error("run {run}: its lease was taken by another orchestrator; this walk was abandoned (ANA-2 §4.9)")]
LeaseLost { run: RunId },
/// D87: an unpark (or A-1's resume) met a live lease this process does not hold.
#[error("run {run}: another orchestrator holds a live lease (ANA-2 §4.9)")]
LeaseHeld { run: RunId },
```

- The Display test (`:901-904`) asserts `claim refused: box full (2 of 2 running)` and `claim refused: overlaps run <RUN_1> (paths)`, plus the two new variants' exact bytes.
- The `:235-237` doc now names D83. The `:432-433` sentence becomes "…refused here as `AlreadySelected`; milestone 6's `Unblock`-shaped verb owns that retry (R-7)".
- Under A-8, `retry_enabled` (`:401`) gains its interrupted exemption.

### 8.2 `LeaseTimes` (commit b)

- Remove `LEASE_SECONDS` (`engine.rs:83-84`).
- Add `fn lease_times(&self) -> LeaseTimes { LeaseTimes::from_app(&self.parts.app) }`.
- `start_run`'s `lease = now + self.lease_times().ttl`.
- Test `lease_times_come_from_app_settings` [Mem, engine unit]: `set_app_setting("lease_ttl_seconds", 300)`, then `StartRun` → the claimed run's `lease_expires_at == claim_instant + 300 s`.

### 8.3 The leased walk (commit c, D107, D108)

```rust
/// D86/D107. Both futures `Box::pin`ned so `select` sees `Unpin` and `drop(walk)` really drops it.
async fn walk_leased<T, F>(&self, run: RunId, walk: F) -> Result<T, EngineError>
where F: Future<Output = Result<T, EngineError>>;
/// D87/D108: `take_lease(run, box, owner, now, now + ttl)`; `Ok(false)` → `LeaseHeld { run }`.
async fn take_lease(&self, run: RunId) -> Result<(), EngineError>;
/// D87: `refresh_lease(run, owner, now)`; `Ok(false)` ignored, `Err` warned. Never raises.
async fn release_lease(&self, run: RunId);
/// D84: `claim_run` then the leased walk.
pub async fn claim(&self, run: RunId) -> Result<CommandOutcome, EngineError>;
```

`walk_leased` body:

```rust
let times = self.lease_times();
let (store, owner) = (self.parts.store, self.parts.owner);
let beat = recover::heartbeat(|until| store.refresh_lease(run, owner, until), self.parts.clock, times);
match futures::future::select(Box::pin(walk), Box::pin(beat)).await {
    Either::Left((out, _beat)) => {
        let out = out?;                                   // Err: write nothing more (the store may be down)
        if self.run(run).await?.status != RunStatus::Running { self.release_lease(run).await; }
        Ok(out)
    }
    Either::Right((Heartbeat::Abandoned, walk)) => {
        drop(walk);                                       // before anything else (D86's probe)
        if let Err(err) = self.parts.isolator.release(run).await { tracing::warn!(%run, %err, "release after abandon"); }
        Err(EngineError::LeaseLost { run })
    }
}
```

**Entry-by-entry (F-M, F-N, D108):**

| Entry | Order |
|---|---|
| `start_run` | resolve → `create_run` → `self.claim(id)` (`claim` = `claim_run` → refused → `ClaimRefused { run, claim }`, else `walk_leased(run, run_to_rest(run))` → `Started`) |
| `answer_gate` | reads, `answer_gate_enabled` (`:500`) → **`take_lease`** → `store.answer_gate` (`:510`) → `unpark` → `walk_leased({ after_rejection \| reconcile_done_step + run_to_rest })` (covers `:529`, `:575`) |
| `retry_step` | guards through `retry_enabled` (`:628`) → **`take_lease`** → the `Retried` answer (`:636`) → `unpark` → `walk_leased({ admit; run_to_rest })` (`:661`) |
| `retry_group` | `retry_group_enabled` (`:692`) → **`take_lease`** → `retire_slot` (`:696`) → `unpark` → `walk_leased({ admit; run_to_rest })` (`:703`) |
| `select_fanout` | `select_enabled` (`:726`) → **`take_lease`** → `store.select_fanout` (`:731`) → note → `unpark` → `walk_leased({ reconcile_done_step; run_to_rest })` (`:761`) |
| `resume` | (A-1: **`take_lease`** first) → resolve → `walk_leased(run_to_rest)` at both `:969` and `:998`; the not-comparable list (`:957-961`) gains `ResolveError::UnknownTouchedRepo { .. }` |
| `run_to_rest` (`:847`) | unchanged, `pub`, unleased primitive |
| `cancel_run` | unchanged (milestone 6) |

If a write after `take_lease` fails, the lease is left to expire at TTL (H-7).

**Doc sentences (F-Q)**: `:935-936` (resume: "milestone 5's sweep is `Engine::sweep`; this is the walk it hands off to"); `:1588-1595` (drive_group: a drop is now the intended abandon, adjudicated by `Engine::sweep` D95, and its guards dropped by `Isolator::release` D99); `:2851-2852` (reconcile: "milestone 6's"); `:2942` and `:2955` ("no sweep retries a terminal run's cleanup; milestone 6").

### 8.4 Harness and cases (commit d)

**`fake.rs` (D118)**: `pub fn restarted(&self) -> Self` shares `store` (the `MemStore` clone shares `Arc<RwLock<State>>`, `mem.rs:55-58`), and builds fresh `FakeIsolator` and `FakeVerifier`. Its clock is `TestClock::at(self.clock.now() + RESTART_GAP)`, with `pub const RESTART_GAP: TimeDelta = 10 min`, above every seeded or fallback TTL. It clones `scripts`, `candidates`, `default_script` and `caps`, sets `after_done_advance` to `None`, keeps the same `box_id`/`user`, and mints a **new** `owner`. T7's stalls are **not** carried.

**`engine.rs`**: `#[cfg(feature = "test-support")] pub async fn claim_fake(orch, run)`, beside `dispatch_fake` (`:3766`).

**`conformance.rs`**: `Orchestrate` gains `async fn claim(&self, run) -> Result<CommandOutcome, EngineError>`, `fn restarted(&self) -> Self where Self: Sized`, and `fn owner(&self) -> Uuid` (F-J).

**`lib.rs`**: `pub use recover::{Heartbeat, LeaseTimes};`; `isolate::ResetReport` joins the `isolate::` list; `claim_fake` joins the test-support line. **Not** `overlap::resolve` (F-G).

**Six `CASES`** (36 → 42), all over [fake] (`FakeDriver` + `FakeIsolator` + `MemStore`):

| Case | Setup | Asserts |
|---|---|---|
| `overlapping_touched_paths_serialise` (15a) | `primary_repo`; `FEAT-3` and `ANA-2` both `touched_paths = ["src/**"]` (via `update_item`) | `StartRun(FEAT-3)` parks at its first gate. `StartRun(ANA-2)` → `Err(ClaimRefused { claim: Overlaps { with: run1, rule: Paths }, .. })`; run 2 and its item are `queued`. `CancelRun(run1)`, then `orch.claim(run2)` → `Started` |
| `the_same_paths_in_two_repos_run_concurrently` (15b) | `htui` (primary) and `web`; items `htui:src/**` and `web:src/**` | both `Started`, and each run's `repo_scope` is its own repo |
| `an_undeclared_item_holds_its_whole_primary_repo` (15c) | run 1 `docs/**`, run 2 undeclared | run 2 → `Overlaps { with: run1, rule: Paths }` |
| `a_third_run_waits_for_a_slot_and_a_parked_run_still_blocks_overlap` (16) | P (`lib/**`) parked through `StartRun`; E (`lib/**`); then A and B (`src/**`, `docs/**`) queued through `StartRun` and claimed **directly** with `store.claim_run` standing in for two live walks; C (`web/**`) | E → `Overlaps { with: P, Paths }` with 0 running. A and B are claimed. C → `SlotFull { running: 2, limit: 2 }` |
| `a_parked_run_releases_its_lease_and_an_answer_takes_it` (D87) | `StartRun` parks at `prd` | `lease_expires_at == Some(orch.clock().now())`. `other = orch.restarted()`; `other.dispatch(AnswerGate(Approved))` → `Answered`. `refresh_lease(run, other.owner(), t)` → `true`; with `orch.owner()` → `false` |
| `a_live_lease_blocks_an_answer_from_another_process` (D87) | park, then `refresh_lease(run, orch.owner(), other_clock_now + 1 day)` | `other.dispatch(AnswerGate(Approved))` → `Err(LeaseHeld { run })`; the step is still `awaiting_approval` with `gate_outcome == None` (D108: nothing written) |

**Unit tests in `engine.rs`**:
- `a_walk_whose_lease_is_taken_is_abandoned_and_writes_nothing` [Mem, `#[tokio::test(start_paused = true)]`]: claim a run through `store.claim_run` with the harness's owner, and snapshot its `Run` and `run_steps`. Then `tokio::join!(engine.walk_leased(run, std::future::pending::<Result<Rest, _>>()), store.adopt_runs(BOX, stranger, lease + 1 s, ..))` → `Err(LeaseLost { run })`. `isolator.releases() == 1`. The rows are equal to the post-adopt snapshot.
- `lease_times_come_from_app_settings` (§8.2).
- `claim_refused_names_the_rule`: over `start_run`, the `Display` of a real refusal.

**Pins**: `tests/fake_conformance.rs:15-16` becomes `cases_len_is_forty_two` / `42`. `conformance.rs:3154` becomes `cases_are_unique_and_forty_two`, with `:3161` and the message at `:3162-3169` **recounted**. The `CASES` doc (`:164-185`) is rewritten. Recount, do not append (§13).

Gate: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; `cargo doc -p htui-orch --no-deps --all-features`.

---

## 9. T7: the recovery sweep (D89–D98, D100, D115, D117)

### 9.1 `status.rs` (commit a)

```rust
/// Plan D94: the sweep failed a step it found `running` after the lease expired.
Interrupted { phase: String, reset: bool },
// Display: reset → "interrupted: {phase}"; !reset → "interrupted, tree not reset: {phase}"
```

`run_failure_display_is_ana2s_bytes` (`:349`) gains both rows (`interrupted: implement`, `interrupted, tree not reset: implement`).

### 9.2 Harness (commit b, A-2)

- `FakeOrchestrator::stall_after_done(&self, phase: &str, attempt: i32, slot: Option<(i32, u32)>, write_output: bool) -> Arc<Notify>` (F-O). It is keyed like `script_candidate`. `after_done` checks the stall key **after** the clock advance and after writing the document when `write_output`. It then calls `notify.notify_one()` and awaits `std::future::pending()`.
- `FakeIsolator::stall_nth_reconcile(&self, n: u32) -> Arc<Notify>` (F-A): the n-th `reconcile` call records itself in `reconciles`, notifies, and never returns.
- `pub async fn until_stalled<F: Future>(fut: F, stalled: &Notify)` in `conformance.rs`, private to the suite: `select(Box::pin(fut), Box::pin(stalled.notified()))`. `Left` panics ("the walk finished instead of stalling"); `Right` drops the future.
- `Orchestrate` gains `async fn sweep(&self) -> Result<Vec<Adopted>, EngineError>` and `stall_after_done` (and `isolator()` already exists).
- `engine.rs` gains `sweep_fake`.

### 9.3 `Engine::sweep` (commit c, D117)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted { pub run: RunId, pub next: Next }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next { Walk, Parked(Rest), Finished(Rest), Error(String) /* A-4 */ }

/// ANA-2 §4.9 `:1284-1307`, plan D98. Adopts, adjudicates, does **not** walk: the caller walks each
/// `Next::Walk` through [`Engine::resume`].
pub async fn sweep(&self) -> Result<Vec<Adopted>, EngineError>;
async fn recover_run(&self, run: &Run) -> Result<Next, EngineError>;
async fn park_interrupted(&self, run: &Run, step: &RunStep, phase: &SnapshotPhase,
                          reset: bool, note: String) -> Result<Rest, EngineError>;
```

`sweep`:
1. `now`; `times`.
2. `adopted = store.adopt_runs(box, owner, now, now + times.ttl)`, in `queued_at` order.
3. For each run: `recover_run` (inside `walk_leased` under A-7). A `Parked`/`Finished` outcome is followed by `release_lease`. An `Err` becomes A-4's `Error`, with a note and a release.

`recover_run`:

| # | Step |
|---|---|
| 0 | `snapshot`, `steps` |
| 1 | **(A-3)** If no step is `running`: `recover::frontier(&snapshot, &steps)` → `Some(w)` → `reconcile_done_step(run, w, siblings_of(w))`, where `Some(rest)` → `Parked(rest)` |
| 2 | For every `running` step in `(position, attempt, fanout_index)` order, `phase = phase_at(position)` and `(kind, adj) = classify(..)` over `step_trees`, `step_commits`, `output_of(item, phase, step).is_some()` and `command_runs`, then act per the table below |
| 3 | **D96**: re-read the run and steps. If the run is `running`, no step is `running` and one is `awaiting_approval` → `transition_run(Running → AwaitingApproval)`, then the item `InProgress → AwaitingApproval` → `Parked(resting)` |
| 4 | Without A-3, the plan's order: D97 goes here |
| 5 | `Next::Walk` |

Step 2's per-kind actions:

| Kind × Adjudication | Action |
|---|---|
| `Judge`, any | `interrupt_step(step, "interrupted", now)`; the walk's `Select` arm re-parks for a human (M4 D59, `select_stage` step 2) |
| `Candidate`, `Finished { verify, code }` | `finish_step(real verify, code, finished_at: now)` when `finished_at` is `None`, then `transition_step(Running → Done)` (M4 D48: verify recorded, not applied) |
| `Candidate`, `Reset \| NeverReset` | `interrupt_step(step, "interrupted", now)`; no reset, no park (D95) |
| `Plain`, `Finished { verify, code }` | `finish_step` when `finished_at` is `None`; `settle = resettle(output, verify, phase.name == REVIEW_PHASE, step.started_at, now)`; `gate::apply(ctx, step, phase, settle)`. `Advance` → `reconcile_done_step` (`Some(rest)` → `Parked`). `Retry { p, a }` → `admit` (a refusal → per its `Rest`). `Rest(r)`: terminal → `cleanup_run` + `Finished(r)`, else `Parked(r)`. The exact tail of `walk_live_step` (`engine.rs:1403-1426`) |
| `Plain`, `Reset` | `report = isolator.reset(step, &trees)`. If `refused` is empty: `interrupt_step(step, "interrupted")`, then note N1, then `may_attempt(attempt + 1, retry_limit)` → `admit(run, snapshot, phase, attempt + 1)`, else `park_interrupted(reset: true, N3)`. If `refused` is non-empty or `Err` → the `NeverReset` row with the refusal or error as its reason |
| `Plain`, `NeverReset { trees }` | `interrupt_step(step, "interrupted, tree not reset")` → `park_interrupted(reset: false, N2)`; **no `git` write** |

An `interrupt_step` that answers `Ok(false)` means another writer moved the step. Skip that step, because step 3 re-derives.

`park_interrupted` uses `park_run`'s order (`engine.rs:2900-2933`) with the step already `failed` and the body passed in. It returns `Rest { AwaitingApproval, Some(position), Some(RunFailure::Interrupted { phase, reset }) }`. `run.failure` stays NULL (R-3).

**Bytes (D115):**

| Where | Bytes |
|---|---|
| `run_step.gate_note` | `interrupted` \| `interrupted, tree not reset` (ANA-2 `:1298-1299`) |
| N1 (reset, retried) | `` interrupted: step {id} (`{phase}` attempt {a}) did not finish; trees: {tree}, …; retrying as attempt {a+1} `` |
| N3 (reset, out of budget) | `` interrupted: retry budget spent: step {id} (`{phase}` attempt {a}); trees: {tree}, …; the run is parked `` |
| N2 (never reset) | `` interrupted, tree not reset: step {id} (`{phase}` attempt {a}); trees: {tree}, …; reason: {reason}; the run is parked for a human `` |
| `{tree}` | `{repo} {path} before_hash {base_ref}` plus ` labelled htui/{step} at {head}` when `labelled` names it |
| `{reason}` | `dirty at step start` (the row's `dirty`) or the refusal/error text |

`lib.rs` (F-K): `pub use engine::{Adopted, Next}` and `sweep_fake` on the test-support line.

### 9.4 Ten `CASES` (42 → 52), all [fake]

Every case calls `primary_repo` (F-R). The crash is `until_stalled(orch.dispatch(StartRun), &stall)` (A-2), and `other = orch.restarted()`.

| Case | Setup | Asserts |
|---|---|---|
| `a_finished_step_is_adopted_through_its_gate` (18a) | Leg 1: every FEAT phase `Never` (F-S); stall `prd` attempt 1 with the document; then `store.record_commits(prd, [{repo, before, after: Some("fake:after:x")}])` | `other.sweep()` → `[Adopted { run, next: Walk }]`; `prd` is `done`; `other.resume(run)` → `Walked(Rest { Done, .. })`. Leg 2, a fresh orchestrator with `prd` `Always`: the sweep gives `Parked`, `prd` is `awaiting_approval` and not `done` |
| `an_unfinished_step_is_reset_and_retried` (18b) | all `Never`; stall `prd` without the document | sweep → `Walk`; `prd` attempt 1 is `failed` with `gate_note == "interrupted"`; `other.isolator().resets() == 1`; a `pending` attempt 2 exists; N1 was written; `resume` → `Done` (no base equality, F-D) |
| `an_interrupted_step_out_of_budget_parks` (F-C) | `repoint` `retry_limit = 0` (`conformance.rs:455`); stall without the document | sweep → `Parked(Rest { AwaitingApproval, failure: Some(Interrupted { reset: true, .. }) })`; run and item are `awaiting_approval`; N3; `RetryStep` → `Retried` (A-8 accepted: the interrupted exemption admits attempt 2 although `retry_limit = 0`), and an attempt-2 row exists at `running` |
| `a_dirty_tree_is_never_reset` (12, fake half) | `prd` `Local`; stall without the document; `upsert_step_tree(prd, [row with dirty: true])` | sweep → `Parked(.. Interrupted { reset: false, .. })`; `gate_note == "interrupted, tree not reset"`; `other.isolator().resets() == 0`; N2 names `path` and `before_hash` |
| `a_crash_before_reconcile_is_reconciled_on_adoption` (D97, F-A) | `prd` `Never`; `stall_nth_reconcile(1)` | after the stall `prd` is `done` and the run `running`; sweep → `Walk`; `other.isolator().reconciles() == [(prd, [])]`, exactly one |
| `a_crash_between_done_and_reconcile_after_a_review_rejection_is_reconciled` (C129) | FEAT with `implement` and `review` `Never`; review scripted `request-changes` then `approve`; `stall_nth_reconcile(k)` where k is the reconcile of implement attempt 2 | the frontier is implement attempt 2 despite the `cancelled` review row at *p* + 1; one reconcile on `other`; `resume` runs review attempt 2 over it |
| `an_interrupted_candidate_fails_alone` (D95) | `ANA-2` `research` `fan_out 3`, `Never`, a judge; stall `(research, 1, Some((1, 0)))` | after the sweep candidates 0 and 2 are `done` and 1 is `failed` with `"interrupted"`; `resets() == 0`; `resume` → the judge's prompt carries `judge_candidate:0` and `:2` only |
| `an_interrupted_judge_parks_for_selection` | as above; stall `(research:judge, 1, Some((-1, 0)))` | the sweep fails the judge with `"interrupted"`; `resume` → `park_selection` with reason `interrupted`; run `awaiting_approval` |
| `a_half_written_park_is_completed` (D96) | stall with the document; `transition_step(prd, Running → AwaitingApproval)` by hand | sweep → `Parked`; run and item `awaiting_approval`; no step moved |
| `the_sweep_never_touches_a_parked_run_or_a_live_lease` (D88, `:1306`) | P parked through a gate; L stalled, then `refresh_lease(L, orch.owner(), other_now + 1 day)` | `other.sweep()` is empty and P and L are row-equal (`Run` + `run_steps`). Then `orch.clock().advance(2 days)`: `orch.sweep()` is empty too (own lease, D88) |

**Pins**: `cases_len_is_fifty_two` / `52`; `cases_are_unique_and_fifty_two` with its message recounted; the `CASES` doc rewritten.

Gate: `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; doc. Then the workspace gate (§15).

---

## 10. T8: real git (`tests/gix_isolator.rs`)

**Additions:**
- `StallingSink<'a> { inner: CommittingSink<'a>, key: (String, i32), write_output: bool, stalled: Arc<Notify> }`. It commits through `inner`'s logic (`:379-414`), writes the document only when `write_output`, notifies and pends.
- `StallAfterReconcile<'a> { inner: &'a GixIsolator, nth: u32, calls: Mutex<u32>, stalled: Arc<Notify> }` with `impl Isolator`. All eight verbs delegate. The n-th `reconcile` awaits the real one, notifies and pends: "the merge happened, `record_commits` did not".
- `Fixture::second_process(&self) -> (GixIsolator, Uuid, TestClock)`: a new `GixIsolator` over the same `IsolatorConfig` repos, `Uuid::now_v7()`, and the clock `+ RESTART_GAP`.
- `Fixture::engine_as(..)`, which builds `EngineParts` exactly as `Fixture::dispatch` does (`:150-189`) with those three swapped.

Every case opens with `let Some(git) = skip_without_git!() else { return; };`, except the `local` one, which mirrors `criterion_12_a_local_step_on_a_dirty_tree` (`:515-522`).

| Case | Asserts |
|---|---|
| `criterion_12_the_sweep_parks_a_dirty_local_step_without_resetting` | `Local`, a dirty tracked edit, stall `prd` without the document. After `sweep`: the edit is byte-identical, `HEAD == core.head`, `htui_branches()` is empty, the step is `failed` with `interrupted, tree not reset`, and N2 names `core.path` and `core.head` |
| `criterion_18_an_unfinished_worktree_step_is_retried_from_the_same_base` | `Worktree`, the sink commits, then stalls without the document. `sweep` + `resume`: attempt 2's `step_commits.before_hash == core.head` (== attempt 1's); the interrupted `htui/<step1>` still names its commit; `git worktree list --porcelain` lists both trees; `CancelRun` then removes both (criterion 13's path) |
| `criterion_18_a_finished_worktree_step_is_adopted_and_merged` | `Worktree`, `prd` `Never`; the sink commits and writes the document, then stalls; then the second isolator's `capture(step, trees)` and `record_commits(after)` by hand ("capture landed, settle lost"). After `sweep`: `prd` is `done`, and `git log --merges --format=%H <primary>` has exactly one entry whose parents are `[core.head, after]` |
| `a_clean_shared_checkout_is_labelled_then_reset_and_retried` | `SharedSerialized`, the sink commits on the checkout, then stalls without the document. After `sweep`: `htui/<step1>` names the agent commit, `HEAD == core.head`, the checkout is clean, attempt 2 exists and `resume` prepares it from `core.head`. Under A-6 the same leg in `Local` parks instead |
| `a_lost_merge_is_recognised_not_repeated` | `Worktree`, `prd` `Never`, the first process uses `StallAfterReconcile { nth: 1 }`. After the stall the primary holds merge M and `prd`'s `after_hash` is the pre-merge commit. The second process's `sweep` (the D97 frontier) answers M through the parents check (`real.rs:1036-1041`): one merge, and `record_commits` now names M |

Gate: `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`, then the workspace gate.

---

## 11. `claim_run` bool → `Claim`: all 31 call sites

**Count**: `grep -rn 'claim_run(' crates/ --include='*.rs'` gives 38 lines. Take away the five implementor signatures (`mem.rs:4461`, `pg/write.rs:2464`, `writer.rs:673`, `htui-agent/src/conformance.rs:927`, `tests/recorder.rs:621`), the trait declaration (`traits.rs:711`) and `State::claim_run`'s definition (`mem.rs:3209`). That leaves **31 call sites**. Every one is **T2 (b)**. T6 revisits one.

| # | Site | Enclosing | Becomes |
|---|---|---|---|
| 1 | `htui-orch/src/engine.rs:453` | `start_run` | T2: `if !claim.is_admitted() { Err(ClaimRefused { run: id }) }`. **T6**: moved into `Engine::claim`, `ClaimRefused { run, claim }` |
| 2–3 | `htui-store/src/writer.rs:682`, `:683` | `Writer::claim_run` | type only |
| 4 | `htui-agent/src/conformance.rs:936` | `UsageSpy` | type only (`StoreResult<Claim>`) |
| 5 | `htui-agent/tests/recorder.rs:630` | `SpyStore` | type only |
| 6 | `htui-core/src/store/mem.rs:4470` | `impl WriteStore for MemStore` → `State` | type only |
| 7 | `mem.rs:5972` | `claim_run_refuses_an_overlapping_scope_and_a_full_box` | `== Claim::Admitted` |
| 8 | `mem.rs:6000` | same | `== Overlaps { with: first, rule: NotIsolated }` |
| 9 | `mem.rs:6018` | same | `== Admitted` (empty scope) |
| 10 | `mem.rs:6025` | same | `== SlotFull { running: 2, limit: 2 }` |
| 11 | `mem.rs:6032` | same | `== NotClaimable` |
| 12–13 | `mem.rs:6041`, `:6050` | same | `NotFound` legs, type only |
| 14 | `mem.rs:6083` | `a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it` | `== Admitted` |
| 15 | `mem.rs:7195` | `the_eleven_inherent_reads_answer_from_the_fixture` | today it discards the answer. With `#[must_use]` on `Claim` it becomes `assert_eq!(…, Claim::Admitted)` (H-8) |
| 16 | `htui-core/src/store/conformance.rs:4101` | `claim_run_admits_one_and_refuses_the_second` | `== Admitted` |
| 17 | `:4140` | same | `== Overlaps { with: first, rule: NotIsolated }` |
| 18 | `:4158` | same | `== Admitted` (H-10: empty scope) |
| 19 | `:4169` | same | `== SlotFull { running: 2, limit: 2 }` |
| 20 | `:4188` | same | `== Overlaps { with: first, rule: NotIsolated }` (parked still overlaps) |
| 21 | `:4195` | same | `== Admitted` (a parked run holds no slot) |
| 22 | `:4203` | same | `== NotClaimable` |
| 23–24 | `:4210`, `:4217` | same | `NotFound` legs, type only |
| 25 | `:4244` | `lease_refresh_is_a_cas_on_owner` | `== Admitted` |
| 26 | `:5923` | `finish_run_moves_run_and_item_together` | `== Admitted` |
| 27 | `:5980` | same (loop over two runs) | `== Admitted` |
| 28 | `:6018` | same | `== Admitted` |
| 29–30 | `htui-store/tests/pg_criteria.rs:624`, `:625` | `admission_is_serialised_by_the_box_row_lock` | `usize::from(one.is_admitted()) + usize::from(two.is_admitted()) == 1`; the loser is `SlotFull { running: 1, limit: 1 }` |
| 31 | `pg_criteria.rs:3357` | the second-run-of-one-item case (`:3345-3361`) | `== Admitted` |

By file: engine 1, writer 2, agent spies 2, `mem.rs` 10 (1 delegation + 9 in three tests), store conformance 13, `pg_criteria` 3. Total 31.

---

## 12. `.sqlx` regeneration (T2 commits b, c, d, and e if needed)

`.cargo/config.toml` sets `SQLX_OFFLINE = "true"`, so **a changed `query!` without its regenerated file breaks every build** (H-1). Regenerate in the same commit as the query.

```bash
df -h /                                                        # memory: disk pressure kills dev Postgres
docker exec htui-postgres psql -U postgres -c "DROP DATABASE IF EXISTS htui_prepare_check;"
docker exec htui-postgres psql -U postgres -c "CREATE DATABASE htui_prepare_check;"
cd crates/htui-store                                           # from the crate, never the workspace root
export DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check
sqlx migrate run --source migrations                           # the compose `htui` DB is empty; this one is migrated
cargo sqlx prepare -- --all-targets --all-features             # --all-targets: pg_criteria.rs's own query! macros
cargo sqlx prepare --check -- --all-targets --all-features     # "potentially unused queries" warning is expected
git status --short .sqlx                                       # compare with the table below
```

`sqlx-cli` is `0.9.0` and matches `Cargo.lock`'s `sqlx 0.9.0`.

| Commit | Removed (old query) | Added |
|---|---|---|
| (b) | `query-036b42bd…json` (`claim_run`'s `SELECT status, target_box_id, repo_scope … FOR UPDATE`); `query-6c5dc37b…json` (the `EXISTS … repo_scope && $2`) | the same `SELECT` plus `graph_snapshot`; the `SELECT id, graph_snapshot, repo_scope … ORDER BY queued_at, id` |
| (c) | — | `take_lease`'s `UPDATE` (its follow-up reuses `query-21106a8a…json`, `SELECT 1 FROM run WHERE id = $1`, byte-identical) |
| (d) | `query-7292ad5f…json` (`adopt_runs`' `WITH swept AS`) | the new `adopt_runs`; `interrupt_step`'s `UPDATE` (its follow-up reuses `step_exists`' `SELECT 1 FROM run_step WHERE id = $1` literal, `pg/write.rs:157`) |

The count goes from **224** today to **226** expected (3 removed, 5 added), plus one per new `query!` literal a `pg_criteria.rs` race adds. **Any other deletion in `git status` means a target was missed. Restore it and re-run with `--all-targets --all-features`.**

---

## 13. Count pins that move

| Pin | Now | After | Where (pre-edit) | Task |
|---|---|---|---|---|
| `htui-core` store `CASES` | 49 (counted, `conformance.rs:36-86`) | 50 → 51 → 52 | list `crates/htui-core/src/store/conformance.rs:36-86`; dispatch `run_case` from `:97` (new arms); pin literal `crates/htui-core/tests/mem_store.rs:37`, message `:38-42` (add "MOD-4 milestone 5's three: the isolation and path rules, `take_lease`, `interrupt_step`") | T2 (b, c, d) |
| `htui-store` `EXPECTED_CASES` | 49 | 50 → 51 → 52 | `crates/htui-store/tests/pg_conformance.rs:19` | T2 (b, c, d) |
| `READ_CASES` | 9 (counted, `conformance.rs:204-214`) | 9 | `mem_store.rs:45-46` | — |
| `htui-orch` `CASES` | 36 (counted, `conformance.rs:186-268`) | 42 → 52 | list `:186-268`; doc `:164-185` ("Thirty-six" `:166`, name `:167`); test name `:3154`, literal `:3161`, message `:3162-3169`; `crates/htui-orch/tests/fake_conformance.rs:15` (name) and `:16` (literal) | T6, T7 |
| `WriteStore` methods | 63 (counted, `traits.rs:195-991`) | 65 | unpinned | T2 |
| `ReadStore` methods | 16 | 16 | `traits.rs:66-183`, unpinned | — |
| `EngineError::ClaimRefused` Display | `claim refused: the box is full or the scope overlaps (ANA-2 §4.7)` | `claim refused: {claim}` | `command.rs:241`, test `:901-904` | T6 |
| `RunFailure` Display rows | 9 variants | 10 | `status.rs:75-95`, test `:349` | T7 |
| `.sqlx` files | 224 | 226 (+ any new `pg_criteria` literal) | `crates/htui-store/.sqlx/` | T2 |
| Migration pins | `vec![1, 2, 3, 4]`, `Pending(4)` | unchanged | `crates/htui-store/tests/migrations.rs:74`, `:591`, `:623`; `crates/htui-store/tests/connect.rs:95` | — |

**Recount, do not append.** The orch pre-milestone-4 entries are 18 (list positions 1–18, `feat_walks_end_to_end` … `cancel_cleans_up_once`), but the doc's categories (7 + 4 + 1 + 3 + 1 + 3) sum to 19. The in-test message calls thirteen fan-out cases "twelve": criteria 8 (1), 9 (2), 10 (2), gated and judgeless (2), failed candidate (1), no survivor (1), review loop (1), the two caps (2) and the serialised sibling (1), which sum to 13. The rewritten doc and message enumerate: 18 pre-M4, 5 M4 stage-1, 13 M4 fan-out, 6 M5 lease/overlap and 10 M5 recovery, 52 in all.

---

## 14. Data flow

### 14.1 Admission: two worktree runs on one repo, disjoint paths

| # | Call | Rows / effect |
|---|---|---|
| 1 | `StartRun(A)` → `graph::resolve` → `overlap::resolve(item.touched_paths = ["src/**"], repos, phases, None)` | `(repo_scope = [core], RunScope { core: {isolated, !local, ["src/"]} })`; `snapshot.scope = Some(..)` |
| 2 | `create_run` | run A `queued`, item `queued`, `graph_snapshot` JSONB holds `scope` |
| 3 | `claim_run(A, box, owner, now, now + ttl)` | lock run A; lock `box` `FOR UPDATE`; `queued` and targeted; running 0 < 2; prefilter (live on box ∩ `[core]`) is empty → `Admitted`; A `running`, lease written; item `in_progress` |
| 4 | `walk_leased(A, run_to_rest)` | stages 1–6 under the heartbeat; the gate parks → run `awaiting_approval`; re-read → `release_lease` (`lease_expires_at = now`) |
| 5 | `StartRun(B)` with `["docs/**"]` → `claim_run(B)` | running 0; prefilter → `[A]`; `overlaps(scope_of(B), scope_of(A))`: both isolated, `docs/` vs `src/` → `None` → `Admitted` |
| 6 | `StartRun(C)` with `["src/lib/**"]` → `claim_run(C)` | prefilter `[A, B]` in `(queued_at, id)` order; A gives `Paths` → `Overlaps { with: A, rule: Paths }`; nothing written → `Err(ClaimRefused { run: C, claim })` |
| 7 | later: `Engine::claim(C)` (milestone 6 calls it when A rests) | repeats 3–4 |

### 14.2 A crash and the sweep: an unfinished `worktree` step

| # | Call | Rows / effect |
|---|---|---|
| 1 | Process P1 walks `prd` attempt 1: `pending → running`, `prepare`, `upsert_step_tree`, `record_commits(before = H)`, session | the step is `running`; the lease is P1's until `t0 + ttl` |
| 2 | P1 dies (a dropped future or a kill) | nothing further is written; the lease stops being refreshed |
| 3 | P2 at `t > t0 + ttl`: `sweep()` → `adopt_runs(box, P2, now, now + ttl)` | the run's `lease_owner = P2` (never P1's own sweep, D88) |
| 4 | (A-3) a step is `running` → no frontier | — |
| 5 | `classify`: no `prd` document → not finished; the tree is `worktree` → `Reset` | — |
| 6 | `isolator.reset(step, trees)` | empty report; the tree is untouched (OQ-8) |
| 7 | `interrupt_step(step, "interrupted", now)`; note N1 | step `failed`, `gate_note = interrupted`, `gate_outcome` NULL |
| 8 | `may_attempt(2, 1)` → `admit(run, snapshot, prd, 2)` | `prd` attempt 2 `pending` |
| 9 | D96: nothing parked → `Next::Walk` | — |
| 10 | Caller: `resume(run)` → (A-1 take: a renewal) → topology check → `walk_leased(run_to_rest)` | attempt 2 prepares from `H` (the primary never merged attempt 1) |

---

## 15. Workspace gate (every task after T7, and T8)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
( cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_check \
    cargo sqlx prepare --check -- --all-targets --all-features )
cargo doc --workspace --no-deps --keep-going    # exactly the two baseline errors (traits.rs:887, pg/write.rs:3316)
cargo doc -p htui-orch --no-deps --all-features # exits 0
cargo doc -p htui-core --no-deps --all-features # no new error beyond MIRRORED_TABLES
```

`--test-threads=1` is not optional: the keyring fake is process-wide (project memory).

---

## 16. Decisions settled in this blueprint (continuing after D106)

| # | Decision |
|---|---|
| **D107** | `walk_leased` `Box::pin`s both the walk and the heartbeat. On `Left(Ok)` it re-reads the run and releases the lease iff the status is not `running`. On `Left(Err)` it writes nothing more. On `Right(Abandoned)` it drops the walk first, then `isolator.release(run)` (warn on error), then `LeaseLost`. It is generic over the walk's output, so one wrapper serves every command tail and (A-7) the sweep. |
| **D108** | `take_lease` runs after a command's pure guard and before its **first** write (F-N). `Ok(false)` → `LeaseHeld` with nothing written. A failure after the take leaves our lease to expire at TTL. |
| **D109** | `Claim` and `OverlapRule` are `Copy + Eq`, and `Claim` is `#[must_use]`. The `Display` bytes are §3.1's table. `scope_of` unions a decoded scope with conservative entries for any `repo_scope` repo it lacks. |
| **D110** | T2 lands the final predicate with the signature change (commit b), because scope-less rows answer exactly as today under the conservative reading. Each new store case lands in its own commit with its two pins. |
| **D111** | `PgStore::claim_run` decides overlap in Rust over `id, graph_snapshot, repo_scope` rows ordered `(queued_at, id)`, with the box `FOR UPDATE` held throughout. `MemStore` sorts `live` the same way. |
| **D112** | The `NotFound` follow-ups of `take_lease`/`interrupt_step` reuse the existing literals `SELECT 1 FROM run WHERE id = $1` and `SELECT 1 FROM run_step WHERE id = $1` byte-for-byte, so they add no `.sqlx` file. |
| **D113** | The inherent `release(step)` becomes `release_step` in both isolators. `FakeIsolator::held` is keyed `(RunId, StepId)` (F-H, F-I). |
| **D114** | `GixIsolator::reset`: check every in-place row (dirt, `HEAD`, the label via `branch_target`) before any write. A label elsewhere is `label_conflict`. Create the label only when absent, then `reset --hard <base_ref>` under `with_retry`. Take no guard, and never read a `worktree`/`copy` path. |
| **D115** | The sweep's bytes: `gate_note` exactly `interrupted` / `interrupted, tree not reset`; notes N1/N2/N3; `RunFailure::Interrupted`'s `Display` (§9.3). |
| **D116** | `recover.rs`'s exact surface (§5.1), including `Adjudication::Finished { verify, verify_exit_code }` and `verify_of`'s `class == VERIFY_CLASS` filter, because MOD-11's queue writes other classes. |
| **D117** | `recover_run`'s order (§9.3), with A-3's frontier first when it is accepted. `interrupt_step` → `Ok(false)` skips the step and leaves the rest to D96's re-derivation. |
| **D118** | `restarted()` shares the store, clones scripts and candidates, gets a fresh isolator, verifier and owner, advances the clock by `RESTART_GAP = 10 min`, and never carries stalls. |
| **D119** | `overlap::resolve`'s rules (§7.1): `UnknownTouchedRepo` fires with or without a requested scope; a requested scope keeps its order and filters prefixes; with no primary, a bare glob is dropped. |
| **D120** | `EngineError::{ClaimRefused { run, claim }, LeaseLost, LeaseHeld}` bytes (§8.1). |
| **D121** | (Maintainer, 2026-09-23, from T3's verifier finding T3-V2.) `GixIsolator::reset` also refuses a `shared_serialized` row that it would reset when the working tree holds an untracked, not-ignored path that is tracked at `base_ref`. It refuses with `dirty_tree_not_reset: <path>`, writes nothing, and the engine takes D93's path. Reason: `git::is_dirty` excludes untracked files, and `reset --hard <base_ref>` silently overwrites an untracked file whose path `base_ref` tracks (reproduced with plain git). An untracked path that `base_ref` does not track survives the reset, so it does not refuse. The claim at `git.rs:1259` that "a reset never deletes an untracked file" is corrected. |

---

## 17. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | `SQLX_OFFLINE = "true"`: a `query!` edit without its `.sqlx` file fails every build, including other tasks'. | §12 in the same commit; `git status .sqlx` against the table. |
| H-2 | Wave A′'s two worktrees mean two `target/`s, and memory records disk pressure crashing Postgres. | `df -h /` before dispatch and before T2's Postgres gate. |
| H-3 | The heartbeat's `tokio::time::sleep` needs a runtime with a time driver. Every engine entry now requires one. | Every shipped harness is `#[tokio::test]` (`time` is enabled). State it in `walk_leased`'s doc for milestone 6. |
| H-4 | Under `start_paused`, an idle runtime auto-advances to the next timer, so the heartbeat fires as soon as the walk is quiescent. | Only T5's heartbeat tests and T6's abandon test are paused. No conformance case is. |
| H-5 | `seam_clock()` ties `queued_at` within a case, so "the first overlapping run" rests on `RunId` order. | F-E: distinct `queued_at` in the new case. |
| H-6 | `FakeIsolator::prepare` yields for `shared_serialized` (`fake.rs:308`), and `now_or_never` stops there. | A-2; if declined, no T7 stall uses `shared_serialized`. |
| H-7 | A command that took the lease and then failed a write leaves a live lease on a parked run for up to TTL, so another process's answer reads `LeaseHeld` meanwhile. | Accepted (D108); the `Display` says so. |
| H-8 | `#[must_use]` on `Claim` makes `mem.rs:7195`'s discarded answer a `-D warnings` failure. | §11 row 15. |
| H-9 | The plan's `other => return Err(..)` is an unused binding. | `claim.is_admitted()`. |
| H-10 | `create_branch` is `MustNotExist`: a pre-existing label elsewhere is an `Err`, not a no-op. | D114's `branch_target` read first. |
| H-11 | `select` needs `Unpin`, and dropping `select`'s output does not drop a stack-pinned walk (the fact-check's probe). | `Box::pin` both, and `drop(walk)` explicitly (D107). |
| H-12 | T8's two `GixIsolator`s have separate D43 guard tables. The first process's held guard is not released by anything. | Intended: the second process never waits on it. Cases never reuse the first isolator after the stall. |
| H-13 | `MemStore::overlapping_runs` / `PgStore::overlapping_runs` (`mem.rs:659`, `pg/read.rs:1729`) still answer "shares a repo". Their doc "what §4.7's overlap refusal names" becomes a superset. | Unchanged, because no caller in this milestone. Record the doc drift for milestone 6's Runs tab. |
| H-14 | `feature_snapshot_matches` compares the whole snapshot, so T4's `scope` key must be spelled exactly (`{"repos": {}}` for the repo-less demo project). | Regenerate from the resolver's output, then diff: one added key only. |
| H-15 | `mem.rs`'s `adopt_runs` closure reads `self.lease_owners` while iterating `self.runs`. | Both are shared borrows inside `&mut self`'s method; collect ids first as today (`:3310-3318`). |

---

## 18. Risks (continuing after R-20)

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-21** An interrupted step parked with its retry budget spent has no resume verb: `retry_enabled` refuses it (`command.rs:412-418`) and only `CancelRun` remains (F-C) | Low (it needs `retry_limit = 0` or a second crash) | A-8 closes it; otherwise milestone 6's `Unblock`-shaped verb |
| **R-22** Without A-4, one adopted run's recovery error aborts the sweep, and D88 keeps this process's later sweeps from re-adopting the runs it already holds | Low | A-4; otherwise a process restart (R-12's shape) |
| **R-23** Without A-5, two concurrent `adopt_runs` serialise on row locks and depend on READ COMMITTED's re-check; lock-order inversion is possible in principle | Low | A-5's `ORDER BY … FOR UPDATE SKIP LOCKED` |
| **R-24** Without A-1, `resume` on a run a live stranger holds double-walks until the first beat (up to `lease_refresh_seconds`) | Low (milestone 6 resumes only adopted runs) | A-1 |
| **R-25** A failed run-terminal cleanup is never retried: the sweep adopts only `running` runs, and three sentences in the tree promise otherwise (F-Q) | Medium (a failed `rm` or `worktree remove`) | Sentences rewritten in T3/T6; the retry is milestone 6's (Runs tab `cleanup_run`, which is `pub` for that reason) |

---

## 19. What this milestone does NOT do

- No `run_worker.rs`, no Runs tab, no promotion, no close-out, no criterion 19 (D103): all milestone 6.
- No re-attempt of a single interrupted candidate (D95; M4 D48's deviation stands).
- No pid recording or signalling of a `SIGKILL`ed orchestrator's agent (R-10).
- No migration, no `cache_migrations`, no `.snap`, no `Cargo.toml`/`Cargo.lock`, no `crates/htui/**`, no `crates/htui-core/src/prompt/**`, no `isolate/git.rs` verb, no `git gc`, no `git worktree prune` (D102).
- No change to `gate.rs`, `select.rs`, `fanout.rs` or `verify.rs` (`gate.rs:935`'s stale sentence is recorded, F-Q).
- No `queue.rs` (MOD-12).

## 20. Gate checks that are not tests (continuing C-14)

- **C-1**: `grep -rn 'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` and `verify.rs` only. `grep -rnE '"gc"|worktree prune' crates/htui-orch/src/isolate/git.rs` finds no spawned verb.
- **C-2**: these are all empty:
  - `git diff --stat main -- crates/htui-store/migrations crates/htui-store/cache_migrations 'crates/**/*.snap' crates/htui crates/htui-core/src/prompt crates/htui-orch/src/isolate/git.rs crates/htui-orch/src/gate.rs Cargo.toml Cargo.lock`
- **C-3**: `git diff main -- crates/htui-orch/tests/fixtures/feature-with-verify.snapshot.json` is empty. `git diff main -- crates/htui-orch/tests/fixtures/feature.snapshot.json` adds exactly the `"scope"` key.
- **C-4**: both orch pins read 52, and `mem_store.rs:37` and `pg_conformance.rs:19` read 52. `READ_CASES` reads 9.
- **C-5**: `grep -nE 'claim_run|RunScope|Claim|GraphSnapshot \{' crates/htui-orch/src/recover.rs` is empty (D106).
- **C-6**: `grep -rn 'LEASE_SECONDS\|resolve_scope' crates/htui-orch/src/` is empty. `grep -rn "milestone 5's sweep is the" crates/htui-orch/src/` is empty.
- **C-7**: `ls crates/htui-store/.sqlx | wc -l` equals 226 plus the number of new `query!` literals in `pg_criteria.rs`.

---

## 21. Review round (rust-reviewer over `main..mod-4-m5`, 2026-09-23)

`rust-reviewer` returned **request-changes**. The main thread checked the cited code for every HIGH before the maintainer saw it. The maintainer **accepted the recommended bundle** on 2026-09-23. Verifier observations K1–K4 were raised during implementation and are decided in the same round. Each fix is test-first. The first failing test is named in each row.

### 21.1 Fixed in milestone 5

| # | Finding | Decision (D122–D138) |
|---|---|---|
| **H1** | `recover::heartbeat` warns and loops on a refresh `Err` (`recover.rs:104-111`), so a walk keeps writing after its lease lapsed. A stranger's sweep can adopt the run, fail attempt 1 and admit attempt 2. The stale walk then reaches `gate.rs:389`, ignores `transition_step`'s `false`, and merges attempt 1 anyway. `from_app` accepts a refresh as high as `ttl - 1`. | **D122** The heartbeat self-fences. It tracks the last `until` it wrote successfully and returns a new `Heartbeat::Expired` once `clock.now() >= last_until - margin` (margin = `refresh`). `walk_leased` treats `Expired` like `Abandoned`: drop the walk, `isolator.release(run)`, return `LeaseLost`. **D123** After an `Err`, the heartbeat retries sooner (`refresh / 4`, at least 1 s) rather than waiting a full interval. **D124** `LeaseTimes::from_app` clamps `refresh` to at most `ttl / 3`. **D125** Every `transition_step` / `transition_run` compare-and-set on a walk path honours `Ok(false)`, starting with `gate.rs:389`: the landing is then a new `EngineError::StaleWrite { .. }`, and nothing further (no reconcile) is written. This supersedes ANA-2's offline narrative ("the live session keeps running"). A walk that cannot prove its lease stops. First failing test: `a_walk_whose_refresh_keeps_failing_stops_before_the_lease_lapses`. |
| **M1** | `Engine::sweep` adopts every run's lease in one `adopt_runs(.., now + ttl)`, then recovers the runs one at a time, and heartbeats only the current one (`engine.rs:1024-1039`). | **D126** Each iteration calls `take_lease(run)` before `recover_run`. On `false` the run is skipped (`Next::Error("lease taken")` is not written, no item_note). |
| **M2** | A run whose walk errored in this process keeps this process's lease, and D88 stops this process's sweep from re-adopting it. | **D127** On `walk_leased`'s `Left(Err)`, release the lease best-effort (`refresh_lease(run, owner, now)`, warn on error), so any process's sweep can adopt the run after it expires. A-4's error path already does this. |
| **K3** | In the sweep, `walk_leased`'s `Err(LeaseLost)` takes A-4's `unrecovered` path, which writes an `item_note` after the lease is gone. | **D128** `LeaseLost` in the sweep gives `Next::Error` with a `tracing::warn!` only: no item_note and no release. |
| **L2** | `walk_leased`'s post-walk `self.run(run).await?` turns a successful walk into an error when the re-read fails. | **D129** The re-read failure is warned. The lease is then released best-effort, and the walk's `Ok` is returned. |
| **L3** | `take_lease`'s `Ok(false)` always reads as `LeaseHeld` ("another orchestrator holds a live lease"), even for a queued or terminal run, or one on another box. | **D130** `resume`/unpark re-read the run on `Ok(false)`. Only a live foreign lease is `LeaseHeld`; any other state returns the existing status error (`RunStatus { .. }`). |
| **H2** | A crash between a step's `Running → Failed` and `admit` / `park` / `finish_run` (`gate.rs:533-535`; the sweep's `interrupt_step` before `admit`/`park_interrupted`) leaves the run `running`. The cursor answers `Rest { Failed }` (`status.rs:255-259`). The sweep says `Walk`, `resume` rests at once, and the run stays `running` forever. | **D131** A `running` run whose latest step at the cursor is `failed` is unfinished gate work, both in `recover_run` and in `run_to_rest`. Cases: budget left gives `admit`; a `gate_note` starting with `interrupted` gives `park_interrupted`; anything else gives `finish_run(Failed)` + `cleanup_run`. First failing test: `a_crash_after_the_failed_write_is_finished_by_the_sweep`. |
| **H3** | A crash between a command's first write and `unpark` (`answer_gate`, `retry_step`, `retry_group`, `select_fanout`) leaves an `awaiting_approval` run whose parked step is already `done` / `superseded` / `selected`. Every guard refuses it. This predates milestone 5. | **D132** `resume` on an `awaiting_approval` run whose cursor is *not* `Rest { AwaitingApproval }` (i.e. `Create`, `Run`, `Finished`, or a `done` frontier) unparks the run and walks it under the lease. First failing test: `a_crash_between_the_answer_and_the_unpark_is_resumed`. |
| **M3** | `answer_gate`'s `Ok(false)` from its first write is discarded (`engine.rs:569-572`), so a stale answer unparks whatever gate the run is parked at now. | **D133** Every command honours `Ok(false)` from its first compare-and-set write. It releases the lease it took and returns `StaleWrite`. A per-run in-process mutex is deferred to milestone 6 (R-27). |
| **M4** | A-8's exemption admits a `RetryStep` on any interrupted row, including an older attempt. | **D134** `retry_enabled` requires the row to be `latest_at(steps, position)` (it already refuses others for the group path with `StaleSlot`). |
| **K2** | After request-changes, `gate.rs:424-425` returns `Landing::Advance` for the retired review step, and the engine reconciles it. A review agent's commits reach the primary checkout. | **D135** `review_loop`'s `Resumed` lands without a reconcile of the retired review step. The review's branch stays labelled, and nothing is merged. First failing test (real git): `a_rejected_reviews_commits_never_reach_the_primary`. |
| **H4** | Rule P admits two isolated runs with disjoint prefixes on one repo concurrently. But `reconcile_isolated` refuses whenever `HEAD != base_ref` and HEAD is not already this step's merge (`real.rs:1057`), so the second run to reconcile always parks with `primary_moved`. | **D136** `reconcile_isolated` accepts a moved `HEAD` when `base_ref` is an ancestor of `HEAD` and the primary is clean. It then merges `after` on top of `HEAD` (`merge --no-ff`), and a real conflict takes the existing merge-conflict path. The "already merged" check becomes "some merge on first-parent history from HEAD back to `base_ref` has `after` as its second parent". First failing test (real git): `two_disjoint_isolated_runs_on_one_repo_both_reconcile`. |
| **K1** | `reset_to_slot_base` (`real.rs` ~:433) and `reconcile_in_place` (~:1134) check only `is_dirty` before `reset --hard`. | **D137** Both apply D121's guard (`git::untracked_paths_base_tracks` against their reset target) and refuse with `dirty_tree_not_reset` on their existing refusal paths. |
| **L1** | A `reset --hard` that fails on a later row, after earlier rows were labelled and reset, is noted "tree not reset". | **D138** The `never_reset` reason names the rows that were already reset. |

### 21.2 Deferred to milestone 6 (risks)

| Risk | Likelihood | Handling |
|---|---|---|
| **R-26** (M5) Dropping a walk sends SIGKILL to a live `git merge`/`reset --hard` child (`git.rs:328`, `kill_on_drop(true)`). That can leave `index.lock` or `MERGE_HEAD` in the primary checkout. | Low (it needs a lease loss during a primary write) | Milestone 6: run verbs that change the primary on a detached task and await the handle. |
| **R-27** (M3) Commands in the same process are not fenced against each other: `take_lease` always succeeds for the owner. | Medium once milestone 6's TUI issues commands concurrently | Milestone 6: a per-run mutex in the engine. D133 already closes the stale-answer half. |
| **R-28** (M6) `cancel_run` writes and removes trees without taking the lease, and `refresh_lease` does not check the run's status. | Low | Milestone 6, already scoped there by §8.3. Also consider `status IN ('running','awaiting_approval')` in `refresh_lease`. |
| **R-29** (L4) Postgres stores `queued_at` in microseconds and `MemStore` in nanoseconds, so a sub-microsecond tie can name a different `Overlaps.with`. | Very low | Recorded only. |
| **R-30** (K4) `recover::classify` counts a step as finished only when every `run_scope` repo has an `after_hash`, so a step that changed only some repos and crashed after capture is retried, not adopted. | Low; the failure is safe (a retry, never a wrong merge) | Recorded as a known limitation. |

### 21.3 Checks corrected

- **C-6** as written cannot pass: `GixIsolator::resolve_scope` (`real.rs:348`) is an unrelated private method. Read C-6 as: `LEASE_SECONDS` absent, and `resolve_scope` only in `isolate/real.rs`.
- Plan D85's "a refresh of `0` reads as `ttl / 2`" is superseded by §5.3/F-U (a `0` refresh is unset and gives 60 s), and now also by D124's clamp.

## 22. Review round 2 (rust-reviewer over `fcdb217..829586f`, 2026-09-23)

The second `rust-reviewer` pass over the round-1 repairs returned **request-changes**. The main thread checked H-A and M-A in the code before the maintainer saw them. The maintainer **accepted the recommended bundle** on 2026-09-23: fix H-A, M-A, M-B, L-a, L-c and L-d; record L-e as R-31 and L-f as R-32. L-b (a hung refresh can still commit on the server) is closed by D139. Each fix is test-first.

### 22.1 Round-1 deviations from §21 (as landed)

- **D131** is a five-branch order: rejected → `rejection()`; not reset → park; budget left → `admit`; interrupted → park; anything else → `finish_run`. It adds `RunFailure::RetryBudgetSpent` (`Display`: "retry budget spent: step `{phase}` attempt {n} failed") and the N2 reason `PARK_CUT_SHORT`.
- **D132** unparks only when the cursor is `Create`, `Run` or `Finished`. A crash right after `AnswerGate(Rejected)` stays parked on a failed step (R-31).
- **D133** cannot cover `select_fanout`: its store write returns `()`.
- **D134** applies to every retry, not only the A-8 exemption (`StaleSlot`).
- **D135** adds `Landing::Retired`. The C129 case moved `stall_nth_reconcile` from 5 to 4.
- **D124**: the default beat is now 40 s.
- **D126**: a lost re-take is omitted from the sweep's `Vec`, not reported as `Next::Error`.
- **D138** adds the `already_reset` text to the `never_reset` reason.
- **D136** also holds the repo's admin lock from `reconcile_isolated`'s `HEAD` read through its merge (commits `a35d69b`, `829586f`).
- **C-6**'s grep must exclude `GixIsolator::resolve_scope` (§21.3).
- About five intermediate commits fail `clippy -D warnings` on their own (e.g. `74c10b5`, `e58d508`, `135dacd`, `b5f0573`). HEAD is clean. Squashing before merge is the maintainer's call.

### 22.2 Fixed in milestone 5

| # | Finding | Decision (D139–D145) |
|---|---|---|
| **H-A** | M2 is not closed for the owning process. `Engine::release_lease` (`engine.rs:1147`) calls `refresh_lease(run, owner, now)`, so `lease_owner` stays this process, and `adopt_runs` skips `lease_owner IS DISTINCT FROM $2` (D88). This process's own sweep never re-adopts a run whose walk died. D122 makes this common: a store outage of more than one fence strands every running walk until restart. | **D139** A new `WriteStore` verb `release_lease(run, owner, now) -> Result<bool>`: `UPDATE run SET lease_owner = NULL, lease_expires_at = $3 WHERE id = $1 AND lease_owner = $2`. `Ok(false)` = zero rows = not ours; `NotFound` is told apart by the reused `SELECT 1 FROM run WHERE id = $1` literal. `Engine::release_lease` uses it on every release path (walk `Err`, `release_after_walk`, `stale_first_write`, the sweep's `unrecovered`). A refresh that hangs and commits late then matches no row (closes L-b). One new conformance case: store pins 52 → 53 (`mem_store.rs`, `pg_conformance.rs`). `.sqlx` 226 → 227. **D140** An in-process dead-walk set, a process-owned value the engine borrows. A run enters it when its walk ended on `Heartbeat::Expired` (never `Abandoned`: a stranger holds that lease), or when a `release_lease` failed. `sweep` first retries `release_lease` for every run in the set and drops each one that answers `Ok(_)`, then calls `adopt_runs`, which now sees those runs as free. D88 still holds for every walk that is live. First failing test: the D127 test extended, so a second `sweep()` from the same process adopts the run. |
| **M-A** | Introduced by D136. A step merged onto a moved primary records `(base_ref, merge)`. `GixIsolator::diff_of` (`real.rs:1279`) diffs `base..merge`, which includes another run's merge, and that diff becomes the next attempt's `previous_diff` (`engine.rs` ~:4053). | **D141** When `after` is a merge whose second parent is the step's tip, `diff_of` diffs `after^1..after`. First failing test (real git, `gix_isolator.rs`): two disjoint runs; the second's diff names only its own paths. |
| **M-B** | `git::is_ancestor` (`git.rs:1265`) and `git::merge_of` (`:1294`) walk to the root while `reconcile_isolated` holds the repo's admin lock (`real.rs` ~:1152–1179), which blocks prepare and cleanup on big repos. | **D142** `is_ancestor` hides `ancestor` in its rev walk, and `merge_of` stops at `base`'s commit time. First failing test: a history below base that the walk must not visit (a walk-count assertion or a long history). |
| **L-a** | The heartbeat fence starts from `clock.now()` at `walk_leased` (`recover.rs` ~:120), not from the `until` the lease take actually wrote. | **D143** `walk_leased` and `recover::heartbeat` take the `until` that the preceding `claim_run` / `take_lease` / `adopt_runs` wrote, and the first fence is `until - margin`. |
| **L-c** | Some compare-and-sets are still unchecked: `answer_gate` right after `move_step` (`gate.rs` ~:422, `engine.rs` ~:3717), and the `Pending → Running` moves at `engine.rs` ~:2162, ~:2695, ~:3208. | **D144** Each of them honours `Ok(false)` as `EngineError::StaleWrite`, as D125 says. Where a conversion is not sound, D125's doc is narrowed to say which writes are exempt and why. |
| **L-d** | A `StaleWrite` in the sweep goes through `unrecovered` (`engine.rs` ~:1221) and writes an `item_note`. | **D145** The sweep treats `StaleWrite` like `LeaseLost` (D128): `Next::Error` and a `tracing::warn!` only, with no item_note. |

### 22.3 Deferred to milestone 6 (risks)

| Risk | Likelihood | Handling |
|---|---|---|
| **R-31** (L-e) `walk_resumed` (`engine.rs:1870`) ignores `unpark`'s `false`. It re-merges on every resume of a run parked by a refused reconcile: the merge is idempotent, but it writes a note each time. Only one of D132's four crash paths is tested. A crash right after `AnswerGate(Rejected)` stays parked on a failed step. | Low | Milestone 6: honour `unpark`'s answer, test the other three crash paths, and give the rejected crash a resume path. |
| **R-32** (L-f) D131's not-reset park loses its labelled detail, and D138's `part_way` turns an `Io` error into a `Git` error. | Low (both are diagnostics only) | Milestone 6: keep the detail and the error kind. |
