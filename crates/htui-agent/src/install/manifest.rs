//! `<root>/<id>/manifest.json` — what this box accepted and what it has installed (plan MOD-20
//! D17).
//!
//! It lives beside the version directories rather than in the database because it is a fact about
//! **this box's disk**, and a box whose database is unreachable must still know whether it
//! accepted a licence before. The rejected homes each had a reason: `agent_box` needs a column and
//! therefore a migration MOD-4 has already been promised, `box.settings` has no write path and no
//! agreed merge semantics, `app_setting` is global rather than per box, and `box.toml` belongs to
//! another crate.
//!
//! One file per registry id, not one record per version directory, because plan D3 deletes version
//! directories and plan D2 needs a digest to outlive the tree it produced: *this version was
//! installed before with a different digest* is only sayable if the record survives the sweep.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::InstallError;
use super::layout::nonce;

/// One registry id's record on this box (plan D17).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// The terms this box accepted, when it did.
    pub consent: Option<Consent>,
    /// One record per installed version, keyed by version string.
    pub installs: BTreeMap<String, InstallRecord>,
}

/// One acceptance of one set of terms (plan D17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    /// The licence identifier as the registry spelled it at acceptance time.
    pub license: Option<String>,
    /// The terms URL as the registry spelled it at acceptance time.
    pub license_url: Option<String>,
    /// When `y` was pressed.
    pub accepted_at: DateTime<Utc>,
    /// The version that was being installed then.
    pub version: String,
}

/// What one install left behind (plan D2, D17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    /// The digest of the archive that produced this tree, lowercase hex.
    pub sha256: String,
    /// Whether the registry published that digest, or `htui` merely computed it.
    pub published: bool,
    /// The archive URL it came from.
    pub archive: String,
    /// The platform key it was fetched for.
    pub platform: String,
    /// When it was installed.
    pub installed_at: DateTime<Utc>,
}

impl Manifest {
    /// The manifest at `path`; [`Default`] when it is absent or does not parse.
    ///
    /// A hand-edited file must not block an install, so a parse failure is a `warn!` and an empty
    /// manifest: the worst it costs is asking for consent that was already given once.
    pub async fn load(path: &Path) -> Self {
        let Ok(bytes) = tokio::fs::read(path).await else {
            return Self::default();
        };
        match serde_json::from_slice(&bytes) {
            Ok(manifest) => manifest,
            Err(error) => {
                warn!(path = %path.display(), %error, "the install manifest does not parse; treating it as empty");
                Self::default()
            }
        }
    }

    /// Writes the manifest through a uniquely named temporary and a rename.
    ///
    /// The pattern is `htui_store::identity::store`'s, for its reason: a reader that arrives
    /// mid-write sees the previous whole file or the new whole file and never a truncated one.
    /// The directory is created first, because the very first install writes this before anything
    /// else has had a reason to make `<root>/<id>/`.
    ///
    /// # Errors
    ///
    /// [`InstallError::Io`] when the directory, the temporary or the rename fails. A caller may
    /// treat that as fatal — an install nobody recorded will ask for consent again — but it is
    /// never a reason to undo a tree that is already promoted and probing.
    pub async fn store(&self, path: &Path) -> Result<(), InstallError> {
        let dir = path.parent().unwrap_or(Path::new("."));
        let name = path.file_name().map_or_else(
            || "manifest.json".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|error| io("create", dir, &error))?;

        let text = serde_json::to_vec_pretty(self).map_err(|error| InstallError::Io {
            what: format!("serialise {}", path.display()),
            message: error.to_string(),
        })?;
        let tmp = dir.join(format!("{name}.{}.tmp", nonce()));
        tokio::fs::write(&tmp, &text)
            .await
            .map_err(|error| io("write", &tmp, &error))?;
        if let Err(error) = tokio::fs::rename(&tmp, path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io("replace", path, &error));
        }
        Ok(())
    }

    /// Whether the recorded consent covers these terms (plan D17).
    ///
    /// Both halves have to match: a licence that stayed `proprietary` while its `license_url`
    /// moved is new terms, and asking again is the cheap side of that mistake.
    #[must_use]
    pub fn consent_covers(&self, license: Option<&str>, license_url: Option<&str>) -> bool {
        self.consent.as_ref().is_some_and(|consent| {
            consent.license.as_deref() == license && consent.license_url.as_deref() == license_url
        })
    }
}

/// One filesystem failure, named by what was being done and to what.
fn io(what: &str, path: &Path, error: &std::io::Error) -> InstallError {
    InstallError::Io {
        what: format!("{what} {}", path.display()),
        message: error.to_string(),
    }
}
