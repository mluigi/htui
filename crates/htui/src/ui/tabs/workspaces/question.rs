//! The Workspaces tab's stop and resume questions (MOD-116 M2 D6) and the run lines its overview's
//! **Runs** section shares (D5). Pure: a [`Report`] in, lines out, so the tab's render only draws
//! them in a kit `Ask` and the overview only appends them.

use htui_core::model::{RunStatus, StopMode};
use ratatui::text::{Line, Span};

use crate::stop_report::{Report, RunLine};
use crate::ui::Theme;
use crate::ui::kit::text::wrapped;

/// The indent of a run's fate under its line.
const FATE_INDENT: &str = "  ";

/// The graceful question's first line (D6), the CLI's question's words.
pub(super) const GRACEFUL_FIRST: &str = "Each agent step stops at its turn boundary, checks stop at \
                                         once; nothing new starts until it is resumed.";

/// The forced question's first line (D6).
pub(super) const FORCED_FIRST: &str =
    "Every step still running is killed mid-turn and redoes its turn on resume.";

/// The resume question's first line (D6), the CLI's.
pub(super) const RESUME_FIRST: &str =
    "Its stopped runs resume on the boxes that ran them and its queued runs can start.";

/// A told box's title when `s` asked.
pub(super) const STOP_TOLD_TITLE: &str = "Stop runs";

/// A told box's title when `r` asked.
pub(super) const RESUME_TOLD_TITLE: &str = "Resume runs";

/// The graceful question's title.
pub(super) fn stop_title(name: &str) -> String {
    format!("Stop the runs in {name}?")
}

/// The forced question's title.
pub(super) fn force_title(name: &str) -> String {
    format!("Stop the runs in {name} now?")
}

/// The resume question's title.
pub(super) fn resume_title(name: &str) -> String {
    format!("Resume {name}?")
}

/// `s` on a workspace with no stop row and no live run.
pub(super) fn nothing_running(name: &str) -> String {
    format!("Nothing is running in {name}")
}

/// `s` on a workspace whose graceful stop has no running run left.
pub(super) fn settled(name: &str) -> String {
    format!("Nothing is running in {name}: its graceful stop has settled")
}

/// `s` on a forced stop, or a write that found the mode already there.
pub(super) fn already_stopped(name: &str, mode: StopMode) -> String {
    format!("{name} is already stopped ({mode})")
}

/// `r` on a workspace with no stop row, or a resume that found none.
pub(super) fn not_stopped(name: &str) -> String {
    format!("{name} is not stopped")
}

/// A write refused because the stop row moved since the question was built (D3); `key` is the
/// chord `workspaces.stop` (or `workspaces.resume`) is bound to, `None` when it is unbound.
pub(super) fn changed(name: &str, stop: bool, key: Option<&str>) -> String {
    let done = if stop { "stopped" } else { "resumed" };
    let again = key.map_or_else(
        || "Ask again.".to_owned(),
        |key| format!("Press {key} again."),
    );
    format!("The stop of {name} changed while asking; nothing was {done}. {again}")
}

/// A refused read or write: `Stopping needs the database: …` or `Resuming …`.
pub(super) fn needs_the_database(stop: bool, message: &str) -> String {
    let what = if stop { "Stopping" } else { "Resuming" };
    format!("{what} needs the database: {message}")
}

/// Whether `line`'s run is live: `queued`, `running` or `awaiting_approval`.
pub(super) fn is_live(line: &RunLine) -> bool {
    matches!(
        line.status,
        RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval
    )
}

/// A run's `<item>  <status_words>` line, then each of `fate` indented under it, wrapped in
/// `width` with its further rows indented the same.
pub(super) fn run_lines(
    line: &RunLine,
    fate: &[String],
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(line.item.clone(), theme.base),
        Span::styled("  ", theme.base),
        Span::styled(line.status_words(), theme.dim),
    ])];
    let room = width.saturating_sub(FATE_INDENT.len()).max(1);
    for words in fate {
        lines.extend(
            wrapped(words, room)
                .into_iter()
                .map(|row| Line::from(vec![Span::raw(FATE_INDENT), Span::styled(row, theme.base)])),
        );
    }
    lines
}

