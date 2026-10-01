//! The Backlog's item form (MOD-13 milestone 2): `N` mints an item, `e` edits the selected one,
//! over the seven `version`-covered spec columns (ANA-9 §4.2).
//!
//! [`ItemForm`] is a capturing panel drawn in the detail pane's rect (D8), opened on an
//! [`ItemFormContext`] the worker read through the writer (D3). It is pure — no `Ctx` — and answers
//! the tab with an [`ItemFormOutcome`], as [`FilterForm`](super::filter::FilterForm) does.
//!
//! - **Ctrl+S is checked before a chord passes** (A6): every text widget passes chords, so the
//!   save would otherwise never reach the form. `Enter` on a one-line field moves on and never
//!   saves (D8).
//! - **The validator gives early feedback** (D4): `item_spec`'s checks run against the form's
//!   catalogue, and a refusal is the notice line, with nothing sent. The worker runs them again as
//!   the authority.
//! - **An edit sends only what changed** (D5, A4): a widget whose text is still what it opened
//!   with keeps the item's stored value unparsed, so a legacy tag never blocks a title fix; no
//!   change at all is `NOTHING_TO_SAVE`.
//! - **The token is the opened item's `version` and never moves** (D6): a stale save diverges, and
//!   a second save diverges again.
//! - **The new form's project picker re-reads that project's catalogue** (blueprint E7): kinds,
//!   graphs and repos are per project. Keys are swallowed until the reply re-targets the form.
//!
//! Redaction (blueprint E10): the form's `Debug` prints lengths, never the body or the paths.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::item_spec::{self, NOTHING_TO_SAVE};
use htui_core::model::{
    ItemId, ItemKindId, ItemSpec, ProjectId, ProjectRef, SpecChanges, SpecError, StepGraphId,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::item_writes::ItemFormContext;
use crate::store_worker::StoreRequest;
use crate::ui::tabs::backlog::filter;
use crate::ui::tabs::backlog::list::clip;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};

/// Lines `PageUp`/`PageDown` move in the paths and body areas.
pub const PAGE: u16 = 5;

/// The hint under a text field. It fits the detail pane's inner width at 100x30 (43 columns).
pub const HINT_TEXT: &str = "Tab field  Ctrl+S save  Esc cancel";

/// The hint under a picker. It fits the detail pane's inner width at 100x30 (43 columns).
pub const HINT_PICK: &str = "\u{2190}/\u{2192} choose  Tab field  Esc cancel";

/// The refusal of a save in a project without item kinds: an item needs one.
pub const NO_KINDS: &str = "this project has no item kinds; Settings \u{2192} Kinds adds one";

/// Width of a row's label column, after the two-cell focus marker.
const LABEL: usize = 9;

/// Rows the paths area takes.
const PATHS_HEIGHT: u16 = 3;

/// Rows the notice takes: the D6 sentence wraps to two at the detail pane's width.
const NOTICE_HEIGHT: u16 = 2;

/// One focusable part of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The project picker; a new form only (D8).
    Project,
    /// The item-kind picker.
    Kind,
    /// The title, one line.
    Title,
    /// The priority, one line, an `i16`.
    Priority,
    /// The required tags, a comma list.
    Tags,
    /// The step-graph picker: the kind's default or one of the project's graphs.
    Graph,
    /// The touched paths, one per line.
    Paths,
    /// The body.
    Body,
}

/// Every field in `Tab` order; an edit form skips the first.
const ORDER: [Field; 8] = [
    Field::Project,
    Field::Kind,
    Field::Title,
    Field::Priority,
    Field::Tags,
    Field::Graph,
    Field::Paths,
    Field::Body,
];

/// What the form is waiting on; it swallows every key but chords meanwhile (D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    /// The project picker moved to this project; its catalogue is being read (E7).
    Reloading(ProjectId),
    /// A `MintItem` is in flight.
    Minting,
    /// An `EditItem` is in flight.
    Editing,
}

/// What one key did to the form, for the tab to act on.
#[derive(Debug)]
pub enum ItemFormOutcome {
    /// The form edited, moved or refused; it stays open and nothing is sent.
    Stay,
    /// A chord the form does not own (`ctrl-c`, `Ctrl+F`): the tab passes it.
    Pass,
    /// `Esc`: close the form, send nothing.
    Cancel,
    /// The new form's project picker moved (E7): read that project's catalogue.
    Reload(ProjectId),
    /// Checked and ready; the form is already `busy`.
    Save(StoreRequest),
}

/// The open item form, new or edit.
///
/// Holds a snapshot of `ctx.projects` from when it opened; the tab drops the form on a scope
/// change, so the snapshot never outlives its scope.
pub struct ItemForm {
    /// `ctx.projects` at open (a new form); empty on an edit, whose project is fixed.
    projects: Vec<ProjectRef>,
    /// The catalogue the form checks against, and for an edit the item and its token.
    context: ItemFormContext,
    /// The chosen kind; `None` only when `context.kinds` is empty.
    kind: Option<ItemKindId>,
    /// The chosen graph; `None` is the kind's default.
    graph: Option<StepGraphId>,
    /// The title as typed.
    title: TextField,
    /// The priority as typed.
    priority: TextField,
    /// The required tags as typed.
    tags: TextField,
    /// The touched paths as typed, one per line.
    paths: TextArea,
    /// The body as typed.
    body: TextArea,
    /// Each widget's text right after open, read back from it: what "unchanged" means (A4).
    opened: Opened,
    /// The focused field; the form opens on Title.
    focus: Field,
    /// A read or write in flight.
    busy: Option<Busy>,
    /// The last refusal or reply, above the hint.
    notice: Option<String>,
}

