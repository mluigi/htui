//! The layout engine (MOD-115 M2 D2, blueprint §6.1): pure geometry from a tab's body region and
//! [`TabLayout`] to its [`PaneAreas`], the visible panes' focus order, mouse hits and seams; the
//! steps the `[layout]` keys and the seam drags take on a [`TabLayout`]; and the note a hidden
//! pane leaves ([`hidden_note`]).
//!
//! The presets: [`Preset::Ide`] puts the list beside the detail; [`Preset::IdeActivity`] puts
//! that pair over the activity pane (`activity_pct` of the height); [`Preset::Columns`] lays the
//! list, the detail and the activity pane out as columns (`activity_pct` of the width);
//! [`Preset::Focus`] shows the detail alone. The list's [`ListSize`] is measured against the
//! width the list shares with the detail.
//!
//! Splits use ratatui's own constraints, so the M2 defaults reproduce the splits the tabs drew
//! before the shell (Backlog 55/45, Requirements 45/55, Skills' 32-cell list) cell for cell. A
//! split is clamped to the pane minimums only when the region holds them all; a smaller region
//! keeps the unclamped split, and nothing is ever hidden for lack of room.
//!
//! A zoomed pane takes the whole region. The detail has no hidden reason in [`PaneAreas`]: when
//! the list or the activity pane is zoomed, the detail is a zero-sized rect at the region's
//! origin, which [`visible`], [`step`] and [`hit`] skip.

use ratatui::layout::{Constraint, Layout, Position, Rect};

use super::{HiddenBy, ListSize, PaneAreas, PaneId, Preset, TabLayout};

/// The list's narrowest width in cells, once the region holds every minimum.
pub const MIN_LIST: u16 = 16;
/// The detail's narrowest width in cells, once the region holds every minimum.
pub const MIN_DETAIL: u16 = 24;
/// The activity pane's fewest rows under [`Preset::IdeActivity`] (and the panes above it keep as
/// many).
pub const MIN_ACTIVITY_ROWS: u16 = 4;
/// The activity pane's narrowest width in cells under [`Preset::Columns`].
pub const MIN_ACTIVITY_COLS: u16 = 20;

/// A border between two panes the mouse can drag (`set_list_at`, `set_activity_at`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seam {
    /// Between the list and the detail.
    List,
    /// Between the activity pane and the panes beside or above it.
    Activity,
}

/// The panes in focus-cycle order.
const ORDER: [PaneId; 3] = [PaneId::List, PaneId::Detail, PaneId::Activity];

/// Where each pane of `layout` is drawn in `region`, and why a missing pane is missing. `focus`
/// matters only while zoomed: the focused pane, or the detail if it is hidden, takes the region.
#[must_use]
pub fn areas(region: Rect, layout: &TabLayout, focus: PaneId) -> PaneAreas {
    let has_list = layout.preset != Preset::Focus;
    let has_activity = matches!(layout.preset, Preset::IdeActivity | Preset::Columns);
    let mut list_hidden = if !has_list {
        Some(HiddenBy::Preset)
    } else if !layout.list_on {
        Some(HiddenBy::Toggle)
    } else {
        None
    };
    let mut activity_hidden = if !has_activity {
        Some(HiddenBy::Preset)
    } else if !layout.activity_on {
        Some(HiddenBy::Toggle)
    } else {
        None
    };

    if layout.zoom {
        let zoomed = match focus {
            PaneId::List if list_hidden.is_none() => PaneId::List,
            PaneId::Activity if activity_hidden.is_none() => PaneId::Activity,
            _ => PaneId::Detail,
        };
        let away = |pane: PaneId, hidden: &mut Option<HiddenBy>| {
            if pane != zoomed && hidden.is_none() {
                *hidden = Some(HiddenBy::Zoom);
            }
        };
        away(PaneId::List, &mut list_hidden);
        away(PaneId::Activity, &mut activity_hidden);
        let gone = Rect::new(region.x, region.y, 0, 0);
        return PaneAreas {
            list: (zoomed == PaneId::List).then_some(region),
            detail: if zoomed == PaneId::Detail {
                region
            } else {
                gone
            },
            activity: (zoomed == PaneId::Activity).then_some(region),
            list_hidden,
            activity_hidden,
        };
    }

    let list_on = list_hidden.is_none();
    let (pair, activity) = match (layout.preset, activity_hidden) {
        (Preset::IdeActivity, None) => {
            let (top, bottom) = split_rows(region, layout.activity_pct);
            (top, Some(bottom))
        }
        (Preset::Columns, None) => {
            let others = if list_on {
                MIN_LIST + MIN_DETAIL
            } else {
                MIN_DETAIL
            };
            let (left, right) = split_columns(region, layout.activity_pct, others);
            (left, Some(right))
        }
        _ => (region, None),
    };
    let (list, detail) = if list_on {
        let (list, detail) = split_list(pair, layout.list);
        (Some(list), detail)
    } else {
        (None, pair)
    };
    PaneAreas {
        list,
        detail,
        activity,
        list_hidden,
        activity_hidden,
    }
}

/// `area` split into the list and the detail beside it, the list clamped into
/// `MIN_LIST..=width - MIN_DETAIL` when `area` holds both.
fn split_list(area: Rect, list: ListSize) -> (Rect, Rect) {
    let [left, right] = match list {
        ListSize::Percent(p) => {
            let p = p.min(100);
            Layout::horizontal([Constraint::Percentage(p), Constraint::Percentage(100 - p)])
                .areas(area)
        }
        ListSize::Cells(n) => {
            Layout::horizontal([Constraint::Length(n), Constraint::Min(1)]).areas(area)
        }
    };
    match clamped(left.width, area.width, MIN_LIST, MIN_DETAIL) {
        Some(width) => cut_columns(area, width),
        None => (left, right),
    }
}

