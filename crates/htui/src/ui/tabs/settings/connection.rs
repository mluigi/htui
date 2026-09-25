//! The connection section of the Settings tab: the backend, the stored DSN, the mirror, and the
//! four keys that change them (MOD-15 milestone 6, D8/D14/D15/D16/D17/D19; `R-TUI-8`, `R-STO-1`,
//! `R-SEC-2`).
//!
//! It holds **no store handle** (`R-NF-3`): it names one read
//! ([`StoreRequest::ConnectionInfo`]), is handed the [`ConnectionSnapshot`] that comes back, and
//! every write leaves through `ctx.request` for the store worker's own loop to carry out. What is
//! on screen is always the last snapshot the worker assembled — no row is ever patched in locally,
//! so there is exactly one source of truth.
//!
//! **Nothing recoverable is ever drawn.** The DSN field is
//! [`TextField::masked`](crate::ui::TextField::masked): one `\u{2022}` per character and a count,
//! with no reveal toggle anywhere in this file, and the buffer is zeroizing. No `Debug` in this
//! module prints a buffer or a notice's text, because [`crate::store_worker::RequestEnvelope`] and
//! every section derive `Debug` and one `tracing::debug!` is all it takes to put a line in the
//! `--log` file. The DSN row shows [`Dsn::summary`], which carries no password because
//! `PgConnectOptions` — the type the summary is built from — has no getter for one.
//!
//! **A refused DSN emits nothing at all** (D2). [`Dsn::parse`] runs here, on the UI task, and its
//! refusal is one of five fixed sentences coined in `htui-store`. Sending the string to the worker
//! to be judged would put an unrecognised query parameter's *value* into sqlx's own `warn!` and so
//! into the log file; parsing it here is what keeps that from happening at all.
//!
//! The section is **read-only about connection state** and derives none of it (D15): the label is
//! [`Backend::label`](htui_store::Backend::label)'s string, carried in the snapshot, and this file
//! never parses it or infers anything from `offline \u{b7} <age>`.

use htui_core::model::Scope;
use htui_store::Dsn;
use htui_store::cache::MIRRORED_TABLES;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use zeroize::Zeroizing;

use crate::app::{Ctx, Handled};
use crate::connection::{AttemptOutcome, ConnectionSnapshot, DsnState, READ_NAME, REQUEST_NAMES};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{SectionId, SettingsSection, wrapped};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What a row's value column says before the first reply.
const NOT_READ: &str = "not read yet";

/// What the pane says when the read itself was refused; the seam's sentence follows it.
const UNAVAILABLE: &str = "connection info is unavailable";

/// Browse's keys, with something to browse.
const HINT_BROWSE: &str =
    "e edit DSN \u{b7} c clear DSN \u{b7} R rebuild cache \u{b7} r reload \u{b7} j/k rows";

/// Browse's keys with nothing read, or the read refused: the only offer is to ask again.
///
/// [`PromptSection`](super::PromptSection)'s rule, one section across: offering `e` over rows
/// nobody can see would be offering a key that is about to refuse.
const HINT_NO_SNAPSHOT: &str = "r reload";

/// The open field's keys, and the promise the mask is.
const HINT_EDITING: &str = "Enter store \u{b7} Esc cancel \u{b7} typed text is never shown";

/// Either confirmation's keys.
const HINT_CONFIRM: &str = "y confirm \u{b7} n / Esc cancel";

/// What a stored DSN's row leads with, and what a plain `SetDsn` reports.
const STORED: &str = "stored";

/// D12: `--offline` is a session choice the user typed, so it is honoured and said out loud
/// rather than silently overridden.
const STORED_OFFLINE: &str =
    "stored; this session was started with --offline, so it takes effect on the next launch";

/// D13: what `ClearDsn` did, and — the half that matters — what it deliberately did not do.
const CLEARED: &str = "the DSN is gone from the keyring \u{2014} this session keeps its current connection until you quit";

/// D14's report. The mirror is empty, and the thing that fills it is already running.
const REBUILT: &str = "mirror rebuilt; the next refresh pass refills it";

/// An `Enter` over an empty field is not a write: there is nothing to store and clearing is `c`.
const EMPTY_FIELD: &str = "nothing typed; the stored DSN is unchanged";

/// What the open field says when the box has no DSN at all (D8).
const NO_DSN_YET: &str = "no DSN is stored; type one and press Enter";

