//! The Requirements tab's right pane in Browse (MOD-39 plan P7, P10): one requirement's body,
//! rationale, `priority · state · version`, its coverage and its revision trail; or what a project
//! or an area header says.
//!
//! Lines are built here and drawn by the tab, which scrolls them (`J`/`K`, `PgUp`/`PgDn`). Offline
//! the mirror holds no revisions, and the pane says so in one line rather than showing an empty
//! trail (PRD "Offline", blueprint F-14).

use htui_core::model::{CoverageRow, ProjectRef, RequirementArea, RequirementState};
use ratatui::text::{Line, Span};

use crate::requirements::{ProjectRequirements, RequirementDetail, RevisionRow};
use crate::ui::Theme;
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::backlog::detail::STAMP;
use crate::ui::tabs::settings::wrapped;

/// What the Revisions heading says over a detail read from the mirror.
pub(super) const REVISIONS_NEED_THE_DATABASE: &str = "revisions need the database";

/// A requirement no item cites.
const NO_COVERAGE: &str = "no item cites it";

/// A project row, this user owning its spec.
const OWNED: &str = "you own these requirements";

/// A project row, another user owning its spec (PRD D1).
const READ_ONLY: &str = "read-only: another user owns these requirements";

/// A project row with no spec yet.
const UNCLAIMED: &str = "no spec yet: the first write claims it";

/// The widest citation kind (`addresses`, `withdraws`).
const KIND_WIDTH: usize = 9;

/// A citation stamped at a version the requirement has since moved past.
const SUSPECT: &str = "! suspect";

/// The widest revision reason the store writes (`withdrawn`).
const REASON_WIDTH: usize = 9;

/// `text` wrapped to `width`, one hard line at a time: a blank line of the body stays blank.
fn paragraph(text: &str, width: usize, style: ratatui::style::Style) -> Vec<Line<'static>> {
    text.lines()
        .flat_map(|line| {
            let rows = wrapped(line, width);
            if rows.is_empty() {
                vec![String::new()]
            } else {
                rows
            }
        })
        .map(|row| Line::styled(row, style))
        .collect()
}

/// One requirement, `width` columns wide (plan P7): the body; its rationale, when it has one;
/// `must · active · v2`; the coverage rows, a suspect one marked in `theme.error`; the revisions,
/// each with the key of the item that decided it, or [`REVISIONS_NEED_THE_DATABASE`].
pub(super) fn lines(detail: &RequirementDetail, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let requirement = &detail.requirement;
    let body_style = match requirement.state {
        RequirementState::Withdrawn => theme.dim,
        RequirementState::Active => theme.base,
    };
    let mut out = paragraph(&requirement.body, width, body_style);
    out.push(Line::default());
    if !requirement.rationale.trim().is_empty() {
        let text = format!("rationale: {}", requirement.rationale);
        out.extend(paragraph(&text, width, theme.dim));
    }
    out.push(Line::styled(
        format!(
            "{} \u{b7} {} \u{b7} v{}",
            requirement.priority, requirement.state, requirement.version
        ),
        theme.base,
    ));
    out.push(Line::default());
    out.push(Line::styled("Coverage", theme.title));
    if detail.coverage.is_empty() {
        out.push(Line::styled(NO_COVERAGE, theme.dim));
    } else {
        out.extend(coverage(&detail.coverage, theme));
    }
    out.push(Line::default());
    out.push(Line::styled("Revisions", theme.title));
    match &detail.revisions {
        Some(rows) => out.extend(rows.iter().map(|row| revision(row, theme))),
        None => out.push(Line::styled(REVISIONS_NEED_THE_DATABASE, theme.dim)),
    }
    out
}

