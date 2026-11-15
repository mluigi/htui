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

/// One workspace's non-terminal runs by status and its stop record (MOD-116 M1 D3): what
/// [`WriteStore::workspace_run_counts`](crate::store::WriteStore::workspace_run_counts) answers
/// per workspace. Never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRunCounts {
    /// `workspace.id`.
    pub workspace_id: WorkspaceId,
    /// Its `queued` runs.
    pub queued: u32,
    /// Its `running` runs.
    pub running: u32,
    /// Its `awaiting_approval` (parked) runs.
    pub awaiting_approval: u32,
    /// Its `stopped` runs (MOD-125 D1): live, resumable, holding no slot.
    pub stopped: u32,
    /// Its `workspace_stop` row, if any.
    pub stop: Option<WorkspaceStop>,
}

impl WorkspaceRunCounts {
    /// Counts `statuses` (terminal ones ignored) for `workspace` with its stop row.
    pub fn tally(
        workspace: WorkspaceId,
        statuses: impl IntoIterator<Item = RunStatus>,
        stop: Option<WorkspaceStop>,
    ) -> Self {
        let mut counts = Self {
            workspace_id: workspace,
            queued: 0,
            running: 0,
            awaiting_approval: 0,
            stopped: 0,
            stop,
        };
        for status in statuses {
            let slot = match status {
                RunStatus::Queued => &mut counts.queued,
                RunStatus::Running => &mut counts.running,
                RunStatus::AwaitingApproval => &mut counts.awaiting_approval,
                RunStatus::Stopped => &mut counts.stopped,
                RunStatus::Done | RunStatus::Failed | RunStatus::Cancelled => continue,
            };
            *slot = slot.saturating_add(1);
        }
        counts
    }

    /// `queued + running + awaiting_approval` (D2's `live`; `stopped` excluded).
    #[must_use]
    pub const fn live(&self) -> u32 {
        self.queued
            .saturating_add(self.running)
            .saturating_add(self.awaiting_approval)
    }

    /// D2's `held`: `stopped`, plus `queued + awaiting_approval` while a stop row exists.
    #[must_use]
    pub const fn held(&self) -> u32 {
        if self.stop.is_some() {
            self.stopped
                .saturating_add(self.queued)
                .saturating_add(self.awaiting_approval)
        } else {
            self.stopped
        }
    }
}

/// How a workspace reads on the Workspaces tab (MOD-116 M1 D2; R-TUI-14). Derived per process from
/// whether it is this TUI's scope, its runs and its stop row; never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLifecycle {
    /// This TUI's scope, whatever its stop row says.
    Open {
        /// Its `running` runs.
        running: u32,
    },
    /// Not open, no stop row, and at least one queued, running or parked run.
    Background {
        /// Its `running` runs (may be 0: queued or parked runs keep it in the background).
        running: u32,
    },
    /// Not open, a stop row, and at least one `running` run: the executors are applying the stop.
    Stopping {
        /// Its `running` runs.
        running: u32,
    },
    /// Not open, and either a settled stop row or no live run (PRD Q4: idle reads stopped).
    Stopped {
        /// D2's `held`: the runs a resume would release.
        held: u32,
    },
}

impl WorkspaceLifecycle {
    /// D2's table: `open` wins; else a stop row reads `Stopping` while any run is `running`, else
    /// `Stopped`; without one a live run reads `Background`, none `Stopped`.
    #[must_use]
    pub const fn of(open: bool, counts: &WorkspaceRunCounts) -> Self {
        if open {
            Self::Open {
                running: counts.running,
            }
        } else if counts.stop.is_some() {
            if counts.running > 0 {
                Self::Stopping {
                    running: counts.running,
                }
            } else {
                Self::Stopped {
                    held: counts.held(),
                }
            }
        } else if counts.live() > 0 {
            Self::Background {
                running: counts.running,
            }
        } else {
            Self::Stopped {
                held: counts.held(),
            }
        }
    }

