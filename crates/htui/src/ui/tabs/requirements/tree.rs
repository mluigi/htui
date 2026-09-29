//! The Requirements tab's left pane: projects, their areas, the areas' requirements (MOD-39 plan
//! P10).
//!
//! The row model is derived, never stored, as the Backlog list's is: [`rows`] rebuilds it from the
//! snapshot, the scope's projects, the folded set and the filter every time it is needed, so a
//! reply, a fold or a keystroke in the filter can never leave a stale index behind. The cursor is a
//! [`Row`] (an id), not an index.

use htui_core::model::{
    ProjectId, ProjectRef, Requirement, RequirementArea, RequirementAreaId, RequirementId,
    RequirementState,
};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::requirements::{ProjectRequirements, RequirementsSnapshot, matches_filter};
use crate::ui::Theme;
use crate::ui::tabs::backlog::list::clip;

/// Two spaces between the columns of a requirement row.
const GAP: usize = 2;

/// How far an area header is indented under its project.
const AREA_INDENT: usize = 2;

/// How far a requirement row is indented under its area.
const ROW_INDENT: usize = 4;

/// Drawn in a withdrawn requirement's indent, under its area's fold marker: the state in a glyph,
/// so a theme without colour still tells it from an active one (PRD scope: key, priority, state,
/// first line of body). Costs no column.
const WITHDRAWN_MARK: char = '\u{2715}';

/// The priority column: `later` is the longer of the two.
const PRIORITY_WIDTH: usize = 5;

/// Which row of the tree the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row {
    /// A project header: `a` adds an area to it, `Enter` folds it.
    Project(ProjectId),
    /// An area header: `n` mints into it, `Enter` folds it.
    Area(RequirementAreaId),
    /// A requirement: the detail pane shows it, `e` amends and `W` withdraws it.
    Requirement(RequirementId),
}

/// A folded header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fold {
    /// A project whose areas are hidden.
    Project(ProjectId),
    /// An area whose requirements are hidden.
    Area(RequirementAreaId),
}

/// What [`lines`] needs, borrowed from the tab.
#[derive(Debug)]
pub(super) struct TreeView<'a> {
    /// The last read.
    pub(super) snapshot: &'a RequirementsSnapshot,
    /// The scope's projects, in `workspace_project.position` order.
    pub(super) projects: &'a [ProjectRef],
    /// Folded headers.
    pub(super) folded: &'a [Fold],
    /// The filter, as applied or as being typed (plan P4).
    pub(super) filter: &'a str,
    /// The cursor.
    pub(super) selected: Option<Row>,
}

/// One project's side of the tree: its entry, and per area the requirements the filter keeps.
struct Group<'a> {
    /// The scope's row for the project: its slug and name.
    project: &'a ProjectRef,
    /// The snapshot's entry.
    entry: &'a ProjectRequirements,
    /// Each area with its matching requirements, `(position, code)` order.
    areas: Vec<(&'a RequirementArea, Vec<&'a Requirement>)>,
}

impl Group<'_> {
    /// How many requirements the filter keeps in this project.
    fn count(&self) -> usize {
        self.areas.iter().map(|(_, rows)| rows.len()).sum()
    }
}

/// The groups in `projects` order. With a non-empty `filter` only matching requirements are kept,
/// and a header with none under it is dropped.
fn groups<'a>(
    snapshot: &'a RequirementsSnapshot,
    projects: &'a [ProjectRef],
    filter: &str,
) -> Vec<Group<'a>> {
    let filtering = !filter.trim().is_empty();
    projects
        .iter()
        .filter_map(|project| {
            let entry = snapshot.project(project.project_id)?;
            let areas: Vec<_> = entry
                .areas
                .iter()
                .map(|area| {
                    let rows: Vec<&Requirement> = entry
                        .in_area(area.id)
                        .filter(|row| matches_filter(row, filter))
                        .collect();
                    (area, rows)
                })
                .filter(|(_, rows)| !filtering || !rows.is_empty())
                .collect();
            let group = Group {
                project,
                entry,
                areas,
            };
            (!filtering || group.count() > 0).then_some(group)
        })
        .collect()
}

/// The visible rows, top to bottom (plan P10): `ctx.projects` order; under an unfolded project its
/// areas, under an unfolded area its requirements. With a non-empty filter only matching
/// requirements, and the headers that have one.
pub(super) fn rows(
    snapshot: &RequirementsSnapshot,
    projects: &[ProjectRef],
    folded: &[Fold],
    filter: &str,
) -> Vec<Row> {
    let mut out = Vec::new();
    for group in groups(snapshot, projects, filter) {
        out.push(Row::Project(group.project.project_id));
        if folded.contains(&Fold::Project(group.project.project_id)) {
            continue;
        }
        for (area, requirements) in &group.areas {
            out.push(Row::Area(area.id));
            if folded.contains(&Fold::Area(area.id)) {
                continue;
            }
            out.extend(requirements.iter().map(|row| Row::Requirement(row.id)));
        }
    }
    out
}

