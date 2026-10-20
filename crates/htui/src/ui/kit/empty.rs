//! The empty state: what is missing and always a next step (MOD-115), and the placeholder pane
//! that frames one where a pane has nothing to show yet (M2's activity pane).

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::ui::Theme;
use crate::ui::cells::clip_line;
use crate::ui::kit::pane::Pane;
use crate::ui::layout::centered;

/// The next step an empty state points at. Required, so no empty state is a dead end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextStep<'a>(pub &'a str);

/// The lines of an empty state for `what`: what is missing in the dim role, then the next step
/// in the focus role. Always two lines, so the next step is never optional.
#[must_use]
pub fn lines(what: &str, next: NextStep<'_>, theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::styled(what.to_owned(), theme.dim),
        Line::styled(next.0.to_owned(), theme.accent),
    ]
}

/// Draws an empty state centred in `area`. Each line is clipped to the width first, so a long
/// sentence loses its tail and not its alignment.
pub fn render(frame: &mut Frame<'_>, area: Rect, what: &str, next: NextStep<'_>, theme: &Theme) {
    let lines: Vec<Line<'static>> = lines(what, next, theme)
        .into_iter()
        .map(|line| clip_line(line, area.width).alignment(Alignment::Center))
        .collect();
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let target = centered(area, area.width, height);
    frame.render_widget(Paragraph::new(lines), target);
}

/// Draws the empty state of a pane nothing fills yet into `area` (MOD-115 M2 D2: the activity
/// pane until MOD-122): a rounded block in the faint border role titled ` {title} `, with the
/// empty state for `what` and `next` centred inside. The shell restyles its border for focus
/// afterwards, like any pane's.
pub fn placeholder(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    what: &str,
    next: NextStep<'_>,
    theme: &Theme,
) {
    let inner = Pane::new(title).render(frame, area, theme);
    render(frame, inner, what, next, theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};

    fn draw(width: u16, height: u16, what: &str, next: &str, theme: &Theme) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), what, NextStep(next), theme))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn an_empty_state_always_carries_a_next_step() {
        let theme = Theme::default();
        let lines = lines("no items yet", NextStep("press n to add one"), &theme);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "no items yet");
        assert_eq!(lines[1].to_string(), "press n to add one");
    }

    #[test]
    fn what_is_missing_is_secondary_and_the_next_step_is_the_focus_role() {
        let theme = Theme::default();
        let lines = lines("nothing here", NextStep("do this"), &theme);
        assert_eq!(lines[0].style, theme.dim);
        assert_eq!(lines[1].style, theme.accent);
    }

    #[test]
    fn the_next_step_stands_out_without_colour() {
        let theme = Theme::monochrome();
        let lines = lines("nothing here", NextStep("do this"), &theme);
        for line in &lines {
            assert_eq!((line.style.fg, line.style.bg), (None, None));
        }
        assert!(lines[0].style.add_modifier.contains(Modifier::DIM));
        assert!(lines[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_rendered_state_shows_the_next_step_centred() {
        let rows = draw(20, 5, "no items", "press n", &Theme::default());
        assert_eq!(rows[2].trim(), "no items");
        assert_eq!(rows[3].trim(), "press n");
        // Centred: ratatui rounds an odd slack either way, so allow one cell.
        let start = |row: &str| row.len() - row.trim_start().len();
        for (row, text) in [(&rows[2], "no items"), (&rows[3], "press n")] {
            let slack = 20 - text.len();
            assert!(
                start(row).abs_diff(slack / 2) <= 1,
                "{text:?} starts at {}",
                start(row)
            );
        }
    }

    #[test]
    fn in_a_two_row_area_both_lines_still_show() {
        let rows = draw(20, 2, "no items", "press n", &Theme::default());
        assert_eq!(rows[0].trim(), "no items");
        assert_eq!(rows[1].trim(), "press n");
    }

    #[test]
    fn a_one_row_area_keeps_what_and_loses_only_the_overflow() {
        let rows = draw(20, 1, "no items", "press n", &Theme::default());
        assert_eq!(rows[0].trim(), "no items");
    }

    #[test]
    fn a_line_wider_than_the_area_is_clipped_not_wrapped() {
        let rows = draw(
            10,
            4,
            "a very long sentence indeed",
            "go",
            &Theme::default(),
        );
        let shown: Vec<&str> = rows.iter().map(|row| row.trim()).collect();
        assert!(shown.contains(&"go"), "{shown:?}");
        assert!(
            shown.iter().filter(|row| row.contains("indeed")).count() == 0,
            "the tail is clipped: {shown:?}"
        );
    }

    #[test]
    fn a_zero_sized_area_draws_nothing_and_does_not_panic() {
        let mut terminal = Terminal::new(TestBackend::new(5, 5)).expect("terminal");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    Rect::new(0, 0, 0, 0),
                    "x",
                    NextStep("y"),
                    &Theme::default(),
                );
            })
            .expect("draw");
    }

    fn draw_placeholder(width: u16, height: u16, theme: &Theme) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                placeholder(
                    frame,
                    frame.area(),
                    "Activity",
                    "no activity yet",
                    NextStep("it lands with MOD-122"),
                    theme,
                );
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_placeholder_is_a_rounded_faint_pane_titled_with_the_state_inside() {
        let theme = Theme::default();
        let buffer = draw_placeholder(30, 6, &theme);
        let top = text(&buffer, 0);
        assert!(top.starts_with("╭─ Activity ─"), "{top}");
        assert!(top.ends_with('╮'), "{top}");
        assert!(text(&buffer, 5).starts_with('╰') && text(&buffer, 5).ends_with('╯'));
        for (x, y) in [(0, 0), (29, 0), (0, 3), (29, 5)] {
            assert_eq!(buffer[(x, y)].fg, Color::Indexed(240), "({x},{y})");
        }
        let inner: Vec<String> = (1..5).map(|y| text(&buffer, y)).collect();
        assert!(
            inner
                .iter()
                .any(|row| row.trim_matches(['│', ' ']) == "no activity yet"),
            "{inner:?}"
        );
        assert!(
            inner
                .iter()
                .any(|row| row.trim_matches(['│', ' ']) == "it lands with MOD-122"),
            "{inner:?}"
        );
        let what = inner
            .iter()
            .position(|row| row.contains("no activity yet"))
            .unwrap();
        assert_eq!(what, 1, "centred in the four inner rows: {inner:?}");
    }

    #[test]
    fn the_placeholder_border_is_dim_in_monochrome() {
        let theme = Theme::monochrome();
        let buffer = draw_placeholder(30, 6, &theme);
        for (x, y) in [(0, 0), (29, 0), (0, 3), (29, 5)] {
            let cell = &buffer[(x, y)];
            assert!(cell.modifier.contains(Modifier::DIM), "({x},{y})");
            assert_eq!(
                (cell.fg, cell.bg),
                (Color::Reset, Color::Reset),
                "({x},{y})"
            );
        }
    }

    #[test]
    fn a_tiny_placeholder_does_not_panic() {
        for (width, height) in [(1, 1), (2, 2), (5, 2), (12, 3)] {
            let buffer = draw_placeholder(width, height, &Theme::default());
            assert_eq!(buffer.area.width, width);
        }
        let mut terminal = Terminal::new(TestBackend::new(5, 5)).expect("terminal");
        terminal
            .draw(|frame| {
                placeholder(
                    frame,
                    Rect::default(),
                    "Activity",
                    "x",
                    NextStep("y"),
                    &Theme::default(),
                );
            })
            .expect("draw");
    }
}
