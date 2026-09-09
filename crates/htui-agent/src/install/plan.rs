//! The pre-flight (plan MOD-20 D11, D13, blueprint P-9): one registry read and one `HEAD`.
//!
//! **No archive body byte is requested here.** `R-AGT-10` says the user is told what will be
//! fetched *before* the download, and D13 makes that precise: the pre-flight issues exactly one
//! `GET` of the registry document — or none at all, from a cache the CDN's `max-age` still calls
//! current — and one `HEAD` of the archive URL, which transfers headers and no adapter. Anything
//! more would be a request the user has not consented to, and `tests/install.rs` asserts the
//! fixture server saw exactly those two lines.
//!
//! The order of the checks is the order of their cost: everything that can be refused from the
//! row alone is refused before the network is touched, everything that can be refused from the
//! document is refused before the `HEAD`, and the disk arithmetic is last because it is the only
//! one that needs the size.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use htui_core::model::Agent;
use tracing::warn;

use super::archive::ArchiveFormat;
use super::layout::Layout;
use super::manifest::Manifest;
use super::registry::{RegistryError, RegistrySource, read_registry, registry_url};
use super::{InstallPlan, Installer, ManualSteps, PlanError};
use crate::launch::{AgentLaunch, InstallSource, ToolProbe};
use crate::probe::{INSTALL_ROOT_VAR, ProbeEnv, install_root, version_key};
use crate::tools::env_override_key;

