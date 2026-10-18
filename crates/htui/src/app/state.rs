//! The shell's state: [`App`], the [`Ctx`] handed to views, and the top bar's projection.

use std::collections::HashMap;
use std::mem::Discriminant;

use htui_core::model::{ProjectRef, Scope, WorkspaceId};
use htui_worker::WaitingView;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::cell::{Cell, RefCell};
use tokio::sync::mpsc;

use crate::app::action::{Action, Handled, OverlayAction, RevealKind, TabAction};
use crate::app::pane::OpenEditor;
use crate::editor::{EDITOR_BUSY, ExternalEdit, ExternalEditOutcome};
use crate::keys::{Act, CTRL_C, Context, HelpLine, KeyChord, Keys, Stack};
use crate::store_worker::{Origin, RequestEnvelope, Seq, StoreRequest};
use crate::ui::cells::clip_line;
use crate::ui::header::{self, Flash};
use crate::ui::kit::pane::paint_bottom_hints;
use crate::ui::overlay::{Overlay, OverlayRegistry, OverlayStack};
use crate::ui::tabs::{Tab, TabId, TabRegistry};
use crate::ui::{Theme, about, layout};
use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use zeroize::Zeroizing;

/// What the status line says when anything but a tab asks for `$EDITOR` (MOD-9 D10): the outcome
/// is handed back through `Tab::on_external_edit`, so there would be no one to hand it to.
pub const EDITOR_NEEDS_A_TAB: &str = "only a tab can open the editor";

/// How many rounds of "apply an action, collect what it emitted" one drain may take before the
/// shell gives up. A view that emits on every drain round is a bug, not a reason to hang.
const DRAIN_ROUNDS: usize = 16;

/// What the top bar shows (`R-TUI-1`). Only `App::observe_reply` writes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopBarState {
    /// Name of the workspace in scope, empty before the first `SetScope`.
    pub workspace: String,
    /// Hostname of this box, empty until the `BoxInfo` reply lands.
    pub box_name: String,
    /// `Backend::label()`: `"memory"` in MOD-1, `"online"` / `"offline · 3m"` in MOD-6.
    pub store: String,
    /// MOD-69 plan D6: the last `Waiting` reply, which the top bar counts and the overlay lists;
    /// `None` until the first one (blueprint A-6).
    pub waiting: Option<WaitingView>,
}

/// The action sink handed to views.
///
/// A view holds no channel and no store handle (`R-NF-3`): it pushes actions here and the shell
/// applies them once the view has returned, which is also why a view can emit while the shell
/// still holds it mutably borrowed.
#[derive(Debug, Default)]
pub struct Emit(RefCell<Vec<Action>>);

impl Emit {
    /// Queues an action.
    pub fn push(&self, action: Action) {
        self.0.borrow_mut().push(action);
    }

    /// Takes everything queued so far, leaving the queue empty.
    #[must_use]
    pub fn take(&self) -> Vec<Action> {
        self.0.take()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

/// Everything a view is allowed to see, plus the way back out.
#[derive(Debug)]
pub struct Ctx<'a> {
    /// The workspace scope every read is issued against (plan D10).
    pub scope: &'a Scope,
    /// The scope's projects, ordered by `workspace_project.position`.
    pub projects: &'a [ProjectRef],
    /// What the top bar currently shows.
    pub top_bar: &'a TopBarState,
    /// The palette.
    pub theme: &'a Theme,
    /// The keys in force (MOD-67 D9): `App`'s own at the shell's sites, a bench's in tests.
    keys: &'a Keys,
    /// Who the shell is talking to; replies to this view's requests come back addressed to it.
    origin: Origin,
    /// The action sink.
    emit: &'a Emit,
    /// MOD-57 P2: where the active tab names its editing rect; `None` at every other site.
    editor_area: Option<&'a Cell<Option<Rect>>>,
}

impl<'a> Ctx<'a> {
    /// Builds the context of one view. The shell is the only caller.
    #[must_use]
    pub fn new(
        scope: &'a Scope,
        projects: &'a [ProjectRef],
        top_bar: &'a TopBarState,
        keys: &'a Keys,
        theme: &'a Theme,
        origin: Origin,
        emit: &'a Emit,
    ) -> Self {
        Self {
            scope,
            projects,
            top_bar,
            keys,
            theme,
            origin,
            emit,
            editor_area: None,
        }
    }

    /// Lends this context the cell [`claim_editor_area`](Self::claim_editor_area) writes. Only
    /// `App::render` calls it, for the active tab's render.
    #[must_use]
    pub fn with_editor_area(mut self, cell: &'a Cell<Option<Rect>>) -> Self {
        self.editor_area = Some(cell);
        self
    }

    /// MOD-57 P2: names `area` as the rect this view's text is edited in, so an in-pane editor the
    /// view asked for draws over it. A no-op outside the active tab's render; the last claim of a
    /// frame wins.
    pub fn claim_editor_area(&self, area: Rect) {
        if let Some(cell) = self.editor_area {
            cell.set(Some(area));
        }
    }

    /// The keys in force, for hint rows and labels (M3-M5). The borrow lives as long as the
    /// context's data (`'a`), not as long as this `Ctx`.
    #[must_use]
    pub fn keys(&self) -> &'a Keys {
        self.keys
    }

    /// Asks the store. The reply comes back to this view through `on_reply`, unless a newer
    /// request of the same kind overtook it (blueprint C.2).
    pub fn request(&self, request: StoreRequest) {
        self.emit.push(Action::Store(request));
    }