/// `ANA-1  addresses v1  ! suspect  done  —`, one line per citing item.
///
/// The suspect marker comes straight after the stamp it judges (as the item's Reqs sub-tab puts
/// it), not at the end of the row: the pane does not wrap, and at 80 columns it is 42 wide, so a
/// last column would be cut first. Its column is there only when some row is suspect, and the
/// resolution, last, is not padded.
fn coverage(rows: &[CoverageRow], theme: &Theme) -> Vec<Line<'static>> {
    let key_width = rows
        .iter()
        .map(|row| cell_width(&row.item.key))
        .max()
        .unwrap_or(0);
    let status_width = rows
        .iter()
        .map(|row| cell_width(row.item.status.as_str()))
        .max()
        .unwrap_or(0);
    let any_suspect = rows.iter().any(|row| row.suspect);
    rows.iter()
        .map(|row| {
            let resolution = row.resolution.map_or_else(
                || "\u{2014}".to_owned(),
                |resolution| resolution.to_string(),
            );
            let mut spans = vec![
                Span::styled(cells::pad(&row.item.key, key_width), theme.accent),
                Span::raw("  "),
                Span::styled(cells::pad(row.kind.as_str(), KIND_WIDTH), theme.base),
                Span::raw(" "),
                Span::styled(
                    cells::pad(&format!("v{}", row.requirement_version), 3),
                    theme.base,
                ),
                Span::raw(" "),
            ];
            if row.suspect {
                spans.push(Span::styled(SUSPECT, theme.error));
                spans.push(Span::raw("  "));
            } else if any_suspect {
                spans.push(Span::raw(cells::pad("", cell_width(SUSPECT) + 2)));
            }
            spans.push(Span::styled(
                cells::pad(row.item.status.as_str(), status_width),
                theme.status_style(row.item.status),
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled(resolution, theme.dim));
            Line::from(spans)
        })
        .collect()
}

/// `v2   amended   by ANA-2  09-05 10:00`.
fn revision(row: &RevisionRow, theme: &Theme) -> Line<'static> {
    let by = row
        .deciding_key
        .as_ref()
        .map_or_else(String::new, |key| format!("by {key}"));
    Line::from(vec![
        Span::styled(
            cells::pad(&format!("v{}", row.revision.version), 4),
            theme.base,
        ),
        Span::raw(" "),
        Span::styled(cells::pad(&row.revision.reason, REASON_WIDTH), theme.base),
        Span::raw(" "),
        Span::styled(cells::pad(&by, 12), theme.accent),
        Span::raw(" "),
        Span::styled(row.revision.created_at.format(STAMP).to_string(), theme.dim),
    ])
}

/// A project header's pane: who owns its requirements, and how many there are.
pub(super) fn project_lines(
    project: &ProjectRef,
    entry: &ProjectRequirements,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let owner = match (&entry.spec, entry.maintainer) {
        (None, _) => UNCLAIMED,
        (Some(_), true) => OWNED,
        (Some(_), false) => READ_ONLY,
    };
    let mut out = vec![Line::styled(project.name.clone(), theme.title)];
    out.extend(paragraph(owner, usize::from(width).max(1), theme.dim));
    out.push(Line::default());
    out.push(Line::styled(
        format!(
            "{} areas \u{b7} {} requirements",
            entry.areas.len(),
            entry.requirements.len()
        ),
        theme.base,
    ));
    out
}

/// An area header's pane: its code, title and how many requirements it holds.
pub(super) fn area_lines(
    area: &RequirementArea,
    entry: &ProjectRequirements,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let count = entry.in_area(area.id).count();
    let mut out = vec![Line::styled(
        format!("{} {}", area.code, area.title),
        theme.title,
    )];
    if !area.description.trim().is_empty() {
        out.extend(paragraph(
            &area.description,
            usize::from(width).max(1),
            theme.dim,
        ));
    }
    out.push(Line::default());
    out.push(Line::styled(format!("{count} requirements"), theme.base));
    out
}

#[cfg(test)]
mod tests {
    use super::{REVISIONS_NEED_THE_DATABASE, lines};
    use crate::requirements::{self, RequirementDetail};
    use crate::ui::Theme;
    use htui_core::fixtures::ids;
    use htui_core::store::MemStore;
    use htui_store::Backend;

