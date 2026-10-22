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
//! It draws through the kit (MOD-115 M3): the list is a `kit::list` (gutter, band), the detail a
//! `kit::form` whose open editor is its focused row (MOD-85: the marker, not an accented label),
//! the notice `kit::text` lines and the empty states `kit::empty` with a next step. Every hint and
//! every sentence that names a key reads it through the mode's stack, and acts resolve from
//! [`Ctx::pressed`] (MD-A).
//!
//! Known residue, the kinds section's H-9 (D56, R-29): a `Boxes` reply carries nothing that says
//! which request it answers, so `busy` is the whole of the attribution. A read that lands between
//! a save and its reply (a scope change or a tab re-activation) is taken for that reply and closes
//! the editor early; if the save then comes back `BoxesStale`, `CHANGED_ELSEWHERE_CLOSED` says
//! nothing was written and how to retry. For a probe spec save the early close also shows
//! [`spec_saved`]'s notice and loses the typed text, as for `edit_box`; the real reply's notice then
//! replaces it (`CHANGED_ELSEWHERE_CLOSED`, or the refusal's sentence), and nothing is written
//! wrongly (MOD-51 F-11, blueprint R-1). The same race can also land a tags or quirks save's
//! `BoxesStale` in the probe spec editor's branch (opened after the early close), refreshing the
//! spec token; nothing is written wrongly either (MOD-51 LOW-5).

use htui_core::model::{
    BoxId, BoxRecord, Executor, Scope, canonical_declared_tags, declared_tags_from_text,
};

use std::cell::Cell;
use std::collections::BTreeSet;

use crate::app::{Ctx, Handled};
use crate::box_settings::{BoxesSnapshot, READ_NAME, REQUEST_NAMES, SpecView};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::{self, cell_width};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form};
use crate::ui::kit::label;
use crate::ui::kit::list::{self, RowKind};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection, changed_elsewhere,
    is_error,
};
use crate::ui::{FieldOutcome, TextArea, TextField, Theme};
use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;
use htui_core::model::BoxEdit;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use serde_json::Value;

/// What the body says before any boxes have arrived; [`reads_them`] is its next step.
const NOT_READ: &str = "boxes not read yet";

/// The next step of [`NOT_READ`]: `reload`, the label of `common.reload` through the browse
/// stack, reads them; unbound, opening the section does.
fn reads_them(reload: Option<&str>) -> String {
    match reload {
        Some(reload) => format!("{reload} reads the boxes"),
        None => "the boxes are read when this section opens".to_owned(),
    }
}

/// The opening of the one line a refused read leaves (the prompt section's `UNAVAILABLE` style);
/// [`asks_again`] is its next step.
const UNAVAILABLE: &str = "boxes unavailable";

/// The next step of [`UNAVAILABLE`] (MOD-83): `reload` asks again; unbound, opening the section
/// does.
fn asks_again(reload: Option<&str>) -> String {
    match reload {
        Some(reload) => format!("{reload} asks again once the store answers"),
        None => "the store is asked again when this section opens".to_owned(),
    }
}

/// What the body says over a list with no box in it.
const NO_BOXES: &str = "no box is registered for this user yet";

/// The next step of [`NO_BOXES`]: a box registers itself when it connects.
const NO_BOXES_NEXT: &str = "a box is listed once htui connects from it";

/// The Browse keys (MOD-51 D6: `s` opens the probe spec editor; `p probe this box` became
/// `p probe` so the line fits an 80-column terminal, blueprint F-10, and [`THIS_BOX_ONLY`] still
/// explains `p` on another box).
const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, "move"),
    Hint::One(Act::BoxesEditTags, "tags"),
    Hint::One(Act::BoxesEditQuirks, "quirks"),
    Hint::One(Act::BoxesExecutor, "executor"),
    Hint::One(Act::BoxesProbe, "probe"),
    Hint::One(Act::BoxesEditSpec, "spec"),
    Hint::One(Act::Reload, "reload"),
];