/// What the open field says when there **is** one.
///
/// A different sentence from [`NO_DSN_YET`] because it is a different situation, and saying "no
/// DSN is stored" over a box that has one would be the screen lying about the state it is there
/// to report. `e` replaces; it has never had a way to reveal.
const REPLACES: &str = "Enter replaces the stored DSN";

/// What a row says on [`Backend::Memory`](htui_store::Backend::Memory) (D10): not "none", which
/// would be a claim about a keyring this session never opened.
const DEMO_ROW: &str = "n/a in a demo session";

/// The DSN row with nothing behind it.
const NOT_STORED: &str = "not stored";

/// The DSN row over a keyring that could not be opened; the seam's sentence follows it.
///
/// A different row from [`NOT_STORED`] because it is a different fact, and the one thing this
/// section must never do is report an unreadable keyring as an empty one: a user whose collection
/// is locked still has their DSN, and telling them otherwise invites a retype into a store that
/// cannot hold it.
const UNREADABLE: &str = "the keyring could not be read";

/// The guide line under an open field when the keyring could not be read.
///
/// Neither [`NO_DSN_YET`] nor [`REPLACES`] is true here — nobody knows whether there is a DSN —
/// so the field says what it is about to attempt instead of claiming a state.
const UNREADABLE_GUIDE: &str = "the keyring could not be read; Enter tries to store a DSN in it";

/// B-7: one `--set-dsn` may have written a string this build's parser refuses. It is still the
/// DSN the next launch will dial with, so it is reported as stored — without a summary that would
/// have to be guessed at.
const STORED_UNREADABLE: &str = "stored \u{2014} not readable by this build; e replaces it";

/// The Status row before the session's first dial.
const NO_ATTEMPT: &str = "no dial yet this session";

/// The Mirror row's `last full refresh` before the first cursor-at-zero pass.
const NEVER: &str = "never";

/// The Rebuild row's value: a row that is an action says which keys run it.
const REBUILD_KEYS: &str = "press Enter or R";

/// D13's question. Names what the session keeps, because losing a working connection is what a
/// user would reasonably fear from a key called "clear".
const CONFIRM_CLEAR: &str = "Remove the DSN from the keyring? This session keeps its current connection; the next launch starts offline. y / n";

/// D14's question, written from `CacheStore::rebuild`'s own body; the table count is
/// [`MIRRORED_TABLES`]'s, so a table joining the mirror cannot leave the copy behind.
///
/// Both lists, always. The action is never extended to clear anything else — if it were, one of
/// these two halves would quietly stop being true, and a confirmation that is wrong about what it
/// destroys is worse than no confirmation at all.
fn confirm_rebuild() -> String {
    format!(
        "Rebuild the mirror? Survives: the file, schema_version, db_fingerprint, built_at, the pending/ buffer. Goes: the {} mirrored tables, cache_cursor, last_full_refresh_at. The next refresh pass refills it. y / n",
        MIRRORED_TABLES.len()
    )
}

/// What the pane says while a rebuild is out. A rebuild can take seconds; a screen that still
/// showed the question would be inviting a second `y` at nothing.
const REBUILDING: &str = "rebuilding\u{2026}";

/// The four rows, top to bottom (D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The backend's own label and the last dial.
    Status,
    /// Whether a DSN is stored, and its redacted summary.
    Dsn,
    /// The mirror this session is reading.
    Mirror,
    /// An action row, not a fact: `Enter` and `R` both run it.
    Rebuild,
}

impl Row {
    /// Every row, in screen order. The cursor indexes this.
    const ALL: [Self; 4] = [Self::Status, Self::Dsn, Self::Mirror, Self::Rebuild];

    /// The label column.
    fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Dsn => "DSN",
            Self::Mirror => "Mirror",
            Self::Rebuild => "Rebuild cache",
        }
    }
}

/// The DSN field.
///
/// One field, so there is no target and no `Tab`. The buffer is masked and zeroizing (D3), and
/// this printer shows only how much of it there is.
struct Editor {
    /// What is being typed. Read exactly once, by [`TextField::take`].
    input: TextField,
}

/// The length and nothing else (the hard rule; `TextField`'s own `Debug` holds the same line).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("len", &self.input.len())
            .finish()
    }
}

