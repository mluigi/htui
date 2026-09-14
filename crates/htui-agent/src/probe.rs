//! ANA-4 §4.6's two-tier probe (plan MOD-2 D45–D51): what this box can run, as the snapshot
//! `agent_box.probe` holds. Tier 1 resolves every `discovery.tools` entry and captures versions;
//! tier 2 spawns the resolved launch and completes `initialize`. Nothing here writes a store row:
//! the caller decides where the answer lands (`R-NF-3` keeps that caller off the UI task).
//!
//! Tier 1 is [`resolve_tool`], the glob walker and version capture; tier 2 is the [`Tier2`] seam,
//! whose production implementation ([`SpawnTier2`]) spawns the resolved launch and completes
//! `initialize` through [`crate::acp::handshake()`]. [`probe_agent`] composes the two into the
//! [`ProbeSnapshot`] the column holds, and answers with a [`ProbeOutcome`] — a row to write, or a
//! hand-written row left exactly as it was.
//!
//! Every tier reads its box from an injected [`ProbeEnv`] rather than from `std::env`. The
//! workspace is `unsafe_code = "forbid"` and `std::env::set_var` is `unsafe` on this edition, so a
//! tier that read the process environment directly could not be tested at all — the same reason
//! `tools::resolve_with` takes its override lookup as an argument.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use htui_core::model::{Agent, AgentBox, BoxId, Transport};
use regex::Regex;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, warn};

use crate::acp::{AcpIo, HANDSHAKE_TIMEOUT, Handshake};
use crate::driver::DriverFuture;
use crate::error::{DriverError, Result};
use crate::launch::{
    AcpSettings, AgentLaunch, AgentSettings, ChildGuard, CredentialProbe, Discovery,
    ResolvedLaunch, ToolMap, ToolProbe, VersionProbe,
};
use crate::tools::env_override_key;

/// How long one of tier 1's short-lived children may run before its process tree is killed: a
/// `--version`, whose version is then recorded unknown, or the `npm root -g` of the node-package
/// tier, which then resolves nowhere.
///
/// Generous on purpose: a cold `node` on a laptop with a slow disk takes seconds, and recording
/// "version unknown" for a tool that is *present* costs the Settings tab a column, not a session.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------------------------
// The box, as a value
// ---------------------------------------------------------------------------------------------

/// The box as the resolver sees it: injected, never read from `std::env` inside a tier.
///
/// `Debug` is hand-written and prints `vars`'s **size**, never its contents — see the impl.
#[derive(Clone)]
pub struct ProbeEnv {
    /// Where relative `node_modules` and `which` lookups start.
    pub cwd: PathBuf,
    /// `<os>-<arch>` in the ACP registry's vocabulary (`launch.rs`'s five keys): [`platform_key`].
    pub platform: String,
    /// What a leading `~` expands to. `None` skips every `~` pattern.
    pub home: Option<PathBuf>,
    /// The environment: `HTUI_TOOL_<NAME>` overrides, `%VAR%` expansion, `PATH`.
    pub vars: BTreeMap<String, String>,
    /// Whether tier 1 runs `--version` children. `tools::resolve` and the `ChatStart` re-probe
    /// say `false`; `Settings > r` says `true`.
    pub versions: bool,
    /// Per short-lived child: a `--version`, and the `npm root -g` that runs whatever
    /// [`versions`](Self::versions) says.
    pub version_timeout: Duration,
}

/// `vars` by **count**, everything else verbatim (`R-SEC-2`).
///
/// [`ProbeEnv::host`] snapshots the whole process environment, and [`ProbeContext`] holds a
/// `ProbeEnv`, so a derived `Debug` would turn one `?env` or `?ctx` in a log line into every secret
/// on the box printed to the log file. This is the footgun [`ResolvedLaunch`] got `RedactedEnv`
/// for; nothing logs a `ProbeEnv` today, and the type closes it anyway rather than relying on
/// nobody ever adding the field to a `warn!`.
impl core::fmt::Debug for ProbeEnv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProbeEnv")
            .field("cwd", &self.cwd)
            .field("platform", &self.platform)
            .field("home", &self.home)
            .field("vars_len", &self.vars.len())
            .field("versions", &self.versions)
            .field("version_timeout", &self.version_timeout)
            .finish()
    }
}

impl ProbeEnv {
    /// This process's box: `std::env::vars_os()` (lossy), `dirs::home_dir()`, [`platform_key`],
    /// `versions: true`, [`VERSION_TIMEOUT`].
    ///
    /// The environment is snapshotted on every call rather than cached: a `OnceLock` would freeze
    /// a `PATH` the user changed mid-session, and the snapshot costs microseconds.
    #[must_use]
    pub fn host(cwd: PathBuf) -> Self {
        let vars = std::env::vars_os()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        let mut env = Self {
            cwd,
            platform: platform_key(),
            home: dirs::home_dir(),
            vars,
            versions: true,
            version_timeout: VERSION_TIMEOUT,
        };
        // Plan MOD-20 D15: the seeds reach the install root through `%HTUI_AGENTS_ROOT%`, so on a
        // box that does not set it the token has to be seeded somewhere — here, once, rather than
        // as a fallback inside `install_root`, which no test could then keep off the real disk.
        //
        // The absence is decided by `var`, not by `vars.contains_key`: on Windows an environment
        // block may carry the name in another case, `var` is what every later reader goes through,
        // and inserting the canonical spelling beside a `htui_agents_root` the user set would
        // shadow their value with the default — exactly the thing "unless the environment sets
        // it" forbids.
        if env.var(INSTALL_ROOT_VAR).is_none()
            && let Some(root) = default_install_root()
        {
            env.vars.insert(
                INSTALL_ROOT_VAR.to_owned(),
                root.to_string_lossy().into_owned(),
            );
        }
        env
    }

    /// The same env with [`versions`](Self::versions) off: resolution without the `--version`
    /// children, which is what a chat start and a tier-2-only re-probe want.
    #[must_use]
    pub fn without_versions(self) -> Self {
        Self {
            versions: false,
            ..self
        }
    }

    /// `vars[key]`, for the override, `%VAR%` and `PATH` tiers.
    ///
    /// Exact first, then — on Windows only — case-insensitively: `PATH` is spelled `Path` in a
    /// Windows environment block and the seeds' patterns say `%LOCALAPPDATA%`, so an exact-only
    /// lookup would make every `Path` probe on Windows resolve nowhere.
    #[must_use]
    pub fn var(&self, key: &str) -> Option<&str> {
        if let Some(value) = self.vars.get(key) {
            return Some(value);
        }
        if cfg!(windows) {
            return self
                .vars
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str());
        }
        None
    }
}

/// `std::env::consts::{OS, ARCH}` as `<os>-<arch>`, with `macos` spelled `darwin` — the one
/// difference between Rust's names and the ACP registry's five keys (`darwin-aarch64`,
/// `linux-x86_64`, `linux-aarch64`, `windows-x86_64`, `windows-aarch64`).
#[must_use]
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

// ---------------------------------------------------------------------------------------------
// Where an installed adapter lives (plan MOD-20 D15)
// ---------------------------------------------------------------------------------------------

/// The variable that names this box's install root.
///
/// [`ProbeEnv::host`] seeds it from [`default_install_root`] unless the environment already says
/// otherwise, and the seed documents reach it as `%HTUI_AGENTS_ROOT%` — a token the glob expander
/// already understands. That is what lets a seed carry **no** root at all: the document and the
/// installer cannot disagree about a path neither of them spells, and a box whose adapters do not
/// fit under the default (a version of one is gigabytes) relocates them with one variable instead
/// of an edited registry row.
pub const INSTALL_ROOT_VAR: &str = "HTUI_AGENTS_ROOT";

