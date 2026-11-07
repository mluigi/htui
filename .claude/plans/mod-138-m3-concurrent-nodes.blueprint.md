# Blueprint: MOD-138 M3, concurrent nodes (T1 to T5)

Written by `code-architect` on 2026-11-07 from the confirmed plan
`.claude/plans/mod-138-m3-concurrent-nodes.plan.md` (Q-1 to Q-4 binding, D1 to D12, T1 to T5, V1 to V23),
the PRD `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md` (milestone 3), `docs/ANA-33.md` §7.3, §7.7, §9,
and the M2 plan and blueprint. Tree read at `3a20e858` (branch `hr/MOD-138`).

The plan stays the spec. §0 lists every place where the tree disagrees with the plan, or where the plan
says nothing:
- rows marked **adopted** are implementation detail or a corrected claim, and need no decision;
- rows marked **A1 to A12** need the maintainer. Each fix is collected, smallest first, under "Plan
  amendments needed" at the end (§9).

Nothing was run. Every claim below comes from reading source.

## 0. Plan claims the tree contradicts or extends

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | **In-flight futures cannot borrow the pass's locals.** Today each arm borrows `&row`, `&snapshot` and moves a `RunStep` out of `steps`. All three are locals of one loop iteration, and a future in a `FuturesUnordered` outlives that iteration. D2's `in_flight: &BTreeSet<&str>` has the same problem: its `&str` borrows one pass's snapshot. | `engine.rs:3237-3345`; `walk.rs` `Graph::of` keys by name | **A1**: the node key is the **position** (`i32`, which every `Cursor` carries or derives from its step). A future owns its inputs: a `Run` clone, an `Arc<GraphSnapshot>`, an `Arc<[RunStep]>`, the `Cursor`. All node futures come from one `async fn drive_node`, so they share one concrete type and need no `dyn` and no `'static` (§3.2). |
| F2 | **"A linear graph yields one cursor per pass" is false for M2's owed writes.** `ready` puts `Decide`/`Loop` first and then every reached node. A chain `T→…→S→U` with a back `asks_you` `S→T` yields `[Loop{S}, Create{U}]`: `S` is done and approved, so `next S→U` is taken. A node with an `if` edge and a `next` edge yields `[Decide{X}, Create{Z}]`. Today `cursor()` takes the first, the loop retires `T..S`, and `U` is never reached. D1 would admit both, and `U` gets a row while its predecessor is being retired. | `walk.rs:452-524` (`ready`), `edge_state` | **A2**: the comparability guard (§3.3 rule c). A candidate is admitted only when its node is **incomparable** with every in-flight node: neither reaches the other over forward edges. Under the guard a chain never has two futures in flight, so linear parity holds by construction. The same rule proves Q6's loop question (F2a), and it is the relation D7 already calls "can be live together". |
| F2a | **A loop's region can meet an in-flight node without F2's guard.** Take `A→S`, `A→S'`, `S' changes_requested→A`, and `S changes_requested→T` with `T→A`. `S'` retires `{A,S'}`, `A` is re-created and runs while `S` is in flight, then `S` rejects and `retire_slot` moves the running `A` to `cancelled` under its own live session. | `gate.rs:1232-1300` (`retire`, `retire_slot`: running → cancelled) | Closed by A2. Every node of `region(target, source)` other than the source is an ancestor of the source. An ancestor is never admitted while the source is in flight, and the reverse also holds. So `region ∩ in_flight ⊆ {source}`, for the gate-time `edge_loop` and for `loop_back` alike. |
| F3 | **D4 is too narrow.** Every `prepare` gets the whole `&run.repo_scope` (`engine.rs:3958`, `:4784`), so all nodes of a run share every repository. `reconcile_isolated` refuses on a dirty primary (`dirty_primary_tree`) and merges `--no-ff` *into the checked-out primary*. So a `worktree` node landing while an in-place node's agent edits that checkout either parks or merges under the agent. `worktree add` at the primary's `HEAD` is also unsafe beside a live in-place agent. | `real.rs:1127-1218`, `:655-718`; engine `:3958` | **A3**: an in-place tree is exclusive. A cursor that touches trees (`Run`, `Fan`, `Select`) of an in-place phase is admitted only when no tree-touching future is in flight. No tree-touching cursor is admitted while an in-place one is in flight. Because the scope is per run, D4's "per repository" means per run. D7 becomes belt-and-braces (F22). |
| F4 | **`cleanup_run` inside a terminal node's future removes live siblings' trees.** Every terminal path cleans up inside its own arm. That is 11 sites: `rejection` `:1104`, `loop_over` `:1138`, `settle_failed` `:3188`, `refuse_no_candidate` `:3794`, `walk_live_step` `:4158`, `fail_hard` `:4333`, group failures `:4635`, `:4662`, `select_stage` `:5054`, judge `:5872`, `block_and_fail` `:6106`. `isolator.cleanup(run, trees)` removes every tree of the run. | `engine.rs:5998-6019` | **A4**: `cleanup_run` defers while a sibling is in flight (§3.5). The scheduler runs the deferred cleanup once the set drains. A future alone in flight cleans up immediately, as today, so linear call order is unchanged. |
| F5 | **There is nowhere to put walk-scoped state, and the cut is easiest as I-6 plus a scheduler write.** `drive_once` is the one reader of `ControlFor` (`:6814`). `Engine` holds only `parts` (`:531-548`). Each layer already maps a cancelled session to `EngineError::Cancelled` and writes nothing: `walk_step` `:3925`, `group_error`, `refused_over_finish`. Threading a new error variant through `walk_live_step`, `candidate_live`, the judge and `fail_hard`'s catch-alls is invasive. | `engine.rs:6797-7016`, `:3893-3938`, `:7631-7656` | **A5**: an Engine-private `WalkScopes` registry carries each walk's own `watch` channel. `drive_once` drives under it, and it is fed from the run's `Control` plus the walk's cut. Inside the futures a cut **is** `Cancelled` (I-6: nothing written). The scheduler, which knows it sent the cut and sees that the run's own control is not a cancel, then writes the rows the way `cancel_leased` does (§3.6). This amends D3's "a cut must not surface as `Cancelled`". |
| F6 | **The park-on-parked list is incomplete, and one reader uses a stale row.** `park_interrupted` (`:3199`, reached from `settle_failed` inside a walk future and from `recover_run`) also calls `move_run(Running→AwaitingApproval)`. `park_on_resume` branches on `row.status` (`:3539`), but `decide` passes the row read at pass start (`run.clone()`, `:1207`), which is stale under concurrency. | engine `:2845`, `:3208`, `:3540`, `:5132`, `:5955`; gate `:535`, `:1332` | **adopted**: one helper, `park_the_run`, replaces the five engine moves (§3.7), plus a gate-side twin. **No store trait change**: `transition_run`, `transition_step` and `run` already exist on `WorkerStore`. |
| F7 | **The re-park rule must be strict.** D1's arm fires only when "every cursor left is a `Rest`", so after one answer the walk would keep admitting work around a step still `awaiting_approval`. That disagrees with three things in the tree. (a) `recover_run`'s D96 already re-parks at once when any step is awaiting, so a `worker`-box answer (`Tails::HandBack`, adopted and recovered) would re-park immediately while a `tui`-box answer walks on. (b) D6's in-place "latest-finished lander" proof needs it: under the loose rule an in-place node parked at its gate can land *after* a later-finished node, and `finished_at` no longer orders landings. (c) `resumable` reads the first cursor (F8). Q-2's own words: on a park the walk "admits nothing new". | `engine.rs:2834-2849`; D6 | **A6** (recommended): the walk admits nothing while any step is `awaiting_approval`. With nothing in flight and the run `running`, the pass re-parks, with no note, and rests. A drained sibling's owed `Decide`/`Loop`/successors wait for the answer. |
| F8 | **`resumable(&cursor(..))` reads only the first cursor.** After a drain, a sibling's owed `Decide` sorts first (A6 of M2), so a run parked on a real gate reads as resumable. `Unblock` is then offered and `walk_resumed_from` unparks it. | `engine.rs:2534`, `:2725`, `:3572`; `views.rs:475`; `status.rs:429-478` | **A6** (with F7): `status::run_resumable(snapshot, steps)` is true when **every** cursor of `ready` is resumable (`resumable_park` or `crashed_rejection`). The four callers switch to it. On a linear graph `ready` holds one node cursor, so it reads exactly as today. |
| F9 | **`recover_run` returns on the first running row's park.** With two concurrent sessions at the crash, the first `recover_step` can park (`reset_interrupted` → `park_interrupted`) and the second row stays `running` under a parked run. The sweep adopts only `running` runs, and a walk rests on a stray `Rest{Running}`, so that row is stuck for good. | `engine.rs:2817-2830` | **A7** (T4): every `running` row is adjudicated. The first `Next` that is a park is kept and the loop goes on, which is the recovery's drain. A terminal `Next` cancels the remaining unsettled rows as the cut does (§3.6). This relies on F6's tolerant park. |
| F10 | **Q-1's "a successor row proves the landing" has a hole.** A loop whose region holds `N` but not `N`'s successor `S` (`S` is off every path from target to source) re-runs `N` and leaves `S`'s **older** row, so "rows win". A crash after `N`'s new winner is `done` and before it lands leaves `S`'s old row as a false proof, and the landing is lost. M2's position rule has the same hole, so this is not a regression. | `gate.rs` `edge_loop` region; `RunStep` has no `created_at` (`run.rs:320-377`) | **A8**: a successor row counts as proof only when its `started_at` is set and `>= winner.finished_at`. Both are stamped by the engine clock (`transition_step`'s `at`, `finish_step`'s `finished_at`). An unstarted `pending` successor proves nothing, so the winner re-lands, which is idempotent (`merge_of`, or the last-lander rule). No column is needed, so Q-1 holds. |
| F11 | **The in-place last-lander rule has to range over every done winner, in every mode.** (a) A later `worktree` merge also moves `HEAD`, so re-landing the last in-place node refuses `primary_moved` (`reconcile_in_place`, `:1220-1265`). (b) `local`'s re-land never refuses, but it returns the current `HEAD` and records **another node's** commit as this node's `after_hash`. (c) A non-member that landed later (its successor has a row) still moved `HEAD`. | `real.rs:1220-1265` | **A9**: an in-place frontier member is kept only when no done winner of the run, in any mode and member or not, has a later `finished_at`. A `worktree`/`copy` member is always kept (`merge_of`). Under A3 and A6, `finished_at` order equals landing order. Ties (a frozen `TestClock`) keep the member, so cases advance the clock. |
| F12 | **An answer lands only the answered step.** Take two parks, A and B. A crash between A's `answer_gate` write and its `unpark` leaves A `done` and unlanded under a parked run. A later answer to B lands B and walks, and A's successor is created on a tree without A's work. Today only one step can be parked, so this cannot happen yet. | `engine.rs:1040-1047` | **A10** (T4): `answer_guarded`'s approve tail first reconciles every *other* `recover::frontiers` member, in order, boxed, and then the answered step exactly as today. On a linear graph that set is empty, so there are no extra calls. |
| F13 | D3 says a drained walk returns `resting(&run)`. `resting` sets `failure: None` on purpose, so a drained park would lose its typed reason (`loop_failure`, `Interrupted`). Today's single park returns the arm's own `Rest`. | `engine.rs:7180-7215` | **A11**: the walk returns the **first terminal rest**, else the **first park rest**, exactly as the future answered it. Linear parity holds. |
| F14 | **Concurrency cases are vacuous on the shipped fakes.** `FakeDriver`, `MemStore` and `FakeIsolator::reconcile` never return `Pending` (`fake.rs:630-687` has no await unless stalled). Each node future therefore runs to completion in its first poll, and a high-water mark reads 1 whatever the scheduler does. The fake also has no admin lock, so "worktree reconciles never overlap" is a property of the real isolator (T5), not of the fake. | `fake.rs:519-687`, `:2229-2270` | **adopted**: opt-in pending points plus counters (§3.9). The fake proves admission (cap, exclusivity, mask). T5 proves the real lock. |
| F15 | T4 removes `recover::frontier`, but T4's file list has no `recover.rs`, where `frontier` and its unit tests live (`recover.rs:366`, test imports `:413`). | as cited | **A12**: T4 owns `recover.rs` too. T1 is merged before T4 starts, so there is no conflict. |
| F16 | `Decide{X}` and `Loop{X}` can both be owed for one node in one pass. | `walk.rs:470-490` | **adopted**: one candidate per key per pass, the first in `ready` order. |
| F17 | A `?` on the scheduler's own reads while futures are in flight would drop them, which is crash semantics: sessions killed, rows `running`. | D1 | **adopted**: a scheduler error ends the walk like a future's error: cut, await, raise (§3.6). |
| F18 | A run cancel today goes on admitting until the next session start. Stopping admission on a cancel would change pinned linear cases. | `engine.rs:6814-6818` | **adopted**: a run cancel is only **forwarded** into the walk channel. Admission is unchanged, and the first `Err(Cancelled)` ends the walk with nothing written (I-6). |
| F19 | `verify_command` and reconcile are not cut points. A cut waits for a running verify or merge to finish, then the future meets the cut at its next session start (`drive_once`'s B-17 check) or returns. | `engine.rs:4181`, `:5908` | **adopted**, documented. |
| F20 | A parked selection leaves no `awaiting_approval` row, only a `Select` cursor. Once another park is answered, A6 admits `Select` again and `park_selection` writes a second "awaits selection" note. | `engine.rs:5123-5175` | **adopted**: a harmless duplicate note, documented. MOD-139 may mark parked slots. |
| F21 | MOD-125's live stop is not wired. `stop_run` has no engine or worker caller. A run read `stopped` mid-walk is drained, and when MOD-125 M3 wires stop it should send it through the walk channel. | `git grep 'stop_run('` | **adopted**: a note in `DECISIONS.md` for MOD-125 M3. |
| F22 | With A3, `local` on a diamond simply serialises, so D7's resolve refusal is no longer what makes it safe. | A3 | Kept as planned (T2): an early, readable author error. The maintainer may drop D7 and T2 shrinks. |
| F23 | PgStore: each in-flight node can hold one pooled connection during a call, plus the heartbeat and the scheduler's reads. | `htui-store` pool | **adopted**: a Risks row. `max_parallel_sessions + 2` connections per walk at most. |

---

## 1. Where things live

| Item | File | Task |
|---|---|---|
| `ready_masked`, mask in `Graph::reach`, `Graph::concurrent`, `Graph::order`, `Graph::forward_successors`, `required_inputs(.., steps)` | `htui-orch/src/walk.rs` | T1 |
| `position_of(cursor, steps)`, `run_resumable(snapshot, steps)` (A6), `cursor` doc | `htui-orch/src/status.rs` | T1 |
| `frontiers` (Q-1 + A8 + A9), beside `frontier` | `htui-orch/src/recover.rs` | T1 (T4 removes `frontier`) |
| `last_output_kind` (D9) | `htui-core/src/model/run.rs` | T1 |
| `phase_spec` reads `run_steps` (D11); tests `:8280`, `:8291` | `htui-orch/src/engine.rs` | T1 |
| `LocalConcurrent`, W5 | as in the plan | T2 |
| concurrent real landings; dirty-primary probe (A3 evidence) | `htui-orch/tests/gix_isolator.rs` | T5 |
| `schedule.rs` (new): `Flight`, `PassPlan`, `plan_pass` (pure) | `htui-orch/src/schedule.rs`, `lib.rs`: `mod schedule;` | T3 |
| `WalkScopes`, `run_to_rest`, `drive_node`, `drive_cursor`, `settle_walk`, `session_control`, `cleanup_run` split, `park_the_run`, park sites, `hand_back_resume`/`unblock_case` on `run_resumable` | `htui-orch/src/engine.rs` | T3 |
| `park` on `RunMoved`, `escalate` tolerant, `:586` doc | `htui-orch/src/gate.rs` | T3 |
| counters and pending points | `htui-orch/src/fake.rs`, `Orchestrate` in `conformance.rs` | T3 |
| `unblock_case` on `run_resumable` | `htui-worker/src/views.rs` | T3 |
| worker answer-waits case | `htui-worker/src/runtime.rs` (tests) | T3 |
| `recover_run`, `walk_resumed_from`, `answer_guarded` (A10) on `frontiers`; `frontier` removed | `engine.rs`, `recover.rs` | T4 |

---

## 2. T1: pure walker, frontier, model

### 2.1 `walk.rs`

```rust
/// MOD-138 M3 D2: `ready` with every node at a position in `in_flight` read as live and
/// unresolved, whatever its rows say: no cursor for it, its forward out-edges `Unresolved`, and
/// no `Decide`/`Loop` owed for it. `ready(s, st) == ready_masked(s, st, &BTreeSet::new())`.
#[must_use]
pub fn ready_masked(snapshot: &GraphSnapshot, steps: &[RunStep], in_flight: &BTreeSet<i32>) -> Vec<Cursor>;
```

- Build `masked: Vec<bool>` per phase index (`phase.position ∈ in_flight`, non-palette).
- In the owed loop, skip a masked node before pushing `Decide`/`Loop`.
- `Graph::reach(&self, states, masked: &[bool])`: a masked node is `Reach::Waiting` before the `Done` test. `edge_state` then answers `Unresolved` for each of its out-edges, through its existing `_` arm. Do **not** drop the masked state to `None`: a `None` node computes its reach from its in-edges and could read `Skipped`, which declines its out-edges.
- In the cursor collection, skip masked nodes. The fallback on a forward cycle also skips them.
- `[Finished]` cannot appear while a mask is non-empty, because a masked node is never done. A test pins this.

```rust
impl Graph<'_> {
    /// M3 A2: positions `a` and `b` (non-palette, known) can be live together: neither reaches the other
    /// over forward edges. `false` for `a == b` and for an unknown position.
    #[must_use] pub fn concurrent(&self, a: i32, b: i32) -> bool;
    /// Walked positions in forward-topological order, ties by `(position, index)`; on a forward
    /// cycle, position order.
    #[must_use] pub fn order(&self) -> Vec<i32>;
    /// Forward successor positions of `position`.
    #[must_use] pub fn forward_successors(&self, position: i32) -> Vec<i32>;
}