/// The two stages of a rebuild (D14), mirroring the kinds section's delete.
///
/// `ConfirmClear` has no such pair on purpose: `y` there sends and returns to Browse with the
/// name in `busy`, exactly as the other sections' writes do. A rebuild keeps the two stages
/// because it can take seconds, and a second `y` at a rebuild already out must be inert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmStage {
    /// The question is on screen and `y` answers it.
    Asking,
    /// `rebuild_cache` is out; there is nothing left to answer.
    InFlight,
}

/// Where the section is. `Browse` is not a mode in the modal sense: it captures nothing (D17).
#[derive(Default)]
enum Mode {
    /// The rows, the cursor and the tab's own `h`/`l`/`[`/`]`.
    #[default]
    Browse,
    /// The masked field, taking every printable key.
    Editing(Editor),
    /// D13's question.
    ConfirmClear,
    /// D14's question, and what follows a `y`.
    ConfirmRebuild {
        /// Asked, or already out.
        stage: ConfirmStage,
    },
}

/// Everything the mode is *about*, never what was typed into it.
impl core::fmt::Debug for Mode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Browse => f.write_str("Browse"),
            Self::Editing(editor) => f.debug_tuple("Editing").field(editor).finish(),
            Self::ConfirmClear => f.write_str("ConfirmClear"),
            Self::ConfirmRebuild { stage } => f
                .debug_struct("ConfirmRebuild")
                .field("stage", stage)
                .finish(),
        }
    }
}

/// The last outcome, and whether it is one the user has to act on.
#[derive(Clone, PartialEq, Eq)]
enum Notice {
    /// One line of report.
    Info(String),
    /// One line the user has to act on, drawn in `theme.error`.
    Error(String),
}

/// The kind and the length, never the sentence (B-8).
///
/// The sentences this section puts in a notice are its own constants, the seam's refusals and
/// [`htui_store::DsnError`]'s five — none of which carries anything typed. The rule is the point
/// anyway: **no section's `Debug` prints text that came out of a field**, and a printer that had
/// to be re-audited every time a new sentence was added would be a rule nobody could keep.
impl core::fmt::Debug for Notice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (kind, text) = match self {
            Self::Info(text) => ("Info", text),
            Self::Error(text) => ("Error", text),
        };
        f.debug_struct(kind)
            .field("len", &text.chars().count())
            .finish()
    }
}

impl Notice {
    /// The sentence.
    fn text(&self) -> &str {
        match self {
            Self::Info(text) | Self::Error(text) => text,
        }
    }

    /// Whether it belongs in `theme.error`.
    fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// `Settings > Connection`: the box's own connection, and the four keys that change it.
#[derive(Debug, Default)]
pub struct ConnectionSection {
    /// The last snapshot the worker assembled, or `None` before the first reply.
    snapshot: Option<ConnectionSnapshot>,
    /// `Some(message)` after `Failed { request: "connection_info" }`: the read itself was refused.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`Row::ALL`].
    selected: usize,
    /// Browsing, typing, or being asked a question.
    mode: Mode,
    /// The last outcome, one line on the hint row or under the field.
    notice: Option<Notice>,
    /// The write in flight, by [`StoreRequest::name`]. A second one is refused until the reply:
    /// the staleness index keeps only the newest request of a kind, so two writes of one kind
    /// racing would lose the reply about the one that landed (D5).
    busy: Option<&'static str>,
    /// D8: the editor opened itself for an empty keyring. Once per session, so the fourth-tick
    /// re-read cannot reopen a field the user closed; reset on a scope change, which is the one
    /// point the session's screen starts again.
    opened_for_empty: bool,
}

impl ConnectionSection {
    /// Identity of the connection section.
    pub const ID: SectionId = SectionId("connection");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the last read said about the keyring, or `None` before the first one.
    fn dsn_state(&self) -> Option<&DsnState> {
        self.snapshot.as_ref().map(|snapshot| &snapshot.dsn_state)
    }

    /// Whether there is a mirror to act on.
    fn has_mirror(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.mirror.is_some())
    }

