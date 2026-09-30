# CLEAN-4 blueprint: make `LoopStop::NoProgressReview` reachable

Refines `.claude/plans/clean-4.plan.md` (approved, D1 to D5) into concrete code. The decisions are
not reopened here. Line numbers are from `hr/CLEAN-4` @ `6bacbc3`, read through Gortex.

## Architecture: CLEAN-4

### Design decisions (refinements only)

- **R1: the D5 unit tests use `review_position = 3`, `target = 2`, not the existing test's `1`/`0`.**
  `RUN_3`'s snapshot is the FEAT graph that `demo_graph_snapshot` builds
  (`crates/htui-core/src/fixtures.rs:1322-1398`), whatever its item is (`HTUI_ANA_1`). Its
  positions are `0 prd`, `1 plan`, `2 implement`, `3 review`. `output_kind == name`, per
  `seed.rs:29` "also its `output_kind`". The existing call `no_progress(&ctx, &steps, 0, 1, 2)` at
  `gate.rs:1547/1557` therefore points the review half at kind **`plan`** at position 1. It answers
  false today and after the fix because `RUN_3` has no position-1 rows. The new tests use the real
  shape: review at position 3 (`output_kind "review"`) and target 2 (`loop_target(snapshot, 3) ==
  Some(2)`, pinned by `the_loop_target_is_the_named_implement_phase`).
- **R2: generalise the test helper.** `step(...)` (`gate.rs:1379-1409`) hard-codes `position: 0,
  phase_name: "research"`. Add `step_at(store, position, phase_name, attempt, fanout_index, path)`
  and have `step` delegate to it. Every existing caller stays byte-identical.
- **R3: documents are written through `WriteStore::write_document(NewDocument)`**
  (`traits.rs:1309`, struct at `model/document.rs:71-89`). The store allocates `version` as
  `max(version of (item, kind)) + 1` (`mem.rs:4711-4747`). "At a chosen version" therefore means
  "in a chosen write order". `produced_by_step_id` must name an existing `run_step`, or you get
  `Constraint`.
- **R4: the hash half is excluded by construction.** `RUN_3.repo_scope` is `Vec::new()`
  (`fixtures.rs:1462`), so `commits_are_identical` returns `Ok(false)` at `gate.rs:841` before
  reading anything. The unit tests assert this, so the verdict cannot come from the hash half.

### Files to create

None (T4 bookkeeping docs excepted; see the plan).

### Files to modify

| File | Changes | Task |
|---|---|---|
| `/home/mluigi/projects/htui/crates/htui-orch/src/engine.rs` | one `&&` clause in `a_review_rejection_loops_then_escalates` | T1 |
| `/home/mluigi/projects/htui/crates/htui-orch/tests/gix_isolator.rs` | two `script` calls + doc sentence in `two_committing_…_worktree_mode` | T1 |
| `/home/mluigi/projects/htui/crates/htui-orch/src/gate.rs` | tests: `step_at`, `produce`, 2 tests, 1 import line (T2); body + 2 doc comments (T3) | T2, T3 |
| `/home/mluigi/projects/htui/crates/htui-orch/src/conformance.rs` | new case fn, CASES entry, dispatch arm, 5 count sites, CASES doc paragraph | T2 |
| `/home/mluigi/projects/htui/crates/htui-orch/tests/fake_conformance.rs` | `73` to `74` at **:16** | T2 |

---

## T1: pin the stop reason (tests only, green on the baseline)

### T1.a `crates/htui-orch/src/engine.rs` `a_review_rejection_loops_then_escalates` (fn at :7562)

The assertion spans `:7708-7715`. Add one clause after `:7712` (`&& note.body.contains("attempt 2")`):

```rust
        assert!(
            notes.iter().any(|note| {
                note.body.contains("review loop exhausted after 2 attempts")
                    && note.body.contains("implement")
                    && note.body.contains("attempt 2")
                    && note.body.contains("stop reason `exhausted`")
            }),
            "criterion 6's exact wording: {notes:?}"
        );
```