/// D11 (Q-4): `phase`'s required kinds. A listed kind is optional when (a) a node reachable from
/// `phase` produces it (M2 D9), or (b) at least one non-palette node that produces it reaches
/// `phase` over forward edges, and every such producer is `Reach::Skipped` in `ready`'s state
/// pass over `steps`. On a forward cycle (no reach) (b) never applies. No producer at all: required.
#[must_use]
pub fn required_inputs(phase: &SnapshotPhase, snapshot: &GraphSnapshot, steps: &[RunStep]) -> Vec<String>;
```

### 2.2 `status.rs`

```rust
/// The position a cursor stands on: `Create`/`Fan`/`Select` carry it; `Run`/`Rest`/`Decide`/`Loop`
/// read their step's row. `None` for `Finished` and for a step `steps` does not hold.
#[must_use] pub fn position_of(cursor: &Cursor, steps: &[RunStep]) -> Option<i32>;

/// M3 A6: whether a parked run is one a resume may walk on from: `ready` is non-empty and EVERY
/// cursor of it is `resumable_park` or a `crashed_rejection`. On a linear graph `ready` holds
/// one node cursor, so this is `resumable(&cursor(..), steps)`.
#[must_use] pub fn run_resumable(snapshot: &GraphSnapshot, steps: &[RunStep]) -> bool;
```

`resting` (`engine.rs:7189`) may switch to `position_of` in T3. That is a pure refactor.

### 2.3 `recover.rs`

```rust
/// M3 D6 (Q-1, A8, A9): every done winner that may not have landed, in forward-topological
/// order (ties by position).
#[must_use] pub fn frontiers(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Vec<StepId>;
```

The algorithm:
1. `g = walk::Graph::of(snapshot)`. For every `p` in `g.order()`, take `w = winner_at(phase(p), steps)`. This is the existing fn: latest attempt, `done`, `selected`, or index 0 when `fan_out == 1`.
2. **The successor rule.** Drop `w` when some `s ∈ g.forward_successors(p)` has a row at position `s` (any `fanout_index`, judge included) whose status is not `Superseded`/`Cancelled`, **and** (A8) whose `started_at` is `Some(t)` with `t >= w.finished_at.unwrap_or(MIN)`. A running successor is excluded by this rule, which makes A-3 per node.
3. **The in-place rule (A9).** Let `last = max over every done winner of the run (all phases, all modes, members or not) by finished_at`. Keep an in-place member (`phase.isolation ∈ {Local, SharedSerialized}`) only when no done winner has `finished_at > w.finished_at`.
4. Return the ids in `order()`.

`frontier` stays in place until T4.

Unit tests are the plan's six, plus:
- A8: an off-region successor's older row does not hide a re-run winner;
- A8: a pending successor re-lands its predecessor;
- A9: an in-place sink followed by a worktree landing is not a member.

### 2.4 `model/run.rs` and `phase_spec`

The D9 and D11 wiring are as in the plan. `phase_spec` (`engine.rs:6232`) adds one `self.parts.store.run_steps(run.id)` read before `required_inputs`.

---

## 3. T3: the concurrent scheduler

### 3.1 `WalkScopes` (A5)

`engine.rs` adds a new private field to `Engine`, `walks: WalkScopes`. `Engine::new` stays a `const fn`, because `Mutex::new` and `BTreeMap::new` are both const.

```rust
/// MOD-138 M3 D3: the state of every walk `run_to_rest` has open in this engine, by run. A std
/// lock, never held across an `.await` (the `DeadWalks` shape).
#[derive(Debug, Default)]
struct WalkScopes(std::sync::Mutex<BTreeMap<RunId, WalkScope>>);

