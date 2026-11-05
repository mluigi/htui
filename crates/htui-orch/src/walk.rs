//! The readiness walker (MOD-138 M2 D2, ANA-33 §7.3): pure, re-derived from the run's rows on
//! every call (plan D16), over the snapshot's nodes and edges.
//!
//! - **Joins.** A node is reached only when every forward in-edge is resolved and at least one
//!   was taken; a root (no forward in-edge) is reached at once.
//! - **Skips.** A node whose forward in-edges are all resolved and none taken is skipped. The
//!   skip is derived, never stored, and it propagates: a skipped node's out-edges are declined.
//! - **Rows win.** A node whose rows read `done` is done whatever its in-edges say now, since a
//!   loop retires only its own region (ANA-33 §7.3).
//! - **Owed writes first** (blueprint A6): an undecided `if` source ([`Cursor::Decide`]) and an
//!   approved back `asks_you` source with budget ([`Cursor::Loop`]) sort before every node.
//!
//! On a lifted linear graph the first of [`ready`] is exactly the pre-M2 position scan's cursor
//! (blueprint §2.1), which the parity oracle in this module's tests enforces. Nothing here reads
//! a phase *name* for meaning: a node reads its verdict because it has a `changes_requested`
//! edge ([`GraphSnapshot::reads_verdict`]), never because of what it is called.

use std::borrow::Cow;
use std::collections::BTreeSet;

use htui_core::model::{
    EdgeKind, GateOutcome, GraphSnapshot, RunStep, SnapshotEdge, SnapshotPhase, StepStatus,
};
use htui_core::store::{WalkEdge, WalkNode, forward_mask};

use crate::status::{Cursor, group_at, group_cursor, latest_at, may_attempt, next_attempt};

/// A node's latest rows, by `status.rs`'s per-position rules, moved here and not rewritten.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeState<'s> {
    /// `fan_out = 1`: the latest attempt at `fanout_index 0` is `done`. `fan_out > 1`: the slot
    /// holds a `selected` `done` candidate, which is the winner carried here.
    Done(&'s RunStep),
    /// [`Cursor::Run`], [`Cursor::Rest`], [`Cursor::Fan`] or [`Cursor::Select`]: a live row or
    /// slot the engine drives or waits on.
    Live(Cursor),
    /// The latest row or slot is superseded or cancelled: create it again at `attempt`.
    Retired {
        /// The attempt to create: [`next_attempt`], or the group's highest plus one.
        attempt: i32,
    },
    /// No row: create attempt 1.
    Absent,
}

impl NodeState<'_> {
    /// The cursor of a reached node at `position`; `None` when it is done.
    fn cursor(&self, position: i32) -> Option<Cursor> {
        match self {
            Self::Done(_) => None,
            Self::Live(cursor) => Some(cursor.clone()),
            Self::Retired { attempt } => Some(Cursor::Create {
                position,
                attempt: *attempt,
            }),
            Self::Absent => Some(Cursor::Create {
                position,
                attempt: 1,
            }),
        }
    }
}

/// D2: what `phase`'s rows say, read exactly as the pre-M2 position scan read one position.
#[must_use]
pub fn node_state<'s>(phase: &SnapshotPhase, steps: &'s [RunStep]) -> NodeState<'s> {
    let position = phase.position;
    if phase.fan_out > 1 {
        return match group_cursor(steps, position, phase.fan_out) {
            // A `selected` `done` candidate completes the slot (plan D59): it is the winner.
            None => steps
                .iter()
                .filter(|step| step.position == position && step.fanout_index >= 0)
                .map(|step| step.attempt)
                .max()
                .and_then(|attempt| {
                    group_at(steps, position, attempt)
                        .into_iter()
                        .find(|step| step.selected == Some(true) && step.status == StepStatus::Done)
                })
                .map_or(NodeState::Absent, NodeState::Done),
            Some(Cursor::Create { attempt: 1, .. }) => NodeState::Absent,
            Some(Cursor::Create { attempt, .. }) => NodeState::Retired { attempt },
            Some(cursor) => NodeState::Live(cursor),
        };
    }
    let Some(step) = latest_at(steps, position) else {
        return NodeState::Absent;
    };
    match step.status {
        StepStatus::Done => NodeState::Done(step),
        StepStatus::Pending => NodeState::Live(Cursor::Run(step.id)),
        StepStatus::Running
        | StepStatus::AwaitingApproval
        | StepStatus::Stopped
        | StepStatus::Failed => NodeState::Live(Cursor::Rest {
            step: step.id,
            status: step.status,
        }),
        StepStatus::Superseded | StepStatus::Cancelled => NodeState::Retired {
            attempt: next_attempt(steps, position),
        },
    }
}

/// D2's edge table for one forward in-edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeState {
    /// The source has not finished, or its `if` decision is not recorded yet.
    Unresolved,
    /// The edge leads on.
    Taken,
    /// The edge is resolved and does not lead on.
    Declined,
}

/// A node once its forward in-edges are read. `Done` wins over everything ("rows win").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The node's rows read `done`.
    Done,
    /// A root, or every forward in-edge resolved and at least one taken: the node walks.
    Reached,
    /// Every forward in-edge resolved and none taken: the node never walks.
    Skipped,
    /// Some forward in-edge is still unresolved.
    Waiting,
}

/// The snapshot as the walker reads it: [`GraphSnapshot::walk_edges`], the forward mask
/// ([`forward_mask`], so it cannot disagree with the validator), and a forward topological
/// order (Kahn, ties by `position`), `None` on a forward cycle.
///
/// Nodes are indices into `snapshot.phases`; a key names the first phase that holds it, as
/// `graph::snapshot_walk_refusal` reads it. Palette nodes are not walked.
#[derive(Debug)]
pub struct Graph<'a> {
    snapshot: &'a GraphSnapshot,
    edges: Cow<'a, [SnapshotEdge]>,
    /// Per edge: `(from, to)` as phase indices, `None` when either key is unknown.
    ends: Vec<Option<(usize, usize)>>,
    /// Per edge: whether it is forward.
    forward: Vec<bool>,
    /// Per phase: its forward successors.
    successors: Vec<Vec<usize>>,
    /// Per phase: its forward predecessors.
    predecessors: Vec<Vec<usize>>,
    /// The non-palette phases, in `(position, index)` order.
    walked: Vec<usize>,
    /// The walked phases in forward-topological order, ties by `(position, index)`.
    topo: Option<Vec<usize>>,
}

