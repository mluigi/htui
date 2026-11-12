//! The kinds section of the Settings tab: every project of the scope with its item kinds, the
//! graphs those kinds run and the phases of each graph, editable in place (MOD-15 milestone 4,
//! D4/D10/D11/D15/D16; `R-ENT-6`, `R-ORCH-1`, `R-TUI-8`).
//!
//! It holds **no store handle, no `UserId` and no `BoxId`** (`R-NF-3`): it names one read
//! ([`StoreRequest::Catalogue`]), is handed the catalogue that comes back, and every write leaves
//! through `ctx.request` for [`crate::catalogue::serve`] to carry out. What is on screen is always
//! the last snapshot the worker assembled — no row is ever patched in locally, so there is exactly
//! one source of truth (D3).
//!
//! The tree is D4's: a project, then each kind with the phases of **its default graph** directly
//! under it, then every graph no kind points at with its own phases. The seed is one graph per
//! kind, so the common shape is kind-then-phases; listing the unreferenced graphs separately is
//! what keeps a graph created here from becoming invisible the moment nothing names it.
//!
//! The phases themselves are edited in the graph editor (MOD-141 D2, D3, D14): `Enter` opens it
//! over the whole section on the row's graph, and `e` on a phase row opens it with that step's form
//! open. The tree's phase rows are read-only lines; a bound persona is named on its phase's line.
//!
//! Every mode resolves its keys through its own stack in [`views`] (MOD-67 M3), so a chord
//! matches with its modifiers (`ctrl-d` is not `d`) and a rebound key acts under its new chord;
//! the hint row, and the delete question's own keys, are rendered from the same stack. A
//! capturing or confirming mode swallows what it does not bind, except the chords its modal global
//! layer admits (`CONTROL`, `ALT`, a function key), so `ctrl-c` quits and `F1` opens help.
//!
//! It draws through the kit (MOD-115 M3 L-kinds): the tree is a [`list`] with the project as a
//! group row and the read-only detail as note rows, an editor is a [`Form`] whose focused row
//! carries the marker, the outcome is a [`notice`] line and an empty tree is a [`empty`] state
//! with a next step. The editor's own keys and every sentence that names a key read the key from
//! the stack in force (MD-A), and the acts resolve from [`Ctx::pressed`].

use core::cell::Cell;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use chrono::{DateTime, Utc};
use htui_core::model::{
    ItemKind, ItemKindId, ItemKindPatch, PersonaId, ProjectId, Scope, StepGraphId, StepGraphPatch,
    StepGraphPhase,
};

use htui_core::store::CasOutcome;

use crate::app::{Ctx, Handled};
use crate::catalogue::{
    CatalogueSnapshot, GraphEntry, PersonaSummary, ProjectCatalogue, REQUEST_NAMES, graph_label,
};
use crate::hierarchy::MirrorAfterDelete;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::graph_editor::{EditorKey, GraphEditor};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::label;
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyEvent, MouseEvent};

/// What the rows pane says before any catalogue has arrived.
const NO_WORKSPACE: &str = "no workspace: nothing to list";

/// What the rows pane says when the read answered with no project at all.
const NO_PROJECT: &str = "no project in scope";

/// The next step of [`NO_PROJECT`]: projects are the hierarchy section's (B-13).
const NO_PROJECT_NEXT: &str = "Settings \u{203a} Hierarchy adds a project";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "catalogue unavailable";

/// What a kind whose `default_graph_id` names no graph of its project says (D4).
const GRAPH_MISSING: &str = "graph missing";

/// Browse's keys.
const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "kind"),
    Hint::One(Act::KindsNewGraph, "graph"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::KindsGraph, "graph"),
    Hint::One(Act::KindsEditor, "graph editor"),
    Hint::One(Act::Delete, "delete kind"),
    Hint::One(Act::Reload, "reload"),
];

/// Browse's keys with nothing to list: the only offer is to ask again.
const HINT_NO_WORKSPACE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// Browse's keys with the read refused: nothing here can be written against a store that did not
/// answer.
const HINT_UNAVAILABLE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// An open editor's keys: the form's own, then the field's submit and cancel through the
/// editor's `[field]` layer (D13; MOD-115 M3 MD-A, C6), `Enter save · Esc cancel` with the
/// compiled keys.
const HINT_EDITING: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
];

/// The prefix warning's keys (D10).
const HINT_CONFIRM_PREFIX: HintSpec = &[
    Hint::One(Act::ConfirmYes, "write"),
    Hint::All(Act::ConfirmNo, "back to the editor"),
];

/// What `e`, `d` and `g` say on a project row: this section owns what is inside a project, and the
/// hierarchy section owns the project (B-13).
const PROJECTS_ELSEWHERE: &str = "projects are edited in Hierarchy";

/// What the kind editor says when its `graph` field names nothing (B-7).
const NO_GRAPH_NAMED: &str = "`graph` names no graph this project can use";

/// What an editor says when a `position` field is not a number (B-8).
const POSITION_IS_A_NUMBER: &str = "`position` is a whole number";

/// The delete confirmation's keys (D11), on the hint row and at the end of the question.
const HINT_DELETING: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// What `d` says on a graph row: the seam has `delete_item_kind` and nothing else in this area,
/// and this milestone adds no method to it (D5). A phase row's `d` points at the graph editor
/// instead (MOD-141 D14), where steps are deleted.
const NOT_DELETED_HERE: &str = "graphs are not deleted here";

/// What one line of the tree is, by index into the snapshot.
///
/// Indices rather than ids: the list is rebuilt from the snapshot on every reply, and an index that
/// outlives its tree is caught by the clamp, where a stale id would silently select nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// One project of the scope.
    Project {
        /// Index into `snapshot.projects`.
        p: usize,
    },
    /// One kind of that project.
    Kind {
        /// Index into `snapshot.projects`.
        p: usize,
        /// Index into that project's `kinds`.
        k: usize,
    },
    /// A graph **no kind points at** (D4).
    Graph {
        /// Index into `snapshot.projects`.
        p: usize,
        /// Index into that project's `graphs`.
        g: usize,
    },
    /// `projects[p].graphs[g].nodes[i].phase`, whichever parent it is drawn under.
    ///
    /// A phase under a kind and a phase under an unreferenced graph are the same row: both carry
    /// the graph's index, and every key on a phase row acts on the graph. Indentation is the only
    /// difference and it is decided at line time (B-6).
    Phase {
        /// Index into `snapshot.projects`.
        p: usize,
        /// Index into that project's `graphs`.
        g: usize,
        /// Index into that graph's `phases`.
        i: usize,
    },
}

/// Which row an open editor writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    /// `n` on a project or kind row.
    NewKind(ProjectId),
    /// `e` on a kind row.
    EditKind(ItemKindId),
    /// `N` on any row.
    NewGraph(ProjectId),
    /// `e` on a graph row, and `g` on a kind, phase or graph row (D19).
    EditGraph(StepGraphId),
}

/// One labelled input of an editor.
#[derive(Debug)]
struct Field {
    /// What the pane prints in front of it.
    label: &'static str,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
    /// Whether `Enter` refuses while it is empty.
    required: bool,
}

/// The open editor: which row, its fields, which one has focus, and the token it opened on.
struct Editor {
    /// The row this writes back to.
    kind: EditorKind,
    /// The inputs, in tab order.
    fields: Vec<Field>,
    /// Index into `fields`.
    focus: usize,
    /// The `updated_at` of the row this opened on; `None` for a create (M1 D3).
    expected: Option<DateTime<Utc>>,
    /// A kind edit's stored prefix, for D10's comparison; `None` for every other editor.
    stored_prefix: Option<String>,
}

impl Editor {
    /// The prefix this edit would change, as `(old, new)`, or `None` when it changes none.
    ///
    /// Only a kind edit has one: a create mints nothing yet, and no other row carries a prefix.
    fn prefix_change(&self) -> Option<(String, String)> {
        let old = self.stored_prefix.clone()?;
        let new = self.text(0);
        (new != old).then_some((old, new))
    }
}

/// Labels, never what was typed into them (H-7).
///
/// Hand-written because [`KindsSection`] derives `Debug` and one `tracing::debug!` of a section is
/// all it takes for a field's text to reach a log. [`TextField`]'s own `Debug` holds the same line.
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("kind", &self.kind)
            .field(
                "fields",
                &self
                    .fields
                    .iter()
                    .map(|field| field.label)
                    .collect::<Vec<_>>(),
            )
            .field("focus", &self.focus)
            .field("expected", &self.expected)
            .finish()
    }
}

/// What the section is doing. `Browse` is not a mode in the modal sense: it captures nothing.
#[derive(Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
    /// One kind being deleted, behind D11's single confirmation.
    Deleting {
        /// The kind the question is about.
        id: ItemKindId,
        /// Its name, for the question.
        name: String,
        /// Its prefix, for the question and for the notice afterwards.
        prefix: String,
        /// How far the confirmation has got.
        stage: DeleteStage,
    },
    /// A kind edit whose prefix changed, held in front of the user before it is written (D10).
    ///
    /// The editor is kept whole, so `n`/`Esc` return to it with its text and `y` writes exactly
    /// what was read.
    ConfirmPrefix {
        /// The editor the warning was raised from.
        editor: Editor,
        /// The prefix the kind carries now.
        old: String,
        /// The prefix that was typed.
        new: String,
    },
    /// MOD-141 D2: the graph editor on one of the project's own graphs, over the whole section. It
    /// takes every key, paste and its own replies while open; its close act returns to Browse.
    Graph {
        /// The editor (boxed: it is by far the largest variant).
        editor: Box<GraphEditor>,
    },
}

