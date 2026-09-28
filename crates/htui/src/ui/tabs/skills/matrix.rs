//! The attachments matrix of the Skills tab (MOD-9 milestone 3, T4; plan D78's UI half, D82, D83;
//! blueprint §5, D92, D100, D103, H-33..H-37).
//!
//! One **row** per level — the global row above the projects and their phases — and one **column**
//! per skill, packed with `format!` into a bordered `Block` rather than a
//! `ratatui::widgets::Table`. D82 as amended: the three `Table` users all have a *row* cursor and
//! none a selected *cell*, and this view selects a cell; the right-hand `pane()` carries the
//! selected cell's detail and the activation form. The layout is the Templates list's own pattern
//! (`templates.rs:876-949`).
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle, no
//! `UserId` and no `BoxId`, renders from the last [`SkillsSnapshot`] and never patches a row into
//! it. The write is refused **here**, before it is sent, in the writer's own sentences, so the two
//! halves of the milestone say the same thing about the same bytes (D100).
//!
//! The level list is not derivable from the skills read: `skill_binding` names a phase, but a
//! project whose phases carry no attachment would have no phase row and could therefore never
//! grow its first one. So the matrix asks the **catalogue** for the scope's graphs and phases when
//! it opens, and the **hierarchy** for the project's repos when the repo picker is opened. Both
//! are existing `StoreRequest` variants, so no query, table or reply variant is added; each is
//! asked for once, on demand, and never one request per project (D92, the `Catalogue(Scope)` doc).

use core::cell::Cell;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::language::{
    LANGUAGE_GLOBS, effective_globs as expand_languages, languages as LANGUAGE_NAMES,
};
use htui_core::model::{
    Activation, NewSkillBinding, PhaseId, ProjectId, SkillAttachmentRow, SkillBindingId, SkillId,
    SkillVersion,
};
use htui_core::store::traits::{skill_binding_refusal, skill_pin_refusal};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::catalogue::CatalogueSnapshot;
use crate::hierarchy::HierarchySnapshot;
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillSummary, SkillsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};

/// The `StoreRequest::SetSkillBinding` name, what a `Busy` holds while it is in flight. The slice
/// index, not a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const SET_NAME: &str = REQUEST_NAMES[2];
/// The `StoreRequest::RemoveSkillBinding` name, likewise `REQUEST_NAMES[3]`.
const UNSET_NAME: &str = REQUEST_NAMES[3];

/// The matrix's list column, borders included: two spaces, [`LEVEL_WIDTH`], and as many
/// [`CELL_WIDTH`] skill columns as [`MAX_COLUMNS`] says fit.
const LIST_WIDTH: u16 = 46;
/// The level column, in chars: `global`, a project slug, and `graph/phase`.
const LEVEL_WIDTH: usize = 18;
/// One skill's cell, in chars: `v12`, `v12g`, `?` and `·` all fit with room to spare.
const CELL_WIDTH: usize = 13;
/// How many skill columns the list draws at once. A library wider than this scrolls sideways
/// rather than pushing the pane off the frame, so the selected cell is always on screen.
const MAX_COLUMNS: usize = (LIST_WIDTH as usize).saturating_sub(2 + LEVEL_WIDTH) / CELL_WIDTH;
/// Rows each of the two multi-line fields draws, so a three-glob list is visible without
/// scrolling.
const FIELD_ROWS: u16 = 3;
/// How many rows the notice may wrap to before it is cut.
const NOTICE_LINES: usize = 3;
/// The field label column, in chars; `activation:` and `attachment:` are the longest.
const LABEL_WIDTH: usize = 11;

/// The global row's label. Every project's row carries the project's `slug` and every phase row
/// `graph/phase`, so the label column is the level and the cells beside it are the skills.
const GLOBAL_LABEL: &str = "global";
/// The header over the level column.
const LEVEL_HEADER: &str = "level";
/// An empty cell: attached nowhere at this level.
const NO_CELL: &str = "\u{b7}";
/// An activation with nothing in force, or a pin the skill has no version for.
const UNKNOWN: &str = "?";

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";
/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";
/// A key pressed with nothing under the cursor.
const SELECT_A_CELL: &str = "select a cell";
/// A write went out.
const WRITING: &str = "writing\u{2026}";
/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// A repo qualifier on a **global** row, which applies to every project and so can name none.
const NO_REPO_ON_GLOBAL: &str =
    "a global attachment cannot name a repo; use a project or phase row";

/// The hint row while the matrix is browsed. It has to fit the 78 columns the content area is at
/// the harness's 80-column frame, which is why the cell verbs share one clause.
const BROWSE_HINT: &str = "j/k level  \u{2190}/\u{2192} skill  g/G ends  a/e/x cell  A/p cell  \
                           r reload  m/Esc close";
/// The hint row while the activation form is open. `Ctrl+S` rather than `Enter`, because `Enter`
/// is what splits a line in the two multi-line fields — the `TextArea` rule (MOD-7 D44), and the
/// same key the Templates editor saves on.
const FORM_HINT: &str = "Tab next field  Ctrl+S save  A activation  p pin  R repo  Esc cancel";
/// The hint row while the repo picker is open.
const PICKER_HINT: &str = "j/k repo  Enter pick  Esc cancel";

/// The attachments matrix (MOD-9 milestone 3, D82): one row per level, one column per skill, the
/// activation form and the repo picker. Holds no store handle, no `UserId` and no `BoxId`
/// (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct MatrixView {
    /// Whether the matrix is showing. The tab asks, and `Esc` or `m` clears it; nothing else
    /// opens it, because the switch line and the strip text are the shell's and byte-identical
    /// (H-15, H-28).
    open: bool,
    /// The last read, or `None` before the first reply. The same [`SkillsSnapshot`] the library
    /// view draws, kept separately so neither patches a row into the other's copy.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale matrix.
    unavailable: Option<String>,
    /// The scope's graphs and phases, read when the matrix opened: the level list is not
    /// derivable from the skills read, and a project with no phase attachment would otherwise
    /// have no phase row to attach one to.
    catalogue: Option<CatalogueSnapshot>,
    /// The scope's repos, read when the repo picker opened. `None` until the reply lands, and
    /// again on every later open, so a project's repos are asked for once per picker.
    repos: Option<Box<HierarchySnapshot>>,
    /// The highlighted **level**, an index into [`levels`]: the global row first, then each scope
    /// project, then that project's phases.
    level: usize,
    /// The first skill column drawn, so a library wider than [`MAX_COLUMNS`] scrolls sideways and
    /// the selected cell is never off screen.
    first_column: usize,
    /// The highlighted **skill**, an index into the snapshot's `skills`.
    skill: usize,
    /// The activation form, open over the selected cell; `None` while the matrix is browsed.
    form: Option<Form>,
    /// The repo picker, open over the form's qualifier.
    picker: Option<Picker>,
    /// The pane's first drawn row.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](MatrixView::scroll) clamps against. A
    /// `Cell` because the row count is known only in `render(&self)`.
    pane_rows: Cell<usize>,
    /// The write in flight, and the cell it was sent for, so the reply can name it.
    busy: Option<Busy>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
}

