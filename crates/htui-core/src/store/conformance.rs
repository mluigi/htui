//! Store conformance suite (feature `test-support`), blueprint B.9.
//!
//! Every rule of `docs/ANA-9.md` §4.1, §4.2 and §7 a store has to show, written against
//! [`WriteStore`] alone: no concrete store is named anywhere in this module, so MOD-6 runs the
//! same cases against `PgStore` and reports per case. The cases assert against the demo fixture
//! (`test-support` implies `demo`), which is why the two features are not independent: MOD-6
//! inherits a seed data set instead of inventing a second one.

use core::future::Future;

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::fixtures::ids;
use crate::model::{
    Agent, AgentBox, AgentId, Billing, BoxId, BoxProbe, BoxRow, ChatRunSpec, CitationKind, Claim,
    CommandQueue, CommandRun, CommandRunId, CommandRunStatus, CoverageRow,
    DEFAULT_MAX_CONCURRENT_ITEMS, DocumentId, EventKind, EventRole, Gate, GateOutcome,
    GraphSnapshot, Isolation, Item, ItemCitation, ItemFilter, ItemId, ItemKindId, ItemKindPatch,
    ItemPatch, ItemSummary, LinkKind, NewCommandRun, NewDocument, NewItem, NewItemKind, NewNote,
    NewProject, NewRepo, NewRequirement, NewRequirementArea, NewRun, NewRunStep, NewStepGraph,
    NewWorkspace, NoteId, OverlapRule, PhaseId, PhasePatch, Priority, ProbedTool, ProjectId,
    ProjectPatch, PromptScope, RepoBoxPath, RepoId, RepoPatch, RepoScope, Requirement,
    RequirementAreaId, RequirementFilter, RequirementId, RequirementPatch, RequirementRevision,
    RequirementState, RequirementUpdate, Resolution, Run, RunId, RunKind, RunMode, RunScope,
    RunStatus, RunStep, RunStepCommit, RunStepTree, Scope, SessionEvent, SnapshotGraph,
    SnapshotSettings, Status, StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOutcome,
    StepStatus, TIMESTAMPTZ_DIGITS, Transport, UpstreamEntry, UserId, VerifyOutcome,
    WorkspaceBoxPath, WorkspaceId, WorkspacePatch, WorkspaceProject,
};
use crate::prompt::TemplateRole;
use crate::prompt::settings::SettingKey;
use crate::store::error::StoreError;
use crate::store::traits::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, SettingRung, UpdateOutcome, WriteStore,
    citation_key, illegal_move, invalid_area_code, requirement_withdrawn, resolution_not_closable,
    withdrawn_requirement_cited,
};

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
    "workspace_round_trip_and_cas",
    "workspace_links_and_box_paths_upsert",
    "workspace_delete_reports_its_reach",
    "project_create_update_cas",
    "project_delete_takes_everything_and_says_so",
    "repo_round_trip_and_primary_flag",
    "item_kind_round_trip_and_prefix_rules",
    "item_kind_delete_refused_while_referenced",
    "step_graph_and_phase_round_trip",
    "settings_app_rung_validates_and_cas",
    "settings_project_rung_merges_keys",
    "settings_phase_rung_writes_token_budget_only",
    "project_create_seeds_the_catalogue",
    "run_create_moves_the_item",
    "claim_run_admits_one_and_refuses_the_second",
    "lease_refresh_is_a_cas_on_owner",
    "step_create_and_transition_law",
    "finish_step_records_the_settle",
    "gate_answers_write_their_outcome",
    "select_fanout_is_one_transaction",
    "trees_and_commits_round_trip",
    "verify_run_is_recorded",
    "write_document_allocates_its_version",
    "close_out_refuses_a_live_run",
    "illegal_transitions_are_constraint",
    "finish_run_moves_run_and_item_together",
    "claim_run_applies_the_isolation_and_path_rules",
    "take_lease_moves_only_our_own_or_an_expired_lease",
    "interrupt_step_is_a_cas_on_running",
    "release_lease_frees_the_run_for_its_own_sweep",
    "record_box_probe_replaces_profile_and_tools",
    "record_box_probe_refuses_an_unknown_box",
    "boxes_lists_every_box_with_its_tools",
    "close_out_resolution_law",
    "transition_never_reaches_closed",
    "requirement_mint_is_per_area_and_never_reused",
    "requirement_area_create_checks_the_code",
    "requirement_amend_is_cas_and_names_the_item",
    "requirement_withdraw_refuses_new_addresses",
    "amend_records_the_deciding_citation",
    "a_newer_version_makes_a_citation_suspect_until_reconfirmed",
    "uncite_tombstones_and_cite_revives",
    "coverage_lists_citing_items_with_resolution",
    "spec_is_cas",
    "project_delete_counts_requirements",
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
        "workspace_round_trip_and_cas" => workspace_round_trip_and_cas(store).await,
        "workspace_links_and_box_paths_upsert" => {
            workspace_links_and_box_paths_upsert(store).await;
        }
        "workspace_delete_reports_its_reach" => workspace_delete_reports_its_reach(store).await,
        "project_create_update_cas" => project_create_update_cas(store).await,
        "project_delete_takes_everything_and_says_so" => {
            project_delete_takes_everything_and_says_so(store).await;
        }
        "repo_round_trip_and_primary_flag" => repo_round_trip_and_primary_flag(store).await,
        "item_kind_round_trip_and_prefix_rules" => {
            item_kind_round_trip_and_prefix_rules(store).await;
        }
        "item_kind_delete_refused_while_referenced" => {
            item_kind_delete_refused_while_referenced(store).await;
        }
        "step_graph_and_phase_round_trip" => step_graph_and_phase_round_trip(store).await,
        "settings_app_rung_validates_and_cas" => settings_app_rung_validates_and_cas(store).await,
        "settings_project_rung_merges_keys" => settings_project_rung_merges_keys(store).await,
        "settings_phase_rung_writes_token_budget_only" => {
            settings_phase_rung_writes_token_budget_only(store).await;
        }
        "project_create_seeds_the_catalogue" => project_create_seeds_the_catalogue(store).await,
        "run_create_moves_the_item" => run_create_moves_the_item(store).await,
        "claim_run_admits_one_and_refuses_the_second" => {
            claim_run_admits_one_and_refuses_the_second(store).await;
        }
        "lease_refresh_is_a_cas_on_owner" => lease_refresh_is_a_cas_on_owner(store).await,
        "step_create_and_transition_law" => step_create_and_transition_law(store).await,
        "finish_step_records_the_settle" => finish_step_records_the_settle(store).await,
        "gate_answers_write_their_outcome" => gate_answers_write_their_outcome(store).await,
        "select_fanout_is_one_transaction" => select_fanout_is_one_transaction(store).await,
        "trees_and_commits_round_trip" => trees_and_commits_round_trip(store).await,
        "verify_run_is_recorded" => verify_run_is_recorded(store).await,
        "write_document_allocates_its_version" => write_document_allocates_its_version(store).await,
        "close_out_refuses_a_live_run" => close_out_refuses_a_live_run(store).await,
        "illegal_transitions_are_constraint" => illegal_transitions_are_constraint(store).await,
        "finish_run_moves_run_and_item_together" => {
            finish_run_moves_run_and_item_together(store).await;
        }
        "claim_run_applies_the_isolation_and_path_rules" => {
            claim_run_applies_the_isolation_and_path_rules(store).await;
        }
        "take_lease_moves_only_our_own_or_an_expired_lease" => {
            take_lease_moves_only_our_own_or_an_expired_lease(store).await;
        }
        "interrupt_step_is_a_cas_on_running" => interrupt_step_is_a_cas_on_running(store).await,
        "release_lease_frees_the_run_for_its_own_sweep" => {
            release_lease_frees_the_run_for_its_own_sweep(store).await;
        }
        "record_box_probe_replaces_profile_and_tools" => {
            record_box_probe_replaces_profile_and_tools(store).await;
        }
        "record_box_probe_refuses_an_unknown_box" => {
            record_box_probe_refuses_an_unknown_box(store).await;
        }
        "boxes_lists_every_box_with_its_tools" => boxes_lists_every_box_with_its_tools(store).await,
        "close_out_resolution_law" => close_out_resolution_law(store).await,
        "transition_never_reaches_closed" => transition_never_reaches_closed(store).await,
        "requirement_mint_is_per_area_and_never_reused" => {
            requirement_mint_is_per_area_and_never_reused(store).await;
        }
        "requirement_area_create_checks_the_code" => {
            requirement_area_create_checks_the_code(store).await;
        }
        "requirement_amend_is_cas_and_names_the_item" => {
            requirement_amend_is_cas_and_names_the_item(store).await;
        }
        "requirement_withdraw_refuses_new_addresses" => {
            requirement_withdraw_refuses_new_addresses(store).await;
        }
        "amend_records_the_deciding_citation" => amend_records_the_deciding_citation(store).await,
        "a_newer_version_makes_a_citation_suspect_until_reconfirmed" => {
            a_newer_version_makes_a_citation_suspect_until_reconfirmed(store).await;
        }
        "uncite_tombstones_and_cite_revives" => uncite_tombstones_and_cite_revives(store).await,
        "coverage_lists_citing_items_with_resolution" => {
            coverage_lists_citing_items_with_resolution(store).await;
        }
        "spec_is_cas" => spec_is_cas(store).await,
        "project_delete_counts_requirements" => project_delete_counts_requirements(store).await,
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
    "run_and_steps_round_trip",
    "trees_and_commits_read_back",
    "resolve_inputs_prefers_this_run_and_skips_losers",
    "requirement_spec_and_areas_read_back",
    "requirements_filter_by_area_state_priority_and_text",
    "item_citations_derive_suspect",
    "coverage_carries_status_and_resolution",
    "requirement_revisions_or_not_cached",
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
        "run_and_steps_round_trip" => run_and_steps_round_trip(store).await,
        "trees_and_commits_read_back" => trees_and_commits_read_back(store).await,
        "resolve_inputs_prefers_this_run_and_skips_losers" => {
            resolve_inputs_prefers_this_run_and_skips_losers(store).await;
        }
        "requirement_spec_and_areas_read_back" => requirement_spec_and_areas_read_back(store).await,
        "requirements_filter_by_area_state_priority_and_text" => {
            requirements_filter_by_area_state_priority_and_text(store).await;
        }
        "item_citations_derive_suspect" => item_citations_derive_suspect(store).await,
        "coverage_carries_status_and_resolution" => {
            coverage_carries_status_and_resolution(store).await;
        }
        "requirement_revisions_or_not_cached" => requirement_revisions_or_not_cached(store).await,
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

    // Move the row on, so the next call's `from` is stale while its `(from, to)` pair is still a
    // sanctioned one (ANA-2 §4.3). The staleness is what this leg tests, so the pair must be
    // legal or the §4.3 guard would pre-empt the compare-and-set with `Constraint` (plan D4).
    assert!(
        store
            .transition(before.id, Status::Queued, Status::InProgress)
            .await
            .expect("status_cas_keeps_version: transition must not fail"),
        "status_cas_keeps_version: queued -> in_progress matches"
    );
    let stale = store
        .transition(before.id, Status::Queued, Status::InProgress)
        .await
        .expect("status_cas_keeps_version: transition must not fail");
    assert!(
        !stale,
        "status_cas_keeps_version: a stale `from` is refused even on a legal pair"
    );
    let unmoved = store
        .item(before.id)
        .await
        .expect("status_cas_keeps_version: read must not fail")
        .expect("status_cas_keeps_version: the item still exists");
    assert_eq!(
        unmoved.status,
        Status::InProgress,
        "status_cas_keeps_version: a refused move is a no-op"
    );

    // `closed_at` tracks the current status, not the history: a move to a terminal status sets
    // it, a move back to a live one clears it (§4.2; blueprint Errata).
    assert!(
        store
            .transition(before.id, Status::InProgress, Status::Done)
            .await
            .expect("status_cas_keeps_version: transition must not fail"),
        "status_cas_keeps_version: in_progress -> done matches"
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
        "status_cas_keeps_version: no move bumped the version"
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
    // Close-out is the only way into `closed` (MOD-38 PRD D1), and an `open` item closes as one
    // of the four non-success resolutions (ANA-11 §4.2), so the item closes straight from `open`.
    store
        .close_out(
            ids::HTUI_ANA_2,
            Resolution::Withdrawn,
            new_document(ids::HTUI_ANA_2, "summary", None),
            &[],
        )
        .await
        .expect("no_delete_path: open closes out as withdrawn");

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
// MOD-15 milestone 1: the hierarchy, one case per entity group (plan D12)
// ------------------------------------------------------------------------------------------------

/// The row a compare-and-set applied, or a panic naming the case.
///
/// `Stale` is a legitimate answer of every edit, so unwrapping it inline in twelve cases would
/// spell the same four lines twelve times and lose the case name from the message.
fn applied<T: core::fmt::Debug>(case: &str, outcome: CasOutcome<T>) -> T {
    match outcome {
        CasOutcome::Applied(row) => row,
        CasOutcome::Stale(row) => panic!("{case}: expected Applied, got Stale({row:?})"),
    }
}

/// The row a spent token found, or a panic naming the case. The mirror of [`applied`].
fn stale<T: core::fmt::Debug>(case: &str, outcome: CasOutcome<T>) -> T {
    match outcome {
        CasOutcome::Stale(row) => row,
        CasOutcome::Applied(row) => panic!("{case}: expected Stale, got Applied({row:?})"),
    }
}

/// A workspace request with a fresh id, authored by the fixture user.
fn new_workspace(slug: &str) -> NewWorkspace {
    NewWorkspace {
        id: WorkspaceId::new(),
        slug: slug.to_owned(),
        name: slug.to_uppercase(),
        description: String::new(),
        created_by: ids::USER,
    }
}

/// A project request with a fresh id, authored by the fixture user.
fn new_project(slug: &str) -> NewProject {
    NewProject {
        id: ProjectId::new(),
        slug: slug.to_owned(),
        name: slug.to_uppercase(),
        description: String::new(),
        created_by: ids::USER,
    }
}

/// A repo request with a fresh id, in the project the caller names.
fn new_repo(project: ProjectId, name: &str, is_primary: bool) -> NewRepo {
    NewRepo {
        id: RepoId::new(),
        project_id: project,
        name: name.to_owned(),
        remote_url: Some(format!("git@example.invalid:{name}.git")),
        default_branch: "main".to_owned(),
        is_primary,
    }
}

/// A kind request with a fresh id; `graph` is what the D11 cross-project rule is checked against.
fn new_item_kind(project: ProjectId, prefix: &str, name: &str, graph: StepGraphId) -> NewItemKind {
    NewItemKind {
        id: ItemKindId::new(),
        project_id: project,
        prefix: prefix.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        default_graph_id: graph,
        position: 9,
    }
}

/// A whole phase row, the shape [`WriteStore::create_phase`] takes (D10). `updated_at` is the
/// caller's only because the struct has the column; the store's clock overwrites it.
fn new_phase(graph: StepGraphId, position: i32, name: &str) -> StepGraphPhase {
    StepGraphPhase {
        id: PhaseId::new(),
        graph_id: graph,
        position,
        name: name.to_owned(),
        fan_out: 1,
        gate: Gate::Always,
        gate_hard: false,
        retry_limit: 1,
        input_kinds: Vec::new(),
        output_kind: name.to_owned(),
        isolation: None,
        command_queue: CommandQueue::FanOutOnly,
        verify_command: None,
        template_name: name.to_owned(),
        template_version: None,
        token_budget: None,
        updated_at: Utc::now(),
    }
}

/// `project.settings` as an object, or a panic: every fixture project seeds a JSON object and the
/// key-level merge of D7 is only defined over one.
fn settings_map(case: &str, settings: &Value) -> serde_json::Map<String, Value> {
    settings
        .as_object()
        .unwrap_or_else(|| panic!("{case}: project.settings is a JSON object, got {settings}"))
        .clone()
}

/// D3 on the smallest table: a create, a slug collision, a read-back, one `Applied` and one
/// `Stale` carrying the current row. `updated_at` is advanced by the store, never by the case.
async fn workspace_round_trip_and_cas<S: WriteStore>(store: &S) {
    const CASE: &str = "workspace_round_trip_and_cas";
    let created = store
        .create_workspace(new_workspace("ops"))
        .await
        .expect(CASE);
    assert_eq!(created.slug, "ops", "{CASE}: the request's slug is stored");

    let duplicate = store.create_workspace(new_workspace("ops")).await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate slug is Constraint, got {duplicate:?}"
    );

    assert_eq!(
        store.workspace(created.id).await.expect(CASE).as_ref(),
        Some(&created),
        "{CASE}: the read-back is the row the create returned"
    );
    assert!(
        store
            .workspace(WorkspaceId::new())
            .await
            .expect(CASE)
            .is_none(),
        "{CASE}: an id nothing has is None, not a default row"
    );

    let patch = WorkspacePatch {
        name: Some("Operations".to_owned()),
        ..WorkspacePatch::default()
    };
    let edited = applied(
        CASE,
        store
            .update_workspace(created.id, created.updated_at, patch.clone())
            .await
            .expect(CASE),
    );
    assert_eq!(edited.name, "Operations", "{CASE}: the patch applied");
    assert!(
        edited.updated_at > created.updated_at,
        "{CASE}: the store's clock advances the token, the caller never does"
    );

    let current = stale(
        CASE,
        store
            .update_workspace(created.id, created.updated_at, patch)
            .await
            .expect(CASE),
    );
    assert_eq!(
        current, edited,
        "{CASE}: Stale carries the row as it is now, so the editor can reload"
    );

    let unknown = store
        .update_workspace(
            WorkspaceId::new(),
            edited.updated_at,
            WorkspacePatch::default(),
        )
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "workspace",
                ..
            })
        ),
        "{CASE}: an unknown id is NotFound, not Stale, got {unknown:?}"
    );
}

/// The three tables with no `updated_at` token of their own (D3): a link that repositions, a link
/// that is removed without taking its project, and a per-box path that is replaced in place.
async fn workspace_links_and_box_paths_upsert<S: WriteStore>(store: &S) {
    const CASE: &str = "workspace_links_and_box_paths_upsert";
    let workspace = store
        .create_workspace(new_workspace("links"))
        .await
        .expect(CASE);

    let link = |project: ProjectId, position: i32| WorkspaceProject {
        workspace_id: workspace.id,
        project_id: project,
        position,
    };
    store
        .upsert_workspace_project(&link(ids::PROJECT_HTUI, 5))
        .await
        .expect(CASE);
    store
        .upsert_workspace_project(&link(ids::PROJECT_AGY, 1))
        .await
        .expect(CASE);
    assert_eq!(
        store
            .workspace_projects(workspace.id)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| (row.project_id, row.position))
            .collect::<Vec<_>>(),
        vec![(ids::PROJECT_AGY, 1), (ids::PROJECT_HTUI, 5)],
        "{CASE}: links come back in position order"
    );

    store
        .upsert_workspace_project(&link(ids::PROJECT_HTUI, 0))
        .await
        .expect(CASE);
    assert_eq!(
        store
            .workspace_projects(workspace.id)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| (row.project_id, row.position))
            .collect::<Vec<_>>(),
        vec![(ids::PROJECT_HTUI, 0), (ids::PROJECT_AGY, 1)],
        "{CASE}: the second upsert repositions rather than inserting a second row"
    );

    let unknown_project = store
        .upsert_workspace_project(&link(ProjectId::new(), 2))
        .await;
    assert!(
        matches!(unknown_project, Err(StoreError::Constraint(_))),
        "{CASE}: a link to no project is Constraint, got {unknown_project:?}"
    );

    store
        .remove_workspace_project(workspace.id, ids::PROJECT_HTUI)
        .await
        .expect(CASE);
    assert_eq!(
        store
            .workspace_projects(workspace.id)
            .await
            .expect(CASE)
            .len(),
        1,
        "{CASE}: only the named link goes"
    );
    assert!(
        store
            .project(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .is_some(),
        "{CASE}: removing a link never removes the project (D4)"
    );
    let gone = store
        .remove_workspace_project(workspace.id, ids::PROJECT_HTUI)
        .await;
    assert!(
        matches!(
            gone,
            Err(StoreError::NotFound {
                entity: "workspace_project",
                ..
            })
        ),
        "{CASE}: removing a link twice is NotFound, got {gone:?}"
    );

    let path = |root: &str| WorkspaceBoxPath {
        workspace_id: workspace.id,
        box_id: ids::BOX,
        root_path: root.to_owned(),
        updated_at: Utc::now(),
    };
    store
        .upsert_workspace_box_path(&path("/srv/first"))
        .await
        .expect(CASE);
    store
        .upsert_workspace_box_path(&path("/srv/second"))
        .await
        .expect(CASE);
    let paths = store.workspace_box_paths(workspace.id).await.expect(CASE);
    assert_eq!(
        paths
            .iter()
            .map(|row| (row.box_id, row.root_path.as_str()))
            .collect::<Vec<_>>(),
        vec![(ids::BOX, "/srv/second")],
        "{CASE}: the per-box path is replaced in place, never doubled (R-BOX-4)"
    );
}

/// D4's first half: a workspace delete reaches its links and its box paths and stops there. The
/// reach is counted before the act and the act reports the same struct (PRD D13).
async fn workspace_delete_reports_its_reach<S: WriteStore>(store: &S) {
    const CASE: &str = "workspace_delete_reports_its_reach";
    let workspace = store
        .create_workspace(new_workspace("doomed"))
        .await
        .expect(CASE);
    store
        .upsert_workspace_project(&WorkspaceProject {
            workspace_id: workspace.id,
            project_id: ids::PROJECT_VULKAN,
            position: 0,
        })
        .await
        .expect(CASE);
    store
        .upsert_workspace_box_path(&WorkspaceBoxPath {
            workspace_id: workspace.id,
            box_id: ids::BOX,
            root_path: "/srv/doomed".to_owned(),
            updated_at: Utc::now(),
        })
        .await
        .expect(CASE);

    let reach = store
        .delete_reach(DeleteTarget::Workspace(workspace.id))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: a workspace that exists has a reach"));
    assert_eq!(
        reach,
        DeleteReach {
            workspace_links: 1,
            workspace_box_paths: 1,
            ..DeleteReach::default()
        },
        "{CASE}: a workspace reaches its two tables and nothing else"
    );

    let report = store.delete_workspace(workspace.id).await.expect(CASE);
    assert_eq!(
        report, reach,
        "{CASE}: the counts shown before the act are the counts the act took (PRD D13)"
    );
    assert!(
        store.workspace(workspace.id).await.expect(CASE).is_none(),
        "{CASE}: the workspace is gone"
    );
    assert!(
        store
            .project(ids::PROJECT_VULKAN)
            .await
            .expect(CASE)
            .is_some(),
        "{CASE}: the project survives its workspace (0001_init.sql:162)"
    );
    assert!(
        store
            .delete_reach(DeleteTarget::Workspace(workspace.id))
            .await
            .expect(CASE)
            .is_none(),
        "{CASE}: a target that does not exist has no reach"
    );
    let again = store.delete_workspace(workspace.id).await;
    assert!(
        matches!(
            again,
            Err(StoreError::NotFound {
                entity: "workspace",
                ..
            })
        ),
        "{CASE}: a second delete is NotFound, got {again:?}"
    );
}

/// D9's create (its seed is `project_create_seeds_the_catalogue`'s to check) plus D3's
/// compare-and-set. `settings` is `{}` on create and is not
/// [`WriteStore::update_project`]'s to touch — that column belongs to
/// [`WriteStore::set_setting`] alone.
async fn project_create_update_cas<S: WriteStore>(store: &S) {
    const CASE: &str = "project_create_update_cas";
    let created = store.create_project(new_project("ops")).await.expect(CASE);
    assert_eq!(
        created.settings,
        json!({}),
        "{CASE}: a created project carries an empty settings document (D9)"
    );
    assert_eq!(
        (
            created.secret_provider.as_deref(),
            created.secret_scope.as_deref()
        ),
        (None, None),
        "{CASE}: the secret columns are MOD-10's and are not written here"
    );

    let duplicate = store.create_project(new_project("ops")).await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate slug is Constraint, got {duplicate:?}"
    );
    assert_eq!(
        store.project(created.id).await.expect(CASE).as_ref(),
        Some(&created),
        "{CASE}: the read-back is the row the create returned"
    );

    let patch = ProjectPatch {
        name: Some("Operations".to_owned()),
        ..ProjectPatch::default()
    };
    let edited = applied(
        CASE,
        store
            .update_project(created.id, created.updated_at, patch.clone())
            .await
            .expect(CASE),
    );
    assert_eq!(edited.name, "Operations", "{CASE}: the patch applied");
    assert!(
        edited.updated_at > created.updated_at,
        "{CASE}: the store's clock advances the token"
    );
    assert_eq!(
        edited.settings,
        json!({}),
        "{CASE}: update_project never touches settings"
    );

    let current = stale(
        CASE,
        store
            .update_project(created.id, created.updated_at, patch)
            .await
            .expect(CASE),
    );
    assert_eq!(
        current, edited,
        "{CASE}: Stale carries the row as it is now"
    );

    let unknown = store
        .update_project(ProjectId::new(), edited.updated_at, ProjectPatch::default())
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "project",
                ..
            })
        ),
        "{CASE}: an unknown id is NotFound, got {unknown:?}"
    );
}

/// PRD D13's cascade, counted before and reported after, over the fixture's fullest project.
///
/// The exact numbers are the fixture's per-project seed (5 kinds, 5 graphs, 15 phases, 10
/// templates); the rest are asserted non-zero, because a project delete that reported zero runs
/// while taking two would be the drift this case exists to catch. `phase_agents`,
/// `run_step_commits`, `run_step_trees` and `command_runs` are `0` on both stores here: the demo
/// database seeds none of the four tables. Only `pg_criteria.rs` can tell that apart from not
/// counting them at all, and its two cases do.
async fn project_delete_takes_everything_and_says_so<S: WriteStore>(store: &S) {
    const CASE: &str = "project_delete_takes_everything_and_says_so";
    let agy_scope = Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_AGY],
    };
    let agy_before = store
        .items(&agy_scope, &ItemFilter::default())
        .await
        .expect(CASE);

    let reach = store
        .delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the fixture project has a reach"));
    let report = store.delete_project(ids::PROJECT_HTUI).await.expect(CASE);
    assert_eq!(
        report, reach,
        "{CASE}: the counts shown before the act are the counts the act took (PRD D13)"
    );

    assert_eq!(
        (
            report.item_kinds,
            report.step_graphs,
            report.phases,
            report.prompt_templates
        ),
        (5, 5, 15, 10),
        "{CASE}: the per-project seed of ANA-9 §5.10 goes whole"
    );
    assert_eq!(report.items, 8, "{CASE}: every item of the project");
    assert_eq!(report.workspace_links, 1, "{CASE}: the Platform membership");
    assert_eq!(report.skill_bindings, 3, "{CASE}: R-SKL-2's three bindings");
    assert_eq!(
        (
            report.repos,
            report.repo_box_paths,
            report.phase_agents,
            report.run_step_commits,
            report.run_step_trees,
            report.command_runs,
            report.workspace_box_paths
        ),
        (0, 0, 0, 0, 0, 0, 0),
        "{CASE}: tables neither store seeds for this project count zero, not one"
    );
    for (label, count) in [
        ("item_key_counters", report.item_key_counters),
        ("runs", report.runs),
        ("run_steps", report.run_steps),
        ("session_events", report.session_events),
        ("notes", report.notes),
        ("revisions", report.revisions),
        ("links", report.links),
        ("documents", report.documents),
    ] {
        assert!(
            count > 0,
            "{CASE}: the fixture gives the project {label}, so the report may not be zero"
        );
    }

    assert!(
        store
            .project(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .is_none(),
        "{CASE}: the project is gone"
    );
    assert_eq!(
        row_ids(
            &store
                .items(&agy_scope, &ItemFilter::default())
                .await
                .expect(CASE)
        ),
        row_ids(&agy_before),
        "{CASE}: a sibling project's items are untouched"
    );
    assert_eq!(
        store
            .links(ids::AGY_FEAT_1, 1)
            .await
            .expect(CASE)
            .nodes
            .into_iter()
            .map(|node| node.item_id)
            .collect::<Vec<_>>(),
        vec![ids::AGY_FEAT_1],
        "{CASE}: the cross-project edge to htui:FEAT-2 went with the project it pointed at"
    );
}

