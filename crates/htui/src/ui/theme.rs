//! The one place a colour is chosen.
//!
//! Views take the theme out of their [`Ctx`](crate::app::Ctx) instead of naming colours. Each
//! role is one meaning (MOD-80 D1): `accent` is focus only, `key` keys, `running` work in
//! progress, `warning` what needs a person, `added` a diff's `+`, `active_tab` the strip entry
//! you are on, `cursor` the text cursor, and `selected` a selected row, which
//! [`Theme::select`] paints over every span (D2).
//!
//! `NO_COLOR` set and non-empty selects [`Theme::monochrome`], the same roles drawn with
//! modifiers alone (D4); `run` applies it once at startup.

use std::ffi::OsStr;

use htui_core::model::Status;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;

/// Styles shared by every view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Ordinary text.
    pub base: Style,
    /// Secondary text: hints, empty states, the status line, separators. `Indexed(244)`, a grey
    /// at least 3.5:1 on light and dark profiles alike (MOD-80 D3).
    pub dim: Style,
    /// Block titles, group headers and the top bar's workspace: the terminal's own foreground,
    /// bold, so it reads on any profile (D3).
    pub title: Style,
    /// Focus: the row an overlay or form is on, a focused form label, the composer prompt, the
    /// Agents table highlight. Nothing else is cyan (D1).
    pub accent: Style,
    /// The selected row of a list. It replaces every cell's colours: a multi-span row goes
    /// through [`Theme::select`], so no span's own colour shows through (D2).
    pub selected: Style,
    /// Failure text (`StoreReply::Failed`, a rejected gate, a failed status).
    pub error: Style,
    /// Item and requirement keys, and the Graph sub-tab's node labels (D1).
    pub key: Style,
    /// Something in progress: an `in_progress` item, a running run or step or graph node (D1).
    pub running: Style,
    /// Something that needs a person: `awaiting_approval`, a degraded store, the waiting
    /// count (D1, D6).
    pub warning: Style,
    /// A diff's `+` lines (D1).
    pub added: Style,
    /// The active entry of a tab strip: main tabs, detail sub-tabs, Settings sections. Bold and
    /// underlined, so it shows without colour (D5).
    pub active_tab: Style,
    /// The one-cell text cursor in `TextField` / `TextArea`, and the flow view's cursor node
    /// border (D1).
    pub cursor: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let base = Style::new();
        let accent = Style::new().fg(Color::Cyan);
        Self {
            base,
            dim: Style::new().fg(Color::Indexed(244)),
            title: Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD),
            accent,
            selected: Style::new().fg(Color::Black).bg(Color::Cyan),
            error: Style::new().fg(Color::Red),
            key: base.add_modifier(Modifier::BOLD),
            running: Style::new().fg(Color::Blue),
            warning: Style::new().fg(Color::Yellow),
            added: Style::new().fg(Color::Green),
            active_tab: accent.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            cursor: Style::new().add_modifier(Modifier::REVERSED),
        }
    }
}

impl Theme {
    /// MOD-80 D4: no colour, only modifiers, for `NO_COLOR` terminals (crossterm 0.29 drops
    /// colour codes there and keeps attributes). `selected`/`cursor` reverse, `accent`/`key`/
    /// `title` are bold, `dim` is DIM, `active_tab` bold+underlined; `error`, `warning`,
    /// `running`, `added` are plain, because the status word or diff gutter already says what
    /// they mean.
    #[must_use]
    pub fn monochrome() -> Self {
        let plain = Style::new();
        let bold = plain.add_modifier(Modifier::BOLD);
        let reversed = plain.add_modifier(Modifier::REVERSED);
        Self {
            base: plain,
            dim: plain.add_modifier(Modifier::DIM),
            title: bold,
            accent: bold,
            selected: reversed,
            error: plain,
            key: bold,
            running: plain,
            warning: plain,
            added: plain,
            active_tab: plain.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            cursor: reversed,
        }
    }

    /// crossterm's `NO_COLOR` rule (`style/types/colored.rs` `ansi_color_disabled`): set and
    /// non-empty means [`Theme::monochrome`], anything else [`Theme::default`]. Takes the value,
    /// not the environment, so it is testable.
    #[must_use]
    pub fn from_no_color(value: Option<&OsStr>) -> Self {
        match value {
            Some(value) if !value.is_empty() => Self::monochrome(),
            _ => Self::default(),
        }
    }

