//! `Settings > Boxes` (MOD-7 milestone 2, PRD D4, `R-TUI-8`): every box of this user with its
//! profile, tools, probed and declared tags, quirks, executor and last probe; declared tags,
//! quirks and the executor (MOD-41 plan D10) edited as a compare-and-set a reconnect cannot
//! stale; the probe on this box only. `s` edits the app-wide probe spec overlay (MOD-51 D6) in
//! the body, box or no box.
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
//! the editor early; if the save then comes back `BoxesStale`, `CHANGED_ELSEWHERE_CLOSED` says
//! nothing was written and how to retry. For a probe spec save the early close also shows
//! `SPEC_SAVED` and loses the typed text, as for `edit_box`; the real reply's notice then
//! replaces it (`CHANGED_ELSEWHERE_CLOSED`, or the refusal's sentence), and nothing is written
//! wrongly (MOD-51 F-11, blueprint R-1).

use htui_core::model::{
    BoxId, BoxRecord, Executor, Scope, canonical_declared_tags, declared_tags_from_text,
};

use std::collections::BTreeSet;

use crate::app::{Ctx, Handled};
use crate::box_settings::{BoxesSnapshot, READ_NAME, REQUEST_NAMES, SpecView};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    is_error, message, wrapped,
};
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use htui_core::model::BoxEdit;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use serde_json::Value;

/// What the body says before any boxes have arrived.
const NOT_READ: &str = "boxes not read yet";

/// The opening of the one line a refused read leaves (the prompt section's `UNAVAILABLE` style).
const UNAVAILABLE: &str = "boxes unavailable";

/// What the body says over a list with no box in it.
const NO_BOXES: &str = "no box is registered for this user yet";

/// The Browse keys (MOD-51 D6: `s` opens the probe spec editor; `p probe this box` became
/// `p probe` so the line fits an 80-column terminal, blueprint F-10, and [`THIS_BOX_ONLY`] still
/// explains `p` on another box).
const HINT_BROWSE: &str = "j/k move \u{b7} t tags \u{b7} e quirks \u{b7} w executor \u{b7} p probe \
                           \u{b7} s spec \u{b7} r reload";

/// The Browse keys over a read list with no box in it: the spec is app-wide, so `s` still works
/// (MOD-51 D6).
const HINT_NO_LIST: &str = "s spec \u{b7} r reload";

/// The Browse keys before the first read and over a refused one, where `s` does nothing
/// (MOD-51 F-3).
const HINT_RELOAD: &str = "r reload";

/// The tag editor's keys.
const HINT_TAGS: &str = "Enter saves \u{b7} Esc cancels \u{b7} comma-separated";

/// The quirks editor's keys (OQ-16: `Enter` is a line break, so `ctrl-s` saves).
const HINT_QUIRKS: &str = "ctrl-s saves \u{b7} Esc cancels \u{b7} Enter breaks the line";

/// The executor confirmation's keys (MOD-41 plan D10).
const HINT_EXECUTOR: &str = "y write \u{b7} n/esc cancel";

/// The probe spec editor's keys (MOD-51 D6; OQ-16: `Enter` is a line break, so `ctrl-s` saves).
const HINT_SPEC: &str =
    "ctrl-s saves \u{b7} Esc cancels \u{b7} Enter breaks the line \u{b7} blank clears";

/// The dim line above the probe spec editor: what the text is, and what blank does (MOD-51 D6).
const SPEC_TITLE: &str =
    "stored overlay (app_setting.box_probe_spec), merged into the seed by name; blank clears it";

/// A probe spec save applied (MOD-51 D6, D8): nothing re-probes now; the next `p` or connect does.
const SPEC_SAVED: &str =
    "probe spec saved \u{2014} the next p (or the next connect) re-probes under it";

/// The opening of a probe spec that does not parse; `serde_json`'s own sentence follows.
const SPEC_NOT_JSON: &str = "the overlay is not JSON";

/// How many lines `PageUp`/`PageDown` move in the probe spec editor: its drawn height follows
/// the frame, which `on_key` cannot see.
const SPEC_PAGE: u16 = 10;

/// The list pane's width, the one-column gutter to the detail pane included.
const LIST_WIDTH: u16 = 28;

/// The detail pane's label column.
const LABEL_WIDTH: usize = 14;