/// `dirs::data_local_dir()/htui/agents`: `~/.local/share` (or `$XDG_DATA_HOME`) on Linux,
/// `~/Library/Application Support` on macOS, `%LOCALAPPDATA%` on Windows.
///
/// The three roots the seeds used to hand-write, in one place. `data_local_dir` and not
/// `dirs::config_dir()` because this is bulk data, not settings, and on Windows the difference is
/// `%LOCALAPPDATA%` against the roaming `%APPDATA%` — nobody wants a gigabyte adapter synchronised
/// to a domain profile.
///
/// `None` on a box with no local data directory: a refusal the caller reports by name, never a
/// path invented on its behalf.
#[must_use]
pub fn default_install_root() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("htui").join("agents"))
}

/// This box's install root as the injected environment states it, through [`ProbeEnv::var`] — so
/// the Windows case-insensitive lookup applies here exactly as it does inside a `%VAR%` pattern,
/// and a reader and the expander can never resolve the same token differently.
///
/// `None` when the token is absent, which is a hand-built [`ProbeEnv`] that did not inject it.
/// Deliberately **no** fallback to [`default_install_root`]: the seeding happens once, in
/// [`ProbeEnv::host`], so a test's env is never quietly answered from the maintainer's real disk.
#[must_use]
pub fn install_root(env: &ProbeEnv) -> Option<PathBuf> {
    env.var(INSTALL_ROOT_VAR).map(PathBuf::from)
}

// ---------------------------------------------------------------------------------------------
// What one tier answers
// ---------------------------------------------------------------------------------------------

/// One tool, found (plan D46).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResolution {
    /// The file `${name}` substitutes to. Absolute after `which`; as expanded after a glob.
    pub path: PathBuf,
    /// Captured version, `None` = present, version unknown (plan D47).
    pub version: Option<String>,
    /// [`PlatformGlob::args`] of the matching platform, appended to `agent.launch.args`.
    ///
    /// [`PlatformGlob::args`]: crate::launch::PlatformGlob::args
    pub args: Vec<String>,
    /// `version` parsed below [`VersionProbe::min`]. Counted as missing by [`probe_tools`], while
    /// the version it does have is still recorded — the log says which floor it missed.
    pub below_min: bool,
}

/// Every tool of one `discovery`, resolved or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolReport {
    /// Resolved tools by `${name}`.
    pub found: BTreeMap<String, ToolResolution>,
    /// Names that resolved nowhere or below their floor, in `BTreeMap` order. A name below its
    /// floor is in **both** maps: it was found, and it does not count.
    pub missing: Vec<String>,
}

impl ToolReport {
    /// `found` as the map [`launch::resolve`](crate::launch::resolve) substitutes from.
    #[must_use]
    pub fn tool_map(&self) -> ToolMap {
        self.found
            .iter()
            .map(|(name, found)| (name.clone(), found.path.to_string_lossy().into_owned()))
            .collect()
    }

    /// The snapshot's `tools` key: every **found** name → its version (or `null`).
    #[must_use]
    pub fn versions(&self) -> BTreeMap<String, Option<String>> {
        self.found
            .iter()
            .map(|(name, found)| (name.clone(), found.version.clone()))
            .collect()
    }

    /// Every found tool's `args`, name order — the platform append of ANA-4 §4.6, which is how
    /// `agy`'s Linux-only `--uid=` reaches the command line without a second registry row.
    #[must_use]
    pub fn extra_args(&self) -> Vec<String> {
        self.found
            .values()
            .flat_map(|found| found.args.iter().cloned())
            .collect()
    }

