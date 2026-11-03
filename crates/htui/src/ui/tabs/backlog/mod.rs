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
//!
//! Milestone 5: Notes and Docs write through their own compose areas. An `$EDITOR` outcome goes to
//! the item form when one is open, else to the active sub-tab.
//!
//! Keys resolve through the focused pane's stack (MOD-67 M5): a project header folds on `Enter`
//! first, then the active sub-tab takes its own acts, then the tab answers `list.*`, the sub-tab
//! cycling, `backlog.*` and `common.edit`, including the replay miss (D6).

pub mod detail;
pub mod divergence;
pub mod filter;
pub mod item_form;
pub mod list;

mod hints;
mod navigation;
mod queue;
mod writes;

use std::cell::Cell;

use htui_core::model::{ItemId, ItemKindId, ItemSummary, ProjectId, RunId, Scope, StepId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::editor::ExternalEditOutcome;
use crate::item_writes;
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::store_worker::{QUEUE_REQUEST_NAMES, StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::kit::pane::{Focus, Pane, capture_area};
use crate::ui::kit::{chips::key_line, scroll::Step};
use crate::ui::shell::{ListSize, PaneAreas, PaneId, Panes, TabLayout, TabLoc};
use crate::ui::tabs::backlog::detail::{
    BodyTab, DetailId, DetailRegistry, DocumentsTab, GraphTab, NotesTab, PromptTab, ReqsTab,
    RunsTab,
};
use crate::ui::tabs::backlog::filter::{BacklogFilter, FilterForm};
use crate::ui::tabs::backlog::hints::{LIST_SEP, styled_spans};
use crate::ui::tabs::backlog::item_form::ItemForm;
use crate::ui::tabs::backlog::list::{ListView, Selection};
use crate::ui::tabs::backlog::navigation::{ITEM_CURSOR, PROJECT_CURSOR, parse_cursor};
use crate::ui::tabs::registry::{CLOSE_THE_FIELD_FIRST, Tab, TabId};
use crossterm::event::{KeyEvent, MouseEvent};

/// The Backlog's default layout (MOD-115 M2 D2): the list beside the detail, 55% of the body.
///
/// The list gets the larger half: it carries the longest strings (`KEY  status  title`, with
/// `awaiting_approval` in it) and it is the pane a reader navigates from. The shell's layout
/// engine splits it as `Layout::horizontal([Percentage(55), Percentage(45)])` did, cell for cell.
pub const BACKLOG_LAYOUT: TabLayout = TabLayout {
    list: ListSize::Percent(55),
    ..TabLayout::DEFAULT
};

/// How one Backlog pane is framed this frame (MOD-115 M4 C4, Q2): whether it holds the keyboard,
/// the tab's accent, and the hints it carries on its bottom border while it does.
#[derive(Debug, Clone)]
pub struct Chrome<'a> {
    /// The pane's focus.
    pub focus: Focus,
    /// The tab's accent (`ctx.accent()`), for the focused border and the cursor's gutter.
    pub accent: Style,
    /// The pane's own keys, drawn only while it is focused.
    pub hints: Option<Line<'a>>,
}

impl<'a> Chrome<'a> {
    /// `pane` with this focus, accent and hints.
    #[must_use]
    pub fn frame(self, pane: Pane<'a>) -> Pane<'a> {
        let pane = pane.focus(self.focus).accent(self.accent);
        match self.hints {
            Some(hints) => pane.hints(hints),
            None => pane,
        }
    }
}

/// `StoreRequest::Items`' name: what a refused list read is answered `Failed` under.
const ITEMS_READ: &str = "items";

/// Refreshes between two `Runs` polls: five of the shell's one-second refreshes, so 5 s (MOD-41
/// plan D16, OQ-3).
const REFRESHES_PER_RUNS_POLL: u32 = 5;

/// MOD-12 D9: the toggle `Q` or `P` asked for, decided by the `QueueState` read it is waiting on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum QueueIntent {
    /// `Q` on this item (its key, for the status line).
    Item {
        /// The item.
        id: ItemId,
        /// Its key.
        key: String,
    },
    /// `P`.
    Pause,
}

/// A reveal waiting for the next `Items` reply (MOD-64 D235), with the key its miss is reported by
/// and, for `RevealTarget::Step`, the run and step the Runs pane is to focus (MOD-69 plan D8).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingReveal {
    /// The item to select.
    id: ItemId,
    /// Its key, for the miss sentence.
    key: String,
    /// The Runs pane's focus; `None` for an item reveal.
    focus: Option<(Option<RunId>, Option<StepId>)>,
}

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
    /// reported by and a step reveal's focus (MOD-69 plan D8).
    pending_reveal: Option<PendingReveal>,
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
    /// MOD-12 D9: the toggle the pending `QueueState` read decides. That read's answer takes
    /// it, so one intent is at most one write (review NIT-intent).
    queue_intent: Option<QueueIntent>,
    /// MOD-12 D9: the item write in flight and its key, for the status line its answer sets.
    /// Kept apart from `queue_intent`, so the answer never wipes a toggle pressed since.
    queue_writing: Option<(ItemId, String)>,
    /// MOD-115 M2 §7 (T7 repair 2): whether the last frame drew the detail pane. A zoom that
    /// hides it leaves the Runs flow's hit-test canvas where it was last drawn, so the mouse is
    /// declined until the pane is on screen again (`wants_mouse`).
    detail_drawn: Cell<bool>,
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

    /// Where the list, the detail and the open filter form are drawn this frame, from the panes
    /// the shell's layout gave the tab (`areas`) over the tab's whole `area`.
    ///
    /// MOD-13 D1: the open form takes the bottom of the list pane; with the list hidden, the
    /// bottom of the detail's, or of the whole area while the activity pane is zoomed (T7
    /// repair), so a form that captures every key is never off screen; there the list it
    /// filters takes the rest of the area above it (RV3), not a blank body. MOD-13 milestone 2 D8:
    /// the open item form takes the detail pane's place, or the whole area while another pane is
    /// zoomed ([`capture_area`]); so does a sub-tab that captures every key, a compose area or a
    /// picker (T7 repair).
    fn body_areas(&self, area: Rect, areas: PaneAreas) -> (Option<Rect>, Rect, Option<Rect>) {
        let below = |host: Rect| -> (Rect, Rect) {
            Layout::vertical([Constraint::Min(0), Constraint::Length(filter::FORM_HEIGHT)])
                .areas::<2>(host)
                .into()
        };
        let (list, detail, form) = match (self.form.is_some(), areas.list) {
            (true, Some(list)) => {
                let (list, form) = below(list);
                (Some(list), areas.detail, Some(form))
            }
            (true, None) if areas.detail.is_empty() => {
                let (list, form) = below(area);
                (Some(list), areas.detail, Some(form))
            }
            (true, None) => {
                let (detail, form) = below(areas.detail);
                (None, detail, Some(form))
            }
            (false, list) => (list, areas.detail, None),
        };
        let capturing = self.item_form.is_some() || self.detail.captures_input();
        let (list, detail) = capture_area(
            &PaneAreas {
                list,
                detail,
                ..areas
            },
            area,
            capturing,
        );
        (list, detail, form)
    }

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
            queue_intent: None,
            queue_writing: None,
            detail_drawn: Cell::new(true),
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
    /// whole tab, `render`) — and its active sub-tab wants it. MOD-115 M2 §7: not while a zoom
    /// hides the pane, so the wheel there is the list's motion (MD8).
    fn wants_mouse(&self) -> bool {
        self.form.is_none()
            && self.item_form.is_none()
            && self.detail_drawn.get()
            && self.detail.wants_mouse()
    }

    /// MOD-71 D4: to the detail pane, under `wants_mouse`'s form and hidden-pane guards; the
    /// registry checks the sub-tab's own answer. The list takes no mouse event.
    fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
        if self.form.is_some() || self.item_form.is_some() || !self.detail_drawn.get() {
            return Handled::Pass;
        }
        self.detail.on_mouse(mouse, ctx)
    }

    /// MOD-74 D3: to the detail pane, unconditionally (H-12): a form opening is one of the losses.
    fn on_mouse_lost(&mut self) {
        self.detail.on_mouse_lost();
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
        // MOD-67 M5 D5: the active pane's browse stack. Chord equality: a CONTROL or ALT chord
        // is no list or sub-tab act, so `ctrl-l` cycles nothing (D12).
        let stack = self.detail.key_stack();
        let acts = ctx.keys().actions(stack, KeyChord::from_event(key));
        // MOD-115 M2 §7: only motion follows the focus. The list's motion and its fold are the
        // list's while it is focused; the detail steps its own pane; the activity pane takes
        // them and nothing moves. Every other act, the view verbs, acts whatever is focused.
        let focus = ctx.focus();
        // D6, PA-2: a project header folds before any pane sees `Enter` (the fold's own guard
        // declines on an item row).
        if focus == PaneId::List
            && acts.contains(&Act::ListFold)
            && self.fold(ctx) == Handled::Consumed
        {
            return Handled::Consumed;
        }
        // D5: the pane first; it takes its own acts and declines the rest. The Runs pane replays
        // the step under its cursor with `Enter` (MOD-2 D39).
        if self.detail.on_key(key, ctx) == Handled::Consumed {
            return Handled::Consumed;
        }
        for act in acts {
            if Step::by_focus(act, focus, |step| self.detail.step(step, ctx)) {
                return Handled::Consumed;
            }
            match act {
                Act::ListDown => self.step(1, ctx),
                Act::ListUp => self.step(-1, ctx),
                Act::ListTop => self.jump(false, ctx),
                Act::ListBottom => self.jump(true, ctx),
                Act::PaneNextSubtab => self.detail.cycle_next(),
                Act::PanePrevSubtab => self.detail.cycle_prev(),
                // MOD-14 D8, `open graph`. Below the capture guard, so an `m` typed into a note
                // stays the note's.
                Act::BacklogOpenGraph => _ = self.detail.select_id(GraphTab::ID),
                // MOD-13 D1: `f` opens the filter form on the active filter, `F` clears it
                // (consumed either way).
                Act::BacklogOpenFilter => {
                    self.form = Some(FilterForm::open(&self.filter, ctx.projects));
                }
                Act::BacklogClearFilter => {
                    if !self.filter.is_empty() {
                        self.apply(BacklogFilter::default(), ctx);
                    }
                }
                // MOD-13 milestone 2 D7: `N` new item, `e` edit the selected one. Each reads the
                // form's catalogue first (D3); the reply opens the form.
                Act::BacklogNewItem => self.open_new(ctx),
                Act::Edit => self.open_edit(ctx),
                // MOD-12 D9: `Q` toggles the cursor item's queue membership, `P` this box's
                // queue. Each reads the queue first; the reply decides the write.
                Act::BacklogQueue => self.toggle_queued(ctx),
                Act::BacklogPauseQueue => {
                    self.queue_intent = Some(QueueIntent::Pause);
                    ctx.request(StoreRequest::QueueState);
                }
                // D6: neither the fold nor the pane took `Enter`: the miss, its keys named.
                Act::BacklogReplay => ctx.emit(Action::Error(replay_miss(ctx.keys()))),
                // The declined fold, a pane act the pane declined, a global act (the shell's).
                _ => continue,
            }
            return Handled::Consumed;
        }
        // The shell resolves the same stack: `q`, `Tab`, the digits, `?`, `w`, ...
        Handled::Pass
    }

    /// The stack of what holds the Backlog's keys (MOD-67 M5 D4): the filter form's, else the
    /// item form's (its divergence view's while resolving), else the active pane's.
    fn key_stack(&self) -> Option<Stack<'static>> {
        if self.form.is_some() {
            return Some(views::BACKLOG_FILTER);
        }
        if let Some(form) = &self.item_form {
            return Some(form.key_stack());
        }
        Some(self.detail.key_stack())
    }

    /// The active pane's stack, whatever form is open (MOD-115 review): what the about panel
    /// labels the tab's actions through.
    fn browse_stack(&self) -> Stack<'static> {
        self.detail.key_stack()
    }

    /// The keys of what holds the Backlog's keyboard (MOD-67 M6 T2): the divergence view's, the
    /// item form's or the filter form's while one is open; a capturing pane's alone; browsing,
    /// the focused pane's [`DetailTab::key_hint`](detail::DetailTab::key_hint), then ` · ` and
    /// `LIST_HINT` when the joined row fits `width`, the pane's alone when not, `LIST_HINT`
    /// alone when the pane has none (Body, Prompt).
    fn key_hint(&self, width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        let (keys, theme) = (ctx.keys(), ctx.theme);
        let dim = |text| key_line(text, theme);
        if let Some(form) = &self.item_form {
            return dim(match form.resolving() {
                Some(view) => view.hint(keys),
                None => form.key_hint(keys),
            });
        }
        if let Some(form) = &self.form {
            return dim(form.key_hint(keys));
        }
        let pane = self.detail.key_hint(keys, theme);
        let list = self.list_hint(keys).and_then(dim);
        match (pane, list) {
            (Some(pane), Some(list))
                if pane.width() + cell_width(LIST_SEP) + list.width() <= usize::from(width) =>
            {
                let mut spans = styled_spans(pane);
                spans.push(Span::styled(LIST_SEP, theme.dim));
                spans.extend(styled_spans(list));
                Some(Line::from(spans))
            }
            (pane, list) => pane.or(list),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        // MOD-12 D9: the queue read and writes, ahead of the list's.
        match reply {
            StoreReply::Queue(view) => return self.on_queue(view, ctx),
            StoreReply::QueueWritten { write, view } => {
                return self.on_queue_written(write, view, ctx);
            }
            // `App::on_reply` already put the failure on the status line.
            // A refused read spends its toggle; a refused write only its in-flight key, never a
            // toggle pressed since (review NIT-intent).
            StoreReply::Failed { request, .. } if QUEUE_REQUEST_NAMES.contains(request) => {
                if *request == StoreRequest::QueueState.name() {
                    self.queue_intent = None;
                } else {
                    self.queue_writing = None;
                }
                return;
            }
            _ => {}
        }
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
            if let Some(PendingReveal { id, key, focus }) = self.pending_reveal.take() {
                match self.items.iter().find(|item| item.id == id) {
                    Some(item) => {
                        let project = item.project_id;
                        let was_selected = self.selected == Some(Selection::Item(id));
                        self.select_item(id, project, ctx);
                        // MOD-69 plan D8: armed after `go`'s item change, so its reset cannot
                        // eat the focus.
                        if let Some((run, step)) = focus {
                            self.focus_runs(id, run, step, was_selected, ctx);
                        }
                    }
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
        // MOD-13 milestone 5 E4, the `N`/`e` rule one level down: a compose area opens only while
        // neither form captures above it, so the item form and a sub-tab's area are never both
        // open.
        if matches!(
            reply,
            StoreReply::NoteForm { .. } | StoreReply::DocumentForm(_)
        ) && (self.form.is_some() || self.item_form.is_some())
        {
            return;
        }
        self.detail.on_reply(reply, ctx);
    }

    /// MOD-13 milestone 4 D4, milestone 5 D7: the item form when one is open (it ignores an
    /// outcome it did not ask for), else the active detail sub-tab. Both cannot be asking: the
    /// key owner is the asker, and the item form owns keys first.
    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {
        match self.item_form.as_mut() {
            Some(form) => form.on_external_edit(outcome, ctx.keys()),
            None => self.detail.on_external_edit(outcome, ctx),
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

    /// MOD-115 M4 E4: the list, the open filter form and the detail as kit panes, the one that
    /// holds the keyboard focused (C4) and carrying its own keys (Q2).
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let accent = ctx.accent();
        // MOD-13 milestone 3 D4: the divergence view takes the whole tab area, list pane included,
        // as one kit pane (M4 D-9).
        if let Some(view) = self.item_form.as_ref().and_then(ItemForm::resolving) {
            divergence::render(frame, Pane::inner(area), view, theme);
            let title = divergence::title(view);
            let chrome = Chrome {
                focus: hints::holder(ctx, false),
                accent,
                hints: self.detail_pane_hint(area.width, ctx),
            };
            chrome.frame(Pane::new(&title)).render(frame, area, theme);
            return;
        }
        // MOD-115 M2 B5: the panes the shell's layout gives the tab, claimed for the hidden-pane
        // note, the activity placeholder and the mouse; today's 55/45 split by default.
        let areas = ctx.split(area, BACKLOG_LAYOUT);
        // The activity pane is on screen beside the tab's (not zoomed over them): with the focus
        // there, no Backlog pane holds the keyboard (C4).
        let activity = areas.activity.is_some_and(|rect| !rect.is_empty())
            && !(areas.list.is_none() && areas.detail.is_empty());
        let focus = self.focuses(ctx, activity);
        let (list_area, detail_area, form_area) = self.body_areas(area, areas);
        // MOD-13 review M1: the filter the rows were read under, not the one still in flight.
        let summary = self.shown.summary(ctx.projects);
        let keys = ctx.keys();
        let stack = self.detail.key_stack();
        let (new_key, clear_key) = (
            keys.label(stack, Act::BacklogNewItem),
            keys.label(stack, Act::BacklogClearFilter),
        );
        let view = ListView {
            items: &self.items,
            projects: ctx.projects,
            folded: &self.folded,
            selected: self.selected,
            filter: summary.as_deref(),
            new_key: new_key.as_deref(),
            clear_key: clear_key.as_deref(),
        };
        if let Some(list_area) = list_area {
            let chrome = Chrome {
                focus: focus.list,
                accent,
                hints: self.list_pane_hint(ctx),
            };
            list::render(frame, list_area, &view, chrome, theme);
        }
        if let (Some(form), Some(at)) = (&self.form, form_area) {
            let chrome = Chrome {
                focus: focus.filter,
                accent,
                hints: self.filter_pane_hint(ctx),
            };
            filter::render(frame, at, form, chrome, theme);
        }
        let chrome = Chrome {
            focus: focus.detail,
            accent,
            hints: self.detail_pane_hint(detail_area.width, ctx),
        };
        match &self.item_form {
            Some(form) => {
                // MOD-115 M4 D-9: the form's rows in a kit pane in the detail's place; the rect
                // it claims is inside the pane's inner rect, as it was inside the block's.
                let claim = item_form::render(frame, Pane::inner(detail_area), form, accent, theme);
                ctx.claim_editor_area(claim);
                let title = item_form::title(form);
                chrome
                    .frame(Pane::new(&title))
                    .render(frame, detail_area, theme);
            }
            None if detail_area.is_empty() => {}
            None => detail::render(frame, detail_area, &self.detail, self.item(), chrome, ctx),
        }
        self.detail_drawn
            .set(self.item_form.is_none() && !detail_area.is_empty());
    }

    /// MOD-115 M2 D2: a list and a detail, today's 55/45 by default.
    fn panes(&self) -> Panes {
        Panes::ListDetail(BACKLOG_LAYOUT)
    }

    /// MOD-115 M2 §8: the active sub-tab, by id and title, and the row under the cursor.
    fn location(&self) -> TabLoc {
        let cursor = match self.selected {
            Some(Selection::Item(id)) => {
                let key = self
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .map_or("", |item| item.key.as_str());
                Some(format!("{ITEM_CURSOR}:{id}:{key}"))
            }
            Some(Selection::Project(id)) => Some(format!("{PROJECT_CURSOR}:{id}")),
            None => None,
        };
        TabLoc {
            subtab: self.detail.active_id().map(|id| id.0),
            subtab_title: self.detail.active().map(|tab| tab.title().to_owned()),
            cursor,
        }
    }

    /// MOD-115 M2 §8: back to a [`location`](Tab::location): its sub-tab, then its item through
    /// the reveal path (unfolded, read, or read again when the list no longer holds it), or its
    /// project header while the list shows it. Nothing while a form or a capturing sub-tab holds
    /// the keys: a move would drop what it is typing.
    fn restore(&mut self, loc: &TabLoc, ctx: &mut Ctx<'_>) -> bool {
        if self.form.is_some() || self.item_form.is_some() || self.detail.captures_input() {
            return false;
        }
        let subtab = loc
            .subtab
            .is_some_and(|id| self.detail.select_id(DetailId(id)));
        let row = match loc.cursor.as_deref().and_then(parse_cursor) {
            Some((Selection::Item(id), key)) => self.reveal(&RevealTarget::Item { id, key }, ctx),
            Some((Selection::Project(id), _))
                if self.rows(ctx).contains(&Selection::Project(id)) =>
            {
                self.go(Some(Selection::Project(id)), ctx);
                true
            }
            Some((Selection::Project(_), _)) | None => false,
        };
        subtab || row
    }

    fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
        let (id, key, focus) = match target {
            RevealTarget::Item { id, key } => (*id, key, None),
            RevealTarget::Step {
                item,
                key,
                run,
                step,
            } => (*item, key, Some((*run, *step))),
            RevealTarget::Requirement { .. } => return false,
        };
        // A half-typed note or reject reason would be lost by the move (`go` resets the sub-tabs),
        // and so would a half-edited filter or a half-typed item.
        if self.form.is_some() || self.item_form.is_some() || self.detail.captures_input() {
            ctx.emit(Action::Error(CLOSE_THE_FIELD_FIRST.to_owned()));
            return true;
        }
        match self.items.iter().find(|item| item.id == id) {
            Some(item) => {
                let project = item.project_id;
                let was_selected = self.selected == Some(Selection::Item(id));
                self.pending_reveal = None;
                self.select_item(id, project, ctx);
                // MOD-69 plan D8: the focus is armed after `go`'s item change, which resets the
                // pane; `on_item_change` keeps the active sub-tab, so Runs stays open.
                if let Some((run, step)) = focus {
                    self.focus_runs(id, run, step, was_selected, ctx);
                }
            }
            // Not loaded, or not in the rows read so far: re-read and decide on arrival (D251).
            // MOD-13 D4: the re-read is unfiltered. The tab cannot tell "hidden by the filter"
            // from "not in the workspace" before the reply, so the filter goes either way, and
            // D251's error then speaks for the whole workspace.
            None => {
                self.filter = BacklogFilter::default();
                self.pending_reveal = Some(PendingReveal {
                    id,
                    key: key.clone(),
                    focus,
                });
                ctx.request(self.filter.to_request(ctx.scope));
            }
        }
        true
    }
}

/// The D6 miss (MOD-67 M5): "select a step in the Runs pane (J/K) to replay it" under the
/// defaults, the step keys read through the Runs browse stack, since the miss fires from any
/// sub-tab.
fn replay_miss(keys: &Keys) -> String {
    const STEP_KEYS: HintSpec = &[Hint::Pair(Act::RunsNextStep, Act::RunsPrevStep, "")];
    match keys.hint(views::RUNS_BROWSE, STEP_KEYS) {
        moves if moves.is_empty() => "select a step in the Runs pane to replay it".to_owned(),
        moves => format!("select a step in the Runs pane ({moves}) to replay it"),
    }
}

/// The Backlog's sentence for a reveal of an item this workspace does not hold (MOD-64 D235, F5).
#[must_use]
pub fn not_in_this_backlog(key: &str) -> String {
    format!("{key} is not in this workspace's backlog")
}

/// The body region split into the list pane and the detail pane by the default layout: what
/// the tests that size a pane measure against. Just above the tests module, so the source
/// guards that read the file up to its first `#[cfg(test)]` (`keys/views.rs`) read every
/// production item.
#[cfg(test)]
fn panes(area: Rect) -> [Rect; 2] {
    let areas = crate::ui::shell::layout::areas(area, &BACKLOG_LAYOUT, PaneId::List);
    [areas.list.unwrap_or_default(), areas.detail]
}

#[cfg(test)]
mod tests;
