//! MOD-141 D4, D5, D10: pure edits of a `GraphBody` (add after, delete with rewire, connect, edit
//! and remove a link, replace a node) and the client-side validator, the store's own rules in the
//! store's order. Every function takes a body and answers a new one or a sentence.

use chrono::{DateTime, Utc};
use htui_core::model::{EdgeId, EdgeKind, Gate, GraphBody, NodeBody, PhaseId, StepGraphEdge};
use htui_core::seed::{PhaseSeed, phase_row};
use htui_core::store::{
    body_agent_refs, body_persona_refs, graph_body_shape_refusal, graph_body_topology_refusal,
    graph_body_walk_refusal, references_no_row,
};

use super::{EdgeSpec, Lookups, forward_edges};
use crate::ui::kit::label;

/// D5: the store's checks in the store's order (`traits.rs:1091-1112`, `mem.rs:4208-4262`):
/// `graph_body_shape_refusal(body.graph.id, body)`; then each `body_persona_refs` id against
/// `lookups.personas` -> `references_no_row(column, id, "persona")`; then, only when
/// `lookups.agents` is `Some`, each `body_agent_refs` id -> `references_no_row(column, id,
/// "agent")`; then `graph_body_topology_refusal`; then `graph_body_walk_refusal` (W1-W5, then
/// the `if` closed set). Steps 4-5 of the store (name taken in scope, ids of other graphs) need
/// store rows and stay the store's. `None`: the body may be sent.
pub(crate) fn validate(body: &GraphBody, lookups: &Lookups) -> Option<String> {
    let graph = body.graph.id;
    if let Some(refusal) = graph_body_shape_refusal(graph, body) {
        return Some(refusal);
    }
    if let Some((column, persona)) = body_persona_refs(body)
        .into_iter()
        .find(|(_, persona)| !lookups.personas.iter().any(|known| known.id == *persona))
    {
        return Some(references_no_row(column, persona, "persona"));
    }
    if let Some(agents) = &lookups.agents
        && let Some((column, agent)) = body_agent_refs(body)
            .into_iter()
            .find(|(_, agent)| !agents.iter().any(|known| known.agent.id == *agent))
    {
        return Some(references_no_row(column, agent, "agent"));
    }
    graph_body_topology_refusal(graph, body).or_else(|| graph_body_walk_refusal(graph, body))
}

/// The refusal of an edit naming a step the body lacks.
pub(crate) const NO_SUCH_STEP: &str = "no such step in this graph";
/// The refusal of an edit naming a link the body lacks.
pub(crate) const NO_SUCH_LINK: &str = "no such link in this graph";
/// The refusal of a link from a step to itself.
pub(crate) const SELF_LINK: &str = "a step cannot link to itself";
/// The refusal of an add whose step id the body already holds.
pub(crate) const STEP_EXISTS: &str = "a step with this id already exists";

/// D10: turning palette on for a step that has links, in words.
pub(crate) fn palette_has_links(links: usize) -> String {
    format!(
        "a palette step has no links; delete its {} first",
        label::plural(links, "link", "links")
    )
}

/// D10: a fresh node for "add after `after`": `htui_core::seed::phase_row(PhaseId::new(),
/// body.graph.id, max position + 1 (0 for an empty graph), &PhaseSeed { name: "",
/// input_kinds: &[], gate_hard: false }, now)`, then `gate = Gate::Never`, `retry_limit = 0`,
/// `template_name` = `after`'s node's (empty when `after` is `None` or unknown), `output_kind`
/// empty (the form fills the name). No agents, no parts.
pub(crate) fn draft_node(body: &GraphBody, after: Option<PhaseId>, now: DateTime<Utc>) -> NodeBody {
    // Saturating: a body already at `i32::MAX` gets a clashing position, which `validate`
    // refuses in the store's words, rather than a panic.
    let position = body
        .nodes
        .iter()
        .map(|node| node.phase.position)
        .max()
        .map_or(0, |max| max.saturating_add(1));
    let seed = PhaseSeed {
        name: "",
        input_kinds: &[],
        gate_hard: false,
    };
    let mut phase = phase_row(PhaseId::new(), body.graph.id, position, &seed, now);
    phase.gate = Gate::Never;
    phase.retry_limit = 0;
    phase.template_name = after
        .and_then(|after| node_of(body, after))
        .map(|node| node.phase.template_name.clone())
        .unwrap_or_default();
    NodeBody {
        phase,
        agents: Vec::new(),
        parts: Vec::new(),
    }
}