    /// `missing.is_empty()`: nothing is spawned unless this is true (plan D50).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

wire_enum!(
    /// `probe.status` (ANA-4 §4.6, plan D50).
    ProbeStatus {
        /// Resolved, spawned, `initialize` answered, no auth demanded.
        Ready => "ready",
        /// `initialize` answered with a non-empty `authMethods`.
        Unauthenticated => "unauthenticated",
        /// A required tool resolved nowhere or below its floor. Nothing was spawned.
        Missing => "missing",
        /// The launch resolved but did not spawn, or `initialize` did not complete.
        Failed => "failed",
    }
);

wire_enum!(
    /// `probe.credential` (plan D59): which tier of `discovery.credential` answered.
    ///
    /// The *value* — a token's text, a variable's contents — is never read into memory by the probe
    /// and never recorded: the file tier is a `stat` and the variable tier asks only whether the
    /// name is set. What the column holds is which of the row's declared tiers said yes, which is
    /// everything the status mapping needs and nothing a `JSONB` column rendered verbatim by the
    /// Settings tab should not hold (`R-SEC-2`).
    CredentialTier {
        /// A declared file exists.
        File => "file",
        /// A declared variable is set and non-empty.
        Env => "env",
        /// The row declares candidates and none answered.
        Absent => "absent",
    }
);

wire_enum!(
    /// `probe.source` (plan D45): who wrote the row. A `manual` row survives a probe that finds
    /// nothing (plan D51, ANA-4 §4.6).
    #[derive(Default)]
    ProbeSource {
        /// Written by the probe itself.
        #[default]
        Probe => "probe",
        /// Hand-written in Settings (the editor is a later milestone's; the rule is honoured now).
        Manual => "manual",
    }
);

// ---------------------------------------------------------------------------------------------
// Tier 1: resolution
// ---------------------------------------------------------------------------------------------

/// One probe, one answer (plan D46). No override tier here: the two callers apply it themselves
/// with different rules ([`probe_tools`] checks that the override exists, `tools::resolve` does
/// not), and this function is the tier walk they share.
///
/// - [`ToolProbe::Path`]: the first of `names` that `which::which_in` finds on `env.var("PATH")`,
///   relative to `env.cwd`; `version` captured when `env.versions` and the probe declares one.
/// - [`ToolProbe::NodePackage`]: `<cwd>/node_modules/<package>/<entry>`, else
///   `<npm root -g>/<package>/<entry>`; a missing or failing `npm` is "not found", not an error.
///   `version` is the package's own `package.json` `"version"`, read from disk — no spawn. The
///   `fallback` is **not** consulted: a [`ToolMap`] entry is one string, and a command with
///   arguments does not fit in one.
/// - [`ToolProbe::Glob`]: [`glob_first`] over the matching platform's `patterns`, then the
///   platform-independent ones; `args` are the matching platform's whenever `env.platform` has an
///   entry, whichever list matched. `version` is always `None` — a globbed agent answers with its
///   handshake, and ANA-4's probe table gives it no `--version` tier.
///
/// # Errors
/// [`DriverError::Transport`] when a blocking lookup could not be scheduled. "Found nothing" is
/// `Ok(None)`: swallowing a runtime fault as "missing" would mark a healthy box `missing`.
pub async fn resolve_tool(probe: &ToolProbe, env: &ProbeEnv) -> Result<Option<ToolResolution>> {
    match probe {
        ToolProbe::Path { names, version } => {
            let Some(path) = on_path(names, env).await? else {
                return Ok(None);
            };
            let captured = match version {
                Some(version) if env.versions => capture_version(&path, version, env).await,
                _ => None,
            };
            let below = match (
                captured.as_deref(),
                version.as_ref().and_then(|v| v.min.as_deref()),
            ) {
                (Some(found), Some(min)) => below_min(found, min),
                _ => false,
            };
            Ok(Some(ToolResolution {
                path,
                version: captured,
                args: Vec::new(),
                below_min: below,
            }))
        }
        ToolProbe::NodePackage { package, entry, .. } => {
            let Some((path, package_dir)) = node_package(package, entry, env).await? else {
                return Ok(None);
            };
            let version = if env.versions {
                package_version(&package_dir).await
            } else {
                None
            };
            Ok(Some(ToolResolution {
                path,
                version,
                args: Vec::new(),
                below_min: false,
            }))
        }
        ToolProbe::Glob { patterns, platform } => {
            let per_platform = platform.get(&env.platform);
            let mut found = match per_platform {
                Some(entry) => glob_first(&entry.patterns, env).await?,
                None => None,
            };
            if found.is_none() {
                found = glob_first(patterns, env).await?;
            }
            Ok(found.map(|path| ToolResolution {
                path,
                version: None,
                args: per_platform
                    .map(|entry| entry.args.clone())
                    .unwrap_or_default(),
                below_min: false,
            }))
        }
    }
}

/// Every tool of `discovery`, override tier first, then [`resolve_tool`].
///
/// The override is `HTUI_TOOL_<NAME>` in `env.vars` and it is **checked**: a path that does not
/// exist is missing, with a `warn!` naming the key. A probe that recorded a nonexistent override
/// as `ready` would lie in the one place a box is meant to be honest about itself — which is why
/// this differs from `tools::resolve`, whose override is the escape hatch for a live chat and is
/// taken on trust. An override that *does* exist is recorded with no version: it says which file
/// is the tool, and the probe does not second-guess a box whose layout it did not understand.
///
/// Walks **all** tools rather than stopping at the first miss, so the snapshot lists every
/// version it did find. A found tool below its floor is recorded in `found` *and* `missing`. A
/// `None` discovery is an empty, complete report.
///
/// # Errors
/// As [`resolve_tool`].
pub async fn probe_tools(discovery: Option<&Discovery>, env: &ProbeEnv) -> Result<ToolReport> {
    let mut report = ToolReport::default();
    let Some(discovery) = discovery else {
        return Ok(report);
    };

    for (name, probe) in &discovery.tools {
        let key = env_override_key(name);
        if let Some(value) = env.var(&key).map(ToOwned::to_owned) {
            let path = PathBuf::from(&value);
            if exists(&path).await {
                report.found.insert(
                    name.clone(),
                    ToolResolution {
                        path,
                        version: None,
                        args: Vec::new(),
                        below_min: false,
                    },
                );
            } else {
                warn!(
                    tool = name,
                    override_key = key,
                    path = value,
                    "the tool override names a path that does not exist"
                );
                report.missing.push(name.clone());
            }
            continue;
        }

        match resolve_tool(probe, env).await? {
            Some(found) => {
                if found.below_min {
                    warn!(
                        tool = name,
                        version = found.version,
                        "the tool is below the version floor its row requires"
                    );
                    report.missing.push(name.clone());
                }
                report.found.insert(name.clone(), found);
            }
            None => {
                debug!(
                    tool = name,
                    override_key = key,
                    "the tool resolved nowhere on this box"
                );
                report.missing.push(name.clone());
            }
        }
    }
    Ok(report)
}

/// The first of `names` that `which` finds on the injected `PATH`, or `None`.
///
/// `which` rather than a bare `Command::new`: `std::process::Command` does not read `PATHEXT`, so
/// a Windows `.cmd` shim handed straight to `CreateProcess` fails with `os error 193` (ANA-4
/// §4.6's Windows shim rule, which `launch::spawn` follows for the same reason). It is
/// synchronous and touches the filesystem, so it runs on `spawn_blocking`.
async fn on_path(names: &[String], env: &ProbeEnv) -> Result<Option<PathBuf>> {
    let names = names.to_vec();
    let paths = env.var("PATH").map(ToOwned::to_owned);
    let cwd = env.cwd.clone();
    tokio::task::spawn_blocking(move || {
        names
            .iter()
            .find_map(|name| which::which_in(name, paths.as_deref(), &cwd).ok())
    })
    .await
    .map_err(|err| DriverError::Transport(format!("tool lookup did not run: {err}")))
}

/// `<cwd>/node_modules/<package>/<entry>`, else `<npm root -g>/<package>/<entry>`, else `None`,
/// as `(entry file, package directory)` — the second half is where `package.json` lives.
async fn node_package(
    package: &str,
    entry: &str,
    env: &ProbeEnv,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let local = env.cwd.join("node_modules").join(package);
    if exists(&local.join(entry)).await {
        return Ok(Some((local.join(entry), local)));
    }

    let Some(root) = npm_root_global(env).await? else {
        return Ok(None);
    };
    let global = root.join(package);
    if exists(&global.join(entry)).await {
        return Ok(Some((global.join(entry), global)));
    }
    Ok(None)
}

/// The directory `npm root -g` prints, or `None` when `npm` is absent, hangs or fails.
///
/// A missing `npm` is not an error: it means this tier found nothing, and "missing" is the honest
/// report. A *broken* `npm` is treated the same way, on purpose — the resolution answer is
/// identical and a transport error would name the wrong problem.
///
/// Through [`run_bounded`], exactly as [`capture_version`] is, and for a stronger reason: this tier
/// is on `tools::resolve`'s path at every `ChatStart`, where no `HANDSHAKE_TIMEOUT` covers it. A
/// bare `Command::output()` here would let one hung `npm` stall the probe task forever — Settings
/// stuck on `probing…`, a second `r` refused for the rest of the session — and orphan the child
/// when `finish_background` or `shutdown` aborts that task.
async fn npm_root_global(env: &ProbeEnv) -> Result<Option<PathBuf>> {
    let Some(npm) = on_path(&["npm".to_owned()], env).await? else {
        return Ok(None);
    };
    let launch = ResolvedLaunch {
        command: npm.to_string_lossy().into_owned(),
        args: vec!["root".to_owned(), "-g".to_owned()],
        env: BTreeMap::new(),
    };
    let Some(child) = run_bounded("npm root -g", &launch, env).await else {
        return Ok(None);
    };
    if !child.status.success() {
        return Ok(None);
    }
    let root = child.stdout.trim().to_owned();
    if root.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(root)))
}

/// Whether `path` exists, off the runtime's worker.
///
/// The two tiers that name a path outright: the `HTUI_TOOL_*` override, whose whole job is to say
/// "the tool is *this*", and the `node_package` tier, whose entry point is a path it composed from
/// the package layout. Both are asking whether the thing they were told about is there at all, and
/// neither has a reason to be pickier than the box's own author. The **glob** tier does not come
/// through here: its leaf filter is [`walk`]'s own `is_file`, applied to every candidate the walk
/// produced rather than to one path a caller had in mind.
///
/// Async `metadata` rather than a blocking `Path::exists`: this runs on the probe's task, which
/// shares a runtime with the UI (`R-NF-3`).
async fn exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

/// Whether `path` is a **file**, off the runtime's worker.
///
/// [`exists`]'s stricter sibling, `pub(crate)` for [`crate::acp::AcpDriver::launch_for`], which
/// applies D58's fourth rule — the recorded command is still on disk — and is asking a narrower
/// question than the tiers above. It is about to hand the path to `execve`, and the self-update
/// ANA-4 §4.6 describes rearranges version-numbered *directories*: a recorded path that has become
/// one exists, is not a launch, and would fail the chat with a `Spawn` error and earn the row a
/// D60 re-probe it does not need. One `is_file` on the same `metadata` call turns that into the
/// fallback the rule already has.
///
/// A path that cannot be `stat`ed at all — gone, or a directory this process may not traverse — is
/// `false` for the same reason: the answer to "can this be spawned" is no either way, and
/// resolution is the honest second attempt.
pub(crate) async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