/// The three axes of the matrix, in the order the rows are drawn: the global row, then each scope
/// project, then each of that project's phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// `project_id IS NULL`: every project.
    Global,
    /// One project's row, and, under it, its phases.
    Project(ProjectId),
    /// One phase's row.
    Phase(ProjectId, PhaseId),
}

/// One drawn row: the level and the text its label column carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Level {
    /// The level, and the `(project_id, phase_id)` the write carries.
    axis: Axis,
    /// The label: `global`, a project `slug`, or `graph/phase`.
    label: String,
}

/// The activation form, open over one `(skill, level)` cell.
#[derive(Debug)]
struct Form {
    /// The cell being edited.
    skill_id: SkillId,
    /// Its library key, so the pane's title and the reply's notice name it without a second read.
    name: String,
    /// Its level, so the write carries the right `(project_id, phase_id)`.
    level: Axis,
    /// The winning attachment's `updated_at`, the CAS token (`None`: no row yet).
    token: Option<DateTime<Utc>>,
    /// `always`, `glob` or `off`.
    activation: Activation,
    /// Follows the latest, or pins a version the skill has.
    pin: Pin,
    /// The globs, one per line — what the store receives, which is why a reopen shows the
    /// **effective** list and not a separately-typed one: the language expansion is already in
    /// it, and [`expand_languages`] de-duplicates, so saving again is a no-op. The one thing
    /// that is **not** in the field is the `<repo>:` prefix [`Form::qualifier`] puts back on
    /// every line, so a reopen qualifies a row exactly once.
    globs: TextArea,
    /// The languages, one per line, each of which [`LANGUAGE_GLOBS`] expands at save.
    languages: TextArea,
    /// `skill_binding.position`, typed.
    position: TextField,
    /// The `<repo>:` the picker wrote, prepended to every **typed** glob; `None` for a bare glob.
    /// Read back off the stored globs when the form opens, and the same prefix is then stripped
    /// off the field by [`unqualified`], so reopening a qualified attachment and saving it again
    /// leaves the qualifier on and does not double it.
    qualifier: Option<String>,
    /// Which of the three fields `Tab` is on.
    field: usize,
}

/// Whether an attachment follows the latest version or pins one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin {
    /// `pinned_version = NULL`: the latest version is in force.
    Latest,
    /// `pinned_version = Some(n)`, cleared back to [`Pin::Latest`] by the last `p`.
    Version(i32),
}

/// The repo picker: which repo the form's qualifier names. Row `0` is "every repo".
#[derive(Debug)]
struct Picker {
    /// The highlighted row, into [`picker_names`](MatrixView::picker_names).
    cursor: usize,
}

/// The write in flight, and the cell it was sent for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Busy {
    /// `StoreRequest::name`, so the reply knows which notice to give.
    request: &'static str,
    /// The cell's skill name.
    name: String,
    /// The cell's level label, which is what the user was looking at.
    level: String,
}

/// `Notice` is `library.rs`'s own type, copied verbatim: two variants, and no third state is
/// needed here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// An outcome that is not a failure.
    Info(String),
    /// A refusal, from this view or from the store.
    Error(String),
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// One line of a sub-list under a pane section header, indented two columns past the header.
fn sub(text: &str, theme: &Theme) -> Line<'static> {
    Line::styled(format!("  {text}"), theme.base)
}

/// H-37: the first language name the form holds that [`LANGUAGE_GLOBS`] does not, in the
/// blueprint's sentence, or `None` when every name is in the map.
///
/// A name the map lacks contributes nothing and is **not** a refusal (D83) — the save still goes
/// out with the name in `languages` — so this is `Info`, and it is the only thing that tells a
/// maintainer who mistyped `cobol` that nothing was expanded for it.
fn unknown_language(form: &Form) -> Option<String> {
    form.language_names().into_iter().find_map(|name| {
        let known = LANGUAGE_GLOBS
            .iter()
            .any(|(candidate, _)| *candidate == name);
        (!known).then(|| format!("no language named `{name}` in the map"))
    })
}

/// A stored glob with the form's own `<repo>:` prefix taken off, which is what the globs field
/// holds: [`Form::typed_globs`] puts the qualifier back on every line, so a field seeded with
/// the stored strings would save `repo:repo:`. A glob that names no repo, or names a **different**
/// one, is left exactly as it is — the qualifier is one repo, and anything else in a stored glob
/// is the row's own text.
fn unqualified(glob: &str, repo: Option<&str>) -> String {
    let Some(repo) = repo else {
        return glob.to_owned();
    };
    if htui_core::prompt::glob::compile(glob)
        .ok()
        .and_then(|p| p.repo)
        .as_deref()
        != Some(repo)
    {
        return glob.to_owned();
    }
    glob.strip_prefix(&format!("{repo}:"))
        .unwrap_or(glob)
        .to_owned()
}

/// `text` cut to `width` chars with an ellipsis, so a long name cannot push the cells after it
/// off the row.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() > width {
        let head: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{head}\u{2026}")
    } else {
        text.to_owned()
    }
}

impl Axis {
    /// The `project_id` the write carries; `None` is the global row.
    const fn project(self) -> Option<ProjectId> {
        match self {
            Self::Global => None,
            Self::Project(project) | Self::Phase(project, _) => Some(project),
        }
    }

    /// The `phase_id` the write carries; `None` is the global or the project level.
    const fn phase(self) -> Option<PhaseId> {
        match self {
            Self::Phase(_, phase) => Some(phase),
            Self::Global | Self::Project(_) => None,
        }
    }
}

impl Pin {
    /// `skill_binding.pinned_version`, or `None` for [`Pin::Latest`].
    const fn version(self) -> Option<i32> {
        match self {
            Self::Latest => None,
            Self::Version(version) => Some(version),
        }
    }

    /// The next rung of `latest → v1 → … → vN → latest`, over the versions the skill has. A
    /// skill with no version has only the first rung, so `p` leaves it where it is.
    fn next(self, versions: &[SkillVersion]) -> Self {
        let Some(current) = self.version() else {
            return versions
                .first()
                .map_or(Self::Latest, |head| Self::Version(head.version));
        };
        let Some(next) = versions.iter().find(|version| version.version > current) else {
            return Self::Latest;
        };
        Self::Version(next.version)
    }
}

