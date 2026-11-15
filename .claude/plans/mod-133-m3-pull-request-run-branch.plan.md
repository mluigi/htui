# Plan: MOD-133 M3 — the `pull_request` run branch

**Source PRD**: `.claude/prds/mod-133-integration-modes.prd.md`
**Selected Milestone**: 3 — `pull_request` run branch
**Complexity**: Large
**Status**: complete 2026-11-15 (`50345b9c..30ad4529`; review fix round R1, brief `R1.md`)
**Routing**: plan path on the accepted PRD (C2 fired: the walk's step base and reconcile target change;
C1, C3, C4 not), no ultracode by the verdict. The lane script runs (more than two tasks), as one serial lane.
A code-architect blueprint ran (C2) and its decisions are folded in below (D1-D8). `/handoff-run` waived
this skill's "WAIT for CONFIRM". Maintainer decision 2026-11-15: an existing `htui/item/<key>` is
**reused** (D1).

## Summary

A repo whose mode is `pull_request` now runs. Its run's `worktree` and `copy` steps base on, and
reconcile into, a run branch `htui/item/<key>` in the primary repository instead of the checked-out
`HEAD`. The run branch is cut from the primary's `HEAD` at the first step. Steps merge into it in a
detached landing worktree, and the ref moves by compare-and-swap, so the primary's `HEAD` and files never
move. After `done`, the executing box pushes the run branch (never forced). The record then stays
`pending` with the pushed head in `landed_hash`, sweeps leave it alone, and it waits for MOD-134 to open
the pull request. Dependents keep waiting (not landed), and the item reads `⇡ landing`. The snapshot
refuses `pull_request` when any phase isolates `local` or `shared_serialized`. Settings keeps
`pull_request` unselectable (PRD Q1), so M3 is reachable through the store, the API and tests only.
`local` and `push` runs are byte-for-byte unchanged, and there is no migration.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-orch/src/graph.rs:362-381` | one `pub fn <refusal>(..) -> String` per refusal sentence, `#[must_use]` |
| Errors | `crates/htui-orch/src/isolate/real.rs:1199-1286` | refusals are `IsolateError::Refused(<sentence fn>)`, git failures `IsolateError::Git`; blocking git under `blocking(..)`, retried verbs under `git::with_retry` |
| Git verbs | `crates/htui-orch/src/isolate/git.rs:589, 632, 784, 1084, 1269-1325, 2187-2199` | `Cli` verbs with a post-condition (`add_worktree --lock --reason`, `merge_no_ff`), gix ref edits with `PreviousValue`, push argv never forced |
| Store | `crates/htui-core/src/store/mem.rs:8852-8925`, `crates/htui-store/src/pg/write.rs:10751-10872` | take/settle/retry of `integration`, guarded by `state = 'pending' AND lease_owner = owner` |
| Tests | `crates/htui-core/src/store/conformance.rs:21354-21878`, `crates/htui-orch/tests/it/gix_isolator.rs`, `crates/htui-orch/tests/it/landing.rs` | conformance cases over Mem and Pg; real-git isolator tests against local bare remotes; engine landing cases |

## Shared decisions

