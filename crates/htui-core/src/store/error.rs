//! The one error type every store returns.

use crate::model::ParseEnumError;

/// The `Result` the `docs/ANA-9.md` §6.1 signatures refer to.
pub type Result<T> = std::result::Result<T, StoreError>;

/// Why a store call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// A row the caller named does not exist.
    #[error("{entity} `{id}` not found")]
    NotFound {
        /// Table name, e.g. `"item"`.
        entity: &'static str,
        /// The identifier as text.
        id: String,
    },
    /// A rule of §4.1 / §5 was violated: unknown kind, cross-project kind, duplicate key, an
    /// attempted delete.
    #[error("constraint violated: {0}")]
    Constraint(String),
    /// The backend cannot write; the offline backend of MOD-6 is the only one that returns this.
    #[error("this backend is read-only ({0})")]
    ReadOnly(&'static str),
    /// The underlying driver failed; MOD-6 wraps `sqlx` here.
    #[error("store backend error: {0}")]
    Backend(String),
    /// A stored enum text is not in the `CHECK` list any more.
    #[error(transparent)]
    ParseEnum(#[from] ParseEnumError),
}