    /// Whether a key that opens an editor or a question is refused right now, with the notice
    /// that says why.
    ///
    /// One write at a time (D5). `r` is deliberately not on this path — re-reading is how a
    /// section that lost a reply recovers, and a read cannot lose a write's reply.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        self.snapshot.is_none() || self.unavailable.is_some()
    }

    /// Moves the cursor one row and stops at the end it reaches.
    ///
    /// No wrap, for the other sections' reason: a key acts on the row the cursor is on, and a
    /// held `j` that wrapped to the top would aim `Enter` at a row nobody looked at.
    fn move_cursor(&mut self, down: bool) {
        let last = Row::ALL.len() - 1;
        self.selected = if down {
            self.selected.saturating_add(1).min(last)
        } else {
            self.selected.saturating_sub(1)
        };
    }

    /// The row under the cursor.
    fn row(&self) -> Row {
        Row::ALL[self.selected.min(Row::ALL.len() - 1)]
    }

    /// `e`: a fresh masked field, whatever is stored.
    ///
    /// Empty rather than prefilled, and there is no other shape available: the stored DSN's text
    /// never leaves `htui-store`, so there is nothing to prefill it with. That is the design, not
    /// a limitation — `e` **replaces**, it does not reveal.
    fn open_edit(&mut self) {
        self.notice = None;
        self.mode = Mode::Editing(Editor {
            input: TextField::masked(),
        });
    }

    /// What `c` says when there is nothing to clear: the DSN row's own words, so the refusal and
    /// the row cannot disagree about why.
    fn dsn_row_refusal(&self) -> String {
        self.dsn_state().map_or_else(
            || NOT_READ.to_owned(),
            |state| {
                dsn_row(
                    state,
                    self.snapshot.as_ref().and_then(|s| s.dsn_summary.as_ref()),
                )
            },
        )
    }

    /// `R`, and `Enter` on the Rebuild row (B-9): one path, so the two keys cannot drift.
    fn rebuild(&mut self) {
        if self.blocked() {
            return;
        }
        if self.has_mirror() {
            self.notice = None;
            self.mode = Mode::ConfirmRebuild {
                stage: ConfirmStage::Asking,
            };
        } else {
            self.refuse(DEMO_ROW.to_owned());
        }
    }

    /// Sends one request and remembers its name until the reply.
    fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(request.name());
        self.notice = None;
        ctx.request(request);
    }

    /// One key while the field is open.
    ///
    /// The field answers first, so `l`, `h`, `[`, `]`, `q` and the digits are characters here;
    /// everything it passes on is swallowed rather than offered to the shell — with `CONTROL`
    /// chords excepted, so `ctrl-c` still quits.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::Editing(editor) => editor.input.on_key(key),
            _ => return Handled::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => Handled::Consumed,
            FieldOutcome::Submit => {
                self.submit(ctx);
                Handled::Consumed
            }
            // The field drops with the mode, and its buffer is wiped on the way out (D3).
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Pass if key.modifiers.contains(KeyModifiers::CONTROL) => Handled::Pass,
            FieldOutcome::Pass => Handled::Consumed,
        }
    }

    /// `Enter` in the field: one parse, and either a request or a sentence (D2).
    ///
    /// The buffer is **moved** out of the field into a zeroizing wrapper, so there is one copy of
    /// what was typed and it is wiped when this function returns. [`Dsn::parse`] takes a
    /// zeroizing copy of its own; nothing else ever holds the text.
    ///
    /// A refusal emits **nothing**: no request, no `Action::Error`, no log line. The field is left
    /// open and empty over the sentence, which is the state a retype starts from.
    fn submit(&mut self, ctx: &mut Ctx<'_>) {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return;
        }
        let Mode::Editing(editor) = &mut self.mode else {
            return;
        };
        if editor.input.is_empty() {
            self.mode = Mode::Browse;
            self.say(EMPTY_FIELD);
            return;
        }

        let raw = Zeroizing::new(editor.input.take());
        match Dsn::parse(&raw) {
            Ok(dsn) => {
                self.mode = Mode::Browse;
                self.send(StoreRequest::SetDsn(dsn), ctx);
            }
            Err(err) => {
                // `take` already emptied the field; give it back its reservation so the retype
                // does not grow a fresh buffer one realloc at a time (D3).
                *editor = Editor {
                    input: TextField::masked(),
                };
                self.refuse(err.to_string());
            }
        }
    }

    /// One key while either question is on screen.
    ///
    /// Modal over the shell as well as over the rows, as the kinds section's delete is: an
    /// unlisted key is swallowed so a `q` at the question does not quit the application, with
    /// `CONTROL` chords excepted so `ctrl-c` still does.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Handled::Pass;
        }
        match &mut self.mode {
            Mode::ConfirmClear => match key.code {
                KeyCode::Char('y') => {
                    self.mode = Mode::Browse;
                    self.send(StoreRequest::ClearDsn, ctx);
                }
                KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::ConfirmRebuild { stage } => match (*stage, key.code) {
                (ConfirmStage::Asking, KeyCode::Char('y')) => {
                    *stage = ConfirmStage::InFlight;
                    self.send(StoreRequest::RebuildCache, ctx);
                }
                (ConfirmStage::Asking, KeyCode::Char('n') | KeyCode::Esc) => {
                    self.mode = Mode::Browse;
                }
                // Nothing left to answer: the rebuild is already out.
                _ => {}
            },
            Mode::Browse | Mode::Editing(_) => return Handled::Pass,
        }
        Handled::Consumed
    }

    /// The value column of one row (D16), before wrapping.
    fn row_text(&self, row: Row) -> String {
        let Some(snapshot) = &self.snapshot else {
            return NOT_READ.to_owned();
        };
        match row {
            // D15: the label is the backend's own string, rendered and never parsed.
            Row::Status => format!("{} \u{b7} {}", snapshot.label, attempt_text(snapshot)),
            Row::Dsn => dsn_row(&snapshot.dsn_state, snapshot.dsn_summary.as_ref()),
            Row::Mirror => match &snapshot.mirror {
                None => DEMO_ROW.to_owned(),
                Some(mirror) => {
                    let fingerprint = mirror
                        .db_fingerprint
                        .get(..12)
                        .unwrap_or(&mirror.db_fingerprint);
                    let refreshed = mirror.last_full_refresh_at.map_or_else(
                        || NEVER.to_owned(),
                        |at| at.format("%Y-%m-%d %H:%M").to_string(),
                    );
                    format!(
                        "{fingerprint} \u{b7} built {} \u{b7} last full refresh {refreshed} \u{b7} schema {}",
                        mirror.built_at.format("%Y-%m-%d %H:%M"),
                        mirror.schema_version,
                    )
                }
            },
            Row::Rebuild => REBUILD_KEYS.to_owned(),
        }
    }

    /// One or more lines per row, in [`Row::ALL`] order; a value too long for the frame wraps
    /// under its own column rather than being clipped.
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let label_width = label_width();
        let gutter = label_width + 4;
        let room = usize::from(width).saturating_sub(gutter).max(1);
        let mut lines = Vec::new();
        for (index, row) in Row::ALL.into_iter().enumerate() {
            let style = if index == self.selected {
                theme.selected
            } else {
                theme.base
            };
            let value = self.row_text(row);
            for (n, chunk) in wrapped(&value, room).into_iter().enumerate() {
                let label = if n == 0 { row.label() } else { "" };
                lines.push(Line::styled(
                    format!("  {label:<label_width$}  {chunk}"),
                    style,
                ));
            }
        }
        lines
    }

    /// The pane under the rows: the field, or the question, or nothing.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let room = usize::from(width).max(1);
        match &self.mode {
            Mode::Browse => Vec::new(),
            Mode::Editing(editor) => {
                let label = "DSN: ";
                let field_room = usize::from(width).saturating_sub(label.chars().count());
                let mut spans = vec![Span::styled(label, theme.accent)];
                spans.extend(
                    editor
                        .input
                        .line(u16::try_from(field_room).unwrap_or(u16::MAX), true, theme)
                        .spans,
                );
                let mut lines = vec![Line::from(spans)];
                // Under the field rather than on the hint line: a refusal is about what was just
                // typed, and it belongs where the typing is.
                let guide = match self.dsn_state() {
                    Some(DsnState::Stored) => REPLACES,
                    Some(DsnState::Unreadable(_)) => UNREADABLE_GUIDE,
                    _ => NO_DSN_YET,
                };
                let (text, style) = match &self.notice {
                    Some(notice) if notice.is_error() => (notice.text(), theme.error),
                    Some(notice) => (notice.text(), theme.dim),
                    None => (guide, theme.dim),
                };
                lines.extend(
                    wrapped(text, room)
                        .into_iter()
                        .map(|line| Line::styled(line, style)),
                );
                lines
            }
            Mode::ConfirmClear => question(CONFIRM_CLEAR, room, theme),
            Mode::ConfirmRebuild { stage } => {
                let mut lines = question(&confirm_rebuild(), room, theme);
                if matches!(stage, ConfirmStage::InFlight) {
                    lines.push(Line::styled(REBUILDING.to_owned(), theme.dim));
                }
                lines
            }
        }
    }

    /// The one line under the pane: the keys this mode binds, then the last outcome.
    ///
    /// The outcome only appears here in Browse; every other mode already draws it in the pane,
    /// where it is next to what it is about.
    fn hint(&self, width: u16, theme: &Theme) -> Line<'static> {
        let keys = self.hint_text();
        let (Some(notice), Mode::Browse) = (&self.notice, &self.mode) else {
            return Line::styled(keys, theme.dim);
        };
        let style = if notice.is_error() {
            theme.error
        } else {
            theme.dim
        };
        let text = notice.text().to_owned();
        // The outcome wins the line when both do not fit: the keys are on screen every other
        // frame, and this is the only place the outcome appears.
        let room = usize::from(width);
        if keys.chars().count() + text.chars().count() + 3 > room {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The keys half of the hint line, plus what a write in flight adds to it.
    fn hint_text(&self) -> String {
        let keys = match self.mode {
            Mode::Editing(_) => HINT_EDITING,
            Mode::ConfirmClear | Mode::ConfirmRebuild { .. } => HINT_CONFIRM,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else {
                    HINT_BROWSE
                }
            }
        };
        match self.busy {
            // Only in Browse: the other modes' hints say what their own key is for, and a write in
            // flight is why that key is not answering.
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {busy} in flight")
            }
            _ => keys.to_owned(),
        }
    }

    /// Reports an outcome, classified by the rule the sections share.
    fn say(&mut self, text: &str) {
        self.notice = Some(if super::is_error(text) {
            Notice::Error(text.to_owned())
        } else {
            Notice::Info(text.to_owned())
        });
    }

    /// Reports a refusal: the seam's own sentence, [`htui_store::DsnError`]'s, or one of this
    /// section's.
    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// A fresh snapshot: every row is replaced whole, and the write that asked for it — if any —
    /// says what it did.
    ///
    /// `busy` is the whole of the attribution: a `Connection` reply carries nothing that names the
    /// request it answers, which is the same trade the other three sections make and argue in the
    /// same place. `r` sets no `busy`, so a reload that lands while a question is on screen leaves
    /// it alone.
    fn on_snapshot(&mut self, snapshot: &ConnectionSnapshot) {
        let write = self.busy.take();
        // A read that answered is the end of an outage: leaving `unavailable` set would say the
        // connection is unreadable over a worker that just described it.
        self.unavailable = None;
        let offline = snapshot.offline;
        self.snapshot = Some(snapshot.clone());

        match write {
            Some(name) if name == REQUEST_NAMES[1] => {
                self.mode = Mode::Browse;
                self.say(if offline { STORED_OFFLINE } else { STORED });
            }
            Some(name) if name == REQUEST_NAMES[2] => {
                self.mode = Mode::Browse;
                self.say(CLEARED);
            }
            Some(name) if name == REQUEST_NAMES[3] => {
                self.mode = Mode::Browse;
                self.say(REBUILT);
            }
            _ => {}
        }

        // D8: the user lands on the field, and this section decides that rather than a third
        // action variant. `write.is_none()` is what keeps a `ClearDsn`'s own report from being
        // overwritten by the editor it would otherwise pop open over it — the box genuinely has
        // no DSN at that moment, and `e` is one key away, but `CLEARED` is the sentence that
        // answers the key the user just pressed.
        // `NotStored` and nothing else: a keyring that could not be *read* knows nothing about a
        // DSN, and opening a masked field over it would ask for a credential the store cannot
        // hold — with the user's real one still there, unreadable, when it unlocks.
        if write.is_none()
            && snapshot.dsn_state == DsnState::NotStored
            && !self.opened_for_empty
            && matches!(self.mode, Mode::Browse)
        {
            self.opened_for_empty = true;
            self.open_edit();
            self.say(NO_DSN_YET);
        }
    }
}

