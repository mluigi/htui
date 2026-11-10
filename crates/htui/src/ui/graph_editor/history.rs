//! MOD-141 D4, D6: undo and redo of whole labelled bodies (100 deep, redo cleared by a new edit)
//! and the one write in flight, with what `Applied`, `Stale` and `Failed` do to them. Pure: no
//! I/O, no keys.

use htui_core::model::GraphBody;

/// How many applied edits undo remembers; past it the oldest is dropped (D6).
pub(crate) const UNDO_DEPTH: usize = 100;
/// The refusal when undo has nothing left.
pub(crate) const NOTHING_TO_UNDO: &str = "nothing to undo";
/// The refusal when redo has nothing left.
pub(crate) const NOTHING_TO_REDO: &str = "nothing to redo";
/// The refusal while a write waits for the store's answer (D4).
pub(crate) const IN_FLIGHT: &str = "still saving \u{2014} wait for the store's answer";

/// One undoable step: the body to go back (or forward) to, and what it was ("adding review").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    /// What the step was, e.g. "adding review".
    pub(crate) label: String,
    /// The whole body to send to take the step back (or forward).
    pub(crate) body: GraphBody,
}

/// The write in flight, by what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Write {
    /// A new edit, by its label.
    Edit(String),
    /// An undo of the edit with this label.
    Undo(String),
    /// A redo of the edit with this label.
    Redo(String),
}

/// Why a write may not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A write is in flight (D4).
    Busy,
    /// The undo stack is empty.
    NothingToUndo,
    /// The redo stack is empty.
    NothingToRedo,
}

impl Refusal {
    /// The refusal in words, for the editor's notice row.
    pub(crate) const fn sentence(self) -> &'static str {
        match self {
            Self::Busy => IN_FLIGHT,
            Self::NothingToUndo => NOTHING_TO_UNDO,
            Self::NothingToRedo => NOTHING_TO_REDO,
        }
    }
}

/// Undo and redo stacks plus the one write in flight (D4, D6).
#[derive(Debug, Default)]
pub(crate) struct History {
    /// Oldest first.
    undo: Vec<Entry>,
    /// Most recently undone last.
    redo: Vec<Entry>,
    /// The write awaiting the store's answer.
    pending: Option<Pending>,
}

/// A write awaiting its answer: what it was, the body it replaces, and the entry it popped.
#[derive(Debug)]
struct Pending {
    /// What the write was.
    write: Write,
    /// The body shown when the write started; what `Applied` remembers.
    before: GraphBody,
    /// The undo (or redo) entry the write took, put back on `Failed`.
    popped: Option<Entry>,
}

impl History {
    /// The write in flight, if any.
    pub(crate) fn busy(&self) -> Option<&Write> {
        self.pending.as_ref().map(|p| &p.write)
    }

