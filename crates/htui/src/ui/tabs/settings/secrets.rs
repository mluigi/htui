//! The secrets section of the Settings tab: the Infisical URL, the machine identity, a health
//! check, and each project's secret scope (MOD-10 milestone 4, D1, D3–D6, D8; OQ-1..4).
//!
//! It holds **no store handle** (`R-NF-3`): it names two reads ([`StoreRequest::SecretsInfo`] and
//! the scope's [`StoreRequest::SecretsTree`]), is handed the [`SecretsSnapshot`] and the tree that
//! come back, and every write and check leaves through `ctx.request`. What is on screen is always
//! the last snapshot the worker assembled.
//!
//! **Nothing recoverable is ever drawn or printed.** The client secret is typed into a
//! [`TextField::masked`] field (one `\u{2022}` per grapheme and a count), read exactly once by
//! [`TextField::take`] into a zeroizing buffer, and leaves as a [`Redacted`] inside an
//! [`IdentityEntry`]. No `Debug` here prints a field's text or a notice's sentence
//! (`settings/connection.rs` B-8): every form and notice prints its lengths.
//!
//! **A refused URL or scope emits nothing** (D3, D6): both are validated here, on the UI task, and
//! a refusal is a sentence under the form. Normalisation's and [`SecretScope::new`]'s sentences
//! name the reason or the field, never the value.
//!
//! Every write answers under its own name: the scope write [`StoreReply::SecretScopeWritten`]
//! (blueprint A-1), a keyring write [`StoreReply::SecretsWritten`] (R1 M-1). This section lands
//! its write on that alone, and takes a read's `Secrets`, a `Hierarchy` or its own `SecretsTree`
//! only as fresh rows; its tree read is its own (R1 L-3), so it never reaches the Hierarchy
//! section as the answer to that section's write. `--demo` (D10) shows `n/a`, refuses keyring
//! edits and checks here (no request is sent), and still edits scopes, which live in the store.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use htui_core::model::{Project, ProjectId, Scope};
use htui_core::secret::{INFISICAL, ProviderHealth, SecretError, SecretScope, project_scope};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use zeroize::Zeroizing;

use crate::app::{Action, Ctx, Handled};
use crate::hierarchy::{HierarchySnapshot, ProjectEntry, ScopeWrite};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Keys, Stack, views};
use crate::secrets_settings::{
    CHECK_SECRET_PROVIDER, CHECK_SECRET_SCOPE, IdentityEntry, IdentityState, READ_NAME,
    REQUEST_NAMES, Redacted, SET_PROJECT_SECRET_SCOPE, SecretCheck, SecretsSnapshot, UrlState,
};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::cells::cell_width;
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    wrapped,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::KeyEvent;

/// What a keyring row says before the first reply.
const NOT_READ: &str = "not read yet";
/// What a keyring row says on `Backend::Memory` (D10): not "none", which would be a claim about a
/// keyring this session never opened.
const DEMO_ROW: &str = "n/a in a demo session";
/// A keyring row with nothing behind it.
const NOT_STORED: &str = "not stored";
/// A keyring row over a keyring that could not be opened; the seam's sentence follows it. Never
/// shown as [`NOT_STORED`].
const UNREADABLE: &str = "the keyring could not be read";
/// The Health row before the session's first check.
const NOT_CHECKED: &str = "not checked this session";
/// A check in flight.
const CHECKING: &str = "checking\u{2026}";
/// A project whose `secret_provider` is unset.
const NO_PROVIDER: &str = "no secret provider";
/// The `Projects` line with no tree to list.
const NO_WORKSPACE: &str = "no workspace: project scopes need one";
/// What the rows say when the keyring read itself was refused; the seam's sentence follows it.
const UNAVAILABLE: &str = "secret settings are unavailable";
/// `SetInfisicalUrl` landed.
const URL_STORED: &str = "stored; the next walk, chat or check uses it";
/// `ClearInfisicalUrl` landed.
const URL_CLEARED: &str = "the Infisical URL is gone from the keyring";
/// `SetMachineIdentity` landed.
const IDENTITY_STORED: &str = "identity stored; the next walk, chat or check logs in with it";
/// `ClearMachineIdentity` landed.
const IDENTITY_CLEARED: &str = "the machine identity is gone from the keyring";
/// A scope write landed.
const SCOPE_SAVED: &str = "scope saved; the next walk or chat on this project uses it";
/// A scope clear landed.
const SCOPE_CLEARED: &str = "scope cleared; walks and chats on this project get no secrets";
/// `Enter` over an empty URL field is not a write: clearing is `c`.
const EMPTY_URL: &str = "nothing typed; the stored URL is unchanged";
/// An identity with a blank half: a blank half reads as absent, so it would store half an
/// identity (D4).
const IDENTITY_BLANK: &str = "both the client ID and the client secret are required";
/// `e` or `c` on the Provider row.
const PROVIDER_FIXED: &str = "infisical is the only provider this build knows";
/// `e` or `c` on the Health row.
const NOTHING_TO_EDIT: &str = "nothing to edit on this row; t checks the provider";
/// `t` on a project with no usable scope: nothing is sent.
const NO_SCOPE_TO_CHECK: &str = "this project has no secret scope to check";
/// `c` on a project with no provider column.
const NO_SCOPE_TO_CLEAR: &str = "this project has no secret scope to clear";
/// One check at a time.
const CHECK_IN_FLIGHT: &str = "a check is still running";
/// `e` on a keyring row in `--demo` (D10).
const DEMO_KEYRING: &str = "a demo session never reads or writes the keyring";
/// `t` in `--demo` (D10, blueprint A-5): the process has no source, so nothing is sent.
const DEMO_CHECK: &str = "a demo session has no secret provider to check";
/// The guide under the rows while the cursor is on Health: a check is a login, and can latch.
const HEALTH_GUIDE: &str = "t logs in afresh: a refused login stops walks, chats and checks from logging in again until the identity is entered again";
/// The line under the rows after a keyring write whose write mark the keyring refused (MOD-90 D3,
/// R1 M-1). Not a failure: the write landed, and this process rebuilds its provider.
const MARK_REFUSED: &str = "the keyring refused htui/infisical-write-mark: a running htui worker sees the last write only after a restart if it left the values unchanged (an identity entered again)";
/// The guide under the rows after a latching refusal (M2 D5).
const LATCHED: &str = "the last login was refused: walks, chats and checks are refused until the identity is entered again (e on Identity)";
/// `c` on the URL row.
const CONFIRM_CLEAR_URL: &str = "Remove the Infisical URL from the keyring? Walks and chats on provider projects are refused until one is stored. y / n";
/// `c` on the Identity row.
const CONFIRM_CLEAR_IDENTITY: &str = "Remove the machine identity (both halves) from the keyring? Walks and chats on provider projects are refused until one is stored. y / n";
/// The URL form's guide.
const URL_GUIDE: &str =
    "the base URL, e.g. https://infisical.example.com; it is checked before it is stored";
/// The identity form's guide.
const IDENTITY_GUIDE: &str = "the client secret is never shown; storing replaces both halves";
/// The scope form's guide.
const SCOPE_GUIDE: &str = "Infisical project ID, environment slug and folder path (starts with /)";
/// The URL form's label.
const URL_LABEL: &str = "URL: ";
/// The identity form's labels.
const IDENTITY_LABELS: [&str; 2] = ["client ID: ", "client secret: "];
/// The scope form's labels.
const SCOPE_LABELS: [&str; 3] = ["project ID: ", "environment: ", "path: "];