/// The widgets' texts right after open.
struct Opened {
    title: String,
    priority: String,
    tags: String,
    paths: String,
    body: String,
}

/// Lengths only: the body and the paths are prose (blueprint E10).
impl core::fmt::Debug for Opened {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Opened")
            .field("title_len", &self.title.len())
            .field("priority_len", &self.priority.len())
            .field("tags_len", &self.tags.len())
            .field("paths_len", &self.paths.len())
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Never the body or the paths (blueprint E10, the `RequirementForm` precedent): `BacklogTab`
/// derives `Debug`, and the form holds an `Item` whose own `Debug` prints its body.
impl core::fmt::Debug for ItemForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ItemForm")
            .field("project", &self.context.project)
            .field(
                "item",
                &self
                    .context
                    .item
                    .as_ref()
                    .map(|item| (item.id, item.key.as_str())),
            )
            .field("kind", &self.kind)
            .field("graph", &self.graph)
            .field("focus", &self.focus)
            .field("busy", &self.busy)
            .field("notice", &self.notice)
            .field("body_len", &self.body.len())
            .field("paths_len", &self.paths.len())
            .finish_non_exhaustive()
    }
}

/// The text each widget opens with.
struct Texts<'a> {
    title: &'a str,
    priority: &'a str,
    tags: &'a str,
    paths: &'a str,
    body: &'a str,
}

/// Ctrl+S, with or without `SHIFT` (the Requirements tab's rule).
fn ctrl_s(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Char('s' | 'S'))
}

/// One step through `len` options from `at`, wrapping.
const fn wrap(at: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        at
    } else if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }
}

/// A field's value: the stored one while its widget still holds the text it opened with (A4),
/// else the widget's text parsed.
fn field<T: Clone>(
    stored: Option<&T>,
    typed: &str,
    opened: &str,
    parse: impl FnOnce(&str) -> Result<T, SpecError>,
) -> Result<T, SpecError> {
    match stored {
        Some(stored) if typed == opened => Ok(stored.clone()),
        _ => parse(typed),
    }
}

impl ItemForm {
    /// `N`: text blank, priority `0`, kind = `kind` when it is one of `context.kinds` else the
    /// first, graph the kind default, focus Title. `projects` is `ctx.projects` (scope order).
    #[must_use]
    pub fn open_new(
        context: ItemFormContext,
        projects: &[ProjectRef],
        kind: Option<ItemKindId>,
    ) -> Self {
        let kind = kind
            .filter(|hint| context.kinds.iter().any(|known| known.id == *hint))
            .or_else(|| context.kinds.first().map(|first| first.id));
        let texts = Texts {
            title: "",
            priority: "0",
            tags: "",
            paths: "",
            body: "",
        };
        Self::build(context, projects.to_vec(), kind, None, &texts)
    }

    /// `e`: every widget from the item's stored spec, through `tags_text`/`paths_text`, focus
    /// Title. `None` when `context.item` is `None`.
    #[must_use]
    pub fn open_edit(context: ItemFormContext) -> Option<Self> {
        let spec = ItemSpec::of(context.item.as_ref()?);
        let (priority, tags, paths) = (
            spec.priority.to_string(),
            item_spec::tags_text(&spec.required_tags),
            item_spec::paths_text(&spec.touched_paths),
        );
        let texts = Texts {
            title: &spec.title,
            priority: &priority,
            tags: &tags,
            paths: &paths,
            body: &spec.body,
        };
        Some(Self::build(
            context,
            Vec::new(),
            Some(spec.kind_id),
            spec.step_graph_id,
            &texts,
        ))
    }

    /// The form over `texts`, with `opened` read back from the widgets it built.
    fn build(
        context: ItemFormContext,
        projects: Vec<ProjectRef>,
        kind: Option<ItemKindId>,
        graph: Option<StepGraphId>,
        texts: &Texts<'_>,
    ) -> Self {
        let (title, priority, tags) = (
            TextField::with_text(texts.title),
            TextField::with_text(texts.priority),
            TextField::with_text(texts.tags),
        );
        let (paths, body) = (
            TextArea::with_text(texts.paths),
            TextArea::with_text(texts.body),
        );
        let opened = Opened {
            title: title.text().unwrap_or_default().to_owned(),
            priority: priority.text().unwrap_or_default().to_owned(),
            tags: tags.text().unwrap_or_default().to_owned(),
            paths: paths.text().to_owned(),
            body: body.text().to_owned(),
        };
        Self {
            projects,
            context,
            kind,
            graph,
            title,
            priority,
            tags,
            paths,
            body,
            opened,
            focus: Field::Title,
            busy: None,
            notice: None,
        }
    }

    /// E7: the reload landed. Kind and graph reset, text kept, busy cleared.
    pub fn retarget(&mut self, context: ItemFormContext) {
        self.kind = context.kinds.first().map(|first| first.id);
        self.graph = None;
        self.context = context;
        self.busy = None;
        self.notice = None;
    }