/// How many lines the quirks editor shows at most.
const QUIRKS_HEIGHT: u16 = 6;

/// How many hex digits of the spec digest the spec line shows (D51).
const DIGEST_SHOWN: usize = 12;

/// How many hex digits of a box id tell two same-named boxes apart (PRD `:262`).
const ID_SUFFIX: usize = 8;

/// What a second write says while one is in flight, after the request's name (D56, MOD-51
/// F-5): `edit_box in flight` is byte-identical to the sentence before MOD-51.
const IN_FLIGHT: &str = "in flight";

/// [`CHANGED_ELSEWHERE`] for the two `ctrl-s` editors, quirks and the probe spec (MOD-51 D6),
/// where `Enter` breaks the line (OQ-16) and `ctrl-s` is the save that retries. Starts
/// `changed elsewhere`, so `is_error` draws it in `theme.error`.
const CHANGED_ELSEWHERE_QUIRKS: &str = "changed elsewhere since you opened it \u{2014} reloaded; ctrl-s retries against the current row";

/// [`CHANGED_ELSEWHERE`] for the executor confirmation, where `y` is the write that retries
/// (MOD-41 plan D10). Starts `changed elsewhere`, so `is_error` draws it in `theme.error`.
const CHANGED_ELSEWHERE_EXECUTOR: &str =
    "changed elsewhere since you opened it \u{2014} reloaded; y retries against the current row";

/// `p` on a box that is not this one (PRD D4).
const THIS_BOX_ONLY: &str = "the probe runs on this box only";

/// The name `busy` carries while an `EditBox` is in flight: the request's own name.
const EDIT_NAME: &str = REQUEST_NAMES[1];

/// The name `busy` carries while a `SetProbeSpec` is in flight (MOD-51 D6).
const SPEC_NAME: &str = REQUEST_NAMES[2];

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
    /// The write in flight (D56): `Some("edit_box")` between an `EditBox` and its reply,
    /// `Some("set_probe_spec")` between a `SetProbeSpec` and its reply (MOD-51 D6).
    busy: Option<&'static str>,
    /// A `ProbeBox` is in flight: the hint says so. Cleared by `BoxProbed` or its `Failed`.
    probing: bool,
    /// The one notice line; drawn in `theme.error` when `is_error` says so.
    notice: Option<String>,
}

/// Browse, or one editor open: over one box, or over the app-wide probe spec (MOD-51 D6).
#[derive(Debug, Default)]
enum Mode {
    /// No editor open.
    #[default]
    Browse,
    /// The declared-tags editor: a comma list in one line.
    Tags(Editor<TextField, Vec<String>>),
    /// The quirks editor: free text over several lines.
    Quirks(Editor<TextArea, String>),
    /// The executor flip, awaiting y/n (MOD-41 plan D10).
    Executor(ExecutorFlip),
    /// The probe spec editor (MOD-51 D6): app-wide, over no box.
    Spec(SpecEditor),
}

/// An open probe spec editor: the token it holds, and the widget.
///
/// `TextArea` redacts its `Debug`, so the derived one prints no typed text.
#[derive(Debug)]
struct SpecEditor {
    /// `SpecView.stored`'s `updated_at` at open, `None` for no row; replaced only by a
    /// `BoxesStale` (D48), never by a plain `Boxes`. The blank rule reads it (D6, F-4; amended
    /// at review (MOD-51 LOW-1): the live token decides).
    expected: Option<DateTime<Utc>>,
    /// The overlay pretty-printed (`serde_json::to_string_pretty`), or empty; the cursor at the
    /// end, as the quirks editor's.
    input: TextArea,
}

/// An open executor confirmation: the box, the token it opened on, and the flip it proposes.
#[derive(Debug)]
struct ExecutorFlip {
    /// From the snapshot row the confirmation opened on.
    box_id: BoxId,
    /// That row's hostname, kept for the question: a plain read that drops the box moves the
    /// selection, and the question must still name the box `y` writes to (D48).
    hostname: String,
    /// `edit_version` at open; replaced only by a `BoxesStale` (D48).
    expected: i32,
    /// `Executor::of(&row.settings)` at open.
    from: Executor,
    /// What `y` writes: always `Tui` or `Worker`, since `edit_box` refuses `Other(_)` (F-35).
    to: Executor,
}

