//! Plan MOD-20: a registry row's declared source made real.
//!
//! [`plan`] reads the ACP registry entry the row names and answers, **before consent**, what this
//! box would fetch and how it can be verified: the archive URL and its size, the licence and its
//! terms, the digest the registry publishes (or the sentence that says it publishes none), where
//! the tree would land, and whether the disk can take it. [`install`] then executes exactly that
//! plan: it fetches, verifies, unpacks into staging, promotes in one rename, and asks the **probe**
//! what the box can now run — `R-AGT-6` in full: nothing in this module ever writes a status, and
//! an install that lands a perfect tree the probe cannot handshake ends `failed` like any other.
//!
//! Nothing here knows an agent's name (`R-AGT-5`). Every coordinate comes from three places and
//! no fourth: the row's `discovery.install` block, the registry document, and
//! [`crate::probe::install_root`]. That is what makes "a third agent installs with no code
//! change" a property of the design rather than a hope.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use htui_core::model::{Agent, AgentBox, AgentId, BoxId};
use serde::{Deserialize, Serialize};

use crate::error::DriverError;
use crate::probe::{INSTALL_ROOT_VAR, ProbeContext, ProbeEnv, ProbeOutcome, ProbeStatus, Tier2};

pub mod archive;
pub mod fetch;
pub mod http;
pub mod layout;
pub mod manifest;
pub mod plan;
pub mod registry;
pub mod run;

pub use archive::ArchiveFormat;
pub use layout::{Layout, SweepReport};
pub use manifest::{Consent, InstallRecord, Manifest};
pub use plan::plan;
pub use registry::{BinaryEntry, Distribution, RegistryAgent, RegistryDocument, RegistrySource};
pub use run::install;

/// The ACP registry's `latest` directory: `<base>/registry.json` is the document.
///
/// The one URL this crate spells, and it names a protocol registry rather than a vendor
/// (`R-AGT-5`): every agent-specific coordinate is a row in the document behind it.
pub const REGISTRY_BASE: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest";

/// Plan D11: the pre-flight refuses when `available < DISK_HEADROOM_FACTOR × content_length`.
///
/// Measured, not guessed: the largest published adapter is a 682 MB archive that unpacks to
/// 2.0 GB — 2.95× — and the archive itself sits on disk beside the tree while it is unpacked,
/// which is 3.95×. Four is that rounded up.
pub const DISK_HEADROOM_FACTOR: u64 = 4;

/// Plan D18: at most one progress frame per this interval.
///
/// The same 250 ms as the event loop's tick, so a faster stream would only queue frames that no
/// redraw ever shows.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Plan D16: `.staging/` entries older than this are swept at the next install.
pub const STAGING_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// What a cached registry document is assumed current for when the CDN sends no `max-age`.
pub const REGISTRY_DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

/// The registry `GET` and the archive `HEAD` are small; fifteen seconds is a stall, not a fetch.
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(15);
/// A TCP connect that has not completed in fifteen seconds will not complete.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A download is bounded *between chunks*, never in total: a 682 MB body on a slow line is
/// patience, a minute of silence is a hang (hazard H-13).
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Everything an installer is told about the box that is not already in a [`ProbeEnv`] (plan D18).
///
/// Production is [`Default`]; a test injects a fixture server and a temporary root, which is the
/// whole reason this is a value and not a set of constants read at the call site. `set_var` is
/// `unsafe` and forbidden here, so an override that has to reach the probe travels as data
/// ([`apply_to`](Self::apply_to)) rather than as an environment mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallConfig {
    /// [`REGISTRY_BASE`], or `http://127.0.0.1:<port>` under test.
    pub registry_base: String,
    /// When `Some`, inserted into [`ProbeEnv::vars`] as [`INSTALL_ROOT_VAR`] before anything reads
    /// it — the `set_var`-free way to keep a test off the maintainer's real install root.
    pub root_override: Option<PathBuf>,
    /// D11's free-space input when `Some`, so a case can state a full disk without owning one.
    /// `None` asks the filesystem through `fs4::available_space`, walking up to the nearest
    /// ancestor that exists — a first-ever install has no root yet — and a filesystem that will
    /// not answer leaves the size *unknown*, which the consent text states rather than refuses.
    pub disk_override: Option<u64>,
    /// Total time for the registry `GET` and the archive `HEAD`.
    pub registry_timeout: Duration,
    /// Time to establish a connection, on either client.
    pub connect_timeout: Duration,
    /// Time between two chunks of an archive body.
    pub read_timeout: Duration,
    /// [`PROGRESS_EVERY`].
    pub progress_every: Duration,
    /// [`STAGING_MAX_AGE`].
    pub staging_max_age: Duration,
    /// [`DISK_HEADROOM_FACTOR`].
    pub headroom_factor: u64,
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            registry_base: REGISTRY_BASE.to_owned(),
            root_override: None,
            disk_override: None,
            registry_timeout: REGISTRY_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            progress_every: PROGRESS_EVERY,
            staging_max_age: STAGING_MAX_AGE,
            headroom_factor: DISK_HEADROOM_FACTOR,
        }
    }
}

