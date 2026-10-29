//! The detail pane: the [`DetailTab`] contract and the registry that holds the seven sub-tabs.
//! The scroll state they share is the kit's ([`Scroll`](crate::ui::kit::scroll::Scroll), MOD-115
//! M4 T0).
//!
//! Sub-tabs are a registry for the same reason tabs are (plan D5): MOD-4's approve/reject and
//! MOD-2's "promote to chat" are `DetailTab::on_key` bodies inside their own file plus one
//! `register` line, never a `match` arm in the Backlog tab. In MOD-1 every sub-tab is read-only
//! and answers to nothing but scrolling (plan D11). MOD-39 plan P12 adds the seventh, [`ReqsTab`],
//! and shortens "Documents" to "Docs" so the strip keeps its 40 columns.
//!
//! MOD-13 milestone 5: Notes and Docs write, each through a [`compose`] area of its own (D6), and
//! an `$EDITOR` outcome comes back to the active sub-tab (D7).
//!
//! MOD-115 M4 E4: the pane is a kit pane titled `KEY · title`, the sub-tabs its chips (MOD-81: no
//! strip row clipping `Notes Prompt Reqs`); a sub-tab draws into the whole inner rect.

pub mod body;
pub mod compose;
pub mod documents;
pub mod graph;
pub mod notes;
pub mod prompt;
pub mod requirements;
pub mod runs;