    /// What the form is waiting on, if anything.
    #[must_use]
    pub const fn busy(&self) -> Option<Busy> {
        self.busy
    }

    /// The edited item's id; `None` on a new form.
    #[must_use]
    pub fn item_id(&self) -> Option<ItemId> {
        self.context.item.as_ref().map(|item| item.id)
    }

    /// The project the form writes in.
    #[must_use]
    pub const fn project(&self) -> ProjectId {
        self.context.project
    }

    /// The focused field.
    #[must_use]
    pub const fn focus(&self) -> Field {
        self.focus
    }

    /// The notice line, if one is set.
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// A reply ended the flight: busy cleared, the notice set (or cleared with `None`).
    pub fn settle(&mut self, notice: Option<String>) {
        self.busy = None;
        self.notice = notice;
    }

    /// Feeds one key.
    ///
    /// In order: Ctrl+S saves (A6; swallowed while busy); any other chord passes; while busy
    /// everything else is swallowed, `Esc` included (D8); `Tab`/`BackTab` cycle the focus; then
    /// the focused field takes the key.
    pub fn on_key(&mut self, key: KeyEvent) -> ItemFormOutcome {
        if ctrl_s(&key) {
            return match self.busy {
                Some(_) => ItemFormOutcome::Stay,
                None => self.save(),
            };
        }
        if key.modifiers.intersects(filter::CHORD) {
            return ItemFormOutcome::Pass;
        }
        if self.busy.is_some() {
            return ItemFormOutcome::Stay;
        }
        match key.code {
            KeyCode::Tab => {
                self.move_focus(true);
                return ItemFormOutcome::Stay;
            }
            KeyCode::BackTab => {
                self.move_focus(false);
                return ItemFormOutcome::Stay;
            }
            _ => {}
        }
        match self.focus {
            Field::Project | Field::Kind | Field::Graph => self.on_picker_key(key),
            Field::Title | Field::Priority | Field::Tags => {
                let text = match self.focus {
                    Field::Title => &mut self.title,
                    Field::Priority => &mut self.priority,
                    _ => &mut self.tags,
                };
                match text.on_key(key) {
                    // D8: `Enter` moves on; only Ctrl+S saves.
                    FieldOutcome::Submit => self.move_focus(true),
                    FieldOutcome::Cancel => return ItemFormOutcome::Cancel,
                    FieldOutcome::Pass => match key.code {
                        KeyCode::Up => self.move_focus(false),
                        KeyCode::Down => self.move_focus(true),
                        _ => {}
                    },
                    FieldOutcome::Consumed => {}
                }
                ItemFormOutcome::Stay
            }
            Field::Paths | Field::Body => {
                let area = if self.focus == Field::Paths {
                    &mut self.paths
                } else {
                    &mut self.body
                };
                match area.on_key(key, PAGE) {
                    FieldOutcome::Cancel => ItemFormOutcome::Cancel,
                    FieldOutcome::Consumed | FieldOutcome::Submit | FieldOutcome::Pass => {
                        ItemFormOutcome::Stay
                    }
                }
            }
        }
    }

    /// Into the focused text widget; dropped on a picker or while busy.
    pub fn on_paste(&mut self, text: &str) {
        if self.busy.is_some() {
            return;
        }
        match self.focus {
            // An unmasked field never refuses a paste.
            Field::Title => _ = self.title.on_paste(text),
            Field::Priority => _ = self.priority.on_paste(text),
            Field::Tags => _ = self.tags.on_paste(text),
            Field::Paths => self.paths.on_paste(text),
            Field::Body => self.body.on_paste(text),
            Field::Project | Field::Kind | Field::Graph => {}
        }
    }

    /// Moves the focus one field, wrapping.
    fn move_focus(&mut self, forward: bool) {
        let fields = self.fields();
        let at = fields
            .iter()
            .position(|field| *field == self.focus)
            .unwrap_or(0);
        self.focus = fields[wrap(at, fields.len(), forward)];
    }