/// The Browse keys over a read list with no box in it: the spec is app-wide, so `s` still works
/// (MOD-51 D6).
const HINT_NO_LIST: HintSpec = &[
    Hint::One(Act::BoxesEditSpec, "spec"),
    Hint::One(Act::Reload, "reload"),
];

/// The Browse keys before the first read and over a refused one, where `s` does nothing
/// (MOD-51 F-3).
const HINT_RELOAD: HintSpec = &[Hint::One(Act::Reload, "reload")];

/// The tag editor's keys: the field's own (MOD-67 D13), `field.submit` and `field.cancel` through
/// [`views::CAPTURE`] (MOD-115 M3 C6): `Enter saves · Esc cancels` by default.
const HINT_TAGS: HintSpec = &[
    Hint::One(Act::FieldSubmit, "saves"),
    Hint::One(Act::FieldCancel, "cancels"),
    Hint::Text("comma-separated"),
];

/// The quirks editor's keys (OQ-16: `Enter` is a line break, so `form.save`, `Ctrl+s`, saves).
/// The way out and the line break are the field's (C6): `field.submit`'s chord reaches the
/// `TextArea` as `Enter`, so it is the key that breaks the line.
const HINT_QUIRKS: HintSpec = &[
    Hint::One(Act::FormSave, "saves"),
    Hint::One(Act::FieldCancel, "cancels"),
    Hint::One(Act::FieldSubmit, "breaks the line"),
];

