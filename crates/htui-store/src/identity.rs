//! The box identity file, the machine fingerprint and the cache directory name (plan D6, ANA-9
//! §4.4, MOD-7 PRD D1).
//!
//! `box.toml` is what makes `box.id` survive a hostname change (`R-BOX-4`): the id is minted once,
//! as a UUIDv7, and the hostname is rewritten around it. The `box` row is keyed on that id; the
//! [`Fingerprint`], a keyed hash of the OS machine identity, is what tells a `box.toml` copied
//! onto another machine apart from the machine that minted it. The raw identity is read here and
//! never leaves: no field, column, log line or file holds it. Nothing here talks to a database.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use hmac::{Hmac, Mac};
use htui_core::model::BoxId;
use htui_core::store::{Result, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::postgres::PgConnectOptions;
use uuid::Uuid;
use zeroize::Zeroizing;

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

/// Overwrites `<root>/box.toml`; used when registration minted a new id for a copied `box.toml`
/// ([`crate::Registration::Copied`], [`crate::connect::try_connect`]).
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

/// `app_user.name` of this OS user: `USERNAME`, then `USER`, then `htui` (ANA-9 §5.10).
///
/// One definition, two callers: [`crate::PgStore::seed_if_empty`] stamps a first, empty database
/// with it, and [`crate::CacheStore::this_user`] looks the mirrored row up by it (MOD-2 plan D33).
/// Two copies of this rule would mean an offline chat naming an author the server does not have,
/// and `upload_pending` inserting a stranger.
#[must_use]
pub fn os_user_name() -> String {
    for key in ["USERNAME", "USER"] {
        if let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            return value;
        }
    }
    "htui".to_owned()
}

/// The compiled message of the box fingerprint's HMAC (plan D1): the domain separator that keeps
/// the value uncorrelated with any other program's use of the same machine identity.
pub const FINGERPRINT_APP_ID: &[u8] = b"htui/box-fingerprint/v1";

/// HMAC-SHA256 keyed by the normalised OS machine identity over [`FINGERPRINT_APP_ID`] (PRD D1).
///
/// No `Display`, no `Serialize`; `Debug` prints `Fingerprint(<redacted>)`.
/// [`Fingerprint::as_hex`] exists only to bind `box.machine_fingerprint`.
#[derive(Clone, PartialEq, Eq)]
pub struct Fingerprint([u8; 32]);

impl core::fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Fingerprint(<redacted>)")
    }
}

impl Fingerprint {
    /// Trims and ASCII-lowercases `raw`; an empty identity is `None`, anything else the keyed hash.
    ///
    /// `raw` is the HMAC **key** and [`FINGERPRINT_APP_ID`] the message, so the stored value says
    /// nothing about the identity without it, and two programs keying the same identity with
    /// different messages produce unrelated values. The normalised copy of the identity lives only
    /// in a [`Zeroizing`] local.
    #[must_use]
    pub fn from_machine_identity(raw: &str) -> Option<Self> {
        let key = Zeroizing::new(raw.trim().to_ascii_lowercase());
        if key.is_empty() {
            return None;
        }
        // HMAC accepts a key of any length, so this never fails; `ok()?` keeps it panic-free.
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes()).ok()?;
        mac.update(FINGERPRINT_APP_ID);
        Some(Self(mac.finalize().into_bytes().into()))
    }

    /// Lowercase hex, 64 characters: what the column stores.
    #[must_use]
    pub fn as_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Read once per process: the machine identity does not change under a running `htui`.
static MACHINE_FINGERPRINT: tokio::sync::OnceCell<Option<Fingerprint>> =
    tokio::sync::OnceCell::const_new();

/// This machine's fingerprint, read from the OS once per process; `None` when no identity is
/// readable.
///
/// Linux reads `/etc/machine-id` (then the dbus copy), macOS the `IOPlatformUUID` of `ioreg`, and
/// Windows `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`. The raw text is only ever a
/// [`Zeroizing`] local of the reader, and only the keyed hash leaves this function.
pub async fn machine_fingerprint() -> Option<Fingerprint> {
    MACHINE_FINGERPRINT
        .get_or_init(|| async {
            let raw = read_os_identity().await?;
            Fingerprint::from_machine_identity(&raw)
        })
        .await
        .clone()
}