impl ExecutorFlip {
    /// The flip `w` proposes over `record`: `tui` to `worker`, and anything else back to `tui`,
    /// the default (F-35: an unknown value cannot be written, so it cannot be kept either).
    fn over(record: &BoxRecord) -> Self {
        let from = Executor::of(&record.row.settings);
        let to = match from {
            Executor::Tui => Executor::Worker,
            Executor::Worker | Executor::Other(_) => Executor::Tui,
        };
        Self {
            box_id: record.row.id,
            hostname: record.row.hostname.clone(),
            expected: record.row.edit_version,
            from,
            to,
        }
    }

    /// The confirmation line over the flip's own box: the flip, then what it changes.
    fn question(&self) -> String {
        let consequence = match self.to {
            Executor::Worker => {
                "The TUI stops walking runs here; `htui worker` must run on this box."
            }
            Executor::Tui | Executor::Other(_) => "The TUI walks this box's runs again.",
        };
        format!(
            "executor of `{}`: `{}` \u{2192} `{}`? {consequence}",
            self.hostname, self.from, self.to
        )
    }
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

    /// The box an open editor is over; the probe spec editor is over none.
    fn editor_box(&self) -> Option<BoxId> {
        match &self.mode {
            Mode::Browse | Mode::Spec(_) => None,
            Mode::Tags(editor) => Some(editor.box_id),
            Mode::Quirks(editor) => Some(editor.box_id),
            Mode::Executor(flip) => Some(flip.box_id),
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
    /// text and its token alone. A probe spec save that applied says the next `p` re-probes
    /// (MOD-51 D6, D8).
    fn on_boxes(&mut self, snapshot: &BoxesSnapshot) {
        self.replace(snapshot);
        if let Some(name) = self.busy.take() {
            self.mode = Mode::Browse;
            self.notice = (name == SPEC_NAME).then(|| SPEC_SAVED.to_owned());
        }
    }

    /// `BoxesStale` (D48): the list is replaced, the editor keeps its text and takes the current
    /// row's token, and the retry is a second save rather than an automatic write.
    fn on_stale(&mut self, snapshot: &BoxesSnapshot) {
        self.busy = None;
        self.replace(snapshot);
        // MOD-51 F-6(a): the probe spec editor is over no box, so it is settled before
        // `editor_box` would take it for no editor at all. Its token is the overlay row's, `None`
        // when the row is gone (the retry is then an insert).
        if let Mode::Spec(editor) = &mut self.mode {
            editor.expected = snapshot.spec.stored.as_ref().map(|row| row.updated_at);
            self.notice = Some(CHANGED_ELSEWHERE_QUIRKS.to_owned());
            return;
        }
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
        let notice = match (now, &mut self.mode) {
            (Some(token), Mode::Tags(editor)) => {
                editor.expected = token;
                CHANGED_ELSEWHERE
            }
            (Some(token), Mode::Quirks(editor)) => {
                editor.expected = token;
                CHANGED_ELSEWHERE_QUIRKS
            }
            (Some(token), Mode::Executor(flip)) => {
                flip.expected = token;
                CHANGED_ELSEWHERE_EXECUTOR
            }
            (Some(_), Mode::Browse | Mode::Spec(_)) => CHANGED_ELSEWHERE,
            (None, _) => {
                self.mode = Mode::Browse;
                DELETED_ELSEWHERE
            }
        };
        self.notice = Some(notice.to_owned());
    }

    /// Whether `t`/`e`/`w`/`s` must do nothing: a list the read could not confirm (the body shows
    /// the refusal, not the rows), or a save in flight (D56, the kinds section's `blocked`): its
    /// reply closes whatever editor is open, so a second one would lose its text to the first's
    /// reply. The notice names the write in flight (MOD-51 F-5).
    fn blocked(&mut self) -> bool {
        if self.unavailable.is_some() {
            return true;
        }
        if let Some(name) = self.busy {
            self.notice = Some(format!("{name} {IN_FLIGHT}"));
            return true;
        }
        false
    }

    /// `t`: the tag editor over the selected row, prefilled with its list. A stored list the rule
    /// refuses (hand-written SQL) opens anyway; its refusal comes on save (R-31).
    fn open_tags(&mut self) {
        if self.blocked() {
            return;
        }
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
        if self.blocked() {
            return;
        }
        let Some(record) = self.selected_record() else {
            return;
        };
        let row = &record.row;
        // Cursor at the end of the note, where a new quirk is appended.
        let mut input = TextArea::with_text(&row.quirks);
        input.set_cursor(usize::MAX);
        let editor = Editor {
            box_id: row.id,
            expected: row.edit_version,
            opened_on: input.text().to_owned(),
            input,
        };
        self.mode = Mode::Quirks(editor);
        self.notice = None;
    }

    /// `w` (MOD-41 plan D10): the executor confirmation over the selected row. Nothing is
    /// written until `y`.
    fn open_executor(&mut self) {
        if self.blocked() {
            return;
        }
        let Some(record) = self.selected_record() else {
            return;
        };
        self.mode = Mode::Executor(ExecutorFlip::over(record));
        self.notice = None;
    }

    /// `s` (MOD-51 D6): the probe spec editor over the stored overlay, with or without a listed
    /// box, whenever a snapshot is present and the read was not refused. An overlay the probe
    /// ignores opens as stored (plan R-6): saving it unchanged is refused, clearing it is not.
    fn open_spec(&mut self) {
        if self.blocked() {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let stored = snapshot.spec.stored.as_ref();
        let text = stored
            .and_then(|row| row.value.as_ref())
            .map(|value| serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
            .unwrap_or_default();
        let mut input = TextArea::with_text(&text);
        input.set_cursor(usize::MAX);
        self.mode = Mode::Spec(SpecEditor {
            expected: stored.map(|row| row.updated_at),
            input,
        });
        self.notice = None;
    }

    /// `p` (D49, PRD D4): milestone 1's probe, on this box only.
    ///
    /// One `ProbeBox` in flight per section, a deviation from D63: a second one would supersede
    /// the first's seq in the shell's freshness gate, so the runtime's instant `BOX_PROBE_RUNNING`
    /// refusal would clear `probing` while the first probe still runs, and the first's
    /// `BoxProbed` would be dropped before the section could re-read. The runtime's own sentence
    /// is shown instead, so there is still no second local sentence.
    fn probe(&mut self, ctx: &Ctx<'_>) {
        if self.unavailable.is_some() {
            return;
        }
        if self.probing {
            self.notice = Some(crate::agent_worker::BOX_PROBE_RUNNING.to_owned());
            return;
        }
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
    /// `CONTROL` chords (the kinds rule), so `ctrl-c` still quits.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Browse => return Handled::Pass,
            Mode::Executor(_) => return self.on_executor_key(key, ctx),
            Mode::Tags(editor) => editor.input.on_key(key),
            // `PageUp`/`PageDown` move by the editor's drawn height.
            Mode::Quirks(editor) => editor.input.on_key(key, QUIRKS_HEIGHT),
            Mode::Spec(editor) => editor.input.on_key(key, SPEC_PAGE),
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

    /// One key over the executor confirmation (MOD-41 plan D10): `y` writes, `n`/`Esc` cancel,
    /// `CONTROL` chords pass (so `ctrl-c` still quits) and every other key is swallowed.
    fn on_executor_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::Pass;
        }
        match key.code {
            KeyCode::Char('y') => self.submit(ctx),
            KeyCode::Char('n') | KeyCode::Esc => {
                // `busy` stays, as in `on_editor_key`: a write already sent is still answered.
                self.mode = Mode::Browse;
                self.notice = None;
            }
            _ => {}
        }
        Handled::Consumed
    }

    /// `Enter` (tags), `ctrl-s` (quirks) or `y` (executor): one `EditBox` with only the edited
    /// field, when the value differs from what the editor opened on (D48, D59); a flip always
    /// differs. `ctrl-s` (probe spec, MOD-51 D6): one `SetProbeSpec` with the parsed overlay,
    /// or a clear when the text is blank. The editor stays open until the reply.
    fn submit(&mut self, ctx: &Ctx<'_>) {
        if let Some(name) = self.busy {
            self.notice = Some(format!("{name} {IN_FLIGHT}"));
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
                            executor: None,
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
                        quirks: Some(text.to_owned()),
                        executor: None,
                    },
                })
            }
            Mode::Executor(flip) => Some(StoreRequest::EditBox {
                box_id: flip.box_id,
                expected: flip.expected,
                edit: BoxEdit {
                    executor: Some(flip.to.clone()),
                    ..BoxEdit::default()
                },
            }),
            Mode::Spec(editor) => {
                let text = editor.input.text();
                if text.trim().is_empty() {
                    // D6, F-4, amended at review (MOD-51 LOW-1): the live token decides. With
                    // no token (no row at open, or the row vanished under the editor) there is
                    // nothing to clear and the editor just closes; under a token, the row it
                    // names is cleared, including one a `BoxesStale` brought in after an open
                    // over no row.
                    editor.expected.map(|token| StoreRequest::SetProbeSpec {
                        overlay: None,
                        expected: Some(token),
                    })
                } else {
                    match serde_json::from_str::<Value>(text) {
                        // Parsed on the render side; nothing is sent until it parses.
                        Err(error) => {
                            self.notice = Some(format!("{SPEC_NOT_JSON}: {error}"));
                            return;
                        }
                        // No unchanged short-circuit (F-15): R-6 needs an unchanged save sent.
                        Ok(overlay) => Some(StoreRequest::SetProbeSpec {
                            overlay: Some(overlay),
                            expected: editor.expected,
                        }),
                    }
                }
            }
        };
        match request {
            // Only a user who typed writes it: unchanged text (or, for the probe spec, blank
            // text with nothing to clear) closes the editor.
            None => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            Some(request) => {
                // `edit_box` or `set_probe_spec`: the name its `Failed` comes back under.
                self.busy = Some(request.name());
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

    /// MOD-22 review M-1: a bracketed paste into the open tags field, quirks editor or probe
    /// spec editor (MOD-51 D6).
    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        match &mut self.mode {
            Mode::Browse | Mode::Executor(_) => return Handled::Pass,
            Mode::Tags(editor) => {
                editor.input.on_paste(text);
            }
            Mode::Quirks(editor) => editor.input.on_paste(text),
            Mode::Spec(editor) => editor.input.on_paste(text),
        }
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if !matches!(self.mode, Mode::Browse) {
            return self.on_editor_key(key, ctx);
        }
        // Browse. `j`, `k`, `t`, `e`, `p`, `r`, `s` are free: the global table binds `q`, `?`, the
        // digits and `w`, and the tab consumes `h`/`l`/`[`/`]`/arrows before a section sees them.
        // `w` (MOD-41 plan D10) shadows the global workspace switcher only while a box is listed
        // to act on; over no list it passes, so the switcher is still one key away.
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
            KeyCode::Char('w')
                if self.unavailable.is_none() && self.selected_record().is_some() =>
            {
                self.open_executor();
                Handled::Consumed
            }
            // MOD-51 D6: the probe spec is app-wide, so `s` needs no listed box.
            KeyCode::Char('s') => {
                self.open_spec();
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
            // A refused probe spec save (MOD-51 D6): the same, with the probe's own sentence when
            // the overlay was refused (D3) or the store's when the write was.
            StoreReply::Failed { request, message } if *request == SPEC_NAME => {
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

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let notice = self
            .notice
            .as_deref()
            .map(|text| wrapped(text, usize::from(area.width).max(1)))
            .unwrap_or_default();
        let [body, hint, notice_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(u16::try_from(notice.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);

        if let (Mode::Spec(editor), Some(snapshot)) = (&self.mode, &self.snapshot) {
            // MOD-51 F-6(b): an editor that takes keys stays on screen (the rule below), box or
            // no box: a refused read takes the body's first line and the editor keeps the rest.
            let area = match &self.unavailable {
                Some(why) => {
                    let [refusal, rest] =
                        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
                    frame.render_widget(
                        Paragraph::new(Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)),
                        refusal,
                    );
                    rest
                }
                None => body,
            };
            render_spec(frame, area, snapshot, editor, theme);
        } else {
            match (&self.unavailable, &self.snapshot) {
                // An open editor survives a refused read (the re-read after a probe, a reconnect):
                // the refusal takes the body's first line and the editor stays on screen under it,
                // because an editor that still takes keys has to be one the user can see.
                (Some(why), Some(snapshot))
                    if !matches!(self.mode, Mode::Browse) && !snapshot.boxes.is_empty() =>
                {
                    let [refusal, rest] =
                        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
                    frame.render_widget(
                        Paragraph::new(Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error)),
                        refusal,
                    );
                    self.render_body(frame, rest, snapshot, theme);
                }
                // The refusal wins the body even with a list behind it: what is on screen
                // would otherwise be boxes nothing has confirmed since the outage started (the
                // kinds rule).
                (Some(why), _) => frame.render_widget(
                    Paragraph::new(Line::styled(format!("{UNAVAILABLE}: {why}"), theme.error))
                        .wrap(Wrap { trim: true }),
                    body,
                ),
                (None, None) => message(frame, body, NOT_READ, theme),
                // MOD-51 LOW-6: the spec is app-wide, so it is drawn with no box listed too: a
                // save over an empty list shows the new source and digest, and an ignored
                // overlay says why.
                (None, Some(snapshot)) if snapshot.boxes.is_empty() => {
                    let width = usize::from(body.width).max(1);
                    let mut lines: Vec<Line<'static>> = wrapped(NO_BOXES, width)
                        .into_iter()
                        .map(|line| Line::styled(line, theme.dim))
                        .collect();
                    lines.push(Line::default());
                    lines.extend(spec_lines(&snapshot.spec, width, theme));
                    frame.render_widget(Paragraph::new(lines), body);
                }
                (None, Some(snapshot)) => self.render_body(frame, body, snapshot, theme),
            }
        }

        frame.render_widget(
            Paragraph::new(Line::styled(self.hint_text(), theme.dim)),
            hint,
        );
        if let Some(text) = &self.notice {
            let style = if is_error(text) {
                theme.error
            } else {
                theme.dim
            };
            let lines: Vec<Line<'static>> = notice
                .into_iter()
                .map(|line| Line::styled(line, style))
                .collect();
            frame.render_widget(Paragraph::new(lines), notice_area);
        }
    }
}

impl BoxesSection {
    /// The list and the detail pane side by side.
    fn render_body(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &BoxesSnapshot,
        theme: &Theme,
    ) {
        let [list, detail] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(0)]).areas(area);
        self.render_list(frame, list, snapshot, theme);
        if let Some(record) = self.selected_record() {
            frame.render_widget(
                Paragraph::new(self.detail(record, snapshot, detail.width, theme)),
                detail,
            );
        }
    }

    /// The list pane: one row per box, the selected one in `theme.selected`, scrolled so the
    /// selection stays on screen.
    fn render_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &BoxesSnapshot,
        theme: &Theme,
    ) {
        // One column is the gutter between the two panes.
        let room = usize::from(area.width.saturating_sub(1));
        let lines: Vec<Line<'static>> = snapshot
            .boxes
            .iter()
            .map(|record| {
                let style = if Some(record.row.id) == self.selected {
                    theme.selected
                } else {
                    theme.base
                };
                Line::styled(list_label(record, snapshot, room), style)
            })
            .collect();
        let at = self
            .selected
            .and_then(|id| snapshot.boxes.iter().position(|record| record.row.id == id))
            .unwrap_or(0);
        let offset = at.saturating_sub(usize::from(area.height).saturating_sub(1));
        frame.render_widget(
            Paragraph::new(lines).scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
            area,
        );
    }

