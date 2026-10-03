//! The Notes sub-tab: the item's notes as a chronological thread, and `a` to add one
//! (MOD-13 milestone 5 D8).
//!
//! - **Keys**: `a` sends `NoteForm` (D1, D2: the worker refuses offline, and the App puts that on
//!   the status line); its answer opens a [`Compose`] area over the pane, which then captures
//!   every key. Ctrl+S sends `AddNote`, Ctrl+E hands the text to `$EDITOR`, `Esc` drops it. While
//!   browsing, `J`/`K` and `PgUp`/`PgDn` scroll the thread and every other key passes.
//! - **Replies** are this pane's own requests for this pane's item: a `NoteForm` opens the area
//!   only for the `a` that asked, and only while Notes is the active sub-tab (the registry hands
//!   it to no other, E4), and `NoteAdded` closes it and re-reads the thread.
//! - **D10**: a refused `AddNote` (offline, the validator, an unknown item) keeps the text and
//!   says the sentence; any other failure may have followed a COMMIT whose answer was lost, so it
//!   is hedged and the thread re-read.
//! - **Review M1**: the area covers the thread, so the pane looks, not the user. The area stays
//!   busy under the hedge until the re-read lands; a note of this item with the sent body, newer
//!   than every note the area opened on, means it was written (the area closes and the pane says
//!   so); none means it was not (the text stays, Ctrl+S tries again). The pane holds no `UserId`
//!   (`R-NF-3`), so the author cannot be matched; `created_at` stands in for "after the area
//!   opened": a note committed after the thread was read has a later `now()` than every note in
//!   it, so a note of the same text already there never counts. An `Unreachable` failure sent the
//!   worker to the mirror before it answered, and the mirror cannot hold the note, so there "none"
//!   proves nothing: the pane says it could not check (round 1).
//! - A note's body is drawn one row per line (D8), so a multi-line note reads as written; a `\t`
//!   as spaces to the next stop, as the compose area drew it (review L2). The thread is wrapped
//!   here, by `cells::wrap` at the width of the last render, so the scroll clamps against the rows
//!   on screen (review L1); after the user's own note lands, the re-read opens at the bottom.

use std::borrow::Cow;
use std::cell::Cell;
use std::iter;

use chrono::{DateTime, Utc};
use htui_core::model::{ItemId, Note};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Text};
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::hand_written::{
    ADD_NOTE_NAME, HandText, NOTE_FORM_NAME, answered_from_the_mirror, write_refused,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::cells;
use crate::ui::tabs::backlog::detail::compose::{
    self, Compose, ComposeOutcome, could_not_check, may_have_landed, not_written,
};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crate::ui::tabs::settings::wrapped;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The browse hint on the pane's last row (D11).
const HINT: &str = "a add note";

/// `StoreRequest::Notes`' name: the `Failed` of the re-read that checks a hedge (review M1).
const NOTES_READ: &str = "notes";

/// Review M1: the re-read found the hedged note, so it was written; only the answer was lost.
const NOTE_WRITTEN: &str = "the note was written; only the store's answer was lost";

/// Review M1: an `AddNote` whose `Failed` may have followed a lost COMMIT, kept while the thread
/// re-read that settles it is in flight.
#[derive(Debug)]
struct Hedged {
    /// The failure's sentence, for the settled notice.
    why: String,
    /// The body as sent, the store's canonical form (`Debug` prints its length).
    body: HandText,
    /// The newest `created_at` in the thread when the area opened; `None` for an empty thread.
    since: Option<DateTime<Utc>>,
    /// The failure was an `Unreachable`, so the re-read is the mirror's: not finding the note
    /// there proves nothing (review M1, round 1).
    mirror: bool,
}

/// Columns between tab stops: `TextArea`'s, so a `\t` from `$EDITOR` reads in the thread as it
/// did in the compose area (review L2).
const TAB_STOP: usize = 4;

/// `row` with each `\t` as spaces to the next [`TAB_STOP`], counted in cells from the row's
/// start, as `TextArea` draws it; `ratatui` would drop it (review L2). Borrowed without one.
fn expand_tabs(row: &str) -> Cow<'_, str> {
    if !row.contains('\t') {
        return Cow::Borrowed(row);
    }
    let mut out = String::with_capacity(row.len() + TAB_STOP);
    let mut col = 0;
    for cluster in cells::graphemes(row) {
        if cluster == "\t" {
            let cells = TAB_STOP - col % TAB_STOP;
            out.extend(iter::repeat_n(' ', cells));
            col += cells;
        } else {
            out.push_str(cluster);
            col += cells::cell_width(cluster);
        }
    }
    Cow::Owned(out)
}

