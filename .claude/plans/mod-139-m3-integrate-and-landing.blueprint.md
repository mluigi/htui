# Blueprint: MOD-139 M3 — Integrate and landing (T1–T5)

Produced by `code-architect` 2026-11-11 from the confirmed, fact-checked plan
`.claude/plans/mod-139-m3-integrate-and-landing.plan.md` (D1–D12; Confirm 1 item note, 2(a) the
path-overlap post-check, 3 refuse approval), PRD `.claude/prds/mod-139-divided-fan-out.prd.md`
(M3 row) and ANA-33 §7.4, §7.5, §7.7, §9. Tree read at `c4fbf8a7` (branch `hr/MOD-139`); every
`file:line` below was read at that commit (`git show HEAD:<path>`). Format: M2's
`.claude/plans/mod-139-m2-planner-and-parts.blueprint.md`.

The plan stays the spec. §0 lists where the tree disagrees with it or leaves a choice open; every
row is **adopted** (the implementation detail or the only form that works) unless it sits under
"Flagged". None reopens a maintainer decision (item note, path-overlap check, approval refused).
M2's Main-thread rulings F-1..F-8 and its "Deferred from review" hold unchanged.

## 0. Plan deltas

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | Confirm 3 reads "the gate note", but a park writes **no** `run_step.gate_note`: `park_step` writes none and `note_step` writes the reason as an **item note** `` step `<phase>` attempt <n>: <note> `` with `via_step_id`. | `gate.rs:506-538` (doc: "park leaves `gate_note` NULL until human answers"), `gate.rs:498-504` (`gate_note` = `failure.to_string()`) | **adopted**: the guard reads the item's notes, `stopped_to_ask`'s shape (`gate.rs:787-792`): new `gate::parked_unmerged(notes, step)` (§1.11). |
| P-2 | Confirm 3 names `Approved` only, but `Skipped` lands too: `answer_guarded` maps it to `Done` and runs `reconcile_done_step`. | `engine.rs:1463-1465`, `:1523-1530` | **adopted**: `Approved` **and** `Skipped` are refused on an `integrate_incomplete` park; `Rejected` and `RetryStep` stay open (§1.11). |
| P-3 | `PromoteStep` then `AcceptArtifact` lands a promoted integrate without the post-check: `promote_enabled` admits an `awaiting_approval` step and accept lands it. | `command.rs:1184-1222`, `engine.rs:2072`, `:2324` | **adopted**: the same refusal guards `PromoteStep` on that park (§1.11). Accept after a promote taken *before* the failure is flagged (F-1). |
| P-4 | D6's `unmerged(trees, heads, parts)` cannot name its step: a repo-less run has no tree or commit row, and D11 needs the fake to log the integrate position. | `fake.rs:580-585` (`log` keyed by step); D11 log filter | **adopted**: `unmerged(step, trees, heads, parts)` (§1.3). |
| P-5 | Confirm 2(a) "integrate's own base..after" fails every `local` run: a `local` integrate prepares with no slot, so its `before_hash` **is** the part's tip and its own range omits the part's paths. | `real.rs:439-451` (`start_of` = `HEAD` without a slot), `real.rs:663-671`; plan D2 local row | **adopted**: per part, the integrate-side range is `<part's before_hash>..<descendant>`, the same commit as `base_refs[repo]` in the three slotted modes (§1.10). |
| P-6 | Confirm 2(a) leaves two path-check edge cases open: a part whose own range changes no path (an empty commit), and a name list cut at `DIFF_CAP`. | `git.rs:69` (`DIFF_CAP`), `git.rs:939` (`NameOnly`), `real.rs:1560-1563` (`truncated`) | **adopted**: both are exempt from the **path** half only; ancestry still applies. Failure reports the same `IntegrateIncomplete` with the part's key (§1.10). |
| P-7 | D6's sentence `part(s)` is not bytes. | plan D6 | **adopted**: singular and plural forms, §1.6. |
| P-8 | `merge_no_ff` is a `Cli` method and its seam `merge_no_ff_reading` (test `git.rs:3657-3690`) takes `step`. | `git.rs:726-740`, `:746-812`, `:1601-1603` | **adopted**: `Cli::merge_no_ff_as(&self, path, message, before, after)`; `merge_no_ff_reading` takes `message: &str` and answers `MergeAttempt`; `merge_no_ff` keeps its signature and maps `Conflicted` to `Refused(merge_conflict(..))` (§1.4). |
| P-9 | D5 "label or hash": a label is `htui/<step id>`, and a `PromptSpec` carries no step id by construction (ANA-5 §4.7 rule 8). | `prompt/mod.rs:73-81` | **adopted**: section and note render the **tip hash** only; `PartPremerge.step` exists for the engine and is never rendered (§1.5). |
| P-10 | Most engine cases are repo-less (`base` of an empty scope is `{}`), so a report with no repo would add a note and a section to every divided case. | `isolate.rs:184-190`; `notes_of(..).last()` asserts, e.g. `conformance.rs:14168-14171` | **adopted**: a report with no repo renders no `premerge` section and writes no note (I-7: no empty section). |
| P-11 | D10's `DivideRefused` needs `SnapshotSettings` and the expanded phases (`admission_refusal(snapshot, ..)`), which exist only once the `GraphSnapshot` literal is built. | `divide.rs:177-181`; `graph.rs:695-749` (settings, literal), `:715` (`check_fan_out_caps(..)?`, an `Err` after phases) | **adopted**: `NoParts`/`PlanScope` live in `not_yet_runnable` (carried as `Resolution::Refused`, as `Divide` was); `DivideRefused` is raised as `Err` after the literal, before `Ok(Resolution::Resolved(..))` (`graph.rs:751`). Consequence: every `NotYetRunnable` outranks every `DivideRefused`. `resume_window`'s not-comparable arm gains it, as D10 says. |
| P-12 | After T4 no engine path reaches the admission backstop: the snapshot is frozen by the same resolve that now refuses the same rules. | `divide.rs:169-250` reused verbatim at resolve | **adopted**: the five M2 admission cases fold into the renamed resolve case (§T4.4); `refuse_admission` keeps `divide.rs`' unit tests only. Flagged F-2. |
| P-13 | `answer_gate_enabled` is shared with `htui-worker` (`views.rs:395-399`); a new parameter moves a crate outside M3. | `command.rs:744-766`; `views.rs:16`, `:395` | **adopted**: signature unchanged; a new pure guard `command::unmerged_park` called by the engine. The TUI may still enable *approve* on such a park (dispatch refuses it): MOD-142 note. |
| P-14 | `call_log_of` matches `IsolatorCall` exhaustively, so T1's two new variants break `conformance.rs` unless T1 adds the arms. | `conformance.rs:11327-11334` | **adopted**: T1 owns those two lines of `conformance.rs` (§5). |
| P-15 | A case cannot script a whole `PremergeReport` or `unmerged` answer: part step ids are minted at admission, after the case starts the run. | `divide.rs` part rows minted in `admit_parts` | **adopted**: the fake is scripted **by part key** (`conflict_premerge(key, paths)`, `script_unmerged(keys)`), §1.3. |
| P-16 | `FakeVerifier::run` pops its FIFO on **every** live session, verify command or not. | `fake.rs:1098-1110` | **adopted**: T3 case 5 scripts `prd` and `plan` before integrate (parts never verify, M2 D4). |
| P-17 | D5's part document read is `output_of`: a part writes the divided node's `output_kind`, which the integrate node carries. | `engine.rs:9345-9371` | **adopted**: `output_of(item, <integrate node>, part.id)`; no new read. |
| P-18 | Two M2 cases used the integrate stop as their crash point (`a_crash_after_admission_…`, `a_local_divided_node_lands_no_part_on_recovery`); once integrate runs there is no stop to crash at. | `conformance.rs:14864-14895`, `:15079-15124` | **adopted**: both crash integrate with `stall_after_done("implement", 1, ..)` (§T3.6). |
| P-19 | D3's "errors fail the step and the run" has no conformance case in the plan's eleven. | plan T3 list | **adopted**: T3 case 12 pins it. CASES arithmetic in §T3.7. |
| P-20 | The `parts` ladder needs a strategy per rung but a record row holds one. | `trim.rs:759-764` (`mark` overwrites), `:869` (memory marks `Dropped`) | **adopted**: rung 1 marks `HeadTail` (elided lines and bytes summed), rungs 2 and 3 `Dropped` (`dropped` counts bodies); the last rung's strategy wins, as memory's. |
| P-21 | Plan D1 "pre-merge before commits are recorded": `before` rows carry prepare's hash, so recording them after the pre-merge would change nothing; the order is kept as written. | `engine.rs:5045-5058` | **adopted** as written (§1.9). |

**Flagged for the coordinator (adopted defaults, no M3 change beyond what is written):**
- **F-1.** An integrate promoted **before** it settles (`running`, or a `Gate::Always` park of an
  `ok` settle) and then accepted lands without the post-check: `accept_artifact` runs verify and
  lands. M3 refuses only the `integrate_incomplete` park (P-3). Recommended: MOD-142 adds the
  post-check to accept, or refuses `PromoteStep` on any integrate row.
- **F-2.** P-12: `refuse_admission`'s write path (`block_and_fail`) loses its end-to-end cases. It
  matters again when MOD-140 admits planner-written plans; MOD-140 owes the engine case.

---

## Main-thread rulings

**M3 coordinator rulings (2026-11-11, binding):**
- **R-1 (architect F-1): closed in M3.** `PromoteStep` is refused on every integrate row, not only on an
  `integrate_incomplete` park. A promoted integrate could otherwise be accepted before its post-check, so the
  divided node lands only through its own settle and post-check. The sentence follows §1.11's shape and names
  the post-check. T3 adds the check and a case (the case count moves by one more if it is a new case; if so,
  re-derive the arithmetic).
- **R-2 (architect F-2): deferred to MOD-140.** `refuse_admission` keeps only `divide.rs` unit coverage after T4.
  MOD-140, which admits planner-written plans, owns its engine case. It is recorded on the MOD-140 HANDOFF line
  at close-out.
