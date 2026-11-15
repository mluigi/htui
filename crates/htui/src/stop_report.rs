//! A workspace's stop report and its stop and resume writes (MOD-116 M2 D1-D4, `R-ORCH-16`,
//! `R-TUI-14`, `R-TUI-18`), shared by `htui stop`/`htui resume` ([`crate::stop_cmd`]) and the
//! Workspaces tab ([`crate::workspaces`]).
//!
//! D1: the report, its wording and the CLI's text moved here from `stop_cmd.rs` unchanged in
//! behaviour, so the CLI prints byte-identical text. D2: a `running` run's [`RunLine::steps`] say
//! what a stop does to each of its running steps ([`LiveStep::under`]). D3: [`write_stop`] and
//! [`write_resume`] re-read the stop row and write nothing when it changed since the question was
//! decided on ([`same_stop`]). Every function is generic over the store, so the unit tests run on
//! `MemStore::demo()`.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::DriverCaps;
use htui_agent::registry::caps_for;
use htui_core::model::{
    AgentId, AgentSummary, BoxId, Run, RunId, RunStatus, StepStatus, StopMode, UserId, WorkspaceId,
    WorkspaceStop, WorkspaceStopState,
};
use htui_core::store::{ReadStore, StoreError, WriteStore};
use htui_orch::promote::{StepResume, stopped_step_resume};

/// How long `htui stop` waits for its workspace's running runs to stop (D7): a graceful stop
/// settles within `CANCEL_GRACE` + `STOP_SLACK` + one command poll.
pub const STOP_WAIT: Duration = Duration::from_secs(20);

/// How often `htui stop` re-reads the workspace while it waits (D7).
pub const STOP_POLL: Duration = Duration::from_secs(1);

/// Each agent's caps, keyed by its id, as the engine reads them (`registry::caps_for`).
#[must_use]
pub fn caps_map(agents: Vec<AgentSummary>) -> HashMap<AgentId, DriverCaps> {
    agents
        .into_iter()
        .map(|summary| (summary.agent.id, caps_for(&summary.agent)))
        .collect()
}

/// MOD-116 M2 D2: each agent's name, keyed by its id: what a [`LiveStep`] names its agent by.
#[must_use]
pub fn agent_names(agents: &[AgentSummary]) -> HashMap<AgentId, String> {
    agents
        .iter()
        .map(|summary| (summary.agent.id, summary.agent.name.clone()))
        .collect()
}

/// D4: a workspace's stop state and each of its runs that is not finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The workspace's slug.
    pub slug: String,
    /// Derived from the stop row and the runs ([`WorkspaceStopState::of`]).
    pub state: WorkspaceStopState,
    /// The stop row, when there is one.
    pub stop: Option<WorkspaceStop>,
    /// One per run that is not finished, in `(queued_at, id)` order.
    pub lines: Vec<RunLine>,
}

/// One run of a [`Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunLine {
    /// The item's key; `-` for a run without an item.
    pub item: String,
    /// The run.
    pub run: RunId,
    /// Its status.
    pub status: RunStatus,
    /// The executing box's hostname (its id when no box row this user sees names it); `None` when
    /// the run executes nowhere.
    pub on_box: Option<String>,
    /// How it will resume, or why it waits.
    pub resume: RunResume,
    /// MOD-116 M2 D2: for a `running` run, its `running` steps in row order; empty for every
    /// other status. The CLI never prints it.
    pub steps: Vec<LiveStep>,
}

/// MOD-116 M2 D2: one `running` step of a `running` run, and whether a stop can pause it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStep {
    /// The step's phase name.
    pub phase: String,
    /// Its agent's name; `None` for a step without an agent, or one the names map lacks.
    pub agent: Option<String>,
    /// Its agent's transport can resume a session (`DriverCaps::resume`; the default caps for an
    /// unknown agent, as a stopped step's forecast reads them); `false` without an agent.
    pub pauses: bool,
}

