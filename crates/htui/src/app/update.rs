//! `App::update`: the single place a state change happens (plan D5).
//!
//! Everything else in the crate produces [`Action`]s; this file is the only consumer. Adding a
//! feature adds an arm here and a file under `ui/`, never an arm in the event loop.

use htui_core::model::{RunId, Scope, StepId, WorkspaceId, WorkspaceSummary};
use htui_orch::Command;

use crate::app::action::{Action, OverlayAction, RevealTarget, TabAction};
use crate::app::state::{App, Ctx, EDITOR_NEEDS_A_TAB};
use crate::connection::DsnState;
use crate::run_worker::OrchRequest;
use crate::store_worker::{Origin, ReplyEnvelope, StoreReply, StoreRequest};
use crate::ui::overlay::{Overlay, OverlayId};
use crate::ui::tabs::SettingsTab;
use crate::ui::tabs::settings::ConnectionSection;

/// The top bar's counts are re-read once a second, i.e. every fourth 250 ms tick.
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
            Action::Reveal(target) => self.reveal(&target),
            // The unstamped path (`Origin::App`, or a keymap binding): only a tab can open the
            // editor, because only a tab can be handed the outcome (MOD-9 D10).
            Action::EditExternally(_) => self.status = Some(EDITOR_NEEDS_A_TAB.to_owned()),
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
            OverlayAction::Close => self.close_top_overlay(),
            OverlayAction::CloseAll => self.close_every_overlay(),
        }
    }

    /// Pops the top overlay and forgets its requests (MOD-64 D240): `latest` is keyed by origin,
    /// and a later overlay under the same id would otherwise take a reply to one of this one's.
    fn close_top_overlay(&mut self) {
        let Some(id) = self.overlays.top().map(Overlay::id) else {
            return;
        };
        self.overlays.pop();
        self.forget_overlay(id);
    }

    /// Closes every overlay and forgets their requests (D240): `CloseAll` and a scope change.
    fn close_every_overlay(&mut self) {
        let ids: Vec<OverlayId> = self.overlays.iter().map(Overlay::id).collect();
        self.overlays.clear();
        for id in ids {
            self.forget_overlay(id);
        }
    }

    /// [`App::forget`] for `Origin::Overlay(id)`, unless an overlay under that id is still open.
    fn forget_overlay(&mut self, id: OverlayId) {
        if !self.overlays.iter().any(|open| open.id() == id) {
            self.forget(&Origin::Overlay(id));
        }
    }

    /// The 250 ms timer. Only every fourth tick costs a redraw, and it is the one that re-reads
    /// the store state and the waiting list so the top bar stays live without a request per
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
                self.dispatch(Origin::App, StoreRequest::Waiting { scope });
            }
            self.refresh_active_tab();
            self.dirty = true;
        }
    }

    /// MOD-41 plan D16: the active tab's refresh hook, with a [`Ctx`] addressed as that tab, then
    /// what it emitted drained, as [`App::finish_external_edit`] does.
    fn refresh_active_tab(&mut self) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
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
            let Some(view) = tabs.active_mut() else {
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
            );
            view.on_refresh(&mut ctx);
        }
        self.drain(&origin);
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
        self.close_every_overlay();
        self.activate_tab();
        // MOD-69 blueprint E9, H-10: the old workspace's rows never show under the new name.
        self.top_bar.waiting = None;
        self.dispatch(Origin::App, StoreRequest::Waiting { scope });
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
        let Some(tab) = self.replay_tab else {
            self.status = Some("no tab can drive a promoted step".to_owned());
            return;
        };
        self.update_tab(TabAction::Focus(tab));
        self.dispatch(
            Origin::Tab(tab),
            StoreRequest::Orch(OrchRequest::Command(Command::PromoteStep {
                run,
                step,
                chat_open: false,
            })),
        );
    }

    /// Selects an entity in the tab registered for its kind (MOD-64 D235): focus that tab —
    /// which re-sends its `wants_requests` if it was not the active one — then hand it the target
    /// with a `Ctx` of its own, as `finish_external_edit` does, and drain what it emitted. A kind
    /// nothing is registered for is a no-op, logged.
    fn reveal(&mut self, target: &RevealTarget) {
        let kind = target.kind();
        let Some(tab) = self
            .reveal_tabs
            .iter()
            .find(|(registered, _)| *registered == kind)
            .map(|(_, tab)| *tab)
        else {
            tracing::debug!(?kind, "no tab reveals this kind");
            return;
        };
        self.update_tab(TabAction::Focus(tab));
        let origin = Origin::Tab(tab);
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
            let Some(view) = tabs.by_id_mut(tab) else {
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
            );
            if !view.reveal(target, &mut ctx) {
                tracing::debug!(%tab, "the tab does not reveal this target");
            }
        }
        self.drain(&origin);
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
            // Review M1: the old scope's queued reply lands after `set_scope` (blueprint H-10). The
            // whole scope is compared, not its workspace: Settings re-scopes the same workspace
            // when a project is created, deleted or reordered (R2).
            StoreReply::Waiting { scope, view } => {
                if *scope == self.scope {
                    self.top_bar.waiting = Some(view.clone());
                } else {
                    tracing::debug!(
                        workspace = %scope.workspace_id,
                        "a waiting reply for a scope since left"
                    );
                }
            }
            StoreReply::StoreState {
                label,
                migrations_pending,
                below_target,
            } => {
                self.top_bar.store = label.clone();
                if migrations_pending.is_some_and(|n| n > 0) {
                    self.offer_migration_prompt();
                }
                if let Some(target) = below_target {
                    self.note_below_target(target);
                }
            }
            StoreReply::MigrationsApplied { applied } => {
                self.status = Some(format!("applied {applied} migration(s)"));
            }
            // MOD-7 D13, blueprint D25: the registration probe answers at `UNSOLICITED`, which the
            // freshness gate drops, so its report is rendered here, above the gate.
            StoreReply::BoxProbed(report) => self.status = Some(report.status_line()),
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

    /// Says once per session that this build is below the database's target version (MOD-40
    /// plan D9, PRD D3): a TUI warns and runs, where a headless process refuses.
    fn note_below_target(&mut self, target: &str) {
        if self.below_target_shown {
            return;
        }
        self.below_target_shown = true;
        self.status = Some(below_target_notice(target));
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

/// The status line's below-the-target sentence (MOD-40 plan D9).
fn below_target_notice(target: &str) -> String {
    format!(
        "htui {} is older than {target}, which last migrated this database; upgrade this box",
        htui_store::HTUI_VERSION
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::EDITOR_NEEDS_A_TAB;
    use crate::app::{Handled, RevealKind};
    use crate::editor::{ExternalEdit, ExternalEditOutcome};
    use crate::keymap::Keymap;
    use crate::store_worker::{RequestEnvelope, UNSOLICITED};
    use crate::ui::overlay::{MigrationPrompt, Overlay, OverlayId, WaitingList};
    use crate::ui::tabs::{Tab, TabId};
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use htui_core::fixtures::ids;
    use htui_core::model::{
        ItemFilter, ItemId, ItemKindId, ItemSummary, ProjectId, ProjectRef, Status,
    };
    use htui_orch::Command;
    use htui_worker::WaitingView;
    use ratatui::Frame;
    use ratatui::layout::Rect;
    use std::cell::{Cell, RefCell};
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
                ready_here: false,
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
    fn a_scope_change_closes_every_overlay_and_re_reads_the_waiting_list() {
        let (mut app, mut rx, _seen) = shell();
        app.top_bar.waiting = Some(WaitingView::default());
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        assert!(app.overlays.is_empty());
        // MOD-69 blueprint E9: the old workspace's rows never show under the new name.
        assert!(app.top_bar.waiting.is_none());
        let mut saw_waiting = false;
        while let Ok(envelope) = rx.try_recv() {
            saw_waiting |= matches!(envelope.request, StoreRequest::Waiting { .. });
        }
        assert!(saw_waiting);
    }

    /// MOD-69 plan D6: `observe_reply` runs before the freshness gate, so a `Waiting` reply sets
    /// the top bar whoever it is addressed to, even an unsolicited one.
    #[test]
    fn a_waiting_reply_sets_the_top_bar_whatever_its_origin() {
        let (mut app, _rx, _seen) = shell();
        let view = WaitingView {
            working: 3,
            rows: Vec::new(),
            permissions_known: true,
            offline: false,
        };
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::Overlay(WaitingList::ID),
            reply: StoreReply::Waiting {
                scope: app.scope.clone(),
                view: view.clone(),
            },
        }));
        assert_eq!(app.top_bar.waiting, Some(view));
    }

    /// MOD-69 review M1: a reply for the workspace the shell just left lands after `set_scope`
    /// (blueprint H-10) and is dropped; the new workspace's reply is taken.
    #[test]
    fn a_waiting_reply_for_the_old_workspace_is_dropped_after_a_scope_change() {
        let (mut app, _rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        let view = |working| WaitingView {
            working,
            rows: Vec::new(),
            permissions_known: true,
            offline: false,
        };
        let scope_of = |workspace_id| Scope {
            workspace_id,
            project_ids: vec![ids::PROJECT_HTUI],
        };
        let reply = |scope, view| {
            Action::Reply(ReplyEnvelope {
                seq: UNSOLICITED,
                origin: Origin::App,
                reply: StoreReply::Waiting { scope, view },
            })
        };

        app.update(reply(scope_of(ids::WORKSPACE_GRAPHICS), view(7)));
        assert_eq!(
            app.top_bar.waiting, None,
            "the old workspace's counts never show"
        );

        app.update(reply(scope_of(ids::WORKSPACE_PLATFORM), view(2)));
        assert_eq!(app.top_bar.waiting, Some(view(2)));

        app.update(reply(scope_of(ids::WORKSPACE_GRAPHICS), view(7)));
        assert_eq!(
            app.top_bar.waiting,
            Some(view(2)),
            "nor overwrite the new one"
        );
    }

    /// MOD-69 review R2 (M1): Settings re-scopes the *same* workspace when its project set
    /// changes, and a reply read under the old project set is dropped all the same.
    #[test]
    fn a_waiting_reply_for_the_old_project_set_of_the_same_workspace_is_dropped() {
        let (mut app, _rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        let old = app.scope.clone();
        let mut grown = workspace("Platform");
        grown.projects.push(ProjectRef {
            project_id: ids::PROJECT_AGY,
            slug: "q".to_owned(),
            name: "Q".to_owned(),
            position: 1,
        });
        app.update(Action::SetScope { workspace: grown });
        assert_eq!(app.scope.workspace_id, old.workspace_id);
        assert_ne!(app.scope, old);

        let view = |working| WaitingView {
            working,
            rows: Vec::new(),
            permissions_known: true,
            offline: false,
        };
        let reply = |scope, view| {
            Action::Reply(ReplyEnvelope {
                seq: UNSOLICITED,
                origin: Origin::App,
                reply: StoreReply::Waiting { scope, view },
            })
        };

        app.update(reply(old, view(7)));
        assert_eq!(
            app.top_bar.waiting, None,
            "the old project set's rows never show"
        );

        app.update(reply(app.scope.clone(), view(2)));
        assert_eq!(app.top_bar.waiting, Some(view(2)));
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
        assert!(matches!(requests[1].request, StoreRequest::Waiting { .. }));
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
                below_target: None,
            },
        }
    }

    /// A `StoreState` reply from a store whose database was last migrated by `target`.
    fn store_state_below(label: &str, target: &str) -> ReplyEnvelope {
        ReplyEnvelope {
            seq: 0,
            origin: Origin::App,
            reply: StoreReply::StoreState {
                label: label.to_owned(),
                migrations_pending: None,
                below_target: Some(target.to_owned()),
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

    /// MOD-40 plan D9, PRD D3: a TUI below the database's target version says so on the status
    /// line, once per session: the next key clears it and the fourth-tick re-read does not bring
    /// it back.
    #[test]
    fn a_store_state_below_the_target_says_so_once() {
        let (mut app, _rx, _seen) = shell();

        app.update(Action::Reply(store_state("online", None)));
        assert_eq!(app.status, None, "no target above this build, no notice");

        app.update(Action::Reply(store_state_below("online", "99.0.0")));
        assert_eq!(app.top_bar.store, "online");
        assert_eq!(app.status, Some(below_target_notice("99.0.0")));
        assert!(
            app.status.as_deref().is_some_and(
                |line| line.contains("99.0.0") && line.contains(htui_store::HTUI_VERSION)
            ),
            "the notice names both versions: {:?}",
            app.status
        );

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(
            app.status, None,
            "the next key clears it, as every status line"
        );

        app.update(Action::Reply(store_state_below("online", "99.0.0")));
        assert_eq!(
            app.status, None,
            "said once per session, not every fourth tick"
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

    /// What an [`Asker`] was handed back by the editor.
    type Heard = Rc<RefCell<Vec<ExternalEditOutcome>>>;

    /// A tab that asks for `$EDITOR` on `e` and remembers every outcome it is handed back
    /// (MOD-9 D10, blueprint D26).
    struct Asker {
        id: TabId,
        heard: Heard,
    }

    impl Asker {
        fn new(id: &'static str) -> (Self, Heard) {
            let heard = Heard::default();
            let asker = Self {
                id: TabId(id),
                heard: Rc::clone(&heard),
            };
            (asker, heard)
        }
    }

    /// The edit every [`Asker`] and [`Popup`] asks for.
    fn asked() -> ExternalEdit {
        ExternalEdit {
            text: "body\n".to_owned(),
            stem: "implement".to_owned(),
        }
    }

    impl Tab for Asker {
        fn id(&self) -> TabId {
            self.id
        }
        fn title(&self) -> &str {
            self.id.0
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            if key.code == KeyCode::Char('e') {
                ctx.emit(Action::EditExternally(asked()));
                return Handled::Consumed;
            }
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
            self.heard.borrow_mut().push(outcome);
            // Emitted from the callback, so the test sees that `finish_external_edit` drains.
            ctx.emit(Action::Error(format!("{} heard back", self.id)));
        }
    }

    /// An overlay that asks for `$EDITOR` on `e`: only a tab may (D10).
    struct Popup;

    impl Popup {
        const ID: OverlayId = OverlayId("popup");
    }

    impl Overlay for Popup {
        fn id(&self) -> OverlayId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Popup"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, _key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            ctx.emit(Action::EditExternally(asked()));
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    #[test]
    fn an_external_edit_from_a_tab_is_held_for_the_loop() {
        let (mut app, _rx, _seen) = shell();
        let (asker, heard) = Asker::new("asker");
        app.register_tab(Box::new(asker));
        app.update(Action::Tab(TabAction::Focus(TabId("asker"))));
        app.dirty = false;

        app.on_key(KeyEvent::from(KeyCode::Char('e')));
        assert!(app.dirty);
        assert_eq!(app.status, None);
        assert_eq!(
            app.take_external_edit(),
            Some((TabId("asker"), asked())),
            "stamped with the emitting tab"
        );
        assert_eq!(app.take_external_edit(), None, "taken once");
        assert!(heard.borrow().is_empty(), "nothing ran yet");
    }

    #[test]
    fn an_external_edit_from_an_overlay_is_refused_on_the_status_line() {
        let (mut app, _rx, _seen) = shell();
        app.push_overlay(Box::new(Popup));

        app.on_key(KeyEvent::from(KeyCode::Char('e')));
        assert_eq!(app.status.as_deref(), Some(EDITOR_NEEDS_A_TAB));
        assert_eq!(app.take_external_edit(), None);

        // The unstamped path: a keymap binding or the shell itself (`Origin::App`).
        app.status = None;
        app.update(Action::EditExternally(asked()));
        assert_eq!(app.status.as_deref(), Some(EDITOR_NEEDS_A_TAB));
        assert_eq!(app.take_external_edit(), None);
    }

    #[test]
    fn finish_external_edit_reaches_only_the_asking_tab() {
        let (mut app, _rx, _seen) = shell();
        let (first, first_heard) = Asker::new("first");
        let (second, second_heard) = Asker::new("second");
        app.register_tab(Box::new(first));
        app.register_tab(Box::new(second));
        app.dirty = false;

        let outcome = ExternalEditOutcome::Edited("new body\n".to_owned());
        app.finish_external_edit(TabId("second"), outcome.clone());
        assert_eq!(*second_heard.borrow(), vec![outcome]);
        assert!(first_heard.borrow().is_empty());
        assert_eq!(
            app.status.as_deref(),
            Some("second heard back"),
            "what the tab emitted was drained"
        );
        assert!(app.dirty);

        // A tab that is no longer registered: the outcome is dropped, nobody else hears it.
        app.finish_external_edit(
            TabId("gone"),
            ExternalEditOutcome::Unchanged { quick: false },
        );
        assert_eq!(first_heard.borrow().len(), 0);
        assert_eq!(second_heard.borrow().len(), 1);
    }

    /// MOD-7 D13, blueprint D25: the registration probe answers at `UNSOLICITED`, which the
    /// freshness gate drops, so the report is rendered above it, and no view sees the reply.
    #[test]
    fn a_box_probed_reply_lands_on_the_status_line_whatever_its_seq() {
        let (mut app, _rx, seen) = shell();
        let report = crate::agent_worker::BoxProbeReport {
            tools: 3,
            probed_tags: vec!["rust".to_owned()],
            ..Default::default()
        };
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::App,
            reply: StoreReply::BoxProbed(report.clone()),
        }));
        assert_eq!(app.status, Some(report.status_line()));
        assert!(seen.borrow().is_empty(), "no tab saw it");
    }

    /// What an [`Asking`] overlay was handed.
    type Answers = Rc<RefCell<Vec<StoreReply>>>;

    /// An overlay that asks for `Workspaces` on `s` and never on open, so a reopened one has no
    /// request of its own to overwrite the staleness entry with (MOD-64 D240, blueprint F8).
    struct Asking {
        seen: Answers,
    }

    impl Asking {
        const ID: OverlayId = OverlayId("asking");
    }

    impl Overlay for Asking {
        fn id(&self) -> OverlayId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Asking"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            if key.code == KeyCode::Char('s') {
                ctx.request(StoreRequest::Workspaces);
                return Handled::Consumed;
            }
            Handled::Pass
        }
        fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
            self.seen.borrow_mut().push(reply.clone());
        }
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    /// A shell with [`Asking`] registered, and what every instance of it is handed.
    fn asking_shell() -> (App, UnboundedReceiver<RequestEnvelope>, Answers) {
        let (mut app, rx, _seen) = shell();
        let answers = Answers::default();
        let shared = Rc::clone(&answers);
        app.overlay_factories.register(Asking::ID, move || {
            Box::new(Asking {
                seen: Rc::clone(&shared),
            })
        });
        (app, rx, answers)
    }

    /// Opens [`Asking`], presses `s` and answers the `seq` of the `Workspaces` it sent.
    fn open_and_ask(app: &mut App, rx: &mut UnboundedReceiver<RequestEnvelope>) -> u64 {
        app.update(Action::Overlay(OverlayAction::Open(Asking::ID)));
        app.on_key(KeyEvent::from(KeyCode::Char('s')));
        std::iter::from_fn(|| rx.try_recv().ok())
            .find(|envelope| {
                envelope.origin == Origin::Overlay(Asking::ID)
                    && matches!(envelope.request, StoreRequest::Workspaces)
            })
            .expect("the overlay asked")
            .seq
    }

    /// The `Workspaces` answer to `seq`, addressed to [`Asking`].
    fn answer(seq: u64) -> Action {
        Action::Reply(ReplyEnvelope {
            seq,
            origin: Origin::Overlay(Asking::ID),
            reply: StoreReply::Workspaces(Vec::new()),
        })
    }

    /// MOD-64 D240: `latest` is keyed by origin, so without the fix a reopened overlay under the
    /// same id takes the answer to a request its closed predecessor made.
    #[test]
    fn a_reply_to_a_closed_overlay_never_reaches_the_next_one() {
        let (mut app, mut rx, answers) = asking_shell();
        let seq = open_and_ask(&mut app, &mut rx);

        app.update(Action::Overlay(OverlayAction::Close));
        app.update(Action::Overlay(OverlayAction::Open(Asking::ID)));
        app.update(answer(seq));
        assert!(
            answers.borrow().is_empty(),
            "the reopened overlay never asked"
        );
    }

    #[test]
    fn close_all_and_a_scope_change_forget_an_overlays_requests_too() {
        let (mut app, mut rx, answers) = asking_shell();

        let seq = open_and_ask(&mut app, &mut rx);
        app.update(Action::Overlay(OverlayAction::CloseAll));
        app.update(Action::Overlay(OverlayAction::Open(Asking::ID)));
        app.update(answer(seq));
        assert!(answers.borrow().is_empty(), "CloseAll forgets");

        app.update(Action::Overlay(OverlayAction::CloseAll));
        let seq = open_and_ask(&mut app, &mut rx);
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        app.update(Action::Overlay(OverlayAction::Open(Asking::ID)));
        app.update(answer(seq));
        assert!(answers.borrow().is_empty(), "a scope change forgets");
    }

    #[test]
    fn closing_the_top_overlay_keeps_the_one_below_fresh() {
        let (mut app, mut rx, answers) = asking_shell();
        let seq = open_and_ask(&mut app, &mut rx);
        app.push_overlay(Box::new(Popup));

        app.update(Action::Overlay(OverlayAction::Close));
        app.update(answer(seq));
        let delivered = matches!(
            answers.borrow().as_slice(),
            [StoreReply::Workspaces(listed)] if listed.is_empty()
        );
        assert!(delivered, "the overlay below still waits for its answer");
    }

    /// What a [`Revealer`] was handed.
    type Revealed = Rc<RefCell<Vec<RevealTarget>>>;

    /// A tab that reveals items: it records the target and asks for `Items`, as the Backlog does
    /// for an item it has not loaded (MOD-64 D235). It asks for nothing on activation, so the
    /// `Items` on the wire is the reveal's.
    struct Revealer {
        revealed: Revealed,
    }

    impl Revealer {
        const ID: TabId = TabId("revealer");
    }

    impl Tab for Revealer {
        fn id(&self) -> TabId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Revealer"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
            self.revealed.borrow_mut().push(target.clone());
            ctx.request(StoreRequest::Items {
                scope: ctx.scope.clone(),
                filter: ItemFilter::default(),
                ready_here: false,
            });
            true
        }
    }

    /// A shell whose recorder tab is active, with a [`Revealer`] registered after it and named as
    /// the tab that reveals items.
    fn revealing_shell() -> (App, UnboundedReceiver<RequestEnvelope>, Revealed) {
        let (mut app, mut rx, _seen) = shell();
        let revealed = Revealed::default();
        app.register_tab(Box::new(Revealer {
            revealed: Rc::clone(&revealed),
        }));
        app.reveal_tabs = vec![(RevealKind::Item, Revealer::ID)];
        while rx.try_recv().is_ok() {}
        assert_eq!(app.tabs.active_id(), Some(Recorder::ID));
        (app, rx, revealed)
    }

    #[test]
    fn reveal_focuses_the_registered_tab_and_hands_it_the_target() {
        let (mut app, mut rx, revealed) = revealing_shell();
        let target = RevealTarget::Item {
            id: ItemId::new(),
            key: "FEAT-1".to_owned(),
        };

        app.update(Action::Reveal(target.clone()));
        assert_eq!(app.tabs.active_id(), Some(Revealer::ID));
        assert_eq!(*revealed.borrow(), vec![target]);
        let asked = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|envelope| matches!(envelope.request, StoreRequest::Items { .. }))
            .expect("what the tab emitted was drained");
        assert_eq!(asked.origin, Origin::Tab(Revealer::ID));
    }

    #[test]
    fn a_reveal_kind_no_tab_is_registered_for_does_nothing() {
        let (mut app, mut rx, revealed) = revealing_shell();

        app.update(Action::Reveal(RevealTarget::Requirement {
            id: htui_core::model::RequirementId::new(),
            key: "R-STO-1".to_owned(),
        }));
        assert_eq!(app.tabs.active_id(), Some(Recorder::ID));
        assert_eq!(app.status, None);
        assert!(revealed.borrow().is_empty());
        assert!(rx.try_recv().is_err(), "nothing is asked for");
    }

    // ---- MOD-71: the mouse seam (plan D1, D4) ------------------------------------------------

    /// What a [`Pointer`] was offered.
    type Pointed = Rc<RefCell<Vec<MouseEventKind>>>;

    /// A tab that wants the mouse while `wants` is set, answers `answer`, and logs every mouse
    /// event it is offered (MOD-71 D4).
    #[derive(Debug)]
    struct Pointer {
        wants: Rc<Cell<bool>>,
        answer: Handled,
        seen: Pointed,
    }

    impl Tab for Pointer {
        fn id(&self) -> TabId {
            TabId("pointer")
        }
        fn title(&self) -> &str {
            "Pointer"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn wants_mouse(&self) -> bool {
            self.wants.get()
        }
        fn on_mouse(&mut self, mouse: MouseEvent, _ctx: &mut Ctx<'_>) -> Handled {
            self.seen.borrow_mut().push(mouse.kind);
            self.answer
        }
    }

    /// A shell whose only tab is a [`Pointer`], `dirty` cleared.
    fn pointing(
        wants: bool,
        answer: Handled,
    ) -> (
        App,
        UnboundedReceiver<RequestEnvelope>,
        Rc<Cell<bool>>,
        Pointed,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx, Keymap::default_global());
        let wants = Rc::new(Cell::new(wants));
        let seen = Pointed::default();
        app.register_tab(Box::new(Pointer {
            wants: Rc::clone(&wants),
            answer,
            seen: Rc::clone(&seen),
        }));
        app.dirty = false;
        (app, rx, wants, seen)
    }

    /// `kind` at a fixed cell, no modifier.
    fn at(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// MOD-71 D1: every view the shell starts with keeps the terminal's own text selection.
    #[test]
    fn no_tab_wants_the_mouse_at_start_up() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx, Keymap::default_global());
        crate::app::register_all(&mut app);
        assert!(!app.tabs.is_empty());
        for i in 0..app.tabs.len() {
            assert!(app.tabs.select(i));
            assert!(!app.wants_mouse(), "{:?}", app.tabs.active_id());
        }
    }

    /// MOD-71 D4: an event queued before capture went off reaches nobody and costs nothing.
    #[test]
    fn a_mouse_event_nobody_wants_changes_nothing() {
        let (mut app, _rx, _wants, seen) = pointing(false, Handled::Consumed);
        app.status = Some("boom".into());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);
        assert_eq!(app.status.as_deref(), Some("boom"));
    }

    /// MOD-71 D4, blueprint H-5: capture is any-motion, so a pointer crossing the canvas must not
    /// redraw once per cell; the horizontal wheel has no meaning here either.
    #[test]
    fn motion_and_the_horizontal_wheel_never_reach_a_tab_or_redraw() {
        let (mut app, _rx, _wants, seen) = pointing(true, Handled::Consumed);
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ] {
            app.on_terminal_event(at(kind));
        }
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);
    }

    /// MOD-71 D4, blueprint E8: a consumed event redraws and, like a key, clears the status line.
    #[test]
    fn a_consumed_mouse_event_redraws_and_clears_the_status_line() {
        let (mut app, _rx, _wants, seen) = pointing(true, Handled::Consumed);
        app.status = Some("boom".into());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert_eq!(
            *seen.borrow(),
            vec![MouseEventKind::Down(MouseButton::Left)]
        );
        assert!(app.dirty);
        assert_eq!(app.status, None);
    }

    /// MOD-71 D4, blueprint E8: an event the tab passed did nothing, so it neither redraws nor
    /// wipes an error nobody acted on.
    #[test]
    fn a_passed_mouse_event_keeps_the_status_line_and_does_not_redraw() {
        let (mut app, _rx, _wants, seen) = pointing(true, Handled::Pass);
        app.status = Some("boom".into());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert_eq!(
            *seen.borrow(),
            vec![MouseEventKind::Down(MouseButton::Left)]
        );
        assert!(!app.dirty);
        assert_eq!(app.status.as_deref(), Some("boom"));
    }

    /// MOD-71 D1, blueprint E7: an overlay and the `?` box both draw over the tab, so either one
    /// takes the mouse away from it.
    #[test]
    fn an_overlay_or_the_help_box_takes_the_mouse_away() {
        let (mut app, _rx, _wants, seen) = pointing(true, Handled::Consumed);
        assert!(app.wants_mouse(), "the tab wants it with nothing over it");
        app.push_overlay(Box::new(Popup));
        app.dirty = false;
        assert!(!app.wants_mouse());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);

        let (mut app, _rx, _wants, seen) = pointing(true, Handled::Consumed);
        app.help_visible = true;
        assert!(!app.wants_mouse());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);
    }
}
