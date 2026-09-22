//! Postgres store for `htui` (`docs/ANA-9.md` §5, §6).
//!
//! `htui-core` owns the domain model and the `ReadStore` / `WriteStore` seam; this crate owns the
//! concrete Postgres backend behind it, the box identity file the schema's `box` row is keyed on,
//! the per-box SQLite mirror the TUI reads while offline, the keyring the DSN lives in, and the
//! [`Backend`] enum the TUI holds (moved here from `htui-core` in MOD-6 T4, because it names
//! [`PgStore`] and the domain crate must not pull `sqlx` in).
//!
//! Everything here is `async` and none of it may be called from the UI task (`R-NF-3`): the store
//! worker of the `htui` crate is the only caller.
#![warn(missing_docs)]

pub mod backend;
pub mod cache;
pub mod connect;
pub mod dsn;
pub mod embed;
pub mod error;
pub mod identity;
pub mod pg;
/// Module defining the QdrantDsn type and logic.
pub mod qdrant_settings;
pub mod secret;
#[cfg(feature = "test-support")]
pub mod testkit;
pub mod vector;
pub mod vector_sync;
pub mod writer;

pub use backend::Backend;
pub use cache::{CacheMeta, CacheStore};
pub use connect::{Applied, ConnEvent, ConnectContext, StartOptions, Started};
pub use dsn::{Dsn, DsnError};
pub use error::map_sqlx;
pub use identity::Identity;
pub use pg::{Connected, MigrationState, PgStore};
pub use writer::{DATABASE_UNREACHABLE, PROMPT_ON_SERVER_ONLY, REGISTRY_ON_SERVER_ONLY, Writer};

/// The hop ceiling of the amended §7.3 upstream walk, re-exported from where the trait it belongs
/// to is defined.
///
/// It was declared here as well as spelled `2` in `htui_core::store::mem`, so the two clamps that
/// have to agree could drift with nothing to catch it (T68, F-52 review, L2). One definition, next
/// to [`htui_core::store::ReadStore::upstream_summaries`]; this re-export keeps `htui_store::MAX_UPSTREAM_HOPS`
/// resolving for anything that already named it.
pub use htui_core::store::MAX_UPSTREAM_HOPS;

/// [`htui_core::store::ReadStore::documents_of_kinds`]' ordering, applied in Rust by both SQL
/// backends over the latest-per-kind rows their statement returned.
///
/// The caller's order is an arbitrary permutation no `ORDER BY` expresses, and it is the contract:
/// `docs/ANA-5.md` §4.7 rule 3 renders documents in the phase's `input_kinds` order and the prompt
/// digest is a function of that order. An empty `kinds` is every kind the item has in kind **byte**
/// order, for the reason
/// [`UpstreamEntry::sort_canonical`](htui_core::model::UpstreamEntry::sort_canonical) gives:
/// Postgres would otherwise order by collation and the mirror by bytes.
///
/// A kind the item has no row for is omitted rather than an error; a kind named twice is returned
/// twice, which is what `MemStore` does with the same input.
pub(crate) fn order_documents(
    latest: Vec<htui_core::model::Document>,
    kinds: &[String],
) -> Vec<htui_core::model::Document> {
    if kinds.is_empty() {
        let mut every = latest;
        every.sort_by(|left, right| left.kind.as_bytes().cmp(right.kind.as_bytes()));
        return every;
    }
    kinds
        .iter()
        .filter_map(|kind| {
            latest
                .iter()
                .find(|document| &document.kind == kind)
                .cloned()
        })
        .collect()
}

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