/// D4's seed on a fresh project, read back through the trait: five graphs, fifteen phases, five
/// kinds and — by count alone, through `delete_reach` — ten templates. D3's table is repeated
/// here as a literal so the case checks the seed against the PRD and ANA-2 §4.1, not against
/// `seed::KINDS`, which is what it is meant to check; the `seed` module's own unit tests pin the
/// table's self-consistency (reserved names, bodies, density) with no store at all.
///
/// Template *content* and the counter's absence are per-backend facts no trait reader returns:
/// `mem.rs::seeded_templates_carry_the_shipped_bodies` and
/// `mem.rs::seed_never_writes_a_counter_row`; on Postgres,
/// `pg_criteria.rs::seeded_templates_carry_the_shipped_bodies` and
/// `pg_criteria.rs::seed_never_writes_a_counter_row`.
async fn project_create_seeds_the_catalogue<S: WriteStore>(store: &S) {
    const CASE: &str = "project_create_seeds_the_catalogue";
    /// One phase of the table below: `(name, input_kinds, gate_hard)`, the three columns D3
    /// varies. Named rather than written inline because `clippy::type_complexity` refuses the
    /// nested tuple otherwise.
    type PhaseRow = (&'static str, &'static [&'static str], bool);
    /// `(graph and kind name, prefix, description, phases)`.
    type KindRow = (
        &'static str,
        &'static str,
        &'static str,
        &'static [PhaseRow],
    );
    /// D3's table, in `item_kind.position` order — PRD D3/D5 and ANA-2 `:307-319`, not the
    /// `seed` module.
    const TABLE: [KindRow; 5] = [
        (
            "analysis",
            "ANA",
            "A question answered in writing",
            &[("research", &[], false), ("verdict", &["research"], true)],
        ),
        (
            "feature",
            "FEAT",
            "New behaviour",
            &[
                ("prd", &[], true),
                ("plan", &["prd"], true),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
        (
            "bug",
            "FIX",
            "Behaviour that is wrong",
            &[
                ("reproduce", &[], false),
                ("fix", &["reproduce", "review"], false),
                ("review", &["fix"], false),
            ],
        ),
        (
            "refactor",
            "CLEAN",
            "Behaviour kept, shape improved",
            &[
                ("plan", &[], false),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
        (
            "tooling",
            "TOOL",
            "The workshop rather than the product",
            &[
                ("plan", &[], false),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
    ];

    let project = store
        .create_project(new_project("seeded"))
        .await
        .expect(CASE);
    let p = project.id;

    // (a) graphs, by name bytes
    let graphs = store.step_graphs(p).await.expect(CASE);
    assert_eq!(
        graphs
            .iter()
            .map(|graph| graph.name.as_str())
            .collect::<Vec<_>>(),
        ["analysis", "bug", "feature", "refactor", "tooling"],
        "{CASE}: five default graphs, ordered by name bytes (a)"
    );
    for graph in &graphs {
        assert_eq!(
            graph.project_id, p,
            "{CASE}: `{}` is the new project's (a)",
            graph.name
        );
        assert_eq!(
            graph.description,
            format!("Default graph for {} items", graph.name),
            "{CASE}: `{}` description (a)",
            graph.name
        );
    }

    // (b) kinds, by position; each default graph is the graph of the same name
    let kinds = store.item_kinds(p).await.expect(CASE);
    assert_eq!(
        kinds
            .iter()
            .map(|kind| (kind.prefix.as_str(), kind.name.as_str(), kind.position))
            .collect::<Vec<_>>(),
        TABLE
            .iter()
            .enumerate()
            .map(|(position, (name, prefix, ..))| (*prefix, *name, position as i32))
            .collect::<Vec<_>>(),
        "{CASE}: five kinds, positions dense from 0 (b)"
    );
    for (kind, (_, _, description, _)) in kinds.iter().zip(TABLE.iter()) {
        let graph = graphs
            .iter()
            .find(|graph| graph.id == kind.default_graph_id)
            .unwrap_or_else(|| panic!("{CASE}: `{}` has a default graph (b)", kind.prefix));
        assert_eq!(
            graph.name, kind.name,
            "{CASE}: `{}`'s graph is its namesake (b)",
            kind.prefix
        );
        assert_eq!(
            kind.description, *description,
            "{CASE}: `{}` description (b)",
            kind.prefix
        );
    }

    // (c) every graph's phases are D3's rows; (d) none is a reserved template name
    for (name, _, _, expected) in TABLE {
        let graph = graphs
            .iter()
            .find(|graph| graph.name == name)
            .unwrap_or_else(|| panic!("{CASE}: graph `{name}` (c)"));
        let phases = store.phases(graph.id).await.expect(CASE);
        let got: Vec<(&str, Vec<&str>, bool)> = phases
            .iter()
            .map(|phase| {
                (
                    phase.name.as_str(),
                    phase.input_kinds.iter().map(String::as_str).collect(),
                    phase.gate_hard,
                )
            })
            .collect();
        let want: Vec<(&str, Vec<&str>, bool)> = expected
            .iter()
            .map(|(phase, inputs, hard)| (*phase, inputs.to_vec(), *hard))
            .collect();
        assert_eq!(got, want, "{CASE}: `{name}` phases are D3's rows (c)");

        for (position, phase) in phases.iter().enumerate() {
            assert_eq!(
                phase.graph_id, graph.id,
                "{CASE}: `{name}`/`{}` (c)",
                phase.name
            );
            assert_eq!(
                phase.position, position as i32,
                "{CASE}: `{name}` positions are dense from 0 (c)"
            );
            if position == 0 {
                assert!(
                    phase.input_kinds.is_empty(),
                    "{CASE}: `{name}` position 0 reads nothing (c)"
                );
            }
            assert_eq!(
                (
                    phase.fan_out,
                    phase.gate,
                    phase.retry_limit,
                    phase.isolation,
                    phase.command_queue,
                    phase.verify_command.as_deref(),
                    phase.template_version,
                    phase.token_budget,
                ),
                (
                    1,
                    Gate::Always,
                    1,
                    None,
                    CommandQueue::FanOutOnly,
                    None,
                    None,
                    None
                ),
                "{CASE}: `{name}`/`{}` carries ANA-2's frozen defaults (c)",
                phase.name
            );
            assert_eq!(
                phase.output_kind, phase.name,
                "{CASE}: output_kind = name (c)"
            );
            assert_eq!(
                phase.template_name, phase.name,
                "{CASE}: template_name = name (c)"
            );
            assert_eq!(
                TemplateRole::of_name(&phase.name),
                TemplateRole::Phase,
                "{CASE}: `{}` is a phase name, not `judge`/`handoff` (d)",
                phase.name
            );
        }
    }

    // (e) the count of what was seeded, templates included
    let reach = store
        .delete_reach(DeleteTarget::Project(p))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the project exists (e)"));
    assert_eq!(
        (
            reach.step_graphs,
            reach.phases,
            reach.item_kinds,
            reach.prompt_templates
        ),
        (5, 15, 5, 10),
        "{CASE}: 5 graphs, 15 phases, 5 kinds, 10 templates (e)"
    );
    assert_eq!(
        (reach.items, reach.item_key_counters),
        (0, 0),
        "{CASE}: the seed mints no item and writes no counter row (e)"
    );

    // (f) the counter is lazy: the first mint under FEAT is FEAT-1
    let feat = kinds
        .iter()
        .find(|kind| kind.prefix == "FEAT")
        .unwrap_or_else(|| panic!("{CASE}: FEAT (f)"));
    let minted = store
        .mint_item(new_item(p, feat.id, "first"))
        .await
        .expect(CASE);
    assert_eq!(
        minted.key, "FEAT-1",
        "{CASE}: no counter row was seeded (f)"
    );

    // (g) an unknown creator is refused before anything is written
    let mut orphan = new_project("orphan");
    orphan.created_by = UserId::new();
    let orphan_id = orphan.id;
    let refused = store.create_project(orphan).await;
    assert!(
        matches!(refused, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown created_by is Constraint, got {refused:?} (g)"
    );
    assert!(
        store.project(orphan_id).await.expect(CASE).is_none(),
        "{CASE}: no project row survives the refusal (g)"
    );
    assert!(
        store.step_graphs(orphan_id).await.expect(CASE).is_empty(),
        "{CASE}: no graph survives the refusal (g)"
    );
    assert!(
        store.item_kinds(orphan_id).await.expect(CASE).is_empty(),
        "{CASE}: no kind survives the refusal (g)"
    );
}

/// D10's `uq_repo_primary` rule: promoting the second repo demotes the first inside the same
/// transaction, and the demoted row's own token advances because its column changed.
async fn repo_round_trip_and_primary_flag<S: WriteStore>(store: &S) {
    const CASE: &str = "repo_round_trip_and_primary_flag";
    let core = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", true))
        .await
        .expect(CASE);
    let docs = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "docs", false))
        .await
        .expect(CASE);
    assert!(core.is_primary && !docs.is_primary, "{CASE}: as requested");

    let duplicate = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", false))
        .await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate (project, name) is Constraint, got {duplicate:?}"
    );
    let orphan = store
        .create_repo(new_repo(ProjectId::new(), "core", false))
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: a repo of no project is Constraint, got {orphan:?}"
    );

    assert_eq!(
        store
            .repos(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["core".to_owned(), "docs".to_owned()],
        "{CASE}: repos come back in name byte order"
    );

    let promoted = applied(
        CASE,
        store
            .update_repo(
                docs.id,
                docs.updated_at,
                RepoPatch {
                    is_primary: Some(true),
                    remote_url: Some(None),
                    ..RepoPatch::default()
                },
            )
            .await
            .expect(CASE),
    );
    assert!(
        promoted.is_primary,
        "{CASE}: the second repo is now primary"
    );
    assert_eq!(
        promoted.remote_url, None,
        "{CASE}: `Some(None)` clears the URL, `None` would have left it"
    );
    let demoted = store
        .repos(ids::PROJECT_HTUI)
        .await
        .expect(CASE)
        .into_iter()
        .find(|row| row.id == core.id)
        .unwrap_or_else(|| panic!("{CASE}: the first repo survives the promotion"));
    assert!(
        !demoted.is_primary,
        "{CASE}: promoting one demotes the other, so uq_repo_primary never trips (D10)"
    );
    assert!(
        demoted.updated_at > core.updated_at,
        "{CASE}: the demoted row's column changed, so its token changed too"
    );

    let spent = stale(
        CASE,
        store
            .update_repo(core.id, core.updated_at, RepoPatch::default())
            .await
            .expect(CASE),
    );
    assert_eq!(spent, demoted, "{CASE}: Stale carries the row as it is now");
    let unknown = store
        .update_repo(RepoId::new(), core.updated_at, RepoPatch::default())
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "repo", .. })),
        "{CASE}: an unknown id is NotFound, got {unknown:?}"
    );

    let path = |local: &str| RepoBoxPath {
        repo_id: core.id,
        box_id: ids::BOX,
        local_path: local.to_owned(),
        updated_at: Utc::now(),
    };
    store
        .upsert_repo_box_path(&path("/src/one"))
        .await
        .expect(CASE);
    store
        .upsert_repo_box_path(&path("/src/two"))
        .await
        .expect(CASE);
    assert_eq!(
        store
            .repo_box_paths(core.id)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| (row.box_id, row.local_path))
            .collect::<Vec<_>>(),
        vec![(ids::BOX, "/src/two".to_owned())],
        "{CASE}: the per-box checkout path is replaced in place (R-BOX-4)"
    );
}

/// D11's three seam-side rules and PRD D12's rename semantics: old keys keep their text, the old
/// counter survives, and the next mint under the kind starts the new prefix at 1.
///
/// The middle fact is the one no trait reader can see, so it is asserted per backend, which
/// `mem.rs::renamed_prefix_leaves_the_old_counter_row` does by reading `item_key_counter` directly
/// and `pg_criteria.rs::renamed_prefix_leaves_the_old_counter_row` off the table.
async fn item_kind_round_trip_and_prefix_rules<S: WriteStore>(store: &S) {
    const CASE: &str = "item_kind_round_trip_and_prefix_rules";
    for (label, prefix) in [
        ("lowercase", "feat"),
        ("leading digit", "1A"),
        ("one byte", "A"),
        ("seventeen bytes", "ABCDEFGHIJKLMNOPQ"),
        ("a hyphen", "AN-A"),
    ] {
        let refused = store
            .create_item_kind(new_item_kind(
                ids::PROJECT_HTUI,
                prefix,
                &format!("kind {label}"),
                ids::GRAPH_HTUI_ANA,
            ))
            .await;
        assert!(
            matches!(refused, Err(StoreError::Constraint(_))),
            "{CASE}: a prefix with {label} is Constraint, got {refused:?}"
        );
    }

    let taken_prefix = store
        .create_item_kind(new_item_kind(
            ids::PROJECT_HTUI,
            "ANA",
            "second analysis",
            ids::GRAPH_HTUI_ANA,
        ))
        .await;
    assert!(
        matches!(taken_prefix, Err(StoreError::Constraint(_))),
        "{CASE}: (project, prefix) is unique, got {taken_prefix:?}"
    );
    let foreign_graph = store
        .create_item_kind(new_item_kind(
            ids::PROJECT_HTUI,
            "SPEC",
            "specification",
            ids::GRAPH_AGY_FEAT,
        ))
        .await;
    assert!(
        matches!(foreign_graph, Err(StoreError::Constraint(_))),
        "{CASE}: a default graph from another project is Constraint, got {foreign_graph:?}"
    );

    let created = store
        .create_item_kind(new_item_kind(
            ids::PROJECT_HTUI,
            "SPEC",
            "specification",
            ids::GRAPH_HTUI_ANA,
        ))
        .await
        .expect(CASE);
    assert_eq!(created.prefix, "SPEC", "{CASE}: the request's prefix");

    let kinds = store.item_kinds(ids::PROJECT_HTUI).await.expect(CASE);
    assert_eq!(
        kinds
            .iter()
            .map(|row| row.prefix.as_str())
            .collect::<Vec<_>>(),
        vec!["ANA", "FEAT", "FIX", "CLEAN", "TOOL", "SPEC"],
        "{CASE}: kinds come back in position order, the fresh one last at position 9"
    );

    let ana = kinds
        .iter()
        .find(|row| row.id == ids::KIND_HTUI_ANA)
        .unwrap_or_else(|| panic!("{CASE}: the fixture kind"))
        .clone();
    let renamed = applied(
        CASE,
        store
            .update_item_kind(
                ana.id,
                ana.updated_at,
                ItemKindPatch {
                    prefix: Some("ANL".to_owned()),
                    ..ItemKindPatch::default()
                },
            )
            .await
            .expect(CASE),
    );
    assert_eq!(renamed.prefix, "ANL", "{CASE}: the rename landed");
    assert_eq!(
        store
            .item(ids::HTUI_ANA_2)
            .await
            .expect(CASE)
            .unwrap_or_else(|| panic!("{CASE}: the fixture item"))
            .key,
        "ANA-2",
        "{CASE}: key_prefix is copied at mint time and never rewritten (ANA-9 §4.1)"
    );
    let minted = store
        .mint_item(new_item(
            ids::PROJECT_HTUI,
            ids::KIND_HTUI_ANA,
            "after the rename",
        ))
        .await
        .expect(CASE);
    assert_eq!(
        minted.key, "ANL-1",
        "{CASE}: the counter is keyed by prefix, so the new prefix mints from 1 (PRD D12)"
    );

    let spent = stale(
        CASE,
        store
            .update_item_kind(ana.id, ana.updated_at, ItemKindPatch::default())
            .await
            .expect(CASE),
    );
    assert_eq!(spent, renamed, "{CASE}: Stale carries the row as it is now");
    let bad_rename = store
        .update_item_kind(
            renamed.id,
            renamed.updated_at,
            ItemKindPatch {
                prefix: Some("anl".to_owned()),
                ..ItemKindPatch::default()
            },
        )
        .await;
    assert!(
        matches!(bad_rename, Err(StoreError::Constraint(_))),
        "{CASE}: the prefix rule applies to a rename too, got {bad_rename:?}"
    );
    let stale_and_invalid = stale(
        CASE,
        store
            .update_item_kind(
                renamed.id,
                ana.updated_at,
                ItemKindPatch {
                    prefix: Some("anl".to_owned()),
                    ..ItemKindPatch::default()
                },
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        stale_and_invalid, renamed,
        "{CASE}: a spent token and an invalid prefix together answer Stale, not Constraint - the \
         compare-and-set is checked first, the way a slug collision already is (review M2)"
    );
    let unknown = store
        .update_item_kind(ItemKindId::new(), ana.updated_at, ItemKindPatch::default())
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "item_kind",
                ..
            })
        ),
        "{CASE}: an unknown id is NotFound, got {unknown:?}"
    );
}

/// D6: a kind items point at cannot be deleted, and the refusal names the count rather than a
/// constraint name. An unreferenced kind goes, and goes only once.
async fn item_kind_delete_refused_while_referenced<S: WriteStore>(store: &S) {
    const CASE: &str = "item_kind_delete_refused_while_referenced";
    let held = store.delete_item_kind(ids::KIND_HTUI_ANA).await;
    match held {
        Err(StoreError::Constraint(text)) => assert!(
            text.contains("held by 2 items"),
            "{CASE}: the refusal names what holds it (D6), got `{text}`"
        ),
        other => panic!("{CASE}: a referenced kind is Constraint, got {other:?}"),
    }
    assert!(
        store
            .item_kinds(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .iter()
            .any(|row| row.id == ids::KIND_HTUI_ANA),
        "{CASE}: the refused delete removed nothing"
    );

    let fresh = store
        .create_item_kind(new_item_kind(
            ids::PROJECT_HTUI,
            "SPEC",
            "specification",
            ids::GRAPH_HTUI_ANA,
        ))
        .await
        .expect(CASE);
    store.delete_item_kind(fresh.id).await.expect(CASE);
    assert!(
        !store
            .item_kinds(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .iter()
            .any(|row| row.id == fresh.id),
        "{CASE}: an unreferenced kind goes"
    );
    let again = store.delete_item_kind(fresh.id).await;
    assert!(
        matches!(
            again,
            Err(StoreError::NotFound {
                entity: "item_kind",
                ..
            })
        ),
        "{CASE}: a second delete is NotFound, got {again:?}"
    );
}

/// The graph and its phases: uniqueness the database owns, the reserved names D11 owns, and
/// [`PhasePatch`]'s five columns — `token_budget` is not among them, because the `Phase` rung of
/// [`WriteStore::set_setting`] is that column's one writer (D8).
async fn step_graph_and_phase_round_trip<S: WriteStore>(store: &S) {
    const CASE: &str = "step_graph_and_phase_round_trip";
    let graph = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "release".to_owned(),
            description: "Cut a release".to_owned(),
        })
        .await
        .expect(CASE);

    let duplicate = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "analysis".to_owned(),
            description: String::new(),
        })
        .await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: (project, name) is unique, got {duplicate:?}"
    );

    assert_eq!(
        store
            .step_graphs(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec![
            "analysis".to_owned(),
            "bug".to_owned(),
            "feature".to_owned(),
            "refactor".to_owned(),
            "release".to_owned(),
            "tooling".to_owned(),
        ],
        "{CASE}: graphs come back in name byte order"
    );

    let collision = store
        .update_step_graph(
            graph.id,
            graph.updated_at,
            StepGraphPatch {
                name: Some("analysis".to_owned()),
                ..StepGraphPatch::default()
            },
        )
        .await;
    assert!(
        matches!(collision, Err(StoreError::Constraint(_))),
        "{CASE}: a rename into a taken name is Constraint, got {collision:?}"
    );
    let described = applied(
        CASE,
        store
            .update_step_graph(
                graph.id,
                graph.updated_at,
                StepGraphPatch {
                    description: Some("Tag and publish".to_owned()),
                    ..StepGraphPatch::default()
                },
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        (described.description.as_str(), described.name.as_str()),
        ("Tag and publish", "release"),
        "{CASE}: `None` leaves the column the patch does not name"
    );
    let spent_graph = stale(
        CASE,
        store
            .update_step_graph(graph.id, graph.updated_at, StepGraphPatch::default())
            .await
            .expect(CASE),
    );
    assert_eq!(
        spent_graph, described,
        "{CASE}: Stale carries the graph as it is now"
    );
    let unknown_graph = store
        .update_step_graph(
            StepGraphId::new(),
            graph.updated_at,
            StepGraphPatch::default(),
        )
        .await;
    assert!(
        matches!(
            unknown_graph,
            Err(StoreError::NotFound {
                entity: "step_graph",
                ..
            })
        ),
        "{CASE}: an unknown graph is NotFound, got {unknown_graph:?}"
    );

    let phase = store
        .create_phase(&new_phase(graph.id, 0, "cut"))
        .await
        .expect(CASE);
    assert_eq!(phase.name, "cut", "{CASE}: the row the caller built");

    for reserved in ["judge", "handoff"] {
        let refused = store.create_phase(&new_phase(graph.id, 7, reserved)).await;
        assert!(
            matches!(refused, Err(StoreError::Constraint(_))),
            "{CASE}: `{reserved}` is a template role, not a phase name (D11), got {refused:?}"
        );
    }
    let taken_position = store
        .create_phase(&new_phase(ids::GRAPH_HTUI_FEAT, 0, "rehearse"))
        .await;
    assert!(
        matches!(taken_position, Err(StoreError::Constraint(_))),
        "{CASE}: (graph, position) is unique, got {taken_position:?}"
    );
    let taken_name = store
        .create_phase(&new_phase(ids::GRAPH_HTUI_FEAT, 9, "prd"))
        .await;
    assert!(
        matches!(taken_name, Err(StoreError::Constraint(_))),
        "{CASE}: (graph, name) is unique, got {taken_name:?}"
    );
    let orphan = store
        .create_phase(&new_phase(StepGraphId::new(), 0, "cut"))
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: a phase of no graph is Constraint, got {orphan:?}"
    );

    assert_eq!(
        store
            .phases(ids::GRAPH_HTUI_FEAT)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| (row.position, row.name))
            .collect::<Vec<_>>(),
        vec![
            (0, "prd".to_owned()),
            (1, "plan".to_owned()),
            (2, "implement".to_owned()),
            (3, "review".to_owned()),
        ],
        "{CASE}: phases come back in position order"
    );

    let edited = applied(
        CASE,
        store
            .update_phase(
                phase.id,
                phase.updated_at,
                PhasePatch {
                    name: Some("tag".to_owned()),
                    position: Some(3),
                    template_name: Some("plan".to_owned()),
                    gate_hard: Some(true),
                    input_kinds: Some(vec!["plan".to_owned()]),
                },
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        (
            edited.name.as_str(),
            edited.position,
            edited.template_name.as_str(),
            edited.gate_hard,
            edited.input_kinds.as_slice()
        ),
        ("tag", 3, "plan", true, ["plan".to_owned()].as_slice()),
        "{CASE}: all five columns of the patch"
    );
    assert_eq!(
        edited.token_budget, None,
        "{CASE}: the patch has no token_budget to write (D8)"
    );

    let reserved_rename = store
        .update_phase(
            edited.id,
            edited.updated_at,
            PhasePatch {
                name: Some("judge".to_owned()),
                ..PhasePatch::default()
            },
        )
        .await;
    assert!(
        matches!(reserved_rename, Err(StoreError::Constraint(_))),
        "{CASE}: a phase cannot be renamed into a reserved name either, got {reserved_rename:?}"
    );
    let spent = stale(
        CASE,
        store
            .update_phase(phase.id, phase.updated_at, PhasePatch::default())
            .await
            .expect(CASE),
    );
    assert_eq!(spent, edited, "{CASE}: Stale carries the row as it is now");
    let stale_and_reserved = stale(
        CASE,
        store
            .update_phase(
                phase.id,
                phase.updated_at,
                PhasePatch {
                    name: Some("judge".to_owned()),
                    ..PhasePatch::default()
                },
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        stale_and_reserved, edited,
        "{CASE}: a spent token and a reserved name together answer Stale, not Constraint - the \
         compare-and-set is checked first, the way a (graph, name) collision already is (review M2)"
    );
    let unknown = store
        .update_phase(PhaseId::new(), phase.updated_at, PhasePatch::default())
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "step_graph_phase",
                ..
            })
        ),
        "{CASE}: an unknown id is NotFound, got {unknown:?}"
    );
}

/// D7's promise on the rung every key accepts: every refusal is a `Constraint` carrying the rule,
/// and nothing is clamped into a number the writer did not ask for.
///
/// The token is read first rather than assumed: migration `0002` seeds the ten rows on Postgres and
/// `MemStore` loads none, so `expected` is `Some` on one store and `None` on the other and the case
/// must pass on both.
async fn settings_app_rung_validates_and_cas<S: WriteStore>(store: &S) {
    const CASE: &str = "settings_app_rung_validates_and_cas";
    let before = store
        .setting(SettingRung::App, SettingKey::TokenBudget)
        .await
        .expect(CASE);
    let token = before.as_ref().map(|row| row.updated_at);

    let written = applied(
        CASE,
        store
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                json!(90_000),
                token,
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        written.value,
        Some(json!(90_000)),
        "{CASE}: the value as stored comes back"
    );
    assert_eq!(
        store
            .setting(SettingRung::App, SettingKey::TokenBudget)
            .await
            .expect(CASE),
        Some(written.clone()),
        "{CASE}: the read-back is the row the write returned"
    );

    for (label, key, value) in [
        ("zero", SettingKey::TokenBudget, json!(0)),
        ("a string", SettingKey::TokenBudget, json!("120000")),
        ("a float", SettingKey::TokenBudget, json!(1.5)),
        ("three hops", SettingKey::UpstreamHops, json!(3)),
        (
            "half the budget and one basis point",
            SettingKey::PromptReserveFraction,
            json!(0.5001),
        ),
        (
            "more files than a u32",
            SettingKey::ExcerptMaxFiles,
            json!(i64::from(u32::MAX) + 1),
        ),
        (
            "a head above the current line cap",
            SettingKey::ExcerptHeadLines,
            json!(500),
        ),
    ] {
        let refused = store
            .set_setting(SettingRung::App, key, value.clone(), None)
            .await;
        match refused {
            Err(StoreError::Constraint(text)) => assert!(
                text.contains(key.key()),
                "{CASE}: the refusal of {label} names its key, got `{text}`"
            ),
            other => panic!("{CASE}: {label} is refused, not clamped, got {other:?}"),
        }
    }
    assert_eq!(
        store
            .setting(SettingRung::App, SettingKey::TokenBudget)
            .await
            .expect(CASE)
            .and_then(|row| row.value),
        Some(json!(90_000)),
        "{CASE}: a refused write stored nothing"
    );

    let spent = stale(
        CASE,
        store
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                json!(80_000),
                token,
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        spent, written,
        "{CASE}: a spent token is Stale carrying the setting as stored now"
    );

    let cleared = applied(
        CASE,
        store
            .clear_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                written.updated_at,
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        cleared.value, None,
        "{CASE}: a clear always answers with no value"
    );
    assert!(
        store
            .setting(SettingRung::App, SettingKey::TokenBudget)
            .await
            .expect(CASE)
            .is_none(),
        "{CASE}: the App row is deleted, so the compiled-in default answers (D7)"
    );
    let gone = store
        .clear_setting(
            SettingRung::App,
            SettingKey::TokenBudget,
            written.updated_at,
        )
        .await;
    assert!(
        matches!(
            gone,
            Err(StoreError::NotFound {
                entity: "app_setting",
                ..
            })
        ),
        "{CASE}: clearing what is not there is NotFound, got {gone:?}"
    );

    let reinserted = applied(
        CASE,
        store
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                json!(70_000),
                None,
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        reinserted.value,
        Some(json!(70_000)),
        "{CASE}: `expected: None` is the insert after a clear (D8)"
    );
    let occupied = stale(
        CASE,
        store
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                json!(60_000),
                None,
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        occupied, reinserted,
        "{CASE}: `expected: None` against a row that exists is Stale, never an overwrite"
    );
}

