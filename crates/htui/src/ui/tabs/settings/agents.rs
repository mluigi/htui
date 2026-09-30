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
//! implied, and `QUOTA_NOTE` on the note line is where this section states it. (A code span, not
//! an intra-doc link: this is the **module**'s documentation and the constant is private, so the
//! link is the one `rustdoc` rejects without `--document-private-items`. The two references further
//! down are inside private items' own docs and stay links.)
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
//!
//! Since MOD-23 it is also where a registry row is **written** (plan D230-D232): `n` opens a
//! create form and `e` an edit form in the pane under the table, and `t` flips this box's
//! per-box switch for the highlighted row. The form is hierarchy's one-line-field shape; its rules
//! are `crate::agent_settings`'s, run here for an instant refusal and again by the worker before
//! it writes (D247). Every write is one request (`R-NF-3`), answered by one self-naming
//! [`StoreReply::AgentWritten`] (D240): a plain [`StoreReply::Agents`] replaces the rows and never
//! closes the form or moves its token. The form never shows `launch.env` (D235), and nothing here
//! branches on an agent's name (`R-AGT-5`).

use chrono::{DateTime, Utc};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::PlanError;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_agent::{InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{Agent, AgentId, AgentSummary, Scope, Transport};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;

use crate::agent_settings::{
    self, AgentDraft, AgentWrite, DraftFields, FIELD_LABELS, REQUEST_NAMES, Refusal,
};
use crate::agent_worker::AUTH_ALREADY_CHOSEN;
use crate::app::{Action, Ctx, Handled};
use crate::store_worker::{AuthFrame, InstallFrame, StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    is_error, message,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the `on this box` column reads while this box has no `agent_box` row for the agent.
const NOT_PROBED: &str = "not probed";

/// What the `on this box` column reads while a probe this section asked for is in flight.
const PROBING: &str = "probing\u{2026}";

/// What the `on this box` column reads between `x` and the install's own last frame.
const CANCELLING: &str = "cancelling\u{2026}";

/// What replaces an absent `default_model`, and a quota this build has nothing to say about.
const NONE: &str = "\u{2014}";

/// The `utilization` at which a window **is** full: `docs/ANA-4.md` §7's own threshold, and the
/// one [`available`](htui_core::model::quota::available) skips a row on (`>= 1.0`).
const FULL: f64 = 1.0;

/// The highest percentage a window below [`FULL`] may render as, so the cell never claims an
/// allowance is exhausted while the predicate would still select it (review L-3).
const NEARLY_FULL: f64 = 99.0;

/// The keys line with nothing in flight (79 of the 98 columns, MOD-23 D245). It stands in for a
/// help entry: a Settings section has no [`KeyScope`](crate::keymap::KeyScope) of its own (MOD-20
/// D19), so the keys are written where they are pressed.
const HINT_IDLE: &str = "j/k select \u{b7} n new \u{b7} e edit \u{b7} t this box \u{b7} r probe \u{b7} i install \u{b7} a authenticate";

/// What the note line says when there is no notice and the section is idle: the one limit of this
/// section that is not a key (MOD-2 D73).
///
/// `r` re-probes every row and moves the `quota` column on none of them, because a probe handshake
/// reports no allowance and the value is latched from what a chat reports instead. `docs/ANA-4.md`
/// §7 asks for that to be "stated in the UI rather than implied", and this is the statement.
///
/// A notice wins the line: it is the answer to what the user just pressed. Since MOD-23 D245 the
/// note has a line of its own under the keys, so neither clips the other; it is on the line the
/// section **rests** in, which is the line a limit is read from.
const QUOTA_NOTE: &str = "quota latches per chat, r cannot refresh it";

/// The keys line while a registry form is open (MOD-23 D245).
const HINT_EDITING: &str = "Tab next field \u{b7} Enter saves \u{b7} Esc cancels";

/// What the `on this box` column reads for a row the human switched off on this box (MOD-23 D244).
/// Twelve characters, inside the 13-wide column.
const SWITCHED_OFF: &str = "switched off";

/// What an edit that changed nothing says as it closes (plan D239's "unchanged closes").
const UNCHANGED: &str = "nothing changed; nothing was written";

/// What a missing row says when no form is open to close: the write was `t`'s, or the form went
/// before its answer came back. Opens like [`DELETED_ELSEWHERE`], so [`is_error`] draws it the same.
const GONE_CLOSED: &str = "deleted elsewhere; nothing was written";

/// What `Created` adds for a `cli` row with no `settings.cli` block (plan R-9, blueprint F-13): such
/// a row resolves to the bare adapter id `cli`, which no build registers. Decided from the row's
/// data alone, never its name (`R-AGT-5`).
const NEEDS_CLI_BLOCK: &str =
    " \u{b7} a `cli` row needs a settings.cli block to chat (adapter id `cli` is not registered)";

/// What a spent token says when fields the user changed were changed elsewhere too (review M-1),
/// before their labels: shorter than [`CHANGED_ELSEWHERE`] so two labels still fit the bordered 98.
/// Labels only, never a value. Opens like [`CHANGED_ELSEWHERE`], so [`is_error`] draws it the same.
const CHANGED_ON_BOTH_SIDES: &str =
    "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: ";

/// What `Edited` adds when the save changed what a probe checked (plan D246): the stored verdict is
/// now older than the row, so the next chat re-probes by itself.
const REPROBES: &str = " \u{b7} the next chat re-probes; r probes now";

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

/// What the section is doing besides an install and a login (MOD-23 D230): browsing the table, or
/// one registry form open under it. `Browse` captures nothing.
#[derive(Debug, Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// A create or an edit form, taking every printable key.
    Editing(Editor),
}

/// The open form (plan D231): which write it makes, its fields in tab order, and which one has
/// focus.
///
/// It holds its own row identity (blueprint F-14) and never an index into the table: a create or a
/// failed read can re-sort or clear the rows under an open form.
#[derive(Debug)]
struct Editor {
    /// The write `Enter` makes.
    target: Target,
    /// The inputs, labelled from [`FIELD_LABELS`].
    fields: Vec<Field>,
    /// Index into `fields`.
    focus: usize,
}

/// What an open form writes.
#[derive(Debug)]
enum Target {
    /// `n`: a new row, whose `settings` start as this row's (OQ-5), named in the header.
    Create {
        /// The highlighted row when `n` was pressed; `None` on an empty table.
        settings_from: Option<(AgentId, String)>,
    },
    /// `e`: one existing row, as a compare-and-set on its `updated_at`.
    Edit {
        /// The row.
        agent_id: AgentId,
        /// Its name, for the header (plan D233: never edited).
        name: String,
        /// The token: the row's `updated_at` as a registry reply answered it (MOD-40 F-17).
        expected: DateTime<Utc>,
        /// The draft the row prefilled, which `Enter` compares against ("unchanged closes").
        opened: AgentDraft,
        /// Whether the draft in flight changes `transport`, `command` or `args` (plan D246).
        relaunch: bool,
    },
}

/// One labelled input of the form: `settings/hierarchy.rs`'s `Field`, private here.
#[derive(Debug)]
struct Field {
    /// One of [`FIELD_LABELS`].
    label: &'static str,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
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
    /// Browsing, or a registry form open under the table (MOD-23 D230).
    mode: Mode,
    /// The registry write in flight, by request name (MOD-23 D232, blueprint F-20). It refuses a
    /// second write and every probe, install and login key until [`StoreReply::AgentWritten`] or
    /// the write's own [`StoreReply::Failed`] lands, and nothing else clears it.
    busy: Option<&'static str>,
    /// The last outcome, on the note line under the keys.
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
    /// answer the question they just asked. Then the per-box switch (MOD-23 D244): a row the human
    /// switched off reads `switched off` ahead of the probe's words, because the switch decides
    /// selection before they do. Then a box with no `agent_box` row at all, or a bare one the
    /// switch wrote that no probe has filled (`probed_at` and `probe` both absent), which is the
    /// same fact. Then the snapshot's own verdict when it is not `ready` — `missing`,
    /// `unauthenticated` and `failed` are the three facts a version string cannot express (plan
    /// D50). Everything left is a row that works, or one written before the `probe` column
    /// existed, and both are answered by the version.
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
        if summary.user_off {
            return SWITCHED_OFF.to_owned();
        }
        let Some(row) = summary.on_box.as_ref() else {
            return NOT_PROBED.to_owned();
        };
        if row.probed_at.is_none() && row.probe.is_none() {
            return NOT_PROBED.to_owned();
        }
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
    ///
    /// An open registry form wins over both (MOD-23 D230): it can only be opened while no install
    /// and no login is in flight, so the one it can cover is a `Manual` pane, which is back the
    /// moment the form closes. It takes the width because its fields draw a window of their text.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        if let Mode::Editing(editor) = &self.mode {
            return core::iter::once(Line::styled(editor.header(), theme.base))
                .chain(editor.lines(width, theme))
                .collect();
        }
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

    /// The two lines under the pane (MOD-23 D245, blueprint F-12): which keys mean something here,
    /// then the note — the last outcome, else [`QUOTA_NOTE`] while the section is idle.
    ///
    /// Two lines rather than one since MOD-23 added three keys: the idle keys and the note were
    /// already 95 of the 98 columns together. A `None` note still takes its line, so the layout
    /// never changes height with it.
    fn hint(&self) -> (&'static str, Option<String>) {
        let keys = match (&self.mode, &self.install) {
            (Mode::Editing(_), _) => HINT_EDITING,
            // With no install in flight the login owns the line, because it is the only other
            // thing here that binds keys of its own.
            (Mode::Browse, InstallState::Idle) => match &self.auth {
                AuthState::Idle => HINT_IDLE,
                AuthState::Choosing { .. } => HINT_CHOOSING,
                AuthState::Starting { .. } | AuthState::Running { .. } => HINT_AUTH_RUNNING,
            },
            // A pre-flight is one registry read and one `HEAD`, so this is usually gone before it
            // is read — but `x` is offered here too, because a plan task that ends without a frame
            // would otherwise leave no way out of this state (review finding, MOD-20 T8).
            (Mode::Browse, InstallState::Planning { .. } | InstallState::Running { .. }) => {
                HINT_RUNNING
            }
            (Mode::Browse, InstallState::Pending { .. }) => HINT_PENDING,
            (Mode::Browse, InstallState::Manual { .. }) => HINT_MANUAL,
        };
        let idle = matches!(self.mode, Mode::Browse)
            && matches!(self.install, InstallState::Idle)
            && matches!(self.auth, AuthState::Idle);
        let note = self
            .notice
            .clone()
            .or_else(|| idle.then(|| QUOTA_NOTE.to_owned()));
        (keys, note)
    }

    /// Whether `n`, `e` or `t` is refused right now, with the status-line sentence that says why
    /// (MOD-23 D232, blueprint F-20).
    ///
    /// A write in flight first: one registry write at a time. Then an install, a login and a probe,
    /// in [`begin_install`](Self::begin_install)'s order and for its reason: each ends by writing
    /// this box's `agent_box` row, and a `launch` edited under a running probe would be recorded
    /// against the old recipe.
    fn refuse_write(&self, ctx: &mut Ctx<'_>) -> bool {
        let refusal = if let Some(busy) = self.busy {
            in_flight(busy)
        } else if self.install_in_flight() {
            "an install is running; edit afterwards".to_owned()
        } else if self.auth_in_flight() {
            "a login is running; edit afterwards".to_owned()
        } else if self.probing {
            "a probe is running; edit afterwards".to_owned()
        } else {
            return false;
        };
        ctx.emit(Action::Error(refusal));
        true
    }

    /// `n`: the create form, its `settings` source the highlighted row (OQ-5).
    fn open_create(&mut self) {
        let settings_from = self
            .selected()
            .map(|summary| (summary.agent.id, summary.agent.name.clone()));
        self.mode = Mode::Editing(Editor::create(settings_from));
        self.notice = None;
    }

    /// `e`: the edit form over the highlighted row, or say there is none.
    fn open_edit(&mut self, ctx: &mut Ctx<'_>) {
        let Some(summary) = self.selected() else {
            ctx.emit(Action::Error("no agent row is selected".to_owned()));
            return;
        };
        self.mode = Mode::Editing(Editor::edit(summary));
        self.notice = None;
    }

    /// `t`: flip this box's switch for the highlighted row (MOD-23 D242). The switch is
    /// `!user_off`, so the request carries `user_off` as the new value.
    fn switch_this_box(&mut self, ctx: &mut Ctx<'_>) {
        let Some(summary) = self.selected() else {
            ctx.emit(Action::Error("no agent row is selected".to_owned()));
            return;
        };
        let request = StoreRequest::SetAgentOnBox {
            agent_id: summary.agent.id,
            enabled: summary.user_off,
        };
        self.notice = None;
        self.send(request, ctx);
    }

    /// Sends one registry write and holds the guard until its answer (blueprint F-20).
    fn send(&mut self, request: StoreRequest, ctx: &mut Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while a form is open (plan D231): `settings/hierarchy.rs`'s rule.
    ///
    /// The focused field answers first, so `l`, `q`, `n` and the digits are letters here; what it
    /// passes on is the form's own navigation, and everything left over is swallowed rather than
    /// offered to the shell — with `CONTROL` chords excepted, so `ctrl-c` still quits.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        let outcome = match editor.fields.get_mut(editor.focus) {
            Some(field) => field.input.on_key(key),
            None => FieldOutcome::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                let len = editor.fields.len().max(1);
                match key.code {
                    KeyCode::Tab | KeyCode::Down => {
                        editor.focus = (editor.focus + 1) % len;
                        Handled::Consumed
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        editor.focus = (editor.focus + len - 1) % len;
                        Handled::Consumed
                    }
                    _ if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
                    _ => Handled::Consumed,
                }
            }
        }
    }

    /// `Enter` in a form: the local rules, then one request (plan D247).
    ///
    /// The form **stays open** until the reply lands, so a refusal from the worker (a taken name)
    /// leaves the text where it was and a second `Enter` retries it — which is why the first
    /// statement refuses while a write is in flight (blueprint F-20). A local refusal sends
    /// nothing, puts the field's sentence on the note line and moves the focus to that field.
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(busy) = self.busy {
            ctx.emit(Action::Error(in_flight(busy)));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        match editor.request(&self.agents) {
            Ok(Some(request)) => {
                self.notice = None;
                self.send(request, ctx);
            }
            Ok(None) => {
                self.mode = Mode::Browse;
                self.notice = Some(UNCHANGED.to_owned());
            }
            Err(refusal) => {
                editor.focus_on(refusal.field);
                self.notice = Some(refusal.to_string());
            }
        }
    }

    /// Whether the open form is the edit form of `id`.
    fn editing(&self, id: AgentId) -> bool {
        matches!(
            &self.mode,
            Mode::Editing(Editor { target: Target::Edit { agent_id, .. }, .. }) if *agent_id == id
        )
    }

    /// What one registry write did (MOD-23 D240), once the rows are the re-read's.
    ///
    /// Every outcome lands here and nowhere else: a plain [`StoreReply::Agents`] never closes the
    /// form or moves its token.
    fn on_written(&mut self, outcome: &AgentWrite) {
        match outcome {
            AgentWrite::Created { id, name } => {
                if matches!(
                    self.mode,
                    Mode::Editing(Editor {
                        target: Target::Create { .. },
                        ..
                    })
                ) {
                    self.mode = Mode::Browse;
                }
                // By id, not by where the cursor was (F-14): the re-read sorted the new row in.
                let row = self
                    .agents
                    .iter()
                    .position(|summary| summary.agent.id == *id);
                if let Some(index) = row {
                    self.cursor.select(Some(index));
                }
                let mut notice = format!("created `{name}`");
                if row
                    .and_then(|index| self.agents.get(index))
                    .is_some_and(|summary| needs_cli_block(&summary.agent))
                {
                    notice.push_str(NEEDS_CLI_BLOCK);
                }
                self.notice = Some(notice);
            }
            AgentWrite::Edited { id, name } => {
                let mut notice = format!("saved `{name}`");
                if self.editing(*id) {
                    if let Mode::Editing(Editor {
                        target: Target::Edit { relaunch: true, .. },
                        ..
                    }) = &self.mode
                    {
                        notice.push_str(REPROBES);
                    }
                    self.mode = Mode::Browse;
                }
                self.notice = Some(notice);
            }
            AgentWrite::Stale { id } => {
                if !self.editing(*id) {
                    self.notice = Some(CHANGED_ELSEWHERE_CLOSED.to_owned());
                    return;
                }
                let current = self
                    .agents
                    .iter()
                    .find(|summary| summary.agent.id == *id)
                    .map(|summary| {
                        (
                            summary.agent.updated_at,
                            agent_settings::draft_of(&summary.agent),
                        )
                    });
                match current {
                    // Review M-1: the fields the user left alone take the re-read's values, the
                    // ones they changed keep their text, and the token and the "unchanged"
                    // baseline are the row's now.
                    Some((updated_at, draft)) => {
                        let clashes = match &mut self.mode {
                            Mode::Editing(editor) => editor.rebase(updated_at, draft),
                            Mode::Browse => Vec::new(),
                        };
                        self.notice = Some(if clashes.is_empty() {
                            CHANGED_ELSEWHERE.to_owned()
                        } else {
                            format!("{CHANGED_ON_BOTH_SIDES}{}", clashes.join(", "))
                        });
                    }
                    None => {
                        self.mode = Mode::Browse;
                        self.notice = Some(DELETED_ELSEWHERE.to_owned());
                    }
                }
            }
            AgentWrite::Gone { id } => {
                if self.editing(*id) {
                    self.mode = Mode::Browse;
                    self.notice = Some(DELETED_ELSEWHERE.to_owned());
                } else {
                    self.notice = Some(GONE_CLOSED.to_owned());
                }
            }
            AgentWrite::Switched { id, name, enabled } => {
                // Review L-2: with no verdict on this box — no `agent_box` row, or a bare one the
                // switch itself wrote — there is nothing for the probe to decide yet.
                let probed = self
                    .agents
                    .iter()
                    .find(|summary| summary.agent.id == *id)
                    .and_then(|summary| summary.on_box.as_ref())
                    .is_some_and(|row| row.probe.is_some() || row.probed_at.is_some());
                self.notice = Some(if *enabled && probed {
                    format!("`{name}` switched on; the probe's verdict decides")
                } else if *enabled {
                    format!("`{name}` switched on; not probed yet \u{b7} r probes")
                } else {
                    format!("`{name}` switched off on this box")
                });
            }
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

        // MOD-2 D89's packing, which **reverses D76's ranking**: `name` takes the slack, and
        // `on this box` is the donor that pays for it.
        //
        // **`name` has no width of its own to tune, and that is the decision.** `Fill(1)` takes
        // every column the seven fixed ones leave, so the answer to "how wide should a name be" is
        // "as wide as the terminal allows" rather than a constant somebody chose. D76 had it as
        // `Length(8)` — a number picked to fit the longest name *then in the tree*, which is a
        // constant that goes stale the next time a registry row is added, and did: `claude-cli` is
        // 10.
        //
        // The instruction that produced this was "wider, like 128", and then "increase it to
        // maximum or like 256, there is no need to optimize the length". Both are satisfied here,
        // and the measured reason a literal 256 is *not* what the code says is worth recording,
        // because it is counter-intuitive: as a `Min` it does not widen this column, it **deletes
        // the other seven**. `Min(256)` and `Min(128)` both resolve to `[91, 0, 0, 0, 0, 0, 0, 0]`
        // at the bordered 98 — one column of names and seven of nothing — and `Min(128)` is still
        // `[128, 4, 3, 4, 3, 4, 3, 4]` at 160. Measured against ratatui 0.30.2's solver, not
        // reasoned about. As a `Max`, 256 is a no-op: `Max(256)` and `Fill(1)` resolve identically
        // at every width, so `Fill(1)` is the same behaviour written without a number that invites
        // a reader to think it means something.
        //
        // What `Fill(1)` gives at each width, which is the whole of the intent: 10 at the bordered
        // 98, 12 at 100, 32 at 120, 72 at 160, 112 at 200. **Every character a wider terminal adds
        // goes here first**, without a cap of any kind.
        //
        // Below ~98 the column yields rather than starving its neighbours, and can reach 0. That is
        // the one thing a `Min(10)` floor would have bought, and it is not bought here: at those
        // widths the table is already past the point of being readable — `default` gets 6 columns
        // for a 21-character model id — so a floor would preserve the *name* of a row whose every
        // other cell had become nonsense. The section is drawn inside a pane the app does not offer
        // below a usable size.
        //
        // The seven fixed columns each sit at their own longest string, which is why they are the
        // ones that do not move: `transport` (9), `models` (6) and `enabled` (7) at their headers,
        // `billing` (12) at `subscription`, `default` (21) at `gemini-3.7-flash-high` — the model id
        // `docs/ANA-4.md` §4.4 selects by, and D76's #1, which D89 leaves untouched — `quota` (13)
        // at `100% to 09-08`, the exhausted window that column exists to warn about, and
        // `on this box` (13), the donor.
        //
        // **The donor's price is stated rather than discovered.** At 13, its two longest cells clip:
        // `unauthenticated` (15) renders `unauthenticat` and `choose a method` (15) renders
        // `choose a meth`. Both are MOD-21's login flow written into a cell and the loss is real.
        // It is the affordable one because this column carries *status words*, where `default`
        // carries a selection coordinate and `quota` carries an exhaustion warning, and a truncated
        // status word is still legible as itself. D76 had already ranked its slack last for that
        // reason. The `a` key is still offered on the row.
        //
        // A number to be sceptical of if you are reading this to add a ninth column: the 98 that
        // every "at the bordered width" claim above refers to is **the test harness**, not a
        // screen. `testkit.rs`'s `DEFAULT_SIZE` is 100x30 and the pane's border costs two of it.
        // Nobody runs `htui` at 98 columns. It is the width the snapshots are pinned to, so it is
        // the width a regression is *caught* at, which is worth something — but it is not the width
        // the packing should be designed for, and D76's ranking was tighter than it needed to be
        // because it treated a fixture constant as a constraint.
        let table = Table::new(
            rows,
            [
                Constraint::Fill(1),
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(21),
                Constraint::Length(7),
                Constraint::Length(13),
                Constraint::Length(13),
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

impl Editor {
    /// The create form (plan D231): all eight fields, with `transport`, `billing` and `enabled`
    /// holding the values a new row most often wants, so a literal `command` is all it needs.
    fn create(settings_from: Option<(AgentId, String)>) -> Self {
        let texts = [
            "",
            Transport::Acp.as_str(),
            "",
            "",
            "",
            "",
            htui_core::model::Billing::Subscription.as_str(),
            "y",
        ];
        Self {
            target: Target::Create { settings_from },
            fields: FIELD_LABELS
                .iter()
                .zip(texts)
                .map(|(label, text)| Field::new(label, text))
                .collect(),
            focus: 0,
        }
    }

    /// The edit form over one row: every field but `name` (plan D233), prefilled from
    /// [`agent_settings::draft_of`] so the prefill and the "unchanged" check read `launch` the way
    /// the worker does. `env` has no field (D235).
    fn edit(summary: &AgentSummary) -> Self {
        let opened = agent_settings::draft_of(&summary.agent);
        let texts = prefill(&opened);
        Self {
            fields: FIELD_LABELS[1..]
                .iter()
                .zip(texts)
                .map(|(label, text)| Field::new(label, &text))
                .collect(),
            target: Target::Edit {
                agent_id: summary.agent.id,
                name: summary.agent.name.clone(),
                expected: summary.agent.updated_at,
                opened,
                relaunch: false,
            },
            focus: 0,
        }
    }

    /// A spent token under an open edit form (review M-1): rebase the form onto `current`, the row
    /// as the re-read answered it, and answer the labels of the fields changed on both sides.
    ///
    /// A field whose text is still what the old baseline prefilled takes `current`'s text, so a
    /// retry carries another writer's change instead of reverting it (`EditAgent` sends the whole
    /// draft). A field the user changed keeps its text; it is a clash when `current` changed it
    /// too, to something else. Then the token and the "unchanged" baseline are `current`'s.
    fn rebase(&mut self, updated_at: DateTime<Utc>, current: AgentDraft) -> Vec<&'static str> {
        let Target::Edit {
            expected,
            opened,
            relaunch,
            ..
        } = &mut self.target
        else {
            return Vec::new();
        };
        let before = prefill(opened);
        let after = prefill(&current);
        let mut clashes = Vec::new();
        // The edit form's fields are `FIELD_LABELS[1..]`, which is `prefill`'s order.
        for ((field, old), new) in self.fields.iter_mut().zip(before).zip(after) {
            if field.text() == old {
                if new != old {
                    field.input = TextField::with_text(&new);
                }
            } else if new != old && field.text() != new {
                clashes.push(field.label);
            }
        }
        *expected = updated_at;
        *opened = current;
        *relaunch = false;
        clashes
    }

    /// The text of the field labelled `label`, or `""` when the form has none (the edit form's
    /// `name`).
    fn text(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|field| field.label == label)
            .map_or("", Field::text)
    }

    /// Moves the focus to the field a refusal names; a field this form does not have (`launch`)
    /// leaves it where it is.
    fn focus_on(&mut self, label: &str) {
        if let Some(index) = self.fields.iter().position(|field| field.label == label) {
            self.focus = index;
        }
    }

    /// The form's request, `None` for an edit that changed nothing, or the first refusal in tab
    /// order (plan D247). A create also refuses a name the table already lists (D234); the store's
    /// `UNIQUE` stays the authority.
    fn request(&mut self, agents: &[AgentSummary]) -> Result<Option<StoreRequest>, Refusal> {
        let name = self.text(FIELD_LABELS[0]).to_owned();
        let draft = agent_settings::draft_from_fields(&DraftFields {
            transport: self.text(FIELD_LABELS[1]),
            command: self.text(FIELD_LABELS[2]),
            args: self.text(FIELD_LABELS[3]),
            models: self.text(FIELD_LABELS[4]),
            default_model: self.text(FIELD_LABELS[5]),
            billing: self.text(FIELD_LABELS[6]),
            enabled: self.text(FIELD_LABELS[7]),
        });
        match &mut self.target {
            Target::Create { settings_from } => {
                let name = agent_settings::parse_name(&name)?;
                if agents.iter().any(|summary| summary.agent.name == name) {
                    return Err(Refusal {
                        field: FIELD_LABELS[0],
                        reason: format!("`{name}` is already registered"),
                    });
                }
                Ok(Some(StoreRequest::CreateAgent {
                    name,
                    draft: draft?,
                    settings_from: settings_from.as_ref().map(|(id, _)| *id),
                }))
            }
            Target::Edit {
                agent_id,
                expected,
                opened,
                relaunch,
                ..
            } => {
                let draft = draft?;
                if draft == *opened {
                    return Ok(None);
                }
                *relaunch = opened.transport != draft.transport
                    || opened.command != draft.command
                    || opened.args != draft.args;
                Ok(Some(StoreRequest::EditAgent {
                    agent_id: *agent_id,
                    expected: *expected,
                    draft,
                }))
            }
        }
    }

    /// The pane's first line: what `Enter` writes, and where a new row's `settings` come from.
    fn header(&self) -> String {
        match &self.target {
            Target::Create {
                settings_from: Some((_, name)),
            } => format!("new agent \u{b7} settings from {name}"),
            Target::Create {
                settings_from: None,
            } => "new agent \u{b7} settings {}".to_owned(),
            Target::Edit { name, .. } => format!("edit {name}"),
        }
    }

    /// One line per field, the focused label accented and the focused field carrying the cursor:
    /// `settings/hierarchy.rs`'s `Editor::lines`. The label column is 13 wide (`default model`),
    /// which leaves 83 columns of text at the bordered 98 (D231).
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let label_width = self
            .fields
            .iter()
            .map(|field| field.label.chars().count())
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let padding = " ".repeat(label_width - field.label.chars().count());
                let style = if focused { theme.accent } else { theme.dim };
                let mut spans = vec![Span::styled(format!("{}{padding}: ", field.label), style)];
                let room = usize::from(width).saturating_sub(label_width + 2);
                spans.extend(
                    field
                        .input
                        .line(u16::try_from(room).unwrap_or(u16::MAX), focused, theme)
                        .spans,
                );
                Line::from(spans)
            })
            .collect()
    }
}

