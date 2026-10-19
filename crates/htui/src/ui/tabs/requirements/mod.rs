//! The Requirements tab (MOD-39 PRD D2, plan P10-P11, blueprint §4): the scope's projects, their
//! areas and requirements on the left, the selected one's body, coverage and revision trail on the
//! right, and the four gated writes: a new area (`a`), a new requirement (`n`), an amend (`e`) and a
//! withdraw (`W`).
//!
//! Every read and write goes through `StoreRequest` and `crate::requirements` (`R-NF-3`): the tab
//! holds no store handle and no `UserId`, renders from the last [`RequirementsSnapshot`] and never
//! patches a row into it. One read per event, never one per keystroke: the filter (`/`, plan P4) is
//! applied to the snapshot as it is typed, and the cursor does not move until `Enter` closes it
//! (MOD-39 blueprint F-13), so typing sends nothing.
//!
//! **The gate is the worker's** (PRD D1, plan P5): a project's requirements are its spec owner's.
//! The tab says the same thing first, from the snapshot's `maintainer` and `writable` flags: a write
//! key on a project this user does not own, or offline, answers the status line and sends nothing,
//! and the hint row dims the four write words.
//!
//! One write in flight (`busy`, the Skills tab's rule), and a write **lands on its own reply**
//! (MOD-59): only a `RequirementWritten` naming the write in flight closes the form; a plain
//! `Requirements` never does, so a read served ahead of the write leaves the form and `busy` alone.
//! A write whose re-read failed lands all the same, and the notice says the tree is the one held. A
//! `RequirementsStale` (an amend or withdraw that missed its version) keeps the form and its text,
//! moves the token to the head and says so; the retry is the user's `Ctrl+S`. Forms save on
//! `Ctrl+S` from any field, because `TextArea` breaks the line on `Enter` (blueprint F-1); a
//! withdraw ends with the requirement's key typed back, the close-out pattern.

mod detail;
mod forms;
mod tree;

use core::cell::Cell;

use htui_core::model::{ProjectId, RequirementArea, RequirementId, RequirementState, Scope};
use htui_core::store::{invalid_area_code, requirement_withdrawn};
use htui_store::DATABASE_UNREACHABLE;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, is_plain, views};
use crate::requirements::{
    BLANK_AREA_TITLE, BLANK_BODY, DECIDING_KEY_NEEDED, DETAIL_NAME, MINT_NAME, READ_NAME,
    RequirementDetail, RequirementText, RequirementWrite, RequirementsSnapshot, is_tab_write,
    mint_refused, not_the_maintainer,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::shell::{ListSize, PaneId, Panes, TabLayout, TabLoc};
use crate::ui::tabs::backlog::detail::{Scroll, Step};
use crate::ui::tabs::backlog::list::window;
use crate::ui::tabs::registry::{CLOSE_THE_FIELD_FIRST, Tab, TabId};
use crate::ui::tabs::settings::{modal_rest, wrapped};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

use forms::{AreaForm, FormFocus, FormOutcome, FormTarget, RequirementForm, WithdrawForm};
use tree::{Fold, Row, TreeView};

/// The Requirements tab's default layout (MOD-115 M2 D2): the tree beside the detail, 45% of
/// the rows above the notice and the filter row, as `Layout::horizontal([Percentage(45),
/// Percentage(55)])` split them before the shell kept layouts.
pub const REQUIREMENTS_LAYOUT: TabLayout = TabLayout {
    list: ListSize::Percent(45),
    ..TabLayout::DEFAULT
};

/// How a [`Row`] is written into a [`TabLoc`] cursor (MOD-115 M2 §8): `requirement:{id}:{key}`
/// (the key names a miss, as a reveal's does), `area:{id}` or `project:{id}`.
const REQUIREMENT_CURSOR: &str = "requirement";

/// An area header's cursor, beside [`REQUIREMENT_CURSOR`].
const AREA_CURSOR: &str = "area";

/// A project header's cursor, beside [`REQUIREMENT_CURSOR`].
const PROJECT_CURSOR: &str = "project";

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The tree before the first reply.
const NOT_READ: &str = "requirements not read yet";

/// A tree with nothing under the filter.
const NOTHING_MATCHES: &str = "nothing matches the filter";

/// A scope with no project to show.
const NO_PROJECTS: &str = "No projects in this workspace.";

/// The detail pane with nothing selected.
const SELECT_A_ROW: &str = "select a requirement";

/// A write went out.
const SAVING: &str = "saving\u{2026}";

/// `n` on a project row.
const SELECT_AN_AREA: &str = "select an area first";

/// `e` or `W` on a header row.
const SELECT_A_REQUIREMENT: &str = "select a requirement first";

/// The withdraw's typed-back key did not match.
const NOT_THE_REQUIREMENT_KEY: &str = "that is not the requirement's key";

/// The hint row's words before the write keys (MOD-67 M4 D9).
const HINT_MOVE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::ListFold, "fold"),
    Hint::One(Act::RequirementsFilter, "filter"),
];

/// The four write keys, dimmed when the selected project refuses them.
const HINT_WRITES: HintSpec = &[
    Hint::One(Act::RequirementsNewArea, "area"),
    Hint::One(Act::New, "new"),
    Hint::One(Act::RequirementsAmend, "amend"),
    Hint::One(Act::RequirementsWithdraw, "withdraw"),
];

/// The hint row's words after the write keys.
const HINT_RELOAD: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// The hint row while the filter is open: the field's own keys, `field.submit` through the keys
/// (MOD-115 M2 MD1) and `Esc` as written (MD7).
const FILTER_HINT: HintSpec = &[
    Hint::One(Act::FieldSubmit, "apply"),
    Hint::Text("Esc clear"),
];

/// The filter field's width on the filter row.
const FILTER_WIDTH: u16 = 30;

/// The hint row on the area and requirement forms (MOD-67 M4 D9).
const FORM_HINT: HintSpec = &[
    Hint::One(Act::FormNextField, "field"),
    Hint::One(Act::FormSave, "save"),
    Hint::Text("Esc cancel"),
];

/// Added to [`FORM_HINT`] on the priority field: the choice's own value keys (D10).
const PRIORITY_HINT: HintSpec = &[Hint::Text("m/l priority")];

/// The hint row on the withdraw form: its field's own keys.
const WITHDRAW_HINT: HintSpec = &[Hint::Text("Enter next"), Hint::Text("Esc cancel")];

/// A key while a write is in flight: one at a time (the Skills tab's rule).
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// MOD-59 D5: a write that applied although its re-read failed. It landed; the tab still draws
/// what it held, and `reload` (`Reload`'s label through `REQUIREMENTS_BROWSE`, MOD-67 M6 D4) reads
/// again; with none bound the clause goes.
fn landed_unread(landed: &str, why: &str, reload: Option<&str>) -> String {
    match reload {
        Some(reload) => format!("{landed} \u{2014} the re-read failed, {reload} reloads: {why}"),
        None => format!("{landed} \u{2014} the re-read failed: {why}"),
    }
}

/// MOD-59 review M1: a mint's `Failed`. A mint whose COMMIT landed but whose answer was lost comes
/// back as the same `Failed`, and its retry would mint a second requirement, so the notice does not
/// call it a refusal. The form keeps its text, and the tab re-reads the tree, which is drawn beside
/// the form: the check needs no `Esc`, which would drop the draft (re-review L1). `save` is
/// `FormSave`'s label through `REQUIREMENTS_FORM` (MOD-67 M6 D4); with none bound the sentence
/// says to look first.
fn mint_failed(why: &str, save: Option<&str>) -> String {
    let hedge = format!(
        "{why} \u{2014} it may have been written; the tree is being re-read, look for it there"
    );
    match save {
        Some(save) => format!("{hedge} before {save}"),
        None => format!("{hedge} first"),
    }
}

/// A `RequirementsStale` over an open form: the text is kept and the token moved to the head.
/// The tab's own sentence rather than Settings' `CHANGED_ELSEWHERE`, whose retry is `Enter`
/// (blueprint F-1). `save` is `FormSave`'s label through the open form's stack (MOD-67 M6 D4);
/// with none bound the clause goes.
fn requirement_changed_elsewhere(head: i32, save: Option<&str>) -> String {
    let kept =
        format!("changed elsewhere since you opened it \u{2014} now v{head}; your text is kept");
    match save {
        Some(save) => format!("{kept} and {save} saves over it"),
        None => kept,
    }
}

/// The [`Row`] a [`TabLoc`] cursor written by [`RequirementsTab`]'s `location` names, with a
/// requirement's key (empty for a header); `None` for any other string (MOD-115 M2 §8).
fn parse_cursor(cursor: &str) -> Option<(Row, String)> {
    let mut parts = cursor.splitn(3, ':');
    let (kind, id) = (parts.next()?, parts.next()?);
    let row = match kind {
        REQUIREMENT_CURSOR => Row::Requirement(id.parse().ok()?),
        AREA_CURSOR => Row::Area(id.parse().ok()?),
        PROJECT_CURSOR => Row::Project(id.parse().ok()?),
        _ => return None,
    };
    Some((row, parts.next().unwrap_or_default().to_owned()))
}

/// The Requirements tab's sentence for a reveal of a requirement this workspace does not hold
/// (MOD-64 D235).
#[must_use]
pub fn not_in_these_requirements(key: &str) -> String {
    format!("{key} is not in this workspace's requirements")
}

/// The Requirements tab (MOD-39 PRD D2, plan P10).
#[derive(Debug, Default)]
pub struct RequirementsTab {
    /// The last read, or `None` before the first reply.
    snapshot: Option<RequirementsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`, until the next good read. With no snapshot
    /// the tree pane draws it; over a snapshot it is the notice and the tree stays drawn.
    unavailable: Option<String>,
    /// Folded headers.
    folded: Vec<Fold>,
    /// The cursor, or `None` before the first reply.
    selected: Option<Row>,
    /// The filter (plan P4); empty is none.
    filter: String,
    /// The selected requirement's detail, `None` until its reply.
    detail: Option<RequirementDetail>,
    /// A refused `DETAIL_NAME`, drawn in the pane.
    detail_error: Option<String>,
    /// The detail pane's first drawn row.
    scroll: Scroll,
    /// The detail pane's rows at the last draw, what `scroll` clamps against. A `Cell` because the
    /// count is known only in `render(&self)`.
    pane_rows: Cell<usize>,
    /// Browsing, filtering, or a form.
    mode: Mode,
    /// The tab write in flight, by `StoreRequest::name`: one at a time, and the only write a
    /// `RequirementWritten` lands (MOD-59 D4).
    busy: Option<&'static str>,
    /// The last outcome, on the notice rows above the filter row.
    notice: Option<Notice>,
    /// A reveal waiting for the next `Requirements` reply (MOD-64 D235).
    pending_reveal: Option<(RequirementId, String)>,
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the tree.
    #[default]
    Browse,
    /// `/`: the filter as typed; every key updates `filter` and sends nothing (F-13).
    Filter {
        /// The filter.
        field: TextField,
    },
    /// `a`.
    NewArea(AreaForm),
    /// `n` (`FormTarget::Mint`) and `e` (`FormTarget::Amend`).
    Requirement(RequirementForm),
    /// `W`.
    Withdraw(WithdrawForm),
}

/// The report on the notice rows, above the filter row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`: something the user has to act on.
    Error(String),
}

