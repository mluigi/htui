//! The compose area (MOD-13 milestone 5 D6): the one capturing editor the Notes and Docs panes
//! share. A pane opens a [`Compose`] when its form read answers, draws it over its whole content
//! rect with [`render`], and acts on the [`ComposeOutcome`] each key gives back, as the Backlog
//! acts on an `ItemFormOutcome`.
//!
//! - **Key order** follows the item form's: Ctrl+S saves (swallowed while busy), then Ctrl+E,
//!   then any other chord passes (`ctrl-c` quits), then, while busy, every other key is
//!   swallowed, `Esc` included; `Tab`/`BackTab` cycle the parts; the focused part takes the rest.
//! - **Ctrl+E hands the body only** to `$EDITOR` (D6, D7; blueprint E7): on the kind or the title
//!   it is swallowed, as on the item form's one-line fields.
//! - **The outcome comes back** through [`Compose::on_external_edit`], with the item form's rules:
//!   control characters dropped, one editor-added newline dropped, the cursor at the end.
//! - **Busy**: a save leaves the area busy with its request's name until the pane's reply
//!   [`settle`](Compose::settle)s it (a lost answer's [`checking`](Compose::checking) keeps it
//!   busy until the pane's re-read does, review M1); the validator in `htui_core::model::hand_written` refuses
//!   first, in the area, and nothing is sent.
//! - **Redaction**: `Compose`'s `Debug` prints lengths and the fixed kind, never the title or the
//!   body.

use crossterm::event::{KeyCode, KeyEvent};
use htui_core::model::{Document, ItemId, hand_written};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::editor::{
    EDITED, ExternalEdit, ExternalEditOutcome, NO_CHANGES, WAIT_FLAG, strip_added_newline,
    without_controls,
};
use crate::hand_written::{ADD_NOTE_NAME, HandText, WRITE_DOCUMENT_NAME};
use crate::store_worker::StoreRequest;
use crate::ui::cells::{self, cell_width};
use crate::ui::tabs::backlog::filter::CHORD;
use crate::ui::tabs::backlog::item_form::{HINT_AREA, HINT_TEXT, PAGE, ctrl_e, ctrl_s, marker};
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};

/// Width of a row's label, after the two-cell marker: `title` plus two.
const LABEL: usize = 7;
/// Rows the body keeps when a long notice grows (the item form's `BODY_MIN`).
const BODY_MIN: u16 = 3;

/// One focusable part of an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A document's kind, typed (`a`) only.
    Kind,
    /// A document's title.
    Title,
    /// The note or document body.
    Body,
}

/// What one key did, for the pane to act on (`ItemFormOutcome`'s shape).
#[derive(Debug)]
pub enum ComposeOutcome {
    /// Edited, moved, refused or swallowed; nothing to send.
    Stay,
    /// A chord the area does not own (`ctrl-c`): the pane passes it.
    Pass,
    /// `Esc`: close the area and drop the text.
    Cancel,
    /// Checked and ready; the area is already busy with it.
    Save(StoreRequest),
    /// Ctrl+E on the body (D7): the pane emits `Action::EditExternally`.
    External(ExternalEdit),
}

/// A document's kind field: typed for `a`, fixed for `v` (D9).
enum KindField {
    /// `a`: the user types the kind.
    Typed(TextField),
    /// `v`: the kind of the version the form came from.
    Fixed(String),
}

/// The fixed kind as it is (a store value, not prose); a typed one as its length.
impl core::fmt::Debug for KindField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Typed(field) => f.debug_struct("Typed").field("len", &field.len()).finish(),
            Self::Fixed(kind) => f.debug_tuple("Fixed").field(kind).finish(),
        }
    }
}

/// The open compose area (D6). `Debug` is hand-written: lengths, never text.
pub struct Compose {
    /// The item the note or document is for.
    item: ItemId,
    /// The item key for the temp-file stem, or `item` (E2).
    owner: String,
    /// The kind; `None` for a note.
    kind: Option<KindField>,
    /// The title; `None` for a note.
    title: Option<TextField>,
    /// The note or document body.
    body: TextArea,
    /// The focused part.
    focus: Part,
    /// The request in flight: `ADD_NOTE_NAME` or `WRITE_DOCUMENT_NAME`.
    busy: Option<&'static str>,
    /// The refusal, hedge or `$EDITOR` notice.
    notice: Option<String>,
    /// Whether the body is out with `$EDITOR` (E7: only the body is ever handed out).
    external: bool,
    /// The `(title, body)` of the version `v` opened, canonicalised as the store would keep
    /// them, for [`Compose::unchanged`]; `None` for a note, `a`, or `v` on a kind with no rows.
    base: Option<(String, String)>,
}

impl core::fmt::Debug for Compose {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Compose")
            .field("item", &self.item)
            .field("kind", &self.kind)
            .field("title_len", &self.title.as_ref().map(TextField::len))
            .field("body_len", &self.body.len())
            .field("focus", &self.focus)
            .field("busy", &self.busy)
            .field("notice_len", &self.notice.as_ref().map(String::len))
            .field("external", &self.external)
            .field("based", &self.base.is_some())
            .finish_non_exhaustive()
    }
}

impl Compose {
    /// `a` in Notes, after `NoteForm` answered: body only, focused, empty.
    #[must_use]
    pub fn note(item: ItemId, owner: Option<&str>) -> Self {
        Self {
            item,
            owner: owner.unwrap_or("item").to_owned(),
            kind: None,
            title: None,
            body: TextArea::new(),
            focus: Part::Body,
            busy: None,
            notice: None,
            external: false,
            base: None,
        }
    }

