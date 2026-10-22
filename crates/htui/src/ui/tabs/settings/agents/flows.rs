//! The Agents flows (MOD-115 M3 D7): install, login, redirect paste and probe, with the panes
//! and cells they draw under the table and the notices they leave.

use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, NO_LOOPBACK_REDIRECT, PASTE_MAX, RedirectUrl,
};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::PlanError;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_agent::{InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::AgentId;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;

use crate::app::{Action, Ctx, Handled};
use crate::keys::{Act, Keys, views};
use crate::store_worker::{AuthFrame, InstallFrame, StoreRequest};
use crate::ui::kit::form::{Field as FormRow, Form, Value as FormValue};
use crate::ui::kit::label::{self, plural};
use crate::ui::kit::list::{self, GUTTER};
use crate::ui::kit::modal::modal_rest;
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

use super::table::verdict;
use super::{AgentsSection, Mode, swallows};

/// What the `on this box` column reads between `x` and the install's own last frame.
pub(super) const CANCELLING: &str = "cancelling\u{2026}";

/// What the note line says when there is no notice and the section is idle: the one limit of this
/// section that is not a key (MOD-2 D73).
///
/// The probe key re-probes every row and moves the `quota` column on none of them, because a probe handshake
/// reports no allowance and the value is latched from what a chat reports instead. `docs/ANA-4.md`
/// §7 asks for that to be "stated in the UI rather than implied", and this is the statement.
///
/// A notice wins the line: it is the answer to what the user just pressed. Since MOD-23 D245 the
/// note has a line of its own under the keys, so neither clips the other; it is on the line the
/// section **rests** in, which is the line a limit is read from.
///
/// `probe` is `AgentsProbe`'s label through `AGENTS_BROWSE` (MOD-67 M6 D4); with none bound the
/// sentence says a probe in general.
pub(super) fn quota_note(probe: Option<&str>) -> String {
    match probe {
        Some(probe) => format!("quota latches per chat, {probe} cannot refresh it"),
        None => "quota latches per chat; a probe cannot refresh it".to_owned(),
    }
}

/// What `Edited` adds when the save changed what a probe checked (plan D246): the stored verdict is
/// now older than the row, so the next chat re-probes by itself. `probe` is `AgentsProbe`'s label
/// through `AGENTS_BROWSE` (MOD-67 M6 D4), the clause dropped with none bound.
pub(super) fn reprobes(probe: Option<&str>) -> String {
    match probe {
        Some(probe) => format!(" \u{b7} the next chat re-probes; {probe} probes now"),
        None => " \u{b7} the next chat re-probes".to_owned(),
    }
}

/// What the install question swallows rather than passes (MOD-20 D19, MOD-67 M3 L-A Q2): the
/// section's own keys, resolved through `AGENTS_BROWSE` so a rebound one is swallowed under its
/// new chord. Kept out of `AGENTS_CONSENT` so the `?` box lists no dead key under the question.
pub(super) const CONSENT_SWALLOWS: &[Act] = &[
    Act::ListDown,
    Act::ListUp,
    Act::AgentsInstall,
    Act::AgentsProbe,
    Act::AgentsCancel,
];

/// What the login chooser swallows rather than passes (MOD-21 D20, MOD-67 M3 L-A Q2), for
/// [`CONSENT_SWALLOWS`]' reason.
pub(super) const CHOOSER_SWALLOWS: &[Act] = &[Act::AgentsAuthenticate, Act::AgentsCancel];

/// What `p` says while the login is being cancelled (MOD-22 D282).
pub(super) const PASTE_CANCELLING: &str =
    "this login is being cancelled; there is nothing to paste into";

/// What the `on this box` column reads between `a` and the flow's own method list.
pub(super) const STARTING: &str = "starting\u{2026}";

/// What it reads while the chooser is up.
pub(super) const CHOOSE: &str = "choose a method";

/// What it reads while an `authenticate` call is in flight.
pub(super) const LOGGING_IN: &str = "logging in\u{2026}";

/// What it reads while a `logout` call is in flight.
pub(super) const LOGGING_OUT: &str = "logging out\u{2026}";

/// The chooser's last row when the agent advertised a logout verb.
pub(super) const LOGOUT_ROW: &str = "log out";

/// What `o` says before the adapter has printed a link.
pub(super) const NO_LINK: &str = "no link yet";

/// What the section says of a login that was stopped.
pub(super) const AUTH_CANCELLED: &str = "login cancelled";

/// What the section says of an opener that was spawned. Never "the browser opened", which `htui`
/// does not own and cannot know (MOD-21 D17).
pub(super) const OPENED: &str = "link opened";

/// MOD-21 D20: how many of the adapter's stderr lines the stream pane keeps.
///
/// Six, because the pane is drawn under a table that has to stay readable on a 24-row terminal and
/// a live flow's useful stderr is one line — the link — with a handful of noise around it.
pub(super) const AUTH_PANE_LINES: usize = 6;

/// What the section says of an install the user turned down.
pub(super) const DECLINED: &str = "install declined";

/// What the section says of an install that was stopped.
pub(super) const CANCELLED: &str = "install cancelled";

/// How far an install this section asked for has got (MOD-20 D19).
///
/// One state rather than a handful of flags because the states are exclusive by construction: the
/// runtime allows one install at a time (hazard H-10), and every key this section binds means
/// something different in each of them.
#[derive(Debug, Default)]
pub(super) enum InstallState {
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
pub(super) enum AuthState {
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
        /// MOD-22 D264: the newest loopback redirect the adapter's links advertised; `p` is
        /// offered only with one, and a paste is pre-checked against it.
        advertised: Option<Advertised>,
        /// MOD-22 D270, D272: the masked paste field, open between `p` and `Enter`/`Esc`. Dropped,
        /// and so wiped, on submit, on cancel and when the flow ends.
        paste: Option<TextField>,
        /// MOD-22 D270: an `AuthDeliver` is unanswered; a second one would make the first's answer
        /// stale in `App::is_fresh`, so `p` is refused until it lands.
        delivering: bool,
    },
}

impl AgentsSection {
    /// Whether an install this section asked for holds the runtime's claim (hazard H-10).
    ///
    /// Planning counts: the runtime's claim covers the pre-flight as well as the download, so a
    /// second `i` during either is refused here rather than reaching a refusal on the worker.
    pub(super) fn install_in_flight(&self) -> bool {
        matches!(
            self.install,
            InstallState::Planning { .. } | InstallState::Running { .. }
        )
    }

    /// Whether a login this section asked for holds the runtime's claim (MOD-21 D19).
    ///
    /// Every state but [`AuthState::Idle`] counts, including the one that is only waiting for the
    /// user to choose: the adapter is already spawned by then, and the claim covers it.
    pub(super) fn auth_in_flight(&self) -> bool {
        !matches!(self.auth, AuthState::Idle)
    }

    /// Which row the running login belongs to, or `None` when none is running.
    pub(super) fn auth_agent(&self) -> Option<AgentId> {
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
    pub(super) fn begin_auth(&mut self, ctx: &mut Ctx<'_>) {
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
                "`{}` is a {} agent; it has no `authenticate` call",
                summary.agent.name,
                label::transport(summary.agent.transport)
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
    /// `Global` key tables, so a blanket `_ => Consumed` here would make `q`, `?`, `Tab` and the digit
    /// tab-switches dead for as long as a human is reading a method list — which is why the choice
    /// is `Enter` and not a digit in the first place.
    pub(super) fn answer_chooser(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::AGENTS_CHOOSER, chord) {
            match act {
                Act::ListDown => self.move_choice(true),
                Act::ListUp => self.move_choice(false),
                Act::AgentsChoose => self.send_choice(ctx),
                // Not "close the pane": the adapter is spawned and waiting on this answer, and only
                // the runtime can kill it. The cell reads `cancelling…` until the flow's own last
                // frame.
                Act::ConfirmNo => self.begin_auth_cancel(ctx),
                Act::AgentsProbe | Act::AgentsInstall => self.refuse_during_login(act, ctx),
                // A global act: the shell applies it.
                _ => continue,
            }
            return Handled::Consumed;
        }
        // The section's own keys, swallowed so a cursor move cannot change the row under a
        // choice the user has not made yet.
        swallows(ctx.keys(), chord, CHOOSER_SWALLOWS)
    }

    /// Moves the chooser's cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for [`move_cursor`](Self::move_cursor)'s reason: `Enter` sends whatever the cursor
    /// is on, and a held `j` that wrapped would send a call the user never aimed at.
    pub(super) fn move_choice(&mut self, down: bool) {
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
    pub(super) fn send_choice(&mut self, ctx: &mut Ctx<'_>) {
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
            advertised: None,
            paste: None,
            delivering: false,
        };
        self.notice = None;
        ctx.request(StoreRequest::AuthChoose { choice });
    }

    /// `x`, `n` and `Esc`: ask the runtime to stop the flow, and say so in the cell.
    ///
    /// A chooser that is cancelled becomes a `Running` with the flag set rather than a state of its
    /// own: what is on screen from here until the flow's own last frame is the same in both, and
    /// one state carrying the flag is one place to clear it.
    pub(super) fn begin_auth_cancel(&mut self, ctx: &mut Ctx<'_>) {
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
                    advertised: None,
                    paste: None,
                    delivering: false,
                };
            }
        }
        ctx.request(StoreRequest::AuthCancel);
    }

    /// `r` and `i` while a login runs: the flow ends by re-probing and writing the row, and either
    /// of the other two would be racing it for that row (MOD-21 D19).
    pub(super) fn refuse_during_login(&self, act: Act, ctx: &mut Ctx<'_>) {
        let what = if act == Act::AgentsProbe {
            "probe"
        } else {
            "install"
        };
        ctx.emit(Action::Error(format!(
            "a login is running; {what} afterwards"
        )));
    }

    /// `o`: open the link the pane is showing, through `htui`'s own opener (MOD-21 D17).
    pub(super) fn open_link(&self, ctx: &mut Ctx<'_>) {
        match &self.auth {
            AuthState::Running { url: Some(url), .. } => {
                ctx.request(StoreRequest::AuthOpen { url: url.clone() });
            }
            _ => ctx.emit(Action::Error(NO_LINK.to_owned())),
        }
    }

    /// `p`: open the masked paste field, or say by name why there is nothing to paste into
    /// (MOD-22 D270, D282). The first matching row wins: a login on its way out, then a delivery
    /// still unanswered, then a redirect to paste for, else none advertised (which covers
    /// `Starting`).
    ///
    /// The field opens at [`PASTE_MAX`] (D275): every paste `validate` would accept fits its first
    /// allocation, so no prefix of a pasted code is ever freed unwiped on the way.
    pub(super) fn open_paste(&mut self, ctx: &mut Ctx<'_>) {
        let refusal = match &mut self.auth {
            AuthState::Running {
                cancelling: true, ..
            } => PASTE_CANCELLING,
            AuthState::Running {
                delivering: true, ..
            } => DELIVERY_IN_FLIGHT,
            AuthState::Running {
                advertised: Some(_),
                paste,
                ..
            } => {
                *paste = Some(TextField::masked_with_capacity(PASTE_MAX));
                return;
            }
            // Named rather than wildcarded (review L-7), so a new state has to say what `p` does
            // in it. `Idle` and `Choosing` do not reach here (`p` is bound only in flight, and the
            // chooser answers every key first); they are answered rather than assumed.
            AuthState::Running {
                advertised: None, ..
            }
            | AuthState::Starting { .. }
            | AuthState::Choosing { .. }
            | AuthState::Idle => NO_LOOPBACK_REDIRECT,
        };
        ctx.emit(Action::Error(refusal.to_owned()));
    }

    /// One key while the paste field is open: `connection.rs`'s editor shape (MOD-22 D270).
    ///
    /// The field answers first, so the section's letters, the shell's `q`, `?` and digits and the
    /// tab's `h`/`l` are characters of the address; everything it passes on is swallowed, except
    /// what the modal global layer admits (CONTROL, ALT, function keys: MOD-67 D5), so `ctrl-c`
    /// still quits and `F1` opens help.
    ///
    /// `Enter` **moves** the buffer into a [`RedirectUrl`], so there is one copy of what was pasted
    /// and it is wiped when the request that carries it is dropped. The local check is a courtesy
    /// and reads the host and the port only ([`loopback::precheck`], review L-8): the worker runs
    /// the full `validate` and is the authority (MOD-23 D247's rule). What the check refuses is
    /// answered with `validate`'s own sentence and a fresh empty field for the re-paste.
    pub(super) fn on_paste_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let AuthState::Running {
            advertised,
            paste,
            delivering,
            ..
        } = &mut self.auth
        else {
            return Handled::Pass;
        };
        let Some(field) = paste.as_mut() else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            // The field drops, and its buffer is wiped on the way out (D272).
            FieldOutcome::Cancel => {
                *paste = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
            FieldOutcome::Submit => {
                let url = RedirectUrl::new(field.take());
                match advertised
                    .as_ref()
                    .map(|advertised| loopback::precheck(&url, advertised))
                {
                    Some(Ok(())) => {
                        *paste = None;
                        *delivering = true;
                        ctx.request(StoreRequest::AuthDeliver { url });
                    }
                    // The old buffer was moved into `url` and dies with it.
                    Some(Err(refusal)) => {
                        *paste = Some(TextField::masked_with_capacity(PASTE_MAX));
                        ctx.emit(Action::Error(refusal.to_string()));
                    }
                    // Unreachable through `open_paste`, which opens the field only with a
                    // redirect; answered rather than assumed.
                    None => {
                        *paste = None;
                        ctx.emit(Action::Error(NO_LOOPBACK_REDIRECT.to_owned()));
                    }
                }
                Handled::Consumed
            }
        }
    }

    /// One frame of a login stream (MOD-21 D18, D20).
    ///
    /// Every terminal frame lands on [`AuthState::Idle`] with a notice, and exactly one of them —
    /// [`AuthFrame::Done`] — re-reads the registry: the flow wrote the row through its own
    /// re-probe, and what the cell then says is that row (`R-AGT-6`). There is no state here that
    /// means "logged in".
    pub(super) fn on_auth_frame(&mut self, frame: &AuthFrame, ctx: &mut Ctx<'_>) {
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
            // MOD-22 D264: the newest link that advertises a loopback redirect is the one a paste
            // is checked against, as the worker does; a link that advertises none leaves the last
            // one standing.
            AuthFrame::Url(url) => {
                if let AuthState::Running {
                    url: shown,
                    advertised,
                    ..
                } = &mut self.auth
                {
                    *shown = Some(url.clone());
                    if let Some(found) = Advertised::from_auth_url(url) {
                        *advertised = Some(found);
                    }
                }
            }
            // The opener was spawned. Not the end of anything: the flow is still waiting on the
            // human this link was for.
            AuthFrame::Opened => self.notice = Some(OPENED.to_owned()),
            // MOD-22 D268, D270: what the listener said, on the note line, and `p` free again. Not
            // the end of anything: the login's verdict is still the probe's (`R-AGT-6`).
            AuthFrame::Delivered(reply) => {
                if let AuthState::Running { delivering, .. } = &mut self.auth {
                    *delivering = false;
                }
                self.notice = Some(reply.summary());
            }
            AuthFrame::Done { call, status } => {
                let what = match call {
                    AuthCall::Authenticate(_) => "logged in",
                    AuthCall::Logout => "logged out",
                };
                self.auth = AuthState::Idle;
                self.notice = Some(format!("{what}: {}", verdict(*status)));
                ctx.request(StoreRequest::Agents);
            }
            // The agent's own sentence, verbatim (MOD-21 D5): the live refusal names the variable
            // the user has to set, and nothing this section could write would say it better.
            AuthFrame::Refused { message } => {
                self.auth = AuthState::Idle;
                self.notice = Some(message.clone());
            }
            // MOD-22 D282: a login on its way out has nothing to paste into, so an open field
            // closes with the acknowledgement, and its buffer is wiped as it drops.
            AuthFrame::Cancelling => {
                if let AuthState::Running {
                    cancelling, paste, ..
                } = &mut self.auth
                {
                    *cancelling = true;
                    *paste = None;
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
    pub(super) fn auth_cell(&self, agent_id: AgentId) -> Option<String> {
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
    ///
    /// Under the stream's link, at most one line of MOD-22's (D270, D282), the first that applies:
    /// the open paste field (a prompt, then the masked field as a one-row `kit::form`), a delivery
    /// in flight, or the advertised redirect with the key that pastes for it. The prompt and the
    /// field replace the redirect line rather than joining it, so the pane is at most six stderr
    /// lines, the link and two more. Every one of them names [`Advertised::target`] and never a
    /// pasted byte. It takes the width because the field draws a window of its dots.
    ///
    /// Every line sits behind `kit::list`'s gutter (MOD-115 M3), the chooser's cursor row in it
    /// and the band.
    pub(super) fn auth_pane(
        &self,
        width: u16,
        accent: Style,
        theme: &Theme,
        keys: &Keys,
    ) -> Vec<Line<'static>> {
        match &self.auth {
            AuthState::Choosing {
                methods,
                logout,
                hidden,
                cursor,
                ..
            } => {
                let choice = |row: usize, text: String| {
                    let line = Line::styled(text, theme.base);
                    if row == *cursor {
                        chosen(line, width, accent, theme)
                    } else {
                        behind_gutter(line, width)
                    }
                };
                let mut lines = vec![behind_gutter(Line::default(), width)];
                // The agent's own words, in the agent's own order. The **id** is what `Enter`
                // sends and is deliberately never drawn: it is a wire value, not a label.
                for (row, method) in methods.iter().enumerate() {
                    let text = match &method.description {
                        Some(description) => format!("{} \u{2014} {description}", method.name),
                        None => method.name.clone(),
                    };
                    lines.push(choice(row, text));
                }
                if *logout {
                    lines.push(choice(methods.len(), LOGOUT_ROW.to_owned()));
                }
                // Named rather than offered (MOD-21 D4, D21): the spec forbids handing a
                // `terminal` method to `authenticate`, and a user who cannot see the one they were
                // told to use would otherwise think the list was broken.
                if *hidden > 0 {
                    lines.push(behind_gutter(
                        Line::styled(
                            format!(
                                "{} a terminal htui does not provide",
                                plural(*hidden, "method needs", "methods need")
                            ),
                            theme.dim,
                        ),
                        width,
                    ));
                }
                lines
            }
            AuthState::Running {
                lines,
                url,
                advertised,
                paste,
                delivering,
                ..
            } => {
                let mut pane: Vec<Line<'static>> = lines
                    .iter()
                    .map(|line| Line::styled(line.clone(), theme.dim))
                    .chain(
                        url.iter()
                            .map(|url| Line::styled(format!("link: {url}"), theme.base)),
                    )
                    .map(|line| behind_gutter(line, width))
                    .collect();
                if let Some(target) = advertised.as_ref().map(Advertised::target) {
                    if let Some(field) = paste {
                        pane.push(behind_gutter(
                            Line::styled(
                                format!("paste the address the browser could not open ({target}):"),
                                theme.base,
                            ),
                            width,
                        ));
                        pane.extend(
                            Form::new()
                                .field(FormRow::new(PASTE_LABEL, FormValue::Text(field)))
                                .focus(0)
                                .editing(true)
                                .lines(width, accent, theme),
                        );
                    } else if *delivering {
                        pane.push(behind_gutter(
                            Line::styled(format!("delivering to {target}\u{2026}"), theme.dim),
                            width,
                        ));
                    } else {
                        let paste = keys.label(views::AGENTS_BROWSE, Act::AgentsPasteRedirect);
                        let line = match paste {
                            Some(paste) => format!(
                                "redirect: {target} \u{b7} {paste} pastes the address if the \
                                 browser cannot reach it"
                            ),
                            None => format!("redirect: {target}"),
                        };
                        pane.push(behind_gutter(Line::styled(line, theme.dim), width));
                    }
                }
                pane
            }
            AuthState::Idle | AuthState::Starting { .. } => Vec::new(),
        }
    }

    /// `i`: pre-flight the highlighted row, or say why not.
    ///
    /// Every refusal is a sentence with the row's own name in it, and every one of them is reached
    /// without a request: the runtime would refuse the same cases (`agent_worker::install_plan`),
    /// but the answer is already in the document this section is holding, and `R-AGT-10` is about
    /// spending nothing before consent.
    pub(super) fn begin_install(&mut self, ctx: &mut Ctx<'_>) {
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
    pub(super) fn answer_consent(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::AGENTS_CONSENT, chord) {
            match act {
                Act::ConfirmYes => {
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
                }
                Act::ConfirmNo => {
                    self.install = InstallState::Idle;
                    self.notice = Some(DECLINED.to_owned());
                }
                // A global act: the shell applies it.
                _ => continue,
            }
            return Handled::Consumed;
        }
        // Swallowed rather than passed on: below this pane is a table whose cursor decides what
        // `i` installs, and a key that moved it would move what the user is consenting to.
        // **Only this section's own keys**, though. `App::on_key` offers the active tab the key
        // first and returns on `Consumed`, so a blanket `Consumed` here would make `q`, `?`, `Tab`
        // and the digit tab-switches dead for as long as the pane is open — the user could not
        // even quit. The pane is modal over the table beneath it, not over the application.
        swallows(ctx.keys(), chord, CONSENT_SWALLOWS)
    }

    /// One frame of an install stream (MOD-20 D18).
    pub(super) fn on_install_frame(&mut self, frame: &InstallFrame, ctx: &mut Ctx<'_>) {
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

    /// The progress cell of the row an install is running on, or `None` for every other row.
    pub(super) fn install_cell(&self, agent_id: AgentId) -> Option<String> {
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
    ///
    /// An open registry form wins over both (MOD-23 D230): it can only be opened while no install
    /// and no login is in flight, so the one it can cover is a `Manual` pane, which is back the
    /// moment the form closes. It takes the width because its fields draw a window of their text.
    /// The tool-paths form (MOD-66 D10) is drawn the same way and wins for the same reason.
    ///
    /// Every line is `width` cells behind `kit::list`'s gutter (MOD-115 M3), so the pane starts
    /// where the table's rows do; a form's rows are `kit::form`'s, marker in `accent`.
    pub(super) fn pane(
        &self,
        width: u16,
        accent: Style,
        theme: &Theme,
        keys: &Keys,
    ) -> Vec<Line<'static>> {
        let plain = |text: String, style: Style| behind_gutter(Line::styled(text, style), width);
        match &self.mode {
            Mode::Editing(editor) => {
                return core::iter::once(plain(editor.header(), theme.base))
                    .chain(editor.lines(width, accent, theme))
                    .collect();
            }
            Mode::Paths(form) => {
                return core::iter::once(plain(form.header(), theme.base))
                    .chain(form.lines(width, accent, theme))
                    .collect();
            }
            Mode::Browse => {}
        }
        match &self.install {
            InstallState::Pending { plan } => core::iter::once(plain(String::new(), theme.base))
                .chain(
                    plan.consent_lines()
                        .into_iter()
                        .map(|line| plain(line, theme.base)),
                )
                .collect(),
            InstallState::Manual { steps, message } => {
                core::iter::once(plain(message.clone(), theme.error))
                    .chain(
                        steps
                            .lines()
                            .into_iter()
                            .map(|line| plain(line, theme.base)),
                    )
                    .collect()
            }
            _ => self.auth_pane(width, accent, theme, keys),
        }
    }

    /// `r` in browse: refused while an install or a login runs, or while a probe already does.
    pub(super) fn probe(&mut self, ctx: &mut Ctx<'_>) {
        // Before the other checks: an install is about to spawn a process of its own, and a probe
        // that spawned one per agent beside it would be racing it for the same `agent_box` row.
        if self.install_in_flight() {
            ctx.emit(Action::Error(
                "an install is running; probe afterwards".to_owned(),
            ));
        // And the same for a login, which ends by re-probing the very row `r` would re-probe.
        } else if self.auth_in_flight() {
            self.refuse_during_login(Act::AgentsProbe, ctx);
        } else if !self.probing {
            self.probing = true;
            ctx.request(StoreRequest::ProbeAgents);
        } else {
            ctx.emit(Action::Error("a probe is already running".to_owned()));
        }
    }
}

/// Whether a row's launch document declares how to install it (MOD-20 D12).
///
/// Parsed here rather than asked of the store: the same document the table is drawn from carries
/// the answer, and `R-AGT-10` is about the user being told before anything is fetched — including
/// being told that there is nothing to fetch.
pub(super) fn declares_a_source(launch: &Value) -> bool {
    // MOD-7 blueprint F-E: the one reading the install pre-flight and the box probe share.
    htui_agent::launch::declares_install(launch)
}

/// The progress cell of one frame (MOD-20 D19).
///
/// A zero numerator renders the phase word alone rather than `0%`. That is not cosmetic: `unpack`
/// counts *completed entries*, so a single-entry archive — which several registry entries are —
/// reports `done = 0` for the whole unpack and then jumps straight to `done == total`. A
/// percentage computed from that reads `0%` for the entire phase and then vanishes, which says
/// less than the phase word does and says it wrongly.
pub(super) fn phase_cell(phase: InstallPhase, done: u64, total: Option<u64>) -> String {
    if done == 0 {
        return format!("{phase}\u{2026}");
    }
    match total {
        Some(total) if total > 0 => format!("{phase} {}%", done.saturating_mul(100) / total),
        // No denominator: what is known is how much has arrived, so that is what is said.
        _ => format!("{phase} {}", human_bytes(done)),
    }
}

/// The one line a finished install leaves under the pane.
///
/// Never "installed" on its own for a failure: `R-AGT-6` says the probe decides, and D16's two
/// failure shapes differ in the one thing the user has to act on — whether the box was left with a
/// working version or with the tree that did not work.
pub(super) fn outcome_notice(outcome: &InstallOutcome) -> String {
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
pub(super) fn entry_id(dir: &Path) -> String {
    dir.parent()
        .and_then(Path::file_name)
        .map_or_else(String::new, |id| id.to_string_lossy().into_owned())
}

/// The first line of a probe's failure text, or the status when it produced none.
pub(super) fn first_line(tail: Option<&[String]>, status: ProbeStatus) -> String {
    tail.and_then(<[String]>::first)
        .cloned()
        .unwrap_or_else(|| verdict(status))
}

/// A byte count as a progress cell says it.
pub(super) fn human_bytes(bytes: u64) -> String {
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
        plural(
            usize::try_from(bytes).unwrap_or(usize::MAX),
            "byte",
            "bytes",
        )
    }
}

/// The paste field's label (MOD-115 M3): the prompt above it names what is pasted, and the row
/// is a `kit::form` row, so the field carries the form's marker rather than a `›` of its own.
const PASTE_LABEL: &str = "address";

/// A pane line behind `kit::list`'s blank gutter cell, cut with `…` to the rest of `width`: what
/// `kit::list` draws for a row that is not selected.
fn behind_gutter(line: Line<'static>, width: u16) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(own_spans(list::ellipsize(line, width.saturating_sub(1))));
    Line::from(spans)
}

/// The login chooser's cursor row: `kit::list`'s gutter in `accent`, then its selection band
/// over the rest of `width`.
fn chosen(line: Line<'static>, width: u16, accent: Style, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(GUTTER, accent)];
    spans.extend(own_spans(list::selection_band(
        line,
        width.saturating_sub(1),
        theme,
    )));
    Line::from(spans)
}

/// `line`'s spans with the line's own style patched under each, so a gutter put in front keeps
/// out of that style (a band ends at the gutter, as `kit::list` draws it).
fn own_spans(line: Line<'static>) -> Vec<Span<'static>> {
    let style = line.style;
    line.spans
        .into_iter()
        .map(|span| {
            let patched = style.patch(span.style);
            span.style(patched)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 D4: the probe notes name the probe key, and drop it unbound.
    #[test]
    fn the_probe_notes_name_the_probe_key_or_none() {
        assert_eq!(
            quota_note(Some("r")),
            "quota latches per chat, r cannot refresh it"
        );
        assert_eq!(
            quota_note(None),
            "quota latches per chat; a probe cannot refresh it"
        );
        assert_eq!(
            reprobes(Some("r")),
            " \u{b7} the next chat re-probes; r probes now"
        );
        assert_eq!(reprobes(None), " \u{b7} the next chat re-probes");
    }
}
