//! The Templates view of the Skills tab (MOD-9 milestone 1; plan D6, D11–D14; blueprint D19, D20,
//! D27): the scope's prompt templates as a tree, any version's body, a line diff between two
//! versions or a version and the compiled default, and an editor that saves through `parse`.
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle
//! and no `UserId`, renders from the last `TemplatesSnapshot` and never patches a row into it.
//! `parse` is the only validator, run on `Ctrl+S` and on an `$EDITOR` return; the store runs it
//! again behind the compare-and-set (plan D4), so a body the view let through cannot land broken.
//!
//! A save **lands on its own reply** (MOD-59): only a `TemplateSaved` while the save is in flight
//! closes the editor; a plain `Templates` read never does, so a read served ahead of the save
//! leaves the editor, the token and `busy` alone. A `TemplatesStale` keeps the draft and moves the
//! token to the head as it is now.
//!
//! The token a save carries is the head version when the editor opened, not the version shown:
//! editing v1 while v3 is head saves v4 (plan D1, OQ-5).

use core::cell::Cell;

use htui_core::model::{ProjectId, PromptTemplate};
use htui_core::prompt::edit_help::{self, HelpTarget};
use htui_core::prompt::{TemplateError, TemplateRole, body_of, parse};
use htui_core::store::invalid_template_name;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::agent_help::{AgentHelp, HelpOutcome, Report, accepted};
use super::{framed, hint_line, holder, joined, unavailable, view_chips, view_chips_if_room};
use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, edited};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::{READ_NAME, REQUEST_NAMES, TemplateBody, TemplatesSnapshot};
use crate::ui::cells;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::list::{self, Row as ListRow, RowKind};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::pane::{Focus, Pane as KitPane, capture_area};
use crate::ui::kit::scroll::{Scroll, Step};
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::shell::PaneId;
use crate::ui::{FieldOutcome, TextArea, TextField, diff};
use crossterm::event::KeyEvent;

/// The save's `StoreRequest::name`, what `busy` holds while it is in flight.
const SAVE_NAME: &str = REQUEST_NAMES[1];

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// A template row's name field, in cells: a name is fitted to it with `cells::fit`, so the head
/// and the role stay on the row and in their columns.
const NAME_WIDTH: usize = 13;

/// The help column's width, borders included: `{{failure_reason}} section required`, the widest
/// placeholder line (a handoff's), is 35 chars.
const HELP_WIDTH: u16 = 38;

/// The pane before the first reply.
const NOT_READ: &str = "templates not read yet";

/// [`NOT_READ`]'s next step (MOD-115 M4 E5).
const NOT_READ_NEXT: &str = "they appear once the store answers";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "templates unavailable";

/// A version key on a project header row.
const SELECT_A_TEMPLATE: &str = "select a template";

/// [`SELECT_A_TEMPLATE`]'s next step, through `TEMPLATES_BROWSE`.
const SELECT_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "chooses one"),
    Hint::One(Act::New, "creates one"),
];

/// [`SELECT_HINT`] with every key unbound.
const SELECT_NEXT: &str = "a template row shows its body here";

/// `Ctrl+S` on a phase body that never places `{{item}}` (ANA-5 §4.1: warn, never refuse). `save`
/// is `FormSave`'s label through `TEMPLATES_EDITOR` (MOD-67 M6 D4); with none bound the sentence
/// says a save.
fn omits_item(save: Option<&str>) -> String {
    let warning = "this phase template never places {{item}}";
    match save {
        Some(save) => format!("{warning} \u{2014} {save} again saves anyway"),
        None => format!("{warning} \u{2014} saving again saves anyway"),
    }
}

/// `Esc` over a modified draft, the first time (OQ-6).
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A save went out.
const SAVING: &str = "saving\u{2026}";

/// Every Browse key (plan D13; MOD-67 M4 D9), through [`views::TEMPLATES_BROWSE`]: the shell's
/// activity pane's hint (M4 C8); the tree and the body show their own ([`LIST_HINT`],
/// [`DETAIL_HINT`]). `move`
/// after `j/k` is dropped: the row would be 104 cells with ` · ` separators (blueprint §1 item 1).
/// The view switch shows its first two chords, `h/l view` (MOD-67 M4 R1 L2): 99 cells.
const BROWSE_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::Pair(Act::SkillsPrevVersion, Act::SkillsNextVersion, "version"),
    Hint::One(Act::SkillsBase, "base"),
    Hint::One(Act::SkillsDiff, "diff"),
    Hint::One(Act::TemplatesDiffDefault, "default"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::SkillsEditExternally, "$EDITOR"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Reload, "reload"),
    Hint::Two(Act::SkillsSwitchView, "view"),
];

/// The tree pane's keys while browsing (MOD-115 M4 E5, Q2: a pane shows its own keys): the
/// tree's motion, as every list's hint leads with it, then a new template and a reload, 22 of
/// its 28 cells. The view switch is the chips' (L-skills repair 1).
const LIST_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "new"),
    Hint::One(Act::Reload, "reload"),
];

/// The body pane's keys while browsing (MOD-115 M4 E5): the shown template's; [`BROWSE_HINT`] is
/// all of them, on the shell's activity pane.
const DETAIL_HINT: HintSpec = &[
    Hint::Pair(Act::SkillsPrevVersion, Act::SkillsNextVersion, "version"),
    Hint::One(Act::SkillsBase, "base"),
    Hint::One(Act::SkillsDiff, "diff"),
    Hint::One(Act::TemplatesDiffDefault, "default"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::SkillsEditExternally, "$EDITOR"),
];

/// The pane's keys while its lines overflow it, through [`views::TEMPLATES_BROWSE`]: they lead
/// the detail pane's hints when it is focused (MOD-115 M4 E5; MOD-82 carried: no key literal in
/// a title).
const SCROLL_HINT: HintSpec = &[
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
    Hint::Pair(Act::PanePageUp, Act::PanePageDown, "page"),
];

/// The hint row while naming a new template, through [`views::TEMPLATES_PROMPT`]: the field's
/// `field.submit` in force (MOD-115 M4 MD-A) and its `Esc`, never rebound (M2 MD7).
const NAMING_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "create"),
    Hint::Text("Esc cancel"),
];

/// The hint row in the editor, through [`views::TEMPLATES_EDITOR`], after the cursor's
/// `L{line}:C{col}` (D20). `skills.ask_agent` is MOD-55's `Ctrl+G` (MOD-67 M4 D9). Save and the
/// way out lead: the draft pane beside the placeholders holds only some of the keys at 100
/// columns, and [`draft_hint`](TemplatesView::draft_hint) drops them from the end (MOD-115 M4
/// L-skills repair 1).
const EDIT_HINT: HintSpec = &[
    Hint::One(Act::FormSave, "save"),
    Hint::Text("Esc cancel"),
    Hint::One(Act::SkillsAskAgent, "ask agent"),
    Hint::One(Act::FormExternalEditor, "$EDITOR"),
];

/// The cells of a kit pane's width its bottom border keeps for hints: they start two cells in
/// and stop one rule cell short of the corner (`kit::pane`).
const HINT_MARGIN: u16 = 4;

/// The hint row while a `Ctrl+E` handoff holds the draft: `$EDITOR` has the keys.
const HANDED_OFF_HINT: HintSpec = &[Hint::Text("the draft is in $EDITOR")];

/// The review phase's wire contract, the one a reviewer's output is parsed by (ANA-5 `:1323-1331`).
const REVIEW_WIRE: &str = "wire: first 3 lines `---` / `verdict: approve|request-changes` / `---`";

/// The judge's wire contract (ANA-5 `:1333-1344`).
const JUDGE_WIRE: &str = "wire: end with one ```json block {winner, reasons}";

/// `TemplatesStale` over an open editor (plan D11). The view's own sentence, not the Settings
/// tab's `changed_elsewhere(retry)`, which names its editor's retry key, not this one's. `save` is
/// `FormSave`'s label through `TEMPLATES_EDITOR` (MOD-67 M6 D4); with none bound the sentence
/// says what saving writes.
fn template_changed_elsewhere(head: i32, save: Option<&str>) -> String {
    let next = head + 1;
    match save {
        Some(save) => format!(
            "saved elsewhere since you opened it \u{2014} v{head} is now the latest; your draft is \
             kept and {save} saves it as v{next}"
        ),
        None => format!(
            "saved elsewhere since you opened it \u{2014} v{head} is now the latest; your draft is \
             kept and saving writes it as v{next}"
        ),
    }
}

/// MOD-59 D5: a save that applied although its re-read failed. It landed; the view still draws
/// what it held, and `reload` (`Reload`'s label through `TEMPLATES_BROWSE`, MOD-67 M6 D4) reads
/// again; with none bound the clause goes.
fn landed_unread(landed: &str, why: &str, reload: Option<&str>) -> String {
    match reload {
        Some(reload) => format!("{landed} \u{2014} the re-read failed, {reload} reloads: {why}"),
        None => format!("{landed} \u{2014} the re-read failed: {why}"),
    }
}

/// The Templates view. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct TemplatesView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<TemplatesSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale tree.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](TemplatesView::rows).
    cursor: usize,
    /// The version shown for the selected name; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against, once `b` or `D` chose it; `None` is the shown version's
    /// predecessor.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// The pane's first drawn row (`J`/`K`, `PageDown`/`PageUp`). Back to the top whenever the
    /// pane shows something else: another row, another version, the other pane.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](TemplatesView::scroll) clamps against. A
    /// `Cell` for [`page`](TemplatesView::page)'s reason.
    pane_rows: Cell<usize>,
    /// Browsing, naming a new template, or editing one.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`. One at a time (`settings/prompt.rs`'s rule):
    /// the staleness index keeps only the newest request of a kind.
    busy: Option<&'static str>,
    /// The last outcome, on the view's last rows.
    notice: Option<Notice>,
    /// What `E`/`Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height: what `PageUp`/`PageDown` move by. A `Cell` because the
    /// height is known only in `render(&self)`.
    page: Cell<u16>,
}

/// One row of the tree, derived from the snapshot on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A project header: its slug.
    Project(ProjectId),
    /// One template name, all its versions.
    Template {
        /// Whose.
        project: ProjectId,
        /// Which.
        name: String,
    },
}

impl Row {
    /// The project the row belongs to.
    fn project(&self) -> ProjectId {
        match self {
            Self::Project(project) | Self::Template { project, .. } => *project,
        }
    }
}

/// What `d` diffs the shown version against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    /// One stored version (`b`).
    Version(i32),
    /// The compiled default, `prompt::body_of` (`D`, OQ-7).
    Default,
}

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The diff from the base to the shown version.
    Diff,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the tree.
    #[default]
    Browse,
    /// Typing a new template's name (`n`).
    Naming {
        /// The project the new name goes into.
        project: ProjectId,
        /// The name.
        field: TextField,
    },
    /// The in-app editor (`e`, `Enter` on a new name, an `$EDITOR` return).
    Editing(Editor),
}

/// An open editor. Never `Debug`s a body.
struct Editor {
    /// The template's project.
    project: ProjectId,
    /// The template's name; its role is `TemplateRole::of_name(name)`.
    name: String,
    /// The head version when the editor opened: the compare-and-set token (`None`: a new name).
    token: Option<i32>,
    /// The version the draft started from, for the title (`None`: a new name).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// The phase body's missing `{{item}}` was warned about; the next `Ctrl+S` saves anyway.
    confirm_item: bool,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// The body the save in flight carries: what tells a draft typed on since from the one that
    /// was saved.
    sent: Option<String>,
    /// MOD-55: the agent help, while open; the draft is locked under it. Boxed: it would
    /// otherwise set the size of every [`Mode`].
    help: Option<Box<AgentHelp>>,
}

impl Editor {
    /// An editor over `text` (line ends normalised by `TextArea::with_text`), cursor at byte 0.
    fn new(
        project: ProjectId,
        name: String,
        token: Option<i32>,
        from: Option<i32>,
        text: &str,
    ) -> Self {
        let area = TextArea::with_text(text);
        Self {
            project,
            name,
            token,
            from,
            original: area.text().to_owned(),
            area,
            confirm_item: false,
            esc_armed: false,
            sent: None,
            help: None,
        }
    }
}

/// Lengths, never the text: `original` and `sent` are the body, as the draft is (`TextArea`'s
/// rule).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("project", &self.project)
            .field("name", &self.name)
            .field("token", &self.token)
            .field("from", &self.from)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("confirm_item", &self.confirm_item)
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
            .field("help", &self.help)
            .finish()
    }
}