    /// The detail pane for one box: a 14-wide label column, then the value; an open editor draws
    /// in its row whichever box is selected, so a reload that moved the selection never leaves
    /// someone typing into a field they cannot see.
    fn detail(
        &self,
        record: &BoxRecord,
        snapshot: &BoxesSnapshot,
        width: u16,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let row = &record.row;
        let room = usize::from(width).saturating_sub(LABEL_WIDTH).max(1);
        let value_width = u16::try_from(room).unwrap_or(u16::MAX);
        let this_box = snapshot.this_box == Some(row.id);
        let mut lines = vec![
            labelled(
                "host",
                format!(
                    "{}{}",
                    row.hostname,
                    if this_box { " (this box)" } else { "" }
                ),
                theme,
            ),
            labelled(
                "os",
                format!("{} {} \u{b7} {}", row.os_family, row.os_version, row.arch),
                theme,
            ),
            labelled("cpu", row.cpu.clone(), theme),
            labelled(
                "ram",
                row.ram_mb
                    .map_or_else(|| "unknown".to_owned(), |mb| format!("{mb} MB")),
                theme,
            ),
            labelled(
                "gpu",
                gpu(row.gpu_present, row.gpu_vendor.as_deref()),
                theme,
            ),
            labelled("htui", row.htui_version.clone(), theme),
            labelled(
                "last probe",
                row.last_probed_at.map_or_else(
                    || "never probed".to_owned(),
                    |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
                ),
                theme,
            ),
            labelled(
                "spec",
                format!(
                    "probed under the current spec: {}",
                    if record.probe_spec_digest.as_deref() == Some(snapshot.spec.digest.as_str()) {
                        "yes"
                    } else {
                        "no"
                    }
                ),
                theme,
            ),
            labelled("probed tags", tag_list(&row.probed_tags), theme),
        ];

        match &self.mode {
            Mode::Tags(editor) => {
                let mut spans = vec![label("declared tags", theme)];
                spans.extend(editor.input.line(value_width, true, theme).spans);
                lines.push(Line::from(spans));
                lines.push(indented(
                    Span::styled(format!("seen: {}", seen(snapshot)), theme.dim),
                    theme,
                ));
            }
            _ => lines.push(labelled(
                "declared tags",
                tag_list(&row.declared_tags),
                theme,
            )),
        }

        match &self.mode {
            Mode::Quirks(editor) => {
                let area = editor.input.lines(value_width, QUIRKS_HEIGHT, true, theme);
                lines.extend(under_label("quirks", area, theme));
            }
            _ if row.quirks.is_empty() => {
                lines.push(labelled("quirks", "(none)".to_owned(), theme))
            }
            _ => {
                let note = row
                    .quirks
                    .lines()
                    .map(|line| Line::styled(line.to_owned(), theme.base))
                    .collect();
                lines.extend(under_label("quirks", note, theme));
            }
        }

        match &self.mode {
            Mode::Executor(flip) => {
                // The flip names its own box, kept from open: after a reload that dropped it the
                // selection is another box, and `y` still writes to the flip's.
                let question = wrapped(&flip.question(), room)
                    .into_iter()
                    .map(|line| Line::styled(line, theme.base))
                    .collect();
                lines.extend(under_label("executor", question, theme));
            }
            _ => lines.push(labelled(
                "executor",
                Executor::of(&row.settings).to_string(),
                theme,
            )),
        }

        let tools: Vec<String> = record
            .tools
            .iter()
            .map(|tool| {
                if tool.version.is_empty() {
                    tool.name.clone()
                } else {
                    format!("{} {}", tool.name, tool.version)
                }
            })
            .collect();
        if tools.is_empty() {
            lines.push(labelled("tools", "(none)".to_owned(), theme));
        } else {
            let wrapped_tools = wrapped(&tools.join(", "), room)
                .into_iter()
                .map(|line| Line::styled(line, theme.base))
                .collect();
            lines.extend(under_label("tools", wrapped_tools, theme));
        }

        lines.push(Line::default());
        lines.extend(spec_lines(&snapshot.spec, usize::from(width).max(1), theme));
        lines
    }

