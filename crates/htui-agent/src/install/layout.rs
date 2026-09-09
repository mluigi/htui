//! Every path under the install root, and the four moves an install makes between them (plan
//! MOD-20 D16, blueprint P-10).
//!
//! One composer is what turns *staging is never walkable* from a habit at six call sites into a
//! fact about a type (hazard H-3). `.staging/` and `.registry/` are **siblings** of `<id>/`, and a
//! seed's glob has its literal root at `<root>/<id>` — the expander puts every leading literal
//! segment into the root it walks from — so nothing this module writes mid-flight can ever be
//! resolved as an adapter, however far the unpack got before it stopped.
//!
//! The moves, in the order an install makes them: [`sweep`](Layout::sweep) clears what an earlier
//! run abandoned, [`set_aside`](Layout::set_aside) takes a same-version tree out of the glob's
//! reach, [`promote`](Layout::promote) renames one directory into place, and
//! [`retain_only`](Layout::retain_only) deletes what the new version replaced — but only once the
//! probe has said the new one works.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::warn;

use super::InstallError;

/// The directory every in-flight artefact lives under: a sibling of `<id>/`, and a dot-directory
/// so that even a glob rooted one level higher would skip it.
const STAGING: &str = ".staging";

/// What a set-aside version directory is renamed to (blueprint P-10).
///
/// The suffix is the whole mechanism: without it, an abort between the set-aside and the promote
/// would leave the *working* version as ordinary staging residue for the next sweep to delete,
/// and plan D3's "a failed install never costs the working one" would be broken by the sweep
/// itself.
const PREVIOUS: &str = ".previous";

/// What a downloaded archive is called before it is unpacked.
const ARCHIVE: &str = ".archive";

/// How many times [`promote`](Layout::promote) retries a refused rename.
///
/// Shared with the rollback in `run.rs`, which undoes a promote for the same reason and against
/// the same Windows behaviour: whatever was holding the tree open while it was being renamed in is
/// still holding it while it is being removed again (plan D21).
pub(crate) const PROMOTE_ATTEMPTS: u32 = 3;

/// The first backoff between those attempts; it doubles (200 ms, 400 ms, 800 ms).
pub(crate) const PROMOTE_BACKOFF: Duration = Duration::from_millis(200);

