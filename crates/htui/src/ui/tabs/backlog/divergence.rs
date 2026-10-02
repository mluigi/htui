//! The three-way divergence view (MOD-13 milestone 3 D3–D5): what a stale edit opens.
//!
//! - **A mode of the item form** (D3): the form owns the open [`Divergence`], and while it is
//!   open every key and paste goes to it, not to the fields.
//! - **Drawn over the whole tab area** (D4), list pane included: three value columns and two
//!   diffs side by side do not fit the detail pane.
//! - **The ancestor is the item the form opened on** (D1), not the reply's `ItemRevision`: a
//!   revision has no history for four of the seven columns, and the opened item is exactly what
//!   the form's untouched fields still hold (A4).
//! - **Rows list only the fields that are not `Same`** (D4), each with its state and the three
//!   values. Body and paths show as two unified diffs side by side, ancestor → theirs and
//!   ancestor → mine.
//! - **`m`/`t` decide the conflicts only** (D2, D5): the rest of the merge keeps both sides'
//!   changes. `Esc` returns to the form unchanged.
//!
//! Redaction (milestone 2 E10): a [`Row`] holds summaries, never body or path text (`3 lines`,
//! `2 paths`), and [`Divergence`]'s `Debug` is hand-written to print the rows' fields and states
//! only.

use crossterm::event::{KeyCode, KeyEvent};
use htui_core::model::item_merge::{self, FieldState, Side, SpecField, SpecMerge};
use htui_core::model::item_spec;
use htui_core::model::{Item, ItemKindId, ItemSpec, StepGraphId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::item_form::{ctrl_s, notice_lines};
use crate::item_writes::{ItemDivergence, ItemFormContext};
use crate::ui::tabs::backlog::{filter, list::clip};
use crate::ui::{Theme, diff};

/// The hint's pick part while a field conflicts (D4).
pub const HINT_SIDES: &str = "t theirs wins  m mine wins";

/// The hint's pick part with no conflict (D5): `m` and `t` give the same spec.
pub const HINT_NO_CONFLICT: &str = "no conflicts \u{2014} m or t continues";

/// Shown when both body and paths differ.
pub const HINT_TAB: &str = "Tab body/paths";

/// Always last.
pub const HINT_BACK: &str = "Esc back";

/// Width of the label column with its trailing gap (`priority` + 2).
pub const LABEL_WIDTH: usize = 10;

/// Width of the state column with its trailing gap (`conflict` + 2).
pub const STATE_WIDTH: usize = 10;

/// Rows the diffs keep at least when a body or paths row is shown.
pub const DIFF_MIN: u16 = 6;

/// Rows `PageUp`/`PageDown` move (blueprint E7: `detail::PAGE`'s value).
pub const PAGE: usize = 10;

/// D4: the block title, `KEY  v{ancestor} → theirs v{head}`.
#[must_use]
pub fn header(key: &str, ancestor: i32, head: i32) -> String {
    format!("{key}  v{ancestor} \u{2192} theirs v{head}")
}

/// D5: the form's notice after `Esc` in the view; its token and text are unchanged.
#[must_use]
pub fn still_behind(head: i32) -> String {
    format!("still behind v{head}; Ctrl+S compares again")
}

/// D5: the rebased form's notice; the token is now `head`.
#[must_use]
pub fn rebased_on(head: i32) -> String {
    format!("rebased on v{head}; Ctrl+S saves the resolution")
}

/// Which text the two diffs show (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffPane {
    /// The body.
    Body,
    /// The touched paths, one per line.
    Paths,
}

impl DiffPane {
    /// The diff section's label: `body` or `paths`.
    const fn label(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Paths => "paths",
        }
    }
}

/// One listed field: its state and three short values.
///
/// Never body or path text: those two rows hold counts (`3 lines`, `2 paths`), and the diffs show
/// the text. That invariant is what lets `Row` derive `Debug` (E10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The field.
    pub field: SpecField,
    /// How it moved (D2).
    pub state: FieldState,
    /// The ancestor's value, as the view draws it.
    pub ancestor: String,
    /// The head's value, as the view draws it.
    pub theirs: String,
    /// The form's value, as the view draws it.
    pub mine: String,
}

