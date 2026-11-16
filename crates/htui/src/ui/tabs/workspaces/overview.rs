//! The Workspaces tab's detail lines (MOD-116 M1 D8): a workspace's overview, or one of its
//! projects', built from one [`WorkspaceOverview`] reply. Pure: lines in, no frame, so the tab's
//! render only scrolls and draws them. A value too long for its row (a path, the inherited
//! target's sentence) wraps under itself rather than losing its tail at the border.
//!
//! MOD-116 M2: the open workspace's state carries its stop (D7), and a workspace's overview lists
//! its runs and what a stop or a resume does to each (D5).

use htui_core::model::{WorkspaceLifecycle, WorkspaceRunCounts};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::hierarchy::{ProjectEntry, RepoEntry};
use crate::stop_report::Report;
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::kit::label::{self, plural};
use crate::ui::kit::text::wrapped;
use crate::workspaces::{ProjectOverview, WorkspaceOverview};

use super::question::{fate_in_overview, run_lines};

/// The label column of a `label  value` line: the widest label, `folder on this box`, and two.
const LABEL: usize = 20;

/// The indent of a project's counts under its name.
const INDENT: usize = 2;

/// Where a repo is not checked out on this box.
const NOT_ON_THIS_BOX: &str = "not on this box";

/// A project with no repo.
const NO_REPOS: &str = "no repos yet";

