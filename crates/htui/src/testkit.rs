//! Deterministic test harness (blueprint C.8).
//!
//! T4 and T5 build their tabs and overlays against this and never touch a T3 file. Everything
//! here is synchronous except [`Harness::settle`], which serves the queued requests **inline**
//! through [`store_worker::serve`] instead of spawning the worker: a snapshot is byte-stable
//! because every request raised by activation, a scope change or a key has been answered before
//! the frame is drawn, with no sleeps anywhere.
//!
//! Panics are deliberate here: a harness that silently rendered the wrong thing would cost more
//! than a failing test.

use std::collections::VecDeque;
use std::time::Duration;

use htui_core::model::{ProjectRef, Scope, StepId};
use htui_core::store::MemStore;
use htui_store::Backend;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::agent_worker::{AgentRuntime, ChatTask, Served};
use crate::app::{Action, App, Ctx, Emit, Handled, TopBarState};
use crate::keymap::{KeyChord, Keymap};
use crate::run_worker::{RunRuntime, RunServed};
use crate::store_worker::{self, Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::overlay::Overlay;
use crate::ui::tabs::Tab;
use crate::ui::tabs::settings::{SettingsSection, SettingsTab};

/// Default snapshot size (plan risk row: snapshots are flaky across terminal sizes).
pub(crate) const DEFAULT_SIZE: (u16, u16) = (100, 30);

/// How many "serve everything queued" rounds [`Harness::settle`] may take.
const SETTLE_ROUNDS: usize = 32;

/// How long [`Harness::drive_to_end`] waits for one chat task after it has asked every session to
/// stop.
///
/// Deliberately far longer than any chat the harness can produce - the fake driver answers in
/// microseconds and the real bound is a `spawn_blocking` file write. It is not a schedule, it is
/// the difference between a suite that **hangs** on a task that will never finish and one that
/// fails with the step id of the task that did not.
const CHAT_END: Duration = Duration::from_secs(30);

/// A shell, a store and a `TestBackend`, wired without a spawned task.
pub struct Harness {
    /// The shell under test.
    app: App,
    /// The store the requests are served from.
    backend: Backend,
    /// What the shell sent to the (absent) worker.
    rx: UnboundedReceiver<RequestEnvelope>,
    /// The chat runtime, when a test installed one. Without it every chat request is refused,
    /// which is the `chat_offline` fixture.
    runtime: Option<AgentRuntime>,
    /// Chat futures [`Harness::drive`] polls inline: production spawns, the harness awaits, and
    /// that is what makes a streamed turn byte-stable in a snapshot with no sleeps.
    chats: Vec<(StepId, ChatTask)>,
    /// The run runtime, when a test installed one (blueprint D207). Without it the orchestrator
    /// requests are answered by `store_worker::serve` (D183).
    runs: Option<RunRuntime>,
    /// The run runtime's event receiver, taken when it was installed (D181).
    run_events: Option<UnboundedReceiver<RunServed>>,
    /// The reply channel a chat writes its frames into.
    replies: (
        mpsc::UnboundedSender<ReplyEnvelope>,
        UnboundedReceiver<ReplyEnvelope>,
    ),
    /// What [`Harness::settle`] answers `StoreRequest::StoreState` with, when a test asks for
    /// something a memory backend cannot represent.
    store_state: Option<(String, Option<usize>)>,
    /// Where frames are drawn.
    term: Terminal<TestBackend>,
    /// Overlays queued by [`Harness::with_overlay`], pushed once the startup scope is settled.
    pending: VecDeque<Box<dyn Overlay>>,
}

impl core::fmt::Debug for Harness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Harness")
            .field("app", &self.app)
            .field("backend", &self.backend)
            .field("pending", &self.pending.len())
            .finish()
    }
}

impl Harness {
    /// A shell over an empty store: no workspace, no items.
    #[must_use]
    pub fn empty() -> Self {
        Self::over(MemStore::new())
    }

    /// A shell over the demo fixture. The first [`Harness::settle`] enters the first workspace,
    /// exactly as `--demo` does at startup.
    #[must_use]
    pub fn demo() -> Self {
        Self::over(MemStore::demo())
    }