    /// The label undo would take back.
    pub(crate) fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    /// The label redo would put back.
    #[cfg(test)]
    pub(crate) fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// The (undo, redo) stack depths.
    #[cfg(test)]
    pub(crate) fn depths(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// A new edit about to be sent from `current`. Err(Busy).
    pub(crate) fn begin_edit(
        &mut self,
        label: impl Into<String>,
        current: &GraphBody,
    ) -> Result<(), Refusal> {
        if self.pending.is_some() {
            return Err(Refusal::Busy);
        }
        self.pending = Some(Pending {
            write: Write::Edit(label.into()),
            before: current.clone(),
            popped: None,
        });
        Ok(())
    }

    /// Pops the undo top; answers the body to send (CAS on `current.graph.revision`, which the
    /// caller reads). Err(Busy | NothingToUndo).
    pub(crate) fn begin_undo(&mut self, current: &GraphBody) -> Result<GraphBody, Refusal> {
        if self.pending.is_some() {
            return Err(Refusal::Busy);
        }
        let entry = self.undo.pop().ok_or(Refusal::NothingToUndo)?;
        let body = entry.body.clone();
        self.pending = Some(Pending {
            write: Write::Undo(entry.label.clone()),
            before: current.clone(),
            popped: Some(entry),
        });
        Ok(body)
    }

    /// Pops the redo top; answers the body to send, as [`Self::begin_undo`].
    /// Err(Busy | NothingToRedo).
    pub(crate) fn begin_redo(&mut self, current: &GraphBody) -> Result<GraphBody, Refusal> {
        if self.pending.is_some() {
            return Err(Refusal::Busy);
        }
        let entry = self.redo.pop().ok_or(Refusal::NothingToRedo)?;
        let body = entry.body.clone();
        self.pending = Some(Pending {
            write: Write::Redo(entry.label.clone()),
            before: current.clone(),
            popped: Some(entry),
        });
        Ok(body)
    }

    /// `Applied`: Edit -> push Entry{label, before} on undo (drop the oldest past 100), clear
    /// redo. Undo -> push Entry{label, before} on redo. Redo -> push Entry{label, before} on undo
    /// (cap), redo kept. Answers the write that landed; `None` with nothing pending.
    pub(crate) fn applied(&mut self) -> Option<Write> {
        let Pending { write, before, .. } = self.pending.take()?;
        match &write {
            Write::Edit(label) => {
                self.push_undo(label.clone(), before);
                self.redo.clear();
            }
            Write::Undo(label) => self.redo.push(Entry {
                label: label.clone(),
                body: before,
            }),
            Write::Redo(label) => self.push_undo(label.clone(), before),
        }
        Some(write)
    }

    /// `Stale` (D4): both stacks and the pending write cleared. Answers the write that missed.
    pub(crate) fn stale(&mut self) -> Option<Write> {
        let pending = self.pending.take()?;
        self.clear();
        Some(pending.write)
    }

    /// `Failed`: a popped entry goes back where it came from; nothing pushed.
    pub(crate) fn failed(&mut self) -> Option<Write> {
        let Pending { write, popped, .. } = self.pending.take()?;
        if let Some(entry) = popped {
            match write {
                Write::Undo(_) => self.undo.push(entry),
                Write::Redo(_) => self.redo.push(entry),
                // An edit pops nothing.
                Write::Edit(_) => {}
            }
        }
        Some(write)
    }

    /// An undo (or redo) the client's rules refused before it was sent (MOD-141 R1 M1): the
    /// pending write is cleared and the entry it popped is dropped, not put back, so the next undo
    /// reaches the one under it. Answers the write that was dropped; `None` with nothing pending.
    pub(crate) fn dropped(&mut self) -> Option<Write> {
        self.pending.take().map(|pending| pending.write)
    }

    /// Forgets both stacks and the pending write.
    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
    }

