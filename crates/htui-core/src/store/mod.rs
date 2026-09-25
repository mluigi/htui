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
    CasOutcome, DeleteReach, DeleteTarget, MAX_UPSTREAM_HOPS, ReadStore, SettingRung,
    StoredSetting, TransitionLaw, UpdateOutcome, WriteStore, already_exists, chat_step_status,
    citation_key, close_out_needs_a_summary, expected_on_row, failure_disagrees_with_status,
    finish_run_item_mirror, finish_run_needs_a_terminal_status, graph_not_in_project, illegal_move,
    invalid_area_code, invalid_prefix, item_has_a_live_run, item_kind_is_held, item_not_in_project,
    legal_move, not_a_fanout_candidate, not_a_terminal_status, references_no_row,
    requirement_withdrawn, reserved_phase_name, resolution_not_closable, row_names_another_step,
    run_is_terminal, step_is_not_promotable, step_slot_is_taken, summary_names_another_item,
    winner_is_not_settled, withdrawn_requirement_cited,
};
