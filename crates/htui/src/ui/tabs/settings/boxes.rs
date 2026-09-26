//! `Settings > Boxes` (MOD-7 milestone 2, PRD D4, `R-TUI-8`): every box of this user with its
//! profile, tools, probed and declared tags, quirks and last probe; declared tags and quirks
//! edited as a compare-and-set a reconnect cannot stale; the probe on this box only.
//!
//! Holds no store handle and mints no id (`R-NF-3`): every `BoxId` it sends came out of a
//! [`BoxesSnapshot`].
//!
//! What is on screen is always the last snapshot the worker assembled
//! ([`crate::box_settings::serve`]): no row is ever patched in locally, so there is exactly one
//! source of truth (the rule [`crate::prompt_settings`] states in its module doc).
//!
//! Known residue, the kinds section's H-9 (D56, R-29): a `Boxes` reply carries nothing that says
//! which request it answers, so `busy` is the whole of the attribution. A read that lands between
//! a save and its reply (a scope change or a tab re-activation) is taken for that reply and closes
//! the editor early; if the save then comes back `BoxesStale`, [`CHANGED_ELSEWHERE_CLOSED`] says
//! nothing was written and how to retry.

use htui_core::model::{BoxId, BoxRecord, Scope, canonical_declared_tags, declared_tags_from_text};

use crate::app::{Ctx, Handled};
use crate::box_settings::{BoxesSnapshot, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
};
use crate::ui::{FieldOutcome, TextArea, TextField};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::BoxEdit;
use ratatui::Frame;
use ratatui::layout::Rect;

/// A second save while the first is in flight (D56).
const IN_FLIGHT: &str = "edit_box in flight";

/// `p` on a box that is not this one (PRD D4).
const THIS_BOX_ONLY: &str = "the probe runs on this box only";

/// The name `busy` carries while an `EditBox` is in flight: the request's own name.
const EDIT_NAME: &str = REQUEST_NAMES[1];

/// The section.
#[derive(Debug, Default)]
pub struct BoxesSection {
    /// The last `Boxes` or `BoxesStale`; `None` before the first.
    snapshot: Option<BoxesSnapshot>,
    /// Why the read was refused (`Failed { request: "boxes" }`); cleared by the next snapshot.
    unavailable: Option<String>,
    /// The selected box, by id so a reload that inserts a box does not move the selection (D62).
    selected: Option<BoxId>,
    /// Browse, or one editor.
    mode: Mode,
    /// The write in flight (D56): `Some("edit_box")` between an `EditBox` and its reply.
    busy: Option<&'static str>,
    /// A `ProbeBox` is in flight: the hint says so. Cleared by `BoxProbed` or its `Failed`.
    probing: bool,
    /// The one notice line; drawn in `theme.error` when `is_error` says so.
    notice: Option<String>,
}

/// Browse, or one editor open over one box.
#[derive(Debug, Default)]
enum Mode {
    /// No editor open.
    #[default]
    Browse,
    /// The declared-tags editor: a comma list in one line.
    Tags(Editor<TextField, Vec<String>>),
    /// The quirks editor: free text over several lines.
    Quirks(Editor<TextArea, String>),
}

/// An open editor: the box, the token it opened on, the value it opened on, the widget.
///
/// `TextField` and `TextArea` both redact their `Debug`, so the derived one prints no typed text;
/// `opened_on` is stored data, not typed text.
#[derive(Debug)]
struct Editor<W, O> {
    /// From the snapshot row the editor opened on.
    box_id: BoxId,
    /// `edit_version` at open; replaced only by a `BoxesStale` (D48).
    expected: i32,
    /// Tags: `canonical_declared_tags(opened list)`; quirks: the normalised opening text. Never
    /// refreshed (D59, the kinds section's `stored_budget` rule).
    opened_on: O,
    /// The widget.
    input: W,
}

impl BoxesSection {
    /// Stable identity (plan D47, OQ-19).
    pub const ID: SectionId = SectionId("boxes");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The listed boxes, empty before the first snapshot.
    fn boxes(&self) -> &[BoxRecord] {
        self.snapshot
            .as_ref()
            .map_or(&[][..], |snapshot| snapshot.boxes.as_slice())
    }

    /// The selected box's record, when it is listed.
    fn selected_record(&self) -> Option<&BoxRecord> {
        let id = self.selected?;
        self.boxes().iter().find(|record| record.row.id == id)
    }

