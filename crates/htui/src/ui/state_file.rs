//! The UI-state file (MOD-115 M2 D1, blueprint §5): `<config root>/ui-state.toml`, per machine,
//! versioned and written atomically the moment the about panel or a tab's layout changes. A
//! missing file is the first launch; an unreadable or invalid one falls back to the defaults with
//! a reason, and never stops htui starting. The testkit keeps it in memory.
//!
//! ```toml
//! version = 1
//!
//! [about]
//! visible = false
//!
//! [layout.backlog]            # the tab's id
//! preset = "ide"              # Preset::name
//! list_percent = 55           # exactly one of list_percent / list_cells
//! activity_percent = 30
//! list = true
//! activity = true
//! zoom = false
//!
//! [modal.item_form]           # reserved for R-TUI-12's modal sizes: read and written back
//! width = 80
//! height = 24
//! ```
//!
//! The serde shapes live here, so the shell's types ([`TabLayout`]) carry no serde. Unknown keys
//! are ignored, so a newer htui's additions survive an older one reading the file (they are not
//! written back).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ui::shell::{ListSize, Preset, TabLayout};

/// The file's name in the config root.
pub const FILE_NAME: &str = "ui-state.toml";

/// The format's version: the only one [`parse`] reads and the one [`render`] writes.
const VERSION: i64 = 1;

/// What the UI-state file holds (D1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiState {
    /// Whether the about panel is shown; `None` leaves it to the first-launch default (on).
    pub about: Option<bool>,
    /// Each tab's layout, keyed by the tab's id (`TabId.0`).
    pub layouts: BTreeMap<String, TabLayout>,
    /// Each modal's size as `(width, height)`, keyed by the modal's name (reserved for `R-TUI-12`:
    /// nothing reads it in M2, it is kept so a later htui's sizes survive).
    pub modals: BTreeMap<String, (u16, u16)>,
}

/// The file as TOML: the tables [`parse`] reads after the version and [`render`] writes.
#[derive(Debug, Serialize)]
struct FileDto {
    version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    about: Option<AboutDto>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    layout: BTreeMap<String, LayoutDto>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    modal: BTreeMap<String, ModalDto>,
}

/// `[about]`.
#[derive(Debug, Serialize, Deserialize)]
struct AboutDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visible: Option<bool>,
}

/// `[layout.<tab>]`.
#[derive(Debug, Serialize, Deserialize)]
struct LayoutDto {
    preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list_cells: Option<u16>,
    activity_percent: u16,
    list: bool,
    activity: bool,
    zoom: bool,
}

/// `[modal.<name>]`.
#[derive(Debug, Serialize, Deserialize)]
struct ModalDto {
    width: u16,
    height: u16,
}

/// Reads the file's text. `Err` is the reason it cannot be used, one line, for the notice.
///
/// # Errors
/// The text is not TOML, its `version` is missing or not 1, or a table does not hold what it
/// should (an unknown preset, both or neither list sizes, a percentage over 100, a wrong type).
pub fn parse(text: &str) -> Result<UiState, String> {
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
        let line = error
            .span()
            .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
        let message = one_line(error.message());
        match line {
            Some(line) => format!("not TOML (line {line}): {message}"),
            None => format!("not TOML: {message}"),
        }
    })?;
    match table.get("version") {
        Some(toml::Value::Integer(VERSION)) => {}
        Some(other) => {
            return Err(format!(
                "version: {other} is not one this htui reads (it writes version = {VERSION})"
            ));
        }
        None => {
            return Err(format!(
                "version: missing (this htui writes version = {VERSION})"
            ));
        }
    }
    let mut state = UiState::default();
    if let Some(about) = table.get("about") {
        let about: AboutDto = shaped("about", about)?;
        state.about = about.visible;
    }
    for (name, value) in tables(&table, "layout")? {
        let at = format!("layout.{name}");
        let layout: LayoutDto = shaped(&at, value)?;
        state.layouts.insert(
            name.clone(),
            layout.into_layout().map_err(|why| format!("{at}: {why}"))?,
        );
    }
    for (name, value) in tables(&table, "modal")? {
        let modal: ModalDto = shaped(&format!("modal.{name}"), value)?;
        state
            .modals
            .insert(name.clone(), (modal.width, modal.height));
    }
    Ok(state)
}

/// Writes `state` as the file's text, tables in key order, so the same state always writes the
/// same bytes.
#[must_use]
pub fn render(state: &UiState) -> String {
    let file = FileDto {
        version: VERSION,
        about: state.about.map(|visible| AboutDto {
            visible: Some(visible),
        }),
        layout: state
            .layouts
            .iter()
            .map(|(name, layout)| (name.clone(), LayoutDto::of(layout)))
            .collect(),
        modal: state
            .modals
            .iter()
            .map(|(name, &(width, height))| (name.clone(), ModalDto { width, height }))
            .collect(),
    };
    toml::to_string(&file).expect("plain tables of strings, integers and booleans serialise")
}