/// Browse's keys, with something to browse: `e edit · c clear · t check · r reload · j/k rows`.
const HINT_BROWSE: HintSpec = &[
    Hint::One(Act::Edit, "edit"),
    Hint::One(Act::Clear, "clear"),
    Hint::One(Act::SecretsCheck, "check"),
    Hint::One(Act::Reload, "reload"),
    Hint::Pair(Act::ListDown, Act::ListUp, "rows"),
];
/// Browse's keys with nothing read, or the read refused: `r reload`.
const HINT_NO_SNAPSHOT: HintSpec = &[Hint::One(Act::Reload, "reload")];
/// The URL form's keys, the field's own: `Enter store · Esc cancel`.
const HINT_URL: HintSpec = &[Hint::Text("Enter store"), Hint::Text("Esc cancel")];
/// The identity form's keys, and the promise the mask is.
const HINT_IDENTITY: HintSpec = &[
    Hint::One(Act::FormNextField, "next field"),
    Hint::Text("Enter store"),
    Hint::Text("Esc cancel"),
    Hint::Text("the secret is never shown"),
];
/// The scope form's keys: `Tab next field · Enter save · Esc cancel`.
const HINT_SCOPE: HintSpec = &[
    Hint::One(Act::FormNextField, "next field"),
    Hint::Text("Enter save"),
    Hint::Text("Esc cancel"),
];
/// Every question's keys: `y confirm · n/Esc cancel`.
const HINT_CONFIRM: HintSpec = &[
    Hint::One(Act::ConfirmYes, "confirm"),
    Hint::All(Act::ConfirmNo, "cancel"),
];

/// The width the four fixed labels pad to.
const FIXED_LABEL_WIDTH: usize = 8;

/// `c` on a scoped project.
fn confirm_clear_scope(slug: &str) -> String {
    format!("Remove `{slug}`'s secret scope? Its walks and chats then get no secrets. y / n")
}

/// What a second write is told while the first is still out.
fn in_flight(busy: &str) -> String {
    format!("`{busy}` is still in flight")
}

/// One row, top to bottom: the four fixed rows, then one per project of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The provider kind; fixed in this build.
    Provider,
    /// The Infisical base URL.
    Url,
    /// The machine identity, as a state.
    Identity,
    /// The last provider check of this session.
    Health,
    /// A project, by index into `tree.projects`.
    Project(usize),
}

impl Row {
    /// The four fixed rows.
    const FIXED: [Self; 4] = [Self::Provider, Self::Url, Self::Identity, Self::Health];

    /// The label of a fixed row.
    fn label(self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::Url => "URL",
            Self::Identity => "Identity",
            Self::Health => "Health",
            Self::Project(_) => "",
        }
    }
}

/// The write in flight. One at a time (Connection's D5 rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Write {
    /// `SetInfisicalUrl`.
    Url,
    /// `ClearInfisicalUrl`.
    ClearUrl,
    /// `SetMachineIdentity`.
    Identity,
    /// `ClearMachineIdentity`.
    ClearIdentity,
    /// `SetProjectSecretScope`, a set or a clear.
    Scope {
        /// The project written.
        project: ProjectId,
        /// A clear (`scope: None`).
        clear: bool,
    },
}

impl Write {
    /// The request's name, as [`StoreRequest::name`] spells it.
    fn name(self) -> &'static str {
        match self {
            Self::Url => REQUEST_NAMES[1],
            Self::ClearUrl => REQUEST_NAMES[2],
            Self::Identity => REQUEST_NAMES[3],
            Self::ClearIdentity => REQUEST_NAMES[4],
            Self::Scope { .. } => SET_PROJECT_SECRET_SCOPE,
        }
    }
}

/// The check in flight. One at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checking {
    /// `CheckSecretProvider`.
    Provider,
    /// `CheckSecretScope` of this project.
    Scope(ProjectId),
}

/// Where the section is. `Browse` captures nothing.
#[derive(Default)]
enum Mode {
    /// The rows and the cursor.
    #[default]
    Browse,
    /// The URL field: plain, prefilled with the stored normalised URL.
    EditingUrl(TextField),
    /// The identity form: the client ID plain, the client secret masked.
    EditingIdentity {
        /// The client ID.
        client_id: TextField,
        /// The client secret, masked; read once by `take`.
        client_secret: TextField,
        /// `0` or `1`.
        focus: usize,
    },
    /// The scope form, open until its write's own reply.
    EditingScope {
        /// The project.
        project: ProjectId,
        /// Its CAS token.
        expected: DateTime<Utc>,
        /// Project ID, environment, path.
        fields: [TextField; 3],
        /// `0..3`.
        focus: usize,
    },
    /// `c` on the URL row.
    ConfirmClearUrl,
    /// `c` on the Identity row.
    ConfirmClearIdentity,
    /// `c` on a scoped project.
    ConfirmClearScope {
        /// The project.
        project: ProjectId,
        /// Its slug, for the question.
        slug: String,
        /// Its CAS token.
        expected: DateTime<Utc>,
    },
}

/// What the mode is *about*, and how much was typed: never what was typed (B-8).
impl core::fmt::Debug for Mode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Browse => f.write_str("Browse"),
            Self::EditingUrl(field) => f
                .debug_struct("EditingUrl")
                .field("len", &field.len())
                .finish(),
            Self::EditingIdentity {
                client_id,
                client_secret,
                focus,
            } => f
                .debug_struct("EditingIdentity")
                .field("client_id_len", &client_id.len())
                .field("client_secret_len", &client_secret.len())
                .field("focus", focus)
                .finish(),
            Self::EditingScope {
                project,
                expected,
                fields,
                focus,
            } => f
                .debug_struct("EditingScope")
                .field("project", project)
                .field("expected", expected)
                .field("lens", &fields.each_ref().map(TextField::len))
                .field("focus", focus)
                .finish(),
            Self::ConfirmClearUrl => f.write_str("ConfirmClearUrl"),
            Self::ConfirmClearIdentity => f.write_str("ConfirmClearIdentity"),
            Self::ConfirmClearScope {
                project, expected, ..
            } => f
                .debug_struct("ConfirmClearScope")
                .field("project", project)
                .field("expected", expected)
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

/// The kind and the length, never the sentence (Connection's B-8).
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

/// `Settings > Secrets`: the keyring rows, the health check, and the project scopes.
#[derive(Debug, Default)]
pub struct SecretsSection {
    /// The last keyring snapshot, or `None` before the first reply.
    snapshot: Option<SecretsSnapshot>,
    /// `Some(message)` after `Failed { request: "secrets_info" }`.
    unavailable: Option<String>,
    /// The scope's workspace tree: the project rows and their tokens.
    tree: Option<HierarchySnapshot>,
    /// Index into the rows, in [`row_at`](SecretsSection::row_at)'s order.
    cursor: usize,
    /// Browsing, typing, or being asked a question.
    mode: Mode,
    /// The write in flight.
    busy: Option<Write>,
    /// The check in flight.
    checking: Option<Checking>,
    /// The last provider check of this session.
    provider_check: Option<(DateTime<Utc>, Result<ProviderHealth, SecretError>)>,
    /// The keyring-write generation the provider [`provider_check`](Self::provider_check) asked
    /// was built at (R1 L-1).
    check_generation: u64,
    /// The highest keyring-write generation a landed write has answered. A provider built at a
    /// lower one is rebuilt on the next `provider()` (blueprint A-4), so the latch line of a check
    /// that asked it is no longer true. Decided from generations, not reply order (R1 L-1): the
    /// check runs in a spawned task, which can wait out a walk's keyring read and build after a
    /// write sent later, and can answer before an earlier write's read-back does.
    written_generation: u64,
    /// The last landed keyring write's write mark was refused (MOD-90 D3, R1 M-1): a running
    /// `htui worker` misses that write until it restarts if it left the values unchanged, so
    /// [`MARK_REFUSED`] stays under the rows, as [`LATCHED`] does. Only the next landed keyring
    /// write changes it, whose stored mark carries every write before it; a re-read cannot tell
    /// whether another process has seen the write, so it keeps the line.
    mark_refused: bool,
    /// The last scope check per project of this workspace, this session.
    scope_checks: BTreeMap<ProjectId, (DateTime<Utc>, Result<usize, SecretError>)>,
    /// Writes sent this session: numbers each write, so a landing one can tell whether it was
    /// sent before or after the check in flight.
    writes_sent: u64,
    /// [`writes_sent`](Self::writes_sent) when the write in flight was sent.
    busy_seq: u64,
    /// [`writes_sent`](Self::writes_sent) when the check in flight was sent.
    check_seq: u64,
    /// A project's scope write sent after that project's scope check in flight has landed, and
    /// the check is about what it replaced: the loop serves in order and reads the project row
    /// before the check's task starts, so the check read first. Its answer is history: it caches
    /// no count. Not used for the provider check, which reads the keyring in its spawned task
    /// ([`written_generation`](Self::written_generation), R1 L-1).
    check_outdated: bool,
    /// The last outcome.
    notice: Option<Notice>,
}