    /// `a` in Docs (`kind: None`: kind, title and body empty, focus Kind) or `v` (`kind: Some`:
    /// fixed; title and body from `base`, empty without one; focus Body).
    #[must_use]
    pub fn document(
        item: ItemId,
        owner: Option<&str>,
        kind: Option<String>,
        base: Option<&Document>,
    ) -> Self {
        let owner = owner.unwrap_or("item").to_owned();
        match kind {
            None => Self {
                item,
                owner,
                kind: Some(KindField::Typed(TextField::new())),
                title: Some(TextField::new()),
                body: TextArea::new(),
                focus: Part::Kind,
                busy: None,
                notice: None,
                external: false,
                base: None,
            },
            Some(kind) => {
                let (title, body) = base.map_or((String::new(), String::new()), |base| {
                    (base.title.clone(), base.body.clone())
                });
                Self {
                    item,
                    owner,
                    kind: Some(KindField::Fixed(kind)),
                    title: Some(TextField::with_text(&title)),
                    body: TextArea::with_text(&body),
                    focus: Part::Body,
                    busy: None,
                    notice: None,
                    external: false,
                    // Kept in the store's canonical form, as `unchanged` compares the typed
                    // side in it: a stored body ending in `\n` (the close-out `summary`) or a
                    // title with spaces still reads as unchanged when saved untouched.
                    base: base.map(|_| {
                        (
                            hand_written::document_title(&title).unwrap_or(title),
                            hand_written::document_body(&body).unwrap_or(body),
                        )
                    }),
                }
            }
        }
    }