- **R-4 (T1, adopted):** a copy-mode part that committed in a repo but has no copy tree row there
  is refused with `real::no_part_copy(part, name)` ("part <id> committed in <name> but has no copy of
  it"), not §1.3's `copy_tree_vanished(..)`, which names a path that does not exist in this case.
  Pinned by `premerge_in_a_copy_refuses_a_part_without_a_copy_row`.
- **R-3:** T3 checks first that integrate inherits the divided node's gate and `verify_command` through
  `expand_divide` (a unit assertion) and pins the reconcile-call ordinal in case 1 before case 8 relies on it.
- **R-5 (R1, review M1):** the integrate refusals name Retry or Reject: `IntegrateIncomplete` and
  `IntegrateUnmerged` end "retry or reject it instead" (`PromoteIntegrate` keeps "answer or retry it
  instead"). The retry budget is not exempted for integrate: an exhausted park refuses Retry
  (`RetryExhausted`) and Reject takes the divided node's `fails` edge or ends the run `rejected`.
  Pinned by `an_exhausted_integrate_park_is_rejected_not_retried`.
- **R-6 (R1, review M2):** the `(Integrate, Finished)` recovery arm fails an isolator or snapshot read
  error (`EngineError::Isolate`, `EngineError::Snapshot` from `integrate_stage` or `unmerged_keys`) hard,
  as the walk does: `fail_hard` with `failure_text`, then `Next::Finished(Rest { Failed,
  Some(position), None })`; store, lease and stale errors still propagate (A-4's transient class). Pinned
  by `an_integrate_read_error_on_recovery_fails_the_run` (FakeIsolator `fail_unmerged`). CASES 228 → 230.

**Deferred from review (M3):**
- **L3.** `unmerged` with no head rows passes (no row, no check). Unreachable today: integrate's capture
  writes one head row per repo of its scope. Harden if capture or scope changes.
- **L4.** `IntegratePrompt` is cloned into the spec (`premerge`, `parts`). Negligible cost; left.

M2's F-1..F-8, the F-8 addendum and its "Deferred from review (R1)" still hold; D9 closes
the deferred `shared_serialized` reset (integrate prepares from `base_refs`), and the at-least-once
substitution note stays (D12).

---

## 1. Shared decisions for every task

### 1.1 Names and placement

| Name | Where (beside) | Task |
|---|---|---|
| `PremergeReport`, `RepoPremerge`, `PartPremerge`, `PartMerge` | `htui-core/src/model/run.rs` after `RunStepCommit` (`:1361-1372`), before `CommandRunStatus` (`:1374`); re-exported in `model/mod.rs:193-202` | T1.0 |
| `PartTip` | `htui-orch/src/isolate.rs` after `ChangedPaths` (`:126-135`); re-exported in `lib.rs:78-81` | T1 |
| `Isolator::premerge`, `Isolator::unmerged` | `isolate.rs` after `reconcile` (`:224-242`) | T1 |
| `MergeAttempt`, `Cli::merge_no_ff_as`, `premerge_message` | `isolate/git.rs`: enum after `Merged` (`:487-492`); method before `merge_no_ff` (`:702`); fn beside `reconcile_message` (`:1600-1603`) | T1 |
| `GixIsolator` `premerge_repo`, `copy_part_ranges`, `premerge_part`, `unmerged_in`, `repo_holding` | `isolate/real.rs` after `reconcile_in_place` (`:1227ff`); `repo_holding` factored out of `range_of` (`:1338-1358`) | T1 |
| `IsolatorCall::Premerge`, `IsolatorCall::Unmerged`, fake scripting/stall fields and methods | `fake.rs:96-178`, `:180-195`, after `pause_nth_reconcile` (`:507-514`) | T1 |
| `IntegratePart`, `PREMERGE_SEPARATOR`, `PARTS_SEPARATOR`, `PromptSpec.premerge`, `PromptSpec.parts`, `premerge_note` | `htui-core/src/prompt/mod.rs`: fields after `part` (`:128-131`); struct after `PartBlock` (`:195-220`); constants after `PART_SEPARATOR` (`:222-224`) | T2 |
| `SectionName::{Premerge, Parts}` | `prompt/mod.rs:344-346` (after `Part`) | T2 |
| `Placeholder::{Premerge, Parts}` | `prompt/template.rs` after `Part` (`:127-131`) | T2 |
| `render::premerge`, `render::premerge_content`, `render::parts`, `PartState`, `PREMERGE_PREAMBLE`, `PREMERGE_PATH_CAP`, `PARTS_PREAMBLE` | `prompt/render.rs` after `part` (`:713-744`) | T2 |
| `TrimStep::Parts`, `Source::Parts`, `Trimmer::trim_parts` | `prompt/trim.rs:381-405`, `:440-459`, after `trim_memory` (`:842-882`) | T2 |
| `RunFailure::IntegrateIncomplete`, `status::integrate_incomplete` | `htui-orch/src/status.rs` after `PartExhausted` (`:197-205`) | T3 |
| `StepFailure::IntegrateIncomplete`, `Unmerged`, `SettleInput.unmerged`, `INTEGRATE_INCOMPLETE`, `parked_unmerged` | `gate.rs:110-131`, `:202-239`, beside `stopped_to_ask` (`:782-792`) | T3 |
| `StepKind::Integrate` | `recover.rs:189-201` (after `Part`) | T3 |
| `divide::IntegrateInputs`, `divide::integrate_inputs` | `divide.rs` after `latest_parts` (`:267-287`) | T3 |
| `EngineError::IntegrateIncomplete`, `command::unmerged_park` | `command.rs` after `RoleRow` (`:638-645`); fn after `answer_gate_enabled` (`:744-766`) | T3 |
| `IntegrateStage`, `IntegratePrompt`, `plan_parts` | `engine.rs` beside `PartStage` (`:9495`) and `part_retries` (`:9546`) | T3 |
| `Engine::{integrate_stage, premerge, integrate_prompt, unmerged_keys}` | `engine.rs` after `walk_live_step` (`:5017-5244`) | T3 |
| `NotRunnable::{NoParts, PlanScope}` (`Divide` deleted), `ResolveError::DivideRefused`, `graph::divide_refusal` | `graph.rs:311-328`, after `NotYetRunnable` (`:293-304`), beside `not_yet_runnable` (`:798-835`) | T4 |
| `engine::enqueue_fake`, `Orchestrate::enqueue` (`enqueue_divided`, `enqueue_divided_fake`, `resolve_divided` deleted) | `engine.rs:10108-10161`; `conformance.rs:257-263`, `:405-407`; `lib.rs:69` | T4 |

Every new **test-helper** name in `conformance.rs` has at most three underscores (M2 G-4's P-9 rule;
the self-scan lives in `htui-core/src/store/conformance.rs:29558`, which M3 does not touch, so the
rule is kept as a convention). Case names are free.

### 1.2 Model types (T1.0; `htui-core/src/model/run.rs`, after `:1372`)

```rust
/// MOD-139 M3 D3: what `htui_orch::Isolator::premerge` did to integrate's tree, one entry
/// per tree row (a repo of the run's scope), in tree-row order. One htui-core renderer turns it into
/// integrate's protected `premerge` section and its item note (D4, D5); it is never stored
/// structurally (Confirm 1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremergeReport {
    /// One per integrate tree row; empty for a repo-less run (blueprint P-10).
    pub repos: Vec<RepoPremerge>,
}

/// MOD-139 M3 D3: one repo's pre-merge, every done part in plan order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoPremerge {
    /// `run_step_tree.repo_id`.
    pub repo_id: RepoId,
    /// The checkout's name (`RepoCheckout.name`); the fake writes the repo id.
    pub name: String,
    /// The tree's `HEAD` once the pre-merge stopped.
    pub head: String,
    /// One per done part, plan (position) order.
    pub parts: Vec<PartPremerge>,
}

/// MOD-139 M3 D3: one part in one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartPremerge {
    /// The part's step: the engine's key back to the row. Never rendered (blueprint P-9).
    pub step: StepId,
    /// `SnapshotPart.key`.
    pub key: String,
    /// The part's `after_hash` in this repo; `None` when it committed nothing here.
    pub tip: Option<String>,
    /// What the pre-merge did with it.
    pub merge: PartMerge,
}

/// MOD-139 M3 D3: the five outcomes of one part's pre-merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PartMerge {
    /// `merge --no-ff` made `commit` (subject `htui: premerge <part step>`).
    Merged {
        /// The merge commit.
        commit: String,
    },
    /// The tip was already an ancestor of the tree's `HEAD` (the single `local` part, or a part
    /// reached through another part's merge).
    Contained,
    /// No `after_hash` in this repo.
    NoCommits,
    /// The repo's first conflict; the merge was aborted. Paths in byte order, uncapped (the
    /// renderer caps, D5).
    Conflicted {
        /// `git::conflicted_paths`' answer.
        paths: Vec<String>,
    },
    /// A later part of a repo that already conflicted.
    NotAttempted,
}
```

`model/mod.rs:193-202`'s `pub use run::{…}` gains `PartMerge, PartPremerge, PremergeReport,
RepoPremerge` (rustfmt orders). Unit test in `model/mod.rs`' tests, beside
`plan_status_moves_by_its_law`: `premerge_report_round_trips_through_serde` (every `PartMerge`
variant; the `outcome` tag spelled `merged`, `contained`, `no_commits`, `conflicted`,
`not_attempted`).

### 1.3 Isolator verbs (T1; `htui-orch/src/isolate.rs`)

```rust
// after ChangedPaths (:126-135)
/// MOD-139 M3 D2, D3: one done part as integrate's pre-merge and post-check read it. The engine
/// builds one per latest done part of the admitted plan, in plan (position) order.
#[derive(Debug, Clone, PartialEq)]
pub struct PartTip {
    /// The part's step.
    pub step: StepId,
    /// Its key (`divide::part_key` of its `node_key`).
    pub key: String,
    /// Its `run_step_tree` rows: under `copy` the copy holding its commits (D2).
    pub trees: Vec<RunStepTree>,
    /// Its `run_step_commit` rows (one per repo; `after_hash` `None` = nothing committed).
    pub commits: Vec<RunStepCommit>,
}

// trait Isolator, after reconcile (:237-242)
/// MOD-139 M3 D2, D3: integrate's pre-merge. Per tree row (repo), in plan order: a part with no
/// `after_hash` here is `NoCommits`; a tip already an ancestor of the tree's `HEAD` is
/// `Contained`; otherwise it is merged `--no-ff` onto the `HEAD` re-read before it, subject
/// `htui: premerge <part step>`. The repo's first conflict is aborted and reported `Conflicted`,
/// and every later part of **that** repo is `NotAttempted`; repos are independent. Under `copy`
/// every part's range is first copied into the integrate copy and labelled `htui/<part step>`
/// there (D2). In-place trees take the repo's admin lock around each part (D70's lock, as
/// `reconcile` does). A conflict is not an error; every `Err` fails the step (D3).
fn premerge<'a>(
    &'a self,
    step: StepId,
    trees: &'a [RunStepTree],
    parts: &'a [PartTip],
) -> IsolatorFuture<'a, PremergeReport>;

/// MOD-139 M3 D6 (Confirm 2(a), blueprint P-4..P-6): the parts integrate's capture does not
/// hold, in plan order, deduplicated across repos. `heads` are integrate's own capture rows.
/// A read: no guard, no write.
fn unmerged<'a>(
    &'a self,
    step: StepId,
    trees: &'a [RunStepTree],
    heads: &'a [RunStepCommit],
    parts: &'a [PartTip],
) -> IsolatorFuture<'a, Vec<StepId>>;
```

The trait doc (`isolate.rs:137-140`, "four verbs plus four reads … all ten") becomes "…, MOD-139
M3's pre-merge verb and post-check read; all twelve". Three implementors change in the same commit:
`GixIsolator` (`real.rs:1435`), `FakeIsolator` (`fake.rs:645`), `StallAfterReconcile`
(`tests/gix_isolator.rs:2014`, two plain delegations).

**`GixIsolator::premerge`** (`Box::pin`): empty `trees` → `Ok(PremergeReport::default())` before
`self.cli()` (a repo-less run needs no `git`, D40's rule); otherwise `let git = self.cli()?.clone()`,
then per tree `premerge_repo`:

```rust
async fn premerge_repo(&self, git: &Cli, tree: &RunStepTree, parts: &[PartTip]) -> Result<RepoPremerge, IsolateError>;
async fn copy_part_ranges(&self, tree: &RunStepTree, parts: &[PartTip]) -> Result<(), IsolateError>;
async fn premerge_part(&self, git: &Cli, tree: &RunStepTree, part: StepId, tip: &str) -> Result<PartMerge, IsolateError>;
```

1. `copy_part_ranges` (only `tree.mode == Copy`): for every part with a commit row of
   `tree.repo_id` whose `after_hash` is `Some(tip)` and `!= before_hash`, find the part's tree row of
   that repo (mode `Copy`; absent → `IsolateError::Refused(copy_tree_vanished(..))`), then
   `with_retry("copy objects", || blocking(copy_range(part_copy, integrate_copy, before, tip)))` and
   `with_retry("create branch", || blocking(create_branch(integrate_copy, "htui/<part>", tip)))`
   (a label already at `tip` is tolerated, `git.rs:2811-2816`). Conflicted and not-attempted parts
   included (D2).
2. Per part, in order: no commit row or `after_hash` `None` (or equal to `before_hash`) →
   `NoCommits`; once the repo stopped → `NotAttempted`; else `premerge_part`.
3. `premerge_part`: `let _admin = in_place.then(|| self.admin_guard(repo))` (awaited; `Local |
   SharedSerialized` only); `head = blocking(head(path))`; `is_ancestor(path, tip, &head)` →
   `Contained`; else `with_retry("merge", || git.merge_no_ff_as(path, &premerge_message(part), &head,
   tip))` → `Merged(m)` → `PartMerge::Merged { commit: m.commit }`, `Conflicted(paths)` →
   `PartMerge::Conflicted { paths }` (and the caller stops the repo). `#[cfg(test)]
   self.probe_reconcile_admin(repo)` inside the guard, as `reconcile_isolated` does (`real.rs:1160-1161`).
4. `head` = the tree's `HEAD` after the last part; `name` = `checkout_of(tree)?.name`.

**`GixIsolator::unmerged`** (`Box::pin`): no head row → `Ok(vec![])` before `cli()`. Per head row
(repo): `descendant = after_hash.unwrap_or(before_hash)`; `repo = repo_holding(trees,
head.repo_id, descendant)` — the copy tree when it holds `descendant` (`has_commit`), else the
checkout: `range_of`'s D74 rule, factored out (`real.rs:1338-1358`) and reused there unchanged. It
never opens a worktree path, so a worktree capture removed (D27, `real.rs:1051-1066`) is never read.
Then §1.10's rule per part. Result in plan order, each step once.

**`FakeIsolator`** (`fake.rs`), new fields after `in_place_overlaps` (`:177`):

```rust
/// MOD-139 M3 T1: scripted `premerge` answers, FIFO, one consumed per call.
premerge_answers: Mutex<VecDeque<PremergeScript>>,
/// `(integrate step, part steps in the order handed)` per `premerge` call.
premerge_requests: Mutex<Vec<(StepId, Vec<StepId>)>>,
/// `premerge` calls so far, refusals and stalls included.
premerges: Mutex<u32>,
/// 1-based `premerge` calls that never return, with their signal (one-shot each).
premerge_stall: Mutex<Vec<(u32, Arc<Notify>)>>,
/// Scripted `unmerged` answers by part key, FIFO, one consumed per call.
unmerged_answers: Mutex<VecDeque<Vec<String>>>,
/// `(integrate step, part steps)` per `unmerged` call.
unmerged_requests: Mutex<Vec<(StepId, Vec<StepId>)>>,
/// 1-based `reset` calls that never return (counted by `resets`), with their signal.
reset_stall: Mutex<Vec<(u32, Arc<Notify>)>>,
```

```rust
/// One scripted `premerge` answer (by part key, blueprint P-15).
#[derive(Debug, Clone)]
enum PremergeScript {
    /// The default report, with `key` `Conflicted { paths }` in every repo and later parts
    /// `NotAttempted`.
    Conflict { key: String, paths: Vec<String> },
    /// `Err(IsolateError::Refused(reason))`.
    Refuse(String),
}

pub fn conflict_premerge(&self, key: &str, paths: &[&str]);
pub fn refuse_premerge(&self, reason: &str);
#[must_use] pub fn stall_nth_premerge(&self, n: u32) -> Arc<Notify>;
#[must_use] pub fn stall_nth_reset(&self, n: u32) -> Arc<Notify>;
pub fn script_unmerged(&self, keys: &[&str]);
#[must_use] pub fn premerge_requests(&self) -> Vec<(StepId, Vec<StepId>)>;
#[must_use] pub fn unmerged_requests(&self) -> Vec<(StepId, Vec<StepId>)>;
```

`IsolatorCall` (`fake.rs:182-195`) gains `Premerge(StepId)` and `Unmerged(StepId)`, both the
**integrate** step (D11's log filter). `premerge`: `log` first, `premerges += 1`, record the
request, stall (`notify_one` then `std::future::pending()`), then the script; unscripted it answers
the **default report**: per tree row `RepoPremerge { repo_id, name: repo_id.to_string(), head:
<last merge commit, else tree.base_ref>, parts }` with, per part, `Merged { commit:
format!("fake:merge:{part}:{n}") }` (`n = self.tick()`) when it has an `after_hash` in that repo,
else `NoCommits`. `unmerged`: `log`, record, then the scripted keys mapped to the parts' steps (an
unknown key is ignored), else `Vec::new()`. `reset` (`:948-973`): after `log` and `resets += 1`, a
matching `reset_stall` entry signals and never returns. `call_log_of` (`conformance.rs:11327-11334`)
gains `IsolatorCall::Premerge(step) => format!("premerge {}", name(step))` and `Unmerged` likewise
(P-14).

### 1.4 The git layer (T1; `isolate/git.rs`)

```rust
// after Merged (:487-492)
/// MOD-139 M3 D3: what one `merge --no-ff` did — merged, or conflicted (aborted) with its paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeAttempt {
    /// The two-parent post-condition held.
    Merged(Merged),
    /// Exit 1 with no lock signature and a non-empty conflicted-path read; the merge was aborted.
    Conflicted(Vec<String>),
}