/// What one key did in the view, for the form to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewOutcome {
    /// Scrolled, switched or swallowed.
    Stay,
    /// A chord ([`filter::CHORD`]) other than Ctrl+S: the form passes it.
    Pass,
    /// `Esc`: back to the form unchanged (D5).
    Back,
    /// `t` or `m` (D5).
    Resolve(Side),
}

/// The open three-way view (D3).
pub struct Divergence {
    /// The reply's catalogue (D7), with `item = Some(head)`.
    context: ItemFormContext,
    /// The item's key, for the header.
    key: String,
    /// The form's token: the version of the item it opened on (D1).
    ancestor_version: i32,
    /// The head's version.
    head_version: i32,
    /// The three specs.
    merge: SpecMerge,
    /// The fields that are not `Same`, in form order.
    rows: Vec<Row>,
    /// The text the diffs show; `None` when body and paths are both `Same`.
    pane: Option<DiffPane>,
    /// The diffs' first line.
    scroll: u16,
}

/// The key, the versions, the rows' fields and states, the pane and the scroll; never the merge
/// or a row's values (E10).
impl core::fmt::Debug for Divergence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Divergence")
            .field("key", &self.key)
            .field("ancestor_version", &self.ancestor_version)
            .field("head_version", &self.head_version)
            .field(
                "rows",
                &self
                    .rows
                    .iter()
                    .map(|row| (row.field, row.state))
                    .collect::<Vec<_>>(),
            )
            .field("pane", &self.pane)
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

impl Divergence {
    /// D1, D2: `ancestor` is the item the form opened on and `catalogue` the form's own (the
    /// label fallback for a kind or graph the fresh one lacks); `mine` is the form's typed spec.
    /// `divergence.context` (D7) becomes the view's, with `item = Some(head)` set here as well.
    #[must_use]
    pub fn open(
        ancestor: &Item,
        catalogue: &ItemFormContext,
        mine: ItemSpec,
        divergence: &ItemDivergence,
    ) -> Self {
        let theirs = ItemSpec::of(&divergence.head);
        let merge = item_merge::merge(&ItemSpec::of(ancestor), &theirs, &mine);
        let mut context = divergence.context.clone();
        context.item = Some(divergence.head.clone());
        let labels = Labels {
            fresh: &context,
            old: catalogue,
        };
        let rows = merge
            .changed()
            .into_iter()
            .map(|field| Row {
                field,
                state: merge.state(field),
                ancestor: labels.value(field, merge.ancestor()),
                theirs: labels.value(field, merge.theirs()),
                mine: labels.value(field, merge.mine()),
            })
            .collect();
        let pane = if merge.state(SpecField::Body) != FieldState::Same {
            Some(DiffPane::Body)
        } else if merge.state(SpecField::Paths) != FieldState::Same {
            Some(DiffPane::Paths)
        } else {
            None
        };
        Self {
            key: divergence.head.key.clone(),
            ancestor_version: ancestor.version,
            head_version: divergence.head.version,
            context,
            merge,
            rows,
            pane,
            scroll: 0,
        }
    }

    /// One key (D5), in order: Ctrl+S is swallowed, another chord passes, `Esc` goes back, `t`/`m`
    /// resolve, `Tab`/`BackTab` switch the diffs, `j`/`k`/`PageDown`/`PageUp` scroll them, and
    /// anything else is swallowed.
    pub fn on_key(&mut self, key: KeyEvent) -> ViewOutcome {
        // Caught before the chord pass (D5): a passed Ctrl+S would reach the app.
        if ctrl_s(&key) {
            return ViewOutcome::Stay;
        }
        if key.modifiers.intersects(filter::CHORD) {
            return ViewOutcome::Pass;
        }
        match key.code {
            KeyCode::Esc => return ViewOutcome::Back,
            KeyCode::Char('t') => return ViewOutcome::Resolve(Side::Theirs),
            KeyCode::Char('m') => return ViewOutcome::Resolve(Side::Mine),
            KeyCode::Tab | KeyCode::BackTab if self.both_texts() => {
                self.pane = match self.pane {
                    Some(DiffPane::Body) => Some(DiffPane::Paths),
                    Some(DiffPane::Paths) | None => Some(DiffPane::Body),
                };
                self.scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1, true),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(1, false),
            KeyCode::PageDown => self.scroll_by(PAGE, true),
            KeyCode::PageUp => self.scroll_by(PAGE, false),
            _ => {}
        }
        ViewOutcome::Stay
    }

