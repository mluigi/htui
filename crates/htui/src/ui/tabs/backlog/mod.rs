//! The Backlog tab: the scope's items on the left, the selected item's detail on the right.
//!
//! MOD-1 is a skeleton (PRD scope): the tab reads, groups, folds and scrolls, and does not write
//! anything. Filters and editing are MOD-13, run actions MOD-4, graph traversal MOD-14 — each of
//! them lands as a [`DetailTab`](detail::DetailTab) body or a binding, not as a change here.

pub mod detail;
pub mod list;

use htui_core::model::{ItemFilter, ItemId, ItemSummary, ProjectId, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{Ctx, Handled};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::{
    BodyTab, DetailRegistry, DocumentsTab, GraphTab, NotesTab, PromptTab, RunsTab,
};
use crate::ui::tabs::backlog::list::{ListView, Selection};
use crate::ui::tabs::registry::{Tab, TabId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Width of the list pane, as a percentage of the body region.
///
/// The list gets the larger half: it carries the longest strings (`KEY  status  title`, with
/// `awaiting_approval` in it) and it is the pane a reader navigates from.
const LIST_PERCENT: u16 = 55;

/// Width of the detail pane, as a percentage of the body region.
const DETAIL_PERCENT: u16 = 45;

/// How far the link traversal of the Graph sub-tab reaches (plan D11).
const HOPS: u8 = 1;

/// The Backlog screen.
///
/// Holds no store handle and no channel (`R-NF-3`): rows arrive through [`Tab::on_reply`] and
/// every read leaves through `ctx.request`.
#[derive(Debug)]
pub struct BacklogTab {
    /// The scope's items, as the `Items` reply delivered them.
    items: Vec<ItemSummary>,
    /// Projects whose group is folded.
    folded: Vec<ProjectId>,
    /// The cursor, or `None` before the first reply.
    selected: Option<Selection>,
    /// Body, Runs, Graph, Documents, Notes, Prompt, in that order.
    detail: DetailRegistry,
}

impl Default for BacklogTab {
    fn default() -> Self {
        Self::new()
    }
}

impl BacklogTab {
    /// Identity of the Backlog tab.
    pub const ID: TabId = TabId("backlog");

    /// A Backlog tab with the six detail sub-tabs registered, in blueprint order.
    #[must_use]
    pub fn new() -> Self {
        let mut detail = DetailRegistry::new();
        detail.register(Box::new(BodyTab::new()));
        detail.register(Box::new(RunsTab::new()));
        detail.register(Box::new(GraphTab::new()));
        detail.register(Box::new(DocumentsTab::new()));
        detail.register(Box::new(NotesTab::new()));
        detail.register(Box::new(PromptTab::new()));
        Self {
            items: Vec::new(),
            folded: Vec::new(),
            selected: None,
            detail,
        }
    }

    /// The selected item, when the cursor is on an item row rather than a project header.
    fn item(&self) -> Option<&ItemSummary> {
        match self.selected? {
            Selection::Item(id) => self.items.iter().find(|item| item.id == id),
            Selection::Project(_) => None,
        }
    }

    /// Id of the selected item, or `None` while a project header is selected.
    ///
    /// The one thing the modules on top of this one (MOD-13's editor, MOD-4's run actions) need
    /// to ask the tab.
    #[must_use]
    pub fn selected_item(&self) -> Option<ItemId> {
        self.item().map(|item| item.id)
    }

    /// Moves the cursor to a row and, when that is a different item, re-reads its detail.
    ///
    /// The six reads go out together rather than per sub-tab, so switching sub-tabs costs
    /// nothing and no sub-tab has to know when it became visible. They are six distinct request
    /// kinds, so the shell's staleness index keeps them apart and a reply to the previous
    /// selection is dropped rather than shown under the new one (blueprint C.2).
    ///
    /// The sixth is MOD-2 milestone 9's preview (plan D102). It goes out with the other five even
    /// though it is the expensive one, for the same reason they do: the pane must be populated
    /// before it is looked at, and the work happens on a task of its own either way (`R-NF-3`).
    fn go(&mut self, next: Option<Selection>, ctx: &Ctx<'_>) {
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
    }

    /// Moves the cursor `delta` rows, clamped to the ends of the list.
    fn step(&mut self, delta: isize, ctx: &Ctx<'_>) {
        let rows = list::rows(&self.items, ctx.projects, &self.folded);
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = self.index(&rows).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(last);
        self.go(rows.get(next).copied(), ctx);
    }

    /// Moves the cursor to the first or the last row (`g` / `G`).
    fn jump(&mut self, to_end: bool, ctx: &Ctx<'_>) {
        let rows = list::rows(&self.items, ctx.projects, &self.folded);
        let target = if to_end { rows.last() } else { rows.first() };
        self.go(target.copied(), ctx);
    }

    /// Where the cursor sits among the visible rows.
    fn index(&self, rows: &[Selection]) -> Option<usize> {
        rows.iter().position(|row| Some(*row) == self.selected)
    }

    /// Folds or unfolds the selected group. Only a project header row folds.
    fn fold(&mut self) -> Handled {
        let Some(Selection::Project(id)) = self.selected else {
            return Handled::Pass;
        };
        if let Some(at) = self.folded.iter().position(|folded| *folded == id) {
            self.folded.remove(at);
        } else {
            self.folded.push(id);
        }
        Handled::Consumed
    }

    /// Keeps the cursor on a row that still exists after a new `Items` reply, preferring the
    /// first item over the project header it sits under.
    fn reselect(&mut self, ctx: &Ctx<'_>) {
        let rows = list::rows(&self.items, ctx.projects, &self.folded);
        if self.index(&rows).is_some() {
            return;
        }
        let first_item = rows
            .iter()
            .find(|row| matches!(row, Selection::Item(_)))
            .or_else(|| rows.first());
        self.go(first_item.copied(), ctx);
    }
}

impl Tab for BacklogTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Backlog"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Items {
            scope: scope.clone(),
            filter: ItemFilter::default(),
        }]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.items.clear();
        self.folded.clear();
        self.selected = None;
        self.detail.on_item_change(None);
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return self.detail.on_key(key, ctx);
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.step(1, ctx),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1, ctx),
            KeyCode::Char('g') | KeyCode::Home => self.jump(false, ctx),
            KeyCode::Char('G') | KeyCode::End => self.jump(true, ctx),
            KeyCode::Char('l') | KeyCode::Char(']') | KeyCode::Right => self.detail.cycle_next(),
            KeyCode::Char('h') | KeyCode::Char('[') | KeyCode::Left => self.detail.cycle_prev(),
            // A project header folds; on an item row there is nothing to fold, and `Enter` is
            // the detail pane's — the Runs pane replays the step under its cursor with it
            // (MOD-2 D39). Without this the pane would never see the key at all.
            KeyCode::Enter => {
                return match self.fold() {
                    Handled::Consumed => Handled::Consumed,
                    Handled::Pass => self.detail.on_key(key, ctx),
                };
            }
            _ => return self.detail.on_key(key, ctx),
        }
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        if let StoreReply::Items(items) = reply {
            self.items.clone_from(items);
            let live: Vec<ProjectId> = self.items.iter().map(|item| item.project_id).collect();
            self.folded.retain(|project| live.contains(project));
            self.reselect(ctx);
            return;
        }
        self.detail.on_reply(reply, ctx);
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let [left, right] = panes(area);

        let view = ListView {
            items: &self.items,
            projects: ctx.projects,
            folded: &self.folded,
            selected: self.selected,
        };
        list::render(frame, left, &view, ctx.theme);
        detail::render(
            frame,
            right,
            &self.detail,
            self.item().map(|item| item.key.as_str()),
            ctx,
        );
    }
}

/// Splits the body region into the list pane and the detail pane.
fn panes(area: Rect) -> [Rect; 2] {
    Layout::horizontal([
        Constraint::Percentage(LIST_PERCENT),
        Constraint::Percentage(DETAIL_PERCENT),
    ])
    .areas(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::DEFAULT_SIZE;
    use crate::ui::Theme;
    use crate::ui::layout::chrome;

    /// The sub-tab strip fits inside the detail pane at the harness's pinned size, so a longer title, another sub-tab or a narrower pane fails here
    /// instead of being re-accepted as a clipped snapshot (MOD-30).
    #[test]
    fn the_detail_strip_fits_the_detail_pane() {
        let (width, height) = DEFAULT_SIZE;
        let body = chrome(Rect::new(0, 0, width, height)).body;
        let [_, right] = panes(body);
        let inner = detail::frame_block(None).inner(right);
        let tab = BacklogTab::new();
        let strip = detail::strip_line(&tab.detail, &Theme::default());
        assert!(
            strip.width() <= usize::from(inner.width),
            "the strip is {} columns against a {}-column pane",
            strip.width(),
            inner.width
        );
    }
}