    /// Emits an action.
    pub fn emit(&self, action: Action) {
        self.emit.push(action);
    }

    /// Who this view is.
    #[must_use]
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
}

/// The whole shell.
#[derive(Debug)]
pub struct App {
    /// The workspace in scope.
    pub scope: Scope,
    /// The scope's projects, in position order.
    pub projects: Vec<ProjectRef>,
    /// The top bar's projection.
    pub top_bar: TopBarState,
    /// Registered tabs, in strip order.
    pub tabs: TabRegistry,
    /// Open overlays, bottom first.
    pub overlays: OverlayStack,
    /// How an [`OverlayId`](crate::ui::overlay::OverlayId) becomes an overlay.
    pub overlay_factories: OverlayRegistry,
    /// The named-action keys in force (MOD-67 D10): the compiled defaults; M2 builds them from
    /// `keys.toml`.
    pub keys: Keys,
    /// Global actions that open a named view, offered by `register_all` (D5). An action not offered
    /// is neither dispatched nor shown.
    pub(super) offered: Vec<(Act, Action)>,
    /// The palette: `Theme::default()` until `run` applies `NO_COLOR` (MOD-80 D4).
    pub theme: Theme,
    /// Whether the help box is up.
    pub help_visible: bool,
    /// Whether the about panel under the header is up (MOD-115). In memory only: it starts on
    /// and M2 persists it.
    pub about_visible: bool,
    /// Last error text, shown on the status line.
    pub status: Option<String>,
    /// Set by `Action::Quit`; the event loop stops after the current iteration.
    pub should_quit: bool,
    /// Set by every state change, cleared by the draw that follows it.
    pub dirty: bool,
    /// What views emitted while the shell held them borrowed.
    pub(super) emit: Emit,
    /// The worker's inbox.
    pub(super) requests: mpsc::UnboundedSender<RequestEnvelope>,
    /// Newest `seq` per `(origin, request kind)`: the staleness index of blueprint C.2.
    pub(super) latest: HashMap<(Origin, Discriminant<StoreRequest>), Seq>,
    /// Next `seq` to stamp.
    pub(super) next_seq: Seq,
    /// Ticks since startup; the top bar refreshes every fourth one.
    pub(super) ticks: u64,
    /// Overlay to open when the first `Workspaces` reply is empty (blueprint D, "Startup").
    ///
    /// Set by [`register_all`](crate::app::register_all); `None` leaves an empty store on the
    /// bare shell. Held as an id, not a view, so the shell never names a concrete overlay.
    pub startup_overlay: Option<crate::ui::overlay::OverlayId>,
    /// Overlay to open when a `StoreState` reply carries a pending-migration count (plan D11).
    ///
    /// Set by [`register_all`](crate::app::register_all), and an id for the same reason
    /// `startup_overlay` is one: the shell must not name a concrete view (MOD-1 blueprint D).
    pub migration_overlay: Option<crate::ui::overlay::OverlayId>,
    /// The quit question (MOD-57 M2 E4): opened by `global.quit` while an in-pane editor is
    /// alive. Set by [`register_all`](crate::app::register_all), an id for `migration_overlay`'s
    /// reason; `None` (a bare shell) quits at once, as M1 did.
    pub quit_overlay: Option<crate::ui::overlay::OverlayId>,
    /// The abort question (MOD-57 M2 R1 M-2): opened by `editor.abort` on the asking tab. Set by
    /// [`register_all`](crate::app::register_all), an id for `migration_overlay`'s reason; `None`
    /// (a bare shell) aborts at once, as M1 did.
    pub abort_overlay: Option<crate::ui::overlay::OverlayId>,
    /// Tab that shows a replayed step (MOD-2 D39, `R-HIS-2`).
    ///
    /// Set by [`register_all`](crate::app::register_all), and an id for the same reason
    /// `migration_overlay` is one: `Action::Replay` has to focus a view and address a reply to
    /// it, and the shell must not name a concrete one to do so. `None` means this build can
    /// replay nothing, which the status line says rather than the shell swallowing the key.
    pub replay_tab: Option<TabId>,
    /// Which tab reveals which kind of entity (MOD-64 D235). Set by
    /// [`register_all`](crate::app::register_all), ids for the reason `replay_tab` is one: the
    /// shell names no concrete view. A kind with no entry reveals nothing.
    pub reveal_tabs: Vec<(RevealKind, TabId)>,
    /// Whether the migration prompt has already been offered this session.
    ///
    /// `StoreState` is re-read every fourth tick, so without this an answered `n` would re-open
    /// the prompt a second later, forever (blueprint D.5).
    pub(super) migration_prompt_shown: bool,
    /// Whether the below-the-target notice has been shown this session (MOD-40 plan D9).
    ///
    /// `StoreState` is re-read every fourth tick; without this a notice the next key cleared
    /// would come back a second later, `migration_prompt_shown`'s reason.
    pub(super) below_target_shown: bool,
    /// Whether the no-DSN redirect has already fired this session (MOD-15 M6 D6).
    ///
    /// `ConnectionInfo` is re-issued whenever the shell asks again, so without this a box with an
    /// empty keyring would be dragged back to `Settings > Connection` from wherever the user had
    /// moved on to — the defect `migration_prompt_shown` exists to prevent, one reply across.
    pub(super) connection_redirect_done: bool,
    /// The `$EDITOR` handoff a tab asked for, and which tab (MOD-9 D10).
    ///
    /// Set by `drain` from an `Action::EditExternally` a tab emitted; taken by the event loop,
    /// which suspends the terminal, runs the editor and hands the outcome back through
    /// [`App::finish_external_edit`]. One slot: a second ask before the loop comes round replaces
    /// the first.
    pub(super) pending_edit: Option<(TabId, ExternalEdit)>,
    /// MOD-74 D1: mouse capture as the loop last applied it, the edge `mouse_capture` detects.
    /// `take_external_edit` clears it for the editor's `leave` (D2).
    pub(super) mouse: bool,
    /// MOD-57: the in-pane editor, while one is alive (one at a time, P6).
    pub(super) editor: Option<OpenEditor>,
    /// MOD-57 P2: the active tab's claim this frame. `None` at the top of every `render`.
    pub(super) editor_area: Cell<Option<Rect>>,
    /// MOD-57 P2: where the active tab's pane goes, as the last frame computed it; read by
    /// `open_editor` (the spawn size) and `resize_editor`.
    pub(super) editor_rect: Option<(TabId, Rect)>,
}

