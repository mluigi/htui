//! The Requirements tab's forms (MOD-39 plan P11): a new area, a new or amended requirement, and a
//! withdraw that ends with the requirement's key typed back, the close-out pattern.
//!
//! `TextField` carries the one-line fields (area code and title, the deciding item's key) and
//! `TextArea`, reused as it is, the body and the rationale. `TextArea` breaks the line on `Enter`,
//! so a form saves on `Ctrl+S` from any field, and on `Enter` only from a one-line last field
//! (MOD-39 blueprint F-1). A form answers each key with a [`FormOutcome`]; the tab validates and
//! sends, so what a form holds never reaches the store but through `StoreRequest`.
//!
//! MOD-67 M4: a form's text fields see only plain keys, and only the keys its focused widget uses
//! are the form's: the field moves and the save are acts the tab resolves on a
//! [`FormOutcome::Pass`]. MOD-115 M2 D7: the priority resolves its keys through the keymap.
//!
//! MOD-115 M3 D6: the forms draw through `kit::form`, one label column for every field, the
//! focused one marked by the kit's gutter in the tab's accent (MOD-85: no label in
//! `theme.accent`), the priority a choice (`‹ must ›`), and each form's rule the help under the
//! field it is about.

use htui_core::model::{Priority, ProjectId, RequirementAreaId, RequirementId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::keys::translate::FieldEvent;
use crate::keys::{Act, KeyChord, Keys, is_plain, views};
use crate::ui::kit::form::{Field, Form, Value};
use crate::ui::kit::label;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use crossterm::event::KeyEvent;

/// How many rows `PageUp`/`PageDown` move the body or the rationale.
const PAGE: u16 = 5;

/// The label column every form shares (the kit caps it at a third of the pane).
const LABEL: u16 = 15;

/// The area code's rule, the help under the code field.
const CODE_RULE: &str = "A code is a capital letter, then 1 to 15 capitals or digits.";

/// What a withdraw does, the help under the withdraw's field.
const WITHDRAWN_STAYS: &str =
    "A withdrawn requirement stays listed, dimmed, and takes no new citation.";

/// A form with the shared label column.
fn form<'a>() -> Form<'a> {
    Form::new().label_width(LABEL)
}

/// What one key did to a form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormOutcome {
    /// The form took it; nothing for the tab to do.
    Stay,
    /// `Esc`: close the form, send nothing.
    Cancel,
    /// Validate and send (or, for a withdraw's first stage, go on).
    Submit,
    /// A key the focused widget did not use: the tab resolves it (MOD-67 M4).
    Pass,
}

/// `a`: a new area in a project.
#[derive(Debug)]
pub(super) struct AreaForm {
    /// The project the area goes in.
    pub(super) project: ProjectId,
    /// The code (`^[A-Z][A-Z0-9]{1,15}$`).
    pub(super) code: TextField,
    /// The title.
    pub(super) title: TextField,
    /// Which field has the keys.
    pub(super) focus: AreaFocus,
}

/// The area form's two fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AreaFocus {
    /// The code.
    Code,
    /// The title.
    Title,
}

impl AreaForm {
    /// An empty form for `project`, on the code.
    pub(super) fn new(project: ProjectId) -> Self {
        Self {
            project,
            code: TextField::new(),
            title: TextField::new(),
            focus: AreaFocus::Code,
        }
    }

    /// A bracketed paste into the focused field (MOD-22 review M-1).
    pub(super) fn on_paste(&mut self, text: &str) {
        match self.focus {
            AreaFocus::Code => self.code.on_paste(text),
            AreaFocus::Title => self.title.on_paste(text),
        };
    }

    /// The other field (`form.next_field` and `form.prev_field`: two fields, one toggle).
    pub(super) fn toggle(&mut self) {
        self.focus = match self.focus {
            AreaFocus::Code => AreaFocus::Title,
            AreaFocus::Title => AreaFocus::Code,
        };
    }

