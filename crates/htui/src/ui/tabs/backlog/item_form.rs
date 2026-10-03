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
//! - **A stale save opens the three-way view** (milestone 3 D3–D5, [`super::divergence`]).
//!   `m`/`t` rebase the form on the head, with a new token and reason `divergence_resolution`.
//!   `Esc` returns to it unchanged.
//! - **Ctrl+E hands Body or Paths to `$EDITOR`** (milestone 4 D1–D5): the form answers
//!   [`ItemFormOutcome::External`] and records the field; [`ItemForm::on_external_edit`] puts the
//!   text back, minus a final newline the editor added (D5). Saving is still Ctrl+S's
//!   compare-and-set.
//! - **The new form's project picker re-reads that project's catalogue** (blueprint E7): kinds,
//!   graphs and repos are per project. Keys are swallowed until the reply re-targets the form.
//!
//! Redaction (blueprint E10): the form's `Debug` prints lengths, never the body or the paths.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::item_spec::{self, NOTHING_TO_SAVE};
use htui_core::model::{
    EditReason, ItemId, ItemKindId, ItemSpec, ProjectId, ProjectRef, SpecChanges, SpecError,
    StepGraphId,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::divergence::{Divergence, ViewOutcome, rebased_on, still_behind};
use crate::editor::{
    EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, strip_added_newline,
    without_controls,
};
use crate::item_writes::{ItemDivergence, ItemFormContext};
use crate::store_worker::StoreRequest;
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::backlog::filter;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};

/// Lines `PageUp`/`PageDown` move in the paths and body areas.
pub const PAGE: u16 = 5;

/// The hint under a text field. It fits the detail pane's inner width at 100x30 (43 columns).
pub const HINT_TEXT: &str = "Tab field  Ctrl+S save  Esc cancel";

/// The hint under a picker. It fits the detail pane's inner width at 100x30 (43 columns).
pub const HINT_PICK: &str = "\u{2190}/\u{2192} choose  Tab field  Esc cancel";

/// The hint under the paths and body areas (milestone 4 D7): 39 columns against the detail
/// pane's 43, so `Tab field` gives way to Ctrl+E (Tab still cycles).
pub const HINT_AREA: &str = "Ctrl+E $EDITOR  Ctrl+S save  Esc cancel";

/// The refusal of a save in a project without item kinds: an item needs one.
pub const NO_KINDS: &str = "this project has no item kinds; Settings \u{2192} Kinds adds one";

/// Width of a row's label column, after the two-cell focus marker.
const LABEL: usize = 9;

/// Rows the paths area takes.
const PATHS_HEIGHT: u16 = 3;

/// Rows the notice takes at least: a notice (a refusal or `still_behind`) wraps to two at the
/// detail pane's width. A longer notice (a D11 hedge after a long store message) grows into the
/// body's rows.
const NOTICE_HEIGHT: u16 = 2;

/// Rows the body keeps when a long notice grows.
const BODY_MIN: u16 = 3;

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
    /// Ctrl+E on Body or Paths (milestone 4 D3): hand this text to `$EDITOR`. The outcome comes
    /// back through [`ItemForm::on_external_edit`]; the tab emits `Action::EditExternally`.
    External(ExternalEdit),
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
    /// The revision reason a save sends: `Edited`, or `DivergenceResolution` once rebased
    /// (milestone 3 D6).
    reason: EditReason,
    /// The open three-way view (milestone 3 D3): while `Some`, every key and paste goes to it,
    /// not the fields.
    resolving: Option<Divergence>,
    /// The field handed to `$EDITOR` (milestone 4 D3) until its outcome comes back. A field, never
    /// text, so `Debug` may print it.
    external: Option<Field>,
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
            .field("notice_len", &self.notice.as_ref().map(String::len))
            .field("body_len", &self.body.len())
            .field("paths_len", &self.paths.len())
            .field("reason", &self.reason)
            .field(
                "resolving",
                &self.resolving.as_ref().map(Divergence::head_version),
            )
            .field("external", &self.external)
            .finish_non_exhaustive()
    }
}

/// The text each widget opens with.
struct Texts {
    title: String,
    priority: String,
    tags: String,
    paths: String,
    body: String,
}

impl Texts {
    /// A new form's: priority `0`, the rest empty.
    fn blank() -> Self {
        Self {
            title: String::new(),
            priority: "0".to_owned(),
            tags: String::new(),
            paths: String::new(),
            body: String::new(),
        }
    }

    /// A stored spec's, through `tags_text`/`paths_text`.
    fn of(spec: &ItemSpec) -> Self {
        Self {
            title: spec.title.clone(),
            priority: spec.priority.to_string(),
            tags: item_spec::tags_text(&spec.required_tags),
            paths: item_spec::paths_text(&spec.touched_paths),
            body: spec.body.clone(),
        }
    }
}

/// Ctrl+S, with or without `SHIFT` (the Requirements tab's rule).
pub(super) fn ctrl_s(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Char('s' | 'S'))
}

