//! `htui stop` and `htui resume` (MOD-125 M4, `R-ORCH-16`, `R-TUI-14`, `R-TUI-18`): a workspace's
//! stop written or deleted from a shell, after a confirmation, and a report of each run that is
//! not finished and how it will resume.
//!
//! Both verbs connect headless as `qdrant_cmd` does (keyring DSN, `box.toml`, never a migration)
//! and run a core generic over the store, so the unit tests run it on `MemStore::demo()` and
//! `tests/it/stop_cli_pg.rs` on Postgres. The confirmation (D2) is a preview on stdout, the
//! question on stderr and one line of stdin; `--yes` answers it, and without a terminal it is
//! required. The stop row is read again once the question is answered, and a request whose stop
//! changed meanwhile writes nothing (refused). A request that writes nothing (a repeat stop, a
//! resume of a workspace that is not stopped) asks nothing. Every ending is a [`StateExit`]: 0 done, 2 refused, 3 Postgres did not
//! answer, 1 any other store failure.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal as _, Write};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::DriverCaps;
use htui_agent::registry::caps_for;
use htui_core::model::{
    AgentId, AgentSummary, BoxId, Run, RunId, RunStatus, StepStatus, StopMode, UserId,
    WorkspaceStop, WorkspaceStopState, WorkspaceSummary,
};
use htui_core::store::{ReadStore, StoreError, WriteStore};
use htui_orch::promote::{StepResume, stopped_step_resume};
use htui_store::PgStore;

use crate::cli;
use crate::qdrant_cmd;
use crate::state_cmd::StateExit;

/// How long `htui stop` waits for its workspace's running runs to stop (D7): a graceful stop
/// settles within `CANCEL_GRACE` + `STOP_SLACK` + one command poll.
pub const STOP_WAIT: Duration = Duration::from_secs(20);

/// How often `htui stop` re-reads the workspace while it waits (D7).
pub const STOP_POLL: Duration = Duration::from_secs(1);

/// The confirmation seam (D2): what `run_*` read from the process.
pub struct Ask<'a> {
    /// `--yes`: the question is answered without being asked.
    pub yes: bool,
    /// Whether stdin is a terminal; without one, and without `--yes`, a write is refused.
    pub interactive: bool,
    /// Where the answer is read from: one line.
    pub input: &'a mut dyn BufRead,
    /// Where the question goes (stderr), and the one line `htui stop` says while it waits.
    pub question: &'a mut dyn Write,
}

impl core::fmt::Debug for Ask<'_> {
    /// The two flags; the streams are not printable.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ask")
            .field("yes", &self.yes)
            .field("interactive", &self.interactive)
            .finish_non_exhaustive()
    }
}

/// What the cores need that `run_*` read from the store and the process: the test seam, as
/// `state_cmd::Context`.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// The user a stop is recorded as requested by (`PgStore::this_user`).
    pub user: UserId,
    /// The clock the stop's `requested_at` is read from, once the question is answered
    /// (`Utc::now`): a prompt left open does not date the stop to when it was asked.
    pub now: fn() -> DateTime<Utc>,
    /// Each agent's caps (`registry::caps_for` over the store's agents, [`caps_map`]): what a
    /// stopped step's resume is forecast from.
    pub caps: &'a HashMap<AgentId, DriverCaps>,
    /// The workspaces a slug is resolved among (the store's inherent `workspaces()`).
    pub workspaces: &'a [WorkspaceSummary],
    /// How long `htui stop` waits for the stop to settle ([`STOP_WAIT`]).
    pub wait: Duration,
    /// How often it re-reads meanwhile ([`STOP_POLL`]).
    pub poll: Duration,
}