/// Everything the mode is *about*, never what was typed into it (H-7).
///
/// Hand-written for the same reason [`Editor`]'s is: [`KindsSection`] derives `Debug` through this
/// enum, so a variant that prints one of its own fields' text needs only one `tracing::debug!` of a
/// section to reach a log. `ConfirmPrefix`'s `new` **is** field 0's typed text — it is read off the
/// editor at submit — so it is the one value this printer drops. `old` comes from the row and
/// stays.
impl core::fmt::Debug for Mode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Browse => f.write_str("Browse"),
            Self::Editing(editor) => f.debug_tuple("Editing").field(editor).finish(),
            Self::Deleting {
                id,
                name,
                prefix,
                stage,
            } => f
                .debug_struct("Deleting")
                .field("id", id)
                .field("name", name)
                .field("prefix", prefix)
                .field("stage", stage)
                .finish(),
            Self::ConfirmPrefix { editor, old, .. } => f
                .debug_struct("ConfirmPrefix")
                .field("editor", editor)
                .field("old", old)
                .finish_non_exhaustive(),
            Self::Graph { editor } => f.debug_struct("Graph").field("editor", editor).finish(),
        }
    }
}

/// The two stages of a kind delete (D11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteStage {
    /// The question is on screen and `y` answers it.
    Asking,
    /// `delete_kind` is out; there is nothing left to answer.
    InFlight,
}

/// The last outcome, and whether it is one the user has to act on.
///
/// Carried with the text rather than derived from it: a refusal is often the **seam's** own
/// sentence (`item_kind FEAT is held by 4 items`), and no string rule can tell one of those from a
/// line of good news. The two sentences both sections coin are classified by the shared
/// [`is_error`](super::is_error), so those two cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// One line of report.
    Info(String),
    /// One line the user has to act on, drawn in `theme.error`.
    Error(String),
}

impl Notice {
    /// The sentence.
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }

    /// Whether it belongs in `theme.error`.
    fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// The catalogue of the scope, with the keys that edit it.
#[derive(Debug, Default)]
pub struct KindsSection {
    /// The last catalogue the worker assembled, or `None` before the first reply.
    snapshot: Option<CatalogueSnapshot>,
    /// `Some(message)` after `Failed { request: "catalogue" }`: the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](KindsSection::rows).
    cursor: usize,
    /// Browsing, or typing.
    mode: Mode,
    /// The write in flight, by [`StoreRequest::name`]. A second one is refused until the reply: the
    /// staleness index keeps only the newest request of a kind, so two writes of one kind racing
    /// would lose the reply about the one that landed.
    busy: Option<&'static str>,
    /// The last outcome, on the notice row under the pane.
    notice: Option<Notice>,
    /// The tree's lines, first line drawn and rows as of the last render, for the Settings
    /// pane's scrollbar (MOD-115 M3 D-11), measured against the rows the tree is drawn into
    /// (RV1 M2). `None` while no tree is drawn.
    scroll: Cell<Option<ScrollSpan>>,
}

impl KindsSection {
    /// Identity of the kinds section.
    pub const ID: SectionId = SectionId("kinds");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The flat list the cursor indexes (D4).
    ///
    /// Derived on demand rather than cached beside the snapshot, so the two cannot disagree about
    /// what is on screen (D3).
    fn rows(&self) -> Vec<Row> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (p, project) in snapshot.projects.iter().enumerate() {
            rows.push(Row::Project { p });
            for (k, kind) in project.kinds.iter().enumerate() {
                rows.push(Row::Kind { p, k });
                // A kind whose default graph names nothing contributes no phase rows: the line
                // says so instead (D4).
                if let Some(g) = project
                    .graphs
                    .iter()
                    .position(|entry| entry.graph.id == kind.default_graph_id)
                {
                    for i in 0..project.graphs[g].nodes.len() {
                        rows.push(Row::Phase { p, g, i });
                    }
                }
            }
            for (g, entry) in project.graphs.iter().enumerate() {
                if project
                    .kinds
                    .iter()
                    .any(|kind| kind.default_graph_id == entry.graph.id)
                {
                    continue;
                }
                rows.push(Row::Graph { p, g });
                for i in 0..entry.nodes.len() {
                    rows.push(Row::Phase { p, g, i });
                }
            }
        }
        rows
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for the reason the hierarchy section gives: `d` acts on the row the cursor is on,
    /// and a held `j` that wrapped to the top would aim a delete at a row nobody looked at.
    fn move_cursor(&mut self, down: bool) {
        let Some(last) = self.rows().len().checked_sub(1) else {
            self.cursor = 0;
            return;
        };
        self.cursor = if down {
            self.cursor.saturating_add(1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
    }

    /// Puts the cursor back inside the list after a reply replaced the tree.
    ///
    /// Against `rows().len()` and nothing else: the detail line of D15 is not a row, so the cursor
    /// cannot land on it (H-8).
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.rows().len().saturating_sub(1));
    }

    /// One project of the snapshot.
    fn project(&self, p: usize) -> Option<&ProjectCatalogue> {
        self.snapshot.as_ref()?.projects.get(p)
    }

    /// One graph of one project.
    fn graph_at(&self, p: usize, g: usize) -> Option<&GraphEntry> {
        self.project(p)?.graphs.get(g)
    }

    /// One phase of one graph.
    fn phase_at(&self, p: usize, g: usize, i: usize) -> Option<&StepGraphPhase> {
        self.graph_at(p, g)?.nodes.get(i).map(|node| &node.phase)
    }

    /// The row under the cursor, or `None` while there is no tree.
    fn selected(&self) -> Option<Row> {
        self.rows().get(self.cursor).copied()
    }

    /// The project a kind belongs to, with the kind.
    fn locate_kind(&self, id: ItemKindId) -> Option<(&ProjectCatalogue, &ItemKind)> {
        self.snapshot.as_ref()?.projects.iter().find_map(|project| {
            project
                .kinds
                .iter()
                .find(|kind| kind.id == id)
                .map(|kind| (project, kind))
        })
    }

    /// The project a graph belongs to, with the graph.
    fn locate_graph(&self, id: StepGraphId) -> Option<(&ProjectCatalogue, &GraphEntry)> {
        self.snapshot.as_ref()?.projects.iter().find_map(|project| {
            project
                .graphs
                .iter()
                .find(|entry| entry.graph.id == id)
                .map(|entry| (project, entry))
        })
    }