/// Ctrl+E, with or without `SHIFT` (milestone 4 D1; the Templates editor's rule).
pub(super) fn ctrl_e(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Char('e' | 'E'))
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
        let texts = Texts::blank();
        Self::build(context, projects.to_vec(), kind, None, &texts, &texts)
    }

    /// `e`: every widget from the item's stored spec, through `tags_text`/`paths_text`, focus
    /// Title. `None` when `context.item` is `None`.
    #[must_use]
    pub fn open_edit(context: ItemFormContext) -> Option<Self> {
        let spec = ItemSpec::of(context.item.as_ref()?);
        let texts = Texts::of(&spec);
        Some(Self::build(
            context,
            Vec::new(),
            Some(spec.kind_id),
            spec.step_graph_id,
            &texts,
            &texts,
        ))
    }

    /// D5: the form rebased on the head after `m`/`t`. `opened` holds the head's texts, so A4's
    /// "unchanged keeps the stored value" now means "unchanged from the head"; the widgets hold
    /// `resolved`'s texts; kind and graph are `resolved`'s; the token is `head.version`; the reason
    /// `DivergenceResolution`; focus Title; the notice `rebased_on(head.version)`. `None` when
    /// `context.item` is `None` (`open_edit`'s rule).
    #[must_use]
    pub fn open_resolution(context: ItemFormContext, resolved: &ItemSpec) -> Option<Self> {
        let head = context.item.as_ref()?;
        let (opened, version) = (Texts::of(&ItemSpec::of(head)), head.version);
        let mut form = Self::build(
            context,
            Vec::new(),
            Some(resolved.kind_id),
            resolved.step_graph_id,
            &opened,
            &Texts::of(resolved),
        );
        form.reason = EditReason::DivergenceResolution;
        form.notice = Some(rebased_on(version));
        Some(form)
    }

    /// The form with its widgets over `shown`, and `opened` read back from throwaway widgets
    /// over `opened`, so the `\r` normalisation applies to both sides alike.
    fn build(
        context: ItemFormContext,
        projects: Vec<ProjectRef>,
        kind: Option<ItemKindId>,
        graph: Option<StepGraphId>,
        opened: &Texts,
        shown: &Texts,
    ) -> Self {
        let (title, priority, tags) = (
            TextField::with_text(&shown.title),
            TextField::with_text(&shown.priority),
            TextField::with_text(&shown.tags),
        );
        let (paths, body) = (
            TextArea::with_text(&shown.paths),
            TextArea::with_text(&shown.body),
        );
        let line = |text: &str| {
            TextField::with_text(text)
                .text()
                .unwrap_or_default()
                .to_owned()
        };
        let area = |text: &str| TextArea::with_text(text).text().to_owned();
        let opened = Opened {
            title: line(&opened.title),
            priority: line(&opened.priority),
            tags: line(&opened.tags),
            paths: area(&opened.paths),
            body: area(&opened.body),
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
            reason: EditReason::Edited,
            resolving: None,
            external: None,
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

    /// The compare-and-set token: `context.item`'s version; `None` on a new form.
    #[must_use]
    pub fn token(&self) -> Option<i32> {
        self.context.item.as_ref().map(|item| item.version)
    }

    /// The reason the next save sends.
    #[must_use]
    pub const fn reason(&self) -> EditReason {
        self.reason
    }

    /// The open three-way view, if any (the tab draws it over the whole area, D4).
    #[must_use]
    pub const fn resolving(&self) -> Option<&Divergence> {
        self.resolving.as_ref()
    }

    /// D1, D3: a divergence the tab matched to this form's save opens the view. `mine` is
    /// `typed_spec()`, which `busy` kept equal to what was sent. On an `Err` (unreachable: the
    /// sent spec passed it), the form settles with that sentence instead. Busy clears, the notice
    /// clears. A new form, which cannot be `Editing`, is left alone.
    pub fn open_divergence(&mut self, divergence: &ItemDivergence) {
        if self.context.item.is_none() {
            return;
        }
        self.busy = None;
        self.notice = None;
        let mine = match self.typed_spec() {
            Ok(mine) => mine,
            Err(sentence) => {
                self.notice = Some(sentence);
                return;
            }
        };
        if let Some(ancestor) = self.context.item.as_ref() {
            self.resolving = Some(Divergence::open(ancestor, &self.context, mine, divergence));
        }
    }

    /// A reply ended the flight: busy cleared, the notice set (or cleared with `None`).
    pub fn settle(&mut self, notice: Option<String>) {
        self.busy = None;
        self.notice = notice;
    }

    /// D4: the `$EDITOR` handoff came back. No field out (`external` is `None`): ignored.
    /// `Edited` replaces only the handed-out field's text (control characters dropped, then D5),
    /// puts the cursor at its end, focuses it and says `EDITED`;
    /// the token, reason, `opened` and every other field are untouched, so A4's "unchanged" still
    /// compares against what the form opened with. `Unchanged` says `NO_CHANGES` (+ `WAIT_FLAG`
    /// when quick); `Failed` says its sentence; neither touches the text.
    pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome) {
        let Some(field) = self.external.take() else {
            return;
        };
        match outcome {
            ExternalEditOutcome::Edited(returned) => {
                // Unreachable `None`: only an area is handed out. Logged (the field, never text)
                // so a future non-area handoff shows.
                let Some((area, _)) = self.area_mut(field) else {
                    tracing::debug!(?field, "an $EDITOR outcome for a field that is not an area");
                    return;
                };
                // Filtered first (review L2), so a return that differs only by control characters
                // reads as unchanged. D5's baseline is the widget: nothing reaches the form between
                // the handoff and its outcome (the event loop runs them back to back).
                let returned = without_controls(&returned);
                let text = strip_added_newline(area.text(), &returned);
                if text == area.text() {
                    self.notice = Some(NO_CHANGES.to_owned());
                } else {
                    *area = TextArea::with_text(text);
                    // Review L4: at the end, so a long body's edited tail is what the area shows.
                    area.set_cursor(usize::MAX);
                    self.focus = field;
                    self.notice = Some(EDITED.to_owned());
                }
            }
            ExternalEditOutcome::Unchanged { quick } => {
                let wait = if quick { WAIT_FLAG } else { "" };
                self.notice = Some(format!("{NO_CHANGES}{wait}"));
            }
            ExternalEditOutcome::Failed(message) => self.notice = Some(message),
        }
    }

    /// The text area behind `field` and its part of the temp-file stem (D3): Paths or Body;
    /// `None` for a one-line field or a picker.
    fn area_mut(&mut self, field: Field) -> Option<(&mut TextArea, &'static str)> {
        match field {
            Field::Paths => Some((&mut self.paths, "paths")),
            Field::Body => Some((&mut self.body, "body")),
            Field::Project
            | Field::Kind
            | Field::Title
            | Field::Priority
            | Field::Tags
            | Field::Graph => None,
        }
    }

    /// The temp-file stem (D3) for an area's `part`: `<key>-body` / `<key>-paths`, `new-body` /
    /// `new-paths` on a new form. `editor::run` sanitises it (D25).
    fn stem(&self, part: &str) -> String {
        let owner = self
            .context
            .item
            .as_ref()
            .map_or("new", |item| item.key.as_str());
        format!("{owner}-{part}")
    }

    /// Ctrl+E while idle (D1, D3): Body or Paths answer `External` with that field's text and
    /// record it in `external`; any other field swallows it (`Stay`). The notice is left alone,
    /// as the Templates handoff leaves it: the outcome always sets it.
    fn hand_off(&mut self) -> ItemFormOutcome {
        let field = self.focus;
        let Some((area, part)) = self.area_mut(field) else {
            return ItemFormOutcome::Stay;
        };
        let text = area.text().to_owned();
        let stem = self.stem(part);
        self.external = Some(field);
        ItemFormOutcome::External(ExternalEdit { text, stem })
    }

    /// Feeds one key.
    ///
    /// In order: an open divergence view takes every key, Ctrl+S and Ctrl+E included (milestone 3
    /// D5, review L3); Ctrl+S saves (A6; swallowed while busy); Ctrl+E hands Body or Paths to
    /// `$EDITOR` (milestone 4 D1, D2; swallowed on any other field and while busy); any other chord
    /// passes; while busy everything else is swallowed, `Esc` included (D8); `Tab`/`BackTab` cycle
    /// the focus; then the focused field takes the key.
    pub fn on_key(&mut self, key: KeyEvent) -> ItemFormOutcome {
        if self.resolving.is_some() {
            return self.on_view_key(key);
        }
        if ctrl_s(&key) {
            return match self.busy {
                Some(_) => ItemFormOutcome::Stay,
                None => self.save(),
            };
        }
        // Milestone 4 D1: before the chord pass, which would hand it to the tab.
        if ctrl_e(&key) {
            return match self.busy {
                Some(_) => ItemFormOutcome::Stay,
                None => self.hand_off(),
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

    /// Into the focused text widget; dropped on a picker, while busy or while the view is open.
    pub fn on_paste(&mut self, text: &str) {
        if self.busy.is_some() || self.resolving.is_some() {
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

    /// A key while the view is open (D5). `Esc` closes it and keeps the text, focus, token and
    /// reason; `m`/`t` rebase the form on the head with the resolved spec. Ctrl+E is swallowed
    /// here, not passed as a chord (review L3): no editor opens over the view, and the key never
    /// reaches a global binding from inside the form (milestone 4 D1).
    fn on_view_key(&mut self, key: KeyEvent) -> ItemFormOutcome {
        let Some(view) = self.resolving.as_mut() else {
            return ItemFormOutcome::Stay;
        };
        if ctrl_e(&key) {
            return ItemFormOutcome::Stay;
        }
        match view.on_key(key) {
            ViewOutcome::Stay => ItemFormOutcome::Stay,
            ViewOutcome::Pass => ItemFormOutcome::Pass,
            ViewOutcome::Back => {
                self.notice = Some(still_behind(view.head_version()));
                self.resolving = None;
                ItemFormOutcome::Stay
            }
            ViewOutcome::Resolve(side) => {
                if let Some(view) = self.resolving.take() {
                    let (context, resolved) = view.resolve(side);
                    if let Some(form) = Self::open_resolution(context, &resolved) {
                        *self = form;
                    }
                }
                ItemFormOutcome::Stay
            }
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
                // The version the form's text came from: the opened item's, or the head's once
                // rebased (milestone 3 D5), with the matching reason (D6).
                Ok((
                    StoreRequest::EditItem {
                        id: item.id,
                        expected_version: item.version,
                        changes,
                        reason: self.reason,
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
        let room = all.saturating_sub(cell_width(&head));
        let mut spans = vec![Span::styled(cells::clip(&head, all), head_style)];
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
                spans.push(Span::styled(cells::clip(&value, room), theme.base));
            }
        }
        Line::from(spans)
    }
}

/// The two-cell focus marker, so focus shows in a text snapshot.
pub(super) const fn marker(focused: bool) -> &'static str {
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
    // The notice in rows of at most the pane's width in cells: broken at spaces, and inside a word
    // only when the word alone is wider (a long path in a refusal). Wrapped here rather than by
    // `Paragraph`'s `Wrap`, because the layout sizes the notice from this count before drawing it
    // (the `settings::wrapped` precedent), and a count that disagreed would cut off the D11 hedge.
    let mut notice_lines = form
        .notice
        .as_deref()
        .map(|sentence| wrapped(sentence, usize::from(inner.width)))
        .unwrap_or_default();
    // Every row but the body and the notice: the one-line rows, the paths and their label, the
    // body label and the hint.
    let fixed = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .saturating_add(PATHS_HEIGHT + 3);
    // The notice grows into the body's rows when it has to (a small terminal), and a notice
    // longer than all of them loses its first rows: the D11 hedge ends the sentence, and it is
    // what stops a second Ctrl+S from minting a duplicate.
    let room = inner.height.saturating_sub(fixed);
    let wanted = u16::try_from(notice_lines.len()).unwrap_or(u16::MAX);
    let notice_height = wanted.min(room).max(NOTICE_HEIGHT);
    let body_min = BODY_MIN.min(room.saturating_sub(notice_height));
    if wanted > notice_height {
        notice_lines.drain(..usize::from(wanted - notice_height));
    }
    let mut constraints: Vec<Constraint> = rows.iter().map(|_| Constraint::Length(1)).collect();
    constraints.extend([
        Constraint::Length(1),
        Constraint::Length(PATHS_HEIGHT),
        Constraint::Length(1),
        Constraint::Min(body_min),
        Constraint::Length(notice_height),
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
            cells::clip(&format!("{}{text}", marker(focused)), usize::from(width)),
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
    frame.render_widget(
        Paragraph::new(
            notice_lines
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect::<Vec<_>>(),
        ),
        notice,
    );
    let text = match form.focus {
        Field::Project | Field::Kind | Field::Graph => HINT_PICK,
        Field::Title | Field::Priority | Field::Tags => HINT_TEXT,
        Field::Paths | Field::Body => HINT_AREA,
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            cells::clip(text, usize::from(width)),
            theme.dim,
        )),
        hint,
    );
}

/// D11: a mint's `Failed` that may have followed a COMMIT whose answer was lost.
#[must_use]
pub fn mint_may_have_landed(why: &str) -> String {
    format!(
        "{why} \u{2014} it may have been written; the list is being re-read, look for it before \
         Ctrl+S"
    )
}

/// Review L4: the same hedge for a mint whose form a scope change closed. Nothing is re-read and
/// no form is open, so it promises neither; it points at the scope the mint was sent to.
#[must_use]
pub fn mint_may_have_landed_in_the_old_scope(why: &str) -> String {
    format!(
        "{why} \u{2014} the new item may have been written in the scope you left; look for it \
         there before creating it again"
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
    use htui_core::model::{ItemPatch, NewItemKind, NewStepGraph};
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
                    reason: EditReason::Edited,
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
        for hint in [HINT_TEXT, HINT_PICK, HINT_AREA] {
            assert!(
                cell_width(hint) <= usize::from(room),
                "{hint:?} is {} columns against {room}",
                cell_width(hint)
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

    /// A path refusal's notice names the entry, so `Debug` prints the notice's length only (E10).
    #[tokio::test]
    async fn debug_prints_no_path_from_a_refusal_notice() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        focus_on(&mut form, Field::Paths);
        type_text(&mut form, "nope:secret/dir");
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert!(
            form.notice()
                .is_some_and(|notice| notice.contains("secret/dir"))
        );
        let printed = format!("{form:?}");
        assert!(!printed.contains("secret/dir"), "{printed}");
        assert!(printed.contains("notice_len"), "{printed}");
    }

    /// The form drawn into the detail pane at the default size, one string per row.
    fn drawn(form: &ItemForm) -> Vec<String> {
        drawn_at(form, DEFAULT_SIZE)
    }

    /// The form drawn into the detail pane of a `(width, height)` terminal, one string per row.
    fn drawn_at(form: &ItemForm, (width, height): (u16, u16)) -> Vec<String> {
        let [_, right] = panes(chrome(Rect::new(0, 0, width, height)).body);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
            right.width,
            right.height,
        ))
        .expect("a test terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), form, &Theme::default()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Every state shows as characters, because the snapshots are text (blueprint §4 render).
    #[tokio::test]
    async fn the_new_form_draws_its_rows_and_the_hint() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let text = drawn(&form).join("\n");
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

    /// D11: a Postgres `Unreachable` message runs past two rows, and the hedge after it still
    /// shows whole, because the warning is what stops a second Ctrl+S from minting a duplicate.
    #[tokio::test]
    async fn the_hedge_after_a_long_failure_shows_whole() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let why = "store unreachable: error communicating with database: Connection reset by \
                   peer (os error 104)";
        let hedge = mint_may_have_landed(why);
        form.settle(Some(hedge.clone()));
        let rows = drawn(&form);
        let flowed = rows
            .iter()
            .map(|row| row.trim_matches(|c| c == ' ' || c == '\u{2502}'))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(flowed.contains(&hedge), "{hedge:?} in\n{}", rows.join("\n"));
        assert!(flowed.contains(HINT_TEXT), "{}", rows.join("\n"));
    }

    /// The notice's rows drawn at `size`, joined back into one sentence.
    fn flowed_at(form: &ItemForm, size: (u16, u16)) -> (String, String) {
        let rows = drawn_at(form, size);
        let flowed = rows
            .iter()
            .map(|row| row.trim_matches(|c| c == ' ' || c == '\u{2502}'))
            .collect::<Vec<_>>()
            .join(" ");
        (flowed, rows.join("\n"))
    }

    /// D11 on a standard 80x24 terminal: the notice takes the body's rows rather than lose the
    /// hedge, because the hedge is the end of the sentence and the rows past the room are cut.
    #[tokio::test]
    async fn the_hedge_shows_whole_on_an_80_by_24_terminal() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let why = "store unreachable: error communicating with database: Connection reset by \
                   peer (os error 104)";
        let hedge = mint_may_have_landed(why);
        form.settle(Some(hedge.clone()));
        let (flowed, rows) = flowed_at(&form, (80, 24));
        assert!(flowed.contains(&hedge), "{hedge:?} in\n{rows}");
    }

    /// A failure too long for the pane loses its start, never the hedge at its end (D11).
    #[tokio::test]
    async fn a_failure_too_long_for_the_pane_keeps_the_hedge() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        let why = format!(
            "store unreachable: {}",
            "connection reset by peer ".repeat(12)
        );
        form.settle(Some(mint_may_have_landed(why.trim_end())));
        let tail = mint_may_have_landed("");
        let (flowed, rows) = flowed_at(&form, (80, 24));
        assert!(flowed.contains(tail.trim_start()), "{tail:?} in\n{rows}");
    }

    /// A word wider than the pane (a long path in a refusal) breaks inside, never off the edge.
    #[tokio::test]
    async fn a_notice_word_wider_than_the_pane_breaks_inside() {
        let mut form = new_form().await;
        let long = format!("touched path `{}` is bad", "a/".repeat(40));
        form.settle(Some(long.clone()));
        let rows = drawn(&form);
        let flowed = rows
            .iter()
            .map(|row| row.trim_matches(|c| c == ' ' || c == '\u{2502}'))
            .collect::<String>();
        assert!(flowed.contains(&"a/".repeat(40)), "{}", rows.join("\n"));
    }

    /// MOD-60 D1: a picker value is clipped in cells, so a CJK graph name stays inside the row.
    #[tokio::test]
    async fn a_wide_graph_name_is_clipped_to_the_row() {
        let store = MemStore::demo();
        let mut htui = context(&store, ids::PROJECT_HTUI, None).await;
        htui.graphs[0].name = "\u{6f22}".repeat(30);
        let id = htui.graphs[0].id;
        let mut form = ItemForm::open_new(htui, &projects(), None);
        form.graph = Some(id);
        let line = form.row_line(Field::Graph, 30, &Theme::default());
        let used: usize = line
            .spans
            .iter()
            .map(|span| cell_width(&span.content))
            .sum();
        assert!(used <= 30, "{line:?} against 30");
    }

    /// MOD-60 D5: a CJK notice wraps by cells, so its last character is on screen, not past the
    /// pane's edge.
    #[tokio::test]
    async fn a_wide_notice_wraps_by_cells_and_keeps_its_end() {
        let mut form = new_form().await;
        form.settle(Some(format!("{}\u{7d42}", "\u{6f22}".repeat(60))));
        let rows = drawn(&form);
        assert!(
            rows.iter().any(|row| row.contains('\u{7d42}')),
            "{}",
            rows.join("\n")
        );
    }

    /// MOD-60 B9: the notice wraps by clusters, so a run of combining-mark clusters exactly as
    /// wide as the pane fills one row. A per-char cut after the wrap (the deleted `notice_lines`)
    /// would halve it, and at an odd width strand an accent at the start of the next row.
    #[tokio::test]
    async fn a_combining_notice_fills_its_row_by_clusters() {
        let mut form = new_form().await;
        let [_, right] = panes(chrome(Rect::new(0, 0, DEFAULT_SIZE.0, DEFAULT_SIZE.1)).body);
        let inner = usize::from(right.width - 2);
        let accented = "e\u{301}".repeat(inner);
        form.settle(Some(accented.clone()));
        let rows = drawn(&form);
        assert!(
            rows.iter().any(|row| row.contains(&accented)),
            "{inner} accented cells in one row of\n{}",
            rows.join("\n")
        );
    }

    // ---- MOD-13 milestone 3: the divergence view -------------------------------------------

    /// Ctrl+S on `form`, its `Save` served over `store`: the divergence it answered.
    async fn diverged(store: &MemStore, form: &mut ItemForm) -> ItemDivergence {
        let outcome = form.on_key(ctrl('s'));
        let ItemFormOutcome::Save(request) = outcome else {
            panic!("a save: {outcome:?} ({:?})", form.notice())
        };
        let backend = Backend::memory(store.clone());
        match item_writes::serve(&backend, &request).await {
            Ok(StoreReply::ItemDiverged(divergence)) => *divergence,
            other => panic!("the stale save answered {other:?}"),
        }
    }

    /// The edit form on ANA-1, then `theirs` written through the store at v1.
    async fn behind(store: &MemStore, theirs: ItemPatch) -> ItemForm {
        let form = edit_form(store, ids::HTUI_ANA_1).await;
        store
            .update_item(ids::HTUI_ANA_1, 1, theirs)
            .await
            .expect("their edit");
        form
    }

    /// `behind`, ` mine` typed into the title, the stale save answered: the view is open.
    async fn resolving(store: &MemStore, theirs: ItemPatch) -> ItemForm {
        let mut form = behind(store, theirs).await;
        type_text(&mut form, " mine");
        let divergence = diverged(store, &mut form).await;
        form.open_divergence(&divergence);
        assert!(form.resolving().is_some(), "the view opened");
        assert_eq!(form.busy(), None);
        form
    }

    fn retitled(title: &str) -> ItemPatch {
        ItemPatch {
            title: Some(title.to_owned()),
            ..patch()
        }
    }

    /// The `EditItem` a Ctrl+S sent, or a panic naming what it answered.
    fn edit_sent(form: &mut ItemForm) -> (i32, SpecChanges, EditReason) {
        match form.on_key(ctrl('s')) {
            ItemFormOutcome::Save(StoreRequest::EditItem {
                expected_version,
                changes,
                reason,
                ..
            }) => (expected_version, changes, reason),
            other => panic!("an edit: {other:?} ({:?})", form.notice()),
        }
    }

    #[tokio::test]
    async fn open_resolution_opens_on_the_head_with_the_resolved_text() {
        let store = MemStore::demo();
        let mut form = behind(&store, retitled("Theirs")).await;
        type_text(&mut form, " mine");
        let divergence = diverged(&store, &mut form).await;
        let head = divergence.head.clone();
        let resolved = ItemSpec {
            title: "Resolved".to_owned(),
            ..ItemSpec::of(&head)
        };
        let rebased = ItemForm::open_resolution(divergence.context.clone(), &resolved)
            .expect("a context with an item");
        assert_eq!(rebased.token(), Some(head.version));
        assert_eq!(rebased.reason(), EditReason::DivergenceResolution);
        assert_eq!(rebased.notice(), Some(rebased_on(head.version).as_str()));
        assert_eq!(rebased.opened.title, head.title, "unchanged means the head");
        assert_eq!(rebased.title.text(), Some("Resolved"));
        assert_eq!(rebased.focus(), Field::Title);
        assert!(rebased.resolving().is_none());

        let mut same = ItemForm::open_resolution(divergence.context.clone(), &ItemSpec::of(&head))
            .expect("a context with an item");
        assert!(matches!(same.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(same.notice(), Some(NOTHING_TO_SAVE));

        let mut new = divergence.context;
        new.item = None;
        assert!(ItemForm::open_resolution(new, &resolved).is_none());
    }

    /// D2, D5: my title wins its conflict, their priority is kept, and the save is a resolution
    /// at the head's version.
    #[tokio::test]
    async fn m_takes_mine_for_conflicts_and_theirs_elsewhere() {
        let store = MemStore::demo();
        let mut form = resolving(
            &store,
            ItemPatch {
                title: Some("Theirs".to_owned()),
                priority: Some(7),
                ..patch()
            },
        )
        .await;
        assert_ne!(
            form.opened.priority, "7",
            "the ancestor's priority is not theirs"
        );
        let mine = format!("{} mine", form.opened.title);
        assert!(matches!(
            form.on_key(key(KeyCode::Char('m'))),
            ItemFormOutcome::Stay
        ));
        assert!(form.resolving().is_none());
        assert_eq!(form.priority.text(), Some("7"), "their priority is kept");
        let (version, changes, reason) = edit_sent(&mut form);
        assert_eq!(version, 2);
        assert_eq!(reason, EditReason::DivergenceResolution);
        assert_eq!(
            changes,
            SpecChanges {
                title: Some(mine),
                ..SpecChanges::default()
            }
        );
    }

    /// Blueprint §6 (MOD-13 review L4): a legacy head value never blocks the resolution save.
    /// Their edit wrote a tag and a path the parsers refuse (through the store, past the
    /// validator); `m` keeps them as the head stores them, and Ctrl+S sends my title only.
    #[tokio::test]
    async fn a_legacy_head_value_does_not_block_the_resolution_save() {
        let store = MemStore::demo();
        let mut form = resolving(
            &store,
            ItemPatch {
                required_tags: Some(vec!["Rust".to_owned()]),
                touched_paths: Some(vec!["nope:x".to_owned()]),
                ..patch()
            },
        )
        .await;
        let mine = format!("{} mine", form.opened.title);
        assert!(matches!(
            form.on_key(key(KeyCode::Char('m'))),
            ItemFormOutcome::Stay
        ));
        assert!(form.resolving().is_none());
        assert_eq!(form.tags.text(), Some("Rust"), "the head's legacy tag");
        let (version, changes, reason) = edit_sent(&mut form);
        assert_eq!(version, 2);
        assert_eq!(reason, EditReason::DivergenceResolution);
        assert_eq!(
            changes,
            SpecChanges {
                title: Some(mine),
                ..SpecChanges::default()
            }
        );
    }

    #[tokio::test]
    async fn t_takes_theirs_for_conflicts_and_keeps_my_other_changes() {
        let store = MemStore::demo();
        let mut form = behind(&store, retitled("Theirs")).await;
        type_text(&mut form, " mine");
        form.priority = TextField::with_text("9");
        let divergence = diverged(&store, &mut form).await;
        form.open_divergence(&divergence);
        form.on_key(key(KeyCode::Char('t')));
        assert_eq!(form.title.text(), Some("Theirs"));
        let (version, changes, reason) = edit_sent(&mut form);
        assert_eq!((version, reason), (2, EditReason::DivergenceResolution));
        assert_eq!(
            changes,
            SpecChanges {
                priority: Some(9),
                ..SpecChanges::default()
            }
        );
    }

    /// D8: `t` when every change of mine conflicted resolves to the head itself.
    #[tokio::test]
    async fn t_with_every_change_conflicting_has_nothing_to_save() {
        let store = MemStore::demo();
        let mut form = resolving(&store, retitled("Theirs")).await;
        form.on_key(key(KeyCode::Char('t')));
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(form.notice(), Some(NOTHING_TO_SAVE));
        assert_eq!(form.busy(), None);
    }

    /// E5: the head already holds my edit, so `m` resolves to it and there is nothing to save.
    #[tokio::test]
    async fn my_edit_already_in_the_head_has_nothing_to_save() {
        let store = MemStore::demo();
        let opened = edit_form(&store, ids::HTUI_ANA_1).await.opened.title;
        let mut form = resolving(&store, retitled(&format!("{opened} mine"))).await;
        let view = form.resolving().expect("the view");
        assert!(view.rows().is_empty(), "{view:?}");
        form.on_key(key(KeyCode::Char('m')));
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(form.notice(), Some(NOTHING_TO_SAVE));
    }

    /// D5: `Esc` closes the view and keeps everything; the next save is the same stale one.
    #[tokio::test]
    async fn esc_in_the_view_returns_to_the_form_unchanged() {
        let store = MemStore::demo();
        let mut form = resolving(&store, retitled("Theirs")).await;
        assert!(matches!(
            form.on_key(key(KeyCode::Esc)),
            ItemFormOutcome::Stay
        ));
        assert!(form.resolving().is_none());
        assert_eq!(form.token(), Some(1));
        assert_eq!(form.reason(), EditReason::Edited);
        assert!(
            form.title
                .text()
                .is_some_and(|text| text.ends_with(" mine")),
            "{:?}",
            form.title.text()
        );
        assert_eq!(form.notice(), Some(still_behind(2).as_str()));

        let (version, changes, reason) = edit_sent(&mut form);
        assert_eq!((version, reason), (1, EditReason::Edited));
        assert!(changes.title.is_some());
        form.settle(None);
        assert!(matches!(
            form.on_key(key(KeyCode::Esc)),
            ItemFormOutcome::Cancel
        ));
    }

    /// D7: the rebased form opens on the reply's catalogue, so a kind and a graph made after the
    /// form opened are labelled, not `?` or `override`.
    #[tokio::test]
    async fn the_rebased_pickers_label_a_kind_and_graph_only_the_new_catalogue_has() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        let graph = store
            .create_step_graph(NewStepGraph {
                id: StepGraphId::new(),
                project_id: ids::PROJECT_HTUI,
                name: "after the form".to_owned(),
                description: String::new(),
                is_override: false,
            })
            .await
            .expect("a graph");
        let kind = store
            .create_item_kind(NewItemKind {
                id: ItemKindId::new(),
                project_id: ids::PROJECT_HTUI,
                prefix: "NEW".to_owned(),
                name: "Newcomer".to_owned(),
                description: String::new(),
                default_graph_id: graph.id,
                position: 99,
            })
            .await
            .expect("a kind");
        store
            .update_item(
                ids::HTUI_ANA_1,
                1,
                ItemPatch {
                    kind_id: Some(kind.id),
                    step_graph_id: Some(Some(graph.id)),
                    ..patch()
                },
            )
            .await
            .expect("their move");
        type_text(&mut form, " mine");
        let divergence = diverged(&store, &mut form).await;
        form.open_divergence(&divergence);
        form.on_key(key(KeyCode::Char('t')));
        assert!(form.resolving().is_none());
        assert_eq!(form.picker_label(Field::Kind), "NEW Newcomer");
        assert_eq!(form.picker_label(Field::Graph), graph.name);
    }

    #[tokio::test]
    async fn the_view_swallows_letters_and_ctrl_s_but_passes_ctrl_c() {
        let store = MemStore::demo();
        let mut form = resolving(&store, retitled("Theirs")).await;
        let title = form.title.text().map(str::to_owned);
        for event in [
            key(KeyCode::Char('x')),
            key(KeyCode::Tab),
            key(KeyCode::Enter),
            ctrl('s'),
        ] {
            assert!(
                matches!(form.on_key(event), ItemFormOutcome::Stay),
                "{event:?}"
            );
        }
        assert!(matches!(form.on_key(ctrl('c')), ItemFormOutcome::Pass));
        assert!(form.resolving().is_some(), "still open");
        assert_eq!(form.busy(), None, "Ctrl+S sent nothing");
        assert_eq!(form.title.text().map(str::to_owned), title);
        assert_eq!(form.focus(), Field::Title);
    }

    #[tokio::test]
    async fn a_paste_while_the_view_is_open_is_dropped() {
        let store = MemStore::demo();
        let mut form = resolving(&store, retitled("Theirs")).await;
        let title = form.title.text().map(str::to_owned);
        form.on_paste("PASTED");
        assert_eq!(form.title.text().map(str::to_owned), title);
    }

    #[tokio::test]
    async fn debug_prints_no_body_while_resolving() {
        let store = MemStore::demo();
        let mut form = behind(
            &store,
            ItemPatch {
                body: Some("SECRET-THEIRS".to_owned()),
                // Through the store, around the validator: the demo project has no repo.
                touched_paths: Some(vec!["secret/dir/**".to_owned()]),
                ..patch()
            },
        )
        .await;
        form.body = TextArea::with_text("SECRET-BODY");
        let divergence = diverged(&store, &mut form).await;
        form.open_divergence(&divergence);
        assert!(form.resolving().is_some());
        for printed in [format!("{form:?}"), format!("{form:#?}")] {
            for secret in ["SECRET-BODY", "SECRET-THEIRS", "secret/dir"] {
                assert!(!printed.contains(secret), "{secret} in {printed}");
            }
            assert!(printed.contains("resolving"), "{printed}");
        }
    }

    // $EDITOR round-trip (milestone 4, plan D1-D8).

    /// Ctrl+E on `form`'s focused field: the `ExternalEdit` it answered, or a panic naming the
    /// outcome.
    fn handed(form: &mut ItemForm) -> ExternalEdit {
        match form.on_key(ctrl('e')) {
            ItemFormOutcome::External(edit) => edit,
            other => panic!("an external edit: {other:?} ({:?})", form.notice()),
        }
    }

    #[tokio::test]
    async fn ctrl_e_on_the_body_or_paths_hands_out_that_text_with_the_item_key_stem() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        focus_on(&mut form, Field::Body);
        assert_eq!(
            handed(&mut form),
            ExternalEdit {
                text: form.opened.body.clone(),
                stem: "ANA-1-body".to_owned(),
            }
        );
        assert_eq!(form.external, Some(Field::Body));
        assert_eq!(form.busy(), None);

        focus_on(&mut form, Field::Paths);
        form.on_paste("src/**");
        let outcome = form.on_key(KeyEvent::new(
            KeyCode::Char('E'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(
            matches!(
                &outcome,
                ItemFormOutcome::External(edit)
                    if edit.text == "src/**" && edit.stem == "ANA-1-paths"
            ),
            "{outcome:?}"
        );
        assert_eq!(form.external, Some(Field::Paths));
    }

    #[tokio::test]
    async fn ctrl_e_on_a_new_form_uses_the_new_stem() {
        let mut form = new_form().await;
        focus_on(&mut form, Field::Body);
        form.on_paste("draft");
        assert_eq!(
            handed(&mut form),
            ExternalEdit {
                text: "draft".to_owned(),
                stem: "new-body".to_owned(),
            }
        );
    }

    /// D1: a one-line field or a picker swallows Ctrl+E; it never passes to the tab.
    #[tokio::test]
    async fn ctrl_e_on_a_one_line_field_or_a_picker_is_swallowed() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        for field in [
            Field::Kind,
            Field::Title,
            Field::Priority,
            Field::Tags,
            Field::Graph,
        ] {
            focus_on(&mut form, field);
            let outcome = form.on_key(ctrl('e'));
            assert!(
                matches!(outcome, ItemFormOutcome::Stay),
                "{field:?}: {outcome:?}"
            );
            assert_eq!(form.external, None, "{field:?}");
            assert_eq!(form.focus(), field);
        }
        let mut form = new_form().await;
        focus_on(&mut form, Field::Project);
        let outcome = form.on_key(ctrl('e'));
        assert!(matches!(outcome, ItemFormOutcome::Stay), "{outcome:?}");
        assert_eq!(form.external, None);
        assert_eq!(form.focus(), Field::Project);
    }

    /// D2: while a save is in flight, Ctrl+E is swallowed, as Ctrl+S is.
    #[tokio::test]
    async fn ctrl_e_while_busy_is_swallowed() {
        let mut form = new_form().await;
        type_text(&mut form, "Fresh item");
        focus_on(&mut form, Field::Body);
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Save(_)));
        assert_eq!(form.busy(), Some(Busy::Minting));
        assert!(matches!(form.on_key(ctrl('e')), ItemFormOutcome::Stay));
        assert_eq!(form.external, None);
    }

    /// Review L3: the open three-way view swallows Ctrl+E, so it never reaches a global binding
    /// (D1's rule) and no editor opens over the view.
    #[tokio::test]
    async fn ctrl_e_in_the_divergence_view_is_swallowed() {
        let store = MemStore::demo();
        let mut form = resolving(&store, retitled("Theirs")).await;
        for event in [
            ctrl('e'),
            KeyEvent::new(
                KeyCode::Char('E'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            let outcome = form.on_key(event);
            assert!(matches!(outcome, ItemFormOutcome::Stay), "{outcome:?}");
        }
        assert!(form.resolving().is_some(), "still open");
        assert_eq!(form.external, None);
    }

    /// D4: only the body moves; the token, reason and other fields stay, so the save is the
    /// compare-and-set at the opened version with the body alone.
    #[tokio::test]
    async fn an_edited_body_replaces_only_the_body_and_saves_it_at_the_opened_version() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        focus_on(&mut form, Field::Body);
        handed(&mut form);
        // Stands in for the move D4 undoes; the loop never does it.
        form.focus = Field::Title;
        let (title, paths) = (
            form.title.text().map(str::to_owned),
            form.paths.text().to_owned(),
        );
        form.on_external_edit(ExternalEditOutcome::Edited("New body.\n".to_owned()));
        assert_eq!(
            form.body.text(),
            "New body.",
            "D5 dropped the editor's newline"
        );
        assert_eq!(
            form.body.cursor_line_col(),
            (0, 9),
            "review L4: the cursor ends the text"
        );
        assert_eq!(form.focus(), Field::Body);
        assert_eq!(form.notice(), Some(EDITED));
        assert_eq!(form.title.text().map(str::to_owned), title);
        assert_eq!(form.paths.text(), paths);
        assert_eq!(form.token(), Some(1));
        assert_eq!(form.reason(), EditReason::Edited);
        assert_eq!(form.external, None);
        assert_eq!(
            edit_sent(&mut form),
            (
                1,
                SpecChanges {
                    body: Some("New body.".to_owned()),
                    ..SpecChanges::default()
                },
                EditReason::Edited,
            )
        );
    }

    #[tokio::test]
    async fn an_edited_paths_area_lands_in_paths() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        let body = form.body.text().to_owned();
        focus_on(&mut form, Field::Paths);
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited("src/**\nlib/**\n".to_owned()));
        assert_eq!(form.paths.text(), "src/**\nlib/**");
        assert_eq!(
            form.paths.cursor_line_col(),
            (1, 6),
            "review L4: at the end"
        );
        assert_eq!(form.body.text(), body);
        assert_eq!(form.focus(), Field::Paths);
        assert_eq!(form.notice(), Some(EDITED));
    }

    /// A4: "unchanged" compares against what the form opened with, not what was handed out.
    #[tokio::test]
    async fn an_edit_back_to_the_opened_text_has_nothing_to_save() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        focus_on(&mut form, Field::Body);
        type_text(&mut form, "x");
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited(form.opened.body.clone()));
        assert_eq!(form.notice(), Some(EDITED));
        assert!(matches!(form.on_key(ctrl('s')), ItemFormOutcome::Stay));
        assert_eq!(form.notice(), Some(NOTHING_TO_SAVE));
    }

    #[tokio::test]
    async fn unchanged_and_failed_keep_the_text() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        focus_on(&mut form, Field::Body);
        let body = form.body.text().to_owned();
        for (outcome, notice) in [
            (
                ExternalEditOutcome::Unchanged { quick: true },
                format!("{NO_CHANGES}{WAIT_FLAG}"),
            ),
            (
                ExternalEditOutcome::Unchanged { quick: false },
                NO_CHANGES.to_owned(),
            ),
            (
                ExternalEditOutcome::Failed("boom".to_owned()),
                "boom".to_owned(),
            ),
        ] {
            handed(&mut form);
            form.on_external_edit(outcome);
            assert_eq!(form.notice(), Some(notice.as_str()));
            assert_eq!(form.body.text(), body);
            assert_eq!(form.external, None);
        }
    }

    /// D5 through the form: `:wq` on a body without a final newline is no change.
    #[tokio::test]
    async fn an_editor_added_final_newline_is_no_change() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        form.body = TextArea::with_text("abc");
        focus_on(&mut form, Field::Body);
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited("abc\n".to_owned()));
        assert_eq!(form.notice(), Some(NO_CHANGES));
        assert_eq!(form.body.text(), "abc");

        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited("abd\n".to_owned()));
        assert_eq!(form.body.text(), "abd");

        form.body = TextArea::with_text("abc\n");
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited("abc\n\n".to_owned()));
        assert_eq!(form.body.text(), "abc\n\n");
        assert_eq!(form.notice(), Some(EDITED));
    }

    /// Review L2: a control character from the editor is dropped, as a paste drops one; a `\t`
    /// is kept, because the area draws it to the next tab stop.
    #[tokio::test]
    async fn an_editor_return_loses_its_control_characters_and_keeps_its_tabs() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        form.body = TextArea::with_text("abc");
        focus_on(&mut form, Field::Body);
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited(
            "a\u{1b}[31mb\u{7}c\u{0}\n\tindented\u{7f}\n".to_owned(),
        ));
        assert_eq!(form.body.text(), "a[31mbc\n\tindented");
        assert_eq!(form.notice(), Some(EDITED));
    }

    /// Review L2: the filter runs before D5's compare, so a return that differs only by control
    /// characters (and the editor's final newline) is no change.
    #[tokio::test]
    async fn a_return_differing_only_by_control_characters_is_no_change() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        form.body = TextArea::with_text("abc");
        focus_on(&mut form, Field::Body);
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Edited("a\u{1b}bc\u{0}\n".to_owned()));
        assert_eq!(form.notice(), Some(NO_CHANGES));
        assert_eq!(form.body.text(), "abc");
    }

    #[tokio::test]
    async fn an_outcome_with_no_edit_out_is_ignored() {
        let store = MemStore::demo();
        let mut form = edit_form(&store, ids::HTUI_ANA_1).await;
        let body = form.body.text().to_owned();
        form.on_external_edit(ExternalEditOutcome::Edited("x".to_owned()));
        assert_eq!(form.body.text(), body);
        assert_eq!(form.notice(), None);

        focus_on(&mut form, Field::Body);
        handed(&mut form);
        form.on_external_edit(ExternalEditOutcome::Failed("one".to_owned()));
        form.on_external_edit(ExternalEditOutcome::Failed("two".to_owned()));
        assert_eq!(form.notice(), Some("one"), "the second had no field out");
    }

    /// D7: the areas hint Ctrl+E, the one-line fields keep `Tab field`.
    #[tokio::test]
    async fn the_body_and_paths_areas_hint_ctrl_e() {
        let mut form = new_form().await;
        for field in [Field::Body, Field::Paths] {
            focus_on(&mut form, field);
            let text = drawn(&form).join("\n");
            assert!(text.contains(HINT_AREA), "{field:?} in\n{text}");
            assert!(!text.contains("Tab field"), "{field:?} in\n{text}");
        }
        focus_on(&mut form, Field::Title);
        let text = drawn(&form).join("\n");
        assert!(text.contains(HINT_TEXT), "{text}");
    }

    #[tokio::test]
    async fn debug_prints_no_body_while_an_edit_is_out() {
        let mut form = new_form().await;
        focus_on(&mut form, Field::Body);
        form.on_paste("SECRET-BODY");
        let outcome = form.on_key(ctrl('e'));
        let printed = format!("{outcome:?}");
        assert!(printed.contains("text_len"), "{printed}");
        assert!(!printed.contains("SECRET-BODY"), "{printed}");
        for printed in [format!("{form:?}"), format!("{form:#?}")] {
            assert!(!printed.contains("SECRET-BODY"), "{printed}");
        }
        let printed = format!("{form:?}");
        assert!(printed.contains("external: Some(Body)"), "{printed}");
    }
}
