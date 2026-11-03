//! The Backlog list pane: the scope's items, grouped by project.
//!
//! The row model is derived, never stored: [`rows`] rebuilds it from the items, the scope's
//! projects and the folded set every time it is needed, so a fold, a reply or a scope change can
//! never leave a stale index behind. The cursor is a [`Selection`] (an id), not an index, which
//! is what keeps it on the same row when a group above it folds.
//!
//! MOD-115 M4 E4: the pane is a kit [`Pane`], its rows `kit::list`'s (group rows, the gutter and
//! the band, the `…` cut), each status `kit::status`'s glyph and word, the active filter the
//! summary band (`kit::filter`), an empty list `kit::empty`'s with a next step.

use htui_core::model::{ItemId, ItemState, ItemSummary, ProjectId, ProjectRef};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::filter::{self as band, Filter};
use crate::ui::kit::label::plural;
use crate::ui::kit::list::{self as kit_list, Row, RowKind, group_row};
use crate::ui::kit::pane::Pane;
use crate::ui::kit::scroll::window;
use crate::ui::kit::status;
use crate::ui::tabs::backlog::Chrome;

/// Two spaces between the columns of an item row, as in the blueprint's `KEY  status  title`.
/// The status column is the item's [`ItemState`](htui_core::model::ItemState) mark (MOD-144).
const GAP: usize = 2;

/// The empty list without a filter.
pub const NO_ITEMS: &str = "No items in this workspace.";

/// The empty list under a filter (MOD-13 D5).
pub const NO_MATCH: &str = "No items match the filter.";

/// [`NO_ITEMS`]' next step with no project in the workspace: where a project is added.
pub const NO_PROJECT_NEXT: &str = "Settings \u{203a} Hierarchy adds a project";

/// [`NO_ITEMS`]' next step while `backlog.new_item` is unbound.
const NEW_UNBOUND: &str = "a new item lists here";

/// [`NO_MATCH`]'s next step while `backlog.clear_filter` is unbound.
const CLEAR_UNBOUND: &str = "clearing the filter lists every item";

/// Which row of the list the cursor is on.
///
/// Selecting a project header is a real state, not a placeholder: it is what `Enter` folds, and
/// the detail pane shows its empty state while it is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// A project header row.
    Project(ProjectId),
    /// An item row.
    Item(ItemId),
}

/// A project header and the items under it, in list order.
pub type Group<'a> = (&'a ProjectRef, Vec<&'a ItemSummary>);

/// What [`render`] needs, borrowed from the tab so the list module owns no state of its own.
#[derive(Debug)]
pub struct ListView<'a> {
    /// The scope's items, as the `Items` reply delivered them.
    pub items: &'a [ItemSummary],
    /// The scope's projects, in `workspace_project.position` order.
    pub projects: &'a [ProjectRef],
    /// Projects whose group is currently folded.
    pub folded: &'a [ProjectId],
    /// The cursor.
    pub selected: Option<Selection>,
    /// The active filter's summary; `None` = no filter (MOD-13 D5).
    pub filter: Option<&'a str>,
    /// `backlog.new_item`'s label for the empty list's next step; `None` while unbound.
    pub new_key: Option<&'a str>,
    /// `backlog.clear_filter`'s label for the no-match next step; `None` while unbound.
    pub clear_key: Option<&'a str>,
}

/// Groups the items by project.
///
/// Project order is `ctx.projects`, i.e. `workspace_project.position`; inside a group the rows are
/// ordered by `key_prefix` then `key_number`, so `ANA-2` follows `ANA-1` rather than `FEAT-2`
/// (several key prefixes share one project, so `key_number` alone does not order them).
#[must_use]
pub fn groups<'a>(items: &'a [ItemSummary], projects: &'a [ProjectRef]) -> Vec<Group<'a>> {
    projects
        .iter()
        .map(|project| {
            let mut rows: Vec<&ItemSummary> = items
                .iter()
                .filter(|item| item.project_id == project.project_id)
                .collect();
            rows.sort_by(|left, right| {
                left.key_prefix
                    .cmp(&right.key_prefix)
                    .then_with(|| left.key_number.cmp(&right.key_number))
            });
            (project, rows)
        })
        .collect()
}

/// The visible rows, top to bottom: every project header, each followed by its items unless the
/// group is folded.
#[must_use]
pub fn rows(
    items: &[ItemSummary],
    projects: &[ProjectRef],
    folded: &[ProjectId],
) -> Vec<Selection> {
    let mut out = Vec::new();
    for (project, group) in groups(items, projects) {
        out.push(Selection::Project(project.project_id));
        if !folded.contains(&project.project_id) {
            out.extend(group.iter().map(|item| Selection::Item(item.id)));
        }
    }
    out
}

