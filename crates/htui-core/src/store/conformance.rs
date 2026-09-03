//! Store conformance suite (feature `test-support`), blueprint B.9.
//!
//! Every rule of `docs/ANA-9.md` §4.1, §4.2 and §7 a store has to show, written against
//! [`WriteStore`] alone: no concrete store is named anywhere in this module, so MOD-6 runs the
//! same cases against `PgStore` and reports per case. The cases assert against the demo fixture
//! (`test-support` implies `demo`), which is why the two features are not independent: MOD-6
//! inherits a seed data set instead of inventing a second one.

use core::future::Future;

use crate::fixtures::ids;
use crate::model::{
    EventKind, EventRole, ItemFilter, ItemId, ItemKindId, ItemPatch, ItemSummary, LinkKind,
    NewItem, ProjectId, Scope, Status, StepId,
};
use crate::store::error::StoreError;
use crate::store::traits::{UpdateOutcome, WriteStore};

/// Case names in run order. A name never changes: MOD-6 reports per case.
pub const CASES: &[&str] = &[
    "mint_consecutive_keys",
    "mint_prefix_isolation",
    "mint_writes_revision_v1",
    "mint_unknown_kind_rejected",
    "update_cas_success",
    "update_cas_diverged",
    "status_cas_keeps_version",
    "no_delete_path",
    "links_hops_1_vs_2",
    "filter_status_project_tags_ready",
    "documents_ordered_by_version",
    "notes_ordered_by_created_at",
    "events_ordered_by_seq",
];

/// Runs every case in [`CASES`].
///
/// `make` must return a store freshly loaded with [`crate::fixtures::DemoData`] and nothing else;
/// each case gets its own store, so a case is free to mint, edit and transition without
/// disturbing the next one. Panics on the first failed assertion, naming the case.
pub async fn run_all<S, F, Fut>(make: F)
where
    S: WriteStore,
    F: Fn() -> Fut,
    Fut: Future<Output = S>,
{
    mint_consecutive_keys(&make().await).await;
    mint_prefix_isolation(&make().await).await;
    mint_writes_revision_v1(&make().await).await;
    mint_unknown_kind_rejected(&make().await).await;
    update_cas_success(&make().await).await;
    update_cas_diverged(&make().await).await;
    status_cas_keeps_version(&make().await).await;
    no_delete_path(&make().await).await;
    links_hops_1_vs_2(&make().await).await;
    filter_status_project_tags_ready(&make().await).await;
    documents_ordered_by_version(&make().await).await;
    notes_ordered_by_created_at(&make().await).await;
    events_ordered_by_seq(&make().await).await;
}

/// The `Platform` workspace of the fixture: projects `htui` (position 0) then `agy` (position 1).
fn platform_scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
    }
}

/// A minimal mint request; the id is a fresh UUIDv7, never a fixture id.
fn new_item(project_id: ProjectId, kind_id: ItemKindId, title: &str) -> NewItem {
    NewItem {
        id: ItemId::new(),
        project_id,
        kind_id,
        title: title.to_owned(),
        body: String::new(),
        required_tags: Vec::new(),
        touched_paths: Vec::new(),
        priority: 0,
        step_graph_id: None,
        created_by: ids::USER,
        box_id: Some(ids::BOX),
    }
}

/// A title-only edit, authored by the fixture user.
fn title_patch(title: &str, reason: &str) -> ItemPatch {
    ItemPatch {
        title: Some(title.to_owned()),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: reason.to_owned(),
        ..ItemPatch::default()
    }
}

/// The ids of a list result, in the store's own order.
fn row_ids(rows: &[ItemSummary]) -> Vec<ItemId> {
    rows.iter().map(|row| row.id).collect()
}

/// Three mints with the `FEAT` kind of project `htui` continue the fixture's counter (§4.1).
async fn mint_consecutive_keys<S: WriteStore>(store: &S) {
    let mut keys = Vec::new();
    for expected in 4_i32..=6 {
        let item = store
            .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "minted"))
            .await
            .expect("mint_consecutive_keys: mint must succeed");
        assert_eq!(
            item.key_number, expected,
            "mint_consecutive_keys: counter starts above the fixture's highest FEAT"
        );
        keys.push(item.key);
    }
    assert_eq!(
        keys,
        vec!["FEAT-4", "FEAT-5", "FEAT-6"],
        "mint_consecutive_keys: keys are consecutive"
    );
}

