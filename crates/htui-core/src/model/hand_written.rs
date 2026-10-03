//! MOD-13 milestone 5 D4: the one validator for hand-written notes and documents.
//!
//! The Notes and Docs compose areas call it for early feedback, and `htui::hand_written` calls it
//! again as the authority before anything is written. Every function answers the canonical text,
//! the exact string the store receives, so a caller never trims on its own.
//!
//! - A note body and a document body are `trim_end`ed (leading indentation is markdown and is
//!   kept), then must not be blank.
//! - A title is trimmed, must not be blank, and must be one line.
//! - A kind is trimmed, must not be blank, and must hold no control character (A2): no line
//!   break, no tab, no U+0000. A space is allowed (`code review`). `judge` and `handoff` are
//!   ordinary kinds here: a hand-written row of either is plain data, read by whoever reads that
//!   kind, so refusing them would only take a word away from the user.
//! - There is no length cap: Postgres `text` has none, and a note is prose.
//! - E3: U+0000 is refused in the title and both bodies, because Postgres `text` cannot store it
//!   (`22021`). Without the rule a NUL would land on `MemStore` and fail on `PgStore` as a
//!   `Backend` error, which reads as "it may have been written". The sentence is
//!   [`crate::store::has_nul`]'s, with the column named. The kind rule already covers NUL.

/// MOD-4 R-43's editable item summary: the kind the Docs pane always offers (D9).
pub const SUMMARY_KIND: &str = "summary";

/// Why a hand-written note or document is refused. Every sentence names its field (D4); the
/// worker answers it as `StoreError::Constraint` through `Display`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandWrittenError {
    /// The note is blank once trimmed.
    #[error("a note needs text")]
    BlankNote,
    /// The kind is blank once trimmed.
    #[error("a document needs a kind")]
    BlankKind,
    /// The trimmed kind holds a control character (A2: `\n`, `\r`, `\t`, U+0000 and the rest).
    #[error("a document kind must not contain a line break, a tab or another control character")]
    ControlInKind,
    /// The title is blank once trimmed.
    #[error("a document needs a title")]
    BlankTitle,
    /// The trimmed title holds `\n` or `\r`.
    #[error("a document title must be one line")]
    MultilineTitle,
    /// The document body is blank once trimmed.
    #[error("a document needs a body")]
    BlankBody,
    /// E3: U+0000, which Postgres `text` cannot store; the column as `has_nul` names it.
    #[error("{0} must not contain a NUL character")]
    Nul(&'static str),
}

/// A note body as stored: `trim_end`, leading indentation kept (markdown).
///
/// # Errors
/// `BlankNote`, then `Nul("item_note.body")`.
pub fn note_body(text: &str) -> Result<String, HandWrittenError> {
    body(text, HandWrittenError::BlankNote, "item_note.body")
}

/// A document kind as stored: trimmed.
///
/// # Errors
/// `BlankKind`, then `ControlInKind`.
pub fn document_kind(text: &str) -> Result<String, HandWrittenError> {
    let kind = text.trim();
    if kind.is_empty() {
        Err(HandWrittenError::BlankKind)
    } else if kind.chars().any(char::is_control) {
        Err(HandWrittenError::ControlInKind)
    } else {
        Ok(kind.to_owned())
    }
}

/// A document title as stored: trimmed.
///
/// # Errors
/// `BlankTitle`, then `MultilineTitle`, then `Nul("document.title")`.
pub fn document_title(text: &str) -> Result<String, HandWrittenError> {
    let title = text.trim();
    if title.is_empty() {
        Err(HandWrittenError::BlankTitle)
    } else if title.contains(['\n', '\r']) {
        Err(HandWrittenError::MultilineTitle)
    } else if title.contains('\0') {
        Err(HandWrittenError::Nul("document.title"))
    } else {
        Ok(title.to_owned())
    }
}

