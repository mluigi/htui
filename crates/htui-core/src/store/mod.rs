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
    MAX_UPSTREAM_HOPS, ReadStore, UpdateOutcome, WriteStore, chat_step_status,
    not_a_terminal_status,
};