impl<'a> Graph<'a> {
    /// Reads `snapshot` once.
    #[must_use]
    pub fn of(snapshot: &'a GraphSnapshot) -> Self {
        let phases = &snapshot.phases;
        let edges = snapshot.walk_edges();
        let index = |key: &str| phases.iter().position(|phase| phase.name == key);
        let ends: Vec<Option<(usize, usize)>> = edges
            .iter()
            .map(|edge| Some((index(&edge.from)?, index(&edge.to)?)))
            .collect();
        let nodes: Vec<WalkNode<'_>> = phases
            .iter()
            .map(|phase| WalkNode {
                name: &phase.name,
                palette: phase.palette,
            })
            .collect();
        let known: Vec<usize> = (0..edges.len()).filter(|&e| ends[e].is_some()).collect();
        let walk_edges: Vec<WalkEdge> = known
            .iter()
            .filter_map(|&e| {
                let (from, to) = ends[e]?;
                Some(WalkEdge {
                    from,
                    to,
                    kind: edges[e].kind,
                })
            })
            .collect();
        let mut forward = vec![false; edges.len()];
        for (&e, is_forward) in known.iter().zip(forward_mask(&nodes, &walk_edges)) {
            forward[e] = is_forward;
        }
        let mut successors = vec![Vec::new(); phases.len()];
        let mut predecessors = vec![Vec::new(); phases.len()];
        for (e, end) in ends.iter().enumerate() {
            if let (true, Some((from, to))) = (forward[e], *end) {
                successors[from].push(to);
                predecessors[to].push(from);
            }
        }
        let mut walked: Vec<usize> = (0..phases.len()).filter(|&i| !phases[i].palette).collect();
        walked.sort_by_key(|&i| (phases[i].position, i));
        let topo = topological(phases, &walked, &successors, &predecessors);
        Self {
            snapshot,
            edges,
            ends,
            forward,
            successors,
            predecessors,
            walked,
            topo,
        }
    }

    /// The phase index of `key`: the first phase that holds it.
    fn index(&self, key: &str) -> Option<usize> {
        self.snapshot
            .phases
            .iter()
            .position(|phase| phase.name == key)
    }

    /// The edges out of `source` of `kind`, with their index, in edge order.
    fn out_of<'g>(
        &'g self,
        source: &'g str,
        kind: EdgeKind,
    ) -> impl Iterator<Item = (usize, &'g SnapshotEdge)> + 'g {
        self.edges
            .iter()
            .enumerate()
            .filter(move |(_, edge)| edge.from == source && edge.kind == kind)
    }

    /// Every phase reachable from `start` over `next` (successors or predecessors), `start`
    /// included.
    fn closure(start: usize, next: &[Vec<usize>]) -> Vec<bool> {
        let mut seen = vec![false; next.len()];
        let mut queue = vec![start];
        seen[start] = true;
        while let Some(at) = queue.pop() {
            for &successor in &next[at] {
                if !seen[successor] {
                    seen[successor] = true;
                    queue.push(successor);
                }
            }
        }
        seen
    }

    /// The edges the walker reads: [`GraphSnapshot::walk_edges`].
    #[must_use]
    pub fn edges(&self) -> &[SnapshotEdge] {
        &self.edges
    }

    /// The first `kind` edge out of `source` in snapshot edge order (blueprint F21); for
    /// `changes_requested` and `fails`, whose loops D7 drives.
    #[must_use]
    pub fn loop_edge(&self, source: &str, kind: EdgeKind) -> Option<&SnapshotEdge> {
        self.edges
            .iter()
            .find(|edge| edge.from == source && edge.kind == kind)
    }

    /// The forward `asks_you` edges out of `source`, in edge order.
    pub fn forward_asks_you<'g>(
        &'g self,
        source: &'g str,
    ) -> impl Iterator<Item = &'g SnapshotEdge> + 'g {
        self.out_of(source, EdgeKind::AsksYou)
            .filter(|(e, _)| self.forward[*e])
            .map(|(_, edge)| edge)
    }

    /// The back `asks_you` edges out of `source` (both ends known, not forward), in edge order.
    pub fn back_asks_you<'g>(
        &'g self,
        source: &'g str,
    ) -> impl Iterator<Item = &'g SnapshotEdge> + 'g {
        self.out_of(source, EdgeKind::AsksYou)
            .filter(|(e, _)| self.ends[*e].is_some() && !self.forward[*e])
            .map(|(_, edge)| edge)
    }

    /// The `if` edges out of `source`, in edge order.
    pub fn if_edges<'g>(&'g self, source: &'g str) -> impl Iterator<Item = &'g SnapshotEdge> + 'g {
        self.out_of(source, EdgeKind::If).map(|(_, edge)| edge)
    }

    /// D7's region: every node on a forward path from `target` to `source`, both included, as
    /// positions in forward-topological order: target first, source last (A6). Empty when
    /// either key is unknown or `target` does not reach `source`.
    #[must_use]
    pub fn region(&self, target: &str, source: &str) -> Vec<i32> {
        let (Some(target), Some(source)) = (self.index(target), self.index(source)) else {
            return Vec::new();
        };
        let from_target = Self::closure(target, &self.successors);
        let to_source = Self::closure(source, &self.predecessors);
        self.topo
            .as_ref()
            .unwrap_or(&self.walked)
            .iter()
            .filter(|&&i| from_target[i] && to_source[i])
            .map(|&i| self.snapshot.phases[i].position)
            .collect()
    }

    /// D9: the nodes reachable from `node` over forward edges, `node` excluded, in position
    /// order. Empty for a key the snapshot does not name.
    #[must_use]
    pub fn reachable_from(&self, node: &str) -> Vec<&'a SnapshotPhase> {
        let Some(start) = self.index(node) else {
            return Vec::new();
        };
        let reached = Self::closure(start, &self.successors);
        let phases = &self.snapshot.phases;
        self.walked
            .iter()
            .filter(|&&i| i != start && reached[i])
            .map(|&i| &phases[i])
            .collect()
    }

    /// Each walked node's [`Reach`], by phase index, in topological order; `None` on a forward
    /// cycle. A palette node stays `Waiting` and is never read.
    fn reach(&self, states: &[Option<NodeState<'_>>]) -> Option<Vec<Reach>> {
        let order = self.topo.as_ref()?;
        let mut reach = vec![Reach::Waiting; states.len()];
        for &node in order {
            reach[node] = if let Some(NodeState::Done(_)) = states[node] {
                Reach::Done
            } else {
                let mut any = false;
                let mut taken = false;
                let mut unresolved = false;
                for (e, end) in self.ends.iter().enumerate() {
                    let Some((from, to)) = *end else { continue };
                    if to != node || !self.forward[e] {
                        continue;
                    }
                    any = true;
                    match edge_state(&self.edges[e], reach[from], states[from].as_ref()) {
                        EdgeState::Unresolved => unresolved = true,
                        EdgeState::Taken => taken = true,
                        EdgeState::Declined => {}
                    }
                }
                if !any || (taken && !unresolved) {
                    Reach::Reached
                } else if unresolved {
                    Reach::Waiting
                } else {
                    Reach::Skipped
                }
            };
        }
        Some(reach)
    }
}

