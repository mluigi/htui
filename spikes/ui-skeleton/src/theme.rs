//! Colours. Three schemes so they can be compared live (`c`).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::data::Status;

pub const TEXT: Color = Color::Rgb(0xc8, 0xcc, 0xd4);
pub const DIM: Color = Color::Rgb(0x7a, 0x80, 0x8c);
pub const FAINT: Color = Color::Rgb(0x3e, 0x44, 0x51);
pub const SEL_BG: Color = Color::Rgb(0x2a, 0x30, 0x3b);
pub const NEUTRAL: Color = Color::Rgb(0x8a, 0xb4, 0xf8);
pub const INK: Color = Color::Rgb(0x16, 0x18, 0x1d);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    /// Each tab has its accent, and item keys are coloured by kind.
    Both,
    /// Each tab has its accent; item keys are plain.
    PerTab,
    /// One accent everywhere; item keys are coloured by kind.
    PerKind,
}

impl Scheme {
    pub fn next(self) -> Self {
        match self {
            Scheme::Both => Scheme::PerTab,
            Scheme::PerTab => Scheme::PerKind,
            Scheme::PerKind => Scheme::Both,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Scheme::Both => "tab + kind colours",
            Scheme::PerTab => "tab colours only",
            Scheme::PerKind => "kind colours only",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Scheme::Both => "both",
            Scheme::PerTab => "tab",
            Scheme::PerKind => "kind",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "both" => Some(Scheme::Both),
            "tab" => Some(Scheme::PerTab),
            "kind" => Some(Scheme::PerKind),
            _ => None,
        }
    }
}

pub fn tab_color(tab: usize) -> Color {
    match tab {
        0 => Color::Rgb(0xf0, 0xc0, 0x6c), // workspaces: amber
        1 => Color::Rgb(0x56, 0xc8, 0xd8), // backlog: cyan
        2 => Color::Rgb(0x98, 0xd3, 0x7a), // chat: green
        3 => Color::Rgb(0xc6, 0x8c, 0xf0), // skills: violet
        4 => Color::Rgb(0xf7, 0x8c, 0x6c), // logs: orange
        _ => Color::Rgb(0x6c, 0xb0, 0xff), // settings: blue
    }
}

/// The colour of an item's kind, from its key's prefix.
pub fn kind_color(key: &str) -> Color {
    let prefix = key.split('-').next().unwrap_or("");
    match prefix {
        "ANA" => Color::Rgb(0x82, 0xaa, 0xff),
        "MOD" => Color::Rgb(0x7f, 0xdb, 0xca),
        "CLEAN" => Color::Rgb(0xad, 0xdb, 0x67),
        "TOOL" => Color::Rgb(0xf7, 0x8c, 0x6c),
        "FIX" => Color::Rgb(0xff, 0x6b, 0x81),
        "FEAT" => Color::Rgb(0xc7, 0x92, 0xea),
        "VAL" => Color::Rgb(0xff, 0xcb, 0x8b),
        "R" => Color::Rgb(0xf0, 0xc0, 0x6c),
        _ => TEXT,
    }
}

pub fn status_color(status: Status) -> Color {
    match status {
        Status::Open => TEXT,
        Status::Queued => DIM,
        Status::InProgress => Color::Rgb(0x6c, 0xb0, 0xff),
        Status::AwaitingApproval => Color::Rgb(0xf0, 0xc0, 0x6c),
        Status::Blocked | Status::Failed => Color::Rgb(0xff, 0x6b, 0x81),
        Status::Done => Color::Rgb(0x98, 0xd3, 0x7a),
        Status::Closed => DIM,
    }
}

pub fn status_glyph(status: Status) -> &'static str {
    match status {
        Status::Open => "○",
        Status::Queued => "◌",
        Status::InProgress => "◐",
        Status::AwaitingApproval => "⚠",
        Status::Blocked => "⊘",
        Status::Failed => "✗",
        Status::Done => "●",
        Status::Closed => "✓",
    }
}

pub fn glyph(status: Status) -> Span<'static> {
    Span::styled(status_glyph(status), Style::new().fg(status_color(status)))
}

/// "● done" — glyph and word, so status never relies on colour alone.
pub fn status_span(status: Status) -> Span<'static> {
    Span::styled(
        format!("{} {}", status_glyph(status), status.label()),
        Style::new().fg(status_color(status)),
    )
}

/// What a view needs to know about the active scheme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub accent: Color,
    pub kinds: bool,
    pub per_tab: bool,
}

impl Palette {
    pub fn new(scheme: Scheme, tab: usize) -> Self {
        Palette {
            accent: if scheme == Scheme::PerKind { NEUTRAL } else { tab_color(tab) },
            kinds: scheme != Scheme::PerTab,
            per_tab: scheme != Scheme::PerKind,
        }
    }

    pub fn key(&self, key: &str) -> Style {
        let fg = if self.kinds { kind_color(key) } else { TEXT };
        Style::new().fg(fg).add_modifier(Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }
}

pub fn dim() -> Style {
    Style::new().fg(DIM)
}

pub fn text() -> Style {
    Style::new().fg(TEXT)
}

pub fn bold() -> Style {
    Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
}

/// A key cap: ` N ` in ink on the accent.
pub fn chip(key: &str, color: Color) -> Span<'static> {
    Span::styled(format!(" {key} "), Style::new().fg(INK).bg(color).add_modifier(Modifier::BOLD))
}