/// How many requirements the tree shows with this filter, folded ones included: the pane title's
/// count.
pub(super) fn count(
    snapshot: &RequirementsSnapshot,
    projects: &[ProjectRef],
    filter: &str,
) -> usize {
    groups(snapshot, projects, filter)
        .iter()
        .map(Group::count)
        .sum()
}

/// `theme.dim` for a withdrawn requirement, `theme.base` otherwise (PRD "withdrawn dimmed").
pub(super) fn requirement_style(requirement: &Requirement, theme: &Theme) -> Style {
    match requirement.state {
        RequirementState::Withdrawn => theme.dim,
        RequirementState::Active => theme.base,
    }
}

/// The first line of `text`.
pub(super) fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// One line per visible row, `width` columns wide, and the index of the cursor's line. A project
/// with no area draws a dim `no areas` line that is not a row.
pub(super) fn lines(
    view: &TreeView<'_>,
    width: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, Option<usize>) {
    let groups = groups(view.snapshot, view.projects, view.filter);
    let key_width = groups
        .iter()
        .flat_map(|group| group.areas.iter().flat_map(|(_, rows)| rows.iter()))
        .map(|row| row.key.chars().count())
        .max()
        .unwrap_or(0);
    let body_width = width
        .saturating_sub(ROW_INDENT + key_width + GAP + PRIORITY_WIDTH + GAP)
        .max(1);

    let mut out = Vec::new();
    let mut cursor = None;
    let mut push = |line: Line<'static>, row: Row, out: &mut Vec<Line<'static>>| {
        let selected = view.selected == Some(row);
        if selected {
            cursor = Some(out.len());
        }
        out.push(pad(line, width, selected, theme));
    };
    for group in &groups {
        let id = group.project.project_id;
        let folded = view.folded.contains(&Fold::Project(id));
        let mut spans = vec![Span::styled(
            format!(
                "{} {} ({})",
                marker(folded),
                group.project.name,
                group.count()
            ),
            theme.title,
        )];
        if !group.entry.maintainer {
            spans.push(Span::styled(" \u{b7} read-only", theme.dim));
        }
        push(Line::from(spans), Row::Project(id), &mut out);
        if folded {
            continue;
        }
        if group.areas.is_empty() {
            out.push(Line::styled(
                format!("{}no areas", " ".repeat(AREA_INDENT)),
                theme.dim,
            ));
            continue;
        }
        for (area, requirements) in &group.areas {
            let folded = view.folded.contains(&Fold::Area(area.id));
            let text = format!(
                "{}{} {} {} ({})",
                " ".repeat(AREA_INDENT),
                marker(folded),
                area.code,
                area.title,
                requirements.len()
            );
            push(
                Line::from(Span::styled(text, theme.title)),
                Row::Area(area.id),
                &mut out,
            );
            if folded {
                continue;
            }
            for requirement in requirements {
                let line = requirement_line(requirement, key_width, body_width, theme);
                push(line, Row::Requirement(requirement.id), &mut out);
            }
        }
    }
    (out, cursor)
}

/// `    R-ENT-1  must   <first line of body>`: the key accented, the rest in
/// [`requirement_style`]; a withdrawn requirement is `  ✕ R-ENT-2  …`, every span dim.
fn requirement_line(
    requirement: &Requirement,
    key_width: usize,
    body_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let style = requirement_style(requirement, theme);
    let withdrawn = requirement.state == RequirementState::Withdrawn;
    let key_style = if withdrawn { theme.dim } else { theme.accent };
    let indent = if withdrawn {
        format!("{}{WITHDRAWN_MARK} ", " ".repeat(AREA_INDENT))
    } else {
        " ".repeat(ROW_INDENT)
    };
    Line::from(vec![
        Span::styled(indent, style),
        Span::styled(padded(&requirement.key, key_width), key_style),
        Span::styled(" ".repeat(GAP), style),
        Span::styled(padded(requirement.priority.as_str(), PRIORITY_WIDTH), style),
        Span::styled(" ".repeat(GAP), style),
        Span::styled(clip(first_line(&requirement.body), body_width), style),
    ])
}

/// `▸` folded, `▾` open.
const fn marker(folded: bool) -> char {
    if folded { '\u{25b8}' } else { '\u{25be}' }
}

