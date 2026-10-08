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
//! A phase's persona (MOD-26 milestone 2, D23) is picked the way a kind's graph is: by typing its
//! name into the phase editor's seventh field, resolved against the persona list the catalogue
//! carries and refused before anything is sent when the list does not hold it.
//!
//! Every mode resolves its keys through its own stack in [`views`] (MOD-67 M3), so a chord
//! matches with its modifiers (`ctrl-d` is not `d`) and a rebound key acts under its new chord;
//! the hint row, and the delete question's own keys, are rendered from the same stack. A
//! capturing or confirming mode swallows what it does not bind, except the chords its modal global
//! layer admits (`CONTROL`, `ALT`, a function key), so `ctrl-c` quits and `F1` opens help.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use chrono::{DateTime, Utc};
use htui_core::model::{
    ItemKind, ItemKindId, ItemKindPatch, PersonaId, PhaseId, PhasePatch, ProjectId, Scope,
    StepGraphId, StepGraphPatch, StepGraphPhase,
};

use crate::app::{Ctx, Handled};
use crate::catalogue::{
    CatalogueSnapshot, GraphEntry, PersonaSummary, ProjectCatalogue, REQUEST_NAMES,
};
use crate::hierarchy::MirrorAfterDelete;
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    message, wrapped,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

/// What the rows pane says before any catalogue has arrived.
const NO_WORKSPACE: &str = "no workspace: nothing to list";

/// What the rows pane says when the read answered with no project at all.
const NO_PROJECT: &str = "no project in scope";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "catalogue unavailable";

/// What a kind whose `default_graph_id` names no graph of its project says (D4).
const GRAPH_MISSING: &str = "graph missing";

/// Browse's keys.
const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::New, "kind/phase"),
    Hint::One(Act::KindsNewGraph, "graph"),
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::KindsGraph, "graph"),
    Hint::One(Act::Delete, "delete kind"),
    Hint::One(Act::Reload, "reload"),
];

/// Browse's keys with nothing to list: the only offer is to ask again.
const HINT_NO_WORKSPACE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// Browse's keys with the read refused: nothing here can be written against a store that did not
/// answer.
const HINT_UNAVAILABLE: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// An open editor's keys: the form's own, then the field's fixed `Enter` and `Esc` (D13).
const HINT_EDITING: HintSpec = &[
    Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
    Hint::Text("Enter save"),
    Hint::Text("Esc cancel"),
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
const NO_GRAPH_NAMED: &str = "`graph` names no graph in this project";

/// What an editor says when a `position` field is not a number (B-8).
const POSITION_IS_A_NUMBER: &str = "`position` is a whole number";

/// What the phase editor says when its budget is neither empty nor a number (B-1).
///
/// The only bound this section checks: every other one — the range, the column's width — is the
/// store's, and it answers with its own sentence (M1 D7).
const BUDGET_IS_A_NUMBER: &str = "`token_budget` is a whole number or empty";

/// What the phase editor says when `gate_hard` is neither `y` nor `n` (B-10).
const GATE_IS_Y_OR_N: &str = "`gate_hard (y/n)` is y or n";

/// The delete confirmation's keys (D11), on the hint row and at the end of the question.
const HINT_DELETING: HintSpec = &[
    Hint::One(Act::ConfirmYes, "delete"),
    Hint::All(Act::ConfirmNo, "stop"),
];

/// What `d` says on a graph or a phase row: the seam has `delete_item_kind` and nothing else in
/// this area, and this milestone adds no method to it (D5).
const NOT_DELETED_HERE: &str = "graphs and phases are not deleted here";

/// What the `persona` field says when the registry is empty (MOD-26 M2 D23).
const NO_PERSONAS: &str = "no personas exist; add one in Settings \u{203a} Personas";

/// The tail of the read-only detail line (D15): the columns are MOD-4's to give semantics to, so
/// they are shown and not edited.
const MOD_4_OWNS: &str = "MOD-4 owns these";

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
    /// `projects[p].graphs[g].phases[i]`, whichever parent it is drawn under.
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
    /// `n` on a graph or phase row.
    NewPhase(StepGraphId),
    /// `e` on a phase row.
    EditPhase(PhaseId),
}

/// The second write of a phase edit that changed both the patch columns and the budget (B-4).
///
/// Held on the editor until the first write's reply carries the token the second one needs: two
/// requests sent on one tick are both delivered, but the second's compare-and-set token would be
/// stale by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowUp {
    /// `set_phase_budget`, with what the field said.
    Budget(Option<i64>),
}

