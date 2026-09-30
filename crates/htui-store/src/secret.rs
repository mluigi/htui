//! The Postgres DSN in the OS keyring (plan D7, blueprint C.3), and the sources a headless worker
//! may fall back to (MOD-41 PRD D3, plan D14).
//!
//! One entry, `("htui", "postgres-dsn")`, written by `htui --set-dsn` and removed by
//! `htui --clear-dsn`. The TUI reads the DSN from here and **nowhere else**. `htui worker` reads
//! it from exactly three sources, through [`headless_dsn`]: with `--dsn-stdin`, one line from
//! stdin and nothing else; otherwise the keyring; otherwise, on Linux, the systemd credential
//! `$CREDENTIALS_DIRECTORY/htui-dsn` ([`CREDENTIAL_NAME`]), which `systemd-creds` keeps encrypted
//! at rest and shows to the unit alone. **Never argv, the environment or a plain file**
//! (`R-STO-1` as amended): there is no env-var fallback, so a DSN never appears in `argv`, in a
//! shell history or in a dotfile. `CREDENTIALS_DIRECTORY` names a directory, not the secret.
//! `HTUI_TEST_DATABASE_URL` is read by the integration-test harness only, never by this crate at
//! run time.
//!
//! TLS is whatever the DSN's `sslmode=` says; `PgConnectOptions::from_str` handles it (`R-STO-2`).
//!
//! Under `--cfg feature = "test-support"` — which the binary never enables — the three functions
//! consult `FAKE` first, so a test can round-trip a DSN without touching the developer's OS
//! store. Nothing installs it but `crate::testkit::mock_keyring`; with it uninstalled the
//! production path below is what runs, unchanged (blueprint flag L, ruling §0.3).

use htui_core::store::{Result, StoreError};
use keyring::{Entry, Error as KeyringError};
use zeroize::Zeroizing;

/// Keyring service name of the DSN entry.
pub const SERVICE: &str = "htui";

/// Keyring user name of the DSN entry.
pub const USER: &str = "postgres-dsn";

/// Keyring user name of the Qdrant DSN entry.
/// The OS keyring username used to store the Qdrant URL.
pub const QDRANT_URL_USER: &str = "qdrant-url";
/// The OS keyring username used to store the Qdrant API key.
pub const QDRANT_KEY_USER: &str = "qdrant-key";

/// What an installed stand-in answers with (D18, flag L).
///
/// Two shapes because a keyring has two failure modes and they are **not** the same state: an
/// entry that is not there, and a store that cannot be opened at all. A box with a session bus
/// and no unlocked collection — a minimal window manager, a container, most CI images — is the
/// second, and the whole of this fix is that the two never collapse into each other.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeSlots {
    pub pg: Option<String>,
    pub qdrant_url: Option<String>,
    pub qdrant_key: Option<String>,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub(crate) enum Fake {
    /// A readable keyring holding DSNs, or holding nothing.
    Slot(FakeSlots),
    /// A keyring that answers nothing at all: every call fails with this sentence, wrapped in
    /// [`backend`]'s shape so a test sees exactly what the platform path would produce.
    Broken(String),
}

/// A process-wide stand-in for the keyring, for tests that must round-trip a DSN (D18, flag L).
///
/// `keyring::mock` keeps the secret **in the entry** and this module opens a fresh [`Slot`] per
/// call (see [`Slot`]'s own note), so the crate's mock cannot see a [`set_dsn`] from a later
/// [`get_dsn`]. Outer `None`: not installed, the real keyring answers — which is the only state a
/// binary can ever be in, because the binary does not enable `test-support` and nothing but
/// `crate::testkit::mock_keyring` and `crate::testkit::mock_keyring_broken` write this.
/// `Some(fake)`: every call is answered by [`Fake`] and no OS store is opened.
#[cfg(feature = "test-support")]
pub(crate) static FAKE: std::sync::Mutex<Option<Fake>> = std::sync::Mutex::new(None);

