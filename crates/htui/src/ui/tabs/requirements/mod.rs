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
//! One write in flight (`busy`, the Skills tab's rule), and a write **lands by content** (blueprint
//! F-16): a `Requirements` reply closes the form only when it shows what was sent, so a read served
//! ahead of the write leaves the form and `busy` alone. A `RequirementsStale` (an amend or withdraw
//! that missed its version) keeps the form and its text, moves the token to the head and says so; the
//! retry is the user's `Ctrl+S`. Forms save on `Ctrl+S` from any field, because `TextArea` breaks
//! the line on `Enter` (blueprint F-1); a withdraw ends with the requirement's key typed back, the
//! close-out pattern.

mod detail;
mod forms;
mod tree;

use core::cell::Cell;

use htui_core::model::{
    Priority, ProjectId, RequirementArea, RequirementAreaId, RequirementId, RequirementState, Scope,
};
use htui_core::store::{invalid_area_code, requirement_withdrawn};
use htui_store::DATABASE_UNREACHABLE;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled, RevealTarget};
use crate::requirements::{
    BLANK_AREA_TITLE, BLANK_BODY, DECIDING_KEY_NEEDED, DETAIL_NAME, READ_NAME, RequirementDetail,
    RequirementText, RequirementsSnapshot, is_tab_write, not_the_maintainer,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::backlog::list::window;
use crate::ui::tabs::registry::{CLOSE_THE_FIELD_FIRST, Tab, TabId};
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use forms::{AreaForm, FormFocus, FormOutcome, FormTarget, RequirementForm, WithdrawForm};
use tree::{Fold, Row, TreeView};

/// The tree's share of the body, as a percentage.
const TREE_PERCENT: u16 = 45;

/// The detail pane's share of the body, as a percentage.
const DETAIL_PERCENT: u16 = 55;

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
/// A refused mint while the re-read checks whether it applied anyway (MOD-39 review).
const CHECKING_MINT: &str = "the save was refused; checking whether it was written\u{2026}";

/// `n` on a project row.
const SELECT_AN_AREA: &str = "select an area first";

/// `e` or `W` on a header row.
const SELECT_A_REQUIREMENT: &str = "select a requirement first";

/// The withdraw's typed-back key did not match.
const NOT_THE_REQUIREMENT_KEY: &str = "that is not the requirement's key";

/// The hint row's words before the write keys.
const HINT_MOVE: &str = "j/k move  Enter fold  / filter  ";

/// The four write keys, dimmed when the selected project refuses them.
const HINT_WRITES: &str = "a area  n new  e amend  W withdraw";

/// The hint row's words after the write keys.
const HINT_RELOAD: &str = "  r reload";

/// The hint row on the filter, after the field.
const FILTER_HINT: &str = "  Enter apply  Esc clear";

/// The filter field's width on the hint row.
const FILTER_WIDTH: u16 = 30;

/// The hint row on the area and requirement forms.
const FORM_HINT: &str = "Tab field  Ctrl+S save  Esc cancel";

/// Added to [`FORM_HINT`] on the priority field.
const PRIORITY_HINT: &str = "  m/l priority";

/// The hint row on the withdraw form.
const WITHDRAW_HINT: &str = "Enter next  Esc cancel";

/// A key while a write is in flight: one at a time (the Skills tab's rule).
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// A `RequirementsStale` over an open form: the text is kept and the token moved to the head.
/// The tab's own sentence rather than Settings' `CHANGED_ELSEWHERE`, whose retry is `Enter`
/// (blueprint F-1).
fn requirement_changed_elsewhere(head: i32) -> String {
    format!(
        "changed elsewhere since you opened it \u{2014} now v{head}; your text is kept and Ctrl+S \
         saves over it"
    )
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
    /// The tab write in flight, by `StoreRequest::name`: one at a time.
    busy: Option<&'static str>,
    /// What the write in flight carries, for the landing (blueprint F-16).
    sent: Option<Sent>,
    /// A refused mint's sentence while the re-read checks whether it landed anyway: the worker
    /// answers `Failed` when the write applied and only its re-read failed, and a retried mint
    /// would write a second requirement that can be withdrawn but never deleted (MOD-39 review).
    verifying: Option<String>,
    /// The last outcome, one line above the hint.
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

/// The write in flight and what tells its own landing from a read served ahead of it (F-16).
/// Custom `Debug`: the body's length only.
enum Sent {
    /// `CreateRequirementArea`: the project has an area with this code.
    Area {
        /// The area's project.
        project: ProjectId,
        /// The code sent.
        code: String,
    },
    /// `MintRequirement`: the area holds a requirement not in `known` with this body.
    Mint {
        /// The area's project.
        project: ProjectId,
        /// The area.
        area: RequirementAreaId,
        /// The body sent.
        body: String,
        /// The project's requirements when the mint went out.
        known: Vec<RequirementId>,
    },
    /// `AmendRequirement`: the version moved past the token and the row reads as sent. The
    /// content, not the version alone: another session's amend moves the version too, and taken
    /// for this one it would close the form over text that was never written.
    Amend {
        /// The requirement.
        id: RequirementId,
        /// The token sent.
        expected_version: i32,
        /// The body sent.
        body: String,
        /// The rationale sent.
        rationale: String,
        /// The priority sent.
        priority: Priority,
    },
    /// `WithdrawRequirement`: the requirement is withdrawn.
    Withdraw {
        /// The requirement.
        id: RequirementId,
    },
}

impl core::fmt::Debug for Sent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Area { project, code } => f
                .debug_struct("Area")
                .field("project", project)
                .field("code", code)
                .finish(),
            Self::Mint {
                project,
                area,
                body,
                known,
            } => f
                .debug_struct("Mint")
                .field("project", project)
                .field("area", area)
                .field("body_len", &body.len())
                .field("known", &known.len())
                .finish(),
            Self::Amend {
                id,
                expected_version,
                body,
                rationale,
                priority,
            } => f
                .debug_struct("Amend")
                .field("id", id)
                .field("expected_version", expected_version)
                .field("body_len", &body.len())
                .field("rationale_len", &rationale.len())
                .field("priority", priority)
                .finish(),
            Self::Withdraw { id } => f.debug_struct("Withdraw").field("id", id).finish(),
        }
    }
}

