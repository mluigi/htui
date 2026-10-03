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
//!
//! MOD-13 milestone 2 adds the writes: `N` opens an [`ItemForm`] for a new item and `e` one for
//! the selected item, drawn in the detail pane's place (D8). Both first read the form's catalogue
//! through the worker (D3), which refuses offline (D2). An applied edit re-reads the item and the
//! list (D9); an applied mint is revealed, which clears the filter (A5). A mint's `Failed` that may
//! follow a COMMIT is hedged and the whole list re-read (D11, §10).
//!
//! A stale edit opens the three-way view in the whole tab area (milestone 3). `m`/`t` rebase the
//! form on the head, and Ctrl+S then lands a `divergence_resolution` revision.
//!
//! Milestone 4: Ctrl+E in the item form's body or paths hands that text to `$EDITOR` (MOD-9's
//! handoff). The outcome comes back through `Tab::on_external_edit` to the form, and Ctrl+S saves
//! it as before.

pub mod detail;
pub mod divergence;
pub mod filter;
pub mod item_form;
pub mod list;

use htui_core::model::{ItemId, ItemKindId, ItemSummary, ProjectId, Scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::editor::ExternalEditOutcome;
use crate::item_writes::{self, ItemDivergence, ItemFormContext, ItemWrite};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::{
    BodyTab, DetailRegistry, DocumentsTab, GraphTab, NotesTab, PromptTab, ReqsTab, RunsTab,
};
use crate::ui::tabs::backlog::filter::{BacklogFilter, FilterForm, FormOutcome};
use crate::ui::tabs::backlog::item_form::{
    Busy, ItemForm, ItemFormOutcome, mint_may_have_landed, mint_may_have_landed_in_the_old_scope,
};
use crate::ui::tabs::backlog::list::{ListView, Selection};
use crate::ui::tabs::registry::{CLOSE_THE_FIELD_FIRST, Tab, TabId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

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
    /// MOD-13 milestone 2 D8 (A7): the open item form, capturing every key but chords while
    /// `Some`.
    item_form: Option<ItemForm>,
    /// Blueprint E8: the `ItemForm` read `N`/`e` is waiting on; a reply opens a form only when it
    /// answers this.
    opening: Option<Opening>,
}

/// What `N` or `e` asked for, so only the answer to it opens a form (blueprint E8): a tab's
/// staleness entry survives a scope change, so a late reply could otherwise open on the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Opening {
    /// The project the read went out for.
    project: ProjectId,
    /// The edited item; `None` for `N`.
    item: Option<ItemId>,
    /// `N`'s kind hint: the selected item's kind (§9 decision 4).
    kind: Option<ItemKindId>,
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
            item_form: None,
            opening: None,
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
        self.read_item(id, ctx);
    }

    /// The seven per-item reads [`go`](Self::go) sends, factored out so an applied edit can send
    /// them for an unchanged selection (MOD-13 milestone 2 A5). It does not reset the sub-tabs:
    /// they keep their state and take the fresh replies.
    fn read_item(&self, id: ItemId, ctx: &Ctx<'_>) {
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

    /// MOD-13 milestone 2 D7: `N` reads the form's catalogue for the selected item's project, a
    /// selected header's project, else the first scope project. The selected item's kind is the
    /// hint (§9 decision 4).
    fn open_new(&mut self, ctx: &Ctx<'_>) {
        let target = match self.selected {
            Some(Selection::Item(_)) => self
                .item()
                .map(|item| (item.project_id, Some(item.kind_id))),
            Some(Selection::Project(project)) => Some((project, None)),
            None => None,
        }
        .or_else(|| ctx.projects.first().map(|first| (first.project_id, None)));
        let Some((project, kind)) = target else {
            return;
        };
        self.opening = Some(Opening {
            project,
            item: None,
            kind,
        });
        ctx.request(StoreRequest::ItemForm {
            project,
            item: None,
        });
    }

    /// D7: `e` reads the selected item fresh with its project's catalogue; with no item
    /// selected nothing is sent.
    fn open_edit(&mut self, ctx: &Ctx<'_>) {
        let Some((project, id)) = self.item().map(|item| (item.project_id, item.id)) else {
            return;
        };
        self.opening = Some(Opening {
            project,
            item: Some(id),
            kind: None,
        });
        ctx.request(StoreRequest::ItemForm {
            project,
            item: Some(id),
        });
    }

    /// A key while the item form is open (D8, A7).
    ///
    /// Every plain key is consumed, used by the form or not: a passed `q`, digit, `w`, `?` or
    /// `Tab` would act globally (blueprint §8). The form itself passes chords but Ctrl+S (A6) and
    /// Ctrl+E (milestone 4 D1).
    fn on_item_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let Some(form) = self.item_form.as_mut() else {
            return Handled::Pass;
        };
        match form.on_key(key) {
            ItemFormOutcome::Pass => return Handled::Pass,
            ItemFormOutcome::Stay => {}
            ItemFormOutcome::Cancel => self.item_form = None,
            ItemFormOutcome::Reload(project) => ctx.request(StoreRequest::ItemForm {
                project,
                item: None,
            }),
            ItemFormOutcome::Save(request) => ctx.request(request),
            ItemFormOutcome::External(edit) => ctx.emit(Action::EditExternally(edit)),
        }
        Handled::Consumed
    }

    /// An `ItemForm` reply: a reloading form re-targets (E7); otherwise it opens a form only when
    /// it answers the read `N`/`e` sent (E8) and nothing else captures meanwhile.
    ///
    /// Review M1: an edit opens only while its item is still the selected one. The cursor may
    /// have moved while the read was out, and a form on the old item would sit under the new
    /// selection's header.
    fn on_item_form(&mut self, context: &ItemFormContext, ctx: &Ctx<'_>) {
        if let Some(form) = self.item_form.as_mut() {
            if form.busy() == Some(Busy::Reloading(context.project)) && context.item.is_none() {
                form.retarget(context.clone());
            }
            return;
        }
        let Some(opening) = self.opening.take() else {
            return;
        };
        let answers = opening.project == context.project
            && opening.item == context.item.as_ref().map(|item| item.id);
        let moved = opening.item.is_some() && opening.item != self.selected_item();
        if !answers || moved || self.form.is_some() || self.detail.captures_input() {
            return;
        }
        self.item_form = match opening.item {
            None => Some(ItemForm::open_new(
                context.clone(),
                ctx.projects,
                opening.kind,
            )),
            Some(_) => ItemForm::open_edit(context.clone()),
        };
    }

    /// An applied write lands only on the write in flight (MOD-59's self-naming rule).
    ///
    /// An edit closes the form and re-reads the item and the list under the active filter (D9,
    /// A5). The item's reads go out only while it is still selected (review M1): a list that
    /// moved the cursor off it would otherwise put its fresh Body under another item's header,
    /// and the list re-read alone carries the edit then. A mint closes it and reveals the new
    /// item, whose miss branch clears the filter (A5).
    fn on_item_written(&mut self, item: ItemId, outcome: &ItemWrite, ctx: &mut Ctx<'_>) {
        let Some(form) = self.item_form.as_ref() else {
            return;
        };
        match outcome {
            ItemWrite::Edited { .. }
                if form.busy() == Some(Busy::Editing) && form.item_id() == Some(item) =>
            {
                self.item_form = None;
                if self.selected == Some(Selection::Item(item)) {
                    self.read_item(item, ctx);
                }
                ctx.request(self.filter.to_request(ctx.scope));
            }
            ItemWrite::Minted { key } if form.busy() == Some(Busy::Minting) => {
                self.item_form = None;
                Tab::reveal(
                    self,
                    &RevealTarget::Item {
                        id: item,
                        key: key.clone(),
                    },
                    ctx,
                );
            }
            ItemWrite::Edited { .. } | ItemWrite::Minted { .. } => {}
        }
    }

    /// Milestone 3 D1, D3: a stale edit opens the three-way view, only on the form whose save it
    /// answers: busy editing this item, and the reply's ancestor at this form's token. A late reply
    /// from an earlier token (a resolution has since moved it) is dropped, as one for another item
    /// is.
    fn on_item_diverged(&mut self, divergence: &ItemDivergence) {
        if let Some(form) = self.item_form.as_mut()
            && form.busy() == Some(Busy::Editing)
            && form.item_id() == Some(divergence.head.id)
            && form.token() == Some(divergence.ancestor.version)
        {
            form.open_divergence(divergence);
        }
    }

    /// An item request's `Failed`. `App::on_reply` already reports every `Failed` on the status
    /// line (A8), so the tab adds no `Action::Error` of its own but in one case below.
    ///
    /// A refused form read opens nothing (D2). A mint that may have followed a COMMIT is hedged
    /// and the whole list re-read, the filter cleared so it cannot hide the item (D11, §10.1,
    /// §10.2); a refusal given before the insert is said plainly with nothing re-read.
    ///
    /// Review L4, the one case: such a mint `Failed` that finds no minting form (a scope change
    /// dropped it, and a form opened since is not the mint's) has nowhere to show its hedge, so
    /// the tab emits a status-line hedge as an `Action::Error`. It is drained after `App`'s plain
    /// report, so the hedge is what the status line keeps. Nothing is re-read: the item would be
    /// the old scope's, so the hedge promises no re-read and no Ctrl+S, and names that scope. A
    /// refused mint stays silent there, `App`'s report says it all.
    fn on_item_failed(&mut self, request: &str, message: &str, ctx: &Ctx<'_>) {
        let busy = self.item_form.as_ref().and_then(ItemForm::busy);
        match (request, busy) {
            (item_writes::FORM_NAME, Some(Busy::Reloading(_))) => {
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(message.to_owned()));
                }
            }
            (item_writes::FORM_NAME, _) => self.opening = None,
            (item_writes::MINT_NAME, Some(Busy::Minting)) => {
                let refused = item_writes::mint_refused(message);
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(if refused {
                        message.to_owned()
                    } else {
                        mint_may_have_landed(message)
                    }));
                }
                if !refused {
                    self.apply(BacklogFilter::default(), ctx);
                }
            }
            (item_writes::MINT_NAME, _) if !item_writes::mint_refused(message) => {
                ctx.emit(Action::Error(mint_may_have_landed_in_the_old_scope(
                    message,
                )));
            }
            (item_writes::EDIT_NAME, Some(Busy::Editing)) => {
                if let Some(form) = self.item_form.as_mut() {
                    form.settle(Some(message.to_owned()));
                }
            }
            _ => {}
        }
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
        // MOD-13 milestone 2 A7, E8: the item form's catalogue and a read in flight were the old
        // scope's.
        self.item_form = None;
        self.opening = None;
    }

    /// A capturing sub-tab (a typed note, a typed-back key, a `y`/`n`) gets every key first: the
    /// guard names no action (ANA-2 `:1694-1697`), it only stops the list from eating the letters
    /// the sub-tab is waiting for (MOD-4 plan OQ-7).
    /// MOD-22 review M-1: a bracketed paste reaches a capturing sub-tab's field and nothing else;
    /// the list never takes one.
    /// MOD-13 D1: the open filter form captures ahead of the sub-tabs; a paste goes to its tag
    /// field.
    /// MOD-13 milestone 2 D8: the open item form captures ahead of both; a paste goes to its
    /// focused text field.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(form) = self.item_form.as_mut() {
            form.on_paste(text);
            return Handled::Consumed;
        }
        if let Some(form) = self.form.as_mut() {
            form.on_paste(text);
            return Handled::Consumed;
        }
        self.detail.on_paste(text, ctx)
    }

    /// MOD-71 D1: the mouse is wanted while the detail pane is on screen with nothing typed over
    /// it — no filter form, no item form (which replaces the pane, and its divergence view the
    /// whole tab, `render`) — and its active sub-tab wants it.
    fn wants_mouse(&self) -> bool {
        todo!("MOD-71 T2")
    }

    /// MOD-71 D4: to the detail pane, under `wants_mouse`'s form guard; the registry checks the
    /// sub-tab's own answer. The list takes no mouse event.
    fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        todo!("MOD-71 T2")
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // MOD-13 D1: first, so the form owns every letter while it is open. It only opens while
        // no sub-tab captures (`f` is below that guard).
        if self.form.is_some() {
            return self.on_form_key(key, ctx);
        }
        // MOD-13 milestone 2 D8 (A7): the item form, likewise. It only opens while neither the
        // filter form nor a sub-tab captures.
        if self.item_form.is_some() {
            return self.on_item_form_key(key, ctx);
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
            // MOD-13 milestone 2 D7: `N` new item, `e` edit the selected one. Each reads the
            // form's catalogue first (D3); the reply opens the form. `N` arrives with or without
            // SHIFT, both as `Char('N')` here (blueprint E4).
            KeyCode::Char('N') => self.open_new(ctx),
            KeyCode::Char('e') => self.open_edit(ctx),
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
        // MOD-13 milestone 2: the item form's read and writes, before the list's.
        match reply {
            StoreReply::ItemForm(context) => return self.on_item_form(context, ctx),
            StoreReply::ItemWritten { item, outcome } => {
                return self.on_item_written(*item, outcome, ctx);
            }
            StoreReply::ItemDiverged(divergence) => return self.on_item_diverged(divergence),
            StoreReply::Failed { request, message } if item_writes::is_item_request(request) => {
                return self.on_item_failed(request, message, ctx);
            }
            _ => {}
        }
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

    /// MOD-13 milestone 4 D4: the `$EDITOR` outcome goes to the open item form, which ignores one
    /// it did not ask for. With no form (a scope change landed first) it is dropped, as
    /// `App::finish_external_edit` drops one for a gone tab.
    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        if let Some(form) = self.item_form.as_mut() {
            form.on_external_edit(outcome);
        } else {
            tracing::debug!("the item form that asked for the editor is gone");
        }
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
        // MOD-13 milestone 3 D4: the divergence view takes the whole tab area, list pane included.
        if let Some(view) = self.item_form.as_ref().and_then(ItemForm::resolving) {
            divergence::render(frame, area, view, ctx.theme);
            return;
        }
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
        // MOD-13 milestone 2 D8: the open item form takes the detail pane's place.
        match &self.item_form {
            Some(form) => item_form::render(frame, right, form, ctx.theme),
            None => detail::render(
                frame,
                right,
                &self.detail,
                self.item().map(|item| item.key.as_str()),
                ctx,
            ),
        }
    }

    fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
        let RevealTarget::Item { id, key } = target else {
            return false;
        };
        // A half-typed note or reject reason would be lost by the move (`go` resets the sub-tabs),
        // and so would a half-edited filter or a half-typed item.
        if self.form.is_some() || self.item_form.is_some() || self.detail.captures_input() {
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
    use crossterm::event::{MouseButton, MouseEventKind};
    use htui_core::model::ItemFilter;
    use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

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
            item_form: None,
            opening: None,
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
            item_form: None,
            opening: None,
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
            item_form: None,
            opening: None,
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

    // ---- MOD-71: the mouse reaches the detail pane (plan D1, D4) ----------------------------

    /// A sub-tab that wants the mouse while `wants` is set and logs what it is offered (MOD-71).
    #[derive(Debug)]
    struct MouseProbe {
        id: DetailId,
        wants: Rc<Cell<bool>>,
        seen: Pointed,
    }

    impl DetailTab for MouseProbe {
        fn id(&self) -> DetailId {
            self.id
        }
        fn title(&self) -> &str {
            "Mouse"
        }
        fn on_item_change(&mut self, _item: Option<ItemId>) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn wants_mouse(&self) -> bool {
            self.wants.get()
        }
        fn on_mouse(&mut self, mouse: MouseEvent, _ctx: &mut Ctx<'_>) -> Handled {
            self.seen.borrow_mut().push(mouse.kind);
            Handled::Consumed
        }
    }

    /// What a [`MouseProbe`] was offered.
    type Pointed = Rc<RefCell<Vec<MouseEventKind>>>;

    /// A [`MouseProbe`] under `id`, wanting the mouse as `wants` says, and its two handles.
    fn mouse_probe(id: &'static str, wants: bool) -> (Box<dyn DetailTab>, Rc<Cell<bool>>, Pointed) {
        let wants = Rc::new(Cell::new(wants));
        let seen = Rc::default();
        let probe = MouseProbe {
            id: DetailId(id),
            wants: Rc::clone(&wants),
            seen: Rc::clone(&seen),
        };
        (Box::new(probe), wants, seen)
    }

    /// A left press at a fixed cell, no modifier.
    fn click() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// MOD-71 D1, D4: the detail pane gets the mouse only while nothing is typed over it — the
    /// filter form and the item form each take it away — and only while its sub-tab wants it.
    #[tokio::test]
    async fn the_detail_gets_the_mouse_only_with_no_form_open() {
        let bench = Bench::new().await;
        let (probe, wants, seen) = mouse_probe("mouse", true);
        let mut detail = DetailRegistry::new();
        detail.register(probe);
        let mut tab = BacklogTab {
            detail,
            ..bench.tab()
        };
        assert!(tab.wants_mouse());
        assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Consumed);
        assert_eq!(seen.borrow().len(), 1);

        press(&mut tab, &bench, KeyCode::Char('f'));
        assert!(tab.form.is_some(), "the filter form is open");
        assert!(!tab.wants_mouse(), "the filter form takes the mouse away");
        assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
        assert_eq!(seen.borrow().len(), 1);
        press(&mut tab, &bench, KeyCode::Esc);
        assert!(tab.form.is_none(), "Esc closes it");
        assert!(tab.wants_mouse(), "and the mouse is wanted again");

        let _ = bench.actions();
        let store = MemStore::demo();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        assert!(!tab.wants_mouse(), "the item form takes the mouse away");
        assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
        assert_eq!(seen.borrow().len(), 1);
        press(&mut tab, &bench, KeyCode::Esc);
        assert!(tab.item_form.is_none(), "Esc closes it");
        assert!(tab.wants_mouse(), "and the mouse is wanted again");

        wants.set(false);
        assert!(!tab.wants_mouse(), "the sub-tab's own answer counts");
        assert_eq!(tab.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
        assert_eq!(seen.borrow().len(), 1);
    }

    /// MOD-71 D1, D4: a hidden sub-tab is not on screen to be clicked, so only the active one is
    /// asked.
    #[tokio::test]
    async fn only_the_active_sub_tab_is_asked_for_the_mouse() {
        let bench = Bench::new().await;
        let (a, _a_wants, a_seen) = mouse_probe("a", false);
        let (b, _b_wants, b_seen) = mouse_probe("b", true);
        let mut registry = DetailRegistry::new();
        registry.register(a);
        registry.register(b);
        assert!(!registry.wants_mouse());
        assert_eq!(registry.on_mouse(click(), &mut bench.ctx()), Handled::Pass);
        assert!(a_seen.borrow().is_empty() && b_seen.borrow().is_empty());

        assert!(registry.select(1));
        assert!(registry.wants_mouse());
        assert_eq!(
            registry.on_mouse(click(), &mut bench.ctx()),
            Handled::Consumed
        );
        assert_eq!(
            *b_seen.borrow(),
            vec![MouseEventKind::Down(MouseButton::Left)]
        );
        assert!(a_seen.borrow().is_empty());
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

    // ---- MOD-13 milestone 2: new and edit ----------------------------------------------------

    /// The store requests among everything emitted since the last drain.
    fn sent(bench: &Bench) -> Vec<StoreRequest> {
        bench
            .actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// The worker's answer to `request` over `store` (clones share state).
    async fn served(store: &MemStore, request: &StoreRequest) -> StoreReply {
        crate::store_worker::serve(&htui_store::Backend::memory(store.clone()), request).await
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_into(tab: &mut BacklogTab, bench: &Bench, text: &str) {
        for c in text.chars() {
            press(tab, bench, KeyCode::Char(c));
        }
    }

    /// `N` or `e` on `tab`, its one `ItemForm` read answered from `store`: the form is open.
    async fn open_with(tab: &mut BacklogTab, bench: &Bench, store: &MemStore, code: KeyCode) {
        press(tab, bench, code);
        let requests = sent(bench);
        let [read @ StoreRequest::ItemForm { .. }] = requests.as_slice() else {
            panic!("one form read: {requests:?}")
        };
        let reply = served(store, read).await;
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(
            tab.item_form.is_some(),
            "the reply opened the form: {reply:?}"
        );
    }

    /// The single request a Ctrl+S sent.
    fn saved(tab: &mut BacklogTab, bench: &Bench) -> StoreRequest {
        assert_eq!(tab.on_key(ctrl('s'), &mut bench.ctx()), Handled::Consumed);
        let mut requests = sent(bench);
        assert_eq!(requests.len(), 1, "one write: {requests:?}");
        requests.remove(0)
    }

    fn notice(tab: &BacklogTab) -> Option<String> {
        tab.item_form
            .as_ref()
            .and_then(|form| form.notice().map(str::to_owned))
    }

    fn errors(actions: &[Action]) -> Vec<&Action> {
        actions
            .iter()
            .filter(|action| matches!(action, Action::Error(_)))
            .collect()
    }

    /// Blueprint E4: a terminal's `N` may or may not carry SHIFT; both read the form.
    #[tokio::test]
    async fn n_reads_the_item_form_on_the_selected_project_with_and_without_shift() {
        use htui_core::fixtures::ids::PROJECT_HTUI;
        let bench = Bench::new().await;
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            let mut tab = bench.tab();
            assert_eq!(
                tab.on_key(
                    KeyEvent::new(KeyCode::Char('N'), modifiers),
                    &mut bench.ctx()
                ),
                Handled::Consumed
            );
            let requests = sent(&bench);
            assert!(
                matches!(
                    requests.as_slice(),
                    [StoreRequest::ItemForm { project, item: None }] if *project == PROJECT_HTUI
                ),
                "{modifiers:?}: {requests:?}"
            );
        }
    }

    /// D7: `e` reads the selected item's form; with nothing selected it sends nothing.
    #[tokio::test]
    async fn e_reads_the_selected_item_and_nothing_without_a_selection() {
        use htui_core::fixtures::ids::PROJECT_HTUI;
        let bench = Bench::new().await;
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let mut tab = bench.tab();
        assert_eq!(
            press(&mut tab, &bench, KeyCode::Char('e')),
            Handled::Consumed
        );
        let requests = sent(&bench);
        assert!(
            matches!(
                requests.as_slice(),
                [StoreRequest::ItemForm { project, item: Some(id) }]
                    if *project == PROJECT_HTUI && *id == first
            ),
            "{requests:?}"
        );

        let mut empty = BacklogTab::new();
        assert_eq!(
            press(&mut empty, &bench, KeyCode::Char('e')),
            Handled::Consumed
        );
        assert!(sent(&bench).is_empty());
        assert!(empty.opening.is_none());
    }

    /// D8: the reply opens the form in the detail pane, and it captures the list's and the
    /// shell's letters.
    #[tokio::test]
    async fn the_item_form_reply_opens_the_form_and_it_captures() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let before = drawn(&tab, &bench);
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        for code in [KeyCode::Char('j'), KeyCode::Char('q'), KeyCode::Char('3')] {
            assert_eq!(press(&mut tab, &bench, code), Handled::Consumed, "{code:?}");
        }
        assert_eq!(tab.selected, Some(bench.first), "the cursor did not move");
        assert!(sent(&bench).is_empty(), "nothing was read");
        let frame = drawn(&tab, &bench);
        assert!(frame.contains(" New item \u{b7} htui "), "{frame}");
        assert_ne!(frame, before);

        press(&mut tab, &bench, KeyCode::Esc);
        assert!(tab.item_form.is_none(), "Esc closes it");
        assert_eq!(
            drawn(&tab, &bench),
            before,
            "and the pane is the detail again"
        );
    }

    /// Blueprint E8: a form opens only on the answer to the read `N`/`e` sent.
    #[tokio::test]
    async fn an_item_form_reply_nobody_asked_for_opens_nothing() {
        use htui_core::fixtures::ids::{PROJECT_AGY, PROJECT_HTUI};
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let unasked = served(
            &store,
            &StoreRequest::ItemForm {
                project: PROJECT_HTUI,
                item: None,
            },
        )
        .await;
        tab.on_reply(&unasked, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "no `N`, no form");

        press(&mut tab, &bench, KeyCode::Char('N'));
        let _ = sent(&bench);
        let other = served(
            &store,
            &StoreRequest::ItemForm {
                project: PROJECT_AGY,
                item: None,
            },
        )
        .await;
        tab.on_reply(&other, &mut bench.ctx());
        assert!(
            tab.item_form.is_none(),
            "another project's answer opens nothing"
        );
        assert!(errors(&bench.actions()).is_empty());
    }

    /// MOD-22 review M-1: a paste reaches the open item form's focused field.
    #[tokio::test]
    async fn a_paste_goes_to_the_open_item_form() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        assert_eq!(tab.on_paste("Pasted", &mut bench.ctx()), Handled::Consumed);
        let request = saved(&mut tab, &bench);
        assert!(
            matches!(&request, StoreRequest::MintItem { spec, .. } if spec.title == "Pasted"),
            "{request:?}"
        );
    }

    #[tokio::test]
    async fn ctrl_s_on_a_blank_title_refuses_in_the_form_and_sends_nothing() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        assert_eq!(tab.on_key(ctrl('s'), &mut bench.ctx()), Handled::Consumed);
        let actions = bench.actions();
        assert!(
            actions.is_empty(),
            "nothing sent, nothing reported: {actions:?}"
        );
        assert_eq!(
            notice(&tab),
            Some(htui_core::model::SpecError::BlankTitle.to_string())
        );
    }

    #[tokio::test]
    async fn an_unchanged_edit_says_nothing_to_save_and_sends_nothing() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
        assert_eq!(tab.on_key(ctrl('s'), &mut bench.ctx()), Handled::Consumed);
        assert!(bench.actions().is_empty());
        assert_eq!(
            notice(&tab).as_deref(),
            Some(htui_core::model::item_spec::NOTHING_TO_SAVE)
        );
    }

    /// The edit form on the first item with ` v2` typed and saved: the one `EditItem` sent.
    async fn an_edit_saved(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) -> StoreRequest {
        open_with(tab, bench, store, KeyCode::Char('e')).await;
        type_into(tab, bench, " v2");
        saved(tab, bench)
    }

    /// D5, D6: only the title, at the version the form's read carried.
    #[tokio::test]
    async fn a_changed_title_sends_edit_item_with_only_the_title_at_the_read_version() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let title = format!(
            "{} v2",
            store
                .item(first)
                .await
                .expect("read")
                .expect("the item")
                .title
        );
        let mut tab = bench.tab();
        let request = an_edit_saved(&mut tab, &bench, &store).await;
        assert!(
            matches!(
                &request,
                StoreRequest::EditItem {
                    id,
                    expected_version: 1,
                    changes,
                    reason: htui_core::model::EditReason::Edited,
                } if *id == first && *changes == htui_core::model::SpecChanges {
                        title: Some(title.clone()),
                        ..htui_core::model::SpecChanges::default()
                    }
            ),
            "{request:?}"
        );
    }

    /// D9, A5: the form closes, the item's seven reads go out again and the list re-reads,
    /// with the cursor where it was.
    #[tokio::test]
    async fn an_applied_edit_closes_the_form_and_re_reads_the_item_keeping_the_selection() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let mut tab = bench.tab();
        let request = an_edit_saved(&mut tab, &bench, &store).await;
        let reply = served(&store, &request).await;
        assert!(matches!(reply, StoreReply::ItemWritten { .. }), "{reply:?}");
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "the form closed");
        assert_eq!(tab.selected, Some(bench.first));

        let requests = sent(&bench);
        let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
        assert_eq!(requests.len(), 8, "{names:?}");
        let per_item = requests
            .iter()
            .filter(|request| match request {
                StoreRequest::Item(id)
                | StoreRequest::Runs(id)
                | StoreRequest::Documents(id)
                | StoreRequest::Notes(id)
                | StoreRequest::ItemRequirements(id)
                | StoreRequest::Links { id, .. }
                | StoreRequest::PromptPreview { item: id, .. } => *id == first,
                _ => false,
            })
            .count();
        assert_eq!(per_item, 7, "{names:?}");
        assert_eq!(
            items_reads(&requests.into_iter().map(Action::Store).collect::<Vec<_>>()).len(),
            1
        );
    }

    /// How many of `requests` are per-item reads (the seven `read_item` sends) of `id`.
    fn per_item_reads(requests: &[StoreRequest], id: ItemId) -> usize {
        requests
            .iter()
            .filter(|request| match request {
                StoreRequest::Item(read)
                | StoreRequest::Runs(read)
                | StoreRequest::Documents(read)
                | StoreRequest::Notes(read)
                | StoreRequest::ItemRequirements(read)
                | StoreRequest::Links { id: read, .. }
                | StoreRequest::PromptPreview { item: read, .. } => *read == id,
                _ => false,
            })
            .count()
    }

    /// Review M1: an `e` whose reply lands after the cursor moved opens nothing, so the form
    /// cannot edit an item the header no longer names.
    #[tokio::test]
    async fn an_edit_reply_after_the_cursor_moved_opens_nothing() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('e'));
        let requests = sent(&bench);
        let [read @ StoreRequest::ItemForm { .. }] = requests.as_slice() else {
            panic!("one form read: {requests:?}")
        };
        press(&mut tab, &bench, KeyCode::Char('j'));
        assert_ne!(tab.selected, Some(bench.first), "the cursor moved");
        let _ = sent(&bench);

        let reply = served(&store, read).await;
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "no form for the old item");
        assert!(tab.opening.is_none(), "the opening is spent");
        assert!(bench.actions().is_empty(), "nothing read, nothing reported");
    }

    /// Review M1: an applied edit re-reads the item only while it is still selected; once a list
    /// moved the cursor off it, the list re-read alone follows, so the Body cannot show the
    /// edited item under the new selection's header.
    #[tokio::test]
    async fn an_applied_edit_after_the_selection_moved_reads_only_the_list() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let mut tab = bench.tab();
        let request = an_edit_saved(&mut tab, &bench, &store).await;
        let others: Vec<ItemSummary> = bench
            .items
            .iter()
            .filter(|item| item.id != first)
            .cloned()
            .collect();
        tab.on_reply(&StoreReply::Items(others), &mut bench.ctx());
        assert_ne!(tab.selected, Some(bench.first), "the list moved the cursor");
        let _ = sent(&bench);

        let reply = served(&store, &request).await;
        assert!(matches!(reply, StoreReply::ItemWritten { .. }), "{reply:?}");
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "the form closed");
        let requests = sent(&bench);
        let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
        assert_eq!(per_item_reads(&requests, first), 0, "{names:?}");
        assert_eq!(
            items_reads(&requests.into_iter().map(Action::Store).collect::<Vec<_>>()).len(),
            1,
            "{names:?}"
        );
    }

    /// The new form on the first item's project with `Fresh item` typed and saved.
    async fn a_mint_saved(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) -> StoreRequest {
        open_with(tab, bench, store, KeyCode::Char('N')).await;
        type_into(tab, bench, "Fresh item");
        saved(tab, bench)
    }

    /// D9, A5: a minted item is never in the loaded rows, so the reveal clears the filter and
    /// the next list selects it.
    #[tokio::test]
    async fn an_applied_mint_closes_the_form_clears_the_filter_and_reveals_it() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        let request = a_mint_saved(&mut tab, &bench, &store).await;
        let reply = served(&store, &request).await;
        let StoreReply::ItemWritten { item: minted, .. } = reply else {
            panic!("a mint answers ItemWritten, not {reply:?}")
        };
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "the form closed");
        assert!(tab.filter.is_empty(), "the filter is cleared");
        assert_eq!(tab.pending_reveal, Some((minted, "ANA-3".to_owned())));
        let actions = bench.actions();
        assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
        assert_eq!(actions.len(), 1, "{actions:?}");

        let list = served(&store, &tab.filter.to_request(&bench.scope)).await;
        tab.on_reply(&list, &mut bench.ctx());
        assert_eq!(tab.selected, Some(Selection::Item(minted)));
        assert!(errors(&bench.actions()).is_empty());
    }

    // ---- MOD-13 milestone 3: the divergence view -------------------------------------------

    /// `e` on the first item, `Theirs` written through the store at v1, ` mine` typed and the
    /// stale save answered: the view is open. The first item's id and key.
    async fn in_the_view(
        tab: &mut BacklogTab,
        bench: &Bench,
        store: &MemStore,
    ) -> (ItemId, String) {
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        open_with(tab, bench, store, KeyCode::Char('e')).await;
        store
            .update_item(
                first,
                1,
                htui_core::model::ItemPatch {
                    title: Some("Theirs".to_owned()),
                    author_id: htui_core::fixtures::ids::USER,
                    reason: "elsewhere".to_owned(),
                    ..htui_core::model::ItemPatch::default()
                },
            )
            .await
            .expect("their edit");
        type_into(tab, bench, " mine");
        let request = saved(tab, bench);
        let reply = served(store, &request).await;
        assert!(matches!(reply, StoreReply::ItemDiverged(_)), "{reply:?}");
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(view(tab).is_some(), "the view opened");
        let key = store
            .item(first)
            .await
            .expect("read")
            .expect("the item")
            .key;
        (first, key)
    }

    /// The open divergence view, if any.
    fn view(tab: &BacklogTab) -> Option<&divergence::Divergence> {
        tab.item_form.as_ref().and_then(ItemForm::resolving)
    }

    /// Milestone 3 D3, D5: a stale edit opens the view; `Esc` returns to the form with its text
    /// and its token, and the next save is the same stale one.
    #[tokio::test]
    async fn a_diverged_edit_opens_the_view_and_esc_keeps_the_text_and_token() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        in_the_view(&mut tab, &bench, &store).await;
        assert!(errors(&bench.actions()).is_empty());

        assert_eq!(press(&mut tab, &bench, KeyCode::Esc), Handled::Consumed);
        assert!(view(&tab).is_none(), "the view closed");
        assert!(tab.item_form.is_some(), "the form stays");
        assert_eq!(notice(&tab), Some(divergence::still_behind(2)));

        let again = saved(&mut tab, &bench);
        assert!(
            matches!(
                &again,
                StoreRequest::EditItem {
                    expected_version: 1,
                    changes,
                    reason: htui_core::model::EditReason::Edited,
                    ..
                } if changes.title.as_deref().is_some_and(|title| title.ends_with(" mine"))
            ),
            "the token never moved and the text is kept: {again:?}"
        );
    }

    /// D1: a divergence opens the view only on the form whose save it answers.
    #[tokio::test]
    async fn a_divergence_for_another_item_or_token_is_dropped() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
        store
            .update_item(
                first,
                1,
                htui_core::model::ItemPatch {
                    title: Some("Theirs".to_owned()),
                    author_id: htui_core::fixtures::ids::USER,
                    reason: "elsewhere".to_owned(),
                    ..htui_core::model::ItemPatch::default()
                },
            )
            .await
            .expect("their edit");
        type_into(&mut tab, &bench, " mine");
        let request = saved(&mut tab, &bench);
        let StoreReply::ItemDiverged(real) = served(&store, &request).await else {
            panic!("a divergence")
        };
        let mut late = real.as_ref().clone();
        late.ancestor.version = 7;
        let mut other = real.as_ref().clone();
        other.head.id = ItemId::new();
        for stray in [late, other] {
            tab.on_reply(&StoreReply::ItemDiverged(Box::new(stray)), &mut bench.ctx());
            assert!(view(&tab).is_none());
            assert_eq!(
                tab.item_form.as_ref().and_then(ItemForm::busy),
                Some(Busy::Editing)
            );
        }
        tab.on_reply(&StoreReply::ItemDiverged(real), &mut bench.ctx());
        assert!(view(&tab).is_some(), "the real one still opens it");
    }

    /// D5, D6: `m` rebases the form on the head, and Ctrl+S lands a resolution there.
    #[tokio::test]
    async fn m_rebases_and_ctrl_s_sends_a_resolution_at_the_head() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        in_the_view(&mut tab, &bench, &store).await;
        assert_eq!(
            press(&mut tab, &bench, KeyCode::Char('m')),
            Handled::Consumed
        );
        assert!(view(&tab).is_none());
        assert_eq!(notice(&tab), Some(divergence::rebased_on(2)));
        let request = saved(&mut tab, &bench);
        assert!(
            matches!(
                &request,
                StoreRequest::EditItem {
                    expected_version: 2,
                    changes,
                    reason: htui_core::model::EditReason::DivergenceResolution,
                    ..
                } if changes.title.as_deref().is_some_and(|title| title.ends_with(" mine"))
                    && *changes == htui_core::model::SpecChanges {
                        title: changes.title.clone(),
                        ..htui_core::model::SpecChanges::default()
                    }
            ),
            "{request:?}"
        );
        let reply = served(&store, &request).await;
        assert!(
            matches!(
                &reply,
                StoreReply::ItemWritten {
                    outcome: ItemWrite::Edited { version: 3, .. },
                    ..
                }
            ),
            "{reply:?}"
        );
        tab.on_reply(&reply, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "the form closed");
    }

    /// D8: a resolution that is itself stale re-opens the view, its ancestor the rebased head.
    #[tokio::test]
    async fn a_second_divergence_after_a_rebase_reopens_the_view() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let (first, _) = in_the_view(&mut tab, &bench, &store).await;
        press(&mut tab, &bench, KeyCode::Char('m'));
        store
            .update_item(
                first,
                2,
                htui_core::model::ItemPatch {
                    priority: Some(42),
                    author_id: htui_core::fixtures::ids::USER,
                    reason: "elsewhere again".to_owned(),
                    ..htui_core::model::ItemPatch::default()
                },
            )
            .await
            .expect("their second edit");
        let request = saved(&mut tab, &bench);
        let reply = served(&store, &request).await;
        let StoreReply::ItemDiverged(divergence) = &reply else {
            panic!("a second divergence: {reply:?}")
        };
        assert_eq!(divergence.ancestor.version, 2);
        assert_eq!(
            tab.item_form.as_ref().and_then(ItemForm::token),
            Some(2),
            "the reply answers this token"
        );
        tab.on_reply(&reply, &mut bench.ctx());
        let open = view(&tab).expect("the view re-opened");
        assert_eq!((open.ancestor_version(), open.head_version()), (2, 3));
    }

    /// D4: the view draws over the whole tab area, the list pane included.
    #[tokio::test]
    async fn the_view_takes_the_whole_tab_area() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let (_, key) = in_the_view(&mut tab, &bench, &store).await;
        let frame = drawn(&tab, &bench);
        assert!(
            frame.contains(&divergence::header(&key, 1, 2)),
            "the header in\n{frame}"
        );
        let other = &bench.items[1].key;
        assert_ne!(*other, key);
        assert!(!frame.contains(other.as_str()), "no list row in\n{frame}");
        let slug = &bench.projects[0].slug;
        assert!(
            !frame.contains(&format!("{slug} ")),
            "no list header in\n{frame}"
        );

        press(&mut tab, &bench, KeyCode::Esc);
        let frame = drawn(&tab, &bench);
        assert!(
            frame.contains(&format!(" Edit {key} (v1) ")),
            "the form again in\n{frame}"
        );
        assert!(frame.contains(slug.as_str()), "the list again in\n{frame}");
    }

    #[tokio::test]
    async fn a_scope_change_closes_the_open_view() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        in_the_view(&mut tab, &bench, &store).await;
        tab.on_scope_change(&bench.scope);
        assert!(tab.item_form.is_none());
    }

    /// A reveal would drop the half-resolved edit, as it would the form.
    #[tokio::test]
    async fn a_reveal_while_the_view_is_open_asks_to_close_it_first() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        in_the_view(&mut tab, &bench, &store).await;
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
        assert!(view(&tab).is_some(), "the view stays");
        assert_eq!(tab.selected, Some(bench.first));
    }

    /// §10.1, §10.2: a mint `Failed` that may follow a COMMIT keeps the text, hedges and re-reads
    /// the whole list, so the filter cannot hide the item the notice asks to look for.
    #[tokio::test]
    async fn a_store_failed_mint_keeps_the_text_hedges_and_re_reads_unfiltered() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        let _ = a_mint_saved(&mut tab, &bench, &store).await;
        let why =
            htui_core::store::StoreError::Backend("the server went away".to_owned()).to_string();
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::MINT_NAME,
                message: why.clone(),
            },
            &mut bench.ctx(),
        );
        let form = tab.item_form.as_ref().expect("the form stays");
        assert_eq!(form.busy(), None);
        assert_eq!(notice(&tab), Some(mint_may_have_landed(&why)));
        assert!(tab.filter.is_empty(), "the filter is cleared");
        let actions = bench.actions();
        assert_eq!(items_reads(&actions), [(ItemFilter::default(), false)]);
        assert_eq!(
            actions.len(),
            1,
            "one read, no error of the tab's own: {actions:?}"
        );

        let retry = saved(&mut tab, &bench);
        assert!(
            matches!(&retry, StoreRequest::MintItem { spec, .. } if spec.title == "Fresh item"),
            "the text is kept: {retry:?}"
        );
    }

    /// §10.1: a refusal given before the insert wrote nothing; it is said plainly, nothing is
    /// re-read and the filter stays.
    #[tokio::test]
    async fn a_refused_mint_keeps_the_text_and_says_why_without_hedging_or_re_reading() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = BacklogTab {
            filter: done_only(),
            ..bench.tab()
        };
        let _ = a_mint_saved(&mut tab, &bench, &store).await;
        let why =
            htui_core::store::StoreError::Constraint("that kind is gone".to_owned()).to_string();
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::MINT_NAME,
                message: why.clone(),
            },
            &mut bench.ctx(),
        );
        assert_eq!(notice(&tab), Some(why));
        assert_eq!(tab.item_form.as_ref().and_then(ItemForm::busy), None);
        assert_eq!(tab.filter, done_only(), "the filter stays");
        assert!(
            bench.actions().is_empty(),
            "nothing re-read, nothing reported"
        );
    }

    /// Review L4: a store-failed mint that lands after a scope change closed its form keeps its
    /// hedge, on the status line, as the one error the tab raises of its own; nothing is re-read.
    #[tokio::test]
    async fn a_store_failed_mint_with_its_form_gone_is_hedged_on_the_status_line() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let _ = a_mint_saved(&mut tab, &bench, &store).await;
        tab.on_scope_change(&bench.scope);
        let why =
            htui_core::store::StoreError::Backend("the server went away".to_owned()).to_string();
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::MINT_NAME,
                message: why.clone(),
            },
            &mut bench.ctx(),
        );
        let actions = bench.actions();
        let [Action::Error(sentence)] = actions.as_slice() else {
            panic!("one status-line hedge, nothing re-read: {actions:?}");
        };
        assert!(sentence.starts_with(&why), "{sentence:?}");
        assert!(sentence.contains("may have been written"), "{sentence:?}");
        // Round 2: nothing is re-read and no form is open, so neither is promised.
        assert!(!sentence.contains("re-read"), "{sentence:?}");
        assert!(!sentence.contains("Ctrl+S"), "{sentence:?}");
        assert_eq!(*sentence, mint_may_have_landed_in_the_old_scope(&why));
        assert!(tab.item_form.is_none());
    }

    /// Review L4: a refused mint wrote nothing, and `App` already reports its `Failed`, so with
    /// its form gone the tab adds nothing.
    #[tokio::test]
    async fn a_refused_mint_with_its_form_gone_adds_nothing() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        let _ = a_mint_saved(&mut tab, &bench, &store).await;
        tab.on_scope_change(&bench.scope);
        let why =
            htui_core::store::StoreError::Constraint("that kind is gone".to_owned()).to_string();
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::MINT_NAME,
                message: why,
            },
            &mut bench.ctx(),
        );
        assert!(bench.actions().is_empty());
    }

    /// D2, A8: a refused form read opens nothing, and the tab adds no error to the one `App`
    /// already reports.
    #[tokio::test]
    async fn a_failed_item_form_read_opens_nothing_and_adds_no_error() {
        use htui_core::fixtures::ids::PROJECT_HTUI;
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('N'));
        let _ = sent(&bench);
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::FORM_NAME,
                message: format!("store unreachable: {}", htui_store::DATABASE_UNREACHABLE),
            },
            &mut bench.ctx(),
        );
        assert!(tab.item_form.is_none());
        assert!(tab.opening.is_none(), "the opening is forgotten");
        assert!(
            bench.actions().is_empty(),
            "no Action::Error of the tab's own"
        );

        let late = served(
            &store,
            &StoreRequest::ItemForm {
                project: PROJECT_HTUI,
                item: None,
            },
        )
        .await;
        tab.on_reply(&late, &mut bench.ctx());
        assert!(tab.item_form.is_none(), "a late answer opens nothing");
    }

    /// A7, E8: a scope change closes the form and forgets a read in flight.
    #[tokio::test]
    async fn a_scope_change_closes_the_item_form_and_forgets_the_opening() {
        use htui_core::fixtures::ids::PROJECT_HTUI;
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        tab.on_scope_change(&bench.scope);
        assert!(tab.item_form.is_none(), "the form closed");

        let mut tab = bench.tab();
        press(&mut tab, &bench, KeyCode::Char('N'));
        let _ = sent(&bench);
        tab.on_scope_change(&bench.scope);
        assert!(tab.opening.is_none());
        let late = served(
            &store,
            &StoreRequest::ItemForm {
                project: PROJECT_HTUI,
                item: None,
            },
        )
        .await;
        tab.on_reply(&late, &mut bench.ctx());
        assert!(
            tab.item_form.is_none(),
            "the old scope's answer opens nothing"
        );
    }

    /// `N` with `Fresh item` typed, the focus moved back to the project picker and `l` pressed:
    /// the one `ItemForm` read the move sent (E7).
    async fn a_project_moved(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) {
        use htui_core::fixtures::ids::PROJECT_AGY;
        open_with(tab, bench, store, KeyCode::Char('N')).await;
        type_into(tab, bench, "Fresh item");
        for _ in 0..2 {
            press(tab, bench, KeyCode::BackTab);
        }
        assert_eq!(
            tab.item_form.as_ref().map(ItemForm::focus),
            Some(item_form::Field::Project)
        );
        assert_eq!(press(tab, bench, KeyCode::Char('l')), Handled::Consumed);
        let requests = sent(bench);
        assert!(
            matches!(
                requests.as_slice(),
                [StoreRequest::ItemForm { project, item: None }] if *project == PROJECT_AGY
            ),
            "one reload for the picked project: {requests:?}"
        );
        assert_eq!(
            tab.item_form.as_ref().and_then(ItemForm::busy),
            Some(Busy::Reloading(PROJECT_AGY))
        );
    }

    /// Review M2, E7: the project picker's reload re-targets the open form; the typed text stays.
    #[tokio::test]
    async fn the_project_picker_reloads_and_the_reply_retargets_keeping_the_text() {
        use htui_core::fixtures::ids::PROJECT_AGY;
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        a_project_moved(&mut tab, &bench, &store).await;

        let reply = served(
            &store,
            &StoreRequest::ItemForm {
                project: PROJECT_AGY,
                item: None,
            },
        )
        .await;
        tab.on_reply(&reply, &mut bench.ctx());
        let form = tab.item_form.as_ref().expect("the form stays");
        assert_eq!(form.busy(), None);
        assert_eq!(form.project(), PROJECT_AGY);
        assert!(bench.actions().is_empty(), "nothing else is read");

        let request = saved(&mut tab, &bench);
        assert!(
            matches!(
                &request,
                StoreRequest::MintItem { project, spec }
                    if *project == PROJECT_AGY && spec.title == "Fresh item"
            ),
            "the text is kept: {request:?}"
        );
    }

    /// Review M2, E7: a refused reload settles the form with the reason; `Esc` then closes it.
    #[tokio::test]
    async fn a_refused_reload_settles_the_form_with_a_notice_and_esc_closes_it() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        a_project_moved(&mut tab, &bench, &store).await;

        let why = format!("store unreachable: {}", htui_store::DATABASE_UNREACHABLE);
        tab.on_reply(
            &StoreReply::Failed {
                request: item_writes::FORM_NAME,
                message: why.clone(),
            },
            &mut bench.ctx(),
        );
        let form = tab.item_form.as_ref().expect("the form stays");
        assert_eq!(form.busy(), None);
        assert_eq!(notice(&tab), Some(why));
        assert!(
            bench.actions().is_empty(),
            "no Action::Error of the tab's own"
        );

        assert_eq!(press(&mut tab, &bench, KeyCode::Esc), Handled::Consumed);
        assert!(tab.item_form.is_none(), "Esc closes it");
    }

    /// A reveal would drop the half-typed item, as it would a half-edited filter.
    #[tokio::test]
    async fn a_reveal_while_the_item_form_is_open_asks_to_close_it_first() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
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
        assert!(tab.item_form.is_some());
        assert_eq!(tab.selected, Some(bench.first));
    }

    /// MOD-52: `ctrl-c` quits from inside the item form, as from every text field.
    #[tokio::test]
    async fn ctrl_c_passes_through_the_open_item_form() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('N')).await;
        assert_eq!(tab.on_key(ctrl('c'), &mut bench.ctx()), Handled::Pass);
        assert!(tab.item_form.is_some());
    }

    // ---- MOD-13 milestone 4: the $EDITOR round-trip ------------------------------------------

    /// From the open form's Title, `Tab` five times to Body: Priority, Tags, Graph, Paths, Body.
    fn to_body(tab: &mut BacklogTab, bench: &Bench) {
        for _ in 0..5 {
            press(tab, bench, KeyCode::Tab);
        }
    }

    /// D3: Ctrl+E on the body reaches the loop as an `EditExternally`, and nothing is read.
    #[tokio::test]
    async fn ctrl_e_on_the_body_emits_an_external_edit_for_the_loop() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let Selection::Item(first) = bench.first else {
            panic!("the first row is an item")
        };
        let item = store.item(first).await.expect("read").expect("the item");
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
        to_body(&mut tab, &bench);
        assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
        let actions = bench.actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::EditExternally(edit)]
                    if edit.stem == format!("{}-body", item.key) && edit.text == item.body
            ),
            "{actions:?}"
        );
    }

    /// D1: on a one-line field Ctrl+E is consumed, so it never reaches a global binding.
    #[tokio::test]
    async fn ctrl_e_on_the_title_is_consumed_and_emits_nothing() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
        assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
        assert!(bench.actions().is_empty());
    }

    /// D4: the outcome reaches the open form, and Ctrl+S sends the body alone at the read version.
    #[tokio::test]
    async fn the_outcome_reaches_the_open_form_and_ctrl_s_saves_the_body() {
        let bench = Bench::new().await;
        let store = MemStore::demo();
        let mut tab = bench.tab();
        open_with(&mut tab, &bench, &store, KeyCode::Char('e')).await;
        to_body(&mut tab, &bench);
        assert_eq!(tab.on_key(ctrl('e'), &mut bench.ctx()), Handled::Consumed);
        assert_eq!(bench.actions().len(), 1, "the handoff");
        tab.on_external_edit(
            ExternalEditOutcome::Edited("New body.".to_owned()),
            &mut bench.ctx(),
        );
        assert_eq!(notice(&tab).as_deref(), Some(crate::editor::EDITED));
        let request = saved(&mut tab, &bench);
        assert!(
            matches!(
                &request,
                StoreRequest::EditItem {
                    expected_version: 1,
                    changes,
                    reason: htui_core::model::EditReason::Edited,
                    ..
                } if *changes == htui_core::model::SpecChanges {
                        body: Some("New body.".to_owned()),
                        ..htui_core::model::SpecChanges::default()
                    }
            ),
            "{request:?}"
        );
    }

    /// D4: with no form open the outcome is dropped; nothing opens and nothing is emitted.
    #[tokio::test]
    async fn an_outcome_with_no_form_open_is_dropped() {
        let bench = Bench::new().await;
        let mut tab = bench.tab();
        tab.on_external_edit(
            ExternalEditOutcome::Edited("x".to_owned()),
            &mut bench.ctx(),
        );
        assert!(tab.item_form.is_none());
        assert!(bench.actions().is_empty());
    }
}