    /// A key on a picker: `Left`/`h` and `Right`/`l` choose, with wrap; `Enter`/`Down` and `Up`
    /// move between fields; `Esc` cancels.
    fn on_picker_key(&mut self, key: KeyEvent) -> ItemFormOutcome {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => return self.pick(false),
            KeyCode::Right | KeyCode::Char('l') => return self.pick(true),
            KeyCode::Enter | KeyCode::Down => self.move_focus(true),
            KeyCode::Up => self.move_focus(false),
            KeyCode::Esc => return ItemFormOutcome::Cancel,
            _ => {}
        }
        ItemFormOutcome::Stay
    }

    /// Moves the focused picker one option. The project picker answers [`ItemFormOutcome::Reload`]
    /// and waits for the catalogue (E7); one with a single project has nowhere to go.
    fn pick(&mut self, forward: bool) -> ItemFormOutcome {
        match self.focus {
            Field::Project => {
                let at = self
                    .projects
                    .iter()
                    .position(|known| known.project_id == self.context.project)
                    .unwrap_or(0);
                let Some(next) = self
                    .projects
                    .get(wrap(at, self.projects.len(), forward))
                    .map(|known| known.project_id)
                else {
                    return ItemFormOutcome::Stay;
                };
                if next == self.context.project {
                    return ItemFormOutcome::Stay;
                }
                self.busy = Some(Busy::Reloading(next));
                return ItemFormOutcome::Reload(next);
            }
            Field::Kind => {
                let kinds: Vec<ItemKindId> =
                    self.context.kinds.iter().map(|kind| kind.id).collect();
                let at = kinds
                    .iter()
                    .position(|id| Some(*id) == self.kind)
                    .unwrap_or(0);
                if let Some(next) = kinds.get(wrap(at, kinds.len(), forward)) {
                    self.kind = Some(*next);
                }
            }
            Field::Graph => {
                let options = self.graph_options();
                let at = options.iter().position(|id| *id == self.graph).unwrap_or(0);
                self.graph = options[wrap(at, options.len(), forward)];
            }
            Field::Title | Field::Priority | Field::Tags | Field::Paths | Field::Body => {}
        }
        ItemFormOutcome::Stay
    }

    /// Ctrl+S: the checked request and the form busy with it, or the refusal in the notice and
    /// nothing sent.
    fn save(&mut self) -> ItemFormOutcome {
        match self.request() {
            Ok((request, busy)) => {
                self.busy = Some(busy);
                self.notice = None;
                ItemFormOutcome::Save(request)
            }
            Err(sentence) => {
                self.notice = Some(sentence);
                ItemFormOutcome::Stay
            }
        }
    }

    /// The request a save sends, checked against the form's catalogue (D4), with the busy state
    /// it puts the form in; or the sentence that refuses it.
    fn request(&self) -> Result<(StoreRequest, Busy), String> {
        let spec = self.typed_spec()?;
        let ctx = self.context.spec_context();
        match &self.context.item {
            None => {
                let spec = item_spec::check_spec(&spec, &ctx).map_err(|err| err.to_string())?;
                Ok((
                    StoreRequest::MintItem {
                        project: self.context.project,
                        spec,
                    },
                    Busy::Minting,
                ))
            }
            Some(item) => {
                let changes = SpecChanges::between(&ItemSpec::of(item), &spec);
                if changes.is_empty() {
                    return Err(NOTHING_TO_SAVE.to_owned());
                }
                let changes =
                    item_spec::check_changes(&changes, &ctx).map_err(|err| err.to_string())?;
                // D6: the version the form's text came from, never moved.
                Ok((
                    StoreRequest::EditItem {
                        id: item.id,
                        expected_version: item.version,
                        changes,
                    },
                    Busy::Editing,
                ))
            }
        }
    }

    /// The spec the widgets hold. On an edit, a widget whose text is what it opened with keeps
    /// the stored value unparsed (A4); every other field is parsed, the body taken as typed.
    fn typed_spec(&self) -> Result<ItemSpec, String> {
        let kind_id = self.kind.ok_or_else(|| NO_KINDS.to_owned())?;
        self.parsed(kind_id).map_err(|err| err.to_string())
    }

    /// [`typed_spec`](Self::typed_spec) past the kind: the text fields, kept or parsed.
    fn parsed(&self, kind_id: ItemKindId) -> Result<ItemSpec, SpecError> {
        let base = self.context.item.as_ref().map(ItemSpec::of);
        let base = base.as_ref();
        let opened = &self.opened;
        Ok(ItemSpec {
            kind_id,
            title: field(
                base.map(|base| &base.title),
                self.title.text().unwrap_or_default(),
                &opened.title,
                item_spec::parse_title,
            )?,
            body: field(
                base.map(|base| &base.body),
                self.body.text(),
                &opened.body,
                |text| Ok(text.to_owned()),
            )?,
            priority: field(
                base.map(|base| &base.priority),
                self.priority.text().unwrap_or_default(),
                &opened.priority,
                item_spec::parse_priority,
            )?,
            required_tags: field(
                base.map(|base| &base.required_tags),
                self.tags.text().unwrap_or_default(),
                &opened.tags,
                item_spec::parse_tags,
            )?,
            touched_paths: field(
                base.map(|base| &base.touched_paths),
                self.paths.text(),
                &opened.paths,
                |text| Ok(item_spec::parse_paths(text)),
            )?,
            step_graph_id: self.graph,
        })
    }

    /// The fields this form has, in `Tab` order: an edit has no project picker.
    fn fields(&self) -> &'static [Field] {
        if self.context.item.is_some() {
            &ORDER[1..]
        } else {
            &ORDER
        }
    }

    /// The graph picker's options: the kind default, the project's graphs, then on an edit the
    /// item's own graph when it is not among them (an override, A3).
    fn graph_options(&self) -> Vec<Option<StepGraphId>> {
        let mut options = vec![None];
        options.extend(self.context.graphs.iter().map(|graph| Some(graph.id)));
        if let Some(own) = self
            .context
            .item
            .as_ref()
            .and_then(|item| item.step_graph_id)
            && !options.contains(&Some(own))
        {
            options.push(Some(own));
        }
        options
    }

    /// The project the picker draws: the one being read while reloading, else the form's.
    fn shown_project(&self) -> ProjectId {
        match self.busy {
            Some(Busy::Reloading(project)) => project,
            _ => self.context.project,
        }
    }

    /// A project's slug from the snapshot, or `?` for one it lacks.
    fn slug(&self, project: ProjectId) -> &str {
        self.projects
            .iter()
            .find(|known| known.project_id == project)
            .map_or("?", |known| known.slug.as_str())
    }

    /// What a picker shows between its `‹ ›`.
    fn picker_label(&self, field: Field) -> String {
        match field {
            Field::Project => self.slug(self.shown_project()).to_owned(),
            Field::Kind => match self.kind {
                None => "(none)".to_owned(),
                Some(id) => self
                    .context
                    .kinds
                    .iter()
                    .find(|kind| kind.id == id)
                    .map_or_else(
                        || "?".to_owned(),
                        |kind| format!("{} {}", kind.prefix, kind.name),
                    ),
            },
            Field::Graph => match self.graph {
                None => "kind default".to_owned(),
                Some(id) => self
                    .context
                    .graphs
                    .iter()
                    .find(|graph| graph.id == id)
                    .map_or_else(
                        || "override (this item's)".to_owned(),
                        |graph| graph.name.clone(),
                    ),
            },
            Field::Title | Field::Priority | Field::Tags | Field::Paths | Field::Body => {
                String::new()
            }
        }
    }

    /// One focused row as a line of at most `width` cells: the marker, the label, the value.
    fn row_line(&self, field: Field, width: u16, theme: &Theme) -> Line<'static> {
        let focused = self.focus == field;
        let label = match field {
            Field::Project => "project",
            Field::Kind => "kind",
            Field::Title => "title",
            Field::Priority => "priority",
            Field::Tags => "tags",
            Field::Graph => "graph",
            Field::Paths => "paths",
            Field::Body => "body",
        };
        let head = format!("{}{label:<LABEL$}", marker(focused));
        let head_style = if focused { theme.title } else { theme.dim };
        let all = usize::from(width);
        let room = all.saturating_sub(head.chars().count());
        let mut spans = vec![Span::styled(clip(&head, all), head_style)];
        let text = match field {
            Field::Title => Some(&self.title),
            Field::Priority => Some(&self.priority),
            Field::Tags => Some(&self.tags),
            Field::Project | Field::Kind | Field::Graph | Field::Paths | Field::Body => None,
        };
        match text {
            Some(text) => {
                let room = u16::try_from(room).unwrap_or(u16::MAX);
                spans.extend(text.line(room, focused, theme).spans);
            }
            None => {
                let value = format!("\u{2039}{}\u{203a}", self.picker_label(field));
                spans.push(Span::styled(clip(&value, room), theme.base));
            }
        }
        Line::from(spans)
    }
}