    /// Whether a key that opens an editor is refused right now, with the notice that says why.
    ///
    /// One write of a kind at a time (M3's rule): a second one would have the staleness index drop
    /// the first one's reply, and the first is the one about the write that landed. `r` is
    /// deliberately not on this path — re-reading is how a section that lost a reply recovers.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        self.snapshot.is_none()
    }

    /// Opens an editor, clearing whatever the last one said.
    fn open(&mut self, kind: EditorKind, fields: Vec<Field>, expected: Option<DateTime<Utc>>) {
        self.open_with(kind, fields, expected, None);
    }

    /// The same, for the one editor that carries a stored value to compare what was typed
    /// against: a kind edit's prefix (D10).
    fn open_with(
        &mut self,
        kind: EditorKind,
        fields: Vec<Field>,
        expected: Option<DateTime<Utc>>,
        stored_prefix: Option<String>,
    ) {
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            kind,
            fields,
            focus: 0,
            expected,
            stored_prefix,
        });
    }

    /// `n`: a kind under a project or a kind row (B-13). On a graph or a phase row it points at
    /// the graph editor, where steps are added (MOD-141 D14).
    fn open_new_child(&mut self, row: Row, keys: &Keys) {
        match row {
            Row::Project { p } | Row::Kind { p, .. } => {
                if let Some(project) = self.project(p) {
                    let id = project.project.id;
                    let fields = new_kind_fields(project);
                    self.open(EditorKind::NewKind(id), fields, None);
                }
            }
            Row::Graph { .. } | Row::Phase { .. } => self.refuse(steps_in_the_editor(keys)),
        }
    }

    /// `N`: a graph in the row's project.
    fn open_new_graph(&mut self, row: Row) {
        let (Row::Project { p }
        | Row::Kind { p, .. }
        | Row::Graph { p, .. }
        | Row::Phase { p, .. }) = row;
        if let Some(project) = self.project(p) {
            let id = project.project.id;
            self.open(
                EditorKind::NewGraph(id),
                vec![
                    Field::required("name", ""),
                    Field::optional("description", ""),
                ],
                None,
            );
        }
    }

    /// `e`: the row under the cursor, prefilled, with its `updated_at` as the token. A phase row
    /// opens the graph editor with that step's form open (MOD-141 D14).
    fn open_edit(&mut self, row: Row, ctx: &Ctx<'_>) {
        match row {
            Row::Project { .. } => self.refuse(PROJECTS_ELSEWHERE.to_owned()),
            Row::Kind { p, k } => {
                let Some(project) = self.project(p) else {
                    return;
                };
                let Some(kind) = project.kinds.get(k) else {
                    return;
                };
                let (id, expected, prefix) = (kind.id, kind.updated_at, kind.prefix.clone());
                let fields = edit_kind_fields(project, kind);
                self.open_with(
                    EditorKind::EditKind(id),
                    fields,
                    Some(expected),
                    Some(prefix),
                );
            }
            Row::Graph { p, g } => {
                let Some(entry) = self.graph_at(p, g) else {
                    return;
                };
                let (id, expected) = (entry.graph.id, entry.graph.updated_at);
                let fields = edit_graph_fields(entry);
                self.open(EditorKind::EditGraph(id), fields, Some(expected));
            }
            Row::Phase { .. } => self.open_editor(row, true, ctx),
        }
    }

    /// `g`: the editor of the graph this row belongs to (D19).
    ///
    /// The answer to H-6: D4 gives a `Graph` row only to a graph no kind points at, so without this
    /// key the five seeded graphs — every graph that matters — would have no editable name or
    /// description. One key reaching the editor `e` already opens beats a fifth row variant that
    /// would double the tree.
    fn open_graph(&mut self, row: Row) {
        let id = match row {
            Row::Project { .. } => {
                self.refuse(PROJECTS_ELSEWHERE.to_owned());
                return;
            }
            Row::Kind { p, k } => {
                let Some(kind) = self.project(p).and_then(|project| project.kinds.get(k)) else {
                    return;
                };
                let id = kind.default_graph_id;
                if self.locate_graph(id).is_none() {
                    let refusal = self.not_a_project_graph(p, id);
                    self.refuse(refusal);
                    return;
                }
                id
            }
            Row::Graph { p, g } | Row::Phase { p, g, .. } => {
                let Some(entry) = self.graph_at(p, g) else {
                    return;
                };
                entry.graph.id
            }
        };
        let Some((_, entry)) = self.locate_graph(id) else {
            return;
        };
        let expected = entry.graph.updated_at;
        let fields = edit_graph_fields(entry);
        self.open(EditorKind::EditGraph(id), fields, Some(expected));
    }

    /// Why a kind's default graph `id` opens no editor here: it is a workspace or library graph
    /// (MOD-138 D12: the kind's to name, not this section's to edit; say which, rather than calling
    /// it missing), or it names nothing at all.
    fn not_a_project_graph(&self, p: usize, id: StepGraphId) -> String {
        self.project(p)
            .and_then(|project| project.graph_name(id))
            .map(|(name, scope)| graph_label(name, scope))
            .map_or_else(
                || GRAPH_MISSING.to_owned(),
                |label| format!("graph {label} is not edited here"),
            )
    }

    /// MOD-141 D3: the graph editor on the row's graph, over the whole section (D2). A graph row
    /// opens on its graph, a phase row on its graph with that step selected (`form`: its step form
    /// open too, D14), a kind row on its default graph when that is one of the project's own. A
    /// project row has no graph, and says which rows do. The caller has checked
    /// [`blocked`](KindsSection::blocked).
    fn open_editor(&mut self, row: Row, form: bool, ctx: &Ctx<'_>) {
        let (p, g, step) = match row {
            Row::Project { .. } => {
                let key = ctx.keys().label(views::KINDS_BROWSE, Act::KindsEditor);
                self.refuse(pick_a_graph(key.as_deref()));
                return;
            }
            Row::Kind { p, k } => {
                let Some(id) = self
                    .project(p)
                    .and_then(|project| project.kinds.get(k))
                    .map(|kind| kind.default_graph_id)
                else {
                    return;
                };
                let Some(g) = self.project(p).and_then(|project| {
                    project.graphs.iter().position(|entry| entry.graph.id == id)
                }) else {
                    let refusal = self.not_a_project_graph(p, id);
                    self.refuse(refusal);
                    return;
                };
                (p, g, None)
            }
            Row::Graph { p, g } => (p, g, None),
            Row::Phase { p, g, i } => (p, g, self.phase_at(p, g, i).map(|phase| phase.id)),
        };
        let Some(entry) = self.graph_at(p, g) else {
            return;
        };
        let (mut editor, requests) =
            GraphEditor::open(entry.clone(), self.personas().to_vec(), ctx.scope);
        if let Some(step) = step {
            editor.select(step);
            if form {
                editor.open_node_form();
            }
        }
        for request in requests {
            ctx.request(request);
        }
        self.notice = None;
        self.mode = Mode::Graph {
            editor: Box::new(editor),
        };
    }

    /// MOD-141 D2: the editor closed. Browse again, the cursor on the first row that shows the
    /// graph it was on: the kind pointing at it, or its own row when no kind does.
    fn close_editor(&mut self) {
        let Mode::Graph { editor } = core::mem::take(&mut self.mode) else {
            return;
        };
        let graph = editor.graph();
        let shows = |row: &Row| match *row {
            Row::Kind { p, k } => self
                .project(p)
                .and_then(|project| project.kinds.get(k))
                .is_some_and(|kind| kind.default_graph_id == graph),
            Row::Graph { p, g } => self
                .graph_at(p, g)
                .is_some_and(|entry| entry.graph.id == graph),
            Row::Project { .. } | Row::Phase { .. } => false,
        };
        if let Some(at) = self.rows().iter().position(shows) {
            self.cursor = at;
        }
        self.clamp_cursor();
    }

    /// MOD-141 D2, D13: one reply while the editor is open, `true` when it was the editor's.
    ///
    /// The editor's write is answered with the scope's catalogue: the tree behind it is adopted
    /// quietly (no `busy` of the section's is involved, so nothing closes and nothing is said) and
    /// the outcome is the editor's. A fresh catalogue still goes the section's way, then hands the
    /// persona registry on. The editor's reads and its refusals are its own.
    fn on_editor_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) -> bool {
        match reply {
            StoreReply::GraphReplaced { catalogue, .. } => {
                match catalogue {
                    Some(catalogue) => {
                        self.snapshot = Some((**catalogue).clone());
                        self.unavailable = None;
                        self.clamp_cursor();
                        if let Mode::Graph { editor } = &mut self.mode {
                            editor.set_personas(&catalogue.personas);
                        }
                    }
                    // MOD-141 R1 L1: the write's outcome stands; the tree it would have moved is
                    // read again, and its `Catalogue` hands the personas on.
                    None => ctx.request(StoreRequest::Catalogue(ctx.scope.clone())),
                }
                if let Mode::Graph { editor } = &mut self.mode {
                    editor.on_reply(reply, ctx);
                }
                true
            }
            StoreReply::Catalogue(snapshot) => {
                self.on_catalogue(snapshot);
                if let Mode::Graph { editor } = &mut self.mode {
                    editor.set_personas(&snapshot.personas);
                }
                true
            }
            StoreReply::Agents(_)
            | StoreReply::Templates(_)
            | StoreReply::GraphEditorSettings(_) => {
                if let Mode::Graph { editor } = &mut self.mode {
                    editor.on_reply(reply, ctx);
                }
                true
            }
            StoreReply::Failed { request, .. }
                if matches!(
                    *request,
                    "replace_graph" | "agents" | "templates" | "graph_editor_settings"
                ) =>
            {
                if let Mode::Graph { editor } = &mut self.mode {
                    editor.on_reply(reply, ctx);
                }
                true
            }
            _ => false,
        }
    }

    /// `d`: the one destructive key this section has (D11).
    ///
    /// One confirmation rather than PRD D13's two: a kind any item holds cannot be deleted at all
    /// (D6), so the destructive case the typed slug exists for does not arise — and two
    /// confirmations over an act the database refuses would train the reflex D13 exists to defeat.
    fn begin_delete(&mut self, row: Row, keys: &Keys) {
        match row {
            Row::Project { .. } => self.refuse(PROJECTS_ELSEWHERE.to_owned()),
            Row::Graph { .. } => self.refuse(NOT_DELETED_HERE.to_owned()),
            // MOD-141 D14: a step is deleted in the graph editor.
            Row::Phase { .. } => self.refuse(steps_in_the_editor(keys)),
            Row::Kind { p, k } => {
                let Some(kind) = self.project(p).and_then(|project| project.kinds.get(k)) else {
                    return;
                };
                let (id, name, prefix) = (kind.id, kind.name.clone(), kind.prefix.clone());
                self.notice = None;
                self.mode = Mode::Deleting {
                    id,
                    name,
                    prefix,
                    stage: DeleteStage::Asking,
                };
            }
        }
    }

    /// One key while a kind delete is being confirmed.
    ///
    /// Modal over the shell as well as over the tree, as the prefix warning is: an unlisted key is
    /// swallowed so a `q` at the question does not quit the application, with the chords the
    /// modal global layer admits excepted, so `ctrl-c` still quits and `F1` opens help (MOD-67
    /// D5). Both stages answer through `views::KINDS_CONFIRM`; in flight an answer is a no-op.
    fn on_deleting_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::KINDS_CONFIRM;
        let chord = ctx.pressed(&key);
        let answer = ctx.keys().actions(stack, chord).first().copied();
        let Mode::Deleting { id, stage, .. } = &mut self.mode else {
            return Handled::Pass;
        };
        let id = *id;
        match (answer, stage) {
            (Some(Act::ConfirmYes), stage @ DeleteStage::Asking) => {
                *stage = DeleteStage::InFlight;
                self.notice = None;
                self.send(
                    StoreRequest::DeleteKind {
                        scope: ctx.scope.clone(),
                        id,
                    },
                    ctx,
                );
                Handled::Consumed
            }
            (Some(Act::ConfirmNo), DeleteStage::Asking) => {
                self.mode = Mode::Browse;
                Handled::Consumed
            }
            // Nothing to answer: the row is already going (blueprint L-C Q8).
            (Some(Act::ConfirmYes | Act::ConfirmNo), DeleteStage::InFlight) => Handled::Consumed,
            // A global act, or nothing.
            _ => modal_rest(stack, chord),
        }
    }

    /// Sends one write and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    /// One key while an editor is open.
    ///
    /// The focused field answers first, so `l`, `q` and the digits are letters here (D13); what it
    /// passes on is the form's own navigation (`form.next_field`/`prev_field` through
    /// `views::KINDS_EDITOR`, `Down`/`Up` included), and everything left over is swallowed rather
    /// than offered to the shell — with the chords the modal global layer admits excepted, so
    /// `ctrl-c` still quits and `F1` opens help (MOD-67 D5).
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::KINDS_EDITOR;
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => match editor.fields.get_mut(editor.focus) {
                Some(field) => field.input.on_key(key),
                None => FieldOutcome::Pass,
            },
            Mode::Browse
            | Mode::ConfirmPrefix { .. }
            | Mode::Deleting { .. }
            | Mode::Graph { .. } => {
                return Handled::Pass;
            }
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => {
                // The chord the user pressed (MOD-115 M3 MD-A): a `[field]` chord the field
                // passed is that row's, never re-read as another act of its canonical key.
                let chord = ctx.pressed(&key);
                let Mode::Editing(editor) = &mut self.mode else {
                    return Handled::Pass;
                };
                let len = editor.fields.len().max(1);
                match ctx.keys().actions(stack, chord).first() {
                    Some(Act::FormNextField) => {
                        editor.focus = (editor.focus + 1) % len;
                        Handled::Consumed
                    }
                    Some(Act::FormPrevField) => {
                        editor.focus = (editor.focus + len - 1) % len;
                        Handled::Consumed
                    }
                    // A global act, or nothing.
                    _ => modal_rest(stack, chord),
                }
            }
        }
    }

    /// `Enter` in an editor: the required fields, then one request per [`EditorKind`].
    ///
    /// The editor **stays open** until the reply lands, so a refusal leaves the text where it was
    /// and a second `Enter` retries it. Which is why the first statement is the refusal the Browse
    /// keys get: the editor being open is not a reply, and a second `Enter` before one arrives
    /// would re-send a write that already landed.
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return;
        }
        let Mode::Editing(editor) = &self.mode else {
            return;
        };
        if let Some(missing) = editor
            .fields
            .iter()
            .find(|field| field.required && field.text().is_empty())
        {
            let label = missing.label;
            self.refuse(required(label));
            return;
        }
        // Built before the warning of D10 is raised, so a `graph` or `position` this section
        // cannot parse is refused first: a warning about a write that could not happen anyway
        // would be one question too many.
        let built = self.build(editor, ctx.scope);
        if built.is_ok()
            && let Some((old, new)) = editor.prefix_change()
        {
            let Mode::Editing(editor) = core::mem::take(&mut self.mode) else {
                return;
            };
            self.notice = None;
            self.mode = Mode::ConfirmPrefix { editor, old, new };
            return;
        }
        match built {
            Ok(request) => {
                self.notice = None;
                self.send(request, ctx);
            }
            Err(why) => self.refuse(why),
        }
    }

    /// One key while the prefix warning is up (D10).
    ///
    /// Modal over the shell as well as over the tree: every key that is not listed is swallowed, so
    /// a `q` typed at the warning does not quit the application (H-10). A chord the modal global
    /// layer admits (`CONTROL`, `ALT`, a function key) is the carve-out, so `ctrl-c` still quits
    /// and `F1` opens help (MOD-67 D5). The answers are `confirm.*` through
    /// `views::KINDS_CONFIRM`.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::KINDS_CONFIRM;
        let chord = ctx.pressed(&key);
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes) => {
                let Mode::ConfirmPrefix { editor, .. } = core::mem::take(&mut self.mode) else {
                    return Handled::Consumed;
                };
                let built = self.build(&editor, ctx.scope);
                // The editor is put back rather than closed: the write is in flight and a
                // `CatalogueStale` has to find something to re-take the token for (D8).
                self.mode = Mode::Editing(editor);
                match built {
                    Ok(request) => {
                        self.notice = None;
                        self.send(request, ctx);
                    }
                    Err(why) => self.refuse(why),
                }
                Handled::Consumed
            }
            Some(Act::ConfirmNo) => {
                let Mode::ConfirmPrefix { editor, .. } = core::mem::take(&mut self.mode) else {
                    return Handled::Consumed;
                };
                self.mode = Mode::Editing(editor);
                Handled::Consumed
            }
            // A global act, or nothing.
            _ => modal_rest(stack, chord),
        }
    }

    /// The request one editor stands for, or the sentence that says why there is none.
    ///
    /// Every editable column goes out on an edit whether or not it changed (B-5): one habit across
    /// both sections, and the compare-and-set token is what makes it safe.
    fn build(&self, editor: &Editor, scope: &Scope) -> Result<StoreRequest, String> {
        match editor.kind {
            EditorKind::NewKind(project) => {
                let owner = self
                    .project_by_id(project)
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok(StoreRequest::CreateKind {
                    scope: scope.clone(),
                    project,
                    prefix: editor.text(0),
                    name: editor.text(1),
                    description: editor.text(2),
                    graph: graph_named(owner, &editor.text(3), None)?,
                    position: position(&editor.text(4))?,
                })
            }
            EditorKind::EditKind(id) => {
                let expected = editor
                    .expected
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                let (owner, kind) = self
                    .locate_kind(id)
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok(StoreRequest::UpdateKind {
                    scope: scope.clone(),
                    id,
                    expected,
                    patch: ItemKindPatch {
                        prefix: Some(editor.text(0)),
                        name: Some(editor.text(1)),
                        description: Some(editor.text(2)),
                        default_graph_id: Some(graph_named(
                            owner,
                            &editor.text(3),
                            Some(kind.default_graph_id),
                        )?),
                        position: Some(position(&editor.text(4))?),
                    },
                })
            }
            EditorKind::NewGraph(project) => Ok(StoreRequest::CreateGraph {
                scope: scope.clone(),
                project,
                name: editor.text(0),
                description: editor.text(1),
            }),
            EditorKind::EditGraph(id) => {
                let expected = editor
                    .expected
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok(StoreRequest::UpdateGraph {
                    scope: scope.clone(),
                    id,
                    expected,
                    patch: StepGraphPatch {
                        name: Some(editor.text(0)),
                        description: Some(editor.text(1)),
                    },
                })
            }
        }
    }

    /// The persona registry the last catalogue carried, or none before the first one (D23).
    fn personas(&self) -> &[PersonaSummary] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.personas.as_slice())
    }

    /// One project of the snapshot, by id.
    fn project_by_id(&self, id: ProjectId) -> Option<&ProjectCatalogue> {
        self.snapshot
            .as_ref()?
            .projects
            .iter()
            .find(|project| project.project.id == id)
    }

    /// A kind is gone, with what the worker did to the mirror afterwards (D12).
    ///
    /// The prefix comes from the mode when there is one; a reply that arrives with none — a scope
    /// change dropped it, say — still reports the mirror rather than panicking (H-16).
    fn on_deleted(&mut self, mirror: &MirrorAfterDelete, snapshot: &CatalogueSnapshot) {
        self.busy = None;
        let prefix = match &self.mode {
            Mode::Deleting { prefix, .. } => Some(prefix.clone()),
            Mode::Browse | Mode::Editing(_) | Mode::ConfirmPrefix { .. } | Mode::Graph { .. } => {
                None
            }
        };
        self.mode = Mode::Browse;
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        self.say(&deleted_notice(prefix.as_deref(), mirror));
    }

    /// A compare-and-set miss (D8): the tree is replaced, the editor keeps its text and takes the
    /// current row's token, and the retry is a second `Enter` rather than an automatic write. The
    /// sentence names that key as `keys` binds it through the editor's stack (MOD-115 M3 D-5).
    fn on_stale(&mut self, snapshot: &CatalogueSnapshot, keys: &Keys) {
        let retry = changed_elsewhere(keys.label(views::KINDS_EDITOR, Act::FieldSubmit).as_deref());
        self.busy = None;
        // A warning collapses back to the editor it was raised from: the prefix it compared
        // against may be the very thing that moved, so the question is asked again against the row
        // as it is now (D10).
        if matches!(self.mode, Mode::ConfirmPrefix { .. })
            && let Mode::ConfirmPrefix { editor, .. } = core::mem::take(&mut self.mode)
        {
            self.mode = Mode::Editing(editor);
        }
        let reloaded = match &self.mode {
            Mode::Editing(editor) => Some(reload(snapshot, editor.kind)),
            Mode::Browse
            | Mode::ConfirmPrefix { .. }
            | Mode::Deleting { .. }
            | Mode::Graph { .. } => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        match reloaded {
            // Nothing is open to press `Enter` on, so the sentence has to carry what the editor
            // would otherwise stand for: the write did not apply, and the way back is to reopen.
            None => self.say(CHANGED_ELSEWHERE_CLOSED),
            Some(Reload::Keep) => self.say(&retry),
            Some(Reload::Gone) => {
                self.mode = Mode::Browse;
                self.say(DELETED_ELSEWHERE);
            }
            Some(Reload::Token(token)) => {
                let stored = self.stored_prefix_of(&self.mode);
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.expected = Some(token);
                    // The comparison D10 makes is against the row as it is now, not as it was when
                    // the editor opened.
                    if editor.stored_prefix.is_some() {
                        editor.stored_prefix = stored;
                    }
                }
                self.say(&retry);
            }
        }
    }

    /// The prefix the edited kind carries in the current snapshot, for D10's comparison.
    fn stored_prefix_of(&self, mode: &Mode) -> Option<String> {
        let Mode::Editing(editor) = mode else {
            return None;
        };
        let EditorKind::EditKind(id) = editor.kind else {
            return None;
        };
        self.locate_kind(id).map(|(_, kind)| kind.prefix.clone())
    }

    /// One list row per tree row, in [`rows`](KindsSection::rows) order, plus the read-only
    /// detail of the selected phase as note rows (D15, B-11): the lines, the cursor's own line
    /// and the selection's last line, so the detail scrolls into view with its phase.
    ///
    /// The cursor's line is answered, not taken for the cursor's row index (RV2 L2): the list
    /// is handed the line it bands. Every row is one line, a blank one should a lookup miss,
    /// so a row never shifts the lines under it. `width` is the list's, the gutter included.
    fn list_rows(&self, width: u16, theme: &Theme) -> Listed {
        let item = |line: Line<'static>| list::Row {
            line,
            kind: RowKind::Item,
        };
        let mut lines = Vec::new();
        let mut at = None;
        let mut end = 0;
        for (index, row) in self.rows().into_iter().enumerate() {
            let selected = index == self.cursor;
            if selected {
                at = Some(lines.len());
            }
            let line = match row {
                // A project heads its kinds and graphs: the kit's group row, never folded.
                Row::Project { p } => self.project(p).map(|entry| list::Row {
                    line: Line::styled(
                        format!("{}  {}", entry.project.slug, entry.project.name),
                        theme.title,
                    ),
                    kind: RowKind::Group { folded: false },
                }),
                Row::Kind { p, k } => self.kind_line(p, k, theme).map(item),
                Row::Graph { p, g } => self.graph_at(p, g).map(|entry| {
                    item(Line::styled(
                        format!("  {} \u{b7} graph, no kind", entry.graph.name),
                        theme.dim,
                    ))
                }),
                Row::Phase { p, g, i } => self.phase_at(p, g, i).map(|phase| {
                    item(Line::styled(phase_line(phase, self.personas()), theme.base))
                }),
            };
            lines.push(line.unwrap_or_else(|| item(Line::default())));
            if selected
                && let Row::Phase { p, g, i } = row
                && let Some(phase) = self.phase_at(p, g, i)
            {
                lines.extend(detail_lines(phase, width.saturating_sub(1), theme));
            }
            if selected {
                end = lines.len().saturating_sub(1);
            }
        }
        Listed { lines, at, end }
    }

    /// A kind's line: its prefix and name, then its graph, or the missing graph in
    /// `theme.error`; `None` should the snapshot not hold it.
    fn kind_line(&self, p: usize, k: usize, theme: &Theme) -> Option<Line<'static>> {
        let project = self.project(p)?;
        let kind = project.kinds.get(k)?;
        let head = format!("  {}  {} \u{b7} ", kind.prefix, kind.name);
        // The tail is the one thing on this line the user has to act on, so it is
        // the one thing in `theme.error` (the band covers it on the cursor row).
        // MOD-138 D12: a workspace or library graph is named with its scope, and has no phase
        // rows under it (`rows` expands the project's own graphs only).
        let tail = match project.graph_name(kind.default_graph_id) {
            Some((name, scope)) => {
                Span::styled(format!("graph {}", graph_label(name, scope)), theme.base)
            }
            None => Span::styled(GRAPH_MISSING.to_owned(), theme.error),
        };
        Some(Line::from(vec![Span::styled(head, theme.base), tail]))
    }

    /// The one line under the pane: the last outcome as a kit notice, in `theme.error` when it is
    /// one to act on rather than read (a compare-and-set miss). The keys this mode binds are the
    /// Settings pane's hints ([`SettingsSection::key_hint`]), so the outcome has the line to
    /// itself; blank without one. Any mode: an editor's outcome is drawn here too.
    fn notice_line(&self, theme: &Theme) -> Line<'static> {
        let Some(notice) = &self.notice else {
            return Line::default();
        };
        let tone = if notice.is_error() {
            Tone::Error
        } else {
            Tone::Info
        };
        self::notice(notice.text(), tone, theme)
    }

    /// The mode's keys for the shell's hint row, plus what a write in flight adds to them.
    /// Rendered through the mode's own stack, so a rebound key shows its new chord.
    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match &self.mode {
            // MOD-141 D2: the editor's own row, its write in flight included.
            Mode::Graph { editor } => return editor.hint_text(keys),
            Mode::Editing(_) => HINT_EDITING,
            Mode::ConfirmPrefix { .. } => HINT_CONFIRM_PREFIX,
            Mode::Deleting { .. } => HINT_DELETING,
            Mode::Browse => {
                if self.unavailable.is_some() {
                    HINT_UNAVAILABLE
                } else if self
                    .snapshot
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.projects.is_empty())
                {
                    HINT_NO_WORKSPACE
                } else {
                    HINT_BROWSE
                }
            }
        };
        let keys = keys.hint(self.stack(), spec);
        match self.busy {
            // Only in Browse: an editor's own hint says what `Enter` is for, and a write in flight
            // is why `Enter` is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
            }
            _ => keys,
        }
    }

    /// The stack of the current mode (MOD-67 M3), as `key_stack` reports it. Each `on_*_key`
    /// handler names its own mode's constant, which is this one whenever it runs.
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::KINDS_BROWSE,
            Mode::Editing(_) => views::KINDS_EDITOR,
            Mode::ConfirmPrefix { .. } | Mode::Deleting { .. } => views::KINDS_CONFIRM,
            Mode::Graph { editor } => editor.stack(),
        }
    }

    /// One key in Browse, through `views::KINDS_BROWSE` (MOD-67 M3). A chord matches with its
    /// modifiers, so `ctrl-d` is not `d` (ANA-26 §2.6 defect 1), and `g` is this view's own
    /// `settings.kinds.graph`, never `list.top` (D2). The tab took `settings.*` before offering
    /// the key; a global act, or one this state declines, is the shell's (`Pass`).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::KINDS_BROWSE, chord) {
            match act {
                Act::New => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_new_child(row, ctx.keys());
                    }
                }
                Act::KindsNewGraph => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_new_graph(row);
                    }
                }
                Act::Edit => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_edit(row, ctx);
                    }
                }
                Act::KindsGraph => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_graph(row);
                    }
                }
                // MOD-141 D3: the graph editor on the row's graph.
                Act::KindsEditor => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_editor(row, false, ctx);
                    }
                }
                Act::Delete => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.begin_delete(row, ctx.keys());
                    }
                }
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                // Allowed whatever else is going on: re-reading is how a section that lost a reply
                // recovers, and a read cannot lose a write's reply — the staleness index is keyed
                // by request kind. What it *can* do is be mistaken for one, because a `Catalogue`
                // says nothing about which request it answers; that trade is argued where the
                // mistake is made, in `on_catalogue` (H-9).
                Act::Reload => ctx.request(StoreRequest::Catalogue(ctx.scope.clone())),
                // Only when there is something to clear: a section that swallowed every `Esc`
                // would take the one the shell uses to close an overlay over it.
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                // A global act, or one this state declines.
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// The pane under the rows: the open editor as a kit form, the question being answered, or
    /// nothing at all in Browse.
    ///
    /// Takes the width because both questions are sentences rather than rows and have to wrap
    /// inside the pane they are measured for — the layout needs the height first.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let warning = |text: &str| -> Vec<Line<'static>> {
            wrapped(text, usize::from(width).max(1))
                .into_iter()
                .map(|line| notice(&line, Tone::Error, theme))
                .collect()
        };
        match &self.mode {
            // The editor draws over the whole section (MOD-141 D2), so it has no pane here.
            Mode::Browse | Mode::Graph { .. } => Vec::new(),
            Mode::Editing(editor) => editor.form().lines(width, ctx.accent(), theme),
            // The question, and while the delete is out, what it is waiting for.
            Mode::Deleting {
                name,
                prefix,
                stage,
                ..
            } => {
                let answers = ctx.keys().hint(views::KINDS_CONFIRM, HINT_DELETING);
                let mut lines = warning(&delete_question(name, prefix, &answers));
                if matches!(stage, DeleteStage::InFlight) {
                    let waiting = format!("{}\u{2026}", doing(REQUEST_NAMES[3]));
                    lines.push(notice(&waiting, Tone::Info, theme));
                }
                lines
            }
            // The warning and nothing else: what is being answered is the sentence, and the fields
            // behind it are one `n` away.
            Mode::ConfirmPrefix { old, new, .. } => warning(&prefix_warning(old, new)),
        }
    }

    /// Reports an outcome, classified by the rule both sections share (D14).
    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    /// Reports a refusal: the seam's own sentence, or one of this section's.
    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// A fresh catalogue: the tree is replaced whole and the cursor put back inside it (D3).
    ///
    /// Only a reply to a **write** closes an open editor (H-9): `r` sets no `busy`, so a reload that
    /// lands while something is being typed leaves the typing alone.
    ///
    /// `busy` is the whole of the attribution, and a `Catalogue` carries nothing that says which
    /// request it answers — so a read that lands between a write and its reply is taken for that
    /// reply and closes the editor early. `r` is not that door today —
    /// `on_key` hands every key to the open editor, where `r` is a letter (`:1336`), and every write
    /// this section sends leaves `Browse` behind — but a scope change, a tab re-activation
    /// (`app/state.rs`'s `wants_requests`) or a future read on the Browse side all are. `r` stays
    /// allowed anyway: re-reading is how a section that lost a reply recovers, and refusing it while
    /// busy would wedge exactly the section that needs it. The loss is bounded and deliberate — the
    /// write itself has already been sent, so all that goes is the retry affordance if it comes back
    /// `CatalogueStale`, and [`CHANGED_ELSEWHERE_CLOSED`] says so when it does.
    fn on_catalogue(&mut self, snapshot: &CatalogueSnapshot) {
        let write = self.busy.take();
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // catalogue is unavailable over a store that just spoke.
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        if write.is_some() {
            self.notice = None;
            if matches!(self.mode, Mode::Editing(_)) {
                self.mode = Mode::Browse;
            }
        }
    }
}