    /// The keys this mode binds, plus what is in flight.
    fn hint_text(&self) -> String {
        // MOD-51 F-3: `s` is offered exactly where it works, a read that was not refused.
        let readable = self.unavailable.is_none() && self.snapshot.is_some();
        let listed = readable && !self.boxes().is_empty();
        let mut hint = match self.mode {
            Mode::Browse if listed => HINT_BROWSE.to_owned(),
            Mode::Browse if readable => HINT_NO_LIST.to_owned(),
            Mode::Browse => HINT_RELOAD.to_owned(),
            Mode::Tags(_) => HINT_TAGS.to_owned(),
            Mode::Quirks(_) => HINT_QUIRKS.to_owned(),
            Mode::Executor(_) => HINT_EXECUTOR.to_owned(),
            Mode::Spec(_) => HINT_SPEC.to_owned(),
        };
        if self.probing {
            hint.push_str(" \u{b7} probing\u{2026}");
        }
        if self.busy.is_some() {
            hint.push_str(" \u{b7} saving\u{2026}");
        }
        hint
    }
}

/// A list row: the hostname, the last eight hex digits of the id only when another listed box has
/// the same hostname (PRD `:262`), and ` (this box)` on this box, in `room` chars.
///
/// Only the hostname is clipped: the suffix and the marker are what tell two same-named boxes
/// apart, so they survive a long hostname.
fn list_label(record: &BoxRecord, snapshot: &BoxesSnapshot, room: usize) -> String {
    let row = &record.row;
    let mut tail = String::new();
    let shared = snapshot
        .boxes
        .iter()
        .any(|other| other.row.id != row.id && other.row.hostname == row.hostname);
    if shared {
        let simple = row.id.as_uuid().simple().to_string();
        let suffix = &simple[simple.len().saturating_sub(ID_SUFFIX)..];
        tail.push_str(&format!(" \u{2026}{suffix}"));
    }
    if snapshot.this_box == Some(row.id) {
        tail.push_str(" (this box)");
    }
    let host_room = room.saturating_sub(tail.chars().count()).max(1);
    let mut label = clip(&row.hostname, host_room);
    label.push_str(&tail);
    clip(&label, room)
}