/// The two-cell focus marker, so focus shows in a text snapshot.
const fn marker(focused: bool) -> &'static str {
    if focused { "> " } else { "  " }
}

/// Draws the form into the detail pane's `area` (D8).
pub fn render(frame: &mut Frame<'_>, area: Rect, form: &ItemForm, theme: &Theme) {
    let title = match &form.context.item {
        Some(item) => format!(" Edit {} (v{}) ", item.key, item.version),
        None => format!(" New item \u{b7} {} ", form.slug(form.context.project)),
    };
    let block = Block::new().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Field> = form
        .fields()
        .iter()
        .copied()
        .filter(|field| !matches!(field, Field::Paths | Field::Body))
        .collect();
    let mut constraints: Vec<Constraint> = rows.iter().map(|_| Constraint::Length(1)).collect();
    constraints.extend([
        Constraint::Length(1),
        Constraint::Length(PATHS_HEIGHT),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(NOTICE_HEIGHT),
        Constraint::Length(1),
    ]);
    let areas = Layout::vertical(constraints).split(inner);
    let width = inner.width;
    for (field, at) in rows.iter().zip(areas.iter()) {
        frame.render_widget(Paragraph::new(form.row_line(*field, width, theme)), *at);
    }
    let rest = &areas[rows.len()..];
    let [paths_label, paths, body_label, body, notice, hint] =
        [rest[0], rest[1], rest[2], rest[3], rest[4], rest[5]];
    let label = |field: Field, text: &str| {
        let focused = form.focus == field;
        let style = if focused { theme.title } else { theme.dim };
        Line::styled(
            clip(&format!("{}{text}", marker(focused)), usize::from(width)),
            style,
        )
    };
    frame.render_widget(
        Paragraph::new(label(Field::Paths, "paths (one per line)")),
        paths_label,
    );
    frame.render_widget(
        Paragraph::new(form.paths.lines(
            paths.width,
            paths.height,
            form.focus == Field::Paths,
            theme,
        )),
        paths,
    );
    frame.render_widget(Paragraph::new(label(Field::Body, "body")), body_label);
    frame.render_widget(
        Paragraph::new(
            form.body
                .lines(body.width, body.height, form.focus == Field::Body, theme),
        ),
        body,
    );
    if let Some(sentence) = &form.notice {
        frame.render_widget(
            Paragraph::new(Line::styled(sentence.clone(), theme.error)).wrap(Wrap { trim: true }),
            notice,
        );
    }
    let text = match form.focus {
        Field::Project | Field::Kind | Field::Graph => HINT_PICK,
        Field::Title | Field::Priority | Field::Tags | Field::Paths | Field::Body => HINT_TEXT,
    };
    frame.render_widget(
        Paragraph::new(Line::styled(clip(text, usize::from(width)), theme.dim)),
        hint,
    );
}