/// D10 add: `node` appended to `body.nodes` (its `graph_id` set to the body's, agents and parts
/// re-tagged and renumbered 0..n). With `after` a flow node (not palette): every forward edge
/// out of `after` (`forward_edges`) is re-pointed to leave from `node` (same id, kind,
/// predicate, limit), then a `next` `after -> node` is added with `EdgeId::new()`. Back edges of
/// `after` stay. With `after` `None`, unknown (the selection can lag a reload) or a palette
/// node, or a palette **new node** (M2 D10): `node` is a root with no edges and takes nothing
/// over. Err `STEP_EXISTS` only when `node.phase.id` is already in the body.
pub(crate) fn add_after(
    body: &GraphBody,
    after: Option<PhaseId>,
    node: NodeBody,
) -> Result<GraphBody, String> {
    if node_of(body, node.phase.id).is_some() {
        return Err(STEP_EXISTS.to_owned());
    }
    let palette = node.phase.palette;
    let node = adopted(body, node);
    let added = node.phase.id;
    let mut next = body.clone();
    next.nodes.push(node);
    let flow = after
        .filter(|_| !palette)
        .filter(|&after| node_of(body, after).is_some_and(|node| !node.phase.palette));
    if let Some(after) = flow {
        for (edge, forward) in next.edges.iter_mut().zip(forward_edges(body)) {
            if forward && edge.from == after {
                edge.from = added;
            }
        }
        next.edges.push(StepGraphEdge {
            id: EdgeId::new(),
            graph_id: body.graph.id,
            from: after,
            to: added,
            kind: EdgeKind::Next,
            predicate: None,
            loop_limit: None,
        });
    }
    Ok(next)
}

/// D8: `node` replaces the node of the same phase id; position, edges and the graph row are
/// kept; agents and parts re-tagged (`phase_id`) and renumbered 0..n in order. Err
/// `NO_SUCH_STEP`; Err [`palette_has_links`] when `node` is a palette step and the body has
/// links touching it (M2 D10: the store refuses a palette node with edges).
pub(crate) fn replace_node(body: &GraphBody, node: NodeBody) -> Result<GraphBody, String> {
    let at = body
        .nodes
        .iter()
        .position(|kept| kept.phase.id == node.phase.id)
        .ok_or_else(|| NO_SUCH_STEP.to_owned())?;
    if node.phase.palette {
        let step = node.phase.id;
        let links = body
            .edges
            .iter()
            .filter(|edge| edge.from == step || edge.to == step)
            .count();
        if links > 0 {
            return Err(palette_has_links(links));
        }
    }
    let mut next = body.clone();
    let mut node = adopted(body, node);
    node.phase.position = next.nodes[at].phase.position;
    next.nodes[at] = node;
    Ok(next)
}

/// D10 delete: the node goes with every edge touching it. For each forward edge `u -> id`
/// (in body order) and each forward edge `id -> v` (in body order): an edge `u -> v` with the
/// in-edge's kind, predicate and loop limit; the first target reuses the in-edge's id, later
/// ones get `EdgeId::new()`. A rewired edge whose `(from, to, kind)` already exists, or with
/// `u == v`, is dropped. Back edges into or out of `id` are dropped. Positions of the other
/// nodes are left alone (gaps are legal). Err `NO_SUCH_STEP`.
pub(crate) fn delete_node(body: &GraphBody, id: PhaseId) -> Result<GraphBody, String> {
    if node_of(body, id).is_none() {
        return Err(NO_SUCH_STEP.to_owned());
    }
    let forward: Vec<&StepGraphEdge> = body
        .edges
        .iter()
        .zip(forward_edges(body))
        .filter_map(|(edge, forward)| forward.then_some(edge))
        .collect();
    let targets: Vec<PhaseId> = forward
        .iter()
        .filter(|edge| edge.from == id && edge.to != id)
        .map(|edge| edge.to)
        .collect();
    let mut next = body.clone();
    next.nodes.retain(|node| node.phase.id != id);
    next.edges.retain(|edge| edge.from != id && edge.to != id);
    for into in forward
        .iter()
        .filter(|edge| edge.to == id && edge.from != id)
    {
        for (nth, &to) in targets.iter().enumerate() {
            let taken = into.from == to
                || next
                    .edges
                    .iter()
                    .any(|edge| (edge.from, edge.to, edge.kind) == (into.from, to, into.kind));
            if !taken {
                next.edges.push(StepGraphEdge {
                    id: if nth == 0 { into.id } else { EdgeId::new() },
                    to,
                    ..(*into).clone()
                });
            }
        }
    }
    Ok(next)
}

/// D10 connect: a new edge `from -> to` (`EdgeId::new()`, the body's graph id) with `spec`.
/// Err `SELF_LINK` when `from == to`, `NO_SUCH_STEP` for an unknown end. A duplicate
/// `(from, to, kind)` is left to `validate` (topology rule c, the store's sentence).
pub(crate) fn connect(
    body: &GraphBody,
    from: PhaseId,
    to: PhaseId,
    spec: EdgeSpec,
) -> Result<GraphBody, String> {
    if from == to {
        return Err(SELF_LINK.to_owned());
    }
    if node_of(body, from).is_none() || node_of(body, to).is_none() {
        return Err(NO_SUCH_STEP.to_owned());
    }
    let mut next = body.clone();
    next.edges.push(StepGraphEdge {
        id: EdgeId::new(),
        graph_id: body.graph.id,
        from,
        to,
        kind: spec.kind,
        predicate: spec.predicate,
        loop_limit: spec.loop_limit,
    });
    Ok(next)
}