    /// The listed fields.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The text the diffs show, if any.
    #[must_use]
    pub const fn pane(&self) -> Option<DiffPane> {
        self.pane
    }

    /// The diffs' first line.
    #[must_use]
    pub const fn scroll(&self) -> u16 {
        self.scroll
    }

    /// The head's version: the rebased form's token.
    #[must_use]
    pub const fn head_version(&self) -> i32 {
        self.head_version
    }

    /// The form's token, the version of the item it opened on (D1).
    #[must_use]
    pub const fn ancestor_version(&self) -> i32 {
        self.ancestor_version
    }

    /// The three specs.
    #[must_use]
    pub const fn merge(&self) -> &SpecMerge {
        &self.merge
    }

    /// D4/D5: [`HINT_SIDES`] or [`HINT_NO_CONFLICT`], then [`HINT_TAB`] when both body and paths
    /// differ, then [`HINT_BACK`], joined by two spaces. With a conflict and both texts that is
    /// exactly D4's `t theirs wins  m mine wins  Tab body/paths  Esc back`.
    #[must_use]
    pub fn hint(&self) -> String {
        let mut parts = vec![if self.merge.has_conflict() {
            HINT_SIDES
        } else {
            HINT_NO_CONFLICT
        }];
        if self.both_texts() {
            parts.push(HINT_TAB);
        }
        parts.push(HINT_BACK);
        parts.join("  ")
    }

    /// The rebased form's inputs: the view's catalogue (item = head) and `merge.resolve(side)`.
    #[must_use]
    pub fn resolve(self, side: Side) -> (ItemFormContext, ItemSpec) {
        let resolved = self.merge.resolve(side);
        (self.context, resolved)
    }

    /// Whether both body and paths differ, so `Tab` has somewhere to go.
    fn both_texts(&self) -> bool {
        self.merge.state(SpecField::Body) != FieldState::Same
            && self.merge.state(SpecField::Paths) != FieldState::Same
    }

    /// Moves the diffs `by` lines, clamped to the longer diff's logical line count minus one
    /// (blueprint E7: a lower bound on the wrapped count, so the view never scrolls blank).
    fn scroll_by(&mut self, by: usize, down: bool) {
        let Some((theirs, mine)) = self.diffs() else {
            return;
        };
        let last = theirs
            .lines()
            .count()
            .max(mine.lines().count())
            .saturating_sub(1);
        let at = usize::from(self.scroll);
        let next = if down {
            at.saturating_add(by)
        } else {
            at.saturating_sub(by)
        };
        self.scroll = u16::try_from(next.min(last)).unwrap_or(u16::MAX);
    }

    /// The two unified diffs, ancestor → theirs and ancestor → mine, of the pane's text; computed
    /// on demand so no text is stored beside the merge. `None` without a pane.
    fn diffs(&self) -> Option<(String, String)> {
        let pane = self.pane?;
        let text = |spec: &ItemSpec| match pane {
            DiffPane::Body => spec.body.clone(),
            DiffPane::Paths => item_spec::paths_text(&spec.touched_paths),
        };
        let (ancestor, theirs, mine) = (
            text(self.merge.ancestor()),
            text(self.merge.theirs()),
            text(self.merge.mine()),
        );
        let from = format!("ancestor v{}", self.ancestor_version);
        Some((
            diff::unified(
                &ancestor,
                &theirs,
                &from,
                &format!("theirs v{}", self.head_version),
            ),
            diff::unified(&ancestor, &mine, &from, "mine"),
        ))
    }
}

