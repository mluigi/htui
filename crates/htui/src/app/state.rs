//! The shell's state: [`App`], the [`Ctx`] handed to views, and the top bar's projection.

use std::collections::HashMap;
use std::mem::Discriminant;

use htui_core::model::{ProjectRef, Scope, WorkspaceId};
use htui_worker::WaitingView;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::cell::{Cell, RefCell};
use tokio::sync::mpsc;

use crate::app::action::{
    Action, Handled, HistoryAction, LayoutAction, OverlayAction, PaneAction, RevealKind, TabAction,
};
use crate::app::pane::OpenEditor;
use crate::editor::{EDITOR_BUSY, ExternalEdit, ExternalEditOutcome};
use crate::keys::translate::translate;
use crate::keys::{Act, Context, HelpLine, KeyChord, Keys, Stack};
use crate::store_worker::{Origin, RequestEnvelope, Seq, StoreRequest};
use crate::ui::cells::clip_line;
use crate::ui::header::{self, Flash, ShellChips};
use crate::ui::kit::empty::{NextStep, placeholder};
use crate::ui::kit::pane::{Focus, paint_bottom_hints, paint_bottom_note, paint_focus};
use crate::ui::overlay::{Overlay, OverlayRegistry, OverlayStack};
use crate::ui::shell::history::{History, Loc};
use crate::ui::shell::layout::{self as pane_layout, NoteLabels, Seam};
use crate::ui::shell::{PaneAreas, PaneId, Panes, TabLayout};
use crate::ui::state_file::{UiState, UiStore};
use crate::ui::tabs::{Tab, TabId, TabRegistry};
use crate::ui::{Theme, about, layout};
use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use zeroize::Zeroizing;

/// What the status line says when anything but a tab asks for `$EDITOR` (MOD-9 D10): the outcome
/// is handed back through `Tab::on_external_edit`, so there would be no one to hand it to.
pub const EDITOR_NEEDS_A_TAB: &str = "only a tab can open the editor";

/// How many rounds of "apply an action, collect what it emitted" one drain may take before the
/// shell gives up. A view that emits on every drain round is a bug, not a reason to hang.
const DRAIN_ROUNDS: usize = 16;

/// What the top bar shows (`R-TUI-1`). Only `App::observe_reply` writes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopBarState {
    /// Name of the workspace in scope, empty before the first `SetScope`.
    pub workspace: String,
    /// Hostname of this box, empty until the `BoxInfo` reply lands.
    pub box_name: String,
    /// `Backend::label()`: `"memory"` in MOD-1, `"online"` / `"offline · 3m"` in MOD-6.
    pub store: String,
    /// MOD-69 plan D6: the last `Waiting` reply, which the top bar counts and the overlay lists;
    /// `None` until the first one (blueprint A-6).
    pub waiting: Option<WaitingView>,
}

/// The action sink handed to views.
///
/// A view holds no channel and no store handle (`R-NF-3`): it pushes actions here and the shell
/// applies them once the view has returned, which is also why a view can emit while the shell
/// still holds it mutably borrowed.
#[derive(Debug, Default)]
pub struct Emit(RefCell<Vec<Action>>);

impl Emit {
    /// Queues an action.
    pub fn push(&self, action: Action) {
        self.0.borrow_mut().push(action);
    }

    /// Takes everything queued so far, leaving the queue empty.
    #[must_use]
    pub fn take(&self) -> Vec<Action> {
        self.0.take()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

/// Everything a view is allowed to see, plus the way back out.
#[derive(Debug)]
pub struct Ctx<'a> {
    /// The workspace scope every read is issued against (plan D10).
    pub scope: &'a Scope,
    /// The scope's projects, ordered by `workspace_project.position`.
    pub projects: &'a [ProjectRef],
    /// What the top bar currently shows.
    pub top_bar: &'a TopBarState,
    /// The palette.
    pub theme: &'a Theme,
    /// The keys in force (MOD-67 D9): `App`'s own at the shell's sites, a bench's in tests.
    keys: &'a Keys,
    /// Who the shell is talking to; replies to this view's requests come back addressed to it.
    origin: Origin,
    /// The action sink.
    emit: &'a Emit,
    /// MOD-57 P2: where the active tab names its editing rect; `None` at every other site.
    editor_area: Option<&'a Cell<Option<Rect>>>,
    /// MOD-115 M2 D3: the tab's focused pane, as the shell keeps it; `None` until the shell sets
    /// one (B7: then the list).
    focus: Option<PaneId>,
    /// MOD-115 M2 D6: the open in-pane edit's label, for the leave dialog; `None` when no edit
    /// is open.
    open_edit: Option<&'a str>,
    /// MOD-115 M2 D2: the tab's layout as the shell keeps it; `None` until the shell sets one
    /// (B7: then the tab's own default, [`Ctx::split`]).
    layout: Option<TabLayout>,
    /// MOD-115 M2 B5: where [`Ctx::split`] records the panes the tab claimed this frame, and the
    /// region it split; `None` at every site but the active tab's render.
    claim: Option<&'a Cell<Option<PaneClaim>>>,
}

/// What a list + detail tab claimed in one frame (MOD-115 M2 B5): the region it split and the
/// panes the shell's layout gave it.
pub type PaneClaim = (Rect, PaneAreas);

/// What the shell keeps for one tab and hands it on every [`Ctx`] it builds for it (MOD-115 M2
/// D2, D3, T6R): the focused pane, always a visible one, and a list + detail tab's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TabView {
    /// The focused pane: the one the shell keeps, else the list (list + detail) or the detail
    /// (single pane); the detail when the layout hides it.
    pub(crate) focus: PaneId,
    /// The stored layout, else the tab's default; `None` for a single-pane tab.
    pub(crate) layout: Option<TabLayout>,
}

impl TabView {
    /// `tab`'s view out of the shell's focus map, UI state and last claim. One function, so the
    /// render, the keys, the mouse, the history and the header all agree (T6R).
    fn of(
        tab: &dyn Tab,
        focus: &HashMap<TabId, PaneId>,
        ui: &UiState,
        last_claim: Option<(TabId, Rect, PaneAreas)>,
    ) -> Self {
        let layout = match tab.panes() {
            Panes::ListDetail(default) => {
                Some(ui.layouts.get(tab.id().0).copied().unwrap_or(default))
            }
            Panes::Single => None,
        };
        let kept = focus.get(&tab.id()).copied().unwrap_or(match tab.panes() {
            Panes::ListDetail(_) => PaneId::List,
            Panes::Single => PaneId::Detail,
        });
        // D3: a pane the layout hides (a saved preset or toggle, before any key moved the
        // focus) gives the focus to the detail, as a layout key that hides it does.
        let focus = match layout {
            Some(layout) if hides(last_claim, tab.id(), &layout, kept) => PaneId::Detail,
            _ => kept,
        };
        Self { focus, layout }
    }
}

/// Whether `layout` hides `pane` of tab `id` (MOD-115 M2 D3), drawn over the region the tab
/// split last, else one that holds every pane: only which panes are shown matters here.
fn hides(
    last_claim: Option<(TabId, Rect, PaneAreas)>,
    id: TabId,
    layout: &TabLayout,
    pane: PaneId,
) -> bool {
    let region = last_claim
        .filter(|(claimed, _, _)| *claimed == id)
        .map_or(Rect::new(0, 0, 200, 60), |(_, region, _)| region);
    let areas = pane_layout::areas(region, layout, pane);
    !pane_layout::visible(&areas).contains(&pane)
}

impl<'a> Ctx<'a> {
    /// Builds the context of one view. The shell is the only caller.
    #[must_use]
    pub fn new(
        scope: &'a Scope,
        projects: &'a [ProjectRef],
        top_bar: &'a TopBarState,
        keys: &'a Keys,
        theme: &'a Theme,
        origin: Origin,
        emit: &'a Emit,
    ) -> Self {
        Self {
            scope,
            projects,
            top_bar,
            keys,
            theme,
            origin,
            emit,
            editor_area: None,
            focus: None,
            open_edit: None,
            layout: None,
            claim: None,
        }
    }

    /// This context with the tab's layout as the shell keeps it (MOD-115 M2 D2). Only the shell
    /// calls it, for a list + detail tab.
    #[must_use]
    pub const fn with_layout(mut self, layout: TabLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// Lends this context the cell [`split`](Self::split) records its claim in (MOD-115 M2 B5).
    /// Only `App::render` calls it, for the active tab's render.
    #[must_use]
    pub const fn with_pane_claim(mut self, cell: &'a Cell<Option<PaneClaim>>) -> Self {
        self.claim = Some(cell);
        self
    }

    /// MOD-115 M2 B5: `region` split into the tab's panes, by the layout the shell keeps for the
    /// tab, else `default` (the tab's own, so a bench draws as before the shell kept layouts).
    /// Called where the tab split its body itself; during the active tab's render the shell
    /// records the claim for focus borders, the hidden-pane note, the activity placeholder, the
    /// hints and the mouse. The last claim of a frame wins.
    #[must_use]
    pub fn split(&self, region: Rect, default: TabLayout) -> PaneAreas {
        let areas =
            crate::ui::shell::layout::areas(region, &self.layout.unwrap_or(default), self.focus());
        if let Some(cell) = self.claim {
            cell.set(Some((region, areas)));
        }
        areas
    }

    /// This context with the tab's focused pane (MOD-115 M2 D3). Only the shell calls it.
    #[must_use]
    pub const fn with_focus(mut self, focus: PaneId) -> Self {
        self.focus = Some(focus);
        self
    }

    /// This context with what the shell keeps for its tab (MOD-115 M2 D2, D3, T6R): the focused
    /// pane, and the layout of a list + detail tab. Only the shell calls it, through
    /// `App::tab_ctx` and `App::call_tab`, so no tab's context goes without them.
    #[must_use]
    pub(crate) const fn with_tab_view(mut self, view: TabView) -> Self {
        self.focus = Some(view.focus);
        self.layout = view.layout;
        self
    }

    /// This context with the open in-pane edit's label, or none (MOD-115 M2 D6).
    #[must_use]
    pub const fn with_open_edit(mut self, open_edit: Option<&'a str>) -> Self {
        self.open_edit = open_edit;
        self
    }

    /// The tab's focused pane (MOD-115 M2 D3): the list unless the shell set another (B7), so
    /// every bench behaves as before the shell kept focus.
    #[must_use]
    pub fn focus(&self) -> PaneId {
        self.focus.unwrap_or(PaneId::List)
    }

    /// The open in-pane edit's label, if an edit is open (MOD-115 M2 D6).
    #[must_use]
    pub const fn open_edit(&self) -> Option<&'a str> {
        self.open_edit
    }

    /// Lends this context the cell [`claim_editor_area`](Self::claim_editor_area) writes. Only
    /// `App::render` calls it, for the active tab's render.
    #[must_use]
    pub fn with_editor_area(mut self, cell: &'a Cell<Option<Rect>>) -> Self {
        self.editor_area = Some(cell);
        self
    }

    /// MOD-57 P2: names `area` as the rect this view's text is edited in, so an in-pane editor the
    /// view asked for draws over it. A no-op outside the active tab's render; the last claim of a
    /// frame wins.
    pub fn claim_editor_area(&self, area: Rect) {
        if let Some(cell) = self.editor_area {
            cell.set(Some(area));
        }
    }

    /// The keys in force, for hint rows and labels (M3-M5). The borrow lives as long as the
    /// context's data (`'a`), not as long as this `Ctx`.
    #[must_use]
    pub fn keys(&self) -> &'a Keys {
        self.keys
    }

    /// Asks the store. The reply comes back to this view through `on_reply`, unless a newer
    /// request of the same kind overtook it (blueprint C.2).
    pub fn request(&self, request: StoreRequest) {
        self.emit.push(Action::Store(request));
    }

    /// Emits an action.
    pub fn emit(&self, action: Action) {
        self.emit.push(action);
    }

    /// Who this view is.
    #[must_use]
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
}