This is green on the baseline and after the fix. The scripts at `:7565-7570` are distinct
(`"first"`/`"second"`). `repo_scope: None` with no repo gives an empty scope, so both halves are
false and the reason is `Exhausted` (default `retry_limit = 1`).

### T1.b `crates/htui-orch/tests/gix_isolator.rs` `two_committing_implement_attempts_never_look_identical_in_worktree_mode`

The doc is at `:1755-1757`, `#[tokio::test]` at `:1758`, and the fn at `:1759`. The API is
confirmed: `fix.orch` is a `FakeOrchestrator`, and `fix.orch.script(phase, attempt, ScriptedStep)`
is already used in this file at `:1817-1823` (`a_rejected_reviews_commits_never_reach_the_primary`).
`ScriptedStep` is already imported.

Doc: extend `:1757` to read:

```rust
/// Plan D66's other half, which is what closes R-8: two `implement` attempts that **do** commit
/// can never agree on `after_hash` in `worktree` mode, because attempt 2 branches from a primary
/// that already holds attempt 1's merge. The loop goes on to a third attempt.
///
/// The two reviews say different things (CLEAN-4 plan D3). The fake's default output is one fixed
/// string, so two default reviews would stop the loop on `no_progress_review` at attempt 2, and
/// the third attempt would stop proving anything about the hash half.
```

Insert after `:1764` (`fix.three_implement_attempts().await;`), before the `sink`:

```rust
    fix.orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "first"),
    );
    fix.orch.script(
        "review",
        2,
        ScriptedStep::review("request-changes", "second"),
    );
```

The gate is unchanged by `three_implement_attempts` (`:1649-1658` only sets isolation and
`retry_limit`), so review keeps the seeded `always` gate. `(Gate::Always, outcome)` parks on
**every** outcome (`gate.rs:393-396`), so `request-changes` still parks for the human
`rejected(...)` at `:1778`/`:1781`. It also adds one settle note (`verdict: request-changes`) per
review. `"approve"`, as at `conformance.rs:1526-1527`, would work equally well and write no
settle note. Either choice is correct. The plan says `request-changes`, so keep it unless
reviewers object. The assertion at `:1800-1804` is unchanged.

**Validate:** `cargo test -p htui-orch --all-features` green. Commit
`test(clean-4): pin the stop reason each shipped review-loop case reaches`. The body lists the
already-pinned cases, as the plan says.

---

## T2: red tests

### T2.a `crates/htui-orch/src/gate.rs` unit tests (module `tests`, opens at :1019)

**Import**: extend `:1029`:

```rust
    use htui_core::model::{
        DocumentId, NewDocument, NewRepo, NewRunStep, RepoId, Run, RunStep, RunStepCommit,
        StepStatus,
    };
```

(`Document` is already imported at `:1026`; `epoch` at `:1022`; `ReadStore as _, WriteStore as _` at `:1030`.)

**Helper**: replace `step` (`:1377-1409`) with `step_at` plus a delegating `step`:

```rust
    /// Inserts a `phase_name` row of `RUN_3` at `(position, attempt, fanout_index)` and walks it
    /// through `path` with legal compare-and-sets, starting from `pending`.
    async fn step_at(
        store: &MemStore,
        position: i32,
        phase_name: &str,
        attempt: i32,
        fanout_index: i32,
        path: &[StepStatus],
    ) -> RunStep {
        let row = store
            .create_step(NewRunStep {
                id: htui_core::model::StepId::new(),
                run_id: ids::RUN_3,
                position,
                attempt,
                fanout_index,
                phase_name: phase_name.to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: Some("opus".to_owned()),
            })
            .await
            .expect("the slot has room for this row");
        let mut from = StepStatus::Pending;
        for &to in path {
            assert!(
                store
                    .transition_step(row.id, from, to, epoch())
                    .await
                    .expect("a legal move"),
                "`{from} -> {to}` applied"
            );
            from = to;
        }
        row
    }

    /// A `research` row of `RUN_3` at `(0, attempt, fanout_index)`: [`step_at`] at position 0.
    async fn step(
        store: &MemStore,
        attempt: i32,
        fanout_index: i32,
        path: &[StepStatus],
    ) -> RunStep {
        step_at(store, 0, "research", attempt, fanout_index, path).await
    }

    /// Writes `body` as a `kind` document of `RUN_3`'s item (`HTUI_ANA_1`), produced by `by` or
    /// by hand. The store allocates the version, so write order is version order (plan D6).
    async fn produce(store: &MemStore, kind: &str, by: Option<&RunStep>, body: &str) {
        store
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: ids::HTUI_ANA_1,
                kind: kind.to_owned(),
                title: kind.to_owned(),
                body: body.to_owned(),
                produced_by_step_id: by.map(|step| step.id),
                created_by: ids::USER,
                created_at: epoch(),
            })
            .await
            .expect("the item, the step and the user exist");
    }
```

**Tests**: place them after `no_progress_compares_the_two_winners_not_index_zero` (which ends at
`:1561`). Each builds `ctx` exactly as the retire tests do (`:1431-1438`).

```rust
    /// CLEAN-4 (plan D5): the review half reads every version, not the latest per kind. Two
    /// reviews of this loop that differ only in `\r\n` against `\n` are one review after
    /// `canonical`, so the loop stops on `no_progress_review`. `RUN_3` holds no repo, so the hash
    /// half cannot decide, and the review half is the one that answers.
    #[tokio::test]
    async fn no_progress_review_fires_on_reviews_equal_after_canonicalisation() {
        let store = MemStore::demo();
        let (run, snapshot) = run_3();
        assert!(
            run.repo_scope.is_empty(),
            "the hash half answers false on an empty scope, so it cannot be what fires"
        );
        let review = snapshot
            .phases
            .iter()
            .find(|phase| phase.position == 3)
            .expect("the FEAT snapshot has a position 3");
        assert_eq!(
            (review.name.as_str(), review.output_kind.as_str()),
            ("review", "review")
        );
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let first = step_at(
            &store,
            3,
            "review",
            1,
            0,
            &[StepStatus::Running, StepStatus::Failed, StepStatus::Cancelled],
        )
        .await;
        let second = step_at(
            &store,
            3,
            "review",
            2,
            0,
            &[StepStatus::Running, StepStatus::Failed],
        )
        .await;
        produce(
            &store,
            "review",
            Some(&first),
            "---\r\nverdict: request-changes\r\n---\r\nno tests\r\n",
        )
        .await;
        produce(
            &store,
            "review",
            Some(&second),
            "---\nverdict: request-changes\n---\nno tests\n",
        )
        .await;
        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");

        assert_eq!(
            no_progress(&ctx, &steps, 2, 3, 2).await.expect("reads"),
            Some(LoopStop::NoProgressReview),
            "the two reviews differ only in line endings, which `canonical` erases"
        );
    }

    /// CLEAN-4 (plan D5), the two ways the review half must still answer no: two reviews of this
    /// loop that say different things, and `review`-kind documents from outside `review_position`
    /// (the implement at position 2, and one written by hand) whose body would make the top two
    /// identical if they were counted. The last leg is the control: a third review of this loop
    /// that repeats the second *is* read, so the `None`s above come from the filter and not from
    /// an empty read.
    #[tokio::test]
    async fn no_progress_review_counts_only_this_loops_reviews() {
        const SECOND: &str = "---\nverdict: request-changes\n---\nstill no tests\n";
        let store = MemStore::demo();
        let (run, snapshot) = run_3();
        let ctx = GateContext {
            store: &store,
            clock: &SystemClock,
            run: &run,
            snapshot: &snapshot,
            user: ids::USER,
            box_id: ids::BOX,
        };
        let first = step_at(
            &store,
            3,
            "review",
            1,
            0,
            &[StepStatus::Running, StepStatus::Failed, StepStatus::Cancelled],
        )
        .await;
        let second = step_at(
            &store,
            3,
            "review",
            2,
            0,
            &[StepStatus::Running, StepStatus::Failed],
        )
        .await;
        produce(
            &store,
            "review",
            Some(&first),
            "---\nverdict: request-changes\n---\nno tests\n",
        )
        .await;
        produce(&store, "review", Some(&second), SECOND).await;
        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");
        assert_eq!(
            no_progress(&ctx, &steps, 2, 3, 2).await.expect("reads"),
            None,
            "the second review says something the first did not"
        );

        // Two higher versions of kind `review` repeating the second: one by the implement at
        // position 2, one by hand. Neither is a review of this loop.
        let implement = step_at(
            &store,
            2,
            "implement",
            2,
            0,
            &[StepStatus::Running, StepStatus::Done],
        )
        .await;
        produce(&store, "review", Some(&implement), SECOND).await;
        produce(&store, "review", None, SECOND).await;
        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");
        assert_eq!(
            no_progress(&ctx, &steps, 2, 3, 2).await.expect("reads"),
            None,
            "a `review`-kind document from outside `review_position` is not a review of this loop"
        );

        // The control: a third review of this loop that repeats the second.
        let third = step_at(
            &store,
            3,
            "review",
            3,
            0,
            &[StepStatus::Running, StepStatus::Failed],
        )
        .await;
        produce(&store, "review", Some(&third), SECOND).await;
        let steps = store.run_steps(ids::RUN_3).await.expect("RUN_3 exists");
        assert_eq!(
            no_progress(&ctx, &steps, 2, 3, 3).await.expect("reads"),
            Some(LoopStop::NoProgressReview),
            "the two latest reviews of this loop repeat each other, whatever lies between them"
        );
    }
```