/// Each agent's caps, keyed by its id, as the engine reads them (`registry::caps_for`).
#[must_use]
pub fn caps_map(agents: Vec<AgentSummary>) -> HashMap<AgentId, DriverCaps> {
    agents
        .into_iter()
        .map(|summary| (summary.agent.id, caps_for(&summary.agent)))
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
    fn words(&self, stopped: bool) -> String {
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
    const fn resumes(&self) -> bool {
        matches!(self, Self::Steps(_) | Self::Continues)
    }
}

impl RunLine {
    /// The status column.
    fn status_words(&self) -> String {
        match (self.status, self.on_box.as_deref()) {
            (RunStatus::Stopped, Some(host)) => format!("stopped on {host}"),
            (RunStatus::Stopped, None) => "stopped, no box".to_owned(),
            (RunStatus::Running, Some(host)) => format!("running on {host}"),
            (RunStatus::AwaitingApproval, _) => "awaiting approval".to_owned(),
            (status, _) => status.to_string(),
        }
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

/// D2: the graceful stop's question.
fn graceful_question(slug: &str) -> String {
    format!(
        "Stop workspace `{slug}` gracefully? Each agent step stops at its turn boundary, checks \
         stop at once; nothing new starts until `htui resume {slug}`. [y/N] "
    )
}

/// D2: the forced stop's question.
fn forced_question(slug: &str) -> String {
    format!(
        "Stop workspace `{slug}` now? Every step is killed mid-turn and redoes its turn on resume; \
         nothing new starts until `htui resume {slug}`. [y/N] "
    )
}

/// D2: `--force` over a graceful stop.
fn escalation_question(slug: &str) -> String {
    format!(
        "Workspace `{slug}` is stopped gracefully; force it? Every step still running is killed \
         mid-turn. [y/N] "
    )
}

/// D2: the resume's question.
fn resume_question(slug: &str) -> String {
    format!(
        "Resume workspace `{slug}`? Its stopped runs resume on the boxes that ran them and its \
         queued runs can start. [y/N] "
    )
}

/// `htui stop`: the headless connect (`qdrant_cmd::connect`), the store's agents and workspaces,
/// then [`stop_on`] with stdin, stdout and stderr; the store is closed whatever it answered.
///
/// # Errors
/// [`StateExit`]: 2 refused (no DSN, keyring, `box.toml`, schema, unknown slug, not confirmed,
/// no terminal without `--yes`, a stop changed while asking, a workspace deleted meanwhile), 3
/// Postgres did not answer, 1 any other store failure.
pub async fn run_stop(args: cli::StopArgs) -> Result<(), StateExit> {
    let mode = if args.force {
        StopMode::Forced
    } else {
        StopMode::Graceful
    };
    let pg = qdrant_cmd::connect().await?;
    let ended = async {
        let (caps, workspaces) = inputs(&pg, &args.workspace).await?;
        let ctx = process_context(&pg, &caps, &workspaces);
        let stdin = std::io::stdin();
        let interactive = stdin.is_terminal();
        let mut input = std::io::BufReader::new(stdin);
        let mut err = std::io::stderr();
        let mut ask = Ask {
            yes: args.yes,
            interactive,
            input: &mut input,
            question: &mut err,
        };
        stop_on(
            &pg,
            &ctx,
            &args.workspace,
            mode,
            &mut ask,
            &mut std::io::stdout(),
        )
        .await
    }
    .await;
    pg.close().await;
    ended
}

/// `htui resume`: as [`run_stop`], then [`resume_on`].
///
/// # Errors
/// As [`run_stop`].
pub async fn run_resume(args: cli::ResumeArgs) -> Result<(), StateExit> {
    let pg = qdrant_cmd::connect().await?;
    let ended = async {
        let (caps, workspaces) = inputs(&pg, &args.workspace).await?;
        let ctx = process_context(&pg, &caps, &workspaces);
        let stdin = std::io::stdin();
        let interactive = stdin.is_terminal();
        let mut input = std::io::BufReader::new(stdin);
        let mut err = std::io::stderr();
        let mut ask = Ask {
            yes: args.yes,
            interactive,
            input: &mut input,
            question: &mut err,
        };
        resume_on(&pg, &ctx, &args.workspace, &mut ask, &mut std::io::stdout()).await
    }
    .await;
    pg.close().await;
    ended
}

/// The agents' caps and the workspace list, read once from Postgres.
async fn inputs(
    pg: &PgStore,
    slug: &str,
) -> Result<(HashMap<AgentId, DriverCaps>, Vec<WorkspaceSummary>), StateExit> {
    let read = |err: StoreError| store_exit(&err, slug, "read");
    let caps = caps_map(pg.agents().await.map_err(read)?);
    let workspaces = pg.workspaces().await.map_err(read)?;
    Ok((caps, workspaces))
}

/// The process's [`Context`]: this user, the wall clock, the real wait.
fn process_context<'a>(
    pg: &PgStore,
    caps: &'a HashMap<AgentId, DriverCaps>,
    workspaces: &'a [WorkspaceSummary],
) -> Context<'a> {
    Context {
        user: pg.this_user(),
        now: Utc::now,
        caps,
        workspaces,
        wait: STOP_WAIT,
        poll: STOP_POLL,
    }
}

/// `htui stop`'s core (D5): reads the stop row; a request that would write nothing (graceful over
/// graceful, anything over forced) prints the report after `workspace <slug> is already stopped
/// (<mode>)` and asks nothing; otherwise it asks (D2), re-reads the row and refuses when it changed
/// meanwhile, writes `stop_workspace(ws, mode, ctx.user, (ctx.now)())`, waits up to `ctx.wait` for
/// the state to read `stopped` (D7), and prints the report.
///
/// # Errors
/// [`StateExit`], as [`run_stop`].
pub async fn stop_on<S: ReadStore + WriteStore>(
    store: &S,
    ctx: &Context<'_>,
    slug: &str,
    mode: StopMode,
    ask: &mut Ask<'_>,
    out: &mut dyn Write,
) -> Result<(), StateExit> {
    let workspace = resolve(ctx.workspaces, slug)?;
    let stored = store
        .workspace_stop(workspace.workspace_id)
        .await
        .map_err(|err| store_exit(&err, slug, "read"))?;
    let question = match (&stored, mode) {
        (None, StopMode::Graceful) => graceful_question(slug),
        (None, StopMode::Forced) => forced_question(slug),
        (Some(row), StopMode::Forced) if row.mode == StopMode::Graceful => {
            escalation_question(slug)
        }
        (Some(row), _) => {
            let mut text = format!("workspace {slug} is already stopped ({})\n", row.mode);
            if mode == StopMode::Graceful && row.mode == StopMode::Forced {
                text.push_str("a graceful stop does not downgrade a forced one\n");
            }
            text.push_str(&report(store, ctx, workspace).await?.to_string());
            return emit(out, &text);
        }
    };
    confirm(store, ctx, workspace, ask, out, &question, "stopped").await?;
    let again = if mode == StopMode::Forced {
        format!("htui stop {slug} --force")
    } else {
        format!("htui stop {slug}")
    };
    unchanged(store, workspace, stored.as_ref(), "stopped", &again).await?;
    store
        .stop_workspace(workspace.workspace_id, mode, ctx.user, (ctx.now)())
        .await
        .map_err(|err| store_exit(&err, slug, "stopped"))?;
    settle(store, ctx, workspace, ask).await?;
    let report = report(store, ctx, workspace).await?;
    let text = if report.state == WorkspaceStopState::Stopping {
        format!("{report}still stopping; run htui stop {slug} again to see it settle\n")
    } else {
        report.to_string()
    };
    emit(out, &text)
}

/// `htui resume`'s core (D6): with a stop row it asks (D2), re-reads it and refuses when it changed
/// meanwhile, deletes it (`Ok(false)`, someone resumed it in between, is no error) and prints the
/// report under `resume requested: …`; without one it asks nothing and prints the report under
/// `workspace <slug> is not stopped`. No wait.
///
/// # Errors
/// [`StateExit`], as [`run_resume`].
pub async fn resume_on<S: ReadStore + WriteStore>(
    store: &S,
    ctx: &Context<'_>,
    slug: &str,
    ask: &mut Ask<'_>,
    out: &mut dyn Write,
) -> Result<(), StateExit> {
    let workspace = resolve(ctx.workspaces, slug)?;
    let stored = store
        .workspace_stop(workspace.workspace_id)
        .await
        .map_err(|err| store_exit(&err, slug, "read"))?;
    let lead = if stored.is_none() {
        format!("workspace {slug} is not stopped\n")
    } else {
        confirm(
            store,
            ctx,
            workspace,
            ask,
            out,
            &resume_question(slug),
            "resumed",
        )
        .await?;
        let again = format!("htui resume {slug}");
        unchanged(store, workspace, stored.as_ref(), "resumed", &again).await?;
        store
            .resume_workspace(workspace.workspace_id)
            .await
            .map_err(|err| store_exit(&err, slug, "resumed"))?;
        "resume requested: each box resumes its stopped runs at its next command poll\n".to_owned()
    };
    let report = report(store, ctx, workspace).await?;
    emit(out, &format!("{lead}{report}"))
}

/// D4's read, before and after the write: the stop row, the runs that are not finished
/// (`workspace_live_runs`), their items' keys, the boxes' hostnames, and each stopped step's
/// forecast ([`stopped_step_resume`] over its row, its agent's caps and its log).
///
/// # Errors
/// [`StateExit`]: 3 Postgres did not answer, 1 any other failure.
pub async fn report<S: ReadStore + WriteStore>(
    store: &S,
    ctx: &Context<'_>,
    workspace: &WorkspaceSummary,
) -> Result<Report, StateExit> {
    let read = |err: StoreError| store_exit(&err, &workspace.slug, "read");
    let stop = store
        .workspace_stop(workspace.workspace_id)
        .await
        .map_err(read)?;
    let runs: Vec<Run> = store
        .workspace_live_runs(workspace.workspace_id)
        .await
        .map_err(read)?
        .into_iter()
        .filter(|run| !run.status.is_terminal())
        .collect();
    let state = WorkspaceStopState::of(stop.as_ref(), runs.iter().map(|run| run.status));
    let hosts: HashMap<BoxId, String> = if runs.iter().any(|run| run.executing_box_id.is_some()) {
        store
            .boxes()
            .await
            .map_err(read)?
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
            Some(id) => store.item(id).await.map_err(read)?.map(|item| item.key),
            None => None,
        }
        .unwrap_or_else(|| "-".to_owned());
        let resume = match run.status {
            RunStatus::Running => RunResume::Stopping,
            RunStatus::Queued => RunResume::Held,
            RunStatus::AwaitingApproval => RunResume::AtGate,
            _ if run.executing_box_id.is_none() => RunResume::NoBox,
            _ => stopped_resume(store, ctx, run.id).await.map_err(read)?,
        };
        lines.push(RunLine {
            item,
            run: run.id,
            status: run.status,
            on_box: run
                .executing_box_id
                .map(|id| hosts.get(&id).cloned().unwrap_or_else(|| id.to_string())),
            resume,
        });
    }
    Ok(Report {
        slug: workspace.slug.clone(),
        state,
        stop,
        lines,
    })
}

