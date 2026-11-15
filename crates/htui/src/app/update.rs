//! `App::update`: the single place a state change happens (plan D5).
//!
//! Everything else in the crate produces [`Action`]s; this file is the only consumer. Adding a
//! feature adds an arm here and a file under `ui/`, never an arm in the event loop.

use htui_core::model::{InBackground, RunId, Scope, StepId, WorkspaceId, WorkspaceSummary};
use htui_core::qdrant::KNOWLEDGE_SEARCH_UNAVAILABLE;
use htui_orch::Command;

use crate::app::action::{
    Action, HistoryAction, LayoutAction, LoggingAction, OverlayAction, PaneAction, RevealTarget,
    TabAction,
};
use crate::app::state::{App, Ctx, EDITOR_NEEDS_A_TAB};
use crate::concepts_worker::QdrantCheck;
use crate::connection::DsnState;
use crate::logging::LoggingPrefs;
use crate::run_worker::OrchRequest;
use crate::store_worker::{Origin, ReplyEnvelope, StoreReply, StoreRequest};
use crate::ui::overlay::{Overlay, OverlayId};
use crate::ui::shell::PaneId;
use crate::ui::shell::activity::ActivityJump;
use crate::ui::shell::history::Loc;
use crate::ui::shell::layout as pane_layout;
use crate::ui::tabs::SettingsTab;
use crate::ui::tabs::settings::ConnectionSection;

/// The top bar's counts are re-read once a second, i.e. every fourth 250 ms tick.
const TICKS_PER_REFRESH: u64 = 4;

/// MOD-122 M3 D6 (blueprint F10): the retention sweep goes once an hour of 250 ms ticks.
const SWEEP_TICKS: u64 = 3600 * 4;

impl App {
    /// Applies one action, recording the move it made in the history (MOD-115 M2 §8: a reply
    /// that reveals or redirects is a new place too). The tick moves nobody and comes four times a
    /// second, so it is not recorded (RV2 M3); it also flashes a save the background writer
    /// could not make (RV2 H1).
    pub fn update(&mut self, action: Action) {
        if matches!(action, Action::Tick) {
            self.apply(action);
        } else {
            self.recorded(|app| app.apply(action));
        }
    }

    /// [`update`](Self::update)'s body.
    fn apply(&mut self, action: Action) {
        // MOD-122 M3 D8: a move that may show the activity pane reads it at once, not a second on.
        let may_show_activity = matches!(
            action,
            Action::Tab(_) | Action::Layout(_) | Action::Pane(_) | Action::History(_)
        );
        match action {
            Action::Quit => self.should_quit = true,
            Action::Tab(tab) => self.update_tab(tab),
            Action::Overlay(overlay) => self.update_overlay(overlay),
            Action::Store(request) => self.dispatch(Origin::App, request),
            Action::Reply(envelope) => self.on_reply(envelope),
            // MOD-57 M2 E5: `set_scope` resets every tab, the asking one included.
            Action::SetScope { workspace } => match self.editor_steer_refusal() {
                Some(refusal) => self.status = Some(refusal),
                None => self.set_scope(workspace),
            },
            Action::Replay { step_id } => self.replay(step_id),
            Action::Promote { run, step } => self.promote(run, step),
            Action::Reveal(target) => self.reveal(&target),
            // The unstamped path (the shell itself, `Origin::App`): only a tab can open the
            // editor, because only a tab can be handed the outcome (MOD-9 D10).
            Action::EditExternally(_) => self.status = Some(EDITOR_NEEDS_A_TAB.to_owned()),
            Action::AbortEdit => self.abort_editor(),
            Action::ToggleHelp => self.help_visible = !self.help_visible,
            Action::ToggleAbout => {
                self.about_visible = !self.about_visible;
                self.save_ui_state();
            }
            Action::Pane(pane) => self.step_pane(pane),
            Action::Layout(step) => self.change_layout(step),
            Action::History(step) => self.go_history(step),
            Action::Logging(change) => self.change_logging(change),
            Action::Error(message) => self.status = Some(message),
            Action::Tick => {
                self.on_tick();
                return;
            }
        }
        if may_show_activity {
            self.poll_activity();
        }
        self.dirty = true;
    }

    /// MOD-122 M3 D8: reads the activity feed for the panel while the active tab shows its
    /// pane, one read in flight at most: the newest page first, then from the panel's cursor.
    /// Sent as `Origin::App`, which is never forgotten, so every reply passes the freshness gate
    /// to [`App::on_app_reply`] (blueprint F9).
    fn poll_activity(&mut self) {
        if self.activity_shown() && self.activity.begin_request() {
            let after = self.activity.after();
            self.dispatch(Origin::App, StoreRequest::Activity { after });
        }
    }

    /// MOD-122 M3 D6 (blueprint F10): the retention sweep, on the first refresh tick and then
    /// every [`SWEEP_TICKS`]; a store that was down is swept by the next one. Idempotent, so no
    /// answer is waited on. Review M2: only to a store that is up (`top_bar.store` online or
    /// memory): one still connecting would answer nothing and put the next sweep an hour off,
    /// so the first goes on the first refresh after the store is up.
    fn sweep_retention(&mut self) {
        let up = matches!(self.top_bar.store.as_str(), "online" | "memory");
        if up && self.ticks >= self.sweep_due {
            self.sweep_due = self.ticks + SWEEP_TICKS;
            self.dispatch(Origin::App, StoreRequest::SweepRetention);
        }
    }

    /// MOD-122 M3 D9: `Enter` on an activity row. A chat replays; an item in this workspace is
    /// revealed; one in another workspace enters it first, then reveals (a scope change clears a
    /// pending reveal) and says so, unless an open in-pane edit refuses the move; a row whose
    /// item or run went says so and moves nothing.
    pub(super) fn activity_jump(&mut self, jump: ActivityJump) {
        match jump {
            ActivityJump::Replay { step } => self.update(Action::Replay { step_id: step }),
            ActivityJump::Gone { key } => self.status = Some(format!("{key} was deleted")),
            ActivityJump::Reveal {
                workspace: None,
                target,
            } => self.reveal_from_activity(target),
            ActivityJump::Reveal {
                workspace: Some(workspace),
                target,
            } => {
                if let Some(refusal) = self.editor_steer_refusal() {
                    self.status = Some(refusal);
                    return;
                }
                let key = match &target {
                    RevealTarget::Item { key, .. }
                    | RevealTarget::Step { key, .. }
                    | RevealTarget::Requirement { key, .. } => key.clone(),
                };
                let said = format!("opened workspace {} for {key}", workspace.name);
                self.update(Action::SetScope { workspace });
                self.reveal_from_activity(target);
                self.status = Some(said);
            }
        }
    }

    /// D9: reveals `target`, then focuses the detail of the tab that revealed it: the jump lands
    /// on the step, not back on the activity pane. A refused reveal moves nothing.
    fn reveal_from_activity(&mut self, target: RevealTarget) {
        let kind = target.kind();
        self.update(Action::Reveal(target));
        if let Some(tab) = self.tabs.active_id()
            && self.editor_tab() != Some(tab)
            && self
                .reveal_tabs
                .iter()
                .any(|(registered, id)| *registered == kind && *id == tab)
        {
            self.focus.insert(tab, PaneId::Detail);
        }
    }

