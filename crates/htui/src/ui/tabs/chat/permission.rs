//! The inline permission answer (`R-TUI-6`, `docs/ANA-4.md` §4.3 stage 3).
//!
//! Stage 3 is the only one the user sees: stages 1 and 2 are decided in the worker, next to the
//! recorder, and never reach the screen. What is rendered is exactly what the agent offered — the
//! option **ids** are the agent's and are sent back unchanged, and the `kind` is a hint this strip
//! renders but never acts on.

use htui_agent::event::{PermissionOption, PermissionOptionKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Theme;
use crate::ui::tabs::chat::transcript::TranscriptRow;

/// The one-line answer strip.
#[derive(Debug, Clone, Copy)]
pub struct PermissionStrip;

impl PermissionStrip {
    /// Draws `[1] Allow once  [2] Reject once` for the parked request.
    pub fn render(frame: &mut Frame<'_>, area: Rect, row: &TranscriptRow, theme: &Theme) {
        let TranscriptRow::Permission { options, .. } = row else {
            return;
        };
        let mut spans = vec![Span::styled("answer  ", theme.title)];
        for (index, option) in options.iter().enumerate() {
            spans.push(Span::styled(format!("[{}] ", index + 1), theme.accent));
            spans.push(Span::styled(option.label.clone(), theme.base));
            // `htui` forwards an `_always` choice so the **agent** remembers it; `htui`'s own
            // `agent.settings.permission.remembered[]` has no editor yet (plan D21), and saying so
            // is better than implying a durable grant this build does not write.
            if matches!(
                option.kind,
                PermissionOptionKind::AllowAlways | PermissionOptionKind::RejectAlways
            ) {
                spans.push(Span::styled(" (agent remembers)", theme.dim));
            }
            spans.push(Span::raw("  "));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The option a digit picks, 1-based, or `None` when the row does not offer one.
    #[must_use]
    pub fn pick(row: &TranscriptRow, digit: char) -> Option<PermissionOption> {
        let TranscriptRow::Permission { options, .. } = row else {
            return None;
        };
        let index = digit.to_digit(10)?;
        if index == 0 {
            return None;
        }
        options.get(index as usize - 1).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_agent::driver::PermissionRequestId;

    fn row() -> TranscriptRow {
        TranscriptRow::Permission {
            request_id: PermissionRequestId::new("req-1"),
            tool_call_id: None,
            options: vec![
                PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow once".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                },
                PermissionOption {
                    id: "reject".to_owned(),
                    label: "Reject".to_owned(),
                    kind: PermissionOptionKind::RejectOnce,
                },
            ],
            resolved: None,
        }
    }

    #[test]
    fn a_digit_picks_the_option_at_that_position() {
        assert_eq!(
            PermissionStrip::pick(&row(), '1').map(|option| option.id),
            Some("allow".to_owned())
        );
        assert_eq!(
            PermissionStrip::pick(&row(), '2').map(|option| option.id),
            Some("reject".to_owned())
        );
    }

    #[test]
    fn a_digit_past_the_options_picks_nothing() {
        assert!(PermissionStrip::pick(&row(), '3').is_none());
        assert!(
            PermissionStrip::pick(&row(), '0').is_none(),
            "the list is 1-based, so `0` is not an option"
        );
    }

    #[test]
    fn a_row_that_is_not_a_permission_offers_nothing() {
        let done = TranscriptRow::Done {
            stop_reason: htui_agent::event::StopReason::EndTurn,
        };
        assert!(PermissionStrip::pick(&done, '1').is_none());
    }
}
