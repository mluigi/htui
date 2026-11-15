//! The Workspaces tab's detail lines (MOD-116 M1 D8): a workspace's overview, or one of its
//! projects', built from one [`WorkspaceOverview`] reply. Pure: lines in, no frame, so the tab's
//! render only scrolls and draws them. A value too long for its row (a path, the inherited
//! target's sentence) wraps under itself rather than losing its tail at the border.

use htui_core::model::WorkspaceLifecycle;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::hierarchy::{ProjectEntry, RepoEntry};
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::label::{self, plural};
use crate::ui::kit::text::wrapped;
use crate::workspaces::{ProjectOverview, WorkspaceOverview};

/// The label column of a `label  value` line: the widest label, `folder on this box`, and two.
const LABEL: usize = 20;

/// The indent of a project's counts under its name.
const INDENT: usize = 2;

/// Where a repo is not checked out on this box.
const NOT_ON_THIS_BOX: &str = "not on this box";

/// A project with no repo.
const NO_REPOS: &str = "no repos yet";

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

/// `lead` (`lead_width` cells), then `value` wrapped in the `width` left after it, each further
/// row indented to line up under the first.
fn hanging(
    lead: Vec<Span<'static>>,
    lead_width: usize,
    value: &str,
    style: Style,
    width: usize,
) -> Vec<Line<'static>> {
    let mut rows = wrapped(value, width.saturating_sub(lead_width).max(1)).into_iter();
    let mut first = lead;
    first.push(Span::styled(rows.next().unwrap_or_default(), style));
    let mut lines = vec![Line::from(first)];
    lines.extend(rows.map(|row| {
        Line::from(vec![
            Span::raw(" ".repeat(lead_width)),
            Span::styled(row, style),
        ])
    }));
    lines
}

/// A `label  value` line, the label dim in its column, the value wrapped under itself.
fn labelled(
    name: &str,
    value: &str,
    style: Style,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    hanging(
        vec![Span::styled(cells::fit(name, LABEL), theme.dim)],
        LABEL,
        value,
        style,
        width,
    )
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

/// Where `repo` is checked out on this box, and the style it reads in; a project with no repo
/// says so.
fn repo_path(repo: Option<&RepoEntry>, theme: &Theme) -> (String, Style) {
    match repo {
        None => (NO_REPOS.to_owned(), theme.dim),
        Some(repo) => repo.local_path.as_ref().map_or_else(
            || (NOT_ON_THIS_BOX.to_owned(), theme.dim),
            |row| (row.local_path.clone(), theme.base),
        ),
    }
}

/// D8's workspace overview: the state and its counts, the folder, the default target, the held
/// runs, the description, the goals and each project with its counts and where its primary repo
/// is. `width` is the pane's inner width.
pub fn workspace_lines(
    overview: &WorkspaceOverview,
    state: WorkspaceLifecycle,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(state_spans(state, theme)), Line::default()];
    let (folder, style) = overview.folder.as_ref().map_or_else(
        || ("not set".to_owned(), theme.dim),
        |path| (path.display().to_string(), theme.base),
    );
    lines.extend(labelled("folder on this box", &folder, style, width, theme));
    lines.extend(labelled(
        "default target",
        &overview.default_target.to_string(),
        theme.base,
        width,
        theme,
    ));
    let held = overview.counts.held();
    let (held, style) = if held == 0 {
        ("none".to_owned(), theme.dim)
    } else {
        (held.to_string(), theme.base)
    };
    lines.extend(labelled("held runs", &held, style, width, theme));

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
        let primary = entry
            .repos
            .iter()
            .find(|repo| repo.repo.is_primary)
            .or_else(|| entry.repos.first());
        let (path, style) = repo_path(primary, theme);
        let name = format!("{}  ", cells::fit(&entry.project.name, column));
        lines.extend(hanging(
            vec![Span::styled(name, theme.base)],
            column + 2,
            &path,
            style,
            width,
        ));
        let counts = project_counts(overview, entry);
        let items = counts.map_or(0, ProjectOverview::item_total);
        let running = counts.map_or(0, |line| line.running);
        let waiting = counts.map_or(0, |line| line.waiting);
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(INDENT)),
            Span::styled(
                format!(
                    "{} \u{b7} {running} running \u{b7} {waiting} waiting",
                    plural(items, "item", "items")
                ),
                theme.dim,
            ),
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
        lines.push(Line::styled(NO_REPOS, theme.dim));
    }
    let column = entry
        .repos
        .iter()
        .map(|repo| cells::cell_width(&repo_name(repo)))
        .max()
        .unwrap_or(0);
    for repo in &entry.repos {
        let (path, style) = repo_path(Some(repo), theme);
        lines.extend(hanging(
            vec![
                Span::styled("repo ", theme.dim),
                Span::styled(
                    format!("{}  ", cells::fit(&repo_name(repo), column)),
                    theme.base,
                ),
            ],
            column + 7,
            &path,
            style,
            width,
        ));
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
fn repo_name(repo: &RepoEntry) -> String {
    if repo.repo.is_primary {
        format!("{} (primary)", repo.repo.name)
    } else {
        repo.repo.name.clone()
    }
}
