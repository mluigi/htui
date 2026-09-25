# MOD-4 - Orchestrator, manual mode (done, 2026-09-25)

**Origin:** ANA-2 (`docs/ANA-2.md`, `docs/decisions/ana/ana-2.md`).
**Requirements:** `R-ORCH-1..5`, `R-ORCH-7..11`, `R-TUI-4`, `R-TUI-9`, and the `run` and `close`
actions of `R-TUI-2`.
**Design authority:** `docs/ANA-2.md` (step graphs, the three compare-and-set status tables,
`htui-orch`), with `docs/ANA-5.md` §4.6 and §8 for the judge and handoff prompts. The amendments this
item made to ANA-2 are listed under "Deviations from ANA-2" below. Each one is marked in
`docs/ANA-2.md` itself.
**Artifacts:** PRD [`.claude/prds/mod-4-orchestrator-manual-mode.prd.md`](../../../.claude/prds/mod-4-orchestrator-manual-mode.prd.md).
Plans and `code-architect` blueprints, one pair per milestone:

| # | Plan | Blueprint |
|---|---|---|
| 1 | [`.claude/plans/mod-4-orch-seam.plan.md`](../../../.claude/plans/mod-4-orch-seam.plan.md) | [`.claude/plans/mod-4-orch-seam.blueprint.md`](../../../.claude/plans/mod-4-orch-seam.blueprint.md) |
| 2 | [`.claude/plans/mod-4-orch-engine.plan.md`](../../../.claude/plans/mod-4-orch-engine.plan.md) | [`.claude/plans/mod-4-orch-engine.blueprint.md`](../../../.claude/plans/mod-4-orch-engine.blueprint.md) |
| 3 | [`.claude/plans/mod-4-orch-tree.plan.md`](../../../.claude/plans/mod-4-orch-tree.plan.md) | [`.claude/plans/mod-4-orch-tree.blueprint.md`](../../../.claude/plans/mod-4-orch-tree.blueprint.md) |
| 4 | [`.claude/plans/mod-4-orch-fanout.plan.md`](../../../.claude/plans/mod-4-orch-fanout.plan.md) | [`.claude/plans/mod-4-orch-fanout.blueprint.md`](../../../.claude/plans/mod-4-orch-fanout.blueprint.md) |
| 5 | [`.claude/plans/mod-4-orch-lease.plan.md`](../../../.claude/plans/mod-4-orch-lease.plan.md) | [`.claude/plans/mod-4-orch-lease.blueprint.md`](../../../.claude/plans/mod-4-orch-lease.blueprint.md) |
| 6 | [`.claude/plans/mod-4-orch-drive.plan.md`](../../../.claude/plans/mod-4-orch-drive.plan.md) | [`.claude/plans/mod-4-orch-drive.blueprint.md`](../../../.claude/plans/mod-4-orch-drive.blueprint.md) |