    /// The request in flight, by name.
    #[must_use]
    pub fn busy(&self) -> Option<&'static str> {
        self.busy
    }

    /// The refusal, hedge or `$EDITOR` notice.
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The kind a save would send: the fixed one, or the typed text trimmed (`None` while blank).
    #[must_use]
    pub fn kind(&self) -> Option<String> {
        match self.kind.as_ref()? {
            KindField::Fixed(kind) => Some(kind.clone()),
            KindField::Typed(field) => {
                let kind = field.text().unwrap_or_default().trim();
                (!kind.is_empty()).then(|| kind.to_owned())
            }
        }
    }

    /// Whether the kind is typed (`a`): the kinds hint is drawn only then.
    #[must_use]
    pub fn kind_is_typed(&self) -> bool {
        matches!(self.kind, Some(KindField::Typed(_)))
    }

    /// The focused part.
    #[must_use]
    pub fn focus(&self) -> Part {
        self.focus
    }

    /// The title as typed; `None` for a note.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title
            .as_ref()
            .map(|title| title.text().unwrap_or_default())
    }

    /// The body as typed.
    #[must_use]
    pub fn body(&self) -> &str {
        self.body.text()
    }

    /// The plan's maintainer answer to blueprint §6 Q1: whether this is a `v` form whose title and
    /// body, as the store would keep them, equal the version it opened. A note, an `a` form and
    /// `v` on a kind with no rows are never unchanged; a title or body the validator refuses is
    /// not either (Ctrl+S says why). The Docs pane asks it on Ctrl+S, before
    /// [`on_key`](Compose::on_key), and refuses with `NOTHING_TO_SAVE` through
    /// [`settle`](Compose::settle).
    #[must_use]
    pub fn unchanged(&self) -> bool {
        let Some((title, body)) = &self.base else {
            return false;
        };
        let typed_title = self.title().unwrap_or_default();
        hand_written::document_title(typed_title).is_ok_and(|typed| typed == *title)
            && hand_written::document_body(self.body.text()).is_ok_and(|typed| typed == *body)
    }

    /// D10, review M1: a write's answer was lost. The notice says so while the pane re-reads to
    /// look for the row, and the area stays busy with the write, so Ctrl+S cannot send it twice
    /// and `Esc` cannot drop the text before the re-read [`settle`](Compose::settle)s it.
    pub fn checking(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    /// A reply ended the flight: busy cleared, the notice set or cleared.
    pub fn settle(&mut self, notice: Option<String>) {
        self.busy = None;
        self.notice = notice;
    }

    /// The parts `Tab` cycles: the body alone for a note, title and body under a fixed kind, all
    /// three under a typed one.
    fn parts(&self) -> &'static [Part] {
        match self.kind {
            None => &[Part::Body],
            Some(KindField::Fixed(_)) => &[Part::Title, Part::Body],
            Some(KindField::Typed(_)) => &[Part::Kind, Part::Title, Part::Body],
        }
    }

    /// One step through [`parts`](Self::parts), wrapping.
    fn move_focus(&mut self, forward: bool) {
        let parts = self.parts();
        let at = parts
            .iter()
            .position(|part| *part == self.focus)
            .unwrap_or(0);
        let len = parts.len();
        let next = if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        };
        self.focus = parts[next];
    }

    /// The one-line field behind `part`: a typed kind or the title; `None` for the body or a
    /// fixed kind.
    fn field_mut(&mut self, part: Part) -> Option<&mut TextField> {
        match part {
            Part::Kind => match self.kind.as_mut() {
                Some(KindField::Typed(field)) => Some(field),
                Some(KindField::Fixed(_)) | None => None,
            },
            Part::Title => self.title.as_mut(),
            Part::Body => None,
        }
    }

    /// Feeds one key: Ctrl+S, Ctrl+E, any other chord, busy, `Tab`/`BackTab`, then the focused
    /// part (the module doc's order).
    pub fn on_key(&mut self, key: KeyEvent) -> ComposeOutcome {
        if ctrl_s(&key) {
            return match self.busy {
                Some(_) => ComposeOutcome::Stay,
                None => self.save(),
            };
        }
        // Before the chord pass, which would hand it to the pane (the item form's rule).
        if ctrl_e(&key) {
            return match (self.busy, self.focus) {
                (None, Part::Body) => self.hand_off(),
                // E7: only the body goes out.
                _ => ComposeOutcome::Stay,
            };
        }
        if key.modifiers.intersects(CHORD) {
            return ComposeOutcome::Pass;
        }
        if self.busy.is_some() {
            return ComposeOutcome::Stay;
        }
        match key.code {
            KeyCode::Tab => {
                self.move_focus(true);
                return ComposeOutcome::Stay;
            }
            KeyCode::BackTab => {
                self.move_focus(false);
                return ComposeOutcome::Stay;
            }
            _ => {}
        }
        let focus = self.focus;
        if let Some(field) = self.field_mut(focus) {
            match field.on_key(key) {
                // The item form's D8: `Enter` moves on; only Ctrl+S saves.
                FieldOutcome::Submit => self.move_focus(true),
                FieldOutcome::Cancel => return ComposeOutcome::Cancel,
                FieldOutcome::Pass => match key.code {
                    KeyCode::Up => self.move_focus(false),
                    KeyCode::Down => self.move_focus(true),
                    _ => {}
                },
                FieldOutcome::Consumed => {}
            }
            return ComposeOutcome::Stay;
        }
        match self.body.on_key(key, PAGE) {
            FieldOutcome::Cancel => ComposeOutcome::Cancel,
            FieldOutcome::Consumed | FieldOutcome::Submit | FieldOutcome::Pass => {
                ComposeOutcome::Stay
            }
        }
    }

    /// Ctrl+S while idle: the validator first, in the area (D4); a refusal is the notice and
    /// focuses its part, else the canonical request goes out and the area is busy with it.
    fn save(&mut self) -> ComposeOutcome {
        let Some(kind) = &self.kind else {
            return match hand_written::note_body(self.body.text()) {
                Ok(body) => {
                    self.busy = Some(ADD_NOTE_NAME);
                    ComposeOutcome::Save(StoreRequest::AddNote {
                        item: self.item,
                        body: HandText::new(body),
                    })
                }
                Err(refusal) => {
                    self.notice = Some(refusal.to_string());
                    ComposeOutcome::Stay
                }
            };
        };
        let kind = match kind {
            KindField::Fixed(kind) => kind.clone(),
            KindField::Typed(field) => field.text().unwrap_or_default().to_owned(),
        };
        let checked = hand_written::document_kind(&kind)
            .map_err(|refusal| (Part::Kind, refusal))
            .and_then(|kind| {
                hand_written::document_title(self.title().unwrap_or_default())
                    .map(|title| (kind, title))
                    .map_err(|refusal| (Part::Title, refusal))
            })
            .and_then(|(kind, title)| {
                hand_written::document_body(self.body.text())
                    .map(|body| (kind, title, body))
                    .map_err(|refusal| (Part::Body, refusal))
            });
        match checked {
            Ok((kind, title, body)) => {
                self.busy = Some(WRITE_DOCUMENT_NAME);
                ComposeOutcome::Save(StoreRequest::WriteDocument {
                    item: self.item,
                    kind,
                    title,
                    body: HandText::new(body),
                })
            }
            Err((part, refusal)) => {
                self.notice = Some(refusal.to_string());
                // A fixed kind came from the store and never fails; were it to, the focus stays.
                if self.parts().contains(&part) {
                    self.focus = part;
                }
                ComposeOutcome::Stay
            }
        }
    }

    /// Ctrl+E on the body while idle (D7): hand it out under `{owner}-note` or `{owner}-{kind}`
    /// (`document` while the kind is blank); `editor::run` sanitises the stem. The notice is left
    /// alone: the outcome always sets it.
    fn hand_off(&mut self) -> ComposeOutcome {
        let part = match &self.kind {
            None => "note".to_owned(),
            Some(_) => self.kind().unwrap_or_else(|| "document".to_owned()),
        };
        self.external = true;
        ComposeOutcome::External(ExternalEdit {
            text: self.body.text().to_owned(),
            stem: format!("{}-{part}", self.owner),
        })
    }

    /// Into the focused field; dropped while busy or on a fixed kind.
    pub fn on_paste(&mut self, text: &str) {
        if self.busy.is_some() {
            return;
        }
        let focus = self.focus;
        match self.field_mut(focus) {
            // An unmasked field never refuses a paste.
            Some(field) => _ = field.on_paste(text),
            None if focus == Part::Body => self.body.on_paste(text),
            None => {}
        }
    }

    /// D7: the item form's `on_external_edit` rules, for the body. Ignored unless the body was
    /// handed out.
    pub fn on_external_edit(&mut self, outcome: ExternalEditOutcome) {
        if !std::mem::take(&mut self.external) {
            return;
        }
        match outcome {
            ExternalEditOutcome::Edited(returned) => {
                // Filtered first (the item form's review L2), so a return that differs only by
                // control characters reads as unchanged.
                let returned = without_controls(&returned);
                let text = strip_added_newline(self.body.text(), &returned);
                if text == self.body.text() {
                    self.notice = Some(NO_CHANGES.to_owned());
                } else {
                    self.body = TextArea::with_text(text);
                    self.body.set_cursor(usize::MAX);
                    self.focus = Part::Body;
                    self.notice = Some(EDITED.to_owned());
                }
            }
            ExternalEditOutcome::Unchanged { quick } => {
                let wait = if quick { WAIT_FLAG } else { "" };
                self.notice = Some(format!("{NO_CHANGES}{wait}"));
            }
            ExternalEditOutcome::Failed(message) => self.notice = Some(message),
        }
    }

    /// One one-line row: the marker, the label, and the field (or a fixed kind, dim).
    fn row_line(&self, part: Part, width: u16, theme: &Theme) -> Line<'static> {
        let focused = self.focus == part;
        let label = if part == Part::Kind { "kind" } else { "title" };
        let head = format!("{}{label:<LABEL$}", marker(focused));
        let head_style = if focused { theme.title } else { theme.dim };
        let all = usize::from(width);
        let room = all.saturating_sub(cell_width(&head));
        let mut spans = vec![Span::styled(cells::clip(&head, all), head_style)];
        let field = match part {
            Part::Kind => match &self.kind {
                Some(KindField::Typed(field)) => Some(field),
                Some(KindField::Fixed(kind)) => {
                    spans.push(Span::styled(cells::clip(kind, room), theme.dim));
                    None
                }
                None => None,
            },
            Part::Title => self.title.as_ref(),
            Part::Body => None,
        };
        if let Some(field) = field {
            let room = u16::try_from(room).unwrap_or(u16::MAX);
            spans.extend(field.line(room, focused, theme).spans);
        }
        Line::from(spans)
    }
}