/// A stopped run's forecast: each `stopped` step through [`stopped_step_resume`], or
/// [`RunResume::Continues`] when none is.
async fn stopped_resume<S: ReadStore>(
    store: &S,
    ctx: &Context<'_>,
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
            .and_then(|agent| ctx.caps.get(&agent).copied())
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

/// D7: re-reads every `ctx.poll` until the workspace no longer reads `stopping` or `ctx.wait`
/// passed, saying once on stderr how many runs it waits for. Only the stop row and the runs are
/// read meanwhile; the report is read once after.
async fn settle<S: ReadStore + WriteStore>(
    store: &S,
    ctx: &Context<'_>,
    workspace: &WorkspaceSummary,
    ask: &mut Ask<'_>,
) -> Result<(), StateExit> {
    let read = |err: StoreError| store_exit(&err, &workspace.slug, "read");
    let deadline = tokio::time::Instant::now() + ctx.wait;
    let mut said = false;
    loop {
        let stop = store
            .workspace_stop(workspace.workspace_id)
            .await
            .map_err(read)?;
        let runs = store
            .workspace_live_runs(workspace.workspace_id)
            .await
            .map_err(read)?;
        let running = runs
            .iter()
            .filter(|run| run.status == RunStatus::Running)
            .count();
        let state = WorkspaceStopState::of(stop.as_ref(), runs.iter().map(|run| run.status));
        let now = tokio::time::Instant::now();
        if state != WorkspaceStopState::Stopping || now >= deadline {
            return Ok(());
        }
        if !said {
            let runs = if running == 1 { "run" } else { "runs" };
            say(
                ask.question,
                &format!("waiting for {running} running {runs} to stop…\n"),
            )?;
            said = true;
        }
        tokio::time::sleep(ctx.poll.min(deadline - now)).await;
    }
}

/// D2: `--yes` confirms without asking; no terminal refuses before anything is printed or
/// written; otherwise the preview goes to `out`, the question to `ask.question`, and one line of
/// `ask.input` answers it: `y` or `yes` (any case, trimmed) confirms, anything else declines.
async fn confirm<S: ReadStore + WriteStore>(
    store: &S,
    ctx: &Context<'_>,
    workspace: &WorkspaceSummary,
    ask: &mut Ask<'_>,
    out: &mut dyn Write,
    question: &str,
    nothing_was: &str,
) -> Result<(), StateExit> {
    if ask.yes {
        return Ok(());
    }
    if !ask.interactive {
        return Err(StateExit::Refused(format!(
            "needs confirmation: pass --yes, or run it in a terminal; nothing was {nothing_was}"
        )));
    }
    let preview = report(store, ctx, workspace).await?;
    emit(out, &preview.to_string())?;
    say(ask.question, question)?;
    let mut line = String::new();
    ask.input
        .read_line(&mut line)
        .map_err(|err| StateExit::Failed(format!("cannot read the answer from stdin: {err}")))?;
    let answer = line.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        Ok(())
    } else {
        Err(StateExit::Refused(format!(
            "not confirmed; nothing was {nothing_was}"
        )))
    }
}

/// The stop row read again after [`confirm`]: gone, or with another mode or request instant than
/// the one the request was decided on, means someone stopped or resumed the workspace while the
/// question was open, so nothing is written and the request is refused (2) with the command to run
/// `again`.
async fn unchanged<S: ReadStore + WriteStore>(
    store: &S,
    workspace: &WorkspaceSummary,
    decided_on: Option<&WorkspaceStop>,
    nothing_was: &str,
    again: &str,
) -> Result<(), StateExit> {
    let stored = store
        .workspace_stop(workspace.workspace_id)
        .await
        .map_err(|err| store_exit(&err, &workspace.slug, "read"))?;
    let key = |row: &WorkspaceStop| (row.mode, row.requested_at);
    if stored.as_ref().map(key) == decided_on.map(key) {
        Ok(())
    } else {
        Err(StateExit::Refused(format!(
            "the stop of workspace {} changed while asking; nothing was {nothing_was}; run {again} \
             again",
            workspace.slug
        )))
    }
}

/// The workspace whose slug is `slug`, exactly; refused naming the slugs there are.
fn resolve<'w>(
    workspaces: &'w [WorkspaceSummary],
    slug: &str,
) -> Result<&'w WorkspaceSummary, StateExit> {
    workspaces.iter().find(|ws| ws.slug == slug).ok_or_else(|| {
        let known: Vec<String> = workspaces
            .iter()
            .map(|ws| format!("`{}`", ws.slug))
            .collect();
        StateExit::Refused(if known.is_empty() {
            format!("no workspace with slug `{slug}`: there are no workspaces")
        } else {
            format!(
                "no workspace with slug `{slug}`; the workspaces are {}",
                known.join(", ")
            )
        })
    })
}

/// D3: `Unreachable` is offline (3), the workspace gone is a refusal (2), anything else a
/// failure (1) that says what could not be `done` (`stopped`, `resumed`, `read`).
pub(crate) fn store_exit(err: &StoreError, slug: &str, done: &str) -> StateExit {
    match err {
        StoreError::Unreachable(_) => StateExit::Offline(format!("cannot reach Postgres: {err}")),
        StoreError::NotFound {
            entity: "workspace",
            ..
        } => StateExit::Refused(format!("workspace {slug} no longer exists")),
        _ => StateExit::Failed(format!("the workspace could not be {done}: {err}")),
    }
}

/// Writes `text` to stdout; a closed pipe (`htui stop x | head`) is not a failure.
fn emit(out: &mut dyn Write, text: &str) -> Result<(), StateExit> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => {
            Err(StateExit::Failed(format!("cannot write to stdout: {err}")))
        }
        _ => Ok(()),
    }
}