    /// D2: `line` with [`Self::selected`] patched over the line **and over every span**, so a
    /// span's own `fg` cannot show through (a `Line::style` alone leaves a cyan key
    /// cyan-on-cyan). Modifiers a span had (a key's BOLD) survive.
    #[must_use]
    pub fn select<'a>(&self, line: Line<'a>) -> Line<'a> {
        let mut line = line.patch_style(self.selected);
        for span in &mut line.spans {
            span.style = span.style.patch(self.selected);
        }
        line
    }

    /// The style an [`Status`] renders with, so every list agrees on what "blocked" looks like.
    #[must_use]
    pub fn status_style(&self, status: Status) -> Style {
        match status {
            Status::Open | Status::Queued => self.base,
            Status::InProgress => self.running,
            Status::AwaitingApproval => self.warning,
            Status::Blocked | Status::Failed => self.error,
            Status::Done | Status::Closed => self.dim,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn the_default_palette_gives_each_role_its_colour() {
        let theme = Theme::default();
        assert_eq!(theme.dim.fg, Some(Color::Indexed(244)));
        assert_eq!(
            theme.title,
            Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD)
        );
        assert_eq!(theme.running.fg, Some(Color::Blue));
        assert_eq!(theme.warning.fg, Some(Color::Yellow));
        assert_eq!(theme.added.fg, Some(Color::Green));
        assert_eq!(theme.accent.fg, Some(Color::Cyan));
        assert_eq!(theme.key.fg, None, "a key keeps the base colour");
        assert!(theme.key.add_modifier.contains(Modifier::BOLD));
        assert_eq!(theme.active_tab.fg, theme.accent.fg);
        assert!(
            theme
                .active_tab
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert!(theme.cursor.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(
            theme.selected,
            Style::new().fg(Color::Black).bg(Color::Cyan)
        );
    }

    #[test]
    fn in_progress_is_running_and_awaiting_is_a_warning() {
        let theme = Theme::default();
        let cases = [
            (Status::Open, theme.base),
            (Status::Queued, theme.base),
            (Status::InProgress, theme.running),
            (Status::AwaitingApproval, theme.warning),
            (Status::Blocked, theme.error),
            (Status::Failed, theme.error),
            (Status::Done, theme.dim),
            (Status::Closed, theme.dim),
        ];
        for (status, style) in cases {
            assert_eq!(theme.status_style(status), style, "{status:?}");
        }
    }

    fn sample(theme: &Theme) -> Line<'static> {
        Line::from(vec![
            Span::styled("FEAT-1", theme.key),
            Span::styled("x", theme.accent),
            Span::styled("d", theme.dim),
        ])
    }

    #[test]
    fn select_paints_the_selection_over_every_span() {
        let theme = Theme {
            selected: Style::new().fg(Color::Black).bg(Color::Cyan),
            ..Theme::default()
        };
        let line = theme.select(sample(&theme));
        for span in &line.spans {
            assert_eq!(span.style.fg, Some(Color::Black), "{span:?}");
            assert_eq!(span.style.bg, Some(Color::Cyan), "{span:?}");
        }
        assert!(
            line.spans[0].style.add_modifier.contains(Modifier::BOLD),
            "the key stays bold"
        );
        assert_eq!(line.style.bg, Some(Color::Cyan));
    }

    #[test]
    fn select_under_monochrome_reverses_every_span() {
        let theme = Theme::monochrome();
        let line = theme.select(sample(&theme));
        for span in &line.spans {
            assert!(
                span.style.add_modifier.contains(Modifier::REVERSED),
                "{span:?}"
            );
            assert_eq!(span.style.fg, None, "{span:?}");
        }
    }

    #[test]
    fn no_color_set_and_non_empty_is_monochrome() {
        assert_eq!(Theme::from_no_color(None), Theme::default());
        assert_eq!(Theme::from_no_color(Some(OsStr::new(""))), Theme::default());
        assert_eq!(
            Theme::from_no_color(Some(OsStr::new("1"))),
            Theme::monochrome()
        );
        assert_eq!(
            Theme::from_no_color(Some(OsStr::new("0"))),
            Theme::monochrome()
        );
    }

    #[test]
    fn the_monochrome_theme_names_no_colour() {
        let theme = Theme::monochrome();
        let roles = [
            theme.base,
            theme.dim,
            theme.title,
            theme.accent,
            theme.selected,
            theme.error,
            theme.key,
            theme.running,
            theme.warning,
            theme.added,
            theme.active_tab,
            theme.cursor,
        ];
        for role in roles {
            assert_eq!(role.fg, None, "{role:?}");
            assert_eq!(role.bg, None, "{role:?}");
        }
        assert!(theme.selected.add_modifier.contains(Modifier::REVERSED));
        assert!(theme.cursor.add_modifier.contains(Modifier::REVERSED));
        assert!(
            theme
                .active_tab
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert!(theme.dim.add_modifier.contains(Modifier::DIM));
    }
}
