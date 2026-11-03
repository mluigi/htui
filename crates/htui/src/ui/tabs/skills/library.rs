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
use ratatui::widgets::{Paragraph, Wrap};

use super::agent_help::{AgentHelp, HelpOutcome, Report, accepted};
use super::attach::{AttachOutcome, AttachPane};
use super::{framed, hint_line, holder, joined, unavailable, view_chips, view_chips_if_room};
use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, edited};
use crate::keys::translate::FieldEvent;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::skill_import::ImportOutcome;
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillEntry, SkillWrite, SkillsSnapshot, StaleWhat};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::TemplateBody;
use crate::ui::cells;
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::list::{self, GUTTER, Row, RowKind, ellipsize, selection_band};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::pane::{Focus, Pane as KitPane, capture_area};
use crate::ui::kit::scroll::{Scroll, Step};
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::shell::{ListSize, PaneId, TabLayout};
use crate::ui::{FieldOutcome, TextArea, TextField, diff};
use crossterm::event::KeyEvent;

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The `StoreRequest::ImportSkills` name: what `busy` holds while a walk is in flight. The slice
/// index rather than a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const IMPORT_NAME: &str = REQUEST_NAMES[5];

/// The list's width, borders included: two spaces, the name fitted to [`NAME_WIDTH`] cells, ` v`
/// and a head of up to three digits make 29 cells.
const LIST_WIDTH: u16 = 32;

/// The Skills tab's default layout (MOD-115 M2 D2, blueprint D-3), the Library's and the
/// Templates view's alike: a [`LIST_WIDTH`]-cell list beside the pane, as
/// `[Length(32), Min(1)]` split them before the shell kept layouts. Cells, not a percentage:
/// none reproduces 32 cells at every width.
pub(super) const LAYOUT: TabLayout = TabLayout {
    list: ListSize::Cells(LIST_WIDTH),
    ..TabLayout::DEFAULT
};

/// A library row's name field, in cells: a name is fitted to it with `cells::fit`.
const NAME_WIDTH: usize = 22;

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";

/// [`NOT_READ`]'s next step (MOD-115 M4 E5).
const NOT_READ_NEXT: &str = "they appear once the store answers";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";

/// The pane with no skill under the cursor (an empty library).
const SELECT_A_SKILL: &str = "select a skill";

/// [`SELECT_A_SKILL`]'s next step in an empty library, through `LIBRARY_BROWSE`.
const EMPTY_LIBRARY_HINT: HintSpec = &[
    Hint::One(Act::New, "creates one"),
    Hint::One(Act::LibraryImport, "imports some"),
];

/// [`EMPTY_LIBRARY_HINT`] with every key unbound.
const EMPTY_LIBRARY_NEXT: &str = "a new or imported skill shows here";

/// [`SELECT_A_SKILL`]'s next step on a skill with no version yet, through `LIBRARY_BROWSE`.
const NO_VERSION_HINT: HintSpec = &[Hint::One(Act::Edit, "writes its first version")];

/// [`NO_VERSION_HINT`] with the key unbound.
const NO_VERSION_NEXT: &str = "its first version shows here";

/// The import form's help, under its path field.
const IMPORT_HELP: &str = "a SKILL.md or rules file, or a directory: its */SKILL.md, and the \
                           *.md / *.mdc of a rules directory";

/// `Esc` over a modified draft, the first time.
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A write went out.
const SAVING: &str = "saving\u{2026}";

/// A `SkillsStale` for a skill the library no longer holds.
const SKILL_GONE: &str = "the skill is gone";

/// A rename over a row that changed since the form opened. `save` is `FormSave`'s label through
/// `LIBRARY_INFO` (MOD-67 M6 D4); with none bound the clause goes.
fn skill_changed_elsewhere(save: Option<&str>) -> String {
    let kept = "changed elsewhere since you opened it \u{2014} your text is kept";
    match save {
        Some(save) => format!("{kept}; {save} saves it over the new row"),
        None => kept.to_owned(),
    }
}

/// An attachment save over a row that changed since the form opened. `save` is `FormSave`'s
/// label through `ATTACH_FORM` (MOD-67 M6 D4); with none bound the clause goes.
fn binding_changed_elsewhere(save: Option<&str>) -> String {
    let kept = "this attachment changed elsewhere \u{2014} your form is kept";
    match save {
        Some(save) => format!("{kept}; {save} saves over it"),
        None => kept.to_owned(),
    }
}

/// A detach over a row that changed or went since the question was asked: nothing was detached,
/// and the pane shows the row as it is now.
const DETACH_CHANGED_ELSEWHERE: &str = "this attachment changed elsewhere \u{2014} nothing was \
                                        detached; its row shows it as it is now";

/// Every Browse key, the shell's activity pane's hint ([`key_hint`](LibraryView::key_hint), M4
/// C8); the list and the body show their own ([`LIST_HINT`], [`DETAIL_HINT`]). 96 cells with the
/// defaults: `j/k` carries no word, and `r reload` is
/// left out, so the row fits 100 columns with its ` \u{b7} ` separators; MOD-67 M4). `I import`
/// took the room `h/l view` had: the view chips name both views (MOD-115 M4 E5).
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

/// The list pane's keys while browsing (MOD-115 M4 E5, Q2: a pane shows its own keys): the
/// list's motion, as every list's hint leads with it, then a new skill and the import, the
/// Browse row's keys before the port that are not the shown skill's; 22 of its 28 cells. The
/// view switch is the chips' (L-skills repair 1: the Browse row never had the room for it).
const LIST_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "new"),
    Hint::One(Act::LibraryImport, "import"),
];

/// The body pane's keys while browsing (MOD-115 M4 E5): the shown skill's, the version and the
/// diff first; [`BROWSE_HINT`] is all of them, on the shell's activity pane.
const DETAIL_HINT: HintSpec = &[
    Hint::Pair(Act::SkillsPrevVersion, Act::SkillsNextVersion, "version"),
    Hint::One(Act::SkillsBase, "base"),
    Hint::One(Act::SkillsDiff, "diff"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::LibraryInfo, "info"),
    Hint::One(Act::LibraryAttach, "attach"),
    Hint::One(Act::SkillsEditExternally, "$EDITOR"),
];

/// The pane's keys while its lines overflow it, through `LIBRARY_BROWSE` whatever the mode:
/// they close the detail pane's hints when it is focused, after the view's own keys, so they are
/// the first a narrow pane drops (MOD-115 M4 E5, T3; MOD-82 carried: no key literal in a title).
const SCROLL_HINT: HintSpec = &[
    Hint::Pair(Act::PaneScrollDown, Act::PaneScrollUp, "scroll"),
    Hint::Pair(Act::PanePageUp, Act::PanePageDown, "page"),
];

/// The hint row while naming or describing a new skill, through `LIBRARY_PROMPT`: the field's
/// `field.submit` names its key in force (MOD-115 M4 MD-A); `Esc` is never rebound (M2 MD7).
const NAMING_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "next"),
    Hint::Text("Esc cancel"),
];