/// Writes `text` to stderr and flushes it, so a question shows before the answer is read.
fn say(question: &mut dyn Write, text: &str) -> Result<(), StateExit> {
    question
        .write_all(text.as_bytes())
        .and_then(|()| question.flush())
        .map_err(|err| StateExit::Failed(format!("cannot write to stderr: {err}")))
}

#[cfg(test)]
mod tests {
    use super::{
        Ask, Context, RunResume, STOP_POLL, STOP_WAIT, StateExit, caps_map, report, resume_on,
        stop_on, store_exit,
    };
    use chrono::{DateTime, TimeDelta, Utc};
    use htui_agent::driver::DriverCaps;
    use htui_agent::event::SESSION_STARTED;
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::{
        AgentId, Claim, EventKind, EventRole, ItemId, NewItem, NewRun, NewRunStep, NewWorkspace,
        RunId, RunMode, RunStatus, SessionEvent, StepId, StepRole, StepStatus, StopMode,
        WorkspaceId, WorkspaceStop, WorkspaceStopState, WorkspaceSummary,
    };
    use htui_core::store::{MemStore, ReadStore as _, StepFence, StoreError, WriteStore as _};
    use htui_orch::promote::StepResume;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::time::Duration;
    use uuid::Uuid;

    const LEASE: TimeDelta = TimeDelta::seconds(600);

    /// The clock every verb is handed: 2026-10-01 00:00 UTC.
    fn now() -> DateTime<Utc> {
        demo_at(30, 0)
    }

    /// The demo store with the caps and the workspace list `run_*` would read.
    struct Fixture {
        store: MemStore,
        caps: HashMap<AgentId, DriverCaps>,
        workspaces: Vec<WorkspaceSummary>,
        now: fn() -> DateTime<Utc>,
    }

    /// What one verb printed and how it ended.
    struct Ran {
        ended: Result<(), StateExit>,
        out: String,
        err: String,
    }

    impl Ran {
        fn refusal(&self) -> &str {
            match &self.ended {
                Err(StateExit::Refused(sentence)) => sentence,
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
    }

    /// How the confirmation is answered.
    #[derive(Clone, Copy)]
    enum Answer<'a> {
        /// `--yes`, on a terminal or not.
        Yes { interactive: bool },
        /// No `--yes`; a terminal answers this text.
        Typed(&'a str),
        /// No `--yes` and no terminal.
        NoTerminal,
    }

    impl Fixture {
        async fn demo() -> Self {
            let store = MemStore::demo();
            let caps = caps_map(store.agents().await.expect("agents"));
            let workspaces = store.workspaces().await.expect("workspaces");
            Self {
                store,
                caps,
                workspaces,
                now,
            }
        }

        /// Re-reads the workspace list after a test created one.
        async fn refresh(&mut self) {
            self.workspaces = self.store.workspaces().await.expect("workspaces");
        }

        fn ctx(&self) -> Context<'_> {
            Context {
                user: ids::USER,
                now: self.now,
                caps: &self.caps,
                workspaces: &self.workspaces,
                wait: STOP_WAIT,
                poll: STOP_POLL,
            }
        }

        fn graphics(&self) -> &WorkspaceSummary {
            self.workspaces
                .iter()
                .find(|ws| ws.workspace_id == ids::WORKSPACE_GRAPHICS)
                .expect("the demo has graphics")
        }

        async fn stop(&self, slug: &str, mode: StopMode, answer: Answer<'_>) -> Ran {
            let (yes, interactive, typed) = match answer {
                Answer::Yes { interactive } => (true, interactive, ""),
                Answer::Typed(text) => (false, true, text),
                Answer::NoTerminal => (false, false, ""),
            };
            let mut input = std::io::Cursor::new(typed.as_bytes().to_vec());
            let mut err = Vec::new();
            let mut out = Vec::new();
            let mut ask = Ask {
                yes,
                interactive,
                input: &mut input,
                question: &mut err,
            };
            let ended = stop_on(&self.store, &self.ctx(), slug, mode, &mut ask, &mut out).await;
            Ran {
                ended,
                out: String::from_utf8(out).expect("utf-8"),
                err: String::from_utf8(err).expect("utf-8"),
            }
        }

        async fn resume(&self, slug: &str, answer: Answer<'_>) -> Ran {
            let (yes, interactive, typed) = match answer {
                Answer::Yes { interactive } => (true, interactive, ""),
                Answer::Typed(text) => (false, true, text),
                Answer::NoTerminal => (false, false, ""),
            };
            let mut input = std::io::Cursor::new(typed.as_bytes().to_vec());
            let mut err = Vec::new();
            let mut out = Vec::new();
            let mut ask = Ask {
                yes,
                interactive,
                input: &mut input,
                question: &mut err,
            };
            let ended = resume_on(&self.store, &self.ctx(), slug, &mut ask, &mut out).await;
            Ran {
                ended,
                out: String::from_utf8(out).expect("utf-8"),
                err: String::from_utf8(err).expect("utf-8"),
            }
        }

        /// No `--yes`, on a terminal whose answer is read from `input`.
        async fn stop_asking(&self, slug: &str, mode: StopMode, input: &mut dyn BufRead) -> Ran {
            let mut err = Vec::new();
            let mut out = Vec::new();
            let mut ask = Ask {
                yes: false,
                interactive: true,
                input,
                question: &mut err,
            };
            let ended = stop_on(&self.store, &self.ctx(), slug, mode, &mut ask, &mut out).await;
            Ran {
                ended,
                out: String::from_utf8(out).expect("utf-8"),
                err: String::from_utf8(err).expect("utf-8"),
            }
        }

        /// [`Self::stop_asking`] for `htui resume`.
        async fn resume_asking(&self, slug: &str, input: &mut dyn BufRead) -> Ran {
            let mut err = Vec::new();
            let mut out = Vec::new();
            let mut ask = Ask {
                yes: false,
                interactive: true,
                input,
                question: &mut err,
            };
            let ended = resume_on(&self.store, &self.ctx(), slug, &mut ask, &mut out).await;
            Ran {
                ended,
                out: String::from_utf8(out).expect("utf-8"),
                err: String::from_utf8(err).expect("utf-8"),
            }
        }

        async fn row(&self, workspace: WorkspaceId) -> Option<WorkspaceStop> {
            self.store.workspace_stop(workspace).await.expect("read")
        }

        /// A queued graph run of a fresh `htui` item, started from `workspace` at `hour`.
        async fn queued(&self, workspace: WorkspaceId, hour: i64) -> RunId {
            let item = self
                .store
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
            let snapshot = self
                .store
                .run(ids::RUN_8)
                .await
                .expect("read")
                .and_then(|run| run.graph_snapshot)
                .expect("the demo's runs carry a snapshot");
            let snapshot = serde_json::from_value(snapshot).expect("a snapshot decodes");
            self.store
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
                    target: None,
                })
                .await
                .expect("create")
                .id
        }

