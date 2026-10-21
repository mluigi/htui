//! The store seam: the `ReadStore` / `WriteStore` split of `docs/ANA-9.md` §6.1.
//!
//! The concrete `Backend` enum the TUI holds lives in `htui-store`, not here: it names `PgStore`
//! and `CacheStore`, and this crate must stay free of `sqlx` (MOD-6 plan D1).

pub mod error;
pub mod graph_rules;
pub mod mem;
pub mod traits;
mod worker;

#[cfg(feature = "test-support")]
pub mod conformance;

pub use error::{Result, StoreError};
pub use graph_rules::{
    body_agent_refs, body_persona_refs, graph_body_shape_refusal, graph_body_topology_refusal,
    phase_row_refusal,
};
pub use mem::MemStore;
pub use traits::{
    BLANK_PERSONA_BODY, BLANK_SKILL_BODY, BindingFacts, CasOutcome, DeleteReach, DeleteTarget,
    GLOB_NEEDS_GLOBS, MAX_LEASE_TTL, MAX_UPSTREAM_HOPS, MODEL_REFUSED, ParkOutcome,
    PersonaNotInSnapshot, QueueStored, QueueTarget, QueueToken, RULE_MATCHES_EVERYTHING, ReadStore,
    SettingRung, StepFence, StoredAttachment, StoredSetting, TransitionLaw, UpdateOutcome,
    WriteStore, allow_names_an_mcp_tool, already_exists, chat_step_status, check_attachment,
    citation_key, close_out_needs_a_summary, divide_columns_off_divide,
    edge_endpoint_outside_graph, edge_is_taken, edge_loop_limit_negative, edge_loops_to_itself,
    edge_needs_loop_limit, edge_predicate_mismatch, expected_on_row, failure_disagrees_with_status,
    finish_run_item_mirror, finish_run_needs_a_terminal_status, glob_names_unknown_repo,
    global_glob_names_a_repo, graph_name_is_taken, graph_not_in_project, has_nul, illegal_move,
    invalid_area_code, invalid_persona_name, invalid_prefix, invalid_skill_name,
    invalid_template_name, item_has_a_live_run, item_kind_is_held, item_not_in_project,
    kind_not_narrowable, lease_ttl_micros, legal_move, max_parts_below_one, negative_position,
    new_persona_refusal, new_skill_refusal, not_a_fanout_candidate, not_a_terminal_status,
    not_a_tool_name, override_outside_project, palette_phase_has_edges, parts_off_divide,
    persona_is_bound, persona_patch_refusal, persona_refusal, phase_attachment_needs_a_project,
    phase_name_is_taken, phase_not_in_project, phase_part_key_is_taken, phase_position_is_taken,
    pin_names_no_version, prompt_template_key, prompt_template_refusal, queue_target_refusal,
    queue_token_refusal, references_no_row, requirement_withdrawn, reserved_phase_name,
    resolution_not_closable, row_names_another_graph, row_names_another_phase,
    row_names_another_step, rule_kind_unknown, run_is_terminal, skill_body_refusal,
    skill_patch_refusal, skill_version_key, split_fan_out_mismatch, step_is_not_promotable,
    step_slot_is_taken, summary_names_another_item, winner_is_not_settled,
    withdrawn_requirement_cited,
};
pub use worker::{RecorderStore, RelayStore, WorkerHost, WorkerStore};
