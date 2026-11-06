//! The Qdrant section of the Settings tab: the database-wide Qdrant URL, this box's API key, and
//! the old keyring URL offered once (CLEAN-8, MOD-10 M4, MOD-158 D9, D10).
//!
//! It holds no store handle (`R-NF-3`): it names one read ([`StoreRequest::QdrantInfo`]), is
//! handed the [`QdrantSnapshot`] that comes back, and every write leaves through `ctx.request`.
//! The URL row writes the `qdrant_url` row by compare-and-set, with the token the snapshot
//! carried; a miss comes back as [`StoreReply::QdrantStale`] and the section takes the fresh
//! token from it. The key is typed into a masked field, read once by `take`, and travels
//! [`Redacted`]; no `Debug` here prints a field or a notice.
//!
//! The rows are one kit form (MOD-115 M3 D3): `[URL, old URL?, API key]`. An open field is its own
//! row right under the row it replaces, its guide or refusal under it; a question is the error
//! line of the focused row; the last outcome is a kit notice line under the form. Every sentence
//! that names a key reads it from the keys in force (MD-A, C5), and the acts resolve from
//! [`Ctx::pressed`].
//!
//! Under the rows, MOD-160 D9's probe row: each snapshot that moved (and each `r`) asks for one
//! [`StoreRequest::ProbeQdrant`], a model-free look at the Qdrant this process uses, served off
//! the store loop; a warning state draws one row in the theme's warning style, a Qdrant that
//! answers without a key an info line. Offline and `--demo` ask nothing: the URL row says why.

use std::borrow::Cow;
use std::cell::Cell;