// ---------------------------------------------------------------------------------------------
// The glob walker (plan D48)
// ---------------------------------------------------------------------------------------------

/// `%VAR%` (any position, from `env.vars`) and a leading `~` (from `env.home`) expanded, then the
/// pattern split on `/`.
///
/// `None` when a named variable is unset or `~` has no home: the pattern is skipped, not an error
/// — a Linux box legitimately has no `%LOCALAPPDATA%`.
///
/// Returns the literal root (every leading segment without `*`, joined with `PathBuf::push`, so an
/// expanded `C:\Users\x\AppData\Local` stays one segment) and the remaining segments.
///
/// A pattern whose **first** segment is a wildcard is also `None`: the walk needs a directory to
/// start from, and an empty root would make [`walk`] call `read_dir("")` and answer "no match" for
/// a pattern that was never tried. Every seed pattern starts with `~`, `%VAR%` or a literal
/// segment, so this is a refusal at the edge of the grammar rather than a limitation inside it.
#[must_use]
pub fn expand(pattern: &str, env: &ProbeEnv) -> Option<(PathBuf, Vec<String>)> {
    let mut root = PathBuf::new();
    let mut rest: Vec<String> = Vec::new();

    for (index, raw) in pattern.split('/').enumerate() {
        if index == 0 && raw.is_empty() {
            // A pattern rooted at `/`. `PathBuf::push("")` would leave the buffer empty and turn
            // an absolute pattern into a relative one.
            root.push(std::path::MAIN_SEPARATOR_STR);
            continue;
        }
        let segment = if index == 0 && raw == "~" {
            env.home.as_ref()?.to_string_lossy().into_owned()
        } else {
            expand_vars(raw, env)?
        };
        if rest.is_empty() && !segment.contains('*') {
            root.push(&segment);
        } else {
            if root.as_os_str().is_empty() {
                debug!(
                    pattern,
                    "a glob pattern starts with a wildcard; it has no root to walk"
                );
                return None;
            }
            rest.push(segment);
        }
    }
    Some((root, rest))
}

/// `%NAME%` replaced from `env.vars`; `None` when one of them is unset. An unpaired `%` is literal
/// text, the same tolerance `launch::resolve` grants an unterminated `${`.
fn expand_vars(raw: &str, env: &ProbeEnv) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return Some(out);
        };
        let name = &after[..end];
        let Some(value) = env.var(name) else {
            debug!(variable = name, "a glob pattern names an unset variable");
            return None;
        };
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// `*` matches any run of characters **within** one name; there is no `**`, no `?` and no `[..]`.
///
/// The seeds use `*` per segment and nothing else, so the grammar is closed and a crate for it
/// would be a dependency bought for one wildcard. Case-insensitive on Windows, exact elsewhere.
#[must_use]
pub fn segment_matches(pattern: &str, name: &str) -> bool {
    // A directory entry never contains a separator; a *pattern* segment cannot either, because
    // `expand` split on `/`. Saying so here is what makes "within one name" a property rather
    // than an accident of how the walker calls this.
    if name.chars().any(std::path::is_separator) {
        return false;
    }
    if cfg!(windows) {
        star_match(&pattern.to_lowercase(), &name.to_lowercase())
    } else {
        star_match(pattern, name)
    }
}

/// [`segment_matches`] without the case folding: anchored at both ends, `*` between the literals.
fn star_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let Some(mut rest) = name.strip_prefix(parts[0]) else {
        return false;
    };
    let last = parts[parts.len() - 1];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// The most candidates one [`walk`] carries from one segment to the next.
///
/// The fan-out of a `*` segment is multiplicative and the patterns are user-authored: a hand-written
/// `~/*/*/*` over a large home enumerates the whole tree on the blocking pool for a walk whose
/// answer is one file. 4096 is far past every seed pattern — a JetBrains box has a handful of IDE
/// directories and a handful of builds under each — and far short of "stat this disk".
pub const MAX_GLOB_CANDIDATES: usize = 4096;

/// One leaf [`walk`] found, and what each `*` segment of the pattern matched on the way to it
/// (plan MOD-20 D14).
///
/// The captures exist so [`newest`] can read the *version* out of a path without knowing which
/// segment held it: the walker is the only code that knows which names came from a wildcard and
/// which were spelled by the pattern, so it is the walker that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobMatch {
    /// The file itself — every [`walk`] result is a leaf that `is_file()`.
    pub path: PathBuf,
    /// One entry per segment containing `*`, in pattern order: the directory (or leaf) name that
    /// segment matched. Empty for a pattern with no wildcard. The seeds put the version segment
    /// last, which is the one [`newest`] keys on.
    pub captures: Vec<String>,
}

/// From `root`, one segment at a time: a literal segment is pushed, a `*` segment reads the
/// directory and forks on every matching entry. Every leaf that `is_file()`.
///
/// Each candidate carries the names its `*` segments matched, so the answer is a [`GlobMatch`]
/// rather than a bare path: a literal segment leaves the captures alone, a `*` segment appends the
/// name it matched.
///
/// Bounded to [`MAX_GLOB_CANDIDATES`] per segment, with a `warn!` naming the pattern when the cap
/// trips: past that point the walk is answering a pattern that was never meant to match, and the
/// truncation is visible in the log rather than silent in the timing.
///
/// Synchronous — it is `read_dir` and `stat`. [`glob_first`] is the async wrapper that keeps it
/// off the runtime's worker.
#[must_use]
pub fn walk(root: &Path, segments: &[String]) -> Vec<GlobMatch> {
    let mut current = vec![GlobMatch {
        path: root.to_path_buf(),
        captures: Vec::new(),
    }];
    for segment in segments {
        let mut next = Vec::new();
        if segment.contains('*') {
            for candidate in &current {
                let Ok(entries) = std::fs::read_dir(&candidate.path) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if segment_matches(segment, &name.to_string_lossy()) {
                        let mut captures = candidate.captures.clone();
                        captures.push(name.to_string_lossy().into_owned());
                        next.push(GlobMatch {
                            path: candidate.path.join(name),
                            captures,
                        });
                    }
                }
            }
        } else {
            for candidate in &current {
                next.push(GlobMatch {
                    path: candidate.path.join(segment),
                    captures: candidate.captures.clone(),
                });
            }
        }
        if next.is_empty() {
            return Vec::new();
        }
        if next.len() > MAX_GLOB_CANDIDATES {
            warn!(
                pattern = %root.join(segments.join("/")).display(),
                candidates = next.len(),
                cap = MAX_GLOB_CANDIDATES,
                "a glob pattern fans out past the candidate cap; the walk is truncated"
            );
            next.truncate(MAX_GLOB_CANDIDATES);
        }
        current = next;
    }
    current.retain(|found| found.path.is_file());
    current
}