impl Form {
    /// The non-empty trimmed lines of one of the two multi-line fields.
    fn lines_of(area: &TextArea) -> Vec<String> {
        area.text()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The globs the field holds, each carrying the repo qualifier when the picker set one.
    /// A line that already carries **that** qualifier is left alone: the field can be typed into,
    /// and a glob typed as `repo:src/**` under a `repo` qualifier is one glob, not two.
    fn typed_globs(&self) -> Vec<String> {
        Self::lines_of(&self.globs)
            .into_iter()
            .map(|glob| match &self.qualifier {
                Some(repo) if !glob.starts_with(&format!("{repo}:")) => format!("{repo}:{glob}"),
                _ => glob,
            })
            .collect()
    }

    /// The languages as typed; `skill_binding.languages` keeps exactly this.
    fn language_names(&self) -> Vec<String> {
        Self::lines_of(&self.languages)
    }

    /// What the store will receive: the typed globs unioned with every named language's
    /// expansion (D83, H-33). The preview the pane draws is this list, so the two cannot
    /// disagree about the order.
    fn effective(&self) -> Vec<String> {
        expand_languages(
            self.typed_globs().iter().map(String::as_str),
            self.language_names().iter().map(String::as_str),
        )
    }

    /// The cursor of the focused field, for the hint's `L{line}:C{col}`.
    fn cursor(&self) -> (usize, usize) {
        match self.field {
            0 => self.globs.cursor_line_col(),
            1 => self.languages.cursor_line_col(),
            // A `TextField` has one line and a char cursor, so its column is its length: the
            // hint's `L1:C{len+1}` is the cell just past the last character, which is where the
            // field draws its cursor.
            _ => (0, self.position.len()),
        }
    }

    /// `skill_binding.position`, or `None` when the field does not hold a whole number.
    fn position(&self) -> Option<i32> {
        let typed = self.position.text().unwrap_or_default().trim();
        if typed.is_empty() {
            return Some(0);
        }
        typed.parse::<i32>().ok()
    }
}

impl MatrixView {
    /// Whether the matrix is showing, and therefore whether it owns every key: `Esc` closes it
    /// and `h`/`l` stop being the tab's view switch while it is up.
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// The scope changed: the matrix, the form, the picker and the write in flight all belong to
    /// the workspace that was left. The notice survives, as in `settings/prompt.rs`.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch. `m` opens and closes the matrix, `Esc`
    /// closes it, and everything else belongs to whichever form is up.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !self.open {
            if key.code == KeyCode::Char('m') && plain(&key) {
                self.open_matrix(ctx);
                return Handled::Consumed;
            }
            return Handled::Pass;
        }
        if self.picker.is_some() {
            return self.on_picker_key(key);
        }
        if self.form.is_some() {
            return self.on_form_key(key, ctx);
        }
        if !plain(&key) {
            return Handled::Pass;
        }
        self.on_browse_key(key, ctx)
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
                if let Some(busy) = self.busy.take() {
                    // The write's own answer closes the form: the reply re-read the whole scope,
                    // so what the pane shows next is the store and not the draft that produced it.
                    self.form = None;
                    self.picker = None;
                    self.notice = Some(Notice::Info(if busy.request == UNSET_NAME {
                        format!("detached {} from {}", busy.name, busy.level)
                    } else {
                        format!("attached {} at {}", busy.name, busy.level)
                    }));
                }
                self.clamp(ctx);
            }
            StoreReply::SkillsStale(snapshot) => {
                if !in_scope(snapshot, ctx) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.unavailable = None;
                if self.busy.take().is_some() {
                    // The row is as the writer that won left it, so the form is dropped rather
                    // than kept: a kept draft would hold a spent token and offer a save that could
                    // only be stale again (D78, R-32).
                    self.form = None;
                    self.picker = None;
                    self.notice = Some(Notice::Error(
                        "this attachment changed elsewhere; it is unchanged".to_owned(),
                    ));
                }
                self.clamp(ctx);
            }
            StoreReply::Catalogue(catalogue) => {
                self.catalogue = Some((**catalogue).clone());
                self.clamp(ctx);
            }
            StoreReply::Hierarchy(tree) => {
                self.repos = tree.as_ref().map(|boxed| Box::new((**boxed).clone()));
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                // A refused write leaves the form over its draft: nothing was written.
                self.busy = None;
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
    }

    /// Draws the view below the switch line: the matrix, the notice row and the hint row.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The notice wraps rather than clips: a refusal is the writer's own sentence and carries
        // the byte the mistake is at, which is the part the user must not lose.
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
        let hint = if self.picker.is_some() {
            PICKER_HINT.to_owned()
        } else if let Some(form) = &self.form {
            let (line, col) = form.cursor();
            format!("{FORM_HINT}  L{}:C{}", line + 1, col + 1)
        } else {
            BROWSE_HINT.to_owned()
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
        self.render_body(frame, content, ctx);
    }

    /// `m`: the matrix is showing, or starts. The catalogue is asked for once per workspace,
    /// because the level list cannot be drawn without it and `ctx.scope` has already changed by
    /// the time a second workspace is opened (`on_scope_change` drops it).
    fn open_matrix(&mut self, ctx: &Ctx<'_>) {
        self.open = true;
        self.level = 0;
        self.skill = 0;
        self.first_column = 0;
        self.scroll.reset();
        self.notice = None;
        if self.catalogue.is_none() {
            ctx.request(StoreRequest::Catalogue(ctx.scope.clone()));
        }
    }

    /// Browse. Every key here misses the global table — see `every_matrix_browse_key_misses_the_
    /// global_table` — and the tab took `h`/`l`/`[`/`]` before them, so neither can be a level.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_level(true, ctx),
            KeyCode::Char('k') | KeyCode::Up => self.move_level(false, ctx),
            KeyCode::Left => self.move_skill(false),
            KeyCode::Right => self.move_skill(true),
            KeyCode::Char('g') => {
                self.level = 0;
                self.after_move();
            }
            KeyCode::Char('G') => {
                self.level = levels(self, ctx).len().saturating_sub(1);
                self.after_move();
            }
            // `a` is `open_form` then `save`, so an attach and an edit are one code path: the
            // defaults a cell with no row gets are exactly the ones the form would open on.
            KeyCode::Char('a') => {
                self.open_form(ctx);
                self.save(ctx);
            }
            KeyCode::Char('e') => self.open_form(ctx),
            KeyCode::Char('x') => self.detach(ctx),
            // `A` and `p` open the form and cycle **in it**. A cycle applied to the cell itself
            // would walk straight into the `glob`-needs-a-glob refusal, and a refusal mid-cycle
            // is a cycle that stopped for a reason the user did not ask about.
            KeyCode::Char('A') => {
                self.open_form(ctx);
                self.cycle_activation();
            }
            KeyCode::Char('p') => {
                self.open_form(ctx);
                self.cycle_pin();
            }
            KeyCode::Char('r') => {
                self.notice = None;
                self.catalogue = None;
                ctx.request(StoreRequest::Skills(ctx.scope.clone()));
                ctx.request(StoreRequest::Catalogue(ctx.scope.clone()));
            }
            KeyCode::Char('J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
                return self.scroll.on_key(key, self.pane_rows.get());
            }
            KeyCode::Char('m') | KeyCode::Esc => {
                self.open = false;
                self.form = None;
                self.picker = None;
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// The form's keys. `Ctrl+S`, `Tab`, `A`, `p` and `R` are taken before the focused field, so
    /// a glob can still hold a `p` and a language a capital `A`.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Esc if plain(&key) => {
                self.form = None;
                self.picker = None;
            }
            KeyCode::Char('A') if plain(&key) => self.cycle_activation(),
            KeyCode::Char('p') if plain(&key) => self.cycle_pin(),
            KeyCode::Char('R') if plain(&key) => self.open_picker(ctx),
            KeyCode::Tab if plain(&key) => {
                if let Some(form) = &mut self.form {
                    form.field = (form.field + 1) % 3;
                }
            }
            KeyCode::Char('s' | 'S') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save(ctx);
            }
            _ => {
                let Some(form) = &mut self.form else {
                    return Handled::Pass;
                };
                let outcome = match form.field {
                    0 => form.globs.on_key(key, FIELD_ROWS),
                    1 => form.languages.on_key(key, FIELD_ROWS),
                    _ => form.position.on_key(key),
                };
                // H-37: the languages field is the one place an unknown name is a silent no-op,
                // so it is also the one place the form can say so. The notice follows the field,
                // which means it clears as soon as the name is one the map holds.
                if form.field == 1 {
                    self.notice = unknown_language(form).map(Notice::Info);
                }
                return match outcome {
                    FieldOutcome::Pass => Handled::Pass,
                    FieldOutcome::Consumed | FieldOutcome::Submit | FieldOutcome::Cancel => {
                        Handled::Consumed
                    }
                };
            }
        }
        Handled::Consumed
    }

    /// The picker's keys. It is a mode of the form, so it takes them first.
    fn on_picker_key(&mut self, key: KeyEvent) -> Handled {
        let names = self.picker_names();
        if names.is_empty() {
            return Handled::Pass;
        }
        let last = names.len() - 1;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(picker) = &mut self.picker {
                    picker.cursor = (picker.cursor + 1).min(last);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(picker) = &mut self.picker {
                    picker.cursor = picker.cursor.saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                let chosen = names[self.picker.as_ref().map_or(0, |picker| picker.cursor)].clone();
                if let Some(form) = &mut self.form {
                    form.qualifier = (chosen != names[0]).then_some(chosen);
                }
                self.picker = None;
            }
            KeyCode::Esc => self.picker = None,
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// `j`/`k`: one level, no wrap. The pane and the notice follow the cursor.
    fn move_level(&mut self, down: bool, ctx: &Ctx<'_>) {
        let last = levels(self, ctx).len().saturating_sub(1);
        self.level = if down {
            (self.level + 1).min(last)
        } else {
            self.level.saturating_sub(1)
        };
        self.after_move();
    }

    /// `Left`/`Right`: one skill column, no wrap.
    fn move_skill(&mut self, right: bool) {
        let last = self.skill_count().saturating_sub(1);
        self.skill = if right {
            (self.skill + 1).min(last)
        } else {
            self.skill.saturating_sub(1)
        };
        self.after_move();
    }

    /// The cursor moved: the pane goes back to the top and the notice is dropped, so a stale
    /// refusal is never read as the new cell's.
    fn after_move(&mut self) {
        self.scroll.reset();
        self.notice = None;
        self.clamp_columns();
    }

    /// Opens the form over the selected cell, prefilled with what the store holds and with the
    /// defaults a cell with no row gets.
    fn open_form(&mut self, ctx: &Ctx<'_>) {
        let levels = levels(self, ctx);
        let (Some(entry), Some(level)) =
            (selected_skill(self), selected_level(&levels, self.level))
        else {
            self.notice = Some(Notice::Info(SELECT_A_CELL.to_owned()));
            return;
        };
        let row = cell(self, entry.id, level.axis);
        // A stored glob is the **effective** list, qualifier and all, so the qualifier is read
        // back off it: a reopened qualified attachment saved again is the same row, not a bare
        // one. The field then holds that same list with **the qualifier stripped**, because
        // `typed_globs` puts it back on every line — seeding the field with the stored strings
        // would save `repo:repo:`, which the matcher refuses with an error about a glob the
        // user never typed.
        let qualifier = row.and_then(|row| {
            row.globs
                .iter()
                .find_map(|glob| htui_core::prompt::glob::compile(glob).ok())
                .and_then(|pattern| pattern.repo.clone())
        });
        let globs = row.map_or_else(String::new, |row| {
            row.globs
                .iter()
                .map(|glob| unqualified(glob, qualifier.as_deref()))
                .collect::<Vec<_>>()
                .join("\n")
        });
        self.form = Some(Form {
            skill_id: entry.id,
            name: entry.name.clone(),
            level: level.axis,
            token: row.map(|row| row.updated_at),
            activation: row.map_or(Activation::Always, |row| row.activation),
            pin: row
                .and_then(|row| row.pinned_version)
                .map_or(Pin::Latest, Pin::Version),
            globs: TextArea::with_text(&globs),
            languages: TextArea::with_text(
                &row.map_or_else(String::new, |row| row.languages.join("\n")),
            ),
            position: TextField::with_text(
                &row.map_or_else(|| "0".to_owned(), |row| row.position.to_string()),
            ),
            qualifier,
            field: 0,
        });
        self.scroll.reset();
        self.notice = None;
    }

    /// `A`: `always → glob → off → always`, in the store's own `CHECK` order.
    fn cycle_activation(&mut self) {
        let Some(form) = &mut self.form else {
            return;
        };
        let at = Activation::ALL
            .iter()
            .position(|variant| *variant == form.activation)
            .unwrap_or(0);
        form.activation = Activation::ALL[(at + 1) % Activation::ALL.len()];
    }

    /// `p`: `latest → v1 → … → vN → latest`, over the versions the skill has.
    fn cycle_pin(&mut self) {
        let Some(form) = &self.form else {
            return;
        };
        let available: Vec<SkillVersion> = versions(self, form.skill_id).to_vec();
        let next = form.pin.next(&available);
        if let Some(form) = &mut self.form {
            form.pin = next;
        }
    }

    /// `R`: the repo picker, refused on a global row before it asks for anything — the same
    /// rule the writer enforces on a qualified glob, and the row the user is on is a global one.
    fn open_picker(&mut self, ctx: &mut Ctx<'_>) {
        let Some(form) = &self.form else {
            return;
        };
        if form.level.project().is_none() {
            self.notice = Some(Notice::Error(NO_REPO_ON_GLOBAL.to_owned()));
            return;
        }
        self.picker = Some(Picker { cursor: 0 });
        self.repos = None;
        ctx.request(StoreRequest::Hierarchy(ctx.scope.workspace_id));
    }

    /// `Ctrl+S`: the form's own copy of D78's rules, refused **before** the request so a mistake
    /// costs no round trip, then the request itself.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = &self.busy {
            self.notice = Some(Notice::Error(format!(
                "`{}` is still in flight",
                busy.request
            )));
            return;
        }
        let Some(form) = &self.form else {
            return;
        };
        if let Some(why) = refusal(self, form) {
            self.notice = Some(Notice::Error(why));
            return;
        }
        let busy = Busy {
            request: SET_NAME,
            name: form.name.clone(),
            level: level_label(self, ctx, form.level),
        };
        let request = StoreRequest::SetSkillBinding {
            scope: ctx.scope.clone(),
            skill_id: form.skill_id,
            project_id: form.level.project(),
            phase_id: form.level.phase(),
            pinned_version: form.pin.version(),
            // `refusal` has already parsed it, so this cannot be `None`.
            position: form.position().unwrap_or_default(),
            activation: form.activation,
            globs: form.effective(),
            languages: form.language_names(),
            expected: form.token,
        };
        self.busy = Some(busy);
        self.notice = Some(Notice::Info(WRITING.to_owned()));
        ctx.request(request);
    }

    /// `x`: `RemoveSkillBinding` with the row's own `updated_at`, so a row another writer changed
    /// survives the unbind that lost the race (OQ-19, R-32).
    fn detach(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = &self.busy {
            self.notice = Some(Notice::Error(format!(
                "`{}` is still in flight",
                busy.request
            )));
            return;
        }
        let levels = levels(self, ctx);
        let (Some(entry), Some(level)) =
            (selected_skill(self), selected_level(&levels, self.level))
        else {
            self.notice = Some(Notice::Info(SELECT_A_CELL.to_owned()));
            return;
        };
        let Some(row) = cell(self, entry.id, level.axis) else {
            self.notice = Some(Notice::Info("nothing is attached at this level".to_owned()));
            return;
        };
        let busy = Busy {
            request: UNSET_NAME,
            name: entry.name.clone(),
            level: level.label.clone(),
        };
        let request = StoreRequest::RemoveSkillBinding {
            scope: ctx.scope.clone(),
            id: row.id,
            expected: row.updated_at,
        };
        self.busy = Some(busy);
        self.notice = Some(Notice::Info(WRITING.to_owned()));
        ctx.request(request);
    }

    /// The matrix on the left, the pane on the right — the Templates view's own split.
    fn render_body(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let [list_area, pane_area] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(1)]).areas(area);

        let list = Block::new().borders(Borders::ALL).title(" Attachments ");
        let inner = list.inner(list_area);
        frame.render_widget(list, list_area);
        let mut lines = vec![self.header_line(theme)];
        for (index, level) in levels(self, ctx).iter().enumerate() {
            lines.push(self.level_line(index, level, theme));
        }
        // The header is line 0 and the selected level is line `level + 1`; the list scrolls just
        // far enough to keep the selected row on screen, as the library's does.
        let offset = (self.level + 1).saturating_sub(usize::from(inner.height));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            inner,
        );

        // The pane's lines wrap: a glob is longer than the pane's width, and a refusal carries a
        // sentence that is. The row count is a character wrap's, a lower bound on the word wrap's
        // (`Scroll`'s rule), so the clamp never scrolls the pane blank.
        let width = pane_area.width.saturating_sub(2);
        let (title, lines) = self.pane(width, ctx);
        let rows: usize = lines
            .iter()
            .map(|line| line.width().div_ceil(usize::from(width).max(1)).max(1))
            .sum();
        self.pane_rows.set(rows);
        let mut block = Block::new().borders(Borders::ALL).title(title);
        if rows > usize::from(pane_area.height.saturating_sub(2)) || self.scroll.offset() > 0 {
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

    /// The header row: the level column's name, then one column per visible skill.
    fn header_line(&self, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(
            format!("  {LEVEL_HEADER:<LEVEL_WIDTH$}"),
            theme.dim,
        )];
        for column in self.visible_columns() {
            let name = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.skills.get(column))
                .map_or(String::new(), |entry| cut(&entry.name, CELL_WIDTH - 1));
            spans.push(Span::styled(format!("{name:<CELL_WIDTH$}"), theme.dim));
        }
        Line::from(spans)
    }

    /// One level's line: the label, then one cell per visible skill. The **cell** carries
    /// `theme.selected`, which is why this is a packed block and not a `Table`: a `Table` has a
    /// row selection and no cell one (D82 as amended).
    fn level_line(&self, index: usize, level: &Level, theme: &Theme) -> Line<'static> {
        let label_style = if index == self.level {
            theme.base
        } else {
            theme.dim
        };
        let mut spans = vec![Span::styled(
            format!("  {:<LEVEL_WIDTH$}", level.label),
            label_style,
        )];
        for column in self.visible_columns() {
            let Some(entry) = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.skills.get(column))
            else {
                continue;
            };
            let style = if column == self.skill {
                theme.selected
            } else {
                theme.base
            };
            spans.push(Span::styled(
                format!("{:<CELL_WIDTH$}", cell_text(self, entry.id, level.axis)),
                style,
            ));
        }
        Line::from(spans)
    }

    /// The right-hand pane's title and lines: the picker, the form, a refusal, the empty states,
    /// or the selected cell's detail. The caller wraps and scrolls them.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        if self.picker.is_some() {
            return self.picker_pane(ctx);
        }
        if self.form.is_some() {
            return self.form_pane(width, ctx);
        }
        let theme = ctx.theme;
        let dim = |text: String| (String::new(), vec![Line::styled(text, theme.dim)]);
        if let Some(why) = &self.unavailable {
            return (
                String::new(),
                vec![Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)],
            );
        }
        if self.snapshot.is_none() {
            return dim(NOT_READ.to_owned());
        }
        let rows = levels(self, ctx);
        let (Some(entry), Some(level)) = (selected_skill(self), selected_level(&rows, self.level))
        else {
            return dim(SELECT_A_CELL.to_owned());
        };
        let title = format!(" {} @ {} ", entry.name, level.label);
        let Some(row) = cell(self, entry.id, level.axis) else {
            return (
                title,
                vec![
                    Line::styled(" nothing is attached at this level".to_owned(), theme.dim),
                    Line::styled(" a attaches, e edits, x detaches".to_owned(), theme.dim),
                ],
            );
        };
        let versions = versions(self, entry.id);
        let head = versions.last().map_or(0, |version| version.version);
        let pin = match (
            row.pinned_version,
            versions
                .iter()
                .find(|v| Some(v.version) == row.pinned_version),
        ) {
            (Some(pinned), Some(_)) => format!("v{pinned} (pinned)"),
            (Some(pinned), None) => format!("v{pinned} (pinned; no such version)"),
            (None, _) => format!("v{head} (latest)"),
        };
        let qualifier = row
            .globs
            .iter()
            .find_map(|glob| htui_core::prompt::glob::compile(glob).ok())
            .and_then(|pattern| pattern.repo);
        let field = |label: &str, value: String| {
            Line::styled(format!(" {label:<LABEL_WIDTH$} {value}"), theme.base)
        };
        let mut lines = vec![
            field("level:", level.label.clone()),
            field("skill:", entry.name.clone()),
            field("version:", pin),
            field("activation:", row.activation.as_str().to_owned()),
            field("position:", row.position.to_string()),
            field(
                "repo:",
                qualifier.unwrap_or_else(|| "every repo".to_owned()),
            ),
        ];
        if row.globs.is_empty() {
            lines.push(field("globs:", "(none)".to_owned()));
        } else {
            lines.push(Line::styled(" globs:", theme.base));
            lines.extend(row.globs.iter().map(|glob| sub(glob, theme)));
        }
        lines.push(field(
            "languages:",
            if row.languages.is_empty() {
                "(none)".to_owned()
            } else {
                row.languages.join(" ")
            },
        ));
        (title, lines)
    }

    /// The form: the two multi-line fields, the cycles, and the effective globs the save will
    /// write. The preview is the same [`Form::effective`] the request carries, so what the pane
    /// shows and what the store receives cannot differ (D83, H-33).
    fn form_pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        let theme = ctx.theme;
        let Some(form) = &self.form else {
            return (String::new(), Vec::new());
        };
        let title = format!(" {} @ {} ", form.name, level_label(self, ctx, form.level));
        let field = |label: &str, value: String| {
            Line::styled(format!(" {label:<LABEL_WIDTH$} {value}"), theme.base)
        };
        let head = versions(self, form.skill_id)
            .last()
            .map_or(0, |version| version.version);
        let mut lines = vec![
            field("activation:", form.activation.as_str().to_owned()),
            field(
                "pin:",
                match form.pin {
                    Pin::Latest => format!("latest (v{head})"),
                    Pin::Version(version) => format!("v{version}"),
                },
            ),
            field(
                "position:",
                form.position.text().unwrap_or_default().to_owned(),
            ),
            field(
                "repo:",
                form.qualifier
                    .clone()
                    .unwrap_or_else(|| "every repo".to_owned()),
            ),
            Line::styled(" globs (one per line):", theme.base),
        ];
        let inner = width.saturating_sub(2);
        lines.extend(form.globs.lines(inner, FIELD_ROWS, form.field == 0, theme));
        lines.push(Line::styled(" languages (one per line):", theme.base));
        lines.extend(
            form.languages
                .lines(inner, FIELD_ROWS, form.field == 1, theme),
        );
        lines.push(Line::styled(
            format!(" known: {}", LANGUAGE_NAMES().join(" ")),
            theme.dim,
        ));
        lines.push(Line::styled(" effective globs:", theme.base));
        let effective = form.effective();
        if effective.is_empty() {
            lines.push(Line::styled("   (none)", theme.dim));
        } else {
            lines.extend(effective.iter().map(|glob| sub(glob, theme)));
        }
        (title, lines)
    }

    /// The repo picker: what the qualifier is now, and the project's repos under it.
    fn picker_pane(&self, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        let theme = ctx.theme;
        let Some(form) = &self.form else {
            return (String::new(), Vec::new());
        };
        let names = self.picker_names();
        let mut lines = vec![Line::styled(
            format!(
                " {:<LABEL_WIDTH$} {}",
                "repo:",
                form.qualifier
                    .clone()
                    .unwrap_or_else(|| "every repo".to_owned())
            ),
            theme.base,
        )];
        if self.repos.is_none() {
            lines.push(Line::styled(
                " reading this project's repos\u{2026}".to_owned(),
                theme.dim,
            ));
            return (" repo ".to_owned(), lines);
        }
        lines.push(Line::styled(
            " pick one of this project's repos:".to_owned(),
            theme.dim,
        ));
        let cursor = self.picker.as_ref().map_or(0, |picker| picker.cursor);
        for (index, name) in names.iter().enumerate() {
            let marker = if index == cursor { ">" } else { " " };
            let style = if index == cursor {
                theme.selected
            } else {
                theme.base
            };
            lines.push(Line::styled(format!("  {marker} {name}"), style));
        }
        (" repo ".to_owned(), lines)
    }

    /// The picker's rows: "every repo" first, then the project's repos by name. Empty until the
    /// hierarchy read lands, which is what makes [`on_picker_key`](Self::on_picker_key) refuse
    /// every key rather than pick a row that is not there yet.
    fn picker_names(&self) -> Vec<String> {
        let mut names = vec!["every repo".to_owned()];
        let (Some(form), Some(tree)) = (&self.form, &self.repos) else {
            return Vec::new();
        };
        let Some(project) = form.level.project() else {
            return Vec::new();
        };
        let Some(entry) = tree.projects.iter().find(|row| row.project.id == project) else {
            return names;
        };
        names.extend(entry.repos.iter().map(|repo| repo.repo.name.clone()));
        names
    }

    /// The skill columns the list draws right now, so the selected one is always on screen.
    fn visible_columns(&self) -> std::ops::Range<usize> {
        self.first_column..(self.first_column + MAX_COLUMNS).min(self.skill_count())
    }

    /// How many skills the last read answered for.
    fn skill_count(&self) -> usize {
        self.snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.skills.len())
    }

    /// Keeps the cursor and the sideways scroll on a row and a column that exist, after a reply
    /// changed what the scope holds.
    fn clamp(&mut self, ctx: &Ctx<'_>) {
        self.level = self.level.min(levels(self, ctx).len().saturating_sub(1));
        self.skill = self.skill.min(self.skill_count().saturating_sub(1));
        self.clamp_columns();
    }

    /// Slides the sideways scroll the least it has to, so the selected cell is drawn.
    fn clamp_columns(&mut self) {
        if self.first_column > self.skill {
            self.first_column = self.skill;
        }
        if self.skill >= self.first_column + MAX_COLUMNS {
            self.first_column = self.skill + 1 - MAX_COLUMNS;
        }
    }
}

