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
//!
//! Since MOD-22 a login can be **finished from another machine**. When the adapter's link advertises
//! a loopback `redirect_uri`, the pane names it and `p` opens a masked field. The address the
//! browser could not open is pasted there, its host and port checked here against that redirect,
//! and sent as one
//! [`StoreRequest::AuthDeliver`]. The login's own task relays it to the listener on this box and
//! answers with what the listener said. The pasted address is a credential for the length of that
//! request; the rule it is held to is `htui_agent::auth::loopback`'s (MOD-22 D273).
//!
//! Since MOD-66 it is also where this box is given a **manual path** for a tool (D10, D11): `m`
//! on the highlighted row opens a form with one field per `${tool}` its `discovery.tools`
//! declares, prefilled from `agent_box.probe.manual`. `Enter` sends one
//! [`StoreRequest::SetToolPaths`], which the agent runtime serves (it probes the row over the
//! paths), not `crate::agent_settings`'s loop; its [`StoreReply::AgentWritten`] closes the form.
//! A row whose `probe.source` is `manual` ends its `on this box` cell in `*`, and the idle note
//! says what the star means while such a row is listed.

use chrono::{DateTime, Utc};
use htui_agent::auth::loopback::{
    self, Advertised, DELIVERY_IN_FLIGHT, NO_LOOPBACK_REDIRECT, PASTE_MAX, RedirectUrl,
};
use htui_agent::auth::{AuthCall, AuthChoice, AuthMethodInfo};
use htui_agent::install::PlanError;
use htui_agent::launch::AgentLaunch;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_agent::{InstallOutcome, InstallPhase, InstallPlan, ManualSteps};
use htui_core::model::{Agent, AgentBox, AgentId, AgentSummary, Scope, Transport};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use crate::agent_settings::{
    self, AgentDraft, AgentWrite, DraftFields, FIELD_LABELS, LITERAL_LAUNCH, REQUEST_NAMES, Refusal,
};
use crate::agent_worker::{AUTH_ALREADY_CHOSEN, LOGIN_ENDED, NO_LOGIN_RUNNING};
use crate::app::{Action, Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{AuthFrame, InstallFrame, StoreReply, StoreRequest};
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    is_error, message, modal_rest,
};
use crate::ui::text_field::PASTE_DOES_NOT_FIT;
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

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

/// The keys line with nothing in flight (89 of the 98 columns, MOD-23 D245, MOD-66 D10). It stands
/// in for a help entry: a Settings section has no [`KeyScope`](crate::keymap::KeyScope) of its own
/// (MOD-20 D19), so the keys are written where they are pressed. `m` sits with the other write
/// keys (MOD-66 B9).
///
/// A hint spec since MOD-67 M3 (D9): rendered through `AGENTS_BROWSE`, so a rebound key shows its
/// new chord. With the default keys it reads `j/k select · n new · e edit · m paths · t this box ·
/// r probe · i install · a authenticate`.
const HINT_IDLE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "select"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::AgentsEditPaths, "paths"),
    Hint::One(Act::AgentsSwitchBox, "this box"),
    Hint::One(Act::AgentsProbe, "probe"),
    Hint::One(Act::AgentsInstall, "install"),
    Hint::One(Act::AgentsAuthenticate, "authenticate"),
];

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

/// The keys line while a registry form is open (MOD-23 D245). `Enter` and `Esc` are the focused
/// field's own keys (MOD-67 D13), so they are fixed text.
const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FormNextField, "next field"),
    Hint::Text("Enter saves"),
    Hint::Text("Esc cancels"),
];

/// What the `on this box` column reads for a row the human switched off on this box (MOD-23 D244).
/// Twelve characters, inside the 13-wide column.
const SWITCHED_OFF: &str = "switched off";

/// What an edit that changed nothing says as it closes (plan D239's "unchanged closes").
const UNCHANGED: &str = "nothing changed; nothing was written";

/// What the `on this box` cell appends when `probe.source` is `manual` (MOD-66 D10, the
/// maintainer's answer to blueprint F-3). One character, so the 13-wide column keeps it on every
/// verdict but `unauthenticated`, which that column already clips.
const MANUAL_SUFFIX: &str = "*";

/// What the idle note adds while any row's `probe.source` is `manual`: the key to
/// [`MANUAL_SUFFIX`] (MOD-66, the amendment to blueprint §5). 60 of the 98 columns with
/// [`QUOTA_NOTE`].
const MANUAL_NOTE: &str = " \u{b7} * manual path";

/// `m` on a row whose launch does not parse (MOD-66 B14): "literal" would misdescribe it.
const LAUNCH_UNREADABLE: &str = "this row's launch does not parse; nothing declares a tool";

/// What a missing row says when no form is open to close: the write was `t`'s, or the form went
/// before its answer came back. Opens like [`DELETED_ELSEWHERE`], so [`is_error`] draws it the same.
const GONE_CLOSED: &str = "deleted elsewhere; nothing was written";

