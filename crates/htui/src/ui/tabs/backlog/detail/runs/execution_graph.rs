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

use std::collections::{BTreeMap, BTreeSet};

use htui_core::model::{RunId, RunStepSummary, RunSummary, StepId, StepStatus};
use rataflow::{
    ControlsAction, Edge, EdgeStyle, Flow, Handle, HandlePosition, Node, NodeContent,
    NodeRenderContext, StepEdge, Viewport,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget as _};

use crate::ui::Theme;
use crate::ui::cells::{cell_width, graphemes};

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
/// The margin `ensure_node_visible` keeps (`rataflow` `state/viewport.rs:299`), reused as the
/// reset viewport's left floor and top offset (blueprint E5).
const MARGIN: f64 = 1.0;

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
    /// The candidates and the judges of one `(position, attempt)`.
    type Group<'a> = (Vec<&'a RunStepSummary>, Vec<&'a RunStepSummary>);
    let mut seen = BTreeSet::new();
    let mut groups: BTreeMap<(i32, i32), Group<'_>> = BTreeMap::new();
    for step in steps {
        // Blueprint B-5: a reply is data, so a repeated id keeps its first step only.
        if !seen.insert(step.id) {
            continue;
        }
        let (candidates, judges) = groups.entry((step.position, step.attempt)).or_default();
        // `-1` is the judge (`engine.rs`); `< 0` keeps the split total over `i32` (E2).
        if step.fanout_index < 0 {
            judges.push(step);
        } else {
            candidates.push(step);
        }
    }
    let mut out = Vec::new();
    for ((position, attempt), (candidates, judges)) in groups {
        // The judge layer goes after its candidates, although `-1` sorts first (plan D6).
        for (judge, mut members) in [(false, candidates), (true, judges)] {
            if members.is_empty() {
                continue;
            }
            members.sort_by_key(|step| (step.fanout_index, step.id));
            out.push(Layer {
                position,
                attempt,
                judge,
                steps: members.iter().map(|step| step.id).collect(),
                winner: members
                    .iter()
                    .find(|step| step.selected == Some(true))
                    .map(|step| step.id),
            });
        }
    }
    out
}

/// The width in world units of a layer of `n` nodes (`n >= 1`).
fn span(n: usize) -> f64 {
    let n = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    n * NODE_W + (n - 1.0) * H_GAP
}

/// Plan D6: a run as layers, edges and positions. Rataflow-free, so T1 tests it alone.
pub(super) fn project(run: &RunSummary) -> Projection {
    let layers = layers(&run.steps);
    let width = layers
        .iter()
        .map(|layer| span(layer.steps.len()))
        .fold(0.0, f64::max);
    let mut positions = Vec::new();
    let mut y = 0.0;
    for layer in &layers {
        // Centred on the widest layer, in whole cells (blueprint E7).
        let mut x = ((width - span(layer.steps.len())) / 2.0).floor();
        for step in &layer.steps {
            positions.push((*step, (x, y)));
            x += X_STEP;
        }
        y += Y_STEP;
    }
    let mut edges = Vec::new();
    for pair in layers.windows(2) {
        let [from, to] = pair else { continue };
        let same_slot = (to.position, to.attempt) == (from.position, from.attempt);
        // The judge reads every candidate; past a judge or an `s`, only the winner feeds on.
        let sources = match from.winner {
            Some(winner) if !(to.judge && same_slot) => vec![winner],
            _ => from.steps.clone(),
        };
        let label = (to.position == from.position && to.attempt > from.attempt).then_some(RETRY);
        for source in &sources {
            for target in &to.steps {
                edges.push((*source, *target, label));
            }
        }
    }
    Projection {
        layers,
        edges,
        positions,
        width,
    }
}

/// Plan D10: one step's node. The flow draws no border (`rataflow` `content.rs:79`), so this does.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StepNode {
    /// Line 1: the list's slot, the status, then `*`/`✓` (blueprint B-8).
    head: String,
    /// Line 2: the phase name.
    phase: String,
    /// What colours the border.
    status: StepStatus,
    /// The pane's theme, copied: `NodeContent::render` sees only rataflow's context.
    theme: Theme,
}

