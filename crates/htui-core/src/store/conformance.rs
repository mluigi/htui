//! Store conformance suite (feature `test-support`), blueprint B.9.
//!
//! Every rule of `docs/ANA-9.md` §4.1, §4.2 and §7 a store has to show, written against
//! [`WriteStore`] alone: no concrete store is named anywhere in this module, so MOD-6 runs the
//! same cases against `PgStore` and reports per case. The cases assert against the demo fixture
//! (`test-support` implies `demo`), which is why the two features are not independent: MOD-6
//! inherits a seed data set instead of inventing a second one.

use core::future::Future;

use chrono::Utc;
use serde_json::{Value, json};

use crate::fixtures::ids;
use crate::model::{
    Agent, AgentBox, AgentId, Billing, ChatRunSpec, DocumentId, EventKind, EventRole, ItemFilter,
    ItemId, ItemKindId, ItemPatch, ItemSummary, LinkKind, NewItem, ProjectId, PromptScope, RunId,
    RunStatus, Scope, SessionEvent, Status, StepId, Transport, UpstreamEntry, UserId,
};
use crate::store::error::StoreError;
use crate::store::traits::{ReadStore, UpdateOutcome, WriteStore};

/// Case names in run order. A name never changes: MOD-6 reports per case.
pub const CASES: &[&str] = &[
    "mint_consecutive_keys",
    "mint_prefix_isolation",
    "mint_writes_revision_v1",
    "mint_unknown_kind_rejected",
    "update_cas_success",
    "update_kind_keeps_key_and_project",
    "update_cas_diverged",
    "status_cas_keeps_version",
    "no_delete_path",
    "links_hops_1_vs_2",
    "filter_status_project_tags_ready",
    "documents_ordered_by_version",
    "notes_ordered_by_created_at",
    "events_ordered_by_seq",
    "nil_author_rejected",
    "append_events_idempotent_and_ordered",
    "set_step_usage_writes_usage_and_digest",
    "start_chat_run_mints_chat_rows",
    "upsert_agent_by_id_name_unique",
    "upsert_agent_box_by_pk",
    "set_agent_box_quota_updates_two_columns_or_not_found",
    "upsert_agent_box_cannot_write_quota",
    "set_step_prompt_writes_digest_and_trim",
];

/// Runs one case by name against an already-loaded store.
///
/// # Panics
///
/// On the first failed assertion, naming the case, and on an unknown `name`. The return type is
/// `()` and not `Result<(), String>` deliberately: the cases are written as `assert_eq!` chains
/// that carry their own messages, and turning twenty bodies into error-returning functions would
/// rewrite the whole suite to gain a string the panic already prints. MOD-6's `pg_conformance.rs`
/// gets its per-case reporting from the loop, not from a `Result`.
pub async fn run_case<S: WriteStore>(name: &str, store: &S) {
    match name {
        "mint_consecutive_keys" => mint_consecutive_keys(store).await,
        "mint_prefix_isolation" => mint_prefix_isolation(store).await,
        "mint_writes_revision_v1" => mint_writes_revision_v1(store).await,
        "mint_unknown_kind_rejected" => mint_unknown_kind_rejected(store).await,
        "update_cas_success" => update_cas_success(store).await,
        "update_kind_keeps_key_and_project" => update_kind_keeps_key_and_project(store).await,
        "update_cas_diverged" => update_cas_diverged(store).await,
        "status_cas_keeps_version" => status_cas_keeps_version(store).await,
        "no_delete_path" => no_delete_path(store).await,
        "links_hops_1_vs_2" => links_hops_1_vs_2(store).await,
        "filter_status_project_tags_ready" => filter_status_project_tags_ready(store).await,
        "documents_ordered_by_version" => documents_ordered_by_version(store).await,
        "notes_ordered_by_created_at" => notes_ordered_by_created_at(store).await,
        "events_ordered_by_seq" => events_ordered_by_seq(store).await,
        "nil_author_rejected" => nil_author_rejected(store).await,
        "append_events_idempotent_and_ordered" => append_events_idempotent_and_ordered(store).await,
        "set_step_usage_writes_usage_and_digest" => {
            set_step_usage_writes_usage_and_digest(store).await;
        }
        "start_chat_run_mints_chat_rows" => start_chat_run_mints_chat_rows(store).await,
        "upsert_agent_by_id_name_unique" => upsert_agent_by_id_name_unique(store).await,
        "upsert_agent_box_by_pk" => upsert_agent_box_by_pk(store).await,
        "set_agent_box_quota_updates_two_columns_or_not_found" => {
            set_agent_box_quota_updates_two_columns_or_not_found(store).await;
        }
        "upsert_agent_box_cannot_write_quota" => upsert_agent_box_cannot_write_quota(store).await,
        "set_step_prompt_writes_digest_and_trim" => {
            set_step_prompt_writes_digest_and_trim(store).await;
        }
        other => panic!("unknown conformance case `{other}`; CASES and run_case disagree"),
    }
}

/// Runs every case in [`CASES`], each against a store `make` produced fresh.
///
/// `make` must return a store freshly loaded with [`crate::fixtures::DemoData`] and nothing else;
/// each case gets its own store, so a case is free to mint, edit and transition without
/// disturbing the next one.
///
/// # Panics
///
/// On the first failed assertion, naming the case.
pub async fn run_all<S, F, Fut>(make: F)
where
    S: WriteStore,
    F: Fn() -> Fut,
    Fut: Future<Output = S>,
{
    for name in CASES {
        run_case(name, &make().await).await;
    }
}

/// The read-only half of the suite, in run order (plan D96). A name never changes.
///
/// A second list beside [`CASES`] rather than a relaxed bound on [`run_case`]: eleven of the
/// twenty-three cases above call [`WriteStore`] methods, so `run_case` cannot be generalised to
/// [`ReadStore`] without splitting every one of them. These six are written against `ReadStore`
/// alone, which is what lets the **mirror** be a target — `CacheStore` implements `ReadStore` and
/// not `WriteStore`, so before this list there was no way to assert that an offline read and an
/// online one agree beyond comparing them pairwise in `htui-store`'s own tests.
///
/// Every case here reads the fixture and writes nothing, so unlike [`CASES`] they may share one
/// store; [`run_all_reads`] still builds a fresh one per case, because a `make` that refreshes a
/// mirror is cheaper to write than one that promises not to have been disturbed.
pub const READ_CASES: &[&str] = &[
    "document_body_round_trip",
    "documents_of_kinds_latest_per_kind_in_order",
    "project_row_has_settings",
    "upstream_diamond_dedup",
    "upstream_in_scope_no_summary",
    "upstream_out_of_scope_stub",
];

/// Runs one [`READ_CASES`] case by name against an already-loaded store.
///
/// # Panics
///
/// On the first failed assertion, naming the case, and on an unknown `name`.
pub async fn run_read_case<S: ReadStore>(name: &str, store: &S) {
    match name {
        "document_body_round_trip" => document_body_round_trip(store).await,
        "documents_of_kinds_latest_per_kind_in_order" => {
            documents_of_kinds_latest_per_kind_in_order(store).await;
        }
        "project_row_has_settings" => project_row_has_settings(store).await,
        "upstream_diamond_dedup" => upstream_diamond_dedup(store).await,
        "upstream_in_scope_no_summary" => upstream_in_scope_no_summary(store).await,
        "upstream_out_of_scope_stub" => upstream_out_of_scope_stub(store).await,
        other => panic!("unknown read case `{other}`; READ_CASES and run_read_case disagree"),
    }
}

