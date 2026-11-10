//! MOD-141: the one step-graph editor (PRD `mod-141-step-graph-editor`, M1 plan D1-D14).
//!
//! It edits one [`GraphBody`] at a time, of any scope (project, workspace, library): nodes and
//! typed edges drawn with rataflow, a node form, an edge form, a links list, the store's own graph
//! rules run before every write (D5), and in-memory undo and redo where every applied edit is one
//! `StoreRequest::ReplaceGraph` compare-and-set on `step_graph.revision` (D4, D6). Nothing in here
//! names `Settings > Kinds`: the host hands it a body, the persona registry and every reply.
//!
//! - `model`: pure body edits and the client-side validator (D4, D5, D10).
//! - `history`: undo, redo and the one write in flight (D4, D6).
//! - `canvas`: the rataflow projection, layout, selection and mouse (D7, D11).
//! - `node_form`, `edge_form`, `links`: the forms and the links list (D8, D9, D10).
// Wave 1 (T1-T5) lands the submodules before anything calls them; T6 removes this allow.
#![allow(dead_code)]

pub(crate) mod canvas;
pub(crate) mod edge_form;
pub(crate) mod history;
pub(crate) mod links;
pub(crate) mod model;
pub(crate) mod node_form;

use std::collections::HashMap;

use htui_core::model::{AgentSummary, EdgeKind, EdgePredicate, GraphBody, PhaseId, PromptTemplate};
use htui_core::store::{WalkEdge, WalkNode, forward_mask};

use crate::catalogue::PersonaSummary;

/// What the host hands the editor beside the body (D1): the persona registry, always known (the
/// catalogue carries it), and the agents and the templates once their reads answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Lookups {
    /// The persona registry by name: the node form's persona choice and the validator's persona
    /// references (D5, D8).
    pub(crate) personas: Vec<PersonaSummary>,
    /// `StoreReply::Agents`, by `agent.name`; `None` until it arrived. While `None` the validator
    /// leaves the agent references to the store, and the candidate list cannot be edited.
    pub(crate) agents: Option<Vec<AgentSummary>>,
    /// The templates the graph's scope may use, every version; `None` until
    /// `StoreReply::Templates` arrived.
    pub(crate) templates: Option<Vec<PromptTemplate>>,
}

/// One link's editable part (D9): what the edge form yields and what `model` writes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdgeSpec {
    /// `step_graph_edge.kind`.
    pub(crate) kind: EdgeKind,
    /// `step_graph_edge.predicate`: `Some` exactly when `kind` is `if`.
    pub(crate) predicate: Option<EdgePredicate>,
    /// `step_graph_edge.loop_limit`: required on `changes_requested` and `fails`, optional on
    /// `asks_you`, `None` otherwise.
    pub(crate) loop_limit: Option<i32>,
}

/// The relations in picker order (D9).
pub(crate) const RELATIONS: [EdgeKind; 5] = [
    EdgeKind::Next,
    EdgeKind::If,
    EdgeKind::AsksYou,
    EdgeKind::ChangesRequested,
    EdgeKind::Fails,
];

/// A relation in words, never its wire name (MOD-83).
pub(crate) const fn relation_word(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Next => "next",
        EdgeKind::If => "if",
        EdgeKind::AsksYou => "asks you",
        EdgeKind::ChangesRequested => "changes requested",
        EdgeKind::Fails => "fails",
    }
}

/// Per edge of `body`, in `body.edges` order, whether it is forward: `forward_mask` over the body
/// exactly as `graph_body_walk_refusal` builds its walk, so the editor and the store cannot
/// disagree (D7). An edge naming a step the body lacks is `false`.
pub(crate) fn forward_edges(body: &GraphBody) -> Vec<bool> {
    let index: HashMap<PhaseId, usize> = body
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.phase.id, at))
        .collect();
    let nodes: Vec<WalkNode<'_>> = body
        .nodes
        .iter()
        .map(|node| WalkNode {
            name: &node.phase.name,
            palette: node.phase.palette,
        })
        .collect();
    // Out of range rather than skipped, so the mask stays aligned with `body.edges`.
    let at = |id: PhaseId| index.get(&id).copied().unwrap_or(usize::MAX);
    let edges: Vec<WalkEdge> = body
        .edges
        .iter()
        .map(|edge| WalkEdge {
            from: at(edge.from),
            to: at(edge.to),
            kind: edge.kind,
        })
        .collect();
    forward_mask(&nodes, &edges)
}

/// The name of step `id` of `body`, or `None` when the body has no such step.
pub(crate) fn step_name(body: &GraphBody, id: PhaseId) -> Option<&str> {
    body.nodes
        .iter()
        .find(|node| node.phase.id == id)
        .map(|node| node.phase.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mask is the store's: a `next` is forward, a `changes_requested` never is (seeded bug
    /// graph: reproduce -> fix -> review, review -changes_requested-> fix).
    #[tokio::test]
    async fn forward_edges_is_the_stores_mask() {
        use htui_core::fixtures::ids;
        use htui_core::store::{MemStore, WriteStore};
        let body = MemStore::demo()
            .graph_body(ids::GRAPH_VULKAN_FIX)
            .await
            .expect("a memory read")
            .expect("the bug graph");
        let mask = forward_edges(&body);
        for (edge, forward) in body.edges.iter().zip(&mask) {
            assert_eq!(*forward, edge.kind == EdgeKind::Next, "{:?}", edge.kind);
        }
    }
}