impl SettingsSection for KindsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Kinds"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        let mut wanted = vec![StoreRequest::Catalogue(scope.clone())];
        // MOD-141 D1: an open editor reads its agents and templates again on activation.
        if let Mode::Graph { editor } = &self.mode {
            wanted.extend(editor.wants_requests(scope));
        }
        wanted
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The tree, the cursor and any open editor belong to the workspace that was left, and that
        // editor's compare-and-set token with them (B-12). The notice survives, because the scope
        // change is often the *consequence* of what it is reporting.
        self.snapshot = None;
        self.mode = Mode::Browse;
        self.busy = None;
        self.cursor = 0;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn takes_paste(&self) -> bool {
        match &self.mode {
            Mode::Graph { editor } => editor.takes_paste(),
            _ => self.captures_input(),
        }
    }

    /// MOD-22 review M-1: a bracketed paste into the open form's focused field. A delete question
    /// is not a field: a paste there is dropped, so its `y` confirms nothing.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        if let Mode::Graph { editor } = &mut self.mode {
            return editor.on_paste(text);
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        if let Some(field) = editor.fields.get_mut(editor.focus) {
            field.input.on_paste(text);
        }
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // MOD-141 D2: the graph editor takes every key while it is open; its close act returns to
        // Browse (the memory pane's shape, `hierarchy.rs`).
        if let Mode::Graph { editor } = &mut self.mode {
            return match editor.on_key(key, ctx) {
                EditorKey::Handled(handled) => handled,
                EditorKey::Close => {
                    self.close_editor();
                    Handled::Consumed
                }
            };
        }
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        if matches!(self.mode, Mode::ConfirmPrefix { .. }) {
            return self.on_confirm_key(key, ctx);
        }
        if matches!(self.mode, Mode::Deleting { .. }) {
            return self.on_deleting_key(key, ctx);
        }
        self.on_browse_key(key, ctx)
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        if matches!(self.mode, Mode::Graph { .. }) && self.on_editor_reply(reply, ctx) {
            return;
        }
        match reply {
            StoreReply::Catalogue(snapshot) => self.on_catalogue(snapshot),
            StoreReply::CatalogueStale(snapshot) => self.on_stale(snapshot, ctx.keys()),
            // MOD-141 D13: `ReplaceGraph`'s answer carries the catalogue; read it as `Catalogue` /
            // `CatalogueStale` were. T7 routes it to an open graph editor first.
            StoreReply::GraphReplaced {
                outcome,
                catalogue: Some(catalogue),
            } => match outcome {
                CasOutcome::Applied(_) => self.on_catalogue(catalogue),
                CasOutcome::Stale(_) => self.on_stale(catalogue, ctx.keys()),
            },
            // MOD-141 R1 L1: the re-read after the write failed; read it again.
            StoreReply::GraphReplaced {
                catalogue: None, ..
            } => ctx.request(StoreRequest::Catalogue(ctx.scope.clone())),
            StoreReply::KindDeleted { mirror, catalogue } => self.on_deleted(mirror, catalogue),
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            // `busy` is deliberately left alone: a read never sets it, so the only thing clearing
            // it here could do is orphan a write that is still in flight — and let a second
            // `Enter` re-send it.
            StoreReply::Failed { request, message } if *request == "catalogue" => {
                self.unavailable = Some(message.clone());
            }
            // MOD-141 M2 D6 (C7): the editor's read, after it closed; nothing of the section's.
            StoreReply::Failed { request, .. } if *request == "graph_editor_settings" => {}
            // Every other refusal of this section's own: the shell has already put
            // `{request}: {message}` on the status line, so all this owes is the sentence and a
            // state the next key can start from.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                // The editor stays open over its text: a refused write is retried by fixing what
                // was refused and pressing `Enter` again. A refused **delete** is not: the seam
                // answered the question the confirmation asked, so leaving `deleting the kind…`
                // on screen would be waiting for something that already came back.
                if matches!(self.mode, Mode::Deleting { .. }) {
                    self.mode = Mode::Browse;
                }
                self.refuse(message.clone());
            }
            _ => {}
        }
    }

    /// MOD-141 D2: a refused graph write is said on the open editor's notice row, so the status
    /// line does not repeat it.
    fn absorbs_failure(&self, request: &str) -> bool {
        matches!(self.mode, Mode::Graph { .. }) && request == "replace_graph"
    }

    /// MOD-141 D11: the open graph editor's canvas takes the mouse; Browse and the forms do not.
    fn wants_mouse(&self) -> bool {
        matches!(self.mode, Mode::Graph { .. })
    }

    fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::Graph { editor } => editor.on_mouse(mouse, ctx),
            _ => Handled::Pass,
        }
    }

    fn on_mouse_lost(&mut self) {
        if let Mode::Graph { editor } = &mut self.mode {
            editor.on_mouse_lost();
        }
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        if matches!(self.mode, Mode::Graph { .. }) {
            return None;
        }
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // MOD-141 D2: the graph editor draws over the whole section.
        if let Mode::Graph { editor } = &self.mode {
            self.scroll.set(None);
            editor.render(frame, area, ctx);
            return;
        }
        let theme = ctx.theme;
        let pane = self.pane(area.width, ctx);
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        self.scroll.set(None);
        let reload = ctx.keys().label(views::KINDS_BROWSE, Act::Reload);
        match (&self.unavailable, &self.snapshot) {
            // The refusal wins the pane even with a tree behind it: what is on screen would
            // otherwise be a catalogue nothing has confirmed since the outage started (H-17).
            (Some(why), _) => {
                let mut lines: Vec<Line<'static>> = wrapped(
                    &format!("{UNAVAILABLE}: {why}"),
                    usize::from(rows.width).max(1),
                )
                .into_iter()
                .map(|line| notice(&line, Tone::Error, theme))
                .collect();
                // The next step in the empty state's own role.
                let next = asks_again(reload.as_deref());
                lines.extend(empty::lines("", NextStep(&next), theme).pop());
                frame.render_widget(Paragraph::new(lines), rows);
            }
            (None, None) => empty::render(
                frame,
                rows,
                NO_WORKSPACE,
                NextStep(&reads_again(reload.as_deref())),
                theme,
            ),
            (None, Some(snapshot)) if snapshot.projects.is_empty() => {
                empty::render(frame, rows, NO_PROJECT, NextStep(NO_PROJECT_NEXT), theme);
            }
            (None, Some(_)) => {
                // Scrolled so the cursor is on screen, its detail with it when it fits: the tree
                // of a scope with several projects is longer than the pane, and a selection
                // nobody can see is a cursor that acts blind.
                let Listed { lines, at, end } = self.list_rows(rows.width, theme);
                let total = lines.len();
                let offset = list::scroll_into_view(end, 0, usize::from(rows.height), total)
                    .min(at.unwrap_or(0));
                list::render(frame, rows, &lines, at, offset, ctx.accent(), theme);
                // The pane's bar is measured against the rows the tree is drawn into and sits
                // beside them (RV1 M2), so the rows under the tree need no padding.
                self.scroll
                    .set(Some(ScrollSpan::new(total, offset).rows(rows)));
            }
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(Paragraph::new(self.notice_line(theme)), hint);
    }
}