    /// A plain key: `Enter` on the code moves to the title and on the title submits, `Esc`
    /// cancels; a key the field does not use passes (the tab moves the focus on it, MOD-67 M4).
    pub(super) fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        let field = match self.focus {
            AreaFocus::Code => &mut self.code,
            AreaFocus::Title => &mut self.title,
        };
        match field.on_key(key) {
            FieldOutcome::Submit if self.focus == AreaFocus::Code => {
                self.focus = AreaFocus::Title;
                FormOutcome::Stay
            }
            FieldOutcome::Submit => FormOutcome::Submit,
            FieldOutcome::Cancel => FormOutcome::Cancel,
            FieldOutcome::Consumed => FormOutcome::Stay,
            FieldOutcome::Pass => FormOutcome::Pass,
        }
    }

    /// The code as typed, trimmed.
    pub(super) fn code(&self) -> String {
        self.code.text().unwrap_or_default().trim().to_owned()
    }

    /// The title as typed, trimmed.
    pub(super) fn title(&self) -> String {
        self.title.text().unwrap_or_default().trim().to_owned()
    }

    /// Draws the form into the pane's inner `area`: the code with its rule as help, and the
    /// title; `accent` marks the focused field (`ctx.accent()`).
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, accent: Style, theme: &Theme) {
        let focus = match self.focus {
            AreaFocus::Code => 0,
            AreaFocus::Title => 1,
        };
        form()
            .field(Field::new("code", Value::Text(&self.code)).help(CODE_RULE))
            .field(Field::new("title", Value::Text(&self.title)))
            .focus(focus)
            .editing(true)
            .render(frame, area, 0, accent, theme);
    }
}

/// What a requirement form saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FormTarget {
    /// `n`: a new requirement in an area (`MintRequirement`).
    Mint {
        /// The area's project: the one the gate checks.
        project: ProjectId,
        /// The area.
        area: RequirementAreaId,
        /// The area's code, for the pane title.
        code: String,
    },
    /// `e`: an amend (`AmendRequirement`).
    Amend {
        /// The requirement.
        id: RequirementId,
        /// Its key, for the pane title.
        key: String,
        /// The version the form opened on, or the head a stale answer showed: the compare-and-set
        /// token.
        expected_version: i32,
    },
}

/// `n` and `e`: body, rationale, priority and, for an amend, the deciding item's key.
pub(super) struct RequirementForm {
    /// What `Ctrl+S` writes.
    pub(super) target: FormTarget,
    /// The body.
    pub(super) body: TextArea,
    /// The rationale; may stay empty.
    pub(super) rationale: TextArea,
    /// `m` must, `l` later.
    pub(super) priority: Priority,
    /// The deciding item's key (amend only).
    pub(super) deciding: TextField,
    /// Which field has the keys.
    pub(super) focus: FormFocus,
}

/// Lengths, never the text: the body and the rationale are user prose.
impl core::fmt::Debug for RequirementForm {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RequirementForm")
            .field("target", &self.target)
            .field("body_len", &self.body.len())
            .field("rationale_len", &self.rationale.len())
            .field("priority", &self.priority)
            .field("deciding_len", &self.deciding.len())
            .field("focus", &self.focus)
            .finish()
    }
}

/// The requirement form's fields, in `Tab` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormFocus {
    /// The body.
    Body,
    /// The rationale.
    Rationale,
    /// The priority toggle.
    Priority,
    /// The deciding item's key; an amend only.
    Deciding,
}

impl RequirementForm {
    /// A blank mint form, on the body; `must` until toggled.
    pub(super) fn mint(project: ProjectId, area: RequirementAreaId, code: String) -> Self {
        Self {
            target: FormTarget::Mint {
                project,
                area,
                code,
            },
            body: TextArea::new(),
            rationale: TextArea::new(),
            priority: Priority::Must,
            deciding: TextField::new(),
            focus: FormFocus::Body,
        }
    }

    /// An amend form prefilled from the requirement as the snapshot shows it, on the body.
    pub(super) fn amend(
        id: RequirementId,
        key: String,
        expected_version: i32,
        body: &str,
        rationale: &str,
        priority: Priority,
    ) -> Self {
        Self {
            target: FormTarget::Amend {
                id,
                key,
                expected_version,
            },
            body: TextArea::with_text(body),
            rationale: TextArea::with_text(rationale),
            priority,
            deciding: TextField::new(),
            focus: FormFocus::Body,
        }
    }

    /// Whether this form amends (and so has a deciding field).
    pub(super) const fn amends(&self) -> bool {
        matches!(self.target, FormTarget::Amend { .. })
    }

