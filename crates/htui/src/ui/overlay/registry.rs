//! The [`Overlay`] contract, the factory registry and the stack of open overlays.
//!
//! An overlay is created on demand (`Action::Overlay(OverlayAction::Open(id))`), so MOD-13's
//! filter overlay and MOD-15's editors are one `register` line plus one file each.

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crossterm::event::KeyEvent;

/// Stable identity of an overlay, also the key of its [`KeyScope`](crate::keymap::KeyScope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayId(pub &'static str);

impl OverlayId {
    /// Wildcard scope: a binding registered under it resolves for every overlay.
    ///
    /// The default `Esc` -> `OverlayAction::Close` binding has to exist before any concrete
    /// overlay is registered, and [`KeyScope::Overlay`](crate::keymap::KeyScope::Overlay) carries
    /// a concrete id, so the keymap looks the exact id up first and falls back to this one.
    pub const ANY: Self = Self("*");
}

impl core::fmt::Display for OverlayId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// A transient view drawn over the frame: the workspace switcher (T5), MOD-13's filters.
///
/// Like a [`Tab`](crate::ui::tabs::Tab) it holds no store handle and no channel.
pub trait Overlay {
    /// Stable identity, used for key scopes and reply addressing.
    fn id(&self) -> OverlayId;
    /// Title of the overlay's block.
    fn title(&self) -> &str;
    /// A modal overlay swallows every key it does not handle, so the tab below never sees it.
    fn is_modal(&self) -> bool;
    /// Requests the shell issues when this overlay is pushed.
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;
    /// A key reached this overlay: it is the first stop of the propagation chain.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// A reply addressed to this overlay arrived and is not stale.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws over the whole frame. Use [`centered`](crate::ui::layout::centered) for the box.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}

type Factory = Box<dyn Fn() -> Box<dyn Overlay>>;

/// How an [`OverlayId`] becomes an overlay. Registered once at startup, called on every open.
#[derive(Default)]
pub struct OverlayRegistry {
    factories: Vec<(OverlayId, Factory)>,
}

impl core::fmt::Debug for OverlayRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayRegistry")
            .field(
                "ids",
                &self.factories.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl OverlayRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) the factory of an overlay.
    pub fn register(&mut self, id: OverlayId, f: impl Fn() -> Box<dyn Overlay> + 'static) {
        self.factories.retain(|(known, _)| *known != id);
        self.factories.push((id, Box::new(f)));
    }

    /// Builds a fresh overlay, or `None` when nothing is registered under `id`.
    #[must_use]
    pub fn create(&self, id: OverlayId) -> Option<Box<dyn Overlay>> {
        self.factories
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, f)| f())
    }
}

/// The open overlays, bottom first. Only the top one receives keys (blueprint C.4).
#[derive(Default)]
pub struct OverlayStack {
    open: Vec<Box<dyn Overlay>>,
}

impl core::fmt::Debug for OverlayStack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OverlayStack")
            .field(
                "open",
                &self.open.iter().map(|o| o.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl OverlayStack {
    /// An empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes an overlay on top.
    pub fn push(&mut self, overlay: Box<dyn Overlay>) {
        self.open.push(overlay);
    }

    /// Closes the topmost overlay, if any.
    pub fn pop(&mut self) {
        self.open.pop();
    }

    /// Closes every overlay: what a scope change does.
    pub fn clear(&mut self) {
        self.open.clear();
    }

    /// The topmost overlay.
    #[must_use]
    pub fn top(&self) -> Option<&dyn Overlay> {
        self.open.last().map(AsRef::as_ref)
    }

    /// The topmost overlay mutably: the first stop of a key.
    pub fn top_mut(&mut self) -> Option<&mut (dyn Overlay + 'static)> {
        self.open.last_mut().map(AsMut::as_mut)
    }

    /// An open overlay by id, for reply addressing. `None` once it has been popped.
    pub fn by_id_mut(&mut self, id: OverlayId) -> Option<&mut (dyn Overlay + 'static)> {
        self.open
            .iter_mut()
            .find(|o| o.id() == id)
            .map(AsMut::as_mut)
    }

    /// Whether nothing is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// How many overlays are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// Every open overlay, bottom-up: render order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Overlay> {
        self.open.iter().map(AsRef::as_ref)
    }
}
