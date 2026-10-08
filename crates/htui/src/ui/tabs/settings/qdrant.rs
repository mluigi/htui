use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::{Ctx, Handled};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use crate::secrets_settings::{DEMO_SESSION, Redacted};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};
use crate::ui::{FieldOutcome, TextField};
use crossterm::event::KeyEvent;

const NOT_READ: &str = "not read yet";
const STORED: &str = "stored";
const CLEARED: &str = "the Qdrant settings are gone from the keyring";
const NO_URL_YET: &str = "no Qdrant URL is stored; type one and press Enter";
const EMPTY_KEY: &str = "nothing typed; the stored API key is unchanged";
const REPLACES_URL: &str = "Enter replaces the stored Qdrant URL";
const NOT_STORED: &str = "not stored";
/// CLEAN-8 #9: both rows in a demo session, which has no keyring.
const DEMO_ROW: &str = "n/a in a demo session";
const UNREADABLE: &str = "the keyring could not be read";
const UNREADABLE_GUIDE: &str =
    "the keyring could not be read; Enter tries to store a Qdrant URL in it";
const CONFIRM_CLEAR: &str = "Remove the Qdrant settings from the keyring? y / n";

// The hint rows (MOD-67 M3 D9): rendered through the mode's stack, so a rebound key shows its
// new chord. `Enter`/`Esc` in the editors are the text field's own keys (D13), written as text.
const HINT_BROWSE: HintSpec = &[
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Clear, "clear all"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_NO_SNAPSHOT: HintSpec = &[
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_EDITING: HintSpec = &[Hint::Text("Enter continue"), Hint::Text("Esc cancel")];
const HINT_EDITING_KEY: HintSpec = &[
    Hint::Text("Enter store"),
    Hint::Text("Esc cancel"),
    Hint::Text("typed text is never shown"),
];
const HINT_CONFIRM: HintSpec = &[
    Hint::One(Act::ConfirmYes, "confirm"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Url,
    Key,
}
impl Row {
    const ALL: [Self; 2] = [Self::Url, Self::Key];
}
struct Editor {
    input: TextField,
}

#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    ConfirmClear,
    EditingUrl(Editor),
    EditingKey(Editor),
}

#[derive(Clone, PartialEq, Eq)]
enum Notice {
    Info(String),
    Error(String),
}

impl Notice {
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }
    fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Qdrant section of Settings tab
pub struct QdrantSection {
    mode: Mode,
    notice: Option<Notice>,
    snapshot: Option<QdrantSnapshot>,
    unavailable: Option<String>,
    busy: Option<&'static str>,
    /// An `r` re-read is out. The worker answers in order, and a newer read supersedes an older one
    /// at the app's staleness gate, so the next snapshot is that read's answer, never a write's.
    read_out: bool,
    opened_for_empty: bool,
    cursor: usize,
}

impl std::fmt::Debug for QdrantSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantSection").finish_non_exhaustive()
    }
}

impl Default for QdrantSection {
    fn default() -> Self {
        Self::new()
    }
}

impl QdrantSection {
    /// The ID for the qdrant settings section
    pub const ID: SectionId = SectionId("qdrant");

    /// Create a new QdrantSection
    pub fn new() -> Self {
        Self {
            mode: Mode::default(),
            notice: None,
            snapshot: None,
            unavailable: None,
            busy: None,
            read_out: false,
            opened_for_empty: false,
            cursor: 0,
        }
    }

    fn state(&self) -> Option<&QdrantState> {
        self.snapshot.as_ref().map(|s| &s.url_state)
    }

    /// CLEAN-8 #9: the snapshot is a demo session's, which has no keyring to edit.
    fn demo(&self) -> bool {
        self.state() == Some(&QdrantState::NotApplicable)
    }

    fn row(&self) -> Row {
        Row::ALL[self.cursor]
    }

    fn move_cursor(&mut self, down: bool) {
        // Moving the cursor is not a write, so a write in flight does not stop it.
        if self.snapshot.is_none() || self.unavailable.is_some() {
            return;
        }
        if down {
            self.cursor = (self.cursor + 1) % Row::ALL.len();
        } else {
            self.cursor = (self.cursor + Row::ALL.len() - 1) % Row::ALL.len();
        }
    }