impl InstallConfig {
    /// [`Default`] with the two knobs every test sets: where the registry is, and where the tree
    /// may be written.
    #[must_use]
    pub fn new(registry_base: impl Into<String>, root_override: Option<PathBuf>) -> Self {
        Self {
            registry_base: registry_base.into(),
            root_override,
            ..Self::default()
        }
    }

    /// `env` with [`root_override`](Self::root_override) applied (plan D18).
    ///
    /// The single place an override becomes a `vars` entry, so [`crate::probe::install_root`] and
    /// every `%HTUI_AGENTS_ROOT%` pattern in a seed agree about the root without either of them
    /// knowing a test is running.
    #[must_use]
    pub fn apply_to(&self, mut env: ProbeEnv) -> ProbeEnv {
        if let Some(root) = &self.root_override {
            env.vars.insert(
                INSTALL_ROOT_VAR.to_owned(),
                root.to_string_lossy().into_owned(),
            );
        }
        env
    }
}

/// The HTTP clients and the config, built once per task (blueprint P-13).
///
/// Built inside the install task and not by `AgentRuntime::new`, so a client that cannot be built
/// is a failure frame at the request's own address rather than a panic on the worker loop, and a
/// runtime that nobody has pressed `i` on has opened no socket.
#[derive(Debug)]
pub struct Installer {
    http: http::HttpClient,
    config: InstallConfig,
}

impl Installer {
    /// Both clients, from one crypto-provider install.
    ///
    /// # Errors
    ///
    /// [`InstallError::Http`] when a client cannot be built.
    pub fn new(config: InstallConfig) -> Result<Self, InstallError> {
        let http = http::HttpClient::new(&config)?;
        Ok(Self { http, config })
    }

    /// What this installer was configured with.
    #[must_use]
    pub fn config(&self) -> &InstallConfig {
        &self.config
    }

    /// The clients, for the modules of this item and nothing outside them.
    pub(crate) fn http(&self) -> &http::HttpClient {
        &self.http
    }
}

wire_enum!(
    /// What the progress cell says while an install runs (plan D19).
    InstallPhase {
        /// Reading the registry and asking for the archive's size.
        Planning => "planning",
        /// Streaming the archive to `.staging/`.
        Downloading => "downloading",
        /// Comparing the computed sha256 with the published one.
        Verifying => "verifying",
        /// Writing the tree out of the archive.
        Unpacking => "unpacking",
        /// Asking the probe what the box can now run.
        Probing => "probing",
    }
);

/// One progress frame, as the sink receives it (blueprint P-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallProgress {
    /// Which step is running.
    pub phase: InstallPhase,
    /// Bytes (or archive entries) done so far in this phase.
    pub done: u64,
    /// The denominator, when there is one. A `HEAD` that carried no size makes this `None` and
    /// the cell reads bytes rather than a percentage.
    pub total: Option<u64>,
}

/// Plan D18's "at most every 250 ms", as a pure decision over an injected instant.
///
/// A phase change and a phase's last frame (`done == total`) are always admitted: the cell must
/// never be left reading `downloading 97%` because the frame that would have said `100%` fell
/// inside the window. Everything else waits for [`every`](Self::new) to have elapsed.
///
/// Injected `now` rather than `Instant::now()` inside, so the rule is tested with hand-made
/// instants and never with a sleep (hazard H-18).
#[derive(Debug)]
pub struct Throttle {
    every: Duration,
    last: Option<(InstallPhase, Instant)>,
}

impl Throttle {
    /// A throttle admitting at most one ordinary frame per `every`.
    #[must_use]
    pub fn new(every: Duration) -> Self {
        Self { every, last: None }
    }

