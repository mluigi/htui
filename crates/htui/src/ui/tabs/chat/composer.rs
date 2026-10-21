//! The one line the user types into (`R-TUI-6`).
//!
//! A mode, not a widget with focus: while it is active every printable key is text, which is what
//! lets the tab's own bindings (`i`, `t`, digits) stay single letters everywhere else. The tab
//! offers it only what `CAPTURE` does not pass, so CONTROL, ALT and function keys reach the shell
//! (MOD-67 M5).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
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

    /// A bracketed paste (MOD-22 review M-1), appended as typing it would be — the composer is one
    /// line, so each line break becomes one space and every other control character is dropped.
    /// `false` while the composer is closed: then it is not its to take.
    pub fn on_paste(&mut self, text: &str) -> bool {
        if !self.active {
            return false;
        }
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    chars.next_if_eq(&'\n');
                    self.text.push(' ');
                }
                '\n' => self.text.push(' '),
                c if c.is_control() => {}
                c => self.text.push(c),
            }
        }
        true
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

    /// Draws the input line while the composer is open; nothing otherwise (the keys that open it
    /// are the pane's hints since MOD-67 M6 T2). The prompt `> ` and the cursor `_` take
    /// `accent`, the tab's focus accent (MOD-115 M3 D-17, MOD-85); the text is plain.
    pub fn render(
        frame: &mut Frame<'_>,
        area: Rect,
        composer: &Self,
        accent: Style,
        theme: &Theme,
    ) {
        if !composer.active {
            return;
        }
        let line = Line::from(vec![
            Span::styled("> ", accent),
            Span::styled(composer.text.clone(), theme.base),
            Span::styled("_", accent),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// MOD-22 review M-1: an open composer takes a paste on its one line, breaks as spaces; a
    /// closed one takes nothing.
    #[test]
    fn a_paste_lands_in_an_open_composer_only() {
        let mut composer = Composer::default();
        assert!(!composer.on_paste("ignored"));
        assert_eq!(composer.text(), "");

        composer.enter();
        assert!(composer.on_paste("one\r\ntwo\nthree\u{7}"));
        assert_eq!(composer.text(), "one two three");
        assert!(composer.is_active(), "a pasted newline is not an Enter");
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

    /// MOD-115 M3 D-17, MOD-85: the prompt `> ` and the cursor `_` take the accent the tab
    /// passes (its focus accent, the one focus use left in the Chat); the text is plain.
    #[test]
    fn the_prompt_marks_take_the_accent_passed_in() {
        use ratatui::style::{Color, Style};
        let theme = Theme::default();
        let accent = Style::new().fg(Color::Red);
        let mut composer = Composer::default();
        composer.enter();
        composer.on_key(key(KeyCode::Char('h')));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(10, 1)).expect("terminal");
        terminal
            .draw(|frame| Composer::render(frame, frame.area(), &composer, accent, &theme))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        let cell = |x: u16| (buffer[(x, 0)].symbol().to_owned(), buffer[(x, 0)].fg);
        assert_eq!(cell(0), (">".to_owned(), Color::Red));
        assert_eq!(cell(3), ("_".to_owned(), Color::Red));
        assert_eq!(buffer[(2, 0)].symbol(), "h");
        assert_eq!(buffer[(2, 0)].fg, theme.base.fg.unwrap_or(Color::Reset));
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