/// Runs every case in [`READ_CASES`], each against a store `make` produced fresh.
///
/// # Panics
///
/// On the first failed assertion, naming the case.
pub async fn run_all_reads<S, F, Fut>(make: F)
where
    S: ReadStore,
    F: Fn() -> Fut,
    Fut: Future<Output = S>,
{
    for name in READ_CASES {
        run_read_case(name, &make().await).await;
    }
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

/// Re-kinding keeps the key minted under the old kind and never moves the item between projects
/// (§4.1: `key_prefix` is copied at mint time and never rewritten; §4.2 covers `kind_id`).
async fn update_kind_keeps_key_and_project<S: WriteStore>(store: &S) {
    let before = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("update_kind_keeps_key_and_project: read must not fail")
        .expect("update_kind_keeps_key_and_project: the fixture item exists");

    let outcome = store
        .update_item(
            before.id,
            before.version,
            ItemPatch {
                kind_id: Some(ids::KIND_HTUI_FEAT),
                ..title_patch(&before.title, "edited")
            },
        )
        .await
        .expect("update_kind_keeps_key_and_project: update must not fail");
    let UpdateOutcome::Updated(head) = outcome else {
        panic!("update_kind_keeps_key_and_project: the edit must match at the head version")
    };

    assert_eq!(
        head.kind_id,
        ids::KIND_HTUI_FEAT,
        "update_kind_keeps_key_and_project: kind_id is a covered column (§4.2)"
    );
    assert_eq!(
        head.version,
        before.version + 1,
        "update_kind_keeps_key_and_project: version"
    );
    assert_eq!(
        head.key_prefix, before.key_prefix,
        "update_kind_keeps_key_and_project: key_prefix is never rewritten (§4.1)"
    );
    assert_eq!(
        head.key_number, before.key_number,
        "update_kind_keeps_key_and_project: key_number is never rewritten (§4.1)"
    );
    assert_eq!(
        head.key, before.key,
        "update_kind_keeps_key_and_project: the key is never rewritten (§4.1)"
    );
    assert_eq!(
        head.project_id, before.project_id,
        "update_kind_keeps_key_and_project: an edit never moves an item between projects"
    );

    let cross = store
        .update_item(
            head.id,
            head.version,
            ItemPatch {
                kind_id: Some(ids::KIND_AGY_FEAT),
                ..title_patch(&head.title, "edited")
            },
        )
        .await;
    assert!(
        matches!(cross, Err(StoreError::Constraint(_))),
        "update_kind_keeps_key_and_project: a kind of another project is refused, got {cross:?}"
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

    // `closed_at` tracks the current status, not the history: a move to a terminal status sets
    // it, a move back to a live one clears it (§4.2; blueprint Errata).
    assert!(
        store
            .transition(before.id, Status::Queued, Status::Done)
            .await
            .expect("status_cas_keeps_version: transition must not fail"),
        "status_cas_keeps_version: queued -> done matches"
    );
    let done = store
        .item(before.id)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the item still exists");
    assert!(
        done.closed_at.is_some(),
        "status_cas_keeps_version: a terminal move sets closed_at"
    );
    assert!(
        store
            .transition(before.id, Status::Done, Status::Open)
            .await
            .expect("status_cas_keeps_version: transition must not fail"),
        "status_cas_keeps_version: done -> open matches"
    );
    let reopened = store
        .item(before.id)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the item still exists");
    assert_eq!(
        reopened.closed_at, None,
        "status_cas_keeps_version: a reopen clears closed_at"
    );
    assert_eq!(
        reopened.version, before.version,
        "status_cas_keeps_version: neither move bumped the version"
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

/// A revision author must name a real `app_user` row: the nil UUID that `UserId::default()`
/// yields is refused on both write paths (§5.5 `author_id REFERENCES app_user(id)`).
async fn nil_author_rejected<S: WriteStore>(store: &S) {
    let minted = store
        .mint_item(NewItem {
            created_by: UserId::default(),
            ..new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "no author")
        })
        .await;
    assert!(
        matches!(minted, Err(StoreError::Constraint(_))),
        "nil_author_rejected: a mint without a known author is refused, got {minted:?}"
    );

    // The refused mint consumed no key number: a §4.1 counter only moves for a mint that lands.
    let landed = store
        .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "authored"))
        .await
        .expect("nil_author_rejected: an authored mint must succeed");
    assert_eq!(
        landed.key, "FEAT-4",
        "nil_author_rejected: the refused mint left the counter alone"
    );

    let before = store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("nil_author_rejected: read must not fail")
        .expect("nil_author_rejected: the fixture item exists");
    let edited = store
        .update_item(
            before.id,
            before.version,
            ItemPatch {
                author_id: UserId::default(),
                ..title_patch("Retitled", "edited")
            },
        )
        .await;
    assert!(
        matches!(edited, Err(StoreError::Constraint(_))),
        "nil_author_rejected: an edit without a known author is refused, got {edited:?}"
    );

    let after = store
        .item(before.id)
        .await
        .expect("nil_author_rejected: read must not fail")
        .expect("nil_author_rejected: the item still exists");
    assert_eq!(
        after.version, before.version,
        "nil_author_rejected: the refused edit left no half-applied change"
    );
    assert_eq!(
        after.title, before.title,
        "nil_author_rejected: the refused edit wrote no field"
    );
}

// ------------------------------------------------------------------------------------------
// MOD-2 (plan D3, D4, D15): the five write paths the driver records through.
//
// Every assertion below goes through [`WriteStore`] / [`ReadStore`] alone, which is what the
// suite is for. Three of the six methods write columns no §6.1 read returns - `run_step.usage`,
// `run_step.prompt_digest` and the `agent` / `agent_box` rows, which `agents()` answers *inherently*
// rather than through `ReadStore` (D3) - so the cases pin the *rules* (idempotence, ordering,
// uniqueness, refusals) and the per-backend tests pin the *column values*:
// `crates/htui-store/tests/pg_criteria.rs` reads them back in SQL and `store::mem`'s own unit tests
// read them out of `State`. D15(a) defers a `ReadStore`-shaped suite to milestone 9; adding a read
// method here to make a case observable is exactly what that ruling forbids.
// ------------------------------------------------------------------------------------------

/// One `assistant_text` row for `step` at `seq`, in the §4.3 payload shape.
fn chat_event(step: StepId, seq: i32) -> SessionEvent {
    SessionEvent {
        run_step_id: step,
        seq,
        turn: 0,
        kind: EventKind::AssistantText,
        role: EventRole::Agent,
        tool_call_id: None,
        payload: json!({ "text": format!("chunk {seq}") }),
        raw: None,
        at: Utc::now(),
    }
}

/// A registry row with the given id and name; every other column is a legal §5.7 value.
fn test_agent(id: AgentId, name: &str, model: &str) -> Agent {
    let now = Utc::now();
    Agent {
        id,
        name: name.to_owned(),
        transport: Transport::Cli,
        launch: json!({ "command": "tester", "args": [], "env": {} }),
        models: vec![model.to_owned()],
        default_model: Some(model.to_owned()),
        billing: Billing::PerToken,
        enabled: true,
        settings: json!({ "cli": { "stream": "fake" } }),
        created_at: now,
        updated_at: now,
    }
}

/// `append_events` is idempotent on `(run_step_id, seq)`, returns the number of rows it actually
/// inserted, orders by `seq` on the way out, and refuses a step that does not exist (§4.3, D3).
async fn append_events_idempotent_and_ordered<S: WriteStore>(store: &S) {
    let step = ids::STEP_IMPL;
    assert!(
        store
            .step_events(step)
            .await
            .expect("append_events_idempotent_and_ordered: read must not fail")
            .is_none(),
        "append_events_idempotent_and_ordered: the fixture caches no log for the implement step"
    );

    let first: Vec<SessionEvent> = (0..5).map(|seq| chat_event(step, seq)).collect();
    assert_eq!(
        store
            .append_events(&first)
            .await
            .expect("append_events_idempotent_and_ordered: the first append must land"),
        5,
        "append_events_idempotent_and_ordered: five new rows are five inserts"
    );

    // The same five plus two more: the primary key swallows the replay and only the new pair lands.
    let mut again = first.clone();
    again.push(chat_event(step, 5));
    again.push(chat_event(step, 6));
    assert_eq!(
        store
            .append_events(&again)
            .await
            .expect("append_events_idempotent_and_ordered: the replay must not fail"),
        2,
        "append_events_idempotent_and_ordered: a replayed row is skipped, not counted (§4.3)"
    );

    let events = store
        .step_events(step)
        .await
        .expect("append_events_idempotent_and_ordered: read must not fail")
        .expect("append_events_idempotent_and_ordered: the step now has a log");
    assert_eq!(
        events.iter().map(|event| event.seq).collect::<Vec<i32>>(),
        (0..7).collect::<Vec<i32>>(),
        "append_events_idempotent_and_ordered: seq 0..7 in order, no duplicate"
    );
    assert_eq!(
        events[0].payload,
        json!({ "text": "chunk 0" }),
        "append_events_idempotent_and_ordered: the replay did not overwrite the first write"
    );

    // A step that does not exist is a constraint violation, and the whole batch is refused: the
    // statement is one `INSERT`, so a bad row cannot leave the good ones behind (§5.8 FK).
    let orphan = StepId::new();
    let mixed = vec![chat_event(step, 7), chat_event(orphan, 0)];
    let refused = store.append_events(&mixed).await;
    assert!(
        matches!(refused, Err(StoreError::Constraint(_))),
        "append_events_idempotent_and_ordered: an unknown run_step_id is refused, got {refused:?}"
    );
    let after = store
        .step_events(step)
        .await
        .expect("append_events_idempotent_and_ordered: read must not fail")
        .expect("append_events_idempotent_and_ordered: the step still has its log");
    assert_eq!(
        after.len(),
        7,
        "append_events_idempotent_and_ordered: a refused batch writes none of its rows"
    );

    assert_eq!(
        store
            .append_events(&[])
            .await
            .expect("append_events_idempotent_and_ordered: an empty batch must not fail"),
        0,
        "append_events_idempotent_and_ordered: an empty batch inserts nothing"
    );
}

/// `set_step_usage` writes `run_step.usage` every time and `run_step.prompt_digest` only when the
/// caller supplies one; an unknown step is [`StoreError::NotFound`] (ANA-4 §4.1, D15(b)).
///
/// Neither column is readable through §6.1, so the values are asserted per backend: `store::mem`'s
/// `set_step_usage_writes_usage_every_time_and_the_digest_only_when_supplied` and
/// `pg_criteria.rs::set_step_usage_keeps_the_digest_a_none_call_does_not_supply`, each of which
/// makes both writes below and then reads the two columns back. The names are checked by
/// `every_cross_referenced_test_name_exists` in this module's `tests`.
async fn set_step_usage_writes_usage_and_digest<S: WriteStore>(store: &S) {
    let step = ids::STEP_IMPL;
    store
        .set_step_usage(
            step,
            json!({ "input_tokens": 10, "output_tokens": 20 }),
            Some("9f8e7d".to_owned()),
        )
        .await
        .expect("set_step_usage_writes_usage_and_digest: the first write must land");
    store
        .set_step_usage(step, json!({ "input_tokens": 30 }), None)
        .await
        .expect("set_step_usage_writes_usage_and_digest: a digest-free write must land");

    let unknown = store.set_step_usage(StepId::new(), json!({}), None).await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "set_step_usage_writes_usage_and_digest: an unknown step is NotFound, got {unknown:?}"
    );
}

