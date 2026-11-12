//! MOD-141 M3 D4: [`GraphEditorHost`], the editor's entry point. It opens a graph of any scope by
//! id (the Toolbox hook) or on a body its host already holds (`Settings > Kinds`), routes every
//! reply the editor or its own reads take, and stands in for the editor while the body is read or
//! after the read was refused.

use crossterm::event::{KeyEvent, MouseEvent};
use htui_core::model::{GraphBody, Persona, Scope, StepGraphId};
use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::widgets::Paragraph;

use super::{EditorKey, GraphEditor};
use crate::app::{Ctx, Handled};
use crate::catalogue::PersonaSummary;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::modal::modal_rest;
use crate::ui::kit::text::{Tone, notice};

/// What a host shows while the body read is out.
pub(crate) const READING_GRAPH: &str = "reading the graph\u{2026}";
/// What a host shows when the body read found no graph.
pub(crate) const GRAPH_NOT_FOUND: &str = "graph not found";

/// What the host holds.
enum State {
    /// The body (and the personas) were asked for; nothing is open yet.
    Loading {
        /// The graph asked for.
        graph: StepGraphId,
        /// The persona registry once a `Personas` or `Catalogue` reply arrived.
        personas: Option<Vec<PersonaSummary>>,
    },
    /// The editor is open.
    Open(Box<GraphEditor>),
    /// The body read found no graph or was refused: nothing opens, close still answers.
    Refused {
        /// The graph asked for.
        graph: StepGraphId,
        /// What the read said, in words.
        message: String,
    },
}

/// MOD-141 M3 D4: one graph editor of any scope and the reads that open it. Holds no store
/// handle (`R-NF-3`): every read leaves through the returned requests or `Ctx::request`, every
/// reply comes back through [`on_reply`](Self::on_reply).
///
/// While the body is read, or after that read found nothing or was refused, the host stands in for
/// the editor: one line drawn, every key swallowed but `graph_editor.close`.
pub struct GraphEditorHost {
    /// Loading, open or refused.
    state: State,
}

/// The close key, alone: what a host offers while no editor is open.
const HINT_CLOSE: HintSpec = &[Hint::One(Act::GraphEditorClose, "close")];

/// The requests a [`GraphEditor`] answers by name in its `on_reply`.
const EDITOR_REQUESTS: [&str; 4] = [
    "replace_graph",
    "agents",
    "templates",
    "graph_editor_settings",
];

impl GraphEditorHost {
    /// Opens graph `graph` of any scope by id (the Toolbox hook) and answers the reads to send now:
    /// `[StoreRequest::GraphBody(graph), StoreRequest::Personas]`. The editor opens when the body
    /// arrives; a second open replaces this host, since a `GraphBody(None)` names no graph.
    #[must_use]
    pub fn open_by_id(graph: StepGraphId) -> (Self, Vec<StoreRequest>) {
        let host = Self {
            state: State::Loading {
                graph,
                personas: None,
            },
        };
        (
            host,
            vec![StoreRequest::GraphBody(graph), StoreRequest::Personas],
        )
    }

    /// Opens on a body the caller already holds (`Settings > Kinds`): [`GraphEditor::open`]
    /// wrapped, so the reads are exactly the editor's three and no persona read (the caller's
    /// catalogue carries the personas).
    #[must_use]
    pub fn with_body(
        body: GraphBody,
        personas: Vec<PersonaSummary>,
        scope: &Scope,
    ) -> (Self, Vec<StoreRequest>) {
        let (editor, requests) = GraphEditor::open(body, personas, scope);
        let host = Self {
            state: State::Open(Box::new(editor)),
        };
        (host, requests)
    }