use htui_core::model::{ItemId, ItemSummary, RunId, StepId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::keys::{Keys, Stack, views};
use crate::store_worker::StoreReply;
use crate::ui::Theme;
use crate::ui::kit::pane::Pane;
use crate::ui::kit::scroll::Step;
use crate::ui::tabs::backlog::Chrome;
use crossterm::event::{KeyEvent, MouseEvent};

pub use body::BodyTab;
pub use documents::DocumentsTab;
pub use graph::GraphTab;
pub use notes::NotesTab;
pub use prompt::PromptTab;
pub use requirements::ReqsTab;
pub use runs::RunsTab;

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
/// A sub-tab holds no store handle either (`R-NF-3`): the Backlog tab issues the seven reads when
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
    /// A key the Backlog tab did not use for navigation, or every key while
    /// [`captures_input`](DetailTab::captures_input) says so.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// A bracketed paste, offered only while [`captures_input`](DetailTab::captures_input) is true
    /// (MOD-22 review M-1): a typed field takes it; a modal answer is not a field, and the
    /// default `Pass` drops it so its letters answer nothing.
    fn on_paste(&mut self, _text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// Whether this sub-tab is taking typed text or a modal answer right now, so the Backlog tab
    /// must hand it every key before its own navigation (MOD-4 plan OQ-7). Derived from a mode,
    /// never a flag.
    ///
    /// A capturing sub-tab returns `Pass` for what its modal stack admits (a `CONTROL` or `ALT`
    /// chord, a function key such as `F1`; MOD-67 M5), so `ctrl-c` still quits and `F1` opens
    /// help, and `Consumed` for everything else: `q`, a digit and `Tab` cannot leave a typed field.
    fn captures_input(&self) -> bool {
        false
    }
    /// A reply addressed to the Backlog tab. Sub-tabs that do not care ignore it.
    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws into the detail pane's inner rect (the chips are on its border, MOD-115 M4 E4).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
    /// Whether this sub-tab shows a run that is still active (MOD-41 plan D16): what the Backlog
    /// tab's `Runs` poll asks before it re-reads. Only [`RunsTab`] shows runs.
    fn has_active_run(&self) -> bool {
        false
    }
    /// MOD-13 milestone 5 D7: an `$EDITOR` outcome the Backlog had no item form for. Only the
    /// sub-tab that emitted `Action::EditExternally` acts on it; the default drops it.
    fn on_external_edit(&mut self, _outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {}
    /// Whether this sub-tab wants the terminal's mouse right now (MOD-71 D1): what the Backlog
    /// tab's [`Tab::wants_mouse`](crate::ui::tabs::Tab::wants_mouse) asks of the active sub-tab.
    /// Only [`RunsTab`] answers yes, in its flow view while browsing.
    fn wants_mouse(&self) -> bool {
        false
    }
    /// A mouse event, offered only while [`wants_mouse`](DetailTab::wants_mouse) is true (MOD-71
    /// D4). The default `Pass` drops it.
    fn on_mouse(&mut self, _mouse: MouseEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// MOD-115 M2 §7: a list motion while the detail is focused. A pane that reads as text
    /// (Body, Notes, Prompt) scrolls; one with a row cursor (Runs, Graph, Docs, Reqs) moves it,
    /// as its own `J`/`K` do. The default `Pass` takes no step, and the Backlog still keeps its
    /// list cursor where it is: only motion follows the focus.
    fn step(&mut self, _step: Step, _ctx: &Ctx<'_>) -> Handled {
        Handled::Pass
    }
    /// Mouse capture went off above the pane (MOD-74 D3): the Backlog tab hands every sub-tab the
    /// loss, active or not. Only [`RunsTab`] holds a gesture; the default does nothing.
    ///
    /// That loss is the on-to-off capture edge, and it means "the gesture holder lost the
    /// pointer" only while [`RunsTab`] is the sole view that wants the mouse: a second capturing
    /// view must also handle a switch between two capturing views (e.g. by tracking the capture
    /// holder), or the review-L3 stale anchor returns.
    fn on_mouse_lost(&mut self) {}

    /// MOD-69 plan D8 (blueprint A-5): a reveal asks for this run and step under the cursor. Only
    /// [`RunsTab`] answers; the default ignores it. Called after the item change, so it survives
    /// `on_item_change`'s reset.
    fn focus(&mut self, _run: Option<RunId>, _step: Option<StepId>, _ctx: &Ctx<'_>) {}

    /// The stack of this sub-tab's current mode (MOD-67 M5 D4): what the Backlog tab resolves
    /// keys through and what the status line and `?` box show while it is the active pane. The
    /// default is Body's browse stack, [`views::BACKLOG_BODY`]: the pane's own verbs none, the
    /// Backlog's and the list's all. A pane with modes or verbs of its own returns its mode's
    /// stack.
    fn key_stack(&self) -> Stack<'static> {
        views::BACKLOG_BODY
    }
    /// The keys of this pane's current mode for the Backlog's part of the shell's hint row
    /// (MOD-67 M6 T2), through [`key_stack`](DetailTab::key_stack). Defaulted to `None` (Body and
    /// Prompt, which have no key line of their own).
    fn key_hint(&self, _keys: &Keys, _theme: &Theme) -> Option<Line<'static>> {
        None
    }
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

    /// Index of the active sub-tab in strip order: the active chip (MOD-115 M4 E4).
    #[must_use]
    pub const fn active_index(&self) -> usize {
        self.active
    }

    /// Id of the active sub-tab, or `None` while the registry is empty.
    #[must_use]
    pub fn active_id(&self) -> Option<DetailId> {
        self.active().map(DetailTab::id)
    }

    /// The active sub-tab's stack, or [`views::BACKLOG_BODY`] while the registry is empty.
    #[must_use]
    pub fn key_stack(&self) -> Stack<'static> {
        self.active()
            .map_or(views::BACKLOG_BODY, DetailTab::key_stack)
    }

    /// The active sub-tab's [`DetailTab::key_hint`], `None` while the registry is empty.
    #[must_use]
    pub fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        self.active().and_then(|tab| tab.key_hint(keys, theme))
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

    /// Activates the sub-tab registered under `id` (MOD-14 D8: `m` opens the Graph). `false` (and no
    /// change) when nothing is registered under it.
    pub fn select_id(&mut self, id: DetailId) -> bool {
        match self.tabs.iter().position(|tab| tab.id() == id) {
            Some(idx) => self.select(idx),
            None => false,
        }
    }

    /// Hands a reveal's run and step to the sub-tab registered under `id` (MOD-69 plan D8);
    /// nothing when nothing is registered under it. No answer: the one caller has no use for it.
    pub fn focus(&mut self, id: DetailId, run: Option<RunId>, step: Option<StepId>, ctx: &Ctx<'_>) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id() == id) {
            tab.focus(run, step, ctx);
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

    /// Hands a reply to every sub-tab: the seven reads are issued together, so a sub-tab is
    /// populated before it is ever looked at.
    ///
    /// MOD-13 milestone 5 E4: a compose read's answer (`NoteForm`, `DocumentForm`) opens an area,
    /// so it goes to the active sub-tab alone. If the strip moved while the read was in flight, no
    /// area opens on a hidden pane that `captures_input` would not see and the item form could
    /// open over.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        if matches!(
            reply,
            StoreReply::NoteForm { .. } | StoreReply::DocumentForm(_)
        ) {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.on_reply(reply, ctx);
            }
            return;
        }
        for tab in &mut self.tabs {
            tab.on_reply(reply, ctx);
        }
    }

    /// Whether any sub-tab shows a run that is still active (MOD-41 plan D16), whichever is the
    /// one on screen: the poll keeps a hidden Runs pane current too.
    #[must_use]
    pub fn has_active_run(&self) -> bool {
        self.tabs.iter().any(|tab| tab.has_active_run())
    }

    /// Whether the active sub-tab is taking every key (MOD-4 plan OQ-7, blueprint D201).
    #[must_use]
    pub fn captures_input(&self) -> bool {
        self.active().is_some_and(DetailTab::captures_input)
    }

    /// Offers a bracketed paste to the active sub-tab while it captures input (MOD-22 review M-1).
    pub fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        match self.tabs.get_mut(self.active) {
            Some(tab) if tab.captures_input() => tab.on_paste(text, ctx),
            _ => Handled::Pass,
        }
    }

    /// Whether the active sub-tab wants the mouse (MOD-71 D1). The active one only, unlike
    /// [`has_active_run`](Self::has_active_run): a hidden pane is not on screen to be clicked.
    #[must_use]
    pub fn wants_mouse(&self) -> bool {
        self.active().is_some_and(DetailTab::wants_mouse)
    }

    /// Offers a mouse event to the active sub-tab while it wants one (MOD-71 D4), as
    /// [`on_paste`](Self::on_paste) offers a paste.
    pub fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.tabs.get_mut(self.active) {
            Some(tab) if tab.wants_mouse() => tab.on_mouse(mouse, ctx),
            _ => Handled::Pass,
        }
    }

    /// MOD-74 D3: a lost capture, to **every** sub-tab, as [`on_item_change`](Self::on_item_change)
    /// goes (B-4): a sub-tab switch is itself a loss, so the pane holding the gesture is no longer
    /// the active one. No `wants_mouse` gate: the pane told no longer wants it.
    ///
    /// The loss is the shell's on-to-off capture edge, which equals "the gesture holder lost the
    /// pointer" only because [`RunsTab`] is the sole view that wants the mouse. A second
    /// capturing sub-tab must also handle a switch between two capturing views (e.g. by tracking
    /// the capture holder), or the review-L3 stale anchor returns.
    pub fn on_mouse_lost(&mut self) {
        for tab in &mut self.tabs {
            tab.on_mouse_lost();
        }
    }

    /// Hands a list motion to the active sub-tab while the detail is focused (MOD-115 M2 §7);
    /// `Pass` while the registry is empty or the sub-tab takes no step.
    pub fn step(&mut self, step: Step, ctx: &Ctx<'_>) -> Handled {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.step(step, ctx),
            None => Handled::Pass,
        }
    }

    /// Offers a key to the active sub-tab.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.on_key(key, ctx),
            None => Handled::Pass,
        }
    }

    /// D7: hands an `$EDITOR` outcome to the active sub-tab, the only one that could have asked
    /// (a capturing sub-tab keeps every key, so the strip cannot move while its area is open).
    pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.on_external_edit(outcome, ctx),
            None => tracing::debug!("no detail sub-tab to take the $EDITOR outcome"),
        }
    }
}