/// PRD's "`project.settings` loses nothing": one key is merged in, every other key comes back
/// equal, and the clear takes only what it wrote. The `mem.rs` twin
/// `set_setting_project_rung_leaves_unknown_keys_byte_identical` asserts the stronger byte
/// identity, which JSONB's key normalisation makes unassertable across both stores; the Postgres
/// twin is `pg_criteria.rs::set_setting_project_rung_changes_only_settings_and_updated_at`, which
/// diffs the whole row to prove the merge touches nothing but `settings` and `updated_at`.
async fn settings_project_rung_merges_keys<S: WriteStore>(store: &S) {
    const CASE: &str = "settings_project_rung_merges_keys";
    let rung = SettingRung::Project(ids::PROJECT_HTUI);
    let project = store
        .project(ids::PROJECT_HTUI)
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the fixture project"));
    let before = settings_map(CASE, &project.settings);
    assert!(
        !before.is_empty(),
        "{CASE}: the fixture seeds keys this write must not disturb"
    );

    let wrong_rung = store
        .set_setting(rung, SettingKey::ExcerptMaxFiles, json!(3), None)
        .await;
    assert!(
        matches!(wrong_rung, Err(StoreError::Constraint(_))),
        "{CASE}: a key the project rung does not accept is Constraint, got {wrong_rung:?}"
    );
    let no_token = store
        .set_setting(rung, SettingKey::UpstreamHops, json!(2), None)
        .await;
    assert!(
        matches!(no_token, Err(StoreError::Constraint(_))),
        "{CASE}: `expected: None` on a rung whose row always exists is misuse, got {no_token:?}"
    );

    let written = applied(
        CASE,
        store
            .set_setting(
                rung,
                SettingKey::UpstreamHops,
                json!(2),
                Some(project.updated_at),
            )
            .await
            .expect(CASE),
    );
    assert_eq!(written.value, Some(json!(2)), "{CASE}: the value as stored");

    let merged = settings_map(
        CASE,
        &store
            .project(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .unwrap_or_else(|| panic!("{CASE}: the project survives its own settings write"))
            .settings,
    );
    assert_eq!(
        merged.get("upstream_hops"),
        Some(&json!(2)),
        "{CASE}: the project rung writes the key `resolve_hops` reads, not the App key (flag A)"
    );
    for (key, value) in &before {
        assert_eq!(
            merged.get(key),
            Some(value),
            "{CASE}: `{key}` survived the merge unchanged"
        );
    }

    let spent = stale(
        CASE,
        store
            .set_setting(
                rung,
                SettingKey::UpstreamHops,
                json!(1),
                Some(project.updated_at),
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        spent, written,
        "{CASE}: the CAS is on project.updated_at, which the merge advanced"
    );

    let cleared = applied(
        CASE,
        store
            .clear_setting(rung, SettingKey::UpstreamHops, written.updated_at)
            .await
            .expect(CASE),
    );
    assert_eq!(cleared.value, None, "{CASE}: a clear answers with no value");
    assert_eq!(
        settings_map(
            CASE,
            &store
                .project(ids::PROJECT_HTUI)
                .await
                .expect(CASE)
                .unwrap_or_else(|| panic!("{CASE}: the project survives the clear"))
                .settings,
        ),
        before,
        "{CASE}: the clear took the one key it wrote and left the document as it found it"
    );
    assert_eq!(
        store
            .setting(rung, SettingKey::UpstreamHops)
            .await
            .expect(CASE)
            .map(|row| row.value),
        Some(None),
        "{CASE}: a row that exists without the key answers Some(value: None)"
    );

    let unknown = store
        .setting(
            SettingRung::Project(ProjectId::new()),
            SettingKey::UpstreamHops,
        )
        .await
        .expect(CASE);
    assert!(
        unknown.is_none(),
        "{CASE}: a rung whose row is absent answers None, got {unknown:?}"
    );
    let unknown_write = store
        .set_setting(
            SettingRung::Project(ProjectId::new()),
            SettingKey::UpstreamHops,
            json!(1),
            Some(project.updated_at),
        )
        .await;
    assert!(
        matches!(
            unknown_write,
            Err(StoreError::NotFound {
                entity: "project",
                ..
            })
        ),
        "{CASE}: an unknown project is NotFound, got {unknown_write:?}"
    );
}

/// The `Phase` rung is one `INTEGER` column, so it accepts one key and caps where the column does
/// (flag C). The value is read back through [`WriteStore::phases`] as well as through
/// [`WriteStore::setting`], because the editor and the resolver read it from different places.
async fn settings_phase_rung_writes_token_budget_only<S: WriteStore>(store: &S) {
    const CASE: &str = "settings_phase_rung_writes_token_budget_only";
    let rung = SettingRung::Phase(ids::PHASE_HTUI_IMPLEMENT);
    let budget_of = |phases: Vec<StepGraphPhase>| {
        phases
            .into_iter()
            .find(|row| row.id == ids::PHASE_HTUI_IMPLEMENT)
            .unwrap_or_else(|| panic!("{CASE}: the fixture phase"))
    };
    let phase = budget_of(store.phases(ids::GRAPH_HTUI_FEAT).await.expect(CASE));
    assert_eq!(
        phase.token_budget, None,
        "{CASE}: the fixture leaves the column NULL"
    );

    let wrong_key = store
        .set_setting(rung, SettingKey::UpstreamHops, json!(1), None)
        .await;
    assert!(
        matches!(wrong_key, Err(StoreError::Constraint(_))),
        "{CASE}: the phase rung accepts token_budget alone, got {wrong_key:?}"
    );
    let no_token = store
        .set_setting(rung, SettingKey::TokenBudget, json!(90_000), None)
        .await;
    assert!(
        matches!(no_token, Err(StoreError::Constraint(_))),
        "{CASE}: `expected: None` on a rung whose row always exists is misuse, got {no_token:?}"
    );
    let too_wide = store
        .set_setting(
            rung,
            SettingKey::TokenBudget,
            json!(i64::from(i32::MAX) + 1),
            Some(phase.updated_at),
        )
        .await;
    assert!(
        matches!(too_wide, Err(StoreError::Constraint(_))),
        "{CASE}: the column is INTEGER, so this rung caps at i32::MAX, got {too_wide:?}"
    );

    let written = applied(
        CASE,
        store
            .set_setting(
                rung,
                SettingKey::TokenBudget,
                json!(90_000),
                Some(phase.updated_at),
            )
            .await
            .expect(CASE),
    );
    assert_eq!(written.value, Some(json!(90_000)), "{CASE}: as stored");
    assert_eq!(
        budget_of(store.phases(ids::GRAPH_HTUI_FEAT).await.expect(CASE)).token_budget,
        Some(90_000),
        "{CASE}: the column the resolver reads carries the number"
    );
    assert_eq!(
        store
            .setting(rung, SettingKey::TokenBudget)
            .await
            .expect(CASE),
        Some(written.clone()),
        "{CASE}: the read-back is the row the write returned"
    );

    let spent = stale(
        CASE,
        store
            .set_setting(
                rung,
                SettingKey::TokenBudget,
                json!(80_000),
                Some(phase.updated_at),
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        spent, written,
        "{CASE}: the CAS is on the phase's own updated_at"
    );

    let cleared = applied(
        CASE,
        store
            .clear_setting(rung, SettingKey::TokenBudget, written.updated_at)
            .await
            .expect(CASE),
    );
    assert_eq!(cleared.value, None, "{CASE}: a clear answers with no value");
    assert_eq!(
        budget_of(store.phases(ids::GRAPH_HTUI_FEAT).await.expect(CASE)).token_budget,
        None,
        "{CASE}: the column is NULL again, so the project rung answers"
    );
    assert_eq!(
        store
            .setting(rung, SettingKey::TokenBudget)
            .await
            .expect(CASE)
            .map(|row| row.value),
        Some(None),
        "{CASE}: a phase that exists without a budget answers Some(value: None)"
    );

    let unknown = SettingRung::Phase(PhaseId::new());
    assert!(
        store
            .setting(unknown, SettingKey::TokenBudget)
            .await
            .expect(CASE)
            .is_none(),
        "{CASE}: a phase that does not exist has no setting"
    );
    let unknown_write = store
        .set_setting(
            unknown,
            SettingKey::TokenBudget,
            json!(1),
            Some(written.updated_at),
        )
        .await;
    assert!(
        matches!(
            unknown_write,
            Err(StoreError::NotFound {
                entity: "step_graph_phase",
                ..
            })
        ),
        "{CASE}: an unknown phase is NotFound, got {unknown_write:?}"
    );
}

// ------------------------------------------------------------------------------------------------
// ANA-2 §8: the run seam (MOD-4 milestone 1, plan D12)
// ------------------------------------------------------------------------------------------------

/// The caller's clock at `timestamptz`'s own resolution, which is what every §8 writer's stamp has
/// to be.
///
/// Blueprint F-S puts the clock in the caller's hands for `started_at`, `finished_at`,
/// `lease_expires_at` and `promoted_at`, and the cases below read those columns back and compare
/// them with the value they passed. A raw [`Utc::now`] is nanoseconds on Linux; Postgres keeps
/// [`TIMESTAMPTZ_DIGITS`] of them and `MemStore` keeps all of them, so an untruncated reading makes
/// the two backends disagree about a column neither of them changed. [`ChatRunSpec::mint`] has
/// truncated for the same reason since MOD-2; this is the §8 writers' half of it, and `PgStore`
/// cannot fix it on its own — a store that truncated on the way in would still hand back something
/// the caller's own variable no longer equals.
fn seam_clock() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS)
}

/// The smallest `R-ORCH-11` snapshot [`WriteStore::create_run`] accepts: the `htui` FEAT graph,
/// no phases, and the settings the seed rungs resolve to.
///
/// Deliberately not the fixture's own `demo_graph_snapshot`: a case that asserts the column
/// round-trips must pass a value it can recognise afterwards, and a snapshot with no phase is the
/// shortest one that still decodes as §5.1's type.
fn run_snapshot() -> GraphSnapshot {
    GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: ids::GRAPH_HTUI_FEAT,
            name: "feature".to_owned(),
            is_override: false,
        },
        topology: "sha256:conformance".to_owned(),
        mode: RunMode::Manual,
        phases: Vec::new(),
        settings: SnapshotSettings {
            default_isolation: Isolation::Worktree,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            max_fan_out: 4,
            max_agents_per_run: 8,
        },
        scope: None,
    }
}

/// A graph-run request with a fresh id, targeting the fixture box and started by the fixture user.
fn new_run(project: ProjectId, item: ItemId, scope: Vec<RepoId>) -> NewRun {
    NewRun {
        id: RunId::new(),
        project_id: project,
        item_id: item,
        mode: RunMode::Manual,
        target_box_id: ids::BOX,
        started_by: ids::USER,
        graph_snapshot: run_snapshot(),
        repo_scope: scope,
        queued_at: seam_clock(),
    }
}

/// A step request with a fresh id at one `(position, attempt, fanout_index)` slot of `run`.
fn new_run_step(run: RunId, position: i32, attempt: i32, fanout_index: i32) -> NewRunStep {
    NewRunStep {
        id: StepId::new(),
        run_id: run,
        position,
        attempt,
        fanout_index,
        phase_name: "implement".to_owned(),
        agent_id: Some(ids::AGENT_CLAUDE),
        model: Some("opus".to_owned()),
    }
}

/// A document request with a fresh id; the version is the store's to allocate, never the caller's.
fn new_document(item: ItemId, kind: &str, step: Option<StepId>) -> NewDocument {
    NewDocument {
        id: DocumentId::new(),
        item_id: item,
        kind: kind.to_owned(),
        title: format!("{kind} of {item}"),
        body: String::new(),
        produced_by_step_id: step,
        created_by: ids::USER,
        created_at: seam_clock(),
    }
}

/// A note request with a fresh id, on the fixture box and by the fixture user.
fn new_note(item: ItemId, author: UserId, step: Option<StepId>) -> NewNote {
    NewNote {
        id: NoteId::new(),
        item_id: item,
        body: "Refused: the tree was dirty.".to_owned(),
        created_by: author,
        box_id: Some(ids::BOX),
        via_step_id: step,
        created_at: seam_clock(),
    }
}

/// Creates `spec` and drives it `pending -> running -> awaiting_approval`, the two legal moves a
/// gate answer needs to have something to answer.
async fn gated_step<S: WriteStore>(case: &str, store: &S, spec: NewRunStep) -> StepId {
    let id = spec.id;
    let at = seam_clock();
    store.create_step(spec).await.expect(case);
    for (from, to) in [
        (StepStatus::Pending, StepStatus::Running),
        (StepStatus::Running, StepStatus::AwaitingApproval),
    ] {
        assert!(
            store.transition_step(id, from, to, at).await.expect(case),
            "{case}: {from} -> {to} is a sanctioned move"
        );
    }
    id
}

/// The one step of `run` with that id, or a panic naming the case: the cases read steps back
/// through [`ReadStore::run_steps`], never through a store-specific accessor.
async fn step_row<S: ReadStore>(case: &str, store: &S, run: RunId, step: StepId) -> RunStep {
    store
        .run_steps(run)
        .await
        .expect(case)
        .into_iter()
        .find(|row| row.id == step)
        .unwrap_or_else(|| panic!("{case}: run {run} holds step {step}"))
}

/// The run row, or a panic naming the case.
async fn run_row<S: ReadStore>(case: &str, store: &S, run: RunId) -> Run {
    store
        .run(run)
        .await
        .expect(case)
        .unwrap_or_else(|| panic!("{case}: run {run} exists"))
}

/// The item row, or a panic naming the case.
async fn item_row<S: ReadStore>(case: &str, store: &S, item: ItemId) -> Item {
    store
        .item(item)
        .await
        .expect(case)
        .unwrap_or_else(|| panic!("{case}: item {item} exists"))
}

/// Plan D6: the `run` row and the item's move to `queued` land together or not at all (ANA-2 §4.3,
/// §5.1). The `MemStore` twin is
/// `mem.rs::create_run_moves_the_item_and_writes_nothing_when_the_law_refuses`; on Postgres the
/// snapshot column's own guard is `pg_criteria.rs::ck_run_graph_snapshot_is_not_valid_for_old_rows_and_checked_for_new`.
async fn run_create_moves_the_item<S: WriteStore>(store: &S) {
    const CASE: &str = "run_create_moves_the_item";
    let before = item_row(CASE, store, ids::HTUI_ANA_2).await;
    assert_eq!(before.status, Status::Open, "{CASE}: fixture precondition");

    let request = new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new());
    let id = request.id;
    let scope = request.repo_scope.clone();
    let created = store.create_run(request).await.expect(CASE);
    assert_eq!(
        created.status,
        RunStatus::Queued,
        "{CASE}: a run starts queued"
    );
    assert_eq!(
        created.kind,
        RunKind::Graph,
        "{CASE}: create_run mints graph runs"
    );
    assert_eq!(
        created.item_id,
        Some(ids::HTUI_ANA_2),
        "{CASE}: the item is carried"
    );
    assert_eq!(
        created.executing_box_id, None,
        "{CASE}: nothing has claimed it"
    );
    assert_eq!(
        created.repo_scope, scope,
        "{CASE}: the request's scope is stored"
    );
    assert_eq!(
        (created.lease_box_id, created.lease_expires_at),
        (None, None),
        "{CASE}: an unclaimed run holds no lease"
    );
    let snapshot: GraphSnapshot = serde_json::from_value(
        created
            .graph_snapshot
            .clone()
            .unwrap_or_else(|| panic!("{CASE}: a graph run carries its snapshot")),
    )
    .unwrap_or_else(|error| panic!("{CASE}: the snapshot decodes as §5.1's type, got {error}"));
    assert_eq!(
        snapshot,
        run_snapshot(),
        "{CASE}: the snapshot round-trips whole"
    );

    let after = item_row(CASE, store, ids::HTUI_ANA_2).await;
    assert_eq!(
        after.status,
        Status::Queued,
        "{CASE}: the item moved with the run"
    );
    assert_eq!(
        after.version, before.version,
        "{CASE}: a transition writes no revision, so the version stands"
    );
    assert_eq!(
        run_row(CASE, store, id).await,
        created,
        "{CASE}: `run` answers the row `create_run` returned"
    );

    let duplicate = NewRun {
        id,
        ..new_run(ids::PROJECT_AGY, ids::AGY_FEAT_1, Vec::new())
    };
    let repeated = store.create_run(duplicate).await;
    assert!(
        matches!(repeated, Err(StoreError::Constraint(_))),
        "{CASE}: a run id that exists is Constraint, got {repeated:?}"
    );

    let unknown = store
        .create_run(new_run(ids::PROJECT_HTUI, ItemId::new(), Vec::new()))
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: an unknown item is NotFound, got {unknown:?}"
    );

    // Plan D14 on a request with two faults at once: the item is looked up first, so the missing
    // row is what the refusal names even though the project is a foreign key that would also
    // refuse. Without this leg each store may order its own checks and still pass.
    let both_wrong = store
        .create_run(new_run(ProjectId::new(), ItemId::new(), Vec::new()))
        .await;
    assert!(
        matches!(both_wrong, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: D14 — an unknown item outranks an unknown project, got {both_wrong:?}"
    );

    // `run.repo_scope` is a `UUID[]`, and an array element cannot carry a `REFERENCES` clause, so
    // Postgres cannot refuse this one for us: the writer has to. A run scoped to a repo that does
    // not exist would make `claim_run`'s `repo_scope && $2::uuid[]` overlap test compare against a
    // phantom id for the rest of the run's life.
    let phantom = RepoId::new();
    let unscoped = store
        .create_run(new_run(ids::PROJECT_AGY, ids::AGY_FEAT_1, vec![phantom]))
        .await;
    assert!(
        matches!(unscoped, Err(StoreError::Constraint(ref message))
            if message.contains("run.repo_scope")),
        "{CASE}: a scope naming an unknown repo is refused by name, got {unscoped:?}"
    );
    assert_eq!(
        item_row(CASE, store, ids::AGY_FEAT_1).await.status,
        Status::Open,
        "{CASE}: and the refusal left the item where it was"
    );

    // `run.project_id` and `item.project_id` are two columns, and nothing in the schema ties them:
    // a run can be filed under project A while its item lives in project B, where `runs(item)`
    // would list it and `delete_project`'s count — which counts by `run.project_id` — would take
    // it with the wrong project. Both stores already hold the item row under lock here, so both
    // can compare.
    let elsewhere = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::AGY_FEAT_1, Vec::new()))
        .await;
    assert!(
        matches!(elsewhere, Err(StoreError::Constraint(ref message))
            if message.contains("is not in project")),
        "{CASE}: a run cannot be filed under a project its item is not in, got {elsewhere:?}"
    );
    assert_eq!(
        item_row(CASE, store, ids::AGY_FEAT_1).await.status,
        Status::Open,
        "{CASE}: and that refusal left the item where it was too"
    );

    let live = new_run(ids::PROJECT_HTUI, ids::HTUI_FEAT_1, Vec::new());
    let live_id = live.id;
    let refused = store.create_run(live).await;
    assert!(
        matches!(refused, Err(StoreError::Constraint(_))),
        "{CASE}: in_progress does not reach queued (ANA-2 §4.3), got {refused:?}"
    );
    assert_eq!(
        store.run(live_id).await.expect(CASE),
        None,
        "{CASE}: the refusal wrote no run row — create_run is one transaction"
    );

    let retry = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_CLEAN_1, Vec::new()))
        .await
        .expect(CASE);
    assert_eq!(
        retry.status,
        RunStatus::Queued,
        "{CASE}: a failed item may be re-run"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_CLEAN_1).await.status,
        Status::Queued,
        "{CASE}: failed -> queued is the retry row of §4.3"
    );
}

/// ANA-2 §4.7's admission: the repo-scope overlap refuses before the slot count does, and the slot
/// count is the box rung of [`DEFAULT_MAX_CONCURRENT_ITEMS`].
///
/// The `MemStore` twin is `mem.rs::claim_run_refuses_an_overlapping_scope_and_a_full_box`; the
/// thing only Postgres can show — that two concurrent claims on the last slot cannot both win — is
/// `pg_criteria.rs::admission_is_serialised_by_the_box_row_lock`.
async fn claim_run_admits_one_and_refuses_the_second<S: WriteStore>(store: &S) {
    const CASE: &str = "claim_run_admits_one_and_refuses_the_second";
    let repo = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", true))
        .await
        .expect(CASE)
        .id;
    let third_item = store
        .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "Third"))
        .await
        .expect(CASE)
        .id;
    let fourth_item = store
        .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "Fourth"))
        .await
        .expect(CASE)
        .id;

    let mut queued = Vec::new();
    for (item, scope) in [
        (ids::HTUI_ANA_2, vec![repo]),
        (ids::HTUI_CLEAN_1, vec![repo]),
        (third_item, Vec::new()),
        (fourth_item, Vec::new()),
    ] {
        queued.push((
            item,
            store
                .create_run(new_run(ids::PROJECT_HTUI, item, scope))
                .await
                .expect(CASE)
                .id,
        ));
    }
    let [(_, first), (second_item, second), (_, third), (_, fourth)] = queued[..] else {
        panic!("{CASE}: four runs were queued")
    };

    let owner = Uuid::now_v7();
    let at = seam_clock();
    let until = at + TimeDelta::minutes(5);

    assert_eq!(
        store
            .claim_run(first, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: the first run is admitted"
    );
    let claimed = run_row(CASE, store, first).await;
    assert_eq!(
        claimed.status,
        RunStatus::Running,
        "{CASE}: the run started"
    );
    assert_eq!(
        claimed.executing_box_id,
        Some(ids::BOX),
        "{CASE}: the claiming box is recorded"
    );
    assert_eq!(
        claimed.started_at,
        Some(at),
        "{CASE}: `at` is the caller's clock"
    );
    assert_eq!(
        claimed.lease_box_id,
        Some(ids::BOX),
        "{CASE}: the lease names the box"
    );
    assert_eq!(
        claimed.lease_expires_at,
        Some(until),
        "{CASE}: the lease expires when the caller said"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_ANA_2).await.status,
        Status::InProgress,
        "{CASE}: the item moved queued -> in_progress with the claim"
    );

    assert_eq!(
        store
            .claim_run(second, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Overlaps {
            with: first,
            rule: OverlapRule::NotIsolated
        },
        "{CASE}: an overlapping repo_scope is refused while a slot is still free"
    );
    assert_eq!(
        run_row(CASE, store, second).await.status,
        RunStatus::Queued,
        "{CASE}: a refused claim writes nothing to the run"
    );
    assert_eq!(
        item_row(CASE, store, second_item).await.status,
        Status::Queued,
        "{CASE}: nor to its item"
    );

    assert_eq!(
        store
            .claim_run(third, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: an empty scope overlaps nothing (hazard H-10)"
    );
    assert_eq!(
        DEFAULT_MAX_CONCURRENT_ITEMS, 2,
        "{CASE}: the next leg reads as it does because the default is two"
    );
    assert_eq!(
        store
            .claim_run(fourth, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::SlotFull {
            running: 2,
            limit: 2
        },
        "{CASE}: the box is full at DEFAULT_MAX_CONCURRENT_ITEMS running runs"
    );
    // §4.7 draws the two predicates over two different sets, and `awaiting_approval` is where they
    // part: the slot count is `status = 'running'` alone, because "an `awaiting_approval` run
    // consumes no compute and must not hold a slot", while the overlap predicate "ranges over
    // non-terminal runs, including `awaiting_approval` ones, because a parked run still owns its
    // trees and its unmerged branch" (`docs/ANA-2.md` §4.7, invariant 6).
    assert!(
        store
            .transition_run(first, RunStatus::Running, RunStatus::AwaitingApproval, at)
            .await
            .expect(CASE),
        "{CASE}: the first run parks at a gate"
    );
    assert_eq!(
        store
            .claim_run(second, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Overlaps {
            with: first,
            rule: OverlapRule::NotIsolated
        },
        "{CASE}: a parked run still holds the trees of its repo_scope"
    );
    assert_eq!(
        store
            .claim_run(fourth, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: but it holds no slot, so the box that was full has one free"
    );

    assert_eq!(
        store
            .claim_run(first, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::NotClaimable,
        "{CASE}: a run that is no longer queued is not claimable"
    );

    let no_box = store
        .claim_run(second, BoxId::new(), owner, at, until)
        .await;
    assert!(
        matches!(no_box, Err(StoreError::NotFound { entity: "box", .. })),
        "{CASE}: an unknown box is NotFound, got {no_box:?}"
    );
    let no_run = store
        .claim_run(RunId::new(), BoxId::new(), owner, at, until)
        .await;
    assert!(
        matches!(no_run, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: the run is looked up before the box, got {no_run:?}"
    );
}

/// ANA-2 §4.7's rules L, I and P, as both stores decide them inside `claim_run` (plan D80, D83):
/// the verdict names the first overlapping live run in `(queued_at, id)` order and the rule that
/// fired, and a refusal writes nothing.
///
/// Criterion 15's parallel half is legs A and B (two isolated runs on disjoint paths of one repo
/// both run). Criterion 16's two halves are C–F and G–H: a **parked** run still refuses an
/// overlapping scope, and holds no slot, so two runs on another repo still fit beside it. The
/// two admitted runs are parked before the overlap legs because the fixture box has two slots
/// and `SlotFull` is decided before `Overlaps` (blueprint F-E); leg J pins that order, since it
/// both overlaps G and meets a full box. Every run gets its own `queued_at`, and B's is set before
/// A's although B's `RunId` is minted after, so "the first" in legs D–F rests on `queued_at` and
/// not on `RunId` order.
async fn claim_run_applies_the_isolation_and_path_rules<S: WriteStore>(store: &S) {
    const CASE: &str = "claim_run_applies_the_isolation_and_path_rules";
    let core = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", true))
        .await
        .expect(CASE)
        .id;
    let web = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "web", false))
        .await
        .expect(CASE)
        .id;

    let isolated = |prefixes: &[&str]| RepoScope {
        isolated: true,
        local: false,
        prefixes: prefixes.iter().map(|prefix| (*prefix).to_owned()).collect(),
    };
    let legs: [(char, Vec<(RepoId, RepoScope)>); 10] = [
        ('A', vec![(core, isolated(&["src/"]))]),
        ('B', vec![(core, isolated(&["docs/"]))]),
        ('C', vec![(core, isolated(&["src/lib/"]))]),
        ('D', vec![(core, RepoScope::default())]),
        (
            'E',
            vec![(
                core,
                RepoScope {
                    local: true,
                    ..RepoScope::default()
                },
            )],
        ),
        ('F', vec![(core, isolated(&[]))]),
        ('G', vec![(web, isolated(&["src/"]))]),
        ('H', vec![(web, isolated(&["docs/"]))]),
        ('I', Vec::new()),
        ('J', vec![(web, isolated(&["src/"]))]),
    ];

    let at = seam_clock();
    let mut runs = Vec::new();
    for (offset, (leg, repos)) in (0_i64..).zip(legs) {
        let item = store
            .mint_item(new_item(
                ids::PROJECT_HTUI,
                ids::KIND_HTUI_FEAT,
                &format!("Leg {leg}"),
            ))
            .await
            .expect(CASE)
            .id;
        let repo_scope: Vec<RepoId> = repos.iter().map(|(repo, _)| *repo).collect();
        let scope = (!repos.is_empty()).then(|| RunScope {
            repos: repos.into_iter().collect(),
        });
        let run = store
            .create_run(NewRun {
                graph_snapshot: GraphSnapshot {
                    scope,
                    ..run_snapshot()
                },
                // B is queued before A, against their `RunId` order (UUIDv7, minted in leg order).
                queued_at: at + TimeDelta::seconds(if leg == 'B' { -1 } else { offset }),
                ..new_run(ids::PROJECT_HTUI, item, repo_scope)
            })
            .await
            .expect(CASE)
            .id;
        runs.push((item, run));
    }
    let [
        (_, a),
        (_, b),
        (c_item, c),
        (_, d),
        (_, e),
        (_, f),
        (_, g),
        (_, h),
        (_, i),
        (_, j),
    ] = runs[..]
    else {
        panic!("{CASE}: ten runs were queued")
    };

    let owner = Uuid::now_v7();
    let claimed_at = at + TimeDelta::minutes(1);
    let until = claimed_at + TimeDelta::minutes(5);
    let claim = |run: RunId| store.claim_run(run, ids::BOX, owner, claimed_at, until);

    assert_eq!(
        claim(a).await.expect(CASE),
        Claim::Admitted,
        "{CASE}: A, isolated on core's src/, is alone on the box"
    );
    assert_eq!(
        claim(b).await.expect(CASE),
        Claim::Admitted,
        "{CASE}: B, isolated on core's docs/, runs beside A (criterion 15)"
    );
    for run in [a, b] {
        assert!(
            store
                .transition_run(
                    run,
                    RunStatus::Running,
                    RunStatus::AwaitingApproval,
                    claimed_at
                )
                .await
                .expect(CASE),
            "{CASE}: the admitted run parks at a gate"
        );
    }

    assert_eq!(
        claim(c).await.expect(CASE),
        Claim::Overlaps {
            with: a,
            rule: OverlapRule::Paths
        },
        "{CASE}: C's src/lib/ is inside parked A's src/ (rule P)"
    );
    assert_eq!(
        run_row(CASE, store, c).await.status,
        RunStatus::Queued,
        "{CASE}: a refused claim writes nothing to the run"
    );
    assert_eq!(
        item_row(CASE, store, c_item).await.status,
        Status::Queued,
        "{CASE}: nor to its item"
    );
    assert_eq!(
        claim(d).await.expect(CASE),
        Claim::Overlaps {
            with: b,
            rule: OverlapRule::NotIsolated
        },
        "{CASE}: D is not isolated on core (rule I) and overlaps A and B; B was queued first"
    );
    assert_eq!(
        claim(e).await.expect(CASE),
        Claim::Overlaps {
            with: b,
            rule: OverlapRule::Local
        },
        "{CASE}: E is local on core (rule L), and B is the first live run"
    );
    assert_eq!(
        claim(f).await.expect(CASE),
        Claim::Overlaps {
            with: b,
            rule: OverlapRule::Paths
        },
        "{CASE}: F declared no path, which is the whole repo, so B's docs/ is inside it"
    );

    assert_eq!(
        claim(g).await.expect(CASE),
        Claim::Admitted,
        "{CASE}: G is on web alone, and the parked runs hold no slot (criterion 16)"
    );
    assert_eq!(
        claim(h).await.expect(CASE),
        Claim::Admitted,
        "{CASE}: H is on web's docs/, disjoint from G's src/"
    );
    assert_eq!(
        DEFAULT_MAX_CONCURRENT_ITEMS, 2,
        "{CASE}: the next leg reads as it does because the default is two"
    );
    assert_eq!(
        claim(i).await.expect(CASE),
        Claim::SlotFull {
            running: 2,
            limit: 2
        },
        "{CASE}: I overlaps nothing, but G and H fill the box"
    );
    assert_eq!(
        run_row(CASE, store, i).await.status,
        RunStatus::Queued,
        "{CASE}: a full box writes nothing either"
    );
    assert_eq!(
        claim(j).await.expect(CASE),
        Claim::SlotFull {
            running: 2,
            limit: 2
        },
        "{CASE}: J overlaps G's src/ too, but the full box is decided first (blueprint F-E)"
    );
    assert_eq!(
        run_row(CASE, store, j).await.status,
        RunStatus::Queued,
        "{CASE}: and J stays queued"
    );
}

/// ANA-2 §4.9: the heartbeat is a compare-and-set on `lease_owner` and the sweep takes an expired
/// lease from whoever held it. The `MemStore` twin is
/// `mem.rs::a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it`.
///
/// `lease_owner` is deliberately not a [`Run`] field, so ownership is asserted only through what
/// `refresh_lease` answers (blueprint F-S).
async fn lease_refresh_is_a_cas_on_owner<S: WriteStore>(store: &S) {
    const CASE: &str = "lease_refresh_is_a_cas_on_owner";
    let first_owner = Uuid::now_v7();
    let second_owner = Uuid::now_v7();
    let at = seam_clock();
    let until = at + TimeDelta::minutes(5);
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(run, ids::BOX, first_owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: the run is admitted"
    );

    let extended = until + TimeDelta::minutes(5);
    assert!(
        store
            .refresh_lease(run, first_owner, extended)
            .await
            .expect(CASE),
        "{CASE}: the owner extends its own lease"
    );
    assert_eq!(
        run_row(CASE, store, run).await.lease_expires_at,
        Some(extended),
        "{CASE}: the new expiry is stored"
    );
    let stranger = extended + TimeDelta::minutes(5);
    assert!(
        !store
            .refresh_lease(run, second_owner, stranger)
            .await
            .expect(CASE),
        "{CASE}: a stranger's heartbeat is zero rows, which means abandon"
    );
    assert_eq!(
        run_row(CASE, store, run).await.lease_expires_at,
        Some(extended),
        "{CASE}: a refused heartbeat writes nothing"
    );
    let unknown = store
        .refresh_lease(RunId::new(), first_owner, extended)
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: an unknown run is NotFound, got {unknown:?}"
    );

    let swept = extended + TimeDelta::minutes(10);
    assert!(
        store
            .adopt_runs(
                ids::BOX,
                second_owner,
                extended - TimeDelta::seconds(1),
                swept,
            )
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: a live lease is not abandoned"
    );
    let adopted = store
        .adopt_runs(
            ids::BOX,
            second_owner,
            extended + TimeDelta::seconds(1),
            swept,
        )
        .await
        .expect(CASE);
    assert_eq!(
        adopted.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![run],
        "{CASE}: the expired lease is adopted"
    );
    assert_eq!(
        adopted.first().and_then(|row| row.lease_expires_at),
        Some(swept),
        "{CASE}: the adopted row carries the sweeper's expiry"
    );
    assert!(
        !store
            .refresh_lease(run, first_owner, swept)
            .await
            .expect(CASE),
        "{CASE}: the old owner has lost it"
    );
    assert!(
        store
            .refresh_lease(run, second_owner, swept)
            .await
            .expect(CASE),
        "{CASE}: and the sweeper holds it"
    );
    assert!(
        store
            .adopt_runs(
                ids::BOX,
                second_owner,
                swept + TimeDelta::seconds(1),
                swept + TimeDelta::minutes(5),
            )
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: a process never adopts its own lease, even expired (plan D88)"
    );
    assert_eq!(
        store
            .adopt_runs(
                ids::BOX,
                first_owner,
                swept + TimeDelta::seconds(1),
                swept + TimeDelta::minutes(5),
            )
            .await
            .expect(CASE)
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![run],
        "{CASE}: but a stranger's sweep adopts it"
    );
    assert!(
        store
            .adopt_runs(BoxId::new(), second_owner, swept, swept)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: a box that does not exist adopts nothing rather than refusing"
    );
}

/// Plan D87: `take_lease` moves a lease only when it is the taker's own or has expired, and only
/// on a `running` or `awaiting_approval` run executing on the taker's box.
///
/// `lease_owner` is not a [`Run`] field, so who holds the lease after each take is asserted
/// through what [`WriteStore::refresh_lease`] answers, as `lease_refresh_is_a_cas_on_owner` does.
async fn take_lease_moves_only_our_own_or_an_expired_lease<S: WriteStore>(store: &S) {
    const CASE: &str = "take_lease_moves_only_our_own_or_an_expired_lease";
    let (x, y, z) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let at = seam_clock();
    let minutes = |n: i64| at + TimeDelta::minutes(n);
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(run, ids::BOX, x, at, minutes(5))
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: X claims the run until at + 5 min"
    );

    assert!(
        !store
            .take_lease(run, ids::BOX, y, at, minutes(10))
            .await
            .expect(CASE),
        "{CASE}: X's lease is live, so Y cannot take it"
    );
    assert_eq!(
        run_row(CASE, store, run).await.lease_expires_at,
        Some(minutes(5)),
        "{CASE}: a refused take writes nothing"
    );
    assert!(
        store
            .take_lease(run, ids::BOX, x, at, minutes(10))
            .await
            .expect(CASE),
        "{CASE}: X renews its own live lease"
    );
    assert_eq!(
        run_row(CASE, store, run).await.lease_expires_at,
        Some(minutes(10)),
        "{CASE}: the renewal stores the new expiry"
    );
    assert!(
        store
            .take_lease(run, ids::BOX, y, minutes(11), minutes(20))
            .await
            .expect(CASE),
        "{CASE}: once X's lease has expired, Y takes it"
    );
    assert!(
        !store.refresh_lease(run, x, minutes(20)).await.expect(CASE),
        "{CASE}: X has lost it"
    );
    assert!(
        store.refresh_lease(run, y, minutes(20)).await.expect(CASE),
        "{CASE}: Y holds it"
    );

    assert!(
        store
            .transition_run(
                run,
                RunStatus::Running,
                RunStatus::AwaitingApproval,
                minutes(11)
            )
            .await
            .expect(CASE),
        "{CASE}: the run parks at a gate"
    );
    assert!(
        store.refresh_lease(run, y, minutes(11)).await.expect(CASE),
        "{CASE}: Y releases the lease at the park (lease_expires_at = now)"
    );
    assert!(
        store
            .take_lease(run, ids::BOX, z, minutes(11), minutes(30))
            .await
            .expect(CASE),
        "{CASE}: a released lease on a parked run is Z's to take"
    );
    let taken = run_row(CASE, store, run).await;
    assert_eq!(
        (taken.lease_box_id, taken.lease_expires_at),
        (Some(ids::BOX), Some(minutes(30))),
        "{CASE}: the take writes the box and the expiry"
    );
    assert!(
        !store
            .take_lease(run, BoxId::new(), z, minutes(40), minutes(50))
            .await
            .expect(CASE),
        "{CASE}: a run executing on another box is not takeable"
    );

    let queued_item = store
        .mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, "Queued"))
        .await
        .expect(CASE)
        .id;
    let queued = store
        .create_run(new_run(ids::PROJECT_HTUI, queued_item, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert!(
        !store
            .take_lease(queued, ids::BOX, z, at, minutes(5))
            .await
            .expect(CASE),
        "{CASE}: an unclaimed queued run has no lease to take"
    );
    assert_eq!(
        run_row(CASE, store, queued).await.lease_expires_at,
        None,
        "{CASE}: and the refusal wrote none"
    );

    let finished = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_CLEAN_1, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(finished, ids::BOX, x, at, minutes(5))
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: the second run is admitted"
    );
    store
        .finish_run(finished, RunStatus::Done, None, minutes(1))
        .await
        .expect(CASE);
    assert!(
        !store
            .take_lease(finished, ids::BOX, x, minutes(10), minutes(20))
            .await
            .expect(CASE),
        "{CASE}: a terminal run is not takeable, even by its own expired owner"
    );

    let unknown = store
        .take_lease(RunId::new(), ids::BOX, x, at, minutes(5))
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: an unknown run is NotFound, got {unknown:?}"
    );
}