/// The two catalogues a kind or graph label is read through: the reply's fresh one first (D7),
/// then the form's own.
struct Labels<'a> {
    fresh: &'a ItemFormContext,
    old: &'a ItemFormContext,
}

impl Labels<'_> {
    /// One field of `spec` as a row draws it: never body or path text, only their counts.
    fn value(&self, field: SpecField, spec: &ItemSpec) -> String {
        match field {
            SpecField::Kind => self.kind(spec.kind_id),
            SpecField::Title => spec.title.clone(),
            SpecField::Priority => spec.priority.to_string(),
            SpecField::Tags if spec.required_tags.is_empty() => "(none)".to_owned(),
            SpecField::Tags => item_spec::tags_text(&spec.required_tags),
            SpecField::Graph => self.graph(spec.step_graph_id),
            SpecField::Paths => count(spec.touched_paths.len(), "path"),
            SpecField::Body => count(spec.body.lines().count(), "line"),
        }
    }

    /// `{prefix} {name}`, or `?` for a kind neither catalogue has.
    fn kind(&self, id: ItemKindId) -> String {
        [self.fresh, self.old]
            .iter()
            .find_map(|context| context.kinds.iter().find(|kind| kind.id == id))
            .map_or_else(
                || "?".to_owned(),
                |kind| format!("{} {}", kind.prefix, kind.name),
            )
    }

    /// The graph picker's words: `kind default`, a project graph's name, or an override.
    fn graph(&self, id: Option<StepGraphId>) -> String {
        let Some(id) = id else {
            return "kind default".to_owned();
        };
        [self.fresh, self.old]
            .iter()
            .find_map(|context| context.graphs.iter().find(|graph| graph.id == id))
            .map_or_else(
                || "override (this item's)".to_owned(),
                |graph| graph.name.clone(),
            )
    }
}

/// `1 line`, `3 lines`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// D4: the view over the whole tab `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, view: &Divergence, theme: &Theme) {
    let title = header(&view.key, view.ancestor_version, view.head_version);
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = usize::from(inner.width);
    let vw = width.saturating_sub(LABEL_WIDTH + STATE_WIDTH + 2) / 3;
    let headings = format!(
        "{:<LABEL_WIDTH$}{:<STATE_WIDTH$}{:<vw$} {:<vw$} {:<vw$}",
        "field",
        "state",
        clip(&format!("ancestor v{}", view.ancestor_version), vw),
        clip(&format!("theirs v{}", view.head_version), vw),
        clip("mine", vw),
    );

    let mut table: Vec<Line<'static>> = view
        .rows
        .iter()
        .flat_map(|row| row_lines(row, vw, theme))
        .collect();
    let all = u16::try_from(table.len()).unwrap_or(u16::MAX);
    // The headings and the hint take a row each; with a pane, the section row and the diffs'
    // `DIFF_MIN` come off the table's share.
    let rest = inner.height.saturating_sub(2);
    let (table_h, section_h) = match view.pane {
        Some(_) => (all.min(rest.saturating_sub(1 + DIFF_MIN)), 1),
        None => (all.min(rest), 0),
    };
    if table.len() > usize::from(table_h) {
        table.truncate(usize::from(table_h));
        if let Some(last) = table.last_mut() {
            *last = Line::styled("\u{2026}", theme.dim);
        }
    }
    let [headings_at, table_at, section_at, diffs_at, hint_at] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(table_h),
        Constraint::Length(section_h),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(Line::styled(clip(&headings, width), theme.dim)),
        headings_at,
    );
    frame.render_widget(Paragraph::new(table), table_at);

    if let (Some(pane), Some((theirs, mine))) = (view.pane, view.diffs()) {
        let columns = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ]);
        let [left, _, right] = columns.areas(section_at);
        let label = pane.label();
        for (text, at) in [
            (format!("{label}  ancestor \u{2192} theirs"), left),
            (format!("{label}  ancestor \u{2192} mine"), right),
        ] {
            frame.render_widget(
                Paragraph::new(Line::styled(
                    clip(&text, usize::from(at.width)),
                    theme.title,
                )),
                at,
            );
        }
        let [left, gap, right] = columns.areas(diffs_at);
        for (unified, at) in [(theirs, left), (mine, right)] {
            frame.render_widget(
                Paragraph::new(diff::lines(&unified, theme))
                    .wrap(Wrap { trim: false })
                    .scroll((view.scroll, 0)),
                at,
            );
        }
        let bar: Vec<Line<'static>> = (0..gap.height)
            .map(|_| Line::styled("\u{2502}", theme.dim))
            .collect();
        frame.render_widget(Paragraph::new(bar), gap);
    }

    frame.render_widget(
        Paragraph::new(Line::styled(clip(&view.hint(), width), theme.dim)),
        hint_at,
    );
}

