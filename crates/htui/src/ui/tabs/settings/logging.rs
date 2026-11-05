//! The logging section of the Settings tab (MOD-122 D5): the global log filter and the log file,
//! both applied at once and kept in `ui-state.toml`.
//!
//! It holds the [`LoggingControl`] `App` applies through and nothing else: every frame and every
//! key reads [`LoggingControl::state`], so the rows show what is in force, never a copy. A saved
//! edit leaves through `ctx.emit(Action::Logging(..))`; `App` applies and saves it. The filter is
//! checked here first by [`logging::parse_filter`], the one validator (B3), so a refused
//! directive keeps the editor open, says why and sends nothing. A file that cannot be opened is
//! the control's to refuse: the status line says so, and so does the line under the row.
//!
//! A field `--log`/`HTUI_LOG` or `HTUI_LOG_FILTER` set for this session says what set it and
//! refuses its edit (B5): those win for the session (D5), so a saved value would not apply.
//!
//! On the kit (MOD-115 M3), as `QueueSection`: one `kit::form` of read-only values, the focused
//! row's help under it, an open editor on the row it edits. Every key a hint names is the key in
//! force (MD-A); `e` edits, and `Enter` is no view default here (B7).

use std::cell::Cell;
use std::path::PathBuf;

use crossterm::event::KeyEvent;
use htui_core::model::Scope;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Action, Ctx, Handled, LoggingAction};
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::logging::{self, DEFAULT_FILTER, LoggingControl, LoggingState};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::form::{self, Field, Form};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{SectionId, SettingsSection};
use crate::ui::{FieldOutcome, TextField, Theme};

/// Browse's keys: `j/k · e edit` by default.
pub const HINT_BROWSE: HintSpec = &[
    Hint::Pair(Act::ListDown, Act::ListUp, ""),
    Hint::One(Act::Edit, "edit"),
];

/// An open editor's keys: the field's own submit and cancel through `CAPTURE`'s `[field]` layer.
pub const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "save"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::Text("empty resets"),
];

/// The filter row's help.
pub const FILTER_HELP: &str = "which lines htui keeps, as an EnvFilter directive such as info or \
    htui=debug,sqlx=warn; applied at once and kept in ui-state.toml";

/// The log file row's help.
pub const FILE_HELP: &str = "a file htui appends its log to, as --log does; empty writes none; \
    applied at once and kept in ui-state.toml";

/// What the log file row shows with no file.
const NO_FILE: &str = "none";

/// The label column: the widest label (`log file`) and two cells.
const LABEL_WIDTH: u16 = 10;

/// B5: why an overridden row refuses its edit. `by` is [`logging::SET_BY_FLAG`] or
/// [`logging::SET_BY_VAR`].
fn overridden(by: &str, row: Row) -> String {
    format!(
        "set by {by} for this session: start htui without it to change {} here",
        row.what()
    )
}

/// A typed log file made absolute against the directory htui runs in, so the path saved to
/// `ui-state.toml` names the same file whichever directory the next session starts from.
/// Without a current directory to resolve against, the path stays as typed.
fn absolute(text: String) -> PathBuf {
    std::path::absolute(&text).unwrap_or_else(|_| PathBuf::from(text))
}

/// One row of the section, in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The global filter directive.
    Filter,
    /// The log file, or none.
    File,
}

impl Row {
    /// Every row, in screen order: a row's index is its line's.
    const ALL: [Self; 2] = [Self::Filter, Self::File];

    /// The row's label.
    const fn label(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::File => "log file",
        }
    }

    /// What the row sets, in a sentence.
    const fn what(self) -> &'static str {
        match self {
            Self::Filter => "the log filter",
            Self::File => "the log file",
        }
    }

    /// The row's help while it is not overridden.
    const fn help(self) -> &'static str {
        match self {
            Self::Filter => FILTER_HELP,
            Self::File => FILE_HELP,
        }
    }

    /// The row's index in [`Row::ALL`], so its line's.
    const fn index(self) -> usize {
        match self {
            Self::Filter => 0,
            Self::File => 1,
        }
    }

    /// What set the row for this session, if anything did.
    const fn set_by(self, state: &LoggingState) -> Option<&'static str> {
        match self {
            Self::Filter => state.filter_set_by,
            Self::File => state.file_set_by,
        }
    }

    /// What the row holds as typed: the editor's prefill (the file row empty with none).
    fn typed(self, state: &LoggingState) -> String {
        match self {
            Self::Filter => state.filter.clone(),
            Self::File => state
                .file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        }
    }

    /// What the row shows.
    fn shown(self, state: &LoggingState) -> String {
        match self {
            Self::File if state.file.is_none() => NO_FILE.to_owned(),
            _ => self.typed(state),
        }
    }
}