    /// Every reply the host's view receives; `true` when the host took it.
    ///
    /// In any state the persona registry (`Personas`, a `Catalogue`'s) is taken: held while
    /// loading, else handed to the editor. Loading, the body of the graph asked for opens the
    /// editor and sends its reads with `ctx.scope`; `GraphBody(None)` or a refused `graph_body`
    /// read refuses with a sentence; a refused `personas` read is taken and the editor opens
    /// without them. Open, the editor's own replies pass through (`GraphReplaced` hands its
    /// catalogue's personas on first).
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) -> bool {
        match reply {
            StoreReply::Personas(rows) => {
                self.set_personas(&summaries(rows));
                true
            }
            StoreReply::Catalogue(snapshot) => {
                self.set_personas(&snapshot.personas);
                true
            }
            _ => match &mut self.state {
                State::Loading { .. } => self.on_loading_reply(reply, ctx),
                State::Open(editor) => on_open_reply(editor, reply, ctx),
                State::Refused { .. } => false,
            },
        }
    }

    /// A reply while the body is read.
    fn on_loading_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) -> bool {
        let State::Loading { graph, personas } = &mut self.state else {
            return false;
        };
        let graph = *graph;
        match reply {
            StoreReply::GraphBody(Some(body)) if body.graph.id == graph => {
                let personas = personas.take().unwrap_or_default();
                let (editor, requests) = GraphEditor::open((**body).clone(), personas, ctx.scope);
                self.state = State::Open(Box::new(editor));
                for request in requests {
                    ctx.request(request);
                }
                true
            }
            StoreReply::GraphBody(None) => {
                self.state = State::Refused {
                    graph,
                    message: GRAPH_NOT_FOUND.to_owned(),
                };
                true
            }
            StoreReply::Failed { request, message } if *request == "graph_body" => {
                self.state = State::Refused {
                    graph,
                    message: message.clone(),
                };
                true
            }
            StoreReply::Failed { request, .. } if *request == "personas" => true,
            _ => false,
        }
    }

    /// The graph asked for, or on screen.
    #[must_use]
    pub fn graph(&self) -> StepGraphId {
        match &self.state {
            State::Loading { graph, .. } | State::Refused { graph, .. } => *graph,
            State::Open(editor) => editor.graph(),
        }
    }

    /// The open editor, for what the caller does to it on opening (`select`, `open_node_form`);
    /// `None` while loading or refused.
    pub fn editor_mut(&mut self) -> Option<&mut GraphEditor> {
        match &mut self.state {
            State::Open(editor) => Some(editor),
            State::Loading { .. } | State::Refused { .. } => None,
        }
    }

    /// The persona registry changed: held while loading, handed to the open editor.
    pub fn set_personas(&mut self, personas: &[PersonaSummary]) {
        match &mut self.state {
            State::Loading { personas: held, .. } => *held = Some(personas.to_vec()),
            State::Open(editor) => editor.set_personas(personas),
            State::Refused { .. } => {}
        }
    }

    /// Whether the open editor has a write in flight.
    #[must_use]
    pub fn busy(&self) -> bool {
        match &self.state {
            State::Open(editor) => editor.busy(),
            State::Loading { .. } | State::Refused { .. } => false,
        }
    }

    /// The open editor's stack, else the canvas's `GRAPH_EDITOR`, where close is bound.
    #[must_use]
    pub fn stack(&self) -> Stack<'static> {
        match &self.state {
            State::Open(editor) => editor.stack(),
            State::Loading { .. } | State::Refused { .. } => views::GRAPH_EDITOR,
        }
    }

    /// The open editor's hint row, else the close key alone.
    #[must_use]
    pub fn hint_text(&self, keys: &Keys) -> String {
        match &self.state {
            State::Open(editor) => editor.hint_text(keys),
            State::Loading { .. } | State::Refused { .. } => {
                keys.hint(views::GRAPH_EDITOR, HINT_CLOSE)
            }
        }
    }

    /// The reads to re-send on activation: loading, the body and the personas again; open, the
    /// editor's; refused, none.
    #[must_use]
    pub fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        match &self.state {
            State::Loading { graph, .. } => {
                vec![StoreRequest::GraphBody(*graph), StoreRequest::Personas]
            }
            State::Open(editor) => editor.wants_requests(scope),
            State::Refused { .. } => Vec::new(),
        }
    }

    /// Whether a paste lands: the open editor's focused text row.
    #[must_use]
    pub fn takes_paste(&self) -> bool {
        match &self.state {
            State::Open(editor) => editor.takes_paste(),
            State::Loading { .. } | State::Refused { .. } => false,
        }
    }

    /// A bracketed paste: the open editor's, else the caller's.
    pub fn on_paste(&mut self, text: &str) -> Handled {
        match &mut self.state {
            State::Open(editor) => editor.on_paste(text),
            State::Loading { .. } | State::Refused { .. } => Handled::Pass,
        }
    }

    /// One key: the open editor's; else `graph_editor.close` closes and any other chord is
    /// swallowed unless the modal global layer lets it through.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> EditorKey {
        if let State::Open(editor) = &mut self.state {
            return editor.on_key(key, ctx);
        }
        let stack = views::GRAPH_EDITOR;
        let chord = ctx.pressed(&key);
        if ctx
            .keys()
            .actions(stack, chord)
            .contains(&Act::GraphEditorClose)
        {
            return EditorKey::Close;
        }
        EditorKey::Handled(modal_rest(stack, chord))
    }

    /// One mouse event: the open editor's, else the caller's.
    pub fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.state {
            State::Open(editor) => editor.on_mouse(mouse, ctx),
            State::Loading { .. } | State::Refused { .. } => Handled::Pass,
        }
    }

    /// The mouse went away mid-gesture.
    pub fn on_mouse_lost(&mut self) {
        if let State::Open(editor) = &mut self.state {
            editor.on_mouse_lost();
        }
    }

    /// Draws over `area`: the open editor, else one line saying the body is being read, or why
    /// nothing opened.
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let line = match &self.state {
            State::Open(editor) => return editor.render(frame, area, ctx),
            State::Loading { .. } => notice(READING_GRAPH, Tone::Info, ctx.theme),
            State::Refused { message, .. } => notice(message, Tone::Error, ctx.theme),
        };
        frame.render_widget(Paragraph::new(line), area);
    }
}