    /// The state's one-cell glyph: `●` open, `◐` background, `◌` stopping, `○` stopped.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Open { .. } => "●",
            Self::Background { .. } => "◐",
            Self::Stopping { .. } => "◌",
            Self::Stopped { .. } => "○",
        }
    }

    /// The state's word, without counts.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Open { .. } => "open",
            Self::Background { .. } => "background",
            Self::Stopping { .. } => "stopping…",
            Self::Stopped { .. } => "stopped",
        }
    }
}

/// The word and its counts, without the glyph: `open · 2 running` (only when > 0),
/// `background · 0 running` (always), `stopping…`, `stopped · 1 held` (only when > 0).
impl core::fmt::Display for WorkspaceLifecycle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.word())?;
        match *self {
            Self::Open { running } if running > 0 => write!(f, " · {running} running"),
            Self::Background { running } => write!(f, " · {running} running"),
            Self::Stopped { held } if held > 0 => write!(f, " · {held} held"),
            Self::Open { .. } | Self::Stopping { .. } | Self::Stopped { .. } => Ok(()),
        }
    }
}

/// The header's `+N in background ◐ n` (MOD-116 M1 D4): the workspaces other than the open one
/// with a `running` run (background or stopping), and their running runs in total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InBackground {
    /// The other workspaces with at least one `running` run.
    pub workspaces: u32,
    /// Their `running` runs, summed.
    pub running: u32,
}