/// How many times [`get_dsn`] has been answered by an installed [`Fake`].
///
/// Only an installed fake counts, and a fake is only installed under the `KEYRING` lock that
/// `crate::testkit::mock_keyring` takes, so a case holding that guard sees no other case's reads.
/// It is how a test proves the keyring was **not read** — `--dsn-stdin` must never open it, since
/// on a headless host the real one may block on a secret-service prompt — rather than merely that
/// its answer lost.
#[cfg(feature = "test-support")]
pub(crate) static FAKE_DSN_READS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// The fake slot, poison-tolerant: a test that panicked mid-assertion must not poison every later
/// one.
#[cfg(feature = "test-support")]
pub(crate) fn fake() -> std::sync::MutexGuard<'static, Option<Fake>> {
    FAKE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The error an installed [`Fake::Broken`] answers `verb` with.
#[cfg(feature = "test-support")]
fn fake_failure(verb: &str, user: &str, why: &str) -> StoreError {
    backend(verb, &format!("{}/{}", SERVICE, user), &why)
}

/// The stored DSN, or `None` when nothing is stored.
///
/// A first launch has no DSN yet, so `keyring::Error::NoEntry` is `Ok(None)` rather than an error;
/// so is an entry holding only whitespace, which is what a mis-typed `--set-dsn` leaves behind.
///
/// Every **other** keyring failure stays an `Err`, and deliberately: a keyring that cannot be read
/// is not a keyring with nothing in it. Telling a user whose collection is merely locked that they
/// have no DSN would invite them to retype a credential into a store that cannot hold it. The
/// callers decide what to do with the distinction — [`crate::connect::start`] starts offline over
/// it rather than refusing to launch — but they are never handed a `None` that means "unknown".
///
/// # Errors
///
/// [`StoreError::Backend`] for every other keyring failure — a locked keychain, a missing secret
/// service, a platform that has none.
pub fn get_dsn() -> Result<Option<String>> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &*fake() {
        FAKE_DSN_READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        return match fake {
            Fake::Slot(slots) => Ok(slots.pg.clone().filter(|dsn| !dsn.trim().is_empty())),
            Fake::Broken(why) => Err(fake_failure("read", USER, why)),
        };
    }
    Slot::open(SERVICE, USER)?.get()
}

/// Stores the DSN, replacing whatever was there.
///
/// # Errors
///
/// [`StoreError::Backend`] when the keyring refuses the write.
pub fn set_dsn(dsn: &str) -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.pg = Some(dsn.to_owned());
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("store", USER, why)),
        };
    }
    Slot::open(SERVICE, USER)?.set(dsn)
}

/// Removes the entry. A missing entry is `Ok(())`.
///
/// # Errors
///
/// [`StoreError::Backend`] when the keyring refuses the delete.
pub fn clear_dsn() -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.pg = None;
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("remove", USER, why)),
        };
    }
    Slot::open(SERVICE, USER)?.clear()
}

/// The systemd credential name `htui worker` reads (`LoadCredentialEncrypted=htui-dsn:…`).
pub const CREDENTIAL_NAME: &str = "htui-dsn";

/// Where `htui worker` may read its DSN (PRD D3, plan D14).
pub struct DsnSources<'a> {
    /// `Some` exactly when `--dsn-stdin` was given: one line from it, and no other source.
    pub stdin: Option<&'a mut dyn std::io::BufRead>,
    /// `$CREDENTIALS_DIRECTORY` as the caller read it (a directory, not the secret; passed in so
    /// tests need no `set_var`). Read on Linux only.
    pub credentials_dir: Option<&'a std::path::Path>,
}

impl core::fmt::Debug for DsnSources<'_> {
    /// Whether stdin is a source, never what it holds.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DsnSources")
            .field("stdin", &self.stdin.is_some())
            .field("credentials_dir", &self.credentials_dir)
            .finish()
    }
}