/// The detail pane's title (MOD-115 M4 E4, MOD-82): `KEY · title` of the selected item, or
/// `Detail` while a project header is selected.
#[must_use]
pub fn title(item: Option<&ItemSummary>) -> String {
    item.map_or_else(
        || "Detail".to_owned(),
        |item| format!("{} \u{b7} {}", item.key, item.title),
    )
}

/// Draws the detail pane as a kit pane (MOD-115 M4 E4): titled [`title`], kept first (C5, Q1),
/// the sub-tabs as chips on its top border windowed around the active one with `‹`/`›`, and the
/// active sub-tab in the whole inner rect. The content is drawn before the frame, which touches
/// border cells only (M3 C3).
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    registry: &DetailRegistry,
    item: Option<&ItemSummary>,
    chrome: Chrome<'_>,
    ctx: &Ctx<'_>,
) {
    let inner = Pane::inner(area);
    match registry.active() {
        Some(tab) => tab.render(frame, inner, ctx),
        None => message(frame, inner, "No detail sub-tab is registered.", ctx.theme),
    }
    let title = title(item);
    let titles: Vec<&str> = registry
        .titles()
        .into_iter()
        .map(|(_, title)| title)
        .collect();
    chrome
        .frame(
            Pane::new(&title)
                .title_first()
                .sub_tabs(&titles, registry.active_index()),
        )
        .render(frame, area, ctx.theme);
}

