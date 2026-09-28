//! The Skills view of the Skills tab (MOD-9 milestone 3, T3; plan D82, D84; blueprint D93, D101,
//! D106, D108): the library, a skill's versions, the line diff, the body editor and the save.
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle and
//! no `UserId`, renders from the last `SkillsSnapshot` and never patches a row into it. A skill
//! body is markdown, not a template, so there is no `parse` here: the name rule and the two
//! compare-and-set tokens are the whole gate, and they live in the writer ([`crate::skills`]).
//!
//! The token a save carries is the head version when the editor opened, not the version shown:
//! editing v1 while v3 is head saves v4 (plan D1, OQ-5). `skill.updated_at` is the second token
//! and guards the description (D101).

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::{Skill, SkillVersion};
use htui_core::prompt::TokenEstimator;
// `invalid_skill_name` is reached through the module rather than a `store::` re-export: unlike
// `invalid_template_name`, T1 did not add it to `store/mod.rs`'s list, and that file is not in
// T3's. The function is the same one both stores call, which is the point — the view's sentence
// and the writer's cannot drift (D100).
use htui_core::store::traits::invalid_skill_name;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome};
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillBody, SkillsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme, diff};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The `StoreRequest::SaveSkill` name, what `busy` holds while it is in flight. The slice index,
/// not a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const SAVE_NAME: &str = REQUEST_NAMES[1];

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The library's width, borders included: `  {name:<26} v{head:<3} {activation:<5} {tokens}` is at
/// most 43 chars for a three-figure estimate, a longer name cut to [`NAME_WIDTH`].
///
/// **This view's own widths, not the Templates view's** (blueprint D106, H-27): a skill row carries
/// a version, an activation and a token estimate where a template row carried a version and a
/// role. `templates.rs`'s 40 and 44 do not move, which is the structural reason the six
/// `templates__*.snap` files cannot.
const LIST_WIDTH: u16 = 46;

/// A skill row's name field, in chars: a longer name is cut to `NAME_WIDTH - 1` and `…`, so the
/// head, the activation and the estimate stay on the row.
const NAME_WIDTH: usize = 26;

/// The help column's width, borders included: the longest help line is 25 chars.
const HELP_WIDTH: u16 = 38;

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";

/// A key pressed with nothing under the cursor.
const SELECT_A_SKILL: &str = "select a skill";

/// `Esc` over a modified draft, the first time.
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A save went out.
const SAVING: &str = "saving\u{2026}";

/// An `$EDITOR` return that came back with text.
const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// An `$EDITOR` return that changed nothing.
const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within `QUICK_EXIT`.
const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row in Browse. The Templates view's clauses in its order, with the skills verbs: no
/// `D default` (a skill has no compiled seed) and no `h/l view` (the tab owns those).
const BROWSE_HINT: &str = "j/k move  ,/. version  b base  d diff  e description  E edit  \
                           n new  r reload";

/// The hint row while naming a skill or editing its description.
const NAMING_HINT: &str = "Tab next field  Enter confirm  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}`.
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+E $EDITOR  Esc cancel";

/// The activation column's "attached nowhere in this scope".
const NO_ATTACHMENT: &str = "\u{2014}";

/// What the editor's right-hand pane lists.
///
/// A skill body is markdown, so there is no placeholder to enumerate and the Templates view's
/// `Placeholder::ALL` pane has no analogue here. What a user about to press `Ctrl+S` needs
/// instead is what that key is about to do to **two** tables, which is D101 in the place it is
/// acted on. The longest line is 25 chars, inside [`HELP_WIDTH`].
const EDITOR_HELP: &[&str] = &[
    "the name is the library key",
    "and it never moves",
    "",
    "a save appends a version;",
    "it never rewrites one",
    "",
    "two tokens guard it: the",
    "head version, and the",
    "skill's updated_at",
    "",
    "a spent token keeps the",
    "draft and moves both",
    "",
    "the body is markdown, so",
    "nothing parses it here",
    "",
    "Ctrl+E hands the draft to",
    "$EDITOR",
];