/// Plan D139: `release_lease` is a compare-and-set on `lease_owner` that clears the owner. The
/// released run is then free to its own owner's sweep, which plan D88 refused while the owner
/// stayed on the row. A stranger's release writes nothing.
///
/// `lease_owner` is not a [`Run`] field, so who holds the lease is asserted through what
/// [`WriteStore::refresh_lease`] answers, as `lease_refresh_is_a_cas_on_owner` does.
async fn release_lease_frees_the_run_for_its_own_sweep<S: WriteStore>(store: &S) {
    const CASE: &str = "release_lease_frees_the_run_for_its_own_sweep";
    let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
    let at = seam_clock();
    let minutes = |n: i64| at + TimeDelta::minutes(n);
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(run, ids::BOX, x, at, minutes(5))
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: X claims the run until at + 5 min"
    );

    assert!(
        !store.release_lease(run, y, minutes(1)).await.expect(CASE),
        "{CASE}: Y does not hold the lease, so Y's release is zero rows"
    );
    assert_eq!(
        run_row(CASE, store, run).await.lease_expires_at,
        Some(minutes(5)),
        "{CASE}: a refused release writes nothing"
    );
    assert!(
        store.refresh_lease(run, x, minutes(5)).await.expect(CASE),
        "{CASE}: the lease is still X's"
    );

    assert!(
        store.release_lease(run, x, minutes(1)).await.expect(CASE),
        "{CASE}: X gives its own lease back"
    );
    let released = run_row(CASE, store, run).await;
    assert_eq!(
        (released.lease_box_id, released.lease_expires_at),
        (Some(ids::BOX), Some(minutes(1))),
        "{CASE}: the expiry reads `now`; the box that held the lease is kept"
    );
    assert!(
        !store.refresh_lease(run, x, minutes(10)).await.expect(CASE),
        "{CASE}: the owner is cleared, so X's late heartbeat matches no row"
    );
    assert!(
        !store.release_lease(run, x, minutes(1)).await.expect(CASE),
        "{CASE}: a second release finds nothing of X's"
    );

    let adopted = store
        .adopt_runs(ids::BOX, x, minutes(1), minutes(10))
        .await
        .expect(CASE);
    assert_eq!(
        adopted.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![run],
        "{CASE}: X's own sweep adopts the released run (plan D88 no longer applies)"
    );
    assert!(
        store.refresh_lease(run, x, minutes(10)).await.expect(CASE),
        "{CASE}: the adoption made the lease X's again"
    );

    let unknown = store.release_lease(RunId::new(), x, minutes(1)).await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: an unknown run is NotFound, got {unknown:?}"
    );
}

/// The fixture's one `box` row, as `load_demo` and `MemStore::from_demo` both load it.
fn fixture_box() -> BoxRow {
    crate::fixtures::demo_data()
        .boxes
        .into_iter()
        .find(|row| row.id == ids::BOX)
        .expect("the fixture has its box")
}

/// A fixed probe instant (MOD-7 D33): truncated to [`TIMESTAMPTZ_DIGITS`] so that Postgres, which
/// keeps microseconds, reads back the value `MemStore` keeps whole.
fn probe_clock(minutes: i64) -> DateTime<Utc> {
    (crate::fixtures::demo_at(10, 0)
        + TimeDelta::minutes(minutes)
        + TimeDelta::nanoseconds(123_456_789))
    .trunc_subsecs(TIMESTAMPTZ_DIGITS)
}

/// One tool of a probe.
fn probed_tool(name: &str, version: &str) -> ProbedTool {
    ProbedTool {
        name: name.to_owned(),
        version: version.to_owned(),
        path: format!("/opt/{name}/bin/{name}"),
    }
}

/// A probe of `box_id` at `at` with `tools` and a digest of `spec` (MOD-7 D33: a real sha256 hex,
/// so Postgres's `CHECK` admits it).
fn box_probe(box_id: BoxId, at: DateTime<Utc>, tools: Vec<ProbedTool>, spec: &str) -> BoxProbe {
    BoxProbe {
        box_id,
        os_version: format!("os {spec}"),
        cpu: format!("cpu {spec}"),
        ram_mb: Some(32_768),
        gpu_present: false,
        gpu_vendor: None,
        tools,
        probed_tags: vec!["cmake".to_owned(), format!("tag-{spec}")],
        htui_version: format!("9.9.{}", spec.len()),
        spec_digest: crate::prompt::digest::sha256_hex(spec),
        probed_at: at,
    }
}

/// MOD-7 D10, D33: `record_box_probe` writes the nine probe columns and **replaces** the box's
/// `box_tool` set, and touches none of the columns a person or registration owns. A probe that
/// repeats a tool name, or whose spec digest is not 64 lowercase hex, is a `Constraint` and writes
/// nothing.
async fn record_box_probe_replaces_profile_and_tools<S: WriteStore>(store: &S) {
    const CASE: &str = "record_box_probe_replaces_profile_and_tools";
    let fixture = fixture_box();
    let (at1, at2) = (probe_clock(0), probe_clock(90));

    store
        .record_box_probe(&box_probe(
            ids::BOX,
            at1,
            vec![probed_tool("a", "1.0"), probed_tool("b", "2.0")],
            "one",
        ))
        .await
        .expect(CASE);
    let second = box_probe(
        ids::BOX,
        at2,
        vec![probed_tool("b", "2.1"), probed_tool("c", "3.0")],
        "two",
    );
    store.record_box_probe(&second).await.expect(CASE);

    let records = store.boxes().await.expect(CASE);
    assert_eq!(records.len(), 1, "{CASE}: the fixture user has one box");
    let record = &records[0];
    let row = &record.row;
    assert_eq!(row.id, ids::BOX, "{CASE}: the probed box");
    assert_eq!(
        record
            .tools
            .iter()
            .map(|tool| (
                tool.box_id,
                tool.name.as_str(),
                tool.version.as_str(),
                tool.path.as_str(),
                tool.probed_at
            ))
            .collect::<Vec<_>>(),
        vec![
            (ids::BOX, "b", "2.1", "/opt/b/bin/b", at2),
            (ids::BOX, "c", "3.0", "/opt/c/bin/c", at2),
        ],
        "{CASE}: the second probe's tool set replaces the first's, and the fixture's"
    );
    assert_eq!(
        (
            row.os_version.as_str(),
            row.cpu.as_str(),
            row.ram_mb,
            row.gpu_present,
            row.gpu_vendor.as_deref(),
        ),
        (
            second.os_version.as_str(),
            second.cpu.as_str(),
            second.ram_mb,
            second.gpu_present,
            second.gpu_vendor.as_deref(),
        ),
        "{CASE}: every hardware column is the second probe's"
    );
    assert_eq!(
        row.probed_tags, second.probed_tags,
        "{CASE}: probed_tags is written"
    );
    assert_eq!(
        row.htui_version, second.htui_version,
        "{CASE}: htui_version is written"
    );
    assert_eq!(
        row.last_probed_at,
        Some(at2),
        "{CASE}: last_probed_at is the probe's instant"
    );
    assert_eq!(
        record.probe_spec_digest.as_deref(),
        Some(crate::prompt::digest::sha256_hex("two").as_str()),
        "{CASE}: the spec digest is the second probe's"
    );
    assert_eq!(
        (
            row.hostname.as_str(),
            &row.declared_tags,
            row.quirks.as_str(),
            &row.settings,
            row.registered_at,
            row.last_seen_at,
        ),
        (
            fixture.hostname.as_str(),
            &fixture.declared_tags,
            fixture.quirks.as_str(),
            &fixture.settings,
            fixture.registered_at,
            fixture.last_seen_at,
        ),
        "{CASE}: a probe never writes what a person or registration owns (MOD-2 D74)"
    );
    assert!(
        row.updated_at > fixture.updated_at,
        "{CASE}: updated_at moves, as the BEFORE UPDATE trigger moves it"
    );

    let before = store.boxes().await.expect(CASE);
    let twice = store
        .record_box_probe(&box_probe(
            ids::BOX,
            probe_clock(120),
            vec![probed_tool("b", "2.2"), probed_tool("b", "2.3")],
            "three",
        ))
        .await;
    assert!(
        matches!(twice, Err(StoreError::Constraint(_))),
        "{CASE}: a tool name listed twice is a Constraint, got {twice:?}"
    );
    assert_eq!(
        store.boxes().await.expect(CASE),
        before,
        "{CASE}: the refused probe wrote nothing"
    );

    let mut malformed = box_probe(
        ids::BOX,
        probe_clock(150),
        vec![probed_tool("d", "4.0")],
        "four",
    );
    malformed.spec_digest = "abc".to_owned();
    let malformed = store.record_box_probe(&malformed).await;
    assert!(
        matches!(malformed, Err(StoreError::Constraint(_))),
        "{CASE}: a digest that is not 64 lowercase hex is a Constraint, got {malformed:?}"
    );
    assert_eq!(
        store.boxes().await.expect(CASE),
        before,
        "{CASE}: the probe with a malformed digest wrote nothing"
    );
}

/// MOD-7 D10: a probe of a box nobody registered is `NotFound` and writes nothing, and the
/// unknown box wins over every `Constraint` the same probe would also hit - a malformed digest or
/// a repeated tool name - because `PgStore`'s `UPDATE .. WHERE id` finds no row before any
/// `CHECK` or key is consulted.
async fn record_box_probe_refuses_an_unknown_box<S: WriteStore>(store: &S) {
    const CASE: &str = "record_box_probe_refuses_an_unknown_box";
    let before = store.boxes().await.expect(CASE);
    let unknown = store
        .record_box_probe(&box_probe(
            BoxId::new(),
            probe_clock(0),
            vec![probed_tool("a", "1.0")],
            "one",
        ))
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "box", .. })),
        "{CASE}: an unknown box is NotFound, got {unknown:?}"
    );

    let mut malformed = box_probe(
        BoxId::new(),
        probe_clock(30),
        vec![probed_tool("a", "1.0")],
        "two",
    );
    malformed.spec_digest = "abc".to_owned();
    let malformed = store.record_box_probe(&malformed).await;
    assert!(
        matches!(malformed, Err(StoreError::NotFound { entity: "box", .. })),
        "{CASE}: an unknown box with a malformed digest is NotFound, not Constraint, got \
         {malformed:?}"
    );

    let twice = store
        .record_box_probe(&box_probe(
            BoxId::new(),
            probe_clock(60),
            vec![probed_tool("a", "1.0"), probed_tool("a", "1.1")],
            "three",
        ))
        .await;
    assert!(
        matches!(twice, Err(StoreError::NotFound { entity: "box", .. })),
        "{CASE}: an unknown box with a repeated tool name is NotFound, not Constraint, got \
         {twice:?}"
    );

    assert_eq!(
        store.boxes().await.expect(CASE),
        before,
        "{CASE}: the refused probes wrote nothing"
    );
}

/// MOD-7 D10, D18: `boxes` lists this user's boxes by id, each with its tools in name byte order
/// and its recorded spec digest, which the fixture never set.
async fn boxes_lists_every_box_with_its_tools<S: WriteStore>(store: &S) {
    const CASE: &str = "boxes_lists_every_box_with_its_tools";
    let records = store.boxes().await.expect(CASE);
    assert_eq!(records.len(), 1, "{CASE}: the fixture user has one box");
    let record = &records[0];
    assert_eq!(record.row.id, ids::BOX, "{CASE}: the fixture's box");
    assert_eq!(record.row.hostname, "DESKTOP-HTUI", "{CASE}: its hostname");
    assert_eq!(
        record.row.probed_tags,
        ["rust", "msvc", "cmake"],
        "{CASE}: its probed tags, in stored order"
    );
    assert_eq!(
        record.row.declared_tags,
        ["gpu"],
        "{CASE}: its declared tags"
    );
    assert_eq!(
        record
            .tools
            .iter()
            .map(|tool| (
                tool.box_id,
                tool.name.as_str(),
                tool.version.as_str(),
                tool.path.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (ids::BOX, "cargo", "1.98.0", "/usr/bin/cargo"),
            (ids::BOX, "cmake", "", "/usr/bin/cmake"),
            (ids::BOX, "git", "2.51.0", "/usr/bin/git"),
            (ids::BOX, "rustc", "1.98.0", "/usr/bin/rustc"),
        ],
        "{CASE}: the fixture's tools, in name byte order"
    );
    assert_eq!(
        record.probe_spec_digest, None,
        "{CASE}: the fixture never recorded a spec digest"
    );
}

/// Plan D89: `interrupt_step` is a compare-and-set on `running`. It writes `failed`, the note and
/// the first `finished_at`, and leaves `gate_outcome` `NULL`, because a crash is not a gate
/// answer. On any other status it writes nothing at all.
async fn interrupt_step_is_a_cas_on_running<S: WriteStore>(store: &S) {
    const CASE: &str = "interrupt_step_is_a_cas_on_running";
    let at = seam_clock();
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    let running = |position: i32| {
        let spec = new_run_step(run, position, 1, 0);
        let id = spec.id;
        async move {
            store.create_step(spec).await.expect(CASE);
            assert!(
                store
                    .transition_step(id, StepStatus::Pending, StepStatus::Running, at)
                    .await
                    .expect(CASE),
                "{CASE}: pending -> running is a sanctioned move"
            );
            id
        }
    };

    let step = running(0).await;
    let later = at + TimeDelta::minutes(1);
    assert!(
        store
            .interrupt_step(step, "interrupted", later)
            .await
            .expect(CASE),
        "{CASE}: a running step is interrupted"
    );
    let interrupted = step_row(CASE, store, run, step).await;
    assert_eq!(
        (
            interrupted.status,
            interrupted.gate_note.as_deref(),
            interrupted.gate_outcome,
            interrupted.finished_at,
        ),
        (StepStatus::Failed, Some("interrupted"), None, Some(later)),
        "{CASE}: failed, the note, no gate outcome, and the caller's instant"
    );
    assert!(
        !store
            .interrupt_step(step, "interrupted", later + TimeDelta::minutes(1))
            .await
            .expect(CASE),
        "{CASE}: a second interruption finds the step no longer running"
    );
    assert_eq!(
        step_row(CASE, store, run, step).await,
        interrupted,
        "{CASE}: and writes nothing"
    );

    let settled = running(1).await;
    store
        .finish_step(
            settled,
            StepOutcome {
                exit_code: Some(0),
                finished_at: at,
                ..StepOutcome::default()
            },
        )
        .await
        .expect(CASE);
    assert!(
        store
            .interrupt_step(settled, "interrupted", later)
            .await
            .expect(CASE),
        "{CASE}: a settled but still running step is interrupted too"
    );
    assert_eq!(
        step_row(CASE, store, run, settled).await.finished_at,
        Some(at),
        "{CASE}: the earlier finished_at is kept (COALESCE)"
    );

    let pending = new_run_step(run, 2, 1, 0);
    let pending_id = pending.id;
    store.create_step(pending).await.expect(CASE);
    let parked = gated_step(CASE, store, new_run_step(run, 3, 1, 0)).await;
    let done = running(4).await;
    assert!(
        store
            .transition_step(done, StepStatus::Running, StepStatus::Done, at)
            .await
            .expect(CASE),
        "{CASE}: running -> done is a sanctioned move"
    );
    for (label, id) in [
        ("pending", pending_id),
        ("awaiting_approval", parked),
        ("done", done),
    ] {
        let before = step_row(CASE, store, run, id).await;
        assert!(
            !store
                .interrupt_step(id, "interrupted", later)
                .await
                .expect(CASE),
            "{CASE}: a {label} step is not running"
        );
        assert_eq!(
            step_row(CASE, store, run, id).await,
            before,
            "{CASE}: the {label} row is untouched"
        );
    }

    let unknown = store
        .interrupt_step(StepId::new(), "interrupted", later)
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an unknown step is NotFound, got {unknown:?}"
    );
}

/// `create_step`, the two compare-and-sets and `supersede_step`, with the order `run_steps`
/// answers in: the judge sorts before its candidates. The `MemStore` twin is
/// `mem.rs::step_creation_and_the_two_compare_and_sets_follow_the_law`.
async fn step_create_and_transition_law<S: WriteStore>(store: &S) {
    const CASE: &str = "step_create_and_transition_law";
    let request = new_run_step(ids::RUN_2, 1, 1, 0);
    let step = request.id;
    let created = store.create_step(request).await.expect(CASE);
    assert_eq!(
        created.status,
        StepStatus::Pending,
        "{CASE}: a step starts pending"
    );
    assert_eq!(
        (
            created.started_at,
            created.finished_at,
            created.selected,
            created.gate_outcome,
            created.exit_code,
            created.verify_outcome,
            created.promoted_at,
        ),
        (None, None, None, None, None, None, None),
        "{CASE}: every settle column is NULL on a fresh step"
    );
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, step).await,
        created,
        "{CASE}: the row reads back as `create_step` returned it"
    );

    let repeat = store.create_step(new_run_step(ids::RUN_2, 0, 1, 0)).await;
    assert!(
        matches!(repeat, Err(StoreError::Constraint(_))),
        "{CASE}: (run_id, position, attempt, fanout_index) is unique, got {repeat:?}"
    );
    let orphan = store.create_step(new_run_step(RunId::new(), 9, 1, 0)).await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown run is a foreign key refusal, got {orphan:?}"
    );

    let judge = new_run_step(ids::RUN_2, 1, 1, -1);
    let judge_id = judge.id;
    store.create_step(judge).await.expect(CASE);
    assert_eq!(
        store
            .run_steps(ids::RUN_2)
            .await
            .expect(CASE)
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![ids::STEP_R2_PRD, judge_id, step],
        "{CASE}: (position, attempt, fanout_index) order puts the judge first"
    );

    let started = seam_clock();
    let finished = started + TimeDelta::minutes(1);
    assert!(
        store
            .transition_step(step, StepStatus::Pending, StepStatus::Running, started)
            .await
            .expect(CASE),
        "{CASE}: pending -> running matches"
    );
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, step).await.started_at,
        Some(started),
        "{CASE}: the move to running stamps started_at"
    );
    assert!(
        store
            .transition_step(step, StepStatus::Running, StepStatus::Done, finished)
            .await
            .expect(CASE),
        "{CASE}: running -> done matches"
    );
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, step).await.finished_at,
        Some(finished),
        "{CASE}: a terminal move stamps finished_at"
    );

    let settled = step_row(CASE, store, ids::RUN_2, step).await;
    let illegal = store
        .transition_step(step, StepStatus::Done, StepStatus::Running, finished)
        .await;
    assert!(
        matches!(illegal, Err(StoreError::Constraint(_))),
        "{CASE}: done reaches only superseded, got {illegal:?}"
    );
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, step).await,
        settled,
        "{CASE}: a refused pair leaves the row byte-identical"
    );
    assert!(
        !store
            .transition_step(step, StepStatus::Pending, StepStatus::Running, finished)
            .await
            .expect(CASE),
        "{CASE}: a stale `from` on a legal pair is Ok(false)"
    );

    assert!(
        store
            .transition_run(ids::RUN_2, RunStatus::Queued, RunStatus::Running, started)
            .await
            .expect(CASE),
        "{CASE}: queued -> running matches"
    );
    assert_eq!(
        run_row(CASE, store, ids::RUN_2).await.started_at,
        Some(started),
        "{CASE}: the run's move to running stamps started_at"
    );
    assert!(
        store
            .transition_run(ids::RUN_2, RunStatus::Running, RunStatus::Done, finished)
            .await
            .expect(CASE),
        "{CASE}: running -> done matches"
    );
    assert_eq!(
        run_row(CASE, store, ids::RUN_2).await.finished_at,
        Some(finished),
        "{CASE}: a terminal run move stamps finished_at"
    );
    let terminal = store
        .transition_run(ids::RUN_2, RunStatus::Done, RunStatus::Queued, finished)
        .await;
    assert!(
        matches!(terminal, Err(StoreError::Constraint(_))),
        "{CASE}: a terminal run reaches nothing, got {terminal:?}"
    );

    let fresh = new_run_step(ids::RUN_2, 2, 1, 0);
    let fresh_id = fresh.id;
    store.create_step(fresh).await.expect(CASE);
    store.supersede_step(fresh_id).await.expect(CASE);
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, fresh_id).await.status,
        StepStatus::Superseded,
        "{CASE}: pending -> superseded is §4.4's loop half"
    );
    store.supersede_step(step).await.expect(CASE);
    let running = new_run_step(ids::RUN_2, 3, 1, 0);
    let running_id = running.id;
    store.create_step(running).await.expect(CASE);
    assert!(
        store
            .transition_step(
                running_id,
                StepStatus::Pending,
                StepStatus::Running,
                started
            )
            .await
            .expect(CASE),
        "{CASE}: the fourth step starts"
    );
    let live = store.supersede_step(running_id).await;
    assert!(
        matches!(live, Err(StoreError::Constraint(_))),
        "{CASE}: running does not reach superseded, got {live:?}"
    );
    let gone = store.supersede_step(StepId::new()).await;
    assert!(
        matches!(
            gone,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an unknown step is NotFound, got {gone:?}"
    );

    assert_eq!(
        item_row(CASE, store, ids::HTUI_FEAT_3).await.status,
        Status::Queued,
        "{CASE}: no run or step move touches the item — only claim_run, promote_step, create_run \
         and close_out do"
    );
}