impl SecretsSection {
    /// Identity of the secrets section.
    pub const ID: SectionId = SectionId("secrets");

    /// A section with nothing read yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many rows: the four fixed, then one per project of the tree.
    fn row_count(&self) -> usize {
        Row::FIXED.len() + self.tree.as_ref().map_or(0, |tree| tree.projects.len())
    }

    /// Row `index` in cursor order; `index < row_count()`.
    fn row_at(&self, index: usize) -> Row {
        match Row::FIXED.get(index) {
            Some(row) => *row,
            None => Row::Project(index - Row::FIXED.len()),
        }
    }

    /// The row under the cursor.
    fn row(&self) -> Row {
        self.row_at(self.cursor.min(self.row_count() - 1))
    }

    /// Moves the cursor one row; no wrap.
    fn move_cursor(&mut self, down: bool) {
        let last = self.row_count() - 1;
        self.cursor = if down {
            self.cursor.saturating_add(1).min(last)
        } else {
            self.cursor.saturating_sub(1)
        };
    }

    /// Puts the cursor back inside the rows after the tree changed.
    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.row_count() - 1);
    }

    /// A project of the tree, by row index.
    fn entry(&self, index: usize) -> Option<&ProjectEntry> {
        self.tree.as_ref().and_then(|tree| tree.projects.get(index))
    }

    /// Whether this session is `--demo`: the keyring read answered `NotApplicable`.
    fn demo(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.url == UrlState::NotApplicable)
    }

    /// Refuses with the write in flight, if there is one.
    fn refused_busy(&mut self) -> bool {
        if let Some(busy) = self.busy {
            self.refuse(in_flight(busy.name()));
            return true;
        }
        false
    }

    /// Whether `e`/`c` on a keyring row is refused now: a write in flight (said), or no rows to
    /// act on (silent, as Connection's `blocked`).
    fn keyring_blocked(&mut self) -> bool {
        self.refused_busy() || self.snapshot.is_none() || self.unavailable.is_some()
    }

    /// Sends one write and remembers it until its reply.
    fn send(&mut self, write: Write, request: StoreRequest, ctx: &Ctx<'_>) {
        self.busy = Some(write);
        self.writes_sent += 1;
        self.busy_seq = self.writes_sent;
        self.notice = None;
        ctx.request(request);
    }

    /// Sends one check and remembers it until its answer.
    fn check(&mut self, checking: Checking, request: StoreRequest, ctx: &Ctx<'_>) {
        self.checking = Some(checking);
        self.check_seq = self.writes_sent;
        self.check_outdated = false;
        self.notice = None;
        ctx.request(request);
    }

    /// `e`.
    fn edit(&mut self) {
        match self.row() {
            Row::Provider => self.refuse(PROVIDER_FIXED.to_owned()),
            Row::Health => self.refuse(NOTHING_TO_EDIT.to_owned()),
            Row::Url | Row::Identity if self.keyring_blocked() => {}
            Row::Url | Row::Identity if self.demo() => self.refuse(DEMO_KEYRING.to_owned()),
            Row::Url => {
                let field = match self.snapshot.as_ref().map(|snapshot| &snapshot.url) {
                    Some(UrlState::Stored(url)) => TextField::with_text(url),
                    _ => TextField::new(),
                };
                self.notice = None;
                self.mode = Mode::EditingUrl(field);
            }
            Row::Identity => {
                self.notice = None;
                self.mode = Mode::EditingIdentity {
                    client_id: TextField::new(),
                    client_secret: TextField::masked(),
                    focus: 0,
                };
            }
            Row::Project(index) => {
                if self.refused_busy() {
                    return;
                }
                let Some(entry) = self.entry(index) else {
                    return;
                };
                let project = &entry.project;
                let fields = match project_scope(project) {
                    Ok(Some(scope)) => [
                        TextField::with_text(scope.project_id()),
                        TextField::with_text(scope.environment()),
                        TextField::with_text(scope.path()),
                    ],
                    _ => [
                        TextField::new(),
                        TextField::new(),
                        TextField::with_text("/"),
                    ],
                };
                let (id, expected) = (project.id, project.updated_at);
                self.notice = None;
                self.mode = Mode::EditingScope {
                    project: id,
                    expected,
                    fields,
                    focus: 0,
                };
            }
        }
    }

    /// `c`.
    fn clear(&mut self) {
        match self.row() {
            Row::Provider => self.refuse(PROVIDER_FIXED.to_owned()),
            Row::Health => self.refuse(NOTHING_TO_EDIT.to_owned()),
            Row::Url | Row::Identity if self.keyring_blocked() => {}
            Row::Url => {
                let offered = matches!(
                    self.snapshot.as_ref().map(|snapshot| &snapshot.url),
                    Some(UrlState::Stored(_) | UrlState::Unusable(_))
                );
                if offered {
                    self.notice = None;
                    self.mode = Mode::ConfirmClearUrl;
                } else {
                    // Nothing stored, no keyring at all, or one that has not answered: a delete
                    // there would be a guess. The row's own words say which.
                    self.refuse(self.row_text(Row::Url));
                }
            }
            Row::Identity => {
                // A half or an unreadable identity is offered: clearing both halves is the fix.
                let offered = matches!(
                    self.snapshot.as_ref().map(|snapshot| &snapshot.identity),
                    Some(
                        IdentityState::Stored
                            | IdentityState::HalfStored(_)
                            | IdentityState::Unreadable(_)
                    )
                );
                if offered {
                    self.notice = None;
                    self.mode = Mode::ConfirmClearIdentity;
                } else {
                    self.refuse(self.row_text(Row::Identity));
                }
            }
            Row::Project(index) => {
                if self.refused_busy() {
                    return;
                }
                let Some(entry) = self.entry(index) else {
                    return;
                };
                let project = &entry.project;
                if project.secret_provider.is_some() {
                    let mode = Mode::ConfirmClearScope {
                        project: project.id,
                        slug: project.slug.clone(),
                        expected: project.updated_at,
                    };
                    self.notice = None;
                    self.mode = mode;
                } else {
                    self.refuse(NO_SCOPE_TO_CLEAR.to_owned());
                }
            }
        }
    }

    /// `t`: a provider check on the fixed rows, a scope check on a project.
    fn test(&mut self, ctx: &Ctx<'_>) {
        match self.row() {
            Row::Project(index) => {
                let Some(entry) = self.entry(index) else {
                    return;
                };
                let project = entry.project.id;
                if !matches!(project_scope(&entry.project), Ok(Some(_))) {
                    self.refuse(NO_SCOPE_TO_CHECK.to_owned());
                } else if self.demo() {
                    self.refuse(DEMO_CHECK.to_owned());
                } else if self.checking.is_some() {
                    self.refuse(CHECK_IN_FLIGHT.to_owned());
                } else {
                    self.check(
                        Checking::Scope(project),
                        StoreRequest::CheckSecretScope { project },
                        ctx,
                    );
                }
            }
            Row::Provider | Row::Url | Row::Identity | Row::Health => {
                if self.demo() {
                    self.refuse(DEMO_CHECK.to_owned());
                } else if self.checking.is_some() {
                    self.refuse(CHECK_IN_FLIGHT.to_owned());
                } else {
                    self.check(Checking::Provider, StoreRequest::CheckSecretProvider, ctx);
                }
            }
        }
    }

    /// The stack of the current mode (MOD-67 D3): the only place a mode maps to its keys;
    /// `key_stack`, the key handlers and the hint all read it. The URL form is one field, so it
    /// captures (`Tab` there is swallowed, as the old no-op focus cycle swallowed it).
    fn stack(&self) -> Stack<'static> {
        match &self.mode {
            Mode::Browse => views::SECRETS_BROWSE,
            Mode::EditingUrl(_) => views::CAPTURE,
            Mode::EditingIdentity { .. } | Mode::EditingScope { .. } => views::SECRETS_FORM,
            Mode::ConfirmClearUrl | Mode::ConfirmClearIdentity | Mode::ConfirmClearScope { .. } => {
                views::SECRETS_CONFIRM
            }
        }
    }

    /// One key while a form is open. The focused field answers first; then the mode's stack:
    /// `form.next_field` (`Tab`, and `Down` as a view default) and `form.prev_field` (`BackTab`,
    /// `Up`) move between fields; a chord a modal mode passes (CONTROL, ALT, function keys) goes
    /// to the shell, so `ctrl-c` still quits and `F1` helps; everything else is swallowed.
    fn on_form_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let outcome = match &mut self.mode {
            Mode::EditingUrl(field) => field.on_key(key),
            Mode::EditingIdentity {
                client_id,
                client_secret,
                focus,
            } => {
                if *focus == 0 {
                    client_id.on_key(key)
                } else {
                    client_secret.on_key(key)
                }
            }
            Mode::EditingScope { fields, focus, .. } => fields[*focus].on_key(key),
            _ => return Handled::Pass,
        };
        match outcome {
            FieldOutcome::Consumed => {}
            FieldOutcome::Submit => self.submit(ctx),
            // The form drops with the mode; every buffer is wiped on the way out (D3).
            FieldOutcome::Cancel => {
                self.mode = Mode::Browse;
                self.notice = None;
            }
            FieldOutcome::Pass => {
                let stack = self.stack();
                let chord = KeyChord::from_event(key);
                match ctx.keys().actions(stack, chord).first() {
                    Some(Act::FormNextField) => self.cycle_focus(true),
                    Some(Act::FormPrevField) => self.cycle_focus(false),
                    _ if stack.passes(chord) => return Handled::Pass,
                    _ => {}
                }
            }
        }
        Handled::Consumed
    }

    /// Moves the focus to the next or previous field, wrapping.
    fn cycle_focus(&mut self, forward: bool) {
        let (focus, count) = match &mut self.mode {
            Mode::EditingIdentity { focus, .. } => (focus, 2),
            Mode::EditingScope { focus, .. } => (focus, 3),
            _ => return,
        };
        *focus = if forward {
            (*focus + 1) % count
        } else {
            (*focus + count - 1) % count
        };
    }

    /// `Enter` in a form.
    fn submit(&mut self, ctx: &Ctx<'_>) {
        if self.refused_busy() || self.closed_if_gone() {
            return;
        }
        match self.mode {
            Mode::EditingUrl(_) => self.submit_url(ctx),
            Mode::EditingIdentity { .. } => self.submit_identity(ctx),
            Mode::EditingScope { .. } => self.submit_scope(ctx),
            _ => {}
        }
    }

    /// `Enter` in the URL field: one normalisation, and either a request or a sentence (D3).
    fn submit_url(&mut self, ctx: &Ctx<'_>) {
        let Mode::EditingUrl(field) = &self.mode else {
            return;
        };
        let typed = field.text().unwrap_or("").trim();
        if typed.is_empty() {
            self.mode = Mode::Browse;
            self.notice = Some(Notice::Info(EMPTY_URL.to_owned()));
            return;
        }
        match htui_secrets::normalise_base_url(typed) {
            Ok(normalised) => {
                self.mode = Mode::Browse;
                self.send(Write::Url, StoreRequest::SetInfisicalUrl(normalised), ctx);
            }
            // Normalisation never echoes its input; the field is kept for the fix.
            Err(err) => self.refuse(err.to_string()),
        }
    }

    /// `Enter` in the identity form. The secret is **moved** out of its field into a zeroizing
    /// buffer, so there is one copy of it and it is wiped when this returns (D4).
    fn submit_identity(&mut self, ctx: &Ctx<'_>) {
        let Mode::EditingIdentity {
            client_id,
            client_secret,
            ..
        } = &mut self.mode
        else {
            return;
        };
        let mut id = Zeroizing::new(client_id.text().unwrap_or("").trim().to_owned());
        let raw = Zeroizing::new(client_secret.take());
        let secret = raw.trim();
        if id.is_empty() || secret.is_empty() {
            // `take` emptied the field; a fresh one gets its reservation back.
            *client_secret = TextField::masked();
            self.refuse(IDENTITY_BLANK.to_owned());
            return;
        }
        let entry = IdentityEntry::new(core::mem::take(&mut *id), Redacted::new(secret.to_owned()));
        self.mode = Mode::Browse;
        self.send(
            Write::Identity,
            StoreRequest::SetMachineIdentity(entry),
            ctx,
        );
    }

    /// `Enter` in the scope form: [`SecretScope::new`] here, and the form stays open until the
    /// write's own reply (Hierarchy's rule).
    fn submit_scope(&mut self, ctx: &Ctx<'_>) {
        let Mode::EditingScope {
            project,
            expected,
            fields,
            ..
        } = &self.mode
        else {
            return;
        };
        let [project_id, environment, path] = fields
            .each_ref()
            .map(|field| field.text().unwrap_or("").trim());
        let (id, expected) = (*project, *expected);
        match SecretScope::new(project_id, environment, path) {
            Ok(scope) => self.send(
                Write::Scope {
                    project: id,
                    clear: false,
                },
                StoreRequest::SetProjectSecretScope {
                    id,
                    expected,
                    scope: Some(scope),
                },
                ctx,
            ),
            // Names the field, never the value.
            Err(err) => self.refuse(err.to_string()),
        }
    }

    /// One key while a question is on screen, through `SECRETS_CONFIRM`: `confirm.yes` clears,
    /// `confirm.no` closes. A chord a modal mode passes (CONTROL, ALT, function keys) goes to the
    /// shell, so `alt-y` answers nothing; everything else is swallowed.
    fn on_confirm_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        let stack = views::SECRETS_CONFIRM;
        let chord = KeyChord::from_event(key);
        // The narrowest candidate decides: a global act is the pass rule's.
        match ctx.keys().actions(stack, chord).first() {
            Some(Act::ConfirmYes) if self.closed_if_gone() => {}
            Some(Act::ConfirmYes) => match core::mem::take(&mut self.mode) {
                Mode::ConfirmClearUrl => {
                    self.send(Write::ClearUrl, StoreRequest::ClearInfisicalUrl, ctx);
                }
                Mode::ConfirmClearIdentity => {
                    self.send(
                        Write::ClearIdentity,
                        StoreRequest::ClearMachineIdentity,
                        ctx,
                    );
                }
                Mode::ConfirmClearScope {
                    project, expected, ..
                } => self.send(
                    Write::Scope {
                        project,
                        clear: true,
                    },
                    StoreRequest::SetProjectSecretScope {
                        id: project,
                        expected,
                        scope: None,
                    },
                    ctx,
                ),
                other => self.mode = other,
            },
            Some(Act::ConfirmNo) => self.mode = Mode::Browse,
            _ if stack.passes(chord) => return Handled::Pass,
            _ => {}
        }
        Handled::Consumed
    }

    /// The value column of one row, before wrapping.
    fn row_text(&self, row: Row) -> String {
        match row {
            Row::Provider => INFISICAL.to_owned(),
            Row::Url => match self.snapshot.as_ref().map(|snapshot| &snapshot.url) {
                None => NOT_READ.to_owned(),
                Some(UrlState::NotApplicable) => DEMO_ROW.to_owned(),
                Some(UrlState::NotStored) => NOT_STORED.to_owned(),
                Some(UrlState::Stored(url)) => format!("stored \u{b7} {url}"),
                Some(UrlState::Unusable(why)) => format!("stored \u{2014} not usable: {why}"),
                Some(UrlState::Unreadable(why)) => format!("{UNREADABLE}: {why}"),
            },
            Row::Identity => match self.snapshot.as_ref().map(|snapshot| &snapshot.identity) {
                None => NOT_READ.to_owned(),
                Some(IdentityState::NotApplicable) => DEMO_ROW.to_owned(),
                Some(IdentityState::NotStored) => NOT_STORED.to_owned(),
                Some(IdentityState::Stored) => "stored".to_owned(),
                Some(IdentityState::HalfStored(why)) => why.clone(),
                Some(IdentityState::Unreadable(why)) => format!("{UNREADABLE}: {why}"),
            },
            Row::Health => self.health_text(),
            Row::Project(index) => self
                .entry(index)
                .map(|entry| self.project_text(&entry.project))
                .unwrap_or_default(),
        }
    }

    /// The Health row.
    fn health_text(&self) -> String {
        if self.demo() {
            return DEMO_ROW.to_owned();
        }
        if self.checking == Some(Checking::Provider) {
            return CHECKING.to_owned();
        }
        let Some((at, outcome)) = &self.provider_check else {
            return NOT_CHECKED.to_owned();
        };
        let at = at.format("%H:%M:%S");
        match outcome {
            Ok(ProviderHealth {
                server_ok: true, ..
            }) => format!("last check {at}: server ok \u{b7} login ok"),
            Ok(ProviderHealth {
                server_ok: false, ..
            }) => format!("last check {at}: server status not ok \u{b7} login ok"),
            Err(err) => format!("last check {at}: {err}"),
        }
    }

    /// A project row: its scope, then its last check.
    fn project_text(&self, project: &Project) -> String {
        let scope = match project_scope(project) {
            Ok(None) => NO_PROVIDER.to_owned(),
            Ok(Some(scope)) => format!(
                "{INFISICAL} \u{b7} {} \u{b7} {} \u{b7} {}",
                scope.project_id(),
                scope.environment(),
                scope.path()
            ),
            Err(err) => err.to_string(),
        };
        let check = if self.checking == Some(Checking::Scope(project.id)) {
            format!(" \u{b7} {CHECKING}")
        } else {
            match self.scope_checks.get(&project.id) {
                None => String::new(),
                Some((at, Ok(1))) => {
                    format!(" \u{b7} checked {}: 1 key visible", at.format("%H:%M:%S"))
                }
                Some((at, Ok(n))) => {
                    format!(
                        " \u{b7} checked {}: {n} keys visible",
                        at.format("%H:%M:%S")
                    )
                }
                Some((at, Err(err))) => format!(" \u{b7} checked {}: {err}", at.format("%H:%M:%S")),
            }
        };
        format!("{scope}{check}")
    }

    /// Whether the last provider check latched the shared provider (M2 D5), and no keyring
    /// write that landed has rebuilt it (A-4, R1 L-1).
    fn latched(&self) -> bool {
        self.check_generation >= self.written_generation
            && matches!(
                self.provider_check,
                Some((
                    _,
                    Err(SecretError::BadCredentials
                        | SecretError::IdentityLocked
                        | SecretError::LoginRefusedEarlier)
                ))
            )
    }

    /// Row `index`'s style, by its absolute index in cursor order (H-22): the cursor's is
    /// `theme.selected`.
    fn style_of(&self, index: usize, theme: &Theme) -> Style {
        if index == self.cursor {
            theme.selected
        } else {
            theme.base
        }
    }

    /// The rows: the four fixed ones, the `Projects` line, one per project, then the guide and
    /// [`MARK_REFUSED`] (R1 M-1).
    fn lines(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (index, row) in Row::FIXED.into_iter().enumerate() {
            push_row(
                &mut lines,
                row.label(),
                FIXED_LABEL_WIDTH,
                &self.row_text(row),
                width,
                self.style_of(index, theme),
            );
        }
        lines.push(Line::default());
        self.project_lines(&mut lines, width, theme);
        let room = usize::from(width).saturating_sub(2).max(1);
        let mark = self.mark_refused.then_some(MARK_REFUSED);
        for note in [self.guide(), mark].into_iter().flatten() {
            lines.push(Line::default());
            lines.extend(
                wrapped(note, room)
                    .into_iter()
                    .map(|line| Line::styled(format!("  {line}"), theme.dim)),
            );
        }
        lines
    }

    /// The `Projects` heading and one row per project, or [`NO_WORKSPACE`].
    fn project_lines(&self, lines: &mut Vec<Line<'static>>, width: u16, theme: &Theme) {
        match &self.tree {
            None => lines.push(Line::styled(format!("  {NO_WORKSPACE}"), theme.dim)),
            Some(tree) => {
                lines.push(Line::styled("  Projects".to_owned(), theme.dim));
                let slug_width = tree
                    .projects
                    .iter()
                    .map(|entry| cell_width(&entry.project.slug))
                    .max()
                    .unwrap_or(0);
                for (index, entry) in tree.projects.iter().enumerate() {
                    push_row(
                        lines,
                        &entry.project.slug,
                        slug_width,
                        &self.project_text(&entry.project),
                        width,
                        self.style_of(Row::FIXED.len() + index, theme),
                    );
                }
            }
        }
    }

    /// The guide under the rows: [`LATCHED`], else [`HEALTH_GUIDE`] on Health, else none.
    fn guide(&self) -> Option<&'static str> {
        if self.latched() {
            Some(LATCHED)
        } else if self.row() == Row::Health {
            Some(HEALTH_GUIDE)
        } else {
            None
        }
    }

    /// The pane under the rows: the form, or the question, or nothing.
    fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>> {
        let room = usize::from(width).max(1);
        let (mut lines, guide) = match &self.mode {
            Mode::Browse => return Vec::new(),
            Mode::ConfirmClearUrl => return question(CONFIRM_CLEAR_URL, room, theme),
            Mode::ConfirmClearIdentity => return question(CONFIRM_CLEAR_IDENTITY, room, theme),
            Mode::ConfirmClearScope { slug, .. } => {
                return question(&confirm_clear_scope(slug), room, theme);
            }
            Mode::EditingUrl(field) => (form(&[URL_LABEL], &[field], 0, width, theme), URL_GUIDE),
            Mode::EditingIdentity {
                client_id,
                client_secret,
                focus,
            } => (
                form(
                    &IDENTITY_LABELS,
                    &[client_id, client_secret],
                    *focus,
                    width,
                    theme,
                ),
                IDENTITY_GUIDE,
            ),
            Mode::EditingScope { fields, focus, .. } => (
                form(
                    &SCOPE_LABELS,
                    &[&fields[0], &fields[1], &fields[2]],
                    *focus,
                    width,
                    theme,
                ),
                SCOPE_GUIDE,
            ),
        };
        // Under the form rather than on the hint line: a refusal is about what was just typed.
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

    /// The one line under the pane: the keys this mode binds, then (in Browse) the last outcome,
    /// which wins the line when both do not fit (Connection's MOD-60 rule).
    fn hint(&self, width: u16, theme: &Theme, keys: &Keys) -> Line<'static> {
        let keys = self.hint_text(keys);
        let (Some(notice), Mode::Browse) = (&self.notice, &self.mode) else {
            return Line::styled(keys, theme.dim);
        };
        let style = if notice.is_error() {
            theme.error
        } else {
            theme.dim
        };
        let text = notice.text().to_owned();
        if cell_width(&keys) + cell_width(&text) + 3 > usize::from(width) {
            return Line::styled(text, style);
        }
        Line::from(vec![
            Span::styled(format!("{keys} \u{b7} "), theme.dim),
            Span::styled(text, style),
        ])
    }

    /// The keys half of the hint line, rendered through the mode's stack with the keys in force,
    /// plus a write in flight in Browse.
    fn hint_text(&self, keys: &Keys) -> String {
        let spec = match self.mode {
            Mode::EditingUrl(_) => HINT_URL,
            Mode::EditingIdentity { .. } => HINT_IDENTITY,
            Mode::EditingScope { .. } => HINT_SCOPE,
            Mode::ConfirmClearUrl | Mode::ConfirmClearIdentity | Mode::ConfirmClearScope { .. } => {
                HINT_CONFIRM
            }
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
                format!("{keys} \u{b7} {} in flight", busy.name())
            }
            _ => keys,
        }
    }

    /// Reports an outcome.
    fn say(&mut self, text: &str) {
        self.notice = Some(Notice::Info(text.to_owned()));
    }

    /// Reports a refusal.
    fn refuse(&mut self, text: String) {
        self.notice = Some(Notice::Error(text));
    }

    /// A tree, adopted only when it is the scope's workspace: rows refresh, nothing else moves.
    fn adopt(&mut self, tree: &HierarchySnapshot, ctx: &Ctx<'_>) -> bool {
        if tree.workspace.id != ctx.scope.workspace_id {
            return false;
        }
        self.tree = Some(tree.clone());
        self.clamp_cursor();
        true
    }

    /// A passive tree (a read, or another section's write): rows refresh, and a scope form or
    /// question whose project left the tree is closed (R1 L-4).
    fn adopt_passive(&mut self, tree: &HierarchySnapshot, ctx: &Ctx<'_>) {
        if self.adopt(tree, ctx) {
            self.closed_if_gone();
        }
    }

    /// Closes a scope form or question whose project is no longer a row of the tree: it was
    /// deleted or unlinked elsewhere, so its token can only be refused (R1 L-4). Whether it did.
    fn closed_if_gone(&mut self) -> bool {
        let project = match &self.mode {
            Mode::EditingScope { project, .. } | Mode::ConfirmClearScope { project, .. } => {
                *project
            }
            _ => return false,
        };
        let known = self.tree.as_ref().is_some_and(|tree| {
            tree.projects
                .iter()
                .any(|entry| entry.project.id == project)
        });
        if known {
            return false;
        }
        self.mode = Mode::Browse;
        self.refuse(DELETED_ELSEWHERE.to_owned());
        true
    }

    /// No tree for the scope: no project rows, and a scope form or question on one is closed.
    fn on_tree_gone(&mut self) {
        self.tree = None;
        self.clamp_cursor();
        self.closed_if_gone();
    }

    /// A fresh keyring snapshot from a read: rows only, never a write's answer (R1 M-1).
    fn on_snapshot(&mut self, snapshot: &SecretsSnapshot) {
        self.unavailable = None;
        self.snapshot = Some(snapshot.clone());
    }

    /// A keyring write's own answer (R1 M-1): fresh rows, and what the write did when it is this
    /// section's write in flight. Any landed keyring write rebuilds a provider built before it
    /// (A-4); its `generation` says which (R1 L-1). `mark_stored` sets or drops [`MARK_REFUSED`]
    /// (MOD-90 D3, R1 M-1).
    fn on_keyring_written(
        &mut self,
        request: &str,
        generation: u64,
        mark_stored: bool,
        snapshot: &SecretsSnapshot,
    ) {
        self.on_snapshot(snapshot);
        self.written_generation = self.written_generation.max(generation);
        self.mark_refused = !mark_stored;
        let said = match self.busy.filter(|busy| busy.name() == request) {
            Some(Write::Url) => URL_STORED,
            Some(Write::ClearUrl) => URL_CLEARED,
            Some(Write::Identity) => IDENTITY_STORED,
            Some(Write::ClearIdentity) => IDENTITY_CLEARED,
            Some(Write::Scope { .. }) | None => return,
        };
        self.busy = None;
        self.say(said);
    }

    /// A scope write landed. When it `outdates` the scope check in flight and was not this
    /// section's write sent before that check, the check ran against what the write replaced.
    fn landed(&mut self, ours: bool, outdates: bool) {
        if outdates && !(ours && self.busy_seq <= self.check_seq) {
            self.check_outdated = true;
        }
    }

    /// The scope write's own answer (D6, blueprint A-1).
    fn on_scope_written(
        &mut self,
        project: ProjectId,
        tree: &HierarchySnapshot,
        outcome: ScopeWrite,
        ctx: &Ctx<'_>,
    ) {
        if !self.adopt(tree, ctx) {
            ctx.request(StoreRequest::SecretsTree(ctx.scope.workspace_id));
        }
        if outcome == ScopeWrite::Applied {
            self.scope_checks.remove(&project);
            let ours = matches!(self.busy, Some(Write::Scope { project: p, .. }) if p == project);
            self.landed(ours, self.checking == Some(Checking::Scope(project)));
        }
        let clear = match self.busy {
            Some(Write::Scope { project: p, clear }) if p == project => clear,
            // Not this section's write in flight (or it went with a scope change).
            _ => return,
        };
        self.busy = None;
        match outcome {
            ScopeWrite::Applied => {
                self.mode = Mode::Browse;
                self.say(if clear { SCOPE_CLEARED } else { SCOPE_SAVED });
            }
            ScopeWrite::Stale => self.on_scope_stale(project, tree),
        }
    }

    /// A stale scope write: refresh the open form's token, close it if the project left, or say so.
    fn on_scope_stale(&mut self, project: ProjectId, tree: &HierarchySnapshot) {
        let current = tree
            .projects
            .iter()
            .find(|entry| entry.project.id == project)
            .map(|entry| entry.project.updated_at);
        match (&mut self.mode, current) {
            (
                Mode::EditingScope {
                    project: open,
                    expected,
                    ..
                },
                Some(updated_at),
            ) if *open == project => {
                *expected = updated_at;
                self.refuse(CHANGED_ELSEWHERE.to_owned());
            }
            (Mode::EditingScope { project: open, .. }, None) if *open == project => {
                self.mode = Mode::Browse;
                self.refuse(DELETED_ELSEWHERE.to_owned());
            }
            _ => self.refuse(CHANGED_ELSEWHERE_CLOSED.to_owned()),
        }
    }

    /// Ends the check in flight when `check` is it; whether a scope write outdated it.
    fn answered(&mut self, check: Checking) -> bool {
        if self.checking != Some(check) {
            return false;
        }
        self.checking = None;
        core::mem::take(&mut self.check_outdated)
    }

    /// One check's answer.
    fn on_check(&mut self, check: &SecretCheck) {
        match check {
            SecretCheck::Provider {
                at,
                generation,
                outcome,
            } => {
                self.answered(Checking::Provider);
                self.provider_check = Some((*at, outcome.clone()));
                // Whether a landed write has rebuilt what it latched (A-4) is `latched`'s
                // comparison, whichever reply came first (R1 L-1).
                self.check_generation = *generation;
            }
            SecretCheck::Scope {
                project,
                at,
                outcome,
            } => {
                let outdated = self.answered(Checking::Scope(*project));
                let known = self.tree.as_ref().is_some_and(|tree| {
                    tree.projects
                        .iter()
                        .any(|entry| entry.project.id == *project)
                });
                if known && !outdated {
                    self.scope_checks.insert(*project, (*at, outcome.clone()));
                }
            }
        }
    }

    /// A refused request: the keyring read, a write, or a check (blueprint A-1), in that order.
    /// Any other request's refusal is not this section's, and changes nothing.
    fn on_failed(&mut self, request: &'static str, message: &str, ctx: &Ctx<'_>) {
        if request == READ_NAME {
            // Only a real `SecretsInfo` is refused under this name (a write's read-back failure
            // is the write's), so a write in flight stays in flight (R1 M-1).
            self.unavailable = Some(message.to_owned());
        } else if REQUEST_NAMES[1..].contains(&request) || request == SET_PROJECT_SECRET_SCOPE {
            // A refused write. The shell has already put `{request}: {message}` on the status
            // line; a question has nothing left to answer, an open scope form keeps its text. A
            // refused scope write re-reads the tree: a project deleted elsewhere is refused
            // `NotFound` before the CAS write, and the tree without it closes the form (R1 L-4).
            if request == SET_PROJECT_SECRET_SCOPE {
                ctx.request(StoreRequest::SecretsTree(ctx.scope.workspace_id));
            }
            self.busy = None;
            if matches!(
                self.mode,
                Mode::ConfirmClearUrl | Mode::ConfirmClearIdentity | Mode::ConfirmClearScope { .. }
            ) {
                self.mode = Mode::Browse;
            }
            self.refuse(message.to_owned());
        } else if request == CHECK_SECRET_PROVIDER {
            // A refused check is not a check result.
            if self.checking == Some(Checking::Provider) {
                self.checking = None;
            }
            self.refuse(message.to_owned());
        } else if request == CHECK_SECRET_SCOPE {
            if matches!(self.checking, Some(Checking::Scope(_))) {
                self.checking = None;
            }
            self.refuse(message.to_owned());
        }
    }
}