- **D1 — the run branch: cut, reuse, refuse.** New trait method
  `Isolator::run_branches(&self, branches: &BTreeMap<RepoId, String>) -> IsolatorFuture<'_, BTreeMap<RepoId, String>>`
  (match the trait's existing future alias) makes sure each `refs/heads/<branch>` exists and answers its
  tip. It runs per repo under that repo's admin lock:
  - absent → create it at the checkout's `HEAD` (`git::create_branch`, `MustNotExist`, `git.rs:2187`; a
    concurrent create loses safely);
  - an ancestor of `HEAD`, or equal to it → fast-forward it to `HEAD` (compare-and-swap on the old value);
  - diverged → keep it as it is (**maintainer: reuse**; a re-run continues the same pull request);
  - checked out in any worktree, the primary included → refuse
    `` run_branch_checked_out: `{branch}` is checked out at {path}; switch that tree off it `` (an
    `update-ref` would leave that tree stale: probe C12).

  It is called lazily, every time the engine computes a step base for a run that has run branches (D2), so
  there is no run-start hook. All the state lives in git, so a crash, resume or adoption simply ensures
  again. The name is frozen into the snapshot: `IntegrateSnapshot.run_branch: Option<String>`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`, so `push` snapshots keep their bytes),
  built by a pure `htui_core::model::run_branch_name(key: &str) -> Result<String, String>`
  (`format!("htui/item/{key}")`, refusing any key character outside `[A-Za-z0-9._-]`, an empty key and a
  `..`). Real keys always pass (`item.rs:224-229`, `kind.rs:169-185`), so this is only a guard.
  `htui/item/<k>` cannot collide with the `htui/<step-uuid>` labels. Two projects that share one checkout
  and have equal keys share a branch: accepted, and documented on `run_branch_name`.
- **D2 — step base.** New `Engine::run_branch_base(&self, run: &Run, snapshot: &GraphSnapshot) ->
  Result<Option<BTreeMap<RepoId, String>>, _>`. It answers `None` when `land::run_branches(snapshot)`
  (new pure fn: `integrate` entries with `run_branch: Some`) is empty, so `local` and `push` runs make no
  extra isolator call and FakeIsolator call logs stay byte-identical. Otherwise it answers
  `isolator.base(&run.repo_scope)` with `isolator.run_branches(..)` laid over it. Three sites use it:
  - `walk_live_step` (`engine.rs:5809-5826`): when integrate gives no slot and the base is `Some(tips)`,
    prepare with `Some(FanoutSlot { index: 0, width: 1, base: &tips })`; `candidate_live` (`:7196-7199`)
    gets the same treatment;
  - the `group_base` fallback (`:6996`), for fan-out groups;
  - `admit_parts` (`:7535-7539`): it feeds `base_refs`, so divided parts (`part_live` `:7934`) and the
    integrate step (`IntegrateStage::slot` `:11197`) follow the run branch.

  A width-1 slot is safe for `worktree`/`copy`: there a slot only fixes the start commit (`real.rs:208-218,
  475-485, 809, 945`) and sizes the copy budget (`:948`, width 1 → 1). The base covers every repo of the
  scope, so `no_slot_base` (`:217`) never fires. `local` refuses a slot, which D4 rules out. The judge,
  `reopen_trees` and `capture_turn` prepare an empty scope (`:8992`, `:6206`, `engine/capture.rs:322`) and
  are untouched. `FanoutSlot`'s doc gains a line: a `pull_request` step's width-1 slot holds run-branch
  tips.
- **D3 — reconcile onto the run branch: a detached landing worktree plus a compare-and-swap ref move.**
  - **Trait.** New `pub struct RunBranches<'a> { pub run: RunId, pub branches: &'a BTreeMap<RepoId, String> }`.
    New required `Isolator::reconcile_onto(winner, trees, siblings, onto: Option<RunBranches<'_>>)`.
    `reconcile` becomes a provided method calling `reconcile_onto(.., None)`, so its existing test call
    sites stay untouched. The four impls rename their `reconcile` to `reconcile_onto`: `real.rs:1895`,
    `fake.rs:1049`, `gix_isolator.rs` `StallAfterReconcile`, and `htui-worker/tests/it/landing.rs` `Watched`.
    The two wrappers delegate `run_branches` too.
  - **Caller.** `reconcile_done_step` (`engine.rs:9245-9280`, the only production caller, `:9255`) decodes
    `Self::snapshot_of(run)` (`:10888`) and passes `Some(RunBranches)` when `land::run_branches` is
    non-empty. Its 8 callers are unchanged.
  - **The landing sequence.** For a `worktree`/`copy` tree whose repo is in `onto`, `reconcile_isolated`
    (`real.rs:1199-1286`) runs this; steps 1-9 run under the admin guard it already holds, so no verb
    here takes the lock again (the tokio mutex is not reentrant):
    1. tip ← `branch_target(local, branch)`; missing → refuse `` no_run_branch: `{branch}` ``; a branch
       checked out in the checkout or any linked worktree (`checked_out_at`, which also reads a rebase or
       bisect in progress) → refuse `run_branch_checked_out` (R1);
    2. `merge_of(local, tip, base_ref, after)`: refs and objects are shared, so D136 recovery works as
       is; found → answer it;
    3. ensure the landing tree `<scratch_root>/<run>/landing/<repo name>`: new
       `Cli::add_detached_worktree(repo, path, run, at)` (`worktree add --lock --reason "htui run <run>"
       --detach <path> <tip>`, mirroring `add_worktree` `git.rs:589`, with a post-condition). A tree
       that is listed and present is reused; otherwise `remove_worktree` (which tolerates a missing tree,
       `git.rs:654-676`) and add again;
    4. landing `HEAD` ≠ tip, a merge in progress (`MERGE_HEAD`: a `git merge` killed with a conflicted
       index and `HEAD` still at the tip) or a dirty tree → `abort_merge`, then `reset_hard` it to the tip
       (scratch tree, nothing to lose) (R1);
    5. a reset that fails, or leaves the tree dirty, refuses a new `` dirty_landing_tree: {path} `` (never
       `dirty_primary_tree`, which would blame the person's checkout);
    6. ancestry checks as today, but refusing a new `` run_branch_moved: `{branch}` is at {short} ``
       where today's code says `primary_moved`;
    7. `copy` → `copy_range` into `local` (the odb is shared with the landing tree);
    8. `merge_no_ff(landing, step, tip, after)` (`git.rs:784`): post-condition, hooks, abort on
       conflict and the `merge_conflict` park, unchanged;
    9. `checked_out_at` again (R1), then move the ref: new gix `git::move_branch(repo, branch, from: &str, to: &str)` with
       `PreviousValue::MustExistAndMatch` (probe: a mismatched compare-and-swap is refused). A failed CAS
       is `IsolateError::Git`, which parks the run; the next attempt merges again.

    A crash between steps 8 and 9 leaves an orphan merge, which step 4 resyncs away. A crash after
    step 9 is found by step 2. A `local`/`shared_serialized` tree with `onto` set refuses
    `` run_branch_in_place `` (defence in depth behind D4).
  - **Cleanup.** `cleanup` (`real.rs:1976-2030`) removes each `<run_dir>/landing/<name>` with
    `git worktree remove` under the admin lock before it removes the run directory, and adds those repos
    to the D46 stale-entry check. The run branch survives cleanup.
  - The primary's `HEAD`, index and files are never touched.
- **D4 — snapshot.** `frozen_integrate` (`graph.rs:516-588`) gains `item_key: &str, phases:
  &[SnapshotPhase]` (one caller, `resolve` `:~470`). Its `PullRequest` arm (`:570-575`) works like this:
  - Any phase whose `isolation` (`run.rs:981-982`, resolved and frozen) is `Local` or `SharedSerialized`
    refuses with `integrate_pull_request_in_place(repo, phase, isolation)`:
    `` "`{repo}` integrates by `pull_request`, which needs `worktree` or `copy` isolation, but phase `{phase}` runs `{isolation}`: set the phase's isolation, or set the repo to `local` or `push`" ``.
    The first such phase in snapshot order is named.
  - Otherwise `run_branch: Some(run_branch_name(item_key)?)` is stored. A bad key refuses with
    `integrate_run_branch_invalid(repo, key)`:
    `` "`{repo}` integrates by `pull_request` but item key `{key}` is not a branch name" ``.

  `integrate_pull_request` (`graph.rs:376-381`, its only user is `:573`) is deleted, and the
  `IntegrateRefused` doc (`:345-350`) is updated. Every phase counts because the scope is per run and every
  phase prepares the whole `run.repo_scope`.
- **D5 — the record after the push: `pending`, with `landed_hash` set.** There is no new state, no
  migration and no cache change: the CHECK never moves (M1 D1), and readiness reads `state` only (fact C19).
  - **Outcome.** New `LandOutcome::Pushed { hash: String }`: the run branch is on the remote at `hash`.
  - **Settle.** `settle_integration` with `Pushed` keeps `state = 'pending'`, sets `landed_hash`, clears
    the lease, and is guarded like the other outcomes (`state = 'pending' AND lease_owner = owner`). It also
    requires `mode = 'pull_request'`, and answers `false` otherwise. Mem `mem.rs:8905-8914`, Pg
    `pg/write.rs:10816-10848`.
  - **Take.** `take_integrations` skips `mode = 'pull_request' AND landed_hash IS NOT NULL`: Pg
    `:10751-10800` (`AND NOT (g.mode = 'pull_request' AND g.landed_hash IS NOT NULL)`), Mem
    `:8852-8889`, trait doc `traits.rs:~2234`. MOD-134 widens this to open the PR → `pr_open`.
  - **What does not change.** `is_landed` is false, so dependents keep waiting at every site. `RunLanding`
    reads `Pending`, so the item shows `⇡ landing`; there is no waiting row (only `refused` makes one).
    `retry_landing` moves only `refused` rows, and a Pg refusal writes `landed_hash = NULL` (Mem leaves it
    `None`), so a retried record pushes again.
  - **Push.** `LandRequest` gains `run_branch: Option<String>`. `GixIsolator::land` (`real.rs:2162-2194`)
    and `landing_tip` (`:2222-2240`), when it is `Some(b)`:
    1. read `refs/heads/<b>`; missing → `Refused { reason: "no_run_branch: …" }`;
    2. the run's commits must be its ancestors (`not_on_target`, naming the branch);
    3. if the tip is an ancestor of the local `refs/heads/<target_branch>` → `Landed { hash: tip }` with
       nothing pushed (an empty run branch, reachable only when every committing step lost a fan-out);
    4. `ls_remote(remote, b)` / `holds_past` find the work already there → `Pushed { hash }` with nothing
       pushed;
    5. otherwise `push(cwd, remote, tip, b)` (`git.rs:1302`, a non-forced `<tip>:refs/heads/<b>` that
       creates the branch) → `Pushed { hash: tip }`;
    6. `NonFastForward` refuses
       `` non_fast_forward: `{b}` on the remote has commits this box lacks; merge them into the local branch, then retry ``.

    `Unreachable` and `Failed` are mapped as in M1, and `run_branch: None` is M1's path unchanged.
  - **Engine.** `land_taken` (`engine.rs:3903-3969`) takes `run_branch` from the run's snapshot
    (`land_reads` `:3973` already reads the run row; `LandReads` gains `run_branches`). For `Pushed` it
    notes ``"integration pushed: repo `{repo}` run branch {b} at {short}; its pull request against {target} waits for MOD-134"``.
- **D6 — close-out.** `integration_section` (`closeout.rs:186`) gains `item_key: &str`; `summary`
  (`:136`) passes `&item.key`. A `pull_request` row keeps `{remote} {target}` in the target cell (the PR
  base). Its outcome cell reads `` pushed `htui/item/<key>` at <short>; pull request not opened (MOD-134) ``
  when `pending` with a `landed_hash`, `pending` without one, and as M1 for anything else. `push` and
  `local` output keeps its bytes.
- **D7 — Settings unchanged.** `selectable_refusal` stays (`integration.rs:56-70`, PRD Q1). Only
  `IntegrateMode::PullRequest`'s doc (`:25`) says the run branch exists and the PR waits for MOD-134.
- **D8 — other readers of the base.** Fan-out goes through `group_base` (D2); divided parts and integrate
  through `base_refs`; `premerge` runs in integrate's own tree, never in place under `pull_request`;
  recovery and re-landing go through `reconcile_done_step` (`recover.rs` is pure); `reset`/`reacquire` are
  in-place only. Nothing else changes.

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/{integration,run,mod}.rs` | UPDATE | T1 |
| `crates/htui-core/src/store/{traits,mem,conformance}.rs`, `crates/htui-core/tests/it/mem_store.rs` | UPDATE | T1 |
| compile-break literals: `crates/htui-orch/src/graph.rs:578`, `crates/htui-worker/tests/it/landing.rs:184` (`run_branch: None`), `crates/htui-orch/src/engine.rs:3955-3964` (`land_taken`'s `Pushed` arm, see T1) | UPDATE | T1 |
| `crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/**`, `crates/htui-store/tests/it/pg_conformance.rs` | UPDATE | T2 |
| `crates/htui-orch/src/isolate.rs`, `src/isolate/{git,real}.rs`, `src/fake.rs`, `tests/it/gix_isolator.rs`, `crates/htui-worker/tests/it/landing.rs` (`Watched`), `crates/htui-orch/src/engine.rs` (`land_taken`, `land_reads`), `src/land.rs` | UPDATE | T3 |
| `crates/htui-orch/src/graph.rs`, `crates/htui-orch/tests/it/landing.rs` (refusal case) | UPDATE | T4 |
| `crates/htui-orch/src/engine.rs` (bases, `reconcile_done_step`), `src/closeout.rs`, `tests/it/landing.rs` (new cases), `tests/it/run_branch.rs` + `tests/it/main.rs` | CREATE/UPDATE | T5 |

## Tasks (one serial lane)

Sandbox: Postgres `localhost:5439` (trust); Qdrant on a tmpfs, so live-Qdrant tests run normally. No agent
runs the workspace gate. Every task fixes the compile breaks its own type changes cause in any crate, and
says so in its report. The lane is serial, so T3 and T5 both touch `engine.rs` (different functions)
without conflict.

### T1 (foundation) — htui-core model, store trait, MemStore, conformance
- **Files**: UPDATE `crates/htui-core/src/model/{integration,run,mod}.rs`,
  `crates/htui-core/src/store/{traits,mem,conformance}.rs`, `crates/htui-core/tests/it/mem_store.rs`;
  compile-break fixes `crates/htui-orch/src/graph.rs:578` and `crates/htui-worker/tests/it/landing.rs:184`
  (`run_branch: None`), and `crates/htui-orch/src/engine.rs` `land_taken`'s exhaustive match: give
  `Pushed` its real note from D5 there, reading the branch as `` `the run branch` `` until T3 supplies the
  name (T3 replaces the wording).
- **Do**: `LandOutcome::Pushed`; `IntegrateSnapshot.run_branch` (D1 serde attrs); `run_branch_name` (D1),
  re-exported beside the other integration items; the `IntegrateMode::PullRequest` doc (D7); Mem take
  filter and `Pushed` settle (D5); trait docs. Conformance cases (Mem here, Pg in T2):
  `settle_pushed_keeps_a_pull_request_record_pending_with_its_hash`,
  `settle_pushed_answers_false_for_a_push_record`, `take_integrations_skips_a_pushed_pull_request_record`
  (and still takes an unpushed `pull_request` one), `a_pushed_pull_request_record_holds_its_dependents`
  (ready/released stays false; the item's landing reads `Pending`),
  `retry_landing_leaves_a_pushed_record_alone`. Move the `CASES` pin in `mem_store.rs:64-65` (349 + 5).
  Unit tests: `run_branch_name` accepts real keys (`MOD-7`, `AB12-300`) and refuses `a/b`, `a..b`,
  `a b`, `a@{b`, `""`; a snapshot without `run_branch` serialises exactly as before (no `run_branch`
  key), and one with it round-trips.
- **Check**: `cargo nextest run -p htui-core --all-features`; `cargo check -p htui-orch -p htui-worker
  --all-features --tests`; clippy and rustfmt on the crates touched. `htui-store` stops compiling until T2
  (its exhaustive `LandOutcome` match, `pg/write.rs:10823`; expected).
- **Verify**: foundation.

### T2 — Postgres
- **Files**: UPDATE `crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/**`,
  `crates/htui-store/tests/it/pg_conformance.rs`.
- **Do**: D5's take filter and `Pushed` settle in Pg (mode guard on the settle); regenerate `.sqlx` per
  `docs/hr-sandbox.md` (prepare against a migrated scratch database, not the empty `htui` one); move
  `EXPECTED_CASES` (`pg_conformance.rs:79`) to T1's count.
- **Check**: `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'`;
  `cargo sqlx prepare --check` from `crates/htui-store` (per `docs/hr-sandbox.md`); clippy and rustfmt.

### T3 (foundation) — isolator: run branches, reconcile onto, land of a run branch
- **Files**: UPDATE `crates/htui-orch/src/isolate.rs`, `src/isolate/{git,real}.rs`, `src/fake.rs`,
  `src/land.rs`, `src/engine.rs` (`land_taken`, `land_reads`, `LandReads` only),
  `tests/it/gix_isolator.rs`, `crates/htui-worker/tests/it/landing.rs` (`Watched` only).
- **Do**: D1 (`run_branches`, `git::move_branch`, a `checked_out_at(main, full_ref) -> Option<PathBuf>`
  helper over the primary's and each linked worktree's `HEAD`); D3 (`RunBranches`, `reconcile_onto` +
  provided `reconcile`, the landing sequence, `Cli::add_detached_worktree`, cleanup, the three new refusal
  sentences as `pub fn`s beside `primary_moved`); D5's land side (`LandRequest.run_branch`, `land`,
  `landing_tip`, `land::run_branches(&GraphSnapshot)`, `land_taken`'s `run_branch` from `land_reads` and
  its final `Pushed` note). FakeIsolator: `run_branches` answers the same synthetic tips as `base` and
  logs `IsolatorCall::RunBranches`; a separate `reconcile_onto_log` records `onto`, so
  `IsolatorCall::Reconcile` (`fake.rs:231`) and existing logs keep their shape. Real-git tests in
  `gix_isolator.rs`: `run_branches_cuts_at_head_then_reuses`,
  `run_branches_fast_forwards_a_branch_behind_head`, `run_branches_keeps_a_diverged_branch`,
  `run_branches_refuses_a_branch_checked_out_in_the_primary`,
  `reconcile_onto_merges_into_the_run_branch_and_leaves_the_primary_head_and_tree` (worktree),
  `reconcile_onto_copy_mode`, `reconcile_onto_conflict_aborts_and_leaves_the_branch`,
  `reconcile_onto_is_idempotent_after_the_ref_moved` (D136),
  `reconcile_onto_recovers_a_merge_whose_ref_move_never_happened`,
  `reconcile_onto_refuses_a_dirty_landing_tree_naming_it`,
  `cleanup_removes_the_landing_tree_and_keeps_the_branch`,
  `land_pushes_the_run_branch_to_a_bare_remote_once_and_answers_pushed` (second call answers `Pushed`
  with no push), `land_refuses_a_run_branch_that_moved_on_the_remote`,
  `land_of_an_empty_run_branch_is_landed_without_a_push`, `the_run_branch_push_argv_never_forces`.
- **Check**: `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^gix_isolator::/)'`;
  `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^landing::/)'`;
  `cargo nextest run -p htui-orch --lib --all-features`;
  `cargo nextest run -p htui-worker --all-features -E 'binary(it) and test(/^landing::/)'`; clippy and
  rustfmt.
- **Verify**: foundation (T5 builds on the trait and the landing sequence). Attack: D136 recovery under
  the landing tree, the CAS failure path, a push that could force, the admin lock taken twice.

### T4 — snapshot: lift the refusal, refuse in-place isolation, freeze the run branch
- **Files**: UPDATE `crates/htui-orch/src/graph.rs`, `crates/htui-orch/tests/it/landing.rs` (the
  `an_unlandable_mode_refuses_the_run_at_start` case only, `:498-504`).
- **Do**: D4. Flip the `pull_request` case of `an_unlandable_mode_refuses_the_run_at_start` to a
  `pull_request` repo whose phase isolates `local`, pinning the new sentence. Graph unit tests:
  `pull_request_freezes_its_run_branch` (worktree default → `run_branch: Some("htui/item/<key>")`),
  `pull_request_refuses_a_shared_serialized_phase_naming_it`, `a_push_snapshot_keeps_its_bytes`.
- **Check**: `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^landing::/)'`;
  `cargo nextest run -p htui-orch --lib --all-features -E 'test(/graph::/)'`; clippy and rustfmt.

### T5 — engine wiring and close-out
- **Files**: UPDATE `crates/htui-orch/src/engine.rs` (`run_branch_base`, `walk_live_step`,
  `candidate_live`, `group_base`, `admit_parts`, `reconcile_done_step`; not T3's land functions),
  `crates/htui-orch/src/closeout.rs`, `crates/htui-orch/tests/it/landing.rs` (new cases); CREATE
  `crates/htui-orch/tests/it/run_branch.rs` and register it in `tests/it/main.rs`.
- **Do**: D2, D3's caller, D6. Engine cases (FakeIsolator) in `landing.rs`:
  `a_pull_request_run_bases_steps_on_its_run_branch_and_reconciles_onto_it`,
  `fan_out_and_divided_parts_follow_the_run_branch`,
  `a_pull_request_run_settles_pushed_and_the_sweep_leaves_it`, `its_dependent_stays_unready`,
  `a_push_run_makes_no_run_branches_call` (the call log is byte-identical to today's). Close-out units:
  `a_pull_request_row_reads_its_pushed_branch`, plus `push`/`local` bytes unchanged. `run_branch.rs`
  (real git, `GixIsolator`): a two-phase `worktree` run against a local bare remote. The primary's `HEAD`
  and tree do not move, `htui/item/<key>` holds both merges, the remote holds the branch, and the record
  is `pending` with its hash.
- **Check**: `cargo nextest run -p htui-orch --all-features`; `cargo check --workspace --all-features
  --tests`; `cargo clippy --workspace -- -D warnings` (featureless, memory: the `--all-features` build
  hides dead code); rustfmt.

## Validation

```bash
bash scripts/gate.sh          # main thread, once after the lane
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The landing tree drifts from the branch (crash between merge and ref move) | medium | step 4 resets it to the tip; step 2 finds a landed merge first; tests for both |
| A person checks out `htui/item/<key>` | low | `run_branch_checked_out` refusal before any ref move |
| FakeIsolator logs move for `local`/`push` runs | medium | `run_branch_base` answers `None` with no isolator call; pinned by `a_push_run_makes_no_run_branches_call` |
| `.sqlx` drift | medium | T2 prepares against a migrated scratch DB; `prepare --check` |
| The engine's 2 MiB debug stack (memory: orch test stack headroom) | low | new awaits in `walk_live_step` stay boxed where their neighbours are |

## Verified claims

Fact-check 2026-11-15: one agent read the tree for C1-C23 at `c4f3eb9d`, and the main thread probed git
2.43 in `/tmp` for C7, C8, C10, C11 and C12. The amendments are folded in above.

| claim | verdict | evidence |
|---|---|---|
| C1 four `Isolator` impls; `reconcile` has no run, `base` only the scope | true | `fake.rs:922`, `real.rs:1732`, `gix_isolator.rs:2018` (`StallAfterReconcile`), `htui-worker/tests/it/landing.rs:557` (`Watched`); `isolate.rs:265-269`, `:~221` |
| C2 a slot only fixes the start commit and the copy budget for worktree/copy | true | `real.rs:208-218, 475-485, 809, 945, 948`; `no_slot_base` `:217` → D2 covers every scope repo |
| C3 every phase prepares the whole scope; judge/reopen pass `&[]` | **partly → D2** | also `candidate_live` `:7196-7199` (added to D2) and `capture_turn` `engine/capture.rs:322` (`&[]`) |
| C4 `admit_parts` → `base_refs` → integrate slot; `group_base` falls back to `base` | true | `engine.rs:7535-7539, 6312-6350, 11197, 6996, 5809-5826` |
| C5 `reconcile_done_step` is the one production caller, 8 callers, has `run` | true | `engine.rs:9245, 9255`; callers `:1869, 1896, 2372, 4046, 4376, 5421, 6087, 8397`; `snapshot_of` `:10888` |
| C6 `MIN_GIT` 2.33.0, so `merge-tree --write-tree` (2.38) is out | true | `git.rs:37` |
| C7 a linked worktree shares the odb and refs | true (probe) | merge made in the landing tree resolves in the primary (`cat-file -t` → commit) |
| C8 `merge --no-ff` in a detached worktree makes a two-parent commit | true (probe) | `rev-list --parents -n1` → 3 words |
| C9 gix CAS via `PreviousValue::MustExistAndMatch` | true (indirect) | `create_branch` uses `MustNotExist` `git.rs:2193`; gix 0.87.1 cited with `MustExistAndMatch` at `git.rs:2877`; probe: `update-ref <new> <old>` refuses a mismatch |
| C10 `worktree add --lock --reason --detach` at the floor | true | already used by `add_worktree` `git.rs:589`, `add_worktree_on_branch` `:632`; probe ok |
| C11 non-forced push creates a new remote branch; names taken verbatim | true (probe + tree) | `[new branch]`; a diverged push → `[rejected] (non-fast-forward)`; `git.rs:1084, 1269, 1302` |
| C12 moving a ref checked out elsewhere leaves that tree stale | true (probe) | the step tree showed a change after `update-ref` |
| C13 `landed_hash` writers; refusal; retry; take/settle guards | **partly → D5** | Pg settle writes NULL on refusal, Mem leaves `None`; closeout reads it (`closeout.rs:217-220`); cache mirrors it (`cache/refresh.rs:1619-1662`); take: `pending`, executing box, lease free; settle: `pending` + lease owner |
| C14 exhaustive `LandOutcome` matches | true | `mem.rs:8905-8914`, `pg/write.rs:10823-10828`, `engine.rs:3955-3964` |
| C15 struct literals and fields | true | `IntegrateSnapshot` (`run.rs:769`) literals `run.rs:2375`, `conformance.rs:21336`, `graph.rs:578`, `htui-worker/tests/it/landing.rs:184`, no skip attrs today; `LandRequest` (`isolate.rs:155`) literals `engine.rs:3919`, `gix_isolator.rs:4586` |
| C16 default isolation is `worktree`; `SnapshotPhase.isolation` is resolved | true | `kind.rs:684`; `run.rs:981-982` |
| C17 cleanup removes the run dir; stale check only for worktree rows | true | `real.rs:1976-2030, 2009-2012, 2016-2018` |
| C18 item key shape and immutability | true | `item.rs:224-229`, `0001_init.sql:316`, `kind.rs:169-185`; `update_item` never sets it (`pg/write.rs:3036`) |
| C19 readiness and `RunLanding` read `state` only | **partly** | true for Pg/Mem/cache read paths; Mem `waiting_candidates` `:2496` filters `Refused` (unaffected); the cache mirrors `landed_hash` but never reads it for readiness |
| C20 `land_taken`/`land_reads` shape | true | `engine.rs:3903-3969, 3973`; notes only when settle answers `true` |
| C21 `integration_section` has no item; `summary` holds it | true | `closeout.rs:186, 136, 146, 177` |
| C22 `CASES` pins at 349 | true | `mem_store.rs:64-65`, `pg_conformance.rs:79` |
| C23 refusal test calls `integrate_pull_request` | **false → T4** | the test pins a string literal (`landing.rs:498-504`); the fn's only user is `graph.rs:573` |
| Task coupling | serial lane | T1 → T2 → T3 → T4 → T5; T3/T5 share `engine.rs` (disjoint functions); T1/T4 share `graph.rs:578` (literal only) |

## Acceptance
- [ ] T1-T5 complete; the gate is green in the sandbox
- [ ] `local`/`push` snapshots, call logs and close-out bytes unchanged
- [ ] the review gate (`rust-reviewer`) has run and its findings are applied or deferred