**Commits:** six milestones, `33277b1`..`22cfeea`. Each milestone below names its own range.
**Spawned:** MOD-36 (weighted agent assignment, from milestone 4's OQ-7, blocked on ANA-21),
CLEAN-4 (R-9) and MOD-37 (the carried hardening risks).

## What shipped

`htui` runs a backlog item through its step graph and the maintainer drives it from the TUI. A new
crate, `htui-orch`, walks a frozen graph snapshot in six stages per step. It gates each step,
retries it, loops review back to implement, and fans a phase out to rival candidates. It then
selects one winner through a two-order judge or a human. It isolates every step's work in one of
four modes over real git trees and reconciles the winner into the primary. Every walk runs under a
leased heartbeat, and a crashed process's runs are adopted and adjudicated by a sweep.
`crates/htui/src/run_worker.rs` hosts the engine inside the store worker. The Runs pane drives
twelve keys. A step can be promoted to the Chat tab and its artifact accepted, and close-out writes
the summary document. Two stores, `MemStore` and `PgStore`, satisfy the seam through one conformance
suite. The migration set is `0003_orchestration.sql` (with its `cache_migrations/0003` companion)
and `0004_max_agents_per_run_default.sql`.

**Scope as it stood when implementation started.** ANA-2 §8 and §9 fixed the crate, the migration
pair, the writer and reader tables and the build order. MOD-25 (2026-09-16,
`docs/decisions/mod/mod-25.md`) made `htui` online-only. That struck ANA-10's M8 from this item:
there is no `LocalStore`, no local-only graph parity, no `local_migrations/0002_orchestration_local.sql`,
and two stores oblige the seam, not three. The `PgStore`-inherent reads of ANA-2 §8 became
`Backend`-inherent with a `match self`. One prohibition survived, which is never to add
`CHECK (fanout_index >= 0)` on either schema (ANA-2 risk 12).
**The cache migration is `0003`, not the `0002` ANA-2 §9 reserved**, because MOD-2 milestone 4 spent
`cache_migrations/0002_agent_mirror.sql` on mirroring the registry. ANA-2 §8's writer table is 17
rows and `docs/ANA-2.md:1744` names two more; its "sixteen" undercounted its own table.

**What MOD-2 handed over by name, and where each landed.**
- ANA-5 criterion 18's persistence half (MOD-2 D107): a handoff prompt is a `follow_up` event at the
  next `turn`, never a second `prompt` row, and `run_step.prompt_digest` is unchanged. **Landed in
  milestone 6** (D164, D165).
- ANA-5 criterion 3's `blocked` transition: **landed in milestone 6** (D162, generalised).
- ANA-5 criterion 16's front-matter parser: **landed in milestone 2** (`gate.rs`), with M2 D10's
  narrowing that only an exact `request-changes` rejects.
- MOD-2 finding F-104 (`excerpt::select`'s `vetted()` refuses provider candidates outside a
  truncated listing): **not loosened**, by PRD D4. Nothing registers an excerpt provider, and the
  engine passes an empty `ExcerptSet`. It stays the conservative direction until a provider exists.
- `available()` skipping every quota status that is not exactly `"allowed"`: **loosened in
  milestone 4** (D61, PRD D3). `allowed_warning` is selectable. `exhausted`, `utilization >= 1.0`
  and the cap rule are untouched.

---

## The six milestones

The PRD approved seven milestones (D1). Milestone 1's plan (D0) merged the first two, because the
store traits carry no default bodies. The first new trait method stops `htui-store` compiling, and
`PgStore` cannot implement one before `0003` exists. ANA-2 §9's build order is unchanged.

### 1 — The seam knows what a run is (`33277b1`..`e3163ca`, 2026-09-18)

37 commits over 120 files. The workspace was green at `--test-threads=1` (1304 passed), and
`sqlx prepare --check` was clean.
- `WriteStore` went from 42 to 60 methods. `ReadStore` gained 5 reads, including ANA-2 §4.2's
  `resolve_inputs`; `documents_of_kinds` deliberately is not one of them.
- There are 11 `Backend`-inherent reads, not sixteen, because five of ANA-2 §8's already existed.
- The three §4.3 tables became `can_move_to`, with the refusal inside the seam.
- `MemStore` and `PgStore` run over `0003_orchestration.sql` and its `cache_migrations/0003`
  companion. `run_step_tree` is the seventeenth mirrored table.
- Store conformance `CASES` went from 36 to 47 and `READ_CASES` from 6 to 9.

Two fixture corrections from the item's list were applied: `attempt` is 1-based, and both graph
runs carry a real `graph_snapshot`. The other two were already MOD-15's.
The review gate raised two seam questions and deferred both (seam blueprint §4.6):
- **R-1**: a terminal `transition_run`/`fail_run` did not write the item mirror in the same
  transaction. ANA-2 §4.3:543 asks for that, and §8's writer table does not provide it. Answered in
  milestone 2.
- **R-2**: `claim_run`'s overlap predicate compared repo sets on one box, so §4.7's
  isolation-aware rules I and P could not be evaluated inside the admission critical section. Two
  worktree-isolated runs on one repo were refused, which is safe but narrower than `R-ORCH-9`.
  Resolved in milestone 5.

### 2 — A graph walks (`cacab31`..`0a53d9c`, 2026-09-19; close-out round `f87c2b7`..`1cbddc5`, 2026-09-22)

22 implementation commits over 37 files, then eight close-out commits.
- **The crate.** `crates/htui-orch` is the fifth workspace crate. It holds `lib.rs`, `graph.rs`,
  `status.rs`, `command.rs`, `isolate.rs`, `engine.rs` and `gate.rs`, plus `fake.rs` and
  `conformance.rs` behind `test-support`. It depends on `htui-core` and `htui-agent` and never on
  `htui-store` (ANA-2 invariant 10).
- **What runs.** `prd -> plan -> implement -> review` runs against `FakeDriver`, a new
  `FakeIsolator` and `MemStore`. That covers the six-stage walk, the gate table, the review loop and
  its no-progress predicate, with no git, no Postgres, no agent and no second migration.
- **R-1 answered.** `finish_run` is the nineteenth writer. It moves the run and mirrors its item in
  one transaction on both stores. `PgStore` takes `run` then `item` `FOR UPDATE`, the order every
  other two-row writer already uses.
- **Pins.** R-2 was deferred to milestone 5; the engine adds no overlap reasoning of its own. Store
  `CASES` went from 47 to 48, and `htui-orch`'s own case list is 15.

The implementation corrected three plan decisions, all recorded in the plan:
- **D5** retires a `failed` step by cancelling it rather than leaving it alone. A `failed` latest
  attempt reads as a rest, so otherwise the walk would stop forever.
- **D10** records an unexpected review verdict as an `item_note` carrying `via_step_id`, not in
  `gate_note`.
- **D19** resolves the graph through a `GraphSource` trait that `htui-orch` defines itself, because
  eleven of milestone 1's reads are store-inherent and on no trait.

**The review gate found three HIGH and five MEDIUM findings.** Every finding was verified against
the tree and every one was applied (`9acb166`, `106671b`, `23fef33`, `9870673`, `60120c7`, `6345a4a`,
`ed7756c`, `c83ae97`, `cf2f08a`, `5d4582a`, `1cbddc5`). Two change behaviour a later item must know
about:
1. **A step that errors after `pending -> running` used to leave the run running forever.** `cursor`
   rests on `running`, and every command refuses a running step. The walk now delegates to
   `walk_live_step` and hard-fails through `fail_hard` (`running -> failed` plus `finish_run`).
   **A spawn failure lands in `failed` under every gate** and never parks, per ANA-2 `:639`.
2. **`RetryStep` on a terminal run created an orphan `pending` step and reported success.** Neither
   store refuses a step on a terminal run, so the refusal is the engine's
   (`EngineError::RunStatus`). ANA-2's `failed -> retry -> queued` row (`:585`) belongs to the
   *item* table and remains unimplemented. `EngineError` also gained `Snapshot` and `Stalled`, so
   the walk's own invariants no longer arrive as `Store(Constraint)`.

A second §4.2/§4.3 disagreement is recorded in the plan's risks, beside D2's. ANA-2 `:450` says a
`never` gate with an exhausted `failed` settle ends the run `awaiting_approval`, while `:575`/`:608`
and the shipped code end it `failed`, so `:450` must not be read literally.
Two known warts are deliberate:
- An engine-side encode failure of the trim record is reported as
  `EngineError::Record(RecordError::Encode)`, whose sentence says "recorder".
- `Rest.failure` is populated only by the transition that caused the stop. `run.failure` is the
  durable record, which keeps D12's one-way grammar in one place.

### 3 — Work happens in a real tree (`6307a3d`..`1b78753`, 2026-09-22)

22 implementation commits plus a seven-commit review round, over 39 files. The workspace was green
at `--test-threads=1` (1445 passed), and `sqlx prepare --check` was clean.
- **New modules.** `htui-orch` gains `isolate/git.rs`, `isolate/real.rs` (`GixIsolator`, all four
  `R-ORCH-8` modes) and `isolate/copy.rs`. It also gains `verify.rs`: `verify_command` runs through
  the platform shell with three outcomes, a deadline, a tail cap and scrubbed output.
- **The engine.** It calls the verify hook between stages 4 and 5, reconciles the winner into the
  primary tree and cleans up. `Command` gains `CancelRun`.
- **The seam.** It gains `record_command_run`/`command_runs`, and `upsert_step_tree` also writes
  `run_step.isolation_path`. Store `CASES` went from 48 to 49.
- **Criteria.** ANA-2 criteria 11, 12 and 13 are proved over a real repository
  (`tests/gix_isolator.rs`, `pg_criteria.rs`).

**OQ-1 was resolved to the `git` CLI, and that has a cost.** `gix` 0.87.1 ships no worktree
mutation, so `isolate/git.rs` shells out for exactly five verbs: `worktree add --lock`,
`worktree remove`, `merge --no-ff`, `merge --abort` and `reset --hard` (D47). The floor is git
2.33.0, checked once when the isolator is built. `gix` keeps every read and every ref write, and its
sync calls run on the blocking pool. `docs/ANA-2.md:1777` and `:2055` were amended then. **`git` is
therefore a runtime dependency of the `worktree` mode and of reconciliation**, which is one more fact
for MOD-16's Windows verification. **`git worktree prune` is never run** (D46). A pruned admin entry
is instead detected when a tree's HEAD cannot be read, and the tree is re-made onto its existing
branch.

**The review gate found three HIGH, four MEDIUM and five LOW findings.** Every one was verified
against the tree and applied (`892fcf3`, `ff47431`, `085fb79`, `2392031`, `a7e929f`, `70bc276`,
`1b78753`). The behaviours a later item must know:
1. **A no-commit `worktree` tree is kept, not removed, when it holds untracked work.** `is_dirty`
   deliberately ignores untracked files, because the dirty-tree refusals rely on that (D24). So the
   step-end removal also asks `has_untracked_files`.
2. **The `shared_serialized` guard lives from `prepare` to that step's capture.** `cleanup(run, _)`
   drops every guard of the run, whether or not it has `step_trees` rows. A failure between
   `prepare` and `upsert_step_tree` used to leak the guard until restart. Guards are taken in
   `RepoId` order, so milestone 4's siblings cannot deadlock.
3. A failed merge always runs `merge --abort`, even when its conflicted paths cannot be read.
4. A copy never carries a `*.lock` out of the source's `.git`.
5. Lock retries fire only on the exact lock wordings. A message that merely names `Cargo.lock`, or a
   ref-name conflict, does not trigger one.

This milestone carried R-3..R-6 from milestone 2 and added two risks:
- **R-7**: a run parked by a reconcile refusal (`park_run`) had no resume verb. Closed in
  milestone 6.
- **R-8**: `after_hash` becomes the merge commit after reconcile (H-19). Closed in milestone 4.

### 4 — Three candidates, one winner (`fa03782`..`7ee0ac6`, 2026-09-23)

36 implementation commits plus a thirteen-commit review round, over 29 files. The workspace was green
at `--test-threads=1` (1554 passed, 0 failed, 26 ignored), and `sqlx prepare --check` was clean.
- **Selection.** `htui-orch` gains `select.rs` (`R-AGT-8`'s candidate walk, D60) and `fanout.rs`
  (the pure half of selection).
- **The group walk.** The engine drives a fan-out group concurrently and prefilters on
  `verify_command`. It runs the two-order judge as one step with two sessions, and it selects one
  winner in one transaction. `Command::SelectFanout` is the human path, and `RetryStep` retries a
  parked group as a whole.
- **The `Isolator` seam widens for fan-out**, with a slot on `prepare`, `base`, `diff` and a sibling
  list on `reconcile`.
- **git.** `git diff` is the sixth CLI verb. `worktree add`/`remove` are serialised per repository
  (D70), because concurrent adds race on `.git/worktrees/<id>/commondir`.
- **Quota.** `htui-core`'s quota predicate treats `allowed_warning` as selectable (D61).
- **Criteria.** ANA-2 criteria 7 (the real-tree half, via R-8), 8, 9 and 10 are proved.
  `htui-orch`'s case list is 36. **R-8 is closed with no predicate change** (D66).

**The review gate found one HIGH, three MEDIUM and six LOW findings.** Every one was verified against
the tree and applied (`20cb2fa`, `8438406`, `86058a9`, `a40b831`, `ad60554`, `375123b`, `87ec1d4`,
plus the doc repairs `a19e633`, `ffe86c0`, `1fa37e2`, `7ee0ac6`). The behaviours a later item must
know:
1. **A candidate's deadline and verify budget start when its `prepare` answers**, not at
   `run_step.started_at`. In a `shared_serialized` group, later siblings were being charged for the
   earlier ones' whole sessions.
2. **A retried group starts from the base its retired attempt recorded** (`before_hash`), not from
   HEAD. In `shared_serialized`, HEAD is the last loser's commit. A retired slot that names two
   bases, or none for a repo in scope, is refused with `EngineError::GroupBase`. An attempt with no
   rows at all is skipped. A slot with a selected winner (the review loop) still starts from
   `Isolator::base`.
3. A store error after the judge went `running` now fails the judge and parks the run, instead of
   leaving both `running`.
4. **`resume` keeps walking the snapshot when the live graph no longer resolves** under lowered caps
   (`FanOutCap`, `AgentCap`, `ReviewFanOut`, `NoCandidate`), and it writes a note (invariant 2).
5. `drive_group` was not cancel-safe. This was closed by milestone 5's D86, D95 and D99, and
   milestone 6 relies on a preempting drop being the intended abandon.
6. A stale-member `RetryStep` is `EngineError::StaleSlot`.

**The plan's "no migration" held until the review gate, and then deliberately did not.** The
maintainer raised `max_agents_per_run`'s default from 6 to 8 (OQ-1 revisited). The literal reading
refused the seeded `feature` graph, whose judged 3-way `implement` needs 7 agents.
`0004_max_agents_per_run_default.sql` moves an untouched seeded `6` to `8` and leaves any other value
alone, because `0003` is on `main` and cannot be edited. `graph.rs`'s fallback follows (`b86b62c`,
`bcce4b9`). A judged 4-way `implement` (8 agents) sits exactly at the cap. The stale `6`s in
`docs/ANA-2.md` and `docs/decisions/ana/ana-2.md` were amended at this close-out.

This milestone added **R-9**: `LoopStop::NoProgressReview` has been unreachable since milestone 2
(fan-out blueprint F-B; the plan's risk row was corrected in `e2986f2`). It is now CLEAN-4. Every
production judge fails until MOD-11 gives an agent a way to write the `judge` document (OQ-4), so
production fan-outs go to the human.
**OQ-7 follow-up.** Milestone 4 runs every fan-out candidate on the one agent the walk selects.
Spreading candidates across agents by weight is MOD-36's, with weights from ANA-21.
`AgentSelector::select` is called once per candidate with its `fanout_index`, so MOD-36 plugs in
there.

### 5 — Two runs do not collide, and a crash is survivable (`a1fb291`..`8dc4755`, 2026-09-23; follow-up `0ea3744`..`1d598f1`, 2026-09-24)

Eight tasks (T1–T8), then two review rounds. The workspace was green at `--test-threads=1` (1700
passed, 0 failed, 26 ignored), and `sqlx prepare --check` was clean (227 files). Store conformance
has 53 cases and the `htui-orch` case list 52.
- **New modules.** `htui-core` gains `model/overlap.rs`, the scope predicate (D80). `htui-orch`
  gains `overlap.rs`, which resolves scope at `StartRun`, and `recover.rs`, the heartbeat and the
  sweep's adjudication.
- **R-2 resolved.** `claim_run` answers a `Claim`. It stores the scope (D79) and evaluates rules L,
  I and P in both stores' critical section (D80). New store verbs are `take_lease`,
  `interrupt_step` and `release_lease`.
- **The lease.** Every walk runs under a leased heartbeat that fences itself before the lease lapses
  (D122, D143). Every compare-and-set on a walk path honours `Ok(false)` as
  `EngineError::StaleWrite` (D125, D144). The exemptions are named on `Engine::move_step`.
- **The sweep.** It adopts expired runs one at a time. A process takes back its own dead walks
  through an in-process `DeadWalks` set (D139, D140).
- **Reconcile.** `reconcile_isolated` merges onto a primary another run moved, under the repo's
  admin lock (D136).
- No migration.

**Round 1 (D122–D138) fixed four HIGH, four MEDIUM and several LOW findings. Round 2 (D139–D145)
fixed one HIGH, two MEDIUM and three LOW** (lease blueprint §21–§22). The final reviewer approved with
fixes. Two LOWs were applied (`ced0808`, `8dc4755`) and three MEDIUMs were carried as R-33..R-35.
About six intermediate commits fail `clippy -D warnings` on their own; the head of the milestone is
clean.

**The follow-up (branch `mod-4-m5-fixups`, lease blueprint §23, D146–D152) closed R-33, R-34 and
R-35.**
- `git::reconcile_parent` accepts a reconcile merge only on the primary's first-parent line, only
  for `worktree`/`copy` rows, and only by subject. `merge_no_ff` runs with `merge.log=false`.
- Every command's window between its lease take and its walk gives the lease back on error
  (`Engine::leased_window`), as does a topology-mismatch resume.
- A dead walk whose run is gone leaves `DeadWalks`. It is told apart from an outage by a
  `test-support` `MemFault` hook on `MemStore`.

The pins were unchanged (store 53, orch 52, `.sqlx` 227). The final review approved with fixes, and
all were applied except L7, which was carried as R-37 with R-36. `0e61f08` alone fails clippy.

Carried into milestone 6: R-3..R-7, R-9, R-10, R-12, R-25..R-32, R-36, R-37. R-11 and R-13..R-20
are recorded in the lease plan as accepted design limits and were not carried further. R-21 was
closed by lease blueprint A-8, and R-22..R-24 were not incurred (A-1, A-4, A-5).

### 6 — The maintainer drives it (`8b2e39f`..`22cfeea`, 2026-09-24..25)

104 commits on branch `mod-4-m6`, cut at `ba68682` after MOD-30 landed (PRD D5). The plan's
decisions are D153–D180 and its risks R-38–R-47. The blueprint added findings F-A..F-S and decisions
D181–D209, and the review round added D210–D216. The blueprint's findings changed the plan in
several load-bearing ways. The step grid did not fit 43 columns as planned (F-A, fixed by D197). The
pane could not call the engine's guards, because it holds summaries, not rows (F-B), so D182 added
`StoreRequest::RunActions` and D184 one admission function per verb. A preempting drop released
neither the guards nor the lease (F-D, fixed by D188's `Engine::abandoned`). The sweep could not be
fenced per run (F-E, fixed by D189's `RunFence` and `sweep_fenced`). A promotion could not wait
for a run lock on the store loop (F-F, fixed by D181 and D191, which make it a task). The ACP driver
never reads `SessionSpec.resume` (F-G, which D192 routes around and which is carried as R-48).

**By task.**
- **T1** (`9a42c25`, `c49e4ac`): the item-law edge `blocked → awaiting_approval` (D161). An escalated
  item can follow its parked run back, so the Runs tab's promote, approve and cancel reach the run.
  `SANCTIONED` pins it as the one deviation from ANA-2 §4.3.
- **T2** (`a003289`, `78e2e86`): `Recorder::continuing` (D164) continues a step's log. It starts at
  the last row's `seq + 1` and turn, never writes `prompt_digest`, and seeds `usage` from the
  persisted rows so the step's pre-promotion spend is not overwritten.
- **T3** (`042de86`, `17f8498`): the pure modules. `closeout.rs` builds the summary document (D167,
  D208), and `promote.rs` chooses the opening and builds the handoff `PromptSpec` (D163, D192).
- **T5** (`62777f4`, `cc51267`): the primary-changing `git` verbs (`merge --no-ff`, `merge --abort`,
  `reset --hard`) run on a detached task and are awaited through the handle (D160). A dropped walk
  no longer SIGKILLs a merge mid-write (R-26).
- **T4** (`cf35ebf`..`c8abf91`): the four commands, `PromoteStep`, `AcceptArtifact`, `Unblock` and
  `CloseOut`. It adds one admission function per verb (D184), called by the engine and by
  `RunActions`, and factors out `Engine::enqueue`, `Engine::abandoned` and `sweep_fenced`/`RunFence`
  (D186, D188, D189). Also:
  - `DeadWalks::mark` (D158).
  - `resume` parks a mismatched `running` run (D159, criterion 3, R-36).
  - `cancel_run` takes the lease (D179, R-28).
  - `walk_resumed` honours `unpark`'s answer (D180).
  - ANA-5 criterion 3 blocks the item at both call sites (D162, D195).
  - Its repairs: `promote` and `accept` refuse an attempt a retry replaced, with `StaleSlot`
    (`b6cb08f`). A fenced sweep's skipped run joins `DeadWalks` (`f6f33b0`). Accept's verify window
    runs under the heartbeat (`c8abf91`).
  - Orch `CASES` went from 52 to 68.
- **T6** (`df2a11e`..`d7e1069`): `crates/htui/src/run_worker.rs`.
  - `RunRuntime` lives inside the store worker's `select!` beside `AgentRuntime` (D153).
    `BackendGraphs` is the `GraphSource` newtype (D155), and the production isolator and verifier
    are singletons per process and server (D156, D202).
  - `RunLocks` and per-run cancellation tokens preempt a live walk (D157, D187). The sweep ticks at
    `lease_ttl_seconds` and supervises its tasks, and a refused claim is retried (D158, D190). The
    `Publisher` sends `RunStream` frames at the subscription's `seq` (D172, D200).
  - `StoreRequest` grew from 58 to 62 variants (`Orch`, `RunStream`, `RunActions`, `Document`).
    `name()` maps `Orch` to eleven per-verb names (D209). A runtime-less `try_serve` refuses only
    `Orch` (D183), and offline every `Orch` request is refused with MOD-25's sentence (D174).
  - Its repairs: settle and shutdown abort the walk, and shutdown closes the runtime and shares one
    window with the chats (`cc34b75`, `348b5ea`). Every command publishes a frame, which added
    `FrameKind::Changed` (`8548711`). A server switch preempts this process's walks and calls
    `forget_server` (`d7e1069`). The claim-retry and promotion-reply fixes are `9acffa2`, `394dd4e`,
    `ae89a81` and `0f88da8`.
- **T7** (`f60be55`..`b527731`): the promoted chat. `ChatBinding::Promoted` (D205) and
  `AgentRuntime::attach_promoted` (D165) bind a Chat-tab session to the graph step. Nothing mints
  or closes a run, and the opening is a `follow_up` at `turn + 1` (criterion 17, ANA-5 criterion
  18). `live_steps` (D206) feeds the live-chat guard. **The `ChatFollow` repair** (`9f7cc5c`): a
  promoted chat streamed at the promotion's `Orch` address, which any later `Orch` request from the
  Chat tab superseded. The tab now sends `StoreRequest::ChatFollow` on `ChatAccepted`, and the
  stream moves there. A promotion refused at the bind is handed the live chat's stream. The
  interval between `ChatAccepted` and the follow being served is not covered; it is carried in
  MOD-37.
- **T8** (`d7ce9d5`..`5a46985`): the Runs pane.
  - Twelve keys: `a` approve, `x` reject with a note, `r` retry, `p` promote, `c` cancel, `o` open
    artifact, `s` select a fan-out winner, `u` unblock, `A` accept artifact, `R` run, `C` close out
    and `T` retry cleanup (D168). Each is greyed by the worker's verdict with the guard's sentence.
  - Each step takes two lines on a 43-column grid (D169, D170, D197), and `awaiting_approval`
    renders `awaiting`.
  - The note, cancel and close-out modes take the keyboard through the one generic `captures_input`
    seam in the Backlog tab (OQ-7, D201). Close-out asks for the item key typed back (D167).
  - The artifact view is read-only inside the pane (D173). The cursor survives a re-read (D198).
- **T9** (`a973a37`, `0ba94fc`): `crates/htui/tests/runs_pg.rs` proves criteria 17 and 20 and the
  new `blocked → awaiting_approval` edge on Postgres through the worker.

**What a later item must know from this milestone.**
- **Production `approve` and `accept` are greyed until MOD-11** (R-50, F-R). Production's
  `SessionSink` is `NoSink`, so no step writes its `output_kind` document outside a test. Tests
  reach those paths through D203's `StepAuthor`.
- **Promotion reaches the Chat tab, one session per process** (D185). A `running` step is promoted
  by preempting its walk, with no grace window (R-38). Resume is used only for CLI rows with a
  session banner, and ACP rows always get the handoff prompt (D192, R-48).
- **`Unblock` has three cases** (D161, blueprint §13.2): `Reopen` for a blocked item with no run,
  `FollowRun` for an escalation, and `Resume` for a reconcile-refused or crash-left park (R-4, R-7).
- **No secrets are wired** (D176). `drive_once` builds every `SessionSpec` with an empty `env`, and
  its comment names MOD-10.
- **`CancelStep` and an `OpenArtifact` command were not built** (D178, D173).
  `GateAnswer::Skipped` stays unexposed (D166).
- `crates/htui-orch/src/graph.rs:45` still says `htui` implements `GraphSource` for `Backend`, which
  the orphan rule forbids. The implementation is `run_worker::BackendGraphs`. This stale doc comment
  sits outside every task's file set (blueprint C-3).

**Review round.** `rust-reviewer` over `main..001ddef` returned request-changes with seven
findings, three of them HIGH. One adversarial verifier per finding reproduced findings 1–5 and 7;
finding 6 is not a defect as designed. The maintainer took the recommended option on each
(blueprint §21):
- **H1, D210** (`223995e`, `073432a`): a panicked walk's `shared_serialized` guard was never released,
  so its adopted run, and any other run on that repository, waited forever. The sweep's dead-walk
  pre-pass and `renew_lease` now release the run's guards.
- **H2, D211** (`869d96e`, `e14941a`): accept's verify measured the phase deadline from the step's
  `started_at`, so a late accept merged unverified. The deadline is now measured from the accept,
  and an unavailable verify is noted.
- **H3, D212** (`91a0865`, `567e4e9`): approve, reject, retry, select and cancel went through on a run
  whose promoted step was still being chatted with. They are now refused with `ChatLive` and greyed
  in the pane.
- **L4, D213** (`59b1734`): `GixIsolator::new` runs on a blocking thread.
- **L5, D214** (`9233fdb`, `1c92c2e`): each sweep tick prunes finished tasks, idle run parents and free
  run locks.
- **Finding 6, D215** (`6c066f4`, `8c0c1e4`): `RunActions` is served from a tracked task. This was
  not a defect; the maintainer chose the change.
- **L7, D216** (`1489e2d`, `9bdf218`): a failed `command_limits` read refuses the build instead of
  caching a default. This added R-55.

The verification of the round found three more issues, all repaired (blueprint §21.5):
- **V1** (`669f4ee`, `440ab8d`): the unavailable note carried the command's tail. It now carries
  only the reason line.
- **V2** (`68d24fe`, `a3d15a0`): the pane kept greying the run's verbs after the chat ended. A
  promoted chat's end now publishes `Changed`.
- **V3** (`2b13b3d`): a test description overstated what the verdict unit proves.

**Gate at `22cfeea`.**
- Workspace tests at `--test-threads=1` against Postgres: 1847 passed, 0 failed, 26 ignored.
- `clippy --workspace --all-features --all-targets -D warnings` and `fmt --check`: clean.
- `cargo doc --workspace --no-deps`: only the two baseline errors (`htui-core` `MIRRORED_TABLES`,
  `htui-store` `step_exists`).
- `cargo insta`: nothing pending.
- `cargo sqlx prepare --check` on a migrated scratch database: clean, 227 `.sqlx` files.
- Pins: store conformance 53, `READ_CASES` 9, orch `CASES` 70, `StoreRequest` 62 variants.
- No migration, and no new `WriteStore` or `ReadStore` method.

Known flake, not caused by this milestone: `htui-store` `tests/cache.rs`'s
`the_spawned_refresher_passes_and_follows_the_scope` timed out once under dev-Postgres load and
passes alone. Under load the dev Postgres also goes into recovery mode (`57P03`). Eight failures of
the review-round gate at `9bdf218` were that, and each passed when re-run alone.
`223995e` fails `cargo fmt --check` on its own; `073432a` fixes it.

---

## Deviations from ANA-2 and the PRD

These are recorded in `docs/ANA-2.md` itself as dated amendments (MOD-4 milestone 6, 2026-09-25)
where ANA-2 states the opposite. Drive plan R-42 and R-47 list them, and blueprint §19 adds the rest.
- **OQ-3/OQ-4, D161**: the item law gains `blocked → awaiting_approval` (§4.3 verdict 3).
- **OQ-5, D163**: promoting a `running` step preempts its walk without the grace window and without
  answering parked permission requests (§4.8, R-38).
- **OQ-9, D173**: open artifact is a read-only view in the Runs pane, not the Documents sub-tab, and
  it is a read, not an `OpenArtifact` command (§6.2).
- **OQ-11, D175**: criterion 19 is re-scoped for online-only. A store outage fences the walk, and
  the next sweep adopts it. Nothing is buffered.
- **D162**: ANA-5 criterion 3 is generalised. Every stage-3 prompt refusal blocks the item, not only
  an unknown placeholder.
- **D166**: `GateAnswer::Skipped` is unexposed, and accept artifact requires the document and lands
  `approved`. D194 adds that accept is refused on a failed verify, where ANA-2 is silent.
- **D178**: `CancelStep` is not built.
- **F-G, R-48**: ACP resume is not used; ANA-2 `:1199-1206` presumes it.
- **F-R, R-50**: criterion 21's actions are reachable in production, but `approve` and `accept` are
  greyed until MOD-11.
- **M4, `0004`**: the `max_agents_per_run` default is 8, not 6.
- Earlier milestones' deviations are recorded in their plans: M2 D10 on criterion 16, M3 D47 on the
  `git` CLI (amended in ANA-2 then), and M5's R-18 list.

---

## Carried

Where every risk MOD-4 did not close now lives.

| Risk | Owner | What |
|---|---|---|
| R-9 | **CLEAN-4** | `NoProgressReview` is unreachable (fan-out blueprint F-B). |
| R-3, R-5, R-29, R-30, R-31 (the rejected-crash remainder), R-32, R-37, R-38, R-40, R-41, R-44, R-46, R-48, R-49, R-51, R-53, R-55, and T7's `ChatAccepted`/`ChatFollow` window | **MOD-37** | Each has a bullet there with its source. |
| R-50 | **MOD-11** | Production `approve`/`accept` are greyed until an agent can write its `output_kind` document. Every production judge fails until the same tool exists (M4 OQ-4). |
| R-10 | **MOD-16** | A SIGKILLed orchestrator's agent survives. The child leads its own process group, and `ChildGuard::drop` cannot run in a killed process. Signalling a stale pid needs `unsafe` or a dependency. |
| R-45 | **MOD-16** | `htui` now links `gix`, `process-wrap` and `walkdir` through `htui-orch`. |
| R-6 | **MOD-15's phase editor** (the plans' owner) | Nothing writes `phase_agent` for an override graph copy, or `is_override`. **MOD-15 closed 2026-09-17**, so no open item holds this today. The next item that edits override graphs inherits it. |
| R-43 | **MOD-13** | The close-out summary is generated, with no human prose. An editable summary is MOD-13's editor's. |
| R-39 | accepted | One isolator per process: a repo added while a walk is live is refused by name until every walk rests (D202). |
| R-11, R-13..R-20 | accepted | Recorded in the lease plan as design limits of milestone 5; not carried. |

Closed by this item: R-1, R-2, R-4, R-7, R-8, R-12, R-21..R-28, R-31 (except its remainder), R-33..R-36,
R-42 (by the ANA-2 amendment), R-47 (by the amendments above), R-52 (D204) and R-54 (T7's
`live_steps`).