How each test behaves on the baseline (`documents_of_kinds` = the single latest `review` of `HTUI_ANA_1`):
- test 1: the latest is `second`, so there is one head, the result is `false`, and the test gets
  `None`. **Red** at its only assertion.
- test 2: legs 1 and 2 are `None` on both readers. Leg 3: the latest is `third`, so there is one
  head, and the test gets `None`. **Red** at the control. After the fix, the heads for leg 3 are
  `{first v?, second, third}`, the top two are `third`/`second`, and they are identical, so `Some`.

`Running -> Failed -> Cancelled` is a legal chain. `retire_cancels_a_failed_candidate_and_a_failed_judge`
(`:1463-1496`) exercises `failed -> cancelled` through the same `transition_step`.

### T2.b `crates/htui-orch/src/conformance.rs` new case `identical_reviews_stop_the_loop`

**Confirmed facts**
- `free_feat_3` (`:827-832`) only cancels `RUN_2`. It creates no repo.
- The demo `MemStore` seeds no repos (`primary_repo` doc, `:1008-1009`). `start` passes
  `repo_scope: None` (`:1035`). Per `Command::StartRun` doc (`command.rs:62-63`), "`None` resolves
  to the project's primary repo, **or to nothing when it has none**". So `run.repo_scope` is empty.
- `commits_are_identical` returns `Ok(false)` on `ctx.run.repo_scope.is_empty()` (`gate.rs:841-843`),
  before any read.
- The fake writes each review with `kind = phase.output_kind`, `produced_by_step_id = Some(step.id)`
  (`fake.rs:1691-1715`, `write_output`), so the fixed reader sees both reviews.
- `no_progress` runs **before** the budget check in `review_loop` (`gate.rs:748-753`), so the reason
  is `no_progress_review` even though the budget would also have allowed escalation.