/// `finish_step` writes the settle columns and never `status`; `usage` and `trim_record` `None`
/// leave the column, every other field overwrites. The `MemStore` twin is
/// `mem.rs::finish_step_settles_the_columns_and_leaves_usage_when_it_is_none`.
///
/// Leg (d) is the writer-side half of the three-builder pin: what `finish_step` stores has to come
/// back through the `RunStepSummary` the Runs sub-tab reads, on every backend.
async fn finish_step_records_the_settle<S: WriteStore>(store: &S) {
    const CASE: &str = "finish_step_records_the_settle";
    let finished = seam_clock();
    store
        .finish_step(
            ids::STEP_R2_PRD,
            StepOutcome {
                exit_code: Some(0),
                usage: Some(json!({ "input_tokens": 3 })),
                trim_record: Some(json!({ "estimated_after": 11, "sections": [] })),
                verify_outcome: Some(VerifyOutcome::Pass),
                verify_exit_code: Some(0),
                finished_at: finished,
            },
        )
        .await
        .expect(CASE);
    let settled = step_row(CASE, store, ids::RUN_2, ids::STEP_R2_PRD).await;
    assert_eq!(
        settled.status,
        StepStatus::Pending,
        "{CASE}: finish_step never moves status"
    );
    assert_eq!(settled.exit_code, Some(0), "{CASE}: exit_code is written");
    assert_eq!(
        settled.verify_outcome,
        Some(VerifyOutcome::Pass),
        "{CASE}: verify_outcome is written"
    );
    assert_eq!(
        settled.verify_exit_code,
        Some(0),
        "{CASE}: so is its exit code"
    );
    assert_eq!(
        settled.finished_at,
        Some(finished),
        "{CASE}: and the caller's clock"
    );

    store
        .finish_step(
            ids::STEP_R2_PRD,
            StepOutcome {
                exit_code: Some(1),
                finished_at: finished,
                ..StepOutcome::default()
            },
        )
        .await
        .expect(CASE);
    let second = step_row(CASE, store, ids::RUN_2, ids::STEP_R2_PRD).await;
    assert_eq!(second.exit_code, Some(1), "{CASE}: exit_code overwrites");
    assert_eq!(
        second.usage,
        Some(json!({ "input_tokens": 3 })),
        "{CASE}: a None usage leaves the column the summer wrote"
    );
    assert_eq!(
        second.trim_record,
        Some(json!({ "estimated_after": 11, "sections": [] })),
        "{CASE}: a None trim_record leaves the column the assembler wrote"
    );
    assert_eq!(
        second.verify_outcome, None,
        "{CASE}: verify_outcome is not one of the two that leave"
    );

    let unknown = store
        .finish_step(StepId::new(), StepOutcome::default())
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an unknown step is NotFound, got {unknown:?}"
    );

    let summary = store
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect(CASE)
        .into_iter()
        .find(|row| row.id == ids::RUN_2)
        .unwrap_or_else(|| panic!("{CASE}: FEAT-3 owns RUN_2"));
    let listed = summary
        .steps
        .iter()
        .find(|row| row.id == ids::STEP_R2_PRD)
        .unwrap_or_else(|| panic!("{CASE}: RUN_2's summary lists its step"));
    assert_eq!(
        (
            listed.usage.clone(),
            listed.exit_code,
            listed.verify_outcome,
            listed.selected,
            listed.promoted_at,
        ),
        (
            second.usage.clone(),
            second.exit_code,
            second.verify_outcome,
            second.selected,
            second.promoted_at,
        ),
        "{CASE}: the summary projection and the row agree column for column"
    );
    assert_eq!(
        listed.agent_name.as_deref(),
        Some("claude"),
        "{CASE}: agent_name is denormalised from agent_id on every backend"
    );
}

/// `R-ORCH-2`'s four answers, §4.8's promotion — which lifts the step, its run and its item in one
/// transaction — and the refusal note of ANA-2 invariant 7. The `MemStore` twins are
/// `mem.rs::gate_answers_write_their_outcome_and_promotion_lifts_the_run_and_the_item` and
/// `mem.rs::add_note_writes_the_row_and_refuses_every_dangling_reference`.
async fn gate_answers_write_their_outcome<S: WriteStore>(store: &S) {
    const CASE: &str = "gate_answers_write_their_outcome";
    let at = seam_clock();
    let approved = gated_step(CASE, store, new_run_step(ids::RUN_2, 1, 1, 0)).await;
    let rejected = gated_step(CASE, store, new_run_step(ids::RUN_2, 2, 1, 0)).await;
    let retried = gated_step(CASE, store, new_run_step(ids::RUN_2, 3, 1, 0)).await;
    let skipped = gated_step(CASE, store, new_run_step(ids::RUN_2, 4, 1, 0)).await;

    assert!(
        store
            .answer_gate(
                approved,
                GateOutcome::Approved,
                Some("looks right".to_owned()),
                at
            )
            .await
            .expect(CASE),
        "{CASE}: an awaiting step takes its answer"
    );
    let row = step_row(CASE, store, ids::RUN_2, approved).await;
    assert_eq!(row.status, StepStatus::Done, "{CASE}: approved -> done");
    assert_eq!(
        row.gate_outcome,
        Some(GateOutcome::Approved),
        "{CASE}: the answer is recorded"
    );
    assert_eq!(
        row.gate_note.as_deref(),
        Some("looks right"),
        "{CASE}: so is the note"
    );
    assert_eq!(row.finished_at, Some(at), "{CASE}: and the caller's clock");
    assert!(
        !store
            .answer_gate(approved, GateOutcome::Approved, None, at)
            .await
            .expect(CASE),
        "{CASE}: a step that is no longer awaiting cannot be answered twice"
    );

    for (step, outcome, expected) in [
        (rejected, GateOutcome::Rejected, StepStatus::Failed),
        (retried, GateOutcome::Retried, StepStatus::Superseded),
        (skipped, GateOutcome::Skipped, StepStatus::Done),
    ] {
        assert!(
            store
                .answer_gate(step, outcome, None, at)
                .await
                .expect(CASE),
            "{CASE}: {outcome} is answerable"
        );
        assert_eq!(
            step_row(CASE, store, ids::RUN_2, step).await.status,
            expected,
            "{CASE}: {outcome} settles the step at {expected}"
        );
    }
    let unknown = store
        .answer_gate(StepId::new(), GateOutcome::Approved, None, at)
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an unknown step is NotFound, got {unknown:?}"
    );

    assert!(
        store
            .transition_run(ids::RUN_2, RunStatus::Queued, RunStatus::Running, at)
            .await
            .expect(CASE),
        "{CASE}: the run starts"
    );
    assert!(
        store
            .transition(ids::HTUI_FEAT_3, Status::Queued, Status::InProgress)
            .await
            .expect(CASE),
        "{CASE}: and so does its item"
    );
    store.promote_step(rejected, at).await.expect(CASE);
    let promoted = step_row(CASE, store, ids::RUN_2, rejected).await;
    assert_eq!(
        promoted.status,
        StepStatus::AwaitingApproval,
        "{CASE}: a failed step under a live run is promotable (§4.8)"
    );
    assert_eq!(
        promoted.promoted_at,
        Some(at),
        "{CASE}: promoted_at is stamped"
    );
    assert_eq!(
        run_row(CASE, store, ids::RUN_2).await.status,
        RunStatus::AwaitingApproval,
        "{CASE}: the run is lifted with the step"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_FEAT_3).await.status,
        Status::AwaitingApproval,
        "{CASE}: and so is the item, in the same transaction"
    );
    let settled = store.promote_step(approved, at).await;
    assert!(
        matches!(settled, Err(StoreError::Constraint(_))),
        "{CASE}: a done step is not promotable, got {settled:?}"
    );

    let written = store
        .add_note(new_note(ids::HTUI_FEAT_3, ids::USER, Some(rejected)))
        .await
        .expect(CASE);
    assert_eq!(
        store.notes(ids::HTUI_FEAT_3).await.expect(CASE).last(),
        Some(&written),
        "{CASE}: the returned note is the stored note"
    );
    let no_author = store
        .add_note(new_note(ids::HTUI_FEAT_3, UserId::new(), None))
        .await;
    assert!(
        matches!(no_author, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown author is a foreign key refusal, got {no_author:?}"
    );
    let no_item = store
        .add_note(new_note(ItemId::new(), ids::USER, None))
        .await;
    assert!(
        matches!(no_item, Err(StoreError::Constraint(_))),
        "{CASE}: so is an unknown item, got {no_item:?}"
    );
}