/// The pre-flight, in plan D13's order.
///
/// 1. The row: `discovery.install` ([`PlanError::NoSource`]), `discovery.tools[tool]` a glob
///    ([`PlanError::NoGlobTool`]), an install root ([`PlanError::NoRoot`]). None of this touches
///    the network.
/// 2. The document: the registry read, then the entry ([`PlanError::UnknownId`]), this platform
///    ([`PlanError::NotAvailable`]), the archive shape ([`PlanError::Unsupported`]) and the `cmd`
///    ([`PlanError::BadCmd`]).
/// 3. The disk as it already is: a version directory that outranks what is offered is refused
///    here ([`PlanError::Outranked`], blueprint P-9), before a byte is spent on a download the
///    post-promote check would roll back.
/// 4. The manifest: whether these terms were already accepted, and whether this exact version was
///    installed before (plan D17).
/// 5. The `HEAD`. A failure is *size unknown*, stated in the consent text — not a refusal.
/// 6. D11's arithmetic, when both numbers are known ([`PlanError::Disk`]).
/// 7. Whether the registry's arguments differ from the row's, which the consent text says out
///    loud rather than resolving silently in the registry's favour.
///
/// # Errors
///
/// [`PlanError`], one variant per thing the user can act on.
pub async fn plan(
    installer: &Installer,
    agent: &Agent,
    env: &ProbeEnv,
    now: DateTime<Utc>,
) -> Result<InstallPlan, PlanError> {
    let config = installer.config();

    // 1. The row alone.
    let launch: AgentLaunch = serde_json::from_value(agent.launch.clone()).map_err(|error| {
        // A row whose launch document does not parse declares nothing, including a source. The
        // probe reaches the same conclusion for the same document and says so in its own log.
        warn!(agent = agent.name, %error, "agent.launch does not parse; there is nothing to install from");
        PlanError::NoSource {
            agent: agent.name.clone(),
        }
    })?;
    let no_source = || PlanError::NoSource {
        agent: agent.name.clone(),
    };
    let discovery = launch.discovery.as_ref().ok_or_else(no_source)?;
    let declared = discovery.install.as_ref().ok_or_else(no_source)?;
    // One source today. The `match` is what a second one has to be answered for, rather than a
    // `_` arm that would silently install from the wrong registry.
    match declared.source {
        InstallSource::AcpRegistry => {}
    }
    let Some(ToolProbe::Glob { platform, .. }) = discovery.tools.get(&declared.tool) else {
        return Err(PlanError::NoGlobTool {
            agent: agent.name.clone(),
            tool: declared.tool.clone(),
        });
    };
    let root = install_root(env).ok_or(PlanError::NoRoot {
        var: INSTALL_ROOT_VAR,
    })?;

    // 2. The document.
    let (document, source) = read_registry(installer.http(), config, &root, now)
        .await
        .map_err(|error| match error {
            RegistryError::Malformed(message) => PlanError::Malformed { message },
            RegistryError::Network(message) => PlanError::Network {
                message,
                manual: Box::new(ManualSteps {
                    registry_url: registry_url(&config.registry_base),
                    id: declared.id.clone(),
                    platform: env.platform.clone(),
                    version: None,
                    unpack_into: root.join(&declared.id).join("<version>"),
                    cmd: None,
                    override_key: env_override_key(&declared.tool),
                }),
            },
        })?;
    let entry = document
        .agent(&declared.id)
        .ok_or_else(|| PlanError::UnknownId {
            id: declared.id.clone(),
        })?;
    let binary = entry
        .distribution
        .binary
        .get(&env.platform)
        .ok_or_else(|| PlanError::NotAvailable {
            id: entry.id.clone(),
            version: entry.version.clone(),
            platform: env.platform.clone(),
        })?;
    let format = ArchiveFormat::for_url(&binary.archive).ok_or_else(|| PlanError::Unsupported {
        url: binary.archive.clone(),
    })?;
    // Checked here and again at run time: the plan is data the user could have edited between
    // `i` and `y` (hazard H-16).
    cmd_relative(&binary.cmd)?;

    // 3. The disk as it already is.
    let layout = Layout::new(root.clone());
    let existing_versions = layout.existing_versions(&entry.id).await;
    if let Some(offered) = version_key(&entry.version) {
        for installed in &existing_versions {
            if version_key(installed).is_some_and(|found| found > offered) {
                return Err(PlanError::Outranked {
                    installed: installed.clone(),
                    offered: entry.version.clone(),
                    path: layout.version_dir(&entry.id, installed),
                });
            }
        }
    }

    // 4. The manifest.
    let manifest = Manifest::load(&layout.manifest(&entry.id)).await;
    let consent = manifest
        .consent_covers(entry.license.as_deref(), entry.license_url.as_deref())
        .then(|| manifest.consent.clone())
        .flatten();
    let recorded = manifest.installs.get(&entry.version).cloned();

    // 5. The `HEAD`.
    let content_length = match installer.http().head(&binary.archive).await {
        Ok(info) if (200..300).contains(&info.status) => info.content_length,
        Ok(info) => {
            warn!(
                status = info.status,
                "the archive HEAD did not answer 2xx; the size stays unknown"
            );
            None
        }
        Err(error) => {
            warn!(
                message = error.message,
                "the archive HEAD failed; the size stays unknown"
            );
            None
        }
    };

    // 6. D11. The injected number wins so a test can state a full disk without owning one; with
    // none, the filesystem answers for itself.
    let available_bytes = match config.disk_override {
        Some(injected) => Some(injected),
        None => available_space(root.clone()).await,
    };
    let need_bytes = content_length.map(|size| size.saturating_mul(config.headroom_factor));
    if let (Some(available), Some(need)) = (available_bytes, need_bytes)
        && available < need
    {
        return Err(PlanError::Disk {
            need,
            available,
            factor: config.headroom_factor,
            root,
        });
    }

    // 7. The arguments the row would append on this platform.
    let row_args = platform
        .get(&env.platform)
        .map_or(&[][..], |glob| glob.args.as_slice());

    Ok(InstallPlan {
        agent_id: agent.id,
        agent_name: agent.name.clone(),
        tool: declared.tool.clone(),
        registry_id: entry.id.clone(),
        registry_name: entry.name.clone(),
        version: entry.version.clone(),
        platform: env.platform.clone(),
        archive_url: binary.archive.clone(),
        format,
        content_length,
        sha256: binary.sha256.clone(),
        cmd: binary.cmd.clone(),
        args: binary.args.clone(),
        env: binary.env.clone(),
        license: entry.license.clone(),
        license_url: entry.license_url.clone(),
        install_dir: layout.version_dir(&entry.id, &entry.version),
        root,
        existing_versions,
        available_bytes,
        need_bytes,
        args_differ: row_args != binary.args.as_slice(),
        consent,
        recorded,
        registry_cached_age_secs: match source {
            RegistrySource::Fresh => None,
            RegistrySource::Cached { age } => Some(age.as_secs()),
        },
        planned_at: now,
    })
}