/// The cells to wrap the thread at: `width`, or no wrap before the first render (each row one,
/// the old lower bound on what is drawn).
fn wrap_width(width: u16) -> usize {
    if width == 0 {
        usize::MAX
    } else {
        usize::from(width)
    }
}

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
    /// The thread's width and height at the last render, for the scroll clamp and the bottom
    /// (review L1). `Cell` because `render` is `&self`.
    drawn: Cell<(u16, u16)>,
    /// The user's own note landed: the re-read opens at the bottom (review L1).
    follow: bool,
    /// The newest `created_at` in the thread when the area opened (review M1).
    since: Option<DateTime<Utc>>,
    /// The `AddNote` body in flight (review M1).
    sent: Option<HandText>,
    /// The hedged `AddNote` the re-read in flight settles (review M1).
    hedged: Option<Hedged>,
    /// Review M1: the hedge settled as written, under the thread until the next key.
    notice: Option<String>,
}

impl NotesTab {
    /// Identity of the Notes sub-tab.
    pub const ID: DetailId = DetailId("notes");

    /// A sub-tab with no notes yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread before wrapping: per note a stamp row, then one row per line of its body (D8),
    /// tabs expanded (review L2), blank-separated; `true` marks a stamp. `split('\n')`, not
    /// `lines()`, so a one-line body (and `""`) gives exactly one row (V17).
    fn rows(&self) -> impl Iterator<Item = (bool, Cow<'_, str>)> {
        self.notes.iter().enumerate().flat_map(|(at, note)| {
            let gap = (at > 0).then_some((false, Cow::Borrowed("")));
            gap.into_iter()
                .chain(iter::once((
                    true,
                    Cow::Owned(note.created_at.format(STAMP).to_string()),
                )))
                .chain(note.body.split('\n').map(|row| (false, expand_tabs(row))))
        })
    }