/// `<root>/etc/machine-id`, then `<root>/var/lib/dbus/machine-id` (plan D20).
///
/// A file counts only when its trimmed content is 32 hex characters: `uninitialized` (systemd's
/// first-boot placeholder) and an empty file fall through to the next one.
#[cfg(any(target_os = "linux", test))]
fn machine_id_under(root: &Path) -> Option<Zeroizing<String>> {
    ["etc/machine-id", "var/lib/dbus/machine-id"]
        .iter()
        .find_map(|relative| {
            let text = Zeroizing::new(std::fs::read_to_string(root.join(relative)).ok()?);
            let trimmed = text.trim();
            (trimmed.len() == 32 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| Zeroizing::new(trimmed.to_owned()))
        })
}

/// The value of the `"IOPlatformUUID" = "…"` line of `ioreg -rd1 -c IOPlatformExpertDevice`.
#[cfg(any(target_os = "macos", test))]
fn ioreg_platform_uuid(text: &str) -> Option<&str> {
    let line = text
        .lines()
        .find(|line| line.contains("\"IOPlatformUUID\""))?;
    let (_, value) = line.split_once('=')?;
    let value = value.trim().strip_prefix('"')?;
    let uuid = &value[..value.find('"')?];
    (!uuid.is_empty()).then_some(uuid)
}

/// The OS machine identity, raw (Linux: `/etc/machine-id`, then the dbus copy).
#[cfg(target_os = "linux")]
async fn read_os_identity() -> Option<Zeroizing<String>> {
    tokio::task::spawn_blocking(|| machine_id_under(Path::new("/")))
        .await
        .ok()
        .flatten()
}

/// The OS machine identity, raw (macOS: `ioreg`'s `IOPlatformUUID`, bounded at five seconds).
#[cfg(target_os = "macos")]
async fn read_os_identity() -> Option<Zeroizing<String>> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let stdout = Zeroizing::new(output.stdout);
    let text = std::str::from_utf8(&stdout).ok()?;
    ioreg_platform_uuid(text).map(|uuid| Zeroizing::new(uuid.to_owned()))
}

/// The OS machine identity, raw (Windows: the registry's `MachineGuid`).
#[cfg(windows)]
async fn read_os_identity() -> Option<Zeroizing<String>> {
    tokio::task::spawn_blocking(|| {
        windows_registry::LOCAL_MACHINE
            .open(r"SOFTWARE\Microsoft\Cryptography")
            .and_then(|key| key.get_string("MachineGuid"))
            .ok()
            .map(Zeroizing::new)
    })
    .await
    .ok()
    .flatten()
}

