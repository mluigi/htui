//! The store seam: the `ReadStore` / `WriteStore` split of `docs/ANA-9.md` §6.1.
//!
//! The concrete `Backend` enum the TUI holds lives in `htui-store`, not here: it names `PgStore`
//! and `CacheStore`, and this crate must stay free of `sqlx` (MOD-6 plan D1).

pub mod error;
pub mod graph_rules;
pub mod mem;
pub mod traits;
mod worker;
mod worker_log;

#[cfg(feature = "test-support")]
pub mod conformance;

pub use error::{Result, StoreError};
pub use graph_rules::{
    WalkEdge, WalkNode, body_agent_refs, body_persona_refs, default_edges_follow, edge_predicate,
    forward_mask, graph_body_shape_refusal, graph_body_topology_refusal, graph_body_walk_refusal,
    graph_usable_by, phase_row_refusal, walk_refusal,
};
pub use mem::MemStore;
pub use traits::{
    BLANK_PERSONA_BODY, BLANK_SKILL_BODY, BindingFacts, CasOutcome, DeleteReach, DeleteTarget,
    GLOB_NEEDS_GLOBS, MAX_LEASE_TTL, MAX_PARENT_DEPTH, MAX_UPSTREAM_HOPS,
    MEMORY_EVIDENCE_OUTSIDE_THE_RUN, MEMORY_NEEDS_A_CAPTURE_STEP, MODEL_REFUSED, ParkOutcome,
    PersonaNotInSnapshot, QueueStored, QueueTarget, QueueToken, RULE_MATCHES_EVERYTHING, ReadStore,
    SettingRung, StepFence, StoredAttachment, StoredSetting, TransitionLaw, UpdateOutcome,
    WriteStore, admission_plan_mismatch, allow_names_an_mcp_tool, already_exists,
    branch_is_not_an_object_of_booleans, branch_refusal, chat_step_status, check_attachment,
    citation_key, close_out_needs_a_summary, deadline_below_one, divide_columns_off_divide,
    edge_endpoint_outside_graph, edge_is_taken, edge_loop_limit_negative, edge_loops_to_itself,
    edge_needs_loop_limit, edge_predicate_mismatch, expected_on_row, failure_disagrees_with_status,
    fan_out_below_one, finish_run_needs_a_terminal_status, forward_edges_form_a_cycle,
    glob_names_unknown_repo, global_glob_names_a_repo, graph_name_is_taken, graph_not_usable,
    has_nul, if_predicate_outside_the_closed_set, illegal_move, invalid_area_code,
    invalid_persona_name, invalid_prefix, invalid_skill_name, invalid_template_name,
    item_has_a_live_run, item_kind_is_held, item_not_in_project, judge_model_without_agent,
    kind_not_narrowable, lease_ttl_micros, legal_move, link_outside_reach, loop_edge_runs_forward,
    max_parts_below_one, memory_entry_closed, memory_subject_taken, negative_position,
    new_persona_refusal, new_skill_refusal, not_a_fanout_candidate, not_a_terminal_status,
    not_a_tool_name, not_startable, override_outside_project, palette_phase_has_edges,
    parent_cycle, parent_too_deep, part_of_another_planner, part_of_another_run, parts_off_divide,
    persona_is_bound, persona_patch_refusal, persona_refusal, phase_attachment_needs_a_project,
    phase_name_is_taken, phase_name_refusal, phase_not_in_project, phase_part_key_is_taken,
    phase_position_is_taken, phase_token_budget_refusal, pin_names_no_version,
    plan_admission_refusal, plan_not_proposed, plan_not_this_planners, plan_revision_taken,
    plan_suffix_phase_name, planner_already_admitted, planner_not_in_run, planner_not_running,
    project_slug_is_reserved, project_slug_refusal, prompt_template_key, prompt_template_refusal,
    queue_target_refusal, queue_token_refusal, references_no_row, requirement_withdrawn,
    reserved_phase_name, resolution_not_closable, retry_limit_negative, row_names_another_graph,
    row_names_another_phase, row_names_another_step, rule_kind_unknown, run_is_terminal,
    run_plan_refusal, run_step_refusal, run_workspace_for, scope_row_is_deleted_with_its_workspace,
    scope_row_joins_no_workspace, scope_row_run_workspace, scope_row_slug_is_fixed,
    second_canonical, second_parent, skill_body_refusal, skill_patch_refusal, skill_version_key,
    split_fan_out_mismatch, startable, step_is_not_promotable, step_not_a_part, step_slot_is_taken,
    summary_names_another_item, winner_is_not_settled, withdrawn_requirement_cited, workspace_gone,
    workspace_graph_is_held, workspace_has_live_run, workspace_has_stopped_run,
    workspace_prefix_taken,
};
pub use worker::{RecorderStore, RelayStore, WorkerHost, WorkerStore};
pub use worker_log::WorkerLogStore;