        /// [`Self::queued`], claimed on the fixture box by `owner`.
        async fn claimed(&self, workspace: WorkspaceId, hour: i64, owner: Uuid) -> RunId {
            let run = self.queued(workspace, hour).await;
            assert_eq!(
                self.store
                    .claim_run(run, ids::BOX, owner, now(), LEASE)
                    .await
                    .expect("claim"),
                Claim::Admitted
            );
            run
        }

        /// A running step of `run` at `fanout_index`, on `agent`, holding `events`.
        async fn running_step(
            &self,
            run: RunId,
            owner: Uuid,
            fanout_index: i32,
            phase: &str,
            events: &[(EventKind, Value)],
        ) {
            let step = self
                .store
                .create_step(NewRunStep {
                    id: StepId::new(),
                    run_id: run,
                    position: 0,
                    attempt: 1,
                    fanout_index,
                    phase_name: phase.to_owned(),
                    agent_id: Some(ids::AGENT_CLAUDE_CLI),
                    model: None,
                    role: StepRole::Step,
                    parent_step_id: None,
                    node_key: None,
                })
                .await
                .expect("create step")
                .id;
            assert!(
                self.store
                    .transition_step(step, StepStatus::Pending, StepStatus::Running, now())
                    .await
                    .expect("start")
            );
            let rows: Vec<SessionEvent> = events
                .iter()
                .enumerate()
                .map(|(seq, (kind, payload))| SessionEvent {
                    run_step_id: step,
                    seq: i32::try_from(seq).expect("short"),
                    turn: 0,
                    kind: *kind,
                    role: EventRole::Agent,
                    tool_call_id: None,
                    payload: payload.clone(),
                    raw: None,
                    at: now(),
                })
                .collect();
            if !rows.is_empty() {
                self.store
                    .append_events(StepFence::Lease(owner), &rows)
                    .await
                    .expect("append");
            }
        }

