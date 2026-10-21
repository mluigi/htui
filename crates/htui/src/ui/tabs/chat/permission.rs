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
use crate::ui::kit::chips::key_chip;
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
    /// Draws `answer   1  Allow once   2  Reject once` for the parked request, each option
    /// labelled with its `answer.option_*` key through `stack`.
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

    /// The strip itself, `answer   1  Allow once   2  Reject once`, over any list of offered
    /// options: the chat's parked row draws it, and so does the Runs pane's relayed request
    /// (MOD-42 plan D14). Each option's key is its `answer.option_*` row's first chord through
    /// `stack` (MOD-115 M2 T5), drawn as a kit key chip (M3 D5; the keys stay the digits until
    /// MOD-121); an option whose row is unbound shows no key. The label is plain text, so no part
    /// of the strip borrows the focus accent (MOD-85).
    #[must_use]
    pub fn line(
        options: &[PermissionOption],
        keys: &Keys,
        stack: Stack<'_>,
        theme: &Theme,
    ) -> Line<'static> {
        let mut spans = vec![Span::styled("answer  ", theme.title)];
        for (index, option) in options.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            if let Some(label) = ANSWERS.get(index).and_then(|&act| keys.label(stack, act)) {
                spans.push(key_chip(&label, theme));
                spans.push(Span::raw(" "));
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
    use crate::ui::kit::chips::key_chip;
    use htui_agent::driver::PermissionRequestId;
    use ratatui::style::Modifier;

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
    /// whose key is unbound shows no key. MOD-115 M3 D5: the key is a kit key chip.
    #[test]
    fn each_option_is_labelled_with_its_answer_key() {
        assert_eq!(
            strip_text(Keys::compiled()),
            "answer   1  Allow once   2  Reject"
        );
        let keys = load_str("version = 2\n[answer]\noption_1 = \"f8\"\noption_2 = []\n")
            .expect("the keys load");
        assert_eq!(strip_text(&keys), "answer   F8  Allow once  Reject");
    }

    /// MOD-115 M3 D5, MOD-85: the digits are key chips (the key role reversed, so a keycap
    /// without colour too), the option's label is plain text and the agent's memory note is
    /// secondary; nothing in the strip takes the focus accent.
    #[test]
    fn the_digits_are_key_chips_and_the_labels_plain() {
        let options = vec![
            PermissionOption {
                id: "allow".to_owned(),
                label: "Allow always".to_owned(),
                kind: PermissionOptionKind::AllowAlways,
            },
            PermissionOption {
                id: "reject".to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ];
        for theme in [Theme::default(), Theme::monochrome()] {
            let line = PermissionStrip::line(&options, Keys::compiled(), views::CHAT_LIVE, &theme);
            let style_of = |text: &str| {
                line.spans
                    .iter()
                    .find(|span| span.content == text)
                    .map(|span| span.style)
                    .unwrap_or_else(|| panic!("`{text}` is a span: {line:?}"))
            };
            assert_eq!(style_of("answer  "), theme.title);
            assert_eq!(style_of(" 1 "), key_chip("1", &theme).style, "a key chip");
            assert!(style_of(" 2 ").add_modifier.contains(Modifier::REVERSED));
            assert_eq!(style_of("Allow always"), theme.base);
            assert_eq!(style_of(" (agent remembers)"), theme.dim);
        }
        let theme = Theme::default();
        let line = PermissionStrip::line(&options, Keys::compiled(), views::CHAT_LIVE, &theme);
        assert!(
            line.spans.iter().all(|span| span.style != theme.accent),
            "no focus accent in the strip: {line:?}"
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
