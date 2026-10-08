//! `htui status`, `htui waiting`, and the document `--search-items --json` prints (MOD-92,
//! `R-TUI-11`, `R-ID-2`, `R-ID-6`, `R-STO-1`, `R-NF-2`). Read-only by construction: the session is
//! `PgStore::connect_read_only` (Postgres refuses writes), the cache is never opened, and `box.toml` is
//! read with `identity::load` under `config_root_path` (nothing is created). Every outcome is one
//! document on stdout under `--json` and an exit code: 0 answered, 3 the store (or the index) did not
//! answer, 2 refused, 1 failed. The schema is `docs/htui-cli.md`'s; [`SCHEMA_VERSION`] bumps only on a
//! breaking change.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use htui_core::model::{
    BoxId, ItemId, ProjectId, ProjectRef, RunId, Scope, StepId, WorkspaceId, WorkspaceSummary,
};
use htui_core::store::{ReadStore as _, StoreError, WriteStore as _};
use htui_store::pg::CONNECT_TIMEOUT;
use htui_store::vector::Hit;
use htui_store::{HeadlessError, Identity, PgStore, identity, secret};
use htui_worker::{WaitingReason, WaitingRow, WaitingView};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::{cli, concepts};

/// The schema version every document carries (MOD-92 D4).
pub const SCHEMA_VERSION: u32 = 1;

/// Which read-only verb runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// `htui status`.
    Status,
    /// `htui waiting`.
    Waiting,
}

impl Verb {
    /// The document kind this verb answers with.
    const fn kind(self) -> Kind {
        match self {
            Self::Status => Kind::Status,
            Self::Waiting => Kind::Waiting,
        }
    }
}

/// How a state verb ends; `main` maps it to the exit code and prints it on stderr.
#[derive(Debug)]
pub enum StateExit {
    /// Exit 3: Postgres (or, for a search, Qdrant) did not answer. Never a crash report.
    Offline(String),
    /// Exit 2: refused before any answer (no DSN, keyring, schema, scope, embedder mismatch).
    /// Never a crash report.
    Refused(String),
    /// Exit 1: a read failed after the connect with something other than `Unreachable`, the model
    /// could not load, or stdout failed (not a closed pipe). Reported to Sentry.
    Failed(String),
}

impl StateExit {
    /// 3 offline, 2 refused, 1 failed.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Offline(_) => 3,
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for StateExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Offline(sentence) | Self::Refused(sentence) | Self::Failed(sentence) => {
                f.write_str(sentence)
            }
        }
    }
}

impl std::error::Error for StateExit {}

/// What [`answer`] needs that [`run`] reads from the keyring, `box.toml` and the clock: the test
/// seam.
#[derive(Clone)]
pub struct Context<'a> {
    /// The Postgres DSN.
    pub dsn: &'a str,
    /// `box.toml`'s identity; `None` when there is none (then `"box": null`, and `--cwd` is
    /// refused).
    pub identity: Option<&'a Identity>,
    /// The instant ages are measured against (D-D).
    pub now: DateTime<Utc>,
    /// How long the connect may take.
    pub connect_timeout: Duration,
}

impl core::fmt::Debug for Context<'_> {
    /// Everything but the DSN, which may carry a password.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Context")
            .field("dsn", &"<redacted>")
            .field("identity", &self.identity)
            .field("now", &self.now)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

/// `htui status` / `htui waiting`: keyring DSN (`secret::get_dsn`), `identity::load` over
/// `identity::config_root_path()`, `Utc::now()`, `pg::CONNECT_TIMEOUT`; then [`answer`], then
/// [`emit`] to a locked stdout. A missing DSN or an unreadable keyring or `box.toml` is answered as
/// a refusal document without connecting.
///
/// # Errors
/// [`StateExit`]; its code is the exit status.
pub async fn run(verb: Verb, args: cli::StateArgs) -> Result<(), StateExit> {
    let now = Utc::now();
    let answer = match read_dsn().and_then(|dsn| Ok((dsn, read_identity()?))) {
        Err(failure) => Answer {
            kind: verb.kind(),
            outcome: Outcome::Failed(failure),
        },
        Ok((dsn, identity)) => {
            let ctx = Context {
                dsn: &dsn,
                identity: identity.as_ref(),
                now,
                connect_timeout: CONNECT_TIMEOUT,
            };
            answer(verb, &args, &ctx).await
        }
    };
    emit(&mut std::io::stdout().lock(), &answer, args.json)
}

/// The keyring's DSN, or the refusal a verb answers without one (`R-STO-1`: never argv or env).
pub(crate) fn read_dsn() -> Result<Zeroizing<String>, Failure> {
    match secret::get_dsn() {
        Ok(Some(dsn)) => Ok(Zeroizing::new(dsn)),
        Ok(None) => Err(Failure::refused(ErrorCode::NoDsn, concepts::NO_DSN)),
        Err(err) => Err(Failure::refused(
            ErrorCode::KeyringUnreadable,
            err.to_string(),
        )),
    }
}

/// `box.toml` read and never written (A-4): `None` when there is none, or when this platform has no
/// config directory at all.
pub(crate) fn read_identity() -> Result<Option<Identity>, Failure> {
    let Ok(root) = identity::config_root_path() else {
        return Ok(None);
    };
    identity::load(&root).map_err(|err| Failure::refused(ErrorCode::BoxIdentity, err.to_string()))
}

/// The answer to `verb` over `ctx`. Never fails: a failure is an [`Answer`] too.
pub async fn answer(verb: Verb, args: &cli::StateArgs, ctx: &Context<'_>) -> Answer {
    let outcome = answer_online(verb, args, ctx)
        .await
        .unwrap_or_else(Outcome::Failed);
    Answer {
        kind: verb.kind(),
        outcome,
    }
}

/// [`answer`]'s reads, `serve_waiting`'s (`store_worker.rs`) on a read-only session.
async fn answer_online(
    verb: Verb,
    args: &cli::StateArgs,
    ctx: &Context<'_>,
) -> Result<Outcome, Failure> {
    let pg = PgStore::connect_read_only(
        ctx.dsn,
        ctx.identity.map(|identity| identity.box_id),
        ctx.connect_timeout,
    )
    .await
    .map_err(Failure::from_connect)?;
    let read = |err: StoreError| Failure::from_read(&err);
    let workspaces = pg.workspaces().await.map_err(read)?;
    let target = state_target(&pg, &workspaces, args, ctx.identity).await?;
    let active = pg.active_runs(&target.scope).await.map_err(read)?;
    let candidates = pg.waiting_candidates(&target.scope).await.map_err(read)?;
    let permissions =
        crate::store_worker::permissions_or_unknown(pg.open_permissions(&target.scope).await)
            .map_err(read)?;
    let view = htui_worker::waiting(&target.scope, active, &candidates, permissions.as_deref());
    Ok(match verb {
        Verb::Status => {
            let this_box = match ctx.identity {
                None => None,
                Some(identity) => Some(match pg.box_info().await.map_err(read)? {
                    Some(info) => BoxDoc {
                        id: identity.box_id,
                        hostname: info.hostname,
                        registered: true,
                    },
                    None => BoxDoc {
                        id: identity.box_id,
                        hostname: identity.hostname.clone(),
                        registered: false,
                    },
                }),
            };
            Outcome::Status(status_body(&target, &view, this_box))
        }
        Verb::Waiting => Outcome::Waiting(waiting_body(&target, &view, ctx.now)),
    })
}