/// Counters are per `(project, prefix)`: minting `ANA` does not move `FEAT` (§4.1).
async fn mint_prefix_isolation<S: WriteStore>(store: &S) {
    let ana = store
        .mint_item(new_item(
            ids::PROJECT_HTUI,
            ids::KIND_HTUI_ANA,
            "minted analysis",
        ))
        .await
        .expect("mint_prefix_isolation: ANA mint must succeed");
    assert_eq!(
        ana.key, "ANA-3",
        "mint_prefix_isolation: the ANA counter is its own"
    );

    let feat = store
        .mint_item(new_item(
            ids::PROJECT_HTUI,
            ids::KIND_HTUI_FEAT,
            "minted feature",
        ))
        .await
        .expect("mint_prefix_isolation: FEAT mint must succeed");
    assert_eq!(
        feat.key, "FEAT-4",
        "mint_prefix_isolation: the FEAT counter is unchanged"
    );
}

/// A mint writes revision 1 and opens the item (§4.1, §4.2).
async fn mint_writes_revision_v1<S: WriteStore>(store: &S) {
    let item = store
        .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "fresh"))
        .await
        .expect("mint_writes_revision_v1: mint must succeed");
    assert_eq!(item.version, 1, "mint_writes_revision_v1: version");
    assert_eq!(item.status, Status::Open, "mint_writes_revision_v1: status");
    assert_eq!(
        item.key_prefix, "FEAT",
        "mint_writes_revision_v1: key_prefix is copied from the kind"
    );

    // The edit can only land if revision 1 exists to be superseded (§4.2).
    let outcome = store
        .update_item(item.id, 1, title_patch("edited", "edited"))
        .await
        .expect("mint_writes_revision_v1: update must not fail");
    match outcome {
        UpdateOutcome::Updated(head) => {
            assert_eq!(
                head.version, 2,
                "mint_writes_revision_v1: version after edit"
            );
        }
        UpdateOutcome::Diverged { .. } => {
            panic!("mint_writes_revision_v1: a freshly minted item cannot diverge at version 1")
        }
    }
}

/// A kind that does not belong to the project is a constraint violation (§4.1).
async fn mint_unknown_kind_rejected<S: WriteStore>(store: &S) {
    let cross = store
        .mint_item(new_item(
            ids::PROJECT_HTUI,
            ids::KIND_AGY_FEAT,
            "cross-project",
        ))
        .await;
    assert!(
        matches!(cross, Err(StoreError::Constraint(_))),
        "mint_unknown_kind_rejected: a kind of another project must be rejected, got {cross:?}"
    );

    let absent = store
        .mint_item(new_item(
            ids::PROJECT_HTUI,
            ItemKindId::new(),
            "unknown kind",
        ))
        .await;
    assert!(
        matches!(absent, Err(StoreError::Constraint(_))),
        "mint_unknown_kind_rejected: an unknown kind must be rejected, got {absent:?}"
    );
}

/// A compare-and-set edit at the head version bumps the version and touches nothing else (§4.2).
async fn update_cas_success<S: WriteStore>(store: &S) {
    let before = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("update_cas_success: read must not fail")
        .expect("update_cas_success: the fixture item exists");

    let outcome = store
        .update_item(before.id, before.version, title_patch("Retitled", "edited"))
        .await
        .expect("update_cas_success: update must not fail");
    let UpdateOutcome::Updated(head) = outcome else {
        panic!("update_cas_success: the compare-and-set must match at the head version")
    };

    assert_eq!(
        head.version,
        before.version + 1,
        "update_cas_success: version"
    );
    assert_eq!(head.title, "Retitled", "update_cas_success: title");
    assert_eq!(head.body, before.body, "update_cas_success: body untouched");
    assert_eq!(
        head.kind_id, before.kind_id,
        "update_cas_success: kind_id untouched"
    );
    assert_eq!(
        head.priority, before.priority,
        "update_cas_success: priority untouched"
    );
    assert_eq!(
        head.required_tags, before.required_tags,
        "update_cas_success: required_tags untouched"
    );
    assert_eq!(
        head.status, before.status,
        "update_cas_success: status is not an edit (§4.2)"
    );
    assert_eq!(
        head.closed_at, before.closed_at,
        "update_cas_success: closed_at is not an edit"
    );
    assert_eq!(
        head.key, before.key,
        "update_cas_success: the key is never rewritten (§4.1)"
    );
}