/// Kahn over the forward edges, ties by `(position, index)`; `None` when a forward cycle leaves
/// a walked node unordered.
fn topological(
    phases: &[SnapshotPhase],
    walked: &[usize],
    successors: &[Vec<usize>],
    predecessors: &[Vec<usize>],
) -> Option<Vec<usize>> {
    let mut indegree: Vec<usize> = predecessors.iter().map(Vec::len).collect();
    let mut ready: BTreeSet<(i32, usize)> = walked
        .iter()
        .filter(|&&i| indegree[i] == 0)
        .map(|&i| (phases[i].position, i))
        .collect();
    let mut order = Vec::with_capacity(walked.len());
    while let Some((_, node)) = ready.pop_first() {
        order.push(node);
        for &successor in &successors[node] {
            indegree[successor] -= 1;
            if indegree[successor] == 0 {
                ready.insert((phases[successor].position, successor));
            }
        }
    }
    (order.len() == walked.len()).then_some(order)
}

/// D2's edge table: one forward edge, by its source's [`Reach`] and rows.
fn edge_state(edge: &SnapshotEdge, source: Reach, state: Option<&NodeState<'_>>) -> EdgeState {
    match (source, state) {
        (Reach::Done, Some(NodeState::Done(winner))) => match edge.kind {
            EdgeKind::Next => EdgeState::Taken,
            EdgeKind::If => match &winner.branch {
                None => EdgeState::Unresolved,
                Some(branch) => {
                    if branch.get(&edge.to).and_then(serde_json::Value::as_bool) == Some(true) {
                        EdgeState::Taken
                    } else {
                        EdgeState::Declined
                    }
                }
            },
            EdgeKind::AsksYou => {
                if winner.gate_outcome == Some(GateOutcome::Approved) {
                    EdgeState::Taken
                } else {
                    EdgeState::Declined
                }
            }
            // Never forward (W3), so never read here; a loop edge leads nowhere forward.
            EdgeKind::ChangesRequested | EdgeKind::Fails => EdgeState::Declined,
        },
        (Reach::Skipped, _) => EdgeState::Declined,
        _ => EdgeState::Unresolved,
    }
}