/// What an editor remembers of the row it opened on, beyond the text in its fields.
///
/// Three values, because three comparisons are made at submit: the prefix D10 warns about, the
/// budget B-4 decides the second write from, and the persona MOD-26 M2 D23 writes only when it was
/// edited (B-8). Everything else goes out whole (B-5).
#[derive(Debug, Default)]
struct Stored {
    /// The kind's prefix as it is stored.
    prefix: Option<String>,
    /// The phase's budget as the field renders it (`""` for inherit).
    budget: Option<String>,
    /// The phase editor's `persona` text at open (`""` for none).
    persona: Option<String>,
}

/// One `Enter`'s outcome: the request that goes out now, and the one it owes afterwards (B-4).
type Built = (StoreRequest, Option<FollowUp>);

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
    /// A phase edit's stored budget as text, for B-4's comparison; `None` for every other editor.
    stored_budget: Option<String>,
    /// A phase edit's `persona` text at open, for D23's comparison; `None` for every other editor.
    ///
    /// Never refreshed, `stored_budget`'s rule (B-8): a `CatalogueStale` leaves it holding what this
    /// user found in the field, so only a user who typed in it rewrites another writer's binding.
    stored_persona: Option<String>,
    /// The write this editor still owes after the one in flight (B-4).
    follow_up: Option<FollowUp>,
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
    /// The last outcome, one line on the hint row.
    notice: Option<Notice>,
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
                    for i in 0..project.graphs[g].phases.len() {
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
                for i in 0..entry.phases.len() {
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
        self.graph_at(p, g)?.phases.get(i)
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
        self.open_with(kind, fields, expected, Stored::default());
    }

    /// The same, for the two editors that carry a stored value to compare what was typed against:
    /// a kind edit's prefix (D10) and a phase edit's budget (B-4).
    fn open_with(
        &mut self,
        kind: EditorKind,
        fields: Vec<Field>,
        expected: Option<DateTime<Utc>>,
        stored: Stored,
    ) {
        let Stored {
            prefix: stored_prefix,
            budget: stored_budget,
            persona: stored_persona,
        } = stored;
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            kind,
            fields,
            focus: 0,
            expected,
            stored_prefix,
            stored_budget,
            stored_persona,
            follow_up: None,
        });
    }

    /// `n`: a kind under a project or a kind row (B-13).
    fn open_new_child(&mut self, row: Row) {
        if let Row::Project { p } | Row::Kind { p, .. } = row
            && let Some(project) = self.project(p)
        {
            let id = project.project.id;
            let fields = new_kind_fields(project);
            self.open(EditorKind::NewKind(id), fields, None);
            return;
        }
        // On a graph or a phase row, `n` is a phase of that graph.
        if let Row::Graph { p, g } | Row::Phase { p, g, .. } = row
            && let Some(entry) = self.graph_at(p, g)
        {
            let id = entry.graph.id;
            let fields = new_phase_fields(entry);
            self.open(EditorKind::NewPhase(id), fields, None);
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

    /// `e`: the row under the cursor, prefilled, with its `updated_at` as the token.
    fn open_edit(&mut self, row: Row) {
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
                    Stored {
                        prefix: Some(prefix),
                        budget: None,
                        persona: None,
                    },
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
            Row::Phase { p, g, i } => {
                let Some(phase) = self.phase_at(p, g, i) else {
                    return;
                };
                let (id, expected) = (phase.id, phase.updated_at);
                let budget = budget_text(phase.token_budget);
                let persona = persona_text(phase.persona_id, self.personas());
                let fields = edit_phase_fields(phase, self.personas());
                self.open_with(
                    EditorKind::EditPhase(id),
                    fields,
                    Some(expected),
                    Stored {
                        prefix: None,
                        budget: Some(budget),
                        persona: Some(persona),
                    },
                );
            }
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
                    self.refuse(GRAPH_MISSING.to_owned());
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

    /// `d`: the one destructive key this section has (D11).
    ///
    /// One confirmation rather than PRD D13's two: a kind any item holds cannot be deleted at all
    /// (D6), so the destructive case the typed slug exists for does not arise — and two
    /// confirmations over an act the database refuses would train the reflex D13 exists to defeat.
    fn begin_delete(&mut self, row: Row) {
        match row {
            Row::Project { .. } => self.refuse(PROJECTS_ELSEWHERE.to_owned()),
            Row::Graph { .. } | Row::Phase { .. } => self.refuse(NOT_DELETED_HERE.to_owned()),
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
        let chord = KeyChord::from_event(key);
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
            _ => unused(stack, chord),
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
            Mode::Browse | Mode::ConfirmPrefix { .. } | Mode::Deleting { .. } => {
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
                let chord = KeyChord::from_event(key);
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
                    _ => unused(stack, chord),
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
            Ok((request, follow_up)) => {
                if let Mode::Editing(editor) = &mut self.mode {
                    editor.follow_up = follow_up;
                }
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
        let chord = KeyChord::from_event(key);
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
                    Ok((request, follow_up)) => {
                        if let Mode::Editing(editor) = &mut self.mode {
                            editor.follow_up = follow_up;
                        }
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
            _ => unused(stack, chord),
        }
    }

    /// The request one editor stands for, or the sentence that says why there is none.
    ///
    /// Every editable column goes out on an edit whether or not it changed (B-5): one habit across
    /// both sections, and the compare-and-set token is what makes it safe.
    fn build(&self, editor: &Editor, scope: &Scope) -> Result<Built, String> {
        match editor.kind {
            EditorKind::NewKind(project) => {
                let owner = self
                    .project_by_id(project)
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok((
                    StoreRequest::CreateKind {
                        scope: scope.clone(),
                        project,
                        prefix: editor.text(0),
                        name: editor.text(1),
                        description: editor.text(2),
                        graph: graph_named(owner, &editor.text(3))?,
                        position: position(&editor.text(4))?,
                    },
                    None,
                ))
            }
            EditorKind::EditKind(id) => {
                let expected = editor
                    .expected
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                let (owner, _) = self
                    .locate_kind(id)
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok((
                    StoreRequest::UpdateKind {
                        scope: scope.clone(),
                        id,
                        expected,
                        patch: ItemKindPatch {
                            prefix: Some(editor.text(0)),
                            name: Some(editor.text(1)),
                            description: Some(editor.text(2)),
                            default_graph_id: Some(graph_named(owner, &editor.text(3))?),
                            position: Some(position(&editor.text(4))?),
                        },
                    },
                    None,
                ))
            }
            EditorKind::NewGraph(project) => Ok((
                StoreRequest::CreateGraph {
                    scope: scope.clone(),
                    project,
                    name: editor.text(0),
                    description: editor.text(1),
                },
                None,
            )),
            EditorKind::EditGraph(id) => {
                let expected = editor
                    .expected
                    .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
                Ok((
                    StoreRequest::UpdateGraph {
                        scope: scope.clone(),
                        id,
                        expected,
                        patch: StepGraphPatch {
                            name: Some(editor.text(0)),
                            description: Some(editor.text(1)),
                        },
                    },
                    None,
                ))
            }
            EditorKind::NewPhase(graph) => Ok((
                StoreRequest::CreatePhase {
                    scope: scope.clone(),
                    graph,
                    name: editor.text(0),
                    position: position(&editor.text(1))?,
                    template_name: editor.text(2),
                    gate_hard: super::yes_or_no(&editor.text(3))
                        .ok_or_else(|| GATE_IS_Y_OR_N.to_owned())?,
                    input_kinds: input_kinds(&editor.text(4)),
                },
                None,
            )),
            EditorKind::EditPhase(id) => self.build_phase_edit(editor, scope, id),
        }
    }

    /// The one editor that can owe two writes (B-4).
    ///
    /// `token_budget` is not a `PhasePatch` column at all — it rides the `Phase` rung (D6, F-14) —
    /// so an `Enter` that moved both sends `update_phase` now and keeps `set_phase_budget` for the
    /// reply that carries the row's new token. An `Enter` that moved only one sends only that one.
    fn build_phase_edit(
        &self,
        editor: &Editor,
        scope: &Scope,
        id: PhaseId,
    ) -> Result<Built, String> {
        let expected = editor
            .expected
            .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;
        let stored = self
            .phase_by_id(id)
            .ok_or_else(|| DELETED_ELSEWHERE.to_owned())?;

        let name = editor.text(0);
        let position = position(&editor.text(1))?;
        let template_name = editor.text(2);
        let gate_hard =
            super::yes_or_no(&editor.text(3)).ok_or_else(|| GATE_IS_Y_OR_N.to_owned())?;
        let input_kinds = input_kinds(&editor.text(4));
        let budget = budget(&editor.text(5))?;
        let persona = persona_patch(
            &editor.text(6),
            editor.stored_persona.as_deref().unwrap_or_default(),
            self.personas(),
        )?;

        // The persona is part of the patch (D-4): left out, a persona + budget edit with no other
        // column moved would take the budget-only branch below and drop the persona.
        let patch_changed = name != stored.name
            || position != stored.position
            || template_name != stored.template_name
            || gate_hard != stored.gate_hard
            || input_kinds != stored.input_kinds
            || persona.is_some();
        let budget_changed = editor.stored_budget.as_deref() != Some(editor.text(5).trim());

        let patch = PhasePatch {
            name: Some(name),
            position: Some(position),
            template_name: Some(template_name),
            gate_hard: Some(gate_hard),
            input_kinds: Some(input_kinds),
            persona,
        };
        // A budget-only change is one write on the rung; anything else starts with the patch.
        if budget_changed && !patch_changed {
            return Ok((
                StoreRequest::SetPhaseBudget {
                    scope: scope.clone(),
                    phase: id,
                    expected,
                    budget,
                },
                None,
            ));
        }
        Ok((
            StoreRequest::UpdatePhase {
                scope: scope.clone(),
                id,
                expected,
                patch,
            },
            budget_changed.then_some(FollowUp::Budget(budget)),
        ))
    }

    /// The persona registry the last catalogue carried, or none before the first one (D23).
    fn personas(&self) -> &[PersonaSummary] {
        self.snapshot
            .as_ref()
            .map_or(&[], |snapshot| snapshot.personas.as_slice())
    }

    /// One phase of the snapshot, by id.
    fn phase_by_id(&self, id: PhaseId) -> Option<&StepGraphPhase> {
        self.snapshot
            .as_ref()?
            .projects
            .iter()
            .flat_map(|project| project.graphs.iter())
            .flat_map(|entry| entry.phases.iter())
            .find(|phase| phase.id == id)
    }

    /// One project of the snapshot, by id.
    fn project_by_id(&self, id: ProjectId) -> Option<&ProjectCatalogue> {
        self.snapshot
            .as_ref()?
            .projects
            .iter()
            .find(|project| project.project.id == id)
    }

    /// The second write of B-4's chain, sent from the first one's reply.
    ///
    /// Answers whether it took the reply: the editor stays open until the budget lands, so the
    /// caller must not close it. The token comes from the snapshot that just arrived — the one the
    /// first write moved — because the editor's own is spent by construction.
    fn follow_up(&mut self, ctx: &Ctx<'_>) -> bool {
        let Mode::Editing(editor) = &self.mode else {
            return false;
        };
        let EditorKind::EditPhase(id) = editor.kind else {
            return false;
        };
        let Some(FollowUp::Budget(budget)) = editor.follow_up else {
            return false;
        };
        let Some(expected) = self.phase_by_id(id).map(|phase| phase.updated_at) else {
            // The row went while the patch was in flight: there is nothing left to set a budget on.
            self.mode = Mode::Browse;
            self.say(DELETED_ELSEWHERE);
            return true;
        };
        // `stored_budget` is left where it was, holding what the *row* said when the editor opened.
        // Writing the typed text here would say the number had landed before its write had even
        // been answered: a `CatalogueStale` or a `Failed` on it would leave the retry `Enter` with
        // `budget_changed == false`, sending a no-op `update_phase` that closes the editor with the
        // budget never written. The editor closes on the budget's own reply either way.
        if let Mode::Editing(editor) = &mut self.mode {
            editor.expected = Some(expected);
            editor.follow_up = None;
        }
        self.send(
            StoreRequest::SetPhaseBudget {
                scope: ctx.scope.clone(),
                phase: id,
                expected,
                budget,
            },
            ctx,
        );
        true
    }

    /// A kind is gone, with what the worker did to the mirror afterwards (D12).
    ///
    /// The prefix comes from the mode when there is one; a reply that arrives with none — a scope
    /// change dropped it, say — still reports the mirror rather than panicking (H-16).
    fn on_deleted(&mut self, mirror: &MirrorAfterDelete, snapshot: &CatalogueSnapshot) {
        self.busy = None;
        let prefix = match &self.mode {
            Mode::Deleting { prefix, .. } => Some(prefix.clone()),
            Mode::Browse | Mode::Editing(_) | Mode::ConfirmPrefix { .. } => None,
        };
        self.mode = Mode::Browse;
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        self.say(&deleted_notice(prefix.as_deref(), mirror));
    }

    /// A compare-and-set miss (D8): the tree is replaced, the editor keeps its text and takes the
    /// current row's token, and the retry is a second `Enter` rather than an automatic write.
    fn on_stale(&mut self, snapshot: &CatalogueSnapshot) {
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
            Mode::Browse | Mode::ConfirmPrefix { .. } | Mode::Deleting { .. } => None,
        };
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        match reloaded {
            // Nothing is open to press `Enter` on, so the sentence has to carry what the editor
            // would otherwise stand for: the write did not apply, and the way back is to reopen.
            None => self.say(CHANGED_ELSEWHERE_CLOSED),
            Some(Reload::Keep) => self.say(CHANGED_ELSEWHERE),
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
                    // `stored_budget` is **not** refreshed with it, which is where this departs
                    // from blueprint §4.9's "`stored_prefix`/`stored_budget` refreshed from the
                    // current row". The two are not symmetric: `stored_prefix` decides whether a
                    // *question* is asked, so asking it against the row as it is now is right,
                    // while `stored_budget` decides whether a *write* goes out at all (B-4). Taken
                    // from the row the other writer just moved, an untouched `token_budget` field
                    // would read as changed and this editor's `Enter` would write the old number
                    // back over theirs. Left alone, it still means what it meant when the editor
                    // opened — what this user found there — so only a user who typed in the column
                    // writes it.
                }
                self.say(CHANGED_ELSEWHERE);
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

    /// One line per row, in [`rows`](KindsSection::rows) order, plus the read-only detail of the
    /// selected phase (D15, B-11).
    ///
    /// The detail is not a row: the cursor never lands on it, so a row's index is its line's index
    /// up to and including the selection.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (index, row) in self.rows().into_iter().enumerate() {
            let selected = index == self.cursor;
            // MOD-80 review L1: the cursor row in `selected`, as Connection's and Boxes' are.
            let style = if selected { theme.selected } else { theme.base };
            match row {
                Row::Project { p } => {
                    if let Some(entry) = self.project(p) {
                        lines.push(Line::styled(
                            format!("{}  {}", entry.project.slug, entry.project.name),
                            style,
                        ));
                    }
                }
                Row::Kind { p, k } => {
                    let Some(project) = self.project(p) else {
                        continue;
                    };
                    let Some(kind) = project.kinds.get(k) else {
                        continue;
                    };
                    let head = format!("  {}  {} \u{b7} ", kind.prefix, kind.name);
                    match project.graph(kind.default_graph_id) {
                        Some(entry) => lines.push(Line::styled(
                            format!("{head}graph {}", entry.graph.name),
                            style,
                        )),
                        // The tail is the one thing on this line the user has to act on, so it is
                        // the one thing in `theme.error`.
                        None => {
                            let line = Line::from(vec![
                                Span::styled(head, style),
                                Span::styled(GRAPH_MISSING.to_owned(), theme.error),
                            ]);
                            lines.push(if selected { theme.select(line) } else { line });
                        }
                    }
                }
                Row::Graph { p, g } => {
                    if let Some(entry) = self.graph_at(p, g) {
                        let style = if selected { theme.selected } else { theme.dim };
                        lines.push(Line::styled(
                            format!("  {} \u{b7} graph, no kind", entry.graph.name),
                            style,
                        ));
                    }
                }
                Row::Phase { p, g, i } => {
                    let Some(phase) = self.phase_at(p, g, i) else {
                        continue;
                    };
                    lines.push(Line::styled(phase_line(phase, self.personas()), style));
                    if selected {
                        lines.extend(detail_lines(phase, width, theme));
                    }
                }
            }
        }
        lines
    }

    /// The one line under the rows: the keys this mode binds, then the last outcome.
    ///
    /// Two spans rather than one string: a compare-and-set miss is reported here and D8 asks for it
    /// in `theme.error`, because "someone else wrote to this row" is the one notice a user has to
    /// act on rather than read.
    fn hint(&self, width: u16, theme: &Theme, keys: &Keys) -> Line<'static> {
        let keys = self.hint_text(keys);
        let Some(notice) = &self.notice else {
            return Line::styled(keys, theme.dim);
        };
        let style = if notice.is_error() {
            theme.error
        } else {
            theme.dim
        };
        let text = notice.text().to_owned();
        // The outcome wins the line when both do not fit: the keys are on screen every other
        // frame, and this is the only place the outcome appears.
        let room = usize::from(width);
        if cell_width(&keys) + cell_width(&text) + 3 > room {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The keys half of the hint line, plus what a write in flight adds to it. Rendered through
    /// the mode's own stack, so a rebound key shows its new chord.
    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match &self.mode {
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
                format!("{keys} \u{b7} {busy} in flight")
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
        }
    }

    /// One key in Browse, through `views::KINDS_BROWSE` (MOD-67 M3). A chord matches with its
    /// modifiers, so `ctrl-d` is not `d` (ANA-26 §2.6 defect 1), and `g` is this view's own
    /// `settings.kinds.graph`, never `list.top` (D2). The tab took `settings.*` before offering
    /// the key; a global act, or one this state declines, is the shell's (`Pass`).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::KINDS_BROWSE, chord) {
            match act {
                Act::New => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_new_child(row);
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
                        self.open_edit(row);
                    }
                }
                Act::KindsGraph => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.open_graph(row);
                    }
                }
                Act::Delete => {
                    if !self.blocked()
                        && let Some(row) = self.selected()
                    {
                        self.begin_delete(row);
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

    /// The pane under the rows: the open editor, the question being answered, or nothing at all in
    /// Browse.
    ///
    /// Takes the width because both questions are sentences rather than rows and have to wrap
    /// inside the pane they are measured for — the layout needs the height first.
    fn pane(&self, width: u16, theme: &Theme, keys: &Keys) -> Vec<Line<'static>> {
        match &self.mode {
            Mode::Browse => Vec::new(),
            Mode::Editing(editor) => editor.lines(width, theme),
            // The question, and while the delete is out, what it is waiting for.
            Mode::Deleting {
                name,
                prefix,
                stage,
                ..
            } => {
                let answers = keys.hint(views::KINDS_CONFIRM, HINT_DELETING);
                let mut lines: Vec<Line<'static>> = wrapped(
                    &delete_question(name, prefix, &answers),
                    usize::from(width).max(1),
                )
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect();
                if matches!(stage, DeleteStage::InFlight) {
                    lines.push(Line::styled("delete_kind in flight".to_owned(), theme.dim));
                }
                lines
            }
            // The warning and nothing else: what is being answered is the sentence, and the fields
            // behind it are one `n` away.
            Mode::ConfirmPrefix { old, new, .. } => {
                wrapped(&prefix_warning(old, new), usize::from(width).max(1))
                    .into_iter()
                    .map(|line| Line::styled(line, theme.error))
                    .collect()
            }
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
    /// reply and closes the editor early. The case the reviewer of M4 names is a re-read issued in
    /// the middle of B-4's two-write chain: the first `Catalogue` sends `set_phase_budget` and the
    /// *read's* reply, arriving next, is counted as the budget's. `r` is not that door today —
    /// `on_key` hands every key to the open editor, where `r` is a letter (`:1336`), and every write
    /// this section sends leaves `Browse` behind — but a scope change, a tab re-activation
    /// (`app/state.rs`'s `wants_requests`) or a future read on the Browse side all are. `r` stays
    /// allowed anyway: re-reading is how a section that lost a reply recovers, and refusing it while
    /// busy would wedge exactly the section that needs it. The loss is bounded and deliberate — the
    /// write itself has already been sent, so all that goes is the retry affordance if it comes back
    /// `CatalogueStale`, and [`CHANGED_ELSEWHERE_CLOSED`] says so when it does.
    fn on_catalogue(&mut self, snapshot: &CatalogueSnapshot, ctx: &Ctx<'_>) {
        let write = self.busy.take();
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // catalogue is unavailable over a store that just spoke.
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
        if write == Some("update_phase") && self.follow_up(ctx) {
            return;
        }
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
        vec![StoreRequest::Catalogue(scope.clone())]
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

    /// MOD-22 review M-1: a bracketed paste into the open form's focused field. A delete question
    /// is not a field: a paste there is dropped, so its `y` confirms nothing.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        if let Some(field) = editor.fields.get_mut(editor.focus) {
            field.input.on_paste(text);
        }
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
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
        match reply {
            StoreReply::Catalogue(snapshot) => self.on_catalogue(snapshot, ctx),
            StoreReply::CatalogueStale(snapshot) => self.on_stale(snapshot),
            StoreReply::KindDeleted { mirror, catalogue } => self.on_deleted(mirror, catalogue),
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            // `busy` is deliberately left alone: a read never sets it, so the only thing clearing
            // it here could do is orphan a write that is still in flight — and let a second
            // `Enter` re-send it.
            StoreReply::Failed { request, message } if *request == "catalogue" => {
                self.unavailable = Some(message.clone());
            }
            // Every other refusal of this section's own: the shell has already put
            // `{request}: {message}` on the status line, so all this owes is the sentence and a
            // state the next key can start from.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                // The editor stays open over its text: a refused write is retried by fixing what
                // was refused and pressing `Enter` again. A refused **delete** is not: the seam
                // answered the question the confirmation asked, so leaving `delete_kind in flight`
                // on screen would be waiting for something that already came back.
                if matches!(self.mode, Mode::Deleting { .. }) {
                    self.mode = Mode::Browse;
                }
                self.refuse(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(area.width, ctx.theme, ctx.keys());
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        match (&self.unavailable, &self.snapshot) {
            // The refusal wins the pane even with a tree behind it: what is on screen would
            // otherwise be a catalogue nothing has confirmed since the outage started (H-17).
            (Some(why), _) => frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("{UNAVAILABLE}: {why}"),
                    ctx.theme.error,
                ))
                .wrap(Wrap { trim: true }),
                rows,
            ),
            (None, None) => message(frame, rows, NO_WORKSPACE, ctx.theme),
            (None, Some(snapshot)) if snapshot.projects.is_empty() => {
                message(frame, rows, NO_PROJECT, ctx.theme);
            }
            (None, Some(_)) => {
                // Scrolled so the cursor is on screen: the tree of a scope with several projects
                // is longer than the pane, and a selection nobody can see is a cursor that acts
                // blind.
                let height = usize::from(rows.height);
                let offset = self.cursor.saturating_sub(height.saturating_sub(1));
                frame.render_widget(
                    Paragraph::new(self.lines(area.width, ctx.theme))
                        .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
                    rows,
                );
            }
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(
            Paragraph::new(self.hint(area.width, ctx.theme, ctx.keys())),
            hint,
        );
    }
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

    /// One line per field, the focused label accented and the focused field carrying the cursor.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let label_width = self
            .fields
            .iter()
            .map(|field| cell_width(field.label))
            .max()
            .unwrap_or(0);
        self.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = index == self.focus;
                let style = if focused { theme.accent } else { theme.dim };
                let label = cells::pad(field.label, label_width);
                let mut spans = vec![Span::styled(format!("{label}: "), style)];
                let room = usize::from(width).saturating_sub(label_width + 2);
                spans.extend(
                    field
                        .input
                        .line(u16::try_from(room).unwrap_or(u16::MAX), focused, theme)
                        .spans,
                );
                Line::from(spans)
            })
            .collect()
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