    /// Whether this frame should reach the sink, recording it when it does.
    pub fn admit(&mut self, frame: InstallProgress, now: Instant) -> bool {
        let admit = match self.last {
            None => true,
            Some((phase, at)) => {
                phase != frame.phase
                    || frame.total == Some(frame.done)
                    || now.duration_since(at) >= self.every
            }
        };
        if admit {
            self.last = Some((frame.phase, now));
        }
        admit
    }
}

/// The pre-flight's product and the consent evidence (plan D12).
///
/// What `y` says yes to is exactly what the install executes: the registry is mutable and has no
/// pinned snapshot URL, so re-reading the document after consent could install something the user
/// never saw. This value travels back through `InstallConfirm` instead — which is also why it is
/// serialisable, and why every path in it is re-checked at run time (hazard H-16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    /// The row this plan installs for.
    pub agent_id: AgentId,
    /// The row's name, for the consent text.
    pub agent_name: String,
    /// The row's `install.tool`: the `discovery.tools` key whose glob must resolve what is
    /// written.
    pub tool: String,
    /// The registry entry id.
    pub registry_id: String,
    /// The registry entry's display name.
    pub registry_name: String,
    /// The version the registry serves.
    pub version: String,
    /// [`crate::probe::platform_key`] at plan time.
    pub platform: String,
    /// The archive URL, exactly as the entry spells it.
    pub archive_url: String,
    /// What kind of archive that URL names.
    pub format: ArchiveFormat,
    /// From the `HEAD`'s `content-length` **header**; `None` when the `HEAD` failed or carried
    /// none. See [`http::content_length_header`] for why the header and not the response.
    pub content_length: Option<u64>,
    /// Lowercase hex when the entry publishes a digest.
    pub sha256: Option<String>,
    /// The command inside the archive, as the registry spells it.
    pub cmd: String,
    /// The arguments the entry declares for this platform.
    pub args: Vec<String>,
    /// The environment the entry declares for this platform.
    pub env: BTreeMap<String, String>,
    /// The licence identifier.
    pub license: Option<String>,
    /// Where the terms are.
    pub license_url: Option<String>,
    /// The install root this plan writes under.
    pub root: PathBuf,
    /// `<root>/<registry_id>/<version>/`.
    pub install_dir: PathBuf,
    /// Directory names under `<root>/<registry_id>/` at plan time — what the consent text lists as
    /// replaced.
    pub existing_versions: Vec<String>,
    /// Free bytes under [`root`](Self::root), when known.
    pub available_bytes: Option<u64>,
    /// `headroom_factor × content_length`, when the size is known.
    pub need_bytes: Option<u64>,
    /// Whether the registry's `args` differ from the row's `PlatformGlob.args` for this platform.
    pub args_differ: bool,
    /// The manifest's recorded consent when it covers this entry's terms (plan D17); `None` means
    /// the pane says `y` accepts them.
    pub consent: Option<Consent>,
    /// The manifest's record for this same version, when it was installed before (plan D2, D17).
    pub recorded: Option<InstallRecord>,
    /// `Some(age)` when the registry was read from the cache **after a network failure**. A cache
    /// hit inside the CDN's own `max-age` is not this: the document is current, and saying
    /// "cached" about it would be noise.
    pub registry_cached_age_secs: Option<u64>,
    /// When this pre-flight ran.
    pub planned_at: DateTime<Utc>,
}

/// Plan D20: everything the user needs to do by hand, every word derived from the row and the
/// helper rather than written down in a document that goes stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualSteps {
    /// `<base>/registry.json`.
    pub registry_url: String,
    /// The entry id to look up in it.
    pub id: String,
    /// This box's platform key, which is the map key inside the entry.
    pub platform: String,
    /// The version, when the document was read far enough to know it.
    pub version: Option<String>,
    /// `<root>/<id>/<version>/` — where the **whole** archive goes.
    pub unpack_into: PathBuf,
    /// The file to make executable, when known.
    pub cmd: Option<String>,
    /// `HTUI_TOOL_<NAME>` — the escape hatch that works whatever the layout.
    pub override_key: String,
}