/// D2: every cursor the walk may take now, never empty.
///
/// Order (A6): every [`Cursor::Decide`], then every [`Cursor::Loop`], then each reached node's
/// cursor, each group in position order; `[Finished]` when every walked node is done or skipped
/// and nothing is owed.
/// - `Decide { step: winner }`: a done node with an `if` edge whose winner has no `branch`.
/// - `Loop { step: winner, to }`: a done node whose winner is `approved`, for its first back
///   `asks_you` edge with [`loop_budget_left`].
/// - Edge resolution, by the source's [`Reach`]: `Done` → `next` taken; `if` taken iff
///   `branch[to]` is `true`, declined for `false` or a missing key, unresolved while `branch` is
///   `None`; a forward `asks_you` taken iff `gate_outcome` is `approved`, else declined.
///   `Skipped` → declined. Otherwise unresolved.
/// - Reach, in topological order: `Done` from rows; else a root is reached; every in-edge
///   resolved → reached if any is taken, else skipped; else waiting. A skipped node's non-done
///   rows are ignored.
/// - Fallback, reachable only on a forward cycle (the validator refuses one, so only on a
///   hand-built snapshot): the first walked node in position order that is not done, read as
///   reached. That is the pre-M2 position rule, and it keeps `ready` total.
#[must_use]
pub fn ready(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Vec<Cursor> {
    let graph = Graph::of(snapshot);
    let phases = &snapshot.phases;
    let states: Vec<Option<NodeState<'_>>> = phases
        .iter()
        .map(|phase| (!phase.palette).then(|| node_state(phase, steps)))
        .collect();

    let mut decide = Vec::new();
    let mut loops = Vec::new();
    for &node in &graph.walked {
        let Some(NodeState::Done(winner)) = states[node] else {
            continue;
        };
        let name = phases[node].name.as_str();
        if winner.branch.is_none() && graph.if_edges(name).next().is_some() {
            decide.push(Cursor::Decide { step: winner.id });
        }
        if winner.gate_outcome == Some(GateOutcome::Approved)
            && let Some(edge) = graph
                .back_asks_you(name)
                .find(|edge| loop_budget_left(snapshot, steps, edge))
        {
            loops.push(Cursor::Loop {
                step: winner.id,
                to: edge.to.clone(),
            });
        }
    }

    let reach = graph.reach(&states);
    let open = |node: usize| match &reach {
        Some(reach) => matches!(reach[node], Reach::Reached | Reach::Waiting),
        None => !matches!(states[node], Some(NodeState::Done(_))),
    };
    let cursor_of = |node: usize| {
        states[node]
            .as_ref()
            .and_then(|state| state.cursor(phases[node].position))
    };
    let mut ready = decide;
    ready.append(&mut loops);
    match &reach {
        Some(reach) => ready.extend(
            graph
                .walked
                .iter()
                .filter(|&&node| reach[node] == Reach::Reached)
                .filter_map(|&node| cursor_of(node)),
        ),
        // A forward cycle: the position rule.
        None => ready.extend(
            graph
                .walked
                .iter()
                .find(|&&node| open(node))
                .and_then(|&node| cursor_of(node)),
        ),
    }
    if ready.is_empty() {
        // Unreachable on an acyclic graph (an open node's earliest open ancestor is reached or
        // owes a decision); kept so `ready` is total.
        ready.push(
            graph
                .walked
                .iter()
                .find(|&&node| open(node))
                .and_then(|&node| cursor_of(node))
                .unwrap_or(Cursor::Finished),
        );
    }
    ready
}

/// D7's budget, pure, shared by [`ready`]'s [`Cursor::Loop`], the `asks_you` stop and
/// `gate::edge_loop`: the target has a latest row and [`may_attempt`]`(attempt + 1,
/// edge.loop_limit or the target's retry_limit)`. An unknown target, or one with no row: `false`.
#[must_use]
pub fn loop_budget_left(snapshot: &GraphSnapshot, steps: &[RunStep], edge: &SnapshotEdge) -> bool {
    let Some(target) = snapshot.phases.iter().find(|phase| phase.name == edge.to) else {
        return false;
    };
    latest_at(steps, target.position).is_some_and(|row| {
        may_attempt(
            row.attempt + 1,
            edge.loop_limit.unwrap_or(target.retry_limit),
        )
    })
}

/// Which of a phase's `input_kinds` are **required** (blueprint H-8, plan D21; MOD-138 M2 D9).
///
/// A kind is optional exactly when a node reachable from `phase` over forward edges (itself
/// excluded) produces it. The seeded `implement` phase lists `review` in `input_kinds`
/// (`crates/htui-core/src/seed.rs`), ANA-2 §4.1's own seed amendment, so the review loop can
/// feed a review back; §4.2 makes a missing input a hard failure before a token is spent. On
/// attempt 1 no review exists, and on a lifted linear graph "reachable" is "a later position",
/// the pre-M2 rule. A forward input is still mandatory, and a loop's back-edge input is absent
/// exactly when it has not run yet; its absence is recorded in the trim record's notes.
///
/// A `phase` the snapshot does not name (a fan-out judge, whose `input_kinds` are empty anyway)
/// reaches nothing, so all its kinds are required.
#[must_use]
pub fn required_inputs(phase: &SnapshotPhase, snapshot: &GraphSnapshot) -> Vec<String> {
    let reachable = Graph::of(snapshot).reachable_from(&phase.name);
    phase
        .input_kinds
        .iter()
        .filter(|kind| !reachable.iter().any(|later| later.output_kind == **kind))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        EdgeKind, EdgePredicate, GateOutcome, GraphSnapshot, RunStep, SnapshotEdge, SnapshotPhase,
        StepId, StepStatus,
    };
    use serde_json::json;

    use super::{Graph, ready, required_inputs};
    use crate::status::Cursor;

    /// `RUN_1`'s snapshot and its first step, the shapes every node and row here is cloned from.
    pub(super) fn fixture() -> (GraphSnapshot, RunStep) {
        let data = demo_data();
        let run = data
            .runs
            .iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        let snapshot: GraphSnapshot = serde_json::from_value(
            run.graph_snapshot
                .clone()
                .expect("RUN_1 carries a snapshot"),
        )
        .expect("the fixture snapshot is a `GraphSnapshot`");
        let row = data
            .steps
            .into_iter()
            .find(|step| step.run_id == ids::RUN_1)
            .expect("RUN_1 has steps");
        (snapshot, row)
    }

    /// A `fan_out = 1` node `name` at `position`, producing `name`, with one retry.
    pub(super) fn node(template: &SnapshotPhase, name: &str, position: i32) -> SnapshotPhase {
        let mut phase = template.clone();
        phase.name = name.to_owned();
        phase.position = position;
        phase.output_kind = name.to_owned();
        phase.input_kinds = Vec::new();
        phase.fan_out = 1;
        phase.retry_limit = 1;
        phase.palette = false;
        phase
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> SnapshotEdge {
        SnapshotEdge {
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            predicate: (kind == EdgeKind::If).then(|| EdgePredicate(json!({ "verify": "passed" }))),
            loop_limit: None,
        }
    }

    /// A v2 snapshot of `nodes` (`(name, position)`) and `edges`.
    fn graph(nodes: &[(&str, i32)], edges: &[(&str, &str, EdgeKind)]) -> GraphSnapshot {
        let (mut snapshot, _) = fixture();
        let template = snapshot.phases[0].clone();
        snapshot.v = GraphSnapshot::V;
        snapshot.phases = nodes
            .iter()
            .map(|(name, position)| node(&template, name, *position))
            .collect();
        snapshot.edges = edges
            .iter()
            .map(|(from, to, kind)| edge(from, to, *kind))
            .collect();
        snapshot
    }

    /// One `fanout_index 0` row of node `name`.
    fn row(snapshot: &GraphSnapshot, name: &str, attempt: i32, status: StepStatus) -> RunStep {
        let (_, mut row) = fixture();
        let phase = snapshot
            .phases
            .iter()
            .find(|phase| phase.name == name)
            .expect("the node exists");
        row.id = StepId::new();
        row.position = phase.position;
        row.phase_name = name.to_owned();
        row.attempt = attempt;
        row.fanout_index = 0;
        row.status = status;
        row.selected = None;
        row.gate_outcome = None;
        row.gate_note = None;
        row.branch = None;
        row
    }

    fn done(snapshot: &GraphSnapshot, name: &str) -> RunStep {
        row(snapshot, name, 1, StepStatus::Done)
    }

    fn create(position: i32) -> Cursor {
        Cursor::Create {
            position,
            attempt: 1,
        }
    }

    const NEXT: EdgeKind = EdgeKind::Next;
    const IF: EdgeKind = EdgeKind::If;
    const ASKS: EdgeKind = EdgeKind::AsksYou;

    /// The diamond `a → {b, c} → d`, every edge `next`.
    fn diamond() -> GraphSnapshot {
        graph(
            &[("a", 0), ("b", 1), ("c", 2), ("d", 3)],
            &[
                ("a", "b", NEXT),
                ("a", "c", NEXT),
                ("b", "d", NEXT),
                ("c", "d", NEXT),
                ("d", "b", EdgeKind::ChangesRequested),
            ],
        )
    }

    /// D2: a node with no forward in-edge is reached; on a v2 snapshot with no edges every node
    /// is a root, and on a chain only the first is.
    #[test]
    fn a_root_is_reached() {
        let roots = graph(&[("a", 0), ("b", 1)], &[]);
        assert_eq!(ready(&roots, &[]), [create(0), create(1)]);
        let chain = graph(&[("a", 0), ("b", 1)], &[("a", "b", NEXT)]);
        assert_eq!(ready(&chain, &[]), [create(0)]);
    }

    /// D2: a join is reached only once every forward in-edge is resolved.
    #[test]
    fn a_join_waits_for_every_forward_in_edge() {
        let snapshot = diamond();
        let a = done(&snapshot, "a");
        let b = done(&snapshot, "b");
        let c = row(&snapshot, "c", 1, StepStatus::Running);
        assert_eq!(
            ready(&snapshot, &[a.clone(), b.clone(), c.clone()]),
            [Cursor::Rest {
                step: c.id,
                status: StepStatus::Running
            }],
            "`d` waits on `c`"
        );
        assert_eq!(
            ready(&snapshot, &[a.clone(), b.clone()]),
            [create(2)],
            "`d` waits on an absent `c`"
        );
        assert_eq!(
            ready(&snapshot, std::slice::from_ref(&a)),
            [create(1), create(2)],
            "both arms are ready together, in position order"
        );
        let c = done(&snapshot, "c");
        assert_eq!(ready(&snapshot, &[a, b, c]), [create(3)]);
    }

    /// D2: a join whose in-edges are all resolved runs when one of them was taken.
    #[test]
    fn a_join_runs_on_one_taken_edge_once_all_resolve() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2), ("d", 3)],
            &[
                ("a", "b", IF),
                ("a", "c", IF),
                ("b", "d", NEXT),
                ("c", "d", NEXT),
            ],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": true, "c": false }));
        assert_eq!(
            ready(&snapshot, &[a.clone()]),
            [create(1)],
            "`d` waits on `b`"
        );
        let b = done(&snapshot, "b");
        assert_eq!(
            ready(&snapshot, &[a, b]),
            [create(3)],
            "`c` is skipped, so its edge is resolved and not taken; `b`'s is taken"
        );
    }

    /// D2: a node whose forward in-edges are all declined is skipped, and never yields a cursor.
    #[test]
    fn all_declined_in_edges_skip_a_node() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", IF), ("a", "c", NEXT)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": false }));
        assert_eq!(ready(&snapshot, &[a.clone()]), [create(2)]);
        let c = done(&snapshot, "c");
        assert_eq!(ready(&snapshot, &[a, c]), [Cursor::Finished]);
    }

    /// D2: a skipped node's out-edges are declined, so the skip propagates down the branch.
    #[test]
    fn a_skip_propagates() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2), ("d", 3)],
            &[("a", "b", IF), ("b", "c", NEXT), ("a", "d", NEXT)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": false }));
        assert_eq!(
            ready(&snapshot, &[a.clone()]),
            [create(3)],
            "`b` and `c` are skipped"
        );
        let d = done(&snapshot, "d");
        assert_eq!(ready(&snapshot, &[a, d]), [Cursor::Finished]);
    }

    /// D2: an `if` edge is taken exactly when the source's recorded branch says `true`.
    #[test]
    fn an_if_edge_follows_the_recorded_branch() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", IF), ("a", "c", IF)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": true, "c": false }));
        assert_eq!(ready(&snapshot, &[a.clone()]), [create(1)]);
        a.branch = Some(json!({ "b": false, "c": true }));
        assert_eq!(ready(&snapshot, &[a.clone()]), [create(2)]);
        a.branch = Some(json!({ "b": true, "c": true }));
        assert_eq!(ready(&snapshot, &[a]), [create(1), create(2)]);
    }

    /// D2, A6: a done source with an `if` edge and no recorded branch owes a decision, which
    /// sorts before every node cursor; its targets wait.
    #[test]
    fn an_undecided_if_source_yields_decide_first() {
        let snapshot = graph(&[("x", 0), ("a", 1), ("b", 2)], &[("a", "b", IF)]);
        let a = done(&snapshot, "a");
        assert_eq!(
            ready(&snapshot, std::slice::from_ref(&a)),
            [Cursor::Decide { step: a.id }, create(0)],
            "`Decide` at position 1 sorts before `x`'s `Create` at position 0"
        );
    }

    /// D2: a recorded branch that does not name an edge's target declines that edge.
    #[test]
    fn a_branch_missing_an_edge_key_reads_as_declined() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", IF), ("a", "c", IF)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": true }));
        assert_eq!(ready(&snapshot, &[a.clone()]), [create(1)]);
        let b = done(&snapshot, "b");
        assert_eq!(ready(&snapshot, &[a, b]), [Cursor::Finished]);
    }

    /// D2 (Q-B): a forward `asks_you` edge is taken only on `approved`; `skipped` or anything else
    /// declines it.
    #[test]
    fn a_forward_asks_you_is_taken_only_when_approved() {
        let snapshot = graph(&[("a", 0), ("b", 1)], &[("a", "b", ASKS)]);
        let mut a = done(&snapshot, "a");
        a.gate_outcome = Some(GateOutcome::Approved);
        assert_eq!(ready(&snapshot, &[a.clone()]), [create(1)]);
        for outcome in [
            None,
            Some(GateOutcome::Skipped),
            Some(GateOutcome::Rejected),
        ] {
            a.gate_outcome = outcome;
            assert_eq!(
                ready(&snapshot, &[a.clone()]),
                [Cursor::Finished],
                "{outcome:?} declines the edge"
            );
        }
    }

    /// `t → s → u` with a back `asks_you` from `s` to `t`.
    fn back_asks() -> GraphSnapshot {
        graph(
            &[("t", 0), ("s", 1), ("u", 2)],
            &[("t", "s", NEXT), ("s", "u", NEXT), ("s", "t", ASKS)],
        )
    }

    /// D7, A6: a done, approved source whose back `asks_you` edge has budget owes the loop, which
    /// sorts before every node cursor.
    #[test]
    fn a_back_asks_you_approved_with_budget_yields_loop_first() {
        let snapshot = back_asks();
        let t = done(&snapshot, "t");
        let mut s = done(&snapshot, "s");
        s.gate_outcome = Some(GateOutcome::Approved);
        assert_eq!(
            ready(&snapshot, &[t.clone(), s.clone()]),
            [
                Cursor::Loop {
                    step: s.id,
                    to: "t".to_owned()
                },
                create(2)
            ]
        );
        s.gate_outcome = Some(GateOutcome::Skipped);
        assert_eq!(
            ready(&snapshot, &[t, s]),
            [create(2)],
            "a declined back edge owes nothing"
        );
    }

    /// A6: the owed writes sort by kind before position. `y`'s `Decide` at position 4 comes
    /// before `s`'s `Loop` at position 2, and both come before root `x`'s `Create` at position 0.
    #[test]
    fn decide_then_loop_then_node_cursors_regardless_of_position() {
        let snapshot = graph(
            &[("x", 0), ("t", 1), ("s", 2), ("u", 3), ("y", 4), ("z", 5)],
            &[
                ("t", "s", NEXT),
                ("s", "u", NEXT),
                ("s", "t", ASKS),
                ("y", "z", IF),
            ],
        );
        let t = done(&snapshot, "t");
        let mut s = done(&snapshot, "s");
        s.gate_outcome = Some(GateOutcome::Approved);
        let y = done(&snapshot, "y");
        assert_eq!(
            ready(&snapshot, &[t, s.clone(), y.clone()]),
            [
                Cursor::Decide { step: y.id },
                Cursor::Loop {
                    step: s.id,
                    to: "t".to_owned()
                },
                create(0),
                create(3)
            ],
            "every `Decide`, then every `Loop`, then node cursors, each group by position"
        );
    }

    /// D7: once the target's attempts reach the edge's budget, the approved source owes no loop.
    #[test]
    fn a_spent_back_asks_you_yields_no_loop() {
        let mut snapshot = back_asks();
        let first = row(&snapshot, "t", 1, StepStatus::Superseded);
        let t = row(&snapshot, "t", 2, StepStatus::Done);
        let mut s = done(&snapshot, "s");
        s.gate_outcome = Some(GateOutcome::Approved);
        let rows = [first, t, s.clone()];
        assert_eq!(
            ready(&snapshot, &rows),
            [create(2)],
            "`retry_limit = 1` admits no third attempt of `t`"
        );
        snapshot.edges[2].loop_limit = Some(5);
        assert_eq!(snapshot.edges[2].kind, ASKS);
        assert_eq!(
            ready(&snapshot, &rows),
            [
                Cursor::Loop {
                    step: s.id,
                    to: "t".to_owned()
                },
                create(2)
            ],
            "the edge's own `loop_limit` wins over the target's `retry_limit`"
        );
    }

    /// D2: `Finished` once every node is done or skipped, a skipped tail included.
    #[test]
    fn finished_counts_skipped_tails() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", IF), ("b", "c", NEXT)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": false }));
        assert_eq!(ready(&snapshot, &[a]), [Cursor::Finished]);
    }

    /// D2: reach is computed over the forward edges, not by position: a chain laid out in
    /// reverse position order starts at its root, the highest position.
    #[test]
    fn topological_order_not_position_decides_reach() {
        let snapshot = graph(
            &[("c", 0), ("b", 1), ("a", 2)],
            &[("a", "b", NEXT), ("b", "c", NEXT)],
        );
        assert_eq!(ready(&snapshot, &[]), [create(2)]);
        let a = done(&snapshot, "a");
        assert_eq!(ready(&snapshot, std::slice::from_ref(&a)), [create(1)]);
        let b = done(&snapshot, "b");
        assert_eq!(ready(&snapshot, &[a, b]), [create(0)]);
    }

    /// D2 "rows win": a node with a `done` row stays done whatever its in-edges say now, and its
    /// out-edges are taken.
    #[test]
    fn rows_win_a_done_node_stays_done() {
        let snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2)],
            &[("a", "b", IF), ("b", "c", NEXT)],
        );
        let mut a = done(&snapshot, "a");
        a.branch = Some(json!({ "b": false }));
        let b = done(&snapshot, "b");
        assert_eq!(ready(&snapshot, &[a, b]), [create(2)]);
    }

    /// D7: the region of a loop is every node on a forward path from target to source, in
    /// forward-topological order.
    #[test]
    fn region_over_a_diamond_is_every_node_on_a_path() {
        let snapshot = diamond();
        let graph = Graph::of(&snapshot);
        assert_eq!(
            graph.region("b", "d"),
            [1, 3],
            "`c` is not on a `b → d` path"
        );
        assert_eq!(graph.region("a", "d"), [0, 1, 2, 3]);
        assert_eq!(graph.region("b", "b"), [1]);
        assert!(graph.region("b", "c").is_empty(), "`b` does not reach `c`");
        assert!(graph.region("x", "d").is_empty(), "an unknown key");
    }

    /// A6: on a lift, the region is `target..=source` in ascending position, so the source is
    /// retired last.
    #[test]
    fn region_on_a_lift_is_target_through_source_source_last() {
        let (snapshot, _) = fixture();
        assert_eq!(snapshot.v, GraphSnapshot::V1, "RUN_1 holds a raw v1 blob");
        assert!(snapshot.edges.is_empty(), "the lift is `walk_edges`'");
        let graph = Graph::of(&snapshot);
        assert_eq!(graph.region("implement", "review"), [2, 3]);
        assert_eq!(graph.region("prd", "review"), [0, 1, 2, 3]);
    }

    /// Replaces `gate.rs`'s `the_loop_target_is_the_named_implement_phase` (A8): the lift's
    /// `changes_requested` edge targets the greatest earlier `implement`, else the phase before.
    #[test]
    fn loop_edge_on_the_lift_targets_the_named_implement() {
        let (mut snapshot, _) = fixture();
        let graph_of = |snapshot: &GraphSnapshot, source: &str| {
            Graph::of(snapshot)
                .loop_edge(source, EdgeKind::ChangesRequested)
                .map(|edge| edge.to.clone())
        };
        assert_eq!(graph_of(&snapshot, "review").as_deref(), Some("implement"));
        assert_eq!(
            Graph::of(&snapshot).loop_edge("review", EdgeKind::Fails),
            None
        );
        let template = snapshot.phases[0].clone();
        snapshot.phases = vec![
            node(&template, "plan", 0),
            node(&template, "implement", 1),
            node(&template, "docs", 2),
            node(&template, "review", 3),
        ];
        assert_eq!(
            graph_of(&snapshot, "review").as_deref(),
            Some("implement"),
            "the named `implement`, not the phase before"
        );
        snapshot.phases[1].name = "build".to_owned();
        assert_eq!(
            graph_of(&snapshot, "review").as_deref(),
            Some("docs"),
            "no `implement`: the phase immediately before"
        );
        snapshot.phases[0].name = "review".to_owned();
        snapshot.phases[3].name = "check".to_owned();
        assert_eq!(
            graph_of(&snapshot, "review"),
            None,
            "a review first in order"
        );
    }

    /// D9: a kind is optional exactly when a node forward-reachable from this one produces it;
    /// a later position on a parallel branch does not count.
    #[test]
    fn required_inputs_follows_forward_reach_on_a_branch() {
        let mut snapshot = graph(
            &[("a", 0), ("b", 1), ("c", 2), ("d", 3)],
            &[
                ("a", "b", NEXT),
                ("a", "c", NEXT),
                ("b", "d", NEXT),
                ("c", "d", NEXT),
            ],
        );
        snapshot.phases[1].input_kinds = vec!["a".into(), "c".into(), "d".into()];
        snapshot.phases[0].input_kinds = vec!["b".into(), "c".into(), "x".into()];
        let b = snapshot.phases[1].clone();
        let a = snapshot.phases[0].clone();
        assert_eq!(
            required_inputs(&b, &snapshot),
            ["a", "c"],
            "`c` sits at a later position but `b` does not reach it; `d` it does"
        );
        assert_eq!(
            required_inputs(&a, &snapshot),
            ["x"],
            "every reachable node's kind is optional"
        );
        let mut stranger = a;
        stranger.name = "judge".to_owned();
        assert_eq!(
            required_inputs(&stranger, &snapshot),
            ["b", "c", "x"],
            "a phase the snapshot does not name reaches nothing"
        );
    }

    /// D9 on every seeded kind's lift (v1 raw and v2 with the default edges): the later-position
    /// rule `engine.rs` used before M2.
    #[test]
    fn required_inputs_on_a_lift_is_the_later_position_rule() {
        let (fixture_snapshot, _) = fixture();
        let template = fixture_snapshot.phases[0].clone();
        for kind in htui_core::seed::KINDS {
            for v in [GraphSnapshot::V1, GraphSnapshot::V] {
                let mut snapshot = fixture_snapshot.clone();
                snapshot.v = v;
                snapshot.phases = kind
                    .phases
                    .iter()
                    .zip(0..)
                    .map(|(seed, position)| {
                        let mut phase = node(&template, seed.name, position);
                        phase.input_kinds =
                            seed.input_kinds.iter().map(|k| (*k).to_owned()).collect();
                        phase
                    })
                    .collect();
                snapshot.edges = if v == GraphSnapshot::V {
                    htui_core::model::default_snapshot_edges(&snapshot.phases)
                } else {
                    Vec::new()
                };
                for phase in &snapshot.phases {
                    let by_position: Vec<String> = phase
                        .input_kinds
                        .iter()
                        .filter(|kind| {
                            !snapshot.phases.iter().any(|later| {
                                later.position > phase.position && later.output_kind == **kind
                            })
                        })
                        .cloned()
                        .collect();
                    assert_eq!(
                        required_inputs(phase, &snapshot),
                        by_position,
                        "{} v{v} `{}`",
                        kind.name,
                        phase.name
                    );
                }
            }
        }
    }

    /// D2's fallback, reachable only on a hand-built snapshot (the validator refuses a forward
    /// cycle): the first node in position order that is not done, read as reached.
    #[test]
    fn a_forward_cycle_falls_back_to_the_position_rule() {
        let snapshot = graph(&[("a", 0), ("b", 1)], &[("a", "b", NEXT), ("b", "a", NEXT)]);
        assert_eq!(ready(&snapshot, &[]), [create(0)]);
        let a = done(&snapshot, "a");
        assert_eq!(ready(&snapshot, std::slice::from_ref(&a)), [create(1)]);
        let b = done(&snapshot, "b");
        assert_eq!(ready(&snapshot, &[a, b]), [Cursor::Finished]);
    }

    /// D2: palette nodes are not walked, so they never hold the run open.
    #[test]
    fn a_palette_node_is_not_walked() {
        let mut snapshot = graph(&[("a", 0), ("p", 1)], &[]);
        snapshot.phases[1].palette = true;
        assert_eq!(ready(&snapshot, &[]), [create(0)]);
        let a = done(&snapshot, "a");
        assert_eq!(ready(&snapshot, &[a]), [Cursor::Finished]);
    }
}

