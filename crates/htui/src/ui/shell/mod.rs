//! The shell's pane model (MOD-115 M2 D2, D3, D5; blueprint §1.8): the types the layout engine
//! ([`layout`]), the history ([`history`]), the UI-state file and the shell share, so those never
//! import each other.
//!
//! A tab with a list and a detail declares [`Panes::ListDetail`] with its default [`TabLayout`];
//! the shell turns the tab's body region and its layout into [`PaneAreas`], keeps one focused
//! [`PaneId`] per tab and records a [`TabLoc`] per history entry.

pub mod history;
pub mod layout;

use ratatui::layout::Rect;

/// One pane of a list + detail tab (D3), in focus-cycle order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneId {
    /// The list on the left.
    List,
    /// The detail: the tab's main pane, and a single-pane tab's only one.
    Detail,
    /// The activity pane (MOD-122 fills it; a placeholder until then).
    Activity,
}

/// A layout preset (D2): which panes a list + detail tab shows, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    /// The list beside the detail.
    Ide,
    /// The list beside the detail, the activity pane with them.
    IdeActivity,
    /// The panes as columns.
    Columns,
    /// The detail alone.
    Focus,
}

impl Preset {
    /// Every preset, in `layout.next_preset` order.
    pub const ALL: &'static [Self] = &[Self::Ide, Self::IdeActivity, Self::Columns, Self::Focus];

    /// The preset's name in the UI-state file: `ide`, `ide_activity`, `columns`, `focus`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ide => "ide",
            Self::IdeActivity => "ide_activity",
            Self::Columns => "columns",
            Self::Focus => "focus",
        }
    }

    /// The preset [`name`](Self::name) spells, or `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|preset| preset.name() == name)
    }

    /// The preset as the header names it (`R-TUI-1`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ide => "list+detail",
            Self::IdeActivity => "list+detail+activity",
            Self::Columns => "columns",
            Self::Focus => "detail only",
        }
    }
}

/// The list's width (blueprint D-3): a share of the body, or a fixed number of cells (Skills'
/// 32-column list, which no percentage reproduces at every width).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListSize {
    /// A percentage of the body's width, 0..=100.
    Percent(u16),
    /// A width in cells.
    Cells(u16),
}

/// One tab's layout (D2): the preset, the split sizes, what is shown, and the zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabLayout {
    /// The preset.
    pub preset: Preset,
    /// The list's width.
    pub list: ListSize,
    /// The activity pane's share of the body, in percent.
    pub activity_pct: u16,
    /// Whether the list is shown (`layout.toggle_list`).
    pub list_on: bool,
    /// Whether the activity pane is shown (`layout.toggle_activity`).
    pub activity_on: bool,
    /// Whether the focused pane fills the body (`layout.zoom`).
    pub zoom: bool,
}

impl TabLayout {
    /// The layout a tab that declares no other gets: [`Preset::Ide`], an even split, the
    /// activity pane at 30%, both shown, no zoom.
    pub const DEFAULT: Self = Self {
        preset: Preset::Ide,
        list: ListSize::Percent(50),
        activity_pct: 30,
        list_on: true,
        activity_on: true,
        zoom: false,
    };
}

impl Default for TabLayout {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The panes a tab has (D2): one, or a list and a detail with the tab's default layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Panes {
    /// One pane, the detail: Chat and Settings in M2. The pane keys do nothing there.
    #[default]
    Single,
    /// A list and a detail (Backlog, Requirements, the two Skills views), with the tab's
    /// default layout.
    ListDetail(TabLayout),
}

/// Why a pane is not drawn (`R-TUI-12`): the pane says how to bring it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HiddenBy {
    /// `layout.toggle_list` / `layout.toggle_activity` hid it.
    Toggle,
    /// The preset has no such pane.
    Preset,
    /// Another pane is zoomed.
    Zoom,
}

/// Where each pane of a tab is drawn this frame (D2), and why a missing one is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneAreas {
    /// The list, if shown.
    pub list: Option<Rect>,
    /// The detail, always shown.
    pub detail: Rect,
    /// The activity pane, if shown.
    pub activity: Option<Rect>,
    /// Why the list is hidden, if it is.
    pub list_hidden: Option<HiddenBy>,
    /// Why the activity pane is hidden, if it is.
    pub activity_hidden: Option<HiddenBy>,
}

/// A tab's own part of a history entry (D5): what `Tab::location` gives and `Tab::restore`
/// takes back. The shell only stores and compares it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TabLoc {
    /// The sub-tab's stable id, if the tab has sub-tabs.
    pub subtab: Option<&'static str>,
    /// The sub-tab's title, for the header's `back to` chip.
    pub subtab_title: Option<String>,
    /// The opaque cursor: a string the tab writes (an item id, a requirement key, a skill id, a
    /// row index) and parses back in `restore`.
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ListSize, PaneAreas, Panes, Preset, TabLayout, TabLoc};
    use ratatui::layout::Rect;

    #[test]
    fn every_preset_round_trips_its_name() {
        for &preset in Preset::ALL {
            assert_eq!(Preset::from_name(preset.name()), Some(preset));
        }
        assert_eq!(Preset::from_name("IDE"), None);
        assert_eq!(Preset::from_name(""), None);
    }

    #[test]
    fn the_presets_are_named_and_labelled_as_the_blueprint_says() {
        let named: Vec<(&str, &str)> = Preset::ALL
            .iter()
            .map(|preset| (preset.name(), preset.label()))
            .collect();
        assert_eq!(
            named,
            [
                ("ide", "list+detail"),
                ("ide_activity", "list+detail+activity"),
                ("columns", "columns"),
                ("focus", "detail only"),
            ]
        );
    }

    #[test]
    fn the_default_layout_is_an_even_ide_split_with_everything_shown() {
        let layout = TabLayout::default();
        assert_eq!(layout, TabLayout::DEFAULT);
        assert_eq!(layout.preset, Preset::Ide);
        assert_eq!(layout.list, ListSize::Percent(50));
        assert_eq!(layout.activity_pct, 30);
        assert!(layout.list_on && layout.activity_on && !layout.zoom);
    }

    #[test]
    fn a_tab_has_one_pane_unless_it_declares_a_list() {
        assert_eq!(Panes::default(), Panes::Single);
        let skills = TabLayout {
            list: ListSize::Cells(32),
            ..TabLayout::DEFAULT
        };
        assert_ne!(
            Panes::ListDetail(skills),
            Panes::ListDetail(TabLayout::DEFAULT)
        );
    }

    #[test]
    fn a_location_and_the_areas_are_plain_values() {
        let here = TabLoc {
            subtab: Some("runs"),
            subtab_title: Some("Runs".to_owned()),
            cursor: Some("MOD-115".to_owned()),
        };
        assert_eq!(here.clone(), here);
        assert_ne!(here, TabLoc::default());
        let body = Rect::new(0, 0, 100, 30);
        let areas = PaneAreas {
            list: None,
            detail: body,
            activity: None,
            list_hidden: None,
            activity_hidden: None,
        };
        assert_eq!(areas.detail, body);
    }
}