/// A second edit at a stale version is refused with both sides and the ancestor (§4.2).
async fn update_cas_diverged<S: WriteStore>(store: &S) {
    let before = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("update_cas_diverged: read must not fail")
        .expect("update_cas_diverged: the fixture item exists");
    let stale = before.version;

    let first = store
        .update_item(before.id, stale, title_patch("Theirs", "edited"))
        .await
        .expect("update_cas_diverged: the first update must not fail");
    assert!(
        matches!(first, UpdateOutcome::Updated(_)),
        "update_cas_diverged: first update lands"
    );

    let second = store
        .update_item(before.id, stale, title_patch("Mine", "edited"))
        .await
        .expect("update_cas_diverged: the second update must not fail");
    let UpdateOutcome::Diverged { head, ancestor } = second else {
        panic!("update_cas_diverged: an edit from a stale version must diverge")
    };

    assert_eq!(head.version, stale + 1, "update_cas_diverged: head version");
    assert_eq!(
        head.title, "Theirs",
        "update_cas_diverged: head is the committed edit"
    );
    assert_eq!(
        ancestor.version, stale,
        "update_cas_diverged: ancestor version"
    );
    assert_eq!(
        ancestor.item_id, before.id,
        "update_cas_diverged: ancestor item"
    );
    assert_eq!(
        ancestor.title, before.title,
        "update_cas_diverged: ancestor is the pre-edit title"
    );
}

/// A status move is its own compare-and-set: it never bumps `version` (§4.2).
async fn status_cas_keeps_version<S: WriteStore>(store: &S) {
    let before = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the fixture item exists");
    assert_eq!(
        before.status,
        Status::Open,
        "status_cas_keeps_version: fixture precondition"
    );

    let moved = store
        .transition(before.id, Status::Open, Status::Queued)
        .await
        .expect("status_cas_keeps_version: transition must not fail");
    assert!(moved, "status_cas_keeps_version: open -> queued matches");

    let after = store
        .item(before.id)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the item still exists");
    assert_eq!(
        after.status,
        Status::Queued,
        "status_cas_keeps_version: status moved"
    );
    assert_eq!(
        after.version, before.version,
        "status_cas_keeps_version: version unchanged"
    );

    let stale = store
        .transition(before.id, Status::Open, Status::Done)
        .await
        .expect("status_cas_keeps_version: transition must not fail");
    assert!(
        !stale,
        "status_cas_keeps_version: a stale `from` is refused"
    );
    let unmoved = store
        .item(before.id)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the item still exists");
    assert_eq!(
        unmoved.status,
        Status::Queued,
        "status_cas_keeps_version: a refused move is a no-op"
    );

    // No revision was written: the edit at the pre-transition version still lands, and the
    // ancestor it leaves behind is still the creation revision.
    let outcome = store
        .update_item(
            before.id,
            before.version,
            title_patch("After the move", "edited"),
        )
        .await
        .expect("status_cas_keeps_version: update must not fail");
    assert!(
        matches!(outcome, UpdateOutcome::Updated(_)),
        "status_cas_keeps_version: the transition left the version line alone"
    );
    let diverged = store
        .update_item(before.id, before.version, title_patch("Mine", "edited"))
        .await
        .expect("status_cas_keeps_version: update must not fail");
    let UpdateOutcome::Diverged { ancestor, .. } = diverged else {
        panic!("status_cas_keeps_version: the stale edit must diverge")
    };
    assert_eq!(
        ancestor.version, before.version,
        "status_cas_keeps_version: ancestor version"
    );
    assert_eq!(
        ancestor.reason, "created",
        "status_cas_keeps_version: the transition wrote no revision"
    );
}

