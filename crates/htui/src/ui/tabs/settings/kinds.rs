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

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use htui_core::model::{Scope, StepGraphPhase};

use crate::app::{Ctx, Handled};
use crate::catalogue::{CatalogueSnapshot, GraphEntry, ProjectCatalogue, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::settings::{SectionId, SettingsSection, message};
use crossterm::event::{KeyCode, KeyEvent};

/// What the rows pane says before any catalogue has arrived.
const NO_WORKSPACE: &str = "no workspace: nothing to list";

/// What the rows pane says when the read answered with no project at all.
const NO_PROJECT: &str = "no project in scope";

/// What the rows pane says when the read itself was refused.
const UNAVAILABLE: &str = "catalogue unavailable";

/// What a kind whose `default_graph_id` names no graph of its project says (D4).
const GRAPH_MISSING: &str = "graph missing";

/// Browse's keys.
const HINT_BROWSE: &str = "j/k \u{b7} n kind/phase \u{b7} N graph \u{b7} e edit \u{b7} g graph \u{b7} d delete kind \u{b7} r reload";

/// Browse's keys with nothing to list: the only offer is to ask again.
const HINT_NO_WORKSPACE: &str = "r reload";

/// Browse's keys with the read refused: nothing here can be written against a store that did not
/// answer.
const HINT_UNAVAILABLE: &str = "r reload";

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

/// The catalogue of the scope, with the keys that edit it.
#[derive(Debug, Default)]
pub struct KindsSection {
    /// The last catalogue the worker assembled, or `None` before the first reply.
    snapshot: Option<CatalogueSnapshot>,
    /// `Some(message)` after `Failed { request: "catalogue" }`: the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](KindsSection::rows).
    cursor: usize,
    /// The last outcome, one line on the hint row.
    notice: Option<String>,
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

    /// One line per row, in [`rows`](KindsSection::rows) order, plus the read-only detail of the
    /// selected phase (D15, B-11).
    ///
    /// The detail is not a row: the cursor never lands on it, so a row's index is its line's index
    /// up to and including the selection.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (index, row) in self.rows().into_iter().enumerate() {
            let selected = index == self.cursor;
            let style = if selected { theme.accent } else { theme.base };
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
                        None => lines.push(Line::from(vec![
                            Span::styled(head, style),
                            Span::styled(GRAPH_MISSING.to_owned(), theme.error),
                        ])),
                    }
                }
                Row::Graph { p, g } => {
                    if let Some(entry) = self.graph_at(p, g) {
                        let style = if selected { theme.accent } else { theme.dim };
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
                    lines.push(Line::styled(phase_line(phase), style));
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
    fn hint(&self, width: u16, theme: &Theme) -> Line<'static> {
        let keys = self.hint_text();
        let Some(notice) = &self.notice else {
            return Line::styled(keys, theme.dim);
        };
        // Every notice this milestone's browse half can raise is a refusal: the seam's own
        // sentence, or this section's answer to a key that cannot act here.
        let style = theme.error;
        let text = notice.clone();
        // The outcome wins the line when both do not fit: the keys are on screen every other
        // frame, and this is the only place the outcome appears.
        let room = usize::from(width);
        if keys.chars().count() + text.chars().count() + 3 > room {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The keys half of the hint line.
    fn hint_text(&self) -> String {
        let keys = if self.unavailable.is_some() {
            HINT_UNAVAILABLE
        } else if self
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.projects.is_empty())
        {
            HINT_NO_WORKSPACE
        } else {
            HINT_BROWSE
        };
        keys.to_owned()
    }

    /// Reports a refusal: the seam's own sentence, or one of this section's.
    fn refuse(&mut self, text: String) {
        self.notice = Some(text);
    }

    /// A fresh catalogue: the tree is replaced whole and the cursor put back inside it (D3).
    fn on_catalogue(&mut self, snapshot: &CatalogueSnapshot) {
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // catalogue is unavailable over a store that just spoke.
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        self.clamp_cursor();
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
        // The tree and the cursor belong to the workspace that was left (B-12). The notice
        // survives, because the scope change is often the *consequence* of what it is reporting.
        self.snapshot = None;
        self.cursor = 0;
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(true);
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(false);
                Handled::Consumed
            }
            // Allowed whatever else is going on: re-reading is how a section that lost a reply
            // recovers, and a read cannot lose a write's reply — the staleness index is keyed by
            // request kind.
            KeyCode::Char('r') => {
                ctx.request(StoreRequest::Catalogue(ctx.scope.clone()));
                Handled::Consumed
            }
            // Only when there is something to clear: a section that swallowed every `Esc` would
            // take the one the shell uses to close an overlay over it.
            KeyCode::Esc if self.notice.is_some() => {
                self.notice = None;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Catalogue(snapshot) => self.on_catalogue(snapshot),
            // The read itself was refused: saying so beats an empty tree that reads as "nothing
            // here yet" (the agent section's rule, one section across).
            StoreReply::Failed { request, message } if *request == "catalogue" => {
                self.unavailable = Some(message.clone());
            }
            // Every other refusal of this section's own: the shell has already put
            // `{request}: {message}` on the status line, so all this owes is the sentence and a
            // state the next key can start from.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.refuse(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let [rows, hint] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);

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
        frame.render_widget(Paragraph::new(self.hint(area.width, ctx.theme)), hint);
    }
}

/// One phase row: the six columns this section edits, in editor order (D2's six, minus the ones
/// the row cannot show).
fn phase_line(phase: &StepGraphPhase) -> String {
    format!(
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
    )
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

/// One sentence broken into lines of at most `width` chars, on spaces.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let extra = if line.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if !line.is_empty() && line.chars().count() + extra > width {
            lines.push(core::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