    /// Builds a harness over a store.
    ///
    /// Public so a test can hand in a store it has *already* written to — the Settings tab's
    /// agent section is tested against a registry row that no fixture contains, which
    /// [`Harness::demo`] cannot express.
    #[must_use]
    pub fn over(store: MemStore) -> Self {
        Self::over_backend(Backend::memory(store))
    }

    /// Builds a harness over any [`Backend`], not only a memory one.
    ///
    /// The offline chat case needs a `Backend::Offline` over a seeded mirror: what it is testing is
    /// that the *backend* the store worker holds decides where a chat's rows go, so handing the
    /// harness a `MemStore` and pretending would test nothing (MOD-2 milestone 4, T21).
    #[must_use]
    pub fn over_backend(backend: Backend) -> Self {
        let (request_tx, rx) = mpsc::unbounded_channel();
        let mut app = App::new(request_tx, Keymap::default_global());
        app.top_bar.store = backend.label();
        app.start();
        let (width, height) = DEFAULT_SIZE;
        let (reply_tx, reply_rx) = mpsc::unbounded_channel();
        Self {
            app,
            backend,
            rx,
            runtime: None,
            chats: Vec::new(),
            runs: None,
            run_events: None,
            replies: (reply_tx, reply_rx),
            store_state: None,
            term: terminal(width, height),
            pending: VecDeque::new(),
        }
    }