impl LiveStep {
    /// D2: what a stop of `mode` does to this step, as `<who>: <fate>`, where `<who>` is
    /// `phase · agent` or `phase`.
    #[must_use]
    pub fn under(&self, mode: StopMode) -> String {
        let fate = match (mode, self.pauses) {
            (StopMode::Graceful, true) => "stops at its turn boundary; its session is kept",
            (StopMode::Graceful, false) => {
                "cannot pause between turns: stops at once; runs again on resume"
            }
            (StopMode::Forced, true) => "stops now, mid-turn; redoes that turn on resume",
            (StopMode::Forced, false) => "stops now; runs again on resume",
        };
        match &self.agent {
            Some(agent) => format!("{} · {agent}: {fate}", self.phase),
            None => format!("{}: {fate}", self.phase),
        }
    }
}

/// D4: what a run does when its workspace resumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResume {
    /// `running`: its executor is applying the stop (or, in a workspace that is not stopped, it
    /// runs).
    Stopping,
    /// `queued`: held until the workspace resumes.
    Held,
    /// `awaiting_approval`: its gate can be answered once the workspace resumes.
    AtGate,
    /// `stopped`, on a box: each stopped step, by phase name, and how it resumes.
    Steps(Vec<(String, StepResume)>),
    /// `stopped`, on a box, with no stopped step: it continues from where it stopped.
    Continues,
    /// `stopped` with no executing box (M3 D6): it can only be cancelled.
    NoBox,
}

impl RunResume {
    /// The line's last column; `stopped` is whether the workspace has a stop row.
    #[must_use]
    pub fn words(&self, stopped: bool) -> String {
        match self {
            Self::Stopping if stopped => "stopping".to_owned(),
            Self::Stopping => "keeps running".to_owned(),
            Self::Held if stopped => "starts when the workspace is resumed".to_owned(),
            Self::Held => "waits for its box to start it".to_owned(),
            Self::AtGate if stopped => "its gate can be answered once resumed".to_owned(),
            Self::AtGate => "waits at its gate".to_owned(),
            Self::Steps(steps) => {
                let steps: Vec<String> = steps
                    .iter()
                    .map(|(phase, how)| match how {
                        StepResume::Reopen(_) => format!("{phase} reopens its agent session"),
                        StepResume::FreshWithHandoff => {
                            format!("{phase} starts a fresh session with the handoff")
                        }
                        StepResume::Rerun => format!("{phase} runs again from its start"),
                    })
                    .collect();
                format!("resumes: {}", steps.join("; "))
            }
            Self::Continues => "resumes: continues from where it stopped".to_owned(),
            Self::NoBox => "cannot be resumed: cancel it from the Runs pane".to_owned(),
        }
    }

    /// A run its box will resume.
    pub(crate) const fn resumes(&self) -> bool {
        matches!(self, Self::Steps(_) | Self::Continues)
    }
}

impl RunLine {
    /// The status column.
    #[must_use]
    pub fn status_words(&self) -> String {
        match (self.status, self.on_box.as_deref()) {
            (RunStatus::Stopped, Some(host)) => format!("stopped on {host}"),
            (RunStatus::Stopped, None) => "stopped, no box".to_owned(),
            (RunStatus::Running, Some(host)) => format!("running on {host}"),
            (RunStatus::AwaitingApproval, _) => "awaiting approval".to_owned(),
            (status, _) => status.to_string(),
        }
    }

    /// D2/D5: what `mode` does to this run: one line per live step, the between-steps words,
    /// or `self.resume.words(true)` for a run that is not running.
    #[must_use]
    pub fn under(&self, mode: StopMode) -> Vec<String> {
        if self.status != RunStatus::Running {
            return vec![self.resume.words(true)];
        }
        if self.steps.is_empty() {
            let between = match mode {
                StopMode::Graceful => "stops before its next step",
                StopMode::Forced => "stops now",
            };
            return vec![between.to_owned()];
        }
        self.steps.iter().map(|step| step.under(mode)).collect()
    }
}

