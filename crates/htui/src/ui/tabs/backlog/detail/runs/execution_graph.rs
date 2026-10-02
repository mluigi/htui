//! MOD-28: the Runs pane's flow view, the run under the cursor drawn as a `rataflow` graph.
//!
//! A pure projection of one `RunSummary` (ANA-12 invariant 1). [`project`] lays the steps out in
//! layers (plan D6), and `ExecutionGraph` owns the `Flow` that draws them. The flow keeps no
//! selection of its own: the cursor step is rebuilt selected on every sync (plan D7), and no key
//! ever reaches rataflow's own bindings (blueprint H-1).

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "MOD-28 T3 wires the flow view into RunsTab")
)]

use htui_core::model::{RunStepSummary, RunSummary, StepId};

/// Plan D10: a node's width in cells at zoom 1. Two candidates and the gap are 41 columns, which a
/// reveal's 1-cell margins keep inside the 43-column pane (fact-check R8; 21 cuts a border).
pub(super) const NODE_W: f64 = 20.0;
/// Plan D10: a node's height, a border, two lines and a border.
pub(super) const NODE_H: f64 = 4.0;
/// Plan D10: the columns between two nodes of one layer.
pub(super) const H_GAP: f64 = 1.0;
/// Plan D10: the rows between two layers. Three keep a `retry` label off the arrowhead (R7).
pub(super) const V_GAP: f64 = 3.0;
/// The x distance from one node of a layer to the next.
const X_STEP: f64 = NODE_W + H_GAP;
/// The y distance from one layer to the next.
const Y_STEP: f64 = NODE_H + V_GAP;
/// Plan D6: the label of an edge into the next attempt of the same position.
pub(super) const RETRY: &str = "retry";

/// One row of the flow (plan D6): the candidates of a `(position, attempt)`, or its judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layer {
    /// `run_step.position` of every step in it.
    pub(super) position: i32,
    /// `run_step.attempt` of every step in it.
    pub(super) attempt: i32,
    /// Whether this is the judge layer (`fanout_index < 0`) of its `(position, attempt)`.
    pub(super) judge: bool,
    /// Its steps, left to right: by `fanout_index`, then id.
    pub(super) steps: Vec<StepId>,
    /// The step marked `selected == Some(true)`, if any (`run.rs:739` is `Option<bool>`).
    pub(super) winner: Option<StepId>,
}

/// What [`project`] makes of a run.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Projection {
    /// The layers, top to bottom.
    pub(super) layers: Vec<Layer>,
    /// `(source, target, label)`, in layer order and then left to right.
    pub(super) edges: Vec<(StepId, StepId, Option<&'static str>)>,
    /// Every node's top-left corner in world units, in layer order and then left to right.
    pub(super) positions: Vec<(StepId, (f64, f64))>,
    /// The widest layer's width in world units, `0.0` for a run with no step (blueprint B-7).
    pub(super) width: f64,
}

/// Plan D6: a run's steps grouped into layers, each id once (blueprint B-5, H-4).
pub(super) fn layers(steps: &[RunStepSummary]) -> Vec<Layer> {
    let _ = steps;
    todo!("MOD-28 T1")
}

