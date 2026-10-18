//! The layout engine (MOD-115 M2 D2, blueprint §6.1): pure geometry from a tab's body region and
//! [`TabLayout`] to its [`PaneAreas`], the visible panes' focus order, mouse hits and seams.
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
}