    /// The box an open editor is over.
    fn editor_box(&self) -> Option<BoxId> {
        match &self.mode {
            Mode::Browse => None,
            Mode::Tags(editor) => Some(editor.box_id),
            Mode::Quirks(editor) => Some(editor.box_id),
        }
    }

    /// `j`/`k`: one row down or up, stopping at the ends (D62: by id, so the move is relative to
    /// wherever the selected box is in the current list).
    fn move_selection(&mut self, down: bool) {
        let boxes = self.boxes();
        let Some(at) = self
            .selected
            .and_then(|id| boxes.iter().position(|record| record.row.id == id))
        else {
            self.selected = boxes.first().map(|record| record.row.id);
            return;
        };
        let next = if down {
            (at + 1).min(boxes.len().saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
        self.selected = boxes.get(next).map(|record| record.row.id);
    }

    /// A snapshot arrived: it replaces the list whole, and the selection is kept when its box is
    /// still listed, else the first box (this box on the first snapshot).
    fn replace(&mut self, snapshot: &BoxesSnapshot) {
        let first = self.snapshot.is_none();
        self.unavailable = None;
        let listed = |id: BoxId| snapshot.boxes.iter().any(|record| record.row.id == id);
        self.selected = match self.selected {
            Some(id) if listed(id) => Some(id),
            _ => match snapshot.this_box {
                Some(this) if first && listed(this) => Some(this),
                _ => snapshot.boxes.first().map(|record| record.row.id),
            },
        };
        self.snapshot = Some(snapshot.clone());
    }

    /// `Boxes` (D48, D56): only the reply to this section's own save closes an editor; a plain
    /// read (`r`, activation, the re-read after a probe or a reconnect) leaves the editor, its
    /// text and its token alone.
    fn on_boxes(&mut self, snapshot: &BoxesSnapshot) {
        self.replace(snapshot);
        if self.busy.take().is_some() {
            self.mode = Mode::Browse;
            self.notice = None;
        }
    }

    /// `BoxesStale` (D48): the list is replaced, the editor keeps its text and takes the current
    /// row's token, and the retry is a second save rather than an automatic write.
    fn on_stale(&mut self, snapshot: &BoxesSnapshot) {
        self.busy = None;
        self.replace(snapshot);
        let Some(box_id) = self.editor_box() else {
            // Nothing is open to retry from, so the sentence has to say the write did not apply.
            self.notice = Some(CHANGED_ELSEWHERE_CLOSED.to_owned());
            return;
        };
        let now = snapshot
            .boxes
            .iter()
            .find(|record| record.row.id == box_id)
            .map(|record| record.row.edit_version);
        match (now, &mut self.mode) {
            (Some(token), Mode::Tags(editor)) => editor.expected = token,
            (Some(token), Mode::Quirks(editor)) => editor.expected = token,
            (Some(_), Mode::Browse) => {}
            (None, _) => {
                self.mode = Mode::Browse;
                self.notice = Some(DELETED_ELSEWHERE.to_owned());
                return;
            }
        }
        self.notice = Some(CHANGED_ELSEWHERE.to_owned());
    }

    /// `t`: the tag editor over the selected row, prefilled with its list. A stored list the rule
    /// refuses (hand-written SQL) opens anyway; its refusal comes on save (R-31).
    fn open_tags(&mut self) {
        let Some(record) = self.selected_record() else {
            return;
        };
        let row = &record.row;
        let editor = Editor {
            box_id: row.id,
            expected: row.edit_version,
            opened_on: canonical_declared_tags(&row.declared_tags)
                .unwrap_or_else(|_| row.declared_tags.clone()),
            input: TextField::with_text(&row.declared_tags.join(", ")),
        };
        self.mode = Mode::Tags(editor);
        self.notice = None;
    }

    /// `e`: the quirks editor over the selected row. "Unchanged" is measured against what the
    /// widget made of the stored text, which normalises line endings (D59).
    fn open_quirks(&mut self) {
        let Some(record) = self.selected_record() else {
            return;
        };
        let row = &record.row;
        let input = TextArea::with_text(&row.quirks);
        let editor = Editor {
            box_id: row.id,
            expected: row.edit_version,
            opened_on: input.text(),
            input,
        };
        self.mode = Mode::Quirks(editor);
        self.notice = None;
    }

    /// `p` (D49, PRD D4): milestone 1's probe, on this box only.
    fn probe(&mut self, ctx: &Ctx<'_>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let Some(selected) = self.selected else {
            return;
        };
        if snapshot.this_box == Some(selected) {
            ctx.request(StoreRequest::ProbeBox);
            self.probing = true;
            self.notice = None;
        } else {
            self.notice = Some(THIS_BOX_ONLY.to_owned());
        }
    }

    /// One key while an editor is open: the widget answers first, so every letter is text.
    ///
    /// Everything the widget passes on is swallowed rather than offered to the shell, except
    /// `CONTROL` chords (the kinds rule; `ctrl-c` quitting is MOD-52's).
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Browse => return Handled::Pass,
            Mode::Tags(editor) => editor.input.on_key(key),
            Mode::Quirks(editor) => editor.input.on_key(key),
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                // `busy` stays: a save already sent is still answered, and its reply is what
                // clears it.
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
            FieldOutcome::Pass => Handled::Consumed,
        }
    }

    /// `Enter` (tags) or `ctrl-s` (quirks): one `EditBox` with only the edited field, when the
    /// value differs from what the editor opened on (D48, D59). The editor stays open until the
    /// reply.
    fn submit(&mut self, ctx: &Ctx<'_>) {
        if self.busy.is_some() {
            self.notice = Some(IN_FLIGHT.to_owned());
            return;
        }
        let request = match &self.mode {
            Mode::Browse => return,
            Mode::Tags(editor) => {
                let text = editor.input.text().unwrap_or_default();
                match declared_tags_from_text(text) {
                    Err(sentence) => {
                        self.notice = Some(sentence);
                        return;
                    }
                    Ok(list) if list == editor.opened_on => None,
                    Ok(list) => Some(StoreRequest::EditBox {
                        box_id: editor.box_id,
                        expected: editor.expected,
                        edit: BoxEdit {
                            declared_tags: Some(list),
                            quirks: None,
                        },
                    }),
                }
            }
            Mode::Quirks(editor) => {
                let text = editor.input.text();
                (text != editor.opened_on).then(|| StoreRequest::EditBox {
                    box_id: editor.box_id,
                    expected: editor.expected,
                    edit: BoxEdit {
                        declared_tags: None,
                        quirks: Some(text),
                    },
                })
            }
        };
        match request {
            // Only a user who typed writes it: unchanged text closes the editor.
            None => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            Some(request) => {
                self.busy = Some(EDIT_NAME);
                ctx.request(request);
            }
        }
    }
}