/// `start_chat_run` mints the `run` / `run_step` pair a free-standing chat records into, is a
/// no-op on replay, leaves every item's run list alone, and `finish_chat_run` closes both rows
/// (plan D4).
///
/// It is the same *pair of rows*, addressed by the same client-side ids, that
/// `crates/htui-store/src/cache/pending.rs` inserts on upload - not the same column values: see
/// [`ChatRunSpec`] for the asymmetry `ON CONFLICT (id) DO NOTHING` leaves between the two paths.
async fn start_chat_run_mints_chat_rows<S: WriteStore>(store: &S) {
    let before = store
        .runs(ids::HTUI_FEAT_1)
        .await
        .expect("start_chat_run_mints_chat_rows: read must not fail");

    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    );

    // Nothing to record into yet: the step row is what the event FK points at.
    let early = store.append_events(&[chat_event(chat.step_id, 0)]).await;
    assert!(
        matches!(early, Err(StoreError::Constraint(_))),
        "start_chat_run_mints_chat_rows: no step exists before the mint, got {early:?}"
    );

    store
        .start_chat_run(&chat)
        .await
        .expect("start_chat_run_mints_chat_rows: the mint must land");
    let rows: Vec<SessionEvent> = (0..3).map(|seq| chat_event(chat.step_id, seq)).collect();
    assert_eq!(
        store
            .append_events(&rows)
            .await
            .expect("start_chat_run_mints_chat_rows: the chat's events must land"),
        3,
        "start_chat_run_mints_chat_rows: the minted step accepts events"
    );

    // `ON CONFLICT (id) DO NOTHING` on both rows: a second start is a no-op, not a duplicate-key
    // error and not a truncation of what the first one recorded (D4).
    store
        .start_chat_run(&chat)
        .await
        .expect("start_chat_run_mints_chat_rows: a replayed mint must not fail");
    assert_eq!(
        store
            .step_events(chat.step_id)
            .await
            .expect("start_chat_run_mints_chat_rows: read must not fail")
            .expect("start_chat_run_mints_chat_rows: the chat step has a log")
            .len(),
        3,
        "start_chat_run_mints_chat_rows: the replayed mint left the recorded rows alone"
    );

    // `item_id` is NULL, so a chat never shows up under an item (§5.8).
    assert_eq!(
        store
            .runs(ids::HTUI_FEAT_1)
            .await
            .expect("start_chat_run_mints_chat_rows: read must not fail"),
        before,
        "start_chat_run_mints_chat_rows: a chat run belongs to no item"
    );

    store
        .finish_chat_run(chat.run_id, chat.step_id, RunStatus::Done, Utc::now())
        .await
        .expect("start_chat_run_mints_chat_rows: the close must land");

    let missing = store
        .finish_chat_run(RunId::new(), StepId::new(), RunStatus::Done, Utc::now())
        .await;
    assert!(
        matches!(missing, Err(StoreError::NotFound { entity: "run", .. })),
        "start_chat_run_mints_chat_rows: closing an unknown run is NotFound, got {missing:?}"
    );

    // `RunStatus` and `StepStatus` share only `done | failed | cancelled`; a live status has no
    // `run_step` counterpart, so it is refused rather than half-written (D4).
    let live = store
        .finish_chat_run(chat.run_id, chat.step_id, RunStatus::Queued, Utc::now())
        .await;
    assert!(
        matches!(live, Err(StoreError::Constraint(_))),
        "start_chat_run_mints_chat_rows: a non-terminal finish status is refused, got {live:?}"
    );
}