/// The hint row on the rename form (`Esc` is the field's).
const INFO_HINT: HintSpec = &[
    Hint::One(Act::FormNextField, "field"),
    Hint::One(Act::FormSave, "save"),
    Hint::Text("Esc cancel"),
];

/// The hint row in the editor, before the cursor's `L{line}:C{col}` (`Esc` is the area's).
const EDIT_HINT: HintSpec = &[
    Hint::One(Act::FormSave, "save"),
    Hint::One(Act::SkillsAskAgent, "ask agent"),
    Hint::One(Act::FormExternalEditor, "$EDITOR"),
    Hint::Text("Esc cancel"),
];

/// The hint row while a `Ctrl+E` handoff holds the draft: `$EDITOR` has the keys.
const HANDED_OFF_HINT: HintSpec = &[Hint::Text("the draft is in $EDITOR")];

/// `Ctrl+G` with no project in the workspace (MOD-55 P7): a help turn is a chat run, and a run
/// belongs to a project.
const NO_PROJECT: &str = "no project in this workspace \u{2014} agent help records its run in one";

/// The hint row on the import form, through `LIBRARY_PROMPT`: `field.submit` in force (MOD-115
/// M4 MD-A), and the field's `Esc`.
const IMPORT_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "import"),
    Hint::Text("Esc cancel"),
];

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