/// The scope `status` / `waiting` read: `--project`, `--cwd`, or the first workspace (plan G4).
async fn state_target<'w>(
    pg: &PgStore,
    workspaces: &'w [WorkspaceSummary],
    args: &cli::StateArgs,
    identity: Option<&Identity>,
) -> Result<Target<'w>, Failure> {
    if let Some(slug) = args.project.as_deref() {
        let unknown = || {
            Failure::refused(
                ErrorCode::UnknownProject,
                format!("no project with slug `{slug}`"),
            )
        };
        let project = workspaces
            .iter()
            .flat_map(|ws| &ws.projects)
            .find(|p| p.slug == slug)
            .map(|p| p.project_id)
            .ok_or_else(unknown)?;
        return project_target(workspaces, project).ok_or_else(unknown);
    }
    if let Some(cwd) = args.cwd.as_deref() {
        let project = cwd_project(pg, workspaces, cwd, identity).await?;
        return project_target(workspaces, project).ok_or_else(|| cwd_unmatched(cwd));
    }
    Ok(first_workspace(workspaces))
}

/// `--cwd`: the project whose repository, registered on this box, holds `cwd` (the deepest
/// checkout). Both sides go through `std::fs::canonicalize` on this machine, so symlinks, Windows'
/// verbatim prefixes and its casing compare alike. Shared by `status`, `waiting` and
/// `--search-items`.
pub(crate) async fn cwd_project(
    pg: &PgStore,
    workspaces: &[WorkspaceSummary],
    cwd: &Path,
    identity: Option<&Identity>,
) -> Result<ProjectId, Failure> {
    let Ok(canonical) = std::fs::canonicalize(cwd) else {
        // `R-BOX-4`: the path as typed, never what it resolves to.
        return Err(Failure::refused(
            ErrorCode::CwdUnresolved,
            format!("`{}` cannot be resolved on this box", cwd.display()),
        ));
    };
    let Some(identity) = identity else {
        return Err(Failure::refused(
            ErrorCode::NoBox,
            "no box.toml on this box: start htui once so this box has an identity",
        ));
    };
    let read = |err: StoreError| Failure::from_read(&err);
    let mut repo_project = HashMap::new();
    let mut projects: Vec<ProjectId> = Vec::new();
    for project in workspaces.iter().flat_map(|ws| &ws.projects) {
        if projects.contains(&project.project_id) {
            continue;
        }
        projects.push(project.project_id);
        for repo in pg.repos(project.project_id).await.map_err(read)? {
            repo_project.insert(repo.id, project.project_id);
        }
    }
    let rows = pg
        .repo_paths(identity.box_id)
        .await
        .map_err(read)?
        .into_iter()
        .filter_map(|row| {
            repo_project
                .get(&row.repo_id)
                .map(|project| (*project, row.local_path))
        })
        .collect();
    match project_for_cwd(&canonical, &canonical_checkouts(rows)) {
        CwdMatch::None => Err(cwd_unmatched(cwd)),
        CwdMatch::One(project) => Ok(project),
        CwdMatch::Ambiguous(projects) => {
            let slugs = projects
                .iter()
                .map(|project| {
                    workspaces
                        .iter()
                        .flat_map(|ws| &ws.projects)
                        .find(|p| p.project_id == *project)
                        .map_or_else(|| project.to_string(), |p| format!("`{}`", p.slug))
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(Failure::refused(
                ErrorCode::CwdAmbiguous,
                format!(
                    "`{}` is in a checkout registered for projects {slugs}; name one with \
                     --project",
                    cwd.display()
                ),
            ))
        }
    }
}

/// `--cwd` inside no checkout of this box.
fn cwd_unmatched(cwd: &Path) -> Failure {
    Failure::refused(
        ErrorCode::CwdUnmatched,
        format!(
            "`{}` is in no repository registered on this box",
            cwd.display()
        ),
    )
}

/// Writes `answer`: under `json` one compact document and a newline, else the human text; a reader
/// that closed the pipe is not an error (`write_out`). Then the exit: `Ok` for 0, else the
/// [`StateExit`] carrying the human sentence (`main` prints it on stderr; nothing here prints to
/// stderr but search's "no matches" / "the index is empty" notes).
///
/// # Errors
/// [`StateExit`] for exit 1-3; [`StateExit::Failed`] when stdout fails with anything but
/// `BrokenPipe`.
pub fn emit(out: &mut dyn Write, answer: &Answer, json: bool) -> Result<(), StateExit> {
    let text = if json {
        let mut text = envelope_text(answer, false);
        text.push('\n');
        text
    } else {
        human_text(answer)
    };
    if !text.is_empty() {
        crate::write_out(out, text.as_bytes())
            .map_err(|err| StateExit::Failed(format!("cannot write to stdout: {err}")))?;
    }
    answer.exit()
}

/// The JSON document of `answer` (what [`emit`] writes under `--json`), for snapshots.
#[must_use]
pub fn document(answer: &Answer) -> serde_json::Value {
    // Through the text, so a score reads back as the shortest `f32` text `emit` wrote, and the
    // value equals what a reader of stdout parses.
    serde_json::from_str(&envelope_text(answer, false)).expect("a document is valid JSON")
}

/// `answer`'s document as text, fields in declaration order: `version, kind, store`, then the body.
fn envelope_text(answer: &Answer, pretty: bool) -> String {
    fn text<B: Serialize>(envelope: &Envelope<B>, pretty: bool) -> String {
        let text = if pretty {
            serde_json::to_string_pretty(envelope)
        } else {
            serde_json::to_string(envelope)
        };
        // String keys, no map, no non-finite float that is not written as `null`: cannot fail.
        text.expect("a document always serialises")
    }
    let kind = answer.kind;
    match &answer.outcome {
        Outcome::Status(body) => text(&Envelope::online(kind, body), pretty),
        Outcome::Waiting(body) => text(&Envelope::online(kind, body), pretty),
        Outcome::Search(search) => text(&Envelope::online(kind, search_doc(search)), pretty),
        Outcome::Failed(failure) => text(
            &Envelope {
                version: SCHEMA_VERSION,
                kind,
                store: failure.store,
                body: FailureBody {
                    error: &failure.error,
                },
            },
            pretty,
        ),
    }
}

/// `answer` as a person reads it.
fn human_text(answer: &Answer) -> String {
    match &answer.outcome {
        Outcome::Status(body) => format!("{}\n", human_status(body)),
        Outcome::Waiting(body) => human_waiting(body),
        Outcome::Search(search) => match &search.index {
            IndexAnswer::Hits(hits) if hits.is_empty() => {
                eprintln!("no matches (run `htui --index-items` if the index is empty)");
                String::new()
            }
            IndexAnswer::Hits(hits) => hits
                .iter()
                .map(|hit| format!("{}\n", concepts::format_hit(hit)))
                .collect(),
            IndexAnswer::Empty => {
                eprintln!("the index is empty; run `htui --index-items`");
                String::new()
            }
            IndexAnswer::Offline(_) | IndexAnswer::Refused(_) | IndexAnswer::Failed(_) => {
                String::new()
            }
        },
        Outcome::Failed(_) => String::new(),
    }
}

/// A verb's answer: the document kind and what happened.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// Which document this is.
    pub kind: Kind,
    /// What the verb found, or why it found nothing.
    pub outcome: Outcome,
}

/// What a verb found.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// `htui status`, answered.
    Status(StatusBody),
    /// `htui waiting`, answered.
    Waiting(WaitingBody),
    /// `--search-items`, Postgres answered; the index's state is inside.
    Search(SearchAnswer),
    /// No answer: Postgres did not answer, refused, or a read failed.
    Failed(Failure),
}

