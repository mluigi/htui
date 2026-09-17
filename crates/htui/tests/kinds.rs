//! `Settings > Kinds`, from the worker side out (MOD-15 milestone 4, D18).
//!
//! The worker half drives [`htui::store_worker::serve`] directly over a `Backend`, exactly as
//! `tests/hierarchy.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 4's task 2 and lands below this one.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::catalogue::REQUEST_NAMES;
use htui::store_worker::StoreRequest;
use htui_core::model::{
    ItemKindId, ItemKindPatch, PhaseId, PhasePatch, ProjectId, Scope, StepGraphId, StepGraphPatch,
    WorkspaceId,
};

/// The empty scope every name-only request is built against: the nine names are a property of the
/// enum, not of a store.
fn nil_scope() -> Scope {
    Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: Vec::new(),
    }
}

/// One of each of the nine, in [`REQUEST_NAMES`] order. The ids are nil where the request never
/// reaches a store.
fn catalogue_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::Catalogue(nil_scope()),
        StoreRequest::CreateKind {
            scope: nil_scope(),
            project: ProjectId::default(),
            prefix: String::new(),
            name: String::new(),
            description: String::new(),
            graph: StepGraphId::default(),
            position: 0,
        },
        StoreRequest::UpdateKind {
            scope: nil_scope(),
            id: ItemKindId::default(),
            expected: Utc::now(),
            patch: ItemKindPatch::default(),
        },
        StoreRequest::DeleteKind {
            scope: nil_scope(),
            id: ItemKindId::default(),
        },
        StoreRequest::CreateGraph {
            scope: nil_scope(),
            project: ProjectId::default(),
            name: String::new(),
            description: String::new(),
        },
        StoreRequest::UpdateGraph {
            scope: nil_scope(),
            id: StepGraphId::default(),
            expected: Utc::now(),
            patch: StepGraphPatch::default(),
        },
        StoreRequest::CreatePhase {
            scope: nil_scope(),
            graph: StepGraphId::default(),
            name: String::new(),
            position: 0,
            template_name: String::new(),
            gate_hard: false,
            input_kinds: Vec::new(),
        },
        StoreRequest::UpdatePhase {
            scope: nil_scope(),
            id: PhaseId::default(),
            expected: Utc::now(),
            patch: PhasePatch::default(),
        },
        StoreRequest::SetPhaseBudget {
            scope: nil_scope(),
            phase: PhaseId::default(),
            expected: Utc::now(),
            budget: None,
        },
    ]
}

/// `REQUEST_NAMES` and [`StoreRequest::name`] are one list read from two places (D5): a tenth
/// request cannot be named in one and matched in the other.
#[test]
fn catalogue_names_are_stable() {
    let requests = catalogue_requests();
    assert_eq!(requests.len(), REQUEST_NAMES.len());
    assert_eq!(REQUEST_NAMES.len(), 9);
    for (request, name) in requests.iter().zip(REQUEST_NAMES) {
        assert_eq!(request.name(), name);
    }
}
