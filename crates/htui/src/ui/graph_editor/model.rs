//! MOD-141 D4, D5, D10: pure edits of a `GraphBody` (add after, delete with rewire, connect, edit
//! and remove a link, replace a node) and the client-side validator, the store's own rules in the
//! store's order. Every function takes a body and answers a new one or a sentence.

use htui_core::model::GraphBody;
use htui_core::store::{
    body_agent_refs, body_persona_refs, graph_body_shape_refusal, graph_body_topology_refusal,
    graph_body_walk_refusal, references_no_row,
};

use super::Lookups;

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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{
        AgentId, EdgeId, EdgeKind, EdgePredicate, GraphBody, GraphScope, IfPredicate, NodeBody,
        PersonaId, PhaseAgent, PhaseId, ProjectId, StepGraph, StepGraphEdge, StepGraphId,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{
        MemStore, WriteStore, edge_needs_loop_limit, if_predicate_outside_the_closed_set,
        loop_edge_runs_forward, phase_name_is_taken, references_no_row,
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
}