/// The highest version first, then the newest `mtime`, then the path **descending** (plan MOD-20
/// D14, amending MOD-2 plan D48).
///
/// The version is `version_key` of the **last** capture — the segment the seeds put the version
/// in. `None < Some` in Rust's `Ord for Option`, so every capture that parses as a version outranks
/// every capture that does not, which is this crate's reading of "a directory named like a version
/// is an install". Among the ones that parse, `semver` decides, and a prerelease sits below its
/// release.
///
/// Among the ones that do **not** parse — a JetBrains-managed tree whose segment is a build number,
/// a pattern with no `*` and so no capture at all — D48's rule stands exactly as it shipped: newest
/// `mtime`, ties broken by the descending path, an unreadable timestamp sorting as the epoch,
/// present but never preferred. That is what keeps a box that installs nothing resolving the file
/// it resolved before.
///
/// D48 keyed on the mtime alone. It had to be amended because unpacking an archive preserves the
/// vendor's build date rather than the install date, so the version installed last routinely
/// carries the older timestamp; and because the path tie-break it fell through to reads `1.9.0` as
/// higher than `1.10.0`.
#[must_use]
pub fn newest(matches: Vec<GlobMatch>) -> Option<PathBuf> {
    matches
        .into_iter()
        .map(|found| {
            let version = found
                .captures
                .last()
                .map(String::as_str)
                .and_then(version_key);
            let mtime = found
                .path
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (version, mtime, found.path)
        })
        .max()
        .map(|(_, _, path)| path)
}

/// The version a capture names, or `None` when it does not name one.
///
/// Strict `semver` but for a leading `v`, which registries and release tags spell both ways.
/// Nothing else is coerced: `2026.1` is two components and stays unparsable on purpose, because a
/// guess here silently reorders somebody's installs.
///
/// Visible to the installer on purpose (MOD-20): the pre-flight's downgrade refusal and the
/// rollback's "what is the box left resolving" both have to rank version *directories* exactly as
/// [`newest`] ranks a glob capture, and a second copy of this rule would be a pair of answers that
/// agree until the day one of them is edited.
pub(crate) fn version_key(capture: &str) -> Option<Version> {
    Version::parse(capture.strip_prefix('v').unwrap_or(capture)).ok()
}

/// `patterns` in order; the first pattern with any match wins, [`newest`] of its matches.
///
/// # Errors
/// [`DriverError::Transport`] when the blocking task did not run.
pub async fn glob_first(patterns: &[String], env: &ProbeEnv) -> Result<Option<PathBuf>> {
    let plans: Vec<(PathBuf, Vec<String>)> = patterns
        .iter()
        .filter_map(|pattern| expand(pattern, env))
        .collect();
    if plans.is_empty() {
        return Ok(None);
    }
    tokio::task::spawn_blocking(move || {
        plans
            .into_iter()
            .find_map(|(root, segments)| newest(walk(&root, &segments)))
    })
    .await
    .map_err(|err| DriverError::Transport(format!("the glob walk did not run: {err}")))
}

// ---------------------------------------------------------------------------------------------
// The credential tier (plan D59, D63)
// ---------------------------------------------------------------------------------------------

/// Which of `probe`'s declared tiers says this box holds a credential: files first, then variables.
///
/// Files go through [`glob_first`], so the grammar, the `spawn_blocking` walk, the "an unset
/// variable skips the candidate" rule and the `is_file` leaf filter are the walker's own — a
/// literal path is a pattern with no `*`, and the walk of it is one `stat`. That is deliberate:
/// the probe answers *whether* a credential is there and never opens it, so no token's text can
/// reach a log line, a `Debug`, or the `JSONB` column (`R-SEC-2`).
///
/// Files before variables per plan D63: the agent's own login flow writes the file, so on the box
/// this was written for it is the tier that fires, and an API key left over in the environment
/// should not mask the fact that the vendor's own credential is present.
///
/// Spawns nothing, reads nothing. `Ok(None)` when `probe` is `None` — a row that declares no block
/// is a row the milestone-5 rule still governs, which is a different answer from
/// [`CredentialTier::Absent`] ("declared, and none of them answered").
///
/// # Errors
/// [`DriverError::Transport`] when the file walk could not be scheduled, propagated rather than
/// swallowed for [`probe_tools`]'s reason: a runtime fault must not read as "no credential" and
/// turn a healthy authenticated box into an `unauthenticated` one.
pub async fn resolve_credential(
    probe: Option<&CredentialProbe>,
    env: &ProbeEnv,
) -> Result<Option<CredentialTier>> {
    let Some(probe) = probe else {
        return Ok(None);
    };

    if glob_first(&probe.files, env).await?.is_some() {
        return Ok(Some(CredentialTier::File));
    }
    // Set *and* non-empty: an exported-but-empty variable is how a shell profile unsets a key.
    if probe
        .env
        .iter()
        .any(|name| env.var(name).is_some_and(|value| !value.is_empty()))
    {
        return Ok(Some(CredentialTier::Env));
    }
    Ok(Some(CredentialTier::Absent))
}

// ---------------------------------------------------------------------------------------------
// Version capture (plan D47)
// ---------------------------------------------------------------------------------------------

/// The first capture group of `pattern` on the first line of `output` that matches, each line
/// `trim`med. A pattern with no group yields the whole match.
///
/// A pattern that does not compile, or that matches nothing, is `None` — "present, version
/// unknown". Neither is a probe failure: the seeds hold hand-written regexes and an agent that
/// prints a banner before its version is a documentation problem, not a broken box
/// (ANA-4 §4.6's tolerance rule).
#[must_use]
pub fn extract_version(output: &str, pattern: &str) -> Option<String> {
    let regex = match Regex::new(pattern) {
        Ok(regex) => regex,
        Err(error) => {
            warn!(%pattern, %error, "a version pattern does not compile; version unknown");
            return None;
        }
    };
    output.lines().find_map(|line| {
        let captures = regex.captures(line.trim())?;
        let matched = captures
            .get(1)
            .or_else(|| captures.get(0))
            .expect("a capture always has group 0");
        Some(matched.as_str().to_owned())
    })
}

/// Whether `version` is below `min`, by semver.
///
/// Either side unparsable is `false`: `agy`'s version string is undocumented, and refusing a box
/// because a vendor printed something semver does not read would be the probe inventing a
/// requirement the row did not state.
#[must_use]
pub fn below_min(version: &str, min: &str) -> bool {
    match (Version::parse(version), Version::parse(min)) {
        (Ok(version), Ok(min)) => version < min,
        _ => false,
    }
}

/// What one bounded one-shot child produced.
struct OneShot {
    /// Its stdout, capped at [`STDOUT_LIMIT`](crate::launch::STDOUT_LIMIT).
    stdout: String,
    /// How it exited.
    status: ExitStatus,
    /// The last lines it wrote to stderr.
    stderr: Vec<String>,
}