/// An `$EDITOR` handoff in flight.
///
/// The editor travels with it: `Edited` opens it on the returned text, and a `Ctrl+E` handoff
/// that came back with nothing puts it back as it was.
#[derive(Debug)]
struct Pending {
    /// The editor the outcome opens, or returns to.
    editor: Editor,
    /// Whether the handoff came from the in-app editor (`Ctrl+E`) rather than from Browse (`E`).
    resume: bool,
}

/// One line of report.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`: something the user has to act on.
    Error(String),
}

impl From<Report> for Notice {
    fn from(report: Report) -> Self {
        match report {
            Report::Info(text) => Self::Info(text),
            Report::Error(text) => Self::Error(text),
        }
    }
}

/// Where a `parse` error points: the opening `{{` for the three that have one, and `None` for
/// `MissingRequired`, whose mistake is the body as a whole (the cursor then goes to the end).
fn error_at(err: &TemplateError) -> Option<usize> {
    match err {
        TemplateError::UnknownPlaceholder { at, .. }
        | TemplateError::WrongRole { at, .. }
        | TemplateError::Unterminated { at } => Some(*at),
        TemplateError::MissingRequired { .. } => None,
    }
}

impl TemplatesView {
    /// A bracketed paste into the name prompt or the editor (MOD-22 review M-1); `false` when
    /// nothing here is taking text.
    pub(super) fn on_paste(&mut self, text: &str) -> bool {
        match &mut self.mode {
            Mode::Browse => return false,
            Mode::Naming { field, .. } => {
                field.on_paste(text);
            }
            Mode::Editing(editor) => {
                // MOD-55: an open help takes the paste (its request field, or nothing); the draft
                // under it is locked.
                if let Some(help) = editor.help.as_mut() {
                    help.on_paste(text);
                    return true;
                }
                editor.area.on_paste(text);
                editor.confirm_item = false;
                editor.esc_armed = false;
                self.notice = None;
            }
        }
        true
    }