    /// Pushes on undo, dropping the oldest past [`UNDO_DEPTH`].
    fn push_undo(&mut self, label: String, body: GraphBody) {
        self.undo.push(Entry { label, body });
        if self.undo.len() > UNDO_DEPTH {
            let excess = self.undo.len() - UNDO_DEPTH;
            self.undo.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use htui_core::model::{GraphScope, StepGraph, StepGraphId};
    use uuid::Uuid;

    use super::*;

    /// A body told apart by its description, at `revision`.
    fn body(tag: &str, revision: i64) -> GraphBody {
        GraphBody {
            graph: StepGraph {
                id: StepGraphId::from_uuid(Uuid::nil()),
                scope: GraphScope::Library,
                name: "flow".to_owned(),
                description: tag.to_owned(),
                is_override: false,
                revision,
                created_at: DateTime::UNIX_EPOCH,
                updated_at: DateTime::UNIX_EPOCH,
            },
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Runs one edit from `current` to applied.
    fn edit(h: &mut History, label: &str, current: &GraphBody) {
        h.begin_edit(label, current).expect("edit starts");
        assert_eq!(h.applied(), Some(Write::Edit(label.to_owned())));
    }

    #[test]
    fn an_applied_edit_pushes_the_previous_body_with_its_label_and_clears_redo() {
        let mut h = History::default();
        let (a, b, c) = (body("a", 0), body("b", 1), body("c", 2));
        edit(&mut h, "adding review", &a);
        // Undo once so redo holds something, then a fresh edit must clear it.
        h.begin_undo(&b).expect("undo starts");
        assert!(h.applied().is_some());
        assert_eq!(h.depths(), (0, 1));
        edit(&mut h, "adding security review", &c);
        assert_eq!(h.depths(), (1, 0));
        assert_eq!(h.undo_label(), Some("adding security review"));
        assert_eq!(h.redo_label(), None);
        assert_eq!(h.busy(), None);
        assert_eq!(h.begin_undo(&body("d", 3)), Ok(c));
    }

    #[test]
    fn undo_sends_the_previous_body_and_applied_moves_it_to_redo() {
        let mut h = History::default();
        let (a, b) = (body("a", 0), body("b", 1));
        edit(&mut h, "adding review", &a);
        assert_eq!(h.begin_undo(&b), Ok(a.clone()));
        assert_eq!(h.busy(), Some(&Write::Undo("adding review".to_owned())));
        assert_eq!(h.depths(), (0, 0));
        assert_eq!(h.applied(), Some(Write::Undo("adding review".to_owned())));
        assert_eq!(h.depths(), (0, 1));
        assert_eq!(h.redo_label(), Some("adding review"));
        assert_eq!(h.busy(), None);
        // The redo entry is the body undo replaced.
        assert_eq!(h.begin_redo(&body("a", 2)), Ok(b));
    }

    #[test]
    fn redo_after_undo_round_trips_the_body() {
        let mut h = History::default();
        let (a, b) = (body("a", 0), body("b", 1));
        edit(&mut h, "adding review", &a);
        assert_eq!(h.begin_undo(&b), Ok(a.clone()));
        h.applied();
        let a2 = body("a", 2);
        assert_eq!(h.begin_redo(&a2), Ok(b.clone()));
        assert_eq!(h.busy(), Some(&Write::Redo("adding review".to_owned())));
        assert_eq!(h.applied(), Some(Write::Redo("adding review".to_owned())));
        assert_eq!(h.depths(), (1, 0));
        assert_eq!(h.undo_label(), Some("adding review"));
        // Undo again sends the body redo replaced.
        assert_eq!(h.begin_undo(&body("b", 3)), Ok(a2));
    }

    #[test]
    fn an_applied_redo_keeps_the_rest_of_redo() {
        let mut h = History::default();
        let (a, b, c) = (body("a", 0), body("b", 1), body("c", 2));
        edit(&mut h, "one", &a);
        edit(&mut h, "two", &b);
        // Undo both, so redo holds two entries: "two" under "one".
        assert_eq!(h.begin_undo(&c), Ok(b.clone()));
        h.applied();
        let b2 = body("b", 3);
        assert_eq!(h.begin_undo(&b2), Ok(a));
        h.applied();
        assert_eq!(h.depths(), (0, 2));
        assert_eq!(h.redo_label(), Some("one"));

        // Redo one: it lands on undo and "two" stays redoable.
        let a2 = body("a", 4);
        assert_eq!(h.begin_redo(&a2), Ok(b2));
        assert_eq!(h.applied(), Some(Write::Redo("one".to_owned())));
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.undo_label(), Some("one"));
        assert_eq!(h.redo_label(), Some("two"));
        assert_eq!(h.begin_redo(&body("b", 5)), Ok(c));
    }

    #[test]
    fn a_new_edit_after_an_undo_clears_redo() {
        let mut h = History::default();
        edit(&mut h, "one", &body("a", 0));
        edit(&mut h, "two", &body("b", 1));
        h.begin_undo(&body("c", 2)).expect("undo starts");
        h.applied();
        assert_eq!(h.depths(), (1, 1));
        edit(&mut h, "three", &body("b", 3));
        assert_eq!(h.depths(), (2, 0));
        assert_eq!(h.undo_label(), Some("three"));
        assert_eq!(h.begin_redo(&body("x", 4)), Err(Refusal::NothingToRedo));
    }

    #[test]
    fn undo_is_capped_at_100_dropping_the_oldest() {
        let mut h = History::default();
        for i in 0..=UNDO_DEPTH {
            let rev = i64::try_from(i).expect("small");
            edit(&mut h, &format!("edit {i}"), &body(&format!("b{i}"), rev));
        }
        assert_eq!(h.depths(), (UNDO_DEPTH, 0));
        assert_eq!(h.undo_label(), Some("edit 100"));
        // Walk to the bottom: the oldest left is edit 1; edit 0 was dropped.
        let mut last = None;
        for _ in 0..UNDO_DEPTH {
            last = Some(h.begin_undo(&body("now", 0)).expect("undo starts"));
            h.applied();
        }
        assert_eq!(last, Some(body("b1", 1)));
        assert_eq!(h.begin_undo(&body("now", 0)), Err(Refusal::NothingToUndo));

        // A redo lands on undo under the same cap, and the oldest stays the oldest.
        let mut h = History::default();
        for i in 0..UNDO_DEPTH {
            let rev = i64::try_from(i).expect("small");
            edit(&mut h, &format!("edit {i}"), &body(&format!("b{i}"), rev));
        }
        h.begin_undo(&body("top", 100)).expect("undo starts");
        h.applied();
        h.begin_redo(&body("b99", 101)).expect("redo starts");
        h.applied();
        assert_eq!(h.depths(), (UNDO_DEPTH, 0));
        assert_eq!(h.undo_label(), Some("edit 99"));
    }

    #[test]
    fn a_second_write_is_refused_while_one_is_in_flight() {
        let a = body("a", 0);
        let b = body("b", 1);
        let mut h = History::default();
        edit(&mut h, "one", &a);
        edit(&mut h, "two", &b);
        h.begin_undo(&body("c", 2)).expect("undo starts");
        h.applied();
        assert_eq!(h.depths(), (1, 1));

        // Edit in flight.
        h.begin_edit("three", &b).expect("edit starts");
        assert_eq!(h.begin_edit("four", &b), Err(Refusal::Busy));
        assert_eq!(h.begin_undo(&b), Err(Refusal::Busy));
        assert_eq!(h.begin_redo(&b), Err(Refusal::Busy));
        assert_eq!(h.busy(), Some(&Write::Edit("three".to_owned())));
        assert_eq!(h.depths(), (1, 1));
        h.failed();

        // Undo in flight.
        h.begin_undo(&b).expect("undo starts");
        assert_eq!(h.begin_edit("four", &b), Err(Refusal::Busy));
        assert_eq!(h.begin_undo(&b), Err(Refusal::Busy));
        assert_eq!(h.begin_redo(&b), Err(Refusal::Busy));
        assert_eq!(h.busy(), Some(&Write::Undo("one".to_owned())));
        h.failed();

        // Redo in flight.
        h.begin_redo(&b).expect("redo starts");
        assert_eq!(h.begin_edit("four", &b), Err(Refusal::Busy));
        assert_eq!(h.begin_undo(&b), Err(Refusal::Busy));
        assert_eq!(h.begin_redo(&b), Err(Refusal::Busy));
        assert_eq!(h.busy(), Some(&Write::Redo("two".to_owned())));
    }

    #[test]
    fn stale_clears_both_stacks_and_the_pending_write() {
        let mut h = History::default();
        edit(&mut h, "one", &body("a", 0));
        edit(&mut h, "two", &body("b", 1));
        h.begin_undo(&body("c", 2)).expect("undo starts");
        h.applied();
        assert_eq!(h.depths(), (1, 1));
        h.begin_edit("three", &body("b", 3)).expect("edit starts");
        assert_eq!(h.stale(), Some(Write::Edit("three".to_owned())));
        assert_eq!(h.depths(), (0, 0));
        assert_eq!(h.busy(), None);
        assert_eq!(h.undo_label(), None);
        assert_eq!(h.redo_label(), None);
        // The stale write is gone: a late Applied changes nothing.
        assert_eq!(h.applied(), None);
        assert_eq!(h.depths(), (0, 0));
    }

    #[test]
    fn failed_restores_a_popped_entry_and_pushes_nothing() {
        let (a, b, c) = (body("a", 0), body("b", 1), body("c", 2));
        let mut h = History::default();
        edit(&mut h, "one", &a);
        edit(&mut h, "two", &b);
        h.begin_undo(&c).expect("undo starts");
        h.applied();
        assert_eq!(h.depths(), (1, 1));

        // Edit: nothing popped, nothing pushed, redo kept.
        h.begin_edit("three", &b).expect("edit starts");
        assert_eq!(h.failed(), Some(Write::Edit("three".to_owned())));
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.undo_label(), Some("one"));
        assert_eq!(h.redo_label(), Some("two"));

        // Undo: the popped entry goes back on undo, unchanged.
        assert_eq!(h.begin_undo(&b), Ok(a.clone()));
        assert_eq!(h.failed(), Some(Write::Undo("one".to_owned())));
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.undo_label(), Some("one"));
        assert_eq!(h.begin_undo(&b), Ok(a));
        h.failed();

        // Redo: the popped entry goes back on redo, unchanged.
        assert_eq!(h.begin_redo(&b), Ok(c.clone()));
        assert_eq!(h.failed(), Some(Write::Redo("two".to_owned())));
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.redo_label(), Some("two"));
        assert_eq!(h.begin_redo(&b), Ok(c));
        h.failed();
        assert_eq!(h.busy(), None);
    }