/// Runs one short-lived child to completion under `env.version_timeout`, or answers `None`.
///
/// Through [`launch::spawn`](crate::launch::spawn), so every child the probe starts gets the same
/// supervision a session does: `CREATE_NO_WINDOW` and a job object on Windows, a process group on
/// unix — which is what makes the timeout's `kill_tree` reach a shim's own child. The child is held
/// in a [`ChildGuard`] rather than a plain local, so a probe task that is **aborted** mid-await
/// (`AgentRuntime::shutdown`, `finish_background`'s limit) leaves no running process behind:
/// `Spawned` has no killing `Drop` of its own and tokio's `Child` does not kill on drop.
///
/// `None` covers every way this fails to produce output — the child did not start, the read or the
/// wait failed, or it hung — because every caller here treats all of those as "this tier found
/// nothing", which is the honest report for a box whose tool is absent or broken. `what` names the
/// child in the log.
async fn run_bounded(what: &str, launch: &ResolvedLaunch, env: &ProbeEnv) -> Option<OneShot> {
    let spawned = match crate::launch::spawn(launch, &env.cwd).await {
        Ok(spawned) => spawned,
        Err(error) => {
            debug!(child = what, %error, "the probe's child did not start");
            return None;
        }
    };
    let mut guard = ChildGuard::new(Some(spawned));

    let outcome = tokio::time::timeout(env.version_timeout, async {
        // The guard was handed a child two lines up and nothing takes it before here, so this is
        // a `let`-else rather than an `expect`: a panic path in a probe is a panic in whatever
        // task ran it, and this one has no reachable cause to be worth carrying.
        let Some(child) = guard.child_mut() else {
            return Err(DriverError::Transport("the guard held no child".to_owned()));
        };
        let stdout = child.read_stdout_to_end().await?;
        let status = child.wait().await?;
        Ok::<_, DriverError>((stdout, status))
    })
    .await;

    match outcome {
        Ok(Ok((stdout, status))) => {
            let stderr = guard.stderr_tail();
            // `wait` reaped it: the guard's `Drop` would only signal an already-dead process
            // group. It is also the one moment a sweep must **not** happen. Killing the group
            // here would catch a helper the tool reparented to init — but the leader has just
            // been reaped, so its pgid is free for reuse, and a late `killpg` can land on an
            // unrelated group. A sweep is only sound *before* the reap (the zombie holds the pgid
            // reserved), and that ordering is not expressible through `ChildWrapper::wait`, which
            // reaps to produce the status `npm root -g` needs. So: a detached helper of a probed
            // tool can outlive its probe. No tool in the seeds daemonises, and `run_session` has
            // the same asymmetry on its normal exit.
            guard.release();
            Some(OneShot {
                stdout,
                status,
                stderr,
            })
        }
        Ok(Err(error)) => {
            debug!(child = what, %error, "the probe's child failed");
            guard.kill_and_reap().await;
            None
        }
        Err(_) => {
            warn!(
                child = what,
                timeout = ?env.version_timeout,
                "the probe's child hung; killing its process tree"
            );
            guard.kill_and_reap().await;
            None
        }
    }
}

/// Runs `<path> <probe.args>` and extracts its version.
///
/// Through `run_bounded`: a `--version` child is supervised, capped and killed on timeout like
/// any other. On timeout the tree is killed and the version is unknown; the tool stays *found*,
/// because a hung `--version` says nothing about whether the binary is there.
///
/// The exit status is deliberately ignored: plenty of tools print their version and exit non-zero
/// (a `--version` that is really a usage error), and the pattern either matched or it did not.
pub async fn capture_version(path: &Path, probe: &VersionProbe, env: &ProbeEnv) -> Option<String> {
    let launch = ResolvedLaunch {
        command: path.to_string_lossy().into_owned(),
        args: probe.args.clone(),
        env: BTreeMap::new(),
    };
    let child = run_bounded(path.to_string_lossy().as_ref(), &launch, env).await?;

    // stdout first, then the stderr tail: `node --version` prints to stdout and plenty of tools
    // print theirs to stderr, and one pattern must find either.
    let mut output = child.stdout;
    for line in child.stderr {
        output.push('\n');
        output.push_str(&line);
    }
    extract_version(&output, &probe.pattern)
}

/// The `"version"` of `<package_dir>/package.json`, or `None`.
///
/// Read, never run: an npm package states its own version in a file, and spawning `node` to ask
/// it would cost a process per probe for a value already on disk.
pub async fn package_version(package_dir: &Path) -> Option<String> {
    let text = tokio::fs::read_to_string(package_dir.join("package.json"))
        .await
        .ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    document.get("version")?.as_str().map(ToOwned::to_owned)
}

// ---------------------------------------------------------------------------------------------
// The snapshot (plan D45)
// ---------------------------------------------------------------------------------------------

/// `agent_box.probe` (ANA-4 §4.6, plus plan D45's `source`). **Field order is key order**: the
/// column is read by a human in `psql` at least as often as by MOD-4's `probe->>'status'`, and
/// ANA-4 wrote the example in this order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSnapshot {
    /// `agent.transport` at probe time.
    pub transport: Transport,
    /// What `AgentDriver::start` spawns on this box; `None` when nothing resolved.
    pub resolved: Option<ResolvedLaunch>,
    /// `${name}` → version, for every **found** tool. A missing tool has no key; a found tool
    /// whose version could not be read has a `null` one.
    pub tools: BTreeMap<String, Option<String>>,
    /// Tier 2's answer; `None` when tier 2 did not run.
    pub handshake: Option<Handshake>,
    /// Plan D59: which tier of `discovery.credential` answered, never the value. `None` when the
    /// row declares no block, or when the probe ended before `agent.launch` parsed.
    ///
    /// Absent in a pre-milestone-6 document reads as `None`, which *is* the milestone-5 rule that
    /// row was written under — so an old row keeps its old meaning rather than acquiring a new one
    /// (blueprint H-6).
    #[serde(default)]
    pub credential: Option<CredentialTier>,
    /// Plan D50.
    pub status: ProbeStatus,
    /// On [`ProbeStatus::Failed`]: the failure text, line by line. The handshake's own error
    /// already carries the child's stderr after the reason, which is why one field holds both.
    pub stderr_tail: Option<Vec<String>>,
    /// Plan D45. Absent in a hand-written row reads as `probe`, which is the safe direction: a row
    /// nobody marked `manual` may be refreshed.
    #[serde(default)]
    pub source: ProbeSource,
}

impl ProbeSnapshot {
    /// `row.probe` parsed, or `None` when the column is `NULL` or does not parse.
    ///
    /// Never an error: the column is hand-editable and the Settings tab must still list a row
    /// whose snapshot it cannot read — the same tolerance `AcpDriver::from_row` grants
    /// `agent.settings`.
    #[must_use]
    pub fn from_row(row: &AgentBox) -> Option<Self> {
        serde_json::from_value(row.probe.clone()?).ok()
    }

    /// The column's document.
    ///
    /// # Panics
    /// Never: every field is a plain value or a `BTreeMap` with `String` keys, and
    /// `serde_json::to_value` fails only on a serialiser that errors or a non-string map key.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a probe snapshot serialises")
    }

    /// What a session should spawn instead of resolving the row again, or `None` (plan D58).
    ///
    /// This is the *only* place a glob tool's per-platform `args` survive: a [`ToolMap`] value is
    /// one string, so `tools::resolve` cannot carry `agy`'s `--uid=` and a chat that resolves for
    /// itself launches the adapter without it (blueprint H-3). The recording does carry them,
    /// because [`resolve_tool`] appended them when it walked the glob.
    ///
    /// Three rules, all readable from the document:
    ///
    /// - `source` is `probe`. A [`ProbeSource::Manual`] row is a human's path, and
    ///   `launch::resolve` already honours it through `agent.launch` itself — spawning the note
    ///   instead of the row would make the two disagree silently (blueprint H-5).
    /// - `status` is `ready` **or** `unauthenticated`. The `unauthenticated` half is not a
    ///   tolerance, it is the point: that status certifies the recording *is* the right binary —
    ///   it resolved, it spawned, it completed `initialize` — and that the only thing wrong with
    ///   this box is that nobody has logged in. Falling back to a second resolution there would
    ///   not produce a worse message, it would produce a **dead adapter**: on Linux the recording
    ///   is the only launch carrying `agy`'s `--uid=`, and without it the server exits in
    ///   `ChangeRootAndUser` before it ever reads stdin (blueprint H-4, and the live suite's
    ///   case 2 measured it). Spawning it is what lets the vendor's own `Authentication required`
    ///   reach the user, which since milestone 6 it does — `session/new` refuses, the connection
    ///   future carries the refusal home and `open_session` reports it verbatim
    ///   ([`crate::acp::open_session`]). `missing` resolved nothing, and `failed` recorded a
    ///   launch that did not answer — resolving again is the honest second attempt.
    /// - `resolved` is `Some`, which the first two do not imply on a hand-edited column.
    ///
    /// The fourth rule D58 states — the recorded `command` still exists — is filesystem I/O and is
    /// deliberately **not** here: this stays a pure read of the row, and the caller applies the
    /// check per session, where a task that may block already is ([`AcpDriver::launch_for`]).
    ///
    /// [`ToolMap`]: crate::launch::ToolMap
    /// [`AcpDriver::launch_for`]: crate::acp::AcpDriver::launch_for
    #[must_use]
    pub fn recorded_launch(&self) -> Option<&ResolvedLaunch> {
        if self.source != ProbeSource::Probe {
            return None;
        }
        if !matches!(
            self.status,
            ProbeStatus::Ready | ProbeStatus::Unauthenticated
        ) {
            return None;
        }
        self.resolved.as_ref()
    }
}

