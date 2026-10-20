//! A workspace's stop record (MOD-125 D4; `0019_stop_runs.sql`). Not mirrored: read through the
//! store (`WriteStore::workspace_stop`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{UserId, WorkspaceId};

str_enum!(
    /// `workspace_stop.mode` (MOD-125 D4, PRD gate decisions).
    StopMode {
        /// Cancel the current turn cleanly and keep the session; resume continues it.
        Graceful => "graceful",
        /// Kill now; resume reopens the session and redoes the interrupted turn.
        Forced => "forced",
    }
);

/// One `workspace_stop` row (MOD-125 D4): its presence is the gate. "Stopping" vs "stopped" is
/// derived from the workspace's runs (M4), never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStop {
    /// `workspace_stop.workspace_id`.
    pub workspace_id: WorkspaceId,
    /// `workspace_stop.mode`; only ever escalates `graceful -> forced`.
    pub mode: StopMode,
    /// `workspace_stop.requested_by`: the latest request that wrote the row.
    pub requested_by: UserId,
    /// `workspace_stop.requested_at`: the caller's clock, microsecond-truncated.
    pub requested_at: DateTime<Utc>,
}