/// Every path under one install root, composed in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// The layout under one install root.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The install root itself.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/.staging/` — where everything in flight lives, and nothing a glob can reach.
    #[must_use]
    pub fn staging(&self) -> PathBuf {
        self.root.join(STAGING)
    }

    /// `<root>/<id>/`.
    #[must_use]
    pub fn agent_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// `<root>/<id>/<version>/`.
    #[must_use]
    pub fn version_dir(&self, id: &str, version: &str) -> PathBuf {
        self.agent_dir(id).join(version)
    }

    /// `<root>/<id>/manifest.json`.
    #[must_use]
    pub fn manifest(&self, id: &str) -> PathBuf {
        self.agent_dir(id).join("manifest.json")
    }

    /// `<root>/.staging/<id>-<version>-<nonce>.archive` — the downloaded file.
    #[must_use]
    pub fn staging_archive(&self, id: &str, version: &str, nonce: &str) -> PathBuf {
        self.staging()
            .join(format!("{id}-{version}-{nonce}{ARCHIVE}"))
    }

    /// `<root>/.staging/<id>-<version>-<nonce>/` — the tree the archive unpacks into.
    #[must_use]
    pub fn staging_tree(&self, id: &str, version: &str, nonce: &str) -> PathBuf {
        self.staging().join(format!("{id}-{version}-{nonce}"))
    }

    /// `<root>/.staging/<id>-<version>-<nonce>.previous/` — a same-version tree taken out of the
    /// way of the promote, and put back if the promote or the re-probe goes wrong.
    #[must_use]
    pub fn staging_previous(&self, id: &str, version: &str, nonce: &str) -> PathBuf {
        self.staging()
            .join(format!("{id}-{version}-{nonce}{PREVIOUS}"))
    }

    /// Directory names under `<root>/<id>/`, `manifest.json` and any other file excluded.
    ///
    /// An absent directory and an unreadable one are both "no versions", and this is not a
    /// `Result` for that reason: the pre-flight of a first-ever install runs before anything has
    /// created the root, and a permissions problem is something the install itself reports far
    /// more usefully than a refusal here would.
    pub async fn existing_versions(&self, id: &str) -> Vec<String> {
        let mut versions = Vec::new();
        let Ok(mut entries) = tokio::fs::read_dir(self.agent_dir(id)).await else {
            return versions;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
                versions.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        versions.sort();
        versions
    }

    /// Plan D16 and blueprint P-10: what the last run left behind, restored or removed.
    ///
    /// Two rules, and the order between them is the point:
    ///
    /// 1. A `.previous` whose `<root>/<id>/<version>/` is **absent** is a working version an abort
    ///    stranded between the set-aside and the promote. It is restored **at once**, whatever its
    ///    age — waiting an hour to give a box back the adapter it had would be the sweep punishing
    ///    the user for a shutdown.
    /// 2. Everything else — a partial download, an abandoned tree, a `.previous` whose version
    ///    directory is there because the promote did happen — is removed once it is older than
    ///    `max_age`. Younger than that it is left alone: another install's own artefacts are
    ///    minutes old, and deleting them would be this installer sabotaging itself.
    ///
    /// A `.previous` whose name cannot be split back into an id and a version is never deleted.
    /// It may be a working tree, and the cost of keeping a directory nobody claims is a directory;
    /// the cost of the other mistake is the adapter.
    ///
    /// An absent `.staging/` is an empty report, not an error: the first install on a box has
    /// nothing to sweep.
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when the directory can be read but an entry inside it cannot be moved
    /// or removed — which is the case where carrying on would silently promote over residue.
    pub async fn sweep(
        &self,
        max_age: Duration,
        now: SystemTime,
    ) -> Result<SweepReport, InstallError> {
        let mut report = SweepReport::default();
        let staging = self.staging();
        let Ok(mut entries) = tokio::fs::read_dir(&staging).await else {
            return Ok(report);
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| io("read", &staging, &error))?
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            if let Some(stem) = name.strip_suffix(PREVIOUS) {
                match self.split_name(stem).await {
                    Some((id, version)) if !self.exists(&self.version_dir(&id, &version)).await => {
                        self.restore(&path, &id, &version).await?;
                        report.restored.push(self.version_dir(&id, &version));
                        continue;
                    }
                    Some(_) => {}
                    None => {
                        warn!(
                            entry = %path.display(),
                            "a set-aside version whose name does not parse is left alone"
                        );
                        continue;
                    }
                }
            }

            if age_of(&entry, now).await < max_age {
                continue;
            }
            remove_any(&path).await?;
            report.removed.push(path);
        }
        Ok(report)
    }

    /// Plan D16: a same-version re-install takes the existing tree out of the way first.
    ///
    /// `Ok(None)` when there is nothing there — the ordinary first install. Otherwise the tree is
    /// renamed under `.staging/` with the `.previous` suffix, which is both out of the glob's
    /// reach (hazard H-3) and recognisable to [`sweep`](Self::sweep) as something to give back
    /// rather than something to delete (hazard H-7).
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when the rename fails.
    pub async fn set_aside(
        &self,
        id: &str,
        version: &str,
        nonce: &str,
    ) -> Result<Option<PathBuf>, InstallError> {
        let from = self.version_dir(id, version);
        if !self.exists(&from).await {
            return Ok(None);
        }
        let to = self.staging_previous(id, version, nonce);
        self.make_staging().await?;
        tokio::fs::rename(&from, &to)
            .await
            .map_err(|error| io("set aside", &from, &error))?;
        Ok(Some(to))
    }

    /// One rename, and the install is committed (plan D16).
    ///
    /// The target is checked first (hazard H-20): a `rename` onto an existing directory is
    /// `ENOTEMPTY` on unix, a silent replacement when the target is empty, and an outright refusal
    /// on Windows — three behaviours where the caller wants one, and [`set_aside`](Self::set_aside)
    /// is what removes the case. If the target is still there, that is a bug in the caller and the
    /// error names the directory in the way.
    ///
    /// The retry is ordinary code with a Windows-only reason (plan D21): Defender opens a freshly
    /// written `.exe` to scan it, and the rename of the directory holding it comes back
    /// `PermissionDenied` for as long as it does. Three attempts at 200 ms, 400 ms and 800 ms cost
    /// nothing where nothing is scanning. Whether it actually clears the lock is MOD-16's to say.
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when the target exists, or when the rename fails for anything but a
    /// refusal it is worth retrying.
    pub async fn promote(
        &self,
        staged: &Path,
        id: &str,
        version: &str,
    ) -> Result<PathBuf, InstallError> {
        let target = self.version_dir(id, version);
        if self.exists(&target).await {
            return Err(InstallError::Io {
                what: format!("promote into {}", target.display()),
                message: "the directory already exists; the previous version was not set aside"
                    .to_owned(),
            });
        }
        tokio::fs::create_dir_all(self.agent_dir(id))
            .await
            .map_err(|error| io("create", &self.agent_dir(id), &error))?;

        let mut attempt = 1;
        let mut backoff = PROMOTE_BACKOFF;
        loop {
            match tokio::fs::rename(staged, &target).await {
                Ok(()) => return Ok(target),
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        && attempt < PROMOTE_ATTEMPTS =>
                {
                    warn!(
                        attempt,
                        target = %target.display(),
                        %error,
                        "the promote was refused; something may still be reading the new tree"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                    attempt += 1;
                }
                Err(error) => return Err(io("promote into", &target, &error)),
            }
        }
    }

    /// Puts a set-aside tree back where it came from (plan D16(b), blueprint P-10).
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when the target is occupied — the caller removes the promoted tree
    /// first — or when the rename fails.
    pub async fn restore(
        &self,
        previous: &Path,
        id: &str,
        version: &str,
    ) -> Result<(), InstallError> {
        let target = self.version_dir(id, version);
        if self.exists(&target).await {
            return Err(InstallError::Io {
                what: format!("restore {}", target.display()),
                message: "the directory already exists; remove the promoted tree first".to_owned(),
            });
        }
        tokio::fs::create_dir_all(self.agent_dir(id))
            .await
            .map_err(|error| io("create", &self.agent_dir(id), &error))?;
        tokio::fs::rename(previous, &target)
            .await
            .map_err(|error| io("restore", &target, &error))
    }

    /// Removes one version directory, if it is there.
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when it is there and cannot be removed.
    pub async fn remove_version(&self, id: &str, version: &str) -> Result<(), InstallError> {
        let dir = self.version_dir(id, version);
        if !self.exists(&dir).await {
            return Ok(());
        }
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|error| io("remove", &dir, &error))
    }

    /// Plan D16(a): every version but `keep`, deleted, once the probe has said `keep` works.
    ///
    /// Answers what it removed, in name order, so the outcome can say so out loud.
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when a directory cannot be removed.
    pub async fn retain_only(&self, id: &str, keep: &str) -> Result<Vec<String>, InstallError> {
        let mut removed = Vec::new();
        for version in self.existing_versions(id).await {
            if version == keep {
                continue;
            }
            self.remove_version(id, &version).await?;
            removed.push(version);
        }
        Ok(removed)
    }

    /// `<root>/.staging/`, created.
    async fn make_staging(&self) -> Result<(), InstallError> {
        let staging = self.staging();
        tokio::fs::create_dir_all(&staging)
            .await
            .map_err(|error| io("create", &staging, &error))
    }

    /// Whether a path is there at all — a directory, a file, or something else.
    async fn exists(&self, path: &Path) -> bool {
        tokio::fs::symlink_metadata(path).await.is_ok()
    }

    /// `<id>-<version>` back into its two halves.
    ///
    /// Neither half can be found by counting dashes: a registry id carries them (`<vendor>-acp`)
    /// and so does a semver prerelease (`1.0.0-rc.1`). So the split is decided by the disk — the
    /// candidate whose `<root>/<id>/` this root actually holds is the right one — and a stem no
    /// candidate matches is `None`, which the sweep reads as *leave it alone*.
    async fn split_name(&self, stem: &str) -> Option<(String, String)> {
        let rest = strip_nonce(stem)?;
        let mut at = rest.len();
        while let Some(dash) = rest[..at].rfind('-') {
            let (id, version) = (&rest[..dash], &rest[dash + 1..]);
            if !id.is_empty()
                && !version.is_empty()
                && tokio::fs::metadata(self.agent_dir(id))
                    .await
                    .is_ok_and(|meta| meta.is_dir())
            {
                return Some((id.to_owned(), version.to_owned()));
            }
            at = dash;
        }
        None
    }
}

