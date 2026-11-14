//! A workspace's stop record (MOD-125 D4; `0019_stop_runs.sql`), and what the executor's poll reads
//! of it (M2). Not mirrored: read through the store (`WriteStore::workspace_stop`,
//! `WriteStore::pending_stops`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{RunId, UserId, WorkspaceId};
use crate::model::run::RunStatus;

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
/// derived from the workspace's runs (M4, [`WorkspaceStopState::of`]), never stored.
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

/// One stop this process applies (MOD-125 M2 D1): a `running` run it leases whose workspace has a
/// `workspace_stop` row, with that row's mode. Read by the command poll
/// ([`WriteStore::pending_stops`](crate::store::WriteStore::pending_stops)); never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingStop {
    /// `run.id`.
    pub run: RunId,
    /// `workspace_stop.mode` of the run's workspace, as read now: a forced upsert over a graceful
    /// stop shows on the next poll.
    pub mode: StopMode,
}

/// How a workspace reads (MOD-125 M4 D4; R-TUI-14's "stopping, stopped"): derived from its stop row
/// and its runs, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStopState {
    /// No `workspace_stop` row: runs claim and start as usual.
    NotStopped,
    /// A stop row and at least one `running` run: the executors are still applying the stop.
    Stopping,
    /// A stop row and no `running` run: the stop has settled.
    Stopped,
}

impl WorkspaceStopState {
    /// `NotStopped` without a row; with one, `Stopping` while any run is `Running`, else `Stopped`.
    /// Queued runs are held and parked ones wait at their gate: neither keeps a workspace stopping.
    pub fn of(stop: Option<&WorkspaceStop>, runs: impl IntoIterator<Item = RunStatus>) -> Self {
        match stop {
            None => Self::NotStopped,
            Some(_) if runs.into_iter().any(|status| status == RunStatus::Running) => {
                Self::Stopping
            }
            Some(_) => Self::Stopped,
        }
    }
}

impl core::fmt::Display for WorkspaceStopState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotStopped => "not stopped",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> WorkspaceStop {
        WorkspaceStop {
            workspace_id: WorkspaceId::new(),
            mode: StopMode::Graceful,
            requested_by: UserId::new(),
            requested_at: Utc::now(),
        }
    }

    #[test]
    fn a_workspace_without_a_stop_row_is_not_stopped() {
        assert_eq!(
            WorkspaceStopState::of(None, []),
            WorkspaceStopState::NotStopped
        );
        assert_eq!(
            WorkspaceStopState::of(None, RunStatus::ALL.iter().copied()),
            WorkspaceStopState::NotStopped,
            "whatever its runs"
        );
    }

    #[test]
    fn a_stopped_workspace_with_a_running_run_is_stopping() {
        let stop = row();
        assert_eq!(
            WorkspaceStopState::of(Some(&stop), [RunStatus::Running]),
            WorkspaceStopState::Stopping
        );
        assert_eq!(
            WorkspaceStopState::of(
                Some(&stop),
                [RunStatus::Stopped, RunStatus::Running, RunStatus::Queued]
            ),
            WorkspaceStopState::Stopping,
            "one running run among others"
        );
    }

    #[test]
    fn queued_parked_and_stopped_runs_leave_it_stopped() {
        let stop = row();
        assert_eq!(
            WorkspaceStopState::of(
                Some(&stop),
                [
                    RunStatus::Queued,
                    RunStatus::AwaitingApproval,
                    RunStatus::Stopped,
                ]
            ),
            WorkspaceStopState::Stopped
        );
        assert_eq!(
            WorkspaceStopState::of(Some(&stop), []),
            WorkspaceStopState::Stopped,
            "no runs"
        );
    }

    #[test]
    fn workspace_stop_states_display_their_words() {
        assert_eq!(WorkspaceStopState::NotStopped.to_string(), "not stopped");
        assert_eq!(WorkspaceStopState::Stopping.to_string(), "stopping");
        assert_eq!(WorkspaceStopState::Stopped.to_string(), "stopped");
    }
}