impl SettingsSection for ConnectionSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Connection"
    }

    fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
        // Not scoped: the connection belongs to the process, not to a workspace (B-6).
        vec![StoreRequest::ConnectionInfo]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The snapshot survives — nothing in it is scoped — but a half-typed DSN does not: a
        // scope change is one of the three disposal points the secret-handling rule names, beside
        // `Esc` and the `take` at submit.
        self.mode = Mode::Browse;
        self.opened_for_empty = false;
        self.notice = None;
    }

    fn captures_input(&self) -> bool {
        // D17 / ANA-10 §4.9 (2). The tab checks this before it takes `h`/`l`/`[`/`]` for section
        // cycling, which is why a DSN containing an `l` types as an `l`.
        !matches!(self.mode, Mode::Browse)
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::Editing(_) => return self.on_editor_key(key, ctx),
            Mode::ConfirmClear | Mode::ConfirmRebuild { .. } => {
                return self.on_confirm_key(key, ctx);
            }
            Mode::Browse => {}
        }
        // Browse. `e`, `c`, `R`, `r`, `j`, `k` are free: the global table binds `q`, `?`, the
        // digits, `ctrl-c` and `-`, and the tab consumes `h`/`l`/`[`/`]`/arrows before a section
        // is offered the key.
        match key.code {
            KeyCode::Char('e') => {
                if !self.blocked() {
                    self.open_edit();
                }
                Handled::Consumed
            }
            KeyCode::Char('c') => {
                if !self.blocked() {
                    if self.dsn_state().is_some_and(DsnState::is_stored) {
                        self.notice = None;
                        self.mode = Mode::ConfirmClear;
                    } else {
                        // The other three: there is no keyring entry to remove, there is no
                        // keyring at all in a demo session, and over an unreadable one there is
                        // nothing *known* to clear — a delete there would be a guess at a store
                        // that has not answered.
                        self.refuse(self.dsn_row_refusal());
                    }
                }
                Handled::Consumed
            }
            // Upper case on purpose (D16): `r` is the shipped re-read on every section, and a
            // destructive action must not sit one shift away from a reflex.
            KeyCode::Char('R') => {
                self.rebuild();
                Handled::Consumed
            }
            // B-9: the Rebuild row is an action row, so the key that runs actions runs it.
            KeyCode::Enter if self.row() == Row::Rebuild => {
                self.rebuild();
                Handled::Consumed
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_cursor(true);
                Handled::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_cursor(false);
                Handled::Consumed
            }
            // Allowed whatever else is going on: re-reading is how a section that lost a reply
            // recovers, and a read cannot lose a write's reply — the staleness index is keyed by
            // request kind.
            KeyCode::Char('r') => {
                ctx.request(StoreRequest::ConnectionInfo);
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

    fn on_reply(&mut self, reply: &StoreReply, _ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Connection(snapshot) => self.on_snapshot(snapshot),
            // The read itself was refused: saying so beats four rows of `not read yet`, which
            // read as "nothing here yet" over a worker whose last word was that it could not
            // answer.
            StoreReply::Failed { request, message } if *request == READ_NAME => {
                self.busy = None;
                self.unavailable = Some(message.clone());
            }
            // Every other refusal of this section's own. The shell has already put
            // `{request}: {message}` on the status line, so all this owes is the sentence and a
            // state the next key can start from — which means leaving a question that was refused,
            // because there is nothing left to answer `y` to.
            StoreReply::Failed { request, message } if REQUEST_NAMES.contains(request) => {
                self.busy = None;
                self.mode = Mode::Browse;
                self.refuse(message.clone());
            }
            _ => {}
        }
    }

    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let pane = self.pane(area.width, ctx.theme);
        let [rows, pane_area, hint] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(u16::try_from(pane.len()).unwrap_or(u16::MAX)),
            Constraint::Length(1),
        ])
        .areas(area);

        match &self.unavailable {
            // The refusal wins the rows even with a snapshot behind it: what is on screen would
            // otherwise be a connection nothing has confirmed since the outage started.
            Some(why) => frame.render_widget(
                Paragraph::new(Line::styled(
                    format!("{UNAVAILABLE}: {why}"),
                    ctx.theme.error,
                ))
                .wrap(Wrap { trim: true }),
                rows,
            ),
            None => frame.render_widget(Paragraph::new(self.lines(rows.width, ctx.theme)), rows),
        }
        if !pane.is_empty() {
            frame.render_widget(Paragraph::new(pane), pane_area);
        }
        frame.render_widget(Paragraph::new(self.hint(area.width, ctx.theme)), hint);
    }
}

