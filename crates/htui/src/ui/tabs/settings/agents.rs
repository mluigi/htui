//! The agent registry section of the Settings tab (`R-TUI-8`, MOD-2 D14, D54, MOD-20 D19, D20,
//! MOD-21 D20).
//!
//! It lists what `agent` and this box's `agent_box` hold, and it is where a probe is asked for:
//! `r` sends [`StoreRequest::ProbeAgents`] and the `on this box` column then says what this box
//! can actually run (`R-AGT-6`). The caps banner is milestone 3's; the `quota` column arrived with
//! milestone 7 (MOD-2 D73) and reads `agent_box.quota` by key — each is a column in this table and
//! a field in the reply, not a rewrite of the section.
//!
//! What `r` does **not** do is refresh that column. Quota is latched from the usage a run reports,
//! never polled: a probe handshake reports no allowance at all, so a re-probe moves every other
//! column and this one on no row. `docs/ANA-4.md` §7 asks for that limit to be stated rather than
//! implied, and [`QUOTA_NOTE`] on the hint line is where this section states it.
//!
//! Since MOD-20 it is also where an adapter is **installed**: `i` on the highlighted row asks for
//! a pre-flight, the plan that comes back is drawn as a consent pane under the table, and only `y`
//! fetches a byte. The pane is in the section rather than in an overlay (MOD-20 D19) because an
//! overlay factory takes no argument and so cannot be told which row, and because replies to a
//! popped overlay are dropped — the section is the one view that outlives a whole install.
//!
//! Since MOD-21 it is also where an agent is **logged in**: `a` on the highlighted row starts one
//! flow, the method list the adapter's own `initialize` answered is drawn as a chooser under the
//! table, and what the flow then prints to its stderr — the link included — is drawn there too
//! (MOD-21 D20). The verdict at the end is the **probe's**: `Done` clears the pane and re-reads the
//! registry, and there is no state in this file that means "logged in" (`R-AGT-6`).
//!
//! Two costs of answering the probe with the reply this section already reads. One: **any**
//! [`StoreReply::Agents`] clears the in-flight *probe* state, so a re-activation or a scope change
//! while a probe is running puts the pre-probe rows back for a moment. The probe's own reply
//! supersedes them when it lands, and the alternative was a second reply variant nothing else
//! would ever use. Two: neither the install state nor the login state joins it (hazard H-17, and
//! MOD-21's H-7) — a probe that loses its flag re-renders one column, an install that lost its
//! state would leave a running download with nothing on screen to cancel it with, and a login that
//! lost its state would leave a spawned adapter, an open loopback listener and a human half-way
//! through a browser page with no key on screen to stop any of it.

