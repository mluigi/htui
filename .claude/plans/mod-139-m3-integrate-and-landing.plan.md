# Plan: MOD-139 M3 — Integrate and landing

**Source PRD**: `.claude/prds/mod-139-divided-fan-out.prd.md`
**Selected Milestone**: 3 — Integrate and landing
**Complexity**: Large

## Summary

A divided node's integrate step runs as an agent session:
1. Its tree is prepared from the admitted plan's `base_refs`.
2. A new isolator verb pre-merges the done parts in plan order. In each repo it stops at the first conflict.
3. The agent gets the pre-merge report, each part's document and its diffstat. It finishes the merges and writes the
   divided node's output document.
4. After the session, a deterministic post-check proves that every done part's tip is an ancestor of integrate's.
5. `verify_command` gates the step.
6. Only the integrate branch lands, through the unchanged `reconcile`.

Integrate recovery follows the plain step test, per isolation mode, plus the post-check.

Resolve stops refusing divided nodes. It still refuses a planner agent (PRD Q1, MOD-140). It now refuses, at resolve,
every admission refusal it can decide from the snapshot. M2's test-only seam is deleted, and every divided
conformance case runs to its real end through a real enqueue.

Grounding: ANA-33 §7.4 (part documents), §7.5 and §7.7; M2 plan D1–D17 and blueprint rulings F-1..F-8 with "Deferred
from review". Three code fact sheets were taken at `9423f50b`. Four checkers then fact-checked a first draft of this
plan (workflow `wf_a831d4e2-7ee`), and every correction is folded in below (§ Fact-check).

## Confirm (maintainer decisions)

**Decided 2026-11-11 (plan confirmed): 1 item note; 2 (a) the path-overlap check; 3 refuse approval.**

1. **Where the pre-merge report is kept (D4).** Recommended: an item note tied to the integrate step. It needs no
   migration. No planned consumer reads the report structurally: MOD-142 shows a part's key, agent and state, and
   MOD-140's `plan_status` lists state, branch, tip, diffstat and document id. If either one ever needs the report,
   it adds a column then. The alternative is a `run_step.premerge` JSONB column now, which costs migration 0029, the
   cache mirror, `.sqlx`, seven count pins and a store verb.
2. **"Ancestry proves commits, not changes" (PRD risk row; ANA §9).** The PRD's mitigation is "per-part diffstat to
   integrate and review". Integrate gets its half (D5). For review's half, M3 has three choices:
   - **(a) Recommended:** a cheap deterministic check added to the post-check. In each repo, a done part's changed
     paths must intersect integrate's own base..after changed paths. A whole-part `-s ours` merge then fails
     `integrate_incomplete`. No review change.
   - **(b)** Give the review node a per-part diffstat section.
   - **(c)** Defer, and amend the PRD risk row.
3. **Approving a failed integrate (D6).** Under `Gate::Always`/`OnFailure`, a failed integrate parks, and
   `answer_gate(Approved)` would land it without the unmerged part. Recommended: refuse `Approved` on an integrate step
   whose gate note is `integrate_incomplete`, and point the operator at Retry. This holds PRD's "a missing part is
   never routed around".

## Decisions

**D1 — Integrate is a session step on the plain path, with hooks.**
- **Entry points:** `drive_cursor`'s `Create` and `Run` arms stop returning `integrate_waits` (`engine.rs:4205-4207`,
  `:4228`), and `integrate_waits` is deleted. Integrate runs through `admit` (role `integrate` via `divide::row_role`)
  and `walk_step`.
- **The hooks:** `walk_live_step` gains integrate-only hooks, each a no-op for `NodeRole::Step`, so linear and compete
  steps are unchanged. Every new await is boxed (the stack rule).
  1. **Prepare** takes `FanoutSlot { index: 0, width: 1, base: &base_refs }`, the parts' pattern
     (`engine.rs:6394-6399`, `:6575-6580`). Under `local` the slot is `None` (`real.rs:668` refuses a slot there).
  2. **Pre-merge** (D3) runs after the tree rows and before commits are recorded, and before stage 3.
  3. **Stage 3:** `assemble_prompt` (`engine.rs:8069`) gains an `integrate: Option<&IntegratePrompt>` argument, which
     carries the report and the parts blocks. Its `MissingInput`/`Refused`/`NoPersona` exits are unchanged. A stage-3
     refusal after a pre-merge leaves merge commits in the tree; that is harmless, because a retry re-prepares from
     base.
  4. **Post-check** (D6) runs after capture and before the settle, and only when the session returned `Ok`, as verify
     does.