// impl Cli, before merge_no_ff (:702)
/// MOD-139 M3 D3: D25's merge under any `message`, detached (plan D160) like `merge_no_ff`.
/// # Errors
/// As `merge_no_ff`, less the conflict: that is `Ok(MergeAttempt::Conflicted(paths))`.
pub async fn merge_no_ff_as(&self, path: &Path, message: &str, before: &str, after: &str) -> Result<MergeAttempt, IsolateError>;

// merge_no_ff (:726) keeps its signature and becomes:
//   match self.merge_no_ff_as(primary, &reconcile_message(step), before, after).await? {
//       MergeAttempt::Merged(merged) => Ok(merged),
//       MergeAttempt::Conflicted(paths) => Err(IsolateError::Refused(merge_conflict(&paths))),
//   }
// merge_no_ff_reading (:746) takes `message: &str` in place of `step` and answers
//   Result<MergeAttempt, IsolateError>; its conflict tail (`:810`) is
//   Ok(MergeAttempt::Conflicted(conflicted)). The test at :3679 passes `&reconcile_message(step)`.

// beside reconcile_message (:1600-1603)
/// MOD-139 M3 D3: a pre-merge commit's message names the part step it merged.
#[must_use]
pub fn premerge_message(part: StepId) -> String { format!("htui: premerge {part}") }
```

`reconcile_isolated`'s refusal text is unchanged (`merge_conflict(..)`, `git.rs:498-500`), now built
from paths rather than recovered from a string; pinned by `merge_no_ff_keeps_its_conflict_refusal`.

### 1.5 Prompt types and renderers (T2; `htui-core/src/prompt/`)

```rust
// mod.rs, PromptSpec after `part` (:128-131)
/// MOD-139 M3 D5: integrate's pre-merge report, rendered as the internal protected `premerge`
/// section after `part`. `None` everywhere but an integrate session; rendered for a Phase spec
/// with at least one repo only (blueprint P-10).
pub premerge: Option<PremergeReport>,
/// MOD-139 M3 D5: the done parts integrate merges, rendered as the internal trimmable `parts`
/// section after `premerge`. Empty everywhere but an integrate session; Phase spec only.
pub parts: Vec<IntegratePart>,

// mod.rs, after PartBlock (:195-220)
/// MOD-139 M3 D5: one done part as integrate's `parts` section renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegratePart {
    /// `SnapshotPart.key`, the block's `key` attribute.
    pub key: String,
    /// `SnapshotPart.title`, the block's `title` attribute.
    pub title: String,
    /// `(repo name, tip)` per repo the part committed to, report order; empty = no commits.
    pub tips: Vec<(String, String)>,
    /// `Isolator::diff` over the part's rows; rendered stat-only. `None` when it answered none
    /// or failed (a trim note, `engine.rs:8469-8481`'s shape).
    pub diff: Option<DiffBlock>,
    /// The part's highest-version output document body; `None` renders no body.
    pub body: Option<String>,
}

// mod.rs, after PART_SEPARATOR (:222-224)
/// MOD-139 M3 D5: the blank line after `premerge`, counted in the **frame** whenever the section
/// renders (protected, never dropped: `Part`'s rule).
pub const PREMERGE_SEPARATOR: &str = "\n\n";
/// MOD-139 M3 D5: the blank line after `parts`, counted in the section's **own span**, written by
/// `substitute` only while the section is live (`MEMORY_SEPARATOR`'s rule).
pub const PARTS_SEPARATOR: &str = "\n\n";

/// MOD-139 M3 D4: integrate's item note, the section's own text under one header line; `None` for
/// a report with no repo (blueprint P-10).
#[must_use]
pub fn premerge_note(node: &str, attempt: i32, report: &PremergeReport) -> Option<String>;
// = Some(format!("premerge `{node}` attempt {attempt}:\n{}", render::premerge_content(report)))
```

`render.rs` (after `part`, `:713-744`):

```rust
/// MOD-139 M3 D5: the section's opening paragraph, byte-pinned.
pub const PREMERGE_PREAMBLE: &str = "htui merged the done parts into this tree, in plan order, before this session. Finish every merge it could not make and commit it: the step passes only when each part's tip is in this branch's history and each part's changed paths are among the ones this step changes.";
/// MOD-139 M3 D5: conflicted paths listed per part, first in byte order; the rest counted.
pub const PREMERGE_PATH_CAP: usize = 20;
/// MOD-139 M3 D5: the `parts` section's opening line.
pub const PARTS_PREAMBLE: &str = "The done parts, in plan order: each one's tips, diffstat and output document.";

/// The `premerge` content (the note's body too): the preamble, then per repo
/// `` repo `<name>` at <head>: `` and one line per part:
///   ``- part `<key>` <tip>: merged as <commit>``
///   ``- part `<key>` <tip>: already contained``
///   ``- part `<key>`: no commits``
///   ``- part `<key>` <tip>: conflicted, merge aborted; conflicted paths:`` then
///     ``  - <path>`` × min(n, 20) in byte order, then ``  - and <n-20> more`` when n > 20
///   ``- part `<key>` <tip>: not attempted, an earlier part conflicted``
/// Lines joined by `\n`; LF-normalised. No step id anywhere (P-9).
#[must_use] pub fn premerge_content(report: &PremergeReport) -> String;
/// `name="premerge"`, no attributes; `None` when `report.repos` is empty.
#[must_use] pub fn premerge(report: &PremergeReport) -> Option<Rendered>;

/// MOD-139 M3 D5: one part block's state on the `parts` trim ladder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartState {
    /// The whole body.
    Full,
    /// The body head+tail-cut to this text.
    Cut(String),
    /// No body: key, title, tips and stat only.
    Bare,
}
/// `PARTS_PREAMBLE`, then per part (blocks joined by `\n\n`):
///   `<part key="…" title="…">` (`attr`, `title_attr`), one ``tip `<repo>` <hash>`` line per
///   tip or `no commits`, the stat (`diff_content(diff, true)`) when present, the body per state
///   (`content_of(body)` / the cut / nothing) after one blank line, then `</part>`.
/// `None` when `parts` is empty. `states.len() == parts.len()`.
#[must_use] pub fn parts(parts: &[IntegratePart], states: &[PartState]) -> Option<Rendered>;
```

Touch points (D5's list, with the compiler-forced ones marked **F**):
- `SectionName`: `Premerge`, `Parts` after `Part`; `render` **F** (`"premerge"`, `"parts"`);
  `is_protected` gains `Premerge` only (hand edit, `_ => false`).
- `Placeholder`: `Premerge`, `Parts` after `Part`, **not** in `ALL`; `token` **F**
  (`"premerge"`, `"parts"`); `allowed_in`'s Phase `!matches!` list gains both (hand edit);
  `is_section` needs no edit (true by construction) but its doc lists both.
- `scrubbed_inputs` (`mod.rs:1119-1134`'s shape, Phase only): under `premerge` every repo name,
  head, part key, tip, merge commit and conflicted path; under `parts` every key, title, tip pair,
  `diff.{range,stat,diff}` and body.
- `render_sections` **F**: `Premerge => premerge_of(spec).and_then(render::premerge)`, `Parts =>
  parts_of(spec).and_then(|p| render::parts(p, &vec![PartState::Full; p.len()]))`, with
  `premerge_of`/`parts_of` beside `part_of` (`:849-855`): Phase only, `premerge_of` also requires a
  repo.
- `assemble`: pushes after `Part` (`:612-614`), `Premerge` then `Parts`; frame pushes
  `PREMERGE_SEPARATOR` when `premerge_of(spec).is_some()` (`:664-667`'s shape).
- `substitute` (`:808-813`): after `Part`, `Premerge` + `PREMERGE_SEPARATOR`, then `Parts` +
  `PARTS_SEPARATOR`. **Render order (byte contract):** persona, memory, part, premerge, parts, body
  spans, output.
- `trim.rs`: `span_tokens` adds `PARTS_SEPARATOR` for `SectionName::Parts` (`:371-377`);
  `TrimStep::Parts`; `trim_order(Phase)` = `[Excerpts, Memory, Upstream, PreviousDiff,
  VerifyFailure, Documents, Parts, Item]` and its doc names R-PRM-3's "prior documents" rank;
  `Source::Parts(Vec<PartState>)`; `source_of` **F** (`Premerge` → `Fixed`, `Parts` →
  `Parts(vec![Full; n])`); `run` **F** (`TrimStep::Parts => self.trim_parts()`); `trim_diff`'s
  enumerated `None` arm **F** gains both.
- `trim_parts` (re-renders from `self.inputs.spec.parts`, the **masked** spec: the C-1 rule): rung 1,
  last part first, each part with a body: `head_tail` of its masked body to
  `floor_for(body, DOCUMENT_FLOOR_LINES, DOCUMENT_FLOOR_PERCENT)` with a yardstick that re-renders
  the whole section with that part `Cut(candidate)` and measures `span_tokens`; record
  `elided_lines`/`elided_bytes` summed, `mark(HeadTail)`; stop at `deficit() <= 0`. Rung 2, last
  first: `Bare`, `dropped += 1`, `mark(Dropped)`. Rung 3: `drop_section` (separator goes with it).
- Every `PromptSpec` literal gains `premerge: None, parts: Vec::new()` — nine at `c4fbf8a7`
  (`git grep -n "part: None\|part: Some" -- crates`): `fixtures.rs:240`, `:306`, `:402`, `:550`;
  `engine.rs:7452`, `:8352`; `promote.rs:110` (handoff, explicit, like `part`), `:484`;
  `htui/src/preview.rs:318`. The rest spread a base (`..spec`).

### 1.6 Failure vocabulary and sentences (T3)

```rust
// status.rs, after PartExhausted (:197-205)
/// MOD-139 M3 D6: integrate's post-check found done parts its capture does not hold.
IntegrateIncomplete {
    /// The divided node's name.
    node: String,
    /// The parts' keys, plan order.
    parts: Vec<String>,
},

/// MOD-139 M3 D6: `IntegrateIncomplete`'s one sentence, shared by `RunFailure` and `StepFailure`.
#[must_use]
pub fn integrate_incomplete(node: &str, parts: &[String]) -> String;
// one part:  integrate_incomplete: part `ui` of `implement` is not merged
// several:   integrate_incomplete: parts `api`, `ui` of `implement` are not merged

// gate.rs, StepFailure after VerifyFailed (:130)
/// MOD-139 M3 D6: integrate's post-check (Confirm 2(a) included) refused its capture.
IntegrateIncomplete { node: String, parts: Vec<String> },
// Display: status::integrate_incomplete(node, parts); run_failure(): Some(RunFailure::IntegrateIncomplete{..})