/// The open editor: its row and its one field.
struct Editor {
    /// The row it edits.
    row: Row,
    /// The buffer; never printed by `Debug`.
    input: TextField,
}

/// The row, never the buffer (`QueueSection`'s habit).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor").field("row", &self.row).finish()
    }
}

/// What the section is doing.
#[derive(Debug, Default)]
enum Mode {
    /// The rows and the cursor.
    #[default]
    Browse,
    /// One row being typed into.
    Editing(Editor),
}

/// The last outcome, and whether the user has to act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// One line of report.
    Info(String),
    /// One line drawn in `theme.error`.
    Error(String),
}

/// `Settings > Logging` (MOD-122 D5): the filter and the log file.
#[derive(Debug)]
pub struct LoggingSection {
    /// The handle `App` applies through: the rows read it, nothing is copied.
    control: LoggingControl,
    /// The highlighted row, an index into [`Row::ALL`].
    cursor: usize,
    /// Browsing, or typing into one row.
    mode: Mode,
    /// The last outcome, on the notice row under the form.
    notice: Option<Notice>,
    /// `(total rows, offset)` as of the last render, for the Settings pane's scrollbar.
    scroll: Cell<Option<ScrollSpan>>,
}

impl LoggingSection {
    /// Stable identity.
    pub const ID: SectionId = SectionId("logging");

    /// A section over `control`, the one `App` holds (D7).
    #[must_use]
    pub fn new(control: LoggingControl) -> Self {
        Self {
            control,
            cursor: 0,
            mode: Mode::Browse,
            notice: None,
            scroll: Cell::new(None),
        }
    }

    /// The row under the cursor.
    fn selected(&self) -> Row {
        Row::ALL[self.cursor.min(Row::ALL.len() - 1)]
    }

    /// Moves the cursor one row, stopping at either end.
    fn move_cursor(&mut self, down: bool) {
        self.cursor = if down {
            (self.cursor + 1).min(Row::ALL.len() - 1)
        } else {
            self.cursor.saturating_sub(1)
        };
    }

    /// `e`: the row under the cursor, prefilled with what is in force, unless a flag or a
    /// variable set it for this session (B5).
    fn open_edit(&mut self) {
        let row = self.selected();
        let state = self.control.state();
        if let Some(by) = row.set_by(&state) {
            self.notice = Some(Notice::Info(overridden(by, row)));
            return;
        }
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            row,
            input: TextField::with_text(&row.typed(&state)),
        });
    }

    /// One key while an editor is open: the field answers first. What it passes on is swallowed,
    /// except the chords [`views::CAPTURE`] passes, so `ctrl-c` still quits and `F1` opens help.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => editor.input.on_key(key),
            Mode::Browse => return Handled::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
        }
    }

    /// `Enter` in the editor, the text trimmed. The filter: empty is [`DEFAULT_FILTER`], and a
    /// directive [`logging::parse_filter`] refuses keeps the editor open and sends nothing. The
    /// file: empty is no file, and a relative path is taken from the directory htui runs in
    /// ([`absolute`]); opening it is the control's, through `App`.
    fn submit(&mut self, ctx: &Ctx<'_>) {
        let Mode::Editing(editor) = &self.mode else {
            return;
        };
        let text = editor.input.text().unwrap_or_default().trim().to_owned();
        let change = match editor.row {
            Row::Filter => {
                let directive = if text.is_empty() {
                    DEFAULT_FILTER.to_owned()
                } else {
                    text
                };
                if let Err(err) = logging::parse_filter(&directive) {
                    self.notice = Some(Notice::Error(err.to_string()));
                    return;
                }
                LoggingAction::Filter(directive)
            }
            Row::File => LoggingAction::File((!text.is_empty()).then(|| absolute(text))),
        };
        self.mode = Mode::Browse;
        self.notice = None;
        ctx.emit(Action::Logging(change));
    }

    /// One row as a form field: the value and, when overridden, what set it; the help (or why it
    /// cannot be edited) under the focused row; a file that could not be opened under its row
    /// whatever has focus. The open editor's row is its field, with what the row holds now.
    fn field<'a>(&'a self, row: Row, state: &LoggingState) -> Field<'a> {
        let shown = row.shown(state);
        let field = match &self.mode {
            Mode::Editing(editor) if editor.row == row => {
                Field::new(row.label(), form::Value::Text(&editor.input))
                    .help(format!("now {shown} \u{b7} empty resets"))
            }
            _ => match row.set_by(state) {
                Some(by) => Field::new(
                    row.label(),
                    form::Value::ReadOnly(format!("{shown} \u{b7} set by {by}").into()),
                )
                .help(overridden(by, row)),
                None => {
                    Field::new(row.label(), form::Value::ReadOnly(shown.into())).help(row.help())
                }
            },
        };
        match (row, &state.file_error) {
            (Row::File, Some(error)) => field.error(error.clone()),
            _ => field,
        }
    }

    /// The one line under the rows: the last outcome as a kit notice, blank without one.
    fn notice_line(&self, theme: &Theme) -> Line<'static> {
        match &self.notice {
            None => Line::default(),
            Some(Notice::Info(text)) => notice(text, Tone::Info, theme),
            Some(Notice::Error(text)) => notice(text, Tone::Error, theme),
        }
    }

    /// The stack of the current mode: the one place a mode maps to its keys.
    const fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::LOGGING_BROWSE,
            Mode::Editing(_) => views::CAPTURE,
        }
    }
}