impl core::fmt::Display for Report {
    /// D4's text: the header, one aligned line per run (or `no runs in this workspace`), and a
    /// footer when a run will resume.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "workspace {}: {}", self.slug, self.state)?;
        match (&self.stop, self.state) {
            (Some(stop), WorkspaceStopState::Stopping | WorkspaceStopState::Stopped) => writeln!(
                f,
                " ({}, requested {})",
                stop.mode,
                stop.requested_at.format("%Y-%m-%d %H:%M UTC")
            )?,
            _ => writeln!(f)?,
        }
        if self.lines.is_empty() {
            return writeln!(f, "  no runs in this workspace");
        }
        let stopped = self.state != WorkspaceStopState::NotStopped;
        let statuses: Vec<String> = self.lines.iter().map(RunLine::status_words).collect();
        let item_width = self
            .lines
            .iter()
            .map(|line| line.item.chars().count())
            .max();
        let status_width = statuses.iter().map(|status| status.chars().count()).max();
        let (item_width, status_width) = (item_width.unwrap_or(0), status_width.unwrap_or(0));
        for (line, status) in self.lines.iter().zip(&statuses) {
            let run = line.run.to_string();
            writeln!(
                f,
                "  {:<item_width$}  {}  {status:<status_width$}  {}",
                line.item,
                &run[..8],
                line.resume.words(stopped)
            )?;
        }
        if self.lines.iter().any(|line| line.resume.resumes()) {
            writeln!(f, "checks rerun after each resumed step")?;
            if stopped {
                writeln!(
                    f,
                    "once resumed, each box resumes its stopped runs at its next command poll"
                )?;
            } else {
                writeln!(
                    f,
                    "each box resumes its stopped runs at its next command poll"
                )?;
            }
        }
        Ok(())
    }
}

/// D4's read: the stop row, the runs that are not finished (`workspace_live_runs`), their items'
/// keys, the boxes' hostnames, each stopped step's forecast ([`stopped_step_resume`] over its
/// row, its agent's caps and its log) and, MOD-116 M2 D2, each running run's running steps.
///
/// # Errors
/// Whatever the store reports.
pub async fn report<S: ReadStore + WriteStore>(
    store: &S,
    caps: &HashMap<AgentId, DriverCaps>,
    names: &HashMap<AgentId, String>,
    workspace: WorkspaceId,
    slug: &str,
) -> Result<Report, StoreError> {
    let stop = store.workspace_stop(workspace).await?;
    let runs: Vec<Run> = store
        .workspace_live_runs(workspace)
        .await?
        .into_iter()
        .filter(|run| !run.status.is_terminal())
        .collect();
    let state = WorkspaceStopState::of(stop.as_ref(), runs.iter().map(|run| run.status));
    let hosts: HashMap<BoxId, String> = if runs.iter().any(|run| run.executing_box_id.is_some()) {
        store
            .boxes()
            .await?
            .into_iter()
            .map(|record| (record.row.id, record.row.hostname))
            .collect()
    } else {
        HashMap::new()
    };
    let mut lines = Vec::with_capacity(runs.len());
    for run in runs {
        // One live graph run per item at most, so no key is read twice.
        let item = match run.item_id {
            Some(id) => store.item(id).await?.map(|item| item.key),
            None => None,
        }
        .unwrap_or_else(|| "-".to_owned());
        let resume = match run.status {
            RunStatus::Running => RunResume::Stopping,
            RunStatus::Queued => RunResume::Held,
            RunStatus::AwaitingApproval => RunResume::AtGate,
            _ if run.executing_box_id.is_none() => RunResume::NoBox,
            _ => stopped_resume(store, caps, run.id).await?,
        };
        let steps = if run.status == RunStatus::Running {
            live_steps(store, caps, names, run.id).await?
        } else {
            Vec::new()
        };
        lines.push(RunLine {
            item,
            run: run.id,
            status: run.status,
            on_box: run
                .executing_box_id
                .map(|id| hosts.get(&id).cloned().unwrap_or_else(|| id.to_string())),
            resume,
            steps,
        });
    }
    Ok(Report {
        slug: slug.to_owned(),
        state,
        stop,
        lines,
    })
}

/// A stopped run's forecast: each `stopped` step through [`stopped_step_resume`], or
/// [`RunResume::Continues`] when none is.
async fn stopped_resume<S: ReadStore>(
    store: &S,
    caps: &HashMap<AgentId, DriverCaps>,
    run: RunId,
) -> Result<RunResume, StoreError> {
    let mut steps = Vec::new();
    for step in store.run_steps(run).await? {
        if step.status != StepStatus::Stopped {
            continue;
        }
        // The engine's `stopped_opening` reads `DriverCaps::default()` for an unknown agent.
        let caps = step
            .agent_id
            .and_then(|agent| caps.get(&agent).copied())
            .unwrap_or_default();
        let events = store.step_events(step.id).await?.unwrap_or_default();
        let how = stopped_step_resume(&step, caps, &events);
        steps.push((step.phase_name, how));
    }
    Ok(if steps.is_empty() {
        RunResume::Continues
    } else {
        RunResume::Steps(steps)
    })
}