/// The level list, in the order the rows are drawn: the global row, then each scope project, then
/// that project's phases by graph name and phase position. Derived, so it cannot disagree with
/// what the pane shows.
fn levels(view: &MatrixView, ctx: &Ctx<'_>) -> Vec<Level> {
    let mut out = vec![Level {
        axis: Axis::Global,
        label: GLOBAL_LABEL.to_owned(),
    }];
    for project in ctx
        .projects
        .iter()
        .filter(|project| ctx.scope.contains(project.project_id))
    {
        out.push(Level {
            axis: Axis::Project(project.project_id),
            label: project.slug.clone(),
        });
        let Some(catalogue) = &view.catalogue else {
            continue;
        };
        let Some(entry) = catalogue
            .projects
            .iter()
            .find(|row| row.project.id == project.project_id)
        else {
            continue;
        };
        for graph in &entry.graphs {
            for phase in &graph.phases {
                out.push(Level {
                    axis: Axis::Phase(project.project_id, phase.id),
                    label: format!("{}/{}", graph.graph.name, phase.name),
                });
            }
        }
    }
    out
}

/// The row the cursor is on, or `None` when the list is empty.
fn selected_level(levels: &[Level], level: usize) -> Option<&Level> {
    levels.get(level)
}

/// The skill under the cursor, if the cursor is on one.
fn selected_skill(view: &MatrixView) -> Option<&SkillSummary> {
    view.snapshot.as_ref()?.skills.get(view.skill)
}

