//! The Reqs sub-tab: the requirements the selected item cites (MOD-39 PRD D3, D4; plan P12).
//!
//! Two lines per citation: the requirement's key, the citation kind, the version the citation was
//! stamped at and a `! suspect` marker when the requirement has moved on since (MOD-38 plan D11),
//! then the requirement's first line. A withdrawn requirement's line is dim.
//!
//! | Key | Does |
//! |---|---|
//! | `J` / `K` | move the cursor |
//! | `r` | re-confirm the suspect citation under the cursor (`ReconfirmCitation`) |
//! | `c` | pick one of the project's active requirements, then `a` addresses / `v` reserves |
//! | `u` | uncite the citation under the cursor, after a `y` |
//! | `PgUp` / `PgDn` | scroll |
//!
//! Only `addresses` and `reserves` are a human's to cite or uncite (PRD D4, plan P9): `amends` and
//! `withdraws` record the decision of a gated amend or withdraw, so `u` on one answers the status
//! line with `decision_citation_stays` and sends nothing. `r` on a citation that is not suspect
//! says so too, and so does `r` on an `addresses` / `reserves` citation of a withdrawn requirement,
//! with the store's own `withdrawn_requirement_cited` sentence (MOD-38 plan D10).
//!
//! The picker never offers a requirement the item already `addresses` or `reserves`: the store's
//! `cite` upserts, so citing it again would re-stamp it at the current version, clearing its
//! `suspect` without the `r` plan P9 asks for, and erase an agent's `proposed_by_step_id`. To
//! change a citation's kind, uncite it and cite again.
//!
//! The pane holds no store handle (`R-NF-3`): the Backlog tab reads `ItemRequirements` with the
//! other six reads, and each write answers a fresh [`ItemCitations`], which lands by content (the
//! item it is for), never by which request it answers (MOD-39 blueprint F-16). One write is in
//! flight at a time, and offline every write key answers `DATABASE_UNREACHABLE` without a request
//! (blueprint F-6); the hint row greys the write words then, as the Requirements tab does (PRD
//! Constraints, plan P10). While the picker or the `y` question is open the pane captures every
//! key but a `CONTROL` chord, the Runs pane's rule (MOD-4 plan OQ-7).
//!
//! The cursor follows a citation, not a position: across a reply it stays on the citation it was
//! on, or lands on the one a cite just made; an uncited row leaves it on the row that took its
//! place. A reply also drops any `PgUp` / `PgDn` scroll, so a shorter list is never scrolled past.
//!
//! Known residue: a `Failed` names its request, not its item, so a write refused for the previous
//! item frees a write of the same request in flight for the new one. It only loosens the
//! one-at-a-time guard; the store is never asked anything it would not have been.

use htui_core::model::{
    CitationKind, ItemCitation, ItemId, Requirement, RequirementId, RequirementState,
};
use htui_core::store::withdrawn_requirement_cited;
use htui_store::DATABASE_UNREACHABLE;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled};
use crate::requirements::{
    CITATIONS_NAME, ItemCitations, decision_citation_stays, is_citation_write,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::Theme;
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What `r` says on a citation stamped at the requirement's current version.
pub const NOT_SUSPECT: &str = "this citation is current; nothing to re-confirm";

/// What `r` and `u` say with no citation under the cursor.
pub const NO_CITATION: &str = "no citation is under the cursor";

/// What `c` says when the item's project has nothing left to cite.
pub const NO_CANDIDATES: &str = "the project has no active requirement left to cite";

/// Marks the row the cursor is on; the other rows carry a space of the same width.
const CURSOR: &str = "\u{25b8}";

/// Marks a cut in [`cut`].
const CUT: char = '\u{2026}';

/// The browsing footer's words before the write keys; with [`HINT_WRITES`], 42 columns, inside
/// the 43 a detail pane has at 100x30.
const HINT_MOVE: &str = "J/K move · ";

/// The three write keys, dim while the database is unreachable.
const HINT_WRITES: &str = "r reconfirm · c cite · u uncite";

/// The picker's footer, under its candidates.
const PICK_HINT: &str = "j/k move · Enter pick · Esc back";

/// The Reqs sub-tab (MOD-39 PRD D3, D4; plan P12).
#[derive(Debug, Default)]
pub struct ReqsTab {
    /// The selected item, or `None` on a project header.
    item: Option<ItemId>,
    /// The selected item's citations and candidates, once the read has answered.
    citations: Option<ItemCitations>,
    /// The refused read's sentence, drawn in place of the rows (blueprint §5.4).
    failed: Option<String>,
    /// Index of the citation under the cursor.
    cursor: usize,
    /// What the keys do right now.
    mode: Mode,
    /// The citation write in flight, by name.
    busy: Option<&'static str>,
    /// The citation a cite in flight makes, for the cursor to land on when its reply does.
    follow: Option<(RequirementId, CitationKind)>,
    /// Lines scrolled past the cursor's (`PgUp` / `PgDn`); a cursor move snaps back.
    scroll: Scroll,
}

/// The pane's modes. Every mode but [`Mode::Browse`] captures input.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The citation list.
    #[default]
    Browse,
    /// `c`: a cursor over the candidates.
    Pick {
        /// Index of the candidate under the picker's cursor.
        cursor: usize,
    },
    /// A candidate picked: `a` addresses, `v` reserves.
    Kind {
        /// The picked requirement.
        requirement: RequirementId,
    },
    /// `u`: `y` uncites.
    ConfirmUncite {
        /// The cited requirement.
        requirement: RequirementId,
        /// The citation's kind: `addresses` or `reserves`.
        kind: CitationKind,
    },
}