- Only the fake binds `htui_orch::conformance` (`tests/fake_conformance.rs` and the in-crate `Demo`).
  No Postgres binding runs these CASES (`pg_conformance.rs` runs `htui_core`'s suite).

**Function**: insert after `identical_after_hash_stops_the_loop`, which ends at `:1718`:

```rust
/// CLEAN-4 (plan D4): two identical review documents stop the loop *before* its retry budget says
/// so, on `no_progress_review`, criterion 7's other half.
///
/// The twin of `identical_after_hash_stops_the_loop` with the halves swapped. **No repo is in
/// scope**, so the hash half answers false on the empty `repo_scope` and cannot be what stops
/// the loop. Both review attempts are scripted with one body, so the review half does.
/// `retry_limit = 3` on `implement` again means the budget would have permitted a third attempt.
async fn identical_reviews_stop_the_loop<H: CaseHarness>(harness: &H) {
    let orch = harness.fresh();
    free_feat_3(&orch).await;
    repoint(&orch, ids::HTUI_FEAT_3, |phase| {
        if phase.name == "implement" {
            phase.retry_limit = 3;
        }
    })
    .await;
    orch.script(
        "review",
        1,
        ScriptedStep::review("request-changes", "no tests"),
    );
    orch.script(
        "review",
        2,
        ScriptedStep::review("request-changes", "no tests"),
    );

    let (run, _) = start(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        run_of(&orch, run).await.repo_scope.is_empty(),
        "plan D14: no primary repo, so `repo_scope: None` resolves to nothing and the hash half \
         cannot decide"
    );

    approve(&orch, run, 3).await; // prd, plan, implement 1
    answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "attempt 1 is no better".to_owned(),
        },
    )
    .await; // review 1 -> the loop resumes at implement 2
    approve(&orch, run, 1).await; // implement 2
    let (_, rest) = answer(
        &orch,
        run,
        GateAnswer::Rejected {
            note: "attempt 2 is no better".to_owned(),
        },
    )
    .await; // review 2 -> the same review, so the loop stops

    assert_eq!(rest.failure, Some(RunFailure::ReviewLoopExhausted(2)));
    assert_eq!(run_of(&orch, run).await.status, RunStatus::AwaitingApproval);
    assert_eq!(
        item_of(&orch, ids::HTUI_FEAT_3).await.status,
        Status::Blocked
    );
    let notes = notes_of(&orch, ids::HTUI_FEAT_3).await;
    assert!(
        notes.iter().any(|body| {
            body.contains("review loop exhausted after 2 attempts")
                && body.contains("stop reason `no_progress_review`")
        }),
        "the budget permitted a third attempt and no repo was in scope; the review half is what \
         stopped it: {notes:?}"
    );
    assert!(
        !steps_of(&orch, run)
            .await
            .iter()
            .any(|step| step.attempt == 3),
        "no third implement attempt was created, though `retry_limit = 3` would have allowed one"
    );
}
```

The `request-changes` verdict under the seeded `always` review gate parks, as in T1.b, and the
human rejection drives the loop exactly as in the twin. `"approve"` would also work.

On the baseline, the review half is dead, so `no_progress` is `None` and `may_attempt(3, 3)`
holds. The loop resumes and parks implement 3, so `rest.failure` is `None`. **Red at the first
`assert_eq!`**, for the right reason. (The plan's "get `None` or `Exhausted`": it is `None`.)

**CASES entry**: insert after `:377` (`"identical_after_hash_stops_the_loop",`):

```rust
    // CLEAN-4 plan D4: criterion 7's other half, two identical review bodies with no repo in
    // scope, stop the loop on `no_progress_review` before its budget does.
    "identical_reviews_stop_the_loop",
```

**Dispatch arm**: insert after `:563-565` (the `identical_after_hash_stops_the_loop` arm):

```rust
        "identical_reviews_stop_the_loop" => Box::pin(identical_reviews_stop_the_loop(harness)),
```

(Let rustfmt pick the one-line or braced form. The name is short enough for one line at 100 columns.)

**Every count site**. The plan lists four; there are **seven edits across five locations**:

| Site | Now | Becomes |
|---|---|---|
| `conformance.rs:310` | `/// Seventy-three, and the count is pinned in two places on purpose — here by` | `/// Seventy-four, and the count …` (rest unchanged) |
| `conformance.rs:314` (**not in the plan**) | `/// Recounted, not appended: 18 + 5 + 13 + 6 + 10 + 18 + 2 + 1.` | `/// Recounted, not appended: 18 + 5 + 13 + 6 + 10 + 18 + 2 + 1 + 1.` |
| `conformance.rs:363` (after it, new paragraph) | (end of the MOD-40 paragraph) | add `///` + `/// **One for CLEAN-4** (plan D4): criterion 7's review half, which a reader that saw only the` / `/// latest version per kind had left unreachable: two identical reviews stop the loop on` / `/// `no_progress_review` with no repo in scope.` |
| `conformance.rs:548` | `/// seventy-three-arm `match` in an `async fn` puts all seventy-three in the one frame every case is` | `/// seventy-four-arm `match` in an `async fn` puts all seventy-four in the one frame every case is` |
| `conformance.rs:5891` | `73,` | `74,` |
| `conformance.rs:5892` (same `assert_eq!` message) | `"18 + 5 + 13 + 6 + 10 + 18 + 2 + 1: eighteen before …` | `"18 + 5 + 13 + 6 + 10 + 18 + 2 + 1 + 1: eighteen before …` |
| `conformance.rs:5912-5913` (same message, tail) | `… and MOD-40 milestone 1's one (a \` / `suspended walk fenced after adoption)"` | `… and MOD-40 milestone 1's one (a \` / `suspended walk fenced after adoption), and CLEAN-4's one (identical reviews stop \` / `the loop on `no_progress_review`)"` |
| `tests/fake_conformance.rs:16` (plan said `:17`) | `assert_eq!(CASES.len(), 73);` | `assert_eq!(CASES.len(), 74);` |

**Validate (red):** `cargo test -p htui-orch --all-features gate::tests::no_progress_review` gives 2
failed. `cargo test -p htui-orch --all-features --test fake_conformance` fails in
`fake_orchestrator_passes_every_case` at `identical_reviews_stop_the_loop`. `cases_len_is_pinned`
and `cases_are_unique_and_counted` pass (the count moved with the list). Commit
`test(clean-4): the review half of the no-progress predicate has cases that reach it` (red).

---

## T3: fix `reviews_are_identical`

### `no_progress` doc, `gate.rs:792-799` (fn at :800, body unchanged)

```rust
/// Plan D11's predicate, evaluated only when there is a previous attempt to compare against.
///
/// Both halves are computed from rows the step already wrote. The hash half compares the two
/// implement attempts' `run_step_commit` rows per repo, with "both `None`" counting as identical —
/// two attempts that committed nothing are exactly the failure mode ANA-2 `:735-739` names. The
/// review half compares the two highest versions of the review phase's `output_kind` that this
/// run's steps at `review_position` produced, read from every version the item holds rather
/// than the latest per kind, and hashes them through `prompt::digest::canonical`, the workspace's
/// one normalisation, so a review re-emitted with different line endings does not read as
/// progress. The hash half is asked first, so a loop where both hold reports `no_progress_hash`.
```

### `reviews_are_identical`: doc at `:864`, fn `:865-903`, full replacement

```rust
/// The two latest review documents this run's steps at `review_position` produced, byte-identical
/// after canonicalisation.
///
/// Reads [`ReadStore::documents`](htui_core::store::ReadStore::documents): heads of every version
/// of every kind, without bodies. `documents_of_kinds` answers only the latest version per kind,
/// so it can never hold two reviews, and reading through it left `LoopStop::NoProgressReview`
/// unreachable from MOD-4 milestone 2 until CLEAN-4. The heads are kept when their `kind` is the
/// review phase's `output_kind` and their `produced_by_step_id` is one of this run's rows at
/// `review_position`. The two highest versions win, and only those two bodies are read, through
/// [`ReadStore::document`](htui_core::store::ReadStore::document).
///
/// Every "cannot tell" answers `false`, so the loop is not stopped: a chat run, a snapshot with
/// no such position, fewer than two reviews, or a head whose body read answers `None` because the
/// row went between the two reads.
async fn reviews_are_identical<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>,
    steps: &[RunStep],
    review_position: i32,
) -> Result<bool, EngineError> {
    let Some(item) = ctx.item() else {
        return Ok(false);
    };
    let Some(kind) = ctx
        .snapshot
        .phases
        .iter()
        .find(|phase| phase.position == review_position)
        .map(|phase| phase.output_kind.as_str())
    else {
        return Ok(false);
    };
    let mine: Vec<StepId> = steps
        .iter()
        .filter(|step| step.position == review_position)
        .map(|step| step.id)
        .collect();
    let mut heads: Vec<_> = ctx
        .store
        .documents(item)
        .await?
        .into_iter()
        .filter(|head| {
            head.kind == kind
                && head
                    .produced_by_step_id
                    .is_some_and(|step| mine.contains(&step))
        })
        .collect();
    heads.sort_by_key(|head| core::cmp::Reverse(head.version));
    let [newest, previous, ..] = heads.as_slice() else {
        return Ok(false);
    };
    let (Some(newest), Some(previous)) = (
        ctx.store.document(newest.id).await?,
        ctx.store.document(previous.id).await?,
    ) else {
        return Ok(false);
    };
    Ok(sha256_hex(&canonical(&newest.body)) == sha256_hex(&canonical(&previous.body)))
}
```

Notes for the implementer:
- `documents`/`document` are `ReadStore` methods, reachable on `S: WriteStore` through the
  supertrait bound, just as `documents_of_kinds` is today. No import is needed.
- `StepId` is already imported (`gate.rs:15-16`). `Document` stays imported because `SettleInput`
  uses it at `:217`. `DocumentHead` needs no import because of `Vec<_>`.
- `kind` is a `&str` borrowed from `ctx.snapshot` (`&'a GraphSnapshot`), and it is live across the
  awaits. That is fine: it is a shared borrow of data the context outlives. `String == &str`
  compares through `PartialEq<&str> for String`.
