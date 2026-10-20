//! The history model (MOD-115 M2 D5, blueprint §8): browser-style back and forward through the
//! places visited, 50 entries, a push only when the coarse place (tab, sub-tab, pane) changes.
//!
//! The shell records around each dispatched key, click or reveal: it hands [`History::record`]
//! the [`Loc`] before and the one after. Moving a cursor inside the same pane is not a new place,
//! so it is not recorded; the entry pushed is the full place left, cursor included, so going back
//! lands on the row the user was on. Going back records the place left on the forward stack, so
//! forward returns to its cursor too. The history lives in memory for the session (D1).

use super::{PaneId, TabLoc};
use crate::ui::tabs::registry::TabId;
use std::collections::VecDeque;

/// How many places the back stack keeps (D5); the oldest drops first.
pub const CAP: usize = 50;

/// One place in the history: the tab, its focused pane, the tab's own location and the label the
/// header's `back to` chip shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    /// The tab.
    pub tab: TabId,
    /// The pane focused in it.
    pub pane: PaneId,
    /// The tab's own part: sub-tab and cursor (`Tab::location`).
    pub at: TabLoc,
    /// `title[ › subtab_title][ · list|activity]`, for the header chip.
    pub label: String,
}

impl Loc {
    /// The coarse place `(tab, sub-tab, pane)`: two locations with the same coarse place differ
    /// only by cursor (or label), and moving between them is not recorded.
    pub fn coarse(&self) -> (TabId, Option<&'static str>, PaneId) {
        (self.tab, self.at.subtab, self.pane)
    }
}

/// The back and forward stacks.
#[derive(Debug, Default)]
pub struct History {
    /// The places to go back to, the most recent last.
    back: VecDeque<Loc>,
    /// The places to go forward to, the next one last.
    forward: Vec<Loc>,
}

impl History {
    /// Records one dispatch that went from `before` to `after`. When the coarse place changed,
    /// `before` (its cursor kept) is pushed on the back stack, the forward stack is dropped and the
    /// back stack is capped at [`CAP`]. A cursor-only move records nothing.
    pub fn record(&mut self, before: Loc, after: &Loc) {
        if before.coarse() == after.coarse() {
            return;
        }
        self.push_back(before);
        self.forward.clear();
    }

    /// Goes back from `here`: the most recent back entry, with `here` kept for forward. `None`
    /// (and nothing kept) when there is nowhere back.
    pub fn back(&mut self, here: Loc) -> Option<Loc> {
        let to = self.back.pop_back()?;
        self.forward.push(here);
        Some(to)
    }

    /// Goes forward from `here`: the next forward entry, with `here` pushed on the back stack
    /// (capped). `None` (and nothing kept) when there is nowhere forward.
    pub fn forward(&mut self, here: Loc) -> Option<Loc> {
        let to = self.forward.pop()?;
        self.push_back(here);
        Some(to)
    }

    /// Where back would go, for the header's `back to` chip.
    pub fn peek_back(&self) -> Option<&Loc> {
        self.back.back()
    }

