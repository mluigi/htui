//! The Body sub-tab: the item's head fields and its markdown body.

use std::borrow::Cow;

use htui_core::model::{Item, ItemId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::KeyEvent;

/// How many lines the head block takes: title, key line, meta line, blank.
const HEAD: u16 = 4;

/// Title, key, kind, status, tags, priority, version and the body as a wrapped paragraph.
///
/// The body is rendered raw, not parsed: MOD-1 shows markdown as text, and nothing downstream
/// depends on that staying true.
#[derive(Debug, Default)]
pub struct BodyTab {
    /// The `Item` reply for the selected item.
    item: Option<Item>,
    /// Scroll of the body paragraph, not of the head block.
    scroll: Scroll,
}

impl BodyTab {
    /// Identity of the Body sub-tab.
    pub const ID: DetailId = DetailId("body");

    /// A sub-tab with no item yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The body's lines, which is what the scroll clamps against.
    fn len(&self) -> usize {
        self.item
            .as_ref()
            .map_or(0, |item| item.body.lines().count())
    }
}

impl DetailTab for BodyTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Body"
    }

    fn on_item_change(&mut self, _item: Option<ItemId>) {
        self.item = None;
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.scroll.on_key(key, self.len())
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Item(item) = reply {
            self.item = (**item).clone();
            self.scroll.reset();
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let Some(item) = self.item.as_ref() else {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        };

        let tags: Cow<'_, str> = if item.required_tags.is_empty() {
            Cow::Borrowed("no tags")
        } else {
            Cow::Owned(item.required_tags.join(", "))
        };
        let head = Text::from(vec![
            Line::styled(item.title.as_str(), ctx.theme.title),
            Line::from(vec![
                Span::styled(item.key.as_str(), ctx.theme.accent),
                Span::raw("  "),
                Span::styled(item.key_prefix.as_str(), ctx.theme.dim),
                Span::raw("  "),
                Span::styled(item.status.as_str(), ctx.theme.status_style(item.status)),
            ]),
            Line::styled(
                format!(
                    "{tags} \u{b7} priority {} \u{b7} version {}",
                    item.priority, item.version
                ),
                ctx.theme.dim,
            ),
            Line::raw(""),
        ]);

        let [head_area, body_area] =
            Layout::vertical([Constraint::Length(HEAD), Constraint::Min(0)]).areas(area);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), head_area);

        if item.body.is_empty() {
            message(frame, body_area, "No body for this item.", ctx.theme);
            return;
        }
        frame.render_widget(
            Paragraph::new(Text::raw(item.body.as_str()))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll.offset(), 0)),
            body_area,
        );
    }
}
