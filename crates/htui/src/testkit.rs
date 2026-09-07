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

use htui_core::model::StepId;
use htui_core::store::MemStore;
use htui_store::Backend;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::agent_worker::{AgentRuntime, ChatTask, Served};
use crate::app::{Action, App};
use crate::keymap::{KeyChord, Keymap};
use crate::store_worker::{self, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};
use crate::ui::overlay::Overlay;
use crate::ui::tabs::Tab;

/// Default snapshot size (plan risk row: snapshots are flaky across terminal sizes).
const DEFAULT_SIZE: (u16, u16) = (100, 30);

/// How many "serve everything queued" rounds [`Harness::settle`] may take.
const SETTLE_ROUNDS: usize = 32;

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
        let backend = Backend::memory(store);
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
    /// `Pending` after a quiet round is a chat waiting on the user, which is exactly the state a
    /// snapshot wants to photograph.
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
                        StoreRequest::ChatStart { .. }
                        | StoreRequest::ChatSend { .. }
                        | StoreRequest::ChatAnswer { .. }
                        | StoreRequest::ChatCancel { .. },
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
                    (request, _) => store_worker::serve(&self.backend, request).await,
                };
                self.app.update(Action::Reply(ReplyEnvelope {
                    seq: envelope.seq,
                    origin: envelope.origin,
                    reply,
                }));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Ctx, Handled};
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
        harness.settle().await;
        insta::assert_snapshot!("shell_empty", harness.render());
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
