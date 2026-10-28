//! The Backlog's key hints (MOD-115 M4 T1): the list's own keys under the shell's hint row.

use ratatui::text::{Line, Span};

use crate::keys::{Act, Hint, HintSpec, Keys};
use crate::ui::tabs::backlog::BacklogTab;

/// The Backlog's own keys under the list (MOD-67 M5 Q1 a, PA-5): `j/k move · h/l sub-tab ·
/// f filter · N new · e edit` under the defaults, 50 cells of the list pane's 53 at 100 columns.
/// The queue keys stay in the `?` box.
pub(super) const LIST_HINT: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::Pair(Act::PanePrevSubtab, Act::PaneNextSubtab, "sub-tab"),
    Hint::One(Act::BacklogOpenFilter, "filter"),
    Hint::One(Act::BacklogNewItem, "new"),
    Hint::One(Act::Edit, "edit"),
];

/// What joins the focused pane's keys and [`LIST_HINT`] in the shell's hint row (MOD-67 M6 T2).
pub(super) const LIST_SEP: &str = " \u{b7} ";

/// `line`'s spans, each with the line's own style under its own, so two lines join into one row
/// without either losing its look (MOD-67 M6 T2).
pub(super) fn styled_spans(line: Line<'static>) -> Vec<Span<'static>> {
    let base = line.style;
    line.spans
        .into_iter()
        .map(|span| {
            let style = base.patch(span.style);
            span.style(style)
        })
        .collect()
}

impl BacklogTab {
    /// The list's own keys (MOD-67 M5 Q1 a, the shell's hint row since M6 T2), read through the
    /// active pane's stack; `None` while the filter form, the item form or a sub-tab captures,
    /// since a hint for keys a field is typing would lie, and when no key of [`LIST_HINT`] is
    /// bound.
    pub(super) fn list_hint(&self, keys: &Keys) -> Option<String> {
        if self.form.is_some() || self.item_form.is_some() || self.detail.captures_input() {
            return None;
        }
        Some(keys.hint(self.detail.key_stack(), LIST_HINT)).filter(|hint| !hint.is_empty())
    }
}
