//! The Backlog tab: the scope's items on the left, the selected item's detail on the right.
//!
//! MOD-1 is a skeleton (PRD scope): the tab reads, groups, folds and scrolls, and does not write
//! anything. Editing is MOD-13, run actions MOD-4, graph traversal MOD-14 (its hop count and the
//! `m` key are the two lines it adds here) — each of them lands as a
//! [`DetailTab`](detail::DetailTab) body or a binding, not as a change here.
//!
//! MOD-13 milestone 1's filters are the exception, because they narrow the list itself: the
//! active [`BacklogFilter`] lives here, `f` opens its [`FilterForm`] as a capturing panel at the
//! bottom of the list pane and `F` clears it (D1).

pub mod detail;
pub mod filter;
pub mod list;

use htui_core::model::{ItemId, ItemSummary, ProjectId, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::{
    BodyTab, DetailRegistry, DocumentsTab, GraphTab, NotesTab, PromptTab, ReqsTab, RunsTab,
};
use crate::ui::tabs::backlog::filter::{BacklogFilter, FilterForm, FormOutcome};
use crate::ui::tabs::backlog::list::{ListView, Selection};
use crate::ui::tabs::registry::{CLOSE_THE_FIELD_FIRST, Tab, TabId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Width of the list pane, as a percentage of the body region.
///
/// The list gets the larger half: it carries the longest strings (`KEY  status  title`, with
/// `awaiting_approval` in it) and it is the pane a reader navigates from.
const LIST_PERCENT: u16 = 55;

/// Width of the detail pane, as a percentage of the body region.
const DETAIL_PERCENT: u16 = 45;

/// How far the link traversal behind the Graph sub-tab reaches: its deepest view (MOD-14 D2).
/// Fetched once per selection and filtered by `+`/`-` locally, so a depth change costs no read and
/// each of the seven reads stays one reply per selection (blueprint C.2).
const HOPS: u8 = GraphTab::MAX_HOPS;

/// `StoreRequest::Items`' name: what a refused list read is answered `Failed` under.
const ITEMS_READ: &str = "items";

/// Refreshes between two `Runs` polls: five of the shell's one-second refreshes, so 5 s (MOD-41
/// plan D16, OQ-3).
const REFRESHES_PER_RUNS_POLL: u32 = 5;

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
    /// Body, Runs, Graph, Docs, Notes, Prompt, Reqs, in that order.
    detail: DetailRegistry,
    /// A reveal waiting for the next `Items` reply (MOD-64 D235), with the key its miss is
    /// reported by.
    pending_reveal: Option<(ItemId, String)>,
    /// The shell's refreshes this tab has seen while active; every
    /// [`REFRESHES_PER_RUNS_POLL`]th is a `Runs` poll (MOD-41 plan D16).
    refreshes: u32,
    /// MOD-13 D1: the active filter, the one the next `Items` read goes out under;
    /// `default()` = the whole scope (D5).
    filter: BacklogFilter,
    /// The filter `items` were read under (MOD-13 review M1): what the title names and what
    /// decides "no match". It catches up with `filter` when an `Items` reply lands (the
    /// staleness gate delivers only the newest read's), and `filter` rolls back to it when that
    /// read is refused.
    shown: BacklogFilter,
    /// The open filter form, capturing every key while `Some`.
    form: Option<FilterForm>,
}

impl Default for BacklogTab {
    fn default() -> Self {
        Self::new()
    }
}

impl BacklogTab {
    /// Identity of the Backlog tab.
    pub const ID: TabId = TabId("backlog");

    /// A Backlog tab with the seven detail sub-tabs registered, in blueprint order; Reqs is
    /// MOD-39 plan P12's, after Prompt.
    #[must_use]
    pub fn new() -> Self {
        let mut detail = DetailRegistry::new();
        detail.register(Box::new(BodyTab::new()));
        detail.register(Box::new(RunsTab::new()));
        detail.register(Box::new(GraphTab::new()));
        detail.register(Box::new(DocumentsTab::new()));
        detail.register(Box::new(NotesTab::new()));
        detail.register(Box::new(PromptTab::new()));
        detail.register(Box::new(ReqsTab::new()));
        Self {
            items: Vec::new(),
            folded: Vec::new(),
            selected: None,
            detail,
            pending_reveal: None,
            refreshes: 0,
            filter: BacklogFilter::default(),
            shown: BacklogFilter::default(),
            form: None,
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
        ctx.request(StoreRequest::ItemRequirements(id));
    }

    /// The rows the list pane draws, which are the only rows the cursor may reach: the one rule
    /// [`list::visible`] gives both (MOD-13 review M2). "Filtered" is the title's own test, a
    /// summary to show.
    fn rows(&self, ctx: &Ctx<'_>) -> Vec<Selection> {
        let filtered = self.shown.summary(ctx.projects).is_some();
        list::visible(&self.items, ctx.projects, &self.folded, filtered)
    }

    /// Moves the cursor `delta` rows, clamped to the ends of the list.
    fn step(&mut self, delta: isize, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = self.index(&rows).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(last);
        self.go(rows.get(next).copied(), ctx);
    }

    /// Moves the cursor to the first or the last row (`g` / `G`).
    fn jump(&mut self, to_end: bool, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let target = if to_end { rows.last() } else { rows.first() };
        self.go(target.copied(), ctx);
    }

    /// Where the cursor sits among the visible rows.
    fn index(&self, rows: &[Selection]) -> Option<usize> {
        rows.iter().position(|row| Some(*row) == self.selected)
    }

    /// Folds or unfolds the selected group. Only a project header row the pane draws folds.
    fn fold(&mut self, ctx: &Ctx<'_>) -> Handled {
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
    fn reselect(&mut self, ctx: &Ctx<'_>) {
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
    fn select_item(&mut self, id: ItemId, project: ProjectId, ctx: &Ctx<'_>) {
        self.folded.retain(|folded| *folded != project);
        self.go(Some(Selection::Item(id)), ctx);
    }

    /// A key while the filter form is open (MOD-13 D1).
    ///
    /// Every plain key is consumed, used by the form or not: the global keymap resolves only what
    /// a tab passes, so a passed `q` would quit, a digit or `Tab` switch tabs, `w` open the
    /// switcher. A chord ([`filter::CHORD`]: any modifier but `SHIFT`) passes, the rule
    /// `TextField::on_key` applies (review L4), so `ctrl-c` still quits and `Ctrl+F` still
    /// searches, and an `Alt` chord is never read as its letter.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        if key.modifiers.intersects(filter::CHORD) {
            return Handled::Pass;
        }
        let Some(form) = self.form.as_mut() else {
            return Handled::Pass;
        };
        match form.on_key(key) {
            FormOutcome::Stay => {}
            FormOutcome::Cancel => self.form = None,
            FormOutcome::Refused(sentence) => ctx.emit(Action::Error(sentence)),
            FormOutcome::Apply(filter) => {
                self.form = None;
                self.apply(filter, ctx);
            }
        }
        Handled::Consumed
    }

    /// Sets the filter and re-reads: one `Items`, the request kind the unfiltered list uses, so
    /// the shell's staleness index supersedes an earlier read (MOD-13 D2). An unchanged filter
    /// still reads.
    fn apply(&mut self, filter: BacklogFilter, ctx: &Ctx<'_>) {
        self.filter = filter;
        // A filtered reply must not decide a reveal sent before it (D251): the item may be
        // hidden. An unfiltered one may, and is the read a reveal miss waits for (review N5).
        if !self.filter.is_empty() {
            self.pending_reveal = None;
        }
        ctx.request(self.filter.to_request(ctx.scope));
    }
}

impl Tab for BacklogTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Backlog"
    }

    /// The list under the active filter (MOD-13 D2); no filter is the whole scope (D5).
    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![self.filter.to_request(scope)]
    }

    /// MOD-13 D6: the filter survives a scope change minus the projects the new scope lacks, so a
    /// gone project cannot silently empty the list. The form closes: its project list was the old
    /// scope's.
    fn on_scope_change(&mut self, scope: &Scope) {
        self.items.clear();
        self.folded.clear();
        self.selected = None;
        self.detail.on_item_change(None);
        self.pending_reveal = None;
        self.filter.retain_projects(scope);
        self.shown.retain_projects(scope);
        self.form = None;
    }

    /// A capturing sub-tab (a typed note, a typed-back key, a `y`/`n`) gets every key first: the
    /// guard names no action (ANA-2 `:1694-1697`), it only stops the list from eating the letters
    /// the sub-tab is waiting for (MOD-4 plan OQ-7).
    /// MOD-22 review M-1: a bracketed paste reaches a capturing sub-tab's field and nothing else;
    /// the list never takes one.
    /// MOD-13 D1: the open filter form captures ahead of the sub-tabs; a paste goes to its tag
    /// field.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(form) = self.form.as_mut() {
            form.on_paste(text);
            return Handled::Consumed;
        }
        self.detail.on_paste(text, ctx)
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // MOD-13 D1: first, so the form owns every letter while it is open. It only opens while
        // no sub-tab captures (`f` is below that guard).
        if self.form.is_some() {
            return self.on_form_key(key, ctx);
        }
        if self.detail.captures_input() {
            return self.detail.on_key(key, ctx);
        }
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
            // MOD-14 D8, `open graph`. Below the capture guard, so an `m` typed into a note stays
            // the note's.
            KeyCode::Char('m') => {
                self.detail.select_id(GraphTab::ID);
            }
            // MOD-13 D1: `f` opens the filter form on the active filter, `F` clears it. A
            // terminal's `F` carries SHIFT; only CONTROL and ALT are diverted above.
            KeyCode::Char('f') => {
                self.form = Some(FilterForm::open(&self.filter, ctx.projects));
            }
            KeyCode::Char('F') => {
                if !self.filter.is_empty() {
                    self.apply(BacklogFilter::default(), ctx);
                }
            }
            // A project header folds; on an item row there is nothing to fold, and `Enter` is
            // the detail pane's — the Runs pane replays the step under its cursor with it
            // (MOD-2 D39). Without this the pane would never see the key at all.
            KeyCode::Enter => {
                return match self.fold(ctx) {
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
            // MOD-13 review M1: the newest read is the only one delivered, and it went out under
            // `filter`.
            self.shown.clone_from(&self.filter);
            // MOD-13 review L5: a fold is the scope's project's, not the reply's, so a filter
            // that hides a project's items keeps its fold. A scope change clears it (D6).
            self.folded.retain(|project| {
                ctx.projects
                    .iter()
                    .any(|scoped| scoped.project_id == *project)
            });
            // MOD-64 D251: a reveal of an item that was not loaded is decided by this list.
            if let Some((id, key)) = self.pending_reveal.take() {
                match self.items.iter().find(|item| item.id == id) {
                    Some(item) => self.select_item(id, item.project_id, ctx),
                    None => ctx.emit(Action::Error(not_in_this_backlog(&key))),
                }
            }
            self.reselect(ctx);
            return;
        }
        // D251, Requirements' rule: a refused read must not leave a reveal armed for a later list.
        if let StoreReply::Failed { request, .. } = reply
            && *request == ITEMS_READ
        {
            self.pending_reveal = None;
            // MOD-13 review M1: the rows on screen are still the ones `shown` read, so the next
            // `f` opens on them rather than on a filter that never arrived.
            self.filter.clone_from(&self.shown);
        }
        self.detail.on_reply(reply, ctx);
    }

    /// MOD-41 plan D16: every fifth refresh (5 s), the selected item's runs are read again while
    /// the detail shows one of them active, so a run another process walks moves on screen. No
    /// frame reaches this process for such a run; MOD-43's `LISTEN` keeps this as its backstop.
    ///
    /// `Runs` only: `RunsTab::on_runs` asks for `RunActions` after every `Runs` reply, so asking
    /// for both would read the verdicts twice. The reply keeps the cursor (MOD-4 D198).
    fn on_refresh(&mut self, ctx: &mut Ctx<'_>) {
        self.refreshes = self.refreshes.wrapping_add(1);
        if !self.refreshes.is_multiple_of(REFRESHES_PER_RUNS_POLL) {
            return;
        }
        if let Some(item) = self.selected_item()
            && self.detail.has_active_run()
        {
            ctx.request(StoreRequest::Runs(item));
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let [left, right] = panes(area);
        // MOD-13 D1: the open form takes the bottom of the list pane.
        let (list_area, form_area) = match self.form {
            Some(_) => {
                let [list, form] =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(filter::FORM_HEIGHT)])
                        .areas(left);
                (list, Some(form))
            }
            None => (left, None),
        };

        // MOD-13 review M1: the filter the rows were read under, not the one still in flight.
        let summary = self.shown.summary(ctx.projects);
        let view = ListView {
            items: &self.items,
            projects: ctx.projects,
            folded: &self.folded,
            selected: self.selected,
            filter: summary.as_deref(),
        };
        list::render(frame, list_area, &view, ctx.theme);
        if let (Some(form), Some(at)) = (&self.form, form_area) {
            filter::render(frame, at, form, ctx.theme);
        }
        detail::render(
            frame,
            right,
            &self.detail,
            self.item().map(|item| item.key.as_str()),
            ctx,
        );
    }

    fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
        let RevealTarget::Item { id, key } = target else {
            return false;
        };
        // A half-typed note or reject reason would be lost by the move (`go` resets the sub-tabs),
        // and so would a half-edited filter.
        if self.form.is_some() || self.detail.captures_input() {
            ctx.emit(Action::Error(CLOSE_THE_FIELD_FIRST.to_owned()));
            return true;
        }
        match self.items.iter().find(|item| item.id == *id) {
            Some(item) => {
                let project = item.project_id;
                self.pending_reveal = None;
                self.select_item(*id, project, ctx);
            }
            // Not loaded, or not in the rows read so far: re-read and decide on arrival (D251).
            // MOD-13 D4: the re-read is unfiltered. The tab cannot tell "hidden by the filter"
            // from "not in the workspace" before the reply, so the filter goes either way, and
            // D251's error then speaks for the whole workspace.
            None => {
                self.filter = BacklogFilter::default();
                self.pending_reveal = Some((*id, key.clone()));
                ctx.request(self.filter.to_request(ctx.scope));
            }
        }
        true
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

