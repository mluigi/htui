//! The Documents sub-tab: the item's documents, without their bodies, and `a`/`v` to write one
//! by hand (MOD-13 milestone 5 D9).
//!
//! - **Keys**: `J`/`K` move a cursor over the rows, drawn by style only; `PgUp`/`PgDn` scroll.
//!   `a` sends `DocumentForm` with no kind (a new document: kind, title and body typed); `v`
//!   sends it with the kind under the cursor (a new version: the kind fixed, title and body from
//!   that kind's latest version, the cursor on the body). Its answer opens a [`Compose`] area over
//!   the pane, which then captures every key; Ctrl+S sends `WriteDocument`, Ctrl+E hands the body
//!   to `$EDITOR`, `Esc` drops it. Every other key passes while browsing.
//! - **Nothing to save** (the plan's maintainer answer to blueprint §6 Q1): Ctrl+S on a `v` form
//!   whose title and body are the base's is refused in the form, and nothing is sent.
//! - **Replies** are this pane's own requests for this pane's item: a `DocumentForm` opens the
//!   area only for the `a`/`v` that asked, and only while Docs is the active sub-tab (the registry
//!   hands it to no other, E4); `DocumentWritten` closes it, re-reads the list and says under the
//!   table which version landed.
//! - **D5**: a document is never a compare-and-set. The store allocates the next version, and
//!   when another landed first, the sentence names it; nothing was overwritten
//!   ([`saved_as`]).
//! - **D10**: a refused `WriteDocument` (offline, the validator, an unknown item) keeps the text
//!   and says the sentence; any other failure may have followed a COMMIT whose answer was lost, so
//!   it is hedged and the list re-read.

use std::collections::BTreeSet;

use htui_core::model::hand_written::SUMMARY_KIND;
use htui_core::model::item_spec::NOTHING_TO_SAVE;
use htui_core::model::{DocumentHead, ItemId};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::app::{Action, Ctx, Handled};
use crate::editor::ExternalEditOutcome;
use crate::hand_written::{DOCUMENT_FORM_NAME, WRITE_DOCUMENT_NAME, write_refused};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells;
use crate::ui::tabs::backlog::detail::compose::{self, Compose, ComposeOutcome, may_have_landed};
use crate::ui::tabs::backlog::detail::{DetailId, DetailTab, Scroll, message};
use crate::ui::tabs::backlog::item_form::ctrl_s;
use crate::ui::tabs::settings::wrapped;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Written by a run step.
const BY_STEP: &str = "step";

/// Written by a human.
const BY_HAND: &str = "hand";

/// The browse hint on the pane's last row (D11).
const HINT: &str = "J/K move \u{b7} a new \u{b7} v new version";

/// What a `WriteDocument` in flight expects (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    /// `opened + 1` for `v`; for `a`, the highest version of the kind in the pane's list, plus
    /// one.
    version: i32,
    /// The base version `v` opened; `None` for `a`.
    opened: Option<i32>,
}

/// The `Documents` reply as a table of kind, version, provenance and title, with a cursor, and
/// the compose area `a`/`v` open over it.
#[derive(Debug, Default)]
pub struct DocumentsTab {
    /// The documents of the selected item, by kind then version.
    documents: Vec<DocumentHead>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// The selected item's key, from `StoreReply::Item` (E2): the temp-file stem.
    key: Option<String>,
    /// Index of the row under the cursor (D9; drawn by style only).
    cursor: usize,
    /// First visible document.
    scroll: Scroll,
    /// The `DocumentForm` read in flight: the item and `v`'s kind.
    opening: Option<(ItemId, Option<String>)>,
    /// The base version the open form came from (`v`), for the version a save expects.
    opened: Option<i32>,
    /// What the `WriteDocument` in flight expects.
    expected: Option<Expected>,
    /// The open compose area (D6).
    compose: Option<Compose>,
    /// D5: the last save's sentence, under the table until the next key.
    notice: Option<String>,
}

impl DocumentsTab {
    /// Identity of the Documents sub-tab.
    pub const ID: DetailId = DetailId("documents");