        /// The executor's stop of a run it leases (M2): `stop_run`, never a bare transition.
        async fn stop_run(&self, run: RunId, owner: Uuid) {
            assert!(self.store.stop_run(run, owner).await.expect("stop_run"));
        }
    }

    /// A step's rows with a `session_started` banner.
    fn banner() -> Vec<(EventKind, Value)> {
        vec![
            (EventKind::Prompt, json!({ "text": "go" })),
            (
                EventKind::Other,
                json!({ "update": SESSION_STARTED, "body": { "session_id": "sess_1" } }),
            ),
            (EventKind::AssistantText, json!({ "text": "on it" })),
        ]
    }

    /// A step's rows without a banner.
    fn no_banner() -> Vec<(EventKind, Value)> {
        vec![
            (EventKind::Prompt, json!({ "text": "go" })),
            (EventKind::AssistantText, json!({ "text": "on it" })),
        ]
    }

    /// A terminal that answers `typed` once `meanwhile` ran: what someone else did while the
    /// question was open.
    struct Meanwhile<F: FnOnce()> {
        meanwhile: Option<F>,
        typed: std::io::Cursor<Vec<u8>>,
    }

    impl<F: FnOnce()> Meanwhile<F> {
        fn new(typed: &str, meanwhile: F) -> Self {
            Self {
                meanwhile: Some(meanwhile),
                typed: std::io::Cursor::new(typed.as_bytes().to_vec()),
            }
        }

        fn before(&mut self) {
            if let Some(meanwhile) = self.meanwhile.take() {
                meanwhile();
            }
        }
    }

    impl<F: FnOnce()> std::io::Read for Meanwhile<F> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.before();
            self.typed.read(buf)
        }
    }

    impl<F: FnOnce()> BufRead for Meanwhile<F> {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            self.before();
            self.typed.fill_buf()
        }

        fn consume(&mut self, amount: usize) {
            self.typed.consume(amount);
        }
    }

    /// Another user's write, made while a question is open: `MemStore` never pends, so a
    /// blocking poll of its future is the write itself.
    fn meanwhile<T>(write: impl Future<Output = Result<T, StoreError>>) -> T {
        futures::executor::block_on(write).expect("the other user's write")
    }

    const GRACEFUL_Q: &str = "Stop workspace `graphics` gracefully? Each agent step stops at its \
                              turn boundary, checks stop at once; nothing new starts until `htui \
                              resume graphics`. [y/N] ";
    const FORCED_Q: &str = "Stop workspace `graphics` now? Every step is killed mid-turn and redoes \
                            its turn on resume; nothing new starts until `htui resume graphics`. \
                            [y/N] ";
    const ESCALATE_Q: &str = "Workspace `graphics` is stopped gracefully; force it? Every step still \
                              running is killed mid-turn. [y/N] ";
    const RESUME_Q: &str = "Resume workspace `graphics`? Its stopped runs resume on the boxes that \
                            ran them and its queued runs can start. [y/N] ";

    /// D2, D5: the preview on stdout, the graceful question on stderr, `y` confirms; the row is
    /// written with this user and clock, and with no running run the report reads `stopped` at
    /// once, with no wait line.
    #[tokio::test(start_paused = true)]
    async fn a_stop_asks_then_writes_graceful_and_reports_stopped() {
        let fx = Fixture::demo().await;
        let ran = fx
            .stop("graphics", StopMode::Graceful, Answer::Typed(" Y \n"))
            .await;
        ran.ended.as_ref().expect("stopped");
        assert_eq!(ran.err, GRACEFUL_Q, "the question, and no wait line");
        let row = fx.row(ids::WORKSPACE_GRAPHICS).await.expect("a stop row");
        assert_eq!(row.mode, StopMode::Graceful);
        assert_eq!(row.requested_by, ids::USER);
        assert_eq!(row.requested_at, now());
        let (preview, after) = ran
            .out
            .split_once("workspace graphics: stopped")
            .expect("the final report");
        assert!(
            preview.starts_with("workspace graphics: not stopped\n"),
            "{}",
            ran.out
        );
        assert!(
            after.starts_with(" (graceful, requested 2026-10-01 00:00 UTC)\n"),
            "{}",
            ran.out
        );
        assert!(!ran.out.contains("still stopping"), "{}", ran.out);
    }

    thread_local! {
        /// [`clock`]'s instant: a `tokio::test` runs on its own thread.
        static CLOCK: core::cell::Cell<DateTime<Utc>> = core::cell::Cell::new(now());
    }

    /// A clock a test moves by hand.
    fn clock() -> DateTime<Utc> {
        CLOCK.with(core::cell::Cell::get)
    }

    /// D5: the stop's `requested_at` is read when it is written, after the answer, not when the
    /// question was asked: a prompt left open an hour records the hour it was answered.
    #[tokio::test(start_paused = true)]
    async fn a_stop_is_stamped_when_answered_not_when_asked() {
        let mut fx = Fixture::demo().await;
        fx.now = clock;
        let mut input = Meanwhile::new("y\n", || CLOCK.with(|at| at.set(demo_at(30, 1))));
        let ran = fx
            .stop_asking("graphics", StopMode::Graceful, &mut input)
            .await;
        ran.ended.as_ref().expect("stopped");
        assert_eq!(
            fx.row(ids::WORKSPACE_GRAPHICS)
                .await
                .map(|row| row.requested_at),
            Some(demo_at(30, 1))
        );
    }

    /// D5: `--force` asks the forced question and writes `forced`; over a graceful row it asks the
    /// escalation and the store escalates.
    #[tokio::test(start_paused = true)]
    async fn force_writes_forced_and_escalates_a_graceful_stop() {
        let fx = Fixture::demo().await;
        let ran = fx
            .stop("graphics", StopMode::Forced, Answer::Typed("yes\n"))
            .await;
        ran.ended.as_ref().expect("stopped");
        assert_eq!(ran.err, FORCED_Q);
        assert_eq!(
            fx.row(ids::WORKSPACE_GRAPHICS).await.map(|row| row.mode),
            Some(StopMode::Forced)
        );

        let fx = Fixture::demo().await;
        fx.stop(
            "graphics",
            StopMode::Graceful,
            Answer::Yes { interactive: false },
        )
        .await
        .ended
        .expect("graceful");
        let ran = fx
            .stop("graphics", StopMode::Forced, Answer::Typed("y\n"))
            .await;
        ran.ended.as_ref().expect("escalated");
        assert_eq!(ran.err, ESCALATE_Q);
        assert_eq!(
            fx.row(ids::WORKSPACE_GRAPHICS).await.map(|row| row.mode),
            Some(StopMode::Forced)
        );
        assert!(
            ran.out.contains("workspace graphics: stopped (forced, "),
            "{}",
            ran.out
        );
    }

    /// D5: graceful over graceful and anything over forced write nothing and ask nothing (no
    /// terminal, no `--yes`, still answered); the first request's instant is kept.
    #[tokio::test(start_paused = true)]
    async fn a_repeat_stop_writes_nothing_and_asks_nothing() {
        let mut fx = Fixture::demo().await;
        fx.stop(
            "graphics",
            StopMode::Graceful,
            Answer::Yes { interactive: true },
        )
        .await
        .ended
        .expect("stopped");
        fx.now = || demo_at(31, 0);
        let ran = fx
            .stop("graphics", StopMode::Graceful, Answer::NoTerminal)
            .await;
        ran.ended.as_ref().expect("a repeat is answered");
        assert_eq!(ran.err, "");
        assert!(
            ran.out.starts_with(
                "workspace graphics is already stopped (graceful)\nworkspace graphics: stopped \
                 (graceful, requested 2026-10-01 00:00 UTC)\n"
            ),
            "{}",
            ran.out
        );
        assert_eq!(
            fx.row(ids::WORKSPACE_GRAPHICS)
                .await
                .map(|row| row.requested_at),
            Some(now()),
            "the first request's instant is kept"
        );

        fx.stop(
            "graphics",
            StopMode::Forced,
            Answer::Yes { interactive: false },
        )
        .await
        .ended
        .expect("escalated");
        let stored = fx.row(ids::WORKSPACE_GRAPHICS).await;
        fx.now = || demo_at(32, 0);
        for mode in [StopMode::Graceful, StopMode::Forced] {
            let ran = fx.stop("graphics", mode, Answer::NoTerminal).await;
            ran.ended.as_ref().expect("a repeat is answered");
            assert_eq!(ran.err, "");
            assert!(
                ran.out
                    .starts_with("workspace graphics is already stopped (forced)\n"),
                "{}",
                ran.out
            );
            assert_eq!(
                ran.out
                    .contains("a graceful stop does not downgrade a forced one\n"),
                mode == StopMode::Graceful,
                "{}",
                ran.out
            );
            assert_eq!(fx.row(ids::WORKSPACE_GRAPHICS).await, stored, "{mode}");
        }
    }

    /// D2: anything but `y`/`yes`, an empty line and EOF decline: exit 2, nothing written.
    #[tokio::test(start_paused = true)]
    async fn declining_or_eof_writes_nothing_and_is_refused() {
        let fx = Fixture::demo().await;
        for typed in ["n\n", "\n", "", "nope\n", "yess\n"] {
            let ran = fx
                .stop("graphics", StopMode::Graceful, Answer::Typed(typed))
                .await;
            assert_eq!(
                ran.refusal(),
                "not confirmed; nothing was stopped",
                "{typed:?}"
            );
            assert_eq!(ran.ended.as_ref().err().map(StateExit::code), Some(2));
            assert_eq!(fx.row(ids::WORKSPACE_GRAPHICS).await, None, "{typed:?}");
        }
        fx.stop(
            "graphics",
            StopMode::Graceful,
            Answer::Yes { interactive: true },
        )
        .await
        .ended
        .expect("stopped");
        let ran = fx.resume("graphics", Answer::Typed("no\n")).await;
        assert_eq!(ran.refusal(), "not confirmed; nothing was resumed");
        assert!(fx.row(ids::WORKSPACE_GRAPHICS).await.is_some());
    }

    /// D2: no terminal and no `--yes` is refused before the preview, the question and any write;
    /// `--yes` without a terminal writes, and asks nothing.
    #[tokio::test(start_paused = true)]
    async fn no_terminal_without_yes_is_refused_before_any_write() {
        let fx = Fixture::demo().await;
        let ran = fx
            .stop("graphics", StopMode::Graceful, Answer::NoTerminal)
            .await;
        assert_eq!(
            ran.refusal(),
            "needs confirmation: pass --yes, or run it in a terminal; nothing was stopped"
        );
        assert_eq!((ran.out.as_str(), ran.err.as_str()), ("", ""));
        assert_eq!(fx.row(ids::WORKSPACE_GRAPHICS).await, None);

        let ran = fx
            .stop(
                "graphics",
                StopMode::Graceful,
                Answer::Yes { interactive: false },
            )
            .await;
        ran.ended.as_ref().expect("stopped");
        assert_eq!(ran.err, "");
        assert!(
            ran.out.starts_with("workspace graphics: stopped (graceful"),
            "no preview: {}",
            ran.out
        );
        assert!(fx.row(ids::WORKSPACE_GRAPHICS).await.is_some());

        let ran = fx.resume("graphics", Answer::NoTerminal).await;
        assert_eq!(
            ran.refusal(),
            "needs confirmation: pass --yes, or run it in a terminal; nothing was resumed"
        );
        assert!(fx.row(ids::WORKSPACE_GRAPHICS).await.is_some());
    }

    /// D3: an unknown slug is refused (exit 2) naming the slugs there are; nothing is asked.
    #[tokio::test(start_paused = true)]
    async fn an_unknown_slug_is_refused_naming_the_workspaces() {
        let fx = Fixture::demo().await;
        let ran = fx
            .stop("Graphics", StopMode::Graceful, Answer::Typed("y\n"))
            .await;
        let refusal = ran.refusal().to_owned();
        assert!(
            refusal.starts_with("no workspace with slug `Graphics`"),
            "{refusal}"
        );
        for ws in &fx.workspaces {
            assert!(refusal.contains(&format!("`{}`", ws.slug)), "{refusal}");
        }
        assert_eq!((ran.out.as_str(), ran.err.as_str()), ("", ""));
        let ran = fx.resume("nope", Answer::Yes { interactive: true }).await;
        assert!(ran.refusal().starts_with("no workspace with slug `nope`"));
    }

    /// D4: a workspace with no run that is not finished says so.
    #[tokio::test(start_paused = true)]
    async fn a_workspace_without_runs_reports_none() {
        let mut fx = Fixture::demo().await;
        fx.store
            .create_workspace(NewWorkspace {
                id: WorkspaceId::new(),
                slug: "empty".to_owned(),
                name: "Empty".to_owned(),
                description: String::new(),
                created_by: ids::USER,
            })
            .await
            .expect("create");
        fx.refresh().await;
        let ran = fx
            .stop(
                "empty",
                StopMode::Graceful,
                Answer::Yes { interactive: false },
            )
            .await;
        ran.ended.as_ref().expect("stopped");
        assert_eq!(
            ran.out,
            "workspace empty: stopped (graceful, requested 2026-10-01 00:00 UTC)\n  no runs in \
             this workspace\n"
        );
    }

    /// D7: a leased running run nobody settles keeps the workspace `stopping`; the verb waits
    /// `wait`, says so once on stderr, prints the report as it stands and exits 0.
    #[tokio::test(start_paused = true)]
    async fn a_running_run_keeps_it_stopping_until_the_wait_runs_out() {
        let fx = Fixture::demo().await;
        fx.claimed(ids::WORKSPACE_GRAPHICS, 1, Uuid::now_v7()).await;
        let started = tokio::time::Instant::now();
        let ran = fx
            .stop(
                "graphics",
                StopMode::Graceful,
                Answer::Yes { interactive: false },
            )
            .await;
        ran.ended
            .as_ref()
            .expect("a wait that runs out is no failure");
        assert!(started.elapsed() >= STOP_WAIT, "{:?}", started.elapsed());
        assert!(
            started.elapsed() < STOP_WAIT + STOP_POLL + Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(ran.err, "waiting for 1 running run to stop…\n");
        assert!(
            ran.out
                .starts_with("workspace graphics: stopping (graceful, "),
            "{}",
            ran.out
        );
        assert!(ran.out.contains("running on DESKTOP-HTUI"), "{}", ran.out);
        assert!(
            ran.out
                .ends_with("still stopping; run htui stop graphics again to see it settle\n"),
            "{}",
            ran.out
        );
    }

    /// D4: one line per run that is not finished, in `(queued_at, id)` order, saying how it will
    /// resume: its own session reopened, a fresh one with the handoff, the step run again, a run
    /// with no stopped step, a box-less run (the demo's `RUN_8`), a queued, a parked and a
    /// running run. Run ids are redacted (`RunId::new` is time-ordered).
    #[tokio::test(start_paused = true)]
    async fn the_report_says_how_each_run_resumes() {
        let fx = Fixture::demo().await;
        let ws = ids::WORKSPACE_GRAPHICS;
        let owner = Uuid::now_v7();

        let reopen = fx.claimed(ws, 1, owner).await;
        fx.running_step(reopen, owner, 0, "implement", &banner())
            .await;
        fx.stop_run(reopen, owner).await;
        let fresh = fx.claimed(ws, 2, owner).await;
        fx.running_step(fresh, owner, 0, "review", &no_banner())
            .await;
        fx.stop_run(fresh, owner).await;
        let parts = fx.claimed(ws, 3, owner).await;
        fx.running_step(parts, owner, 0, "research", &no_banner())
            .await;
        fx.running_step(parts, owner, 1, "research", &[]).await;
        fx.stop_run(parts, owner).await;
        let bare = fx.claimed(ws, 4, owner).await;
        fx.stop_run(bare, owner).await;
        fx.queued(ws, 5).await;
        let parked = fx.queued(ws, 6).await;
        for (from, to) in [
            (RunStatus::Queued, RunStatus::Running),
            (RunStatus::Running, RunStatus::AwaitingApproval),
        ] {
            assert!(
                fx.store
                    .transition_run(parked, from, to, now())
                    .await
                    .expect("park")
            );
        }
        fx.claimed(ws, 7, owner).await;
        fx.store
            .stop_workspace(ws, StopMode::Graceful, ids::USER, now())
            .await
            .expect("stop");

        let report = report(&fx.store, &fx.ctx(), fx.graphics())
            .await
            .expect("report");
        assert_eq!(report.state, WorkspaceStopState::Stopping);
        let resumes: Vec<&RunResume> = report.lines.iter().map(|line| &line.resume).collect();
        assert_eq!(
            resumes,
            [
                &RunResume::NoBox,
                &RunResume::Steps(vec![(
                    "implement".to_owned(),
                    StepResume::Reopen(htui_agent::driver::AgentSessionRef("sess_1".to_owned()))
                )]),
                &RunResume::Steps(vec![("review".to_owned(), StepResume::FreshWithHandoff)]),
                &RunResume::Steps(vec![
                    ("research".to_owned(), StepResume::FreshWithHandoff),
                    ("research".to_owned(), StepResume::Rerun),
                ]),
                &RunResume::Continues,
                &RunResume::Held,
                &RunResume::AtGate,
                &RunResume::Stopping,
            ]
        );
        insta::with_settings!({ filters => vec![(r"\b[0-9a-f]{8}\b", "[run]")] }, {
            insta::assert_snapshot!("report_stopping", report.to_string());
        });
    }

    /// D6: a stopped workspace asks, deletes the row and reports under the resume line.
    #[tokio::test(start_paused = true)]
    async fn resume_asks_then_deletes_the_row() {
        let fx = Fixture::demo().await;
        fx.stop(
            "graphics",
            StopMode::Forced,
            Answer::Yes { interactive: true },
        )
        .await
        .ended
        .expect("stopped");
        let ran = fx.resume("graphics", Answer::Typed("y\n")).await;
        ran.ended.as_ref().expect("resumed");
        assert_eq!(ran.err, RESUME_Q);
        assert_eq!(fx.row(ids::WORKSPACE_GRAPHICS).await, None);
        let (preview, after) = ran
            .out
            .split_once(
                "resume requested: each box resumes its stopped runs at its next command poll\n",
            )
            .expect("the resume line");
        assert!(
            preview.starts_with("workspace graphics: stopped (forced, "),
            "{}",
            ran.out
        );
        assert!(
            after.starts_with("workspace graphics: not stopped\n"),
            "{}",
            ran.out
        );
    }

    /// D6: no row: no question, no write; the report still lists the box-less stopped run.
    #[tokio::test(start_paused = true)]
    async fn resume_of_a_workspace_not_stopped_asks_nothing_and_lists_stopped_runs() {
        let fx = Fixture::demo().await;
        let ran = fx.resume("graphics", Answer::NoTerminal).await;
        ran.ended.as_ref().expect("answered");
        assert_eq!(ran.err, "");
        let run8 = &ids::RUN_8.to_string()[..8];
        assert_eq!(
            ran.out,
            format!(
                "workspace graphics is not stopped\nworkspace graphics: not stopped\n  FEAT-1  \
                 {run8}  stopped, no box  cannot be resumed: cancel it from the Runs pane\n"
            )
        );
    }

    /// D6: a resume deletes only the stop it previewed. Someone else resumed, then forced a new
    /// stop, while the question was open: the forced stop stands and the resume is refused (2);
    /// the stop gone meanwhile is refused the same way.
    #[tokio::test(start_paused = true)]
    async fn a_resume_refuses_a_stop_that_changed_while_asking() {
        let fx = Fixture::demo().await;
        let ws = ids::WORKSPACE_GRAPHICS;
        let resume_again = "the stop of workspace graphics changed while asking; nothing was \
                            resumed; run htui resume graphics again";
        fx.stop(
            "graphics",
            StopMode::Graceful,
            Answer::Yes { interactive: true },
        )
        .await
        .ended
        .expect("stopped");
        let store = fx.store.clone();
        let mut input = Meanwhile::new("y\n", move || {
            assert!(meanwhile(store.resume_workspace(ws)));
            meanwhile(store.stop_workspace(ws, StopMode::Forced, ids::USER, demo_at(30, 1)));
        });
        let ran = fx.resume_asking("graphics", &mut input).await;
        assert_eq!(ran.refusal(), resume_again);
        assert_eq!(ran.ended.as_ref().err().map(StateExit::code), Some(2));
        let row = fx.row(ws).await.expect("the forced stop stands");
        assert_eq!(
            (row.mode, row.requested_at),
            (StopMode::Forced, demo_at(30, 1))
        );

        let store = fx.store.clone();
        let mut input = Meanwhile::new("y\n", move || {
            assert!(meanwhile(store.resume_workspace(ws)));
        });
        let ran = fx.resume_asking("graphics", &mut input).await;
        assert_eq!(ran.refusal(), resume_again);
        assert_eq!(fx.row(ws).await, None);
    }

    /// D5: `--force` over a graceful stop that someone resumed while the question was open writes
    /// no fresh forced stop; a stop over a workspace someone stopped meanwhile is refused alike.
    #[tokio::test(start_paused = true)]
    async fn a_stop_refuses_a_stop_that_changed_while_asking() {
        let fx = Fixture::demo().await;
        let ws = ids::WORKSPACE_GRAPHICS;
        fx.stop(
            "graphics",
            StopMode::Graceful,
            Answer::Yes { interactive: true },
        )
        .await
        .ended
        .expect("stopped");
        let store = fx.store.clone();
        let mut input = Meanwhile::new("y\n", move || {
            assert!(meanwhile(store.resume_workspace(ws)));
        });
        let ran = fx
            .stop_asking("graphics", StopMode::Forced, &mut input)
            .await;
        assert_eq!(ran.err, ESCALATE_Q);
        assert_eq!(
            ran.refusal(),
            "the stop of workspace graphics changed while asking; nothing was stopped; run htui \
             stop graphics --force again"
        );
        assert_eq!(fx.row(ws).await, None, "no fresh forced stop");

        let store = fx.store.clone();
        let mut input = Meanwhile::new("y\n", move || {
            meanwhile(store.stop_workspace(ws, StopMode::Graceful, ids::USER, demo_at(30, 1)));
        });
        let ran = fx
            .stop_asking("graphics", StopMode::Graceful, &mut input)
            .await;
        assert_eq!(
            ran.refusal(),
            "the stop of workspace graphics changed while asking; nothing was stopped; run htui \
             stop graphics again"
        );
        assert_eq!(
            fx.row(ws).await.map(|row| row.requested_at),
            Some(demo_at(30, 1)),
            "the other user's stop stands"
        );
    }

    /// D7: the wait ends at the first poll after the running run settles, well before
    /// `ctx.wait`: the header reads `stopped` and nothing says `still stopping`.
    #[tokio::test(start_paused = true)]
    async fn the_wait_ends_once_the_runs_settle() {
        let fx = Fixture::demo().await;
        let owner = Uuid::now_v7();
        let run = fx.claimed(ids::WORKSPACE_GRAPHICS, 1, owner).await;
        let settles = Duration::from_secs(3);
        let started = tokio::time::Instant::now();
        let (ran, ()) = tokio::join!(
            fx.stop(
                "graphics",
                StopMode::Graceful,
                Answer::Yes { interactive: false },
            ),
            async {
                tokio::time::sleep(settles).await;
                fx.stop_run(run, owner).await;
            },
        );
        ran.ended.as_ref().expect("stopped");
        let waited = started.elapsed();
        assert!(waited >= settles, "{waited:?}");
        assert!(waited <= settles + STOP_POLL, "{waited:?}");
        assert!(
            STOP_WAIT >= settles + STOP_POLL * 10,
            "well under the bound"
        );
        assert_eq!(ran.err, "waiting for 1 running run to stop…\n");
        assert!(
            ran.out
                .starts_with("workspace graphics: stopped (graceful, "),
            "{}",
            ran.out
        );
        assert!(!ran.out.contains("still stopping"), "{}", ran.out);
    }

    /// D3: `Unreachable` is offline (3), a vanished workspace refused (2), anything else failed
    /// (1).
    #[test]
    fn store_failures_map_to_the_state_exits() {
        let ended = store_exit(&StoreError::Unreachable("refused".into()), "gfx", "stopped");
        assert_eq!(ended.code(), 3);
        assert_eq!(
            ended.to_string(),
            "cannot reach Postgres: store unreachable: refused"
        );
        let ended = store_exit(
            &StoreError::NotFound {
                entity: "workspace",
                id: WorkspaceId::new().to_string(),
            },
            "gfx",
            "resumed",
        );
        assert_eq!(ended.code(), 2);
        assert_eq!(ended.to_string(), "workspace gfx no longer exists");
        let ended = store_exit(&StoreError::Backend("boom".into()), "gfx", "stopped");
        assert_eq!(ended.code(), 1);
        assert_eq!(
            ended.to_string(),
            "the workspace could not be stopped: store backend error: boom"
        );
    }
}