/// Why no DSN was found. No variant carries the DSN.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DsnSourceError {
    /// `--dsn-stdin` read a blank line or end of input.
    #[error("no DSN on stdin; nothing was read")]
    EmptyStdin,
    /// Stdin failed.
    #[error("stdin could not be read: {0}")]
    Stdin(String),
    /// The credential exists and could not be read.
    #[error("the systemd credential {path} could not be read: {why}")]
    Credential {
        /// Its path.
        path: String,
        /// The OS error.
        why: String,
    },
    /// No source had one.
    #[error(
        "no Postgres DSN: the OS keyring has none ({keyring}), and there is no \
         $CREDENTIALS_DIRECTORY/htui-dsn; run `htui --set-dsn`, provision the systemd \
         credential, or pass `--dsn-stdin`"
    )]
    NoSource {
        /// What the keyring answered: "empty" or its error.
        keyring: String,
    },
}

/// One DSN line, trimmed, in a wiped buffer; `None` for a blank line or end of input. `htui
/// --set-dsn` and `htui worker --dsn-stdin` share it.
///
/// # Errors
///
/// Whatever `reader` fails with.
pub fn read_dsn_line(
    reader: &mut dyn std::io::BufRead,
) -> std::io::Result<Option<Zeroizing<String>>> {
    let mut line = Zeroizing::new(String::new());
    reader.read_line(&mut line)?;
    let dsn = line.trim();
    Ok((!dsn.is_empty()).then(|| Zeroizing::new(dsn.to_owned())))
}

/// `htui worker`'s DSN (PRD D3): `--dsn-stdin` alone when given; else the keyring (an `Err` is
/// "no keyring", not a failure); else, on Linux, `credentials_dir/htui-dsn`.
///
/// # Errors
///
/// [`DsnSourceError`]; never the DSN.
pub fn headless_dsn(
    sources: DsnSources<'_>,
) -> core::result::Result<Zeroizing<String>, DsnSourceError> {
    if let Some(stdin) = sources.stdin {
        return read_dsn_line(stdin)
            .map_err(|err| DsnSourceError::Stdin(err.to_string()))?
            .ok_or(DsnSourceError::EmptyStdin);
    }
    // `get_dsn` answers a plain `String`: wrapped at once.
    let keyring = match get_dsn() {
        Ok(Some(dsn)) => return Ok(Zeroizing::new(dsn)),
        Ok(None) => "empty".to_owned(),
        Err(err) => err.to_string(),
    };
    #[cfg(target_os = "linux")]
    if let Some(dir) = sources.credentials_dir {
        let path = dir.join(CREDENTIAL_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let text = Zeroizing::new(text);
                let dsn = text.trim();
                if !dsn.is_empty() {
                    return Ok(Zeroizing::new(dsn.to_owned()));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(DsnSourceError::Credential {
                    path: path.display().to_string(),
                    why: err.to_string(),
                });
            }
        }
    }
    Err(DsnSourceError::NoSource { keyring })
}

/// Retrieves the stored Qdrant URL, if any.
pub fn get_qdrant_url() -> Result<Option<String>> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &*fake() {
        return match fake {
            Fake::Slot(slots) => Ok(slots.qdrant_url.clone().filter(|u| !u.trim().is_empty())),
            Fake::Broken(why) => Err(fake_failure("read", QDRANT_URL_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_URL_USER)?.get()
}

/// Stores the Qdrant URL in the keyring.
pub fn set_qdrant_url(url: &str) -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.qdrant_url = Some(url.to_owned());
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("store", QDRANT_URL_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_URL_USER)?.set(url)
}

/// Removes the Qdrant URL from the keyring.
pub fn clear_qdrant_url() -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.qdrant_url = None;
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("remove", QDRANT_URL_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_URL_USER)?.clear()
}