/// MOD-116 M2 D2: a running run's `running` steps in row order, each with its agent's name and
/// whether its caps can pause it (`DriverCaps::default()` for an unknown agent, as
/// [`stopped_resume`] reads them).
async fn live_steps<S: ReadStore>(
    store: &S,
    caps: &HashMap<AgentId, DriverCaps>,
    names: &HashMap<AgentId, String>,
    run: RunId,
) -> Result<Vec<LiveStep>, StoreError> {
    Ok(store
        .run_steps(run)
        .await?
        .into_iter()
        .filter(|step| step.status == StepStatus::Running)
        .map(|step| LiveStep {
            agent: step.agent_id.and_then(|agent| names.get(&agent).cloned()),
            pauses: step
                .agent_id
                .is_some_and(|agent| caps.get(&agent).copied().unwrap_or_default().resume),
            phase: step.phase_name,
        })
        .collect())
}

/// MOD-116 M2 D3: whether two stop rows are the same stop: the same `(mode, requested_at)`, or
/// both absent. `requested_by` is not compared.
#[must_use]
pub fn same_stop(a: Option<&WorkspaceStop>, b: Option<&WorkspaceStop>) -> bool {
    let key = |row: &WorkspaceStop| (row.mode, row.requested_at);
    a.map(key) == b.map(key)
}

/// MOD-116 M2 D4: one workspace a stop request writes, with the stop row its question was
/// decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTarget {
    /// The workspace to stop.
    pub workspace: WorkspaceId,
    /// The stop row read when the question was built; `None` when it had none.
    pub decided_on: Option<WorkspaceStop>,
}

/// MOD-116 M2 D3: what [`write_stop`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopWrite {
    /// The stop was written (inserted, or a graceful stop escalated to forced).
    Written,
    /// The row already held this mode or more (graceful over graceful, anything over forced):
    /// nothing was written. Carries the row's mode.
    Already(StopMode),
    /// The row changed since the question was decided on: nothing was written.
    Changed,
}

/// MOD-116 M2 D3: what [`write_resume`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeWrite {
    /// The stop row was deleted (or someone resumed it in between, as the CLI says).
    Resumed,
    /// There was no stop row, as decided on: nothing was written.
    NotStopped,
    /// The row changed since the question was decided on: nothing was written.
    Changed,
}

/// MOD-116 M2 D3: re-reads the stop row; one that differs from `decided_on` writes nothing
/// ([`StopWrite::Changed`]); one that already holds `mode` or more writes nothing
/// ([`StopWrite::Already`]); otherwise `stop_workspace(workspace, mode, user, at)`.
///
/// # Errors
/// Whatever the store reports.
pub async fn write_stop<S: WriteStore>(
    store: &S,
    workspace: WorkspaceId,
    mode: StopMode,
    decided_on: Option<&WorkspaceStop>,
    user: UserId,
    at: DateTime<Utc>,
) -> Result<StopWrite, StoreError> {
    let stored = store.workspace_stop(workspace).await?;
    if !same_stop(stored.as_ref(), decided_on) {
        return Ok(StopWrite::Changed);
    }
    if let Some(row) = stored
        && (row.mode == StopMode::Forced || mode == StopMode::Graceful)
    {
        return Ok(StopWrite::Already(row.mode));
    }
    store.stop_workspace(workspace, mode, user, at).await?;
    Ok(StopWrite::Written)
}

/// MOD-116 M2 D3: re-reads the stop row; one that differs from `decided_on` writes nothing
/// ([`ResumeWrite::Changed`]); no row is [`ResumeWrite::NotStopped`]; otherwise
/// `resume_workspace` (its `Ok(false)`, resumed in between, is still [`ResumeWrite::Resumed`]).
///
/// # Errors
/// Whatever the store reports.
pub async fn write_resume<S: WriteStore>(
    store: &S,
    workspace: WorkspaceId,
    decided_on: Option<&WorkspaceStop>,
) -> Result<ResumeWrite, StoreError> {
    let stored = store.workspace_stop(workspace).await?;
    if !same_stop(stored.as_ref(), decided_on) {
        return Ok(ResumeWrite::Changed);
    }
    if stored.is_none() {
        return Ok(ResumeWrite::NotStopped);
    }
    // `Ok(false)`: resumed by someone else in between; the workspace is resumed all the same.
    store.resume_workspace(workspace).await?;
    Ok(ResumeWrite::Resumed)
}