/// The one line a sub-tab renders instead of a blank pane (plan D11).
pub fn message(frame: &mut Frame<'_>, area: Rect, text: &str, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(Line::styled(text.to_owned(), theme.dim)).wrap(Wrap { trim: true }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEvent;
    use htui_core::model::ItemId;
    use ratatui::Frame;
    use ratatui::layout::Rect;

    use super::{DetailId, DetailRegistry, DetailTab};
    use crate::app::{Ctx, Handled};
    use crate::keys::{Stack, views};
    use crate::store_worker::StoreReply;
    use crate::ui::kit::scroll::Step;

    /// A sub-tab that keeps every default, `key_stack` included.
    struct Plain;

    /// A sub-tab in a mode of its own: it reports the Runs browse stack.
    struct Converted;

    impl DetailTab for Plain {
        fn id(&self) -> DetailId {
            DetailId("plain")
        }
        fn title(&self) -> &str {
            "Plain"
        }
        fn on_item_change(&mut self, _item: Option<ItemId>) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    impl DetailTab for Converted {
        fn id(&self) -> DetailId {
            DetailId("converted")
        }
        fn title(&self) -> &str {
            "Converted"
        }
        fn on_item_change(&mut self, _item: Option<ItemId>) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn key_stack(&self) -> Stack<'static> {
            views::RUNS_BROWSE
        }
    }

    /// MOD-67 M5 D4 (blueprint T-D1): the registry reports the active sub-tab's stack, and Body's
    /// browse stack while nothing is registered or the sub-tab keeps the default.
    #[test]
    fn the_registry_reports_the_active_panes_stack() {
        let mut registry = DetailRegistry::new();
        assert_eq!(
            registry.key_stack(),
            views::BACKLOG_BODY,
            "an empty registry"
        );

        registry.register(Box::new(Converted));
        registry.register(Box::new(Plain));
        assert_eq!(registry.active_id(), Some(DetailId("converted")));
        assert_eq!(registry.key_stack(), views::RUNS_BROWSE, "the override");

        registry.cycle_next();
        assert_eq!(registry.active_id(), Some(DetailId("plain")));
        assert_eq!(registry.key_stack(), views::BACKLOG_BODY, "the default");

        registry.cycle_next();
        assert_eq!(
            registry.key_stack(),
            views::RUNS_BROWSE,
            "back on the override"
        );
    }

    /// A sub-tab that records the steps it is handed.
    struct Stepper(std::rc::Rc<std::cell::RefCell<Vec<Step>>>);

    impl DetailTab for Stepper {
        fn id(&self) -> DetailId {
            DetailId("stepper")
        }
        fn title(&self) -> &str {
            "Stepper"
        }
        fn on_item_change(&mut self, _item: Option<ItemId>) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn step(&mut self, step: Step, _ctx: &Ctx<'_>) -> Handled {
            self.0.borrow_mut().push(step);
            Handled::Consumed
        }
    }

    /// MOD-115 M2 §7: the registry hands a step to the active sub-tab only; a sub-tab that keeps
    /// the default declines it, and an empty registry has nobody to hand it to.
    #[test]
    fn the_registry_hands_a_step_to_the_active_pane() {
        let shell = super::compose::bench::Shell::new();
        let ctx = shell.ctx();
        let mut registry = DetailRegistry::new();
        assert_eq!(registry.step(Step::Down, &ctx), Handled::Pass, "empty");

        let seen = std::rc::Rc::default();
        registry.register(Box::new(Plain));
        registry.register(Box::new(Stepper(std::rc::Rc::clone(&seen))));
        assert_eq!(
            registry.step(Step::Down, &ctx),
            Handled::Pass,
            "the default"
        );
        assert!(seen.borrow().is_empty(), "a hidden pane is not stepped");

        registry.cycle_next();
        assert_eq!(registry.step(Step::Bottom, &ctx), Handled::Consumed);
        assert_eq!(*seen.borrow(), [Step::Bottom]);
    }
}