    /// Installs the chat runtime the chat requests are served by.
    ///
    /// Without one, [`Harness::drive`] answers every chat request `Failed`, exactly as a build
    /// with no transport would.
    #[must_use]
    pub fn with_agent_runtime(mut self, runtime: AgentRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    /// Installs the run runtime the orchestrator requests are served by (blueprint D207), and
    /// takes its event receiver.
    ///
    /// [`Harness::drive`] then settles the runtime's tasks every round, so a render never
    /// photographs a half-walked run (H-5). [`Harness::settle`] stays runtime-free.
    #[must_use]
    pub fn with_run_runtime(mut self, mut runtime: RunRuntime) -> Self {
        self.run_events = Some(runtime.take_events());
        self.runs = Some(runtime);
        self
    }

    /// What the harness does with a [`RunServed`] that is not a plain reply, as the store loop's
    /// helper does (blueprint D181). **T6 stub**: a promotion's `Attach` is answered
    /// `promote_step: promotion needs the chat runtime` at its address; T7 binds the chat.
    fn on_run_served(&mut self, served: RunServed) {
        match served {
            RunServed::Attach { addr, .. } => {
                let _ = self.replies.0.send(ReplyEnvelope {
                    seq: addr.seq,
                    origin: addr.origin,
                    reply: StoreReply::Failed {
                        request: "promote_step",
                        message: store_worker::PROMOTION_NEEDS_CHAT.to_owned(),
                    },
                });
            }
            other @ (RunServed::Reply(_) | RunServed::Deferred) => {
                debug_assert!(false, "the run runtime's event channel carries only Attach");
                tracing::error!(?other, "a run event that is not an attach was dropped");
            }
        }
    }

    /// The steps of the chats this harness has started, oldest first.
    #[must_use]
    pub fn chat_steps(&self) -> Vec<StepId> {
        self.runtime
            .as_ref()
            .map(AgentRuntime::steps)
            .unwrap_or_default()
    }

    /// [`Harness::settle`] plus the chat pump.
    ///
    /// Three things per round: serve every queued request (chat ones through the runtime), poll
    /// every held chat future **once**, and deliver every frame the chats produced. A chat that is
    /// `Pending` after a quiet round is taken to be a chat waiting on the user, which is exactly
    /// the state a snapshot wants to photograph.
    ///
    /// The runtime-served list below is written out by hand rather than derived, so every request
    /// the runtime owns has to be added to it deliberately. The three MOD-20 install requests and
    /// MOD-21's four login ones are on it for that reason: without them, `Settings > i` or
    /// `Settings > a` in a harness test would be answered "no agent runtime in this harness" by a
    /// harness that has one.
    ///
    /// A login is the one of them that can be `Pending` on a **human** rather than on work: the
    /// flow's task makes progress only between drives, and a case that is waiting for a frame
    /// alternates `drive()` with a short sleep rather than calling [`Harness::drive_to_end`],
    /// which would await the flow for `CHAT_END` first (blueprint P-2).
    ///
    /// That reading holds for a chat whose writer never leaves the task — `MemStore`'s
    /// `append_events` is a lock and a `Vec` push, and `PgStore`'s is served inline here too. It
    /// does **not** hold for `Writer::Buffered`, which appends to a file through `spawn_blocking`:
    /// such a chat is `Pending` for as long as a blocking thread takes, and polling it once more
    /// would be a race rather than a test. [`Harness::drive_to_end`] is that case's answer.
    ///
    /// # Panics
    ///
    /// If the shell never goes quiet, like [`Harness::settle`].
    pub async fn drive(&mut self) {
        for _ in 0..SETTLE_ROUNDS {
            let mut progress = false;

            while let Ok(envelope) = self.rx.try_recv() {
                progress = true;
                let reply = match (&envelope.request, &self.store_state) {
                    (StoreRequest::StoreState, Some((label, migrations_pending))) => {
                        StoreReply::StoreState {
                            label: label.clone(),
                            migrations_pending: *migrations_pending,
                        }
                    }
                    (
                        StoreRequest::PromptPreview { .. }
                        | StoreRequest::ChatStart { .. }
                        | StoreRequest::ChatSend { .. }
                        | StoreRequest::ChatAnswer { .. }
                        | StoreRequest::ChatCancel { .. }
                        | StoreRequest::ProbeAgents
                        | StoreRequest::InstallPlan { .. }
                        | StoreRequest::InstallConfirm { .. }
                        | StoreRequest::InstallCancel
                        | StoreRequest::AuthStart { .. }
                        | StoreRequest::AuthChoose { .. }
                        | StoreRequest::AuthOpen { .. }
                        | StoreRequest::AuthCancel,
                        _,
                    ) => match self.runtime.as_mut() {
                        Some(runtime) => {
                            match runtime
                                .serve(&self.backend, &self.replies.0, &envelope)
                                .await
                            {
                                Served::Reply(reply) => reply,
                                // The chat answers this request itself, through the reply channel.
                                Served::Deferred => continue,
                                Served::Start { step_id, task } => {
                                    self.chats.push((step_id, task));
                                    continue;
                                }
                            }
                        }
                        None => StoreReply::Failed {
                            request: envelope.request.name(),
                            message: "no agent runtime in this harness".to_owned(),
                        },
                    },
                    // Blueprint D207: the run runtime's three, through it when a test installed
                    // one; without one `store_worker::serve` answers them (D183).
                    (
                        StoreRequest::Orch(_)
                        | StoreRequest::RunStream { .. }
                        | StoreRequest::RunActions(_),
                        _,
                    ) => {
                        let live = self
                            .runtime
                            .as_ref()
                            .map(store_worker::live_chats)
                            .unwrap_or_default();
                        let served = match self.runs.as_mut() {
                            Some(runs) => {
                                runs.serve(&self.backend, &self.replies.0, &envelope, &live)
                                    .await
                            }
                            None => RunServed::Reply(
                                store_worker::serve(&self.backend, &envelope.request).await,
                            ),
                        };
                        match served {
                            RunServed::Reply(reply) => reply,
                            // The runtime's task answers this request itself.
                            RunServed::Deferred => continue,
                            attach @ RunServed::Attach { .. } => {
                                self.on_run_served(attach);
                                continue;
                            }
                        }
                    }
                    (request, _) => store_worker::serve(&self.backend, request).await,
                };
                self.app.update(Action::Reply(ReplyEnvelope {
                    seq: envelope.seq,
                    origin: envelope.origin,
                    reply,
                }));
            }

            // H-5: every walk task this round started is awaited to its rest, so the frame this
            // drive is taken for never shows a half-walked run; then the runtime's events.
            if let Some(runs) = self.runs.as_mut() {
                let stuck = runs.settle(CHAT_END).await;
                assert!(
                    stuck.is_empty(),
                    "the walks of runs {stuck:?} did not rest within {CHAT_END:?}: their task is \
                     stuck, not slow"
                );
            }
            let mut events = Vec::new();
            if let Some(receiver) = self.run_events.as_mut() {
                while let Ok(served) = receiver.try_recv() {
                    events.push(served);
                }
            }
            for served in events {
                progress = true;
                self.on_run_served(served);
            }

            // One poll each: a chat that is ready finishes, one that is waiting stays where it is.
            let mut still_running = Vec::new();
            for (step, mut task) in std::mem::take(&mut self.chats) {
                match futures::poll!(&mut task) {
                    std::task::Poll::Ready(()) => progress = true,
                    std::task::Poll::Pending => still_running.push((step, task)),
                }
            }
            self.chats = still_running;

            while let Ok(envelope) = self.replies.1.try_recv() {
                progress = true;
                self.app.update(Action::Reply(envelope));
            }

            if progress {
                continue;
            }
            match self.pending.pop_front() {
                Some(overlay) => self.app.push_overlay(overlay),
                None => return,
            }
        }
        panic!("the shell never settled: a view is issuing a request for every reply");
    }

    /// [`Harness::drive`], then **awaits** every chat still held to its end, then drives again.
    ///
    /// For a writer that reaches the filesystem there is no honest "poll once more and hope": a
    /// chat mid-`spawn_blocking` is indistinguishable from one waiting on the user, and a bounded
    /// spin over the difference is exactly the flakiness snapshots are kept free of. Awaiting is
    /// the deterministic answer, and it needs a chat that *has* an end — so every live chat is
    /// asked to stop first, whether or not the test already pressed `Esc Esc`. A chat the user
    /// ended ignores the second ask (its command channel is already closed), and one the user did
    /// not is ended here rather than left to hang the suite.
    ///
    /// The last `drive` is what delivers the frames the finished chats produced, so a render taken
    /// after this call is the whole conversation. The runtime holds no live chat afterwards, which
    /// is the trade: this is the end of a chat, not a pause in one.
    ///
    /// # Panics
    ///
    /// If the shell never goes quiet, like [`Harness::settle`], or if a chat task has not ended
    /// `CHAT_END` after being asked to. A task that never returns would otherwise hang the whole
    /// suite with no output at all; the panic names the step, which is the one fact that turns
    /// "the tests stopped" into a diagnosis.
    pub async fn drive_to_end(&mut self) {
        self.drive().await;
        if let Some(runtime) = self.runtime.as_mut() {
            // Before the shutdown, not after: a probe and an install each answer from a task the
            // runtime owns, and `shutdown` aborts those. Awaiting them here is what makes their
            // replies part of the frame this call is taken for (blueprint P-2 — an install that
            // was still downloading when the shell went quiet is finished here, not photographed
            // half-done).
            runtime.finish_background(CHAT_END).await;
            runtime.shutdown(Duration::ZERO).await;
        }
        for (step, task) in std::mem::take(&mut self.chats) {
            assert!(
                tokio::time::timeout(CHAT_END, task).await.is_ok(),
                "the chat on step {step} did not end within {CHAT_END:?} of being asked to: its \
                 session task is stuck, not slow",
            );
        }
        self.drive().await;
    }

    /// Answers every `StoreRequest::StoreState` with this label and pending count.
    ///
    /// The one thing a `Backend::Memory` cannot represent: `connecting`, `offline · 3m` and a
    /// pending-migration count are states of a *connection*, and the harness deliberately has
    /// none. Without an override the harness behaves exactly as it did in MOD-1 — the reply is
    /// `store_worker::serve`'s, i.e. `"memory"` and no pending count — so no existing snapshot
    /// moves.
    #[must_use]
    pub fn with_store_state(mut self, label: &str, migrations_pending: Option<usize>) -> Self {
        self.store_state = Some((label.to_owned(), migrations_pending));
        self
    }

    /// Registers a tab, as [`register_all`](crate::app::register_all) would.
    #[must_use]
    pub fn with_tab(mut self, tab: Box<dyn Tab>) -> Self {
        self.app.register_tab(tab);
        self
    }

    /// Names the tab an `Action::Replay` focuses and addresses its rows to, as
    /// [`register_all`](crate::app::register_all) names it.
    ///
    /// A test that registers its tabs by hand has to set this by hand too: without it the shell
    /// can replay nothing, which is a state worth testing on its own.
    #[must_use]
    pub fn with_replay_tab(mut self, tab: crate::ui::tabs::TabId) -> Self {
        self.app.replay_tab = Some(tab);
        self
    }

    /// Opens an overlay.
    ///
    /// The push is deferred to the first [`Harness::settle`]: entering the startup workspace
    /// closes every open overlay (blueprint D, "Scope change"), so an overlay pushed before that
    /// would be swallowed by the shell's own startup.
    #[must_use]
    pub fn with_overlay(mut self, overlay: Box<dyn Overlay>) -> Self {
        self.pending.push_back(overlay);
        self
    }

    /// Changes the frame size. Snapshots are size-sensitive; the default is 100x30.
    #[must_use]
    pub fn size(mut self, width: u16, height: u16) -> Self {
        self.term = terminal(width, height);
        self
    }

    /// Serves every queued request inline until nothing is left to answer.
    ///
    /// # Panics
    ///
    /// If the shell never goes quiet, which means a view requests on every reply.
    pub async fn settle(&mut self) {
        for _ in 0..SETTLE_ROUNDS {
            let mut served = false;
            while let Ok(envelope) = self.rx.try_recv() {
                let reply = match (&envelope.request, &self.store_state) {
                    (StoreRequest::StoreState, Some((label, migrations_pending))) => {
                        StoreReply::StoreState {
                            label: label.clone(),
                            migrations_pending: *migrations_pending,
                        }
                    }
                    (request, _) => store_worker::serve(&self.backend, request).await,
                };
                self.app.update(Action::Reply(ReplyEnvelope {
                    seq: envelope.seq,
                    origin: envelope.origin,
                    reply,
                }));
                served = true;
            }
            if served {
                continue;
            }
            match self.pending.pop_front() {
                Some(overlay) => self.app.push_overlay(overlay),
                None => return,
            }
        }
        panic!("the shell never settled: a view is issuing a request for every reply");
    }

    /// Feeds one key, written the way [`KeyChord::parse`] reads it: `"j"`, `"Enter"`,
    /// `"shift-tab"`.
    ///
    /// # Panics
    ///
    /// If the spec is not a key.
    pub fn key(&mut self, chord: &str) {
        let parsed =
            KeyChord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is not a key chord"));
        self.app.on_key(parsed.to_event());
    }

    /// Draws a frame and returns the buffer as text, one line per row, trailing blanks trimmed.
    ///
    /// # Panics
    ///
    /// If the `TestBackend` fails to draw, which it only does when a widget panics.
    pub fn render(&mut self) -> String {
        let Self { term, app, .. } = self;
        if let Err(error) = term.draw(|frame| app.render(frame)) {
            panic!("the test backend failed to draw: {error}");
        }
        buffer_text(term.backend().buffer())
    }

    /// The shell, for assertions and for registrations a test wants to make by hand.
    pub fn app(&mut self) -> &mut App {
        &mut self.app
    }
}

/// A `TestBackend` terminal of this size.
fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height))
        .unwrap_or_else(|error| panic!("the test backend could not be created: {error}"))
}