/// One line of report above the hint.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`: something the user has to act on.
    Error(String),
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// `Ctrl+S`.
fn ctrl_s(key: &KeyEvent) -> bool {
    key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL
        && matches!(key.code, KeyCode::Char('s' | 'S'))
}

impl RequirementsTab {
    /// Identity of the Requirements tab.
    pub const ID: TabId = TabId("requirements");

    /// A tab with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a form or the filter is taking every key but `CONTROL` chords.
    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
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

    /// `a`, `n`, `e` or `W` (blueprint §4.4), checked in order: a write in flight; offline; not
    /// the maintainer; a withdrawn requirement for `e`/`W`. None of these sends anything.
    fn write_key(&mut self, key: char, ctx: &Ctx<'_>) {
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
            let slug = ctx
                .projects
                .iter()
                .find(|row| row.project_id == project)
                .map_or_else(|| project.to_string(), |row| row.slug.clone());
            ctx.emit(Action::Error(not_the_maintainer(&slug)));
            return;
        }
        let mode = match (key, row) {
            ('a', _) => Mode::NewArea(AreaForm::new(project)),
            ('n', Row::Project(_)) => {
                self.notice = Some(Notice::Info(SELECT_AN_AREA.to_owned()));
                return;
            }
            ('n', Row::Area(id)) => {
                let Some(area) = snapshot.area(id) else {
                    return;
                };
                Mode::Requirement(RequirementForm::mint(project, id, area.code.clone()))
            }
            ('n', Row::Requirement(id)) => {
                let Some(requirement) = snapshot.requirement(id) else {
                    return;
                };
                Mode::Requirement(RequirementForm::mint(
                    project,
                    requirement.area_id,
                    requirement.area_code.clone(),
                ))
            }
            ('e' | 'W', Row::Requirement(id)) => {
                let Some(requirement) = snapshot.requirement(id) else {
                    return;
                };
                if requirement.state == RequirementState::Withdrawn {
                    ctx.emit(Action::Error(requirement_withdrawn(&requirement.key)));
                    return;
                }
                if key == 'e' {
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
            ('e' | 'W', _) => {
                self.notice = Some(Notice::Info(SELECT_A_REQUIREMENT.to_owned()));
                return;
            }
            _ => return,
        };
        self.notice = None;
        self.mode = mode;
    }

    /// Browse (blueprint §4.4). `CONTROL`/`ALT` chords, digits, `q`, `w`, `?` and `Tab` pass to
    /// the shell.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.step(1, ctx),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1, ctx),
            KeyCode::Char('g') | KeyCode::Home => self.jump(false, ctx),
            KeyCode::Char('G') | KeyCode::End => self.jump(true, ctx),
            KeyCode::Enter => return self.fold(),
            KeyCode::Char('J' | 'K') | KeyCode::PageDown | KeyCode::PageUp => {
                return self.scroll.on_key(key, self.pane_rows.get());
            }
            KeyCode::Char('/') => {
                self.notice = None;
                self.mode = Mode::Filter {
                    field: TextField::with_text(&self.filter),
                };
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reselect(ctx);
            }
            KeyCode::Char(c @ ('a' | 'n' | 'e' | 'W')) => self.write_key(c, ctx),
            KeyCode::Char('r') => {
                self.notice = None;
                ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
                if let Some(Row::Requirement(id)) = self.selected {
                    ctx.request(StoreRequest::RequirementDetail(id));
                }
            }
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// The filter: every key updates `filter` and sends nothing (F-13); `Enter` applies and
    /// re-selects, `Esc` clears and re-selects.
    fn on_filter_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
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
            FieldOutcome::Pass => {}
        }
        Handled::Consumed
    }

    /// A key on an open form. While a write is in flight every key is swallowed: the reply closes
    /// the form or keeps it, and a form closed now would leave it nothing to land on.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        if self.busy.is_some() {
            return Handled::Consumed;
        }
        let outcome = match &mut self.mode {
            Mode::NewArea(form) => form.on_key(key),
            Mode::Requirement(form) => form.on_key(key),
            Mode::Withdraw(form) => form.on_key(key),
            Mode::Browse | Mode::Filter { .. } => return Handled::Pass,
        };
        match outcome {
            FormOutcome::Stay => {}
            FormOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            FormOutcome::Submit => self.save(ctx),
        }
        Handled::Consumed
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
                        code: code.clone(),
                        title,
                    },
                    Sent::Area { project, code },
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
                let (request, sent) = match &form.target {
                    FormTarget::Mint { project, area, .. } => {
                        let known = self
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.project(*project))
                            .map(|entry| entry.requirements.iter().map(|row| row.id).collect())
                            .unwrap_or_default();
                        (
                            StoreRequest::MintRequirement {
                                scope,
                                project: *project,
                                area: *area,
                                body: RequirementText::new(body.clone()),
                                rationale,
                                priority,
                            },
                            Sent::Mint {
                                project: *project,
                                area: *area,
                                body,
                                known,
                            },
                        )
                    }
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
                        (
                            StoreRequest::AmendRequirement {
                                scope,
                                id: *id,
                                expected_version: *expected_version,
                                body: RequirementText::new(body.clone()),
                                rationale: rationale.clone(),
                                priority,
                                deciding,
                            },
                            Sent::Amend {
                                id: *id,
                                expected_version: *expected_version,
                                body,
                                rationale: rationale.as_str().to_owned(),
                                priority,
                            },
                        )
                    }
                };
                self.send(request, sent, ctx);
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
                let id = *id;
                let request = StoreRequest::WithdrawRequirement {
                    scope,
                    id,
                    expected_version: *expected_version,
                    deciding: deciding.clone(),
                };
                self.send(request, Sent::Withdraw { id }, ctx);
            }
            Mode::Browse | Mode::Filter { .. } => {}
        }
    }

    /// Sends one tab write and marks it in flight.
    fn send(&mut self, request: StoreRequest, sent: Sent, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.sent = Some(sent);
        self.notice = Some(Notice::Info(SAVING.to_owned()));
        ctx.request(request);
    }

    /// A `Requirements` reply while a write is in flight: whether it shows the write (F-16). When
    /// it does, the form closes, the notice says what landed, the cursor goes to the row it landed
    /// on and that row's detail is re-read.
    fn land(&mut self, ctx: &Ctx<'_>) -> bool {
        let (Some(snapshot), Some(sent)) = (&self.snapshot, &self.sent) else {
            return false;
        };
        let landed = match sent {
            Sent::Area { project, code } => snapshot
                .project(*project)
                .and_then(|entry| entry.areas.iter().find(|area| area.code == *code))
                .map(|area| (format!("added area {code}"), Row::Area(area.id))),
            Sent::Mint {
                project,
                area,
                body,
                known,
            } => snapshot
                .project(*project)
                .and_then(|entry| {
                    entry
                        .in_area(*area)
                        .find(|row| !known.contains(&row.id) && row.body == *body)
                })
                .map(|row| (format!("minted {}", row.key), Row::Requirement(row.id))),
            Sent::Amend {
                id,
                expected_version,
                body,
                rationale,
                priority,
            } => snapshot
                .requirement(*id)
                .filter(|row| {
                    row.version > *expected_version
                        && row.body == *body
                        && row.rationale == *rationale
                        && row.priority == *priority
                })
                .map(|row| {
                    (
                        format!("amended {} to v{}", row.key, row.version),
                        Row::Requirement(row.id),
                    )
                }),
            Sent::Withdraw { id } => snapshot
                .requirement(*id)
                .filter(|row| row.state == RequirementState::Withdrawn)
                .map(|row| (format!("withdrew {}", row.key), Row::Requirement(row.id))),
        };
        let Some((message, row)) = landed else {
            return false;
        };
        self.busy = None;
        self.sent = None;
        self.mode = Mode::Browse;
        self.notice = Some(Notice::Info(message));
        self.select_row(row, ctx);
        true
    }

    /// Makes `row` visible, puts the cursor on it and re-reads its detail: `land`'s tail, shared
    /// with a reveal (MOD-64 D235). The detail is re-read even when `row` was already selected —
    /// `land` needs that, the row having just been written.
    fn select_row(&mut self, row: Row, ctx: &Ctx<'_>) {
        self.unhide(row, ctx);
        if self.selected != Some(row) {
            self.detail = None;
            self.scroll.reset();
        }
        self.selected = Some(row);
        self.detail_error = None;
        if let Row::Requirement(id) = row {
            ctx.request(StoreRequest::RequirementDetail(id));
        }
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

    /// A `RequirementsStale` over the write in flight: the form stays with its text, its token
    /// moves to the head, and the notice says so. A head withdrawn elsewhere takes no retry, and
    /// the notice says that instead.
    fn stale(&mut self) {
        if self.busy.take().is_none() {
            return;
        }
        self.sent = None;
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
        match &mut self.mode {
            Mode::Requirement(RequirementForm {
                target:
                    FormTarget::Amend {
                        expected_version, ..
                    },
                ..
            }) => *expected_version = head,
            Mode::Withdraw(form) => form.set_expected_version(head),
            _ => {}
        }
        self.notice = Some(Notice::Error(requirement_changed_elsewhere(head)));
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

    /// The hint row for the current mode.
    fn hint(&self, theme: &Theme) -> Line<'static> {
        match &self.mode {
            Mode::Browse => {
                let writes = if self.writes_allowed() {
                    theme.base
                } else {
                    theme.dim
                };
                Line::from(vec![
                    Span::styled(format!(" {HINT_MOVE}"), theme.base),
                    Span::styled(HINT_WRITES, writes),
                    Span::styled(HINT_RELOAD, theme.base),
                ])
            }
            Mode::Filter { field } => {
                let mut spans = vec![Span::styled(" /", theme.accent)];
                spans.extend(field.line(FILTER_WIDTH, true, theme).spans);
                spans.push(Span::styled(FILTER_HINT, theme.dim));
                Line::from(spans)
            }
            Mode::NewArea(_) => Line::styled(format!(" {FORM_HINT}"), theme.dim),
            Mode::Requirement(form) => {
                let extra = if form.focus == FormFocus::Priority {
                    PRIORITY_HINT
                } else {
                    ""
                };
                Line::styled(format!(" {FORM_HINT}{extra}"), theme.dim)
            }
            Mode::Withdraw(_) => Line::styled(format!(" {WITHDRAW_HINT}"), theme.dim),
        }
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

    /// A form or the filter captures every key but `CONTROL`/`ALT` chords, so digits, `q` and
    /// `Tab` are text there; `Ctrl+S` saves a form.
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // `save` does nothing on the filter, which has nothing to save.
        if self.captures_input() && ctrl_s(&key) {
            self.save(ctx);
            return Handled::Consumed;
        }
        if !plain(&key) {
            return Handled::Pass;
        }
        match self.mode {
            Mode::Browse => self.on_browse_key(key, ctx),
            Mode::Filter { .. } => self.on_filter_key(key, ctx),
            Mode::NewArea(_) | Mode::Requirement(_) | Mode::Withdraw(_) => {
                self.on_form_key(key, ctx)
            }
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Requirements(snapshot) => {
                if !snapshot.is_for(ctx.scope) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.recovered();
                if self.busy.is_some() && self.land(ctx) {
                    self.verifying = None;
                    return;
                }
                if let Some(message) = self.verifying.take() {
                    // The mint did not land after all: the refusal stands, and the form keeps
                    // its text for a retry.
                    self.busy = None;
                    self.sent = None;
                    self.notice = Some(Notice::Error(message));
                }
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
            StoreReply::RequirementsStale(snapshot) => {
                if !snapshot.is_for(ctx.scope) {
                    return;
                }
                self.snapshot = Some((**snapshot).clone());
                self.recovered();
                self.stale();
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
                if let Some(refused) = self.verifying.take() {
                    // The check could not be made: report the mint's own refusal.
                    self.busy = None;
                    self.sent = None;
                    self.notice = Some(Notice::Error(refused));
                    self.unavailable = Some(message.clone());
                    return;
                }
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
                if matches!(self.sent, Some(Sent::Mint { .. })) {
                    // The mint may have applied with only its re-read failing: look before
                    // offering a retry that would mint a second requirement.
                    self.verifying = Some(message.clone());
                    self.notice = Some(Notice::Info(CHECKING_MINT.to_owned()));
                    ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
                    return;
                }
                // A refused write leaves the form as it was: nothing was written.
                self.busy = None;
                self.sent = None;
                self.notice = Some(Notice::Error(message.clone()));
            }
            _ => {}
        }
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
        let [left, right] = Layout::horizontal([
            Constraint::Percentage(TREE_PERCENT),
            Constraint::Percentage(DETAIL_PERCENT),
        ])
        .areas(content);
        self.render_tree(frame, left, ctx);
        self.render_detail(frame, right, ctx);
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(format!(" {line}"), style))
                    .collect::<Vec<_>>(),
            ),
            notice_row,
        );
        frame.render_widget(Paragraph::new(self.hint(ctx.theme)), hint_row);
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
    use super::{HINT_WRITES, Mode, Notice, RequirementsTab, Row};
    use crate::app::{Action, Ctx, Emit, Handled, TopBarState};
    use crate::keymap::Keymap;
    use crate::requirements::{self, READ_NAME, RequirementsSnapshot, not_the_maintainer};
    use crate::store_worker::{Origin, StoreReply, StoreRequest};
    use crate::ui::Theme;
    use crate::ui::tabs::registry::Tab as _;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, RequirementId, RequirementState, Scope};
    use htui_core::store::MemStore;
    use htui_core::store::requirement_withdrawn;
    use htui_store::{Backend, DATABASE_UNREACHABLE};
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
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Bench {
        fn new(scope: Scope, projects: Vec<ProjectRef>) -> Self {
            Self {
                scope,
                projects,
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

    #[tokio::test]
    async fn captures_input_follows_the_mode() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot);
        assert!(!tab.captures_input());
        assert_eq!(
            bench.key(&mut tab, KeyCode::Char('q')),
            Handled::Pass,
            "`q` is the shell's in Browse"
        );
        assert_eq!(bench.key(&mut tab, KeyCode::Char('3')), Handled::Pass);

        bench.key(&mut tab, KeyCode::Char('n'));
        assert!(tab.captures_input(), "the mint form is open");
        for code in [KeyCode::Char('q'), KeyCode::Char('3'), KeyCode::Tab] {
            assert_eq!(
                bench.key(&mut tab, code),
                Handled::Consumed,
                "{code:?} is the form's"
            );
        }
        bench.key(&mut tab, KeyCode::Esc);
        assert!(!tab.captures_input(), "Esc closes the form");
        assert!(bench.emit.take().is_empty(), "and nothing was sent");
    }

    /// The style the Browse hint draws the four write words in.
    fn write_words_style(tab: &RequirementsTab, theme: &Theme) -> Style {
        tab.hint(theme)
            .spans
            .iter()
            .find(|span| span.content == HINT_WRITES)
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

    /// Blueprint F-16 by content: a read showing another session's amend (the version moved, the
    /// text is theirs) is not this amend landing. The form stays, and the stale answer that
    /// follows moves its token.
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

    /// MOD-39 review #2: a refused mint may have applied with only its re-read failing, so the
    /// tab reads again before offering a retry, and a mint the read shows landed closes the form.
    #[tokio::test]
    async fn a_refused_mint_that_landed_anyway_is_not_offered_again() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope.clone(), projects);
        let mut tab = tab_on(snapshot.clone());
        let name = mint(&bench, &mut tab, "Twice is too many.");

        bench.reply(
            &mut tab,
            &StoreReply::Failed {
                request: name,
                message: "the re-read failed".to_owned(),
            },
        );
        assert_eq!(tab.busy, Some(name), "still waiting to know");
        let sent = bench.emit.take();
        assert!(
            matches!(requests(&sent).as_slice(), [StoreRequest::Requirements(read)] if *read == scope),
            "{sent:?}"
        );

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
        entry.requirements.push(row);
        bench.reply(&mut tab, &StoreReply::Requirements(Box::new(landed)));

        assert_eq!(tab.busy, None);
        assert!(matches!(tab.mode, Mode::Browse), "the form closed");
        assert!(
            matches!(&tab.notice, Some(Notice::Info(text)) if text.contains("R-ENT-3")),
            "{:?}",
            tab.notice
        );
    }

    /// MOD-39 review #2, the other way: the read shows no such requirement, so the refusal stands
    /// and the form keeps its text for a retry.
    #[tokio::test]
    async fn a_refused_mint_the_read_does_not_show_stays_refused() {
        let (snapshot, projects, scope) = platform().await;
        let bench = Bench::new(scope, projects);
        let mut tab = tab_on(snapshot.clone());
        let name = mint(&bench, &mut tab, "Never written.");
        bench.reply(
            &mut tab,
            &StoreReply::Failed {
                request: name,
                message: "refused".to_owned(),
            },
        );
        let _ = bench.emit.take();

        bench.reply(&mut tab, &StoreReply::Requirements(Box::new(snapshot)));

        assert_eq!(tab.busy, None);
        assert_eq!(tab.notice, Some(Notice::Error("refused".to_owned())));
        assert!(
            matches!(tab.mode, Mode::Requirement(_)),
            "the form keeps its text"
        );
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
}