    /// The fields in `Tab` order.
    fn order(&self) -> &'static [FormFocus] {
        if self.amends() {
            &[
                FormFocus::Body,
                FormFocus::Rationale,
                FormFocus::Priority,
                FormFocus::Deciding,
            ]
        } else {
            &[FormFocus::Body, FormFocus::Rationale, FormFocus::Priority]
        }
    }

    /// The next (`forward`) or previous field, wrapping (`form.next_field`, `form.prev_field`).
    pub(super) fn cycle(&mut self, forward: bool) {
        let order = self.order();
        let at = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % order.len()
        } else {
            (at + order.len() - 1) % order.len()
        };
        self.focus = order[next];
    }

    /// A bracketed paste into the focused field (MOD-22 review M-1). The priority toggle is not a
    /// field: a paste there is dropped, so its `m` and `l` toggle nothing.
    pub(super) fn on_paste(&mut self, text: &str) {
        match self.focus {
            FormFocus::Body => self.body.on_paste(text),
            FormFocus::Rationale => self.rationale.on_paste(text),
            FormFocus::Priority => {}
            FormFocus::Deciding => {
                self.deciding.on_paste(text);
            }
        }
    }

    /// A key to the focused field (MOD-115 M2 D7). On the priority the key resolves through
    /// `views::REQUIREMENTS_FORM`, first matching candidate wins: `requirements.priority_must`
    /// (`m`) is must, `priority_later` (`l`) later, and `choice.prev`/`next`/`toggle` (`h`/`←`,
    /// `→`, `Space`) toggle; the canonical `Esc` (`field.cancel` after the shell's translation)
    /// cancels. The text fields take plain keys only (a chord never reaches them, so the
    /// `TextArea` never sees `ctrl-s`); `Enter` on the deciding key submits and `Esc` anywhere
    /// cancels. A key the field does not use passes: the tab moves the focus `Body → Rationale →
    /// Priority (→ Deciding)` or saves on it.
    ///
    /// MOD-115 M2 RV1 M: `key` is the event the widgets read (the shell's `[field]` translation
    /// made a rebound editing chord canonical) and `pressed` the chord the user pressed
    /// (`Ctx::pressed`): the priority resolves its keys from `pressed`, so `field.left =
    /// ["ctrl-b"]` never toggles it.
    pub(super) fn on_pressed(
        &mut self,
        key: KeyEvent,
        pressed: KeyChord,
        keys: &Keys,
    ) -> FormOutcome {
        if self.focus != FormFocus::Priority && !is_plain(&key) {
            return FormOutcome::Pass;
        }
        match self.focus {
            FormFocus::Body | FormFocus::Rationale => {
                let area = if self.focus == FormFocus::Body {
                    &mut self.body
                } else {
                    &mut self.rationale
                };
                match area.on_key(key, PAGE) {
                    FieldOutcome::Cancel => FormOutcome::Cancel,
                    FieldOutcome::Consumed => FormOutcome::Stay,
                    // A `TextArea` never submits since MOD-67 M4 D6: `ctrl-s` passes, and only
                    // `form.save` saves.
                    FieldOutcome::Submit | FieldOutcome::Pass => FormOutcome::Pass,
                }
            }
            FormFocus::Priority => {
                let chosen = keys
                    .actions(views::REQUIREMENTS_FORM, pressed)
                    .into_iter()
                    .find_map(|act| match act {
                        Act::RequirementsPriorityMust => Some(Some(Priority::Must)),
                        Act::RequirementsPriorityLater => Some(Some(Priority::Later)),
                        Act::ChoicePrev | Act::ChoiceNext | Act::ChoiceToggle => Some(None),
                        _ => None,
                    });
                match chosen {
                    Some(Some(priority)) => self.priority = priority,
                    Some(None) => {
                        self.priority = match self.priority {
                            Priority::Must => Priority::Later,
                            Priority::Later => Priority::Must,
                        };
                    }
                    None if FieldEvent::of(&key) == Some(FieldEvent::Cancel) => {
                        return FormOutcome::Cancel;
                    }
                    None => return FormOutcome::Pass,
                }
                FormOutcome::Stay
            }
            FormFocus::Deciding => match self.deciding.on_key(key) {
                FieldOutcome::Submit => FormOutcome::Submit,
                FieldOutcome::Cancel => FormOutcome::Cancel,
                FieldOutcome::Consumed => FormOutcome::Stay,
                FieldOutcome::Pass => FormOutcome::Pass,
            },
        }
    }

    /// [`on_pressed`](Self::on_pressed) for a key the shell did not translate: the chord pressed
    /// is the key's own.
    #[cfg(test)]
    pub(super) fn on_key(&mut self, key: KeyEvent, keys: &Keys) -> FormOutcome {
        self.on_pressed(key, KeyChord::from_event(key), keys)
    }

    /// The deciding key as typed, trimmed.
    pub(super) fn deciding(&self) -> String {
        self.deciding.text().unwrap_or_default().trim().to_owned()
    }

    /// Draws the form into the pane's inner `area`: the body and the rationale (`TextArea`s, two
    /// thirds and a third of the rows the one-line fields leave), the priority as a choice and,
    /// for an amend, the deciding item; `accent` marks the focused field (`ctx.accent()`). Only
    /// the priority is not typed into, so it alone is banded whole (the kit's browsing marker).
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, accent: Style, theme: &Theme) {
        let deciding = u16::from(self.amends());
        let [body, rationale, _, _] = Layout::vertical([
            Constraint::Fill(2),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(deciding),
        ])
        .areas(area);
        let shared = form();
        let value = area
            .width
            .saturating_sub(1 + shared.label_column(area.width) + 2);
        let text = |field: &TextArea, rows: u16, focus: FormFocus| {
            let mut lines = field.lines(value, rows, self.focus == focus, theme);
            lines.resize(usize::from(rows).max(1), Line::default());
            Value::Lines(lines)
        };
        let mut form = shared
            .field(Field::new(
                "body",
                text(&self.body, body.height, FormFocus::Body),
            ))
            .field(Field::new(
                "rationale",
                text(&self.rationale, rationale.height, FormFocus::Rationale),
            ))
            .field(Field::new(
                "priority",
                Value::Choice(label::priority(self.priority)),
            ));
        if self.amends() {
            form = form.field(Field::new("deciding item", Value::Text(&self.deciding)));
        }
        let focus = self
            .order()
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        form.focus(focus)
            .editing(self.focus != FormFocus::Priority)
            .render(frame, area, 0, accent, theme);
    }
}