/// The tree as [`KindsSection::list_rows`] lists it (RV2 L2).
struct Listed {
    /// One line per tree row, the selected phase's detail under it.
    lines: Vec<list::Row<'static>>,
    /// The cursor's line, the one the list bands; `None` with no row under the cursor.
    at: Option<usize>,
    /// The selection's last line, its detail's when it has one.
    end: usize,
}

impl Field {
    /// A field `Enter` refuses while it is empty.
    fn required(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
            required: true,
        }
    }

    /// A field that may stay empty: an empty `description` is `""` and an empty `input_kinds` is
    /// the empty list.
    fn optional(label: &'static str, text: &str) -> Self {
        Self {
            label,
            input: TextField::with_text(text),
            required: false,
        }
    }

    /// What was typed. Never masked here, so [`TextField::text`] always answers.
    fn text(&self) -> &str {
        self.input.text().unwrap_or_default()
    }
}

impl Editor {
    /// The text of field `index`, or `""` when the editor has no such field.
    fn text(&self, index: usize) -> String {
        self.fields
            .get(index)
            .map_or_else(String::new, |field| field.text().to_owned())
    }

    /// The fields as a kit form (MOD-115 M3 D3): one row per field in tab order, the focused one
    /// carrying the marker and the cursor, the labels in one shared column.
    fn form(&self) -> Form<'_> {
        self.fields
            .iter()
            .fold(Form::new(), |form, field| {
                form.field(form::Field::new(field.label, Value::Text(&field.input)))
            })
            .focus(self.focus)
            .editing(true)
    }
}

