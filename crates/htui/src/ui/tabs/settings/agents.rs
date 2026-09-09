//! The agent registry section of the Settings tab (`R-TUI-8`, MOD-2 D14, D54, MOD-20 D19, D20).
//!
//! It lists what `agent` and this box's `agent_box` hold, and it is where a probe is asked for:
//! `r` sends [`StoreRequest::ProbeAgents`] and the `on this box` column then says what this box
//! can actually run (`R-AGT-6`). The caps banner is milestone 3's and the quota column milestone
//! 7's — each is a column in this table and a field in the reply, not a rewrite of the section.
//!
//! Since MOD-20 it is also where an adapter is **installed**: `i` on the highlighted row asks for
//! a pre-flight, the plan that comes back is drawn as a consent pane under the table, and only `y`
//! fetches a byte. The pane is in the section rather than in an overlay (MOD-20 D19) because an
//! overlay factory takes no argument and so cannot be told which row, and because replies to a
//! popped overlay are dropped — the section is the one view that outlives a whole install.
//!
//! Two costs of answering the probe with the reply this section already reads. One: **any**
//! [`StoreReply::Agents`] clears the in-flight *probe* state, so a re-activation or a scope change
//! while a probe is running puts the pre-probe rows back for a moment. The probe's own reply
//! supersedes them when it lands, and the alternative was a second reply variant nothing else
//! would ever use. Two: the install state deliberately does **not** join it (hazard H-17) — a
//! probe that loses its flag re-renders one column, an install that lost its state would leave a
//! running download with nothing on screen to cancel it with.