/// `text` cut to `width` chars, the last one a `…` when anything was cut.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(width.saturating_sub(1)).collect();
    if width > 0 {
        clipped.push('\u{2026}');
    }
    clipped
}

/// The label column's span.
fn label(name: &str, theme: &Theme) -> Span<'static> {
    Span::styled(format!("{name:<LABEL_WIDTH$}"), theme.dim)
}

/// One detail row: the label, then the value.
fn labelled(name: &str, value: String, theme: &Theme) -> Line<'static> {
    Line::from(vec![label(name, theme), Span::styled(value, theme.base)])
}

/// A continuation row: the value column, with nothing in the label column.
fn indented(value: Span<'static>, theme: &Theme) -> Line<'static> {
    Line::from(vec![label("", theme), value])
}

/// Several value lines: the first on the label's row, the rest indented to the value column.
fn under_label(name: &str, values: Vec<Line<'static>>, theme: &Theme) -> Vec<Line<'static>> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let head = if index == 0 {
                label(name, theme)
            } else {
                label("", theme)
            };
            let mut spans = vec![head];
            spans.extend(value.spans);
            Line::from(spans)
        })
        .collect()
}

/// The `gpu` row: the vendor when there is one, `present` without, `none` without a GPU.
fn gpu(present: bool, vendor: Option<&str>) -> String {
    match (present, vendor) {
        (true, Some(vendor)) => vendor.to_owned(),
        (true, None) => "present".to_owned(),
        (false, _) => "none".to_owned(),
    }
}

