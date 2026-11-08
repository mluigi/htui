# MOD-138 — Snapshot v2 and the graph walker

> Routed as **PRD** by `/handoff-run` in sandbox run `hr/MOD-138` (criteria C2 and C4 fired, exactly at
> the threshold; flagged low-confidence, maintainer accepted 2026-10-27). Ultracode accepted for the
> implement phase and for the review verify pass. Origin: ANA-33 (`docs/ANA-33.md` §7.2, §7.3, §7.7,
> §10; decisions accepted 2026-10-18). Requirements: `R-ORCH-1`, `R-ORCH-2`, `R-ORCH-3`, `R-ORCH-4`
> (seam), `R-ORCH-11`. PRD gate: maintainer accepted the recommended answers to Q1-Q4, 2026-10-27.

## Problem

Step graphs now store a topology of their own (MOD-137: edges, owner scope, `revision`), but nothing
reads it at run time. A run still freezes a v1 snapshot, a flat list of phases, and walks it strictly
in position order; the only loop is the review rule keyed on the phase *name*; and a resumed v1 run
compares itself against the live graph and parks when someone edited it. So the graphs `R-ORCH-1` asks
for (branch on a condition, run phases in parallel, loop to a declared target, wait for the person on
a link) cannot run, `R-ORCH-3`'s "loop back to the phase it declares" is still a name rule, and
MOD-139 (divided fan-out), MOD-141 (the editor's validator) and MOD-143 (subgraphs) are all blocked on
this item.

## Evidence

Read at `8a4a88c6` (branch `hr/MOD-138`). Paths are relative to the repo root.

- **Maintainer design decision, not an incident.** `R-ORCH-1` was amended in the 2026-10-15
  requirements review to allow branching, parallel phases and loops (`docs/REQUIREMENTS.md:500-517`),
  `R-ORCH-3` to a per-phase loop-back target (`:523-525`). ANA-33 settled the design and the
  maintainer accepted all ten §8 decisions on 2026-10-18. Real use of branching graphs is an
  **assumption — needs validation once MOD-141's editor can author them** (Q3 below).
- **The snapshot is v1 only.** `GraphSnapshot { v, graph, topology, mode, phases, settings, scope,
  personas }` with `GraphSnapshot::V = 1`, "the only one it reads"
  (`crates/htui-core/src/model/run.rs:516-563`). There are no nodes, edges or bounds in it.
- **The walk is a position scan.** `status::cursor` walks `snapshot.phases` in order and "the first
  position whose latest step is not `done` decides" (`crates/htui-orch/src/status.rs:302-352`). One
  cursor at a time; no joins, no skips.
- **The loop is a name rule.** `is_review: phase.name == REVIEW_PHASE`
  (`crates/htui-orch/src/engine.rs:3793`, `:4586`), read by `gate::settle`
  (`crates/htui-orch/src/gate.rs:277`).
- **v1 resume compares against the live graph.** `resume_window` returns `Resume::TopologyChanged`
  on a mismatch (`crates/htui-orch/src/engine.rs:3240`), pinned by conformance cases
  (`crates/htui-orch/src/conformance.rs:1757`, `:5627`, `:7283`). ANA-33 decision 4 keeps that for
  v1 runs and drops it for v2.
- **Recovery assumes a line.** `recover::frontier` takes the done winner at the highest position and
  requires every later row to be retired (`crates/htui-orch/src/recover.rs:355-376`). Two nodes
  done in parallel break that assumption. ANA-33 §7.7 leaves to this item's plan whether a
  `reconciled_at` column makes the frontier explicit.
- **None of the new bounds exist.** No `max_parallel_sessions`, `max_plan_revisions`,
  `runtime_changes` or `max_child_runs` anywhere under `crates/`. Today's caps are `app_setting`
  key/value rows (`crates/htui-store/migrations/0003_orchestration.sql:133`) frozen into
  `SnapshotSettings` (`run.rs:676-683`) and checked by `check_fan_out_caps`
  (`crates/htui-orch/src/graph.rs:855`).
- **Storage is ready.** MOD-137 stored edges (`step_graph_edge`, kinds `next`, `if`, `asks_you`,
  `changes_requested`, `fails`, with `predicate` and `loop_limit`), `split`, `palette`, `plan_scope`,
  owner scope and `revision`, and backfilled today's topology as data (`docs/decisions/mod/mod-137.md`).
  `seed::default_edges` is the rule the v1 lift reuses. `replace_graph` is to call the shared validator
  once it exists (`crates/htui/src/catalogue.rs:316-324`). MOD-137 D4 owes item and kind references to
  workspace and library graphs (`check_graph`, `crates/htui-core/src/model/item_spec.rs:321`).
- **The stack is tight.** Engine walks sat at the 2 MiB debug stack until MOD-10 M3 boxed the
  dispatch arms. Driving several nodes at once multiplies the futures in flight (ANA-33 §9).

## Users

- **Primary**: the maintainer, authoring kinds' graphs and library workflows that branch, run phases
  in parallel and loop to a declared target, and running items through them on the TUI or a worker
  box. Indirectly, the follow-on items that build on this walker: MOD-139, MOD-140, MOD-141, MOD-142,
  MOD-143.
- **Not for**: authoring graphs by hand (MOD-141's editor); divided fan-out, planners and parts
  (MOD-139, MOD-140); subgraph child runs (MOD-143); the run display of a non-linear graph
  (MOD-142, MOD-120).

## Hypothesis

We believe **a pinned v2 snapshot with nodes, edges and bounds, walked by a readiness set instead of
a position scan**, will **let graphs branch, run in parallel, ask the person on a link and loop to a
declared target, while every linear graph and every in-flight v1 run behaves exactly as today**.
We'll know we're right when **every existing orchestration conformance case passes unchanged through
the v1 lift, and new cases prove joins, derived skips, each `if` predicate, `asks_you` take and
decline, loop retirement with limits and the no-progress escalation, and concurrent nodes bounded by
`max_parallel_sessions`, on both MemStore and PgStore**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Linear graphs unchanged | every existing `htui-orch` conformance case (109) and engine test passes with no expectation edited, except the ones the v1/v2 resume split must change | gate run, MemStore and PgStore |
| v1 runs unchanged | a run created with a v1 snapshot resumes, parks on a live-topology change and finishes as today | conformance cases on v1 fixtures |
| v2 runs are pinned | editing the live graph never parks a v2 run; a tampered snapshot or plan digest is refused at resume | conformance cases |
| Branches resolve | joins wait for every forward in-edge, all-not-taken in-edges skip and propagate, each `if` predicate (`all_changed_paths_match`, `any_changed_path_matches`, `verify`, `item_kind`, `item_tag`, `all`/`any`/`not`) decides as specified | pure `ready()` unit tests plus conformance cases |
| Loops follow edges | a `changes_requested` or `fails` edge loops to its target, retires the region, stops at `loop_limit` and escalates on no progress; `is_review`'s name rule is gone | conformance cases; grep for `REVIEW_PHASE` in the walk |
| `asks_you` works | the run parks `awaiting_approval` on the edge's source step; approve takes (or loops), reject skips; never downgraded by auto mode | conformance cases |
| Concurrency is bounded | two ready nodes run at once, never more than `max_parallel_sessions`; landings into one repo are serialised | engine tests with the fake driver; a lock-order test |
| One validator | the same pure function refuses a bad graph at save (`replace_graph`) and at resolve | unit tests over every refusal; both call sites |
| No stack overflow | the full suite passes with no SIGABRT | gate `--no-fail-fast`, grep for SIGABRT |

## Scope

**MVP**: `GraphSnapshot` v2 (nodes, edges, bounds, `topology` over canonical nodes and edges) and its
resolve, with workspace and library graphs usable by items and kinds; the v1 lift in `snapshot_of`;
pinned resume for v2; the shared pure graph validator, called by `replace_graph` and by resolve; the
readiness walker `ready()` replacing `status::cursor`, with all-joins, derived skip propagation, the
closed `if` set, `asks_you` parks, per-edge loop targets with region retirement, limits and the
no-progress test, and graph-aware `required_inputs`; concurrent driving under
`max_parallel_sessions` with every per-node future boxed, a per-repo reconcile lock and a recovery
frontier that handles concurrent landings.

Decisions taken at the PRD gate (maintainer, 2026-10-27):

- **Q1 — divided and subgraph nodes.** The v2 snapshot carries the divide expansion (`<name>:plan`,
  role `planner`, and `<name>`, role `integrate`, joined by a synthetic edge) and the validator checks
  it, but resolve **refuses** to start a run whose graph has a `divide` or `subgraph` node, with a
  named error, until MOD-139 (and MOD-143) land. No half-working runs.
- **Q2 — answering `asks_you`.** The run parks `awaiting_approval` on the edge's source step and the
  existing gate answer means take or decline: approve takes (creates the target, or loops), reject
  declines (skips the target). No new keys or commands; MOD-142 and MOD-120 refine the display.
- **Q3 — validation.** By tests only: conformance and engine tests over MemStore and PgStore. Real use
  of branching graphs waits for MOD-141's editor; linear graphs must keep running unchanged in the app.
- **Q4 — milestones.** Three, as below.

**Out of scope**

- Divided fan-out runtime: planner sessions, parts, integrate, `run_plan`, `run_step.role` and friends.
  **Why:** MOD-139 (and MOD-140 for the planner's tools). MOD-138 freezes the expansion and refuses it.
- Subgraph child runs. **Why:** MOD-143, after ANA-35.
- Run-time changes (`successors` plans) and the `runtime_changes` bound's behaviour. **Why:** MOD-140.
  The bound may be frozen into `bounds` here if the plan finds that cheaper than adding it later.
- The graph editor and any way to author a branching graph from the TUI or CLI. **Why:** MOD-141 (Q3).
- The Runs pane and run graph for a non-linear run. **Why:** MOD-142, MOD-120.
- A box-level session cap across runs. **Why:** ANA-33 §9 leaves it open for MOD-139's plan.
- Re-run verbs ("re-run on this version" / "on the latest"). **Why:** ANA-33 §7.2 names them; no item
  owns them yet. TBD — needs a HANDOFF item if wanted.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Snapshot v2, the validator and the lift | New runs freeze a v2 snapshot (nodes, edges, bounds; divide expansion frozen, divide and subgraph refused) and are pinned to it; v1 runs read through the lift and behave as today; one validator refuses bad graphs at save and at resolve; items and kinds may use workspace and library graphs. The walk itself still runs one node at a time in order | complete (2026-11-03, `d114f91f`; record `docs/decisions/mod/mod-138.md`) | `.claude/plans/mod-138-m1-snapshot-v2.plan.md` |
| 2 | The readiness walker | `ready()` replaces `cursor`: joins, derived skips, `if` predicates, `asks_you` parks answered through the gate, per-edge loops with region retirement, limits and no-progress, graph-aware `required_inputs`; the `is_review` name rule is gone. Driven one ready node at a time | complete (2026-11-05, `ac4e2cd5`; record `docs/decisions/mod/mod-138.md`) | `.claude/plans/mod-138-m2-readiness-walker.plan.md` |
| 3 | Concurrent nodes | Every ready node runs at once up to `max_parallel_sessions`, each per-node future boxed; landings into one repo serialised by a reconcile lock; `local` isolation on two nodes that can be live together refused at resolve; crash recovery finds the frontier among concurrent landings (owed from M2: graph-aware frontier, `last_output_kind`, mixed `asks_you` edges) | complete (2026-11-08, `055ea1d7`; record `docs/decisions/mod/mod-138.md`) | `.claude/plans/mod-138-m3-concurrent-nodes.plan.md` |

## Open Questions

- [x] Recovery frontier: is an explicit `run_step.reconciled_at` (migration `0024`, and a cache mirror
  if the column must be read offline) needed, or can the frontier be derived from existing rows once
  nodes land concurrently? ANA-33 §7.7 leaves this to the plan. **Settled in M3 (Q-1, maintainer
  2026-11-07):** derived, no column: a done winner is on the frontier when no forward successor has a
  non-retired row (the walk creates a successor only after its predecessor landed); re-landing is
  idempotent (H-3).
- [x] Where the new bounds live: `max_parallel_sessions` (default 4) and the other v2 bounds as
  `app_setting` rows like `max_agents_per_run` (a seed or migration), or code defaults until a
  Settings surface exists. **Settled in M1 (D13):** read from `app_setting` like `max_fan_out`, code
  defaults, no rows seeded; MOD-123's Settings surface may seed them.
- [x] `if` predicate inputs: where "the source step's changed paths" are read from (its tree's
  `base_ref`..`after_hash` diff per repo, or a recorded diffstat), and how a source with no tree
  (a check, a judge) answers path predicates. **Settled in M2 (Q-A, maintainer 2026-11-05):**
  `Isolator::changed_paths` over the winner's recorded ranges, decided once and stored on
  `run_step.branch` (migration `0025`, cache `0008`); no changed paths makes both path predicates
  false.
- [x] Gate interplay on `asks_you` (Q2): a source step that has its own `always` gate *and* an
  outgoing `asks_you` edge — one park or two in sequence. **Settled in M2 (Q-B):** one park, never
  downgraded; approve takes, reject or accept-artifact declines with the step `done` (`skipped`) and
  its work landed.
- [x] Which v1-pinned conformance expectations must change at the v1/v2 resume split (the
  `TopologyChanged` cases), and whether they are kept on v1 fixtures instead. **Settled in M1:** eight tests
  moved to v1 fixtures (`fake::rewrite_as_v1`), each with a v2 twin.
- [ ] The ANA-2 and `R-ORCH-1` text edits ANA-33 §10 lists for the graph-model phase (invariant 2's v2
  pinning; `retries` as a self-loop). Invariant 2 done at M1 close-out (`docs/ANA-2.md`). The `R-ORCH-1`
  `retries` wording is a requirements edit, the maintainer's to apply (still open).

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The walker swap regresses a linear behaviour the position scan gave for free (lazy re-insertion after `superseded`, compete slot reading, resumable parks) | High | Wide engine regression | M1 keeps the walk; M2 swaps it behind the full existing conformance suite, expectations unedited; `ready()` is pure and unit-tested first |
| Concurrent nodes overflow the 2 MiB debug stack | Medium | SIGABRT in tests | Box every per-node future; gate `--no-fail-fast` and grep SIGABRT (memory: stack headroom) |
| Concurrent landings race §4.6's primary-tree-clean check | Medium | A landing over a dirty tree | Per-repo reconcile lock with the clean check under it; a lock-order test |
| A v2 snapshot breaks offline readers (`step_graph` is not mirrored; the snapshot is the only offline route to a graph) | Medium | The Runs pane cannot read new runs | Keep v1-shaped fields readable or adapt every decoder in the same milestone; offline cache tests |
| An older binary meets a v2 snapshot | Low | Refused run | Already refused by `v`; the migration bump gates headless workers (ANA-33 §7.2) |
| Suite flakiness hides a real failure | Medium | False green or red | Verify gates with `--test-threads=1` (memory: suite green is scheduling-dependent) |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
