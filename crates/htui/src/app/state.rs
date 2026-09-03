//! The shell's state: [`App`], the [`Ctx`] handed to views, and the top bar's projection.

use std::collections::HashMap;
use std::mem::Discriminant;

use htui_core::model::{ProjectRef, Scope, WorkspaceId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::cell::RefCell;
use tokio::sync::mpsc;

use crate::app::action::{Action, Handled};
use crate::keymap::{KeyChord, KeyScope, Keymap};
use crate::store_worker::{Origin, RequestEnvelope, Seq, StoreRequest};
use crate::ui::overlay::{Overlay, OverlayRegistry, OverlayStack};
use crate::ui::tabs::{Tab, TabRegistry};
use crate::ui::{Theme, layout, top_bar};
use crossterm::event::{Event, KeyEvent, KeyEventKind};

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
    /// Active runs in scope (`RunStatus::is_active`).
    pub active_runs: usize,
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
    /// The key table, for help lines.
    pub keymap: &'a Keymap,
    /// The palette.
    pub theme: &'a Theme,
    /// Who the shell is talking to; replies to this view's requests come back addressed to it.
    origin: Origin,
    /// The action sink.
    emit: &'a Emit,
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
            origin,
            emit,
        }
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
    /// The key table.
    pub keymap: Keymap,
    /// The palette.
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
        }
    }

    /// Issues the reads the shell itself needs: the workspace list and this box's row.
    ///
    /// The first `Workspaces` reply also picks the startup scope (blueprint D, "Startup").
    pub fn start(&mut self) {
        self.dispatch(Origin::App, StoreRequest::Workspaces);
        self.dispatch(Origin::App, StoreRequest::BoxInfo);
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
                    other => self.update(other),
                }
            }
        }
        if !self.emit.is_empty() {
            tracing::warn!("emit queue did not settle; dropping the rest");
            let _ = self.emit.take();
        }
    }

    /// A terminal event. Only key presses reach views; a resize just asks for a redraw.
    pub fn on_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.on_key(key);
            }
            Event::Resize(_, _) => self.dirty = true,
            _ => {}
        }
    }

    /// The propagation chain of blueprint C.4, stopping at the first `Handled::Consumed`.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        let chord = KeyChord::from_event(key);

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keymap,
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
                        );
                        top.on_key(key, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed {
                return;
            }
            if let Some(action) = self.keymap.resolve(&KeyScope::Overlay(id), chord).cloned() {
                self.update(action);
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
                        );
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

        if let Some(action) = self.keymap.resolve(&KeyScope::Global, chord).cloned() {
            self.update(action);
        }
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
    }

    /// Draws one frame: top bar, tab strip, the active tab, the status line, then the overlays
    /// bottom-up and the help box on top.
    ///
    /// Rendering is read-only by contract: a view that emits here is not applied until the next
    /// key or reply drains the queue.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        self.dirty = false;
        let area = frame.area();
        let chrome = layout::chrome(area);

        top_bar::render(frame, chrome.top_bar, &self.top_bar, &self.theme);
        crate::ui::tabs::registry::render_strip(frame, chrome.tab_strip, &self.tabs, &self.theme);

        match self.tabs.active() {
            Some(tab) => {
                let ctx = self.ctx(Origin::Tab(tab.id()));
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

        let (status, style) = match &self.status {
            Some(message) => (message.clone(), self.theme.error),
            None => (self.keymap.help_line(&KeyScope::Global), self.theme.dim),
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

    /// The `?` box: the bindings of the global scope and of the active tab.
    fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let mut lines = vec![Line::styled(
            self.keymap.help_line(&KeyScope::Global),
            self.theme.base,
        )];
        if let Some(id) = self.tabs.active_id() {
            let tab_help = self.keymap.help_line(&KeyScope::Tab(id));
            if !tab_help.is_empty() {
                lines.push(Line::styled(tab_help, self.theme.base));
            }
        }
        lines.push(Line::styled("? closes this box", self.theme.dim));
        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let box_area = layout::centered(area, area.width.saturating_sub(10).max(20), height);
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .wrap(Wrap { trim: true })
                .block(Block::new().borders(Borders::ALL).title(" Keys ")),
            box_area,
        );
    }
}