/// Why there is no plan (plan D12, D20, blueprint P-9).
///
/// One variant per thing the user can act on, because the consent pane renders each differently
/// and a `Transport(String)` could not carry the difference.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The row declares no source — every `NodePackage`-served adapter, and any row whose
    /// `launch` document does not parse.
    #[error("nothing declares how to install `{agent}`")]
    NoSource {
        /// The row's name.
        agent: String,
    },
    /// The row names a tool its own discovery does not declare as a glob, so nothing could check
    /// after promotion that the installer wrote where the row looks (plan D5, hazard H-4).
    #[error("`{agent}` names the tool `{tool}`, which its discovery does not declare as a glob")]
    NoGlobTool {
        /// The row's name.
        agent: String,
        /// The `install.tool` that is missing or is not a glob.
        tool: String,
    },
    /// No install root: the token is unset and the box has no local data directory.
    #[error("{var} is unset and this box has no local data directory")]
    NoRoot {
        /// [`INSTALL_ROOT_VAR`].
        var: &'static str,
    },
    /// The document parsed and lists no such entry.
    #[error("the registry lists no entry `{id}`")]
    UnknownId {
        /// The id the row named.
        id: String,
    },
    /// The entry exists and publishes nothing for this platform — a first-class answer, not a
    /// failure.
    #[error("`{id}` {version} is not available for {platform}")]
    NotAvailable {
        /// The entry id.
        id: String,
        /// The version the entry serves.
        version: String,
        /// This box's platform key.
        platform: String,
    },
    /// The archive is a shape this installer does not unpack.
    #[error("`{url}` is not a .zip, .tar.gz or .tgz archive; not installable from this entry")]
    Unsupported {
        /// The archive URL.
        url: String,
    },
    /// The entry's `cmd` is not a relative path inside the archive.
    #[error("the entry's cmd `{cmd}` is not a relative path inside the archive")]
    BadCmd {
        /// The `cmd` as the registry spells it.
        cmd: String,
    },
    /// A higher version is already installed (blueprint P-9, hazard H-15). Refused here, before
    /// consent and before a byte is fetched, because the row's glob would resolve the higher one
    /// afterwards and the post-promote check would roll the whole download back.
    #[error(
        "{installed} is already installed at {path} and outranks {offered}; \
         remove it to install this version"
    )]
    Outranked {
        /// The version already on disk.
        installed: String,
        /// The version the registry offers.
        offered: String,
        /// The directory to remove by hand.
        path: PathBuf,
    },
    /// D11's arithmetic said no.
    #[error("{need} bytes needed ({factor}× the archive), {available} available under {root}")]
    Disk {
        /// `factor × content_length`.
        need: u64,
        /// What the filesystem reports free.
        available: u64,
        /// [`DISK_HEADROOM_FACTOR`].
        factor: u64,
        /// The install root the number is about.
        root: PathBuf,
    },
    /// The document was fetched and does not parse.
    #[error("the registry document does not parse: {message}")]
    Malformed {
        /// What serde said.
        message: String,
    },
    /// No network and no cache. Carries the derived instructions the section renders instead
    /// (plan D20).
    #[error("{message}")]
    Network {
        /// What the transport said.
        message: String,
        /// What to do by hand. Boxed: it is much the largest variant and every other one would
        /// otherwise pay for it.
        manual: Box<ManualSteps>,
    },
}

/// Why the pipeline stopped short of the probe.
///
/// `Ok(InstallOutcome)` covers everything the probe got to say, including "no": a re-probe that
/// answers `missing` is an outcome with a rollback, not an error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    /// The user pressed `x`, or the runtime shut down.
    #[error("cancelled")]
    Cancelled,
    /// The transport failed. Carries D20's derived instructions.
    #[error("{message}")]
    Network {
        /// What the transport said.
        message: String,
        /// What to do by hand.
        manual: Box<ManualSteps>,
    },
    /// A client could not be built.
    #[error("HTTP client: {0}")]
    Http(String),
    /// The download does not match the digest the registry publishes. Nothing is unpacked.
    #[error(
        "sha256 mismatch: the registry publishes {expected}, the download is {computed}; \
         nothing was unpacked"
    )]
    DigestMismatch {
        /// What the registry publishes.
        expected: String,
        /// What the bytes hash to.
        computed: String,
    },
    /// The archive itself is refused: an entry that escapes the tree, or a shape that will not
    /// read.
    #[error("the archive is refused: {message}")]
    Archive {
        /// Which entry, and why.
        message: String,
    },
    /// A filesystem step failed.
    #[error("{what}: {message}")]
    Io {
        /// The step, named.
        what: String,
        /// The operating system's words.
        message: String,
    },
    /// Promoted, but the row's own glob does not resolve what was written (plan D5, hazard H-4).
    #[error("promoted {promoted} but the row's glob resolves {resolved:?}; rolled back")]
    NotWhereTheRowLooks {
        /// What was promoted.
        promoted: PathBuf,
        /// What the glob answered instead.
        resolved: Option<PathBuf>,
    },
    /// The re-probe's own machinery failed.
    #[error(transparent)]
    Driver(#[from] DriverError),
}