impl Answer {
    /// 0, 1, 2 or 3, by D3 and A-2.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match &self.outcome {
            Outcome::Status(_) | Outcome::Waiting(_) => 0,
            Outcome::Failed(failure) => match failure.store {
                StoreState::Offline => 3,
                StoreState::Refused => 2,
                StoreState::Online | StoreState::Error => 1,
            },
            Outcome::Search(search) => match search.index {
                IndexAnswer::Hits(_) | IndexAnswer::Empty => 0,
                IndexAnswer::Offline(_) => 3,
                IndexAnswer::Refused(_) => 2,
                IndexAnswer::Failed(_) => 1,
            },
        }
    }

    /// The exit as [`emit`] returns it.
    fn exit(&self) -> Result<(), StateExit> {
        let message = match &self.outcome {
            Outcome::Failed(failure) => failure.error.message.clone(),
            Outcome::Search(SearchAnswer {
                index:
                    IndexAnswer::Offline(message)
                    | IndexAnswer::Refused(message)
                    | IndexAnswer::Failed(message),
                ..
            }) => message.clone(),
            _ => String::new(),
        };
        match self.exit_code() {
            0 => Ok(()),
            3 => Err(StateExit::Offline(message)),
            2 => Err(StateExit::Refused(message)),
            _ => Err(StateExit::Failed(message)),
        }
    }
}

/// A failure before an answer, with Postgres's state as `store` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// [`StoreState::Offline`], [`StoreState::Refused`] or [`StoreState::Error`]; never online.
    pub store: StoreState,
    /// The code and the sentence.
    pub error: ErrorDoc,
}

impl Failure {
    /// `HeadlessError` -> Failure: `Store(Unreachable)` offline/`unreachable`; `MigrationsPending`
    /// refused/`migrations_pending`; `BelowTarget` refused/`below_target`; `Store(_)` refused/
    /// `store_refused` (A-8). `message` is `format!("{:#}", concepts::headless_refusal(err))`.
    #[must_use]
    pub fn from_connect(err: HeadlessError) -> Self {
        let (store, code) = match &err {
            HeadlessError::Store(StoreError::Unreachable(_)) => {
                (StoreState::Offline, ErrorCode::Unreachable)
            }
            HeadlessError::Store(_) => (StoreState::Refused, ErrorCode::StoreRefused),
            HeadlessError::MigrationsPending(_) => {
                (StoreState::Refused, ErrorCode::MigrationsPending)
            }
            HeadlessError::BelowTarget { .. } => (StoreState::Refused, ErrorCode::BelowTarget),
        };
        Self {
            store,
            error: ErrorDoc {
                code,
                message: format!("{:#}", concepts::headless_refusal(err)),
            },
        }
    }

    /// A read after the connect: `Unreachable` offline/`unreachable`, else error/`read_failed`.
    #[must_use]
    pub fn from_read(err: &StoreError) -> Self {
        match err {
            StoreError::Unreachable(_) => Self {
                store: StoreState::Offline,
                error: ErrorDoc {
                    code: ErrorCode::Unreachable,
                    message: format!("cannot reach Postgres: {err}"),
                },
            },
            _ => Self {
                store: StoreState::Error,
                error: ErrorDoc {
                    code: ErrorCode::ReadFailed,
                    message: format!("reading the store failed: {err}"),
                },
            },
        }
    }

    /// A refusal with `code` and `message` (store `refused`).
    #[must_use]
    pub fn refused(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            store: StoreState::Refused,
            error: ErrorDoc {
                code,
                message: message.into(),
            },
        }
    }
}

/// What `--search-items` found: the query echo, then the index's state.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchAnswer {
    /// The query, as typed.
    pub query: String,
    /// `--decisions`.
    pub decisions: bool,
    /// What the index answered.
    pub index: IndexAnswer,
}

/// The index's answer to a search whose Postgres answered (A-2).
#[derive(Debug, Clone, PartialEq)]
pub enum IndexAnswer {
    /// The collection answered (`online`, exit 0).
    Hits(Vec<Hit>),
    /// There is no collection (`empty`, exit 0).
    Empty,
    /// Qdrant did not answer (`offline`, exit 3); the sentence.
    Offline(String),
    /// Qdrant holds another embedder's vectors (`refused`, exit 2); the sentence.
    Refused(String),
    /// The embedding model could not be fetched or loaded (`error`, exit 1); the sentence.
    Failed(String),
}

/// The document kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `htui status`.
    Status,
    /// `htui waiting`.
    Waiting,
    /// `htui --search-items`.
    Search,
}

/// Postgres's state, as a document reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreState {
    /// It answered.
    Online,
    /// It did not answer (exit 3).
    Offline,
    /// The verb refused before an answer (exit 2).
    Refused,
    /// A read failed after the connect (exit 1).
    Error,
}

/// The concepts index's state, on a search document (A-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    /// The collection answered.
    Online,
    /// There is no collection.
    Empty,
    /// Qdrant did not answer.
    Offline,
    /// Qdrant holds another embedder's vectors.
    Refused,
    /// The embedding model could not load.
    Error,
}