/// `W`: the deciding item's key, then the requirement's key typed back.
#[derive(Debug)]
pub(super) enum WithdrawForm {
    /// Stage 1: which item decides the withdraw.
    Deciding {
        /// The requirement.
        id: RequirementId,
        /// Its key: what stage 2 wants typed back.
        key: String,
        /// The compare-and-set token.
        expected_version: i32,
        /// The deciding key.
        field: TextField,
    },
    /// Stage 2: the requirement's key, typed back.
    Typed {
        /// The requirement.
        id: RequirementId,
        /// Its key.
        key: String,
        /// The compare-and-set token.
        expected_version: i32,
        /// The key stage 1 accepted, trimmed.
        deciding: String,
        /// What is typed.
        field: TextField,
    },
}

impl WithdrawForm {
    /// Stage 1 over `id`.
    pub(super) fn new(id: RequirementId, key: String, expected_version: i32) -> Self {
        Self::Deciding {
            id,
            key,
            expected_version,
            field: TextField::new(),
        }
    }

    /// The requirement's key.
    pub(super) fn key(&self) -> &str {
        match self {
            Self::Deciding { key, .. } | Self::Typed { key, .. } => key,
        }
    }

    /// The requirement.
    pub(super) const fn id(&self) -> RequirementId {
        match self {
            Self::Deciding { id, .. } | Self::Typed { id, .. } => *id,
        }
    }

    /// Moves the compare-and-set token to `head` (a stale answer).
    pub(super) const fn set_expected_version(&mut self, head: i32) {
        match self {
            Self::Deciding {
                expected_version, ..
            }
            | Self::Typed {
                expected_version, ..
            } => *expected_version = head,
        }
    }

    /// A bracketed paste into the stage's field (MOD-22 review M-1).
    pub(super) fn on_paste(&mut self, text: &str) {
        match self {
            Self::Deciding { field, .. } | Self::Typed { field, .. } => field.on_paste(text),
        };
    }