impl SettingsSection for LoggingSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Logging"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // The settings are the process's, held by the control: nothing to read.
        Vec::new()
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The log is the process's, not a workspace's: nothing to drop.
    }

    fn captures_input(&self) -> bool {
        matches!(self.mode, Mode::Editing(_))
    }

    fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
        let Mode::Editing(editor) = &mut self.mode else {
            return Handled::Pass;
        };
        editor.input.on_paste(text);
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if matches!(self.mode, Mode::Editing(_)) {
            return self.on_editor_key(key, ctx);
        }
        // Browse: the first candidate this state accepts; a global act or a declined one is the
        // shell's.
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::LOGGING_BROWSE, chord) {
            match act {
                Act::Edit => self.open_edit(),
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                _ => continue,
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        // No read and no write goes through the store.
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        let spec = match self.mode {
            Mode::Browse => HINT_BROWSE,
            Mode::Editing(_) => HINT_EDITING,
        };
        key_line(keys.hint(self.stack(), spec), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let state = self.control.state();
        let [rows, hint] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let (focus, editing) = match &self.mode {
            Mode::Editing(editor) => (editor.row.index(), true),
            Mode::Browse => (self.cursor, false),
        };
        let form = Row::ALL.into_iter().fold(
            Form::new()
                .label_width(LABEL_WIDTH)
                .focus(focus)
                .editing(editing),
            |form, row| form.field(self.field(row, &state)),
        );
        let laid = form.layout(rows.width, ctx.accent(), theme);
        let offset = laid.offset(rows.height);
        laid.render(frame, rows, offset);
        self.scroll
            .set(Some(ScrollSpan::new(laid.lines.len(), offset).rows(rows)));
        frame.render_widget(Paragraph::new(self.notice_line(theme)), hint);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::LoggingSection;
    use crate::app::{Action, Handled, LoggingAction};
    use crate::logging::{Logging, Startup};
    use crate::testkit::{Harness, SectionBench};
    use crate::ui::tabs::settings::SettingsSection;

    /// The width every frame here is drawn at: the widest temporary path still fits one line.
    const WIDTH: u16 = 120;

    /// A section over a control with no subscriber at all (D7).
    fn detached() -> (Logging, LoggingSection) {
        let logging = Logging::detached();
        let section = LoggingSection::new(logging.control().clone());
        (logging, section)
    }

    /// A section over a control `HTUI_LOG_FILTER=debug` and `--log <dir>/htui.log` started:
    /// built, never installed.
    fn overridden(dir: &Path) -> (Logging, LoggingSection) {
        let logging = Logging::build(Startup {
            file: Some(dir.join("htui.log")),
            filter: Some("debug".to_owned()),
        })
        .expect("the startup file opens");
        let section = LoggingSection::new(logging.control().clone());
        (logging, section)
    }

    /// `n` backspaces, then `text` pasted into the open editor.
    fn retype(bench: &SectionBench, section: &mut LoggingSection, n: usize, text: &str) {
        for _ in 0..n {
            assert_eq!(bench.key(section, "backspace"), Handled::Consumed);
        }
        if !text.is_empty() {
            assert_eq!(bench.paste(section, text), Handled::Consumed);
        }
    }

    /// The logging actions the section emitted since the last call.
    fn emitted(bench: &SectionBench) -> Vec<LoggingAction> {
        bench
            .drained()
            .into_iter()
            .map(|action| match action {
                Action::Logging(change) => change,
                other => panic!("only logging actions are emitted: {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn both_fields_render_with_their_values_and_the_focused_help() {
        let bench = SectionBench::new().await;
        let (_logging, section) = detached();

        let frame = bench.render_section(&section, WIDTH);

        assert!(frame.contains("filter"), "{frame}");
        assert!(frame.contains("info"), "{frame}");
        assert!(frame.contains("log file"), "{frame}");
        assert!(frame.contains("none"), "{frame}");
        assert!(
            frame.contains("EnvFilter directive"),
            "the focused help: {frame}"
        );
        assert!(
            !frame.contains("appends its log"),
            "only the focused help: {frame}"
        );
        assert_eq!(bench.key_hint(&section), "j/k \u{b7} e edit");
        insta::assert_snapshot!("logging_section", frame);
    }

    #[tokio::test]
    async fn an_overridden_field_says_what_set_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let bench = SectionBench::new().await;
        let (_logging, mut section) = overridden(dir.path());
        assert_eq!(bench.key(&mut section, "j"), Handled::Consumed);

        // The whole path, so the separator `join` used is masked too on every platform.
        let frame = bench.render_section(&section, WIDTH).replace(
            &dir.path().join("htui.log").display().to_string(),
            "[TMP]/htui.log",
        );

        assert!(
            frame.contains("debug \u{b7} set by `HTUI_LOG_FILTER`"),
            "{frame}"
        );
        assert!(
            frame.contains("[TMP]/htui.log \u{b7} set by `--log`"),
            "{frame}"
        );
        assert!(
            frame.contains(
                "set by `--log` for this session: start htui without it to change the log file here"
            ),
            "the focused row says why: {frame}"
        );
        insta::assert_snapshot!("logging_section_overridden", frame);
    }

    /// A relative path under a directory that does not exist: nothing is created, and the
    /// refusal reads the same on every Unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_that_could_not_open_is_said_under_its_row() {
        let bench = SectionBench::new().await;
        let (logging, section) = detached();
        let refused = Path::new("no-such-dir-mod122/htui.log");
        logging
            .control()
            .set_file(Some(refused))
            .expect_err("the directory does not exist");

        let frame = bench.render_section(&section, WIDTH);

        assert!(
            frame.contains("cannot open the log file no-such-dir-mod122/htui.log"),
            "under the unfocused row too: {frame}"
        );
        insta::assert_snapshot!("logging_section_file_error", frame);
    }

    #[tokio::test]
    async fn editing_the_filter_to_a_valid_directive_emits_it() {
        let bench = SectionBench::new().await;
        let (_logging, mut section) = detached();

        assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
        assert!(section.captures_input(), "the editor is open");
        assert_eq!(
            bench.key_hint(&section),
            "Enter save \u{b7} Esc cancel \u{b7} empty resets"
        );
        retype(&bench, &mut section, "info".len(), "debug");
        assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);

        assert_eq!(emitted(&bench), [LoggingAction::Filter("debug".to_owned())]);
        assert!(!section.captures_input(), "back to browse");
    }

    #[tokio::test]
    async fn an_invalid_directive_is_refused_and_emits_nothing() {
        let bench = SectionBench::new().await;
        let (_logging, mut section) = detached();

        bench.key(&mut section, "e");
        retype(&bench, &mut section, "info".len(), "htui=[");
        bench.key(&mut section, "enter");

        assert!(emitted(&bench).is_empty());
        assert!(
            section.captures_input(),
            "the editor stays open over its text"
        );
        let frame = bench.render_section(&section, WIDTH);
        assert!(
            frame.contains("`htui=[` is not a log filter: "),
            "the refusal says why: {frame}"
        );
    }

    #[tokio::test]
    async fn an_empty_filter_emits_the_default() {
        let bench = SectionBench::new().await;
        let (_logging, mut section) = detached();

        bench.key(&mut section, "e");
        retype(&bench, &mut section, "info".len(), "");
        bench.key(&mut section, "enter");

        assert_eq!(emitted(&bench), [LoggingAction::Filter("info".to_owned())]);
    }

    #[tokio::test]
    async fn a_file_path_is_emitted_as_typed_and_clearing_it_emits_no_file() {
        let bench = SectionBench::new().await;
        let (_logging, mut section) = detached();

        bench.key(&mut section, "j");
        bench.key(&mut section, "e");
        retype(&bench, &mut section, 0, "  /var/log/htui.log  ");
        bench.key(&mut section, "enter");
        assert_eq!(
            emitted(&bench),
            [LoggingAction::File(Some(PathBuf::from(
                "/var/log/htui.log"
            )))]
        );

        bench.key(&mut section, "e");
        retype(&bench, &mut section, 0, "   ");
        bench.key(&mut section, "enter");
        assert_eq!(emitted(&bench), [LoggingAction::File(None)]);
    }

    /// A relative path is taken from the directory htui runs in and emitted absolute, so the
    /// saved file does not move with the directory the next session starts from.
    #[tokio::test]
    async fn a_relative_file_path_is_emitted_absolute() {
        let bench = SectionBench::new().await;
        let (_logging, mut section) = detached();
        let here = std::env::current_dir().expect("the current directory");

        bench.key(&mut section, "j");
        bench.key(&mut section, "e");
        retype(&bench, &mut section, 0, "logs/htui.log");
        bench.key(&mut section, "enter");

        assert_eq!(
            emitted(&bench),
            [LoggingAction::File(Some(here.join("logs/htui.log")))]
        );
    }

    /// B5: a field `--log` or `HTUI_LOG_FILTER` set for the session refuses its edit and says
    /// why, rather than saving a value that would not apply.
    #[tokio::test]
    async fn an_overridden_field_refuses_the_edit_and_says_why() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let bench = SectionBench::new().await;
        let (_logging, mut section) = overridden(dir.path());

        assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
        assert!(!section.captures_input(), "no editor over the filter");
        // The focused row's help says it, and so does the notice under the rows.
        let frame = bench.render_section(&section, WIDTH);
        let why_filter = "set by `HTUI_LOG_FILTER` for this session: start htui without it to change the log filter here";
        assert_eq!(
            frame.matches(why_filter).count(),
            2,
            "help and notice: {frame}"
        );
        assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
        assert_eq!(
            bench.key(&mut section, "esc"),
            Handled::Pass,
            "the notice is gone, so the next esc is the shell's"
        );

        bench.key(&mut section, "j");
        assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
        assert!(!section.captures_input(), "no editor over the file");
        let frame = bench.render_section(&section, WIDTH);
        let why_file =
            "set by `--log` for this session: start htui without it to change the log file here";
        assert_eq!(
            frame.matches(why_file).count(),
            2,
            "help and notice: {frame}"
        );
        assert!(emitted(&bench).is_empty());
        assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
    }

    /// The rows read the control `App` applies through, so an applied value shows on the next
    /// frame with no copy to refresh.
    #[tokio::test]
    async fn the_section_shows_the_applied_value_on_the_next_frame() {
        let bench = SectionBench::new().await;
        let (logging, section) = detached();

        logging
            .control()
            .set_filter("warn")
            .expect("a valid directive");

        let frame = bench.render_section(&section, WIDTH);
        let row = frame
            .lines()
            .find(|line| line.contains("filter "))
            .unwrap_or_default();
        assert!(row.ends_with(" warn"), "{frame}");
    }

    /// MOD-122 D5: the product registers `Logging` last, so eleven `l` reach it from Agents.
    #[tokio::test]
    async fn the_product_registers_logging_last() {
        let mut harness = Harness::demo();
        crate::app::register_all(harness.app());
        harness.settle().await;
        // Settings is the fourth tab.
        harness.key("4");
        harness.settle().await;
        for _ in 0..11 {
            harness.key("l");
        }
        harness.settle().await;

        let frame = harness.render();
        assert!(
            frame.contains(" Project types  Logging "),
            "Logging follows Project types: {frame}"
        );
        assert!(
            frame.contains("j/k \u{b7} e edit"),
            "the active section is Logging: {frame}"
        );
        assert!(frame.contains("EnvFilter directive"), "{frame}");
    }
}
