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
//! implied, and `quota_note` on the note line is where this section states it. (A code span, not
//! an intra-doc link: this is the **module**'s documentation and the function is private, so the
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

mod flows;
mod form;
mod table;

use htui_agent::auth::loopback::{self, PASTE_MAX};
use htui_core::model::{AgentId, AgentSummary, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use std::cell::Cell;

use crate::agent_settings::{self, AgentWrite, LITERAL_LAUNCH, REQUEST_NAMES};
use crate::agent_worker::{AUTH_ALREADY_CHOSEN, LOGIN_ENDED, NO_LOGIN_RUNNING};
use crate::app::{Action, Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::text::{Tone, notice};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
    is_error,
};
use crate::ui::text_field::PASTE_DOES_NOT_FIT;
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

use flows::{AuthState, InstallState, quota_note, reprobes};
use form::{
    Editor, GONE_CLOSED, NEEDS_CLI_BLOCK, PathsForm, Target, UNCHANGED, clash_notice,
    needs_cli_block, tool_paths_saved,
};
use table::MANUAL_NOTE;

/// What the section says when the store could not read the registry, and its next step
/// (MOD-115 M3 D4, MOD-83): an offline store mirrors `agent` but a store that is neither online nor
/// holding a mirror has nothing to show.
const UNAVAILABLE: &str = "agent registry needs Postgres";
/// See [`UNAVAILABLE`].
const UNAVAILABLE_NEXT: &str = "Settings \u{203a} Connection stores a DSN";

/// What an empty registry says (MOD-115 M3 D4): the next step is [`no_agents_next`].
const NO_AGENTS: &str = "no agents registered";

/// [`NO_AGENTS`]' next step: the create key through `AGENTS_BROWSE`, so a rebound one is the one
/// named.
fn no_agents_next(keys: &Keys) -> String {
    match keys.label(views::AGENTS_BROWSE, Act::New) {
        Some(new) => format!("{new} adds one"),
        None => "a key bound to new in keys.toml adds one".to_owned(),
    }
}

/// The keys line with nothing in flight (89 of the 98 columns, MOD-23 D245, MOD-66 D10). It stands
/// in for a help entry: a Settings section has no tab-level key rows of its own
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

/// The keys line while a registry form is open (MOD-23 D245). Saving and cancelling are the
/// focused field's own keys (MOD-67 D13), named through `AGENTS_FORM`'s `[field]` layer since
/// MOD-115 M3 (MD-A, C6), so a rebound `field.submit` or `field.cancel` shows its chord: with the
/// default keys it reads `Tab next field · Enter saves · Esc cancels`.
const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FormNextField, "next field"),
    Hint::One(Act::FieldSubmit, "saves"),
    Hint::One(Act::FieldCancel, "cancels"),
];

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

/// The hint line while a login is spawning or running (MOD-22 D270).
const HINT_AUTH_RUNNING: HintSpec = &[
    Hint::One(Act::AgentsOpenLink, "open link"),
    Hint::One(Act::AgentsPasteRedirect, "paste redirect"),
    Hint::One(Act::AgentsCancel, "cancel"),
];

/// The hint line while the paste field is open (MOD-22 D270): the field's own keys (MOD-67 D13),
/// through `CAPTURE`'s `[field]` layer since MOD-115 M3 (MD-A, C6): `Enter sends · Esc cancels`
/// under the default keys.
const HINT_PASTING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "sends"),
    Hint::One(Act::FieldCancel, "cancels"),
];

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
    /// `i` acts on one row, so there has to be a row it acts on. An index into `agents`, `None`
    /// while the table is empty; the table scrolls to keep it in view (`kit::list`).
    cursor: Option<usize>,
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
    /// `(rows, offset)` as the last render drew the table, measured in the section's rows, for the
    /// Settings pane's scrollbar (MOD-115 M3 D-11); `None` while every row fits.
    scroll: Cell<Option<(usize, usize)>>,
}