#[derive(Debug)]
struct WalkScope {
    /// The walk's control: the run's cancel forwarded into it, and the walk's own cut.
    signal: watch::Sender<Signal>,
    /// Node futures in flight; `cleanup_run` defers while this is above 1 (A4).
    in_flight: usize,
    /// A terminal path's cleanup was deferred (A4).
    cleanup_owed: bool,
}

/// Removes the run's scope on drop, including the drop of a walk `heartbeaten` abandons.
struct ScopeGuard<'e> { scopes: &'e WalkScopes, run: RunId }
```

`WalkScopes` methods:
- `open(run) -> ScopeGuard` (`debug_assert!` that it is absent);
- `control(run) -> Option<Control>`;
- `forward(run, &Control)`: copies a run cancel in, `send_if_modified`, cancel-once;
- `cut(run)` sends `Signal::Cancel { grace: RELAY_GRACE }`;
- `set_in_flight(run, n)`;
- `defer_cleanup(run) -> bool`;
- `take_cleanup_owed(run) -> bool`.

`drive_once` (`:6814`) changes in one line: `let mut control = self.session_control(run.id);`. That returns the scope's `Control` when a walk is open for the run, else `(self.parts.control)(run.id)` as today. Every later use of `control` in `drive_once` (the B-17 check, the deadline's local channel, the post-drive `is_cancel`) reads the walk control unchanged.

### 3.2 `drive_node` and `drive_cursor` (D1, F1)

```rust
/// D1: one node's future, the unit `run_to_rest` keeps in flight. Owned inputs (F1), so the
/// future borrows `&self` only. Every one is boxed at its push (D10).
async fn drive_node(
    &self, row: Run, snapshot: Arc<GraphSnapshot>, steps: Arc<[RunStep]>, cursor: Cursor, position: i32,
) -> (i32, Result<Option<Rest>, EngineError>) {
    (position, self.drive_cursor(&row, &snapshot, &steps, cursor).await)
}