/// The same five fields, prefilled from the row (B-5's whole patch goes out from them).
fn edit_kind_fields(project: &ProjectCatalogue, kind: &ItemKind) -> Vec<Field> {
    let graph = project
        .graph(kind.default_graph_id)
        .map_or("", |entry| entry.graph.name.as_str());
    vec![
        Field::required("prefix", &kind.prefix),
        Field::required("name", &kind.name),
        Field::optional("description", &kind.description),
        Field::required("graph", graph),
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

/// The five columns a new phase carries (D9, B-3).
///
/// No `token_budget`: a phase is born inheriting it, and the sixth column is set by `e` afterwards
/// — a create followed by a budget write would need the new row's id, which only the reply knows.
fn new_phase_fields(entry: &GraphEntry) -> Vec<Field> {
    vec![
        Field::required("name", ""),
        Field::required(
            "position",
            &next_position(entry.phases.iter().map(|phase| phase.position)),
        ),
        Field::required("template_name", ""),
        Field::required("gate_hard (y/n)", "n"),
        Field::optional("input_kinds", ""),
    ]
}

/// PRD D2's six editable columns plus MOD-26 D23's persona, in editor order.
fn edit_phase_fields(phase: &StepGraphPhase, personas: &[PersonaSummary]) -> Vec<Field> {
    vec![
        Field::required("name", &phase.name),
        Field::required("position", &phase.position.to_string()),
        Field::required("template_name", &phase.template_name),
        Field::required("gate_hard (y/n)", if phase.gate_hard { "y" } else { "n" }),
        Field::optional("input_kinds", &phase.input_kinds.join(",")),
        Field::optional("token_budget", &budget_text(phase.token_budget)),
        Field::optional("persona", &persona_text(phase.persona_id, personas)),
    ]
}

/// `token_budget` as the editor holds it: empty is `inherit` (D16).
fn budget_text(budget: Option<i32>) -> String {
    budget.map_or_else(String::new, |n| n.to_string())
}

/// `input_kinds` as D13 reads it: split on `,`, each part trimmed, empty parts dropped, order
/// preserved, no de-duplication, replaced whole.
///
/// De-duplicating would silently edit what was typed; the store takes the list as given.
fn input_kinds(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The budget field as a number, or `None` for "inherit" (B-1, D16).
fn budget(text: &str) -> Result<Option<i64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<i64>()
        .map(Some)
        .map_err(|_| BUDGET_IS_A_NUMBER.to_owned())
}

/// One past the largest position, or `0` when there is none.
fn next_position(positions: impl Iterator<Item = i32>) -> String {
    positions
        .max()
        .map_or(0, |last| last.saturating_add(1))
        .to_string()
}

/// The graph of this project that carries the typed name (B-7).
///
/// A name rather than an id because that is what is on screen; the store checks the project again
/// on the write, so this resolution is a convenience and not the guard.
fn graph_named(project: &ProjectCatalogue, name: &str) -> Result<StepGraphId, String> {
    project
        .graphs
        .iter()
        .find(|entry| entry.graph.name == name)
        .map(|entry| entry.graph.id)
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

/// The `persona` field's text at open: blank for none (D23).
fn persona_text(bound: Option<PersonaId>, personas: &[PersonaSummary]) -> String {
    bound.map_or_else(String::new, |id| persona_label(id, personas))
}

/// The `persona` field as `PhasePatch.persona` (MOD-26 M2 D23, OQ-11; B-8): `None` while the
/// trimmed text is still the trimmed text the editor opened with — an id the list does not know
/// included, so it is left alone unless edited; `Some(None)` for a blank field; `Some(Some(id))`
/// for a name the list holds.
///
/// # Errors
/// [`no_persona_named`] for any other text.
fn persona_patch(
    text: &str,
    opened: &str,
    personas: &[PersonaSummary],
) -> Result<Option<Option<PersonaId>>, String> {
    let text = text.trim();
    if text == opened.trim() {
        return Ok(None);
    }
    if text.is_empty() {
        return Ok(Some(None));
    }
    personas
        .iter()
        .find(|row| row.name == text)
        .map(|row| Some(Some(row.id)))
        .ok_or_else(|| no_persona_named(text, personas))
}

/// The refusal of a name the loaded list does not hold, before anything is sent (D23).
fn no_persona_named(name: &str, personas: &[PersonaSummary]) -> String {
    if personas.is_empty() {
        return NO_PERSONAS.to_owned();
    }
    let known: Vec<&str> = personas.iter().map(|row| row.name.as_str()).collect();
    format!(
        "no persona named `{}`; known: {}",
        name.escape_debug(),
        known.join(", ")
    )
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
        EditorKind::NewPhase(graph) => snapshot
            .projects
            .iter()
            .flat_map(|project| project.graphs.iter())
            .find(|entry| entry.graph.id == graph)
            .map_or(Reload::Gone, |_| Reload::Keep),
        EditorKind::EditPhase(id) => snapshot
            .projects
            .iter()
            .flat_map(|project| project.graphs.iter())
            .flat_map(|entry| entry.phases.iter())
            .find(|phase| phase.id == id)
            .map_or(Reload::Gone, |phase| Reload::Token(phase.updated_at)),
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
        MirrorAfterDelete::NotNeeded => "no rebuild needed".to_owned(),
        MirrorAfterDelete::Failed(err) => format!("mirror not rebuilt: {err}"),
    };
    match prefix {
        Some(prefix) => format!("deleted kind `{prefix}`; {mirror}"),
        None => format!("deleted kind; {mirror}"),
    }
}

/// What a second write of the same kind is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// What an empty required field is told, before the store is asked anything.
fn required(label: &str) -> String {
    format!("`{label}` is required")
}

/// One phase row: the six columns this section edits, in editor order (D2's six, minus the ones
/// the row cannot show), then the bound persona's name — only when one is bound, so an unbound
/// phase's line is exactly what it was before MOD-26 M2 (D23, I-12).
fn phase_line(phase: &StepGraphPhase, personas: &[PersonaSummary]) -> String {
    let mut line = format!(
        "    {}. {} \u{b7} template {} \u{b7} gate {} \u{b7} in {} \u{b7} budget {}",
        phase.position,
        phase.name,
        phase.template_name,
        if phase.gate_hard { "hard" } else { "soft" },
        if phase.input_kinds.is_empty() {
            "none".to_owned()
        } else {
            phase.input_kinds.join(",")
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
fn budget_label(budget: Option<i32>) -> String {
    budget.map_or_else(|| "inherit".to_owned(), |n| n.to_string())
}

/// MOD-4's eight columns under the selected phase, dimmed and unselectable (D15).
///
/// Wrapped here rather than clipped: the last words are the ones that say why nothing on the line
/// can be typed into, and a detail line cut off at `verify_command` would read as a row of the tree
/// that someone forgot to finish.
fn detail_lines(phase: &StepGraphPhase, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let text = format!(
        "fan_out {} \u{b7} gate {} \u{b7} isolation {} \u{b7} command_queue {} \u{b7} verify_command {} \u{b7} retry_limit {} \u{b7} output_kind {} \u{b7} template_version {} \u{2014} {MOD_4_OWNS}",
        phase.fan_out,
        phase.gate.as_str(),
        phase
            .isolation
            .map_or("none", htui_core::model::Isolation::as_str),
        phase.command_queue.as_str(),
        phase.verify_command.as_deref().unwrap_or("none"),
        phase.retry_limit,
        phase.output_kind,
        phase
            .template_version
            .map_or_else(|| "none".to_owned(), |n| n.to_string()),
    );
    let room = usize::from(width).saturating_sub(6).max(1);
    wrapped(&text, room)
        .into_iter()
        .map(|line| Line::styled(format!("      {line}"), theme.dim))
        .collect()
}

/// What a modal mode answers for a chord none of its own acts took (MOD-67 M3 PA-5): `Pass` when
/// the stack's global layer admits its shape (`CONTROL`, `ALT`, a function key), so the shell
/// quits, opens help or finds; otherwise it is swallowed, so a `q` at a question does not quit.
fn unused(stack: Stack<'_>, chord: KeyChord) -> Handled {
    if stack.passes(chord) {
        Handled::Pass
    } else {
        Handled::Consumed
    }
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
            stored_budget: None,
            stored_persona: None,
            follow_up: None,
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

    /// MOD-60: the hint line measures the notice in cells, so a CJK notice that fits by `char`
    /// count but not on screen takes the line alone rather than overrunning it.
    #[test]
    fn a_wide_notice_takes_the_hint_line_alone() {
        let k = 10;
        let notice = "\u{6f22}".repeat(k);
        let section = KindsSection {
            notice: Some(Notice::Error(notice.clone())),
            ..KindsSection::new()
        };
        let keys = section.hint_text(Keys::compiled());
        let width = cell_width(&keys) + k + 3;

        let line = section.hint(
            u16::try_from(width).expect("a hint this narrow fits u16"),
            &Theme::default(),
            Keys::compiled(),
        );

        assert_eq!(line.spans.len(), 1, "{line:?} against {width}");
        assert_eq!(line.spans[0].content, notice);
    }
}