/// The label of any level, whether or not it is the one under the cursor — a `Busy` and a pane
/// title both need it after the cursor has moved on.
fn level_label(view: &MatrixView, ctx: &Ctx<'_>, axis: Axis) -> String {
    levels(view, ctx)
        .into_iter()
        .find(|level| level.axis == axis)
        .map_or_else(|| GLOBAL_LABEL.to_owned(), |level| level.label)
}

/// The attachment of one `(skill, level)` cell, or `None` when the level holds none.
fn cell(view: &MatrixView, skill: SkillId, axis: Axis) -> Option<&SkillAttachmentRow> {
    view.snapshot
        .as_ref()?
        .attachment(skill, axis.project(), axis.phase())
}

/// One cell's text: the version in force, `?` for a pin the skill has no version for, a `g` for a
/// `glob` activation, and [`NO_CELL`] for no attachment at all.
fn cell_text(view: &MatrixView, skill: SkillId, axis: Axis) -> String {
    let Some(row) = cell(view, skill, axis) else {
        return NO_CELL.to_owned();
    };
    let known = versions(view, skill);
    let version = row
        .pinned_version
        .or_else(|| known.last().map(|version| version.version));
    let text = match version {
        Some(version) if known.iter().any(|row| row.version == version) => format!("v{version}"),
        _ => UNKNOWN.to_owned(),
    };
    if row.activation == Activation::Glob {
        format!("{text}g")
    } else {
        text
    }
}