/// The whole shell.
#[derive(Debug)]
pub struct App {
    /// The workspace in scope.
    pub scope: Scope,
    /// The scope's projects, in position order.
    pub projects: Vec<ProjectRef>,
    /// The top bar's projection.
    pub top_bar: TopBarState,
    /// Registered tabs, in strip order.
    pub tabs: TabRegistry,
    /// Open overlays, bottom first.
    pub overlays: OverlayStack,
    /// How an [`OverlayId`](crate::ui::overlay::OverlayId) becomes an overlay.
    pub overlay_factories: OverlayRegistry,
    /// The named-action keys in force (MOD-67 D10): the compiled defaults; M2 builds them from
    /// `keys.toml`.
    pub keys: Keys,
    /// Global actions that open a named view, offered by `register_all` (D5). An action not offered
    /// is neither dispatched nor shown.
    pub(super) offered: Vec<(Act, Action)>,
    /// The palette: `Theme::default()` until `run` applies `NO_COLOR` (MOD-80 D4).
    pub theme: Theme,
    /// Whether the help box is up.
    pub help_visible: bool,
    /// Whether the about panel under the header is up (MOD-115). In memory only: it starts on
    /// and M2 persists it.
    pub about_visible: bool,
    /// Last error text, shown on the status line.
    pub status: Option<String>,
    /// Set by `Action::Quit`; the event loop stops after the current iteration.
    pub should_quit: bool,
    /// Set by every state change, cleared by the draw that follows it.
    pub dirty: bool,
    /// What views emitted while the shell held them borrowed.
    pub(super) emit: Emit,
    /// The worker's inbox.
    pub(super) requests: mpsc::UnboundedSender<RequestEnvelope>,
    /// Newest `seq` per `(origin, request kind)`: the staleness index of blueprint C.2.
    pub(super) latest: HashMap<(Origin, Discriminant<StoreRequest>), Seq>,
    /// Next `seq` to stamp.
    pub(super) next_seq: Seq,
    /// Ticks since startup; the top bar refreshes every fourth one.
    pub(super) ticks: u64,
    /// Overlay to open when the first `Workspaces` reply is empty (blueprint D, "Startup").
    ///
    /// Set by [`register_all`](crate::app::register_all); `None` leaves an empty store on the
    /// bare shell. Held as an id, not a view, so the shell never names a concrete overlay.
    pub startup_overlay: Option<crate::ui::overlay::OverlayId>,
    /// Overlay to open when a `StoreState` reply carries a pending-migration count (plan D11).
    ///
    /// Set by [`register_all`](crate::app::register_all), and an id for the same reason
    /// `startup_overlay` is one: the shell must not name a concrete view (MOD-1 blueprint D).
    pub migration_overlay: Option<crate::ui::overlay::OverlayId>,
    /// The leave dialog (MOD-115 M2 D6, `R-TUI-18`; it replaced MOD-57 M2 E4's quit question):
    /// opened by `global.quit` (`q`, `ctrl-c`) on every screen. Set by
    /// [`register_all`](crate::app::register_all), an id for `migration_overlay`'s reason; `None`
    /// (a bare shell) quits at once, as M1 did.
    pub leave_overlay: Option<crate::ui::overlay::OverlayId>,
    /// The abort question (MOD-57 M2 R1 M-2): opened by `editor.abort` on the asking tab. Set by
    /// [`register_all`](crate::app::register_all), an id for `migration_overlay`'s reason; `None`
    /// (a bare shell) aborts at once, as M1 did.
    pub abort_overlay: Option<crate::ui::overlay::OverlayId>,
    /// Tab that shows a replayed step (MOD-2 D39, `R-HIS-2`).
    ///
    /// Set by [`register_all`](crate::app::register_all), and an id for the same reason
    /// `migration_overlay` is one: `Action::Replay` has to focus a view and address a reply to
    /// it, and the shell must not name a concrete one to do so. `None` means this build can
    /// replay nothing, which the status line says rather than the shell swallowing the key.
    pub replay_tab: Option<TabId>,
    /// Which tab reveals which kind of entity (MOD-64 D235). Set by
    /// [`register_all`](crate::app::register_all), ids for the reason `replay_tab` is one: the
    /// shell names no concrete view. A kind with no entry reveals nothing.
    pub reveal_tabs: Vec<(RevealKind, TabId)>,
    /// Whether the migration prompt has already been offered this session.
    ///
    /// `StoreState` is re-read every fourth tick, so without this an answered `n` would re-open
    /// the prompt a second later, forever (blueprint D.5).
    pub(super) migration_prompt_shown: bool,
    /// Whether the below-the-target notice has been shown this session (MOD-40 plan D9).
    ///
    /// `StoreState` is re-read every fourth tick; without this a notice the next key cleared
    /// would come back a second later, `migration_prompt_shown`'s reason.
    pub(super) below_target_shown: bool,
    /// Whether the no-DSN redirect has already fired this session (MOD-15 M6 D6).
    ///
    /// `ConnectionInfo` is re-issued whenever the shell asks again, so without this a box with an
    /// empty keyring would be dragged back to `Settings > Connection` from wherever the user had
    /// moved on to — the defect `migration_prompt_shown` exists to prevent, one reply across.
    pub(super) connection_redirect_done: bool,
    /// The `$EDITOR` handoff a tab asked for, and which tab (MOD-9 D10).
    ///
    /// Set by `drain` from an `Action::EditExternally` a tab emitted; taken by the event loop,
    /// which suspends the terminal, runs the editor and hands the outcome back through
    /// [`App::finish_external_edit`]. One slot: a second ask before the loop comes round replaces
    /// the first.
    pub(super) pending_edit: Option<(TabId, ExternalEdit)>,
    /// MOD-74 D1: mouse capture as the loop last applied it, the edge `mouse_capture` detects.
    /// `take_external_edit` clears it for the editor's `leave` (D2).
    pub(super) mouse: bool,
    /// MOD-57: the in-pane editor, while one is alive (one at a time, P6).
    pub(super) editor: Option<OpenEditor>,
    /// MOD-57 P2: the active tab's claim this frame. `None` at the top of every `render`.
    pub(super) editor_area: Cell<Option<Rect>>,
    /// MOD-57 P2: where the active tab's pane goes, as the last frame computed it; read by
    /// `open_editor` (the spawn size) and `resize_editor`.
    pub(super) editor_rect: Option<(TabId, Rect)>,
    /// MOD-115 M2 D1: the UI state the shell keeps and saves: the about panel's and each tab's
    /// layout (keyed by `TabId.0`), and the modal sizes read back and written again.
    pub(crate) ui: UiState,
    /// MOD-115 M2 D3: the focused pane per tab; a tab with none focuses its list (a list + detail
    /// tab) or its detail (a single-pane one).
    pub(crate) focus: HashMap<TabId, PaneId>,
    /// MOD-115 M2 B5: the active tab's claim this frame. `None` at the top of every `render`.
    pub(super) pane_claim: Cell<Option<PaneClaim>>,
    /// MOD-115 M2 B5: the panes the active tab claimed in the last frame, and the region it
    /// split, for the pane keys, the layout keys and the mouse (like `editor_rect`).
    pub(crate) last_claim: Option<(TabId, Rect, PaneAreas)>,
    /// MOD-115 M2 D1: where the UI state is saved. `App::new` keeps it in memory, so a test never
    /// touches the home directory; only `run` hands it the real `ui-state.toml`.
    pub ui_store: UiStore,
    /// MOD-115 M2 D5: the places visited this session, in memory.
    pub(crate) history: History,
    /// MOD-115 M2 §8: set while an outer entry (`on_key`, `on_mouse`, `update`) records, so the
    /// `update`s it nests do not record again.
    pub(super) recording: bool,
    /// MOD-115 M2 §8: set by `history.back`/`forward`, so the move itself is not recorded.
    pub(super) history_move: bool,
    /// MOD-115 M2 §7: the seam being dragged, from its `Down` to its `Up`.
    pub(super) drag: Option<SeamDrag>,
    /// MOD-74 D1, MOD-115 M2 MD8: the tab whose own gesture holds the mouse (its
    /// [`Tab::wants_mouse`] said yes while capture was on), so a capture that stays on across a
    /// switch between two list + detail tabs still tells the one that held it.
    pub(super) mouse_view: Option<TabId>,
}

/// A seam drag in progress (MOD-115 M2 §7): the tab, the seam and the offset from the cell
/// grabbed to the line the layout's setter reads (the list's right border, the activity pane's
/// top or left border), so grabbing either of the seam's two lines never jumps it a cell (T2
/// carried finding 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SeamDrag {
    tab: TabId,
    seam: Seam,
    dx: i32,
    dy: i32,
}

impl App {
    /// A shell with no tabs, no overlays and no workspace yet.
    ///
    /// The caller registers views ([`register_all`](crate::app::register_all)), sets
    /// `top_bar.store` from `Backend::label()` and calls [`App::start`].
    #[must_use]
    pub fn new(requests: mpsc::UnboundedSender<RequestEnvelope>) -> Self {
        Self {
            scope: Scope {
                workspace_id: WorkspaceId::default(),
                project_ids: Vec::new(),
            },
            projects: Vec::new(),
            top_bar: TopBarState::default(),
            tabs: TabRegistry::new(),
            overlays: OverlayStack::new(),
            overlay_factories: OverlayRegistry::new(),
            keys: Keys::compiled().clone(),
            offered: Vec::new(),
            theme: Theme::default(),
            help_visible: false,
            about_visible: true,
            status: None,
            should_quit: false,
            dirty: true,
            emit: Emit::default(),
            requests,
            latest: HashMap::new(),
            next_seq: 0,
            ticks: 0,
            startup_overlay: None,
            migration_overlay: None,
            leave_overlay: None,
            abort_overlay: None,
            replay_tab: None,
            reveal_tabs: Vec::new(),
            migration_prompt_shown: false,
            below_target_shown: false,
            connection_redirect_done: false,
            pending_edit: None,
            mouse: false,
            editor: None,
            editor_area: Cell::new(None),
            editor_rect: None,
            ui: UiState::default(),
            focus: HashMap::new(),
            pane_claim: Cell::new(None),
            last_claim: None,
            ui_store: UiStore::memory(),
            history: History::default(),
            recording: false,
            history_move: false,
            drag: None,
            mouse_view: None,
        }
    }

    /// Takes the UI state read at startup (MOD-115 M2 D1): the about panel as it was left (a
    /// state with no about entry keeps the first-launch default) and every tab's layout.
    pub fn apply_ui_state(&mut self, state: UiState) {
        if let Some(about) = state.about {
            self.about_visible = about;
        }
        self.ui = state;
        self.dirty = true;
    }

    /// Writes the UI state the moment it changes (MOD-115 M2 D1): the about panel and every
    /// layout. A save that fails says why in the header and changes nothing on screen.
    pub(super) fn save_ui_state(&mut self) {
        self.ui.about = Some(self.about_visible);
        if let Err(reason) = self.ui_store.save(&self.ui) {
            self.status = Some(reason);
        }
    }

    /// Where the user is (MOD-115 M2 §8): the active tab, its focused pane, the tab's own
    /// location, and the header's label `title[ › sub-tab][ · list|activity]` (the pane named
    /// when it is not the detail). `None` with no tab.
    #[must_use]
    pub(crate) fn here(&self) -> Option<Loc> {
        let tab = self.tabs.active()?;
        let pane = self.focus_of(tab);
        let at = tab.location();
        let mut label = tab.title().to_owned();
        if let Some(subtab) = &at.subtab_title {
            label.push_str(" \u{203a} ");
            label.push_str(subtab);
        }
        match pane {
            PaneId::List => label.push_str(" \u{b7} list"),
            PaneId::Activity => label.push_str(" \u{b7} activity"),
            PaneId::Detail => {}
        }
        Some(Loc {
            tab: tab.id(),
            pane,
            at,
            label,
        })
    }

    /// Runs `step` and records the move it made (MOD-115 M2 D5): when the coarse place changed,
    /// the place left goes on the history. Nested calls (the `update`s a key drains) only run,
    /// and a `history.back`/`forward` is not recorded.
    pub(super) fn recorded(&mut self, step: impl FnOnce(&mut Self)) {
        if self.recording {
            step(self);
            return;
        }
        self.recording = true;
        let before = self.here();
        step(self);
        self.recording = false;
        if std::mem::take(&mut self.history_move) {
            return;
        }
        if let (Some(before), Some(after)) = (before, self.here()) {
            self.history.record(before, &after);
        }
    }

    /// What the shell keeps for `tab` ([`TabView`]): its focused pane and its layout.
    pub(crate) fn tab_view(&self, tab: &dyn Tab) -> TabView {
        TabView::of(tab, &self.focus, &self.ui, self.last_claim)
    }

    /// The layout the shell keeps for `tab` (MOD-115 M2 D2): the stored one, else the tab's own
    /// default; `None` for a single-pane tab.
    pub(crate) fn layout_of(&self, tab: &dyn Tab) -> Option<TabLayout> {
        self.tab_view(tab).layout
    }

    /// The focused pane of `tab` (MOD-115 M2 D3): the one the shell keeps, else the list of a
    /// list + detail tab and the detail of a single-pane one; always a pane the layout shows
    /// (the detail when it hides that one, T6R).
    pub(crate) fn focus_of(&self, tab: &dyn Tab) -> PaneId {
        self.tab_view(tab).focus
    }

    /// Whether `layout` hides `pane` of tab `id` (MOD-115 M2 D3), over the region it split last.
    pub(super) fn hides(&self, id: TabId, layout: &TabLayout, pane: PaneId) -> bool {
        hides(self.last_claim, id, layout, pane)
    }

