//! The Backlog filter (MOD-13 milestone 1): what narrows the list, and the form that edits it.
//!
//! [`BacklogFilter`] is the tab's state (D1): statuses, projects, required tags and "ready here".
//! It leaves as one [`StoreRequest::Items`], so the shell's staleness index sees one request kind
//! whether a filter is set or not (D2). The view never sets `ItemFilter::ready`; `ready_here` asks
//! the worker to compose ANA-9 §7.4 for this box.
//!
//! [`FilterForm`] is an inline, capturing panel at the bottom of the list pane (D1), not an
//! overlay: an overlay factory cannot be seeded with the current filter. It is pure — no `Ctx` —
//! and answers the tab with a [`FormOutcome`], as `requirements/forms.rs` does.
//!
//! MOD-67 M5: its keys resolve through [`views::BACKLOG_FILTER`]. The rows move with
//! `backlog.filter_form.next_row`/`prev_row` and `form.next_field`/`prev_field`, `x` clears
//! (`backlog.filter_form.clear`); an option row's value keys are the `[choice]` rows (MOD-115 M2
//! D7) and its `Enter`/`Esc` the canonical field events.

use crossterm::event::KeyEvent;
use htui_core::model::{ItemFilter, ProjectId, ProjectRef, Scope, Status, declared_tags_from_text};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::keys::translate::FieldEvent;
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, views};
use crate::store_worker::StoreRequest;
use crate::ui::cells::{self, cell_width};
use crate::ui::{FieldOutcome, TextField, Theme};

/// Rows the form takes at the bottom of the list pane: two borders and four rows (its keys are
/// the shell's hint row since MOD-67 M6 T2).
pub const FORM_HEIGHT: u16 = 6;

/// The hint under every row but Tags, through [`views::BACKLOG_FILTER`]: `choice.toggle` and
/// `field.submit` through the keys in force (MOD-115 M2 RV1 H2), `Esc` as written (MD7).
pub const HINT_CHOICES: HintSpec = &[
    Hint::One(Act::ChoiceToggle, "toggle"),
    Hint::One(Act::FilterClear, "clear"),
    Hint::One(Act::FieldSubmit, "apply"),
    Hint::Text("Esc cancel"),
];

/// The hint under the Tags row, where the letters are text (blueprint E7): the row moves with
/// `form.next_field` (`Tab row` under the defaults, MOD-67 M5).
pub const HINT_TAGS: HintSpec = &[
    Hint::Text("comma list"),
    Hint::One(Act::FormNextField, "row"),
    Hint::One(Act::FieldSubmit, "apply"),
    Hint::Text("Esc cancel"),
];

/// Width of a row's label column, after the two-cell focus marker.
const LABEL: usize = 8;

/// `declared_tags_from_text`'s refusal, worded for this form (MOD-13 review N2). The rule is the
/// declared-tag rule (D3), but these are the tags an item *requires*, so the sentence names a
/// `tag`, not a `declared tag`. The shared sentence stays Settings > Boxes'.
fn required_tag_refusal(sentence: String) -> String {
    match sentence.strip_prefix("declared tag ") {
        Some(rest) => format!("tag {rest}"),
        None => sentence,
    }
}

/// What narrows the Backlog list. `default()` is the whole scope, and shows nothing (D5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BacklogFilter {
    /// Statuses to keep; empty = any. Kept in [`Status::ALL`] order.
    pub statuses: Vec<Status>,
    /// Projects to keep; empty = every project of the scope. Kept in `ctx.projects` order.
    pub projects: Vec<ProjectId>,
    /// Required tags every kept item carries (D3), canonical through
    /// [`declared_tags_from_text`]; empty = no capability filter.
    pub tags: Vec<String>,
    /// ANA-9 §7.4 for this box: open, unblocked, and every required tag probed or declared here
    /// (D2). The worker composes it; the view only asks.
    pub ready_here: bool,
}

impl BacklogFilter {
    /// Whether no facet is set: the list reads and draws as it did before MOD-13 (D5).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The store-side filter. An empty facet is `None`, never `Some(vec![])`; `ready` is left to
    /// the worker (D2).
    #[must_use]
    pub fn item_filter(&self) -> ItemFilter {
        fn facet<T: Clone>(values: &[T]) -> Option<Vec<T>> {
            (!values.is_empty()).then(|| values.to_vec())
        }
        ItemFilter {
            statuses: facet(&self.statuses),
            project_ids: facet(&self.projects),
            tags: facet(&self.tags),
            ready: None,
            text: None,
        }
    }

    /// The one `Items` read this filter makes over `scope`.
    #[must_use]
    pub fn to_request(&self, scope: &Scope) -> StoreRequest {
        StoreRequest::Items {
            scope: scope.clone(),
            filter: self.item_filter(),
            ready_here: self.ready_here,
        }
    }

