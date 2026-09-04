//! Domain model and store seam for `htui`.
//!
//! The types in [`model`] mirror the Postgres schema of `docs/ANA-9.md` §5 column for column
//! (ANA-9 §3 conventions, plan D3): every `UUID` is an ID newtype, every `TEXT ... CHECK (... IN
//! ...)` column is a Rust enum whose variant order is the `CHECK` order, and every field carries
//! its column name verbatim. [`store`] holds the `ReadStore` / `WriteStore` seam of ANA-9 §6.1;
//! the concrete `Backend` enum the TUI owns lives in `htui-store`, which is the crate that may
//! name a database driver (MOD-6 plan D1).
#![warn(missing_docs)]

pub mod model;
pub mod store;

#[cfg(feature = "demo")]
pub mod fixtures;