/// Retrieves the stored Qdrant API key, if any.
pub fn get_qdrant_api_key() -> Result<Option<String>> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &*fake() {
        return match fake {
            Fake::Slot(slots) => Ok(slots.qdrant_key.clone().filter(|k| !k.trim().is_empty())),
            Fake::Broken(why) => Err(fake_failure("read", QDRANT_KEY_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_KEY_USER)?.get()
}

/// Stores the Qdrant API key in the keyring.
pub fn set_qdrant_api_key(api_key: &str) -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.qdrant_key = Some(api_key.to_owned());
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("store", QDRANT_KEY_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_KEY_USER)?.set(api_key)
}

/// Removes the Qdrant API key from the keyring.
pub fn clear_qdrant_api_key() -> Result<()> {
    #[cfg(feature = "test-support")]
    if let Some(fake) = &mut *fake() {
        return match fake {
            Fake::Slot(slots) => {
                slots.qdrant_key = None;
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("remove", QDRANT_KEY_USER, why)),
        };
    }
    Slot::open(SERVICE, QDRANT_KEY_USER)?.clear()
}

/// One keyring entry, plus the `service/user` pair its error messages quote.
///
/// A value rather than three `(service, user)` free functions because of the unit tests below:
/// `keyring::mock` keeps the secret **in the entry**, so two `Entry::new` calls for the same names
/// are two unrelated credentials and a round trip has to go through one of them. Production opens
/// a fresh slot per call, which is what the real backends want anyway.
struct Slot {
    /// The credential this slot reads and writes.
    entry: Entry,
    /// `"<service>/<user>"`, for the one error shape this module produces.
    name: String,
}

impl Slot {
    /// Opens the entry named `(service, user)` through the default credential builder.
    fn open(service: &str, user: &str) -> Result<Self> {
        let name = format!("{service}/{user}");
        let entry = Entry::new(service, user).map_err(|err| backend("open", &name, &err))?;
        Ok(Self { entry, name })
    }

    /// The stored secret, with a missing entry and a blank one both reading as `None`.
    fn get(&self) -> Result<Option<String>> {
        match self.entry.get_password() {
            Ok(dsn) if dsn.trim().is_empty() => Ok(None),
            Ok(dsn) => Ok(Some(dsn)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(err) => Err(backend("read", &self.name, &err)),
        }
    }

    /// Replaces whatever was there.
    fn set(&self, dsn: &str) -> Result<()> {
        self.entry
            .set_password(dsn)
            .map_err(|err| backend("store", &self.name, &err))
    }

    /// Removes the entry. A missing entry is `Ok(())`.
    fn clear(&self) -> Result<()> {
        match self.entry.delete_credential() {
            // `delete_credential`, not the 2.x `delete_password` (blueprint C.3).
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(backend("remove", &self.name, &err)),
        }
    }
}

/// The one error shape this module produces.
///
/// `err` is a `Display` rather than a [`KeyringError`] so the `test-support` fake produces the
/// platform path's sentence byte for byte instead of a second, almost-identical one.
fn backend(verb: &str, name: &str, err: &dyn core::fmt::Display) -> StoreError {
    StoreError::Backend(format!("cannot {verb} the keyring entry ({name}): {err}"))
}

#[cfg(test)]
mod tests {
    use super::{SERVICE, Slot, USER};
    use keyring::Entry;

    /// Installs `keyring::mock` as the default credential builder, once per test binary.
    ///
    /// Without it every case here would write a real credential into the Windows Credential
    /// Manager, the login keychain or the secret service - and leave it behind whenever one
    /// panicked before its `clear`. The mock is platform-independent, keeps the secret in the
    /// entry and touches no OS store at all, so `cargo test` provably creates no `htui*`
    /// credential. It has to be installed before the first `Entry::new`, which is why every
    /// helper below goes through [`mock_slot`].
    fn install_mock() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        });
    }

    /// A slot backed by the mock store. Never [`SERVICE`]: a test must not read, overwrite or
    /// delete the DSN the user stored with `htui --set-dsn`.
    fn mock_slot(user: &str) -> Slot {
        install_mock();
        Slot::open("htui-test", user).expect("open a mock entry")
    }

    #[test]
    fn a_dsn_round_trips_and_a_missing_entry_is_none() {
        let slot = mock_slot("round-trip");
        assert_eq!(
            slot.get().expect("read"),
            None,
            "an entry nothing was written to holds nothing"
        );

        slot.set("postgres://u:p@h:5433/db").expect("store");
        assert_eq!(
            slot.get().expect("read"),
            Some("postgres://u:p@h:5433/db".to_owned())
        );

        slot.set("postgres://u:p@h:5433/other").expect("replace");
        assert_eq!(
            slot.get().expect("read"),
            Some("postgres://u:p@h:5433/other".to_owned()),
            "a second store replaces rather than appends"
        );

        slot.clear().expect("clear");
        assert_eq!(slot.get().expect("read"), None);
        slot.clear().expect("clearing twice is not an error");
    }

    #[test]
    fn a_blank_entry_reads_as_no_dsn() {
        let slot = mock_slot("blank");
        slot.set("   ").expect("store");
        assert_eq!(
            slot.get().expect("read"),
            None,
            "whitespace is what a mis-typed --set-dsn leaves; it is not a DSN"
        );
        slot.clear().expect("clear");
    }

    #[test]
    fn the_real_entry_is_named_exactly_as_the_plan_says() {
        assert_eq!((SERVICE, USER), ("htui", "postgres-dsn"));
    }

    /// The same round trip against the real platform store, for a manual run.
    ///
    /// Builds its credential from `keyring::default` rather than `Entry::new`, so it stays honest
    /// even under `--include-ignored`, where another case in this module has already replaced the
    /// default builder with the mock. The service name carries the pid, and the entry is removed
    /// on the way out - but it is `#[ignore]`d because a panic in between would leave a real
    /// credential behind.
    #[test]
    #[ignore = "writes a real OS credential; run manually with `cargo test -- --ignored`"]
    fn the_real_backend_round_trips_too() {
        let service = format!("htui-test-{}", std::process::id());
        let user = "round-trip";
        assert_ne!(service, SERVICE, "the tests own their own service name");

        let credential = keyring::default::default_credential_builder()
            .build(None, &service, user)
            .expect("build a platform credential");
        let slot = Slot {
            entry: Entry::new_with_credential(credential),
            name: format!("{service}/{user}"),
        };

        slot.clear().expect("whatever a crashed run left behind");
        assert_eq!(slot.get().expect("read"), None);
        slot.set("postgres://u:p@h:5433/db").expect("store");
        assert_eq!(
            slot.get().expect("read"),
            Some("postgres://u:p@h:5433/db".to_owned())
        );
        slot.clear().expect("clear");
        assert_eq!(slot.get().expect("read"), None);
    }
}