/// `area` split into the panes above and the activity pane under them (`pct` of the height),
/// the activity clamped into `MIN_ACTIVITY_ROWS..=height - MIN_ACTIVITY_ROWS` when `area` holds
/// both.
fn split_rows(area: Rect, pct: u16) -> (Rect, Rect) {
    let pct = pct.min(100);
    let [top, bottom] = Layout::vertical([
        Constraint::Percentage(100 - pct),
        Constraint::Percentage(pct),
    ])
    .areas(area);
    match clamped(
        bottom.height,
        area.height,
        MIN_ACTIVITY_ROWS,
        MIN_ACTIVITY_ROWS,
    ) {
        Some(height) => {
            let top = Rect {
                height: area.height - height,
                ..area
            };
            let bottom = Rect {
                y: top.bottom(),
                height,
                ..area
            };
            (top, bottom)
        }
        None => (top, bottom),
    }
}

/// `area` split into the panes on the left and the activity column on the right (`pct` of the
/// width), the activity clamped into `MIN_ACTIVITY_COLS..=width - others` when `area` holds both.
fn split_columns(area: Rect, pct: u16, others: u16) -> (Rect, Rect) {
    let pct = pct.min(100);
    let [left, right] = Layout::horizontal([
        Constraint::Percentage(100 - pct),
        Constraint::Percentage(pct),
    ])
    .areas(area);
    match clamped(right.width, area.width, MIN_ACTIVITY_COLS, others) {
        Some(width) => cut_columns(area, area.width - width),
        None => (left, right),
    }
}

/// `size` clamped into `low..=total - rest`, or `None` when `total` cannot hold both minimums
/// (the unclamped split stands) or `size` is already in range.
fn clamped(size: u16, total: u16, low: u16, rest: u16) -> Option<u16> {
    if total < low.saturating_add(rest) {
        return None;
    }
    let size_in = size.clamp(low, total - rest);
    (size_in != size).then_some(size_in)
}

/// `area` cut into a left part `width` cells wide and the rest.
fn cut_columns(area: Rect, width: u16) -> (Rect, Rect) {
    let left = Rect { width, ..area };
    let right = Rect {
        x: left.right(),
        width: area.width - width,
        ..area
    };
    (left, right)
}

/// The rect `pane` is drawn in, if it is shown.
fn rect_of(areas: &PaneAreas, pane: PaneId) -> Option<Rect> {
    match pane {
        PaneId::List => areas.list,
        PaneId::Detail => Some(areas.detail).filter(|detail| !detail.is_empty()),
        PaneId::Activity => areas.activity,
    }
}

/// The panes `areas` shows, in focus-cycle order: list, detail, activity.
#[must_use]
pub fn visible(areas: &PaneAreas) -> Vec<PaneId> {
    ORDER
        .into_iter()
        .filter(|pane| rect_of(areas, *pane).is_some())
        .collect()
}

/// The next visible pane after `from` (or before it, when `forward` is false), wrapping; `from`
/// itself when nothing else is visible. A hidden `from` steps from its place in the cycle.
#[must_use]
pub fn step(areas: &PaneAreas, from: PaneId, forward: bool) -> PaneId {
    let at = ORDER.iter().position(|pane| *pane == from).unwrap_or(0);
    let len = ORDER.len();
    (1..=len)
        .map(|k| {
            if forward {
                ORDER[(at + k) % len]
            } else {
                ORDER[(at + len - k) % len]
            }
        })
        .find(|pane| rect_of(areas, *pane).is_some())
        .unwrap_or(from)
}

/// The visible pane under the cell `(x, y)`, if any.
#[must_use]
pub fn hit(areas: &PaneAreas, x: u16, y: u16) -> Option<PaneId> {
    ORDER
        .into_iter()
        .find(|pane| rect_of(areas, *pane).is_some_and(|rect| rect.contains(Position::new(x, y))))
}

/// The seam under the cell `(x, y)`, if any: the two border lines where two visible panes meet.
#[must_use]
pub fn seam(areas: &PaneAreas, x: u16, y: u16) -> Option<Seam> {
    let at = Position::new(x, y);
    let detail = rect_of(areas, PaneId::Detail);
    if let (Some(list), Some(detail)) = (areas.list, detail)
        && (list.contains(at) && x + 1 == list.right() || detail.contains(at) && x == detail.x)
    {
        return Some(Seam::List);
    }
    let activity = areas.activity?;
    let neighbour = detail.or(areas.list)?;
    let beside = activity.y == neighbour.y;
    let on_activity = activity.contains(at)
        && if beside {
            x == activity.x
        } else {
            y == activity.y
        };
    let on_neighbour = [areas.list, detail].into_iter().flatten().any(|pane| {
        pane.contains(at)
            && if beside {
                x + 1 == pane.right() && pane.right() == activity.x
            } else {
                y + 1 == pane.bottom() && pane.bottom() == activity.y
            }
    });
    (on_activity || on_neighbour).then_some(Seam::Activity)
}

/// The labels the hidden-pane note names, already resolved from the keymap (`Keys::label` of
/// `layout.toggle_list`, `layout.toggle_activity`, `layout.next_preset`, `layout.zoom`); a
/// `None` label (the act has no chord) drops its part of the note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteLabels {
    /// `layout.toggle_list`.
    pub toggle_list: Option<String>,
    /// `layout.toggle_activity`.
    pub toggle_activity: Option<String>,
    /// `layout.next_preset`.
    pub next_preset: Option<String>,
    /// `layout.zoom`.
    pub zoom: Option<String>,
}

