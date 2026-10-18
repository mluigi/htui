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

/// Every status with its mark, in the order the stages run.
pub const TABLE: &[(Status, StatusMark)] = &[
    (
        Status::Done,
        StatusMark {
            glyph: "●",
            word: "done",
        },
    ),
    (
        Status::InProgress,
        StatusMark {
            glyph: "◐",
            word: "running",
        },
    ),
    (
        Status::AwaitingApproval,
        StatusMark {
            glyph: "⚠",
            word: "needs you",
        },
    ),
    (
        Status::Failed,
        StatusMark {
            glyph: "✗",
            word: "failed",
        },
    ),
    (
        Status::Blocked,
        StatusMark {
            glyph: "⊘",
            word: "blocked",
        },
    ),
    (
        Status::Queued,
        StatusMark {
            glyph: "◌",
            word: "queued",
        },
    ),
    (
        Status::Open,
        StatusMark {
            glyph: "○",
            word: "open",
        },
    ),
    (
        Status::Closed,
        StatusMark {
            glyph: "✓",
            word: "closed",
        },
    ),
];

/// What a status missing from [`TABLE`] shows. A test pins that no [`Status`] needs it.
const UNKNOWN: StatusMark = StatusMark {
    glyph: "?",
    word: "unknown",
};

/// The mark for `status`.
#[must_use]
pub fn mark(status: Status) -> StatusMark {
    TABLE
        .iter()
        .find(|(candidate, _)| *candidate == status)
        .map_or(UNKNOWN, |(_, mark)| *mark)
}

/// The glyph and word of `status` as styled spans, both in [`Theme::status_style`]: the glyph, a
/// space, the word.
#[must_use]
pub fn spans(status: Status, theme: &Theme) -> Vec<Span<'static>> {
    let mark = mark(status);
    let style = theme.status_style(status);
    vec![
        Span::styled(mark.glyph, style),
        Span::styled(" ", style),
        Span::styled(mark.word, style),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::cells::cell_width;

    #[test]
    fn every_status_has_exactly_one_row_in_the_table() {
        for status in Status::ALL {
            let rows = TABLE.iter().filter(|(s, _)| s == status).count();
            assert_eq!(rows, 1, "{status:?}");
            assert_ne!(mark(*status), UNKNOWN, "{status:?} fell through");
        }
        assert_eq!(
            TABLE.len(),
            Status::ALL.len(),
            "a row for a status that is gone"
        );
    }

    #[test]
    fn the_table_reads_as_the_maintainer_listed_it() {
        let shown: Vec<(&str, &str)> = TABLE
            .iter()
            .map(|(_, mark)| (mark.glyph, mark.word))
            .collect();
        assert_eq!(
            shown,
            [
                ("●", "done"),
                ("◐", "running"),
                ("⚠", "needs you"),
                ("✗", "failed"),
                ("⊘", "blocked"),
                ("◌", "queued"),
                ("○", "open"),
                ("✓", "closed"),
            ]
        );
        assert_eq!(mark(Status::AwaitingApproval).word, "needs you");
        assert_eq!(mark(Status::InProgress).word, "running");
    }

    #[test]
    fn no_word_carries_an_underscore_or_a_raw_database_name() {
        for (status, mark) in TABLE {
            assert!(!mark.word.contains('_'), "{status:?}: {}", mark.word);
            assert!(!mark.word.is_empty(), "{status:?}");
            if status.as_str().contains('_') {
                assert_ne!(mark.word, status.as_str());
            }
        }
    }

    #[test]
    fn each_glyph_is_one_cell_and_no_two_statuses_share_a_mark() {
        for (status, mark) in TABLE {
            assert_eq!(cell_width(mark.glyph), 1, "{status:?}: {}", mark.glyph);
        }
        for (i, (_, a)) in TABLE.iter().enumerate() {
            for (_, b) in &TABLE[i + 1..] {
                assert_ne!(a.glyph, b.glyph);
                assert_ne!(a.word, b.word);
            }
        }
    }

    #[test]
    fn the_spans_are_glyph_space_word_in_the_status_style() {
        let theme = Theme::default();
        for status in Status::ALL {
            let spans = spans(*status, &theme);
            let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
            let mark = mark(*status);
            assert_eq!(text, format!("{} {}", mark.glyph, mark.word));
            for span in &spans {
                assert_eq!(span.style, theme.status_style(*status), "{status:?}");
            }
        }
    }

    #[test]
    fn a_failure_and_a_wait_are_told_apart_by_colour() {
        let theme = Theme::default();
        let failed = spans(Status::Failed, &theme);
        let waiting = spans(Status::AwaitingApproval, &theme);
        assert_eq!(failed[0].style, theme.error);
        assert_eq!(waiting[0].style, theme.warning);
        assert_ne!(failed[0].style, waiting[0].style);
    }

    #[test]
    fn under_monochrome_no_status_names_a_colour() {
        let theme = Theme::monochrome();
        for status in Status::ALL {
            for span in spans(*status, &theme) {
                assert_eq!((span.style.fg, span.style.bg), (None, None), "{status:?}");
            }
        }
        // Without colour the glyph and the word alone tell the statuses apart.
        let shown: Vec<_> = Status::ALL.iter().map(|s| mark(*s)).collect();
        for (i, a) in shown.iter().enumerate() {
            assert!(
                shown[i + 1..]
                    .iter()
                    .all(|b| a.glyph != b.glyph && a.word != b.word)
            );
        }
    }
}