    /// MOD-141 R1 M1: an undo (or redo) the client's rules refuse before it leaves is dropped:
    /// nothing in flight, nothing put back, and the next undo reaches the entry under it.
    #[test]
    fn a_dropped_entry_is_gone_and_the_next_undo_reaches_the_older_one() {
        let (a, b, c) = (body("a", 0), body("b", 1), body("c", 2));
        let mut h = History::default();
        edit(&mut h, "one", &a);
        edit(&mut h, "two", &b);
        assert_eq!(h.begin_undo(&c), Ok(b));
        assert_eq!(h.dropped(), Some(Write::Undo("two".to_owned())));
        assert_eq!(h.busy(), None);
        assert_eq!(h.depths(), (1, 0));
        assert_eq!(h.undo_label(), Some("one"));
        assert_eq!(h.begin_undo(&c), Ok(a));
        assert_eq!(h.applied(), Some(Write::Undo("one".to_owned())));
        assert_eq!(h.depths(), (0, 1));

        // A redo dropped the same way.
        assert_eq!(h.begin_redo(&body("a", 3)), Ok(c));
        assert_eq!(h.dropped(), Some(Write::Redo("one".to_owned())));
        assert_eq!(h.depths(), (0, 0));
        assert_eq!(h.busy(), None);

        // Nothing in flight: nothing to drop.
        assert_eq!(h.dropped(), None);
    }