use htui_agent::install::PlanError;
use htui_agent::{AgentLaunch, InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{AgentId, AgentSummary, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use serde_json::Value;
use std::path::Path;

use crate::app::{Action, Ctx, Handled};
use crate::store_worker::{InstallFrame, StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::{KeyCode, KeyEvent};

/// What the `on this box` column reads while this box has no `agent_box` row for the agent.
const NOT_PROBED: &str = "not probed";

/// What the `on this box` column reads while a probe this section asked for is in flight.
const PROBING: &str = "probing\u{2026}";

/// What the `on this box` column reads between `x` and the install's own last frame.
const CANCELLING: &str = "cancelling\u{2026}";

/// What replaces an absent `default_model`.
const NONE: &str = "\u{2014}";

/// The hint line with nothing in flight. It stands in for a help entry: a Settings section has no
/// [`KeyScope`](crate::keymap::KeyScope) of its own (MOD-20 D19), so the keys are written where
/// they are pressed.
const HINT_IDLE: &str = "j/k select \u{b7} r probe \u{b7} i install";

/// The hint line while a plan waits for an answer.
const HINT_PENDING: &str = "y install \u{b7} n cancel";

/// The hint line while an install streams.
const HINT_RUNNING: &str = "x cancel install";

/// The hint line while the manual steps are up.
const HINT_MANUAL: &str = "Esc close";

/// What the section says of an install the user turned down.
const DECLINED: &str = "install declined";

/// What the section says of an install that was stopped.
const CANCELLED: &str = "install cancelled";

/// How far an install this section asked for has got (MOD-20 D19).
///
/// One state rather than a handful of flags because the states are exclusive by construction: the
/// runtime allows one install at a time (hazard H-10), and every key this section binds means
/// something different in each of them.
#[derive(Debug, Default)]
enum InstallState {
    /// Nothing is planned, running or waiting to be read.
    #[default]
    Idle,
    /// `i` was pressed and the pre-flight has not answered yet.
    Planning {
        /// The row it was pressed on.
        agent_id: AgentId,
    },
    /// A plan is on screen and the section is waiting for `y`, `n` or `Esc`.
    ///
    /// The plan is held whole rather than summarised: [`StoreRequest::InstallConfirm`] carries it
    /// back unchanged, which is what makes what the user read and what the installer executes the
    /// same value (MOD-20 D12).
    Pending {
        /// Exactly what `y` accepts.
        plan: Box<InstallPlan>,
    },
    /// The user consented and the stream is running.
    Running {
        /// Whose row the progress cell belongs to.
        agent_id: AgentId,
        /// The phase of the last frame.
        phase: InstallPhase,
        /// How far into that phase.
        done: u64,
        /// The denominator, when the phase has one.
        total: Option<u64>,
        /// `x` was pressed, or the runtime acknowledged one.
        cancelling: bool,
    },
    /// MOD-20 D20: the install failed in a way the user can route around by hand.
    Manual {
        /// The steps, every word of them derived from the row and the registry helper.
        steps: Box<ManualSteps>,
        /// Why the install stopped.
        message: String,
    },
}

/// The registry as a table of name, transport, billing, models, default, enabled and per-box
/// state, with a row cursor and the install one row at a time.
#[derive(Debug, Default)]
pub struct AgentsSection {
    /// The registry, ordered by name.
    agents: Vec<AgentSummary>,
    /// Set when the store could not answer. Since MOD-2 milestone 4 mirrored `agent` (plan D31) an
    /// offline backend answers this section, so in practice only a store that is neither online nor
    /// holding a mirror gets here.
    unavailable: Option<String>,
    /// `r` was pressed and no reply has come back yet. A second `r` is refused (plan D54): a probe
    /// spawns a process per agent, and two of them racing to write the same `agent_box` rows would
    /// be paid for twice for one answer.
    probing: bool,
    /// The highlighted row — the first cursor in a Settings section (MOD-20 D19).
    ///
    /// `i` acts on one row, so there has to be a row it acts on. [`TableState`] is ratatui's own,
    /// and `R-TUI-8`'s quota column will want it too.
    cursor: TableState,
    /// The install this section is waiting on.
    install: InstallState,
    /// The last outcome, one line on the hint row.
    notice: Option<String>,
}

impl AgentsSection {
    /// Identity of the agents section.
    pub const ID: SectionId = SectionId("agents");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The highlighted row, or `None` while the table is empty.
    fn selected(&self) -> Option<&AgentSummary> {
        self.cursor
            .selected()
            .and_then(|index| self.agents.get(index))
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, deliberately: `i` installs whatever the cursor is on, and a cursor that jumped
    /// from the last row to the first would make a held `j` install a row the user never aimed at.
    fn move_cursor(&mut self, down: bool) {
        let Some(last) = self.agents.len().checked_sub(1) else {
            self.cursor.select(None);
            return;
        };
        let current = self.cursor.selected().unwrap_or(0).min(last);
        let next = if down {
            current.saturating_add(1).min(last)
        } else {
            current.saturating_sub(1)
        };
        self.cursor.select(Some(next));
    }

    /// Puts the cursor back inside the table after a read replaced the rows.
    ///
    /// A fresh read selects the first row rather than nothing, so `i` has something to act on the
    /// moment the section is looked at.
    fn clamp_cursor(&mut self) {
        match self.agents.len().checked_sub(1) {
            Some(last) => self
                .cursor
                .select(Some(self.cursor.selected().unwrap_or(0).min(last))),
            None => self.cursor.select(None),
        }
    }

    /// Whether an install this section asked for holds the runtime's claim (hazard H-10).
    ///
    /// Planning counts: the runtime's claim covers the pre-flight as well as the download, so a
    /// second `i` during either is refused here rather than reaching a refusal on the worker.
    fn install_in_flight(&self) -> bool {
        matches!(
            self.install,
            InstallState::Planning { .. } | InstallState::Running { .. }
        )
    }

    /// `i`: pre-flight the highlighted row, or say why not.
    ///
    /// Every refusal is a sentence with the row's own name in it, and every one of them is reached
    /// without a request: the runtime would refuse the same cases (`agent_worker::install_plan`),
    /// but the answer is already in the document this section is holding, and `R-AGT-10` is about
    /// spending nothing before consent.
    fn begin_install(&mut self, ctx: &mut Ctx<'_>) {
        if self.install_in_flight() {
            ctx.emit(Action::Error("an install is already running".to_owned()));
            return;
        }
        if self.probing {
            ctx.emit(Action::Error("a probe is running".to_owned()));
            return;
        }
        let Some(summary) = self.selected() else {
            ctx.emit(Action::Error("no agent row is selected".to_owned()));
            return;
        };
        if !declares_a_source(&summary.agent.launch) {
            let refusal = PlanError::NoSource {
                agent: summary.agent.name.clone(),
            };
            ctx.emit(Action::Error(refusal.to_string()));
            return;
        }
        let agent_id = summary.agent.id;
        self.install = InstallState::Planning { agent_id };
        self.notice = None;
        ctx.request(StoreRequest::InstallPlan { agent_id });
    }

    /// The keys a pending plan answers, and the ones it swallows.
    ///
    /// Modality, local and sufficient (MOD-20 D19): the tab consumes `h`/`l`/`[`/`]`/arrows before
    /// the section is offered a key, so the strip still works and a plan waits rather than traps.
    fn answer_consent(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('y') => {
                if let InstallState::Pending { plan } =
                    core::mem::replace(&mut self.install, InstallState::Idle)
                {
                    self.install = InstallState::Running {
                        agent_id: plan.agent_id,
                        phase: InstallPhase::Planning,
                        done: 0,
                        total: None,
                        cancelling: false,
                    };
                    self.notice = None;
                    ctx.request(StoreRequest::InstallConfirm { plan });
                }
                Handled::Consumed
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.install = InstallState::Idle;
                self.notice = Some(DECLINED.to_owned());
                Handled::Consumed
            }
            // Swallowed rather than passed on: below this pane is a table whose cursor decides
            // what `i` installs, and a key that moved it would move what the user is consenting to.
            // **Only this section's own keys**, though. `App::on_key` (`app/state.rs:380-421`)
            // offers the active tab the key *before* the `Tab` and `Global` keymaps and returns on
            // `Consumed`, so a blanket `_ => Consumed` here makes `q`, `?`, `Tab` and the digit
            // tab-switches dead for as long as the pane is open — the user cannot even quit. The
            // pane is modal over the table beneath it, not over the application.
            KeyCode::Char('j' | 'k' | 'i' | 'r' | 'x') => Handled::Consumed,
            _ => Handled::Pass,
        }
    }

    /// One frame of an install stream (MOD-20 D18).
    fn on_install_frame(&mut self, frame: &InstallFrame, ctx: &mut Ctx<'_>) {
        match frame {
            InstallFrame::Plan(plan) => {
                self.install = InstallState::Pending { plan: plan.clone() };
                self.notice = None;
            }
            InstallFrame::Progress { phase, done, total } => {
                if let InstallState::Running {
                    phase: at,
                    done: reached,
                    total: of,
                    ..
                } = &mut self.install
                {
                    *at = *phase;
                    *reached = *done;
                    *of = *total;
                }
            }
            // There is no `Installed` state here, on purpose (`R-AGT-6`): the row was written
            // before this frame was sent, so the honest thing to render is what a fresh read of
            // the registry says — including "the probe could not run it".
            InstallFrame::Done(outcome) => {
                self.install = InstallState::Idle;
                self.notice = Some(outcome_notice(outcome));
                ctx.request(StoreRequest::Agents);
            }
            InstallFrame::Cancelling => {
                if let InstallState::Running { cancelling, .. } = &mut self.install {
                    *cancelling = true;
                }
            }
            InstallFrame::Cancelled => {
                self.install = InstallState::Idle;
                self.notice = Some(CANCELLED.to_owned());
            }
            InstallFrame::Failed {
                message,
                manual: Some(steps),
            } => {
                self.install = InstallState::Manual {
                    steps: steps.clone(),
                    message: message.clone(),
                };
                self.notice = None;
            }
            InstallFrame::Failed { message, .. } => {
                self.install = InstallState::Idle;
                self.notice = Some(message.clone());
            }
        }
    }

    /// What the `on this box` column says about one row, in the order plan D54 sets.
    ///
    /// An install this section is running wins over everything, because it is the one thing on
    /// screen the user is waiting on. Then a probe in flight, because none of the rows on screen
    /// answer the question they just asked. Then a box with no `agent_box` row at all. Then the
    /// snapshot's own verdict when it is not `ready` — `missing`, `unauthenticated` and `failed`
    /// are the three facts a version string cannot express (plan D50). Everything left is a row
    /// that works, or one written before the `probe` column existed, and both are answered by the
    /// version.
    fn on_box_cell(&self, summary: &AgentSummary) -> String {
        if let Some(cell) = self.install_cell(summary.agent.id) {
            return cell;
        }
        if self.probing {
            return PROBING.to_owned();
        }
        let Some(row) = summary.on_box.as_ref() else {
            return NOT_PROBED.to_owned();
        };
        let status = row
            .probe
            .as_ref()
            .and_then(|probe| probe.get("status"))
            .and_then(Value::as_str);
        if let Some(status @ ("missing" | "unauthenticated" | "failed")) = status {
            return status.to_owned();
        }
        let version = row.version.as_deref().unwrap_or(NONE);
        if row.enabled {
            version.to_owned()
        } else {
            format!("{version} (off)")
        }
    }

    /// The progress cell of the row an install is running on, or `None` for every other row.
    fn install_cell(&self, agent_id: AgentId) -> Option<String> {
        match &self.install {
            InstallState::Planning { agent_id: running } if *running == agent_id => {
                Some(phase_cell(InstallPhase::Planning, 0, None))
            }
            InstallState::Running {
                agent_id: running,
                cancelling: true,
                ..
            } if *running == agent_id => Some(CANCELLING.to_owned()),
            InstallState::Running {
                agent_id: running,
                phase,
                done,
                total,
                ..
            } if *running == agent_id => Some(phase_cell(*phase, *done, *total)),
            _ => None,
        }
    }

    /// The pane drawn under the table: a consent, a set of manual steps, or nothing.
    ///
    /// One line longer than what it is a pane *of*, in both cases that have one: a blank line
    /// separates a consent from the table it covers, and a failure's own sentence heads the steps
    /// it made necessary.
    fn pane(&self, theme: &Theme) -> Vec<Line<'static>> {
        match &self.install {
            InstallState::Pending { plan } => core::iter::once(Line::default())
                .chain(
                    plan.consent_lines()
                        .into_iter()
                        .map(|line| Line::styled(line, theme.base)),
                )
                .collect(),
            InstallState::Manual { steps, message } => {
                core::iter::once(Line::styled(message.clone(), theme.error))
                    .chain(
                        steps
                            .lines()
                            .into_iter()
                            .map(|line| Line::styled(line, theme.base)),
                    )
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The one line under the pane: which keys mean something here, and the last outcome.
    fn hint(&self) -> String {
        let keys = match &self.install {
            InstallState::Idle => HINT_IDLE,
            // A pre-flight is one registry read and one `HEAD`, so this is usually gone before it
            // is read — but `x` is offered here too, because a plan task that ends without a frame
            // would otherwise leave no way out of this state (review finding, MOD-20 T8).
            InstallState::Planning { .. } => HINT_RUNNING,
            InstallState::Pending { .. } => HINT_PENDING,
            InstallState::Running { .. } => HINT_RUNNING,
            InstallState::Manual { .. } => HINT_MANUAL,
        };
        match &self.notice {
            Some(notice) => format!("{keys} \u{b7} {notice}"),
            None => keys.to_owned(),
        }
    }

    /// The seven-column table, with the cursor row accented.
    fn render_table(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let rows = self.agents.iter().map(|summary| {
            let agent = &summary.agent;
            let on_box = self.on_box_cell(summary);
            let style = if agent.enabled {
                ctx.theme.base
            } else {
                ctx.theme.dim
            };
            Row::new(vec![
                Cell::from(Line::styled(agent.name.clone(), style)),
                Cell::from(Line::styled(agent.transport.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.billing.as_str(), ctx.theme.dim)),
                Cell::from(Line::styled(agent.models.len().to_string(), ctx.theme.dim)),
                Cell::from(Line::styled(
                    agent
                        .default_model
                        .clone()
                        .unwrap_or_else(|| NONE.to_owned()),
                    ctx.theme.dim,
                )),
                Cell::from(Line::styled(
                    if agent.enabled { "yes" } else { "no" },
                    style,
                )),
                Cell::from(Line::styled(on_box, ctx.theme.dim)),
            ])
        });

        let header = Row::new(vec![
            Cell::from("name"),
            Cell::from("transport"),
            Cell::from("billing"),
            Cell::from("models"),
            Cell::from("default"),
            Cell::from("enabled"),
            Cell::from("on this box"),
        ])
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(9),
                Constraint::Length(7),
                Constraint::Min(11),
            ],
        )
        .header(header)
        .row_highlight_style(ctx.theme.accent);

        // The cursor is cloned because `render` takes `&self`: ratatui writes the scroll offset it
        // computed back into the state, and a view that mutated during a draw would be a view
        // whose frame depends on how often it was drawn.
        frame.render_stateful_widget(table, area, &mut self.cursor.clone());
    }
}

impl SettingsSection for AgentsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Agents"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `agent` is global, so the read does not change with the workspace.
        vec![StoreRequest::Agents]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A plan on screen answers first, and answers everything: MOD-20 D19's modality.
        if matches!(self.install, InstallState::Pending { .. }) {
            return self.answer_consent(key, ctx);
        }
        // `i`, `j`, `k`, `x` and `r` are free: the global table binds `q`, `?`, the digits,
        // `ctrl-c` and `-`, and the tab itself consumes `h`/`l`/`[`/`]`/arrows before a section is
        // offered the key.
        match key.code {
            KeyCode::Char('j') => {
                self.move_cursor(true);
                Handled::Consumed
            }
            KeyCode::Char('k') => {
                self.move_cursor(false);
                Handled::Consumed
            }
            KeyCode::Char('i') => {
                self.begin_install(ctx);
                Handled::Consumed
            }
            // `x` cancels a running install **and** a pre-flight. Planning is one `GET` and one
            // `HEAD`, so it is usually over before a key lands — but if the plan task ends without
            // a frame (a panic inside it is caught by `tokio::spawn` and swept by `is_finished`),
            // `Planning` would otherwise be a state with no way out but a restart. The runtime
            // serves a cancel during planning and answers `Failed { "install_cancel" }` for a task
            // already swept, and both land on `Idle`.
            KeyCode::Char('x')
                if matches!(
                    self.install,
                    InstallState::Running { .. } | InstallState::Planning { .. }
                ) =>
            {
                if let InstallState::Running { cancelling, .. } = &mut self.install {
                    *cancelling = true;
                }
                ctx.request(StoreRequest::InstallCancel);
                Handled::Consumed
            }
            KeyCode::Esc if matches!(self.install, InstallState::Manual { .. }) => {
                self.install = InstallState::Idle;
                Handled::Consumed
            }
            // Before the two `r` arms below: an install is about to spawn a process of its own,
            // and a probe that spawned one per agent beside it would be racing it for the same
            // `agent_box` row.
            KeyCode::Char('r') if self.install_in_flight() => {
                ctx.emit(Action::Error(
                    "an install is running; probe afterwards".to_owned(),
                ));
                Handled::Consumed
            }
            KeyCode::Char('r') if !self.probing => {
                self.probing = true;
                ctx.request(StoreRequest::ProbeAgents);
                Handled::Consumed
            }
            KeyCode::Char('r') => {
                ctx.emit(Action::Error("a probe is already running".to_owned()));
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            // Hazard H-17: `agents`, `unavailable`, `probing` and the cursor that indexes them.
            // Not `install` — this reply arrives from a scope change as readily as from the read
            // the section asked for, and a download is not over because the workspace changed.
            StoreReply::Agents(agents) => {
                self.agents = agents.clone();
                self.unavailable = None;
                self.probing = false;
                self.clamp_cursor();
            }
            StoreReply::Install(frame) => self.on_install_frame(frame, ctx),
            // The shell has already put the message on the status line (`App::update`), so the
            // section's whole job here is to stop saying a probe is running.
            StoreReply::Failed { request, .. } if *request == "probe_agents" => {
                self.probing = false;
            }
            // The same, for the three install requests: a refusal is one sentence the shell owns,
            // and all this section owes is a state a second `i` can start from.
            StoreReply::Failed { request, .. }
                if matches!(
                    *request,
                    "install_plan" | "install_confirm" | "install_cancel"
                ) =>
            {
                self.install = InstallState::Idle;
            }
            // `agent` is mirrored since MOD-2 milestone 4 (plan D31), so an offline backend
            // answers this read from the mirror; `agent_box` is not, which is why every offline
            // row's `on this box` column reads `not probed`. A refusal is therefore a store that
            // can reach neither the server nor a mirror, and saying so beats rendering an empty
            // table that reads as "no agents registered".
            StoreReply::Failed { request, message } if *request == "agents" => {
                self.agents.clear();
                self.clamp_cursor();
                self.unavailable = Some(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(ctx.theme);
        let [rows, consent, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        if self.unavailable.is_some() {
            message(frame, rows, "agent registry needs Postgres", ctx.theme);
        } else if self.agents.is_empty() {
            message(frame, rows, "no agents registered", ctx.theme);
        } else {
            self.render_table(frame, rows, ctx);
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), consent);
        }
        frame.render_widget(
            Paragraph::new(Line::styled(self.hint(), ctx.theme.dim)),
            hint,
        );
    }
}

/// Whether a row's launch document declares how to install it (MOD-20 D12).
///
/// Parsed here rather than asked of the store: the same document the table is drawn from carries
/// the answer, and `R-AGT-10` is about the user being told before anything is fetched — including
/// being told that there is nothing to fetch.
fn declares_a_source(launch: &Value) -> bool {
    serde_json::from_value::<AgentLaunch>(launch.clone())
        .ok()
        .and_then(|launch| launch.discovery)
        .is_some_and(|discovery| discovery.install.is_some())
}

/// The progress cell of one frame (MOD-20 D19).
///
/// A zero numerator renders the phase word alone rather than `0%`. That is not cosmetic: `unpack`
/// counts *completed entries*, so a single-entry archive — which several registry entries are —
/// reports `done = 0` for the whole unpack and then jumps straight to `done == total`. A
/// percentage computed from that reads `0%` for the entire phase and then vanishes, which says
/// less than the phase word does and says it wrongly.
fn phase_cell(phase: InstallPhase, done: u64, total: Option<u64>) -> String {
    if done == 0 {
        return format!("{phase}\u{2026}");
    }
    match total {
        Some(total) if total > 0 => format!("{phase} {}%", done.saturating_mul(100) / total),
        // No denominator: what is known is how much has arrived, so that is what is said.
        _ => format!("{phase} {}", human_bytes(done)),
    }
}

/// The one line a finished install leaves on the hint row.
///
/// Never "installed" on its own for a failure: `R-AGT-6` says the probe decides, and D16's two
/// failure shapes differ in the one thing the user has to act on — whether the box was left with a
/// working version or with the tree that did not work.
fn outcome_notice(outcome: &InstallOutcome) -> String {
    match outcome {
        InstallOutcome::Installed { dir, version, .. } => {
            format!("installed {} {version}", entry_id(dir))
        }
        InstallOutcome::Failed {
            version,
            status,
            stderr_tail,
            restored: Some(previous),
            ..
        } => format!(
            "install of {version} failed: {}; {previous} restored",
            first_line(stderr_tail.as_deref(), *status)
        ),
        InstallOutcome::Failed {
            version,
            status,
            stderr_tail,
            ..
        } => format!(
            "install of {version} failed: {}; the tree is left in place",
            first_line(stderr_tail.as_deref(), *status)
        ),
    }
}

/// The registry entry id an install directory belongs to: `<root>/<id>/<version>`.
fn entry_id(dir: &Path) -> String {
    dir.parent()
        .and_then(Path::file_name)
        .map_or_else(String::new, |id| id.to_string_lossy().into_owned())
}

/// The first line of a probe's failure text, or the status when it produced none.
fn first_line(tail: Option<&[String]>, status: htui_agent::probe::ProbeStatus) -> String {
    tail.and_then(<[String]>::first)
        .cloned()
        .unwrap_or_else(|| status.to_string())
}

/// A byte count as a progress cell says it.
fn human_bytes(bytes: u64) -> String {
    /// One kibibyte.
    const KIB: u64 = 1024;
    /// One mebibyte.
    const MIB: u64 = 1024 * KIB;
    /// One gibibyte.
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}
