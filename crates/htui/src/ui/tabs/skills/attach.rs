//! The attachments pane of the Skills view (MOD-9 milestone 3, plan D83; blueprint D101-D104):
//! one skill's attachments, a `global` row, then each scope project's row and its non-override
//! graphs' phases, with a form for one row and a picker of the project's repo names.
//!
//! The pane holds no snapshot and sends nothing: every method takes the view's
//! [`SkillsSnapshot`], and a save comes back to the view as [`AttachOutcome::Save`], which owns
//! the one-write-at-a-time rule, the notice and the landing (`library.rs`, D98). The form's
//! `effective:` line runs the writer's own [`canonical_globs`] on every draw, and its repo line the
//! writer's own sentences, so what the form shows before save is what the store would store or
//! refuse (D79, D93). The store checks everything again behind its compare-and-set (D78).
//!
//! A `glob` attachment is written and shown, but nothing matches it before PRD milestone 5 (D86,
//! OQ-14): its row says so, as a run's record says `no_path`.

use chrono::{DateTime, Utc};
use htui_core::model::skill::resolve;
use htui_core::model::skill_glob::{canonical_globs, split_list};
use htui_core::model::skill_language;
use htui_core::model::{
    Activation, Attachment, BindingChange, PhaseId, ProjectId, SkillBinding, SkillBindingKey,
    SkillGlob, SkillId, SkillLevel,
};
use htui_core::store::{glob_names_unknown_repo, global_glob_names_a_repo};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::library::{Notice, Sent};
use crate::app::Ctx;
use crate::skills::{ProjectSkills, SkillsSnapshot};
use crate::store_worker::StoreRequest;
use crate::ui::{TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A row's label field, in chars: a longer label is cut to `LABEL_WIDTH - 1` and `…`, so the
/// summary stays on the row.
const LABEL_WIDTH: usize = 28;

/// The form's label column, in chars (`description` is the longest label, `effective:` is 10).
const FORM_LABEL: usize = 12;

/// The form panel's height, borders included: seven lines.
const FORM_HEIGHT: u16 = 9;

/// A row with no attachment.
const NO_ROW: &str = "\u{2014}";

/// A `glob` row, until PRD milestone 5 (R-28).
const FIRES_LATER: &str = "fires from milestone 5";

/// The form's last line: the any-repo rule of a bare glob (R-30).
const GLOB_HELP: &str =
    "a bare glob matches in every repo of the project; <repo>:<glob> matches in that repo only";

/// `Ctrl+R` on the global row.
const GLOBAL_HAS_NO_REPOS: &str = "a global attachment's globs cannot name a repo";

/// A pin the form cannot read.
const BAD_PIN: &str = "pin is `latest` or a version number";

/// A position the form cannot read.
const BAD_POSITION: &str = "position is a whole number, 0 or more";

/// `x` on a row with no attachment.
const NOTHING_ATTACHED: &str = "nothing attached here";

/// Any key but `y` at the detach question.
const KEPT: &str = "kept";

/// The hint row in Browse.
const BROWSE_HINT: &str = "j/k move  Enter edit  x detach  r reload  Esc back";

/// The hint row on the form.
const FORM_HINT: &str = "Tab/Up/Down field  Space activation  Ctrl+R repo  Ctrl+S save  Esc cancel";

/// The hint row in the repo picker.
const PICKER_HINT: &str = "j/k move  Enter insert  Esc back";

/// The hint row at the detach question.
const CONFIRM_HINT: &str = "y detach  any other key keeps it";

/// The attachments of one skill (D83). Holds no snapshot; every method takes the view's.
#[derive(Debug)]
pub(super) struct AttachPane {
    /// Whose attachments.
    skill: SkillId,
    /// The highlighted row, an index into [`rows`].
    cursor: usize,
    /// Browsing, the form, the picker, or the detach question.
    mode: AttachMode,
}

/// What the pane's keys are doing.
#[derive(Debug, Default)]
enum AttachMode {
    /// Moving through the rows.
    #[default]
    Browse,
    /// The form over one row.
    Form(Form),
    /// The repo picker over the form's project (`Ctrl+R`).
    Picker {
        /// The form the picked name goes into.
        form: Form,
        /// Index into the project's `repos`.
        cursor: usize,
    },
    /// `x` asked; `y` detaches (the kinds editor's modal question).
    ConfirmDetach {
        /// The row's key.
        key: SkillBindingKey,
        /// The row's `updated_at`: the detach's compare-and-set token.
        token: DateTime<Utc>,
        /// What the notice calls the row.
        target: String,
    },
}

/// One line of the pane, by index into the snapshot (the `settings/kinds.rs` precedent): rebuilt
/// from every reply, and an index that outlives its snapshot is caught by the clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ARow {
    /// The global attachment.
    Global,
    /// One scope project's attachment.
    Project {
        /// Index into `snapshot.projects`.
        p: usize,
    },
    /// One phase's attachment.
    Phase {
        /// Index into `snapshot.projects`.
        p: usize,
        /// Index into that project's `graphs`.
        g: usize,
        /// Index into that graph's phases.
        i: usize,
    },
}