    /// A sub-tab with no documents yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The kinds hint under a typed kind (D9): the item's kinds plus `summary`, sorted, once each.
    fn kinds(&self) -> String {
        self.documents
            .iter()
            .map(|document| document.kind.as_str())
            .chain([SUMMARY_KIND])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The highest listed version of `kind`, or 0 when the list holds none.
    fn max_version(&self, kind: &str) -> i32 {
        self.documents
            .iter()
            .filter(|document| document.kind == kind)
            .map(|document| document.version)
            .max()
            .unwrap_or(0)
    }

    /// Moves the cursor `delta` rows, clamped to the list, and brings it back into view
    /// (`ReqsTab`'s rule).
    fn move_cursor(&mut self, delta: isize) {
        let last = self.documents.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
        self.scroll.reset();
    }

    /// `a` (`kind: None`) or `v`: asks for the form; its answer opens the area.
    fn open(&mut self, item: ItemId, kind: Option<String>, ctx: &Ctx<'_>) {
        self.opening = Some((item, kind.clone()));
        self.notice = None;
        ctx.request(StoreRequest::DocumentForm { item, kind });
    }

    /// A key while the area is open: its outcome, mapped for the Backlog.
    fn on_compose_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Option<Handled> {
        let compose = self.compose.as_mut()?;
        // The maintainer answer to blueprint §6 Q1: a `v` form saved untouched writes nothing.
        if ctrl_s(&key) && compose.busy().is_none() && compose.unchanged() {
            compose.settle(Some(NOTHING_TO_SAVE.to_owned()));
            return Some(Handled::Consumed);
        }
        Some(match compose.on_key(key) {
            ComposeOutcome::Stay => Handled::Consumed,
            ComposeOutcome::Pass => Handled::Pass,
            ComposeOutcome::Cancel => {
                self.compose = None;
                self.opened = None;
                Handled::Consumed
            }
            ComposeOutcome::Save(request) => {
                if let StoreRequest::WriteDocument { kind, .. } = &request {
                    self.expected = Some(Expected {
                        version: self
                            .opened
                            .map_or_else(|| self.max_version(kind) + 1, |opened| opened + 1),
                        opened: self.opened,
                    });
                }
                ctx.request(request);
                Handled::Consumed
            }
            ComposeOutcome::External(edit) => {
                ctx.emit(Action::EditExternally(edit));
                Handled::Consumed
            }
        })
    }

    /// D10: a `WriteDocument` failed. A refusal keeps the text and says the sentence; anything
    /// else is hedged and the list re-read. A failure with no busy area (an item change dropped
    /// it) is ignored.
    fn on_write_failed(&mut self, message: &str, ctx: &Ctx<'_>) {
        let Some(compose) = self
            .compose
            .as_mut()
            .filter(|compose| compose.busy() == Some(WRITE_DOCUMENT_NAME))
        else {
            return;
        };
        self.expected = None;
        if write_refused(message) {
            compose.settle(Some(message.to_owned()));
        } else {
            compose.settle(Some(may_have_landed(message, "list")));
            if let Some(item) = self.item {
                ctx.request(StoreRequest::Documents(item));
            }
        }
    }

    /// The compose area's top rule: `a`'s, or `v`'s with the version it came from.
    fn compose_title(&self, compose: &Compose) -> String {
        if compose.kind_is_typed() {
            return " New document ".to_owned();
        }
        let kind = compose.kind().unwrap_or_default();
        match self.opened {
            Some(opened) => format!(" New version of {kind} (from v{opened}) "),
            None => format!(" New version of {kind} "),
        }
    }

    /// The table, the cursor's row selected, scrolled so the cursor stays in view.
    fn render_table(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        // The header takes one row.
        let visible = usize::from(area.height).saturating_sub(1);
        let skip = self
            .scroll
            .skip()
            .max((self.cursor + 1).saturating_sub(visible));
        let rows = self
            .documents
            .iter()
            .enumerate()
            .skip(skip)
            .map(|(at, document)| {
                let by = if document.produced_by_step_id.is_some() {
                    BY_STEP
                } else {
                    BY_HAND
                };
                let row = Row::new(vec![
                    Cell::from(Line::styled(document.kind.clone(), ctx.theme.accent)),
                    Cell::from(Line::styled(
                        format!("v{}", document.version),
                        ctx.theme.base,
                    )),
                    Cell::from(Line::styled(by, ctx.theme.dim)),
                    Cell::from(Line::styled(document.title.clone(), ctx.theme.base)),
                ]);
                if at == self.cursor {
                    row.style(ctx.theme.selected)
                } else {
                    row
                }
            })
            .collect::<Vec<_>>();

        let header = Row::new(vec![
            Cell::from("kind"),
            Cell::from("ver"),
            Cell::from("by"),
            Cell::from("title"),
        ])
        .style(ctx.theme.title);

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Length(4),
                Constraint::Min(0),
            ],
        )
        .header(header)
        .column_spacing(1);
        frame.render_widget(table, area);
    }
}

/// D5: what a landed `WriteDocument` says under the table. `landed` past `expected` means
/// someone else wrote in between; nothing was overwritten (append-only, `0001_init.sql:399`).
/// `opened` is the base version `v` opened, `None` for `a` (E6).
#[must_use]
pub fn saved_as(kind: &str, landed: i32, expected: i32, opened: Option<i32>) -> String {
    let saved = format!("saved as {kind} v{landed}");
    if landed <= expected {
        return saved;
    }
    let since = opened.map_or_else(|| "the form".to_owned(), |opened| format!("v{opened}"));
    let last = landed - 1;
    if last == expected {
        format!("{saved} \u{2014} v{expected} was written after you opened {since}; both are kept")
    } else {
        format!(
            "{saved} \u{2014} v{expected}\u{2013}v{last} were written after you opened {since}; \
             all are kept"
        )
    }
}

