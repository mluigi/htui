//! The Notes sub-tab: the item's notes as a chronological thread, and `a` to add one
//! (MOD-13 milestone 5 D8).
//!
//! - **Keys**: `a` sends `NoteForm` (D1, D2: the worker refuses offline, and the App puts that on
//!   the status line); its answer opens a [`Compose`] area over the pane, which then captures
//!   every key. Ctrl+S sends `AddNote`, Ctrl+E hands the text to `$EDITOR`, `Esc` drops it. While
//!   browsing, `J`/`K` and `PgUp`/`PgDn` scroll the thread and every other key passes.
//! - **Replies** are this pane's own requests for this pane's item: a `NoteForm` opens the area
//!   only for the `a` that asked, and `NoteAdded` closes it and re-reads the thread.
//! - **D10**: a refused `AddNote` (offline, the validator, an unknown item) keeps the text and
//!   says the sentence; any other failure may have followed a COMMIT whose answer was lost, so it
//!   is hedged and the thread re-read.
//! - A note's body is drawn one row per line (D8), so a multi-line note reads as written.

use htui_core::model::{ItemId, Note};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::hand_written::{ADD_NOTE_NAME, NOTE_FORM_NAME, write_refused};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::compose::{self, Compose, ComposeOutcome, may_have_landed};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The `Notes` reply as a thread, oldest first.
///
/// Notes are append-only (`R-ENT-11`), so the thread is a log: a stamp line and the body, no
/// edited-at column and nothing to sort by but `created_at`.
#[derive(Debug, Default)]
pub struct NotesTab {
    /// The notes of the selected item, oldest first.
    notes: Vec<Note>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// The selected item's key, from `StoreReply::Item` (E2): the temp-file stem.
    key: Option<String>,
    /// First visible line.
    scroll: Scroll,
    /// The `NoteForm` read `a` sent; its reply opens the area only for this item.
    opening: Option<ItemId>,
    /// The open compose area (D6).
    compose: Option<Compose>,
}

impl NotesTab {
    /// Identity of the Notes sub-tab.
    pub const ID: DetailId = DetailId("notes");

    /// A sub-tab with no notes yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread: per note a stamp line, then one line per row of its body (D8), blank-separated.
    /// `split('\n')`, not `lines()`, so a one-line body (and `""`) gives exactly one line (V17).
    fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for note in &self.notes {
            if !out.is_empty() {
                out.push(Line::raw(""));
            }
            out.push(Line::styled(
                note.created_at.format(STAMP).to_string(),
                theme.dim,
            ));
            out.extend(
                note.body
                    .split('\n')
                    .map(|row| Line::styled(row.to_owned(), theme.base)),
            );
        }
        out
    }

    /// A key while the area is open: its outcome, mapped for the Backlog.
    fn on_compose_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Option<Handled> {
        let compose = self.compose.as_mut()?;
        Some(match compose.on_key(key) {
            ComposeOutcome::Stay => Handled::Consumed,
            ComposeOutcome::Pass => Handled::Pass,
            ComposeOutcome::Cancel => {
                self.compose = None;
                Handled::Consumed
            }
            ComposeOutcome::Save(request) => {
                ctx.request(request);
                Handled::Consumed
            }
            ComposeOutcome::External(edit) => {
                ctx.emit(Action::EditExternally(edit));
                Handled::Consumed
            }
        })
    }

    /// D10: an `AddNote` failed. A refusal keeps the text and says the sentence; anything else is
    /// hedged and the thread re-read. A failure with no busy area (an item change dropped it) is
    /// ignored.
    fn on_add_failed(&mut self, message: &str, ctx: &Ctx<'_>) {
        let Some(compose) = self
            .compose
            .as_mut()
            .filter(|compose| compose.busy() == Some(ADD_NOTE_NAME))
        else {
            return;
        };
        if write_refused(message) {
            compose.settle(Some(message.to_owned()));
        } else {
            compose.settle(Some(may_have_landed(message, "thread")));
            if let Some(item) = self.item {
                ctx.request(StoreRequest::Notes(item));
            }
        }
    }
}