    /// What the list title appends while a filter is set (D5), e.g.
    /// `status:open,done · project:htui · tags:gpu,rust · ready here`; `None` without one.
    ///
    /// Project names come from `projects` (`ctx.projects`); an id it lacks is skipped, and so is
    /// the whole `project:` part when it names none (MOD-13 review N3). A filter with nothing to
    /// name is `None`, like no filter.
    #[must_use]
    pub fn summary(&self, projects: &[ProjectRef]) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::with_capacity(4);
        if !self.statuses.is_empty() {
            let names: Vec<&str> = self.statuses.iter().map(|status| status.as_str()).collect();
            parts.push(format!("status:{}", names.join(",")));
        }
        if !self.projects.is_empty() {
            let names: Vec<&str> = self
                .projects
                .iter()
                .filter_map(|id| projects.iter().find(|project| project.project_id == *id))
                .map(|project| project.name.as_str())
                .collect();
            if !names.is_empty() {
                parts.push(format!("project:{}", names.join(",")));
            }
        }
        if !self.tags.is_empty() {
            parts.push(format!("tags:{}", self.tags.join(",")));
        }
        if self.ready_here {
            parts.push("ready here".to_owned());
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Drops the projects `scope` lacks (D6), so a gone project cannot silently empty the list.
    /// Status, tags and readiness are scope-independent and stay.
    pub fn retain_projects(&mut self, scope: &Scope) {
        self.projects.retain(|project| scope.contains(*project));
    }
}

/// A row of the form, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterRow {
    /// The status options.
    Status,
    /// The scope's projects.
    Project,
    /// The required-tag text.
    Tags,
    /// "Only what this box can start".
    Ready,
}

/// The form's rows, in order.
const ROWS: [FilterRow; 4] = [
    FilterRow::Status,
    FilterRow::Project,
    FilterRow::Tags,
    FilterRow::Ready,
];

/// What one key did to the form, for the tab to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome {
    /// The form edited or moved; it stays open.
    Stay,
    /// `Esc`: close the form, keep the active filter.
    Cancel,
    /// `Enter` with a valid tag list: close the form and read with this filter.
    Apply(BacklogFilter),
    /// `Enter` with a refused tag list: `declared_tags_from_text`'s sentence, worded for a
    /// required tag, for the status line (D3, review N2). The form stays open with the text to
    /// fix.
    Refused(String),
    /// A key the form did not use (MOD-67 M5): the tab answers it with `modal_rest` over
    /// [`views::BACKLOG_FILTER`], so a chord or a function key reaches the shell.
    Pass,
}

/// The open filter form.
///
/// Holds a snapshot of `ctx.projects` from when it opened; the tab drops the form on a scope
/// change, so the snapshot never outlives its scope.
#[derive(Debug)]
pub struct FilterForm {
    /// Statuses, projects and readiness being edited; `draft.tags` is replaced from `tags` on
    /// apply.
    draft: BacklogFilter,
    /// The focused row; the form opens on Status.
    row: FilterRow,
    /// The status option under the cursor, an index into [`Status::ALL`].
    status_at: usize,
    /// The project option under the cursor, an index into `projects`.
    project_at: usize,
    /// `ctx.projects` when the form opened.
    projects: Vec<ProjectRef>,
    /// The required-tag list as typed.
    tags: TextField,
}

impl FilterForm {
    /// A form over the active `filter`, its option cursors on the first option.
    #[must_use]
    pub fn open(filter: &BacklogFilter, projects: &[ProjectRef]) -> Self {
        Self {
            draft: filter.clone(),
            row: FilterRow::Status,
            status_at: 0,
            project_at: 0,
            projects: projects.to_vec(),
            tags: TextField::with_text(&filter.tags.join(", ")),
        }
    }

    /// The focused row.
    #[must_use]
    pub const fn row(&self) -> FilterRow {
        self.row
    }