    /// The browse mode's stack, whatever mode the view is in (MOD-115 review).
    pub(super) const fn browse_stack() -> Stack<'static> {
        views::TEMPLATES_BROWSE
    }

    /// The one place a mode maps to its stack (MOD-67 M4 D4, blueprint §6.5), always one: an
    /// open agent help's, else the mode's (a handed-off draft stays on the browse stack).
    /// `SkillsTab::key_stack`, `on_key` and the hint read it.
    pub(super) fn key_stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::TEMPLATES_BROWSE,
            Mode::Naming { .. } => views::TEMPLATES_PROMPT,
            Mode::Editing(editor) => editor
                .help
                .as_ref()
                .map_or(views::TEMPLATES_EDITOR, |help| help.key_stack()),
        }
    }

    /// Whether an editor or the name prompt is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// The scope changed: the tree, the editor, the pending handoff and the write in flight all
    /// belong to the workspace that was left. The notice survives, as in `settings/prompt.rs`,
    /// because the scope change is often the consequence of what it reports.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = self.key_stack();
        match self.mode {
            Mode::Browse => self.on_browse_key(key, stack, ctx),
            Mode::Naming { .. } => self.on_naming_key(key, stack, ctx),
            Mode::Editing(_) => self.on_editor_key(key, stack, ctx),
        }
    }

    /// A reply addressed to the Skills tab.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        // MOD-55: the open help sees every reply first (A-1: its per-state table is what makes a
        // stray frame harmless). Its frames match none of the arms below.
        if let Mode::Editing(editor) = &mut self.mode
            && let Some(help) = editor.help.as_mut()
        {
            let outcome = help.on_reply(reply, ctx);
            if outcome != HelpOutcome::Consumed {
                self.apply_help(outcome, ctx.keys());
            }
        }
        match reply {
            StoreReply::Templates(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                // A read never lands a save (MOD-59 D4): only the save's own reply does.
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                self.clamp_cursor();
            }
            StoreReply::TemplateSaved {
                snapshot,
                project,
                name,
                version,
            } => match snapshot {
                Ok(snapshot) => {
                    if !in_scope(snapshot, ctx) {
                        return;
                    }
                    self.snapshot = Some((**snapshot).clone());
                    self.unavailable = None;
                    self.land_save(*project, name, *version, None, ctx.keys());
                    self.clamp_cursor();
                }
                // No snapshot to check the scope by: only the save in flight owns this reply, and
                // a scope change has already forgotten that (`on_scope_change`).
                Err(why) if self.busy == Some(SAVE_NAME) => {
                    if self.snapshot.is_none() {
                        self.unavailable = Some(why.clone());
                    }
                    self.land_save(*project, name, *version, Some(why), ctx.keys());
                }
                Err(_) => {}
            },
            StoreReply::TemplatesStale(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy == Some(SAVE_NAME) {
                    self.busy = None;
                    if let Mode::Editing(editor) = &mut self.mode {
                        editor.sent = None;
                        if let Some(head) = snapshot.head(editor.project, &editor.name) {
                            // The draft stays; the token moves to the head the user has now been
                            // told about, so the next `Ctrl+S` is a deliberate overwrite-by-append.
                            editor.token = Some(head.version);
                            let save = ctx.keys().label(views::TEMPLATES_EDITOR, Act::FormSave);
                            self.notice = Some(Notice::Error(template_changed_elsewhere(
                                head.version,
                                save.as_deref(),
                            )));
                        }
                    }
                }
                self.clamp_cursor();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                // A refused save leaves the editor over its text: nothing was written.
                self.busy = None;
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.sent = None;
                }
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    /// The `$EDITOR` handoff came back (plan D11, OQ-3). No handoff pending (the scope changed
    /// meanwhile): ignored.
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        let Some(Pending { mut editor, resume }) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(text) => {
                // The `Ctrl+S` gate minus the send: the cursor goes to the error, else to 0. The
                // gate parses the draft as the area holds it (line ends normalised), so the
                // error's byte is a byte of that text.
                editor.area = TextArea::with_text(&text);
                let text = editor.area.text().to_owned();
                editor.confirm_item = false;
                editor.esc_armed = false;
                self.notice = Some(match parse(TemplateRole::of_name(&editor.name), &text) {
                    Err(err) => {
                        editor.area.set_cursor(error_at(&err).unwrap_or(text.len()));
                        Notice::Error(err.to_string())
                    }
                    Ok(_) => Notice::Info(edited(
                        ctx.keys()
                            .label(views::TEMPLATES_EDITOR, Act::FormSave)
                            .as_deref(),
                    )),
                });
                self.mode = Mode::Editing(editor);
            }
            ExternalEditOutcome::Unchanged { quick } => {
                if resume {
                    self.mode = Mode::Editing(editor);
                }
                let wait = if quick { WAIT_FLAG } else { "" };
                self.notice = Some(Notice::Info(format!("{NO_CHANGES}{wait}")));
            }
            ExternalEditOutcome::Failed(message) => {
                if resume {
                    self.mode = Mode::Editing(editor);
                }
                self.notice = Some(Notice::Error(message));
            }
        }
    }

    /// Draws the view over the tab's body: the content and the notice row. Its keys are
    /// [`key_hint`](Self::key_hint), drawn on the pane that holds the keyboard (MOD-115 M4 C4).
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The notice wraps rather than clips (at most `NOTICE_LINES`): the stale sentence ends on
        // the version the next `Ctrl+S` writes, which is the one part the user must not lose.
        let (notice_text, tone) = match &self.notice {
            Some(Notice::Info(text)) => (text.as_str(), Tone::Info),
            Some(Notice::Error(text)) => (text.as_str(), Tone::Error),
            None => ("", Tone::Info),
        };
        let mut rows = wrapped(
            notice_text,
            usize::from(area.width.saturating_sub(1)).max(1),
        );
        rows.truncate(NOTICE_LINES);
        let notice_height = u16::try_from(rows.len().max(1)).unwrap_or(1);
        let [content, notice_row] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(notice_height)]).areas(area);
        match (&self.mode, self.handed_off()) {
            (Mode::Editing(editor), _) | (Mode::Browse, Some(editor)) => {
                self.render_editor(frame, content, editor, ctx);
            }
            (Mode::Naming { .. } | Mode::Browse, _) => self.render_browse(frame, content, ctx),
        }
        frame.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .map(|line| notice(&format!(" {line}"), tone, ctx.theme))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
    }

    /// The keys of the shown mode (MOD-67 M6 T2), drawn on the pane that holds the keyboard and,
    /// for Browse, on the shell's activity pane (MOD-115 M4 C4, C8), through
    /// [`key_stack`](Self::key_stack): an open agent help's, the editor's with the cursor's
    /// `L:C` first, else the mode's.
    pub(super) fn key_hint(&self, keys: &Keys) -> String {
        match (&self.mode, self.handed_off()) {
            (Mode::Editing(editor), _) => match &editor.help {
                Some(help) => help.hint(keys),
                None => {
                    // MOD-115 M4 E5: the cursor's `L:C` leads, so a pane narrower than the row
                    // was (the Templates draft beside its placeholders) still shows it.
                    let (line, col) = editor.area.cursor_line_col();
                    joined(&[
                        format!("L{}:C{}", line + 1, col + 1),
                        keys.hint(self.key_stack(), EDIT_HINT),
                    ])
                }
            },
            (Mode::Browse, Some(_)) => keys.hint(self.key_stack(), HANDED_OFF_HINT),
            (Mode::Naming { .. }, _) => keys.hint(self.key_stack(), NAMING_HINT),
            (Mode::Browse, None) => keys.hint(self.key_stack(), BROWSE_HINT),
        }
    }

    /// The editor's keys fitted to the draft pane `width` cells wide (MOD-115 M4 L-skills repair
    /// 1): [`key_hint`](Self::key_hint)'s whole row when it fits or an agent help has the keys,
    /// else the cursor's `L:C` and as many of [`EDIT_HINT`]'s keys, in order and whole, as the
    /// bottom border holds; the first that does not fit goes with every one after it, and ` ·…`
    /// says so. Never half a key.
    fn draft_hint(&self, editor: &Editor, keys: &Keys, width: u16) -> String {
        let full = self.key_hint(keys);
        let room = usize::from(width.saturating_sub(HINT_MARGIN));
        if editor.help.is_some() || cells::cell_width(&full) <= room {
            return full;
        }
        let (line, col) = editor.area.cursor_line_col();
        let at = format!("L{}:C{}", line + 1, col + 1);
        (0..EDIT_HINT.len())
            .rev()
            .map(|kept| {
                let fit = joined(&[at.clone(), keys.hint(self.key_stack(), &EDIT_HINT[..kept])]);
                format!("{fit} \u{b7}\u{2026}")
            })
            .find(|cut| cells::cell_width(cut) <= room)
            .unwrap_or(full)
    }

    // --- keys ----------------------------------------------------------------------------------

    /// Browse (plan D13; MOD-67 M4, blueprint §6.1): the first candidate through
    /// [`views::TEMPLATES_BROWSE`] this view acts on; a global act is the shell's. The tab took
    /// `skills.switch_view` first.
    ///
    /// MOD-115 M2 §7: only motion follows the focus ([`Step::by_focus`]): the tree's cursor
    /// with the tree focused, the pane's scroll with the pane focused, nothing with the activity
    /// pane focused. Every other act acts whatever pane is focused.
    fn on_browse_key(
        &mut self,
        key: KeyEvent,
        stack: Stack<'static>,
        ctx: &mut Ctx<'_>,
    ) -> Handled {
        let chord = ctx.pressed(&key);
        let (focus, rows) = (ctx.focus(), self.pane_rows.get());
        for act in ctx.keys().actions(stack, chord) {
            if Step::by_focus(act, focus, |step| self.scroll.follow(step, rows)) {
                return Handled::Consumed;
            }
            match act {
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::Reload => {
                    self.notice = None;
                    ctx.request(StoreRequest::Templates(ctx.scope.clone()));
                }
                Act::New => self.open_naming(),
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.pane_rows.get());
                }
                Act::SkillsPrevVersion
                | Act::SkillsNextVersion
                | Act::SkillsBase
                | Act::SkillsDiff
                | Act::TemplatesDiffDefault
                | Act::Edit
                | Act::SkillsEditExternally => {
                    self.notice = None;
                    match self.selected_template() {
                        Some((project, name)) => self.on_template_key(act, project, name, ctx),
                        None if !self.rows().is_empty() => {
                            self.notice = Some(Notice::Info(SELECT_A_TEMPLATE.to_owned()));
                        }
                        None => {}
                    }
                }
                _ => continue, // a global act
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// A version, diff or edit act with a template under the cursor.
    fn on_template_key(&mut self, act: Act, project: ProjectId, name: String, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let versions: Vec<&PromptTemplate> = snapshot
            .projects
            .iter()
            .filter(|entry| entry.project_id == project)
            .flat_map(|entry| entry.templates.iter())
            .filter(|row| row.name == name)
            .collect();
        let (Some(head), Some(shown)) = (
            snapshot.head(project, &name),
            self.shown_row(snapshot, project, &name),
        ) else {
            return;
        };
        let (head, shown_version, shown_body) = (head.version, shown.version, shown.body.clone());
        let index = versions
            .iter()
            .position(|row| row.version == shown_version)
            .unwrap_or(0);
        let has_earlier = versions.iter().any(|row| row.version == shown_version - 1);
        match act {
            Act::SkillsPrevVersion | Act::SkillsNextVersion => {
                let index = if act == Act::SkillsPrevVersion {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(versions.len().saturating_sub(1))
                };
                let version = versions.get(index).map_or(head, |row| row.version);
                self.shown = (version != head).then_some(version);
                self.scroll.reset();
            }
            Act::SkillsBase => {
                self.base = Some(DiffBase::Version(shown_version));
                self.notice = Some(Notice::Info(format!("base v{shown_version}")));
            }
            Act::SkillsDiff => {
                if self.pane == Pane::Diff {
                    self.pane = Pane::Body;
                    self.scroll.reset();
                } else if self.base.is_none() && !has_earlier {
                    self.notice = Some(Notice::Info(format!(
                        "v{shown_version} has no earlier version"
                    )));
                } else {
                    self.pane = Pane::Diff;
                    self.scroll.reset();
                }
            }
            Act::TemplatesDiffDefault => {
                if body_of(&name).is_some() {
                    self.base = Some(DiffBase::Default);
                    self.pane = Pane::Diff;
                    self.scroll.reset();
                } else {
                    self.notice = Some(Notice::Info(format!("`{name}` has no compiled default")));
                }
            }
            Act::Edit => {
                let editor =
                    Editor::new(project, name, Some(head), Some(shown_version), &shown_body);
                self.mode = Mode::Editing(editor);
            }
            Act::SkillsEditExternally => {
                ctx.emit(Action::EditExternally(ExternalEdit {
                    text: shown_body.clone(),
                    stem: name.clone(),
                }));
                let editor =
                    Editor::new(project, name, Some(head), Some(shown_version), &shown_body);
                self.external = Some(Pending {
                    editor,
                    resume: false,
                });
            }
            _ => {}
        }
    }

    /// `n`: the name prompt, for the project of the row under the cursor.
    fn open_naming(&mut self) {
        let Some(project) = self.rows().get(self.cursor).map(Row::project) else {
            return;
        };
        self.notice = None;
        self.scroll.reset();
        self.mode = Mode::Naming {
            project,
            field: TextField::new(),
        };
    }

    /// The name prompt. A refused name keeps the prompt open, so a typo is one `Backspace` away.
    /// The field answers first; what it passes resolves through [`views::TEMPLATES_PROMPT`]:
    /// `global.next_tab`/`prev_tab` go to the shell with the prompt kept (MOD-67 M4 PA-2), the
    /// rest is `modal_rest`'s.
    fn on_naming_key(&mut self, key: KeyEvent, stack: Stack<'static>, ctx: &Ctx<'_>) -> Handled {
        let Mode::Naming { project, field } = &mut self.mode else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                match ctx.keys().actions(stack, chord).first() {
                    Some(Act::NextTab | Act::PrevTab) => Handled::Pass,
                    _ => modal_rest(stack, chord),
                }
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                let (project, name) = (*project, field.text().unwrap_or_default().to_owned());
                self.create(project, name, ctx.keys());
                Handled::Consumed
            }
        }
    }

    /// `Enter` on a new name (plan D13): an invalid or taken name is refused; otherwise the editor
    /// opens on the compiled default of that name, or empty, with no token. A taken name sends the
    /// user to the browse edit key `keys` binds (MOD-67 M6 D4).
    fn create(&mut self, project: ProjectId, name: String, keys: &Keys) {
        if !PromptTemplate::name_is_valid(&name) {
            self.notice = Some(Notice::Error(invalid_template_name(&name)));
            return;
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.head(project, &name).is_some())
        {
            self.notice = Some(Notice::Error(
                match keys.label(views::TEMPLATES_BROWSE, Act::Edit) {
                    Some(edit) => format!("`{name}` exists \u{2014} select it and press {edit}"),
                    None => format!("`{name}` exists"),
                },
            ));
            return;
        }
        let body = body_of(&name).unwrap_or("");
        self.notice = None;
        self.mode = Mode::Editing(Editor::new(project, name, None, None, body));
    }

    /// The editor (plan D11, D13; MOD-67 M4 D6, blueprint §6.2): the area answers first, so
    /// every letter is text and `Esc` is its `Cancel`; every chord it passes, `ctrl-s` included
    /// (D6), resolves through
    /// [`views::TEMPLATES_EDITOR`]: `form.save`, `skills.ask_agent` and `form.external_editor`
    /// are the view's (PA-3: after the widget), `global.next_tab`/`prev_tab` pass so the shell
    /// switches tabs with the draft kept (PA-2), the rest is `modal_rest`'s. An open agent help
    /// (MOD-55) takes every key first: the draft is locked under it, and those three verbs are
    /// refused until it closes.
    fn on_editor_key(
        &mut self,
        key: KeyEvent,
        stack: Stack<'static>,
        ctx: &mut Ctx<'_>,
    ) -> Handled {
        if let Mode::Editing(editor) = &mut self.mode
            && let Some(help) = editor.help.as_mut()
        {
            let outcome = help.on_key(key, ctx);
            return self.apply_help(outcome, ctx.keys());
        }
        let busy = self.busy;
        let page = self.page.get();
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        let before = editor.area.text().len();
        match editor.area.on_key(key, page) {
            FieldOutcome::Consumed => {
                if editor.area.text().len() != before {
                    editor.confirm_item = false;
                    editor.esc_armed = false;
                    self.notice = None;
                }
                return Handled::Consumed;
            }
            FieldOutcome::Cancel => {
                if let Some(busy) = busy {
                    // The save's reply closes the editor or keeps it; leaving now would leave the
                    // reply with no editor to land on and `busy` with nothing to clear it.
                    self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
                } else if editor.esc_armed || editor.area.text() == editor.original {
                    self.mode = Mode::Browse;
                    self.notice = None;
                } else {
                    editor.esc_armed = true;
                    self.notice = Some(Notice::Info(UNSAVED.to_owned()));
                }
                return Handled::Consumed;
            }
            // A `TextArea` never submits since MOD-67 M4 D6: `ctrl-s` passes, and only
            // `form.save` below saves.
            FieldOutcome::Submit | FieldOutcome::Pass => {}
        }
        let chord = ctx.pressed(&key);
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::FormSave) => self.save(ctx),
            Some(Act::SkillsAskAgent) => self.open_help(ctx),
            Some(Act::FormExternalEditor) => self.hand_off(ctx),
            Some(Act::NextTab | Act::PrevTab) => return Handled::Pass,
            _ => return modal_rest(stack, chord),
        }
        Handled::Consumed
    }

    /// `Ctrl+S` (plan D11), first match wins: a write in flight; a `parse` refusal, with the
    /// cursor on its byte (or at the end for a missing required placeholder) and nothing sent; a
    /// phase body without `{{item}}`, once; otherwise the save.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let role = TemplateRole::of_name(&editor.name);
        match parse(role, editor.area.text()) {
            Err(err) => {
                let at = error_at(&err).unwrap_or(editor.area.text().len());
                editor.area.set_cursor(at);
                self.notice = Some(Notice::Error(err.to_string()));
            }
            // `omits_item` is true of every judge and handoff body, hence the role guard.
            Ok(parsed)
                if role == TemplateRole::Phase && parsed.omits_item() && !editor.confirm_item =>
            {
                editor.confirm_item = true;
                self.notice = Some(Notice::Info(omits_item(
                    ctx.keys()
                        .label(views::TEMPLATES_EDITOR, Act::FormSave)
                        .as_deref(),
                )));
            }
            Ok(_) => {
                let body = editor.area.text().to_owned();
                editor.sent = Some(body.clone());
                self.busy = Some(SAVE_NAME);
                self.notice = Some(Notice::Info(SAVING.to_owned()));
                ctx.request(StoreRequest::SaveTemplate {
                    scope: ctx.scope.clone(),
                    project: editor.project,
                    name: editor.name.clone(),
                    body: TemplateBody::new(body),
                    expected: editor.token,
                });
            }
        }
    }

    /// `Ctrl+E`: the draft goes to `$EDITOR`, and the editor waits in the pending handoff. Not
    /// while a save is in flight, for `Esc`'s reason.
    fn hand_off(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = core::mem::take(&mut self.mode) else {
            return;
        };
        ctx.emit(Action::EditExternally(ExternalEdit {
            text: editor.area.text().to_owned(),
            stem: editor.name.clone(),
        }));
        self.external = Some(Pending {
            editor,
            resume: true,
        });
    }

    /// The draft a `Ctrl+E` handed to `$EDITOR`, while the handoff is pending: still drawn, so its
    /// text rect is still claimed for an in-pane editor (MOD-57 P2). Browse's `E` has no draft.
    fn handed_off(&self) -> Option<&Editor> {
        match &self.external {
            Some(Pending {
                editor,
                resume: true,
            }) => Some(editor),
            _ => None,
        }
    }

    /// `Ctrl+G` (MOD-55 P7, P8): the agent help opens on the draft as it is, for the template's
    /// own project. Not while a save is in flight, for `Esc`'s reason.
    fn open_help(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        editor.help = Some(Box::new(AgentHelp::open(
            HelpTarget::Template {
                name: editor.name.clone(),
            },
            editor.project,
            editor.area.text(),
            ctx,
        )));
        editor.esc_armed = false;
        self.notice = None;
    }

    /// What the open help's key or reply did. An accepted proposal replaces the draft through the
    /// `$EDITOR` return's gate (`on_external_edit`'s `Edited` arm): `parse` runs, the cursor goes
    /// to its error, and nothing is sent. `original` is left alone, so `Esc` still asks first.
    fn apply_help(&mut self, outcome: HelpOutcome, keys: &Keys) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        match outcome {
            HelpOutcome::Consumed => {}
            HelpOutcome::Pass => return Handled::Pass,
            HelpOutcome::Note(report) => self.notice = Some(report.into()),
            HelpOutcome::Close(report) => {
                editor.help = None;
                editor.esc_armed = false;
                self.notice = report.map(Notice::from);
            }
            HelpOutcome::Accept(text) => {
                editor.help = None;
                editor.area = TextArea::with_text(&text);
                let text = editor.area.text().to_owned();
                editor.confirm_item = false;
                editor.esc_armed = false;
                self.notice = Some(match parse(TemplateRole::of_name(&editor.name), &text) {
                    Err(err) => {
                        editor.area.set_cursor(error_at(&err).unwrap_or(text.len()));
                        Notice::Error(err.to_string())
                    }
                    Ok(_) => Notice::Info(accepted(
                        keys.label(views::TEMPLATES_EDITOR, Act::FormSave)
                            .as_deref(),
                    )),
                });
            }
        }
        Handled::Consumed
    }

    // --- state ---------------------------------------------------------------------------------

    /// The flat list the cursor indexes: per project in snapshot order, a header, then one row per
    /// distinct name in byte order (the read's order). Derived, so it cannot disagree with the
    /// snapshot.
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for entry in &snapshot.projects {
            rows.push(Row::Project(entry.project_id));
            let mut last: Option<&str> = None;
            for row in &entry.templates {
                if last != Some(row.name.as_str()) {
                    rows.push(Row::Template {
                        project: entry.project_id,
                        name: row.name.clone(),
                    });
                    last = Some(row.name.as_str());
                }
            }
        }
        rows
    }

    /// The template under the cursor, if the cursor is on one.
    fn selected_template(&self) -> Option<(ProjectId, String)> {
        match self.rows().into_iter().nth(self.cursor)? {
            Row::Template { project, name } => Some((project, name)),
            Row::Project(_) => None,
        }
    }

    /// The version shown for `(project, name)`: the pinned one, else the head. A pin the snapshot
    /// no longer holds falls back to the head.
    fn shown_row<'a>(
        &self,
        snapshot: &'a TemplatesSnapshot,
        project: ProjectId,
        name: &str,
    ) -> Option<&'a PromptTemplate> {
        self.shown
            .and_then(|version| snapshot.version(project, name, version))
            .or_else(|| snapshot.head(project, name))
    }

    /// `j`/`k`: one row, no wrap; the version, the base and the pane go back to the head's body,
    /// from its top.
    fn move_cursor(&mut self, down: bool) {
        let last = self.rows().len().saturating_sub(1);
        self.cursor = if down {
            (self.cursor + 1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        self.notice = None;
    }

    /// The row under the cursor, for the tab's history location (MOD-115 M2 §8): its index and
    /// what it holds (RV3), `{index}:{project}:{name}` on a template, `{index}:{project}` on a
    /// project header, so Back finds the row again after a create moved it.
    pub(super) fn location_cursor(&self) -> String {
        match self.rows().into_iter().nth(self.cursor) {
            Some(Row::Template { project, name }) => format!("{}:{project}:{name}", self.cursor),
            Some(Row::Project(project)) => format!("{}:{project}", self.cursor),
            None => self.cursor.to_string(),
        }
    }

    /// MOD-115 M2 §8: the cursor back on a [`location_cursor`](Self::location_cursor): onto its
    /// template or header wherever the tree now holds it, else on its row (clamped to the
    /// tree), the head's body shown from its top, as a move shows it. Only while browsing with
    /// no draft handed off: the name prompt and an editor keep their row. Whether it moved; a
    /// cursor with no row moves nothing.
    pub(super) fn restore_cursor(&mut self, cursor: &str) -> bool {
        let mut parts = cursor.splitn(3, ':');
        let Some(Ok(index)) = parts.next().map(str::parse::<usize>) else {
            return false;
        };
        let held = match (parts.next().map(str::parse), parts.next()) {
            (Some(Ok(project)), Some(name)) => Some(Row::Template {
                project,
                name: name.to_owned(),
            }),
            (Some(Ok(project)), None) => Some(Row::Project(project)),
            _ => None,
        };
        if !matches!(self.mode, Mode::Browse) || self.handed_off().is_some() {
            return false;
        }
        let rows = self.rows();
        self.cursor = held
            .and_then(|held| rows.iter().position(|row| *row == held))
            .unwrap_or_else(|| index.min(rows.len().saturating_sub(1)));
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        true
    }

    /// Keeps the cursor on a row after the tree changed.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// MOD-59 D4: the save in flight landed. `project`, `name` and `version` are the store's own
    /// answer, so nothing here searches the snapshot for the body sent; every plain `Templates`
    /// read leaves the editor, the token and `busy` alone. `unread` is the re-read's failure (D5):
    /// the version was appended, and the notice says the tree drawn is the one held, which may not
    /// show a new name yet (the cursor then stays where it was). A `TemplateSaved` answers
    /// `SaveTemplate` alone, so the request name `busy` must hold for it to land is `SAVE_NAME`
    /// (review L3: every view's landing checks `busy` itself).
    ///
    /// Keys typed while the save was in flight still edit the draft. When they did, the editor
    /// stays open on them with the token at the saved version, so the next `Ctrl+S` appends them.
    fn land_save(
        &mut self,
        project: ProjectId,
        name: &str,
        version: i32,
        unread: Option<&str>,
        keys: &Keys,
    ) {
        if self.busy != Some(SAVE_NAME) {
            return;
        }
        self.busy = None;
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        let saved = Row::Template {
            project,
            name: name.to_owned(),
        };
        if let Some(index) = self.rows().iter().position(|row| *row == saved) {
            self.cursor = index;
        }
        let landed = match &mut self.mode {
            Mode::Editing(editor) => {
                let sent = editor.sent.take().unwrap_or_default();
                if editor.area.text() == sent {
                    self.mode = Mode::Browse;
                    format!("saved v{version}")
                } else {
                    editor.token = Some(version);
                    editor.from = Some(version);
                    editor.original = sent;
                    editor.confirm_item = false;
                    editor.esc_armed = false;
                    match keys.label(views::TEMPLATES_EDITOR, Act::FormSave) {
                        Some(save) => format!(
                            "saved v{version} \u{2014} later edits kept, {save} saves them as v{}",
                            version + 1
                        ),
                        None => format!("saved v{version} \u{2014} later edits kept"),
                    }
                }
            }
            Mode::Browse | Mode::Naming { .. } => format!("saved v{version}"),
        };
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(
                &landed,
                why,
                keys.label(views::TEMPLATES_BROWSE, Act::Reload).as_deref(),
            )),
        });
    }

    // --- frames --------------------------------------------------------------------------------

    /// Browse and Naming: the tree on the left, the pane on the right, each a kit pane (MOD-115 M4
    /// E5). The tree leads the view, so it carries the view chips. The name prompt holds the
    /// keyboard, so the pane it is drawn in is the focused one (C4); otherwise the shell's focus
    /// decides. Each pane draws its hints only while focused (Q2).
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // MOD-115 M2 B5: the panes the shell's layout gives the tab, claimed for the focus, the
        // hidden-pane note and the mouse; today's 32-cell list by default. A hidden pane is an
        // empty rect, which draws nothing.
        let areas = ctx.split(area, super::library::LAYOUT);
        let prompt = !matches!(self.mode, Mode::Browse);
        // T7 repair: the name prompt takes every key, so with another pane zoomed the pane is
        // drawn over the zoomed one rather than into its own empty rect.
        let (list_area, pane_area) = capture_area(&areas, area, prompt);
        // The shell's activity pane is on screen unless it is the zoomed pane drawn over.
        let activity = areas.activity.is_some_and(|rect| !rect.is_empty())
            && !(areas.list.is_none() && areas.detail.is_empty());
        let (list_focus, pane_focus) = if prompt {
            (Focus::Unfocused, holder(ctx, activity))
        } else {
            (ctx.pane_focus(PaneId::List), ctx.pane_focus(PaneId::Detail))
        };
        if let Some(list_area) = list_area.filter(|rect| !rect.is_empty()) {
            self.render_tree(frame, list_area, list_focus, ctx);
        }
        if !pane_area.is_empty() {
            self.render_pane(frame, pane_area, pane_focus, prompt, ctx);
        }
    }

    /// The tree as a kit list (MOD-115 M4 E5): the project rows in the title role, the template
    /// rows after the gutter, the cursor the gutter and band (MOD-85), kept in view; the view
    /// chips on the top border.
    fn render_tree(&self, frame: &mut Frame<'_>, area: Rect, focus: Focus, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let inner = KitPane::inner(area);
        let rows: Vec<ListRow<'static>> = self
            .rows()
            .iter()
            .map(|row| {
                let line = match row {
                    Row::Project(project) => Line::styled(slug(ctx, *project), theme.title),
                    Row::Template { project, name } => {
                        let head = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.head(*project, name))
                            .map_or(0, |row| row.version);
                        let role = TemplateRole::of_name(name).as_str();
                        Line::styled(template_row(name, head, role), theme.base)
                    }
                };
                ListRow {
                    line,
                    kind: RowKind::Item,
                }
            })
            .collect();
        let selected = (!rows.is_empty()).then_some(self.cursor);
        let offset = list::scroll_into_view(self.cursor, 0, usize::from(inner.height), rows.len());
        list::render(frame, inner, &rows, selected, offset, ctx.accent(), theme);
        let hints = hint_line(ctx.keys().hint(views::TEMPLATES_BROWSE, LIST_HINT), ctx);
        framed(view_chips(KitPane::new(""), 1), focus, hints, ctx)
            .scrollbar(rows.len(), offset)
            .render(frame, area, theme);
    }

    /// The right-hand pane: the name prompt's form, an empty state, a refused read, or the shown
    /// body or diff, wrapped and scrolled with a scrollbar. Its title names no key (MOD-82
    /// carried): the scroll keys lead its hints while its lines overflow it.
    fn render_pane(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        focus: Focus,
        prompt: bool,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = KitPane::inner(area);
        let (title, detail) = self.detail(ctx);
        let mut pane = KitPane::new(&title);
        let mut overflow = false;
        self.pane_rows.set(0);
        match detail {
            Detail::Form(form) => form.render(frame, inner, 0, ctx.accent(), theme),
            Detail::Empty(what, next) => empty::render(frame, inner, what, NextStep(&next), theme),
            Detail::Unavailable(sentence) => unavailable(frame, inner, &sentence, theme),
            Detail::Lines(lines) => {
                // The body and the diff wrap: a default body's lines run to 150-200 columns, and
                // a change past the pane's edge would be off screen. The row count is a character
                // wrap's, a lower bound on the word wrap's (`Scroll`'s rule), so the clamp never
                // scrolls the pane blank.
                let width = usize::from(inner.width).max(1);
                let rows: usize = lines
                    .iter()
                    .map(|line| line.width().div_ceil(width).max(1))
                    .sum();
                self.pane_rows.set(rows);
                overflow = rows > usize::from(inner.height) || self.scroll.offset() > 0;
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .scroll((self.scroll.offset(), 0)),
                    inner,
                );
                pane = pane.scrollbar(rows, self.scroll.skip());
            }
        }
        let keys = ctx.keys();
        let hint = if prompt {
            self.key_hint(keys)
        } else {
            let scroll = if overflow {
                keys.hint(views::TEMPLATES_BROWSE, SCROLL_HINT)
            } else {
                String::new()
            };
            joined(&[scroll, keys.hint(views::TEMPLATES_BROWSE, DETAIL_HINT)])
        };
        framed(pane, focus, hint_line(hint, ctx), ctx).render(frame, area, theme);
    }

    /// The right-hand pane's title and what it shows: the name prompt, a refusal, the empty
    /// states, the shown body, or the diff.
    fn detail(&self, ctx: &Ctx<'_>) -> (String, Detail<'_>) {
        let theme = ctx.theme;
        if let Mode::Naming { project, field } = &self.mode {
            let form = Form::new()
                .field(Field::new(
                    "project",
                    Value::ReadOnly(slug(ctx, *project).into()),
                ))
                .field(Field::new("name", Value::Text(field)))
                .focus(1)
                .editing(true);
            return ("new template".to_owned(), Detail::Form(form));
        }
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                Detail::Unavailable(format!("{UNAVAILABLE}: {why}")),
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return (
                String::new(),
                Detail::Empty(NOT_READ, NOT_READ_NEXT.to_owned()),
            );
        };
        let select = || {
            let next = ctx.keys().hint(views::TEMPLATES_BROWSE, SELECT_HINT);
            let next = if next.is_empty() {
                SELECT_NEXT.to_owned()
            } else {
                next
            };
            (String::new(), Detail::Empty(SELECT_A_TEMPLATE, next))
        };
        let Some((project, name)) = self.selected_template() else {
            return select();
        };
        let (Some(head), Some(shown)) = (
            snapshot.head(project, &name),
            self.shown_row(snapshot, project, &name),
        ) else {
            return select();
        };
        if self.pane == Pane::Body {
            return (
                format!("{name} v{} (head v{})", shown.version, head.version),
                Detail::Lines(
                    shown
                        .body
                        .lines()
                        .map(|line| Line::styled(line.to_owned(), theme.base))
                        .collect(),
                ),
            );
        }
        let base = match self.base {
            Some(DiffBase::Default) => body_of(&name).map(|body| ("default".to_owned(), body)),
            Some(DiffBase::Version(version)) => snapshot
                .version(project, &name, version)
                .map(|row| (format!("v{version}"), row.body.as_str())),
            None => snapshot
                .version(project, &name, shown.version - 1)
                .map(|row| (format!("v{}", row.version), row.body.as_str())),
        };
        let Some((label, text)) = base else {
            return (
                format!("{name} v{}", shown.version),
                Detail::Lines(vec![Line::styled(
                    format!("v{} has no base to diff against", shown.version),
                    theme.dim,
                )]),
            );
        };
        let unified = diff::unified(
            text,
            &shown.body,
            &format!("{name} {label}"),
            &format!("{name} v{}", shown.version),
        );
        (
            format!("diff {label} \u{2192} v{}", shown.version),
            Detail::Lines(diff::lines(&unified, theme)),
        )
    }

    /// The editor: the draft on the left, the role's placeholders on the right (plan D14). An
    /// open agent help draws in the draft's place (MOD-55 B-2): a panel under a locked draft, or
    /// the proposal over all of it; the placeholder column stays, to check a proposal against.
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let [left, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(HELP_WIDTH)]).areas(area);
        let draft = match &editor.help {
            Some(help) => help.render(frame, left, ctx, 1),
            None => Some(left),
        };
        if let Some(draft) = draft {
            self.render_draft(frame, draft, editor, ctx);
        }
        self.render_placeholders(frame, right, editor, ctx);
    }

    /// The draft's pane: the name and the versions as its title, the view chips while the whole
    /// title fits beside them (`view_chips_if_room`), and the text with its cursor (dim under an
    /// open help, which has the keys and the focus). The text's rect is claimed for an in-pane
    /// editor (MOD-57 P2), inside the pane, so the name and versions stay visible beside it.
    fn render_draft(&self, frame: &mut Frame<'_>, left: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let title = match (editor.token, editor.from) {
            (Some(token), from) => format!(
                "{} \u{b7} editing from v{}, saves v{}",
                editor.name,
                from.unwrap_or(token),
                token + 1
            ),
            (None, _) => format!("{} \u{b7} new, saves v1", editor.name),
        };
        let inner = KitPane::inner(left);
        self.page.set(inner.height);
        let open = editor.help.is_none();
        frame.render_widget(
            Paragraph::new(
                editor
                    .area
                    .lines(inner.width, inner.height, open, ctx.theme),
            ),
            inner,
        );
        ctx.claim_editor_area(inner);
        let focus = if open {
            holder(ctx, false)
        } else {
            Focus::Unfocused
        };
        let hints = hint_line(self.draft_hint(editor, ctx.keys(), left.width), ctx);
        framed(
            view_chips_if_room(KitPane::new(&title), &title, 1, left.width),
            focus,
            hints,
            ctx,
        )
        .render(frame, left, ctx.theme);
    }

    /// The role's placeholders and, for the judge and `review`, the wire contract: a kit pane
    /// beside the draft, never focused (the draft or the help has the keys).
    fn render_placeholders(
        &self,
        frame: &mut Frame<'_>,
        right: Rect,
        editor: &Editor,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        // One table for the column and the help prompt (MOD-55 §1.3).
        let role = TemplateRole::of_name(&editor.name);
        let mut lines: Vec<Line<'static>> = edit_help::placeholder_table(role)
            .into_iter()
            .map(|line| Line::styled(line, theme.base))
            .collect();
        let wire = match (role, editor.name.as_str()) {
            (TemplateRole::Judge, _) => Some(JUDGE_WIRE),
            (_, "review") => Some(REVIEW_WIRE),
            _ => None,
        };
        if let Some(wire) = wire {
            lines.push(Line::default());
            lines.push(Line::styled(wire, theme.dim));
        }
        let inner = KitPane::inner(right);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        KitPane::new(&format!("{} placeholders", role.as_str()))
            .accent(ctx.accent())
            .render(frame, right, theme);
    }
}