/// The executor confirmation's keys (MOD-41 plan D10): `y write · n/Esc cancel` by default.
const HINT_EXECUTOR: HintSpec = &[
    Hint::One(Act::ConfirmYes, "write"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

/// The probe spec editor's keys (MOD-51 D6; OQ-16: `Enter` is a line break, so `form.save`
/// saves).
const HINT_SPEC: HintSpec = &[
    Hint::One(Act::FormSave, "saves"),
    Hint::One(Act::FieldCancel, "cancels"),
    Hint::One(Act::FieldSubmit, "breaks the line"),
    Hint::Text("blank clears"),
];

/// The dim line above the probe spec editor: what the text is, and what blank does (MOD-51 D6).
/// It names no setting key (MOD-115 M3: no internal value on screen) and wraps in a narrow body.
const SPEC_TITLE: &str = "stored overlay, merged into the seed by tool name; blank clears it";

/// A probe spec save applied (MOD-51 D6, D8): nothing re-probes now; the next probe or connect
/// does. `probe` is the label of `settings.boxes.probe` through the browse stack (MOD-115 M3 C5),
/// `p` by default; unbound, the sentence names the probe and no key.
fn spec_saved(probe: Option<&str>) -> String {
    let next = probe.unwrap_or("probe");
    format!("probe spec saved \u{2014} the next {next} (or the next connect) re-probes under it")
}

/// The opening of a probe spec that does not parse; `serde_json`'s own sentence follows.
const SPEC_NOT_JSON: &str = "the overlay is not JSON";

/// How many lines `PageUp`/`PageDown` move in the probe spec editor: its drawn height follows
/// the frame, which `on_key` cannot see.
const SPEC_PAGE: u16 = 10;

/// The list pane's width: the kit's one-cell gutter, 27 cells of label and the one-column gap to
/// the detail pane (MOD-115 M3: one more than before, for the gutter, so a label keeps its room).
const LIST_WIDTH: u16 = 29;

/// The detail form's label column (the kit form adds its gutter before it and two cells after).
const LABEL_WIDTH: u16 = 14;

/// How many lines the quirks editor shows at most.
const QUIRKS_HEIGHT: u16 = 6;

/// How many hex digits of the spec digest the spec line shows (D51).
const DIGEST_SHOWN: usize = 12;

/// How many hex digits of a box id tell two same-named boxes apart (PRD `:262`).
const ID_SUFFIX: usize = 8;

/// What a second write is told while the first is still out (D56, MOD-51 F-5), in words
/// (MOD-115 M3: no request name on screen).
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// A request in flight in words, by [`StoreRequest::name`] ([`REQUEST_NAMES`]).
fn doing(busy: &str) -> &'static str {
    match busy {
        "boxes" => "reading the boxes",
        "edit_box" => "saving the box",
        "set_probe_spec" => "saving the probe spec",
        _ => "writing",
    }
}

/// [`changed_elsewhere`] for the executor confirmation, where `yes`, `ConfirmYes`'s label through
/// `BOXES_EXECUTOR` (MOD-67 M6 D4), is the write that retries (MOD-41 plan D10); with none bound
/// the clause goes. Starts `changed elsewhere`, so `is_error` draws it in `theme.error`.
fn changed_elsewhere_executor(yes: Option<&str>) -> String {
    match yes {
        Some(yes) => format!(
            "changed elsewhere since you opened it \u{2014} reloaded; {yes} retries against the \
             current row"
        ),
        None => "changed elsewhere since you opened it \u{2014} reloaded".to_owned(),
    }
}

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
    /// The list's scroll as of the last render, for the Settings pane's scrollbar (MOD-115 M3
    /// D-11, RV2): beside the list's rows; `None` while no list is drawn.
    scroll: Cell<Option<ScrollSpan>>,
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
    fn on_boxes(&mut self, snapshot: &BoxesSnapshot, keys: &Keys) {
        self.replace(snapshot);
        if let Some(name) = self.busy.take() {
            self.mode = Mode::Browse;
            self.notice = (name == SPEC_NAME)
                .then(|| spec_saved(keys.label(views::BOXES_BROWSE, Act::BoxesProbe).as_deref()));
        }
    }

    /// `BoxesStale` (D48): the list is replaced, the editor keeps its text and takes the current
    /// row's token, and the retry is a second save rather than an automatic write. The sentence
    /// names the retry key `keys` binds there (MOD-67 M6 D4): `form.save` in the quirks and spec
    /// editors, where `Enter` breaks the line, `field.submit` in the tag editor (MOD-115 M3 D-5).
    fn on_stale(&mut self, snapshot: &BoxesSnapshot, keys: &Keys) {
        let save = keys.label(views::BOXES_EDITOR, Act::FormSave);
        let submit = keys.label(views::CAPTURE, Act::FieldSubmit);
        self.busy = None;
        self.replace(snapshot);
        // MOD-51 F-6(a): the probe spec editor is over no box, so it is settled before
        // `editor_box` would take it for no editor at all. Its token is the overlay row's, `None`
        // when the row is gone (the retry is then an insert).
        if let Mode::Spec(editor) = &mut self.mode {
            editor.expected = snapshot.spec.stored.as_ref().map(|row| row.updated_at);
            self.notice = Some(changed_elsewhere(save.as_deref()));
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
                changed_elsewhere(submit.as_deref())
            }
            (Some(token), Mode::Quirks(editor)) => {
                editor.expected = token;
                changed_elsewhere(save.as_deref())
            }
            (Some(token), Mode::Executor(flip)) => {
                flip.expected = token;
                changed_elsewhere_executor(
                    keys.label(views::BOXES_EXECUTOR, Act::ConfirmYes)
                        .as_deref(),
                )
            }
            (Some(_), Mode::Browse | Mode::Spec(_)) => changed_elsewhere(submit.as_deref()),
            (None, _) => {
                self.mode = Mode::Browse;
                DELETED_ELSEWHERE.to_owned()
            }
        };
        self.notice = Some(notice);
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
            self.notice = Some(in_flight(name));
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

    /// One key while an editor is open: the widget answers first, so every letter is text
    /// (MOD-67 D6, skeleton (a)).
    ///
    /// The quirks and spec editors then take `form.save` from the resolver: a `TextArea` passes
    /// every chord, `ctrl-s` included (MOD-67 M4 D6), so this is their only save. Everything else
    /// the widget passes on is swallowed rather than offered to the shell, except the chords the
    /// mode's stack passes (CONTROL, ALT, function keys: D5), so `ctrl-c` still quits and `F1`
    /// opens help.
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
            FieldOutcome::Consumed => return Handled::Consumed,
            // `Enter` (tags); a `TextArea` (quirks, spec) never submits since MOD-67 M4 D6.
            FieldOutcome::Submit => {
                self.submit(ctx);
                return Handled::Consumed;
            }
            FieldOutcome::Cancel => {
                // `busy` stays: a save already sent is still answered, and its reply is what
                // clears it.
                self.mode = Mode::Browse;
                self.notice = None;
                return Handled::Consumed;
            }
            FieldOutcome::Pass => {}
        }
        let stack = self.stack();
        let chord = ctx.pressed(&key);
        // `form.save` is the first layer below the view's, so the first candidate decides; only
        // the quirks and spec editors' stack offers it.
        if ctx.keys().actions(stack, chord).first() == Some(&Act::FormSave) {
            self.submit(ctx);
            return Handled::Consumed;
        }
        modal_rest(stack, chord)
    }

    /// One key over the executor confirmation (MOD-41 plan D10; MOD-67 D6, skeleton (b)): `y`
    /// (`yes`) writes, `n`/`Esc` (`no`) cancel, the chords [`views::BOXES_EXECUTOR`] passes
    /// (CONTROL, ALT, function keys) go to the shell, so `ctrl-c` still quits, and every other
    /// key is swallowed. `alt-y` is not `y`.
    fn on_executor_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
        let stack = views::BOXES_EXECUTOR;
        let chord = ctx.pressed(&key);
        // The answers' layer comes before the global one, so the first candidate decides.
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes) => self.submit(ctx),
            Some(Act::ConfirmNo) => {
                // `busy` stays, as in `on_editor_key`: a write already sent is still answered.
                self.mode = Mode::Browse;
                self.notice = None;
            }
            _ => return modal_rest(stack, chord),
        }
        Handled::Consumed
    }

    /// `Enter` (tags), `ctrl-s` (quirks) or `y` (executor): one `EditBox` with only the edited
    /// field, when the value differs from what the editor opened on (D48, D59); a flip always
    /// differs. `ctrl-s` (probe spec, MOD-51 D6): one `SetProbeSpec` with the parsed overlay,
    /// or a clear when the text is blank. The editor stays open until the reply.
    fn submit(&mut self, ctx: &Ctx<'_>) {
        if let Some(name) = self.busy {
            self.notice = Some(in_flight(name));
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
        // Browse (MOD-67 D6, skeleton (c)): the first candidate this state accepts; a global act
        // or a declined one is the shell's. `executor` (MOD-41 plan D10) shadows the global
        // workspace switcher only while a box is listed to act on; over no list it declines, so
        // the switcher is still one key away (`STATE_GUARDED`). `Ctrl+W` is another chord: the
        // global waiting list (MOD-69 D7). Resolved from the chord pressed (MOD-115 M3 MD-A).
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::BOXES_BROWSE, chord) {
            match act {
                Act::ListDown => self.move_selection(true),
                Act::ListUp => self.move_selection(false),
                Act::BoxesEditTags => self.open_tags(),
                Act::BoxesEditQuirks => self.open_quirks(),
                Act::BoxesExecutor
                    if self.unavailable.is_none() && self.selected_record().is_some() =>
                {
                    self.open_executor();
                }
                // MOD-51 D6: the probe spec is app-wide, so `s` needs no listed box.
                Act::BoxesEditSpec => self.open_spec(),
                Act::BoxesProbe => self.probe(ctx),
                // Always allowed (the kinds section's reason): re-reading is how a section that
                // lost a reply recovers, and a read never touches an open editor's token.
                Act::Reload => ctx.request(StoreRequest::Boxes),
                // Only when there is something to clear: a section that swallowed every `Esc`
                // would take the one the shell uses to close an overlay over it.
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                _ => continue, // a global act, or one this state declines
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Boxes(snapshot) => self.on_boxes(snapshot, ctx.keys()),
            StoreReply::BoxesStale(snapshot) => self.on_stale(snapshot, ctx.keys()),
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
            // For a probe spec save (MOD-51 D6) the sentence is the probe's own when the overlay
            // was refused (D3), the store's when the write was.
            StoreReply::Failed { request, message }
                if *request == EDIT_NAME || *request == SPEC_NAME =>
            {
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

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        self.scroll.set(None);
        let width = usize::from(area.width).max(1);
        let notices: Vec<Line<'static>> = self
            .notice
            .as_deref()
            .map(|text| {
                let tone = if is_error(text) {
                    Tone::Error
                } else {
                    Tone::Info
                };
                wrapped(text, width)
                    .into_iter()
                    .map(|line| notice(&line, tone, theme))
                    .collect()
            })
            .unwrap_or_default();
        // MOD-67 M6 T2: the keys row went to the shell's hint row; the body gains it.
        let [body, notice_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(notices.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        let reload = ctx.keys().label(views::BOXES_BROWSE, Act::Reload);
        // An editor that still takes keys stays on screen under a refused read: the refusal takes
        // the body's first line and the editor keeps the rest.
        let under_refusal = |frame: &mut Frame<'_>, why: &str| -> Rect {
            let [refusal, rest] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
            frame.render_widget(
                Paragraph::new(notice(&format!("{UNAVAILABLE}: {why}"), Tone::Error, theme)),
                refusal,
            );
            rest
        };

        if let (Mode::Spec(editor), Some(snapshot)) = (&self.mode, &self.snapshot) {
            // MOD-51 F-6(b): an editor that takes keys stays on screen (the rule above), box or
            // no box.
            let area = match &self.unavailable {
                Some(why) => under_refusal(frame, why),
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
                    let rest = under_refusal(frame, why);
                    self.render_body(frame, rest, snapshot, ctx);
                }
                // The refusal wins the body even with a list behind it: what is on screen
                // would otherwise be boxes nothing has confirmed since the outage started (the
                // kinds rule). Its next step follows in the empty state's own role.
                (Some(why), _) => {
                    let next = asks_again(reload.as_deref());
                    let mut lines: Vec<Line<'static>> =
                        wrapped(&format!("{UNAVAILABLE}: {why}"), width)
                            .into_iter()
                            .map(|line| notice(&line, Tone::Error, theme))
                            .collect();
                    lines.extend(empty::lines("", NextStep(&next), theme).pop());
                    frame.render_widget(Paragraph::new(lines), body);
                }
                (None, None) => empty::render(
                    frame,
                    body,
                    NOT_READ,
                    NextStep(&reads_them(reload.as_deref())),
                    theme,
                ),
                // MOD-51 LOW-6: the spec is app-wide, so it is drawn with no box listed too: a
                // save over an empty list shows the new source and digest, and an ignored
                // overlay says why.
                (None, Some(snapshot)) if snapshot.boxes.is_empty() => {
                    let mut lines: Vec<Line<'static>> = wrapped(NO_BOXES, width)
                        .into_iter()
                        .map(|line| Line::styled(line, theme.dim))
                        .collect();
                    lines.extend(empty::lines("", NextStep(NO_BOXES_NEXT), theme).pop());
                    lines.push(Line::default());
                    lines.extend(spec_lines(&snapshot.spec, width, theme));
                    frame.render_widget(Paragraph::new(lines), body);
                }
                (None, Some(snapshot)) => self.render_body(frame, body, snapshot, ctx),
            }
        }

        frame.render_widget(Paragraph::new(notices), notice_area);
    }
}

impl BoxesSection {
    /// The list and the detail pane side by side.
    fn render_body(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &BoxesSnapshot,
        ctx: &Ctx<'_>,
    ) {
        let [list, detail] =
            Layout::horizontal([Constraint::Length(LIST_WIDTH), Constraint::Min(0)]).areas(area);
        self.render_list(frame, list, snapshot, ctx);
        if let Some(record) = self.selected_record() {
            frame.render_widget(
                Paragraph::new(self.detail(record, snapshot, detail.width, ctx)),
                detail,
            );
        }
    }

    /// The list pane: one `kit::list` row per box, the selected one behind the gutter in the
    /// tab's accent and in the band, scrolled so the selection stays on screen.
    fn render_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        snapshot: &BoxesSnapshot,
        ctx: &Ctx<'_>,
    ) {
        // One column is the gap between the two panes, and one more the list's gutter.
        let area = Rect {
            width: area.width.saturating_sub(1),
            ..area
        };
        let room = usize::from(area.width.saturating_sub(1));
        let rows: Vec<list::Row<'static>> = snapshot
            .boxes
            .iter()
            .map(|record| list::Row {
                line: Line::styled(list_label(record, snapshot, room), ctx.theme.base),
                kind: RowKind::Item,
            })
            .collect();
        let at = self
            .selected
            .and_then(|id| snapshot.boxes.iter().position(|record| record.row.id == id));
        let offset =
            list::scroll_into_view(at.unwrap_or(0), 0, usize::from(area.height), rows.len());
        list::render(frame, area, &rows, at, offset, ctx.accent(), ctx.theme);
        // Beside the list's rows (RV1 M2's span): the detail pane beside them does not scroll.
        self.scroll
            .set(Some(ScrollSpan::new(rows.len(), offset).rows(area)));
    }

    /// The detail pane for one box: a `kit::form` with a 14-wide label column, one read-only row
    /// per field; an open editor is its row's value and the form's focus, whichever box is
    /// selected, so a reload that moved the selection never leaves someone typing into a field
    /// they cannot see. The spec the next probe runs under follows the form.
    fn detail(
        &self,
        record: &BoxRecord,
        snapshot: &BoxesSnapshot,
        width: u16,
        ctx: &Ctx<'_>,
    ) -> Vec<Line<'static>> {
        let theme = ctx.theme;
        let row = &record.row;
        let value_width = Form::new()
            .label_width(LABEL_WIDTH)
            .value_width(width)
            .max(1);
        let read = |name: &'static str, value: String| {
            form::Field::new(name, form::Value::ReadOnly(value.into()))
        };
        let this_box = snapshot.this_box == Some(row.id);
        let mut fields = vec![
            read(
                "host",
                format!(
                    "{}{}",
                    row.hostname,
                    if this_box { " (this box)" } else { "" }
                ),
            ),
            read(
                "os",
                format!(
                    "{} {} \u{b7} {}",
                    label::os_family(row.os_family),
                    row.os_version,
                    row.arch
                ),
            ),
            read("cpu", row.cpu.clone()),
            read(
                "ram",
                row.ram_mb
                    .map_or_else(|| "unknown".to_owned(), |mb| format!("{mb} MB")),
            ),
            read("gpu", gpu(row.gpu_present, row.gpu_vendor.as_deref())),
            read("htui", row.htui_version.clone()),
            read(
                "last probe",
                row.last_probed_at.map_or_else(
                    || "never probed".to_owned(),
                    |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
                ),
            ),
            read(
                "spec",
                format!(
                    "probed under the current spec: {}",
                    if record.probe_spec_digest.as_deref() == Some(snapshot.spec.digest.as_str()) {
                        "yes"
                    } else {
                        "no"
                    }
                ),
            ),
            read("probed tags", tag_list(&row.probed_tags)),
        ];
        let mut focus = None;

        match &self.mode {
            // D50: every tag seen on a listed box, as the field's help under it.
            Mode::Tags(editor) => {
                focus = Some(fields.len());
                fields.push(
                    form::Field::new("declared tags", form::Value::Text(&editor.input))
                        .help(format!("seen: {}", seen(snapshot))),
                );
            }
            _ => fields.push(read("declared tags", tag_list(&row.declared_tags))),
        }

        match &self.mode {
            Mode::Quirks(editor) => {
                focus = Some(fields.len());
                fields.push(form::Field::new(
                    "quirks",
                    form::Value::Lines(editor.input.lines(value_width, QUIRKS_HEIGHT, true, theme)),
                ));
            }
            _ if row.quirks.is_empty() => fields.push(read("quirks", "(none)".to_owned())),
            _ => fields.push(form::Field::new(
                "quirks",
                form::Value::Lines(
                    row.quirks
                        .lines()
                        .map(|line| Line::styled(line.to_owned(), theme.base))
                        .collect(),
                ),
            )),
        }

        match &self.mode {
            // The flip names its own box, kept from open: after a reload that dropped it the
            // selection is another box, and `y` still writes to the flip's.
            Mode::Executor(flip) => {
                focus = Some(fields.len());
                fields.push(read("executor", flip.question()));
            }
            _ => fields.push(read("executor", Executor::of(&row.settings).to_string())),
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
            fields.push(read("tools", "(none)".to_owned()));
        } else {
            fields.push(read("tools", tools.join(", ")));
        }

        let form = fields
            .into_iter()
            .fold(Form::new().label_width(LABEL_WIDTH), Form::field);
        // An open editor is the focused row: its label is marked and its value takes the keys.
        let form = match focus {
            Some(item) => form.focus(item).editing(true),
            None => form,
        };
        let mut lines = form.lines(width, ctx.accent(), theme);
        lines.push(Line::default());
        // Behind the form's gutter cell, so the spec lines up with the labels above it.
        let room = usize::from(width.saturating_sub(1)).max(1);
        lines.extend(
            spec_lines(&snapshot.spec, room, theme)
                .into_iter()
                .map(|line| {
                    let mut spans = vec![Span::raw(" ")];
                    spans.extend(line.spans);
                    Line::from(spans).style(line.style)
                }),
        );
        lines
    }

    /// The stack of the current mode (MOD-67 D3, D4): the one place a mode maps to its keys, read
    /// by `key_stack`, the key handlers and the hint.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::BOXES_BROWSE,
            Mode::Tags(_) => views::CAPTURE,
            Mode::Quirks(_) | Mode::Spec(_) => views::BOXES_EDITOR,
            Mode::Executor(_) => views::BOXES_EXECUTOR,
        }
    }

    /// The keys this mode binds, through its stack (MOD-67 D9), plus what is in flight.
    fn hint_text(&self, keys: &Keys) -> String {
        // MOD-51 F-3: `s` is offered exactly where it works, a read that was not refused.
        let readable = self.unavailable.is_none() && self.snapshot.is_some();
        let listed = readable && !self.boxes().is_empty();
        let spec = match self.mode {
            Mode::Browse if listed => HINT_BROWSE,
            Mode::Browse if readable => HINT_NO_LIST,
            Mode::Browse => HINT_RELOAD,
            Mode::Tags(_) => HINT_TAGS,
            Mode::Quirks(_) => HINT_QUIRKS,
            Mode::Executor(_) => HINT_EXECUTOR,
            Mode::Spec(_) => HINT_SPEC,
        };
        let mut hint = keys.hint(self.stack(), spec);
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
/// the same hostname (PRD `:262`), and ` (this box)` on this box, in `room` cells.
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
    let host_room = room.saturating_sub(cell_width(&tail)).max(1);
    let mut label = cells::clip(&row.hostname, host_room);
    label.push_str(&tail);
    cells::clip(&label, room)
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
    let width = usize::from(area.width).max(1);
    let mut lines = spec_lines(&snapshot.spec, width, theme);
    lines.extend(
        wrapped(SPEC_TITLE, width)
            .into_iter()
            .map(|line| Line::styled(line, theme.dim)),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    use htui_core::model::{BoxRow, OsFamily, UserId};

    /// MOD-67 M6 D4: the stale notices name the retry key, and say a save (or nothing) unbound.
    /// The save-key editors' sentence is the shared one (MOD-115 M3 D-5).
    #[test]
    fn the_stale_notices_name_the_retry_key_or_none() {
        assert_eq!(
            changed_elsewhere(Some("Ctrl+s")),
            "changed elsewhere since you opened it \u{2014} reloaded; Ctrl+s retries against the \
             current row"
        );
        assert_eq!(
            changed_elsewhere(None),
            "changed elsewhere since you opened it \u{2014} reloaded; save again to retry against \
             the current row"
        );
        assert_eq!(
            changed_elsewhere_executor(Some("y")),
            "changed elsewhere since you opened it \u{2014} reloaded; y retries against the current \
             row"
        );
        assert_eq!(
            changed_elsewhere_executor(None),
            "changed elsewhere since you opened it \u{2014} reloaded"
        );
    }

    /// MOD-115 M3: a write in flight reads in words. Every request name has its own phrase, and
    /// no refusal prints the name itself.
    #[test]
    fn a_write_in_flight_reads_in_words() {
        for name in REQUEST_NAMES {
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
        assert_eq!(
            in_flight(EDIT_NAME),
            "still saving the box \u{2014} wait for the store's answer"
        );
    }

    /// MOD-115 M3 C6: the editors' hints read as before under the compiled keys.
    #[test]
    fn the_editor_hints_read_as_before_under_the_compiled_keys() {
        let keys = Keys::compiled();
        assert_eq!(
            keys.hint(views::CAPTURE, HINT_TAGS),
            "Enter saves \u{b7} Esc cancels \u{b7} comma-separated"
        );
        assert_eq!(
            keys.hint(views::BOXES_EDITOR, HINT_QUIRKS),
            "Ctrl+s saves \u{b7} Esc cancels \u{b7} Enter breaks the line"
        );
        assert_eq!(
            keys.hint(views::BOXES_EDITOR, HINT_SPEC),
            "Ctrl+s saves \u{b7} Esc cancels \u{b7} Enter breaks the line \u{b7} blank clears"
        );
    }

    /// A box called `hostname`, every other field a placeholder.
    fn record(hostname: &str) -> BoxRecord {
        let at = DateTime::<Utc>::UNIX_EPOCH;
        BoxRecord {
            row: BoxRow {
                id: BoxId::new(),
                user_id: UserId::from_uuid(uuid::Uuid::nil()),
                hostname: hostname.to_owned(),
                os_family: OsFamily::Linux,
                os_version: "6.8".to_owned(),
                arch: "x86_64".to_owned(),
                cpu: String::new(),
                ram_mb: None,
                gpu_present: false,
                gpu_vendor: None,
                htui_version: "0.1.0".to_owned(),
                probed_tags: Vec::new(),
                declared_tags: Vec::new(),
                quirks: String::new(),
                settings: serde_json::json!({}),
                registered_at: at,
                last_seen_at: at,
                last_probed_at: None,
                updated_at: at,
                edit_version: 0,
            },
            tools: Vec::new(),
            probe_spec_digest: None,
        }
    }

    /// MOD-60: a CJK hostname is clipped in cells, so the row stays inside its room and the
    /// ` (this box)` marker that tells boxes apart still shows.
    #[test]
    fn a_wide_hostname_keeps_the_suffix_and_the_marker() {
        let room = 24;
        let record = record(&"\u{6f22}".repeat(20));
        let snapshot = BoxesSnapshot {
            this_box: Some(record.row.id),
            boxes: vec![record.clone()],
            spec: SpecView {
                digest: String::new(),
                overlay: false,
                error: None,
                stored: None,
            },
        };

        let out = list_label(&record, &snapshot, room);

        assert!(cell_width(&out) <= room, "{out:?} against {room}");
        // ` (this box)` takes 11 cells and leaves the hostname 13: six two-cell clusters and `…`.
        assert_eq!(out, format!("{}\u{2026} (this box)", "\u{6f22}".repeat(6)));
    }

    /// MOD-69 D7 (blueprint H-14): `Ctrl+W` is the global waiting list, so over a listed box it
    /// passes on to the shell; a plain `w` still opens the executor confirmation.
    #[cfg(feature = "testkit")]
    #[tokio::test]
    async fn ctrl_w_over_a_listed_box_passes_and_w_still_opens_the_executor() {
        use crate::store_worker::serve;
        use crate::testkit::SectionBench;
        use htui_core::store::MemStore;
        use htui_store::Backend;

        let reply = serve(&Backend::memory(MemStore::demo()), &StoreRequest::Boxes).await;
        assert!(matches!(reply, StoreReply::Boxes(_)), "{reply:?}");
        let bench = SectionBench::new().await;
        let mut section = BoxesSection::new();
        bench.reply(&mut section, &reply);
        let _ = bench.drained();

        assert_eq!(bench.key(&mut section, "ctrl-w"), Handled::Pass);
        assert!(!section.captures_input(), "Ctrl+W opens no confirmation");
        assert_eq!(bench.key(&mut section, "w"), Handled::Consumed);
        assert!(
            section.captures_input(),
            "w opens the executor confirmation"
        );
    }
}
