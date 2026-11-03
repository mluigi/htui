//! The Backlog's selection (MOD-115 M4 T1): the cursor over the list's rows, the per-item
//! reads a move sends, the fold, a reveal's Runs focus, and the `TabLoc` cursor format.

use htui_core::model::{ItemId, ProjectId, RunId, StepId};

use crate::app::{Ctx, Handled};
use crate::store_worker::StoreRequest;
use crate::ui::tabs::backlog::BacklogTab;
use crate::ui::tabs::backlog::detail::{GraphTab, RunsTab};
use crate::ui::tabs::backlog::list::{self, Selection};

/// How a [`Selection`] is written into a [`TabLoc`] cursor: `item:{id}:{key}` (the key names a
/// miss, as a reveal's does) or `project:{id}` (MOD-115 M2 §8).
pub(super) const ITEM_CURSOR: &str = "item";

/// The project-header half of the cursor format beside [`ITEM_CURSOR`].
pub(super) const PROJECT_CURSOR: &str = "project";

/// How far the link traversal behind the Graph sub-tab reaches: its deepest view (MOD-14 D2).
/// Fetched once per selection and filtered by `+`/`-` locally, so a depth change costs no read and
/// each of the seven reads stays one reply per selection (blueprint C.2).
pub(super) const HOPS: u8 = GraphTab::MAX_HOPS;

impl BacklogTab {
    /// Moves the cursor to a row and, when that is a different item, re-reads its detail.
    ///
    /// The seven reads go out together rather than per sub-tab, so switching sub-tabs costs
    /// nothing and no sub-tab has to know when it became visible. They are seven distinct request
    /// kinds, so the shell's staleness index keeps them apart and a reply to the previous
    /// selection is dropped rather than shown under the new one (blueprint C.2).
    ///
    /// The sixth is MOD-2 milestone 9's preview (plan D102). It goes out with the others even
    /// though it is the expensive one, for the same reason they do: the pane must be populated
    /// before it is looked at, and the work happens on a task of its own either way (`R-NF-3`).
    ///
    /// The seventh is MOD-39 plan P12's: the item's requirement citations for the Reqs sub-tab.
    pub(super) fn go(&mut self, next: Option<Selection>, ctx: &Ctx<'_>) {
        if next == self.selected {
            return;
        }
        self.selected = next;
        let item = match next {
            Some(Selection::Item(id)) => Some(id),
            _ => None,
        };
        self.detail.on_item_change(item);
        let Some(id) = item else { return };
        self.read_item(id, ctx);
    }

    /// The seven per-item reads [`go`](Self::go) sends, factored out so an applied edit can send
    /// them for an unchanged selection (MOD-13 milestone 2 A5). It does not reset the sub-tabs:
    /// they keep their state and take the fresh replies.
    pub(super) fn read_item(&self, id: ItemId, ctx: &Ctx<'_>) {
        ctx.request(StoreRequest::Item(id));
        ctx.request(StoreRequest::Runs(id));
        ctx.request(StoreRequest::Links { id, hops: HOPS });
        ctx.request(StoreRequest::Documents(id));
        ctx.request(StoreRequest::Notes(id));
        ctx.request(StoreRequest::PromptPreview {
            item: id,
            template_name: None,
            scope: ctx.scope.clone(),
        });
        ctx.request(StoreRequest::ItemRequirements(id));
    }

    /// The rows the list pane draws, which are the only rows the cursor may reach: the one rule
    /// [`list::visible`] gives both (MOD-13 review M2). "Filtered" is the title's own test, a
    /// summary to show.
    pub(super) fn rows(&self, ctx: &Ctx<'_>) -> Vec<Selection> {
        let filtered = self.shown.summary(ctx.projects).is_some();
        list::visible(&self.items, ctx.projects, &self.folded, filtered)
    }

    /// Moves the cursor `delta` rows, clamped to the ends of the list.
    pub(super) fn step(&mut self, delta: isize, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = self.index(&rows).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(last);
        self.go(rows.get(next).copied(), ctx);
    }

    /// Moves the cursor to the first or the last row (`g` / `G`).
    pub(super) fn jump(&mut self, to_end: bool, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let target = if to_end { rows.last() } else { rows.first() };
        self.go(target.copied(), ctx);
    }

    /// Where the cursor sits among the visible rows.
    pub(super) fn index(&self, rows: &[Selection]) -> Option<usize> {
        rows.iter().position(|row| Some(*row) == self.selected)
    }

    /// Folds or unfolds the selected group. Only a project header row the pane draws folds.
    pub(super) fn fold(&mut self, ctx: &Ctx<'_>) -> Handled {
        let Some(Selection::Project(id)) = self.selected else {
            return Handled::Pass;
        };
        if self.index(&self.rows(ctx)).is_none() {
            return Handled::Pass;
        }
        if let Some(at) = self.folded.iter().position(|folded| *folded == id) {
            self.folded.remove(at);
        } else {
            self.folded.push(id);
        }
        Handled::Consumed
    }

    /// Keeps the cursor on a row that still exists after a new `Items` reply, preferring the
    /// first item over the project header it sits under.
    /// A filter nothing matches has no row, so the cursor goes to `None` and the detail resets
    /// (MOD-13 review M2).
    pub(super) fn reselect(&mut self, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        if self.index(&rows).is_some() {
            return;
        }
        let first_item = rows
            .iter()
            .find(|row| matches!(row, Selection::Item(_)))
            .or_else(|| rows.first());
        self.go(first_item.copied(), ctx);
    }

    /// Unfolds `project` and moves the cursor to `id`, reading its detail (MOD-64 D235).
    pub(super) fn select_item(&mut self, id: ItemId, project: ProjectId, ctx: &Ctx<'_>) {
        self.folded.retain(|folded| *folded != project);
        self.go(Some(Selection::Item(id)), ctx);
    }

    /// MOD-69 plan D8: opens the Runs sub-tab and hands it the reveal's run and step. The item is
    /// already selected; `was_selected` says `go` short-circuited, so no `Runs` read went out and
    /// one is asked here for the pane to apply the focus to (blueprint A-5).
    pub(super) fn focus_runs(
        &mut self,
        id: ItemId,
        run: Option<RunId>,
        step: Option<StepId>,
        was_selected: bool,
        ctx: &Ctx<'_>,
    ) {
        self.detail.select_id(RunsTab::ID);
        if run.is_none() && step.is_none() {
            return; // a Reopen row: the item and its Runs pane are the target
        }
        self.detail.focus(RunsTab::ID, run, step, ctx);
        if was_selected {
            ctx.request(StoreRequest::Runs(id));
        }
    }
}

/// The [`Selection`] a [`TabLoc`] cursor written by [`BacklogTab`]'s `location` names, with an
/// item's key (empty for a project header); `None` for any other string (MOD-115 M2 §8).
pub(super) fn parse_cursor(cursor: &str) -> Option<(Selection, String)> {
    let mut parts = cursor.splitn(3, ':');
    match (parts.next()?, parts.next()?) {
        (ITEM_CURSOR, id) => Some((
            Selection::Item(id.parse().ok()?),
            parts.next().unwrap_or_default().to_owned(),
        )),
        (PROJECT_CURSOR, id) => Some((Selection::Project(id.parse().ok()?), String::new())),
        _ => None,
    }
}
