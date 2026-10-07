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
use crate::keymap::{KeyScope, Keymap};
use crate::keys::{Act, CTRL_C, Context, HelpLine, KeyChord, Keys, Stack};
use crate::store_worker::{Origin, RequestEnvelope, Seq, StoreRequest};
use crate::ui::overlay::{Overlay, OverlayRegistry, OverlayStack};
use crate::ui::tabs::{Tab, TabId, TabRegistry};
use crate::ui::{Theme, layout, top_bar};
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
    /// The legacy Backlog tab rows (MOD-67 D2). No view reads it; the named-action keys are
    /// [`keys()`](Self::keys).
    pub keymap: &'a Keymap,
    /// The palette.
    pub theme: &'a Theme,
    /// The keys in force (MOD-67 D9): `App`'s own at the shell's sites, else the compiled
    /// defaults. No view reads it in M1.
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
        keymap: &'a Keymap,
        theme: &'a Theme,
        origin: Origin,
        emit: &'a Emit,
    ) -> Self {
        Self {
            scope,
            projects,
            top_bar,
            keymap,
            theme,
            keys: Keys::compiled(),
            origin,
            emit,
            editor_area: None,
        }
    }

    /// Hands the view these keys instead of the compiled defaults: how `App` passes its own.
    #[must_use]
    pub fn with_keys(mut self, keys: &'a Keys) -> Self {
        self.keys = keys;
        self
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
    /// The legacy tab rows: the six Backlog rows `register_all` binds (MOD-67 D2). Global and
    /// overlay keys are `keys`' since MOD-67.
    pub keymap: Keymap,
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
    pub fn new(requests: mpsc::UnboundedSender<RequestEnvelope>, keymap: Keymap) -> Self {
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
            keymap,
            keys: Keys::compiled().clone(),
            offered: Vec::new(),
            theme: Theme::default(),
            help_visible: false,
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
        matches!(act, Act::Workspaces | Act::Find | Act::Waiting)
    }

    /// What the shell does for `act`: the fixed mapping, then the offered table. `None` for
    /// everything a view must handle (every shared context) and for an offerable act nobody
    /// offered.
    fn action_for(&self, act: Act) -> Option<Action> {
        match act {
            Act::Quit => Some(Action::Quit),
            Act::NextTab => Some(Action::Tab(TabAction::Next)),
            Act::PrevTab => Some(Action::Tab(TabAction::Prev)),
            Act::Help => Some(Action::ToggleHelp),
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
                keymap,
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
            let mut ctx = Ctx::new(
                scope,
                projects,
                top_bar,
                keymap,
                theme,
                origin.clone(),
                emit,
            )
            .with_keys(keys);
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
    /// with **no keymap in it**. The top overlay, then — unless that overlay is modal — the active
    /// tab, each taking it only into a field it is capturing input with; whatever none of them
    /// takes is dropped and does nothing else. So a paste made before a field is open is never a
    /// key: its digits switch no tab, its `/` opens no filter, its `q` quits nothing.
    ///
    /// Unlike a key it does not clear the status line: a paste nothing took did nothing, and one a
    /// field refused says why through the same line.
    ///
    /// MOD-57: while an in-pane editor is alive a paste goes to it (focused and on screen) or
    /// nowhere; it never reaches a view or an overlay (the M1 lock).
    pub fn on_paste(&mut self, text: &Zeroizing<String>) {
        self.dirty = true;
        if self.editor.is_some() {
            self.editor_paste(text);
            return;
        }

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keymap,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx = Ctx::new(
                            scope,
                            projects,
                            top_bar,
                            keymap,
                            theme,
                            origin.clone(),
                            emit,
                        )
                        .with_keys(keys);
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
                    keymap,
                    keys,
                    theme,
                    emit,
                    tabs,
                    ..
                } = self;
                if let Some(tab) = tabs.active_mut() {
                    let mut ctx = Ctx::new(
                        scope,
                        projects,
                        top_bar,
                        keymap,
                        theme,
                        origin.clone(),
                        emit,
                    )
                    .with_keys(keys);
                    // `Pass` is the drop: there is no keymap for a paste to fall through to.
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
    /// MOD-57: off while an in-pane editor is alive, so the terminal keeps its own selection over
    /// the pane; M2 narrows this to a focused pane on screen.
    #[must_use]
    pub fn wants_mouse(&self) -> bool {
        self.editor.is_none()
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

    /// A mouse event (MOD-71 D4): to the active tab only, with no keymap and no overlay in the
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
                keymap,
                keys,
                theme,
                emit,
                tabs,
                ..
            } = self;
            match tabs.active_mut() {
                Some(tab) => {
                    let mut ctx = Ctx::new(
                        scope,
                        projects,
                        top_bar,
                        keymap,
                        theme,
                        origin.clone(),
                        emit,
                    )
                    .with_keys(keys);
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
    /// in-pane editor (MOD-57), `ctrl-c`, the top overlay, the overlay stack, the modal swallow,
    /// the active tab, its legacy rows, the base stack.
    ///
    /// `ctrl-c` is checked here only: a bracketed paste has no key table
    /// ([`on_paste`](Self::on_paste)), so a pasted `U+0003` never quits.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        // An error survives until the user does something, then the help line comes back. Cleared
        // before the key is dispatched, so a failure this key causes still lands.
        self.status = None;
        let chord = KeyChord::from_event(key);

        // MOD-57 P5, P6: a live in-pane editor answers first. Focused, it takes every key,
        // `ctrl-c` included (ANA-26 §6.4's one exception); unfocused, the M1 lock, except under a
        // modal overlay, whose keys take the path below.
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
                    keymap,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx = Ctx::new(
                            scope,
                            projects,
                            top_bar,
                            keymap,
                            theme,
                            origin.clone(),
                            emit,
                        )
                        .with_keys(keys);
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
            if self.apply_keys(Stack::OVERLAY, chord) {
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
                    keymap,
                    keys,
                    theme,
                    emit,
                    tabs,
                    ..
                } = self;
                match tabs.active_mut() {
                    Some(tab) => {
                        let mut ctx = Ctx::new(
                            scope,
                            projects,
                            top_bar,
                            keymap,
                            theme,
                            origin.clone(),
                            emit,
                        )
                        .with_keys(keys);
                        tab.on_key(key, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed {
                return;
            }
            if let Some(action) = self.keymap.resolve(&KeyScope::Tab(id), chord).cloned() {
                self.update(action);
                return;
            }
        }

        // The base stack.
        self.apply_keys(Stack::BASE, chord);
    }

    /// The context of one view.
    #[must_use]
    pub(super) fn ctx(&self, origin: Origin) -> Ctx<'_> {
        Ctx::new(
            &self.scope,
            &self.projects,
            &self.top_bar,
            &self.keymap,
            &self.theme,
            origin,
            &self.emit,
        )
        .with_keys(&self.keys)
    }

    /// Draws one frame: top bar, tab strip, the active tab, the in-pane editor over it (MOD-57
    /// P1), the status line, then the overlays bottom-up and the help box on top.
    ///
    /// Rendering is read-only by contract: a view that emits here is not applied until the next
    /// key or reply drains the queue.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        self.dirty = false;
        self.editor_area.set(None);
        let area = frame.area();
        let chrome = layout::chrome(area);

        top_bar::render(frame, chrome.top_bar, &self.top_bar, &self.theme);
        crate::ui::tabs::registry::render_strip(frame, chrome.tab_strip, &self.tabs, &self.theme);

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

        let (status, style) = match &self.status {
            Some(message) => (message.clone(), self.theme.error),
            None => (
                self.editor_status()
                    .unwrap_or_else(|| self.keys.status_line(|act| self.action_for(act).is_some())),
                self.theme.dim,
            ),
        };
        frame.render_widget(Paragraph::new(Line::styled(status, style)), chrome.status);

        for overlay in self.overlays.iter() {
            let ctx = self.ctx(Origin::Overlay(overlay.id()));
            overlay.render(frame, area, &ctx);
        }

        if self.help_visible {
            self.render_help(frame, area);
        }
    }

    /// The `?` box (MOD-67 D8), rebuilt from the live state every frame: the overlay context (if
    /// one is up), the editor context (while an in-pane editor is alive, MOD-57), the active tab's
    /// legacy rows, the global context, then the closing line.
    ///
    /// Each logical line is packed into rows that fit the box ([`HelpLine::rows`]), so the box is
    /// exactly as tall as what it shows and no row is clipped.
    fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let box_width = area.width.saturating_sub(10).max(20);
        let inner = usize::from(box_width.saturating_sub(2));
        let offered = |act| self.action_for(act).is_some();
        let mut lines: Vec<HelpLine> = Vec::new();
        if !self.overlays.is_empty() {
            lines.extend(self.keys.help_line(Context::Overlay, offered));
        }
        if self.editor.is_some() {
            lines.extend(self.keys.help_line(Context::Editor, |_| true));
        }
        if let Some(tab) = self.tabs.active() {
            let legacy = self.keymap.help_line(&KeyScope::Tab(tab.id()));
            if !legacy.is_empty() {
                lines.push(HelpLine::new(
                    tab.title(),
                    legacy.split(" · ").map(str::to_owned).collect(),
                ));
            }
        }
        lines.extend(self.keys.help_line(Context::Global, offered));
        let mut rows: Vec<Line<'_>> = lines
            .iter()
            .flat_map(|line| line.rows(inner))
            .map(|row| Line::styled(row, self.theme.base))
            .collect();
        if let Some(closer) = self.keys.help_closer() {
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::App;
    use crate::keymap::Keymap;
    use crate::keys::{CATALOGUE, Context};

    /// MOD-67 review L1: `action_for`'s wildcard arm is only safe while every global act is
    /// either fixed-mapped by the shell or offerable (and so dispatched once `register_all`
    /// offers it). A new global act that is neither would be silently inert.
    #[test]
    fn every_global_act_is_fixed_mapped_or_offerable() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let app = App::new(tx, Keymap::new());
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