impl StepNode {
    /// The node of `step`, one of `siblings` (its run's steps, for the slot's `/i` and `/j`).
    pub(super) fn new(step: &RunStepSummary, siblings: &[RunStepSummary], theme: &Theme) -> Self {
        // Blueprint B-8/E10: the marks in `gate()`'s order, `*` then `✓`.
        let mut marks = String::new();
        if step.promoted_at.is_some() {
            marks.push('*');
        }
        if step.selected == Some(true) {
            marks.push('\u{2713}');
        }
        let status = super::step_status(step.status);
        let slot = super::slot(step, siblings);
        let head = if marks.is_empty() {
            format!("{slot} {status}")
        } else {
            format!("{slot} {status} {marks}")
        };
        Self {
            head,
            phase: step.phase_name.clone(),
            status: step.status,
            theme: *theme,
        }
    }

    /// The border: `selected` on the cursor, else by status (plan D10).
    fn border(&self, selected: bool) -> Style {
        if selected {
            return self.theme.selected;
        }
        match self.status {
            StepStatus::Failed => self.theme.error,
            StepStatus::Running | StepStatus::AwaitingApproval => self.theme.accent,
            StepStatus::Superseded | StepStatus::Cancelled => self.theme.dim,
            StepStatus::Pending | StepStatus::Done => self.theme.base,
        }
    }

    /// The text: `dim` for a node whose border is `dim` (a loser, a retried attempt), `base`
    /// otherwise.
    fn text(&self) -> Style {
        match self.status {
            StepStatus::Superseded | StepStatus::Cancelled => self.theme.dim,
            StepStatus::Pending
            | StepStatus::Running
            | StepStatus::AwaitingApproval
            | StepStatus::Done
            | StepStatus::Failed => self.theme.base,
        }
    }
}

impl NodeContent for StepNode {
    fn render(&self, ctx: &NodeRenderContext, buf: &mut Buffer) {
        let _ = (ctx, buf, Block::bordered());
        todo!("MOD-28 T2")
    }
}

/// `text` in at most `width` terminal cells, by grapheme, cut with `…` (blueprint E11).
fn clip(text: &str, width: usize) -> String {
    let _ = (text, width, cell_width(""), graphemes(""));
    todo!("MOD-28 T2")
}

/// Plan D9: one source on the bottom and one target on the top, both hidden. Baked into every
/// builder, because `set_nodes` resets anything set after it (fact-check R2).
fn handles() -> Vec<Handle> {
    vec![
        Handle::source(HandlePosition::Bottom).with_hidden(true),
        Handle::target(HandlePosition::Top).with_hidden(true),
    ]
}

/// One projected edge, drawn in the pane's own styles (blueprint B-4).
fn edge(
    source: StepId,
    target: StepId,
    label: Option<&'static str>,
    theme: &Theme,
) -> Edge<StepEdge> {
    let content = StepEdge::default().with_style(
        EdgeStyle::default()
            .with_stroke_style(theme.dim)
            .with_label_style(theme.accent),
    );
    let edge = Edge::new(
        format!("{source}>{target}"),
        source.to_string(),
        target.to_string(),
    )
    .with_content(content)
    .with_deletable(false)
    .with_selectable(false);
    match label {
        Some(label) => edge.with_label(label),
        None => edge,
    }
}

/// Blueprint E4: what the next `render` does to the viewport before it draws.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Reveal {
    /// Nothing.
    #[default]
    None,
    /// Keep the cursor node on screen (`ensure_node_visible`, plan D7).
    Cursor,
    /// A new run: zoom 1, the run centred, then the cursor kept on screen (plan D12, B-1).
    Reset,
}

/// Plan D7/D12/D13: the flow view of one run, rebuilt from every sync.
#[derive(Debug)]
pub(super) struct ExecutionGraph {
    /// The canvas. Never fed a key or a mouse event (blueprint H-1).
    flow: Flow<StepNode, StepEdge>,
    /// The run last synced, which tells a new run apart (plan D12).
    run: Option<RunId>,
    /// The cursor step last synced, the node a reveal keeps on screen.
    cursor: Option<StepId>,
    /// The run's widest layer in world units (B-7), for the reset's centring.
    width: f64,
    /// The pending viewport move (B-3).
    reveal: Reveal,
}