impl App {
    /// A shell with no tabs, no overlays and no workspace yet.
    ///
    /// The caller registers views ([`register_all`](crate::app::register_all)), sets
    /// `top_bar.store` from `Backend::label()` and calls [`App::start`].
    #[must_use]
    pub fn new(requests: mpsc::UnboundedSender<RequestEnvelope>) -> Self {
        Self {
            scope: Scope {
                workspace_id: WorkspaceId::default(),
                project_ids: Vec::new(),
            },
            projects: Vec::new(),
            top_bar: TopBarState::default(),
            tabs: TabRegistry::new(),
            overlays: OverlayStack::new(),
            overlay_factories: OverlayRegistry::new(),
            keys: Keys::compiled().clone(),
            offered: Vec::new(),
            theme: Theme::default(),
            help_visible: false,
            about_visible: true,
            status: None,
            should_quit: false,
            dirty: true,
            emit: Emit::default(),
            requests,
            latest: HashMap::new(),
            next_seq: 0,
            ticks: 0,
            startup_overlay: None,
            migration_overlay: None,
            quit_overlay: None,
            abort_overlay: None,
            replay_tab: None,
            reveal_tabs: Vec::new(),
            migration_prompt_shown: false,
            below_target_shown: false,
            connection_redirect_done: false,
            pending_edit: None,
            mouse: false,
            editor: None,
            editor_area: Cell::new(None),
            editor_rect: None,
        }
    }

    /// Issues the reads the shell itself needs: the workspace list, this box's row, the store
    /// state and the connection.
    ///
    /// The first `Workspaces` reply also picks the startup scope (blueprint D, "Startup"); the
    /// `StoreState` one is what makes the top bar right on the first frame rather than a second
    /// later, and what carries a pending-migration count into the shell (plan D11).
    ///
    /// `ConnectionInfo` is the shell's own read rather than the section's (MOD-15 M6 D6): a box
    /// whose keyring is empty has to be steered to the field *before* the user finds the Settings
    /// tab, and no section that has never been activated can ask for anything. It is addressed
    /// under `Origin::App`, so the section's own read — which is keyed by origin — is not stale to
    /// it and it is not stale to the section's.
    pub fn start(&mut self) {
        self.dispatch(Origin::App, StoreRequest::Workspaces);
        self.dispatch(Origin::App, StoreRequest::BoxInfo);
        self.dispatch(Origin::App, StoreRequest::StoreState);
        self.dispatch(Origin::App, StoreRequest::ConnectionInfo);
    }

    /// Registers a tab and, when it becomes the active one, issues its requests.
    pub fn register_tab(&mut self, tab: Box<dyn Tab>) {
        let first = self.tabs.is_empty();
        self.tabs.register(tab);
        if first {
            self.activate_tab();
        }
        self.dirty = true;
    }

    /// Pushes an overlay and issues the requests it asks for.
    pub fn push_overlay(&mut self, overlay: Box<dyn Overlay>) {
        let id = overlay.id();
        let requests = overlay.wants_requests(&self.scope);
        self.overlays.push(overlay);
        for request in requests {
            self.dispatch(Origin::Overlay(id), request);
        }
        self.dirty = true;
    }

    /// Offers a global action that names a view (MOD-67 D5), replacing an earlier offer of `act`.
    /// Only `register_all` calls it: the shell itself never names a concrete overlay.
    pub fn offer(&mut self, act: Act, action: Action) {
        debug_assert!(Self::is_offerable(act), "{act:?} is not offerable");
        self.offered.retain(|(known, _)| *known != act);
        self.offered.push((act, action));
    }