- Do not rely on backend order. Pg returns `ORDER BY kind, version` (`pg/read.rs:243-264`), Mem
  returns "ascending by version" per kind. The explicit descending sort is the contract.
- Versions are unique per `(item, kind)`, so the sort has no ties after the kind filter.

**Validate:** `cargo test -p htui-orch --all-features` green, T1 and T2 included. Commit
`fix(clean-4): the review half of the no-progress predicate reads every version`.

---

## Data flow (after the fix)

A human or a `never` gate rejects a review, and `review_loop` (`gate.rs:701`) runs. It reads
`run_steps`, takes `latest_at(target)` as `attempt`, and calls `no_progress(ctx, steps, target,
review.position, attempt)`. With `attempt ≥ 2`:
1. `commits_are_identical`: an empty scope gives `false`; otherwise it compares the winners'
   `after_hash`.
2. `reviews_are_identical`: `documents(item)` heads are filtered by `kind == output_kind(review_position)`
   and producer ∈ this run's `review_position` rows, sorted by version descending, cut to the top
   two, fetched with `document(id)` ×2, run through `canonical`, then `sha256` and compared.

`Some(reason)` leads to `escalate` (the note `…, stop reason \`no_progress_review\``). `None` leads to
the budget check, then either `retire` or `Exhausted`.