/// The fields of a new kind: the first graph of the project by name, and the position after its
/// last kind (B-7, B-8).
///
/// The position prefill is a courtesy and not a guard: `(project_id, position)` is not unique on
/// `item_kind` (H-11), so two kinds may share one — it is simply the one value that cannot
/// surprise.
fn new_kind_fields(project: &ProjectCatalogue) -> Vec<Field> {
    let graph = project
        .graphs
        .first()
        .map_or("", |entry| entry.graph.name.as_str());
    vec![
        Field::required("prefix", ""),
        Field::required("name", ""),
        Field::optional("description", ""),
        Field::required("graph", graph),
        Field::required(
            "position",
            &next_position(project.kinds.iter().map(|k| k.position)),
        ),
    ]
}

/// The same five fields, prefilled from the row (B-5's whole patch goes out from them). The
/// graph reads as its [`graph_label`] (MOD-138 D12), so a workspace or library graph keeps its
/// scope on screen and [`graph_named`] does not mistake it for a project graph of that name.
fn edit_kind_fields(project: &ProjectCatalogue, kind: &ItemKind) -> Vec<Field> {
    let graph = project
        .graph_name(kind.default_graph_id)
        .map_or_else(String::new, |(name, scope)| graph_label(name, scope));
    vec![
        Field::required("prefix", &kind.prefix),
        Field::required("name", &kind.name),
        Field::optional("description", &kind.description),
        Field::required("graph", &graph),
        Field::required("position", &kind.position.to_string()),
    ]
}