impl Field {
    /// A field labelled `label` holding `text`, cursor at the end.
    fn new(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
        }
    }

    /// What was typed. Never masked here, so [`TextField::text`] always answers.
    fn text(&self) -> &str {
        self.input.text().unwrap_or_default()
    }
}

/// The edit form's text for `draft`, in `FIELD_LABELS[1..]` order: what the form prefills, and
/// what a `Stale` rebase compares against (review M-1).
fn prefill(draft: &AgentDraft) -> [String; 7] {
    [
        draft.transport.as_str().to_owned(),
        draft.command.clone(),
        agent_settings::format_args(&draft.args),
        agent_settings::format_models(&draft.models),
        draft.default_model.clone().unwrap_or_default(),
        draft.billing.as_str().to_owned(),
        if draft.enabled { "y" } else { "n" }.to_owned(),
    ]
}

/// The refusal of a key pressed while a registry write is in flight (blueprint F-20).
fn in_flight(request: &str) -> String {
    format!("`{request}` is still in flight")
}

/// Whether a row cannot chat for want of a `settings.cli` block (plan R-9, blueprint F-13): a `cli`
/// row without one resolves to the bare adapter id `cli`. Read from the row's data, never its name
/// (`R-AGT-5`), and without `htui_agent`'s private `adapter_id_from`.
fn needs_cli_block(agent: &Agent) -> bool {
    agent.transport == Transport::Cli && agent.settings.get("cli").is_none()
}