/// `upsert_agent` keys on `agent.id` and keeps `agent.name` unique (§5.7 `name TEXT NOT NULL
/// UNIQUE`): a second write of the same id is an update, a second id under a taken name is refused.
///
/// The registry read is inherent, not a [`WriteStore`] or `ReadStore` method (MOD-2 plan D3), so
/// this case can assert only that the second write is *accepted*. That the row was updated **in
/// place** and kept its `created_at` is read back per backend, by
/// `pg_criteria.rs::upsert_agent_updates_in_place_and_keeps_created_at`.
async fn upsert_agent_by_id_name_unique<S: WriteStore>(store: &S) {
    let id = AgentId::new();
    store
        .upsert_agent(&test_agent(id, "tester", "small"))
        .await
        .expect("upsert_agent_by_id_name_unique: the insert must land");

    // Same id, different columns: an update in place, not a duplicate-key refusal.
    store
        .upsert_agent(&test_agent(id, "tester", "large"))
        .await
        .expect("upsert_agent_by_id_name_unique: the update must land");

    let stolen = store
        .upsert_agent(&test_agent(AgentId::new(), "tester", "small"))
        .await;
    assert!(
        matches!(stolen, Err(StoreError::Constraint(_))),
        "upsert_agent_by_id_name_unique: a second id under a taken name is refused, got {stolen:?}"
    );

    let seeded = store
        .upsert_agent(&test_agent(AgentId::new(), "claude", "small"))
        .await;
    assert!(
        matches!(seeded, Err(StoreError::Constraint(_))),
        "upsert_agent_by_id_name_unique: the fixture's own names are taken too, got {seeded:?}"
    );

    // The refused writes left the row that did land alone: it still answers to its own id.
    store
        .upsert_agent(&test_agent(id, "tester", "largest"))
        .await
        .expect("upsert_agent_by_id_name_unique: the row is still updatable by its id");
}

/// `upsert_agent_box` keys on the composite primary key `(agent_id, box_id)` and needs both
/// referents to exist (§5.7).
///
/// The `probe` snapshot (`docs/ANA-4.md` §4.6, MOD-2 D44) rides the same three writes: it lands on
/// the insert, a second upsert replaces it, and a third clears it with `None`. Only the writes are
/// asserted here - [`WriteStore`] has no registry read, so the read-back lives where a concrete
/// store can be named (`MemStore`'s `agents_join_this_box_only` and `pg_criteria.rs`).
async fn upsert_agent_box_by_pk<S: WriteStore>(store: &S) {
    let row = AgentBox {
        agent_id: ids::AGENT_CLAUDE,
        box_id: ids::BOX,
        enabled: true,
        version: Some("1.2.3".to_owned()),
        path: Some("/usr/bin/claude".to_owned()),
        probed_at: Some(Utc::now()),
        quota: Some(json!({ "remaining": 100 })),
        quota_at: Some(Utc::now()),
        updated_at: Utc::now(),
        probe: Some(json!({ "status": "ready", "source": "probe" })),
    };
    store
        .upsert_agent_box(&row)
        .await
        .expect("upsert_agent_box_by_pk: the insert must land");

    store
        .upsert_agent_box(&AgentBox {
            enabled: false,
            version: Some("1.3.0".to_owned()),
            quota: None,
            quota_at: None,
            probe: Some(json!({ "status": "unauthenticated", "source": "probe" })),
            ..row.clone()
        })
        .await
        .expect("upsert_agent_box_by_pk: the same primary key is an update, not a duplicate");

    store
        .upsert_agent_box(&AgentBox {
            probe: None,
            ..row.clone()
        })
        .await
        .expect("upsert_agent_box_by_pk: a `None` probe clears the column, it does not refuse");

    let orphan = store
        .upsert_agent_box(&AgentBox {
            agent_id: AgentId::new(),
            ..row
        })
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "upsert_agent_box_by_pk: a row for an unknown agent is refused, got {orphan:?}"
    );
}

/// `set_agent_box_quota` writes the two quota columns of an **existing** row and refuses a key it
/// does not hold (MOD-2 plan D67).
///
/// The narrow setter has no insert path: a row that has never been probed has no columns to latch
/// into, so an unknown `(agent_id, box_id)` is
/// [`StoreError::NotFound`], never a silent no-op and never a fresh row. What the write leaves
/// alone - `probe` above all - is asserted where a concrete store can be named and read back
/// (`mem.rs::set_agent_box_quota_leaves_probe_and_version_alone` and
/// `pg_criteria.rs::set_agent_box_quota_leaves_probe_byte_identical`), because [`WriteStore`]
/// carries no registry read.
async fn set_agent_box_quota_updates_two_columns_or_not_found<S: WriteStore>(store: &S) {
    store
        .upsert_agent_box(&AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            enabled: true,
            version: Some("1.2.3".to_owned()),
            path: Some("/usr/bin/claude".to_owned()),
            probed_at: Some(Utc::now()),
            quota: None,
            quota_at: None,
            updated_at: Utc::now(),
            probe: Some(json!({ "status": "ready", "source": "probe" })),
        })
        .await
        .expect("set_agent_box_quota_updates_two_columns_or_not_found: the probed row must land");

    store
        .set_agent_box_quota(
            ids::AGENT_CLAUDE,
            ids::BOX,
            json!({ "source": "none", "spend": { "session_micros": 7, "currency": "USD" } }),
            Utc::now(),
        )
        .await
        .expect("set_agent_box_quota_updates_two_columns_or_not_found: the latch lands on a row");

    let missing = store
        .set_agent_box_quota(
            AgentId::new(),
            ids::BOX,
            json!({ "source": "none" }),
            Utc::now(),
        )
        .await;
    assert!(
        matches!(
            missing,
            Err(StoreError::NotFound {
                entity: "agent_box",
                ..
            })
        ),
        "set_agent_box_quota_updates_two_columns_or_not_found: an unprobed row has nothing to \
         latch into, got {missing:?}"
    );
}

