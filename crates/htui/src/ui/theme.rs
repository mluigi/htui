//! The one place a colour is chosen.
//!
//! Views take the theme out of their [`Ctx`](crate::app::Ctx) instead of naming colours, so a
//! future palette (MOD-12 settings) changes one file and every tab follows.

use htui_core::model::Status;
use ratatui::style::{Color, Modifier, Style};

/// Styles shared by every view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Ordinary text.
    pub base: Style,
    /// Secondary text: hints, empty states, the status line.
    pub dim: Style,
    /// Block titles and the top bar.
    pub title: Style,
    /// The active tab, the selected row's key, anything the eye should land on first.
    pub accent: Style,
    /// The selected row of a list.
    pub selected: Style,
    /// Failure text (`StoreReply::Failed`, a rejected gate).
    pub error: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            base: Style::new(),
            dim: Style::new().fg(Color::DarkGray),
            title: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            accent: Style::new().fg(Color::Cyan),
            selected: Style::new().add_modifier(Modifier::REVERSED),
            error: Style::new().fg(Color::Red),
        }
    }
}

impl Theme {
    /// The style an [`Status`] renders with, so every list agrees on what "blocked" looks like.
    #[must_use]
    pub fn status_style(&self, status: Status) -> Style {
        match status {
            Status::Open | Status::Queued => self.base,
            Status::InProgress => Style::new().fg(Color::Cyan),
            Status::AwaitingApproval => Style::new().fg(Color::Yellow),
            Status::Blocked | Status::Failed => self.error,
            Status::Done | Status::Closed => self.dim,
        }
    }
}