/// The open form: which row, the four text inputs and the activation, and the token it opened on.
#[derive(Debug)]
struct Form {
    /// The attachment's key.
    key: SkillBindingKey,
    /// What the title and the notices call the row.
    target: String,
    /// The row's `updated_at`; `None` when no row sat there (an attach).
    token: Option<DateTime<Utc>>,
    /// `Space` cycles it.
    activation: Activation,
    /// `latest` (or empty) or a version number.
    pin: TextField,
    /// A whole number, 0 or more.
    position: TextField,
    /// A comma list of language names (D93's splitter).
    languages: TextField,
    /// A comma list of globs, braces and classes kept whole (D93).
    globs: TextField,
    /// Which input the keys go to.
    focus: FormField,
}

/// The form's inputs, in tab order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum FormField {
    /// `Space` cycles `always → glob → off`.
    #[default]
    Activation,
    /// The pin.
    Pin,
    /// The position.
    Position,
    /// The languages.
    Languages,
    /// The globs.
    Globs,
}

impl FormField {
    /// Tab order.
    const ALL: [Self; 5] = [
        Self::Activation,
        Self::Pin,
        Self::Position,
        Self::Languages,
        Self::Globs,
    ];

    /// The next (or previous) field, wrapping.
    fn step(self, forward: bool) -> Self {
        let at = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }]
    }
}

/// What a key did, for the view to act on: the pane never sends and never writes the notice.
#[derive(Debug)]
pub(super) enum AttachOutcome {
    /// Used; nothing for the view to do.
    Consumed,
    /// Not the pane's: the view passes it on.
    Pass,
    /// `Esc` or `a` in Browse: the view drops the pane.
    Close,
    /// A line for the view's notice.
    Notice(Notice),
    /// A write for the view to send, unless one is already in flight.
    Save {
        /// The request (boxed: it and `sent` are most of the enum's size).
        request: Box<StoreRequest>,
        /// What it carries, for the landing (D98).
        sent: Box<Sent>,
    },
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// A `CONTROL` chord (`SHIFT` allowed).
fn chord(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
}

impl AttachPane {
    /// The pane over `skill`, cursor on the `global` row.
    pub(super) fn new(skill: SkillId) -> Self {
        Self {
            skill,
            cursor: 0,
            mode: AttachMode::Browse,
        }
    }

    /// Whose attachments the pane shows.
    pub(super) fn skill(&self) -> SkillId {
        self.skill
    }

    /// Whether the form, the picker or the detach question is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        !matches!(self.mode, AttachMode::Browse)
    }

    /// The write the form sent landed: back to the rows.
    pub(super) fn on_landed(&mut self) {
        self.mode = AttachMode::Browse;
    }

    /// Whether the form is open (not the picker over it): what `Esc` would close.
    pub(super) fn in_form(&self) -> bool {
        matches!(self.mode, AttachMode::Form(_))
    }

    /// The write the form sent missed its token: the form keeps its text and takes the row's
    /// token as it is now, so the next `Ctrl+S` is a deliberate overwrite. With no form open (a
    /// detach went stale) there is nothing to keep.
    pub(super) fn on_stale(&mut self, snapshot: &SkillsSnapshot) {
        if let AttachMode::Form(form) | AttachMode::Picker { form, .. } = &mut self.mode {
            form.token = snapshot.binding(form.key).map(|row| row.updated_at);
        }
    }