/// D6: the lines a stop of `mode` lists: graceful, every live run with what the stop does to it
/// (a held run's resume words); forced, the running runs with what killing them does. `width` is
/// the box's content width.
pub(super) fn stop_lines(
    report: &Report,
    mode: StopMode,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    report
        .lines
        .iter()
        .filter(|line| match mode {
            StopMode::Graceful => is_live(line),
            StopMode::Forced => line.status == RunStatus::Running,
        })
        .flat_map(|line| run_lines(line, &line.under(mode), width, theme))
        .collect()
}

/// D6: the lines a resume lists: every run with how it resumes. `width` is the box's content
/// width.
pub(super) fn resume_lines(report: &Report, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    report
        .lines
        .iter()
        .flat_map(|line| run_lines(line, &[line.resume.words(true)], width, theme))
        .collect()
}

/// D5: what the overview's **Runs** section says under `line`: a running run under a stop, what
/// the stop does to each of its steps; a running run with none, nothing; any other, how it
/// resumes (`stopped`: the workspace has a stop row).
pub(super) fn fate_in_overview(line: &RunLine, stop: Option<StopMode>) -> Vec<String> {
    match (line.status, stop) {
        (RunStatus::Running, Some(mode)) => line.under(mode),
        (RunStatus::Running, None) => Vec::new(),
        _ => vec![line.resume.words(stop.is_some())],
    }
}

#[cfg(test)]
mod tests {
    use htui_core::model::{RunId, WorkspaceStopState};

    use super::*;
    use crate::stop_report::{LiveStep, RunResume};

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

    fn line(item: &str, status: RunStatus, resume: RunResume, steps: Vec<LiveStep>) -> RunLine {
        RunLine {
            item: item.to_owned(),
            run: RunId::new(),
            status,
            on_box: Some("demo-box".to_owned()),
            resume,
            steps,
        }
    }

    fn report() -> Report {
        Report {
            slug: "platform".to_owned(),
            state: WorkspaceStopState::NotStopped,
            stop: None,
            lines: vec![
                line("HT-1", RunStatus::Queued, RunResume::Held, Vec::new()),
                line(
                    "HT-2",
                    RunStatus::Running,
                    RunResume::Stopping,
                    vec![LiveStep {
                        phase: "implement".to_owned(),
                        agent: Some("claude".to_owned()),
                        pauses: true,
                    }],
                ),
                line("HT-3", RunStatus::Stopped, RunResume::Continues, Vec::new()),
            ],
        }
    }

    #[test]
    fn a_graceful_stop_lists_the_live_runs_and_a_forced_one_the_running_ones() {
        let theme = Theme::default();
        assert_eq!(
            text(&stop_lines(&report(), StopMode::Graceful, 80, &theme)),
            [
                "HT-1  queued",
                "  starts when the workspace is resumed",
                "HT-2  running on demo-box",
                "  implement · claude: stops at its turn boundary; its session is kept",
            ]
        );
        assert_eq!(
            text(&stop_lines(&report(), StopMode::Forced, 80, &theme)),
            [
                "HT-2  running on demo-box",
                "  implement · claude: stops now, mid-turn; redoes that turn on resume",
            ]
        );
    }

    #[test]
    fn a_resume_lists_every_run_with_how_it_resumes() {
        assert_eq!(
            text(&resume_lines(&report(), 80, &Theme::default())),
            [
                "HT-1  queued",
                "  starts when the workspace is resumed",
                "HT-2  running on demo-box",
                "  stopping",
                "HT-3  stopped on demo-box",
                "  resumes: continues from where it stopped",
            ]
        );
    }

    #[test]
    fn a_long_fate_wraps_under_its_indent() {
        let theme = Theme::default();
        let report = report();
        assert_eq!(
            text(&run_lines(
                &report.lines[0],
                &["one two three four".to_owned()],
                12,
                &theme
            )),
            ["HT-1  queued", "  one two", "  three four"]
        );
    }

    #[test]
    fn the_told_texts_name_the_workspace() {
        assert_eq!(
            changed("Platform", true, Some("s")),
            "The stop of Platform changed while asking; nothing was stopped. Press s again."
        );
        assert_eq!(
            changed("Platform", false, Some("r")),
            "The stop of Platform changed while asking; nothing was resumed. Press r again."
        );
        assert_eq!(
            already_stopped("Platform", StopMode::Forced),
            "Platform is already stopped (forced)"
        );
        assert_eq!(
            needs_the_database(false, "offline"),
            "Resuming needs the database: offline"
        );
    }
}