    /// The thread as drawn at `width` (0: unwrapped), each row wrapped by `cells::wrap`.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let width = wrap_width(width);
        self.rows()
            .flat_map(|(stamp, row)| {
                let style = if stamp { theme.dim } else { theme.base };
                cells::wrap(&row, width)
                    .into_iter()
                    .map(move |wrapped| Line::styled(wrapped, style))
            })
            .collect()
    }

    /// How many rows [`lines`](Self::lines) gives at `width`, without styling them (review L1).
    fn row_count(&self, width: u16) -> usize {
        let width = wrap_width(width);
        self.rows()
            .map(|(_, row)| cells::wrap(&row, width).len())
            .sum()
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
                if let StoreRequest::AddNote { body, .. } = &request {
                    self.sent = Some(body.clone());
                }
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
    /// hedged, the area kept busy, and the thread re-read to settle it (review M1). A failure with
    /// no busy area (an item change dropped it) is ignored.
    fn on_add_failed(&mut self, message: &str, ctx: &Ctx<'_>) {
        let Some(compose) = self
            .compose
            .as_mut()
            .filter(|compose| compose.busy() == Some(ADD_NOTE_NAME))
        else {
            return;
        };
        let sent = self.sent.take();
        if write_refused(message) {
            compose.settle(Some(message.to_owned()));
            return;
        }
        let hedge = may_have_landed(message, "thread");
        match (sent, self.item) {
            (Some(body), Some(item)) => {
                compose.checking(hedge);
                self.hedged = Some(Hedged {
                    why: message.to_owned(),
                    body,
                    since: self.since,
                    mirror: answered_from_the_mirror(message),
                });
                ctx.request(StoreRequest::Notes(item));
            }
            // Nothing to look for: the hedge alone, as before review M1.
            _ => compose.settle(Some(hedge)),
        }
    }

    /// Review M1: the re-read after a hedge. A thread of another item settles nothing. The note
    /// found closes the area and says so (and the thread opens at its bottom, where it is); none
    /// keeps the text and says it was not written, or, read from the mirror, that it could not be
    /// checked (round 1).
    fn settle_hedge(&mut self, notes: &[Note]) {
        let Some(item) = self.item else {
            return;
        };
        if notes.first().is_some_and(|note| note.item_id != item) {
            return;
        }
        let Some(hedged) = self.hedged.take() else {
            return;
        };
        let written = notes.iter().any(|note| {
            note.item_id == item
                && note.body == hedged.body.as_str()
                && hedged.since.is_none_or(|since| note.created_at > since)
        });
        if written {
            self.compose = None;
            self.notice = Some(NOTE_WRITTEN.to_owned());
            self.follow = true;
        } else if let Some(compose) = self.compose.as_mut() {
            compose.settle(Some(if hedged.mirror {
                could_not_check(&hedged.why, "thread")
            } else {
                not_written(&hedged.why)
            }));
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
        self.follow = false;
        self.since = None;
        self.sent = None;
        self.hedged = None;
        self.notice = None;
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(handled) = self.on_compose_key(key, ctx) {
            return handled;
        }
        // Review M1: the sentence stays until the next key.
        self.notice = None;
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
        // D8, review L1: the rows as wrapped at the last render, not the unwrapped lines.
        let (width, _) = self.drawn.get();
        self.scroll.on_key(key, self.row_count(width))
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
                self.settle_hedge(notes);
                // Review L1: the user's own note is the newest, so the thread opens at its
                // bottom; before a first render there is no height to fit, and it opens at the top.
                let (width, height) = self.drawn.get();
                self.scroll = if std::mem::take(&mut self.follow) && height > 0 {
                    let bottom = self.row_count(width).saturating_sub(usize::from(height));
                    Scroll::at(u16::try_from(bottom).unwrap_or(u16::MAX))
                } else {
                    Scroll::default()
                };
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
                self.since = self.notes.iter().map(|note| note.created_at).max();
                self.compose = Some(Compose::note(*item, self.key.as_deref()));
            }
            StoreReply::NoteAdded { item }
                if Some(*item) == self.item
                    && self.compose.as_ref().and_then(Compose::busy) == Some(ADD_NOTE_NAME) =>
            {
                self.compose = None;
                self.sent = None;
                self.follow = true;
                ctx.request(StoreRequest::Notes(*item));
            }
            // D2: the App already put the refusal on the status line.
            StoreReply::Failed { request, .. } if *request == NOTE_FORM_NAME => {
                self.opening = None;
            }
            StoreReply::Failed { request, message } if *request == ADD_NOTE_NAME => {
                self.on_add_failed(message, ctx);
            }
            // Review M1: the check failed too; say so rather than leave the area busy.
            StoreReply::Failed { request, .. } if *request == NOTES_READ => {
                if let (Some(hedged), Some(compose)) = (self.hedged.take(), self.compose.as_mut()) {
                    compose.settle(Some(could_not_check(&hedged.why, "thread")));
                }
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
        // Review M1's sentence under the thread; D11: the browse hint takes the last row.
        let notice = self
            .notice
            .as_deref()
            .map(|sentence| wrapped(sentence, usize::from(area.width)))
            .unwrap_or_default();
        let height = u16::try_from(notice.len()).unwrap_or(u16::MAX);
        let [list, below, hint] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(line, ctx.theme.base))
                    .collect::<Vec<_>>(),
            ),
            below,
        );
        frame.render_widget(
            Line::styled(cells::clip(HINT, usize::from(hint.width)), ctx.theme.dim),
            hint,
        );
        if self.notes.is_empty() {
            message(frame, list, "No notes for this item.", ctx.theme);
            return;
        }
        self.drawn.set((list.width, list.height));
        let lines = self.lines(list.width, ctx.theme);
        // A pane that widened since the last key has fewer rows than the offset assumed.
        let last = u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).scroll((self.scroll.offset().min(last), 0)),
            list,
        );
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::store::{MemStore, ReadStore as _, StoreError};

    use super::*;
    use crate::app::Action;
    use crate::editor::{EDITED, ExternalEdit, ExternalEditOutcome};
    use crate::hand_written::{ADD_NOTE_NAME, NOTE_FORM_NAME};
    use crate::store_worker::StoreRequest;
    use crate::ui::tabs::backlog::detail::compose::bench::{Shell, ctrl, drawn, key};
    use crate::ui::tabs::backlog::detail::compose::{
        could_not_check, may_have_landed, not_written,
    };

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
        // Review M1: busy until the re-read settles it, so Ctrl+S cannot write it twice.
        assert_eq!(compose(&pane).busy(), Some(ADD_NOTE_NAME));
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        assert!(shell.actions().is_empty(), "nothing sent while checking");
    }

    /// [`saving`] `text` and the `AddNote` answered with a store failure: hedged, and the thread
    /// re-read is in flight. Returns the failure's sentence.
    async fn hedged(shell: &Shell, text: &str) -> (NotesTab, String) {
        let mut pane = saving(shell, text).await;
        let message = "store backend error: connection reset".to_owned();
        pane.on_reply(
            &StoreReply::Failed {
                request: ADD_NOTE_NAME,
                message: message.clone(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(shell.requests(), [sent(&StoreRequest::Notes(ITEM))]);
        (pane, message)
    }

    /// `body` as a note on `item`, written now.
    async fn written(item: ItemId, body: &str) -> Note {
        let mut note = MemStore::demo()
            .notes(ITEM)
            .await
            .expect("the notes")
            .remove(0);
        note.id = htui_core::model::NoteId::new();
        note.item_id = item;
        note.body = body.to_owned();
        note.created_at = Utc::now();
        note
    }

    /// Review M1: the re-read holds the note (its body, newer than the thread the area opened
    /// on): the area closes and the pane says it was written.
    #[tokio::test]
    async fn a_reread_with_the_note_closes_the_area_and_says_it_was_written() {
        let shell = Shell::new();
        let (mut pane, _) = hedged(&shell, "Hi.\n\n").await;
        let mut thread = pane.notes.clone();
        thread.push(written(ITEM, "Hi.").await);
        pane.on_reply(&StoreReply::Notes(thread), &mut shell.ctx());
        assert!(pane.compose.is_none());
        assert!(!pane.captures_input());
        assert_eq!(pane.notice.as_deref(), Some(NOTE_WRITTEN));
        let text = drawn(43, 23, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(text.contains("the note was written;"), "{text}");
        assert!(text.contains("Hi."), "{text}");
        assert_eq!(text.lines().last().map(str::trim_end), Some(HINT), "{text}");
        assert!(shell.actions().is_empty(), "the re-read was the check");
        // The sentence stays until the next key.
        let _ = pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(pane.notice, None);
    }

    /// Review M1: the re-read holds no such note (one with the body from before the area opened
    /// does not count): the text stays, busy settles, and Ctrl+S sends it again.
    #[tokio::test]
    async fn a_reread_without_the_note_keeps_the_text_and_says_it_was_not_written() {
        let shell = Shell::new();
        let (mut pane, message) = hedged(&shell, "Hi.").await;
        let mut thread = pane.notes.clone();
        let mut older = written(ITEM, "Hi.").await;
        older.created_at = thread[0].created_at;
        thread.push(older);
        pane.on_reply(&StoreReply::Notes(thread), &mut shell.ctx());
        assert_eq!(
            compose(&pane).notice(),
            Some(not_written(&message).as_str())
        );
        assert_eq!(compose(&pane).body(), "Hi.");
        assert_eq!(compose(&pane).busy(), None);
        assert!(pane.notice.is_none());
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        let requests = shell.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(requests[0].starts_with("AddNote"), "{requests:?}");
    }

    /// Review M1: a thread of another item does not settle the hedge; this item's does.
    #[tokio::test]
    async fn a_reread_for_another_item_is_ignored() {
        let shell = Shell::new();
        let (mut pane, message) = hedged(&shell, "Hi.").await;
        let other = written(ids::HTUI_ANA_1, "Hi.").await;
        pane.on_reply(&StoreReply::Notes(vec![other]), &mut shell.ctx());
        assert_eq!(compose(&pane).busy(), Some(ADD_NOTE_NAME));
        assert_eq!(
            compose(&pane).notice(),
            Some(may_have_landed(&message, "thread").as_str())
        );
        pane.on_reply(&StoreReply::Notes(Vec::new()), &mut shell.ctx());
        assert_eq!(
            compose(&pane).notice(),
            Some(not_written(&message).as_str())
        );
    }

    /// Review M1: a re-read that fails too settles the area saying it could not check.
    #[tokio::test]
    async fn a_failed_reread_settles_saying_it_could_not_check() {
        let shell = Shell::new();
        let (mut pane, message) = hedged(&shell, "Hi.").await;
        assert_eq!(StoreRequest::Notes(ITEM).name(), NOTES_READ);
        pane.on_reply(
            &StoreReply::Failed {
                request: NOTES_READ,
                message: "store unreachable".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(compose(&pane).busy(), None);
        assert_eq!(
            compose(&pane).notice(),
            Some(could_not_check(&message, "thread").as_str())
        );
        assert_eq!(compose(&pane).body(), "Hi.");
    }

    /// `a`, the `NoteForm` answered, `text` pasted and saved, and the `AddNote` answered with
    /// `message`, a store failure: hedged, and the thread re-read is in flight.
    fn hedge(shell: &Shell, pane: &mut NotesTab, text: &str, message: &str) {
        let _ = pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        pane.on_reply(&StoreReply::NoteForm { item: ITEM }, &mut shell.ctx());
        let _ = pane.on_paste(text, &mut shell.ctx());
        let _ = pane.on_key(ctrl('s'), &mut shell.ctx());
        let _ = shell.requests();
        pane.on_reply(
            &StoreReply::Failed {
                request: ADD_NOTE_NAME,
                message: message.to_owned(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(shell.requests(), [sent(&StoreRequest::Notes(ITEM))]);
        assert_eq!(compose(pane).busy(), Some(ADD_NOTE_NAME));
    }

    /// Review round 1, finding 1: an `Unreachable` failure sent the worker to the mirror before it
    /// answered, so the re-read is the mirror's, which cannot hold a note whose answer was lost.
    /// Not finding it there says nothing; finding it (the refresher mirrored it first) still does.
    #[tokio::test]
    async fn after_an_unreachable_failure_the_mirror_cannot_say_it_was_not_written() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let message = StoreError::Unreachable("connection reset by peer".to_owned()).to_string();
        hedge(&shell, &mut pane, "Hi.", &message);
        let thread = pane.notes.clone();
        pane.on_reply(&StoreReply::Notes(thread), &mut shell.ctx());
        assert_eq!(compose(&pane).busy(), None);
        assert_eq!(
            compose(&pane).notice(),
            Some(could_not_check(&message, "thread").as_str())
        );
        assert_eq!(compose(&pane).body(), "Hi.");

        let mut pane = self::pane(&shell).await;
        hedge(&shell, &mut pane, "Hi.", &message);
        let mut thread = pane.notes.clone();
        thread.push(written(ITEM, "Hi.").await);
        pane.on_reply(&StoreReply::Notes(thread), &mut shell.ctx());
        assert!(pane.compose.is_none());
        assert_eq!(pane.notice.as_deref(), Some(NOTE_WRITTEN));
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
        let lines = pane.lines(0, &shell.theme);
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
        assert_eq!(pane.lines(0, theme), old);
    }

    /// A thread of `count` notes on FEAT-1, oldest first, each body wider than a narrow pane
    /// several times over; the newest ends in `THE END`.
    async fn long_thread(count: usize) -> Vec<Note> {
        let template = MemStore::demo()
            .notes(ITEM)
            .await
            .expect("the notes")
            .remove(0);
        (0..count)
            .map(|at| {
                let mut note = template.clone();
                note.created_at += chrono::Duration::minutes(i64::try_from(at).expect("small"));
                note.body = format!("note {at}: {}", "lorem ipsum dolor ".repeat(6));
                if at + 1 == count {
                    note.body.push_str("THE END");
                }
                note
            })
            .collect()
    }

    /// Review L1: the clamp counts the rows the thread wraps to at the last render's width, so
    /// the newest note of a long thread can be scrolled to in a narrow pane.
    #[tokio::test]
    async fn the_newest_note_of_a_long_thread_can_be_scrolled_to() {
        let shell = Shell::new();
        let mut pane = NotesTab::new();
        pane.on_item_change(Some(ITEM));
        pane.on_reply(&StoreReply::Notes(long_thread(3).await), &mut shell.ctx());
        let text = drawn(20, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(!text.contains("THE END"), "{text}");
        for _ in 0..20 {
            let _ = pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        }
        let text = drawn(20, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(text.contains("THE END"), "{text}");
        assert_eq!(text.lines().last().map(str::trim_end), Some(HINT), "{text}");
    }

    /// Review L1: after the user's own note lands, the re-read thread opens at its bottom, where
    /// the new note is; any other re-read opens at the top.
    #[tokio::test]
    async fn the_thread_opens_at_the_bottom_after_the_user_s_own_note() {
        let shell = Shell::new();
        let mut pane = pane(&shell).await;
        let _ = drawn(20, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        let _ = pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        pane.on_reply(&StoreReply::NoteForm { item: ITEM }, &mut shell.ctx());
        let _ = pane.on_paste("THE END", &mut shell.ctx());
        let _ = pane.on_key(ctrl('s'), &mut shell.ctx());
        pane.on_reply(&StoreReply::NoteAdded { item: ITEM }, &mut shell.ctx());
        pane.on_reply(&StoreReply::Notes(long_thread(3).await), &mut shell.ctx());
        let text = drawn(20, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(text.contains("THE END"), "{text}");
        assert!(!text.contains("note 0"), "{text}");

        // The next re-read is not the user's note: back to the top.
        pane.on_reply(&StoreReply::Notes(long_thread(3).await), &mut shell.ctx());
        let text = drawn(20, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(text.contains("note 0"), "{text}");
    }

    /// Review L2: a `\t` in a saved note draws as spaces to the next tab stop, as the compose
    /// area drew it; `ratatui` would drop it.
    #[tokio::test]
    async fn a_tab_in_a_note_draws_as_spaces_to_the_next_stop() {
        let shell = Shell::new();
        let mut pane = NotesTab::new();
        pane.on_item_change(Some(ITEM));
        let mut note = MemStore::demo()
            .notes(ITEM)
            .await
            .expect("the notes")
            .remove(0);
        note.body = "a\tb\n\tc\nabcd\te".to_owned();
        pane.on_reply(&StoreReply::Notes(vec![note]), &mut shell.ctx());
        let rows: Vec<String> = pane
            .lines(0, &shell.theme)
            .iter()
            .skip(1)
            .map(ToString::to_string)
            .collect();
        assert_eq!(rows, ["a   b", "    c", "abcd    e"]);
        let text = drawn(43, 8, |frame, area| pane.render(frame, area, &shell.ctx()));
        assert!(text.contains("a   b"), "{text}");
        assert!(!text.contains('\t'), "{text}");
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

    #[test]
    fn the_hint_fits_the_detail_pane() {
        assert!(cells::cell_width(HINT) <= 43, "{HINT}");
    }

    /// D11: with an item and no area, the last row is the hint, over a thread or an empty one;
    /// with no item there is none.
    #[tokio::test]
    async fn the_hint_is_drawn_under_the_thread() {
        let shell = Shell::new();
        let pane = pane(&shell).await;
        let text = drawn(43, 23, |frame, area| pane.render(frame, area, &shell.ctx()));
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[22].trim_end(), HINT, "{text}");
        assert_eq!(text.matches(HINT).count(), 1, "{text}");

        let mut empty = NotesTab::new();
        empty.on_item_change(Some(ITEM));
        empty.on_reply(&StoreReply::Notes(Vec::new()), &mut shell.ctx());
        let text = drawn(43, 23, |frame, area| {
            empty.render(frame, area, &shell.ctx())
        });
        assert!(text.contains("No notes for this item."), "{text}");
        assert_eq!(text.lines().last().map(str::trim_end), Some(HINT), "{text}");

        let mut none = NotesTab::new();
        none.on_item_change(None);
        let text = drawn(43, 23, |frame, area| none.render(frame, area, &shell.ctx()));
        assert!(!text.contains(HINT), "{text}");

        let composing = composing(&shell).await;
        let text = drawn(43, 23, |frame, area| {
            composing.render(frame, area, &shell.ctx());
        });
        assert!(!text.contains(HINT), "{text}");
    }
}