    /// MOD-122 D5, D6: applies one logging setting through the control; on success records it in
    /// the UI state, saves `ui-state.toml` (the latest-wins writer) and logs the directive or the
    /// path, never a value read from the keyring or the environment. A refusal goes on the status
    /// line and nothing changes.
    fn change_logging(&mut self, change: LoggingAction) {
        let control = self.logging.control().clone();
        let applied = match &change {
            LoggingAction::Filter(directive) => control.set_filter(directive),
            LoggingAction::File(path) => control.set_file(path.as_deref()),
        };
        if let Err(err) = applied {
            self.status = Some(err.to_string());
            return;
        }
        let prefs = self.ui.logging.get_or_insert_with(LoggingPrefs::default);
        match change {
            LoggingAction::Filter(directive) => {
                let directive = directive.trim().to_owned();
                tracing::info!(filter = %directive, "log filter set in Settings");
                prefs.filter = Some(directive);
            }
            LoggingAction::File(path) => {
                tracing::info!(file = ?path, "log file set in Settings");
                prefs.file = path;
            }
        }
        self.save_ui_state();
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
            // MOD-57 M2 R2 LOW-3: the abort question asks on the asking tab only; left up over
            // another, its `editor.abort` would abort from there (E2).
            if self.editor_tab().is_some() && self.tabs.active_id() != self.editor_tab() {
                self.close_abort_question();
            }
            self.activate_tab();
        }
    }

    /// `global.next_pane`/`prev_pane` (MOD-115 M2 D3): the next visible pane of the panes the
    /// active tab claimed in the last frame. Nothing on a tab that claimed none (a single-pane
    /// tab).
    fn step_pane(&mut self, step: PaneAction) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let id = tab.id();
        let from = self.focus_of(tab);
        let Some((_, _, areas)) = self.last_claim.filter(|(claimed, _, _)| *claimed == id) else {
            return;
        };
        let to = pane_layout::step(&areas, from, step == PaneAction::Next);
        self.focus.insert(id, to);
    }

    /// One `[layout]` key on the active tab (MOD-115 M2 D2): the stored layout steps, a focused
    /// pane it hides gives the focus to the detail, and the UI state is saved. Nothing on a
    /// single-pane tab.
    fn change_layout(&mut self, step: LayoutAction) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let id = tab.id();
        let focus = self.focus_of(tab);
        let Some(mut layout) = self.layout_of(tab) else {
            return;
        };
        match step {
            LayoutAction::NextPreset => layout.next_preset(),
            LayoutAction::PrevPreset => layout.prev_preset(),
            LayoutAction::GrowList => layout.grow_list(),
            LayoutAction::ShrinkList => layout.shrink_list(),
            LayoutAction::GrowActivity => layout.grow_activity(),
            LayoutAction::ShrinkActivity => layout.shrink_activity(),
            LayoutAction::ToggleList => layout.toggle_list(),
            LayoutAction::ToggleActivity => layout.toggle_activity(),
            LayoutAction::Zoom => layout.toggle_zoom(),
        }
        self.ui.layouts.insert(id.0.to_owned(), layout);
        // The focus is stored as resolved, always: one the old layout only derived (the detail of
        // a restart onto a hidden list) stays put when the new layout shows the list again (T6R).
        let focus = if self.hides(id, &layout, focus) {
            PaneId::Detail
        } else {
            focus
        };
        self.focus.insert(id, focus);
        self.save_ui_state();
    }

    /// `history.back`/`forward` (MOD-115 M2 D5): to the place on the history, the place left
    /// kept for the way back; the move itself is not recorded.
    fn go_history(&mut self, step: HistoryAction) {
        let Some(here) = self.here() else {
            return;
        };
        let to = match step {
            HistoryAction::Back => self.history.back(here),
            HistoryAction::Forward => self.history.forward(here),
        };
        let Some(to) = to else {
            return;
        };
        self.history_move = true;
        self.go(&to);
    }

    /// Goes to `loc`: its tab, the focused pane and the tab's own location (`Tab::restore`, with
    /// the pane it restores focused in its `Ctx`, and what it emitted drained). A tab that is no
    /// longer registered is a no-op; the tab an in-pane editor is open for is focused but not
    /// restored (MOD-57 M2 E5: nothing steers it).
    fn go(&mut self, loc: &Loc) {
        if self.tabs.title(loc.tab).is_none() {
            return;
        }
        self.update_tab(TabAction::Focus(loc.tab));
        // D3: a pane the layout has hidden since the place was left gives the focus to the
        // detail, as a layout key that hides it does.
        let hidden = self
            .tabs
            .active()
            .filter(|tab| tab.id() == loc.tab)
            .and_then(|tab| self.layout_of(tab))
            .is_some_and(|layout| self.hides(loc.tab, &layout, loc.pane));
        let pane = if hidden { PaneId::Detail } else { loc.pane };
        self.focus.insert(loc.tab, pane);
        if self.editor_tab() != Some(loc.tab) {
            let _ = self.call_tab(loc.tab, |view, ctx| view.restore(&loc.at, ctx));
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

    /// Closes the overlay `id` wherever it is in the stack and forgets its requests (D240): the
    /// abort question when its edit ends (MOD-57 M2 R1 M-2). Nothing if it is not open.
    pub(super) fn close_overlay(&mut self, id: OverlayId) {
        if self.overlays.remove(id) {
            self.forget_overlay(id);
        }
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
        // RV2 H1: a save is written off the loop, so its failure is known only later.
        if let Some(reason) = self.ui_store.take_failure() {
            self.status = Some(reason);
            self.dirty = true;
        }
        self.say_qdrant_offer();
        self.say_qdrant_warning();
        if self.ticks.is_multiple_of(TICKS_PER_REFRESH) {
            self.dispatch(Origin::App, StoreRequest::StoreState);
            if !self.scope.is_empty() {
                let scope = self.scope.clone();
                self.dispatch(Origin::App, StoreRequest::Waiting { scope });
            }
            // MOD-116 M1 D5: every workspace's counts, for the header's background chip and the
            // Workspaces tab, which never polls them itself; M3 D16: once the scope names a
            // workspace, with a project or not (a new workspace starts with none).
            if self.scope.workspace_id != WorkspaceId::default() {
                self.dispatch(Origin::App, StoreRequest::WorkspaceRunCounts);
            }
            self.poll_activity();
            self.sweep_retention();
            self.refresh_active_tab();
            self.refresh_top_overlay();
            self.dirty = true;
        }
    }

    /// MOD-41 plan D16: the active tab's refresh hook, with a [`Ctx`] addressed as that tab, then
    /// what it emitted drained, as [`App::finish_external_edit`] does.
    fn refresh_active_tab(&mut self) {
        if let Some(id) = self.tabs.active_id() {
            let _ = self.call_tab(id, |view, ctx| view.on_refresh(ctx));
        }
    }

    /// MOD-12 M3 D9: the top overlay's refresh hook, with a [`Ctx`] addressed as that overlay, then
    /// what it emitted drained. Overlays below the top are not ticked.
    fn refresh_top_overlay(&mut self) {
        let Some(id) = self.overlays.top().map(Overlay::id) else {
            return;
        };
        let origin = Origin::Overlay(id);
        {
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
            let Some(top) = overlays.top_mut() else {
                return;
            };
            let mut ctx = Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
            top.refresh(&mut ctx);
        }
        self.drain(&origin);
    }

    /// Enters a workspace: the only path that changes the scope (plan D10).
    fn set_scope(&mut self, workspace: WorkspaceSummary) {
        // MOD-116 M1 D12: the workspace entered last, saved when it moved, for the next start
        // (never under `--demo`, R1 F5). The startup scope has set it already, unsaved (R1 F4).
        if self.remembers_workspace && self.ui.last_workspace != Some(workspace.workspace_id) {
            self.ui.last_workspace = Some(workspace.workspace_id);
            self.save_ui_state();
        }
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
        // MOD-116 M1 D5: the rows are every workspace's, so they stay; only which one is open
        // moved, and the chip is recounted from them at once, before the fresh read lands.
        self.recount_background();
        self.dispatch(Origin::App, StoreRequest::Waiting { scope });
        self.dispatch(Origin::App, StoreRequest::WorkspaceRunCounts);
    }

    /// MOD-116 M1 D4: the header's background chip from the cached run counts and the scope.
    fn recount_background(&mut self) {
        if let Some(rows) = &self.top_bar.workspace_runs {
            self.top_bar.background = InBackground::of(self.scope.workspace_id, rows);
        }
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
    /// nothing is registered for is a no-op, logged. MOD-57 M2 E5: a reveal into the tab an
    /// in-pane editor is open for is refused on the status line; it would re-select what the
    /// edit waits on.
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
        if self.editor_tab() == Some(tab) {
            self.status = self.editor_steer_refusal();
            return;
        }
        self.update_tab(TabAction::Focus(tab));
        if self.call_tab(tab, |view, ctx| view.reveal(target, ctx)) == Some(false) {
            tracing::debug!(%tab, "the tab does not reveal this target");
        }
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
        // A tab that draws the refusal itself keeps it off the line (MOD-146 R1 L4).
        if let StoreReply::Failed { request, message } = &envelope.reply
            && !self.absorbed(&envelope.origin, request)
        {
            self.update(Action::Error(format!("{request}: {message}")));
        }
        let ReplyEnvelope { origin, reply, .. } = envelope;
        match origin {
            Origin::App => self.on_app_reply(reply),
            Origin::Tab(id) => {
                let _ = self.call_tab(id, |tab, ctx| tab.on_reply(&reply, ctx));
            }
            Origin::Overlay(id) => {
                let origin = Origin::Overlay(id);
                {
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
                    // A reply to an overlay that has been popped is dropped silently.
                    if let Some(overlay) = overlays.by_id_mut(id) {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        overlay.on_reply(&reply, &mut ctx);
                    }
                }
                self.drain(&origin);
            }
        }
    }

    /// Whether the tab a refused `request` was issued from draws it itself
    /// ([`Tab::absorbs_failure`]). The shell's and an overlay's refusals are the line's, but for
    /// the polls below.
    ///
    /// One shell request is not the line's either: the launch's old-URL check (MOD-158 R1, C-L7),
    /// which the user never asked for. Its refusal is logged at `debug` instead.
    fn absorbed(&mut self, origin: &Origin, request: &str) -> bool {
        match origin {
            Origin::Tab(id) => self
                .tabs
                .by_id_mut(*id)
                .is_some_and(|tab| tab.absorbs_failure(request)),
            Origin::App if request == StoreRequest::QdrantOldUrlCheck.name() => {
                tracing::debug!("the launch's old Qdrant URL check was refused; nothing is said");
                true
            }
            // MOD-122 M3 D6, D8: the panel's poll and the retention sweep go on their own, once
            // a second and once an hour; a database that is down must not fill the line. The
            // panel keeps its rows (`on_app_reply` lets the next read go).
            // MOD-116 M1 D5: so does the counts read, once a second; the top bar keeps the last
            // rows.
            Origin::App
                if request == StoreRequest::Activity { after: None }.name()
                    || request == StoreRequest::SweepRetention.name()
                    || request == StoreRequest::WorkspaceRunCounts.name() =>
            {
                true
            }
            // MOD-116 M2 R1 F8: so does the leave dialog's own counts read while its stops
            // settle, once a second; it keeps the last rows and its bound still runs.
            Origin::Overlay(id)
                if *id == crate::ui::overlay::LeaveDialog::ID
                    && request == StoreRequest::WorkspaceRunCounts.name() =>
            {
                true
            }
            Origin::App | Origin::Overlay(_) => false,
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
            // MOD-116 M1 D5: whatever its origin, as `Waiting`; the rows are every workspace's,
            // so no scope check: the chip is counted against the scope in force now.
            StoreReply::WorkspaceRunCounts(rows) => {
                self.top_bar.workspace_runs = Some(rows.clone());
                self.recount_background();
            }
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
                // MOD-160 D9 (blueprint P-12): the launch's Qdrant probe, once the store is first
                // online: `App::start` runs while it is still `connecting`, when the URL row
                // cannot be read and a probe would say nothing. Off the loop, never blocking.
                if label == "online" && self.qdrant_offer_check && !self.qdrant_probe_sent {
                    self.qdrant_probe_sent = true;
                    self.dispatch(Origin::App, StoreRequest::ProbeQdrant);
                }
                if migrations_pending.is_some_and(|n| n > 0) {
                    self.offer_migration_prompt();
                }
                if let Some(target) = below_target {
                    self.note_below_target(target);
                }
            }
            StoreReply::MigrationsApplied { applied } => {
                self.status = Some(format!(
                    "applied {}",
                    crate::ui::kit::label::plural(*applied, "migration", "migrations")
                ));
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

    /// Says the waiting old-URL offer (MOD-158 D10) once the status line is free.
    fn say_qdrant_offer(&mut self) {
        if self.qdrant_offer_pending && self.status.is_none() {
            self.qdrant_offer_pending = false;
            self.status = Some(QDRANT_OFFER_NOTICE.to_owned());
            self.dirty = true;
        }
    }

    /// Says the launch's knowledge-search warning (MOD-160 D9) once the status line is free.
    fn say_qdrant_warning(&mut self) {
        if self.status.is_none()
            && let Some(said) = self.qdrant_warning_pending.take()
        {
            self.status = Some(said);
            self.dirty = true;
        }
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
    /// inside a workspace instead of an empty shell: the workspace entered last when the list
    /// still holds it, else the first (MOD-116 M1 D12; the first under `--demo`, R1 F5), set in
    /// memory and not saved (R1 F4). An empty list lands on `empty_tab` (the
    /// Workspaces tab, once registered) instead of leaving the shell blank (blueprint D,
    /// "Startup"; MOD-116 M1 D11). Nothing moves before that reply, so the first frame is always
    /// the shell itself.
    fn on_app_reply(&mut self, reply: StoreReply) {
        // MOD-122 M3 D8 (blueprint F9): the activity panel's read, after the freshness gate,
        // taken by value (review L2: it comes once a second). A refusal lets the next read go
        // and keeps the rows; a sweep's answer needs nothing, and its refusal, kept off the line
        // (`absorbed`), is logged (review L3).
        let reply = match reply {
            StoreReply::Activity { page, workspaces } => {
                self.activity.merge(page, workspaces);
                return;
            }
            reply => reply,
        };
        let reply = &reply;
        match reply {
            StoreReply::Failed { request, .. }
                if *request == StoreRequest::Activity { after: None }.name() =>
            {
                self.activity.failed();
            }
            StoreReply::Failed { request, message }
                if *request == StoreRequest::SweepRetention.name() =>
            {
                tracing::warn!(%message, "the retention sweep failed; the next goes in an hour");
            }
            _ => {}
        }

        if let StoreReply::Workspaces(workspaces) = reply
            && !self.has_scope()
        {
            let last = self
                .ui
                .last_workspace
                .filter(|_| self.remembers_workspace)
                .and_then(|last| {
                    workspaces
                        .iter()
                        .find(|workspace| workspace.workspace_id == last)
                });
            match last.or_else(|| workspaces.first()) {
                Some(open) => {
                    // R1 F4: the startup scope is no change of the user's: it is remembered in
                    // memory, so `set_scope` saves nothing, and a `ui-state.toml` that failed to
                    // load is left alone until the user changes something (MOD-115 M2 D1).
                    if self.remembers_workspace {
                        self.ui.last_workspace = Some(open.workspace_id);
                    }
                    self.update(Action::SetScope {
                        workspace: open.clone(),
                    });
                }
                None => {
                    if let Some(id) = self.empty_tab {
                        self.land_on(id);
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

        // MOD-158 D10: this box's keyring still holds the retired Qdrant URL. Said once per launch,
        // for `below_target_shown`'s reason; the offer itself is the section's row. It never
        // displaces a notice already on the line (the below-target warning, the ui-state load
        // notice: each said once): it waits, and `on_tick` says it once the line is clear.
        if let StoreReply::QdrantOldUrl(true) = reply
            && !self.qdrant_offer_noted
        {
            self.qdrant_offer_noted = true;
            self.qdrant_offer_pending = true;
            self.say_qdrant_offer();
        }

        // MOD-160 D9: the launch's probe found knowledge search unavailable. Said once per launch,
        // the state and where to look (the cause is Settings › Qdrant's row), and like the offer
        // it never displaces a notice already on the line. Ready, keyless (an info line, the
        // section's alone) and nothing probed say nothing. No modal, nothing blocked.
        if let StoreReply::QdrantProbe(check) = reply
            && !self.qdrant_warning_noted
        {
            self.qdrant_warning_noted = true;
            if let Some(state) = check.as_ref().and_then(QdrantCheck::state) {
                self.qdrant_warning_pending = Some(format!(
                    "{KNOWLEDGE_SEARCH_UNAVAILABLE}: {state} \u{2014} Settings \u{203a} Qdrant"
                ));
                self.say_qdrant_warning();
            }
        }
    }
}

/// The status line's old-URL sentence (MOD-158 D10). It promises no action (R1, C-L3): the old URL
/// can be used for every box only while no URL is set and it parses; otherwise only discarded.
pub(crate) const QDRANT_OFFER_NOTICE: &str =
    "this box's keyring holds an old Qdrant URL that is no longer read; see Settings > Qdrant";

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
    use crate::store_worker::{RequestEnvelope, UNSOLICITED};
    use crate::ui::overlay::{MigrationPrompt, Overlay, OverlayId, WaitingList};
    use crate::ui::tabs::{Tab, TabId};
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use htui_core::fixtures::ids;
    use htui_core::model::{
        InBackground, ItemFilter, ItemId, ItemKindId, ItemSummary, ProjectId, ProjectRef,
        RunStatus, Status, WorkspaceRunCounts,
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
            if let StoreReply::Items { items, .. } = reply {
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
        let mut app = App::new(tx);
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
            latest_run_status: None,
            priority: 0,
            required_tags: Vec::new(),
            updated_at: htui_core::fixtures::demo_at(0, 0),
            touched_paths: Vec::new(),
        };
        ReplyEnvelope {
            seq,
            origin: Origin::Tab(Recorder::ID),
            reply: StoreReply::Items {
                items: vec![row; count],
                scope_row: None,
            },
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

    /// A tab that only shows: for a second registered tab (MOD-116 M1 D11's empty tab).
    #[derive(Debug)]
    struct Plain(TabId);

    impl Tab for Plain {
        fn id(&self) -> TabId {
            self.0
        }
        fn title(&self) -> &str {
            "Plain"
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
    }

    /// A workspace named `name` with id `id` and no projects.
    fn named(id: WorkspaceId, name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: id,
            slug: name.to_lowercase(),
            name: name.to_owned(),
            projects: Vec::new(),
        }
    }

    /// `app.start()`, then the first `Workspaces` reply answered with `rows`.
    fn first_list(
        app: &mut App,
        rx: &mut UnboundedReceiver<RequestEnvelope>,
        rows: Vec<WorkspaceSummary>,
    ) {
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
            reply: StoreReply::Workspaces(rows),
        }));
    }

    /// MOD-116 M1 D12: the UI state's last workspace, still in the list, is the one entered.
    #[test]
    fn the_first_workspace_list_reopens_the_last_workspace() {
        let (mut app, mut rx, _seen) = shell();
        let graphics = WorkspaceId::new();
        app.ui.last_workspace = Some(graphics);
        first_list(
            &mut app,
            &mut rx,
            vec![
                named(WorkspaceId::new(), "Platform"),
                named(graphics, "Graphics"),
            ],
        );
        assert_eq!(app.top_bar.workspace, "Graphics");
        assert_eq!(app.scope.workspace_id, graphics);
    }

    /// D12: a last workspace the list no longer holds opens the first, as a first launch does.
    #[test]
    fn a_last_workspace_that_is_gone_opens_the_first() {
        let (mut app, mut rx, _seen) = shell();
        app.ui.last_workspace = Some(WorkspaceId::new());
        let platform = WorkspaceId::new();
        first_list(
            &mut app,
            &mut rx,
            vec![
                named(platform, "Platform"),
                named(WorkspaceId::new(), "Graphics"),
            ],
        );
        assert_eq!(app.top_bar.workspace, "Platform");
        assert_eq!(
            app.ui.last_workspace,
            Some(platform),
            "the first is now the last"
        );
    }

    /// MOD-116 M1 D11: an empty first list lands on the empty tab: no overlay opens and no
    /// history place is recorded.
    #[test]
    fn an_empty_first_list_lands_on_the_empty_tab() {
        const EMPTY: TabId = TabId("empty");
        let (mut app, mut rx, _seen) = shell();
        app.register_tab(Box::new(Plain(EMPTY)));
        app.empty_tab = Some(EMPTY);
        assert_eq!(app.tabs.active_id(), Some(Recorder::ID));
        first_list(&mut app, &mut rx, Vec::new());
        assert_eq!(app.tabs.active_id(), Some(EMPTY));
        assert!(app.overlays.is_empty(), "no overlay opened");
        assert!(app.history.peek_back().is_none(), "no history place");
        assert!(!app.has_scope());
    }

    /// D12: a scope change saves the last workspace when it moved, and only then.
    #[test]
    fn a_scope_change_saves_the_last_workspace_once() {
        use std::sync::{Arc, Mutex};

        use crate::ui::state_file::UiStore;

        let (mut app, _rx, _seen) = shell();
        let text = Arc::new(Mutex::new(None));
        app.ui_store = UiStore::Memory(Arc::clone(&text));
        let platform = WorkspaceId::new();
        app.update(Action::SetScope {
            workspace: named(platform, "Platform"),
        });
        let saved = text
            .lock()
            .expect("the lock")
            .take()
            .expect("the first entry saves");
        assert!(saved.contains(&format!("last = \"{platform}\"")), "{saved}");
        app.update(Action::SetScope {
            workspace: named(platform, "Platform"),
        });
        assert_eq!(
            *text.lock().expect("the lock"),
            None,
            "re-entering saves nothing"
        );
        app.update(Action::SetScope {
            workspace: named(WorkspaceId::new(), "Graphics"),
        });
        assert!(
            text.lock().expect("the lock").is_some(),
            "another workspace saves"
        );
    }

    /// MOD-116 M1 R1 F5: a shell that does not remember the workspace (`--demo`) neither reads
    /// nor saves it: the first list opens the first workspace, a scope change saves nothing, and
    /// a later save writes the file's last workspace back as it was read.
    #[test]
    fn a_shell_that_does_not_remember_the_workspace_neither_reads_nor_saves_it() {
        use std::sync::{Arc, Mutex};

        use crate::ui::state_file::UiStore;

        let (mut app, mut rx, _seen) = shell();
        let text = Arc::new(Mutex::new(None));
        app.ui_store = UiStore::Memory(Arc::clone(&text));
        app.remembers_workspace = false;
        let (platform, graphics) = (WorkspaceId::new(), WorkspaceId::new());
        app.ui.last_workspace = Some(graphics);
        first_list(
            &mut app,
            &mut rx,
            vec![named(platform, "Platform"), named(graphics, "Graphics")],
        );
        assert_eq!(
            app.scope.workspace_id, platform,
            "the first, not the saved one"
        );
        app.update(Action::SetScope {
            workspace: named(graphics, "Graphics"),
        });
        app.update(Action::SetScope {
            workspace: named(WorkspaceId::new(), "Other"),
        });
        assert_eq!(
            *text.lock().expect("the lock"),
            None,
            "no scope change saved"
        );
        assert_eq!(app.ui.last_workspace, Some(graphics));
        app.update(Action::ToggleAbout);
        let saved = text
            .lock()
            .expect("the lock")
            .take()
            .expect("the about toggle saves");
        assert!(saved.contains(&format!("last = \"{graphics}\"")), "{saved}");
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
        app.top_bar.store = "online".to_owned();
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        // MOD-122 M3 D6: the first refresh also sweeps a store that is up; `activity_tests`
        // pins its schedule.
        assert_eq!(requests.len(), 4, "one refresh per second, not per tick");
        assert!(matches!(requests[0].request, StoreRequest::StoreState));
        assert!(matches!(requests[1].request, StoreRequest::Waiting { .. }));
        assert!(matches!(
            requests[2].request,
            StoreRequest::WorkspaceRunCounts
        ));
        assert!(matches!(requests[3].request, StoreRequest::SweepRetention));
    }

    /// MOD-116 M3 D16: a workspace with no project (every one the Workspaces tab creates starts
    /// so) still reads its counts once a second, or its state and the background chip would
    /// freeze; `Waiting` keeps its project gate.
    #[test]
    fn a_workspace_with_no_project_still_reads_its_counts() {
        let (mut app, mut rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: WorkspaceSummary {
                projects: Vec::new(),
                ..workspace("Mobile App")
            },
        });
        assert!(app.scope.is_empty(), "a scope with no project");
        app.top_bar.store = "online".to_owned();
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(requests.len(), 3, "{requests:?}");
        assert!(matches!(requests[0].request, StoreRequest::StoreState));
        assert!(matches!(
            requests[1].request,
            StoreRequest::WorkspaceRunCounts
        ));
        assert!(matches!(requests[2].request, StoreRequest::SweepRetention));
    }

    /// `workspace`'s counts with `running` running runs and nothing else.
    fn running_in(workspace: WorkspaceId, running: usize) -> WorkspaceRunCounts {
        WorkspaceRunCounts::tally(workspace, vec![RunStatus::Running; running], None)
    }

    /// `name`'s summary under `workspace_id`, with [`workspace`]'s one project.
    fn workspace_at(name: &str, workspace_id: WorkspaceId) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id,
            ..workspace(name)
        }
    }

    /// MOD-116 M1 D5: the run counts reach the top bar whoever they are addressed to, as
    /// `Waiting` does: an overlay's reply and an unsolicited one both set them.
    #[test]
    fn a_run_counts_reply_sets_the_background_whatever_its_origin() {
        let (mut app, _rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace_at("Platform", ids::WORKSPACE_PLATFORM),
        });
        let rows = vec![
            running_in(ids::WORKSPACE_PLATFORM, 1),
            running_in(ids::WORKSPACE_GRAPHICS, 2),
        ];
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::Overlay(WaitingList::ID),
            reply: StoreReply::WorkspaceRunCounts(rows.clone()),
        }));
        assert_eq!(app.top_bar.workspace_runs.as_ref(), Some(&rows));
        assert_eq!(
            app.top_bar.background,
            InBackground {
                workspaces: 1,
                running: 2
            }
        );

        let fewer = vec![
            running_in(ids::WORKSPACE_PLATFORM, 1),
            running_in(ids::WORKSPACE_GRAPHICS, 3),
        ];
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::App,
            reply: StoreReply::WorkspaceRunCounts(fewer.clone()),
        }));
        assert_eq!(app.top_bar.workspace_runs, Some(fewer));
        assert_eq!(
            app.top_bar.background,
            InBackground {
                workspaces: 1,
                running: 3
            }
        );
    }

    /// MOD-116 M1 D5: entering another workspace recounts the chip from the cached rows at once
    /// (the one left becomes background, the one entered stops being so) and asks again.
    #[test]
    fn a_scope_change_recounts_the_background_and_rereads() {
        let (mut app, mut rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace_at("Platform", ids::WORKSPACE_PLATFORM),
        });
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::App,
            reply: StoreReply::WorkspaceRunCounts(vec![
                running_in(ids::WORKSPACE_PLATFORM, 1),
                running_in(ids::WORKSPACE_GRAPHICS, 2),
            ]),
        }));
        assert_eq!(
            app.top_bar.background,
            InBackground {
                workspaces: 1,
                running: 2
            }
        );
        while rx.try_recv().is_ok() {}

        app.update(Action::SetScope {
            workspace: workspace_at("Graphics", ids::WORKSPACE_GRAPHICS),
        });
        assert_eq!(
            app.top_bar.background,
            InBackground {
                workspaces: 1,
                running: 1
            },
            "recounted before any reply"
        );
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|envelope| envelope.request)
            .collect();
        let waiting = requests
            .iter()
            .position(|request| matches!(request, StoreRequest::Waiting { .. }))
            .expect("the waiting list is re-read");
        let counts = requests
            .iter()
            .position(|request| matches!(request, StoreRequest::WorkspaceRunCounts))
            .expect("the counts are re-read");
        assert!(waiting < counts, "after `Waiting`: {requests:?}");
    }

    /// MOD-116 M1 D5: the shell's once-a-second counts read goes on its own, so a database that
    /// is down must not fill the status line; the last rows are kept.
    #[test]
    fn a_refused_run_counts_read_stays_off_the_status_line() {
        let (mut app, mut rx, _seen) = shell();
        app.update(Action::SetScope {
            workspace: workspace_at("Platform", ids::WORKSPACE_PLATFORM),
        });
        let rows = vec![running_in(ids::WORKSPACE_GRAPHICS, 2)];
        app.update(Action::Reply(ReplyEnvelope {
            seq: UNSOLICITED,
            origin: Origin::App,
            reply: StoreReply::WorkspaceRunCounts(rows.clone()),
        }));
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let seq = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|envelope| matches!(envelope.request, StoreRequest::WorkspaceRunCounts))
            .expect("the tick asked")
            .seq;
        app.update(Action::Reply(ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply: StoreReply::Failed {
                request: StoreRequest::WorkspaceRunCounts.name(),
                message: htui_store::DATABASE_UNREACHABLE.to_owned(),
            },
        }));
        assert_eq!(app.status, None, "{:?}", app.status);
        assert_eq!(app.top_bar.workspace_runs, Some(rows), "the last rows stay");
        assert_eq!(
            app.top_bar.background,
            InBackground {
                workspaces: 1,
                running: 2
            }
        );
    }

    /// An overlay that counts the refresh ticks it is handed (MOD-12 M3 D9).
    struct Ticked {
        id: OverlayId,
        refreshed: Rc<Cell<usize>>,
    }

    impl Overlay for Ticked {
        fn id(&self) -> OverlayId {
            self.id
        }
        fn title(&self) -> &str {
            self.id.0
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn refresh(&mut self, ctx: &mut Ctx<'_>) {
            self.refreshed.set(self.refreshed.get() + 1);
            ctx.request(StoreRequest::QueueOverview);
        }
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    /// MOD-12 M3 D9: the refresh tick reaches the top overlay once a second, and only the top
    /// one; what it requests leaves addressed to it.
    #[test]
    fn the_refresh_tick_refreshes_the_top_overlay_only() {
        let (mut app, mut rx, _seen) = shell();
        let (lower, top) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        app.push_overlay(Box::new(Ticked {
            id: OverlayId("lower"),
            refreshed: Rc::clone(&lower),
        }));
        app.push_overlay(Box::new(Ticked {
            id: OverlayId("top"),
            refreshed: Rc::clone(&top),
        }));
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        assert_eq!((lower.get(), top.get()), (0, 1));
        let asked: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|envelope| matches!(envelope.request, StoreRequest::QueueOverview))
            .collect();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].origin, Origin::Overlay(OverlayId("top")));
    }

    #[test]
    fn the_store_state_refresh_is_not_gated_on_a_scope() {
        // A first launch never enters a workspace, and `offline · 3m` still has to keep ticking.
        let (mut app, mut rx, _seen) = shell();
        app.top_bar.store = "online".to_owned();
        while rx.try_recv().is_ok() {}
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        let requests: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        // MOD-122 M3 D6: nor is the retention sweep, which the first refresh sends a store
        // that is up.
        assert_eq!(requests.len(), 2);
        assert!(matches!(requests[0].request, StoreRequest::StoreState));
        assert!(matches!(requests[1].request, StoreRequest::SweepRetention));
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

    /// The old-URL check `start` sent, if any: its `seq`. Never the whole `QdrantInfo`
    /// (MOD-158 R1, C-M2): the launch reads the retired keyring entry and nothing else.
    fn qdrant_info_seq(rx: &mut UnboundedReceiver<RequestEnvelope>) -> Option<u64> {
        let mut found = None;
        while let Ok(envelope) = rx.try_recv() {
            assert!(
                !matches!(envelope.request, StoreRequest::QdrantInfo),
                "start never reads the whole Qdrant snapshot"
            );
            if matches!(envelope.request, StoreRequest::QdrantOldUrlCheck) {
                assert_eq!(envelope.origin, Origin::App, "the shell's own read");
                found = Some(envelope.seq);
            }
        }
        found
    }

    /// The old-URL check's answer addressed to the shell, with or without an old keyring URL.
    fn qdrant_reply(seq: u64, old_url: bool) -> ReplyEnvelope {
        ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply: StoreReply::QdrantOldUrl(old_url),
        }
    }

    /// MOD-158 R1 (C-L7): the launch's check is one the user never asked for, so its refusal (a
    /// keyring task that failed to join) is logged, never put on the status line.
    #[test]
    fn a_failed_launch_check_stays_off_the_status_line() {
        let (mut app, mut rx, _seen) = shell();
        app.qdrant_offer_check = true;
        app.start();
        let seq = qdrant_info_seq(&mut rx).expect("start asked");
        app.update(Action::Reply(ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply: StoreReply::Failed {
                request: "qdrant_old_url_check",
                message: "keyring task failed: panicked".to_owned(),
            },
        }));
        assert_eq!(app.status, None, "{:?}", app.status);
    }

    /// MOD-158 R1 (C-L3): the notice is true whether the old URL can be used for every box or
    /// only discarded: it names the section, not an action it may not offer.
    #[test]
    fn the_offer_notice_promises_no_action() {
        assert!(QDRANT_OFFER_NOTICE.contains("Settings > Qdrant"));
        assert!(
            !QDRANT_OFFER_NOTICE.contains("use it"),
            "{QDRANT_OFFER_NOTICE}"
        );
        assert!(
            !QDRANT_OFFER_NOTICE.contains("for every box"),
            "{QDRANT_OFFER_NOTICE}"
        );
    }

    /// MOD-158 D10, blueprint P-6: `start` asks for the Qdrant snapshot only once `register_all`
    /// has run, as the binary does; a bare shell (every test harness) never reads a keyring for it.
    #[test]
    fn start_asks_for_the_offer_only_after_register_all() {
        let (mut app, mut rx, _seen) = shell();
        app.start();
        assert_eq!(qdrant_info_seq(&mut rx), None, "a bare shell asks nothing");

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        crate::app::register_all(&mut app);
        app.start();
        assert!(
            qdrant_info_seq(&mut rx).is_some(),
            "after register_all, start asks"
        );
    }

    /// MOD-158 D10: an old keyring URL is said once per launch on the status line, and a snapshot
    /// without one says nothing.
    #[test]
    fn the_qdrant_offer_is_said_once_per_launch() {
        let (mut app, mut rx, _seen) = shell();
        app.qdrant_offer_check = true;
        app.start();
        let seq = qdrant_info_seq(&mut rx).expect("start asked");

        app.update(Action::Reply(qdrant_reply(seq, false)));
        assert_eq!(app.status, None, "no old URL, no notice");

        app.update(Action::Reply(qdrant_reply(seq, true)));
        assert_eq!(app.status.as_deref(), Some(QDRANT_OFFER_NOTICE));
        assert!(
            QDRANT_OFFER_NOTICE.contains("Settings > Qdrant"),
            "{QDRANT_OFFER_NOTICE}"
        );

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(app.status, None, "the next key clears it");
        app.update(Action::Reply(qdrant_reply(seq, true)));
        assert_eq!(app.status, None, "said once per launch");
    }

    /// MOD-158 D10, repair 1: the offer never displaces a notice already on the line (the
    /// below-target warning or the ui-state load notice, each said once per launch). It waits for
    /// the line to clear and is said on the next tick after that, still once.
    #[test]
    fn the_qdrant_offer_waits_for_a_busy_status_line() {
        let (mut app, mut rx, _seen) = shell();
        app.qdrant_offer_check = true;
        app.start();
        let seq = qdrant_info_seq(&mut rx).expect("start asked");

        app.note_below_target("9.9.9");
        let below = app.status.clone().expect("the below-target warning");

        app.update(Action::Reply(qdrant_reply(seq, true)));
        assert_eq!(
            app.status.as_deref(),
            Some(below.as_str()),
            "the warning stays"
        );
        app.update(Action::Tick);
        assert_eq!(app.status.as_deref(), Some(below.as_str()), "still busy");

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(app.status, None, "the key clears the warning");
        app.update(Action::Tick);
        assert_eq!(
            app.status.as_deref(),
            Some(QDRANT_OFFER_NOTICE),
            "then the offer"
        );

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        app.update(Action::Tick);
        app.update(Action::Reply(qdrant_reply(seq, true)));
        assert_eq!(app.status, None, "said once per launch");
    }

    /// The `seq` of every Qdrant probe on the wire, each the shell's own (MOD-160 D9).
    fn probe_seqs(rx: &mut UnboundedReceiver<RequestEnvelope>) -> Vec<u64> {
        let mut found = Vec::new();
        while let Ok(envelope) = rx.try_recv() {
            if matches!(envelope.request, StoreRequest::ProbeQdrant) {
                assert_eq!(envelope.origin, Origin::App, "the shell's own probe");
                found.push(envelope.seq);
            }
        }
        found
    }

    /// The probe's answer addressed to the shell.
    fn probe_reply(seq: u64, check: Option<QdrantCheck>) -> ReplyEnvelope {
        ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply: StoreReply::QdrantProbe(check),
        }
    }

    /// A shell after `register_all`'s flag and `start`, its launch requests drained.
    fn launched() -> (App, UnboundedReceiver<RequestEnvelope>) {
        let (mut app, mut rx, _seen) = shell();
        app.qdrant_offer_check = true;
        app.start();
        assert!(probe_seqs(&mut rx).is_empty(), "start probes nothing yet");
        (app, rx)
    }

    /// MOD-160 D9, blueprint P-12: the launch probe waits for the store to be online (a probe
    /// while `connecting` would resolve nothing), and is sent once per launch, not on every
    /// `StoreState` re-read nor after a reconnect.
    #[test]
    fn the_qdrant_probe_is_sent_once_when_the_store_is_first_online() {
        let (mut app, mut rx) = launched();
        app.update(Action::Reply(store_state("connecting", None)));
        assert!(probe_seqs(&mut rx).is_empty(), "not while connecting");

        app.update(Action::Reply(store_state("online", None)));
        assert_eq!(probe_seqs(&mut rx).len(), 1, "once the store is online");

        for label in ["online", "offline \u{b7} 3s", "online"] {
            app.update(Action::Reply(store_state(label, None)));
        }
        assert!(probe_seqs(&mut rx).is_empty(), "once per launch");
    }

    /// MOD-160 D9 (P-12, MOD-158 P-6): a shell `register_all` never ran on (every test harness)
    /// sends no launch probe.
    #[test]
    fn no_probe_without_register_all() {
        let (mut app, mut rx, _seen) = shell();
        app.start();
        app.update(Action::Reply(store_state("online", None)));
        assert!(probe_seqs(&mut rx).is_empty(), "{:?}", app.status);
    }

    /// MOD-160 D9: a warning state is said once on the status line, naming the state and where to
    /// look, never its cause's key; it never displaces an earlier notice: it waits for the line to
    /// clear, as the old-URL offer does.
    #[test]
    fn a_warning_is_said_once_and_never_over_an_earlier_notice() {
        let (mut app, mut rx) = launched();
        app.update(Action::Reply(store_state("online", None)));
        let [seq] = probe_seqs(&mut rx)[..] else {
            panic!("one probe")
        };
        let said = "knowledge search unavailable: this box lacks the Qdrant API key \
                    \u{2014} Settings \u{203a} Qdrant";

        app.note_below_target("9.9.9");
        let below = app.status.clone().expect("the below-target warning");
        let check = QdrantCheck::NoKey("qdrant: access refused".to_owned());
        app.update(Action::Reply(probe_reply(seq, Some(check.clone()))));
        assert_eq!(
            app.status.as_deref(),
            Some(below.as_str()),
            "the warning stays"
        );
        app.update(Action::Tick);
        assert_eq!(app.status.as_deref(), Some(below.as_str()), "still busy");

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        assert_eq!(app.status, None, "the key clears the warning");
        app.update(Action::Tick);
        assert_eq!(app.status.as_deref(), Some(said), "then the probe's");

        app.on_key(KeyEvent::from(KeyCode::Char('x')));
        app.update(Action::Tick);
        app.update(Action::Reply(probe_reply(seq, Some(check))));
        app.update(Action::Tick);
        assert_eq!(app.status, None, "said once per launch");
    }

    /// MOD-160 D9: a free status line says the warning at once.
    #[test]
    fn a_warning_on_a_free_line_is_said_at_once() {
        let (mut app, mut rx) = launched();
        app.update(Action::Reply(store_state("online", None)));
        let [seq] = probe_seqs(&mut rx)[..] else {
            panic!("one probe")
        };
        app.update(Action::Reply(probe_reply(
            seq,
            Some(QdrantCheck::NotConfigured(
                "no Qdrant URL is set".to_owned(),
            )),
        )));
        assert_eq!(
            app.status.as_deref(),
            Some(
                "knowledge search unavailable: no Qdrant is configured \u{2014} Settings \u{203a} \
                 Qdrant"
            )
        );
    }

    /// MOD-160 R1 (review gate): settings this box cannot read (the `qdrant_url` row, or its
    /// keyring) never reached Qdrant, so the notice says so and never calls Qdrant unreachable;
    /// the cause stays Settings › Qdrant's row.
    #[test]
    fn unreadable_settings_are_never_said_unreachable() {
        let (mut app, mut rx) = launched();
        app.update(Action::Reply(store_state("online", None)));
        let [seq] = probe_seqs(&mut rx)[..] else {
            panic!("one probe")
        };
        app.update(Action::Reply(probe_reply(
            seq,
            Some(QdrantCheck::Unreadable(
                "the keyring cannot be read: no Secret Service".to_owned(),
            )),
        )));
        let said = app.status.clone().expect("the notice");
        assert_eq!(
            said,
            "knowledge search unavailable: this box cannot read the Qdrant settings \u{2014} \
             Settings \u{203a} Qdrant"
        );
        assert!(!said.contains("unreachable"), "{said}");
        assert!(
            !said.contains("Secret Service"),
            "the cause is the row's: {said}"
        );
    }

    /// MOD-160 D9, Q4: a ready Qdrant, a keyless one (an info line, Settings' alone) and nothing
    /// probed (offline, no runtime) say nothing at launch.
    #[test]
    fn ready_keyless_and_nothing_probed_say_nothing() {
        for check in [Some(QdrantCheck::Ready), Some(QdrantCheck::Keyless), None] {
            let (mut app, mut rx) = launched();
            app.update(Action::Reply(store_state("online", None)));
            let [seq] = probe_seqs(&mut rx)[..] else {
                panic!("one probe")
            };
            app.update(Action::Reply(probe_reply(seq, check.clone())));
            app.update(Action::Tick);
            assert_eq!(app.status, None, "{check:?}");
        }
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
        assert_eq!(app.status.as_deref(), Some("applied 2 migrations"));
    }

    /// MOD-115 M4 D-2: the count reads as a word, never `(s)`.
    #[test]
    fn one_applied_migration_is_one_migration() {
        let (mut app, _rx, _seen) = shell();
        app.update(Action::Reply(ReplyEnvelope {
            seq: 0,
            origin: Origin::App,
            reply: StoreReply::MigrationsApplied { applied: 1 },
        }));
        assert_eq!(app.status.as_deref(), Some("applied 1 migration"));
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

        // The unstamped path: the shell itself (`Origin::App`).
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
        /// How many times it was told capture went off (MOD-74 D1).
        lost: Lost,
    }

    /// How many capture losses a [`Pointer`] was told of (MOD-74 D1).
    type Lost = Rc<Cell<usize>>;

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
        fn on_mouse_lost(&mut self) {
            self.lost.set(self.lost.get() + 1);
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
        Lost,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        let wants = Rc::new(Cell::new(wants));
        let seen = Pointed::default();
        let lost = Lost::default();
        app.register_tab(Box::new(Pointer {
            wants: Rc::clone(&wants),
            answer,
            seen: Rc::clone(&seen),
            lost: Rc::clone(&lost),
        }));
        app.dirty = false;
        (app, rx, wants, seen, lost)
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

    /// MOD-71 D1: every single-pane view the shell starts with keeps the terminal's own text
    /// selection; MOD-115 M2 MD8 (T7): a tab with a list and a detail takes the mouse for pane
    /// focus and seams.
    #[test]
    fn only_a_list_and_detail_tab_wants_the_mouse_at_start_up() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        crate::app::register_all(&mut app);
        assert!(!app.tabs.is_empty());
        for i in 0..app.tabs.len() {
            assert!(app.tabs.select(i));
            let split = app
                .tabs
                .active()
                .is_some_and(|tab| matches!(tab.panes(), crate::ui::shell::Panes::ListDetail(_)));
            assert_eq!(app.wants_mouse(), split, "{:?}", app.tabs.active_id());
        }
    }

    /// MOD-71 D4: an event queued before capture went off reaches nobody and costs nothing.
    #[test]
    fn a_mouse_event_nobody_wants_changes_nothing() {
        let (mut app, _rx, _wants, seen, _lost) = pointing(false, Handled::Consumed);
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
        let (mut app, _rx, _wants, seen, _lost) = pointing(true, Handled::Consumed);
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
        let (mut app, _rx, _wants, seen, _lost) = pointing(true, Handled::Consumed);
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
        let (mut app, _rx, _wants, seen, _lost) = pointing(true, Handled::Pass);
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
        let (mut app, _rx, _wants, seen, _lost) = pointing(true, Handled::Consumed);
        assert!(app.wants_mouse(), "the tab wants it with nothing over it");
        app.push_overlay(Box::new(Popup));
        app.dirty = false;
        assert!(!app.wants_mouse());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);

        let (mut app, _rx, _wants, seen, _lost) = pointing(true, Handled::Consumed);
        app.help_visible = true;
        assert!(!app.wants_mouse());
        app.on_terminal_event(at(MouseEventKind::Down(MouseButton::Left)));
        assert!(seen.borrow().is_empty());
        assert!(!app.dirty);
    }

    // ---- MOD-74: a lost capture (plan D1, D2) -----------------------------------------------

    /// MOD-74 D1: only an on-to-off edge tells the tabs; off staying off and on staying on never
    /// do.
    #[test]
    fn only_an_on_to_off_edge_is_a_lost_capture() {
        let (mut app, _rx, wants, _seen, lost) = pointing(false, Handled::Consumed);
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 0, "off to off");
        wants.set(true);
        assert!(app.mouse_capture());
        assert!(app.mouse_capture());
        assert_eq!(lost.get(), 0, "off to on, on to on");
        wants.set(false);
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 1, "on to off");
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 1, "off stays off");
    }

    /// MOD-74 D1: an overlay or the `?` box opening over a tab that held capture is a loss.
    #[test]
    fn an_overlay_or_the_help_box_opening_is_a_lost_capture() {
        let (mut app, _rx, _wants, _seen, lost) = pointing(true, Handled::Consumed);
        assert!(app.mouse_capture());
        app.push_overlay(Box::new(Popup));
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 1, "the overlay");

        let (mut app, _rx, _wants, _seen, lost) = pointing(true, Handled::Consumed);
        assert!(app.mouse_capture());
        app.help_visible = true;
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 1, "the help box");
    }

    /// MOD-74 D1: after a tab switch the tab holding the gesture is no longer the active one, and
    /// it is still told.
    #[test]
    fn a_tab_switch_tells_the_tab_it_left() {
        let (mut app, _rx, _wants, _seen, lost) = pointing(true, Handled::Consumed);
        let (asker, _heard) = Asker::new("asker");
        app.register_tab(Box::new(asker));
        assert_eq!(app.tabs.active_id(), Some(TabId("pointer")));
        assert!(app.mouse_capture());
        app.update(Action::Tab(TabAction::Focus(TabId("asker"))));
        assert_eq!(app.tabs.active_id(), Some(TabId("asker")));
        assert!(!app.mouse_capture());
        assert_eq!(lost.get(), 1, "the inactive pointer is told");
    }

    /// MOD-74 D2: `Suspend::leave` turns capture off without the app knowing, so an editor
    /// handoff under capture is a loss, and the next `mouse_capture` turns it back on with no
    /// second broadcast.
    #[test]
    fn an_editor_handoff_under_capture_is_a_lost_capture() {
        let (mut app, _rx, _wants, _seen, lost) = pointing(true, Handled::Consumed);
        assert!(app.mouse_capture());
        app.pending_edit = Some((TabId("pointer"), asked()));
        assert!(app.take_external_edit().is_some());
        assert_eq!(lost.get(), 1, "the handoff is a loss");
        assert!(!app.mouse, "the editor's `leave` turns capture off");
        assert!(app.mouse_capture(), "back from the editor, on again");
        assert_eq!(lost.get(), 1, "with no second broadcast");
        assert!(app.take_external_edit().is_none());
        assert_eq!(lost.get(), 1, "nothing pending is no handoff");

        let (mut app, _rx, _wants, _seen, lost) = pointing(true, Handled::Consumed);
        app.pending_edit = Some((TabId("pointer"), asked()));
        assert!(app.take_external_edit().is_some());
        assert_eq!(lost.get(), 0, "capture never applied: nothing to lose");
    }

    /// The text a memory store holds.
    fn stored(store: &crate::ui::state_file::UiStore) -> Option<String> {
        let crate::ui::state_file::UiStore::Memory(text) = store else {
            panic!("a memory store");
        };
        text.lock().expect("unpoisoned").clone()
    }

    /// MOD-122 D5: a logging setting applies through the control and is saved in
    /// `ui-state.toml` beside what was there; nothing else moves.
    #[test]
    fn a_logging_action_applies_and_saves() {
        use crate::app::LoggingAction;
        use crate::logging::LoggingPrefs;
        use crate::ui::shell::{ListSize, TabLayout};
        use crate::ui::state_file::UiStore;

        let (mut app, _rx, _seen) = shell();
        let store = UiStore::memory();
        app.ui_store = store.clone();
        let layout = TabLayout {
            list: ListSize::Percent(55),
            ..TabLayout::DEFAULT
        };
        app.ui.layouts.insert("backlog".to_owned(), layout);

        app.update(Action::Logging(LoggingAction::Filter("debug".to_owned())));
        assert_eq!(app.status, None);
        assert_eq!(app.logging.control().state().filter, "debug");
        let saved = store.load().state;
        assert_eq!(
            saved.logging,
            Some(LoggingPrefs {
                filter: Some("debug".to_owned()),
                file: None,
            })
        );
        assert_eq!(saved.layouts.get("backlog"), Some(&layout));

        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("htui.log");
        app.update(Action::Logging(LoggingAction::File(Some(file.clone()))));
        assert_eq!(
            app.logging.control().state().file.as_deref(),
            Some(file.as_path())
        );
        assert_eq!(
            store.load().state.logging,
            Some(LoggingPrefs {
                filter: Some("debug".to_owned()),
                file: Some(file),
            })
        );

        app.update(Action::Logging(LoggingAction::File(None)));
        assert_eq!(app.logging.control().state().file, None);
        let saved = store.load().state;
        assert_eq!(
            saved.logging,
            Some(LoggingPrefs {
                filter: Some("debug".to_owned()),
                file: None,
            })
        );
        assert_eq!(saved.layouts.get("backlog"), Some(&layout));
    }

    /// MOD-122 D5: a setting that cannot apply says why on the status line; nothing changes and
    /// nothing is saved.
    #[test]
    fn a_refused_logging_action_says_why_and_saves_nothing() {
        use crate::app::LoggingAction;
        use crate::ui::state_file::UiStore;

        let (mut app, _rx, _seen) = shell();
        let store = UiStore::memory();
        app.ui_store = store.clone();
        let dir = tempfile::tempdir().expect("a temp dir");

        app.update(Action::Logging(LoggingAction::File(Some(
            dir.path().to_path_buf(),
        ))));
        let status = app.status.clone().expect("the refusal is said");
        assert!(
            status.contains(&dir.path().display().to_string()),
            "{status}"
        );
        assert_eq!(app.logging.control().state().file, None);
        assert_eq!(stored(&store), None, "nothing is saved");

        app.update(Action::Logging(LoggingAction::Filter("htui=[".to_owned())));
        let status = app.status.clone().expect("the refusal is said");
        assert!(
            status.starts_with("`htui=[` is not a log filter: "),
            "{status}"
        );
        assert_eq!(app.logging.control().state().filter, "info");
        assert_eq!(stored(&store), None, "nothing is saved");
        assert_eq!(app.ui.logging, None);
    }
}