/// `htui worker`'s DSN sources (MOD-41 PRD D3, blueprint §13.4), over the process-wide fake
/// keyring: never the developer's.
#[cfg(test)]
mod headless_dsn_tests {
    use super::{
        CREDENTIAL_NAME, DsnSourceError, DsnSources, FAKE_DSN_READS, headless_dsn, set_dsn,
    };
    use crate::testkit::{mock_keyring, mock_keyring_broken};
    use std::io::Cursor;
    use std::sync::atomic::Ordering;

    /// Keyring reads answered by the fake so far; stable while a `mock_keyring` guard is held.
    fn keyring_reads() -> usize {
        FAKE_DSN_READS.load(Ordering::SeqCst)
    }

    /// A DSN nothing else in these cases spells, so an answer names its source.
    const KEYRING_DSN: &str = "postgres://keyring@h:5432/db";
    /// What `--dsn-stdin` is handed.
    const STDIN_DSN: &str = "postgres://stdin@h:5432/db";
    /// What the systemd credential holds.
    #[cfg(target_os = "linux")]
    const CREDENTIAL_DSN: &str = "postgres://credential@h:5432/db";

    /// A credentials directory holding `htui-dsn` with `text` in it.
    #[cfg(target_os = "linux")]
    fn credentials(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a credentials directory");
        std::fs::write(dir.path().join(CREDENTIAL_NAME), text).expect("write the credential");
        dir
    }