/// The versions of the skill a pin has to name.
fn versions(view: &MatrixView, skill: SkillId) -> &[SkillVersion] {
    view.snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.skills.iter().find(|entry| entry.id == skill))
        .map_or(&[], |entry| entry.versions.as_slice())
}

/// The form's own copy of D78's rules, in D78's order, **before** the request is sent (D100).
///
/// The position is this view's own rule — the writer has no `position` check and never had one —
/// so it is parsed first, and its sentence names this file. Everything after it is
/// [`skill_binding_refusal`]'s or [`skill_pin_refusal`]'s own sentence, verbatim, which is what
/// makes the view's notice and the store's `Constraint` the same string.
fn refusal(view: &MatrixView, form: &Form) -> Option<String> {
    let Some(position) = form.position() else {
        return Some(format!(
            "skill_binding.position must be a whole number, at `{}`",
            form.position.text().unwrap_or_default().trim()
        ));
    };
    // `NewSkillBinding::id` is client-minted and this one is never sent: the worker mints the
    // write's own (`skills::serve`), and the refusals read no id.
    let new = NewSkillBinding {
        id: SkillBindingId::new(),
        skill_id: form.skill_id,
        project_id: form.level.project(),
        phase_id: form.level.phase(),
        pinned_version: form.pin.version(),
        position,
        activation: form.activation,
        globs: form.effective(),
        languages: form.language_names(),
    };
    if let Some(why) = skill_binding_refusal(&new) {
        return Some(why);
    }
    if let Pin::Version(pinned) = form.pin
        && let Some(why) = skill_pin_refusal(pinned, form.skill_id, versions(view, form.skill_id))
    {
        return Some(why);
    }
    None
}

