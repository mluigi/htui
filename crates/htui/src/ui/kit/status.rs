//! One glyph and one human word per [`Status`], from a single table (MOD-115). No raw internal
//! name (`awaiting_approval`) reaches the screen.

use htui_core::model::Status;
use ratatui::text::Span;

use crate::ui::Theme;

/// A status as shown: its glyph and its word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusMark {
    /// The one-cell glyph.
    pub glyph: &'static str,
    /// The plain word.
    pub word: &'static str,
}

/// Every status with its mark. Stub: filled in by M1 task 2.
pub const TABLE: &[(Status, StatusMark)] = &[];

/// The mark for `status`. Stub: a blank mark.
#[must_use]
pub fn mark(_status: Status) -> StatusMark {
    StatusMark {
        glyph: "",
        word: "",
    }
}

/// The glyph and word of `status` as styled spans. Stub: none.
#[must_use]
pub fn spans(_status: Status, _theme: &Theme) -> Vec<Span<'static>> {
    Vec::new()
}