/// The widest row label, so the value column lines up whatever the rows are called.
///
/// Computed rather than a constant: a renamed row moves the column by itself, and four `&'static
/// str`s per frame is not a cost worth caching against.
fn label_width() -> usize {
    Row::ALL
        .iter()
        .map(|row| row.label().chars().count())
        .max()
        .unwrap_or(0)
}

/// The DSN row's value column, and the sentence `c` refuses with when there is nothing to clear.
///
/// One function for both so the row and the refusal cannot disagree about why a key did nothing.
fn dsn_row(state: &DsnState, summary: Option<&String>) -> String {
    match (state, summary) {
        (DsnState::NotApplicable, _) => DEMO_ROW.to_owned(),
        (DsnState::NotStored, _) => NOT_STORED.to_owned(),
        (DsnState::Unreadable(why), _) => format!("{UNREADABLE}: {why}"),
        (DsnState::Stored, Some(summary)) => format!("{STORED} \u{2014} {summary}"),
        (DsnState::Stored, None) => STORED_UNREADABLE.to_owned(),
    }
}

/// The Status row's second half: the last dial and how it ended (B-5).
///
/// `Failed`'s text is sqlx's connection error, which the worker already logs; showing it here is
/// what puts it in front of the person who can act on it rather than only in a file.
fn attempt_text(snapshot: &ConnectionSnapshot) -> String {
    let Some(attempt) = &snapshot.last_attempt else {
        return NO_ATTEMPT.to_owned();
    };
    let at = attempt.at.format("%H:%M:%S");
    match &attempt.outcome {
        AttemptOutcome::Online => format!("last dial {at}: online"),
        AttemptOutcome::MigrationsPending(n) => format!("last dial {at}: {n} migrations pending"),
        AttemptOutcome::Failed(why) => format!("last dial {at}: failed: {why}"),
    }
}