    /// Feeds one key, resolved through [`views::BACKLOG_FILTER`] (MOD-67 M5).
    ///
    /// On the Tags row every plain character is text (blueprint E7): the field takes the key
    /// first, and what it passes resolves as on any row, so the row moves on
    /// `form.next_field`/`prev_field` (`Tab`/`Down`, `BackTab`/`Up`). On the other rows the
    /// canonical `Enter` applies and `Esc` cancels (`field.submit`/`cancel` after the shell's
    /// translation, MOD-115 M2 D7), then the acts: `choice.prev`/`next` (`h`/`l`/arrows) move the
    /// option, `choice.toggle` (Space) ticks it, `backlog.filter_form.next_row`/`prev_row`
    /// (`j`/`k`) and the field moves move the row, and `backlog.filter_form.clear` (`x`) clears.
    /// Anything else is [`FormOutcome::Pass`]: a chord no row binds is never a form key (MOD-13
    /// review L4), so `Alt+x` clears nothing.
    ///
    /// MOD-115 M2 RV1 M: `key` is the event the widgets read (the shell's `[field]` translation
    /// made a rebound editing chord canonical) and `pressed` the chord the user pressed
    /// (`Ctx::pressed`): the acts resolve from `pressed`, so `field.left = ["ctrl-b"]` never moves
    /// an option.
    pub fn on_pressed(&mut self, key: KeyEvent, pressed: KeyChord, keys: &Keys) -> FormOutcome {
        if self.row == FilterRow::Tags {
            match self.tags.on_key(key) {
                FieldOutcome::Submit => return self.apply(),
                FieldOutcome::Cancel => return FormOutcome::Cancel,
                FieldOutcome::Consumed => return FormOutcome::Stay,
                FieldOutcome::Pass => {}
            }
        } else {
            match FieldEvent::of(&key) {
                Some(FieldEvent::Submit) => return self.apply(),
                Some(FieldEvent::Cancel) => return FormOutcome::Cancel,
                _ => {}
            }
        }
        let options = self.row != FilterRow::Tags;
        for act in keys.actions(views::BACKLOG_FILTER, pressed) {
            match act {
                Act::ChoicePrev if options => return self.move_option(false),
                Act::ChoiceNext if options => return self.move_option(true),
                Act::ChoiceToggle if options => return self.toggle(),
                Act::FormNextField => self.move_row(1),
                Act::FormPrevField => self.move_row(-1),
                // On the Tags row the field took every printable key above (`j`/`k`/`x` are
                // text there); a chord it passed acts on every row (M5 R1).
                Act::FilterNextRow => self.move_row(1),
                Act::FilterPrevRow => self.move_row(-1),
                Act::FilterClear => {
                    self.draft = BacklogFilter::default();
                    self.tags.clear();
                }
                _ => continue,
            }
            return FormOutcome::Stay;
        }
        FormOutcome::Pass
    }

    /// [`on_pressed`](Self::on_pressed) for a key the shell did not translate: the chord pressed
    /// is the key's own.
    #[cfg(test)]
    pub fn on_key(&mut self, key: KeyEvent, keys: &Keys) -> FormOutcome {
        self.on_pressed(key, KeyChord::from_event(key), keys)
    }

    /// A bracketed paste: the tag field's on the Tags row, dropped on every other row.
    pub fn on_paste(&mut self, text: &str) {
        if self.row == FilterRow::Tags {
            self.tags.on_paste(text);
        }
    }

    /// Moves the focus one row, clamped at both ends.
    fn move_row(&mut self, delta: isize) {
        let at = ROWS.iter().position(|row| *row == self.row).unwrap_or(0);
        let next = at.saturating_add_signed(delta).min(ROWS.len() - 1);
        self.row = ROWS[next];
    }

    /// Moves the option cursor of the Status or Project row, wrapping; nothing elsewhere.
    fn move_option(&mut self, forward: bool) -> FormOutcome {
        let step = |at: usize, len: usize| {
            if len == 0 {
                at
            } else if forward {
                (at + 1) % len
            } else {
                (at + len - 1) % len
            }
        };
        match self.row {
            FilterRow::Status => self.status_at = step(self.status_at, Status::ALL.len()),
            FilterRow::Project => self.project_at = step(self.project_at, self.projects.len()),
            FilterRow::Tags | FilterRow::Ready => {}
        }
        FormOutcome::Stay
    }

    /// Ticks or unticks the option under the cursor, keeping each list in its canonical order.
    fn toggle(&mut self) -> FormOutcome {
        match self.row {
            FilterRow::Status => {
                let status = Status::ALL[self.status_at];
                toggle(&mut self.draft.statuses, status);
                self.draft
                    .statuses
                    .sort_by_key(|status| Status::ALL.iter().position(|all| all == status));
            }
            FilterRow::Project => {
                let Some(project) = self.projects.get(self.project_at) else {
                    return FormOutcome::Stay;
                };
                toggle(&mut self.draft.projects, project.project_id);
                let order = &self.projects;
                self.draft
                    .projects
                    .sort_by_key(|id| order.iter().position(|project| project.project_id == *id));
            }
            FilterRow::Ready => self.draft.ready_here = !self.draft.ready_here,
            FilterRow::Tags => {}
        }
        FormOutcome::Stay
    }

    /// The draft with the typed tags, or the refusal (D3).
    fn apply(&self) -> FormOutcome {
        match declared_tags_from_text(self.tags.text().unwrap_or_default()) {
            Ok(tags) => FormOutcome::Apply(BacklogFilter {
                tags,
                ..self.draft.clone()
            }),
            Err(sentence) => FormOutcome::Refused(required_tag_refusal(sentence)),
        }
    }