/// D1: today's `match` arm for `cursor`, moved unchanged (`engine.rs:3263-3337`). A step id the
/// pass's rows do not hold answers `Ok(None)`, today's `continue`: the next pass re-derives.
/// `Finished` and a non-`Failed` `Rest` never reach here (`plan_pass` answers them).
async fn drive_cursor(&self, row: &Run, snapshot: &GraphSnapshot, steps: &[RunStep], cursor: Cursor)
    -> Result<Option<Rest>, EngineError>;
```

Arms:
- `Rest{Failed}` → `settle_failed`;
- `Create` → `admit`;
- `Fan` → `drive_group`;
- `Select` → `select_stage`;
- `Run` → `walk_step` (with `find(..).cloned()`);
- `Decide` → `Box::pin(decide)`;
- `Loop` → `Box::pin(loop_back)`;
- `Finished` and `Rest{..}` → `debug_assert!(false)` and `Ok(None)`.

### 3.3 `schedule.rs` (new, pure): admission (D1, D4/A3, A2, A6)

```rust
/// A future's admission facts, kept while it is in flight.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flight { pub weight: u32, pub touches: bool, pub in_place: bool }

#[derive(Debug)]
pub(crate) enum PassPlan {
    /// Push these, in order.
    Admit(Vec<(Cursor, i32, Flight)>),
    /// Nothing to admit; something is in flight.
    Wait,
    /// Nothing in flight, `ready == [Finished]`: finish the run `done`.
    Finished,
    /// Nothing in flight; the run is `running` over an `awaiting_approval` step (V20, A6).
    RePark,
    /// Nothing in flight; answer `resting` (a stray `running`/`stopped` row, or a parked run).
    Rest,
}