/// `upsert_agent_box` neither sets nor clears `quota` / `quota_at`: since MOD-2 plan D74 the two
/// columns are [`WriteStore::set_agent_box_quota`]'s alone.
///
/// This case pins the **sharp edge** the design keeps: an [`AgentBox`] still *carries* both
/// fields, and handing an upsert a populated `quota` is neither an error nor a write. It is
/// accepted and ignored, on the insert path and on the conflict path alike, so a probe that read a
/// row seconds ago cannot write a stale allowance back over a latch that landed in between - the
/// lost update D74 exists to remove.
///
/// What the *stored* values then are is asserted where a store can be read back, because
/// [`WriteStore`] carries no `agent_box` read: `mem.rs::upsert_agent_box_cannot_write_the_quota_columns`
/// in-module, and `pg_criteria.rs::an_upsert_can_neither_set_nor_clear_the_quota_columns` with a
/// raw `SELECT`. That one has to be SQL-level: the thing being removed is an `EXCLUDED.quota`
/// line, and no trait call can see it.
async fn upsert_agent_box_cannot_write_quota<S: WriteStore>(store: &S) {
    let probed = AgentBox {
        agent_id: ids::AGENT_CLAUDE,
        box_id: ids::BOX,
        enabled: true,
        version: Some("1.2.3".to_owned()),
        path: Some("/usr/bin/claude".to_owned()),
        probed_at: Some(Utc::now()),
        // The insert path: a probe seeding an allowance it never observed. Accepted, not written.
        quota: Some(json!({ "invented": "by the probe" })),
        quota_at: Some(Utc::now()),
        updated_at: Utc::now(),
        probe: Some(json!({ "status": "ready", "source": "probe" })),
    };
    store.upsert_agent_box(&probed).await.expect(
        "upsert_agent_box_cannot_write_quota: an `AgentBox` carrying a quota is not an error - the \
         column is simply not in the INSERT list",
    );

    // The latch is the only writer, and it finds the row the upsert inserted.
    store
        .set_agent_box_quota(
            ids::AGENT_CLAUDE,
            ids::BOX,
            json!({ "source": "acp_meta_rate_limit", "spend": { "session_micros": 351 } }),
            Utc::now(),
        )
        .await
        .expect("upsert_agent_box_cannot_write_quota: the latch lands");

    // The conflict path, twice: a re-probe carrying a *different* document, then one carrying
    // `None`. Neither is a write, so neither can discard the latch above.
    store
        .upsert_agent_box(&AgentBox {
            version: Some("1.3.0".to_owned()),
            quota: Some(json!({ "stale": "read before the latch" })),
            quota_at: None,
            ..probed.clone()
        })
        .await
        .expect(
            "upsert_agent_box_cannot_write_quota: a re-probe carrying a stale quota is accepted \
             and ignored, not refused",
        );
    store
        .upsert_agent_box(&AgentBox {
            quota: None,
            quota_at: None,
            ..probed.clone()
        })
        .await
        .expect(
            "upsert_agent_box_cannot_write_quota: nor does a `None` quota clear the column the \
             way a `None` probe clears its own",
        );

    // The row is still one row under one key, and still latchable: the upserts updated it rather
    // than replacing or removing it.
    store
        .set_agent_box_quota(
            ids::AGENT_CLAUDE,
            ids::BOX,
            json!({ "source": "none" }),
            Utc::now(),
        )
        .await
        .expect(
            "upsert_agent_box_cannot_write_quota: the latch still finds the row after two upserts",
        );
}

/// `set_step_prompt` writes `run_step.prompt_digest` and `run_step.trim_record`, both, on an
/// existing step and answers `NotFound` on one that does not exist (`docs/ANA-5.md` §4.4).
///
/// The write is observed through [`ReadStore::runs`], because §6.1 returns neither column: the two
/// fields plan D106 put on `RunStepSummary` — `prompt_tokens` and `trimmed` — are the read seam's
/// whole trace of the record, so this case pins the projection and the write at once. That the
/// **only** columns written are those two is asserted per backend, where the columns can be read
/// back: `mem.rs::set_step_prompt_writes_both_columns` here, and
/// `pg_criteria.rs::set_step_prompt_writes_only_the_digest_and_the_record` for Postgres, which
/// diffs the whole row as `jsonb` either side of the write. The names are checked by
/// `every_cross_referenced_test_name_exists` in this module's `tests`.
///
/// The second write is what makes it an overwrite rather than an append: a re-run of the same step
/// assembles a new prompt, and a record that kept the first `trimmed` would report a trim that no
/// longer happened.
async fn set_step_prompt_writes_digest_and_trim<S: WriteStore>(store: &S) {
    let tokens_of = |runs: &[crate::model::RunSummary], step: StepId| {
        runs.iter()
            .flat_map(|run| run.steps.iter())
            .find(|row| row.id == step)
            .map(|row| (row.prompt_tokens, row.trimmed))
    };

    store
        .set_step_prompt(
            ids::STEP_IMPL,
            "9f8e",
            &json!({
                "estimated_after": 34_000,
                "sections": [
                    { "name": "template", "trimmed": false },
                    { "name": "excerpts", "trimmed": true },
                ],
                "v": 1,
            }),
        )
        .await
        .expect("set_step_prompt_writes_digest_and_trim: the first write must land");

    let after_first = store
        .runs(ids::HTUI_FEAT_1)
        .await
        .expect("set_step_prompt_writes_digest_and_trim: the read must not fail");
    assert_eq!(
        tokens_of(&after_first, ids::STEP_IMPL),
        Some((Some(34_000), true)),
        "set_step_prompt_writes_digest_and_trim: estimated_after, and one trimmed section is enough"
    );
    assert_eq!(
        tokens_of(&after_first, ids::STEP_PLAN),
        Some((None, false)),
        "set_step_prompt_writes_digest_and_trim: a step nobody wrote has no figure"
    );

    store
        .set_step_prompt(
            ids::STEP_IMPL,
            "0a1b",
            &json!({ "estimated_after": 12, "sections": [], "v": 1 }),
        )
        .await
        .expect("set_step_prompt_writes_digest_and_trim: the second write must land");
    assert_eq!(
        tokens_of(
            &store
                .runs(ids::HTUI_FEAT_1)
                .await
                .expect("set_step_prompt_writes_digest_and_trim: the second read must not fail"),
            ids::STEP_IMPL,
        ),
        Some((Some(12), false)),
        "set_step_prompt_writes_digest_and_trim: the second record replaces the first, trim and all"
    );

    // `trim_record` is an untyped JSON column, so "the three projections agree" has to hold for
    // documents no assembler would write as well as for the one it does (T68, F-52). Each of
    // these made one backend disagree with `prompt_summary` before T68: the SQL casts raised, and
    // an error is not a projection — it fails the whole Runs read over one bad row.
    //
    // Every shape carries its **expected** pair rather than sharing one hard-coded `(None, false)`
    // (T68, F-52 review, M1). The old shape of this loop could only hold shapes that project to
    // nothing, and "projects to nothing" is exactly the answer a backend gives by accident: a
    // jsonpath that matches the wrong thing and a jsonpath that matches nothing are told apart only
    // by a shape whose right answer is `true`. The `prompt_summary` equality below is kept as the
    // second assertion, so a shape pins the projection against the reference *and* against a
    // literal, and a wrong `prompt_summary` cannot make a wrong backend look right.
    for (label, record, expected) in [
        (
            "a string where a number belongs",
            json!({ "estimated_after": "34000" }),
            (None, false),
        ),
        (
            "a fractional float",
            json!({ "estimated_after": 35_988.5 }),
            (None, false),
        ),
        // The float whose fractional part is zero, which is not the same shape as the one above:
        // it satisfies every numeric predicate an integer does — `floor(v) == v`, in `i32` range —
        // and is still a float, so `Value::as_i64` rejects it. A backend that tests the *value*
        // rather than the *type* admits it and then has `35988.0` to turn into an integer.
        (
            "an integral float",
            json!({ "estimated_after": 35_988.0_f64 }),
            (None, false),
        ),
        (
            "a number past i32",
            json!({ "estimated_after": 3_000_000_000_i64 }),
            (None, false),
        ),
        (
            "a number past -i32",
            json!({ "estimated_after": -3_000_000_000_i64 }),
            (None, false),
        ),
        (
            "sections as an object",
            json!({ "sections": { "a": { "trimmed": true } } }),
            (None, false),
        ),
        // `sections` as the section itself, and `sections` nested one array too deep. Both are
        // `false`: `Value::as_array` refuses the first outright, and the second's one element is an
        // array, whose `trimmed` is absent. They are here because a *lax* jsonpath `$.sections[*]`
        // auto-wraps a non-array and unwraps a nested one, so both used to read `true` on Postgres
        // alone (T68, F-52 review, H2).
        (
            "sections as the section",
            json!({ "sections": { "trimmed": true } }),
            (None, false),
        ),
        (
            "sections nested one array too deep",
            json!({ "sections": [[{ "trimmed": true }]] }),
            (None, false),
        ),
        // The mixed array, the one shape in this loop whose answer is `true`: a projection that
        // walks the elements finds the trimmed one past the scalar, and one that gives up on the
        // first non-object does not. The three agree today and nothing pinned it.
        (
            "a scalar beside a trimmed section",
            json!({ "sections": [5, { "trimmed": true }] }),
            (None, true),
        ),
        (
            "a trimmed that is a string",
            json!({ "sections": [{ "trimmed": "nope" }] }),
            (None, false),
        ),
        (
            "a trimmed that is the integer 1",
            json!({ "sections": [{ "trimmed": 1 }] }),
            (None, false),
        ),
        // `[true]` is not `true`, and it is the third shape lax auto-unwrapping used to admit:
        // `@.trimmed == true` unwrapped the one-element array before comparing (F-90).
        (
            "a trimmed that is a one-element array",
            json!({ "sections": [{ "trimmed": [true] }] }),
            (None, false),
        ),
        (
            "sections of scalars",
            json!({ "sections": [5, "x"] }),
            (None, false),
        ),
        ("an empty record", json!({}), (None, false)),
    ] {
        store
            .set_step_prompt(ids::STEP_IMPL, "dead", &record)
            .await
            .unwrap_or_else(|error| {
                panic!("set_step_prompt_writes_digest_and_trim: writing {label} must land: {error}")
            });
        let read = store.runs(ids::HTUI_FEAT_1).await.unwrap_or_else(|error| {
            panic!(
                "set_step_prompt_writes_digest_and_trim: {label} must project, not fail the \
                 whole read: {error}"
            )
        });
        assert_eq!(
            tokens_of(&read, ids::STEP_IMPL),
            Some(expected),
            "set_step_prompt_writes_digest_and_trim: {label} projects as {expected:?}"
        );
        assert_eq!(
            tokens_of(&read, ids::STEP_IMPL),
            Some(crate::model::prompt_summary(Some(&record))),
            "set_step_prompt_writes_digest_and_trim: {label} projects as `prompt_summary` does"
        );
    }

    let unknown = store
        .set_step_prompt(StepId::new(), "9f8e", &json!({}))
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "set_step_prompt_writes_digest_and_trim: an unknown step is NotFound, got {unknown:?}"
    );
}

