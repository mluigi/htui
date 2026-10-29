//! The Backlog's key hints (MOD-115 M4 T1) and which pane holds the keyboard (M4 B1, C4): each
//! pane carries its own keys on its bottom border while it is focused (Q2).

use ratatui::text::{Line, Span};

use crate::app::Ctx;
use crate::keys::{Act, Hint, HintSpec, Keys};
use crate::ui::kit::chips::key_line;
use crate::ui::kit::pane::Focus;
use crate::ui::shell::PaneId;
use crate::ui::tabs::backlog::BacklogTab;
use crate::ui::tabs::registry::Tab as _;

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

    /// The pane that holds the keyboard while something takes every key (MOD-115 M4 C4): the
    /// Filter pane while the filter form is open, the detail while the item form (or its
    /// divergence view) is open or a sub-tab captures; `None` while browsing, when the shell's
    /// focus decides.
    pub(super) fn keyboard_pane(&self) -> Option<Holder> {
        if self.form.is_some() {
            Some(Holder::Filter)
        } else if self.item_form.is_some() || self.detail.captures_input() {
            Some(Holder::Detail)
        } else {
            None
        }
    }

    /// How the list, the Filter pane and the detail are drawn this frame (C4): the pane holding
    /// the keyboard is the focused one and the others are not; browsing, the shell's focused
    /// pane. `activity` is whether the shell's activity pane is on screen beside them.
    pub(super) fn focuses(&self, ctx: &Ctx<'_>, activity: bool) -> Focuses {
        let held = holder(ctx, activity);
        let none = Focus::Unfocused;
        match self.keyboard_pane() {
            Some(Holder::Filter) => Focuses {
                list: none,
                filter: held,
                detail: none,
            },
            Some(Holder::Detail) => Focuses {
                list: none,
                filter: none,
                detail: held,
            },
            None => Focuses {
                list: ctx.pane_focus(PaneId::List),
                filter: none,
                detail: ctx.pane_focus(PaneId::Detail),
            },
        }
    }

    /// The list pane's hints (Q2): [`LIST_HINT`] through the keys in force, `None` while
    /// something captures or no key of it is bound.
    pub(super) fn list_pane_hint(&self, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.list_hint(ctx.keys())
            .and_then(|hint| key_line(hint, ctx.theme))
    }

    /// The Filter pane's hints (Q2): the open form's focused row's keys.
    pub(super) fn filter_pane_hint(&self, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.form
            .as_ref()
            .and_then(|form| key_line(form.key_hint(ctx.keys()), ctx.theme))
    }

    /// The hints of the detail, or of the pane drawn in its place, `width` cells wide (Q2):
    /// the divergence view's, the item form's or a capturing sub-tab's keys; browsing, the
    /// sub-tab's own then ` · ` and [`LIST_HINT`] when both fit the pane, else the sub-tab's,
    /// else [`LIST_HINT`]. That is `Tab::key_hint`'s composition, fitted to the pane: two border
    /// cells and a rule cell on each side.
    pub(super) fn detail_pane_hint(&self, width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.key_hint(width.saturating_sub(4), ctx)
    }
}

/// The Backlog pane that holds the keyboard (MOD-115 M4 C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Holder {
    /// The open filter form, at the bottom of the list pane.
    Filter,
    /// The detail: the item form or its divergence view in its place, or a capturing sub-tab.
    Detail,
}

/// How each Backlog pane is drawn this frame (MOD-115 M4 C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Focuses {
    /// The list pane.
    pub(super) list: Focus,
    /// The Filter pane, while the form is open.
    pub(super) filter: Focus,
    /// The detail pane, or the pane drawn in its place.
    pub(super) detail: Focus,
}

/// How the pane that holds the keyboard is drawn (MOD-115 M4 C4): focused unless something
/// covers the tab (`ctx.pane_focus`), or the shell's activity pane is on screen (`activity`) and
/// has the focus.
pub(super) fn holder(ctx: &Ctx<'_>, activity: bool) -> Focus {
    let focus = ctx.focus();
    if activity && focus == PaneId::Activity {
        Focus::Unfocused
    } else {
        ctx.pane_focus(focus)
    }
}