impl Default for ExecutionGraph {
    fn default() -> Self {
        Self {
            // Plan D9: an edge is never reconnected.
            flow: Flow::new().with_edges_reconnectable(false),
            run: None,
            cursor: None,
            width: 0.0,
            reveal: Reveal::None,
        }
    }
}

impl ExecutionGraph {
    /// Plan D7/D12: rebuilds the flow from `run`, `cursor` selected. `None` clears it.
    pub(super) fn sync(&mut self, run: Option<&RunSummary>, cursor: Option<StepId>, theme: &Theme) {
        let _ = (run, cursor, theme, handles(), MARGIN);
        todo!("MOD-28 T2")
    }

    /// Plan D8: `+`, one step (1.2) in, clamped to 2.0. The cursor stays on screen (E9).
    pub(super) fn zoom_in(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomIn);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// Plan D8: `-`, one step out, clamped to 0.5 (E9).
    pub(super) fn zoom_out(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::ZoomOut);
        self.reveal = self.reveal.max(Reveal::Cursor);
    }

    /// Plan D8: `=`, the whole run fitted on the next render. A fit shows every node, so it raises
    /// no reveal (E9).
    pub(super) fn fit(&mut self) {
        let _ = self.flow.apply_controls_action(ControlsAction::FitView);
    }

    /// Plan D7, blueprint B-2/E6: draws `area`, revealing first when a reveal is pending.
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let _ = (frame, area, Viewport::default());
        todo!("MOD-28 T2")
    }

    /// The run on the canvas, for the tests.
    #[cfg(test)]
    pub(super) fn shown_run(&self) -> Option<RunId> {
        self.run
    }

    /// The id of the node the flow has selected, for the tests.
    #[cfg(test)]
    pub(super) fn selected(&self) -> Option<String> {
        self.flow.first_selected_node_id()
    }

    /// The canvas zoom, for the tests.
    #[cfg(test)]
    pub(super) fn zoom(&self) -> f64 {
        self.flow.viewport.zoom
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use htui_core::fixtures::demo_at;
    use htui_core::model::{BoxId, ProjectId, RunId, RunKind, RunMode, RunStatus, StepStatus};
    use ratatui::style::{Color, Modifier};
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

    // ---------------------------------------------------------------------------------------------
    // T2: the widget (plan D7, D9, D10, D12, D15).
    // ---------------------------------------------------------------------------------------------

    /// `step` with `phase` as its phase name.
    fn phased(step: RunStepSummary, phase: &str) -> RunStepSummary {
        RunStepSummary {
            phase_name: phase.to_owned(),
            ..step
        }
    }

    /// A fresh graph synced to `run`, the cursor on `cursor`.
    fn synced(run: &RunSummary, cursor: Option<StepId>) -> ExecutionGraph {
        let mut graph = ExecutionGraph::default();
        graph.sync(Some(run), cursor, &Theme::default());
        graph
    }

    /// One frame of `graph` at 43x23 (the 100x30 frame's Runs canvas: 24 rows less the header).
    fn draw(graph: &mut ExecutionGraph) -> Buffer {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(43, 23))
            .expect("the test backend is constructible");
        term.draw(|frame| graph.render(frame, frame.area()))
            .expect("the graph draws");
        term.backend().buffer().clone()
    }

    /// Each row as a `String`, trailing blanks trimmed (as `runs.rs`'s `lines`).
    fn rows(buf: &Buffer) -> Vec<String> {
        (buf.area.top()..buf.area.bottom())
            .map(|y| {
                let row: String = (buf.area.left()..buf.area.right())
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// The `(x, y)` of the `┌` of the node whose text holds `needle` (an ASCII-only row).
    fn corner_of(buf: &Buffer, needle: &str) -> (u16, u16) {
        let rows = rows(buf);
        let (y, x) = rows
            .iter()
            .enumerate()
            .find_map(|(y, row)| {
                row.find(needle)
                    .map(|byte| (y, row[..byte].chars().count()))
            })
            .expect("the needle is drawn");
        let x = u16::try_from(x - 1).expect("the column fits");
        let y = u16::try_from(y).expect("the row fits");
        (0..y)
            .rev()
            .find(|row| buf[(x, *row)].symbol() == "\u{250c}")
            .map(|row| (x, row))
            .expect("the node has a top-left corner above its text")
    }

    #[test]
    fn the_cursor_node_is_drawn_selected() {
        let run = run(1, vec![step(1, 0, 1, 0), step(2, 1, 1, 0)]);
        let mut graph = synced(&run, Some(id(2)));
        let buf = draw(&mut graph);
        let cursor = corner_of(&buf, "1.1 done");
        let other = corner_of(&buf, "0.1 done");
        assert!(buf[cursor].modifier.contains(Modifier::REVERSED));
        assert!(!buf[other].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn a_failed_step_is_drawn_with_the_error_style() {
        let run = run(
            1,
            vec![
                step(1, 0, 1, 0),
                with_status(step(2, 1, 1, 0), StepStatus::Failed),
            ],
        );
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        assert_eq!(buf[corner_of(&buf, "1.1 failed")].fg, Color::Red);
    }

    #[test]
    fn the_winner_carries_a_check_and_the_loser_is_dim() {
        let run = run(
            1,
            vec![
                phased(candidate(1, 0, 1, 0, Some(true)), "research"),
                phased(candidate(2, 0, 1, 1, Some(false)), "research"),
            ],
        );
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        let rows = rows(&buf);
        assert!(
            rows.iter().any(|row| row.contains("0.1/0 done \u{2713}")),
            "{rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("0.1/1 superseded")),
            "{rows:#?}"
        );
        let (x, y) = corner_of(&buf, "0.1/1 superseded");
        assert_eq!(buf[(x, y)].fg, Color::DarkGray);
        assert_eq!(buf[(x + 1, y + 1)].fg, Color::DarkGray);
    }

    #[test]
    fn a_promoted_winner_reads_star_then_check() {
        let promoted = RunStepSummary {
            selected: Some(true),
            promoted_at: Some(demo_at(0, 1)),
            ..step(1, 0, 1, 0)
        };
        let mut graph = synced(&run(1, vec![promoted]), Some(id(1)));
        let rows = rows(&draw(&mut graph));
        assert!(
            rows.iter().any(|row| row.contains("0.1 done *\u{2713}")),
            "{rows:#?}"
        );
    }

    #[test]
    fn text_is_clipped_by_display_width() {
        let wide = "\u{8abf}\u{67fb}\u{30d5}\u{30a7}\u{30fc}\u{30ba}\u{306e}\u{9577}\u{3044}\u{540d}\u{524d}\u{3067}\u{3059}";
        let cut = clip(wide, 18);
        assert!(cell_width(&cut) <= 18, "{cut}");
        assert!(cut.ends_with(super::super::CUT), "{cut}");

        let run = run(1, vec![phased(step(1, 0, 1, 0), wide)]);
        let mut graph = synced(&run, Some(id(1)));
        let buf = draw(&mut graph);
        let (x, y) = corner_of(&buf, "0.1 done");
        let right = (x..buf.area.right())
            .find(|column| buf[(*column, y)].symbol() == "\u{2510}")
            .expect("the node has a top-right corner");
        for row in y + 1..y + 3 {
            assert_eq!(buf[(right, row)].symbol(), "\u{2502}", "row {row}");
        }

        assert_eq!(clip("ab", 5), "ab");
        assert_eq!(clip("abcdef", 4), "abc\u{2026}");
        assert_eq!(clip("\u{65e5}\u{672c}\u{8a9e}", 4), "\u{65e5}\u{2026}");
        assert_eq!(clip("a\nb", 3), "a b");
        assert_eq!(clip("abc", 0), "");
    }

    #[test]
    fn a_re_sync_of_the_same_run_keeps_zoom_selection_and_hidden_handles() {
        let theme = Theme::default();
        let first = run(1, vec![step(1, 0, 1, 0), step(2, 1, 1, 0)]);
        let mut graph = synced(&first, Some(id(2)));
        draw(&mut graph);
        graph.zoom_in();
        draw(&mut graph);
        let viewport = graph.flow.viewport;
        assert!(graph.zoom() > 1.0);

        let mut changed = first.clone();
        changed.steps[1].status = StepStatus::Running;
        graph.sync(Some(&changed), Some(id(2)), &theme);
        assert_eq!(graph.flow.viewport, viewport);
        assert_eq!(graph.selected(), Some(id(2).to_string()));
        assert!(
            graph
                .flow
                .nodes()
                .all(|node| node.handles.iter().all(|handle| handle.hidden))
        );
        draw(&mut graph);
        assert_eq!(graph.flow.viewport, viewport);

        let other = run(2, vec![step(3, 0, 1, 0)]);
        graph.sync(Some(&other), Some(id(3)), &theme);
        draw(&mut graph);
        assert_eq!(graph.shown_run(), Some(other.id));
        assert!((graph.zoom() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_new_run_is_centred_at_zoom_one() {
        let mut one = synced(&run(1, vec![step(1, 0, 1, 0)]), Some(id(1)));
        let buf = draw(&mut one);
        assert_eq!(corner_of(&buf, "0.1 done"), (11, 1));

        let fanout = run(
            2,
            vec![
                phased(candidate(1, 0, 1, 0, Some(true)), "research"),
                phased(candidate(2, 0, 1, 1, Some(false)), "research"),
            ],
        );
        let mut graph = synced(&fanout, Some(id(1)));
        let buf = draw(&mut graph);
        assert_eq!(corner_of(&buf, "0.1/0 done"), (1, 1));
        assert_eq!(corner_of(&buf, "0.1/1 superseded"), (22, 1));
        assert_eq!(buf[(41, 1)].symbol(), "\u{2510}");
    }

    #[test]
    fn a_cursor_below_the_fold_is_visible_on_the_first_render() {
        let steps = (0..8)
            .map(|n| step(n, i32::try_from(n).expect("small"), 1, 0))
            .collect();
        let mut graph = synced(&run(1, steps), Some(id(7)));
        let rows = rows(&draw(&mut graph));
        assert!(rows.iter().any(|row| row.contains("7.1 done")), "{rows:#?}");
    }

    #[test]
    fn a_retry_edge_is_drawn_with_its_label() {
        let run = run(
            1,
            vec![
                with_status(step(1, 1, 1, 0), StepStatus::Failed),
                with_status(step(2, 1, 2, 0), StepStatus::Running),
            ],
        );
        let mut graph = synced(&run, Some(id(2)));
        let rows = rows(&draw(&mut graph));
        let label = rows
            .iter()
            .position(|row| row.contains(RETRY))
            .unwrap_or_else(|| panic!("no retry label: {rows:#?}"));
        assert!(
            rows[label + 1..].iter().any(|row| row.contains('\u{25bc}')),
            "{rows:#?}"
        );
    }

    #[test]
    fn two_renders_of_the_same_state_are_identical() {
        let run = run(
            1,
            vec![
                step(3, 0, 1, -1),
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(4, 1, 1, 0),
            ],
        );
        let mut graph = synced(&run, Some(id(4)));
        assert_eq!(draw(&mut graph), draw(&mut graph));
    }

    #[test]
    fn no_node_is_draggable_connectable_or_deletable() {
        let run = run(
            1,
            vec![
                candidate(1, 0, 1, 0, Some(true)),
                candidate(2, 0, 1, 1, Some(false)),
                step(3, 1, 1, 0),
            ],
        );
        let graph = synced(&run, Some(id(1)));
        assert_eq!(graph.flow.nodes().count(), 3);
        assert!(
            graph
                .flow
                .nodes()
                .all(|node| !node.draggable && !node.connectable && !node.deletable)
        );
        assert!(!graph.flow.edges().is_empty());
        assert!(graph.flow.edges().iter().all(|edge| !edge.deletable));
        assert!(!graph.flow.edges_reconnectable);
    }

    #[test]
    fn a_sync_of_no_run_clears_the_canvas() {
        let mut graph = synced(&run(1, vec![step(1, 0, 1, 0)]), Some(id(1)));
        draw(&mut graph);
        graph.sync(None, None, &Theme::default());
        assert_eq!(graph.flow.nodes().count(), 0);
        assert_eq!(graph.shown_run(), None);
        assert!(rows(&draw(&mut graph)).iter().all(String::is_empty));
    }
}