/// D6: a stale edit's notice. The text and the token are kept, so a second save diverges again.
#[must_use]
pub fn item_changed_elsewhere(head: i32) -> String {
    format!(
        "changed elsewhere \u{2014} now v{head}; your text is kept. Esc and `e` reopen on the head"
    )
}

/// D11: a mint's `Failed` that may have followed a COMMIT whose answer was lost.
#[must_use]
pub fn mint_may_have_landed(why: &str) -> String {
    format!(
        "{why} \u{2014} it may have been written; the list is being re-read, look for it before \
         Ctrl+S"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_writes;
    use crate::store_worker::StoreReply;
    use crate::testkit::DEFAULT_SIZE;
    use crate::ui::layout::chrome;
    use crate::ui::tabs::backlog::panes;
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemPatch, NewStepGraph};
    use htui_core::store::{MemStore, WriteStore as _};
    use htui_store::Backend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(form: &mut ItemForm, text: &str) {
        for c in text.chars() {
            form.on_key(key(KeyCode::Char(c)));
        }
    }

    /// The Platform workspace's two projects, as `ctx.projects` holds them.
    fn projects() -> Vec<ProjectRef> {
        vec![
            ProjectRef {
                project_id: ids::PROJECT_HTUI,
                slug: "htui".to_owned(),
                name: "htui".to_owned(),
                position: 0,
            },
            ProjectRef {
                project_id: ids::PROJECT_AGY,
                slug: "agy".to_owned(),
                name: "agy".to_owned(),
                position: 1,
            },
        ]
    }

    /// The worker's answer to `ItemForm { project, item }` over `store`.
    async fn context(
        store: &MemStore,
        project: ProjectId,
        item: Option<ItemId>,
    ) -> ItemFormContext {
        let backend = Backend::memory(store.clone());
        match item_writes::serve(&backend, &StoreRequest::ItemForm { project, item }).await {
            Ok(StoreReply::ItemForm(context)) => *context,
            other => panic!("the form read answered {other:?}"),
        }
    }

    async fn new_form() -> ItemForm {
        let store = MemStore::demo();
        ItemForm::open_new(
            context(&store, ids::PROJECT_HTUI, None).await,
            &projects(),
            None,
        )
    }

    async fn edit_form(store: &MemStore, id: ItemId) -> ItemForm {
        ItemForm::open_edit(context(store, ids::PROJECT_HTUI, Some(id)).await)
            .expect("an edit context opens a form")
    }

    /// A patch through the store itself, around the validator (A4's legacy rows).
    fn patch() -> ItemPatch {
        ItemPatch {
            author_id: ids::USER,
            reason: "elsewhere".to_owned(),
            ..ItemPatch::default()
        }
    }

    fn focus_on(form: &mut ItemForm, field: Field) {
        for _ in 0..ORDER.len() {
            if form.focus == field {
                return;
            }
            form.on_key(key(KeyCode::Tab));
        }
        assert_eq!(form.focus, field, "Tab reaches every field");
    }

    #[tokio::test]
    async fn a_new_form_opens_on_the_title_with_the_hinted_kind() {
        let store = MemStore::demo();
        let htui = context(&store, ids::PROJECT_HTUI, None).await;
        let form = ItemForm::open_new(htui.clone(), &projects(), Some(ids::KIND_HTUI_FEAT));
        assert_eq!(form.focus, Field::Title);
        assert_eq!(form.kind, Some(ids::KIND_HTUI_FEAT));
        assert_eq!(form.graph, None, "the kind default");
        assert_eq!(form.priority.text(), Some("0"));
        assert_eq!(form.title.text(), Some(""));
        assert_eq!(form.busy(), None);
        assert_eq!(form.item_id(), None);
        assert_eq!(form.project(), ids::PROJECT_HTUI);

        let unknown = ItemForm::open_new(htui.clone(), &projects(), Some(ids::KIND_AGY_FEAT));
        assert_eq!(
            unknown.kind,
            Some(htui.kinds[0].id),
            "another project's kind falls back to the first"
        );
    }

    #[tokio::test]
    async fn tab_cycles_every_field_and_the_edit_form_has_no_project() {
        let mut form = new_form().await;
        let mut seen = vec![form.focus];
        for _ in 0..ORDER.len() {
            form.on_key(key(KeyCode::Tab));
            seen.push(form.focus);
        }
        assert_eq!(
            seen,
            [
                Field::Title,
                Field::Priority,
                Field::Tags,
                Field::Graph,
                Field::Paths,
                Field::Body,
                Field::Project,
                Field::Kind,
                Field::Title,
            ]
        );
        form.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(form.focus, Field::Kind, "BackTab goes back");

        let store = MemStore::demo();
        let mut edit = edit_form(&store, ids::HTUI_ANA_1).await;
        let mut seen = vec![edit.focus];
        for _ in 0..ORDER.len() - 1 {
            edit.on_key(key(KeyCode::Tab));
            seen.push(edit.focus);
        }
        assert!(!seen.contains(&Field::Project), "{seen:?}");
        assert_eq!(seen.len(), 8);
        assert_eq!(seen.last(), Some(&Field::Title), "wrapped after seven");
    }

    #[tokio::test]
    async fn enter_on_a_text_field_moves_on_and_never_saves() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        assert!(matches!(
            form.on_key(key(KeyCode::Enter)),
            ItemFormOutcome::Stay
        ));
        assert_eq!(form.focus, Field::Priority);
        assert_eq!(form.busy(), None, "nothing went out");
        form.on_key(key(KeyCode::Down));
        assert_eq!(form.focus, Field::Tags, "Down moves on too");
        form.on_key(key(KeyCode::Up));
        assert_eq!(form.focus, Field::Priority, "and Up back");
    }

    #[tokio::test]
    async fn ctrl_s_on_a_blank_title_refuses_in_the_notice() {
        let mut form = new_form().await;
        type_text(&mut form, "   ");
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(
            form.notice(),
            Some(SpecError::BlankTitle.to_string().as_str())
        );
        assert_eq!(form.busy(), None);
    }

    #[tokio::test]
    async fn an_unchanged_edit_says_nothing_to_save() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(form.notice(), Some(NOTHING_TO_SAVE));
        assert_eq!(form.busy(), None);
    }

    #[tokio::test]
    async fn a_changed_title_saves_only_the_title_at_the_opened_version() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        let title = format!("{} v2", form.opened.title);
        type_text(&mut form, " v2");
        let outcome = form.on_key(ctrl('s'));
        assert!(
            matches!(
                &outcome,
                ItemFormOutcome::Save(StoreRequest::EditItem {
                    id,
                    expected_version: 1,
                    changes,
                }) if *id == ids::HTUI_ANA_1
                    && *changes == SpecChanges { title: Some(title.clone()), ..SpecChanges::default() }
            ),
            "{outcome:?}"
        );
        assert_eq!(form.busy(), Some(Busy::Editing));
    }

    #[tokio::test]
    async fn reformatted_tags_are_not_a_change() {
        let store = MemStore::demo();
        store
            .update_item(
                ids::HTUI_ANA_1,
                1,
                ItemPatch {
                    required_tags: Some(vec!["docker".to_owned(), "rust".to_owned()]),
                    ..patch()
                },
            )
            .await
            .expect("tag the item");
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        assert_eq!(form.tags.text(), Some("docker, rust"));
        form.tags = TextField::with_text("rust,docker");
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(form.notice(), Some(NOTHING_TO_SAVE));
    }

    #[tokio::test]
    async fn a_legacy_tag_does_not_block_a_title_edit() {
        let store = MemStore::demo();
        store
            .update_item(
                ids::HTUI_ANA_1,
                1,
                ItemPatch {
                    required_tags: Some(vec!["Rust".to_owned()]),
                    touched_paths: Some(vec!["nope:x".to_owned()]),
                    ..patch()
                },
            )
            .await
            .expect("a legacy row");
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        type_text(&mut form, " fixed");
        let outcome = form.on_key(ctrl('s'));
        assert!(
            matches!(
                &outcome,
                ItemFormOutcome::Save(StoreRequest::EditItem {
                    expected_version: 2,
                    changes,
                    ..
                }) if changes.title.is_some()
                    && changes.required_tags.is_none()
                    && changes.touched_paths.is_none()
            ),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_bad_path_is_refused_by_name_before_anything_is_sent() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        focus_on(&mut form, Field::Paths);
        type_text(&mut form, "nope:src");
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        let notice = form.notice().expect("a refusal");
        assert!(notice.contains("`nope`"), "{notice}");
        assert_eq!(form.busy(), None);
    }

    #[tokio::test]
    async fn moving_the_project_picker_reloads_and_retarget_keeps_the_text() {
        let store = MemStore::demo();
        let mut form = ItemForm::open_new(
            context(&store, ids::PROJECT_HTUI, None).await,
            &projects(),
            None,
        );
        type_text(&mut form, "Fresh item");
        focus_on(&mut form, Field::Kind);
        form.on_key(key(KeyCode::Right));
        focus_on(&mut form, Field::Project);
        assert_eq!(form.picker_label(Field::Project), "htui");

        let outcome = form.on_key(key(KeyCode::Char('l')));
        assert!(
            matches!(outcome, ItemFormOutcome::Reload(project) if project == ids::PROJECT_AGY),
            "{outcome:?}"
        );
        assert_eq!(form.busy(), Some(Busy::Reloading(ids::PROJECT_AGY)));
        assert_eq!(form.picker_label(Field::Project), "agy", "drawn while read");
        assert!(matches!(
            form.on_key(key(KeyCode::Char('h'))),
            ItemFormOutcome::Stay
        ));
        assert_eq!(form.busy(), Some(Busy::Reloading(ids::PROJECT_AGY)));

        let agy = context(&store, ids::PROJECT_AGY, None).await;
        let first = agy.kinds[0].id;
        form.retarget(agy);
        assert_eq!(form.busy(), None);
        assert_eq!(form.project(), ids::PROJECT_AGY);
        assert_eq!(form.kind, Some(first), "the kind resets");
        assert_eq!(form.graph, None);
        assert_eq!(form.title.text(), Some("Fresh item"), "the text is kept");
    }

    #[tokio::test]
    async fn busy_swallows_plain_keys_esc_and_ctrl_s_but_passes_ctrl_c() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let outcome = form.on_key(ctrl('s'));
        assert!(
            matches!(
                &outcome,
                ItemFormOutcome::Save(StoreRequest::MintItem { project, spec })
                    if *project == ids::PROJECT_HTUI && spec.title == "Fresh item"
            ),
            "{outcome:?}"
        );
        assert_eq!(form.busy(), Some(Busy::Minting));

        for code in [
            KeyCode::Char('x'),
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::Enter,
        ] {
            assert!(
                matches!(form.on_key(key(code)), ItemFormOutcome::Stay),
                "{code:?}"
            );
        }
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert!(matches!(form.on_key(ctrl('c')), ItemFormOutcome::Pass));
        assert_eq!(form.title.text(), Some("Fresh item"));
        assert_eq!(form.focus, Field::Title);

        form.settle(Some("why".to_owned()));
        assert_eq!(form.busy(), None);
        assert_eq!(form.notice(), Some("why"));
        assert!(matches!(
            form.on_key(key(KeyCode::Esc)),
            ItemFormOutcome::Cancel
        ));
    }

    #[tokio::test]
    async fn a_paste_lands_in_the_focused_text_field_and_not_on_a_picker() {
        let mut form = new_form().await;
        form.on_paste("Pasted\n");
        assert_eq!(form.title.text(), Some("Pasted"));
        focus_on(&mut form, Field::Kind);
        let kind = form.kind;
        form.on_paste("ANA");
        assert_eq!(form.kind, kind);
        assert_eq!(form.title.text(), Some("Pasted"));
        focus_on(&mut form, Field::Body);
        form.on_paste("one\r\ntwo");
        assert_eq!(form.body.text(), "one\ntwo");
    }

    #[tokio::test]
    async fn the_graph_picker_offers_kind_default_project_graphs_and_keeps_an_override() {
        let store = MemStore::demo();
        let htui = context(&store, ids::PROJECT_HTUI, None).await;
        let mut form = ItemForm::open_new(htui.clone(), &projects(), None);
        let mut expected = vec![None];
        expected.extend(htui.graphs.iter().map(|graph| Some(graph.id)));
        assert_eq!(form.graph_options(), expected);
        assert_eq!(form.picker_label(Field::Graph), "kind default");
        focus_on(&mut form, Field::Graph);
        form.on_key(key(KeyCode::Right));
        assert_eq!(form.graph, Some(htui.graphs[0].id));
        assert_eq!(form.picker_label(Field::Graph), htui.graphs[0].name);
        form.on_key(key(KeyCode::Left));
        form.on_key(key(KeyCode::Left));
        assert_eq!(form.graph, expected.last().copied().flatten(), "wraps");

        let clone = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "ANA-1 override".to_owned(),
                description: String::new(),
                is_override: true,
            })
            .await
            .expect("an override graph");
        store
            .update_item(
                ids::HTUI_ANA_1,
                1,
                ItemPatch {
                    step_graph_id: Some(Some(clone.id)),
                    ..patch()
                },
            )
            .await
            .expect("the item takes it");
        let edit = edit_form(&store, ids::HTUI_ANA_1).await;
        assert_eq!(edit.graph_options().last(), Some(&Some(clone.id)));
        assert_eq!(edit.graph_options().len(), expected.len() + 1);
        assert_eq!(edit.picker_label(Field::Graph), "override (this item's)");
    }

    #[test]
    fn the_hints_fit_the_detail_pane() {
        let (width, height) = DEFAULT_SIZE;
        let room = panes(chrome(Rect::new(0, 0, width, height)).body)[1].width - 2;
        for hint in [HINT_TEXT, HINT_PICK] {
            assert!(
                hint.chars().count() <= usize::from(room),
                "{hint:?} is {} columns against {room}",
                hint.chars().count()
            );
        }
    }

    #[tokio::test]
    async fn debug_prints_no_body() {
        let mut form = new_form().await;
        focus_on(&mut form, Field::Body);
        form.on_paste("SECRET-BODY");
        focus_on(&mut form, Field::Paths);
        form.on_paste("secret/dir/**");
        assert_eq!(form.body.text(), "SECRET-BODY");
        let printed = format!("{form:?}");
        assert!(!printed.contains("SECRET-BODY"), "{printed}");
        assert!(!printed.contains("secret/dir"), "{printed}");
        assert!(printed.contains("body_len"), "{printed}");
    }

    /// Every state shows as characters, because the snapshots are text (blueprint §4 render).
    #[tokio::test]
    async fn the_new_form_draws_its_rows_and_the_hint() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let [_, right] = panes(chrome(Rect::new(0, 0, DEFAULT_SIZE.0, DEFAULT_SIZE.1)).body);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
            right.width,
            right.height,
        ))
        .expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), &form, &Theme::default()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        let text: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for wanted in [
            " New item \u{b7} htui ",
            "> title    Fresh item",
            "  project  \u{2039}htui\u{203a}",
            "\u{2039}ANA ",
            "\u{2039}kind default\u{203a}",
            "paths (one per line)",
            HINT_TEXT,
        ] {
            assert!(text.contains(wanted), "{wanted:?} in\n{text}");
        }
    }
}