    /// Whether a key that opens an editor or a question is refused right now. A write in flight
    /// says so, as the Connection section's does, and so does a demo session (CLEAN-8 #9), as the
    /// Secrets section's does; `r` is deliberately not on this path.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(format!("`{busy}` is still in flight"));
            return true;
        }
        if self.demo() {
            self.refuse(DEMO_SESSION.to_owned());
            return true;
        }
        self.unavailable.is_some() || self.snapshot.is_none()
    }

    fn open_edit(&mut self) {
        match self.row() {
            Row::Url => {
                self.mode = Mode::EditingUrl(Editor {
                    input: TextField::new(),
                });
            }
            Row::Key => {
                self.mode = Mode::EditingKey(Editor {
                    input: TextField::masked(),
                });
            }
        }
    }

    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let (outcome, url_to_submit, key_to_submit) = match &mut self.mode {
            Mode::EditingUrl(editor) => {
                let outcome = editor.input.on_key(key);
                let mut url = None;
                if let FieldOutcome::Submit = outcome {
                    url = Some(editor.input.text().unwrap_or("").trim().to_owned());
                }
                (outcome, url, None)
            }
            Mode::EditingKey(editor) => {
                let outcome = editor.input.on_key(key);
                let mut key_text = None;
                if let FieldOutcome::Submit = outcome {
                    // MOD-10 M4 blueprint A-6: the field is masked, so `text()` is `None`; `take`
                    // is its only read. The typed key is wiped here and travels redacted.
                    let raw = zeroize::Zeroizing::new(editor.input.take());
                    key_text = Some(Redacted::new(raw.trim().to_owned()));
                }
                (outcome, None, key_text)
            }
            _ => return Handled::Pass,
        };

        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                if let Some(url) = url_to_submit {
                    if url.is_empty() {
                        self.mode = Mode::Browse;
                        self.say("nothing typed; the stored URL is unchanged");
                        return Handled::Consumed;
                    }
                    self.mode = Mode::Browse;
                    self.send(StoreRequest::SetQdrantUrl(url), ctx);
                }
                if let Some(key_text) = key_to_submit {
                    self.mode = Mode::Browse;
                    // MOD-10 M4 R1 L-5: as the URL row, an empty submit changes nothing; removing
                    // the key stays on `c`, behind its question.
                    if key_text.is_empty() {
                        self.say(EMPTY_KEY);
                        return Handled::Consumed;
                    }
                    self.send(StoreRequest::SetQdrantApiKey(key_text), ctx);
                }
                Handled::Consumed
            }
            // The field passed it (`Tab`, arrows, a CONTROL chord …): the editor keeps it unless
            // the modal global layer admits it (MOD-67 D5, PA-5). `Tab` stays here rather than
            // switching tabs (ANA-26 §2.6 defect 2); `ctrl-c` and `F1` reach the shell.
            FieldOutcome::Pass => {
                if views::CAPTURE.passes(KeyChord::from_event(key)) {
                    Handled::Pass
                } else {
                    Handled::Consumed
                }
            }
        }
    }

    /// The clear question (MOD-67 M3 §6.4): `confirm.yes` clears, `confirm.no` goes back, a chord
    /// the modal global layer admits passes, and everything else is swallowed.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::QDRANT_CONFIRM;
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(stack, chord) {
            match act {
                Act::ConfirmYes => {
                    self.mode = Mode::Browse;
                    self.send(StoreRequest::ClearQdrantSettings, ctx);
                    return Handled::Consumed;
                }
                Act::ConfirmNo => {
                    self.mode = Mode::Browse;
                    return Handled::Consumed;
                }
                _ => break,
            }
        }
        if stack.passes(chord) {
            Handled::Pass
        } else {
            Handled::Consumed
        }
    }

    /// The stack of the current mode (MOD-67 D4): the one place a mode maps to its keys;
    /// `key_stack`, the key handlers and the hint all read it.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::QDRANT_BROWSE,
            Mode::ConfirmClear => views::QDRANT_CONFIRM,
            Mode::EditingUrl(_) | Mode::EditingKey(_) => views::CAPTURE,
        }
    }

    /// One key in Browse (MOD-67 M3 §6.2): whole chords through `QDRANT_BROWSE`, so `ctrl-e`
    /// edits nothing and `ctrl-r` reads nothing (defect 1).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::QDRANT_BROWSE, chord) {
            match act {
                Act::Edit => {
                    if !self.blocked() {
                        self.open_edit();
                    }
                    return Handled::Consumed;
                }
                Act::Clear => {
                    if !self.blocked() {
                        if matches!(self.state(), Some(QdrantState::Stored))
                            || matches!(
                                self.snapshot.as_ref().map(|s| &s.key_state),
                                Some(QdrantState::Stored)
                            )
                        {
                            self.notice = None;
                            self.mode = Mode::ConfirmClear;
                        } else {
                            self.refuse("qdrant settings unavailable".to_owned());
                        }
                    }
                    return Handled::Consumed;
                }
                Act::ListDown => {
                    self.move_cursor(true);
                    return Handled::Consumed;
                }
                Act::ListUp => {
                    self.move_cursor(false);
                    return Handled::Consumed;
                }
                // A re-read, never refused: it is how the unavailable state recovers (MOD-63). It
                // sets no `busy`, like the Connection section's `r`.
                Act::Reload => {
                    self.read_out = true;
                    ctx.request(StoreRequest::QdrantInfo);
                    return Handled::Consumed;
                }
                // A global act: the shell resolves the same stack and applies it.
                _ => continue,
            }
        }
        Handled::Pass
    }

    /// Sends one write and remembers its name until the reply, so `on_snapshot` can say what it
    /// did and a failure lands on this section (the Connection section's `send`).
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.notice = None;
        ctx.request(request);
    }

    /// The one line under the pane: the keys this mode binds, then the last outcome.
    ///
    /// The outcome only appears here in Browse. When both do not fit in `room` cells the outcome
    /// wins the line: the keys are on screen every other frame, and this is the only place the
    /// outcome appears.
    fn hint(&self, keys: &Keys, room: usize, theme: &crate::ui::Theme) -> Line<'static> {
        let keys = self.hint_text(keys);
        let (Some(notice), Mode::Browse) = (&self.notice, &self.mode) else {
            return Line::styled(keys, theme.dim);
        };
        let text = notice.text().to_owned();
        let sty = if notice.is_error() {
            theme.error
        } else {
            theme.dim
        };
        if cell_width(&keys) + cell_width(&text) + 3 > room {
            return Line::styled(text, sty);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, sty),
        ])
    }

    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match self.mode {
            Mode::EditingUrl(_) => HINT_EDITING,
            Mode::EditingKey(_) => HINT_EDITING_KEY,
            Mode::ConfirmClear => HINT_CONFIRM,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        let keys = keys.hint(self.stack(), spec);
        match self.busy {
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {busy} in flight")
            }
            _ => keys,
        }
    }

    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// A fresh snapshot, and the write that asked for it, if any, says what it did.
    ///
    /// A `Qdrant` reply names no request, so attribution is by order. A snapshot that lands while
    /// an `r` re-read is out answers that read and leaves `busy` alone, so a reload sent just before
    /// a write can never report the write as stored or cleared before it has run. A write sent
    /// before the `r` answers first, so its notice waits for the read's snapshot and is then true.
    fn on_snapshot(&mut self, snapshot: &QdrantSnapshot) {
        let write = if std::mem::take(&mut self.read_out) {
            None
        } else {
            self.busy.take()
        };
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());

        match write {
            Some("set_qdrant_url") | Some("set_qdrant_api_key") => {
                self.mode = Mode::Browse;
                self.say(STORED);
            }
            Some("clear_qdrant_settings") => {
                self.mode = Mode::Browse;
                self.say(CLEARED);
            }
            _ => {}
        }

        if write.is_none()
            && snapshot.url_state == QdrantState::NotStored
            && !self.opened_for_empty
            && matches!(self.mode, Mode::Browse)
        {
            self.opened_for_empty = true;
            self.open_edit();
            self.say(NO_URL_YET);
        }
    }
}

