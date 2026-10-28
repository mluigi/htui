//! How a pane's content moves (MOD-115 M4 T0): [`Scroll`], the row offset a scrolled pane keeps,
//! [`Step`], the list motion a focused detail pane takes, [`PAGE`], the rows a page key moves,
//! and [`window`], the first visible row of a list kept around its cursor.
//!
//! The Backlog's detail pane and list lent them to the Requirements and Skills tabs; MOD-115 M4
//! E1 gave them this neutral home, so no tab imports another tab to scroll.

use crate::app::Handled;
use crate::keys::Act;
use crate::ui::shell::PaneId;

/// How many rows `PageDown` / `PageUp` move.
pub const PAGE: isize = 10;

/// One list motion, as a focused detail pane takes it (MOD-115 M2 §7): `list.down`, `list.up`,
/// `list.top` or `list.bottom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// A row down.
    Down,
    /// A row up.
    Up,
    /// To the first row.
    Top,
    /// To the last row.
    Bottom,
}

impl Step {
    /// The step `act` is, if it is a list motion.
    #[must_use]
    pub const fn of(act: Act) -> Option<Self> {
        match act {
            Act::ListDown => Some(Self::Down),
            Act::ListUp => Some(Self::Up),
            Act::ListTop => Some(Self::Top),
            Act::ListBottom => Some(Self::Bottom),
            _ => None,
        }
    }

    /// The step as rows to move a clamped cursor by: one either way, or as far as it goes.
    #[must_use]
    pub const fn delta(self) -> isize {
        match self {
            Self::Down => 1,
            Self::Up => -1,
            Self::Top => isize::MIN,
            Self::Bottom => isize::MAX,
        }
    }

    /// MOD-115 M2 §7, the one rule every list + detail tab routes motion by: only motion
    /// follows the focus. Whether the motion was taken away from the list: `false` when `act`
    /// is no list motion or the list is focused, so the tab's own arms take it; with the detail
    /// focused `detail` takes the step, and whatever it answers the motion is taken (a pane at
    /// its end does not hand the key back); with the activity pane focused nothing moves.
    pub fn by_focus(act: Act, focus: PaneId, detail: impl FnOnce(Self) -> Handled) -> bool {
        let Some(step) = Self::of(act).filter(|_| focus != PaneId::List) else {
            return false;
        };
        if focus == PaneId::Detail {
            let _ = detail(step);
        }
        true
    }
}

/// Vertical scroll shared by every detail sub-tab, and by the panes that scroll as they do.
///
/// The clamp is against the number of *unwrapped* rows, which is a lower bound on the number of
/// rendered ones, so the last page can never be scrolled off the top even though the wrapped
/// height is only known at render time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    /// First rendered row.
    offset: u16,
}

impl Scroll {
    /// The current offset, for `Paragraph::scroll` or for skipping table rows.
    #[must_use]
    pub const fn offset(self) -> u16 {
        self.offset
    }

    /// The offset as a row index.
    #[must_use]
    pub const fn skip(self) -> usize {
        self.offset as usize
    }

    /// A scroll whose first rendered row is `offset`: the Notes pane's bottom after the user's
    /// own note lands (MOD-13 review L1). The next key clamps it as any other.
    #[must_use]
    pub const fn at(offset: u16) -> Self {
        Self { offset }
    }

    /// Back to the top: what a new item or a new reply does.
    pub const fn reset(&mut self) {
        self.offset = 0;
    }

    /// One pane-scroll act (MOD-67 M4): `pane.scroll_down`/`scroll_up` one row,
    /// `pane.page_down`/`page_up` ten, clamped to the last row; `Pass` for any other act.
    pub fn apply(&mut self, act: Act, len: usize) -> Handled {
        let step: isize = match act {
            Act::PaneScrollDown => 1,
            Act::PaneScrollUp => -1,
            Act::PanePageDown => PAGE,
            Act::PanePageUp => -PAGE,
            _ => return Handled::Pass,
        };
        self.step(step, len)
    }

    /// A list motion over `len` rows while the pane is focused (MOD-115 M2 §7): a row, as
    /// `pane.scroll_down`/`scroll_up`, or to either end, clamped to the last row.
    pub fn follow(&mut self, step: Step, len: usize) -> Handled {
        self.step(step.delta(), len)
    }

    /// Moves `step` rows, clamped to the last of `len` rows.
    fn step(&mut self, step: isize, len: usize) -> Handled {
        let max = u16::try_from(len.saturating_sub(1)).unwrap_or(u16::MAX);
        let next = usize::from(self.offset).saturating_add_signed(step);
        self.offset = u16::try_from(next).unwrap_or(u16::MAX).min(max);
        Handled::Consumed
    }
}

/// First visible row: enough to keep `cursor` inside a `height`-tall viewport.
///
/// Stateless on purpose — it is a function of the cursor, so no scroll offset can survive a fold
/// or a reply that shortens the list.
#[must_use]
pub fn window(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    cursor
        .saturating_sub(height / 2)
        .min(len.saturating_sub(height))
}

#[cfg(test)]
mod tests {
    use super::{Scroll, Step, window};
    use crate::app::Handled;
    use crate::keys::Act;
    use crate::ui::shell::PaneId;