/// D10: a write `Failed` that may have followed a COMMIT whose answer was lost (the mint hedge's
/// opening, `item_form::mint_may_have_landed`); `reread` is `thread` or `list`. Review M1: the
/// area covers the pane, so the user cannot look; the pane does, and the sentence stays only
/// while that re-read is in flight.
#[must_use]
pub fn may_have_landed(why: &str, reread: &str) -> String {
    format!("{why} \u{2014} it may have been written; the {reread} is being re-read to check")
}

/// Review M1: the re-read after [`may_have_landed`] holds no such row, so nothing was written and
/// a retry writes it once.
#[must_use]
pub fn not_written(why: &str) -> String {
    format!("{why} \u{2014} the re-read shows it was not written; Ctrl+S tries again")
}

/// Review M1: the re-read after [`may_have_landed`] failed too, so whether it landed is unknown.
#[must_use]
pub fn could_not_check(why: &str, reread: &str) -> String {
    format!(
        "{why} \u{2014} it may have been written, and the {reread} could not be re-read to check; \
         Ctrl+S may write it twice"
    )
}

/// Draws the area over the sub-tab's whole content rect (D6): a top rule titled `title`, the
/// one-line rows, the body, the notice and the hint. `kinds` is the Docs pane's hint text,
/// drawn under a typed kind only.
///
/// Returns the body's rect, which the pane claims for an in-pane editor (MOD-57 P2).
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    compose: &Compose,
    title: &str,
    kinds: Option<&str>,
    theme: &Theme,
) -> Rect {
    let block = Block::new()
        .borders(Borders::TOP)
        .title(cells::clip(title, usize::from(area.width)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width;
    let all = usize::from(width);

    // The one-line rows above the body, each with what it draws.
    let mut rows: Vec<Line<'static>> = Vec::new();
    if compose.kind.is_some() {
        rows.push(compose.row_line(Part::Kind, width, theme));
        if let (true, Some(kinds)) = (compose.kind_is_typed(), kinds) {
            rows.push(Line::styled(
                cells::clip(&format!("  kinds: {kinds}"), all),
                theme.dim,
            ));
        }
        rows.push(compose.row_line(Part::Title, width, theme));
        let focused = compose.focus == Part::Body;
        rows.push(Line::styled(
            cells::clip(&format!("{}body", marker(focused)), all),
            if focused { theme.title } else { theme.dim },
        ));
    }
    // The item form's notice rule: wrapped here, so the layout sizes it from the same count;
    // it grows into the body's rows, and one longer than all of them loses its first rows (the
    // D10 hedge ends the sentence).
    let mut notice_lines = compose
        .notice
        .as_deref()
        .map(|sentence| wrapped(sentence, all))
        .unwrap_or_default();
    let fixed = u16::try_from(rows.len())
        .unwrap_or(u16::MAX)
        .saturating_add(1);
    let room = inner.height.saturating_sub(fixed);
    let wanted = u16::try_from(notice_lines.len()).unwrap_or(u16::MAX);
    let notice_height = wanted.min(room);
    let body_min = BODY_MIN.min(room.saturating_sub(notice_height));
    if wanted > notice_height {
        notice_lines.drain(..usize::from(wanted - notice_height));
    }
    let mut constraints: Vec<Constraint> = rows.iter().map(|_| Constraint::Length(1)).collect();
    constraints.extend([
        Constraint::Min(body_min),
        Constraint::Length(notice_height),
        Constraint::Length(1),
    ]);
    let areas = Layout::vertical(constraints).split(inner);
    let count = rows.len();
    for (line, at) in rows.into_iter().zip(areas.iter()) {
        frame.render_widget(Paragraph::new(line), *at);
    }
    let [body, notice, hint] = [areas[count], areas[count + 1], areas[count + 2]];
    frame.render_widget(
        Paragraph::new(compose.body.lines(
            body.width,
            body.height,
            compose.focus == Part::Body,
            theme,
        )),
        body,
    );
    frame.render_widget(
        Paragraph::new(
            notice_lines
                .into_iter()
                .map(|line| Line::styled(line, theme.error))
                .collect::<Vec<_>>(),
        ),
        notice,
    );
    let text = match compose.focus {
        Part::Kind | Part::Title => HINT_TEXT,
        Part::Body => HINT_AREA,
    };
    frame.render_widget(
        Paragraph::new(Line::styled(cells::clip(text, all), theme.dim)),
        hint,
    );
    body
}

/// The compose, Notes and Docs panes' shared test bench (blueprint E8): a `Ctx` kept alive for a
/// test, the actions it gathered, keys, and a drawn buffer as text.
#[cfg(test)]
pub(super) mod bench {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::model::{ProjectRef, Scope, WorkspaceId};
    use ratatui::Frame;
    use ratatui::layout::Rect;

    use crate::app::{Action, Ctx, Emit, TopBarState};
    use crate::keymap::Keymap;
    use crate::store_worker::Origin;
    use crate::ui::Theme;
    use crate::ui::tabs::BacklogTab;

    /// Everything a `Ctx` borrows, kept alive for the length of a test (`requirements.rs`'
    /// `Shell`).
    pub struct Shell {
        scope: Scope,
        projects: Vec<ProjectRef>,
        top_bar: TopBarState,
        keymap: Keymap,
        /// The palette, for the panes' `lines`.
        pub theme: Theme,
        emit: Emit,
    }

    impl Shell {
        /// An empty scope with the default keymap and theme.
        pub fn new() -> Self {
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

        /// The Backlog's `Ctx`.
        pub fn ctx(&self) -> Ctx<'_> {
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

        /// Everything emitted since the last take.
        pub fn actions(&self) -> Vec<Action> {
            self.emit.take()
        }

        /// The store requests emitted since the last take, each as its `Debug` (`StoreRequest`
        /// has no `PartialEq`); every other action is dropped.
        pub fn requests(&self) -> Vec<String> {
            self.actions()
                .into_iter()
                .filter_map(|action| match action {
                    Action::Store(request) => Some(format!("{request:?}")),
                    _ => None,
                })
                .collect()
        }
    }

    /// A plain key.
    pub fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// `Ctrl` plus `c`.
    pub fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// `draw` into a `width` x `height` test terminal, its buffer as text, rows joined by `\n`.
    pub fn drawn(width: u16, height: u16, draw: impl FnOnce(&mut Frame<'_>, Rect)) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, frame.area()))
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

    /// Row `rect.y` of a [`drawn`] text from cell column `rect.x` on (one char per cell, which
    /// holds for the ASCII and box-drawing rows these tests read).
    pub fn row_from(text: &str, rect: Rect) -> String {
        text.lines()
            .nth(usize::from(rect.y))
            .unwrap_or_default()
            .chars()
            .skip(usize::from(rect.x))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;
    use htui_core::fixtures::ids;
    use htui_core::model::DocumentId;

    use super::bench::{ctrl, drawn, key};
    use super::*;

    fn note() -> Compose {
        Compose::note(ids::HTUI_FEAT_1, Some("FEAT-1"))
    }

    /// A typed (`a`) document form on `owner`.
    fn typed(owner: &str) -> Compose {
        Compose::document(ids::HTUI_FEAT_1, Some(owner), None, None)
    }

    /// A `plan` v2 row with a title and body.
    fn base() -> Document {
        Document {
            id: DocumentId::new(),
            item_id: ids::HTUI_FEAT_1,
            kind: "plan".to_owned(),
            version: 2,
            title: "Plan".to_owned(),
            body: "# Plan\n\nBody.".to_owned(),
            produced_by_step_id: None,
            created_by: ids::USER,
            created_at: chrono::Utc::now(),
        }
    }

    fn type_text(compose: &mut Compose, text: &str) {
        for c in text.chars() {
            assert!(matches!(
                compose.on_key(key(KeyCode::Char(c))),
                ComposeOutcome::Stay
            ));
        }
    }

    #[test]
    fn ctrl_s_on_a_blank_note_refuses_in_the_area_and_sends_nothing() {
        let mut compose = note();
        assert!(matches!(compose.on_key(ctrl('s')), ComposeOutcome::Stay));
        assert_eq!(compose.notice(), Some("a note needs text"));
        assert_eq!(compose.busy(), None);
    }

    #[test]
    fn ctrl_s_sends_add_note_with_the_trimmed_text_and_is_busy() {
        let mut compose = note();
        compose.on_paste("Hi.\n\n");
        match compose.on_key(ctrl('s')) {
            ComposeOutcome::Save(StoreRequest::AddNote { item, body }) => {
                assert_eq!(item, ids::HTUI_FEAT_1);
                assert_eq!(body.as_str(), "Hi.");
            }
            other => panic!("expected AddNote, got {other:?}"),
        }
        assert_eq!(compose.busy(), Some(ADD_NOTE_NAME));
    }

    #[test]
    fn esc_cancels() {
        assert!(matches!(
            note().on_key(key(KeyCode::Esc)),
            ComposeOutcome::Cancel
        ));
    }

    #[test]
    fn ctrl_c_and_other_chords_pass() {
        let mut compose = note();
        assert!(matches!(compose.on_key(ctrl('c')), ComposeOutcome::Pass));
        let alt = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert!(matches!(compose.on_key(alt), ComposeOutcome::Pass));
    }

    #[test]
    fn while_busy_every_plain_key_is_swallowed() {
        let mut compose = note();
        compose.on_paste("Hi.\n\n");
        assert!(matches!(compose.on_key(ctrl('s')), ComposeOutcome::Save(_)));
        for key in [
            key(KeyCode::Char('x')),
            key(KeyCode::Esc),
            ctrl('s'),
            ctrl('e'),
        ] {
            assert!(
                matches!(compose.on_key(key), ComposeOutcome::Stay),
                "{key:?}"
            );
        }
        assert_eq!(compose.body(), "Hi.\n\n");
        // A paste is dropped too.
        compose.on_paste("more");
        assert_eq!(compose.body(), "Hi.\n\n");
    }

    #[test]
    fn ctrl_e_on_the_body_hands_it_out_with_the_note_stem() {
        let mut compose = note();
        compose.on_paste("draft");
        match compose.on_key(ctrl('e')) {
            ComposeOutcome::External(edit) => assert_eq!(
                edit,
                ExternalEdit {
                    text: "draft".to_owned(),
                    stem: "FEAT-1-note".to_owned(),
                }
            ),
            other => panic!("expected External, got {other:?}"),
        }
        let mut keyless = Compose::note(ids::HTUI_FEAT_1, None);
        match keyless.on_key(ctrl('e')) {
            ComposeOutcome::External(edit) => assert_eq!(edit.stem, "item-note"),
            other => panic!("expected External, got {other:?}"),
        }
    }

    #[test]
    fn an_edited_outcome_lands_in_the_body_with_the_cursor_at_the_end() {
        let mut compose = note();
        assert!(matches!(
            compose.on_key(ctrl('e')),
            ComposeOutcome::External(_)
        ));
        compose.on_external_edit(ExternalEditOutcome::Edited("New.\n".to_owned()));
        assert_eq!(compose.body(), "New.");
        assert_eq!(compose.body.cursor(), 4);
        assert_eq!(compose.notice(), Some(EDITED));

        assert!(matches!(
            compose.on_key(ctrl('e')),
            ComposeOutcome::External(_)
        ));
        compose.on_external_edit(ExternalEditOutcome::Edited("New.\n".to_owned()));
        assert_eq!(compose.notice(), Some(NO_CHANGES));
        assert_eq!(compose.body(), "New.");
    }

    #[test]
    fn unchanged_and_failed_keep_the_text() {
        let mut compose = note();
        compose.on_paste("kept");
        let _ = compose.on_key(ctrl('e'));
        compose.on_external_edit(ExternalEditOutcome::Unchanged { quick: true });
        assert_eq!(
            compose.notice(),
            Some(format!("{NO_CHANGES}{WAIT_FLAG}").as_str())
        );
        assert_eq!(compose.body(), "kept");

        let _ = compose.on_key(ctrl('e'));
        compose.on_external_edit(ExternalEditOutcome::Failed("boom".to_owned()));
        assert_eq!(compose.notice(), Some("boom"));
        assert_eq!(compose.body(), "kept");
    }

    #[test]
    fn an_outcome_with_nothing_handed_out_is_ignored() {
        let mut compose = note();
        compose.on_paste("kept");
        compose.on_external_edit(ExternalEditOutcome::Edited("other".to_owned()));
        assert_eq!(compose.body(), "kept");
        assert_eq!(compose.notice(), None);

        // One hand-off, one outcome: a second outcome is ignored.
        let _ = compose.on_key(ctrl('e'));
        compose.on_external_edit(ExternalEditOutcome::Failed("boom".to_owned()));
        compose.on_external_edit(ExternalEditOutcome::Edited("other".to_owned()));
        assert_eq!(compose.body(), "kept");
        assert_eq!(compose.notice(), Some("boom"));
    }

    #[test]
    fn a_typed_document_cycles_kind_title_body_and_a_fixed_kind_is_skipped() {
        let mut compose = typed("FEAT-1");
        assert_eq!(compose.focus(), Part::Kind);
        assert!(compose.kind_is_typed());
        for want in [Part::Title, Part::Body, Part::Kind] {
            assert!(matches!(
                compose.on_key(key(KeyCode::Tab)),
                ComposeOutcome::Stay
            ));
            assert_eq!(compose.focus(), want);
        }
        let _ = compose.on_key(key(KeyCode::BackTab));
        assert_eq!(compose.focus(), Part::Body);
        // Enter on a one-line field moves on; Down too.
        let _ = compose.on_key(key(KeyCode::Tab));
        let _ = compose.on_key(key(KeyCode::Enter));
        assert_eq!(compose.focus(), Part::Title);
        let _ = compose.on_key(key(KeyCode::Down));
        assert_eq!(compose.focus(), Part::Body);

        let base = base();
        let mut fixed = Compose::document(
            ids::HTUI_FEAT_1,
            Some("FEAT-1"),
            Some("plan".to_owned()),
            Some(&base),
        );
        assert_eq!(fixed.focus(), Part::Body);
        assert!(!fixed.kind_is_typed());
        assert_eq!(fixed.kind(), Some("plan".to_owned()));
        assert_eq!(fixed.title(), Some("Plan"));
        assert_eq!(fixed.body(), "# Plan\n\nBody.");
        for want in [Part::Title, Part::Body, Part::Title] {
            let _ = fixed.on_key(key(KeyCode::Tab));
            assert_eq!(fixed.focus(), want);
        }

        // A note has the body alone.
        let mut note = note();
        let _ = note.on_key(key(KeyCode::Tab));
        assert_eq!(note.focus(), Part::Body);
        assert_eq!(note.title(), None);
        assert_eq!(note.kind(), None);
    }

    #[test]
    fn ctrl_e_on_the_kind_or_title_is_swallowed() {
        let mut compose = typed("X");
        type_text(&mut compose, "code review");
        assert!(matches!(compose.on_key(ctrl('e')), ComposeOutcome::Stay));
        let _ = compose.on_key(key(KeyCode::Tab));
        assert!(matches!(compose.on_key(ctrl('e')), ComposeOutcome::Stay));
        let _ = compose.on_key(key(KeyCode::Tab));
        match compose.on_key(ctrl('e')) {
            ComposeOutcome::External(edit) => assert_eq!(edit.stem, "X-code review"),
            other => panic!("expected External, got {other:?}"),
        }
        // A blank kind names the stem `document`.
        let mut blank = typed("X");
        let _ = blank.on_key(key(KeyCode::BackTab));
        match blank.on_key(ctrl('e')) {
            ComposeOutcome::External(edit) => assert_eq!(edit.stem, "X-document"),
            other => panic!("expected External, got {other:?}"),
        }
    }

    #[test]
    fn a_document_refusal_focuses_its_part() {
        let mut compose = typed("FEAT-1");
        type_text(&mut compose, "plan");
        let _ = compose.on_key(key(KeyCode::Tab));
        let _ = compose.on_key(key(KeyCode::Tab));
        compose.on_paste("Body.");
        assert_eq!(compose.focus(), Part::Body);
        assert!(matches!(compose.on_key(ctrl('s')), ComposeOutcome::Stay));
        assert_eq!(compose.focus(), Part::Title);
        assert!(compose.notice().is_some_and(|n| n.contains("title")));
        assert_eq!(compose.busy(), None);

        // A blank kind is refused first, on the kind.
        let mut blank = typed("FEAT-1");
        let _ = blank.on_key(key(KeyCode::BackTab));
        assert!(matches!(blank.on_key(ctrl('s')), ComposeOutcome::Stay));
        assert_eq!(blank.focus(), Part::Kind);
        assert!(blank.notice().is_some_and(|n| n.contains("kind")));

        // All three good: the canonical values go out.
        type_text(&mut compose, " Plan ");
        let _ = compose.on_key(key(KeyCode::Tab));
        match compose.on_key(ctrl('s')) {
            ComposeOutcome::Save(StoreRequest::WriteDocument {
                item,
                kind,
                title,
                body,
            }) => {
                assert_eq!(item, ids::HTUI_FEAT_1);
                assert_eq!(kind, "plan");
                assert_eq!(title, "Plan");
                assert_eq!(body.as_str(), "Body.");
            }
            other => panic!("expected WriteDocument, got {other:?}"),
        }
        assert_eq!(compose.busy(), Some(WRITE_DOCUMENT_NAME));
        compose.settle(Some("why".to_owned()));
        assert_eq!(compose.busy(), None);
        assert_eq!(compose.notice(), Some("why"));
    }

    #[test]
    fn a_paste_goes_to_the_focused_field() {
        let mut compose = typed("FEAT-1");
        compose.on_paste("plan");
        let _ = compose.on_key(key(KeyCode::Tab));
        compose.on_paste("Title");
        let _ = compose.on_key(key(KeyCode::Tab));
        compose.on_paste("Body\nline");
        assert_eq!(compose.kind(), Some("plan".to_owned()));
        assert_eq!(compose.title(), Some("Title"));
        assert_eq!(compose.body(), "Body\nline");

        // A fixed kind takes no paste (`Tab` never reaches it, so the focus is put there by
        // hand), and a note's body does.
        let base = base();
        let mut fixed =
            Compose::document(ids::HTUI_FEAT_1, None, Some("plan".to_owned()), Some(&base));
        fixed.focus = Part::Kind;
        fixed.on_paste("review");
        assert_eq!(fixed.kind(), Some("plan".to_owned()));
        assert_eq!(fixed.title(), Some("Plan"));
        assert_eq!(fixed.body(), "# Plan\n\nBody.");
        let mut note = note();
        note.on_paste("x");
        assert_eq!(note.body(), "x");
    }

    #[test]
    fn debug_prints_no_text() {
        let mut compose = typed("FEAT-1");
        compose.on_paste("plan");
        let _ = compose.on_key(key(KeyCode::Tab));
        compose.on_paste("SECRET title");
        let _ = compose.on_key(key(KeyCode::Tab));
        compose.on_paste("SECRET body");
        for shown in [format!("{compose:?}"), format!("{compose:#?}")] {
            assert!(!shown.contains("SECRET"), "{shown}");
            assert!(shown.contains("body_len"), "{shown}");
        }
        let base = base();
        let fixed = Compose::document(ids::HTUI_FEAT_1, None, Some("plan".to_owned()), Some(&base));
        let shown = format!("{fixed:?}");
        assert!(!shown.contains("Body."), "{shown}");
        assert!(shown.contains("plan"), "{shown}");
    }

    #[test]
    fn the_area_draws_its_hint_inside_the_pane() {
        let compose = note();
        let text = drawn(43, 23, |frame, area| {
            render(frame, area, &compose, " New note ", None, &Theme::default());
        });
        assert!(text.contains(" New note "), "{text}");
        assert!(text.contains(HINT_AREA), "{text}");

        let compose = typed("FEAT-1");
        let text = drawn(43, 23, |frame, area| {
            render(
                frame,
                area,
                &compose,
                " New document ",
                Some("plan, prd, summary"),
                &Theme::default(),
            );
        });
        assert!(text.contains(HINT_TEXT), "{text}");
        assert!(text.contains("kinds: plan"), "{text}");
        assert!(text.contains("> kind"), "{text}");
        assert!(text.contains("  title"), "{text}");

        // A fixed kind draws no kinds hint, and every row stays inside the pane.
        let base = base();
        let fixed = Compose::document(ids::HTUI_FEAT_1, None, Some("plan".to_owned()), Some(&base));
        let text = drawn(43, 23, |frame, area| {
            render(
                frame,
                area,
                &fixed,
                " New version of plan (from v2) ",
                Some("plan, prd, summary"),
                &Theme::default(),
            );
        });
        assert!(!text.contains("kinds:"), "{text}");
        assert!(text.contains("  kind   plan"), "{text}");
        assert!(text.contains("  title  Plan"), "{text}");
        assert!(text.contains("> body"), "{text}");
        for row in text.lines() {
            assert!(cell_width(row) <= 43, "{row:?}");
        }
    }

    /// MOD-57 P2 (PD-3): `render` returns the rect its body is drawn in, which the pane claims
    /// for an in-pane editor: under the top rule for a note, under the kind, kinds, title and
    /// body-label rows for a typed document, and above the hint either way.
    #[test]
    fn render_returns_the_body_rect() {
        let mut compose = note();
        compose.on_paste("First line.\nSecond.");
        let mut body = None;
        let text = drawn(43, 23, |frame, area| {
            body = Some(render(
                frame,
                area,
                &compose,
                " New note ",
                None,
                &Theme::default(),
            ));
        });
        let body = body.expect("the frame drew");
        assert_eq!(body, Rect::new(0, 1, 43, 21));
        assert!(
            bench::row_from(&text, body).starts_with("First line."),
            "{body:?} in\n{text}"
        );

        let mut compose = typed("FEAT-1");
        let _ = compose.on_key(key(KeyCode::Tab));
        let _ = compose.on_key(key(KeyCode::Tab));
        assert_eq!(compose.focus(), Part::Body);
        compose.on_paste("First line.");
        let mut body = None;
        let text = drawn(43, 23, |frame, area| {
            body = Some(render(
                frame,
                area,
                &compose,
                " New document ",
                Some("plan, prd, summary"),
                &Theme::default(),
            ));
        });
        let body = body.expect("the frame drew");
        assert_eq!(body, Rect::new(0, 5, 43, 17));
        assert!(
            bench::row_from(&text, body).starts_with("First line."),
            "{body:?} in\n{text}"
        );
        let above = Rect {
            y: body.y - 1,
            ..body
        };
        assert!(
            bench::row_from(&text, above).starts_with("> body"),
            "{text}"
        );
    }

    /// A long notice keeps its end (the hedge) and the hint.
    #[test]
    fn a_long_notice_keeps_its_last_rows_and_the_hint() {
        let mut compose = note();
        compose.settle(Some(may_have_landed(
            &"store backend error: connection reset ".repeat(8),
            "thread",
        )));
        let text = drawn(43, 8, |frame, area| {
            render(frame, area, &compose, " New note ", None, &Theme::default());
        });
        assert!(text.contains("re-read to check"), "{text}");
        assert!(text.contains(HINT_AREA), "{text}");
    }

    /// The maintainer's answer to blueprint §6 Q1: `v` with nothing changed is `unchanged`, in
    /// the store's canonical form; any change, `a`, or a note is not.
    #[test]
    fn unchanged_is_a_v_form_whose_title_and_body_are_the_base_s() {
        let base = base();
        let mut fixed =
            Compose::document(ids::HTUI_FEAT_1, None, Some("plan".to_owned()), Some(&base));
        assert!(fixed.unchanged());
        // A trailing newline the store would trim is no change.
        fixed.body.set_cursor(usize::MAX);
        fixed.on_paste("\n\n");
        assert!(fixed.unchanged());
        fixed.on_paste("More.");
        assert!(!fixed.unchanged());

        let mut retitled =
            Compose::document(ids::HTUI_FEAT_1, None, Some("plan".to_owned()), Some(&base));
        let _ = retitled.on_key(key(KeyCode::Tab));
        type_text(&mut retitled, " 2");
        assert!(!retitled.unchanged());

        // No base, an `a` form and a note are never unchanged.
        let fresh = Compose::document(ids::HTUI_FEAT_1, None, Some("review".to_owned()), None);
        assert!(!fresh.unchanged());
        assert!(!typed("FEAT-1").unchanged());
        assert!(!note().unchanged());
    }

    /// A stored version the store did not trim (the close-out `summary` ends in `\n`, a title may
    /// carry spaces) still reads as unchanged when opened and saved untouched: both sides are
    /// compared in the store's canonical form.
    #[test]
    fn unchanged_canonicalises_the_base_too() {
        let mut base = base();
        base.kind = "summary".to_owned();
        base.title = "  Summary ".to_owned();
        base.body = "| a | b |\n|---|---|\n".to_owned();
        let mut opened = Compose::document(
            ids::HTUI_FEAT_1,
            None,
            Some("summary".to_owned()),
            Some(&base),
        );
        assert!(opened.unchanged());
        opened.body.set_cursor(usize::MAX);
        opened.on_paste("More.");
        assert!(!opened.unchanged());
    }

    /// The bench itself: `requests` keeps the store requests as their `Debug` and drops the
    /// rest, and a take empties the sink.
    #[test]
    fn the_bench_keeps_store_requests_and_drops_the_rest() {
        let shell = bench::Shell::new();
        let ctx = shell.ctx();
        ctx.request(StoreRequest::Notes(ids::HTUI_FEAT_1));
        ctx.emit(crate::app::Action::Error("boom".to_owned()));
        assert_eq!(
            shell.requests(),
            [format!("{:?}", StoreRequest::Notes(ids::HTUI_FEAT_1))]
        );
        assert!(shell.actions().is_empty());
        let _ = &shell.theme;
    }

    #[test]
    fn may_have_landed_names_what_is_re_read() {
        let hedge = may_have_landed("boom", "thread");
        assert!(hedge.starts_with("boom \u{2014} it may have been written"));
        assert!(hedge.contains("the thread is being re-read"));
        // Review M1: the pane looks, not the user, who cannot see past the area.
        assert!(!hedge.contains("look for it"), "{hedge}");
    }

    /// Review M1: the two ways the re-read settles a hedge it could not decide by itself.
    #[test]
    fn not_written_and_could_not_check_say_what_ctrl_s_does() {
        assert_eq!(
            not_written("boom"),
            "boom \u{2014} the re-read shows it was not written; Ctrl+S tries again"
        );
        let unknown = could_not_check("boom", "list");
        assert!(unknown.starts_with("boom \u{2014} it may have been written"));
        assert!(unknown.contains("the list could not be re-read"));
        assert!(unknown.ends_with("Ctrl+S may write it twice"));
    }

    /// Review M1: while the re-read checks, the area keeps the write busy: the hedge shows, and
    /// Ctrl+S, `Esc` and a paste are swallowed until a settle.
    #[test]
    fn checking_keeps_the_area_busy_with_the_hedge() {
        let mut compose = note();
        compose.on_paste("Hi.");
        assert!(matches!(compose.on_key(ctrl('s')), ComposeOutcome::Save(_)));
        compose.checking(may_have_landed("boom", "thread"));
        assert_eq!(compose.busy(), Some(ADD_NOTE_NAME));
        assert_eq!(
            compose.notice(),
            Some(may_have_landed("boom", "thread").as_str())
        );
        for key in [ctrl('s'), key(KeyCode::Esc)] {
            assert!(matches!(compose.on_key(key), ComposeOutcome::Stay));
        }
        compose.on_paste("more");
        assert_eq!(compose.body(), "Hi.");
        compose.settle(Some(not_written("boom")));
        assert_eq!(compose.busy(), None);
        assert!(matches!(compose.on_key(ctrl('s')), ComposeOutcome::Save(_)));
    }
}