/// `error.code`: why a document carries no answer. Each ends in one exit code
/// (`docs/htui-cli.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Postgres did not answer (3).
    Unreachable,
    /// No DSN in the OS keyring (2).
    NoDsn,
    /// The OS keyring could not be read (2).
    KeyringUnreadable,
    /// `box.toml` exists but cannot be read (2).
    BoxIdentity,
    /// The schema has pending migrations (2).
    MigrationsPending,
    /// This build is below the database's target version (2).
    BelowTarget,
    /// Postgres answered and refused this htui (2, A-8).
    StoreRefused,
    /// `--project` names no project (2).
    UnknownProject,
    /// `--cwd` without a `box.toml` (2, A-9).
    NoBox,
    /// `--cwd` cannot be resolved (2).
    CwdUnresolved,
    /// `--cwd` is in no checkout registered on this box (2).
    CwdUnmatched,
    /// `--cwd`'s checkout is registered for two projects or more (2, A-9).
    CwdAmbiguous,
    /// A read failed after the connect (1).
    ReadFailed,
    /// No Qdrant URL in the OS keyring (2).
    NoQdrantUrl,
    /// The stored Qdrant URL is not usable (2).
    QdrantUrlUnusable,
    /// Qdrant did not answer (3).
    QdrantUnreachable,
    /// The collection holds another embedder's vectors (2).
    EmbedderMismatch,
    /// The embedding model could not be fetched or loaded (1).
    ModelUnavailable,
}

/// A document's `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDoc {
    /// The machine-readable reason.
    pub code: ErrorCode,
    /// The sentence `htui` prints on stderr.
    pub message: String,
}

/// A workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceDoc {
    /// `workspace.id`.
    pub id: WorkspaceId,
    /// `workspace.slug`.
    pub slug: String,
    /// `workspace.name`.
    pub name: String,
}

/// A project of the scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectDoc {
    /// `project.id`.
    pub id: ProjectId,
    /// `project.slug`.
    pub slug: String,
    /// `project.name`.
    pub name: String,
}

/// This box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoxDoc {
    /// `box.toml`'s id.
    pub id: BoxId,
    /// The `box` row's hostname when registered, else this machine's.
    pub hostname: String,
    /// Whether the store holds a `box` row with this id.
    pub registered: bool,
}

/// `htui status`'s body: the top bar's facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusBody {
    /// `null`: the store holds no workspace.
    pub workspace: Option<WorkspaceDoc>,
    /// The scope's projects, in scope order.
    pub projects: Vec<ProjectDoc>,
    /// `null`: no `box.toml` on this machine.
    #[serde(rename = "box")]
    pub this_box: Option<BoxDoc>,
    /// Active runs that own no row.
    pub working: usize,
    /// Rows waiting on a person.
    pub waiting: usize,
    /// Whether the open permission requests were listed.
    pub permissions_known: bool,
}

/// `htui waiting`'s body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingBody {
    /// `null`: the store holds no workspace.
    pub workspace: Option<WorkspaceDoc>,
    /// The scope's projects, in scope order.
    pub projects: Vec<ProjectDoc>,
    /// Active runs that own no row.
    pub working: usize,
    /// Whether the open permission requests were listed.
    pub permissions_known: bool,
    /// The rows, in the overlay's order.
    pub rows: Vec<WaitingRowDoc>,
}

/// Why a row waits: [`WaitingReason`], snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonDoc {
    /// A parked gate.
    Gate,
    /// A fan-out slot whose judge failed.
    JudgeFailed,
    /// A fan-out slot awaiting selection.
    Selection,
    /// A blocked item `u` clears.
    Unblock,
    /// A run parked by an interrupt.
    Interrupted,
    /// An open permission request.
    Permission,
}

impl From<WaitingReason> for ReasonDoc {
    /// Exhaustive: a new reason fails to compile here rather than serialising silently.
    fn from(reason: WaitingReason) -> Self {
        match reason {
            WaitingReason::Gate => Self::Gate,
            WaitingReason::JudgeFailed => Self::JudgeFailed,
            WaitingReason::Selection => Self::Selection,
            WaitingReason::Unblock => Self::Unblock,
            WaitingReason::Interrupted => Self::Interrupted,
            WaitingReason::Permission => Self::Permission,
        }
    }
}

impl ReasonDoc {
    /// The overlay's reason column ([`WaitingReason::label`]).
    const fn label(self) -> &'static str {
        match self {
            Self::Gate => WaitingReason::Gate.label(),
            Self::JudgeFailed => WaitingReason::JudgeFailed.label(),
            Self::Selection => WaitingReason::Selection.label(),
            Self::Unblock => WaitingReason::Unblock.label(),
            Self::Interrupted => WaitingReason::Interrupted.label(),
            Self::Permission => WaitingReason::Permission.label(),
        }
    }
}

/// One waiting row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingRowDoc {
    /// Why it waits.
    pub reason: ReasonDoc,
    /// `item.key`.
    pub item_key: String,
    /// `item.id`.
    pub item_id: ItemId,
    /// The run; `null` for a Reopen row.
    pub run_id: Option<RunId>,
    /// The step; `null` when the row focuses a run or nothing.
    pub step_id: Option<StepId>,
    /// The step as the Runs pane reads it (`prd 0.1`); `null` when there is none.
    pub step: Option<String>,
    /// The reason's text.
    pub text: String,
    /// When the row began waiting, RFC 3339 UTC, whole seconds (A-11).
    pub since: String,
    /// Seconds since `since` on this process's clock, floored at 0 (D-D).
    pub age_seconds: u64,
}

/// `--search-items`' body.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchDoc {
    /// The index's state.
    pub index: IndexState,
    /// The query, as typed.
    pub query: String,
    /// `--decisions`.
    pub decisions: bool,
    /// `Some` for `online` and `empty` only (A-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hits: Option<Vec<HitDoc>>,
    /// `Some` for `offline`, `refused` and `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDoc>,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HitDoc {
    /// The item's or requirement's key.
    pub key: String,
    /// `item`, `document` or `requirement`.
    pub place: &'static str,
    /// The document's kind, on document hits.
    pub document_kind: Option<String>,
    /// A closed item's resolution.
    pub resolution: Option<&'static str>,
    /// A requirement's state.
    pub requirement_state: Option<&'static str>,
    /// Fused rank score, comparable within one search.
    pub score: f32,
    /// The start of the indexed text.
    pub snippet: String,
}