impl ReqsTab {
    /// Identity of the Reqs sub-tab.
    pub const ID: DetailId = DetailId("reqs");

    /// A sub-tab with no citations yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The item's live citations, empty until the read answers.
    fn rows(&self) -> &[ItemCitation] {
        self.citations
            .as_ref()
            .map_or(&[], |citations| citations.citations.as_slice())
    }

    /// What `c` can cite, empty until the read answers: the project's active requirements but
    /// those the item already `addresses` or `reserves`, which a cite would silently re-stamp.
    fn candidates(&self) -> Vec<&Requirement> {
        let Some(citations) = &self.citations else {
            return Vec::new();
        };
        let cited = |candidate: &Requirement| {
            citations.citations.iter().any(|citation| {
                citation.requirement.id == candidate.id
                    && matches!(
                        citation.kind,
                        CitationKind::Addresses | CitationKind::Reserves
                    )
            })
        };
        citations
            .candidates
            .iter()
            .filter(|candidate| !cited(candidate))
            .collect()
    }

    /// Whether the database takes a write: false until the read answers.
    fn writable(&self) -> bool {
        self.citations
            .as_ref()
            .is_some_and(|citations| citations.writable)
    }

    /// The citation under the cursor.
    fn under_cursor(&self) -> Option<&ItemCitation> {
        self.rows().get(self.cursor)
    }

    /// Moves the cursor `delta` rows, clamped to the list, and brings it back into view.
    fn move_cursor(&mut self, delta: isize) {
        let last = self.rows().len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.scroll.reset();
    }

    /// Why a write cannot go out now: one already in flight, or no database to write to.
    fn refusal(&self) -> Option<String> {
        if let Some(busy) = self.busy {
            return Some(format!("`{busy}` is still in flight"));
        }
        (!self.writable()).then(|| DATABASE_UNREACHABLE.to_owned())
    }