/// A project's slug, or its id when the scope does not list it: the worker's `project_label`, so
/// the gate's sentence is spelled the same on both sides.
fn slug(ctx: &Ctx<'_>, project: ProjectId) -> String {
    ctx.projects
        .iter()
        .find(|row| row.project_id == project)
        .map_or_else(|| project.to_string(), |row| row.slug.clone())
}

impl RequirementsTab {
    /// Identity of the Requirements tab.
    pub const ID: TabId = TabId("requirements");

    /// A tab with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current mode's stack (MOD-67 M4 D4): the only place a mode maps to its keys.
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::REQUIREMENTS_BROWSE,
            Mode::Filter { .. } => views::CAPTURE,
            Mode::NewArea(_) => views::AREA_FORM,
            Mode::Requirement(_) => views::REQUIREMENTS_FORM,
            Mode::Withdraw(_) => views::REQUIREMENTS_WITHDRAW,
        }
    }

    /// The visible rows under the current filter and folds.
    fn rows(&self, ctx: &Ctx<'_>) -> Vec<Row> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            tree::rows(snapshot, ctx.projects, &self.folded, &self.filter)
        })
    }

    /// Moves the cursor to `next`; landing on another requirement asks for its detail.
    fn go(&mut self, next: Option<Row>, ctx: &Ctx<'_>) {
        if next == self.selected {
            return;
        }
        self.selected = next;
        self.detail = None;
        self.detail_error = None;
        self.scroll.reset();
        if let Some(Row::Requirement(id)) = next {
            ctx.request(StoreRequest::RequirementDetail(id));
        }
    }

    /// Moves the cursor `delta` rows, clamped to the ends.
    fn step(&mut self, delta: isize, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let current = rows
            .iter()
            .position(|row| Some(*row) == self.selected)
            .unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(last);
        self.go(rows.get(next).copied(), ctx);
    }

    /// The first or the last row (`g` / `G`).
    fn jump(&mut self, to_end: bool, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        let target = if to_end { rows.last() } else { rows.first() };
        self.go(target.copied(), ctx);
    }

    /// Keeps the cursor on a row that is still visible, else puts it on the first requirement (or
    /// the first row). One detail read at most.
    fn reselect(&mut self, ctx: &Ctx<'_>) {
        let rows = self.rows(ctx);
        if rows.iter().any(|row| Some(*row) == self.selected) {
            return;
        }
        let first = rows
            .iter()
            .find(|row| matches!(row, Row::Requirement(_)))
            .or_else(|| rows.first());
        self.go(first.copied(), ctx);
    }

    /// Folds or unfolds the selected header; a requirement row has nothing to fold.
    fn fold(&mut self) -> Handled {
        let fold = match self.selected {
            Some(Row::Project(id)) => Fold::Project(id),
            Some(Row::Area(id)) => Fold::Area(id),
            Some(Row::Requirement(_)) | None => return Handled::Pass,
        };
        if let Some(at) = self.folded.iter().position(|row| *row == fold) {
            self.folded.remove(at);
        } else {
            self.folded.push(fold);
        }
        Handled::Consumed
    }

    /// The project a row belongs to.
    fn project_of(snapshot: &RequirementsSnapshot, row: Row) -> Option<ProjectId> {
        match row {
            Row::Project(id) => Some(id),
            Row::Area(id) => snapshot.area(id).map(|area| area.project_id),
            Row::Requirement(id) => snapshot.requirement(id).map(|row| row.project_id),
        }
    }

    /// Whether the selected row's project takes a write: the snapshot is writable and this user
    /// maintains it. What the hint row dims on.
    fn writes_allowed(&self) -> bool {
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        snapshot.writable
            && self
                .selected
                .and_then(|row| Self::project_of(snapshot, row))
                .and_then(|project| snapshot.project(project))
                .is_some_and(|entry| entry.maintainer)
    }

    /// `requirements.new_area`, `common.new`, `requirements.amend` or `requirements.withdraw`
    /// (`a`, `n`, `e`, `W` by default; blueprint §4.4), checked in order: a write in flight;
    /// offline; not the maintainer; a withdrawn requirement for an amend or withdraw. None of
    /// these sends anything.
    fn write_key(&mut self, act: Act, ctx: &Ctx<'_>) {
        let (Some(snapshot), Some(row)) = (&self.snapshot, self.selected) else {
            return;
        };
        let Some(project) = Self::project_of(snapshot, row) else {
            return;
        };
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        if !snapshot.writable {
            ctx.emit(Action::Error(DATABASE_UNREACHABLE.to_owned()));
            return;
        }
        if !snapshot
            .project(project)
            .is_some_and(|entry| entry.maintainer)
        {
            ctx.emit(Action::Error(not_the_maintainer(&slug(ctx, project))));
            return;
        }
        let mode = match (act, row) {
            (Act::RequirementsNewArea, _) => Mode::NewArea(AreaForm::new(project)),
            (Act::New, Row::Project(_)) => {
                self.notice = Some(Notice::Info(SELECT_AN_AREA.to_owned()));
                return;
            }
            (Act::New, Row::Area(id)) => {
                let Some(area) = snapshot.area(id) else {
                    return;
                };
                Mode::Requirement(RequirementForm::mint(project, id, area.code.clone()))
            }
            (Act::New, Row::Requirement(id)) => {
                let Some(requirement) = snapshot.requirement(id) else {
                    return;
                };
                Mode::Requirement(RequirementForm::mint(
                    project,
                    requirement.area_id,
                    requirement.area_code.clone(),
                ))
            }
            (Act::RequirementsAmend | Act::RequirementsWithdraw, Row::Requirement(id)) => {
                let Some(requirement) = snapshot.requirement(id) else {
                    return;
                };
                if requirement.state == RequirementState::Withdrawn {
                    ctx.emit(Action::Error(requirement_withdrawn(&requirement.key)));
                    return;
                }
                if act == Act::RequirementsAmend {
                    Mode::Requirement(RequirementForm::amend(
                        id,
                        requirement.key.clone(),
                        requirement.version,
                        &requirement.body,
                        &requirement.rationale,
                        requirement.priority,
                    ))
                } else {
                    Mode::Withdraw(WithdrawForm::new(
                        id,
                        requirement.key.clone(),
                        requirement.version,
                    ))
                }
            }
            (Act::RequirementsAmend | Act::RequirementsWithdraw, _) => {
                self.notice = Some(Notice::Info(SELECT_A_REQUIREMENT.to_owned()));
                return;
            }
            _ => return,
        };
        self.notice = None;
        self.mode = mode;
    }

    /// Browse (blueprint §4.4; MOD-67 M4 D6): the first candidate of the browse stack this state
    /// accepts. A global act, or one this state declines (`list.fold` on a requirement row,
    /// `common.back` with no filter), is the shell's, through the same stack.
    ///
    /// MOD-115 M2 §7: only motion follows the focus. The tree's motion and its fold are the
    /// tree's while it is focused; with the detail focused the motion scrolls the detail (the
    /// `pane.scroll_*` arm's scroll); with the activity pane focused it is taken and nothing
    /// moves. Every other act acts whatever pane is focused.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        let focus = ctx.focus();
        for act in ctx.keys().actions(views::REQUIREMENTS_BROWSE, chord) {
            if let (Some(step), false) = (Step::of(act), focus == PaneId::List) {
                if focus == PaneId::Detail {
                    return self.scroll.follow(step, self.pane_rows.get());
                }
                return Handled::Consumed;
            }
            match act {
                Act::ListDown => self.step(1, ctx),
                Act::ListUp => self.step(-1, ctx),
                Act::ListTop => self.jump(false, ctx),
                Act::ListBottom => self.jump(true, ctx),
                // A requirement row has nothing to fold, and the fold is the tree's.
                Act::ListFold => {
                    if focus != PaneId::List || self.fold() == Handled::Pass {
                        continue;
                    }
                }
                Act::PaneScrollDown | Act::PaneScrollUp | Act::PanePageDown | Act::PanePageUp => {
                    return self.scroll.apply(act, self.pane_rows.get());
                }
                Act::RequirementsFilter => {
                    self.notice = None;
                    self.mode = Mode::Filter {
                        field: TextField::with_text(&self.filter),
                    };
                }
                // Only with a filter to clear: otherwise `Esc` is the shell's.
                Act::Back if !self.filter.is_empty() => {
                    self.filter.clear();
                    self.reselect(ctx);
                }
                Act::RequirementsNewArea
                | Act::New
                | Act::RequirementsAmend
                | Act::RequirementsWithdraw => self.write_key(act, ctx),
                Act::Reload => {
                    self.notice = None;
                    ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
                    if let Some(Row::Requirement(id)) = self.selected {
                        ctx.request(StoreRequest::RequirementDetail(id));
                    }
                }
                _ => continue, // a global act, or one this state declines
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    /// The filter: every key updates `filter` and sends nothing (F-13); `Enter` applies and
    /// re-selects, `Esc` clears and re-selects (the field's own keys, MOD-67 M4 D10). A key the
    /// field passes is swallowed but for the chords its stack passes (`ctrl-c`, `F1`); the stack
    /// offers no `form.save`, so `ctrl-s` saves nothing here.
    fn on_filter_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = self.stack();
        let Mode::Filter { field } = &mut self.mode else {
            return Handled::Pass;
        };
        match field.on_key(key) {
            FieldOutcome::Consumed => {
                field
                    .text()
                    .unwrap_or_default()
                    .clone_into(&mut self.filter);
            }
            FieldOutcome::Submit => {
                self.mode = Mode::Browse;
                self.reselect(ctx);
            }
            FieldOutcome::Cancel => {
                self.filter.clear();
                self.mode = Mode::Browse;
                self.reselect(ctx);
            }
            FieldOutcome::Pass => return modal_rest(stack, KeyChord::from_event(key)),
        }
        Handled::Consumed
    }

    /// A key on an open form (MOD-67 M4 D6, PA-3). A plain key goes to the focused widget first;
    /// a chord never reaches a widget, so `ctrl-m` sets no priority and the `TextArea` never sees
    /// `ctrl-s`. What the widget passes, and every chord, is resolved through the form's stack:
    /// `form.save` saves, `form.next_field`/`prev_field` move the focus, and the rest is swallowed
    /// but for the chords the stack passes (CONTROL, ALT, function keys: `ctrl-c`, `F1`).
    ///
    /// While a write is in flight the reply closes the form or keeps it, and a form closed now
    /// would leave it nothing to land on: `form.save` says the write is in flight, a chord the
    /// stack passes still reaches the shell, everything else is swallowed.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = self.stack();
        let chord = KeyChord::from_event(key);
        if self.busy.is_some() {
            return match ctx.keys().actions(stack, chord).first() {
                Some(Act::FormSave) => {
                    self.save(ctx);
                    Handled::Consumed
                }
                _ => modal_rest(stack, chord),
            };
        }
        // MOD-115 M2 D7: the requirement form resolves its priority's keys itself, chords too.
        let outcome = match &mut self.mode {
            Mode::Requirement(form) => form.on_key(key, ctx.keys()),
            Mode::NewArea(form) if is_plain(&key) => form.on_key(key),
            Mode::Withdraw(form) if is_plain(&key) => form.on_key(key),
            Mode::NewArea(_) | Mode::Withdraw(_) => FormOutcome::Pass,
            Mode::Browse | Mode::Filter { .. } => return Handled::Pass,
        };
        match outcome {
            FormOutcome::Stay => {}
            FormOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            FormOutcome::Submit => self.save(ctx),
            FormOutcome::Pass => match ctx.keys().actions(stack, chord).first() {
                Some(Act::FormSave) => self.save(ctx),
                Some(Act::FormNextField) => self.focus_step(true),
                Some(Act::FormPrevField) => self.focus_step(false),
                _ => return modal_rest(stack, chord),
            },
        }
        Handled::Consumed
    }

    /// `form.next_field` (`forward`) or `form.prev_field`: the area form's other field, or the
    /// requirement form's next or previous one, wrapping. The withdraw form has one field.
    fn focus_step(&mut self, forward: bool) {
        match &mut self.mode {
            Mode::NewArea(form) => form.toggle(),
            Mode::Requirement(form) => form.cycle(forward),
            Mode::Browse | Mode::Filter { .. } | Mode::Withdraw(_) => {}
        }
    }

    /// `Ctrl+S`, or `Enter` where it submits: validate, then send; a withdraw's first stage moves
    /// on to the typed key instead.
    fn save(&mut self, ctx: &Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.notice = Some(Notice::Error(in_flight(busy)));
            return;
        }
        let scope = ctx.scope.clone();
        match &mut self.mode {
            Mode::NewArea(form) => {
                let (code, title) = (form.code(), form.title());
                if !RequirementArea::code_is_valid(&code) {
                    self.notice = Some(Notice::Error(invalid_area_code(&code)));
                    return;
                }
                if title.is_empty() {
                    self.notice = Some(Notice::Error(BLANK_AREA_TITLE.to_owned()));
                    return;
                }
                let project = form.project;
                self.send(
                    StoreRequest::CreateRequirementArea {
                        scope,
                        project,
                        code,
                        title,
                    },
                    ctx,
                );
            }
            Mode::Requirement(form) => {
                let body = form.body.text().to_owned();
                if body.trim().is_empty() {
                    self.notice = Some(Notice::Error(BLANK_BODY.to_owned()));
                    return;
                }
                let rationale = RequirementText::new(form.rationale.text());
                let priority = form.priority;
                let request = match &form.target {
                    FormTarget::Mint { project, area, .. } => StoreRequest::MintRequirement {
                        scope,
                        project: *project,
                        area: *area,
                        body: RequirementText::new(body),
                        rationale,
                        priority,
                    },
                    FormTarget::Amend {
                        id,
                        expected_version,
                        ..
                    } => {
                        // A head withdrawn elsewhere (a stale answer kept the form open) takes no
                        // retry: say so here rather than send one that can only come back stale.
                        if let Some(row) = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.requirement(*id))
                            .filter(|row| row.state == RequirementState::Withdrawn)
                        {
                            self.notice = Some(Notice::Error(requirement_withdrawn(&row.key)));
                            return;
                        }
                        let deciding = form.deciding();
                        if deciding.is_empty() {
                            form.focus = FormFocus::Deciding;
                            self.notice = Some(Notice::Error(DECIDING_KEY_NEEDED.to_owned()));
                            return;
                        }
                        StoreRequest::AmendRequirement {
                            scope,
                            id: *id,
                            expected_version: *expected_version,
                            body: RequirementText::new(body),
                            rationale,
                            priority,
                            deciding,
                        }
                    }
                };
                self.send(request, ctx);
            }
            Mode::Withdraw(WithdrawForm::Deciding {
                id,
                key,
                expected_version,
                field,
            }) => {
                let deciding = field.text().unwrap_or_default().trim().to_owned();
                if deciding.is_empty() {
                    self.notice = Some(Notice::Error(DECIDING_KEY_NEEDED.to_owned()));
                    return;
                }
                self.notice = None;
                self.mode = Mode::Withdraw(WithdrawForm::Typed {
                    id: *id,
                    key: core::mem::take(key),
                    expected_version: *expected_version,
                    deciding,
                    field: TextField::new(),
                });
            }
            Mode::Withdraw(WithdrawForm::Typed {
                id,
                key,
                expected_version,
                deciding,
                field,
            }) => {
                if field.text() != Some(key.as_str()) {
                    field.clear();
                    self.notice = Some(Notice::Error(NOT_THE_REQUIREMENT_KEY.to_owned()));
                    return;
                }
                let request = StoreRequest::WithdrawRequirement {
                    scope,
                    id: *id,
                    expected_version: *expected_version,
                    deciding: deciding.clone(),
                };
                self.send(request, ctx);
            }
            Mode::Browse | Mode::Filter { .. } => {}
        }
    }

    /// Sends one tab write and marks it in flight.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(request);
    }

    /// MOD-59 D4: the tab write in flight landed. `outcome` is the store's own answer, so nothing
    /// here searches the snapshot: the form closes, the notice says what landed, and the cursor
    /// goes to the row it landed on, whose detail is re-read, when the tree drawn holds it.
    /// `unread` is the re-read's failure (D5): the tree is the one held before the write, which may
    /// not hold a new row yet, and no detail is read (review L4). A reply for any other write than
    /// the one `busy` names lands nothing, and the answer is `false` (review L3).
    fn land(&mut self, outcome: &RequirementWrite, unread: Option<&str>, ctx: &Ctx<'_>) -> bool {
        if self.busy != Some(outcome.request_name()) {
            return false;
        }
        let (landed, row) = match outcome {
            RequirementWrite::Area { id, code } => (format!("added area {code}"), Row::Area(*id)),
            RequirementWrite::Minted { id, key } => {
                (format!("minted {key}"), Row::Requirement(*id))
            }
            RequirementWrite::Amended { id, key, version } => (
                format!("amended {key} to v{version}"),
                Row::Requirement(*id),
            ),
            RequirementWrite::Withdrawn { id, key } => {
                (format!("withdrew {key}"), Row::Requirement(*id))
            }
        };
        self.busy = None;
        self.mode = Mode::Browse;
        self.notice = Some(match unread {
            None => Notice::Info(landed),
            Some(why) => Notice::Error(landed_unread(
                &landed,
                why,
                ctx.keys()
                    .label(views::REQUIREMENTS_BROWSE, Act::Reload)
                    .as_deref(),
            )),
        });
        let held = self.snapshot.as_ref().is_some_and(|snapshot| match row {
            Row::Area(id) => snapshot.area(id).is_some(),
            Row::Requirement(id) => snapshot.requirement(id).is_some(),
            Row::Project(id) => snapshot.project(id).is_some(),
        });
        if !held {
            return true;
        }
        match unread {
            None => self.select_row(row, ctx),
            // The store has just failed a read, and the detail's would most likely fail too
            // (MOD-59 review L4): the pane says why rather than show the row before the write.
            Some(why) => {
                self.put_cursor(row, ctx);
                if matches!(row, Row::Requirement(_)) {
                    self.detail = None;
                    self.detail_error = Some(why.to_owned());
                }
            }
        }
        true
    }

    /// A read's tail, shared by a `Requirements` reply and a `RequirementWritten` that lands
    /// nothing (MOD-59 H-10): a pending reveal is decided, and the cursor re-selected.
    fn after_read(&mut self, ctx: &Ctx<'_>) {
        // MOD-64 D251: a reveal of a requirement that was not read is decided by this one.
        if let Some((id, key)) = self.pending_reveal.take() {
            if self.holds(id) {
                self.select_row(Row::Requirement(id), ctx);
            } else {
                self.notice = Some(Notice::Error(not_in_these_requirements(&key)));
            }
        }
        self.reselect(ctx);
    }

    /// Makes `row` visible, puts the cursor on it and re-reads its detail: `land`'s tail, shared
    /// with a reveal (MOD-64 D235). The detail is re-read even when `row` was already selected —
    /// `land` needs that, the row having just been written.
    fn select_row(&mut self, row: Row, ctx: &Ctx<'_>) {
        self.put_cursor(row, ctx);
        if let Row::Requirement(id) = row {
            ctx.request(StoreRequest::RequirementDetail(id));
        }
    }

    /// Makes `row` visible and puts the cursor on it, reading nothing: `select_row` without the
    /// detail read.
    fn put_cursor(&mut self, row: Row, ctx: &Ctx<'_>) {
        self.unhide(row, ctx);
        if self.selected != Some(row) {
            self.detail = None;
            self.scroll.reset();
        }
        self.selected = Some(row);
        self.detail_error = None;
    }

    /// Whether the last read holds requirement `id` (MOD-64 D235).
    fn holds(&self, id: RequirementId) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.requirement(id).is_some())
    }

    /// Makes `row` visible: unfolds what hides it, and drops a filter that would.
    fn unhide(&mut self, row: Row, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let project = Self::project_of(snapshot, row);
        let area = match row {
            Row::Area(id) => Some(id),
            Row::Requirement(id) => snapshot.requirement(id).map(|row| row.area_id),
            Row::Project(_) => None,
        };
        self.folded.retain(|fold| match fold {
            Fold::Project(id) => Some(*id) != project || row == Row::Project(*id),
            Fold::Area(id) => Some(*id) != area || row == Row::Area(*id),
        });
        if !self.rows(ctx).contains(&row) {
            self.filter.clear();
        }
    }

    /// A good read after a refused one: the refusal goes, and so does its notice if nothing has
    /// replaced it.
    fn recovered(&mut self) {
        if let Some(error) = self.unavailable.take()
            && self.notice == Some(Notice::Error(error))
        {
            self.notice = None;
        }
    }

    /// Whether a mint's `Failed` is a refusal the worker gave before the insert, which wrote
    /// nothing (MOD-59 re-review L2). With no mint form open there is nothing to name the project
    /// and area by, and the answer is `false`: the hedge is the safe side.
    fn mint_refused(&self, message: &str, ctx: &Ctx<'_>) -> bool {
        let Mode::Requirement(RequirementForm {
            target: FormTarget::Mint { project, area, .. },
            ..
        }) = &self.mode
        else {
            return false;
        };
        mint_refused(message, &slug(ctx, *project), *area)
    }

    /// A `RequirementsStale` over the write in flight: the form stays with its text, its token
    /// moves to the head, and the notice says so. A head withdrawn elsewhere takes no retry, and
    /// the notice says that instead. The retry is the save key `keys` binds in the open form's
    /// stack (MOD-67 M6 D4).
    fn stale(&mut self, keys: &Keys) {
        if self.busy.take().is_none() {
            return;
        }
        let id = match &self.mode {
            Mode::Requirement(RequirementForm {
                target: FormTarget::Amend { id, .. },
                ..
            }) => *id,
            Mode::Withdraw(form) => form.id(),
            _ => return,
        };
        let Some((head, state, key)) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.requirement(id))
            .map(|row| (row.version, row.state, row.key.clone()))
        else {
            return;
        };
        if state == RequirementState::Withdrawn {
            // Withdrawn elsewhere: no retry can land, so the form does not offer one. An amend
            // keeps its text on screen until `Esc`; a withdraw has nothing left to do.
            if matches!(self.mode, Mode::Withdraw(_)) {
                self.mode = Mode::Browse;
            }
            self.notice = Some(Notice::Error(requirement_withdrawn(&key)));
            return;
        }
        let stack = match &mut self.mode {
            Mode::Requirement(RequirementForm {
                target:
                    FormTarget::Amend {
                        expected_version, ..
                    },
                ..
            }) => {
                *expected_version = head;
                views::REQUIREMENTS_FORM
            }
            Mode::Withdraw(form) => {
                form.set_expected_version(head);
                views::REQUIREMENTS_WITHDRAW
            }
            _ => views::REQUIREMENTS_FORM,
        };
        let save = keys.label(stack, Act::FormSave);
        self.notice = Some(Notice::Error(requirement_changed_elsewhere(
            head,
            save.as_deref(),
        )));
    }

    // --- render --------------------------------------------------------------------------------

    /// The left pane.
    fn render_tree(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let count = self.snapshot.as_ref().map_or(0, |snapshot| {
            tree::count(snapshot, ctx.projects, &self.filter)
        });
        let title = if self.filter.is_empty() {
            format!(" Requirements ({count}) ")
        } else {
            format!(" Requirements ({count}) \u{b7} /{} ", self.filter)
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let message = |text: &str, style: Style| {
            Paragraph::new(Line::styled(text.to_owned(), style)).wrap(Wrap { trim: true })
        };
        let Some(snapshot) = &self.snapshot else {
            let (text, style) = self
                .unavailable
                .as_ref()
                .map_or((NOT_READ, ctx.theme.dim), |error| {
                    (error.as_str(), ctx.theme.error)
                });
            frame.render_widget(message(text, style), inner);
            return;
        };
        let view = TreeView {
            snapshot,
            projects: ctx.projects,
            folded: &self.folded,
            filter: &self.filter,
            selected: self.selected,
        };
        let (mut lines, cursor) = tree::lines(&view, usize::from(inner.width), ctx.theme);
        if lines.is_empty() {
            let text = if self.filter.is_empty() {
                NO_PROJECTS
            } else {
                NOTHING_MATCHES
            };
            frame.render_widget(message(text, ctx.theme.dim), inner);
            return;
        }
        let offset = window(cursor.unwrap_or(0), lines.len(), usize::from(inner.height));
        frame.render_widget(
            Paragraph::new(lines.split_off(offset.min(lines.len()))),
            inner,
        );
    }

    /// The right pane: the open form, or what the selected row says.
    fn render_detail(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let snapshot = self.snapshot.as_ref();
        let project_name = |id: ProjectId| {
            ctx.projects
                .iter()
                .find(|row| row.project_id == id)
                .map_or_else(|| id.to_string(), |row| row.name.clone())
        };
        let title = match &self.mode {
            Mode::NewArea(form) => format!(" New area in {} ", project_name(form.project)),
            Mode::Requirement(form) => match &form.target {
                FormTarget::Mint { code, .. } => format!(" New requirement in {code} "),
                FormTarget::Amend {
                    key,
                    expected_version,
                    ..
                } => format!(" Amend {key} (v{expected_version}) "),
            },
            Mode::Withdraw(form) => format!(" Withdraw {} ", form.key()),
            Mode::Browse | Mode::Filter { .. } => match (self.selected, snapshot) {
                (Some(Row::Requirement(id)), Some(snapshot)) => snapshot
                    .requirement(id)
                    .map_or_else(String::new, |row| format!(" {} ", row.key)),
                (Some(Row::Area(id)), Some(snapshot)) => snapshot
                    .area(id)
                    .map_or_else(String::new, |row| format!(" {} ", row.code)),
                (Some(Row::Project(id)), _) => format!(" {} ", project_name(id)),
                _ => String::new(),
            },
        };
        let block = Block::new().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        match &self.mode {
            Mode::NewArea(form) => return form.render(frame, inner, theme),
            Mode::Requirement(form) => return form.render(frame, inner, theme),
            Mode::Withdraw(form) => {
                let body = snapshot
                    .and_then(|snapshot| snapshot.requirement(form.id()))
                    .map_or("", |row| row.body.as_str());
                return form.render(frame, inner, body, theme);
            }
            Mode::Browse | Mode::Filter { .. } => {}
        }
        let lines = match (self.selected, snapshot) {
            (Some(Row::Requirement(id)), Some(snapshot)) => {
                match (&self.detail, &self.detail_error) {
                    (Some(detail), _) if detail.requirement.id == id => {
                        detail::lines(detail, inner.width, theme)
                    }
                    (_, Some(error)) => vec![Line::styled(error.clone(), theme.error)],
                    _ => {
                        let key = snapshot.requirement(id).map_or("", |row| row.key.as_str());
                        vec![Line::styled(format!("reading {key}\u{2026}"), theme.dim)]
                    }
                }
            }
            (Some(Row::Area(id)), Some(snapshot)) => snapshot
                .area(id)
                .and_then(|area| {
                    snapshot
                        .project(area.project_id)
                        .map(|entry| detail::area_lines(area, entry, inner.width, theme))
                })
                .unwrap_or_default(),
            (Some(Row::Project(id)), Some(snapshot)) => ctx
                .projects
                .iter()
                .find(|row| row.project_id == id)
                .zip(snapshot.project(id))
                .map(|(project, entry)| detail::project_lines(project, entry, inner.width, theme))
                .unwrap_or_default(),
            _ => vec![Line::styled(SELECT_A_ROW, theme.dim)],
        };
        self.pane_rows.set(lines.len());
        frame.render_widget(
            Paragraph::new(lines).scroll((self.scroll.offset(), 0)),
            inner,
        );
    }

    /// The keys of the current mode for the shell's hint row, through its stack and `keys`
    /// (MOD-67 M4 D9, M6 T2): Browse's three groups, the writes greyed where a write would be
    /// refused; a form's spec. `None` when no key of it is bound.
    fn keys_line(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        let line = match &self.mode {
            Mode::Browse => {
                let writes = if self.writes_allowed() {
                    theme.base
                } else {
                    theme.dim
                };
                // Three groups, the writes in their own style; an unbound group drops out with
                // its separator.
                let stack = self.stack();
                let mut spans = Vec::new();
                for (spec, style) in [
                    (HINT_MOVE, theme.base),
                    (HINT_WRITES, writes),
                    (HINT_RELOAD, theme.base),
                ] {
                    let text = keys.hint(stack, spec);
                    if text.is_empty() {
                        continue;
                    }
                    if !spans.is_empty() {
                        spans.push(Span::styled(" \u{b7} ", theme.base));
                    }
                    spans.push(Span::styled(text, style));
                }
                Line::from(spans)
            }
            Mode::Filter { .. } => Line::styled(keys.hint(self.stack(), FILTER_HINT), theme.dim),
            Mode::NewArea(_) => Line::styled(keys.hint(self.stack(), FORM_HINT), theme.dim),
            Mode::Requirement(form) => {
                let mut text = keys.hint(self.stack(), FORM_HINT);
                if form.focus == FormFocus::Priority {
                    text.push_str(" \u{b7} ");
                    text.push_str(&keys.hint(self.stack(), PRIORITY_HINT));
                }
                Line::styled(text, theme.dim)
            }
            Mode::Withdraw(_) => Line::styled(keys.hint(self.stack(), WITHDRAW_HINT), theme.dim),
        };
        (line.width() > 0).then_some(line)
    }

    /// The row under the notice: the filter's input (` /` and the field) while it is open, blank
    /// otherwise. Kept in every mode so the tree's height does not move; the keys that shared it
    /// are the shell's hint row (MOD-67 M6 T2, [`Tab::key_hint`]).
    fn filter_row(&self, theme: &Theme) -> Line<'static> {
        let Mode::Filter { field } = &self.mode else {
            return Line::default();
        };
        let mut spans = vec![Span::styled(" /", theme.accent)];
        spans.extend(field.line(FILTER_WIDTH, true, theme).spans);
        Line::from(spans)
    }
}

