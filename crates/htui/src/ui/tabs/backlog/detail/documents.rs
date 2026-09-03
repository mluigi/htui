//! The Documents sub-tab: the item's documents, without their bodies.

use htui_core::model::{DocumentHead, ItemId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::KeyEvent;

/// Written by a run step.
const BY_STEP: &str = "step";

/// Written by a human.
const BY_HAND: &str = "hand";

/// The `Documents` reply as a table of kind, version, provenance and title.
#[derive(Debug, Default)]
pub struct DocumentsTab {
    /// The documents of the selected item, by kind then version.
    documents: Vec<DocumentHead>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// First visible document.
    scroll: Scroll,
}

impl DocumentsTab {
    /// Identity of the Documents sub-tab.
    pub const ID: DetailId = DetailId("documents");

    /// A sub-tab with no documents yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl DetailTab for DocumentsTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Documents"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.documents.clear();
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.documents.len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Documents(documents) = reply {
            self.documents = documents.clone();
            self.scroll.reset();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        if self.documents.is_empty() {
            message(frame, area, "No documents for this item.", ctx.theme);
            return;
        }

        let rows = self
            .documents
            .iter()
            .skip(self.scroll.skip())
            .map(|document| {
                let by = if document.produced_by_step_id.is_some() {
                    BY_STEP
                } else {
                    BY_HAND
                };
                Row::new(vec![
                    Cell::from(Line::styled(document.kind.clone(), ctx.theme.accent)),
                    Cell::from(Line::styled(
                        format!("v{}", document.version),
                        ctx.theme.base,
                    )),
                    Cell::from(Line::styled(by, ctx.theme.dim)),
                    Cell::from(Line::styled(document.title.clone(), ctx.theme.base)),
                ])
            })
            .collect::<Vec<_>>();

        let header = Row::new(vec![
            Cell::from("kind"),
            Cell::from("ver"),
            Cell::from("by"),
            Cell::from("title"),
        ])
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Min(0),
            ],
        )
        .header(header)
        .column_spacing(1);
        frame.render_widget(table, area);
    }
}