/// What one [`sweep`](Layout::sweep) did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Staging entries deleted for being older than the limit.
    pub removed: Vec<PathBuf>,
    /// Version directories a stranded `.previous` was put back at.
    pub restored: Vec<PathBuf>,
}

/// `<pid>-<micros>`: unique per process per microsecond, and no new dependency for it.
///
/// It is what keeps two installs — or an install and the remains of an earlier one — from ever
/// naming the same staging entry, which is the collision hazard H-6 is about. `htui_store`'s
/// atomic write reaches for `Uuid::now_v7()` because `uuid` is already in that crate's graph; it
/// is not in this one, and a directory name does not justify adding it.
#[must_use]
pub fn nonce() -> String {
    let micros = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_micros())
        .unwrap_or_default();
    format!("{}-{micros}", std::process::id())
}

/// `<id>-<version>-<pid>-<micros>` with the two numeric trailing segments taken off.
///
/// Both halves of the nonce are digits, which is what makes the strip unambiguous however many
/// dashes the id and the version carry between them.
fn strip_nonce(stem: &str) -> Option<&str> {
    let (rest, micros) = stem.rsplit_once('-')?;
    let (rest, pid) = rest.rsplit_once('-')?;
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (numeric(micros) && numeric(pid)).then_some(rest)
}

/// How old one staging entry is, by its modification time. Unreadable metadata and a clock that
/// went backwards both read as *brand new*, which errs towards keeping.
async fn age_of(entry: &tokio::fs::DirEntry, now: SystemTime) -> Duration {
    let Ok(meta) = entry.metadata().await else {
        return Duration::ZERO;
    };
    meta.modified()
        .ok()
        .and_then(|at| now.duration_since(at).ok())
        .unwrap_or_default()
}

/// Removes a staging entry whether it is a file or a directory.
async fn remove_any(path: &Path) -> Result<(), InstallError> {
    let is_dir = tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|meta| meta.is_dir());
    let removed = if is_dir {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    };
    removed.map_err(|error| io("remove", path, &error))
}

/// One filesystem failure, named by what was being done and to what.
fn io(what: &str, path: &Path, error: &std::io::Error) -> InstallError {
    InstallError::Io {
        what: format!("{what} {}", path.display()),
        message: error.to_string(),
    }
}
