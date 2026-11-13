//! MOD-138 M3 (plan D1, D4; blueprint §3.3, A2, A3, A6): which of a pass's cursors the concurrent
//! walk admits. Pure: the engine's `run_to_rest` reads the run, its rows and `ready_masked`, and
//! asks [`plan_pass`] what to do with them.
//!
//! The rules, in order:
//! 1. **Blocking rests (A6).** Nothing is admitted while any cursor is a `Rest` at
//!    `awaiting_approval`, `running` or `stopped`: a parked step, or a session this walk does not
//!    own. With nothing in flight the pass re-parks a `running` run over a parked step
//!    ([`PassPlan::RePark`], V20) or rests ([`PassPlan::Rest`]).
//! 2. `[Finished]` finishes the run once nothing is in flight.
//! 3. Every other cursor is a candidate, in `ready` order, admitted when all of these hold:
//!    - (a) its node is not taken already (F16: one future per node);
//!    - (b) A2: its node is incomparable with every taken node (neither reaches the other over
//!      forward edges), so a chain never has two futures in flight and a loop's region never
//!      meets one (F2, F2a);
//!    - (c) A3: an in-place (`local`, `shared_serialized`) tree-touching cursor runs alone among
//!      the tree-touching ones, both ways, and (T1 verify) not while another node's `Select` is
//!      owed: that selection lands first, so the in-place frontier rule sees its landing;
//!    - (d) the weight cap: the taken weight plus its own is at most `max_parallel_sessions`
//!      (`0` reads as `1`), or nothing is taken (a node heavier than the cap runs alone);
//!    - (e) R1, "order by inputs" (maintainer, 2026-11-08): a cursor that starts sessions
//!      (`Create`, `Run`, `Fan`, MOD-139 M2's `Parts`) waits while its node lists a kind that an incomparable,
//!      unfinished node produces ([`crate::walk::input_waits`]: in flight, or neither done nor
//!      skipped; two-way, the lower position goes first). Without it (b) would admit both, and
//!      the consumer's stage 3 would meet `MissingInput` or read an older run's document.
//!
//!    - (f) MOD-139 M2 D2: a cursor that admits a plan (a `Run` on a planner row) is not admitted
//!      beside another in flight or taken. Each reads the run's rows and plans for its revision and
//!      `base_position` before its own `admit_plan`, so two at once would claim the same slots on
//!      a store whose reads yield, and the store would refuse the second.
//!
//!    **No deadlock (R1).** When nothing is in flight and nothing was admitted, but a candidate
//!    passed (a) to (d) and waits only on (e), the waits form a cycle among incomparable
//!    nodes: the lowest-position such candidate is admitted alone. Its stage 3 reads the kind as
//!    optional ([`crate::walk::InputNeed::Concurrent`]). No graph is refused for such a cycle.
//!
//! **Linear parity by construction.** On a graph whose walked nodes are totally ordered by
//! forward reachability, (b) admits at most one candidate per pass and none while one is in
//! flight, and that candidate is `ready()[0]`, today's `cursor()`. A chain has no incomparable
//! pair, so (e) and its fallback never apply.

use std::collections::BTreeMap;

use htui_core::model::{
    GraphSnapshot, Isolation, RunStatus, RunStep, SnapshotPhase, StepRole, StepStatus,
};

use crate::status::{Cursor, position_of};
use crate::walk::Graph;

/// A future's admission facts, kept while it is in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Flight {
    /// The sessions it can start: `fan_out` for `Run`/`Create`/`Fan`, `1` for a judged `Select`,
    /// the round's width for `Parts` (MOD-139 M2 D4), else `0`.
    pub weight: u32,
    /// Whether it works in its node's trees: `Run`, `Fan`, `Select` or `Parts`.
    pub touches: bool,
    /// Whether its node's trees are the primary checkout (`local`, `shared_serialized`).
    pub in_place: bool,
    /// Whether it admits a plan: a `Run` on a planner row (MOD-139 M2 D2), rule (f).
    pub admits: bool,
}

/// What one pass of the concurrent walk does (blueprint §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PassPlan {
    /// Push these, in order: the cursor, its node's position, and its flight.
    Admit(Vec<(Cursor, i32, Flight)>),
    /// Nothing to admit; something is in flight.
    Wait,
    /// Nothing in flight and `ready == [Finished]`: finish the run `done`.
    Finished,
    /// Nothing in flight; the run is `running` over an `awaiting_approval` step (V20, A6).
    RePark,
    /// Nothing in flight; answer `resting` (a stray `running`/`stopped` row, or a parked step).
    Rest,
}

