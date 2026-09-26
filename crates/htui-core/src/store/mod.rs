//! The store seam: the `ReadStore` / `WriteStore` split of `docs/ANA-9.md` §6.1.
//!
//! The concrete `Backend` enum the TUI holds lives in `htui-store`, not here: it names `PgStore`
//! and `CacheStore`, and this crate must stay free of `sqlx` (MOD-6 plan D1).

pub mod error;
pub mod mem;
pub mod traits;

#[cfg(feature = "test-support")]
pub mod conformance;

pub use error::{Result, StoreError};
pub use mem::MemStore;
pub use traits::{
    BLANK_SKILL_BODY, BindingFacts, CasOutcome, DeleteReach, DeleteTarget, GLOB_NEEDS_GLOBS,
    MAX_UPSTREAM_HOPS, ReadStore, SettingRung, StoredAttachment, StoredSetting, TransitionLaw,
    UpdateOutcome, WriteStore, already_exists, chat_step_status, check_attachment, citation_key,
    close_out_needs_a_summary, expected_on_row, failure_disagrees_with_status,
    finish_run_item_mirror, finish_run_needs_a_terminal_status, glob_names_unknown_repo,
    global_glob_names_a_repo, graph_not_in_project, has_nul, illegal_move, invalid_area_code,
    invalid_prefix, invalid_skill_name, invalid_template_name, item_has_a_live_run,
    item_kind_is_held, item_not_in_project, legal_move, negative_position, new_skill_refusal,
    not_a_fanout_candidate, not_a_terminal_status, phase_attachment_needs_a_project,
    phase_not_in_project, pin_names_no_version, prompt_template_key, prompt_template_refusal,
    references_no_row, requirement_withdrawn, reserved_phase_name, resolution_not_closable,
    row_names_another_step, run_is_terminal, skill_body_refusal, skill_patch_refusal,
    skill_version_key, step_is_not_promotable, step_slot_is_taken, summary_names_another_item,
    winner_is_not_settled, withdrawn_requirement_cited,
};
