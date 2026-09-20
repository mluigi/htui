use htui_core::model::Scope;
use htui_store::qdrant_settings::QdrantSettings;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::app::{Ctx, Handled};
use crate::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};
use crate::ui::{FieldOutcome, TextField};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const NOT_READ: &str = "not read yet";
const STORED: &str = "stored";
const CLEARED: &str = "the Qdrant settings are gone from the keyring";
const NO_URL_YET: &str = "no Qdrant URL is stored; type one and press Enter";
const REPLACES_URL: &str = "Enter replaces the stored Qdrant URL";
const NOT_STORED: &str = "not stored";
const UNREADABLE: &str = "the keyring could not be read";
const UNREADABLE_GUIDE: &str = "the keyring could not be read; Enter tries to store a Qdrant URL in it";
const EMPTY_URL: &str = "nothing typed; the stored Qdrant settings are unchanged";
const CONFIRM_CLEAR: &str = "Remove the Qdrant settings from the keyring? y / n";

const HINT_BROWSE: &str = "e edit \u{b7} c clear \u{b7} r reload";
const HINT_NO_SNAPSHOT: &str = "r reload";
const HINT_EDITING: &str = "Enter continue \u{b7} Esc cancel";
const HINT_EDITING_KEY: &str = "Enter store \u{b7} Esc cancel \u{b7} typed text is never shown";
const HINT_CONFIRM: &str = "y confirm \u{b7} n / Esc cancel";

struct Editor {
    input: TextField,
}

#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    EditingUrl(Editor),
    EditingKey { url: String, editor: Editor },
    ConfirmClear,
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

pub struct QdrantSection {
    mode: Mode,
    notice: Option<Notice>,
    snapshot: Option<QdrantSnapshot>,
    unavailable: Option<String>,
    busy: Option<&'static str>,
    opened_for_empty: bool,
}

impl QdrantSection {
    pub const ID: SectionId = SectionId("qdrant");

    pub fn new() -> Self {
        Self {
            mode: Mode::default(),
            notice: None,
            snapshot: None,
            unavailable: None,
            busy: None,
            opened_for_empty: false,
        }
    }

    fn state(&self) -> Option<&QdrantState> {
        self.snapshot.as_ref().map(|s| &s.url_state)
    }

    fn blocked(&self) -> bool {
        self.unavailable.is_some() || self.snapshot.is_none() || self.busy.is_some()
    }

    fn settings_row_refusal(&self) -> String {
        match self.state() {
            Some(QdrantState::Stored) => unreachable!("this UI only blocks the missing states"),
            Some(QdrantState::NotStored) => "there are no Qdrant settings to clear".to_owned(),
            Some(QdrantState::Unreadable(e)) => format!("keyring unreadable: {}", e),
            None => NOT_READ.to_owned(),
        }
    }

    fn send(&mut self, request: StoreRequest, ctx: &mut Ctx<'_>) {
        self.busy = Some(request.name());
        ctx.request(request);
    }

    fn open_edit_url(&mut self) {
        self.notice = None;
        self.mode = Mode::EditingUrl(Editor {
            input: TextField::new(),
        });
    }
    