/// What the right-hand pane shows in Browse (MOD-115 M4 E5).
enum Detail<'a> {
    /// The name prompt, through `kit::form`.
    Form(Form<'a>),
    /// Nothing to show: what is missing and the next step (`kit::empty`).
    Empty(&'static str, String),
    /// A refused read: the store's sentence, and where to fix it.
    Unavailable(String),
    /// The shown body or the diff, wrapped and scrolled.
    Lines(Vec<Line<'static>>),
}

/// Whether a snapshot answers the scope the view is in now: a save's reply that crossed a scope
/// change is fresh to the staleness index (its kind was not re-issued), and its tree belongs to
/// the workspace that was left.
fn in_scope(snapshot: &TemplatesSnapshot, ctx: &Ctx<'_>) -> bool {
    snapshot
        .projects
        .iter()
        .map(|entry| entry.project_id)
        .eq(ctx.scope.project_ids.iter().copied())
}

/// A project header: its slug from the scope's projects, else the id's first eight chars.
fn slug(ctx: &Ctx<'_>, project: ProjectId) -> String {
    ctx.projects
        .iter()
        .find(|entry| entry.project_id == project)
        .map_or_else(
            || project.to_string().chars().take(8).collect(),
            |entry| entry.slug.clone(),
        )
}

/// One template row: two spaces, the name fitted to [`NAME_WIDTH`] cells, ` v` and the head, a
/// space and the role.
fn template_row(name: &str, head: i32, role: &str) -> String {
    format!("  {} v{head:<3} {role}", cells::fit(name, NAME_WIDTH))
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::Scope;
    use htui_core::store::MemStore;
    use htui_store::Backend;

    use super::*;

    /// The tree's width, borders included: two spaces, the name fitted to [`NAME_WIDTH`] cells,
    /// ` v`, a head of up to three digits, a space and the role make at most 28 cells. The view
    /// draws the Skills tab's shared layout (`library::LAYOUT`, MOD-115 M2 D2), which
    /// `the_browse_frame_claims_the_32_cell_list` pins to this width.
    const LIST_WIDTH: u16 = 32;

    /// MOD-67 M6 D4: the Templates notices name the key in force, and drop it unbound.
    #[test]
    fn the_templates_notices_name_their_key_or_none() {
        assert_eq!(
            omits_item(Some("F2")),
            "this phase template never places {{item}} \u{2014} F2 again saves anyway"
        );
        assert_eq!(
            omits_item(None),
            "this phase template never places {{item}} \u{2014} saving again saves anyway"
        );
        assert!(template_changed_elsewhere(2, None).ends_with("saving writes it as v3"));
        assert_eq!(
            landed_unread("saved v2", "gone", Some("F5")),
            "saved v2 \u{2014} the re-read failed, F5 reloads: gone"
        );
        assert_eq!(
            landed_unread("saved v2", "gone", None),
            "saved v2 \u{2014} the re-read failed: gone"
        );
    }
    use crate::app::{Action, Emit, TopBarState};
    use crate::keys::Keys;
    use crate::store_worker::{Origin, StoreRequest};
    use crate::templates;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crate::ui::tabs::SkillsTab;
    use crate::ui::tabs::registry::Tab as _;
    use crate::ui::tabs::skills::agent_help::fixtures as agent_fixtures;
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_agent::event::StopReason;
    use htui_core::model::StepId;
    use std::cell::Cell;

    /// MOD-60 D1: the name is fitted in cells, so a wide name never pushes the head or the role
    /// right.
    #[test]
    fn a_wide_template_name_keeps_the_head_and_role_columns() {
        for name in [
            "plan".to_owned(),
            "a".repeat(40),
            "\u{6f22}".repeat(3),
            "\u{6f22}".repeat(20),
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}".repeat(7),
        ] {
            let row = template_row(&name, 3, "plan");
            let at = row.find(" v3").expect("the head is on the row");
            assert_eq!(
                cell_width(&row[..at]),
                2 + NAME_WIDTH,
                "{row:?} for {name:?}"
            );
            assert!(row.ends_with(" v3   plan"), "{row:?} keeps the role");
        }
    }

    /// MOD-60: the name prompt carries the runtime project slug, so the field's budget is the
    /// pane less the prompt in cells. A CJK slug measured in chars leaves the field 5 cells too
    /// many and the prompt line runs past the pane.
    #[test]
    fn a_wide_project_slug_keeps_the_name_prompt_within_the_pane() {
        let (top_bar, keys, theme, emit) = (
            TopBarState::default(),
            Keys::compiled(),
            Theme::default(),
            Emit::default(),
        );
        let scope = vulkan();
        let projects = [htui_core::model::ProjectRef {
            project_id: ids::PROJECT_VULKAN,
            slug: "\u{6f22}".repeat(5),
            name: "Vulkan".to_owned(),
            position: 0,
        }];
        let ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            keys,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let view = TemplatesView {
            mode: Mode::Naming {
                project: ids::PROJECT_VULKAN,
                field: TextField::with_text(&"x".repeat(80)),
            },
            ..TemplatesView::default()
        };
        // MOD-115 M4 E5: the prompt is a `kit::form`; no row of it runs past the pane.
        let (_, detail) = view.detail(&ctx);
        let Detail::Form(form) = detail else {
            panic!("the prompt is a form");
        };
        let lines = form.lines(40, ctx.accent(), &theme);
        assert!(lines.iter().all(|line| line.width() <= 40), "{lines:?}");
        assert!(
            lines.iter().any(|line| line.width() == 40),
            "the field takes the value column: {lines:?}"
        );
    }

    /// MOD-57 P2 (PD-3, F-12): an open draft claims the text rect inside its block, so the name
    /// and versions in the block's title stay visible beside an in-pane editor; browse (where
    /// `E` hands a selected row off) draws no draft and claims nothing.
    #[test]
    fn the_draft_claims_its_text_rect_and_browse_claims_nothing() {
        let (top_bar, keys, theme, emit) = (
            TopBarState::default(),
            Keys::compiled(),
            Theme::default(),
            Emit::default(),
        );
        let scope = vulkan();
        let area = Rect::new(0, 0, 100, 28);
        let drawn = |view: &TemplatesView| {
            let cell = Cell::new(None);
            let ctx = Ctx::new(
                &scope,
                &[],
                &top_bar,
                keys,
                &theme,
                Origin::Tab(SkillsTab::ID),
                &emit,
            )
            .with_editor_area(&cell);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                    .expect("a test terminal");
            terminal
                .draw(|frame| view.render(frame, area, &ctx))
                .expect("the frame draws");
            (terminal.backend().buffer().clone(), cell.get())
        };

        let (_, claim) = drawn(&TemplatesView::default());
        assert_eq!(claim, None, "browse claims nothing");

        let view = TemplatesView {
            mode: Mode::Editing(Editor::new(
                ids::PROJECT_VULKAN,
                "plan".to_owned(),
                Some(2),
                None,
                "First line.\nSecond.\n",
            )),
            ..TemplatesView::default()
        };
        let (buffer, claim) = drawn(&view);
        let claim = claim.expect("the draft claims its text");
        // The content above the notice row, less the placeholder column, inside the draft's
        // border (the keys are the shell's hint row since MOD-67 M6 T2).
        assert_eq!(claim, Rect::new(1, 1, 98 - HELP_WIDTH, 25));
        let row = |y: u16| {
            (claim.x..claim.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(
            row(claim.y).starts_with("First line."),
            "{:?}",
            row(claim.y)
        );
        let title: String = (0..claim.right())
            .map(|x| buffer[(x, 0)].symbol())
            .collect();
        assert!(title.contains("plan \u{b7} editing from v2"), "{title:?}");

        // `Ctrl+E` moves the draft into the pending handoff: while `$EDITOR` runs it is still
        // drawn (locked: the editor has the keys) and its text still claimed, so the in-pane
        // editor lands over the text with the title beside it, not over the whole body.
        let mut view = view;
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            keys,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        assert_eq!(view.on_key(ctrl('e'), &mut ctx), Handled::Consumed);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        let (buffer, handed_off) = drawn(&view);
        assert_eq!(
            handed_off,
            Some(claim),
            "the handed-off draft claims its text"
        );
        let line = |y: u16, from: u16, to: u16| {
            (from..to)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(
            line(claim.y, claim.x, claim.right()).starts_with("First line."),
            "{:?}",
            line(claim.y, claim.x, claim.right())
        );
        let title = line(0, 0, claim.right());
        assert!(title.contains("plan \u{b7} editing from v2"), "{title:?}");
        let hint = view.key_hint(ctx.keys());
        assert!(hint.contains("the draft is in $EDITOR"), "{hint:?}");

        // Browse's `E` (F-12) has no draft to draw: the pane takes the tab body.
        let view = TemplatesView {
            external: Some(Pending {
                editor: Editor::new(
                    ids::PROJECT_VULKAN,
                    "plan".to_owned(),
                    Some(2),
                    Some(2),
                    "First line.\n",
                ),
                resume: false,
            }),
            ..TemplatesView::default()
        };
        assert_eq!(drawn(&view).1, None, "browse's `E` claims nothing");
    }

    /// The Harness's startup scope: the Graphics workspace and its one project.
    fn vulkan() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// One served reply, as the worker would send it.
    async fn serve(backend: &Backend, request: StoreRequest) -> StoreReply {
        templates::serve(backend, &request)
            .await
            .unwrap_or_else(|err| panic!("the template request failed: {err}"))
    }

    /// The requests `emit` holds, drained.
    fn sent(emit: &Emit) -> Vec<StoreRequest> {
        emit.take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// `Ctrl+S` on a body `parse` refuses dispatches nothing: the store would refuse it too, so
    /// only a count of the requests tells the local refusal from a sent-and-refused save.
    #[tokio::test]
    async fn a_refused_save_dispatches_no_request() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keys, theme, emit) = (
            TopBarState::default(),
            Keys::compiled(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            keys,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        for _ in 0..3 {
            view.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut ctx,
        );
        for c in "{{itme}}".chars() {
            view.on_key(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        assert!(sent(&emit).is_empty(), "typing sends nothing");
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let requests = sent(&emit);
        assert!(requests.is_empty(), "the refused save sent {requests:?}");
        assert_eq!(view.busy, None);
        assert!(
            matches!(&view.notice, Some(Notice::Error(text)) if text.starts_with("unknown prompt placeholder")),
            "{:?}",
            view.notice
        );
    }

    /// The view derives `Debug` down to the open editor, whose `original` and `sent` are the
    /// body: lengths only, as the draft's `TextArea`.
    #[test]
    fn an_open_editor_debug_prints_lengths_not_text() {
        let mut editor = Editor::new(
            ids::PROJECT_VULKAN,
            "implement".to_owned(),
            Some(1),
            Some(1),
            "secret original",
        );
        editor.sent = Some("secret sent".to_owned());
        let view = TemplatesView {
            mode: Mode::Editing(editor),
            ..TemplatesView::default()
        };
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("original_len: 15"), "{shown}");
        assert!(shown.contains("sent_len: Some(11)"), "{shown}");
    }

    /// MOD-59 D4: a `Templates` read served while a save is in flight — `Tab` away and back, `2`,
    /// or `r` — is not the save's answer: a plain `Templates` never lands; the save's own
    /// `TemplateSaved` does. `settle` serves in queue order, save first, so only a direct drive can
    /// put a read's reply ahead of the save's.
    #[tokio::test]
    async fn a_read_reply_does_not_close_the_editor_mid_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keys, theme, emit) = (
            TopBarState::default(),
            Keys::compiled(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            keys,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = TemplatesView::default();
        let untouched = serve(&backend, StoreRequest::Templates(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);

        // The tree is `[vulkan, fix, handoff, implement, …]`: three `j`s reach `implement`.
        for _ in 0..3 {
            view.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
                &mut ctx,
            );
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            &mut ctx,
        );
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let sent: Vec<StoreRequest> = emit
            .take()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect();
        let [save] = sent.as_slice() else {
            panic!("exactly the save was sent: {sent:?}");
        };
        let StoreRequest::SaveTemplate { name, expected, .. } = save else {
            panic!("not a save: {save:?}");
        };
        assert_eq!((name.as_str(), *expected), ("implement", Some(1)));
        assert_eq!(view.busy, Some("save_template"));

        // A read at the token's own head: the save has not landed in it.
        view.on_reply(&untouched, &mut ctx);
        assert!(
            matches!(&view.mode, Mode::Editing(editor) if editor.name == "implement"),
            "a read whose head is the token is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(
            view.busy,
            Some("save_template"),
            "the save is still in flight"
        );

        // The save's own answer, `TemplateSaved`: the version it appended is token + 1.
        let saved = serve(&backend, save.clone()).await;
        view.on_reply(&saved, &mut ctx);
        assert!(
            matches!(view.mode, Mode::Browse),
            "the head moved past the token, so the save landed: {:?}",
            view.mode
        );
        assert_eq!(view.busy, None);
        assert_eq!(view.notice, Some(Notice::Info("saved v2".to_owned())));
    }

    /// What a `Ctx` borrows, held by the test: the Harness's startup scope and a fresh `Emit`.
    struct Bench {
        /// [`vulkan`].
        scope: Scope,
        /// The top bar.
        top_bar: TopBarState,
        /// The compiled default keys.
        keys: Keys,
        /// The default theme.
        theme: Theme,
        /// What the view emitted.
        emit: Emit,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                scope: vulkan(),
                top_bar: TopBarState::default(),
                keys: Keys::compiled().clone(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &[],
                &self.top_bar,
                &self.keys,
                &self.theme,
                Origin::Tab(SkillsTab::ID),
                &self.emit,
            )
        }

        /// The one request the view sent since the last drain, or a panic naming what it sent.
        fn one(&self) -> StoreRequest {
            let requests = sent(&self.emit);
            let [request] = requests.as_slice() else {
                panic!("exactly one request was sent: {requests:?}");
            };
            request.clone()
        }
    }

    /// A plain key.
    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// From a fresh read of the tree: `implement` opened (`[vulkan, fix, handoff, implement, …]`,
    /// three `j`s), one `x` typed and `Ctrl+S`. The save it sent, at token 1.
    fn save_implement(view: &mut TemplatesView, bench: &Bench, ctx: &mut Ctx<'_>) -> StoreRequest {
        for _ in 0..3 {
            view.on_key(key('j'), ctx);
        }
        view.on_key(key('e'), ctx);
        view.on_key(key('x'), ctx);
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            ctx,
        );
        let save = bench.one();
        assert!(
            matches!(
                &save,
                StoreRequest::SaveTemplate { name, expected: Some(1), .. } if name == "implement"
            ),
            "{save:?}"
        );
        assert_eq!(view.busy, Some("save_template"));
        save
    }

    /// MOD-59 D4: a read that already shows the saved version is still a read, and a read never
    /// lands a save; only the save's own `TemplateSaved` does. Before MOD-59 the read landed it by
    /// content, `token + 1` with the body sent.
    #[tokio::test]
    async fn a_read_that_shows_the_saved_version_does_not_land_the_save() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let save = save_implement(&mut view, &bench, &mut ctx);

        let saved = serve(&backend, save).await;
        let shows = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        let StoreReply::Templates(shown) = &shows else {
            panic!("the read answers `Templates`: {shows:?}");
        };
        assert_eq!(
            shown
                .head(ids::PROJECT_VULKAN, "implement")
                .map(|row| row.version),
            Some(2),
            "precondition: the read shows the saved version"
        );

        view.on_reply(&shows, &mut ctx);
        assert!(
            matches!(&view.mode, Mode::Editing(editor) if editor.name == "implement"),
            "a read is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(view.busy, Some("save_template"), "the save is in flight");

        view.on_reply(&saved, &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert_eq!(view.notice, Some(Notice::Info("saved v2".to_owned())));
    }

    /// MOD-59 D4: the save lands on its own reply, and the view does not look in the re-read for
    /// the body it sent. Here the row at the saved version carries another body, as no store
    /// would write it; the save lands all the same (DV-2: append-only rows cannot be raced).
    #[tokio::test]
    async fn a_save_lands_on_its_own_reply_whatever_body_the_reread_shows() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let save = save_implement(&mut view, &bench, &mut ctx);

        let reply = match serve(&backend, save).await {
            StoreReply::TemplateSaved {
                snapshot: Ok(mut snapshot),
                project,
                name,
                version,
            } => {
                for row in snapshot
                    .projects
                    .iter_mut()
                    .flat_map(|entry| entry.templates.iter_mut())
                    .filter(|row| row.name == "implement" && row.version == version)
                {
                    "Theirs.\n".clone_into(&mut row.body);
                }
                StoreReply::TemplateSaved {
                    snapshot: Ok(snapshot),
                    project,
                    name,
                    version,
                }
            }
            other => panic!("an applied save answers `TemplateSaved`, not {other:?}"),
        };
        view.on_reply(&reply, &mut ctx);

        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert_eq!(view.notice, Some(Notice::Info("saved v2".to_owned())));
    }

    /// MOD-59 D5: a save whose re-read failed was appended all the same. It lands: the editor
    /// closes and the notice says what was saved and that the tree drawn is the one held, so a new
    /// name it does not show leaves the cursor where it was. A `TemplateSaved` whose re-read failed
    /// while no save is in flight has no scope to check, so it changes nothing.
    #[tokio::test]
    async fn a_save_whose_reread_failed_lands_and_keeps_the_tree_drawn() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let before = view.snapshot.clone();
        let unread = StoreReply::TemplateSaved {
            snapshot: Err("store unreachable: gone".to_owned()),
            project: ids::PROJECT_VULKAN,
            name: "implement".to_owned(),
            version: 2,
        };

        view.on_reply(&unread, &mut ctx);
        assert_eq!(view.unavailable, None, "no save in flight owns it");
        assert_eq!(view.notice, None);

        save_implement(&mut view, &bench, &mut ctx);
        view.on_reply(&unread, &mut ctx);

        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Error(text))
                    if text.starts_with("saved v2 \u{2014} the re-read failed")
                        && text.ends_with("store unreachable: gone")
            ),
            "{:?}",
            view.notice
        );
        assert_eq!(view.snapshot, before, "the tree drawn is the one held");
        assert_eq!(view.unavailable, None, "a tree is held, so it stays drawn");

        // A new name lands the same way, but the held tree has no row for it yet: the cursor
        // stays where it was rather than move onto a row that is not drawn.
        let cursor = view.cursor;
        view.on_key(key('n'), &mut ctx);
        for c in "release-notes".chars() {
            view.on_key(key(c), &mut ctx);
        }
        view.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut ctx);
        // `{{item}}`, so a phase role's "no item" question does not hold the save back.
        for c in "{{item}}".chars() {
            view.on_key(key(c), &mut ctx);
        }
        view.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut ctx,
        );
        let create = bench.one();
        assert!(
            matches!(
                &create,
                StoreRequest::SaveTemplate { name, expected: None, .. } if name == "release-notes"
            ),
            "{create:?}"
        );
        view.on_reply(
            &StoreReply::TemplateSaved {
                snapshot: Err("store unreachable: gone".to_owned()),
                project: ids::PROJECT_VULKAN,
                name: "release-notes".to_owned(),
                version: 1,
            },
            &mut ctx,
        );
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Error(text)) if text.starts_with("saved v1 \u{2014}")
            ),
            "{:?}",
            view.notice
        );
        assert_eq!(
            view.cursor, cursor,
            "the held tree has no `release-notes` row"
        );
        assert_eq!(
            view.selected_template(),
            Some((ids::PROJECT_VULKAN, "implement".to_owned()))
        );
    }

    // --- MOD-55: agent help -------------------------------------------------------------------

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    /// The open editor.
    fn editor(view: &TemplatesView) -> &Editor {
        match &view.mode {
            Mode::Editing(editor) => editor,
            other => panic!("an editor is open: {other:?}"),
        }
    }

    /// From a fresh read: `implement` opened (three `j`s, `e`), then `Ctrl+G`. The one request
    /// it sent, drained.
    fn open_help(view: &mut TemplatesView, bench: &Bench, ctx: &mut Ctx<'_>) -> StoreRequest {
        for _ in 0..3 {
            view.on_key(key('j'), ctx);
        }
        view.on_key(key('e'), ctx);
        assert_eq!(view.on_key(ctrl('g'), ctx), Handled::Consumed);
        bench.one()
    }

    /// [`open_help`], one enabled agent read, `shorter` asked. The `EditHelp` it sent.
    fn ask_help(view: &mut TemplatesView, bench: &Bench, ctx: &mut Ctx<'_>) -> StoreRequest {
        open_help(view, bench, ctx);
        view.on_reply(
            &StoreReply::Agents(vec![agent_fixtures::summary("scripted", true)]),
            ctx,
        );
        for c in "shorter".chars() {
            view.on_key(key(c), ctx);
        }
        view.on_key(enter(), ctx);
        bench.one()
    }

    /// TV-1 (B-1): `Ctrl+G` in the editor opens the help, which reads the agents.
    #[tokio::test]
    async fn ctrl_g_opens_help_and_asks_for_agents() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let request = open_help(&mut view, &bench, &mut ctx);
        assert!(matches!(request, StoreRequest::Agents), "{request:?}");
        assert!(editor(&view).help.is_some());
        assert!(view.captures_input(), "the editor still owns every key");
    }

    /// TV-2 (P7): the request names the editor's project and the template, and carries the
    /// draft as it was when the help opened.
    #[tokio::test]
    async fn the_help_request_carries_the_editor_s_project_and_name() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let request = ask_help(&mut view, &bench, &mut ctx);
        let StoreRequest::EditHelp {
            project_id, prompt, ..
        } = &request
        else {
            panic!("an EditHelp: {request:?}");
        };
        assert_eq!(*project_id, ids::PROJECT_VULKAN);
        assert_eq!(
            prompt.target,
            HelpTarget::Template {
                name: "implement".to_owned()
            }
        );
        assert_eq!(prompt.body, editor(&view).area.text());
        assert_eq!(prompt.request, "shorter");
    }

    /// TV-3: a save in flight refuses `Ctrl+G` with the in-flight notice; no help opens.
    #[tokio::test]
    async fn ctrl_g_is_refused_while_a_save_is_in_flight() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        save_implement(&mut view, &bench, &mut ctx);
        view.on_key(ctrl('g'), &mut ctx);
        assert_eq!(
            view.notice,
            Some(Notice::Error(
                "`save_template` is still in flight".to_owned()
            ))
        );
        assert!(editor(&view).help.is_none());
        assert!(sent(&bench.emit).is_empty());
    }

    /// TV-4: an accepted proposal replaces the draft through the `$EDITOR` return's gate: `parse`
    /// runs, the cursor goes to the error, and nothing is sent. A body `parse` accepts says so,
    /// and `Ctrl+S` saves it.
    #[tokio::test]
    async fn an_accepted_proposal_replaces_the_draft_through_parse() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        ask_help(&mut view, &bench, &mut ctx);
        let original = editor(&view).original.clone();
        view.on_reply(&agent_fixtures::accepted(StepId::new()), &mut ctx);
        view.on_reply(
            &agent_fixtures::chunk("Here:\n```\nx {{itme}}\n```\n"),
            &mut ctx,
        );
        view.on_reply(&agent_fixtures::ended(StopReason::EndTurn), &mut ctx);
        assert!(editor(&view).help.is_some(), "the proposal is shown");
        assert_eq!(view.on_key(enter(), &mut ctx), Handled::Consumed);

        let open = editor(&view);
        assert!(open.help.is_none(), "accepting closes the help");
        assert_eq!(open.area.text(), "x {{itme}}\n");
        assert_eq!(open.area.cursor_line_col(), (0, 2), "on the braces");
        assert_eq!(open.original, original, "Esc still asks first");
        assert!(
            matches!(&view.notice, Some(Notice::Error(text)) if text.starts_with("unknown prompt placeholder")),
            "{:?}",
            view.notice
        );
        assert!(sent(&bench.emit).is_empty(), "accepting sends nothing");

        view.on_key(ctrl('g'), &mut ctx);
        assert!(matches!(bench.one(), StoreRequest::Agents));
        view.on_reply(
            &StoreReply::Agents(vec![agent_fixtures::summary("scripted", true)]),
            &mut ctx,
        );
        view.on_key(key('x'), &mut ctx);
        view.on_key(enter(), &mut ctx);
        bench.one();
        view.on_reply(&agent_fixtures::accepted(StepId::new()), &mut ctx);
        view.on_reply(&agent_fixtures::chunk("```\nDo {{item}}.\n```\n"), &mut ctx);
        view.on_reply(&agent_fixtures::ended(StopReason::EndTurn), &mut ctx);
        view.on_key(key('y'), &mut ctx);
        assert_eq!(view.notice, Some(Notice::Info(accepted(Some("Ctrl+s")))));
        view.on_key(ctrl('s'), &mut ctx);
        let save = bench.one();
        assert!(
            matches!(&save, StoreRequest::SaveTemplate { body, .. } if body.as_str() == "Do {{item}}.\n"),
            "{save:?}"
        );
    }

    /// TV-5: while the help is open, keys and pastes go to it: the draft under it is locked.
    #[tokio::test]
    async fn typing_under_an_open_help_leaves_the_draft_alone() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = TemplatesView::default();
        let read = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        open_help(&mut view, &bench, &mut ctx);
        let before = editor(&view).area.text().to_owned();
        for c in "zzz".chars() {
            view.on_key(key(c), &mut ctx);
        }
        assert!(view.on_paste("pasted"));
        view.on_key(ctrl('e'), &mut ctx);
        assert_eq!(editor(&view).area.text(), before, "the draft is locked");
        assert!(
            bench.emit.take().is_empty(),
            "Ctrl+E is refused while the help is open"
        );
        // `Esc` closes the asking help; the draft is back, editable.
        view.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut ctx);
        assert!(editor(&view).help.is_none());
        view.on_key(key('z'), &mut ctx);
        assert_eq!(editor(&view).area.text(), format!("z{before}"));
    }

    // ---- MOD-115 M2 T7: the panes, motion by focus, the location ---------------------------

    /// A view browsing the demo tree, the cursor on its first row.
    async fn browsing(bench: &Bench) -> TemplatesView {
        let backend = Backend::memory(MemStore::demo());
        let mut view = TemplatesView::default();
        let reply = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        view.on_reply(&reply, &mut bench.ctx());
        assert!(
            view.rows().len() >= 3,
            "the demo tree has rows to move over"
        );
        view
    }

    /// MOD-115 M2 D2: the Templates view shares the Skills tab's layout, the 32-cell list both
    /// views drew, and its browse frame claims exactly the old `[Length(32), Min(1)]` split.
    #[tokio::test]
    async fn the_browse_frame_claims_the_32_cell_list() {
        use crate::ui::shell::ListSize;
        assert_eq!(LIST_WIDTH, 32);
        assert_eq!(
            super::super::library::LAYOUT.list,
            ListSize::Cells(LIST_WIDTH)
        );

        let bench = Bench::new();
        let view = browsing(&bench).await;
        let claim = Cell::new(None);
        let ctx = bench.ctx().with_pane_claim(&claim);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 28)).expect("terminal");
        terminal
            .draw(|frame| view.render(frame, frame.area(), &ctx))
            .expect("the frame draws");
        let (region, areas) = claim.get().expect("browse claims its panes");
        let [list, pane] =
            Layout::horizontal([Constraint::Length(32), Constraint::Min(1)]).areas(region);
        assert_eq!((areas.list, areas.detail), (Some(list), pane));
    }

    /// MOD-115 M2 §7: with the tree focused, motion moves the cursor; with the detail focused
    /// it scrolls the pane and the cursor stays; with the activity pane focused nothing moves.
    #[tokio::test]
    async fn motion_follows_the_focused_pane() {
        use crate::ui::shell::PaneId;
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.pane_rows.set(10);
        let press = |view: &mut TemplatesView, focus: PaneId, code: KeyCode| {
            view.on_key(
                KeyEvent::new(code, KeyModifiers::NONE),
                &mut bench.ctx().with_focus(focus),
            )
        };
        for (code, offset) in [
            (KeyCode::Char('j'), 1),
            (KeyCode::Down, 2),
            (KeyCode::Char('k'), 1),
        ] {
            assert_eq!(press(&mut view, PaneId::Detail, code), Handled::Consumed);
            assert_eq!(view.scroll.offset(), offset, "{code:?}");
        }
        assert_eq!(view.cursor, 0, "the cursor stayed");
        assert_eq!(
            press(&mut view, PaneId::Activity, KeyCode::Char('j')),
            Handled::Consumed
        );
        assert_eq!((view.cursor, view.scroll.offset()), (0, 1), "nothing moved");
        assert_eq!(
            press(&mut view, PaneId::List, KeyCode::Char('j')),
            Handled::Consumed
        );
        assert_eq!(view.cursor, 1, "the tree's");
        assert!(bench.emit.take().is_empty());
    }

    /// MOD-115 M2 §8, RV3 (review M2): the location names the row under the cursor as well as
    /// its index: a template by `{index}:{project}:{name}`, a project header by
    /// `{index}:{project}`. A template created before it moves its row, and the cursor goes back
    /// onto the same template; one that is gone falls back to the row, clamped.
    #[tokio::test]
    async fn the_cursor_finds_its_template_after_the_tree_changed() {
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        let project = match view.rows()[0] {
            Row::Project(project) => project,
            Row::Template { .. } => panic!("the tree opens on a project header"),
        };
        assert_eq!(view.location_cursor(), format!("0:{project}"));
        view.move_cursor(true);
        view.move_cursor(true);
        let Some((_, name)) = view.selected_template() else {
            panic!("a template on the third row");
        };
        let at = view.location_cursor();
        assert_eq!(at, format!("2:{project}:{name}"));

        let snapshot = view.snapshot.as_mut().expect("read");
        let mut created = snapshot.projects[0].templates[0].clone();
        created.name = "aaa-triage".to_owned();
        snapshot.projects[0].templates.insert(0, created);
        view.cursor = 0;
        assert!(view.restore_cursor(&at));
        assert_eq!(
            view.selected_template(),
            Some((project, name.clone())),
            "on the same template"
        );
        assert_eq!(view.cursor, 3, "a row down");

        let snapshot = view.snapshot.as_mut().expect("read");
        snapshot.projects[0]
            .templates
            .retain(|row| row.name != name);
        view.cursor = 0;
        assert!(view.restore_cursor(&at));
        assert_eq!(view.cursor, 2, "the template is gone: its row");
        assert!(!view.restore_cursor("not a row"));
        assert_eq!(view.cursor, 2, "an unreadable cursor moves nothing");
    }

    /// MOD-115 M2 §8: the Skills tab's location is the shown view (`library` or `templates`,
    /// with its title) and that view's row; restoring it shows the view and puts the row back.
    /// Nothing moves under an open prompt.
    #[tokio::test]
    async fn the_location_is_the_view_and_its_row_and_restore_returns_to_them() {
        let bench = Bench::new();
        let backend = Backend::memory(MemStore::demo());
        let mut tab = SkillsTab::new();
        let templates = serve(&backend, StoreRequest::Templates(bench.scope.clone())).await;
        let skills = crate::skills::serve(&backend, &StoreRequest::Skills(bench.scope.clone()))
            .await
            .expect("the library reads");
        tab.on_reply(&templates, &mut bench.ctx());
        tab.on_reply(&skills, &mut bench.ctx());

        let library = tab.location();
        assert_eq!(library.subtab, Some("library"));
        assert_eq!(library.subtab_title.as_deref(), Some("Skills"));
        assert!(
            library
                .cursor
                .as_deref()
                .is_some_and(|at| at.starts_with("0:")),
            "the library's first row and its skill: {library:?}"
        );

        tab.on_key(key('l'), &mut bench.ctx());
        tab.on_key(key('j'), &mut bench.ctx());
        tab.on_key(key('j'), &mut bench.ctx());
        let here = tab.location();
        assert_eq!(here.subtab, Some("templates"));
        assert_eq!(here.subtab_title.as_deref(), Some("Templates"));
        assert!(
            here.cursor
                .as_deref()
                .is_some_and(|at| at.starts_with("2:")),
            "the tree's third row and its template: {here:?}"
        );

        // RV3 (review M1): each view's cursor moves off the row first, so a restore that put
        // back only the view would leave it there.
        tab.on_key(key('h'), &mut bench.ctx());
        tab.on_key(key('j'), &mut bench.ctx());
        assert_eq!(tab.location().subtab, Some("library"));
        assert_ne!(tab.location(), library, "the library's cursor moved");
        assert!(tab.restore(&library, &mut bench.ctx()));
        assert_eq!(tab.location(), library, "back on the library's first row");
        tab.on_key(key('l'), &mut bench.ctx());
        tab.on_key(key('k'), &mut bench.ctx());
        assert_eq!(tab.location().subtab, Some("templates"));
        assert_ne!(tab.location(), here, "the tree's cursor moved");
        assert!(tab.restore(&here, &mut bench.ctx()));
        assert_eq!(tab.location(), here, "and forward to the template");

        tab.on_key(key('n'), &mut bench.ctx());
        assert!(tab.templates.captures_input(), "the name prompt is open");
        assert!(!tab.restore(&library, &mut bench.ctx()));
        assert_eq!(tab.location().subtab, Some("templates"), "nothing moved");
        let mut fresh = SkillsTab::new();
        assert!(
            !fresh.restore(&crate::ui::shell::TabLoc::default(), &mut bench.ctx()),
            "a location that names nothing restores nothing"
        );
    }

    // ---- MOD-115 M4 L-skills: the view on the kit ----------------------------------------------

    /// `view` drawn at `width` x `height` through `ctx`, as the buffer and its rows.
    fn frame_of(
        view: &TemplatesView,
        ctx: &Ctx<'_>,
        width: u16,
        height: u16,
    ) -> (ratatui::buffer::Buffer, Vec<String>) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test terminal");
        terminal
            .draw(|frame| view.render(frame, frame.area(), ctx))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (buffer, rows)
    }

    /// A view naming a new template in `vulkan-tutorials`.
    fn naming() -> TemplatesView {
        TemplatesView {
            mode: Mode::Naming {
                project: ids::PROJECT_VULKAN,
                field: TextField::new(),
            },
            ..TemplatesView::default()
        }
    }

    /// MOD-115 M4 MD-A: the name prompt's hint names `field.submit` through `TEMPLATES_PROMPT`,
    /// so a rebound one shows its chord; `Esc` stays the field's (M2 MD7).
    #[test]
    fn the_name_prompt_hint_names_the_submit_key_in_force() {
        let rebound = crate::keys::load_str("version = 2\n[field]\nsubmit = \"ctrl-t\"\n")
            .expect("the key file loads");
        assert_eq!(
            naming().key_hint(Keys::compiled()),
            "Enter create \u{b7} Esc cancel"
        );
        assert_eq!(
            naming().key_hint(&rebound),
            "Ctrl+t create \u{b7} Esc cancel"
        );
    }

    /// MOD-115 M4 MD-A: what the name prompt's field passes resolves from the chord pressed
    /// (`Ctx::pressed`): a pressed `ctrl-n` the shell delivered as `Down` passes, a pressed
    /// `Down` is swallowed.
    #[test]
    fn the_name_prompt_resolves_what_its_field_passes_from_the_chord_pressed() {
        let bench = Bench::new();
        let chord = |spec: &str| crate::keys::KeyChord::parse_strict(spec).expect("a valid spec");
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let mut view = naming();
        assert_eq!(
            view.on_key(down, &mut bench.ctx().with_pressed(down, chord("ctrl-n"))),
            Handled::Pass
        );
        assert_eq!(
            view.on_key(down, &mut bench.ctx().with_pressed(down, chord("down"))),
            Handled::Consumed
        );
        assert!(view.captures_input(), "the prompt stays");
    }

    /// MOD-115 M4 E5, MOD-85: the tree's cursor is the kit's gutter and band, legible without
    /// colour; a project row keeps the title role and the template rows their columns.
    #[tokio::test]
    async fn the_tree_cursor_is_the_gutter_and_band() {
        use ratatui::style::Modifier;
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.on_key(key('j'), &mut bench.ctx());
        let bench = Bench {
            theme: Theme::monochrome(),
            ..Bench::new()
        };
        let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 28);
        assert!(rows[0].starts_with('\u{256d}'), "rounded: {rows:#?}");
        assert_eq!(buffer[(1, 2)].symbol(), list::GUTTER, "{rows:#?}");
        assert!(
            buffer[(5, 2)].modifier.contains(Modifier::REVERSED),
            "banded"
        );
        assert_eq!(
            buffer[(1, 1)].symbol(),
            " ",
            "the project row is not the cursor"
        );
        assert!(
            buffer[(2, 1)].modifier.contains(Modifier::BOLD),
            "the title role"
        );
        assert!(rows[2].contains("  fix           v1   phase"), "{rows:#?}");
    }

    /// MOD-115 M4 C4: the editor's draft holds the keyboard, so it is the focused pane with the
    /// editor's keys on its border and the view chips on its top; the placeholders beside it are
    /// a pane that is never focused.
    #[test]
    fn the_draft_is_the_focused_pane_and_the_placeholders_are_not() {
        let bench = Bench::new();
        let view = TemplatesView {
            mode: Mode::Editing(Editor::new(
                ids::PROJECT_VULKAN,
                "implement".to_owned(),
                Some(1),
                Some(1),
                "body\n",
            )),
            ..TemplatesView::default()
        };
        let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 20);
        let accent = bench.theme.tab_accent(SkillsTab::ID.0);
        assert_eq!(Some(buffer[(0, 0)].fg), accent.fg, "the draft: {rows:#?}");
        assert_eq!(
            Some(buffer[(62, 0)].fg),
            bench.theme.border_faint.fg,
            "the placeholders"
        );
        assert!(
            rows[0].contains("implement \u{b7} editing from v1, saves v2"),
            "{rows:#?}"
        );
        assert!(rows[0].contains(" Templates "), "the view chips: {rows:#?}");
        assert!(rows[0].contains("phase placeholders"), "{rows:#?}");
        assert!(rows[18].contains("Ctrl+s save"), "{rows:#?}");
    }

    /// MOD-115 M4 L-skills repair 1: the draft pane is narrower than the row the editor's keys
    /// had, so they are fitted to it whole: the cursor's `L:C`, then save and the way out, then
    /// the rest while they fit; a key that does not fit goes with those after it, and `…` says
    /// so. Never half a key; at a width that holds them all, all of them.
    #[test]
    fn the_draft_hint_keeps_the_way_out_and_never_cuts_a_key() {
        let bench = Bench::new();
        let view = TemplatesView {
            mode: Mode::Editing(Editor::new(
                ids::PROJECT_VULKAN,
                "implement".to_owned(),
                Some(1),
                Some(1),
                "body\n",
            )),
            ..TemplatesView::default()
        };
        let bottom = |width: u16| {
            let (_, rows) = frame_of(&view, &bench.ctx(), width, 20);
            let draft = usize::from(width - HELP_WIDTH);
            let border = rows
                .iter()
                .rposition(|row| row.starts_with('\u{2570}'))
                .expect("the draft's bottom border");
            let row: String = rows[border].chars().take(draft).collect();
            row.trim_matches(['\u{2570}', '\u{256f}', '\u{2500}'])
                .to_owned()
        };
        assert_eq!(
            bottom(100),
            "L1:C1 \u{b7} Ctrl+s save \u{b7} Esc cancel \u{b7} Ctrl+g ask agent \u{b7}\u{2026}"
        );
        assert_eq!(
            bottom(120),
            "L1:C1 \u{b7} Ctrl+s save \u{b7} Esc cancel \u{b7} Ctrl+g ask agent \u{b7} Ctrl+e $EDITOR"
        );
        let narrow = bottom(80);
        assert!(
            narrow.starts_with("L1:C1 \u{b7} Ctrl+s save \u{b7} Esc cancel"),
            "{narrow:?}"
        );
        assert!(narrow.ends_with(" \u{b7}\u{2026}"), "{narrow:?}");
    }

    /// MOD-115 M4 E5, MOD-85: the body's and the draft's titles are the kit pane's title role,
    /// the tab's accent in bold while the pane is focused and dim while it is not. In colour and
    /// in monochrome.
    #[tokio::test]
    async fn the_body_and_draft_titles_are_the_pane_title_role() {
        use ratatui::style::{Modifier, Style};
        let in_style = |cell: &ratatui::buffer::Cell, style: Style| {
            style.fg.is_none_or(|fg| cell.fg == fg)
                && style.bg.is_none_or(|bg| cell.bg == bg)
                && cell.modifier.contains(style.add_modifier)
        };
        let column_of = |row: &str, text: &str| {
            let at = row
                .find(text)
                .unwrap_or_else(|| panic!("{text:?} is not on {row:?}"));
            u16::try_from(row[..at].chars().count()).expect("a short row")
        };
        for theme in [Theme::default(), Theme::monochrome()] {
            let bench = Bench {
                theme,
                ..Bench::new()
            };
            let focused = bench
                .theme
                .tab_accent(SkillsTab::ID.0)
                .add_modifier(Modifier::BOLD);
            let mut view = browsing(&bench).await;
            view.on_key(key('j'), &mut bench.ctx());
            let ctx = bench.ctx().with_focus(PaneId::Detail);
            let (buffer, rows) = frame_of(&view, &ctx, 100, 12);
            // The body's title follows the list's corner and its own: `╮╭─ {name} v1 (head v1)`.
            let x = column_of(&rows[0], "\u{256e}\u{256d}\u{2500} ") + 4;
            assert!(rows[0].contains(" v1 (head v1)"), "{rows:#?}");
            assert!(in_style(&buffer[(x, 0)], focused), "focused: {rows:#?}");
            let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 12);
            let title = &buffer[(x, 0)];
            assert!(in_style(title, bench.theme.dim), "unfocused: {rows:#?}");
            assert!(!title.modifier.contains(Modifier::BOLD), "{rows:#?}");

            let view = TemplatesView {
                mode: Mode::Editing(Editor::new(
                    ids::PROJECT_VULKAN,
                    "implement".to_owned(),
                    Some(1),
                    Some(1),
                    "body\n",
                )),
                ..TemplatesView::default()
            };
            let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 12);
            let x = column_of(&rows[0], "implement \u{b7} editing");
            assert!(in_style(&buffer[(x, 0)], focused), "the draft: {rows:#?}");
        }
    }

    /// MOD-115 M4 L-skills repair 1 (Q2): the focused list shows its own keys whole in its 28
    /// cells: its motion first, as every list's hint does, then a new template and the view's own
    /// act, the keys the Browse row showed before the port; the row's keys are the body's.
    #[tokio::test]
    async fn the_list_hint_leads_with_its_motion() {
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.on_key(key('j'), &mut bench.ctx());
        let (_, rows) = frame_of(&view, &bench.ctx(), 100, 12);
        let bottom = rows
            .iter()
            .rposition(|row| row.starts_with('\u{2570}'))
            .expect("the list's bottom border");
        let list: String = rows[bottom].chars().take(32).collect();
        assert_eq!(
            list.trim_matches(['\u{2570}', '\u{256f}', '\u{2500}']),
            "j/k \u{b7} n new \u{b7} r reload",
            "{rows:#?}"
        );
    }

    /// MOD-115 M4 L-skills repair 2: every empty state of the pane says what is missing and the
    /// next step (`kit::empty`), and a refused read names where it is fixed: before the first
    /// reply, on a refused read, and with the cursor on a project's row; with the keys of the
    /// next step unbound, it is said in words.
    #[tokio::test]
    async fn every_empty_pane_names_its_next_step() {
        let bench = Bench::new();
        let shows = |view: &TemplatesView, bench: &Bench, texts: &[&str]| {
            let (_, rows) = frame_of(view, &bench.ctx(), 100, 12);
            for text in texts {
                assert!(
                    rows.iter().any(|row| row.contains(text)),
                    "{text:?}: {rows:#?}"
                );
            }
        };
        shows(
            &TemplatesView::default(),
            &bench,
            &[
                "templates not read yet",
                "they appear once the store answers",
            ],
        );
        let refused = TemplatesView {
            unavailable: Some("store unreachable: gone".to_owned()),
            ..TemplatesView::default()
        };
        shows(
            &refused,
            &bench,
            &[
                "templates unavailable: store unreachable: gone",
                "Settings \u{203a} Connection stores a DSN",
            ],
        );
        // The cursor starts on the project's row.
        let view = browsing(&bench).await;
        shows(
            &view,
            &bench,
            &["select a template", "j/k chooses one \u{b7} n creates one"],
        );
        let unbound = Bench {
            keys: crate::keys::load_str(
                "version = 2\n[list]\ndown = []\nup = []\n[common]\nnew = []\n",
            )
            .expect("the key file loads"),
            ..Bench::new()
        };
        shows(&view, &unbound, &["a template row shows its body here"]);
    }

    /// MOD-115 M4 L-skills repair 2: beside the placeholders, an 80-column draft is 42 cells
    /// wide. Its title, the version a save writes, stays whole there: the view chips, which do
    /// nothing while the draft has the keys, are drawn only when the whole title fits beside
    /// them, as it does at 100 columns.
    #[test]
    fn the_draft_title_stays_whole_beside_the_placeholders_at_80_columns() {
        let bench = Bench::new();
        let view = TemplatesView {
            mode: Mode::Editing(Editor::new(
                ids::PROJECT_VULKAN,
                "handoff".to_owned(),
                Some(1),
                Some(1),
                "body\n",
            )),
            ..TemplatesView::default()
        };
        let (_, rows) = frame_of(&view, &bench.ctx(), 80, 12);
        assert!(
            rows[0].contains(" handoff \u{b7} editing from v1, saves v2 "),
            "{rows:#?}"
        );
        let (_, rows) = frame_of(&view, &bench.ctx(), 100, 12);
        assert!(
            rows[0].contains(" handoff \u{b7} editing from v1, saves v2 "),
            "{rows:#?}"
        );
        assert!(rows[0].contains(" Templates "), "the view chips: {rows:#?}");
    }
}