    /// The context of `tab`, with its focused pane and layout (T6R): the read-only sites (the
    /// render, the hints).
    #[must_use]
    pub(super) fn tab_ctx(&self, tab: &dyn Tab) -> Ctx<'_> {
        self.ctx(Origin::Tab(tab.id()))
            .with_tab_view(self.tab_view(tab))
    }

    /// Hands tab `id` its [`Ctx`] (its focused pane and layout, T6R) for `call`, then drains what
    /// it emitted, addressed as that tab. Every mutable entry into a tab goes through here (keys,
    /// mouse, paste, replies, refresh, reveal, restore, the editor's outcome). `None` when no tab
    /// `id` is registered.
    pub(super) fn call_tab<R>(
        &mut self,
        id: TabId,
        call: impl FnOnce(&mut dyn Tab, &mut Ctx<'_>) -> R,
    ) -> Option<R> {
        let origin = Origin::Tab(id);
        let result = {
            let Self {
                scope,
                projects,
                top_bar,
                keys,
                theme,
                emit,
                tabs,
                focus,
                ui,
                last_claim,
                ..
            } = self;
            let tab = tabs.by_id_mut(id)?;
            let view = TabView::of(&*tab, focus, ui, *last_claim);
            let mut ctx = Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit)
                .with_tab_view(view);
            call(tab, &mut ctx)
        };
        self.drain(&origin);
        Some(result)
    }

    /// The shell's paint over the panes the active tab claimed (MOD-115 M2 §6.4): the activity
    /// placeholder, then each visible pane's border restyled for focus (the tab accent on the
    /// focused one, the faint border on the others).
    fn paint_panes(&self, frame: &mut Frame<'_>, areas: &PaneAreas) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let stack = tab.key_stack().unwrap_or(Stack::BASE);
        if let Some(activity) = areas.activity.filter(|rect| !rect.is_empty()) {
            let next = self
                .keys
                .reachable_chord(stack, Act::LayoutToggleActivity)
                .map(|chord| chord.label())
                .map_or_else(
                    || ACTIVITY_NEXT_UNBOUND.to_owned(),
                    |key| format!("{key} {ACTIVITY_NEXT}"),
                );
            placeholder(
                frame,
                activity,
                ACTIVITY_TITLE,
                ACTIVITY_WHAT,
                NextStep(&next),
                &self.theme,
            );
        }
        let focus = self.focus_of(tab);
        let accent = self.theme.tab_accent(tab.id().0);
        for pane in pane_layout::visible(areas) {
            if let Some(rect) = focused_rect(areas, pane) {
                let state = if pane == focus {
                    Focus::Focused
                } else {
                    Focus::Unfocused
                };
                paint_focus(frame.buffer_mut(), rect, state, accent, &self.theme);
            }
        }
    }

    /// R-TUI-12: what a hidden pane says about bringing it back, right-aligned on the bottom
    /// border of the detail, or of the zoomed pane while the detail is zoomed away (T2 carried
    /// finding 3). Painted after the hints, so it keeps one rule cell before them (finding 2).
    /// Only keys that reach their act are named: a part whose key a narrower act takes (`p` in
    /// Backlog > Runs, blueprint D-1) is left out.
    fn paint_hidden_note(&self, frame: &mut Frame<'_>, areas: &PaneAreas) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let stack = tab.key_stack().unwrap_or(Stack::BASE);
        let label = |act| {
            self.keys
                .reachable_chord(stack, act)
                .map(|chord| chord.label())
        };
        let labels = NoteLabels {
            toggle_list: label(Act::LayoutToggleList),
            toggle_activity: label(Act::LayoutToggleActivity),
            next_preset: label(Act::LayoutNextPreset),
            zoom: label(Act::LayoutZoom),
        };
        let (Some(note), Some(area)) = (
            pane_layout::hidden_note(areas, &labels),
            pane_layout::note_area(areas),
        ) else {
            return;
        };
        if area.height < 2 {
            return;
        }
        let row = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        paint_bottom_note(frame.buffer_mut(), row, Line::from(note), self.theme.dim);
    }

    /// Issues the reads the shell itself needs: the workspace list, this box's row, the store
    /// state and the connection.
    ///
    /// The first `Workspaces` reply also picks the startup scope (blueprint D, "Startup"); the
    /// `StoreState` one is what makes the top bar right on the first frame rather than a second
    /// later, and what carries a pending-migration count into the shell (plan D11).
    ///
    /// `ConnectionInfo` is the shell's own read rather than the section's (MOD-15 M6 D6): a box
    /// whose keyring is empty has to be steered to the field *before* the user finds the Settings
    /// tab, and no section that has never been activated can ask for anything. It is addressed
    /// under `Origin::App`, so the section's own read — which is keyed by origin — is not stale to
    /// it and it is not stale to the section's.
    pub fn start(&mut self) {
        self.dispatch(Origin::App, StoreRequest::Workspaces);
        self.dispatch(Origin::App, StoreRequest::BoxInfo);
        self.dispatch(Origin::App, StoreRequest::StoreState);
        self.dispatch(Origin::App, StoreRequest::ConnectionInfo);
    }

    /// Registers a tab and, when it becomes the active one, issues its requests.
    pub fn register_tab(&mut self, tab: Box<dyn Tab>) {
        let first = self.tabs.is_empty();
        self.tabs.register(tab);
        if first {
            self.activate_tab();
        }
        self.dirty = true;
    }

    /// Pushes an overlay and issues the requests it asks for.
    pub fn push_overlay(&mut self, overlay: Box<dyn Overlay>) {
        let id = overlay.id();
        let requests = overlay.wants_requests(&self.scope);
        self.overlays.push(overlay);
        for request in requests {
            self.dispatch(Origin::Overlay(id), request);
        }
        self.dirty = true;
    }

    /// Offers a global action that names a view (MOD-67 D5), replacing an earlier offer of `act`.
    /// Only `register_all` calls it: the shell itself never names a concrete overlay.
    pub fn offer(&mut self, act: Act, action: Action) {
        debug_assert!(Self::is_offerable(act), "{act:?} is not offerable");
        self.offered.retain(|(known, _)| *known != act);
        self.offered.push((act, action));
    }

    /// The global actions [`offer`](Self::offer) takes: those that open a named view. Every other
    /// global action is [`action_for`](Self::action_for)'s fixed mapping.
    const fn is_offerable(act: Act) -> bool {
        matches!(act, Act::Workspaces | Act::Find | Act::Waiting | Act::Queue)
    }

    /// What the shell does for `act`: the fixed mapping, then the offered table. `None` for
    /// everything a view must handle (every shared context) and for an offerable act nobody
    /// offered.
    pub(super) fn action_for(&self, act: Act) -> Option<Action> {
        match act {
            // MOD-115 M2 D6: the leave dialog, always.
            Act::Quit => Some(self.request_quit()),
            Act::NextTab => Some(Action::Tab(TabAction::Next)),
            Act::PrevTab => Some(Action::Tab(TabAction::Prev)),
            Act::Help => Some(Action::ToggleHelp),
            Act::About => Some(Action::ToggleAbout),
            Act::OverlayClose => Some(Action::Overlay(OverlayAction::Close)),
            // MOD-115 M2 D3, D2, D5: the pane, layout and history keys are the shell's.
            Act::NextPane => Some(Action::Pane(PaneAction::Next)),
            Act::PrevPane => Some(Action::Pane(PaneAction::Prev)),
            Act::LayoutNextPreset => Some(Action::Layout(LayoutAction::NextPreset)),
            Act::LayoutPrevPreset => Some(Action::Layout(LayoutAction::PrevPreset)),
            Act::LayoutGrowList => Some(Action::Layout(LayoutAction::GrowList)),
            Act::LayoutShrinkList => Some(Action::Layout(LayoutAction::ShrinkList)),
            Act::LayoutGrowActivity => Some(Action::Layout(LayoutAction::GrowActivity)),
            Act::LayoutShrinkActivity => Some(Action::Layout(LayoutAction::ShrinkActivity)),
            Act::LayoutToggleList => Some(Action::Layout(LayoutAction::ToggleList)),
            Act::LayoutToggleActivity => Some(Action::Layout(LayoutAction::ToggleActivity)),
            Act::LayoutZoom => Some(Action::Layout(LayoutAction::Zoom)),
            Act::HistoryBack => Some(Action::History(HistoryAction::Back)),
            Act::HistoryForward => Some(Action::History(HistoryAction::Forward)),
            other => other
                .tab_index()
                .map(|index| Action::Tab(TabAction::Select(index)))
                .or_else(|| {
                    self.offered
                        .iter()
                        .find(|(known, _)| *known == other)
                        .map(|(_, action)| action.clone())
                }),
        }
    }

    /// Whether `act` does something here (the `?` box's `offered`): the shell maps it, and a pane
    /// or layout key only on a tab with a list and a detail (MOD-115 M2 D2, D3: on a single-pane
    /// tab they do nothing, so the box does not list them).
    pub(super) fn offers(&self, act: Act) -> bool {
        match self.action_for(act) {
            Some(Action::Pane(_) | Action::Layout(_)) => self
                .tabs
                .active()
                .is_some_and(|tab| matches!(tab.panes(), Panes::ListDetail(_))),
            Some(_) => true,
            None => false,
        }
    }

    /// The stack of the view that holds the keyboard (MOD-67 D4, PA-8): the top overlay's when
    /// an overlay is up, else the active tab's. `None` while that view is unconverted: the
    /// status line and the `?` box are then today's.
    #[must_use]
    pub fn active_stack(&self) -> Option<Stack<'static>> {
        match self.overlays.top() {
            Some(top) => top.key_stack(),
            None => self.tabs.active().and_then(Tab::key_stack),
        }
    }

    /// D6 steps 2 and 6: the first candidate the shell maps to an action wins. `true` if one did.
    fn apply_keys(&mut self, stack: Stack<'_>, chord: KeyChord) -> bool {
        let action = self
            .keys
            .actions(stack, chord)
            .into_iter()
            .find_map(|act| self.action_for(act));
        match action {
            Some(action) => {
                self.update(action);
                true
            }
            None => false,
        }
    }

    /// Issues the active tab's `wants_requests`: called on activation and after a scope change.
    pub(super) fn activate_tab(&mut self) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let id = tab.id();
        let requests = tab.wants_requests(&self.scope);
        for request in requests {
            self.dispatch(Origin::Tab(id), request);
        }
    }

    /// Stamps a request with a fresh `seq`, records it as the newest of its kind for this origin
    /// and hands it to the worker.
    pub(super) fn dispatch(&mut self, origin: Origin, request: StoreRequest) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.latest
            .insert((origin.clone(), std::mem::discriminant(&request)), seq);
        let envelope = RequestEnvelope {
            seq,
            origin,
            request,
        };
        if self.requests.send(envelope).is_err() {
            tracing::warn!("store worker is gone; request dropped");
        }
    }

    /// Whether a reply is still the newest of its kind for its origin (blueprint C.2).
    ///
    /// `seq` values are globally unique and each one is recorded under exactly one
    /// `(origin, request kind)` key, so "is this `seq` still in the index for this origin?" is
    /// the same question as "is `latest[(origin, kind)] == seq`?" without having to recover the
    /// request kind from the reply.
    pub(super) fn is_fresh(&self, origin: &Origin, seq: Seq) -> bool {
        self.latest
            .iter()
            .any(|((known, _), newest)| known == origin && *newest == seq)
    }

    /// Forgets every staleness entry of `origin` (MOD-64 D240), so no reply to a request made
    /// before this can pass [`App::is_fresh`] again.
    pub(super) fn forget(&mut self, origin: &Origin) {
        self.latest.retain(|(known, _), _| known != origin);
    }

    /// Applies every action views emitted while the shell held them borrowed.
    ///
    /// `Action::Store` is stamped with `origin` here: that is why the action itself carries none.
    pub(super) fn drain(&mut self, origin: &Origin) {
        for _ in 0..DRAIN_ROUNDS {
            let actions = self.emit.take();
            if actions.is_empty() {
                return;
            }
            for action in actions {
                match action {
                    Action::Store(request) => self.dispatch(origin.clone(), request),
                    // Stamped like `Store` (MOD-9 D10): the outcome goes back to the tab that
                    // asked, and nothing but a tab can be handed one.
                    Action::EditExternally(edit) => match origin {
                        Origin::Tab(id) => {
                            self.pending_edit = Some((*id, edit));
                            self.dirty = true;
                        }
                        _ => self.update(Action::Error(EDITOR_NEEDS_A_TAB.to_owned())),
                    },
                    // MOD-57 M2 R2 LOW-2: only the abort question confirms an abort. The bare
                    // shell's direct abort (no question registered) goes through `update`, not
                    // here.
                    Action::AbortEdit => match origin {
                        Origin::Overlay(id) if self.abort_overlay == Some(*id) => {
                            self.update(Action::AbortEdit);
                        }
                        _ => tracing::warn!(?origin, "AbortEdit from outside the abort question"),
                    },
                    other => self.update(other),
                }
            }
        }
        if !self.emit.is_empty() {
            tracing::warn!("emit queue did not settle; dropping the rest");
            let _ = self.emit.take();
        }
    }

    /// The edit a tab asked for, taken by the event loop (MOD-9 D10). `Some` exactly when the loop
    /// is about to suspend, and `Suspend::leave` turns capture off without the app knowing: so a
    /// capture that was on is lost here (MOD-74 D2), and the loop's next `mouse_capture` re-enables
    /// it with no stale gesture.
    ///
    /// MOD-57 PD-8: while an in-pane editor is alive a second edit is not handed to the loop; the
    /// asking tab is answered `Failed(EDITOR_BUSY)` here, so a view waiting for one outcome gets
    /// exactly one.
    pub fn take_external_edit(&mut self) -> Option<(TabId, ExternalEdit)> {
        if self.editor.is_some()
            && let Some((tab, _)) = self.pending_edit.take()
        {
            self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_BUSY.to_owned()));
            return None;
        }
        let edit = self.pending_edit.take();
        if edit.is_some() && self.mouse {
            self.lose_mouse();
            self.mouse = false;
            self.mouse_view = None;
            self.end_drag();
        }
        edit
    }

    /// Routes the editor's outcome to the tab that asked, with a [`Ctx`], then drains what it
    /// emitted, as `on_reply` does. A tab that is no longer registered drops it. Sets `dirty`.
    pub fn finish_external_edit(&mut self, tab: TabId, outcome: ExternalEditOutcome) {
        self.dirty = true;
        if self
            .call_tab(tab, |view, ctx| view.on_external_edit(outcome, ctx))
            .is_none()
        {
            tracing::debug!(%tab, "the tab that asked for the editor is gone");
        }
    }

    /// A terminal event. Key presses, bracketed pastes and (while a view wants them, MOD-71 D4)
    /// mouse events reach views; a resize just asks for a redraw.
    pub fn on_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.on_key(key);
            }
            // Wrapped before anything reads it: a paste may be a credential (MOD-22's redirect),
            // and this buffer is wiped however the paste ends.
            Event::Paste(text) => self.on_paste(&Zeroizing::new(text)),
            Event::Mouse(mouse) => self.on_mouse(mouse),
            Event::Resize(_, _) => self.dirty = true,
            _ => {}
        }
    }

    /// A bracketed paste (MOD-22 review M-1): the propagation chain of [`on_key`](Self::on_key)
    /// with **no key table in it**. The top overlay, then — unless that overlay is modal — the active
    /// tab, each taking it only into a field it is capturing input with; whatever none of them
    /// takes is dropped and does nothing else. So a paste made before a field is open is never a
    /// key: its digits switch no tab, its `/` opens no filter, its `q` quits nothing.
    ///
    /// Unlike a key it does not clear the status line: a paste nothing took did nothing, and one a
    /// field refused says why through the same line.
    ///
    /// MOD-57: while an in-pane editor is alive a paste follows a key's rule (R0 F2): to the
    /// editor while it has the keys, else swallowed (the first that finds it without them) or,
    /// on the asking tab, dropped; under a modal overlay the overlay's path below has it, and on
    /// another tab that tab's (M2 E3). It never reaches the asking view.
    pub fn on_paste(&mut self, text: &Zeroizing<String>) {
        self.dirty = true;
        if self.editor.is_some() && self.editor_paste(text) {
            return;
        }

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        top.on_paste(text, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed || self.overlays.top().is_some_and(Overlay::is_modal) {
                return;
            }
        }

        if let Some(id) = self.tabs.active_id() {
            // `Pass` is the drop: there is no key table for a paste to fall through to.
            let _ = self.call_tab(id, |tab, ctx| tab.on_paste(text, ctx));
        }
    }

    /// MOD-71 D1: whether the view on screen wants the mouse, which
    /// [`mouse_capture`](Self::mouse_capture) records and the event loop applies after every
    /// step. No overlay may be open and the `?` box may not be up — both draw over the tab, and a
    /// click through them would act on what they hide (blueprint E7) — and the active tab must
    /// want it.
    ///
    /// MOD-57 M2 E8: off while the in-pane editor is on screen (the pane draws over the view, and
    /// the terminal keeps its own selection over it); on another tab the view decides.
    ///
    /// MOD-115 M2 MD8: a tab with a list and a detail wants it too, so a click focuses a pane and
    /// a seam drag resizes it; a single-pane tab (Chat, Settings) keeps the terminal's selection.
    #[must_use]
    pub fn wants_mouse(&self) -> bool {
        !self.editor_visible()
            && self.overlays.is_empty()
            && !self.help_visible
            && self
                .tabs
                .active()
                .is_some_and(|tab| tab.wants_mouse() || matches!(tab.panes(), Panes::ListDetail(_)))
    }

    /// MOD-74 D1: what the event loop hands `TerminalGuard::set_mouse_capture` after every step:
    /// [`wants_mouse`](Self::wants_mouse), recorded. On an on-to-off edge every registered tab is
    /// told ([`Tab::on_mouse_lost`]): after a tab switch the tab holding the gesture is no longer
    /// the active one.
    ///
    /// A "loss" is the on-to-off capture edge, or (below) a change of the gesture holder.
    ///
    /// MOD-115 M2 MD8: list + detail tabs want the mouse too, so capture can stay on across a
    /// tab switch; the gesture holder ([`Tab::wants_mouse`] while capture is on) is tracked, and
    /// every tab is told when it changes or goes, as on the off edge. A seam drag ends with
    /// capture.
    #[must_use = "the loop must hand this to TerminalGuard::set_mouse_capture"]
    pub fn mouse_capture(&mut self) -> bool {
        let wants = self.wants_mouse();
        let holder = self
            .tabs
            .active()
            .filter(|tab| wants && tab.wants_mouse())
            .map(Tab::id);
        if (self.mouse && !wants) || (self.mouse_view.is_some() && self.mouse_view != holder) {
            self.lose_mouse();
        }
        if !wants {
            self.end_drag();
        }
        self.mouse = wants;
        self.mouse_view = holder;
        wants
    }

    /// Ends a seam drag (MOD-115 M2 §7) and saves the layout it left on screen (D1): on its
    /// `Up`, and where its `Up` will never reach the shell: capture going off (an overlay, the
    /// help box, the editor), the loop suspending for an external edit, or a new press after a
    /// release the terminal lost. `true` when a drag was in progress.
    fn end_drag(&mut self) -> bool {
        let ended = self.drag.take().is_some();
        if ended {
            self.save_ui_state();
        }
        ended
    }

    /// MOD-74 D1, D2: tells every tab capture went off.
    fn lose_mouse(&mut self) {
        for tab in self.tabs.iter_mut() {
            tab.on_mouse_lost();
        }
    }

    /// A mouse event (MOD-71 D4): to the active tab only, with no key table and no overlay in the
    /// chain (an open overlay turns capture off, D1).
    ///
    /// Gated first, so an event queued before capture went off does nothing. `Moved` (Windows still
    /// reports motion, and a terminal may ignore button-only mode, MOD-74 D5) and the horizontal
    /// wheel are dropped before dispatch. Only a `Consumed` event sets `dirty` and clears the
    /// status line (blueprint E8): a pointer crossing the canvas must neither redraw once per cell
    /// nor wipe an error nobody acted on. The status is taken before dispatch, as
    /// [`on_key`](Self::on_key) clears it, so a failure the event causes still lands.
    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        self.recorded(|app| app.dispatch_mouse(mouse));
    }

    /// [`on_mouse`](Self::on_mouse)'s body, recorded by it.
    fn dispatch_mouse(&mut self, mouse: MouseEvent) {
        if !self.wants_mouse() {
            return;
        }
        if matches!(
            mouse.kind,
            MouseEventKind::Moved | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
        ) {
            return;
        }
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        // MOD-115 M2 §7: the shell's own gestures first: a seam drag, a click that focuses a
        // pane (still handed on, the Runs flow takes clicks).
        if self.shell_mouse(id, mouse) {
            self.dirty = true;
            return;
        }
        // Only a tab whose own gesture wants the mouse is handed it (MOD-71 D4). For the others
        // the wheel over a pane is the keymap's list motion there (MD8).
        if !self.tabs.active().is_some_and(Tab::wants_mouse) {
            if let Some(key) = self.wheel_key(id, mouse) {
                self.dispatch_key(key);
            }
            return;
        }
        let status = self.status.take();
        let handled = self
            .call_tab(id, |tab, ctx| tab.on_mouse(mouse, ctx))
            .unwrap_or(Handled::Pass);
        if handled == Handled::Consumed {
            self.dirty = true;
        } else if self.status.is_none() {
            self.status = status;
        }
    }

    /// MOD-115 M2 MD8: capture is on for every list + detail tab, so the terminal no longer turns
    /// the wheel into arrow keys (alternate scroll). For a tab with no gesture of its own, the
    /// wheel over a pane it claimed focuses that pane and stands for the first chord that
    /// reaches `list.down`/`list.up` through the tab's stack, so a rebound motion key is what
    /// the wheel presses. `None` off every pane, for any other event, or with no such chord.
    fn wheel_key(&mut self, id: TabId, mouse: MouseEvent) -> Option<KeyEvent> {
        let act = match mouse.kind {
            MouseEventKind::ScrollDown => Act::ListDown,
            MouseEventKind::ScrollUp => Act::ListUp,
            _ => return None,
        };
        let (_, _, areas) = self.last_claim.filter(|(claimed, _, _)| *claimed == id)?;
        let pane = pane_layout::hit(&areas, mouse.column, mouse.row)?;
        let stack = self.tabs.active()?.key_stack().unwrap_or(Stack::BASE);
        let chord = self.keys.reachable_chord(stack, act)?;
        self.focus.insert(id, pane);
        Some(chord.to_event())
    }

    /// The shell's mouse (MOD-115 M2 §7), over the panes the active tab `id` claimed last frame:
    /// a left press on a seam starts a drag (consumed), one in a pane focuses it (passed on); a
    /// drag moves the seam; the release ends it and saves the layout. `true` when consumed.
    fn shell_mouse(&mut self, id: TabId, mouse: MouseEvent) -> bool {
        use crossterm::event::MouseButton;

        let (x, y) = (mouse.column, mouse.row);
        let claim = self.last_claim.filter(|(claimed, _, _)| *claimed == id);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // A drag whose `Up` never came ends here, saved.
                self.end_drag();
                let Some((_, _, areas)) = claim else {
                    return false;
                };
                if let Some(seam) = pane_layout::seam(&areas, x, y) {
                    let (dx, dy) = seam_offset(&areas, seam, x, y);
                    self.drag = Some(SeamDrag {
                        tab: id,
                        seam,
                        dx,
                        dy,
                    });
                    return true;
                }
                if let Some(pane) = pane_layout::hit(&areas, x, y) {
                    self.focus.insert(id, pane);
                    self.dirty = true;
                }
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(drag) = self.drag.filter(|drag| drag.tab == id) else {
                    return false;
                };
                let (Some((_, region, _)), Some(tab)) = (claim, self.tabs.active()) else {
                    return true;
                };
                let Some(mut layout) = self.layout_of(tab) else {
                    return true;
                };
                let (x, y) = (shifted(x, drag.dx), shifted(y, drag.dy));
                match drag.seam {
                    Seam::List => layout.set_list_at(region, x),
                    Seam::Activity => layout.set_activity_at(region, x, y),
                }
                self.ui.layouts.insert(id.0.to_owned(), layout);
                true
            }
            MouseEventKind::Up(MouseButton::Left) => self.end_drag(),
            _ => false,
        }
    }

    /// The propagation chain (blueprint C.4, MOD-67 D6), stopping at the first consumer: the
    /// in-pane editor (MOD-57), the `[field]` translation (MOD-115 M2 D7), the top overlay, the
    /// overlay's stack (`Stack::OVERLAY` while unconverted), the modal swallow, the active tab,
    /// the tab's stack (`Stack::BASE` while unconverted). Each dispatch is recorded in the
    /// history (MOD-115 M2 D5).
    ///
    /// MOD-115 M2 D7: `ctrl-c` is `global.quit`'s default chord like any other, resolved through
    /// the stacks: every overlay's stack has a quit layer (B4), so it opens the leave dialog from
    /// every screen, and the dialog's `leave.confirm` takes the second. A bracketed paste has no
    /// key table ([`on_paste`](Self::on_paste)), so a pasted `U+0003` never quits.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.recorded(|app| app.dispatch_key(key));
    }

    /// [`on_key`](Self::on_key)'s body, recorded by it.
    fn dispatch_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        // An error survives until the user does something, then the help line comes back. Cleared
        // before the key is dispatched, so a failure this key causes still lands.
        self.status = None;
        let chord = KeyChord::from_event(key);

        // MOD-57 P5, P6, M2 E1-E3: a live in-pane editor answers first. Focused, it takes every
        // key, `ctrl-c` included (`R-TUI-10`'s terminal-pane clause; `editor.focus` is the way
        // out); on the asking tab, the editor's and the global keys only; on another tab, only
        // `editor.focus`. A modal overlay's keys take the path below.
        if self.editor.is_some() && self.editor_key(key, chord) {
            return;
        }

        // MOD-115 M2 D7 (blueprint §3): while the view holding the keyboard captures text, a
        // chord bound to a `[field]` row becomes that row's canonical editing event, and a
        // canonical key no row binds any more is not delivered. The focused in-pane editor above
        // never goes through it: it takes raw keys.
        let Some(key) = self
            .active_stack()
            .map_or(Some(key), |stack| translate(&self.keys, stack, key))
        else {
            return;
        };
        let chord = KeyChord::from_event(key);

        if let Some(id) = self.overlays.top().map(Overlay::id) {
            let origin = Origin::Overlay(id);
            let handled = {
                let Self {
                    scope,
                    projects,
                    top_bar,
                    keys,
                    theme,
                    emit,
                    overlays,
                    ..
                } = self;
                match overlays.top_mut() {
                    Some(top) => {
                        let mut ctx =
                            Ctx::new(scope, projects, top_bar, keys, theme, origin.clone(), emit);
                        top.on_key(key, &mut ctx)
                    }
                    None => Handled::Pass,
                }
            };
            self.drain(&origin);
            if handled == Handled::Consumed {
                return;
            }
            // `Esc` closes; `?`/`F1` toggle help over any overlay, for a key the overlay passed.
            // Before the modal swallow: under a future non-modal overlay, help pre-empts the active
            // tab for a key the overlay passed (D6). The first non-modal overlay revisits this.
            // A converted overlay resolves through its own stack (MOD-67 M3 D6), which carries
            // the layer filter.
            let stack = self
                .overlays
                .top()
                .and_then(Overlay::key_stack)
                .unwrap_or(Stack::OVERLAY);
            if self.apply_keys(stack, chord) {
                return;
            }
            // A modal overlay swallows what it did not handle: the tab below never sees it.
            if self.overlays.top().is_some_and(Overlay::is_modal) {
                return;
            }
        }

        if let Some(id) = self.tabs.active_id()
            && self.call_tab(id, |tab, ctx| tab.on_key(key, ctx)) == Some(Handled::Consumed)
        {
            return;
        }

        // The active tab's stack (MOD-67 M3 D6), else the base stack. The tab's, not
        // `active_stack()`: a key reaches this step only when no modal overlay swallowed it.
        let stack = self
            .tabs
            .active()
            .and_then(Tab::key_stack)
            .unwrap_or(Stack::BASE);
        self.apply_keys(stack, chord);
    }

    /// The context of one view.
    #[must_use]
    pub(super) fn ctx(&self, origin: Origin) -> Ctx<'_> {
        Ctx::new(
            &self.scope,
            &self.projects,
            &self.top_bar,
            &self.keys,
            &self.theme,
            origin,
            &self.emit,
        )
    }

    /// The global keys the status line listed before MOD-115, as text: the global actions the
    /// active view's stack admits and the shell offers, chords from [`Keys`]. Nothing draws it
    /// any more (the header's chips and the `?` box carry the same keys); tests read it to pin how
    /// a view's stack and a rebind shape what a user can press. Compiled only for tests and the
    /// `testkit` feature.
    #[cfg(any(test, feature = "testkit"))]
    #[must_use]
    pub fn keys_line(&self) -> String {
        self.keys
            .status_line(self.active_stack().unwrap_or(Stack::BASE), |act| {
                self.offers(act)
            })
    }

    /// The header's shell chips (MOD-115 M2 §8, `R-TUI-1`): `⌫ back to <place>` while there is
    /// somewhere back, `Alt→ forward` while there is somewhere forward, each labelled with the
    /// first chord that reaches its act through the stack holding the keyboard (no chip where
    /// none does), and the active list + detail tab's layout (`· zoom` while zoomed).
    #[must_use]
    pub(super) fn shell_chips(&self) -> ShellChips {
        let stack = self.active_stack().unwrap_or(Stack::BASE);
        let back = self
            .history
            .peek_back()
            .zip(self.keys.reachable_label(stack, Act::HistoryBack))
            .map(|(loc, key)| {
                (
                    format!("{key} back to {}", loc.label),
                    format!("{key} back"),
                )
            });
        let forward = self
            .history
            .can_forward()
            .then(|| self.keys.reachable_label(stack, Act::HistoryForward))
            .flatten()
            .map(|key| format!("{key} forward"));
        let layout = self
            .tabs
            .active()
            .and_then(|tab| self.layout_of(tab))
            .map(|layout| {
                if layout.zoom {
                    format!("{} \u{b7} zoom", layout.preset.label())
                } else {
                    layout.preset.label().to_owned()
                }
            });
        ShellChips {
            back,
            forward,
            layout,
        }
    }

    /// The header's flash (MOD-115): what used to be the status line's text, now on the right of
    /// the header. The first of the last error ([`App::status`], error style), the in-pane
    /// editor's line ([`App::editor_status`], dim) and another tab's `editing in <Tab> · ...`
    /// notice ([`App::editor_notice`], dim, its trailing separator stripped); `None` when none of
    /// them is up. The global help line is gone: `?` opens the box and the header carries the
    /// `F9 about ? keys` chips.
    #[must_use]
    pub(super) fn flash(&self) -> Option<Flash> {
        if let Some(message) = &self.status {
            return Some(Flash {
                text: message.clone(),
                style: self.theme.error,
            });
        }
        let text = self.editor_status().or_else(|| {
            self.editor_notice().map(|notice| {
                notice
                    .strip_suffix(" \u{b7} ")
                    .map_or(notice.clone(), str::to_owned)
            })
        })?;
        Some(Flash {
            text,
            style: self.theme.dim,
        })
    }

    /// Draws one frame: the one-row header, the about panel when it is up, the active tab, the
    /// in-pane editor over it (MOD-57 P1), the active tab's key hints on the body's bottom border
    /// (MOD-115 M1 adapter), then the overlays bottom-up and the help box on top.
    ///
    /// The hints are the active tab's [`Tab::key_hint`]; they are left off under any overlay (the
    /// overlay's box has its own hint, plan Q1 (a)) and while the in-pane editor's tab is on
    /// screen (focused, the editor has the keys; unfocused, the tab refuses its own,
    /// [`Stack::EDITOR_OPEN`]). The `?` box leaves them alone: the box is centred over the body.
    ///
    /// Rendering is read-only by contract: a view that emits here is not applied until the next
    /// key or reply drains the queue.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        self.dirty = false;
        self.editor_area.set(None);
        self.pane_claim.set(None);
        let area = frame.area();
        // A tab with no about text (a test double, a reserved tab) leaves no empty hole under the
        // header: the panel's rows go to the body.
        let about = self
            .tabs
            .active()
            .filter(|_| self.about_visible)
            .and_then(|tab| about::about_for(tab.id()).map(|text| (tab, text)));
        let chrome = layout::chrome_with(area, about.is_some());

        let flash = self.flash();
        let chips = self.shell_chips();
        // M1 wires no mouse handler to the header's hit rects.
        let _hits = header::render(
            frame,
            chrome.header,
            &self.top_bar,
            &self.tabs,
            flash.as_ref(),
            &self.keys,
            // The chips are labelled through the stack that holds the keyboard, so `F9 about`
            // is not offered under an overlay that does not take it.
            self.active_stack().unwrap_or(Stack::BASE),
            &chips,
            &self.theme,
        );
        if let (Some(panel), Some((tab, text))) = (chrome.about, about) {
            // The tab's browse stack, not the mode it is in: an open form or composer must not
            // blank the actions.
            let stack = tab.browse_stack();
            about::render(frame, panel, text, &self.keys, stack, &self.theme);
        }

        match self.tabs.active() {
            Some(tab) => {
                let ctx = self
                    .tab_ctx(tab)
                    .with_editor_area(&self.editor_area)
                    .with_pane_claim(&self.pane_claim);
                tab.render(frame, chrome.body, &ctx);
            }
            None => {
                frame.render_widget(
                    Paragraph::new(Line::styled("No tab is registered.", self.theme.dim))
                        .block(Block::new().borders(Borders::ALL)),
                    chrome.body,
                );
            }
        }
        // MOD-115 M2 B5: what the active tab claimed, for the keys and the mouse until the next
        // frame; a single-pane tab claims nothing.
        let claim = self.pane_claim.get();
        self.last_claim = self
            .tabs
            .active_id()
            .zip(claim)
            .map(|(id, (region, areas))| (id, region, areas));
        if let Some((_, areas)) = claim {
            self.paint_panes(frame, &areas);
        }

        self.draw_editor(frame, chrome.body);

        // The M1 hint adapter: tabs still frame their own bodies, so the hints are painted onto
        // the last row's border glyphs once the tab is drawn. Deleted as tabs move to `Pane::hints`.
        // MOD-115 M2 §6.4: on a tab that claimed panes they go on the focused pane's border,
        // from its left edge to the body's right one (with the list focused, the M1 rect).
        if let Some(tab) = self.tabs.active()
            && self.overlays.is_empty()
            && !self.editor_visible()
            && chrome.body.height > 0
        {
            let target = claim.map_or(chrome.body, |(_, areas)| {
                let pane = focused_rect(&areas, self.focus_of(tab)).unwrap_or(chrome.body);
                Rect {
                    x: pane.x,
                    width: chrome.body.right().saturating_sub(pane.x),
                    height: pane.bottom().saturating_sub(chrome.body.y),
                    ..chrome.body
                }
            });
            let width = target.width.saturating_sub(4);
            let hint = tab.key_hint(width, &self.tab_ctx(tab));
            if let Some(line) = hint {
                paint_shell_hints(frame.buffer_mut(), target, line, self.theme.base);
            }
        }
        // R-TUI-12: the hidden pane's note, after the hints (it keeps one rule cell before them),
        // on the detail's bottom border, or the zoomed pane's.
        if let Some((_, areas)) = claim
            && !self.editor_visible()
        {
            self.paint_hidden_note(frame, &areas);
        }

        // MOD-115 M2 D6: the leave dialog names an open in-pane edit.
        let open_edit = self.editor.is_some().then(|| self.editor_title());
        for overlay in self.overlays.iter() {
            let ctx = self
                .ctx(Origin::Overlay(overlay.id()))
                .with_open_edit(open_edit);
            overlay.render(frame, area, &ctx);
        }

        if self.help_visible {
            self.render_help(frame, area);
        }
    }

    /// The `?` box (MOD-67 D8), rebuilt from the live state every frame. With a converted view
    /// holding the keyboard ([`App::active_stack`]): one line per layer of its stack, then the
    /// closing line its global layer admits. Otherwise: the overlay context (if one is
    /// up), then the global context, then the closing line.
    ///
    /// MOD-57 M2 E7 (PD-M2-2): with an in-pane editor alive the box lists the editor keys that
    /// act where the user is. On the asking tab, [`Stack::EDITOR_OPEN`]'s box (the editor's keys,
    /// then every global; none of the asking tab's own, which it refuses); on another tab,
    /// `editor.focus` first, then that view's box; under a modal overlay, the overlay's box with
    /// no editor key.
    ///
    /// Each logical line is packed into rows that fit the box ([`HelpLine::rows`]), so the box is
    /// exactly as tall as what it shows and no row is clipped.
    fn render_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let box_width = area.width.saturating_sub(10).max(20);
        let inner = usize::from(box_width.saturating_sub(2));
        let offered = |act| self.offers(act);
        // MOD-57 M2 E7 (PD-M2-2): the editor keys that act where the user is.
        let uncovered = self.editor.is_some() && !self.editor_covered();
        let locked = uncovered && self.editor_visible();
        let elsewhere = uncovered && !self.editor_visible();
        let mut lines: Vec<HelpLine> = Vec::new();
        if elsewhere {
            lines.extend(
                self.keys
                    .help_line(Context::Editor, |act| act == Act::EditorFocus),
            );
        }
        let stack = if locked {
            Some(Stack::EDITOR_OPEN)
        } else {
            self.active_stack()
        };
        let closer = match stack {
            Some(stack) => {
                lines.extend(self.keys.help_lines(stack, offered));
                self.keys.help_closer(stack)
            }
            None => {
                if !self.overlays.is_empty() {
                    lines.extend(self.keys.help_line(Context::Overlay, offered));
                }
                lines.extend(self.keys.help_line(Context::Global, offered));
                self.keys.help_closer(Stack::BASE)
            }
        };
        let mut rows: Vec<Line<'_>> = lines
            .iter()
            .flat_map(|line| line.rows(inner))
            .map(|row| Line::styled(row, self.theme.base))
            .collect();
        if let Some(closer) = closer {
            rows.push(Line::styled(closer, self.theme.dim));
        }
        let height = u16::try_from(rows.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(area.height);
        let box_area = layout::centered(area, box_width, height);
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(Text::from(rows))
                .block(Block::new().borders(Borders::ALL).title(" Keys ")),
            box_area,
        );
    }
}

