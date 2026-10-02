//! The Backlog list pane: the scope's items, grouped by project.
//!
//! The row model is derived, never stored: [`rows`] rebuilds it from the items, the scope's
//! projects and the folded set every time it is needed, so a fold, a reply or a scope change can
//! never leave a stale index behind. The cursor is a [`Selection`] (an id), not an index, which
//! is what keeps it on the same row when a group above it folds.

use htui_core::model::{ItemId, ItemSummary, ProjectId, ProjectRef};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};

/// Two spaces between the columns of an item row, as in the blueprint's `KEY  status  title`.
const GAP: usize = 2;

/// How far an item row is indented under its project header.
const INDENT: usize = 2;

/// The empty list without a filter.
pub const NO_ITEMS: &str = "No items in this workspace.";

/// The empty list under a filter (MOD-13 D5).
pub const NO_MATCH: &str = "No items match the filter.";

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

/// The list pane's title: the count, then the active filter's summary when there is one
/// (MOD-13 D5). Without a filter it is byte-for-byte the pre-MOD-13 title.
#[must_use]
pub fn title(count: usize, filter: Option<&str>) -> String {
    match filter {
        None => format!(" Backlog ({count}) "),
        Some(summary) => format!(" Backlog ({count}) · {summary} "),
    }
}

/// Draws the list pane.
pub fn render(frame: &mut Frame<'_>, area: Rect, view: &ListView<'_>, theme: &Theme) {
    let block = Block::new()
        .borders(Borders::ALL)
        .title(title(view.items.len(), view.filter));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
        let empty = if view.filter.is_some() {
            NO_MATCH
        } else {
            NO_ITEMS
        };
        frame.render_widget(Paragraph::new(Line::styled(empty, theme.dim)), inner);
        return;
    }

    let mut lines = lines(view, theme, usize::from(inner.width));
    let cursor = visible
        .iter()
        .position(|row| Some(*row) == view.selected)
        .unwrap_or(0);
    let offset = window(cursor, lines.len(), usize::from(inner.height));
    frame.render_widget(
        Paragraph::new(lines.split_off(offset.min(lines.len()))),
        inner,
    );
}

/// One [`Line`] per visible row.
fn lines(view: &ListView<'_>, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let key_width = view
        .items
        .iter()
        .map(|item| cell_width(&item.key))
        .max()
        .unwrap_or(0);
    let status_width = view
        .items
        .iter()
        .map(|item| cell_width(item.status.as_str()))
        .max()
        .unwrap_or(0);
    let title_width = width
        .saturating_sub(INDENT + key_width + GAP + status_width + GAP)
        .max(1);

    let mut out = Vec::new();
    for (project, group) in groups(view.items, view.projects) {
        let folded = view.folded.contains(&project.project_id);
        let marker = if folded { '\u{25b8}' } else { '\u{25be}' };
        let selected = view.selected == Some(Selection::Project(project.project_id));
        out.push(row(
            vec![Span::styled(
                format!("{marker} {} ({})", project.name, group.len()),
                theme.title,
            )],
            width,
            selected,
            theme,
        ));
        if folded {
            continue;
        }
        for item in group {
            let selected = view.selected == Some(Selection::Item(item.id));
            out.push(row(
                vec![
                    Span::raw(" ".repeat(INDENT)),
                    Span::styled(cells::pad(&item.key, key_width), theme.accent),
                    Span::raw(" ".repeat(GAP)),
                    Span::styled(
                        cells::pad(item.status.as_str(), status_width),
                        theme.status_style(item.status),
                    ),
                    Span::raw(" ".repeat(GAP)),
                    Span::styled(cells::clip(&item.title, title_width), theme.base),
                ],
                width,
                selected,
                theme,
            ));
        }
    }
    out
}

/// Pads the row out to the pane width so the selected style covers the whole line.
fn row(
    mut spans: Vec<Span<'static>>,
    width: usize,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let used: usize = spans.iter().map(|span| cell_width(&span.content)).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let line = Line::from(spans);
    if selected {
        line.style(theme.selected)
    } else {
        line
    }
}

/// First visible row: enough to keep `cursor` inside a `height`-tall viewport.
///
/// Stateless on purpose — it is a function of the cursor, so no scroll offset can survive a fold
/// or a reply that shortens the list.
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
    use super::*;
    use htui_core::fixtures::ids;
    use htui_core::model::Status;

    /// MOD-13 D5: no filter, the title every existing snapshot pins.
    #[test]
    fn the_title_without_a_filter_is_unchanged() {
        assert_eq!(title(11, None), " Backlog (11) ");
    }

    /// MOD-13 D5: an active filter appends its summary after the count.
    #[test]
    fn the_title_with_a_filter_appends_its_summary() {
        assert_eq!(title(2, Some("status:done")), " Backlog (2) · status:done ");
    }

    fn summary(key: &str, number: i32, title: &str, status: Status) -> ItemSummary {
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
        }
    }

    /// MOD-60 D1: the key column is padded and the title clipped in cells, so a CJK key or
    /// title neither pushes the status column right nor runs the row past the pane.
    #[test]
    fn a_wide_key_and_title_keep_every_row_the_pane_width() {
        let items = vec![
            summary("FEAT-12", 12, "short", Status::Open),
            summary(
                "\u{6f22}\u{5b57}-1",
                1,
                &"\u{6587}".repeat(40),
                Status::InProgress,
            ),
        ];
        let projects = vec![ProjectRef {
            project_id: ids::PROJECT_HTUI,
            slug: "htui".to_owned(),
            name: "htui".to_owned(),
            position: 0,
        }];
        let view = ListView {
            items: &items,
            projects: &projects,
            folded: &[],
            selected: None,
            filter: None,
        };
        let lines = lines(&view, &Theme::default(), 50);
        assert_eq!(lines.len(), 3, "one header and two items");
        for line in &lines {
            let used: usize = line
                .spans
                .iter()
                .map(|span| cell_width(&span.content))
                .sum();
            assert_eq!(used, 50, "{line:?}");
        }
        let status_at: Vec<usize> = lines[1..]
            .iter()
            .map(|line| {
                line.spans[..3]
                    .iter()
                    .map(|span| cell_width(&span.content))
                    .sum()
            })
            .collect();
        assert_eq!(status_at[0], status_at[1], "{lines:?}");
    }
}
