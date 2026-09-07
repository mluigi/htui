//! The one line the user types into (`R-TUI-6`).
//!
//! A mode, not a widget with focus: while it is active every printable key is text, which is what
//! lets the tab's own bindings (`i`, `t`, digits) stay single letters everywhere else.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Theme;
use crossterm::event::{KeyCode, KeyEvent};

/// What a key did to the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutcome {
    /// The composer used the key.
    Consumed,
    /// The user pressed Enter on this text; the composer is now empty and inactive.
    Submit(String),
    /// The user pressed Esc.
    Leave,
    /// The composer is not active and did not want the key.
    Pass,
}

/// A single-line text buffer.
#[derive(Debug, Default)]
pub struct Composer {
    text: String,
    active: bool,
}

impl Composer {
    /// Whether keys are going into the buffer.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// What has been typed so far.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Starts composing.
    pub const fn enter(&mut self) {
        self.active = true;
    }

    /// Stops composing and forgets what was typed.
    pub fn leave(&mut self) {
        self.active = false;
        self.text.clear();
    }

    /// Feeds one key.
    pub fn on_key(&mut self, key: KeyEvent) -> ComposerOutcome {
        if !self.active {
            return ComposerOutcome::Pass;
        }
        match key.code {
            KeyCode::Char(ch) => {
                self.text.push(ch);
                ComposerOutcome::Consumed
            }
            KeyCode::Backspace => {
                self.text.pop();
                ComposerOutcome::Consumed
            }
            // An empty Enter is not a turn: it would send a prompt with no text, which every
            // transport refuses anyway.
            KeyCode::Enter if !self.text.trim().is_empty() => {
                let text = std::mem::take(&mut self.text);
                self.active = false;
                ComposerOutcome::Submit(text)
            }
            KeyCode::Enter => ComposerOutcome::Consumed,
            KeyCode::Esc => {
                self.leave();
                ComposerOutcome::Leave
            }
            _ => ComposerOutcome::Consumed,
        }
    }

    /// Draws the input line, or the hint that says how to open it.
    pub fn render(frame: &mut Frame<'_>, area: Rect, composer: &Self, hint: &str, theme: &Theme) {
        let line = if composer.active {
            Line::from(vec![
                Span::styled("> ", theme.accent),
                Span::styled(composer.text.clone(), theme.base),
                Span::styled("_", theme.accent),
            ])
        } else {
            Line::styled(hint.to_owned(), theme.dim)
        };
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn an_inactive_composer_passes_every_key_through() {
        let mut composer = Composer::default();
        assert_eq!(
            composer.on_key(key(KeyCode::Char('j'))),
            ComposerOutcome::Pass,
            "`j` scrolls the transcript until the composer is open"
        );
    }

    #[test]
    fn typing_then_enter_submits_and_closes() {
        let mut composer = Composer::default();
        composer.enter();
        for ch in "hi".chars() {
            assert_eq!(
                composer.on_key(key(KeyCode::Char(ch))),
                ComposerOutcome::Consumed
            );
        }
        assert_eq!(composer.text(), "hi");
        assert_eq!(
            composer.on_key(key(KeyCode::Enter)),
            ComposerOutcome::Submit("hi".to_owned())
        );
        assert!(!composer.is_active(), "submitting closes the composer");
        assert!(composer.text().is_empty());
    }

    #[test]
    fn backspace_edits_and_an_empty_enter_sends_nothing() {
        let mut composer = Composer::default();
        composer.enter();
        composer.on_key(key(KeyCode::Char('a')));
        composer.on_key(key(KeyCode::Backspace));
        assert_eq!(composer.text(), "");
        assert_eq!(
            composer.on_key(key(KeyCode::Enter)),
            ComposerOutcome::Consumed,
            "an empty turn is not a turn"
        );
        assert!(composer.is_active());
    }

    #[test]
    fn esc_leaves_and_forgets() {
        let mut composer = Composer::default();
        composer.enter();
        composer.on_key(key(KeyCode::Char('x')));
        assert_eq!(composer.on_key(key(KeyCode::Esc)), ComposerOutcome::Leave);
        assert!(!composer.is_active());
        assert!(composer.text().is_empty());
    }
}