impl DetailTab for NotesTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Notes"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.notes.clear();
        self.key = None;
        self.opening = None;
        self.compose = None;
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(handled) = self.on_compose_key(key, ctx) {
            return handled;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        if key.code == KeyCode::Char('a') {
            let Some(item) = self.item else {
                return Handled::Pass;
            };
            self.opening = Some(item);
            ctx.request(StoreRequest::NoteForm { item });
            return Handled::Consumed;
        }
        // D8: a real row count, now that a body may span rows.
        self.scroll.on_key(key, self.lines(ctx.theme).len())
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match self.compose.as_mut() {
            Some(compose) => {
                compose.on_paste(text);
                Handled::Consumed
            }
            None => Handled::Pass,
        }
    }

    fn captures_input(&self) -> bool {
        self.compose.is_some()
    }

    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        match self.compose.as_mut() {
            Some(compose) => compose.on_external_edit(outcome),
            None => tracing::debug!("no note compose area to take the $EDITOR outcome"),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Notes(notes) => {
                self.notes.clone_from(notes);
                self.scroll.reset();
            }
            StoreReply::Item(row) => {
                if let Some(row) = row.as_ref()
                    && Some(row.id) == self.item
                {
                    self.key = Some(row.key.clone());
                }
            }
            StoreReply::NoteForm { item }
                if self.opening == Some(*item) && Some(*item) == self.item =>
            {
                self.opening = None;
                self.compose = Some(Compose::note(*item, self.key.as_deref()));
            }
            StoreReply::NoteAdded { item }
                if Some(*item) == self.item
                    && self.compose.as_ref().and_then(Compose::busy) == Some(ADD_NOTE_NAME) =>
            {
                self.compose = None;
                ctx.request(StoreRequest::Notes(*item));
            }
            // D2: the App already put the refusal on the status line.
            StoreReply::Failed { request, .. } if *request == NOTE_FORM_NAME => {
                self.opening = None;
            }
            StoreReply::Failed { request, message } if *request == ADD_NOTE_NAME => {
                self.on_add_failed(message, ctx);
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if let Some(compose) = &self.compose {
            compose::render(frame, area, compose, " New note ", None, ctx.theme);
            return;
        }
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if self.notes.is_empty() {
            message(frame, area, "No notes for this item.", ctx.theme);
            return;
        }
        frame.render_widget(
            Paragraph::new(Text::from(self.lines(ctx.theme)))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::store::{MemStore, ReadStore as _};

    use super::*;
    use crate::app::Action;
    use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome};
    use crate::hand_written::{ADD_NOTE_NAME, NOTE_FORM_NAME};
    use crate::store_worker::StoreRequest;
    use crate::ui::tabs::backlog::detail::compose::bench::{Shell, ctrl, drawn, key};
    use crate::ui::tabs::backlog::detail::compose::may_have_landed;

    const ITEM: ItemId = ids::HTUI_FEAT_1;

    /// A request as the bench prints it.
    fn sent(request: &StoreRequest) -> String {
        format!("{request:?}")
    }

    /// A pane on FEAT-1 that has landed the demo's notes and the item row (E2's key).
    async fn pane(shell: &Shell) -> NotesTab {
        let store = MemStore::demo();
        let mut pane = NotesTab::new();
        pane.on_item_change(Some(ITEM));
        let notes = store.notes(ITEM).await.expect("the notes");
        pane.on_reply(&StoreReply::Notes(notes), &mut shell.ctx());
        let item = store.item(ITEM).await.expect("the item");
        pane.on_reply(&StoreReply::Item(Box::new(item)), &mut shell.ctx());
        pane
    }

    /// [`pane`] with `a` pressed and its `NoteForm` read answered: the area is open.
    async fn composing(shell: &Shell) -> NotesTab {
        let mut pane = pane(shell).await;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(
            shell.requests(),
            [sent(&StoreRequest::NoteForm { item: ITEM })]
        );
        pane.on_reply(&StoreReply::NoteForm { item: ITEM }, &mut shell.ctx());
        assert!(pane.captures_input());
        pane
    }

    /// [`composing`] with `text` pasted and Ctrl+S pressed: an `AddNote` is in flight.
    async fn saving(shell: &Shell, text: &str) -> NotesTab {
        let mut pane = composing(shell).await;
        assert_eq!(pane.on_paste(text, &mut shell.ctx()), Handled::Consumed);
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        assert_eq!(shell.requests().len(), 1, "one AddNote");
        pane
    }

    fn compose(pane: &NotesTab) -> &Compose {
        pane.compose.as_ref().expect("the area is open")
    }

    #[tokio::test]
    async fn a_sends_note_form_and_nothing_without_an_item() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(
            shell.requests(),
            [sent(&StoreRequest::NoteForm { item: ITEM })]
        );
        assert!(!pane.captures_input(), "nothing opens before the reply");

        let mut none = NotesTab::new();
        none.on_item_change(None);
        assert_eq!(
            none.on_key(key(KeyCode::Char('a')), &mut shell.ctx()),
            Handled::Pass
        );
        assert!(shell.actions().is_empty());
    }

    #[tokio::test]
    async fn the_note_form_reply_opens_the_area_and_it_captures() {
        let shell = Shell::new();
        let pane = composing(&shell).await;
        assert_eq!(compose(&pane).body(), "");
        assert_eq!(compose(&pane).busy(), None);
        let text = drawn(43, 23, |frame, area| {
            pane.render(frame, area, &shell.ctx());
        });
        assert!(text.contains(" New note "), "{text}");
    }

    #[tokio::test]
    async fn a_note_form_reply_for_another_item_or_unasked_is_ignored() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        // Unasked.
        pane.on_reply(&StoreReply::NoteForm { item: ITEM }, &mut shell.ctx());
        assert!(!pane.captures_input());
        // Asked, answered for another item.
        let _ = pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        pane.on_reply(
            &StoreReply::NoteForm {
                item: ids::HTUI_ANA_1,
            },
            &mut shell.ctx(),
        );
        assert!(!pane.captures_input());
    }

    #[tokio::test]
    async fn ctrl_s_sends_add_note_with_the_text() {
        let shell = Shell::new();
        let mut pane = composing(&shell).await;
        let _ = pane.on_paste("Hi.\n\n", &mut shell.ctx());
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        let requests = shell.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(requests[0].starts_with("AddNote"), "{requests:?}");
        assert!(requests[0].contains("HandText { len: 3 }"), "{requests:?}");
        assert!(!requests[0].contains("Hi."), "{requests:?}");
        assert_eq!(compose(&pane).busy(), Some(ADD_NOTE_NAME));
    }

    #[tokio::test]
    async fn note_added_closes_the_area_and_rereads_the_thread() {
        let shell = Shell::new();
        let mut pane = saving(&shell, "Hi.").await;
        pane.on_reply(&StoreReply::NoteAdded { item: ITEM }, &mut shell.ctx());
        assert!(pane.compose.is_none());
        assert!(!pane.captures_input());
        assert_eq!(shell.requests(), [sent(&StoreRequest::Notes(ITEM))]);
    }

    #[tokio::test]
    async fn a_refusal_keeps_the_text_and_says_the_sentence() {
        let shell = Shell::new();
        let mut pane = saving(&shell, "Hi.").await;
        let message = "constraint violated: a note needs text".to_owned();
        pane.on_reply(
            &StoreReply::Failed {
                request: ADD_NOTE_NAME,
                message: message.clone(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(compose(&pane).notice(), Some(message.as_str()));
        assert_eq!(compose(&pane).body(), "Hi.");
        assert_eq!(compose(&pane).busy(), None);
        assert!(shell.actions().is_empty(), "no re-read for a refusal");
    }

    #[tokio::test]
    async fn a_store_failure_hedges_and_rereads() {
        let shell = Shell::new();
        let mut pane = saving(&shell, "Hi.").await;
        let message = "store backend error: connection reset".to_owned();
        pane.on_reply(
            &StoreReply::Failed {
                request: ADD_NOTE_NAME,
                message: message.clone(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(
            compose(&pane).notice(),
            Some(may_have_landed(&message, "thread").as_str())
        );
        assert_eq!(shell.requests(), [sent(&StoreRequest::Notes(ITEM))]);
        assert_eq!(compose(&pane).body(), "Hi.");
        assert_eq!(compose(&pane).busy(), None);
    }

    #[tokio::test]
    async fn a_failed_note_form_opens_nothing_and_adds_no_error() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let _ = pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        let _ = shell.actions();
        pane.on_reply(
            &StoreReply::Failed {
                request: NOTE_FORM_NAME,
                message: "store unreachable".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(shell.actions().is_empty(), "the App says it");
        pane.on_reply(&StoreReply::NoteForm { item: ITEM }, &mut shell.ctx());
        assert!(!pane.captures_input(), "a late reply opens nothing");
    }

    #[tokio::test]
    async fn ctrl_e_emits_an_external_edit_with_the_key_stem() {
        let shell = Shell::new();
        let mut pane = composing(&shell).await;
        let _ = pane.on_paste("draft", &mut shell.ctx());
        assert_eq!(pane.on_key(ctrl('e'), &mut shell.ctx()), Handled::Consumed);
        let actions = shell.actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::EditExternally(edit)] if *edit == ExternalEdit {
                    text: "draft".to_owned(),
                    stem: "FEAT-1-note".to_owned(),
                }
            ),
            "{actions:?}"
        );
    }

    #[tokio::test]
    async fn an_edited_outcome_reaches_the_area_through_on_external_edit() {
        let shell = Shell::new();
        let mut open = composing(&shell).await;
        let _ = open.on_key(ctrl('e'), &mut shell.ctx());
        open.on_external_edit(
            ExternalEditOutcome::Edited("From the editor.\n".to_owned()),
            &mut shell.ctx(),
        );
        assert_eq!(compose(&open).body(), "From the editor.");
        assert_eq!(compose(&open).notice(), Some(EDITED));
        // With no area open the outcome is dropped.
        let mut closed = pane(&shell).await;
        closed.on_external_edit(
            ExternalEditOutcome::Edited("x".to_owned()),
            &mut shell.ctx(),
        );
        assert!(closed.compose.is_none());
    }

    #[tokio::test]
    async fn a_two_line_note_renders_as_two_rows() {
        let shell = Shell::new();
        let mut pane = NotesTab::new();
        pane.on_item_change(Some(ITEM));
        let mut note = MemStore::demo()
            .notes(ITEM)
            .await
            .expect("the notes")
            .remove(0);
        note.body = "a\nb".to_owned();
        pane.on_reply(&StoreReply::Notes(vec![note]), &mut shell.ctx());
        let lines = pane.lines(&shell.theme);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[1], Line::styled("a", shell.theme.base));
        assert_eq!(lines[2], Line::styled("b", shell.theme.base));
    }

    /// V17: a one-line note gives exactly the stamp and body lines it gave before D8.
    #[tokio::test]
    async fn a_one_line_note_renders_as_before() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let theme = &shell.theme;
        let mut old = Vec::new();
        for note in &pane.notes {
            assert!(!note.body.contains('\n'), "the demo's notes are one line");
            if !old.is_empty() {
                old.push(Line::raw(""));
            }
            old.push(Line::styled(
                note.created_at.format(STAMP).to_string(),
                theme.dim,
            ));
            old.push(Line::styled(note.body.clone(), theme.base));
        }
        assert_eq!(old.len(), 5, "two demo notes");
        assert_eq!(pane.lines(theme), old);
    }

    /// D11: `w` is not the pane's; it passes while browsing.
    #[tokio::test]
    async fn w_passes_while_browsing() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        assert_eq!(
            pane.on_key(key(KeyCode::Char('w')), &mut shell.ctx()),
            Handled::Pass
        );
        let alt_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(pane.on_key(alt_a, &mut shell.ctx()), Handled::Pass);
        assert!(shell.actions().is_empty());
    }

    #[tokio::test]
    async fn an_item_change_drops_the_area() {
        let shell = Shell::new();
        let mut pane = saving(&shell, "Hi.").await;
        pane.on_item_change(Some(ids::HTUI_ANA_1));
        assert!(pane.compose.is_none());
        assert!(pane.key.is_none());
        assert!(pane.opening.is_none());
        // The old item's `NoteAdded` finds no busy area and re-reads nothing.
        pane.on_reply(&StoreReply::NoteAdded { item: ITEM }, &mut shell.ctx());
        assert!(shell.actions().is_empty());
    }

    /// While composing, `Esc` closes the area and `ctrl-c` passes.
    #[tokio::test]
    async fn esc_closes_the_area_and_ctrl_c_passes() {
        let shell = Shell::new();
        let mut pane = composing(&shell).await;
        assert_eq!(pane.on_key(ctrl('c'), &mut shell.ctx()), Handled::Pass);
        assert!(pane.captures_input());
        assert_eq!(
            pane.on_key(key(KeyCode::Esc), &mut shell.ctx()),
            Handled::Consumed
        );
        assert!(!pane.captures_input());
        assert!(shell.actions().is_empty());
    }
}