    /// Keeps the cursor on a row after the snapshot changed.
    pub(super) fn clamp(&mut self, snapshot: &SkillsSnapshot) {
        self.cursor = self.cursor.min(rows(snapshot).len().saturating_sub(1));
    }

    /// One key (the table of blueprint §6.6).
    pub(super) fn on_key(
        &mut self,
        key: KeyEvent,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) -> AttachOutcome {
        match core::mem::take(&mut self.mode) {
            AttachMode::Browse => self.on_browse_key(key, snapshot, ctx),
            AttachMode::Form(form) => self.on_form_key(form, key, snapshot, ctx),
            AttachMode::Picker { form, cursor } => self.on_picker_key(form, cursor, key, snapshot),
            AttachMode::ConfirmDetach {
                key: row,
                token,
                target,
            } => self.on_confirm_key(row, token, target, key, ctx),
        }
    }

    /// Browse. `h`/`l` and every other key pass, so the view switch still works and the pane
    /// stays open behind it.
    fn on_browse_key(
        &mut self,
        key: KeyEvent,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) -> AttachOutcome {
        if !plain(&key) {
            return AttachOutcome::Pass;
        }
        let rows = rows(snapshot);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(rows.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(row) = rows.get(self.cursor) {
                    self.mode = AttachMode::Form(self.open_form(*row, snapshot, ctx));
                }
            }
            KeyCode::Char('x') => {
                let Some(row) = rows.get(self.cursor) else {
                    return AttachOutcome::Consumed;
                };
                let key = self.key_of(*row, snapshot);
                let Some(stored) = snapshot.binding(key) else {
                    return AttachOutcome::Notice(Notice::Info(NOTHING_ATTACHED.to_owned()));
                };
                let target = target(*row, snapshot, ctx);
                let name = skill_name(snapshot, self.skill);
                let question =
                    format!("detach `{name}` from {target}? y detaches, any other key keeps it");
                self.mode = AttachMode::ConfirmDetach {
                    key,
                    token: stored.updated_at,
                    target,
                };
                return AttachOutcome::Notice(Notice::Info(question));
            }
            KeyCode::Char('r') => ctx.request(StoreRequest::Skills(ctx.scope.clone())),
            KeyCode::Esc | KeyCode::Char('a') => return AttachOutcome::Close,
            _ => return AttachOutcome::Pass,
        }
        AttachOutcome::Consumed
    }

    /// `Enter` on a row: the stored attachment, or the defaults for a new one (`always`, `latest`,
    /// 0, nothing typed). The globs field holds the stored globs minus the stored languages'
    /// expansion (D102), so a re-save is the same attachment and the field shows what was typed.
    fn open_form(&self, row: ARow, snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> Form {
        let key = self.key_of(row, snapshot);
        let target = target(row, snapshot, ctx);
        let Some(stored) = snapshot.binding(key) else {
            return Form {
                key,
                target,
                token: None,
                activation: Activation::Always,
                pin: TextField::with_text("latest"),
                position: TextField::with_text("0"),
                languages: TextField::new(),
                globs: TextField::new(),
                focus: FormField::Activation,
            };
        };
        let expanded = skill_language::expand(&stored.languages).unwrap_or_default();
        let typed: Vec<&str> = stored
            .globs
            .iter()
            .filter(|glob| !expanded.contains(glob))
            .map(String::as_str)
            .collect();
        Form {
            key,
            target,
            token: Some(stored.updated_at),
            activation: stored.activation,
            pin: TextField::with_text(
                &stored
                    .pinned_version
                    .map_or_else(|| "latest".to_owned(), |pin| pin.to_string()),
            ),
            position: TextField::with_text(&stored.position.to_string()),
            languages: TextField::with_text(&stored.languages.join(", ")),
            globs: TextField::with_text(&typed.join(", ")),
            focus: FormField::Activation,
        }
    }

    /// The form (D103: `Enter` and `Ctrl+S` both save, `Esc` closes without asking).
    fn on_form_key(
        &mut self,
        mut form: Form,
        key: KeyEvent,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) -> AttachOutcome {
        if chord(&key) {
            return match key.code {
                KeyCode::Char('r' | 'R') => self.open_picker(form, snapshot, ctx),
                KeyCode::Char('s' | 'S') => self.save(form, ctx),
                _ => {
                    self.mode = AttachMode::Form(form);
                    AttachOutcome::Pass
                }
            };
        }
        match key.code {
            KeyCode::Esc => return AttachOutcome::Consumed,
            KeyCode::Enter => return self.save(form, ctx),
            KeyCode::Tab | KeyCode::Down => form.focus = form.focus.step(true),
            KeyCode::BackTab | KeyCode::Up => form.focus = form.focus.step(false),
            KeyCode::Char(' ') if form.focus == FormField::Activation => {
                form.activation = match form.activation {
                    Activation::Always => Activation::Glob,
                    Activation::Glob => Activation::Off,
                    Activation::Off => Activation::Always,
                };
            }
            // The activation is not typed: every other key on it is swallowed, so a stray letter
            // neither lands in a field nor reaches the shell.
            _ if form.focus == FormField::Activation => {}
            _ => {
                let field = match form.focus {
                    FormField::Pin => &mut form.pin,
                    FormField::Position => &mut form.position,
                    FormField::Languages => &mut form.languages,
                    FormField::Globs | FormField::Activation => &mut form.globs,
                };
                // `Enter`, `Esc`, `Tab` and the arrows were taken above, so what reaches the field
                // is text or a cursor key; anything it passes (`F(n)`, `Insert`) is swallowed with
                // the form open, as the activation's keys are.
                field.on_key(key);
            }
        }
        self.mode = AttachMode::Form(form);
        AttachOutcome::Consumed
    }

    /// `Ctrl+R`: the project's repo names, or why there are none to pick.
    fn open_picker(
        &mut self,
        form: Form,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) -> AttachOutcome {
        let refusal = match form.key.project {
            None => Some(GLOBAL_HAS_NO_REPOS.to_owned()),
            Some(project) => snapshot
                .project(project)
                .is_none_or(|entry| entry.repos.is_empty())
                .then(|| format!("project `{}` has no repos", slug(ctx, project))),
        };
        if let Some(refusal) = refusal {
            self.mode = AttachMode::Form(form);
            return AttachOutcome::Notice(Notice::Error(refusal));
        }
        self.mode = AttachMode::Picker { form, cursor: 0 };
        AttachOutcome::Consumed
    }

    /// The picker. `Enter` types `<repo>:` into the globs field at its cursor, one char key at a
    /// time (D101: `TextField` has no insert API, and a key is what inserts at the cursor).
    fn on_picker_key(
        &mut self,
        mut form: Form,
        mut cursor: usize,
        key: KeyEvent,
        snapshot: &SkillsSnapshot,
    ) -> AttachOutcome {
        if chord(&key) {
            self.mode = AttachMode::Picker { form, cursor };
            return AttachOutcome::Pass;
        }
        let repos: &[String] = form
            .key
            .project
            .and_then(|project| snapshot.project(project))
            .map_or(&[], |entry| entry.repos.as_slice());
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                cursor = (cursor + 1).min(repos.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => cursor = cursor.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(repo) = repos.get(cursor) {
                    for c in format!("{repo}:").chars() {
                        form.globs.on_key(KeyEvent::from(KeyCode::Char(c)));
                    }
                    form.focus = FormField::Globs;
                }
                self.mode = AttachMode::Form(form);
                return AttachOutcome::Consumed;
            }
            KeyCode::Esc => {
                self.mode = AttachMode::Form(form);
                return AttachOutcome::Consumed;
            }
            _ => {}
        }
        self.mode = AttachMode::Picker { form, cursor };
        AttachOutcome::Consumed
    }

    /// The detach question: modal, `y` or keep (the kinds editor's shape, a `CONTROL` chord the
    /// carve-out so `ctrl-c` still quits).
    fn on_confirm_key(
        &mut self,
        row: SkillBindingKey,
        token: DateTime<Utc>,
        target: String,
        key: KeyEvent,
        ctx: &Ctx<'_>,
    ) -> AttachOutcome {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.mode = AttachMode::ConfirmDetach {
                key: row,
                token,
                target,
            };
            return AttachOutcome::Pass;
        }
        if key.code != KeyCode::Char('y') {
            return AttachOutcome::Notice(Notice::Info(KEPT.to_owned()));
        }
        AttachOutcome::Save {
            request: Box::new(StoreRequest::SetSkillBinding {
                scope: ctx.scope.clone(),
                key: row,
                expected: Some(token),
                change: BindingChange::Detach,
            }),
            sent: Box::new(Sent::Binding {
                key: row,
                token: Some(token),
                change: BindingChange::Detach,
                target,
            }),
        }
    }

    /// `Ctrl+S` / `Enter` on the form: the pin and the position are read here, the lists split by
    /// D93's rule and run through [`canonical_globs`]; the first refusal is the notice and the form
    /// stays. The store re-checks everything (D78, D79); a refusal of its own comes back as
    /// `Failed` and leaves the form as it is. The form stays open until the reply lands.
    fn save(&mut self, form: Form, ctx: &Ctx<'_>) -> AttachOutcome {
        let built = build(&form);
        let key = form.key;
        let (token, target) = (form.token, form.target.clone());
        self.mode = AttachMode::Form(form);
        let attachment = match built {
            Ok(attachment) => attachment,
            Err(why) => return AttachOutcome::Notice(Notice::Error(why)),
        };
        let change = BindingChange::Attach(attachment);
        AttachOutcome::Save {
            request: Box::new(StoreRequest::SetSkillBinding {
                scope: ctx.scope.clone(),
                key,
                expected: token,
                change: change.clone(),
            }),
            sent: Box::new(Sent::Binding {
                key,
                token,
                change,
                target,
            }),
        }
    }

    /// The key a row writes under.
    fn key_of(&self, row: ARow, snapshot: &SkillsSnapshot) -> SkillBindingKey {
        let (project, phase) = match row {
            ARow::Global => (None, None),
            ARow::Project { p } => (snapshot.projects.get(p).map(|entry| entry.project), None),
            ARow::Phase { p, g, i } => {
                let entry = snapshot.projects.get(p);
                (
                    entry.map(|entry| entry.project),
                    entry.and_then(|entry| phase_id(entry, g, i)),
                )
            }
        };
        SkillBindingKey {
            skill: self.skill,
            project,
            phase,
        }
    }

    // --- frames --------------------------------------------------------------------------------

    /// Draws the pane into `area` and returns its hint row.
    pub(super) fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) -> &'static str {
        let (rows_area, panel) = match &self.mode {
            AttachMode::Form(_) | AttachMode::Picker { .. } => {
                let [rows_area, panel] =
                    Layout::vertical([Constraint::Min(3), Constraint::Length(FORM_HEIGHT)])
                        .areas(area);
                (rows_area, Some(panel))
            }
            AttachMode::Browse | AttachMode::ConfirmDetach { .. } => (area, None),
        };
        self.render_rows(frame, rows_area, snapshot, ctx);
        match (&self.mode, panel) {
            (AttachMode::Form(form), Some(panel)) => {
                render_form(frame, panel, form, snapshot, self.skill, ctx);
                FORM_HINT
            }
            (AttachMode::Picker { form, cursor }, Some(panel)) => {
                render_picker(frame, panel, form, *cursor, snapshot, ctx);
                PICKER_HINT
            }
            (AttachMode::ConfirmDetach { .. }, _) => CONFIRM_HINT,
            _ => BROWSE_HINT,
        }
    }

    /// The rows, the cursor's highlighted and kept in view.
    fn render_rows(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &SkillsSnapshot,
        ctx: &Ctx<'_>,
    ) {
        let theme = ctx.theme;
        let block = Block::new().borders(Borders::ALL).title(format!(
            " attachments \u{b7} {} ",
            skill_name(snapshot, self.skill)
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let winners = self.winners(snapshot);
        let lines: Vec<Line<'static>> = rows(snapshot)
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let star = if winners.contains(&row) { '*' } else { ' ' };
                let label = cut(&label(row, snapshot, ctx), LABEL_WIDTH);
                let summary = self.summary(row, snapshot);
                let style = if index == self.cursor {
                    theme.selected
                } else if matches!(row, ARow::Phase { .. }) {
                    theme.base
                } else {
                    theme.title
                };
                Line::styled(format!("{star} {label:<LABEL_WIDTH$} {summary}"), style)
            })
            .collect();
        let offset = self
            .cursor
            .saturating_sub(usize::from(inner.height).saturating_sub(1));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            inner,
        );
    }

    /// A row's summary: `—` with no attachment, else `<activation> · <v N | latest> · pos <p>`,
    /// then `<n> globs` (with ` ?` when a qualifier names a repo the project no longer has, D79's
    /// rename case) and, on a `glob` row, that it fires from milestone 5 (R-28).
    fn summary(&self, row: ARow, snapshot: &SkillsSnapshot) -> String {
        let Some(stored) = snapshot.binding(self.key_of(row, snapshot)) else {
            return NO_ROW.to_owned();
        };
        let mut parts = vec![
            stored.activation.as_str().to_owned(),
            stored
                .pinned_version
                .map_or_else(|| "latest".to_owned(), |pin| format!("v{pin}")),
            format!("pos {}", stored.position),
        ];
        if !stored.globs.is_empty() {
            let repos = stored
                .project_id
                .and_then(|project| snapshot.project(project))
                .map_or(&[][..], |entry| entry.repos.as_slice());
            let stale =
                stored.project_id.is_some() && first_unknown_repo(&stored.globs, repos).is_some();
            let mark = if stale { " ?" } else { "" };
            parts.push(format!("{} globs{mark}", stored.globs.len()));
        }
        if stored.activation == Activation::Glob {
            parts.push(FIRES_LATER.to_owned());
        }
        parts.join(" \u{b7} ")
    }

    /// D104: the rows that win somewhere. For each scope project and each phase of its
    /// non-override graphs (or no phase, for a project with none), [`resolve`] over this skill's
    /// global row and the project's rows that apply there; the one candidate's level names the row
    /// that wins, which is the row the run would record. An `off` winner counts: it is what hides
    /// the broader attachment.
    fn winners(&self, snapshot: &SkillsSnapshot) -> Vec<ARow> {
        let Some(entry) = snapshot.entry(self.skill) else {
            return Vec::new();
        };
        let name = entry.skill.name.clone();
        let mine = |row: &&SkillBinding| row.skill_id == self.skill;
        let mut winners = Vec::new();
        for (p, project) in snapshot.projects.iter().enumerate() {
            let mut phases: Vec<Option<(usize, usize, PhaseId)>> = project
                .graphs
                .iter()
                .enumerate()
                .flat_map(|(g, (_, phases))| {
                    phases
                        .iter()
                        .enumerate()
                        .map(move |(i, phase)| Some((g, i, phase.id)))
                })
                .collect();
            if phases.is_empty() {
                phases.push(None);
            }
            for at in phases {
                let phase = at.map(|(_, _, id)| id);
                let applies: Vec<(SkillBinding, String)> = snapshot
                    .global
                    .iter()
                    .filter(mine)
                    .chain(
                        project
                            .bindings
                            .iter()
                            .filter(mine)
                            .filter(|row| row.phase_id.is_none() || row.phase_id == phase),
                    )
                    .map(|row| (row.clone(), name.clone()))
                    .collect();
                let Some(winner) = resolve(applies, &entry.versions).into_iter().next() else {
                    continue;
                };
                let row = match (winner.level, at) {
                    (SkillLevel::Global, _) => ARow::Global,
                    (SkillLevel::Project, _) => ARow::Project { p },
                    (SkillLevel::Phase, Some((g, i, _))) => ARow::Phase { p, g, i },
                    (SkillLevel::Phase, None) => continue,
                };
                if !winners.contains(&row) {
                    winners.push(row);
                }
            }
        }
        winners
    }
}

