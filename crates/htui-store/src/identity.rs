//! The box identity file and the cache directory name (plan D6, ANA-9 §4.4).
//!
//! `box.toml` is what makes `box.id` survive a hostname change (`R-BOX-4`): the id is minted once,
//! as a UUIDv7, and the hostname is rewritten around it. The full box probe (`box_tool`, tags,
//! RAM, GPU) is MOD-7's; nothing here talks to a database.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use htui_core::model::BoxId;
use htui_core::store::{Result, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgConnectOptions;
use uuid::Uuid;

/// File name under [`config_root`] holding [`Identity`].
pub const BOX_FILE: &str = "box.toml";

/// This box, as `<config_root>/box.toml` records it (ANA-9 §4.4 layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// `box.id`, minted as a UUIDv7 on first launch and never re-minted.
    pub box_id: BoxId,
    /// `box.hostname` as of the last launch; a hostname change does not change `box_id`.
    pub hostname: String,
}

/// The on-disk form; `toml` round-trips it.
#[derive(Debug, Serialize, Deserialize)]
struct BoxToml {
    box_id: Uuid,
    hostname: String,
}

/// `<dirs::config_dir()>/htui`, created if missing.
///
/// `%APPDATA%\htui` on Windows, `~/.config/htui` on Linux, `~/Library/Application Support/htui` on
/// macOS.
///
/// # Errors
///
/// [`StoreError::Backend`] when the platform has no config directory or the directory cannot be
/// created.
pub fn config_root() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| StoreError::Backend("this platform has no config directory".to_owned()))?;
    let root = base.join("htui");
    std::fs::create_dir_all(&root)
        .map_err(|e| StoreError::Backend(format!("cannot create {}: {e}", root.display())))?;
    Ok(root)
}

/// Reads `<root>/box.toml`, or mints and writes it.
///
/// Minting takes [`BoxId::new`] (UUIDv7) and `gethostname()`. An existing file whose `hostname`
/// differs from the current one is rewritten with the new hostname and the **same** `box_id`.
///
/// # Errors
///
/// [`StoreError::Backend`] when the file exists but cannot be read or parsed, or when a mint
/// cannot be written.
pub fn load_or_mint(root: &Path) -> Result<Identity> {
    let path = root.join(BOX_FILE);
    let hostname = hostname();

    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let parsed: BoxToml = toml::from_str(&text).map_err(|e| {
                StoreError::Backend(format!("{} is not valid box.toml: {e}", path.display()))
            })?;
            let identity = Identity {
                box_id: BoxId::from_uuid(parsed.box_id),
                hostname,
            };
            if parsed.hostname != identity.hostname {
                store(root, &identity)?;
            }
            Ok(identity)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let identity = Identity {
                box_id: BoxId::new(),
                hostname,
            };
            store(root, &identity)?;
            Ok(identity)
        }
        Err(e) => Err(StoreError::Backend(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

/// Overwrites `<root>/box.toml`; used by the adopt-DB-id rule of [`crate::PgStore::register_box`].
///
/// The write goes to a uniquely named temporary file and is then renamed over the target, so a
/// second process reading the file concurrently never sees a half-written one.
///
/// # Errors
///
/// [`StoreError::Backend`] when the directory or the file cannot be written.
pub fn store(root: &Path, identity: &Identity) -> Result<()> {
    std::fs::create_dir_all(root)
        .map_err(|e| StoreError::Backend(format!("cannot create {}: {e}", root.display())))?;

    let text = toml::to_string_pretty(&BoxToml {
        box_id: identity.box_id.as_uuid(),
        hostname: identity.hostname.clone(),
    })
    .map_err(|e| StoreError::Backend(format!("cannot serialise box.toml: {e}")))?;

    let tmp = root.join(format!("{BOX_FILE}.{}.tmp", Uuid::now_v7().simple()));
    std::fs::write(&tmp, text)
        .map_err(|e| StoreError::Backend(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, root.join(BOX_FILE)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StoreError::Backend(format!(
            "cannot replace {}: {e}",
            root.join(BOX_FILE).display()
        ))
    })
}

/// Lowercase hex sha256 of `host:port/dbname`, the cache directory name (ANA-9 §4.4).
///
/// Never includes credentials: the DSN is parsed with `PgConnectOptions::from_str` and only the
/// three coordinates are hashed. A DSN that does not parse hashes its own text, so a broken DSN
/// still gets a stable, non-colliding directory instead of a panic.
#[must_use]
pub fn db_fingerprint(dsn: &str) -> String {
    let material = match PgConnectOptions::from_str(dsn) {
        Ok(opts) => format!(
            "{}:{}/{}",
            opts.get_host(),
            opts.get_port(),
            opts.get_database().unwrap_or_default()
        ),
        Err(_) => dsn.to_owned(),
    };
    let digest = Sha256::digest(material.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The cache directory for a DSN: `<root>/cache/<db_fingerprint>` (ANA-9 §4.4).
///
/// Holds `cache.sqlite` and `pending/`; MOD-6 T3 fills it. `root` is a caller-supplied config
/// root ([`config_root`] in production, a temporary directory under test), so nothing here reaches
/// into the user's real configuration. Pure: computes a path, creates nothing.
#[must_use]
pub fn cache_dir(root: &Path, dsn: &str) -> PathBuf {
    root.join("cache").join(db_fingerprint(dsn))
}

/// This machine's host name, or `unknown-host` when the OS will not say.
fn hostname() -> String {
    let raw = gethostname::gethostname().to_string_lossy().into_owned();
    if raw.trim().is_empty() {
        "unknown-host".to_owned()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::{Identity, db_fingerprint, load_or_mint, store};
    use htui_core::model::BoxId;

    #[test]
    fn a_hostname_change_keeps_the_box_id() {
        let root = tempfile::tempdir().expect("temp root");
        let minted = load_or_mint(root.path()).expect("mint");

        store(
            root.path(),
            &Identity {
                box_id: minted.box_id,
                hostname: "SOME-OTHER-NAME".to_owned(),
            },
        )
        .expect("rewrite with an old hostname");

        let reread = load_or_mint(root.path()).expect("re-read");
        assert_eq!(reread.box_id, minted.box_id, "the id survives (R-BOX-4)");
        assert_eq!(
            reread.hostname, minted.hostname,
            "the hostname is refreshed"
        );
    }

    #[test]
    fn store_then_load_round_trips() {
        let root = tempfile::tempdir().expect("temp root");
        let written = Identity {
            box_id: BoxId::new(),
            hostname: super::hostname(),
        };
        store(root.path(), &written).expect("write");
        assert_eq!(load_or_mint(root.path()).expect("read"), written);
    }

    #[test]
    fn the_fingerprint_ignores_credentials() {
        assert_eq!(
            db_fingerprint("postgres://a:b@h:5433/db"),
            db_fingerprint("postgres://c:d@h:5433/db")
        );
    }
}
