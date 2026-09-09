//! The one error type every driver returns.

use htui_core::scrub::Unmasked;
use htui_core::store::StoreError;

/// The `Result` the `docs/ANA-4.md` §4.1 signatures refer to.
pub type Result<T> = std::result::Result<T, DriverError>;

/// Why a driver call failed.
///
/// Mirrors `htui-core`'s [`StoreError`] in shape: one `thiserror` enum per crate, one variant per
/// thing a caller can actually do something about (plan MOD-2 "Patterns to Mirror").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DriverError {
    /// A `${name}` placeholder in `agent.launch` (§5.1) has no entry in the tool map the caller
    /// supplied. The box has not been probed, or the probe found nothing under that name.
    #[error("launch placeholder `{0}` is unresolved")]
    Unresolved(String),
    /// No transport builder is registered under this adapter id (plan D12). The id is derived
    /// from the `agent` row, never from `agent.name`, so this names a transport, not an agent.
    #[error("no adapter registered for `{0}`")]
    UnknownAdapter(String),
    /// The child process could not be started: the command is missing, not executable, or the
    /// operating system refused the spawn.
    #[error("agent spawn failed: {0}")]
    Spawn(String),
    /// The wire protocol failed: a malformed frame, a JSON-RPC error, a decode failure, or a
    /// transport that closed mid-turn.
    #[error("agent transport error: {0}")]
    Transport(String),
    /// The session is finished: no further event will arrive and no further call will be
    /// accepted. `AgentSession::next_event` reports the same state as `Ok(None)`; this variant is
    /// what the *other* four operations return once that has happened.
    #[error("agent session is closed")]
    Closed,
    /// This transport has no such operation (plan MOD-21 D10). The name is the trait method's, so
    /// the Settings section and the runtime can branch on the variant and print the sentence
    /// rather than matching on a message.
    #[error("this transport has no `{0}` operation")]
    Unsupported(&'static str),
    /// A store write the driver had to make failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Scrubbing refused a payload. `R-SEC-3` is fail-closed: the row is dropped rather than
    /// persisted, and the offending text is never carried in the error (`htui-core`'s
    /// [`Unmasked`] holds a JSON pointer and a rule name only).
    #[error(transparent)]
    Scrub(#[from] Unmasked),
}