/// Tier 2's outcome mapped onto plan D50 as amended by D59: an empty `auth_methods` is
/// [`ProbeStatus::Ready`] whatever `credential` says; a non-empty one is `Ready` on
/// [`CredentialTier::File`] or [`CredentialTier::Env`], and [`ProbeStatus::Unauthenticated`] on
/// `None` (the row declared no block, so milestone 5's rule stands) or [`CredentialTier::Absent`].
///
/// The vendor's own auth flow is not `htui`'s to drive (ANA-4 §4.6), so a box that would have to
/// log in first is recorded as such rather than as a box that can run the agent. What D59 adds is
/// that some agents advertise their auth methods unconditionally — Antigravity lists four whether
/// or not you are logged in (ANA-4 §4.5) — so on those rows the list is an *offer*, not a demand,
/// and the credential the row points at is what settles it. Which rows those are is the registry's
/// business: this function is told the tier and never asks which agent it is looking at
/// (`R-AGT-5`).
#[must_use]
pub fn status_for(handshake: &Handshake, credential: Option<CredentialTier>) -> ProbeStatus {
    if handshake.auth_methods.is_empty() {
        return ProbeStatus::Ready;
    }
    match credential {
        Some(CredentialTier::File | CredentialTier::Env) => ProbeStatus::Ready,
        None | Some(CredentialTier::Absent) => ProbeStatus::Unauthenticated,
    }
}

// ---------------------------------------------------------------------------------------------
// Tier 2 (plan D49)
// ---------------------------------------------------------------------------------------------

/// Tier 2, as a seam: production spawns the resolved launch, a test answers from a duplex or from
/// a canned value.
///
/// A seam rather than a `cfg`: the status mapping of plan D50 has four outcomes and only one of
/// them spawns anything, so the cases that assert "**nothing** was spawned" need a tier 2 that
/// panics when it is called at all.
pub trait Tier2: Send + Sync {
    /// Spawns `launch` in `env.cwd`, completes `initialize`, kills the child, and answers.
    ///
    /// # Errors
    /// Whatever the spawn or the handshake failed with; [`probe_agent`] records the text as
    /// `stderr_tail` and the row as [`ProbeStatus::Failed`].
    fn handshake<'a>(
        &'a self,
        launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake>;
}

/// The production tier 2: [`launch::spawn`](crate::launch::spawn) →
/// [`AcpIo::from_spawned`] → [`crate::acp::handshake()`].
#[derive(Debug, Clone, Copy)]
pub struct SpawnTier2 {
    /// How long the agent gets to answer `initialize`. [`HANDSHAKE_TIMEOUT`] by default — the same
    /// window a chat start allows, because it is the same question being asked.
    pub timeout: Duration,
}

impl Default for SpawnTier2 {
    fn default() -> Self {
        Self {
            timeout: HANDSHAKE_TIMEOUT,
        }
    }
}

impl Tier2 for SpawnTier2 {
    fn handshake<'a>(
        &'a self,
        launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            let spawned = crate::launch::spawn(launch, &env.cwd).await?;
            let io = AcpIo::from_spawned(spawned)?;
            crate::acp::handshake(io, settings, self.timeout).await
        })
    }
}

// ---------------------------------------------------------------------------------------------
// `probe_agent` (plan D50, D51)
// ---------------------------------------------------------------------------------------------

/// What one probe run is told about the box and the clock.
#[derive(Debug, Clone)]
pub struct ProbeContext {
    /// The box.
    pub env: ProbeEnv,
    /// `probed_at`, `updated_at`, and what a handshake-less snapshot is stamped with. Injected so
    /// a test can assert the row carries *this* instant rather than "roughly now".
    pub now: DateTime<Utc>,
}

/// What [`probe_agent`] decided.
#[derive(Debug, Clone, PartialEq)]
#[must_use = "a dropped outcome is a probe whose row was never written"]
#[expect(
    clippy::large_enum_variant,
    reason = "one of these is built per registry row and moved straight into the upsert; boxing \
              the row would buy an allocation per probed agent to save 248 bytes on a value \
              nothing keeps"
)]
pub enum ProbeOutcome {
    /// Write this row (`WriteStore::upsert_agent_box`).
    Row(AgentBox),
    /// Plan D51: the stored row is `source: manual` and this probe **resolved nothing**. Write
    /// nothing — not even `probed_at`, because a bump on a no-op would make a hand-written row look
    /// freshly verified, which is the one reading the rule exists to prevent.
    Kept {
        /// For the caller's log.
        reason: &'static str,
    },
}

/// The `reason` of a [`ProbeOutcome::Kept`].
const MANUAL_KEPT: &str = "manual entry kept: the probe resolved nothing";

/// One registry row on this box, start to finish (plan D50, in this order):
///
/// 1. `agent.launch` parses, or the row is `failed` with the serde text and **nothing is spawned**;
///    `agent.settings` falls back to the documented defaults, as the registry's own rule has it.
/// 2. [`resolve_credential`] over `launch.discovery.credential` (plan D59). First, because it needs
///    only the parsed launch and `ctx.env` and costs a `stat` and a map lookup: running it here
///    means **every** snapshot with a parsed launch records the tier, so a `missing` box still says
///    whether it holds a token. A transport fault is `failed`, as step 3's is.
/// 3. [`probe_tools`]. A transport fault is `failed`, not `missing`: a box whose lookup could not
///    run has not been shown to be missing anything.
/// 4. An incomplete report is `missing`, `resolved: None`, and **nothing is spawned**.
/// 5. [`launch::resolve`](crate::launch::resolve) over the report's tool map; an unresolved
///    placeholder — a row that names a `${tool}` its `discovery` never declared — is `missing`
///    too. The found tools' platform `args` are then appended (ANA-4 §4.6's `--uid=` rule).
/// 6. Tier 2 runs only for an `acp` row whose `discovery` asks for a handshake. A `cli` row has no
///    `initialize` to complete, so resolution is the whole probe and the status is `ready`.
/// 7. The handshake's answer and step 2's tier map through [`status_for`]; a handshake failure is
///    `failed` with the error text line by line.
/// 8. A snapshot with **no `resolved`** over a stored `source: manual` row is
///    [`ProbeOutcome::Kept`]. ANA-4 §4.6 says "a probe that finds nothing", which is `missing` *and*
///    the three `failed`s that never got as far as a launch — an `agent.launch` that does not parse
///    (step 1), a credential fault (step 2) and a `probe_tools` transport fault (step 3). Anything
///    the probe *did* resolve refreshes a manual row like any other, including a `failed`
///    handshake: that one found the binary and is a fact about this box. A refreshed row says
///    `source: probe`.
/// 9. Otherwise [`agent_box_row`].
///
/// Nothing here writes a store row: the caller owns the `Writer`, and `R-NF-3` keeps that caller
/// off the UI task.
pub async fn probe_agent(
    agent: &Agent,
    box_id: BoxId,
    existing: Option<&AgentBox>,
    ctx: &ProbeContext,
    tier2: &dyn Tier2,
) -> ProbeOutcome {
    let snapshot = snapshot_for(agent, ctx, tier2).await;
    // `resolved.is_none()`, not `status == Missing`: ANA-4:795-798's rule is "a probe that finds
    // nothing", and two `failed` outcomes also find nothing — an `agent.launch` that does not parse
    // and a `probe_tools` transport fault, neither of which learned anything about this box. Keying
    // on the status would let either of them overwrite a hand-written row with
    // `enabled = false, source: probe`. A `failed` that *did* resolve refreshes, as before.
    if snapshot.resolved.is_none()
        && existing
            .and_then(ProbeSnapshot::from_row)
            .is_some_and(|stored| stored.source == ProbeSource::Manual)
    {
        return ProbeOutcome::Kept {
            reason: MANUAL_KEPT,
        };
    }
    ProbeOutcome::Row(agent_box_row(agent, box_id, &snapshot, ctx.now))
}

