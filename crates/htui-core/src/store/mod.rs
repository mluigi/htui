//! The store seam: the `ReadStore` / `WriteStore` split of `docs/ANA-9.md` §6.1 and the concrete
//! [`Backend`] the TUI holds.

pub mod backend;
pub mod error;
pub mod mem;
pub mod traits;

#[cfg(feature = "test-support")]
pub mod conformance;

pub use backend::Backend;
pub use error::{Result, StoreError};
pub use mem::MemStore;
pub use traits::{ReadStore, UpdateOutcome, WriteStore};