impl Tab for RequirementsTab {
    fn id(&self) -> TabId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Requirements"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::Requirements(scope.clone())]
    }

    /// Everything read belongs to the workspace that was left, and so does an open form and the
    /// write in flight.
    fn on_scope_change(&mut self, _scope: &Scope) {
        *self = Self::default();
    }

    /// The current mode's stack (MOD-67 M4 D4).
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    /// The tree's own stack, whatever form or filter is open (MOD-115 review): what the about
    /// panel labels the tab's actions through.
    fn browse_stack(&self) -> Stack<'static> {
        views::REQUIREMENTS_BROWSE
    }

    /// Each mode resolves through its stack (MOD-67 M4 D4, D6). A form or the filter captures
    /// every key but the chords its stack passes, so digits, `q` and `Tab` are text or field moves
    /// there; `form.save` (`Ctrl+S`) saves a form.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Filter { .. } => self.on_filter_key(key, ctx),
            Mode::NewArea(_) | Mode::Requirement(_) | Mode::Withdraw(_) => {
                self.on_form_key(key, ctx)
            }
        }
    }

    /// MOD-22 review M-1: a bracketed paste into the filter — which then filters by it, as typing
    /// it would — or into the open form's focused field. While a write is in flight the form
    /// swallows it, as it swallows keys.
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::Browse => return Handled::Pass,
            Mode::Filter { field } => {
                field.on_paste(text);
                field
                    .text()
                    .unwrap_or_default()
                    .clone_into(&mut self.filter);
            }
            _ if self.busy.is_some() => {}
            Mode::NewArea(form) => form.on_paste(text),
            Mode::Requirement(form) => form.on_paste(text),
            Mode::Withdraw(form) => form.on_paste(text),
        }
        Handled::Consumed
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Requirements(snapshot) => {
                if !snapshot.is_for(ctx.scope) {
                    return;
                }
                // A read never lands a write (MOD-59 D4): only the write's own reply does.
                self.snapshot = Some((**snapshot).clone());
                self.recovered();
                self.after_read(ctx);
            }
            StoreReply::RequirementWritten { snapshot, outcome } => match snapshot {
                Ok(snapshot) => {
                    if !snapshot.is_for(ctx.scope) {
                        return;
                    }
                    self.snapshot = Some((**snapshot).clone());
                    self.recovered();
                    if !self.land(outcome, None, ctx) {
                        // Not this tab's write in flight: a read like any other (H-10).
                        self.after_read(ctx);
                    }
                }
                // No snapshot to check the scope by: only the write in flight owns this reply,
                // and a scope change has already forgotten that (`on_scope_change`).
                Err(why) if self.busy == Some(outcome.request_name()) => {
                    if self.snapshot.is_none() {
                        self.unavailable = Some(why.clone());
                    }
                    self.land(outcome, Some(why), ctx);
                }
                Err(_) => {}
            },
            StoreReply::RequirementsStale(snapshot) => {
                if !snapshot.is_for(ctx.scope) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.recovered();
                self.stale(ctx.keys());
                let before = self.selected;
                self.reselect(ctx);
                // The head moved elsewhere, so a detail still on screen for the same row is the
                // old version's: re-read it, as `land` does (MOD-39 review).
                if self.selected == before
                    && let Some(Row::Requirement(id)) = self.selected
                {
                    ctx.request(StoreRequest::RequirementDetail(id));
                }
            }
            StoreReply::RequirementDetail(detail) => {
                if self.selected == Some(Row::Requirement(detail.requirement.id)) {
                    self.detail = Some((**detail).clone());
                    self.detail_error = None;
                }
            }
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                // D251: a refused read must not leave a jump armed for a later one.
                self.pending_reveal = None;
                // A snapshot already held stays drawn, and the refusal is the notice.
                if self.snapshot.is_some() {
                    self.notice = Some(Notice::Error(message.clone()));
                }
                self.unavailable = Some(message.clone());
            }
            StoreReply::Failed { request, message } if *request == DETAIL_NAME => {
                self.detail_error = Some(message.clone());
            }
            // Only the write in flight: a refusal of one `on_scope_change` dropped must not free
            // the next (the Reqs pane's guard, MOD-39 review).
            StoreReply::Failed { request, message }
                if is_tab_write(request) && self.busy == Some(*request) =>
            {
                // D5: a write that applied answers `RequirementWritten`, even when only its
                // re-read failed, so the form keeps its text. An area, amend or withdraw that
                // failed wrote nothing, or its retry is refused. A mint's `Failed` may be a COMMIT
                // whose answer was lost, and its retry mints again, so its notice hedges (MOD-59
                // review M1) and the tree is re-read for the user to check (re-review L1). That
                // read is a plain `Requirements`, which lands nothing and closes no form (D4). A
                // mint the worker refused before its insert is a refusal like the others (L2).
                self.busy = None;
                if *request == MINT_NAME && !self.mint_refused(message, ctx) {
                    let save = ctx.keys().label(views::REQUIREMENTS_FORM, Act::FormSave);
                    self.notice = Some(Notice::Error(mint_failed(message, save.as_deref())));
                    ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
                } else {
                    self.notice = Some(Notice::Error(message.clone()));
                }
            }
            _ => {}
        }
    }

    fn key_hint(&self, _width: u16, ctx: &Ctx<'_>) -> Option<Line<'static>> {
        self.keys_line(ctx.keys(), ctx.theme)
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let (notice, style) = match &self.notice {
            Some(Notice::Info(text)) => (text.as_str(), ctx.theme.dim),
            Some(Notice::Error(text)) => (text.as_str(), ctx.theme.error),
            None => ("", ctx.theme.dim),
        };
        let mut notice = wrapped(notice, usize::from(area.width.saturating_sub(1)).max(1));
        notice.truncate(NOTICE_LINES);
        let notice_height = u16::try_from(notice.len().max(1)).unwrap_or(1);
        let [content, notice_row, hint_row] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(notice_height),
            Constraint::Length(1),
        ])
        .areas(area);
        // MOD-115 M2 B5: the panes the shell's layout gives the rows above the notice, claimed
        // for the focus borders, the hints and the mouse; today's 45/55 by default.
        let areas = ctx.split(content, REQUIREMENTS_LAYOUT);
        if let Some(tree) = areas.list {
            self.render_tree(frame, tree, ctx);
        }
        if !areas.detail.is_empty() {
            self.render_detail(frame, areas.detail, ctx);
        }
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(format!(" {line}"), style))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
        frame.render_widget(Paragraph::new(self.filter_row(ctx.theme)), hint_row);
    }

    /// MOD-115 M2 D2: the tree and the detail, today's 45/55 by default.
    fn panes(&self) -> Panes {
        Panes::ListDetail(REQUIREMENTS_LAYOUT)
    }

    /// MOD-115 M2 §8: the row under the cursor; the tab has no sub-tabs.
    fn location(&self) -> TabLoc {
        let cursor = self.selected.map(|row| match row {
            Row::Requirement(id) => {
                let key = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.requirement(id))
                    .map_or("", |row| row.key.as_str());
                format!("{REQUIREMENT_CURSOR}:{id}:{key}")
            }
            Row::Area(id) => format!("{AREA_CURSOR}:{id}"),
            Row::Project(id) => format!("{PROJECT_CURSOR}:{id}"),
        });
        TabLoc {
            cursor,
            ..TabLoc::default()
        }
    }

    /// MOD-115 M2 §8: back to a [`location`](Tab::location)'s row: a requirement through the
    /// reveal path (unhidden and its detail read, or read again when the tree no longer holds
    /// it), a header the tree holds made visible. The row already under the cursor reads
    /// nothing; an open form keeps its text, so nothing moves under it.
    fn restore(&mut self, loc: &TabLoc, ctx: &mut Ctx<'_>) -> bool {
        if !matches!(self.mode, Mode::Browse | Mode::Filter { .. }) {
            return false;
        }
        let Some((row, key)) = loc.cursor.as_deref().and_then(parse_cursor) else {
            return false;
        };
        if self.selected == Some(row) && self.rows(ctx).contains(&row) {
            return true;
        }
        match row {
            Row::Requirement(id) => self.reveal(&RevealTarget::Requirement { id, key }, ctx),
            Row::Area(_) | Row::Project(_) => {
                let held = self.snapshot.as_ref().is_some_and(|snapshot| {
                    tree::rows(snapshot, ctx.projects, &[], "").contains(&row)
                });
                if held {
                    self.mode = Mode::Browse;
                    self.put_cursor(row, ctx);
                }
                held
            }
        }
    }

    fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
        let RevealTarget::Requirement { id, key } = target else {
            return false;
        };
        match self.mode {
            Mode::Browse => {}
            // The typed filter is kept as applied; `unhide` drops it if it hides the row.
            Mode::Filter { .. } => self.mode = Mode::Browse,
            // An open form keeps its text: the reveal says why it did not move (D252).
            Mode::NewArea(_) | Mode::Requirement(_) | Mode::Withdraw(_) => {
                self.notice = Some(Notice::Error(CLOSE_THE_FIELD_FIRST.to_owned()));
                return true;
            }
        }
        if self.holds(*id) {
            self.pending_reveal = None;
            self.select_row(Row::Requirement(*id), ctx);
        } else {
            // Not read yet, or minted since: re-read and decide on arrival (D251).
            self.pending_reveal = Some((*id, key.clone()));
            ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{HINT_WRITES, Mode, Notice, REQUIREMENTS_LAYOUT, RequirementsTab, Row, SAVING};
    use crate::app::{Action, Ctx, Emit, Handled, TopBarState};
    use crate::keys::{Keys, views};
    use crate::requirements::{
        self, READ_NAME, RequirementWrite, RequirementsSnapshot, not_the_maintainer,
    };
    use crate::store_worker::{Origin, StoreReply, StoreRequest};
    use crate::ui::TextField;
    use crate::ui::Theme;
    use crate::ui::shell::{ListSize, PaneId, Panes, TabLoc};
    use crate::ui::tabs::registry::Tab as _;
    use core::cell::Cell;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, RequirementId, RequirementState, Scope};
    use htui_core::store::{MemStore, StoreError, requirement_withdrawn};
    use htui_store::{Backend, DATABASE_UNREACHABLE};
    use ratatui::layout::{Constraint, Layout, Rect};
    use ratatui::style::Style;

    /// The demo's Platform snapshot, its projects (`htui`, `agy`) and its scope.
    pub(in crate::ui::tabs::requirements) async fn platform()
    -> (RequirementsSnapshot, Vec<ProjectRef>, Scope) {
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
        let snapshot = requirements::snapshot(&Backend::memory(store), &scope)
            .await
            .expect("the demo's requirements");
        (snapshot, projects, scope)
    }

    /// A tab on `snapshot` with R-ENT-1 selected.
    fn tab_on(snapshot: RequirementsSnapshot) -> RequirementsTab {
        RequirementsTab {
            snapshot: Some(snapshot),
            selected: Some(Row::Requirement(ids::REQ_ENT_1)),
            ..RequirementsTab::default()
        }
    }

    /// Everything a `Ctx` borrows.
    struct Bench {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keys: Keys,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        fn new(scope: Scope, projects: Vec<ProjectRef>) -> Self {
            Self {
                scope,
                projects,
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
                Origin::Tab(RequirementsTab::ID),
                &self.emit,
            )
        }

        fn key(&self, tab: &mut RequirementsTab, code: KeyCode) -> Handled {
            tab.on_key(KeyEvent::from(code), &mut self.ctx())
        }

        fn reply(&self, tab: &mut RequirementsTab, reply: &StoreReply) {
            tab.on_reply(reply, &mut self.ctx());
        }

        fn typed(&self, tab: &mut RequirementsTab, text: &str) {
            for c in text.chars() {
                self.key(tab, KeyCode::Char(c));
            }
        }
    }

    fn requests(actions: &[Action]) -> Vec<&StoreRequest> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    fn errors(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Error(message) => Some(message.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Blueprint F-13: typing the filter reads nothing and leaves the cursor; `Enter` re-selects
    /// onto a visible row with one read.
    #[tokio::test]
    async fn a_keystroke_in_the_filter_sends_no_request() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        bench.key(&mut tab, KeyCode::Char('/'));
        bench.typed(&mut tab, "sto");
        assert_eq!(tab.filter, "sto");
        assert!(bench.emit.take().is_empty(), "no keystroke sent anything");
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_ENT_1)));

        bench.key(&mut tab, KeyCode::Enter);
        let actions = bench.emit.take();
        assert!(
            matches!(
                requests(&actions).as_slice(),
                [StoreRequest::RequirementDetail(id)] if *id == ids::REQ_STO_1
            ),
            "{actions:?}"
        );
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_STO_1)));
        assert!(matches!(tab.mode, Mode::Browse));
    }

    #[tokio::test]
    async fn write_keys_are_refused_offline_without_a_request() {
        let (mut snapshot, projects, scope) = platform().await;
        snapshot.writable = false;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        for code in ['a', 'n', 'e', 'W'] {
            assert_eq!(bench.key(&mut tab, KeyCode::Char(code)), Handled::Consumed);
            let actions = bench.emit.take();
            assert!(requests(&actions).is_empty(), "`{code}` sent {actions:?}");
            assert_eq!(errors(&actions), vec![DATABASE_UNREACHABLE], "`{code}`");
            assert!(matches!(tab.mode, Mode::Browse), "`{code}` opened no form");
        }
    }

    #[tokio::test]
    async fn a_non_maintainer_write_key_says_so_and_sends_nothing() {
        let (mut snapshot, projects, scope) = platform().await;
        for entry in &mut snapshot.projects {
            entry.maintainer = entry.project_id != ids::PROJECT_HTUI;
        }
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        for code in ['a', 'n', 'e', 'W'] {
            bench.key(&mut tab, KeyCode::Char(code));
            let actions = bench.emit.take();
            assert!(requests(&actions).is_empty(), "`{code}` sent {actions:?}");
            let expected = not_the_maintainer("htui");
            assert_eq!(errors(&actions), vec![expected.as_str()], "`{code}`");
            assert!(matches!(tab.mode, Mode::Browse), "`{code}` opened no form");
        }
        assert!(!tab.writes_allowed(), "the hint dims the write words");
    }

    /// MOD-67 M4 D4: each mode resolves through its own stack. A form captures `q`, digits and
    /// `Tab`, and `Esc` brings Browse's stack back with nothing sent.
    #[tokio::test]
    async fn key_stack_follows_the_mode() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        assert_eq!(tab.key_stack(), Some(views::REQUIREMENTS_BROWSE));
        assert_eq!(
            bench.key(&mut tab, KeyCode::Char('q')),
            Handled::Pass,
            "`q` is the shell's in Browse"
        );
        assert_eq!(bench.key(&mut tab, KeyCode::Char('3')), Handled::Pass);

        bench.key(&mut tab, KeyCode::Char('/'));
        assert_eq!(tab.key_stack(), Some(views::CAPTURE), "the filter is open");
        bench.key(&mut tab, KeyCode::Esc);
        assert_eq!(tab.key_stack(), Some(views::REQUIREMENTS_BROWSE));

        bench.key(&mut tab, KeyCode::Char('n'));
        assert_eq!(
            tab.key_stack(),
            Some(views::REQUIREMENTS_FORM),
            "the mint form is open"
        );
        for code in [KeyCode::Char('q'), KeyCode::Char('3'), KeyCode::Tab] {
            assert_eq!(
                bench.key(&mut tab, code),
                Handled::Consumed,
                "{code:?} is the form's"
            );
        }
        bench.key(&mut tab, KeyCode::Esc);
        assert_eq!(
            tab.key_stack(),
            Some(views::REQUIREMENTS_BROWSE),
            "Esc closes the form"
        );

        bench.key(&mut tab, KeyCode::Char('W'));
        assert_eq!(
            tab.key_stack(),
            Some(views::REQUIREMENTS_WITHDRAW),
            "the withdraw form is open"
        );
        bench.key(&mut tab, KeyCode::Esc);
        assert_eq!(tab.key_stack(), Some(views::REQUIREMENTS_BROWSE));
        assert!(bench.emit.take().is_empty(), "and nothing was sent");
    }

    /// The style the Browse hint draws the four write words in.
    fn write_words_style(tab: &RequirementsTab, theme: &Theme) -> Style {
        let writes = Keys::compiled().hint(views::REQUIREMENTS_BROWSE, HINT_WRITES);
        tab.keys_line(Keys::compiled(), theme)
            .expect("the defaults bind the keys")
            .spans
            .iter()
            .find(|span| span.content == writes)
            .expect("the hint names the write keys")
            .style
    }

    /// PRD D1 "mirrored in the view as greyed keys": the write words are dim offline and on a
    /// project this user does not own, and in the base style where a write would be sent.
    #[tokio::test]
    async fn the_hint_dims_the_write_words_where_a_write_is_refused() {
        let (snapshot, _, _) = platform().await;
        let theme = Theme::default();

        let tab = tab_on(snapshot.clone());
        assert_eq!(
            write_words_style(&tab, &theme),
            theme.base,
            "the maintainer"
        );

        let mut offline = snapshot.clone();
        offline.writable = false;
        assert_eq!(
            write_words_style(&tab_on(offline), &theme),
            theme.dim,
            "offline"
        );

        let mut stranger = snapshot;
        for entry in &mut stranger.projects {
            entry.maintainer = false;
        }
        assert_eq!(
            write_words_style(&tab_on(stranger), &theme),
            theme.dim,
            "not the maintainer"
        );
    }

    /// A refused re-read over a snapshot keeps the tree and says why in the notice; the next good
    /// read takes the notice away.
    #[tokio::test]
    async fn a_refused_reread_keeps_the_tree() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        let refused = StoreReply::Failed {
            request: READ_NAME,
            message: DATABASE_UNREACHABLE.to_owned(),
        };
        bench.reply(&mut tab, &refused);
        assert!(tab.snapshot.is_some(), "the tree stays");
        assert_eq!(
            tab.notice,
            Some(Notice::Error(DATABASE_UNREACHABLE.to_owned()))
        );

        bench.reply(&mut tab, &StoreReply::Requirements(Box::new(snapshot)));
        assert_eq!(tab.notice, None, "a good read clears it");
        assert_eq!(tab.unavailable, None);
    }

    /// MOD-59 D4: a read showing another session's amend (the version moved, the text is theirs)
    /// is not this amend landing; no plain `Requirements` is. The form stays, and the stale answer
    /// that follows moves its token.
    #[tokio::test]
    async fn another_sessions_amend_does_not_land_this_one() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('e'));
        bench.key(&mut tab, KeyCode::End);
        bench.typed(&mut tab, " Mine.");
        for _ in 0..3 {
            bench.key(&mut tab, KeyCode::Tab);
        }
        bench.typed(&mut tab, "ANA-2");
        bench.key(&mut tab, KeyCode::Enter);
        assert!(tab.busy.is_some(), "the amend went out");
        let sent = bench.emit.take();
        assert!(
            matches!(
                requests(&sent).as_slice(),
                [StoreRequest::AmendRequirement { .. }]
            ),
            "{sent:?}"
        );

        let mut theirs = snapshot;
        for entry in &mut theirs.projects {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    row.version = 3;
                    "Theirs.".clone_into(&mut row.body);
                }
            }
        }
        bench.reply(
            &mut tab,
            &StoreReply::Requirements(Box::new(theirs.clone())),
        );
        assert!(tab.busy.is_some(), "still in flight");
        assert!(matches!(tab.mode, Mode::Requirement(_)), "the form stays");

        bench.reply(&mut tab, &StoreReply::RequirementsStale(Box::new(theirs)));
        assert!(tab.busy.is_none());
        assert!(matches!(tab.mode, Mode::Requirement(_)), "with its text");
        assert!(
            matches!(&tab.notice, Some(Notice::Error(text)) if text.contains("now v3")),
            "{:?}",
            tab.notice
        );
    }

    /// A stale answer whose head was withdrawn elsewhere offers no retry: it says the requirement
    /// is withdrawn, and a withdraw form closes.
    #[tokio::test]
    async fn a_stale_answer_over_a_withdrawn_head_says_so() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('W'));
        bench.typed(&mut tab, "ANA-2");
        bench.key(&mut tab, KeyCode::Enter);
        bench.typed(&mut tab, "R-ENT-1");
        bench.key(&mut tab, KeyCode::Enter);
        assert!(tab.busy.is_some(), "the withdraw went out");
        let sent = bench.emit.take();
        assert!(
            matches!(
                requests(&sent).as_slice(),
                [StoreRequest::WithdrawRequirement { .. }]
            ),
            "{sent:?}"
        );

        let mut withdrawn = snapshot;
        for entry in &mut withdrawn.projects {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    row.version = 3;
                    row.state = RequirementState::Withdrawn;
                }
            }
        }
        bench.reply(
            &mut tab,
            &StoreReply::RequirementsStale(Box::new(withdrawn)),
        );
        assert!(tab.busy.is_none());
        assert!(matches!(tab.mode, Mode::Browse), "nothing left to retry");
        assert_eq!(
            tab.notice,
            Some(Notice::Error(requirement_withdrawn("R-ENT-1")))
        );
    }

    /// `Ctrl+S`: a form's save.
    fn save(bench: &Bench, tab: &mut RequirementsTab) {
        tab.on_key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &mut bench.ctx(),
        );
    }

    /// R-ENT-1 at `version` in `state`, everything else as the demo has it.
    fn with_ent_1(
        mut snapshot: RequirementsSnapshot,
        version: i32,
        state: RequirementState,
    ) -> RequirementsSnapshot {
        for entry in &mut snapshot.projects {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    row.version = version;
                    row.state = state;
                }
            }
        }
        snapshot
    }

    /// MOD-39 review #1: a stale answer moves the head, so the detail still on screen for the
    /// same row is re-read rather than left showing the old version.
    #[tokio::test]
    async fn a_stale_answer_rereads_the_detail_on_screen() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('W'));
        bench.typed(&mut tab, "ANA-2");
        bench.key(&mut tab, KeyCode::Enter);
        bench.typed(&mut tab, "R-ENT-1");
        bench.key(&mut tab, KeyCode::Enter);
        let _ = bench.emit.take();

        let withdrawn = with_ent_1(snapshot, 3, RequirementState::Withdrawn);
        bench.reply(
            &mut tab,
            &StoreReply::RequirementsStale(Box::new(withdrawn)),
        );

        let sent = bench.emit.take();
        assert!(
            requests(&sent)
                .iter()
                .any(|request| matches!(request, StoreRequest::RequirementDetail(id) if *id == ids::REQ_ENT_1)),
            "{sent:?}"
        );
    }

    /// MOD-115 M2 D7 (T5 repair): the area form has no priority and no choice field, so its
    /// stack has neither layer: its `?` box lists only keys that act there, and `m`, `l` and
    /// `Space` are text in its code field.
    #[tokio::test]
    async fn the_area_form_box_lists_only_keys_that_act() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        bench.key(&mut tab, KeyCode::Char('a'));
        let stack = tab.key_stack().expect("the area form is open");
        assert_eq!(stack, views::AREA_FORM);
        let lines: Vec<String> = bench
            .keys
            .help_lines(stack, |_| false)
            .iter()
            .map(|line| line.text())
            .collect();
        assert_eq!(
            lines,
            [
                "Requirements: Tab/Down next field · Shift+Tab/Up previous field",
                // MOD-115 M2 T6: the field's editing keys are wired, so the box lists them.
                "Text field: Enter submit · Esc cancel · Left cursor left · Right cursor right \
                 · Home line start · End line end · Backspace delete back · Del delete forward \
                 · Up line up · Down line down · PgUp page up · PgDn page down",
                "Form: Ctrl+s save",
                "Global: Ctrl+c quit",
            ]
        );
        for code in [KeyCode::Char('m'), KeyCode::Char('l'), KeyCode::Char(' ')] {
            assert_eq!(bench.key(&mut tab, code), Handled::Consumed, "{code:?}");
        }
        let Mode::NewArea(form) = &tab.mode else {
            panic!("the area form is still open");
        };
        assert_eq!(form.code.text().unwrap_or_default(), "ml ");
    }

    /// MOD-39 review #3: a refusal of a write that is not the one in flight (one a scope change
    /// dropped) frees nothing; the refusal of the write in flight does.
    #[tokio::test]
    async fn only_the_write_in_flight_is_freed_by_its_refusal() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        bench.key(&mut tab, KeyCode::Char('a'));
        bench.typed(&mut tab, "API");
        bench.key(&mut tab, KeyCode::Tab);
        bench.typed(&mut tab, "Interface");
        save(&bench, &mut tab);
        let sent = bench.emit.take();
        let sent = requests(&sent);
        let [request @ StoreRequest::CreateRequirementArea { .. }] = sent.as_slice() else {
            panic!("the area create went out: {sent:?}")
        };
        let area = request.name();

        bench.reply(
            &mut tab,
            &StoreReply::Failed {
                request: requirements::REQUEST_NAMES
                    .iter()
                    .copied()
                    .find(|name| *name != area && requirements::is_tab_write(name))
                    .expect("another tab write"),
                message: "an older refusal".to_owned(),
            },
        );
        assert_eq!(tab.busy, Some(area), "still in flight");

        bench.reply(
            &mut tab,
            &StoreReply::Failed {
                request: area,
                message: "refused".to_owned(),
            },
        );
        assert_eq!(tab.busy, None);
        assert_eq!(tab.notice, Some(Notice::Error("refused".to_owned())));
        assert!(
            matches!(tab.mode, Mode::NewArea(_)),
            "the form keeps its text"
        );
    }

    /// Opens the mint form under ENT, types `body` and saves: the mint in flight.
    fn mint(bench: &Bench, tab: &mut RequirementsTab, body: &str) -> &'static str {
        bench.key(tab, KeyCode::Char('n'));
        bench.typed(tab, body);
        save(bench, tab);
        let sent = bench.emit.take();
        let sent = requests(&sent);
        let [request @ StoreRequest::MintRequirement { .. }] = sent.as_slice() else {
            panic!("the mint went out: {sent:?}")
        };
        request.name()
    }

    /// HANDOFF (a), MOD-59 D4: another session amends again between this amend and its re-read,
    /// so the snapshot the reply carries holds their v4, not the text sent. The amend still lands
    /// on its own reply, at the version the store says it wrote, and that row's detail is re-read.
    #[tokio::test]
    async fn an_amend_lands_on_its_own_reply_although_another_session_amended_again() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('e'));
        bench.key(&mut tab, KeyCode::End);
        bench.typed(&mut tab, " Mine.");
        for _ in 0..3 {
            bench.key(&mut tab, KeyCode::Tab);
        }
        bench.typed(&mut tab, "ANA-2");
        bench.key(&mut tab, KeyCode::Enter);
        let sent = bench.emit.take();
        let [request @ StoreRequest::AmendRequirement { .. }] = requests(&sent)[..] else {
            panic!("the amend went out: {sent:?}")
        };
        assert_eq!(tab.busy, Some(request.name()));

        let mut theirs = with_ent_1(snapshot, 4, RequirementState::Active);
        for entry in &mut theirs.projects {
            for row in &mut entry.requirements {
                if row.id == ids::REQ_ENT_1 {
                    "Theirs.".clone_into(&mut row.body);
                }
            }
        }
        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Ok(Box::new(theirs)),
                outcome: RequirementWrite::Amended {
                    id: ids::REQ_ENT_1,
                    key: "R-ENT-1".to_owned(),
                    version: 3,
                },
            },
        );

        assert_eq!(tab.busy, None);
        assert!(matches!(tab.mode, Mode::Browse), "the form closed");
        assert_eq!(
            tab.notice,
            Some(Notice::Info("amended R-ENT-1 to v3".to_owned()))
        );
        let sent = bench.emit.take();
        assert!(
            requests(&sent).iter().any(|request| matches!(
                request,
                StoreRequest::RequirementDetail(id) if *id == ids::REQ_ENT_1
            )),
            "{sent:?}"
        );
    }

    /// MOD-59 D4: a read that already shows the mint is not its answer. It refreshes the tree and
    /// leaves the form and `busy` alone; the mint's own `RequirementWritten` lands it.
    #[tokio::test]
    async fn a_read_that_shows_the_mint_does_not_land_it() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        let name = mint(&bench, &mut tab, "Twice is too many.");

        let mut landed = snapshot;
        let entry = landed
            .projects
            .iter_mut()
            .find(|entry| entry.project_id == ids::PROJECT_HTUI)
            .expect("htui");
        let mut row = entry
            .requirements
            .iter()
            .find(|row| row.id == ids::REQ_ENT_1)
            .expect("R-ENT-1")
            .clone();
        row.id = RequirementId::new();
        row.number = 3;
        "R-ENT-3".clone_into(&mut row.key);
        "Twice is too many.".clone_into(&mut row.body);
        row.version = 1;
        let id = row.id;
        entry.requirements.push(row);

        bench.reply(
            &mut tab,
            &StoreReply::Requirements(Box::new(landed.clone())),
        );
        assert_eq!(tab.busy, Some(name), "a read never lands the mint");
        assert!(matches!(tab.mode, Mode::Requirement(_)), "the form stays");

        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Ok(Box::new(landed)),
                outcome: RequirementWrite::Minted {
                    id,
                    key: "R-ENT-3".to_owned(),
                },
            },
        );
        assert_eq!(tab.busy, None);
        assert!(matches!(tab.mode, Mode::Browse), "the form closed");
        assert_eq!(tab.notice, Some(Notice::Info("minted R-ENT-3".to_owned())));
        assert_eq!(tab.selected, Some(Row::Requirement(id)));
    }

    /// Opens the amend form on R-ENT-1, appends to its body, names ANA-2 and saves: the amend in
    /// flight.
    fn amend(bench: &Bench, tab: &mut RequirementsTab) -> &'static str {
        bench.key(tab, KeyCode::Char('e'));
        bench.key(tab, KeyCode::End);
        bench.typed(tab, " Mine.");
        for _ in 0..3 {
            bench.key(tab, KeyCode::Tab);
        }
        bench.typed(tab, "ANA-2");
        bench.key(tab, KeyCode::Enter);
        let sent = bench.emit.take();
        let [request @ StoreRequest::AmendRequirement { .. }] = requests(&sent)[..] else {
            panic!("the amend went out: {sent:?}")
        };
        request.name()
    }

    /// MOD-59 H-10 (review M2): a `RequirementWritten` for another write than the one in flight
    /// lands nothing. With a snapshot it is a read like any other, so a pending reveal is decided
    /// by it; with only a failed re-read it has no scope to check, so it changes nothing.
    #[tokio::test]
    async fn a_requirement_written_for_another_write_is_a_read_or_nothing() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        let name = amend(&bench, &mut tab);
        tab.pending_reveal = Some((ids::REQ_STO_1, "R-STO-1".to_owned()));
        let minted = RequirementWrite::Minted {
            id: RequirementId::new(),
            key: "R-ENT-3".to_owned(),
        };

        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Ok(Box::new(snapshot)),
                outcome: minted.clone(),
            },
        );
        assert_eq!(tab.busy, Some(name), "not the amend's answer");
        assert!(matches!(tab.mode, Mode::Requirement(_)), "the form stays");
        assert_eq!(tab.notice, Some(Notice::Info(SAVING.to_owned())));
        assert_eq!(tab.pending_reveal, None, "the read decided the reveal");
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_STO_1)));
        let _ = bench.emit.take();

        tab.pending_reveal = Some((ids::REQ_STO_1, "R-STO-1".to_owned()));
        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Err("store unreachable: gone".to_owned()),
                outcome: minted,
            },
        );
        assert_eq!(tab.busy, Some(name), "still in flight");
        assert!(matches!(tab.mode, Mode::Requirement(_)), "the form stays");
        assert_eq!(tab.unavailable, None);
        assert_eq!(tab.notice, Some(Notice::Info(SAVING.to_owned())));
        assert!(
            tab.pending_reveal.is_some(),
            "nothing was read to decide it"
        );
        assert!(requests(&bench.emit.take()).is_empty(), "nothing was sent");
    }

    /// MOD-59 review L4: an amend whose re-read failed lands on the row the tree holds, but its
    /// detail is not read: the store has just failed a read, and the detail's would most likely
    /// fail too. The pane says why instead of the version before the amend; `r` reads both again.
    #[tokio::test]
    async fn an_amend_whose_reread_failed_reads_no_detail() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        amend(&bench, &mut tab);

        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Err("store unreachable: gone".to_owned()),
                outcome: RequirementWrite::Amended {
                    id: ids::REQ_ENT_1,
                    key: "R-ENT-1".to_owned(),
                    version: 3,
                },
            },
        );

        assert_eq!(tab.busy, None);
        assert!(matches!(tab.mode, Mode::Browse), "the form closed");
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_ENT_1)));
        let sent = bench.emit.take();
        assert!(requests(&sent).is_empty(), "no detail read: {sent:?}");
        assert_eq!(tab.detail, None, "not the version before the amend");
        assert_eq!(tab.detail_error.as_deref(), Some("store unreachable: gone"));
    }

    /// MOD-59 D5: a mint whose re-read failed has landed all the same. The form closes, the notice
    /// says it landed and that the tree is the one held, and nothing more is read: no re-check of
    /// the mint, and no detail of a row the tree does not hold yet.
    #[tokio::test]
    async fn a_mint_whose_reread_failed_lands_and_reads_nothing_more() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        mint(&bench, &mut tab, "Twice is too many.");
        let before = tab.snapshot.clone();

        bench.reply(
            &mut tab,
            &StoreReply::RequirementWritten {
                snapshot: Err("store unreachable: gone".to_owned()),
                outcome: RequirementWrite::Minted {
                    id: RequirementId::new(),
                    key: "R-ENT-3".to_owned(),
                },
            },
        );

        assert_eq!(tab.busy, None);
        assert!(matches!(tab.mode, Mode::Browse), "the form closed");
        assert!(
            matches!(&tab.notice, Some(Notice::Error(text))
                if text.starts_with("minted R-ENT-3 \u{2014} the re-read failed")
                    && text.ends_with("store unreachable: gone")),
            "{:?}",
            tab.notice
        );
        let sent = bench.emit.take();
        assert!(requests(&sent).is_empty(), "nothing is read: {sent:?}");
        assert_eq!(tab.snapshot, before, "the held tree stays drawn");
        assert_eq!(tab.unavailable, None, "a tree was held");
    }

    /// The open form's body, or `None` when no requirement form is open.
    fn draft(tab: &RequirementsTab) -> Option<&str> {
        match &tab.mode {
            Mode::Requirement(form) => Some(form.body.text()),
            _ => None,
        }
    }

    /// MOD-59 review M1, re-review L1: a mint's `Failed` may not be a refusal. A mint whose
    /// COMMIT landed but whose answer was lost comes back as the same `Failed`, and a retry would
    /// mint it twice, so the notice hedges. `Esc` would drop the draft, so the tab re-reads the
    /// tree itself (drawn beside the form) and the notice points there, not at `Esc`. That plain
    /// read lands nothing: the form stays open with its text.
    #[tokio::test]
    async fn a_failed_mint_rereads_the_tree_and_keeps_the_draft() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope.clone(), projects);
        let mut tab = tab_on(snapshot.clone());
        let name = mint(&bench, &mut tab, "Never written.");

        bench.reply(
            &mut tab,
            &StoreReply::Failed {
                request: name,
                message: "store unreachable: reset".to_owned(),
            },
        );

        assert_eq!(tab.busy, None);
        let hedge = tab.notice.clone();
        assert!(
            matches!(&hedge, Some(Notice::Error(text))
                if text.starts_with("store unreachable: reset \u{2014} it may have been written")
                    && text.contains("Ctrl+s")
                    && !text.contains("Esc")),
            "{hedge:?}"
        );
        assert_eq!(
            draft(&tab),
            Some("Never written."),
            "the form keeps its text"
        );
        let sent = bench.emit.take();
        assert!(
            matches!(
                requests(&sent).as_slice(),
                [StoreRequest::Requirements(read)] if *read == scope
            ),
            "exactly one tree read: {sent:?}"
        );

        bench.reply(&mut tab, &StoreReply::Requirements(Box::new(snapshot)));
        assert_eq!(draft(&tab), Some("Never written."), "a read closes no form");
        assert_eq!(tab.busy, None);
        assert_eq!(tab.notice, hedge, "the hedge stays up");
    }

    /// MOD-59 re-review L2: a mint refused before its insert (offline, a blank body, an area that
    /// is not the project's, not the maintainer) wrote nothing, so its notice is the plain refusal:
    /// no hedge, and no re-read.
    #[tokio::test]
    async fn a_mint_refused_before_its_insert_is_a_plain_refusal() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let refusals = [
            StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()),
            StoreError::Constraint(not_the_maintainer("htui")),
            StoreError::Constraint(requirements::BLANK_BODY.to_owned()),
            StoreError::NotFound {
                entity: "requirement_area",
                id: ids::AREA_ENT.to_string(),
            },
        ];
        for refusal in refusals {
            let message = refusal.to_string();
            let mut tab = tab_on(snapshot.clone());
            let name = mint(&bench, &mut tab, "Never written.");

            bench.reply(
                &mut tab,
                &StoreReply::Failed {
                    request: name,
                    message: message.clone(),
                },
            );

            assert_eq!(tab.busy, None);
            assert_eq!(tab.notice, Some(Notice::Error(message.clone())));
            assert_eq!(draft(&tab), Some("Never written."), "{message}");
            let sent = bench.emit.take();
            assert!(requests(&sent).is_empty(), "{message}: {sent:?}");
        }
    }

    /// MOD-39 review #4: an amend form a stale answer kept open over a head withdrawn elsewhere
    /// refuses its save here and sends nothing.
    #[tokio::test]
    async fn an_amend_over_a_withdrawn_head_is_refused_here() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('e'));
        for _ in 0..3 {
            bench.key(&mut tab, KeyCode::Tab);
        }
        bench.typed(&mut tab, "ANA-2");
        bench.key(&mut tab, KeyCode::Enter);
        let _ = bench.emit.take();
        let withdrawn = with_ent_1(snapshot, 3, RequirementState::Withdrawn);
        bench.reply(
            &mut tab,
            &StoreReply::RequirementsStale(Box::new(withdrawn)),
        );
        assert!(
            matches!(tab.mode, Mode::Requirement(_)),
            "the amend keeps its text"
        );
        let _ = bench.emit.take();

        save(&bench, &mut tab);

        assert!(requests(&bench.emit.take()).is_empty(), "nothing was sent");
        assert_eq!(
            tab.notice,
            Some(Notice::Error(requirement_withdrawn("R-ENT-1")))
        );
    }

    /// MOD-67 M6 D4: the Requirements hedges name the save key in force through
    /// `REQUIREMENTS_FORM`, and the reload key through `REQUIREMENTS_BROWSE`; unbound, neither.
    #[test]
    fn a_rebound_save_key_is_named_by_a_requirements_hedge() {
        use super::{landed_unread, mint_failed, requirement_changed_elsewhere};
        use crate::keys::{Act, load_str};
        let keys = load_str("version = 1\n[form]\nsave = \"f2\"\n").expect("loads");
        let save = keys.label(views::REQUIREMENTS_FORM, Act::FormSave);
        assert_eq!(save.as_deref(), Some("F2"));
        assert!(
            mint_failed("boom", save.as_deref()).ends_with("look for it there before F2"),
            "{}",
            mint_failed("boom", save.as_deref())
        );
        assert!(requirement_changed_elsewhere(4, save.as_deref()).ends_with("F2 saves over it"));
        assert!(mint_failed("boom", None).ends_with("look for it there first"));
        assert!(requirement_changed_elsewhere(4, None).ends_with("your text is kept"));
        assert_eq!(
            landed_unread("saved", "gone", Some("r")),
            "saved \u{2014} the re-read failed, r reloads: gone"
        );
        assert_eq!(
            landed_unread("saved", "gone", None),
            "saved \u{2014} the re-read failed: gone"
        );
    }

    // ---- MOD-115 M2 T7: the panes, motion by focus, the location ---------------------------

    /// `code` with the tab's pane `focus` focused.
    fn key_at(bench: &Bench, tab: &mut RequirementsTab, focus: PaneId, code: KeyCode) -> Handled {
        tab.on_key(KeyEvent::from(code), &mut bench.ctx().with_focus(focus))
    }

    /// The tab drawn at 100x30 through `ctx`.
    fn draw(tab: &RequirementsTab, ctx: &Ctx<'_>) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).expect("terminal");
        terminal
            .draw(|frame| tab.render(frame, frame.area(), ctx))
            .expect("the frame draws");
    }

    /// MOD-115 M2 D2: the Requirements tab is a list + detail tab, its default today's 45/55
    /// split of the rows above the notice and the filter row, claimed cell for cell.
    #[tokio::test]
    async fn the_tree_and_the_detail_are_claimed_at_today_s_split() {
        assert_eq!(
            RequirementsTab::new().panes(),
            Panes::ListDetail(REQUIREMENTS_LAYOUT)
        );
        assert_eq!(REQUIREMENTS_LAYOUT.list, ListSize::Percent(45));
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let claim = Cell::new(None);
        draw(&tab_on(snapshot), &bench.ctx().with_pane_claim(&claim));
        let (region, areas) = claim.get().expect("the render claims its panes");
        assert_eq!(
            region,
            Rect::new(0, 0, 100, 28),
            "above the notice and filter rows"
        );
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                .areas(region);
        assert_eq!((areas.list, areas.detail), (Some(left), right));
    }

    /// MOD-115 M2 §7: with the tree focused, motion moves the cursor as before; with the
    /// detail focused it scrolls the detail and the cursor stays; with the activity pane
    /// focused nothing moves.
    #[tokio::test]
    async fn motion_follows_the_focused_pane() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        tab.pane_rows.set(10);

        for (code, offset) in [
            (KeyCode::Char('j'), 1),
            (KeyCode::Down, 2),
            (KeyCode::Char('k'), 1),
            (KeyCode::Char('G'), 9),
            (KeyCode::Home, 0),
        ] {
            assert_eq!(
                key_at(&bench, &mut tab, PaneId::Detail, code),
                Handled::Consumed
            );
            assert_eq!(tab.scroll.offset(), offset, "{code:?}");
        }
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_ENT_1)));
        assert!(bench.emit.take().is_empty(), "no detail read");

        for code in [KeyCode::Char('j'), KeyCode::Char('G')] {
            assert_eq!(
                key_at(&bench, &mut tab, PaneId::Activity, code),
                Handled::Consumed
            );
        }
        assert_eq!(tab.selected, Some(Row::Requirement(ids::REQ_ENT_1)));
        assert_eq!(tab.scroll.offset(), 0);

        key_at(&bench, &mut tab, PaneId::List, KeyCode::Char('j'));
        assert_ne!(
            tab.selected,
            Some(Row::Requirement(ids::REQ_ENT_1)),
            "the tree's"
        );
    }

    /// MOD-115 M2 §7: the fold is the tree's: `Enter` on a header folds it with the tree
    /// focused, and passes with the detail focused. A view verb acts whatever is focused.
    #[tokio::test]
    async fn the_fold_is_the_tree_s_and_a_view_verb_acts_anywhere() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        tab.selected = Some(Row::Project(ids::PROJECT_HTUI));
        assert_eq!(
            key_at(&bench, &mut tab, PaneId::Detail, KeyCode::Enter),
            Handled::Pass
        );
        assert!(tab.folded.is_empty(), "no fold from the detail");
        assert_eq!(
            key_at(&bench, &mut tab, PaneId::List, KeyCode::Enter),
            Handled::Consumed
        );
        assert_eq!(tab.folded.len(), 1, "the tree's fold");

        assert_eq!(
            key_at(&bench, &mut tab, PaneId::Detail, KeyCode::Char('/')),
            Handled::Consumed
        );
        assert!(matches!(tab.mode, Mode::Filter { .. }), "the filter opened");
    }

    /// MOD-115 M2 §8: the location is the row under the cursor; restoring it on a tab moved
    /// since puts the cursor back, unfolds what hides it and reads the requirement's detail.
    #[tokio::test]
    async fn the_location_is_the_row_and_restore_returns_to_it() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let here = tab_on(snapshot.clone()).location();
        assert_eq!(here.subtab, None);
        assert!(here.cursor.is_some());

        let mut moved = tab_on(snapshot.clone());
        bench.key(&mut moved, KeyCode::Char('j'));
        assert_ne!(moved.selected, Some(Row::Requirement(ids::REQ_ENT_1)));
        let _ = bench.emit.take();
        assert!(moved.restore(&here, &mut bench.ctx()));
        assert_eq!(moved.selected, Some(Row::Requirement(ids::REQ_ENT_1)));
        assert!(
            requests(&bench.emit.take())
                .iter()
                .any(|request| matches!(request, StoreRequest::RequirementDetail(id) if *id == ids::REQ_ENT_1)),
            "its detail is read"
        );
        assert_eq!(moved.location(), here);

        // A header is a location too, and restoring onto the row already selected reads nothing.
        let mut header = tab_on(snapshot.clone());
        header.selected = Some(Row::Project(ids::PROJECT_HTUI));
        let at = header.location();
        assert!(header.restore(&at, &mut bench.ctx()));
        assert!(bench.emit.take().is_empty(), "already there");
        let mut other = tab_on(snapshot);
        assert!(other.restore(&at, &mut bench.ctx()));
        assert_eq!(other.selected, Some(Row::Project(ids::PROJECT_HTUI)));
    }

    /// MOD-115 M2 §8: an open form keeps its text, so nothing is restored over it; a location
    /// that names no row restores nothing.
    #[tokio::test]
    async fn restore_moves_nothing_over_a_form_or_without_a_row() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let here = tab_on(snapshot.clone()).location();
        let mut tab = tab_on(snapshot.clone());
        bench.key(&mut tab, KeyCode::Char('j'));
        let at = tab.selected;
        bench.key(&mut tab, KeyCode::Char('a'));
        assert!(
            matches!(tab.mode, Mode::NewArea(_)),
            "the area form is open"
        );
        assert!(!tab.restore(&here, &mut bench.ctx()));
        assert_eq!(tab.selected, at);

        let mut tab = tab_on(snapshot);
        assert!(!tab.restore(&TabLoc::default(), &mut bench.ctx()));
    }

    /// MOD-115 M2 MD1 (T6R finding): the filter's hint names `field.submit` through the keys,
    /// as every other hint does; `Esc` stays `Esc` (MD7).
    #[tokio::test]
    async fn the_filter_hint_names_a_rebound_submit_key() {
        use crate::keys::load_str;
        let (snapshot, _, _) = platform().await;
        let mut tab = tab_on(snapshot);
        tab.mode = Mode::Filter {
            field: TextField::new(),
        };
        let theme = Theme::default();
        let text = |keys: &Keys| {
            tab.keys_line(keys, &theme)
                .map(|line| line.to_string())
                .unwrap_or_default()
        };
        assert_eq!(text(Keys::compiled()), "Enter apply \u{b7} Esc clear");
        let keys = load_str("version = 2\n[field]\nsubmit = \"ctrl-t\"\n").expect("loads");
        assert_eq!(text(&keys), "Ctrl+t apply \u{b7} Esc clear");
    }
}