    fn open_edit_key(&mut self, url: String) {
        self.notice = None;
        self.mode = Mode::EditingKey { 
            url, 
            editor: Editor { input: TextField::masked() }
        };
    }

    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.mode = Mode::Browse;
            return Handled::Consumed;
        }

        let (mut url_to_transition, mut key_to_submit) = (None, None);

        match &mut self.mode {
            Mode::EditingUrl(editor) => {
                match editor.input.on_key(key) {
                    FieldOutcome::Consumed => return Handled::Consumed,
                    FieldOutcome::Cancel => {
                        self.mode = Mode::Browse;
                        return Handled::Consumed;
                    }
                    FieldOutcome::Submit => {
                        let text = editor.input.text().unwrap_or("").trim();
                        if text.is_empty() {
                            self.mode = Mode::Browse;
                            self.say(EMPTY_URL);
                            return Handled::Consumed;
                        }
                        url_to_transition = Some(text.to_owned());
                    }
                    _ => return Handled::Pass,
                }
            }
            Mode::EditingKey { url, editor } => {
                match editor.input.on_key(key) {
                    FieldOutcome::Consumed => return Handled::Consumed,
                    FieldOutcome::Cancel => {
                        self.mode = Mode::Browse;
                        return Handled::Consumed;
                    }
                    FieldOutcome::Submit => {
                        key_to_submit = Some((url.clone(), editor.input.text().unwrap_or("").to_owned()));
                    }
                    _ => return Handled::Pass,
                }
            }
            _ => return Handled::Pass,
        }

        if let Some(url) = url_to_transition {
            self.open_edit_key(url);
            return Handled::Consumed;
        }

        if let Some((url, key_text)) = key_to_submit {
            let api_key = if key_text.is_empty() { None } else { Some(key_text) };
            match QdrantSettings::new(url, api_key) {
                Ok(settings) => {
                    self.send(StoreRequest::SetQdrantSettings(settings), ctx);
                }
                Err(err) => {
                    self.refuse(err.to_string());
                }
            }
        }
        Handled::Consumed
    }

    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::Pass;
        }
        match &mut self.mode {
            Mode::ConfirmClear => match key.code {
                KeyCode::Char('y') => {
                    self.mode = Mode::Browse;
                    self.send(StoreRequest::ClearQdrantSettings, ctx);
                }
                KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            _ => return Handled::Pass,
        }
        Handled::Consumed
    }

    fn hint_text(&self) -> String {
        let keys = match self.mode {
            Mode::EditingUrl(_) => HINT_EDITING,
            Mode::EditingKey { .. } => HINT_EDITING_KEY,
            Mode::ConfirmClear => HINT_CONFIRM,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        match self.busy {
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {busy} in flight")
            }
            _ => keys.to_owned(),
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

    fn on_snapshot(&mut self, snapshot: &QdrantSnapshot) {
        let write = self.busy.take();
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());

        match write {
            Some("set_qdrant_settings") => {
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
            self.open_edit_url();
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

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::EditingUrl(_) | Mode::EditingKey { .. } => return self.on_editor_key(key, ctx),
            Mode::ConfirmClear => return self.on_confirm_key(key, ctx),
            Mode::Browse => {}
        }
        match key.code {
            KeyCode::Char('e') => {
                if !self.blocked() {
                    self.open_edit_url();
                }
                Handled::Consumed
            }
            KeyCode::Char('c') => {
                if !self.blocked() {
                    if matches!(self.state(), Some(QdrantState::Stored)) {
                        self.notice = None;
                        self.mode = Mode::ConfirmClear;
                    } else {
                        self.refuse(self.settings_row_refusal());
                    }
                }
                Handled::Consumed
            }
            KeyCode::Char('r') => Handled::Pass,
            _ => Handled::Pass,
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
            None => NOT_READ.to_owned(),
        };

        let key_val = match self.snapshot.as_ref().map(|s| &s.key_state) {
            Some(QdrantState::Stored) => "stored \u{b7} <redacted>".to_owned(),
            Some(QdrantState::NotStored) => NOT_STORED.to_owned(),
            Some(QdrantState::Unreadable(_)) => UNREADABLE.to_owned(),
            None => NOT_READ.to_owned(),
        };

        let style = ctx.theme.base;
        for (n, chunk) in wrapped(&url_val, room.saturating_sub(10)).into_iter().enumerate() {
            let l = if n == 0 { "URL" } else { "" };
            lines.push(Line::styled(format!("  {l:<5}  {chunk}"), style));
        }
        for (n, chunk) in wrapped(&key_val, room.saturating_sub(10)).into_iter().enumerate() {
            let l = if n == 0 { "Key" } else { "" };
            lines.push(Line::styled(format!("  {l:<5}  {chunk}"), style));
        }

        match &self.mode {
            Mode::Browse => {}
            Mode::EditingUrl(editor) => {
                let l = "URL: ";
                let field_room = area.width.saturating_sub(l.chars().count() as u16);
                let mut spans = vec![Span::styled(l, ctx.theme.accent)];
                spans.extend(
                    editor
                        .input
                        .line(field_room.max(1), true, ctx.theme)
                        .spans,
                );
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
            Mode::EditingKey { editor, .. } => {
                let l = "Key: ";
                let field_room = area.width.saturating_sub(l.chars().count() as u16);
                let mut spans = vec![Span::styled(l, ctx.theme.accent)];
                spans.extend(
                    editor
                        .input
                        .line(field_room.max(1), true, ctx.theme)
                        .spans,
                );
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

        let keys = self.hint_text();
        let hint_line = match (&self.notice, &self.mode) {
            (Some(notice), Mode::Browse) => {
                let text = notice.text().to_owned();
                let sty = if notice.is_error() { ctx.theme.error } else { ctx.theme.dim };
                if keys.chars().count() + text.chars().count() + 3 > room {
                    Line::styled(text, sty)
                } else {
                    Line::from(vec![
                        Span::styled(format!("{keys} \u{b7} "), ctx.theme.dim),
                        Span::styled(text, sty),
                    ])
                }
            }
            _ => Line::styled(keys, ctx.theme.dim),
        };
        lines.push(hint_line);

        let p = ratatui::widgets::Paragraph::new(lines);
        frame.render_widget(p, area);
    }
}
