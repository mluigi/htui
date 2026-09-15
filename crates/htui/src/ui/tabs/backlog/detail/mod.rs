//! The detail pane: the [`DetailTab`] contract, the registry that holds the six sub-tabs, and
//! the scroll state they share.
//!
//! Sub-tabs are a registry for the same reason tabs are (plan D5): MOD-4's approve/reject and
//! MOD-2's "promote to chat" are `DetailTab::on_key` bodies inside their own file plus one
//! `register` line, never a `match` arm in the Backlog tab. In MOD-1 every sub-tab is read-only
//! and answers to nothing but scrolling (plan D11).

pub mod body;
pub mod documents;
pub mod graph;
pub mod notes;
pub mod prompt;
pub mod runs;

use htui_core::model::ItemId;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crossterm::event::{KeyCode, KeyEvent};

pub use body::BodyTab;
pub use documents::DocumentsTab;
pub use graph::GraphTab;
pub use notes::NotesTab;
pub use prompt::PromptTab;
pub use runs::RunsTab;

/// How many rows `PageDown` / `PageUp` move.
const PAGE: isize = 10;

/// `strftime` format every timestamp in the detail pane is rendered with: the pane is 43 columns
/// wide at 100x30 and the year is the same on every row that would fit next to it.
pub const STAMP: &str = "%m-%d %H:%M";

/// Stable identity of a detail sub-tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DetailId(pub &'static str);

impl core::fmt::Display for DetailId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// One view of the selected item.
///
/// A sub-tab holds no store handle either (`R-NF-3`): the Backlog tab issues the six reads when
/// the selection changes and hands every reply to every sub-tab, which keeps each one to "here is
/// my variant, here is how it draws". MOD-2 milestone 9's [`PromptTab`] is the worked example of
/// how far that goes: its reply is assembled by a task the store worker spawned, and the sub-tab
/// still only knows how to draw it.
pub trait DetailTab {
    /// Stable identity.
    fn id(&self) -> DetailId;
    /// Label in the sub-tab strip.
    fn title(&self) -> &str;
    /// The selected item changed: drop the cached rows, they belong to the previous item.
    ///
    /// The mirror of [`Tab::on_scope_change`](crate::ui::tabs::Tab::on_scope_change), one level
    /// down.
    fn on_item_change(&mut self, item: Option<ItemId>);
    /// A key the Backlog tab did not use for navigation.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// A reply addressed to the Backlog tab. Sub-tabs that do not care ignore it.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws into the detail pane, below the sub-tab strip.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

/// The sub-tabs of the detail pane, in registration order, plus which one is active.
#[derive(Default)]
pub struct DetailRegistry {
    /// The registered sub-tabs, in strip order.
    tabs: Vec<Box<dyn DetailTab>>,
    /// Index of the active one.
    active: usize,
}

impl core::fmt::Debug for DetailRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DetailRegistry")
            .field(
                "tabs",
                &self.tabs.iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            )
            .field("active", &self.active)
            .finish()
    }
}

impl DetailRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a sub-tab. Registration order is strip order and cycling order.
    pub fn register(&mut self, tab: Box<dyn DetailTab>) {
        self.tabs.push(tab);
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// How many sub-tabs are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// The active sub-tab, or `None` while the registry is empty.
    #[must_use]
    pub fn active(&self) -> Option<&dyn DetailTab> {
        self.tabs.get(self.active).map(AsRef::as_ref)
    }

    /// Id of the active sub-tab, or `None` while the registry is empty.
    #[must_use]
    pub fn active_id(&self) -> Option<DetailId> {
        self.active().map(DetailTab::id)
    }

    /// Activates the sub-tab at `idx`. `false` (and no change) when the index is out of range.
    pub fn select(&mut self, idx: usize) -> bool {
        if idx < self.tabs.len() {
            self.active = idx;
            true
        } else {
            false
        }
    }

    /// Activates the next sub-tab, wrapping (`l`, `]`, `Right`).
    pub fn cycle_next(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    /// Activates the previous sub-tab, wrapping (`h`, `[`, `Left`).
    pub fn cycle_prev(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    /// Every sub-tab's id and title, in strip order.
    #[must_use]
    pub fn titles(&self) -> Vec<(DetailId, &str)> {
        self.tabs
            .iter()
            .map(|tab| (tab.id(), tab.title()))
            .collect()
    }

    /// Tells every sub-tab the selection changed.
    pub fn on_item_change(&mut self, item: Option<ItemId>) {
        for tab in &mut self.tabs {
            tab.on_item_change(item);
        }
    }

    /// Hands a reply to every sub-tab: the six reads are issued together, so a sub-tab is
    /// populated before it is ever looked at.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        for tab in &mut self.tabs {
            tab.on_reply(reply, ctx);
        }
    }

    /// Offers a key to the active sub-tab.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.on_key(key, ctx),
            None => Handled::Pass,
        }
    }
}

/// Draws the detail pane: a border titled with the selected item's key, the sub-tab strip, and
/// the active sub-tab below it.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    registry: &DetailRegistry,
    key: Option<&str>,
    ctx: &Ctx<'_>,
) {
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" {} ", key.unwrap_or("Detail")));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [strip, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    render_strip(frame, strip, registry, ctx.theme);
    match registry.active() {
        Some(tab) => tab.render(frame, content, ctx),
        None => message(
            frame,
            content,
            "No detail sub-tab is registered.",
            ctx.theme,
        ),
    }
}

/// Draws the sub-tab strip: `Body  Runs  Graph  Documents  Notes  Prompt`, the active one
/// accented.
pub fn render_strip(frame: &mut Frame<'_>, area: Rect, registry: &DetailRegistry, theme: &Theme) {
    let active = registry.active_id();
    let spans: Vec<Span<'_>> = registry
        .titles()
        .into_iter()
        .map(|(id, title)| {
            let style: Style = if Some(id) == active {
                theme.accent
            } else {
                theme.dim
            };
            Span::styled(format!(" {title} "), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The one line a sub-tab renders instead of a blank pane (plan D11).
pub fn message(frame: &mut Frame<'_>, area: Rect, text: &str, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Line::styled(text.to_owned(), theme.dim)).wrap(Wrap { trim: true }),
        area,
    );
}

/// Vertical scroll shared by every sub-tab.
///
/// The clamp is against the number of *unwrapped* rows, which is a lower bound on the number of
/// rendered ones, so the last page can never be scrolled off the top even though the wrapped
/// height is only known at render time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    /// First rendered row.
    offset: u16,
}

impl Scroll {
    /// The current offset, for `Paragraph::scroll` or for skipping table rows.
    #[must_use]
    pub const fn offset(self) -> u16 {
        self.offset
    }

    /// The offset as a row index.
    #[must_use]
    pub const fn skip(self) -> usize {
        self.offset as usize
    }

    /// Back to the top: what a new item or a new reply does.
    pub const fn reset(&mut self) {
        self.offset = 0;
    }

    /// `J` / `K` scroll one row, `PageDown` / `PageUp` ten, everything else passes through.
    pub fn on_key(&mut self, key: KeyEvent, len: usize) -> Handled {
        let step: isize = match key.code {
            KeyCode::Char('J') => 1,
            KeyCode::Char('K') => -1,
            KeyCode::PageDown => PAGE,
            KeyCode::PageUp => -PAGE,
            _ => return Handled::Pass,
        };
        let max = u16::try_from(len.saturating_sub(1)).unwrap_or(u16::MAX);
        let next = usize::from(self.offset).saturating_add_signed(step);
        self.offset = u16::try_from(next).unwrap_or(u16::MAX).min(max);
        Handled::Consumed
    }
}