    /// A plain key: the stage's field takes it; `Enter` submits the stage, `Esc` cancels, a key
    /// the field does not use passes (MOD-67 M4).
    pub(super) fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        let field = match self {
            Self::Deciding { field, .. } | Self::Typed { field, .. } => field,
        };
        match field.on_key(key) {
            FieldOutcome::Submit => FormOutcome::Submit,
            FieldOutcome::Cancel => FormOutcome::Cancel,
            FieldOutcome::Consumed => FormOutcome::Stay,
            FieldOutcome::Pass => FormOutcome::Pass,
        }
    }

    /// Draws the form into the pane's inner `area`; `body` is the requirement's, as the snapshot
    /// shows it, one hard line at a time (a `Line` drops the `\n` of a `TextArea` body). Under it
    /// the stage's form: the deciding item with what a withdraw does as its help, then the key
    /// typed back with what to type; `accent` marks the focused field (`ctx.accent()`).
    pub(super) fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        body: &str,
        accent: Style,
        theme: &Theme,
    ) {
        let mut lines = body_lines(body, theme);
        lines.push(Line::default());
        let stage = match self {
            Self::Deciding { field, .. } => form()
                .field(Field::new("deciding item", Value::Text(field)).help(WITHDRAWN_STAYS))
                .focus(0),
            Self::Typed {
                key,
                deciding,
                field,
                ..
            } => form()
                .field(Field::new(
                    "deciding item",
                    Value::ReadOnly(deciding.as_str().into()),
                ))
                .field(
                    Field::new("key", Value::Text(field))
                        .help(format!("type {key} to withdraw it. {WITHDRAWN_STAYS}")),
                )
                .focus(1),
        };
        lines.extend(stage.editing(true).lines(area.width, accent, theme));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
}

/// `body` as lines, one per hard line; the paragraph around them wraps each.
fn body_lines(body: &str, theme: &Theme) -> Vec<Line<'static>> {
    body.lines()
        .map(|line| Line::styled(line.to_owned(), theme.base))
        .collect()
}

#[cfg(test)]
mod tests {
    use htui_core::model::{Priority, ProjectId, RequirementAreaId};

    use super::{FormFocus, FormOutcome, RequirementForm, body_lines};
    use crate::keys::{Act, KeyChord, Keys, load_str, views};
    use crate::ui::Theme;

    fn on_priority(priority: Priority) -> RequirementForm {
        let mut form = RequirementForm::mint(
            ProjectId::new(),
            RequirementAreaId::new(),
            "AREA".to_owned(),
        );
        form.priority = priority;
        form.focus = FormFocus::Priority;
        form
    }

    fn press(form: &mut RequirementForm, spec: &str, keys: &Keys) -> FormOutcome {
        let key = KeyChord::parse_strict(spec)
            .expect("a valid spec")
            .to_event();
        form.on_key(key, keys)
    }

    /// MOD-115 M2 D7 (T0 verify, T5R): every chord the `?` box lists over the form for the
    /// priority's rows acts on the priority as its entry says, so the help line and the
    /// behaviour agree: `l` is listed as later only, since on the priority it always sets later.
    #[test]
    fn every_listed_priority_chord_acts() {
        let keys = Keys::compiled();
        let lines = keys.help_lines(views::REQUIREMENTS_FORM, |_| true);
        // The chords the box lists for `act`: its entry's labels, read back as the row's chords.
        let listed = |act: Act| -> Vec<String> {
            let spec = act.spec().expect("a row");
            let suffix = format!(" {}", spec.help);
            let entry = lines
                .iter()
                .flat_map(|line| &line.entries)
                .find(|entry| entry.ends_with(&suffix))
                .unwrap_or_else(|| panic!("{act:?} is listed: {lines:?}"));
            let labels = entry.strip_suffix(&suffix).expect("the suffix");
            labels
                .split('/')
                .map(|label| {
                    keys.chords(spec.context, act)
                        .iter()
                        .find(|chord| chord.label() == label)
                        .unwrap_or_else(|| panic!("{label} is a chord of {act:?}"))
                        .spec()
                })
                .collect()
        };
        assert_eq!(listed(Act::RequirementsPriorityMust), ["m"]);
        assert_eq!(listed(Act::RequirementsPriorityLater), ["l"]);
        assert!(
            !listed(Act::ChoiceNext).contains(&"l".to_owned()),
            "`l` sets later on the priority, so the box does not call it the next option"
        );
        for spec in listed(Act::RequirementsPriorityMust) {
            let mut form = on_priority(Priority::Later);
            assert_eq!(press(&mut form, &spec, keys), FormOutcome::Stay, "{spec}");
            assert_eq!(form.priority, Priority::Must, "{spec}");
        }
        for spec in listed(Act::RequirementsPriorityLater) {
            for from in [Priority::Must, Priority::Later] {
                let mut form = on_priority(from);
                assert_eq!(press(&mut form, &spec, keys), FormOutcome::Stay, "{spec}");
                assert_eq!(form.priority, Priority::Later, "{spec}");
            }
        }
        for act in [Act::ChoicePrev, Act::ChoiceNext, Act::ChoiceToggle] {
            for spec in listed(act) {
                for (from, to) in [
                    (Priority::Must, Priority::Later),
                    (Priority::Later, Priority::Must),
                ] {
                    let mut form = on_priority(from);
                    assert_eq!(press(&mut form, &spec, keys), FormOutcome::Stay, "{spec}");
                    assert_eq!(form.priority, to, "{act:?} {spec} toggles");
                }
            }
        }
        let mut form = on_priority(Priority::Must);
        assert_eq!(press(&mut form, "esc", keys), FormOutcome::Cancel);
        assert_eq!(press(&mut form, "x", keys), FormOutcome::Pass);
        assert_eq!(press(&mut form, "ctrl-s", keys), FormOutcome::Pass);
    }