/// A reply while the editor is open: the editor's own pass through.
fn on_open_reply(editor: &mut GraphEditor, reply: &StoreReply, ctx: &mut Ctx<'_>) -> bool {
    match reply {
        StoreReply::GraphReplaced { catalogue, .. } => {
            if let Some(catalogue) = catalogue {
                editor.set_personas(&catalogue.personas);
            }
        }
        StoreReply::Agents(_) | StoreReply::Templates(_) | StoreReply::GraphEditorSettings(_) => {}
        StoreReply::Failed { request, .. } if EDITOR_REQUESTS.contains(request) => {}
        _ => return false,
    }
    editor.on_reply(reply, ctx);
    true
}

/// The registry rows as the editor names them: id and name.
fn summaries(rows: &[Persona]) -> Vec<PersonaSummary> {
    rows.iter()
        .map(|row| PersonaSummary {
            id: row.id,
            name: row.name.clone(),
        })
        .collect()
}

/// The state's name and the graph; never a step name, a body or a refusal's text (H-7).
impl core::fmt::Debug for GraphEditorHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match &self.state {
            State::Loading { .. } => "loading",
            State::Open(_) => "open",
            State::Refused { .. } => "refused",
        };
        f.debug_struct("GraphEditorHost")
            .field("state", &state)
            .field("graph", &self.graph())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
    use htui_core::fixtures::ids;
    use htui_core::model::{
        EdgeId, EdgeKind, GraphScope, NewPersona, NewStepGraph, NodeBody, PersonaId,
        PersonaPermission, PersonaTools, PhaseId, ProjectRef, StepGraphEdge,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_core::store::{MemStore, WriteStore};
    use htui_store::Backend;
    use ratatui::backend::TestBackend;
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use uuid::Uuid;

    use super::super::Notice;
    use super::*;
    use crate::app::{Action, Emit, TopBarState};
    use crate::catalogue::CatalogueSnapshot;
    use crate::keys::KeyChord;
    use crate::store_worker::{self, Origin};
    use crate::ui::Theme;
    use crate::ui::tabs::settings::SettingsTab;

    /// Everything [`Ctx::new`] borrows, and a demo store to serve the requests on.
    struct HostBench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        theme: Theme,
        emit: Emit,
        keys: Keys,
        backend: Backend,
    }

    impl HostBench {
        /// The demo fixture's first workspace (`Graphics`) over a fresh demo store.
        async fn new() -> Self {
            let store = MemStore::demo();
            let workspaces = store
                .workspaces()
                .await
                .expect("the memory store never fails");
            Self {
                scope: Scope::from_workspace(workspaces.first().expect("a workspace")),
                projects: Vec::new(),
                top_bar: TopBarState::default(),
                theme: Theme::default(),
                emit: Emit::default(),
                keys: Keys::compiled().clone(),
                backend: Backend::memory(store),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keys,
                &self.theme,
                Origin::Tab(SettingsTab::ID),
                &self.emit,
            )
        }

        /// One chord, written as `KeyChord::parse` reads it.
        fn key(&self, host: &mut GraphEditorHost, chord: &str) -> EditorKey {
            let parsed = KeyChord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is no chord"));
            let mut ctx = self.ctx();
            host.on_key(parsed.to_event(), &mut ctx)
        }

        /// The chord `graph_editor.close` is bound to.
        fn close_chord(&self) -> String {
            self.keys
                .label(views::GRAPH_EDITOR, Act::GraphEditorClose)
                .expect("close is bound")
        }

        fn reply(&self, host: &mut GraphEditorHost, reply: &StoreReply) -> bool {
            let mut ctx = self.ctx();
            host.on_reply(reply, &mut ctx)
        }

        /// The store requests emitted since the last call.
        fn requests(&self) -> Vec<StoreRequest> {
            self.emit
                .take()
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(request) => Some(request),
                    _ => None,
                })
                .collect()
        }

        async fn served(&self, request: &StoreRequest) -> StoreReply {
            store_worker::serve(&self.backend, request).await
        }

        /// Serves each of `requests` and hands its reply to `host`; every one must be taken.
        async fn answer(&self, host: &mut GraphEditorHost, requests: &[StoreRequest]) {
            for request in requests {
                let reply = self.served(request).await;
                assert!(self.reply(host, &reply), "{request:?} answered {reply:?}");
            }
        }

        /// `open_by_id(graph)`, its two reads answered, then the editor's three reads answered.
        async fn opened(&self, graph: StepGraphId) -> GraphEditorHost {
            let (mut host, requests) = GraphEditorHost::open_by_id(graph);
            self.answer(&mut host, &requests).await;
            let reads = self.requests();
            assert_eq!(
                format!("{reads:?}"),
                format!("{:?}", editor_reads(&self.scope)),
                "the editor's three reads leave once the body arrived"
            );
            self.answer(&mut host, &reads).await;
            assert!(host.editor_mut().is_some(), "the editor is open");
            host
        }

        /// The store's persona registry, as the host names it.
        async fn registry(&self) -> Vec<PersonaSummary> {
            let StoreReply::Personas(rows) = self.served(&StoreRequest::Personas).await else {
                panic!("the registry reads");
            };
            summaries(&rows)
        }

        /// A new graph of `scope` holding `research -next-> plan`, written through the store.
        async fn seeded(&self, scope: GraphScope) -> StepGraphId {
            let graph = StepGraphId::new();
            let writer = self.backend.writer().expect("a memory backend writes");
            writer
                .create_step_graph(NewStepGraph {
                    id: graph,
                    scope,
                    name: "flow".to_owned(),
                    description: String::new(),
                    is_override: false,
                })
                .await
                .expect("the graph is created");
            let StoreReply::GraphBody(Some(mut body)) =
                self.served(&StoreRequest::GraphBody(graph)).await
            else {
                panic!("the new graph reads");
            };
            let (research, plan) = (PhaseId::new(), PhaseId::new());
            body.nodes = vec![
                step(graph, research, 0, "research"),
                step(graph, plan, 1, "plan"),
            ];
            body.edges = vec![StepGraphEdge {
                id: EdgeId::from_uuid(Uuid::new_v4()),
                graph_id: graph,
                from: research,
                to: plan,
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            }];
            let reply = self
                .served(&StoreRequest::ReplaceGraph {
                    scope: self.scope.clone(),
                    graph,
                    expected_revision: body.graph.revision,
                    body,
                })
                .await;
            assert!(
                matches!(
                    &reply,
                    StoreReply::GraphReplaced {
                        outcome: htui_core::store::CasOutcome::Applied(_),
                        ..
                    }
                ),
                "the seed applies: {reply:?}"
            );
            graph
        }

        /// The host drawn into `width` x `height`, as text.
        fn render(&self, host: &GraphEditorHost, width: u16, height: u16) -> String {
            let area = Rect::new(0, 0, width, height);
            let mut terminal = Terminal::with_options(
                TestBackend::new(width, height),
                TerminalOptions {
                    viewport: Viewport::Fixed(area),
                },
            )
            .expect("a test terminal");
            let ctx = self.ctx();
            terminal
                .draw(|frame| host.render(frame, frame.area(), &ctx))
                .expect("the host draws");
            let buffer = terminal.backend().buffer();
            let mut out = String::new();
            for y in 0..buffer.area.height {
                let mut line = String::new();
                for x in 0..buffer.area.width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
            out
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    /// A step `name` of `graph`, its template of its name.
    fn step(graph: StepGraphId, id: PhaseId, position: i32, name: &'static str) -> NodeBody {
        let seed = PhaseSeed {
            name,
            input_kinds: &[],
            gate_hard: false,
        };
        NodeBody {
            phase: phase_row(id, graph, position, &seed, now()),
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// `research -next-> plan` in the library, never written to a store.
    fn library_body() -> GraphBody {
        let graph = StepGraphId::from_uuid(Uuid::from_u128(0x0141_3700));
        let (research, plan) = (
            PhaseId::from_uuid(Uuid::from_u128(0x0141_3701)),
            PhaseId::from_uuid(Uuid::from_u128(0x0141_3702)),
        );
        GraphBody {
            graph: htui_core::model::StepGraph {
                id: graph,
                scope: GraphScope::Library,
                name: "flow".to_owned(),
                description: String::new(),
                is_override: false,
                revision: 3,
                created_at: now(),
                updated_at: now(),
            },
            nodes: vec![
                step(graph, research, 0, "research"),
                step(graph, plan, 1, "plan"),
            ],
            edges: vec![StepGraphEdge {
                id: EdgeId::from_uuid(Uuid::from_u128(0x0141_3703)),
                graph_id: graph,
                from: research,
                to: plan,
                kind: EdgeKind::Next,
                predicate: None,
                loop_limit: None,
            }],
        }
    }

    /// The three reads `GraphEditor::open` sends.
    fn editor_reads(scope: &Scope) -> Vec<StoreRequest> {
        vec![
            StoreRequest::Agents,
            StoreRequest::Templates(scope.clone()),
            StoreRequest::GraphEditorSettings,
        ]
    }

    fn reviewer() -> PersonaSummary {
        PersonaSummary {
            id: PersonaId::from_uuid(Uuid::from_u128(0x0141_3900)),
            name: "reviewer".to_owned(),
        }
    }

    fn catalogue(personas: Vec<PersonaSummary>) -> StoreReply {
        StoreReply::Catalogue(Box::new(CatalogueSnapshot {
            projects: Vec::new(),
            personas,
        }))
    }

    fn open(host: &GraphEditorHost) -> &GraphEditor {
        match &host.state {
            State::Open(editor) => editor,
            State::Loading { .. } => panic!("still loading"),
            State::Refused { message, .. } => panic!("refused: {message}"),
        }
    }

    fn names(body: &GraphBody) -> Vec<&str> {
        body.nodes
            .iter()
            .map(|node| node.phase.name.as_str())
            .collect()
    }

    fn notice_text(editor: &GraphEditor) -> Option<&str> {
        match &editor.notice {
            Some(Notice::Info(text) | Notice::Error(text)) => Some(text),
            None => None,
        }
    }

    fn refused(request: &'static str, message: &str) -> StoreReply {
        StoreReply::Failed {
            request,
            message: message.to_owned(),
        }
    }

    #[tokio::test]
    async fn open_by_id_asks_for_the_body_and_the_personas() {
        let bench = HostBench::new().await;
        let (mut host, requests) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let both = format!(
            "{:?}",
            vec![
                StoreRequest::GraphBody(ids::GRAPH_VULKAN_ANA),
                StoreRequest::Personas
            ]
        );
        assert_eq!(format!("{requests:?}"), both);
        assert_eq!(
            format!("{:?}", host.wants_requests(&bench.scope)),
            both,
            "loading re-sends both"
        );
        assert!(matches!(host.state, State::Loading { .. }));
        assert_eq!(host.graph(), ids::GRAPH_VULKAN_ANA);
        assert_eq!(host.stack(), views::GRAPH_EDITOR);
        assert!(!host.busy());
        assert!(!host.takes_paste());
        assert!(host.editor_mut().is_none());
        assert!(bench.requests().is_empty(), "the caller sends them");

        // A refused registry read is taken; the editor then opens without personas.
        assert!(bench.reply(&mut host, &refused("personas", "refused")));
        let body = bench
            .served(&StoreRequest::GraphBody(ids::GRAPH_VULKAN_ANA))
            .await;
        assert!(bench.reply(&mut host, &body));
        assert!(open(&host).lookups.personas.is_empty());
        assert_eq!(host.graph(), ids::GRAPH_VULKAN_ANA);
        assert_eq!(
            format!("{:?}", bench.requests()),
            format!("{:?}", editor_reads(&bench.scope))
        );
        assert_eq!(
            format!("{:?}", host.wants_requests(&bench.scope)),
            format!("{:?}", editor_reads(&bench.scope)),
            "open: the editor's reads"
        );
        // Once open, a refused registry read is not the host's.
        assert!(!bench.reply(&mut host, &refused("personas", "refused")));
    }

    /// The pattern of `each_scope_saves_its_own_graph_and_revision`, opened by id over the store:
    /// each scope's graph opens, takes an edit, saves on its own revision and reads back renamed.
    #[tokio::test]
    async fn each_scope_opens_by_id_edits_saves_and_rereads() {
        let bench = HostBench::new().await;
        let workspace = bench
            .seeded(GraphScope::Workspace(ids::WORKSPACE_GRAPHICS))
            .await;
        let library = bench.seeded(GraphScope::Library).await;
        for (graph, scope) in [
            (
                ids::GRAPH_VULKAN_ANA,
                GraphScope::Project(ids::PROJECT_VULKAN),
            ),
            (workspace, GraphScope::Workspace(ids::WORKSPACE_GRAPHICS)),
            (library, GraphScope::Library),
        ] {
            let mut host = bench.opened(graph).await;
            let body = &open(&host).body;
            assert_eq!(body.graph.scope, scope, "{scope:?}");
            assert_eq!(names(body)[0], "research", "{scope:?}");
            let revision = body.graph.revision;

            bench.key(&mut host, "e");
            bench.key(&mut host, "2");
            bench.key(&mut host, "enter");
            assert!(host.busy(), "{scope:?}");
            let write = bench.requests();
            let [
                StoreRequest::ReplaceGraph {
                    graph: written,
                    expected_revision,
                    ..
                },
            ] = write.as_slice()
            else {
                panic!("one write: {write:?}");
            };
            assert_eq!(*written, graph, "{scope:?}");
            assert_eq!(*expected_revision, revision, "{scope:?}");
            bench.answer(&mut host, &write).await;
            assert!(!host.busy(), "{scope:?}");
            let editor = open(&host);
            assert_eq!(editor.body.graph.revision, revision + 1, "{scope:?}");
            assert_eq!(notice_text(editor), Some("saved: editing research"));

            let mut again = bench.opened(graph).await;
            let body = &open(&again).body;
            assert_eq!(body.graph.revision, revision + 1, "{scope:?}");
            assert_eq!(names(body)[0], "research2", "{scope:?}");
            assert_eq!(again.graph(), graph);
            assert!(again.editor_mut().is_some());
        }
    }

    #[tokio::test]
    async fn a_missing_graph_is_refused_and_close_still_answers() {
        let bench = HostBench::new().await;
        let missing = StepGraphId::new();
        let (mut host, _) = GraphEditorHost::open_by_id(missing);
        let reply = bench.served(&StoreRequest::GraphBody(missing)).await;
        assert!(matches!(reply, StoreReply::GraphBody(None)));
        assert!(bench.reply(&mut host, &reply));
        assert!(
            matches!(&host.state, State::Refused { message, .. } if message == GRAPH_NOT_FOUND)
        );
        assert_eq!(host.graph(), missing);
        assert!(host.editor_mut().is_none());
        assert!(host.wants_requests(&bench.scope).is_empty());
        assert!(bench.requests().is_empty(), "nothing opened, nothing read");
        assert_eq!(
            bench.render(&host, 40, 3).lines().next(),
            Some(GRAPH_NOT_FOUND)
        );
        // The personas still land (and go nowhere); the state stays refused.
        let registry = bench.served(&StoreRequest::Personas).await;
        assert!(bench.reply(&mut host, &registry));
        assert!(matches!(host.state, State::Refused { .. }));

        assert_eq!(
            bench.key(&mut host, "e"),
            EditorKey::Handled(Handled::Consumed)
        );
        let close = bench.close_chord();
        assert_eq!(bench.key(&mut host, &close), EditorKey::Close);
    }

    #[tokio::test]
    async fn a_refused_body_read_says_the_stores_sentence() {
        let bench = HostBench::new().await;
        let (mut host, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let sentence = "the database is unreachable";
        assert!(bench.reply(&mut host, &refused("graph_body", sentence)));
        assert!(matches!(&host.state, State::Refused { message, .. } if message == sentence));
        assert_eq!(bench.render(&host, 40, 3).lines().next(), Some(sentence));
        // Only a loading host takes it: refused, and open, it is another view's.
        assert!(!bench.reply(&mut host, &refused("graph_body", "again")));
        let (mut open_host, _) =
            GraphEditorHost::with_body(library_body(), Vec::new(), &bench.scope);
        assert!(!bench.reply(&mut open_host, &refused("graph_body", sentence)));
        assert!(notice_text(open(&open_host)).is_none());
    }

    #[tokio::test]
    async fn another_graphs_body_is_not_taken() {
        let bench = HostBench::new().await;
        let other = bench.seeded(GraphScope::Library).await;
        let (mut host, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let reply = bench.served(&StoreRequest::GraphBody(other)).await;
        assert!(matches!(reply, StoreReply::GraphBody(Some(_))));
        assert!(!bench.reply(&mut host, &reply));
        assert!(matches!(host.state, State::Loading { .. }));
        assert_eq!(host.graph(), ids::GRAPH_VULKAN_ANA);
        assert!(bench.requests().is_empty());

        // Open, no body is the host's, not even its own graph's.
        let mut host = bench.opened(ids::GRAPH_VULKAN_ANA).await;
        let own = bench
            .served(&StoreRequest::GraphBody(ids::GRAPH_VULKAN_ANA))
            .await;
        assert!(!bench.reply(&mut host, &own));
        assert!(!bench.reply(&mut host, &StoreReply::GraphBody(None)));
    }

    #[tokio::test]
    async fn personas_reach_the_editor_before_and_after_it_opens() {
        let bench = HostBench::new().await;
        let scout = NewPersona {
            id: PersonaId::new(),
            name: "scout".to_owned(),
            description: "Scouts ahead.".to_owned(),
            body: "You scout.\n".to_owned(),
            tools: PersonaTools::default(),
            permission: PersonaPermission::default(),
        };
        let _ = bench
            .served(&StoreRequest::CreatePersona { new: scout })
            .await;
        let registry = bench.registry().await;
        assert!(
            registry.iter().any(|persona| persona.name == "scout"),
            "{registry:?}"
        );
        let StoreReply::Personas(rows) = bench.served(&StoreRequest::Personas).await else {
            panic!("the registry reads");
        };
        assert_eq!(rows.len(), registry.len());

        // Loading: a catalogue's personas are held, then the registry's replace them.
        let (mut host, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        assert!(bench.reply(&mut host, &catalogue(vec![reviewer()])));
        assert!(
            matches!(&host.state, State::Loading { personas: Some(held), .. } if *held == [reviewer()])
        );
        assert!(bench.reply(&mut host, &StoreReply::Personas(rows.clone())));
        assert!(
            matches!(&host.state, State::Loading { personas: Some(held), .. } if *held == registry)
        );
        host.set_personas(&[reviewer()]);
        assert!(
            matches!(&host.state, State::Loading { personas: Some(held), .. } if *held == [reviewer()])
        );
        host.set_personas(&registry);
        let body = bench
            .served(&StoreRequest::GraphBody(ids::GRAPH_VULKAN_ANA))
            .await;
        assert!(bench.reply(&mut host, &body));
        assert_eq!(open(&host).lookups.personas, registry, "opened with them");

        // Open: each source reaches the editor.
        assert!(bench.reply(&mut host, &catalogue(vec![reviewer()])));
        assert_eq!(open(&host).lookups.personas, [reviewer()]);
        assert!(bench.reply(&mut host, &StoreReply::Personas(rows)));
        assert_eq!(open(&host).lookups.personas, registry);
        host.set_personas(&[]);
        assert!(open(&host).lookups.personas.is_empty());

        // A write's answer carries the catalogue; its personas reach the editor, and an answer
        // without one leaves them.
        let replaced = |catalogue: Option<Vec<PersonaSummary>>| StoreReply::GraphReplaced {
            outcome: htui_core::store::CasOutcome::Stale(Box::new(open(&host).body.clone())),
            catalogue: catalogue.map(|personas| {
                Box::new(CatalogueSnapshot {
                    projects: Vec::new(),
                    personas,
                })
            }),
        };
        let with = replaced(Some(vec![reviewer()]));
        let without = replaced(None);
        assert!(bench.reply(&mut host, &with));
        assert_eq!(open(&host).lookups.personas, [reviewer()]);
        assert!(bench.reply(&mut host, &without));
        assert_eq!(open(&host).lookups.personas, [reviewer()]);
    }

    /// Kinds opens on the body its catalogue holds: exactly `GraphEditor::open`'s reads, no
    /// persona read (its personas come from its catalogue).
    #[tokio::test]
    async fn with_body_sends_exactly_the_editors_three_reads() {
        let bench = HostBench::new().await;
        let (mut host, requests) =
            GraphEditorHost::with_body(library_body(), vec![reviewer()], &bench.scope);
        let (_, direct) = GraphEditor::open(library_body(), Vec::new(), &bench.scope);
        assert_eq!(format!("{requests:?}"), format!("{direct:?}"));
        assert_eq!(
            format!("{requests:?}"),
            format!("{:?}", editor_reads(&bench.scope))
        );
        assert!(bench.requests().is_empty(), "the caller sends them");
        assert_eq!(host.graph(), library_body().graph.id);
        assert_eq!(open(&host).lookups.personas, [reviewer()]);
        assert_eq!(host.stack(), views::GRAPH_EDITOR);
        assert!(host.editor_mut().is_some());
        assert_eq!(
            host.hint_text(&bench.keys),
            open(&host).hint_text(&bench.keys)
        );
    }

    #[tokio::test]
    async fn the_editors_own_replies_pass_through() {
        let bench = HostBench::new().await;
        let (mut host, requests) =
            GraphEditorHost::with_body(library_body(), Vec::new(), &bench.scope);
        bench.answer(&mut host, &requests).await;
        let editor = open(&host);
        assert!(editor.lookups.agents.is_some());
        assert!(editor.lookups.templates.is_some());
        assert!(editor.settings.is_some());

        for (request, word) in [
            ("agents", "agents"),
            ("templates", "templates"),
            ("graph_editor_settings", "run limits"),
        ] {
            assert!(
                bench.reply(&mut host, &refused(request, "refused")),
                "{request}"
            );
            assert_eq!(
                notice_text(open(&host)),
                Some(format!("{word} unavailable: refused").as_str())
            );
        }

        // The write's refusal reaches the editor, which clears its write in flight.
        bench.key(&mut host, "e");
        bench.key(&mut host, "2");
        bench.key(&mut host, "enter");
        assert!(host.busy());
        assert_eq!(bench.requests().len(), 1, "one write");
        assert!(bench.reply(&mut host, &refused("replace_graph", "no such template")));
        assert!(!host.busy());
        assert_eq!(notice_text(open(&host)), Some("no such template"));

        // Another view's reply is not the host's.
        assert!(!bench.reply(&mut host, &refused("catalogue", "refused")));
        assert!(!bench.reply(&mut host, &refused("item_kinds", "refused")));
        assert_eq!(notice_text(open(&host)), Some("no such template"));

        // Loading, the editor's replies are not taken: nothing is open to take them.
        let (mut loading, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let agents = bench.served(&StoreRequest::Agents).await;
        assert!(!bench.reply(&mut loading, &agents));
        assert!(!bench.reply(&mut loading, &refused("agents", "refused")));
        assert!(!bench.reply(&mut loading, &refused("replace_graph", "refused")));
    }

    #[tokio::test]
    async fn loading_draws_one_line_and_offers_close() {
        let bench = HostBench::new().await;
        let (mut host, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let screen = bench.render(&host, 60, 4);
        let rows: Vec<&str> = screen.lines().collect();
        assert_eq!(rows, [READING_GRAPH, "", "", ""], "{screen}");

        let close = bench.close_chord();
        let hint = host.hint_text(&bench.keys);
        assert_eq!(
            hint,
            bench.keys.hint(
                views::GRAPH_EDITOR,
                &[Hint::One(Act::GraphEditorClose, "close")]
            )
        );
        assert!(hint.contains(&close), "{hint}");
        assert!(hint.contains("close"), "{hint}");
        assert!(!hint.contains("edit"), "{hint}");

        assert!(!host.takes_paste());
        assert_eq!(host.on_paste("flow"), Handled::Pass);
        let mut ctx = bench.ctx();
        let press = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(host.on_mouse(press, &mut ctx), Handled::Pass);
        host.on_mouse_lost();
        assert!(!host.busy());
        assert_eq!(
            bench.key(&mut host, "e"),
            EditorKey::Handled(Handled::Consumed)
        );
        assert!(matches!(host.state, State::Loading { .. }));
        assert_eq!(bench.key(&mut host, &close), EditorKey::Close);
    }

    #[tokio::test]
    async fn debug_prints_no_step_names() {
        let bench = HostBench::new().await;
        let (host, _) = GraphEditorHost::with_body(library_body(), Vec::new(), &bench.scope);
        let printed = format!("{host:?}");
        assert!(printed.contains("open"), "{printed}");
        assert!(
            printed.contains(&format!("{:?}", library_body().graph.id)),
            "{printed}"
        );
        for name in ["research", "plan", "flow"] {
            assert!(!printed.contains(name), "{printed}");
        }

        let (mut loading, _) = GraphEditorHost::open_by_id(ids::GRAPH_VULKAN_ANA);
        let printed = format!("{loading:?}");
        assert!(printed.contains("loading"), "{printed}");
        assert!(
            printed.contains(&format!("{:?}", ids::GRAPH_VULKAN_ANA)),
            "{printed}"
        );
        bench.reply(&mut loading, &refused("graph_body", "secret research path"));
        let printed = format!("{loading:?}");
        assert!(printed.contains("refused"), "{printed}");
        assert!(!printed.contains("research"), "{printed}");
    }
}