    /// Sends a citation write, or puts the reason it cannot go on the status line; whether it
    /// went.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) -> bool {
        if let Some(sentence) = self.refusal() {
            ctx.emit(Action::Error(sentence));
            return false;
        }
        self.busy = Some(request.name());
        ctx.request(request);
        true
    }

    /// `r`: re-confirms the suspect citation under the cursor.
    fn reconfirm(&mut self, item: ItemId, ctx: &Ctx<'_>) {
        let Some(citation) = self.under_cursor() else {
            ctx.emit(Action::Error(NO_CITATION.to_owned()));
            return;
        };
        if !citation.suspect {
            ctx.emit(Action::Error(NOT_SUSPECT.to_owned()));
            return;
        }
        let requirement = &citation.requirement;
        if requirement.state == RequirementState::Withdrawn
            && matches!(
                citation.kind,
                CitationKind::Addresses | CitationKind::Reserves
            )
        {
            let sentence = withdrawn_requirement_cited(&requirement.key, citation.kind);
            ctx.emit(Action::Error(sentence));
            return;
        }
        let request = StoreRequest::ReconfirmCitation {
            item,
            requirement: citation.requirement.id,
            kind: citation.kind,
        };
        self.send(request, ctx);
    }

    /// `c`: opens the picker over the project's active requirements.
    fn open_picker(&mut self, ctx: &Ctx<'_>) {
        if self.candidates().is_empty() {
            ctx.emit(Action::Error(NO_CANDIDATES.to_owned()));
        } else if let Some(sentence) = self.refusal() {
            ctx.emit(Action::Error(sentence));
        } else {
            self.mode = Mode::Pick { cursor: 0 };
        }
    }

    /// `u`: asks before unciting the citation under the cursor; a decision citation stays.
    fn ask_uncite(&mut self, ctx: &Ctx<'_>) {
        let Some(citation) = self.under_cursor() else {
            ctx.emit(Action::Error(NO_CITATION.to_owned()));
            return;
        };
        let (requirement, kind) = (citation.requirement.id, citation.kind);
        if matches!(kind, CitationKind::Amends | CitationKind::Withdraws) {
            ctx.emit(Action::Error(decision_citation_stays(kind)));
        } else if let Some(sentence) = self.refusal() {
            ctx.emit(Action::Error(sentence));
        } else {
            self.mode = Mode::ConfirmUncite { requirement, kind };
        }
    }

    /// A key while the pane captures: a `CONTROL` chord passes, so `ctrl-c` still quits, and
    /// everything else is the mode's, answered or swallowed.
    fn modal_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::Pass;
        }
        let Some(item) = self.item else {
            self.mode = Mode::Browse;
            return Handled::Consumed;
        };
        self.mode = match self.mode {
            Mode::Browse => Mode::Browse,
            Mode::Pick { cursor } => self.pick_key(cursor, key.code),
            Mode::Kind { requirement } => {
                let kind = match key.code {
                    KeyCode::Char('a') => Some(CitationKind::Addresses),
                    KeyCode::Char('v') => Some(CitationKind::Reserves),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let cite = StoreRequest::CiteRequirement {
                        item,
                        requirement,
                        kind,
                    };
                    if self.send(cite, ctx) {
                        self.follow = Some((requirement, kind));
                    }
                    Mode::Browse
                } else if key.code == KeyCode::Esc {
                    let cursor = self
                        .candidates()
                        .iter()
                        .position(|candidate| candidate.id == requirement)
                        .unwrap_or(0);
                    Mode::Pick { cursor }
                } else {
                    Mode::Kind { requirement }
                }
            }
            Mode::ConfirmUncite { requirement, kind } => match key.code {
                KeyCode::Char('y') => {
                    self.send(
                        StoreRequest::UnciteRequirement {
                            item,
                            requirement,
                            kind,
                        },
                        ctx,
                    );
                    Mode::Browse
                }
                KeyCode::Char('n') | KeyCode::Esc => Mode::Browse,
                _ => Mode::ConfirmUncite { requirement, kind },
            },
        };
        Handled::Consumed
    }

    /// A key in the picker: move, pick, or back to the list.
    fn pick_key(&self, cursor: usize, code: KeyCode) -> Mode {
        let last = self.candidates().len().saturating_sub(1);
        match code {
            KeyCode::Char('j' | 'J') | KeyCode::Down => Mode::Pick {
                cursor: (cursor + 1).min(last),
            },
            KeyCode::Char('k' | 'K') | KeyCode::Up => Mode::Pick {
                cursor: cursor.saturating_sub(1),
            },
            KeyCode::Enter => {
                self.candidates()
                    .get(cursor)
                    .map_or(Mode::Pick { cursor }, |candidate| Mode::Kind {
                        requirement: candidate.id,
                    })
            }
            KeyCode::Esc => Mode::Browse,
            _ => Mode::Pick { cursor },
        }
    }

    /// The citation rows, two lines each, and the line index just past the cursor's pair.
    fn list_lines(&self, width: usize, theme: &Theme) -> (Vec<Line<'static>>, usize) {
        let mut lines = Vec::with_capacity(self.rows().len() * 2);
        for (index, citation) in self.rows().iter().enumerate() {
            let requirement = &citation.requirement;
            let withdrawn = requirement.state == RequirementState::Withdrawn;
            let (key_style, text_style): (Style, Style) = if withdrawn {
                (theme.dim, theme.dim)
            } else {
                (theme.accent, theme.base)
            };
            let marker = if index == self.cursor { CURSOR } else { " " };
            let mut head = vec![
                Span::styled(format!("{marker} "), theme.base),
                Span::styled(requirement.key.clone(), key_style),
                Span::styled(
                    format!(" {} v{}", citation.kind, citation.requirement_version),
                    text_style,
                ),
            ];
            if citation.suspect {
                head.push(Span::styled(" ! suspect", theme.error));
            }
            lines.push(Line::from(head));
            lines.push(Line::styled(
                format!(
                    "  {}",
                    cut(first_line(&requirement.body), width.saturating_sub(2))
                ),
                theme.dim,
            ));
        }
        (lines, (self.cursor + 1) * 2)
    }

    /// The footer of the current mode, every line cut to `width`; the picker shows at most
    /// `room` candidates, a window that keeps its cursor in view.
    fn footer(&self, width: usize, room: usize, theme: &Theme) -> Vec<Line<'static>> {
        let hint = |text: &str| Line::styled(cut(text, width), theme.dim);
        let title = |text: String| Line::styled(cut(&text, width), theme.title);
        match self.mode {
            Mode::Browse => {
                let moves = cut(HINT_MOVE, width);
                let rest = width.saturating_sub(moves.chars().count());
                let writes = if self.writable() {
                    theme.base
                } else {
                    theme.dim
                };
                vec![Line::from(vec![
                    Span::styled(moves, theme.base),
                    Span::styled(cut(HINT_WRITES, rest), writes),
                ])]
            }
            Mode::Pick { cursor } => {
                let room = room.max(1);
                let skip = (cursor + 1).saturating_sub(room);
                let mut lines: Vec<Line<'static>> = self
                    .candidates()
                    .iter()
                    .enumerate()
                    .skip(skip)
                    .take(room)
                    .map(|(index, candidate)| {
                        let marker = if index == cursor { CURSOR } else { " " };
                        let text = format!(
                            "{marker} {} {} {}",
                            candidate.key,
                            candidate.priority,
                            first_line(&candidate.body)
                        );
                        Line::styled(cut(&text, width), theme.base)
                    })
                    .collect();
                lines.push(hint(PICK_HINT));
                lines
            }
            Mode::Kind { requirement } => vec![
                title(format!("cite {} as:", self.key_of(requirement))),
                hint("a addresses · v reserves · Esc back"),
            ],
            Mode::ConfirmUncite { requirement, kind } => vec![
                title(format!("uncite {} ({kind})?", self.key_of(requirement))),
                hint("y uncite · n keep"),
            ],
        }
    }

    /// A requirement's key, from the candidates or the citations; its id when neither has it.
    fn key_of(&self, requirement: RequirementId) -> String {
        self.candidates()
            .into_iter()
            .chain(self.rows().iter().map(|citation| &citation.requirement))
            .find(|candidate| candidate.id == requirement)
            .map_or_else(|| requirement.to_string(), |found| found.key.clone())
    }
}