impl From<&Hit> for HitDoc {
    fn from(hit: &Hit) -> Self {
        Self {
            key: hit.key.clone(),
            place: hit.point_type.as_str(),
            document_kind: hit.document.as_ref().map(|(_, kind)| kind.clone()),
            resolution: hit.resolution.map(|resolution| resolution.as_str()),
            requirement_state: hit.state.map(|state| state.as_str()),
            score: hit.score,
            snippet: hit.snippet.clone(),
        }
    }
}

/// `version, kind, store`, then the body's fields (D4).
#[derive(Debug, Serialize)]
struct Envelope<B> {
    version: u32,
    kind: Kind,
    store: StoreState,
    #[serde(flatten)]
    body: B,
}

impl<B> Envelope<B> {
    const fn online(kind: Kind, body: B) -> Self {
        Self {
            version: SCHEMA_VERSION,
            kind,
            store: StoreState::Online,
            body,
        }
    }
}

/// A failure document's body.
#[derive(Debug, Serialize)]
struct FailureBody<'a> {
    error: &'a ErrorDoc,
}

/// The search document's body for `search`.
fn search_doc(search: &SearchAnswer) -> SearchDoc {
    let failed = |code, message: &String| {
        Some(ErrorDoc {
            code,
            message: message.clone(),
        })
    };
    let (index, hits, error) = match &search.index {
        IndexAnswer::Hits(hits) => (
            IndexState::Online,
            Some(hits.iter().map(HitDoc::from).collect()),
            None,
        ),
        IndexAnswer::Empty => (IndexState::Empty, Some(Vec::new()), None),
        IndexAnswer::Offline(message) => (
            IndexState::Offline,
            None,
            failed(ErrorCode::QdrantUnreachable, message),
        ),
        IndexAnswer::Refused(message) => (
            IndexState::Refused,
            None,
            failed(ErrorCode::EmbedderMismatch, message),
        ),
        IndexAnswer::Failed(message) => (
            IndexState::Error,
            None,
            failed(ErrorCode::ModelUnavailable, message),
        ),
    };
    SearchDoc {
        index,
        query: search.query.clone(),
        decisions: search.decisions,
        hits,
        error,
    }
}

/// RFC 3339 UTC, whole seconds, always `Z` (A-11).
fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `now - since` in whole seconds, floored at 0 (D-D).
fn age_seconds(now: DateTime<Utc>, since: DateTime<Utc>) -> u64 {
    u64::try_from((now - since).num_seconds()).unwrap_or(0)
}

/// An age as the human list prints it: `42s`, `5m`, `3h`, `2d`.
fn age_text(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

/// What a verb reads: the workspace it reports, its projects, and the scope of the reads.
#[derive(Debug)]
struct Target<'a> {
    workspace: Option<&'a WorkspaceSummary>,
    projects: Vec<&'a ProjectRef>,
    scope: Scope,
}

/// The first workspace, as the TUI scopes at start (plan G4); an empty scope when there is none.
fn first_workspace(workspaces: &[WorkspaceSummary]) -> Target<'_> {
    let Some(ws) = workspaces.first() else {
        return Target {
            workspace: None,
            projects: Vec::new(),
            scope: Scope {
                workspace_id: WorkspaceId::default(),
                project_ids: Vec::new(),
            },
        };
    };
    let scope = Scope::from_workspace(ws);
    let projects = scope
        .project_ids
        .iter()
        .filter_map(|id| ws.projects.iter().find(|p| p.project_id == *id))
        .collect();
    Target {
        workspace: Some(ws),
        projects,
        scope,
    }
}

/// `project` in the first workspace, in `workspaces()` order, that lists it (plan G7).
fn project_target(workspaces: &[WorkspaceSummary], project: ProjectId) -> Option<Target<'_>> {
    workspaces.iter().find_map(|ws| {
        ws.projects
            .iter()
            .find(|p| p.project_id == project)
            .map(|p| Target {
                workspace: Some(ws),
                projects: vec![p],
                scope: Scope {
                    workspace_id: ws.workspace_id,
                    project_ids: vec![project],
                },
            })
    })
}

/// Which project `--cwd` names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CwdMatch {
    /// No checkout holds it.
    None,
    /// The deepest checkout holding it belongs to this project.
    One(ProjectId),
    /// The deepest checkout holding it is registered for these projects (A-9).
    Ambiguous(Vec<ProjectId>),
}

/// The deepest checkout holding `cwd`, component-wise (`/src/htui2` is not inside `/src/htui`).
/// Pure; both sides already canonical.
fn project_for_cwd(cwd: &Path, checkouts: &[(ProjectId, PathBuf)]) -> CwdMatch {
    let mut deepest = 0;
    let mut found: Vec<ProjectId> = Vec::new();
    for (project, checkout) in checkouts {
        let depth = checkout.components().count();
        if depth == 0 || !cwd.starts_with(checkout) {
            continue;
        }
        match depth.cmp(&deepest) {
            core::cmp::Ordering::Less => continue,
            core::cmp::Ordering::Greater => {
                deepest = depth;
                found.clear();
            }
            core::cmp::Ordering::Equal => {}
        }
        if !found.contains(project) {
            found.push(*project);
        }
    }
    match found.as_slice() {
        [] => CwdMatch::None,
        [one] => CwdMatch::One(*one),
        _ => CwdMatch::Ambiguous(found),
    }
}

/// The stored checkout paths through `std::fs::canonicalize`; a path that does not resolve (gone
/// from this box) cannot hold an existing `cwd` and is skipped.
fn canonical_checkouts(rows: Vec<(ProjectId, String)>) -> Vec<(ProjectId, PathBuf)> {
    rows.into_iter()
        .filter_map(|(project, path)| std::fs::canonicalize(path).ok().map(|path| (project, path)))
        .collect()
}

fn workspace_doc(ws: &WorkspaceSummary) -> WorkspaceDoc {
    WorkspaceDoc {
        id: ws.workspace_id,
        slug: ws.slug.clone(),
        name: ws.name.clone(),
    }
}

fn project_docs(target: &Target<'_>) -> Vec<ProjectDoc> {
    target
        .projects
        .iter()
        .map(|p| ProjectDoc {
            id: p.project_id,
            slug: p.slug.clone(),
            name: p.name.clone(),
        })
        .collect()
}

fn status_body(target: &Target<'_>, view: &WaitingView, this_box: Option<BoxDoc>) -> StatusBody {
    StatusBody {
        workspace: target.workspace.map(workspace_doc),
        projects: project_docs(target),
        this_box,
        working: view.working,
        waiting: view.waiting(),
        permissions_known: view.permissions_known,
    }
}

fn waiting_body(target: &Target<'_>, view: &WaitingView, now: DateTime<Utc>) -> WaitingBody {
    WaitingBody {
        workspace: target.workspace.map(workspace_doc),
        projects: project_docs(target),
        working: view.working,
        permissions_known: view.permissions_known,
        rows: view.rows.iter().map(|row| row_doc(row, now)).collect(),
    }
}