/// The rows the list draws **and** navigates (MOD-13 review M2): [`rows`], except that a filter
/// nothing matches has none. Its project headers would still be rows, so the cursor would sit on,
/// fold and step over headers the pane does not draw.
#[must_use]
pub fn visible(
    items: &[ItemSummary],
    projects: &[ProjectRef],
    folded: &[ProjectId],
    filtered: bool,
) -> Vec<Selection> {
    if filtered && items.is_empty() {
        return Vec::new();
    }
    rows(items, projects, folded)
}

/// The list pane's title: the count of the rows read (MOD-115 M4 E4). The active filter is the
/// summary band under it, not part of the title.
#[must_use]
pub fn title(count: usize) -> String {
    format!("Backlog ({count})")
}

/// Draws the list pane as a kit pane (MOD-115 M4 E4): titled with the count, the summary band
/// as its first row while a filter is applied (D-13), then the rows through `kit::list`, the
/// cursor row behind the gutter and in the band, scrolled with the pane's scrollbar. An empty
/// list is `kit::empty`'s, with its next step.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    view: &ListView<'_>,
    chrome: Chrome<'_>,
    theme: &Theme,
) {
    let title = title(view.items.len());
    let accent = chrome.accent;
    let pane = chrome.frame(Pane::new(&title));
    let inner = Pane::inner(area);
    // D-13: the list is read already filtered, so the band carries the count in words.
    let band_rows = u16::from(view.filter.is_some()).min(inner.height);
    if let Some(query) = view.filter {
        let count = plural(view.items.len(), "item", "items");
        band::render(
            frame,
            inner,
            &Filter::Summary {
                query,
                count: &count,
            },
            accent,
            theme,
        );
    }
    let body = Rect {
        y: inner.y + band_rows,
        height: inner.height - band_rows,
        ..inner
    };

    // MOD-13 blueprint E3, review M2: every scope project has a header row even with no item
    // under it, so an empty filtered result is told by `visible`, the rule the tab navigates by.
    // Unfiltered, it is the pre-MOD-13 check: no project, no row.
    let visible = visible(
        view.items,
        view.projects,
        view.folded,
        view.filter.is_some(),
    );
    if visible.is_empty() {
        let (what, next) = empty_state(view);
        empty::render(frame, body, what, NextStep(&next), theme);
        pane.render(frame, area, theme);
        return;
    }

    let rows = list_rows(view, theme);
    let cursor = visible.iter().position(|row| Some(*row) == view.selected);
    let offset = window(cursor.unwrap_or(0), rows.len(), usize::from(body.height));
    kit_list::render(frame, body, &rows, cursor, offset, accent, theme);
    pane.scrollbar(rows.len(), offset)
        .scrollbar_span(body)
        .render(frame, area, theme);
}

/// What an empty list says and its next step: [`NO_MATCH`] and the key that clears the filter
/// under a filter; [`NO_ITEMS`] and [`NO_PROJECT_NEXT`] with no project to list; [`NO_ITEMS`] and
/// the key that adds an item otherwise. A key left unbound gets a sentence without it.
fn empty_state(view: &ListView<'_>) -> (&'static str, String) {
    if view.filter.is_some() {
        let next = view.clear_key.map_or_else(
            || CLEAR_UNBOUND.to_owned(),
            |key| format!("{key} clears the filter"),
        );
        (NO_MATCH, next)
    } else if view.projects.is_empty() {
        (NO_ITEMS, NO_PROJECT_NEXT.to_owned())
    } else {
        let next = view
            .new_key
            .map_or_else(|| NEW_UNBOUND.to_owned(), |key| format!("{key} adds one"));
        (NO_ITEMS, next)
    }
}

/// The cells `state`'s mark takes: its glyph, a blank and its word.
fn mark_width(state: ItemState) -> usize {
    let mark = status::mark(state);
    cell_width(mark.glyph) + 1 + cell_width(mark.word)
}