/// The pane's rows (D83): `global`; then per scope project its row and, under it, each phase of
/// its non-override graphs, graphs by name and phases by position (the snapshot's order).
fn rows(snapshot: &SkillsSnapshot) -> Vec<ARow> {
    let mut rows = vec![ARow::Global];
    for (p, project) in snapshot.projects.iter().enumerate() {
        rows.push(ARow::Project { p });
        for (g, (_, phases)) in project.graphs.iter().enumerate() {
            rows.extend((0..phases.len()).map(|i| ARow::Phase { p, g, i }));
        }
    }
    rows
}

/// `projects[p].graphs[g]`'s phase `i`, by id.
fn phase_id(project: &ProjectSkills, g: usize, i: usize) -> Option<PhaseId> {
    project
        .graphs
        .get(g)
        .and_then(|(_, phases)| phases.get(i))
        .map(|phase| phase.id)
}

/// A row's label: `global`, the project's slug, or `  <graph> › <phase>` under its project.
fn label(row: ARow, snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> String {
    match row {
        ARow::Global => "global".to_owned(),
        ARow::Project { p } => snapshot
            .projects
            .get(p)
            .map_or_else(String::new, |entry| slug(ctx, entry.project)),
        ARow::Phase { p, g, i } => format!("  {}", graph_phase(snapshot, p, g, i)),
    }
}

/// What the form's title and the notices call a row: `global`, the slug, or
/// `<slug> <graph> › <phase>`.
fn target(row: ARow, snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> String {
    match row {
        ARow::Global | ARow::Project { .. } => label(row, snapshot, ctx),
        ARow::Phase { p, g, i } => {
            let project = snapshot
                .projects
                .get(p)
                .map_or_else(String::new, |entry| slug(ctx, entry.project));
            format!("{project} {}", graph_phase(snapshot, p, g, i))
        }
    }
}

/// `<graph> › <phase>`.
fn graph_phase(snapshot: &SkillsSnapshot, p: usize, g: usize, i: usize) -> String {
    snapshot
        .projects
        .get(p)
        .and_then(|entry| entry.graphs.get(g))
        .and_then(|(graph, phases)| {
            phases
                .get(i)
                .map(|phase| format!("{} \u{203a} {}", graph.name, phase.name))
        })
        .unwrap_or_default()
}

/// A skill's name from the snapshot, else its id (a skill never leaves the library, but a reply
/// can race the pane).
fn skill_name(snapshot: &SkillsSnapshot, skill: SkillId) -> String {
    snapshot
        .entry(skill)
        .map_or_else(|| skill.to_string(), |entry| entry.skill.name.clone())
}

/// A project's slug from the scope's projects, else the id's first eight chars (the Templates
/// view's rule).
pub(super) fn slug(ctx: &Ctx<'_>, project: ProjectId) -> String {
    ctx.projects
        .iter()
        .find(|entry| entry.project_id == project)
        .map_or_else(
            || project.to_string().chars().take(8).collect(),
            |entry| entry.slug.clone(),
        )
}

/// `text` cut to `width` chars, the last one `…` when it was longer.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() > width {
        let kept: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{kept}\u{2026}")
    } else {
        text.to_owned()
    }
}