impl DetailTab for ReqsTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Reqs"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        self.item = item;
        self.citations = None;
        self.failed = None;
        self.cursor = 0;
        self.mode = Mode::Browse;
        self.busy = None;
        self.follow = None;
        self.scroll.reset();
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !matches!(self.mode, Mode::Browse) {
            return self.modal_key(key, ctx);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Handled::Pass;
        }
        let Some(item) = self.item else {
            return Handled::Pass;
        };
        match key.code {
            KeyCode::Char('J') => self.move_cursor(1),
            KeyCode::Char('K') => self.move_cursor(-1),
            KeyCode::Char('r') => self.reconfirm(item, ctx),
            KeyCode::Char('c') => self.open_picker(ctx),
            KeyCode::Char('u') => self.ask_uncite(ctx),
            _ => return self.scroll.on_key(key, self.rows().len() * 2),
        }
        Handled::Consumed
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// Blueprint §5.4: a fresh [`ItemCitations`] for this item lands whichever request it
    /// answers, the cursor following its citation; a refused read is drawn in the pane; a refused
    /// write only frees the pane, the status line already has its sentence. Only the write in
    /// flight is freed: a refusal of another request is not this pane's to act on.
    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::ItemCitations(citations) if Some(citations.item) == self.item => {
                let target = self.follow.take().or_else(|| {
                    self.under_cursor()
                        .map(|citation| (citation.requirement.id, citation.kind))
                });
                self.citations = Some((**citations).clone());
                self.failed = None;
                let found = target.and_then(|(requirement, kind)| {
                    self.rows().iter().position(|citation| {
                        citation.requirement.id == requirement && citation.kind == kind
                    })
                });
                let last = self.rows().len().saturating_sub(1);
                self.cursor = found.unwrap_or(self.cursor).min(last);
                self.scroll.reset();
                let last_candidate = self.candidates().len().saturating_sub(1);
                if let Mode::Pick { cursor } = &mut self.mode {
                    *cursor = (*cursor).min(last_candidate);
                }
                self.busy = None;
            }
            StoreReply::Failed { request, message } if *request == CITATIONS_NAME => {
                self.failed = Some(message.clone());
            }
            StoreReply::Failed { request, .. }
                if is_citation_write(request) && self.busy == Some(*request) =>
            {
                self.busy = None;
                self.follow = None;
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        let width = usize::from(area.width);
        let footer = self.footer(width, usize::from(area.height / 2), ctx.theme);
        let [list, prompt] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::try_from(footer.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(footer), prompt);

        if self.citations.is_none()
            && let Some(failed) = &self.failed
        {
            frame.render_widget(
                Paragraph::new(Line::styled(failed.clone(), ctx.theme.error))
                    .wrap(Wrap { trim: true }),
                list,
            );
            return;
        }
        if self.rows().is_empty() {
            message(frame, list, "No requirements cited.", ctx.theme);
            return;
        }
        let (lines, cursor_end) = self.list_lines(width, ctx.theme);
        let skip = self
            .scroll
            .skip()
            .max(cursor_end.saturating_sub(usize::from(list.height)));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(skip).unwrap_or(u16::MAX), 0)),
            list,
        );
    }
}