impl LayoutDto {
    fn of(layout: &TabLayout) -> Self {
        let (list_percent, list_cells) = match layout.list {
            ListSize::Percent(percent) => (Some(percent), None),
            ListSize::Cells(cells) => (None, Some(cells)),
        };
        Self {
            preset: layout.preset.name().to_owned(),
            list_percent,
            list_cells,
            activity_percent: layout.activity_pct,
            list: layout.list_on,
            activity: layout.activity_on,
            zoom: layout.zoom,
        }
    }

    fn into_layout(self) -> Result<TabLayout, String> {
        let preset = Preset::from_name(&self.preset).ok_or_else(|| {
            let names: Vec<&str> = Preset::ALL.iter().map(|preset| preset.name()).collect();
            format!(
                "unknown preset \"{}\" (one of {})",
                self.preset,
                names.join(", ")
            )
        })?;
        let list = match (self.list_percent, self.list_cells) {
            (Some(percent), None) => ListSize::Percent(within_100("list_percent", percent)?),
            (None, Some(cells)) => ListSize::Cells(cells),
            _ => return Err("write exactly one of list_percent and list_cells".to_owned()),
        };
        Ok(TabLayout {
            preset,
            list,
            activity_pct: within_100("activity_percent", self.activity_percent)?,
            list_on: self.list,
            activity_on: self.activity,
            zoom: self.zoom,
        })
    }
}

/// `percent` if it is a percentage, else why not.
fn within_100(key: &str, percent: u16) -> Result<u16, String> {
    if percent > 100 {
        Err(format!("{key} is {percent}, over 100"))
    } else {
        Ok(percent)
    }
}

/// The sub-tables of `[key]` (`[layout.<tab>]`, `[modal.<name>]`); none when it is absent.
fn tables<'t>(
    table: &'t toml::Table,
    key: &str,
) -> Result<impl Iterator<Item = (&'t String, &'t toml::Value)>, String> {
    match table.get(key) {
        None => Ok(None.into_iter().flatten()),
        Some(toml::Value::Table(inner)) => Ok(Some(inner.iter()).into_iter().flatten()),
        Some(_) => Err(format!("{key}: not a table")),
    }
}

/// `value` read as `T`, or why not, prefixed with where it is (`layout.backlog: …`).
fn shaped<T: serde::de::DeserializeOwned>(at: &str, value: &toml::Value) -> Result<T, String> {
    value
        .clone()
        .try_into()
        .map_err(|error: toml::de::Error| format!("{at}: {}", one_line(error.message())))
}

/// `message` with its line breaks folded, for a one-line notice.
fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{FILE_NAME, UiState, parse, render};
    use crate::ui::shell::{ListSize, Preset, TabLayout};

    /// A state with every table: the blueprint's example plus a Skills-shaped layout.
    fn full() -> UiState {
        let mut state = UiState {
            about: Some(false),
            ..UiState::default()
        };
        state.layouts.insert(
            "backlog".to_owned(),
            TabLayout {
                list: ListSize::Percent(55),
                ..TabLayout::DEFAULT
            },
        );
        state.layouts.insert(
            "skills".to_owned(),
            TabLayout {
                preset: Preset::Columns,
                list: ListSize::Cells(32),
                activity_pct: 25,
                list_on: false,
                activity_on: true,
                zoom: true,
            },
        );
        state.modals.insert("item_form".to_owned(), (80, 24));
        state
    }

    /// The blueprint's file, verbatim apart from comments.
    const BACKLOG_FILE: &str = "\
version = 1

[about]
visible = false

[layout.backlog]
preset = \"ide\"
list_percent = 55
activity_percent = 30
list = true
activity = true
zoom = false