    /// MOD-115 M2 D7: a rebound priority key works, and the old letter no longer does.
    #[test]
    fn a_rebound_priority_key_works_and_the_old_one_does_not() {
        let keys = load_str(
            "version = 2\n[requirements]\npriority_must = \"M\"\npriority_later = \"alt-l\"\n\
             [choice]\ntoggle = \"t\"\n",
        )
        .expect("the file loads");
        // MOD-115 M2 RV4: the hint row and the `?` box name the rebound keys, not `m/l`/`Space`.
        assert_eq!(
            keys.hint(
                views::REQUIREMENTS_FORM,
                crate::ui::tabs::requirements::PRIORITY_HINT
            ),
            "M/Alt+l priority"
        );
        let entries: Vec<String> = keys
            .help_lines(views::REQUIREMENTS_FORM, |_| true)
            .into_iter()
            .flat_map(|line| line.entries)
            .collect();
        for listed in ["M priority must", "Alt+l priority later", "t toggle"] {
            assert!(entries.iter().any(|entry| entry == listed), "{entries:?}");
        }
        let mut form = on_priority(Priority::Later);
        assert_eq!(press(&mut form, "m", &keys), FormOutcome::Pass);
        assert_eq!(form.priority, Priority::Later);
        assert_eq!(press(&mut form, "M", &keys), FormOutcome::Stay);
        assert_eq!(form.priority, Priority::Must);
        assert_eq!(press(&mut form, "alt-l", &keys), FormOutcome::Stay);
        assert_eq!(form.priority, Priority::Later);
        assert_eq!(press(&mut form, "space", &keys), FormOutcome::Pass);
        assert_eq!(press(&mut form, "t", &keys), FormOutcome::Stay);
        assert_eq!(form.priority, Priority::Must);
        // `l` is still `choice.next`, which toggles.
        assert_eq!(press(&mut form, "l", &keys), FormOutcome::Stay);
        assert_eq!(form.priority, Priority::Later);
    }

    /// MOD-115 M2 RV1 M: the priority resolves its keys from the chord pressed, not from the
    /// `[field]` translation's canonical event: with `field.left = ["ctrl-b"]`, `ctrl-b` arrives
    /// as `Left` and toggles nothing, while a pressed `Left` still toggles.
    #[test]
    fn a_translated_field_chord_never_sets_the_priority() {
        let keys = load_str("version = 2\n[field]\nleft = [\"ctrl-b\"]\n").expect("loads");
        let chord = |spec: &str| KeyChord::parse_strict(spec).expect("a valid spec");
        let left = chord("left").to_event();
        let mut form = on_priority(Priority::Must);
        assert_eq!(
            form.on_pressed(left, chord("ctrl-b"), &keys),
            FormOutcome::Pass
        );
        assert_eq!(form.priority, Priority::Must);
        assert_eq!(
            form.on_pressed(left, chord("left"), &keys),
            FormOutcome::Stay
        );
        assert_eq!(form.priority, Priority::Later);
    }

    /// A chord never reaches a text field: the body passes `ctrl-s` to the tab.
    #[test]
    fn a_chord_on_a_text_field_passes() {
        let mut form = on_priority(Priority::Must);
        form.focus = FormFocus::Body;
        assert_eq!(
            press(&mut form, "alt-m", Keys::compiled()),
            FormOutcome::Pass
        );
        assert_eq!(press(&mut form, "m", Keys::compiled()), FormOutcome::Stay);
        assert_eq!(form.body.text(), "m");
    }

    /// A `TextArea` body keeps its line breaks on the withdraw form: `Line` would drop the `\n`
    /// and run the lines together.
    #[test]
    fn a_multi_line_body_stays_on_its_lines() {
        let lines = body_lines("First line.\nSecond line.", &Theme::default());
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        assert_eq!(text, vec!["First line.", "Second line."]);
    }
}