/// Items are never deleted; closing is a status (§4.1).
async fn no_delete_path<S: WriteStore>(store: &S) {
    let closed = store
        .transition(ids::HTUI_ANA_2, Status::Open, Status::Closed)
        .await
        .expect("no_delete_path: transition must not fail");
    assert!(closed, "no_delete_path: open -> closed matches");

    let still_there = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("no_delete_path: read must not fail");
    assert!(
        still_there.is_some(),
        "no_delete_path: a closed item is still readable"
    );

    let filter = ItemFilter {
        statuses: Some(vec![Status::Closed]),
        ..ItemFilter::default()
    };
    let rows = store
        .items(&platform_scope(), &filter)
        .await
        .expect("no_delete_path: list must not fail");
    assert!(
        row_ids(&rows).contains(&ids::HTUI_ANA_2),
        "no_delete_path: a closed item is still listed when the filter asks for closed"
    );
}

/// The link traversal of §7.3: live edges only, both directions, across projects.
async fn links_hops_1_vs_2<S: WriteStore>(store: &S) {
    let hop0 = store
        .links(ids::HTUI_FEAT_2, 0)
        .await
        .expect("links_hops_1_vs_2: hops 0 must not fail");
    assert_eq!(
        hop0.nodes.len(),
        1,
        "links_hops_1_vs_2: hops 0 is the root alone"
    );
    assert_eq!(
        hop0.nodes[0].item_id,
        ids::HTUI_FEAT_2,
        "links_hops_1_vs_2: hops 0 root"
    );
    assert_eq!(
        hop0.nodes[0].depth, 0,
        "links_hops_1_vs_2: the root sits at depth 0"
    );
    assert!(
        hop0.edges.is_empty(),
        "links_hops_1_vs_2: hops 0 has no edges"
    );

    let hop1 = store
        .links(ids::HTUI_FEAT_2, 1)
        .await
        .expect("links_hops_1_vs_2: hops 1 must not fail");
    let mut got = hop1
        .nodes
        .iter()
        .map(|node| node.item_id)
        .collect::<Vec<_>>();
    got.sort_unstable();
    let mut want = vec![ids::HTUI_FEAT_2, ids::HTUI_FEAT_1, ids::AGY_FEAT_1];
    want.sort_unstable();
    assert_eq!(got, want, "links_hops_1_vs_2: hops 1 nodes");
    assert_eq!(
        hop1.node(ids::HTUI_FEAT_1).map(|node| node.depth),
        Some(1),
        "links_hops_1_vs_2: the blocked_by target is one hop out"
    );

    let cross = hop1
        .node(ids::AGY_FEAT_1)
        .expect("links_hops_1_vs_2: the cross-project neighbour is here");
    assert_eq!(
        cross.depth, 1,
        "links_hops_1_vs_2: the incoming edge is followed too"
    );
    assert_eq!(
        cross.project_id,
        ids::PROJECT_AGY,
        "links_hops_1_vs_2: a cross-project node carries its own project_id"
    );
    assert_eq!(
        cross.project_slug, "agy",
        "links_hops_1_vs_2: the node is labelled by its project"
    );

    let hop2 = store
        .links(ids::HTUI_FEAT_2, 2)
        .await
        .expect("links_hops_1_vs_2: hops 2 must not fail");
    let mut got2 = hop2
        .nodes
        .iter()
        .map(|node| node.item_id)
        .collect::<Vec<_>>();
    got2.sort_unstable();
    let mut want2 = vec![
        ids::HTUI_FEAT_2,
        ids::HTUI_FEAT_1,
        ids::AGY_FEAT_1,
        ids::HTUI_ANA_1,
        ids::HTUI_FEAT_3,
    ];
    want2.sort_unstable();
    assert_eq!(
        got2, want2,
        "links_hops_1_vs_2: hops 2 adds the second ring"
    );
    assert_eq!(
        hop2.node(ids::HTUI_ANA_1).map(|node| node.depth),
        Some(2),
        "links_hops_1_vs_2: depth of the origin target"
    );
    assert_eq!(
        hop2.node(ids::HTUI_FEAT_3).map(|node| node.depth),
        Some(2),
        "links_hops_1_vs_2: depth of the incoming relates source"
    );

    assert!(
        hop2.node(ids::HTUI_TOOL_1).is_none(),
        "links_hops_1_vs_2: the tombstoned TOOL-1 -> FEAT-1 edge is not traversed"
    );
    assert!(
        !hop2
            .edges
            .iter()
            .any(|edge| edge.from_item_id == ids::HTUI_TOOL_1),
        "links_hops_1_vs_2: a tombstoned edge is never returned"
    );
    assert!(
        hop2.edges
            .iter()
            .any(|edge| edge.from_item_id == ids::HTUI_FEAT_2
                && edge.to_item_id == ids::HTUI_FEAT_1
                && edge.kind == LinkKind::BlockedBy),
        "links_hops_1_vs_2: the blocked_by edge is returned with its kind"
    );
}