/// The save that landed while the head moved: the draft is kept and **both** tokens move to the
/// head the user has now been told about, so the next `Ctrl+S` is a deliberate
/// overwrite-by-append rather than a second stale refusal.
fn skill_changed_elsewhere(head: i32) -> String {
    format!(
        "this skill changed elsewhere and is now at v{head}; the draft is kept, and Ctrl+S \
         appends to v{}",
        head + 1
    )
}

/// The Skills view. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct SkillsView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale list.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](SkillsView::rows).
    cursor: usize,
    /// The version shown for the selected skill; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against; `None` is the shown version's predecessor.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// The pane's first drawn row (`J`/`K`, `PageDown`/`PageUp`). Back to the top whenever the
    /// pane shows something else: another row, another version, the other pane.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](SkillsView::scroll) clamps against. A
    /// `Cell` for [`page`](SkillsView::page)'s reason.
    pane_rows: Cell<usize>,
    /// Browsing, naming a skill, or editing its body.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`. One at a time (`settings/prompt.rs`'s rule):
    /// the staleness index keeps only the newest request of a kind.
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height: what `PageUp`/`PageDown` move by. A `Cell` because the
    /// height is known only in `render(&self)`.
    page: Cell<u16>,
}

/// One row of the library, derived from the snapshot on demand.
///
/// **One variant, not two**: the library is global (`skill` has no `project_id`, plan D92), so
/// there is no project header to draw — the first and only structural deviation from the
/// Templates view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// One library entry, by name: the row's label and its identity.
    Skill(String),
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the library.
    #[default]
    Browse,
    /// `n` (empty) or `e` (filled): the name and the description. `Tab` moves between the two
    /// fields, so create and edit-description are one code path and one hint.
    ///
    /// **Two fields, not one**: a template has no description, so `templates.rs`'s
    /// `Naming { project, field }` carries one, while a skill's name **and** description are both
    /// `R-SKL-1` ("the library is `name` + `description` + versioned body").
    Naming {
        /// The library key. Prefilled and immutable under `e`; free under `n`.
        name: TextField,
        /// `skill.description`, the list's one-liner.
        description: TextField,
        /// Which field has the cursor. The Templates view's single field has no such state, so
        /// `Tab` is what makes this one necessary.
        focus: Focus,
        /// `true` when the name is fixed (`e`), so a typing key never reaches it.
        fixed_name: bool,
    },
    /// `E`, or `Enter` on the form: the body editor.
    Editing(Editor),
}

/// Which of [`Mode::Naming`]'s two fields has the cursor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The library key, which is what `n` opens on.
    #[default]
    Name,
    /// The description, which is what `e` opens on.
    Description,
}

/// An open body editor. Never `Debug`s a body.
struct Editor {
    /// The library key, and the subject of the `upsert_skill` half of the save.
    name: String,
    /// The head version when the editor opened: the `add_skill_version` token (`None`: a new
    /// name).
    token: Option<i32>,
    /// The skill's `updated_at` when the editor opened: the `upsert_skill` token (`None`: no row).
    /// **D101**: two tokens, two tables, and the request carries both.
    updated_at: Option<DateTime<Utc>>,
    /// The version the draft started from, for the pane title (`None`: a new name).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// The description the save carries: what `e` last left in the form, and the stored one when
    /// the editor was opened with `E`.
    ///
    /// Never empty in practice, and the reason is not politeness: `SaveSkill` always carries a
    /// description, so an empty one would be a save that erases the row's one-liner. The
    /// `updated_at` token is what says the row has not moved underneath in the meantime.
    description: String,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// The body the save in flight carries: what tells that save's version from another session's,
    /// and a draft typed on since from the one that was saved.
    sent: Option<String>,
}

impl Editor {
    /// An editor over `text` (line ends normalised by `TextArea::with_text`), cursor at byte 0.
    fn new(
        name: String,
        token: Option<i32>,
        updated_at: Option<DateTime<Utc>>,
        from: Option<i32>,
        description: String,
        text: &str,
    ) -> Self {
        let area = TextArea::with_text(text);
        Self {
            name,
            token,
            updated_at,
            from,
            original: area.text().to_owned(),
            area,
            description,
            esc_armed: false,
            sent: None,
        }
    }
}