#[cfg(test)]
mod tests {
    use super::{
        LiveStep, ResumeWrite, RunLine, RunResume, StopWrite, agent_names, caps_map, report,
        same_stop, write_resume, write_stop,
    };
    use chrono::{DateTime, TimeDelta, Utc};
    use htui_agent::registry::caps_for;
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{
        AgentId, Claim, ItemId, NewItem, NewRun, NewRunStep, RunId, RunMode, RunStatus, StepId,
        StepRole, StepStatus, StopMode, UserId, WorkspaceId, WorkspaceStop,
    };
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};
    use uuid::Uuid;

    const LEASE: TimeDelta = TimeDelta::seconds(600);

    /// The clock the fixture's writes are stamped with: 2026-10-01 00:00 UTC.
    fn now() -> DateTime<Utc> {
        demo_at(30, 0)
    }

    /// A queued graph run of a fresh `htui` item, started from `workspace` at `hour` (as
    /// `stop_cmd`'s fixture).
    async fn queued(store: &MemStore, workspace: WorkspaceId, hour: i64) -> RunId {
        let item = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: ids::KIND_HTUI_FEAT,
                title: format!("run at {hour}"),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                target_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("mint")
            .id;
        let snapshot = store
            .run(ids::RUN_8)
            .await
            .expect("read")
            .and_then(|run| run.graph_snapshot)
            .expect("the demo's runs carry a snapshot");
        let snapshot = serde_json::from_value(snapshot).expect("a snapshot decodes");
        store
            .create_run(NewRun {
                id: RunId::new(),
                project_id: ids::PROJECT_HTUI,
                item_id: item,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: snapshot,
                repo_scope: Vec::new(),
                queued_at: demo_at(20, hour),
                batch_id: None,
                workspace_id: Some(workspace),
            })
            .await
            .expect("create")
            .id
    }

    /// [`queued`], claimed on the fixture box by `owner`.
    async fn claimed(store: &MemStore, workspace: WorkspaceId, hour: i64, owner: Uuid) -> RunId {
        let run = queued(store, workspace, hour).await;
        assert_eq!(
            store
                .claim_run(run, ids::BOX, owner, now(), LEASE)
                .await
                .expect("claim"),
            Claim::Admitted
        );
        run
    }

    /// A running step of `run` at `fanout_index`, on `agent`.
    async fn running_step(
        store: &MemStore,
        run: RunId,
        fanout_index: i32,
        phase: &str,
        agent: Option<AgentId>,
    ) {
        let step = store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index,
                phase_name: phase.to_owned(),
                agent_id: agent,
                model: None,
                role: StepRole::Step,
                parent_step_id: None,
                node_key: None,
            })
            .await
            .expect("create step")
            .id;
        assert!(
            store
                .transition_step(step, StepStatus::Pending, StepStatus::Running, now())
                .await
                .expect("start")
        );
    }

    fn stop_row(mode: StopMode, at: DateTime<Utc>) -> WorkspaceStop {
        WorkspaceStop {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            mode,
            requested_by: ids::USER,
            requested_at: at,
        }
    }

    /// D3: the key is `(mode, requested_at)`; `requested_by` is not compared.
    #[test]
    fn same_stop_compares_mode_and_instant() {
        let graceful = stop_row(StopMode::Graceful, now());
        assert!(same_stop(None, None));
        assert!(!same_stop(Some(&graceful), None));
        assert!(!same_stop(None, Some(&graceful)));
        assert!(same_stop(Some(&graceful), Some(&graceful.clone())));
        let forced = stop_row(StopMode::Forced, now());
        assert!(!same_stop(Some(&graceful), Some(&forced)));
        let later = stop_row(StopMode::Graceful, now() + TimeDelta::seconds(1));
        assert!(!same_stop(Some(&graceful), Some(&later)));
        let by_other = WorkspaceStop {
            requested_by: UserId::new(),
            ..graceful.clone()
        };
        assert!(same_stop(Some(&graceful), Some(&by_other)));
    }

    /// D2's four sentences, with and without an agent name, and the between-steps words.
    #[test]
    fn the_live_step_words_follow_the_mode_and_the_caps() {
        let named = |pauses| LiveStep {
            phase: "implement".to_owned(),
            agent: Some("claude".to_owned()),
            pauses,
        };
        assert_eq!(
            named(true).under(StopMode::Graceful),
            "implement · claude: stops at its turn boundary; its session is kept"
        );
        assert_eq!(
            named(false).under(StopMode::Graceful),
            "implement · claude: cannot pause between turns: stops at once; runs again on resume"
        );
        assert_eq!(
            named(true).under(StopMode::Forced),
            "implement · claude: stops now, mid-turn; redoes that turn on resume"
        );
        assert_eq!(
            named(false).under(StopMode::Forced),
            "implement · claude: stops now; runs again on resume"
        );
        let bare = LiveStep {
            phase: "checks".to_owned(),
            agent: None,
            pauses: false,
        };
        assert_eq!(
            bare.under(StopMode::Graceful),
            "checks: cannot pause between turns: stops at once; runs again on resume"
        );
        assert_eq!(
            bare.under(StopMode::Forced),
            "checks: stops now; runs again on resume"
        );

        let line = |status, steps| RunLine {
            item: "HTUI-1".to_owned(),
            run: RunId::new(),
            status,
            on_box: None,
            resume: match status {
                RunStatus::Running => RunResume::Stopping,
                _ => RunResume::Held,
            },
            steps,
        };
        let between = line(RunStatus::Running, Vec::new());
        assert_eq!(
            between.under(StopMode::Graceful),
            ["stops before its next step"]
        );
        assert_eq!(between.under(StopMode::Forced), ["stops now"]);
        let two = line(RunStatus::Running, vec![named(true), bare.clone()]);
        assert_eq!(
            two.under(StopMode::Forced),
            [
                "implement · claude: stops now, mid-turn; redoes that turn on resume",
                "checks: stops now; runs again on resume",
            ]
        );
        let held = line(RunStatus::Queued, Vec::new());
        assert_eq!(
            held.under(StopMode::Graceful),
            ["starts when the workspace is resumed"]
        );
        assert_eq!(
            held.under(StopMode::Forced),
            ["starts when the workspace is resumed"]
        );
    }

    /// D2: a running run lists its running steps, its agent named from `names` and paused per
    /// its caps; a queued and a stopped run list none.
    #[tokio::test(start_paused = true)]
    async fn a_running_run_lists_its_running_steps() {
        let store = MemStore::demo();
        let ws = ids::WORKSPACE_GRAPHICS;
        let agents = store.agents().await.expect("agents");
        let claude = agents
            .iter()
            .find(|summary| summary.agent.id == ids::AGENT_CLAUDE_CLI)
            .expect("the demo has the claude CLI agent")
            .agent
            .clone();
        let names = agent_names(&agents);
        assert_eq!(names.get(&claude.id), Some(&claude.name));
        let caps = caps_map(agents);

        let owner = Uuid::now_v7();
        let running = claimed(&store, ws, 1, owner).await;
        running_step(&store, running, 0, "implement", Some(claude.id)).await;
        running_step(&store, running, 1, "checks", None).await;
        let held = queued(&store, ws, 2).await;

        let read = report(&store, &caps, &names, ws, "graphics")
            .await
            .expect("report");
        let line = |run| {
            read.lines
                .iter()
                .find(|line| line.run == run)
                .unwrap_or_else(|| panic!("a line for {run}: {:?}", read.lines))
        };
        assert_eq!(
            line(running).steps,
            [
                LiveStep {
                    phase: "implement".to_owned(),
                    agent: Some(claude.name.clone()),
                    pauses: caps_for(&claude).resume,
                },
                LiveStep {
                    phase: "checks".to_owned(),
                    agent: None,
                    pauses: false,
                },
            ]
        );
        assert_eq!(line(held).status, RunStatus::Queued);
        assert_eq!(line(held).steps, []);
        assert_eq!(line(ids::RUN_8).status, RunStatus::Stopped);
        assert_eq!(line(ids::RUN_8).steps, []);

        // Without the names map the agent is unnamed, and its caps still decide `pauses`.
        let unnamed = report(&store, &caps, &Default::default(), ws, "graphics")
            .await
            .expect("report");
        let steps = &unnamed
            .lines
            .iter()
            .find(|line| line.run == running)
            .expect("the running line")
            .steps;
        assert_eq!(steps[0].agent, None);
        assert_eq!(steps[0].pauses, caps_for(&claude).resume);
    }

    /// D3: written once, never downgraded, and nothing written over a stop that changed.
    #[tokio::test]
    async fn write_stop_writes_once_never_downgrades_and_refuses_a_changed_stop() {
        let store = MemStore::demo();
        let ws = ids::WORKSPACE_GRAPHICS;
        let user = ids::USER;
        let first = now();
        assert_eq!(
            write_stop(&store, ws, StopMode::Graceful, None, user, first)
                .await
                .expect("write"),
            StopWrite::Written
        );
        let graceful = store
            .workspace_stop(ws)
            .await
            .expect("read")
            .expect("a row");
        assert_eq!(
            (graceful.mode, graceful.requested_by, graceful.requested_at),
            (StopMode::Graceful, user, first)
        );

        let second = first + TimeDelta::seconds(5);
        assert_eq!(
            write_stop(
                &store,
                ws,
                StopMode::Graceful,
                Some(&graceful),
                user,
                second
            )
            .await
            .expect("write"),
            StopWrite::Already(StopMode::Graceful)
        );
        assert_eq!(
            store.workspace_stop(ws).await.expect("read").as_ref(),
            Some(&graceful)
        );

        assert_eq!(
            write_stop(&store, ws, StopMode::Forced, Some(&graceful), user, second)
                .await
                .expect("write"),
            StopWrite::Written
        );
        let forced = store
            .workspace_stop(ws)
            .await
            .expect("read")
            .expect("a row");
        assert_eq!(
            (forced.mode, forced.requested_at),
            (StopMode::Forced, second)
        );

        let third = second + TimeDelta::seconds(5);
        assert_eq!(
            write_stop(&store, ws, StopMode::Graceful, Some(&forced), user, third)
                .await
                .expect("write"),
            StopWrite::Already(StopMode::Forced)
        );
        assert_eq!(
            write_stop(&store, ws, StopMode::Forced, Some(&forced), user, third)
                .await
                .expect("write"),
            StopWrite::Already(StopMode::Forced)
        );

        // Decided on the graceful row (or on none) the forced one replaced: nothing is written.
        for decided_on in [Some(&graceful), None] {
            assert_eq!(
                write_stop(&store, ws, StopMode::Forced, decided_on, user, third)
                    .await
                    .expect("write"),
                StopWrite::Changed
            );
            assert_eq!(
                store.workspace_stop(ws).await.expect("read").as_ref(),
                Some(&forced)
            );
        }
    }

    /// D3: a resume deletes the row it was decided on, says when there is none, and leaves a row
    /// that replaced it meanwhile.
    #[tokio::test]
    async fn write_resume_deletes_only_the_stop_it_was_decided_on() {
        let store = MemStore::demo();
        let ws = ids::WORKSPACE_GRAPHICS;
        assert_eq!(
            write_resume(&store, ws, None).await.expect("resume"),
            ResumeWrite::NotStopped
        );

        let first = store
            .stop_workspace(ws, StopMode::Graceful, ids::USER, now())
            .await
            .expect("stop");
        assert_eq!(
            write_resume(&store, ws, Some(&first))
                .await
                .expect("resume"),
            ResumeWrite::Resumed
        );
        assert_eq!(store.workspace_stop(ws).await.expect("read"), None);

        let second = store
            .stop_workspace(
                ws,
                StopMode::Graceful,
                ids::USER,
                now() + TimeDelta::seconds(5),
            )
            .await
            .expect("stop");
        for decided_on in [Some(&first), None] {
            assert_eq!(
                write_resume(&store, ws, decided_on).await.expect("resume"),
                ResumeWrite::Changed
            );
            assert_eq!(
                store.workspace_stop(ws).await.expect("read").as_ref(),
                Some(&second)
            );
        }
    }
}