/// `text` padded with spaces to `width` chars.
fn padded(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    out.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(text.chars().count()),
    ));
    out
}

/// The line cut to `width`, padded out to it so the selected style covers the whole row.
fn pad(line: Line<'static>, width: usize, selected: bool, theme: &Theme) -> Line<'static> {
    let mut spans = line.spans;
    let used: usize = spans.iter().map(|span| span.content.chars().count()).sum();
    if used > width {
        // A header too long for the pane: cut its last span, as a requirement's body is cut.
        let over = used - width;
        if let Some(last) = spans.last_mut() {
            let keep = last.content.chars().count().saturating_sub(over);
            last.content = clip(&last.content, keep).into();
        }
    } else {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    let line = Line::from(spans);
    if selected {
        line.style(theme.selected)
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::{Fold, Row, TreeView, lines, rows};
    use crate::ui::Theme;
    use crate::ui::tabs::requirements::tests::platform;
    use htui_core::fixtures::ids;
    use htui_core::model::RequirementState;

    #[tokio::test]
    async fn the_filter_keeps_matching_rows_and_their_headers() {
        let (snapshot, projects, _) = platform().await;
        let all = rows(&snapshot, &projects, &[], "");
        assert_eq!(
            all,
            vec![
                Row::Project(ids::PROJECT_HTUI),
                Row::Area(ids::AREA_ENT),
                Row::Requirement(ids::REQ_ENT_1),
                Row::Requirement(ids::REQ_ENT_2),
                Row::Area(ids::AREA_STO),
                Row::Requirement(ids::REQ_STO_1),
                Row::Project(ids::PROJECT_AGY),
            ]
        );
        // `sto` matches R-STO-1's key; ENT and agy have nothing to show and go.
        assert_eq!(
            rows(&snapshot, &projects, &[], "sto"),
            vec![
                Row::Project(ids::PROJECT_HTUI),
                Row::Area(ids::AREA_STO),
                Row::Requirement(ids::REQ_STO_1),
            ]
        );
        // `priority` matches R-ENT-2's body only.
        assert_eq!(
            rows(&snapshot, &projects, &[], "PRIORITY"),
            vec![
                Row::Project(ids::PROJECT_HTUI),
                Row::Area(ids::AREA_ENT),
                Row::Requirement(ids::REQ_ENT_2),
            ]
        );
        assert!(rows(&snapshot, &projects, &[], "nothing like it").is_empty());
        // A folded header keeps its row and hides what is under it.
        assert_eq!(
            rows(&snapshot, &projects, &[Fold::Area(ids::AREA_ENT)], ""),
            vec![
                Row::Project(ids::PROJECT_HTUI),
                Row::Area(ids::AREA_ENT),
                Row::Area(ids::AREA_STO),
                Row::Requirement(ids::REQ_STO_1),
                Row::Project(ids::PROJECT_AGY),
            ]
        );
    }

    #[tokio::test]
    async fn withdrawn_rows_render_dim() {
        let (mut snapshot, projects, _) = platform().await;
        for entry in &mut snapshot.projects {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_2 {
                    row.state = RequirementState::Withdrawn;
                }
            }
        }
        let theme = Theme::default();
        let view = TreeView {
            snapshot: &snapshot,
            projects: &projects,
            folded: &[],
            filter: "",
            selected: None,
        };
        let (lines, _) = lines(&view, 45, &theme);
        let text = |line: &ratatui::text::Line<'_>| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        let withdrawn = lines
            .iter()
            .find(|line| text(line).contains("R-ENT-2"))
            .expect("a withdrawn requirement stays listed");
        assert!(
            text(withdrawn).starts_with("  \u{2715} R-ENT-2"),
            "the state is marked without colour too: {}",
            text(withdrawn)
        );
        for span in withdrawn
            .spans
            .iter()
            .filter(|span| !span.content.trim().is_empty())
        {
            assert_eq!(span.style, theme.dim, "`{}` is dim", span.content);
        }
        let active = lines
            .iter()
            .find(|line| text(line).contains("R-ENT-1"))
            .expect("R-ENT-1 is listed");
        assert!(
            active.spans.iter().any(|span| span.style == theme.accent),
            "an active key is accented"
        );
        assert!(
            active
                .spans
                .iter()
                .filter(|span| !span.content.trim().is_empty())
                .all(|span| span.style != theme.dim),
            "and nothing of an active row is dim"
        );
        assert!(
            text(active).starts_with("    R-ENT-1"),
            "nor marked: {}",
            text(active)
        );
    }
}