impl SettingsSection for BoxesSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Boxes"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Boxes are the user's, not the workspace's: the same read whatever the scope.
        vec![StoreRequest::Boxes]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // Nothing here belongs to a workspace.
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !matches!(self.mode, Mode::Browse) {
            return self.on_editor_key(key, ctx);
        }
        // Browse. `j`, `k`, `t`, `e`, `p`, `r` are free: the global table binds `q`, `?`, the
        // digits and `w`, and the tab consumes `h`/`l`/`[`/`]`/arrows before a section sees them.
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_selection(true);
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_selection(false);
                Handled::Consumed
            }
            KeyCode::Char('t') => {
                self.open_tags();
                Handled::Consumed
            }
            KeyCode::Char('e') => {
                self.open_quirks();
                Handled::Consumed
            }
            KeyCode::Char('p') => {
                self.probe(ctx);
                Handled::Consumed
            }
            // Always allowed (the kinds section's reason): re-reading is how a section that lost
            // a reply recovers, and a read never touches an open editor's token.
            KeyCode::Char('r') => {
                ctx.request(StoreRequest::Boxes);
                Handled::Consumed
            }
            // Only when there is something to clear: a section that swallowed every `Esc` would
            // take the one the shell uses to close an overlay over it.
            KeyCode::Esc if self.notice.is_some() => {
                self.notice = None;
                Handled::Consumed
            }
            _ => Handled::Pass,
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Boxes(snapshot) => self.on_boxes(snapshot),
            StoreReply::BoxesStale(snapshot) => self.on_stale(snapshot),
            // The report carries counts, not rows (D49): the list is read again.
            StoreReply::BoxProbed(_) => {
                self.probing = false;
                ctx.request(StoreRequest::Boxes);
            }
            // The read itself was refused: saying so beats an empty list that reads as "no box".
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.unavailable = Some(message.clone());
            }
            // A refused write: the editor stays open over its text, and a second save retries.
            StoreReply::Failed { request, message } if *request == EDIT_NAME => {
                self.busy = None;
                self.notice = Some(message.clone());
            }
            StoreReply::Failed { request, message }
                if *request == StoreRequest::ProbeBox.name() =>
            {
                self.probing = false;
                self.notice = Some(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
}