/// MOD-139 M3 D6: the post-check's answer `settle` reads.
#[derive(Debug, Clone, Copy)]
pub struct Unmerged<'a> {
    /// The divided node (`divide::divided_name`).
    pub node: &'a str,
    /// The parts that failed, by key; empty = the check passed.
    pub parts: &'a [String],
}
// SettleInput gains, last: `pub unmerged: Option<Unmerged<'a>>` — `None` for every step but an
// integrate whose session returned `Ok`.
// settle(): after the MissingOutput return (:267-269), before VerifyFailed (:270-272):
//   if let Some(found) = input.unmerged.filter(|found| !found.parts.is_empty()) {
//       return Settle::Failed(StepFailure::IntegrateIncomplete { node: found.node.to_owned(), parts: found.parts.to_vec() });
//   }

/// The word a parked integrate's note carries (Confirm 3).
pub const INTEGRATE_INCOMPLETE: &str = "integrate_incomplete";
```

| Text | Where written | Pinned by |
|---|---|---|
| `integrate_incomplete: part `ui` of `implement` is not merged` (and the plural) | `run.failure`, `RunFailure` `Display` | `run_failure_display_is_ana2s_bytes` (`status.rs:929`) both forms |
| same bytes as the step's gate note; item note `` step `implement` attempt 1: integrate_incomplete: part `ui` of `implement` is not merged `` (`note_step`, `gate.rs:515-538`) | `StepFailure` `Display` | `step_failure_display_is_the_gate_notes_vocabulary` (`gate.rs:1844`): display + `run_failure_text` |
| `premerge `implement` attempt 1:\n` + `premerge_content` | item note, `via_step_id` integrate (D4) | `prompt_parts::the_premerge_note_is_the_section_text`; T3 cases 1, 2 |
| `parts: diffstat of part `<key>` unavailable: <err>` (withheld if unmaskable, `withhold_unmaskable_notes`) | spec trim note | T3 unit `a_part_diff_error_is_a_trim_note` |
| `step <id> is parked on integrate_incomplete: approving, skipping or promoting it would land it without a part; retry or reject it instead` (R-5; `EngineError::IntegrateUnmerged` ends the same) | `EngineError::IntegrateIncomplete` | T3 case 11, R1's `an_exhausted_integrate_park_is_rejected_not_retried` (`to_string()` asserted) |
| `integrate node `<name>` has no done planner, admitted plan or done parts` | `EngineError::Snapshot { reason }` from `integrate_stage` | `divide.rs` unit tests (the pure `None`) |

Settle precedence (pinned in `gate.rs` tests): driver error > stop > cap > deadline >
`MissingOutput` > `IntegrateIncomplete` > `VerifyFailed`.

### 1.7 Resolve vocabulary (T4; `graph.rs`)

```rust
// NotRunnable (:311-328): `Divide` deleted; after PlannerAgent:
/// MOD-139 M3 D10: a divided node with no default parts; only its planner can plan them.
#[error("a divided node with no default parts needs its planner, which runs from MOD-140 on")]
NoParts,
/// MOD-139 M3 D10: a divided node whose planner may plan past its own parts.
#[error("a divided node that plans past its own parts (`plan_scope` `successors` or `graph`) runs from MOD-140 and MOD-141 on")]
PlanScope,

// ResolveError, after NotYetRunnable (:293-304):
/// MOD-139 M3 D10 (blueprint P-11): a default-parts divided node admission would refuse, refused
/// before any run exists. The sentence is the `RunFailure`'s, which names the node.
#[error("{reason}")]
DivideRefused {
    /// The divided node's name.
    node: String,
    /// What `divide::admission_refusal` answered.
    reason: crate::status::RunFailure,
},

/// MOD-139 M3 D10: the first planner node, position order, `divide::admission_refusal` refuses.
fn divide_refusal(snapshot: &GraphSnapshot) -> Option<ResolveError>;
```

`not_yet_runnable(phases: &[SnapshotPhase])` (flag gone), first refusal wins: (0) `PlannerAgent`
(unchanged, first); (1) per **planner node** in position order, `divide.default_parts` empty →
`NoParts`, then `plan_scope` not `None | Some(PlanScope::Parts)` → `PlanScope`; (2) `Palette`;
then the rest unchanged. `divide_refusal` runs after the `GraphSnapshot` literal
(`graph.rs:722-749`) and before `Ok(Resolution::Resolved(..))` (`:751`), returning `Err`.
`resolve_divided` (`:410-438`) deleted; `resolve_with`/`resolve_or_refused_with` lose `lift_divide`
and their `too_many_arguments` allows (`:442-445`, `:544-547`; seven parameters remain);
`resolve_or_refused` (`:520-540`) and `resolve` call them without it. `resume_window`'s
not-comparable arm (`engine.rs:4530-4537`) gains `| ResolveError::DivideRefused { .. }`. The
`RunFailure` sentences of §1.6 and M2 (`status.rs:292-347`) are the refusal bytes.

### 1.8 `divide::integrate_inputs` (T3; pure)

```rust
/// MOD-139 M3 D1: what an integrate attempt reads, selected from rows the caller holds.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegrateInputs {
    /// The latest `done` planner row of the divided node.
    pub planner: RunStep,
    /// Its highest-revision `admitted` plan.
    pub plan: RunPlan,
    /// The plan's latest parts, every one `done`, position order.
    pub parts: Vec<RunStep>,
}

/// The planner node is the `Planner` phase whose `divided_name` equals `node.name`; its row is
/// `latest_at(steps, planner_node.position)` with `role == Planner` and `status == Done`; the plan
/// is `plans` with that `planner_step_id`, `Admitted`, highest `revision`; the parts are
/// `latest_parts(steps, planner.id)`. `None` when any is missing, no part exists, or a latest part
/// is not `done` (integrate is reached only after `walk::parts_state` read the node `Done`).
#[must_use]
pub fn integrate_inputs(snapshot: &GraphSnapshot, node: &SnapshotPhase, steps: &[RunStep], plans: &[RunPlan]) -> Option<IntegrateInputs>;
```

Unit tests (`divide.rs` tests): `integrate_inputs_take_the_latest_planner_plan_and_parts` (a
re-planned revision 1 wins; retried parts give their latest attempt) and
`integrate_inputs_are_none_while_a_part_is_not_done`.

### 1.9 The engine's integrate path (T3)

```rust
/// MOD-139 M3 D1: one integrate attempt's inputs, read once and shared by its hooks (beside
/// `PartStage`, engine.rs:9495).
#[derive(Debug)]
struct IntegrateStage {
    /// `divide::integrate_inputs`' answer.
    inputs: crate::divide::IntegrateInputs,
    /// The plan's `base_refs`, decoded by `plan_parts`.
    base: BTreeMap<RepoId, String>,
    /// The plan's parts, decoded by `plan_parts` (titles).
    planned: Vec<htui_core::model::SnapshotPart>,
    /// One per `inputs.parts`, plan order.
    tips: Vec<PartTip>,
}
impl IntegrateStage {
    /// `FanoutSlot { index: 0, width: 1, base }`, `None` under `local` (`real.rs:666-668`).
    fn slot(&self, isolation: Isolation) -> Option<FanoutSlot<'_>>;
    /// `steps`' keys in plan order (unknown steps skipped).
    fn keys(&self, steps: &[StepId]) -> Vec<String>;
}

/// MOD-139 M3 D5: what stage 3 adds to integrate's spec.
#[derive(Debug, Clone)]
struct IntegratePrompt {
    premerge: PremergeReport,
    parts: Vec<htui_core::prompt::IntegratePart>,
    /// Diff-error trim notes, already withheld when unmaskable.
    notes: Vec<String>,
}

/// `drive_parts`' decode (`engine.rs:6326-6344`), factored out so the parts round and integrate
/// cannot diverge: `base_refs` and `body.parts`, with the two existing `Snapshot` sentences.
fn plan_parts(run: RunId, plan: &RunPlan) -> Result<(BTreeMap<RepoId, String>, Vec<htui_core::model::SnapshotPart>), EngineError>;