// ------------------------------------------------------------------------------------------------
// READ_CASES: the read-only half, so the mirror can be a target too (plan D96)
// ------------------------------------------------------------------------------------------------

/// The walk's bound in the fixture's usual scope: the `Platform` workspace, for an `agy` item.
fn platform_prompt_scope() -> PromptScope {
    PromptScope::from_scope(&platform_scope(), ids::PROJECT_AGY)
}

/// The fixture's `document.body` for one id, so a case asserts against the seed rather than
/// against a copy of it that a fixture edit would leave stale.
fn fixture_document_body(id: DocumentId) -> String {
    crate::fixtures::demo_data()
        .documents
        .into_iter()
        .find(|document| document.id == id)
        .expect("the fixture holds that document")
        .body
}

/// `ReadStore::document` answers the row **with its body**, and `None` for an id nothing has.
///
/// `documents()` answers heads, which is what a list needs and what every shipped caller uses; the
/// prompt assembler needs the text, and reading a body through a head would be a second round trip
/// per document (`docs/ANA-5.md` §8).
async fn document_body_round_trip<S: ReadStore>(store: &S) {
    let found = store
        .document(ids::DOC_FEAT_1_PLAN_V2)
        .await
        .expect("document_body_round_trip: the read must not fail")
        .expect("document_body_round_trip: the fixture holds plan v2");
    assert_eq!(
        (found.item_id, found.kind.as_str(), found.version),
        (ids::HTUI_FEAT_1, "plan", 2),
        "document_body_round_trip: the row is the one that was asked for"
    );
    assert_eq!(
        found.body,
        fixture_document_body(ids::DOC_FEAT_1_PLAN_V2),
        "document_body_round_trip: the body is the seeded one, byte for byte"
    );

    assert!(
        store
            .document(DocumentId::new())
            .await
            .expect("document_body_round_trip: an unknown id is not an error")
            .is_none(),
        "document_body_round_trip: an id nothing has is None, not a default row"
    );
}

/// `documents_of_kinds` answers the **latest version of each kind, in the caller's order**, and
/// omits a kind the item has no row for.
///
/// The order is the caller's because `docs/ANA-5.md` §4.7 rule 3 renders documents in the phase's
/// `input_kinds` order and the prompt digest is a function of that order; a store that sorted
/// would make itself the authority on something the step graph owns. An empty `kinds` is the
/// preview's form (plan D103) and orders by kind **bytes**, so Postgres's collation and the
/// mirror's byte order cannot disagree.
async fn documents_of_kinds_latest_per_kind_in_order<S: ReadStore>(store: &S) {
    let shape = |documents: &[crate::model::Document]| {
        documents
            .iter()
            .map(|document| (document.kind.clone(), document.version))
            .collect::<Vec<_>>()
    };
    let prd = ("prd".to_owned(), 1);
    let plan = ("plan".to_owned(), 2);

    let asked = store
        .documents_of_kinds(ids::HTUI_FEAT_1, &["prd".to_owned(), "plan".to_owned()])
        .await
        .expect("documents_of_kinds_latest_per_kind_in_order: the read must not fail");
    assert_eq!(
        shape(&asked),
        vec![prd.clone(), plan.clone()],
        "documents_of_kinds_latest_per_kind_in_order: kinds order, and plan v1 loses to v2"
    );

    let reversed = store
        .documents_of_kinds(
            ids::HTUI_FEAT_1,
            &["plan".to_owned(), "prd".to_owned(), "missing".to_owned()],
        )
        .await
        .expect("documents_of_kinds_latest_per_kind_in_order: the second read must not fail");
    assert_eq!(
        shape(&reversed),
        vec![plan.clone(), prd.clone()],
        "documents_of_kinds_latest_per_kind_in_order: the caller's order, and an absent kind is \
         omitted rather than an error"
    );

    let all = store
        .documents_of_kinds(ids::HTUI_FEAT_1, &[])
        .await
        .expect("documents_of_kinds_latest_per_kind_in_order: the third read must not fail");
    assert_eq!(
        shape(&all),
        vec![plan, prd],
        "documents_of_kinds_latest_per_kind_in_order: an empty `kinds` is every kind in byte order"
    );

    assert!(
        store
            .documents_of_kinds(ids::HTUI_FEAT_3, &["plan".to_owned()])
            .await
            .expect("documents_of_kinds_latest_per_kind_in_order: the fourth read must not fail")
            .is_empty(),
        "documents_of_kinds_latest_per_kind_in_order: an item with no documents answers none"
    );

    // The two empties together, which is the combination that has to be written down: a store that
    // resolves "every kind" by listing the item's kinds and re-entering itself has an item with no
    // documents as its non-terminating case, and `MemStore` did — the read overflowed the stack
    // rather than answering. It is a normal call: the preview's form of this read passes no kinds
    // (plan D103) and an item may have no documents yet.
    assert!(
        store
            .documents_of_kinds(ids::HTUI_FEAT_3, &[])
            .await
            .expect("documents_of_kinds_latest_per_kind_in_order: the fifth read must not fail")
            .is_empty(),
        "documents_of_kinds_latest_per_kind_in_order: an empty `kinds` on an item with no \
         documents is an empty answer, not a recursion"
    );
}