/// The activity placeholder's title, what it says and its next step (MOD-115 M2 D2): the pane
/// MOD-122 fills.
const ACTIVITY_TITLE: &str = "Activity";
const ACTIVITY_WHAT: &str = "Activity appears here once the activity panel lands";
const ACTIVITY_NEXT: &str = "hides this pane";
const ACTIVITY_NEXT_UNBOUND: &str = "a layout without it hides this pane";

/// The offset from the grabbed cell `(x, y)` on `seam` to the line the layout's setter reads:
/// the list's right border, the activity pane's left border (columns) or top border (stacked).
/// Grabbing the other line of the seam is one cell off it (T2 carried finding 1).
fn seam_offset(areas: &PaneAreas, seam: Seam, x: u16, y: u16) -> (i32, i32) {
    let delta = |anchor: u16, at: u16| i32::from(anchor) - i32::from(at);
    match seam {
        Seam::List => areas
            .list
            .map_or((0, 0), |list| (delta(list.right().saturating_sub(1), x), 0)),
        Seam::Activity => match areas.activity {
            Some(activity) if activity.y == areas.detail.y || areas.detail.is_empty() => {
                (delta(activity.x, x), 0)
            }
            Some(activity) => (0, delta(activity.y, y)),
            None => (0, 0),
        },
    }
}

