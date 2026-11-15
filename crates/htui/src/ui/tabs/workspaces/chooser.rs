//! A short list in a kit modal over the Workspaces tab (MOD-116 M3 D1): `N`'s "what to create"
//! and, in T4, the default-target picker. The path picker's shape: a `Modal` sized to its rows, a
//! `kit::list` inside it, the cursor's row banded, the keys on the border.
//!
//! The tab resolves a key through [`views::WORKSPACES_PICK`](crate::keys::views::WORKSPACES_PICK)
//! and hands the chooser the act: `list.down`/`list.up` ([`Act::ListDown`]/[`Act::ListUp`]) move
//! (skipping nothing: a disabled row is drawn dim and can be landed on, so it can say why),
//! `confirm.yes` ([`Act::ConfirmYes`]) chooses the cursor's row, and `confirm.no`
//! ([`Act::ConfirmNo`]) closes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::Ctx;
use crate::keys::Act;
use crate::ui::cells;
use crate::ui::kit::list::{self, Row, RowKind};
use crate::ui::kit::modal::{Modal, ModalSize};

/// The blank cells between a row's label and its note.
const NOTE_GAP: usize = 2;

/// The narrowest the box's content is drawn, so a short list's border still holds its hint.
const MIN_WIDTH: usize = 36;

/// One row of a [`Chooser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChoiceRow<T> {
    /// What the row says.
    pub label: String,
    /// A dim note after the label (`· current`, `planned · MOD-n`).
    pub note: Option<String>,
    /// Whether choosing it does anything; a disabled row is drawn dim.
    pub enabled: bool,
    /// What choosing it answers.
    pub value: T,
}

/// What one act did to a [`Chooser`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChooserOutcome<T> {
    /// It moved, or the act was not its own: it stays up.
    Stay,
    /// The cursor's row was chosen.
    Chosen(T),
    /// The cursor's row is disabled: the caller says why; the chooser stays up.
    Disabled(usize),
    /// Closed without a choice.
    Closed,
}

/// A titled list in a modal with one row under the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Chooser<T> {
    /// The box's title.
    title: String,
    /// The rows, in order.
    rows: Vec<ChoiceRow<T>>,
    /// Index into `rows`.
    cursor: usize,
}

impl<T: Clone> Chooser<T> {
    /// A chooser titled `title` over `rows`, the cursor on `selected` (clamped to the rows).
    pub(super) fn new(title: impl Into<String>, rows: Vec<ChoiceRow<T>>, selected: usize) -> Self {
        let cursor = selected.min(rows.len().saturating_sub(1));
        Self {
            title: title.into(),
            rows,
            cursor,
        }
    }

    /// One act of [`views::WORKSPACES_PICK`](crate::keys::views::WORKSPACES_PICK).
    pub(super) fn on_act(&mut self, act: Act) -> ChooserOutcome<T> {
        match act {
            Act::ListDown => {
                self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1));
                ChooserOutcome::Stay
            }
            Act::ListUp => {
                self.cursor = self.cursor.saturating_sub(1);
                ChooserOutcome::Stay
            }
            Act::ConfirmYes => match self.rows.get(self.cursor) {
                Some(row) if row.enabled => ChooserOutcome::Chosen(row.value.clone()),
                Some(_) => ChooserOutcome::Disabled(self.cursor),
                None => ChooserOutcome::Closed,
            },
            Act::ConfirmNo => ChooserOutcome::Closed,
            _ => ChooserOutcome::Stay,
        }
    }

    /// The box centred over `area`, sized to its rows, `hint` on its border.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, hint: &str, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let width = self
            .rows
            .iter()
            .map(|row| {
                cells::cell_width(&row.label)
                    + row
                        .note
                        .as_ref()
                        .map_or(0, |note| NOTE_GAP + cells::cell_width(note))
            })
            .max()
            .unwrap_or(0)
            // The list's gutter, and the hint on the border with its chrome.
            .saturating_add(1)
            .max(cells::cell_width(hint) + 2)
            .max(MIN_WIDTH);
        let content = Modal::new(&self.title)
            .size(ModalSize::Fit {
                content_w: u16::try_from(width).unwrap_or(u16::MAX),
                content_h: u16::try_from(self.rows.len()).unwrap_or(u16::MAX),
            })
            .hint(hint)
            .render(frame, area, theme);
        let rows: Vec<Row<'static>> = self
            .rows
            .iter()
            .map(|row| {
                let style = if row.enabled { theme.base } else { theme.dim };
                let mut spans = vec![Span::styled(row.label.clone(), style)];
                if let Some(note) = &row.note {
                    spans.push(Span::raw(" ".repeat(NOTE_GAP)));
                    spans.push(Span::styled(note.clone(), theme.dim));
                }
                Row {
                    line: Line::from(spans),
                    kind: RowKind::Item,
                }
            })
            .collect();
        // The modal's own accent marks the cursor, as its border does (the path picker's rule).
        list::render(
            frame,
            content,
            &rows,
            Some(self.cursor),
            0,
            theme.accent,
            theme,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ChoiceRow, Chooser, ChooserOutcome};
    use crate::keys::Act;

    fn row(label: &str, enabled: bool, value: u8) -> ChoiceRow<u8> {
        ChoiceRow {
            label: label.to_owned(),
            note: None,
            enabled,
            value,
        }
    }

    #[test]
    fn the_cursor_moves_within_the_rows_and_a_choice_answers_its_value() {
        let mut chooser = Chooser::new("t", vec![row("a", true, 1), row("b", true, 2)], 9);
        assert_eq!(chooser.on_act(Act::ConfirmYes), ChooserOutcome::Chosen(2));
        assert_eq!(chooser.on_act(Act::ListDown), ChooserOutcome::Stay);
        assert_eq!(chooser.on_act(Act::ConfirmYes), ChooserOutcome::Chosen(2));
        assert_eq!(chooser.on_act(Act::ListUp), ChooserOutcome::Stay);
        assert_eq!(chooser.on_act(Act::ListUp), ChooserOutcome::Stay);
        assert_eq!(chooser.on_act(Act::ConfirmYes), ChooserOutcome::Chosen(1));
        assert_eq!(chooser.on_act(Act::ConfirmNo), ChooserOutcome::Closed);
    }

    #[test]
    fn a_disabled_row_is_landed_on_and_not_chosen() {
        let mut chooser = Chooser::new("t", vec![row("a", true, 1), row("b", false, 2)], 0);
        assert_eq!(chooser.on_act(Act::ListDown), ChooserOutcome::Stay);
        assert_eq!(chooser.on_act(Act::ConfirmYes), ChooserOutcome::Disabled(1));
    }
}
