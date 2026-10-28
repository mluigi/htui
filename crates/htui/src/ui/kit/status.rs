//! One glyph and one human word per [`ItemState`], from a single table (MOD-115, MOD-144). No raw
//! internal name (`needs_you`) reaches the screen.

use htui_core::model::ItemState;
use ratatui::text::Span;

use crate::ui::Theme;

/// An item state as shown: its glyph and its word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusMark {
    /// The one-cell glyph.
    pub glyph: &'static str,
    /// The plain word.
    pub word: &'static str,
}

/// Every item state with its mark, in attention order ([`ItemState::ALL`], MOD-144 Decision A).
pub const TABLE: &[(ItemState, StatusMark)] = &[
    (
        ItemState::NeedsYou,
        StatusMark {
            glyph: "⚠",
            word: "needs you",
        },
    ),
    (
        ItemState::Failed,
        StatusMark {
            glyph: "✗",
            word: "failed",
        },
    ),
    (
        ItemState::Running,
        StatusMark {
            glyph: "◐",
            word: "running",
        },
    ),
    (
        ItemState::Stopped,
        StatusMark {
            glyph: "■",
            word: "stopped",
        },
    ),
    (
        ItemState::Blocked,
        StatusMark {
            glyph: "⊘",
            word: "blocked",
        },
    ),
    (
        ItemState::Queued,
        StatusMark {
            glyph: "◌",
            word: "queued",
        },
    ),
    (
        ItemState::Open,
        StatusMark {
            glyph: "○",
            word: "open",
        },
    ),
    (
        ItemState::Done,
        StatusMark {
            glyph: "●",
            word: "done",
        },
    ),
    (
        ItemState::Closed,
        StatusMark {
            glyph: "✓",
            word: "closed",
        },
    ),
];

/// What a state missing from [`TABLE`] shows. A test pins that no [`ItemState`] needs it.
const UNKNOWN: StatusMark = StatusMark {
    glyph: "?",
    word: "unknown",
};

/// The mark for `state`.
#[must_use]
pub fn mark(state: ItemState) -> StatusMark {
    TABLE
        .iter()
        .find(|(candidate, _)| *candidate == state)
        .map_or(UNKNOWN, |(_, mark)| *mark)
}

/// The glyph and word of `state` as styled spans, both in [`Theme::status_style`]: the glyph, a
/// space, the word.
#[must_use]
pub fn spans(state: ItemState, theme: &Theme) -> Vec<Span<'static>> {
    let mark = mark(state);
    let style = theme.status_style(state);
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
    use crate::ui::kit::label;
    use htui_core::model::RunStatus;

    #[test]
    fn every_state_has_exactly_one_row_in_the_table() {
        for state in ItemState::ALL {
            let rows = TABLE.iter().filter(|(s, _)| s == state).count();
            assert_eq!(rows, 1, "{state:?}");
            assert_ne!(mark(*state), UNKNOWN, "{state:?} fell through");
        }
        assert_eq!(
            TABLE.len(),
            ItemState::ALL.len(),
            "a row for a state that is gone"
        );
        let order: Vec<ItemState> = TABLE.iter().map(|(state, _)| *state).collect();
        assert_eq!(order, ItemState::ALL, "the table is in attention order");
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
                ("⚠", "needs you"),
                ("✗", "failed"),
                ("◐", "running"),
                ("■", "stopped"),
                ("⊘", "blocked"),
                ("◌", "queued"),
                ("○", "open"),
                ("●", "done"),
                ("✓", "closed"),
            ]
        );
        assert_eq!(mark(ItemState::NeedsYou).word, "needs you");
        assert_eq!(mark(ItemState::Running).word, "running");
    }

    #[test]
    fn no_word_carries_an_underscore_or_a_raw_database_name() {
        for (state, mark) in TABLE {
            assert!(!mark.word.contains('_'), "{state:?}: {}", mark.word);
            assert!(!mark.word.is_empty(), "{state:?}");
            if state.as_str().contains('_') {
                assert_ne!(mark.word, state.as_str());
            }
        }
    }

    #[test]
    fn the_word_is_item_state_word() {
        for (state, mark) in TABLE {
            assert_eq!(mark.word, state.word(), "{state:?}");
        }
    }

    #[test]
    fn an_item_state_reads_as_its_run_status() {
        for (state, run) in [
            (ItemState::Running, RunStatus::Running),
            (ItemState::NeedsYou, RunStatus::AwaitingApproval),
            (ItemState::Stopped, RunStatus::Stopped),
            (ItemState::Queued, RunStatus::Queued),
            (ItemState::Done, RunStatus::Done),
            (ItemState::Failed, RunStatus::Failed),
        ] {
            let item = mark(state);
            let run = label::run_status(run);
            assert_eq!((item.glyph, item.word), (run.glyph, run.word), "{state:?}");
        }
    }

    #[test]
    fn each_glyph_is_one_cell_and_no_two_states_share_a_mark() {
        for (state, mark) in TABLE {
            assert_eq!(cell_width(mark.glyph), 1, "{state:?}: {}", mark.glyph);
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
        for state in ItemState::ALL {
            let spans = spans(*state, &theme);
            let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
            let mark = mark(*state);
            assert_eq!(text, format!("{} {}", mark.glyph, mark.word));
            for span in &spans {
                assert_eq!(span.style, theme.status_style(*state), "{state:?}");
            }
        }
    }

    #[test]
    fn a_failure_and_a_wait_are_told_apart_by_colour() {
        let theme = Theme::default();
        let failed = spans(ItemState::Failed, &theme);
        let waiting = spans(ItemState::NeedsYou, &theme);
        assert_eq!(failed[0].style, theme.error);
        assert_eq!(waiting[0].style, theme.warning);
        assert_ne!(failed[0].style, waiting[0].style);
    }

    #[test]
    fn under_monochrome_no_state_names_a_colour() {
        let theme = Theme::monochrome();
        for state in ItemState::ALL {
            for span in spans(*state, &theme) {
                assert_eq!((span.style.fg, span.style.bg), (None, None), "{state:?}");
            }
        }
        // Without colour the glyph and the word alone tell the states apart.
        let shown: Vec<_> = ItemState::ALL.iter().map(|s| mark(*s)).collect();
        for (i, a) in shown.iter().enumerate() {
            assert!(
                shown[i + 1..]
                    .iter()
                    .all(|b| a.glyph != b.glyph && a.word != b.word)
            );
        }
    }
}
