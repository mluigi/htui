//! The Requirements tab's forms (MOD-39 plan P11): a new area, a new or amended requirement, and a
//! withdraw that ends with the requirement's key typed back, the close-out pattern.
//!
//! `TextField` carries the one-line fields (area code and title, the deciding item's key) and
//! `TextArea`, reused as it is, the body and the rationale. `TextArea` breaks the line on `Enter`,
//! so a form saves on `Ctrl+S` from any field, and on `Enter` only from a one-line last field
//! (MOD-39 blueprint F-1). A form answers each key with a [`FormOutcome`]; the tab validates and
//! sends, so what a form holds never reaches the store but through `StoreRequest`.

use htui_core::model::{Priority, ProjectId, RequirementAreaId, RequirementId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent};

/// How many rows `PageUp`/`PageDown` move the body or the rationale.
const PAGE: u16 = 5;

/// The label column of the one-line fields.
const LABEL: usize = 15;

/// What one key did to a form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FormOutcome {
    /// The form took it; nothing for the tab to do.
    Stay,
    /// `Esc`: close the form, send nothing.
    Cancel,
    /// Validate and send (or, for a withdraw's first stage, go on).
    Submit,
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

    /// A key that is not a `CONTROL` chord: `Tab`/`Shift+Tab`/`Up`/`Down` switch field, `Enter`
    /// on the code moves to the title and on the title submits, `Esc` cancels.
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
            FieldOutcome::Pass
                if matches!(
                    key.code,
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down
                ) =>
            {
                self.focus = match self.focus {
                    AreaFocus::Code => AreaFocus::Title,
                    AreaFocus::Title => AreaFocus::Code,
                };
                FormOutcome::Stay
            }
            FieldOutcome::Consumed | FieldOutcome::Pass => FormOutcome::Stay,
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

    /// Draws the form into the pane's inner `area`.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let width = area.width;
        let lines = vec![
            field_line(
                "code",
                &self.code,
                width,
                self.focus == AreaFocus::Code,
                theme,
            ),
            field_line(
                "title",
                &self.title,
                width,
                self.focus == AreaFocus::Title,
                theme,
            ),
            Line::default(),
            Line::styled(
                "A code is a capital letter, then 1 to 15 capitals or digits.",
                theme.dim,
            ),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
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

    /// The next (`forward`) or previous field, wrapping.
    fn cycle(&mut self, forward: bool) {
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

    /// A key that is not a `CONTROL` chord (the tab took `Ctrl+S`): `Tab`/`Shift+Tab` cycle
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

    /// `Body → Rationale → Priority (→ Deciding)`; on the priority `m` is must, `l` later and
    /// `Space`/`←`/`→` toggle; `Enter` on the deciding key submits; `Esc` anywhere cancels.
    pub(super) fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        match key.code {
            KeyCode::Tab => {
                self.cycle(true);
                return FormOutcome::Stay;
            }
            KeyCode::BackTab => {
                self.cycle(false);
                return FormOutcome::Stay;
            }
            _ => {}
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
                    FieldOutcome::Submit => FormOutcome::Submit,
                    FieldOutcome::Consumed | FieldOutcome::Pass => FormOutcome::Stay,
                }
            }
            FormFocus::Priority => {
                match key.code {
                    KeyCode::Char('m') => self.priority = Priority::Must,
                    KeyCode::Char('l') => self.priority = Priority::Later,
                    KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                        self.priority = match self.priority {
                            Priority::Must => Priority::Later,
                            Priority::Later => Priority::Must,
                        };
                    }
                    KeyCode::Esc => return FormOutcome::Cancel,
                    _ => {}
                }
                FormOutcome::Stay
            }
            FormFocus::Deciding => match self.deciding.on_key(key) {
                FieldOutcome::Submit => FormOutcome::Submit,
                FieldOutcome::Cancel => FormOutcome::Cancel,
                FieldOutcome::Consumed | FieldOutcome::Pass => FormOutcome::Stay,
            },
        }
    }

    /// The deciding key as typed, trimmed.
    pub(super) fn deciding(&self) -> String {
        self.deciding.text().unwrap_or_default().trim().to_owned()
    }

    /// Draws the form into the pane's inner `area`.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let deciding = u16::from(self.amends());
        let [
            body_label,
            body,
            rationale_label,
            rationale,
            priority,
            deciding_row,
        ] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(2),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(deciding),
        ])
        .areas(area);
        let label = |text: &str, focus: FormFocus| {
            Paragraph::new(Line::styled(
                text.to_owned(),
                label_style(self.focus == focus, theme),
            ))
        };
        frame.render_widget(label("body", FormFocus::Body), body_label);
        frame.render_widget(
            Paragraph::new(self.body.lines(
                body.width,
                body.height,
                self.focus == FormFocus::Body,
                theme,
            )),
            body,
        );
        frame.render_widget(label("rationale", FormFocus::Rationale), rationale_label);
        frame.render_widget(
            Paragraph::new(self.rationale.lines(
                rationale.width,
                rationale.height,
                self.focus == FormFocus::Rationale,
                theme,
            )),
            rationale,
        );
        let choice = |value: Priority| {
            if self.priority == value {
                Span::styled(format!("[{value}]"), theme.accent)
            } else {
                Span::styled(format!(" {value} "), theme.dim)
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    padded("priority", LABEL),
                    label_style(self.focus == FormFocus::Priority, theme),
                ),
                choice(Priority::Must),
                Span::raw(" "),
                choice(Priority::Later),
            ])),
            priority,
        );
        if self.amends() {
            frame.render_widget(
                Paragraph::new(field_line(
                    "deciding item",
                    &self.deciding,
                    deciding_row.width,
                    self.focus == FormFocus::Deciding,
                    theme,
                )),
                deciding_row,
            );
        }
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

    /// A key that is not a `CONTROL` chord: the stage's field takes it; `Enter` submits the
    /// stage, `Esc` cancels.
    pub(super) fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        let field = match self {
            Self::Deciding { field, .. } | Self::Typed { field, .. } => field,
        };
        match field.on_key(key) {
            FieldOutcome::Submit => FormOutcome::Submit,
            FieldOutcome::Cancel => FormOutcome::Cancel,
            FieldOutcome::Consumed | FieldOutcome::Pass => FormOutcome::Stay,
        }
    }

    /// Draws the form into the pane's inner `area`; `body` is the requirement's, as the snapshot
    /// shows it, one hard line at a time (a `Line` drops the `\n` of a `TextArea` body).
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, body: &str, theme: &Theme) {
        let width = area.width;
        let mut lines = body_lines(body, theme);
        lines.extend([
            Line::default(),
            Line::styled(
                "A withdrawn requirement stays listed, dimmed, and takes no new citation.",
                theme.dim,
            ),
            Line::default(),
        ]);
        match self {
            Self::Deciding { field, .. } => {
                lines.push(field_line("deciding item", field, width, true, theme));
            }
            Self::Typed {
                key,
                deciding,
                field,
                ..
            } => {
                lines.push(Line::from(vec![
                    Span::styled(padded("deciding item", LABEL), theme.dim),
                    Span::styled(deciding.clone(), theme.base),
                ]));
                lines.push(Line::default());
                lines.push(Line::styled(
                    format!("type {key} to withdraw it"),
                    theme.base,
                ));
                lines.push(field_line("key", field, width, true, theme));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
}

/// `body` as lines, one per hard line; the paragraph around them wraps each.
fn body_lines(body: &str, theme: &Theme) -> Vec<Line<'static>> {
    body.lines()
        .map(|line| Line::styled(line.to_owned(), theme.base))
        .collect()
}

/// `theme.accent` on the field with the keys, `theme.dim` otherwise.
fn label_style(focused: bool, theme: &Theme) -> Style {
    if focused { theme.accent } else { theme.dim }
}

/// `label` in its column, then the field.
fn field_line(
    label: &str,
    field: &TextField,
    width: u16,
    focused: bool,
    theme: &Theme,
) -> Line<'static> {
    let room = width.saturating_sub(u16::try_from(LABEL).unwrap_or(u16::MAX));
    let mut spans = vec![Span::styled(
        padded(label, LABEL),
        label_style(focused, theme),
    )];
    spans.extend(field.line(room, focused, theme).spans);
    Line::from(spans)
}

/// `text` padded with spaces to `width` chars.
fn padded(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    out.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(text.chars().count()),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::body_lines;
    use crate::ui::Theme;

    /// A `TextArea` body keeps its line breaks on the withdraw form: `Line` would drop the `\n`
    /// and run the lines together.
    #[test]
    fn a_multi_line_body_stays_on_its_lines() {
        let lines = body_lines("First line.\nSecond line.", &Theme::default());
        let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
        assert_eq!(text, vec!["First line.", "Second line."]);
    }
}
