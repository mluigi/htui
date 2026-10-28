//! The Backlog tab's tests (MOD-115 M4 T1): the shared fixtures and probes, then one module per
//! concern.
#![cfg(test)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::app::{Emit, TopBarState};
use crate::item_writes::ItemWrite;
use crate::keys::Keys;
use crate::store_worker::{Origin, QueueView, QueueWrite};
use crate::testkit::DEFAULT_SIZE;
use crate::ui::Theme;
use crate::ui::kit::scroll::Step;
use crate::ui::layout::chrome;
use crate::ui::shell::{ListSize, PaneId, Preset};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab};
use crate::ui::tabs::backlog::item_form::{
    Busy, ItemFormOutcome, mint_may_have_landed, mint_may_have_landed_in_the_old_scope,
};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use htui_core::model::{ItemFilter, Status};
use htui_core::store::{MemStore, ReadStore as _, WriteStore as _};

/// The queue requests among `actions`, by name; a `queue_item` names its item and the
/// workspace it records (MOD-125 D3) too.
fn queue_requests(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Store(StoreRequest::QueueItem { item, workspace }) => {
                Some(format!("queue_item {item} in {workspace}"))
            }
            Action::Store(request) if QUEUE_REQUEST_NAMES.contains(&request.name()) => {
                Some(request.name().to_owned())
            }
            _ => None,
        })
        .collect()
}

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

/// The Platform fixture a `Ctx` borrows from, for the filter cases.
struct Bench {
    scope: Scope,
    projects: Vec<htui_core::model::ProjectRef>,
    items: Vec<ItemSummary>,
    first: Selection,
    top_bar: TopBarState,
    keys: Keys,
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
            keys: Keys::compiled().clone(),
            theme: Theme::default(),
            emit: Emit::default(),
        }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx::new(
            &self.scope,
            &self.projects,
            &self.top_bar,
            &self.keys,
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
        statuses: vec![Status::Done],
        ..BacklogFilter::default()
    }
}

/// A sub-tab that wants the mouse while `wants` is set and logs what it is offered (MOD-71).
#[derive(Debug)]
struct MouseProbe {
    id: DetailId,
    wants: Rc<Cell<bool>>,
    seen: Pointed,
    /// How many times it was told capture went off (MOD-74 D3).
    lost: Lost,
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
    fn on_mouse_lost(&mut self) {
        self.lost.set(self.lost.get() + 1);
    }
}

/// What a [`MouseProbe`] was offered.
type Pointed = Rc<RefCell<Vec<MouseEventKind>>>;

/// How many capture losses a [`MouseProbe`] was told of (MOD-74 D3).
type Lost = Rc<Cell<usize>>;

/// A [`MouseProbe`] under `id`, wanting the mouse as `wants` says, and its three handles.
fn mouse_probe(
    id: &'static str,
    wants: bool,
) -> (Box<dyn DetailTab>, Rc<Cell<bool>>, Pointed, Lost) {
    let wants = Rc::new(Cell::new(wants));
    let seen = Rc::default();
    let lost = Lost::default();
    let probe = MouseProbe {
        id: DetailId(id),
        wants: Rc::clone(&wants),
        seen: Rc::clone(&seen),
        lost: Rc::clone(&lost),
    };
    (Box::new(probe), wants, seen, lost)
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

/// The `Runs` reads among `actions`, by item.
fn runs_reads(actions: &[Action]) -> Vec<ItemId> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::Store(StoreRequest::Runs(id)) => Some(*id),
            _ => None,
        })
        .collect()
}

/// `FEAT-1`'s `review` step, as a waiting-list row names it.
fn feat_1_review() -> RevealTarget {
    RevealTarget::Step {
        item: htui_core::fixtures::ids::HTUI_FEAT_1,
        key: "FEAT-1".to_owned(),
        run: Some(htui_core::fixtures::ids::RUN_1),
        step: Some(htui_core::fixtures::ids::STEP_REVIEW),
    }
}