    /// Whether there is somewhere forward, for the header's `forward` chip.
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Pushes on the back stack, dropping the oldest entries past [`CAP`].
    fn push_back(&mut self, loc: Loc) {
        self.back.push_back(loc);
        while self.back.len() > CAP {
            self.back.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CAP, History, Loc};
    use crate::ui::shell::{PaneId, TabLoc};
    use crate::ui::tabs::registry::TabId;

    fn loc(tab: &'static str, subtab: Option<&'static str>, pane: PaneId, cursor: &str) -> Loc {
        Loc {
            tab: TabId(tab),
            pane,
            at: TabLoc {
                subtab,
                subtab_title: subtab.map(str::to_uppercase),
                cursor: Some(cursor.to_owned()),
            },
            label: format!("{tab} {cursor}"),
        }
    }

    #[test]
    fn a_change_of_tab_sub_tab_or_pane_pushes_the_place_left() {
        let start = loc("backlog", Some("body"), PaneId::Detail, "MOD-1");
        for after in [
            loc("chat", None, PaneId::Detail, "MOD-1"),
            loc("backlog", Some("runs"), PaneId::Detail, "MOD-1"),
            loc("backlog", Some("body"), PaneId::List, "MOD-1"),
        ] {
            let mut history = History::default();
            history.record(start.clone(), &after);
            assert_eq!(history.peek_back(), Some(&start), "after {after:?}");
            assert!(!history.can_forward());
        }
    }

    #[test]
    fn a_cursor_only_move_is_not_recorded() {
        let mut history = History::default();
        let before = loc("backlog", Some("body"), PaneId::List, "MOD-1");
        let mut after = loc("backlog", Some("body"), PaneId::List, "MOD-2");
        after.label = "another label".to_owned();
        after.at.subtab_title = Some("Another title".to_owned());
        history.record(before, &after);
        assert_eq!(history.peek_back(), None);
        assert!(history.back(after).is_none());
        assert!(
            !history.can_forward(),
            "a back with nowhere to go keeps nothing"
        );
    }

    #[test]
    fn a_new_place_drops_the_forward_stack() {
        let mut history = History::default();
        let a = loc("backlog", None, PaneId::List, "MOD-1");
        let b = loc("chat", None, PaneId::Detail, "");
        let c = loc("skills", Some("library"), PaneId::List, "3");
        history.record(a.clone(), &b);
        assert_eq!(history.back(b.clone()), Some(a.clone()));
        assert!(history.can_forward());
        // From `a` again, a cursor-only move keeps forward.
        let a2 = loc("backlog", None, PaneId::List, "MOD-2");
        history.record(a.clone(), &a2);
        assert!(history.can_forward());
        // A new place drops it.
        history.record(a2.clone(), &c);
        assert!(!history.can_forward());
        assert!(history.forward(c.clone()).is_none());
        assert_eq!(history.peek_back(), Some(&a2));
    }

    #[test]
    fn the_back_stack_keeps_fifty_and_drops_the_oldest() {
        let mut history = History::default();
        let places: Vec<Loc> = (0..=CAP)
            .map(|i| {
                let pane = if i % 2 == 0 {
                    PaneId::List
                } else {
                    PaneId::Detail
                };
                loc("backlog", None, pane, &i.to_string())
            })
            .collect();
        for pair in places.windows(2) {
            history.record(pair[0].clone(), &pair[1]);
        }
        let here = places[CAP].clone();
        history.record(here.clone(), &loc("chat", None, PaneId::Detail, ""));
        // CAP + 1 pushes: `0` is gone, `1` is the oldest kept.
        let mut back = Vec::new();
        let mut at = loc("chat", None, PaneId::Detail, "");
        while let Some(to) = history.back(at) {
            back.push(to.at.cursor.clone().unwrap_or_default());
            at = to;
        }
        assert_eq!(back.len(), CAP);
        assert_eq!(back.first().map(String::as_str), Some("50"));
        assert_eq!(back.last().map(String::as_str), Some("1"));
    }

    #[test]
    fn forward_pushes_on_the_back_stack_within_the_cap() {
        let mut history = History::default();
        let places: Vec<Loc> = (0..=CAP + 1)
            .map(|i| {
                let pane = if i % 2 == 0 {
                    PaneId::List
                } else {
                    PaneId::Detail
                };
                loc("backlog", None, pane, &i.to_string())
            })
            .collect();
        for pair in places.windows(2) {
            history.record(pair[0].clone(), &pair[1]);
        }
        // Full: back once, then forward pushes `here` back on a full stack.
        let to = history.back(places[CAP + 1].clone()).unwrap();
        assert_eq!(history.forward(to), Some(places[CAP + 1].clone()));
        assert_eq!(history.back.len(), CAP);
    }

    #[test]
    fn back_and_forward_restore_the_full_location() {
        let mut history = History::default();
        let list = loc("backlog", Some("body"), PaneId::List, "MOD-7");
        let runs = loc("backlog", Some("runs"), PaneId::Detail, "MOD-7");
        let chat = loc("chat", None, PaneId::Detail, "");
        history.record(list.clone(), &runs);
        // The cursor moves inside Runs before the next coarse change.
        let runs_moved = loc("backlog", Some("runs"), PaneId::Detail, "MOD-9");
        history.record(runs.clone(), &runs_moved);
        history.record(runs_moved.clone(), &chat);
        assert_eq!(history.peek_back(), Some(&runs_moved));

        // Back lands on the cursor left, then on the first place.
        let back1 = history.back(chat.clone()).unwrap();
        assert_eq!(back1, runs_moved);
        // Back again from there keeps the Runs place, its cursor included, for forward.
        let back2 = history.back(back1.clone()).unwrap();
        assert_eq!(back2, list);
        assert_eq!(history.peek_back(), None);

        // Forward returns to the places left, cursors included.
        let fwd1 = history.forward(back2.clone()).unwrap();
        assert_eq!(fwd1, runs_moved);
        assert_eq!(history.peek_back(), Some(&list));
        let fwd2 = history.forward(fwd1.clone()).unwrap();
        assert_eq!(fwd2, chat);
        assert!(!history.can_forward());
        assert_eq!(history.peek_back(), Some(&runs_moved));
    }
}