    #[tokio::test]
    async fn stdin_wins_and_the_keyring_is_not_read() {
        let guard = mock_keyring().await;
        set_dsn(KEYRING_DSN).expect("the fake keyring stores");
        let reads = keyring_reads();
        let mut stdin = Cursor::new(format!("{STDIN_DSN}\n").into_bytes());
        let dsn = headless_dsn(DsnSources {
            stdin: Some(&mut stdin),
            credentials_dir: None,
        })
        .expect("stdin holds a DSN");
        assert_eq!(dsn.as_str(), STDIN_DSN, "--dsn-stdin is explicit and wins");
        assert_eq!(
            keyring_reads(),
            reads,
            "--dsn-stdin never reads the keyring"
        );
        drop(guard);

        let _broken = mock_keyring_broken().await;
        let reads = keyring_reads();
        let mut stdin = Cursor::new(format!("{STDIN_DSN}\n").into_bytes());
        let dsn = headless_dsn(DsnSources {
            stdin: Some(&mut stdin),
            credentials_dir: None,
        })
        .expect("stdin wins even over a keyring that cannot be opened");
        assert_eq!(dsn.as_str(), STDIN_DSN);
        assert_eq!(
            keyring_reads(),
            reads,
            "a keyring that cannot be opened is never opened under --dsn-stdin"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_keyring_is_read_before_the_credential() {
        let _guard = mock_keyring().await;
        set_dsn(KEYRING_DSN).expect("the fake keyring stores");
        let dir = credentials(CREDENTIAL_DSN);
        let dsn = headless_dsn(DsnSources {
            stdin: None,
            credentials_dir: Some(dir.path()),
        })
        .expect("the keyring holds a DSN");
        assert_eq!(
            dsn.as_str(),
            KEYRING_DSN,
            "the keyring comes first (plan D14)"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_missing_keyring_falls_back_to_the_credential() {
        let _broken = mock_keyring_broken().await;
        let dir = credentials(&format!("{CREDENTIAL_DSN}\n"));
        let dsn = headless_dsn(DsnSources {
            stdin: None,
            credentials_dir: Some(dir.path()),
        })
        .expect("a keyring-less host reads the credential");
        assert_eq!(
            dsn.as_str(),
            CREDENTIAL_DSN,
            "a keyring error is no keyring, not a failure; the credential is trimmed"
        );
    }

    #[tokio::test]
    async fn no_source_is_an_error_naming_all_three() {
        let _guard = mock_keyring().await;
        let err = headless_dsn(DsnSources {
            stdin: None,
            credentials_dir: None,
        })
        .expect_err("an empty keyring and no credentials directory hold no DSN");
        assert_eq!(
            err,
            DsnSourceError::NoSource {
                keyring: "empty".to_owned()
            }
        );
        let text = err.to_string();
        for source in ["keyring", "CREDENTIALS_DIRECTORY", "--dsn-stdin"] {
            assert!(
                text.contains(source),
                "the refusal names {source}, got {text}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_credential_is_read_only_from_the_given_directory() {
        let _guard = mock_keyring().await;
        let elsewhere = credentials(CREDENTIAL_DSN);
        let given = tempfile::tempdir().expect("an empty credentials directory");
        let err = headless_dsn(DsnSources {
            stdin: None,
            credentials_dir: Some(given.path()),
        })
        .expect_err("a credential outside the given directory is not a source");
        assert!(
            matches!(err, DsnSourceError::NoSource { .. }),
            "got {err:?}"
        );
        drop(elsewhere);
    }

    #[tokio::test]
    async fn a_blank_line_is_no_dsn() {
        let _guard = mock_keyring().await;
        set_dsn(KEYRING_DSN).expect("the fake keyring stores");
        let mut stdin = Cursor::new(b"   \n".to_vec());
        let err = headless_dsn(DsnSources {
            stdin: Some(&mut stdin),
            credentials_dir: None,
        })
        .expect_err("a blank line is no DSN");
        assert_eq!(
            err,
            DsnSourceError::EmptyStdin,
            "explicit wins: a blank --dsn-stdin never falls through to the keyring"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_credential_file_holding_only_whitespace_is_no_dsn() {
        let _guard = mock_keyring().await;
        let dir = credentials(" \n\t\n");
        let err = headless_dsn(DsnSources {
            stdin: None,
            credentials_dir: Some(dir.path()),
        })
        .expect_err("whitespace is not a DSN");
        assert!(
            matches!(err, DsnSourceError::NoSource { .. }),
            "got {err:?}"
        );
    }
}
