//! The Workspaces tab's detail lines (MOD-116 M1 D8): a workspace's overview, or one of its
//! projects', built from one [`WorkspaceOverview`] reply. Pure: lines in, no frame, so the tab's
//! render only scrolls and draws them.

use htui_core::model::WorkspaceLifecycle;
use ratatui::text::{Line, Span};

use crate::hierarchy::ProjectEntry;
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::label::{self, plural};
use crate::ui::kit::text::wrapped;
use crate::workspaces::{ProjectOverview, WorkspaceOverview};

/// The label column of a `label  value` line: the widest label, `folder on this box`, and two.
const LABEL: usize = 20;

/// Where a repo or a folder is not checked out on this box.
const NOT_ON_THIS_BOX: &str = "not on this box";

/// The mark and its counts: `● open · 2 running`, the counts in the dim role.
pub fn state_spans(state: WorkspaceLifecycle, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = label::spans(label::workspace_lifecycle(state), theme);
    let counts = state.to_string();
    let counts = counts.strip_prefix(state.word()).unwrap_or_default();
    if !counts.is_empty() {
        spans.push(Span::styled(counts.to_owned(), theme.dim));
    }
    spans
}

/// A `label  value` line, the label dim in its column.
fn labelled(name: &str, value: Span<'static>, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(cells::fit(name, LABEL), theme.dim),
        value,
    ])
}

/// A heading line.
fn heading(text: &str, theme: &Theme) -> Line<'static> {
    Line::styled(text.to_owned(), theme.title)
}

/// `text` wrapped at `width`, paragraph by paragraph, a blank line kept between paragraphs.
fn paragraphs(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let rows = wrapped(paragraph, width);
        if rows.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(rows.into_iter().map(|row| Line::styled(row, theme.base)));
    }
    lines
}

/// D8's workspace overview: the state and its counts, the folder, the default target, the held
/// runs, the description, the goals and one line per project. `width` wraps the description and
/// the goals.
pub fn workspace_lines(
    overview: &WorkspaceOverview,
    state: WorkspaceLifecycle,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(state_spans(state, theme)), Line::default()];
    let folder = overview.folder.as_ref().map_or_else(
        || Span::styled("not set", theme.dim),
        |path| Span::styled(path.display().to_string(), theme.base),
    );
    lines.push(labelled("folder on this box", folder, theme));
    lines.push(labelled(
        "default target",
        Span::styled(overview.default_target.to_string(), theme.base),
        theme,
    ));
    let held = overview.counts.held();
    let held = if held == 0 {
        Span::styled("none", theme.dim)
    } else {
        Span::styled(held.to_string(), theme.base)
    };
    lines.push(labelled("held runs", held, theme));

    let description = &overview.tree.workspace.description;
    if !description.trim().is_empty() {
        lines.push(Line::default());
        lines.extend(paragraphs(description, width, theme));
    }

    lines.push(Line::default());
    lines.push(heading("Goals", theme));
    match overview
        .goals
        .as_deref()
        .filter(|goals| !goals.trim().is_empty())
    {
        Some(goals) => lines.extend(paragraphs(goals, width, theme)),
        None => lines.push(Line::styled("no goals yet", theme.dim)),
    }

    lines.push(Line::default());
    lines.push(heading("Projects", theme));
    if overview.tree.projects.is_empty() {
        lines.push(Line::styled("no projects yet", theme.dim));
    }
    let column = overview
        .tree
        .projects
        .iter()
        .map(|entry| cells::cell_width(&entry.project.name))
        .max()
        .unwrap_or(0);
    for entry in &overview.tree.projects {
        let counts = project_counts(overview, entry);
        let items = counts.map_or(0, ProjectOverview::item_total);
        let running = counts.map_or(0, |line| line.running);
        let waiting = counts.map_or(0, |line| line.waiting);
        let primary = entry
            .repos
            .iter()
            .find(|repo| repo.repo.is_primary)
            .or_else(|| entry.repos.first());
        let path = primary
            .and_then(|repo| repo.local_path.as_ref())
            .map_or_else(
                || Span::styled(NOT_ON_THIS_BOX, theme.dim),
                |row| Span::styled(row.local_path.clone(), theme.base),
            );
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}  ", cells::fit(&entry.project.name, column)),
                theme.base,
            ),
            Span::styled(
                format!(
                    "{} \u{b7} {running} running \u{b7} {waiting} waiting  ",
                    plural(items, "item", "items")
                ),
                theme.dim,
            ),
            path,
        ]));
    }
    lines
}

/// The counts the overview holds for `entry`'s project.
fn project_counts<'a>(
    overview: &'a WorkspaceOverview,
    entry: &ProjectEntry,
) -> Option<&'a ProjectOverview> {
    overview
        .projects
        .iter()
        .find(|line| line.project == entry.project.id)
}

/// D8's project overview: its repos and where each is on this box, its description, and its items
/// by state.
pub fn project_lines(
    overview: &WorkspaceOverview,
    entry: &ProjectEntry,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if entry.repos.is_empty() {
        lines.push(Line::styled("no repos yet", theme.dim));
    }
    let column = entry
        .repos
        .iter()
        .map(|repo| cells::cell_width(&repo_name(repo)))
        .max()
        .unwrap_or(0);
    for repo in &entry.repos {
        let path = repo.local_path.as_ref().map_or_else(
            || Span::styled(NOT_ON_THIS_BOX, theme.dim),
            |row| Span::styled(row.local_path.clone(), theme.base),
        );
        lines.push(Line::from(vec![
            Span::styled("repo ", theme.dim),
            Span::styled(
                format!("{}  ", cells::fit(&repo_name(repo), column)),
                theme.base,
            ),
            path,
        ]));
    }

    let description = &entry.project.description;
    if !description.trim().is_empty() {
        lines.push(Line::default());
        lines.extend(paragraphs(description, width, theme));
    }

    lines.push(Line::default());
    lines.push(heading("Items", theme));
    let items = project_counts(overview, entry).map_or(&[][..], |line| line.items.as_slice());
    if items.is_empty() {
        lines.push(Line::styled("no items yet", theme.dim));
    }
    let column = items
        .iter()
        .map(|(state, _)| cells::cell_width(state.word()))
        .max()
        .unwrap_or(0);
    for (state, count) in items {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}  ", cells::fit(state.word(), column)),
                theme.base,
            ),
            Span::styled(count.to_string(), theme.base),
        ]));
    }
    lines
}

/// `name`, or `name (primary)` for the project's primary repo.
fn repo_name(repo: &crate::hierarchy::RepoEntry) -> String {
    if repo.repo.is_primary {
        format!("{} (primary)", repo.repo.name)
    } else {
        repo.repo.name.clone()
    }
}
