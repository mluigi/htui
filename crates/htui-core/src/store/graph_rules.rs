//! MOD-137's pure graph rules (blueprint §T3.2): the store checks both `MemStore` and `PgStore`
//! decide before any write, so they refuse the same input with the same sentence (F6). The seed of
//! MOD-138's shared graph validator.

use std::collections::HashSet;

use crate::model::{
    AgentId, EdgePredicate, GraphBody, PersonaId, Split, StepGraphEdge, StepGraphId, StepGraphPhase,
};
use crate::prompt::template::TemplateRole;
use crate::store::traits::{
    already_exists, divide_columns_off_divide, edge_endpoint_outside_graph, edge_is_taken,
    edge_loop_limit_negative, edge_loops_to_itself, edge_needs_loop_limit, edge_predicate_mismatch,
    fan_out_below_one, max_parts_below_one, palette_phase_has_edges, parts_off_divide,
    phase_name_is_taken, phase_part_key_is_taken, phase_position_is_taken,
    phase_token_budget_refusal, reserved_phase_name, retry_limit_negative, row_names_another_graph,
    row_names_another_phase, split_fan_out_mismatch,
};

/// D7's node CHECKs on one row: `ck_phase_split_fan_out`, then `ck_phase_divide_columns`, then
/// `ck_phase_max_parts`; then 0001's column CHECKs `fan_out >= 1` and `retry_limit >= 0`, so
/// `MemStore` never stores a row Postgres refuses (`Split::None` with `fan_out 0` passes
/// `ck_phase_split_fan_out`); then a `token_budget` below 1, which no CHECK refuses but
/// `set_setting`'s `Phase` rung does ([`phase_token_budget_refusal`], review R1 B-L2). `None` when
/// the row passes. `create_phase` and `replace_graph` call it.
#[must_use]
pub fn phase_row_refusal(phase: &StepGraphPhase) -> Option<String> {
    if (phase.split == Split::Compete) != (phase.fan_out > 1) {
        return Some(split_fan_out_mismatch(
            &phase.name,
            phase.split,
            phase.fan_out,
        ));
    }
    let divide_columns = phase.planner_persona_id.is_some()
        || phase.planner_agent_id.is_some()
        || phase.planner_model.is_some()
        || phase.integrate_persona_id.is_some()
        || phase.integrate_agent_id.is_some()
        || phase.integrate_model.is_some()
        || phase.max_parts.is_some();
    if phase.split != Split::Divide && divide_columns {
        return Some(divide_columns_off_divide(&phase.name, phase.split));
    }
    if let Some(max_parts) = phase.max_parts.filter(|max_parts| *max_parts < 1) {
        return Some(max_parts_below_one(&phase.name, max_parts));
    }
    if phase.fan_out < 1 {
        return Some(fan_out_below_one(&phase.name, phase.fan_out));
    }
    if phase.retry_limit < 0 {
        return Some(retry_limit_negative(&phase.name, phase.retry_limit));
    }
    phase.token_budget.and_then(phase_token_budget_refusal)
}

/// The predicate an edge carries, a JSON `null` read as none (review R1 B-L1): serde reads
/// `Some(EdgePredicate(Value::Null))` back as `None`, so the topology rule judges an edge by this
/// and both stores write this, never `jsonb 'null'`.
#[must_use]
pub fn edge_predicate(edge: &StepGraphEdge) -> Option<&EdgePredicate> {
    edge.predicate
        .as_ref()
        .filter(|predicate| !predicate.0.is_null())
}