## Build sequence

1. T1.a engine assertion, then T1.b gix scripts and doc. Run `cargo test -p htui-orch --all-features`
   (green) and commit.
2. T2.a gate test helpers and two tests. Then T2.b the conformance case, CASES, dispatch, and the
   seven count edits, plus `fake_conformance.rs:16`. Confirm exactly the expected failures (red)
   and commit.
3. T3 body and the two doc comments. Run `cargo test -p htui-orch --all-features` (green) and commit.
4. T4 gates and bookkeeping, as in the plan.

## Hazards

1. **Wrong review position in the unit tests.** If a D5 test copies the existing call's
   `(0, 1, 2)`, the review half reads kind `plan`, and `review`-kind documents are never seen. The
   positive test would then fail after the fix and look like a broken fix. Use `(2, 3, …)` (R1).
2. **The hash half firing instead.** In the unit tests `RUN_3.repo_scope` is empty (asserted). In
   the conformance case, do **not** call `primary_repo` or script `after` hashes. The
   `repo_scope.is_empty()` assertion guards this. With a repo and default hashes, the case would
   report `no_progress_hash` and pass the `ReviewLoopExhausted(2)` assertion for the wrong reason.
   The note-text assertion is what catches that.
3. **Negatives that pass on an empty read.** A reader that always answers `false` passes the
   "different content" and "outsider" legs. The control leg in test 2 (a third identical review
   gives `Some`) is what makes those legs meaningful. Keep it in the same test, after the negatives.