/// `./x` → `x`; refuses an absolute path, a `..`, a drive prefix and an empty result.
///
/// Both spellings the registry uses are relative — `./name` on the unix platforms, a bare
/// `name.exe` on some Windows ones — and this is what makes accepting both safe. It splits on
/// **both** separators rather than trusting `Path::components`, because a backslash is an
/// ordinary character in a unix path and `sub\..\..\etc` would otherwise pass on Linux and
/// escape on Windows.
///
/// # Errors
///
/// [`PlanError::BadCmd`], naming the `cmd` as the registry spells it.
pub(crate) fn cmd_relative(cmd: &str) -> Result<PathBuf, PlanError> {
    let bad = || PlanError::BadCmd {
        cmd: cmd.to_owned(),
    };
    if cmd.is_empty() || cmd.starts_with('/') || cmd.starts_with('\\') {
        return Err(bad());
    }
    // `C:\...` and `C:x` alike: anything with a drive letter is naming a volume, not an entry.
    if cmd.chars().nth(1) == Some(':') {
        return Err(bad());
    }
    let mut relative = PathBuf::new();
    for part in cmd.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(bad()),
            name => relative.push(name),
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(bad());
    }
    Ok(relative)
}

/// Free bytes on the filesystem that holds `root` (plan D11), or `None` when nothing can be
/// asked.
///
/// The walk up the ancestors is what makes this useful on the install that needs it most: on a
/// first-ever `i` the root does not exist yet, and asking about a path that is not there answers
/// an error rather than a number. The nearest ancestor that *does* exist is on the same
/// filesystem the root will be created on — barring a mount point conjured in between, which
/// would cost an over-estimate and not a wrong install.
///
/// `fs4::available_space` is the space a non-privileged user may actually take, not the total
/// free space, which is the number D11's arithmetic is about. It is a blocking `statvfs`, so it
/// runs where blocking calls belong.
async fn available_space(root: PathBuf) -> Option<u64> {
    tokio::task::spawn_blocking(move || {
        root.ancestors()
            .find_map(|path| fs4::available_space(path).ok())
    })
    .await
    .ok()
    .flatten()
}