impl InBackground {
    /// Counts `rows` other than `open` whose `running` is above zero.
    #[must_use]
    pub fn of(open: WorkspaceId, rows: &[WorkspaceRunCounts]) -> Self {
        rows.iter()
            .filter(|row| row.workspace_id != open && row.running > 0)
            .fold(Self::default(), |sum, row| Self {
                workspaces: sum.workspaces.saturating_add(1),
                running: sum.running.saturating_add(row.running),
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

    fn counts(
        queued: u32,
        running: u32,
        awaiting_approval: u32,
        stopped: u32,
        stop: Option<WorkspaceStop>,
    ) -> WorkspaceRunCounts {
        WorkspaceRunCounts {
            workspace_id: WorkspaceId::new(),
            queued,
            running,
            awaiting_approval,
            stopped,
            stop,
        }
    }

    #[test]
    fn the_lifecycle_table_reads_every_combination() {
        use WorkspaceLifecycle::{Background, Open, Stopped, Stopping};
        let table = [
            (
                "open, no row, no runs",
                true,
                counts(0, 0, 0, 0, None),
                Open { running: 0 },
            ),
            (
                "open, no row, running",
                true,
                counts(1, 2, 0, 0, None),
                Open { running: 2 },
            ),
            (
                "open, a stop row and running runs",
                true,
                counts(0, 2, 0, 0, Some(row())),
                Open { running: 2 },
            ),
            (
                "open, a settled stop row",
                true,
                counts(1, 0, 1, 1, Some(row())),
                Open { running: 0 },
            ),
            (
                "a stop row and a running run",
                false,
                counts(1, 1, 0, 1, Some(row())),
                Stopping { running: 1 },
            ),
            (
                "a stop row, queued, parked and stopped runs",
                false,
                counts(1, 0, 1, 1, Some(row())),
                Stopped { held: 3 },
            ),
            (
                "a stop row and no runs",
                false,
                counts(0, 0, 0, 0, Some(row())),
                Stopped { held: 0 },
            ),
            (
                "no row, only queued runs",
                false,
                counts(2, 0, 0, 0, None),
                Background { running: 0 },
            ),
            (
                "no row, only a parked run",
                false,
                counts(0, 0, 1, 0, None),
                Background { running: 0 },
            ),
            (
                "no row, running runs",
                false,
                counts(1, 3, 0, 1, None),
                Background { running: 3 },
            ),
            (
                "no row, no runs",
                false,
                counts(0, 0, 0, 0, None),
                Stopped { held: 0 },
            ),
            (
                "no row, one stopped run",
                false,
                counts(0, 0, 0, 1, None),
                Stopped { held: 1 },
            ),
        ];
        for (case, open, counts, want) in table {
            assert_eq!(WorkspaceLifecycle::of(open, &counts), want, "{case}");
        }
    }

    #[test]
    fn the_lifecycle_words_carry_their_counts() {
        use WorkspaceLifecycle::{Background, Open, Stopped, Stopping};
        let table = [
            (Open { running: 0 }, "●", "open", "open"),
            (Open { running: 2 }, "●", "open", "open · 2 running"),
            (
                Background { running: 0 },
                "◐",
                "background",
                "background · 0 running",
            ),
            (
                Background { running: 3 },
                "◐",
                "background",
                "background · 3 running",
            ),
            (Stopping { running: 1 }, "◌", "stopping…", "stopping…"),
            (Stopped { held: 0 }, "○", "stopped", "stopped"),
            (Stopped { held: 1 }, "○", "stopped", "stopped · 1 held"),
        ];
        for (state, glyph, word, display) in table {
            assert_eq!(state.glyph(), glyph, "{state:?}");
            assert_eq!(state.glyph().chars().count(), 1, "{state:?}: one cell");
            assert_eq!(state.word(), word, "{state:?}");
            assert_eq!(state.to_string(), display, "{state:?}");
        }
    }

    #[test]
    fn tally_ignores_terminal_runs_and_held_counts_queued_only_under_a_stop() {
        let workspace = WorkspaceId::new();
        let tallied = WorkspaceRunCounts::tally(workspace, RunStatus::ALL.iter().copied(), None);
        assert_eq!(
            tallied,
            WorkspaceRunCounts {
                workspace_id: workspace,
                queued: 1,
                running: 1,
                awaiting_approval: 1,
                stopped: 1,
                stop: None,
            },
            "done, failed and cancelled count nowhere"
        );
        assert_eq!(tallied.live(), 3, "stopped is not live");
        assert_eq!(
            tallied.held(),
            1,
            "without a stop only stopped runs are held"
        );

        let stop = WorkspaceStop {
            workspace_id: workspace,
            ..row()
        };
        let under_stop = WorkspaceRunCounts::tally(
            workspace,
            [
                RunStatus::Queued,
                RunStatus::Queued,
                RunStatus::AwaitingApproval,
                RunStatus::Running,
                RunStatus::Stopped,
                RunStatus::Done,
            ],
            Some(stop.clone()),
        );
        assert_eq!(under_stop.queued, 2);
        assert_eq!(under_stop.stop, Some(stop));
        assert_eq!(under_stop.live(), 4);
        assert_eq!(
            under_stop.held(),
            4,
            "stopped + queued + parked under a stop"
        );
        assert_eq!(
            WorkspaceRunCounts::tally(workspace, [], None),
            counts_for(workspace),
            "no runs: zeros"
        );
    }

    fn counts_for(workspace: WorkspaceId) -> WorkspaceRunCounts {
        WorkspaceRunCounts {
            workspace_id: workspace,
            queued: 0,
            running: 0,
            awaiting_approval: 0,
            stopped: 0,
            stop: None,
        }
    }

    #[test]
    fn in_background_counts_other_workspaces_with_running_runs() {
        let open = counts(0, 4, 0, 0, None);
        let background = counts(1, 2, 0, 0, None);
        let stopping = counts(0, 1, 0, 0, Some(row()));
        let idle_background = counts(3, 0, 1, 0, None);
        let stopped = counts(0, 0, 0, 2, None);
        let rows = [open.clone(), background, stopping, idle_background, stopped];
        assert_eq!(
            InBackground::of(open.workspace_id, &rows),
            InBackground {
                workspaces: 2,
                running: 3,
            },
            "background and stopping count; the open one and those without running runs do not"
        );
        assert_eq!(
            InBackground::of(open.workspace_id, &[open.clone()]),
            InBackground::default(),
            "the open workspace never counts"
        );
        assert_eq!(
            InBackground::of(WorkspaceId::new(), &[]),
            InBackground::default()
        );
    }

    #[test]
    fn workspace_stop_states_display_their_words() {
        assert_eq!(WorkspaceStopState::NotStopped.to_string(), "not stopped");
        assert_eq!(WorkspaceStopState::Stopping.to_string(), "stopping");
        assert_eq!(WorkspaceStopState::Stopped.to_string(), "stopped");
    }
}
