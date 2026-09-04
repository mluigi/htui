//! Postgres store for `htui` (`docs/ANA-9.md` §5, §6).
//!
//! `htui-core` owns the domain model and the `ReadStore` / `WriteStore` seam; this crate owns the
//! concrete Postgres backend behind it, the box identity file the schema's `box` row is keyed on,
//! and (from MOD-6 T3 onwards) the per-box SQLite mirror the TUI reads while offline.
//!
//! Everything here is `async` and none of it may be called from the UI task (`R-NF-3`): the store
//! worker of the `htui` crate is the only caller.
#![warn(missing_docs)]

pub mod cache;
pub mod error;
pub mod identity;
pub mod pg;

pub use cache::{CacheMeta, CacheStore};
pub use error::map_sqlx;
pub use identity::Identity;
pub use pg::{Connected, MigrationState, PgStore};

/// The embedded Postgres schema (ANA-9 §5), applied by [`PgStore::apply_migrations`].
///
/// Forward-only: a later ANA adds `000N_*.sql` next to `0001_init.sql` and never edits it, because
/// editing an applied migration changes its checksum and [`PgStore::connect`] then refuses
/// (`R-STO-5`).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The embedded SQLite mirror schema (ANA-9 §4.4), applied by [`CacheStore::open`].
///
/// Independent of [`MIGRATOR`]: this one versions the *local* file, and a mismatch between
/// `cache_meta.schema_version` and [`PgStore::schema_version`] is what rebuilds it (plan D8), so
/// the mirror never has to migrate its own data forward.
pub static CACHE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./cache_migrations");