/// The mark and its counts: `● open · 2 running`, the counts in the dim role. MOD-116 M2 D7: an
/// open workspace whose `counts` row has a stop row says so after them, dim: ` · stopping…` while
/// a run is running, else ` · stopped` and ` · <n> held` when a run is held (`● open · stopped ·
/// 3 held`). `counts: None` (offline, or no row read) draws no annotation.
pub fn state_spans(
    state: WorkspaceLifecycle,
    counts: Option<&WorkspaceRunCounts>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut spans = label::spans(label::workspace_lifecycle(state), theme);
    let words = state.to_string();
    let words = words.strip_prefix(state.word()).unwrap_or_default();
    if !words.is_empty() {
        spans.push(Span::styled(words.to_owned(), theme.dim));
    }
    if let (WorkspaceLifecycle::Open { .. }, Some(row)) = (state, counts)
        && row.stop.is_some()
    {
        let annotation = if row.running > 0 {
            " \u{b7} stopping\u{2026}".to_owned()
        } else if row.held() > 0 {
            format!(" \u{b7} stopped \u{b7} {} held", row.held())
        } else {
            " \u{b7} stopped".to_owned()
        };
        spans.push(Span::styled(annotation, theme.dim));
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
/// runs, (MOD-116 M2 D5) the runs, the description, the goals and each project with its counts
/// and where its primary repo is. `width` is the pane's inner width.
///
/// MOD-116 M2 D7: `current` is whether the overview's counts and report are current (online).
/// When they are not, `state` is the one the caller still knows (the open workspace's `open`, or
/// `None`: no state line at all), the state carries no stop annotation and the **Runs** section,
/// whose fates are the last read's, is left out.
pub fn workspace_lines(
    overview: &WorkspaceOverview,
    state: Option<WorkspaceLifecycle>,
    current: bool,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(state) = state {
        let counts = current.then_some(&overview.counts);
        lines.push(Line::from(state_spans(state, counts, theme)));
        lines.push(Line::default());
    }
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
    if current {
        lines.push(Line::default());
        lines.extend(runs_lines(&overview.report, width, theme));
    }

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

/// MOD-116 M2 D5: the **Runs** section: each run that is not finished, `<item>  <status>`, and
/// under it what the workspace's stop does to it, or how it resumes (`no runs` without one).
pub(super) fn runs_lines(report: &Report, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![heading("Runs", theme)];
    if report.lines.is_empty() {
        lines.push(Line::styled("no runs", theme.dim));
    }
    let stop = report.stop.as_ref().map(|row| row.mode);
    for line in &report.lines {
        lines.extend(run_lines(line, &fate_in_overview(line, stop), width, theme));
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::fixtures::ids;
    use htui_core::model::{RunId, RunStatus, StopMode, WorkspaceStop, WorkspaceStopState};
    use htui_orch::promote::StepResume;

    use super::*;
    use crate::stop_report::{LiveStep, RunLine, RunResume};

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect()
    }

    fn spans_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn stop(mode: StopMode) -> WorkspaceStop {
        WorkspaceStop {
            workspace_id: ids::WORKSPACE_PLATFORM,
            mode,
            requested_by: ids::USER,
            requested_at: Utc::now(),
        }
    }

    fn counts(
        running: u32,
        queued: u32,
        stopped: u32,
        stop: Option<WorkspaceStop>,
    ) -> WorkspaceRunCounts {
        WorkspaceRunCounts {
            workspace_id: ids::WORKSPACE_PLATFORM,
            queued,
            running,
            awaiting_approval: 0,
            stopped,
            stop,
        }
    }

    fn line(
        item: &str,
        status: RunStatus,
        on_box: Option<&str>,
        resume: RunResume,
        steps: Vec<LiveStep>,
    ) -> RunLine {
        RunLine {
            item: item.to_owned(),
            run: RunId::new(),
            status,
            on_box: on_box.map(str::to_owned),
            resume,
            steps,
        }
    }

    /// The Runs section at a width nothing here wraps at.
    fn runs_lines_at(report: &Report, theme: &Theme) -> Vec<Line<'static>> {
        runs_lines(report, 200, theme)
    }

    fn report(stop: Option<WorkspaceStop>, lines: Vec<RunLine>) -> Report {
        Report {
            slug: "platform".to_owned(),
            state: WorkspaceStopState::NotStopped,
            stop,
            lines,
        }
    }

    /// D7: an open workspace under a stop says so after its counts; without a stop row, or
    /// with no counts row, it reads as M1.
    #[test]
    fn the_open_state_carries_its_stop() {
        let theme = Theme::default();
        let read = |row: &WorkspaceRunCounts| {
            spans_text(&state_spans(
                WorkspaceLifecycle::of(true, row),
                Some(row),
                &theme,
            ))
        };
        assert_eq!(
            read(&counts(2, 0, 0, Some(stop(StopMode::Graceful)))),
            "\u{25cf} open \u{b7} 2 running \u{b7} stopping\u{2026}"
        );
        assert_eq!(
            read(&counts(0, 2, 1, Some(stop(StopMode::Forced)))),
            "\u{25cf} open \u{b7} stopped \u{b7} 3 held"
        );
        assert_eq!(
            read(&counts(0, 0, 0, Some(stop(StopMode::Graceful)))),
            "\u{25cf} open \u{b7} stopped"
        );
        assert_eq!(
            read(&counts(2, 1, 0, None)),
            "\u{25cf} open \u{b7} 2 running"
        );
        let stopped = counts(0, 0, 1, Some(stop(StopMode::Graceful)));
        assert_eq!(
            spans_text(&state_spans(
                WorkspaceLifecycle::Open { running: 0 },
                None,
                &theme
            )),
            "\u{25cf} open"
        );
        assert_eq!(
            spans_text(&state_spans(
                WorkspaceLifecycle::of(false, &stopped),
                Some(&stopped),
                &theme
            )),
            "\u{25cb} stopped \u{b7} 1 held",
            "a workspace that is not open carries its stop in its state already"
        );
    }

    /// D5: every run that is not running says how it resumes, in the CLI's words.
    #[test]
    fn the_runs_section_says_how_each_run_resumes() {
        let theme = Theme::default();
        let lines = vec![
            line("HT-1", RunStatus::Queued, None, RunResume::Held, Vec::new()),
            line(
                "HT-2",
                RunStatus::Stopped,
                None,
                RunResume::NoBox,
                Vec::new(),
            ),
            line(
                "HT-3",
                RunStatus::Stopped,
                Some("demo-box"),
                RunResume::Steps(vec![
                    ("implement".to_owned(), StepResume::Rerun),
                    ("review".to_owned(), StepResume::FreshWithHandoff),
                ]),
                Vec::new(),
            ),
        ];
        assert_eq!(
            text(&runs_lines_at(
                &report(Some(stop(StopMode::Graceful)), lines.clone()),
                &theme
            )),
            [
                "Runs",
                "HT-1  queued",
                "  starts when the workspace is resumed",
                "HT-2  stopped, no box",
                "  cannot be resumed: cancel it from the Runs pane",
                "HT-3  stopped on demo-box",
                "  resumes: implement runs again from its start; review starts a fresh session \
                 with the handoff",
            ]
        );
        assert_eq!(
            text(&runs_lines_at(&report(None, lines[..1].to_vec()), &theme))[1..],
            ["HT-1  queued", "  waits for its box to start it"]
        );
        assert_eq!(
            text(&runs_lines_at(&report(None, Vec::new()), &theme)),
            ["Runs", "no runs"]
        );
    }

    /// D2/D5: under a stop, a running run's steps say when they stop, a pausing agent's at its
    /// turn boundary and the others' at once; forced, both now. A running run with no stop row
    /// says nothing under it.
    #[test]
    fn while_stopping_the_runs_say_when_they_stop() {
        let theme = Theme::default();
        let running = line(
            "HT-4",
            RunStatus::Running,
            Some("demo-box"),
            RunResume::Stopping,
            vec![
                LiveStep {
                    phase: "implement".to_owned(),
                    agent: Some("claude".to_owned()),
                    pauses: true,
                },
                LiveStep {
                    phase: "checks".to_owned(),
                    agent: None,
                    pauses: false,
                },
            ],
        );
        assert_eq!(
            text(&runs_lines_at(
                &report(Some(stop(StopMode::Graceful)), vec![running.clone()]),
                &theme
            )),
            [
                "Runs",
                "HT-4  running on demo-box",
                "  implement \u{b7} claude: stops at its turn boundary; its session is kept",
                "  checks: cannot pause between turns: stops at once; runs again on resume",
            ]
        );
        assert_eq!(
            text(&runs_lines_at(
                &report(Some(stop(StopMode::Forced)), vec![running.clone()]),
                &theme
            )),
            [
                "Runs",
                "HT-4  running on demo-box",
                "  implement \u{b7} claude: stops now, mid-turn; redoes that turn on resume",
                "  checks: stops now; runs again on resume",
            ]
        );
        assert_eq!(
            text(&runs_lines_at(&report(None, vec![running]), &theme)),
            ["Runs", "HT-4  running on demo-box"]
        );
    }
}