    /// The global actions [`offer`](Self::offer) takes: those that open a named view. Every other
    /// global action is [`action_for`](Self::action_for)'s fixed mapping.
    const fn is_offerable(act: Act) -> bool {
        matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)
    }

    /// What the shell does for `act`: the fixed mapping, then the offered table. `None` for
    /// everything a view must handle (every shared context) and for an offerable act nobody
    /// offered.
    pub(super) fn action_for(&self, act: Act) -> Option<Action> {
        match act {
            // MOD-57 M2 E4: asks first while an in-pane editor is alive.
            Act::Quit => Some(self.request_quit()),
            Act::NextTab => Some(Action::Tab(TabAction::Next)),
            Act::PrevTab => Some(Action::Tab(TabAction::Prev)),
            Act::Help => Some(Action::ToggleHelp),
            Act::About => Some(Action::ToggleAbout),
            Act::OverlayClose => Some(Action::Overlay(OverlayAction::Close)),
            other => other
                .tab_index()
                .map(|index| Action::Tab(TabAction::Select(index)))
                .or_else(|| {
                    self.offered
                        .iter()
                        .find(|(known, _)| *known == other)
                        .map(|(_, action)| action.clone())
                }),
        }
    }

    /// The stack of the view that holds the keyboard (MOD-67 D4, PA-8): the top overlay's when
    /// an overlay is up, else the active tab's. `None` while that view is unconverted: the
    /// status line and the `?` box are then today's.
    #[must_use]
    pub fn active_stack(&self) -> Option<Stack<'static>> {
        match self.overlays.top() {
            Some(top) => top.key_stack(),
            None => self.tabs.active().and_then(Tab::key_stack),
        }
    }

    /// D6 steps 2 and 6: the first candidate the shell maps to an action wins. `true` if one did.
    fn apply_keys(&mut self, stack: Stack<'_>, chord: KeyChord) -> bool {
        let action = self
            .keys
            .actions(stack, chord)
            .into_iter()
            .find_map(|act| self.action_for(act));
        match action {
            Some(action) => {
                self.update(action);
                true
            }
            None => false,
        }
    }

    /// Issues the active tab's `wants_requests`: called on activation and after a scope change.
    pub(super) fn activate_tab(&mut self) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let id = tab.id();
        let requests = tab.wants_requests(&self.scope);
        for request in requests {
            self.dispatch(Origin::Tab(id), request);
        }
    }

    /// Stamps a request with a fresh `seq`, records it as the newest of its kind for this origin
    /// and hands it to the worker.
    pub(super) fn dispatch(&mut self, origin: Origin, request: StoreRequest) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.latest
            .insert((origin.clone(), std::mem::discriminant(&request)), seq);
        let envelope = RequestEnvelope {
            seq,
            origin,
            request,
        };
        if self.requests.send(envelope).is_err() {
            tracing::warn!("store worker is gone; request dropped");
        }
    }

    /// Whether a reply is still the newest of its kind for its origin (blueprint C.2).
    ///
    /// `seq` values are globally unique and each one is recorded under exactly one
    /// `(origin, request kind)` key, so "is this `seq` still in the index for this origin?" is
    /// the same question as "is `latest[(origin, kind)] == seq`?" without having to recover the
    /// request kind from the reply.
    pub(super) fn is_fresh(&self, origin: &Origin, seq: Seq) -> bool {
        self.latest
            .iter()
            .any(|((known, _), newest)| known == origin && *newest == seq)
    }

    /// Forgets every staleness entry of `origin` (MOD-64 D240), so no reply to a request made
    /// before this can pass [`App::is_fresh`] again.
    pub(super) fn forget(&mut self, origin: &Origin) {
        self.latest.retain(|(known, _), _| known != origin);
    }

    /// Applies every action views emitted while the shell held them borrowed.
    ///
    /// `Action::Store` is stamped with `origin` here: that is why the action itself carries none.
    pub(super) fn drain(&mut self, origin: &Origin) {
        for _ in 0..DRAIN_ROUNDS {
            let actions = self.emit.take();
            if actions.is_empty() {
                return;
            }
            for action in actions {
                match action {
                    Action::Store(request) => self.dispatch(origin.clone(), request),
                    // Stamped like `Store` (MOD-9 D10): the outcome goes back to the tab that
                    // asked, and nothing but a tab can be handed one.
                    Action::EditExternally(edit) => match origin {
                        Origin::Tab(id) => {
                            self.pending_edit = Some((*id, edit));
                            self.dirty = true;
                        }
                        _ => self.update(Action::Error(EDITOR_NEEDS_A_TAB.to_owned())),
                    },
                    // MOD-57 M2 R2 LOW-2: only the abort question confirms an abort. The bare
                    // shell's direct abort (no question registered) goes through `update`, not
                    // here.
                    Action::AbortEdit => match origin {
                        Origin::Overlay(id) if self.abort_overlay == Some(*id) => {
                            self.update(Action::AbortEdit);
                        }
                        _ => tracing::warn!(?origin, "AbortEdit from outside the abort question"),
                    },
                    other => self.update(other),
                }
            }
        }
        if !self.emit.is_empty() {
            tracing::warn!("emit queue did not settle; dropping the rest");
            let _ = self.emit.take();
        }
    }

    /// The edit a tab asked for, taken by the event loop (MOD-9 D10). `Some` exactly when the loop
    /// is about to suspend, and `Suspend::leave` turns capture off without the app knowing: so a
    /// capture that was on is lost here (MOD-74 D2), and the loop's next `mouse_capture` re-enables
    /// it with no stale gesture.
    ///
    /// MOD-57 PD-8: while an in-pane editor is alive a second edit is not handed to the loop; the
    /// asking tab is answered `Failed(EDITOR_BUSY)` here, so a view waiting for one outcome gets
    /// exactly one.
    pub fn take_external_edit(&mut self) -> Option<(TabId, ExternalEdit)> {
        if self.editor.is_some()
            && let Some((tab, _)) = self.pending_edit.take()
        {
            self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_BUSY.to_owned()));
            return None;
        }
        let edit = self.pending_edit.take();
        if edit.is_some() && self.mouse {
            self.lose_mouse();
            self.mouse = false;
        }
        edit
    }

    /// Routes the editor's outcome to the tab that asked, with a [`Ctx`], then drains what it
    /// emitted, as `on_reply` does. A tab that is no longer registered drops it. Sets `dirty`.
    pub fn finish_external_edit(&mut self, tab: TabId, outcome: ExternalEditOutcome) {
        self.dirty = true;
        let origin = Origin::Tab(tab);
        {
            let Self {
                scope,
                projects,
                top_bar,
                keys,
                theme,
                emit,
                tabs,
                ..
            } = self;
            let Some(view) = tabs.by_id_mut(tab) else {
                tracing::debug!(%tab, "the tab that asked for the editor is gone");
                return;
            };
            let mut ctx = Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
            view.on_external_edit(outcome, &mut ctx);
        }
        self.drain(&origin);
    }

    /// A terminal event. Key presses, bracketed pastes and (while a view wants them, MOD-71 D4)
    /// mouse events reach views; a resize just asks for a redraw.
    pub fn on_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.on_key(key);
            }
            // Wrapped before anything reads it: a paste may be a credential (MOD-22's redirect),
            // and this buffer is wiped however the paste ends.
            Event::Paste(text) => self.on_paste(&Zeroizing::new(text)),
            Event::Mouse(mouse) => self.on_mouse(mouse),
            Event::Resize(_, _) => self.dirty = true,
            _ => {}
        }
    }

    /// A bracketed paste (MOD-22 review M-1): the propagation chain of [`on_key`](Self::on_key)
    /// with **no key table in it**. The top overlay, then — unless that overlay is modal — the active
    /// tab, each taking it only into a field it is capturing input with; whatever none of them
    /// takes is dropped and does nothing else. So a paste made before a field is open is never a
    /// key: its digits switch no tab, its `/` opens no filter, its `q` quits nothing.
    ///
    /// Unlike a key it does not clear the status line: a paste nothing took did nothing, and one a
    /// field refused says why through the same line.
    ///
    /// MOD-57: while an in-pane editor is alive a paste follows a key's rule (R0 F2): to the
    /// editor while it has the keys, else swallowed (the first that finds it without them) or,
    /// on the asking tab, dropped; under a modal overlay the overlay's path below has it, and on
    /// another tab that tab's (M2 E3). It never reaches the asking view.
    pub fn on_paste(&mut self, text: &Zeroizing<String>) {
        self.dirty = true;
        if self.editor.is_some() && self.editor_paste(text) {
            return;
        }

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        top.on_paste(text, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed || self.overlays.top().is_some_and(Overlay::is_modal) {
                return;
            }
        }

        if let Some(id) = self.tabs.active_id() {
            let origin = Origin::Tab(id);
            {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    tabs,
                    ..
                } = self;
                if let Some(tab) = tabs.active_mut() {
                    let mut ctx =
                        Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                    // `Pass` is the drop: there is no key table for a paste to fall through to.
                    let _ = tab.on_paste(text, &mut ctx);
                }
            }
            self.drain(&origin);
        }
    }

    /// MOD-71 D1: whether the view on screen wants the mouse, which
    /// [`mouse_capture`](Self::mouse_capture) records and the event loop applies after every
    /// step. No overlay may be open and the `?` box may not be up — both draw over the tab, and a
    /// click through them would act on what they hide (blueprint E7) — and the active tab must
    /// want it.
    ///
    /// MOD-57 M2 E8: off while the in-pane editor is on screen (the pane draws over the view, and
    /// the terminal keeps its own selection over it); on another tab the view decides.
    #[must_use]
    pub fn wants_mouse(&self) -> bool {
        !self.editor_visible()
            && self.overlays.is_empty()
            && !self.help_visible
            && self.tabs.active().is_some_and(Tab::wants_mouse)
    }

    /// MOD-74 D1: what the event loop hands `TerminalGuard::set_mouse_capture` after every step:
    /// [`wants_mouse`](Self::wants_mouse), recorded. On an on-to-off edge every registered tab is
    /// told ([`Tab::on_mouse_lost`]): after a tab switch the tab holding the gesture is no longer
    /// the active one.
    ///
    /// A "loss" is the on-to-off capture edge. That equals "the gesture holder lost the pointer"
    /// only because `RunsTab` is the sole view that wants the mouse: a switch between two
    /// capturing views keeps capture on and fires nothing. A second `wants_mouse` implementor
    /// must also handle that switch (e.g. by tracking the capture holder), or the review-L3 stale
    /// anchor returns.
    #[must_use = "the loop must hand this to TerminalGuard::set_mouse_capture"]
    pub fn mouse_capture(&mut self) -> bool {
        let wants = self.wants_mouse();
        if self.mouse && !wants {
            self.lose_mouse();
        }
        self.mouse = wants;
        wants
    }

    /// MOD-74 D1, D2: tells every tab capture went off.
    fn lose_mouse(&mut self) {
        for tab in self.tabs.iter_mut() {
            tab.on_mouse_lost();
        }
    }

    /// A mouse event (MOD-71 D4): to the active tab only, with no key table and no overlay in the
    /// chain (an open overlay turns capture off, D1).
    ///
    /// Gated first, so an event queued before capture went off does nothing. `Moved` (Windows still
    /// reports motion, and a terminal may ignore button-only mode, MOD-74 D5) and the horizontal
    /// wheel are dropped before dispatch. Only a `Consumed` event sets `dirty` and clears the
    /// status line (blueprint E8): a pointer crossing the canvas must neither redraw once per cell
    /// nor wipe an error nobody acted on. The status is taken before dispatch, as
    /// [`on_key`](Self::on_key) clears it, so a failure the event causes still lands.
    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        if !self.wants_mouse() {
            return;
        }
        if matches!(
            mouse.kind,
            MouseEventKind::Moved | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
        ) {
            return;
        }
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let origin = Origin::Tab(id);
        let status = self.status.take();
        let handled = {
            let Self {
                scope,
                projects,
                top_bar,
                keys,
                theme,
                emit,
                tabs,
                ..
            } = self;
            match tabs.active_mut() {
                Some(tab) => {
                    let mut ctx =
                        Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                    tab.on_mouse(mouse, &mut ctx)
                }
                None => Handled::Pass,
            }
        };
        self.drain(&origin);
        if handled == Handled::Consumed {
            self.dirty = true;
        } else if self.status.is_none() {
            self.status = status;
        }
    }

    /// The propagation chain (blueprint C.4, MOD-67 D6), stopping at the first consumer: the
    /// in-pane editor (MOD-57), `ctrl-c`, the top overlay, the overlay's stack (`Stack::OVERLAY`
    /// while unconverted), the modal swallow, the active tab, the tab's stack
    /// (`Stack::BASE` while unconverted).
    ///
    /// `ctrl-c` is checked here only: a bracketed paste has no key table
    /// ([`on_paste`](Self::on_paste)), so a pasted `U+0003` never quits.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        // An error survives until the user does something, then the help line comes back. Cleared
        // before the key is dispatched, so a failure this key causes still lands.
        self.status = None;
        let chord = KeyChord::from_event(key);

        // MOD-57 P5, P6, M2 E1-E3: a live in-pane editor answers first. Focused, it takes every
        // key, `ctrl-c` included (ANA-26 §6.4's one exception); on the asking tab, the editor's and
        // the global keys only; on another tab, only `editor.focus`. `ctrl-c` and a modal
        // overlay's keys take the path below.
        if self.editor.is_some() && self.editor_key(key, chord) {
            return;
        }

        // `ctrl-c` quits before any overlay or view sees it (ANA-26 §6.4, MOD-67 D6). MOD-57's
        // exception is the step above.
        if chord == CTRL_C {
            self.update(Action::Quit);
            return;
        }

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        top.on_key(key, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed {
                return;
            }
            // `Esc` closes; `?`/`F1` toggle help over any overlay, for a key the overlay passed.
            // Before the modal swallow: under a future non-modal overlay, help pre-empts the active
            // tab for a key the overlay passed (D6). The first non-modal overlay revisits this.
            // A converted overlay resolves through its own stack (MOD-67 M3 D6), which carries
            // the layer filter.
            let stack = self
                .overlays
                .top()
                .and_then(Overlay::key_stack)
                .unwrap_or(Stack::OVERLAY);
            if self.apply_keys(stack, chord) {
                return;
            }
            // A modal overlay swallows what it did not handle: the tab below never sees it.
            if self.overlays.top().is_some_and(Overlay::is_modal) {
                return;
            }
        }

        if let Some(id) = self.tabs.active_id() {
            let origin = Origin::Tab(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    tabs,
                    ..
                } = self;
                match tabs.active_mut() {
                    Some(tab) => {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        tab.on_key(key, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed {
                return;
            }
        }

        // The active tab's stack (MOD-67 M3 D6), else the base stack. The tab's, not
        // `active_stack()`: a key reaches this step only when no modal overlay swallowed it.
        let stack = self
            .tabs
            .active()
            .and_then(Tab::key_stack)
            .unwrap_or(Stack::BASE);
        self.apply_keys(stack, chord);
    }

    /// The context of one view.
    #[must_use]
    pub(super) fn ctx(&self, origin: Origin) -> Ctx<'_> {
        Ctx::new(
            &self.scope,
            &self.projects,
            &self.top_bar,
            &self.keys,
            &self.theme,
            origin,
            &self.emit,
        )
    }

    /// The global keys the status line listed before MOD-115, as text: the global actions the
    /// active view's stack admits and the shell offers, chords from [`Keys`]. Nothing draws it
    /// any more (the header's chips and the `?` box carry the same keys); tests read it to pin how
    /// a view's stack and a rebind shape what a user can press.
    #[must_use]
    pub fn keys_line(&self) -> String {
        self.keys
            .status_line(self.active_stack().unwrap_or(Stack::BASE), |act| {
                self.action_for(act).is_some()
            })
    }

    /// The header's flash (MOD-115): what used to be the status line's text, now on the right of
    /// the header. The first of the last error ([`App::status`], error style), the in-pane
    /// editor's line ([`App::editor_status`], dim) and another tab's `editing in <Tab> · ...`
    /// notice ([`App::editor_notice`], dim, its trailing separator stripped); `None` when none of
    /// them is up. The global help line is gone: `?` opens the box and the header carries the
    /// `F9 about ? keys` chips.
    #[must_use]
    pub(super) fn flash(&self) -> Option<Flash> {
        if let Some(message) = &self.status {
            return Some(Flash {
                text: message.clone(),
                style: self.theme.error,
            });
        }
        let text = self.editor_status().or_else(|| {
            self.editor_notice().map(|notice| {
                notice
                    .strip_suffix(" \u{b7} ")
                    .map_or(notice.clone(), str::to_owned)
            })
        })?;
        Some(Flash {
            text,
            style: self.theme.dim,
        })
    }

    /// Draws one frame: the one-row header, the about panel when it is up, the active tab, the
    /// in-pane editor over it (MOD-57 P1), the active tab's key hints on the body's bottom border
    /// (MOD-115 M1 adapter), then the overlays bottom-up and the help box on top.
    ///
    /// The hints are the active tab's [`Tab::key_hint`]; they are left off under any overlay (the
    /// overlay's box has its own hint, plan Q1 (a)) and while the in-pane editor's tab is on
    /// screen (focused, the editor has the keys; unfocused, the tab refuses its own,
    /// [`Stack::EDITOR_OPEN`]). The `?` box leaves them alone: the box is centred over the body.
    ///
    /// Rendering is read-only by contract: a view that emits here is not applied until the next
    /// key or reply drains the queue.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        self.dirty = false;
        self.editor_area.set(None);
        let area = frame.area();
        // A tab with no about text (a test double, a reserved tab) leaves no empty hole under the
        // header: the panel's rows go to the body.
        let about = self
            .tabs
            .active()
            .filter(|_| self.about_visible)
            .and_then(|tab| about::about_for(tab.id()).map(|text| (tab, text)));
        let chrome = layout::chrome_with(area, about.is_some());

        let flash = self.flash();
        // M1 wires no mouse handler to the header's hit rects.
        let _hits = header::render(
            frame,
            chrome.header,
            &self.top_bar,
            &self.tabs,
            flash.as_ref(),
            &self.keys,
            // The chips are labelled through the stack that holds the keyboard, so `F9 about`
            // is not offered under an overlay that does not take it.
            self.active_stack().unwrap_or(Stack::BASE),
            &self.theme,
        );
        if let (Some(panel), Some((tab, text))) = (chrome.about, about) {
            let stack = tab.key_stack().unwrap_or(Stack::BASE);
            about::render(frame, panel, text, &self.keys, stack, &self.theme);
        }

        match self.tabs.active() {
            Some(tab) => {
                let ctx = self
                    .ctx(Origin::Tab(tab.id()))
                    .with_editor_area(&self.editor_area);
                tab.render(frame, chrome.body, &ctx);
            }
            None => {
                frame.render_widget(
                    Paragraph::new(Line::styled("No tab is registered.", self.theme.dim))
                        .block(Block::new().borders(Borders::ALL)),
                    chrome.body,
                );
            }
        }

        self.draw_editor(frame, chrome.body);

        // The M1 hint adapter: tabs still frame their own bodies, so the hints are painted onto
        // the last row's border glyphs once the tab is drawn. Deleted as tabs move to `Pane::hints`.
        if let Some(tab) = self.tabs.active()
            && self.overlays.is_empty()
            && !self.editor_visible()
            && chrome.body.height > 0
        {
            let width = chrome.body.width.saturating_sub(4);
            let hint = tab.key_hint(width, &self.ctx(Origin::Tab(tab.id())));
            if let Some(line) = hint
                && let Some(row) = hint_row(frame.buffer_mut(), chrome.body)
            {
                let line = clip_line(line, width);
                join_panes_under(frame.buffer_mut(), row, line.width());
                paint_bottom_hints(frame.buffer_mut(), row, line, self.theme.base);
            }
        }

        for overlay in self.overlays.iter() {
            let ctx = self.ctx(Origin::Overlay(overlay.id()));
            overlay.render(frame, area, &ctx);
        }

        if self.help_visible {
            self.render_help(frame, area);
        }
    }

    /// The `?` box (MOD-67 D8), rebuilt from the live state every frame. With a converted view
    /// holding the keyboard ([`App::active_stack`]): one line per layer of its stack, then the
    /// closing line its global layer admits. Otherwise: the overlay context (if one is
    /// up), then the global context, then the closing line.
    ///
    /// MOD-57 M2 E7 (PD-M2-2): with an in-pane editor alive the box lists the editor keys that
    /// act where the user is. On the asking tab, [`Stack::EDITOR_OPEN`]'s box (the editor's keys,
    /// then every global; none of the asking tab's own, which it refuses); on another tab,
    /// `editor.focus` first, then that view's box; under a modal overlay, the overlay's box with
    /// no editor key.
    ///
    /// Each logical line is packed into rows that fit the box ([`HelpLine::rows`]), so the box is
    /// exactly as tall as what it shows and no row is clipped.
    fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let box_width = area.width.saturating_sub(10).max(20);
        let inner = usize::from(box_width.saturating_sub(2));
        let offered = |act| self.action_for(act).is_some();
        // MOD-57 M2 E7 (PD-M2-2): the editor keys that act where the user is.
        let uncovered = self.editor.is_some() && !self.editor_covered();
        let locked = uncovered && self.editor_visible();
        let elsewhere = uncovered && !self.editor_visible();
        let mut lines: Vec<HelpLine> = Vec::new();
        if elsewhere {
            lines.extend(
                self.keys
                    .help_line(Context::Editor, |act| act == Act::EditorFocus),
            );
        }
        let stack = if locked {
            Some(Stack::EDITOR_OPEN)
        } else {
            self.active_stack()
        };
        let closer = match stack {
            Some(stack) => {
                lines.extend(self.keys.help_lines(stack, offered));
                self.keys.help_closer(stack)
            }
            None => {
                if !self.overlays.is_empty() {
                    lines.extend(self.keys.help_line(Context::Overlay, offered));
                }
                lines.extend(self.keys.help_line(Context::Global, offered));
                self.keys.help_closer(Stack::BASE)
            }
        };
        let mut rows: Vec<Line<'_>> = lines
            .iter()
            .flat_map(|line| line.rows(inner))
            .map(|row| Line::styled(row, self.theme.base))
            .collect();
        if let Some(closer) = closer {
            rows.push(Line::styled(closer, self.theme.dim));
        }
        let height = u16::try_from(rows.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(area.height);
        let box_area = layout::centered(area, box_width, height);
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(rows))
                .block(Block::new().borders(Borders::ALL).title(" Keys ")),
            box_area,
        );
    }
}

