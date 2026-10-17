//! Key chips and the colour legend (MOD-115). Labels arrive pre-resolved from
//! [`Keys`](crate::keys::Keys); nothing here hard-codes a chord.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::Theme;

/// One key as a chip. Stub: the bare label.
#[must_use]
pub fn key_chip(key: &str, _theme: &Theme) -> Span<'static> {
    Span::raw(key.to_owned())
}

/// `(key label, action word)` pairs as one line of chips. Stub: empty.
#[must_use]
pub fn chip_line(_pairs: &[(String, &str)], _theme: &Theme) -> Line<'static> {
    Line::default()
}

/// A legend of `(word, style)` entries. Stub: empty.
#[must_use]
pub fn legend(_entries: &[(&str, Style)], _theme: &Theme) -> Line<'static> {
    Line::default()
}