/// What `Created` adds for a `cli` row with no `settings.cli` block (plan R-9, blueprint F-13): such
/// a row resolves to the bare adapter id `cli`, which no build registers. Decided from the row's
/// data alone, never its name (`R-AGT-5`).
const NEEDS_CLI_BLOCK: &str =
    " \u{b7} a `cli` row needs a settings.cli block to chat (adapter id `cli` is not registered)";

/// What a spent token says when fields the user changed were changed elsewhere too (review M-1),
/// before their labels ([`clash_notice`]): shorter than [`CHANGED_ELSEWHERE`] so labels fit the
/// bordered 98. Labels only, never a value. Opens like [`CHANGED_ELSEWHERE`], so [`is_error`] draws
/// it the same.
const CHANGED_ON_BOTH_SIDES: &str =
    "changed elsewhere \u{2014} reloaded; Enter retries \u{b7} also changed elsewhere: ";

/// The widest note line the running app draws (MOD-23 re-review Low-2): the Settings pane's
/// border costs two of the harness's 100, and the note is one line with no wrap.
const NOTE_WIDTH: usize = 98;

/// What `Edited` adds when the save changed what a probe checked (plan D246): the stored verdict is
/// now older than the row, so the next chat re-probes by itself.
const REPROBES: &str = " \u{b7} the next chat re-probes; r probes now";

/// The hint line while a plan waits for an answer.
const HINT_PENDING: HintSpec = &[
    Hint::One(Act::ConfirmYes, "install"),
    Hint::One(Act::ConfirmNo, "cancel"),
];

/// The hint line while an install streams.
const HINT_RUNNING: HintSpec = &[Hint::One(Act::AgentsCancel, "cancel install")];

/// The hint line while the manual steps are up.
const HINT_MANUAL: HintSpec = &[Hint::One(Act::Dismiss, "close")];

/// The hint line while the login chooser is waiting for a method (MOD-21 D20). Every chord of
/// `confirm.no` is listed (`n/Esc cancel`, MOD-67 M3 L-A Q5): `One` would drop the `Esc` users know.
const HINT_CHOOSING: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "choose"),
    Hint::One(Act::AgentsChoose, "select"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

/// The hint line while a login is spawning or running (MOD-22 D270: 41 of the 98 columns).
const HINT_AUTH_RUNNING: HintSpec = &[
    Hint::One(Act::AgentsOpenLink, "open link"),
    Hint::One(Act::AgentsPasteRedirect, "paste redirect"),
    Hint::One(Act::AgentsCancel, "cancel"),
];

/// The hint line while the paste field is open (MOD-22 D270): the field's own keys (MOD-67 D13).
const HINT_PASTING: HintSpec = &[Hint::Text("Enter sends"), Hint::Text("Esc cancels")];

/// What the install question swallows rather than passes (MOD-20 D19, MOD-67 M3 L-A Q2): the
/// section's own keys, resolved through `AGENTS_BROWSE` so a rebound one is swallowed under its
/// new chord. Kept out of `AGENTS_CONSENT` so the `?` box lists no dead key under the question.
const CONSENT_SWALLOWS: &[Act] = &[
    Act::ListDown,
    Act::ListUp,
    Act::AgentsInstall,
    Act::AgentsProbe,
    Act::AgentsCancel,
];

/// What the login chooser swallows rather than passes (MOD-21 D20, MOD-67 M3 L-A Q2), for
/// [`CONSENT_SWALLOWS`]' reason.
const CHOOSER_SWALLOWS: &[Act] = &[Act::AgentsAuthenticate, Act::AgentsCancel];

/// Which of the section's key modes is live (MOD-67 M3 L-A §2.1), in the order `on_key` answers
/// them: a form, the install question, the login chooser, the paste field, else browse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    Form,
    Consent,
    Chooser,
    Paste,
    Browse,
}

/// What `p` says while the login is being cancelled (MOD-22 D282).
const PASTE_CANCELLING: &str = "this login is being cancelled; there is nothing to paste into";

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

/// What the section is doing besides an install and a login (MOD-23 D230): browsing the table, or
/// one form open under it. `Browse` captures nothing.
#[derive(Debug, Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// A create or an edit form, taking every printable key.
    Editing(Editor),
    /// `m`'s form (MOD-66 D10): one path per tool the row declares, taking every printable key.
    /// Its own type, so the create and edit forms keep their `&'static str` labels (D11, B7).
    Paths(PathsForm),
}

/// The tool-paths form (MOD-66 D10, B7). It holds its own row identity (MOD-23 F-14), never an
/// index into the table. Its `Debug` is written by hand: it prints how many paths were
/// `opened`, never the paths, as [`PathField`]'s buffer never prints its text (review N3).
struct PathsForm {
    /// The row.
    agent_id: AgentId,
    /// Its name, for the header.
    name: String,
    /// The stored `probe.manual`, restricted to the declared tools: the prefill, and what `Enter`
    /// compares against ("unchanged closes").
    opened: BTreeMap<String, String>,
    /// One per declared tool, in `discovery.tools` (name) order.
    fields: Vec<PathField>,
    /// Index into `fields`.
    focus: usize,
}