/// ANA-2 §4.5's bookkeeping is one transaction (plan D6): a refused winner leaves every candidate
/// exactly as it was, and the selection is what `resolve_inputs` reads afterwards.
///
/// The `MemStore` twins are `mem.rs::select_fanout_settles_every_candidate_or_none` and
/// `mem.rs::resolve_inputs_prefers_this_run_and_skips_a_loser`.
async fn select_fanout_is_one_transaction<S: WriteStore>(store: &S) {
    const CASE: &str = "select_fanout_is_one_transaction";
    let at = seam_clock();
    let winner = gated_step(CASE, store, new_run_step(ids::RUN_2, 1, 1, 0)).await;
    let loser = gated_step(CASE, store, new_run_step(ids::RUN_2, 1, 1, 1)).await;
    let failed = gated_step(CASE, store, new_run_step(ids::RUN_2, 1, 1, 2)).await;
    let elsewhere = gated_step(CASE, store, new_run_step(ids::RUN_2, 2, 1, 0)).await;
    let judge = new_run_step(ids::RUN_2, 1, 1, -1);
    let judge_id = judge.id;
    store.create_step(judge).await.expect(CASE);
    assert!(
        store
            .transition_step(judge_id, StepStatus::Pending, StepStatus::Running, at)
            .await
            .expect(CASE),
        "{CASE}: the judge runs"
    );
    assert!(
        store
            .answer_gate(failed, GateOutcome::Rejected, None, at)
            .await
            .expect(CASE),
        "{CASE}: the third candidate fails"
    );

    let before = step_row(CASE, store, ids::RUN_2, loser).await;
    let wrong_slot = store.select_fanout(ids::RUN_2, 1, 1, elsewhere, None).await;
    assert!(
        matches!(wrong_slot, Err(StoreError::Constraint(_))),
        "{CASE}: a winner from another position is refused, got {wrong_slot:?}"
    );
    let absent = store
        .select_fanout(ids::RUN_2, 1, 1, StepId::new(), None)
        .await;
    assert!(
        matches!(
            absent,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an unknown winner is NotFound, got {absent:?}"
    );
    assert_eq!(
        step_row(CASE, store, ids::RUN_2, loser).await,
        before,
        "{CASE}: a refused selection writes nothing at all"
    );

    let by_winner = store
        .write_document(new_document(
            ids::HTUI_FEAT_3,
            "implementation",
            Some(winner),
        ))
        .await
        .expect(CASE);
    let by_loser = store
        .write_document(new_document(
            ids::HTUI_FEAT_3,
            "implementation",
            Some(loser),
        ))
        .await
        .expect(CASE);
    assert_eq!(
        (by_winner.version, by_loser.version),
        (1, 2),
        "{CASE}: the loser holds the higher version, which is what makes (b) a real contrast"
    );

    store
        .select_fanout(ids::RUN_2, 1, 1, winner, Some("shorter diff".to_owned()))
        .await
        .expect(CASE);
    let settled = |id| step_row(CASE, store, ids::RUN_2, id);
    let won = settled(winner).await;
    assert_eq!(
        (won.selected, won.status),
        (Some(true), StepStatus::Done),
        "{CASE}: the winner is selected and done"
    );
    let lost = settled(loser).await;
    assert_eq!(
        (lost.selected, lost.status),
        (Some(false), StepStatus::Superseded),
        "{CASE}: an awaiting loser is superseded"
    );
    let rejected = settled(failed).await;
    assert_eq!(
        (rejected.selected, rejected.status),
        (Some(false), StepStatus::Failed),
        "{CASE}: a failed loser is marked but keeps its status"
    );
    let arbiter = settled(judge_id).await;
    assert_eq!(
        (arbiter.status, arbiter.gate_note.as_deref()),
        (StepStatus::Done, Some("shorter diff")),
        "{CASE}: the judge is settled with the reason"
    );
    assert_eq!(
        settled(elsewhere).await.selected,
        None,
        "{CASE}: another position is not part of this fan-out"
    );

    let resolved = store
        .resolve_inputs(ids::HTUI_FEAT_3, ids::RUN_2, &["implementation".to_owned()])
        .await
        .expect(CASE);
    assert_eq!(
        resolved
            .first()
            .and_then(|row| row.document.as_ref())
            .map(|row| row.id),
        Some(by_winner.id),
        "{CASE}: `selected IS NOT FALSE` skips the loser's higher version"
    );
    assert_eq!(
        store
            .documents_of_kinds(ids::HTUI_FEAT_3, &["implementation".to_owned()])
            .await
            .expect(CASE)
            .first()
            .map(|row| row.id),
        Some(by_loser.id),
        "{CASE}: documents_of_kinds excludes no loser — plan D2's contrast"
    );

    // §4.5's judge-failure path (`docs/ANA-2.md:858-862`): the judge failed, its `gate_note` names
    // why, the run parked at `awaiting_approval` and a human picks. That pick is this method, so
    // the selection may not move the judge `failed -> done` — a pair `StepStatus::can_move_to`
    // rejects — nor replace the reason with its own, which on the human path is `None`. "Nothing
    // is lost" is the sentence being asserted. A second slot because a slot holds one judge.
    let picked = gated_step(CASE, store, new_run_step(ids::RUN_2, 3, 1, 0)).await;
    let beaten = gated_step(CASE, store, new_run_step(ids::RUN_2, 3, 1, 1)).await;
    let broken = gated_step(CASE, store, new_run_step(ids::RUN_2, 3, 1, -1)).await;
    assert!(
        store
            .answer_gate(
                broken,
                GateOutcome::Rejected,
                Some("the verdict block did not parse".to_owned()),
                at,
            )
            .await
            .expect(CASE),
        "{CASE}: the judge fails with the reason in its `gate_note`"
    );

    store
        .select_fanout(ids::RUN_2, 3, 1, picked, None)
        .await
        .expect(CASE);
    let arbiter = settled(broken).await;
    assert_eq!(
        (arbiter.status, arbiter.gate_note.as_deref()),
        (StepStatus::Failed, Some("the verdict block did not parse")),
        "{CASE}: a judge that failed keeps the status and the reason the human picked from"
    );
    assert_eq!(
        (settled(picked).await.selected, settled(picked).await.status),
        (Some(true), StepStatus::Done),
        "{CASE}: the human's winner is settled all the same"
    );
    assert_eq!(
        settled(beaten).await.status,
        StepStatus::Superseded,
        "{CASE}: and so is its loser"
    );

    let listed = store
        .runs(ids::HTUI_FEAT_3)
        .await
        .expect(CASE)
        .into_iter()
        .find(|row| row.id == ids::RUN_2)
        .unwrap_or_else(|| panic!("{CASE}: FEAT-3 owns RUN_2"));
    assert_eq!(
        listed
            .steps
            .iter()
            .find(|row| row.id == winner)
            .and_then(|row| row.selected),
        Some(true),
        "{CASE}: the Runs sub-tab's projection carries the verdict"
    );
}

/// `run_step_tree` and `run_step_commit` upsert on `(run_step_id, repo_id)`, read back in
/// `repo_id` order, and are counted by a project delete — two tables `MemStore` never held before
/// MOD-4 (plan D12). The `MemStore` twin is
/// `mem.rs::trees_and_commits_upsert_on_their_repo_key_and_the_delete_counts_them`; the cascade
/// itself is `pg_criteria.rs::step_tree_rows_cascade_with_their_step` on Postgres.
///
/// The same call also writes `run_step.isolation_path` (ANA-2 `:903`, plan D33), which is why the
/// two repos here are given paths that differ: the column carries **one** path, so the batch has
/// to choose, and a fixture where both rows say the same thing would pass whichever choice the
/// store made. The rule is the primary repo's path, else the lowest `repo_id`'s — the order
/// `step_trees` answers in — and an empty batch leaves the column alone. Read here through
/// `run_steps`, which is the only projection that carries it; the column itself, and the
/// `updated_at` trigger the write has to fire, are
/// `pg_criteria.rs::upsert_step_tree_writes_the_primary_isolation_path`.
async fn trees_and_commits_round_trip<S: WriteStore>(store: &S) {
    const CASE: &str = "trees_and_commits_round_trip";
    let core = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", true))
        .await
        .expect(CASE)
        .id;
    let docs = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "docs", false))
        .await
        .expect(CASE)
        .id;
    let (first, second) = if core < docs {
        (core, docs)
    } else {
        (docs, core)
    };

    let tree = |repo: RepoId, dirty: bool| RunStepTree {
        run_step_id: ids::STEP_R2_PRD,
        repo_id: repo,
        mode: Isolation::Worktree,
        path: format!(
            "/srv/trees/prd/{}",
            if repo == core { "core" } else { "docs" }
        ),
        base_ref: "main".to_owned(),
        dirty,
    };
    // `STEP_R2_PRD` belongs to the fixture's `RUN_2`; `run_steps` is the only read that carries
    // `run_step.isolation_path` through the seam.
    let isolation_path = async || -> Option<String> {
        store
            .run_steps(ids::RUN_2)
            .await
            .expect(CASE)
            .into_iter()
            .find(|row| row.id == ids::STEP_R2_PRD)
            .unwrap_or_else(|| panic!("{CASE}: RUN_2 carries STEP_R2_PRD"))
            .isolation_path
    };
    assert_eq!(
        isolation_path().await,
        None,
        "{CASE}: nothing has written the column yet"
    );

    store
        .upsert_step_tree(ids::STEP_R2_PRD, &[tree(second, false), tree(first, false)])
        .await
        .expect(CASE);
    assert_eq!(
        store
            .step_trees(ids::STEP_R2_PRD)
            .await
            .expect(CASE)
            .iter()
            .map(|row| row.repo_id)
            .collect::<Vec<_>>(),
        vec![first, second],
        "{CASE}: repo_id order regardless of input order"
    );
    assert_eq!(
        isolation_path().await,
        Some("/srv/trees/prd/core".to_owned()),
        "{CASE}: `core` is the primary repo, so its path is the step's (plan D33)"
    );

    store
        .upsert_step_tree(ids::STEP_R2_PRD, &[tree(first, true)])
        .await
        .expect(CASE);
    assert_eq!(
        isolation_path().await,
        Some(tree(first, true).path),
        "{CASE}: a one-row batch chooses that row, primary or not"
    );
    let trees = store.step_trees(ids::STEP_R2_PRD).await.expect(CASE);
    assert_eq!(
        trees.len(),
        2,
        "{CASE}: the upsert replaced rather than inserted"
    );
    assert!(
        trees.first().is_some_and(|row| row.dirty),
        "{CASE}: and it replaced on the key"
    );

    let stray = RunStepTree {
        run_step_id: ids::STEP_PLAN,
        ..tree(first, false)
    };
    let foreign = store.upsert_step_tree(ids::STEP_R2_PRD, &[stray]).await;
    assert!(
        matches!(foreign, Err(StoreError::Constraint(_))),
        "{CASE}: a row naming another step is refused, got {foreign:?}"
    );
    let no_repo = store
        .upsert_step_tree(ids::STEP_R2_PRD, &[tree(RepoId::new(), false)])
        .await;
    assert!(
        matches!(no_repo, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown repo is a foreign key refusal, got {no_repo:?}"
    );
    assert_eq!(
        store.step_trees(ids::STEP_R2_PRD).await.expect(CASE).len(),
        2,
        "{CASE}: neither refusal wrote a row"
    );
    let no_step = store.upsert_step_tree(StepId::new(), &[]).await;
    assert!(
        matches!(
            no_step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an empty slice still checks the step, got {no_step:?}"
    );
    // Plan D14 once more, on the combination the two legs above never reach together: the step is
    // looked up before the batch is judged, so an unknown step carrying a stray row is `NotFound`
    // and not the `Constraint` the row alone would earn.
    let stray_on_no_step = store
        .upsert_step_tree(
            StepId::new(),
            &[RunStepTree {
                run_step_id: ids::STEP_PLAN,
                ..tree(first, false)
            }],
        )
        .await;
    assert!(
        matches!(
            stray_on_no_step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: D14 — the step is looked up before the batch is judged, got {stray_on_no_step:?}"
    );
    store
        .upsert_step_tree(ids::STEP_R2_PRD, &[])
        .await
        .expect(CASE);
    assert_eq!(
        isolation_path().await,
        Some(tree(first, true).path),
        "{CASE}: an empty batch names no tree, so it leaves the column alone (plan D33)"
    );

    let commit = |repo: RepoId, after: Option<&str>| RunStepCommit {
        run_step_id: ids::STEP_R2_PRD,
        repo_id: repo,
        before_hash: "0000000000000000000000000000000000000000".to_owned(),
        after_hash: after.map(ToOwned::to_owned),
    };
    store
        .record_commits(
            ids::STEP_R2_PRD,
            &[commit(second, None), commit(first, None)],
        )
        .await
        .expect(CASE);
    store
        .record_commits(ids::STEP_R2_PRD, &[commit(first, Some("deadbeef"))])
        .await
        .expect(CASE);
    let commits = store.step_commits(ids::STEP_R2_PRD).await.expect(CASE);
    assert_eq!(
        commits.iter().map(|row| row.repo_id).collect::<Vec<_>>(),
        vec![first, second],
        "{CASE}: repo_id order, and still two rows"
    );
    assert_eq!(
        commits.first().and_then(|row| row.after_hash.as_deref()),
        Some("deadbeef"),
        "{CASE}: after_hash None then Some is an update, not a second row"
    );
    let stray_commit = RunStepCommit {
        run_step_id: ids::STEP_PLAN,
        ..commit(first, None)
    };
    let foreign_commit = store
        .record_commits(ids::STEP_R2_PRD, &[stray_commit])
        .await;
    assert!(
        matches!(foreign_commit, Err(StoreError::Constraint(_))),
        "{CASE}: a commit row naming another step is refused, got {foreign_commit:?}"
    );
    let no_commit_repo = store
        .record_commits(ids::STEP_R2_PRD, &[commit(RepoId::new(), None)])
        .await;
    assert!(
        matches!(no_commit_repo, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown repo is a foreign key refusal, got {no_commit_repo:?}"
    );
    let no_commit_step = store.record_commits(StepId::new(), &[]).await;
    assert!(
        matches!(
            no_commit_step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: an empty slice still checks the step, got {no_commit_step:?}"
    );
    let stray_commit_on_no_step = store
        .record_commits(
            StepId::new(),
            &[RunStepCommit {
                run_step_id: ids::STEP_PLAN,
                ..commit(first, None)
            }],
        )
        .await;
    assert!(
        matches!(
            stray_commit_on_no_step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: D14 holds for commits too, got {stray_commit_on_no_step:?}"
    );
    store
        .record_commits(ids::STEP_R2_PRD, &[])
        .await
        .expect(CASE);

    let reach = store
        .delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the fixture project exists"));
    assert_eq!(
        (reach.run_step_trees, reach.run_step_commits),
        (2, 2),
        "{CASE}: the delete counts both tables (plan D12)"
    );
    let taken = store.delete_project(ids::PROJECT_HTUI).await.expect(CASE);
    assert_eq!(taken, reach, "{CASE}: the report equals the act");
    assert!(
        store
            .step_trees(ids::STEP_R2_PRD)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: the tree rows went with their step"
    );
    assert!(
        store
            .step_commits(ids::STEP_R2_PRD)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: and so did the commit rows"
    );
}

/// ANA-2 §4.2's verify report, in the table that holds it (plan D31): one `command_run` row per
/// run of a step's `verify_command`, written whole by the caller and read back in `queued_at`
/// order rather than insertion order.
///
/// `record_command_run` takes every column, `queued_at` included, for the reason blueprint F-S
/// gives the rest of the §8 writers: the row is the durable input of a later `verify_failure`
/// render, so the instants have to be the orchestrator's own and not two servers' clocks.
/// `command_runs` is a total read - an unknown step is an empty answer, never `NotFound` - which
/// is what lets the Runs tab ask for a step it has not yet seen written.
///
/// The refusals are asserted by variant and by the table name the sentence carries, not by the
/// whole sentence: `MemStore` speaks [`references_no_row`](crate::store::traits::references_no_row)
/// and [`already_exists`](crate::store::traits::already_exists) while Postgres speaks its own
/// constraint names, and the only thing both are obliged to agree on is that the input was
/// refused and which table refused it. The Postgres-only halves - a 70 KiB `output` byte for
/// byte, and the column widths - are
/// `pg_criteria.rs::command_run_round_trips_and_orders_by_queued_at`.
async fn verify_run_is_recorded<S: WriteStore>(store: &S) {
    const CASE: &str = "verify_run_is_recorded";
    let t0 = seam_clock();

    let first = NewCommandRun {
        id: CommandRunId::new(),
        run_step_id: ids::STEP_R2_PRD,
        box_id: ids::BOX,
        class: "verify".to_owned(),
        command: "cargo test".to_owned(),
        cwd: "/srv/trees/prd/core".to_owned(),
        status: CommandRunStatus::Done,
        exit_code: Some(0),
        output: Some("ok".to_owned()),
        queued_at: t0,
        started_at: Some(t0),
        finished_at: Some(t0 + TimeDelta::seconds(1)),
    };
    let written = store.record_command_run(first.clone()).await.expect(CASE);
    assert_eq!(
        written,
        CommandRun {
            id: first.id,
            run_step_id: first.run_step_id,
            box_id: first.box_id,
            class: first.class.clone(),
            command: first.command.clone(),
            cwd: first.cwd.clone(),
            status: first.status,
            exit_code: first.exit_code,
            output: first.output.clone(),
            queued_at: first.queued_at,
            started_at: first.started_at,
            finished_at: first.finished_at,
        },
        "{CASE}: every column is the caller's, handed straight back"
    );

    // Leg 2: a second row queued *earlier* than the first, written second. The read is ordered by
    // `queued_at`, so it comes out first — insertion order would put it last.
    let second = NewCommandRun {
        id: CommandRunId::new(),
        status: CommandRunStatus::Failed,
        exit_code: None,
        output: Some("no `sh` on PATH".to_owned()),
        queued_at: t0 - TimeDelta::seconds(1),
        started_at: None,
        finished_at: None,
        ..first.clone()
    };
    store.record_command_run(second.clone()).await.expect(CASE);
    let rows = store.command_runs(ids::STEP_R2_PRD).await.expect(CASE);
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![second.id, first.id],
        "{CASE}: `(queued_at, id)` order, not insertion order"
    );
    assert_eq!(
        rows.first().map(|row| row.status),
        Some(CommandRunStatus::Failed),
        "{CASE}: and a `failed` row keeps its status through the round trip"
    );
    assert_eq!(
        rows.first().and_then(|row| row.output.as_deref()),
        Some("no `sh` on PATH"),
        "{CASE}: ANA-2 §4.2's `unavailable` reason survives as the row's output"
    );

    // Leg 3: the read is total. A step with no rows and a step that does not exist both answer
    // `Ok(vec![])`.
    assert!(
        store
            .command_runs(ids::STEP_PLAN)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: a step with no command runs reads empty"
    );
    assert!(
        store
            .command_runs(StepId::new())
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: and so does a step id nothing has, never `NotFound`"
    );

    // Leg 4: the step is looked up before the box, so an input wrong in both ways is `NotFound`
    // and not the `Constraint` the box alone would earn.
    let no_step = store
        .record_command_run(NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: StepId::new(),
            box_id: BoxId::new(),
            ..first.clone()
        })
        .await;
    assert!(
        matches!(
            no_step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: the step is checked first, got {no_step:?}"
    );

    // Leg 5: an unknown box is the foreign key's refusal, and the sentence names the table.
    let no_box = store
        .record_command_run(NewCommandRun {
            id: CommandRunId::new(),
            box_id: BoxId::new(),
            ..first.clone()
        })
        .await;
    assert!(
        matches!(&no_box, Err(StoreError::Constraint(text)) if text.contains("command_run")),
        "{CASE}: an unknown box is a foreign key refusal naming the table, got {no_box:?}"
    );

    // Leg 6: the id is the caller's, so writing it twice is a duplicate key.
    let duplicate = store.record_command_run(first.clone()).await;
    assert!(
        matches!(&duplicate, Err(StoreError::Constraint(text)) if text.contains("command_run")),
        "{CASE}: a second row under the same id is refused, got {duplicate:?}"
    );

    assert_eq!(
        store
            .command_runs(ids::STEP_R2_PRD)
            .await
            .expect(CASE)
            .len(),
        2,
        "{CASE}: neither refusal wrote a row"
    );

    // Leg 7: the table is part of what a project delete takes, and was a hard-coded `0` on
    // `MemStore` until this milestone gave it rows to count (review L1).
    let reach = store
        .delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the fixture project exists"));
    assert_eq!(
        reach.command_runs, 2,
        "{CASE}: both rows are counted under the step's project"
    );
    let taken = store.delete_project(ids::PROJECT_HTUI).await.expect(CASE);
    assert_eq!(taken, reach, "{CASE}: the report equals the act");
    assert!(
        store
            .command_runs(ids::STEP_R2_PRD)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: and the rows went with their step"
    );
}

/// The version is allocated per `(item, kind)` inside the transaction (plan D6), and ANA-2 §4.2's
/// resolver ranks this run's output above another run's above a hand-written document.
///
/// The `MemStore` twin is `mem.rs::write_document_allocates_the_next_version_of_its_kind`; the
/// thing only Postgres can show — that two concurrent writers cannot allocate the same version —
/// is `pg_criteria.rs::document_versions_do_not_collide_under_contention`.
async fn write_document_allocates_its_version<S: WriteStore>(store: &S) {
    const CASE: &str = "write_document_allocates_its_version";
    let third = store
        .write_document(new_document(ids::HTUI_FEAT_1, "plan", Some(ids::STEP_PLAN)))
        .await
        .expect(CASE);
    assert_eq!(third.version, 3, "{CASE}: the fixture holds plan v1 and v2");
    let fourth = store
        .write_document(new_document(ids::HTUI_FEAT_1, "plan", None))
        .await
        .expect(CASE);
    assert_eq!(
        fourth.version, 4,
        "{CASE}: the next call takes the next number"
    );
    let fresh = store
        .write_document(new_document(ids::HTUI_FEAT_1, "review", None))
        .await
        .expect(CASE);
    assert_eq!(fresh.version, 1, "{CASE}: a kind with no rows starts at 1");
    let heads = store.documents(ids::HTUI_FEAT_1).await.expect(CASE);
    for id in [third.id, fourth.id, fresh.id] {
        assert!(
            heads.iter().any(|head| head.id == id),
            "{CASE}: {id} is listed among the item's documents"
        );
    }

    let no_item = store
        .write_document(new_document(ItemId::new(), "plan", None))
        .await;
    assert!(
        matches!(no_item, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: an unknown item is NotFound, got {no_item:?}"
    );
    let no_step = store
        .write_document(new_document(ids::HTUI_FEAT_1, "plan", Some(StepId::new())))
        .await;
    assert!(
        matches!(no_step, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown producing step is a foreign key refusal, got {no_step:?}"
    );
    let duplicate = store
        .write_document(NewDocument {
            id: third.id,
            ..new_document(ids::HTUI_FEAT_1, "plan", None)
        })
        .await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate id is Constraint, got {duplicate:?}"
    );

    // The preference leg: two runs of one item, each with a step, and a hand-written version on
    // top. Nothing here is a fan-out loser, so `selected IS NOT FALSE` admits every row and the
    // ranking is what decides.
    let second_run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert!(
        store
            .transition(ids::HTUI_ANA_2, Status::Queued, Status::Open)
            .await
            .expect(CASE),
        "{CASE}: queued -> open frees the item for a second run"
    );
    let third_run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    let step_of_second = new_run_step(second_run, 0, 1, 0);
    let second_step = step_of_second.id;
    store.create_step(step_of_second).await.expect(CASE);
    let step_of_third = new_run_step(third_run, 0, 1, 0);
    let third_step = step_of_third.id;
    store.create_step(step_of_third).await.expect(CASE);

    let by_second = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", Some(second_step)))
        .await
        .expect(CASE);
    let by_third = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", Some(third_step)))
        .await
        .expect(CASE);
    let by_hand = store
        .write_document(new_document(ids::HTUI_ANA_2, "research", None))
        .await
        .expect(CASE);
    assert_eq!(
        (by_second.version, by_third.version, by_hand.version),
        (1, 2, 3),
        "{CASE}: the hand-written version is the highest, so the ranking has to bite"
    );

    let picked = |run| async move {
        store
            .resolve_inputs(ids::HTUI_ANA_2, run, &["research".to_owned()])
            .await
            .expect(CASE)
            .first()
            .and_then(|row| row.document.as_ref())
            .map(|row| row.id)
    };
    assert_eq!(
        picked(second_run).await,
        Some(by_second.id),
        "{CASE}: this run's own output wins"
    );
    assert_eq!(
        picked(third_run).await,
        Some(by_third.id),
        "{CASE}: and so does the other run's, for the other run"
    );
    assert_eq!(
        picked(ids::RUN_1).await,
        Some(by_third.id),
        "{CASE}: for a third run, any run's output outranks hand-written and the highest \
         version wins among equals"
    );
    assert_eq!(
        store
            .documents_of_kinds(ids::HTUI_ANA_2, &["research".to_owned()])
            .await
            .expect(CASE)
            .first()
            .map(|row| row.id),
        Some(by_hand.id),
        "{CASE}: documents_of_kinds ranks by version alone — plan D2's contrast"
    );
    let with_gap = store
        .resolve_inputs(
            ids::HTUI_ANA_2,
            second_run,
            &["research".to_owned(), "nope".to_owned()],
        )
        .await
        .expect(CASE);
    assert_eq!(
        with_gap
            .iter()
            .map(|row| row.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["research", "nope"],
        "{CASE}: one entry per requested kind, in request order"
    );
    assert!(
        with_gap.get(1).is_some_and(|row| row.document.is_none()),
        "{CASE}: a kind the item has no eligible row for is carried as None (blueprint F-O)"
    );
}

/// `R-TUI-9`'s close-out is one transaction, refused while any run of the item is active. The
/// `MemStore` twin is `mem.rs::close_out_refuses_a_live_run_and_otherwise_writes_all_three_effects`.
async fn close_out_refuses_a_live_run<S: WriteStore>(store: &S) {
    const CASE: &str = "close_out_refuses_a_live_run";
    let repo = store
        .create_repo(new_repo(ids::PROJECT_HTUI, "core", true))
        .await
        .expect(CASE)
        .id;
    let commits = [RunStepCommit {
        run_step_id: ids::STEP_IMPL,
        repo_id: repo,
        before_hash: "0000000000000000000000000000000000000000".to_owned(),
        after_hash: Some("deadbeef".to_owned()),
    }];

    let live = store
        .close_out(
            ids::HTUI_FEAT_3,
            Resolution::Withdrawn,
            new_document(ids::HTUI_FEAT_3, "summary", None),
            &[],
        )
        .await;
    let StoreError::Constraint(sentence) = live.expect_err(&format!(
        "{CASE}: FEAT-3 still owns the queued RUN_2, so it cannot be closed out"
    )) else {
        panic!("{CASE}: a live run is Constraint, not NotFound")
    };
    assert!(
        sentence.contains(&ids::RUN_2.to_string()),
        "{CASE}: the refusal names the run that is in the way, got `{sentence}`"
    );
    assert!(
        store
            .documents(ids::HTUI_FEAT_3)
            .await
            .expect(CASE)
            .is_empty(),
        "{CASE}: the refusal wrote no summary"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_FEAT_3).await.status,
        Status::Queued,
        "{CASE}: nor moved the item"
    );

    // MOD-38's open -> closed edge: `queued -> open` leaves RUN_2 queued, and the law alone
    // would close the now-`open` item as `withdrawn`; the live run still refuses it.
    assert!(
        store
            .transition(ids::HTUI_FEAT_3, Status::Queued, Status::Open)
            .await
            .expect(CASE),
        "{CASE}: FEAT-3 is un-queued, RUN_2 still live"
    );
    let reopened = store
        .close_out(
            ids::HTUI_FEAT_3,
            Resolution::Withdrawn,
            new_document(ids::HTUI_FEAT_3, "summary", None),
            &[],
        )
        .await;
    assert!(
        matches!(&reopened, Err(StoreError::Constraint(sentence))
            if sentence.contains(&ids::RUN_2.to_string())),
        "{CASE}: an `open` item with a live run is not closed out either, got {reopened:?}"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_FEAT_3).await.status,
        Status::Open,
        "{CASE}: and stays open"
    );

    for (item, resolution, summary, why) in [
        (
            ids::HTUI_ANA_2,
            Resolution::Done,
            new_document(ids::HTUI_ANA_2, "summary", None),
            "open does not close as done (ANA-11 §4.2)",
        ),
        (
            ids::HTUI_FEAT_1,
            Resolution::Done,
            new_document(ids::HTUI_FEAT_1, "plan", None),
            "the document must be a summary",
        ),
        (
            ids::HTUI_FEAT_1,
            Resolution::Done,
            new_document(ids::HTUI_ANA_2, "summary", None),
            "the summary must name the item being closed",
        ),
    ] {
        let refused = store.close_out(item, resolution, summary, &[]).await;
        assert!(
            matches!(refused, Err(StoreError::Constraint(_))),
            "{CASE}: {why}, got {refused:?}"
        );
    }
    let unknown = store
        .close_out(
            ItemId::new(),
            Resolution::Withdrawn,
            new_document(ItemId::new(), "summary", None),
            &[],
        )
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: an unknown item is NotFound, got {unknown:?}"
    );
    assert_eq!(
        store.documents(ids::HTUI_FEAT_1).await.expect(CASE).len(),
        3,
        "{CASE}: no refusal wrote a document"
    );

    assert!(
        store
            .transition(ids::HTUI_FEAT_1, Status::InProgress, Status::Done)
            .await
            .expect(CASE),
        "{CASE}: the item finishes before it is closed"
    );
    let written = store
        .close_out(
            ids::HTUI_FEAT_1,
            Resolution::Done,
            new_document(ids::HTUI_FEAT_1, "summary", None),
            &commits,
        )
        .await
        .expect(CASE);
    assert_eq!(written.version, 1, "{CASE}: the first summary of the item");
    let closed = item_row(CASE, store, ids::HTUI_FEAT_1).await;
    assert_eq!(closed.status, Status::Closed, "{CASE}: the item is closed");
    assert_eq!(
        closed.resolution,
        Some(Resolution::Done),
        "{CASE}: as the resolution it was handed"
    );
    assert!(
        closed.closed_at.is_some(),
        "{CASE}: closed_at tracks the current status"
    );
    assert_eq!(
        store
            .step_commits(ids::STEP_IMPL)
            .await
            .expect(CASE)
            .iter()
            .map(|row| row.repo_id)
            .collect::<Vec<_>>(),
        vec![repo],
        "{CASE}: the close-out's commits landed on their own step"
    );
}

/// Every pair ANA-2 §4.3 leaves out is [`StoreError::Constraint`] with no write, on all three
/// tables — and plan D14's precedence holds: an unknown id is `NotFound` even when the pair is
/// also illegal. `mem.rs::supersede_and_fail_run_refuse_what_the_law_forbids` covers the same
/// ground for `fail_run` on `MemStore`.
async fn illegal_transitions_are_constraint<S: WriteStore>(store: &S) {
    const CASE: &str = "illegal_transitions_are_constraint";

    for (from, to) in [
        (Status::Open, Status::Queued),
        (Status::Queued, Status::InProgress),
        (Status::InProgress, Status::Done),
        // The fourth pair is what makes `done` a `from` rather than only the row the loop stops
        // on: `done` reaches `open` and nothing else, so seven of its eight targets are refusals
        // this leg would otherwise never ask for (`closed` among them since MOD-38 PRD D1).
        (Status::Done, Status::Open),
    ] {
        let before = item_row(CASE, store, ids::HTUI_ANA_2).await;
        assert_eq!(before.status, from, "{CASE}: the item is driven to {from}");
        for target in Status::ALL.iter().copied() {
            if from.can_move_to(target) {
                continue;
            }
            let refused = store.transition(ids::HTUI_ANA_2, from, target).await;
            assert!(
                matches!(refused, Err(StoreError::Constraint(_))),
                "{CASE}: item {from} -> {target} is outside §4.3, got {refused:?}"
            );
        }
        assert_eq!(
            item_row(CASE, store, ids::HTUI_ANA_2).await,
            before,
            "{CASE}: every refused item pair left the row byte-identical"
        );
        assert!(
            store
                .transition(ids::HTUI_ANA_2, from, to)
                .await
                .expect(CASE),
            "{CASE}: {from} -> {to} is the sanctioned step on"
        );
    }

    let at = seam_clock();
    for (from, to) in [
        (RunStatus::Queued, RunStatus::Running),
        (RunStatus::Running, RunStatus::AwaitingApproval),
    ] {
        let before = run_row(CASE, store, ids::RUN_2).await;
        assert_eq!(before.status, from, "{CASE}: the run is driven to {from}");
        for target in RunStatus::ALL.iter().copied() {
            if from.can_move_to(target) {
                continue;
            }
            let refused = store.transition_run(ids::RUN_2, from, target, at).await;
            assert!(
                matches!(refused, Err(StoreError::Constraint(_))),
                "{CASE}: run {from} -> {target} is outside §4.3, got {refused:?}"
            );
        }
        assert_eq!(
            run_row(CASE, store, ids::RUN_2).await,
            before,
            "{CASE}: every refused run pair left the row byte-identical"
        );
        assert!(
            store
                .transition_run(ids::RUN_2, from, to, at)
                .await
                .expect(CASE),
            "{CASE}: {from} -> {to} is the sanctioned step on"
        );
    }

    for (from, to) in [
        (StepStatus::Pending, StepStatus::Running),
        (StepStatus::Running, StepStatus::Done),
        (StepStatus::Done, StepStatus::Superseded),
    ] {
        let before = step_row(CASE, store, ids::RUN_2, ids::STEP_R2_PRD).await;
        assert_eq!(before.status, from, "{CASE}: the step is driven to {from}");
        for target in StepStatus::ALL.iter().copied() {
            if from.can_move_to(target) {
                continue;
            }
            let refused = store
                .transition_step(ids::STEP_R2_PRD, from, target, at)
                .await;
            assert!(
                matches!(refused, Err(StoreError::Constraint(_))),
                "{CASE}: run_step {from} -> {target} is outside §4.3, got {refused:?}"
            );
        }
        assert_eq!(
            step_row(CASE, store, ids::RUN_2, ids::STEP_R2_PRD).await,
            before,
            "{CASE}: every refused step pair left the row byte-identical"
        );
        assert!(
            store
                .transition_step(ids::STEP_R2_PRD, from, to, at)
                .await
                .expect(CASE),
            "{CASE}: {from} -> {to} is the sanctioned step on"
        );
    }

    let item = store
        .transition(ItemId::new(), Status::Queued, Status::Done)
        .await;
    assert!(
        matches!(item, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: plan D14 — an unknown item is NotFound even though the pair is illegal, got \
         {item:?}"
    );
    let run = store
        .transition_run(RunId::new(), RunStatus::Done, RunStatus::Queued, at)
        .await;
    assert!(
        matches!(run, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: and so is an unknown run, got {run:?}"
    );
    let step = store
        .transition_step(StepId::new(), StepStatus::Done, StepStatus::Pending, at)
        .await;
    assert!(
        matches!(
            step,
            Err(StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "{CASE}: and an unknown step, got {step:?}"
    );

    store.fail_run(ids::RUN_2, "boom", at).await.expect(CASE);
    let failed = run_row(CASE, store, ids::RUN_2).await;
    assert_eq!(failed.status, RunStatus::Failed, "{CASE}: the run failed");
    assert_eq!(
        failed.failure.as_deref(),
        Some("boom"),
        "{CASE}: the reason is stored"
    );
    assert_eq!(
        failed.finished_at,
        Some(at),
        "{CASE}: fail_run stamps finished_at from the caller's clock"
    );
    let again = store.fail_run(ids::RUN_2, "again", at).await;
    let StoreError::Constraint(sentence) =
        again.expect_err(&format!("{CASE}: a terminal run cannot fail twice"))
    else {
        panic!("{CASE}: a terminal run is Constraint, not NotFound")
    };
    assert!(
        sentence.contains(RunStatus::Failed.as_str()) && sentence.contains("§4.3"),
        "{CASE}: the refusal is `illegal_move`'s sentence, naming both statuses, got `{sentence}`"
    );
    let missing = store.fail_run(RunId::new(), "boom", at).await;
    assert!(
        matches!(missing, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: an unknown run is NotFound, got {missing:?}"
    );
}

/// The HTUI project's fixture items, which between them hold every [`Status`] once (a test in
/// `fixtures.rs` pins that): the two MOD-38 cases sweep the laws over them rather than over a
/// list of their own.
fn htui_fixture_items() -> Vec<ItemId> {
    crate::fixtures::demo_data()
        .items
        .into_iter()
        .filter(|item| item.project_id == ids::PROJECT_HTUI)
        .map(|item| item.id)
        .collect()
}

/// MOD-38 plan D4: close-out's law is ANA-11 §4.2's [`Resolution::closes_from`], not ANA-2
/// §4.3's transition table. Every pair the law refuses is exactly [`resolution_not_closable`]'s
/// sentence with nothing written; an `open` item closes as a non-success resolution and reads it
/// back; a `closed` item closes as nothing.
async fn close_out_resolution_law<S: WriteStore>(store: &S) {
    const CASE: &str = "close_out_resolution_law";

    let mut swept = Vec::new();
    for item in htui_fixture_items() {
        // A live run is refused before the law is asked (the guard order of `close_out`'s doc),
        // which is `close_out_refuses_a_live_run`'s ground, not this case's.
        if store
            .runs(item)
            .await
            .expect(CASE)
            .iter()
            .any(|run| run.status.is_active())
        {
            continue;
        }
        let before = item_row(CASE, store, item).await;
        let documents = store.documents(item).await.expect(CASE);
        for resolution in Resolution::ALL.iter().copied() {
            if resolution.closes_from(before.status) {
                continue;
            }
            let refused = store
                .close_out(item, resolution, new_document(item, "summary", None), &[])
                .await;
            let StoreError::Constraint(sentence) = refused.expect_err(&format!(
                "{CASE}: `{}` does not close as `{resolution}`",
                before.status
            )) else {
                panic!("{CASE}: a refused pair is Constraint, not NotFound")
            };
            assert_eq!(
                sentence,
                resolution_not_closable(item, before.status, resolution),
                "{CASE}: the refusal is the close-out law's sentence"
            );
        }
        assert_eq!(
            item_row(CASE, store, item).await,
            before,
            "{CASE}: every refused pair left the `{}` row byte-identical",
            before.status
        );
        assert_eq!(
            store.documents(item).await.expect(CASE),
            documents,
            "{CASE}: and wrote no summary"
        );
        swept.push(before.status);
    }
    for status in [
        Status::Open,
        Status::Blocked,
        Status::Failed,
        Status::Done,
        Status::Closed,
    ] {
        assert!(
            swept.contains(&status),
            "{CASE}: the sweep reached a `{status}` item (fixture precondition)"
        );
    }

    let open = item_row(CASE, store, ids::HTUI_ANA_2).await;
    assert_eq!(open.status, Status::Open, "{CASE}: fixture precondition");
    let refused = store
        .close_out(
            ids::HTUI_ANA_2,
            Resolution::Done,
            new_document(ids::HTUI_ANA_2, "summary", None),
            &[],
        )
        .await;
    assert!(
        matches!(&refused, Err(StoreError::Constraint(sentence))
            if *sentence == resolution_not_closable(ids::HTUI_ANA_2, Status::Open, Resolution::Done)),
        "{CASE}: an open item does not close as done, got {refused:?}"
    );
    let written = store
        .close_out(
            ids::HTUI_ANA_2,
            Resolution::Withdrawn,
            new_document(ids::HTUI_ANA_2, "summary", None),
            &[],
        )
        .await
        .expect(CASE);
    assert_eq!(written.version, 1, "{CASE}: the first summary of the item");
    let closed = item_row(CASE, store, ids::HTUI_ANA_2).await;
    assert_eq!(
        (closed.status, closed.resolution),
        (Status::Closed, Some(Resolution::Withdrawn)),
        "{CASE}: an open item closes as withdrawn, and says so"
    );
    assert!(
        closed.closed_at.is_some(),
        "{CASE}: closed_at tracks the current status"
    );

    for item in [ids::HTUI_FIX_1, ids::HTUI_ANA_2] {
        let before = item_row(CASE, store, item).await;
        for resolution in Resolution::ALL.iter().copied() {
            let refused = store
                .close_out(item, resolution, new_document(item, "summary", None), &[])
                .await;
            assert!(
                matches!(&refused, Err(StoreError::Constraint(sentence))
                    if *sentence == resolution_not_closable(item, Status::Closed, resolution)),
                "{CASE}: a closed item does not close again as `{resolution}`, got {refused:?}"
            );
        }
        assert_eq!(
            item_row(CASE, store, item).await,
            before,
            "{CASE}: a second close-out keeps the first resolution"
        );
    }
}

/// MOD-38 PRD D1: `transition` never reaches `closed`, from any status; close-out is the only way
/// in. The refusal is [`illegal_move`]'s sentence with the row untouched, and plan D14's
/// precedence still holds: an unknown id is `NotFound` first.
async fn transition_never_reaches_closed<S: WriteStore>(store: &S) {
    const CASE: &str = "transition_never_reaches_closed";

    for item in htui_fixture_items() {
        let before = item_row(CASE, store, item).await;
        let refused = store.transition(item, before.status, Status::Closed).await;
        let StoreError::Constraint(sentence) = refused.expect_err(&format!(
            "{CASE}: `{}` does not transition into `closed`",
            before.status
        )) else {
            panic!("{CASE}: an illegal move is Constraint, not NotFound")
        };
        assert_eq!(
            sentence,
            illegal_move("item", before.status, Status::Closed),
            "{CASE}: the refusal is `illegal_move`'s sentence"
        );
        assert_eq!(
            item_row(CASE, store, item).await,
            before,
            "{CASE}: the refused `{}` row is byte-identical",
            before.status
        );
    }

    let unknown = store
        .transition(ItemId::new(), Status::Done, Status::Closed)
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "item", .. })),
        "{CASE}: plan D14 — an unknown item is NotFound even though the pair is illegal, got \
         {unknown:?}"
    );
}

// ---- MOD-38 (plan D8-D14): requirements and citations --------------------------------------
//
// Every case below reads and writes through the ANA-11 §5.1 methods alone and asserts against
// the fixture's requirement set (blueprint §8): `htui` holds `R-ENT-1` (v2, amended by `ANA-2`),
// `R-ENT-2` and `R-STO-1`; `ANA-1`'s citation of `R-ENT-1` is stamped at v1 and so is suspect.

/// A requirement request with a fresh id, by the fixture user on the fixture box.
fn new_requirement(body: &str) -> NewRequirement {
    NewRequirement {
        id: RequirementId::new(),
        body: body.to_owned(),
        rationale: String::new(),
        priority: Priority::Must,
        created_by: ids::USER,
        box_id: Some(ids::BOX),
    }
}

/// An amend of the body alone, by the fixture user on the fixture box, with the ordinary reason.
fn body_patch(body: &str) -> RequirementPatch {
    RequirementPatch {
        body: Some(body.to_owned()),
        rationale: None,
        priority: None,
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "amended".to_owned(),
    }
}

/// An area request with a fresh id and an empty description.
fn new_area(project: ProjectId, code: &str, position: i32) -> NewRequirementArea {
    NewRequirementArea {
        id: RequirementAreaId::new(),
        project_id: project,
        code: code.to_owned(),
        title: format!("{code} area"),
        description: String::new(),
        position,
    }
}

/// The head an amend or withdraw landed, or a panic naming the case.
fn requirement_updated(case: &str, outcome: RequirementUpdate) -> Requirement {
    match outcome {
        RequirementUpdate::Updated(row) => row,
        RequirementUpdate::Diverged { head, ancestor } => {
            panic!(
                "{case}: expected Updated, got Diverged {{ head: {head:?}, ancestor: {ancestor:?} }}"
            )
        }
    }
}

/// One requirement row as the store holds it now.
async fn requirement_row<S: ReadStore>(case: &str, store: &S, id: RequirementId) -> Requirement {
    store
        .requirement(id)
        .await
        .expect(case)
        .unwrap_or_else(|| panic!("{case}: requirement {id} exists"))
}

/// A requirement's revisions on a store that keeps them (`MemStore`, `PgStore`).
async fn revisions_of<S: ReadStore>(
    case: &str,
    store: &S,
    id: RequirementId,
) -> Vec<RequirementRevision> {
    store
        .requirement_revisions(id)
        .await
        .expect(case)
        .unwrap_or_else(|| panic!("{case}: a writable store keeps the revisions of {id}"))
}

/// A citation as [`ReadStore::item_requirements`] should answer it, with the requirement as it is
/// now and `suspect` derived from the two versions (plan D11).
async fn citation<S: ReadStore>(
    case: &str,
    store: &S,
    requirement: RequirementId,
    kind: CitationKind,
    stamp: i32,
) -> ItemCitation {
    let requirement = requirement_row(case, store, requirement).await;
    let suspect = requirement.makes_suspect(stamp);
    ItemCitation {
        requirement,
        kind,
        requirement_version: stamp,
        proposed_by_step_id: None,
        suspect,
    }
}

/// `(item, kind, stamp, suspect)` of every coverage row, in the order the store answered.
fn coverage_view(rows: &[CoverageRow]) -> Vec<(ItemId, CitationKind, i32, bool)> {
    rows.iter()
        .map(|row| (row.item.id, row.kind, row.requirement_version, row.suspect))
        .collect()
}

/// `NotFound { entity: "item_requirement" }` naming exactly this triple (plan D10).
fn is_missing_citation<T>(
    outcome: &Result<T, StoreError>,
    item: ItemId,
    requirement: RequirementId,
    kind: CitationKind,
) -> bool {
    matches!(outcome, Err(StoreError::NotFound { entity: "item_requirement", id })
        if *id == citation_key(item, requirement, kind))
}

/// Plan D8: the number comes from the area's counter, one per area, and a refused mint burns
/// none. The generated key, the copied `area_code` and revision 1 come with the row.
async fn requirement_mint_is_per_area_and_never_reused<S: WriteStore>(store: &S) {
    const CASE: &str = "requirement_mint_is_per_area_and_never_reused";

    let request = new_requirement("Every area mints its own numbers.");
    let ent = store
        .mint_requirement(ids::AREA_ENT, request.clone())
        .await
        .expect(CASE);
    assert_eq!(
        (
            ent.id,
            ent.project_id,
            ent.area_id,
            ent.area_code.as_str(),
            ent.number,
            ent.key.as_str(),
        ),
        (
            request.id,
            ids::PROJECT_HTUI,
            ids::AREA_ENT,
            "ENT",
            3,
            "R-ENT-3"
        ),
        "{CASE}: the fixture minted R-ENT-1 and R-ENT-2, so the next ENT number is 3"
    );
    assert_eq!(
        (
            ent.body.as_str(),
            ent.rationale.as_str(),
            ent.priority,
            ent.state,
            ent.version,
            ent.created_by,
        ),
        (
            request.body.as_str(),
            "",
            Priority::Must,
            RequirementState::Active,
            1,
            ids::USER
        ),
        "{CASE}: the row carries the request, active at v1"
    );
    assert_eq!(
        store.requirement(ent.id).await.expect(CASE),
        Some(ent.clone()),
        "{CASE}: the mint reads back as it was returned"
    );

    let sto = store
        .mint_requirement(
            ids::AREA_STO,
            new_requirement("A second area, a second counter."),
        )
        .await
        .expect(CASE);
    assert_eq!(
        (sto.key.as_str(), sto.number),
        ("R-STO-2", 2),
        "{CASE}: STO counts on its own"
    );

    let nil_author = store
        .mint_requirement(
            ids::AREA_ENT,
            NewRequirement {
                created_by: UserId::default(),
                ..new_requirement("By nobody.")
            },
        )
        .await;
    assert!(
        matches!(nil_author, Err(StoreError::Constraint(_))),
        "{CASE}: a created_by that names no row is Constraint, got {nil_author:?}"
    );
    let duplicate = store.mint_requirement(ids::AREA_ENT, request.clone()).await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate id is Constraint, got {duplicate:?}"
    );
    let next = store
        .mint_requirement(ids::AREA_ENT, new_requirement("After two refusals."))
        .await
        .expect(CASE);
    assert_eq!(
        next.key, "R-ENT-4",
        "{CASE}: neither refused mint consumed a number (plan D8)"
    );

    let unknown = store
        .mint_requirement(RequirementAreaId::new(), new_requirement("Nowhere."))
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "requirement_area",
                ..
            })
        ),
        "{CASE}: an unknown area is NotFound, got {unknown:?}"
    );

    let revisions = revisions_of(CASE, store, ent.id).await;
    assert_eq!(
        revisions
            .iter()
            .map(|row| (
                row.requirement_id,
                row.version,
                row.body.as_str(),
                row.rationale.as_str(),
                row.priority,
                row.state,
                row.author_id,
                row.box_id,
                row.reason.as_str(),
                row.amended_by_item_id,
            ))
            .collect::<Vec<_>>(),
        vec![(
            ent.id,
            1,
            request.body.as_str(),
            "",
            Priority::Must,
            RequirementState::Active,
            ids::USER,
            Some(ids::BOX),
            "created",
            None,
        )],
        "{CASE}: the mint wrote revision 1, reason `created`"
    );
}