- **Shared inputs:** the hooks share one `IntegrateInputs`. A pure `divide::integrate_inputs(snapshot, node, &steps,
  &plans)` selects the planner row, its admitted plan and its latest done parts; `divide.rs` stays "no store, no
  clock". An engine method reads the rows once, calls it, and decodes `base_refs`/`body` through a helper factored out
  of `drive_parts` (`engine.rs:6307-6342`), so the parts round and integrate cannot diverge. Finding nothing is an
  engine error, never a fallback: integrate is only reached after `parts_state` read the node `Done`.

**D2 — Where part commits are reachable.** In each mode a part's tip must be named or present in integrate's tree:
- **worktree:** the `htui/<part_step>` label moves with the part's commits, in the shared ref store.
- **shared_serialized:** capture writes the label in the primary when the part committed (`real.rs:1006-1022`). A part
  with no commits has no label and `after_hash` None.
- **local:** at most one part exists. It ran on the checked-out branch and the round does not reset after it
  (`engine.rs:6476-6497` gates the reset on SharedSerialized). Integrate prepares in place with slot `None`, so its
  `before_hash` is the part's tip (`real.rs:441-451`).
- **copy:** a part's label inside its own copy stays at the base (`real.rs:960-965`, `:1113-1115`), and the primary
  has no such ref. ANA's "fetch each part's label" would therefore bring nothing. Instead, the pre-merge first copies
  every part's range into the integrate copy with `git::copy_range(part_copy, integrate_copy, before, after)`
  (`git.rs:1960`, objects only, the `reconcile_isolated` precedent at `real.rs:1197-1206`). That includes conflicted and
  not-attempted parts. It then writes `htui/<part_step>` at the tip in the integrate copy; `create_branch` accepts a
  label already at that target (`git.rs:1814-1825`). The part copy's path comes from the part's `run_step_tree` rows.
  Part copies still exist while integrate runs, because `cleanup` is run-terminal.

**D3 — `Isolator::premerge`.**
- **Signature:** `premerge(step, trees: &[RunStepTree], parts: &[PartTip]) -> IsolatorFuture<PremergeReport>`, where
  `PartTip { step, key, trees: Vec<RunStepTree>, commits: Vec<RunStepCommit> }` comes in plan order.
- **Rules, per repo of the integrate tree, in plan order:**
  - no `after_hash` → `NoCommits`;
  - a tip already an ancestor of the tree's HEAD (`git::is_ancestor`, `git.rs:1496`) → `Contained`. This covers the
    single `local` part, and a part reached through another part's merge;
  - otherwise merge the part. The first conflict is reported `Conflicted { paths }` (the merge is aborted), and every
    later part in that repo is `NotAttempted`.

  Repos are independent.
- **Merging:** the git layer gets a public detached `merge_no_ff_as(path, message, before, after) -> MergeAttempt`, with
  `enum MergeAttempt { Merged(Merged), Conflicted(Vec<String>) }`. `merge_no_ff(step)` delegates to it.
  `reconcile_isolated` keeps mapping `Conflicted` to `Refused(merge_conflict(..))`, so the paths are no longer
  recovered from a string. Calls go through `with_retry("merge", ..)`, as reconcile does, and re-read HEAD before each
  part (it is the post-condition's `before`). The subject is `htui: premerge <part_step>`. That keeps it readable; no
  code path at HEAD would misread a reconcile subject there (`reconcile_parent` is asked only of worktree/copy rows,
  `real.rs:1361-1378`).
- **Locking:** in-place trees (shared_serialized, local) take `admin_guard(repo)` around each part's HEAD read, merge
  and post-condition, as `reconcile_isolated` does (`real.rs:1159`). Another run's reconcile into the same primary
  would otherwise trip the two-parent post-condition. Worktree and copy trees own their HEAD.
- **Types:** `PremergeReport`/`PartMerge` live in `htui-core` (`model/run.rs`, serde), so the prompt renderer
  (htui-core) and the isolator (htui-orch) share them. `PartMerge` is `Merged { commit }`, `Contained`, `NoCommits`,
  `Conflicted { paths }` or `NotAttempted`.
- **Errors:** any `premerge` error, refusal or not, fails the step and the run through `walk_step`'s `fail_hard` (no
  retry), as a prepare error does. A conflict is not an error: it is the agent's work.

