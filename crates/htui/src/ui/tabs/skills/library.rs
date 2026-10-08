//! The Skills view of the Skills tab (MOD-9 milestone 3, plan D82; blueprint D98-D100): the skill
//! library, any version's body or a line diff between two versions, a token estimate, an editor
//! that appends a version, create and rename, and the attachments pane (`attach`).
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle and
//! no `UserId`, renders from the last [`SkillsSnapshot`] and never patches a row into it. A write
//! **lands on its own reply** (MOD-59): only a `SkillWritten` naming the write in flight closes
//! the draft; a plain `Skills` read never does, so a read served ahead of the write leaves the
//! draft, the token and `busy` alone. A `SkillsStale` keeps the draft and moves the token to the
//! row as it is now.
//!
//! The token a version save carries is the head when the editor opened, not the version shown:
//! editing v1 while v2 is head saves v3 (milestone 1's OQ-5, as the Templates view).
//!
//! The estimate is the skill's own block as the assembler renders it, without the section frame it
//! shares with the other skills (D99, F-I): what adding this skill to a prompt costs.
//!
//! Milestone 4 adds the import (import plan D102, `.claude/plans/mod-9-skill-import.plan.md`): `I`
//! opens a one-line path form, `Enter` sends `ImportSkills` and returns to Browse while the worker
//! walks, and the reply either says the counts in the notice or, when a file was refused or
//! skipped, opens a per-file report.

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::skill::validate_name;
use htui_core::model::{
    Activation, BindingChange, BoundSkill, SkillId, SkillLevel, SkillPatch, SkillVersion,
};
use htui_core::prompt::edit_help::HelpTarget;
use htui_core::prompt::{TokenEstimator, render};
use htui_core::store::{invalid_skill_name, skill_body_refusal};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::agent_help::{ACCEPTED, AgentHelp, HelpOutcome, Report};
use super::attach::{AttachOutcome, AttachPane};
use crate::app::{Action, Ctx, Handled};
use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack, views};
use crate::skill_import::ImportOutcome;
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillWrite, SkillsSnapshot, StaleWhat};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::TemplateBody;
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme, diff};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The `StoreRequest::ImportSkills` name: what `busy` holds while a walk is in flight. The slice
/// index rather than a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const IMPORT_NAME: &str = REQUEST_NAMES[5];

/// The list's width, borders included: two spaces, the name fitted to [`NAME_WIDTH`] cells, ` v`
/// and a head of up to three digits make 29 cells.
const LIST_WIDTH: u16 = 32;

/// A library row's name field, in cells: a name is fitted to it with `cells::fit`.
const NAME_WIDTH: usize = 22;

/// The rename form's label column: `description` and two spaces.
const INFO_LABEL: usize = 13;

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";

/// The pane with no skill under the cursor (an empty library).
const SELECT_A_SKILL: &str = "select a skill";

/// `Esc` over a modified draft, the first time.
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A write went out.
const SAVING: &str = "saving\u{2026}";

/// A `SkillsStale` for a skill the library no longer holds.
const SKILL_GONE: &str = "the skill is gone";

/// A rename over a row that changed since the form opened.
const SKILL_CHANGED_ELSEWHERE: &str = "changed elsewhere since you opened it \u{2014} your text is \
                                       kept; Ctrl+S saves it over the new row";

/// An attachment save over a row that changed since the form opened.
const BINDING_CHANGED_ELSEWHERE: &str = "this attachment changed elsewhere \u{2014} your form is \
                                         kept; Ctrl+S saves over it";

/// A detach over a row that changed or went since the question was asked: nothing was detached,
/// and the pane shows the row as it is now.
const DETACH_CHANGED_ELSEWHERE: &str = "this attachment changed elsewhere \u{2014} nothing was \
                                        detached; its row shows it as it is now";

/// The hint row in Browse (96 cells with the defaults: `j/k` carries no word, and `r reload` is
/// left out, so the row fits 100 columns with its ` \u{b7} ` separators; MOD-67 M4). `I import`
/// took the room `h/l view` had: the switch line right above names both views.
const BROWSE_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::Pair(Act::SkillsPrevVersion, Act::SkillsNextVersion, "version"),
    Hint::One(Act::SkillsBase, "base"),
    Hint::One(Act::SkillsDiff, "diff"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::SkillsEditExternally, "$EDITOR"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::LibraryImport, "import"),
    Hint::One(Act::LibraryInfo, "info"),
    Hint::One(Act::LibraryAttach, "attach"),
];

/// The pane's bottom border while its lines overflow it: the pane keys, through
/// `LIBRARY_BROWSE` whatever the mode.
const SCROLL_HINT: HintSpec = &[
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
    Hint::Pair(Act::PanePageUp, Act::PanePageDown, "page"),
];

/// The hint row while naming or describing a new skill.
const NAMING_HINT: &str = "Enter next  Esc cancel";

/// The hint row on the rename form.
const INFO_HINT: &str = "Tab field  Ctrl+S save  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}`.
///
/// `Ctrl+G` is MOD-55's, hard-coded beside `Ctrl+S`/`Ctrl+E` (plan P10).
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+G ask agent  Ctrl+E $EDITOR  Esc cancel";

/// The hint row while a `Ctrl+E` handoff holds the draft: `$EDITOR` has the keys.
const HANDED_OFF_HINT: HintSpec = &[Hint::Text("the draft is in $EDITOR")];

/// `Ctrl+G` with no project in the workspace (MOD-55 P7): a help turn is a chat run, and a run
/// belongs to a project.
const NO_PROJECT: &str = "no project in this workspace \u{2014} agent help records its run in one";

/// The hint row on the import form.
const IMPORT_HINT: &str = "Enter import  Esc cancel";

/// The hint row on the import report.
const REPORT_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::Reload, "reload"),
    Hint::One(Act::Back, "back"),
];

/// An import went out.
const IMPORTING: &str = "importing\u{2026}";

/// `Enter` on an empty path.
const ENTER_A_PATH: &str = "type a path to a SKILL.md file or a directory";

/// An import that found nothing to import.
const NOTHING_IMPORTED: &str = "nothing to import at that path";

/// `SkillsStale` over an open editor: the Templates view's sentence (`templates.rs`).
fn version_changed_elsewhere(head: i32) -> String {
    format!(
        "saved elsewhere since you opened it \u{2014} v{head} is now the latest; your draft is \
         kept and Ctrl+S saves it as v{}",
        head + 1
    )
}

/// A key while a write is in flight: one at a time (`settings/prompt.rs`' rule).
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// MOD-59 D5: a write that applied although its re-read failed. It landed; the view still draws
/// what it held, and `r` reads again.
fn landed_unread(landed: &str, why: &str) -> String {
    format!("{landed} \u{2014} the re-read failed, r reloads: {why}")
}

/// D99 (F-I): the estimate of one skill's block as `render::skills` writes it, `version` and
/// `body` as given; without the `<section name="skills">` frame the assembler adds once for all
/// skills.
fn estimate(name: &str, version: i32, body: &str) -> i64 {
    render::skills(&[BoundSkill {
        skill_id: SkillId::default(),
        name: name.to_owned(),
        version: Some(version),
        position: 0,
        body: body.to_owned(),
        level: SkillLevel::Global,
        activation: Activation::Always,
        globs: Vec::new(),
    }])
    .map_or(0, |rendered| {
        TokenEstimator::DEFAULT.estimate(&rendered.content)
    })
}

