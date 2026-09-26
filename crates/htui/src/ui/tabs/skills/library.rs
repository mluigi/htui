//! The Skills view of the Skills tab (MOD-9 milestone 3, plan D82; blueprint D98-D100): the skill
//! library, any version's body or a line diff between two versions, a token estimate, an editor
//! that appends a version, create and rename, and the attachments pane (`attach`).
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle and
//! no `UserId`, renders from the last [`SkillsSnapshot`] and never patches a row into it. A write
//! **lands by content** (D98, milestone 1's D27): a `Skills` reply closes the draft only when it
//! holds what was sent, so a read served ahead of the write leaves the draft, the token and `busy`
//! alone. A `SkillsStale` keeps the draft and moves the token to the row as it is now.
//!
//! The token a version save carries is the head when the editor opened, not the version shown:
//! editing v1 while v2 is head saves v3 (milestone 1's OQ-5, as the Templates view).
//!
//! The estimate is the skill's own block as the assembler renders it, without the section frame it
//! shares with the other skills (D99, F-I): what adding this skill to a prompt costs.

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::skill::validate_name;
use htui_core::model::skill_language;
use htui_core::model::{
    Activation, BindingChange, BoundSkill, SkillBindingKey, SkillId, SkillLevel, SkillPatch,
    SkillVersion,
};
use htui_core::prompt::{TokenEstimator, render};
use htui_core::store::{invalid_skill_name, skill_body_refusal};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::attach::{AttachOutcome, AttachPane};
use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome};
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillsSnapshot, StaleWhat};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::templates::TemplateBody;
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme, diff};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The list's width, borders included: `  {name:<22} v{head:<3}` is 29 chars, a longer name cut
/// to [`NAME_WIDTH`].
const LIST_WIDTH: u16 = 32;

/// A library row's name field, in chars: a longer name is cut to `NAME_WIDTH - 1` and `…`.
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

/// An `$EDITOR` return with text in it.
const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// An `$EDITOR` return that changed nothing, and a rename that changes nothing.
const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within `QUICK_EXIT` (milestone 1 D24).
const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

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
const BINDING_GONE_ELSEWHERE: &str = "this attachment changed elsewhere \u{2014} nothing was detached; the row shows it as it is now";

/// The hint row in Browse (96 chars: `j/k` carries no word so the row fits 100 columns).
const BROWSE_HINT: &str = "j/k  ,/. version  b base  d diff  e edit  E $EDITOR  n new  i info  \
                           a attach  r reload  h/l view";

/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row while naming or describing a new skill.
const NAMING_HINT: &str = "Enter next  Esc cancel";

/// The hint row on the rename form.
const INFO_HINT: &str = "Tab field  Ctrl+S save  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}`.
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+E $EDITOR  Esc cancel";

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

/// The write in flight and what it carries (D98): what tells its own landing from a read served
/// ahead of it. Custom `Debug`: body lengths only.
pub(super) enum Sent {
    /// `CreateSkill`.
    Create {
        /// The new skill's name.
        name: String,
        /// Version 1's body.
        body: String,
    },
    /// `EditSkill`.
    Rename {
        /// Which skill.
        skill: SkillId,
        /// The token it carried.
        token: DateTime<Utc>,
        /// What it changes.
        patch: SkillPatch,
    },
    /// `SaveSkillVersion`.
    Version {
        /// Which skill.
        skill: SkillId,
        /// The head it carried; the save writes `token + 1`.
        token: i32,
        /// The body.
        body: String,
    },
    /// `SetSkillBinding`.
    Binding {
        /// The attachment's key.
        key: SkillBindingKey,
        /// The row's `updated_at` it carried; `None` for "no row".
        token: Option<DateTime<Utc>>,
        /// Attach or detach.
        change: BindingChange,
        /// What the notice calls the row.
        target: String,
    },
}

impl core::fmt::Debug for Sent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Create { name, body } => f
                .debug_struct("Create")
                .field("name", name)
                .field("body_len", &body.len())
                .finish(),
            Self::Rename {
                skill,
                token,
                patch,
            } => f
                .debug_struct("Rename")
                .field("skill", skill)
                .field("token", token)
                .field("patch", patch)
                .finish(),
            Self::Version { skill, token, body } => f
                .debug_struct("Version")
                .field("skill", skill)
                .field("token", token)
                .field("body_len", &body.len())
                .finish(),
            Self::Binding {
                key,
                token,
                change,
                target,
            } => f
                .debug_struct("Binding")
                .field("key", key)
                .field("token", token)
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

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// A `CONTROL` chord (`SHIFT` allowed).
fn chord(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
}