/// The first stored or typed glob whose qualifier names no repo in `repos`, as
/// `(glob, repo)`. Entries that do not parse are skipped: [`canonical_globs`] reports those.
fn first_unknown_repo(globs: &[String], repos: &[String]) -> Option<(String, String)> {
    globs.iter().find_map(|glob| {
        let repo = SkillGlob::parse(glob).ok()?.repo?;
        (!repos.contains(&repo)).then(|| (glob.clone(), repo))
    })
}

/// The first qualified glob, as `(glob, repo)`: what a global row may not hold.
fn first_qualified(globs: &[String]) -> Option<(String, String)> {
    globs.iter().find_map(|glob| {
        let repo = SkillGlob::parse(glob).ok()?.repo?;
        Some((glob.clone(), repo))
    })
}

/// The form's attachment, or the first sentence refusing it.
fn build(form: &Form) -> Result<Attachment, String> {
    let pin = form.pin.text().unwrap_or_default().trim();
    let pinned_version = match pin {
        "" | "latest" => None,
        text => match text.parse::<i32>() {
            Ok(version) if version >= 1 => Some(version),
            _ => return Err(BAD_PIN.to_owned()),
        },
    };
    let position = match form.position.text().unwrap_or_default().trim() {
        "" => 0,
        text => match text.parse::<i32>() {
            Ok(position) if position >= 0 => position,
            _ => return Err(BAD_POSITION.to_owned()),
        },
    };
    let globs = split_list(form.globs.text().unwrap_or_default());
    let languages = split_list(form.languages.text().unwrap_or_default());
    canonical_globs(&globs, &languages).map_err(|err| err.to_string())?;
    Ok(Attachment {
        pinned_version,
        position,
        activation: form.activation,
        globs,
        languages,
    })
}

