//! `sqlx` errors mapped onto the one [`StoreError`] every store returns.

use htui_core::store::StoreError;
use sqlx::migrate::MigrateError;

/// Maps a [`sqlx::Error`] onto the one [`StoreError`] every store returns.
///
/// SQLSTATE class `23` (integrity constraint violation) is [`StoreError::Constraint`], a missing
/// row is [`StoreError::NotFound`], everything else is [`StoreError::Backend`] (ANA-9 §6.1, MOD-1
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