/// D10 step 3 (blueprint §T3.2): the body's own shape, before any store row is read. The first
/// failure, in this order:
///
/// - a. `body.graph.id` is not `graph` ([`row_names_another_graph`] `"step_graph.id"`);
/// - b. per node in body order, its `graph_id` (`"step_graph_phase.graph_id"`), then each of its
///   candidates and parts naming another phase ([`row_names_another_phase`] `"phase_agent"` /
///   `"phase_part"`); then each edge's `graph_id` (`"step_graph_edge.graph_id"`);
/// - c. per node, a reserved name ([`reserved_phase_name`]);
/// - d. per node, an id, then a position, then a name an earlier node already holds
///   ([`already_exists`] `"step_graph_phase"`, [`phase_position_is_taken`],
///   [`phase_name_is_taken`]);
/// - e. per node, a candidate position, then a part position, then a part key taken by an
///   earlier row of the node ([`already_exists`] `"phase_agent"` / `"phase_part"`
///   `"({phase}, {position})"`, [`phase_part_key_is_taken`]);
/// - f. an edge id an earlier edge holds ([`already_exists`] `"step_graph_edge"`).
#[must_use]
pub fn graph_body_shape_refusal(graph: StepGraphId, body: &GraphBody) -> Option<String> {
    if body.graph.id != graph {
        return Some(row_names_another_graph(
            "step_graph.id",
            body.graph.id,
            graph,
        ));
    }
    for node in &body.nodes {
        let phase = &node.phase;
        if phase.graph_id != graph {
            return Some(row_names_another_graph(
                "step_graph_phase.graph_id",
                phase.graph_id,
                graph,
            ));
        }
        if let Some(row) = node.agents.iter().find(|row| row.phase_id != phase.id) {
            return Some(row_names_another_phase(
                "phase_agent",
                row.phase_id,
                phase.id,
            ));
        }
        if let Some(row) = node.parts.iter().find(|row| row.phase_id != phase.id) {
            return Some(row_names_another_phase(
                "phase_part",
                row.phase_id,
                phase.id,
            ));
        }
    }
    if let Some(edge) = body.edges.iter().find(|edge| edge.graph_id != graph) {
        return Some(row_names_another_graph(
            "step_graph_edge.graph_id",
            edge.graph_id,
            graph,
        ));
    }
    if let Some(node) = body
        .nodes
        .iter()
        .find(|node| TemplateRole::of_name(&node.phase.name) != TemplateRole::Phase)
    {
        return Some(reserved_phase_name(&node.phase.name));
    }
    let (mut ids, mut positions, mut names) = (HashSet::new(), HashSet::new(), HashSet::new());
    for node in &body.nodes {
        let phase = &node.phase;
        if !ids.insert(phase.id) {
            return Some(already_exists("step_graph_phase", phase.id));
        }
        if !positions.insert(phase.position) {
            return Some(phase_position_is_taken(phase.position, graph));
        }
        if !names.insert(phase.name.as_str()) {
            return Some(phase_name_is_taken(&phase.name, graph));
        }
    }
    for node in &body.nodes {
        let phase = node.phase.id;
        let mut slots = HashSet::new();
        if let Some(row) = node.agents.iter().find(|row| !slots.insert(row.position)) {
            return Some(already_exists(
                "phase_agent",
                format!("({phase}, {})", row.position),
            ));
        }
        let mut slots = HashSet::new();
        if let Some(row) = node.parts.iter().find(|row| !slots.insert(row.position)) {
            return Some(already_exists(
                "phase_part",
                format!("({phase}, {})", row.position),
            ));
        }
        let mut keys = HashSet::new();
        if let Some(row) = node.parts.iter().find(|row| !keys.insert(row.key.as_str())) {
            return Some(phase_part_key_is_taken(&row.key, phase));
        }
    }
    let mut edges = HashSet::new();
    body.edges
        .iter()
        .find(|edge| !edges.insert(edge.id))
        .map(|edge| already_exists("step_graph_edge", edge.id))
}

/// D10 step 7 (blueprint §T3.2): the body's topology and node CHECKs, once every row it names
/// is known to exist. The first failure, in this order:
///
/// - a. per edge, `from`, then `to`, not a node of the body ([`edge_endpoint_outside_graph`]
///   `"from_phase_id"` / `"to_phase_id"`);
/// - b. per edge: [`edge_loops_to_itself`], [`edge_predicate_mismatch`] (a JSON `null`
///   predicate is none, [`edge_predicate`]),
///   [`edge_loop_limit_negative`], [`edge_needs_loop_limit`];
/// - c. an edge whose `(from, to, kind)` an earlier edge holds ([`edge_is_taken`]);
/// - d. per node, [`phase_row_refusal`];
/// - e. per edge, an endpoint (`from` first) on a `palette` node ([`palette_phase_has_edges`]);
/// - f. per node, parts on a node whose split is not `divide` ([`parts_off_divide`]).
#[must_use]
pub fn graph_body_topology_refusal(graph: StepGraphId, body: &GraphBody) -> Option<String> {
    let node = |id| body.nodes.iter().find(|node| node.phase.id == id);
    for edge in &body.edges {
        for (column, end) in [("from_phase_id", edge.from), ("to_phase_id", edge.to)] {
            if node(end).is_none() {
                return Some(edge_endpoint_outside_graph(column, end, graph));
            }
        }
    }
    for edge in &body.edges {
        if edge.from == edge.to {
            return Some(edge_loops_to_itself(edge.id));
        }
        if edge_predicate(edge).is_some() != edge.kind.takes_predicate() {
            return Some(edge_predicate_mismatch(edge.id, edge.kind));
        }
        match edge.loop_limit {
            Some(limit) if limit < 0 => return Some(edge_loop_limit_negative(edge.id, limit)),
            None if edge.kind.needs_loop_limit() => {
                return Some(edge_needs_loop_limit(edge.id, edge.kind));
            }
            _ => {}
        }
    }
    let mut triples = HashSet::new();
    if let Some(edge) = body
        .edges
        .iter()
        .find(|edge| !triples.insert((edge.from, edge.to, edge.kind)))
    {
        return Some(edge_is_taken(edge.from, edge.to, edge.kind, graph));
    }
    if let Some(refusal) = body
        .nodes
        .iter()
        .find_map(|node| phase_row_refusal(&node.phase))
    {
        return Some(refusal);
    }
    for edge in &body.edges {
        for end in [edge.from, edge.to] {
            if let Some(palette) = node(end).filter(|node| node.phase.palette) {
                return Some(palette_phase_has_edges(&palette.phase.name, edge.id));
            }
        }
    }
    body.nodes
        .iter()
        .find(|node| !node.parts.is_empty() && node.phase.split != Split::Divide)
        .map(|node| parts_off_divide(&node.phase.name, node.phase.split))
}