/// The Backlog's sentence for a reveal of an item this workspace does not hold (MOD-64 D235, F5).
#[must_use]
pub fn not_in_this_backlog(key: &str) -> String {
    format!("{key} is not in this workspace's backlog")
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crate::testkit::DEFAULT_SIZE;
    use crate::ui::Theme;
    use crate::ui::layout::chrome;
    use crate::ui::tabs::backlog::detail::{DetailId, DetailTab};
    use htui_core::model::ItemFilter;
    use htui_core::store::{MemStore, ReadStore as _};

    /// A sub-tab that records every key it is offered and captures while `capturing` is set (the
    /// `tests/settings.rs` probe, one level down).
    #[derive(Debug)]
    struct CapturingProbe {
        capturing: Rc<Cell<bool>>,
        seen: Rc<RefCell<Vec<KeyCode>>>,
    }

    impl DetailTab for CapturingProbe {
        fn id(&self) -> DetailId {
            DetailId("probe")
        }
        fn title(&self) -> &str {
            "Probe"
        }
        fn on_item_change(&mut self, _item: Option<ItemId>) {}
        fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            self.seen.borrow_mut().push(key.code);
            if self.capturing.get() {
                Handled::Consumed
            } else {
                Handled::Pass
            }
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn captures_input(&self) -> bool {
            self.capturing.get()
        }
    }

    /// MOD-4 plan OQ-7, blueprint D201: while a sub-tab captures, the list's own keys reach it
    /// and the list cursor does not move; once it stops, they are the list's again.
    #[tokio::test]
    async fn a_capturing_sub_tab_gets_the_navigation_keys() {
        let store = MemStore::demo();
        let workspace = store
            .workspaces()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|workspace| workspace.slug == "platform")
            .expect("the demo holds `platform`");
        let scope = Scope::from_workspace(&workspace);
        let projects = store.projects(&scope).await.expect("the projects");
        let items = store
            .items(&scope, &ItemFilter::default())
            .await
            .expect("the items");
        let rows = list::rows(&items, &projects, &[]);
        let first = rows
            .iter()
            .copied()
            .find(|row| matches!(row, Selection::Item(_)))
            .expect("the scope has an item");

        let capturing = Rc::new(Cell::new(true));
        let seen = Rc::default();
        let mut detail = DetailRegistry::new();
        detail.register(Box::new(CapturingProbe {
            capturing: Rc::clone(&capturing),
            seen: Rc::clone(&seen),
        }));
        let mut tab = BacklogTab {
            items,
            folded: Vec::new(),
            selected: Some(first),
            detail,
            pending_reveal: None,
            refreshes: 0,
            filter: BacklogFilter::default(),
            shown: BacklogFilter::default(),
            form: None,
        };

        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::new(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(BacklogTab::ID),
            &emit,
        );
        let keys = [
            KeyCode::Char('j'),
            KeyCode::Char('G'),
            KeyCode::Char('l'),
            KeyCode::Char('['),
            KeyCode::Enter,
        ];
        for code in keys {
            assert_eq!(
                tab.on_key(KeyEvent::from(code), &mut ctx),
                Handled::Consumed,
                "{code:?} is the capturing sub-tab's"
            );
        }
        assert_eq!(*seen.borrow(), keys, "every key reached the sub-tab");
        assert_eq!(
            tab.selected,
            Some(first),
            "and the list cursor did not move"
        );
        assert!(emit.is_empty(), "so no detail read went out");

        capturing.set(false);
        tab.on_key(KeyEvent::from(KeyCode::Char('j')), &mut ctx);
        assert_ne!(
            tab.selected,
            Some(first),
            "a sub-tab that stopped capturing gives `j` back to the list"
        );
        assert_eq!(seen.borrow().len(), keys.len(), "without offering it first");
    }

    /// MOD-64 review 6, Requirements' D251 rule: a refused `Items` read disarms a pending reveal,
    /// so a later list neither jumps to nor reports an item the user has since moved on from.
    #[tokio::test]
    async fn a_refused_items_read_disarms_a_pending_reveal() {
        let store = MemStore::demo();
        let workspace = store
            .workspaces()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|workspace| workspace.slug == "platform")
            .expect("the demo holds `platform`");
        let scope = Scope::from_workspace(&workspace);
        let projects = store.projects(&scope).await.expect("the projects");
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::new(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(BacklogTab::ID),
            &emit,
        );
        let read = StoreRequest::Items {
            scope: scope.clone(),
            filter: ItemFilter::default(),
            ready_here: false,
        };
        assert_eq!(ITEMS_READ, read.name());
        let mut tab = BacklogTab::new();
        let target = RevealTarget::Item {
            id: ItemId::default(),
            key: "GONE-1".to_owned(),
        };
        assert!(tab.reveal(&target, &mut ctx));
        assert!(
            tab.pending_reveal.is_some(),
            "not loaded: the next list decides"
        );
        let _ = emit.take();

        tab.on_reply(
            &StoreReply::Failed {
                request: "items",
                message: "the server went away".to_owned(),
            },
            &mut ctx,
        );
        assert!(tab.pending_reveal.is_none(), "the refusal disarms it");

        tab.on_reply(&StoreReply::Items(Vec::new()), &mut ctx);
        let errors: Vec<Action> = emit
            .take()
            .into_iter()
            .filter(|action| matches!(action, Action::Error(_)))
            .collect();
        assert!(
            errors.is_empty(),
            "a later list reports nothing: {errors:?}"
        );
    }

    /// The Platform scope, its projects and its items, and the first item row (MOD-14 cases).
    async fn platform() -> (
        Scope,
        Vec<htui_core::model::ProjectRef>,
        Vec<ItemSummary>,
        Selection,
    ) {
        let store = MemStore::demo();
        let workspace = store
            .workspaces()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|workspace| workspace.slug == "platform")
            .expect("the demo holds `platform`");
        let scope = Scope::from_workspace(&workspace);
        let projects = store.projects(&scope).await.expect("the projects");
        let items = store
            .items(&scope, &ItemFilter::default())
            .await
            .expect("the items");
        let first = list::rows(&items, &projects, &[])
            .into_iter()
            .find(|row| matches!(row, Selection::Item(_)))
            .expect("the scope has an item");
        (scope, projects, items, first)
    }

    /// MOD-14 D8: `select_id` activates a registered sub-tab by id and leaves an unknown id alone.
    #[test]
    fn select_id_finds_a_registered_sub_tab() {
        let mut detail = BacklogTab::new().detail;
        assert!(detail.select_id(GraphTab::ID));
        assert_eq!(detail.active_id(), Some(GraphTab::ID));
        assert!(!detail.select_id(DetailId("nope")));
        assert_eq!(
            detail.active_id(),
            Some(GraphTab::ID),
            "an unknown id changes nothing"
        );
    }

    /// MOD-14 D8, `open graph`: `m` on the list selects the Graph sub-tab and nothing else.
    #[tokio::test]
    async fn m_on_the_list_opens_the_graph() {
        let (scope, projects, items, first) = platform().await;
        let mut tab = BacklogTab {
            items,
            folded: Vec::new(),
            selected: Some(first),
            detail: BacklogTab::new().detail,
            pending_reveal: None,
            refreshes: 0,
            filter: BacklogFilter::default(),
            shown: BacklogFilter::default(),
            form: None,
        };
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::new(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(BacklogTab::ID),
            &emit,
        );
        assert_ne!(tab.detail.active_id(), Some(GraphTab::ID));
        assert_eq!(
            tab.on_key(KeyEvent::from(KeyCode::Char('m')), &mut ctx),
            Handled::Consumed
        );
        assert_eq!(tab.detail.active_id(), Some(GraphTab::ID));
        assert_eq!(tab.selected, Some(first), "the list cursor did not move");
        assert!(emit.is_empty(), "so no detail read went out");
    }

    /// MOD-14 D8: `m` is below the capture guard, so it is a capturing sub-tab's letter.
    #[tokio::test]
    async fn m_while_a_sub_tab_captures_is_the_sub_tab_s() {
        let (scope, projects, items, first) = platform().await;
        let capturing = Rc::new(Cell::new(true));
        let seen: Rc<RefCell<Vec<KeyCode>>> = Rc::default();
        let mut detail = DetailRegistry::new();
        detail.register(Box::new(CapturingProbe {
            capturing: Rc::clone(&capturing),
            seen: Rc::clone(&seen),
        }));
        detail.register(Box::new(GraphTab::new()));
        let mut tab = BacklogTab {
            items,
            folded: Vec::new(),
            selected: Some(first),
            detail,
            pending_reveal: None,
            refreshes: 0,
            filter: BacklogFilter::default(),
            shown: BacklogFilter::default(),
            form: None,
        };
        let (top_bar, keymap, theme, emit) = (
            TopBarState::default(),
            Keymap::new(),
            Theme::default(),
            Emit::default(),
        );
        let mut ctx = Ctx::new(
            &scope,
            &projects,
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(BacklogTab::ID),
            &emit,
        );
        let m = KeyEvent::from(KeyCode::Char('m'));
        assert_eq!(tab.on_key(m, &mut ctx), Handled::Consumed);
        assert_eq!(*seen.borrow(), [KeyCode::Char('m')], "the probe got it");
        assert_eq!(tab.detail.active_id(), Some(DetailId("probe")));

        capturing.set(false);
        assert_eq!(tab.on_key(m, &mut ctx), Handled::Consumed);
        assert_eq!(tab.detail.active_id(), Some(GraphTab::ID));
        assert_eq!(seen.borrow().len(), 1, "the probe saw nothing new");
    }

    // ---- MOD-13 milestone 1: the filter ------------------------------------------------------

    /// The Platform fixture a `Ctx` borrows from, for the filter cases.
    struct Bench {
        scope: Scope,
        projects: Vec<htui_core::model::ProjectRef>,
        items: Vec<ItemSummary>,
        first: Selection,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        async fn new() -> Self {
            let (scope, projects, items, first) = platform().await;
            Self {
                scope,
                projects,
                items,
                first,
                top_bar: TopBarState::default(),
                keymap: Keymap::new(),
                theme: Theme::default(),
                emit: Emit::default(),
            }
        }

        fn ctx(&self) -> Ctx<'_> {
            Ctx::new(
                &self.scope,
                &self.projects,
                &self.top_bar,
                &self.keymap,
                &self.theme,
                Origin::Tab(BacklogTab::ID),
                &self.emit,
            )
        }

        /// A settled tab over the whole scope, its cursor on the first item.
        fn tab(&self) -> BacklogTab {
            BacklogTab {
                items: self.items.clone(),
                selected: Some(self.first),
                ..BacklogTab::new()
            }
        }

        /// Everything emitted since the last call, drained.
        fn actions(&self) -> Vec<Action> {
            self.emit.take()
        }
    }

    /// The `Items` reads among `actions`.
    fn items_reads(actions: &[Action]) -> Vec<(ItemFilter, bool)> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Store(StoreRequest::Items {
                    filter, ready_here, ..
                }) => Some((filter.clone(), *ready_here)),
                _ => None,
            })
            .collect()
    }

    fn press(tab: &mut BacklogTab, bench: &Bench, code: KeyCode) -> Handled {
        tab.on_key(KeyEvent::from(code), &mut bench.ctx())
    }

    fn done_only() -> BacklogFilter {
        BacklogFilter {
            statuses: vec![htui_core::model::Status::Done],
            ..BacklogFilter::default()
        }
    }

    /// MOD-13 D1: `f` opens the form, and while it is open the list's letters and the global
    /// ones (`q` would quit) are the form's.
    #[tokio::test]
    async fn f_opens_the_filter_form_and_it_captures() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        assert_eq!(
            press(&mut tab, &bench, KeyCode::Char('f')),
            Handled::Consumed
        );
        assert!(tab.form.is_some(), "the form is open");
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('q'),
            KeyCode::Char('3'),
            KeyCode::Tab,
        ] {
            assert_eq!(
                press(&mut tab, &bench, code),
                Handled::Consumed,
                "{code:?} is the form's"
            );
        }
        assert_eq!(
            tab.selected,
            Some(bench.first),
            "the list cursor did not move"
        );
        assert!(bench.actions().is_empty(), "and nothing was read");
    }

    /// MOD-13 D2: applying sends exactly one `Items`, the same request kind the unfiltered list
    /// uses, so the staleness index supersedes it.
    #[tokio::test]
    async fn applying_the_form_sends_one_filtered_items_read() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('f'));
        for _ in 0..5 {
            press(&mut tab, &bench, KeyCode::Char('l'));
        }
        press(&mut tab, &bench, KeyCode::Char(' '));
        assert!(bench.actions().is_empty(), "nothing is read before `Enter`");
        assert_eq!(press(&mut tab, &bench, KeyCode::Enter), Handled::Consumed);

        let actions = bench.actions();
        assert_eq!(
            items_reads(&actions),
            [(
                ItemFilter {
                    statuses: Some(vec![htui_core::model::Status::Done]),
                    ..ItemFilter::default()
                },
                false
            )],
            "{actions:?}"
        );
        assert_eq!(actions.len(), 1, "and nothing else: {actions:?}");
        assert!(tab.form.is_none(), "the form closed");
        assert_eq!(tab.filter, done_only());
    }

    /// MOD-13 D3: a refused tag list goes to the status line and the form stays open to fix it.
    #[tokio::test]
    async fn a_refused_tag_list_keeps_the_form_open_and_reports_it() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('f'));
        press(&mut tab, &bench, KeyCode::Down);
        press(&mut tab, &bench, KeyCode::Down);
        tab.on_key(
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
            &mut bench.ctx(),
        );
        press(&mut tab, &bench, KeyCode::Enter);

        let actions = bench.actions();
        assert!(
            matches!(actions.as_slice(), [Action::Error(_)]),
            "one refusal and no read: {actions:?}"
        );
        assert!(tab.form.is_some(), "the form stays open");
        assert!(tab.filter.is_empty(), "and the filter is untouched");
    }

    /// MOD-13 D5: `F` clears an active filter and reads the whole scope again.
    #[tokio::test]
    async fn shift_f_clears_the_filter_and_re_reads() {
        let bench = Bench::new().await;
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        assert_eq!(
            press(&mut tab, &bench, KeyCode::Char('F')),
            Handled::Consumed
        );
        assert!(tab.filter.is_empty());
        let actions = bench.actions();
        assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
        assert_eq!(actions.len(), 1, "{actions:?}");
    }

    #[tokio::test]
    async fn shift_f_without_a_filter_sends_nothing() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        assert_eq!(
            press(&mut tab, &bench, KeyCode::Char('F')),
            Handled::Consumed
        );
        assert!(bench.actions().is_empty());
    }

    /// MOD-13 D2: a tab activation or refresh reads with the active filter, not the whole scope.
    #[tokio::test]
    async fn wants_requests_carries_the_active_filter() {
        let bench = Bench::new().await;
        let tab = BacklogTab {
            filter: BacklogFilter {
                tags: vec!["rust".to_owned()],
                ready_here: true,
                ..BacklogFilter::default()
            },
            ..bench.tab()
        };
        let requests: Vec<Action> = tab
            .wants_requests(&bench.scope)
            .into_iter()
            .map(Action::Store)
            .collect();
        assert_eq!(
            items_reads(&requests),
            [(
                ItemFilter {
                    tags: Some(vec!["rust".to_owned()]),
                    ..ItemFilter::default()
                },
                true
            )]
        );
        assert_eq!(requests.len(), 1);
        assert!(
            BacklogTab::new()
                .wants_requests(&bench.scope)
                .iter()
                .all(|request| matches!(
                    request,
                    StoreRequest::Items { filter, ready_here: false, .. }
                        if *filter == ItemFilter::default()
                )),
            "no filter is the pre-MOD-13 read (D5)"
        );
    }

    /// MOD-13 D4: a reveal of an item the filter hides clears the filter and re-reads, so D251's
    /// "not in this backlog" only fires for an item the workspace does not hold.
    #[tokio::test]
    async fn a_reveal_of_a_hidden_item_clears_the_filter() {
        let bench = Bench::new().await;
        let done: Vec<ItemSummary> = bench
            .items
            .iter()
            .filter(|item| item.status == htui_core::model::Status::Done)
            .cloned()
            .collect();
        assert!(
            done.iter()
                .all(|item| item.id != htui_core::fixtures::ids::HTUI_ANA_2)
        );
        let mut tab = BacklogTab {
            items: done,
            filter: done_only(),
            ..bench.tab()
        };
        let target = RevealTarget::Item {
            id: htui_core::fixtures::ids::HTUI_ANA_2,
            key: "ANA-2".to_owned(),
        };
        assert!(tab.reveal(&target, &mut bench.ctx()));
        assert!(tab.filter.is_empty(), "the filter is cleared");
        assert!(tab.pending_reveal.is_some(), "the next list decides");
        let actions = bench.actions();
        assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);

        tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
        assert_eq!(
            tab.selected,
            Some(Selection::Item(htui_core::fixtures::ids::HTUI_ANA_2))
        );
        assert!(
            !bench
                .actions()
                .iter()
                .any(|action| matches!(action, Action::Error(_))),
            "the whole list holds it, so nothing is reported"
        );
    }

    /// A reveal would drop the half-edited form, as it would a half-typed note.
    #[tokio::test]
    async fn a_reveal_while_the_form_is_open_asks_to_close_it_first() {
        let bench = Bench::new().await;
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        press(&mut tab, &bench, KeyCode::Char('f'));
        let target = RevealTarget::Item {
            id: htui_core::fixtures::ids::HTUI_ANA_2,
            key: "ANA-2".to_owned(),
        };
        assert!(tab.reveal(&target, &mut bench.ctx()));
        let actions = bench.actions();
        assert!(
            matches!(actions.as_slice(), [Action::Error(sentence)] if sentence == CLOSE_THE_FIELD_FIRST),
            "{actions:?}"
        );
        assert!(tab.form.is_some());
        assert_eq!(tab.filter, done_only(), "the filter stays");
        assert!(tab.pending_reveal.is_none());
    }

    /// MOD-13 blueprint §6: a filtered reply must not decide a reveal sent before it.
    #[tokio::test]
    async fn applying_a_filter_disarms_a_pending_reveal() {
        let bench = Bench::new().await;
        let mut tab = BacklogTab::new();
        let target = RevealTarget::Item {
            id: ItemId::default(),
            key: "GONE-1".to_owned(),
        };
        assert!(tab.reveal(&target, &mut bench.ctx()));
        assert!(tab.pending_reveal.is_some());
        let _ = bench.actions();

        press(&mut tab, &bench, KeyCode::Char('f'));
        press(&mut tab, &bench, KeyCode::Char(' '));
        press(&mut tab, &bench, KeyCode::Enter);
        assert!(tab.pending_reveal.is_none(), "the filter disarmed it");
        let _ = bench.actions();

        tab.on_reply(&StoreReply::Items(Vec::new()), &mut bench.ctx());
        assert!(
            !bench
                .actions()
                .iter()
                .any(|action| matches!(action, Action::Error(_))),
            "the filtered list reports nothing"
        );
    }

    /// MOD-13 review N5: applying no filter right after a reveal miss keeps the reveal armed. The
    /// unfiltered reply it waits for can still decide it.
    #[tokio::test]
    async fn applying_no_filter_keeps_a_pending_reveal() {
        let bench = Bench::new().await;
        let mut tab = BacklogTab::new();
        let target = RevealTarget::Item {
            id: htui_core::fixtures::ids::HTUI_ANA_2,
            key: "ANA-2".to_owned(),
        };
        assert!(tab.reveal(&target, &mut bench.ctx()));
        press(&mut tab, &bench, KeyCode::Char('f'));
        press(&mut tab, &bench, KeyCode::Enter);
        assert!(tab.pending_reveal.is_some(), "still armed");

        tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
        assert_eq!(
            tab.selected,
            Some(Selection::Item(htui_core::fixtures::ids::HTUI_ANA_2)),
            "the reply decided it"
        );
    }

    /// MOD-13 review L4: the open form passes every chord but `SHIFT`, as `TextField` does, so an
    /// `Alt` chord reaches the shell instead of acting as its letter.
    #[tokio::test]
    async fn an_alt_chord_passes_through_the_open_form() {
        let bench = Bench::new().await;
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        press(&mut tab, &bench, KeyCode::Char('f'));
        for modifier in [
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::META,
            KeyModifiers::HYPER,
        ] {
            assert_eq!(
                tab.on_key(
                    KeyEvent::new(KeyCode::Char('x'), modifier),
                    &mut bench.ctx()
                ),
                Handled::Pass,
                "{modifier:?}"
            );
        }
        press(&mut tab, &bench, KeyCode::Enter);
        assert_eq!(tab.filter, done_only(), "no chord cleared the draft");
    }

    /// MOD-13 D6: a scope change keeps the filter minus the projects the new scope lacks, and
    /// closes the form, whose project list belonged to the old scope.
    #[tokio::test]
    async fn a_scope_change_drops_the_projects_it_lacks_and_closes_the_form() {
        use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
        let bench = Bench::new().await;
        let htui_only = Scope {
            workspace_id: bench.scope.workspace_id,
            project_ids: vec![PROJECT_HTUI],
        };
        let mut tab = BacklogTab {
            filter: BacklogFilter {
                projects: vec![PROJECT_HTUI, PROJECT_AGY],
                ..done_only()
            },
            ..bench.tab()
        };
        press(&mut tab, &bench, KeyCode::Char('f'));
        tab.on_scope_change(&htui_only);
        assert_eq!(tab.filter.projects, [PROJECT_HTUI]);
        assert_eq!(tab.filter.statuses, done_only().statuses, "status is kept");
        assert!(tab.form.is_none(), "the form closed");

        let mut agy = BacklogTab {
            filter: BacklogFilter {
                projects: vec![PROJECT_AGY],
                ..BacklogFilter::default()
            },
            ..bench.tab()
        };
        agy.on_scope_change(&htui_only);
        assert!(
            agy.filter.is_empty(),
            "a filter naming only a gone project is no filter, not an empty list"
        );
    }

    /// A filtered reply without the cursor's row moves the cursor to its first item, which
    /// re-reads that item's detail.
    #[tokio::test]
    async fn a_filtered_reply_moves_the_cursor_to_its_first_item() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let others: Vec<ItemSummary> = bench
            .items
            .iter()
            .filter(|item| item.id != first)
            .cloned()
            .collect();
        let expected = list::rows(&others, &bench.projects, &[])
            .into_iter()
            .find(|row| matches!(row, Selection::Item(_)))
            .expect("another item");
        tab.on_reply(&StoreReply::Items(others), &mut bench.ctx());
        assert_eq!(tab.selected, Some(expected));
        let Selection::Item(id) = expected else {
            unreachable!()
        };
        assert!(
            bench.actions().iter().any(
                |action| matches!(action, Action::Store(StoreRequest::Item(read)) if *read == id)
            ),
            "the new row's detail is read"
        );
    }

    /// MOD-52: `ctrl-c` quits from inside the form, as it does from every text field.
    #[tokio::test]
    async fn ctrl_c_passes_through_the_open_form() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('f'));
        assert_eq!(
            tab.on_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &mut bench.ctx()
            ),
            Handled::Pass
        );
        assert!(tab.form.is_some());
    }

    /// MOD-22 review M-1: a paste reaches the open form's tag field, not the detail pane.
    #[tokio::test]
    async fn a_paste_while_the_form_is_open_goes_to_its_tag_field() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('f'));
        press(&mut tab, &bench, KeyCode::Down);
        press(&mut tab, &bench, KeyCode::Down);
        assert_eq!(
            tab.on_paste("rust, gpu", &mut bench.ctx()),
            Handled::Consumed
        );
        press(&mut tab, &bench, KeyCode::Enter);
        assert_eq!(tab.filter.tags, ["gpu", "rust"]);
    }

    /// The tab drawn at the harness's size, as snapshot text.
    fn drawn(tab: &BacklogTab, bench: &Bench) -> String {
        let (width, height) = DEFAULT_SIZE;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), &bench.ctx()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The rows of `bench` with `status`.
    fn with_status(bench: &Bench, status: htui_core::model::Status) -> Vec<ItemSummary> {
        bench
            .items
            .iter()
            .filter(|item| item.status == status)
            .cloned()
            .collect()
    }

    /// MOD-13 review M1: the title names the filter the rows were read under. A refused read
    /// leaves the old rows under the old title, and the filter rolls back to it, so the next `f`
    /// opens on what is on screen.
    #[tokio::test]
    async fn a_refused_filtered_read_keeps_the_title_and_rolls_the_filter_back() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        tab.apply(done_only(), &bench.ctx());
        let pending = drawn(&tab, &bench);
        assert!(
            pending.contains("Backlog (11) ") && !pending.contains("status:done"),
            "the reply has not arrived, so the title is the whole list's:\n{pending}"
        );

        tab.on_reply(
            &StoreReply::Failed {
                request: ITEMS_READ,
                message: "the server went away".to_owned(),
            },
            &mut bench.ctx(),
        );
        let refused = drawn(&tab, &bench);
        assert!(
            refused.contains("Backlog (11) ") && !refused.contains("status:done"),
            "{refused}"
        );
        assert_eq!(tab.filter, tab.shown, "the filter is what is shown");
        assert!(tab.filter.is_empty());
    }

    /// MOD-13 review M1: the reply to a filtered read brings its filter into the title.
    #[tokio::test]
    async fn a_filtered_reply_names_its_filter_in_the_title() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        tab.apply(done_only(), &bench.ctx());
        tab.on_reply(
            &StoreReply::Items(with_status(&bench, htui_core::model::Status::Done)),
            &mut bench.ctx(),
        );
        assert_eq!(tab.shown, done_only());
        let frame = drawn(&tab, &bench);
        assert!(frame.contains("Backlog (2) · status:done "), "{frame}");
    }

    /// MOD-13 review M2: a filter nothing matches draws no row, so there is no row to select,
    /// fold or step over; `F` then brings the whole list back with nothing folded.
    #[tokio::test]
    async fn a_filter_nothing_matches_leaves_no_row_to_move_or_fold() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        let cuda = BacklogFilter {
            tags: vec!["cuda".to_owned()],
            ..BacklogFilter::default()
        };
        tab.apply(cuda, &bench.ctx());
        tab.on_reply(&StoreReply::Items(Vec::new()), &mut bench.ctx());
        assert_eq!(tab.selected, None, "no row, no cursor");
        assert!(drawn(&tab, &bench).contains(list::NO_MATCH));
        let _ = bench.actions();

        for code in [
            KeyCode::Enter,
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('G'),
            KeyCode::Char('g'),
        ] {
            press(&mut tab, &bench, code);
            assert_eq!(tab.selected, None, "{code:?} found no row");
        }
        assert!(tab.folded.is_empty(), "`Enter` folded nothing");
        assert!(bench.actions().is_empty(), "and nothing was read");

        press(&mut tab, &bench, KeyCode::Char('F'));
        tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
        assert!(tab.folded.is_empty(), "no project comes back folded");
        assert_eq!(tab.selected, Some(bench.first));
    }

    /// MOD-13 review L5: a fold belongs to the scope's project, not to the rows one read holds,
    /// so a filter that hides the project's items does not forget it.
    #[tokio::test]
    async fn a_fold_survives_a_filter_that_hides_its_project() {
        use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
        let bench = Bench::new().await;
        let mut tab = BacklogTab {
            folded: vec![PROJECT_AGY],
            ..bench.tab()
        };
        let htui_only = BacklogFilter {
            projects: vec![PROJECT_HTUI],
            ..BacklogFilter::default()
        };
        tab.apply(htui_only, &bench.ctx());
        let htui: Vec<ItemSummary> = bench
            .items
            .iter()
            .filter(|item| item.project_id == PROJECT_HTUI)
            .cloned()
            .collect();
        tab.on_reply(&StoreReply::Items(htui), &mut bench.ctx());
        assert_eq!(tab.folded, [PROJECT_AGY]);

        press(&mut tab, &bench, KeyCode::Char('F'));
        tab.on_reply(&StoreReply::Items(bench.items.clone()), &mut bench.ctx());
        assert_eq!(tab.folded, [PROJECT_AGY], "still folded after `F`");
    }

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
