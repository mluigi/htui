//! `sqlx` errors mapped onto the one [`StoreError`] every store returns.

use htui_core::store::StoreError;
use sqlx::migrate::MigrateError;

/// Maps a [`sqlx::Error`] onto the one [`StoreError`] every store returns.
///
/// SQLSTATE class `23` (integrity constraint violation) is [`StoreError::Constraint`], a missing
/// row is [`StoreError::NotFound`], a lost connection is [`StoreError::Unreachable`] (see
/// [`is_unreachable`]) and everything else is [`StoreError::Backend`] (ANA-9 §6.1, MOD-1
/// blueprint B.5). The SQLSTATE prefix is the rule rather than `sqlx`'s own `ErrorKind`, which
/// collapses `23502` into `NotNullViolation` but leaves `23000` and `23001` as `Other`.
///
/// `23503` on `item.created_by` / `item_revision.author_id` is what makes the conformance case
/// `nil_author_rejected` pass against Postgres.
#[must_use]
pub fn map_sqlx(err: sqlx::Error) -> StoreError {
    map_sqlx_for("row", "", err)
}

/// [`map_sqlx`] with a caller-supplied entity name for the [`StoreError::NotFound`] arm.
#[must_use]
pub fn map_sqlx_for(
    entity: &'static str,
    id: impl core::fmt::Display,
    err: sqlx::Error,
) -> StoreError {
    if is_unreachable(&err) {
        return StoreError::Unreachable(err.to_string());
    }
    match err {
        sqlx::Error::RowNotFound => StoreError::NotFound {
            entity,
            id: id.to_string(),
        },
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default().into_owned();
            let text = match db.constraint() {
                Some(name) => format!("{name}: {}", db.message()),
                None => db.message().to_owned(),
            };
            // 23000 restrict, 23001 dependent-fields, 23502 not-null, 23503 foreign key,
            // 23505 unique, 23514 check, 23P01 exclusion.
            if code.starts_with("23") {
                StoreError::Constraint(text)
            } else {
                StoreError::Backend(format!("{code}: {text}"))
            }
        }
        other => StoreError::Backend(other.to_string()),
    }
}

/// Whether a [`sqlx::Error`] means "the server is not reachable" rather than "the query was wrong".
///
/// Four driver-side arms - [`sqlx::Error::Io`] (the socket), `PoolTimedOut` (no connection became
/// available in the acquire timeout), `PoolClosed` and `WorkerCrashed` - plus the server-side ones
/// that arrive as a `Database` error over a connection that is about to go away:
///
/// - SQLSTATE class `08`, *connection exception*: `08000`, `08003`, `08006`, `08001`, `08004`,
///   `08007`, `08P01`;
/// - `57P01` admin shutdown, `57P02` crash shutdown, `57P03` cannot connect now (the server is
///   starting up or in recovery).
///
/// `57014` (query cancelled) and `57P04` (database dropped) are deliberately **not** here: the
/// connection survives both, so dropping to the mirror would be wrong.
#[must_use]
pub fn is_unreachable(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Io(_)
        | sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::WorkerCrashed => true,
        sqlx::Error::Database(db) => db.code().is_some_and(|code| {
            code.starts_with("08") || matches!(&*code, "57P01" | "57P02" | "57P03")
        }),
        _ => false,
    }
}

/// Maps a [`MigrateError`] onto [`StoreError`], keeping the three refusals of ANA-9 §5.0 readable.
///
/// `VersionMissing`, `VersionMismatch` and `Dirty` all mean "this binary does not own this
/// schema": they refuse rather than repair (`R-STO-5`, plan D4). The texts match the ones
/// [`crate::PgStore::connect`] produces from its own check, so the two paths cannot disagree.
#[must_use]
pub fn map_migrate(err: MigrateError) -> StoreError {
    match err {
        MigrateError::VersionMissing(v) => StoreError::Backend(schema_is_newer(v)),
        MigrateError::VersionMismatch(v) => StoreError::Backend(checksum_drift(v)),
        MigrateError::Dirty(v) => StoreError::Backend(format!(
            "migration {v} is partially applied; fix it and remove its `_sqlx_migrations` row"
        )),
        MigrateError::Execute(inner) | MigrateError::ExecuteMigration(inner, _) => map_sqlx(inner),
        other => StoreError::Backend(other.to_string()),
    }
}

/// The refusal text for an applied migration this binary does not embed.
#[must_use]
pub fn schema_is_newer(version: i64) -> String {
    format!("schema is newer than this htui: migration {version} is applied but not embedded")
}

/// The refusal text for an applied migration whose recorded checksum has drifted.
#[must_use]
pub fn checksum_drift(version: i64) -> String {
    format!("migration {version} was applied with a different checksum")
}

#[cfg(test)]
mod tests {
    use super::{is_unreachable, map_sqlx, map_sqlx_for};
    use htui_core::store::StoreError;

    /// The four driver-side arms of [`is_unreachable`].
    ///
    /// The SQLSTATE half needs a real server to produce a `DatabaseError`, so it is covered by
    /// `tests/connect.rs::a_terminated_backend_is_an_unreachable_error`.
    #[test]
    fn the_driver_side_arms_are_unreachable() {
        for err in [
            sqlx::Error::PoolClosed,
            sqlx::Error::PoolTimedOut,
            sqlx::Error::WorkerCrashed,
            sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ] {
            assert!(is_unreachable(&err), "{err:?} is a lost connection");
            let text = err.to_string();
            assert_eq!(
                map_sqlx(err),
                StoreError::Unreachable(text),
                "and map_sqlx says so"
            );
        }
    }

    #[test]
    fn a_query_that_is_merely_wrong_is_not_unreachable() {
        for err in [
            sqlx::Error::Protocol("nonsense on the wire".to_owned()),
            sqlx::Error::ColumnNotFound("nope".to_owned()),
        ] {
            assert!(!is_unreachable(&err), "{err:?} is not a lost connection");
            assert!(matches!(map_sqlx(err), StoreError::Backend(_)));
        }
    }

    #[test]
    fn a_missing_row_still_names_its_entity() {
        assert!(!is_unreachable(&sqlx::Error::RowNotFound));
        assert_eq!(
            map_sqlx_for("item", "FEAT-1", sqlx::Error::RowNotFound),
            StoreError::NotFound {
                entity: "item",
                id: "FEAT-1".to_owned(),
            }
        );
    }
}