fn question(text: &str, room: usize, theme: &crate::ui::Theme) -> Vec<Line<'static>> {
    wrapped(text, room)
        .into_iter()
        .map(|line| Line::styled(line, theme.error))
        .collect()
}

impl SettingsSection for QdrantSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Qdrant"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        vec![StoreRequest::QdrantInfo]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        self.mode = Mode::Browse;
        self.opened_for_empty = false;
        self.notice = None;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// MOD-22 review M-1: a bracketed paste into the open URL or key field, whole; the masked key
    /// within its reservation or refused by name.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        let (Mode::EditingUrl(editor) | Mode::EditingKey(editor)) = &mut self.mode else {
            return Handled::Pass;
        };
        if !editor.input.on_paste(text) {
            ctx.emit(crate::app::Action::Error(
                crate::ui::text_field::PASTE_DOES_NOT_FIT.to_owned(),
            ));
        }
        Handled::Consumed
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::EditingUrl(_) | Mode::EditingKey(_) => self.on_editor_key(key, ctx),
            Mode::ConfirmClear => self.on_confirm_key(key, ctx),
            Mode::Browse => self.on_browse_key(key, ctx),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        if let StoreReply::Qdrant(snapshot) = reply {
            self.on_snapshot(snapshot);
        } else if let StoreReply::Failed { request, message } = reply {
            if self.busy == Some(*request) {
                self.busy = None;
                self.refuse(message.clone());
            } else if *request == "qdrant_info" {
                self.read_out = false;
                self.unavailable = Some(message.clone());
            }
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let room = usize::from(area.width).max(1);
        let mut lines = Vec::new();

        let url_val = match self.state() {
            Some(QdrantState::Stored) => {
                if let Some(summary) = self.snapshot.as_ref().and_then(|s| s.url_summary.as_ref()) {
                    format!("stored \u{b7} {}", summary)
                } else {
                    STORED.to_owned()
                }
            }
            Some(QdrantState::NotStored) => NOT_STORED.to_owned(),
            Some(QdrantState::Unreadable(_)) => UNREADABLE.to_owned(),
            Some(QdrantState::NotApplicable) => DEMO_ROW.to_owned(),
            None => NOT_READ.to_owned(),
        };

        let key_val = match self.snapshot.as_ref().map(|s| &s.key_state) {
            Some(QdrantState::Stored) => "stored \u{b7} <redacted>".to_owned(),
            Some(QdrantState::NotStored) => NOT_STORED.to_owned(),
            Some(QdrantState::Unreadable(_)) => UNREADABLE.to_owned(),
            Some(QdrantState::NotApplicable) => DEMO_ROW.to_owned(),
            None => NOT_READ.to_owned(),
        };

        for (index, row) in Row::ALL.into_iter().enumerate() {
            let style = if index == self.cursor {
                ctx.theme.selected
            } else {
                ctx.theme.base
            };

            let val = match row {
                Row::Url => &url_val,
                Row::Key => &key_val,
            };

            for (n, chunk) in wrapped(val, room.saturating_sub(10))
                .into_iter()
                .enumerate()
            {
                let l = if n == 0 {
                    match row {
                        Row::Url => "URL",
                        Row::Key => "Key",
                    }
                } else {
                    ""
                };
                lines.push(Line::styled(format!("  {l:<5}  {chunk}"), style));
            }
        }

        match &self.mode {
            Mode::Browse => {}
            Mode::EditingUrl(editor) => {
                let l = "URL: ";
                let field_room = area
                    .width
                    .saturating_sub(u16::try_from(cell_width(l)).unwrap_or(u16::MAX));
                let mut spans = vec![Span::styled(l, ctx.theme.accent)];
                spans.extend(editor.input.line(field_room.max(1), true, ctx.theme).spans);
                lines.push(Line::from(spans));
                let guide = match self.state() {
                    Some(QdrantState::Stored) => REPLACES_URL,
                    Some(QdrantState::Unreadable(_)) => UNREADABLE_GUIDE,
                    _ => NO_URL_YET,
                };
                let (text, sty) = match &self.notice {
                    Some(notice) if notice.is_error() => (notice.text(), ctx.theme.error),
                    Some(notice) => (notice.text(), ctx.theme.dim),
                    None => (guide, ctx.theme.dim),
                };
                lines.extend(
                    wrapped(text, room)
                        .into_iter()
                        .map(|line| Line::styled(line, sty)),
                );
            }
            Mode::EditingKey(editor) => {
                let l = "Key: ";
                let field_room = area
                    .width
                    .saturating_sub(u16::try_from(cell_width(l)).unwrap_or(u16::MAX));
                let mut spans = vec![Span::styled(l, ctx.theme.accent)];
                spans.extend(editor.input.line(field_room.max(1), true, ctx.theme).spans);
                lines.push(Line::from(spans));
                let guide = "Enter API key, or leave blank if none";
                let (text, sty) = match &self.notice {
                    Some(notice) if notice.is_error() => (notice.text(), ctx.theme.error),
                    Some(notice) => (notice.text(), ctx.theme.dim),
                    None => (guide, ctx.theme.dim),
                };
                lines.extend(
                    wrapped(text, room)
                        .into_iter()
                        .map(|line| Line::styled(line, sty)),
                );
            }
            Mode::ConfirmClear => {
                lines.extend(question(CONFIRM_CLEAR, room, ctx.theme));
            }
        }

        lines.push(self.hint(ctx.keys(), room, ctx.theme));

        let p = ratatui::widgets::Paragraph::new(lines);
        frame.render_widget(p, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Theme;

    /// MOD-60: the hint line measures the notice in cells, so a CJK notice that fits by `char`
    /// count but not on screen takes the line alone rather than overrunning it.
    #[test]
    fn a_wide_notice_takes_the_hint_line_alone() {
        let k = 10;
        let notice = "\u{6f22}".repeat(k);
        let section = QdrantSection {
            notice: Some(Notice::Error(notice.clone())),
            ..QdrantSection::new()
        };
        let keys = section.hint_text(Keys::compiled());
        let width = cell_width(&keys) + k + 3;

        let line = section.hint(Keys::compiled(), width, &Theme::default());

        assert_eq!(line.spans.len(), 1, "{line:?} against {width}");
        assert_eq!(line.spans[0].content, notice);
    }
}