pub(crate) fn plan_pass(
    snapshot: &GraphSnapshot, steps: &[RunStep], run: RunStatus,
    cursors: &[Cursor], in_flight: &BTreeMap<i32, Flight>,
) -> PassPlan;
```

The rules are applied in order.

1. **Blocking rests (A6).** The pass admits nothing when any cursor is `Rest{AwaitingApproval | Running | Stopped}`. Masked nodes yield none, so such a row is a parked step or a session this walk does not own.
   - Nothing in flight, an `AwaitingApproval` among them and `run == Running`: answer `RePark`.
   - Nothing in flight otherwise: answer `Rest`.
   - Something in flight: answer `Wait`.
   - Under the plan's looser D1 (A6 rejected), only `AwaitingApproval` is excluded from this rule, and it applies only when every cursor is a blocking rest.
2. `cursors == [Finished]`: answer `Finished` when nothing is in flight, else `Wait`. The second case is unreachable, as F2 shows.
3. **Candidates** are every other cursor, in `ready` order, with `position_of` and `phase_at` from the snapshot. Compute the flight:
   - `weight`: `Run`/`Create`/`Fan` weigh `fan_out.max(1)`; `Select` weighs `1` with `phase.judge.is_some()`, else `0`; `Decide`/`Loop`/`Rest{Failed}` weigh `0`;
   - `touches = matches!(cursor, Run | Fan | Select)`;
   - `in_place = matches!(phase.isolation, Local | SharedSerialized)`.

   Against `taken = in_flight ∪ admitted so far`, a candidate is admitted when **all** of these hold:
   - (a) its position is not in `taken` (F16);
   - (b) A2: `graph.concurrent(position, q)` for every `q ∈ taken`;
   - (c) A3: when `touches && in_place`, no taken flight has `touches`; when `touches`, no taken flight has `touches && in_place`;
   - (d) weight: `sum(taken) + weight <= max(settings.max_parallel_sessions, 1)`, **or** `taken` is empty.

   A refused candidate is skipped and the next one tried.
4. With something admitted, answer `Admit`. With nothing admitted and something in flight, answer `Wait`. With nothing admitted and nothing in flight the case is unreachable: the first candidate passes (a) to (d) against an empty `taken`. Answer `Rest` defensively.

**Linear parity, by construction.** On a graph whose walked nodes are totally ordered by forward reachability, rule (b) admits at most one candidate per pass and none while one is in flight. That candidate is `ready()[0]`, which is today's `cursor()`, unless rule 1 or 2 answers first, as today's `Rest`/`Finished` arms did.

The pure unit tests in `schedule.rs` cover: the cap; cap 0 behaving as 1; a heavy node alone; exclusivity in both directions; comparability; dedupe; and the three no-admission answers.

### 3.4 `run_to_rest` (D1, D3)

```rust
pub async fn run_to_rest(&self, run: RunId) -> Result<Rest, EngineError> {
    let _scope = self.walks.open(run);
    let mut flights: BTreeMap<i32, schedule::Flight> = BTreeMap::new();
    let mut in_flight = futures::stream::FuturesUnordered::new();
    let mut end = WalkEnd::default();   // first terminal rest, first park rest, first error, ender, cut, parked
    let mut passes = 0_u32;
    loop {
        let mut run_control = (self.parts.control)(run);          // per pass, as today's per-session lookup
        self.walks.forward(run, &run_control);                    // F18: forward only
        if end.admits() {
            passes += 1;
            if passes > MAX_ITERATIONS { end.error(EngineError::Stalled { run, passes: MAX_ITERATIONS }, None, &self.walks, run, !flights.is_empty()); }
            else if let Some(plan) = self.pass(run, &flights, &mut end).await {  // reads; errors → end (F17)
                match plan {
                    Pass::Finished => /* flights empty */ { finish_run(Done); self.cleanup_run(run).await?; return Ok(Rest { Done, None, None }) }
                    Pass::RePark   => { self.park_the_run(run, self.now()).await?; return self.resting(&self.run(run).await?).await; }
                    Pass::Rest(row) => return self.resting(&row).await,
                    Pass::Admit { row, snapshot, steps, admitted } => for (cursor, position, flight) in admitted {
                        flights.insert(position, flight);
                        in_flight.push(Box::pin(self.drive_node(row.clone(), Arc::clone(&snapshot), Arc::clone(&steps), cursor, position)));
                    },
                    Pass::Wait => {}
                }
                self.walks.set_in_flight(run, flights.len());
            }
        }
        if in_flight.is_empty() { return self.settle_walk(run, end).await; }
        tokio::select! {
            biased;
            () = run_control.changed() => {}                      // forwarded at the loop's top
            Some((position, out)) = in_flight.next() => {
                flights.remove(&position);
                self.walks.set_in_flight(run, flights.len());
                let siblings = !flights.is_empty();
                match out {
                    Ok(None) => {}
                    Ok(Some(rest)) if rest.run.is_terminal() => end.terminal(rest, position, &self.walks, run, siblings),
                    Ok(Some(rest)) => end.park(rest),             // drain: `admits()` turns false
                    Err(err) => end.error(err, Some(position), &self.walks, run, siblings),
                }
            }
        }
    }
}
```

`pass` does:
- reads `self.run(run)`;
- on status `Running`, continues;
- on `Queued` with nothing in flight, raises `RunStatus` (as today);
- on any other status with nothing in flight, answers `Rest(row)` (as today);
- on any other status with something in flight, calls `end.drain()`. Some other writer moved the run (F21), so the walk keeps awaiting.

Then:
- `snapshot_of` into an `Arc`;
- `run_steps` into an `Arc<[RunStep]>`;
- `walk::ready_masked(&snapshot, &steps, &flights.keys().copied().collect())`;
- `schedule::plan_pass(..)`.

`WalkEnd` is private to `engine.rs`.
- `admits()` is true while there is no park, no terminal rest and no error.
- `terminal`/`error` record only the first of each. When `siblings` is true they set `cut = true`, remember `ender` (the position's phase name) and call `walks.cut(run)`.
- `park` records the first park.
- `MAX_ITERATIONS` still counts reading passes. A select wake-up that only forwarded a cancel is not a pass.

### 3.5 Cleanup deferral (A4)

```rust
pub async fn cleanup_run(&self, run: RunId) -> Result<(), EngineError> {
    if self.walks.defer_cleanup(run) { return Ok(()); }   // a sibling is live: the walk cleans up when it drains
    self.cleanup_now(run).await                           // today's body, renamed
}
```

`defer_cleanup` is true when a scope exists for `run` with `in_flight > 1`, and it sets `cleanup_owed`. With no scope (a command, the worker's direct call, `cancel_leased`), and with one future, the cleanup runs as today.

### 3.6 `settle_walk`: drain, cut, and what a cut writes (D3, A5, A11)

`settle_walk(run, end)` is called once nothing is in flight and the walk does not admit. It decides in this order.

1. **Any fenced error** (`is_fenced`): raise it. Nothing else is written; `heartbeaten` makes it `LeaseLost`.
2. **Owed cleanup**: when `walks.take_cleanup_owed(run)`, run `cleanup_now(run)`. It was owed by a terminal write that already happened.
3. **The run's own control is a cancel** (`(self.parts.control)(run).signal().is_cancel()`) and no terminal rest was recorded: raise `Err(Cancelled)`. I-6 writes nothing, and `cancel_leased` owns the rows.
4. **Cut writes.** They run when the run (re-read) is terminal **and** (`end.cut` **or** a park preceded the end). Every row of the run at `Pending | Running | AwaitingApproval` goes through `transition_step(id, status, Cancelled, now)`. `Ok(false)` is skipped. Then one `item_note` per moved row, `via_step_id = row`, with body ``step `{phase}` attempt {n} cancelled: the run ended at `{ender}` ``. This is `cancel_leased`'s loop (`:2124-2146`) narrowed to unsettled rows: a `failed` row keeps its failure. A linear walk never sets `cut` and never parks before an end, so it writes nothing here.
5. Answer the **first terminal rest**, else the **first error**, else the **first park rest**, exactly as the future answered it (A11).
6. Every later sibling error is logged at `warn` and dropped. The precedence is `group_error`'s: fenced, then cancel, then the first.

What a cut node meets:
- **Mid-session**: the walk channel carries `Cancel`, `drive` drains and answers `DriverError::Cancelled`, `drive_once` raises `Cancelled`, and `walk_step`/`group_error` write nothing. The row stays `running` until step 4.
- **Between stages** (prepare, verify, reconcile, gate; F19): the future runs on to its next session start, where B-17 raises `Cancelled`, or it returns.
- **A landing that completes after the end** is harmless; the merge is on its branch.
- **A gate park on the terminal run**: `park_step` answers `RunMoved`, the re-read shows the run terminal, and `stale_run` follows. The error is dropped by rule 6 and the row is cancelled by step 4.
- **A `Landing::Retry`'s pending row** is cancelled by step 4.

### 3.7 Park on a parked run (D3, F6)

```rust
/// M3 D3: the run `running -> awaiting_approval`, or already there because a sibling parked it
/// first. Any other status is `stale_run`, as before.
async fn park_the_run(&self, run: RunId, now: DateTime<Utc>) -> Result<(), EngineError> {
    if self.parts.store.transition_run(run, RunStatus::Running, RunStatus::AwaitingApproval, now).await? { return Ok(()); }
    if self.run(run).await?.status == RunStatus::AwaitingApproval { return Ok(()); }
    Err(stale_run(run, RunStatus::Running, RunStatus::AwaitingApproval))
}
```

It replaces `move_run(Running→AwaitingApproval)` in:
- `park_run` (`:5955`);
- `park_selection` (`:5132`);
- `park_interrupted` (`:3208`, F6);
- the scheduler's `RePark`;
- the inner move of `park_on_resume` (`:3540`). Its outer `row.status == Running` test stays, so the resume-window callers make exactly today's calls.

`recover_run`'s D96 (`:2845`) is left as it is.

`gate.rs` gets the twin: `async fn park_the_run(ctx, now)`, over `ctx.store.transition_run` and `ctx.store.run(id)` (both `WorkerStore`).
- `escalate` (`:1332`) calls it.
- `park` (`:530`) on `ParkOutcome::RunMoved` re-reads the run. When it is `AwaitingApproval`: `move_step(ctx, step, Running, AwaitingApproval, now)`, which is `StaleWrite` on `false`, then `Ok(Landing::Rest(AwaitingApproval))`. Otherwise `stale_run`, as today.
- The note was already written by `note_step`/`asks_you_stop` before `park`, so nothing more is written.
- The `:586` `MixedAsksYou` doc is reworded to W5.

**No store trait change.**

### 3.8 The `cursor` callers (V3, A6)

| Caller | M3 |
|---|---|
| `run_to_rest` `:3263` | replaced by `ready_masked` + `plan_pass` |
| `hand_back_resume` `:2534` | `run_resumable(&snapshot, &steps)` (T3) |
| `unblock_case` `:2725` | `run_resumable` (T3) |
| `recover_run` `:2856` (D131) | unchanged. A `Rest{Failed}` that is not first is a candidate of the next walk (`settle_failed` future) |
| `walk_resumed_from` `:3572` | T3: `run_resumable` gates it. T4: `crashed_rejection` read from the first `Rest{Failed}` of `ready`, and the frontier step on `frontiers` |
| `resting` `:7192` | unchanged (display position; may use `position_of`) |
| `views.rs:475` `unblock_case` | `run_resumable` (T3), so the Runs pane and the engine agree |
| `views.rs:744` interrupted row | unchanged: the first `Rest` is display only (MOD-142 owns it). Under A6 the first cursor of a parked run can be a `Decide`; the row then falls back to the run alone, its text unchanged |

### 3.9 Test seams (F14)

`fake.rs`. Counters are always on; they add no awaits, so no existing case changes.

| fn | What |
|---|---|
| `FakeOrchestrator::live_sessions_high_water() -> usize` | Most sessions live at once. A `Counted` wrapper over the boxed driver: its session counts from `start` to drop, and it delegates the four `AgentSession` methods (the `ToolWriting` shape, `fake.rs:1588`). |
| `FakeOrchestrator::interleave_sessions()` | Opt-in. Each wrapped session `yield_now`s once before its first event, so admitted futures interleave. |
| `FakeOrchestrator::hold_sessions(keys: &[(&str, i32)]) -> SessionHold` | The listed sessions wait in `next_event` until released or cancelled (`cancel` releases). `SessionHold::live(n).await` returns once `n` held sessions are live; `release()` frees them. A serial scheduler deadlocks, so cases run under `tokio::time::timeout`. |
| `FakeIsolator::interleave_reconciles()` | Opt-in `yield_now` inside `reconcile`, after the call is recorded. |
| `FakeIsolator::reconcile_high_water() -> u32` | Most `reconcile` calls in progress at once. |
| `FakeIsolator::in_place_overlaps() -> u32` | Times an in-place step's window (`prepare..capture`, or inside `reconcile`) overlapped any other step's window, in either order. Asserted `0` by every concurrent case. |
| `FakeIsolator::call_log() -> Vec<IsolatorCall>` | `Prepare/Capture/Reconcile/Cleanup/Release(step or run)` in order, for the linear call-order case. Its literal is pinned **before** the scheduler lands (TDD). |

`Orchestrate` (`conformance.rs`) gains `hold_sessions`, `live_sessions_high_water` and `interleave_sessions`, with `FakeOrchestrator`'s impl. It is the only impl (`conformance.rs:241`).

The existing seams still apply:
- `ScriptedStep::parks` holds a session on a relay until a cut cancels it;
- `pause_nth_reconcile`/`stall_nth_reconcile` provide D2's window;
- `suspend_after_done` holds a step after its session.

### 3.10 T3 cases

Every new case is a named fn under a new `concurrency_case` dispatcher in `case()`. The plan's thirteen are joined by four from the amendments:
- `a_loop_owed_beside_its_successor_runs_first` (A2/F2: `[Loop{S}, Create{U}]`; `U` never created);
- `an_in_place_node_never_overlaps_a_worktree_node` (A3);
- `a_terminal_branch_defers_cleanup_until_its_sibling_is_cut` (A4: `cleanups() == 1`, after the sibling's row is `cancelled`);
- `one_answer_of_two_parks_re_parks_without_walking_on` (A6: the answered step's successor is not created until the second answer).

The htui-worker case is `an_answer_sent_while_a_walk_drains_waits_then_lands` (`runtime.rs` tests).

Both `CASES` pins move: `tests/fake_conformance.rs:15-41` and `src/conformance.rs:10622-10626`.

---

## 4. T4: recovery among concurrent landings

### 4.1 `recover_run` (D6, A7)

1. Read `steps` once. For each `w` in `recover::frontiers(&snapshot, &steps)`, take `siblings` from `group_at`. The first `Some(rest)` from `Box::pin(self.reconcile_done_step(run, row, &siblings))` returns `Next::Parked`, and the rest wait for the next recovery. The `!any(Running)` guard goes.
2. **A7.** For every `running` row in `(position, attempt, fanout_index)` order, call `recover_step`. Keep the first `Next` and go on. A `Next::Finished` stops adjudication: the remaining unsettled rows are cancelled by the shared writer `cancel_unsettled(run, ender)` from §3.6 step 4. Return the kept `Next`, if any.
3. D96 and D131 are unchanged. With F6, a second park in step 2 no longer raises `StaleWrite`.

### 4.2 `walk_resumed_from`

The gate is `run_resumable` (from T3). `crashed_rejection` is read from the first `Rest{Failed}` cursor of `ready`. The single `frontier` becomes the boxed reconcile of every `frontiers` member, and the first refusal returns its rest. Then `run_to_rest`, as today.

### 4.3 `answer_guarded` (A10)

In the approve tail, before `reconcile_done_step(run, &step, &[])`: read `run_steps` and boxed-reconcile every `frontiers` member `!= step.id` in order. The first `Some(rest)` returns. The answered step is then reconciled exactly as today.

### 4.4 Removal

`recover::frontier` and its unit tests go (A12: `recover.rs` joins T4).

### 4.5 Cases

The plan's six, plus:
- `a_crash_with_two_running_branches_adjudicates_both` (A7);
- `answering_one_of_two_parks_lands_a_crashed_answers_winner` (A10);
- `a_re_run_loop_target_is_re_landed_beside_an_older_side_successor` (A8).

---

## 5. T2 and T5

T2 is as the plan says. The relation in D7 ("neither reaches the other over forward edges") is the same one `Graph::concurrent` computes. T2 builds its own reachability over `forward_mask` in `graph.rs`, since T1 runs in parallel; the duplication is accepted.

T5 is as planned, plus `a_worktree_landing_beside_a_dirty_in_place_checkout_refuses`: a real `dirty_primary_tree`, the A3 evidence.

---

## 6. Interactions (Q6)

| Interaction | Result |
|---|---|
| `Landing::Retry` in a node future | It creates the next attempt's pending row (`admit`) and returns `None`. The node stays masked until its future returns, so it is never admitted twice. The next pass yields `Run` for it, under the same key. |
| `loop_back`/`edge_loop` regions | `region ∩ in_flight ⊆ {source}` by A2 (F2a). |
| `settle_failed` (D131) | A weight-0 `Rest{Failed}` future keyed by its node. Its loop region is covered by A2, its terminal cleanup is deferred by A4, and its `park_interrupted` is tolerant (F6). |
| `drive_group`/`select_stage` | One node future each. `join_all` runs inside, under the walk control. On a cut, `group_error` prefers `Cancelled`, and §3.6 cancels every candidate and judge row. |
| MOD-125 stop/resume | Not wired live (F21). A `stopped` run read mid-walk drains. |
| Run cancel | Forwarded into the walk channel (F18). Every session cancels, `settle_walk` step 3 applies, and nothing is written. |
| MOD-41 `Tails::HandBack` | The answer unparks and hands back. The adopter's `recover_run` lands the frontiers, and D96 re-parks a remaining parked step, which is strict already (F7). |
| `max_agents_per_run` | Resolve-time, counts every node's `fan_out` (V19). Unchanged. |
| 2 MiB stack | Each node future is `Box::pin`ned. `drive_cursor`'s `decide`/`loop_back` stay boxed. Recovery reconciles are boxed. The `run_to_rest` frame shrinks, since the arms leave it. The gate still runs `--no-fail-fast` and greps `SIGABRT`. |
| Lease heartbeat | `walk_leased` races the whole `run_to_rest`. A drain is just a longer walk. An abandon drops the `FuturesUnordered` (crash semantics) and `ScopeGuard` drops the scope. |
| MemStore/PgStore | MemStore's std lock is never held across an await. PgStore uses one pooled connection per in-flight call (F23). The scheduler's reads do not block on row locks (MVCC). |
| A cancel, park or end racing in one poll | Recording is first-wins per kind. Final precedence is §3.6: fenced, cancel, terminal, error, park. |

---

## 7. Build sequence and file sets

T1 ∥ T2 ∥ T5 still holds; the files are pairwise disjoint. They merge in the order T2 → T1 → T5. Then T3, then T4.

| Task | Files |
|---|---|
| T1 | `htui-orch/src/{walk.rs,status.rs,recover.rs}`; `htui-orch/src/engine.rs` (`phase_spec`, tests `:8280`, `:8291` only); `htui-core/src/model/run.rs` |
| T2 | `htui-orch/src/graph.rs`; `htui-core/src/store/{graph_rules.rs,traits.rs,conformance.rs}`; `htui-core/tests/mem_store.rs`; `htui-store/tests/pg_conformance.rs` |
| T5 | `htui-orch/tests/gix_isolator.rs` |
| T3 | `htui-orch/src/{schedule.rs (new),lib.rs,engine.rs,gate.rs,fake.rs,conformance.rs}`; `htui-orch/tests/fake_conformance.rs`; `htui-worker/src/{views.rs,runtime.rs (tests)}` |
| T4 | `htui-orch/src/{engine.rs,recover.rs,conformance.rs,fake.rs}`; `htui-orch/tests/fake_conformance.rs` |

TDD order inside T3:
1. pin `call_log` on an existing linear case;
2. `schedule.rs` with its unit tests;
3. `WalkScopes` and `session_control`;
4. `drive_node`/`drive_cursor` extracted with the loop still serial, then green on the 140;
5. the loop;
6. the cut and drain cases;
7. park-on-parked;
8. A6.

---

## 8. Risks added

| Risk | Mitigation |
|---|---|
| A6 rejected: the in-place last-lander proof (D6, A9) no longer holds | Keep A6. Otherwise frontiers must re-land every in-place winner and accept `primary_moved` parks. |
| The fake hides concurrency bugs (F14) | Opt-in pending points; `hold_sessions` under `timeout`. T5 carries the real lock. |
| The cut leaves rows when the run is not terminal (an `Err` such as `StaleWrite`) | The sweep adjudicates every running row (A7). |
| The PgStore pool is exhausted (F23) | `max_parallel_sessions + 2` per walk. A note for MOD-139's box-level cap. |

---

## 9. Plan amendments needed

- **A1** (D1, D2): the node key is the position, `in_flight: &BTreeSet<i32>`. Node futures own their inputs (`Run`, `Arc<GraphSnapshot>`, `Arc<[RunStep]>`) through one `drive_node` async fn.
- **A2** (D1): a candidate is admitted only when incomparable with every in-flight node over forward edges. This fixes M2's `[Loop, Create]` and `[Decide, Create]` passes, proves loop regions never touch an in-flight node, and makes linear parity hold by construction.
- **A3** (D4): an in-place phase's tree-touching cursors (`Run`/`Fan`/`Select`) are exclusive against every tree-touching future, of any mode. A worktree landing merges into, and refuses on, the checkout an in-place agent edits.
- **A4** (D3): `cleanup_run` defers while a sibling is in flight, and the walk runs it once the set drains. It is immediate when alone, so linear parity holds.
- **A5** (D3): a cut is a walk-scoped `watch` (`WalkScopes`) that `drive_once` drives under. Inside a future it surfaces as `Cancelled` (I-6, nothing written), and the scheduler then cancels every unsettled row with the note. This replaces "its own arm".
- **A6** (D1, D3): the strict park. Nothing is admitted while any step awaits; the re-park fires on any awaiting row when nothing is in flight. `status::run_resumable` (every `ready` cursor resumable) replaces first-cursor `resumable` in the four callers, `views.rs` included.
- **A7** (D6, T4): `recover_run` adjudicates every running row: a park is kept and the loop goes on, and a terminal outcome cancels the rest.
- **A8** (Q-1 detail): a successor row proves its predecessor landed only if `started_at >= winner.finished_at`. Unstarted pending rows prove nothing, and an idempotent re-land follows.
- **A9** (D6): an in-place frontier member is kept only when no done winner of the run, in any mode, finished later. This covers `worktree` merges and `local`'s silent mis-recording of `after_hash`.
- **A10** (T4): `answer_guarded` first lands every other frontier member. A crashed answer's winner is otherwise skipped when a second park is answered. The set is empty on a linear graph.
- **A11** (D3): a drained walk returns the first park's `Rest` as answered, keeping its `failure`, not `resting()`. A terminal rest wins over a park.
- **A12** (files): T4 owns `recover.rs` to delete `frontier`. T3 gains `schedule.rs` (new), `lib.rs` and `htui-worker/src/views.rs`.