impl core::fmt::Debug for PathsForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PathsForm")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("opened", &self.opened.len())
            .field("fields", &self.fields)
            .field("focus", &self.focus)
            .finish()
    }
}

/// One input of the tool-paths form, labelled with a tool name: a runtime string (MOD-66 D11).
#[derive(Debug)]
struct PathField {
    /// The `${tool}` name this path is for.
    tool: String,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
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
        let chord = KeyChord::from_event(key);
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
    fn refuse_during_login(&self, act: Act, ctx: &mut Ctx<'_>) {
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
    fn open_link(&self, ctx: &mut Ctx<'_>) {
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
    fn open_paste(&mut self, ctx: &mut Ctx<'_>) {
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
    fn on_paste_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
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
            FieldOutcome::Pass => modal_rest(views::CAPTURE, KeyChord::from_event(key)),
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
                self.notice = Some(format!("{what}: {status}"));
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
    ///
    /// Under the stream's link, at most one line of MOD-22's (D270, D282), the first that applies:
    /// the open paste field (a prompt, then `› ` and the masked field), a delivery in flight, or
    /// the advertised redirect with the key that pastes for it. The prompt and the field replace
    /// the redirect line rather than joining it, so the pane is at most six stderr lines, the link
    /// and two more. Every one of them names [`Advertised::target`] and never a pasted byte. It
    /// takes the width because the field draws a window of its dots.
    fn auth_pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
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
                    .collect();
                if let Some(target) = advertised.as_ref().map(Advertised::target) {
                    if let Some(field) = paste {
                        pane.push(Line::styled(
                            format!("paste the address the browser could not open ({target}):"),
                            theme.base,
                        ));
                        let mut spans = vec![Span::styled("\u{203a} ", theme.accent)];
                        spans.extend(field.line(width.saturating_sub(2), true, theme).spans);
                        pane.push(Line::from(spans));
                    } else if *delivering {
                        pane.push(Line::styled(
                            format!("delivering to {target}\u{2026}"),
                            theme.dim,
                        ));
                    } else {
                        pane.push(Line::styled(
                            format!(
                                "redirect: {target} \u{b7} p pastes the address if the browser \
                                 cannot reach it"
                            ),
                            theme.dim,
                        ));
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
        let chord = KeyChord::from_event(key);
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
    ///
    /// A `manual` row's verdict ends in `*`, whatever it is (MOD-66 D10, B17): the status word or
    /// the version, never the install, login, probing, switched-off or not-probed cells, which are
    /// not the snapshot's verdict.
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
        let cell = if let Some(status @ ("missing" | "unauthenticated" | "failed")) = status {
            status.to_owned()
        } else {
            let version = row.version.as_deref().unwrap_or(NONE);
            if row.enabled {
                version.to_owned()
            } else {
                format!("{version} (off)")
            }
        };
        if is_manual(row) {
            format!("{cell}{MANUAL_SUFFIX}")
        } else {
            cell
        }
    }

    /// Whether any listed row's cell shows `*`: the idle note then says what it means (MOD-66,
    /// the amendment to blueprint §5). Only a star [`on_box_cell`](Self::on_box_cell) can draw
    /// counts (review N1): none while `probing` covers every cell, and none on a row switched off
    /// here, whose cell reads `switched off`. The install and login cells need no test: the note
    /// is the idle one, so neither is running.
    fn lists_a_manual_row(&self) -> bool {
        !self.probing
            && self
                .agents
                .iter()
                .any(|summary| !summary.user_off && summary.on_box.as_ref().is_some_and(is_manual))
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
    /// The tool-paths form (MOD-66 D10) is drawn the same way and wins for the same reason.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        match &self.mode {
            Mode::Editing(editor) => {
                return core::iter::once(Line::styled(editor.header(), theme.base))
                    .chain(editor.lines(width, theme))
                    .collect();
            }
            Mode::Paths(form) => {
                return core::iter::once(Line::styled(form.header(), theme.base))
                    .chain(form.lines(width, theme))
                    .collect();
            }
            Mode::Browse => {}
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
            _ => self.auth_pane(width, theme),
        }
    }

    /// The two lines under the pane (MOD-23 D245, blueprint F-12): which keys mean something here,
    /// then the note — the last outcome, else [`QUOTA_NOTE`] while the section is idle.
    ///
    /// Two lines rather than one since MOD-23 added three keys: the idle keys and the note were
    /// already 95 of the 98 columns together. A `None` note still takes its line, so the layout
    /// never changes height with it. While a `manual` row is listed, the idle note ends in
    /// [`MANUAL_NOTE`], the key to the cell's `*` (MOD-66).
    ///
    /// The keys line follows the live key mode first (MOD-67 M3 L-A Q6), so it always names the
    /// keys `on_key` answers: a login chooser or paste field opened over an install's manual steps
    /// shows its own keys, not `Esc close`.
    fn hint(&self, keys: &Keys) -> (String, Option<String>) {
        let spec = match self.key_mode() {
            KeyMode::Form => HINT_EDITING,
            KeyMode::Consent => HINT_PENDING,
            KeyMode::Chooser => HINT_CHOOSING,
            KeyMode::Paste => HINT_PASTING,
            KeyMode::Browse => match (&self.install, &self.auth) {
                // A pre-flight is one registry read and one `HEAD`, so this is usually gone before
                // it is read — but `x` is offered here too, because a plan task that ends without a
                // frame would otherwise leave no way out of this state (review finding, MOD-20 T8).
                (InstallState::Planning { .. } | InstallState::Running { .. }, _) => HINT_RUNNING,
                (InstallState::Manual { .. }, _) => HINT_MANUAL,
                // With no install in flight the login owns the line, because it is the only other
                // thing here that binds keys of its own.
                (_, AuthState::Idle) => HINT_IDLE,
                (_, _) => HINT_AUTH_RUNNING,
            },
        };
        let keys = keys.hint(self.stack(), spec);
        let idle = matches!(self.mode, Mode::Browse)
            && matches!(self.install, InstallState::Idle)
            && matches!(self.auth, AuthState::Idle);
        let note = self.notice.clone().or_else(|| {
            idle.then(|| {
                if self.lists_a_manual_row() {
                    format!("{QUOTA_NOTE}{MANUAL_NOTE}")
                } else {
                    QUOTA_NOTE.to_owned()
                }
            })
        });
        (keys, note)
    }

    /// One key with no form, question, chooser or paste field up (MOD-67 M3 §6.2): whole chords
    /// through `AGENTS_BROWSE`, so `ctrl-r` probes nothing and `ctrl-n` opens nothing (defect 1),
    /// and `Down`/`Up` move the table as `j`/`k` do (ANA-26 §6.6).
    ///
    /// The section's keys are free of the shell's: the global layer binds `q`, `Tab`/`BackTab`,
    /// the digits, `?`, `F1`, `w`, `ctrl-f` and `ctrl-w`, and the tab takes `settings.*` (`h`/`l`/
    /// `[`/`]`/arrows) before a section is offered the key. `n` is also the consent and chooser
    /// modals' "no", and they answer before this. An act this state declines (`o`, `p`, `x`, `Esc`
    /// with nothing to act on) falls through to the next candidate, and finally to the shell.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::AGENTS_BROWSE, chord) {
            match act {
                // MOD-23 D232 and blueprint F-20: one registry write at a time, and none of the
                // three flows that write this box's `agent_box` row beside it.
                Act::AgentsProbe | Act::AgentsInstall | Act::AgentsAuthenticate
                    if self.busy.is_some() =>
                {
                    if let Some(busy) = self.busy {
                        ctx.emit(Action::Error(in_flight(busy)));
                    }
                }
                Act::New => {
                    if !self.refuse_write(ctx) {
                        self.open_create();
                    }
                }
                Act::Edit => {
                    if !self.refuse_write(ctx) {
                        self.open_edit(ctx);
                    }
                }
                Act::AgentsSwitchBox => {
                    if !self.refuse_write(ctx) {
                        self.switch_this_box(ctx);
                    }
                }
                // MOD-66 D10: refused as `n`, `e` and `t` are, because it writes this box's row too.
                Act::AgentsEditPaths => {
                    if !self.refuse_write(ctx) {
                        self.open_paths(ctx);
                    }
                }
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::AgentsInstall => self.begin_install(ctx),
                Act::AgentsAuthenticate => self.begin_auth(ctx),
                // `o` is bound only while a flow is running: it is otherwise a free letter, and a
                // section that swallowed it everywhere would be claiming a key it does nothing with.
                Act::AgentsOpenLink if self.auth_in_flight() => self.open_link(ctx),
                // `o`'s kind of key (MOD-22 D282): bound while a flow is in flight, and a refusal
                // names itself rather than the key doing nothing.
                Act::AgentsPasteRedirect if self.auth_in_flight() => self.open_paste(ctx),
                // `x` cancels a running install **and** a pre-flight. Planning is one `GET` and one
                // `HEAD`, so it is usually over before a key lands — but if the plan task ends
                // without a frame (a panic inside it is caught by `tokio::spawn` and swept by
                // `is_finished`), `Planning` would otherwise be a state with no way out but a
                // restart. The runtime serves a cancel during planning and answers
                // `Failed { "install_cancel" }` for a task already swept, and both land on `Idle`.
                Act::AgentsCancel
                    if matches!(
                        self.install,
                        InstallState::Running { .. } | InstallState::Planning { .. }
                    ) =>
                {
                    if let InstallState::Running { cancelling, .. } = &mut self.install {
                        *cancelling = true;
                    }
                    ctx.request(StoreRequest::InstallCancel);
                }
                // The same key for the other stream: `x` stops a login wherever it has got to, and
                // the cell reads `cancelling…` until the flow's own last frame says it stopped.
                Act::AgentsCancel if self.auth_in_flight() => self.begin_auth_cancel(ctx),
                Act::Dismiss if matches!(self.install, InstallState::Manual { .. }) => {
                    self.install = InstallState::Idle;
                }
                Act::AgentsProbe => self.probe(ctx),
                // A global act, or one this state declines: the next candidate.
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// `r` in browse: refused while an install or a login runs, or while a probe already does.
    fn probe(&mut self, ctx: &mut Ctx<'_>) {
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

    /// Which key mode is live (MOD-67 M3 L-A §2.1): `on_key`'s order, read by [`stack`] and
    /// [`hint`], so the keys answered and the keys shown cannot disagree.
    ///
    /// [`stack`]: Self::stack
    /// [`hint`]: Self::hint
    fn key_mode(&self) -> KeyMode {
        if matches!(self.mode, Mode::Editing(_) | Mode::Paths(_)) {
            KeyMode::Form
        } else if matches!(self.install, InstallState::Pending { .. }) {
            KeyMode::Consent
        } else if matches!(self.auth, AuthState::Choosing { .. }) {
            KeyMode::Chooser
        } else if matches!(self.auth, AuthState::Running { paste: Some(_), .. }) {
            KeyMode::Paste
        } else {
            KeyMode::Browse
        }
    }

    /// The stack of the live key mode (MOD-67 D4): `key_stack`, the key handlers and the hint
    /// all read it.
    fn stack(&self) -> Stack<'static> {
        match self.key_mode() {
            KeyMode::Form => views::AGENTS_FORM,
            KeyMode::Consent => views::AGENTS_CONSENT,
            KeyMode::Chooser => views::AGENTS_CHOOSER,
            KeyMode::Paste => views::CAPTURE,
            KeyMode::Browse => views::AGENTS_BROWSE,
        }
    }

    /// Whether `n`, `e`, `m` or `t` is refused right now, with the status-line sentence that says
    /// why (MOD-23 D232, blueprint F-20; MOD-66 D10).
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
        // Review L-4: a stored model name with a comma would not survive the form's `models`.
        if let Err(refusal) = agent_settings::editable(&summary.agent) {
            ctx.emit(Action::Error(refusal.to_string()));
            return;
        }
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

    /// `m`: the tool-paths form over the highlighted row, or the sentence that says why not
    /// (MOD-66 D10, B14).
    fn open_paths(&mut self, ctx: &mut Ctx<'_>) {
        let Some(summary) = self.selected() else {
            ctx.emit(Action::Error("no agent row is selected".to_owned()));
            return;
        };
        match PathsForm::open(summary) {
            Ok(form) => {
                self.mode = Mode::Paths(form);
                self.notice = None;
            }
            Err(refusal) => ctx.emit(Action::Error(refusal.to_owned())),
        }
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
    /// offered to the shell — except what the modal global layer admits (CONTROL, ALT, function
    /// keys: MOD-67 D5), so `ctrl-c` still quits and `F1` opens help.
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
                form_navigation(key, ctx.keys(), &mut editor.focus, editor.fields.len())
            }
        }
    }

    /// One key while the tool-paths form is open (MOD-66 D10): [`on_editor_key`]'s rule over
    /// the form's own fields, so a path's `h`, `l` and `q` are letters of it (H-12).
    ///
    /// [`on_editor_key`]: Self::on_editor_key
    fn on_paths_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Paths(form) = &mut self.mode else {
            return Handled::Pass;
        };
        let outcome = match form.fields.get_mut(form.focus) {
            Some(field) => field.input.on_key(key),
            None => FieldOutcome::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit_paths(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                form_navigation(key, ctx.keys(), &mut form.focus, form.fields.len())
            }
        }
    }

    /// `Enter` in the tool-paths form: [`submit`](Self::submit)'s shape (MOD-66 D10). The guard
    /// first; then one `SetToolPaths`, or `UNCHANGED` and the form closed, or a local refusal
    /// naming the tool, with the focus moved to its field. The form **stays open** until the
    /// reply, so a refusal from the worker leaves the text where it was.
    fn submit_paths(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(busy) = self.busy {
            ctx.emit(Action::Error(in_flight(busy)));
            return;
        }
        let Mode::Paths(form) = &mut self.mode else {
            return;
        };
        match form.request() {
            Ok(Some(request)) => {
                self.notice = None;
                self.send(request, ctx);
            }
            Ok(None) => {
                self.mode = Mode::Browse;
                self.notice = Some(UNCHANGED.to_owned());
            }
            Err((index, sentence)) => {
                form.focus = index;
                self.notice = Some(sentence);
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
                            // `editing(id)` held above, so only the edit form reaches here.
                            Mode::Browse | Mode::Paths(_) => Vec::new(),
                        };
                        self.notice = Some(if clashes.is_empty() {
                            CHANGED_ELSEWHERE.to_owned()
                        } else {
                            clash_notice(&clashes)
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
            // MOD-66 D10, B18: the form closes only on its own row's answer (MOD-23 F-14).
            AgentWrite::ToolPaths { id, name, status } => {
                if matches!(&self.mode, Mode::Paths(form) if form.agent_id == *id) {
                    self.mode = Mode::Browse;
                }
                self.notice = Some(tool_paths_saved(name, *status));
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
            .map(|field| cell_width(field.label))
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let style = if focused { theme.accent } else { theme.dim };
                let label = cells::pad(field.label, label_width);
                let mut spans = vec![Span::styled(format!("{label}: "), style)];
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

impl PathsForm {
    /// The form over `summary`, or the refusal sentence: [`LAUNCH_UNREADABLE`] for a `launch`
    /// that does not parse, [`LITERAL_LAUNCH`] for no `discovery` or no `tools` (MOD-66 D10,
    /// B14). Fields in `discovery.tools` order, prefilled from the stored `probe.manual`. An
    /// unreadable snapshot prefills nothing (blueprint H-21).
    fn open(summary: &AgentSummary) -> Result<Self, &'static str> {
        let launch =
            AgentLaunch::deserialize(&summary.agent.launch).map_err(|_| LAUNCH_UNREADABLE)?;
        let tools: Vec<String> = launch
            .discovery
            .map(|discovery| discovery.tools.into_keys().collect())
            .unwrap_or_default();
        if tools.is_empty() {
            return Err(LITERAL_LAUNCH);
        }
        let opened: BTreeMap<String, String> = summary
            .on_box
            .as_ref()
            .and_then(ProbeSnapshot::from_row)
            .map(|snapshot| snapshot.manual)
            .unwrap_or_default()
            .into_iter()
            .filter(|(tool, _)| tools.contains(tool))
            .collect();
        let fields = tools
            .into_iter()
            .map(|tool| PathField {
                input: TextField::with_text(opened.get(&tool).map_or("", String::as_str)),
                tool,
            })
            .collect();
        Ok(Self {
            agent_id: summary.agent.id,
            name: summary.agent.name.clone(),
            opened,
            fields,
            focus: 0,
        })
    }

    /// The form's request (MOD-66 D9, D10): `Ok(None)` for an unchanged map, else one
    /// `SetToolPaths` with every non-blank field trimmed (an empty map clears). A field
    /// `parse_tool_path` refuses is `Err((its index, the sentence))`, the first in tab order.
    fn request(&self) -> Result<Option<StoreRequest>, (usize, String)> {
        let mut paths = BTreeMap::new();
        for (index, field) in self.fields.iter().enumerate() {
            let text = field.input.text().unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let path = agent_settings::parse_tool_path(&field.tool, text)
                .map_err(|sentence| (index, sentence))?;
            paths.insert(field.tool.clone(), path);
        }
        if paths == self.opened {
            return Ok(None);
        }
        Ok(Some(StoreRequest::SetToolPaths {
            agent_id: self.agent_id,
            paths,
        }))
    }

    /// The pane's first line: whose paths `Enter` writes, and what an empty field means.
    fn header(&self) -> String {
        format!("tool paths for {} \u{b7} empty = no manual path", self.name)
    }

    /// One line per tool, [`Editor::lines`]' shape (MOD-66 D11, B8). The label column is the
    /// longest tool name in cells, capped at a third of `width`. Each name is `cells::fit` to the
    /// column (MOD-60): padded when it fits, else cut to at most one cell less than the column and
    /// ending in `…`, then padded back, so every label ends at the same cell.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let longest = self
            .fields
            .iter()
            .map(|field| cell_width(&field.tool))
            .max()
            .unwrap_or(0);
        let column = longest.min(usize::from(width) / 3);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let style = if focused { theme.accent } else { theme.dim };
                let label = cells::fit(&field.tool, column);
                let mut spans = vec![Span::styled(format!("{label}: "), style)];
                let room = usize::from(width).saturating_sub(column + 2);
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

/// What a form does with a key its focused field passed on (plan D231): `form.next_field` and
/// `form.prev_field` (`Tab`/`Down` and `BackTab`/`Up` by default, the `Down`/`Up` from
/// `VIEW_DEFAULTS`) move the focus with a wrap; a chord the modal global layer admits passes, so
/// `ctrl-c` still quits and `F1` opens help (MOD-67 D5); everything else is swallowed rather than
/// offered to the shell.
fn form_navigation(key: KeyEvent, keys: &Keys, focus: &mut usize, fields: usize) -> Handled {
    let len = fields.max(1);
    let stack = views::AGENTS_FORM;
    let chord = KeyChord::from_event(key);
    // The first candidate decides (MOD-67 M3 §6.3): a global act there is the pass rule's.
    match keys.actions(stack, chord).first() {
        Some(Act::FormNextField) => {
            *focus = (*focus + 1) % len;
            return Handled::Consumed;
        }
        Some(Act::FormPrevField) => {
            *focus = (*focus + len - 1) % len;
            return Handled::Consumed;
        }
        _ => {}
    }
    modal_rest(stack, chord)
}

/// What the install question or the login chooser does with a key none of its own acts took
/// (MOD-67 M3 L-A Q2): swallowed when it is one of the section's `swallowed` browse keys, so the
/// row under the pane cannot move; otherwise passed on to the shell.
fn swallows(keys: &Keys, chord: KeyChord, swallowed: &[Act]) -> Handled {
    if keys
        .actions(views::AGENTS_BROWSE, chord)
        .iter()
        .any(|act| swallowed.contains(act))
    {
        Handled::Consumed
    } else {
        Handled::Pass
    }
}

/// Whether an `agent_box` row's `probe.source` is `manual` (MOD-66 D10). Read by key, as
/// [`AgentsSection::on_box_cell`] reads `status`, so a snapshot this build cannot parse whole is
/// still marked.
fn is_manual(row: &AgentBox) -> bool {
    row.probe
        .as_ref()
        .and_then(|probe| probe.get("source"))
        .and_then(Value::as_str)
        == Some("manual")
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

/// [`CHANGED_ON_BOTH_SIDES`] with the clashing labels in form order, as many as fit
/// [`NOTE_WIDTH`], then ` +N more` for the rest (MOD-23 re-review Low-2). Greedy and
/// deterministic: a label is shown only if it and the count still owed after it fit, so the
/// line never passes the width.
fn clash_notice(clashes: &[&str]) -> String {
    let mut notice = CHANGED_ON_BOTH_SIDES.to_owned();
    let mut shown = 0;
    for (index, label) in clashes.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        let after = clashes.len() - index - 1;
        let owed = if after == 0 {
            String::new()
        } else {
            format!(" +{after} more")
        };
        let width =
            cell_width(&notice) + cell_width(separator) + cell_width(label) + cell_width(&owed);
        if width > NOTE_WIDTH {
            break;
        }
        notice.push_str(separator);
        notice.push_str(label);
        shown += 1;
    }
    let hidden = clashes.len() - shown;
    if hidden > 0 {
        notice.push_str(&format!(" +{hidden} more"));
    }
    notice
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

    /// While a registry or tool-paths form is open (blueprint F-11, MOD-66 H-12), and while the
    /// login's paste field is (MOD-22 D270): `l` and `h` are letters there, not section cycling.
    /// Derived from the state, never a flag.
    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::Editing(_) | Mode::Paths(_))
            || matches!(self.auth, AuthState::Running { paste: Some(_), .. })
    }

    /// MOD-22 review M-1: a bracketed paste lands in the open paste field — masked, whole, and
    /// only within the [`PASTE_MAX`] the field was opened with, so a longer one is refused by
    /// `validate`'s own sentence rather than reallocating the buffer — or in the registry form's
    /// focused field. Anywhere else it is not this section's.
    ///
    /// Review R2-L3: a paste made before `p` while a login runs is `p` and the paste at once — the
    /// field opens and takes it — or, where `p` would be refused (no redirect, a delivery in
    /// flight, a login being cancelled), the same refusal and nothing opened.
    fn takes_paste(&self) -> bool {
        self.captures_input()
            || matches!(
                self.auth,
                AuthState::Starting { .. } | AuthState::Running { .. }
            )
    }

    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        if matches!(
            self.auth,
            AuthState::Starting { .. } | AuthState::Running { paste: None, .. }
        ) && matches!(self.mode, Mode::Browse)
        {
            self.open_paste(ctx);
        }
        if let AuthState::Running {
            paste: Some(field), ..
        } = &mut self.auth
        {
            if !field.on_paste(text) {
                // R2-L3: `TooLong` is the paste's own length; a shorter one that does not fit
                // beside what was already typed is the field's.
                let alone: usize = text
                    .chars()
                    .filter(|c| !c.is_control())
                    .map(char::len_utf8)
                    .sum();
                let refusal = if alone > PASTE_MAX {
                    loopback::PasteError::TooLong.to_string()
                } else {
                    PASTE_DOES_NOT_FIT.to_owned()
                };
                ctx.emit(Action::Error(refusal));
            }
            return Handled::Consumed;
        }
        if matches!(
            self.auth,
            AuthState::Starting { .. } | AuthState::Running { .. }
        ) && matches!(self.mode, Mode::Browse)
        {
            // `open_paste` refused by name; the paste is dropped with that sentence.
            return Handled::Consumed;
        }
        if let Mode::Editing(editor) = &mut self.mode {
            if let Some(field) = editor.fields.get_mut(editor.focus) {
                field.input.on_paste(text);
            }
            return Handled::Consumed;
        }
        // MOD-66 H-12: a path is the kind of text that is pasted.
        if let Mode::Paths(form) = &mut self.mode {
            if let Some(field) = form.fields.get_mut(form.focus) {
                field.input.on_paste(text);
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Unscoped: `agent` is global, so the read does not change with the workspace.
        vec![StoreRequest::Agents]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {}

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // An open registry form answers first (MOD-23 D231). It cannot coexist with either modal
        // below: it only opens while no install and no login is in flight (D232), and each modal
        // belongs to one of those. The order only fixes which answers first.
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        // The tool-paths form, for the same reason and under the same rule (MOD-66 D10, H-12).
        if matches!(self.mode, Mode::Paths(_)) {
            return self.on_paths_key(key, ctx);
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
        // Then an open paste field (MOD-22 D270), which takes every key but the chords the modal
        // global layer admits: `q`, `x`, `o`, `?` and the digits are characters of the address.
        if matches!(self.auth, AuthState::Running { paste: Some(_), .. }) {
            return self.on_paste_key(key, ctx);
        }
        self.on_browse_key(key, ctx)
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
            //
            // `SET_TOOL_PATHS` beside them (MOD-66 H-6): the agent runtime serves it, so it is not
            // in `REQUEST_NAMES`, but `send` holds the guard on its name all the same.
            StoreReply::Failed { request, message }
                if REQUEST_NAMES.contains(request)
                    || *request == agent_settings::SET_TOOL_PATHS =>
            {
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
            // MOD-22 D270: the same for a refused or failed delivery — one request answered no;
            // the login and its link stay, and `p` works again. Unless the refusal says no login
            // is held at all, which is `auth_choose`'s rule: a state a second `a` can start from.
            //
            // `agent_worker::LOGIN_ENDED` is not that (review L-2): the flow's end overtook the delivery, and
            // the worker guarantees the flow's own terminal frame follows — after its drain and
            // its re-probe. Idling here would offer `a` while the old flow still holds its claim,
            // and a refused `AuthStart` would make that last frame stale; so the arm below only
            // stops waiting for the delivery, and the terminal frame ends the pane.
            StoreReply::Failed { request, message }
                if *request == "auth_deliver" && message == NO_LOGIN_RUNNING =>
            {
                self.auth = AuthState::Idle;
            }
            StoreReply::Failed { request, message } if *request == "auth_deliver" => {
                if let AuthState::Running {
                    delivering,
                    cancelling,
                    advertised,
                    paste,
                    ..
                } = &mut self.auth
                {
                    *delivering = false;
                    // Review R2-L1: the pane checks host and port only, so most refusals of a
                    // paste arrive here, after the field closed. One the worker answered — not
                    // the flow's end overtaking it, which waits for the last frame — reopens a
                    // fresh masked field for the re-paste; the sentence is the shell's, from this
                    // same reply.
                    if message != LOGIN_ENDED && !*cancelling && advertised.is_some() {
                        *paste = Some(TextField::masked_with_capacity(PASTE_MAX));
                    }
                }
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
        let (keys, note) = self.hint(ctx.keys());
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

/// What `AgentWritten::ToolPaths` says (MOD-66 D10): the row and the probe's word, never a path.
fn tool_paths_saved(name: &str, status: ProbeStatus) -> String {
    format!("tool paths saved for `{name}` \u{b7} this box: {status}")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-60: the tool-name column is measured and cut in cells. A CJK name that fits the
    /// column by `char` count is twice as wide on screen; it is cut with `…` so every label,
    /// wide or not, ends at the same cell. At an odd column (63 / 3 = 21) the cut lands on a
    /// cluster boundary and `…` sits right against the `": "`; at an even one (60 / 3 = 20)
    /// a wide glyph cannot fill the last cell before `…`, so `cells::fit` pads one space after
    /// it. Both pin the exact label, so an over-clip padded back to the column cannot pass.
    #[test]
    fn a_wide_tool_name_is_fitted_to_the_label_column() {
        let wide = "\u{6f22}";
        let cases = [
            (63, format!("{}\u{2026}: ", wide.repeat(10))),
            (60, format!("{}\u{2026} : ", wide.repeat(9))),
        ];
        for (width, expected) in cases {
            let column = usize::from(width) / 3;
            let form = PathsForm {
                agent_id: AgentId::new(),
                name: "agent".to_owned(),
                opened: BTreeMap::new(),
                fields: [wide.repeat(20), "git".to_owned()]
                    .into_iter()
                    .map(|tool| PathField {
                        tool,
                        input: TextField::new(),
                    })
                    .collect(),
                focus: 0,
            };

            let lines = form.lines(width, &Theme::default());

            for line in &lines {
                let label = &line.spans[0].content;
                assert_eq!(cell_width(label), column + 2, "{label:?} against {column}");
            }
            assert_eq!(lines[0].spans[0].content, expected, "width {width}");
            assert_eq!(
                lines[1].spans[0].content,
                format!("{:<column$}: ", "git"),
                "width {width}"
            );
        }
    }
}
