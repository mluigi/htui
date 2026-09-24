//! `App::update`: the single place a state change happens (plan D5).
//!
//! Everything else in the crate produces [`Action`]s; this file is the only consumer. Adding a
//! feature adds an arm here and a file under `ui/`, never an arm in the event loop.

use htui_core::model::{RunId, Scope, StepId, WorkspaceId, WorkspaceSummary};

use crate::app::action::{Action, OverlayAction, TabAction};
use crate::app::state::{App, Ctx};
use crate::connection::DsnState;
use crate::store_worker::{Origin, ReplyEnvelope, StoreReply, StoreRequest};
use crate::ui::tabs::SettingsTab;
use crate::ui::tabs::settings::ConnectionSection;

/// The top bar's run count is re-read once a second, i.e. every fourth 250 ms tick.
const TICKS_PER_REFRESH: u64 = 4;

impl App {
    /// Applies one action.
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::Tab(tab) => self.update_tab(tab),
            Action::Overlay(overlay) => self.update_overlay(overlay),
            Action::Store(request) => self.dispatch(Origin::App, request),
            Action::Reply(envelope) => self.on_reply(envelope),
            Action::SetScope { workspace } => self.set_scope(workspace),
            Action::Replay { step_id } => self.replay(step_id),
            Action::Promote { run, step } => self.promote(run, step),
            Action::ToggleHelp => self.help_visible = !self.help_visible,
            Action::Error(message) => self.status = Some(message),
            Action::Tick => {
                self.on_tick();
                return;
            }
        }
        self.dirty = true;
    }

    /// Tab movement. A move that lands on another tab issues that tab's `wants_requests`.
    fn update_tab(&mut self, action: TabAction) {
        let before = self.tabs.active_id();
        match action {
            TabAction::Next => self.tabs.next(),
            TabAction::Prev => self.tabs.prev(),
            TabAction::Select(idx) => {
                self.tabs.select(idx);
            }
            TabAction::Focus(id) => {
                self.tabs.focus(id);
            }
            // The tab first, then the section inside it (D7). A section id nothing registers
            // leaves the tab focused and the section where it was, which is the honest outcome:
            // the addressed tab *is* on screen, and moving its cursor somewhere unasked-for
            // would be worse than not moving it.
            TabAction::FocusSection(id, section) => {
                if self.tabs.focus(id)
                    && let Some(tab) = self.tabs.by_id_mut(id)
                    && !tab.focus_section(section)
                {
                    tracing::debug!(tab = %id, %section, "no such section to focus");
                }
            }
        }
        if self.tabs.active_id() != before {
            self.activate_tab();
        }
    }

    /// Opening and closing overlays. An unregistered id is ignored: the shell must not die
    /// because a binding names an overlay a build did not register.
    fn update_overlay(&mut self, action: OverlayAction) {
        match action {
            OverlayAction::Open(id) => {
                if let Some(overlay) = self.overlay_factories.create(id) {
                    self.push_overlay(overlay);
                } else {
                    tracing::warn!(overlay = %id, "no factory registered");
                }
            }
            OverlayAction::Close => self.overlays.pop(),
            OverlayAction::CloseAll => self.overlays.clear(),
        }
    }

    /// The 250 ms timer. Only every fourth tick costs a redraw, and it is the one that re-reads
    /// the store state and the active-run count so the top bar stays live without a request per
    /// tick.
    ///
    /// `StoreState` is **not** gated on a non-empty scope: `connecting`, `offline · 3m` and a
    /// pending-migration count all have to keep arriving on a shell that never entered a
    /// workspace, which is exactly the shell a first launch shows (blueprint D.5).
    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks.is_multiple_of(TICKS_PER_REFRESH) {
            self.dispatch(Origin::App, StoreRequest::StoreState);
            if !self.scope.is_empty() {
                let scope = self.scope.clone();
                self.dispatch(Origin::App, StoreRequest::ActiveRuns { scope });
            }
            self.dirty = true;
        }
    }

    /// Enters a workspace: the only path that changes the scope (plan D10).
    fn set_scope(&mut self, workspace: WorkspaceSummary) {
        self.scope = Scope::from_workspace(&workspace);
        let mut projects = workspace.projects;
        projects.sort_by_key(|p| p.position);
        self.projects = projects;
        self.top_bar.workspace = workspace.name;

        let scope = self.scope.clone();
        for tab in self.tabs.iter_mut() {
            tab.on_scope_change(&scope);
        }
        self.overlays.clear();
        self.activate_tab();
        self.dispatch(Origin::App, StoreRequest::ActiveRuns { scope });
    }

    /// Reopens a past step: focus the replay tab, then ask for its rows on that tab's behalf
    /// (MOD-2 D39).
    ///
    /// The request is stamped with the *replay* tab's origin, not the origin of whoever pressed
    /// the key, which is the whole reason [`App::dispatch`] takes one: the reply lands in that
    /// tab's `on_reply` like every other read, so no `Tab` method and no downcast is needed to
    /// hand a step to a view that did not ask for it. The staleness index does its ordinary work
    /// too — a second replay supersedes the first under `(Tab(replay), StepEvents)`.
    fn replay(&mut self, step_id: StepId) {
        let Some(tab) = self.replay_tab else {
            self.status = Some("no tab can replay a step".to_owned());
            return;
        };
        self.update_tab(TabAction::Focus(tab));
        self.dispatch(Origin::Tab(tab), StoreRequest::StepEvents(step_id));
    }

    /// Promotes a step to a chat: focus the tab that drives chats, then ask for the promotion on
    /// its behalf (MOD-4 plan D165), the shape of [`Self::replay`].
    ///
    /// The request carries `chat_open: false`; the run runtime overwrites it with what this
    /// process knows (blueprint D185).
    fn promote(&mut self, run: RunId, step: StepId) {
        let _ = (run, step);
        todo!()
    }

    /// A reply came back: top bar first, then the staleness gate, then the addressee.
    fn on_reply(&mut self, envelope: ReplyEnvelope) {
        self.observe_reply(&envelope.reply);
        if !self.is_fresh(&envelope.origin, envelope.seq) {
            // A newer request of the same kind was issued from the same origin (blueprint C.2).
            return;
        }
        // Below the gate: a superseded failure is as stale as any other reply, and reporting it
        // would blame the user's current selection for a request they have already moved past.
        if let StoreReply::Failed { request, message } = &envelope.reply {
            self.update(Action::Error(format!("{request}: {message}")));
        }
        let ReplyEnvelope { origin, reply, .. } = envelope;
        match origin {
            Origin::App => self.on_app_reply(&reply),
            Origin::Tab(id) => {
                let origin = Origin::Tab(id);
                {
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
                    if let Some(tab) = tabs.by_id_mut(id) {
                        let mut ctx = Ctx::new(
                            scope,
                            projects,
                            top_bar,
                            keymap,
                            theme,
                            origin.clone(),
                            emit,
                        );
                        tab.on_reply(&reply, &mut ctx);
                    }
                }
                self.drain(&origin);
            }
            Origin::Overlay(id) => {
                let origin = Origin::Overlay(id);
                {
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
                    // A reply to an overlay that has been popped is dropped silently.
                    if let Some(overlay) = overlays.by_id_mut(id) {
                        let mut ctx = Ctx::new(
                            scope,
                            projects,
                            top_bar,
                            keymap,
                            theme,
                            origin.clone(),
                            emit,
                        );
                        overlay.on_reply(&reply, &mut ctx);
                    }
                }
                self.drain(&origin);
            }
        }
    }

    /// The read-only look every reply gets, top bar only (blueprint C.2).
    pub fn observe_reply(&mut self, reply: &StoreReply) {
        match reply {
            StoreReply::Workspaces(workspaces) => {
                if let Some(current) = workspaces
                    .iter()
                    .find(|w| w.workspace_id == self.scope.workspace_id)
                {
                    self.top_bar.workspace = current.name.clone();
                }
            }
            StoreReply::BoxInfo(Some(info)) => self.top_bar.box_name = info.hostname.clone(),
            StoreReply::ActiveRuns(count) => self.top_bar.active_runs = *count,
            StoreReply::StoreState {
                label,
                migrations_pending,
            } => {
                self.top_bar.store = label.clone();
                if migrations_pending.is_some_and(|n| n > 0) {
                    self.offer_migration_prompt();
                }
            }
            StoreReply::MigrationsApplied { applied } => {
                self.status = Some(format!("applied {applied} migration(s)"));
            }
            _ => {}
        }
    }

    /// Opens the migration prompt, once per session (`R-STO-5`, plan D11).
    ///
    /// Opening an overlay is the one thing `observe_reply` does beyond the top bar, and
    /// `migration_prompt_shown` is why it can: the count is re-reported every fourth tick, so
    /// without it an answered `n` would be asked again a second later. The overlay is named by id
    /// and set by `register_all`, so the shell still never names a concrete view.
    fn offer_migration_prompt(&mut self) {
        if self.migration_prompt_shown {
            return;
        }
        let Some(id) = self.migration_overlay else {
            return;
        };
        if self.overlays.iter().any(|overlay| overlay.id() == id) {
            return;
        }
        self.migration_prompt_shown = true;
        self.update(Action::Overlay(OverlayAction::Open(id)));
    }

    /// Whether a workspace has been entered yet.
    ///
    /// `App::new` starts on the nil workspace id, which no row can carry (`uuid v7`, ANA-9 §3),
    /// so "is the scope still the startup placeholder?" is a comparison rather than a flag.
    fn has_scope(&self) -> bool {
        self.scope.workspace_id != WorkspaceId::default()
    }

    /// Replies the shell asked for itself.
    ///
    /// The first workspace list picks the startup scope, which is what makes `--demo` open
    /// inside a workspace instead of an empty shell. An empty list opens `startup_overlay`
    /// (the switcher, once registered) over the bare shell instead of leaving it blank
    /// (blueprint D, "Startup"). Nothing is opened before that reply, so the first frame is
    /// always the shell itself.
    fn on_app_reply(&mut self, reply: &StoreReply) {
        if let StoreReply::Workspaces(workspaces) = reply
            && !self.has_scope()
        {
            match workspaces.first() {
                Some(first) => self.update(Action::SetScope {
                    workspace: first.clone(),
                }),
                None => {
                    if let Some(id) = self.startup_overlay
                        && !self.overlays.iter().any(|o| o.id() == id)
                    {
                        self.update(Action::Overlay(OverlayAction::Open(id)));
                    }
                }
            }
        }

        // MOD-15 M6 D6: a box whose keyring is empty is redirected rather than stranded.
        //
        // Without this it parks at `offline · 0s` forever — the reconnect ticker is guarded on a
        // `reconnect` that a box with no DSN never has — pointing at a CLI flag the user cannot
        // reach without quitting.
        //
        // `NotStored` and nothing else. `NotApplicable` is `--demo`, which has no keyring story at
        // all; `Unreadable` is a keyring nobody could open, and sending that user to a masked
        // credential field would ask them to retype a DSN into a store that cannot hold it, over a
        // stored one that is still there and will read again when the collection unlocks.
        //
        // Once per session, for the reason `migration_prompt_shown` exists: `ConnectionInfo` is
        // re-issued whenever the shell asks again, and a redirect that fired on every reply would
        // drag the user back to Settings from whatever they had moved on to.
        if let StoreReply::Connection(snapshot) = reply
            && snapshot.dsn_state == DsnState::NotStored
            && !self.connection_redirect_done
        {
            self.connection_redirect_done = true;
            self.update(Action::Tab(TabAction::FocusSection(
                SettingsTab::ID,
                ConnectionSection::ID,
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Handled;
    use crate::keymap::Keymap;
    use crate::run_worker::OrchRequest;
    use crate::store_worker::RequestEnvelope;
    use crate::ui::overlay::MigrationPrompt;
    use crate::ui::tabs::{Tab, TabId};
    use crossterm::event::{KeyCode, KeyEvent};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        ItemFilter, ItemId, ItemKindId, ItemSummary, ProjectId, ProjectRef, Status,
    };
    use htui_orch::Command;
    use ratatui::Frame;
    use ratatui::layout::Rect;
    use std::cell::RefCell;
    use std::rc::Rc;
    use tokio::sync::mpsc;
    use tokio::sync::mpsc::UnboundedReceiver;

    /// What a [`Recorder`] was handed: the length of every `Items` reply it received.
    type Seen = Rc<RefCell<Vec<usize>>>;

    /// A tab that asks for `Items` and remembers every reply it was handed.
    #[derive(Debug, Default)]
    struct Recorder {
        seen: Seen,
    }

    impl Recorder {
        const ID: TabId = TabId("recorder");
    }

    impl Tab for Recorder {
        fn id(&self) -> TabId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Recorder"
        }
        fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
            vec![StoreRequest::Items {
                scope: scope.clone(),
                filter: ItemFilter::default(),
            }]
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
            if let StoreReply::Items(items) = reply {
                self.seen.borrow_mut().push(items.len());
            }
        }
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    fn workspace(name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: ids::WORKSPACE_PLATFORM,
            slug: name.to_lowercase(),
            name: name.to_owned(),
            projects: vec![ProjectRef {
                project_id: ids::PROJECT_HTUI,
                slug: "p".to_owned(),
                name: "P".to_owned(),
                position: 0,
            }],
        }
    }

    /// A shell with one recorder tab, the channel the worker would read, and the recorder's log.
    fn shell() -> (App, UnboundedReceiver<RequestEnvelope>, Seen) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx, Keymap::default_global());
        let seen: Seen = Seen::default();
        app.register_tab(Box::new(Recorder {
            seen: Rc::clone(&seen),
        }));
        (app, rx, seen)
    }

    /// The `seq` of the next `Items` request on the wire.
    fn next_items_seq(rx: &mut UnboundedReceiver<RequestEnvelope>) -> u64 {
        while let Ok(envelope) = rx.try_recv() {
            if matches!(envelope.request, StoreRequest::Items { .. }) {
                return envelope.seq;
            }
        }
        panic!("no Items request was dispatched")
    }

    fn items_reply(seq: u64, count: usize) -> ReplyEnvelope {
        let row = ItemSummary {
            id: ItemId::default(),
            project_id: ProjectId::default(),
            kind_id: ItemKindId::default(),
            key: "FEAT-1".to_owned(),
            key_prefix: "FEAT".to_owned(),
            key_number: 1,
            title: "t".to_owned(),
            status: Status::Open,
            priority: 0,
            required_tags: Vec::new(),
            updated_at: htui_core::fixtures::demo_at(0, 0),
            touched_paths: Vec::new(),
        };
        ReplyEnvelope {
            seq,
            origin: Origin::Tab(Recorder::ID),
            reply: StoreReply::Items(vec![row; count]),
        }
    }

    #[test]
    fn a_reply_overtaken_by_a_newer_request_of_the_same_kind_is_dropped() {
        let (mut app, mut rx, seen) = shell();
        let first = next_items_seq(&mut rx);

        // The first reply is still the newest of its kind, so it reaches the tab.
        app.update(Action::Reply(items_reply(first, 1)));
        assert_eq!(*seen.borrow(), vec![1]);

        // A scope change re-issues `Items` from the same origin: `first` is now stale.
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        let second = next_items_seq(&mut rx);
        assert_ne!(first, second);

        app.update(Action::Reply(items_reply(first, 7)));
        assert_eq!(*seen.borrow(), vec![1], "the stale reply is dropped");

        app.update(Action::Reply(items_reply(second, 2)));
        assert_eq!(*seen.borrow(), vec![1, 2], "the newest reply is delivered");
    }

    #[test]
    fn a_reply_addressed_to_another_view_never_reaches_a_tab() {
        let (mut app, mut rx, seen) = shell();
        let seq = next_items_seq(&mut rx);
        let mut envelope = items_reply(seq, 3);
        envelope.origin = Origin::Tab(TabId("someone-else"));
        app.update(Action::Reply(envelope));
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn the_first_workspace_list_picks_the_startup_scope() {
        let (mut app, mut rx, _seen) = shell();
        app.start();
        let seq = loop {
            let envelope = rx.try_recv().expect("start dispatched its requests");
            if matches!(envelope.request, StoreRequest::Workspaces) {
                break envelope.seq;
            }
        };
        app.update(Action::Reply(ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply: StoreReply::Workspaces(vec![workspace("Platform")]),
        }));
        assert_eq!(app.top_bar.workspace, "Platform");
        assert_eq!(app.scope.project_ids.len(), 1);
        assert_eq!(app.projects.len(), 1);
    }

    #[test]
    fn a_scope_change_closes_every_overlay_and_re_reads_the_run_count() {
        let (mut app, mut rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        assert!(app.overlays.is_empty());
        let mut saw_active_runs = false;
        while let Ok(envelope) = rx.try_recv() {
            saw_active_runs |= matches!(envelope.request, StoreRequest::ActiveRuns { .. });
        }
        assert!(saw_active_runs);
    }

    /// A `Failed` reply at `seq`, addressed to the recorder tab.
    fn failed_reply(seq: u64) -> ReplyEnvelope {
        ReplyEnvelope {
            seq,
            origin: Origin::Tab(Recorder::ID),
            reply: StoreReply::Failed {
                request: "items",
                message: "boom".to_owned(),
            },
        }
    }

    #[test]
    fn the_status_line_shows_a_failed_reply() {
        let (mut app, mut rx, _seen) = shell();
        let seq = next_items_seq(&mut rx);
        app.update(Action::Reply(failed_reply(seq)));
        assert_eq!(app.status.as_deref(), Some("items: boom"));
    }

    #[test]
    fn a_failure_overtaken_by_a_newer_request_never_reaches_the_status_line() {
        let (mut app, mut rx, seen) = shell();
        let first = next_items_seq(&mut rx);

        // A scope change re-issues `Items` from the same origin: `first` is now stale.
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        let second = next_items_seq(&mut rx);
        assert_ne!(first, second);

        app.update(Action::Reply(failed_reply(first)));
        assert_eq!(
            app.status, None,
            "a stale failure is dropped like any other"
        );
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn the_next_key_clears_a_failure_from_the_status_line() {
        let (mut app, mut rx, _seen) = shell();
        let seq = next_items_seq(&mut rx);
        app.update(Action::Reply(failed_reply(seq)));
        assert_eq!(app.status.as_deref(), Some("items: boom"));

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(app.status, None, "the help line comes back on the next key");
    }

    #[test]
    fn a_tick_only_asks_for_the_top_bar_once_a_second() {
        let (mut app, mut rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(requests.len(), 2, "one refresh per second, not per tick");
        assert!(matches!(requests[0].request, StoreRequest::StoreState));
        assert!(matches!(
            requests[1].request,
            StoreRequest::ActiveRuns { .. }
        ));
    }

    #[test]
    fn the_store_state_refresh_is_not_gated_on_a_scope() {
        // A first launch never enters a workspace, and `offline · 3m` still has to keep ticking.
        let (mut app, mut rx, _seen) = shell();
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(requests.len(), 1);
        assert!(matches!(requests[0].request, StoreRequest::StoreState));
    }

    /// A `StoreState` reply addressed to the shell.
    fn store_state(label: &str, migrations_pending: Option<usize>) -> ReplyEnvelope {
        ReplyEnvelope {
            seq: 0,
            origin: Origin::App,
            reply: StoreReply::StoreState {
                label: label.to_owned(),
                migrations_pending,
            },
        }
    }

    #[test]
    fn a_store_state_reply_writes_the_top_bar_and_opens_the_prompt_once() {
        let (mut app, _rx, _seen) = shell();
        // The real prompt, registered as `register_all` registers it: the shell names it by id.
        app.overlay_factories
            .register(MigrationPrompt::ID, || Box::new(MigrationPrompt::new()));
        app.migration_overlay = Some(MigrationPrompt::ID);

        app.update(Action::Reply(store_state("offline · 3m", None)));
        assert_eq!(app.top_bar.store, "offline · 3m");
        assert!(
            app.overlays.is_empty(),
            "no pending count, no prompt (plan D11)"
        );

        app.update(Action::Reply(store_state("online", Some(3))));
        assert_eq!(app.top_bar.store, "online");
        assert_eq!(app.overlays.iter().count(), 1, "the prompt opened");

        app.update(Action::Overlay(OverlayAction::Close));
        app.update(Action::Reply(store_state("online", Some(3))));
        assert!(
            app.overlays.is_empty(),
            "an answered prompt is not re-asked every fourth tick"
        );
    }

    /// MOD-4 plan D165: the promotion is asked for on the chat-driving tab's behalf, so its replies
    /// land there.
    #[test]
    fn promote_focuses_the_chat_tab_and_addresses_it() {
        let (mut app, mut rx, _seen) = shell();
        app.replay_tab = Some(Recorder::ID);
        while rx.try_recv().is_ok() {}
        let (run, step) = (RunId::new(), StepId::new());

        app.update(Action::Promote { run, step });
        assert_eq!(app.tabs.active_id(), Some(Recorder::ID));
        let envelope = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|envelope| matches!(envelope.request, StoreRequest::Orch(_)))
            .expect("the promotion is dispatched");
        assert_eq!(envelope.origin, Origin::Tab(Recorder::ID));
        assert!(matches!(
            envelope.request,
            StoreRequest::Orch(OrchRequest::Command(Command::PromoteStep {
                run: asked,
                step: promoted,
                chat_open: false,
            })) if asked == run && promoted == step
        ));
    }

    #[test]
    fn promote_with_no_chat_tab_says_so() {
        let (mut app, mut rx, _seen) = shell();
        while rx.try_recv().is_ok() {}
        app.update(Action::Promote {
            run: RunId::new(),
            step: StepId::new(),
        });
        assert_eq!(
            app.status.as_deref(),
            Some("no tab can drive a promoted step")
        );
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok())
                .all(|envelope| !matches!(envelope.request, StoreRequest::Orch(_))),
            "nothing is asked for"
        );
    }

    #[test]
    fn an_applied_migration_lands_on_the_status_line() {
        let (mut app, _rx, _seen) = shell();
        app.update(Action::Reply(ReplyEnvelope {
            seq: 0,
            origin: Origin::App,
            reply: StoreReply::MigrationsApplied { applied: 2 },
        }));
        assert_eq!(app.status.as_deref(), Some("applied 2 migration(s)"));
    }
}