    /// MOD-115 M2 §7 (RV3): only motion follows the focus. A list motion with the list focused,
    /// or any other act, is the tab's own; with the detail focused the detail takes the step,
    /// and the motion is taken even when the detail declines it; with the activity pane focused
    /// it is taken and nothing moves.
    #[test]
    fn by_focus_routes_only_motion_away_from_the_list() {
        let never = |_: Step| -> Handled { panic!("the detail is not asked") };
        assert!(!Step::by_focus(Act::ListDown, PaneId::List, never));
        assert!(!Step::by_focus(Act::Edit, PaneId::Detail, never));
        assert!(!Step::by_focus(Act::ListFold, PaneId::Activity, never));
        let mut seen = None;
        let taken = Step::by_focus(Act::ListBottom, PaneId::Detail, |step| {
            seen = Some(step);
            Handled::Pass
        });
        assert_eq!((taken, seen), (true, Some(Step::Bottom)));
        assert!(Step::by_focus(Act::ListUp, PaneId::Activity, never));
    }

    /// `Scroll::apply`, the pane scroll's one entry (MOD-67 M5 dropped the key-matching
    /// `on_key`): a row or a page per act, clamped to the last row; any other act passes.
    #[test]
    fn apply_steps_and_clamps() {
        for len in [0, 1] {
            let mut scroll = Scroll::at(3);
            assert_eq!(scroll.apply(Act::PanePageDown, len), Handled::Consumed);
            assert_eq!(scroll.offset(), 0, "nothing to scroll over {len} rows");
        }

        let mut scroll = Scroll::default();
        assert_eq!(scroll.apply(Act::PaneScrollDown, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 1);
        assert_eq!(scroll.apply(Act::PanePageDown, 25), Handled::Consumed);
        assert_eq!(scroll.offset(), 11);
        assert_eq!(scroll.apply(Act::PanePageDown, 15), Handled::Consumed);
        assert_eq!(scroll.offset(), 14, "the page clamps at len - 1");
        assert_eq!(scroll.apply(Act::PanePageUp, 15), Handled::Consumed);
        assert_eq!(scroll.offset(), 4);
        assert_eq!(scroll.apply(Act::PanePageUp, 15), Handled::Consumed);
        assert_eq!(scroll.offset(), 0);

        let mut scroll = Scroll::at(2);
        assert_eq!(scroll.apply(Act::Edit, 5), Handled::Pass);
        assert_eq!(scroll.apply(Act::ListDown, 5), Handled::Pass);
        assert_eq!(scroll, Scroll::at(2), "a passed act moves nothing");
    }

    /// MOD-115 M2 §7: the four list motions are the steps a focused detail pane takes; every
    /// other act is none.
    #[test]
    fn the_list_motions_are_the_detail_steps() {
        assert_eq!(Step::of(Act::ListDown), Some(Step::Down));
        assert_eq!(Step::of(Act::ListUp), Some(Step::Up));
        assert_eq!(Step::of(Act::ListTop), Some(Step::Top));
        assert_eq!(Step::of(Act::ListBottom), Some(Step::Bottom));
        for act in [
            Act::ListFold,
            Act::PaneScrollDown,
            Act::PanePageDown,
            Act::Edit,
        ] {
            assert_eq!(Step::of(act), None, "{act:?}");
        }
    }

    /// MOD-115 M2 §7: a step scrolls a row, or to either end, clamped to the last row.
    #[test]
    fn a_step_scrolls_a_row_or_to_an_end() {
        let mut scroll = Scroll::default();
        assert_eq!(scroll.follow(Step::Down, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 1);
        assert_eq!(scroll.follow(Step::Bottom, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 4, "the last of five rows");
        assert_eq!(scroll.follow(Step::Down, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 4, "clamped");
        assert_eq!(scroll.follow(Step::Up, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 3);
        assert_eq!(scroll.follow(Step::Top, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 0);
        assert_eq!(scroll.follow(Step::Up, 5), Handled::Consumed);
        assert_eq!(scroll.offset(), 0, "clamped at the top");
        assert_eq!(scroll.follow(Step::Bottom, 0), Handled::Consumed);
        assert_eq!(scroll.offset(), 0, "nothing to scroll over");
    }

    /// MOD-115 M4 T0: the first visible row keeps the cursor in a `height`-tall viewport, the
    /// cursor centred where the list allows, and never scrolls past the last full page; a list
    /// that fits, or a viewport with no rows, starts at the top.
    #[test]
    fn window_keeps_the_cursor_in_view_and_never_past_the_end() {
        assert_eq!(window(7, 10, 10), 0, "a list that fits starts at the top");
        assert_eq!(window(3, 4, 10), 0, "a shorter list too");
        assert_eq!(window(5, 30, 0), 0, "no rows to show");
        assert_eq!(window(0, 30, 10), 0, "the cursor at the top");
        assert_eq!(window(4, 30, 10), 0, "above the middle of the first page");
        assert_eq!(window(12, 30, 10), 7, "the cursor centred");
        assert_eq!(
            window(15, 30, 9),
            11,
            "an odd height centres on its lower half"
        );
        assert_eq!(window(29, 30, 10), 20, "clamped to the last full page");
        assert_eq!(window(40, 30, 10), 20, "a cursor past the end clamps too");
        for cursor in 0..30 {
            let first = window(cursor, 30, 10);
            assert!(first <= cursor && cursor < first + 10, "{cursor} in view");
        }
    }
}