/// One row's lines: each value wrapped in its `vw`-wide cell (D4: the short fields wrap, never
/// clip), the label and state on the first line only.
fn row_lines(row: &Row, vw: usize, theme: &Theme) -> Vec<Line<'static>> {
    let cells = [&row.ancestor, &row.theirs, &row.mine].map(|value| notice_lines(value, vw));
    let height = cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let state_style = match row.state {
        FieldState::Conflict => theme.error,
        FieldState::Theirs | FieldState::Mine => theme.accent,
        FieldState::Same => theme.dim,
    };
    (0..height)
        .map(|at| {
            let (label, state) = if at == 0 {
                (row.field.label(), row.state.label())
            } else {
                ("", "")
            };
            let mut spans = vec![
                Span::styled(format!("{label:<LABEL_WIDTH$}"), theme.base),
                Span::styled(format!("{state:<STATE_WIDTH$}"), state_style),
            ];
            for (index, cell) in cells.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::raw(" "));
                }
                let text = cell.get(at).map_or("", String::as_str);
                spans.push(Span::styled(format!("{text:<vw$}"), theme.base));
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item_writes;
    use crate::store_worker::{StoreReply, StoreRequest};
    use crate::ui::layout::chrome;
    use crossterm::event::KeyModifiers;
    use htui_core::fixtures::ids;
    use htui_core::model::{EditReason, ItemPatch, SpecChanges};
    use htui_core::store::{MemStore, WriteStore as _};
    use htui_store::Backend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// A patch through the store itself: "their" edit.
    fn patch() -> ItemPatch {
        ItemPatch {
            author_id: ids::USER,
            reason: "elsewhere".to_owned(),
            ..ItemPatch::default()
        }
    }

    /// The edit form's catalogue on ANA-1 (its item the ancestor), then `theirs` written at v1
    /// and a stale edit served for real: the opened context and the reply's divergence.
    async fn diverged(
        store: &MemStore,
        theirs: impl FnOnce(&ItemSpec) -> ItemPatch,
    ) -> (ItemFormContext, ItemDivergence) {
        let backend = Backend::memory(store.clone());
        let read = StoreRequest::ItemForm {
            project: ids::PROJECT_HTUI,
            item: Some(ids::HTUI_ANA_1),
        };
        let context = match item_writes::serve(&backend, &read).await {
            Ok(StoreReply::ItemForm(context)) => *context,
            other => panic!("the form read answered {other:?}"),
        };
        let opened = ItemSpec::of(context.item.as_ref().expect("an edit context"));
        store
            .update_item(ids::HTUI_ANA_1, 1, theirs(&opened))
            .await
            .expect("their edit");
        let stale = StoreRequest::EditItem {
            id: ids::HTUI_ANA_1,
            expected_version: 1,
            changes: SpecChanges {
                title: Some("Stale".to_owned()),
                ..SpecChanges::default()
            },
            reason: EditReason::Edited,
        };
        match item_writes::serve(&backend, &stale).await {
            Ok(StoreReply::ItemDiverged(divergence)) => (context, *divergence),
            other => panic!("the stale edit answered {other:?}"),
        }
    }

    /// The view over a real divergence: `theirs` written by the store, `mine` the opened spec
    /// with `mine` applied.
    async fn view(
        theirs: impl FnOnce(&ItemSpec) -> ItemPatch,
        mine: impl FnOnce(&mut ItemSpec),
    ) -> Divergence {
        let store = MemStore::demo();
        let (context, divergence) = diverged(&store, theirs).await;
        let ancestor = context.item.clone().expect("an edit context");
        let mut spec = ItemSpec::of(&ancestor);
        mine(&mut spec);
        Divergence::open(&ancestor, &context, spec, &divergence)
    }

    /// The view drawn at 100x30 into the tab area, one string per row of the whole frame.
    fn drawn(view: &Divergence) -> Vec<String> {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30))
            .expect("a test terminal");
        terminal
            .draw(|frame| {
                let area = chrome(frame.area()).body;
                render(frame, area, view, &Theme::default());
            })
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn states(view: &Divergence) -> Vec<(SpecField, FieldState)> {
        view.rows()
            .iter()
            .map(|row| (row.field, row.state))
            .collect()
    }

    #[tokio::test]
    async fn only_the_fields_that_moved_are_rows_each_with_its_state() {
        let view = view(
            |opened| ItemPatch {
                title: Some("Theirs".to_owned()),
                priority: Some(opened.priority + 1),
                ..patch()
            },
            |spec| {
                spec.title = "Mine".to_owned();
                spec.required_tags = vec!["gpu".to_owned(), "zz-mine".to_owned()];
            },
        )
        .await;
        assert_eq!(
            states(&view),
            [
                (SpecField::Title, FieldState::Conflict),
                (SpecField::Priority, FieldState::Theirs),
                (SpecField::Tags, FieldState::Mine),
            ]
        );
        let title = &view.rows()[0];
        assert_eq!(
            (title.theirs.as_str(), title.mine.as_str()),
            ("Theirs", "Mine")
        );
        assert_eq!(view.rows()[2].mine, "gpu, zz-mine");
        assert_eq!(view.pane(), None, "body and paths are both the same");
        assert_eq!((view.ancestor_version(), view.head_version()), (1, 2));
    }

    #[tokio::test]
    async fn kind_and_graph_rows_read_through_the_catalogue() {
        let view = view(
            |_| ItemPatch {
                kind_id: Some(ids::KIND_HTUI_FEAT),
                step_graph_id: Some(Some(ids::GRAPH_HTUI_FEAT)),
                ..patch()
            },
            |_| {},
        )
        .await;
        let feat = view
            .context
            .kinds
            .iter()
            .find(|kind| kind.id == ids::KIND_HTUI_FEAT)
            .expect("the feat kind");
        let graph = view
            .context
            .graphs
            .iter()
            .find(|graph| graph.id == ids::GRAPH_HTUI_FEAT)
            .expect("the feat graph");
        let row = |field| {
            view.rows()
                .iter()
                .find(|row| row.field == field)
                .unwrap_or_else(|| panic!("a {field:?} row"))
        };
        assert_eq!(row(SpecField::Kind).state, FieldState::Theirs);
        assert_eq!(
            row(SpecField::Kind).theirs,
            format!("{} {}", feat.prefix, feat.name)
        );
        assert_eq!(row(SpecField::Graph).ancestor, "kind default");
        assert_eq!(row(SpecField::Graph).mine, "kind default");
        assert_eq!(row(SpecField::Graph).theirs, graph.name);
    }

    #[tokio::test]
    async fn a_body_conflict_draws_both_diffs() {
        let view = view(
            |_| ItemPatch {
                body: Some("one\ntheirs\n".to_owned()),
                ..patch()
            },
            |spec| spec.body = "one\nmine\n".to_owned(),
        )
        .await;
        assert_eq!(states(&view), [(SpecField::Body, FieldState::Conflict)]);
        assert_eq!(view.rows()[0].theirs, "2 lines");
        let rows = drawn(&view);
        let text = rows.join("\n");
        for wanted in [
            "ANA-1  v1 \u{2192} theirs v2",
            "--- ancestor v1",
            "+++ theirs v2",
            "+++ mine",
            "conflict",
            "+theirs",
            "+mine",
        ] {
            assert!(text.contains(wanted), "{wanted:?} in\n{text}");
        }
        for row in &rows {
            assert!(row.chars().count() <= 100, "{row:?}");
        }
    }

    #[tokio::test]
    async fn tab_switches_to_paths_only_when_both_differ() {
        let mut only_body = view(
            |_| ItemPatch {
                body: Some("theirs\n".to_owned()),
                ..patch()
            },
            |_| {},
        )
        .await;
        assert_eq!(only_body.pane(), Some(DiffPane::Body));
        assert_eq!(only_body.on_key(key(KeyCode::Tab)), ViewOutcome::Stay);
        assert_eq!(only_body.pane(), Some(DiffPane::Body), "nowhere to go");

        let mut both = view(
            |_| ItemPatch {
                body: Some("theirs\n".to_owned()),
                ..patch()
            },
            |spec| spec.touched_paths = vec!["src/mine/**".to_owned()],
        )
        .await;
        assert_eq!(both.pane(), Some(DiffPane::Body));
        both.on_key(key(KeyCode::Char('j')));
        assert_eq!(both.scroll(), 1);
        assert_eq!(both.on_key(key(KeyCode::Tab)), ViewOutcome::Stay);
        assert_eq!(both.pane(), Some(DiffPane::Paths));
        assert_eq!(both.scroll(), 0, "a switch starts at the top");
        both.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(both.pane(), Some(DiffPane::Body));
    }

    #[tokio::test]
    async fn scrolling_is_clamped_to_the_longer_diff() {
        let long: String = (0..20).map(|n| format!("their line {n}\n")).collect();
        let mut view = view(
            move |_| ItemPatch {
                body: Some(long),
                ..patch()
            },
            |_| {},
        )
        .await;
        let merge = view.merge().clone();
        let lines = diff::unified(
            &merge.ancestor().body,
            &merge.theirs().body,
            "ancestor v1",
            "theirs v2",
        )
        .lines()
        .count();
        assert!(lines > 1 && lines - 1 < 5 * PAGE, "{lines}");
        for _ in 0..5 {
            assert_eq!(view.on_key(key(KeyCode::PageDown)), ViewOutcome::Stay);
        }
        assert_eq!(usize::from(view.scroll()), lines - 1);
        view.on_key(key(KeyCode::Down));
        assert_eq!(usize::from(view.scroll()), lines - 1, "clamped");
        view.on_key(key(KeyCode::PageUp));
        assert_eq!(usize::from(view.scroll()), lines - 1 - PAGE);
        for _ in 0..3 {
            view.on_key(key(KeyCode::PageUp));
        }
        assert_eq!(view.scroll(), 0);
        view.on_key(key(KeyCode::Char('k')));
        assert_eq!(view.scroll(), 0, "k at the top stays");
    }

    #[tokio::test]
    async fn the_hint_follows_the_conflicts() {
        let conflicted = view(
            |_| ItemPatch {
                title: Some("Theirs".to_owned()),
                body: Some("theirs\n".to_owned()),
                ..patch()
            },
            |spec| {
                spec.title = "Mine".to_owned();
                spec.touched_paths = vec!["src/mine/**".to_owned()];
            },
        )
        .await;
        assert_eq!(
            conflicted.hint(),
            "t theirs wins  m mine wins  Tab body/paths  Esc back"
        );
        let clean = view(
            |_| ItemPatch {
                title: Some("Theirs".to_owned()),
                ..patch()
            },
            |spec| spec.required_tags = vec!["zz-mine".to_owned()],
        )
        .await;
        assert!(
            clean.hint().starts_with(HINT_NO_CONFLICT),
            "{}",
            clean.hint()
        );
        assert!(clean.hint().ends_with(HINT_BACK), "{}", clean.hint());
        assert!(!clean.hint().contains(HINT_TAB), "{}", clean.hint());
    }

    #[test]
    fn the_longest_hint_fits_the_tab_area() {
        let room = usize::from(chrome(Rect::new(0, 0, 100, 30)).body.width - 2);
        for hint in [
            [HINT_SIDES, HINT_TAB, HINT_BACK].join("  "),
            [HINT_NO_CONFLICT, HINT_TAB, HINT_BACK].join("  "),
        ] {
            assert!(hint.chars().count() <= room, "{hint:?} against {room}");
        }
    }

    #[tokio::test]
    async fn a_long_title_wraps_inside_its_column() {
        let title = "A deliberately long title that has to wrap inside its value column, ok";
        assert_eq!(title.chars().count(), 70);
        let view = view(
            |_| ItemPatch {
                title: Some(title.to_owned()),
                ..patch()
            },
            |_| {},
        )
        .await;
        let rows = drawn(&view);
        let inner = usize::from(chrome(Rect::new(0, 0, 100, 30)).body.width) - 2;
        let vw = (inner - LABEL_WIDTH - STATE_WIDTH - 2) / 3;
        let start = 1 + LABEL_WIDTH + STATE_WIDTH + vw + 1;
        let column = rows
            .iter()
            .map(|row| {
                row.chars()
                    .skip(start)
                    .take(vw)
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|cell| !cell.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(column.contains(title), "{column:?} in\n{}", rows.join("\n"));
        for row in &rows {
            assert!(row.chars().count() <= 100, "{row:?}");
        }
    }

    #[tokio::test]
    async fn keys_map_to_outcomes() {
        let mut view = view(
            |_| ItemPatch {
                title: Some("Theirs".to_owned()),
                ..patch()
            },
            |spec| spec.title = "Mine".to_owned(),
        )
        .await;
        assert_eq!(view.on_key(key(KeyCode::Esc)), ViewOutcome::Back);
        assert_eq!(
            view.on_key(key(KeyCode::Char('t'))),
            ViewOutcome::Resolve(Side::Theirs)
        );
        assert_eq!(
            view.on_key(key(KeyCode::Char('m'))),
            ViewOutcome::Resolve(Side::Mine)
        );
        assert_eq!(view.on_key(ctrl('s')), ViewOutcome::Stay);
        assert_eq!(
            view.on_key(KeyEvent::new(
                KeyCode::Char('S'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            ViewOutcome::Stay
        );
        assert_eq!(view.on_key(ctrl('c')), ViewOutcome::Pass);
        assert_eq!(view.on_key(key(KeyCode::Char('x'))), ViewOutcome::Stay);
        assert_eq!(view.on_key(key(KeyCode::Enter)), ViewOutcome::Stay);
    }

    #[tokio::test]
    async fn resolve_hands_back_the_head_context_and_the_merged_spec() {
        let view = view(
            |opened| ItemPatch {
                title: Some("Theirs".to_owned()),
                priority: Some(opened.priority + 1),
                ..patch()
            },
            |spec| spec.title = "Mine".to_owned(),
        )
        .await;
        let merge = view.merge().clone();
        let (context, resolved) = view.resolve(Side::Mine);
        assert_eq!(
            context.item.as_ref().map(|item| item.version),
            Some(2),
            "the head"
        );
        assert_eq!(resolved, merge.resolve(Side::Mine));
        assert_eq!(resolved.title, "Mine");
        assert_eq!(resolved.priority, merge.theirs().priority);
    }

    #[tokio::test]
    async fn debug_prints_no_body_and_no_paths() {
        let view = view(
            |_| ItemPatch {
                body: Some("SECRET-THEIRS".to_owned()),
                ..patch()
            },
            |spec| {
                spec.body = "SECRET-BODY".to_owned();
                spec.touched_paths = vec!["secret/dir/**".to_owned()];
            },
        )
        .await;
        for printed in [format!("{view:?}"), format!("{view:#?}")] {
            for secret in ["SECRET-BODY", "SECRET-THEIRS", "secret/dir"] {
                assert!(!printed.contains(secret), "{secret} in {printed}");
            }
            assert!(printed.contains("ANA-1"), "{printed}");
        }
    }
}
