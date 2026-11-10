//! MOD-141 D7, D11: a `GraphBody` drawn as a rataflow `Flow`. Forward edges (`forward_edges`) are
//! layered by longest path; back edges leave and enter by the sides (`changes_requested` and a
//! back `asks_you` on the right, `fails` on the left); palette nodes sit in a band below. The flow
//! keeps no selection of its own and never sees a key; the mouse reaches it only through
//! `Canvas::on_mouse`, and `FlowAction::Delete` is never routed to it.

use std::collections::HashMap;

use htui_core::model::{EdgeKind, GraphBody, PhaseId, StepGraphEdge};
use serde_json::Value;

use super::forward_edges;
use crate::ui::cells;

/// D7: a node's width in cells at zoom 1: a border and 22 cells for the template, the gate and
/// the agents lines.
pub(crate) const NODE_W: f64 = 24.0;
/// D7: a node's height, a border, three lines and a border.
pub(crate) const NODE_H: f64 = 5.0;
/// D7: the columns between two nodes of one layer, room for a side loop between them.
pub(crate) const H_GAP: f64 = 6.0;
/// D7: the rows between two layers, which keep a label off the arrowhead (execution_graph R7).
pub(crate) const V_GAP: f64 = 3.0;
/// The x distance from one node of a row to the next.
const X_STEP: f64 = NODE_W + H_GAP;
/// The y distance from one layer to the next.
const Y_STEP: f64 = NODE_H + V_GAP;
/// The widest an edge label is drawn, in cells.
const LABEL_W: usize = 24;
/// The sign between a relation and its loop limit.
const TIMES: char = '\u{d7}';

/// Layout of one body, rataflow-free (unit-tested alone).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// Every node's top-left corner in world units: flow layers top to bottom, left to right,
    /// then the palette band.
    pub(crate) positions: Vec<(PhaseId, (f64, f64))>,
    /// Flow layers (palette band not counted).
    pub(crate) layers: usize,
    /// The widest row in world units (0.0 for an empty body).
    pub(crate) width: f64,
}

/// The width in world units of a row of `n` nodes (`n >= 1`).
fn span(n: usize) -> f64 {
    let n = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    n.mul_add(NODE_W, (n - 1.0) * H_GAP)
}