/// Steps 1–7 of [`probe_agent`]: everything that decides the snapshot, with no knowledge of what
/// was stored before.
async fn snapshot_for(agent: &Agent, ctx: &ProbeContext, tier2: &dyn Tier2) -> ProbeSnapshot {
    // `credential` is a parameter rather than a capture: the step-1 failure below happens before
    // there is a launch to read one from, and that snapshot has to say `None` rather than guess.
    let blank = |status: ProbeStatus, tools: BTreeMap<String, Option<String>>, credential, tail| {
        ProbeSnapshot {
            transport: agent.transport,
            resolved: None,
            tools,
            handshake: None,
            credential,
            status,
            stderr_tail: tail,
            source: ProbeSource::Probe,
        }
    };

    let launch: AgentLaunch = match serde_json::from_value(agent.launch.clone()) {
        Ok(launch) => launch,
        Err(error) => {
            warn!(agent = agent.name, %error, "agent.launch does not parse; nothing is spawned");
            return blank(
                ProbeStatus::Failed,
                BTreeMap::new(),
                None,
                Some(lines(&format!("agent.launch does not parse: {error}"))),
            );
        }
    };
    let settings: AgentSettings =
        serde_json::from_value(agent.settings.clone()).unwrap_or_default();

    let credential = match resolve_credential(
        launch
            .discovery
            .as_ref()
            .and_then(|discovery| discovery.credential.as_ref()),
        &ctx.env,
    )
    .await
    {
        Ok(credential) => credential,
        Err(error) => {
            return blank(
                ProbeStatus::Failed,
                BTreeMap::new(),
                None,
                Some(lines(&error.to_string())),
            );
        }
    };
    if let Some(tier) = credential {
        // The tier, never the path and never the value: this line lands in the same log file the
        // maintainer pastes into an issue.
        debug!(agent = agent.name, tier = %tier, "a credential tier answered");
    }

    let report = match probe_tools(launch.discovery.as_ref(), &ctx.env).await {
        Ok(report) => report,
        Err(error) => {
            return blank(
                ProbeStatus::Failed,
                BTreeMap::new(),
                credential,
                Some(lines(&error.to_string())),
            );
        }
    };
    if !report.is_complete() {
        debug!(
            agent = agent.name,
            missing = ?report.missing,
            "a required tool resolved nowhere; nothing is spawned"
        );
        return blank(ProbeStatus::Missing, report.versions(), credential, None);
    }

    let mut resolved = match crate::launch::resolve(&launch, &report.tool_map()) {
        Ok(resolved) => resolved,
        Err(error) => {
            // A placeholder with no `discovery` entry: the row names a tool it never told the
            // probe how to find, which from this box's side is the same fact as "not installed".
            debug!(agent = agent.name, %error, "the launch names a tool the discovery does not");
            return blank(ProbeStatus::Missing, report.versions(), credential, None);
        }
    };
    resolved.args.extend(report.extra_args());

    let tier2_runs = agent.transport == Transport::Acp
        && launch
            .discovery
            .as_ref()
            .is_none_or(|discovery| discovery.handshake);
    if !tier2_runs {
        return ProbeSnapshot {
            transport: agent.transport,
            resolved: Some(resolved),
            tools: report.versions(),
            handshake: None,
            credential,
            status: ProbeStatus::Ready,
            stderr_tail: None,
            source: ProbeSource::Probe,
        };
    }

    match tier2.handshake(&resolved, &settings.acp, &ctx.env).await {
        Ok(handshake) => ProbeSnapshot {
            transport: agent.transport,
            resolved: Some(resolved),
            tools: report.versions(),
            status: status_for(&handshake, credential),
            handshake: Some(handshake),
            credential,
            stderr_tail: None,
            source: ProbeSource::Probe,
        },
        Err(error) => ProbeSnapshot {
            transport: agent.transport,
            resolved: Some(resolved),
            tools: report.versions(),
            handshake: None,
            credential,
            status: ProbeStatus::Failed,
            // `DriverError`'s `Display` prefixes its variant ("agent transport error: …"), and
            // this field is documented as what the child said. `handshake` returns exactly one
            // variant, so unwrapping it here keeps a Rust type name out of a JSONB column the
            // Settings tab renders verbatim.
            stderr_tail: Some(lines(&match &error {
                DriverError::Transport(text) => text.clone(),
                other => other.to_string(),
            })),
            source: ProbeSource::Probe,
        },
    }
}

/// A failure text as the snapshot holds it: one array element per line, so the Settings tab can
/// render it without re-splitting a blob.
fn lines(text: &str) -> Vec<String> {
    text.lines().map(ToOwned::to_owned).collect()
}

/// The columnar projection of `snapshot` (ANA-4 §4.6): the four fields that have been in the
/// schema since `0001`, beside the document itself.
///
/// `version` is the **handshake's** `agent_version` when tier 2 ran, which is what ANA-4 asks for;
/// with no handshake it falls back to the tool that shares the agent's name (`claude`'s own CLI
/// version for the `claude` row), and to `None` when neither answered.
///
/// `quota` and `quota_at` are `None`, and this function takes no stored row to read them from.
/// The probe owns neither field and, since MOD-2 milestone 7 (plan D74), no longer carries them
/// forward either: `WriteStore::upsert_agent_box` cannot write those two columns at all, so there
/// is nothing to carry them *to*. `WriteStore::set_agent_box_quota` is their only writer, and the
/// values it wrote stay in the row this projection is about to update. Carrying a value into a
/// statement that discards it is how a latch got lost - the probe read the row at chat start and
/// wrote it back seconds later, throwing away every latch in between.
///
/// **MOD-7 inherits this**: box registration writes quota through `set_agent_box_quota` like
/// everything else, not through the row.
#[must_use]
pub fn agent_box_row(
    agent: &Agent,
    box_id: BoxId,
    snapshot: &ProbeSnapshot,
    now: DateTime<Utc>,
) -> AgentBox {
    AgentBox {
        agent_id: agent.id,
        box_id,
        enabled: snapshot.status == ProbeStatus::Ready,
        version: snapshot
            .handshake
            .as_ref()
            .and_then(|handshake| handshake.agent_version.clone())
            .or_else(|| snapshot.tools.get(&agent.name).cloned().flatten()),
        path: snapshot
            .resolved
            .as_ref()
            .map(|resolved| resolved.command.clone()),
        probed_at: Some(now),
        quota: None,
        quota_at: None,
        updated_at: now,
        probe: Some(snapshot.to_value()),
    }
}
