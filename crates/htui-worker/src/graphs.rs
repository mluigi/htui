//! The graph source over any host (MOD-4 plan D155, MOD-41 plan D6).

use htui_core::model::{
    Agent, AgentBox, AgentId, BoundSkill, BoxId, ItemId, PhaseAgent, PhaseId, ProjectId,
    PromptTemplate, ResolvedGraph,
};
use htui_core::store::Result as StoreResult;
use htui_orch::GraphSource;

/// Plan D155: `GraphSource` is `htui-orch`'s and `Backend` is `htui-store`'s, so `impl GraphSource
/// for Backend` here is E0117 (proven: plan Verified claims). A local newtype is the answer, and it
/// keeps invariant 10 (the orchestrator never names `htui-store`).
///
/// MOD-41 plan D7: over any [`WorkerHost`](htui_core::store::WorkerHost) (it was `BackendGraphs`).
/// Every read delegates to the host's read of the same name; `agent` filters the host's `agents`
/// exactly as the `MemStore` implementation in `htui-orch`'s fake does.
#[derive(Debug, Clone)]
pub struct HostGraphs<H>(pub H);

impl<H: htui_core::store::WorkerHost> GraphSource for HostGraphs<H> {
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

    async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> StoreResult<Vec<BoundSkill>> {
        self.0.bound_skills(project, phase).await
    }

    async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> StoreResult<Vec<String>> {
        self.0.missing_tags(item, box_id).await
    }
}
