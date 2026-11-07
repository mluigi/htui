# Plan: MOD-138 M3 — Concurrent nodes

**Source PRD**: `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md`
**Selected Milestone**: 3 — Concurrent nodes
**Requirements**: `R-ORCH-1`, `R-ORCH-4`, `R-ORCH-11`.
**Design**: `docs/ANA-33.md` §7.3 ("Parallel nodes reconcile on their own"), §7.7 (frontier), §7.8, §9 (stack depth,
parallel landings); M1/M2 record `docs/decisions/mod/mod-138-decisions.md`; M2 plan's owed items (A11, A12, Q-C, D9 gap).
**Complexity**: Large. The engine's driving loop changes from one node at a time to a set of in-flight nodes. No
migration (Q-1). Linear graphs must walk exactly as before.
**Routing**: PRD path (continuing), ultracode for implement and review (maintainer, 2026-11-07).
**Status**: CONFIRMED 2026-11-07 (maintainer). Fact-checked: 23 claims, 8 amended.
Blueprint `.claude/plans/mod-138-m3-concurrent-nodes.blueprint.md` (findings F1-F23). Blueprint amendments **A1-A12
accepted by the maintainer 2026-11-07**, binding over the text below; under A3, D7 (`local` on concurrent nodes
refused at resolve) is kept. In short: A1 nodes keyed by position, owned `drive_node` futures; A2 a node is admitted
only when incomparable with every in-flight node; A3 an in-place node's session is exclusive against every
tree-touching future; A4 `cleanup_run` deferred while siblings are in flight; A5 a walk-scoped cut channel, the
scheduler writes the `cancelled` rows; A6 strict park and `status::run_resumable`; A7 `recover_run` adjudicates every
running row; A8 a successor proves a landing only if it started at or after the winner finished; A9 the in-place
frontier rule over every mode; A10 an answer lands the other frontier members first; A11 a drained walk returns the
first park's rest, an ending rest wins; A12 task file sets per blueprint §7.

## Summary

`run_to_rest` stops taking `ready()[0]` and instead keeps a set of in-flight node futures. Every ready node
runs at once, each future boxed, up to the run's frozen `max_parallel_sessions`. A node whose future is still in flight
counts as not done, so a successor (a join in particular) is created only after its predecessor has landed. Landings
into one repository already take turns under `GixIsolator`'s per-repository admin lock (D70/D136); M3 proves that
and adds the one case the lock does not cover (`shared_serialized`, whose guard is released at capture). Resolve
refuses `local` isolation on two nodes that can be live together. Recovery finds every done node that may not have
landed, from the graph alone. The three items M2 owed close: the graph-aware frontier, `last_output_kind`, and
mixed `asks_you` edges (made a permanent validator refusal). The join-behind-a-declined-`if` input gap closes too.

## Maintainer decisions (2026-11-07, before drafting)

- **Q-1, recovery frontier: derived, no column.** Settles the PRD open question. A done winner is on the frontier
  when no forward successor of its node has a non-retired row. The walk creates a successor only after its
  predecessor's future returned, which is after the predecessor landed (D2), so a successor row proves the landing.
  Re-landing a node that already landed is idempotent: `reconcile_isolated` finds the existing merge (H-3,
  `merge_of`), which today's position rule already relies on for the highest done winner. No migration, no store
  verb, no cache mirror.
- **Q-2, a park while siblings are live: drain on park, cut on end.**
  - **Park** (a gate stop, an `asks_you` stop, a reconcile conflict, an undecided branch): the walk admits nothing
    new; every in-flight sibling runs on through its session, settle, gate and landing, and may park on its own
    gate. The walk returns once nothing is in flight.
  - **End** (any node's outcome finishes the run: retries spent, `no_loop_target`, a refused prompt, a loop
    exhausted that ends the run): every in-flight sibling is cut through the same signal a run cancel sends; each
    cut session settles and its row ends `cancelled` with a note naming the node that ended the run. Then
    `cleanup_run`.
  - **An answer during a drain is taken only after the drain ends.** Amended at fact-check (V13, V14): the
    worker already makes the answer wait on the run's lock and shows it as waiting, so no new refusal is added
    (D3).
- **Q-3, mixed `asks_you` edges: refused permanently, at save too.** M2's resolve-only `NotRunnable::MixedAsksYou`
  becomes rule W5 of the shared validator (`graph_body_walk_refusal`), so `replace_graph` refuses the graph as
  well as resolve. Authors split the node. No new runtime meaning.
- **Q-4, a required input behind a declined `if`: fixed in M3.** At stage 3, a required kind is optional when
  every node that produces it, upstream of this one, is skipped.

