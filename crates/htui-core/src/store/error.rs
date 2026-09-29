//! The one error type every store returns.

use crate::model::{ParseEnumError, StepId};

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
    /// A step write named a lease its run does not carry (MOD-40 plan D1): another process
    /// adopted the run, or the fence names a lease on a run that has none, or none on a run that
    /// has one. Nothing was written, and a retry under the same fence answers the same: the writer
    /// has lost the step.
    #[error("run_step {step} is not writable under this lease")]
    Fenced {
        /// The step the write named; for a batch, the lowest-ordered fenced one.
        step: StepId,
    },
    /// The backend cannot write; the offline backend of MOD-6 is the only one that returns this.
    #[error("this backend is read-only ({0})")]
    ReadOnly(&'static str),
    /// The server cannot be reached: the socket, the pool or the connection itself is gone.
    ///
    /// The narrower half of [`StoreError::Backend`], and the only one that means *retrying later
    /// may work*. MOD-6's store worker treats it as the mid-session `Online` → `Offline`
    /// transition: it swaps the backend for the mirror, stops the refresher and lets the reconnect
    /// ticker re-dial. A query that is merely wrong stays [`StoreError::Backend`], because
    /// dropping to the mirror would not help and would hide the bug.
    #[error("store unreachable: {0}")]
    Unreachable(String),
    /// The underlying driver failed; MOD-6 wraps `sqlx` here. Everything that is neither a missing
    /// row, a violated constraint nor a lost connection.
    #[error("store backend error: {0}")]
    Backend(String),
    /// A stored enum text is not in the `CHECK` list any more.
    #[error(transparent)]
    ParseEnum(#[from] ParseEnumError),
}