impl SettingsSection for SecretsSection {
    fn id(&self) -> SectionId {
        Self::ID
    }

    fn title(&self) -> &str {
        "Secrets"
    }

    fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest> {
        // The tree under this section's own name (R1 L-3): a plain `Hierarchy` read answered
        // while a Hierarchy write is in flight would be taken for that write's answer.
        vec![
            StoreRequest::SecretsInfo,
            StoreRequest::SecretsTree(scope.workspace_id),
        ]
    }

    fn on_scope_change(&mut self, _scope: &Scope) {
        // The keyring rows and the provider check survive: neither is scoped. A half-typed form
        // does not: a scope change is one of the three disposal points (D3), beside `Esc` and
        // the `take` at submit.
        self.tree = None;
        self.scope_checks.clear();
        self.mode = Mode::Browse;
        if matches!(self.busy, Some(Write::Scope { .. })) {
            self.busy = None;
        }
        if matches!(self.checking, Some(Checking::Scope(_))) {
            self.checking = None;
        }
        self.cursor = 0;
        self.notice = None;
    }

    fn captures_input(&self) -> bool {
        !matches!(self.mode, Mode::Browse)
    }

    /// The current mode's stack (MOD-67 D4).
    fn key_stack(&self) -> Option<Stack<'static>> {
        Some(self.stack())
    }

    /// A bracketed paste into the focused field; a masked one takes it whole or refuses by name.
    fn on_paste(&mut self, text: &str, ctx: &mut Ctx<'_>) -> Handled {
        let field = match &mut self.mode {
            Mode::EditingUrl(field) => field,
            Mode::EditingIdentity {
                client_id,
                client_secret,
                focus,
            } => {
                if *focus == 0 {
                    client_id
                } else {
                    client_secret
                }
            }
            Mode::EditingScope { fields, focus, .. } => &mut fields[*focus],
            _ => return Handled::Pass,
        };
        if !field.on_paste(text) {
            ctx.emit(Action::Error(
                crate::ui::text_field::PASTE_DOES_NOT_FIT.to_owned(),
            ));
        }
        Handled::Consumed
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        match self.mode {
            Mode::EditingUrl(_) | Mode::EditingIdentity { .. } | Mode::EditingScope { .. } => {
                return self.on_form_key(key, ctx);
            }
            Mode::ConfirmClearUrl | Mode::ConfirmClearIdentity | Mode::ConfirmClearScope { .. } => {
                return self.on_confirm_key(key, ctx);
            }
            Mode::Browse => {}
        }
        // Browse, through `SECRETS_BROWSE`. Chord equality includes modifiers: `ctrl-t` is not
        // `t` (a provider check is a login that can latch), `ctrl-e` is not `e`.
        let chord = KeyChord::from_event(key);
        for act in ctx.keys().actions(views::SECRETS_BROWSE, chord) {
            match act {
                Act::Edit => self.edit(),
                Act::Clear => self.clear(),
                Act::SecretsCheck => self.test(ctx),
                Act::ListDown => self.move_cursor(true),
                Act::ListUp => self.move_cursor(false),
                // Never refused: re-reading is how a section that lost a reply recovers.
                Act::Reload => {
                    ctx.request(StoreRequest::SecretsInfo);
                    ctx.request(StoreRequest::SecretsTree(ctx.scope.workspace_id));
                }
                // Declined with no notice: `Esc` is then the next candidate's, or the shell's.
                Act::Dismiss if self.notice.is_some() => self.notice = None,
                _ => continue, // a global act, or one this state declines
            }
            return Handled::Consumed;
        }
        Handled::Pass
    }

    fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        match reply {
            StoreReply::Secrets(snapshot) => self.on_snapshot(snapshot),
            StoreReply::SecretsWritten {
                request,
                generation,
                mark_stored,
                snapshot,
            } => {
                self.on_keyring_written(request, *generation, *mark_stored, snapshot);
            }
            // Passive: fresh rows and tokens, never this section's write's answer (H-4).
            StoreReply::Hierarchy(Some(tree))
            | StoreReply::SecretsTree(Some(tree))
            | StoreReply::HierarchyStale(tree)
            | StoreReply::RepoPathsInferred { tree, .. } => self.adopt_passive(tree, ctx),
            StoreReply::Hierarchy(None) | StoreReply::SecretsTree(None) => self.on_tree_gone(),
            StoreReply::SecretScopeWritten {
                project,
                tree,
                outcome,
            } => self.on_scope_written(*project, tree, *outcome, ctx),
            StoreReply::SecretCheck(check) => self.on_check(check),
            StoreReply::Failed { request, message } => self.on_failed(request, message, ctx),
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
            // The refusal wins the rows (Connection's rule): what is on screen would otherwise be
            // a keyring nothing has confirmed since the outage started.
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
        frame.render_widget(
            Paragraph::new(self.hint(area.width, ctx.theme, ctx.keys())),
            hint,
        );
    }
}