/// A reply that names a project outside the current scope, which is the case a scope change
/// actually produces. `on_scope_change` has already dropped the previous catalogue and hierarchy
/// reads, so this only ever sees a reply that was in flight across the change.
fn in_scope(snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> bool {
    snapshot.attachments.iter().all(|row| {
        row.project_id
            .is_none_or(|project| ctx.scope.project_ids.contains(&project))
    })
}

#[cfg(test)]
mod tests {
    use super::LANGUAGE_NAMES;
    use crate::keymap::{KeyChord, KeyScope};
    use crate::ui::overlay::OverlayId;
    use htui_core::store::traits::invalid_skill_name;

    /// D102 / F-13: every key the matrix claims is checked against the global table, so a key
    /// added to `default_global` later cannot silently become a second binding.
    #[test]
    fn every_matrix_browse_key_misses_the_global_table() {
        let map = crate::keymap::Keymap::default_global();
        let claimed = [
            "j", "k", "down", "up", "left", "right", "g", "G", "a", "e", "x", "A", "p", "r", "R",
            "m", "J", "K", "pagedown", "pageup", "ctrl-s", "esc",
        ];
        for spec in claimed {
            let chord = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` parses"));
            assert!(
                map.resolve(&KeyScope::Global, chord).is_none(),
                "`{spec}` is claimed by the matrix's keys and must not be in the global table"
            );
        }
        // `Esc` is the shell's **overlay** close, under `KeyScope::Overlay(ANY)` and not
        // `KeyScope::Global`, which is what lets the matrix take it: no overlay is up while a tab
        // is drawn, so the chord never reaches the overlay's arm.
        assert!(
            map.resolve(
                &KeyScope::Overlay(OverlayId::ANY),
                KeyChord::parse("esc").expect("parses")
            )
            .is_some(),
            "`Esc` is the overlay close, under the overlay scope and not the global one"
        );
        // `Tab` is the one deliberate exception: it is bound globally and the form takes it to move
        // its cursor, which is what H-32 names for the Templates view's naming prompt.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("tab").expect("parses"))
                .is_some(),
            "`Tab` is the form's documented exception, so it stays in the global table"
        );
        // D102: `w` is the workspace switcher's, bound in `register_all` rather than in
        // `default_global`, and the matrix leaves it alone.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("w").expect("parses"))
                .is_none(),
            "`w` is the switcher's and is bound outside `default_global`, so this table cannot \
             see it — which is why the matrix never claims it"
        );
    }

    /// The names the form lists under the language field, so a maintainer can see what the map
    /// holds without reading the source.
    #[test]
    fn the_language_list_is_the_map_own_order() {
        assert_eq!(
            LANGUAGE_NAMES(),
            htui_core::model::language::languages(),
            "the list and the map cannot be two orders"
        );
        assert!(
            LANGUAGE_NAMES().contains(&"shell"),
            "`shell` is the three-pattern language the expansion test rides"
        );
    }

    /// The refusal sentences the view shows are the writer's own, so this pins that the name rule
    /// is reached through one function rather than re-spelled per view (D100).
    #[test]
    fn the_writers_sentences_are_the_ones_the_view_shows() {
        assert!(
            invalid_skill_name("House Rules").contains("must be 1-64 characters of `[a-z0-9-]`"),
            "the name rule is one function beside the template's, and the library view shows it \
             verbatim for the same reason"
        );
    }

    /// H-30: the view derives `Debug` down to the open form, and a glob and a language name are
    /// what a maintainer types. The two multi-line fields are `TextArea`s and the position is a
    /// `TextField`, so all three print lengths — the same rule the Templates body editor and the
    /// `$EDITOR` handoff follow.
    #[test]
    fn an_open_form_debug_prints_lengths_not_text() {
        let form = super::Form {
            skill_id: htui_core::fixtures::ids::SKILL_TESTS,
            name: "tests".to_owned(),
            level: super::Axis::Global,
            token: None,
            activation: htui_core::model::Activation::Glob,
            pin: super::Pin::Latest,
            globs: crate::ui::TextArea::with_text("secret-glob\nsecond-line"),
            languages: crate::ui::TextArea::with_text("cobol"),
            position: crate::ui::TextField::with_text("4242"),
            qualifier: Some("private-repo".to_owned()),
            field: 0,
        };
        let printed = format!("{form:?}");
        for secret in ["secret-glob", "second-line", "cobol", "4242"] {
            assert!(
                !printed.contains(secret),
                "`{secret}` reached a `Debug`: {printed}"
            );
        }
        assert!(
            printed.contains("line_count: 2"),
            "the globs area: {printed}"
        );
        assert!(
            printed.contains("tests"),
            "the library key is not user text: {printed}"
        );
    }

    /// The reviewer found the reopen qualified twice: the form seeds its field with the **stored**
    /// list, which already carries the `<repo>:`, and `typed_globs` puts it back on every line.
    /// Both halves are pinned here, because either alone would leave the row uneditable.
    #[test]
    fn a_qualifier_is_written_once_over_a_reopened_glob() {
        let form = |globs: &str| super::Form {
            skill_id: htui_core::fixtures::ids::SKILL_TESTS,
            name: "tests".to_owned(),
            level: super::Axis::Project(htui_core::fixtures::ids::PROJECT_VULKAN),
            token: None,
            activation: htui_core::model::Activation::Glob,
            pin: super::Pin::Latest,
            globs: crate::ui::TextArea::with_text(globs),
            languages: crate::ui::TextArea::with_text(""),
            position: crate::ui::TextField::with_text("0"),
            qualifier: Some("tutorials".to_owned()),
            field: 0,
        };
        assert_eq!(
            form("**/*.rs").typed_globs(),
            ["tutorials:**/*.rs"],
            "a bare line takes the qualifier the picker chose"
        );
        assert_eq!(
            form("tutorials:**/*.rs").typed_globs(),
            ["tutorials:**/*.rs"],
            "a line that already carries **that** qualifier is one glob, not two — this is what a \
             reopen seeds the field with"
        );
        assert_eq!(
            form("api:**\ntutorials:**").typed_globs(),
            ["tutorials:api:**", "tutorials:**"],
            "and a line naming a different repo is the row's own text, which the form's single \
             qualifier cannot mean"
        );
        assert_eq!(
            super::unqualified("tutorials:**/*.rs", Some("tutorials")),
            "**/*.rs",
            "the field is seeded with the stored list, not the stored strings"
        );
        assert_eq!(
            super::unqualified("api:**", Some("tutorials")),
            "api:**",
            "a glob the form's qualifier does not name is left alone"
        );
    }

    /// The level label column is fixed, because the cells beside it are packed by width: a wider
    /// level would push every skill column off the frame. The demo seed's longest label is
    /// exactly [`LEVEL_WIDTH`](super::LEVEL_WIDTH) — `feature/implement` — so nothing in the
    /// fixture exercises the case, and the assertion has to drive [`level_line`] with a label the
    /// program can actually produce: `graph.name` and `project.slug` are free-form text and
    /// `StoreRequest::CreateGraph` takes whatever the user typed.
    #[test]
    fn a_level_label_wider_than_the_column_leaves_both_cell_columns_on_the_row() {
        let skill = |name: &str| super::SkillSummary {
            id: htui_core::model::SkillId::new(),
            name: name.to_owned(),
            description: String::new(),
            updated_at: chrono::DateTime::UNIX_EPOCH,
            versions: Vec::new(),
        };
        let mut view = super::MatrixView::default();
        view.snapshot = Some(crate::skills::SkillsSnapshot {
            skills: vec![skill("rust-style"), skill("tests")],
            attachments: Vec::new(),
        });
        let level = |label: &str| super::Level {
            axis: super::Axis::Global,
            label: label.to_owned(),
        };
        let theme = crate::ui::Theme::default();

        for label in [
            "global",
            "vulkan-tutorials",
            "feature/implement",
            "analysis/research",
            // 28 chars, and a name a maintainer really would type.
            "documentation-and-examples",
        ] {
            let line = view.level_line(0, &level(label), &theme);
            assert_eq!(
                line.width(),
                super::LIST_WIDTH as usize,
                "`{label}` is {} chars: the two cell columns must still be on the row, because the \
                 cells after a label wider than the column are otherwise outside the block and the \
                 selected one is invisible",
                label.chars().count()
            );
        }
        assert_eq!(
            super::MAX_COLUMNS,
            2,
            "the list's 46 columns hold two skills"
        );
        assert_eq!(
            2 + super::LEVEL_WIDTH + super::CELL_WIDTH * super::MAX_COLUMNS,
            super::LIST_WIDTH as usize,
            "and the row is packed to exactly the block's width"
        );
    }
}