/// MOD-122 M3 T5 (D6, D8, blueprint F9, F10): the shell's activity poll, its replies and the
/// retention sweep, read off the wire the store worker would read.
#[cfg(test)]
mod activity_tests {
    use htui_core::fixtures::{demo_at, ids};
    use htui_core::model::activity::{ActivityEvent, ActivityKind, ActivityPage};
    use tokio::sync::mpsc::{self, UnboundedReceiver};

    use super::{SWEEP_TICKS, TICKS_PER_REFRESH};
    use crate::app::Action;
    use crate::app::state::App;
    use crate::app::state::shell_tests::{SPLIT, SPLIT_LAYOUT, SplitTab};
    use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};
    use crate::ui::shell::{Preset, TabLayout};

    /// A shell whose one list + detail tab is laid out in `preset`, and the worker's inbox.
    fn shell(preset: Preset) -> (App, UnboundedReceiver<RequestEnvelope>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.register_tab(Box::new(SplitTab::new(SPLIT, "Backlog")));
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                preset,
                ..SPLIT_LAYOUT
            },
        );
        (app, rx)
    }

    /// Everything sent since the last look.
    fn sent(rx: &mut UnboundedReceiver<RequestEnvelope>) -> Vec<RequestEnvelope> {
        let mut out = Vec::new();
        while let Ok(envelope) = rx.try_recv() {
            out.push(envelope);
        }
        out
    }

    /// The activity reads among `sent`: their `seq` and cursor.
    fn reads(sent: &[RequestEnvelope]) -> Vec<(u64, Option<i64>)> {
        sent.iter()
            .filter_map(|envelope| match envelope.request {
                StoreRequest::Activity { after } => {
                    assert_eq!(envelope.origin, Origin::App, "the shell's own read");
                    Some((envelope.seq, after))
                }
                _ => None,
            })
            .collect()
    }

    /// How many sweeps are among `sent`.
    fn sweeps(sent: &[RequestEnvelope]) -> usize {
        sent.iter()
            .filter(|envelope| matches!(envelope.request, StoreRequest::SweepRetention))
            .count()
    }

    /// One shell refresh.
    fn refresh(app: &mut App) {
        for _ in 0..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
    }

    /// Event `id`: a gate on htui `ANA-2` in Platform.
    fn event(id: i64) -> ActivityEvent {
        ActivityEvent {
            id,
            at: demo_at(0, 0),
            kind: ActivityKind::Gate,
            workspace: Some(ids::WORKSPACE_PLATFORM),
            project: Some(ids::PROJECT_HTUI),
            project_slug: "htui".to_owned(),
            item: Some(ids::HTUI_ANA_2),
            item_key: Some("ANA-2".to_owned()),
            run: Some(ids::RUN_1),
            chat: false,
            step: Some(ids::STEP_PLAN),
            step_name: Some("plan".to_owned()),
            permission: None,
            text: String::new(),
        }
    }

    /// The shell's reply to read `seq`.
    fn reply(seq: u64, reply: StoreReply) -> Action {
        Action::Reply(ReplyEnvelope {
            seq,
            origin: Origin::App,
            reply,
        })
    }

    /// A page holding `events`, none pinned.
    fn page(events: Vec<ActivityEvent>) -> StoreReply {
        StoreReply::Activity {
            page: ActivityPage {
                events,
                pinned: Vec::new(),
            },
            workspaces: Vec::new(),
        }
    }

    /// D8: shown, the pane is read on the refresh tick, the newest page first, then from the
    /// highest id held minus the overlap; never a second read while one is out.
    #[test]
    fn the_shown_activity_pane_is_read_on_the_refresh_tick_one_read_at_a_time() {
        let (mut app, mut rx) = shell(Preset::IdeActivity);
        let _ = sent(&mut rx);
        refresh(&mut app);
        let first = reads(&sent(&mut rx));
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].1, None, "the first read takes the newest page");

        refresh(&mut app);
        assert!(
            reads(&sent(&mut rx)).is_empty(),
            "one read in flight at most"
        );

        app.update(reply(first[0].0, page(vec![event(300), event(299)])));
        assert_eq!(app.activity.rows(&app.scope).len(), 2);
        refresh(&mut app);
        let next = reads(&sent(&mut rx));
        assert_eq!(next.len(), 1, "{next:?}");
        assert_eq!(next[0].1, Some(100), "max id minus the overlap");
    }

    /// D8, F9: a refused read says nothing on the status line, keeps the rows, and lets the next
    /// read go.
    #[test]
    fn a_refused_read_says_nothing_and_keeps_the_rows() {
        let (mut app, mut rx) = shell(Preset::IdeActivity);
        refresh(&mut app);
        let (seq, _) = reads(&sent(&mut rx))[0];
        app.update(reply(seq, page(vec![event(7)])));

        refresh(&mut app);
        let (seq, _) = reads(&sent(&mut rx))[0];
        app.update(reply(
            seq,
            StoreReply::Failed {
                request: "activity",
                message: "the database is down".to_owned(),
            },
        ));
        assert_eq!(app.status, None, "absorbed");
        assert_eq!(app.activity.rows(&app.scope).len(), 1, "the rows stay");

        refresh(&mut app);
        assert_eq!(reads(&sent(&mut rx)).len(), 1, "the next read goes");
    }

    /// D8: a hidden pane is not read; a layout that shows it reads at once, before any tick.
    #[test]
    fn a_hidden_pane_is_not_read_and_showing_it_reads_at_once() {
        let (mut app, mut rx) = shell(Preset::Ide);
        refresh(&mut app);
        assert!(reads(&sent(&mut rx)).is_empty(), "hidden: no read");

        app.update(Action::Layout(crate::app::LayoutAction::NextPreset));
        let layout = app.layout_of(app.tabs.active().expect("a tab"));
        assert_eq!(layout.map(|l| l.preset), Some(Preset::IdeActivity));
        assert_eq!(reads(&sent(&mut rx)).len(), 1, "read at once");
    }

    /// D6, F10: the retention sweep goes on the first refresh tick, shown pane or not, then not
    /// again before `SWEEP_TICKS` more ticks; its answer and its refusal say nothing. Review M2:
    /// not while the store is still connecting (or down): the first sweep goes on the first
    /// refresh after it is up.
    #[test]
    fn the_sweep_goes_on_the_first_refresh_once_up_and_then_hourly() {
        let (mut app, mut rx) = shell(Preset::Ide);
        app.top_bar.store = "connecting".to_owned();
        refresh(&mut app);
        assert_eq!(sweeps(&sent(&mut rx)), 0, "not to a store still connecting");
        app.top_bar.store = "offline \u{b7} 3m".to_owned();
        refresh(&mut app);
        assert_eq!(sweeps(&sent(&mut rx)), 0, "nor to one that is down");

        app.top_bar.store = "online".to_owned();
        for _ in 1..TICKS_PER_REFRESH {
            app.update(Action::Tick);
        }
        assert_eq!(sweeps(&sent(&mut rx)), 0, "not before the next refresh");
        app.update(Action::Tick);
        let first = sent(&mut rx);
        assert_eq!(sweeps(&first), 1, "the first refresh once up sweeps");
        let seq = first
            .iter()
            .find(|envelope| matches!(envelope.request, StoreRequest::SweepRetention))
            .map(|envelope| envelope.seq)
            .expect("the sweep");
        app.update(reply(seq, StoreReply::RetentionSwept(3)));
        assert_eq!(app.status, None);

        for _ in 1..SWEEP_TICKS {
            app.update(Action::Tick);
        }
        assert_eq!(sweeps(&sent(&mut rx)), 0, "not again within the hour");
        app.update(Action::Tick);
        let again = sent(&mut rx);
        assert_eq!(sweeps(&again), 1, "an hour on, again");
        let seq = again
            .iter()
            .find(|envelope| matches!(envelope.request, StoreRequest::SweepRetention))
            .map(|envelope| envelope.seq)
            .expect("the sweep");
        app.update(reply(
            seq,
            StoreReply::Failed {
                request: "sweep_retention",
                message: "the database is down".to_owned(),
            },
        ));
        assert_eq!(app.status, None, "a refused sweep is absorbed");
    }

    /// Review M2: the in-memory store is up from the start, so it is swept on the first refresh.
    #[test]
    fn a_memory_store_is_swept_on_the_first_refresh() {
        let (mut app, mut rx) = shell(Preset::Ide);
        app.top_bar.store = "memory".to_owned();
        refresh(&mut app);
        assert_eq!(sweeps(&sent(&mut rx)), 1);
    }

    /// Review L6 (D9): `Enter` on another workspace's row while an in-pane editor is open is
    /// refused: the status line says why, and neither the scope nor the tab moves.
    #[test]
    fn enter_on_another_workspaces_row_under_an_open_editor_is_refused() {
        use htui_core::model::WorkspaceSummary;

        use crate::editor::pane::PaneChild;
        use crate::editor::{EditorCommand, ExternalEdit};
        use crate::keys::Act;

        /// A child that does nothing: the editor only has to be open.
        #[derive(Debug)]
        struct Idle;
        impl PaneChild for Idle {
            fn write(&mut self, _bytes: &[u8]) {}
            fn resize(&mut self, _size: crate::editor::pane::PaneSize) {}
            fn kill(&mut self) {}
        }

        let (mut app, _rx) = shell(Preset::IdeActivity);
        // The shell starts on no workspace, so Platform's row is another workspace's.
        let platform = WorkspaceSummary {
            workspace_id: ids::WORKSPACE_PLATFORM,
            slug: "platform".to_owned(),
            name: "Platform".to_owned(),
            projects: Vec::new(),
        };
        app.activity.merge(
            ActivityPage {
                events: vec![event(1)],
                pinned: Vec::new(),
            },
            vec![platform],
        );
        app.open_editor(
            SPLIT,
            &ExternalEdit {
                text: "body\n".to_owned(),
                stem: "implement".to_owned(),
            },
            EditorCommand::resolve(|key| (key == "VISUAL").then(|| "nvim".to_owned())),
            |_id, _command, _size| Ok(Box::new(Idle) as Box<dyn PaneChild>),
        );
        assert!(app.editor.is_some(), "the editor is open");
        let (scope, tab) = (app.scope.clone(), app.tabs.active_id());

        let jump = app
            .activity
            .on_act(Act::ActivityOpen, &app.scope)
            .expect("Enter on the row jumps");
        app.activity_jump(jump);
        let refusal = app.editor_steer_refusal().expect("an editor is open");
        assert_eq!(app.status.as_deref(), Some(refusal.as_str()));
        assert!(refusal.contains("finish or abort it first"), "{refusal}");
        assert_eq!(app.scope, scope, "the scope does not move");
        assert_eq!(app.tabs.active_id(), tab, "nor the tab");
    }

    /// Review L6 (D9): `Enter` on a row whose item went says `<KEY> was deleted` and moves
    /// nothing.
    #[test]
    fn enter_on_a_deleted_items_row_says_so_and_moves_nothing() {
        use crate::keys::Act;

        let (mut app, _rx) = shell(Preset::IdeActivity);
        let gone = ActivityEvent {
            item: None,
            ..event(1)
        };
        app.activity.merge(
            ActivityPage {
                events: vec![gone],
                pinned: Vec::new(),
            },
            Vec::new(),
        );
        let (scope, tab) = (app.scope.clone(), app.tabs.active_id());
        let jump = app
            .activity
            .on_act(Act::ActivityOpen, &app.scope)
            .expect("Enter on the row jumps");
        app.activity_jump(jump);
        assert_eq!(app.status.as_deref(), Some("ANA-2 was deleted"));
        assert_eq!(app.scope, scope);
        assert_eq!(app.tabs.active_id(), tab);
    }

    /// Review L3: a refused sweep stays off the status line, but leaves a `warn` line.
    #[test]
    fn a_refused_sweep_is_a_warning_in_the_log() {
        use std::sync::{Arc, Mutex, PoisonError};

        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (mut app, mut rx) = shell(Preset::Ide);
        app.top_bar.store = "online".to_owned();
        refresh(&mut app);
        let seq = sent(&mut rx)
            .iter()
            .find(|envelope| matches!(envelope.request, StoreRequest::SweepRetention))
            .map(|envelope| envelope.seq)
            .expect("the sweep");
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let buffer = buffer.clone();
                move || buffer.clone()
            })
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            app.update(reply(
                seq,
                StoreReply::Failed {
                    request: "sweep_retention",
                    message: "the database is down".to_owned(),
                },
            ));
        });
        assert_eq!(app.status, None, "absorbed");
        let logged = String::from_utf8(
            buffer
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8 logs");
        assert!(logged.contains(" WARN "), "{logged}");
        assert!(logged.contains("the database is down"), "{logged}");
    }
}