/// Either question, wrapped and in `theme.error`: it is a line the user has to act on rather than
/// read.
fn question(text: &str, room: usize, theme: &Theme) -> Vec<Line<'static>> {
    wrapped(text, room)
        .into_iter()
        .map(|line| Line::styled(line, theme.error))
        .collect()
}

/// What a second write is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hard rule reaches the *mode* as well as the editor: [`ConnectionSection`] derives
    /// `Debug` through [`Mode`], so a variant that printed its own field's text would put a DSN
    /// in the `--log` file the moment one `tracing::debug!` names the section.
    #[test]
    fn an_editor_never_prints_its_buffer() {
        let mut input = TextField::masked();
        for c in "postgres://u:pw@h/d".chars() {
            input.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let section = ConnectionSection {
            mode: Mode::Editing(Editor { input }),
            ..ConnectionSection::new()
        };

        let printed = format!("{section:?}");

        assert!(
            !printed.contains("postgres") && !printed.contains("pw"),
            "what was typed stays out of the line: {printed}"
        );
        assert!(
            printed.contains("Editing") && printed.contains("len: 19"),
            "the mode and the length are still legible: {printed}"
        );
    }

    /// B-8: a notice's text is the seam's, this module's or `DsnError`'s — none of which carries
    /// anything typed — and the printer does not depend on that staying true.
    #[test]
    fn a_notice_never_prints_its_text() {
        let printed = format!("{:?}", Notice::Error("postgres://u:pw@h/d".to_owned()));

        assert!(!printed.contains("postgres"), "{printed}");
        assert!(
            printed.contains("Error") && printed.contains("len: 19"),
            "{printed}"
        );
    }

    /// The `busy` arms read the names from [`REQUEST_NAMES`] rather than spelling them, so a
    /// request renamed in one place cannot be matched by a literal in the other.
    #[test]
    fn the_request_names_are_the_module_constants() {
        assert_eq!(
            [REQUEST_NAMES[1], REQUEST_NAMES[2], REQUEST_NAMES[3]],
            [
                StoreRequest::SetDsn(Dsn::parse("postgres://h:5432/d").expect("a parseable DSN"))
                    .name(),
                StoreRequest::ClearDsn.name(),
                StoreRequest::RebuildCache.name(),
            ],
            "the three writers, in the order `on_snapshot` matches them"
        );
        assert_eq!(READ_NAME, REQUEST_NAMES[0]);
    }

    /// D14: both lists, in one question, so a reader of this file sees what the key destroys.
    #[test]
    fn the_rebuild_copy_names_both_lists() {
        let tables = format!("{} mirrored tables", MIRRORED_TABLES.len());
        for named in [
            "schema_version",
            "db_fingerprint",
            "built_at",
            "pending/",
            tables.as_str(),
            "cache_cursor",
            "last_full_refresh_at",
        ] {
            assert!(confirm_rebuild().contains(named), "`{named}`");
        }
    }
}
