//! The orchestrator, driven from the store worker loop (MOD-4 milestone 6, plan D153).
//!
//! `htui-orch` never names `htui-store` (ANA-2 invariant 10), so everything that joins the two
//! lives here: the graph source over a [`Backend`], and — as the milestone lands — the runtime
//! that serves the orchestrator's requests on its own tasks beside `AgentRuntime`.

use htui_core::model::{
    Agent, AgentBox, AgentId, BoxId, ItemId, PhaseAgent, PhaseId, ProjectId, PromptTemplate,
    ResolvedGraph,
};
use htui_core::store::Result as StoreResult;
use htui_orch::GraphSource;
use htui_store::Backend;

/// Plan D155: `GraphSource` is `htui-orch`'s and `Backend` is `htui-store`'s, so `impl GraphSource
/// for Backend` here is E0117 (proven: plan Verified claims). A local newtype is the answer, and it
/// keeps invariant 10 (the orchestrator never names `htui-store`).
///
/// Every read delegates to the `Backend`-inherent read of the same name; `agent` filters
/// [`Backend::agents`] exactly as the `MemStore` implementation in `htui-orch`'s fake does.
#[derive(Debug, Clone)]
pub struct BackendGraphs(pub Backend);

impl GraphSource for BackendGraphs {
    async fn resolve_graph(&self, item: ItemId) -> StoreResult<Option<ResolvedGraph>> {
        self.0.resolve_graph(item).await
    }

    async fn phase_agents(&self, phase: PhaseId) -> StoreResult<Vec<PhaseAgent>> {
        self.0.phase_agents(phase).await
    }

    async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> StoreResult<Option<PromptTemplate>> {
        self.0.prompt_template(project, name, version).await
    }

    async fn agent(&self, id: AgentId) -> StoreResult<Option<Agent>> {
        Ok(self
            .0
            .agents()
            .await?
            .into_iter()
            .map(|summary| summary.agent)
            .find(|agent| agent.id == id))
    }

    async fn agent_boxes(&self, box_id: BoxId) -> StoreResult<Vec<AgentBox>> {
        self.0.agent_boxes(box_id).await
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{Agent, AgentBox, AgentId, Billing, Transport};
    use htui_core::store::{MemStore, WriteStore as _};
    use htui_orch::GraphSource;
    use htui_store::Backend;
    use serde_json::json;

    use super::BackendGraphs;

    /// The scripted registry row every walk test runs on (blueprint F-O): an `acp` row, because
    /// the fixture graphs gate every phase and stage 1's inline-approval interlock skips a `cli`
    /// row at a gated phase; the factory reaches the fake by row data alone.
    fn scripted_row(id: AgentId) -> Agent {
        Agent {
            id,
            name: "scripted".to_owned(),
            transport: Transport::Acp,
            billing: Billing::Subscription,
            models: Vec::new(),
            default_model: Some("sonnet".to_owned()),
            launch: json!({ "command": "unused", "args": [] }),
            settings: json!({}),
            enabled: true,
            created_at: demo_at(0, 0),
            updated_at: demo_at(0, 0),
        }
    }

    /// The scripted agent's `agent_box` on the demo box, probed ready (rung 3, plan D62).
    fn ready_on_box(agent_id: AgentId, at: DateTime<Utc>) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: ids::BOX,
            enabled: true,
            version: Some("0.0.0-fake".to_owned()),
            path: None,
            probed_at: Some(at),
            quota: None,
            quota_at: None,
            updated_at: at,
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        }
    }

    /// Blueprint F-O: the demo with every fixture agent disabled, one scripted agent and its
    /// `agent_box` on the demo box, so rung 3 of the candidate chain names exactly it.
    async fn seeded_store() -> (MemStore, AgentId) {
        let store = MemStore::demo();
        for summary in store.agents().await.expect("the fixture's agents") {
            let mut row = summary.agent;
            row.enabled = false;
            store.upsert_agent(&row).await.expect("the row is disabled");
        }
        let agent = AgentId::new();
        store
            .upsert_agent(&scripted_row(agent))
            .await
            .expect("the scripted row lands");
        store
            .upsert_agent_box(&ready_on_box(agent, demo_at(0, 0)))
            .await
            .expect("the agent_box row lands");
        (store, agent)
    }

    /// Plan D155: the five trait reads are the inherent reads of the same name.
    #[tokio::test]
    async fn backend_graphs_delegates_each_read() {
        let (store, agent) = seeded_store().await;
        let backend = Backend::memory(store);
        let graphs = BackendGraphs(backend.clone());

        let resolved = GraphSource::resolve_graph(&graphs, ids::HTUI_ANA_2)
            .await
            .expect("the read answers");
        assert_eq!(
            resolved,
            backend
                .resolve_graph(ids::HTUI_ANA_2)
                .await
                .expect("the read answers")
        );
        let resolved = resolved.expect("the demo item has a graph");
        let phase = resolved.phases.first().expect("the graph has a phase");

        assert_eq!(
            GraphSource::phase_agents(&graphs, phase.phase.id)
                .await
                .expect("the read answers"),
            backend
                .phase_agents(phase.phase.id)
                .await
                .expect("the read answers")
        );
        assert_eq!(
            GraphSource::prompt_template(&graphs, ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers"),
            backend
                .prompt_template(ids::PROJECT_HTUI, &phase.phase.name, None)
                .await
                .expect("the read answers")
        );
        let row = GraphSource::agent(&graphs, agent)
            .await
            .expect("the read answers")
            .expect("the scripted row is registered");
        assert_eq!(row.name, "scripted");
        assert_eq!(
            GraphSource::agent(&graphs, AgentId::new())
                .await
                .expect("the read answers"),
            None,
            "an unknown id is no row"
        );
        let boxes = GraphSource::agent_boxes(&graphs, ids::BOX)
            .await
            .expect("the read answers");
        assert_eq!(
            boxes,
            backend
                .agent_boxes(ids::BOX)
                .await
                .expect("the read answers")
        );
        assert!(boxes.iter().any(|row| row.agent_id == agent));
    }
}