    #[test]
    fn nothing_to_undo_and_nothing_to_redo_are_refusals_with_their_sentences() {
        let mut h = History::default();
        let a = body("a", 0);
        assert_eq!(h.begin_undo(&a), Err(Refusal::NothingToUndo));
        assert_eq!(h.begin_redo(&a), Err(Refusal::NothingToRedo));
        assert_eq!(h.busy(), None);
        assert_eq!(Refusal::NothingToUndo.sentence(), NOTHING_TO_UNDO);
        assert_eq!(Refusal::NothingToRedo.sentence(), NOTHING_TO_REDO);
        assert_eq!(Refusal::Busy.sentence(), IN_FLIGHT);
        assert_eq!(NOTHING_TO_UNDO, "nothing to undo");
        assert_eq!(NOTHING_TO_REDO, "nothing to redo");
        assert_eq!(
            IN_FLIGHT,
            "still saving \u{2014} wait for the store's answer"
        );
    }

    #[test]
    fn an_answer_with_nothing_pending_changes_nothing() {
        let mut h = History::default();
        edit(&mut h, "one", &body("a", 0));
        edit(&mut h, "two", &body("b", 1));
        h.begin_undo(&body("c", 2)).expect("undo starts");
        h.applied();
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.applied(), None);
        assert_eq!(h.stale(), None);
        assert_eq!(h.failed(), None);
        assert_eq!(h.depths(), (1, 1));
        assert_eq!(h.undo_label(), Some("one"));
        assert_eq!(h.redo_label(), Some("two"));
        assert_eq!(h.busy(), None);
    }

    #[test]
    fn clear_forgets_both_stacks_and_the_pending_write() {
        let mut h = History::default();
        edit(&mut h, "one", &body("a", 0));
        h.begin_undo(&body("b", 1)).expect("undo starts");
        h.clear();
        assert_eq!(h.depths(), (0, 0));
        assert_eq!(h.busy(), None);
        assert_eq!(h.applied(), None);
    }
}