/// The Skills view: the library, one skill's body or diff, the editor, and the attachments pane.
/// Holds no store handle and no `UserId` (`R-NF-3`); renders from the last `SkillsSnapshot`.
#[derive(Debug, Default)]
pub(super) struct LibraryView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale library.
    unavailable: Option<String>,
    /// The highlighted skill, an index into `snapshot.skills`.
    cursor: usize,
    /// The version shown; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against, once `b` chose it; `None` is the shown version's predecessor.
    base: Option<i32>,
    /// Which pane Browse shows.
    pane: Pane,
    /// The pane's first drawn row. Back to the top whenever the pane shows something else.
    scroll: Scroll,
    /// The pane's rows at the last draw, what `scroll` clamps against. A `Cell` because the count
    /// is known only in `render(&self)`.
    pane_rows: Cell<usize>,
    /// Browsing, naming, renaming, or editing.
    mode: Mode,
    /// The attachments pane, while open (D83).
    attach: Option<AttachPane>,
    /// The write in flight, by `StoreRequest::name`: one at a time, because the staleness index
    /// keeps only the newest request of a kind.
    busy: Option<&'static str>,
    /// What the write in flight carries, for the landing (D98).
    sent: Option<Sent>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `E`/`Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height: what `PageUp`/`PageDown` move by.
    page: Cell<u16>,
}

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's description and body.
    #[default]
    Body,
    /// The diff from the base to the shown version.
    Diff,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the library.
    #[default]
    Browse,
    /// `n`, step 1: the new skill's name.
    Naming {
        /// The name.
        field: TextField,
    },
    /// `n`, step 2: its description (may be empty).
    Describing {
        /// The name step 1 accepted.
        name: String,
        /// The description.
        field: TextField,
    },
    /// `i`: rename and re-describe.
    Info(InfoForm),
    /// `e`, `n`'s step 3, or an `$EDITOR` return.
    Editing(Editor),
    /// `I`: the path to import. One field, because one line is one path — a path may contain a
    /// space, and nothing here splits on one (import plan OQ-24).
    ImportPath {
        /// The path, exactly as typed.
        field: TextField,
    },
    /// The answer to an import, one row per file in the order the files were named or found.
    ///
    /// **Opened only when something was refused or skipped** (import plan D102). A clean import
    /// says so in the notice instead, because a report the reader must dismiss to learn that
    /// nothing went wrong is a report most people learn to dismiss.
    Report {
        /// Every outcome, in order.
        outcomes: Vec<ImportOutcome>,
        /// The highlighted row.
        cursor: usize,
    },
}

/// The rename form. Its `updated_at` is the compare-and-set token (D76).
#[derive(Debug)]
struct InfoForm {
    /// Which skill.
    skill: SkillId,
    /// `skill.updated_at` when the form opened, or as a `SkillsStale` last showed it.
    token: DateTime<Utc>,
    /// The name.
    name: TextField,
    /// The description.
    description: TextField,
    /// `0`: the name; `1`: the description.
    focus: usize,
}

/// What an open editor saves.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// `n`: the skill and its v1 (`CreateSkill`).
    New {
        /// The name step 1 accepted.
        name: String,
        /// The description step 2 took.
        description: String,
    },
    /// `e`: a new version of a skill (`SaveSkillVersion`).
    Version {
        /// Which skill.
        skill: SkillId,
        /// Its name, for the title and the estimate.
        name: String,
    },
}

impl Target {
    /// The skill's name.
    fn name(&self) -> &str {
        match self {
            Self::New { name, .. } | Self::Version { name, .. } => name,
        }
    }
}

/// An open editor. Never `Debug`s a body.
struct Editor {
    /// What `Ctrl+S` writes.
    target: Target,
    /// The head when the editor opened: the compare-and-set token (`0` for a new skill, D89).
    token: i32,
    /// The version the draft started from, for the title (`None`: a new skill).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// MOD-55: the agent help, while open; the draft is locked under it. Boxed: it would
    /// otherwise set the size of every [`Mode`].
    help: Option<Box<AgentHelp>>,
}

impl Editor {
    /// An editor over `text` (line ends normalised by `TextArea::with_text`), cursor at byte 0.
    fn new(target: Target, token: i32, from: Option<i32>, text: &str) -> Self {
        let area = TextArea::with_text(text);
        Self {
            target,
            token,
            from,
            original: area.text().to_owned(),
            area,
            esc_armed: false,
            help: None,
        }
    }

    /// The version a save writes.
    fn saves(&self) -> i32 {
        self.token + 1
    }
}

/// Lengths, never the text: `original` is a body, as the draft is (`TextArea`'s rule).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("target", &self.target)
            .field("token", &self.token)
            .field("from", &self.from)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("esc_armed", &self.esc_armed)
            .field("help", &self.help)
            .finish()
    }
}

/// An `$EDITOR` handoff in flight: the editor the outcome opens, or returns to.
#[derive(Debug)]
struct Pending {
    /// The editor.
    editor: Editor,
    /// Whether the handoff came from the in-app editor (`Ctrl+E`) rather than from Browse (`E`).
    resume: bool,
}

/// The write in flight and what it carries (D98): the bodies the estimate and the "later edits
/// kept" rule compare with, and the attachment's change and name for the landing and the stale
/// sentence. Nothing else: which skill or row was written, its name and its new token are the
/// reply's outcome (MOD-59), never a search for what was sent. Custom `Debug`: body lengths only.
pub(super) enum Sent {
    /// `CreateSkill`.
    Create {
        /// Version 1's body.
        body: String,
    },
    /// `EditSkill`: the outcome names the skill and its name.
    Rename,
    /// `SaveSkillVersion`.
    Version {
        /// The body.
        body: String,
    },
    /// `SetSkillBinding`.
    Binding {
        /// Attach or detach.
        change: BindingChange,
        /// What the notice calls the row.
        target: String,
    },
}

impl core::fmt::Debug for Sent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Create { body } => f
                .debug_struct("Create")
                .field("body_len", &body.len())
                .finish(),
            Self::Rename => f.write_str("Rename"),
            Self::Version { body } => f
                .debug_struct("Version")
                .field("body_len", &body.len())
                .finish(),
            Self::Binding { change, target } => f
                .debug_struct("Binding")
                .field("change", change)
                .field("target", target)
                .finish(),
        }
    }
}

/// One line of report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Notice {
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

/// One report row: its sign, its text, and whether it is something the maintainer must act on.
///
/// The sign is the outcome's own verb — `+` written, `=` unchanged, `!` refused, `\u{b7}` left
/// alone — because a report is read down a column, and the word for each would be the same word
/// nine times over.
fn outcome_row(outcome: &ImportOutcome) -> (char, String, bool) {
    match outcome {
        ImportOutcome::Imported { name, version, .. }
        | ImportOutcome::Updated { name, version, .. } => {
            ('+', format!("{name} v{version}"), false)
        }
        ImportOutcome::Unchanged { name, path } => {
            ('=', format!("{name} \u{2014} unchanged ({path})"), false)
        }
        ImportOutcome::Refused { path, message } => {
            ('!', format!("{path} \u{2014} {message}"), true)
        }
        ImportOutcome::Skipped { path, reason } => {
            ('\u{b7}', format!("{path} \u{2014} {reason}"), true)
        }
    }
}

/// A `CONTROL` chord (`SHIFT` allowed).
fn chord(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
}

impl LibraryView {
    /// The current mode's stack (MOD-67 M4 D4): the attachments pane's while it is open, else an
    /// open agent help's, else the mode's. `SkillsTab::key_stack`, `on_key` and the hint read it.
    pub(super) fn key_stack(&self) -> Option<Stack<'static>> {
        if self.attach.is_some() {
            return None;
        }
        match &self.mode {
            Mode::Browse => Some(views::LIBRARY_BROWSE),
            Mode::Report { .. } => Some(views::LIBRARY_REPORT),
            _ => None,
        }
    }

    /// [`key_stack`](Self::key_stack), which always has one: what `render` reads.
    fn stack(&self) -> Stack<'static> {
        self.key_stack().unwrap_or(views::LIBRARY_BROWSE)
    }

    /// Whether an editor, a prompt, the rename form, or the attachments pane's form, picker or
    /// question is taking every key (D100).
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse | Mode::Report { .. })
            || self.attach.as_ref().is_some_and(AttachPane::captures_input)
    }

    /// A bracketed paste into the open prompt, form or editor (MOD-22 review M-1); `false` when
    /// nothing here is taking text.
    pub(super) fn on_paste(&mut self, text: &str) -> bool {
        if let Some(attach) = &mut self.attach {
            return attach.on_paste(text);
        }
        match &mut self.mode {
            Mode::Browse | Mode::Report { .. } => return false,
            Mode::Naming { field }
            | Mode::Describing { field, .. }
            | Mode::ImportPath { field } => {
                field.on_paste(text);
            }
            Mode::Info(form) => {
                let field = if form.focus == 0 {
                    &mut form.name
                } else {
                    &mut form.description
                };
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
                editor.esc_armed = false;
                self.notice = None;
            }
        }
        true
    }

    /// The scope changed: the library, the editor, the pane, the pending handoff and the write in
    /// flight all belong to the workspace that was left. The notice survives, as in the Templates
    /// view.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if self.attach.is_some() {
            return self.on_attach_key(key, ctx);
        }
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Naming { .. } | Mode::Describing { .. } => self.on_naming_key(key),
            Mode::Info(_) => self.on_info_key(key, ctx),
            Mode::Editing(_) => self.on_editor_key(key, ctx),
            Mode::ImportPath { .. } => self.on_import_key(key, ctx),
            Mode::Report { .. } => self.on_report_key(key, ctx),
        }
    }

    /// A reply addressed to the Skills tab (§6.4).
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        // MOD-55: the open help sees every reply first (A-1: its per-state table is what makes a
        // stray frame harmless). Its frames match none of the arms below; an editor and the
        // attachments pane are never open together.
        if let Mode::Editing(editor) = &mut self.mode
            && let Some(help) = editor.help.as_mut()
        {
            let outcome = help.on_reply(reply, ctx);
            if outcome != HelpOutcome::Consumed {
                self.apply_help(outcome);
            }
        }
        match reply {
            StoreReply::Skills(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                // A read never lands a write (MOD-59 D4): only the write's own reply does.
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                self.clamp();
            }
            StoreReply::SkillWritten { snapshot, outcome } => match snapshot {
                Ok(snapshot) => {
                    if !in_scope(snapshot, ctx) {
                        return;
                    }
                    self.snapshot = Some((**snapshot).clone());
                    self.unavailable = None;
                    self.land(outcome, None);
                    self.clamp();
                }
                // No snapshot to check the scope by: only the write in flight owns this reply, and
                // a scope change has already forgotten that (`on_scope_change`).
                Err(why) if self.busy == Some(outcome.request_name()) => {
                    if self.snapshot.is_none() {
                        self.unavailable = Some(why.clone());
                    }
                    self.land(outcome, Some(why));
                }
                Err(_) => {}
            },
            StoreReply::SkillsStale { snapshot, what } => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy.take().is_some() {
                    let detach = matches!(
                        self.sent.take(),
                        Some(Sent::Binding {
                            change: BindingChange::Detach,
                            ..
                        })
                    );
                    self.stale(*what, detach);
                }
                self.clamp();
            }
            StoreReply::SkillImports(imports) => {
                if !in_scope(&imports.snapshot, ctx) {
                    return;
                }
                self.snapshot = Some(imports.snapshot.clone());
                self.unavailable = None;
                self.land_import(&imports.report);
                self.clamp();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            // Only the write in flight: a refusal of one `on_scope_change` dropped must not free
            // the next (the Requirements tab's guard; MOD-59 review L8).
            StoreReply::Failed { request, message }
                if REQUEST_NAMES.contains(request) && self.busy == Some(*request) =>
            {
                // A refused write leaves the draft, the form or the question's row as it was:
                // nothing was written.
                self.busy = None;
                self.sent = None;
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    /// The `$EDITOR` handoff came back. No handoff pending (the scope changed meanwhile, or the
    /// Templates view asked for it): ignored (D100).
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        let Some(Pending { mut editor, resume }) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(text) => {
                editor.area = TextArea::with_text(&text);
                editor.esc_armed = false;
                self.notice = Some(match skill_body_refusal(editor.area.text()) {
                    Some(sentence) => Notice::Error(sentence),
                    None => Notice::Info(EDITED.to_owned()),
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

    /// Draws the view below the switch line: the content, the notice row and the hint row.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The notice wraps rather than clips (at most `NOTICE_LINES`): the stale sentence ends on
        // the version the next `Ctrl+S` writes.
        let (notice, style) = match &self.notice {
            Some(Notice::Info(text)) => (text.as_str(), ctx.theme.dim),
            Some(Notice::Error(text)) => (text.as_str(), ctx.theme.error),
            None => ("", ctx.theme.dim),
        };
        let mut notice = wrapped(notice, usize::from(area.width.saturating_sub(1)).max(1));
        notice.truncate(NOTICE_LINES);
        let notice_height = u16::try_from(notice.len().max(1)).unwrap_or(1);
        let [content, notice_row, hint_row] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(notice_height),
            Constraint::Length(1),
        ])
        .areas(area);
        let hint = match (&self.attach, &self.snapshot, &self.mode, self.handed_off()) {
            (Some(pane), Some(snapshot), _, _) => {
                pane.render(frame, content, snapshot, ctx).to_owned()
            }
            (_, _, Mode::Editing(editor), _) => {
                self.render_editor(frame, content, editor, ctx);
                match &editor.help {
                    Some(help) => help.hint(ctx.keys()),
                    None => {
                        let (line, col) = editor.area.cursor_line_col();
                        format!("{EDIT_HINT}  L{}:C{}", line + 1, col + 1)
                    }
                }
            }
            (_, _, Mode::Browse, Some(editor)) => {
                self.render_editor(frame, content, editor, ctx);
                ctx.keys().hint(self.stack(), HANDED_OFF_HINT)
            }
            (_, _, Mode::Report { outcomes, cursor }, _) => {
                self.render_report(frame, content, outcomes, *cursor, ctx);
                ctx.keys().hint(self.stack(), REPORT_HINT)
            }
            (_, _, mode, _) => {
                self.render_browse(frame, content, ctx);
                match mode {
                    Mode::Naming { .. } | Mode::Describing { .. } => NAMING_HINT.to_owned(),
                    Mode::Info(_) => INFO_HINT.to_owned(),
                    Mode::ImportPath { .. } => IMPORT_HINT.to_owned(),
                    Mode::Browse | Mode::Editing(_) | Mode::Report { .. } => {
                        ctx.keys().hint(self.stack(), BROWSE_HINT)
                    }
                }
            }
        };
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(format!(" {line}"), style))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {hint}"), ctx.theme.dim)),
            hint_row,
        );
    }

    // --- keys ----------------------------------------------------------------------------------

    /// Browse (§6.3; MOD-67 M4 L-A §2): each candidate act of the chord through
    /// `views::LIBRARY_BROWSE`, the first this view uses taken. Chord equality includes the
    /// modifiers, so `ctrl-e` is not `e`. Anything else passes for the shell to resolve through the
    /// same stack (`q`, `Tab`, the digits, `?`). The tab took `skills.switch_view` first.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::LIBRARY_BROWSE, chord) {
            match act {
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::Reload => {
                    self.notice = None;
                    ctx.request(StoreRequest::Skills(ctx.scope.clone()));
                }
                Act::New => {
                    if self.snapshot.is_some() {
                        self.notice = None;
                        self.mode = Mode::Naming {
                            field: TextField::new(),
                        };
                    }
                }
                Act::LibraryImport => {
                    self.notice = None;
                    self.mode = Mode::ImportPath {
                        field: TextField::new(),
                    };
                }
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.pane_rows.get());
                }
                Act::SkillsPrevVersion
                | Act::SkillsNextVersion
                | Act::SkillsBase
                | Act::SkillsDiff
                | Act::Edit
                | Act::SkillsEditExternally
                | Act::LibraryInfo
                | Act::LibraryAttach => {
                    self.notice = None;
                    self.on_skill_key(act, ctx);
                }
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// A version, diff, edit, rename or attach key. With no skill under the cursor (an empty
    /// library, or nothing read yet) it does nothing. A skill with no version row (only a
    /// hand-written row reaches that) opens `e`/`E` on an empty body over head token 0, which the
    /// writer saves as v1; `i` and `a` work as for any skill; the version, base and diff keys have
    /// nothing to act on (MOD-9 D127, D134).
    fn on_skill_key(&mut self, act: Act, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(entry) = snapshot.skills.get(self.cursor) else {
            return;
        };
        let (skill, name) = (entry.skill.id, entry.skill.name.clone());
        let versions: Vec<i32> = entry.versions.iter().map(|row| row.version).collect();
        // MOD-9 D127: no early return on a missing head any more — only the keys that read a
        // version need one.
        let head = snapshot.head(skill).map(|row| row.version);
        let shown = self
            .shown_row(snapshot, skill)
            .map(|row| (row.version, row.body.clone()));
        let versioned = head.zip(shown.as_ref().map(|(version, _)| *version));
        // What `e` and `E` edit: the shown version over the head's token, or, with no version,
        // an empty body over token 0 (`add_skill_version`'s "no version yet", MOD-9 D127).
        let (token, from, body) = match (head, shown) {
            (Some(head), Some((version, body))) => (head, Some(version), body),
            _ => (0, None, String::new()),
        };
        match act {
            Act::SkillsPrevVersion | Act::SkillsNextVersion => {
                let Some((head, shown_version)) = versioned else {
                    return;
                };
                let index = versions
                    .iter()
                    .position(|version| *version == shown_version)
                    .unwrap_or(0);
                let index = if act == Act::SkillsPrevVersion {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(versions.len().saturating_sub(1))
                };
                let version = versions.get(index).copied().unwrap_or(head);
                self.shown = (version != head).then_some(version);
                self.scroll.reset();
            }
            Act::SkillsBase => {
                let Some((_, shown_version)) = versioned else {
                    return;
                };
                self.base = Some(shown_version);
                self.notice = Some(Notice::Info(format!("base v{shown_version}")));
            }
            Act::SkillsDiff => {
                let Some((_, shown_version)) = versioned else {
                    return;
                };
                if self.pane == Pane::Diff {
                    self.pane = Pane::Body;
                } else if self.base.is_none() && !versions.contains(&(shown_version - 1)) {
                    self.notice = Some(Notice::Info(format!(
                        "v{shown_version} has no earlier version"
                    )));
                    return;
                } else {
                    self.pane = Pane::Diff;
                }
                self.scroll.reset();
            }
            Act::Edit => {
                let target = Target::Version { skill, name };
                self.mode = Mode::Editing(Editor::new(target, token, from, &body));
            }
            Act::SkillsEditExternally => {
                ctx.emit(Action::EditExternally(ExternalEdit {
                    text: body.clone(),
                    stem: name.clone(),
                }));
                let target = Target::Version { skill, name };
                self.external = Some(Pending {
                    editor: Editor::new(target, token, from, &body),
                    resume: false,
                });
            }
            Act::LibraryInfo => {
                self.mode = Mode::Info(InfoForm {
                    skill,
                    token: entry.skill.updated_at,
                    name: TextField::with_text(&name),
                    description: TextField::with_text(&entry.skill.description),
                    focus: 0,
                });
            }
            Act::LibraryAttach => self.attach = Some(AttachPane::new(skill)),
            _ => {}
        }
    }

    /// `n`'s two prompts. A refused name keeps the prompt open, so a typo is one `Backspace` away.
    fn on_naming_key(&mut self, key: KeyEvent) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Naming { field } | Mode::Describing { field, .. } => field.on_key(key),
            _ => return Handled::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => Handled::Pass,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                match core::mem::take(&mut self.mode) {
                    Mode::Naming { field } => self.named(field),
                    Mode::Describing { name, field } => {
                        let description = field.text().unwrap_or_default().to_owned();
                        self.notice = None;
                        self.mode = Mode::Editing(Editor::new(
                            Target::New { name, description },
                            0,
                            None,
                            "",
                        ));
                    }
                    other => self.mode = other,
                }
                Handled::Consumed
            }
        }
    }

    /// The import form (import plan D102). `Enter` sends one path and returns to Browse, so the
    /// library is on screen while the worker walks; `Esc` closes it and sends nothing.
    fn on_import_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let Mode::ImportPath { field } = &mut self.mode else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => Handled::Pass,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                let path = field.text().unwrap_or_default().trim().to_owned();
                if path.is_empty() {
                    self.notice = Some(Notice::Error(ENTER_A_PATH.to_owned()));
                } else if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else {
                    self.busy = Some(IMPORT_NAME);
                    self.notice = Some(Notice::Info(IMPORTING.to_owned()));
                    self.mode = Mode::Browse;
                    ctx.request(StoreRequest::ImportSkills {
                        scope: ctx.scope.clone(),
                        paths: vec![path],
                    });
                }
                Handled::Consumed
            }
        }
    }

    /// The import report, through `views::LIBRARY_REPORT`: `list.down`/`up` move, the pane acts
    /// scroll, `common.reload` re-reads the library behind it, `common.back` returns to Browse.
    /// Every other key passes, so the tab and the shell keep theirs.
    fn on_report_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::LIBRARY_REPORT, chord) {
            let Mode::Report { outcomes, cursor } = &mut self.mode else {
                return Handled::Pass;
            };
            match act {
                Act::Back => {
                    self.mode = Mode::Browse;
                    self.notice = None;
                    self.scroll.reset();
                }
                Act::ListDown => *cursor = (*cursor + 1).min(outcomes.len().saturating_sub(1)),
                Act::ListUp => *cursor = cursor.saturating_sub(1),
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.pane_rows.get());
                }
                Act::Reload => ctx.request(StoreRequest::Skills(ctx.scope.clone())),
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// `Enter` on a new name (D71): an invalid or taken name is refused and the prompt stays;
    /// otherwise the description prompt opens.
    fn named(&mut self, field: TextField) {
        let name = field.text().unwrap_or_default().to_owned();
        let refusal = if validate_name(&name) {
            self.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.by_name(&name).is_some())
                .then(|| format!("`{name}` exists \u{2014} select it and press e"))
        } else {
            Some(invalid_skill_name(&name))
        };
        if let Some(refusal) = refusal {
            self.notice = Some(Notice::Error(refusal));
            self.mode = Mode::Naming { field };
            return;
        }
        self.notice = None;
        self.mode = Mode::Describing {
            name,
            field: TextField::new(),
        };
    }

    /// The rename form: `Tab`/`Shift+Tab`/arrows move, `Enter` or `Ctrl+S` save, `Esc` closes.
    fn on_info_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if chord(&key) && matches!(key.code, KeyCode::Char('s' | 'S')) {
            self.save_info(ctx);
            return Handled::Consumed;
        }
        let Mode::Info(form) = &mut self.mode else {
            return Handled::Pass;
        };
        let field = if form.focus == 0 {
            &mut form.name
        } else {
            &mut form.description
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.save_info(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                if let Some(busy) = self.busy {
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else {
                    self.mode = Mode::Browse;
                    self.notice = None;
                }
                Handled::Consumed
            }
            FieldOutcome::Pass => match key.code {
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                    form.focus = 1 - form.focus;
                    Handled::Consumed
                }
                _ if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
                _ => Handled::Consumed,
            },
        }
    }

    /// The rename form's save: a write in flight; a name `validate_name` refuses; nothing changed;
    /// otherwise `EditSkill` with only the changed fields (D76).
    fn save_info(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let (Mode::Info(form), Some(snapshot)) = (&self.mode, &self.snapshot) else {
            return;
        };
        let name = form.name.text().unwrap_or_default().to_owned();
        let description = form.description.text().unwrap_or_default().to_owned();
        if !validate_name(&name) {
            self.notice = Some(Notice::Error(invalid_skill_name(&name)));
            return;
        }
        let Some(entry) = snapshot.entry(form.skill) else {
            self.notice = Some(Notice::Error(SKILL_GONE.to_owned()));
            return;
        };
        let patch = SkillPatch {
            name: (name != entry.skill.name).then_some(name),
            description: (description != entry.skill.description).then_some(description),
        };
        if patch == SkillPatch::default() {
            self.notice = Some(Notice::Info(NO_CHANGES.to_owned()));
            return;
        }
        let (skill, token) = (form.skill, form.token);
        self.send(
            StoreRequest::EditSkill {
                scope: ctx.scope.clone(),
                skill,
                expected: token,
                patch,
            },
            Sent::Rename,
            ctx,
        );
    }

    /// The editor: `Ctrl+S` (the area's `Submit`), `Ctrl+G`, `Ctrl+E` and `Esc` are the view's;
    /// `Tab` and `Shift+Tab` pass so the shell switches tabs with the draft kept; everything else
    /// is text. An open agent help (MOD-55) takes every key first: the draft is locked under it,
    /// and `Ctrl+S`/`Ctrl+E`/`Ctrl+G` are refused until it closes.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if let Mode::Editing(editor) = &mut self.mode
            && let Some(help) = editor.help.as_mut()
        {
            let outcome = help.on_key(key, ctx);
            return self.apply_help(outcome);
        }
        if chord(&key) && matches!(key.code, KeyCode::Char('g' | 'G')) {
            self.open_help(ctx);
            return Handled::Consumed;
        }
        if chord(&key) && matches!(key.code, KeyCode::Char('e' | 'E')) {
            self.hand_off(ctx);
            return Handled::Consumed;
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
                    editor.esc_armed = false;
                    self.notice = None;
                }
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                self.save_editor(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                if let Some(busy) = busy {
                    // The save's reply closes the editor or keeps it; leaving now would leave the
                    // reply with no editor to land on.
                    self.notice = Some(Notice::Error(in_flight(busy)));
                } else if editor.esc_armed || editor.area.text() == editor.original {
                    self.mode = Mode::Browse;
                    self.notice = None;
                } else {
                    editor.esc_armed = true;
                    self.notice = Some(Notice::Info(UNSAVED.to_owned()));
                }
                Handled::Consumed
            }
            FieldOutcome::Pass => Handled::Pass,
        }
    }

    /// `Ctrl+S` in the editor, first match wins: a write in flight; a blank (or NUL) body, nothing
    /// sent (D77); a version that says what the head says (the view prevents the duplicate the
    /// store would append); otherwise `CreateSkill` or `SaveSkillVersion`.
    fn save_editor(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Editing(editor) = &self.mode else {
            return;
        };
        let body = editor.area.text().to_owned();
        if let Some(sentence) = skill_body_refusal(&body) {
            self.notice = Some(Notice::Error(sentence));
            return;
        }
        let (request, sent) = match &editor.target {
            Target::New { name, description } => (
                StoreRequest::CreateSkill {
                    scope: ctx.scope.clone(),
                    name: name.clone(),
                    description: description.clone(),
                    body: TemplateBody::new(body.clone()),
                },
                Sent::Create { body },
            ),
            Target::Version { skill, .. } => {
                let head = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.head(*skill));
                if let Some(head) = head.filter(|head| head.body == body) {
                    self.notice = Some(Notice::Info(format!(
                        "v{} already says this \u{2014} nothing to save",
                        head.version
                    )));
                    return;
                }
                (
                    StoreRequest::SaveSkillVersion {
                        scope: ctx.scope.clone(),
                        skill: *skill,
                        expected: editor.token,
                        body: TemplateBody::new(body.clone()),
                    },
                    Sent::Version { body },
                )
            }
        };
        self.send(request, sent, ctx);
    }

    /// `Ctrl+G` (MOD-55 P7, P8): the agent help opens on the draft as it is (blank for a new
    /// skill), for the active project, the scope's first, as the Chat tab picks it. Not while a
    /// save is in flight, for `Esc`'s reason; not with no project, which a run needs.
    fn open_help(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Some(project) = ctx.projects.first().map(|project| project.project_id) else {
            self.notice = Some(Notice::Error(NO_PROJECT.to_owned()));
            return;
        };
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        editor.help = Some(Box::new(AgentHelp::open(
            HelpTarget::Skill {
                name: editor.target.name().to_owned(),
            },
            project,
            editor.area.text(),
            ctx,
        )));
        editor.esc_armed = false;
        self.notice = None;
    }

    /// What the open help's key or reply did. An accepted proposal replaces the draft as the
    /// `$EDITOR` return does (`on_external_edit`'s `Edited` arm): skills have no `parse`, so the
    /// only check is `skill_body_refusal`'s, shown and not enforced; `Ctrl+S` stays the gate.
    /// `original` is left alone, so `Esc` still asks first.
    fn apply_help(&mut self, outcome: HelpOutcome) -> Handled {
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
                editor.esc_armed = false;
                self.notice = Some(match skill_body_refusal(editor.area.text()) {
                    Some(sentence) => Notice::Error(sentence),
                    None => Notice::Info(ACCEPTED.to_owned()),
                });
            }
        }
        Handled::Consumed
    }

    /// `Ctrl+E`: the draft goes to `$EDITOR`, and the editor waits in the pending handoff. Not
    /// while a save is in flight, for `Esc`'s reason.
    fn hand_off(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let Mode::Editing(editor) = core::mem::take(&mut self.mode) else {
            return;
        };
        ctx.emit(Action::EditExternally(ExternalEdit {
            text: editor.area.text().to_owned(),
            stem: editor.target.name().to_owned(),
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

    /// A key while the attachments pane is open: the pane decides, the view sends.
    fn on_attach_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let (Some(pane), Some(snapshot)) = (&mut self.attach, &self.snapshot) else {
            self.attach = None;
            return Handled::Pass;
        };
        // The form's save is in flight: its reply closes the form or keeps it, so `Esc` waits for
        // it, as the editor's does.
        if let Some(busy) = self.busy
            && pane.in_form()
            && key.code == KeyCode::Esc
        {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return Handled::Consumed;
        }
        match pane.on_key(key, snapshot, ctx) {
            AttachOutcome::Consumed => {}
            AttachOutcome::Pass => return Handled::Pass,
            AttachOutcome::Close => self.attach = None,
            AttachOutcome::Notice(notice) => self.notice = Some(notice),
            AttachOutcome::Save { request, sent } => match self.busy {
                Some(busy) => self.notice = Some(Notice::Error(in_flight(busy))),
                None => self.send(*request, *sent, ctx),
            },
        }
        Handled::Consumed
    }

    /// Every write (D98): `busy`, what it carries, the notice, the request.
    fn send(&mut self, request: StoreRequest, sent: Sent, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.sent = Some(sent);
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(request);
    }

    // --- replies -------------------------------------------------------------------------------

    /// MOD-59 D4: the write in flight landed. `outcome` is the store's own answer, so nothing here
    /// searches the snapshot for what was sent; a reply for any other write, and every plain
    /// `Skills` read, leaves the draft, the token and `busy` alone. `unread` is the re-read's
    /// failure (D5): the write landed, and the notice says the library drawn is the one held.
    fn land(&mut self, outcome: &SkillWrite, unread: Option<&str>) {
        if self.busy != Some(outcome.request_name()) {
            return;
        }
        let landed = match (outcome, self.sent.take()) {
            (SkillWrite::Created { skill, name }, Some(Sent::Create { body })) => {
                self.landed_version(*skill, name, 1, &body)
            }
            (SkillWrite::Versioned { skill, version }, Some(Sent::Version { body })) => {
                let name = snapshot_name(self.snapshot.as_ref(), *skill);
                self.landed_version(*skill, &name, *version, &body)
            }
            (SkillWrite::Edited { skill, name }, Some(Sent::Rename)) => {
                self.select(*skill);
                self.mode = Mode::Browse;
                format!("saved `{name}`")
            }
            (SkillWrite::Attached { key, updated_at }, Some(Sent::Binding { change, target })) => {
                let kept = self
                    .attach
                    .as_mut()
                    .is_some_and(|pane| pane.on_landed(*key, &change, Some(*updated_at)));
                if kept {
                    format!("attached to {target} \u{2014} later edits kept, Ctrl+S saves them")
                } else {
                    format!("attached to {target}")
                }
            }
            (SkillWrite::Detached { key }, Some(Sent::Binding { change, target })) => {
                if let Some(pane) = &mut self.attach {
                    pane.on_landed(*key, &change, None);
                }
                format!("detached from {target}")
            }
            // `send` sets `busy` and `sent` together, so the write `busy` names has its own
            // `Sent` and this arm is unreachable. Were it reached, the write has still landed:
            // the draft closes rather than stay "saving…" with nothing in flight (MOD-59 review
            // L1).
            _ => {
                self.busy = None;
                self.mode = Mode::Browse;
                self.attach = None;
                self.notice = Some(Notice::Error(format!(
                    "`{}` landed \u{2014} r reloads",
                    outcome.request_name()
                )));
                return;
            }
        };
        self.busy = None;
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(&landed, why)),
        });
    }

    /// An import came back (import plan D102): the counts in the notice, or the report when a file
    /// was refused or skipped. Only the import's own reply lands it — the reply is its own variant,
    /// so a `Skills` read can never be mistaken for it.
    ///
    /// The report opens only over Browse. Keys typed while the walk was in flight may have opened
    /// an editor or a form, and a report that replaced it would throw a draft away: there the
    /// notice says what the report would have, in counts.
    fn land_import(&mut self, report: &[ImportOutcome]) {
        if self.busy != Some(IMPORT_NAME) {
            return;
        }
        self.busy = None;
        let count =
            |wanted: fn(&ImportOutcome) -> bool| report.iter().filter(|row| wanted(row)).count();
        let written = count(|row| matches!(row, ImportOutcome::Imported { .. }));
        let updated = count(|row| matches!(row, ImportOutcome::Updated { .. }));
        let unchanged = count(|row| matches!(row, ImportOutcome::Unchanged { .. }));
        let refused = count(|row| matches!(row, ImportOutcome::Refused { .. }));
        let skipped = count(|row| matches!(row, ImportOutcome::Skipped { .. }));
        let counts = format!("imported {written}, updated {updated}, unchanged {unchanged}");
        if refused + skipped == 0 {
            self.notice = Some(Notice::Info(if report.is_empty() {
                NOTHING_IMPORTED.to_owned()
            } else {
                counts
            }));
            return;
        }
        if matches!(self.mode, Mode::Browse) && self.attach.is_none() {
            self.notice = None;
            self.scroll.reset();
            self.mode = Mode::Report {
                outcomes: report.to_vec(),
                cursor: 0,
            };
        } else {
            self.notice = Some(Notice::Error(format!(
                "{counts}, refused {refused}, skipped {skipped}"
            )));
        }
    }

    /// A version landed (a create's v1 or an append): the cursor goes onto the skill and the pane
    /// back to its head's body; the answer is the notice's text. `skill` is the write's own
    /// outcome, so a create whose re-read failed, which the held library does not show yet, still
    /// lands (MOD-59 D5; `select` leaves the cursor on an unknown id). Keys typed while the save
    /// was in flight still edited the draft; when they did, the editor stays open on them with the
    /// token at the saved version, so the next `Ctrl+S` appends them (the Templates view's rule).
    fn landed_version(&mut self, skill: SkillId, name: &str, version: i32, body: &str) -> String {
        let tokens = estimate(name, version, body);
        self.select(skill);
        let verb = if version == 1 {
            format!("created `{name}` v1")
        } else {
            format!("saved v{version}")
        };
        let Mode::Editing(editor) = &mut self.mode else {
            return format!("{verb} \u{b7} ~{tokens} tokens");
        };
        if editor.area.text() == body {
            self.mode = Mode::Browse;
            format!("{verb} \u{b7} ~{tokens} tokens")
        } else {
            editor.target = Target::Version {
                skill,
                name: name.to_owned(),
            };
            editor.token = version;
            editor.from = Some(version);
            body.clone_into(&mut editor.original);
            editor.esc_armed = false;
            format!(
                "{verb} \u{b7} ~{tokens} tokens \u{2014} later edits kept, Ctrl+S saves them as \
                 v{}",
                version + 1
            )
        }
    }

    /// A write missed its token, or its row is gone (§6.4's second table): the draft keeps its
    /// text and takes the token as it is now, so the next `Ctrl+S` is a deliberate overwrite.
    fn stale(&mut self, what: StaleWhat, detach: bool) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match what {
            StaleWhat::Version(skill) => match (snapshot.head(skill), &mut self.mode) {
                (Some(head), Mode::Editing(editor)) => {
                    editor.token = head.version;
                    self.notice = Some(Notice::Error(version_changed_elsewhere(head.version)));
                }
                (Some(_), _) => {}
                (None, _) => self.gone(),
            },
            StaleWhat::Skill(skill) => match (snapshot.entry(skill), &mut self.mode) {
                (Some(entry), Mode::Info(form)) => {
                    form.token = entry.skill.updated_at;
                    self.notice = Some(Notice::Error(SKILL_CHANGED_ELSEWHERE.to_owned()));
                }
                (Some(_), _) => {}
                (None, _) => self.gone(),
            },
            StaleWhat::Binding(_) => {
                if let Some(pane) = &mut self.attach {
                    pane.on_stale(snapshot);
                }
                let sentence = if detach {
                    DETACH_CHANGED_ELSEWHERE
                } else {
                    BINDING_CHANGED_ELSEWHERE
                };
                self.notice = Some(Notice::Error(sentence.to_owned()));
            }
        }
    }

    /// The skill a draft belonged to is gone: back to Browse.
    fn gone(&mut self) {
        self.mode = Mode::Browse;
        self.notice = Some(Notice::Error(SKILL_GONE.to_owned()));
    }

    // --- state ---------------------------------------------------------------------------------

    /// The version shown for `skill`: the pinned one, else the head. A pin the snapshot no longer
    /// holds falls back to the head.
    fn shown_row<'a>(
        &self,
        snapshot: &'a SkillsSnapshot,
        skill: SkillId,
    ) -> Option<&'a SkillVersion> {
        self.shown
            .and_then(|version| snapshot.version(skill, version))
            .or_else(|| snapshot.head(skill))
    }

    /// `j`/`k`: one row, no wrap; the version, the base and the pane go back to the head's body.
    fn move_cursor(&mut self, down: bool) {
        let last = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.skills.len().saturating_sub(1));
        self.cursor = if down {
            (self.cursor + 1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
        self.reset_pane();
        self.notice = None;
    }

    /// The cursor onto `skill`, its head's body shown.
    fn select(&mut self, skill: SkillId) {
        if let Some(index) = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .skills
                .iter()
                .position(|entry| entry.skill.id == skill)
        }) {
            self.cursor = index;
        }
        self.reset_pane();
    }

    /// The head's body, from its top.
    fn reset_pane(&mut self) {
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
    }

    /// Keeps the cursors on a row after the snapshot changed; the pane closes if its skill left.
    fn clamp(&mut self) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        self.cursor = self.cursor.min(snapshot.skills.len().saturating_sub(1));
        if let Some(pane) = &mut self.attach {
            if snapshot.entry(pane.skill()).is_some() {
                pane.clamp(snapshot);
            } else {
                self.attach = None;
            }
        }
    }

    // --- frames --------------------------------------------------------------------------------

    /// Browse, the prompts and the rename form: the list on the left, the pane on the right.
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let [list_area, pane_area] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)]).areas(area);

        let list = Block::new().borders(Borders::ALL).title(" Skills ");
        let inner = list.inner(list_area);
        frame.render_widget(list, list_area);
        let lines: Vec<Line<'static>> = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.skills.iter())
            .enumerate()
            .map(|(index, entry)| {
                let head = entry
                    .versions
                    .iter()
                    .map(|row| row.version)
                    .max()
                    .unwrap_or(0);
                let style = if index == self.cursor {
                    theme.selected
                } else {
                    theme.base
                };
                Line::styled(browse_row(&entry.skill.name, head), style)
            })
            .collect();
        let offset = self
            .cursor
            .saturating_sub(usize::from(inner.height).saturating_sub(1));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            inner,
        );

        // The body and the diff wrap; the row count is a character wrap's, a lower bound on the
        // word wrap's (`Scroll`'s rule), so the clamp never scrolls the pane blank.
        let width = pane_area.width.saturating_sub(2);
        let (title, lines) = self.pane(width, ctx);
        let rows: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(usize::from(width).max(1)).max(1))
            .sum();
        self.pane_rows.set(rows);
        let mut block = Block::new().borders(Borders::ALL).title(title);
        let visible = usize::from(pane_area.height.saturating_sub(2));
        if rows > visible || self.scroll.offset() > 0 {
            let hint = format!(" {} ", ctx.keys().hint(views::LIBRARY_BROWSE, SCROLL_HINT));
            block = block.title_bottom(Line::styled(hint, theme.dim).right_aligned());
        }
        let inner = block.inner(pane_area);
        frame.render_widget(block, pane_area);
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            inner,
        );
    }

    /// The right-hand pane's title and lines: a prompt, the rename form, a refusal, the empty
    /// states, the shown body, or the diff. The caller wraps and scrolls them.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        let theme = ctx.theme;
        let prompt = |prompt: String, field: &TextField| {
            let budget =
                width.saturating_sub(u16::try_from(cell_width(&prompt)).unwrap_or(u16::MAX));
            let mut spans = vec![Span::styled(prompt, theme.base)];
            spans.extend(field.line(budget, true, theme).spans);
            (" new skill ".to_owned(), vec![Line::from(spans)])
        };
        match &self.mode {
            Mode::Naming { field } => return prompt("new skill: ".to_owned(), field),
            Mode::Describing { name, field } => {
                return prompt(format!("description of {name}: "), field);
            }
            Mode::Info(form) => return self.info_pane(form, width, theme),
            Mode::ImportPath { field } => {
                let label = "path: ";
                let budget =
                    width.saturating_sub(u16::try_from(cell_width(label)).unwrap_or(u16::MAX));
                let mut spans = vec![Span::styled(label, theme.base)];
                spans.extend(field.line(budget, true, theme).spans);
                return (
                    " import skills ".to_owned(),
                    vec![
                        Line::from(spans),
                        Line::default(),
                        Line::styled(
                            "a SKILL.md or rules file, or a directory: its */SKILL.md, and the \
                             *.md / *.mdc of a rules directory",
                            theme.dim,
                        ),
                    ],
                );
            }
            Mode::Browse | Mode::Editing(_) | Mode::Report { .. } => {}
        }
        let dim = |text: &str| {
            (
                String::new(),
                vec![Line::styled(text.to_owned(), theme.dim)],
            )
        };
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                vec![Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)],
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return dim(NOT_READ);
        };
        let Some(entry) = snapshot.skills.get(self.cursor) else {
            return dim(SELECT_A_SKILL);
        };
        let (skill, name) = (entry.skill.id, entry.skill.name.as_str());
        let (Some(head), Some(shown)) = (snapshot.head(skill), self.shown_row(snapshot, skill))
        else {
            return dim(SELECT_A_SKILL);
        };
        if self.pane == Pane::Body {
            let mut lines = Vec::new();
            if !entry.skill.description.is_empty() {
                lines.push(Line::styled(entry.skill.description.clone(), theme.dim));
                lines.push(Line::default());
            }
            lines.extend(
                shown
                    .body
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), theme.base)),
            );
            return (
                format!(
                    " {name} v{} (head v{}) \u{b7} ~{} tokens ",
                    shown.version,
                    head.version,
                    estimate(name, shown.version, &shown.body)
                ),
                lines,
            );
        }
        let base = self.base.unwrap_or(shown.version - 1);
        let Some(old) = snapshot.version(skill, base) else {
            return (
                format!(" {name} v{} ", shown.version),
                vec![Line::styled(
                    format!("v{} has no base to diff against", shown.version),
                    theme.dim,
                )],
            );
        };
        let unified = diff::unified(
            &old.body,
            &shown.body,
            &format!("{name} v{base}"),
            &format!("{name} v{}", shown.version),
        );
        (
            format!(" diff v{base} \u{2192} v{} ", shown.version),
            diff::lines(&unified, theme),
        )
    }

    /// The rename form's pane: the name and the description, the focused one highlighted.
    fn info_pane(
        &self,
        form: &InfoForm,
        width: u16,
        theme: &Theme,
    ) -> (String, Vec<Line<'static>>) {
        let name = snapshot_name(self.snapshot.as_ref(), form.skill);
        let budget = width.saturating_sub(u16::try_from(INFO_LABEL).unwrap_or(0));
        let line = |label: &str, field: &TextField, focused: bool| {
            let mut spans = vec![Span::styled(format!("{label:<INFO_LABEL$}"), theme.base)];
            spans.extend(field.line(budget, focused, theme).spans);
            Line::from(spans)
        };
        (
            format!(" {name} \u{b7} rename "),
            vec![
                line("name", &form.name, form.focus == 0),
                line("description", &form.description, form.focus == 1),
            ],
        )
    }

    /// The import report over the whole content: one row per outcome, the cursor on one of them,
    /// the rows that need acting on in the error style.
    fn render_report(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        outcomes: &[ImportOutcome],
        cursor: usize,
        ctx: &Ctx<'_>,
    ) {
        let block = Block::new().borders(Borders::ALL).title(" import report ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let width = usize::from(inner.width).max(1);
        let lines: Vec<Line<'static>> = outcomes
            .iter()
            .enumerate()
            .map(|(at, outcome)| {
                let (sign, text, problem) = outcome_row(outcome);
                let marker = if at == cursor { '>' } else { ' ' };
                let style = if problem {
                    ctx.theme.error
                } else {
                    ctx.theme.dim
                };
                Line::from(vec![
                    Span::styled(format!("{marker}{sign} "), ctx.theme.base),
                    Span::styled(text, style),
                ])
            })
            .collect();
        self.pane_rows.set(
            lines
                .iter()
                .map(|line| line.width().div_ceil(width).max(1))
                .sum(),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            inner,
        );
    }

    /// The editor over the whole content, its title carrying the draft's estimate (D82). An open
    /// agent help draws in the draft's place (MOD-55 B-2): a panel under a locked draft, or the
    /// proposal over all of it.
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let draft = match &editor.help {
            Some(help) => help.render(frame, area, ctx.theme),
            None => Some(area),
        };
        if let Some(draft) = draft {
            self.render_draft(frame, draft, editor, ctx);
        }
    }

    /// The draft's block: the name, the versions and the estimate, and the text with its cursor
    /// (dim under an open help, which has the keys). The text's rect is claimed for an in-pane
    /// editor (MOD-57 P2), inside the block, so the name and versions stay visible beside it.
    fn render_draft(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let name = editor.target.name();
        let tokens = estimate(name, editor.saves(), editor.area.text());
        let title = match (&editor.target, editor.from) {
            (Target::New { .. }, _) => {
                format!(" {name} \u{b7} new, saves v1 \u{b7} ~{tokens} tokens ")
            }
            // MOD-9 D127: a skill with no version row has nothing to edit from.
            (Target::Version { .. }, None) => format!(
                " {name} \u{b7} no version yet, saves v{} \u{b7} ~{tokens} tokens ",
                editor.saves()
            ),
            (Target::Version { .. }, Some(from)) => format!(
                " {name} \u{b7} editing from v{from}, saves v{} \u{b7} ~{tokens} tokens ",
                editor.saves()
            ),
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.page.set(inner.height);
        frame.render_widget(
            Paragraph::new(editor.area.lines(
                inner.width,
                inner.height,
                editor.help.is_none(),
                ctx.theme,
            )),
            inner,
        );
        ctx.claim_editor_area(inner);
    }
}

/// A skill's name in `snapshot`, else its id.
fn snapshot_name(snapshot: Option<&SkillsSnapshot>, skill: SkillId) -> String {
    snapshot
        .and_then(|snapshot| snapshot.entry(skill))
        .map_or_else(|| skill.to_string(), |entry| entry.skill.name.clone())
}

/// Whether a snapshot answers the scope the view is in now: a write's reply that crossed a scope
/// change belongs to the workspace that was left.
fn in_scope(snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> bool {
    snapshot
        .projects
        .iter()
        .map(|entry| entry.project)
        .eq(ctx.scope.project_ids.iter().copied())
}

/// One browse row: two spaces, the name fitted to [`NAME_WIDTH`] cells, then ` v` and the head.
fn browse_row(name: &str, head: i32) -> String {
    format!("  {} v{head:<3}", cells::fit(name, NAME_WIDTH))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use chrono::TimeDelta;
    use htui_core::clock::{TestClock, epoch};
    use htui_core::fixtures::ids;
    use htui_core::model::{Attachment, ProjectRef, Scope, SkillBindingKey, StepId};
    use htui_core::store::{BLANK_SKILL_BODY, CasOutcome, MemStore, WriteStore as _};
    use htui_store::Backend;

    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::skills;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crate::ui::tabs::SkillsTab;
    use crate::ui::tabs::skills::agent_help::fixtures as agent_fixtures;
    use htui_agent::event::StopReason;

    /// MOD-60 D1: the name is fitted in cells, so a wide name never pushes the version right.
    #[test]
    fn a_wide_skill_name_keeps_the_version_column() {
        for name in [
            "release-notes".to_owned(),
            "a".repeat(40),
            "\u{6f22}".repeat(3),
            "\u{6f22}".repeat(20),
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}".repeat(12),
        ] {
            let row = browse_row(&name, 7);
            let at = row.find(" v7").expect("the version is on the row");
            assert_eq!(
                cell_width(&row[..at]),
                2 + NAME_WIDTH,
                "{row:?} for {name:?}"
            );
        }
    }

    /// MOD-60: the description prompt carries the runtime skill name, so the field's budget is
    /// the pane less the prompt in cells. A CJK name measured in chars leaves the field 5 cells
    /// too many and the prompt line runs past the pane.
    #[test]
    fn a_wide_skill_name_keeps_the_description_prompt_within_the_pane() {
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let scope = vulkan();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let view = LibraryView {
            mode: Mode::Describing {
                name: "\u{6f22}".repeat(5),
                field: TextField::with_text(&"x".repeat(80)),
            },
            ..LibraryView::default()
        };
        let (_, lines) = view.pane(40, &ctx);
        let drawn: usize = lines[0]
            .spans
            .iter()
            .map(|span| cell_width(&span.content))
            .sum();
        assert_eq!(drawn, 40, "{lines:?}");
    }

    /// The Harness's startup scope: the Graphics workspace and its one project.
    fn vulkan() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// One served reply, as the worker would send it.
    async fn serve(backend: &Backend, request: &StoreRequest) -> StoreReply {
        skills::serve(backend, request)
            .await
            .unwrap_or_else(|err| panic!("the skill request failed: {err}"))
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

    /// A plain key.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// `Ctrl+<c>`.
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// Each char of `text` as a plain key.
    fn type_text(view: &mut LibraryView, text: &str, ctx: &mut Ctx<'_>) {
        for c in text.chars() {
            view.on_key(key(KeyCode::Char(c)), ctx);
        }
    }

    /// The snapshot a reply carries, whichever variant it is; a panic when it carries none.
    fn snapshot_of(reply: &StoreReply) -> &SkillsSnapshot {
        match reply {
            StoreReply::Skills(snapshot)
            | StoreReply::SkillWritten {
                snapshot: Ok(snapshot),
                ..
            }
            | StoreReply::SkillsStale { snapshot, .. } => snapshot,
            other => panic!("no snapshot in {other:?}"),
        }
    }

    /// HANDOFF MOD-59's race: another session wrote between the write and the worker's re-read,
    /// so the write's own reply carries the snapshot `read` saw. A panic when `written` is not a
    /// `SkillWritten` (the worker before MOD-59 answered `Skills`).
    fn raced(written: StoreReply, read: StoreReply) -> StoreReply {
        match (written, read) {
            (StoreReply::SkillWritten { outcome, .. }, StoreReply::Skills(snapshot)) => {
                StoreReply::SkillWritten {
                    snapshot: Ok(snapshot),
                    outcome,
                }
            }
            (written, read) => panic!("not a write and a read: {written:?}, {read:?}"),
        }
    }

    /// MOD-59 D5: the reply to a write that applied although its re-read failed.
    fn unread(outcome: SkillWrite) -> StoreReply {
        StoreReply::SkillWritten {
            snapshot: Err("store unreachable: gone".to_owned()),
            outcome,
        }
    }

    /// MOD-59 D4: a `Skills` read served while a version save is in flight is not the save's
    /// answer: a plain `Skills` never lands; the write's own `SkillWritten` does. `settle` serves
    /// in queue order, save first, so only a direct drive can put a read's reply ahead of the
    /// save's.
    #[tokio::test]
    async fn a_read_reply_does_not_close_the_editor_mid_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = LibraryView::default();
        let untouched = serve(&backend, &StoreRequest::Skills(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);

        // The library is `[rust-style, tests]`: the cursor starts on `rust-style`, head v2.
        view.on_key(key(KeyCode::Char('e')), &mut ctx);
        view.on_key(key(KeyCode::Char('x')), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        let [save] = requests.as_slice() else {
            panic!("exactly the save was sent: {requests:?}");
        };
        let StoreRequest::SaveSkillVersion {
            skill, expected, ..
        } = save
        else {
            panic!("not a version save: {save:?}");
        };
        assert_eq!((*skill, *expected), (ids::SKILL_RUST_STYLE, 2));
        assert_eq!(view.busy, Some("save_skill_version"));

        view.on_reply(&untouched, &mut ctx);
        assert!(
            matches!(&view.mode, Mode::Editing(_)),
            "a read without v3 is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(view.busy, Some("save_skill_version"), "still in flight");

        let saved = serve(&backend, save).await;
        view.on_reply(&saved, &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Info(text)) if text.starts_with("saved v3 \u{b7} ~")
            ),
            "{:?}",
            view.notice
        );
    }

    /// The same rule for the attachment form: a plain `Skills` never lands, so the form stays
    /// open over the read; the write's own `SkillWritten` closes it (MOD-59 D4).
    #[tokio::test]
    async fn a_read_reply_does_not_close_the_attach_form_mid_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = LibraryView::default();
        let untouched = serve(&backend, &StoreRequest::Skills(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);

        // `a` on `rust-style`; `j` from `global` to `vulkan-tutorials`; `Enter` opens a new form.
        view.on_key(key(KeyCode::Char('a')), &mut ctx);
        view.on_key(key(KeyCode::Char('j')), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        assert!(view.captures_input(), "the form takes every key");
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        let [save] = requests.as_slice() else {
            panic!("exactly the attach was sent: {requests:?}");
        };
        let StoreRequest::SetSkillBinding {
            key: written,
            expected,
            ..
        } = save
        else {
            panic!("not an attach: {save:?}");
        };
        assert_eq!(
            (*written, *expected),
            (
                SkillBindingKey {
                    skill: ids::SKILL_RUST_STYLE,
                    project: Some(ids::PROJECT_VULKAN),
                    phase: None,
                },
                None
            )
        );

        view.on_key(key(KeyCode::Esc), &mut ctx);
        assert!(view.captures_input(), "`Esc` waits for the save in flight");
        assert_eq!(
            view.notice,
            Some(Notice::Error(
                "`set_skill_binding` is still in flight".to_owned()
            ))
        );

        view.on_reply(&untouched, &mut ctx);
        assert!(
            view.captures_input(),
            "a read without the row keeps the form"
        );
        assert_eq!(view.busy, Some("set_skill_binding"));

        let saved = serve(&backend, save).await;
        view.on_reply(&saved, &mut ctx);
        assert!(!view.captures_input(), "the row landed: the form closed");
        assert!(view.attach.is_some(), "the pane stays open on its rows");
        assert_eq!(view.busy, None);
        assert!(
            matches!(&view.notice, Some(Notice::Info(text)) if text.starts_with("attached to ")),
            "{:?}",
            view.notice
        );
    }

    /// The editor's "later edits kept" rule for the attachment form: a key typed while the save
    /// was in flight keeps the form open on the landed row's token, so the next `Ctrl+S` writes
    /// the edit over the row just saved; a save with nothing typed after it closes the form.
    #[tokio::test]
    async fn the_attach_form_keeps_edits_typed_during_its_save() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = LibraryView::default();
        let untouched = serve(&backend, &StoreRequest::Skills(scope.clone())).await;
        view.on_reply(&untouched, &mut ctx);
        let written = SkillBindingKey {
            skill: ids::SKILL_RUST_STYLE,
            project: Some(ids::PROJECT_VULKAN),
            phase: None,
        };

        // `a` on `rust-style`; `j` to `vulkan-tutorials`; `Enter` opens a new form (`always`).
        view.on_key(key(KeyCode::Char('a')), &mut ctx);
        view.on_key(key(KeyCode::Char('j')), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        let [save] = requests.as_slice() else {
            panic!("exactly the attach was sent: {requests:?}");
        };

        // `Space` twice on the activation while the save is in flight: `always` becomes `off`.
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        let saved = serve(&backend, save).await;
        view.on_reply(&saved, &mut ctx);
        assert_eq!(view.busy, None, "the save landed");
        assert!(view.captures_input(), "the later edit keeps the form open");
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Info(text))
                    if text.starts_with("attached to ") && text.contains("later edits kept")
            ),
            "{:?}",
            view.notice
        );
        let StoreReply::SkillWritten {
            outcome: SkillWrite::Attached {
                updated_at: token, ..
            },
            ..
        } = &saved
        else {
            panic!("the attach answers with its own outcome: {saved:?}");
        };
        let token = *token;

        // The next save writes the edit over the landed row, and nothing typed after it closes
        // the form.
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        let [again] = requests.as_slice() else {
            panic!("exactly the second attach was sent: {requests:?}");
        };
        let StoreRequest::SetSkillBinding {
            key: rewritten,
            expected,
            change: BindingChange::Attach(attachment),
            ..
        } = again
        else {
            panic!("not an attach: {again:?}");
        };
        assert_eq!(
            (*rewritten, *expected, attachment.activation),
            (written, Some(token), Activation::Off),
            "the kept edit is saved under the landed row's token"
        );
        let saved = serve(&backend, again).await;
        view.on_reply(&saved, &mut ctx);
        assert_eq!(view.busy, None, "the second save landed");
        assert!(
            !view.captures_input(),
            "nothing typed since: the form closed"
        );
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Info(text))
                    if text.starts_with("attached to ") && !text.contains("later edits kept")
            ),
            "{:?}",
            view.notice
        );
    }

    /// What a `Ctx` borrows, held by the test: the Harness's startup scope and a fresh `Emit`.
    struct Bench {
        /// [`vulkan`].
        scope: Scope,
        /// The top bar.
        top_bar: TopBarState,
        /// The default keymap.
        keymap: Keymap,
        /// The default theme.
        theme: Theme,
        /// What the view emitted.
        emit: Emit,
        /// The scope's projects: none, unless [`Bench::in_vulkan`] (MOD-55 P7).
        projects: Vec<ProjectRef>,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                scope: vulkan(),
                top_bar: TopBarState::default(),
                keymap: Keymap::default_global(),
                theme: Theme::default(),
                emit: Emit::default(),
                projects: Vec::new(),
            }
        }

        /// [`Bench::new`] with the scope's one project listed, as the shell lists it: the active
        /// project agent help records its run in.
        fn in_vulkan() -> Self {
            Self {
                projects: vec![ProjectRef {
                    project_id: ids::PROJECT_VULKAN,
                    slug: "vulkan-tutorials".to_owned(),
                    name: "Vulkan".to_owned(),
                    position: 0,
                }],
                ..Self::new()
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keymap,
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

    /// MOD-57 P2 (PD-3, F-12): an open draft claims the text rect inside its block, so the name,
    /// versions and estimate in the block's title stay visible beside an in-pane editor; browse
    /// (where `E` hands a selected row off) draws no draft and claims nothing.
    #[test]
    fn the_draft_claims_its_text_rect_and_browse_claims_nothing() {
        let bench = Bench::new();
        let area = Rect::new(0, 0, 100, 28);
        let drawn = |view: &LibraryView| {
            let cell = Cell::new(None);
            let ctx = bench.ctx().with_editor_area(&cell);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(area.width, area.height))
                    .expect("a test terminal");
            terminal
                .draw(|frame| view.render(frame, area, &ctx))
                .expect("the frame draws");
            (terminal.backend().buffer().clone(), cell.get())
        };

        let (_, claim) = drawn(&LibraryView::default());
        assert_eq!(claim, None, "browse claims nothing");

        let target = Target::New {
            name: "docs-style".to_owned(),
            description: String::new(),
        };
        let view = LibraryView {
            mode: Mode::Editing(Editor::new(target, 0, None, "First line.\nSecond.\n")),
            ..LibraryView::default()
        };
        let (buffer, claim) = drawn(&view);
        let claim = claim.expect("the draft claims its text");
        // The content above the notice and hint rows, inside the draft's border.
        assert_eq!(claim, Rect::new(1, 1, 98, 24));
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
        assert!(
            row(0).contains("docs-style \u{b7} new, saves v1"),
            "{:?}",
            row(0)
        );

        // `Ctrl+E` moves the draft into the pending handoff: while `$EDITOR` runs it is still
        // drawn (locked: the editor has the keys) and its text still claimed, so the in-pane
        // editor lands over the text with the title beside it, not over the whole body.
        let mut view = view;
        let mut ctx = bench.ctx();
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
        assert!(
            title.contains("docs-style \u{b7} new, saves v1"),
            "{title:?}"
        );
        let hint = line(area.bottom() - 1, 0, area.width);
        assert!(hint.contains("the draft is in $EDITOR"), "{hint:?}");

        // Browse's `E` (F-12) has no draft to draw: the pane takes the tab body.
        let target = Target::New {
            name: "docs-style".to_owned(),
            description: String::new(),
        };
        let view = LibraryView {
            external: Some(Pending {
                editor: Editor::new(target, 0, None, "First line.\n"),
                resume: false,
            }),
            ..LibraryView::default()
        };
        assert_eq!(drawn(&view).1, None, "browse's `E` claims nothing");
    }

    /// A `MemStore` whose clock never moves unless the test moves it: every write it stamps
    /// carries the same instant (HANDOFF MOD-59, scenario (b)).
    fn frozen() -> (TestClock, Backend) {
        let clock = TestClock::new();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        (clock, Backend::memory(store))
    }

    /// HANDOFF MOD-59 (a): another session re-describes the skill between our `EditSkill` (a
    /// re-describe too) and the worker's re-read, so the re-read holds their description, not ours.
    /// The edit's own `SkillWritten` still closes the form; before MOD-59 the content match never
    /// held and the form refused `Esc` until the workspace changed.
    #[tokio::test]
    async fn an_edit_lands_although_another_session_redescribed_the_skill_before_the_reread() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        view.on_key(key(KeyCode::Tab), &mut ctx);
        type_text(&mut view, " Mine.", &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let edit = bench.one();
        assert!(matches!(edit, StoreRequest::EditSkill { .. }), "{edit:?}");
        let written = serve(&backend, &edit).await;
        let ours = snapshot_of(&written)
            .entry(ids::SKILL_RUST_STYLE)
            .expect("rust-style is in the re-read")
            .skill
            .updated_at;

        // Another session, over the row our rename left.
        let theirs = backend
            .writer()
            .expect("a memory backend has a writer")
            .update_skill(
                ids::SKILL_RUST_STYLE,
                ours,
                SkillPatch {
                    name: None,
                    description: Some("Theirs.".to_owned()),
                },
            )
            .await
            .expect("the other session's write");
        assert!(matches!(theirs, CasOutcome::Applied(_)), "{theirs:?}");
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;

        view.on_reply(&raced(written, read), &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert_eq!(
            view.notice,
            Some(Notice::Info("saved `rust-style`".to_owned()))
        );
    }

    /// MOD-59 DV-1: an attach whose form was edited during its save stays open, and its next
    /// `Ctrl+S` carries the token of the row *this* write landed, not the re-read's row, which
    /// another session wrote over in between. So their row is met with `SkillsStale`, never
    /// silently overwritten.
    #[tokio::test]
    async fn kept_attach_edits_save_over_the_writes_own_row_not_the_racing_one() {
        let (clock, backend) = frozen();
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let written = SkillBindingKey {
            skill: ids::SKILL_RUST_STYLE,
            project: Some(ids::PROJECT_VULKAN),
            phase: None,
        };

        // `a` on `rust-style`; `j` to `vulkan-tutorials`; `Enter` opens a new form (`always`);
        // `Space` twice while the save is in flight: `always` becomes `off`.
        view.on_key(key(KeyCode::Char('a')), &mut ctx);
        view.on_key(key(KeyCode::Char('j')), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let save = bench.one();
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        let reply = serve(&backend, &save).await;
        let ours = snapshot_of(&reply)
            .binding(written)
            .expect("the attach landed")
            .updated_at;

        // Another session, a second later, over the row our attach left.
        clock.advance(TimeDelta::seconds(1));
        let theirs = backend
            .writer()
            .expect("a memory backend has a writer")
            .set_skill_binding(
                written,
                Some(ours),
                BindingChange::Attach(Attachment {
                    pinned_version: None,
                    position: 1,
                    activation: Activation::Always,
                    globs: Vec::new(),
                    languages: Vec::new(),
                }),
            )
            .await
            .expect("the other session's write");
        assert!(matches!(theirs, CasOutcome::Applied(_)), "{theirs:?}");
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;

        view.on_reply(&raced(reply, read), &mut ctx);
        assert_eq!(view.busy, None, "the save landed");
        assert!(view.captures_input(), "the later edit keeps the form open");
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Info(text)) if text.contains("later edits kept")
            ),
            "{:?}",
            view.notice
        );

        view.on_key(ctrl('s'), &mut ctx);
        let again = bench.one();
        let StoreRequest::SetSkillBinding { expected, .. } = &again else {
            panic!("not an attach: {again:?}");
        };
        assert_eq!(
            *expected,
            Some(ours),
            "the kept edit saves over our own row's token"
        );
        let answer = serve(&backend, &again).await;
        assert!(
            matches!(
                answer,
                StoreReply::SkillsStale {
                    what: StaleWhat::Binding(key),
                    ..
                } if key == written
            ),
            "their row is a conflict, not an overwrite: {answer:?}"
        );
    }

    /// HANDOFF MOD-59 (b): a `MemStore` whose clock does not move stamps a second rename with
    /// the instant the first one left, so the row's `updated_at` equals the token the form carried.
    /// The rename still lands on its own reply; before MOD-59 the `updated_at != token` check never
    /// held and the form wedged.
    #[tokio::test]
    async fn a_rename_at_the_stores_frozen_instant_still_lands() {
        let (_clock, backend) = frozen();
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        // The first rename moves the row from the fixture's instant to the clock's.
        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        view.on_key(key(KeyCode::Tab), &mut ctx);
        type_text(&mut view, "A", &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let first = bench.one();
        let reply = serve(&backend, &first).await;
        view.on_reply(&reply, &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);

        // The second opens on the clock's instant, and the store stamps it with the same one.
        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        view.on_key(key(KeyCode::Tab), &mut ctx);
        type_text(&mut view, "B", &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let second = bench.one();
        let StoreRequest::EditSkill { expected, .. } = &second else {
            panic!("not a rename: {second:?}");
        };
        assert_eq!(*expected, epoch(), "the form opened on the frozen instant");
        let reply = serve(&backend, &second).await;
        let landed = snapshot_of(&reply)
            .entry(ids::SKILL_RUST_STYLE)
            .expect("rust-style is in the re-read");
        assert!(
            landed.skill.description.ends_with("AB"),
            "the rename applied"
        );
        assert_eq!(
            landed.skill.updated_at, *expected,
            "precondition: the row carries the very token the rename sent"
        );

        view.on_reply(&reply, &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert_eq!(
            view.notice,
            Some(Notice::Info("saved `rust-style`".to_owned()))
        );
    }

    /// HANDOFF MOD-59 (b) for an attachment: a change to a row stamped at the store's frozen
    /// instant lands, though the row it leaves carries the token the form sent.
    #[tokio::test]
    async fn an_attach_over_a_row_stamped_at_the_same_instant_still_lands() {
        let (_clock, backend) = frozen();
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        // `a` on `rust-style`; `j` to `vulkan-tutorials`; `Enter` opens a new form; save it.
        view.on_key(key(KeyCode::Char('a')), &mut ctx);
        view.on_key(key(KeyCode::Char('j')), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let first = bench.one();
        let reply = serve(&backend, &first).await;
        view.on_reply(&reply, &mut ctx);
        assert_eq!(view.busy, None);
        assert!(!view.captures_input(), "the first attach closed its form");

        // `Enter` on the row it left: the form opens on the frozen instant; `Space` twice turns
        // `always` into `off`.
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        view.on_key(key(KeyCode::Char(' ')), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let second = bench.one();
        let StoreRequest::SetSkillBinding { expected, .. } = &second else {
            panic!("not an attach: {second:?}");
        };
        assert_eq!(
            *expected,
            Some(epoch()),
            "the form opened on the frozen instant"
        );
        let reply = serve(&backend, &second).await;
        assert!(
            !matches!(reply, StoreReply::SkillsStale { .. }),
            "the change applied: {reply:?}"
        );

        view.on_reply(&reply, &mut ctx);
        assert_eq!(view.busy, None, "the change landed");
        assert!(
            !view.captures_input(),
            "nothing typed since: the form closed"
        );
        assert!(
            matches!(&view.notice, Some(Notice::Info(text)) if text.starts_with("attached to ")),
            "{:?}",
            view.notice
        );
    }

    /// MOD-59 D4: a read served after the save, which already shows the saved version, still is
    /// not the save's answer. Only the save's own `SkillWritten` closes the editor.
    #[tokio::test]
    async fn a_read_that_shows_the_saved_version_does_not_land_the_save() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        view.on_key(key(KeyCode::Char('e')), &mut ctx);
        view.on_key(key(KeyCode::Char('x')), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let save = bench.one();
        let saved = serve(&backend, &save).await;
        let shows = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        assert_eq!(
            snapshot_of(&shows)
                .head(ids::SKILL_RUST_STYLE)
                .map(|row| row.version),
            Some(3),
            "precondition: the read holds the saved version"
        );

        view.on_reply(&shows, &mut ctx);
        assert!(
            matches!(view.mode, Mode::Editing(_)),
            "a read is not the save's answer: {:?}",
            view.mode
        );
        assert_eq!(view.busy, Some("save_skill_version"), "still in flight");

        view.on_reply(&saved, &mut ctx);
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Info(text)) if text.starts_with("saved v3 \u{b7} ~")
            ),
            "{:?}",
            view.notice
        );
    }

    /// MOD-59 D5: a save that applied although its re-read failed closes the editor, says what
    /// landed and that the re-read failed, and keeps drawing the library the view held.
    #[tokio::test]
    async fn a_save_whose_reread_failed_lands_and_keeps_the_library_drawn() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        let before = view.snapshot.clone();

        view.on_key(key(KeyCode::Char('e')), &mut ctx);
        view.on_key(key(KeyCode::Char('x')), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let _save = bench.one();

        view.on_reply(
            &unread(SkillWrite::Versioned {
                skill: ids::SKILL_RUST_STYLE,
                version: 3,
            }),
            &mut ctx,
        );
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(view.sent.is_none(), "{:?}", view.sent);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Error(text))
                    if text.starts_with("saved v3 \u{b7} ~")
                        && text.contains("the re-read failed, r reloads: store unreachable: gone")
            ),
            "{:?}",
            view.notice
        );
        assert_eq!(view.snapshot, before, "the held library stays drawn");
        assert_eq!(view.unavailable, None, "there is a library to draw");
    }

    /// MOD-59 D5: a create whose re-read failed lands with nothing to select, and the pane says
    /// the library is unavailable only because the view held none.
    #[test]
    fn a_create_whose_reread_failed_is_unavailable_only_when_nothing_was_held() {
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let target = Target::New {
            name: "docs-style".to_owned(),
            description: String::new(),
        };
        let mut view = LibraryView {
            busy: Some("create_skill"),
            sent: Some(Sent::Create {
                body: "B.\n".to_owned(),
            }),
            mode: Mode::Editing(Editor::new(target, 0, None, "B.\n")),
            ..LibraryView::default()
        };

        view.on_reply(
            &unread(SkillWrite::Created {
                skill: SkillId::new(),
                name: "docs-style".to_owned(),
            }),
            &mut ctx,
        );
        assert_eq!(view.unavailable.as_deref(), Some("store unreachable: gone"));
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert_eq!(view.busy, None);
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Error(text)) if text.starts_with("created `docs-style` v1")
            ),
            "{:?}",
            view.notice
        );
    }

    /// MOD-59 review L1: `send` stores `busy` and `sent` together, so a landing whose `Sent` is
    /// another write's cannot happen. Were it to, the write has still landed: the draft closes on
    /// an error notice rather than stay "saving\u{2026}" with nothing in flight.
    #[test]
    fn a_landing_without_its_own_sent_falls_back_to_browse() {
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let target = Target::New {
            name: "docs-style".to_owned(),
            description: String::new(),
        };
        let mut view = LibraryView {
            busy: Some("edit_skill"),
            sent: Some(Sent::Create {
                body: "B.\n".to_owned(),
            }),
            mode: Mode::Editing(Editor::new(target, 0, None, "B.\n")),
            notice: Some(Notice::Info(SAVING.to_owned())),
            ..LibraryView::default()
        };

        view.on_reply(
            &unread(SkillWrite::Edited {
                skill: ids::SKILL_RUST_STYLE,
                name: "rust-style".to_owned(),
            }),
            &mut ctx,
        );
        assert_eq!(view.busy, None);
        assert!(view.sent.is_none());
        assert!(matches!(view.mode, Mode::Browse), "{:?}", view.mode);
        assert!(
            matches!(&view.notice, Some(Notice::Error(text)) if text.contains("edit_skill")),
            "{:?}",
            view.notice
        );
    }

    /// MOD-59 D4, H-5: a `SkillWritten` for a write other than the one in flight lands nothing;
    /// with no snapshot it cannot even be scope-checked, so it touches neither the pane nor the
    /// notice.
    #[tokio::test]
    async fn a_skill_written_for_another_write_does_not_land() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        view.on_key(key(KeyCode::Tab), &mut ctx);
        type_text(&mut view, "x", &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let _edit = bench.one();
        assert_eq!(view.busy, Some("edit_skill"));

        let versioned = SkillWrite::Versioned {
            skill: ids::SKILL_RUST_STYLE,
            version: 3,
        };
        view.on_reply(
            &StoreReply::SkillWritten {
                snapshot: Ok(Box::new(snapshot_of(&read).clone())),
                outcome: versioned.clone(),
            },
            &mut ctx,
        );
        assert_eq!(view.busy, Some("edit_skill"), "not the rename's answer");
        assert!(matches!(view.mode, Mode::Info(_)), "{:?}", view.mode);

        view.on_reply(&unread(versioned), &mut ctx);
        assert_eq!(view.busy, Some("edit_skill"));
        assert_eq!(view.unavailable, None);
        assert_eq!(view.notice, Some(Notice::Info(SAVING.to_owned())));
    }

    /// MOD-59 review L8: a late refusal of another skill write (one a scope change dropped) frees
    /// nothing; the refusal of the write in flight does, and the form keeps its text. The
    /// Requirements tab's guard (MOD-39 review #3).
    #[tokio::test]
    async fn only_the_write_in_flight_is_freed_by_its_refusal() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        view.on_key(key(KeyCode::Tab), &mut ctx);
        type_text(&mut view, "x", &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let _edit = bench.one();
        assert_eq!(view.busy, Some("edit_skill"));

        view.on_reply(
            &StoreReply::Failed {
                request: "save_skill_version",
                message: "an older refusal".to_owned(),
            },
            &mut ctx,
        );
        assert_eq!(view.busy, Some("edit_skill"), "still in flight");
        assert!(view.sent.is_some(), "with what it carries");
        assert_eq!(view.notice, Some(Notice::Info(SAVING.to_owned())));

        view.on_reply(
            &StoreReply::Failed {
                request: "edit_skill",
                message: "refused".to_owned(),
            },
            &mut ctx,
        );
        assert_eq!(view.busy, None);
        assert!(view.sent.is_none());
        assert_eq!(view.notice, Some(Notice::Error("refused".to_owned())));
        assert!(
            matches!(view.mode, Mode::Info(_)),
            "the form keeps its text"
        );
    }

    /// MOD-9 D127, D134 (milestone 3's review finding 6): a skill with no version row — only a
    /// hand-written row reaches the state, so the reply is doctored — opens the editor on an
    /// empty body with head token 0, and the save asks the writer for v1. `i` and `a` are not
    /// blocked; the version, base and diff keys have nothing to act on and stay inert.
    #[tokio::test]
    async fn a_skill_with_no_version_opens_an_empty_editor_on_token_0() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = LibraryView::default();
        let StoreReply::Skills(mut snapshot) =
            serve(&backend, &StoreRequest::Skills(scope.clone())).await
        else {
            panic!("the read answers `Skills`");
        };
        // The cursor starts on `rust-style`; strip its versions.
        assert_eq!(snapshot.skills[0].skill.id, ids::SKILL_RUST_STYLE);
        snapshot.skills[0].versions.clear();
        view.on_reply(&StoreReply::Skills(snapshot), &mut ctx);

        for c in [',', '.', 'b', 'd'] {
            view.on_key(key(KeyCode::Char(c)), &mut ctx);
            assert!(matches!(view.mode, Mode::Browse), "`{c}`: {:?}", view.mode);
            assert_eq!(
                (view.shown, view.base, view.pane),
                (None, None, Pane::Body),
                "`{c}` has no version to act on"
            );
            assert_eq!(view.notice, None, "`{c}` says nothing");
        }
        assert!(sent(&emit).is_empty(), "the inert keys send nothing");

        view.on_key(key(KeyCode::Char('i')), &mut ctx);
        assert!(
            matches!(view.mode, Mode::Info(_)),
            "`i` opens the info form (D134): {:?}",
            view.mode
        );
        view.mode = Mode::Browse;
        view.on_key(key(KeyCode::Char('a')), &mut ctx);
        assert!(
            view.attach.is_some(),
            "`a` opens the attachments pane (D134)"
        );
        view.attach = None;

        view.on_key(key(KeyCode::Char('E')), &mut ctx);
        let pending = view
            .external
            .take()
            .expect("`E` hands an editor to $EDITOR");
        assert_eq!(
            (
                pending.editor.token,
                pending.editor.from,
                pending.editor.area.text()
            ),
            (0, None, "")
        );
        let _ = emit.take();

        view.on_key(key(KeyCode::Char('e')), &mut ctx);
        let Mode::Editing(editor) = &view.mode else {
            panic!("`e` opens the editor: {:?}", view.mode);
        };
        assert_eq!(
            (editor.token, editor.from, editor.area.text()),
            (0, None, ""),
            "an empty body over head token 0"
        );
        for c in "Body.".chars() {
            view.on_key(key(KeyCode::Char(c)), &mut ctx);
        }
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        let [
            StoreRequest::SaveSkillVersion {
                skill,
                expected,
                body,
                ..
            },
        ] = requests.as_slice()
        else {
            panic!("exactly one version save was sent: {requests:?}");
        };
        assert_eq!(
            (*skill, *expected, body.as_str()),
            (ids::SKILL_RUST_STYLE, 0, "Body.")
        );
    }

    /// D77: a blank body is refused by the view, and the refusal sends nothing.
    #[tokio::test]
    async fn a_blank_body_dispatches_no_request() {
        let backend = Backend::memory(MemStore::demo());
        let scope = vulkan();
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        view.on_key(key(KeyCode::Char('n')), &mut ctx);
        for c in "blank".chars() {
            view.on_key(key(KeyCode::Char(c)), &mut ctx);
        }
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        for c in "  ".chars() {
            view.on_key(key(KeyCode::Char(c)), &mut ctx);
        }
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        let requests = sent(&emit);
        assert!(requests.is_empty(), "the refused save sent {requests:?}");
        assert_eq!(view.busy, None);
        assert_eq!(
            view.notice,
            Some(Notice::Error(BLANK_SKILL_BODY.to_owned()))
        );
    }

    /// The view derives `Debug` down to the open editor and the write in flight, whose texts are
    /// bodies: lengths only, as the draft's `TextArea`.
    #[test]
    fn an_open_editor_and_a_sent_body_debug_print_lengths_not_text() {
        let editor = Editor::new(
            Target::Version {
                skill: ids::SKILL_TESTS,
                name: "tests".to_owned(),
            },
            1,
            Some(1),
            "secret original",
        );
        let view = LibraryView {
            mode: Mode::Editing(editor),
            sent: Some(Sent::Version {
                body: "secret sent".to_owned(),
            }),
            ..LibraryView::default()
        };
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("original_len: 15"), "{shown}");
        assert!(shown.contains("body_len: 11"), "{shown}");
        let create = format!(
            "{:?}",
            Sent::Create {
                body: "secret body".to_owned(),
            }
        );
        assert!(!create.contains("secret"), "{create}");
    }

    // --- MOD-55: agent help -------------------------------------------------------------------

    /// The open editor.
    fn editor(view: &LibraryView) -> &Editor {
        match &view.mode {
            Mode::Editing(editor) => editor,
            other => panic!("an editor is open: {other:?}"),
        }
    }

    /// A fresh read, then `e` on `rust-style` (the cursor's first row): the editor over v2.
    async fn edit_rust_style(view: &mut LibraryView, bench: &Bench, ctx: &mut Ctx<'_>) {
        let backend = Backend::memory(MemStore::demo());
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, ctx);
        view.on_key(key(KeyCode::Char('e')), ctx);
        assert_eq!(
            editor(view).target.name(),
            "rust-style",
            "the cursor starts on rust-style"
        );
    }

    /// `Ctrl+G`, one enabled agent read, `request` asked. The `EditHelp` it sent.
    fn ask_help(
        view: &mut LibraryView,
        bench: &Bench,
        ctx: &mut Ctx<'_>,
        request: &str,
    ) -> StoreRequest {
        assert_eq!(view.on_key(ctrl('g'), ctx), Handled::Consumed);
        let agents = bench.one();
        assert!(matches!(agents, StoreRequest::Agents), "{agents:?}");
        view.on_reply(
            &StoreReply::Agents(vec![agent_fixtures::summary("scripted", true)]),
            ctx,
        );
        type_text(view, request, ctx);
        view.on_key(key(KeyCode::Enter), ctx);
        bench.one()
    }

    /// The help's turn: accepted, `reply` in one chunk, ended on `EndTurn`.
    fn reply_with(view: &mut LibraryView, ctx: &mut Ctx<'_>, reply: &str) {
        view.on_reply(&agent_fixtures::accepted(StepId::new()), ctx);
        view.on_reply(&agent_fixtures::chunk(reply), ctx);
        view.on_reply(&agent_fixtures::ended(StopReason::EndTurn), ctx);
    }

    /// LV-1 (P7): with no project in the workspace there is nowhere to record the run: `Ctrl+G`
    /// is refused with a notice, no help opens and nothing is sent.
    #[tokio::test]
    async fn ctrl_g_without_a_project_is_refused() {
        let bench = Bench::new();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        let _ = bench.emit.take();
        assert_eq!(view.on_key(ctrl('g'), &mut ctx), Handled::Consumed);
        assert_eq!(view.notice, Some(Notice::Error(NO_PROJECT.to_owned())));
        assert!(editor(&view).help.is_none());
        assert!(bench.emit.take().is_empty(), "nothing is sent");
    }

    /// LV-2 (P7, A-4): the request names the active project (the scope's first) and the skill,
    /// and carries the draft as it was when the help opened.
    #[tokio::test]
    async fn ctrl_g_sends_edit_help_for_the_skill_in_the_first_project() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        let _ = bench.emit.take();
        let request = ask_help(&mut view, &bench, &mut ctx, "shorter");
        let StoreRequest::EditHelp {
            project_id, prompt, ..
        } = &request
        else {
            panic!("an EditHelp: {request:?}");
        };
        assert_eq!(*project_id, ids::PROJECT_VULKAN);
        assert_eq!(
            prompt.target,
            HelpTarget::Skill {
                name: "rust-style".to_owned()
            }
        );
        assert_eq!(
            prompt.body,
            "Prefer `expect` with a reason. One error enum per crate."
        );
        assert_eq!(prompt.request, "shorter");
    }

    /// The blueprint's "an empty body is allowed": a new skill's blank editor offers the help,
    /// named after the skill `n` is creating.
    #[tokio::test]
    async fn ctrl_g_in_a_new_skill_s_blank_editor_asks_for_it_by_name() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);
        view.on_key(key(KeyCode::Char('n')), &mut ctx);
        type_text(&mut view, "fresh", &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        view.on_key(key(KeyCode::Enter), &mut ctx);
        assert_eq!(editor(&view).area.text(), "");
        let request = ask_help(&mut view, &bench, &mut ctx, "write it");
        let StoreRequest::EditHelp { prompt, .. } = &request else {
            panic!("an EditHelp: {request:?}");
        };
        assert_eq!(
            prompt.target,
            HelpTarget::Skill {
                name: "fresh".to_owned()
            }
        );
        assert_eq!(prompt.body, "");
    }

    /// `Ctrl+G` while a save is in flight: the in-flight notice, as `Ctrl+E` and `Esc` say it.
    #[tokio::test]
    async fn ctrl_g_is_refused_while_a_save_is_in_flight() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        view.on_key(key(KeyCode::Char('Z')), &mut ctx);
        view.on_key(ctrl('s'), &mut ctx);
        assert!(matches!(bench.one(), StoreRequest::SaveSkillVersion { .. }));
        view.on_key(ctrl('g'), &mut ctx);
        assert_eq!(
            view.notice,
            Some(Notice::Error(
                "`save_skill_version` is still in flight".to_owned()
            ))
        );
        assert!(editor(&view).help.is_none());
        assert!(bench.emit.take().is_empty());
    }

    /// While the help is open it takes every key and paste: the draft is locked, `Ctrl+S`,
    /// `Ctrl+E` and `Ctrl+G` are refused with the help's notice, and `Esc` leaves the help (not
    /// the editor).
    #[tokio::test]
    async fn an_open_help_locks_the_draft_and_refuses_ctrl_s_and_ctrl_e() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        view.on_key(ctrl('g'), &mut ctx);
        let _ = bench.emit.take();
        let before = editor(&view).area.text().to_owned();
        type_text(&mut view, "zzz", &mut ctx);
        assert!(view.on_paste("pasted"));
        for c in ['s', 'e', 'g'] {
            assert_eq!(view.on_key(ctrl(c), &mut ctx), Handled::Consumed);
            assert_eq!(
                view.notice,
                Some(Notice::Info(
                    "agent help is open \u{2014} Esc leaves it first".to_owned()
                )),
                "Ctrl+{c}"
            );
        }
        assert_eq!(editor(&view).area.text(), before, "the draft is locked");
        assert!(bench.emit.take().is_empty(), "no save, no $EDITOR, no help");
        assert!(view.external.is_none());
        assert!(view.captures_input());

        view.on_key(key(KeyCode::Esc), &mut ctx);
        assert!(editor(&view).help.is_none(), "Esc closes the help");
        assert_eq!(view.notice, None);
        view.on_key(key(KeyCode::Char('z')), &mut ctx);
        assert_eq!(editor(&view).area.text(), format!("z{before}"));
    }

    /// An accepted proposal replaces the draft and sends nothing; `Ctrl+S` then saves it as the
    /// next version. `original` is untouched, so `Esc` still asks first.
    #[tokio::test]
    async fn an_accepted_proposal_replaces_the_draft_and_ctrl_s_saves_it() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        let _ = bench.emit.take();
        ask_help(&mut view, &bench, &mut ctx, "shorter");
        let original = editor(&view).original.clone();
        reply_with(&mut view, &mut ctx, "Here:\n```\nUse `expect`.\n```\n");
        assert!(editor(&view).help.is_some(), "the proposal is shown");
        assert_eq!(
            view.on_key(key(KeyCode::Enter), &mut ctx),
            Handled::Consumed
        );

        let open = editor(&view);
        assert!(open.help.is_none(), "accepting closes the help");
        assert_eq!(
            open.area.text(),
            "Use `expect`.",
            "the sent body's ending kept"
        );
        assert_eq!(open.original, original, "Esc still asks first");
        assert_eq!(view.notice, Some(Notice::Info(ACCEPTED.to_owned())));
        assert!(bench.emit.take().is_empty(), "accepting sends nothing");

        view.on_key(ctrl('s'), &mut ctx);
        let save = bench.one();
        assert!(
            matches!(&save, StoreRequest::SaveSkillVersion { expected: 2, body, .. }
                if body.as_str() == "Use `expect`."),
            "{save:?}"
        );
    }

    /// The save gate still runs after an accept: a proposal that says what the head says is not
    /// saved as a duplicate version, and a blank one is refused (D77). Neither sends anything.
    #[tokio::test]
    async fn an_accepted_proposal_still_meets_the_save_gate() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        // The draft drifts from the head; the agent puts the head's text back.
        view.on_key(key(KeyCode::Char('Z')), &mut ctx);
        let _ = bench.emit.take();
        ask_help(&mut view, &bench, &mut ctx, "undo that");
        reply_with(
            &mut view,
            &mut ctx,
            "```\nPrefer `expect` with a reason. One error enum per crate.\n```\n",
        );
        view.on_key(key(KeyCode::Char('y')), &mut ctx);
        assert_eq!(view.notice, Some(Notice::Info(ACCEPTED.to_owned())));
        view.on_key(ctrl('s'), &mut ctx);
        assert_eq!(
            view.notice,
            Some(Notice::Info(
                "v2 already says this \u{2014} nothing to save".to_owned()
            ))
        );
        assert!(bench.emit.take().is_empty(), "the duplicate is not sent");

        ask_help(&mut view, &bench, &mut ctx, "empty it");
        reply_with(&mut view, &mut ctx, "```\n \n```\n");
        view.on_key(key(KeyCode::Enter), &mut ctx);
        assert_eq!(editor(&view).area.text(), " ");
        assert_eq!(
            view.notice,
            Some(Notice::Error(BLANK_SKILL_BODY.to_owned())),
            "the $EDITOR return's check, on accept"
        );
        view.on_key(ctrl('s'), &mut ctx);
        assert_eq!(
            view.notice,
            Some(Notice::Error(BLANK_SKILL_BODY.to_owned()))
        );
        assert!(bench.emit.take().is_empty(), "the blank body is not sent");
        assert_eq!(view.busy, None);
    }

    /// The editor's `Debug` carries the help's (lengths only): no draft text through it.
    #[tokio::test]
    async fn an_open_help_debugs_without_the_draft() {
        let bench = Bench::in_vulkan();
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        edit_rust_style(&mut view, &bench, &mut ctx).await;
        view.on_key(ctrl('g'), &mut ctx);
        let shown = format!("{:?}", editor(&view));
        assert!(shown.contains("help: Some("), "{shown}");
        assert!(!shown.contains("One error enum"), "{shown}");
    }
}