/// No machine identity reader on this platform: registration goes by `box.toml`'s id alone.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn read_os_identity() -> Option<Zeroizing<String>> {
    None
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
    use super::{
        Fingerprint, Identity, db_fingerprint, ioreg_platform_uuid, load_or_mint, machine_id_under,
        store,
    };
    use htui_core::model::BoxId;

    /// A synthetic machine identity: 32 hex characters, never this machine's.
    const SYNTHETIC: &str = "0123456789abcdef0123456789abcdef";

    /// HMAC-SHA256(key = [`SYNTHETIC`], message = `htui/box-fingerprint/v1`), computed outside
    /// this crate (Python's `hmac`), so the test pins the construction and not just itself.
    const SYNTHETIC_FINGERPRINT: &str =
        "a2e8a0ed177cf8b655e4a8c2d16f745209325966a8339fd26e7c6437dae364df";

    #[test]
    fn the_fingerprint_is_hmac_sha256_known_answer() {
        let fp = Fingerprint::from_machine_identity(SYNTHETIC).expect("a fingerprint");
        assert_eq!(fp.as_hex(), SYNTHETIC_FINGERPRINT);
    }

    #[test]
    fn the_fingerprint_ignores_case_and_surrounding_whitespace() {
        let upper = format!("  {}\n", SYNTHETIC.to_ascii_uppercase());
        assert_eq!(
            Fingerprint::from_machine_identity(&upper),
            Fingerprint::from_machine_identity(SYNTHETIC),
        );
    }

    #[test]
    fn an_empty_identity_is_no_fingerprint() {
        assert_eq!(Fingerprint::from_machine_identity(""), None);
        assert_eq!(Fingerprint::from_machine_identity(" \n"), None);
    }

    #[test]
    fn debug_never_prints_the_fingerprint() {
        let fp = Fingerprint::from_machine_identity(SYNTHETIC).expect("a fingerprint");
        assert_eq!(format!("{fp:?}"), "Fingerprint(<redacted>)");
    }

    #[test]
    fn linux_reads_machine_id_then_the_dbus_copy() {
        const ETC: &str = "11111111111111111111111111111111";
        const DBUS: &str = "22222222222222222222222222222222";
        let write = |root: &std::path::Path, rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        };
        let read = |root: &std::path::Path| machine_id_under(root).map(|id| id.to_string());

        let none = tempfile::tempdir().expect("temp root");
        assert_eq!(read(none.path()), None, "neither file: no identity");

        let dbus_only = tempfile::tempdir().expect("temp root");
        write(
            dbus_only.path(),
            "var/lib/dbus/machine-id",
            &format!("{DBUS}\n"),
        );
        assert_eq!(
            read(dbus_only.path()).as_deref(),
            Some(DBUS),
            "the dbus copy alone"
        );

        let both = tempfile::tempdir().expect("temp root");
        write(both.path(), "etc/machine-id", &format!("{ETC}\n"));
        write(both.path(), "var/lib/dbus/machine-id", &format!("{DBUS}\n"));
        assert_eq!(read(both.path()).as_deref(), Some(ETC), "/etc wins");

        let uninitialized = tempfile::tempdir().expect("temp root");
        write(uninitialized.path(), "etc/machine-id", "uninitialized\n");
        write(
            uninitialized.path(),
            "var/lib/dbus/machine-id",
            &format!("{DBUS}\n"),
        );
        assert_eq!(
            read(uninitialized.path()).as_deref(),
            Some(DBUS),
            "`uninitialized` falls through to the dbus copy (D20)"
        );

        let empty = tempfile::tempdir().expect("temp root");
        write(empty.path(), "etc/machine-id", "");
        assert_eq!(
            read(empty.path()),
            None,
            "an empty file is no identity (D20)"
        );
    }

    #[test]
    fn the_ioreg_line_is_parsed() {
        let sample = r#"+-o J316sAP  <class IOPlatformExpertDevice, id 0x100000223, registered, matched, active, busy 0 (1234 ms), retain 42>
    {
      "IOPlatformSerialNumber" = "C02XXXXXXXXX"
      "IOPlatformUUID" = "0A1B2C3D-4E5F-6A7B-8C9D-0E1F2A3B4C5D"
      "model" = <"MacBookPro18,1">
    }
"#;
        assert_eq!(
            ioreg_platform_uuid(sample),
            Some("0A1B2C3D-4E5F-6A7B-8C9D-0E1F2A3B4C5D")
        );
        assert_eq!(
            ioreg_platform_uuid("  \"IOPlatformSerialNumber\" = \"C02XXXXXXXXX\"\n"),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn this_box_has_a_fingerprint_when_machine_id_is_readable() {
        // Gated on what the reader itself accepts, not on the file merely being readable: an
        // `uninitialized` or empty /etc/machine-id is readable and still no identity (D20).
        if machine_id_under(std::path::Path::new("/")).is_some() {
            assert!(
                super::machine_fingerprint().await.is_some(),
                "a well-formed machine-id gives a fingerprint"
            );
        }
    }

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
