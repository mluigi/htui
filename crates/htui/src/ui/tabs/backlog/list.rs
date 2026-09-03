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

/// Two spaces between the columns of an item row, as in the blueprint's `KEY  status  title`.
const GAP: usize = 2;

/// How far an item row is indented under its project header.
const INDENT: usize = 2;

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

/// Draws the list pane.
pub fn render(frame: &mut Frame<'_>, area: Rect, view: &ListView<'_>, theme: &Theme) {
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" Backlog ({}) ", view.items.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = lines(view, theme, usize::from(inner.width));
    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("No items in this workspace.", theme.dim)),
            inner,
        );
        return;
    }

    let cursor = rows(view.items, view.projects, view.folded)
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
        .map(|item| item.key.chars().count())
        .max()
        .unwrap_or(0);
    let status_width = view
        .items
        .iter()
        .map(|item| item.status.as_str().len())
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
                    Span::styled(pad(&item.key, key_width), theme.accent),
                    Span::raw(" ".repeat(GAP)),
                    Span::styled(
                        pad(item.status.as_str(), status_width),
                        theme.status_style(item.status),
                    ),
                    Span::raw(" ".repeat(GAP)),
                    Span::styled(clip(&item.title, title_width), theme.base),
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
    let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let line = Line::from(spans);
    if selected {
        line.style(theme.selected)
    } else {
        line
    }
}

/// `text` padded with spaces to `width`.
fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    out.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(text.chars().count()),
    ));
    out
}

/// `text` cut to `width` characters, the last one replaced by an ellipsis when it did not fit.
pub fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
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