## Decisions

- **D1 — The scheduler** (`engine.rs` `run_to_rest`, the one loop that changes).
  - Each pass reads the run and its rows as today, then `walk::ready_masked(snapshot, steps, &in_flight)`.
  - Every cursor on a node that is not in flight is a candidate. They are taken in the order `ready` returns
    (writes owed first, blueprint A6; then position order). Each becomes one future: the body of today's
    `match` arm for that cursor, moved unchanged into `drive_cursor(&row, &snapshot, cursor)`, pushed as
    `Box::pin` into a `FuturesUnordered` that borrows `&self` (the `join_all` reasoning at `drive_group`,
    `engine.rs:4356`: no `'static`, no `JoinSet`).
  - **Weight.** A cursor's weight is the sessions it can start: `Run`, `Create` and `Fan` weigh the phase's
    `fan_out`; `Select` weighs 1 when the phase has a judge, else 0; `Decide`, `Loop` and `Rest { Failed }`
    weigh 0. A candidate is admitted when the in-flight weight plus its own is at most
    `snapshot.settings.max_parallel_sessions`, **or** nothing is in flight (a node heavier than the cap still
    runs, alone).
  - **In-place modes never overlap** (D4).
  - When nothing is in flight and nothing was admitted, the pass answers as today: `Finished` finishes the run,
    and any other cursor rests through `resting`. `Finished` is answered only when nothing is in flight.
  - Otherwise the loop awaits the next completed future, then re-derives. `MAX_ITERATIONS` counts passes as
    today.
  - A linear graph yields one cursor per pass, so it runs one future at a time, the same calls in the same order:
    the 140 existing cases pass unedited.
  - A frozen `max_parallel_sessions` of 0 (`app_u32` can freeze it, V12) behaves as 1 through the "nothing in
    flight" rule.
  - **Re-park arm** (V20, missing today): when nothing is in flight, the run is `running` and every cursor left is a
    `Rest` on an `awaiting_approval` step, the pass moves the run `running -> awaiting_approval` with no new note,
    instead of answering `Rest { run: Running }` and waiting for the sweep's D96.
- **D2 — In-flight masking** (`walk.rs`). `ready_masked(snapshot, steps, in_flight: &BTreeSet<&str>)` treats
  every in-flight node as live whatever its rows say, so it yields no cursor for that node and resolves none of
  its out-edges. `ready(snapshot, steps)` is `ready_masked` over an empty set; `cursor` keeps its signature and
  its eight callers (V3). Reason: a node's row turns `done` inside its future (`finish_step`, then the gate), and
  the landing comes after that. Without the mask, a join whose other predecessor finishes in that window is
  created on a tree that lacks the first predecessor's work.