/// D10 step 6's walk, personas first: per node in body order, `persona_id`,
/// `planner_persona_id`, `integrate_persona_id`, then its parts' `persona_id`, each as the
/// `(column, id)` [`references_no_row`](crate::store::references_no_row) words against
/// `"persona"`.
#[must_use]
pub fn body_persona_refs(body: &GraphBody) -> Vec<(&'static str, PersonaId)> {
    let mut refs = Vec::new();
    for node in &body.nodes {
        let phase = &node.phase;
        refs.extend(
            [
                ("step_graph_phase.persona_id", phase.persona_id),
                (
                    "step_graph_phase.planner_persona_id",
                    phase.planner_persona_id,
                ),
                (
                    "step_graph_phase.integrate_persona_id",
                    phase.integrate_persona_id,
                ),
            ]
            .into_iter()
            .filter_map(|(column, id)| id.map(|id| (column, id))),
        );
        refs.extend(
            node.parts
                .iter()
                .filter_map(|part| part.persona_id.map(|id| ("phase_part.persona_id", id))),
        );
    }
    refs
}

/// D10 step 6's walk after the personas: per node in body order, `planner_agent_id`,
/// `integrate_agent_id`, its candidates' `agent_id`, then its parts' `agent_id`, each as the
/// `(column, id)` [`references_no_row`](crate::store::references_no_row) words against
/// `"agent"`.
#[must_use]
pub fn body_agent_refs(body: &GraphBody) -> Vec<(&'static str, AgentId)> {
    let mut refs = Vec::new();
    for node in &body.nodes {
        let phase = &node.phase;
        refs.extend(
            [
                ("step_graph_phase.planner_agent_id", phase.planner_agent_id),
                (
                    "step_graph_phase.integrate_agent_id",
                    phase.integrate_agent_id,
                ),
            ]
            .into_iter()
            .filter_map(|(column, id)| id.map(|id| (column, id))),
        );
        refs.extend(
            node.agents
                .iter()
                .map(|row| ("phase_agent.agent_id", row.agent_id)),
        );
        refs.extend(
            node.parts
                .iter()
                .filter_map(|part| part.agent_id.map(|id| ("phase_part.agent_id", id))),
        );
    }
    refs
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        body_agent_refs, body_persona_refs, edge_predicate, graph_body_shape_refusal,
        graph_body_topology_refusal, phase_row_refusal,
    };
    use crate::model::{
        AgentId, CommandQueue, EdgeId, EdgeKind, EdgePredicate, Gate, GraphBody, GraphScope,
        NodeBody, PersonaId, PhaseAgent, PhaseId, PhasePart, ProjectId, Split, StepGraph,
        StepGraphEdge, StepGraphId, StepGraphPhase,
    };
    use crate::store::traits::{
        already_exists, divide_columns_off_divide, edge_endpoint_outside_graph, edge_is_taken,
        edge_loop_limit_negative, edge_loops_to_itself, edge_needs_loop_limit,
        edge_predicate_mismatch, palette_phase_has_edges, parts_off_divide, phase_name_is_taken,
        phase_part_key_is_taken, phase_position_is_taken, reserved_phase_name,
        row_names_another_graph, row_names_another_phase, split_fan_out_mismatch,
    };
    use crate::store::traits::{
        fan_out_below_one, phase_token_budget_refusal, retry_limit_negative,
    };

    fn phase(split: Split, fan_out: i32) -> StepGraphPhase {
        StepGraphPhase {
            id: PhaseId::new(),
            graph_id: StepGraphId::new(),
            position: 0,
            name: "implement".to_owned(),
            fan_out,
            gate: Gate::Never,
            gate_hard: false,
            retry_limit: 0,
            input_kinds: Vec::new(),
            output_kind: "diff".to_owned(),
            isolation: None,
            command_queue: CommandQueue::Off,
            verify_command: None,
            template_name: "implement".to_owned(),
            template_version: None,
            token_budget: None,
            persona_id: None,
            split,
            palette: false,
            plan_scope: None,
            planner_persona_id: None,
            planner_agent_id: None,
            planner_model: None,
            integrate_persona_id: None,
            integrate_agent_id: None,
            integrate_model: None,
            max_parts: None,
            updated_at: Utc::now(),
        }
    }

    /// `ck_phase_split_fan_out`: `compete` is exactly `fan_out > 1`, both ways.
    #[test]
    fn split_must_follow_fan_out() {
        assert_eq!(phase_row_refusal(&phase(Split::None, 1)), None);
        assert_eq!(phase_row_refusal(&phase(Split::Compete, 2)), None);
        assert_eq!(phase_row_refusal(&phase(Split::Divide, 1)), None);
        assert_eq!(
            phase_row_refusal(&phase(Split::None, 2)).as_deref(),
            Some(
                "phase `implement` is split `none` with fan_out 2; `compete` is exactly \
                 fan_out > 1"
            )
        );
        assert_eq!(
            phase_row_refusal(&phase(Split::Compete, 1)).as_deref(),
            Some(
                "phase `implement` is split `compete` with fan_out 1; `compete` is exactly \
                 fan_out > 1"
            )
        );
        assert!(phase_row_refusal(&phase(Split::Divide, 3)).is_some());
    }

    /// `ck_phase_divide_columns`: each of the seven columns alone refuses off a `divide` node.
    #[test]
    fn divide_columns_belong_to_a_divide_phase() {
        let setters: [fn(&mut StepGraphPhase); 7] = [
            |p| p.planner_persona_id = Some(PersonaId::new()),
            |p| p.planner_agent_id = Some(AgentId::new()),
            |p| p.planner_model = Some("m".to_owned()),
            |p| p.integrate_persona_id = Some(PersonaId::new()),
            |p| p.integrate_agent_id = Some(AgentId::new()),
            |p| p.integrate_model = Some("m".to_owned()),
            |p| p.max_parts = Some(2),
        ];
        for set in setters {
            let mut row = phase(Split::None, 1);
            set(&mut row);
            assert_eq!(
                phase_row_refusal(&row).as_deref(),
                Some(
                    "phase `implement` is split `none`; planner_*, integrate_* and max_parts \
                     belong to a `divide` phase"
                )
            );
            let mut row = phase(Split::Divide, 1);
            set(&mut row);
            assert_eq!(phase_row_refusal(&row), None, "a divide phase takes it");
        }
    }

    /// `ck_phase_max_parts`: a divide phase takes at least one part.
    #[test]
    fn max_parts_is_at_least_one() {
        let mut row = phase(Split::Divide, 1);
        row.max_parts = Some(0);
        assert_eq!(
            phase_row_refusal(&row).as_deref(),
            Some("phase `implement` has max_parts 0; a divide phase takes at least 1")
        );
        row.max_parts = Some(1);
        assert_eq!(phase_row_refusal(&row), None);
    }

    /// The order: the split rule first, then the divide columns, then `max_parts`.
    #[test]
    fn the_node_rules_fire_in_order() {
        let mut row = phase(Split::None, 2);
        row.max_parts = Some(0);
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("fan_out 2")
        );
        row.fan_out = 1;
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("belong to a `divide` phase")
        );
        row.split = Split::Divide;
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("max_parts 0")
        );
        row.max_parts = None;
        row.split = Split::None;
        row.fan_out = 0;
        row.retry_limit = -1;
        assert_eq!(
            phase_row_refusal(&row),
            Some(fan_out_below_one("implement", 0)),
            "0001's fan_out CHECK after the D7 ones"
        );
        row.fan_out = 1;
        assert_eq!(
            phase_row_refusal(&row),
            Some(retry_limit_negative("implement", -1)),
            "then 0001's retry_limit CHECK"
        );
    }

    /// 0001's `fan_out >= 1`: `Split::None` with `fan_out 0` passes `ck_phase_split_fan_out`
    /// (both sides false), so the column CHECK must be a rule of its own (MemStore would store
    /// a row Postgres refuses).
    #[test]
    fn fan_out_is_at_least_one() {
        assert_eq!(
            phase_row_refusal(&phase(Split::None, 0)).as_deref(),
            Some("phase `implement` has fan_out 0; a phase runs at least 1 candidate")
        );
        assert!(phase_row_refusal(&phase(Split::Divide, -3)).is_some());
        assert_eq!(phase_row_refusal(&phase(Split::None, 1)), None);
    }

    /// 0001's `retry_limit >= 0`.
    #[test]
    fn retry_limit_is_never_negative() {
        let mut row = phase(Split::None, 1);
        row.retry_limit = -1;
        assert_eq!(
            phase_row_refusal(&row).as_deref(),
            Some("phase `implement` has retry_limit -1; a retry limit is 0 or more")
        );
        row.retry_limit = 0;
        assert_eq!(phase_row_refusal(&row), None);
    }

    /// Review R1 B-L2: a `token_budget` below 1 is refused in `set_setting`'s `Phase`-rung
    /// sentence, after the CHECK rules; none, or 1 and up, passes.
    #[test]
    fn token_budget_is_at_least_one() {
        assert_eq!(
            phase_token_budget_refusal(0).as_deref(),
            Some("`token_budget` = 0 is outside 1..=2147483647 tokens"),
            "set_setting's Phase rung words it so"
        );
        assert_eq!(phase_token_budget_refusal(1), None);
        assert_eq!(phase_token_budget_refusal(i32::MAX), None);
        let mut row = phase(Split::None, 1);
        row.token_budget = Some(-1);
        assert_eq!(phase_row_refusal(&row), phase_token_budget_refusal(-1));
        row.retry_limit = -1;
        assert_eq!(
            phase_row_refusal(&row),
            Some(retry_limit_negative("implement", -1)),
            "after 0001's CHECKs"
        );
        row.retry_limit = 0;
        row.token_budget = Some(1);
        assert_eq!(phase_row_refusal(&row), None);
        row.token_budget = None;
        assert_eq!(phase_row_refusal(&row), None);
    }

    // ---- graph bodies (§T3.2) ----

    fn node(graph: StepGraphId, position: i32, name: &str) -> NodeBody {
        let mut row = phase(Split::None, 1);
        row.graph_id = graph;
        row.position = position;
        row.name = name.to_owned();
        NodeBody {
            phase: row,
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    fn agent(phase: PhaseId, position: i32) -> PhaseAgent {
        PhaseAgent {
            phase_id: phase,
            position,
            agent_id: AgentId::new(),
            model: "m".to_owned(),
        }
    }

    fn part(phase: PhaseId, position: i32, key: &str) -> PhasePart {
        PhasePart {
            phase_id: phase,
            position,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: Vec::new(),
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    fn edge(graph: StepGraphId, from: PhaseId, to: PhaseId, kind: EdgeKind) -> StepGraphEdge {
        StepGraphEdge {
            id: EdgeId::new(),
            graph_id: graph,
            from,
            to,
            kind,
            predicate: None,
            loop_limit: kind.needs_loop_limit().then_some(2),
        }
    }

    /// A valid three-node body: `plan@0 -> implement@1 -> review@2`, a review loop, one
    /// candidate on `implement`.
    fn body() -> (StepGraphId, GraphBody) {
        let graph = StepGraphId::new();
        let now = Utc::now();
        let nodes = vec![
            node(graph, 0, "plan"),
            node(graph, 1, "implement"),
            node(graph, 2, "review"),
        ];
        let (plan, implement, review) = (nodes[0].phase.id, nodes[1].phase.id, nodes[2].phase.id);
        let mut body = GraphBody {
            graph: StepGraph {
                id: graph,
                scope: GraphScope::Project(ProjectId::new()),
                name: "feature".to_owned(),
                description: String::new(),
                is_override: false,
                revision: 0,
                created_at: now,
                updated_at: now,
            },
            nodes,
            edges: vec![
                edge(graph, plan, implement, EdgeKind::Next),
                edge(graph, implement, review, EdgeKind::Next),
                edge(graph, review, implement, EdgeKind::ChangesRequested),
            ],
        };
        body.nodes[1].agents.push(agent(implement, 0));
        (graph, body)
    }

    #[test]
    fn a_valid_body_passes_both_rule_sets() {
        let (graph, body) = body();
        assert_eq!(graph_body_shape_refusal(graph, &body), None);
        assert_eq!(graph_body_topology_refusal(graph, &body), None);
    }

    /// Rule a: the body is the graph being written.
    #[test]
    fn shape_a_the_body_names_the_graph() {
        let (graph, mut body) = body();
        let other = StepGraphId::new();
        body.graph.id = other;
        assert_eq!(
            graph_body_shape_refusal(graph, &body),
            Some(row_names_another_graph("step_graph.id", other, graph))
        );
    }

    /// Rule b: every row names its graph or its node.
    #[test]
    fn shape_b_rows_name_their_graph_and_node() {
        let (graph, body) = body();
        let other = StepGraphId::new();
        let stray = PhaseId::new();

        let mut nodes = body.clone();
        nodes.nodes[2].phase.graph_id = other;
        assert_eq!(
            graph_body_shape_refusal(graph, &nodes),
            Some(row_names_another_graph(
                "step_graph_phase.graph_id",
                other,
                graph
            ))
        );

        let mut agents = body.clone();
        agents.nodes[0].agents.push(agent(stray, 0));
        assert_eq!(
            graph_body_shape_refusal(graph, &agents),
            Some(row_names_another_phase(
                "phase_agent",
                stray,
                agents.nodes[0].phase.id
            ))
        );

        let mut parts = body.clone();
        parts.nodes[1].parts.push(part(stray, 0, "a"));
        assert_eq!(
            graph_body_shape_refusal(graph, &parts),
            Some(row_names_another_phase(
                "phase_part",
                stray,
                parts.nodes[1].phase.id
            ))
        );

        let mut edges = body;
        edges.edges[1].graph_id = other;
        assert_eq!(
            graph_body_shape_refusal(graph, &edges),
            Some(row_names_another_graph(
                "step_graph_edge.graph_id",
                other,
                graph
            ))
        );
    }

    /// Rule c: `judge` and `handoff` are template roles, not phase names.
    #[test]
    fn shape_c_reserved_names() {
        let (graph, mut body) = body();
        body.nodes[1].phase.name = "judge".to_owned();
        assert_eq!(
            graph_body_shape_refusal(graph, &body),
            Some(reserved_phase_name("judge"))
        );
    }

    /// Rule d: the second node holding an id, a position or a name is refused.
    #[test]
    fn shape_d_node_ids_positions_and_names_are_unique() {
        let (graph, body) = body();

        let mut ids = body.clone();
        ids.nodes[2].phase.id = ids.nodes[0].phase.id;
        assert_eq!(
            graph_body_shape_refusal(graph, &ids),
            Some(already_exists("step_graph_phase", ids.nodes[0].phase.id))
        );

        let mut positions = body.clone();
        positions.nodes[2].phase.position = 0;
        assert_eq!(
            graph_body_shape_refusal(graph, &positions),
            Some(phase_position_is_taken(0, graph))
        );

        let mut names = body;
        names.nodes[2].phase.name = "plan".to_owned();
        assert_eq!(
            graph_body_shape_refusal(graph, &names),
            Some(phase_name_is_taken("plan", graph))
        );
    }

    /// Rule e: candidate and part positions, and part keys, are unique within the node.
    #[test]
    fn shape_e_candidate_and_part_slots_are_unique() {
        let (graph, body) = body();
        let implement = body.nodes[1].phase.id;

        let mut agents = body.clone();
        agents.nodes[1].agents.push(agent(implement, 0));
        assert_eq!(
            graph_body_shape_refusal(graph, &agents),
            Some(already_exists("phase_agent", format!("({implement}, 0)")))
        );

        let mut parts = body.clone();
        parts.nodes[1].parts = vec![part(implement, 3, "a"), part(implement, 3, "b")];
        assert_eq!(
            graph_body_shape_refusal(graph, &parts),
            Some(already_exists("phase_part", format!("({implement}, 3)")))
        );

        let mut keys = body;
        keys.nodes[1].parts = vec![part(implement, 0, "a"), part(implement, 1, "a")];
        assert_eq!(
            graph_body_shape_refusal(graph, &keys),
            Some(phase_part_key_is_taken("a", implement))
        );
    }

    /// Rule f: edge ids are unique within the body.
    #[test]
    fn shape_f_edge_ids_are_unique() {
        let (graph, mut body) = body();
        body.edges[2].id = body.edges[0].id;
        assert_eq!(
            graph_body_shape_refusal(graph, &body),
            Some(already_exists("step_graph_edge", body.edges[0].id))
        );
    }

    /// One body breaking every shape rule, mended one rule at a time.
    #[test]
    fn the_shape_rules_fire_in_order() {
        let (graph, mut body) = body();
        let other = StepGraphId::new();
        let real = body.graph.id;
        let implement = body.nodes[1].phase.id;
        let stray = PhaseId::new();
        body.graph.id = other;
        body.edges[0].graph_id = other;
        body.nodes[0].phase.name = "handoff".to_owned();
        body.nodes[2].phase.position = 1;
        body.nodes[1].agents.push(agent(implement, 0));
        body.edges[2].id = body.edges[1].id;
        body.nodes[1].parts.push(part(stray, 0, "a"));
        let edge = body.edges[1].id;

        let mut expect = |mend: &mut dyn FnMut(&mut GraphBody), refusal: Option<String>| {
            assert_eq!(graph_body_shape_refusal(graph, &body), refusal);
            mend(&mut body);
        };
        expect(
            &mut |b| b.graph.id = real,
            Some(row_names_another_graph("step_graph.id", other, graph)),
        );
        expect(
            &mut |b| b.nodes[1].parts.clear(),
            Some(row_names_another_phase("phase_part", stray, implement)),
        );
        expect(
            &mut |b| b.edges[0].graph_id = graph,
            Some(row_names_another_graph(
                "step_graph_edge.graph_id",
                other,
                graph,
            )),
        );
        expect(
            &mut |b| b.nodes[0].phase.name = "plan".to_owned(),
            Some(reserved_phase_name("handoff")),
        );
        expect(
            &mut |b| b.nodes[2].phase.position = 2,
            Some(phase_position_is_taken(1, graph)),
        );
        expect(
            &mut |b| {
                b.nodes[1].agents.pop();
            },
            Some(already_exists("phase_agent", format!("({implement}, 0)"))),
        );
        expect(
            &mut |b| b.edges[2].id = EdgeId::new(),
            Some(already_exists("step_graph_edge", edge)),
        );
        assert_eq!(graph_body_shape_refusal(graph, &body), None);
    }

    /// Rule a: both endpoints are nodes of the body, `from` first.
    #[test]
    fn topology_a_endpoints_are_nodes() {
        let (graph, body) = body();
        let stray = PhaseId::new();

        let mut from = body.clone();
        from.edges[1].from = stray;
        assert_eq!(
            graph_body_topology_refusal(graph, &from),
            Some(edge_endpoint_outside_graph("from_phase_id", stray, graph))
        );

        let mut to = body;
        to.edges[1].to = stray;
        assert_eq!(
            graph_body_topology_refusal(graph, &to),
            Some(edge_endpoint_outside_graph("to_phase_id", stray, graph))
        );
    }

    /// Rule b: the edge CHECKs, in their order.
    #[test]
    fn topology_b_edge_checks() {
        let (graph, body) = body();
        let review = body.nodes[2].phase.id;

        let mut own = body.clone();
        own.edges[0].to = own.edges[0].from;
        assert_eq!(
            graph_body_topology_refusal(graph, &own),
            Some(edge_loops_to_itself(own.edges[0].id))
        );

        let mut no_predicate = body.clone();
        no_predicate.edges[0].kind = EdgeKind::If;
        assert_eq!(
            graph_body_topology_refusal(graph, &no_predicate),
            Some(edge_predicate_mismatch(
                no_predicate.edges[0].id,
                EdgeKind::If
            ))
        );

        let mut stray_predicate = body.clone();
        stray_predicate.edges[0].predicate = Some(EdgePredicate(serde_json::json!({"x": 1})));
        assert_eq!(
            graph_body_topology_refusal(graph, &stray_predicate),
            Some(edge_predicate_mismatch(
                stray_predicate.edges[0].id,
                EdgeKind::Next
            ))
        );

        // Review R1 B-L1: a JSON `null` predicate is none, on an `if` edge and on any other.
        let mut null_if = body.clone();
        null_if.edges[0].kind = EdgeKind::If;
        null_if.edges[0].predicate = Some(EdgePredicate(serde_json::Value::Null));
        assert_eq!(edge_predicate(&null_if.edges[0]), None);
        assert_eq!(
            graph_body_topology_refusal(graph, &null_if),
            Some(edge_predicate_mismatch(null_if.edges[0].id, EdgeKind::If))
        );
        let mut null_next = body.clone();
        null_next.edges[0].predicate = Some(EdgePredicate(serde_json::Value::Null));
        assert_eq!(graph_body_topology_refusal(graph, &null_next), None);

        let mut negative = body.clone();
        negative.edges[2].loop_limit = Some(-1);
        assert_eq!(
            graph_body_topology_refusal(graph, &negative),
            Some(edge_loop_limit_negative(negative.edges[2].id, -1))
        );

        let mut missing = body.clone();
        missing.edges[2].loop_limit = None;
        assert_eq!(
            graph_body_topology_refusal(graph, &missing),
            Some(edge_needs_loop_limit(
                missing.edges[2].id,
                EdgeKind::ChangesRequested
            ))
        );

        let mut asks = body;
        asks.edges.push(edge(
            graph,
            review,
            asks.nodes[0].phase.id,
            EdgeKind::AsksYou,
        ));
        assert_eq!(
            graph_body_topology_refusal(graph, &asks),
            None,
            "asks_you takes no limit here"
        );
    }

    /// Rule c: `(from, to, kind)` is unique.
    #[test]
    fn topology_c_edges_are_unique() {
        let (graph, mut body) = body();
        let twin = edge(graph, body.edges[0].from, body.edges[0].to, EdgeKind::Next);
        body.edges.push(twin.clone());
        assert_eq!(
            graph_body_topology_refusal(graph, &body),
            Some(edge_is_taken(twin.from, twin.to, EdgeKind::Next, graph))
        );
        body.edges.last_mut().expect("pushed").kind = EdgeKind::AsksYou;
        assert_eq!(graph_body_topology_refusal(graph, &body), None);
    }

    /// Rule d: each node passes `phase_row_refusal`.
    #[test]
    fn topology_d_node_checks() {
        let (graph, mut body) = body();
        body.nodes[1].phase.fan_out = 2;
        assert_eq!(
            graph_body_topology_refusal(graph, &body),
            Some(split_fan_out_mismatch("implement", Split::None, 2))
        );
    }

    /// Rule e: a palette node has no edges, in or out.
    #[test]
    fn topology_e_palette_nodes_have_no_edges() {
        let (graph, mut body) = body();
        body.nodes[0].phase.palette = true;
        assert_eq!(
            graph_body_topology_refusal(graph, &body),
            Some(palette_phase_has_edges("plan", body.edges[0].id))
        );
        body.edges.remove(0);
        assert_eq!(graph_body_topology_refusal(graph, &body), None);
        body.nodes[2].phase.palette = true;
        assert_eq!(
            graph_body_topology_refusal(graph, &body),
            Some(palette_phase_has_edges("review", body.edges[0].id)),
            "an incoming edge counts"
        );
    }

    /// Rule f: only a `divide` node has parts.
    #[test]
    fn topology_f_parts_need_a_divide_node() {
        let (graph, mut body) = body();
        let implement = body.nodes[1].phase.id;
        body.nodes[1].parts.push(part(implement, 0, "a"));
        assert_eq!(
            graph_body_topology_refusal(graph, &body),
            Some(parts_off_divide("implement", Split::None))
        );
        body.nodes[1].phase.split = Split::Divide;
        assert_eq!(graph_body_topology_refusal(graph, &body), None);
    }

    /// One body breaking every topology rule, mended one rule at a time.
    #[test]
    fn the_topology_rules_fire_in_order() {
        let (graph, mut body) = body();
        let (implement, review) = (body.nodes[1].phase.id, body.nodes[2].phase.id);
        let stray = PhaseId::new();
        body.edges[0].to = stray;
        body.edges[1].kind = EdgeKind::If;
        let twin = edge(graph, review, implement, EdgeKind::ChangesRequested);
        body.edges.push(twin.clone());
        body.nodes[2].phase.max_parts = Some(2);
        body.nodes[0].phase.palette = true;
        body.nodes[1].parts.push(part(implement, 0, "a"));
        let (first, second) = (body.edges[0].id, body.edges[1].id);

        let mut expect = |mend: &mut dyn FnMut(&mut GraphBody), refusal: Option<String>| {
            assert_eq!(graph_body_topology_refusal(graph, &body), refusal);
            mend(&mut body);
        };
        expect(
            &mut |b| b.edges[0].to = implement,
            Some(edge_endpoint_outside_graph("to_phase_id", stray, graph)),
        );
        expect(
            &mut |b| b.edges[1].kind = EdgeKind::Next,
            Some(edge_predicate_mismatch(second, EdgeKind::If)),
        );
        expect(
            &mut |b| {
                b.edges.pop();
            },
            Some(edge_is_taken(
                review,
                implement,
                EdgeKind::ChangesRequested,
                graph,
            )),
        );
        expect(
            &mut |b| b.nodes[2].phase.max_parts = None,
            Some(divide_columns_off_divide("review", Split::None)),
        );
        expect(
            &mut |b| b.nodes[0].phase.palette = false,
            Some(palette_phase_has_edges("plan", first)),
        );
        expect(
            &mut |b| b.nodes[1].parts.clear(),
            Some(parts_off_divide("implement", Split::None)),
        );
        assert_eq!(graph_body_topology_refusal(graph, &body), None);
    }

    /// Personas first, per node: the three phase columns, then its parts.
    #[test]
    fn body_refs_walk_personas_then_agents_in_body_order() {
        let (_, mut body) = body();
        let ids: Vec<PersonaId> = (0..5).map(|_| PersonaId::new()).collect();
        let agents: Vec<AgentId> = (0..4).map(|_| AgentId::new()).collect();
        let plan = body.nodes[0].phase.id;
        body.nodes[0].phase.split = Split::Divide;
        body.nodes[0].phase.persona_id = Some(ids[0]);
        body.nodes[0].phase.planner_persona_id = Some(ids[1]);
        body.nodes[0].phase.integrate_persona_id = Some(ids[2]);
        body.nodes[0].phase.planner_agent_id = Some(agents[0]);
        body.nodes[0].phase.integrate_agent_id = Some(agents[1]);
        let mut p = part(plan, 0, "a");
        p.persona_id = Some(ids[3]);
        p.agent_id = Some(agents[3]);
        body.nodes[0].parts.push(p);
        body.nodes[0].agents.push(PhaseAgent {
            phase_id: plan,
            position: 0,
            agent_id: agents[2],
            model: "m".to_owned(),
        });
        body.nodes[2].phase.persona_id = Some(ids[4]);
        let implement_agent = body.nodes[1].agents[0].agent_id;

        assert_eq!(
            body_persona_refs(&body),
            vec![
                ("step_graph_phase.persona_id", ids[0]),
                ("step_graph_phase.planner_persona_id", ids[1]),
                ("step_graph_phase.integrate_persona_id", ids[2]),
                ("phase_part.persona_id", ids[3]),
                ("step_graph_phase.persona_id", ids[4]),
            ]
        );
        assert_eq!(
            body_agent_refs(&body),
            vec![
                ("step_graph_phase.planner_agent_id", agents[0]),
                ("step_graph_phase.integrate_agent_id", agents[1]),
                ("phase_agent.agent_id", agents[2]),
                ("phase_part.agent_id", agents[3]),
                ("phase_agent.agent_id", implement_agent),
            ]
        );
    }
}