    /// One row as a line of at most `width` cells: the focus marker, the label, the value.
    fn line(&self, row: FilterRow, width: usize, theme: &Theme) -> Line<'static> {
        let focused = row == self.row;
        let label = match row {
            FilterRow::Status => "status",
            FilterRow::Project => "project",
            FilterRow::Tags => "tags",
            FilterRow::Ready => "ready",
        };
        let head = format!("{} {label:<LABEL$}", if focused { '>' } else { ' ' });
        let head_style = if focused { theme.title } else { theme.dim };
        let room = width.saturating_sub(cell_width(&head));
        let value = match row {
            FilterRow::Tags => {
                let field = self
                    .tags
                    .line(u16::try_from(room).unwrap_or(u16::MAX), focused, theme);
                let mut spans = vec![Span::styled(cells::clip(&head, width), head_style)];
                spans.extend(field.spans);
                return Line::from(spans);
            }
            FilterRow::Status => {
                let option = Status::ALL[self.status_at];
                let chosen: Vec<&str> = self.draft.statuses.iter().map(|s| s.as_str()).collect();
                format!(
                    "‹{} {}›  {}",
                    check(self.draft.statuses.contains(&option)),
                    option.as_str(),
                    any(&chosen)
                )
            }
            FilterRow::Project => match self.projects.get(self.project_at) {
                None => "(none)".to_owned(),
                Some(option) => {
                    let chosen: Vec<&str> = self
                        .projects
                        .iter()
                        .filter(|project| self.draft.projects.contains(&project.project_id))
                        .map(|project| project.name.as_str())
                        .collect();
                    format!(
                        "‹{} {}›  {}",
                        check(self.draft.projects.contains(&option.project_id)),
                        option.name,
                        any(&chosen)
                    )
                }
            },
            FilterRow::Ready => format!(
                "{} only what this box can start",
                check(self.draft.ready_here)
            ),
        };
        Line::from(vec![
            Span::styled(cells::clip(&head, width), head_style),
            Span::styled(cells::clip(&value, room), theme.base),
        ])
    }
}

/// Adds `value` when absent, removes it when present.
fn toggle<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if let Some(at) = values.iter().position(|held| *held == value) {
        values.remove(at);
    } else {
        values.push(value);
    }
}

/// A tick box as characters, so a text snapshot shows the state.
const fn check(on: bool) -> &'static str {
    if on { "[x]" } else { "[ ]" }
}

/// The chosen options, or `any` when none is.
fn any(chosen: &[&str]) -> String {
    if chosen.is_empty() {
        "any".to_owned()
    } else {
        chosen.join(", ")
    }
}