impl SettingsSection for AgentsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Agents"
    }

    /// While a registry form is open (blueprint F-11): `l` and `h` are letters there, not section
    /// cycling. Derived from the mode, never a flag.
    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::Editing(_))
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `agent` is global, so the read does not change with the workspace.
        vec![StoreRequest::Agents]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // An open registry form answers first (MOD-23 D231). It cannot coexist with either modal
        // below: it only opens while no install and no login is in flight (D232), and each modal
        // belongs to one of those. The order only fixes which answers first.
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
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
        // `a`, `e`, `i`, `j`, `k`, `n`, `o`, `r`, `t` and `x` are free: the global keymap binds `q`,
        // `Tab`/`BackTab`, the digits, `?`, `ctrl-c` and `w`, and the tab itself consumes
        // `h`/`l`/`[`/`]`/arrows before a section is offered the key. `n` is also the consent and
        // chooser modals' "no", and they answer above, before this match.
        match key.code {
            // MOD-23 D232 and blueprint F-20: one registry write at a time, and none of the three
            // flows that write this box's `agent_box` row beside it.
            KeyCode::Char('r' | 'i' | 'a') if self.busy.is_some() => {
                if let Some(busy) = self.busy {
                    ctx.emit(Action::Error(in_flight(busy)));
                }
                Handled::Consumed
            }
            KeyCode::Char('n') => {
                if !self.refuse_write(ctx) {
                    self.open_create();
                }
                Handled::Consumed
            }
            KeyCode::Char('e') => {
                if !self.refuse_write(ctx) {
                    self.open_edit(ctx);
                }
                Handled::Consumed
            }
            KeyCode::Char('t') => {
                if !self.refuse_write(ctx) {
                    self.switch_this_box(ctx);
                }
                Handled::Consumed
            }
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
            // MOD-23 D240: the one reply a registry write lands on. The rows are the re-read's, and
            // the guard is down. Not `probing`, `install` or `auth`: none of them is this write's.
            StoreReply::AgentWritten { agents, outcome } => {
                self.agents = agents.clone();
                self.unavailable = None;
                self.busy = None;
                self.clamp_cursor();
                self.on_written(outcome);
            }
            // A registry write was refused — a taken name, a field the worker refused, this box not
            // registered, or offline. The shell has put `{request}: {message}` on the status line;
            // the note line says it under the form, which stays open over its text.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                self.notice = Some(message.clone());
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
        let pane = self.pane(area.width, ctx.theme);
        let [rows, consent, keys_area, note_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
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
        let (keys, note) = self.hint();
        frame.render_widget(Paragraph::new(Line::styled(keys, ctx.theme.dim)), keys_area);
        if let Some(note) = note {
            // `settings/mod.rs`'s rule: a compare-and-set miss is the one notice to act on.
            let style = if is_error(&note) {
                ctx.theme.error
            } else {
                ctx.theme.dim
            };
            frame.render_widget(Paragraph::new(Line::styled(note, style)), note_area);
        }
    }
}

