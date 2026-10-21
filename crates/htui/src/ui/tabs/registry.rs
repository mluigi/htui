//! The [`Tab`] contract and the registry the shell holds.
//!
//! A new tab (MOD-2's Chat, MOD-12's Skills) is a `Box<dyn Tab>` handed to
//! [`TabRegistry::register`] plus one line in [`register_all`](crate::app::register_all): no
//! `match` arm anywhere in the event loop changes (PRD extensibility metric, plan D5).

use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Ctx, Handled, RevealTarget};
use crate::editor::ExternalEditOutcome;
use crate::keys::Stack;
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::shell::{Panes, TabLoc};
use crate::ui::tabs::settings::SectionId;
use crossterm::event::{KeyEvent, MouseEvent};

/// Stable identity of a tab, used for reply addressing and the reveal targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub &'static str);

/// What a tab says when a reveal arrives over a field the user is typing in (MOD-64 D252). "Search
/// again", not "open the hit again": the search overlay has closed, and a reopened one is empty
/// (review 7).
pub const CLOSE_THE_FIELD_FIRST: &str = "close the open field (Esc) first, then search again";

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
    /// A bracketed paste reached this tab, whole (MOD-22 review M-1).
    ///
    /// Only a view that is capturing input takes one, into its focused field; everything else
    /// answers `Pass` and the shell drops it — no key table ever sees a paste, so text pasted before
    /// a field is open never runs as commands. Defaulted to `Pass` so a tab with no field changes
    /// nothing.
    fn on_paste(&mut self, _text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// A reply addressed to this tab arrived and is not stale.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Whether this tab says a refused `request` (its [`StoreRequest::name`]) itself, so the shell
    /// keeps it off the status line (MOD-146 R1 L4). Asked before [`on_reply`](Tab::on_reply),
    /// which is handed the `Failed` reply either way. Defaulted to `false`: every other refusal
    /// is the status line's.
    fn absorbs_failure(&self, _request: &str) -> bool {
        false
    }
    /// Draws into the body region.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
    /// Moves this tab's own focus to `section` (MOD-15 M6 D7).
    ///
    /// `false` when the tab has no such section — which is every tab but Settings, and is what
    /// the default body says. Defaulted so that adding an addressable jump to one tab changes no
    /// other tab's file; this is the trait's first default, and the same move
    /// [`SettingsSection::captures_input`](crate::ui::tabs::settings::SettingsSection::captures_input)
    /// made one level down.
    fn focus_section(&mut self, _section: SectionId) -> bool {
        false
    }
    /// This tab's key stack in its current mode (MOD-67 D4). Every production tab returns one; the
    /// default `None` is the test doubles', which the shell treats as `Stack::BASE`.
    fn key_stack(&self) -> Option<Stack<'static>> {
        None
    }
    /// The stack of this tab's browse mode, the one the about panel labels its actions through
    /// (MOD-115 review): the tab's main actions are its browse keys, and a form, a composer or a
    /// filter being open must not blank them. Defaulted to the tab's current
    /// [`key_stack`](Tab::key_stack), else `Stack::BASE`; a tab whose current stack can differ
    /// from its browse stack overrides it.
    fn browse_stack(&self) -> Stack<'static> {
        self.key_stack().unwrap_or(Stack::BASE)
    }
    /// The keys of this tab's focused mode for the shell's hint row (MOD-67 M6 D10, design review
    /// finding 3): the mode's hint spec rendered through its own stack, so a rebound key shows its
    /// new chord. `width` is the row's; a tab that composes two hints keeps the second only when
    /// both fit. `None`: no row (the test doubles; a tab with nothing to say). Defaulted.
    fn key_hint(&self, _width: u16, _ctx: &Ctx<'_>) -> Option<Line<'static>> {
        None
    }
    /// The `$EDITOR` handoff this tab asked for came back (MOD-9 D10). Defaulted, the trait's
    /// second default after `focus_section`, so no other tab changes.
    fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}
    /// Selects `target` (MOD-64 D235): unfold or clear what hides it, move the cursor, read its
    /// detail; or, when this tab has not loaded it, keep it until the next list reply. `false` when
    /// this tab shows no such thing — every tab but Backlog and Requirements, which is what the
    /// default says. The trait's third default, after `focus_section` and `on_external_edit`.
    fn reveal(&mut self, _target: &RevealTarget, _ctx: &mut Ctx<'_>) -> bool {
        false
    }

    /// The shell's once-a-second refresh reached the active tab (MOD-41 plan D16). Defaulted, the
    /// trait's third default after `focus_section` and `on_external_edit`, so no other tab
    /// changes. A view trait, not a store trait: the no-default rule does not apply.
    fn on_refresh(&mut self, _ctx: &mut Ctx<'_>) {}

    /// Whether this tab's own gesture wants the terminal's mouse right now (MOD-71 D1). Capture
    /// takes the terminal's own text selection away, so the event loop turns it on only while
    /// the active tab says yes (or, MOD-115 M2 MD8, is a list + detail tab, for the shell's pane
    /// clicks and seam drags) and nothing is drawn over it. Defaulted to `false`, so no other tab
    /// changes: only the Backlog answers, for its Runs pane's flow view.
    ///
    /// MOD-74 review L1: a loss used to be only the shell's on-to-off capture edge. Since the
    /// tabs with a list and a detail keep capture on across a tab switch, the shell tracks the
    /// tab whose own gesture holds the mouse and tells every tab when it changes too
    /// (`App::mouse_capture`), so the review-L3 stale anchor stays gone.
    fn wants_mouse(&self) -> bool {
        false
    }
    /// A mouse event reached this tab (MOD-71 D4), only while [`wants_mouse`](Tab::wants_mouse)
    /// says so (the shell's own gestures, MOD-115 M2 §7, come first). `Consumed` asks for a redraw; `Pass` is the drop and costs nothing — no key table
    /// reads a mouse event. Defaulted to `Pass`, so no other tab changes.
    fn on_mouse(&mut self, _mouse: MouseEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// MOD-74 D1, D3: mouse capture went off while this tab may hold a gesture: an overlay, `?`, a
    /// form, a sub-tab or tab switch, or an `$EDITOR` handoff. Told to every registered tab, active
    /// or not, once per on-to-off edge, and (MOD-115 M2 MD8) once whenever the tab whose own
    /// gesture held the mouse changes while capture stays on. Defaulted to nothing, so no other
    /// tab changes.
    fn on_mouse_lost(&mut self) {}

    /// The panes this tab has (MOD-115 M2 D2): one (the default: Chat, Settings, the test
    /// doubles) or a list and a detail with the tab's default layout. A list + detail tab splits
    /// its body with [`Ctx::split`](crate::app::Ctx::split), where the shell's layout for it
    /// applies; on a single-pane tab the pane and layout keys do nothing.
    fn panes(&self) -> Panes {
        Panes::Single
    }

    /// Where this tab is (MOD-115 M2 D5): its sub-tab and its opaque cursor, for the history.
    /// Defaulted to nowhere in particular, so only the tab itself is recorded.
    fn location(&self) -> TabLoc {
        TabLoc::default()
    }

    /// Goes back to `loc`, a [`location`](Tab::location) this tab gave (MOD-115 M2 D5): selects
    /// the sub-tab and moves the cursor. `false` when nothing could be restored, which is what
    /// the default says.
    fn restore(&mut self, _loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
        false
    }
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

    /// The title of the tab with this id; `None` when it is not registered.
    #[must_use]
    pub fn title(&self, id: TabId) -> Option<&str> {
        self.tabs.iter().find(|t| t.id() == id).map(|t| t.title())
    }

    /// The header's tab chips' input: every tab's id and title in registration order.
    #[must_use]
    pub fn titles(&self) -> Vec<(TabId, &str)> {
        self.tabs.iter().map(|t| (t.id(), t.title())).collect()
    }
}