impl AgentsSection {
    /// Identity of the agents section.
    pub const ID: SectionId = SectionId("agents");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Which keys mean something here (MOD-23 D245, blueprint F-12), for the shell's hint row
    /// (MOD-67 M6 T2, [`SettingsSection::key_hint`]).
    ///
    /// It follows the live key mode first (MOD-67 M3 L-A Q6), so it always names the keys
    /// `on_key` answers: a login chooser or paste field opened over an install's manual steps shows
    /// its own keys, not `Esc close`.
    fn hint_text(&self, keys: &Keys) -> String {
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
        keys.hint(self.stack(), spec)
    }

    /// The line under the pane (MOD-23 D245): the last outcome, else [`quota_note`] while the
    /// section is idle. A `None` note still takes its line, so the layout never changes height with
    /// it. While a `manual` row is listed, the idle note ends in [`MANUAL_NOTE`], the key to the
    /// cell's `*` (MOD-66). The keys that shared the pane with it moved to the shell's hint row
    /// (MOD-67 M6 T2).
    fn note(&self, keys: &Keys) -> Option<String> {
        let idle = matches!(self.mode, Mode::Browse)
            && matches!(self.install, InstallState::Idle)
            && matches!(self.auth, AuthState::Idle);
        self.notice.clone().or_else(|| {
            idle.then(|| {
                let note = quota_note(
                    keys.label(views::AGENTS_BROWSE, Act::AgentsProbe)
                        .as_deref(),
                );
                if self.lists_a_manual_row() {
                    format!("{note}{MANUAL_NOTE}")
                } else {
                    note
                }
            })
        })
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
        let chord = ctx.pressed(&key);
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

    /// Which key mode is live (MOD-67 M3 L-A §2.1): `on_key` dispatches on it, and [`stack`] and
    /// [`hint`] read it, so the keys answered and the keys shown cannot disagree.
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
            // MOD-67 M6 PA-8: the shared refusal names no key; here the edit key is in reach.
            Err(LITERAL_LAUNCH) => ctx.emit(Action::Error(agent_settings::literal_launch(
                ctx.keys().label(views::AGENTS_BROWSE, Act::Edit).as_deref(),
            ))),
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
            FieldOutcome::Pass => form_navigation(
                ctx.pressed(&key),
                ctx.keys(),
                &mut editor.focus,
                editor.fields.len(),
            ),
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
            FieldOutcome::Pass => form_navigation(
                ctx.pressed(&key),
                ctx.keys(),
                &mut form.focus,
                form.fields.len(),
            ),
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
    fn on_written(&mut self, outcome: &AgentWrite, keys: &Keys) {
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
                    self.cursor = Some(index);
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
                        notice.push_str(&reprobes(
                            keys.label(views::AGENTS_BROWSE, Act::AgentsProbe)
                                .as_deref(),
                        ));
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
                        // MD-A (D-5): the key that retries, through the form's own stack.
                        let retry = keys.label(views::AGENTS_FORM, Act::FieldSubmit);
                        self.notice = Some(if clashes.is_empty() {
                            changed_elsewhere(retry.as_deref())
                        } else {
                            clash_notice(&clashes, retry.as_deref())
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
                    match keys.label(views::AGENTS_BROWSE, Act::AgentsProbe) {
                        Some(probe) => {
                            format!("`{name}` switched on; not probed yet \u{b7} {probe} probes")
                        }
                        None => format!("`{name}` switched on; not probed yet"),
                    }
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
}

/// What a form does with a key its focused field passed on (plan D231): `form.next_field` and
/// `form.prev_field` (`Tab`/`Down` and `BackTab`/`Up` by default, the `Down`/`Up` from
/// `VIEW_DEFAULTS`) move the focus with a wrap; a chord the modal global layer admits passes, so
/// `ctrl-c` still quits and `F1` opens help (MOD-67 D5); everything else is swallowed rather than
/// offered to the shell.
///
/// `chord` is the one the user pressed (`Ctx::pressed`, MOD-115 M3 MD-A), never the canonical
/// event the shell's `[field]` translation handed the field: a chord bound to a `[field]` row is
/// only that row's.
fn form_navigation(chord: KeyChord, keys: &Keys, focus: &mut usize, fields: usize) -> Handled {
    let len = fields.max(1);
    let stack = views::AGENTS_FORM;
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

/// The refusal of a key pressed while a registry write is in flight (blueprint F-20).
fn in_flight(request: &str) -> String {
    format!("`{request}` is still in flight")
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
        // One dispatch on `key_mode` (MOD-67 M3 R1), so the keys answered and the hint shown
        // follow one order by construction.
        match self.key_mode() {
            // An open form answers first (MOD-23 D231): the registry form, or the tool-paths form
            // under the same rule (MOD-66 D10, H-12). Neither can coexist with a modal below: a
            // form only opens while no install and no login is in flight (D232), and each modal
            // belongs to one of those. The order only fixes which answers first.
            KeyMode::Form if matches!(self.mode, Mode::Paths(_)) => self.on_paths_key(key, ctx),
            KeyMode::Form => self.on_editor_key(key, ctx),
            // A plan on screen answers first, and answers everything: MOD-20 D19's modality.
            KeyMode::Consent => self.answer_consent(key, ctx),
            // Then a method list, for the same reason and with the same limit (MOD-21 D20): the
            // adapter is spawned and waiting on an answer, and `j`/`k` mean the list rather than
            // the table for as long as it is up.
            KeyMode::Chooser => self.answer_chooser(key, ctx),
            // Then an open paste field (MOD-22 D270), which takes every key but the chords the
            // modal global layer admits: `q`, `x`, `o`, `?` and the digits are characters of the
            // address.
            KeyMode::Paste => self.on_paste_key(key, ctx),
            KeyMode::Browse => self.on_browse_key(key, ctx),
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
                self.on_written(outcome, ctx.keys());
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

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    /// The table, the pane under it (a consent, manual steps, a login, a form) and the note line
    /// (MOD-115 M3): the table through `kit::list`, an empty or unreadable registry as
    /// `kit::empty` with its next step, and the note as `kit::text::notice`. The pane's and the
    /// note's lines start behind `kit::list`'s gutter, so every row of the section starts at the
    /// table's first cell. The table's scroll is kept for the Settings pane's scrollbar (C3).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(area.width, ctx.accent(), ctx.theme, ctx.keys());
        let [rows, under, note_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        self.scroll.set(None);
        if self.unavailable.is_some() {
            empty::render(
                frame,
                rows,
                UNAVAILABLE,
                NextStep(UNAVAILABLE_NEXT),
                ctx.theme,
            );
        } else if self.agents.is_empty() {
            let next = no_agents_next(ctx.keys());
            empty::render(frame, rows, NO_AGENTS, NextStep(&next), ctx.theme);
        } else {
            let (total, shown, offset) = self.render_table(frame, rows, ctx);
            if total > shown {
                // Measured in the section's rows, the pane's whole inner height being its
                // viewport: the rows the table could not show are what the bar travels over.
                self.scroll
                    .set(Some((usize::from(area.height) + total - shown, offset)));
            }
        }
        let buf = frame.buffer_mut();
        for (y, line) in (under.y..under.bottom()).zip(&pane) {
            buf.set_line(under.x, y, line, under.width);
        }
        if let Some(note) = self.note(ctx.keys())
            && note_area.width > 1
        {
            // `settings/mod.rs`'s rule: a compare-and-set miss is the one notice to act on.
            let tone = if is_error(&note) {
                Tone::Error
            } else {
                Tone::Info
            };
            let line = notice(&note, tone, ctx.theme);
            buf.set_line(note_area.x + 1, note_area.y, &line, note_area.width - 1);
        }
    }

    fn scroll(&self) -> Option<(usize, usize)> {
        self.scroll.get()
    }
}