/// `n` as world units.
fn units(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// D7: `body` laid out by longest path over its forward edges, deterministic and independent of
/// the order of `body.edges`. A body the store would refuse (a forward cycle) still lays out:
/// the steps the walk never reaches each take a layer of their own below the rest.
pub(crate) fn layout(body: &GraphBody) -> Layout {
    let mask = forward_edges(body);
    // Flow nodes in body order (position order), palette nodes apart.
    let flow: Vec<usize> = (0..body.nodes.len())
        .filter(|at| !body.nodes[*at].phase.palette)
        .collect();
    let palette: Vec<usize> = (0..body.nodes.len())
        .filter(|at| body.nodes[*at].phase.palette)
        .collect();
    let slot: HashMap<PhaseId, usize> = flow
        .iter()
        .enumerate()
        .map(|(slot, at)| (body.nodes[*at].phase.id, slot))
        .collect();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); flow.len()];
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); flow.len()];
    let mut indegree = vec![0_usize; flow.len()];
    for (edge, forward) in body.edges.iter().zip(&mask) {
        let (Some(&from), Some(&to)) = (slot.get(&edge.from), slot.get(&edge.to)) else {
            continue;
        };
        if !*forward || from == to {
            continue;
        }
        successors[from].push(to);
        predecessors[to].push(from);
        indegree[to] += 1;
    }
    // Kahn's algorithm, the longest path from a root.
    let mut layer = vec![0_usize; flow.len()];
    let mut reached = vec![false; flow.len()];
    let mut queue: std::collections::VecDeque<usize> = (0..flow.len())
        .filter(|slot| indegree[*slot] == 0)
        .collect();
    while let Some(from) = queue.pop_front() {
        reached[from] = true;
        for &to in &successors[from] {
            layer[to] = layer[to].max(layer[from] + 1);
            indegree[to] -= 1;
            if indegree[to] == 0 {
                queue.push_back(to);
            }
        }
    }
    // Only a forward cycle leaves a step unreached: each takes its own layer below the rest.
    let mut next = (0..flow.len())
        .filter(|slot| reached[*slot])
        .map(|slot| layer[slot] + 1)
        .max()
        .unwrap_or(0);
    for slot in 0..flow.len() {
        if !reached[slot] {
            layer[slot] = next;
            next += 1;
        }
    }
    let layers = (0..flow.len())
        .map(|slot| layer[slot] + 1)
        .max()
        .unwrap_or(0);
    // Order within a layer, top-down: by the leftmost forward predecessor above, then position.
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for slot in 0..flow.len() {
        rows[layer[slot]].push(slot);
    }
    let mut order = vec![usize::MAX; flow.len()];
    for row in &mut rows {
        let key = |slot: usize| {
            let above = predecessors[slot]
                .iter()
                .filter(|from| layer[**from] < layer[slot])
                .map(|from| order[*from])
                .min()
                .unwrap_or(usize::MAX);
            (above, body.nodes[flow[slot]].phase.position, slot)
        };
        row.sort_by_key(|slot| key(*slot));
        for (at, slot) in row.iter().enumerate() {
            order[*slot] = at;
        }
    }
    let width = rows
        .iter()
        .map(Vec::len)
        .chain(core::iter::once(palette.len()))
        .filter(|n| *n > 0)
        .map(span)
        .fold(0.0, f64::max);
    let mut positions = Vec::with_capacity(body.nodes.len());
    for (at, row) in rows.iter().enumerate() {
        // Centred on the widest row, in whole cells (execution_graph E7).
        let left = ((width - span(row.len())) / 2.0).floor();
        for (i, slot) in row.iter().enumerate() {
            let id = body.nodes[flow[*slot]].phase.id;
            positions.push((id, (units(i).mul_add(X_STEP, left), units(at) * Y_STEP)));
        }
    }
    // One empty band between the last layer and the palette, which has no edges.
    let band = if layers > 0 {
        units(layers + 1) * Y_STEP
    } else {
        0.0
    };
    for (i, at) in palette.iter().enumerate() {
        positions.push((body.nodes[*at].phase.id, (units(i) * X_STEP, band)));
    }
    Layout {
        positions,
        layers,
        width,
    }
}

/// D7: the handle ids an edge leaves and enters by, `(source, target)`. A forward edge runs
/// bottom to top; a back `changes_requested` or `asks_you` loops on the right, a back `fails` on
/// the left. A back `next` or `if` cannot exist (the walk refuses it); it is drawn as forward.
pub(crate) const fn edge_handles(kind: EdgeKind, forward: bool) -> (&'static str, &'static str) {
    match (kind, forward) {
        (EdgeKind::ChangesRequested | EdgeKind::AsksYou, false) => ("r-out", "r-in"),
        (EdgeKind::Fails, false) => ("l-out", "l-in"),
        _ => ("bottom", "top"),
    }
}

/// D7: the label an edge carries, in words and at most [`LABEL_W`] cells; `None` for `next`.
pub(crate) fn edge_label(edge: &StepGraphEdge) -> Option<String> {
    let limit = |word: &str| match edge.loop_limit {
        Some(n) => format!("{word} {TIMES}{n}"),
        None => word.to_owned(),
    };
    let label = match edge.kind {
        EdgeKind::Next => return None,
        EdgeKind::If => match &edge.predicate {
            Some(predicate) => format!("if {}", predicate_words(&predicate.0)),
            None => "if".to_owned(),
        },
        EdgeKind::AsksYou => limit("asks you"),
        EdgeKind::ChangesRequested => limit("changes requested"),
        EdgeKind::Fails => limit("fails"),
    };
    Some(cells::clip(&label, LABEL_W))
}