/// Blueprint §3.3: the pass's answer over `cursors` (`ready_masked` over `in_flight`'s keys),
/// the run's `status` and the futures `in_flight`, by node position.
pub(crate) fn plan_pass(
    snapshot: &GraphSnapshot,
    steps: &[RunStep],
    status: RunStatus,
    cursors: &[Cursor],
    in_flight: &BTreeMap<i32, Flight>,
) -> PassPlan {
    let idle = in_flight.is_empty();
    // Rule 1 (A6): a parked step, or a session this walk does not own, admits nothing.
    let blocking = |wanted: &[StepStatus]| {
        cursors
            .iter()
            .any(|cursor| matches!(cursor, Cursor::Rest { status, .. } if wanted.contains(status)))
    };
    if blocking(&[
        StepStatus::AwaitingApproval,
        StepStatus::Running,
        StepStatus::Stopped,
    ]) {
        return if !idle {
            PassPlan::Wait
        } else if status == RunStatus::Running && blocking(&[StepStatus::AwaitingApproval]) {
            PassPlan::RePark
        } else {
            PassPlan::Rest
        };
    }
    // Rule 2.
    if matches!(cursors, [Cursor::Finished]) {
        return if idle {
            PassPlan::Finished
        } else {
            PassPlan::Wait
        };
    }

    // Rule 3.
    let graph = Graph::of(snapshot);
    let cap = snapshot.settings.max_parallel_sessions.max(1);
    let mut taken: Vec<(i32, Flight)> = in_flight
        .iter()
        .map(|(position, flight)| (*position, *flight))
        .collect();
    let mut admitted = Vec::new();
    // (e) R1: the producers each node waits for, read once per pass.
    let waits = crate::walk::input_waits(snapshot, steps, &in_flight.keys().copied().collect());
    let mut held = Vec::new();
    for cursor in cursors {
        let Some((position, flight)) = candidate(snapshot, steps, cursor) else {
            continue;
        };
        // (a) F16: one future per node.
        if taken.iter().any(|(held, _)| *held == position) {
            continue;
        }
        // (b) A2: incomparable with every taken node.
        if !taken
            .iter()
            .all(|(held, _)| graph.concurrent(position, *held))
        {
            continue;
        }
        // (c) A3: an in-place tree is exclusive among the tree-touching futures, both ways.
        if flight.touches
            && taken
                .iter()
                .any(|(_, held)| held.touches && (flight.in_place || held.in_place))
        {
            continue;
        }
        // (c) T1 verify: an owed selection lands before an in-place node touches its trees.
        if flight.touches
            && flight.in_place
            && cursors.iter().any(
                |other| matches!(other, Cursor::Select { position: at, .. } if *at != position),
            )
        {
            continue;
        }
        // (d) The weight cap; a node heavier than the cap runs alone.
        let weight: u32 = taken.iter().map(|(_, held)| held.weight).sum();
        if !taken.is_empty() && weight.saturating_add(flight.weight) > cap {
            continue;
        }
        // (f) MOD-139 M2 D2: one plan admission at a time.
        if flight.admits && taken.iter().any(|(_, held)| held.admits) {
            continue;
        }
        // (e) R1: a node that starts sessions waits for the incomparable producers of its inputs.
        if matches!(
            cursor,
            Cursor::Create { .. } | Cursor::Run(_) | Cursor::Fan { .. } | Cursor::Parts { .. }
        ) && waits.contains_key(&position)
        {
            held.push((cursor.clone(), position, flight));
            continue;
        }
        taken.push((position, flight));
        admitted.push((cursor.clone(), position, flight));
    }
    match (admitted.is_empty(), idle) {
        (false, _) => PassPlan::Admit(admitted),
        (true, false) => PassPlan::Wait,
        // Rule 3's fallback (R1): every candidate waits on (e) with nothing in flight, so the waits form a
        // cycle; the lowest position goes first. It passed (a) to (d) against nothing taken.
        (true, true) => held
            .into_iter()
            .min_by_key(|(_, position, _)| *position)
            .map_or(PassPlan::Rest, |first| PassPlan::Admit(vec![first])),
        // Otherwise unreachable: the first candidate passes (a) to (d) against nothing taken. A
        // pass with no candidate at all (every cursor names a row the pass does not hold) rests.
    }
}