/// The first line of `text`, the whole of it when it has no newline.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// `text` in at most `width` characters, cut with `…` as its last one.
///
/// Counted in `char`s, like the Runs pane's `fit`; a control character becomes a space, so one
/// line stays one line.
fn cut(text: &str, width: usize) -> String {
    let flat: Vec<char> = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.len() <= width {
        flat.into_iter().collect()
    } else if width == 0 {
        String::new()
    } else {
        let mut out: String = flat[..width - 1].iter().collect();
        out.push(CUT);
        out
    }
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::{ProjectRef, Scope, WorkspaceId};
    use htui_core::store::{MemStore, WriteStore as _};
    use htui_store::Backend;

    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::requirements;
    use crate::store_worker::Origin;
    use crate::ui::tabs::BacklogTab;

    /// Everything a `Ctx` borrows, kept alive for the length of a test (the Runs pane's `Shell`).
    struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        theme: Theme,
        emit: Emit,
    }

    impl Shell {
        fn new() -> Self {
            Self {
                scope: Scope {
                    workspace_id: WorkspaceId::default(),
                    project_ids: Vec::new(),
                },
                projects: Vec::new(),
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

        /// What the pane emitted since the last call, as `(requests, status-line sentences)`; the
        /// requests by [`sent`], since `StoreRequest` has no `PartialEq`.
        fn drained(&self) -> (Vec<String>, Vec<String>) {
            let mut requests = Vec::new();
            let mut errors = Vec::new();
            for action in self.emit.take() {
                match action {
                    Action::Store(request) => requests.push(sent(&request)),
                    Action::Error(sentence) => errors.push(sentence),
                    other => panic!("the pane emits requests and errors only, not {other:?}"),
                }
            }
            (requests, errors)
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A request as the assertions compare it: its `Debug`, which names every field.
    fn sent(request: &StoreRequest) -> String {
        format!("{request:?}")
    }

    /// The demo's citations of `item`, as the worker answers them.
    async fn citations(item: ItemId) -> ItemCitations {
        requirements::citations(&Backend::memory(MemStore::demo()), item)
            .await
            .expect("the memory store never fails")
    }

    /// A pane on `item` that has landed `reply`.
    fn pane(shell: &Shell, item: ItemId, reply: ItemCitations) -> ReqsTab {
        let mut pane = ReqsTab::new();
        pane.on_item_change(Some(item));
        pane.on_reply(
            &StoreReply::ItemCitations(Box::new(reply)),
            &mut shell.ctx(),
        );
        pane
    }

    /// Plan P9: `ANA-2`'s `amends` citation records a decision; `u` says so and asks nothing.
    #[tokio::test]
    async fn u_is_refused_on_a_decision_citation() {
        let shell = Shell::new();
        let mut pane = pane(&shell, ids::HTUI_ANA_2, citations(ids::HTUI_ANA_2).await);
        assert_eq!(
            pane.on_key(key(KeyCode::Char('u')), &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(
            shell.drained(),
            (
                Vec::new(),
                vec![decision_citation_stays(CitationKind::Amends)]
            )
        );
        assert!(!pane.captures_input(), "nothing is asked");
    }

    /// `r` only re-confirms a suspect citation: `ANA-2`'s is current, `ANA-1`'s is not.
    #[tokio::test]
    async fn r_on_a_current_citation_says_so() {
        let shell = Shell::new();
        let mut current = pane(&shell, ids::HTUI_ANA_2, citations(ids::HTUI_ANA_2).await);
        current.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        assert_eq!(shell.drained(), (Vec::new(), vec![NOT_SUSPECT.to_owned()]));

        let mut suspect = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        suspect.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        assert_eq!(
            shell.drained(),
            (
                vec![sent(&StoreRequest::ReconfirmCitation {
                    item: ids::HTUI_ANA_1,
                    requirement: ids::REQ_ENT_1,
                    kind: CitationKind::Addresses,
                })],
                Vec::new()
            )
        );
        suspect.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        assert_eq!(
            shell.drained(),
            (
                Vec::new(),
                vec!["`reconfirm_citation` is still in flight".to_owned()]
            ),
            "one write at a time"
        );
    }

    /// Blueprint §5.4, F-16: a reply lands by the item it is for, so an answer for another item
    /// changes nothing.
    #[tokio::test]
    async fn a_reply_for_another_item_is_ignored() {
        let shell = Shell::new();
        let mut pane = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        let before = pane.citations.clone();
        pane.on_reply(
            &StoreReply::ItemCitations(Box::new(citations(ids::HTUI_FEAT_1).await)),
            &mut shell.ctx(),
        );
        assert_eq!(pane.citations, before, "FEAT-1's rows are not ANA-1's");

        pane.on_item_change(Some(ids::HTUI_CLEAN_1));
        assert_eq!(pane.citations, None, "a new item drops the old rows");
    }

    /// The pane captures exactly while it is not browsing, and a `CONTROL` chord always passes.
    #[tokio::test]
    async fn captures_input_follows_the_mode() {
        let shell = Shell::new();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut pane = pane(&shell, ids::HTUI_FEAT_1, citations(ids::HTUI_FEAT_1).await);
        assert!(!pane.captures_input(), "browsing");

        pane.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        assert!(pane.captures_input(), "the picker");
        assert_eq!(pane.on_key(ctrl_c, &mut shell.ctx()), Handled::Pass);
        assert_eq!(
            pane.on_key(key(KeyCode::Char('q')), &mut shell.ctx()),
            Handled::Consumed,
            "`q` does not leave the picker"
        );
        pane.on_key(key(KeyCode::Char('j')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        assert!(pane.captures_input(), "the kind question");
        pane.on_key(key(KeyCode::Esc), &mut shell.ctx());
        assert_eq!(
            pane.mode,
            Mode::Pick { cursor: 1 },
            "Esc goes back to the picker, on the candidate it left"
        );
        pane.on_key(key(KeyCode::Enter), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('v')), &mut shell.ctx());
        assert!(!pane.captures_input(), "a cite ends the capture");
        assert_eq!(
            shell.drained(),
            (
                vec![sent(&StoreRequest::CiteRequirement {
                    item: ids::HTUI_FEAT_1,
                    requirement: ids::REQ_ENT_2,
                    kind: CitationKind::Reserves,
                })],
                Vec::new()
            )
        );

        pane.on_key(key(KeyCode::Char('u')), &mut shell.ctx());
        assert_eq!(
            shell.drained(),
            (
                Vec::new(),
                vec!["`cite_requirement` is still in flight".to_owned()]
            ),
            "`u` waits for the cite"
        );
        pane.on_reply(
            &StoreReply::Failed {
                request: "cite_requirement",
                message: "refused".to_owned(),
            },
            &mut shell.ctx(),
        );
        pane.on_key(key(KeyCode::Char('u')), &mut shell.ctx());
        assert!(
            pane.captures_input(),
            "the uncite question, once the cite is answered"
        );
        assert_eq!(
            pane.on_key(key(KeyCode::Char('x')), &mut shell.ctx()),
            Handled::Consumed
        );
        pane.on_key(key(KeyCode::Char('n')), &mut shell.ctx());
        assert!(!pane.captures_input(), "`n` keeps it");
        assert_eq!(shell.drained(), (Vec::new(), Vec::new()));
    }

    /// Blueprint F-6: offline every write key answers `DATABASE_UNREACHABLE` and sends nothing.
    #[tokio::test]
    async fn writes_are_refused_offline() {
        let shell = Shell::new();
        let mut reply = citations(ids::HTUI_ANA_1).await;
        reply.writable = false;
        let refused = (Vec::new(), vec![DATABASE_UNREACHABLE.to_owned()]);
        let mut pane = pane(&shell, ids::HTUI_ANA_1, reply);

        pane.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        assert_eq!(shell.drained(), refused, "`r`");
        pane.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        assert_eq!(shell.drained(), refused, "`c`");
        pane.on_key(key(KeyCode::Char('u')), &mut shell.ctx());
        assert_eq!(shell.drained(), refused, "`u`");
        assert!(!pane.captures_input(), "no question was opened");
    }

    /// The width a detail pane has at the 100x30 the snapshots render at.
    const PANE_WIDTH: u16 = 43;

    /// The candidates' keys, in the picker's order.
    fn candidate_keys(pane: &ReqsTab) -> Vec<String> {
        pane.candidates()
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect()
    }

    /// A line's text, its spans joined.
    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The demo's citations of `item`, with `requirement` withdrawn.
    async fn with_withdrawn(item: ItemId, requirement: RequirementId) -> ItemCitations {
        let mut reply = citations(item).await;
        for citation in &mut reply.citations {
            if citation.requirement.id == requirement {
                citation.requirement.state = RequirementState::Withdrawn;
            }
        }
        reply
    }

    /// Review of T4: the store's `cite` upserts, so the picker leaves out what the item already
    /// `addresses` or `reserves`; `ANA-1` could otherwise clear its suspect `R-ENT-1` without `r`.
    /// A decision citation does not count: `ANA-2` amended `R-ENT-1` and may still address it.
    #[tokio::test]
    async fn the_picker_skips_what_the_item_already_cites() {
        let shell = Shell::new();
        let ana_1 = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        assert_eq!(candidate_keys(&ana_1), ["R-ENT-2", "R-STO-1"]);
        let feat_1 = pane(&shell, ids::HTUI_FEAT_1, citations(ids::HTUI_FEAT_1).await);
        assert_eq!(candidate_keys(&feat_1), ["R-ENT-1", "R-ENT-2"]);
        let ana_2 = pane(&shell, ids::HTUI_ANA_2, citations(ids::HTUI_ANA_2).await);
        assert_eq!(candidate_keys(&ana_2), ["R-ENT-1", "R-ENT-2", "R-STO-1"]);

        let mut reply = citations(ids::HTUI_ANA_1).await;
        reply
            .candidates
            .retain(|candidate| candidate.id == ids::REQ_ENT_1);
        let mut nothing_left = pane(&shell, ids::HTUI_ANA_1, reply);
        nothing_left.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        assert_eq!(
            shell.drained(),
            (Vec::new(), vec![NO_CANDIDATES.to_owned()])
        );
        assert!(!nothing_left.captures_input(), "no picker over nothing");
    }

    /// Blueprint §5.3: a withdrawn requirement's key and text are dim; an active one's key is
    /// accented. The `! suspect` marker keeps its colour either way.
    #[tokio::test]
    async fn withdrawn_rows_render_dim() {
        let shell = Shell::new();
        let theme = Theme::default();
        let withdrawn = pane(
            &shell,
            ids::HTUI_ANA_1,
            with_withdrawn(ids::HTUI_ANA_1, ids::REQ_ENT_1).await,
        );
        let (lines, _) = withdrawn.list_lines(usize::from(PANE_WIDTH), &theme);
        let head = &lines[0];
        assert!(text(head).contains("R-ENT-1 addresses v1 ! suspect"));
        for span in &head.spans {
            let expected = match span.content.trim() {
                "" | CURSOR => theme.base,
                "! suspect" => theme.error,
                _ => theme.dim,
            };
            assert_eq!(span.style, expected, "`{}`", span.content);
        }
        assert_eq!(lines[1].style, theme.dim, "the first line under it");

        let active = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        let (lines, _) = active.list_lines(usize::from(PANE_WIDTH), &theme);
        let key = lines[0]
            .spans
            .iter()
            .find(|span| span.content == "R-ENT-1")
            .expect("the key is its own span");
        assert_eq!(key.style, theme.accent, "an active key is accented");
    }

    /// Review of T4: `r` on a suspect citation of a withdrawn requirement answers the store's
    /// sentence without the round trip (MOD-38 plan D10).
    #[tokio::test]
    async fn r_on_a_withdrawn_requirement_is_refused_here() {
        let shell = Shell::new();
        let mut pane = pane(
            &shell,
            ids::HTUI_ANA_1,
            with_withdrawn(ids::HTUI_ANA_1, ids::REQ_ENT_1).await,
        );
        pane.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        assert_eq!(
            shell.drained(),
            (
                Vec::new(),
                vec![withdrawn_requirement_cited(
                    "R-ENT-1",
                    CitationKind::Addresses
                )]
            )
        );
        assert_eq!(pane.busy, None, "nothing is in flight");
    }

    /// The pane drawn at [`PANE_WIDTH`], one `String` per row, trailing blanks trimmed.
    fn drawn(pane: &ReqsTab, shell: &Shell) -> Vec<String> {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(PANE_WIDTH, 12))
            .expect("the test backend is constructible");
        term.draw(|frame| pane.render(frame, frame.area(), &shell.ctx()))
            .expect("the pane draws");
        let buffer = term.backend().buffer();
        (buffer.area.top()..buffer.area.bottom())
            .map(|y| {
                let row: String = (buffer.area.left()..buffer.area.right())
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect();
                row.trim_end().to_owned()
            })
            .collect()
    }

    /// Blueprint §5.4: a refused read is drawn in the pane, above the hint, and a later answer
    /// replaces it.
    #[tokio::test]
    async fn a_refused_read_is_drawn_in_the_pane() {
        let shell = Shell::new();
        let mut pane = ReqsTab::new();
        pane.on_item_change(Some(ids::HTUI_ANA_1));
        pane.on_reply(
            &StoreReply::Failed {
                request: CITATIONS_NAME,
                message: "the read was refused".to_owned(),
            },
            &mut shell.ctx(),
        );
        let rows = drawn(&pane, &shell);
        assert_eq!(rows[0], "the read was refused", "{rows:#?}");
        assert_eq!(rows.last().map(String::as_str), Some(BROWSE_HINT_TEXT));

        pane.on_reply(
            &StoreReply::ItemCitations(Box::new(citations(ids::HTUI_ANA_1).await)),
            &mut shell.ctx(),
        );
        let rows = drawn(&pane, &shell);
        assert!(rows[0].contains("R-ENT-1 addresses v1"), "{rows:#?}");
    }

    /// The hint row in full: [`HINT_MOVE`] then [`HINT_WRITES`].
    const BROWSE_HINT_TEXT: &str = "J/K move · r reconfirm · c cite · u uncite";

    /// PRD Constraints, plan P10: offline the write words of the hint are dim; online they are
    /// not.
    #[tokio::test]
    async fn the_hint_greys_the_write_words_offline() {
        let shell = Shell::new();
        let theme = Theme::default();
        let writes_style = |pane: &ReqsTab| {
            let footer = pane.footer(usize::from(PANE_WIDTH), 8, &theme);
            assert_eq!(text(&footer[0]), BROWSE_HINT_TEXT);
            let writes = footer[0]
                .spans
                .iter()
                .find(|span| span.content == HINT_WRITES)
                .expect("the write words are their own span");
            writes.style
        };
        let online = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        assert_eq!(writes_style(&online), theme.base);
        let mut reply = citations(ids::HTUI_ANA_1).await;
        reply.writable = false;
        let offline = pane(&shell, ids::HTUI_ANA_1, reply);
        assert_eq!(writes_style(&offline), theme.dim);
    }

    /// The picker shows at most `room` candidates, a window that keeps its cursor in view.
    #[tokio::test]
    async fn the_picker_window_follows_its_cursor() {
        let shell = Shell::new();
        let theme = Theme::default();
        let mut pane = pane(
            &shell,
            ids::HTUI_CLEAN_1,
            citations(ids::HTUI_CLEAN_1).await,
        );
        assert_eq!(candidate_keys(&pane), ["R-ENT-1", "R-ENT-2", "R-STO-1"]);
        pane.on_key(key(KeyCode::Char('c')), &mut shell.ctx());
        let window = |pane: &ReqsTab| {
            pane.footer(usize::from(PANE_WIDTH), 2, &theme)
                .iter()
                .map(|line| text(line).chars().take(10).collect::<String>())
                .collect::<Vec<_>>()
        };
        assert_eq!(window(&pane), ["▸ R-ENT-1 ", "  R-ENT-2 ", "j/k move ·"]);
        pane.on_key(key(KeyCode::Char('j')), &mut shell.ctx());
        pane.on_key(key(KeyCode::Char('j')), &mut shell.ctx());
        assert_eq!(window(&pane), ["  R-ENT-2 ", "▸ R-STO-1 ", "j/k move ·"]);
        pane.on_key(key(KeyCode::Char('j')), &mut shell.ctx());
        assert_eq!(pane.mode, Mode::Pick { cursor: 2 }, "the last stays last");
    }

    /// Review of T4: across a reply the cursor stays on its citation, lands on the one a cite
    /// made, and any `PgDn` scroll is dropped.
    #[tokio::test]
    async fn the_cursor_follows_its_citation_across_a_reply() {
        let shell = Shell::new();
        let before = citations(ids::HTUI_FEAT_1).await;
        let mut pane = pane(&shell, ids::HTUI_FEAT_1, before.clone());
        for code in [KeyCode::Char('c'), KeyCode::Char('j'), KeyCode::Enter] {
            pane.on_key(key(code), &mut shell.ctx());
        }
        pane.on_key(key(KeyCode::Char('a')), &mut shell.ctx());
        let (requests, _) = shell.drained();
        assert_eq!(requests.len(), 1, "the cite of R-ENT-2 went");

        let store = MemStore::demo();
        store
            .cite(
                ids::HTUI_FEAT_1,
                ids::REQ_ENT_2,
                CitationKind::Addresses,
                None,
            )
            .await
            .expect("the demo takes the cite");
        let after = requirements::citations(&Backend::memory(store), ids::HTUI_FEAT_1)
            .await
            .expect("the memory store never fails");
        let at = |reply: &ItemCitations, key: &str| {
            reply
                .citations
                .iter()
                .position(|citation| citation.requirement.key == key)
        };
        assert_eq!(at(&after, "R-STO-1"), Some(1), "R-ENT-2 sorts first");
        pane.on_key(key(KeyCode::PageDown), &mut shell.ctx());
        assert_eq!(pane.scroll.skip(), 1, "scrolled past the cursor's row");
        pane.on_reply(
            &StoreReply::ItemCitations(Box::new(after)),
            &mut shell.ctx(),
        );
        let under = |pane: &ReqsTab| pane.under_cursor().map(|c| c.requirement.key.clone());
        assert_eq!(under(&pane).as_deref(), Some("R-ENT-2"), "on the new one");
        assert_eq!(pane.scroll.skip(), 0, "the scroll is dropped");

        pane.on_key(key(KeyCode::Char('J')), &mut shell.ctx());
        assert_eq!(under(&pane).as_deref(), Some("R-STO-1"));
        pane.on_reply(
            &StoreReply::ItemCitations(Box::new(before)),
            &mut shell.ctx(),
        );
        assert_eq!(
            (pane.cursor, under(&pane).as_deref()),
            (0, Some("R-STO-1")),
            "R-STO-1 moved up a row and the cursor with it"
        );
    }

    /// Review of T4: a refused write frees the pane only when it is the write in flight.
    #[tokio::test]
    async fn only_the_write_in_flight_is_freed_by_its_refusal() {
        let shell = Shell::new();
        let mut pane = pane(&shell, ids::HTUI_ANA_1, citations(ids::HTUI_ANA_1).await);
        pane.on_key(key(KeyCode::Char('r')), &mut shell.ctx());
        shell.drained();
        let refused = |request: &'static str| StoreReply::Failed {
            request,
            message: "refused".to_owned(),
        };
        pane.on_reply(&refused("cite_requirement"), &mut shell.ctx());
        assert_eq!(
            pane.busy,
            Some("reconfirm_citation"),
            "another write's refusal"
        );
        pane.on_reply(&refused("reconfirm_citation"), &mut shell.ctx());
        assert_eq!(pane.busy, None, "its own");
    }
}