/// One labelled row, its value wrapped under its own column.
fn push_row(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    label_width: usize,
    value: &str,
    width: u16,
    style: Style,
) {
    let gutter = label_width + 4;
    let room = usize::from(width).saturating_sub(gutter).max(1);
    let chunks = wrapped(value, room);
    let chunks = if chunks.is_empty() {
        vec![String::new()]
    } else {
        chunks
    };
    for (n, chunk) in chunks.into_iter().enumerate() {
        let label = if n == 0 { label } else { "" };
        let pad = " ".repeat(label_width.saturating_sub(cell_width(label)));
        lines.push(Line::styled(format!("  {label}{pad}  {chunk}"), style));
    }
}

/// A form: one labelled field per line, labels padded to the widest, the focused one drawing the
/// cursor.
fn form(
    labels: &[&str],
    fields: &[&TextField],
    focus: usize,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let label_width = labels
        .iter()
        .map(|label| cell_width(label))
        .max()
        .unwrap_or(0);
    let field_room = usize::from(width).saturating_sub(label_width);
    labels
        .iter()
        .zip(fields)
        .enumerate()
        .map(|(index, (label, field))| {
            let pad = " ".repeat(label_width.saturating_sub(cell_width(label)));
            let mut spans = vec![Span::styled(format!("{label}{pad}"), theme.accent)];
            spans.extend(
                field
                    .line(
                        u16::try_from(field_room).unwrap_or(u16::MAX),
                        index == focus,
                        theme,
                    )
                    .spans,
            );
            Line::from(spans)
        })
        .collect()
}