/// `SkillsStale` over an open editor: the Templates view's sentence (`templates.rs`). `save` is
/// `FormSave`'s label through `LIBRARY_EDITOR` (MOD-67 M6 D4); with none bound the sentence says
/// what saving writes.
fn version_changed_elsewhere(head: i32, save: Option<&str>) -> String {
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

/// A key while a write is in flight: one at a time (`settings/prompt.rs`' rule).
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// MOD-59 D5: a write that applied although its re-read failed. It landed; the view still draws
/// what it held, and `reload` (`Reload`'s label through `LIBRARY_BROWSE`, MOD-67 M6 D4) reads
/// again; with none bound the clause goes.
fn landed_unread(landed: &str, why: &str, reload: Option<&str>) -> String {
    match reload {
        Some(reload) => format!("{landed} \u{2014} the re-read failed, {reload} reloads: {why}"),
        None => format!("{landed} \u{2014} the re-read failed: {why}"),
    }
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
    /// The last outcome, on the view's last rows.
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

/// A prompt's key its field passed (MOD-67 M4 PA-2): `global.next_tab`/`prev_tab` pass, so the
/// shell switches tabs with the prompt kept; anything else is `modal_rest`'s through
/// `views::LIBRARY_PROMPT`.
fn prompt_rest(key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
    let chord = ctx.pressed(&key);
    match ctx.keys().actions(views::LIBRARY_PROMPT, chord).first() {
        Some(Act::NextTab | Act::PrevTab) => Handled::Pass,
        _ => modal_rest(views::LIBRARY_PROMPT, chord),
    }
}

impl LibraryView {
    /// The browse mode's stack, whatever mode the view is in (MOD-115 review).
    pub(super) const fn browse_stack() -> Stack<'static> {
        views::LIBRARY_BROWSE
    }

    /// The current mode's stack (MOD-67 M4 D4), always one: the attachments pane's while it is
    /// open, else an open agent help's, else the mode's. `SkillsTab::key_stack`, `on_key` and
    /// the hint read it.
    pub(super) fn key_stack(&self) -> Stack<'static> {
        if let Some(pane) = &self.attach {
            return pane.key_stack();
        }
        match &self.mode {
            Mode::Browse => views::LIBRARY_BROWSE,
            Mode::Naming { .. } | Mode::Describing { .. } | Mode::ImportPath { .. } => {
                views::LIBRARY_PROMPT
            }
            Mode::Info(_) => views::LIBRARY_INFO,
            Mode::Editing(editor) => match &editor.help {
                Some(help) => help.key_stack(),
                None => views::LIBRARY_EDITOR,
            },
            Mode::Report { .. } => views::LIBRARY_REPORT,
        }
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
            Mode::Naming { .. } | Mode::Describing { .. } => self.on_naming_key(key, ctx),
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
                self.apply_help(outcome, ctx.keys());
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
                    self.land(outcome, None, ctx.keys());
                    self.clamp();
                }
                // No snapshot to check the scope by: only the write in flight owns this reply, and
                // a scope change has already forgotten that (`on_scope_change`).
                Err(why) if self.busy == Some(outcome.request_name()) => {
                    if self.snapshot.is_none() {
                        self.unavailable = Some(why.clone());
                    }
                    self.land(outcome, Some(why), ctx.keys());
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
                    self.stale(*what, detach, ctx.keys());
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
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        let Some(Pending { mut editor, resume }) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(text) => {
                editor.area = TextArea::with_text(&text);
                editor.esc_armed = false;
                self.notice = Some(match skill_body_refusal(editor.area.text()) {
                    Some(sentence) => Notice::Error(sentence),
                    None => Notice::Info(edited(
                        ctx.keys()
                            .label(views::LIBRARY_EDITOR, Act::FormSave)
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
        // the version the next `Ctrl+S` writes.
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
        match (&self.attach, &self.snapshot, &self.mode, self.handed_off()) {
            (Some(pane), Some(snapshot), _, _) => pane.render(frame, content, snapshot, ctx),
            (_, _, Mode::Editing(editor), _) | (_, _, Mode::Browse, Some(editor)) => {
                self.render_editor(frame, content, editor, ctx);
            }
            (_, _, Mode::Report { outcomes, cursor }, _) => {
                self.render_report(frame, content, outcomes, *cursor, ctx);
            }
            _ => self.render_browse(frame, content, ctx),
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
    /// [`key_stack`](Self::key_stack): the attachments pane's while it is open, an open agent
    /// help's, the editor's after the cursor's `L:C`, else the mode's.
    pub(super) fn key_hint(&self, keys: &Keys) -> String {
        match (&self.attach, &self.snapshot, &self.mode, self.handed_off()) {
            (Some(pane), Some(_), _, _) => pane.hint(keys),
            (_, _, Mode::Editing(editor), _) => match &editor.help {
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
            (_, _, Mode::Browse, Some(_)) => keys.hint(self.key_stack(), HANDED_OFF_HINT),
            (_, _, Mode::Report { .. }, _) => keys.hint(self.key_stack(), REPORT_HINT),
            (_, _, mode, _) => {
                let spec = match mode {
                    Mode::Naming { .. } | Mode::Describing { .. } => NAMING_HINT,
                    Mode::Info(_) => INFO_HINT,
                    Mode::ImportPath { .. } => IMPORT_HINT,
                    Mode::Browse | Mode::Editing(_) | Mode::Report { .. } => BROWSE_HINT,
                };
                keys.hint(self.key_stack(), spec)
            }
        }
    }

    // --- keys ----------------------------------------------------------------------------------

    /// Browse (§6.3; MOD-67 M4 L-A §2): each candidate act of the chord through
    /// `views::LIBRARY_BROWSE`, the first this view uses taken. Chord equality includes the
    /// modifiers, so `ctrl-e` is not `e`. Anything else passes for the shell to resolve through the
    /// same stack (`q`, `Tab`, the digits, `?`). The tab took `skills.switch_view` first.
    ///
    /// MOD-115 M2 §7: only motion follows the focus. With the list focused `list.*` move the
    /// cursor; with the pane focused they scroll it (`pane.scroll_*`'s scroll); with the
    /// activity pane focused they are taken and nothing moves. Every other act acts whatever
    /// pane is focused.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        let (focus, rows) = (ctx.focus(), self.pane_rows.get());
        for act in ctx.keys().actions(views::LIBRARY_BROWSE, chord) {
            if Step::by_focus(act, focus, |step| self.scroll.follow(step, rows)) {
                return Handled::Consumed;
            }
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
    /// The field sees a key first (`Enter` and `Esc` are its own); what it passes goes to
    /// [`prompt_rest`].
    fn on_naming_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Naming { field } | Mode::Describing { field, .. } => field.on_key(key),
            _ => return Handled::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => prompt_rest(key, ctx),
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                match core::mem::take(&mut self.mode) {
                    Mode::Naming { field } => self.named(field, ctx.keys()),
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
            FieldOutcome::Pass => prompt_rest(key, ctx),
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
        let chord = ctx.pressed(&key);
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
    ///
    /// A taken name sends the user to the browse edit key `keys` binds (MOD-67 M6 D4).
    fn named(&mut self, field: TextField, keys: &Keys) {
        let name = field.text().unwrap_or_default().to_owned();
        let refusal = if validate_name(&name) {
            self.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.by_name(&name).is_some())
                .then(|| match keys.label(views::LIBRARY_BROWSE, Act::Edit) {
                    Some(edit) => format!("`{name}` exists \u{2014} select it and press {edit}"),
                    None => format!("`{name}` exists"),
                })
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

    /// The rename form, widget first (MOD-67 M4 PA-3): the focused field's `Enter` saves and its
    /// `Esc` closes; what it passes resolves through `views::LIBRARY_INFO`, where
    /// `form.next_field`/`prev_field` (`Tab`, `Shift+Tab`, `Down`, `Up`) move and `form.save`
    /// (`Ctrl+s`) saves. Any other key is `modal_rest`'s.
    fn on_info_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
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
            FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                match ctx.keys().actions(views::LIBRARY_INFO, chord).first() {
                    Some(Act::FormNextField | Act::FormPrevField) => {
                        form.focus = 1 - form.focus;
                        Handled::Consumed
                    }
                    Some(Act::FormSave) => {
                        self.save_info(ctx);
                        Handled::Consumed
                    }
                    _ => modal_rest(views::LIBRARY_INFO, chord),
                }
            }
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

    /// The editor, widget first (MOD-67 M4 PA-3): the area takes text and `Esc` (its `Cancel`);
    /// every chord it passes, `ctrl-s` included (D6), resolves through `views::LIBRARY_EDITOR`:
    /// `form.save` saves, `skills.ask_agent` opens the agent help, `form.external_editor` hands the
    /// draft to `$EDITOR`, and `global.next_tab`/`prev_tab` pass so the shell switches tabs with
    /// the draft kept (PA-2). Any other key is `modal_rest`'s. An open agent help (MOD-55) takes
    /// every key first: the draft is locked under it, and the editor's verbs are refused until it
    /// closes.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
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
                    editor.esc_armed = false;
                    self.notice = None;
                }
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
            // A `TextArea` never submits since MOD-67 M4 D6: `ctrl-s` passes, and only
            // `form.save` below saves.
            FieldOutcome::Submit | FieldOutcome::Pass => {
                let chord = ctx.pressed(&key);
                match ctx.keys().actions(views::LIBRARY_EDITOR, chord).first() {
                    Some(Act::FormSave) => self.save_editor(ctx),
                    Some(Act::SkillsAskAgent) => self.open_help(ctx),
                    Some(Act::FormExternalEditor) => self.hand_off(ctx),
                    Some(Act::NextTab | Act::PrevTab) => return Handled::Pass,
                    _ => return modal_rest(views::LIBRARY_EDITOR, chord),
                }
                Handled::Consumed
            }
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
                editor.esc_armed = false;
                self.notice = Some(match skill_body_refusal(editor.area.text()) {
                    Some(sentence) => Notice::Error(sentence),
                    None => Notice::Info(accepted(
                        keys.label(views::LIBRARY_EDITOR, Act::FormSave).as_deref(),
                    )),
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
        // The form's save is in flight: its reply closes the form or keeps it, so the cancel
        // waits for it, as the editor's does. The canonical `Esc` is `field.cancel` after the
        // shell's translation (MOD-115 M2 D7), so it is decoded, not resolved.
        if let Some(busy) = self.busy
            && pane.in_form()
            && FieldEvent::of(&key) == Some(FieldEvent::Cancel)
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
    fn land(&mut self, outcome: &SkillWrite, unread: Option<&str>, keys: &Keys) {
        if self.busy != Some(outcome.request_name()) {
            return;
        }
        let landed = match (outcome, self.sent.take()) {
            (SkillWrite::Created { skill, name }, Some(Sent::Create { body })) => {
                self.landed_version(*skill, name, 1, &body, keys)
            }
            (SkillWrite::Versioned { skill, version }, Some(Sent::Version { body })) => {
                let name = snapshot_name(self.snapshot.as_ref(), *skill);
                self.landed_version(*skill, &name, *version, &body, keys)
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
                    match keys.label(views::ATTACH_FORM, Act::FormSave) {
                        Some(save) => format!(
                            "attached to {target} \u{2014} later edits kept, {save} saves them"
                        ),
                        None => format!("attached to {target} \u{2014} later edits kept"),
                    }
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
                let name = outcome.request_name();
                self.notice = Some(Notice::Error(
                    match keys.label(views::LIBRARY_BROWSE, Act::Reload) {
                        Some(reload) => format!("`{name}` landed \u{2014} {reload} reloads"),
                        None => format!("`{name}` landed"),
                    },
                ));
                return;
            }
        };
        self.busy = None;
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(
                &landed,
                why,
                keys.label(views::LIBRARY_BROWSE, Act::Reload).as_deref(),
            )),
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
    fn landed_version(
        &mut self,
        skill: SkillId,
        name: &str,
        version: i32,
        body: &str,
        keys: &Keys,
    ) -> String {
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
            match keys.label(views::LIBRARY_EDITOR, Act::FormSave) {
                Some(save) => format!(
                    "{verb} \u{b7} ~{tokens} tokens \u{2014} later edits kept, {save} saves them \
                     as v{}",
                    version + 1
                ),
                None => format!("{verb} \u{b7} ~{tokens} tokens \u{2014} later edits kept"),
            }
        }
    }

    /// A write missed its token, or its row is gone (§6.4's second table): the draft keeps its
    /// text and takes the token as it is now, so the next `Ctrl+S` is a deliberate overwrite.
    fn stale(&mut self, what: StaleWhat, detach: bool, keys: &Keys) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        match what {
            StaleWhat::Version(skill) => match (snapshot.head(skill), &mut self.mode) {
                (Some(head), Mode::Editing(editor)) => {
                    editor.token = head.version;
                    self.notice = Some(Notice::Error(version_changed_elsewhere(
                        head.version,
                        keys.label(views::LIBRARY_EDITOR, Act::FormSave).as_deref(),
                    )));
                }
                (Some(_), _) => {}
                (None, _) => self.gone(),
            },
            StaleWhat::Skill(skill) => match (snapshot.entry(skill), &mut self.mode) {
                (Some(entry), Mode::Info(form)) => {
                    form.token = entry.skill.updated_at;
                    self.notice = Some(Notice::Error(skill_changed_elsewhere(
                        keys.label(views::LIBRARY_INFO, Act::FormSave).as_deref(),
                    )));
                }
                (Some(_), _) => {}
                (None, _) => self.gone(),
            },
            StaleWhat::Binding(_) => {
                if let Some(pane) = &mut self.attach {
                    pane.on_stale(snapshot);
                }
                let sentence = if detach {
                    DETACH_CHANGED_ELSEWHERE.to_owned()
                } else {
                    binding_changed_elsewhere(
                        keys.label(views::ATTACH_FORM, Act::FormSave).as_deref(),
                    )
                };
                self.notice = Some(Notice::Error(sentence));
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

    /// The row under the cursor, for the tab's history location (MOD-115 M2 §8): its index and,
    /// on a skill, the skill's id (`{index}:{id}`, RV3), so Back finds the skill again after a
    /// create, an import or a delete moved its row.
    pub(super) fn location_cursor(&self) -> String {
        match self.entry_at(self.cursor) {
            Some(entry) => format!("{}:{}", self.cursor, entry.skill.id),
            None => self.cursor.to_string(),
        }
    }

    /// MOD-115 M2 §8: the cursor back on a [`location_cursor`](Self::location_cursor): onto its
    /// skill wherever the library now holds it ([`select`](Self::select)), else on its row
    /// (clamped to the library), the head's body shown from its top, as a move shows it. Only
    /// while browsing with nothing open over the list: a prompt, the attachments pane or a
    /// handed-off draft keeps its row. Whether it moved; a cursor with no row moves nothing.
    pub(super) fn restore_cursor(&mut self, cursor: &str) -> bool {
        let (index, skill) = cursor
            .split_once(':')
            .map_or((cursor, None), |(index, skill)| (index, skill.parse().ok()));
        let Ok(index) = index.parse::<usize>() else {
            return false;
        };
        if !matches!(self.mode, Mode::Browse)
            || self.attach.is_some()
            || self.handed_off().is_some()
        {
            return false;
        }
        let last = self
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.skills.len().saturating_sub(1));
        self.cursor = index.min(last);
        match skill {
            Some(skill) => self.select(skill),
            None => self.reset_pane(),
        }
        true
    }

    /// The library's entry on row `index`, if the library is read and holds that row.
    fn entry_at(&self, index: usize) -> Option<&SkillEntry> {
        self.snapshot.as_ref()?.skills.get(index)
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

    /// Browse, the prompts and the rename form: the list on the left, the pane on the right, each
    /// a kit pane (MOD-115 M4 E5). The list leads the view, so it carries the view chips. An open
    /// prompt holds the keyboard, so the pane it is drawn in is the focused one (C4); otherwise
    /// the shell's focus decides. Each pane draws its hints only while focused (Q2).
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // MOD-115 M2 B5: the panes the shell's layout gives the tab, claimed for the focus,
        // the hidden-pane note and the mouse; today's 32-cell list by default. A hidden pane is
        // an empty rect, which draws nothing.
        let areas = ctx.split(area, LAYOUT);
        let prompt = !matches!(self.mode, Mode::Browse);
        // T7 repair: a prompt or the rename form takes every key, so with another pane zoomed
        // the pane is drawn over the zoomed one rather than into its own empty rect.
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
            self.render_list(frame, list_area, list_focus, ctx);
        }
        if !pane_area.is_empty() {
            self.render_pane(frame, pane_area, pane_focus, prompt, ctx);
        }
    }

    /// The library as a kit list (MOD-115 M4 E5): one row per skill after the gutter, the cursor
    /// the gutter and band (MOD-85), kept in view; the view chips on the top border.
    fn render_list(&self, frame: &mut Frame<'_>, area: Rect, focus: Focus, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let inner = KitPane::inner(area);
        let rows: Vec<Row<'static>> = self
            .snapshot
            .iter()
            .flat_map(|snapshot| snapshot.skills.iter())
            .map(|entry| {
                let head = entry
                    .versions
                    .iter()
                    .map(|row| row.version)
                    .max()
                    .unwrap_or(0);
                Row {
                    line: Line::styled(browse_row(&entry.skill.name, head), theme.base),
                    kind: RowKind::Item,
                }
            })
            .collect();
        let selected = (!rows.is_empty()).then_some(self.cursor);
        let offset = list::scroll_into_view(self.cursor, 0, usize::from(inner.height), rows.len());
        list::render(frame, inner, &rows, selected, offset, ctx.accent(), theme);
        let hints = hint_line(ctx.keys().hint(views::LIBRARY_BROWSE, LIST_HINT), ctx);
        framed(view_chips(KitPane::new(""), 0), focus, hints, ctx)
            .scrollbar(rows.len(), offset)
            .render(frame, area, theme);
    }

    /// The right-hand pane: a prompt's form, an empty state, a refused read, or the shown body or
    /// diff, wrapped and scrolled with a scrollbar. Its title names no key (MOD-82 carried): the
    /// scroll keys lead its hints while its lines overflow it.
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
        let mut pane = KitPane::new(&title).focus(focus).accent(ctx.accent());
        let mut overflow = false;
        self.pane_rows.set(0);
        match detail {
            Detail::Form(form) => form.render(frame, inner, 0, ctx.accent(), theme),
            Detail::Empty(what, next) => empty::render(frame, inner, what, NextStep(&next), theme),
            Detail::Unavailable(sentence) => unavailable(frame, inner, &sentence, theme),
            Detail::Lines(lines) => {
                // The body and the diff wrap; the row count is a character wrap's, a lower bound
                // on the word wrap's (`Scroll`'s rule), so the clamp never scrolls the pane blank.
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
                keys.hint(views::LIBRARY_BROWSE, SCROLL_HINT)
            } else {
                String::new()
            };
            joined(&[keys.hint(views::LIBRARY_BROWSE, DETAIL_HINT), scroll])
        };
        if let Some(hints) = hint_line(hint, ctx) {
            pane = pane.hints(hints);
        }
        pane.render(frame, area, theme);
    }

    /// The right-hand pane's title and what it shows: a prompt or the rename form, a refusal, the
    /// empty states, the shown body, or the diff.
    fn detail(&self, ctx: &Ctx<'_>) -> (String, Detail<'_>) {
        let theme = ctx.theme;
        match &self.mode {
            Mode::Naming { field } => {
                let form = Form::new()
                    .field(Field::new("name", Value::Text(field)))
                    .focus(0)
                    .editing(true);
                return ("new skill".to_owned(), Detail::Form(form));
            }
            Mode::Describing { name, field } => {
                let form = Form::new()
                    .field(Field::new("name", Value::ReadOnly(name.as_str().into())))
                    .field(Field::new("description", Value::Text(field)))
                    .focus(1)
                    .editing(true);
                return ("new skill".to_owned(), Detail::Form(form));
            }
            Mode::Info(form) => {
                let name = snapshot_name(self.snapshot.as_ref(), form.skill);
                let fields = Form::new()
                    .field(Field::new("name", Value::Text(&form.name)))
                    .field(Field::new("description", Value::Text(&form.description)))
                    .focus(form.focus)
                    .editing(true);
                return (format!("{name} \u{b7} rename"), Detail::Form(fields));
            }
            Mode::ImportPath { field } => {
                let form = Form::new()
                    .field(Field::new("path", Value::Text(field)).help(IMPORT_HELP))
                    .focus(0)
                    .editing(true);
                return ("import skills".to_owned(), Detail::Form(form));
            }
            Mode::Browse | Mode::Editing(_) | Mode::Report { .. } => {}
        }
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                Detail::Unavailable(format!("{UNAVAILABLE}: {why}")),
            );
        }
        let keys = ctx.keys();
        let next = |spec: HintSpec, unbound: &str| {
            let hint = keys.hint(views::LIBRARY_BROWSE, spec);
            if hint.is_empty() {
                unbound.to_owned()
            } else {
                hint
            }
        };
        let Some(snapshot) = &self.snapshot else {
            return (
                String::new(),
                Detail::Empty(NOT_READ, NOT_READ_NEXT.to_owned()),
            );
        };
        let Some(entry) = snapshot.skills.get(self.cursor) else {
            return (
                String::new(),
                Detail::Empty(SELECT_A_SKILL, next(EMPTY_LIBRARY_HINT, EMPTY_LIBRARY_NEXT)),
            );
        };
        let (skill, name) = (entry.skill.id, entry.skill.name.as_str());
        let (Some(head), Some(shown)) = (snapshot.head(skill), self.shown_row(snapshot, skill))
        else {
            return (
                name.to_owned(),
                Detail::Empty(SELECT_A_SKILL, next(NO_VERSION_HINT, NO_VERSION_NEXT)),
            );
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
                    "{name} v{} (head v{}) \u{b7} ~{} tokens",
                    shown.version,
                    head.version,
                    estimate(name, shown.version, &shown.body)
                ),
                Detail::Lines(lines),
            );
        }
        let base = self.base.unwrap_or(shown.version - 1);
        let Some(old) = snapshot.version(skill, base) else {
            return (
                format!("{name} v{}", shown.version),
                Detail::Lines(vec![Line::styled(
                    format!("v{} has no base to diff against", shown.version),
                    theme.dim,
                )]),
            );
        };
        let unified = diff::unified(
            &old.body,
            &shown.body,
            &format!("{name} v{base}"),
            &format!("{name} v{}", shown.version),
        );
        (
            format!("diff v{base} \u{2192} v{}", shown.version),
            Detail::Lines(diff::lines(&unified, theme)),
        )
    }

    /// The import report over the whole content, a kit pane with the view chips: one block of
    /// rows per outcome, wrapped (no message is cut), the cursor's block in the gutter and band
    /// (MOD-115 M4 E5, MOD-85: the `>` marker is gone), the rows that need acting on in the error
    /// style.
    fn render_report(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        outcomes: &[ImportOutcome],
        cursor: usize,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let inner = KitPane::inner(area);
        let content = inner.width.saturating_sub(1);
        // `{sign} ` leads a block's first row; its other rows are indented as far.
        let text_width = usize::from(content.saturating_sub(2)).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (at, outcome) in outcomes.iter().enumerate() {
            let (sign, text, problem) = outcome_row(outcome);
            let style = if problem { theme.error } else { theme.dim };
            let rows = cells::wrap(&text, text_width);
            for (index, row) in rows.into_iter().enumerate() {
                let lead = if index == 0 {
                    format!("{sign} ")
                } else {
                    "  ".to_owned()
                };
                let line = Line::from(vec![
                    Span::styled(lead, theme.base),
                    Span::styled(row, style),
                ]);
                let (gutter, line) = if at == cursor {
                    (
                        Span::styled(GUTTER, ctx.accent()),
                        selection_band(line, content, theme),
                    )
                } else {
                    (Span::raw(" "), ellipsize(line, content))
                };
                let mut spans = vec![gutter];
                spans.extend(line.spans);
                lines.push(Line::from(spans));
            }
        }
        let total = lines.len();
        self.pane_rows.set(total);
        frame.render_widget(
            Paragraph::new(lines).scroll((self.scroll.offset(), 0)),
            inner,
        );
        let mut pane = view_chips(KitPane::new("import report"), 0)
            .focus(holder(ctx, false))
            .accent(ctx.accent())
            .scrollbar(total, self.scroll.skip());
        if let Some(hints) = hint_line(self.key_hint(ctx.keys()), ctx) {
            pane = pane.hints(hints);
        }
        pane.render(frame, area, theme);
    }

    /// The editor over the whole content, its title carrying the draft's estimate (D82). An open
    /// agent help draws in the draft's place (MOD-55 B-2): a panel under a locked draft, or the
    /// proposal over all of it.
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let draft = match &editor.help {
            Some(help) => help.render(frame, area, ctx, 0),
            None => Some(area),
        };
        if let Some(draft) = draft {
            self.render_draft(frame, draft, editor, ctx);
        }
    }

    /// The draft's pane: the name, the versions and the estimate as its title, the view chips while
    /// the whole title fits beside them (`view_chips_if_room`), and the text with its cursor (dim
    /// under an open help, which has the keys and the focus). The text's rect is claimed for an
    /// in-pane editor (MOD-57 P2), inside the pane, so the name and versions stay visible beside
    /// it.
    fn render_draft(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, ctx: &Ctx<'_>) {
        let name = editor.target.name();
        let tokens = estimate(name, editor.saves(), editor.area.text());
        let title = match (&editor.target, editor.from) {
            (Target::New { .. }, _) => {
                format!("{name} \u{b7} new, saves v1 \u{b7} ~{tokens} tokens")
            }
            // MOD-9 D127: a skill with no version row has nothing to edit from.
            (Target::Version { .. }, None) => format!(
                "{name} \u{b7} no version yet, saves v{} \u{b7} ~{tokens} tokens",
                editor.saves()
            ),
            (Target::Version { .. }, Some(from)) => format!(
                "{name} \u{b7} editing from v{from}, saves v{} \u{b7} ~{tokens} tokens",
                editor.saves()
            ),
        };
        let inner = KitPane::inner(area);
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
        let mut pane = view_chips_if_room(KitPane::new(&title), &title, 0, area.width)
            .focus(focus)
            .accent(ctx.accent());
        if let Some(hints) = hint_line(self.key_hint(ctx.keys()), ctx) {
            pane = pane.hints(hints);
        }
        pane.render(frame, area, ctx.theme);
    }
}

/// What the right-hand pane shows in Browse (MOD-115 M4 E5).
enum Detail<'a> {
    /// A prompt or the rename form, through `kit::form`.
    Form(Form<'a>),
    /// Nothing to show: what is missing and the next step (`kit::empty`).
    Empty(&'static str, String),
    /// A refused read: the store's sentence, and where to fix it.
    Unavailable(String),
    /// The shown body or the diff, wrapped and scrolled.
    Lines(Vec<Line<'static>>),
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
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::clock::{TestClock, epoch};
    use htui_core::fixtures::ids;
    use htui_core::model::{Attachment, ProjectRef, Scope, SkillBindingKey, StepId};
    use htui_core::store::{BLANK_SKILL_BODY, CasOutcome, MemStore, WriteStore as _};
    use htui_store::Backend;

    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keys::Keys;
    use crate::skills;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crate::ui::tabs::SkillsTab;
    use crate::ui::tabs::registry::Tab as _;
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
        let (top_bar, keys, theme, emit) = (
            TopBarState::default(),
            Keys::compiled(),
            Theme::default(),
            Emit::default(),
        );
        let scope = vulkan();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            keys,
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
        /// The compiled default keys.
        keys: Keys,
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
                keys: Keys::compiled().clone(),
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
        // The content above the notice row, inside the draft's border (the keys are the shell's
        // hint row since MOD-67 M6 T2).
        assert_eq!(claim, Rect::new(1, 1, 98, 25));
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
        let hint = view.key_hint(ctx.keys());
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

    /// MOD-67 M6 D4: the Library's key-naming notices with their key unbound.
    #[test]
    fn the_library_notices_without_their_keys() {
        assert_eq!(
            skill_changed_elsewhere(None),
            "changed elsewhere since you opened it \u{2014} your text is kept"
        );
        assert_eq!(
            binding_changed_elsewhere(None),
            "this attachment changed elsewhere \u{2014} your form is kept"
        );
        assert!(version_changed_elsewhere(3, None).ends_with("saving writes it as v4"));
        assert!(version_changed_elsewhere(3, Some("F2")).ends_with("F2 saves it as v4"));
        assert_eq!(
            landed_unread("saved v3", "gone", None),
            "saved v3 \u{2014} the re-read failed: gone"
        );
    }

    /// MOD-67 M6 D4: a landed-unread write names the reload key in force through
    /// `LIBRARY_BROWSE`.
    #[tokio::test]
    async fn a_rebound_reload_key_is_named_by_a_landed_unread_write() {
        let backend = Backend::memory(MemStore::demo());
        let bench = Bench {
            keys: crate::keys::load_str("version = 1\n[skills.library]\nreload = \"f5\"\n")
                .expect("loads"),
            ..Bench::new()
        };
        let mut ctx = bench.ctx();
        let mut view = LibraryView::default();
        let read = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&read, &mut ctx);

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
        assert!(
            matches!(
                &view.notice,
                Some(Notice::Error(text))
                    if text.contains("the re-read failed, F5 reloads: store unreachable: gone")
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
        assert_eq!(view.notice, Some(Notice::Info(accepted(Some("Ctrl+s")))));
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
        assert_eq!(view.notice, Some(Notice::Info(accepted(Some("Ctrl+s")))));
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

    // ---- MOD-115 M2 T7: the panes, motion by focus, the location ---------------------------

    /// A view browsing the demo library, the cursor on its first skill.
    async fn browsing(bench: &Bench) -> LibraryView {
        let backend = Backend::memory(MemStore::demo());
        let mut view = LibraryView::default();
        let reply = serve(&backend, &StoreRequest::Skills(bench.scope.clone())).await;
        view.on_reply(&reply, &mut bench.ctx());
        assert!(
            view.snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.skills.len() >= 2),
            "the demo library has two skills"
        );
        view
    }

    /// MOD-115 M2 D2: the Skills tab's default layout is the 32-cell list both views drew, and
    /// the browse frame claims exactly the old `[Length(32), Min(1)]` split.
    #[tokio::test]
    async fn the_browse_frame_claims_the_32_cell_list() {
        use crate::ui::shell::{ListSize, Panes};
        assert_eq!(LAYOUT.list, ListSize::Cells(LIST_WIDTH));
        assert_eq!(LIST_WIDTH, 32);
        assert_eq!(SkillsTab::new().panes(), Panes::ListDetail(LAYOUT));

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

    /// MOD-115 M2 §7: with the list focused, motion moves the cursor; with the detail focused
    /// it scrolls the pane and the cursor stays; with the activity pane focused nothing moves.
    #[tokio::test]
    async fn motion_follows_the_focused_pane() {
        use crate::ui::shell::PaneId;
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.pane_rows.set(10);
        let press = |view: &mut LibraryView, focus: PaneId, code: KeyCode| {
            view.on_key(key(code), &mut bench.ctx().with_focus(focus))
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
        assert_eq!(view.cursor, 1, "the list's");
        assert!(bench.emit.take().is_empty());
    }

    /// MOD-115 M2 §8: the cursor goes back to a row while browsing (the head's body from its
    /// top, as a move shows it), clamped to the library; never under an open prompt, and not
    /// for a cursor this view did not write.
    #[tokio::test]
    async fn the_cursor_is_restored_only_while_browsing() {
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.scroll = Scroll::at(3);
        assert!(view.restore_cursor("1"));
        assert_eq!((view.cursor, view.scroll.offset()), (1, 0));
        assert!(view.restore_cursor("99"));
        let last = view.snapshot.as_ref().map_or(0, |s| s.skills.len() - 1);
        assert_eq!(view.cursor, last, "clamped");
        assert!(!view.restore_cursor("not a row"));
        assert_eq!(view.cursor, last, "an unreadable cursor moves nothing");

        view.mode = Mode::Naming {
            field: TextField::new(),
        };
        assert!(!view.restore_cursor("0"));
        assert_eq!(view.cursor, last, "the prompt keeps the row");
    }

    /// MOD-115 M2 §8, RV3 (review M2): the location names the skill under the cursor as well as
    /// its row (`{index}:{id}`). A skill created before it moves its row, and the cursor goes
    /// back onto the same skill; a skill that is gone falls back to the row, clamped.
    #[tokio::test]
    async fn the_cursor_finds_its_skill_after_the_library_changed() {
        let bench = Bench::new();
        let mut view = browsing(&bench).await;
        view.on_key(key(KeyCode::Char('j')), &mut bench.ctx());
        let skill = view.snapshot.as_ref().expect("read").skills[1].skill.id;
        let at = view.location_cursor();
        assert_eq!(at, format!("1:{skill}"));

        let snapshot = view.snapshot.as_mut().expect("read");
        let mut created = snapshot.skills[0].clone();
        created.skill.id = SkillId::new();
        created.skill.name = "aaa-docs".to_owned();
        snapshot.skills.insert(0, created);
        view.cursor = 0;
        assert!(view.restore_cursor(&at));
        assert_eq!(view.cursor, 2, "on the same skill, a row down");

        let snapshot = view.snapshot.as_mut().expect("read");
        snapshot.skills.retain(|entry| entry.skill.id != skill);
        view.cursor = 0;
        assert!(view.restore_cursor(&at));
        assert_eq!(view.cursor, 1, "the skill is gone: its row");
    }

    // ---- MOD-115 M4 L-skills: the view on the kit ----------------------------------------------

    /// `view` drawn at `width` x `height` through `ctx`, as the buffer and its rows.
    fn frame_of(
        view: &LibraryView,
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

    /// MOD-115 M4 MD-A: the prompts' hints name `field.submit` through `LIBRARY_PROMPT`, so a
    /// rebound one shows its chord; `Esc` stays the field's (M2 MD7).
    #[test]
    fn the_prompt_hints_name_the_submit_key_in_force() {
        let rebound = crate::keys::load_str("version = 2\n[field]\nsubmit = \"ctrl-t\"\n")
            .expect("the key file loads");
        let naming = LibraryView {
            mode: Mode::Naming {
                field: TextField::new(),
            },
            ..LibraryView::default()
        };
        let import = LibraryView {
            mode: Mode::ImportPath {
                field: TextField::new(),
            },
            ..LibraryView::default()
        };
        assert_eq!(
            naming.key_hint(Keys::compiled()),
            "Enter next \u{b7} Esc cancel"
        );
        assert_eq!(naming.key_hint(&rebound), "Ctrl+t next \u{b7} Esc cancel");
        assert_eq!(
            import.key_hint(Keys::compiled()),
            "Enter import \u{b7} Esc cancel"
        );
        assert_eq!(import.key_hint(&rebound), "Ctrl+t import \u{b7} Esc cancel");
    }

    /// MOD-115 M4 MD-A: what a prompt's field passes resolves from the chord pressed
    /// (`Ctx::pressed`), not the event delivered: with `field.line_down` on `ctrl-n` the shell
    /// delivers the canonical `Down`, which the one-line field passes; the pressed `ctrl-n` is a
    /// chord the modal stack passes on, a pressed `Down` is swallowed.
    #[test]
    fn a_prompt_resolves_what_its_field_passes_from_the_chord_pressed() {
        let bench = Bench::new();
        let chord = |spec: &str| crate::keys::KeyChord::parse_strict(spec).expect("a valid spec");
        let down = key(KeyCode::Down);
        for mode in [
            Mode::Naming {
                field: TextField::new(),
            },
            Mode::ImportPath {
                field: TextField::new(),
            },
        ] {
            let mut view = LibraryView {
                mode,
                ..LibraryView::default()
            };
            assert_eq!(
                view.on_key(down, &mut bench.ctx().with_pressed(down, chord("ctrl-n"))),
                Handled::Pass,
                "a pressed `ctrl-n` delivered as `Down` passes"
            );
            assert_eq!(
                view.on_key(down, &mut bench.ctx().with_pressed(down, chord("down"))),
                Handled::Consumed,
                "a pressed `Down` is swallowed"
            );
            assert!(view.captures_input(), "the prompt stays");
        }
    }

    /// MOD-115 M4 E5, MOD-85: the list's cursor is the kit's gutter and band, legible without
    /// colour (the band reverses), not `selected` on the text alone; the list and the body are
    /// rounded kit panes.
    #[tokio::test]
    async fn the_list_cursor_is_the_gutter_and_band() {
        use ratatui::style::Modifier;
        let bench = Bench::new();
        let view = browsing(&bench).await;
        for theme in [Theme::default(), Theme::monochrome()] {
            let bench = Bench {
                theme,
                ..Bench::new()
            };
            let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 28);
            assert!(rows[0].starts_with('\u{256d}'), "rounded: {rows:#?}");
            assert_eq!(buffer[(1, 1)].symbol(), GUTTER, "{rows:#?}");
            assert_eq!(buffer[(1, 2)].symbol(), " ", "only the cursor's row");
            let band = |y: u16| {
                let cell = &buffer[(5, y)];
                (cell.bg, cell.modifier.contains(Modifier::REVERSED))
            };
            let selected = (
                theme.selected.bg.unwrap_or(ratatui::style::Color::Reset),
                theme.selected.add_modifier.contains(Modifier::REVERSED),
            );
            assert_eq!(band(1), selected, "the cursor's row is banded");
            assert_ne!(band(2), selected, "the next row is not");
            // The band runs past the text, to the pane's edge.
            assert_eq!(buffer[(30, 1)].bg, selected.0);
        }
    }

    /// MOD-115 M4 C4: an open prompt holds the keyboard, so the pane it is drawn in is the
    /// focused one, in the tab's accent with the prompt's keys on its bottom border, and the
    /// list is not, whatever the shell's focus says.
    #[test]
    fn an_open_prompt_holds_the_focus_and_its_hints() {
        let bench = Bench::new();
        let view = LibraryView {
            mode: Mode::Naming {
                field: TextField::new(),
            },
            snapshot: Some(SkillsSnapshot {
                skills: Vec::new(),
                global: Vec::new(),
                projects: Vec::new(),
            }),
            ..LibraryView::default()
        };
        let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 20);
        let accent = bench.theme.tab_accent(SkillsTab::ID.0);
        assert_eq!(
            Some(buffer[(32, 0)].fg),
            accent.fg,
            "the prompt's pane: {rows:#?}"
        );
        assert_eq!(
            Some(buffer[(0, 0)].fg),
            bench.theme.border_faint.fg,
            "not the list"
        );
        let bottom = &rows[18];
        let detail: String = bottom.chars().skip(32).collect();
        assert!(detail.contains("Enter next"), "the prompt's pane: {bottom}");
        let list: String = bottom.chars().take(32).collect();
        assert!(!list.contains("Enter"), "no hint on the list: {bottom}");
        assert!(rows[0].contains("new skill"), "{rows:#?}");
        assert!(
            rows.iter().any(|row| row.contains("name")),
            "the prompt is a form row: {rows:#?}"
        );
    }

    /// MOD-115 M4 E5 (MOD-82 carried), T3: the body's title names no key; while its lines
    /// overflow it, the scroll keys close the hints of the body pane, shown when it is focused,
    /// after the view's own keys, so a pane too narrow for them all drops those first, whole (the
    /// scrollbar still says it scrolls). The list's hints never carry them.
    #[tokio::test]
    async fn the_scroll_keys_close_the_focused_body_and_no_title_names_a_key() {
        use crate::ui::shell::PaneId;
        let bench = Bench::new();
        let view = browsing(&bench).await;
        let (_, rows) = frame_of(&view, &bench.ctx(), 100, 6);
        assert!(rows[0].contains("rust-style v2 (head v2)"), "{rows:#?}");
        assert!(
            rows.iter().all(|row| !row.contains("J/K")),
            "list focused: {rows:#?}"
        );
        let focused = bench.ctx().with_focus(PaneId::Detail);
        let (_, rows) = frame_of(&view, &focused, 100, 6);
        assert!(!rows[0].contains("J/K"), "no key in a title: {rows:#?}");
        let narrow: String = rows[4].chars().skip(32).collect();
        assert!(
            narrow.starts_with("\u{2570}\u{2500},/. version \u{b7} b base \u{b7} d diff")
                && narrow.contains(" \u{2026}"),
            "the view's keys lead, cut by whole keys: {rows:#?}"
        );
        assert!(
            !narrow.contains("J/K"),
            "the scroll keys drop first: {narrow}"
        );
        let (_, rows) = frame_of(&view, &focused, 140, 4);
        let wide = &rows[2];
        let (own, scroll) = (
            wide.find("b base"),
            wide.find("J/K scroll \u{b7} PgUp/PgDn page"),
        );
        assert!(
            own.zip(scroll).is_some_and(|(own, scroll)| own < scroll),
            "wide enough, the scroll keys close the hints: {wide}"
        );
    }

    /// Whether `cell` is drawn in `style`: its colours where it names them, and its modifiers.
    fn in_style(cell: &ratatui::buffer::Cell, style: ratatui::style::Style) -> bool {
        style.fg.is_none_or(|fg| cell.fg == fg)
            && style.bg.is_none_or(|bg| cell.bg == bg)
            && cell.modifier.contains(style.add_modifier)
    }

    /// The buffer column `text` starts at in `row`, counted in cells.
    fn column_of(row: &str, text: &str) -> u16 {
        let at = row
            .find(text)
            .unwrap_or_else(|| panic!("{text:?} is not on {row:?}"));
        u16::try_from(row[..at].chars().count()).expect("a short row")
    }

    /// MOD-115 M4 E5, MOD-85: the body's and the draft's titles are the kit pane's title role,
    /// the tab's accent in bold while the pane is focused and dim while it is not, never a
    /// style the view paints on them. In colour and in monochrome.
    #[tokio::test]
    async fn the_body_and_draft_titles_are_the_pane_title_role() {
        use crate::ui::shell::PaneId;
        use ratatui::style::Modifier;
        for theme in [Theme::default(), Theme::monochrome()] {
            let bench = Bench {
                theme,
                ..Bench::new()
            };
            let focused = bench
                .theme
                .tab_accent(SkillsTab::ID.0)
                .add_modifier(Modifier::BOLD);
            let view = browsing(&bench).await;
            let ctx = bench.ctx().with_focus(PaneId::Detail);
            let (buffer, rows) = frame_of(&view, &ctx, 100, 12);
            let x = column_of(&rows[0], "rust-style v2");
            assert!(in_style(&buffer[(x, 0)], focused), "focused: {rows:#?}");
            let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 12);
            let title = &buffer[(x, 0)];
            assert!(in_style(title, bench.theme.dim), "unfocused: {rows:#?}");
            assert!(!title.modifier.contains(Modifier::BOLD), "{rows:#?}");

            let target = Target::New {
                name: "docs-style".to_owned(),
                description: String::new(),
            };
            let view = LibraryView {
                mode: Mode::Editing(Editor::new(target, 0, None, "First line.\n")),
                ..LibraryView::default()
            };
            let (buffer, rows) = frame_of(&view, &bench.ctx(), 100, 12);
            let x = column_of(&rows[0], "docs-style");
            assert!(in_style(&buffer[(x, 0)], focused), "the draft: {rows:#?}");
        }
    }

    /// MOD-115 M4 L-skills repair 1 (Q2): the focused list shows its own keys whole in its 28
    /// cells: its motion first, as every list's hint does, then a new skill and the view's own
    /// act, the keys the Browse row showed before the port; the row's keys are the body's.
    #[tokio::test]
    async fn the_list_hint_leads_with_its_motion() {
        let bench = Bench::new();
        let view = browsing(&bench).await;
        let (_, rows) = frame_of(&view, &bench.ctx(), 100, 12);
        let bottom = rows
            .iter()
            .rposition(|row| row.starts_with('\u{2570}'))
            .expect("the list's bottom border");
        let list: String = rows[bottom].chars().take(32).collect();
        assert_eq!(
            list.trim_matches(['\u{2570}', '\u{256f}', '\u{2500}']),
            "j/k \u{b7} n new \u{b7} I import",
            "{rows:#?}"
        );
    }

    /// MOD-115 M4 L-skills repair 2: every empty state of the pane says what is missing and the
    /// next step (`kit::empty`), and a refused read names where it is fixed: before the first
    /// reply, on a refused read, on an empty library and on a skill with no version yet.
    #[tokio::test]
    async fn every_empty_pane_names_its_next_step() {
        let bench = Bench::new();
        let shows = |view: &LibraryView, bench: &Bench, texts: &[&str]| {
            let (_, rows) = frame_of(view, &bench.ctx(), 100, 12);
            for text in texts {
                assert!(
                    rows.iter().any(|row| row.contains(text)),
                    "{text:?}: {rows:#?}"
                );
            }
        };
        shows(
            &LibraryView::default(),
            &bench,
            &["skills not read yet", "they appear once the store answers"],
        );
        let refused = LibraryView {
            unavailable: Some("store unreachable: gone".to_owned()),
            ..LibraryView::default()
        };
        shows(
            &refused,
            &bench,
            &[
                "skills unavailable: store unreachable: gone",
                "Settings \u{203a} Connection stores a DSN",
            ],
        );
        let empty = LibraryView {
            snapshot: Some(SkillsSnapshot {
                skills: Vec::new(),
                global: Vec::new(),
                projects: Vec::new(),
            }),
            ..LibraryView::default()
        };
        shows(
            &empty,
            &bench,
            &["select a skill", "n creates one \u{b7} I imports some"],
        );
        let mut unversioned = browsing(&bench).await;
        if let Some(snapshot) = unversioned.snapshot.as_mut() {
            snapshot.skills[0].versions.clear();
        }
        shows(
            &unversioned,
            &bench,
            &["select a skill", "e writes its first version"],
        );

        // With every key of a next step unbound, the step is said in words, never left blank.
        let unbound = Bench {
            keys: crate::keys::load_str(
                "version = 2\n[common]\nnew = []\nedit = []\n[skills.library]\nimport = []\n",
            )
            .expect("the key file loads"),
            ..Bench::new()
        };
        shows(&empty, &unbound, &["a new or imported skill shows here"]);
        shows(&unversioned, &unbound, &["its first version shows here"]);
    }

    /// MOD-115 M4 L-skills repair 2: the draft's title (the versions and the token estimate,
    /// D82) stays whole in a pane just wide enough for it: the view chips, which do nothing while
    /// the draft has the keys, are drawn only when the whole title fits beside them.
    #[test]
    fn the_draft_title_stays_whole_where_the_chips_would_cut_it() {
        let bench = Bench::new();
        let body = "First line.\n";
        let target = Target::New {
            name: "docs-style".to_owned(),
            description: String::new(),
        };
        let view = LibraryView {
            mode: Mode::Editing(Editor::new(target, 0, None, body)),
            ..LibraryView::default()
        };
        let title = format!(
            "docs-style \u{b7} new, saves v1 \u{b7} ~{} tokens",
            estimate("docs-style", 1, body)
        );
        let width = u16::try_from(cell_width(&title)).expect("a short title") + 6;
        let (_, rows) = frame_of(&view, &bench.ctx(), width, 8);
        assert!(rows[0].contains(&format!(" {title} ")), "{rows:#?}");
        let (_, rows) = frame_of(&view, &bench.ctx(), 100, 8);
        assert!(rows[0].contains(&format!(" {title} ")), "{rows:#?}");
        assert!(rows[0].contains(" Skills "), "the view chips: {rows:#?}");
    }
}
