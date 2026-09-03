//! The Notes sub-tab: the item's notes as a chronological thread.

use htui_core::model::{ItemId, Note};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, STAMP, Scroll, message};
use crossterm::event::KeyEvent;

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
    /// First visible line.
    scroll: Scroll,
}

impl NotesTab {
    /// Identity of the Notes sub-tab.
    pub const ID: DetailId = DetailId("notes");

    /// A sub-tab with no notes yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread, one stamp line and one body line per note, blank-separated.
    fn lines(&self, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for note in &self.notes {
            if !out.is_empty() {
                out.push(Line::raw(""));
            }
            out.push(Line::styled(
                note.created_at.format(STAMP).to_string(),
                ctx.theme.dim,
            ));
            out.push(Line::styled(note.body.clone(), ctx.theme.base));
        }
        out
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
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.notes.len() * 3)
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Notes(notes) = reply {
            self.notes = notes.clone();
            self.scroll.reset();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if self.notes.is_empty() {
            message(frame, area, "No notes for this item.", ctx.theme);
            return;
        }
        frame.render_widget(
            Paragraph::new(Text::from(self.lines(ctx)))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            area,
        );
    }
}