/// D9: the edge `id` takes `spec`'s kind, predicate and limit; ends kept. Err `NO_SUCH_LINK`.
pub(crate) fn edit_edge(body: &GraphBody, id: EdgeId, spec: EdgeSpec) -> Result<GraphBody, String> {
    let mut next = body.clone();
    let edge = next
        .edges
        .iter_mut()
        .find(|edge| edge.id == id)
        .ok_or_else(|| NO_SUCH_LINK.to_owned())?;
    edge.kind = spec.kind;
    edge.predicate = spec.predicate;
    edge.loop_limit = spec.loop_limit;
    Ok(next)
}

/// D10 links list: the edge `id` removed. Err `NO_SUCH_LINK`.
pub(crate) fn remove_edge(body: &GraphBody, id: EdgeId) -> Result<GraphBody, String> {
    let at = body
        .edges
        .iter()
        .position(|edge| edge.id == id)
        .ok_or_else(|| NO_SUCH_LINK.to_owned())?;
    let mut next = body.clone();
    next.edges.remove(at);
    Ok(next)
}

/// The node of step `id`, if the body has it.
fn node_of(body: &GraphBody, id: PhaseId) -> Option<&NodeBody> {
    body.nodes.iter().find(|node| node.phase.id == id)
}

/// `node` made a node of `body`: its `graph_id` the body's, its candidates and its default parts
/// each re-tagged to its own phase and renumbered 0..n in order.
fn adopted(body: &GraphBody, mut node: NodeBody) -> NodeBody {
    node.phase.graph_id = body.graph.id;
    let phase = node.phase.id;
    for (row, position) in node.agents.iter_mut().zip(0..) {
        row.phase_id = phase;
        row.position = position;
    }
    for (part, position) in node.parts.iter_mut().zip(0..) {
        part.phase_id = phase;
        part.position = position;
    }
    node
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{
        AgentId, EdgeId, EdgeKind, EdgePredicate, Gate, GraphBody, GraphScope, IfPredicate,
        NodeBody, PersonaId, PhaseAgent, PhaseId, PhasePart, ProjectId, Split, StepGraph,
        StepGraphEdge, StepGraphId,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{
        MemStore, WriteStore, edge_is_taken, edge_needs_loop_limit,
        if_predicate_outside_the_closed_set, loop_edge_runs_forward, phase_name_is_taken,
        references_no_row,
    };
    use serde_json::json;

    use super::*;
    use crate::catalogue::PersonaSummary;

    // ---- fixtures, mirroring `graph_rules.rs`'s ----

    /// A step named `name` at `position` of `graph`: the seed's row, its name everywhere.
    fn node(graph: StepGraphId, position: i32, name: &str) -> NodeBody {
        let seed = PhaseSeed {
            name: "",
            input_kinds: &[],
            gate_hard: false,
        };
        let mut phase = phase_row(PhaseId::new(), graph, position, &seed, Utc::now());
        phase.name = name.to_owned();
        phase.output_kind = name.to_owned();
        phase.template_name = name.to_owned();
        NodeBody {
            phase,
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// A link `from -> to`: a loop limit of 2 where the kind needs one, and `{"verify":
    /// "failed"}` on an `if`.
    fn edge(graph: StepGraphId, from: PhaseId, to: PhaseId, kind: EdgeKind) -> StepGraphEdge {
        StepGraphEdge {
            id: EdgeId::new(),
            graph_id: graph,
            from,
            to,
            kind,
            predicate: (kind == EdgeKind::If).then(|| EdgePredicate(json!({ "verify": "failed" }))),
            loop_limit: kind.needs_loop_limit().then_some(2),
        }
    }

    /// A project graph of `names` at positions 0.., no edges.
    fn body(names: &[&str]) -> GraphBody {
        let graph = StepGraphId::new();
        let now = Utc::now();
        GraphBody {
            graph: StepGraph {
                id: graph,
                scope: GraphScope::Project(ProjectId::new()),
                name: "reference".to_owned(),
                description: String::new(),
                is_override: false,
                revision: 4,
                created_at: now,
                updated_at: now,
            },
            nodes: names
                .iter()
                .zip(0..)
                .map(|(name, position)| node(graph, position, name))
                .collect(),
            edges: Vec::new(),
        }
    }

    /// The id of the step named `name`.
    fn id(body: &GraphBody, name: &str) -> PhaseId {
        body.nodes
            .iter()
            .find(|node| node.phase.name == name)
            .map(|node| node.phase.id)
            .unwrap_or_else(|| panic!("no step `{name}`"))
    }

    /// The reference graph: `implement -next-> review`, `review -changes_requested(3)->
    /// implement`, `implement -if {"verify":"failed"}-> fix`.
    fn reference() -> GraphBody {
        let mut body = body(&["implement", "review", "fix"]);
        let graph = body.graph.id;
        let (implement, review, fix) = (
            id(&body, "implement"),
            id(&body, "review"),
            id(&body, "fix"),
        );
        let mut changes = edge(graph, review, implement, EdgeKind::ChangesRequested);
        changes.loop_limit = Some(3);
        body.edges = vec![
            edge(graph, implement, review, EdgeKind::Next),
            changes,
            edge(graph, implement, fix, EdgeKind::If),
        ];
        body
    }

    /// The five seeded graphs of the demo's `htui` project, with the demo's personas and agents.
    async fn seeded() -> (Vec<GraphBody>, Lookups) {
        let store = MemStore::demo();
        let mut bodies = Vec::new();
        for graph in [
            ids::GRAPH_HTUI_ANA,
            ids::GRAPH_HTUI_FEAT,
            ids::GRAPH_HTUI_FIX,
            ids::GRAPH_HTUI_CLEAN,
            ids::GRAPH_HTUI_TOOL,
        ] {
            bodies.push(
                store
                    .graph_body(graph)
                    .await
                    .expect("a memory read")
                    .expect("a seeded graph"),
            );
        }
        let personas = store
            .personas()
            .await
            .expect("a memory read")
            .into_iter()
            .map(|persona| PersonaSummary {
                id: persona.id,
                name: persona.name,
            })
            .collect();
        let agents = store.agents().await.expect("a memory read");
        let lookups = Lookups {
            personas,
            agents: Some(agents),
            templates: None,
        };
        (bodies, lookups)
    }

    // ---- validate ----

    #[tokio::test]
    async fn validate_accepts_every_seeded_graph_and_the_reference_graph() {
        let (bodies, lookups) = seeded().await;
        assert_eq!(bodies.len(), 5);
        for body in &bodies {
            assert_eq!(validate(body, &lookups), None, "{}", body.graph.name);
        }
        assert_eq!(validate(&reference(), &Lookups::default()), None);
    }

    /// Each stage gives exactly the store function's sentence, and a body failing two stages
    /// gives the earlier one.
    #[test]
    fn validate_runs_the_store_rules_in_order() {
        let read_agents = Lookups {
            agents: Some(Vec::new()),
            ..Lookups::default()
        };
        let base = reference();
        let graph = base.graph.id;
        let (implement, fix) = (id(&base, "implement"), id(&base, "fix"));

        // Stage 6 (walk), the `if` closed set: last of all.
        let mut bogus = base.clone();
        let bogus_value = json!({ "bogus": 1 });
        bogus.edges[2].predicate = Some(EdgePredicate(bogus_value.clone()));
        let reason = IfPredicate::parse(&bogus_value)
            .expect_err("outside the closed set")
            .to_string();
        let if_sentence = if_predicate_outside_the_closed_set("implement", "fix", graph, &reason);
        assert_eq!(
            validate(&bogus, &read_agents).as_deref(),
            Some(if_sentence.as_str())
        );

        // Stage 6 (walk), W3: a forward `fails` comes before the `if` closed set.
        let mut forward = bogus.clone();
        forward
            .edges
            .push(edge(graph, implement, fix, EdgeKind::Fails));
        let forward_sentence = loop_edge_runs_forward(EdgeKind::Fails, "implement", "fix", graph);
        assert_eq!(
            validate(&forward, &read_agents).as_deref(),
            Some(forward_sentence.as_str())
        );

        // Stage 5 (topology): a `fails` with no limit comes before the walk.
        let mut unlimited = forward.clone();
        unlimited.edges[3].loop_limit = None;
        let unlimited_sentence = edge_needs_loop_limit(unlimited.edges[3].id, EdgeKind::Fails);
        assert_eq!(
            validate(&unlimited, &read_agents).as_deref(),
            Some(unlimited_sentence.as_str())
        );

        // Stage 4 (agents, once read): an unknown agent comes before the topology.
        let mut agent = unlimited.clone();
        let stranger = AgentId::new();
        agent.nodes[0].agents.push(PhaseAgent {
            phase_id: implement,
            position: 0,
            agent_id: stranger,
            model: "m".to_owned(),
        });
        let agent_sentence = references_no_row("phase_agent.agent_id", stranger, "agent");
        assert_eq!(
            validate(&agent, &read_agents).as_deref(),
            Some(agent_sentence.as_str())
        );

        // Stage 3 (personas): an unknown persona comes before the agents.
        let mut persona = agent.clone();
        let nobody = PersonaId::new();
        persona.nodes[1].phase.persona_id = Some(nobody);
        let persona_sentence = references_no_row("step_graph_phase.persona_id", nobody, "persona");
        assert_eq!(
            validate(&persona, &read_agents).as_deref(),
            Some(persona_sentence.as_str())
        );

        // Stage 1 (shape): a duplicate name comes first of all.
        let mut shape = persona.clone();
        shape.nodes[2].phase.name = "implement".to_owned();
        let shape_sentence = phase_name_is_taken("implement", graph);
        assert_eq!(
            validate(&shape, &read_agents).as_deref(),
            Some(shape_sentence.as_str())
        );
    }

    /// While the agents are not read, an unknown agent is the store's to refuse.
    #[test]
    fn validate_leaves_agents_to_the_store_until_they_are_read() {
        let mut body = reference();
        let implement = id(&body, "implement");
        body.nodes[0].agents.push(PhaseAgent {
            phase_id: implement,
            position: 0,
            agent_id: AgentId::new(),
            model: "m".to_owned(),
        });
        assert_eq!(validate(&body, &Lookups::default()), None);
    }

    // ---- edits ----

    /// The seeded `feature` graph (`prd -> plan -> implement -> review`, `review
    /// -changes_requested-> implement`) with the demo's lookups.
    async fn feature() -> (GraphBody, Lookups) {
        let (bodies, lookups) = seeded().await;
        let feature = bodies
            .into_iter()
            .find(|body| body.graph.name == "feature")
            .expect("the seeded feature graph");
        (feature, lookups)
    }

    /// A draft after `after`, named `name`.
    fn named(body: &GraphBody, after: Option<PhaseId>, name: &str) -> NodeBody {
        let mut node = draft_node(body, after, Utc::now());
        node.phase.name = name.to_owned();
        node.phase.output_kind = name.to_owned();
        node
    }

    /// The edge `from -> to` of `kind`.
    fn link(body: &GraphBody, from: PhaseId, to: PhaseId, kind: EdgeKind) -> StepGraphEdge {
        body.edges
            .iter()
            .find(|edge| (edge.from, edge.to, edge.kind) == (from, to, kind))
            .cloned()
            .unwrap_or_else(|| panic!("no `{kind}` link"))
    }

    #[test]
    fn draft_node_takes_the_selected_template_gate_never_retry_0_and_the_next_position() {
        let mut graph = reference();
        graph.nodes[2].phase.position = 7;
        graph.nodes[1].phase.template_name = "review-v2".to_owned();
        let review = id(&graph, "review");
        let now = Utc::now();

        let draft = draft_node(&graph, Some(review), now);
        assert_eq!(draft.phase.template_name, "review-v2");
        assert_eq!(draft.phase.gate, Gate::Never);
        assert_eq!(draft.phase.retry_limit, 0);
        assert_eq!(draft.phase.position, 8, "max position + 1, over a gap");
        assert_eq!(draft.phase.graph_id, graph.graph.id);
        assert_eq!(draft.phase.updated_at, now);
        assert_eq!(draft.phase.name, "", "the form fills the name");
        assert_eq!(draft.phase.output_kind, "");
        assert!(draft.agents.is_empty() && draft.parts.is_empty());
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.phase.id != draft.phase.id),
            "a fresh id"
        );
        assert_ne!(
            draft_node(&graph, Some(review), now).phase.id,
            draft.phase.id
        );
        for after in [None, Some(PhaseId::new())] {
            assert_eq!(draft_node(&graph, after, now).phase.template_name, "");
        }
        assert_eq!(draft_node(&body(&[]), None, now).phase.position, 0);
    }

    #[tokio::test]
    async fn add_after_takes_over_the_forward_out_edges_and_links_from_the_selected() {
        let (feature, lookups) = feature().await;
        let (plan, implement, review) = (
            id(&feature, "plan"),
            id(&feature, "implement"),
            id(&feature, "review"),
        );
        let old = link(&feature, plan, implement, EdgeKind::Next);
        let looped = link(&feature, review, implement, EdgeKind::ChangesRequested);

        let new = named(&feature, Some(plan), "security");
        let fresh = new.phase.id;
        let added = add_after(&feature, Some(plan), new).expect("a new step");

        assert_eq!(added.nodes.len(), feature.nodes.len() + 1);
        assert_eq!(added.nodes.last().map(|node| node.phase.id), Some(fresh));
        let taken = added
            .edges
            .iter()
            .find(|edge| edge.id == old.id)
            .expect("the old edge id survives");
        assert_eq!(
            (taken.from, taken.to, taken.kind),
            (fresh, implement, EdgeKind::Next)
        );
        let into = link(&added, plan, fresh, EdgeKind::Next);
        assert_ne!(into.id, old.id);
        assert_eq!((into.predicate, into.loop_limit), (None, None));
        assert_eq!(into.graph_id, feature.graph.id);
        assert!(added.edges.contains(&looped), "the loop is untouched");
        assert_eq!(added.edges.len(), feature.edges.len() + 1);
        assert_eq!(validate(&added, &lookups), None);
    }

    #[tokio::test]
    async fn add_after_leaves_back_edges_where_they_were() {
        let (feature, lookups) = feature().await;
        let (implement, review) = (id(&feature, "implement"), id(&feature, "review"));
        let looped = link(&feature, review, implement, EdgeKind::ChangesRequested);

        let new = named(&feature, Some(review), "release");
        let fresh = new.phase.id;
        let added = add_after(&feature, Some(review), new).expect("a new step");

        assert!(
            added.edges.contains(&looped),
            "the loop still leaves review"
        );
        link(&added, review, fresh, EdgeKind::Next);
        assert_eq!(added.edges.len(), feature.edges.len() + 1);
        assert_eq!(validate(&added, &lookups), None);
    }

    #[test]
    fn add_with_no_selection_or_a_palette_selection_adds_a_root() {
        let mut graph = reference();
        graph.edges.retain(|edge| edge.kind != EdgeKind::If);
        let fix = id(&graph, "fix");
        graph.nodes[2].phase.palette = true;
        assert_eq!(validate(&graph, &Lookups::default()), None);

        for after in [None, Some(PhaseId::new()), Some(fix)] {
            let new = named(&graph, after, "root");
            let fresh = new.phase.id;
            let added = add_after(&graph, after, new).expect("a root");
            assert_eq!(
                added.edges, graph.edges,
                "{after:?}: no edges move or appear"
            );
            assert_eq!(added.nodes.last().map(|node| node.phase.id), Some(fresh));
            assert_eq!(validate(&added, &Lookups::default()), None);
        }

        let mut stray = named(&graph, None, "stray");
        stray.phase.graph_id = StepGraphId::new();
        let added = add_after(&graph, None, stray).expect("a root");
        assert_eq!(
            added.nodes.last().map(|node| node.phase.graph_id),
            Some(graph.graph.id),
            "the body's graph"
        );

        assert_eq!(
            add_after(&graph, None, graph.nodes[0].clone()),
            Err(STEP_EXISTS.to_owned())
        );
    }

    /// D10: a palette step saved new is a root, whatever the selection.
    #[tokio::test]
    async fn a_palette_step_is_added_with_no_edges_and_takes_nothing_over() {
        let (feature, lookups) = feature().await;
        let plan = id(&feature, "plan");

        let mut new = named(&feature, Some(plan), "security");
        new.phase.palette = true;
        let fresh = new.phase.id;
        let added = add_after(&feature, Some(plan), new).expect("a palette step");

        assert_eq!(added.nodes.len(), feature.nodes.len() + 1);
        assert_eq!(added.nodes[..feature.nodes.len()], feature.nodes[..]);
        assert_eq!(
            added
                .nodes
                .last()
                .map(|node| (node.phase.id, node.phase.palette)),
            Some((fresh, true))
        );
        assert_eq!(added.edges, feature.edges, "no edge moves or appears");
        assert_eq!(validate(&added, &lookups), None);
    }

    /// D10: the store refuses a palette node with edges, so the editor says so first.
    #[test]
    fn palette_on_for_a_linked_step_is_refused_in_words() {
        let graph = reference();
        let links = |step: PhaseId| {
            graph
                .edges
                .iter()
                .filter(|edge| edge.from == step || edge.to == step)
                .count()
        };

        let mut review = graph.nodes[1].clone();
        review.phase.palette = true;
        let n = links(review.phase.id);
        assert_eq!(n, 2, "implement -> review and the loop back");
        assert_eq!(replace_node(&graph, review), Err(palette_has_links(n)));

        let mut fix = graph.nodes[2].clone();
        fix.phase.palette = true;
        assert_eq!(links(fix.phase.id), 1);
        assert_eq!(
            replace_node(&graph, fix),
            Err("a palette step has no links; delete its 1 link first".to_owned())
        );
        assert_eq!(
            palette_has_links(2),
            "a palette step has no links; delete its 2 links first"
        );
    }

    #[test]
    fn palette_on_for_an_unlinked_step_is_kept() {
        let mut graph = reference();
        graph.edges.retain(|edge| edge.kind != EdgeKind::If);
        let fix = id(&graph, "fix");
        let mut edited = graph.nodes[2].clone();
        edited.phase.palette = true;

        let replaced = replace_node(&graph, edited).expect("an unlinked step");
        let kept = &replaced.nodes[2];
        assert_eq!((kept.phase.id, kept.phase.palette), (fix, true));
        assert_eq!(replaced.edges, graph.edges);
        assert_eq!(replaced.nodes[..2], graph.nodes[..2]);
        assert_eq!(validate(&replaced, &Lookups::default()), None);
    }

    #[test]
    fn adopted_retags_and_renumbers_parts() {
        let graph = reference();
        let implement = id(&graph, "implement");
        let part = |phase_id: PhaseId, position: i32, key: &str| PhasePart {
            phase_id,
            position,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        };
        let mut edited = graph.nodes[0].clone();
        edited.phase.split = Split::Divide;
        edited.parts = vec![
            part(PhaseId::new(), 7, "api"),
            part(implement, 3, "ui"),
            part(PhaseId::new(), 0, "docs"),
        ];

        let replaced = replace_node(&graph, edited.clone()).expect("a known step");
        let parts = |node: &NodeBody| {
            node.parts
                .iter()
                .map(|part| (part.phase_id, part.position, part.key.clone()))
                .collect::<Vec<_>>()
        };
        let expected = vec![
            (implement, 0, "api".to_owned()),
            (implement, 1, "ui".to_owned()),
            (implement, 2, "docs".to_owned()),
        ];
        assert_eq!(parts(&replaced.nodes[0]), expected);

        let mut fresh = named(&graph, None, "split");
        let added_id = fresh.phase.id;
        fresh.parts = edited.parts;
        let added = add_after(&graph, None, fresh).expect("a new step");
        let last = added.nodes.last().expect("the new step");
        assert_eq!(
            parts(last),
            expected
                .into_iter()
                .map(|(_, position, key)| (added_id, position, key))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn replace_node_retags_and_renumbers_candidates_and_keeps_edges() {
        let graph = reference();
        let implement = id(&graph, "implement");
        let (first, second) = (AgentId::new(), AgentId::new());
        let mut edited = graph.nodes[0].clone();
        edited.phase.name = "build".to_owned();
        edited.phase.position = 99;
        edited.agents = vec![
            PhaseAgent {
                phase_id: PhaseId::new(),
                position: 5,
                agent_id: first,
                model: "a".to_owned(),
            },
            PhaseAgent {
                phase_id: implement,
                position: 2,
                agent_id: second,
                model: "b".to_owned(),
            },
        ];

        let replaced = replace_node(&graph, edited).expect("a known step");
        let kept = &replaced.nodes[0];
        assert_eq!(kept.phase.id, implement);
        assert_eq!(kept.phase.name, "build");
        assert_eq!(kept.phase.position, 0, "the position is kept");
        assert_eq!(
            kept.agents
                .iter()
                .map(|row| (row.phase_id, row.position, row.agent_id))
                .collect::<Vec<_>>(),
            vec![(implement, 0, first), (implement, 1, second)]
        );
        assert_eq!(replaced.edges, graph.edges);
        assert_eq!(replaced.nodes.len(), graph.nodes.len());
        assert_eq!(replaced.nodes[1..], graph.nodes[1..]);
        assert_eq!(validate(&replaced, &Lookups::default()), None);

        assert_eq!(
            replace_node(&graph, node(graph.graph.id, 5, "ghost")),
            Err(NO_SUCH_STEP.to_owned())
        );
    }

    #[test]
    fn delete_rewires_each_forward_in_edge_to_each_forward_out_target_keeping_its_kind() {
        let mut graph = body(&["a", "b", "x", "c", "d"]);
        let g = graph.graph.id;
        let [a, b, x, c, d] = ["a", "b", "x", "c", "d"].map(|name| id(&graph, name));
        graph.edges = vec![
            edge(g, a, x, EdgeKind::Next),
            edge(g, b, x, EdgeKind::If),
            edge(g, x, c, EdgeKind::Next),
            edge(g, x, d, EdgeKind::Next),
        ];
        let (from_a, from_b) = (graph.edges[0].id, graph.edges[1].id);
        let predicate = graph.edges[1].predicate.clone();
        assert!(predicate.is_some());

        let deleted = delete_node(&graph, x).expect("a known step");
        assert!(deleted.nodes.iter().all(|node| node.phase.id != x));
        assert_eq!(deleted.nodes.len(), 4);
        let shape: Vec<_> = deleted
            .edges
            .iter()
            .map(|edge| (edge.from, edge.to, edge.kind, edge.predicate.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (a, c, EdgeKind::Next, None),
                (a, d, EdgeKind::Next, None),
                (b, c, EdgeKind::If, predicate.clone()),
                (b, d, EdgeKind::If, predicate),
            ]
        );
        assert_eq!(deleted.edges[0].id, from_a, "the first target keeps the id");
        assert_eq!(deleted.edges[2].id, from_b);
        let mut ids: Vec<EdgeId> = deleted.edges.iter().map(|edge| edge.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4, "later targets get fresh ids");
        assert!(deleted.edges.iter().all(|edge| edge.graph_id == g));
        assert_eq!(validate(&deleted, &Lookups::default()), None);
    }

    #[test]
    fn delete_drops_duplicates_self_links_and_back_edges_on_the_node() {
        let mut graph = body(&["a", "x", "b"]);
        let g = graph.graph.id;
        let [a, x, b] = ["a", "x", "b"].map(|name| id(&graph, name));
        graph.edges = vec![
            edge(g, a, x, EdgeKind::Next),
            edge(g, x, b, EdgeKind::Next),
            edge(g, a, b, EdgeKind::Next),
            edge(g, b, x, EdgeKind::ChangesRequested),
            edge(g, x, a, EdgeKind::Fails),
        ];
        assert_eq!(validate(&graph, &Lookups::default()), None);
        let kept = graph.edges[2].clone();
        let deleted = delete_node(&graph, x).expect("a known step");
        assert_eq!(
            deleted.edges,
            vec![kept],
            "the rewired a -> b is a duplicate"
        );
        assert_eq!(validate(&deleted, &Lookups::default()), None);

        // Only a body that is already refused (a forward cycle) can rewire onto itself.
        let mut cycle = body(&["a", "x"]);
        let g = cycle.graph.id;
        let [a, x] = ["a", "x"].map(|name| id(&cycle, name));
        cycle.edges = vec![edge(g, a, x, EdgeKind::Next), edge(g, x, a, EdgeKind::Next)];
        let deleted = delete_node(&cycle, x).expect("a known step");
        assert!(deleted.edges.is_empty(), "a -> a is dropped");

        assert_eq!(
            delete_node(&graph, PhaseId::new()),
            Err(NO_SUCH_STEP.to_owned())
        );
    }

    /// C6: no valid body has a delete the walk refuses.
    #[tokio::test]
    async fn deleting_any_step_of_the_seeded_and_reference_graphs_validates() {
        let (mut bodies, lookups) = seeded().await;
        bodies.push(reference());
        for graph in &bodies {
            for node in &graph.nodes {
                let deleted = delete_node(graph, node.phase.id).expect("a known step");
                assert_eq!(deleted.nodes.len(), graph.nodes.len() - 1);
                assert_eq!(
                    validate(&deleted, &lookups),
                    None,
                    "deleting `{}` of `{}`",
                    node.phase.name,
                    graph.graph.name
                );
            }
        }
    }

    #[test]
    fn connect_mints_an_edge_and_refuses_a_self_link() {
        let graph = reference();
        let g = graph.graph.id;
        let (implement, review, fix) = (
            id(&graph, "implement"),
            id(&graph, "review"),
            id(&graph, "fix"),
        );
        let next = EdgeSpec {
            kind: EdgeKind::Next,
            predicate: None,
            loop_limit: None,
        };
        let connected = connect(&graph, review, fix, next.clone()).expect("two known steps");
        assert_eq!(connected.edges[..3], graph.edges[..]);
        let minted = connected.edges.last().expect("a new edge");
        assert_eq!(
            (minted.graph_id, minted.from, minted.to, minted.kind),
            (g, review, fix, EdgeKind::Next)
        );
        assert_eq!((minted.predicate.clone(), minted.loop_limit), (None, None));
        assert!(graph.edges.iter().all(|edge| edge.id != minted.id));
        assert_eq!(validate(&connected, &Lookups::default()), None);

        let asks = EdgeSpec {
            kind: EdgeKind::AsksYou,
            predicate: None,
            loop_limit: Some(4),
        };
        let asked = connect(&graph, fix, review, asks).expect("two known steps");
        let minted = asked.edges.last().expect("a new edge");
        assert_eq!(
            (minted.kind, minted.loop_limit),
            (EdgeKind::AsksYou, Some(4))
        );

        assert_eq!(
            connect(&graph, fix, fix, next.clone()),
            Err(SELF_LINK.to_owned())
        );
        assert_eq!(
            connect(&graph, fix, PhaseId::new(), next.clone()),
            Err(NO_SUCH_STEP.to_owned())
        );
        assert_eq!(
            connect(&graph, PhaseId::new(), fix, next.clone()),
            Err(NO_SUCH_STEP.to_owned())
        );

        // A duplicate is the validator's, in the store's words.
        let twice = connect(&graph, implement, review, next).expect("two known steps");
        let sentence = edge_is_taken(implement, review, EdgeKind::Next, g);
        assert_eq!(
            validate(&twice, &Lookups::default()).as_deref(),
            Some(sentence.as_str())
        );
    }

    #[test]
    fn edit_edge_and_remove_edge_by_id_refuse_an_unknown_id() {
        let graph = reference();
        let chosen = graph.edges[2].clone();
        let spec = EdgeSpec {
            kind: EdgeKind::AsksYou,
            predicate: None,
            loop_limit: Some(1),
        };
        let edited = edit_edge(&graph, chosen.id, spec.clone()).expect("a known link");
        let after = &edited.edges[2];
        assert_eq!(
            (after.id, after.graph_id, after.from, after.to),
            (chosen.id, chosen.graph_id, chosen.from, chosen.to),
            "the ends are kept"
        );
        assert_eq!(
            (after.kind, after.predicate.clone(), after.loop_limit),
            (EdgeKind::AsksYou, None, Some(1))
        );
        assert_eq!(edited.edges[..2], graph.edges[..2]);
        assert_eq!(
            edit_edge(&graph, EdgeId::new(), spec),
            Err(NO_SUCH_LINK.to_owned())
        );

        let removed = remove_edge(&graph, graph.edges[1].id).expect("a known link");
        assert_eq!(
            removed.edges,
            vec![graph.edges[0].clone(), graph.edges[2].clone()]
        );
        assert_eq!(
            remove_edge(&graph, EdgeId::new()),
            Err(NO_SUCH_LINK.to_owned())
        );
    }

    #[test]
    fn no_op_touches_the_graph_row() {
        let graph = reference();
        let (implement, review, fix) = (
            id(&graph, "implement"),
            id(&graph, "review"),
            id(&graph, "fix"),
        );
        let spec = EdgeSpec {
            kind: EdgeKind::Next,
            predicate: None,
            loop_limit: None,
        };
        let mut renamed = graph.nodes[2].clone();
        renamed.phase.name = "patch".to_owned();
        let outcomes = [
            add_after(
                &graph,
                Some(implement),
                named(&graph, Some(implement), "lint"),
            ),
            add_after(&graph, None, named(&graph, None, "root")),
            replace_node(&graph, renamed),
            delete_node(&graph, implement),
            delete_node(&graph, review),
            connect(&graph, review, fix, spec.clone()),
            edit_edge(&graph, graph.edges[0].id, spec),
            remove_edge(&graph, graph.edges[0].id),
        ];
        for outcome in outcomes {
            let edited = outcome.expect("every op applies");
            assert_eq!(edited.graph, graph.graph);
        }
    }
}