/// A graph's two columns, prefilled from the row — the fields `e` on a graph row and `g` on any
/// row both open (D19).
///
/// Takes the entry rather than a cloned row: two `&str` are read out of it, and the editor's own
/// fields own their text from there.
fn edit_graph_fields(entry: &GraphEntry) -> Vec<Field> {
    vec![
        Field::required("name", &entry.graph.name),
        Field::optional("description", &entry.graph.description),
    ]
}

/// One past the largest position, or `0` when there is none.
fn next_position(positions: impl Iterator<Item = i32>) -> String {
    positions
        .max()
        .map_or(0, |last| last.saturating_add(1))
        .to_string()
}

/// The graph the typed text names (B-7). A graph name is unique within one scope only (MOD-138
/// D12), so the text is matched as a [`graph_label`]: the project's own graph by its bare name
/// first, then a workspace or library graph the project may use as `name · workspace` /
/// `name · library`, in [`ProjectCatalogue::shared`]'s order; a bare name no project graph has
/// still finds a shared graph of that name. `current` is the edited kind's graph: text that still
/// reads as its label keeps it, so an untouched field never moves the kind, not even between two
/// workspaces' graphs that share a name and so a label.
///
/// A name rather than an id because that is what is on screen; the store checks the rule again
/// on the write, so this resolution is a convenience and not the guard.
fn graph_named(
    project: &ProjectCatalogue,
    text: &str,
    current: Option<StepGraphId>,
) -> Result<StepGraphId, String> {
    if let Some(id) = current.filter(|&id| {
        project
            .graph_name(id)
            .is_some_and(|(name, scope)| graph_label(name, scope) == text)
    }) {
        return Ok(id);
    }
    project
        .graphs
        .iter()
        .map(|entry| &entry.graph)
        .chain(&project.shared)
        .find(|graph| graph_label(&graph.name, graph.scope) == text)
        .or_else(|| project.shared.iter().find(|graph| graph.name == text))
        .map(|graph| graph.id)
        .ok_or_else(|| NO_GRAPH_NAMED.to_owned())
}

/// A bound persona's name, or its id when the loaded list does not know it (a persona created
/// after the catalogue was read; D23).
fn persona_label(id: PersonaId, personas: &[PersonaSummary]) -> String {
    personas
        .iter()
        .find(|row| row.id == id)
        .map_or_else(|| id.to_string(), |row| row.name.clone())
}

/// A `position` field as a number (B-8).
fn position(text: &str) -> Result<i32, String> {
    text.trim()
        .parse::<i32>()
        .map_err(|_| POSITION_IS_A_NUMBER.to_owned())
}

/// What a reloaded catalogue does to an open editor's token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The editor's row is not in the reloaded tree.
    Gone,
    /// The row's `updated_at` as it is now.
    Token(DateTime<Utc>),
    /// The editor has no token to refresh (a create).
    Keep,
}

/// The token an open editor should retry against after a reload.
fn reload(snapshot: &CatalogueSnapshot, kind: EditorKind) -> Reload {
    match kind {
        EditorKind::NewKind(project) | EditorKind::NewGraph(project) => snapshot
            .projects
            .iter()
            .find(|entry| entry.project.id == project)
            .map_or(Reload::Gone, |_| Reload::Keep),
        EditorKind::EditKind(id) => snapshot
            .projects
            .iter()
            .flat_map(|project| project.kinds.iter())
            .find(|kind| kind.id == id)
            .map_or(Reload::Gone, |kind| Reload::Token(kind.updated_at)),
        EditorKind::EditGraph(id) => snapshot
            .projects
            .iter()
            .flat_map(|project| project.graphs.iter())
            .find(|entry| entry.graph.id == id)
            .map_or(Reload::Gone, |entry| Reload::Token(entry.graph.updated_at)),
    }
}

/// PRD D12's sentence, verbatim: what a prefix change does and what it leaves alone.
///
/// The old keys keep their text, the old counter row survives, and the new prefix mints from 1 —
/// three facts a user cannot check afterwards, so they are put in front of the write rather than
/// reported after it (D10).
fn prefix_warning(old: &str, new: &str) -> String {
    format!(
        "items keyed {old}-* keep their keys and their counter; the next item minted under this \
         kind is {new}-1."
    )
}

/// D11's question, verbatim: what is being deleted, the one refusal the store may answer with,
/// then `answers`, the keys that answer it as the hint row renders them (MOD-67 M3, blueprint L-C
/// Q6), so the question cannot name a key a rebinding moved.
fn delete_question(name: &str, prefix: &str, answers: &str) -> String {
    format!("delete kind {name} ({prefix})? a kind any item uses is refused. {answers}")
}

/// What a delete that happened reports, mirror included (D12).
fn deleted_notice(prefix: Option<&str>, mirror: &MirrorAfterDelete) -> String {
    let mirror = match mirror {
        MirrorAfterDelete::Rebuilt => "mirror rebuilt".to_owned(),
        MirrorAfterDelete::NoMirror => "no mirror".to_owned(),
        MirrorAfterDelete::Failed(err) => format!("mirror not rebuilt: {err}"),
    };
    match prefix {
        Some(prefix) => format!("deleted kind `{prefix}`; {mirror}"),
        None => format!("deleted kind; {mirror}"),
    }
}

/// What a second write of the same kind is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// A write in flight in words (MOD-115 M3: no request name on screen), by
/// [`StoreRequest::name`].
fn doing(busy: &str) -> &'static str {
    match busy {
        "catalogue" => "reading the catalogue",
        "create_kind" => "adding the kind",
        "update_kind" => "saving the kind",
        "delete_kind" => "deleting the kind",
        "create_graph" => "adding the graph",
        "update_graph" => "saving the graph",
        "create_phase" => "adding the phase",
        "update_phase" => "saving the phase",
        "set_phase_budget" => "saving the budget",
        "replace_graph" => "replacing the graph",
        "delete_phase" => "deleting the phase",
        "graph_body" => "reading the graph",
        "graph_editor_settings" => "reading the run limits",
        _ => "writing",
    }
}

/// What an empty required field is told, before the store is asked anything.
fn required(label: &str) -> String {
    format!("`{label}` is required")
}

/// One phase row, read-only (MOD-141 D14: the graph editor edits it): D2's six columns, minus the
/// ones the row cannot show, read as words (MOD-83: `soft gate`, `no inputs`, `budget inherited`),
/// then the bound persona's name — only when one is bound, so an unbound phase's line says
/// nothing of personas (D23, I-12).
fn phase_line(phase: &StepGraphPhase, personas: &[PersonaSummary]) -> String {
    let inputs = if phase.input_kinds.is_empty() {
        "no inputs".to_owned()
    } else {
        format!("inputs {}", phase.input_kinds.join(", "))
    };
    let mut line = format!(
        "    {}. {} \u{b7} template {} \u{b7} {} \u{b7} {inputs} \u{b7} {}",
        phase.position,
        phase.name,
        phase.template_name,
        if phase.gate_hard {
            "hard gate"
        } else {
            "soft gate"
        },
        budget_label(phase.token_budget),
    );
    if let Some(id) = phase.persona_id {
        line.push_str(" \u{b7} persona ");
        line.push_str(&persona_label(id, personas));
    }
    line
}

/// `token_budget` as the tree prints it: an empty column inherits from the project or the app rung
/// rather than holding a number nobody wrote (D16).
///
/// The inherited case reads `budget inherited`, not blueprint §5.4's `inherits the budget`: the
/// same meaning three cells shorter, so a bound research row (`kinds__phase_persona`: `... ·
/// budget inherited · persona reviewer`, 98 cells) still fits a 100-column pane; the blueprint's
/// wording takes it to 101 and clips the persona's name. Keeping `budget` first also lines it up with `budget {n}
/// tokens`. Pinned by `a_phase_row_reads_its_gate_inputs_and_budget_in_words`.
fn budget_label(budget: Option<i32>) -> String {
    budget.map_or_else(
        || "budget inherited".to_owned(),
        |n| format!("budget {n} tokens"),
    )
}