/// A question, wrapped and in `theme.error`.
fn question(text: &str, room: usize, theme: &Theme) -> Vec<Line<'static>> {
    wrapped(text, room)
        .into_iter()
        .map(|line| Line::styled(line, theme.error))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    /// The bordered section is 98 cells at 100 columns (H-16), each hint rendered through the
    /// stack its mode resolves with the default keys.
    #[test]
    fn every_hint_fits_the_section() {
        let keys = Keys::compiled();
        for (stack, spec) in [
            (views::SECRETS_BROWSE, HINT_BROWSE),
            (views::SECRETS_BROWSE, HINT_NO_SNAPSHOT),
            (views::CAPTURE, HINT_URL),
            (views::SECRETS_FORM, HINT_IDENTITY),
            (views::SECRETS_FORM, HINT_SCOPE),
            (views::SECRETS_CONFIRM, HINT_CONFIRM),
        ] {
            let hint = keys.hint(stack, spec);
            assert!(!hint.is_empty(), "{spec:?}");
            assert!(cell_width(&hint) <= 98, "{hint}");
        }
    }

    /// The default rendering of every hint: unchanged text, but `n / Esc` reads `n/Esc` (D9).
    #[test]
    fn the_hints_render_todays_text_with_the_default_keys() {
        let keys = Keys::compiled();
        let rendered = |stack, spec| keys.hint(stack, spec);
        assert_eq!(
            rendered(views::SECRETS_BROWSE, HINT_BROWSE),
            "e edit \u{b7} c clear \u{b7} t check \u{b7} r reload \u{b7} j/k rows"
        );
        assert_eq!(
            rendered(views::SECRETS_BROWSE, HINT_NO_SNAPSHOT),
            "r reload"
        );
        assert_eq!(
            rendered(views::CAPTURE, HINT_URL),
            "Enter store \u{b7} Esc cancel"
        );
        assert_eq!(
            rendered(views::SECRETS_FORM, HINT_IDENTITY),
            "Tab next field \u{b7} Enter store \u{b7} Esc cancel \u{b7} the secret is never shown"
        );
        assert_eq!(
            rendered(views::SECRETS_FORM, HINT_SCOPE),
            "Tab next field \u{b7} Enter save \u{b7} Esc cancel"
        );
        assert_eq!(
            rendered(views::SECRETS_CONFIRM, HINT_CONFIRM),
            "y confirm \u{b7} n/Esc cancel"
        );
    }

    /// MOD-60: the notice is measured in cells and takes the line alone when both do not fit.
    #[test]
    fn a_wide_notice_takes_the_hint_line_alone() {
        let k = 10;
        let notice = "\u{6f22}".repeat(k);
        let section = SecretsSection {
            notice: Some(Notice::Error(notice.clone())),
            ..SecretsSection::new()
        };
        let keys = section.hint_text(Keys::compiled());
        let width = cell_width(&keys) + k + 3;

        let line = section.hint(
            u16::try_from(width).expect("a hint this narrow fits u16"),
            &Theme::default(),
            Keys::compiled(),
        );

        assert_eq!(line.spans.len(), 1, "{line:?} against {width}");
        assert_eq!(line.spans[0].content, notice);
    }

    /// MOD-90 D3, R1 M-1: a refused write mark is a dim line under the rows, never an error.
    #[test]
    fn a_refused_write_mark_is_a_dim_line_under_the_rows() {
        let theme = Theme::default();
        let section = SecretsSection {
            mark_refused: true,
            ..SecretsSection::new()
        };
        let lines = section.lines(100, &theme);
        let shown: Vec<&Line<'_>> = lines
            .iter()
            .filter(|line| line.to_string().contains("htui/infisical-write-mark"))
            .collect();
        assert_eq!(shown.len(), 1, "{lines:?}");
        assert_eq!(shown[0].style, theme.dim);
        assert!(
            !SecretsSection::new()
                .lines(100, &theme)
                .iter()
                .any(|line| line.to_string().contains("write-mark")),
            "no line without a refused mark"
        );
    }

    /// No mode prints what was typed into it; a notice prints its length.
    #[test]
    fn neither_a_form_nor_a_notice_prints_its_text() {
        let typed = |text: &str, mut field: TextField| {
            for c in text.chars() {
                field.on_key(KeyEvent::from(KeyCode::Char(c)));
            }
            field
        };
        let modes = [
            Mode::EditingUrl(typed("https://u:hunter2@x", TextField::new())),
            Mode::EditingIdentity {
                client_id: typed("cid-typed-1", TextField::new()),
                client_secret: typed("zq7-secret", TextField::masked()),
                focus: 1,
            },
            Mode::EditingScope {
                project: ProjectId::new(),
                expected: Utc::now(),
                fields: [
                    typed("p-typed", TextField::new()),
                    typed("env-typed", TextField::new()),
                    typed("/path-typed", TextField::new()),
                ],
                focus: 0,
            },
        ];
        for mode in modes {
            let printed = format!("{mode:?}");
            for value in [
                "hunter2",
                "cid-typed-1",
                "zq7-secret",
                "p-typed",
                "env-typed",
                "path-typed",
            ] {
                assert!(!printed.contains(value), "{printed}");
            }
        }
        let printed = format!("{:?}", Notice::Error("cid-typed-1".to_owned()));
        assert!(
            !printed.contains("cid-typed-1") && printed.contains("len: 11"),
            "{printed}"
        );
    }

    /// CLEAN-8 #5: the rows, indexed without a `Vec`, are the four fixed ones, then one per
    /// project of the tree; with no tree, the four alone.
    #[tokio::test]
    async fn row_at_walks_the_fixed_rows_then_the_projects() {
        let tree = crate::hierarchy::snapshot(
            &htui_core::store::MemStore::demo(),
            htui_core::fixtures::ids::WORKSPACE_GRAPHICS,
            None,
        )
        .await
        .expect("the demo store reads")
        .expect("the demo store has the graphics workspace");
        let n = tree.projects.len();
        assert!(n > 0, "the graphics workspace has projects");
        let walk = |section: &SecretsSection| {
            (0..section.row_count())
                .map(|index| section.row_at(index))
                .collect::<Vec<_>>()
        };

        let section = SecretsSection {
            tree: Some(tree.clone()),
            ..SecretsSection::new()
        };
        assert_eq!(
            walk(&section),
            Row::FIXED
                .into_iter()
                .chain((0..n).map(Row::Project))
                .collect::<Vec<_>>()
        );

        let section = SecretsSection {
            tree: None,
            ..SecretsSection::new()
        };
        assert_eq!(walk(&section), Row::FIXED);
    }

    /// The `busy` names are the request names, so a refused write is matched by its own name.
    #[test]
    fn the_write_names_are_the_request_names() {
        assert_eq!(
            [
                Write::Url.name(),
                Write::ClearUrl.name(),
                Write::Identity.name(),
                Write::ClearIdentity.name(),
                Write::Scope {
                    project: ProjectId::new(),
                    clear: true,
                }
                .name(),
            ],
            [
                StoreRequest::SetInfisicalUrl(String::new()).name(),
                StoreRequest::ClearInfisicalUrl.name(),
                StoreRequest::SetMachineIdentity(IdentityEntry::new(
                    String::new(),
                    Redacted::new(String::new()),
                ))
                .name(),
                StoreRequest::ClearMachineIdentity.name(),
                StoreRequest::SetProjectSecretScope {
                    id: ProjectId::new(),
                    expected: Utc::now(),
                    scope: None,
                }
                .name(),
            ]
        );
    }
}
