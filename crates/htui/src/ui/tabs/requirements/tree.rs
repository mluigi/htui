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
use crate::ui::cells::{self, cell_width};

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

/// A project header's tail when another user owns the spec.
const READ_ONLY_TAIL: &str = " \u{b7} read-only";

/// The fewest cells of a project name worth drawing, its `…` counted, before the header gives up
/// its tail (MOD-60 D7; `graph.rs::fit_label`'s `slug_room >= 2`). Measured on the clipped name,
/// not the room: two cells of room hold no 2-cell glyph, only a bare `…`.
const NAME_MIN: usize = 2;

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
        .map(|row| cell_width(&row.key))
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
        push(
            project_header(group, folded, width, theme),
            Row::Project(id),
            &mut out,
        );
        if folded {
            continue;
        }
        if group.areas.is_empty() {
            // Not a row, so `pad` never sees it: cut here (MOD-60 B4).
            out.push(Line::styled(
                cells::clip(&format!("{}no areas", " ".repeat(AREA_INDENT)), width),
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

/// `▾ htui (3) · read-only`, the name cut so the rest fits `width` cells (MOD-60 D7): the marker,
/// the count and the tail are the information, the name is what gives way. When fewer than
/// [`NAME_MIN`] cells of name would be left, the tail goes first; narrower still, the whole header
/// is handed to [`pad`], which cuts it from the end.
fn project_header(group: &Group<'_>, folded: bool, width: usize, theme: &Theme) -> Line<'static> {
    let head = format!("{} ", marker(folded));
    let count = format!(" ({})", group.count());
    let name = &group.project.name;
    let read_only = !group.entry.maintainer;
    let fixed = cell_width(&head) + cell_width(&count);
    let tail_width = if read_only {
        cell_width(READ_ONLY_TAIL)
    } else {
        0
    };
    // The name clipped to `room`, kept when it fits whole or still draws [`NAME_MIN`] cells (B11).
    let clipped = |room: usize| {
        let cut = cells::clip(name, room);
        (cell_width(name) <= room || cell_width(&cut) >= NAME_MIN).then_some(cut)
    };
    let title = |name: &str| Span::styled(format!("{head}{name}{count}"), theme.title);
    let tail = || Span::styled(READ_ONLY_TAIL, theme.dim);
    if let Some(cut) = width.checked_sub(fixed + tail_width).and_then(clipped) {
        let mut spans = vec![title(&cut)];
        if read_only {
            spans.push(tail());
        }
        Line::from(spans)
    } else if read_only && let Some(cut) = width.checked_sub(fixed).and_then(clipped) {
        Line::from(title(&cut))
    } else {
        let mut spans = vec![title(name)];
        if read_only {
            spans.push(tail());
        }
        Line::from(spans)
    }
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
        Span::styled(cells::pad(&requirement.key, key_width), key_style),
        Span::styled(" ".repeat(GAP), style),
        Span::styled(
            cells::pad(requirement.priority.as_str(), PRIORITY_WIDTH),
            style,
        ),
        Span::styled(" ".repeat(GAP), style),
        Span::styled(
            cells::clip(first_line(&requirement.body), body_width),
            style,
        ),
    ])
}

/// `▸` folded, `▾` open.
const fn marker(folded: bool) -> char {
    if folded { '\u{25b8}' } else { '\u{25be}' }
}

/// The line cut to `width` cells (MOD-60 D7: `cells::clip_spans`, so a body row cuts its body),
/// padded out to it so the selection covers the whole row.
fn pad(line: Line<'static>, width: usize, selected: bool, theme: &Theme) -> Line<'static> {
    let mut spans = cells::clip_spans(&line.spans, width);
    let used: usize = spans.iter().map(|span| cell_width(&span.content)).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    let line = Line::from(spans);
    if selected { theme.select(line) } else { line }
}

#[cfg(test)]
mod tests {
    use super::{Fold, Row, TreeView, lines, rows};
    use crate::requirements::RequirementsSnapshot;
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crate::ui::tabs::requirements::tests::platform;
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, RequirementAreaId, RequirementState};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::text::Line;
    use ratatui::widgets::{Paragraph, Widget as _};
    use uuid::Uuid;

    /// A line's text, span after span.
    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// A line's width in cells, as the terminal draws it.
    fn line_width(line: &Line<'_>) -> usize {
        line.spans
            .iter()
            .map(|span| cell_width(&span.content))
            .sum()
    }

    /// Project htui renamed to `name`.
    fn rename(projects: &mut [ProjectRef], name: &str) {
        for project in projects
            .iter_mut()
            .filter(|project| project.project_id == ids::PROJECT_HTUI)
        {
            project.name = name.to_owned();
        }
    }

    /// Project htui's spec owned by this user (`true`) or another (`false`).
    fn set_maintainer(snapshot: &mut RequirementsSnapshot, maintainer: bool) {
        for entry in snapshot
            .projects
            .iter_mut()
            .filter(|entry| entry.project_id == ids::PROJECT_HTUI)
        {
            entry.maintainer = maintainer;
        }
    }

    /// The tree's lines at `width`, nothing folded, nothing selected.
    fn tree_lines(
        snapshot: &RequirementsSnapshot,
        projects: &[ProjectRef],
        width: usize,
    ) -> Vec<Line<'static>> {
        let view = TreeView {
            snapshot,
            projects,
            folded: &[],
            filter: "",
            selected: None,
        };
        lines(&view, width, &Theme::default()).0
    }

    /// MOD-60 D7: the marker, the count and ` · read-only` are the information; the name is what
    /// gives way.
    #[tokio::test]
    async fn a_narrow_pane_keeps_read_only_and_elides_the_project_name() {
        let (mut snapshot, mut projects, _) = platform().await;
        rename(&mut projects, "a very long project name");
        set_maintainer(&mut snapshot, false);
        let lines = tree_lines(&snapshot, &projects, 24);
        assert_eq!(
            line_text(&lines[0]),
            "\u{25be} a ver\u{2026} (3) \u{b7} read-only"
        );
    }

    /// MOD-60 D7's second step: with fewer than two cells of name left beside the tail, the tail
    /// goes and the name takes its room; narrower still, the header is cut from the end and the
    /// marker is what stays.
    #[tokio::test]
    async fn a_pane_too_narrow_for_the_tail_drops_it_before_the_name() {
        let (mut snapshot, mut projects, _) = platform().await;
        rename(&mut projects, "a very long project name");
        set_maintainer(&mut snapshot, false);
        assert_eq!(
            line_text(&tree_lines(&snapshot, &projects, 18)[0]),
            "\u{25be} a very long\u{2026} (3)"
        );
        assert_eq!(
            line_text(&tree_lines(&snapshot, &projects, 7)[0]),
            "\u{25be} a ve\u{2026}"
        );
    }

    /// MOD-60 D7 counts the name that is drawn, not the room: two cells of room hold no 2-cell
    /// glyph, only a bare `…`, so the tail gives way rather than the whole name.
    #[tokio::test]
    async fn a_wide_first_glyph_drops_the_tail_before_the_whole_name() {
        let (mut snapshot, mut projects, _) = platform().await;
        rename(&mut projects, &"\u{6f22}\u{5b57}".repeat(6));
        set_maintainer(&mut snapshot, false);
        assert_eq!(
            line_text(&tree_lines(&snapshot, &projects, 21)[0]),
            "\u{25be} \u{6f22}\u{2026} (3) \u{b7} read-only"
        );
        assert_eq!(
            line_text(&tree_lines(&snapshot, &projects, 20)[0]),
            format!("\u{25be} {}\u{2026} (3) ", "\u{6f22}\u{5b57}".repeat(3)),
            "19 cells of header, padded to the pane"
        );
    }

    #[tokio::test]
    async fn a_wide_project_name_is_clipped_by_cells() {
        let (mut snapshot, mut projects, _) = platform().await;
        rename(&mut projects, &"\u{6f22}\u{5b57}".repeat(6));
        set_maintainer(&mut snapshot, true);
        let lines = tree_lines(&snapshot, &projects, 20);
        let header = line_text(&lines[0]);
        assert_eq!(cell_width(&header), 20, "{header:?}");
        assert!(header.contains(" (3)"), "{header:?}");
        assert!(header.contains('\u{2026}'), "{header:?}");
        assert!(header.starts_with("\u{25be} "), "{header:?}");
    }

    /// Every row kind (project header with its tail, area header, an area with no requirement,
    /// a requirement with a wide body, `no areas`) at every width a pane can be.
    #[tokio::test]
    async fn no_tree_line_is_wider_than_the_pane_at_any_width() {
        let (mut snapshot, mut projects, _) = platform().await;
        rename(&mut projects, &"\u{6f22}\u{5b57}".repeat(6));
        set_maintainer(&mut snapshot, false);
        for entry in snapshot
            .projects
            .iter_mut()
            .filter(|entry| entry.project_id == ids::PROJECT_HTUI)
        {
            let mut empty = entry.areas[0].clone();
            empty.id = RequirementAreaId::from_uuid(Uuid::from_u128(0x60));
            empty.code = "EMPTY".to_owned();
            empty.title = "\u{7a7a}".repeat(10);
            entry.areas.push(empty);
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    row.body = "\u{6587}".repeat(30);
                }
            }
        }
        let wide: Vec<String> = tree_lines(&snapshot, &projects, 45)
            .iter()
            .map(line_text)
            .collect();
        for needle in ["read-only", "EMPTY", "\u{6587}", "no areas"] {
            assert!(
                wide.iter().any(|text| text.contains(needle)),
                "the fixture draws {needle:?}: {wide:#?}"
            );
        }
        for width in 1..=45 {
            for line in &tree_lines(&snapshot, &projects, width) {
                let text = line_text(line);
                assert!(line_width(line) <= width, "{text:?} at {width}");
            }
        }
    }

    /// MOD-60 D12: the key column is measured in cells, so a key of wide glyphs (6 chars,
    /// 10 cells) does not push its row's priority right of its neighbours' (7 chars, 7 cells).
    #[tokio::test]
    async fn a_wide_key_keeps_the_priority_column() {
        let (mut snapshot, projects, _) = platform().await;
        let mut priorities = Vec::new();
        for entry in snapshot
            .projects
            .iter_mut()
            .filter(|entry| entry.project_id == ids::PROJECT_HTUI)
        {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    row.key = "\u{6f22}\u{5b57}\u{6f22}\u{5b57}-1".to_owned();
                }
                priorities.push((row.id, row.priority.as_str()));
            }
        }
        let drawn = tree_lines(&snapshot, &projects, 80);
        let offsets: Vec<(String, usize)> = rows(&snapshot, &projects, &[], "")
            .iter()
            .zip(&drawn)
            .filter_map(|(row, line)| match row {
                Row::Requirement(id) => Some((*id, line_text(line))),
                _ => None,
            })
            .map(|(id, text)| {
                let (_, priority) = priorities
                    .iter()
                    .find(|(row, _)| *row == id)
                    .expect("every drawn requirement is htui's");
                let at = text
                    .find(priority)
                    .unwrap_or_else(|| panic!("{text:?} shows its priority"));
                let offset = cell_width(&text[..at]);
                (text, offset)
            })
            .collect();
        assert!(
            offsets.len() >= 3,
            "the wide key and its ASCII neighbours are drawn: {offsets:#?}"
        );
        assert!(
            offsets.iter().any(|(text, _)| text.contains('\u{6f22}')),
            "the wide key is drawn: {offsets:#?}"
        );
        assert!(
            offsets.windows(2).all(|pair| pair[0].1 == pair[1].1),
            "the priority starts at one cell offset on every row: {offsets:#?}"
        );
    }

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

    /// MOD-80 D2: a selected requirement row is one even block; its key's own style does not
    /// show through the selection.
    #[tokio::test]
    async fn a_selected_requirement_row_is_one_even_block() {
        let (snapshot, projects, _) = platform().await;
        let view = TreeView {
            snapshot: &snapshot,
            projects: &projects,
            folded: &[],
            filter: "",
            selected: Some(Row::Requirement(ids::REQ_ENT_1)),
        };
        let (lines, cursor) = lines(&view, 45, &Theme::default());
        let y = cursor.expect("R-ENT-1 is drawn");
        assert!(line_text(&lines[y]).contains("R-ENT-1"), "{lines:#?}");
        let height = u16::try_from(lines.len()).expect("a short tree");
        let area = Rect::new(0, 0, 45, height);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(lines).render(area, &mut buffer);
        let y = u16::try_from(y).expect("a short tree");
        for x in 0..45 {
            let cell = &buffer[(x, y)];
            assert_eq!(cell.fg, Color::Black, "x = {x}: {cell:?}");
            assert_eq!(cell.bg, Color::Cyan, "x = {x}: {cell:?}");
        }
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
        let text = |line: &Line<'_>| {
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