/// Draws the form in `area`, the bottom [`FORM_HEIGHT`] rows of the list pane. Its keys are
/// [`FilterForm::key_hint`], the shell's hint row since MOD-67 M6 T2.
pub fn render(frame: &mut Frame<'_>, area: Rect, form: &FilterForm, theme: &Theme) {
    let block = Block::new().borders(Borders::ALL).title(" Filter ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let lines: Vec<Line<'static>> = ROWS
        .iter()
        .map(|row| form.line(*row, width, theme))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

impl FilterForm {
    /// The focused row's keys for the shell's hint row (MOD-67 M6 T2): [`HINT_TAGS`] on Tags,
    /// [`HINT_CHOICES`] elsewhere, through [`views::BACKLOG_FILTER`].
    #[must_use]
    pub fn key_hint(&self, keys: &Keys) -> String {
        let spec = if self.row == FilterRow::Tags {
            HINT_TAGS
        } else {
            HINT_CHOICES
        };
        keys.hint(views::BACKLOG_FILTER, spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::DEFAULT_SIZE;
    use crate::ui::layout::chrome;
    use crate::ui::tabs::backlog::panes;
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::WorkspaceId;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Block, Borders};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn chars(form: &mut FilterForm, text: &str) -> Vec<FormOutcome> {
        text.chars()
            .map(|c| form.on_key(key(KeyCode::Char(c)), Keys::compiled()))
            .collect()
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

    fn scope(projects: &[ProjectId]) -> Scope {
        Scope {
            workspace_id: WorkspaceId::new(),
            project_ids: projects.to_vec(),
        }
    }

    fn open() -> FilterForm {
        FilterForm::open(&BacklogFilter::default(), &projects())
    }

    /// MOD-67 M5 R1 NIT 4: a rebound row or clear chord the tag field passes works on the Tags row
    /// as on the others; a printable one is the field's text (and, onto an option row's key,
    /// refused by the validator).
    #[test]
    fn a_rebound_clear_the_tag_field_passes_clears_on_the_tags_row() {
        let keys = crate::keys::load_str("[backlog.filter_form]\nclear = \"f2\"\n")
            .expect("the keys load");
        let filter = BacklogFilter {
            statuses: vec![Status::Open],
            ..BacklogFilter::default()
        };
        let mut form = FilterForm::open(&filter, &projects());
        form.on_key(key(KeyCode::Tab), &keys);
        form.on_key(key(KeyCode::Tab), &keys);
        assert_eq!(form.row(), FilterRow::Tags);
        assert_eq!(form.on_key(key(KeyCode::F(2)), &keys), FormOutcome::Stay);
        assert_eq!(
            form.on_key(key(KeyCode::Enter), &keys),
            FormOutcome::Apply(BacklogFilter::default())
        );
    }

    /// MOD-115 M2 D7: an option row's value keys are the `[choice]` rows, so rebound ones move
    /// and tick the option, and the old letters no longer do.
    #[test]
    fn rebound_choice_keys_move_and_tick_the_option() {
        let keys = crate::keys::load_str(
            "version = 2\n[choice]\nnext = \"ctrl-n\"\ntoggle = \"ctrl-t\"\n",
        )
        .expect("the keys load");
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let mut form = open();
        assert_eq!(
            form.on_key(key(KeyCode::Char('l')), &keys),
            FormOutcome::Pass
        );
        assert_eq!(
            form.on_key(key(KeyCode::Char(' ')), &keys),
            FormOutcome::Pass
        );
        assert_eq!(form.on_key(ctrl('n'), &keys), FormOutcome::Stay);
        assert_eq!(form.on_key(ctrl('t'), &keys), FormOutcome::Stay);
        // RV1 H2: the hint names the toggle in force.
        assert_eq!(
            form.key_hint(&keys),
            "Ctrl+t toggle \u{b7} x clear \u{b7} Enter apply \u{b7} Esc cancel"
        );
        assert_eq!(
            form.on_key(key(KeyCode::Enter), &keys),
            FormOutcome::Apply(BacklogFilter {
                statuses: vec![Status::ALL[1]],
                ..BacklogFilter::default()
            })
        );
        // The compiled keys: `l` moves and Space ticks, as before M2.
        let mut form = open();
        chars(&mut form, "l ");
        assert_eq!(
            form.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Apply(BacklogFilter {
                statuses: vec![Status::ALL[1]],
                ..BacklogFilter::default()
            })
        );
        assert_eq!(
            form.on_key(key(KeyCode::Esc), Keys::compiled()),
            FormOutcome::Cancel
        );
    }

    /// MOD-115 M2 RV1 M: the form resolves its own acts from the chord pressed, not from the
    /// `[field]` translation's canonical event: with `field.left = ["ctrl-b"]`, `ctrl-b` arrives
    /// as `Left` and moves no option, while a pressed `Left` still does.
    #[test]
    fn a_translated_field_chord_moves_no_option() {
        let keys = crate::keys::load_str("version = 2\n[field]\nleft = [\"ctrl-b\"]\n")
            .expect("the keys load");
        let chord = |spec: &str| KeyChord::parse_strict(spec).expect("a valid spec");
        let mut form = open();
        assert_eq!(
            form.on_pressed(key(KeyCode::Left), chord("ctrl-b"), &keys),
            FormOutcome::Pass
        );
        assert_eq!(form.status_at, 0, "`ctrl-b` is the text field's");
        assert_eq!(
            form.on_pressed(key(KeyCode::Left), chord("left"), &keys),
            FormOutcome::Stay
        );
        assert_eq!(
            form.status_at,
            Status::ALL.len() - 1,
            "a pressed `Left` moves"
        );
    }

    /// MOD-115 M2 RV1 H2: both hints name the toggle and the submit key in force (`Space` and
    /// `Enter` under the compiled keys); `Esc` is written as is (MD7).
    #[test]
    fn the_hints_name_the_choice_and_field_keys_in_force() {
        let mut form = open();
        let sep = " \u{b7} ";
        assert_eq!(
            form.key_hint(Keys::compiled()),
            ["Space toggle", "x clear", "Enter apply", "Esc cancel"].join(sep)
        );
        let keys = crate::keys::load_str(
            "version = 2\n[choice]\ntoggle = \"ctrl-t\"\n[field]\nsubmit = \"ctrl-y\"\n",
        )
        .expect("the keys load");
        assert_eq!(
            form.key_hint(&keys),
            ["Ctrl+t toggle", "x clear", "Ctrl+y apply", "Esc cancel"].join(sep)
        );
        form.on_key(key(KeyCode::Tab), &keys);
        form.on_key(key(KeyCode::Tab), &keys);
        assert_eq!(form.row(), FilterRow::Tags);
        assert_eq!(
            form.key_hint(&keys),
            ["comma list", "Tab row", "Ctrl+y apply", "Esc cancel"].join(sep)
        );
        assert_eq!(
            form.key_hint(Keys::compiled()),
            ["comma list", "Tab row", "Enter apply", "Esc cancel"].join(sep)
        );
    }

    /// MOD-13 D5: no filter reads exactly what the tab read before MOD-13.
    #[test]
    fn an_empty_filter_is_the_default_read() {
        let filter = BacklogFilter::default();
        assert!(filter.is_empty());
        assert_eq!(filter.item_filter(), ItemFilter::default());
        let everything = scope(&[ids::PROJECT_HTUI]);
        assert!(matches!(
            filter.to_request(&everything),
            StoreRequest::Items { scope, filter, ready_here: false }
                if scope == everything && filter == ItemFilter::default()
        ));
    }

    /// MOD-13 D2, D3: every facet lands in its `ItemFilter` field; readiness is the worker's.
    #[test]
    fn to_request_maps_every_field() {
        let filter = BacklogFilter {
            statuses: vec![Status::Open, Status::Done],
            projects: vec![ids::PROJECT_AGY],
            tags: vec!["gpu".to_owned(), "rust".to_owned()],
            ready_here: true,
        };
        assert!(!filter.is_empty());
        let platform = scope(&[ids::PROJECT_HTUI, ids::PROJECT_AGY]);
        let StoreRequest::Items {
            scope: sent,
            filter: item_filter,
            ready_here,
        } = filter.to_request(&platform)
        else {
            panic!("the filter reads `Items`")
        };
        assert_eq!(sent, platform);
        assert!(ready_here);
        assert_eq!(
            item_filter,
            ItemFilter {
                statuses: Some(vec![Status::Open, Status::Done]),
                project_ids: Some(vec![ids::PROJECT_AGY]),
                tags: Some(vec!["gpu".to_owned(), "rust".to_owned()]),
                ready: None,
                text: None,
            },
            "the view never sets `ready`: the worker does (D2)"
        );
    }

    #[test]
    fn summary_is_none_without_a_filter() {
        assert_eq!(BacklogFilter::default().summary(&projects()), None);
    }

    /// MOD-13 D5: the title's summary, facet by facet, in a fixed order.
    #[test]
    fn summary_names_statuses_projects_tags_and_readiness() {
        let filter = BacklogFilter {
            statuses: vec![Status::Open, Status::Done],
            projects: vec![ids::PROJECT_HTUI],
            tags: vec!["gpu".to_owned(), "rust".to_owned()],
            ready_here: true,
        };
        assert_eq!(
            filter.summary(&projects()).as_deref(),
            Some("status:open,done · project:htui · tags:gpu,rust · ready here")
        );
        let ready = BacklogFilter {
            ready_here: true,
            ..BacklogFilter::default()
        };
        assert_eq!(ready.summary(&projects()).as_deref(), Some("ready here"));
    }

    /// MOD-13 D6: a project the new scope lacks leaves the filter; the rest stays.
    #[test]
    fn retain_projects_drops_what_the_scope_lacks() {
        let mut filter = BacklogFilter {
            statuses: vec![Status::Done],
            projects: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
            ..BacklogFilter::default()
        };
        filter.retain_projects(&scope(&[ids::PROJECT_HTUI]));
        assert_eq!(filter.projects, [ids::PROJECT_HTUI]);
        assert_eq!(filter.statuses, [Status::Done], "status is scope-free");
    }

    #[test]
    fn j_and_k_move_between_rows_and_stop_at_the_ends() {
        let mut form = open();
        assert_eq!(form.row, FilterRow::Status, "the form opens on Status");
        assert_eq!(
            form.on_key(key(KeyCode::Char('k')), Keys::compiled()),
            FormOutcome::Stay
        );
        assert_eq!(form.row, FilterRow::Status, "no wrap at the top");
        form.on_key(key(KeyCode::Char('j')), Keys::compiled());
        assert_eq!(form.row, FilterRow::Project);
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert_eq!(form.row, FilterRow::Tags);
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert_eq!(form.row, FilterRow::Ready);
        form.on_key(key(KeyCode::Char('j')), Keys::compiled());
        assert_eq!(form.row, FilterRow::Ready, "no wrap at the bottom");
        form.on_key(key(KeyCode::Char('k')), Keys::compiled());
        assert_eq!(form.row, FilterRow::Tags);
    }

    /// MOD-13 blueprint E7: on the Tags row the letters are text, so the arrows and `Tab` move.
    #[test]
    fn arrows_and_tab_leave_the_tags_row() {
        let mut form = open();
        form.on_key(key(KeyCode::Tab), Keys::compiled());
        form.on_key(key(KeyCode::Tab), Keys::compiled());
        assert_eq!(form.row, FilterRow::Tags);
        assert_eq!(
            form.on_key(key(KeyCode::Up), Keys::compiled()),
            FormOutcome::Stay
        );
        assert_eq!(form.row, FilterRow::Project);
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert_eq!(
            form.on_key(key(KeyCode::Down), Keys::compiled()),
            FormOutcome::Stay
        );
        assert_eq!(form.row, FilterRow::Ready);
        form.on_key(key(KeyCode::BackTab), Keys::compiled());
        assert_eq!(form.row, FilterRow::Tags);
        form.on_key(key(KeyCode::Tab), Keys::compiled());
        assert_eq!(form.row, FilterRow::Ready);
    }

    #[test]
    fn h_and_l_move_within_the_status_options_and_wrap() {
        let mut form = open();
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        assert_eq!(Status::ALL[form.status_at], Status::Queued);
        form.on_key(key(KeyCode::Left), Keys::compiled());
        form.on_key(key(KeyCode::Char('h')), Keys::compiled());
        assert_eq!(
            Status::ALL[form.status_at],
            Status::Closed,
            "`h` on the first wraps to the last"
        );
        form.on_key(key(KeyCode::Right), Keys::compiled());
        assert_eq!(Status::ALL[form.status_at], Status::Open, "and back");

        form.on_key(key(KeyCode::Char('j')), Keys::compiled());
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        assert_eq!(form.project_at, 0, "two projects: `l` twice wraps");
        // `Down`, not `j`: on the Tags row `j` is text (blueprint E7, review L3).
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert_eq!(form.row(), FilterRow::Ready);
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        assert_eq!(
            (form.status_at, form.project_at),
            (0, 0),
            "`l` on Ready moves nothing"
        );
    }

    #[test]
    fn space_toggles_a_status_in_all_order() {
        let mut form = open();
        for _ in 0..5 {
            form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        }
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(form.draft.statuses, [Status::Done]);
        for _ in 0..5 {
            form.on_key(key(KeyCode::Char('h')), Keys::compiled());
        }
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(
            form.draft.statuses,
            [Status::Open, Status::Done],
            "kept in `Status::ALL` order, not toggle order"
        );
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(
            form.draft.statuses,
            [Status::Done],
            "a second space unticks"
        );
    }

    #[test]
    fn space_toggles_a_project() {
        let mut form = open();
        form.on_key(key(KeyCode::Char('j')), Keys::compiled());
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        form.on_key(key(KeyCode::Char('l')), Keys::compiled());
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(
            form.draft.projects,
            [ids::PROJECT_HTUI, ids::PROJECT_AGY],
            "kept in `ctx.projects` order"
        );
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(form.draft.projects, [ids::PROJECT_AGY]);

        let mut none = FilterForm::open(&BacklogFilter::default(), &[]);
        none.on_key(key(KeyCode::Char('j')), Keys::compiled());
        none.on_key(key(KeyCode::Char('l')), Keys::compiled());
        assert_eq!(
            none.on_key(key(KeyCode::Char(' ')), Keys::compiled()),
            FormOutcome::Stay
        );
        assert!(
            none.draft.projects.is_empty(),
            "no project, nothing to tick"
        );
    }

    #[test]
    fn space_toggles_ready_here() {
        let mut form = open();
        for _ in 0..3 {
            form.on_key(key(KeyCode::Down), Keys::compiled());
        }
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert!(form.draft.ready_here);
        form.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert!(!form.draft.ready_here);
    }

    /// MOD-13 blueprint E7: `TextField` inserts every plain character.
    #[test]
    fn the_tags_row_types_every_letter() {
        let mut form = open();
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert!(
            chars(&mut form, "jk x hl")
                .iter()
                .all(|outcome| *outcome == FormOutcome::Stay)
        );
        assert_eq!(form.row, FilterRow::Tags, "no letter moved the row");
        assert_eq!(form.tags.text(), Some("jk x hl"));
        assert_eq!(
            form.draft,
            BacklogFilter::default(),
            "and `x` cleared nothing"
        );
    }

    /// MOD-13 D3: the tag text goes through `declared_tags_from_text`.
    #[test]
    fn enter_applies_canonical_tags() {
        let mut form = open();
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        chars(&mut form, " rust , gpu,rust");
        assert_eq!(
            form.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Apply(BacklogFilter {
                tags: vec!["gpu".to_owned(), "rust".to_owned()],
                ..BacklogFilter::default()
            })
        );

        let mut status = open();
        status.on_key(key(KeyCode::Char(' ')), Keys::compiled());
        assert_eq!(
            status.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Apply(BacklogFilter {
                statuses: vec![Status::Open],
                ..BacklogFilter::default()
            }),
            "`Enter` applies from any row"
        );
    }

    #[test]
    fn esc_cancels() {
        let mut form = open();
        assert_eq!(
            form.on_key(key(KeyCode::Esc), Keys::compiled()),
            FormOutcome::Cancel
        );
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        assert_eq!(
            form.on_key(key(KeyCode::Esc), Keys::compiled()),
            FormOutcome::Cancel,
            "from the tags row too"
        );
    }

    #[test]
    fn x_clears_the_draft_without_applying() {
        let filter = BacklogFilter {
            statuses: vec![Status::Done],
            projects: vec![ids::PROJECT_AGY],
            tags: vec!["rust".to_owned()],
            ready_here: true,
        };
        let mut form = FilterForm::open(&filter, &projects());
        assert_eq!(
            form.on_key(key(KeyCode::Char('x')), Keys::compiled()),
            FormOutcome::Stay
        );
        assert_eq!(form.draft, BacklogFilter::default());
        assert_eq!(form.tags.text(), Some(""));
        assert_eq!(
            form.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Apply(BacklogFilter::default())
        );
    }

    /// MOD-13 D3, review N2: a refusal is `declared_tags_from_text`'s rule, worded for a
    /// required tag, and the form stays open.
    #[test]
    fn a_refused_tag_list_stays_open() {
        let mut form = open();
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        chars(&mut form, "Rust");
        assert_eq!(
            form.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Refused(
                "tag `Rust` is not 1-64 characters of a-z, 0-9, `_` and `-` starting with a \
                 letter or a digit"
                    .to_owned()
            )
        );
        assert_eq!(form.tags.text(), Some("Rust"), "the text is kept to fix");
        assert_eq!(
            form.on_key(key(KeyCode::Esc), Keys::compiled()),
            FormOutcome::Cancel
        );
    }

    #[test]
    fn the_form_opens_on_the_active_filter() {
        let filter = BacklogFilter {
            statuses: vec![Status::Done],
            projects: vec![ids::PROJECT_AGY],
            tags: vec!["gpu".to_owned(), "rust".to_owned()],
            ready_here: true,
        };
        let mut form = FilterForm::open(&filter, &projects());
        assert_eq!(form.row, FilterRow::Status);
        assert_eq!((form.status_at, form.project_at), (0, 0));
        assert_eq!(form.tags.text(), Some("gpu, rust"));
        assert_eq!(
            form.on_key(key(KeyCode::Enter), Keys::compiled()),
            FormOutcome::Apply(filter),
            "applied untouched, it is the same filter"
        );
    }

    /// MOD-13 review L4, MOD-67 M5 D7: a chord is not a form key. `Alt+x` does not clear,
    /// `Alt+Enter` does not apply, `Alt+space` does not tick: the form answers `Pass`, and the tab
    /// leaves them to `BACKLOG_FILTER`'s modal layer.
    #[test]
    fn a_chord_on_a_choice_row_changes_nothing() {
        let filter = BacklogFilter {
            statuses: vec![Status::Done],
            ..BacklogFilter::default()
        };
        let mut form = FilterForm::open(&filter, &projects());
        for code in [
            KeyCode::Char('x'),
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Esc,
        ] {
            assert_eq!(
                form.on_key(KeyEvent::new(code, KeyModifiers::ALT), Keys::compiled()),
                FormOutcome::Pass,
                "Alt+{code:?}"
            );
        }
        assert_eq!(form.draft, filter, "the draft is untouched");
        assert_eq!(
            form.on_key(
                KeyEvent::new(KeyCode::Char('j'), KeyModifiers::SUPER),
                Keys::compiled()
            ),
            FormOutcome::Pass
        );
        assert_eq!(form.row, FilterRow::Status, "and no chord moved the row");
    }

    /// MOD-67 M5 D5: a function key is no form key on any row, Tags included: the form answers
    /// `Pass` (the tab's `modal_rest` lets `F1` open help) and the draft is untouched.
    #[test]
    fn f1_passes_from_every_row() {
        let mut form = open();
        for row in ROWS {
            assert_eq!(form.row, row);
            assert_eq!(
                form.on_key(key(KeyCode::F(1)), Keys::compiled()),
                FormOutcome::Pass,
                "{row:?}"
            );
            form.on_key(key(KeyCode::Down), Keys::compiled());
        }
        assert_eq!(form.draft, BacklogFilter::default());
    }

    /// MOD-13 review N3: a project the list cannot name is left out of the summary rather than
    /// printed as a bare `project:`.
    #[test]
    fn summary_skips_a_project_it_cannot_name() {
        let unknown = BacklogFilter {
            statuses: vec![Status::Done],
            projects: vec![ProjectId::new()],
            ..BacklogFilter::default()
        };
        assert_eq!(unknown.summary(&projects()).as_deref(), Some("status:done"));
        let only = BacklogFilter {
            projects: vec![ProjectId::new()],
            ..BacklogFilter::default()
        };
        assert_eq!(only.summary(&projects()), None, "nothing to name");
    }

    /// A paste lands in the tag field on the Tags row and nowhere else.
    #[test]
    fn a_paste_reaches_only_the_tags_row() {
        let mut form = open();
        form.on_paste("rust");
        assert_eq!(form.tags.text(), Some(""), "dropped on the Status row");
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_key(key(KeyCode::Down), Keys::compiled());
        form.on_paste("rust,\ngpu");
        assert_eq!(form.tags.text(), Some("rust,gpu"));
    }

    /// MOD-60 D1: the value is clipped in cells, so a CJK project name stays inside the row.
    #[test]
    fn a_wide_project_name_is_clipped_to_the_row() {
        let projects = vec![ProjectRef {
            project_id: ids::PROJECT_HTUI,
            slug: "wide".to_owned(),
            name: "\u{6f22}".repeat(30),
            position: 0,
        }];
        let form = FilterForm::open(&BacklogFilter::default(), &projects);
        let line = form.line(FilterRow::Project, 30, &Theme::default());
        let used: usize = line
            .spans
            .iter()
            .map(|span| cell_width(&span.content))
            .sum();
        assert!(used <= 30, "{line:?} against 30");
    }

    /// Both hints fit the form's inner width inside the list pane at the harness's pinned size, so
    /// a longer hint fails here rather than as a clipped snapshot (MOD-30).
    #[test]
    fn the_hints_fit_the_list_pane() {
        let (width, height) = DEFAULT_SIZE;
        let body = chrome(Rect::new(0, 0, width, height)).body;
        let [left, _] = panes(body);
        let list = Block::new().borders(Borders::ALL).inner(left);
        let room = usize::from(list.width) - 2;
        for spec in [HINT_CHOICES, HINT_TAGS] {
            let hint = Keys::compiled().hint(views::BACKLOG_FILTER, spec);
            let hint = hint.as_str();
            assert!(
                cell_width(hint) <= room,
                "{hint:?} is {} columns against {room}",
                cell_width(hint)
            );
        }
    }
}