/// The buffer as snapshot text.
fn buffer_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell((x, y)) {
                line.push_str(cell.symbol());
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Everything [`Ctx::new`] borrows, owned in one place, for testing a section without a shell
/// around it.
///
/// A section is tested on its own — that is what makes a key and a reply independently
/// assertable — and every one of those tests needs the same six values. Holding them together
/// also holds the [`Emit`] queue, which is how a test sees what the section asked the store for
/// without a store existing at all (`R-NF-3`).
#[derive(Debug)]
pub struct SectionBench {
    /// The workspace every request is issued against.
    scope: Scope,
    /// The scope's projects. Empty: a section that needs them builds its own.
    projects: Vec<ProjectRef>,
    /// What the top bar would be showing.
    top_bar: TopBarState,
    /// The key table, for the hint lines a section renders.
    keymap: Keymap,
    /// The palette.
    theme: Theme,
    /// What the section emitted, drained by [`drained`](SectionBench::drained).
    emit: Emit,
}

impl SectionBench {
    /// A bench in the demo fixture's first workspace (`Graphics`; workspaces come back by name).
    ///
    /// # Panics
    ///
    /// If the demo fixture has no workspace.
    pub async fn new() -> Self {
        let workspaces = MemStore::demo()
            .workspaces()
            .await
            .expect("the memory store never fails");
        Self {
            scope: Scope::from_workspace(workspaces.first().expect("the fixture has a workspace")),
            projects: Vec::new(),
            top_bar: TopBarState::default(),
            keymap: Keymap::default_global(),
            theme: Theme::default(),
            emit: Emit::default(),
        }
    }

    /// A context addressed to the Settings tab, as the shell builds one.
    #[must_use]
    pub fn ctx(&self) -> Ctx<'_> {
        Ctx::new(
            &self.scope,
            &self.projects,
            &self.top_bar,
            &self.keymap,
            &self.theme,
            Origin::Tab(SettingsTab::ID),
            &self.emit,
        )
    }

    /// Feeds one key to a section, written the way [`KeyChord::parse`] reads it.
    ///
    /// # Panics
    ///
    /// If `chord` is not a chord.
    pub fn key(&self, section: &mut dyn SettingsSection, chord: &str) -> Handled {
        let parsed = KeyChord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is not a chord"));
        let mut ctx = self.ctx();
        section.on_key(parsed.to_event(), &mut ctx)
    }

    /// Hands one reply to a section.
    pub fn reply(&self, section: &mut dyn SettingsSection, reply: &StoreReply) {
        let mut ctx = self.ctx();
        section.on_reply(reply, &mut ctx);
    }

    /// Everything the section emitted since this was last called, leaving the queue empty.
    #[must_use]
    pub fn drained(&self) -> Vec<Action> {
        self.emit.take()
    }

    /// The error texts the section put on the status line since this was last called.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        self.drained()
            .into_iter()
            .filter_map(|action| match action {
                Action::Error(message) => Some(message),
                _ => None,
            })
            .collect()
    }

    /// Draws one section into a `width`x30 buffer and returns it as snapshot text.
    ///
    /// The section's own test file keeps its `Buffer`-returning twin: a cursor is a *style*, and
    /// text alone cannot tell a moved cursor from a stuck one.
    ///
    /// # Panics
    ///
    /// If the `TestBackend` fails to draw, which it only does when the section panics.
    pub fn render_section(&self, section: &dyn SettingsSection, width: u16) -> String {
        let area = Rect::new(0, 0, width, 30);
        let mut terminal = Terminal::with_options(
            TestBackend::new(width, 30),
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .expect("a test terminal");
        let ctx = self.ctx();
        terminal
            .draw(|frame| section.render(frame, frame.area(), &ctx))
            .expect("the section draws");
        buffer_text(terminal.backend().buffer())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::centered;
    use crate::ui::overlay::OverlayId;
    use crossterm::event::{KeyCode, KeyEvent};
    use htui_core::model::Scope;
    use ratatui::Frame;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;

    #[tokio::test]
    async fn an_empty_store_renders_the_shell_with_the_switcher_over_it() {
        let mut harness = Harness::empty();
        crate::app::register_all(harness.app());
        harness.drive_to_end().await;
        insta::assert_snapshot!("shell_empty", harness.render());
    }

    /// Blueprint D207: a harness with a run runtime serves `StartRun` through it, and `drive`
    /// returns only once the walk has rested (H-5).
    #[tokio::test]
    async fn drive_settles_a_walk_of_the_run_runtime() {
        use crate::run_worker::tests::{Fixture, only_run, start_run};

        let fixture = Fixture::new().await;
        let mut harness = Harness::over(fixture.store.clone()).with_run_runtime(fixture.runtime());
        harness.drive().await;
        harness.app().update(Action::Store(start_run(
            htui_core::fixtures::ids::HTUI_ANA_2,
        )));
        harness.drive().await;

        let run = only_run(&fixture.store, htui_core::fixtures::ids::HTUI_ANA_2).await;
        assert_eq!(
            fixture.run(run).await.status,
            htui_core::model::RunStatus::AwaitingApproval,
            "the walk rested inside the drive"
        );
        assert_eq!(harness.app().status, None, "and nothing failed");
    }

    /// Blueprint D183, D207: without a run runtime the stream and the verdicts still answer, so
    /// nothing reaches the status line; only a command is refused, by name.
    #[tokio::test]
    async fn a_harness_without_a_run_runtime_answers_the_reads_and_refuses_a_command() {
        use crate::run_worker::tests::start_run;

        let mut harness = Harness::demo();
        harness.drive().await;
        let item = htui_core::fixtures::ids::HTUI_ANA_2;
        harness
            .app()
            .update(Action::Store(StoreRequest::RunStream { item }));
        harness
            .app()
            .update(Action::Store(StoreRequest::RunActions(item)));
        harness.drive().await;
        assert_eq!(harness.app().status, None);

        harness.app().update(Action::Store(start_run(item)));
        harness.drive().await;
        assert_eq!(
            harness.app().status.as_deref(),
            Some("start_run: no run runtime in this build")
        );
    }

    /// Blueprint D181's T6 stub, through the harness: the promotion's writes land and the attach
    /// hand-off answers the promoting tab with its sentence.
    #[tokio::test]
    async fn a_promotion_in_the_harness_reaches_the_attach_stub() {
        use crate::run_worker::tests::{Fixture, only_run, start_run, step_at};

        let fixture = Fixture::new().await;
        let mut harness = Harness::over(fixture.store.clone())
            .with_run_runtime(fixture.runtime())
            .with_replay_tab(crate::ui::tabs::TabId("chat"));
        harness.drive().await;
        let item = htui_core::fixtures::ids::HTUI_ANA_2;
        harness.app().update(Action::Store(start_run(item)));
        harness.drive().await;
        let run = only_run(&fixture.store, item).await;
        let step = step_at(&fixture, run, 0).await;

        harness.app().update(Action::Promote { run, step: step.id });
        harness.drive().await;
        assert_eq!(
            harness.app().status.as_deref(),
            Some("promote_step: promotion needs the chat runtime")
        );
        assert!(
            step_at(&fixture, run, 0).await.promoted_at.is_some(),
            "the engine's writes are done"
        );
    }

    #[tokio::test]
    async fn the_tab_strip_follows_the_tab_bindings() {
        let mut harness = Harness::empty();
        crate::app::register_all(harness.app());
        harness.settle().await;
        // An empty store leaves the registration's switcher up and a modal overlay swallows what
        // it does not handle, so the tab bindings only apply once it is closed.
        harness.key("esc");
        harness.key("tab");
        harness.settle().await;
        assert!(harness.render().contains("Skills"));
        assert_eq!(
            harness.app().tabs.active_id().map(|id| id.0),
            Some("skills")
        );
        harness.key("1");
        assert_eq!(
            harness.app().tabs.active_id().map(|id| id.0),
            Some("backlog")
        );
    }

    #[tokio::test]
    async fn the_demo_store_settles_into_the_first_workspace() {
        let mut harness = Harness::demo();
        crate::app::register_all(harness.app());
        harness.settle().await;
        let top_bar = harness.app().top_bar.clone();
        assert_eq!(
            top_bar.workspace, "Graphics",
            "workspaces are ordered by name"
        );
        assert_eq!(top_bar.box_name, "DESKTOP-HTUI");
        assert_eq!(top_bar.store, "memory");
        assert_eq!(harness.app().projects.len(), 1);
        assert!(
            harness
                .render()
                .starts_with("Graphics · DESKTOP-HTUI · memory · 0 runs")
        );
    }

    /// A modal overlay that counts the keys it was offered and consumes `j` only.
    #[derive(Debug, Default)]
    struct Probe {
        keys: usize,
        replies: usize,
    }

    impl Probe {
        const ID: OverlayId = OverlayId("probe");
    }

    impl Overlay for Probe {
        fn id(&self) -> OverlayId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Probe"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            vec![StoreRequest::Workspaces]
        }
        fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            self.keys += 1;
            if key.code == KeyCode::Char('j') {
                Handled::Consumed
            } else {
                Handled::Pass
            }
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {
            self.replies += 1;
        }
        fn render(&self, frame: &mut Frame<'_>, area: Rect, _ctx: &Ctx<'_>) {
            frame.render_widget(Paragraph::new("probe"), centered(area, 10, 1));
        }
    }

    #[tokio::test]
    async fn an_overlay_survives_startup_sees_keys_first_and_closes_on_esc() {
        let mut harness = Harness::demo().with_overlay(Box::new(Probe::default()));
        crate::app::register_all(harness.app());
        harness.settle().await;

        assert!(
            harness.render().contains("probe"),
            "the deferred push happens inside settle, after the startup scope change"
        );
        assert!(!harness.app().overlays.is_empty());

        // A modal overlay swallows what it does not consume: `q` never reaches the global table.
        harness.key("q");
        assert!(!harness.app().should_quit);

        // `Esc` is not consumed by the overlay, so the wildcard overlay binding closes it.
        harness.key("esc");
        assert!(harness.app().overlays.is_empty());

        // With no overlay left, the global table sees the key again.
        harness.key("q");
        assert!(harness.app().should_quit);
    }
}