/// Whether a row's launch document declares how to install it (MOD-20 D12).
///
/// Parsed here rather than asked of the store: the same document the table is drawn from carries
/// the answer, and `R-AGT-10` is about the user being told before anything is fetched — including
/// being told that there is nothing to fetch.
fn declares_a_source(launch: &Value) -> bool {
    // MOD-7 blueprint F-E: the one reading the install pre-flight and the box probe share.
    htui_agent::launch::declares_install(launch)
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
        // **A window that is not full never reads `100%`** (review L-3). `{:.0}` alone rounds
        // `0.995..0.999` up to `100%`, and a cell saying `100%` says
        // [`available`](htui_core::model::quota::available) would skip this row — which it would
        // not, because that predicate's rule is `utilization >= 1.0` and this window is not full.
        // The two have to agree, and the honest side of the disagreement is the one that does not
        // announce an exhausted allowance the operator still has room in.
        //
        // A **cap** rather than the review's `floor` of the product, because a bare floor trades
        // this error for the one the rounding format was there to avoid: `0.57 * 100.0` is
        // `56.99999999999999289` in binary floating point, so `floor` reports a fifty-seven-percent
        // window as `56%`. Capping at 99 below the threshold is the same answer as `floor` for
        // every value the finding is about — `0.996` reads `99%` either way — and leaves every
        // other window reading as written.
        let scaled = utilization * 100.0;
        let percentage = if utilization < FULL {
            format!("{:.0}%", scaled.min(NEARLY_FULL))
        } else {
            format!("{scaled:.0}%")
        };
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