/// What a hidden pane says about bringing it back (`R-TUI-12`), for the detail's bottom border:
/// `z unzoom` while another pane is zoomed, `F2 show list · p layouts` while the list is hidden
/// by its toggle or the preset, `F3 show activity` while the activity pane is toggled off;
/// joined with ` · `. `None` when nothing is hidden but by the preset's lack of an activity pane.
#[must_use]
pub fn hidden_note(areas: &PaneAreas, labels: &NoteLabels) -> Option<String> {
    let part =
        |label: &Option<String>, what: &str| label.as_ref().map(|key| format!("{key} {what}"));
    let detail_zoomed_away =
        areas.detail.is_empty() && (areas.list.is_some() || areas.activity.is_some());
    let zoomed = detail_zoomed_away
        || areas.list_hidden == Some(HiddenBy::Zoom)
        || areas.activity_hidden == Some(HiddenBy::Zoom);
    let list = matches!(areas.list_hidden, Some(HiddenBy::Toggle | HiddenBy::Preset));
    let activity = areas.activity_hidden == Some(HiddenBy::Toggle);
    let parts: Vec<String> = [
        zoomed.then(|| part(&labels.zoom, "unzoom")),
        list.then(|| part(&labels.toggle_list, "show list")),
        list.then(|| part(&labels.next_preset, "layouts")),
        activity.then(|| part(&labels.toggle_activity, "show activity")),
    ]
    .into_iter()
    .flatten()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The list's percentage step (`layout.grow_list` / `shrink_list`).
const LIST_PERCENT_STEP: u16 = 5;
/// The list's percentage bounds.
const LIST_PERCENT_RANGE: (u16, u16) = (15, 85);
/// The list's cell step; the cell width is clamped where it is drawn.
const LIST_CELLS_STEP: u16 = 4;
/// The activity pane's percentage step (`layout.grow_activity` / `shrink_activity`).
const ACTIVITY_STEP: u16 = 5;
/// The activity pane's percentage bounds.
const ACTIVITY_RANGE: (u16, u16) = (15, 60);

/// `value` in `range`, inclusive.
fn within(value: u16, (low, high): (u16, u16)) -> u16 {
    value.clamp(low, high)
}

/// `part` of `whole` in percent, rounded to the nearest.
fn percent_of(part: u16, whole: u16) -> u16 {
    if whole == 0 {
        return 0;
    }
    let rounded = (u32::from(part) * 100 + u32::from(whole) / 2) / u32::from(whole);
    u16::try_from(rounded).unwrap_or(u16::MAX)
}

impl TabLayout {
    /// The next preset in [`Preset::ALL`], wrapping (`layout.next_preset`).
    pub fn next_preset(&mut self) {
        self.cycle_preset(1);
    }

    /// The previous preset, wrapping (`layout.prev_preset`).
    pub fn prev_preset(&mut self) {
        self.cycle_preset(Preset::ALL.len() - 1);
    }

    fn cycle_preset(&mut self, by: usize) {
        let at = Preset::ALL
            .iter()
            .position(|preset| *preset == self.preset)
            .unwrap_or(0);
        self.preset = Preset::ALL[(at + by) % Preset::ALL.len()];
    }

    /// A wider list (`layout.grow_list`): 5 points of a percentage up to 85, or 4 cells.
    pub fn grow_list(&mut self) {
        self.list = match self.list {
            ListSize::Percent(p) => ListSize::Percent(within(
                p.saturating_add(LIST_PERCENT_STEP),
                LIST_PERCENT_RANGE,
            )),
            ListSize::Cells(n) => ListSize::Cells(n.saturating_add(LIST_CELLS_STEP)),
        };
    }

    /// A narrower list (`layout.shrink_list`): 5 points of a percentage down to 15, or 4 cells
    /// down to one.
    pub fn shrink_list(&mut self) {
        self.list = match self.list {
            ListSize::Percent(p) => ListSize::Percent(within(
                p.saturating_sub(LIST_PERCENT_STEP),
                LIST_PERCENT_RANGE,
            )),
            ListSize::Cells(n) => ListSize::Cells(n.saturating_sub(LIST_CELLS_STEP).max(1)),
        };
    }

    /// A larger activity pane (`layout.grow_activity`): 5 points, up to 60.
    pub fn grow_activity(&mut self) {
        self.activity_pct = within(
            self.activity_pct.saturating_add(ACTIVITY_STEP),
            ACTIVITY_RANGE,
        );
    }

    /// A smaller activity pane (`layout.shrink_activity`): 5 points, down to 15.
    pub fn shrink_activity(&mut self) {
        self.activity_pct = within(
            self.activity_pct.saturating_sub(ACTIVITY_STEP),
            ACTIVITY_RANGE,
        );
    }

    /// Hides a shown list, shows a hidden one (`layout.toggle_list`); a list hidden by
    /// [`Preset::Focus`] is shown by leaving it for [`Preset::Ide`].
    pub fn toggle_list(&mut self) {
        if self.preset == Preset::Focus {
            self.preset = Preset::Ide;
            self.list_on = true;
        } else {
            self.list_on = !self.list_on;
        }
    }

    /// Hides a shown activity pane, shows a hidden one (`layout.toggle_activity`); a preset
    /// without one moves to [`Preset::IdeActivity`].
    pub fn toggle_activity(&mut self) {
        let has_activity = matches!(self.preset, Preset::IdeActivity | Preset::Columns);
        if !has_activity {
            self.preset = Preset::IdeActivity;
            self.activity_on = true;
        } else {
            self.activity_on = !self.activity_on;
        }
    }

    /// Zooms the focused pane, or unzooms (`layout.zoom`).
    pub fn toggle_zoom(&mut self) {
        self.zoom = !self.zoom;
    }

    /// The list seam dragged to column `x` of `region` (the tab's body): the list's right border
    /// moves there. The size keeps its kind: a percentage of the width the list shares with the
    /// detail (15..=85), or cells (clamped to the minimums when the pair holds them). Nothing
    /// changes while the list or the detail is hidden.
    pub fn set_list_at(&mut self, region: Rect, x: u16) {
        let drawn = areas(region, self, PaneId::List);
        let (Some(list), Some(detail)) = (drawn.list, rect_of(&drawn, PaneId::Detail)) else {
            return;
        };
        let shared = list.width + detail.width;
        if shared < 2 {
            return;
        }
        let width = (x.saturating_add(1))
            .saturating_sub(list.x)
            .clamp(1, shared - 1);
        self.list = match self.list {
            ListSize::Percent(_) => {
                ListSize::Percent(within(percent_of(width, shared), LIST_PERCENT_RANGE))
            }
            ListSize::Cells(_) => {
                ListSize::Cells(clamped(width, shared, MIN_LIST, MIN_DETAIL).unwrap_or(width))
            }
        };
    }

    /// The activity seam dragged to the cell `(x, y)` of `region` (the tab's body): the activity
    /// pane's top border (stacked) or left border (columns) moves there, its share of the
    /// region kept in 15..=60. Nothing changes while the activity pane is hidden.
    pub fn set_activity_at(&mut self, region: Rect, x: u16, y: u16) {
        let drawn = areas(region, self, PaneId::List);
        if drawn.activity.is_none() {
            return;
        }
        let share = match self.preset {
            Preset::IdeActivity => percent_of(region.bottom().saturating_sub(y), region.height),
            Preset::Columns => percent_of(region.right().saturating_sub(x), region.width),
            Preset::Ide | Preset::Focus => return,
        };
        self.activity_pct = within(share, ACTIVITY_RANGE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Rect = Rect::new(0, 1, 100, 30);
    const NARROW: Rect = Rect::new(0, 1, 60, 20);

    fn layout(preset: Preset) -> TabLayout {
        TabLayout {
            preset,
            ..TabLayout::DEFAULT
        }
    }

    fn with_list(list: ListSize) -> TabLayout {
        TabLayout {
            list,
            ..TabLayout::DEFAULT
        }
    }

    /// The split each tab drew before the shell (`backlog/mod.rs` `panes`, `requirements/mod.rs`
    /// `render_browse`, `library.rs`/`templates.rs` `render_browse`).
    fn today(region: Rect, list: ListSize) -> [Rect; 2] {
        match list {
            ListSize::Percent(p) => {
                Layout::horizontal([Constraint::Percentage(p), Constraint::Percentage(100 - p)])
                    .areas(region)
            }
            ListSize::Cells(n) => {
                Layout::horizontal([Constraint::Length(n), Constraint::Min(1)]).areas(region)
            }
        }
    }

    #[test]
    fn the_four_default_splits_equal_the_tabs_own_splits_at_both_sizes() {
        // Backlog 55/45, Requirements 45/55, Library and Templates a 32-cell list.
        for list in [
            ListSize::Percent(55),
            ListSize::Percent(45),
            ListSize::Cells(32),
            ListSize::Cells(32),
        ] {
            for region in [WIDE, NARROW] {
                let got = areas(region, &with_list(list), PaneId::List);
                let [left, right] = today(region, list);
                assert_eq!(got.list, Some(left), "{list:?} in {region:?}");
                assert_eq!(got.detail, right, "{list:?} in {region:?}");
                assert_eq!(got.activity, None);
                assert_eq!(got.list_hidden, None);
                assert_eq!(got.activity_hidden, Some(HiddenBy::Preset));
            }
        }
    }

    #[test]
    fn ide_puts_the_list_beside_the_detail() {
        let got = areas(WIDE, &TabLayout::DEFAULT, PaneId::Detail);
        assert_eq!(got.list, Some(Rect::new(0, 1, 50, 30)));
        assert_eq!(got.detail, Rect::new(50, 1, 50, 30));
        assert_eq!(got.activity, None);
        assert_eq!(got.activity_hidden, Some(HiddenBy::Preset));
    }

    #[test]
    fn ide_activity_puts_the_pair_over_the_activity_pane() {
        let got = areas(WIDE, &layout(Preset::IdeActivity), PaneId::List);
        // 30% of 30 rows is the activity pane's 9; the pair keeps the 21 above.
        assert_eq!(got.list, Some(Rect::new(0, 1, 50, 21)));
        assert_eq!(got.detail, Rect::new(50, 1, 50, 21));
        assert_eq!(got.activity, Some(Rect::new(0, 22, 100, 9)));
        assert_eq!((got.list_hidden, got.activity_hidden), (None, None));
    }

    #[test]
    fn columns_lays_the_three_panes_side_by_side() {
        let got = areas(WIDE, &layout(Preset::Columns), PaneId::List);
        // The activity pane takes 30% of the width; the list half of the 70 left.
        assert_eq!(got.activity, Some(Rect::new(70, 1, 30, 30)));
        assert_eq!(got.list, Some(Rect::new(0, 1, 35, 30)));
        assert_eq!(got.detail, Rect::new(35, 1, 35, 30));
        assert_eq!((got.list_hidden, got.activity_hidden), (None, None));
    }

    #[test]
    fn focus_shows_the_detail_alone() {
        let got = areas(WIDE, &layout(Preset::Focus), PaneId::List);
        assert_eq!(got.detail, WIDE);
        assert_eq!(got.list, None);
        assert_eq!(got.activity, None);
        assert_eq!(got.list_hidden, Some(HiddenBy::Preset));
        assert_eq!(got.activity_hidden, Some(HiddenBy::Preset));
    }

    #[test]
    fn a_toggled_off_list_gives_the_detail_its_width() {
        let off = TabLayout {
            list_on: false,
            ..TabLayout::DEFAULT
        };
        let got = areas(WIDE, &off, PaneId::List);
        assert_eq!(got.list, None);
        assert_eq!(got.list_hidden, Some(HiddenBy::Toggle));
        assert_eq!(got.detail, WIDE);
        let columns = areas(
            WIDE,
            &TabLayout {
                preset: Preset::Columns,
                ..off
            },
            PaneId::List,
        );
        assert_eq!(columns.detail, Rect::new(0, 1, 70, 30));
        assert_eq!(columns.activity, Some(Rect::new(70, 1, 30, 30)));
    }

    #[test]
    fn a_toggled_off_activity_pane_is_hidden_by_the_toggle_only_where_the_preset_has_one() {
        let off = |preset| TabLayout {
            preset,
            activity_on: false,
            ..TabLayout::DEFAULT
        };
        let stacked = areas(WIDE, &off(Preset::IdeActivity), PaneId::List);
        assert_eq!(stacked.activity, None);
        assert_eq!(stacked.activity_hidden, Some(HiddenBy::Toggle));
        assert_eq!(stacked.list, Some(Rect::new(0, 1, 50, 30)));
        let columns = areas(WIDE, &off(Preset::Columns), PaneId::List);
        assert_eq!(columns.activity_hidden, Some(HiddenBy::Toggle));
        assert_eq!(columns.detail, Rect::new(50, 1, 50, 30));
        for preset in [Preset::Ide, Preset::Focus] {
            assert_eq!(
                areas(WIDE, &off(preset), PaneId::List).activity_hidden,
                Some(HiddenBy::Preset),
                "{preset:?} has no activity pane to toggle"
            );
        }
    }

    #[test]
    fn a_zoomed_pane_takes_the_region_and_hides_the_others_by_zoom() {
        let zoomed = |preset| TabLayout {
            preset,
            zoom: true,
            ..TabLayout::DEFAULT
        };
        let detail = areas(WIDE, &zoomed(Preset::Columns), PaneId::Detail);
        assert_eq!(detail.detail, WIDE);
        assert_eq!((detail.list, detail.activity), (None, None));
        assert_eq!(detail.list_hidden, Some(HiddenBy::Zoom));
        assert_eq!(detail.activity_hidden, Some(HiddenBy::Zoom));

        let list = areas(WIDE, &zoomed(Preset::Ide), PaneId::List);
        assert_eq!(list.list, Some(WIDE));
        assert!(list.detail.is_empty(), "the zoomed-away detail has no area");
        assert_eq!(list.activity_hidden, Some(HiddenBy::Preset), "kept");
        assert_eq!(visible(&list), [PaneId::List]);

        let activity = areas(WIDE, &zoomed(Preset::IdeActivity), PaneId::Activity);
        assert_eq!(activity.activity, Some(WIDE));
        assert_eq!(activity.list_hidden, Some(HiddenBy::Zoom));
        assert!(activity.detail.is_empty());
    }

    #[test]
    fn zoom_on_a_hidden_focus_zooms_the_detail_and_keeps_the_first_reason() {
        let layout = TabLayout {
            list_on: false,
            zoom: true,
            ..TabLayout::DEFAULT
        };
        let got = areas(WIDE, &layout, PaneId::List);
        assert_eq!(got.detail, WIDE);
        assert_eq!(got.list_hidden, Some(HiddenBy::Toggle));
        let ide = areas(
            WIDE,
            &TabLayout {
                zoom: true,
                ..TabLayout::DEFAULT
            },
            PaneId::Activity,
        );
        assert_eq!(ide.detail, WIDE, "Ide has no activity pane to zoom");
        assert_eq!(ide.list_hidden, Some(HiddenBy::Zoom));
    }

    #[test]
    fn the_list_is_clamped_to_both_minimums_when_the_region_holds_them() {
        let narrow_list = areas(WIDE, &with_list(ListSize::Percent(5)), PaneId::List);
        assert_eq!(narrow_list.list.map(|r| r.width), Some(MIN_LIST));
        assert_eq!(narrow_list.detail, Rect::new(16, 1, 84, 30));
        let wide_list = areas(WIDE, &with_list(ListSize::Cells(90)), PaneId::List);
        assert_eq!(wide_list.list.map(|r| r.width), Some(100 - MIN_DETAIL));
        assert_eq!(wide_list.detail.width, MIN_DETAIL);
        // 50 cells hold both minimums: Skills' 32-cell list gives the detail its 24.
        let skills = areas(
            Rect::new(0, 0, 50, 10),
            &with_list(ListSize::Cells(32)),
            PaneId::List,
        );
        assert_eq!(skills.list.map(|r| r.width), Some(26));
        assert_eq!(skills.detail, Rect::new(26, 0, 24, 10));
    }

    #[test]
    fn a_region_below_the_minimums_keeps_the_unclamped_split_and_hides_nothing() {
        let tiny = Rect::new(0, 0, 30, 5);
        let got = areas(tiny, &with_list(ListSize::Cells(32)), PaneId::List);
        assert_eq!(got.list, Some(today(tiny, ListSize::Cells(32))[0]));
        assert_eq!(got.list_hidden, None);
        let half = areas(tiny, &TabLayout::DEFAULT, PaneId::List);
        assert_eq!(half.list, Some(Rect::new(0, 0, 15, 5)));
        assert_eq!(half.detail, Rect::new(15, 0, 15, 5));
    }

    #[test]
    fn the_activity_pane_is_clamped_to_its_minimum_and_leaves_the_others_theirs() {
        let small = TabLayout {
            preset: Preset::IdeActivity,
            activity_pct: 5,
            ..TabLayout::DEFAULT
        };
        let rows = areas(WIDE, &small, PaneId::List);
        assert_eq!(rows.activity.map(|r| r.height), Some(MIN_ACTIVITY_ROWS));
        assert_eq!(rows.detail.height, 30 - MIN_ACTIVITY_ROWS);
        let huge = TabLayout {
            activity_pct: 100,
            ..small
        };
        assert_eq!(
            areas(Rect::new(0, 0, 100, 10), &huge, PaneId::List)
                .activity
                .map(|r| r.height),
            Some(10 - MIN_ACTIVITY_ROWS)
        );
        let columns = TabLayout {
            preset: Preset::Columns,
            ..small
        };
        let cols = areas(WIDE, &columns, PaneId::List);
        assert_eq!(cols.activity.map(|r| r.width), Some(MIN_ACTIVITY_COLS));
        let crowded = areas(
            WIDE,
            &TabLayout {
                activity_pct: 90,
                ..columns
            },
            PaneId::List,
        );
        assert_eq!(
            crowded.activity.map(|r| r.width),
            Some(100 - MIN_LIST - MIN_DETAIL)
        );
        assert_eq!(crowded.list.map(|r| r.width), Some(MIN_LIST));
        assert_eq!(crowded.detail.width, MIN_DETAIL);
    }

    #[test]
    fn the_panes_tile_the_region_in_every_preset() {
        for &preset in Preset::ALL {
            for region in [WIDE, NARROW, Rect::new(3, 2, 41, 9)] {
                let got = areas(region, &layout(preset), PaneId::List);
                let cells: u32 = [got.list, Some(got.detail), got.activity]
                    .into_iter()
                    .flatten()
                    .map(|r| u32::from(r.width) * u32::from(r.height))
                    .sum();
                assert_eq!(
                    cells,
                    u32::from(region.width) * u32::from(region.height),
                    "{preset:?} in {region:?}"
                );
                for pane in [got.list, Some(got.detail), got.activity]
                    .into_iter()
                    .flatten()
                {
                    assert_eq!(pane.intersection(region), pane, "{preset:?} in {region:?}");
                }
            }
        }
    }

    #[test]
    fn visible_lists_the_shown_panes_in_cycle_order() {
        let all = areas(WIDE, &layout(Preset::Columns), PaneId::List);
        assert_eq!(
            visible(&all),
            [PaneId::List, PaneId::Detail, PaneId::Activity]
        );
        let ide = areas(WIDE, &TabLayout::DEFAULT, PaneId::List);
        assert_eq!(visible(&ide), [PaneId::List, PaneId::Detail]);
        let focus = areas(WIDE, &layout(Preset::Focus), PaneId::List);
        assert_eq!(visible(&focus), [PaneId::Detail]);
    }

    #[test]
    fn step_cycles_the_visible_panes_both_ways_and_wraps() {
        let all = areas(WIDE, &layout(Preset::Columns), PaneId::List);
        assert_eq!(step(&all, PaneId::List, true), PaneId::Detail);
        assert_eq!(step(&all, PaneId::Detail, true), PaneId::Activity);
        assert_eq!(step(&all, PaneId::Activity, true), PaneId::List);
        assert_eq!(step(&all, PaneId::List, false), PaneId::Activity);
        assert_eq!(step(&all, PaneId::Detail, false), PaneId::List);
        let ide = areas(WIDE, &TabLayout::DEFAULT, PaneId::List);
        assert_eq!(step(&ide, PaneId::Detail, true), PaneId::List);
        assert_eq!(step(&ide, PaneId::List, false), PaneId::Detail);
    }

    #[test]
    fn step_stays_put_alone_and_leaves_a_hidden_pane_from_its_place() {
        let focus = areas(WIDE, &layout(Preset::Focus), PaneId::List);
        assert_eq!(step(&focus, PaneId::Detail, true), PaneId::Detail);
        assert_eq!(step(&focus, PaneId::Detail, false), PaneId::Detail);
        assert_eq!(step(&focus, PaneId::List, true), PaneId::Detail);
        let ide = areas(WIDE, &TabLayout::DEFAULT, PaneId::List);
        assert_eq!(step(&ide, PaneId::Activity, true), PaneId::List);
        assert_eq!(step(&ide, PaneId::Activity, false), PaneId::Detail);
    }

    #[test]
    fn hit_names_the_pane_under_a_cell() {
        let all = areas(WIDE, &layout(Preset::IdeActivity), PaneId::List);
        assert_eq!(hit(&all, 0, 1), Some(PaneId::List));
        assert_eq!(hit(&all, 49, 21), Some(PaneId::List));
        assert_eq!(hit(&all, 50, 1), Some(PaneId::Detail));
        assert_eq!(hit(&all, 99, 22), Some(PaneId::Activity));
        assert_eq!(hit(&all, 0, 0), None, "above the region");
        assert_eq!(hit(&all, 100, 5), None, "right of the region");
        let zoomed = areas(
            WIDE,
            &TabLayout {
                zoom: true,
                ..TabLayout::DEFAULT
            },
            PaneId::List,
        );
        assert_eq!(hit(&zoomed, 0, 1), Some(PaneId::List));
        assert_eq!(hit(&zoomed, 99, 30), Some(PaneId::List));
    }

    #[test]
    fn a_seam_is_the_two_border_lines_where_two_panes_meet() {
        let ide = areas(WIDE, &TabLayout::DEFAULT, PaneId::List);
        assert_eq!(
            seam(&ide, 49, 5),
            Some(Seam::List),
            "the list's right border"
        );
        assert_eq!(
            seam(&ide, 50, 5),
            Some(Seam::List),
            "the detail's left border"
        );
        assert_eq!(seam(&ide, 48, 5), None);
        assert_eq!(seam(&ide, 51, 5), None);
        assert_eq!(seam(&ide, 49, 0), None, "outside the panes' rows");

        let stacked = areas(WIDE, &layout(Preset::IdeActivity), PaneId::List);
        assert_eq!(seam(&stacked, 10, 21), Some(Seam::Activity));
        assert_eq!(seam(&stacked, 10, 22), Some(Seam::Activity));
        assert_eq!(seam(&stacked, 10, 20), None);
        assert_eq!(seam(&stacked, 49, 10), Some(Seam::List));
        assert_eq!(
            seam(&stacked, 49, 25),
            None,
            "the list seam stops at the pair"
        );

        let columns = areas(WIDE, &layout(Preset::Columns), PaneId::List);
        assert_eq!(seam(&columns, 69, 3), Some(Seam::Activity));
        assert_eq!(seam(&columns, 70, 3), Some(Seam::Activity));
        assert_eq!(seam(&columns, 34, 3), Some(Seam::List));
    }

    #[test]
    fn a_hidden_pane_has_no_seam() {
        let focus = areas(WIDE, &layout(Preset::Focus), PaneId::List);
        for x in 0..100 {
            assert_eq!(seam(&focus, x, 5), None, "x = {x}");
        }
        let zoomed = areas(
            WIDE,
            &TabLayout {
                preset: Preset::Columns,
                zoom: true,
                ..TabLayout::DEFAULT
            },
            PaneId::Detail,
        );
        for x in 0..100 {
            assert_eq!(seam(&zoomed, x, 5), None, "x = {x}");
        }
    }

    fn labels() -> NoteLabels {
        NoteLabels {
            toggle_list: Some("F2".to_owned()),
            toggle_activity: Some("F3".to_owned()),
            next_preset: Some("p".to_owned()),
            zoom: Some("z".to_owned()),
        }
    }

    fn note(layout: &TabLayout, focus: PaneId) -> Option<String> {
        hidden_note(&areas(WIDE, layout, focus), &labels())
    }

    #[test]
    fn nothing_hidden_but_the_presets_activity_pane_leaves_no_note() {
        assert_eq!(note(&TabLayout::DEFAULT, PaneId::List), None);
        assert_eq!(note(&layout(Preset::IdeActivity), PaneId::List), None);
        assert_eq!(note(&layout(Preset::Columns), PaneId::List), None);
    }

    #[test]
    fn a_hidden_list_says_how_to_show_it_and_where_the_layouts_are() {
        let off = TabLayout {
            list_on: false,
            ..TabLayout::DEFAULT
        };
        assert_eq!(
            note(&off, PaneId::Detail).as_deref(),
            Some("F2 show list · p layouts")
        );
        assert_eq!(
            note(&layout(Preset::Focus), PaneId::Detail).as_deref(),
            Some("F2 show list · p layouts"),
            "the preset hides it the same way"
        );
    }

    #[test]
    fn a_zoom_says_how_to_unzoom_whichever_pane_is_zoomed() {
        let zoomed = TabLayout {
            zoom: true,
            ..TabLayout::DEFAULT
        };
        assert_eq!(note(&zoomed, PaneId::Detail).as_deref(), Some("z unzoom"));
        assert_eq!(note(&zoomed, PaneId::List).as_deref(), Some("z unzoom"));
        let activity = TabLayout {
            preset: Preset::IdeActivity,
            ..zoomed
        };
        assert_eq!(
            note(&activity, PaneId::Activity).as_deref(),
            Some("z unzoom")
        );
    }

    #[test]
    fn a_toggled_off_activity_pane_says_how_to_show_it() {
        let off = TabLayout {
            preset: Preset::Columns,
            activity_on: false,
            ..TabLayout::DEFAULT
        };
        assert_eq!(
            note(&off, PaneId::List).as_deref(),
            Some("F3 show activity")
        );
    }

    #[test]
    fn every_reason_joins_the_note_unzoom_first() {
        let all = TabLayout {
            preset: Preset::IdeActivity,
            list_on: false,
            activity_on: false,
            zoom: true,
            ..TabLayout::DEFAULT
        };
        assert_eq!(
            note(&all, PaneId::Detail).as_deref(),
            Some("F2 show list · p layouts · F3 show activity"),
            "nothing left to zoom away: no unzoom part"
        );
        let mixed = TabLayout {
            preset: Preset::IdeActivity,
            list_on: false,
            zoom: true,
            ..TabLayout::DEFAULT
        };
        assert_eq!(
            note(&mixed, PaneId::Detail).as_deref(),
            Some("z unzoom · F2 show list · p layouts")
        );
    }

    #[test]
    fn a_missing_label_drops_its_part_of_the_note() {
        let off = TabLayout {
            list_on: false,
            ..TabLayout::DEFAULT
        };
        let hidden = areas(WIDE, &off, PaneId::Detail);
        let no_toggle = NoteLabels {
            toggle_list: None,
            ..labels()
        };
        assert_eq!(
            hidden_note(&hidden, &no_toggle).as_deref(),
            Some("p layouts")
        );
        assert_eq!(hidden_note(&hidden, &NoteLabels::default()), None);
        let zoomed = areas(
            WIDE,
            &TabLayout {
                zoom: true,
                ..TabLayout::DEFAULT
            },
            PaneId::Detail,
        );
        assert_eq!(
            hidden_note(
                &zoomed,
                &NoteLabels {
                    zoom: None,
                    ..labels()
                }
            ),
            None
        );
    }

    #[test]
    fn the_presets_cycle_both_ways_and_wrap() {
        let mut layout = TabLayout::DEFAULT;
        let mut seen = Vec::new();
        for _ in 0..Preset::ALL.len() {
            layout.next_preset();
            seen.push(layout.preset);
        }
        assert_eq!(
            seen,
            [
                Preset::IdeActivity,
                Preset::Columns,
                Preset::Focus,
                Preset::Ide
            ]
        );
        layout.prev_preset();
        assert_eq!(layout.preset, Preset::Focus);
        layout.prev_preset();
        assert_eq!(layout.preset, Preset::Columns);
    }

    #[test]
    fn a_percent_list_steps_by_five_within_fifteen_and_eighty_five() {
        let mut layout = TabLayout::DEFAULT;
        layout.grow_list();
        assert_eq!(layout.list, ListSize::Percent(55));
        layout.shrink_list();
        layout.shrink_list();
        assert_eq!(layout.list, ListSize::Percent(45));
        for _ in 0..20 {
            layout.grow_list();
        }
        assert_eq!(layout.list, ListSize::Percent(85));
        for _ in 0..20 {
            layout.shrink_list();
        }
        assert_eq!(layout.list, ListSize::Percent(15));
    }

    #[test]
    fn a_cells_list_steps_by_four_and_never_below_one_cell() {
        let mut layout = with_list(ListSize::Cells(32));
        layout.grow_list();
        assert_eq!(layout.list, ListSize::Cells(36));
        layout.shrink_list();
        layout.shrink_list();
        assert_eq!(layout.list, ListSize::Cells(28));
        for _ in 0..20 {
            layout.shrink_list();
        }
        assert_eq!(layout.list, ListSize::Cells(1));
    }

    #[test]
    fn the_activity_pane_steps_by_five_within_fifteen_and_sixty() {
        let mut layout = TabLayout::DEFAULT;
        layout.grow_activity();
        assert_eq!(layout.activity_pct, 35);
        layout.shrink_activity();
        layout.shrink_activity();
        assert_eq!(layout.activity_pct, 25);
        for _ in 0..20 {
            layout.grow_activity();
        }
        assert_eq!(layout.activity_pct, 60);
        for _ in 0..20 {
            layout.shrink_activity();
        }
        assert_eq!(layout.activity_pct, 15);
    }

    #[test]
    fn toggling_the_list_hides_and_shows_it_and_leaves_focus_for_ide() {
        let mut layout = TabLayout::DEFAULT;
        layout.toggle_list();
        assert!(!layout.list_on);
        layout.toggle_list();
        assert!(layout.list_on);
        let mut focus = TabLayout {
            preset: Preset::Focus,
            ..TabLayout::DEFAULT
        };
        focus.toggle_list();
        assert_eq!((focus.preset, focus.list_on), (Preset::Ide, true));
        let mut both = TabLayout {
            preset: Preset::Focus,
            list_on: false,
            ..TabLayout::DEFAULT
        };
        both.toggle_list();
        assert_eq!((both.preset, both.list_on), (Preset::Ide, true));
    }

    #[test]
    fn toggling_the_activity_pane_on_moves_to_a_preset_that_has_one() {
        let mut stacked = layout(Preset::IdeActivity);
        stacked.toggle_activity();
        assert!(!stacked.activity_on);
        stacked.toggle_activity();
        assert_eq!(
            (stacked.preset, stacked.activity_on),
            (Preset::IdeActivity, true)
        );
        let mut columns = layout(Preset::Columns);
        columns.toggle_activity();
        columns.toggle_activity();
        assert_eq!(columns.preset, Preset::Columns, "it has one already");
        for preset in [Preset::Ide, Preset::Focus] {
            let mut without = layout(preset);
            without.toggle_activity();
            assert_eq!(
                (without.preset, without.activity_on),
                (Preset::IdeActivity, true),
                "from {preset:?}"
            );
        }
    }

    #[test]
    fn zoom_toggles() {
        let mut layout = TabLayout::DEFAULT;
        layout.toggle_zoom();
        assert!(layout.zoom);
        layout.toggle_zoom();
        assert!(!layout.zoom);
    }

    #[test]
    fn dragging_the_list_seam_keeps_the_size_kind() {
        let mut percent = TabLayout::DEFAULT;
        percent.set_list_at(WIDE, 59);
        assert_eq!(percent.list, ListSize::Percent(60));
        assert_eq!(
            areas(WIDE, &percent, PaneId::List).list.map(|r| r.right()),
            Some(60)
        );
        percent.set_list_at(WIDE, 2);
        assert_eq!(percent.list, ListSize::Percent(15));
        percent.set_list_at(WIDE, 99);
        assert_eq!(percent.list, ListSize::Percent(85));

        let mut cells = with_list(ListSize::Cells(32));
        cells.set_list_at(WIDE, 39);
        assert_eq!(cells.list, ListSize::Cells(40));
        cells.set_list_at(WIDE, 2);
        assert_eq!(cells.list, ListSize::Cells(MIN_LIST));
        cells.set_list_at(WIDE, 99);
        assert_eq!(cells.list, ListSize::Cells(100 - MIN_DETAIL));
    }

    #[test]
    fn dragging_the_list_seam_in_columns_measures_against_the_pair() {
        let mut columns = layout(Preset::Columns);
        columns.set_list_at(WIDE, 41);
        // The pair is 70 cells wide: a 42-cell list is 60% of it.
        assert_eq!(columns.list, ListSize::Percent(60));
    }

    #[test]
    fn dragging_a_hidden_lists_seam_changes_nothing() {
        let mut focus = layout(Preset::Focus);
        focus.set_list_at(WIDE, 30);
        assert_eq!(focus, layout(Preset::Focus));
        let mut zoomed = TabLayout {
            zoom: true,
            ..TabLayout::DEFAULT
        };
        let before = zoomed;
        zoomed.set_list_at(WIDE, 30);
        assert_eq!(zoomed, before);
    }

    #[test]
    fn dragging_the_activity_seam_sets_its_share_of_the_region() {
        let mut stacked = layout(Preset::IdeActivity);
        // The activity's top border on row 21: 10 of the 30 rows, 33%.
        stacked.set_activity_at(WIDE, 5, 21);
        assert_eq!(stacked.activity_pct, 33);
        stacked.set_activity_at(WIDE, 5, 30);
        assert_eq!(stacked.activity_pct, 15);
        stacked.set_activity_at(WIDE, 5, 1);
        assert_eq!(stacked.activity_pct, 60);

        let mut columns = layout(Preset::Columns);
        columns.set_activity_at(WIDE, 59, 5);
        assert_eq!(columns.activity_pct, 41);

        let mut ide = TabLayout::DEFAULT;
        ide.set_activity_at(WIDE, 5, 21);
        assert_eq!(ide, TabLayout::DEFAULT, "no activity pane to drag");
    }
}