/// The form panel: five inputs, the `effective:` line, and the help line — or, in its place, the
/// sentence the store would refuse a qualifier with.
fn render_form(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &Form,
    snapshot: &SkillsSnapshot,
    skill: SkillId,
    ctx: &Ctx<'_>,
) {
    let theme = ctx.theme;
    let verb = if form.token.is_some() {
        "edit"
    } else {
        "attach"
    };
    let block = Block::new().borders(Borders::ALL).title(format!(
        " {verb} {} \u{b7} {} ",
        skill_name(snapshot, skill),
        form.target
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner
        .width
        .saturating_sub(u16::try_from(FORM_LABEL).unwrap_or(0));
    let field = |name: &'static str, input: &TextField, which: FormField| {
        let mut spans = vec![Span::styled(format!("{name:<FORM_LABEL$}"), theme.base)];
        spans.extend(input.line(width, form.focus == which, theme).spans);
        Line::from(spans)
    };
    let activation_style = if form.focus == FormField::Activation {
        theme.selected
    } else {
        theme.base
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<FORM_LABEL$}", "activation"), theme.base),
            Span::styled(form.activation.as_str(), activation_style),
            Span::styled("  (Space cycles)", theme.dim),
        ]),
        field("pin", &form.pin, FormField::Pin),
        field("position", &form.position, FormField::Position),
        field("languages", &form.languages, FormField::Languages),
        field("globs", &form.globs, FormField::Globs),
    ];
    let typed = split_list(form.globs.text().unwrap_or_default());
    let languages = split_list(form.languages.text().unwrap_or_default());
    let label = Span::styled(format!("{:<FORM_LABEL$}", "effective:"), theme.base);
    let effective = canonical_globs(&typed, &languages);
    lines.push(match &effective {
        Ok(globs) if globs.is_empty() => Line::from(vec![label, Span::styled("none", theme.dim)]),
        Ok(globs) => Line::from(vec![label, Span::styled(globs.join(", "), theme.base)]),
        Err(err) => Line::from(vec![label, Span::styled(err.to_string(), theme.error)]),
    });
    lines.push(
        match repo_refusal(form, effective.as_deref().unwrap_or(&[]), snapshot, ctx) {
            Some(sentence) => Line::styled(sentence, theme.error),
            None => Line::styled(GLOB_HELP, theme.dim),
        },
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The store's sentence for the first qualifier it would refuse (D78, D79): any qualifier on the
/// global row, one naming a repo the project does not have on a project or phase row.
fn repo_refusal(
    form: &Form,
    globs: &[String],
    snapshot: &SkillsSnapshot,
    ctx: &Ctx<'_>,
) -> Option<String> {
    match form.key.project {
        None => first_qualified(globs).map(|(glob, repo)| global_glob_names_a_repo(&glob, &repo)),
        Some(project) => {
            let repos = snapshot
                .project(project)
                .map_or(&[][..], |entry| entry.repos.as_slice());
            first_unknown_repo(globs, repos)
                .map(|(glob, repo)| glob_names_unknown_repo(&glob, &repo, &slug(ctx, project)))
        }
    }
}

/// The picker panel: the form's project's repos, one per line.
fn render_picker(
    frame: &mut Frame<'_>,
    area: Rect,
    form: &Form,
    cursor: usize,
    snapshot: &SkillsSnapshot,
    ctx: &Ctx<'_>,
) {
    let theme: &Theme = ctx.theme;
    let Some(project) = form.key.project else {
        return;
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" repos of {} ", slug(ctx, project)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let repos = snapshot
        .project(project)
        .map_or(&[][..], |entry| entry.repos.as_slice());
    let lines: Vec<Line<'static>> = repos
        .iter()
        .enumerate()
        .map(|(index, repo)| {
            let style = if index == cursor {
                theme.selected
            } else {
                theme.base
            };
            Line::styled(repo.clone(), style)
        })
        .collect();
    let offset = cursor.saturating_sub(usize::from(inner.height).saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        inner,
    );
}