/// `ReadStore::project` answers the row with its `settings` document, and `None` for an unknown
/// id.
///
/// The settings column is where the prompt's `token_budget` rung lives (`docs/ANA-5.md` §4.4), and
/// it is mirrored, which is why this is a trait method rather than an inherent read beside
/// `project_settings` — that one returns the column alone and `run_read_case` could not call it.
async fn project_row_has_settings<S: ReadStore>(store: &S) {
    let project = store
        .project(ids::PROJECT_HTUI)
        .await
        .expect("project_row_has_settings: the read must not fail")
        .expect("project_row_has_settings: the fixture holds `htui`");
    assert_eq!(
        project.slug, "htui",
        "project_row_has_settings: the row is the one that was asked for"
    );
    assert_eq!(
        project.settings.get("token_budget").and_then(Value::as_i64),
        Some(120_000),
        "project_row_has_settings: the settings document arrives whole, cap and all"
    );

    assert!(
        store
            .project(ProjectId::new())
            .await
            .expect("project_row_has_settings: an unknown id is not an error")
            .is_none(),
        "project_row_has_settings: an id nothing has is None"
    );
}

/// The diamond of the fixture's §4.3 walk: `htui:ANA-1` is reached at depth 2 down **both** arms
/// and must be rendered **once**, at that depth, with its summary.
///
/// The shipped `UNION` of `docs/ANA-9.md` §7.3 dedups whole rows, that is `(item_id, depth)`
/// pairs, so a diamond emitted the same item twice and the prompt carried its summary twice — a
/// digest that depended on edge insertion order. `MIN(depth)` is the amendment, and this is what
/// asserts it on every backend.
///
/// The whole vector is compared against a canonically re-sorted copy of itself, because the order
/// is the contract: Postgres orders `qualified_key` by the database collation and the SQLite
/// mirror by byte value, so a backend that forgot the Rust re-sort would still pass every
/// per-entry assertion here.
async fn upstream_diamond_dedup<S: ReadStore>(store: &S) {
    let scope = platform_prompt_scope();
    let two = store
        .upstream_summaries(ids::AGY_FIX_1, 2, &scope)
        .await
        .expect("upstream_diamond_dedup: the walk must not fail");

    assert_eq!(
        two.iter()
            .map(|entry| (entry.qualified_key.as_str(), entry.depth))
            .collect::<Vec<_>>(),
        vec![
            ("agy:ANA-1", 1),
            ("htui:TOOL-1", 1),
            ("vulkan-tutorials:FEAT-1", 1),
            ("htui:ANA-1", 2),
        ],
        "upstream_diamond_dedup: canonical order is depth, then qualified_key bytes, then id"
    );

    let apex: Vec<&UpstreamEntry> = two
        .iter()
        .filter(|entry| entry.item_id == ids::HTUI_ANA_1)
        .collect();
    assert_eq!(
        apex.len(),
        1,
        "upstream_diamond_dedup: two paths reach ANA-1 and it is rendered once, got {apex:?}"
    );
    assert_eq!(
        (apex[0].depth, apex[0].in_scope, apex[0].summary.is_some()),
        (2, true, true),
        "upstream_diamond_dedup: at its minimum depth, in scope, with the summary document"
    );
    assert_eq!(
        apex[0].summary.as_deref(),
        Some(fixture_document_body(ids::DOC_ANA_1_SUMMARY).as_str()),
        "upstream_diamond_dedup: the summary is the document body, verbatim"
    );

    let mut canonical = two.clone();
    UpstreamEntry::sort_canonical(&mut canonical);
    assert_eq!(
        two, canonical,
        "upstream_diamond_dedup: the walk returns canonical order, not the backend's ORDER BY"
    );

    let one = store
        .upstream_summaries(ids::AGY_FIX_1, 1, &scope)
        .await
        .expect("upstream_diamond_dedup: the one-hop walk must not fail");
    assert!(
        !one.iter().any(|entry| entry.item_id == ids::HTUI_ANA_1),
        "upstream_diamond_dedup: ANA-1 is two hops out and `hops == 1` does not reach it"
    );
    assert!(
        one.iter().all(|entry| entry.depth == 1),
        "upstream_diamond_dedup: `hops == 1` is the first ring and nothing else, got {one:?}"
    );

    let none = store
        .upstream_summaries(ids::AGY_FIX_1, 0, &scope)
        .await
        .expect("upstream_diamond_dedup: a zero-hop walk is not an error");
    assert!(
        none.is_empty(),
        "upstream_diamond_dedup: `hops == 0` is no upstream at all — the root is the step's own \
         item and is never an entry, unlike `links(id, 0)`"
    );

    // Directed and kind-filtered, both in one assertion: `agy:FEAT-1` is one undirected hop from
    // `agy:FIX-1`'s neighbourhood through the `relates` edge `AGY_FEAT_1 -> HTUI_FEAT_2`, and
    // `htui:FEAT-1` is reachable from `htui:ANA-1` only by following an `origin` edge backwards.
    let reached: Vec<&str> = two
        .iter()
        .map(|entry| entry.qualified_key.as_str())
        .collect();
    assert!(
        !reached.contains(&"htui:FEAT-1") && !reached.contains(&"agy:FEAT-1"),
        "upstream_diamond_dedup: the walk follows `to_item_id` only, and only `blocked_by` and \
         `origin`, got {reached:?}"
    );
}

/// `docs/ANA-5.md` §4.3's third render state: in scope, and nobody has written a summary yet.
///
/// It is the common case rather than the edge case — `document.kind = 'summary'` is written at
/// close-out and `done` is not `closed` — and it is a different fact from the `R-PRM-2` stub: this
/// one names an item the agent can go and read documents about.
async fn upstream_in_scope_no_summary<S: ReadStore>(store: &S) {
    let entries = store
        .upstream_summaries(ids::AGY_FIX_1, 2, &platform_prompt_scope())
        .await
        .expect("upstream_in_scope_no_summary: the walk must not fail");

    for (item, key, status) in [
        (ids::AGY_ANA_1, "agy:ANA-1", Status::Done),
        (ids::HTUI_TOOL_1, "htui:TOOL-1", Status::AwaitingApproval),
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry.item_id == item)
            .unwrap_or_else(|| panic!("upstream_in_scope_no_summary: {key} is one hop out"));
        assert_eq!(
            (
                entry.qualified_key.as_str(),
                entry.depth,
                entry.in_scope,
                entry.summary.is_none(),
                entry.status,
            ),
            (key, 1, true, true, status),
            "upstream_in_scope_no_summary: {key} renders as `no summary yet`"
        );
        assert!(
            entry.is_pending() && !entry.is_summary(),
            "upstream_in_scope_no_summary: {key} is Pending, not Summary"
        );
    }
}