use htui_core::model::Scope;
use htui_core::qdrant::{QdrantUrl, QdrantUrlError};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{Ctx, Handled};
use crate::concepts_worker::QdrantCheck;
use crate::keys::{Act, Hint, HintSpec, Keys, Stack, views};
use crate::qdrant_settings_info::{
    NO_OLD_URL, OldUrl, QdrantSnapshot, QdrantState, URL_WRITE_OFFLINE, UrlRowState,
};
use crate::secrets_settings::{DEMO_SESSION, Redacted};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::kit::empty::{self, NextStep};
use crate::ui::kit::form::{self, Form, Value};
use crate::ui::kit::pane::ScrollSpan;
use crate::ui::kit::text::{Tone, notice, wrapped};
use crate::ui::kit::{chips::key_line, modal::modal_rest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE_CLOSED, SectionId, SettingsSection, answers, asked,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

const NOT_READ: &str = "not read yet";
const STORED: &str = "stored";
const EMPTY_KEY: &str = "nothing typed; the stored API key is unchanged";
const EMPTY_URL: &str = "nothing typed; the Qdrant URL is unchanged";
/// CLEAN-8 #9: every row in a demo session, which has no keyring and no database.
const DEMO_ROW: &str = "n/a in a demo session";
const UNREADABLE: &str = "the keyring could not be read";
/// The read itself was refused; the seam's sentence follows it.
const UNAVAILABLE: &str = "Qdrant settings are unavailable";
/// The key field's guide. Not "Enter API key": that named a verb with a key's name (C5, D-15).
const KEY_GUIDE: &str = "type the API key, or leave it blank if there is none";
/// The URL row's label.
const URL_LABEL: &str = "URL";
/// The old keyring URL's row (MOD-158 D10).
const OFFER_LABEL: &str = "old URL";
/// The key row's label.
const KEY_LABEL: &str = "API key";
/// The open URL field's label, under the URL row.
const NEW_URL_LABEL: &str = "new URL";
/// The open key field's label, under the key row.
const NEW_KEY_LABEL: &str = "new key";
/// One label column for every row and field, so opening a field never moves the values.
const LABEL_WIDTH: u16 = 8;
/// What the URL row says it applies to (MOD-158 D9): one row for the whole database.
const EVERY_BOX: &str = "for every box";
/// The URL row offline (MOD-158 D9): the row lives in Postgres.
const URL_OFFLINE_ROW: &str = "cannot be read while Postgres is offline";
/// `HTUI_QDRANT_API_KEY` in force (MOD-158 D4).
const KEY_OVERRIDE: &str = "HTUI_QDRANT_API_KEY is in force in this process";

/// The notices after a write's reply (MOD-158 D9, D10).
const URL_SET: &str = "the Qdrant URL is set for every box";
const URL_CLEARED: &str = "the Qdrant URL is cleared for every box";
const KEY_CLEARED: &str = "this box's API key is gone from the keyring";
const ADOPTED: &str =
    "the old URL is now the Qdrant URL for every box; this box's keyring entry is gone";
const DISCARDED: &str = "the old Qdrant URL is gone from this box's keyring";

/// The questions (MOD-158 D9, Q2): one per row, the answers appended as the keys bind them
/// (MOD-67 M6 PA-6).
const CONFIRM_CLEAR_URL: &str = "Remove the Qdrant URL for every box?";
const CONFIRM_CLEAR_KEY: &str = "Remove this box's Qdrant API key from the keyring?";
const CONFIRM_DISCARD: &str = "Discard the old Qdrant URL from this box's keyring?";
/// The adopt's question when the old URL can no longer be named (MOD-158 R1, C-L1): never the
/// discard's, since `confirm.yes` here sends the adopt.
const CONFIRM_ADOPT_OLD: &str = "Use the old Qdrant URL for every box?";
/// A miss on a URL write no editor was open for (the clear, the adopt; MOD-158 R1, C-L4):
/// [`CHANGED_ELSEWHERE_CLOSED`] would send the user to an editor that never was.
const URL_CHANGED_ELSEWHERE: &str =
    "changed elsewhere; nothing was written \u{2014} the URL row shows the Qdrant URL now";
/// What the URL row's refusal says when its read failed.
const URL_NOT_READ: &str = "the Qdrant URL could not be read";

/// What the open URL field says when no URL is set. `submit` is `field.submit`'s label through
/// `CAPTURE` (MD-A, C5); with none bound the sentence says what to do without a key.
fn no_url_yet(submit: Option<&str>) -> String {
    match submit {
        Some(key) => format!("no Qdrant URL is set; type one and press {key}"),
        None => "no Qdrant URL is set; type one to set it".to_owned(),
    }
}

/// What the open URL field says over a row that is there; `submit` as [`no_url_yet`]'s.
fn replaces_url(submit: Option<&str>) -> String {
    match submit {
        Some(key) => format!("{key} replaces the Qdrant URL for every box"),
        None => "setting replaces the Qdrant URL for every box".to_owned(),
    }
}

/// `c` on a row with nothing in it (MOD-83): what is the case and the key that changes it, rather
/// than a claim that the settings are unavailable. `edit` is `Edit`'s label through
/// `QDRANT_BROWSE`.
fn nothing_to_clear(row: Row, edit: Option<&str>) -> String {
    let (said, verb) = match row {
        Row::Url => (
            "no Qdrant URL is set, so there is nothing to clear",
            "sets one",
        ),
        Row::Key | Row::Offer => (
            "no Qdrant API key is stored on this box, so there is nothing to clear",
            "stores one",
        ),
    };
    match edit {
        Some(key) => format!("{said}; {key} {verb}"),
        None => said.to_owned(),
    }
}

/// `c` over an unreadable row (MOD-83): the row's own words and the key that reads it again,
/// never [`nothing_to_clear`], since nobody knows whether anything is there. `reload` is
/// `Reload`'s label through `QDRANT_BROWSE`.
fn unreadable_refusal(what: &str, why: &str, reload: Option<&str>) -> String {
    match reload {
        Some(key) => format!("{what}: {why}; {key} reads it again"),
        None => format!("{what}: {why}"),
    }
}

/// The URL row's refusal when its read failed: its own words and the key that reads it again.
fn url_not_read(why: &str, keys: &Keys) -> String {
    let reload = keys.label(views::QDRANT_BROWSE, Act::Reload);
    unreadable_refusal(URL_NOT_READ, why, reload.as_deref())
}

/// The URL row's words when its read failed (MOD-158 D9).
fn url_unreadable(why: &str) -> String {
    format!("could not be read: {why}")
}

/// The next step of [`UNAVAILABLE`] (MOD-83): the reload key in force, or what happens without
/// one.
fn asks_again(reload: Option<&str>) -> String {
    reload.map_or_else(
        || "it is asked again when the section opens".to_owned(),
        |key| format!("{key} asks again once the store answers"),
    )
}

/// `HTUI_QDRANT_URL` in force, under the URL row (MOD-158 D3, D9). `why` is the shipped
/// [`QdrantUrlError`](htui_core::qdrant::QdrantUrlError) sentence (blueprint P-20).
fn url_override(found: &Result<QdrantUrl, QdrantUrlError>) -> String {
    match found {
        Ok(url) => format!(
            "HTUI_QDRANT_URL={url} is in force in this process, which does not use this setting"
        ),
        Err(why) => format!("HTUI_QDRANT_URL is not usable ({why}); this process cannot search"),
    }
}

/// The old URL row (MOD-158 D10): what this box's keyring holds and the keys in force that use
/// or discard it. `url_row` decides whether it can be used: only over no row.
fn offer_text(
    old: &OldUrl,
    url_row: &UrlRowState,
    adopt: Option<&str>,
    clear: Option<&str>,
) -> String {
    let discards = clear.map(|key| format!("{key} discards it"));
    let (said, verbs) = match old {
        OldUrl::Unusable(why) => (
            format!("this box's keyring holds an old Qdrant URL that is not usable ({why})"),
            discards.into_iter().collect::<Vec<_>>(),
        ),
        OldUrl::Usable(url) => match url_row {
            UrlRowState::NotSet => (
                format!("this box's keyring holds {url} from before"),
                adopt
                    .map(|key| format!("{key} uses it for every box"))
                    .into_iter()
                    .chain(discards)
                    .collect(),
            ),
            UrlRowState::Set { .. } | UrlRowState::Unusable { .. } => (
                format!("this box's keyring holds {url} from before, no longer used"),
                discards.into_iter().collect(),
            ),
            UrlRowState::Offline | UrlRowState::Unreadable(_) | UrlRowState::NotApplicable => (
                format!("this box's keyring holds {url} from before"),
                discards.into_iter().collect(),
            ),
        },
    };
    if verbs.is_empty() {
        said
    } else {
        format!("{said}: {}", verbs.join(", "))
    }
}

/// `e` on the old URL row: it is not edited, it is used or discarded.
fn offer_is_not_edited(adopt: Option<&str>, clear: Option<&str>, adoptable: bool) -> String {
    let uses = adopt
        .filter(|_| adoptable)
        .map(|key| format!("{key} uses the old URL"));
    let discards = clear.map(|key| format!("{key} discards it"));
    let verbs: Vec<String> = uses.into_iter().chain(discards).collect();
    if verbs.is_empty() {
        "the old URL is not edited here".to_owned()
    } else {
        verbs.join(", ")
    }
}

/// The adopt's question (MOD-158 D10).
fn confirm_adopt(url: &QdrantUrl) -> String {
    format!("Use {url} for every box?")
}

// The hint rows (MOD-67 M3 D9): rendered through the mode's stack, so a rebound key shows its
// new chord. The editors' keys are `field.submit` and `field.cancel` through `CAPTURE` (MOD-115
// M3 C6): `Enter store \u{b7} Esc cancel` under the compiled keys.
const HINT_BROWSE: HintSpec = &[
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Clear, "clear"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
/// MOD-158 D10: on the old URL row, while it can be used for every box.
const HINT_BROWSE_OFFER: HintSpec = &[
    Hint::One(Act::QdrantAdopt, "use for every box"),
    Hint::One(Act::Clear, "discard"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
/// MOD-158 D10: on the old URL row, once it can only be discarded.
const HINT_BROWSE_DISCARD: HintSpec = &[
    Hint::One(Act::Clear, "discard"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_NO_SNAPSHOT: HintSpec = &[
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
const HINT_EDITING: HintSpec = &[
    Hint::One(Act::FieldSubmit, "store"),
    Hint::One(Act::FieldCancel, "cancel"),
];
const HINT_EDITING_KEY: HintSpec = &[
    Hint::One(Act::FieldSubmit, "store"),
    Hint::One(Act::FieldCancel, "cancel"),
    Hint::Text("typed text is never shown"),
];
const HINT_CONFIRM: HintSpec = &[
    Hint::One(Act::ConfirmYes, "confirm"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

/// One row of the form, in order: the URL, the old URL while there is one, the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Url,
    Offer,
    Key,
}

struct Editor {
    input: TextField,
}

/// What a question asks (MOD-158 D9, D10): each row's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    /// The URL row, for every box.
    ClearUrl,
    /// This box's key.
    ClearKey,
    /// The old keyring URL, discarded.
    Discard,
    /// The old keyring URL, used for every box.
    Adopt,
}

impl Question {
    /// The row the question is asked on, and stays on while a snapshot moves the rows (C-L2).
    const fn row(self) -> Row {
        match self {
            Self::ClearUrl => Row::Url,
            Self::ClearKey => Row::Key,
            Self::Discard | Self::Adopt => Row::Offer,
        }
    }
}

#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    Confirm(Question),
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

/// The snapshot fields a probe was sent for (MOD-160 D9): the URL row, the override and the key.
/// The same three again ask for no second probe; any of them moving asks for one.
type ProbedFor = (
    UrlRowState,
    Option<Result<QdrantUrl, QdrantUrlError>>,
    QdrantState,
);

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
    /// The form's `(lines, first line drawn)` as of the last render, for the Settings pane's
    /// scrollbar (MOD-115 M3 D-11), the notice rows under it counted; `None` while the refusal
    /// stands in for the rows.
    scroll: Cell<Option<ScrollSpan>>,
    /// The last probe's answer (MOD-160 D9); `None` until it answers, or when nothing was probed.
    probe: Option<QdrantCheck>,
    /// What the last probe was sent for; `None` before the first, after `r`, after a snapshot
    /// that is not probed (offline, `--demo`), and once a write is answered or refused: a written
    /// key reads the same (`Stored`) as the one it replaced.
    probed_for: Option<ProbedFor>,
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
            scroll: Cell::new(None),
            probe: None,
            probed_for: None,
        }
    }

    fn url_row(&self) -> Option<&UrlRowState> {
        self.snapshot.as_ref().map(|s| &s.url)
    }

    fn key_state(&self) -> Option<&QdrantState> {
        self.snapshot.as_ref().map(|s| &s.key_state)
    }

    fn old_url(&self) -> Option<&OldUrl> {
        self.snapshot.as_ref().and_then(|s| s.old_url.as_ref())
    }

    /// CLEAN-8 #9: the snapshot is a demo session's, which has nothing to edit.
    fn demo(&self) -> bool {
        self.url_row() == Some(&UrlRowState::NotApplicable)
    }

    /// The rows in order (MOD-158 D10): the old URL only while this box's keyring holds one.
    fn rows(&self) -> &'static [Row] {
        if self.old_url().is_some() {
            &[Row::Url, Row::Offer, Row::Key]
        } else {
            &[Row::Url, Row::Key]
        }
    }

    fn row(&self) -> Row {
        let rows = self.rows();
        rows[self.cursor.min(rows.len() - 1)]
    }

    /// Where `row` is in the form, or `None` when it is not drawn.
    fn index_of(&self, row: Row) -> Option<usize> {
        self.rows().iter().position(|r| *r == row)
    }

    /// The old URL can be used for every box: it parses and no row is set (MOD-158 D10).
    fn adoptable(&self) -> Option<&QdrantUrl> {
        match (self.old_url(), self.url_row()) {
            (Some(OldUrl::Usable(url)), Some(UrlRowState::NotSet)) => Some(url),
            _ => None,
        }
    }

    fn move_cursor(&mut self, down: bool) {
        // Moving the cursor is not a write, so a write in flight does not stop it.
        if self.snapshot.is_none() || self.unavailable.is_some() {
            return;
        }
        let len = self.rows().len();
        let at = self.cursor.min(len - 1);
        self.cursor = if down {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        };
    }

    /// Whether a key that opens an editor or a question is refused right now. A write in flight
    /// says so, as the Connection section's does, and so does a demo session (CLEAN-8 #9), as the
    /// Secrets section's does; `r` is deliberately not on this path.
    fn blocked(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy));
            return true;
        }
        if self.demo() {
            self.refuse(DEMO_SESSION.to_owned());
            return true;
        }
        self.unavailable.is_some() || self.snapshot.is_none()
    }

    /// Why the URL row refuses a write right now, or `None` when it takes one.
    fn url_refusal(&self, keys: &Keys) -> Option<String> {
        match self.url_row()? {
            UrlRowState::Offline => Some(URL_WRITE_OFFLINE.to_owned()),
            UrlRowState::Unreadable(why) => Some(url_not_read(why, keys)),
            UrlRowState::NotApplicable => Some(DEMO_SESSION.to_owned()),
            UrlRowState::Set { .. } | UrlRowState::Unusable { .. } | UrlRowState::NotSet => None,
        }
    }

    fn open_url_editor(&mut self) {
        self.mode = Mode::EditingUrl(Editor {
            input: TextField::new(),
        });
    }

    /// `e` on the focused row.
    fn on_edit(&mut self, keys: &Keys) {
        match self.row() {
            Row::Url => match self.url_refusal(keys) {
                Some(why) => self.refuse(why),
                None => self.open_url_editor(),
            },
            Row::Key => {
                self.mode = Mode::EditingKey(Editor {
                    input: TextField::masked(),
                });
            }
            Row::Offer => {
                let adopt = keys.label(views::QDRANT_BROWSE, Act::QdrantAdopt);
                let clear = keys.label(views::QDRANT_BROWSE, Act::Clear);
                let adoptable = self.adoptable().is_some();
                self.refuse(offer_is_not_edited(
                    adopt.as_deref(),
                    clear.as_deref(),
                    adoptable,
                ));
            }
        }
    }

    /// `c` on the focused row (MOD-158 Q2): that row's question, or why there is none.
    fn on_clear(&mut self, keys: &Keys) {
        let row = self.row();
        let edit = keys.label(views::QDRANT_BROWSE, Act::Edit);
        let reload = keys.label(views::QDRANT_BROWSE, Act::Reload);
        let question = match row {
            // Every state its own arm (C-L6): a new one cannot fall through to an empty refusal.
            Row::Url => match self.url_row() {
                Some(UrlRowState::Set { .. } | UrlRowState::Unusable { .. }) => {
                    Ok(Question::ClearUrl)
                }
                Some(UrlRowState::NotSet) => Err(nothing_to_clear(row, edit.as_deref())),
                Some(UrlRowState::Offline) => Err(URL_WRITE_OFFLINE.to_owned()),
                Some(UrlRowState::Unreadable(why)) => Err(url_not_read(why, keys)),
                Some(UrlRowState::NotApplicable) => Err(DEMO_SESSION.to_owned()),
                // Nothing read yet: `blocked` stops every key that gets here first.
                None => return,
            },
            Row::Key => match self.key_state() {
                Some(QdrantState::Stored) => Ok(Question::ClearKey),
                // An unreadable row is not an empty one (Connection's rule): its own words,
                // never a claim that nothing is stored.
                Some(QdrantState::Unreadable(why)) => {
                    Err(unreadable_refusal(UNREADABLE, why, reload.as_deref()))
                }
                _ => Err(nothing_to_clear(row, edit.as_deref())),
            },
            Row::Offer => Ok(Question::Discard),
        };
        match question {
            Ok(question) => {
                self.notice = None;
                self.mode = Mode::Confirm(question);
            }
            Err(why) => self.refuse(why),
        }
    }

    /// `u` on the old URL row (MOD-158 D10): the question, or why the old URL cannot be used.
    fn on_adopt(&mut self, keys: &Keys) {
        let clear = keys.label(views::QDRANT_BROWSE, Act::Clear);
        let discards = clear.map_or_else(String::new, |key| format!("; {key} discards it"));
        // Every pair its own arm (C-L6): a new state cannot fall through to an empty refusal.
        let why = match (self.old_url(), self.url_row()) {
            (Some(OldUrl::Usable(_)), Some(UrlRowState::NotSet)) => {
                self.notice = None;
                self.mode = Mode::Confirm(Question::Adopt);
                return;
            }
            (Some(OldUrl::Unusable(why)), _) => {
                format!("the old Qdrant URL is not usable ({why}){discards}")
            }
            (None, _) => NO_OLD_URL.to_owned(),
            (
                Some(OldUrl::Usable(_)),
                Some(UrlRowState::Set { .. } | UrlRowState::Unusable { .. }),
            ) => {
                format!("a Qdrant URL is already set for every box{discards}")
            }
            (Some(OldUrl::Usable(_)), Some(UrlRowState::Offline)) => URL_WRITE_OFFLINE.to_owned(),
            (Some(OldUrl::Usable(_)), Some(UrlRowState::Unreadable(why))) => {
                url_not_read(why, keys)
            }
            (Some(OldUrl::Usable(_)), Some(UrlRowState::NotApplicable)) => DEMO_SESSION.to_owned(),
            // Nothing read yet: `blocked` stops every key that gets here first.
            (Some(OldUrl::Usable(_)), None) => return,
        };
        self.refuse(why);
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
                self.notice = None;
                Handled::Consumed
            }
            FieldOutcome::Submit => {
                if let Some(url) = url_to_submit {
                    self.submit_url(&url, ctx);
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
            // The chord the user pressed (MOD-115 M3 MD-A): a `[field]` chord the field passed is
            // the field's, never re-read as another act of its canonical key.
            FieldOutcome::Pass => modal_rest(views::CAPTURE, ctx.pressed(&key)),
        }
    }

    /// The URL field's submit (MOD-158 D2, D9): checked here first, so a URL D2 refuses never
    /// leaves the field; its sentence stays under it.
    fn submit_url(&mut self, text: &str, ctx: &Ctx<'_>) {
        if text.is_empty() {
            self.mode = Mode::Browse;
            self.say(EMPTY_URL);
            return;
        }
        let url = match QdrantUrl::parse(text) {
            Ok(url) => url,
            Err(why) => {
                self.refuse(why.to_string());
                return;
            }
        };
        // The row may have moved while the field was open: the token of the snapshot now.
        if let Some(why) = self.url_refusal(ctx.keys()) {
            self.mode = Mode::Browse;
            self.refuse(why);
            return;
        }
        let expected = self.url_row().and_then(UrlRowState::token);
        self.mode = Mode::Browse;
        self.send(StoreRequest::SetQdrantUrl { url, expected }, ctx);
    }

    /// A question (MOD-67 M3 §6.4): `confirm.yes` answers it, `confirm.no` goes back, a chord the
    /// modal global layer admits passes, and everything else is swallowed.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::QDRANT_CONFIRM;
        let chord = ctx.pressed(&key);
        let Mode::Confirm(question) = self.mode else {
            return Handled::Pass;
        };
        // The first candidate decides (MOD-67 M3 §6.4): a global act there is the pass rule's.
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes) => {
                self.mode = Mode::Browse;
                let request = match question {
                    Question::ClearUrl => {
                        match self.url_row().and_then(UrlRowState::token) {
                            Some(expected) => StoreRequest::ClearQdrantUrl { expected },
                            // The row went while the question was up: nothing to clear, and no
                            // editor to reopen (C-L4).
                            None => {
                                self.refuse(URL_CHANGED_ELSEWHERE.to_owned());
                                return Handled::Consumed;
                            }
                        }
                    }
                    Question::ClearKey => StoreRequest::ClearQdrantApiKey,
                    Question::Discard => StoreRequest::DiscardOldQdrantUrl,
                    Question::Adopt => StoreRequest::AdoptOldQdrantUrl,
                };
                self.send(request, ctx);
                return Handled::Consumed;
            }
            Some(Act::ConfirmNo) => {
                self.mode = Mode::Browse;
                return Handled::Consumed;
            }
            _ => {}
        }
        modal_rest(stack, chord)
    }

    /// The stack of the current mode (MOD-67 D4): the one place a mode maps to its keys;
    /// `key_stack`, the key handlers and the hint all read it.
    fn stack(&self) -> Stack<'static> {
        match self.mode {
            Mode::Browse => views::QDRANT_BROWSE,
            Mode::Confirm(_) => views::QDRANT_CONFIRM,
            Mode::EditingUrl(_) | Mode::EditingKey(_) => views::CAPTURE,
        }
    }

    /// One key in Browse (MOD-67 M3 §6.2): whole chords through `QDRANT_BROWSE`, so `ctrl-e`
    /// edits nothing and `ctrl-r` reads nothing (defect 1).
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let chord = ctx.pressed(&key);
        for act in ctx.keys().actions(views::QDRANT_BROWSE, chord) {
            match act {
                Act::Edit => {
                    if !self.blocked() {
                        self.on_edit(ctx.keys());
                    }
                    return Handled::Consumed;
                }
                Act::Clear => {
                    if !self.blocked() {
                        self.on_clear(ctx.keys());
                    }
                    return Handled::Consumed;
                }
                // MOD-158 D10: the old URL row's own key; on any other row it is not this
                // section's, so it falls through to `Pass`.
                Act::QdrantAdopt if self.snapshot.is_some() && self.row() == Row::Offer => {
                    if !self.blocked() {
                        self.on_adopt(ctx.keys());
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
                    // MOD-160 D9, "on its reload key": the read's snapshot probes again.
                    self.probed_for = None;
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

    /// The guide under the open URL field: what setting there does, named with the submit key in
    /// force (C5).
    fn url_guide(&self, keys: &Keys) -> String {
        let submit = keys.label(views::CAPTURE, Act::FieldSubmit);
        match self.url_row() {
            Some(UrlRowState::Set { .. } | UrlRowState::Unusable { .. }) => {
                replaces_url(submit.as_deref())
            }
            _ => no_url_yet(submit.as_deref()),
        }
    }

    /// The URL row's value (MOD-158 D9): the database-wide row, said to apply to every box.
    fn url_value(&self) -> Value<'static> {
        let text = match self.url_row() {
            Some(UrlRowState::Set { url, .. }) => format!("{url} \u{b7} {EVERY_BOX}"),
            Some(UrlRowState::Unusable { why, .. }) => {
                format!("not usable: {why} \u{b7} {EVERY_BOX}")
            }
            Some(UrlRowState::NotSet) => format!("not set \u{b7} {EVERY_BOX}"),
            Some(UrlRowState::Offline) => URL_OFFLINE_ROW.to_owned(),
            Some(UrlRowState::Unreadable(why)) => url_unreadable(why),
            Some(UrlRowState::NotApplicable) => DEMO_ROW.to_owned(),
            None => NOT_READ.to_owned(),
        };
        Value::ReadOnly(Cow::Owned(text))
    }

    /// The key row's value: a secret, stored or not, and never a placeholder that reads as data.
    fn key_value(&self) -> Value<'static> {
        match self.key_state() {
            Some(QdrantState::Stored) => Value::Secret(true),
            Some(QdrantState::NotStored) => Value::Secret(false),
            Some(QdrantState::Unreadable(why)) => {
                Value::ReadOnly(Cow::Owned(format!("{UNREADABLE}: {why}")))
            }
            Some(QdrantState::NotApplicable) => Value::ReadOnly(Cow::Borrowed(DEMO_ROW)),
            None => Value::ReadOnly(Cow::Borrowed(NOT_READ)),
        }
    }

    /// The open field's row: `label`, the field, and its guide or what was refused under it.
    fn field_row<'a>(
        &'a self,
        label: &'a str,
        input: &'a TextField,
        guide: String,
    ) -> form::Field<'a> {
        let field = form::Field::new(label, Value::Text(input));
        match &self.notice {
            Some(Notice::Error(text)) => field.error(text.as_str()),
            Some(Notice::Info(text)) => field.help(text.as_str()),
            None => field.help(guide),
        }
    }

    /// The question's sentence, its answers appended as the keys bind them.
    fn question_text(&self, question: Question, keys: &Keys) -> String {
        let text = match question {
            Question::ClearUrl => CONFIRM_CLEAR_URL.to_owned(),
            Question::ClearKey => CONFIRM_CLEAR_KEY.to_owned(),
            Question::Discard => CONFIRM_DISCARD.to_owned(),
            Question::Adopt => match self.old_url() {
                Some(OldUrl::Usable(url)) => confirm_adopt(url),
                Some(OldUrl::Unusable(_)) | None => CONFIRM_ADOPT_OLD.to_owned(),
            },
        };
        asked(&text, &answers(keys, views::QDRANT_CONFIRM))
    }

    /// The rows as one kit form (MOD-115 M3 D3): the cursor row focused while browsing; an open
    /// field on its own row right under the row it replaces (so what is set stays on screen); a
    /// question under the focused row; the override in force as the URL and key rows' help.
    fn form(&self, keys: &Keys) -> Form<'_> {
        let snapshot = self.snapshot.as_ref();
        let mut url = form::Field::new(URL_LABEL, self.url_value());
        if let Some(found) = snapshot.and_then(|s| s.url_override.as_ref()) {
            url = url.help(url_override(found));
        }
        let mut key = form::Field::new(KEY_LABEL, self.key_value());
        if snapshot.is_some_and(|s| s.key_override) {
            key = key.help(KEY_OVERRIDE);
        }
        let mut offer = snapshot.zip(self.old_url()).map(|(s, old)| {
            let adopt = keys.label(views::QDRANT_BROWSE, Act::QdrantAdopt);
            let clear = keys.label(views::QDRANT_BROWSE, Act::Clear);
            form::Field::new(
                OFFER_LABEL,
                Value::ReadOnly(Cow::Owned(offer_text(
                    old,
                    &s.url,
                    adopt.as_deref(),
                    clear.as_deref(),
                ))),
            )
        });
        if let Mode::Confirm(question) = self.mode {
            let asked = self.question_text(question, keys);
            match question.row() {
                Row::Url => url = url.error(asked),
                Row::Key => key = key.error(asked),
                Row::Offer => offer = offer.map(|field| field.error(asked)),
            }
        }
        let mut form = Form::new().label_width(LABEL_WIDTH).field(url);
        let mut items = 1;
        let mut focus = None;
        if let Mode::EditingUrl(editor) = &self.mode {
            form = form.field(self.field_row(NEW_URL_LABEL, &editor.input, self.url_guide(keys)));
            focus = Some(items);
            items += 1;
        }
        if let Some(offer) = offer {
            form = form.field(offer);
            items += 1;
        }
        form = form.field(key);
        items += 1;
        if let Mode::EditingKey(editor) = &self.mode {
            form = form.field(self.field_row(NEW_KEY_LABEL, &editor.input, KEY_GUIDE.to_owned()));
            focus = Some(items);
        }
        // Browsing or asking, the cursor row: the URL row is first, so an open URL field never
        // shifts it, and nothing is open under the others then.
        let focus = focus.unwrap_or_else(|| self.index_of(self.row()).unwrap_or(0));
        form.focus(focus).editing(matches!(
            self.mode,
            Mode::EditingUrl(_) | Mode::EditingKey(_)
        ))
    }

    /// The rows under the form in Browse: the last outcome, as kit notice lines in `theme.error`
    /// when it is one to act on. The keys this mode binds are on the pane's border (MOD-67 M6
    /// T2, [`SettingsSection::key_hint`]); the editors and the question draw theirs in the form.
    fn notice_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let (Some(said), Mode::Browse) = (&self.notice, &self.mode) else {
            return Vec::new();
        };
        let tone = if said.is_error() {
            Tone::Error
        } else {
            Tone::Info
        };
        wrapped(said.text(), usize::from(width).max(1))
            .into_iter()
            .map(|line| notice(&line, tone, theme))
            .collect()
    }

    /// MOD-160 D9: the probe's row under the form: a warning in `theme.warning`, the keyless info
    /// line in `theme.dim`, nothing for a ready Qdrant or no answer. Nothing either while the read
    /// is refused or the URL row cannot be read here (offline, `--demo`): the answer, if any, was
    /// about settings no longer on screen. A check carries no key, so neither does the row.
    fn probe_lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        if self.unavailable.is_some() || !matches!(self.url_row(), Some(row) if probes(row)) {
            return Vec::new();
        }
        let Some(check) = &self.probe else {
            return Vec::new();
        };
        let (text, style) = match (check.warning(), check.info()) {
            (Some(warning), _) => (htui_core::qdrant::unavailable(&warning), theme.warning),
            (None, Some(info)) => (info.to_owned(), theme.dim),
            (None, None) => return Vec::new(),
        };
        wrapped(&text, usize::from(width).max(1))
            .into_iter()
            .map(|line| Line::styled(line, style))
            .collect()
    }

    /// MOD-160 D9: after a snapshot (a read's or a compare-and-set miss's), one probe for what it
    /// shows, unless the last probe was sent for the same URL row, override and key. The old
    /// answer is dropped with it: it was about the settings before. A snapshot that is not probed
    /// (offline, `--demo`) forgets both, so the first online snapshot after it probes again even
    /// when it reads as before: the pre-outage answer may no longer hold.
    fn probe_for(&mut self, snapshot: &QdrantSnapshot, ctx: &Ctx<'_>) {
        if !probes(&snapshot.url) {
            self.probed_for = None;
            self.probe = None;
            return;
        }
        let now = (
            snapshot.url.clone(),
            snapshot.url_override.clone(),
            snapshot.key_state.clone(),
        );
        if self.probed_for.as_ref() == Some(&now) {
            return;
        }
        self.probed_for = Some(now);
        self.probe = None;
        ctx.request(StoreRequest::ProbeQdrant);
    }

    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match self.mode {
            Mode::EditingUrl(_) => HINT_EDITING,
            Mode::EditingKey(_) => HINT_EDITING_KEY,
            Mode::Confirm(_) => HINT_CONFIRM,
            Mode::Browse => {
                if self.unavailable.is_some() || self.snapshot.is_none() {
                    HINT_NO_SNAPSHOT
                } else if self.row() == Row::Offer {
                    if self.adoptable().is_some() {
                        HINT_BROWSE_OFFER
                    } else {
                        HINT_BROWSE_DISCARD
                    }
                } else {
                    HINT_BROWSE
                }
            }
        };
        let keys = keys.hint(self.stack(), spec);
        match self.busy {
            Some(busy) if matches!(self.mode, Mode::Browse) && self.notice.is_none() => {
                format!("{keys} \u{b7} {}\u{2026}", doing(busy))
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

    /// Takes `snapshot` as what is there now, the outage over. The cursor stays on the row it was
    /// on, found again by what it is, since the old URL's row comes and goes above the key's
    /// (C-L2); a question whose row is gone closes with it.
    fn take(&mut self, snapshot: &QdrantSnapshot) {
        let focused = match self.mode {
            Mode::Confirm(question) => question.row(),
            _ => self.row(),
        };
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
        match self.index_of(focused) {
            Some(at) => self.cursor = at,
            None => {
                self.cursor = self.cursor.min(self.rows().len() - 1);
                if matches!(self.mode, Mode::Confirm(_)) {
                    self.mode = Mode::Browse;
                }
            }
        }
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
        self.take(snapshot);
        // MOD-160 D9: a write's snapshot probes again even when it reads as before, since one
        // stored key replaced by another reads `Stored` both times.
        if write.is_some() {
            self.probed_for = None;
        }

        let said = match write {
            Some("set_qdrant_url") => Some(URL_SET),
            Some("clear_qdrant_url") => Some(URL_CLEARED),
            Some("set_qdrant_api_key") => Some(STORED),
            Some("clear_qdrant_api_key") => Some(KEY_CLEARED),
            Some("adopt_old_qdrant_url") => Some(ADOPTED),
            Some("discard_old_qdrant_url") => Some(DISCARDED),
            _ => None,
        };
        if let Some(said) = said {
            self.mode = Mode::Browse;
            self.say(said);
        }

        // Not over an override (this process does not use the row) nor an old keyring URL (the
        // offer row is the shorter way to a URL).
        if write.is_none()
            && snapshot.url == UrlRowState::NotSet
            && snapshot.url_override.is_none()
            && snapshot.old_url.is_none()
            && !self.opened_for_empty
            && matches!(self.mode, Mode::Browse)
        {
            // No notice: the open field's own guide says there is no URL, with the key in force.
            self.opened_for_empty = true;
            self.cursor = 0;
            self.open_url_editor();
        }
    }

    /// A compare-and-set miss (MOD-158 D9, blueprint R-3): the row moved under the write, which
    /// did not apply. The fresh snapshot carries the token the next write presents.
    ///
    /// Only the URL's set was typed into an editor, so only its miss says to reopen one; the clear
    /// and the adopt say the row shows the URL now (C-L4).
    fn on_stale(&mut self, snapshot: &QdrantSnapshot) {
        let write = self.busy.take();
        self.take(snapshot);
        self.mode = Mode::Browse;
        self.say(match write {
            Some("set_qdrant_url") => CHANGED_ELSEWHERE_CLOSED,
            _ => URL_CHANGED_ELSEWHERE,
        });
    }

    /// A refused request. A refused write may have landed part-way (the adopt's row set, its
    /// keyring entry kept) or have landed with its follow-up read failing, so the section reads
    /// itself again rather than keep a view nothing has confirmed (MOD-158 R1, C-M1); the
    /// refusal's sentence stays. A refused read is the outage.
    fn on_failed(&mut self, request: &str, message: &str, ctx: &Ctx<'_>) {
        if self.busy == Some(request) {
            self.busy = None;
            self.refuse(message.to_owned());
            self.read_out = true;
            // It may have landed before its follow-up read failed: the re-read probes again.
            self.probed_for = None;
            ctx.request(StoreRequest::QdrantInfo);
        } else if request == "qdrant_info" {
            self.read_out = false;
            self.unavailable = Some(message.to_owned());
        }
    }
}

/// MOD-160 D9: whether a snapshot with this URL row is probed. Offline the settings cannot resolve
/// and `--demo` has no Qdrant: the row says why, and a probe would say nothing more.
const fn probes(row: &UrlRowState) -> bool {
    !matches!(row, UrlRowState::Offline | UrlRowState::NotApplicable)
}

/// What a second write is told while the first is still out, in words (no request name).
fn in_flight(busy: &str) -> String {
    format!("still {} \u{2014} wait for the store's answer", doing(busy))
}

/// What the request `busy` is doing, as the user reads it.
fn doing(busy: &str) -> &'static str {
    match busy {
        "set_qdrant_url" => "setting the Qdrant URL for every box",
        "clear_qdrant_url" => "clearing the Qdrant URL for every box",
        "set_qdrant_api_key" => "storing the API key",
        "clear_qdrant_api_key" => "removing this box's API key",
        "adopt_old_qdrant_url" => "using the old Qdrant URL for every box",
        "discard_old_qdrant_url" => "discarding the old Qdrant URL",
        "qdrant_info" => "reading the Qdrant settings",
        "qdrant_old_url_check" => "looking for the old Qdrant URL",
        _ => "writing",
    }
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
            Mode::Confirm(_) => self.on_confirm_key(key, ctx),
            Mode::Browse => self.on_browse_key(key, ctx),
        }
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Qdrant(snapshot) => {
                self.on_snapshot(snapshot);
                self.probe_for(snapshot, ctx);
            }
            // A miss installs a fresh snapshot too, perhaps another box's URL: probe for it.
            StoreReply::QdrantStale(snapshot) => {
                self.on_stale(snapshot);
                self.probe_for(snapshot, ctx);
            }
            StoreReply::QdrantProbe(check) => self.probe.clone_from(check),
            StoreReply::Failed { request, message } => self.on_failed(request, message, ctx),
            _ => {}
        }
    }

    fn key_hint(&self, keys: &Keys, theme: &Theme) -> Option<Line<'static>> {
        key_line(self.hint_text(keys), theme)
    }

    fn scroll(&self) -> Option<ScrollSpan> {
        self.scroll.get()
    }

    /// The form over the rows, the notice under it; the refusal and its next step stand in for
    /// the rows while the read is refused.
    fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        let theme = ctx.theme;
        let said = self.notice_lines(area.width, theme);
        let probe = self.probe_lines(area.width, theme);
        let [rows, probe_area, notice_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(u16::try_from(probe.len()).unwrap_or(u16::MAX)),
            Constraint::Length(u16::try_from(said.len()).unwrap_or(u16::MAX)),
        ])
        .areas(area);
        self.scroll.set(None);
        match &self.unavailable {
            // The refusal wins the rows even with a snapshot behind it (Connection's rule): what
            // is on screen would otherwise be a keyring nothing has confirmed since the outage.
            Some(why) => {
                let mut lines: Vec<Line<'static>> = wrapped(
                    &format!("{UNAVAILABLE}: {why}"),
                    usize::from(rows.width).max(1),
                )
                .into_iter()
                .map(|line| notice(&line, Tone::Error, theme))
                .collect();
                // MOD-83: the next step, in the empty state's own role.
                let reload = ctx.keys().label(views::QDRANT_BROWSE, Act::Reload);
                let next = asks_again(reload.as_deref());
                lines.extend(empty::lines("", NextStep(&next), theme).pop());
                frame.render_widget(Paragraph::new(lines), rows);
            }
            None => {
                let mut laid = self
                    .form(ctx.keys())
                    .layout(rows.width, ctx.accent(), theme);
                // The form's own measure of the focused row (RV2 M1), not a guess.
                let offset = laid.offset(rows.height);
                let total = laid.lines.len();
                let shown: Vec<Line<'static>> = laid.lines.drain(offset.min(total)..).collect();
                frame.render_widget(Paragraph::new(shown), rows);
                // Beside the rows the form was drawn into (RV1 M2): the notice under them does
                // not scroll.
                self.scroll
                    .set(Some(ScrollSpan::new(total, offset).rows(rows)));
            }
        }
        if !probe.is_empty() {
            frame.render_widget(Paragraph::new(probe), probe_area);
        }
        if !said.is_empty() {
            frame.render_widget(Paragraph::new(said), notice_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MOD-67 M6 T2: in Browse the rows under the form carry the notice alone, as kit notice
    /// lines, and the keys are the section's [`SettingsSection::key_hint`], the pane's border.
    #[test]
    fn the_notice_line_carries_the_notice_alone() {
        let notice = "\u{6f22}".repeat(10);
        let section = QdrantSection {
            notice: Some(Notice::Error(notice.clone())),
            ..QdrantSection::new()
        };
        let theme = Theme::default();
        let lines = section.notice_lines(100, &theme);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].to_string(), notice);
        assert_eq!(lines[0].style, theme.error);
        assert!(QdrantSection::new().notice_lines(100, &theme).is_empty());
        let keys = section
            .key_hint(Keys::compiled(), &theme)
            .expect("the defaults bind the keys");
        assert_eq!(keys.to_string(), section.hint_text(Keys::compiled()));
    }

    /// MD-A, C5: the guides with their key unbound, and a write in flight in words.
    #[test]
    fn the_qdrant_sentences_without_their_keys() {
        assert_eq!(
            no_url_yet(Some("Enter")),
            "no Qdrant URL is set; type one and press Enter"
        );
        assert_eq!(no_url_yet(None), "no Qdrant URL is set; type one to set it");
        assert_eq!(
            replaces_url(None),
            "setting replaces the Qdrant URL for every box"
        );
        assert_eq!(
            nothing_to_clear(Row::Url, None),
            "no Qdrant URL is set, so there is nothing to clear"
        );
        assert_eq!(
            nothing_to_clear(Row::Key, None),
            "no Qdrant API key is stored on this box, so there is nothing to clear"
        );
        let old = OldUrl::Usable(QdrantUrl::parse("http://old.example:6334").expect("valid"));
        assert_eq!(
            offer_text(&old, &UrlRowState::NotSet, None, None),
            "this box's keyring holds http://old.example:6334 from before"
        );
        assert_eq!(
            offer_is_not_edited(None, None, true),
            "the old URL is not edited here"
        );
        for name in crate::qdrant_settings_info::REQUEST_NAMES {
            assert_ne!(doing(name), doing("unknown"), "`{name}` has its own phrase");
            assert!(!in_flight(name).contains('_'), "{}", in_flight(name));
        }
    }
}