4. **Outsider documents must share the review kind.** They must be kind `review`, write after the
   loop's reviews (higher versions), and repeat the latest body. Otherwise the leg proves nothing
   about the `produced_by_step_id` filter.
5. **Half ordering.** `identical_after_hash_stops_the_loop` (conformance `:1660`), engine
   `two_identical_after_hashes_stop_the_loop_early` (`:7829`) and gix `:1742` all have identical
   default reviews **and** identical hashes. They stay `no_progress_hash` only because
   `no_progress` asks the hash half first (`gate.rs:810-815`). Do not reorder the halves.
6. **A silent reason change outside htui-orch.** `crates/htui/tests/runs_pg.rs:1052`
   `unblock_follows_an_escalated_run_on_postgres` rejects two reviews and asserts the escalation
   and the attempt sequence, but no stop reason. If its stack's two reviews have equal bodies, its
   reason moves from `exhausted` to `no_progress_review` with the same observable outcome, so it
   stays green. The plan's probe did not run it (htui-orch + `pg_criteria` only). The T4 workspace
   gate runs it. Mention it in the write-up if it turns out to use equal bodies.
7. **Other loop cases.** `an_intermediate_position_is_retired_by_the_loop` and the fan-out loop case
   reject once (`attempt < 2`, predicate skipped). `review_rejection_loops_then_escalates`,
   `a_never_gate_rejection_escalates_when_the_budget_is_out` and
   `unblock_lets_an_escalated_run_be_promoted_and_approved` script distinct bodies. The probe
   confirmed that only the gix case flips.
8. **Stack depth.** `case()` boxes each arm (`conformance.rs:543-551`). Add the new arm in the same
   `Box::pin(...)` shape, never as an inline `.await` in `run_case`.
9. **Integration tests need `--all-features`.** Without it, `tests/*.rs` run 0 tests and report ok
   (memory). Run every validation with `--all-features`.

## Deviations from the plan

- **Count sites.** `CASES` is pinned at **five locations with seven edits**, not four. The plan
  omits the sum at `conformance.rs:314` ("Recounted, not appended: … + 1") and the sum and tail of
  the `assert_eq!` message at `:5892` / `:5912-5913`, which sit next to the `73,` at `:5891`.
- **`tests/fake_conformance.rs`.** The literal is at **:16**, not `:17`.
- **The review in the existing fixture.** At the plan's cited call (`review_position = 1`), the
  "review" phase is actually `plan` (kind `plan`). The D5 tests use position 3 (`review`/`review`)
  and target 2 instead. This is the plan's intent: "mirroring the fixture" means the same
  `MemStore::demo` / `run_3` / helper, not the same position arguments.
- **The step helper.** `step(...)` cannot create position-3 rows. It gains a `step_at`
  generalisation (test-only).
- **The red conformance failure.** On the baseline the new case gets `rest.failure == None` (the
  loop resumed to implement 3), not `Exhausted`.
- **The verdict word (optional, not a deviation).** Under the seeded `always` review gate,
  `request-changes` and `approve` both park for the human rejection. `request-changes` writes one
  extra settle note per review, which no assertion reads.