/// `at` moved by `by` cells, clamped to the terminal's range.
fn shifted(at: u16, by: i32) -> u16 {
    u16::try_from((i32::from(at) + by).max(0)).unwrap_or(u16::MAX)
}

/// The rect `pane` is drawn in, if it has a cell (a zoomed-away detail is 0x0).
fn focused_rect(areas: &PaneAreas, pane: PaneId) -> Option<Rect> {
    match pane {
        PaneId::List => areas.list,
        PaneId::Detail => Some(areas.detail),
        PaneId::Activity => areas.activity,
    }
    .filter(|rect| !rect.is_empty())
}

/// How many of the body's last rows [`hint_row`] looks through for the tab's bottom border.
const HINT_SEARCH_ROWS: u16 = 4;

/// The row of `body` the M1 hint adapter paints on: the lowest of its last [`HINT_SEARCH_ROWS`]
/// rows that starts with a bottom-left corner, i.e. the bottom border of the tab's content. A tab
/// that puts a notice or a filter row under its frame still gets the hints on the frame; one with
/// no such border in its last rows loses them until it moves to `Pane::hints` (M3, M4).
fn hint_row(buf: &ratatui::buffer::Buffer, body: Rect) -> Option<Rect> {
    let body = body.intersection(buf.area);
    if body.is_empty() {
        // A body with no columns (a 0-wide frame) has no cell to look at.
        return None;
    }
    (0..HINT_SEARCH_ROWS.min(body.height))
        .map(|up| body.bottom() - 1 - up)
        .find(|y| matches!(buf[(body.x, *y)].symbol(), "\u{2514}" | "\u{2570}"))
        .map(|y| Rect::new(body.x, y, body.width, 1))
}

/// The M1 hint adapter's whole job: finds the bottom border of the tab's content in `body`
/// ([`hint_row`]), joins the seams under the hint ([`join_panes_under`]) and paints the hint on
/// the rule ([`paint_bottom_hints`]), which ends it in an ellipsis where the tab drew text of its
/// own on that border. Deleted with the rest of the adapter as tabs move to `Pane::hints`
/// (M3, M4).
fn paint_shell_hints(
    buf: &mut ratatui::buffer::Buffer,
    body: Rect,
    hint: Line<'static>,
    style: ratatui::style::Style,
) {
    let Some(row) = hint_row(buf, body) else {
        return;
    };
    let line = clip_line(hint, body.width.saturating_sub(4));
    join_panes_under(buf, row, line.width());
    paint_bottom_hints(buf, row, line, style);
}