[modal.item_form]
width = 80
height = 24
";

    fn reason(text: &str) -> String {
        parse(text).expect_err("the text is invalid")
    }

    #[test]
    fn the_file_is_named_ui_state_toml() {
        assert_eq!(FILE_NAME, "ui-state.toml");
    }

    #[test]
    fn a_state_round_trips_through_its_text() {
        let state = full();
        assert_eq!(parse(&render(&state)), Ok(state));
    }

    #[test]
    fn the_default_state_round_trips_and_writes_only_the_version() {
        assert_eq!(render(&UiState::default()), "version = 1\n");
        assert_eq!(parse("version = 1\n"), Ok(UiState::default()));
    }

    #[test]
    fn the_blueprint_file_reads_and_writes_back_byte_for_byte() {
        let state = parse(BACKLOG_FILE).expect("the blueprint's file reads");
        assert_eq!(state.about, Some(false));
        assert_eq!(
            state.layouts.get("backlog"),
            Some(&TabLayout {
                list: ListSize::Percent(55),
                ..TabLayout::DEFAULT
            })
        );
        assert_eq!(state.modals.get("item_form"), Some(&(80, 24)));
        assert_eq!(render(&state), BACKLOG_FILE);
    }

    #[test]
    fn the_same_state_always_renders_the_same_text_in_key_order() {
        let text = render(&full());
        assert_eq!(text, render(&full()));
        let backlog = text.find("[layout.backlog]").expect("backlog is written");
        let skills = text.find("[layout.skills]").expect("skills is written");
        assert!(backlog < skills, "{text}");
        let (backlog_table, skills_table) = text[backlog..].split_at(skills - backlog);
        assert!(backlog_table.contains("list_percent = 55"), "{text}");
        assert!(!backlog_table.contains("list_cells"), "{text}");
        assert!(skills_table.contains("list_cells = 32"), "{text}");
        assert!(!skills_table.contains("list_percent"), "{text}");
        assert!(skills_table.contains("preset = \"columns\""), "{text}");
    }

    #[test]
    fn every_preset_is_written_by_name_and_read_back() {
        for &preset in Preset::ALL {
            let mut state = UiState::default();
            state.layouts.insert(
                "requirements".to_owned(),
                TabLayout {
                    preset,
                    ..TabLayout::DEFAULT
                },
            );
            let text = render(&state);
            assert!(
                text.contains(&format!("preset = \"{}\"", preset.name())),
                "{text}"
            );
            assert_eq!(parse(&text), Ok(state));
        }
    }

    #[test]
    fn a_file_without_about_leaves_the_panel_to_the_first_launch_default() {
        assert_eq!(parse("version = 1\n").map(|s| s.about), Ok(None));
        assert_eq!(parse("version = 1\n[about]\n").map(|s| s.about), Ok(None));
        assert_eq!(
            parse("version = 1\n[about]\nvisible = true\n").map(|s| s.about),
            Ok(Some(true))
        );
    }

    #[test]
    fn unknown_keys_and_tables_are_ignored() {
        let text = "\
version = 1
theme = \"dark\"

[about]
visible = true
pinned = 3

[layout.backlog]
preset = \"focus\"
list_cells = 40
activity_percent = 20
list = false
activity = false
zoom = false
split = \"vertical\"

[history]
cap = 50
";
        let state = parse(text).expect("unknown keys are ignored");
        assert_eq!(state.about, Some(true));
        assert_eq!(
            state.layouts.get("backlog"),
            Some(&TabLayout {
                preset: Preset::Focus,
                list: ListSize::Cells(40),
                activity_pct: 20,
                list_on: false,
                activity_on: false,
                zoom: false,
            })
        );
        assert!(state.modals.is_empty());
    }

    #[test]
    fn text_that_is_not_toml_says_so_with_its_line() {
        let why = reason("version = 1\n[layout.backlog\n");
        assert!(why.starts_with("not TOML (line 2): "), "{why}");
        assert!(!why.contains('\n'), "one line for the notice: {why}");
    }

    #[test]
    fn a_missing_or_other_version_is_refused() {
        assert_eq!(
            reason(""),
            "version: missing (this htui writes version = 1)"
        );
        assert_eq!(
            reason("version = 2\n"),
            "version: 2 is not one this htui reads (it writes version = 1)"
        );
        assert_eq!(
            reason("version = \"1\"\n"),
            "version: \"1\" is not one this htui reads (it writes version = 1)"
        );
    }

    #[test]
    fn an_unknown_preset_is_refused_with_the_names_it_could_be() {
        let why = reason(
            "version = 1\n[layout.backlog]\npreset = \"wide\"\nlist_percent = 55\n\
             activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n",
        );
        assert_eq!(
            why,
            "layout.backlog: unknown preset \"wide\" (one of ide, ide_activity, columns, focus)"
        );
    }

    #[test]
    fn a_layout_needs_exactly_one_list_size() {
        let tail = "activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n";
        let both = format!(
            "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 55\nlist_cells = 32\n{tail}"
        );
        let neither = format!("version = 1\n[layout.backlog]\npreset = \"ide\"\n{tail}");
        let expected = "layout.backlog: write exactly one of list_percent and list_cells";
        assert_eq!(reason(&both), expected);
        assert_eq!(reason(&neither), expected);
    }

    #[test]
    fn a_percentage_over_100_is_refused() {
        let over_list = "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 101\n\
                         activity_percent = 30\nlist = true\nactivity = true\nzoom = false\n";
        let over_activity = "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 50\n\
                             activity_percent = 150\nlist = true\nactivity = true\nzoom = false\n";
        assert_eq!(
            reason(over_list),
            "layout.backlog: list_percent is 101, over 100"
        );
        assert_eq!(
            reason(over_activity),
            "layout.backlog: activity_percent is 150, over 100"
        );
    }

    #[test]
    fn a_table_of_the_wrong_shape_names_where() {
        let missing = reason(
            "version = 1\n[layout.backlog]\npreset = \"ide\"\nlist_percent = 50\n\
             activity_percent = 30\nlist = true\nactivity = true\n",
        );
        assert!(missing.starts_with("layout.backlog: "), "{missing}");
        assert!(missing.contains("zoom"), "{missing}");
        let negative = reason("version = 1\n[modal.item_form]\nwidth = -1\nheight = 24\n");
        assert!(negative.starts_with("modal.item_form: "), "{negative}");
        let about = reason("version = 1\n[about]\nvisible = \"yes\"\n");
        assert!(about.starts_with("about: "), "{about}");
        assert_eq!(reason("version = 1\nlayout = 3\n"), "layout: not a table");
        for why in [missing, negative, about] {
            assert!(!why.contains('\n'), "one line for the notice: {why}");
        }
    }
}