**D4 — Report storage (Confirm 1).** Recommended form: an item note via `note(item, body, via_step_id)`
(`engine.rs:4934`). It is written once per attempt, recovery re-runs included, so it is at-least-once like substitution
notes. Its text comes from the same htui-core renderer as the prompt section.

**D5 — Two prompt sections, `premerge` and `parts`.** Both are internal placeholders: not in `ALL`, Phase-only, set
only for integrate, and both carried in `IntegratePrompt`.

`premerge` is **protected** and mirrors `Part`:
- `PREMERGE_SEPARATOR` is pushed into the frame estimate whenever it renders.
- It lists per repo each part's key, label or hash and outcome.
- Conflicted paths are capped deterministically: the first 20 in byte order, then "and K more", in both the section
  and the note. A huge conflict therefore cannot refuse the very step meant to resolve it.

`parts` is **trimmable** and mirrors `Memory`:
- `PARTS_SEPARATOR` is counted in `span_tokens`, written by `substitute` only while the section is live, and kept out
  of the frame.
- It has its own `Source::Parts(Vec<PartState>)`, re-rendered from the masked spec (the C-1 rule, `mod.rs:1014-1036`).
- Each block is one part's key, title, its tip per repo, its diffstat, and its document body.
  - The diffstat is `isolator.diff` over the part's trees and commits, stat only, embedded with `diff_content(&block,
    true)`. A diff error becomes a trim note, as `forwarded` does (`engine.rs:8469-8481`), and the block renders
    without a stat.
  - The body is the highest `version` of the part's output-kind document, read with `run_steps`, then `documents(item)`
    filtered on `produced_by_step_id`, then `document(id)`. A part with no document renders its header and stat
    without a body.
- Its trim ladder works on the last part first: head+tail body to a floor, then drop bodies (keeping key, title, tips
  and stat), then drop the section. It uses the existing `TrimStrategy` values only.
- `TrimStep::Parts` sits in R-PRM-3's "prior documents" rank: after `Documents`, before `Item`. The `trim_order` doc
  says so, and the order pin moves.

**Render order** (a byte contract, pinned): persona, memory, part, premerge, parts, body spans, output.

**Touch points:**
- `PromptSpec` fields;
- `SectionName` (`render`, `is_protected` gains `Premerge` only);
- `Placeholder` (token, `allowed_in`'s Phase exclusion gains both, `is_section`);
- `scrubbed_inputs` masking;
- `render_sections`;
- `assemble` frame and `span_tokens`;
- `substitute`;
- `source_of`;
- `Trimmer::run`;
- `trim_diff`;
- `trim_order` and its pin;
- the `SectionName` render and protected-set tests;
- every `PromptSpec` literal (count them with `git grep`).

Tests:
- `tests/prompt_parts.rs` mirrors `prompt_part.rs`: placeholders are internal, render order, each trim rung, the
  protected premerge, the path cap, masking.
- Part-less specs keep the `prompt_golden__*` snapshots unmoved.
- A twin test pins that a handoff spec built from an integrate spec carries neither section.

**D6 — Post-check, failure and approval.**

The isolator read `unmerged(trees, heads: &[RunStepCommit], parts: &[PartTip]) -> Vec<StepId>` returns the parts that
fail the check:
- **The rule:** per repo, a part fails when its tip is not an ancestor of the descendant `after_hash.unwrap_or(before)`
  (capture records `None` when HEAD == base). A part with no commits is exempt.
- **Repository:** it opens the copy tree when it holds the descendant, else the checkout (`range_of`'s D74 rule,
  `real.rs:1344-1358`). It never opens a worktree that capture removed.
- **Confirm 2(a) (adopted):** a part also fails when its changed paths do not intersect integrate's base..after
  changed paths (`changed_paths`, `isolate.rs:208`).

Settle and failure:
- `SettleInput.unmerged: Option<Unmerged<'a>>`, where `Unmerged { node, parts }` carries part keys.
- `gate::settle` places it after `MissingOutput` and immediately before `VerifyFailed`. It therefore outranks verify
  (ANA's post-check-then-verify) but never a driver error, stop, cap or deadline.
- The failure is `StepFailure::IntegrateIncomplete { node, parts }`. Its `Display` is the gate note, and its
  `run_failure()` is `RunFailure::IntegrateIncomplete { node, parts }`, read as ``integrate_incomplete: part(s) `a`,
  `b` of `implement` are not merged``.
- Pins: `run_failure_display_is_ana2s_bytes` (`status.rs:929`) and `step_failure_display_is_the_gate_notes_vocabulary`
  (`gate.rs:1844`).
- About 20 `SettleInput` literals gain `unmerged: None`.

The failure takes the existing paths: a `fails` edge looked up by `divided_name` (`gate.rs:453`), `retry_or_fail`, or a
park under `Gate::Always`/`OnFailure`. Confirm 3: `Approved` is refused on that park.

Inherited and recorded: a crash that cuts the gate's Running→Failed write ends on `RetryBudgetSpent` through
`settle_failed` (D131), as MissingOutput already does. A cut-short `fails` loop spends one more loop turn on replay
(MOD-138 F6 / the F-8 addendum).

**D7 — Landing.** It is unchanged: `Landing::Advance` → `reconcile_done_step(&row, &step, &[])` (`engine.rs:5222`).
- Integrate lands alone; parts never land.
- In copy mode, reconcile's `copy_range` walks `.all()`, so the part commits reach the primary on the second-parent side.
- In-place integrate lands by identity (`real.rs:1241-1245`).

**D8 — Recovery (ANA §7.7), by mode.** `StepKind::Integrate` is classified after `Part`. It shares Plain's `captured`
rule in `classify` (`recover.rs:265-270`).

`(Integrate, Finished)` runs, boxed:
1. `finish_recovered`;
2. the post-check over the recorded commits;
3. `resettle` with `unmerged`;
4. `land_recovered`.

It is checked before landing. `recover_run` reconciles `frontiers` first (`engine.rs:3361`), but a `running` integrate
is not yet a `done` winner, so the frontier cannot land it before the check.

The Reset and NeverReset arms stay plain, and their effect depends on the mode:
- **worktree/copy:** the old tree is abandoned. The next attempt re-prepares from `base_refs` and re-runs the
  deterministic pre-merge, within `retry_limit`.
- **shared_serialized:** a clean checkout is labelled `htui/<integrate>` and reset to base (`real.rs:1738-1767`). A
  crash mid-merge or mid-edit leaves it dirty, and the run parks `interrupted, tree not reset` (D93).
- **local:** a moved HEAD parks `local_moved` (`real.rs:1720-1724`).

The frontier is unchanged: integrate's done row is a `winner_at`, parts are excluded by position, and the planner lands
nothing. No twin sessions: the lease is unchanged.

**D9 — `shared_serialized` resolves M2's deferred reset.** Integrate prepares in place with the `base_refs` slot.
`reset_to_slot_base` refuses dirt and resets to base (`real.rs:461-489`), so the last part's position no longer
matters. The part commits stay labelled. **Residual, recorded:** `reset_to_slot_base` writes no label. A commit someone
else makes on the primary between the round's end and integrate's prepare is dropped from the branch. This is the
existing D56 hazard between parts, and integrate widens the window.

**D10 — The resolve lift (graph.rs).**
- **Deleted:**
  - `not_yet_runnable`'s `lift_divide`, the Divide refusal and `NotRunnable::Divide`;
  - `resolve_divided`, and the flag on `resolve_with`/`resolve_or_refused_with`, with their `too_many_arguments` allows
    where they no longer apply;
  - `engine::enqueue_divided_fake` (and the `lib.rs:69` re-export) and `Orchestrate::enqueue_divided`.
- **Order:**
  1. PlannerAgent stays first.
  2. Then, for every planner-less divided node in position order, resolve refuses what admission would:
     - no default parts → `NotRunnable::NoParts` ("a divided node with no default parts needs its planner, which runs
       from MOD-140 on");
     - a `plan_scope` other than `None`/`Parts` → `NotRunnable::PlanScope` (MOD-140/141);
     - every other refusal → M2's pure `divide::admission_refusal` (local with more than one part, `max_parts`,
       `max_fan_out`, part persona, part agent not a candidate, part path), surfaced as `ResolveError::DivideRefused
       { node, reason }` with the `RunFailure` sentence.

  These checks run in `resolve_or_refused_with` after `not_yet_runnable`, not in `snapshot_phase`, so a planner agent
  still wins.
- **Kept as backstops:** M2's admission checks stay unchanged.
- **Resume:** `resume_window`'s not-comparable arm (`engine.rs:4530-4537`) gains `DivideRefused`.
- **Doc comments** refreshed: `htui-core/src/model/run.rs:1095`, `:1203`, and any other "M3"/"seam" mention
  `git grep` finds.

**D11 — Cases: the real end in T3, the real enqueue in T4.**
- **T3** deletes `stops_at_integrate` (20 sites, 19 cases) and rewrites each site to its case's real end, still through
  the seam:
  - the run completes with integrate landed; or
  - the case holds integrate (`hold_sessions`, or `stall_after_done("implement", 1, ..)` for a crash) where it checks a
    state before integrate.

  `reconciles` assertions gain integrate where it settles.
- **T4** replaces `Orchestrate::enqueue_divided` with `async fn enqueue(&self, item) -> Result<RunId, EngineError>`,
  backed by a new `engine::enqueue_fake` beside `enqueue_in_batch_fake`. It calls `Engine::enqueue(item, Manual, ..)`,
  so `start_enabled`/`missing_tags` run as in production. Every `enqueue_divided`/`start_divided`/`crash_divided` use
  moves to it: 11 at `9423f50b`, plus T3's.
- **Admission refusals move to resolve:** M2's admission-refusal cases decided at resolve are rewritten to expect
  `EngineError::Resolve(..)` with no run: `local_with_two_parts_fails_admission`, `an_empty_divided_node_fails_admission`,
  the cap, persona, agent and path ones. They fold into a renamed `start_run_refuses_a_planner_agent_and_what_admission_would`
  where they duplicate.
- **graph.rs tests:** `a_divide_node_is_expanded_then_refused` and `the_seam_lifts_only_the_divide_refusal` are
  rewritten.
- **Engine recovery cases are MemStore-only:** no orch harness runs on PgStore. The PgStore half of the PRD's
  per-role recovery metric is the store conformance of the verbs recovery reads (M1/M2 cases), plus any new verb's
  cases if D4's column is chosen.
- **Log filter:** `call_log_of` filters like `call.contains(" 5/")` must not catch premerge/unmerged lines, so those log
  only the integrate step's own position.

**D12 — Out of scope, recorded.**
- Substitution notes stay at-least-once (M2).
- `ReviewFanOut`/`fan_out_edge_refusal` do not apply (divided `fan_out` is 1).
- MOD-125's future stop/resume matrix owes an integrate row (note on its HANDOFF line at close-out).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Isolator verb | `isolate.rs:165-269`, `fake.rs:182-195` | boxed `IsolatorFuture`; FakeIsolator logs an `IsolatorCall` first, FIFO script/refuse/stall fields; `call_log_of` arm (`conformance.rs:11316`) |
| Git merge | `git.rs:726-811` | `merge_no_ff_reading`, abort on conflict |
| Copy objects | `real.rs:1197-1206` | `copy_range` before a merge in copy mode |
| In-place lock | `real.rs:1159`, `:537` | `admin_guard` around HEAD read + merge + post-condition |
| Slot prepare | `engine.rs:6394-6399`, `:6575-6580` | `FanoutSlot` from `base_refs`, `None` under `local` |
| Protected internal section | M2 `Part` (`mod.rs:131,197,609-614,664-670,810-813,851,967`, `render.rs:717`) | frame-counted separator |
| Trimmable structured section | `Memory` (MOD-163 D14/D16), `Source::Memory`/`Upstream` (`trim.rs:850-937`) | span-counted separator, re-render from masked spec |
| Failure sentence | `status.rs:243-349` (pin `:929`), `gate.rs:150-156` (pin `:1844`) | `Display` arm + byte pin |
| Recovery arm | `recover.rs:207-216`, `engine.rs:3570-3583` | role-first `StepKind`, finished → resettle → land |
| Resolve refusal | `graph.rs:804-831`, `:1665-1670` | first refusal wins, named node |
| Real-git test | `real.rs:3201`, `git.rs:3622`, `gix_isolator.rs:922` | fixture repos, one test per mode |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/run.rs` (+ `mod.rs`) | UPDATE | `PremergeReport`, `PartMerge`; stale docs |
| `crates/htui-orch/src/isolate.rs`, `isolate/{real,git}.rs` | UPDATE | `premerge`, `unmerged`, `PartTip`, `MergeAttempt`, `merge_no_ff_as` |
| `crates/htui-orch/src/fake.rs`, `tests/gix_isolator.rs` | UPDATE | fake verbs, log, `stall_nth_premerge`/`stall_nth_reset`; `StallAfterReconcile` |
| `crates/htui-core/src/prompt/{mod,template,render,trim,fixtures}.rs`, `tests/prompt_parts.rs` (new) | UPDATE/CREATE | the two sections |
| `crates/htui-orch/src/promote.rs`, `crates/htui/src/preview.rs` | UPDATE | spec literals |
| `crates/htui-orch/src/{engine,divide,gate,status,recover,command}.rs` | UPDATE | integrate path, settle, failure, approval refusal, recovery |
| `crates/htui-orch/src/{graph,lib,conformance}.rs`, `tests/fake_conformance.rs` | UPDATE | lift, seam, `enqueue`, cases, pins |
| (Confirm 1 column only) migrations 0029 / cache 0011, store verb, `.sqlx`, pins | CREATE/UPDATE | the report column |

## Tasks

### Task 1: Isolator — `premerge`, `unmerged` and git (parallel with T2)
- **Action**: D2, D3, D6's isolator half (and Confirm 2(a) if adopted). `PremergeReport` types in htui-core first (T2
  needs them). The trait, `GixIsolator`, `FakeIsolator` (log, scriptable report and unmerged, `stall_nth_premerge`,
  `stall_nth_reset`), and the `StallAfterReconcile` double.
- **Real-git tests:**
  - one per mode: worktree; copy (answers the PRD open question: the two-parent post-condition inside a copy after
    `copy_range`); shared_serialized under `admin_guard`; local, where the single part reads `Contained`;
  - a conflict stops with paths, aborts, and marks the rest `NotAttempted`;
  - a no-commit part;
  - two repos, one of which conflicts;
  - `Contained` through another part's merge;
  - `unmerged` true and false, including a local integrate with no commits of its own (descendant = before);
  - `reconcile_isolated`'s conflict text is unchanged.
- **Validate**: `cargo test -p htui-orch --features test-support --lib -- isolate::`; `--test gix_isolator`.

### Task 2: Prompt — `premerge` and `parts` (parallel with T1, after T1's htui-core types)
- **Action**: D5 in htui-core, plus `PromptSpec` literals elsewhere, and the note renderer (D4).
- **Validate**: `cargo test -p htui-core --features test-support --test prompt_parts --test prompt_part --test prompt_golden --test prompt_digest`; `--lib prompt::`.

### Task 3: Engine — integrate runs, fails, recovers; every divided case to its real end (serial, after T1, T2)
- **Action**: D1, D4, D5's engine half, D6, D7, D8, D9, Confirm 3, and D11's T3 half. New cases, MemStore:
  1. a two-part run completes with one integrate landing;
  2. a conflicted pre-merge reaches the agent (prompt section and note) and the run lands;
  3. an unmerged part fails `integrate_incomplete` and retries;
  4. an unmerged part with a `fails` edge loops back over the planner to revision 1;
  5. integrate verify fail;
  6. a crash mid-pre-merge (`stall_nth_premerge`, worktree, `retry_limit` ≥ 1) re-prepares and re-runs;
  7. a crash after the session with an unmerged part (repo-less, `stall_after_done` + scripted `unmerged`) fails
     rather than lands;
  8. a crash after integrate done lands through the frontier, with no twin session;
  9. shared_serialized, with a repo, crashes before the round's end reset (`stall_nth_reset`); integrate's
     `before_hash` == `base_refs`;
  10. a dirty in-place integrate parks on recovery;
  11. approving an `integrate_incomplete` park is refused.

  All 19 `stops_at_integrate` cases are rewritten to their real end.
- **Validate**: as M2 §T3.13: the orch suite `--no-fail-fast -- --test-threads=1`, SIGABRT 0, fmt, both clippy runs.

### Task 4: Resolve lift and the real enqueue (serial, after T3)
- **Action**: D10, D11's T4 half. Delete the seam and the Divide refusal. Add `NoParts`, `PlanScope` and
  `DivideRefused`. Add `Orchestrate::enqueue`. Rewrite the admission-refusal cases. Refresh docs and CASES pins.
- **Validate**: as T3, plus `--lib -- graph::`. `git grep -n "enqueue_divided\|resolve_divided\|lift_divide\|NotRunnable::Divide\|stops_at_integrate"` is empty.

### Task 5: Real-git end to end and the workspace gate (serial)
- **Action**: a `gix_isolator.rs` test runs a two-part divided `implement` through the real isolator, under worktree and
  copy. It asserts one integrate merge on the primary and every part's tip an ancestor of it. A two-part conflict case
  is resolved by a scripted agent commit. Then the full gate, `sqlx prepare --check` (column only), and the validator.
- **Validate**: `bash scripts/gate.sh`; SIGABRT grep; Qdrant-live timeouts are owed on the host.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui-orch --features test-support --no-fail-fast -- --test-threads=1   # grep -c SIGABRT = 0
cargo test -p htui-core --features test-support --no-fail-fast -- --test-threads=1
bash scripts/gate.sh
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `merge_no_ff` in a worktree/copy tree (untested) | Low | the checker traced it as path-generic (`git.rs:327-346`, `:1925-1947`); T1 real-git tests per mode before T3 |
| Concurrent reconcile into the same primary during an in-place pre-merge | Medium | `admin_guard` (D3) |
| 19 cases change their end (D11) and hide a regression | Medium | T3 asserts each case's real end; a checker lens compares old vs new assertions |
| A conflict too large for the protected report | Low | the path cap (D5) |
| A large `parts` section | Low | trim ladder (D5); trim record shows it |
| Foreign commit on the primary before an in-place integrate | Low | recorded residual (D9), existing D56 hazard |
| Stack overflow from new integrate futures | Medium | box every new await; SIGABRT grep |
| Engine recovery is MemStore-only | Low | stated (D11); PgStore covers the verbs |

## Acceptance
- [ ] A two-part divided run lands exactly one integrate merge whose history holds every done part's tip (fake + real git)
- [ ] A conflicting pre-merge reaches the agent; the orchestrator writes no resolution
- [ ] An unmerged part fails `integrate_incomplete`; approval cannot land it (Confirm 3); integrate never starts on a partial set
- [ ] Recovery after admission, mid-part, mid-pre-merge and after integrate: no twin session, no re-planning, per-mode reset/park as D8
- [ ] Resolve runs default-parts divided nodes; refuses a planner agent, a part-less divide and every snapshot-decidable admission refusal
- [ ] No seam left; linear and compete suites unchanged; the gate green apart from host-owed Qdrant tests

## Fact-check

Four checkers (`wf_a831d4e2-7ee`) found these against the first draft, now fixed:
- **2 blocking:**
  - `StepFailure::run_failure()` has no phase, so the failure carries `node` (D6);
  - T3 could not pass its gate while the case rewrites sat in T4 (D11).
- **Important:**
  - Recovery reset differs per mode (D8).
  - The post-check's `None` after_hash and repository choice (D6).
  - Conflict paths came back only as a string (D3, `MergeAttempt`).
  - Settle precedence was unstated (D6).
  - The fake had no stall points for the crash cases (T1).
  - Approval of a failed integrate would land it (Confirm 3).
  - `Orchestrate` has no plain enqueue (D11).
  - The admission-refusal cases move to resolve (D11).
  - Caps were not counted at resolve (D10).
  - `LocalDivide` in `snapshot_phase` would beat PlannerAgent (D10).
  - `resume_window` (D10).
  - `parts` must mirror Memory, not Part, and needs a trim source (D5).
  - The touch-point list was incomplete (D5).
  - Unbounded conflict paths (D5).
  - The report types' crate (D3).
  - The D4 MOD-142 claim (Confirm 1).
  - Review's diffstat is a PRD deviation (Confirm 2).
  - In-place pre-merge needs `admin_guard` (D3).
- **Verified unchanged:** D2's copy-mode finding; `copy_range` direction; `merge_no_ff` path-generic; local integrate
  on the part tip; copy landing carries part commits; frontier excludes parts; integrate's `fails` lookup.