async fn integrate_stage(&self, run: &Run, snapshot: &GraphSnapshot, node: &SnapshotPhase) -> Result<IntegrateStage, EngineError>;
async fn premerge(&self, run: &Run, step: &RunStep, node: &SnapshotPhase, stage: &IntegrateStage, trees: &[RunStepTree]) -> Result<PremergeReport, EngineError>;
async fn integrate_prompt(&self, run: &Run, node: &SnapshotPhase, stage: &IntegrateStage, premerge: PremergeReport) -> Result<IntegratePrompt, EngineError>;
async fn unmerged_keys(&self, step: &RunStep, stage: &IntegrateStage, trees: &[RunStepTree], heads: &[RunStepCommit]) -> Result<Vec<String>, EngineError>;
```

- `integrate_stage`: `run_steps` + `run_plans` once; `integrate_inputs` → `None` →
  `EngineError::Snapshot { run, reason: "integrate node `<name>` has no done planner, admitted plan or
  done parts" }`; `plan_parts`; per part `step_trees`, `step_commits`, key from
  `divide::part_key(node_key)`.
- `premerge`: `isolator.premerge(step.id, trees, &stage.tips)?`; then, when
  `premerge_note(divided_name(node), step.attempt, &report)` is `Some`, `self.note(item, body,
  Some(step.id), self.now())`. Once per attempt, recovery re-runs included (D4).
- `integrate_prompt`: repo names from `store.repos(run.project_id)` (id string when absent); per tip:
  `isolator.diff(&tip.trees, &tip.commits)` (`Err` → note `parts: diffstat of part `<key>`
  unavailable: <err>` through `withhold_unmaskable_notes`, block without stat), `output_of(item,
  node, tip.step)` body, title from `planned`.
- `unmerged_keys`: `isolator.unmerged(step.id, trees, heads, &stage.tips)?` → `stage.keys(..)`.
- `assemble_prompt` (`engine.rs:8069-8094`) gains a last parameter `integrate:
  Option<&IntegratePrompt>` (its `too_many_arguments` allow stays, reason updated); `Some` sets
  `spec.premerge = Some(p.premerge.clone())`, `spec.parts = p.parts.clone()`, extends
  `spec.notes`. Its three callers pass `None` except `walk_live_step`.

`walk_live_step` (`engine.rs:5017-5244`), hooks a no-op unless `phase.role == NodeRole::Integrate`:
1. Before `prepare` (`:5031`): `let integrate = (phase.role == Integrate).then(..)` →
   `Box::pin(self.integrate_stage(run, snapshot, phase)).await?`; `prepare(.., phase.isolation,
   integrate.as_ref().and_then(|stage| stage.slot(phase.isolation)))`.
2. After `upsert_step_tree` (`:5041-5044`), before `record_commits(before)` (`:5055-5058`, P-21):
   `Box::pin(self.premerge(run, step, phase, stage, &trees)).await?`.
3. Stage 3 (`:5064-5066`): `Box::pin(self.integrate_prompt(..)).await?`, passed to
   `assemble_prompt`. Its `MissingInput`/`Refused`/`NoPersona` exits are unchanged.
4. After `record_commits(after)` (`:5175-5178`), before `settle` (`:5183`): when the session
   returned `Ok`, `Box::pin(self.unmerged_keys(step, stage, &trees, &after)).await?`; `SettleInput {
   .., unmerged: keys.as_deref().map(|parts| Unmerged { node: divided_name(phase), parts }) }`.
5. Landing unchanged (`Landing::Advance` → `reconcile_done_step(&row, &step, &[])`, `:5222-5225`).

Every `Err` in 1–4 reaches `walk_step`'s `fail_hard` (`engine.rs:5001-5009`): the step and the run
fail, no retry (D3), as a prepare error does. `drive_cursor` (`:4203-4210`, `:4219-4233`) drops
both `integrate_waits` returns: `Create` loses its `NodeRole::Integrate` early return and admits
as every node does (`divide::row_role` writes `role = integrate`); `Run(step)` reads
`NodeRole::Integrate | NodeRole::Step => self.walk_step(..)` (`Planner` keeps `admit_planner`). `integrate_waits`
(`:9531-9538`) is deleted.

**Recovery** (`engine.rs:3529-3597`): `StepKind::of` (`recover.rs:207-217`) classifies
`StepRole::Integrate` → `Integrate` after `Part`; `classify`'s `captured` arm (`recover.rs:265-271`)
reads `StepKind::Plain | Judge | Part | Integrate`. `recover_step` gains, **before** `(_,
Adjudication::Finished { .. })` (M2 G-6's lesson):

```rust
(StepKind::Integrate, Adjudication::Finished { verify, verify_exit_code }) => {
    self.finish_recovered(step, verify, verify_exit_code, now).await?;
    let stage = Box::pin(self.integrate_stage(run, snapshot, phase)).await?;
    let parts = Box::pin(self.unmerged_keys(step, &stage, &trees, &commits)).await?;
    let settled = recover::resettle(output.as_ref(), verify, snapshot.reads_verdict(&phase.name),
        step.started_at.unwrap_or(now), now,
        Some(Unmerged { node: crate::divide::divided_name(phase), parts: &parts }));
    Box::pin(self.land_recovered(run, snapshot, phase, step, settled)).await
}
```

`recover::resettle` (`recover.rs:340-361`) gains a sixth parameter `unmerged:
Option<Unmerged<'_>>` passed into its `SettleInput`; the generic arm passes `None`. Reset and
NeverReset stay on the `(_, …)` arms (D8).

### 1.10 Confirm 2(a): the post-check rule, exactly (T1 implements, T3 consumes)

Per head row `h` of integrate's capture (one per repo of its scope):
- `descendant = h.after_hash.unwrap_or(h.before_hash)`; `repo = repo_holding(trees, h.repo_id,
  descendant)` (the integrate copy when it holds `descendant`, else the checkout; P-4..P-6).
- Per part `p` in plan order with a commit row `c` of `h.repo_id`:
  - `c.after_hash` `None` or equal to `c.before_hash` → **exempt** (no commits: plan D6).
  - `tip = c.after_hash`. **Ancestry:** `!is_ancestor(repo, tip, descendant)` → `p` fails.
  - **Paths:** `own = git.name_only(repo, c.before_hash, tip)`, `merged = git.name_only(repo,
    c.before_hash, descendant)` (the **part's** `before_hash`, P-5). Exempt when `own.paths` is
    empty or either list is `truncated` (P-6). Otherwise `own ∩ merged == ∅` → `p` fails.
- A part failing in any repo appears once. The engine maps the steps to keys; `settle` reports one
  `StepFailure::IntegrateIncomplete { node, parts }` for both halves (same sentence, §1.6).
- Ranges are two-dot tree diffs (`name_only`'s own shape), so a whole-part `-s ours` merge leaves
  `merged` without the part's paths and fails; a part whose paths another part also changed passes
  the path half (accepted: the check is cheap and deterministic, not a proof).

### 1.11 Confirm 3: the refused answers (T3)

```rust
// gate.rs, beside stopped_to_ask (:782-792)
/// MOD-139 M3 D6 (Confirm 3, blueprint P-1): whether `step` is parked on its own
/// `integrate_incomplete` settle — the item note `note_step` wrote via the step,
/// ``step `<phase>` attempt <n>: integrate_incomplete:``. A park writes no `gate_note`.
#[must_use]
pub fn parked_unmerged(notes: &[Note], step: &RunStep) -> bool;

// command.rs, EngineError after RoleRow (:638-645)
/// MOD-139 M3 D6 (Confirm 3): an answer that would land an integrate step parked on a part its
/// capture does not hold. Retry or reject is the way on (R-5).
#[error("step {step} is parked on integrate_incomplete: approving, skipping or promoting it would land it without a part; retry or reject it instead")] // R-5
IntegrateIncomplete {
    /// The parked integrate step.
    step: StepId,
},

// command.rs, after answer_gate_enabled (:744-766)
/// MOD-139 M3 D6 (Confirm 3): `Err(IntegrateIncomplete)` for an `awaiting_approval` integrate
/// row `parked_unmerged` read true; `Ok(())` for every other row.
/// # Errors
/// [`EngineError::IntegrateIncomplete`].
pub fn unmerged_park(step: &RunStep, parked_unmerged: bool) -> Result<(), EngineError>;
```

Command paths: `Engine::answer_gate` (`engine.rs:1414-1433`), after `answer_gate_enabled(..)?`,
when `step.role == StepRole::Integrate` and the answer is `Approved | Skipped` (P-2): `let notes =
Box::pin(self.parts.store.notes(item)).await?; unmerged_park(&step, gate::parked_unmerged(&notes,
&step))?`. `Engine::promote` (`engine.rs:2072`), after `promote_enabled(..)?`, the same for an
integrate row (P-3). `Rejected` and `RetryStep` are untouched. No row is written before the refusal.

### 1.12 The stack rule (ANA-33 §9; memory htui-orch-test-stack-headroom)

Box every new await of a node-sized future: `integrate_stage`, `premerge`, `integrate_prompt` and
`unmerged_keys` in `walk_live_step`; `integrate_stage`, `unmerged_keys` and `land_recovered` in the
new `recover_step` arm; the `answer_gate`/`promote` notes read; `Orchestrate::enqueue`'s
`enqueue_fake` call; every new `case()` frame arm in `conformance.rs` (`integrate_case`,
`divided_case`'s and `divided_recovery_case`'s new arms). The isolator verbs are already boxed
(`IsolatorFuture`). The suite runs `--no-fail-fast` and the log is grepped for `SIGABRT`; an
overflow is fixed by boxing the rare branch first (`premerge`'s refusal path, the recovery arm).

### 1.13 Harness (T1 fake, T3/T4 cases)

- Integrate is position 3 in divided FEAT-3 (`prd` 0, `plan` 1, `implement:plan` 2, `implement` 3,
  `review` 4; parts 5, 6; `conformance.rs:13800-13870`). Its session key is `("implement",
  attempt)`; `hold_sessions(&[("implement", 1)])` holds it; `stall_after_done("implement", 1, None,
  write_output)` crashes it.
- Live reconcile order in a full run: `prd`, `plan`, integrate, `review` (planner and parts never
  reconcile), so integrate's reconcile is the **third** call (`stall_nth_reconcile(3)`); case 1's
  pinned log (`reconcile` entries in order) is the check on that ordinal before case 8 relies on it.
- New helpers (`conformance.rs`, in the M3 block): `fn integrate_of(steps: &[RunStep]) -> RunStep`
  (latest `role == Integrate` row; panics on none), `fn integrate_rows(steps: &[RunStep]) ->
  Vec<(i32, StepStatus)>` (attempt order), `async fn finished<O: Orchestrate>(orch: &O, run: RunId)`
  (asserts `run_of(..).status == Done`, `failure == None`, `walk::ready == [Finished]`). All ≤ 3
  underscores.

---

## T1 — Isolator: `premerge`, `unmerged`, git (parallel with T2)

### T1.0 The report types, on the primary checkout, before either worktree exists

The coordinator (or T1's implementer) writes §1.2 on `hr/MOD-139` in the primary checkout and
commits it **before** creating the T1 and T2 worktrees; both branch from that commit. This is the
only htui-core model change of M3; T2 reads the types and never edits `model/run.rs` or
`model/mod.rs`.

- Commit: `feat(core): MOD-139 M3 T1, pre-merge report types`
- Gate: `cargo test -p htui-core --features test-support --lib -- model::` (the round-trip test),
  `cargo clippy -p htui-core --all-targets --all-features -- -D warnings`.

### T1.1 Git (`isolate/git.rs`)

§1.4. Tests in `git.rs`' tests (beside `merge_no_ff_of_a_descendant_makes_a_two_parent_commit`,
`:3583`):
1. `merge_no_ff_as_writes_the_given_message` — the merge commit's subject is `htui: premerge <id>`
   and its parents `[before, after]`.
2. `merge_no_ff_as_reports_conflicted_paths_and_aborts` — two commits change one line apart:
   `Ok(MergeAttempt::Conflicted(["f"]))`, no `MERGE_HEAD`, `HEAD == before`, clean tree.
3. `merge_no_ff_keeps_its_conflict_refusal` — the same conflict through `merge_no_ff`:
   `Err(IsolateError::Refused(s))` with `s == merge_conflict(&["f".into()])` (reconcile's text
   unchanged).
4. The existing seam test (`:3657-3690`) passes `&reconcile_message(step)` and still sees the read
   failure surfaced after the abort.

### T1.2 Trait, `PartTip`, the three implementors

§1.3. `lib.rs:78-81` re-exports `PartTip`. `StallAfterReconcile` (`tests/gix_isolator.rs:2014-2101`)
delegates both verbs. `conformance.rs:11327-11334` gains the two `call_log_of` arms (P-14).

### T1.3 `GixIsolator` (`isolate/real.rs`)

§1.3's `premerge`/`unmerged`, §1.10's rule; `repo_holding` factored out of `range_of`
(`:1338-1358`) with `range_of` reading through it unchanged. Real-git tests in `real.rs`' tests
(fixture repos as `real.rs`' existing ones; `git` absent → the test returns early as theirs do):

| # | Test | Asserts |
|---|---|---|
| 1 | `premerge_in_a_worktree_merges_each_part_in_plan_order` | two worktree parts on disjoint files: two `Merged`, in order; integrate `HEAD`'s first-parent chain is `merge(ui) → merge(api) → base`; each subject `htui: premerge <part>` |
| 2 | `premerge_in_a_copy_copies_every_part_range_and_labels_it` | copy mode: both part tips resolve in the integrate copy, `htui/<part>` names each tip there, two `Merged` whose parents are `[HEAD, tip]` (the PRD's open question: the two-parent post-condition inside a copy after `copy_range`) |
| 3 | `premerge_in_place_takes_the_admin_lock` | `shared_serialized`: `Merged`; `reconcile_held`/`reconcile_unheld` probes (`real.rs:300-304`, `:582`) read the admin lock held during each merge |
| 4 | `premerge_under_local_reads_the_single_part_contained` | `local`, integrate prepared with no slot on the part's tip: `Contained`, no commit made |
| 5 | `premerge_stops_a_repo_at_its_first_conflict` | parts `a`, `b`, `c`; `b` conflicts with `a`: `[Merged, Conflicted { paths: ["f"] }, NotAttempted]`, no `MERGE_HEAD`, `head` = `a`'s merge |
| 6 | `premerge_reports_a_part_without_commits` | `NoCommits`, nothing written |
| 7 | `premerge_keeps_repos_independent` | two repos, `core` conflicts and `ui` does not: `core` `[.., Conflicted, NotAttempted]`, `ui` all `Merged` |
| 8 | `premerge_reads_a_part_reached_through_another_merge_contained` | `b` branched from `a`'s tip: `[Merged, Contained]` |
| 9 | `unmerged_names_a_part_whose_tip_is_not_an_ancestor` | integrate committed without merging `ui`: `[ui]` |
| 10 | `unmerged_passes_every_merged_part` | after test 1's pre-merge and capture: `[]` |
| 11 | `unmerged_reads_before_for_a_local_integrate_with_no_commits` | `local`, integrate `after_hash` `None`, `before` = the part tip: `[]` (P-5) |
| 12 | `unmerged_names_a_part_merged_with_strategy_ours` | `git merge -s ours <ui tip>` committed by hand: ancestry holds, paths do not intersect: `[ui]` (Confirm 2(a)) |
| 13 | `unmerged_exempts_parts_with_no_commits_or_no_paths` | a no-commit part and an `--allow-empty` part: `[]` (P-6) |
| 14 | `unmerged_opens_the_checkout_after_capture_removed_the_worktree` | worktree integrate that committed nothing (capture removed the tree, D27): reads the checkout, answers `[api, ui]` (tips not ancestors of base), no error |
| 15 | `premerge_without_trees_needs_no_git` | `GixIsolator::with_git(config, Err(..))`, empty trees: `Ok(PremergeReport::default())` |

### T1.4 `FakeIsolator` (`fake.rs`)

§1.3. Tests in `fake.rs`' tests (beside `the_fake_reset_is_scripted_and_empty_by_default`, `:3371`):
1. `the_fake_premerge_merges_by_default_and_logs_first` — default report shape (`Merged` per
   committed part, `NoCommits` otherwise, `head` = the last merge); `call_log()` holds
   `Premerge(step)`; `premerge_requests()` the parts in order.
2. `a_scripted_premerge_conflict_stops_the_repo` — `conflict_premerge("ui", &["src/ui/a.rs"])`: `ui`
   `Conflicted`, later parts `NotAttempted`; next call default again (FIFO).
3. `a_refused_premerge_is_an_error` — `refuse_premerge("x")`: `Err(Refused("x"))`.
4. `the_fake_unmerged_is_scripted_by_key_and_empty_by_default`.
5. `premerge_and_reset_stalls_never_return` — `stall_nth_premerge(1)`, `stall_nth_reset(2)`: each
   signals and stays pending (`tokio::time::timeout` elapses), the call logged first.

### T1.5 Commits

1. `feat(core): MOD-139 M3 T1, pre-merge report types` (T1.0, primary checkout).
2. `feat(orch): MOD-139 M3 T1, merge_no_ff_as and MergeAttempt` — §T1.1.
3. `test(orch): MOD-139 M3 T1, premerge and unmerged verbs, the fake's script and stall points,
   real-git cases (red)` — §T1.2, §T1.4 (green), §T1.3's tests (red): the three implementors change
   together, `GixIsolator`'s two verbs answering `Err(IsolateError::Git("premerge: not built
   yet".into()))` until commit 4.
4. `feat(orch): MOD-139 M3 T1, GixIsolator premerge and the post-check` — §T1.3 green.

Each commit compiles: `cargo check -p htui-core -p htui-orch --all-targets --all-features` first.

### T1.6 Validate (in the T1 worktree)

```bash
cargo fmt --all -- --check
cargo test -p htui-core --features test-support --lib -- model::
cargo test -p htui-orch --features test-support --lib -- isolate:: fake:: 2>&1 | tee /tmp/mod139-m3-t1.log
cargo test -p htui-orch --features test-support --test gix_isolator 2>&1 | tee -a /tmp/mod139-m3-t1.log
grep -c SIGABRT /tmp/mod139-m3-t1.log    # expect 0
cargo clippy -p htui-core -p htui-orch --all-targets --all-features -- -D warnings
cargo clippy -p htui-core -p htui-orch -- -D warnings
```

---

## T2 — Prompt: `premerge` and `parts` (parallel with T1, from T1.0's commit)

### T2.1 Vocabulary and fields

§1.5's types, fields, `SectionName`, `Placeholder` and the nine literals. Pins moved here:
`section_names_are_one_vocabulary_with_two_uses` (`mod.rs:1556-1572`) gains `Premerge` →
`"premerge"`, `Parts` → `"parts"`; `the_protected_set_is_role_dependent_only_for_failure_reason`
(`:1574-1610`) lists `Premerge` protected and `Parts` trimmable;
`the_three_orders_are_the_documented_ones` (`trim.rs:1317-1343`) gains `TrimStep::Parts` between
`Documents` and `Item`. Fixtures (`fixtures.rs`, beside `part_api`): `pub fn premerge_one_conflict()
-> PremergeReport` (one repo `core`, `api` merged, `ui` conflicted on 22 paths, `db` not attempted)
and `pub fn integrate_parts() -> Vec<IntegratePart>` (`api` and `ui`, tips, stats, 300-line bodies).

### T2.2 Renderers, frame, substitute, trim

§1.5. `premerge_note` is re-exported from `prompt` for the engine.

### T2.3 Tests (`htui-core/tests/prompt_parts.rs`, new; `prompt_part.rs`' helpers mirrored)

| # | Test | Asserts |
|---|---|---|
| 1 | `the_premerge_and_parts_placeholders_are_internal` | `{{premerge}}`, `{{parts}}` refused as unknown in every role's body; neither in `Placeholder::ALL`; `allowed_in(Phase)` false |
| 2 | `integrate_sections_render_in_the_pinned_order` | a spec with persona, memory, part, premerge and parts and `document_tool`: byte offsets persona < memory < part < premerge < parts < body < output |
| 3 | `the_integrate_sections_are_recorded_in_render_order` | `trim.sections` names `template, persona, memory, part, premerge, parts, …, output` |
| 4 | `the_premerge_separator_is_counted_in_the_frame` | `template_tokens` with a premerge = without + `est(PREMERGE_SEPARATOR)` |
| 5 | `the_parts_separator_rides_with_its_section` | the `parts` row's tokens include `PARTS_SEPARATOR`; once dropped, the text holds no extra blank line |
| 6 | `an_oversized_premerge_refuses_the_prompt` | protected: a target below the premerge alone → `AssembleError` (the protected-floor refusal, as `an_oversized_brief_refuses_the_prompt`) |
| 7 | `conflicted_paths_are_capped_at_twenty_in_byte_order` | 22 unsorted paths: the first 20 in byte order, then `  - and 2 more`, in the section **and** `premerge_note` |
| 8 | `the_parts_ladder_cuts_the_last_body_first` | rung 1 at a target that only `ui`'s cut clears: `ui` head+tail, `api` whole; row `head_tail`, `elided_lines` > 0 |
| 9 | `the_parts_ladder_drops_bodies_last_first_and_keeps_tips_and_stat` | rung 2: `ui` bare (key, title, tip, stat kept), row `dropped` 1, strategy `dropped` |
| 10 | `the_parts_ladder_drops_the_section_last` | rung 3: no `parts` section, row `tokens_after` 0 |
| 11 | `parts_trim_after_documents_and_before_item` | an over-budget spec with a document, parts and a long item: the document floors first, the item last |
| 12 | `premerge_and_parts_secrets_are_masked` | a secret in a conflicted path, a part title and a part body is masked under each section's name |
| 13 | `only_a_phase_spec_renders_premerge_and_parts` | judge and handoff specs carrying both render neither, and are not refused over them |
| 14 | `a_part_with_no_document_renders_its_header_and_stat` | `body: None`: block has key, title, tip, stat, no body |
| 15 | `a_report_with_no_repo_renders_no_premerge_section` | `PremergeReport::default()`: no section, no frame separator, `premerge_note` `None` |
| 16 | `the_premerge_note_is_the_section_text` | `premerge_note("implement", 1, r)` == `"premerge \`implement\` attempt 1:\n"` + the section's content |

`promote.rs` (beside `the_handoff_spec_drops_the_part`, `:539-557`):
`the_handoff_spec_drops_premerge_and_parts` — a phase spec with both → `handoff_spec` has `premerge:
None`, `parts` empty, and its assembled text names neither section.

The `prompt_golden__*` snapshots do not move (no spec there carries either field).

### T2.4 Commits

1. `feat(core): MOD-139 M3 T2, premerge and parts spec fields and vocabulary` — §T2.1 (every
   literal, including `engine.rs`'s two, `promote.rs` and `preview.rs`), pins moved.
2. `test(core): MOD-139 M3 T2, premerge and parts prompt cases (red)` — §T2.3's file.
3. `feat(core): MOD-139 M3 T2, premerge and parts render, frame and trim ladder` — §T2.2, green.
4. `test(orch): MOD-139 M3 T2, a handoff spec carries neither integrate section`.

### T2.5 Validate (in the T2 worktree)

```bash
cargo fmt --all -- --check
cargo test -p htui-core --features test-support --test prompt_parts --test prompt_part --test prompt_golden --test prompt_digest --test prompt_memory --test prompt_persona
cargo test -p htui-core --features test-support --lib -- prompt::
cargo test -p htui-orch --features test-support --lib -- promote::
cargo test -p htui --all-features --lib -- preview
git status --short '*.snap'     # expect empty
cargo clippy -p htui-core -p htui-orch -p htui --all-targets --all-features -- -D warnings
cargo clippy -p htui-core -p htui-orch -p htui -- -D warnings
```

### Merging T1 and T2

Both branch from T1.0. Merge T1, then T2, into `hr/MOD-139` on the primary checkout (no shared
file, §5); run `cargo check --workspace --all-targets --all-features` and both validate blocks on
the merged tree before T3 starts (memory: parallel fan-out hidden coupling). Remove both worktrees
before deleting their branches.

---

## T3 — Engine: integrate runs, fails, recovers; every divided case to its real end (serial, after T1+T2)

Test-first: §T3.6's twelve cases and the nineteen rewrites (commit 3) are red on the merged T1+T2
tree; commits 4 and 5 turn them green. Cases still enqueue through the seam
(`orch.enqueue_divided`); T4 swaps it.

### T3.1 `status.rs`, `gate.rs`, `recover.rs`: the failure vocabulary

§1.6. Twenty `SettleInput` literals gain `unmerged: None` (`git grep -n "SettleInput {"`:
`engine.rs:5183`, `:5985`, `:6735`; sixteen in `gate.rs`; `recover.rs:350`). `resettle` gains its
sixth parameter; its five test calls (`recover.rs:1456-1499`) pass `None`. New `gate.rs` tests
beside `settle`'s: `integrate_incomplete_outranks_verify_not_missing_output` (unmerged + verify
fail → `IntegrateIncomplete`; unmerged + no output → `MissingOutput`; unmerged + deadline →
`DeadlineElapsed`) and `an_empty_unmerged_list_settles_ok`. `parked_unmerged` unit test:
`parked_unmerged_reads_the_steps_own_note` (another step's note, another attempt's, and a
non-integrate note read false).

### T3.2 `divide.rs` and `plan_parts`

§1.8 with its two unit tests; `plan_parts` (§1.9) factored out of `drive_parts`
(`engine.rs:6312-6344`, decode only; the plan lookup stays), `drive_parts` calls it.

### T3.3 `engine.rs`: the integrate path

§1.9: `IntegrateStage`, `IntegratePrompt`, the four methods, the `walk_live_step` hooks,
`assemble_prompt`'s parameter, `drive_cursor`'s two arms, `integrate_waits` deleted. Unit test in
`engine.rs`' tests (beside `changed_paths_unavailable_is_a_note_not_a_failure`, `:19289`):
`a_part_diff_error_is_a_trim_note` (`fail_diff` on a part: the block renders without stat, the
spec's notes hold the withheld sentence, the step runs on).

### T3.4 Recovery and the refused answers

§1.9's recovery (`recover.rs`: `StepKind::Integrate`, `classify`; `engine.rs`: the arm), §1.11
(`gate.rs`, `command.rs`, `engine.rs` `answer_gate` and `promote`). `recover.rs` unit test beside
`classify`'s: `an_integrate_row_classifies_by_the_plain_rule`. `command.rs` unit test beside
`operator_row_refuses_parts_and_stages` (`:1490`): `unmerged_park_refuses_only_a_parked_integrate`.

### T3.5 Harness (`conformance.rs`)

§1.13's helpers in a `// -- MOD-139 M3 T3: integrate runs, fails and recovers (D1-D9, Confirm 2(a), 3) --`
block after the M2 R1 block (`:15126ff`), and the frame `fn integrate_case<'a, H: CaseHarness>(name:
&str, harness: &'a H) -> Option<Pin<Box<dyn Future<Output = ()> + 'a>>>` (every arm `Box::pin`),
tried in `case()` (`:1022-1041`) after `divided_recovery_case`. `stops_at_integrate`
(`:13705-13718`) is deleted with its twenty call sites (`git grep -n stops_at_integrate` empty after
T3). `CASES` (`:617-1001`) gains, after `"a_planner_left_done_replays_the_loop"`, the comment
`// MOD-139 M3 T3 (plan D1-D9, Confirm 2(a), 3): integrate runs, fails and recovers.` and §T3.6's
twelve names; its doc (`:429-616`) gains a "**Twelve for MOD-139 M3 T3**" paragraph, and the
"Eighteen for MOD-139 M2 T3" paragraph's "where M2 run stops, D15" clause becomes "and then runs".

### T3.6 The twelve new cases

Each starts `harness.fresh()` and `divided_feat_3(&orch, &[("api", None), ("ui", None)], ..)`
unless stated. "Done" means `finished(&orch, run)` (§1.13).

| # | Case | Setup | Asserts |
|---|---|---|---|
| 1 | `a_divided_run_lands_one_integrate_merge` | `primary_repo` | Done. `integrate_of`: position 3, `role Integrate`, attempt 1, `Done`; its tree's `base_ref == base_ref_of(run, repo)` (the slot). `call_log_of` holds, in this order, `prepare 3/1/0`, `premerge 3/1/0`, `capture 3/1/0`, `unmerged 3/1/0`, `reconcile 3/1/0`, and no `reconcile 5/…`/`6/…`. `isolator().reconciles()` has integrate once, no part. `premerge_requests() == [(integrate, [api, ui])]`. `prompt_sections(integrate)` starts `["template", "premerge", "parts"]`; its text holds `key="api"` and `key="ui"`. Notes: one starting ``premerge `implement` attempt 1:\n`` with `via` integrate. |
| 2 | `a_conflicted_premerge_reaches_the_agent_and_the_run_lands` | `primary_repo`; `conflict_premerge("ui", &["src/ui/b.rs", "src/ui/a.rs"])` | The integrate prompt and the premerge note both hold ``- part `ui` `` … `conflicted, merge aborted; conflicted paths:` then `  - src/ui/a.rs`, `  - src/ui/b.rs` (byte order). Between `premerge 3/1/0` and `capture 3/1/0` the log holds no isolator call (the orchestrator writes no resolution). Done. |
| 3 | `an_unmerged_part_fails_integrate_incomplete_and_retries` | `retry_limit = 1`; `script_unmerged(&["ui"])` | `integrate_rows == [(1, Failed), (2, Done)]`; a note `` step `implement` attempt 1: integrate_incomplete: part `ui` of `implement` is not merged ``; log holds `premerge 3/2/0`, `unmerged 3/2/0`, `reconcile 3/2/0`, no `reconcile 3/1/0`; `premerge_requests().len() == 2`. Done. |
| 4 | `an_unmerged_part_with_a_fails_edge_re_plans_to_revision_one` | `retry_limit = 0`; `add_edge(implement → plan, Fails, None, Some(1))`; `script_unmerged(&["ui"])` | Position 1: `[(1, Superseded), (2, Done)]`; planners `[(1, Superseded), (2, Done)]`; first planner's parts both `Superseded`; second's `implement/api@1` (7), `implement/ui@1` (8) `Done`; plans `[(0, Abandoned), (1, Admitted, base_position 7)]`; `integrate_rows == [(1, Failed), (2, Done)]`; the second `premerge_requests` entry names the `@1` parts. Done. |
| 5 | `an_integrate_verify_fail_retries_the_integrate` | `retry_limit = 1`, `verify_command = Some("cargo test")`; verifier FIFO `pass()`, `pass()`, `fail(1)`, `pass()` (prd, plan, integrate ×2; parts never verify, P-16) | Integrate attempt 1 `Failed`, `verify_outcome Fail`, note `` step `implement` attempt 1: verify_outcome: fail ``; attempt 2 `Done`; `unmerged 3/1/0` was logged (the post-check ran before verify's settle reading). Done. |
| 6 | `a_crash_mid_premerge_re_prepares_and_re_runs` | `retry_limit = 1`; `primary_repo`; `stall = orch.isolator().stall_nth_premerge(1)`; `crash_divided(&orch, &[stall])` | Integrate attempt 1 `running` at the crash, no session spec. `orch.restarted()`: `adopted_next == Walk`; attempt 1 `Failed`, gate note `interrupted`. Resume → Done; `integrate_rows == [(1, Failed), (2, Done)]`; `other`'s log holds `prepare 3/2/0`, `premerge 3/2/0`; one integrate session (`spec_for` attempt 2 only). |
| 7 | `a_crash_after_an_unmerged_session_fails_rather_than_lands` | repo-less; `retry_limit = 0`; `stall_after_done("implement", 1, None, true)`; crash | `other.isolator().script_unmerged(&["ui"])`; `adopted_next` answers the run `Finished` with `RunFailure::IntegrateIncomplete { node: "implement", parts: ["ui"] }`; `run.failure` is its sentence; integrate `Failed`; `other.isolator().reconciles()` holds no integrate; `unmerged_requests()` names integrate once. |
| 8 | `a_crash_after_integrate_done_lands_through_the_frontier` | `primary_repo`; `stall_nth_reconcile(3)` (§1.13); crash | Integrate `Done` at the crash, unmerged. `other`: `adopted_next == Walk`; `other.isolator().reconciles()` holds integrate exactly once; resume → Done; `other.spec_for(session("implement", 0))` is `None` and no attempt 2 exists (no twin session). |
| 9 | `a_shared_serialized_integrate_prepares_from_the_base_refs` | `isolation = SharedSerialized`; `primary_repo`; `stall_nth_reset(1)` (the round's end reset); crash | `orch`'s log ends `reset 6/1/0`; no integrate row. `other`: Walk; resume → Done; integrate's tree `base_ref == base_ref_of(run, repo)` (prepared from `base_refs`, D9); `other.isolator().in_place_overlaps() == 0`. |
| 10 | `a_dirty_in_place_integrate_parks_on_recovery` | `isolation = SharedSerialized`; `primary_repo`; `retry_limit = 1`; `stall_after_done("implement", 1, None, false)`; crash | `other.isolator().script_reset_refusal("dirty_tree_not_reset: <path>")`; the sweep parks: `Rest { AwaitingApproval, Some(3), Some(Interrupted { phase: "implement", reset: false }) }`; integrate `Failed`, `interrupted, tree not reset`; resume re-parks; no attempt 2 (D8, D93). |
| 11 | `approving_an_integrate_incomplete_park_is_refused` | `mutate: gate = OnFailure`; `script_unmerged(&["ui"])` | Run `AwaitingApproval` at 3. `AnswerGate { Approved }`, `AnswerGate { Skipped }`, `PromoteStep { chat_open: false }` on integrate each `Err(EngineError::IntegrateIncomplete { step })` with §1.11's sentence; rows unchanged. `RetryStep` → attempt 2 runs → Done. |
| 12 | `a_premerge_refusal_fails_the_integrate_and_the_run` | `retry_limit = 1`; `refuse_premerge("premerge refused")` | Run `Failed`; integrate attempt 1 `Failed` (`fail_hard`), no attempt 2; log has `premerge 3/1/0` and no `capture 3/1/0`; no integrate session (`spec_for` `None`) (D3: no retry). |

### T3.7 The nineteen rewritten cases (twenty `stops_at_integrate` sites)

Each keeps every M2 assertion that still holds and replaces the stop with the run's real end. The
first column is the call site at `c4fbf8a7`.

| Site | Case | Real end and what it asserts now |
|---|---|---|
| `:13797` | `a_divided_node_admits_its_default_parts_as_revision_zero` | Done. "No row at 3" and `ready == [Create { 3, 1 }]` become: integrate row at 3 (`role Integrate`, attempt 1, `Done`) and `ready == [Finished]`; plan, planner and part shape asserts unchanged. |
| `:13905` | `each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent` | Done. Part trees, agents, prompts unchanged; log adds `reconcile 3/1/0`, still no `reconcile 5/…`/`6/…`. |
| `:13961` | `part_documents_stay_out_of_input_resolution` | Done. `implement` producers = both parts **plus integrate**; `resolve_inputs(.., ["implement"])` answers integrate's document (`produced_by_step_id == integrate`), never a part's. |
| `:14023` | `integrate_is_offered_only_once_every_part_is_done` | In-flight asserts unchanged; after release Done; integrate's `started_at` ≥ every part's `finished_at`; `ready == [Finished]`. |
| `:14048` | `two_parts_are_live_at_once_under_the_cap` | Done; high water still 2; both parts `Done`. |
| `:14076` | `max_parallel_sessions_one_runs_the_parts_one_at_a_time` | Done; high water 1. |
| `:14095` | `shared_serialized_parts_reset_to_the_base_refs` | Done; the `" 5/"`/`" 6/"` filtered log unchanged (premerge/unmerged log position 3, D11); integrate tree `base_ref == base`; overlaps 0; high water 1. |
| `:14132` | `a_single_local_part_prepares_with_no_slot` | Done; integrate prepared with no slot too: its `base_ref` starts `fake:base:` and differs from `base`; `premerge_requests()` names `[api]`. |
| `:14285` | `a_part_with_a_live_sibling_gets_command_run` | Done; part asserts unchanged; integrate's spec (`session("implement", 0)`) has **no** `command_run` (alone, no live sibling: D13). |
| `:14333` | `the_operator_verbs_refuse_a_part_row` | `hold_sessions(&[("implement", 1)])`; joined: `within(hold.live(1))`, then the four refusals and "no row moved" exactly as before, then `release`; claim → Done. |
| `:14608` | `a_failed_part_retries_alone_from_the_base_refs` | Done; retry asserts unchanged; `premerge_requests()[0].1 == [api.id, retry.id]` (the latest attempt). |
| `:14733` | `an_exhausted_part_takes_the_fails_edge_and_re_plans` | Done; loop asserts unchanged; one integrate row (attempt 1 `Done`) whose premerge names the `@1` parts. |
| `:14847` | `a_crash_before_admission_re_admits_on_the_same_planner_row` | `resume` → `Resume::Walked(Rest { Done, .. })`; part asserts unchanged; one integrate row `Done`. |
| `:14869`, `:14880` | `a_crash_after_admission_re_plans_nothing_and_lands_no_part` | P-18: `stall_after_done("implement", 1, None, true)` + `crash_divided` replaces the stop. `other`: Walk through `(Integrate, Finished)`; `reconciles()` has no part, integrate once, the planner at most once; resume → Done; one planner, one plan, one integrate row; `other.spec_for(session("implement", 0))` `None`. |
| `:14931` | `a_crashed_part_with_its_output_is_done_on_recovery` | resume → Done; part shape unchanged; one integrate row `Done`. |
| `:14971` | `a_crashed_part_without_output_retries_alone` | resume → Done; shape unchanged; premerge names api attempt 2. |
| `:15087`, `:15117` | `a_local_divided_node_lands_no_part_on_recovery` | P-18: `stall_after_done("implement", 1, None, false)` + crash; `other`: Walk, reconciles hold no part (at most the planner), no `reconcile 5/` in either log; integrate attempt 1 `Failed` `interrupted`; resume → Done; `integrate_rows == [(1, Failed), (2, Done)]`; part shape and one plan unchanged. |
| `:15482` | `a_cut_short_part_retirement_replays_the_loop`, `a_planner_left_done_replays_the_loop` (both through `cut_retirement_replays`) | resume → Done; replay asserts unchanged; one integrate row (attempt 1 `Done`) on revision 1's parts. |

Not rewritten (no stop): the five admission cases, `a_fan_out_only_branch_…`, `stage_rows_…`,
`start_run_refuses_…`, `an_exhausted_part_fails_the_run_with_part_exhausted` (fails before
integrate), `a_crashed_part_in_a_dirty_local_tree_parks` (parks before), the M2 R1 cases.

### T3.8 Pins

- `conformance.rs` `cases_are_unique_and_counted` (`:15563-15571`): `219` → **`231`**; the
  arithmetic string gains ` + 12` and the prose ", and MOD-139 M3 T3's twelve integrate cases (plan
  D1-D9, Confirm 2(a), 3)".
- `tests/fake_conformance.rs:16-71`: `219` → **`231`**; the message appends ", and MOD-139 M3 T3's
  twelve integrate cases (one integrate merge landed, a conflicted pre-merge handed to the agent,
  an unmerged part retried and looped, a verify failure, crashes mid-pre-merge, after an unmerged
  session and after integrate's `done`, `shared_serialized` from base refs, a dirty in-place park,
  the refused approval and a refused pre-merge) 231".
- `run_failure_display_is_ana2s_bytes` and `step_failure_display_is_the_gate_notes_vocabulary`
  (§1.6).

### T3.9 Commits

1. `feat(orch): MOD-139 M3 T3, integrate_incomplete settle and failure vocabulary` — §T3.1.
2. `feat(orch): MOD-139 M3 T3, integrate inputs and the shared plan decode` — §T3.2.
3. `test(orch): MOD-139 M3 T3, twelve integrate cases and the divided cases' real ends (red)` —
   §T3.5–§T3.8.
4. `feat(orch): MOD-139 M3 T3, integrate runs as a session step with pre-merge and post-check` —
   §T3.3.
5. `feat(orch): MOD-139 M3 T3, integrate recovery and the refused landing answers` — §T3.4.

### T3.10 Validate

```bash
cargo fmt --all -- --check
cargo test -p htui-orch --features test-support --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m3-t3.log
grep -c SIGABRT /tmp/mod139-m3-t3.log      # expect 0
grep -E "^test result|FAILED|panicked" /tmp/mod139-m3-t3.log | sort | uniq -c
cargo test -p htui-core --features test-support --test prompt_parts --test prompt_golden
cargo test -p htui-worker --all-features --lib -- views
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
git grep -n "stops_at_integrate\|integrate_waits" -- crates     # expect empty
```

Never wait on the suite with a `pgrep -f "cargo test…"` loop (it matches itself; memory
agent-pgrep-wait-loops-self-match).

---

## T4 — Resolve lift and the real enqueue (serial, after T3)

Test-first: commit 1's resolve case and graph tests are red on T3's tree.

### T4.1 `graph.rs`

§1.7. Unit tests (`graph.rs` tests):
- `a_divide_node_is_expanded_then_refused` (`:5239-5278`) → `a_divide_node_is_expanded_and_resolves`:
  the same graph resolves; `phases[2] == ("implement:plan", Planner)`, `phases[3] == ("implement",
  Integrate)`. (Its owned path `src/api` parses: `divided_feat_3`'s `src/<key>/**` already admits.)
- `the_seam_lifts_only_the_divide_refusal` (`:5345-5394`) →
  `resolve_refuses_a_planner_agent_first_then_what_admission_would`: planner agent with no parts →
  `PlannerAgent` (beats `NoParts`); no parts → `NoParts`; `plan_scope Successors` → `PlanScope`;
  palette → `Palette` (unchanged); `isolation Local` with two parts → `DivideRefused { node:
  "implement", reason: LocalCannotDivide { node: "implement", parts: 2 } }` and its `to_string()`
  is the `RunFailure` sentence.
- `not_runnable_sentences_name_their_milestone`: `NoParts` and `PlanScope` `to_string()` byte pins
  (§1.7).
- `resolve_divided_feat` (`:5308`) deleted with `resolve_divided`.

### T4.2 The seam deleted, the real enqueue added

- `engine.rs`: `enqueue_divided_fake` (`:10108-10161`) → `enqueue_fake`:

```rust
/// MOD-139 M3 D11: [`Engine::enqueue`] over a `FakeOrchestrator`'s parts — `Manual`, the item's
/// default scope, no workspace — so `start_enabled`, `missing_tags` and the production resolve
/// run as for `StartRun`.
///
/// # Errors
/// [`Engine::enqueue`]'s.
#[cfg(feature = "test-support")]
pub async fn enqueue_fake(orch: &crate::fake::FakeOrchestrator, item: ItemId) -> Result<RunId, EngineError>;
// body: enqueue_in_batch_fake's (:10168-10179) with `engine.enqueue(item, RunMode::Manual, None, None).await`
```

- `lib.rs:69`: `pub use engine::{claim_fake, dispatch_fake, enqueue_fake, resume_fake, sweep_fake};`.
- `conformance.rs`: `Orchestrate::enqueue_divided` (`:257-263`) → `async fn enqueue(&self, item:
  ItemId) -> Result<RunId, EngineError>;` (doc as `enqueue_fake`'s); the impl (`:405-407`) →
  `Box::pin(crate::engine::enqueue_fake(self, item)).await`. Every `.enqueue_divided(` becomes
  `.enqueue(` — eleven at `9423f50b` (`:13699`, `:13996`, `:14040`, `:14064`, `:14277`, `:14467`,
  `:14541`, `:15164`, `:15187`, `:15235`, `:15422`; `crash_divided` carries `:16968`'s test along),
  plus any T3 case that calls it directly. `start_divided`'s and `crash_divided`'s docs drop "D16 seam".
- `resume_window` (`engine.rs:4530-4537`) gains `| ResolveError::DivideRefused { .. }`.
- Doc refresh: `htui-core/src/model/run.rs:1095` ("MOD-139 M2 writes it, M3 runs it" → "MOD-139 M2
  writes it and M3 runs it: pre-merge, session, post-check, landing"), `:1202-1204`
  (`SnapshotDivide`: "M1 never stores one (resolve refuses…)" → resolve freezes and runs
  default-parts nodes from MOD-139 M3), and every hit of `git grep -n "seam\|D16\|M3 deletes" --
  crates/htui-orch/src crates/htui-core/src/model` that speaks of the divide seam.

### T4.3 Admission refusals move to resolve (P-12)

The five M2 cases `local_with_two_parts_fails_admission`,
`a_part_agent_outside_the_candidates_fails_admission`, `a_part_persona_fails_admission`,
`an_over_cap_plan_fails_admission` and `an_empty_divided_node_fails_admission` (`:14174-14264`) and
their helper `assert_admission_refused` (`:14146-14172`) are deleted from the code, `CASES` and
`divided_case`; `start_run_refuses_a_divided_graph_and_first_a_planner_agent` (`:14425-14473`) is
renamed **`start_run_refuses_a_planner_agent_and_what_admission_would`** (in `CASES`, the frame and
the fn) and folds them.

### T4.4 The renamed case

One orchestrator, `divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {})`; each block edits
the graph (`edit_graph`), then asserts that **both** `orch.enqueue(HTUI_FEAT_3)` and `StartRun`
answer `Err(EngineError::Resolve(<refusal>))`, `feat_3_run(&orch) == None`, the item stays `open`
and `notes_of` is unchanged. Refusals restored after each block. Order:
1. `planner_agent_id = Some(AGENT_CLAUDE)` and no parts → `NotYetRunnable { "implement", PlannerAgent }`.
2. No parts → `NotYetRunnable { "implement", NoParts }`.
3. `plan_scope = Some(PlanScope::Successors)` → `NotYetRunnable { "implement", PlanScope }`.
4. `isolation = Local` → `DivideRefused { "implement", LocalCannotDivide { parts: 2 } }`.
5. `max_parts = Some(1)` → `DivideRefused { .., PlanOverCaps { MaxParts, planned 2, max 1 } }`.
6. `ui.persona_id = Some(PERSONA_ARCHITECT)` → `DivideRefused { .., PartPersonaUnsupported { "ui" } }`.
7. `ui.agent_id = Some(AGENT_AGY)` → `DivideRefused { .., PartAgentNotCandidate { "ui", AGENT_AGY } }`.
8. `ui.owned_paths = ["/abs/**"]` → `DivideRefused { .., PartPathRefused { "ui", "/abs/**", "is an absolute path" } }`.
9. The valid graph: `StartRun` → `Ok(Started { rest })` with `rest.run == Done`.

Each `DivideRefused`'s `to_string()` equals its `RunFailure`'s.

### T4.5 Pins

- `cases_are_unique_and_counted`: `231` → **`226`**; the arithmetic string's M2 T3 term `18`
  becomes `13`, and the prose's "eighteen divided-node cases (…, five admission refusals, …)"
  becomes "thirteen divided-node cases (…, the resolve refusals and what admission would refuse,
  …)".
- `tests/fake_conformance.rs`: `231` → **`226`**; the message appends ", less the five admission
  refusals MOD-139 M3 T4 folds into the resolve case 226".
- `CASES` doc: the "Eighteen for MOD-139 M2 T3" paragraph becomes "Thirteen", drops "built through
  D16 resolve seam" and the five admission refusals, and names the renamed case.

### T4.6 Commits

1. `test(orch): MOD-139 M3 T4, resolve refuses what admission would and the real enqueue (red)` —
   §T4.1 tests, §T4.3–§T4.5.
2. `feat(orch): MOD-139 M3 T4, resolve runs divided nodes; NoParts, PlanScope and DivideRefused` —
   §1.7.
3. `refactor(orch): MOD-139 M3 T4, the divide seam deleted; every divided case through enqueue` —
   §T4.2.
4. `docs(core): MOD-139 M3 T4, stale M3 and seam docs refreshed`.

### T4.7 Validate

```bash
cargo fmt --all -- --check
cargo test -p htui-orch --features test-support --lib -- graph::
cargo test -p htui-orch --features test-support --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m3-t4.log
grep -c SIGABRT /tmp/mod139-m3-t4.log      # expect 0
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
git grep -n "enqueue_divided\|resolve_divided\|lift_divide\|NotRunnable::Divide\|stops_at_integrate" -- crates   # expect empty
```

---

## T5 — Real-git end to end and the workspace gate (serial)

### T5.1 Cases (`tests/gix_isolator.rs`, beside `criterion_18_a_finished_worktree_step_is_adopted_and_merged`, `:2430`)

A helper `async fn divide_implement(store: &MemStore, item: ItemId, parts: &[&str])` (≤ 3
underscores) rewrites the fixture graph's `implement` node to `Split::Divide`, `fan_out 1`, one
`PhasePart` per key with owned path `src/<key>/**` (the `repoint` shape, `:337`, through the store's
graph replace). Sessions commit through `CommittingSink` (`:431`) for `implement/api`,
`implement/ui`; a new `MergingSink` (≤ 3 underscores in its helpers) resolves the integrate
session's conflict: reads the conflicted part's tip from the premerge note, runs `git merge --no-ff
<tip>`, writes the union of both sides, `git add`, `git commit`, then writes the output document.

1. `a_divided_run_lands_one_integrate_merge_in_worktree_mode` — `Fixture::new(Isolation::Worktree,
   None)`, two parts on disjoint files, `StartRun` → `Done`. Primary `HEAD` is one `htui: reconcile
   <integrate>` merge whose first parent is the base; every part tip `is_ancestor` of it; no
   `htui: reconcile <part>` subject anywhere in `git log`; integrate's history holds two `htui:
   premerge <part>` merges.
2. `a_divided_run_lands_one_integrate_merge_in_copy_mode` — the same under `Isolation::Copy`: the
   part commits reach the primary on the reconcile merge's second-parent side (D7: `copy_range`
   walks `.all()`).
3. `a_conflicting_premerge_is_resolved_by_the_agent` — worktree, both parts change `src/shared.rs`:
   the premerge note names `ui` conflicted; `MergingSink` resolves; run `Done`; both tips ancestors
   of the primary `HEAD`; the post-check passed (`integrate` attempt 1 `Done`).

### T5.2 The gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m3-gate.log
grep -c SIGABRT /tmp/mod139-m3-gate.log    # expect 0
bash scripts/gate.sh
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
git status --short '*.snap'                # expect empty
```

Confirm in the gate: htui-orch `CASES.len() == 226` in both pins and
`fake_orchestrator_passes_every_case` green; htui-core's store pins unchanged at 267 (no store verb,
no migration: Confirm 1 chose the note; `cargo sqlx prepare --check` is not owed); linear and
compete suites green with no `.snap` drift; `htui-agent`'s extensibility test green (no `zeta`
literal); Qdrant-live timeouts in the sandbox are owed on the host (memory
qdrant-live-times-out-under-load); a `pg_conformance` stack overflow appears only in workspace
builds (grep `SIGABRT`, box the arm). Reviewer: rust-reviewer, unpinned model (memory
htui-opus-402-blocks-pinned-agents).

---

## 5. Per-task file sets

| Task | Where | Files |
|---|---|---|
| T1.0 | primary checkout, before fan-out | `htui-core/src/model/{run,mod}.rs` |
| T1 | worktree `target/wt/mod139-m3-t1` | `htui-orch/src/{isolate,fake,lib}.rs`; `htui-orch/src/isolate/{git,real}.rs`; `htui-orch/src/conformance.rs` (**only** `call_log_of`'s two arms, P-14); `htui-orch/tests/gix_isolator.rs` (`StallAfterReconcile` delegations) |
| T2 | worktree `target/wt/mod139-m3-t2` | `htui-core/src/prompt/{mod,template,render,trim,fixtures}.rs`; `htui-core/tests/prompt_parts.rs` (new); `htui-orch/src/engine.rs` (**only** the two `PromptSpec` literals, `:7420-7452`, `:8318-8352`); `htui-orch/src/promote.rs`; `htui/src/preview.rs` |
| T3 | primary | `htui-orch/src/{engine,divide,gate,status,recover,command,conformance}.rs`; `htui-orch/tests/fake_conformance.rs` |
| T4 | primary | `htui-orch/src/{graph,engine,lib,conformance}.rs`; `htui-orch/tests/fake_conformance.rs`; `htui-core/src/model/run.rs` (docs only) |
| T5 | primary | `htui-orch/tests/gix_isolator.rs`; whatever the gate exposes |

**T1 ∩ T2 = ∅.** T1 touches `isolate*`, `fake.rs`, `lib.rs`, two lines of `conformance.rs` and
`gix_isolator.rs`; T2 touches `prompt/*`, two literals of `engine.rs`, `promote.rs`, `preview.rs`.
No prompt type is in a T1 file, and T2's only T1 dependency (§1.2's report types) is in T1.0's commit,
which both branch from. `fake.rs` is T1's alone; T3 adds no fake method (every scripting and stall
point it uses is §1.3's). T3 and T4 share `engine.rs`, `conformance.rs` and `fake_conformance.rs` and
run in sequence on the primary checkout.

## 6. Risks and gotchas for implementers

- **G-1 Gortex hook.** In the primary checkout the hook blocks `Read`/`Grep`/`cat`/`wc` on indexed
  source: read with `mcp__gortex__read` (`operation:"source"`/`"file"`) or `git show HEAD:<path> |
  sed -n 'A,Bp'` and `git grep -n`; edit with anchored `python3 -I` replaces (memory
  gortex-in-hr-sandbox-answers-despite-inactive). Worktrees under `target/wt` are **not**
  Gortex-indexed and a Gortex `edit` writes the **primary** checkout (memory
  workflow-worktree-implementers): in a worktree use plain reads and scripted replaces with absolute
  worktree paths. A worktree's own `target/` costs about 10 GB (check `df -h .`; memory
  dev-postgres-crash-is-disk-pressure). Commit incrementally (memory
  implementer-agents-commit-incrementally); remove worktrees before `git branch -d`.
- **G-2 Features.** `-p htui-core`/`-p htui-orch` integration tests need `--features test-support`
  or they run zero tests and report ok (memory htui-integration-tests-need-testkit); `htui` uses
  `--all-features`. `--no-fail-fast` goes **before** `--`, `--test-threads=1` after (the keyring
  fake is process-wide).
- **G-3 Stack.** §1.12. htui-orch's walks sit near the 2 MiB debug stack; grep every log for
  `SIGABRT` (memory htui-orch-test-stack-headroom).
- **G-4 Exhaustive vs non-exhaustive matches.** The compiler forces: `IsolatorCall` in
  `call_log_of`; `SectionName::render`; `Placeholder::token`; `render_sections`; `source_of`;
  `Trimmer::run`; `trim_diff`'s enumerated arm; `RunFailure`'s and `StepFailure`'s `Display`;
  `classify`'s `captured` match. It does **not** force (hand edits): `is_protected` (`_ => false`),
  `allowed_in` (`!matches!`), `scrubbed_inputs`, `assemble`'s pushes and frame, `substitute`'s
  blocks, `span_tokens`' separator, `StepFailure::run_failure` (`_ => None`), `StepKind::of`, the
  `recover_step` arm **before** `(_, Finished)` (else integrate lands unchecked), `not_yet_runnable`,
  `resume_window`'s arm (`Err(err) => return Err(..)` catches it silently).
- **G-5 The fake is not git.** `FakeIsolator` merges nothing, never conflicts unless scripted, keeps
  no `HEAD`, and its `reset` relabels nothing (M2 G-2). A green fake case proves the engine's calls;
  T1's and T5's real-git tests own git's behaviour.
- **G-6 Verify FIFO and reconcile ordinals.** `FakeVerifier` and `stall_nth_reconcile` count every
  session and every landing of the run (P-16, §1.13): script and count from `prd`.
- **G-7 The frozen test clock.** `TestClock` moves only on `advance`/`set` (M2 G-15): assert
  "integrate started after the parts" with `>=`.
- **G-8 Notes.** The premerge note is skipped for repo-less runs (P-10); integrate's gate notes are
  item notes, not `run_step.gate_note` (P-1). Cases reading `notes_of(..).last()` in a repo run see
  the premerge note before the settle's.
- **G-9 `local` integrate.** No slot, `before` = the part's tip; the part reads `Contained` and the
  path check uses the part's own `before_hash` (P-5). A `local` integrate that commits nothing still
  passes.
- **G-10 Copy mode.** The pre-merge copies part objects into the integrate copy **before**
  `is_ancestor` (an absent ancestor reads "not one", `git.rs:1490-1503`), and the post-check opens the
  copy only when it holds the descendant.
- **G-11 Forbidden literal.** Never write the literal `zeta` anywhere (memory
  htui-zeta-is-a-forbidden-literal); only the workspace gate catches it.
- **G-12 Clippy twice.** `--all-features` hides code dead without `test-support`; also run
  `cargo clippy --workspace -- -D warnings` (memory htui-featureless-clippy-gate). `assemble_prompt`
  keeps its `too_many_arguments` allow (eight parameters); `resolve_with` and
  `resolve_or_refused_with` drop theirs (seven).
- **G-13 Push policy.** Never push; `origin` is the private forge (AGENTS.md).