/// `R-PRM-2`'s stub, and the separability the amended query exists for.
///
/// `vulkan-tutorials:FEAT-1` is in the `Graphics` workspace, so under the `Platform` scope it is
/// out of scope. And under a **project-only** bound (`R-ENT-2`, no workspace), `htui:ANA-1` is out
/// of scope with `summary == None` even though its summary document exists and the same walk
/// returned it a moment ago: `in_scope` and `summary.is_some()` are two facts, and the shipped
/// query conflated them into one `NULL`.
async fn upstream_out_of_scope_stub<S: ReadStore>(store: &S) {
    let entries = store
        .upstream_summaries(ids::AGY_FIX_1, 2, &platform_prompt_scope())
        .await
        .expect("upstream_out_of_scope_stub: the walk must not fail");
    let stub = entries
        .iter()
        .find(|entry| entry.item_id == ids::VULKAN_FEAT_1)
        .expect("upstream_out_of_scope_stub: the cross-workspace origin is still reached");
    assert_eq!(
        (
            stub.qualified_key.as_str(),
            stub.depth,
            stub.in_scope,
            stub.summary.as_deref(),
        ),
        ("vulkan-tutorials:FEAT-1", 1, false, None),
        "upstream_out_of_scope_stub: an item outside the workspace is a stub, summary unread"
    );
    assert!(
        !stub.is_summary() && !stub.is_pending(),
        "upstream_out_of_scope_stub: a stub is neither of the two in-scope states"
    );

    let project_only = store
        .upstream_summaries(
            ids::AGY_FIX_1,
            2,
            &PromptScope::project_only(ids::PROJECT_AGY),
        )
        .await
        .expect("upstream_out_of_scope_stub: the project-only walk must not fail");
    assert_eq!(
        project_only
            .iter()
            .map(|entry| (entry.qualified_key.as_str(), entry.in_scope))
            .collect::<Vec<_>>(),
        vec![
            ("agy:ANA-1", true),
            ("htui:TOOL-1", false),
            ("vulkan-tutorials:FEAT-1", false),
            ("htui:ANA-1", false),
        ],
        "upstream_out_of_scope_stub: with no workspace the bound is the one project"
    );
    assert_eq!(
        project_only
            .iter()
            .find(|entry| entry.item_id == ids::HTUI_ANA_1)
            .and_then(|entry| entry.summary.as_deref()),
        None,
        "upstream_out_of_scope_stub: an out-of-scope item's summary is not read, though it exists"
    );
}

#[cfg(test)]
mod tests {
    use super::{CASES, READ_CASES, run_case, run_read_case};
    use crate::store::MemStore;

    #[test]
    fn case_names_are_unique() {
        let mut sorted = CASES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");

        // The two lists are reported separately by MOD-6's binding, so a name may not be in both
        // either: a duplicate across them would be run twice and reported once.
        let mut both: Vec<&str> = CASES.iter().chain(READ_CASES).copied().collect();
        both.sort_unstable();
        both.dedup();
        assert_eq!(
            both.len(),
            CASES.len() + READ_CASES.len(),
            "a case name is unique across both lists"
        );
    }

    /// Every test this module's doc comments cross-refer to exists under the name they give it.
    ///
    /// The suite delegates what §6.1 cannot read back - `run_step.usage`, `run_step.prompt_digest`,
    /// the `agent` row - to *named* per-backend tests, so a name that never existed (or has since
    /// been renamed) turns the delegation into a dead end that reads like coverage:
    /// `set_step_usage_writes_usage_and_digest` shipped naming a `store::mem` test that was never
    /// written. Two shapes of reference are checked, both taken from inline-code spans: a bare
    /// snake_case name of four or more underscores (a test in this crate) and `<file>.rs::<name>`
    /// (a test in the named file). A file the table below does not know is a failure, not a skip:
    /// an unchecked reference is exactly the hole this test exists to close. Spans that are not
    /// plain snake_case are skipped, which is what keeps this test's own message templates -
    /// compiled into the source it scans - from being read as references.
    #[test]
    fn every_cross_referenced_test_name_exists() {
        /// This module's own source: the text that was compiled, doc comments included.
        const SELF: &str = include_str!("conformance.rs");
        /// The sibling `MemStore` unit tests.
        const MEM: &str = include_str!("mem.rs");

        // Another crate's integration test binary, so it is read at run time rather than through
        // `include_str!`: `htui-core` must not take a compile-time dependency on `htui-store`.
        let pg_criteria = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../htui-store/tests/pg_criteria.rs"),
        )
        .expect("read crates/htui-store/tests/pg_criteria.rs");

        // `(` for a plain fn, `<` for a generic one: every case in this module is `fn name<S: ..>`.
        let defines = |source: &str, name: &str| {
            source.contains(&format!("fn {name}(")) || source.contains(&format!("fn {name}<"))
        };
        let snake_case = |token: &str| {
            token.starts_with(|c: char| c.is_ascii_lowercase())
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };

        let mut checked = 0_usize;
        // Inline-code spans, line by line: a span never wraps a line in this file, and taking the
        // odd fields of a line-local split keeps an unbalanced backtick from shifting every later
        // token by one.
        for token in SELF
            .lines()
            .flat_map(|line| line.split('`').skip(1).step_by(2))
        {
            if let Some((file, name)) = token.split_once(".rs::") {
                if !snake_case(file) || !snake_case(name) {
                    continue;
                }
                let source = match file {
                    "conformance" => SELF,
                    "mem" => MEM,
                    "pg_criteria" => pg_criteria.as_str(),
                    other => panic!(
                        "`{other}.rs::{name}` points at a file this test cannot read: add it to \
                         the table in `every_cross_referenced_test_name_exists`"
                    ),
                };
                assert!(
                    defines(source, name),
                    "a doc comment names `{file}.rs::{name}`, but `{file}.rs` defines no such fn"
                );
                checked += 1;
            } else if snake_case(token) && token.matches('_').count() >= 4 {
                assert!(
                    defines(SELF, token) || defines(MEM, token),
                    "a doc comment names the test `{token}`, but neither `conformance.rs` nor \
                     `mem.rs` defines a fn by that name"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 4,
            "the scanner found only {checked} cross-references: the doc comments it reads have \
             been reshaped and it is no longer checking them"
        );
    }

    #[tokio::test]
    async fn run_case_accepts_every_name_in_cases() {
        // `run_case` panics on a name it does not know, which is the whole point of its `match`:
        // running the list through it is what keeps `CASES` and the dispatcher in step.
        for name in CASES {
            run_case(name, &MemStore::demo()).await;
        }
    }

    /// The same guard for the read half (plan D96): `run_read_case`'s `match` is the only thing
    /// that can disagree with [`READ_CASES`], and a name in the list with no arm is a case that
    /// silently never ran on the mirror.
    #[tokio::test]
    async fn run_read_case_accepts_every_name_in_read_cases() {
        // One store for all six: every read case writes nothing, which is what makes the list
        // runnable against a `CacheStore` at all.
        let store = MemStore::demo();
        for name in READ_CASES {
            run_read_case(name, &store).await;
        }
    }
}