/// Plan D6: a run as layers, edges and positions. Rataflow-free, so T1 tests it alone.
pub(super) fn project(run: &RunSummary) -> Projection {
    let _ = (run, X_STEP, Y_STEP, RETRY, V_GAP);
    todo!("MOD-28 T1")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use htui_core::fixtures::demo_at;
    use htui_core::model::{BoxId, ProjectId, RunId, RunKind, RunMode, RunStatus, StepStatus};
    use uuid::Uuid;

    use super::*;

    /// A settled step at `(position, attempt, fanout_index)`, id `0x100 + n` (the `htui-orch`
    /// `closeout.rs` idiom).
    fn step(n: u128, position: i32, attempt: i32, fanout_index: i32) -> RunStepSummary {
        RunStepSummary {
            id: id(n),
            position,
            attempt,
            fanout_index,
            phase_name: "plan".to_owned(),
            agent_id: None,
            model: None,
            status: StepStatus::Done,
            gate_outcome: None,
            started_at: None,
            finished_at: None,
            prompt_tokens: None,
            trimmed: false,
            usage: None,
            selected: None,
            exit_code: None,
            verify_outcome: None,
            promoted_at: None,
            agent_name: None,
            gate_note: None,
        }
    }

    /// `step` with `selected: won`, `Superseded` when it lost and `Done` otherwise.
    fn candidate(
        n: u128,
        position: i32,
        attempt: i32,
        index: i32,
        won: Option<bool>,
    ) -> RunStepSummary {
        RunStepSummary {
            selected: won,
            status: if won == Some(false) {
                StepStatus::Superseded
            } else {
                StepStatus::Done
            },
            ..step(n, position, attempt, index)
        }
    }

    /// `step` with `status`.
    fn with_status(step: RunStepSummary, status: StepStatus) -> RunStepSummary {
        RunStepSummary { status, ..step }
    }

    /// A running graph run `0x900 + n` holding `steps` as given (order not normalised).
    fn run(n: u128, steps: Vec<RunStepSummary>) -> RunSummary {
        RunSummary {
            id: RunId::from_uuid(Uuid::from_u128(0x900 + n)),
            item_id: None,
            project_id: ProjectId::default(),
            kind: RunKind::Graph,
            mode: RunMode::Manual,
            status: RunStatus::Running,
            target_box_id: BoxId::default(),
            executing_box_id: None,
            box_hostname: "box".to_owned(),
            queued_at: demo_at(0, 0),
            started_at: None,
            finished_at: None,
            failure: None,
            steps,
        }
    }

    /// The id `step(n, ..)` carries.
    fn id(n: u128) -> StepId {
        StepId::from_uuid(Uuid::from_u128(0x100 + n))
    }

    /// The layers' step ids, top to bottom.
    fn ids_of(projection: &Projection) -> Vec<Vec<StepId>> {
        projection
            .layers
            .iter()
            .map(|layer| layer.steps.clone())
            .collect()
    }

    /// The top-left corner `project` gave `step`.
    fn at(projection: &Projection, step: StepId) -> (f64, f64) {
        projection
            .positions
            .iter()
            .find(|(id, _)| *id == step)
            .map(|(_, at)| *at)
            .expect("the step has a position")
    }

    #[test]
    fn a_linear_run_is_a_chain() {
        let projection = project(&run(
            1,
            vec![step(1, 0, 1, 0), step(2, 1, 1, 0), step(3, 2, 1, 0)],
        ));
        assert_eq!(ids_of(&projection), [vec![id(1)], vec![id(2)], vec![id(3)]]);
        assert_eq!(
            projection.edges,
            [(id(1), id(2), None), (id(2), id(3), None)]
        );
        assert_eq!(
            projection.positions,
            [
                (id(1), (0.0, 0.0)),
                (id(2), (0.0, 7.0)),
                (id(3), (0.0, 14.0))
            ]
        );
        assert!((projection.width - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_fanout_puts_its_candidates_in_one_layer_and_only_the_winner_feeds_the_next() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, Some(false)),
                candidate(2, 0, 1, 1, Some(true)),
                step(3, 1, 1, 0),
            ],
        ));
        assert_eq!(ids_of(&projection), [vec![id(1), id(2)], vec![id(3)]]);
        assert_eq!(projection.layers[0].winner, Some(id(2)));
        assert_eq!(projection.edges, [(id(2), id(3), None)]);
        assert_eq!(at(&projection, id(3)), (10.0, 7.0));
        assert!((projection.width - 41.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_judge_sorted_before_its_candidates_lands_in_the_layer_after_them() {
        let projection = project(&run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, None),
            ],
        ));
        assert_eq!(
            projection.layers,
            [
                Layer {
                    position: 0,
                    attempt: 1,
                    judge: false,
                    steps: vec![id(1), id(2)],
                    winner: None,
                },
                Layer {
                    position: 0,
                    attempt: 1,
                    judge: true,
                    steps: vec![id(3)],
                    winner: None,
                },
            ]
        );
    }

    #[test]
    fn every_candidate_feeds_the_judge() {
        let projection = project(&run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(4, 1, 1, 0),
            ],
        ));
        assert_eq!(
            projection.edges,
            [
                (id(1), id(3), None),
                (id(2), id(3), None),
                (id(3), id(4), None)
            ]
        );
    }

    #[test]
    fn a_retry_layer_is_fed_by_an_edge_labelled_retry() {
        let single = project(&run(
            1,
            vec![
                with_status(step(1, 1, 1, 0), StepStatus::Failed),
                with_status(step(2, 1, 2, 0), StepStatus::Running),
            ],
        ));
        assert_eq!(single.edges, [(id(1), id(2), Some(RETRY))]);

        let judged = project(&run(
            2,
            vec![
                step(3, 1, 1, -1),
                candidate(1, 1, 1, 0, None),
                candidate(2, 1, 1, 1, None),
                step(4, 1, 2, 0),
            ],
        ));
        assert_eq!(
            judged.edges,
            [
                (id(1), id(3), None),
                (id(2), id(3), None),
                (id(3), id(4), Some(RETRY))
            ]
        );
    }

    #[test]
    fn with_no_winner_yet_every_candidate_feeds_the_next_layer() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, Some(false)),
                step(3, 1, 1, 0),
            ],
        ));
        assert_eq!(
            projection.edges,
            [(id(1), id(3), None), (id(2), id(3), None)]
        );
    }

    #[test]
    fn an_empty_run_has_no_layers() {
        let projection = project(&run(1, Vec::new()));
        assert!(projection.layers.is_empty());
        assert!(projection.edges.is_empty());
        assert!(projection.positions.is_empty());
        assert!(projection.width.abs() < f64::EPSILON);
    }

    #[test]
    fn node_ids_are_step_ids_and_no_two_nodes_overlap() {
        let steps = vec![
            step(4, 0, 1, -1),
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, Some(true)),
            candidate(3, 0, 1, 2, Some(false)),
            step(7, 0, 2, -1),
            candidate(5, 0, 2, 0, None),
            candidate(6, 0, 2, 1, None),
            step(8, 1, 1, 0),
        ];
        let wanted: BTreeSet<StepId> = steps.iter().map(|step| step.id).collect();
        let projection = project(&run(1, steps));

        let placed: BTreeSet<StepId> = projection.positions.iter().map(|(id, _)| *id).collect();
        assert_eq!(placed, wanted);
        assert_eq!(projection.positions.len(), wanted.len());
        for (i, (_, (ax, ay))) in projection.positions.iter().enumerate() {
            for (_, (bx, by)) in &projection.positions[i + 1..] {
                let apart = ax + NODE_W <= *bx
                    || bx + NODE_W <= *ax
                    || ay + NODE_H <= *by
                    || by + NODE_H <= *ay;
                assert!(apart, "({ax}, {ay}) overlaps ({bx}, {by})");
            }
        }
        for (source, target, _) in &projection.edges {
            assert_ne!(source, target);
            assert!(placed.contains(source) && placed.contains(target));
        }
    }

    #[test]
    fn the_projection_does_not_depend_on_the_input_order() {
        let store = vec![
            step(3, 0, 1, -1),
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, None),
            step(4, 1, 1, 0),
        ];
        let judge_last = vec![
            candidate(1, 0, 1, 0, None),
            candidate(2, 0, 1, 1, None),
            step(3, 0, 1, -1),
            step(4, 1, 1, 0),
        ];
        let reversed: Vec<_> = store.iter().rev().cloned().collect();
        let expected = project(&run(1, store));
        assert_eq!(project(&run(1, judge_last)), expected);
        assert_eq!(project(&run(1, reversed)), expected);
    }

    #[test]
    fn a_repeated_step_id_is_kept_once() {
        let projection = project(&run(1, vec![step(1, 0, 1, 0), step(1, 1, 1, 0)]));
        assert_eq!(projection.positions, [(id(1), (0.0, 0.0))]);
        assert_eq!(projection.layers.len(), 1);
        assert_eq!(projection.layers[0].position, 0);
        assert!(projection.edges.is_empty());
    }

    #[test]
    fn layers_are_centred_on_the_widest() {
        let projection = project(&run(
            1,
            vec![
                candidate(1, 0, 1, 0, None),
                candidate(2, 0, 1, 1, None),
                candidate(3, 0, 1, 2, None),
                step(4, 1, 1, 0),
                candidate(5, 2, 1, 0, None),
                candidate(6, 2, 1, 1, None),
            ],
        ));
        assert!((projection.width - 62.0).abs() < f64::EPSILON);
        assert_eq!(at(&projection, id(1)), (0.0, 0.0));
        assert_eq!(at(&projection, id(3)), (42.0, 0.0));
        assert_eq!(at(&projection, id(4)), (21.0, 7.0));
        assert_eq!(at(&projection, id(5)), (10.0, 14.0));
        assert_eq!(at(&projection, id(6)), (31.0, 14.0));
    }
}