impl LibraryView {
    /// Whether an editor, a prompt, the rename form, or the attachments pane's form, picker or
    /// question is taking every key (D100).
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
            || self.attach.as_ref().is_some_and(AttachPane::captures_input)
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
        }
    }

    /// A reply addressed to the Skills tab (§6.4).
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Skills(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                self.land();
                self.clamp();
            }
            StoreReply::SkillsStale { snapshot, what } => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy.take().is_some() {
                    self.sent = None;
                    self.stale(*what);
                }
                self.clamp();
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
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
        let hint = match (&self.attach, &self.snapshot, &self.mode) {
            (Some(pane), Some(snapshot), _) => {
                pane.render(frame, content, snapshot, ctx).to_owned()
            }
            (_, _, Mode::Editing(editor)) => {
                self.render_editor(frame, content, editor, ctx.theme);
                let (line, col) = editor.area.cursor_line_col();
                format!("{EDIT_HINT}  L{}:C{}", line + 1, col + 1)
            }
            (_, _, mode) => {
                self.render_browse(frame, content, ctx);
                match mode {
                    Mode::Naming { .. } | Mode::Describing { .. } => NAMING_HINT,
                    Mode::Info(_) => INFO_HINT,
                    Mode::Browse | Mode::Editing(_) => BROWSE_HINT,
                }
                .to_owned()
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

    /// Browse (§6.3). Every key here misses the global table, and the tab took `h`/`l`/`[`/`]` and
    /// the arrows first.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !plain(&key) {
            return Handled::Pass;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(true),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(false),
            KeyCode::Char('r') => {
                self.notice = None;
                ctx.request(StoreRequest::Skills(ctx.scope.clone()));
            }
            KeyCode::Char('n') => {
                if self.snapshot.is_some() {
                    self.notice = None;
                    self.mode = Mode::Naming {
                        field: TextField::new(),
                    };
                }
            }
            KeyCode::Char('J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
                return self.scroll.on_key(key, self.pane_rows.get());
            }
            KeyCode::Char(c @ (',' | '.' | 'b' | 'd' | 'e' | 'E' | 'i' | 'a')) => {
                self.notice = None;
                self.on_skill_key(c, ctx);
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// A version, diff, edit, rename or attach key. With no skill under the cursor (an empty
    /// library, or nothing read yet) it does nothing.
    fn on_skill_key(&mut self, key: char, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(entry) = snapshot.skills.get(self.cursor) else {
            return;
        };
        let (skill, name) = (entry.skill.id, entry.skill.name.clone());
        let versions: Vec<i32> = entry.versions.iter().map(|row| row.version).collect();
        let (Some(head), Some(shown)) = (snapshot.head(skill), self.shown_row(snapshot, skill))
        else {
            return;
        };
        let (head, shown_version, shown_body) = (head.version, shown.version, shown.body.clone());
        let index = versions
            .iter()
            .position(|version| *version == shown_version)
            .unwrap_or(0);
        match key {
            ',' | '.' => {
                let index = if key == ',' {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(versions.len().saturating_sub(1))
                };
                let version = versions.get(index).copied().unwrap_or(head);
                self.shown = (version != head).then_some(version);
                self.scroll.reset();
            }
            'b' => {
                self.base = Some(shown_version);
                self.notice = Some(Notice::Info(format!("base v{shown_version}")));
            }
            'd' => {
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
            'e' => {
                let target = Target::Version { skill, name };
                self.mode =
                    Mode::Editing(Editor::new(target, head, Some(shown_version), &shown_body));
            }
            'E' => {
                ctx.emit(Action::EditExternally(ExternalEdit {
                    text: shown_body.clone(),
                    stem: name.clone(),
                }));
                let target = Target::Version { skill, name };
                self.external = Some(Pending {
                    editor: Editor::new(target, head, Some(shown_version), &shown_body),
                    resume: false,
                });
            }
            'i' => {
                self.mode = Mode::Info(InfoForm {
                    skill,
                    token: entry.skill.updated_at,
                    name: TextField::with_text(&name),
                    description: TextField::with_text(&entry.skill.description),
                    focus: 0,
                });
            }
            'a' => self.attach = Some(AttachPane::new(skill)),
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
                patch: patch.clone(),
            },
            Sent::Rename {
                skill,
                token,
                patch,
            },
            ctx,
        );
    }

    /// The editor: `Ctrl+S` (the area's `Submit`), `Ctrl+E` and `Esc` are the view's; `Tab` and
    /// `Shift+Tab` pass so the shell switches tabs with the draft kept; everything else is text.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
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
                Sent::Create {
                    name: name.clone(),
                    body,
                },
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
                    Sent::Version {
                        skill: *skill,
                        token: editor.token,
                        body,
                    },
                )
            }
        };
        self.send(request, sent, ctx);
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

    /// A key while the attachments pane is open: the pane decides, the view sends.
    fn on_attach_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let (Some(pane), Some(snapshot)) = (&mut self.attach, &self.snapshot) else {
            self.attach = None;
            return Handled::Pass;
        };
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

    /// D98: a `Skills` reply is the write's answer only while the write is in flight **and** the
    /// snapshot holds what it sent (§6.4's table). Otherwise it refreshes the library and leaves
    /// the draft, the token and `busy` alone: a read served ahead of the write, or another
    /// session's row at the same key.
    fn land(&mut self) {
        let (Some(snapshot), Some(sent)) = (&self.snapshot, &self.sent) else {
            return;
        };
        let landed = match sent {
            Sent::Create { name, body } => snapshot.by_name(name).is_some_and(|entry| {
                snapshot
                    .version(entry.skill.id, 1)
                    .is_some_and(|row| row.body == *body)
            }),
            Sent::Version { skill, token, body } => snapshot
                .version(*skill, token + 1)
                .is_some_and(|row| row.body == *body),
            Sent::Rename {
                skill,
                token,
                patch,
            } => snapshot.entry(*skill).is_some_and(|entry| {
                entry.skill.updated_at != *token
                    && patch
                        .name
                        .as_ref()
                        .is_none_or(|name| *name == entry.skill.name)
                    && patch
                        .description
                        .as_ref()
                        .is_none_or(|description| *description == entry.skill.description)
            }),
            Sent::Binding {
                key,
                token,
                change: BindingChange::Attach(attachment),
                ..
            } => snapshot.binding(*key).is_some_and(|row| {
                Some(row.updated_at) != *token
                    && (row.pinned_version, row.position, row.activation)
                        == (
                            attachment.pinned_version,
                            attachment.position,
                            attachment.activation,
                        )
                    && row.languages == skill_language::normalise(&attachment.languages)
            }),
            Sent::Binding {
                key,
                change: BindingChange::Detach,
                ..
            } => snapshot.binding(*key).is_none(),
        };
        if !landed {
            return;
        }
        self.busy = None;
        let Some(sent) = self.sent.take() else {
            return;
        };
        match sent {
            Sent::Create { name, body } => self.landed_version(&name, 1, &body),
            Sent::Version { skill, token, body } => {
                let name = snapshot_name(self.snapshot.as_ref(), skill);
                self.landed_version(&name, token + 1, &body);
            }
            Sent::Rename { skill, .. } => {
                let name = snapshot_name(self.snapshot.as_ref(), skill);
                self.select(skill);
                self.mode = Mode::Browse;
                self.notice = Some(Notice::Info(format!("saved `{name}`")));
            }
            Sent::Binding { change, target, .. } => {
                if let Some(pane) = &mut self.attach {
                    pane.on_landed();
                }
                self.notice = Some(Notice::Info(match change {
                    BindingChange::Attach(_) => format!("attached to {target}"),
                    BindingChange::Detach => format!("detached from {target}"),
                }));
            }
        }
    }

    /// A version landed (a create's v1 or an append): the cursor goes onto the skill and the pane
    /// back to its head's body. Keys typed while the save was in flight still edited the draft;
    /// when they did, the editor stays open on them with the token at the saved version, so the
    /// next `Ctrl+S` appends them (the Templates view's rule).
    fn landed_version(&mut self, name: &str, version: i32, body: &str) {
        let tokens = estimate(name, version, body);
        let Some(skill) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.by_name(name))
            .map(|entry| entry.skill.id)
        else {
            return;
        };
        self.select(skill);
        let verb = if version == 1 {
            format!("created `{name}` v1")
        } else {
            format!("saved v{version}")
        };
        let Mode::Editing(editor) = &mut self.mode else {
            self.notice = Some(Notice::Info(format!("{verb} \u{b7} ~{tokens} tokens")));
            return;
        };
        if editor.area.text() == body {
            self.mode = Mode::Browse;
            self.notice = Some(Notice::Info(format!("{verb} \u{b7} ~{tokens} tokens")));
        } else {
            editor.target = Target::Version {
                skill,
                name: name.to_owned(),
            };
            editor.token = version;
            editor.from = Some(version);
            body.clone_into(&mut editor.original);
            editor.esc_armed = false;
            self.notice = Some(Notice::Info(format!(
                "{verb} \u{b7} ~{tokens} tokens \u{2014} later edits kept, Ctrl+S saves them as \
                 v{}",
                version + 1
            )));
        }
    }

    /// A write missed its token, or its row is gone (§6.4's second table): the draft keeps its
    /// text and takes the token as it is now, so the next `Ctrl+S` is a deliberate overwrite.
    fn stale(&mut self, what: StaleWhat) {
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
                let kept = self
                    .attach
                    .as_mut()
                    .is_some_and(|pane| pane.on_stale(snapshot));
                let sentence = if kept {
                    BINDING_CHANGED_ELSEWHERE
                } else {
                    BINDING_GONE_ELSEWHERE
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
                let name = if entry.skill.name.chars().count() > NAME_WIDTH {
                    let cut: String = entry.skill.name.chars().take(NAME_WIDTH - 1).collect();
                    format!("{cut}\u{2026}")
                } else {
                    entry.skill.name.clone()
                };
                let style = if index == self.cursor {
                    theme.selected
                } else {
                    theme.base
                };
                Line::styled(format!("  {name:<NAME_WIDTH$} v{head:<3}"), style)
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
            block = block.title_bottom(Line::styled(SCROLL_HINT, theme.dim).right_aligned());
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
            let budget = width.saturating_sub(u16::try_from(prompt.chars().count()).unwrap_or(0));
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
            Mode::Browse | Mode::Editing(_) => {}
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

    /// The editor over the whole content, its title carrying the draft's estimate (D82).
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, theme: &Theme) {
        let name = editor.target.name();
        let tokens = estimate(name, editor.saves(), editor.area.text());
        let title = match (&editor.target, editor.from) {
            (Target::New { .. }, _) => {
                format!(" {name} \u{b7} new, saves v1 \u{b7} ~{tokens} tokens ")
            }
            (Target::Version { .. }, from) => format!(
                " {name} \u{b7} editing from v{}, saves v{} \u{b7} ~{tokens} tokens ",
                from.unwrap_or(editor.token),
                editor.saves()
            ),
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.page.set(inner.height);
        frame.render_widget(
            Paragraph::new(editor.area.lines(inner.width, inner.height, true, theme)),
            inner,
        );
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

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::Scope;
    use htui_core::store::{BLANK_SKILL_BODY, MemStore};
    use htui_store::Backend;

    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::skills;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::tabs::SkillsTab;

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

    /// D98: a `Skills` read served while a version save is in flight is not the save's answer.
    /// `settle` serves in queue order, save first, so only a direct drive can put a read's reply
    /// ahead of the save's.
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
            matches!(&view.notice, Some(Notice::Info(text)) if text.starts_with("saved v3 \u{b7} ~")),
            "{:?}",
            view.notice
        );
    }

    /// The same rule for the attachment form: the form stays open over a read that does not hold
    /// the row it sent, and closes on the one that does.
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
        let StoreRequest::SetSkillBinding { key, expected, .. } = save else {
            panic!("not an attach: {save:?}");
        };
        assert_eq!(
            (*key, *expected),
            (
                SkillBindingKey {
                    skill: ids::SKILL_RUST_STYLE,
                    project: Some(ids::PROJECT_VULKAN),
                    phase: None,
                },
                None
            )
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
                skill: ids::SKILL_TESTS,
                token: 1,
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
                name: "docs".to_owned(),
                body: "secret body".to_owned(),
            }
        );
        assert!(!create.contains("secret"), "{create}");
    }
}