/// How many of the body's last rows [`hint_row`] looks through for the tab's bottom border.
const HINT_SEARCH_ROWS: u16 = 4;

/// The row of `body` the M1 hint adapter paints on: the lowest of its last [`HINT_SEARCH_ROWS`]
/// rows that starts with a bottom-left corner, i.e. the bottom border of the tab's content. A tab
/// that puts a notice or a filter row under its frame still gets the hints on the frame; one with
/// no such border in its last rows loses them until it moves to `Pane::hints` (M3, M4).
fn hint_row(buf: &ratatui::buffer::Buffer, body: Rect) -> Option<Rect> {
    let body = body.intersection(buf.area);
    (0..HINT_SEARCH_ROWS.min(body.height))
        .map(|up| body.bottom() - 1 - up)
        .find(|y| matches!(buf[(body.x, *y)].symbol(), "\u{2514}" | "\u{2570}"))
        .map(|y| Rect::new(body.x, y, body.width, 1))
}

/// Turns the corners where two side-by-side panes meet on `row` into plain rule, across the
/// `width` cells the hints will cover from two cells in, so a hint longer than the first pane
/// reads on across the seam instead of being cut by `┘└`.
fn join_panes_under(buf: &mut ratatui::buffer::Buffer, row: Rect, width: usize) {
    let end = row
        .x
        .saturating_add(2)
        .saturating_add(u16::try_from(width).unwrap_or(u16::MAX))
        .min(row.right().saturating_sub(1));
    for x in row.x.saturating_add(2)..end {
        if matches!(
            buf[(x, row.y)].symbol(),
            "\u{2518}" | "\u{2514}" | "\u{256f}" | "\u{2570}"
        ) {
            buf[(x, row.y)].set_symbol("\u{2500}");
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::{App, hint_row, join_panes_under};
    use crate::keys::{CATALOGUE, Context};

    /// MOD-115 M1 adapter: the hints go on the lowest bottom border among the body's last four
    /// rows, so a tab with a notice under its frame still shows them; none there, none shown.
    #[test]
    fn the_hint_row_is_the_lowest_bottom_border_in_the_last_rows() {
        let body = Rect::new(0, 1, 20, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 11));
        assert_eq!(hint_row(&buf, body), None);
        buf[(0, 5)].set_symbol("\u{2514}");
        assert_eq!(hint_row(&buf, body), None, "five rows up is too far");
        buf[(0, 8)].set_symbol("\u{2514}");
        assert_eq!(hint_row(&buf, body), Some(Rect::new(0, 8, 20, 1)));
        buf[(0, 10)].set_symbol("\u{2570}");
        assert_eq!(hint_row(&buf, body), Some(Rect::new(0, 10, 20, 1)));
    }

    /// A hint longer than the first pane reads on across the seam where two panes meet.
    #[test]
    fn a_long_hint_joins_the_panes_it_runs_across() {
        let row = Rect::new(0, 0, 14, 1);
        let mut buf = Buffer::empty(row);
        buf.set_string(0, 0, "\u{2514}\u{2500}\u{2500}\u{2500}\u{2518}\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}", ratatui::style::Style::default());
        join_panes_under(&mut buf, row, 2);
        assert_eq!(
            buf[(4, 0)].symbol(),
            "\u{2518}",
            "the hint stops before the seam"
        );
        join_panes_under(&mut buf, row, 8);
        assert_eq!(buf[(4, 0)].symbol(), "\u{2500}");
        assert_eq!(buf[(5, 0)].symbol(), "\u{2500}");
        assert_eq!(buf[(0, 0)].symbol(), "\u{2514}", "the outer corners stay");
        assert_eq!(buf[(13, 0)].symbol(), "\u{2518}");
    }

    /// MOD-67 review L1: `action_for`'s wildcard arm is only safe while every global act is
    /// either fixed-mapped by the shell or offerable (and so dispatched once `register_all`
    /// offers it). A new global act that is neither would be silently inert.
    #[test]
    fn every_global_act_is_fixed_mapped_or_offerable() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let app = App::new(tx);
        for row in CATALOGUE
            .iter()
            .filter(|row| row.context == Context::Global)
        {
            let fixed = app.action_for(row.act).is_some();
            assert!(
                fixed != App::is_offerable(row.act),
                "{:?}: fixed-mapped {fixed}, offerable {}",
                row.act,
                App::is_offerable(row.act)
            );
        }
    }
}
