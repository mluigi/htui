use htui_core::fixtures::ids;
use htui_core::model::{
    GraphSnapshot, ItemPatch, NewStepGraph, NodeRole, PhaseId, RunMode, StepGraphId, StepGraphPhase,
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
        ids::BOX,
    )
    .await
    .unwrap();
    let expected_str = std::fs::read_to_string("tests/fixtures/feature.v2.snapshot.json").unwrap();
    let expected: GraphSnapshot = serde_json::from_str(&expected_str).unwrap();
    // The whole snapshot, which is what blueprint §6.1 asks for: the recorded row equals a fresh
    // `graph::resolve` of the same graph. The digest alone would pass on a snapshot whose phases
    // were right and whose `graph`, `mode`, `v` or `settings` were not — `topology` is hashed over
    // `phases[]` and `edges[]` and nothing else (plan D9, MOD-138 D3) — and it would say "the
    // digests differ" about any of the seventeen `SnapshotPhase` fields rather than naming the one
    // that moved.
    //
    // MOD-138 F15: `feature.v2.snapshot.json` was recorded once from this resolve (`"v": 2`, the
    // default edges, the source, the graph's scope and revision); `feature.snapshot.json` stays
    // the v1 decode fixture below.
    assert_eq!(resolved.snapshot, expected);
    assert_eq!(resolved.snapshot.v, GraphSnapshot::V);

    // D1's byte-identity, end to end: the v2 resolve's nodes are the v1 fixture's phases, field
    // for field, and the v1 lift of that fixture freezes the same edges.
    let v1: GraphSnapshot = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/feature.snapshot.json").unwrap(),
    )
    .unwrap();
    assert_eq!(resolved.snapshot.phases, v1.phases);
    let mut lifted = v1;
    lifted.lift_v1();
    assert_eq!(resolved.snapshot.edges, lifted.edges);
}

/// MOD-138 D1's byte-identity proof on a recorded v1 blob (F15): `feature.snapshot.json`, written
/// before MOD-138, decodes with every v2 node field at its default, and its phases re-serialise
/// to exactly the digest it stored, so no v1 digest moved and no in-flight v1 run parks.
#[test]
fn the_v1_fixture_decodes_and_its_phases_hash_to_its_own_digest() {
    let text = std::fs::read_to_string("tests/fixtures/feature.snapshot.json").unwrap();
    let decoded: GraphSnapshot = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded.v, GraphSnapshot::V1);
    assert!(decoded.edges.is_empty() && decoded.source.is_none());
    assert_eq!(decoded.phases.len(), 4);
    for phase in &decoded.phases {
        assert_eq!(phase.role, NodeRole::Step, "{}", phase.name);
        assert!(!phase.palette, "{}", phase.name);
        assert!(
            phase.divide.is_none() && phase.plan_scope.is_none(),
            "{}",
            phase.name
        );
    }
    assert_eq!(
        htui_orch::graph::topology(&decoded.phases).unwrap(),
        decoded.topology,
        "the v1 digest of a recorded v1 blob is unchanged by the v2 fields"
    );
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
            scope: htui_core::model::GraphScope::Project(item.project_id),
            name: "feature-with-verify".to_owned(),
            description: "intermediate verify phase".to_owned(),
            is_override: false,
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
        ids::BOX,
    )
    .await
    .unwrap();

    let expected_str =
        std::fs::read_to_string("tests/fixtures/feature-with-verify.snapshot.json").unwrap();
    let expected: GraphSnapshot = serde_json::from_str(&expected_str).unwrap();

    // MOD-138 F15: the fixture is a v1 blob, so its digest is the v1 one over the phases; the
    // resolve carries the v2 digest in `topology` since `GraphSnapshot::V = 2`.
    assert_eq!(
        htui_orch::graph::topology(&resolved_edited.snapshot.phases).unwrap(),
        expected.topology
    );
    assert_eq!(
        resolved_edited.snapshot.topology,
        htui_orch::graph::topology_v2(
            &resolved_edited.snapshot.phases,
            &resolved_edited.snapshot.edges
        )
        .unwrap()
    );
}