/// A document body as stored: `trim_end`.
///
/// # Errors
/// `BlankBody`, then `Nul("document.body")`.
pub fn document_body(text: &str) -> Result<String, HandWrittenError> {
    body(text, HandWrittenError::BlankBody, "document.body")
}

/// The two bodies' rule: `trim_end`, then not blank, then no NUL in `column`.
fn body(
    text: &str,
    blank: HandWrittenError,
    column: &'static str,
) -> Result<String, HandWrittenError> {
    let kept = text.trim_end();
    if kept.trim_start().is_empty() {
        Err(blank)
    } else if kept.contains('\0') {
        Err(HandWrittenError::Nul(column))
    } else {
        Ok(kept.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_note_is_refused_and_trailing_blank_lines_go() {
        assert_eq!(note_body(""), Err(HandWrittenError::BlankNote));
        assert_eq!(note_body("  \n"), Err(HandWrittenError::BlankNote));
        assert_eq!(note_body("  - item\n\n").as_deref(), Ok("  - item"));
    }

    #[test]
    fn a_kind_is_trimmed_and_may_hold_a_space() {
        assert_eq!(document_kind(" plan ").as_deref(), Ok("plan"));
        assert_eq!(document_kind("code review").as_deref(), Ok("code review"));
    }

    #[test]
    fn a_blank_kind_or_one_with_a_control_character_is_refused() {
        for blank in ["", "  "] {
            assert_eq!(
                document_kind(blank),
                Err(HandWrittenError::BlankKind),
                "{blank:?}"
            );
        }
        for control in ["pl\nan", "a\0", "a\tb"] {
            assert_eq!(
                document_kind(control),
                Err(HandWrittenError::ControlInKind),
                "{control:?}"
            );
        }
    }

    #[test]
    fn summary_and_judge_are_ordinary_kinds() {
        for kind in [SUMMARY_KIND, "judge", "handoff"] {
            assert_eq!(document_kind(kind).as_deref(), Ok(kind));
        }
    }

    #[test]
    fn a_title_is_trimmed_one_line_and_not_blank() {
        assert_eq!(document_title("  Plan  ").as_deref(), Ok("Plan"));
        assert_eq!(document_title(" "), Err(HandWrittenError::BlankTitle));
        assert_eq!(
            document_title("a\nb"),
            Err(HandWrittenError::MultilineTitle)
        );
        assert_eq!(
            document_title("a\rb"),
            Err(HandWrittenError::MultilineTitle)
        );
    }

    #[test]
    fn a_document_body_is_checked_as_a_note() {
        assert_eq!(document_body("   "), Err(HandWrittenError::BlankBody));
        assert_eq!(document_body("# T\n\nx\n\n").as_deref(), Ok("# T\n\nx"));
    }

    #[test]
    fn a_nul_is_refused_with_has_nul_s_sentence() {
        assert_eq!(
            note_body("x\0"),
            Err(HandWrittenError::Nul("item_note.body"))
        );
        assert_eq!(
            document_title("a\0b"),
            Err(HandWrittenError::Nul("document.title"))
        );
        assert_eq!(
            document_body("x\0"),
            Err(HandWrittenError::Nul("document.body"))
        );
        assert_eq!(
            HandWrittenError::Nul("document.body").to_string(),
            crate::store::has_nul("document.body")
        );
    }

    #[test]
    fn every_refusal_names_its_field() {
        let cases = [
            (HandWrittenError::BlankNote, "note"),
            (HandWrittenError::BlankKind, "kind"),
            (HandWrittenError::ControlInKind, "kind"),
            (HandWrittenError::BlankTitle, "title"),
            (HandWrittenError::MultilineTitle, "title"),
            (HandWrittenError::BlankBody, "body"),
            (HandWrittenError::Nul("document.body"), "NUL"),
        ];
        for (err, field) in cases {
            assert!(
                err.to_string().contains(field),
                "{err:?} should name {field}: {err}"
            );
        }
    }
}