impl InstallPlan {
    /// Every consent line, in plan D13's order, ready to draw.
    ///
    /// The wording lives here rather than in the Settings section for two reasons: this crate can
    /// test it offline, and the pane then composes nothing — what the user reads and what the
    /// installer executes come from the same value.
    #[must_use]
    pub fn consent_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "install {} {} for {} ({})",
                self.registry_name, self.version, self.agent_name, self.platform
            ),
            match self.content_length {
                Some(size) => format!("from    {} — {}", self.archive_url, human_bytes(size)),
                None => format!("from    {} — size unknown", self.archive_url),
            },
            format!("into    {}", self.install_dir.display()),
            self.licence_line(),
            format!("digest  {}", self.digest_sentence()),
            self.disk_line(),
            if self.existing_versions.is_empty() {
                "existing none".to_owned()
            } else {
                format!(
                    "existing {} replaced and deleted once the new version probes usable",
                    self.existing_versions.join(", ")
                )
            },
        ];
        if self.args_differ {
            lines.push(format!(
                "args    the registry's args differ from the row's: {}",
                self.args.join(" ")
            ));
        }
        if let Some(age) = self.registry_cached_age_secs {
            lines.push(format!("registry cached {}", human_age(age)));
        }
        if let Some(record) = &self.recorded {
            lines.push(format!(
                "this version was installed before with digest {}",
                record.sha256
            ));
        }
        lines
    }

    /// The digest sentence, in the two wordings plan D13 fixes.
    ///
    /// Eight of the registry's forty entries publish no `sha256`, and the honest sentence for
    /// those says what `htui` will do instead — record what it received — rather than implying a
    /// check that is not happening.
    #[must_use]
    pub fn digest_sentence(&self) -> &'static str {
        if self.sha256.is_some() {
            "sha256 published: verified before unpacking"
        } else {
            "none published: htui cannot verify this download and will record what it receives"
        }
    }

    /// Plan D20's fallback, derived from this plan.
    #[must_use]
    pub fn manual_steps(&self, registry_base: &str) -> ManualSteps {
        ManualSteps {
            registry_url: registry_url(registry_base),
            id: self.registry_id.clone(),
            platform: self.platform.clone(),
            version: Some(self.version.clone()),
            unpack_into: self.install_dir.clone(),
            cmd: cmd_relative(&self.cmd)
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            override_key: env_override_key(&self.tool),
        }
    }

    /// `licence <id> — <url> — <what y means>`, with every part that is absent left out.
    fn licence_line(&self) -> String {
        let mut line = format!(
            "licence {}",
            self.license.as_deref().unwrap_or("not stated")
        );
        if let Some(url) = &self.license_url {
            line.push_str(" — ");
            line.push_str(url);
        }
        match &self.consent {
            Some(consent) => {
                line.push_str(&format!(
                    " — accepted on {}",
                    consent.accepted_at.format("%Y-%m-%d")
                ));
            }
            None => line.push_str(" — y accepts these terms"),
        }
        line
    }

    /// `disk <free> free, <need> needed (4× the archive)`, degrading to what is known.
    fn disk_line(&self) -> String {
        let factor = self.headroom_factor();
        match (self.available_bytes, self.need_bytes) {
            (Some(available), Some(need)) => format!(
                "disk    {} free, {} needed ({factor}× the archive)",
                human_bytes(available),
                human_bytes(need),
            ),
            (None, Some(need)) => format!(
                "disk    {} needed ({factor}× the archive), free space unknown",
                human_bytes(need),
            ),
            _ => "disk    need unknown".to_owned(),
        }
    }

    /// The factor `need_bytes` was actually multiplied by, recovered rather than assumed.
    ///
    /// `need_bytes` is `content_length × config.headroom_factor`, and a box whose config carries
    /// anything but [`DISK_HEADROOM_FACTOR`](super::DISK_HEADROOM_FACTOR) would otherwise be shown
    /// a line whose two numbers do not multiply out — the consent pane quietly lying about the
    /// arithmetic it just refused or allowed an install on. The plan itself is a wire type shared
    /// with the UI crate, so the factor is derived from the two numbers that are already in it
    /// rather than added as a field the section would have to fill in.
    fn headroom_factor(&self) -> u64 {
        match (self.content_length, self.need_bytes) {
            (Some(size), Some(need)) if size > 0 => need / size,
            _ => super::DISK_HEADROOM_FACTOR,
        }
    }
}

impl ManualSteps {
    /// The steps as lines, ready to draw under the table.
    ///
    /// Every word is derived: the registry URL from the config, the id and the tool from the row,
    /// the platform from the probe, the directory from the install root. That is what lets this
    /// replace the hand-written install paragraph in `README.md` without inheriting its defect —
    /// a pinned URL and a pinned version that go stale the next release.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let version = self.version.as_deref().unwrap_or("<version>");
        let mut lines = vec![
            format!(
                "1. open {} and find the entry `{}`",
                self.registry_url, self.id
            ),
            format!(
                "2. take distribution.binary[\"{}\"].archive — that is {} {}",
                self.platform, self.id, version
            ),
            format!(
                "3. unpack the whole archive into {}",
                self.unpack_into.display()
            ),
        ];
        if let Some(cmd) = &self.cmd {
            lines.push(format!("4. make {cmd} executable inside that directory"));
        }
        lines.push(format!(
            "or: set {} to the command's absolute path and skip all of it",
            self.override_key
        ));
        lines
    }
}

/// A byte count as the consent pane says it.
fn human_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} bytes")
    }
}

/// A cached document's age, coarse on purpose: the user needs "days old", not seconds.
fn human_age(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if seconds < MINUTE {
        format!("{seconds}s")
    } else if seconds < HOUR {
        format!("{}m", seconds / MINUTE)
    } else if seconds < DAY {
        format!("{}h", seconds / HOUR)
    } else {
        format!("{}d", seconds / DAY)
    }
}