/// Turns the corners where two side-by-side panes meet on `row` into plain rule, across the
/// `width` cells the hints will cover from two cells in, so a hint longer than the first pane
/// reads on across the seam instead of being cut by `┘└`. The join stops at the first cell that
/// is text on the border (a title the tab drew there), where [`paint_bottom_hints`] ends the hint.
fn join_panes_under(buf: &mut ratatui::buffer::Buffer, row: Rect, width: usize) {
    let end = row
        .x
        .saturating_add(2)
        .saturating_add(u16::try_from(width).unwrap_or(u16::MAX))
        .min(row.right().saturating_sub(1));
    for x in row.x.saturating_add(2)..end {
        match buf[(x, row.y)].symbol() {
            "\u{2518}" | "\u{2514}" | "\u{256f}" | "\u{2570}" => {
                buf[(x, row.y)].set_symbol("\u{2500}");
            }
            "\u{2500}" | "\u{2501}" => {}
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::{App, hint_row, join_panes_under};
    use crate::keys::{CATALOGUE, Context};

    /// MOD-115 M2 B7: a view sees the list focused and no open edit unless the shell says so;
    /// the builders set both.
    #[test]
    fn a_ctx_defaults_to_the_list_focused_and_no_open_edit() {
        use htui_core::model::{Scope, WorkspaceId};

        use super::{Ctx, Emit, TopBarState};
        use crate::keys::Keys;
        use crate::store_worker::Origin;
        use crate::ui::Theme;
        use crate::ui::overlay::AbortConfirm;
        use crate::ui::shell::PaneId;

        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let (top_bar, theme, emit) = (TopBarState::default(), Theme::default(), Emit::default());
        let ctx = || {
            Ctx::new(
                &scope,
                &[],
                &top_bar,
                Keys::compiled(),
                &theme,
                Origin::Overlay(AbortConfirm::ID),
                &emit,
            )
        };
        assert_eq!(ctx().focus(), PaneId::List);
        assert_eq!(ctx().open_edit(), None);
        let set = ctx()
            .with_focus(PaneId::Detail)
            .with_open_edit(Some("MOD-115 body"));
        assert_eq!(set.focus(), PaneId::Detail);
        assert_eq!(set.open_edit(), Some("MOD-115 body"));
        assert_eq!(ctx().with_open_edit(None).open_edit(), None);
    }

    /// MOD-115 M2 B5, B7: `split` uses the tab's own default until the shell hands it a layout,
    /// and records the region and the areas only where the shell lent the claim cell.
    #[test]
    fn a_split_follows_the_default_until_the_shell_sets_a_layout_and_records_its_claim() {
        use std::cell::Cell;

        use htui_core::model::{Scope, WorkspaceId};

        use super::{Ctx, Emit, PaneClaim, TopBarState};
        use crate::keys::Keys;
        use crate::store_worker::Origin;
        use crate::ui::Theme;
        use crate::ui::shell::{ListSize, PaneId, Preset, TabLayout, layout};
        use crate::ui::tabs::TabId;

        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: Vec::new(),
        };
        let (top_bar, theme, emit) = (TopBarState::default(), Theme::default(), Emit::default());
        let ctx = || {
            Ctx::new(
                &scope,
                &[],
                &top_bar,
                Keys::compiled(),
                &theme,
                Origin::Tab(TabId("bench")),
                &emit,
            )
        };
        let region = Rect::new(0, 1, 100, 28);
        let default = TabLayout {
            list: ListSize::Percent(55),
            ..TabLayout::DEFAULT
        };
        let focus = TabLayout {
            preset: Preset::Focus,
            ..default
        };
        assert_eq!(
            ctx().split(region, default),
            layout::areas(region, &default, PaneId::List),
            "a bench splits by the tab's default"
        );
        let cell: Cell<Option<PaneClaim>> = Cell::new(None);
        let areas = ctx()
            .with_layout(focus)
            .with_focus(PaneId::Detail)
            .with_pane_claim(&cell)
            .split(region, default);
        assert_eq!(areas, layout::areas(region, &focus, PaneId::Detail));
        assert_eq!(areas.list, None, "the shell's layout wins over the default");
        assert_eq!(cell.get(), Some((region, areas)), "the claim is recorded");
        let _ = ctx().split(region, default);
        assert_eq!(
            cell.get(),
            Some((region, areas)),
            "a context without the cell records nothing"
        );
    }

    /// MOD-115 M2 §6.3: a tab that declares nothing has one pane, no location and restores
    /// nothing.
    #[test]
    fn a_tab_has_one_pane_no_location_and_restores_nothing_by_default() {
        use crate::ui::shell::{Panes, TabLoc};
        use crate::ui::tabs::Tab;

        struct Bare;
        impl Tab for Bare {
            fn id(&self) -> crate::ui::tabs::TabId {
                crate::ui::tabs::TabId("bare")
            }
            fn title(&self) -> &str {
                "Bare"
            }
            fn wants_requests(
                &self,
                _scope: &htui_core::model::Scope,
            ) -> Vec<crate::store_worker::StoreRequest> {
                Vec::new()
            }
            fn on_scope_change(&mut self, _scope: &htui_core::model::Scope) {}
            fn on_key(
                &mut self,
                _key: crossterm::event::KeyEvent,
                _ctx: &mut super::Ctx<'_>,
            ) -> crate::app::Handled {
                crate::app::Handled::Pass
            }
            fn on_reply(
                &mut self,
                _reply: &crate::store_worker::StoreReply,
                _ctx: &mut super::Ctx<'_>,
            ) {
            }
            fn render(&self, _frame: &mut ratatui::Frame<'_>, _area: Rect, _ctx: &super::Ctx<'_>) {}
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let app = App::new(tx);
        let mut ctx = app.ctx(crate::store_worker::Origin::App);
        let mut bare = Bare;
        assert_eq!(bare.panes(), Panes::Single);
        assert_eq!(bare.location(), TabLoc::default());
        assert!(!bare.restore(&TabLoc::default(), &mut ctx));
    }

    /// MOD-115 M1 adapter: the hints go on the lowest bottom border among the body's last four
    /// rows, so a tab with a notice under its frame still shows them; none there, none shown.
    #[test]
    fn the_hint_row_is_the_lowest_bottom_border_in_the_last_rows() {
        let body = Rect::new(0, 1, 20, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 11));
        assert_eq!(hint_row(&buf, body), None);
        buf[(0, 5)].set_symbol("\u{2514}");
        assert_eq!(hint_row(&buf, body), None, "five rows up is too far");
        buf[(0, 8)].set_symbol("\u{2514}");
        assert_eq!(hint_row(&buf, body), Some(Rect::new(0, 8, 20, 1)));
        buf[(0, 10)].set_symbol("\u{2570}");
        assert_eq!(hint_row(&buf, body), Some(Rect::new(0, 10, 20, 1)));
    }

    /// A frame with no columns has no border to paint the hints on: the adapter finds no row
    /// instead of reading a cell the buffer does not have (T6 repair 2, finding 5).
    #[test]
    fn a_body_with_no_columns_has_no_hint_row() {
        let buf = Buffer::empty(Rect::new(0, 0, 0, 6));
        for height in 0..=6 {
            assert_eq!(hint_row(&buf, Rect::new(0, 0, 0, height)), None, "{height}");
        }
    }

    /// A hint longer than the first pane reads on across the seam where two panes meet.
    #[test]
    fn a_long_hint_joins_the_panes_it_runs_across() {
        let row = Rect::new(0, 0, 14, 1);
        let mut buf = Buffer::empty(row);
        buf.set_string(0, 0, "\u{2514}\u{2500}\u{2500}\u{2500}\u{2518}\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}", ratatui::style::Style::default());
        join_panes_under(&mut buf, row, 2);
        assert_eq!(
            buf[(4, 0)].symbol(),
            "\u{2518}",
            "the hint stops before the seam"
        );
        join_panes_under(&mut buf, row, 8);
        assert_eq!(buf[(4, 0)].symbol(), "\u{2500}");
        assert_eq!(buf[(5, 0)].symbol(), "\u{2500}");
        assert_eq!(buf[(0, 0)].symbol(), "\u{2514}", "the outer corners stay");
        assert_eq!(buf[(13, 0)].symbol(), "\u{2518}");
    }

    /// Two rounded panes side by side over `width` x 8 cells, the right one carrying a scroll hint
    /// on its bottom border as the Skills library and templates views do.
    fn two_panes(width: u16) -> Buffer {
        use ratatui::text::Line;
        use ratatui::widgets::{Block, BorderType, Widget};

        let area = Rect::new(0, 0, width, 8);
        let mut buf = Buffer::empty(area);
        let left = Rect::new(0, 0, width / 3, 8);
        let right = Rect::new(left.right(), 0, width - left.width, 8);
        Block::bordered()
            .border_type(BorderType::Rounded)
            .render(left, &mut buf);
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title_bottom(Line::from(" 12/40 ").right_aligned())
            .render(right, &mut buf);
        buf
    }

    fn bottom_row(buf: &Buffer) -> String {
        let y = buf.area.bottom() - 1;
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// MOD-115 review (hint adapter): where a tab drew its own text on the bottom border, the
    /// shell hint ends with an ellipsis just before it instead of skipping cells one by one; the
    /// seam between the panes is still joined under the part of the hint that is drawn.
    #[test]
    fn a_hint_running_into_a_title_on_the_bottom_border_ends_in_an_ellipsis_before_it() {
        use ratatui::style::Style;
        use ratatui::text::Line;

        use super::paint_shell_hints;

        let hint = "j/k move \u{b7} enter open \u{b7} f filter \u{b7} N new \u{b7} e edit \u{b7} / search \u{b7} ? keys \u{b7} q quit \u{b7} Tab next tab \u{b7} Ctrl+f find";
        for width in [100_u16, 80] {
            let mut buf = two_panes(width);
            let body = Rect::new(0, 0, width, 8);
            paint_shell_hints(&mut buf, body, Line::from(hint), Style::new());
            let row = bottom_row(&buf);
            let cells: Vec<char> = row.chars().collect();
            assert_eq!(cells.len(), usize::from(width), "{width}: {row}");
            assert!(
                row.starts_with("\u{2570}\u{2500}j/k move"),
                "{width}: {row}"
            );
            assert!(
                row.ends_with("\u{2026} 12/40 \u{256f}"),
                "{width}: the hint ends in an ellipsis right before the title: {row}"
            );
            let inner = &cells[1..cells.len() - 1];
            assert!(
                !inner
                    .iter()
                    .any(|c| "\u{2514}\u{2518}\u{2570}\u{256f}".contains(*c)),
                "{width}: the seam is joined under the hint: {row}"
            );
        }
        // A hint that fits before the title is drawn whole, with no ellipsis.
        let mut buf = two_panes(100);
        paint_shell_hints(
            &mut buf,
            Rect::new(0, 0, 100, 8),
            Line::from("j/k move"),
            Style::new(),
        );
        let row = bottom_row(&buf);
        assert!(row.starts_with("\u{2570}\u{2500}j/k move"), "{row}");
        assert!(!row.contains('\u{2026}'), "{row}");
        assert!(row.ends_with(" 12/40 \u{256f}"), "{row}");
    }

    /// The seam is not joined past text on the border: a corner after the title stays.
    #[test]
    fn the_seam_join_stops_at_text_on_the_border() {
        let row = Rect::new(0, 0, 14, 1);
        let mut buf = Buffer::empty(row);
        buf.set_string(
            0,
            0,
            "\u{2570}\u{2500}\u{2500}ab\u{2500}\u{256f}\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}",
            ratatui::style::Style::default(),
        );
        join_panes_under(&mut buf, row, 12);
        assert_eq!(buf[(6, 0)].symbol(), "\u{256f}", "text ends the join");
        assert_eq!(buf[(7, 0)].symbol(), "\u{2570}");
    }

    /// MOD-67 review L1: `action_for`'s wildcard arm is only safe while every global act is
    /// either fixed-mapped by the shell or offerable (and so dispatched once `register_all`
    /// offers it). A new global act that is neither would be silently inert.
    #[test]
    fn every_global_act_is_fixed_mapped_or_offerable() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let app = App::new(tx);
        for row in CATALOGUE
            .iter()
            .filter(|row| row.context == Context::Global)
        {
            let fixed = app.action_for(row.act).is_some();
            assert!(
                fixed != App::is_offerable(row.act),
                "{:?}: fixed-mapped {fixed}, offerable {}",
                row.act,
                App::is_offerable(row.act)
            );
        }
    }
}

/// MOD-115 M2 T6: the shell's panes (render), focus, layout, history and mouse, over a list +
/// detail test double.
#[cfg(test)]
pub(crate) mod shell_tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::model::Scope;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use ratatui::widgets::{Block, BorderType};
    use ratatui::{Frame, Terminal};
    use tokio::sync::mpsc;

    use super::{App, Ctx};
    use crate::app::Handled;
    use crate::keys::{Stack, views};
    use crate::store_worker::{StoreReply, StoreRequest};
    use crate::ui::Theme;
    use crate::ui::shell::{ListSize, PaneId, Panes, Preset, TabLayout, TabLoc};
    use crate::ui::tabs::{Tab, TabId};

    /// The double's id: one the theme gives an accent of its own.
    pub(crate) const SPLIT: TabId = TabId("backlog");

    /// The double's default layout: Backlog's 55/45.
    pub(crate) const SPLIT_LAYOUT: TabLayout = TabLayout {
        list: ListSize::Percent(55),
        ..TabLayout::DEFAULT
    };

    /// A list + detail tab that splits its body with `ctx.split` and frames each pane, as the
    /// list + detail tabs do once they adopt the layout (Task 7).
    pub(crate) struct SplitTab {
        pub(crate) id: TabId,
        pub(crate) title: &'static str,
        pub(crate) subtab: Option<&'static str>,
        pub(crate) cursor: usize,
        pub(crate) restored: Vec<TabLoc>,
        /// The stack it resolves through: Backlog's body sub-tab unless a test names another.
        pub(crate) stack: Stack<'static>,
    }

    impl SplitTab {
        pub(crate) fn new(id: TabId, title: &'static str) -> Self {
            Self {
                id,
                title,
                subtab: None,
                cursor: 0,
                restored: Vec::new(),
                stack: views::BACKLOG_BODY,
            }
        }
    }

    impl Tab for SplitTab {
        fn id(&self) -> TabId {
            self.id
        }
        fn title(&self) -> &str {
            self.title
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            if !key.modifiers.is_empty() {
                return Handled::Pass;
            }
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.cursor += 1;
                    Handled::Consumed
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.cursor = self.cursor.saturating_sub(1);
                    Handled::Consumed
                }
                KeyCode::Char('l') => {
                    self.subtab = Some(match self.subtab {
                        Some("runs") => "body",
                        _ => "runs",
                    });
                    Handled::Consumed
                }
                _ => Handled::Pass,
            }
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
            let areas = ctx.split(area, SPLIT_LAYOUT);
            let block = || Block::bordered().border_type(BorderType::Rounded);
            if let Some(list) = areas.list {
                frame.render_widget(block(), list);
            }
            if !areas.detail.is_empty() {
                frame.render_widget(block(), areas.detail);
            }
        }
        fn key_stack(&self) -> Option<Stack<'static>> {
            Some(self.stack)
        }
        fn key_hint(&self, _width: u16, _ctx: &Ctx<'_>) -> Option<Line<'static>> {
            Some(Line::from("j/k move"))
        }
        fn panes(&self) -> Panes {
            Panes::ListDetail(SPLIT_LAYOUT)
        }
        fn location(&self) -> TabLoc {
            TabLoc {
                subtab: self.subtab,
                subtab_title: self.subtab.map(|id| {
                    if id == "runs" {
                        "Runs".to_owned()
                    } else {
                        "Body".to_owned()
                    }
                }),
                cursor: Some(self.cursor.to_string()),
            }
        }
        fn restore(&mut self, loc: &TabLoc, _ctx: &mut Ctx<'_>) -> bool {
            self.restored.push(loc.clone());
            self.subtab = loc.subtab;
            self.cursor = loc
                .cursor
                .as_deref()
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);
            true
        }
    }

    /// A shell with the double registered, about off.
    pub(crate) fn split_app() -> App {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.about_visible = false;
        app.register_tab(Box::new(SplitTab::new(SPLIT, "Backlog")));
        app
    }

    pub(crate) fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        term.draw(|frame| app.render(frame)).expect("draws");
        term.backend().buffer().clone()
    }

    pub(crate) fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    pub(crate) fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.on_key(KeyEvent::new(code, mods));
    }

    fn claim(app: &App) -> (Rect, crate::ui::shell::PaneAreas) {
        let (tab, region, areas) = app.last_claim.expect("the double claimed its panes");
        assert_eq!(tab, SPLIT);
        (region, areas)
    }

    /// D3: the focused pane's border takes the tab accent, the other the faint border role;
    /// with no focus set the list is focused (B7).
    #[test]
    fn the_focused_pane_has_the_tab_accent_and_the_other_the_faint_border() {
        for theme in [Theme::default(), Theme::monochrome()] {
            let mut app = split_app();
            app.theme = theme;
            let buf = draw(&mut app, 100, 30);
            let (_, areas) = claim(&app);
            let list = areas.list.expect("the list is shown");
            let accent = theme.tab_accent(SPLIT.0);
            assert_eq!(buf[(list.x, list.y)].fg, accent.fg.unwrap_or_default());
            assert_eq!(buf[(list.x, list.y)].modifier, accent.add_modifier);
            let detail = areas.detail;
            assert_eq!(
                buf[(detail.x, detail.y)].fg,
                theme.border_faint.fg.unwrap_or_default()
            );
            assert_eq!(
                buf[(detail.x, detail.y)].modifier,
                theme.border_faint.add_modifier
            );
            app.focus.insert(SPLIT, PaneId::Detail);
            let buf = draw(&mut app, 100, 30);
            assert_eq!(buf[(detail.x, detail.y)].fg, accent.fg.unwrap_or_default());
            assert_eq!(buf[(detail.x, detail.y)].modifier, accent.add_modifier);
            assert_eq!(
                buf[(list.x, list.y)].modifier,
                theme.border_faint.add_modifier
            );
        }
    }

    /// §6.4: the hints go on the focused pane's bottom border; with the list focused that is
    /// the body's row from its left edge, as the M1 adapter drew them.
    #[test]
    fn the_hints_follow_the_focused_pane() {
        let mut app = split_app();
        let buf = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let bottom = areas.detail.bottom() - 1;
        assert!(
            row(&buf, bottom).starts_with("\u{2570}\u{2500}j/k move"),
            "{}",
            row(&buf, bottom)
        );
        app.focus.insert(SPLIT, PaneId::Detail);
        let buf = draw(&mut app, 100, 30);
        let text = row(&buf, bottom);
        let at = usize::from(areas.detail.x);
        let cells: Vec<&str> = (0..buf.area.width)
            .map(|x| buf[(x, bottom)].symbol())
            .collect();
        assert_eq!(cells[at + 2..at + 10].concat(), "j/k move", "{text}");
        assert!(!cells[..at].concat().contains("j/k"), "{text}");
    }

    /// R-TUI-12, carried finding 2: a hidden list leaves its note on the detail's bottom border,
    /// right-aligned, painted after the hints with a rule cell between them.
    #[test]
    fn a_hidden_list_says_how_to_bring_it_back_beside_the_hints() {
        let mut app = split_app();
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                list_on: false,
                ..SPLIT_LAYOUT
            },
        );
        app.focus.insert(SPLIT, PaneId::Detail);
        let buf = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        assert_eq!(areas.list, None);
        let text = row(&buf, areas.detail.bottom() - 1);
        assert!(text.starts_with("\u{2570}\u{2500}j/k move"), "{text}");
        assert!(
            text.ends_with("F2 show list \u{b7} p layouts\u{2500}\u{256f}"),
            "{text}"
        );
        // Where the note's run is cut short by the hints, it is clipped with an ellipsis and
        // exactly one rule cell parts the last hint glyph from it: the hints are painted first,
        // whole, and the note after them.
        let buf = draw(&mut app, 30, 12);
        let (_, areas) = claim(&app);
        let bottom = areas.detail.bottom() - 1;
        let text = row(&buf, bottom);
        let at = usize::from(areas.detail.x);
        let cells: Vec<&str> = (0..buf.area.width)
            .map(|x| buf[(x, bottom)].symbol())
            .collect();
        assert_eq!(cells[at + 2..at + 10].concat(), "j/k move", "{text}");
        assert_eq!(cells[at + 10], "\u{2500}", "one rule cell: {text}");
        assert_eq!(cells[at + 11], "F", "the note starts after it: {text}");
        let right = at + usize::from(areas.detail.width);
        assert_eq!(
            cells[right - 3..right].concat(),
            "\u{2026}\u{2500}\u{256f}",
            "the note is clipped: {text}"
        );
    }

    /// R-TUI-12 with D-1: the note names only keys that reach their act. In Backlog > Runs `p`
    /// is `runs.promote`, so the note does not offer it for the layouts.
    #[test]
    fn a_hidden_note_leaves_out_a_key_a_narrower_act_takes() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.about_visible = false;
        let mut runs = SplitTab::new(SPLIT, "Backlog");
        runs.stack = views::RUNS_BROWSE;
        app.register_tab(Box::new(runs));
        let p = crate::keys::KeyChord::new(KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(
            app.keys.actions(views::RUNS_BROWSE, p).first(),
            Some(&crate::keys::Act::RunsPromote)
        );
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                list_on: false,
                ..SPLIT_LAYOUT
            },
        );
        app.focus.insert(SPLIT, PaneId::Detail);
        let buf = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let text = row(&buf, areas.detail.bottom() - 1);
        assert!(text.ends_with("F2 show list\u{2500}\u{256f}"), "{text}");
        assert!(!text.contains("layouts"), "{text}");
    }

    /// Carried finding 3: a zoomed list leaves the detail 0x0; the note goes on the zoomed pane.
    #[test]
    fn a_zoomed_list_carries_the_unzoom_note() {
        let mut app = split_app();
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                zoom: true,
                ..SPLIT_LAYOUT
            },
        );
        let buf = draw(&mut app, 100, 30);
        let (region, areas) = claim(&app);
        assert_eq!(areas.list, Some(region));
        assert!(areas.detail.is_empty());
        let text = row(&buf, region.bottom() - 1);
        assert!(text.ends_with("z unzoom\u{2500}\u{256f}"), "{text}");
    }

    /// D2: until MOD-122 the activity pane is a placeholder that says what will be there.
    #[test]
    fn the_activity_pane_is_a_placeholder() {
        let mut app = split_app();
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                preset: Preset::IdeActivity,
                ..SPLIT_LAYOUT
            },
        );
        let buf = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let activity = areas.activity.expect("the preset shows the activity pane");
        assert!(row(&buf, activity.y).contains(" Activity "));
        let inside: String = (activity.y..activity.bottom())
            .map(|y| row(&buf, y))
            .collect();
        assert!(inside.contains("Activity appears here"), "{inside}");
    }

    fn focused(app: &App) -> PaneId {
        app.focus_of(app.tabs.active().expect("a tab"))
    }

    fn layout(app: &App) -> TabLayout {
        app.layout_of(app.tabs.active().expect("a tab"))
            .expect("a list + detail tab")
    }

    /// D3: the pane keys cycle the panes the last frame showed, wrapping; on a single-pane tab
    /// they do nothing.
    #[test]
    fn the_pane_keys_cycle_the_visible_panes_and_do_nothing_on_a_single_pane_tab() {
        let mut app = split_app();
        let _ = draw(&mut app, 100, 30);
        assert_eq!(focused(&app), PaneId::List);
        press(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(focused(&app), PaneId::Detail);
        press(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(focused(&app), PaneId::List, "it wraps");
        press(&mut app, KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(focused(&app), PaneId::Detail);

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut chat = App::new(tx);
        chat.register_tab(Box::new(crate::ui::tabs::ChatTab::new()));
        let _ = draw(&mut chat, 100, 30);
        press(&mut chat, KeyCode::Right, KeyModifiers::CONTROL);
        assert!(chat.focus.is_empty(), "{:?}", chat.focus);
        assert_eq!(focused(&chat), PaneId::Detail);
    }

    /// D4, the headline flip: `Tab`/`BackTab` move the pane focus on a list + detail tab, and
    /// on Settings (a single-pane tab) they do nothing: no focus, and no tab change either.
    #[test]
    fn tab_moves_the_panes_and_does_nothing_on_settings() {
        let mut app = split_app();
        let _ = draw(&mut app, 100, 30);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(focused(&app), PaneId::Detail);
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(focused(&app), PaneId::List, "it wraps");
        press(&mut app, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(focused(&app), PaneId::Detail);
        assert_eq!(app.tabs.active_id(), Some(SPLIT), "no tab change");

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut settings = App::new(tx);
        settings.register_tab(Box::new(crate::ui::tabs::SettingsTab::with_sections(vec![
            Box::new(crate::ui::tabs::settings::QdrantSection::new()),
        ])));
        settings.register_tab(Box::new(SplitTab::new(SPLIT, "Backlog")));
        let _ = draw(&mut settings, 100, 30);
        let before = settings.tabs.active_id();
        press(&mut settings, KeyCode::Tab, KeyModifiers::NONE);
        press(&mut settings, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(settings.tabs.active_id(), before, "Tab moves no tab");
        assert!(settings.focus.is_empty(), "{:?}", settings.focus);
    }

    /// D5 with D3: back to a place whose pane the layout has since hidden focuses the detail,
    /// not the hidden pane.
    #[test]
    fn back_never_focuses_a_pane_the_layout_hides() {
        let mut app = split_app();
        let _ = draw(&mut app, 100, 30);
        assert_eq!(focused(&app), PaneId::List);
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE);
        assert_eq!(focused(&app), PaneId::Detail);
        assert!(
            app.history.peek_back().is_some(),
            "the focus move is a place"
        );
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        let _ = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        assert_eq!(areas.list, None, "the list stays hidden");
        assert_eq!(focused(&app), PaneId::Detail);
    }

    /// D2, D1: the layout keys change the active tab's layout and save it the moment it changes;
    /// a pane the change hides gives the focus to the detail.
    #[test]
    fn the_layout_keys_change_the_layout_save_it_and_move_focus_off_a_hidden_pane() {
        use crate::ui::state_file::UiStore;

        let store = UiStore::memory();
        let mut app = split_app();
        app.ui_store = store.clone();
        let _ = draw(&mut app, 100, 30);
        press(&mut app, KeyCode::Char('>'), KeyModifiers::NONE);
        assert_eq!(layout(&app).list, ListSize::Percent(60));
        assert_eq!(
            store.load().state.layouts.get(SPLIT.0).map(|l| l.list),
            Some(ListSize::Percent(60)),
            "saved at once"
        );
        press(&mut app, KeyCode::Char('<'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('<'), KeyModifiers::NONE);
        assert_eq!(layout(&app).list, ListSize::Percent(50));
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(layout(&app).zoom);
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(!layout(&app).zoom);
        press(&mut app, KeyCode::Char('p'), KeyModifiers::NONE);
        assert_eq!(layout(&app).preset, Preset::IdeActivity);
        press(&mut app, KeyCode::Down, KeyModifiers::CONTROL);
        assert_eq!(
            layout(&app).activity_pct,
            35,
            "ctrl-down grows the activity pane"
        );
        press(&mut app, KeyCode::Up, KeyModifiers::CONTROL);
        assert_eq!(layout(&app).activity_pct, 30);
        press(&mut app, KeyCode::Char('2'), KeyModifiers::ALT);
        assert!(!layout(&app).activity_on, "alt-2 hides the activity pane");
        assert_eq!(focused(&app), PaneId::List);
        press(&mut app, KeyCode::F(2), KeyModifiers::NONE);
        assert!(!layout(&app).list_on);
        assert_eq!(
            focused(&app),
            PaneId::Detail,
            "the hidden list loses the focus"
        );
        assert_eq!(store.load().state.layouts.get(SPLIT.0), Some(&layout(&app)));
    }

    /// D1, the plan's acceptance: the layout and the about panel survive a restart: change both,
    /// drop the shell, build another over the same store, and both are as they were left.
    #[test]
    fn the_layout_and_the_about_panel_survive_a_restart() {
        use crate::ui::state_file::UiStore;

        let store = UiStore::memory();
        let mut app = split_app();
        app.apply_ui_state(store.load().state);
        app.ui_store = store.clone();
        let _ = draw(&mut app, 100, 30);
        press(&mut app, KeyCode::Char('>'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(!app.about_visible);
        press(&mut app, KeyCode::F(9), KeyModifiers::NONE);
        assert!(app.about_visible);
        press(&mut app, KeyCode::F(9), KeyModifiers::NONE);
        let left = layout(&app);
        drop(app);

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut again = App::new(tx);
        assert!(again.about_visible, "a first launch shows the about panel");
        let loaded = store.load();
        assert_eq!(loaded.notice, None);
        again.apply_ui_state(loaded.state);
        again.ui_store = store;
        again.register_tab(Box::new(SplitTab::new(SPLIT, "Backlog")));
        assert_eq!(layout(&again), left);
        assert_eq!(left.list, ListSize::Percent(60));
        assert!(left.zoom);
        assert!(!again.about_visible, "the about panel stays off");
    }

    /// D5: a tab change is a new place, a cursor move is not; back returns to the place left,
    /// cursor included, and forward to where back was pressed.
    #[test]
    fn back_and_forward_return_to_the_places_left_cursor_included() {
        let mut app = split_app();
        app.register_tab(Box::new(SplitTab::new(
            TabId("requirements"),
            "Requirements",
        )));
        let _ = draw(&mut app, 100, 30);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(
            app.history.peek_back().is_none(),
            "a cursor move is not recorded"
        );
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(app.tabs.active_id(), Some(TabId("requirements")));
        let back = app.history.peek_back().expect("the tab change is recorded");
        assert_eq!(back.label, "Backlog \u{b7} list");
        assert_eq!(back.at.cursor.as_deref(), Some("2"));
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
        assert_eq!(
            app.history.peek_back().map(|loc| loc.label.as_str()),
            Some("Requirements \u{b7} list"),
            "a sub-tab change is a new place"
        );
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.tabs.active_id(), Some(TabId("requirements")));
        assert_eq!(app.here().map(|loc| loc.at.subtab), Some(None));
        assert!(app.history.can_forward());
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.tabs.active_id(), Some(SPLIT));
        assert_eq!(
            app.here().and_then(|loc| loc.at.cursor),
            Some("2".to_owned()),
            "the cursor comes back"
        );
        assert!(app.history.peek_back().is_none());
        press(&mut app, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(app.tabs.active_id(), Some(TabId("requirements")));
        press(&mut app, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(app.here().and_then(|loc| loc.at.subtab), Some("runs"));
        assert!(!app.history.can_forward());
        // A pane move is a new place too, and a new place drops the forward stack.
        press(&mut app, KeyCode::Left, KeyModifiers::ALT);
        assert!(app.history.can_forward());
        let _ = draw(&mut app, 100, 30);
        press(&mut app, KeyCode::Right, KeyModifiers::CONTROL);
        assert!(!app.history.can_forward());
        assert_eq!(
            app.history.peek_back().map(|loc| loc.label.as_str()),
            Some("Requirements \u{b7} list")
        );
    }

    /// Registers the leave dialog, as `register_all` does.
    pub(crate) fn leavable(app: &mut App) {
        use crate::ui::overlay::LeaveDialog;
        app.overlay_factories
            .register(LeaveDialog::ID, || Box::new(LeaveDialog::new()));
        app.leave_overlay = Some(LeaveDialog::ID);
    }

    /// A tab whose mode captures text and passes every key, as a form whose field passes a
    /// chord does.
    struct Capturing;

    impl Tab for Capturing {
        fn id(&self) -> TabId {
            TabId("capturing")
        }
        fn title(&self) -> &str {
            "Capturing"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn key_stack(&self) -> Option<Stack<'static>> {
            Some(views::CAPTURE)
        }
    }

    /// MOD-115 M2 D6, D7, `R-TUI-18`: `ctrl-c` is `global.quit` on every screen: from a tab, from
    /// a text field and over every modal overlay it opens the leave dialog, and a second one
    /// leaves. `q` opens it too; under it a second `q` opens no second dialog.
    #[test]
    fn ctrl_c_opens_the_leave_dialog_from_every_screen_and_a_second_leaves() {
        use crate::ui::overlay::{
            AbortConfirm, ConceptsSearch, LeaveDialog, MigrationPrompt, Overlay, QueueOverlay,
            WaitingList, WorkspaceSwitcher,
        };

        type MakeOverlay = fn() -> Box<dyn Overlay>;
        let overlays: Vec<Option<MakeOverlay>> = vec![
            None,
            Some(|| Box::new(WorkspaceSwitcher::new())),
            Some(|| Box::new(ConceptsSearch::new())),
            Some(|| Box::new(WaitingList::new())),
            Some(|| Box::new(QueueOverlay::new())),
            Some(|| Box::new(MigrationPrompt::new())),
            Some(|| Box::new(AbortConfirm::new())),
        ];
        for capturing in [false, true] {
            for overlay in &overlays {
                let mut app = split_app();
                if capturing {
                    let (tx, _rx) = mpsc::unbounded_channel();
                    app = App::new(tx);
                    app.register_tab(Box::new(Capturing));
                }
                leavable(&mut app);
                if let Some(make) = overlay {
                    app.push_overlay(make());
                }
                let at = app.overlays.top().map(Overlay::id);
                press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
                assert!(!app.should_quit, "{capturing} {at:?}: asked first");
                assert_eq!(
                    app.overlays.top().map(Overlay::id),
                    Some(LeaveDialog::ID),
                    "{capturing} {at:?}"
                );
                press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
                assert!(app.should_quit, "{capturing} {at:?}: the second leaves");
            }
        }
        let mut app = split_app();
        leavable(&mut app);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.overlays.top().map(Overlay::id), Some(LeaveDialog::ID));
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.overlays.iter().count(), 1, "no second dialog");
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.overlays.is_empty() && !app.should_quit, "Esc stays");
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.should_quit, "Enter leaves");
    }

    /// D6, D7: from the open chat composer `ctrl-c` opens the leave dialog (the composer passes
    /// it, it is no text), and a second one leaves.
    #[test]
    fn ctrl_c_from_the_chat_composer_asks_then_leaves() {
        use crate::ui::overlay::{LeaveDialog, Overlay};

        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.register_tab(Box::new(crate::ui::tabs::ChatTab::new()));
        leavable(&mut app);
        press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
        let stack = app.tabs.active().and_then(|tab| tab.key_stack());
        assert!(
            stack.is_some_and(Stack::captures_text),
            "the composer is open"
        );
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.should_quit, "asked first");
        assert_eq!(app.overlays.top().map(Overlay::id), Some(LeaveDialog::ID));
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.should_quit, "the second leaves");
    }

    /// D6: the dialog names the open in-pane edit; without one it reads as nothing live once the
    /// counts are known.
    #[test]
    fn the_leave_dialog_says_nothing_is_running_when_nothing_is() {
        let mut app = split_app();
        leavable(&mut app);
        app.top_bar.waiting = Some(htui_worker::WaitingView::default());
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        let buf = draw(&mut app, 100, 30);
        let text: String = (0..30).map(|y| row(&buf, y)).collect::<Vec<_>>().join("\n");
        assert!(text.contains("Nothing is running. Leave htui?"), "{text}");
        assert!(
            text.contains("Enter  leave") && text.contains("Esc  stay"),
            "{text}"
        );
    }

    /// §8: the header names the place back, the way forward and the layout, with the keys in
    /// force.
    #[test]
    fn the_header_names_the_way_back_the_way_forward_and_the_layout() {
        let mut app = split_app();
        app.register_tab(Box::new(SplitTab::new(
            TabId("requirements"),
            "Requirements",
        )));
        let header = |app: &mut App| row(&draw(app, 220, 30), 0);
        let text = header(&mut app);
        assert!(text.contains("list+detail"), "{text}");
        assert!(!text.contains("back"), "{text}");
        press(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        assert!(header(&mut app).contains("list+detail \u{b7} zoom"));
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        let text = header(&mut app);
        assert!(
            text.contains("\u{232b} back to Backlog \u{b7} list"),
            "{text}"
        );
        assert!(!text.contains("forward"), "{text}");
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        let text = header(&mut app);
        assert!(text.contains("Alt\u{2192} forward"), "{text}");
        // A rebound key labels the chip; an unbound one drops it.
        app.keys = crate::keys::load_str("version = 2\n[history]\nforward = [\"f7\"]\n")
            .expect("the file loads");
        assert!(header(&mut app).contains("F7 forward"));
        app.keys = crate::keys::load_str("version = 2\n[history]\nforward = []\n")
            .expect("the file loads");
        assert!(!header(&mut app).contains("forward"));
    }

    fn mouse(app: &mut App, kind: crossterm::event::MouseEventKind, column: u16, row: u16) {
        app.on_mouse(crossterm::event::MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn down(app: &mut App, x: u16, y: u16) {
        use crossterm::event::{MouseButton, MouseEventKind};
        mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
    }

    fn drag_to(app: &mut App, x: u16, y: u16) {
        use crossterm::event::{MouseButton, MouseEventKind};
        mouse(app, MouseEventKind::Drag(MouseButton::Left), x, y);
    }

    fn up(app: &mut App, x: u16, y: u16) {
        use crossterm::event::{MouseButton, MouseEventKind};
        mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
    }

    /// MD8: a list + detail tab turns mouse capture on; a single-pane one (Chat) keeps the
    /// terminal's own selection.
    #[test]
    fn capture_is_on_for_a_list_and_detail_tab_only() {
        let mut app = split_app();
        assert!(app.wants_mouse());
        assert!(app.mouse_capture());
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut chat = App::new(tx);
        chat.register_tab(Box::new(crate::ui::tabs::ChatTab::new()));
        assert!(!chat.wants_mouse());
        assert!(!chat.mouse_capture());
    }

    /// §7: a click in a pane focuses it.
    #[test]
    fn a_click_focuses_the_pane_under_it() {
        let mut app = split_app();
        let _ = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        down(&mut app, areas.detail.x + 5, areas.detail.y + 3);
        assert_eq!(focused(&app), PaneId::Detail);
        up(&mut app, areas.detail.x + 5, areas.detail.y + 3);
        let list = areas.list.expect("shown");
        down(&mut app, list.x + 3, list.y + 3);
        assert_eq!(focused(&app), PaneId::List);
    }

    /// §7, T2 carried finding 1: dragging the list seam moves it with the pointer, from either of
    /// its two lines, without a one-cell jump at the grab; the release saves the layout.
    #[test]
    fn a_seam_drag_follows_the_pointer_from_either_line_and_saves_on_release() {
        use crate::ui::state_file::UiStore;

        for far_line in [false, true] {
            let store = UiStore::memory();
            let mut app = split_app();
            app.ui_store = store.clone();
            let _ = draw(&mut app, 100, 30);
            let (region, areas) = claim(&app);
            let list = areas.list.expect("shown");
            let grab = if far_line {
                areas.detail.x
            } else {
                list.right() - 1
            };
            let y = list.y + 4;
            down(&mut app, grab, y);
            drag_to(&mut app, grab, y);
            let _ = draw(&mut app, 100, 30);
            let (_, still) = claim(&app);
            assert_eq!(still.list, areas.list, "{far_line}: the grab moves nothing");
            drag_to(&mut app, grab + 10, y);
            let _ = draw(&mut app, 100, 30);
            let (_, moved) = claim(&app);
            assert_eq!(
                moved.list.map(|list| list.width),
                Some(list.width + 10),
                "{far_line}: the seam follows the pointer"
            );
            assert_eq!(
                store.load().state.layouts.get(SPLIT.0),
                None,
                "not saved yet"
            );
            up(&mut app, grab + 10, y);
            let saved = store.load().state.layouts.get(SPLIT.0).copied();
            assert_eq!(saved, Some(layout(&app)), "{far_line}: saved on release");
            assert_eq!(focused(&app), PaneId::List, "a seam grab focuses nothing");
            assert_eq!(region.width, 100);
        }
    }

    /// §7, T2 carried finding 1: the activity seam, stacked, grabbed on the line above it.
    #[test]
    fn an_activity_seam_grabbed_on_either_line_does_not_jump() {
        let mut app = split_app();
        app.ui.layouts.insert(
            SPLIT.0.to_owned(),
            TabLayout {
                preset: Preset::IdeActivity,
                ..SPLIT_LAYOUT
            },
        );
        let _ = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let activity = areas.activity.expect("shown");
        for grab in [activity.y, activity.y - 1] {
            down(&mut app, activity.x + 30, grab);
            drag_to(&mut app, activity.x + 30, grab);
            let _ = draw(&mut app, 100, 30);
            let (_, after) = claim(&app);
            assert_eq!(
                after.activity,
                Some(activity),
                "{grab}: the grab moves nothing"
            );
            up(&mut app, activity.x + 30, grab);
        }
        down(&mut app, activity.x + 30, activity.y);
        drag_to(&mut app, activity.x + 30, activity.y - 3);
        up(&mut app, activity.x + 30, activity.y - 3);
        assert!(layout(&app).activity_pct > 30, "{:?}", layout(&app));
    }

    /// D1, §7: a drag that capture going off cuts short (an overlay opens, so its `Up` never
    /// reaches the shell) still saves the layout it left on screen.
    #[test]
    fn a_drag_cut_short_by_an_overlay_is_saved() {
        use crate::ui::state_file::UiStore;

        let store = UiStore::memory();
        let mut app = split_app();
        leavable(&mut app);
        app.ui_store = store.clone();
        assert!(app.mouse_capture());
        let _ = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let list = areas.list.expect("shown");
        let (grab, y) = (list.right() - 1, list.y + 4);
        down(&mut app, grab, y);
        drag_to(&mut app, grab + 10, y);
        assert_eq!(store.load().state.layouts.get(SPLIT.0), None);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.mouse_capture(), "the dialog turns capture off");
        up(&mut app, grab + 10, y);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(
            store.load().state.layouts.get(SPLIT.0).copied(),
            Some(layout(&app)),
            "the dragged layout is saved"
        );
        assert_ne!(layout(&app).list, SPLIT_LAYOUT.list);
    }

    /// MD8: with capture on, the wheel over a pane focuses it and moves it as the keymap's
    /// `list.down`/`list.up` would (what the terminal's alternate scroll sent before capture).
    #[test]
    fn the_wheel_moves_the_pane_under_the_pointer() {
        use crossterm::event::MouseEventKind;

        let mut app = split_app();
        assert!(app.mouse_capture());
        let _ = draw(&mut app, 100, 30);
        let (_, areas) = claim(&app);
        let list = areas.list.expect("shown");
        mouse(&mut app, MouseEventKind::ScrollDown, list.x + 3, list.y + 3);
        mouse(&mut app, MouseEventKind::ScrollDown, list.x + 3, list.y + 3);
        mouse(&mut app, MouseEventKind::ScrollUp, list.x + 3, list.y + 3);
        assert_eq!(
            app.here().and_then(|loc| loc.at.cursor).as_deref(),
            Some("1")
        );
        assert_eq!(focused(&app), PaneId::List);
        mouse(
            &mut app,
            MouseEventKind::ScrollDown,
            areas.detail.x + 3,
            areas.detail.y + 3,
        );
        assert_eq!(focused(&app), PaneId::Detail);
        assert_eq!(
            app.here().and_then(|loc| loc.at.cursor).as_deref(),
            Some("2")
        );
        // Outside every pane (the header), the wheel does nothing.
        mouse(&mut app, MouseEventKind::ScrollDown, 1, 0);
        assert_eq!(
            app.here().and_then(|loc| loc.at.cursor).as_deref(),
            Some("2")
        );
    }

    /// A list + detail tab whose own gesture wants the mouse (as the Runs flow does), counting
    /// the losses it is told of.
    struct Gesture(std::rc::Rc<std::cell::Cell<usize>>);

    impl Tab for Gesture {
        fn id(&self) -> TabId {
            TabId("gesture")
        }
        fn title(&self) -> &str {
            "Gesture"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Pass
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
        fn panes(&self) -> Panes {
            Panes::ListDetail(SPLIT_LAYOUT)
        }
        fn wants_mouse(&self) -> bool {
            true
        }
        fn on_mouse_lost(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    /// MOD-74 L1 with MD8: capture stays on between two list + detail tabs, so the shell tracks
    /// the gesture holder and tells the tabs when it leaves, as on the off edge.
    #[test]
    fn a_switch_between_two_capturing_tabs_tells_the_gesture_holder() {
        let lost = std::rc::Rc::new(std::cell::Cell::new(0));
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.register_tab(Box::new(Gesture(lost.clone())));
        app.register_tab(Box::new(SplitTab::new(SPLIT, "Backlog")));
        assert!(app.mouse_capture());
        assert_eq!(lost.get(), 0);
        press(&mut app, KeyCode::PageDown, KeyModifiers::CONTROL);
        assert!(
            app.mouse_capture(),
            "capture stays on: the list + detail tab wants it"
        );
        assert_eq!(lost.get(), 1, "the holder was told");
        assert!(app.mouse_capture());
        assert_eq!(lost.get(), 1, "once");
    }

    /// A single-pane tab claims nothing: no claim is kept, and the frame is the M1 one.
    #[test]
    fn a_single_pane_tab_claims_nothing() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.register_tab(Box::new(crate::ui::tabs::ChatTab::new()));
        let _ = draw(&mut app, 100, 30);
        assert_eq!(app.last_claim, None);
    }

    /// What a [`Watcher`] saw: each entry the shell called it through, and the focus its `Ctx`
    /// carried there.
    type Seen = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, PaneId)>>>;

    /// A list + detail tab that records `ctx.focus()` at every entry the shell calls it through
    /// (T6R finding 1); `mouse` is whether its own gesture wants the mouse.
    struct Watcher {
        seen: Seen,
        mouse: bool,
    }

    impl Watcher {
        fn saw(&self, entry: &'static str, ctx: &Ctx<'_>) {
            self.seen.borrow_mut().push((entry, ctx.focus()));
        }
    }

    impl Tab for Watcher {
        fn id(&self) -> TabId {
            TabId("watcher")
        }
        fn title(&self) -> &str {
            "Watcher"
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, _key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            self.saw("key", ctx);
            Handled::Pass
        }
        fn on_mouse(&mut self, _mouse: crossterm::event::MouseEvent, ctx: &mut Ctx<'_>) -> Handled {
            self.saw("mouse", ctx);
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
            let areas = ctx.split(area, SPLIT_LAYOUT);
            let block = || Block::bordered().border_type(BorderType::Rounded);
            if let Some(list) = areas.list {
                frame.render_widget(block(), list);
            }
            if !areas.detail.is_empty() {
                frame.render_widget(block(), areas.detail);
            }
        }
        fn key_stack(&self) -> Option<Stack<'static>> {
            Some(views::BACKLOG_BODY)
        }
        fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
            self.saw("hint", ctx);
            Some(Line::from("j/k move"))
        }
        fn panes(&self) -> Panes {
            Panes::ListDetail(SPLIT_LAYOUT)
        }
        fn wants_mouse(&self) -> bool {
            self.mouse
        }
        fn restore(&mut self, _loc: &TabLoc, ctx: &mut Ctx<'_>) -> bool {
            self.saw("restore", ctx);
            true
        }
    }

    /// A shell with a [`Watcher`] registered, about off, drawn once (so it claimed its panes).
    fn watched(mouse: bool) -> (App, Seen) {
        let seen = Seen::default();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        app.about_visible = false;
        app.register_tab(Box::new(Watcher {
            seen: seen.clone(),
            mouse,
        }));
        let _ = draw(&mut app, 100, 30);
        (app, seen)
    }

    /// The focus the watcher's last `entry` call carried.
    fn last_seen(seen: &Seen, entry: &str) -> Option<PaneId> {
        seen.borrow()
            .iter()
            .rev()
            .find(|(at, _)| *at == entry)
            .map(|(_, focus)| *focus)
    }

    /// T6R finding 1 (D3): every `Ctx` the shell builds for a tab carries the tab's focused
    /// pane, not only the render's: `on_key`, `key_hint`, `on_mouse` and `restore` see the
    /// focus `Tab` moved.
    #[test]
    fn every_ctx_the_shell_builds_for_a_tab_carries_its_focus() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let (mut app, seen) = watched(true);
        assert_eq!(last_seen(&seen, "hint"), Some(PaneId::List));
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(focused(&app), PaneId::Detail);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(last_seen(&seen, "key"), Some(PaneId::Detail), "on_key");
        let _ = draw(&mut app, 100, 30);
        assert_eq!(last_seen(&seen, "hint"), Some(PaneId::Detail), "key_hint");
        let (_, _, areas) = app.last_claim.expect("claimed");
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: areas.detail.x + 3,
            row: areas.detail.y + 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(last_seen(&seen, "mouse"), Some(PaneId::Detail), "on_mouse");
        // Back to the place `Tab` left (the list), then forward to the detail: each restore
        // sees the pane it restores.
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(last_seen(&seen, "restore"), Some(PaneId::List), "back");
        press(&mut app, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(last_seen(&seen, "restore"), Some(PaneId::Detail), "forward");
    }

    /// T6R finding 1 with MD8: the wheel over the detail focuses it, and the motion key it
    /// presses reaches the tab with the detail focused.
    #[test]
    fn the_wheel_over_the_detail_reaches_the_tab_with_the_detail_focused() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let (mut app, seen) = watched(false);
        let (_, _, areas) = app.last_claim.expect("claimed");
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: areas.detail.x + 3,
            row: areas.detail.y + 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(focused(&app), PaneId::Detail);
        assert_eq!(last_seen(&seen, "key"), Some(PaneId::Detail));
        assert_eq!(last_seen(&seen, "mouse"), None, "the tab wants no mouse");
    }

    /// T6R finding 2 (D3, D1): a restart onto a saved layout that hides the list (the detail
    /// preset, or the list toggled off) focuses the detail: at load, in the frame (the accent),
    /// in `here()`, and the first `Tab` records no way back to the hidden list.
    #[test]
    fn a_restart_onto_a_layout_that_hides_the_list_focuses_the_detail() {
        use crate::ui::state_file::UiStore;

        let hiding = [
            TabLayout {
                preset: Preset::Focus,
                ..SPLIT_LAYOUT
            },
            TabLayout {
                list_on: false,
                ..SPLIT_LAYOUT
            },
        ];
        for layout in hiding {
            let store = UiStore::memory();
            let mut state = store.load().state;
            state.layouts.insert(SPLIT.0.to_owned(), layout);
            store.save(&state).expect("the memory store saves");

            let mut app = split_app();
            app.apply_ui_state(store.load().state);
            app.ui_store = store;
            assert_eq!(focused(&app), PaneId::Detail, "{layout:?}: at load");
            let here = app.here().expect("a place");
            assert_eq!(here.pane, PaneId::Detail, "{layout:?}");
            assert_eq!(here.label, "Backlog", "{layout:?}: the detail is not named");

            let buf = draw(&mut app, 100, 30);
            let (_, areas) = claim(&app);
            assert_eq!(areas.list, None, "{layout:?}");
            let accent = app.theme.tab_accent(SPLIT.0);
            let detail = areas.detail;
            assert_eq!(
                buf[(detail.x, detail.y)].fg,
                accent.fg.unwrap_or_default(),
                "{layout:?}: the accent is on the detail"
            );

            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
            assert_eq!(focused(&app), PaneId::Detail, "{layout:?}");
            assert!(
                app.history.peek_back().is_none(),
                "{layout:?}: no way back to a hidden pane: {:?}",
                app.history.peek_back().map(|loc| &loc.label)
            );
        }
    }
}