impl DetailTab for DocumentsTab {
    fn id(&self) -> DetailId {
        Self::ID
    }

    /// "Docs", not "Documents": the seventh sub-tab needed the five columns (MOD-39 plan P12).
    fn title(&self) -> &str {
        "Docs"
    }

    fn on_item_change(&mut self, item: Option<ItemId>) {
        *self = Self {
            item,
            ..Self::default()
        };
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if let Some(handled) = self.on_compose_key(key, ctx) {
            return handled;
        }
        // D9: the sentence stays until the next key.
        self.notice = None;
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
            KeyCode::Char('J') => {
                self.move_cursor(1);
                Handled::Consumed
            }
            KeyCode::Char('K') => {
                self.move_cursor(-1);
                Handled::Consumed
            }
            KeyCode::Char('a') => {
                self.open(item, None, ctx);
                Handled::Consumed
            }
            KeyCode::Char('v') => {
                if let Some(kind) = self.documents.get(self.cursor).map(|row| row.kind.clone()) {
                    self.open(item, Some(kind), ctx);
                }
                Handled::Consumed
            }
            _ => self.scroll.on_key(key, self.documents.len()),
        }
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match self.compose.as_mut() {
            Some(compose) => {
                compose.on_paste(text);
                Handled::Consumed
            }
            None => Handled::Pass,
        }
    }

    fn captures_input(&self) -> bool {
        self.compose.is_some()
    }

    fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        match self.compose.as_mut() {
            Some(compose) => compose.on_external_edit(outcome),
            None => tracing::debug!("no document compose area to take the $EDITOR outcome"),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Documents(documents) => {
                self.documents.clone_from(documents);
                self.cursor = self.cursor.min(documents.len().saturating_sub(1));
                self.scroll.reset();
            }
            StoreReply::Item(row) => {
                if let Some(row) = row.as_ref()
                    && Some(row.id) == self.item
                {
                    self.key = Some(row.key.clone());
                }
            }
            StoreReply::DocumentForm(context)
                if matches!(&self.opening, Some((item, _)) if *item == context.item)
                    && Some(context.item) == self.item =>
            {
                // A base must be of the asked kind, and `a` asks for none; anything else is
                // another read's answer, so the one asked for may still come.
                let asked = self.opening.as_ref().and_then(|(_, kind)| kind.as_deref());
                let answers = match (asked, &context.base) {
                    (Some(kind), Some(base)) => base.kind == kind,
                    (None, Some(_)) => false,
                    (_, None) => true,
                };
                if !answers {
                    return;
                }
                let Some((item, kind)) = self.opening.take() else {
                    return;
                };
                self.opened = context.base.as_ref().map(|base| base.version);
                self.compose = Some(Compose::document(
                    item,
                    self.key.as_deref(),
                    kind,
                    context.base.as_ref(),
                ));
            }
            StoreReply::DocumentWritten {
                item,
                kind,
                version,
            } if Some(*item) == self.item
                && self.compose.as_ref().and_then(Compose::busy) == Some(WRITE_DOCUMENT_NAME) =>
            {
                let expected = self.expected.take().unwrap_or(Expected {
                    version: *version,
                    opened: None,
                });
                self.notice = Some(saved_as(kind, *version, expected.version, expected.opened));
                self.compose = None;
                self.opened = None;
                ctx.request(StoreRequest::Documents(*item));
            }
            // D2: the App already put the refusal on the status line.
            StoreReply::Failed { request, .. } if *request == DOCUMENT_FORM_NAME => {
                self.opening = None;
            }
            StoreReply::Failed { request, message } if *request == WRITE_DOCUMENT_NAME => {
                self.on_write_failed(message, ctx);
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        if let Some(compose) = &self.compose {
            let title = self.compose_title(compose);
            let kinds = self.kinds();
            compose::render(frame, area, compose, &title, Some(&kinds), ctx.theme);
            return;
        }
        if self.item.is_none() {
            message(frame, area, "No item selected.", ctx.theme);
            return;
        }
        // D5's sentence under the table; with none the table keeps the rect above the hint (E5).
        let notice = self
            .notice
            .as_deref()
            .map(|sentence| wrapped(sentence, usize::from(area.width)))
            .unwrap_or_default();
        let height = u16::try_from(notice.len()).unwrap_or(u16::MAX);
        // D11: the browse hint takes the last row, under the table and the notice.
        let [table, below, hint] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(height),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(
            Line::styled(cells::clip(HINT, usize::from(hint.width)), ctx.theme.dim),
            hint,
        );
        if self.documents.is_empty() {
            message(frame, table, "No documents for this item.", ctx.theme);
        } else {
            self.render_table(frame, table, ctx);
        }
        frame.render_widget(
            Paragraph::new(
                notice
                    .into_iter()
                    .map(|line| Line::styled(line, ctx.theme.base))
                    .collect::<Vec<_>>(),
            ),
            below,
        );
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::item_spec::NOTHING_TO_SAVE;
    use htui_core::store::{MemStore, ReadStore as _, StoreError};
    use htui_store::Backend;
    use ratatui::style::Modifier;

    use super::*;
    use crate::app::Action;
    use crate::hand_written::{self, DOCUMENT_FORM_NAME, HandText, WRITE_DOCUMENT_NAME};
    use crate::store_worker::StoreRequest;
    use crate::ui::tabs::backlog::detail::compose::bench::{Shell, ctrl, drawn, key};
    use crate::ui::tabs::backlog::detail::compose::{Part, may_have_landed};

    const ITEM: ItemId = ids::HTUI_FEAT_1;

    /// A request as the bench prints it.
    fn sent(request: &StoreRequest) -> String {
        format!("{request:?}")
    }

    /// The store requests emitted since the last take, as values (to serve them).
    fn taken(shell: &Shell) -> Vec<StoreRequest> {
        shell
            .actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::Store(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// The demo store and a backend over it; clones share state.
    fn demo() -> (MemStore, Backend) {
        let store = MemStore::demo();
        (store.clone(), Backend::memory(store))
    }

    /// `request` served by the worker over `backend`, as the pane would be answered.
    async fn served(backend: &Backend, request: &StoreRequest) -> StoreReply {
        hand_written::serve(backend, request)
            .await
            .expect("the demo serves it")
    }

    /// A pane on `item` that has landed its documents and its row (E2's key).
    async fn pane_on(shell: &Shell, store: &MemStore, item: ItemId) -> DocumentsTab {
        let mut pane = DocumentsTab::new();
        pane.on_item_change(Some(item));
        let documents = store.documents(item).await.expect("the documents");
        pane.on_reply(&StoreReply::Documents(documents), &mut shell.ctx());
        let row = store.item(item).await.expect("the item");
        pane.on_reply(&StoreReply::Item(Box::new(row)), &mut shell.ctx());
        pane
    }

    /// [`pane_on`] FEAT-1 (plan v1, plan v2, prd v1).
    async fn pane(shell: &Shell, store: &MemStore) -> DocumentsTab {
        pane_on(shell, store, ITEM).await
    }

    /// `c` pressed on `pane` and its `DocumentForm` read answered by `backend`.
    async fn open(shell: &Shell, backend: &Backend, pane: &mut DocumentsTab, c: char) {
        assert_eq!(
            pane.on_key(key(KeyCode::Char(c)), &mut shell.ctx()),
            Handled::Consumed
        );
        let requests = taken(shell);
        assert_eq!(requests.len(), 1, "one DocumentForm: {requests:?}");
        let reply = served(backend, &requests[0]).await;
        pane.on_reply(&reply, &mut shell.ctx());
        assert!(pane.captures_input(), "the form is open");
    }

    /// `v` on plan (the cursor's first row), its body edited, and Ctrl+S: a `WriteDocument` in
    /// flight, returned.
    async fn saving_v(shell: &Shell, backend: &Backend, pane: &mut DocumentsTab) -> StoreRequest {
        open(shell, backend, pane, 'v').await;
        assert_eq!(
            pane.on_paste("Edited by hand.\n", &mut shell.ctx()),
            Handled::Consumed
        );
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        let mut requests = taken(shell);
        assert_eq!(requests.len(), 1, "one WriteDocument: {requests:?}");
        requests.remove(0)
    }

    fn compose(pane: &DocumentsTab) -> &Compose {
        pane.compose.as_ref().expect("the form is open")
    }

    fn press(pane: &mut DocumentsTab, shell: &Shell, code: KeyCode) -> Handled {
        pane.on_key(key(code), &mut shell.ctx())
    }

    #[tokio::test]
    async fn j_and_k_move_the_cursor_and_clamp() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut pane = pane(&shell, &store).await;
        assert_eq!(pane.documents.len(), 3);
        assert_eq!(
            press(&mut pane, &shell, KeyCode::Char('K')),
            Handled::Consumed
        );
        assert_eq!(pane.cursor, 0);
        for _ in 0..5 {
            assert_eq!(
                press(&mut pane, &shell, KeyCode::Char('J')),
                Handled::Consumed
            );
        }
        assert_eq!(pane.cursor, 2);
        assert_eq!(
            press(&mut pane, &shell, KeyCode::Char('K')),
            Handled::Consumed
        );
        assert_eq!(pane.cursor, 1);
        assert!(shell.actions().is_empty());
        // A shorter list lands the cursor on its last row.
        let mut fewer = store.documents(ITEM).await.expect("the documents");
        fewer.truncate(1);
        pane.on_reply(&StoreReply::Documents(fewer), &mut shell.ctx());
        assert_eq!(pane.cursor, 0);
    }

    #[tokio::test]
    async fn a_sends_document_form_with_no_kind() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut pane = pane(&shell, &store).await;
        assert_eq!(
            press(&mut pane, &shell, KeyCode::Char('a')),
            Handled::Consumed
        );
        assert_eq!(
            shell.requests(),
            [sent(&StoreRequest::DocumentForm {
                item: ITEM,
                kind: None
            })]
        );
        assert!(!pane.captures_input(), "nothing opens before the reply");
    }

    #[tokio::test]
    async fn v_sends_document_form_with_the_kind_under_the_cursor() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut pane = pane(&shell, &store).await;
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        assert_eq!(
            press(&mut pane, &shell, KeyCode::Char('v')),
            Handled::Consumed
        );
        assert_eq!(
            shell.requests(),
            [sent(&StoreRequest::DocumentForm {
                item: ITEM,
                kind: Some("prd".to_owned())
            })]
        );
    }

    #[tokio::test]
    async fn a_and_v_send_nothing_without_an_item_and_v_nothing_without_rows() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut none = DocumentsTab::new();
        none.on_item_change(None);
        assert_eq!(press(&mut none, &shell, KeyCode::Char('a')), Handled::Pass);
        assert_eq!(press(&mut none, &shell, KeyCode::Char('v')), Handled::Pass);
        let mut empty = pane_on(&shell, &store, ids::HTUI_ANA_2).await;
        assert!(empty.documents.is_empty(), "ANA-2 has no documents");
        assert_eq!(
            press(&mut empty, &shell, KeyCode::Char('v')),
            Handled::Consumed
        );
        assert!(shell.actions().is_empty());
        assert!(empty.opening.is_none());
    }

    #[tokio::test]
    async fn a_v_reply_opens_the_form_with_the_kind_fixed_and_the_base_text() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        open(&shell, &backend, &mut pane, 'v').await;
        let base = store
            .document(ids::DOC_FEAT_1_PLAN_V2)
            .await
            .expect("the read")
            .expect("plan v2");
        let area = compose(&pane);
        assert_eq!(area.kind().as_deref(), Some("plan"));
        assert!(!area.kind_is_typed());
        assert_eq!(area.title(), Some("Plan: TUI scaffold (revised)"));
        assert_eq!(area.body(), base.body);
        // Maintainer answer Q2: `v` opens on the body.
        assert_eq!(area.focus(), Part::Body);
        assert_eq!(pane.opened, Some(2));
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(text.contains(" New version of plan (from v2) "), "{text}");
        assert!(!text.contains("kinds:"), "{text}");
    }

    #[tokio::test]
    async fn an_a_reply_opens_an_empty_form_on_the_kind() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        open(&shell, &backend, &mut pane, 'a').await;
        let area = compose(&pane);
        assert!(area.kind_is_typed());
        assert_eq!(area.kind(), None);
        assert_eq!(area.title(), Some(""));
        assert_eq!(area.body(), "");
        assert_eq!(area.focus(), Part::Kind);
        assert_eq!(pane.opened, None);
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(text.contains(" New document "), "{text}");
    }

    #[tokio::test]
    async fn a_document_form_reply_unasked_for_another_item_or_kind_is_ignored() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let plan = served(
            &backend,
            &StoreRequest::DocumentForm {
                item: ITEM,
                kind: Some("plan".to_owned()),
            },
        )
        .await;
        // Unasked.
        pane.on_reply(&plan, &mut shell.ctx());
        assert!(!pane.captures_input());
        // Asked for prd, answered with a plan base: not this read's answer.
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        let _ = press(&mut pane, &shell, KeyCode::Char('v'));
        let _ = shell.actions();
        pane.on_reply(&plan, &mut shell.ctx());
        assert!(!pane.captures_input());
        // The read asked for still opens the form.
        let prd = served(
            &backend,
            &StoreRequest::DocumentForm {
                item: ITEM,
                kind: Some("prd".to_owned()),
            },
        )
        .await;
        pane.on_reply(&prd, &mut shell.ctx());
        assert_eq!(compose(&pane).kind().as_deref(), Some("prd"));
        let _ = press(&mut pane, &shell, KeyCode::Esc);
        assert!(!pane.captures_input());
        // `v`, then `a` before the answer: `v`'s base is not `a`'s, and `a`'s opens a typed form.
        let _ = press(&mut pane, &shell, KeyCode::Char('v'));
        let _ = press(&mut pane, &shell, KeyCode::Char('a'));
        let mut requests = taken(&shell);
        assert_eq!(requests.len(), 2, "{requests:?}");
        let typed = served(&backend, &requests.remove(1)).await;
        let based = served(&backend, &requests.remove(0)).await;
        pane.on_reply(&based, &mut shell.ctx());
        assert!(!pane.captures_input());
        pane.on_reply(&typed, &mut shell.ctx());
        assert!(compose(&pane).kind_is_typed());
        assert_eq!(pane.opened, None);
        let _ = press(&mut pane, &shell, KeyCode::Esc);
        // Asked, answered for another item.
        let other = served(
            &backend,
            &StoreRequest::DocumentForm {
                item: ids::HTUI_ANA_1,
                kind: None,
            },
        )
        .await;
        let _ = press(&mut pane, &shell, KeyCode::Char('a'));
        pane.on_reply(&other, &mut shell.ctx());
        assert!(!pane.captures_input());
    }

    #[tokio::test]
    async fn a_failed_document_form_opens_nothing_and_adds_no_error() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let _ = press(&mut pane, &shell, KeyCode::Char('a'));
        let request = taken(&shell).remove(0);
        pane.on_reply(
            &StoreReply::Failed {
                request: DOCUMENT_FORM_NAME,
                message: "store unreachable".to_owned(),
            },
            &mut shell.ctx(),
        );
        assert!(shell.actions().is_empty(), "the App says it");
        let late = served(&backend, &request).await;
        pane.on_reply(&late, &mut shell.ctx());
        assert!(!pane.captures_input(), "a late reply opens nothing");
    }

    #[tokio::test]
    async fn ctrl_s_sends_write_document() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let request = saving_v(&shell, &backend, &mut pane).await;
        let text = sent(&request);
        assert!(text.starts_with("WriteDocument"), "{text}");
        assert!(text.contains("kind: \"plan\""), "{text}");
        assert!(text.contains("Plan: TUI scaffold (revised)"), "{text}");
        assert!(
            !text.contains("Edited by hand."),
            "the body prints as its length: {text}"
        );
        assert_eq!(compose(&pane).busy(), Some(WRITE_DOCUMENT_NAME));
        assert_eq!(
            pane.expected,
            Some(Expected {
                version: 3,
                opened: Some(2)
            })
        );

        // `a` expects the kind's highest listed version plus one, or 1 for a new kind.
        let shell = Shell::new();
        let mut typed = pane_on(&shell, &store, ITEM).await;
        open(&shell, &backend, &mut typed, 'a').await;
        let _ = typed.on_paste("plan", &mut shell.ctx());
        let _ = press(&mut typed, &shell, KeyCode::Tab);
        let _ = typed.on_paste("Mine", &mut shell.ctx());
        let _ = press(&mut typed, &shell, KeyCode::Tab);
        let _ = typed.on_paste("Body.", &mut shell.ctx());
        assert_eq!(typed.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        assert_eq!(taken(&shell).len(), 1);
        assert_eq!(
            typed.expected,
            Some(Expected {
                version: 3,
                opened: None
            })
        );
    }

    /// Maintainer answer (blueprint §6 Q1): Ctrl+S on a `v` form whose title and body are the
    /// base's is refused in the pane with `NOTHING_TO_SAVE`, and nothing is sent.
    #[tokio::test]
    async fn an_unchanged_v_save_is_refused_with_nothing_to_save() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        open(&shell, &backend, &mut pane, 'v').await;
        // The demo's v2 body ends in `\n`, which the store would drop: still unchanged.
        assert!(compose(&pane).body().ends_with('\n'));
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        assert_eq!(compose(&pane).notice(), Some(NOTHING_TO_SAVE));
        assert_eq!(compose(&pane).busy(), None);
        assert!(pane.expected.is_none());
        assert!(shell.actions().is_empty(), "nothing sent");
        // An edit to the body makes it a save.
        let _ = pane.on_paste("x", &mut shell.ctx());
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        let requests = shell.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(requests[0].starts_with("WriteDocument"), "{requests:?}");
        assert_eq!(compose(&pane).busy(), Some(WRITE_DOCUMENT_NAME));
    }

    #[tokio::test]
    async fn written_at_the_expected_version_closes_rereads_and_says_saved() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let request = saving_v(&shell, &backend, &mut pane).await;
        let reply = served(&backend, &request).await;
        assert!(
            matches!(&reply, StoreReply::DocumentWritten { version: 3, .. }),
            "{reply:?}"
        );
        pane.on_reply(&reply, &mut shell.ctx());
        assert_eq!(pane.notice.as_deref(), Some("saved as plan v3"));
        assert!(pane.compose.is_none());
        assert!(!pane.captures_input());
        assert!(pane.expected.is_none());
        assert!(pane.opened.is_none());
        assert_eq!(shell.requests(), [sent(&StoreRequest::Documents(ITEM))]);
    }

    #[tokio::test]
    async fn written_past_the_expected_version_names_the_version_in_between() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let request = saving_v(&shell, &backend, &mut pane).await;
        // Someone else lands plan v3 first.
        let theirs = StoreRequest::WriteDocument {
            item: ITEM,
            kind: "plan".to_owned(),
            title: "Theirs".to_owned(),
            body: HandText::new("Theirs."),
        };
        let _ = served(&backend, &theirs).await;
        let reply = served(&backend, &request).await;
        assert!(
            matches!(&reply, StoreReply::DocumentWritten { version: 4, .. }),
            "{reply:?}"
        );
        pane.on_reply(&reply, &mut shell.ctx());
        assert_eq!(
            pane.notice.as_deref(),
            Some("saved as plan v4 \u{2014} v3 was written after you opened v2; both are kept")
        );
    }

    #[tokio::test]
    async fn kind_title_and_body_refusals_show_in_the_form() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        open(&shell, &backend, &mut pane, 'a').await;
        let refused = |pane: &mut DocumentsTab, sentence: &str, part: Part| {
            assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
            assert_eq!(compose(pane).notice(), Some(sentence));
            assert_eq!(compose(pane).focus(), part);
            assert_eq!(compose(pane).busy(), None);
            assert!(pane.expected.is_none());
        };
        refused(&mut pane, "a document needs a kind", Part::Kind);
        let _ = pane.on_paste("review", &mut shell.ctx());
        refused(&mut pane, "a document needs a title", Part::Title);
        let _ = pane.on_paste("A review", &mut shell.ctx());
        refused(&mut pane, "a document needs a body", Part::Body);
        assert!(shell.actions().is_empty(), "nothing sent");

        // The worker's refusal keeps the text and says the sentence, with no re-read.
        let _ = pane.on_paste("Looks good.", &mut shell.ctx());
        assert_eq!(pane.on_key(ctrl('s'), &mut shell.ctx()), Handled::Consumed);
        assert_eq!(taken(&shell).len(), 1);
        let message = StoreError::Constraint("a document needs a body".to_owned()).to_string();
        pane.on_reply(
            &StoreReply::Failed {
                request: WRITE_DOCUMENT_NAME,
                message: message.clone(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(compose(&pane).notice(), Some(message.as_str()));
        assert_eq!(compose(&pane).body(), "Looks good.");
        assert_eq!(compose(&pane).busy(), None);
        assert!(pane.expected.is_none());
        assert!(shell.actions().is_empty(), "no re-read for a refusal");
    }

    #[tokio::test]
    async fn a_store_failure_hedges_and_rereads_the_list() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let _ = saving_v(&shell, &backend, &mut pane).await;
        let message = "store backend error: connection reset".to_owned();
        pane.on_reply(
            &StoreReply::Failed {
                request: WRITE_DOCUMENT_NAME,
                message: message.clone(),
            },
            &mut shell.ctx(),
        );
        assert_eq!(
            compose(&pane).notice(),
            Some(may_have_landed(&message, "list").as_str())
        );
        assert_eq!(shell.requests(), [sent(&StoreRequest::Documents(ITEM))]);
        assert!(compose(&pane).body().starts_with("Edited by hand.\n"));
        assert_eq!(compose(&pane).busy(), None);
        assert!(pane.expected.is_none());
        assert_eq!(pane.opened, Some(2), "the form still comes from v2");
    }

    #[tokio::test]
    async fn the_kinds_hint_names_the_item_s_kinds_and_summary() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        assert_eq!(pane.kinds(), "plan, prd, summary");
        open(&shell, &backend, &mut pane, 'a').await;
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(text.contains("kinds: plan, prd, summary"), "{text}");

        let empty = pane_on(&shell, &store, ids::HTUI_ANA_2).await;
        assert_eq!(empty.kinds(), "summary");
    }

    #[tokio::test]
    async fn the_notice_clears_on_the_next_key() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let request = saving_v(&shell, &backend, &mut pane).await;
        let reply = served(&backend, &request).await;
        pane.on_reply(&reply, &mut shell.ctx());
        let rows = store.documents(ITEM).await.expect("the documents");
        pane.on_reply(&StoreReply::Documents(rows), &mut shell.ctx());
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(text.contains("saved as plan v3"), "{text}");
        assert!(text.contains("v3  hand"), "{text}");
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        assert_eq!(pane.notice, None);
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(!text.contains("saved as"), "{text}");
    }

    /// D11: `w` is not the pane's; it passes while browsing, as a chord does.
    #[tokio::test]
    async fn w_passes_while_browsing() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut pane = pane(&shell, &store).await;
        assert_eq!(press(&mut pane, &shell, KeyCode::Char('w')), Handled::Pass);
        let alt_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(pane.on_key(alt_a, &mut shell.ctx()), Handled::Pass);
        assert!(shell.actions().is_empty());
    }

    #[tokio::test]
    async fn an_item_change_drops_the_form() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let request = saving_v(&shell, &backend, &mut pane).await;
        // Review NIT3: `Esc` is swallowed while busy; only the item change drops the form.
        assert_eq!(press(&mut pane, &shell, KeyCode::Esc), Handled::Consumed);
        assert!(pane.captures_input(), "Esc does not close a busy form");
        pane.on_item_change(Some(ids::HTUI_ANA_1));
        assert!(pane.compose.is_none());
        assert!(pane.key.is_none());
        assert!(pane.opening.is_none());
        assert!(pane.opened.is_none());
        assert!(pane.expected.is_none());
        assert_eq!(pane.cursor, 0);
        // The old item's `DocumentWritten` finds no busy form and re-reads nothing.
        let reply = served(&backend, &request).await;
        pane.on_reply(&reply, &mut shell.ctx());
        assert!(pane.notice.is_none());
        assert!(shell.actions().is_empty());
    }

    #[tokio::test]
    async fn ctrl_e_emits_an_external_edit_with_the_key_and_kind_stem() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        open(&shell, &backend, &mut pane, 'v').await;
        assert_eq!(pane.on_key(ctrl('e'), &mut shell.ctx()), Handled::Consumed);
        let actions = shell.actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::EditExternally(edit)] if edit.stem == "FEAT-1-plan"
            ),
            "{actions:?}"
        );
        pane.on_external_edit(
            ExternalEditOutcome::Edited("From the editor.\n".to_owned()),
            &mut shell.ctx(),
        );
        // v2's body ended in `\n`, so the editor added none to drop.
        assert_eq!(compose(&pane).body(), "From the editor.\n");
    }

    #[test]
    fn saved_as_covers_every_case() {
        assert_eq!(saved_as("plan", 3, 3, Some(2)), "saved as plan v3");
        assert_eq!(
            saved_as("plan", 4, 3, Some(2)),
            "saved as plan v4 \u{2014} v3 was written after you opened v2; both are kept"
        );
        assert_eq!(
            saved_as("plan", 5, 3, Some(2)),
            "saved as plan v5 \u{2014} v3\u{2013}v4 were written after you opened v2; all are kept"
        );
        assert_eq!(
            saved_as("review", 2, 1, None),
            "saved as review v2 \u{2014} v1 was written after you opened the form; both are kept"
        );
        assert_eq!(
            saved_as("review", 4, 1, None),
            "saved as review v4 \u{2014} v1\u{2013}v3 were written after you opened the form; all \
             are kept"
        );
        assert_eq!(saved_as("summary", 1, 1, None), "saved as summary v1");
    }

    #[tokio::test]
    async fn the_cursor_row_is_drawn_selected() {
        let shell = Shell::new();
        let (store, _) = demo();
        let mut pane = pane(&shell, &store).await;
        let _ = press(&mut pane, &shell, KeyCode::Char('J'));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(43, 10)).expect("terminal");
        terminal
            .draw(|frame| pane.render(frame, frame.area(), &shell.ctx()))
            .expect("the frame draws");
        let buffer = terminal.backend().buffer();
        let reversed = |y: u16| buffer[(0, y)].modifier.contains(Modifier::REVERSED);
        assert!(!reversed(0), "the header");
        assert!(!reversed(1), "plan v1");
        assert!(reversed(2), "plan v2, under the cursor");
        assert!(!reversed(3), "prd v1");
    }

    #[test]
    fn the_hint_fits_the_detail_pane() {
        assert!(cells::cell_width(HINT) <= 43, "{HINT}");
    }

    /// D11: with an item and no form, the last row is the hint, under the table and under D5's
    /// notice, or under an empty list; with no item there is none.
    #[tokio::test]
    async fn the_hint_is_drawn_under_the_table() {
        let shell = Shell::new();
        let (store, backend) = demo();
        let mut pane = pane(&shell, &store).await;
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[22].trim_end(), HINT, "{text}");
        assert_eq!(text.matches(HINT).count(), 1, "{text}");

        // With D5's notice the hint is still last, and the notice sits right above it.
        let request = saving_v(&shell, &backend, &mut pane).await;
        let reply = served(&backend, &request).await;
        pane.on_reply(&reply, &mut shell.ctx());
        assert!(!pane.captures_input(), "the form closed");
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        let rows: Vec<&str> = text.lines().collect();
        assert_eq!(rows[22].trim_end(), HINT, "{text}");
        assert_eq!(rows[21].trim_end(), "saved as plan v3", "{text}");

        let mut empty = DocumentsTab::new();
        empty.on_item_change(Some(ITEM));
        empty.on_reply(&StoreReply::Documents(Vec::new()), &mut shell.ctx());
        let text = drawn(43, 23, |frame, rect| {
            empty.render(frame, rect, &shell.ctx())
        });
        assert!(text.contains("No documents for this item."), "{text}");
        assert_eq!(text.lines().last().map(str::trim_end), Some(HINT), "{text}");

        let mut none = DocumentsTab::new();
        none.on_item_change(None);
        let text = drawn(43, 23, |frame, rect| none.render(frame, rect, &shell.ctx()));
        assert!(!text.contains(HINT), "{text}");

        // The landed write re-read the list.
        let _ = taken(&shell);
        open(&shell, &backend, &mut pane, 'a').await;
        let text = drawn(43, 23, |frame, rect| pane.render(frame, rect, &shell.ctx()));
        assert!(!text.contains(HINT), "{text}");
    }
}