fn row_doc(row: &WaitingRow, now: DateTime<Utc>) -> WaitingRowDoc {
    WaitingRowDoc {
        reason: row.reason.into(),
        item_key: row.item_key.clone(),
        item_id: row.item,
        run_id: row.run,
        step_id: row.step,
        step: (!row.step_label.is_empty()).then(|| row.step_label.clone()),
        text: row.text.clone(),
        since: rfc3339(row.since),
        age_seconds: age_seconds(now, row.since),
    }
}

/// Stored text stripped of control characters (`format_hit`'s rule): a gate note must not drive the
/// terminal it is printed on.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// The top bar's line (`ui/top_bar.rs`): `workspace · box · online · N working · M waiting`.
fn human_status(body: &StatusBody) -> String {
    let workspace = body
        .workspace
        .as_ref()
        .map_or_else(|| "no workspace".to_owned(), |ws| clean(&ws.name));
    let this_box = body
        .this_box
        .as_ref()
        .map_or_else(|| "no box".to_owned(), |b| clean(&b.hostname));
    format!(
        "{workspace} · {this_box} · online · {} working · {} waiting",
        body.working, body.waiting
    )
}

/// One line per row: key, reason, step, age, text. No rows, no text.
fn human_waiting(body: &WaitingBody) -> String {
    body.rows
        .iter()
        .map(|row| {
            format!(
                "{:<10} {:<12} {:<16} {:>4}  {}\n",
                clean(&row.item_key),
                row.reason.label(),
                clean(row.step.as_deref().unwrap_or_default()),
                age_text(row.age_seconds),
                clean(&row.text)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use htui_core::fixtures::demo_at;
    use htui_core::model::{DocumentId, Resolution};
    use htui_store::vector::{EMBEDDER_MISMATCH, Owner, PointType};
    use uuid::Uuid;

    /// A writer whose every write fails with `kind` (`lib.rs`' `Failing`).
    struct Failing(std::io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(self.0.into())
        }
    }

    const EVERY_REASON: [WaitingReason; 6] = [
        WaitingReason::Gate,
        WaitingReason::JudgeFailed,
        WaitingReason::Selection,
        WaitingReason::Unblock,
        WaitingReason::Interrupted,
        WaitingReason::Permission,
    ];

    fn now() -> DateTime<Utc> {
        demo_at(3, 0)
    }

    fn project(n: u128) -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(n))
    }

    /// Platform, with `agy` before `htui` in the summary but after it by position.
    fn workspace() -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: WorkspaceId::from_uuid(Uuid::from_u128(0x1)),
            slug: "platform".into(),
            name: "Platform".into(),
            projects: vec![
                ProjectRef {
                    project_id: project(0x11),
                    slug: "agy".into(),
                    name: "Antigravity".into(),
                    position: 1,
                },
                ProjectRef {
                    project_id: project(0x10),
                    slug: "htui".into(),
                    name: "htui".into(),
                    position: 0,
                },
            ],
        }
    }

    fn row(
        n: u128,
        key: &str,
        reason: WaitingReason,
        with_run: bool,
        step_label: &str,
        text: &str,
        since: DateTime<Utc>,
    ) -> WaitingRow {
        WaitingRow {
            item: ItemId::from_uuid(Uuid::from_u128(0x100 + n)),
            item_key: key.into(),
            run: with_run.then(|| RunId::from_uuid(Uuid::from_u128(0x200 + n))),
            step: (!step_label.is_empty()).then(|| StepId::from_uuid(Uuid::from_u128(0x300 + n))),
            step_label: step_label.into(),
            reason,
            text: text.into(),
            since,
        }
    }

    /// One row per reason, a Reopen row among them.
    fn six_rows() -> Vec<WaitingRow> {
        vec![
            row(
                1,
                "MOD-4",
                WaitingReason::Gate,
                true,
                "prd 0.1",
                "approve the PRD",
                demo_at(2, 6),
            ),
            row(
                2,
                "MOD-5",
                WaitingReason::JudgeFailed,
                true,
                "research 0.1/j",
                "no candidate passes - pick a candidate",
                demo_at(2, 7),
            ),
            row(
                3,
                "MOD-6",
                WaitingReason::Selection,
                true,
                "research 0.1/1",
                "awaits selection",
                demo_at(2, 5),
            ),
            row(
                4,
                "MOD-7",
                WaitingReason::Unblock,
                false,
                "",
                "blocked, no active run: u reopens it",
                demo_at(1, 0),
            ),
            row(
                5,
                "MOD-8",
                WaitingReason::Interrupted,
                true,
                "build 0.2",
                "interrupted",
                demo_at(2, 5),
            ),
            row(
                6,
                "MOD-9",
                WaitingReason::Permission,
                true,
                "prd 0.1",
                "Bash: cargo test",
                demo_at(2, 23) + chrono::Duration::seconds(3_507),
            ),
        ]
    }

    fn view(rows: Vec<WaitingRow>) -> WaitingView {
        WaitingView {
            working: 1,
            rows,
            permissions_known: true,
            offline: false,
        }
    }

    fn status_answer() -> Answer {
        let workspaces = [workspace()];
        let target = first_workspace(&workspaces);
        let rows = six_rows().into_iter().take(2).collect();
        let this_box = BoxDoc {
            id: BoxId::from_uuid(Uuid::from_u128(0xb0)),
            hostname: "desk".into(),
            registered: true,
        };
        Answer {
            kind: Kind::Status,
            outcome: Outcome::Status(status_body(&target, &view(rows), Some(this_box))),
        }
    }

    fn failed(kind: Kind, failure: Failure) -> Answer {
        Answer {
            kind,
            outcome: Outcome::Failed(failure),
        }
    }

    fn search(index: IndexAnswer) -> Answer {
        Answer {
            kind: Kind::Search,
            outcome: Outcome::Search(SearchAnswer {
                query: "closed items".into(),
                decisions: false,
                index,
            }),
        }
    }

    /// As `concepts.rs`' `tests::hit`.
    fn hit(point_type: PointType, document: Option<&str>) -> Hit {
        Hit {
            point_type,
            owner: Owner::Item(ItemId(Uuid::nil())),
            key: "ANA-11".into(),
            document: document.map(|kind| (DocumentId(Uuid::nil()), kind.to_owned())),
            resolution: None,
            state: None,
            score: 0.5,
            snippet: "a decision is a closed item".into(),
        }
    }

    fn two_hits() -> Vec<Hit> {
        vec![
            Hit {
                score: 0.731,
                ..hit(PointType::Item, None)
            },
            Hit {
                key: "MOD-34".into(),
                resolution: Some(Resolution::Done),
                snippet: "the concepts index".into(),
                ..hit(PointType::Document, Some("prd"))
            },
        ]
    }

    fn pretty(answer: &Answer) -> String {
        envelope_text(answer, true)
    }

    fn unreachable() -> HeadlessError {
        HeadlessError::Store(StoreError::Unreachable("connection refused".into()))
    }

    #[test]
    fn status_document_pins_every_field() {
        insta::assert_snapshot!("status_online", pretty(&status_answer()));
    }

    #[test]
    fn status_document_without_workspace_or_box() {
        let target = first_workspace(&[]);
        let body = status_body(&target, &WaitingView::default(), None);
        let answer = Answer {
            kind: Kind::Status,
            outcome: Outcome::Status(body),
        };
        insta::assert_snapshot!("status_no_workspace_no_box", pretty(&answer));
    }

    #[test]
    fn waiting_document_has_one_row_per_reason() {
        let workspaces = [workspace()];
        let target = first_workspace(&workspaces);
        let answer = Answer {
            kind: Kind::Waiting,
            outcome: Outcome::Waiting(waiting_body(&target, &view(six_rows()), now())),
        };
        insta::assert_snapshot!("waiting_online", pretty(&answer));
    }

    #[test]
    fn reason_doc_covers_every_reason() {
        for reason in EVERY_REASON {
            assert_eq!(
                serde_json::to_value(ReasonDoc::from(reason)).expect("serialises"),
                serde_json::Value::String(reason.label().replace(' ', "_")),
                "{reason:?}"
            );
            assert_eq!(ReasonDoc::from(reason).label(), reason.label());
        }
    }

    #[test]
    fn age_is_floored_at_zero() {
        let since = demo_at(2, 0);
        assert_eq!(age_seconds(since, since + chrono::Duration::seconds(5)), 0);
        assert_eq!(
            age_seconds(since + chrono::Duration::seconds(93), since),
            93
        );
        assert_eq!(
            rfc3339(since + chrono::Duration::milliseconds(1_500)).len(),
            20
        );
        assert!(rfc3339(since).ends_with('Z'), "{}", rfc3339(since));
    }

    #[test]
    fn age_text_names_the_largest_unit() {
        assert_eq!(age_text(42), "42s");
        assert_eq!(age_text(5 * 60 + 59), "5m");
        assert_eq!(age_text(3 * 3_600), "3h");
        assert_eq!(age_text(2 * 86_400 + 1), "2d");
    }

    #[test]
    fn failure_documents_carry_store_and_code() {
        let offline = failed(Kind::Status, Failure::from_connect(unreachable()));
        insta::assert_snapshot!("offline_unreachable", pretty(&offline));
        let no_dsn = failed(
            Kind::Waiting,
            Failure::refused(ErrorCode::NoDsn, concepts::NO_DSN),
        );
        insta::assert_snapshot!("refused_no_dsn", pretty(&no_dsn));
        let read = failed(
            Kind::Status,
            Failure::from_read(&StoreError::Backend("XX000: planted".into())),
        );
        insta::assert_snapshot!("error_read_failed", pretty(&read));
    }

    #[test]
    fn search_documents_by_index_state() {
        insta::assert_snapshot!(
            "search_online",
            pretty(&search(IndexAnswer::Hits(two_hits())))
        );
        insta::assert_snapshot!("search_index_empty", pretty(&search(IndexAnswer::Empty)));
        insta::assert_snapshot!(
            "search_index_offline",
            pretty(&search(IndexAnswer::Offline(
                "cannot reach Qdrant: qdrant: collection exists: transport error".into()
            )))
        );
        insta::assert_snapshot!(
            "search_index_refused",
            pretty(&search(IndexAnswer::Refused(format!(
                "{EMBEDDER_MISMATCH}: collection `htui_concepts` holds vectors from hash/384"
            ))))
        );
    }

    #[test]
    fn exit_codes_follow_d3() {
        assert_eq!(status_answer().exit_code(), 0);
        assert_eq!(
            failed(Kind::Status, Failure::from_connect(unreachable())).exit_code(),
            3
        );
        let refused = Failure::refused(ErrorCode::UnknownProject, "no project");
        assert_eq!(failed(Kind::Waiting, refused).exit_code(), 2);
        let error = Failure::from_read(&StoreError::Backend("boom".into()));
        assert_eq!(failed(Kind::Status, error).exit_code(), 1);
        assert_eq!(search(IndexAnswer::Hits(two_hits())).exit_code(), 0);
        assert_eq!(search(IndexAnswer::Empty).exit_code(), 0);
        assert_eq!(search(IndexAnswer::Offline("o".into())).exit_code(), 3);
        assert_eq!(search(IndexAnswer::Refused("r".into())).exit_code(), 2);
        assert_eq!(search(IndexAnswer::Failed("f".into())).exit_code(), 1);
        assert_eq!(StateExit::Offline(String::new()).code(), 3);
        assert_eq!(StateExit::Refused(String::new()).code(), 2);
        assert_eq!(StateExit::Failed(String::new()).code(), 1);
    }

    /// A-8: only an unreachable server is offline; every other connect refusal is refused/2, with
    /// `headless_refusal`'s sentence.
    #[test]
    fn connect_failures_map_to_codes() {
        type Case = (fn() -> HeadlessError, StoreState, ErrorCode);
        let cases: [Case; 5] = [
            (unreachable, StoreState::Offline, ErrorCode::Unreachable),
            (
                || HeadlessError::Store(StoreError::Backend("28P01: password failed".into())),
                StoreState::Refused,
                ErrorCode::StoreRefused,
            ),
            (
                || HeadlessError::Store(StoreError::Backend("the schema is newer".into())),
                StoreState::Refused,
                ErrorCode::StoreRefused,
            ),
            (
                || HeadlessError::MigrationsPending(3),
                StoreState::Refused,
                ErrorCode::MigrationsPending,
            ),
            (
                || HeadlessError::BelowTarget {
                    ours: "0.1.0".into(),
                    target: "999.0.0".into(),
                },
                StoreState::Refused,
                ErrorCode::BelowTarget,
            ),
        ];
        for (err, store, code) in cases {
            let failure = Failure::from_connect(err());
            assert_eq!(failure.store, store, "{failure:?}");
            assert_eq!(failure.error.code, code, "{failure:?}");
            assert_eq!(
                failure.error.message,
                format!("{:#}", concepts::headless_refusal(err()))
            );
        }
        assert!(
            Failure::from_connect(unreachable())
                .error
                .message
                .starts_with("cannot reach Postgres")
        );
    }

    #[test]
    fn emit_writes_one_parsable_line_and_survives_a_closed_pipe() {
        for answer in [status_answer(), search(IndexAnswer::Hits(two_hits()))] {
            let mut out = Vec::new();
            emit(&mut out, &answer, true).expect("exit 0");
            let text = String::from_utf8(out).expect("UTF-8");
            assert!(text.ends_with('\n'), "{text}");
            assert_eq!(text.matches('\n').count(), 1, "{text}");
            let parsed: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            assert_eq!(parsed, document(&answer));
        }
        let broken = || Failing(std::io::ErrorKind::BrokenPipe);
        emit(&mut broken(), &status_answer(), true).expect("a closed pipe is not an error");
        emit(&mut broken(), &status_answer(), false).expect("nor in human mode");
        let offline = failed(Kind::Status, Failure::from_connect(unreachable()));
        assert!(matches!(
            emit(&mut broken(), &offline, true),
            Err(StateExit::Offline(_))
        ));
        assert!(matches!(
            emit(
                &mut Failing(std::io::ErrorKind::Other),
                &status_answer(),
                true
            ),
            Err(StateExit::Failed(_))
        ));
        // Human mode, a failure: nothing on stdout, the sentence in the exit.
        let mut out = Vec::new();
        let Err(StateExit::Offline(sentence)) = emit(&mut out, &offline, false) else {
            panic!("offline is exit 3");
        };
        assert!(out.is_empty(), "{out:?}");
        assert!(sentence.starts_with("cannot reach Postgres"), "{sentence}");
    }

    #[test]
    fn human_status_is_the_top_bar_line() {
        let Outcome::Status(body) = status_answer().outcome else {
            unreachable!("a status answer");
        };
        insta::assert_snapshot!("status_line", human_status(&body));
        let empty = status_body(&first_workspace(&[]), &WaitingView::default(), None);
        assert_eq!(
            human_status(&empty),
            "no workspace · no box · online · 0 working · 0 waiting"
        );
    }

    #[test]
    fn human_waiting_strips_control_characters() {
        let workspaces = [workspace()];
        let target = first_workspace(&workspaces);
        let rows = vec![row(
            1,
            "MOD-\u{7}4",
            WaitingReason::Gate,
            true,
            "prd 0.1",
            "approve\u{1b}[2J the PRD",
            demo_at(2, 6),
        )];
        let text = human_waiting(&waiting_body(&target, &view(rows), now()));
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
        assert!(text.starts_with("MOD-4"), "{text}");
        assert!(text.contains("approve[2J the PRD"), "{text}");
        assert!(text.contains(" 18h "), "{text}");
        assert_eq!(
            human_waiting(&waiting_body(&target, &view(Vec::new()), now())),
            "",
            "no rows, no text"
        );
    }

    #[test]
    fn cwd_picks_the_deepest_checkout() {
        let (a, b) = (project(0xa), project(0xb));
        let checkouts = [
            (a, PathBuf::from("/src/mono")),
            (b, PathBuf::from("/src/mono/sub")),
        ];
        let at = |cwd: &str| project_for_cwd(Path::new(cwd), &checkouts);
        assert_eq!(at("/src/mono/sub/deep"), CwdMatch::One(b));
        assert_eq!(at("/src/mono/sub"), CwdMatch::One(b));
        assert_eq!(at("/src/mono/other"), CwdMatch::One(a));
        assert_eq!(at("/src/mono2"), CwdMatch::None);
        assert_eq!(at("/elsewhere"), CwdMatch::None);
    }

    #[test]
    fn cwd_tie_between_projects_is_ambiguous() {
        let (a, b) = (project(0xa), project(0xb));
        let shared = PathBuf::from("/src/shared");
        let both = [(a, shared.clone()), (b, shared.clone())];
        assert_eq!(
            project_for_cwd(Path::new("/src/shared/x"), &both),
            CwdMatch::Ambiguous(vec![a, b])
        );
        let twice = [(a, shared.clone()), (a, shared)];
        assert_eq!(
            project_for_cwd(Path::new("/src/shared/x"), &twice),
            CwdMatch::One(a)
        );
    }

    #[cfg(unix)]
    #[test]
    fn cwd_through_a_symlink_matches_its_target() {
        let dir = tempfile::tempdir().expect("a tempdir");
        let checkout = dir.path().join("checkout");
        std::fs::create_dir_all(checkout.join("sub")).expect("mkdir");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&checkout, &link).expect("symlink");
        let a = project(0xa);
        let checkouts = canonical_checkouts(vec![
            (a, checkout.display().to_string()),
            (project(0xb), dir.path().join("gone").display().to_string()),
        ]);
        assert_eq!(
            checkouts.len(),
            1,
            "a path that does not resolve is skipped"
        );
        let cwd = std::fs::canonicalize(link.join("sub")).expect("resolves");
        assert_eq!(project_for_cwd(&cwd, &checkouts), CwdMatch::One(a));
    }

    #[cfg(windows)]
    #[test]
    fn cwd_windows_verbatim_paths_compare() {
        let dir = tempfile::tempdir().expect("a tempdir");
        let checkout = dir.path().join("Checkout");
        std::fs::create_dir_all(checkout.join("Sub")).expect("mkdir");
        let stored = std::fs::canonicalize(&checkout).expect("resolves");
        assert!(
            stored.display().to_string().starts_with(r"\\?\"),
            "{}",
            stored.display()
        );
        let a = project(0xa);
        let checkouts = canonical_checkouts(vec![(a, stored.display().to_string())]);
        let typed = checkout.join("Sub").display().to_string().to_uppercase();
        let cwd = std::fs::canonicalize(typed).expect("resolves case-insensitively");
        assert_eq!(project_for_cwd(&cwd, &checkouts), CwdMatch::One(a));
    }

    #[test]
    fn first_workspace_orders_projects_by_position_and_project_target_narrows() {
        let workspaces = [workspace()];
        let target = first_workspace(&workspaces);
        let slugs: Vec<_> = target.projects.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["htui", "agy"]);
        assert_eq!(target.scope.project_ids, [project(0x10), project(0x11)]);
        let narrowed = project_target(&workspaces, project(0x11)).expect("listed");
        assert_eq!(narrowed.scope.project_ids, [project(0x11)]);
        assert_eq!(narrowed.scope.workspace_id, workspaces[0].workspace_id);
        assert!(project_target(&workspaces, project(0x99)).is_none());
    }

    #[test]
    fn context_debug_never_prints_the_dsn() {
        let ctx = Context {
            dsn: "postgres://u:SENTINEL@db/htui",
            identity: None,
            now: now(),
            connect_timeout: Duration::from_secs(1),
        };
        assert!(!format!("{ctx:?}").contains("SENTINEL"));
    }
}