    fn text(lines: &[ratatui::text::Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn r_ent_1() -> RequirementDetail {
        requirements::detail(&Backend::memory(MemStore::demo()), ids::REQ_ENT_1)
            .await
            .expect("the demo's R-ENT-1")
    }

    #[tokio::test]
    async fn the_detail_lists_coverage_and_revisions() {
        let theme = Theme::default();
        let drawn = text(&lines(&r_ent_1().await, 53, &theme));
        assert!(drawn.contains("must \u{b7} active \u{b7} v2"), "{drawn}");
        assert!(
            drawn.contains("rationale: Keys are what people type."),
            "{drawn}"
        );
        let suspect = drawn
            .lines()
            .find(|line| line.starts_with("ANA-1"))
            .expect("ANA-1 cites R-ENT-1");
        assert!(suspect.contains("addresses v1  ! suspect"), "{suspect}");
        let amends = drawn
            .lines()
            .find(|line| line.starts_with("ANA-2"))
            .expect("ANA-2 amended R-ENT-1");
        assert!(!amends.contains("suspect"), "{amends}");
        assert!(drawn.contains("by ANA-2"), "{drawn}");
        assert!(!drawn.contains(REVISIONS_NEED_THE_DATABASE), "{drawn}");
    }

    /// The pane draws without wrapping, so a row is cut at its right edge. At 80x24 the detail
    /// pane is 42 columns inside its border: the suspect marker must fit in them, whatever the
    /// citing item's status (`awaiting_approval` is the widest).
    #[tokio::test]
    async fn the_suspect_marker_survives_an_80_column_terminal() {
        use htui_core::model::Status;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::{Paragraph, Widget as _};

        let theme = Theme::default();
        let mut detail = r_ent_1().await;
        for row in &mut detail.coverage {
            row.item.status = Status::AwaitingApproval;
        }
        let area = Rect::new(0, 0, 42, 40);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(lines(&detail, area.width, &theme)).render(area, &mut buffer);
        let drawn: Vec<String> = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        let suspect = drawn
            .iter()
            .find(|line| line.starts_with("ANA-1"))
            .unwrap_or_else(|| panic!("ANA-1 is drawn: {drawn:#?}"));
        assert!(suspect.contains("! suspect"), "{suspect}");
    }

    /// MOD-60: the key column is measured in cells, so a wide key does not push its row's kind
    /// a cell right of the others'.
    #[tokio::test]
    async fn a_wide_key_keeps_the_kind_column() {
        use crate::ui::cells::cell_width;

        let theme = Theme::default();
        let mut detail = r_ent_1().await;
        assert!(detail.coverage.len() >= 2, "R-ENT-1 has two citing items");
        detail.coverage[0].item.key = "\u{6f22}\u{5b57}-1".to_owned();
        let rows = super::coverage(&detail.coverage, &theme);
        let offsets: Vec<usize> = rows
            .iter()
            .zip(&detail.coverage)
            .map(|(line, row)| {
                let drawn: String = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect();
                let at = drawn
                    .find(row.kind.as_str())
                    .unwrap_or_else(|| panic!("{drawn:?} shows its kind"));
                cell_width(&drawn[..at])
            })
            .collect();
        assert!(
            offsets.windows(2).all(|pair| pair[0] == pair[1]),
            "the kind starts at one cell offset on every row: {offsets:?}"
        );
    }

    /// Blueprint F-14: the mirror holds no revisions, and the pane says so.
    #[tokio::test]
    async fn revisions_none_says_they_need_the_database() {
        let theme = Theme::default();
        let mut detail = r_ent_1().await;
        detail.revisions = None;
        let drawn = lines(&detail, 53, &theme);
        let note = drawn
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content == REVISIONS_NEED_THE_DATABASE)
            })
            .expect("the note is drawn");
        assert_eq!(note.style, theme.dim);
        assert!(
            !text(&drawn).contains("by ANA-2"),
            "no revision row is drawn"
        );
    }
}