/// The `requirement_area.code` CHECK is decided before the insert with
/// [`invalid_area_code`]'s sentence; the other refusals are `Constraint`, and a code is unique per
/// project only. Areas read back in `(position, code)` order.
async fn requirement_area_create_checks_the_code<S: WriteStore>(store: &S) {
    const CASE: &str = "requirement_area_create_checks_the_code";

    let too_long = format!("E{}", "N".repeat(16));
    for code in ["ent", "E", "E-1", too_long.as_str()] {
        let refused = store
            .create_requirement_area(new_area(ids::PROJECT_HTUI, code, 2))
            .await;
        assert!(
            matches!(&refused, Err(StoreError::Constraint(sentence))
                if *sentence == invalid_area_code(code)),
            "{CASE}: `{code}` is outside the CHECK, got {refused:?}"
        );
    }
    let taken = store
        .create_requirement_area(new_area(ids::PROJECT_HTUI, "ENT", 2))
        .await;
    assert!(
        matches!(taken, Err(StoreError::Constraint(_))),
        "{CASE}: htui already has ENT, got {taken:?}"
    );
    let duplicate = store
        .create_requirement_area(NewRequirementArea {
            id: ids::AREA_ENT,
            ..new_area(ids::PROJECT_HTUI, "DUP", 2)
        })
        .await;
    assert!(
        matches!(duplicate, Err(StoreError::Constraint(_))),
        "{CASE}: a duplicate id is Constraint, got {duplicate:?}"
    );
    let orphan = store
        .create_requirement_area(new_area(ProjectId::new(), "ENT", 0))
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown project is Constraint, got {orphan:?}"
    );
    assert_eq!(
        store
            .requirement_areas(ids::PROJECT_HTUI)
            .await
            .expect(CASE)
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![ids::AREA_ENT, ids::AREA_STO],
        "{CASE}: no refusal wrote a row"
    );

    let request = new_area(ids::PROJECT_AGY, "ENT", 1);
    let ent = store
        .create_requirement_area(request.clone())
        .await
        .expect(CASE);
    assert_eq!(
        (
            ent.id,
            ent.project_id,
            ent.code.as_str(),
            ent.title.as_str(),
            ent.description.as_str(),
            ent.position,
        ),
        (
            request.id,
            ids::PROJECT_AGY,
            "ENT",
            request.title.as_str(),
            "",
            1
        ),
        "{CASE}: ENT is free in agy: a code is unique per project"
    );
    let api = store
        .create_requirement_area(new_area(ids::PROJECT_AGY, "API", 1))
        .await
        .expect(CASE);
    let first = store
        .create_requirement_area(new_area(ids::PROJECT_AGY, "ZZ", 0))
        .await
        .expect(CASE);
    assert_eq!(
        store.requirement_areas(ids::PROJECT_AGY).await.expect(CASE),
        vec![first, api, ent],
        "{CASE}: position first, then code by bytes"
    );
}

/// ANA-9 §4.2's compare-and-set on `requirement.version` (plan D9): the edit lands at `v + 1`
/// with a revision naming the deciding item, and a spent token answers the head and the revision
/// it was taken from. The checks run NotFound, divergence, Constraint.
async fn requirement_amend_is_cas_and_names_the_item<S: WriteStore>(store: &S) {
    const CASE: &str = "requirement_amend_is_cas_and_names_the_item";

    let before = requirement_row(CASE, store, ids::REQ_STO_1).await;
    assert_eq!(before.version, 1, "{CASE}: fixture precondition");
    let patch = body_patch("Postgres is the only source of truth.");
    let head = requirement_updated(
        CASE,
        store
            .amend_requirement(ids::REQ_STO_1, 1, patch.clone(), ids::HTUI_FEAT_3)
            .await
            .expect(CASE),
    );
    assert_eq!(
        (
            head.version,
            head.body.as_str(),
            head.rationale.as_str(),
            head.priority,
            head.state,
            head.key.as_str(),
        ),
        (
            2,
            "Postgres is the only source of truth.",
            before.rationale.as_str(),
            before.priority,
            RequirementState::Active,
            "R-STO-1"
        ),
        "{CASE}: the patched column moved, the others did not, and the version went up by one"
    );
    assert_eq!(
        requirement_row(CASE, store, ids::REQ_STO_1).await,
        head,
        "{CASE}: the head reads back as it was returned"
    );
    let revisions = revisions_of(CASE, store, ids::REQ_STO_1).await;
    assert_eq!(
        revisions
            .iter()
            .map(|row| (
                row.version,
                row.body.as_str(),
                row.reason.as_str(),
                row.amended_by_item_id,
                row.author_id,
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, before.body.as_str(), "created", None, ids::USER),
            (
                2,
                head.body.as_str(),
                patch.reason.as_str(),
                Some(ids::HTUI_FEAT_3),
                ids::USER
            ),
        ],
        "{CASE}: revision 2 names the deciding item and carries the patch's reason"
    );

    let spent = store
        .amend_requirement(ids::REQ_STO_1, 1, body_patch("Too late."), ids::HTUI_FEAT_3)
        .await
        .expect(CASE);
    assert_eq!(
        spent,
        RequirementUpdate::Diverged {
            head: head.clone(),
            ancestor: revisions[0].clone(),
        },
        "{CASE}: a spent token answers the head and the revision it was taken from"
    );

    let unknown = store
        .amend_requirement(RequirementId::new(), 1, body_patch("x"), ids::HTUI_FEAT_3)
        .await;
    assert!(
        matches!(
            unknown,
            Err(StoreError::NotFound {
                entity: "requirement",
                ..
            })
        ),
        "{CASE}: an unknown requirement is NotFound, got {unknown:?}"
    );
    let orphan = store
        .amend_requirement(ids::REQ_STO_1, 2, body_patch("By nothing."), ItemId::new())
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: an amended_by that names no item is Constraint, got {orphan:?}"
    );
    assert_eq!(
        requirement_row(CASE, store, ids::REQ_STO_1).await,
        head,
        "{CASE}: the refused amend left the head at v2"
    );
    assert_eq!(
        revisions_of(CASE, store, ids::REQ_STO_1).await.len(),
        2,
        "{CASE}: and wrote no revision"
    );
}

/// Plan D10: a withdraw is an amend to `state = withdrawn`, recorded with the deciding item's
/// `withdraws` citation; afterwards the requirement takes no new `addresses` / `reserves`
/// citation, has none re-stamped by `reconfirm`, and is neither amended nor withdrawn again.
async fn requirement_withdraw_refuses_new_addresses<S: WriteStore>(store: &S) {
    const CASE: &str = "requirement_withdraw_refuses_new_addresses";

    store
        .cite(
            ids::HTUI_FIX_1,
            ids::REQ_ENT_2,
            CitationKind::Addresses,
            None,
        )
        .await
        .expect(CASE);
    let head = requirement_updated(
        CASE,
        store
            .withdraw_requirement(
                ids::REQ_ENT_2,
                1,
                ids::HTUI_FEAT_3,
                ids::USER,
                Some(ids::BOX),
            )
            .await
            .expect(CASE),
    );
    assert_eq!(
        (head.state, head.version),
        (RequirementState::Withdrawn, 2),
        "{CASE}: the withdraw lands at v2"
    );
    let last = revisions_of(CASE, store, ids::REQ_ENT_2)
        .await
        .pop()
        .unwrap_or_else(|| panic!("{CASE}: the withdraw wrote a revision"));
    assert_eq!(
        (
            last.version,
            last.state,
            last.reason.as_str(),
            last.amended_by_item_id,
        ),
        (
            2,
            RequirementState::Withdrawn,
            "withdrawn",
            Some(ids::HTUI_FEAT_3)
        ),
        "{CASE}: revision 2 is the withdraw, naming the deciding item"
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_3).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_ENT_2, CitationKind::Withdraws, 2).await],
        "{CASE}: the deciding item cites the requirement as `withdraws`, at the new version"
    );

    let addresses = store
        .cite(
            ids::AGY_FEAT_1,
            ids::REQ_ENT_2,
            CitationKind::Addresses,
            None,
        )
        .await;
    assert!(
        matches!(&addresses, Err(StoreError::Constraint(sentence))
            if *sentence == withdrawn_requirement_cited("R-ENT-2", CitationKind::Addresses)),
        "{CASE}: a withdrawn requirement takes no new `addresses`, got {addresses:?}"
    );
    let revived = store
        .cite(
            ids::HTUI_FEAT_2,
            ids::REQ_ENT_2,
            CitationKind::Reserves,
            None,
        )
        .await;
    assert!(
        matches!(&revived, Err(StoreError::Constraint(sentence))
            if *sentence == withdrawn_requirement_cited("R-ENT-2", CitationKind::Reserves)),
        "{CASE}: nor does a tombstoned `reserves` come back, got {revived:?}"
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_2).await.expect(CASE),
        Vec::new(),
        "{CASE}: the refused revival left the tombstone alone"
    );

    let reconfirmed = store
        .reconfirm(ids::HTUI_FIX_1, ids::REQ_ENT_2, CitationKind::Addresses)
        .await;
    assert!(
        matches!(&reconfirmed, Err(StoreError::Constraint(sentence))
            if *sentence == withdrawn_requirement_cited("R-ENT-2", CitationKind::Addresses)),
        "{CASE}: nor has a live `addresses` from before the withdraw re-stamped, \
         got {reconfirmed:?}"
    );
    assert_eq!(
        store
            .item_requirements(ids::HTUI_FIX_1)
            .await
            .expect(CASE)
            .into_iter()
            .filter(|row| row.requirement.id == ids::REQ_ENT_2)
            .collect::<Vec<_>>(),
        vec![citation(CASE, store, ids::REQ_ENT_2, CitationKind::Addresses, 1).await],
        "{CASE}: the refused reconfirm left the citation at v1, suspect"
    );
    store
        .reconfirm(ids::HTUI_FEAT_3, ids::REQ_ENT_2, CitationKind::Withdraws)
        .await
        .unwrap_or_else(|error| {
            panic!("{CASE}: the deciding `withdraws` still reconfirms, got {error:?}")
        });

    let again = store
        .withdraw_requirement(
            ids::REQ_ENT_2,
            2,
            ids::HTUI_FEAT_3,
            ids::USER,
            Some(ids::BOX),
        )
        .await;
    assert!(
        matches!(&again, Err(StoreError::Constraint(sentence))
            if *sentence == requirement_withdrawn("R-ENT-2")),
        "{CASE}: a withdrawn requirement is not withdrawn again, got {again:?}"
    );
    let amended = store
        .amend_requirement(ids::REQ_ENT_2, 2, body_patch("Revived?"), ids::HTUI_FEAT_3)
        .await;
    assert!(
        matches!(&amended, Err(StoreError::Constraint(sentence))
            if *sentence == requirement_withdrawn("R-ENT-2")),
        "{CASE}: nor amended, got {amended:?}"
    );
    assert_eq!(
        requirement_row(CASE, store, ids::REQ_ENT_2).await,
        head,
        "{CASE}: both refusals left the head at v2"
    );
}

/// PRD D3: an amend records the deciding item's `amends` citation at the new version in the same
/// transaction, and a later amend by the same item revives and re-stamps that one row.
async fn amend_records_the_deciding_citation<S: WriteStore>(store: &S) {
    const CASE: &str = "amend_records_the_deciding_citation";

    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_3).await.expect(CASE),
        Vec::new(),
        "{CASE}: fixture precondition, FEAT-3 cites nothing"
    );
    requirement_updated(
        CASE,
        store
            .amend_requirement(ids::REQ_STO_1, 1, body_patch("v2"), ids::HTUI_FEAT_3)
            .await
            .expect(CASE),
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_3).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_STO_1, CitationKind::Amends, 2).await],
        "{CASE}: the amend wrote FEAT-3's `amends` citation at v2, not suspect"
    );

    store
        .uncite(ids::HTUI_FEAT_3, ids::REQ_STO_1, CitationKind::Amends)
        .await
        .expect(CASE);
    requirement_updated(
        CASE,
        store
            .amend_requirement(ids::REQ_STO_1, 2, body_patch("v3"), ids::HTUI_FEAT_3)
            .await
            .expect(CASE),
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_3).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_STO_1, CitationKind::Amends, 3).await],
        "{CASE}: the second amend revived the tombstone at v3: one row, not two"
    );
}

/// Plan D11: a citation is suspect while the requirement is newer than its stamp, on both reads,
/// and `reconfirm` re-stamps it at the current version. A triple with no live row is `NotFound`
/// naming [`citation_key`].
async fn a_newer_version_makes_a_citation_suspect_until_reconfirmed<S: WriteStore>(store: &S) {
    const CASE: &str = "a_newer_version_makes_a_citation_suspect_until_reconfirmed";

    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_1).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_STO_1, CitationKind::Addresses, 1).await],
        "{CASE}: fixture precondition, FEAT-1 addresses R-STO-1 at its current v1"
    );
    requirement_updated(
        CASE,
        store
            .amend_requirement(ids::REQ_STO_1, 1, body_patch("v2"), ids::HTUI_FEAT_3)
            .await
            .expect(CASE),
    );
    let suspect = citation(CASE, store, ids::REQ_STO_1, CitationKind::Addresses, 1).await;
    assert!(suspect.suspect, "{CASE}: v2 > stamp 1");
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_1).await.expect(CASE),
        vec![suspect],
        "{CASE}: the amend made FEAT-1's citation suspect"
    );
    assert_eq!(
        coverage_view(
            &store
                .requirement_coverage(ids::REQ_STO_1)
                .await
                .expect(CASE)
        ),
        vec![
            (ids::HTUI_FEAT_1, CitationKind::Addresses, 1, true),
            (ids::HTUI_FEAT_3, CitationKind::Amends, 2, false),
            (ids::HTUI_FIX_1, CitationKind::Addresses, 1, true),
        ],
        "{CASE}: coverage derives the same flag, in item key order"
    );

    let restamped = store
        .reconfirm(ids::HTUI_FEAT_1, ids::REQ_STO_1, CitationKind::Addresses)
        .await
        .expect(CASE);
    assert_eq!(
        (
            restamped.item_id,
            restamped.requirement_id,
            restamped.kind,
            restamped.requirement_version,
            restamped.deleted_at,
        ),
        (
            ids::HTUI_FEAT_1,
            ids::REQ_STO_1,
            CitationKind::Addresses,
            2,
            None
        ),
        "{CASE}: reconfirm re-stamps the live row at the current version"
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_1).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_STO_1, CitationKind::Addresses, 2).await],
        "{CASE}: and the flag clears"
    );

    let absent = store
        .reconfirm(ids::HTUI_FEAT_1, ids::REQ_STO_1, CitationKind::Reserves)
        .await;
    assert!(
        is_missing_citation(
            &absent,
            ids::HTUI_FEAT_1,
            ids::REQ_STO_1,
            CitationKind::Reserves
        ),
        "{CASE}: a triple with no row is NotFound naming it, got {absent:?}"
    );
}

/// Plan D10: `uncite` tombstones, so the citation leaves both reads and a second `uncite` or a
/// `reconfirm` finds nothing live; `cite` upserts, reviving a tombstone at the current version
/// and overwriting `proposed_by_step_id`. `cite`'s refusals run item, requirement, step.
async fn uncite_tombstones_and_cite_revives<S: WriteStore>(store: &S) {
    const CASE: &str = "uncite_tombstones_and_cite_revives";

    store
        .uncite(ids::HTUI_FIX_1, ids::REQ_STO_1, CitationKind::Addresses)
        .await
        .expect(CASE);
    assert_eq!(
        store.item_requirements(ids::HTUI_FIX_1).await.expect(CASE),
        Vec::new(),
        "{CASE}: the tombstone leaves the item's citations"
    );
    assert_eq!(
        coverage_view(
            &store
                .requirement_coverage(ids::REQ_STO_1)
                .await
                .expect(CASE)
        ),
        vec![(ids::HTUI_FEAT_1, CitationKind::Addresses, 1, false)],
        "{CASE}: and the requirement's coverage"
    );
    let twice = store
        .uncite(ids::HTUI_FIX_1, ids::REQ_STO_1, CitationKind::Addresses)
        .await;
    assert!(
        is_missing_citation(
            &twice,
            ids::HTUI_FIX_1,
            ids::REQ_STO_1,
            CitationKind::Addresses
        ),
        "{CASE}: a tombstone is not uncited again, got {twice:?}"
    );
    let reconfirmed = store
        .reconfirm(ids::HTUI_FIX_1, ids::REQ_STO_1, CitationKind::Addresses)
        .await;
    assert!(
        is_missing_citation(
            &reconfirmed,
            ids::HTUI_FIX_1,
            ids::REQ_STO_1,
            CitationKind::Addresses
        ),
        "{CASE}: nor reconfirmed, got {reconfirmed:?}"
    );

    // Moved on first, so the revival's stamp is visibly the current version and not the old one.
    requirement_updated(
        CASE,
        store
            .amend_requirement(ids::REQ_STO_1, 1, body_patch("v2"), ids::HTUI_FEAT_3)
            .await
            .expect(CASE),
    );
    let revived = store
        .cite(
            ids::HTUI_FIX_1,
            ids::REQ_STO_1,
            CitationKind::Addresses,
            None,
        )
        .await
        .expect(CASE);
    assert_eq!(
        (
            revived.requirement_version,
            revived.proposed_by_step_id,
            revived.deleted_at
        ),
        (2, None, None),
        "{CASE}: cite revives the tombstone at the current version"
    );
    assert_eq!(
        store.item_requirements(ids::HTUI_FIX_1).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_STO_1, CitationKind::Addresses, 2).await],
        "{CASE}: and it is live again, not suspect"
    );
    let restepped = store
        .cite(
            ids::HTUI_FIX_1,
            ids::REQ_STO_1,
            CitationKind::Addresses,
            Some(ids::STEP_PRD),
        )
        .await
        .expect(CASE);
    assert_eq!(
        restepped.proposed_by_step_id,
        Some(ids::STEP_PRD),
        "{CASE}: a cite of a live row overwrites the proposing step"
    );
    assert_eq!(
        store
            .item_requirements(ids::HTUI_FIX_1)
            .await
            .expect(CASE)
            .iter()
            .map(|row| (row.kind, row.proposed_by_step_id))
            .collect::<Vec<_>>(),
        vec![(CitationKind::Addresses, Some(ids::STEP_PRD))],
        "{CASE}: still one row"
    );

    store
        .cite(
            ids::HTUI_FEAT_2,
            ids::REQ_ENT_2,
            CitationKind::Reserves,
            None,
        )
        .await
        .expect(CASE);
    assert_eq!(
        store.item_requirements(ids::HTUI_FEAT_2).await.expect(CASE),
        vec![citation(CASE, store, ids::REQ_ENT_2, CitationKind::Reserves, 1).await],
        "{CASE}: the fixture's tombstone revives too"
    );

    let both_unknown = store
        .cite(
            ItemId::new(),
            RequirementId::new(),
            CitationKind::Addresses,
            None,
        )
        .await;
    assert!(
        matches!(
            both_unknown,
            Err(StoreError::NotFound { entity: "item", .. })
        ),
        "{CASE}: an unknown item is NotFound first, got {both_unknown:?}"
    );
    let no_requirement = store
        .cite(
            ids::HTUI_FEAT_1,
            RequirementId::new(),
            CitationKind::Addresses,
            None,
        )
        .await;
    assert!(
        matches!(
            no_requirement,
            Err(StoreError::NotFound {
                entity: "requirement",
                ..
            })
        ),
        "{CASE}: an unknown requirement is NotFound, got {no_requirement:?}"
    );
    let no_step = store
        .cite(
            ids::HTUI_FEAT_1,
            ids::REQ_ENT_1,
            CitationKind::Addresses,
            Some(StepId::new()),
        )
        .await;
    assert!(
        matches!(no_step, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown step is Constraint, got {no_step:?}"
    );
}

/// ANA-11 §4.3: a requirement's coverage names each citing item with its status and, once it is
/// closed, its resolution, in item key order.
async fn coverage_lists_citing_items_with_resolution<S: WriteStore>(store: &S) {
    const CASE: &str = "coverage_lists_citing_items_with_resolution";

    store
        .cite(
            ids::HTUI_ANA_2,
            ids::REQ_STO_1,
            CitationKind::Addresses,
            None,
        )
        .await
        .expect(CASE);
    store
        .close_out(
            ids::HTUI_ANA_2,
            Resolution::Withdrawn,
            new_document(ids::HTUI_ANA_2, "summary", None),
            &[],
        )
        .await
        .expect(CASE);

    let coverage = store
        .requirement_coverage(ids::REQ_STO_1)
        .await
        .expect(CASE);
    assert_eq!(
        coverage
            .iter()
            .map(|row| (
                row.item.id,
                row.item.key.as_str(),
                row.item.status,
                row.resolution,
                row.kind,
                row.requirement_version,
                row.suspect,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                ids::HTUI_ANA_2,
                "ANA-2",
                Status::Closed,
                Some(Resolution::Withdrawn),
                CitationKind::Addresses,
                1,
                false
            ),
            (
                ids::HTUI_FEAT_1,
                "FEAT-1",
                Status::InProgress,
                None,
                CitationKind::Addresses,
                1,
                false
            ),
            (
                ids::HTUI_FIX_1,
                "FIX-1",
                Status::Closed,
                Some(Resolution::Done),
                CitationKind::Addresses,
                1,
                false
            ),
        ],
        "{CASE}: every citing item, with the resolution close-out wrote"
    );
}