/// A stored tag list in stored order, or `(none)`.
fn tag_list(tags: &[String]) -> String {
    if tags.is_empty() {
        "(none)".to_owned()
    } else {
        tags.join(", ")
    }
}

/// D50: every probed and declared tag on a listed box, sorted and deduplicated.
fn seen(snapshot: &BoxesSnapshot) -> String {
    let tags: BTreeSet<&str> = snapshot
        .boxes
        .iter()
        .flat_map(|record| {
            record
                .row
                .probed_tags
                .iter()
                .chain(&record.row.declared_tags)
        })
        .map(String::as_str)
        .collect();
    tags.into_iter().collect::<Vec<_>>().join(", ")
}

/// The probe spec editor in the body (MOD-51 D6, F-6(c)): the spec in force as it stands (D51's
/// lines, the ignored-overlay sentence included), the dim title, then the editor over every line
/// left. The spec line does not follow the typing: it says what the next probe runs under, and the
/// `Boxes` that answers a save carries the new digest.
fn render_spec(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &BoxesSnapshot,
    editor: &SpecEditor,
    theme: &Theme,
) {
    let mut lines = spec_lines(&snapshot.spec, usize::from(area.width).max(1), theme);
    lines.push(Line::styled(SPEC_TITLE, theme.dim));
    let used = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    lines.extend(
        editor
            .input
            .lines(area.width, area.height.saturating_sub(used), true, theme),
    );
    frame.render_widget(Paragraph::new(lines), area);
}

/// D51: the spec the next probe runs under, and why a stored overlay was ignored when it was.
fn spec_lines(spec: &SpecView, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let source = if spec.overlay {
        "seed + stored overlay"
    } else {
        "seed"
    };
    let digest: String = spec.digest.chars().take(DIGEST_SHOWN).collect();
    let mut lines = vec![Line::styled(
        format!("probe spec: {source} \u{b7} {digest}"),
        theme.dim,
    )];
    if let Some(error) = &spec.error {
        let style: Style = theme.error;
        lines.extend(
            wrapped(error, width)
                .into_iter()
                .map(|line| Line::styled(line, style)),
        );
    }
    lines
}