/// MOD-4's eight columns under the selected phase, dimmed and unselectable (D15), each named in
/// words and its enums through `kit::label` (MOD-83): `fan-out 1 · gate: never · isolation: none
/// (local tree) · commands: not queued · …`.
///
/// Wrapped here rather than clipped: a detail line cut off at the verify command would read as a
/// row of the tree that someone forgot to finish. No owner tail any more (MOD-141 D14): the step's
/// own form in the graph editor is where its columns are seen whole. `width` is the row's, after
/// the gutter.
fn detail_lines(phase: &StepGraphPhase, width: u16, theme: &Theme) -> Vec<list::Row<'static>> {
    let text = format!(
        "fan-out {} \u{b7} gate: {} \u{b7} isolation: {} \u{b7} commands: {} \u{b7} verify: {} \u{b7} retries {} \u{b7} output: {} \u{b7} template: {}",
        phase.fan_out,
        label::gate(phase.gate),
        // `None` is the project's default (`step_graph_phase.isolation`).
        phase.isolation.map_or("project default", label::isolation),
        label::command_queue(phase.command_queue),
        phase.verify_command.as_deref().unwrap_or("none"),
        phase.retry_limit,
        phase.output_kind,
        // `None` follows the latest version.
        phase
            .template_version
            .map_or_else(|| "latest".to_owned(), |n| format!("v{n}")),
    );
    let room = usize::from(width).saturating_sub(6).max(1);
    wrapped(&text, room)
        .into_iter()
        .map(|line| list::Row {
            line: Line::styled(format!("      {line}"), theme.dim),
            kind: RowKind::Note,
        })
        .collect()
}

/// What `n` and `d` say about steps (MOD-141 D14): they are added and deleted in the graph
/// editor, and the sentence names the key that opens it in force (MD-A).
fn steps_in_the_editor(keys: &Keys) -> String {
    let key = keys.label(views::KINDS_BROWSE, Act::KindsEditor);
    key.map_or_else(
        || "steps are added and deleted in the graph editor".to_owned(),
        |key| format!("steps are added and deleted in the graph editor: {key} opens it"),
    )
}

/// What the graph editor's key says on a project row (MOD-141 D3): a project has no graph of its
/// own, so the sentence names the rows that do and the key in force (MD-A).
fn pick_a_graph(key: Option<&str>) -> String {
    key.map_or_else(
        || "select a graph or a step to open the graph editor".to_owned(),
        |key| format!("select a graph or a step: {key} opens the graph editor"),
    )
}

/// The next step of [`NO_WORKSPACE`]: the reload key in force (MD-A), or what happens without one.
fn reads_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "the catalogue is read again when the section opens".to_owned(),
        |key| format!("{key} reads the catalogue again"),
    )
}

/// The next step of [`UNAVAILABLE`]: the reload key in force (MD-A), or what happens without one.
fn asks_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "it is asked again when the section opens".to_owned(),
        |key| format!("{key} asks again once the store answers"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H-7 reaches the *mode* as well as the editor: [`KindsSection`] derives `Debug` through
    /// [`Mode`], so a variant that prints a field's own text puts it in a log the moment one
    /// `tracing::debug!` names the section. The prefix is only the first such field — milestone 6's
    /// masked column is the one that makes the rule load-bearing.
    #[test]
    fn a_confirm_prefix_never_prints_the_typed_prefix() {
        let editor = Editor {
            kind: EditorKind::EditKind(ItemKindId::default()),
            fields: vec![Field::required("prefix", "ZZTOP")],
            focus: 0,
            expected: None,
            stored_prefix: Some("ANA".to_owned()),
        };
        let section = KindsSection {
            mode: Mode::ConfirmPrefix {
                editor,
                old: "ANA".to_owned(),
                new: "ZZTOP".to_owned(),
            },
            ..KindsSection::new()
        };

        let printed = format!("{section:?}");

        assert!(
            !printed.contains("ZZTOP"),
            "what was typed stays out of the line: {printed}"
        );
        assert!(
            printed.contains("ConfirmPrefix") && printed.contains("ANA"),
            "the variant and the stored prefix are still legible: {printed}"
        );
    }

    /// MOD-67 M6 T2: the row under the pane carries the notice alone, and the keys are the
    /// section's [`SettingsSection::key_hint`], the shell's row.
    #[test]
    fn the_notice_row_carries_the_notice_alone() {
        let notice = "\u{6f22}".repeat(10);
        let section = KindsSection {
            notice: Some(Notice::Error(notice.clone())),
            ..KindsSection::new()
        };
        let theme = Theme::default();
        let line = section.notice_line(&theme);
        assert_eq!(line.spans.len(), 1, "{line:?}");
        assert_eq!(line.spans[0].content, notice);
        assert_eq!(line.style, theme.error);
        let keys = section
            .key_hint(Keys::compiled(), &theme)
            .expect("the defaults bind the keys");
        assert_eq!(keys.to_string(), section.hint_text(Keys::compiled()));
    }

    /// MOD-115 M3: a write in flight reads in words. Every request name has its own phrase, and
    /// neither the refusal nor the Browse hint prints the name itself.
    #[test]
    fn a_write_in_flight_reads_in_words() {
        for name in REQUEST_NAMES {
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
        let section = KindsSection {
            busy: Some(REQUEST_NAMES[8]),
            ..KindsSection::new()
        };
        let hint = section.hint_text(Keys::compiled());
        assert!(
            hint.ends_with(" \u{b7} saving the budget\u{2026}"),
            "{hint}"
        );
    }

    /// MOD-141 M2 D6 (C7): the editor's settings read is the editor's own. While the editor is
    /// open its answer and its refusal go to the editor; a refusal that lands after it closed is
    /// nobody's, never a refusal of the section's, though its name is in `REQUEST_NAMES`.
    #[tokio::test]
    async fn the_editors_settings_read_is_never_the_sections() {
        use htui_core::fixtures::ids;
        use htui_core::store::{MemStore, WriteStore};

        use crate::app::{Emit, TopBarState};
        use crate::store_worker::Origin;
        use crate::ui::tabs::settings::SettingsTab;

        let store = MemStore::demo();
        let workspaces = store
            .workspaces()
            .await
            .expect("the memory store never fails");
        let scope = Scope::from_workspace(workspaces.first().expect("a workspace"));
        let body = store
            .graph_body(ids::GRAPH_VULKAN_ANA)
            .await
            .expect("a memory read")
            .expect("the analysis graph");
        let (projects, top_bar, theme, emit) = (
            Vec::new(),
            TopBarState::default(),
            Theme::default(),
            Emit::default(),
        );
        let keys = Keys::compiled().clone();
        let mut ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            &keys,
            &theme,
            Origin::Tab(SettingsTab::ID),
            &emit,
        );
        let (editor, _) = GraphEditor::open(body, Vec::new(), &scope);
        let mut section = KindsSection {
            mode: Mode::Graph {
                editor: Box::new(editor),
            },
            ..KindsSection::new()
        };
        let answer = StoreReply::GraphEditorSettings(Box::new(crate::catalogue::EditorSettings {
            bounds: htui_orch::graph::run_bounds(&std::collections::BTreeMap::new()),
            compete: false,
        }));
        let refused = StoreReply::Failed {
            request: "graph_editor_settings",
            message: "refused".to_owned(),
        };
        assert!(
            section.on_editor_reply(&answer, &mut ctx),
            "the answer is the editor's"
        );
        assert!(
            section.on_editor_reply(&refused, &mut ctx),
            "so is its refusal"
        );
        // Review R1 C-L4: the refusal reached the editor, which says so in its own words.
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(98, 27))
            .expect("a test terminal");
        terminal
            .draw(|frame| section.render(frame, frame.area(), &ctx))
            .expect("the section draws");
        let buffer = terminal.backend().buffer();
        let screen: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    + "\n"
            })
            .collect();
        assert!(
            screen.contains("run limits unavailable: refused"),
            "{screen}"
        );

        section.mode = Mode::Browse;
        section.busy = Some(REQUEST_NAMES[8]);
        section.on_reply(&refused, &mut ctx);
        assert_eq!(
            section.busy,
            Some(REQUEST_NAMES[8]),
            "the section's write stays"
        );
        assert!(section.notice.is_none(), "{:?}", section.notice);
    }

    /// MOD-115 M3 RV2 L2: the list is handed the cursor's own line, which `list_rows` answers,
    /// not the cursor's row index: over the whole demo tree the line it names is the cursor
    /// row's, and the selection's last line closes the detail drawn under it.
    #[tokio::test]
    async fn list_rows_answer_the_cursors_line() {
        use htui_core::store::MemStore;
        let store = MemStore::demo();
        let workspaces = store
            .workspaces()
            .await
            .expect("the memory store never fails");
        let scope = Scope::from_workspace(workspaces.first().expect("a workspace"));
        let snapshot = crate::catalogue::snapshot(&store, &scope)
            .await
            .expect("the demo catalogue");
        let theme = Theme::default();
        let mut section = KindsSection {
            snapshot: Some(snapshot),
            ..KindsSection::new()
        };
        let rows = section.rows().len();
        assert!(rows > 3, "a tree to walk");
        let mut details = 0;
        for cursor in 0..rows {
            section.cursor = cursor;
            let listed = section.list_rows(100, &theme);
            let at = listed.at.expect("the cursor's row is listed");
            assert!(
                matches!(listed.lines[at].kind, RowKind::Item | RowKind::Group { .. }),
                "{cursor}: the cursor's line is a row, not a note"
            );
            assert!(listed.end >= at, "{cursor}");
            assert!(
                listed.lines[at + 1..=listed.end]
                    .iter()
                    .all(|line| matches!(line.kind, RowKind::Note)),
                "{cursor}: the lines after it, through `end`, are its detail"
            );
            details += listed.end - at;
        }
        assert!(details > 0, "a phase with its detail was walked");
    }
}
