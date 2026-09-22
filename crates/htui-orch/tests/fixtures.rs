use htui_core::fixtures::ids;
use htui_core::model::{
    GraphSnapshot, ItemPatch, NewStepGraph, PhaseId, RunMode, StepGraphId, StepGraphPhase,
};
use htui_core::store::{ReadStore, WriteStore};
use htui_orch::fake::FakeOrchestrator;
use htui_orch::graph::resolve;
use std::collections::BTreeMap;
use uuid::Uuid;

#[tokio::test]
async fn feature_snapshot_matches() {
    let orch = FakeOrchestrator::demo();
    let item = orch.store.item(ids::HTUI_FEAT_3).await.unwrap().unwrap();
    let resolved = resolve(
        &orch.store,
        &orch.graphs(),
        &item,
        RunMode::Manual,
        &BTreeMap::new(),
        None,
    )
    .await
    .unwrap();
    let expected_str = std::fs::read_to_string("tests/fixtures/feature.snapshot.json").unwrap();
    let expected: GraphSnapshot = serde_json::from_str(&expected_str).unwrap();
    // The whole snapshot, which is what blueprint §6.1 asks for: the recorded row equals a fresh
    // `graph::resolve` of the same graph. The digest alone would pass on a snapshot whose phases
    // were right and whose `graph`, `mode`, `v` or `settings` were not — `topology` is hashed over
    // `phases[]` and nothing else (plan D9) — and it would say "the digests differ" about any of
    // the seventeen `SnapshotPhase` fields rather than naming the one that moved.
    assert_eq!(resolved.snapshot, expected);
}

#[tokio::test]
async fn feature_with_verify_snapshot_matches() {
    let edited_orch = FakeOrchestrator::demo();
    let item = edited_orch
        .store
        .item(ids::HTUI_FEAT_3)
        .await
        .unwrap()
        .unwrap();
    let graph = edited_orch
        .store
        .resolve_graph(ids::HTUI_FEAT_3)
        .await
        .unwrap()
        .unwrap();
    let clone_id = StepGraphId(Uuid::parse_str("01a0b8da-ffc9-72d3-b6a0-f9658ec5d903").unwrap());
    edited_orch
        .store
        .create_step_graph(NewStepGraph {
            id: clone_id,
            project_id: item.project_id,
            name: "feature-with-verify".to_owned(),
            description: "intermediate verify phase".to_owned(),
        })
        .await
        .unwrap();

    for phase in &graph.phases {
        let mut p = phase.phase.clone();
        if p.name == "review" {
            p.position += 1;
            p.input_kinds = vec!["plan".to_owned(), "verify".to_owned()];
        }
        let p = StepGraphPhase {
            id: PhaseId::new(),
            graph_id: clone_id,
            ..p
        };
        edited_orch.store.create_phase(&p).await.unwrap();
        if phase.phase.name == "implement" {
            let verify_phase = StepGraphPhase {
                id: PhaseId::new(),
                graph_id: clone_id,
                position: p.position + 1,
                name: "verify".to_owned(),
                template_name: "review".to_owned(),
                input_kinds: vec!["implement".to_owned()],
                ..p.clone()
            };
            edited_orch.store.create_phase(&verify_phase).await.unwrap();
        }
    }
    edited_orch
        .store
        .update_item(
            ids::HTUI_FEAT_3,
            item.version,
            ItemPatch {
                step_graph_id: Some(Some(clone_id)),
                author_id: item.created_by,
                ..ItemPatch::default()
            },
        )
        .await
        .unwrap();

    let item_edited = edited_orch
        .store
        .item(ids::HTUI_FEAT_3)
        .await
        .unwrap()
        .unwrap();
    let resolved_edited = resolve(
        &edited_orch.store,
        &edited_orch.graphs(),
        &item_edited,
        RunMode::Manual,
        &BTreeMap::new(),
        None,
    )
    .await
    .unwrap();

    let expected_str =
        std::fs::read_to_string("tests/fixtures/feature-with-verify.snapshot.json").unwrap();
    let expected: GraphSnapshot = serde_json::from_str(&expected_str).unwrap();

    assert_eq!(resolved_edited.snapshot.topology, expected.topology);
}
