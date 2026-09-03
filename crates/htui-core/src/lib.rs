//! Domain model and store seam for `htui`.
//!
//! The types in [`model`] mirror the Postgres schema of `docs/ANA-9.md` §5 column for column
//! (ANA-9 §3 conventions, plan D3): every `UUID` is an ID newtype, every `TEXT ... CHECK (... IN
//! ...)` column is a Rust enum whose variant order is the `CHECK` order, and every field carries
//! its column name verbatim. [`store`] holds the `ReadStore` / `WriteStore` seam of ANA-9 §6.1 and
//! the [`store::Backend`] enum the TUI owns.
#![warn(missing_docs)]

pub mod model;
pub mod store;

#[cfg(feature = "demo")]
pub mod fixtures;