/// What the pipeline ended with, once the probe has spoken (plan D16).
///
/// Both arms carry an [`InstallRecord`]: an install that failed still downloaded a specific
/// archive with a specific digest, and that is the fact plan D2 needs to survive so a later
/// re-install of the same version can be told apart from this one.
///
/// There is no `status` the installer chose. `R-AGT-6` is the whole reason this type has the shape
/// it does: [`Installed::status`](Self::Installed) and [`Failed::status`](Self::Failed) are read
/// off the row [`crate::probe::probe_agent`] returned, and a pipeline that downloaded, verified,
/// unpacked and promoted perfectly into a tree the probe cannot handshake ends
/// [`Failed`](Self::Failed) like any other.
#[derive(Debug, Clone, PartialEq)]
#[must_use = "a dropped outcome is an install whose row was never written"]
pub enum InstallOutcome {
    /// Plan D16(a): promoted, re-probed `ready` or `unauthenticated`, the siblings removed and the
    /// manifest written.
    Installed {
        /// What was recorded in `<root>/<id>/manifest.json`.
        record: InstallRecord,
        /// The version that was installed.
        version: String,
        /// `<root>/<id>/<version>/`, as it now exists.
        dir: PathBuf,
        /// What the probe wrote; the caller upserts it.
        row: AgentBox,
        /// The row's own status, repeated for a caller that does not want to parse the snapshot.
        status: ProbeStatus,
        /// The version directories plan D3's retention removed, in name order.
        removed_versions: Vec<String>,
        /// Plan D2/D17: this version was installed before and the archive hashes differently now.
        digest_changed: bool,
    },
    /// Plan D16(b)/(c): the re-probe said `missing` or `failed`.
    Failed {
        /// What was downloaded, whether or not it is still on disk.
        record: InstallRecord,
        /// The version that was being installed.
        version: String,
        /// The **first** re-probe's status — the verdict on the tree that was promoted.
        status: ProbeStatus,
        /// That probe's failure text, line by line.
        stderr_tail: Option<Vec<String>>,
        /// (b): the version the box was left resolving, once the promoted tree was removed.
        /// (c): `None`, and the tree is still there.
        restored: Option<String>,
        /// The **last** probe's answer, for the caller to write. In case (b) that is the second
        /// probe, so the row describes what is left rather than what was deleted; a
        /// [`ProbeOutcome::Kept`] writes nothing at all (plan D51).
        probe: ProbeOutcome,
    },
}

/// The row-side inputs of [`install`] (blueprint P-1).
///
/// One value rather than six arguments because they travel together and always have: five of them
/// are exactly what [`crate::probe::probe_agent`] is called with, and the sixth is the plan the
/// user consented to.
pub struct InstallJob<'a> {
    /// What `y` accepted. Re-checked at run time, never trusted (hazard H-16).
    pub plan: &'a InstallPlan,
    /// The registry row being installed for.
    pub agent: &'a Agent,
    /// This box.
    pub box_id: BoxId,
    /// The stored row, so the re-probe carries `quota` over and honours a `manual` source.
    pub existing: Option<&'a AgentBox>,
    /// The box as a value, and the instant every timestamp is stamped with.
    pub ctx: &'a ProbeContext,
    /// Tier 2, injected: production spawns, a test answers from a duplex.
    pub tier2: &'a dyn Tier2,
}

/// The coordinates, and tier 2 as the fact that there is one.
///
/// Hand-written because `dyn Tier2` has no `Debug` — and because a derive would be the wrong
/// habit here anyway: `ctx` holds a [`ProbeEnv`], whose own `Debug` prints the environment by
/// **count** so that one `?job` in a log line cannot spill every secret on the box (`R-SEC-2`).
impl core::fmt::Debug for InstallJob<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InstallJob")
            .field("plan", &self.plan)
            .field("agent", &self.agent.name)
            .field("box_id", &self.box_id)
            .field("existing", &self.existing.is_some())
            .field("ctx", &self.ctx)
            .finish_non_exhaustive()
    }
}