/// Every [`ItemFilter`] field, against the fixture's `Platform` workspace.
async fn filter_status_project_tags_ready<S: WriteStore>(store: &S) {
    let scope = platform_scope();

    let by_status = store
        .items(
            &scope,
            &ItemFilter {
                statuses: Some(vec![Status::Open]),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("filter_status_project_tags_ready: list must not fail");
    assert_eq!(
        row_ids(&by_status),
        vec![ids::HTUI_ANA_2, ids::AGY_FEAT_1, ids::AGY_FIX_1],
        "filter_status_project_tags_ready: statuses, ordered by scope position then key"
    );

    let by_project = store
        .items(
            &scope,
            &ItemFilter {
                project_ids: Some(vec![ids::PROJECT_AGY]),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("filter_status_project_tags_ready: list must not fail");
    assert_eq!(
        row_ids(&by_project),
        vec![ids::AGY_ANA_1, ids::AGY_FEAT_1, ids::AGY_FIX_1],
        "filter_status_project_tags_ready: project_ids narrows the scope"
    );

    let by_tag = store
        .items(
            &scope,
            &ItemFilter {
                tags: Some(vec!["rust".to_owned()]),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("filter_status_project_tags_ready: list must not fail");
    assert_eq!(
        row_ids(&by_tag),
        vec![
            ids::HTUI_FEAT_1,
            ids::HTUI_FEAT_2,
            ids::HTUI_FEAT_3,
            ids::AGY_FEAT_1
        ],
        "filter_status_project_tags_ready: tags are matched against required_tags"
    );

    let ready = store
        .items(
            &scope,
            &ItemFilter {
                ready: Some(true),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("filter_status_project_tags_ready: list must not fail");
    assert_eq!(
        row_ids(&ready),
        vec![ids::HTUI_ANA_2, ids::AGY_FEAT_1, ids::AGY_FIX_1],
        "filter_status_project_tags_ready: ready excludes every non-open item"
    );
    assert!(
        !row_ids(&ready).contains(&ids::HTUI_FEAT_2),
        "filter_status_project_tags_ready: FEAT-2 is blocked by the non-terminal FEAT-1"
    );

    // The blocked_by half of §7.4 on its own: open FEAT-2 up and it is still not ready, because
    // FEAT-1 is in_progress.
    let opened = store
        .transition(ids::HTUI_FEAT_2, Status::Blocked, Status::Open)
        .await
        .expect("filter_status_project_tags_ready: transition must not fail");
    assert!(
        opened,
        "filter_status_project_tags_ready: blocked -> open matches"
    );
    let ready_again = store
        .items(
            &scope,
            &ItemFilter {
                ready: Some(true),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("filter_status_project_tags_ready: list must not fail");
    assert!(
        !row_ids(&ready_again).contains(&ids::HTUI_FEAT_2),
        "filter_status_project_tags_ready: a live blocked_by to a non-terminal item blocks (§7.4)"
    );
}

/// Documents are grouped by kind and ascend by version (§5.5, append-only).
async fn documents_ordered_by_version<S: WriteStore>(store: &S) {
    let docs = store
        .documents(ids::HTUI_FEAT_1)
        .await
        .expect("documents_ordered_by_version: read must not fail");
    let shape = docs
        .iter()
        .map(|doc| (doc.kind.as_str(), doc.version))
        .collect::<Vec<_>>();
    assert_eq!(
        shape,
        vec![("plan", 1), ("plan", 2), ("prd", 1)],
        "documents_ordered_by_version: ordered by (kind, version)"
    );
    assert!(
        docs.iter().all(|doc| doc.item_id == ids::HTUI_FEAT_1),
        "documents_ordered_by_version: only the item's own documents"
    );
    assert!(
        docs.iter().any(|doc| doc.produced_by_step_id.is_some()),
        "documents_ordered_by_version: a step-produced document is distinguishable"
    );
    assert!(
        docs.iter().any(|doc| doc.produced_by_step_id.is_none()),
        "documents_ordered_by_version: a hand-written document is distinguishable"
    );
}

/// Notes ascend by `created_at` and the order is stable (§5.5, append-only).
async fn notes_ordered_by_created_at<S: WriteStore>(store: &S) {
    let notes = store
        .notes(ids::HTUI_FEAT_1)
        .await
        .expect("notes_ordered_by_created_at: read must not fail");
    assert_eq!(
        notes.iter().map(|note| note.id).collect::<Vec<_>>(),
        vec![ids::NOTE_1, ids::NOTE_2],
        "notes_ordered_by_created_at: ascending"
    );
    assert!(
        notes
            .windows(2)
            .all(|pair| pair[0].created_at <= pair[1].created_at),
        "notes_ordered_by_created_at: created_at never decreases"
    );

    let again = store
        .notes(ids::HTUI_FEAT_1)
        .await
        .expect("notes_ordered_by_created_at: read must not fail");
    assert_eq!(
        notes, again,
        "notes_ordered_by_created_at: stable across reads"
    );
}

/// A step's replay log is ordered by `seq`, and an uncached step reads as `None` (§4.3, §7.5).
async fn events_ordered_by_seq<S: WriteStore>(store: &S) {
    let events = store
        .step_events(ids::STEP_PLAN)
        .await
        .expect("events_ordered_by_seq: read must not fail")
        .expect("events_ordered_by_seq: the plan step has a cached log");
    assert_eq!(
        events.iter().map(|event| event.seq).collect::<Vec<_>>(),
        (0..8).collect::<Vec<i32>>(),
        "events_ordered_by_seq: seq 0..7 in order"
    );
    assert_eq!(
        events[0].kind,
        EventKind::Prompt,
        "events_ordered_by_seq: seq 0 is the prompt"
    );
    assert_eq!(
        events[0].role,
        EventRole::Htui,
        "events_ordered_by_seq: htui assembles the prompt"
    );
    assert_eq!(
        events[0].turn, 0,
        "events_ordered_by_seq: the prompt opens turn 0"
    );
    assert!(
        events
            .iter()
            .all(|event| event.run_step_id == ids::STEP_PLAN),
        "events_ordered_by_seq: only the step's own events"
    );
    assert_eq!(
        events[3].tool_call_id, events[4].tool_call_id,
        "events_ordered_by_seq: a tool_result carries its call's tool_call_id (§4.3)"
    );
    assert!(
        events.iter().all(|event| event.raw.is_none()),
        "events_ordered_by_seq: keep_raw_events is off in the fixture"
    );

    let uncached = store
        .step_events(StepId::new())
        .await
        .expect("events_ordered_by_seq: read must not fail");
    assert!(
        uncached.is_none(),
        "events_ordered_by_seq: an uncached step is None, not an error"
    );
}

#[cfg(test)]
mod tests {
    use super::CASES;

    #[test]
    fn case_names_are_unique() {
        let mut sorted = CASES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
    }
}