/// Plan D9: the spec header is a compare-and-set on its own `version`. `None` creates it, a spent
/// token answers the row as it is, and `Some(_)` with no header is `NotFound`.
async fn spec_is_cas<S: WriteStore>(store: &S) {
    const CASE: &str = "spec_is_cas";

    assert_eq!(
        store.requirement_spec(ids::PROJECT_AGY).await.expect(CASE),
        None,
        "{CASE}: fixture precondition, agy has no header"
    );
    let early = store
        .set_requirement_spec(ids::PROJECT_AGY, Some(1), ids::USER, "p".to_owned())
        .await;
    assert!(
        matches!(
            early,
            Err(StoreError::NotFound {
                entity: "requirement_spec",
                ..
            })
        ),
        "{CASE}: a token with no header to compare against is NotFound, got {early:?}"
    );

    let v1 = applied(
        CASE,
        store
            .set_requirement_spec(ids::PROJECT_AGY, None, ids::USER, "p".to_owned())
            .await
            .expect(CASE),
    );
    assert_eq!(
        (v1.project_id, v1.owner_id, v1.preamble.as_str(), v1.version),
        (ids::PROJECT_AGY, ids::USER, "p", 1),
        "{CASE}: `None` creates the header at v1"
    );
    assert_eq!(
        stale(
            CASE,
            store
                .set_requirement_spec(ids::PROJECT_AGY, None, ids::USER, "again".to_owned())
                .await
                .expect(CASE),
        ),
        v1,
        "{CASE}: `None` against an existing header is Stale with the row as it is"
    );
    let v2 = applied(
        CASE,
        store
            .set_requirement_spec(ids::PROJECT_AGY, Some(1), ids::USER, "q".to_owned())
            .await
            .expect(CASE),
    );
    assert_eq!(
        (v2.preamble.as_str(), v2.version),
        ("q", 2),
        "{CASE}: the token matched, so the header moved to v2"
    );
    assert_eq!(
        stale(
            CASE,
            store
                .set_requirement_spec(ids::PROJECT_AGY, Some(1), ids::USER, "r".to_owned())
                .await
                .expect(CASE),
        ),
        v2,
        "{CASE}: the spent token answers v2 as it is"
    );
    assert_eq!(
        store.requirement_spec(ids::PROJECT_AGY).await.expect(CASE),
        Some(v2),
        "{CASE}: the header reads back at v2"
    );

    let fixture = crate::fixtures::demo_data().requirement_specs;
    assert_eq!(
        stale(
            CASE,
            store
                .set_requirement_spec(ids::PROJECT_HTUI, None, ids::USER, "x".to_owned())
                .await
                .expect(CASE),
        ),
        fixture[0],
        "{CASE}: htui's fixture header answers a `None` token as it is"
    );

    let orphan = store
        .set_requirement_spec(ProjectId::new(), None, ids::USER, "x".to_owned())
        .await;
    assert!(
        matches!(orphan, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown project is Constraint, got {orphan:?}"
    );
    let no_owner = store
        .set_requirement_spec(ids::PROJECT_VULKAN, None, UserId::new(), "x".to_owned())
        .await;
    assert!(
        matches!(no_owner, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown owner is Constraint, got {no_owner:?}"
    );
    assert_eq!(
        store
            .requirement_spec(ids::PROJECT_VULKAN)
            .await
            .expect(CASE),
        None,
        "{CASE}: and wrote nothing"
    );
}

/// Blueprint F1: a project delete takes six requirement tables, and [`DeleteReach`] counts every
/// one of them - a citation from another project's item included, which goes with the
/// requirement it cites, as a cross-project link goes with either end.
async fn project_delete_counts_requirements<S: WriteStore>(store: &S) {
    const CASE: &str = "project_delete_counts_requirements";

    store
        .cite(
            ids::AGY_FEAT_1,
            ids::REQ_STO_1,
            CitationKind::Addresses,
            None,
        )
        .await
        .expect(CASE);
    let reach = store
        .delete_reach(DeleteTarget::Project(ids::PROJECT_HTUI))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the fixture project has a reach"));
    let report = store.delete_project(ids::PROJECT_HTUI).await.expect(CASE);
    assert_eq!(
        report, reach,
        "{CASE}: the counts shown before the act are the counts the act took (PRD D13)"
    );
    assert_eq!(
        (
            report.requirement_specs,
            report.requirement_areas,
            report.requirement_key_counters,
            report.requirements,
            report.requirement_revisions,
            report.item_requirements,
        ),
        (1, 2, 2, 3, 4, 6),
        "{CASE}: the fixture's requirement set, plus agy's citation of R-STO-1"
    );

    assert_eq!(
        store.item_requirements(ids::AGY_FEAT_1).await.expect(CASE),
        Vec::new(),
        "{CASE}: agy's citation went with the requirement it cited"
    );
    assert_eq!(
        store.requirement(ids::REQ_STO_1).await.expect(CASE),
        None,
        "{CASE}: the requirement is gone"
    );
    assert_eq!(
        store
            .requirement_areas(ids::PROJECT_HTUI)
            .await
            .expect(CASE),
        Vec::new(),
        "{CASE}: and its areas"
    );
    assert_eq!(
        store.requirement_spec(ids::PROJECT_HTUI).await.expect(CASE),
        None,
        "{CASE}: and its header"
    );
}

/// MOD-4 milestone 2, plan D7: the run's terminal move and the item's mirror are one act
/// (ANA-2 §4.3's verdict table and the propagation rule at `:660-664`).
///
/// Six legs, in the order the writer's own checks run. The one the plan's Risks row names is the
/// second: an item whose *other* run is still live stays where it is, so a store that derived the
/// item from the finishing run alone would lose the second run. Getting two live graph runs onto
/// one item takes a detour, because `create_run` admits only `open | failed` items and its own
/// move leaves the item at `queued` — so the item is walked back `queued -> open` between the two
/// creates, which is a sanctioned pair (`item.rs:46-60`) and the only way through the seam as
/// milestone 1 shipped it.
///
/// The Postgres twin is `pg_criteria.rs::finish_run_holds_the_item_while_another_run_is_live`,
/// which repeats that leg against a real server and adds the `closed_at` column this seam sets
/// but does not project.
async fn finish_run_moves_run_and_item_together<S: WriteStore>(store: &S) {
    const CASE: &str = "finish_run_moves_run_and_item_together";
    let owner = Uuid::now_v7();
    let at = seam_clock();
    let until = at + TimeDelta::minutes(5);

    // Leg 1: `done` reaches the item, and the finish stamps the caller's instant.
    let before = item_row(CASE, store, ids::HTUI_ANA_2).await;
    let first = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(first, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: the run is admitted, so the item is in_progress"
    );
    store
        .finish_run(first, RunStatus::Done, None, at)
        .await
        .expect(CASE);
    let finished = run_row(CASE, store, first).await;
    assert_eq!(finished.status, RunStatus::Done, "{CASE}: the run is done");
    assert_eq!(
        finished.finished_at,
        Some(at),
        "{CASE}: `finished_at` is the caller's clock, not the store's"
    );
    assert_eq!(
        finished.failure, None,
        "{CASE}: a `done` run has no failure"
    );
    let item = item_row(CASE, store, ids::HTUI_ANA_2).await;
    assert_eq!(
        item.status,
        Status::Done,
        "{CASE}: the item moved with the run"
    );
    assert!(
        item.closed_at.is_some(),
        "{CASE}: `done` is terminal, so `closed_at` follows"
    );
    assert_eq!(
        item.version, before.version,
        "{CASE}: a transition writes no revision, so the version stands"
    );

    // Leg 2: the item is held while another run of it is still live (the plan's Risks row).
    let held = ids::AGY_FEAT_1;
    let left = store
        .create_run(new_run(ids::PROJECT_AGY, held, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert!(
        store
            .transition(held, Status::Queued, Status::Open)
            .await
            .expect(CASE),
        "{CASE}: the item is walked back so a second run can be queued on it"
    );
    let right = store
        .create_run(new_run(ids::PROJECT_AGY, held, Vec::new()))
        .await
        .expect(CASE)
        .id;
    for run in [left, right] {
        assert_eq!(
            store
                .claim_run(run, ids::BOX, owner, at, until)
                .await
                .expect(CASE),
            Claim::Admitted,
            "{CASE}: both runs fit the box's two slots"
        );
    }
    assert_eq!(
        item_row(CASE, store, held).await.status,
        Status::InProgress,
        "{CASE}: the first claim took the item to in_progress"
    );
    store
        .finish_run(left, RunStatus::Done, None, at)
        .await
        .expect(CASE);
    assert_eq!(
        item_row(CASE, store, held).await.status,
        Status::InProgress,
        "{CASE}: the item is held while its other run is still live"
    );
    store
        .finish_run(right, RunStatus::Done, None, at)
        .await
        .expect(CASE);
    assert_eq!(
        item_row(CASE, store, held).await.status,
        Status::Done,
        "{CASE}: the last run out moves the item"
    );

    // Leg 3: `failed` carries its reason onto the run and its verdict onto the item.
    let failing = store
        .create_run(new_run(ids::PROJECT_AGY, ids::AGY_FIX_1, Vec::new()))
        .await
        .expect(CASE)
        .id;
    assert_eq!(
        store
            .claim_run(failing, ids::BOX, owner, at, until)
            .await
            .expect(CASE),
        Claim::Admitted,
        "{CASE}: the box is free again"
    );
    store
        .finish_run(failing, RunStatus::Failed, Some("missing_output"), at)
        .await
        .expect(CASE);
    let failed = run_row(CASE, store, failing).await;
    assert_eq!(failed.status, RunStatus::Failed, "{CASE}: the run failed");
    assert_eq!(
        failed.failure.as_deref(),
        Some("missing_output"),
        "{CASE}: the reason is the caller's, written in the same act"
    );
    let failed_item = item_row(CASE, store, ids::AGY_FIX_1).await;
    assert_eq!(
        failed_item.status,
        Status::Failed,
        "{CASE}: in_progress -> failed mirrors the run"
    );
    assert_eq!(
        failed_item.closed_at, None,
        "{CASE}: `failed` is not terminal for an item, so `closed_at` stays clear"
    );

    // Leg 4: a `cancelled` run releases its item rather than ending it, from `queued` and with no
    // claim in between — the only row of the table whose `from` is `queued`.
    let cancelled = store
        .create_run(new_run(ids::PROJECT_VULKAN, ids::VULKAN_TOOL_1, Vec::new()))
        .await
        .expect(CASE)
        .id;
    store
        .finish_run(cancelled, RunStatus::Cancelled, None, at)
        .await
        .expect(CASE);
    assert_eq!(
        run_row(CASE, store, cancelled).await.status,
        RunStatus::Cancelled,
        "{CASE}: the run is cancelled"
    );
    assert_eq!(
        item_row(CASE, store, ids::VULKAN_TOOL_1).await.status,
        Status::Open,
        "{CASE}: a cancelled run hands the item back to the backlog"
    );

    // Leg 5: the three refusals, each decided before any write.
    let live = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_CLEAN_1, Vec::new()))
        .await
        .expect(CASE)
        .id;
    for (to, failure, fragment) in [
        (RunStatus::Running, None, "terminal status"),
        (RunStatus::Done, Some("x"), "refused on a move to `done`"),
        (RunStatus::Failed, None, "needs a failure text"),
    ] {
        let refused = store.finish_run(live, to, failure, at).await;
        let Err(StoreError::Constraint(sentence)) = refused else {
            panic!("{CASE}: `finish_run({to}, {failure:?})` is Constraint, got {refused:?}")
        };
        assert!(
            sentence.contains(fragment),
            "{CASE}: the refusal says `{fragment}`, got `{sentence}`"
        );
    }
    assert_eq!(
        run_row(CASE, store, live).await.status,
        RunStatus::Queued,
        "{CASE}: a refused finish writes nothing to the run"
    );
    assert_eq!(
        item_row(CASE, store, ids::HTUI_CLEAN_1).await.status,
        Status::Queued,
        "{CASE}: nor to its item"
    );

    // Leg 6: plan D14's precedence — the row is looked up before the target is judged, so an
    // unknown run answers `NotFound` even when `to` is also wrong.
    let unknown = store
        .finish_run(RunId::new(), RunStatus::Running, None, at)
        .await;
    assert!(
        matches!(unknown, Err(StoreError::NotFound { entity: "run", .. })),
        "{CASE}: an unknown run is NotFound before the terminal check, got {unknown:?}"
    );

    // Leg 7: a terminal run reaches nothing, `done -> done` included.
    let again = store.finish_run(first, RunStatus::Done, None, at).await;
    let Err(StoreError::Constraint(sentence)) = again else {
        panic!("{CASE}: finishing a finished run is Constraint, got {again:?}")
    };
    assert!(
        sentence.contains(RunStatus::Done.as_str()) && sentence.contains("§4.3"),
        "{CASE}: the refusal is `illegal_move`'s sentence, got `{sentence}`"
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

/// The fixture's `run` row for one id, so a case asserts against the seed rather than against a
/// copy of it a fixture edit would leave stale.
fn fixture_run(id: RunId) -> Run {
    crate::fixtures::demo_data()
        .runs
        .into_iter()
        .find(|row| row.id == id)
        .expect("the fixture holds that run")
}

/// The fixture's `run_step` rows of one run, in the order [`ReadStore::run_steps`] promises.
fn fixture_steps(run: RunId) -> Vec<RunStep> {
    let mut rows: Vec<RunStep> = crate::fixtures::demo_data()
        .steps
        .into_iter()
        .filter(|row| row.run_id == run)
        .collect();
    rows.sort_by_key(|row| (row.position, row.attempt, row.fanout_index));
    rows
}

/// `run` and `run_steps` answer the seeded rows whole, in `(position, attempt, fanout_index)`
/// order, and a list read is total where a row read is optional.
///
/// Leg (g) is the three-builder pin: `RunStepSummary` is assembled separately by `MemStore`, by
/// `PgStore`'s SQL and by the mirror's, and the only thing that keeps the three from drifting is a
/// case that reads the same columns both ways on all three. `updated_at` is deliberately taken
/// from the answer rather than compared: it is the trigger's on Postgres and the store's own in
/// memory, and is never asserted equal across backends (blueprint F-S).
async fn run_and_steps_round_trip<S: ReadStore>(store: &S) {
    const CASE: &str = "run_and_steps_round_trip";
    let seeded = fixture_run(ids::RUN_1);
    let found = run_row(CASE, store, ids::RUN_1).await;
    assert_eq!(
        found,
        Run {
            updated_at: found.updated_at,
            ..seeded
        },
        "{CASE}: every column of the seeded run reads back"
    );
    assert!(
        found.repo_scope.is_empty(),
        "{CASE}: a run seeded before 0003 declares no scope"
    );
    assert_eq!(
        (found.lease_box_id, found.lease_expires_at),
        (None, None),
        "{CASE}: and holds no lease"
    );
    let snapshot: GraphSnapshot = serde_json::from_value(
        found
            .graph_snapshot
            .clone()
            .unwrap_or_else(|| panic!("{CASE}: a graph run carries its snapshot")),
    )
    .unwrap_or_else(|error| panic!("{CASE}: the snapshot decodes as §5.1's type, got {error}"));
    assert_eq!(snapshot.v, 1, "{CASE}: at the version this crate writes");
    assert_eq!(
        snapshot.phases.len(),
        4,
        "{CASE}: the `feature` graph's four phases survive the round trip through JSONB"
    );

    assert_eq!(
        run_row(CASE, store, ids::RUN_2).await.status,
        RunStatus::Queued,
        "{CASE}: the fixture's only active run"
    );
    assert_eq!(
        run_row(CASE, store, ids::RUN_3).await.item_id,
        Some(ids::HTUI_ANA_1),
        "{CASE}: the fan-out run belongs to ANA-1"
    );
    assert_eq!(
        store.run(RunId::new()).await.expect(CASE),
        None,
        "{CASE}: an unknown run is None, not an error"
    );

    let steps = store.run_steps(ids::RUN_1).await.expect(CASE);
    assert_eq!(
        steps.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![
            ids::STEP_PRD,
            ids::STEP_PLAN,
            ids::STEP_IMPL,
            ids::STEP_REVIEW
        ],
        "{CASE}: position order"
    );
    assert!(
        steps.iter().all(|row| row.attempt == 1),
        "{CASE}: ANA-2 §4.4 counts attempts from one (plan D13)"
    );
    assert!(
        steps
            .iter()
            .all(|row| row.verify_outcome.is_none() && row.promoted_at.is_none()),
        "{CASE}: the 0003 columns are NULL on a row seeded before them"
    );
    assert_eq!(
        steps,
        fixture_steps(ids::RUN_1)
            .into_iter()
            .zip(&steps)
            .map(|(seeded, found)| RunStep {
                updated_at: found.updated_at,
                ..seeded
            })
            .collect::<Vec<_>>(),
        "{CASE}: every other column of every seeded step reads back"
    );

    let fanout = store.run_steps(ids::RUN_3).await.expect(CASE);
    assert_eq!(
        fanout
            .iter()
            .map(|row| (row.id, row.fanout_index, row.selected, row.status))
            .collect::<Vec<_>>(),
        vec![
            (ids::STEP_R3_RESEARCH_A, 0, Some(true), StepStatus::Done),
            (
                ids::STEP_R3_RESEARCH_B,
                1,
                Some(false),
                StepStatus::Superseded
            ),
        ],
        "{CASE}: the winner sorts first and only it is selected"
    );
    assert!(
        store.run_steps(RunId::new()).await.expect(CASE).is_empty(),
        "{CASE}: an unknown run has no steps, and is not NotFound"
    );

    for item in [ids::HTUI_ANA_1, ids::HTUI_FEAT_1] {
        for summary in store.runs(item).await.expect(CASE) {
            let rows = store.run_steps(summary.id).await.expect(CASE);
            for listed in &summary.steps {
                let row = rows
                    .iter()
                    .find(|row| row.id == listed.id)
                    .unwrap_or_else(|| panic!("{CASE}: step {} is a row of its run", listed.id));
                assert_eq!(
                    (
                        listed.usage.clone(),
                        listed.selected,
                        listed.exit_code,
                        listed.verify_outcome,
                        listed.promoted_at,
                    ),
                    (
                        row.usage.clone(),
                        row.selected,
                        row.exit_code,
                        row.verify_outcome,
                        row.promoted_at,
                    ),
                    "{CASE}: the summary projection and the row agree on step {}",
                    listed.id
                );
                let expected = match row.agent_id {
                    Some(id) if id == ids::AGENT_CLAUDE => Some("claude"),
                    Some(id) if id == ids::AGENT_AGY => Some("agy"),
                    _ => None,
                };
                assert_eq!(
                    listed.agent_name.as_deref(),
                    expected,
                    "{CASE}: agent_name is denormalised from agent_id on step {}",
                    listed.id
                );
            }
        }
    }
}

/// `step_trees` and `step_commits` are total reads: an empty answer for a step with no rows, and
/// for an id nothing has, never `NotFound`.
///
/// The fixture seeds no `repo`, so no tree or commit row can exist here (blueprint F-P); row
/// content is `trees_and_commits_round_trip` on both writers and
/// `pg_criteria.rs::step_tree_rows_cascade_with_their_step` on Postgres.
///
/// That is the whole of what this case can claim, and it is less than its name suggests: both
/// writer twins are [`WriteStore`] cases, which a `CacheStore` cannot run, so a mirror that
/// answers `Ok(vec![])` unconditionally - one carrying neither table - passes this byte for byte.
/// The compensating control is T2's `crates/htui-store/tests/cache.rs` tree case, which is the
/// only thing pinning that the mirror carries `run_step_tree` and `run_step_commit` at all.
async fn trees_and_commits_read_back<S: ReadStore>(store: &S) {
    const CASE: &str = "trees_and_commits_read_back";
    for step in [
        ids::STEP_PRD,
        ids::STEP_PLAN,
        ids::STEP_IMPL,
        ids::STEP_REVIEW,
        ids::STEP_R2_PRD,
        ids::STEP_R3_RESEARCH_A,
        ids::STEP_R3_RESEARCH_B,
        StepId::new(),
    ] {
        assert_eq!(
            store.step_trees(step).await.expect(CASE),
            Vec::new(),
            "{CASE}: step {step} has no tree row, and asking is not an error"
        );
        assert_eq!(
            store.step_commits(step).await.expect(CASE),
            Vec::new(),
            "{CASE}: step {step} has no commit row, and asking is not an error"
        );
    }
}

/// ANA-2 §4.2's resolver on the seeded fan-out: this run's selected output wins, a loser's higher
/// version is skipped, and a kind the item has no eligible row for comes back as `None`.
///
/// The writer-side twin is `write_document_allocates_its_version`, which builds the same shape by
/// hand; this one proves the mirror answers it too.
async fn resolve_inputs_prefers_this_run_and_skips_losers<S: ReadStore>(store: &S) {
    const CASE: &str = "resolve_inputs_prefers_this_run_and_skips_losers";
    let asked = [
        "research".to_owned(),
        "verdict".to_owned(),
        "missing".to_owned(),
    ];
    let resolved = store
        .resolve_inputs(ids::HTUI_ANA_1, ids::RUN_3, &asked)
        .await
        .expect(CASE);
    assert_eq!(
        resolved
            .iter()
            .map(|row| (
                row.kind.as_str(),
                row.document.as_ref().map(|document| document.id)
            ))
            .collect::<Vec<_>>(),
        vec![
            ("research", Some(ids::DOC_ANA_1_RESEARCH_V2)),
            ("verdict", Some(ids::DOC_ANA_1_VERDICT)),
            ("missing", None),
        ],
        "{CASE}: one entry per requested kind in request order; the loser's v3 is skipped"
    );
    assert_eq!(
        store
            .documents_of_kinds(ids::HTUI_ANA_1, &["research".to_owned()])
            .await
            .expect(CASE)
            .first()
            .map(|row| row.id),
        Some(ids::DOC_ANA_1_RESEARCH_V3),
        "{CASE}: documents_of_kinds ranks by version alone — plan D2's contrast"
    );

    let other_run = store
        .resolve_inputs(ids::HTUI_ANA_1, ids::RUN_1, &["research".to_owned()])
        .await
        .expect(CASE);
    assert_eq!(
        other_run
            .first()
            .and_then(|row| row.document.as_ref())
            .map(|row| row.id),
        Some(ids::DOC_ANA_1_RESEARCH_V2),
        "{CASE}: another run's selected output still outranks the hand-written v1"
    );

    let every_kind = store
        .resolve_inputs(ids::HTUI_ANA_1, ids::RUN_3, &[])
        .await
        .expect(CASE);
    assert_eq!(
        every_kind
            .iter()
            .map(|row| row.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["research", "summary", "verdict"],
        "{CASE}: an empty `kinds` is every kind the item has, in byte order"
    );
    assert!(
        every_kind.iter().all(|row| row.document.is_some()),
        "{CASE}: each of those kinds has an eligible row"
    );

    let not_a_loser = store
        .resolve_inputs(ids::HTUI_FEAT_1, ids::RUN_1, &["plan".to_owned()])
        .await
        .expect(CASE);
    assert_eq!(
        not_a_loser
            .first()
            .and_then(|row| row.document.as_ref())
            .map(|row| row.id),
        Some(ids::DOC_FEAT_1_PLAN_V2),
        "{CASE}: a NULL `selected` is not a loser, so the latest version of this run's output wins"
    );

    let unknown = store
        .resolve_inputs(ItemId::new(), ids::RUN_1, &["x".to_owned()])
        .await
        .expect(CASE);
    assert_eq!(
        unknown.len(),
        1,
        "{CASE}: an unknown item still answers one entry per requested kind"
    );
    assert!(
        unknown.first().is_some_and(|row| row.document.is_none()),
        "{CASE}: and resolves it to None rather than refusing"
    );
}

// ---- MOD-38 (plan D12): the requirement reads, over the fixture ------------------------------

/// The fixture's requirement row `id`, as [`crate::fixtures::demo_data`] builds it.
fn fixture_requirement(id: RequirementId) -> Requirement {
    crate::fixtures::demo_data()
        .requirements
        .into_iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("requirement {id} is a fixture row"))
}

/// A fixture citation as [`ReadStore::item_requirements`] answers it, `suspect` derived.
fn fixture_citation(requirement: RequirementId, kind: CitationKind, stamp: i32) -> ItemCitation {
    let requirement = fixture_requirement(requirement);
    let suspect = requirement.makes_suspect(stamp);
    ItemCitation {
        requirement,
        kind,
        requirement_version: stamp,
        proposed_by_step_id: None,
        suspect,
    }
}

/// The spec header and the areas read back as the fixture wrote them; a project with none
/// answers `None` and an empty list rather than an error.
async fn requirement_spec_and_areas_read_back<S: ReadStore>(store: &S) {
    const CASE: &str = "requirement_spec_and_areas_read_back";
    let data = crate::fixtures::demo_data();

    assert_eq!(
        store.requirement_spec(ids::PROJECT_HTUI).await.expect(CASE),
        data.requirement_specs.first().cloned(),
        "{CASE}: htui's header, v1, owned by the fixture user"
    );
    assert_eq!(
        store.requirement_spec(ids::PROJECT_AGY).await.expect(CASE),
        None,
        "{CASE}: agy has no header"
    );
    let areas = store
        .requirement_areas(ids::PROJECT_HTUI)
        .await
        .expect(CASE);
    assert_eq!(
        areas
            .iter()
            .map(|row| (row.code.as_str(), row.position))
            .collect::<Vec<_>>(),
        vec![("ENT", 0), ("STO", 1)],
        "{CASE}: htui's two areas in position order"
    );
    assert_eq!(
        areas, data.requirement_areas,
        "{CASE}: each area reads back whole"
    );
    assert_eq!(
        store.requirement_areas(ids::PROJECT_AGY).await.expect(CASE),
        Vec::new(),
        "{CASE}: agy has no areas"
    );
}

/// Plan D14: every [`RequirementFilter`] field is a conjunct, `None` does not filter, and the
/// text filter is a case-insensitive literal substring of the key or the body. Rows come in
/// `(area_code, number)` order.
async fn requirements_filter_by_area_state_priority_and_text<S: ReadStore>(store: &S) {
    const CASE: &str = "requirements_filter_by_area_state_priority_and_text";
    let keys = |rows: &[Requirement]| rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>();

    let all = store
        .requirements(ids::PROJECT_HTUI, &RequirementFilter::default())
        .await
        .expect(CASE);
    assert_eq!(
        all,
        [ids::REQ_ENT_1, ids::REQ_ENT_2, ids::REQ_STO_1]
            .into_iter()
            .map(fixture_requirement)
            .collect::<Vec<_>>(),
        "{CASE}: the default filter is every requirement, whole, in key order"
    );
    for (filter, expected, why) in [
        (
            RequirementFilter {
                area_codes: Some(vec!["STO".to_owned()]),
                ..RequirementFilter::default()
            },
            vec!["R-STO-1"],
            "one area",
        ),
        (
            RequirementFilter {
                priorities: Some(vec![Priority::Later]),
                ..RequirementFilter::default()
            },
            vec!["R-ENT-2"],
            "one priority",
        ),
        (
            RequirementFilter {
                states: Some(vec![RequirementState::Withdrawn]),
                ..RequirementFilter::default()
            },
            Vec::new(),
            "no withdrawn requirement in the fixture",
        ),
        (
            RequirementFilter {
                text: Some("r-ent".to_owned()),
                ..RequirementFilter::default()
            },
            vec!["R-ENT-1", "R-ENT-2"],
            "a key match, case-insensitive",
        ),
        (
            RequirementFilter {
                text: Some("MIRROR".to_owned()),
                ..RequirementFilter::default()
            },
            vec!["R-STO-1"],
            "a body match, case-insensitive",
        ),
        (
            RequirementFilter {
                area_codes: Some(vec!["ENT".to_owned()]),
                priorities: Some(vec![Priority::Must]),
                ..RequirementFilter::default()
            },
            vec!["R-ENT-1"],
            "two fields are a conjunction",
        ),
    ] {
        assert_eq!(
            keys(
                &store
                    .requirements(ids::PROJECT_HTUI, &filter)
                    .await
                    .expect(CASE)
            ),
            expected,
            "{CASE}: {why}"
        );
    }
    assert_eq!(
        store
            .requirements(ids::PROJECT_AGY, &RequirementFilter::default())
            .await
            .expect(CASE),
        Vec::new(),
        "{CASE}: agy has none"
    );

    for row in &all {
        assert_eq!(
            store.requirement(row.id).await.expect(CASE).as_ref(),
            Some(row),
            "{CASE}: {} reads the same alone as in the list",
            row.key
        );
    }
    assert_eq!(
        store.requirement(RequirementId::new()).await.expect(CASE),
        None,
        "{CASE}: an unknown id is None"
    );
}

/// Plan D11 on the fixture: `ANA-1` cites `R-ENT-1` at v1 while it stands at v2, so that one
/// citation is suspect; `ANA-2`'s `amends` is at v2 and is not; a tombstone is not a citation.
async fn item_citations_derive_suspect<S: ReadStore>(store: &S) {
    const CASE: &str = "item_citations_derive_suspect";

    for (item, expected, why) in [
        (
            ids::HTUI_ANA_1,
            vec![fixture_citation(ids::REQ_ENT_1, CitationKind::Addresses, 1)],
            "stamped at v1 against v2: suspect",
        ),
        (
            ids::HTUI_ANA_2,
            vec![fixture_citation(ids::REQ_ENT_1, CitationKind::Amends, 2)],
            "the deciding item's citation is at the current version",
        ),
        (
            ids::HTUI_FEAT_1,
            vec![fixture_citation(ids::REQ_STO_1, CitationKind::Addresses, 1)],
            "current",
        ),
        (ids::HTUI_FEAT_2, Vec::new(), "a tombstone is not listed"),
        (ids::AGY_FEAT_1, Vec::new(), "an item that cites nothing"),
        (
            ItemId::new(),
            Vec::new(),
            "an unknown item is empty, not an error",
        ),
    ] {
        assert_eq!(
            store.item_requirements(item).await.expect(CASE),
            expected,
            "{CASE}: {why}"
        );
    }
    assert!(
        fixture_citation(ids::REQ_ENT_1, CitationKind::Addresses, 1).suspect,
        "{CASE}: the fixture's one suspect citation (precondition)"
    );
}

/// ANA-11 §4.3's coverage on the fixture: each citing item's status and resolution, suspect
/// derived, in item key order; the tombstone is absent.
async fn coverage_carries_status_and_resolution<S: ReadStore>(store: &S) {
    const CASE: &str = "coverage_carries_status_and_resolution";
    let view = |rows: Vec<CoverageRow>| {
        rows.into_iter()
            .map(|row| {
                (
                    row.item.id,
                    row.item.status,
                    row.resolution,
                    row.kind,
                    row.requirement_version,
                    row.suspect,
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        view(
            store
                .requirement_coverage(ids::REQ_ENT_1)
                .await
                .expect(CASE)
        ),
        vec![
            (
                ids::HTUI_ANA_1,
                Status::Done,
                None,
                CitationKind::Addresses,
                1,
                true
            ),
            (
                ids::HTUI_ANA_2,
                Status::Open,
                None,
                CitationKind::Amends,
                2,
                false
            ),
        ],
        "{CASE}: R-ENT-1 is addressed by ANA-1 (suspect) and was amended by ANA-2"
    );
    assert_eq!(
        view(
            store
                .requirement_coverage(ids::REQ_STO_1)
                .await
                .expect(CASE)
        ),
        vec![
            (
                ids::HTUI_FEAT_1,
                Status::InProgress,
                None,
                CitationKind::Addresses,
                1,
                false
            ),
            (
                ids::HTUI_FIX_1,
                Status::Closed,
                Some(Resolution::Done),
                CitationKind::Addresses,
                1,
                false
            ),
        ],
        "{CASE}: R-STO-1 is addressed by the open FEAT-1 and the closed FIX-1"
    );
    assert_eq!(
        store
            .requirement_coverage(ids::REQ_ENT_2)
            .await
            .expect(CASE),
        Vec::new(),
        "{CASE}: R-ENT-2's only citation is a tombstone"
    );
    assert_eq!(
        store
            .item(ids::HTUI_FIX_1)
            .await
            .expect(CASE)
            .and_then(|row| row.resolution),
        Some(Resolution::Done),
        "{CASE}: the item row carries the same resolution"
    );
}

/// Plan D12: a store that keeps revisions answers them in version order; the mirror keeps none
/// and says so with `None` rather than an empty history.
async fn requirement_revisions_or_not_cached<S: ReadStore>(store: &S) {
    const CASE: &str = "requirement_revisions_or_not_cached";
    let expected: Vec<RequirementRevision> = crate::fixtures::demo_data()
        .requirement_revisions
        .into_iter()
        .filter(|row| row.requirement_id == ids::REQ_ENT_1)
        .collect();
    assert_eq!(
        expected
            .iter()
            .map(|row| (row.version, row.reason.as_str(), row.amended_by_item_id))
            .collect::<Vec<_>>(),
        vec![(1, "created", None), (2, "amended", Some(ids::HTUI_ANA_2))],
        "{CASE}: fixture precondition"
    );

    match store
        .requirement_revisions(ids::REQ_ENT_1)
        .await
        .expect(CASE)
    {
        None => {}
        Some(rows) => assert_eq!(rows, expected, "{CASE}: R-ENT-1's two revisions, whole"),
    }
    let unknown = store
        .requirement_revisions(RequirementId::new())
        .await
        .expect(CASE);
    assert!(
        unknown.is_none_or(|rows| rows.is_empty()),
        "{CASE}: an unknown id has no history"
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
    ///
    /// A third shape is a failure rather than either: `<file>::<name>`, two snake_case halves
    /// joined by `::` where the right one reads like a test name. It is the near-miss of the
    /// second shape - MOD-4 shipped four of them - and it reaches neither arm, so it was skipped
    /// in silence while reading exactly like a checked delegation. Module and lint paths
    /// (`store::mem`, `clippy::type_complexity`) are not test names and stay skipped. `PENDING`
    /// below is the one sanctioned exemption, for a delegation whose target another task writes.
    #[test]
    fn every_cross_referenced_test_name_exists() {
        /// This module's own source: the text that was compiled, doc comments included.
        const SELF: &str = include_str!("conformance.rs");
        /// The sibling `MemStore` unit tests.
        const MEM: &str = include_str!("mem.rs");

        /// Names the doc comments above delegate to that the tree does not carry **yet**: each is
        /// a `pg_criteria.rs` test T2 of MOD-4 writes, against a column or a concurrency the
        /// `MemStore` half cannot show.
        ///
        /// An entry is an exemption from the assertion below and nothing else: the reference is
        /// still spelled `pg_criteria.rs::<name>`, so the moment T2 defines the fn the exemption
        /// goes stale and this test says so by name. Deleting the entry is then the whole of the
        /// work, and the delegation becomes checked for real. Writing the reference *without* the
        /// `.rs` instead - which is how these four shipped - is no longer possible: a span that
        /// joins two snake_case halves with `::` and whose right half reads like a test name is a
        /// failure below, because that shape reaches neither arm and was silently skipped.
        const PENDING: &[&str] = &[];

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
                if PENDING.contains(&name) {
                    assert!(
                        !defines(source, name),
                        "`{file}.rs` now defines `{name}`: delete its PENDING entry, which is \
                         what turns the reference into a checked one"
                    );
                    continue;
                }
                assert!(
                    defines(source, name),
                    "a doc comment names `{file}.rs::{name}`, but `{file}.rs` defines no such fn"
                );
                checked += 1;
            } else if let Some((left, right)) = token.split_once("::")
                && snake_case(left)
                && snake_case(right)
                && right.matches('_').count() >= 4
            {
                // `store::mem` and `clippy::type_complexity` are module and lint paths and stay
                // skipped; a right half shaped like a test name is a delegation that meant to be
                // checked and reached neither arm above.
                panic!(
                    "a doc comment names the test `{token}`, which this scanner cannot check: \
                     spell a test in another file with its file name and extension, as the doc \
                     comment on this fn describes"
                );
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
