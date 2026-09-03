//! The [`Tab`] contract and the registry the shell holds.
//!
//! A new tab (MOD-2's Chat, MOD-12's Skills) is a `Box<dyn Tab>` handed to
//! [`TabRegistry::register`] plus one line in [`register_all`](crate::app::register_all): no
//! `match` arm anywhere in the event loop changes (PRD extensibility metric, plan D5).

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crossterm::event::KeyEvent;

/// Stable identity of a tab. The string is also the key of its [`KeyScope`](crate::keymap::KeyScope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub &'static str);

impl core::fmt::Display for TabId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// One top-level screen.
///
/// A tab never holds a store handle: it asks through [`Ctx::request`](crate::app::Ctx::request)
/// and is handed the answer in [`Tab::on_reply`] (`R-NF-3`).
pub trait Tab {
    /// Stable identity, used for key scopes and reply addressing.
    fn id(&self) -> TabId;
    /// Label in the tab strip.
    fn title(&self) -> &str;
    /// Requests the shell should issue for this tab. Called on activation and after every scope
    /// change, never during render.
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;
    /// The scope changed: drop every cached row, the ids in them belong to another workspace.
    fn on_scope_change(&mut self, scope: &Scope);
    /// A key reached this tab (propagation order: `docs` blueprint C.4).
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// A reply addressed to this tab arrived and is not stale.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws into the body region.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

/// Every registered tab, in registration order, plus which one is active.
#[derive(Default)]
pub struct TabRegistry {
    tabs: Vec<Box<dyn Tab>>,
    active: usize,
}

impl core::fmt::Debug for TabRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabRegistry")
            .field(
                "tabs",
                &self.tabs.iter().map(|t| t.id()).collect::<Vec<_>>(),
            )
            .field("active", &self.active)
            .finish()
    }
}

impl TabRegistry {
    /// An empty registry. The shell renders an empty body until something is registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a tab. Registration order is tab-strip order and `1`..`9` order.
    pub fn register(&mut self, tab: Box<dyn Tab>) {
        self.tabs.push(tab);
    }

    /// Whether no tab is registered at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// How many tabs are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// The active tab, or `None` while the registry is empty.
    #[must_use]
    pub fn active(&self) -> Option<&dyn Tab> {
        self.tabs.get(self.active).map(AsRef::as_ref)
    }

    /// The active tab mutably, or `None` while the registry is empty.
    pub fn active_mut(&mut self) -> Option<&mut (dyn Tab + 'static)> {
        self.tabs.get_mut(self.active).map(AsMut::as_mut)
    }

    /// Id of the active tab, or `None` while the registry is empty.
    #[must_use]
    pub fn active_id(&self) -> Option<TabId> {
        self.active().map(Tab::id)
    }

    /// A registered tab by id.
    pub fn by_id_mut(&mut self, id: TabId) -> Option<&mut (dyn Tab + 'static)> {
        self.tabs
            .iter_mut()
            .find(|t| t.id() == id)
            .map(AsMut::as_mut)
    }

    /// Every registered tab, for scope-change fan-out.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Tab>> {
        self.tabs.iter_mut()
    }

    /// Activates the tab at `idx`. `false` (and no change) when the index is out of range.
    pub fn select(&mut self, idx: usize) -> bool {
        if idx < self.tabs.len() {
            self.active = idx;
            true
        } else {
            false
        }
    }

    /// Activates the tab with this id. `false` when it is not registered.
    pub fn focus(&mut self, id: TabId) -> bool {
        match self.tabs.iter().position(|t| t.id() == id) {
            Some(idx) => self.select(idx),
            None => false,
        }
    }

    /// Activates the next tab, wrapping.
    pub fn next(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    /// Activates the previous tab, wrapping.
    pub fn prev(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    /// The tab strip's input: every tab's id and title in registration order.
    #[must_use]
    pub fn titles(&self) -> Vec<(TabId, &str)> {
        self.tabs.iter().map(|t| (t.id(), t.title())).collect()
    }
}

/// Draws the tab strip: `1 Backlog  2 Skills  3 Settings`, the active one accented.
pub fn render_strip(frame: &mut Frame<'_>, area: Rect, registry: &TabRegistry, theme: &Theme) {
    let active = registry.active_id();
    let mut spans: Vec<Span<'_>> = Vec::new();
    for (idx, (id, title)) in registry.titles().into_iter().enumerate() {
        let style: Style = if Some(id) == active {
            theme.accent
        } else {
            theme.dim
        };
        spans.push(Span::styled(format!(" {} {title} ", idx + 1), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