/// The parity oracle (plan D2, blueprint §3.1): on every seeded kind's lifted, linear snapshot
/// the walker answers exactly what the pre-M2 position scan answered, over an exhaustive set of
/// row states per position.
#[cfg(test)]
mod parity {
    use htui_core::model::{GraphSnapshot, RunStep, SnapshotPhase, StepId, StepStatus};

    use super::ready;
    use super::tests::{fixture, node};
    use crate::status::{Cursor, cursor, cursor_by_position};

    /// One row a letter of an alphabet writes at its node: `(attempt, fanout_index, status,
    /// selected)`.
    type Spec = (i32, i32, StepStatus, Option<bool>);

    /// The three forms of plan D2: a raw v1 blob (lifted by `walk_edges`), a v2 snapshot carrying
    /// the default edges, and a raw v1 blob whose positions are gapped (×10).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Form {
        V1,
        V2,
        Gapped,
    }

    /// Every seeded kind's snapshot in `form`, with node `fanned` (if any) at `fan_out = 3`.
    fn snapshots(form: Form, fanned: Option<usize>) -> Vec<(&'static str, GraphSnapshot)> {
        let (base, _) = fixture();
        let template = base.phases[0].clone();
        htui_core::seed::KINDS
            .iter()
            .map(|kind| {
                let mut snapshot = base.clone();
                let step = if form == Form::Gapped { 10 } else { 1 };
                snapshot.phases = kind
                    .phases
                    .iter()
                    .zip(0..)
                    .map(|(seed, index)| node(&template, seed.name, index * step))
                    .collect();
                if let Some(phase) = fanned.and_then(|index| snapshot.phases.get_mut(index)) {
                    phase.fan_out = 3;
                }
                if form == Form::V2 {
                    snapshot.v = GraphSnapshot::V;
                    snapshot.edges = htui_core::model::default_snapshot_edges(&snapshot.phases);
                } else {
                    snapshot.v = GraphSnapshot::V1;
                    snapshot.edges = Vec::new();
                }
                (kind.name, snapshot)
            })
            .collect()
    }

    /// Sweep A1's alphabet: ∅, one attempt-1 row at each status, and a superseded attempt 1
    /// under an attempt 2 at each status.
    fn single_alphabet() -> Vec<Vec<Spec>> {
        let mut alphabet = vec![Vec::new()];
        for &status in StepStatus::ALL {
            alphabet.push(vec![(1, 0, status, None)]);
        }
        for &status in StepStatus::ALL {
            alphabet.push(vec![
                (1, 0, StepStatus::Superseded, None),
                (2, 0, status, None),
            ]);
        }
        alphabet
    }

    /// Sweep A2's other nodes.
    fn plain_alphabet() -> Vec<Vec<Spec>> {
        let mut alphabet = vec![Vec::new()];
        for status in [
            StepStatus::Done,
            StepStatus::Pending,
            StepStatus::Failed,
            StepStatus::Superseded,
        ] {
            alphabet.push(vec![(1, 0, status, None)]);
        }
        alphabet
    }

    /// Sweep A2's fanned node: every three-candidate group over six statuses, with and without
    /// a selection and a running judge; the short groups; a whole group retired.
    fn fanned_alphabet() -> Vec<Vec<Spec>> {
        const STATUSES: [StepStatus; 6] = [
            StepStatus::Pending,
            StepStatus::Running,
            StepStatus::Done,
            StepStatus::Failed,
            StepStatus::Superseded,
            StepStatus::Cancelled,
        ];
        let mut alphabet = vec![Vec::new()];
        for a in STATUSES {
            for b in STATUSES {
                for c in STATUSES {
                    let statuses = [a, b, c];
                    let unselected: Vec<Spec> = statuses
                        .iter()
                        .zip(0..)
                        .map(|(&status, index)| (1, index, status, None))
                        .collect();
                    let mut groups = vec![unselected];
                    if let Some(winner) = statuses.iter().position(|s| *s == StepStatus::Done) {
                        groups.push(
                            statuses
                                .iter()
                                .zip(0..)
                                .map(|(&status, index)| {
                                    let selected = usize::try_from(index) == Ok(winner);
                                    (1, index, status, Some(selected))
                                })
                                .collect(),
                        );
                    }
                    for group in groups {
                        let mut judged = group.clone();
                        judged.push((1, -1, StepStatus::Running, None));
                        alphabet.push(group);
                        alphabet.push(judged);
                    }
                }
            }
        }
        let short = [StepStatus::Done, StepStatus::Pending];
        for a in short {
            alphabet.push(vec![(1, 0, a, None)]);
            for b in short {
                alphabet.push(vec![(1, 0, a, None), (1, 1, b, None)]);
            }
        }
        let retired: Vec<Spec> = (0..3)
            .map(|index| (1, index, StepStatus::Superseded, Some(false)))
            .collect();
        alphabet.push(retired.clone());
        for status in short {
            let mut group = retired.clone();
            group.push((2, 0, status, None));
            alphabet.push(group);
        }
        alphabet
    }

    /// Each letter of each node's alphabet, as rows, with ids minted once.
    fn letters(snapshot: &GraphSnapshot, alphabets: &[Vec<Vec<Spec>>]) -> Vec<Vec<Vec<RunStep>>> {
        let (_, template) = fixture();
        snapshot
            .phases
            .iter()
            .zip(alphabets)
            .map(|(phase, alphabet): (&SnapshotPhase, _)| {
                alphabet
                    .iter()
                    .map(|letter| {
                        letter
                            .iter()
                            .map(|&(attempt, index, status, selected)| {
                                let mut row = template.clone();
                                row.id = StepId::new();
                                row.position = phase.position;
                                row.phase_name.clone_from(&phase.name);
                                row.attempt = attempt;
                                row.fanout_index = index;
                                row.status = status;
                                row.selected = selected;
                                row.gate_outcome = None;
                                row.branch = None;
                                row
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }

    /// Every word over the nodes' alphabets: the walker, `status::cursor` and the position scan
    /// agree, and no lifted graph owes a decision or a loop. Answers the number of cases.
    fn sweep(name: &str, snapshot: &GraphSnapshot, alphabets: &[Vec<Vec<Spec>>]) -> usize {
        let letters = letters(snapshot, alphabets);
        let mut word = vec![0_usize; letters.len()];
        let mut cases = 0;
        let mut rows: Vec<RunStep> = Vec::new();
        loop {
            rows.clear();
            for (node, &letter) in word.iter().enumerate() {
                rows.extend(letters[node][letter].iter().cloned());
            }
            let old = cursor_by_position(snapshot, &rows);
            let walked = ready(snapshot, &rows);
            assert!(
                !walked
                    .iter()
                    .any(|c| matches!(c, Cursor::Decide { .. } | Cursor::Loop { .. })),
                "{name} {word:?}: a lifted graph owes no decision or loop: {walked:?}"
            );
            assert_eq!(
                walked.first(),
                Some(&old),
                "{name} {word:?}: walker vs scan"
            );
            assert_eq!(
                cursor(snapshot, &rows),
                old,
                "{name} {word:?}: cursor vs scan"
            );
            cases += 1;
            // The next word, little-endian.
            let mut digit = 0;
            loop {
                if digit == word.len() {
                    return cases;
                }
                word[digit] += 1;
                if word[digit] < letters[digit].len() {
                    break;
                }
                word[digit] = 0;
                digit += 1;
            }
        }
    }

    fn sweep_single(form: Form) {
        let mut cases = 0;
        for (name, snapshot) in snapshots(form, None) {
            let alphabets = vec![single_alphabet(); snapshot.phases.len()];
            cases += sweep(name, &snapshot, &alphabets);
        }
        // ANA 17², FEAT 17⁴, FIX, CLEAN and TOOL 17³.
        assert_eq!(
            cases,
            289 + 83_521 + 3 * 4_913,
            "{form:?}: the whole cross product"
        );
    }

    /// Sweep A1 over the raw v1 lift and the v2 default edges.
    #[test]
    fn a_linear_lift_walks_like_the_position_scan() {
        sweep_single(Form::V1);
        sweep_single(Form::V2);
    }

    /// Sweep A1 over a raw v1 lift with gapped positions.
    #[test]
    fn a_gapped_lift_walks_like_the_position_scan() {
        sweep_single(Form::Gapped);
    }

    /// Sweep A2: one node per kind fanned out three ways, chosen so the first, a middle and the
    /// last position are each fanned somewhere: `verdict`, FEAT's `plan`, `reproduce`, CLEAN's
    /// `review` and TOOL's `implement`.
    #[test]
    fn a_fanned_out_lift_walks_like_the_position_scan() {
        let fanned = fanned_alphabet();
        let plain = plain_alphabet();
        let mut kinds = 0;
        for (kind, index) in [1, 1, 0, 2, 1].into_iter().enumerate() {
            let (name, snapshot) = snapshots(Form::V1, Some(index)).swap_remove(kind);
            assert_eq!(name, htui_core::seed::KINDS[kind].name);
            let alphabets: Vec<Vec<Vec<Spec>>> = (0..snapshot.phases.len())
                .map(|node| {
                    if node == index {
                        fanned.clone()
                    } else {
                        plain.clone()
                    }
                })
                .collect();
            let words = fanned.len()
                * plain
                    .len()
                    .pow(u32::try_from(alphabets.len() - 1).unwrap_or(0));
            assert_eq!(
                sweep(name, &snapshot, &alphabets),
                words,
                "{name}: every word"
            );
            kinds += 1;
        }
        assert_eq!(kinds, htui_core::seed::KINDS.len());
    }
}