use chrono::{DateTime, Utc};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::PlanError;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_agent::{AgentLaunch, InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{AgentId, AgentSummary, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;

use crate::agent_worker::AUTH_ALREADY_CHOSEN;
use crate::app::{Action, Ctx, Handled};
use crate::store_worker::{AuthFrame, InstallFrame, StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::{KeyCode, KeyEvent};

/// What the `on this box` column reads while this box has no `agent_box` row for the agent.
const NOT_PROBED: &str = "not probed";

/// What the `on this box` column reads while a probe this section asked for is in flight.
const PROBING: &str = "probing\u{2026}";

/// What the `on this box` column reads between `x` and the install's own last frame.
const CANCELLING: &str = "cancelling\u{2026}";

/// What replaces an absent `default_model`, and a quota this build has nothing to say about.
const NONE: &str = "\u{2014}";

/// The hint line with nothing in flight. It stands in for a help entry: a Settings section has no
/// [`KeyScope`](crate::keymap::KeyScope) of its own (MOD-20 D19), so the keys are written where
/// they are pressed.
const HINT_IDLE: &str = "j/k select \u{b7} r probe \u{b7} i install \u{b7} a authenticate";

/// What the idle line adds when it has the room: the one limit of this section that is not a key
/// (MOD-2 D73).
///
/// `r` re-probes every row and moves the `quota` column on none of them, because a probe handshake
/// reports no allowance and the value is latched from what a chat reports instead. `docs/ANA-4.md`
/// §7 asks for that to be "stated in the UI rather than implied", and this is the statement.
///
/// *When it has the room*, because the hint row is one line of a 100-column frame: a notice is the
/// answer to what the user just pressed, and a standing sentence that clipped a failure's own words
/// off the right edge would be the wrong half of the line to keep. It is on the line the section
/// **rests** in, which is the line a limit is read from.
const QUOTA_NOTE: &str = "quota latches per chat, r cannot refresh it";

/// The hint line while a plan waits for an answer.
const HINT_PENDING: &str = "y install \u{b7} n cancel";

/// The hint line while an install streams.
const HINT_RUNNING: &str = "x cancel install";

/// The hint line while the manual steps are up.
const HINT_MANUAL: &str = "Esc close";

/// The hint line while the login chooser is waiting for a method (MOD-21 D20).
const HINT_CHOOSING: &str = "j/k choose \u{b7} Enter select \u{b7} Esc cancel";

/// The hint line while a login is spawning or running.
const HINT_AUTH_RUNNING: &str = "o open link \u{b7} x cancel";

/// What the `on this box` column reads between `a` and the flow's own method list.
const STARTING: &str = "starting\u{2026}";

/// What it reads while the chooser is up.
const CHOOSE: &str = "choose a method";

/// What it reads while an `authenticate` call is in flight.
const LOGGING_IN: &str = "logging in\u{2026}";

/// What it reads while a `logout` call is in flight.
const LOGGING_OUT: &str = "logging out\u{2026}";

/// The chooser's last row when the agent advertised a logout verb.
const LOGOUT_ROW: &str = "log out";

/// What `o` says before the adapter has printed a link.
const NO_LINK: &str = "no link yet";

/// What the section says of a login that was stopped.
const AUTH_CANCELLED: &str = "login cancelled";

/// What the section says of an opener that was spawned. Never "the browser opened", which `htui`
/// does not own and cannot know (MOD-21 D17).
const OPENED: &str = "link opened";

/// MOD-21 D20: how many of the adapter's stderr lines the stream pane keeps.
///
/// Six, because the pane is drawn under a table that has to stay readable on a 24-row terminal and
/// a live flow's useful stderr is one line — the link — with a handful of noise around it.
const AUTH_PANE_LINES: usize = 6;

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

/// How far a login this section asked for has got (MOD-21 D20).
///
/// [`InstallState`]'s shape and its reason: the runtime allows one login at a time (D19), the
/// states are exclusive by construction, and every key the section binds means something different
/// in each of them. What is *not* here is a "logged in" state — the flow does not decide that, the
/// probe does (`R-AGT-6`), and the cell reads the row the flow's own re-probe wrote.
#[derive(Debug, Default)]
enum AuthState {
    /// No login is running.
    #[default]
    Idle,
    /// `a` was pressed and the agent has not answered `initialize` yet.
    Starting {
        /// The row it was pressed on.
        agent_id: AgentId,
    },
    /// The agent's own method list is on screen, waiting for `Enter`.
    ///
    /// Held whole rather than summarised, and taken from the live frame rather than from the
    /// stored snapshot (MOD-21 D7): the snapshot decides whether `a` is *offered*, the flow
    /// decides what is offered *in* it.
    Choosing {
        /// Whose row the pane belongs to.
        agent_id: AgentId,
        /// Every method the agent advertised, in its own order.
        methods: Vec<AuthMethodInfo>,
        /// The agent advertised a logout verb, so the last row is one.
        logout: bool,
        /// How many `terminal`-typed methods were withheld (MOD-21 D4).
        hidden: usize,
        /// Which row `Enter` sends.
        cursor: usize,
    },
    /// A call is in flight and its stream is on screen.
    Running {
        /// Whose row the progress cell belongs to.
        agent_id: AgentId,
        /// Which call, so the cell can say `logging in…` or `logging out…`.
        call: AuthCall,
        /// The last [`AUTH_PANE_LINES`] stderr lines, oldest first.
        lines: VecDeque<String>,
        /// The newest link the adapter printed, which is the one `o` opens.
        url: Option<String>,
        /// `x` was pressed, or the runtime acknowledged one.
        cancelling: bool,
    },
}

/// The registry as a table of name, transport, billing, models, default, enabled and per-box
/// state, with a row cursor, the install one row at a time and the login one row at a time.
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
    /// and it is what scrolls the table once there are more agents than rows on screen.
    cursor: TableState,
    /// The install this section is waiting on.
    install: InstallState,
    /// The login this section is waiting on (MOD-21 D20).
    auth: AuthState,
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

    /// Whether a login this section asked for holds the runtime's claim (MOD-21 D19).
    ///
    /// Every state but [`AuthState::Idle`] counts, including the one that is only waiting for the
    /// user to choose: the adapter is already spawned by then, and the claim covers it.
    fn auth_in_flight(&self) -> bool {
        !matches!(self.auth, AuthState::Idle)
    }

    /// Which row the running login belongs to, or `None` when none is running.
    fn auth_agent(&self) -> Option<AgentId> {
        match &self.auth {
            AuthState::Idle => None,
            AuthState::Starting { agent_id }
            | AuthState::Choosing { agent_id, .. }
            | AuthState::Running { agent_id, .. } => Some(*agent_id),
        }
    }

    /// `a`: log the highlighted row in, or say why not (MOD-21 D20, `R-AGT-9`).
    ///
    /// [`begin_install`](Self::begin_install)'s rule, for the same reason: every refusal is a
    /// sentence with the row's own name in it and every one of them is reached from the documents
    /// this section is already holding, before a request is spent. The runtime refuses the same
    /// cases in the same order (`agent_worker::auth_start`) — but a login that reached it would
    /// have cost a spawn, a handshake and a human's attention to be told what the table on screen
    /// already says.
    fn begin_auth(&mut self, ctx: &mut Ctx<'_>) {
        if self.auth_in_flight() {
            ctx.emit(Action::Error("a login is already running".to_owned()));
            return;
        }
        if self.install_in_flight() {
            ctx.emit(Action::Error("an install is running".to_owned()));
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
        // MOD-21 D10's predicate: a transport with no `authenticate` call has none for any row,
        // and the seam's own sentence is the one to print.
        if !caps_for(&summary.agent).authenticate {
            ctx.emit(Action::Error(format!(
                "`{}` is a `{}` agent; it has no `authenticate` call",
                summary.agent.name,
                summary.agent.transport.as_str()
            )));
            return;
        }
        // MOD-21 D7's stated cost: the *snapshot* decides whether a login is offered at all. A box
        // nobody has probed, and a box whose probe could not run the adapter, are the same answer —
        // a spawn would only say again what the last one said.
        let snapshot = summary.on_box.as_ref().and_then(ProbeSnapshot::from_row);
        let Some(snapshot) = snapshot.filter(|snapshot| {
            matches!(
                snapshot.status,
                ProbeStatus::Unauthenticated | ProbeStatus::Ready
            )
        }) else {
            ctx.emit(Action::Error(format!(
                "probe `{}` first",
                summary.agent.name
            )));
            return;
        };
        if snapshot
            .handshake
            .is_none_or(|handshake| handshake.auth_methods.is_empty())
        {
            ctx.emit(Action::Error(format!(
                "`{}` advertises no authentication methods",
                summary.agent.name
            )));
            return;
        }
        let agent_id = summary.agent.id;
        self.auth = AuthState::Starting { agent_id };
        self.notice = None;
        ctx.request(StoreRequest::AuthStart { agent_id });
    }

    /// The keys the method chooser answers, and the ones it swallows (MOD-21 D20).
    ///
    /// [`answer_consent`](Self::answer_consent)'s modality and its warning: **only this section's
    /// own keys** are consumed. `App::on_key` offers the active tab a key before the `Tab` and
    /// `Global` keymaps, so a blanket `_ => Consumed` here would make `q`, `?`, `Tab` and the digit
    /// tab-switches dead for as long as a human is reading a method list — which is why the choice
    /// is `Enter` and not a digit in the first place.
    fn answer_chooser(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('j') => {
                self.move_choice(true);
                Handled::Consumed
            }
            KeyCode::Char('k') => {
                self.move_choice(false);
                Handled::Consumed
            }
            KeyCode::Enter => {
                self.send_choice(ctx);
                Handled::Consumed
            }
            // Not "close the pane": the adapter is spawned and waiting on this answer, and only the
            // runtime can kill it. The cell reads `cancelling…` until the flow's own last frame.
            KeyCode::Char('n') | KeyCode::Esc => {
                self.begin_auth_cancel(ctx);
                Handled::Consumed
            }
            KeyCode::Char('r' | 'i') => {
                self.refuse_during_login(key, ctx);
                Handled::Consumed
            }
            // The section's own keys, swallowed so a cursor move cannot change the row under a
            // choice the user has not made yet.
            KeyCode::Char('a' | 'x') => Handled::Consumed,
            _ => Handled::Pass,
        }
    }

    /// Moves the chooser's cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for [`move_cursor`](Self::move_cursor)'s reason: `Enter` sends whatever the cursor
    /// is on, and a held `j` that wrapped would send a call the user never aimed at.
    fn move_choice(&mut self, down: bool) {
        if let AuthState::Choosing {
            methods,
            logout,
            cursor,
            ..
        } = &mut self.auth
        {
            let last = methods.len().saturating_add(usize::from(*logout)).max(1) - 1;
            *cursor = if down {
                cursor.saturating_add(1).min(last)
            } else {
                cursor.saturating_sub(1)
            };
        }
    }

    /// `Enter`: send the row the cursor is on and move to the stream pane.
    fn send_choice(&mut self, ctx: &mut Ctx<'_>) {
        let AuthState::Choosing {
            agent_id,
            methods,
            logout,
            cursor,
            ..
        } = &self.auth
        else {
            return;
        };
        let choice = match methods.get(*cursor) {
            Some(method) => AuthChoice::Method(method.id.clone()),
            // Past the last method with a logout advertised: the logout row. Without one there is
            // nothing under the cursor and nothing to send.
            None if *logout => AuthChoice::Logout,
            None => return,
        };
        let call = match &choice {
            AuthChoice::Method(id) => AuthCall::Authenticate(id.clone()),
            AuthChoice::Logout => AuthCall::Logout,
        };
        self.auth = AuthState::Running {
            agent_id: *agent_id,
            call,
            lines: VecDeque::new(),
            url: None,
            cancelling: false,
        };
        self.notice = None;
        ctx.request(StoreRequest::AuthChoose { choice });
    }

    /// `x`, `n` and `Esc`: ask the runtime to stop the flow, and say so in the cell.
    ///
    /// A chooser that is cancelled becomes a `Running` with the flag set rather than a state of its
    /// own: what is on screen from here until the flow's own last frame is the same in both, and
    /// one state carrying the flag is one place to clear it.
    fn begin_auth_cancel(&mut self, ctx: &mut Ctx<'_>) {
        match &mut self.auth {
            AuthState::Idle => return,
            AuthState::Running { cancelling, .. } => *cancelling = true,
            AuthState::Starting { agent_id } | AuthState::Choosing { agent_id, .. } => {
                self.auth = AuthState::Running {
                    agent_id: *agent_id,
                    call: AuthCall::Authenticate(String::new()),
                    lines: VecDeque::new(),
                    url: None,
                    cancelling: true,
                };
            }
        }
        ctx.request(StoreRequest::AuthCancel);
    }

    /// `r` and `i` while a login runs: the flow ends by re-probing and writing the row, and either
    /// of the other two would be racing it for that row (MOD-21 D19).
    fn refuse_during_login(&self, key: KeyEvent, ctx: &mut Ctx<'_>) {
        let what = if key.code == KeyCode::Char('r') {
            "probe"
        } else {
            "install"
        };
        ctx.emit(Action::Error(format!(
            "a login is running; {what} afterwards"
        )));
    }

    /// `o`: open the link the pane is showing, through `htui`'s own opener (MOD-21 D17).
    fn open_link(&self, ctx: &mut Ctx<'_>) {
        match &self.auth {
            AuthState::Running { url: Some(url), .. } => {
                ctx.request(StoreRequest::AuthOpen { url: url.clone() });
            }
            _ => ctx.emit(Action::Error(NO_LINK.to_owned())),
        }
    }

    /// One frame of a login stream (MOD-21 D18, D20).
    ///
    /// Every terminal frame lands on [`AuthState::Idle`] with a notice, and exactly one of them —
    /// [`AuthFrame::Done`] — re-reads the registry: the flow wrote the row through its own
    /// re-probe, and what the cell then says is that row (`R-AGT-6`). There is no state here that
    /// means "logged in".
    fn on_auth_frame(&mut self, frame: &AuthFrame, ctx: &mut Ctx<'_>) {
        match frame {
            AuthFrame::Methods {
                methods,
                logout,
                hidden,
            } => {
                // The list belongs to the flow that is running; a frame with no flow behind it is
                // a stale one the shell's own freshness check let through (blueprint H-26).
                //
                // And to a flow that is still *starting*: `begin_auth_cancel` from `Starting`
                // synthesises a `Running { cancelling: true }` at the same `AuthStart` address, so
                // a list already in flight when `x` was pressed would otherwise replace it with a
                // chooser and lose the flag until `Cancelled` arrived (review L-3).
                if let Some(agent_id) = self.auth_agent()
                    && matches!(self.auth, AuthState::Starting { .. })
                {
                    self.auth = AuthState::Choosing {
                        agent_id,
                        methods: methods.clone(),
                        logout: *logout,
                        hidden: hidden.len(),
                        cursor: 0,
                    };
                    self.notice = None;
                }
            }
            AuthFrame::Line(line) => {
                if let AuthState::Running { lines, .. } = &mut self.auth {
                    lines.push_back(line.clone());
                    while lines.len() > AUTH_PANE_LINES {
                        lines.pop_front();
                    }
                }
            }
            AuthFrame::Url(url) => {
                if let AuthState::Running { url: shown, .. } = &mut self.auth {
                    *shown = Some(url.clone());
                }
            }
            // The opener was spawned. Not the end of anything: the flow is still waiting on the
            // human this link was for.
            AuthFrame::Opened => self.notice = Some(OPENED.to_owned()),
            AuthFrame::Done { call, status } => {
                let what = match call {
                    AuthCall::Authenticate(_) => "logged in",
                    AuthCall::Logout => "logged out",
                };
                self.auth = AuthState::Idle;
                self.notice = Some(format!("{what}: {status}"));
                ctx.request(StoreRequest::Agents);
            }
            // The agent's own sentence, verbatim (MOD-21 D5): the live refusal names the variable
            // the user has to set, and nothing this section could write would say it better.
            AuthFrame::Refused { message } => {
                self.auth = AuthState::Idle;
                self.notice = Some(message.clone());
            }
            AuthFrame::Cancelling => {
                if let AuthState::Running { cancelling, .. } = &mut self.auth {
                    *cancelling = true;
                }
            }
            AuthFrame::Cancelled => {
                self.auth = AuthState::Idle;
                self.notice = Some(AUTH_CANCELLED.to_owned());
            }
            // Its own frame rather than a `Cancelled`, so the notice says what happened instead of
            // implying the user did it (MOD-21 D13).
            AuthFrame::Idle { after } => {
                self.auth = AuthState::Idle;
                self.notice = Some(format!("no activity for {after:?}; login cancelled"));
            }
            AuthFrame::Failed { message } => {
                self.auth = AuthState::Idle;
                self.notice = Some(message.clone());
            }
        }
    }

    /// The login cell of the row a flow is running on, or `None` for every other row.
    fn auth_cell(&self, agent_id: AgentId) -> Option<String> {
        match &self.auth {
            AuthState::Starting { agent_id: running } if *running == agent_id => {
                Some(STARTING.to_owned())
            }
            AuthState::Choosing {
                agent_id: running, ..
            } if *running == agent_id => Some(CHOOSE.to_owned()),
            AuthState::Running {
                agent_id: running,
                cancelling: true,
                ..
            } if *running == agent_id => Some(CANCELLING.to_owned()),
            AuthState::Running {
                agent_id: running,
                call,
                ..
            } if *running == agent_id => Some(
                match call {
                    AuthCall::Authenticate(_) => LOGGING_IN,
                    AuthCall::Logout => LOGGING_OUT,
                }
                .to_owned(),
            ),
            _ => None,
        }
    }

    /// The login half of the pane: the chooser, or the stream (MOD-21 D20).
    fn auth_pane(&self, theme: &Theme) -> Vec<Line<'static>> {
        match &self.auth {
            AuthState::Choosing {
                methods,
                logout,
                hidden,
                cursor,
                ..
            } => {
                let style = |row: usize| {
                    if row == *cursor {
                        theme.accent
                    } else {
                        theme.base
                    }
                };
                let mut lines = vec![Line::default()];
                // The agent's own words, in the agent's own order. The **id** is what `Enter`
                // sends and is deliberately never drawn: it is a wire value, not a label.
                for (row, method) in methods.iter().enumerate() {
                    let text = match &method.description {
                        Some(description) => format!("{} \u{2014} {description}", method.name),
                        None => method.name.clone(),
                    };
                    lines.push(Line::styled(text, style(row)));
                }
                if *logout {
                    lines.push(Line::styled(LOGOUT_ROW, style(methods.len())));
                }
                // Named rather than offered (MOD-21 D4, D21): the spec forbids handing a
                // `terminal` method to `authenticate`, and a user who cannot see the one they were
                // told to use would otherwise think the list was broken.
                if *hidden > 0 {
                    lines.push(Line::styled(
                        format!("{hidden} method(s) need a terminal htui does not provide"),
                        theme.dim,
                    ));
                }
                lines
            }
            AuthState::Running { lines, url, .. } => lines
                .iter()
                .map(|line| Line::styled(line.clone(), theme.dim))
                .chain(
                    url.iter()
                        .map(|url| Line::styled(format!("link: {url}"), theme.base)),
                )
                .collect(),
            AuthState::Idle | AuthState::Starting { .. } => Vec::new(),
        }
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
        // Before the probe check and for the probe check's reason: a login ends by re-probing and
        // writing the same `agent_box` row, and an install beside it would race it for that row
        // (MOD-21 D19).
        if self.auth_in_flight() {
            ctx.emit(Action::Error(
                "a login is running; install afterwards".to_owned(),
            ));
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
        // Beside the install and for its reason: a login this section is running is the one thing
        // on screen the user is waiting on, and it outlasts every other answer here.
        if let Some(cell) = self.auth_cell(summary.agent.id) {
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
    /// An install pane wins over a login one: the two cannot be *in flight* together (each refuses
    /// while the other is), and the one state that can outlive its action — `Manual` — is a failure
    /// the user still has to read and dismiss.
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
            _ => self.auth_pane(theme),
        }
    }

    /// The one line under the pane: which keys mean something here, the one limit that is not a
    /// key, and the last outcome.
    ///
    /// Keys and note separately because only one state has anything to say beyond its keys, and
    /// because all three want the same room: a notice wins it, [`QUOTA_NOTE`] takes it when there
    /// is no notice, and every other state has only keys to write there.
    fn hint(&self) -> String {
        let (keys, note) = match &self.install {
            // With no install in flight the login owns the line, because it is the only other
            // thing here that binds keys of its own.
            InstallState::Idle => match &self.auth {
                AuthState::Idle => (HINT_IDLE, Some(QUOTA_NOTE)),
                AuthState::Choosing { .. } => (HINT_CHOOSING, None),
                AuthState::Starting { .. } | AuthState::Running { .. } => (HINT_AUTH_RUNNING, None),
            },
            // A pre-flight is one registry read and one `HEAD`, so this is usually gone before it
            // is read — but `x` is offered here too, because a plan task that ends without a frame
            // would otherwise leave no way out of this state (review finding, MOD-20 T8).
            InstallState::Planning { .. } => (HINT_RUNNING, None),
            InstallState::Pending { .. } => (HINT_PENDING, None),
            InstallState::Running { .. } => (HINT_RUNNING, None),
            InstallState::Manual { .. } => (HINT_MANUAL, None),
        };
        match (&self.notice, note) {
            (Some(notice), _) => format!("{keys} \u{b7} {notice}"),
            (None, Some(note)) => format!("{keys} \u{b7} {note}"),
            (None, None) => keys.to_owned(),
        }
    }

    /// The eight-column table, with the cursor row accented.
    fn render_table(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let rows = self.agents.iter().map(|summary| {
            let agent = &summary.agent;
            let quota = quota_cell(summary);
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
                Cell::from(Line::styled(quota, ctx.theme.dim)),
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
            Cell::from("quota"),
            Cell::from("on this box"),
        ])
        .style(ctx.theme.title);

        // MOD-2 D76's packing. The eight columns want ~107 characters and the bordered section
        // draws in 98 (hazard H-12), so the width does not divide and a **ranking** decides it
        // rather than arithmetic. The maintainer's order, highest first: the whole `default_model`
        // string; then the quota window's reset time; then `on this box`'s slack.
        //
        // So `default` is 21 — `gemini-3.7-flash-high`, the longest id the seeds carry — because
        // that id is the coordinate `docs/ANA-4.md` §4.4 selects a model by, and `gemini-3.` under
        // the old `Length(9)` named nothing at all. The 12 characters it needed were bought as
        // follows, and the last of the three donors was found only after the first two had left
        // `on this box` too narrow for its own longest verdict:
        //
        // - `quota` gave 6, which is exactly the reset's ` %H:%M` (19 → 13): `62% to 09-08 08:00`
        //   now reads `62% to 09-08`. [`reset_of`] drops the time by *format* rather than by clip,
        //   so `100% to 09-08` — the exhausted window this column exists to warn about — still
        //   fits whole in 13.
        // - `on this box` gave 6 of slack, and that was 4 too many. `Min(11)` is its header, but
        //   its *cells* are longer: `unauthenticated` is 15, and 15 is the width this column has to
        //   have because that word is MOD-21's whole login flow written into one cell — `a` is
        //   offered on an `unauthenticated` row, and `unauthentic` is not a shorter way of saying
        //   so. `choose a method` is 15 for the same reason.
        // - `name` gave the missing 4 (12 → 8), which is why it is the third donor rather than an
        //   untouched column. It is the **only** one of the seven whose content is shorter than its
        //   allowance, so it is the only one that can give width up without giving anything
        //   observable up: its header is `name` (4) and every registry name in the tree fits in 8 —
        //   `claude` (6), `agy` (3), `kappa` (5), and `amp-acp` (7), the longest id MOD-20's live
        //   install proof carries. The other four are each already at their own longest string:
        //   `transport` (9), `models` (6) and `enabled` (7) at their headers, `billing` (12) at
        //   `subscription`.
        //
        // That leaves 8 + 9 + 12 + 6 + 21 + 7 + 13 = 76 fixed, plus 7 single-space gaps of
        // `column_spacing`, so the `Min` column draws 15 at the bordered 98 and every character a
        // wider terminal adds still goes to it first.
        //
        // What is *still* clipped at 15, and is not new damage: the install progress cells, whose
        // longest is `downloading 12.0 MB` (19). That one exceeded the old 17 as well, so no width
        // was ever spent on it; a clipped byte count is a figure that keeps ticking on the next
        // frame, which is the one thing here that a status word is not.
        //
        // The floor under the whole trade is that **all eight headers stay readable**, and above
        // that floor a column may only donate what its own content does not use. A ninth column
        // (MOD-23's editor, MOD-12's caps section) has to be paid for out of this same 98, and
        // there is no slack of the `name` kind left to pay with — so it costs a ranking decision
        // like D76's, not an adjustment.
        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(21),
                Constraint::Length(7),
                Constraint::Length(13),
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
        // Then a method list, for the same reason and with the same limit (MOD-21 D20): the
        // adapter is spawned and waiting on an answer, and `j`/`k` mean the list rather than the
        // table for as long as it is up.
        if matches!(self.auth, AuthState::Choosing { .. }) {
            return self.answer_chooser(key, ctx);
        }
        // `a`, `i`, `j`, `k`, `o`, `x` and `r` are free: the global table binds `q`, `?`, the
        // digits, `ctrl-c` and `-`, and the tab itself consumes `h`/`l`/`[`/`]`/arrows before a
        // section is offered the key.
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
            KeyCode::Char('a') => {
                self.begin_auth(ctx);
                Handled::Consumed
            }
            // `o` is bound only while a flow is running: it is otherwise a free letter, and a
            // section that swallowed it everywhere would be claiming a key it does nothing with.
            KeyCode::Char('o') if self.auth_in_flight() => {
                self.open_link(ctx);
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
            // The same key for the other stream: `x` stops a login wherever it has got to, and the
            // cell reads `cancelling…` until the flow's own last frame says it stopped.
            KeyCode::Char('x') if self.auth_in_flight() => {
                self.begin_auth_cancel(ctx);
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
            // And the same for a login, which ends by re-probing the very row `r` would re-probe.
            KeyCode::Char('r') if self.auth_in_flight() => {
                self.refuse_during_login(key, ctx);
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
            StoreReply::Auth(frame) => self.on_auth_frame(frame, ctx),
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
            // Hazard H-22, the first of the two `Failed`s: a **request** of the flow was refused,
            // so the flow this section thought it had is not there. The shell owns the sentence;
            // all this section owes is a state a second `a` can start from.
            //
            // With one exception, and it is the reverse of the rule rather than a hole in it
            // (review L-4): a second choice is refused *by a login that is still running*, and
            // clearing the pane on that one would leave a live adapter — with its loopback
            // listener open — and no `x` to cancel it. The pane stays exactly where it was, as it
            // does for a refused `auth_open`.
            StoreReply::Failed { request, message }
                if *request == "auth_choose" && message == AUTH_ALREADY_CHOSEN => {}
            StoreReply::Failed { request, .. }
                if matches!(*request, "auth_start" | "auth_choose" | "auth_cancel") =>
            {
                self.auth = AuthState::Idle;
            }
            // And the exception that makes the rule worth writing: a refused `auth_open` is one
            // request answered no, not the end of a login. The pane stays exactly as it was, and
            // the link is still there to try again with.
            StoreReply::Failed { request, .. } if *request == "auth_open" => {}
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

/// The `quota` column of one row (`R-TUI-8`, MOD-2 D73).
///
/// Read off `agent_box.quota` the way [`AgentsSection::on_box_cell`] reads `probe.status`: **by
/// key**, off a [`Value`]. That is what keeps this column free of the driver crate, and it is also
/// `R-AGT-5` — nothing here is keyed on an agent's name, only on what its row says.
///
/// Three answers and a fallback, in the order `docs/ANA-4.md` §7 describes the document: the
/// tightest window as a percentage with its reset when the blob has windows; the session spend
/// when it has only that (a `per_token` row has no allowance to be a fraction of, so what has been
/// spent is the only true thing to say); and [`NONE`] for a box with no `agent_box` row, a row with
/// no blob, or a blob this build does not understand. A vendor shape nobody has written a mapper
/// for is the last of those, and it renders as nothing rather than as a guess.
///
/// Refreshing is deliberately not offered: a probe handshake reports no allowance, so `r` cannot
/// (§7), and [`QUOTA_NOTE`] says so.
fn quota_cell(summary: &AgentSummary) -> String {
    let Some(quota) = summary.on_box.as_ref().and_then(|row| row.quota.as_ref()) else {
        return NONE.to_owned();
    };
    if let Some((utilization, window)) = tightest_window(quota) {
        // `{:.0}` rather than a cast: `0.57 * 100.0` is `56.99999999999999`, so an `as u32` would
        // report a fifty-seven-percent window as `56%`. A rounding format reads it as written.
        let percentage = format!("{:.0}%", utilization * 100.0);
        return match reset_of(window) {
            Some(resets) => format!("{percentage} to {resets}"),
            None => percentage,
        };
    }
    quota
        .get("spend")
        .and_then(|spend| spend.get("session_micros"))
        .and_then(Value::as_i64)
        // The `usage_line` cast precedent (`chat/transcript.rs`): micros are money, and a figure
        // this column has room for is nowhere near `f64`'s exact range.
        .map_or_else(
            || NONE.to_owned(),
            |micros| format!("${:.2} spent", micros as f64 / 1_000_000.0),
        )
}

/// The window a row is closest to the end of: the highest `utilization`, and the lowest `id` of
/// the ones that tie.
///
/// The tie is broken on the `id` rather than on the array's own order so that two sources which
/// serialise the same allowance in a different order render the same cell. A window carrying no
/// numeric `utilization` is not one this column can render, so it is not a candidate at all —
/// which is also why a blob with `"windows": []` falls through to the spend.
fn tightest_window(quota: &Value) -> Option<(f64, &Value)> {
    quota
        .get("windows")?
        .as_array()?
        .iter()
        .filter_map(|window| {
            window
                .get("utilization")
                .and_then(Value::as_f64)
                .map(|utilization| (utilization, window))
        })
        .reduce(|tightest, next| {
            let (utilization, window) = next;
            if utilization > tightest.0
                || (utilization == tightest.0 && window_id(window) < window_id(tightest.1))
            {
                next
            } else {
                tightest
            }
        })
}

/// A window's `id`, or the empty string for one that carries none — enough to break a tie with.
fn window_id(window: &Value) -> &str {
    window.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// A window's `resets_at` as the column says it: `%m-%d`, in UTC.
///
/// The date alone since MOD-2 D76, which ranked the reset's clock time below the whole
/// `default_model` string and above nothing else: the six characters of ` %H:%M` are half of what
/// bought `default` its 21, so `62% to 09-08 08:00` now reads `62% to 09-08`. The year was already
/// left off, and for the same kind of reason — a reset a year out is not what this column warns
/// about.
///
/// Dropped by **format** and not by letting the 13-wide column clip it, because the clip is wrong
/// where it matters most: `100% to 09-08 08:00` cut at 13 reads `100% to 09-0`, a broken date on
/// the exhausted window this column exists for, while the formatted `100% to 09-08` is 13 exactly
/// and fits whole.
///
/// Hazard H-18: this parse runs on the UI task and the string comes from an agent, so
/// [`DateTime::parse_from_rfc3339`] is taken through `ok()`. A malformed one renders the
/// percentage alone, which is still the true half of the answer, and never brings a frame down.
fn reset_of(window: &Value) -> Option<String> {
    let resets_at = window.get("resets_at")?.as_str()?;
    DateTime::parse_from_rfc3339(resets_at)
        .ok()
        .map(|resets_at| resets_at.with_timezone(&Utc).format("%m-%d").to_string())
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
fn first_line(tail: Option<&[String]>, status: ProbeStatus) -> String {
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