- **D3 — Drain and cut** (Q-2), in the scheduler.
  - A future answering `Some(rest)` with `rest.run` not terminal sets `draining`: no new admissions; the loop
    awaits the rest of the set and then returns `resting(&run)` over a fresh read.
  - **A sibling's own park on an already-parked run parks its step only, engine-side** (V15). Today every park path
    answers `StaleWrite`, and `gate::park`'s store verb `park_step` moves step and run in one transaction, so on
    `ParkOutcome::RunMoved` it parks neither. The store contract stays as it is (seven `park_step` implementations,
    and `traits.rs` is T2's). Instead, the engine handles it in two places:
    - `gate::park` on `RunMoved` with the run `awaiting_approval` moves the step `running -> awaiting_approval`
      (`transition_step`) and writes its note;
    - `escalate`, `park_run`, `park_selection` and `park_on_resume` treat a run already `awaiting_approval` as
      parked rather than as `stale_run`.
  - A future answering a terminal `rest` (or an `Err`) cuts every other in-flight future.
    - **The cut signal** (V16). A session gets its own `local` channel only when its phase has a deadline;
      otherwise `drive` runs on the run's `Control` (`(self.parts.control)(run.id)`, `engine.rs:6814`). So the
      walk wraps the run's `Control` in a walk-scoped one that also carries a walk cut, and every session of the
      walk drives under it. A run cancel still arrives as before.
    - **What a cut writes.** A cut must not surface as `EngineError::Cancelled`, whose I-6 path writes nothing
      (`engine.rs:3925`). It gets its own arm that ends the row `cancelled` with a note naming the node that ended
      the run, the way `cancel_leased` (`engine.rs:2124-2146`) writes `cancelled` rows.
    - **Then.** The loop awaits the cut futures, runs `cleanup_run` for a terminal rest, and returns the first
      rest (or the first error). Dropping the futures is not the mechanism: dropped sessions would leave
      `running` rows on a finished run.
  - **An answer during a drain waits; no new guard** (V13, V14). In the shipped worker, `AnswerGate`, `RetryStep`,
    `SelectFanout` and `AcceptArtifact` are `Preempt::Never`. They wait on the run's `RunLocks` mutex, which the
    walk holds for its whole duration, and publish `FrameKind::Waiting` (`htui-worker/src/runtime.rs:2688-2691`,
    `:2866-2878`). So the answer runs after the drain, and a second walk never starts. Direct `Engine` callers
    with no fence (the conformance harness) are outside that contract, as today. A worker-level case pins the
    wait.
  - **Re-park after an answer.** When a human answers one parked step and another step of the run is still
    `awaiting_approval`, the walk after the unpark must park the run again on that step, with no second note.
    The blueprint checks `resting`, `resumable_park` and `unblock_case` for that shape (several `awaiting`
    rows under one run) and adds the arm if one is missing.
- **D4 — In-place isolation.** `local` and `shared_serialized` trees are the primary checkout itself.
  - **`local`** on two nodes that can be live together is refused at resolve (D7), so it never reaches the
    scheduler.
  - **`shared_serialized`**: the scheduler admits a node with an in-place tree only when no other in-place node of
    the run, on any repository in its scope, is in flight. `GixIsolator`'s `(box, repo)` guard spans
    `prepare`..`capture` (blueprint A-1, `real.rs:270-280`), not the reconcile, so without this a second node's
    agent could be working in the checkout while the first node lands in place.
  - `worktree` and `copy` nodes overlap freely. Their landings take turns under the admin lock (D5).
- **D5 — The reconcile lock is the isolator's, proven.** `reconcile_isolated` holds the repository's admin lock
  from the clean check through the merge's post-condition (`real.rs`, D70/D136). It merges onto a primary that
  moved ahead, and `worktree add`/`remove` take the same lock. That is the per-repository reconcile lock §7.3 and
  §9 ask for, within one process, and a run is walked by one process. M3 adds no second lock. It adds:
  - a `gix_isolator.rs` test: two `worktree` steps of one run, branched from one base, reconciled concurrently,
    both land (two merges, the second on top of the first), with the primary tree clean;
  - `FakeIsolator` instrumentation: the most concurrent `reconcile` calls on one repository. Every concurrent
    engine case asserts it never exceeds 1 for in-place modes.
- **D6 — Recovery frontier** (Q-1), `recover.rs` and `recover_run`.
  - `recover::frontiers(snapshot, steps) -> Vec<StepId>` is added beside `frontier`. T4 converts both callers,
    `recover_run` (`engine.rs:2813`) and `walk_resumed_from`'s crash-cut unpark (`:3595`), then removes
    `frontier`.
  - **Membership.** The done winner (`selected`, or index 0 when `fan_out = 1`) of each non-palette node's latest
    attempt, where no forward successor (D2's forward edges: `next`, `if`, forward `asks_you`; `forward_mask`)
    has a row that is not superseded or cancelled.
  - **Order.** Topological over forward edges, ties broken by position.
  - **A-3 becomes per node.** A `running` successor row keeps its predecessor off the frontier. `recover_run`'s
    global guard, which consults the frontier only when no step is `running` (`engine.rs:2812`), goes. With
    concurrency, a done but unlanded node beside a running sibling would otherwise be skipped, and the next walk
    would create its successor on a tree without its work.
  - **In-place trees: only the last lander per repository** (V5(e)). `reconcile_in_place` is not idempotent for
    an earlier `shared_serialized` node: HEAD sits at a later node's label, and it refuses with `primary_moved`.
    D4 admits an in-place node only after the previous in-place node's future, landing included, returned. So
    for each repository, only the latest-finished in-place done winner is a frontier member; earlier ones have
    provably landed. `local` cannot be concurrent (D7), and on a chain the successor rule already leaves one
    member.
  - `recover_run` and `walk_resumed_from` re-reconcile each frontier member in order. The first refusal parks, as
    today, and the rest wait for the next recovery. Each re-reconcile is boxed (D10).
  - **Fixes the M2 Q-C gap.** On a reordered chain or a loop on a branching graph, the frontier is the node the
    walk actually finished last, not the highest position.
- **D7 — Resolve refuses `local` on concurrent nodes** (`graph.rs`). After the phases are snapshotted:
  - **Two non-palette nodes can be live together** when neither reaches the other over forward edges (the
    `forward_mask` classification).
  - If two such nodes both resolve to `Isolation::Local`, resolve refuses with `ResolveError::LocalConcurrent {
    first, second }`, in the style of `LocalFanOut` (`graph.rs:1507`): first pair in position order, one
    sentence.
  - Every phase of a run resolves to `phase.isolation.unwrap_or(settings.default_isolation)` (V11). A `local`
    project therefore needs a per-phase `worktree` override on one of the two nodes. The sentence names both
    nodes and that remedy.
- **D8 — W5, mixed `asks_you` in the shared validator** (Q-3), `htui-core` `store/graph_rules.rs`.
  - The rule body of `graph.rs` `mixed_asks_you_refusal` moves into **`walk_refusal`** as W5: a node with two or
    more outgoing `asks_you` edges, at least one of them back. It gets one sentence in `traits.rs`'s style.
  - **Why `walk_refusal`** (V8): `graph_body_walk_refusal` wraps it, which covers `replace_graph` in both stores,
    resolve's `live_graph_refusal` and `override_graph`. Resolve's post-expansion `snapshot_walk_refusal`
    (`graph.rs:529`) calls `walk_refusal` directly, so a rule placed only in the wrapper would miss
    divide-expanded graphs.
  - `NotRunnable::MixedAsksYou` and `mixed_asks_you_refusal` are removed. At resolve the shape now refuses as
    `Resolution::Refused` with `InvalidGraph`, not `NotYetRunnable`.
  - Store conformance: a case where `replace_graph` refuses the shape on MemStore and PgStore (`CASES` 233 → 234).
  - **The orch test is rewritten.** `graph.rs` `a_node_with_a_back_asks_you_beside_another_asks_you_is_refused`
    (`:5229-5258`) saves its graph through `replace_graph` and asserts `Applied`, which W5 now refuses
    (`:4747-4751`). It becomes the save-refusal assertion. The stale `MixedAsksYou` doc at `gate.rs:586` is fixed
    in T3, which owns `gate.rs`.
- **D9 — `last_output_kind` follows the graph** (`htui-core` `model/run.rs`). It still reads a raw `&Value`, so any
  `v` decodes.
  - **Snapshot with an `edges` array:** the output kind of the highest-position non-palette phase that has no
    forward out-edge (a sink, forward per `forward_mask` over the decoded edges).
  - **Snapshot without edges (v1):** today's highest position.
  - **No sink is readable:** the highest position as before.
  - Callers (`command.rs:1296` `close_out_enabled`, store conformance `:14348`) are unchanged.
- **D10 — Boxing** (ANA-33 §9; memory: htui-orch stack headroom).
  - Every per-node future in the set is `Box::pin`ned.
  - `drive_cursor`'s arms box what M2 boxed (`decide`, `loop_back`).
  - Every recovery re-reconcile is boxed.
  - The gate runs `--no-fail-fast` with a grep for `SIGABRT`.
- **D11 — Required input behind a declined `if`** (Q-4), `walk.rs`.
  - `required_inputs(phase, snapshot)` becomes `required_inputs(phase, snapshot, steps)`: a listed kind is
    optional when a node reachable from this one produces it (M2 D9, unchanged), **or** when every node that
    produces it and reaches this one over forward edges is derived-skipped by `ready`'s state pass.
  - With no producer at all the kind stays required, as today.
  - The one non-test caller, `phase_spec` (`engine.rs:6256`), does not hold the rows (V18). It reads them with
    one `run_steps` call, so its own callers (`:1706`, `:6130`) are unchanged. Engine tests `:8280` and `:8291`
    and the `walk.rs` tests move to the new signature.
- **D12 — No new bound, no box-level cap.** `max_parallel_sessions` is already frozen (M1 D13). A box-level session
  cap stays MOD-139's question (PRD out of scope). `max_agents_per_run` already counts every node's `fan_out`, so
  concurrency adds no session it does not count.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Concurrent futures borrowing `&self` | `crates/htui-orch/src/engine.rs:4356-4370`, `:4475` (`drive_group`'s `join_all`) | no `'static`; no `std` guard across `.await`; a failed sibling never fails the others |
| Session cut | `engine.rs:7539-7596` (`drive_with_deadline`, `forward_or_cut`) | a `watch::Sender<Signal>` per session; `cancel_once` |
| Pure walk | `crates/htui-orch/src/walk.rs:452-524` (`ready`) | re-derived from rows each pass; every state arm explicit |
| Resolve refusal | `crates/htui-orch/src/graph.rs:1505-1511` (`LocalFanOut`) | named `ResolveError` variant, one sentence |
| Validator rule | `crates/htui-core/src/store/graph_rules.rs:343`, `:376`, `:481` | `Option<String>`, first refusal wins; `forward_mask` is the one classification |
| Engine conformance | `crates/htui-orch/src/conformance.rs:501` `CASES` (140) + pins at `tests/fake_conformance.rs:15-41` and `src/conformance.rs:10622-10626` | named case fns over `CaseHarness`; both pins (and their arithmetic strings) move |
| Holding a reconcile, then letting it answer | `crates/htui-orch/src/fake.rs` `pause_nth_reconcile` (~`:446`) | D2's window: a predecessor `done` but not yet landed |
| Worker command waiting on a walk | `crates/htui-worker/src/runtime.rs:571-591` (`lock_announcing`), `:2688-2691` | `Preempt::Never` commands wait on `RunLocks` and publish `Waiting` |
| Holding a session open | `crates/htui-orch/src/fake.rs:2089` (`stall_after_done`), `suspend_after_done`; `:427` (`stall_nth_reconcile`) | a `Notify` the case awaits, then releases |
| Real isolator test | `crates/htui-orch/tests/gix_isolator.rs`, `isolate/real.rs` tests | temp repos, real `git` |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-orch/src/walk.rs` | UPDATE | D2 `ready_masked`; D11 `required_inputs`; unit tests |
| `crates/htui-orch/src/status.rs` | UPDATE | `cursor` over `ready` (unchanged signature); doc |
| `crates/htui-orch/src/recover.rs` | UPDATE | D6 `frontiers`, unit tests |
| `crates/htui-core/src/model/run.rs` | UPDATE | D9 `last_output_kind` |
| `crates/htui-core/src/store/graph_rules.rs`, `store/traits.rs` | UPDATE | D8 W5 and its sentence |
| `crates/htui-core/src/store/conformance.rs`, `htui-core/tests/mem_store.rs:36-37`, `htui-store/tests/pg_conformance.rs:50-58` | UPDATE | D8 case; pins 233 → 234 |
| `crates/htui-orch/src/graph.rs` | UPDATE | D7 `LocalConcurrent`; D8 removal of `MixedAsksYou` |
| `crates/htui-orch/src/engine.rs` | UPDATE | D1 scheduler, `drive_cursor`, re-park arm; D3 drain, walk-scoped cut, the cut's `cancelled` arm, park-on-parked in `park_run`/`park_selection`/`park_on_resume`; D4; D6 wiring (`recover_run`, `walk_resumed_from`); D10; D11 (`phase_spec`, tests `:8280`, `:8291`) |
| `crates/htui-orch/src/gate.rs` | UPDATE | D3 `park` on `RunMoved` and `escalate` on a parked run; the stale `:586` doc |
| `crates/htui-worker/src/runtime.rs` (tests) | UPDATE | D3: an `AnswerGate` during a drained walk waits and then lands |
| `crates/htui-orch/src/fake.rs` | UPDATE | D5 reconcile-concurrency instrumentation; live-session counter for the cap |
| `crates/htui-orch/src/conformance.rs`, `tests/fake_conformance.rs` | UPDATE | new cases; `CASES` 140 moves (both pins) |
| `crates/htui-orch/tests/gix_isolator.rs` | UPDATE | D5 real concurrent landing |
| `crates/htui-worker/src/views.rs` | CHECK | the two `cursor` callers keep compiling; display stays "first ready" (MOD-142 owns the tree) |

## Tasks

TDD per task: failing tests first, then the change, then `cargo check --workspace --all-targets --all-features`.
Implementers commit incrementally. Worktree tasks follow the "workflow worktree implementers" memory.

### T1: Pure walker, frontier and model (parallel with T2 and T5)
- **Action:** D2 `ready_masked`; D11 `required_inputs(.., steps)`, with `phase_spec`'s `run_steps` read and the
  engine tests `:8280`/`:8291`; D6 `frontiers`, added beside `frontier` (callers untouched until T4); D9
  `last_output_kind`.
- **Unit tests:**
  - a masked predecessor holds its join;
  - a masked node yields no cursor and resolves no out-edge;
  - an empty mask equals `ready` (the parity oracle still passes);
  - `frontiers` on: two parallel done nodes; a reordered chain (the Q-C shape); a loop on a branch; a running
    successor; a skipped successor; a fan-out winner;
  - `last_output_kind` on a diamond, a branch with two sinks, and v1;
  - `required_inputs` behind a declined `if`, with a second producer that ran, and with no producer.
- **Files:** `htui-orch/src/{walk.rs,status.rs,recover.rs}`, `htui-orch/src/engine.rs` (`phase_spec` and the two
  tests at `:8280`/`:8291` only), `htui-core/src/model/run.rs`.
- **Validate:** `cargo test -p htui-orch --all-features walk:: status:: recover::`;
  `cargo test -p htui-core --all-features model::run`.

### T2: Resolve and the validator (parallel with T1 and T5)
- **Action:** D7 `LocalConcurrent`; D8 W5 in `graph_body_walk_refusal`, with `MixedAsksYou` removed from
  `graph.rs`.
- **Tests:**
  - `graph.rs`: `local` on a diamond's two arms is refused; on a chain it is not; `worktree` on a diamond is not;
    `shared_serialized` on a diamond is not;
  - `graph_rules.rs`: W5 unit tests;
  - store conformance: `replace_graph` refuses the mixed shape;
  - rewrite `graph.rs`'s mixed-`asks_you` case as a save refusal (D8);
  - a divide-expanded graph with the shape is refused at resolve (`snapshot_walk_refusal`).
- **Files:** `htui-orch/src/graph.rs`, `htui-core/src/store/{graph_rules.rs,traits.rs,conformance.rs}`,
  `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`. `MixedAsksYou` lives only in `graph.rs`
  (V9).
- **Validate:** `cargo test -p htui-orch --all-features graph::`; both stores' conformance with
  `--test-threads=1`.

### T5: The real isolator under concurrent landings (parallel with T1 and T2)
- **Action:** D5's `gix_isolator.rs` case, and a `shared_serialized` probe documenting that the guard does not span
  the reconcile (the reason for D4).
- **Files:** `htui-orch/tests/gix_isolator.rs`.
- **Validate:** `cargo test -p htui-orch --all-features --test gix_isolator`.

### T3: The concurrent scheduler (after T1, T2, T5; serial)
- **Action:** D1 `drive_cursor`, the in-flight set and the re-park arm; D3 drain, the walk-scoped cut and its
  `cancelled` arm, and park-on-parked in `gate.rs` and the engine's park paths; D4 the in-place rule; D10
  boxing; the stale `gate.rs:586` doc. Plus the `FakeIsolator` and fake-driver instrumentation (the most
  concurrent reconciles per repository; the most live sessions).
- **New conformance cases:**
  - two independent branches are live at once (both sessions held open together by the fake);
  - three branches under `max_parallel_sessions = 2`: never more than two live, all three done;
  - a fan-out node heavier than the cap runs alone;
  - a join is not created until both predecessors landed (a stalled reconcile on one holds the join);
  - two `worktree` landings, one repository: both land, reconciles never overlap;
  - `shared_serialized` branches never overlap;
  - a park on one branch drains the other: the sibling finishes and lands, and the run rests `awaiting_approval`;
  - a sibling parking on an already-parked run parks its step only; both answered in turn, then the run walks on;
  - after a drain, the run rests `awaiting_approval` on two parked steps; one answer re-parks on the other (the
    re-park arm), with no second note;
  - (htui-worker) an `AnswerGate` sent while the walk drains waits (`Waiting`), then lands;
  - a node ending the run cuts its live sibling: the sibling's row is `cancelled` with the note, and cleanup ran;
  - a loop on one branch while the other is live retires only its region;
  - a linear graph's call order is unchanged (an existing case's recorded call log, asserted equal).
- **Files:** `htui-orch/src/{engine.rs,gate.rs,fake.rs,conformance.rs}`, `htui-orch/tests/fake_conformance.rs`,
  `htui-worker/src/runtime.rs` (tests only).
- **Validate:** `cargo test -p htui-orch --all-features -- --test-threads=1`;
  `cargo test -p htui-worker --all-features -- --test-threads=1`.

### T4: Recovery among concurrent landings (after T3; serial)
- **Action:** D6 wiring in `recover_run` and `walk_resumed_from`; the global "no running step" guard replaced by
  the per-node rule; `frontier` removed; the `cursor` destructure in `recover_run` (V3) made safe for several
  parked or running rows.
- **New conformance cases:**
  - a crash with two done, unlanded branches: both land on recovery, in topological order;
  - a crash on a reordered chain recovers the right node (the M2 Q-C regression);
  - a crash with one branch done and the other running: the done one lands, the running one is adjudicated as
    today;
  - re-landing an already-landed node is idempotent (no second merge in the fake's log);
  - two `shared_serialized` nodes landed one after the other: only the later one is re-landed, and the run does
    not park on `primary_moved`;
  - a crash-cut unpark (`walk_resumed_from`) on a branching run lands every frontier node.
- **Files:** `htui-orch/src/{engine.rs,conformance.rs,fake.rs}`, `htui-orch/tests/fake_conformance.rs`.
- **Validate:** `cargo test -p htui-orch --all-features -- --test-threads=1`; `cargo test -p htui-worker
  --all-features`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                                  # featureless (memory)
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m3-test.log
grep -c SIGABRT /tmp/m3-test.log                                         # must be 0
cargo insta test --workspace --all-features --check
(cd crates/htui-store && cargo sqlx prepare --check)                     # no query changes expected
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Concurrent node futures overflow the 2 MiB debug stack | Medium | D10 boxing; `--no-fail-fast` and the `SIGABRT` grep |
| A linear graph's behaviour shifts (call order, rest shape) | Medium | D1 admits one future per pass on a linear graph; the 140 cases unedited; a recorded call-log case |
| A join starts on a tree missing a predecessor's landing | High without D2 | D2's mask; the stalled-reconcile case |
| Two walks of one run in one process (an answer renews this process's lease during a drain) | Low | the worker's `RunLocks` already serialises every command behind the walk (V13, V14); the worker case pins it |
| A park on a parked run errors with `StaleWrite` and leaves the sibling's step unparked | High without D3 | engine-side park-on-parked (V15); the two-park case |
| A sibling park or a cut leaves the run in a shape `resting`/`unblock` never met (several `awaiting` rows) | Medium | D3's re-park note; the two-park case; the blueprint audits every `cursor` caller (V3) |
| `shared_serialized` lands in place under a live agent | High without D4 | D4; the non-overlap case |
| Re-landing an already-landed node misbehaves in a mode H-3 does not cover | Medium | in-place: only the last lander per repository (D6, V5(e)); `worktree`/`copy`: `merge_of`; the idempotence and `shared_serialized` cases |
| A sink whose successors are all skipped stays on the frontier until the run ends | Low | harmless re-land (idempotent); noted in the record |
| Suite flakiness hides a regression | Medium | `--test-threads=1`; known sandbox Qdrant timeouts excluded by name |
| T1 ∥ T2 ∥ T5 merge conflicts | Low | disjoint file sets (V12); separate worktrees, merged T2 → T1 → T5 |

## Verified claims (plan fact-check, step 3.5; read at `1dc1e9e4`)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `run_to_rest` drives one cursor per pass; `admit` only creates rows | true | `engine.rs:3237-3330`, one `match cursor(..)` at `:3263`; `admit` `:3621` → `admit_indices` (`stage_one`, `create_step`), `Ok(None)`, or a `refuse_no_candidate` rest |
| V2 | The row turns `done` before its landing, in one future | true | `walk_live_step`: `finish_step` ~`:4113`, `gate::apply` ~`:4135`, `reconcile_done_step` ~`:4140`; `Landing::Retry` admits the next attempt in the same future (same node) |
| V3 | `cursor` has 8 non-test callers | true | `engine.rs:2534, 2725, 2856, 3263, 3572, 7192`; `htui-worker/src/views.rs:475, 744`; `pub`, re-exported `lib.rs:82` |
| V4 | `drive_group` uses `join_all` over `&self` | true | `engine.rs:4356-4370` (doc), `:4475` |
| V5 | The isolator's per-repo lock covers the `worktree`/`copy` landing; the `shared_serialized` guard does not cover reconcile; in-place re-landing | partly → D4, D6 | (a) `admin_guard` `real.rs:1158` through the post-probe, tip read before it; (b) `under_admin` for `worktree add`/`remove`; (c) guard released at `capture` (`:1465-1476`), a waiting mutex; (d) `reconcile_in_place` (`:1220`) takes no lock; (e) `merge_of` makes `worktree`/`copy` idempotent, but an earlier `shared_serialized` node refuses `primary_moved` |
| V6 | `recover::frontier` is the position rule, with one caller | partly → D6 | `recover.rs:366`; callers `recover_run` `engine.rs:2813` (only when no step is `running`) **and** `walk_resumed_from` `:3595` |
| V7 | `last_output_kind` reads the highest position; callers | true | `run.rs:657`; `command.rs:1296`, store `conformance.rs:14348`; `item.rs:138` is a doc link |
| V8 | Resolve reaches the shared validator | partly → D8 | `live_graph_refusal` `graph.rs:653-654` → `graph_body_walk_refusal`; post-expansion `snapshot_walk_refusal` `:529` → `walk_refusal` directly; `replace_graph` `mem.rs:4160`, `pg/write.rs:4349`; `override_graph` `graph.rs:1207` |
| V9 | Where `MixedAsksYou` lives | answered → D8 | `graph.rs` only (`:315`, `:551`, `:723-770`; test `:5229-5258` seeded through `replace_graph` asserting `Applied`, `:4747-4751`); stale doc `gate.rs:586` |
| V10 | `CASES` counts | **false → amended** | orch **140** (`conformance.rs:501-785`), pins `tests/fake_conformance.rs:15-41` and `src/conformance.rs:10622-10626`; store **233** (`:81-325`), pins `htui-core/tests/mem_store.rs:36-37`, `htui-store/tests/pg_conformance.rs:50-58` |
| V11 | `LocalFanOut` and the isolation fallback | true | `graph.rs:1505-1511`; a per-phase `isolation` override exists |
| V12 | `max_parallel_sessions` is frozen and unread by the engine | true | frozen `graph.rs:572`, default 4 `run.rs:866`; no engine read; `app_u32` can freeze 0 (D1) |
| V13 | The same process could start a second walk through an answer | partly → D3 | Engine level yes (`take_lease` → `renew_lease`, `engine.rs:2329`); the worker's `Preempt::Never` commands wait on `RunLocks` (`runtime.rs:2688-2691`, `:2866-2878`) |
| V14 | A per-process live-walk registry exists | partly → D3 | `DeadWalks` marks dead walks only; `RunLocks` (`runtime.rs:658-705`) is the per-run exclusion |
| V15 | A park on a parked run | **false → D3** | every path errors `StaleWrite`; `gate::park` → store `park_step` → `ParkOutcome::RunMoved` (`mem.rs:7332`), so the step is not parked either; `escalate`, `park_run`, `park_selection`, `park_interrupted`, D96 go through `move_run` (`:7165`) |
| V16 | A cut relays like a run cancel | partly → D3 | a per-session `local` channel only with a deadline (`engine.rs:6952`); `Control` per run (`:6814`, `ControlFor` `:308`); a run cancel is `EngineError::Cancelled`, writes nothing (I-6, `:3925`); `cancel_leased` (`:2124-2146`) writes `cancelled` rows |
| V17 | Fake seams | true | `fake.rs:427` `stall_nth_reconcile`, `:2089` `stall_after_done`, `:2114` `suspend_after_done`, ~`:446` `pause_nth_reconcile` |
| V18 | `required_inputs`'s one caller holds the rows | partly → D11 | `walk.rs:555`, `lib.rs:85`; caller `engine.rs:6256` `phase_spec` holds no rows (callers `:1706`, `:6130`); tests `engine.rs:8280`, `:8291`, `walk.rs:1106-1162` |
| V19 | `check_fan_out_caps` counts each node's `fan_out` | true | `graph.rs:1368-1397`, called `:584` |
| V20 | A `running` run resting on an `awaiting` step re-parks | **false → D1** | `Cursor::Rest` → `resting` (`:7189`) writes nothing, `Rest { Running }`; only the sweep's D96 (`:2836-2848`) re-parks |
| V21 | The frontier needs no new column | true | `RunStep` (`run.rs:322-377`): `position`, `attempt`, `fanout_index`, `status`, `selected`, `branch`, `gate_outcome` |
| V22 | T1, T2 and T5 are pairwise disjoint by file | true | T1 `htui-orch/src/{walk,status,recover}.rs` + `engine.rs` (`phase_spec`, two tests) + `htui-core/src/model/run.rs`; T2 `htui-orch/src/graph.rs` + `htui-core/src/store/{graph_rules,traits,conformance}.rs` + `htui-core/tests/mem_store.rs` + `htui-store/tests/pg_conformance.rs`; T5 `htui-orch/tests/gix_isolator.rs`. D3's park fix stays engine-side, so `traits.rs` stays T2's alone. T3 and T4 both touch `engine.rs`, so they run serially |
| V23 | No migration needed; current highest numbers | true | `migrations/0025_run_step_branch.sql`, `cache_migrations/0008_run_step_branch.sql`; M3 adds none |

## Acceptance

- [ ] Every task done, each with failing-first tests
- [ ] The existing orch cases and every engine test pass unedited (D8's one amended expectation excepted)
- [ ] Ready nodes run concurrently up to `max_parallel_sessions`; a join waits for its predecessors' landings
- [ ] Drain on park, cut on end, park on a parked run, re-park after one answer, an answer waiting on a drain: each
      has a passing case
- [ ] `local` on concurrent nodes is refused at resolve; `shared_serialized` never overlaps; `worktree` landings
      take turns and both land (fake and real isolator)
- [ ] Recovery lands every frontier node in topological order; the Q-C shape recovers the right node
- [ ] `last_output_kind` reads the sink; mixed `asks_you` is refused at save; the join-behind-`if` input is optional
- [ ] Validation passes (both stores, `--test-threads=1`, no SIGABRT)
