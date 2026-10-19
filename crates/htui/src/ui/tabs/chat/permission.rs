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

use crate::keys::{Act, Keys, Stack};
use crate::ui::Theme;
use crate::ui::tabs::chat::transcript::TranscriptRow;

/// The rows that answer a parked request, option by option (MOD-115 M2 D7): the strip labels
/// each option with its row's key, and the chat's parked hint lists them.
pub const ANSWERS: &[Act] = &[
    Act::AnswerOption1,
    Act::AnswerOption2,
    Act::AnswerOption3,
    Act::AnswerOption4,
    Act::AnswerOption5,
    Act::AnswerOption6,
    Act::AnswerOption7,
    Act::AnswerOption8,
    Act::AnswerOption9,
];

/// The one-line answer strip.
#[derive(Debug, Clone, Copy)]
pub struct PermissionStrip;

impl PermissionStrip {
    /// Draws `[1] Allow once  [2] Reject once` for the parked request, each option labelled with
    /// its `answer.option_*` key through `stack`.
    pub fn render(
        frame: &mut Frame<'_>,
        area: Rect,
        row: &TranscriptRow,
        keys: &Keys,
        stack: Stack<'_>,
        theme: &Theme,
    ) {
        let TranscriptRow::Permission { options, .. } = row else {
            return;
        };
        frame.render_widget(
            Paragraph::new(Self::line(options, keys, stack, theme)),
            area,
        );
    }

    /// The strip itself, `answer  [1] Allow once  [2] Reject once`, over any list of offered
    /// options: the chat's parked row draws it, and so does the Runs pane's relayed request
    /// (MOD-42 plan D14). Each option's key is its `answer.option_*` row's first chord through
    /// `stack` (MOD-115 M2 T5); an option whose row is unbound shows no key.
    #[must_use]
    pub fn line(
        options: &[PermissionOption],
        keys: &Keys,
        stack: Stack<'_>,
        theme: &Theme,
    ) -> Line<'static> {
        let mut spans = vec![Span::styled("answer  ", theme.title)];
        for (index, option) in options.iter().enumerate() {
            if let Some(label) = ANSWERS.get(index).and_then(|&act| keys.label(stack, act)) {
                spans.push(Span::styled(format!("[{label}] "), theme.accent));
            }
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
        Line::from(spans)
    }

    /// The option a digit picks, 1-based, or `None` when the row does not offer one.
    #[must_use]
    pub fn pick(row: &TranscriptRow, digit: char) -> Option<PermissionOption> {
        let TranscriptRow::Permission { options, .. } = row else {
            return None;
        };
        Self::pick_from(options, digit)
    }

    /// The option a digit picks among `options`, 1-based, or `None` past the last one and for `0`
    /// (MOD-42 plan D14: the Runs pane picks from a relayed request's options).
    #[must_use]
    pub fn pick_from(options: &[PermissionOption], digit: char) -> Option<PermissionOption> {
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
    use crate::keys::{Keys, load_str, views};
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

    fn strip_text(keys: &Keys) -> String {
        let TranscriptRow::Permission { options, .. } = row() else {
            unreachable!("row() is a permission");
        };
        PermissionStrip::line(&options, keys, views::CHAT_LIVE, &Theme::default())
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// T5 repair: each option is labelled with its `answer.option_*` key in force, and an option
    /// whose key is unbound shows no key.
    #[test]
    fn each_option_is_labelled_with_its_answer_key() {
        assert_eq!(
            strip_text(Keys::compiled()),
            "answer  [1] Allow once  [2] Reject  "
        );
        let keys = load_str("version = 2\n[answer]\noption_1 = \"f8\"\noption_2 = []\n")
            .expect("the keys load");
        assert_eq!(strip_text(&keys), "answer  [F8] Allow once  Reject  ");
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