/// The tab drawn at the harness's size, as snapshot text.
fn drawn(tab: &BacklogTab, bench: &Bench) -> String {
    let (width, height) = DEFAULT_SIZE;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
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
fn with_status(bench: &Bench, status: Status) -> Vec<ItemSummary> {
    bench
        .items
        .iter()
        .filter(|item| item.status == status)
        .cloned()
        .collect()
}

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

/// The edit form on the first item with ` v2` typed and saved: the one `EditItem` sent.
async fn an_edit_saved(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) -> StoreRequest {
    open_with(tab, bench, store, KeyCode::Char('e')).await;
    type_into(tab, bench, " v2");
    saved(tab, bench)
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

/// The new form on the first item's project with `Fresh item` typed and saved.
async fn a_mint_saved(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) -> StoreRequest {
    open_with(tab, bench, store, KeyCode::Char('N')).await;
    type_into(tab, bench, "Fresh item");
    saved(tab, bench)
}

/// `e` on the first item, `Theirs` written through the store at v1, ` mine` typed and the
/// stale save answered: the view is open. The first item's id and key.
async fn in_the_view(tab: &mut BacklogTab, bench: &Bench, store: &MemStore) -> (ItemId, String) {
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

/// From the open form's Title, `Tab` five times to Body: Priority, Tags, Graph, Paths, Body.
fn to_body(tab: &mut BacklogTab, bench: &Bench) {
    for _ in 0..5 {
        press(tab, bench, KeyCode::Tab);
    }
}

/// A sub-tab that records the `$EDITOR` outcomes it is handed.
#[derive(Debug, Default)]
struct OutcomeProbe {
    seen: Rc<RefCell<Vec<ExternalEditOutcome>>>,
}

impl DetailTab for OutcomeProbe {
    fn id(&self) -> DetailId {
        DetailId("outcomes")
    }
    fn title(&self) -> &str {
        "Outcomes"
    }
    fn on_item_change(&mut self, _item: Option<ItemId>) {}
    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        Handled::Pass
    }
    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        self.seen.borrow_mut().push(outcome);
    }
}

/// A sub-tab that starts capturing when a `NoteForm` reply reaches it, as a compose area
/// opens (E4), without depending on the Notes pane.
#[derive(Debug, Default)]
struct ComposeProbe {
    capturing: Rc<Cell<bool>>,
}

impl DetailTab for ComposeProbe {
    fn id(&self) -> DetailId {
        DetailId("compose")
    }
    fn title(&self) -> &str {
        "Compose"
    }
    fn on_item_change(&mut self, _item: Option<ItemId>) {}
    fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        if self.capturing.get() {
            Handled::Consumed
        } else {
            Handled::Pass
        }
    }
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if matches!(reply, StoreReply::NoteForm { .. }) {
            self.capturing.set(true);
        }
    }
    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    fn captures_input(&self) -> bool {
        self.capturing.get()
    }
}

/// A registry holding `tab` alone.
fn only(tab: impl DetailTab + 'static) -> DetailRegistry {
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(tab));
    detail
}

/// The list hint under the compiled defaults.
const DEFAULT_LIST_HINT: &str =
    "j/k move \u{b7} h/l sub-tab \u{b7} f filter \u{b7} N new \u{b7} e edit";

/// The Backlog's row of the shell's hint row at `width` cells, as text (MOD-67 M6 T2).
fn row_at(tab: &BacklogTab, bench: &Bench, width: u16) -> Option<String> {
    tab.key_hint(width, &bench.ctx())
        .map(|line| line.to_string())
}

/// A sub-tab that records the steps it takes and the keys it is offered, passing every key.
#[derive(Debug)]
struct StepProbe {
    steps: Rc<RefCell<Vec<Step>>>,
    keys: Rc<RefCell<Vec<KeyCode>>>,
}

impl DetailTab for StepProbe {
    fn id(&self) -> DetailId {
        DetailId("probe")
    }
    fn title(&self) -> &str {
        "Probe"
    }
    fn on_item_change(&mut self, _item: Option<ItemId>) {}
    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
        self.keys.borrow_mut().push(key.code);
        Handled::Pass
    }
    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    fn step(&mut self, step: Step, _ctx: &Ctx<'_>) -> Handled {
        self.steps.borrow_mut().push(step);
        Handled::Consumed
    }
}

/// What a [`StepProbe`] took.
type Steps = Rc<RefCell<Vec<Step>>>;

/// What a [`StepProbe`] was offered.
type Offered = Rc<RefCell<Vec<KeyCode>>>;

/// The settled tab with a [`StepProbe`] as its only sub-tab, and the probe's two logs.
fn stepping(bench: &Bench) -> (BacklogTab, Steps, Offered) {
    let (steps, keys): (Steps, Offered) = (Rc::default(), Rc::default());
    let mut detail = DetailRegistry::new();
    detail.register(Box::new(StepProbe {
        steps: Rc::clone(&steps),
        keys: Rc::clone(&keys),
    }));
    let tab = BacklogTab {
        detail,
        ..bench.tab()
    };
    (tab, steps, keys)
}

/// The tab drawn at the harness's size through `ctx`, as snapshot text.
fn drawn_through(tab: &BacklogTab, ctx: &Ctx<'_>) -> String {
    let (width, height) = DEFAULT_SIZE;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("a test terminal");
    terminal
        .draw(|frame| tab.render(frame, frame.area(), ctx))
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

/// The first project's id: the project `bench.first` sits under.
fn first_project(bench: &Bench) -> ProjectId {
    bench
        .items
        .iter()
        .find(|item| Selection::Item(item.id) == bench.first)
        .map(|item| item.project_id)
        .expect("the first row is an item")
}

mod filter;
mod hints;
mod input;
mod navigation;
mod queue;
mod render;
mod writes;