/// One `kit::list` row per visible row: a project is the kit's group row, an item
/// ` {key}  {status mark}  {title}` with the key and the mark padded to their columns, so every
/// title starts at one column. The status column is as wide as the widest mark shown (MOD-81:
/// `⚠ needs you`, 11 cells, where `awaiting_approval` took 17). `kit::list` cuts the row with `…`.
fn list_rows(view: &ListView<'_>, theme: &Theme) -> Vec<Row<'static>> {
    let key_width = view
        .items
        .iter()
        .map(|item| cell_width(&item.key))
        .max()
        .unwrap_or(0);
    let status_width = view
        .items
        .iter()
        .map(|item| mark_width(item.state()))
        .max()
        .unwrap_or(0);

    let mut out = Vec::new();
    for (project, group) in groups(view.items, view.projects) {
        let folded = view.folded.contains(&project.project_id);
        out.push(group_row(&project.name, group.len(), folded, theme));
        if folded {
            continue;
        }
        for item in group {
            let mut spans = vec![
                Span::raw(" "),
                Span::styled(cells::pad(&item.key, key_width), theme.key),
                Span::raw(" ".repeat(GAP)),
            ];
            spans.extend(status::spans(item.state(), theme));
            let pad = status_width.saturating_sub(mark_width(item.state())) + GAP;
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(item.title.clone(), theme.base));
            out.push(Row {
                line: Line::from(spans),
                kind: RowKind::Item,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::kit::pane::Focus;
    use htui_core::fixtures::ids;
    use htui_core::model::{RunStatus, Status};
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier, Style};

    /// MOD-115 M4 E4: the title is the count alone; the filter is the summary band.
    #[test]
    fn the_title_is_the_count_alone() {
        assert_eq!(title(11), "Backlog (11)");
    }

    fn summary(
        key: &str,
        number: i32,
        title: &str,
        status: Status,
        latest: Option<RunStatus>,
    ) -> ItemSummary {
        ItemSummary {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            key: key.to_owned(),
            key_prefix: key.split('-').next().unwrap_or_default().to_owned(),
            key_number: number,
            title: title.to_owned(),
            status,
            priority: 0,
            required_tags: Vec::new(),
            updated_at: htui_core::fixtures::demo_at(0, 0),
            touched_paths: Vec::new(),
            latest_run_status: latest,
        }
    }

    /// The `htui` project alone, as `ctx.projects` holds it.
    fn htui() -> Vec<ProjectRef> {
        vec![ProjectRef {
            project_id: ids::PROJECT_HTUI,
            slug: "htui".to_owned(),
            name: "htui".to_owned(),
            position: 0,
        }]
    }

    /// The `htui` project with `FEAT-12` (running) and `FEAT-13` (open).
    fn two_items() -> (Vec<ItemSummary>, Vec<ProjectRef>) {
        let items = vec![
            summary(
                "FEAT-12",
                12,
                "short",
                Status::Open,
                Some(RunStatus::Running),
            ),
            summary("FEAT-13", 13, "other", Status::Open, None),
        ];
        (items, htui())
    }

    /// A view over `items` and `projects` with `selected` under the cursor and no filter.
    fn view<'a>(
        items: &'a [ItemSummary],
        projects: &'a [ProjectRef],
        selected: Option<Selection>,
    ) -> ListView<'a> {
        ListView {
            items,
            projects,
            folded: &[],
            selected,
            filter: None,
            new_key: Some("N"),
            clear_key: Some("F"),
        }
    }

    /// The list pane drawn unfocused into a `width` x `height` buffer.
    fn drawn(view: &ListView<'_>, theme: &Theme, width: u16, height: u16) -> Buffer {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test terminal");
        let chrome = Chrome {
            focus: Focus::Unfocused,
            accent: Style::new().fg(Color::Yellow),
            hints: None,
        };
        terminal
            .draw(|frame| render(frame, frame.area(), view, chrome, theme))
            .expect("the frame draws");
        terminal.backend().buffer().clone()
    }

    /// Row `y` of `buffer`, as text.
    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    /// MOD-60 D1: the key column is padded and the title cut in cells, so a CJK key or title
    /// neither pushes the status column right nor runs the row past the pane.
    #[test]
    fn a_wide_key_and_title_keep_the_columns_and_the_pane() {
        let items = vec![
            summary("FEAT-12", 12, "short", Status::Open, None),
            summary(
                "\u{6f22}\u{5b57}-1",
                1,
                &"\u{6587}".repeat(40),
                Status::Open,
                Some(RunStatus::Running),
            ),
        ];
        let projects = htui();
        let buffer = drawn(&view(&items, &projects, None), &Theme::default(), 50, 6);
        // The border, the gutter and a blank, the 7-cell key column and the gap.
        let status_x = 1 + 1 + 1 + 7 + 2;
        let glyphs: Vec<&str> = (2..4).map(|y| buffer[(status_x, y)].symbol()).collect();
        assert_eq!(glyphs, ["\u{25cb}", "\u{25d0}"], "{buffer:?}");
        let wide = row(&buffer, 3);
        assert!(wide.contains('\u{2026}'), "cut before the border: {wide:?}");
    }

    /// MOD-80 D2, MOD-115 M4 E4: the selected row is the gutter in the accent, then one even
    /// band across the content, the key's and the status's colours included; the key stays bold.
    #[test]
    fn a_selected_item_row_is_the_gutter_then_one_even_band() {
        let (items, projects) = two_items();
        let theme = Theme::default();
        let buffer = drawn(
            &view(&items, &projects, Some(Selection::Item(items[0].id))),
            &theme,
            50,
            5,
        );
        assert_eq!(buffer[(1, 2)].symbol(), kit_list::GUTTER);
        assert_eq!(buffer[(1, 2)].fg, Color::Yellow, "the accent");
        for x in 2..49 {
            let cell = &buffer[(x, 2)];
            assert_eq!(cell.fg, Color::Black, "x = {x}: {cell:?}");
            assert_eq!(cell.bg, Color::Cyan, "x = {x}: {cell:?}");
        }
        assert_eq!(
            buffer[(2, 3)].bg,
            Color::Reset,
            "the next row is not selected"
        );
        for x in 3..3 + 7 {
            assert!(
                buffer[(x, 2)].modifier.contains(Modifier::BOLD),
                "the key stays bold under the band, x = {x}"
            );
        }
    }

    /// MOD-80 review L2, D4: under the monochrome theme the band is REVERSED in every content
    /// cell of the selected row and in no cell of the next.
    #[test]
    fn a_selected_item_row_is_reversed_without_colour() {
        let (items, projects) = two_items();
        let buffer = drawn(
            &view(&items, &projects, Some(Selection::Item(items[0].id))),
            &Theme::monochrome(),
            50,
            5,
        );
        for x in 2..49 {
            assert!(
                buffer[(x, 2)].modifier.contains(Modifier::REVERSED),
                "x = {x}: {:?}",
                buffer[(x, 2)]
            );
            assert!(
                !buffer[(x, 3)].modifier.contains(Modifier::REVERSED),
                "x = {x}: {:?}",
                buffer[(x, 3)]
            );
        }
    }

    /// MOD-80 D1, MOD-115 M4 E4: a key is bold in the terminal's own colour, and a running item
    /// (MOD-144: open over a running latest graph run) is `kit::status`'s `◐ running`, glyph
    /// and word in the running colour.
    #[test]
    fn an_item_key_is_bold_and_a_running_item_is_running() {
        let (items, projects) = two_items();
        let theme = Theme::default();
        let buffer = drawn(&view(&items, &projects, None), &theme, 50, 5);
        let key = &buffer[(3, 2)];
        assert_eq!(key.symbol(), "F");
        assert!(key.modifier.contains(Modifier::BOLD), "{key:?}");
        assert_eq!(key.fg, Color::Reset, "a key is not cyan: {key:?}");
        assert!(
            row(&buffer, 2).contains("\u{25d0} running"),
            "{}",
            row(&buffer, 2)
        );
        let glyph = &buffer[(1 + 1 + 1 + 7 + 2, 2)];
        assert_eq!(glyph.symbol(), "\u{25d0}");
        assert_eq!(glyph.fg, Color::Blue, "{glyph:?}");
    }

    /// MOD-115 M4 E4: an empty list is `kit::empty`'s: what is missing and the next step, the key
    /// named through the keys in force, a sentence without it when it is unbound.
    #[test]
    fn an_empty_list_names_its_next_step() {
        let theme = Theme::default();
        let text = |view: &ListView<'_>| {
            let buffer = drawn(view, &theme, 60, 8);
            (0..8)
                .map(|y| row(&buffer, y))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let projects = htui();
        let none = view(&[], &[], None);
        let frame = text(&none);
        assert!(frame.contains(NO_ITEMS), "{frame}");
        assert!(frame.contains(NO_PROJECT_NEXT), "{frame}");

        let filtered = ListView {
            filter: Some("status: done"),
            ..view(&[], &projects, None)
        };
        let frame = text(&filtered);
        assert!(frame.contains(NO_MATCH), "{frame}");
        assert!(frame.contains("F clears the filter"), "{frame}");
        assert!(frame.contains(" / status: done"), "the band stays: {frame}");
        assert!(frame.contains("0 items"), "{frame}");
        let unbound = ListView {
            clear_key: None,
            ..filtered
        };
        assert!(text(&unbound).contains(CLEAR_UNBOUND));

        let (what, next) = empty_state(&view(&[], &projects, None));
        assert_eq!((what, next.as_str()), (NO_ITEMS, "N adds one"));
        let unbound = ListView {
            new_key: None,
            ..view(&[], &projects, None)
        };
        assert_eq!(empty_state(&unbound).1, NEW_UNBOUND);
    }
}