/// `cursor`'s node position and flight, or `None` for a cursor no future runs (`Finished`, a
/// non-`failed` `Rest`) or one whose step or phase the pass does not hold.
fn candidate(
    snapshot: &GraphSnapshot,
    steps: &[RunStep],
    cursor: &Cursor,
) -> Option<(i32, Flight)> {
    let position = position_of(cursor, steps)?;
    let phase: &SnapshotPhase = snapshot
        .phases
        .iter()
        .find(|phase| phase.position == position)?;
    let fan_out = u32::try_from(phase.fan_out.max(1)).unwrap_or(1);
    let (weight, touches) = match cursor {
        // MOD-125 M3 D4: a stopped step resumed weighs and touches as the run it was.
        Cursor::Run(_) | Cursor::Resume(_) | Cursor::Fan { .. } => (fan_out, true),
        Cursor::Create { .. } => (fan_out, false),
        Cursor::Select { .. } => (u32::from(phase.judge.is_some()), true),
        // MOD-139 M2 D4: a divided node's parts round, weighed at the parts it runs at once.
        Cursor::Parts { .. } => (crate::divide::parts_width(snapshot, steps, phase), true),
        Cursor::Decide { .. }
        | Cursor::Loop { .. }
        | Cursor::Rest {
            status: StepStatus::Failed,
            ..
        } => (0, false),
        Cursor::Rest { .. } | Cursor::Finished => return None,
    };
    let admits = matches!(cursor, Cursor::Run(id) | Cursor::Resume(id) if steps
        .iter()
        .any(|step| step.id == *id && step.role == StepRole::Planner));
    Some((
        position,
        Flight {
            weight,
            touches,
            in_place: matches!(
                phase.isolation,
                Isolation::Local | Isolation::SharedSerialized
            ),
            admits,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use htui_core::model::{
        AgentId, GraphSnapshot, Isolation, RunStatus, RunStep, SnapshotJudge, StepId, StepRole,
        StepStatus,
    };

    use super::{Flight, PassPlan, plan_pass};
    use crate::status::Cursor;
    use crate::walk::tests::{NEXT, create, diamond, done, graph, row};

    /// Three independent roots `a`, `b`, `c`, every one `worktree` with `fan_out` 1.
    fn three() -> GraphSnapshot {
        let mut snapshot = graph(&[("a", 0), ("b", 1), ("c", 2)], &[]);
        for phase in &mut snapshot.phases {
            phase.isolation = Isolation::Worktree;
        }
        snapshot
    }

    fn admitted(plan: &PassPlan) -> Vec<i32> {
        match plan {
            PassPlan::Admit(admitted) => {
                admitted.iter().map(|(_, position, _)| *position).collect()
            }
            other => panic!("expected an admission, got {other:?}"),
        }
    }

    fn flight(weight: u32, touches: bool, in_place: bool) -> Flight {
        Flight {
            weight,
            touches,
            in_place,
            admits: false,
        }
    }

    fn plan(snapshot: &GraphSnapshot, cursors: &[Cursor], in_flight: &[(i32, Flight)]) -> PassPlan {
        plan_pass(
            snapshot,
            &[],
            RunStatus::Running,
            cursors,
            &in_flight.iter().copied().collect::<BTreeMap<_, _>>(),
        )
    }

    /// D1: every incomparable ready node is admitted up to the cap, in `ready` order.
    #[test]
    fn the_cap_bounds_the_admitted_weight() {
        let mut snapshot = three();
        snapshot.settings.max_parallel_sessions = 2;
        let cursors = [create(0), create(1), create(2)];
        assert_eq!(admitted(&plan(&snapshot, &cursors, &[])), [0, 1]);
        assert_eq!(
            plan(
                &snapshot,
                &cursors[2..],
                &[(0, flight(1, true, false)), (1, flight(1, true, false))]
            ),
            PassPlan::Wait,
            "two in flight fill a cap of two"
        );
        assert_eq!(
            admitted(&plan(
                &snapshot,
                &cursors[2..],
                &[(0, flight(1, true, false))]
            )),
            [2]
        );
    }

    /// D1: a frozen cap of 0 behaves as 1.
    #[test]
    fn a_cap_of_zero_reads_as_one() {
        let mut snapshot = three();
        snapshot.settings.max_parallel_sessions = 0;
        let cursors = [create(0), create(1)];
        assert_eq!(admitted(&plan(&snapshot, &cursors, &[])), [0]);
        assert_eq!(
            plan(&snapshot, &cursors[1..], &[(0, flight(1, false, false))]),
            PassPlan::Wait
        );
    }

    /// D1: a node heavier than the cap runs alone, and nothing joins it.
    #[test]
    fn a_heavy_node_runs_alone() {
        let mut snapshot = three();
        snapshot.settings.max_parallel_sessions = 2;
        snapshot.phases[0].fan_out = 3;
        let cursors = [create(0), create(1)];
        assert_eq!(admitted(&plan(&snapshot, &cursors, &[])), [0]);
        assert_eq!(
            plan(&snapshot, &cursors[1..], &[(0, flight(3, false, false))]),
            PassPlan::Wait
        );
        assert_eq!(
            plan(&snapshot, &cursors[..1], &[(1, flight(1, true, false))]),
            PassPlan::Wait,
            "beside `b` in flight, `a` would exceed the cap"
        );
    }

    /// A3: an in-place tree-touching cursor excludes every tree-touching one, both ways; a
    /// `Create` touches nothing and is not excluded.
    #[test]
    fn an_in_place_tree_is_exclusive_both_ways() {
        let mut snapshot = three();
        snapshot.phases[0].isolation = Isolation::SharedSerialized;
        let a = row(&snapshot, "a", 1, StepStatus::Pending);
        let b = row(&snapshot, "b", 1, StepStatus::Pending);
        let steps = [a.clone(), b.clone()];
        let at = |cursors: &[Cursor], in_flight: &[(i32, Flight)]| {
            plan_pass(
                &snapshot,
                &steps,
                RunStatus::Running,
                cursors,
                &in_flight.iter().copied().collect(),
            )
        };
        assert_eq!(
            admitted(&at(&[Cursor::Run(a.id), Cursor::Run(b.id)], &[])),
            [0]
        );
        assert_eq!(
            at(&[Cursor::Run(b.id)], &[(0, flight(1, true, true))]),
            PassPlan::Wait,
            "a worktree run waits on an in-place run"
        );
        assert_eq!(
            at(&[Cursor::Run(a.id)], &[(1, flight(1, true, false))]),
            PassPlan::Wait,
            "an in-place run waits on a worktree run"
        );
        assert_eq!(
            admitted(&at(
                &[Cursor::Run(a.id), create(2)],
                &[(1, flight(1, true, false))]
            )),
            [2],
            "a `Create` touches no tree"
        );
        assert_eq!(
            admitted(&at(&[Cursor::Run(b.id)], &[(0, flight(1, false, true))])),
            [1],
            "an in-place `Create` in flight excludes nothing"
        );
    }

    /// T1 verify: an in-place node does not touch its trees while another node's selection is
    /// owed; the selection is admitted instead.
    #[test]
    fn an_owed_selection_lands_before_an_in_place_node() {
        let mut snapshot = three();
        snapshot.phases[0].isolation = Isolation::SharedSerialized;
        snapshot.phases[1].fan_out = 2;
        let a = row(&snapshot, "a", 1, StepStatus::Pending);
        let select = Cursor::Select {
            position: 1,
            attempt: 1,
        };
        assert_eq!(
            admitted(&plan_pass(
                &snapshot,
                std::slice::from_ref(&a),
                RunStatus::Running,
                &[Cursor::Run(a.id), select.clone()],
                &BTreeMap::new(),
            )),
            [1]
        );
        assert_eq!(
            admitted(&plan_pass(
                &snapshot,
                std::slice::from_ref(&a),
                RunStatus::Running,
                &[Cursor::Run(a.id), create(2)],
                &BTreeMap::new(),
            )),
            [0, 2],
            "with no selection owed the in-place run goes first"
        );
    }

    /// A2: a candidate comparable with a taken node waits; F16: one future per node.
    #[test]
    fn comparable_nodes_and_one_future_per_node() {
        let snapshot = diamond();
        let a = done(&snapshot, "a");
        let b = row(&snapshot, "b", 1, StepStatus::Pending);
        let steps = [a.clone(), b.clone()];
        let at = |cursors: &[Cursor], in_flight: &[(i32, Flight)]| {
            plan_pass(
                &snapshot,
                &steps,
                RunStatus::Running,
                cursors,
                &in_flight.iter().copied().collect(),
            )
        };
        // `a` reaches `d`: a `Decide` owed on `a` beside `d`'s `Create`.
        assert_eq!(
            admitted(&at(&[Cursor::Decide { step: a.id }, create(3)], &[])),
            [0],
            "`d` follows `a`"
        );
        assert_eq!(
            admitted(&at(&[Cursor::Run(b.id), create(2)], &[])),
            [1, 2],
            "`b` and `c` are incomparable"
        );
        assert_eq!(
            admitted(&at(
                &[
                    Cursor::Decide { step: a.id },
                    Cursor::Loop {
                        step: a.id,
                        to: "b".to_owned()
                    }
                ],
                &[]
            )),
            [0],
            "one candidate per node"
        );
        assert_eq!(
            at(&[create(3)], &[(1, flight(1, true, false))]),
            PassPlan::Wait,
            "`d` waits on `b` in flight"
        );
    }

    /// A chain admits one future at a time: the first cursor, as the serial walk did.
    #[test]
    fn a_chain_admits_one_at_a_time() {
        let snapshot = graph(&[("a", 0), ("b", 1)], &[("a", "b", NEXT)]);
        let a = done(&snapshot, "a");
        assert_eq!(
            admitted(&plan_pass(
                &snapshot,
                std::slice::from_ref(&a),
                RunStatus::Running,
                &[
                    Cursor::Loop {
                        step: a.id,
                        to: "a".to_owned()
                    },
                    create(1)
                ],
                &BTreeMap::new(),
            )),
            [0],
            "F2: the owed loop runs first and `b` waits"
        );
    }

    /// R1 (e): a consumer waits for an incomparable producer of its inputs until it finished; the
    /// reversed direction waits too; a two-way pair puts the lower position first.
    #[test]
    fn a_consumer_waits_for_its_incomparable_producer() {
        let base = diamond();
        let a = done(&base, "a");
        let steps = [a.clone()];
        let at = |snapshot: &GraphSnapshot,
                  steps: &[RunStep],
                  cursors: &[Cursor],
                  in_flight: &[(i32, Flight)]| {
            plan_pass(
                snapshot,
                steps,
                RunStatus::Running,
                cursors,
                &in_flight.iter().copied().collect(),
            )
        };
        let mut snapshot = base.clone();
        snapshot.phases[2].input_kinds = vec!["b".into()];
        assert_eq!(
            admitted(&at(&snapshot, &steps, &[create(1), create(2)], &[])),
            [1],
            "`c` lists `b`'s kind"
        );
        assert_eq!(
            at(
                &snapshot,
                &steps,
                &[create(2)],
                &[(1, flight(1, true, false))]
            ),
            PassPlan::Wait,
            "`b` in flight is unfinished"
        );
        let b = done(&snapshot, "b");
        assert_eq!(
            admitted(&at(&snapshot, &[a.clone(), b], &[create(2)], &[])),
            [2],
            "once `b` is done `c` runs"
        );

        let mut reversed = base.clone();
        reversed.phases[1].input_kinds = vec!["c".into()];
        assert_eq!(
            admitted(&at(&reversed, &steps, &[create(1), create(2)], &[])),
            [2]
        );

        let mut mutual = base;
        mutual.phases[1].input_kinds = vec!["c".into()];
        mutual.phases[2].input_kinds = vec!["b".into()];
        assert_eq!(
            admitted(&at(&mutual, &steps, &[create(1), create(2)], &[])),
            [1],
            "two-way: the lower position goes first"
        );
        assert_eq!(
            admitted(&at(
                &mutual,
                &steps,
                &[Cursor::Decide { step: a.id }, create(2)],
                &[]
            )),
            [0],
            "a write the engine owes never waits on inputs"
        );
    }

    /// R1, rule 3's fallback: a cycle of input waits among incomparable nodes admits its lowest position
    /// alone once nothing is in flight, and then orders the rest after it.
    #[test]
    fn a_cycle_of_input_waits_admits_its_lowest_position() {
        let mut snapshot = three();
        snapshot.phases[0].input_kinds = vec!["b".into()];
        snapshot.phases[1].input_kinds = vec!["c".into()];
        snapshot.phases[2].input_kinds = vec!["a".into()];
        let cursors = [create(0), create(1), create(2)];
        assert_eq!(admitted(&plan(&snapshot, &cursors, &[])), [0]);
        assert_eq!(
            plan(&snapshot, &cursors[1..], &[(0, flight(1, true, false))]),
            PassPlan::Wait,
            "`c` waits for `a` in flight, `b` for `c`"
        );
        let a = done(&snapshot, "a");
        assert_eq!(
            admitted(&plan_pass(
                &snapshot,
                std::slice::from_ref(&a),
                RunStatus::Running,
                &cursors[1..],
                &BTreeMap::new(),
            )),
            [2],
            "then `c`, and `b` after it"
        );
    }

    /// Rules 1, 2 and 4: the three no-admission answers.
    #[test]
    fn the_no_admission_answers() {
        let snapshot = three();
        let parked = Cursor::Rest {
            step: StepId::new(),
            status: StepStatus::AwaitingApproval,
        };
        let running = Cursor::Rest {
            step: StepId::new(),
            status: StepStatus::Running,
        };
        let busy = [(1, flight(1, true, false))];
        assert_eq!(
            plan(&snapshot, &[parked.clone(), create(2)], &[]),
            PassPlan::RePark
        );
        assert_eq!(
            plan_pass(
                &snapshot,
                &[],
                RunStatus::AwaitingApproval,
                std::slice::from_ref(&parked),
                &BTreeMap::new()
            ),
            PassPlan::Rest,
            "a parked run is not parked again"
        );
        assert_eq!(plan(&snapshot, &[parked, create(2)], &busy), PassPlan::Wait);
        assert_eq!(
            plan(&snapshot, std::slice::from_ref(&running), &[]),
            PassPlan::Rest
        );
        assert_eq!(plan(&snapshot, &[running], &busy), PassPlan::Wait);
        assert_eq!(
            plan(&snapshot, &[Cursor::Finished], &[]),
            PassPlan::Finished
        );
        assert_eq!(
            plan(&snapshot, &[], &busy),
            PassPlan::Wait,
            "a masked pass waits"
        );
        assert_eq!(plan(&snapshot, &[], &[]), PassPlan::Rest);
    }

    /// The weights: `fan_out` for a node's sessions, `1` for a judged selection, `0` for writes.
    #[test]
    fn the_weights() {
        let mut snapshot = three();
        snapshot.phases[0].fan_out = 3;
        snapshot.phases[0].judge = Some(SnapshotJudge {
            agent_id: AgentId::new(),
            agent_name: "judge".to_owned(),
            model: None,
            template: None,
        });
        let a = done(&snapshot, "a");
        let weight = |cursor: Cursor| match plan_pass(
            &snapshot,
            std::slice::from_ref(&a),
            RunStatus::Running,
            &[cursor],
            &BTreeMap::new(),
        ) {
            PassPlan::Admit(admitted) => admitted[0].2.weight,
            other => panic!("{other:?}"),
        };
        assert_eq!(weight(create(0)), 3);
        assert_eq!(
            weight(Cursor::Fan {
                position: 0,
                attempt: 1
            }),
            3
        );
        assert_eq!(
            weight(Cursor::Select {
                position: 0,
                attempt: 1
            }),
            1
        );
        assert_eq!(weight(Cursor::Decide { step: a.id }), 0);
        assert_eq!(
            weight(Cursor::Rest {
                step: a.id,
                status: StepStatus::Failed
            }),
            0
        );
        assert_eq!(weight(create(1)), 1);
    }

    /// The diamond with `c` a divided node's planner whose row is `done`, holding `parts` pending
    /// parts at 5.., and the rows: `a` done, the planner, its parts.
    fn parted(parts: i32) -> (GraphSnapshot, Vec<RunStep>) {
        let mut snapshot = diamond();
        snapshot.phases[2].role = htui_core::model::NodeRole::Planner;
        snapshot.phases[2].isolation = Isolation::Worktree;
        snapshot.phases[1].isolation = Isolation::Worktree;
        let a = done(&snapshot, "a");
        let mut planner = done(&snapshot, "c");
        planner.role = StepRole::Planner;
        let mut steps = vec![a, planner.clone()];
        for position in 5..5 + parts {
            let mut part = row(&snapshot, "c", 1, StepStatus::Pending);
            part.position = position;
            part.role = StepRole::Part;
            part.parent_step_id = Some(planner.id);
            steps.push(part);
        }
        (snapshot, steps)
    }

    /// MOD-139 M2 D4: a parts round touches its trees and weighs its width, the live parts
    /// under the session cap.
    #[test]
    fn a_parts_cursor_weighs_its_width_and_touches() {
        let (mut snapshot, steps) = parted(3);
        snapshot.settings.max_parallel_sessions = 2;
        let parts = Cursor::Parts { position: 2 };
        let PassPlan::Admit(admitted) = plan_pass(
            &snapshot,
            &steps,
            RunStatus::Running,
            std::slice::from_ref(&parts),
            &BTreeMap::new(),
        ) else {
            panic!("a parts round is admitted");
        };
        assert_eq!(admitted, [(parts.clone(), 2, flight(2, true, false))]);
        assert_eq!(
            plan_pass(
                &snapshot,
                &steps,
                RunStatus::Running,
                &[create(1), parts],
                &[(2, flight(2, true, false))].into_iter().collect(),
            ),
            PassPlan::Wait,
            "the cap is full while the round is in flight"
        );
    }

    /// MOD-139 M2 D4 (rule (e)): a parts round starts sessions, so it waits on the incomparable
    /// producer of its inputs like a `Run`.
    #[test]
    fn a_parts_cursor_waits_on_its_inputs_like_a_run() {
        let (mut snapshot, steps) = parted(1);
        snapshot.phases[2].input_kinds = vec!["b".into()];
        let parts = Cursor::Parts { position: 2 };
        assert_eq!(
            admitted(&plan_pass(
                &snapshot,
                &steps,
                RunStatus::Running,
                &[create(1), parts.clone()],
                &BTreeMap::new(),
            )),
            [1],
            "`c`'s parts list `b`'s kind"
        );
        assert_eq!(
            plan_pass(
                &snapshot,
                &steps,
                RunStatus::Running,
                std::slice::from_ref(&parts),
                &[(1, flight(1, true, false))].into_iter().collect(),
            ),
            PassPlan::Wait,
            "`b` in flight is unfinished"
        );
    }

    /// MOD-139 M2 D2 (rule (f), T3 repair): two planners on parallel branches are admitted one at
    /// a time. Each reads the run's rows and plans for its revision and `base_position` before
    /// its own `admit_plan`, so two in flight would claim the same slots on a store that yields.
    #[test]
    fn planners_admit_one_at_a_time() {
        let mut snapshot = diamond();
        for phase in &mut snapshot.phases {
            phase.isolation = Isolation::Worktree;
        }
        snapshot.phases[1].role = htui_core::model::NodeRole::Planner;
        snapshot.phases[2].role = htui_core::model::NodeRole::Planner;
        let a = done(&snapshot, "a");
        let mut b = row(&snapshot, "b", 1, StepStatus::Pending);
        b.role = StepRole::Planner;
        let mut c = row(&snapshot, "c", 1, StepStatus::Pending);
        c.role = StepRole::Planner;
        let steps = [a, b.clone(), c.clone()];
        let at = |steps: &[RunStep], cursors: &[Cursor], in_flight: &[(i32, Flight)]| {
            plan_pass(
                &snapshot,
                steps,
                RunStatus::Running,
                cursors,
                &in_flight.iter().copied().collect(),
            )
        };
        let PassPlan::Admit(first) = at(&steps, &[Cursor::Run(b.id), Cursor::Run(c.id)], &[])
        else {
            panic!("a planner is admitted");
        };
        assert_eq!(
            first,
            [(
                Cursor::Run(b.id),
                1,
                Flight {
                    admits: true,
                    ..flight(1, true, false)
                }
            )],
            "the second planner waits for the first's admission"
        );
        let admitting = Flight {
            admits: true,
            ..flight(1, true, false)
        };
        assert_eq!(
            at(&steps, &[Cursor::Run(c.id)], &[(1, admitting)]),
            PassPlan::Wait,
            "a planner waits on a planner in flight"
        );
        assert_eq!(
            admitted(&at(
                &steps,
                &[Cursor::Run(c.id)],
                &[(1, flight(1, true, false))]
            )),
            [2],
            "beside a plain run a planner is admitted"
        );
        let mut plain = c;
        plain.role = StepRole::Step;
        assert_eq!(
            admitted(&at(
                &[b.clone(), plain.clone()],
                &[Cursor::Run(plain.id)],
                &[(1, admitting)]
            )),
            [2],
            "beside a planner a plain run is admitted"
        );
    }
}