/// Lengths, never the text: `original` and `sent` are the body, as the draft is (`TextArea`'s
/// rule). `description` is the library's one-liner and is shown in the form, so its length is the
/// honest half here.
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("name", &self.name)
            .field("token", &self.token)
            .field("updated_at", &self.updated_at)
            .field("from", &self.from)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("description_len", &self.description.len())
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
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
    /// Whether the handoff came from the in-app editor (`Ctrl+E`).
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

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The diff from the base to the shown version.
    Diff,
}

/// What `d` diffs the shown version against.
///
/// **Two variants, not the Templates view's three**: the Templates view's `D` diffs against the
/// compiled default, and a skill has no compiled seed, so the "default" of that key would have no
/// key to press.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    /// The shown version's predecessor.
    #[default]
    Predecessor,
    /// One named version (`b`).
    Version(i32),
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

impl SkillsView {
    /// Whether an editor or the form is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// The scope changed: the library, the editor, the pending handoff and the write in flight all
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
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Naming { .. } => self.on_naming_key(key),
            Mode::Editing(_) => self.on_editor_key(key, ctx),
        }
    }

    /// A reply addressed to the Skills tab.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Skills(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                self.land_save();
                self.clamp_cursor();
            }
            StoreReply::SkillsStale(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy == Some(SAVE_NAME) {
                    self.busy = None;
                    if let Mode::Editing(editor) = &mut self.mode {
                        editor.sent = None;
                        if let Some(entry) = snapshot.named(&editor.name) {
                            // The draft stays; both tokens move to the head the user has now been
                            // told about, so the next `Ctrl+S` is a deliberate append.
                            editor.token = entry.versions.last().map(|row| row.version);
                            editor.updated_at = Some(entry.updated_at);
                            self.notice = Some(Notice::Error(skill_changed_elsewhere(
                                editor.token.unwrap_or_default(),
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

    /// The `$EDITOR` handoff came back. No handoff pending (the scope changed meanwhile): ignored.
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        let Some(Pending { mut editor, resume }) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(text) => {
                // **No gate here, and that is the whole difference from the Templates view**:
                // there, `Ctrl+E` re-parses the returned text so a broken body never reaches the
                // editor. A skill body is markdown and the writer has no parser, so there is
                // nothing to refuse and no byte to put the cursor on.
                editor.area = TextArea::with_text(&text);
                editor.esc_armed = false;
                self.notice = Some(Notice::Info(EDITED.to_owned()));
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
        // The notice wraps rather than clips (at most `NOTICE_LINES`): the stale sentence names
        // the version the next `Ctrl+S` writes, which is the one part the user must not lose.
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
        let hint = match &self.mode {
            Mode::Editing(editor) => {
                self.render_editor(frame, content, editor, ctx.theme);
                let (line, col) = editor.area.cursor_line_col();
                format!("{EDIT_HINT}  L{}:C{}", line + 1, col + 1)
            }
            Mode::Naming { .. } => {
                self.render_browse(frame, content, ctx);
                NAMING_HINT.to_owned()
            }
            Mode::Browse => {
                self.render_browse(frame, content, ctx);
                BROWSE_HINT.to_owned()
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

    /// Browse (plan D82). Every key here misses the global table: `q`, `Tab`, `Shift+Tab`, the
    /// digits, `?` and `w` are not among them, and the tab took `h`/`l`/`[`/`]`/arrows first.
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
            KeyCode::Char('n') => self.open_naming(),
            KeyCode::Char('J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
                return self.scroll.on_key(key, self.pane_rows.get());
            }
            KeyCode::Char(c @ (',' | '.' | 'b' | 'd' | 'e' | 'E')) => {
                self.notice = None;
                match self.selected() {
                    Some(name) => self.on_skill_key(c, name, ctx),
                    None if !self.rows().is_empty() => {
                        self.notice = Some(Notice::Info(SELECT_A_SKILL.to_owned()));
                    }
                    None => {}
                }
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// A version, diff or edit key with a skill under the cursor.
    fn on_skill_key(&mut self, key: char, name: String, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(entry) = snapshot.named(&name) else {
            return;
        };
        let (Some(head), Some(shown)) = (entry.versions.last(), self.shown_row(snapshot, &name))
        else {
            return;
        };
        let (head_version, shown_version, shown_body) =
            (head.version, shown.version, shown.body.clone());
        let versions: Vec<&SkillVersion> = entry.versions.iter().collect();
        let index = versions
            .iter()
            .position(|row| row.version == shown_version)
            .unwrap_or(0);
        let has_earlier = versions.iter().any(|row| row.version == shown_version - 1);
        let description = entry.description.clone();
        let updated_at = entry.updated_at;
        match key {
            ',' | '.' => {
                let index = if key == ',' {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(versions.len().saturating_sub(1))
                };
                let version = versions.get(index).map_or(head_version, |row| row.version);
                self.shown = (version != head_version).then_some(version);
                self.scroll.reset();
            }
            'b' => {
                self.base = Some(DiffBase::Version(shown_version));
                self.notice = Some(Notice::Info(format!("base v{shown_version}")));
            }
            'd' => {
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
            // `e` opens the form; `E` skips it and goes straight to the body, carrying the stored
            // description so the save does not have to guess one.
            'e' => self.open_description(),
            'E' => {
                let editor = Editor::new(
                    name,
                    Some(head_version),
                    Some(updated_at),
                    Some(shown_version),
                    description,
                    &shown_body,
                );
                self.mode = Mode::Editing(editor);
            }
            _ => {}
        }
        let _ = ctx;
    }

    /// `n`: the form, both fields empty.
    fn open_naming(&mut self) {
        self.notice = None;
        self.scroll.reset();
        self.mode = Mode::Naming {
            name: TextField::new(),
            description: TextField::new(),
            focus: Focus::Name,
            fixed_name: false,
        };
    }

    /// `e`: the form with the selected skill's name prefilled and fixed and its description
    /// prefilled and focused. Nothing is sent until `Enter`, which opens the body editor with
    /// that description — so "rename" is not a verb this view has, and does not pretend to be.
    fn open_description(&mut self) {
        let Some(name) = self.selected() else {
            return;
        };
        let Some(description) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.named(&name))
            .map(|entry| entry.description.clone())
        else {
            return;
        };
        self.notice = None;
        self.scroll.reset();
        self.mode = Mode::Naming {
            name: TextField::with_text(&name),
            description: TextField::with_text(&description),
            focus: Focus::Description,
            fixed_name: true,
        };
    }

    /// The two-field form. `Tab` and `Shift+Tab` move the cursor in both directions — the
    /// Templates view's single-field prompt lets `Tab` through to the shell, which is right for
    /// one field and wrong for two (H-32). A refused confirm keeps the form open, so a typo is
    /// one `Backspace` away.
    fn on_naming_key(&mut self, key: KeyEvent) -> Handled {
        let Mode::Naming {
            name,
            description,
            focus,
            fixed_name,
        } = &mut self.mode
        else {
            return Handled::Pass;
        };
        if plain(&key) && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            *focus = match focus {
                Focus::Name => Focus::Description,
                Focus::Description => Focus::Name,
            };
            return Handled::Consumed;
        }
        let (field, typing_allowed) = match focus {
            Focus::Name => (name, !*fixed_name),
            Focus::Description => (description, true),
        };
        if !typing_allowed
            && !matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Enter
                    | KeyCode::Esc
            )
        {
            // `e` fixed the name because it is the library key and a save never moves it. The
            // cursor still walks it, so the field reads as a field; nothing else reaches it.
            return Handled::Consumed;
        }
        match field.on_key(key) {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Pass => Handled::Pass,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                self.confirm_naming();
                Handled::Consumed
            }
        }
    }

    /// `Enter` on the form. Under `e` the name is the key of a skill that exists, so the editor
    /// opens on it; under `n` the name is a key nothing holds yet, so the name rule decides and
    /// an editor opens with no token at all.
    fn confirm_naming(&mut self) {
        let Mode::Naming {
            name,
            description,
            fixed_name,
            ..
        } = &self.mode
        else {
            return;
        };
        let (name, description, fixed) = (
            name.text().unwrap_or_default().to_owned(),
            description.text().unwrap_or_default().to_owned(),
            *fixed_name,
        );
        if fixed {
            // `e` opened on a skill the snapshot holds, so its head and its `updated_at` are
            // right there: the editor opens on the shown version with the two tokens a save
            // needs (D101) and the body the pane was showing.
            let head = self.snapshot.as_ref().and_then(|s| s.head(&name)).cloned();
            let updated_at = self
                .snapshot
                .as_ref()
                .and_then(|s| s.named(&name))
                .map(|entry| entry.updated_at);
            let (token, from, body) = head
                .map(|head| (Some(head.version), Some(head.version), head.body))
                .unwrap_or((None, None, String::new()));
            self.mode = Mode::Editing(Editor::new(
                name,
                token,
                updated_at,
                from,
                description,
                &body,
            ));
            return;
        }
        if !Skill::name_is_valid(&name) {
            self.notice = Some(Notice::Error(invalid_skill_name(&name)));
            return;
        }
        if self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.named(&name).is_some())
        {
            self.notice = Some(Notice::Error(format!(
                "`{name}` exists \u{2014} select it and press e"
            )));
            return;
        }
        self.notice = None;
        self.mode = Mode::Editing(Editor::new(name, None, None, None, description, ""));
    }

    /// The editor: `Ctrl+S` (the area's `Submit`), `Ctrl+E` and `Esc` are the view's, `Tab` and
    /// `Shift+Tab` pass so the shell switches tabs with the draft kept, everything else is text.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL;
        if chord && matches!(key.code, KeyCode::Char('e' | 'E')) {
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
                self.save(ctx);
                Handled::Consumed
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
                Handled::Consumed
            }
            FieldOutcome::Pass => Handled::Pass,
        }
    }

    /// `Ctrl+S` (D78's OQ-18 default, D101's two tokens). **No parse gate**: a skill body is
    /// markdown, not a template, so there is no byte-offset error to point at and no role's
    /// closed set. First match wins: a write in flight, then the save. A body byte-identical to
    /// the head's still appends, exactly as the Templates view appends unconditionally (PRD D5) —
    /// the diff pane makes the no-op visible *before* the save, which is where the plan puts it.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(format!("`{busy}` is still in flight")));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let body = editor.area.text().to_owned();
        editor.sent = Some(body.clone());
        self.busy = Some(SAVE_NAME);
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(StoreRequest::SaveSkill {
            scope: ctx.scope.clone(),
            name: editor.name.clone(),
            description: editor.description.clone(),
            body: SkillBody::new(body),
            // D101: both tokens, read from the same snapshot the editor opened on.
            expected: editor.updated_at,
            expected_version: editor.token,
        });
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

    // --- state ---------------------------------------------------------------------------------

    /// The flat list the cursor indexes, one row per library entry in the read's order
    /// (`skill.name` bytes, so the order is defined). Derived, so it cannot disagree with the
    /// snapshot.
    fn rows(&self) -> Vec<Row> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .skills
                    .iter()
                    .map(|entry| Row::Skill(entry.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The skill under the cursor, if the cursor is on one.
    fn selected(&self) -> Option<String> {
        match self.rows().into_iter().nth(self.cursor)? {
            Row::Skill(name) => Some(name),
        }
    }

    /// The version shown for `name`: the pinned one, else the head. A version the snapshot no
    /// longer holds falls back to the head.
    fn shown_row<'a>(&self, snapshot: &'a SkillsSnapshot, name: &str) -> Option<&'a SkillVersion> {
        self.shown
            .and_then(|version| snapshot.version(name, version))
            .or_else(|| snapshot.head(name))
    }

    /// The activation column: the first attachment the read answered for this skill, else an em
    /// dash. The matrix is where the three levels are compared (T4); a library row is one line
    /// and can only name one of them, so it names the first in the read's own order and says so
    /// here rather than pretending to be a resolution.
    fn activation_of(&self, name: &str) -> &'static str {
        let Some(entry) = self.snapshot.as_ref().and_then(|s| s.named(name)) else {
            return NO_ATTACHMENT;
        };
        self.snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .attachments
                    .iter()
                    .find(|row| row.skill_id == entry.id)
            })
            .map_or(NO_ATTACHMENT, |row| row.activation.as_str())
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

    /// Keeps the cursor on a row after the library changed.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// A `Skills` reply is the save's answer only while the save is in flight **and** it holds a
    /// version above the token whose body is the one that was sent. A read served before the save
    /// (a `Tab` away and back, `2`, `r`) refreshes the library and leaves the editor, the tokens
    /// and `busy` alone; a read showing another session's `token + 1` is not the answer either,
    /// because the save's own reply is then `SkillsStale`, which keeps the draft.
    fn land_save(&mut self) {
        if self.busy != Some(SAVE_NAME) {
            return;
        }
        let (Some(snapshot), Mode::Editing(editor)) = (&self.snapshot, &self.mode) else {
            return;
        };
        let Some(sent) = editor.sent.as_deref() else {
            return;
        };
        let version = editor.token.unwrap_or(0) + 1;
        if snapshot
            .version(&editor.name, version)
            .is_none_or(|row| row.body != sent)
        {
            return;
        }
        let saved_name = editor.name.clone();
        let saved = Row::Skill(editor.name.clone());
        self.busy = None;
        self.shown = None;
        self.base = None;
        self.pane = Pane::Body;
        self.scroll.reset();
        if let Some(index) = self.rows().iter().position(|row| *row == saved) {
            self.cursor = index;
        }
        // The description moved with the body, so the second token moves too: a second `Ctrl+S`
        // must not re-send a `updated_at` the save that just landed has already spent.
        let updated_at = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.named(&saved_name))
            .map(|entry| entry.updated_at);
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        let sent = editor.sent.take().unwrap_or_default();
        if editor.area.text() == sent {
            self.notice = Some(Notice::Info(format!("saved v{version}")));
            self.mode = Mode::Browse;
        } else {
            editor.token = Some(version);
            editor.updated_at = updated_at;
            editor.from = Some(version);
            editor.original = sent;
            editor.esc_armed = false;
            self.notice = Some(Notice::Info(format!(
                "saved v{version} \u{2014} later edits kept, Ctrl+S saves them as v{}",
                version + 1
            )));
        }
    }

    // --- frames --------------------------------------------------------------------------------

    /// Browse and Naming: the library on the left, the pane on the right.
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let [list_area, pane_area] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)]).areas(area);

        let list = Block::new().borders(Borders::ALL).title(" Skills ");
        let inner = list.inner(list_area);
        frame.render_widget(list, list_area);
        let lines: Vec<Line<'static>> = self
            .rows()
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (text, style) = match row {
                    Row::Skill(name) => {
                        let head = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.named(name))
                            .map_or(0, |entry| entry.versions.len() as i32);
                        let activation = self.activation_of(name);
                        // MOD-9 D84: the head body's token estimate, computed here in `render`
                        // from bytes the snapshot already holds. Thirty rows is thirty short
                        // scans of strings in memory, no store round-trip, and
                        // `TokenEstimator::estimate` is a pure function of one `&str` — so
                        // `R-NF-3` holds. The estimator's own id goes in the pane title, so one
                        // id never carries two arithmetics (D108).
                        let tokens = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.named(name))
                            .and_then(|entry| entry.versions.last())
                            .map_or_else(String::new, |head| {
                                format!("~{}", TokenEstimator::DEFAULT.estimate(&head.body))
                            });
                        let name = if name.chars().count() > NAME_WIDTH {
                            let cut: String = name.chars().take(NAME_WIDTH - 1).collect();
                            format!("{cut}\u{2026}")
                        } else {
                            name.clone()
                        };
                        (
                            format!("  {name:<NAME_WIDTH$} v{head:<3} {activation:<5} {tokens}"),
                            theme.base,
                        )
                    }
                };
                let style = if index == self.cursor {
                    theme.selected
                } else {
                    style
                };
                Line::styled(text, style)
            })
            .collect();
        let offset = self
            .cursor
            .saturating_sub(usize::from(inner.height).saturating_sub(1));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            inner,
        );

        // The body and the diff wrap: a skill body runs past the pane's edge, and a change past
        // it would be off screen. The row count is a character wrap's, a lower bound on the word
        // wrap's (`Scroll`'s rule), so the clamp never scrolls the pane blank.
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

    /// The right-hand pane's title and lines: the form, a refusal, the empty states, the shown
    /// body, or the diff. The caller wraps and scrolls them.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        let theme = ctx.theme;
        if let Mode::Naming {
            name,
            description,
            focus,
            ..
        } = &self.mode
        {
            let mut lines = Vec::with_capacity(2);
            for (label, field, focused) in [
                (" name: ", name, *focus == Focus::Name),
                (" description: ", description, *focus == Focus::Description),
            ] {
                let budget =
                    width.saturating_sub(u16::try_from(label.chars().count()).unwrap_or(0));
                let mut spans = vec![Span::styled(label, theme.base)];
                spans.extend(field.line(budget, focused, theme).spans);
                lines.push(Line::from(spans));
            }
            return (" name and description ".to_owned(), lines);
        }
        let dim = |text: String| (String::new(), vec![Line::styled(text, theme.dim)]);
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                vec![Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)],
            );
        }
        let Some(snapshot) = &self.snapshot else {
            return dim(NOT_READ.to_owned());
        };
        let Some(name) = self.selected() else {
            return dim(SELECT_A_SKILL.to_owned());
        };
        let (Some(head), Some(shown)) = (snapshot.head(&name), self.shown_row(snapshot, &name))
        else {
            return dim(SELECT_A_SKILL.to_owned());
        };
        if self.pane == Pane::Body {
            return (
                format!(
                    " {name} v{} (head v{}) ~{} tok ({}) ",
                    shown.version,
                    head.version,
                    TokenEstimator::DEFAULT.estimate(&shown.body),
                    TokenEstimator::DEFAULT.id,
                ),
                shown
                    .body
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), theme.base))
                    .collect(),
            );
        }
        let base = match self.base {
            Some(DiffBase::Version(version)) => snapshot
                .version(&name, version)
                .map(|row| (format!("v{version}"), row.body.as_str())),
            Some(DiffBase::Predecessor) | None => snapshot
                .version(&name, shown.version - 1)
                .map(|row| (format!("v{}", row.version), row.body.as_str())),
        };
        let Some((label, text)) = base else {
            return (
                format!(" {name} v{} ", shown.version),
                vec![Line::styled(
                    format!("v{} has no base to diff against", shown.version),
                    theme.dim,
                )],
            );
        };
        let unified = diff::unified(
            text,
            &shown.body,
            &format!("{name} {label}"),
            &format!("{name} v{}", shown.version),
        );
        (
            format!(" diff {label} \u{2192} v{} ", shown.version),
            diff::lines(&unified, theme),
        )
    }

    /// The editor: the draft on the left, what a save does on the right.
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, theme: &Theme) {
        let [left, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(HELP_WIDTH)]).areas(area);
        let title = match (editor.token, editor.from) {
            (Some(token), from) => format!(
                " {} \u{b7} editing from v{}, saves v{} ",
                editor.name,
                from.unwrap_or(token),
                token + 1
            ),
            (None, _) => format!(" {} \u{b7} new, saves v1 ", editor.name),
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(left);
        frame.render_widget(block, left);
        self.page.set(inner.height);
        frame.render_widget(
            Paragraph::new(editor.area.lines(inner.width, inner.height, true, theme)),
            inner,
        );

        let lines: Vec<Line<'static>> = EDITOR_HELP
            .iter()
            .map(|line| Line::styled((*line).to_owned(), theme.base))
            .collect();
        let block = Block::new()
            .borders(Borders::ALL)
            .title(format!(" {} ", editor.name));
        let inner = block.inner(right);
        frame.render_widget(block, right);
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

/// Whether a snapshot answers the scope the view is in now: a save's reply that crossed a scope
/// change is fresh to the staleness index (its kind was not re-issued), and its rows belong to the
/// workspace that was left.
///
/// **Containment, not equality, and the difference is `StoreReply::Skills`'s**: the reply carries
/// a `SkillsSnapshot` and no `Scope`, and the library half of that snapshot is global, so the
/// scope's project list cannot be read back off it. What can be checked is that no attachment
/// names a project outside the current scope, which is the case a scope change actually produces.
/// `on_scope_change` has already dropped the previous snapshot, so this only ever sees a reply
/// that was in flight across the change.
fn in_scope(snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> bool {
    snapshot.attachments.iter().all(|row| {
        row.project_id
            .is_none_or(|project| ctx.scope.project_ids.contains(&project))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::{KeyChord, KeyScope, Keymap};
    use crate::store_worker::Origin;
    use crate::ui::tabs::SkillsTab;
    use htui_core::fixtures::ids;
    use htui_core::model::{Activation, ProjectId, Scope, SkillAttachmentRow, SkillBindingId};

    /// The scope the tests are in, as the shell builds it.
    fn scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// A context addressed to the Skills tab.
    fn bench() -> (TopBarState, Keymap, Theme, Emit) {
        (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        )
    }

    /// One attachment row on `project`.
    fn attachment(project: Option<ProjectId>) -> SkillAttachmentRow {
        SkillAttachmentRow {
            id: SkillBindingId::new(),
            skill_id: ids::SKILL_TESTS,
            name: "tests".to_owned(),
            project_id: project,
            project_slug: project.map(|_| "p".to_owned()),
            phase_id: None,
            phase_name: None,
            pinned_version: None,
            position: 0,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
            updated_at: htui_core::fixtures::demo_at(0, 0),
        }
    }

    /// A snapshot whose only content is one attachment.
    fn with_attachment(project: Option<ProjectId>) -> SkillsSnapshot {
        SkillsSnapshot {
            skills: Vec::new(),
            attachments: vec![attachment(project)],
        }
    }

    /// H-29: a reply issued for the scope that was left is not painted over the new one. The
    /// attachment half of the snapshot is what says so, because the library half is global and
    /// carries no scope at all — which is the whole reason this guard is a containment check.
    #[test]
    fn a_reply_from_a_left_scope_is_not_painted() {
        let scope = scope();
        let (top_bar, keymap, theme, emit) = bench();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );

        assert!(
            in_scope(&with_attachment(Some(ids::PROJECT_VULKAN)), &ctx),
            "a row on a project of this scope"
        );
        assert!(
            in_scope(&with_attachment(None), &ctx),
            "a global row belongs to every scope"
        );
        assert!(
            !in_scope(&with_attachment(Some(ids::PROJECT_HTUI)), &ctx),
            "a row on the workspace that was left is not this scope's"
        );
    }

    /// D102 / F-13: every browse key the view claims is checked against the global table, so a key
    /// added to `default_global` later cannot silently become a second binding.
    #[test]
    fn every_browse_key_misses_the_global_table() {
        let map = Keymap::default_global();
        let claimed = [
            "j", "k", "down", "up", "r", "n", "e", "E", ",", ".", "b", "d", "J", "K", "pagedown",
            "pageup", "ctrl-s", "ctrl-e",
        ];
        for spec in claimed {
            let chord = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` parses"));
            assert!(
                map.resolve(&KeyScope::Global, chord).is_none(),
                "`{spec}` is claimed by the browse keys and must not be in the global table"
            );
        }
        // `Tab` is the one deliberate exception, and only inside the form.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("tab").expect("parses"))
                .is_some(),
            "the form's `Tab` shadows the global next-tab, which is why H-32 names the hint"
        );
        // D102: `w` is T6's and is not bound anywhere, so the view leaves it alone.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("w").expect("parses"))
                .is_none(),
            "`w` is unbound and unused, so the global table does not claim it either"
        );
    }

    /// The view derives `Debug` down to the open editor, whose `original` and `sent` are the body:
    /// lengths only, as the draft's `TextArea`.
    #[test]
    fn an_open_editor_debug_prints_lengths_not_text() {
        let mut editor = Editor::new(
            "house-rules".to_owned(),
            Some(1),
            None,
            Some(1),
            "secret description".to_owned(),
            "secret original",
        );
        editor.sent = Some("secret sent".to_owned());
        let view = SkillsView {
            mode: Mode::Editing(editor),
            ..SkillsView::default()
        };
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret original"), "{shown}");
        assert!(!shown.contains("secret sent"), "{shown}");
        assert!(!shown.contains("secret description"), "{shown}");
        assert!(shown.contains("original_len: 15"), "{shown}");
        assert!(shown.contains("sent_len: Some(11)"), "{shown}");
    }
}