/// An `if` predicate in words (ANA-33 §7.3's closed set); anything outside it as compact JSON.
fn predicate_words(value: &Value) -> String {
    let one = value
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.iter().next());
    let globs = |word: &str, globs: &Value| {
        let list = globs.as_array()?;
        let first = list.first()?.as_str()?;
        Some(match list.len() - 1 {
            0 => format!("{word} {first}"),
            more => format!("{word} {first} +{more}"),
        })
    };
    let words = one.and_then(|(key, inner)| match (key.as_str(), inner) {
        ("verify", Value::String(want)) if want == "failed" || want == "passed" => {
            Some(format!("verify {want}"))
        }
        ("item_kind", Value::String(prefix)) => Some(format!("kind {prefix}")),
        ("item_tag", Value::String(tag)) => Some(format!("tag {tag}")),
        ("any_changed_path_matches", list) => globs("any path", list),
        ("all_changed_paths_match", list) => globs("all paths", list),
        ("not", inner) => Some(format!("not ({})", predicate_words(inner))),
        ("all", Value::Array(list)) => Some(format!("all of {}", list.len())),
        ("any", Value::Array(list)) => Some(format!("any of {}", list.len())),
        _ => None,
    });
    words.unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EdgeId, EdgePredicate, GraphScope, NodeBody, StepGraph, StepGraphId, StepGraphPhase,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{MemStore, WriteStore};
    use serde_json::json;

    use super::*;

    /// A fixed clock: nothing drawn reads it.
    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// A library graph row with id `id`.
    fn graph_row(id: StepGraphId) -> StepGraph {
        StepGraph {
            id,
            scope: GraphScope::Library,
            name: "hand-built".to_owned(),
            description: String::new(),
            is_override: false,
            revision: 0,
            created_at: now(),
            updated_at: now(),
        }
    }

    /// A seeded-default node `name` at `position`, its template named after it.
    fn node(graph: StepGraphId, position: i32, name: &str) -> NodeBody {
        let seed = PhaseSeed {
            name: "step",
            input_kinds: &[],
            gate_hard: false,
        };
        NodeBody {
            phase: StepGraphPhase {
                name: name.to_owned(),
                template_name: name.to_owned(),
                output_kind: name.to_owned(),
                ..phase_row(PhaseId::new(), graph, position, &seed, now())
            },
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// The id of the step named `name` in `body`.
    fn id_of(body: &GraphBody, name: &str) -> PhaseId {
        body.nodes
            .iter()
            .find(|node| node.phase.name == name)
            .map(|node| node.phase.id)
            .unwrap_or_else(|| panic!("no step {name}"))
    }

    /// An edge `from -> to` of `body`'s graph, a loop limit of 1 where the kind needs one.
    fn edge(body: &GraphBody, from: &str, to: &str, kind: EdgeKind) -> StepGraphEdge {
        StepGraphEdge {
            id: EdgeId::new(),
            graph_id: body.graph.id,
            from: id_of(body, from),
            to: id_of(body, to),
            kind,
            predicate: None,
            loop_limit: kind.needs_loop_limit().then_some(1),
        }
    }

    /// A body of `names` at positions 0.., no edges.
    fn body(names: &[&str]) -> GraphBody {
        let graph = StepGraphId::new();
        GraphBody {
            graph: graph_row(graph),
            nodes: (0..)
                .zip(names)
                .map(|(position, name)| node(graph, position, name))
                .collect(),
            edges: Vec::new(),
        }
    }

    /// `{"verify": "failed"}`.
    fn verify_failed() -> Option<EdgePredicate> {
        Some(EdgePredicate(json!({"verify": "failed"})))
    }

    /// The reference graph: `implement -next-> review`, `review -changes_requested(3)->
    /// implement`, `implement -if {"verify":"failed"}-> fix`.
    fn reference() -> GraphBody {
        let mut body = body(&["implement", "review", "fix"]);
        let mut back = edge(&body, "review", "implement", EdgeKind::ChangesRequested);
        back.loop_limit = Some(3);
        let mut branch = edge(&body, "implement", "fix", EdgeKind::If);
        branch.predicate = verify_failed();
        body.edges = vec![
            edge(&body, "implement", "review", EdgeKind::Next),
            back,
            branch,
        ];
        body
    }

    /// A seeded demo graph's body.
    async fn demo(graph: StepGraphId) -> GraphBody {
        MemStore::demo()
            .graph_body(graph)
            .await
            .expect("a memory read")
            .expect("the seeded graph")
    }

    /// The `(x, y)` of `name`'s node in `layout`.
    fn at(body: &GraphBody, layout: &Layout, name: &str) -> (f64, f64) {
        let id = id_of(body, name);
        layout
            .positions
            .iter()
            .find(|(at, _)| *at == id)
            .map(|(_, at)| *at)
            .unwrap_or_else(|| panic!("{name} is placed"))
    }

    /// The layer `name` sits in.
    fn layer_of(body: &GraphBody, layout: &Layout, name: &str) -> f64 {
        at(body, layout, name).1 / Y_STEP
    }

    #[tokio::test]
    async fn the_feature_seed_layers_one_step_per_layer() {
        let body = demo(ids::GRAPH_VULKAN_FEAT).await;
        let layout = layout(&body);
        assert_eq!(layout.layers, 4);
        for (layer, name) in [
            (0.0, "prd"),
            (1.0, "plan"),
            (2.0, "implement"),
            (3.0, "review"),
        ] {
            assert!(
                (layer_of(&body, &layout, name) - layer).abs() < f64::EPSILON,
                "{name}: {layout:?}"
            );
            assert!(at(&body, &layout, name).0.abs() < f64::EPSILON, "{name}");
        }
        assert!((layout.width - NODE_W).abs() < f64::EPSILON);
        assert!(
            body.edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::ChangesRequested),
            "the seed carries the review loop"
        );
    }

    #[test]
    fn back_edges_do_not_move_layers() {
        let mut body = body(&["plan", "build", "review"]);
        body.edges = vec![
            edge(&body, "plan", "build", EdgeKind::Next),
            edge(&body, "build", "review", EdgeKind::Next),
        ];
        let before = layout(&body);
        let mut back = edge(&body, "review", "plan", EdgeKind::AsksYou);
        back.loop_limit = None;
        body.edges.push(back);
        body.edges
            .push(edge(&body, "review", "build", EdgeKind::Fails));
        body.edges
            .push(edge(&body, "review", "build", EdgeKind::ChangesRequested));
        assert_eq!(layout(&body), before);
        assert_eq!(before.layers, 3);
    }

    #[test]
    fn an_if_branch_puts_both_targets_in_one_layer_centred() {
        let body = reference();
        let layout = layout(&body);
        assert_eq!(layout.layers, 2);
        assert_eq!(at(&body, &layout, "review"), (0.0, Y_STEP));
        assert_eq!(at(&body, &layout, "fix"), (X_STEP, Y_STEP));
        assert!((layout.width - span(2)).abs() < f64::EPSILON);
        // Centred on the two below, in whole cells.
        assert_eq!(
            at(&body, &layout, "implement"),
            (((span(2) - NODE_W) / 2.0).floor(), 0.0)
        );
    }

    #[test]
    fn palette_nodes_sit_in_their_own_band_below() {
        let mut body = body(&["plan", "triage", "build", "lint"]);
        body.nodes[1].phase.palette = true;
        body.nodes[3].phase.palette = true;
        body.edges = vec![edge(&body, "plan", "build", EdgeKind::Next)];
        let layout = layout(&body);
        assert_eq!(layout.layers, 2);
        assert_eq!(at(&body, &layout, "triage"), (0.0, 3.0 * Y_STEP));
        assert_eq!(at(&body, &layout, "lint"), (X_STEP, 3.0 * Y_STEP));
        // The palette row is the widest, so the flow is centred on it.
        assert!((layout.width - span(2)).abs() < f64::EPSILON);
        let left = ((span(2) - NODE_W) / 2.0).floor();
        assert_eq!(at(&body, &layout, "plan"), (left, 0.0));
        assert_eq!(at(&body, &layout, "build"), (left, Y_STEP));
        // Flow layers first, then the band.
        let order: Vec<PhaseId> = layout.positions.iter().map(|(id, _)| *id).collect();
        let names = ["plan", "build", "triage", "lint"].map(|name| id_of(&body, name));
        assert_eq!(order, names);

        let mut alone = self::body(&["triage"]);
        alone.nodes[0].phase.palette = true;
        let layout = super::layout(&alone);
        assert_eq!(layout.layers, 0);
        assert_eq!(
            layout.positions,
            vec![(alone.nodes[0].phase.id, (0.0, 0.0))]
        );
    }

    #[tokio::test]
    async fn layout_is_deterministic_and_ignores_edge_order() {
        for mut body in [demo(ids::GRAPH_VULKAN_FEAT).await, reference()] {
            let first = layout(&body);
            assert_eq!(layout(&body), first);
            body.edges.reverse();
            assert_eq!(layout(&body), first);
            body.edges.rotate_left(1);
            assert_eq!(layout(&body), first);
        }
    }

    #[test]
    fn a_forward_cycle_still_lays_out() {
        let mut body = body(&["root", "a", "b"]);
        body.edges = vec![
            edge(&body, "root", "a", EdgeKind::Next),
            edge(&body, "a", "b", EdgeKind::Next),
            edge(&body, "b", "a", EdgeKind::Next),
        ];
        let layout = layout(&body);
        assert_eq!(layout.positions.len(), 3);
        let mut ys: Vec<f64> = layout.positions.iter().map(|(_, (_, y))| *y).collect();
        ys.dedup();
        assert_eq!(ys.len(), 3, "no two steps overlap: {layout:?}");

        let mut empty = self::body(&[]);
        empty.edges.clear();
        assert_eq!(
            super::layout(&empty),
            Layout {
                positions: Vec::new(),
                layers: 0,
                width: 0.0,
            }
        );
    }

    #[test]
    fn edge_handles_by_kind_and_direction() {
        for kind in super::super::RELATIONS {
            assert_eq!(edge_handles(kind, true), ("bottom", "top"), "{kind:?}");
        }
        assert_eq!(
            edge_handles(EdgeKind::ChangesRequested, false),
            ("r-out", "r-in")
        );
        assert_eq!(edge_handles(EdgeKind::AsksYou, false), ("r-out", "r-in"));
        assert_eq!(edge_handles(EdgeKind::Fails, false), ("l-out", "l-in"));
        assert_eq!(edge_handles(EdgeKind::Next, false), ("bottom", "top"));
        assert_eq!(edge_handles(EdgeKind::If, false), ("bottom", "top"));
    }

    #[test]
    fn edge_labels_read_in_words() {
        let body = body(&["a", "b"]);
        let labelled = |kind: EdgeKind, limit: Option<i32>, predicate: Option<Value>| {
            let mut edge = edge(&body, "a", "b", kind);
            edge.loop_limit = limit;
            edge.predicate = predicate.map(EdgePredicate);
            edge_label(&edge)
        };
        assert_eq!(labelled(EdgeKind::Next, None, None), None);
        assert_eq!(
            labelled(EdgeKind::AsksYou, None, None).as_deref(),
            Some("asks you")
        );
        assert_eq!(
            labelled(EdgeKind::AsksYou, Some(2), None).as_deref(),
            Some("asks you \u{d7}2")
        );
        assert_eq!(
            labelled(EdgeKind::ChangesRequested, Some(3), None).as_deref(),
            Some("changes requested \u{d7}3")
        );
        assert_eq!(
            labelled(EdgeKind::Fails, Some(1), None).as_deref(),
            Some("fails \u{d7}1")
        );
        let words = |predicate: Value| labelled(EdgeKind::If, None, Some(predicate));
        for (predicate, want) in [
            (json!({"verify": "failed"}), "if verify failed"),
            (json!({"verify": "passed"}), "if verify passed"),
            (json!({"item_kind": "FIX"}), "if kind FIX"),
            (json!({"item_tag": "x"}), "if tag x"),
            (
                json!({"any_changed_path_matches": ["src/**"]}),
                "if any path src/**",
            ),
            (
                json!({"any_changed_path_matches": ["src/**", "a", "b"]}),
                "if any path src/** +2",
            ),
            (
                json!({"all_changed_paths_match": ["docs/*"]}),
                "if all paths docs/*",
            ),
            (
                json!({"all_changed_paths_match": ["docs/*", "x"]}),
                "if all paths docs/* +1",
            ),
            (
                json!({"not": {"verify": "passed"}}),
                "if not (verify passed)",
            ),
            (
                json!({"all": [{"verify": "passed"}, {"item_tag": "x"}]}),
                "if all of 2",
            ),
            (json!({"any": [{"item_kind": "FIX"}]}), "if any of 1"),
            (json!({"bogus": 1}), "if {\"bogus\":1}"),
        ] {
            assert_eq!(
                words(predicate.clone()).as_deref(),
                Some(want),
                "{predicate}"
            );
        }
        // Clipped to 24 cells with an ellipsis.
        let long =
            words(json!({"item_tag": "a-very-long-tag-name-indeed"})).expect("an if has a label");
        assert_eq!(cells::cell_width(&long), LABEL_W);
        assert!(long.ends_with('\u{2026}'), "{long}");
    }
}
