//! The Postgres DSN in the OS keyring (plan D7, blueprint C.3).
//!
//! One entry, `("htui", "postgres-dsn")`, written by `htui --set-dsn` and removed by
//! `htui --clear-dsn`. The binary reads the DSN from here and **nowhere else**: there is no
//! env-var fallback (`R-STO-1`), so a DSN never appears in `argv`, in a shell history or in a
//! dotfile. `HTUI_TEST_DATABASE_URL` is read by the integration-test harness only, never by this
//! crate at run time.
//!
//! TLS is whatever the DSN's `sslmode=` says; `PgConnectOptions::from_str` handles it (`R-STO-2`).
//!
//! Under `--cfg feature = "test-support"` — which the binary never enables — the three functions
//! consult `FAKE` first, so a test can round-trip a DSN without touching the developer's OS
//! store. Nothing installs it but `crate::testkit::mock_keyring`; with it uninstalled the
//! production path below is what runs, unchanged (blueprint flag L, ruling §0.3).

use htui_core::store::{Result, StoreError};
use keyring::{Entry, Error as KeyringError};

/// Keyring service name of the DSN entry.
pub const SERVICE: &str = "htui";

/// Keyring user name of the DSN entry.
pub const USER: &str = "postgres-dsn";

/// What an installed stand-in answers with (D18, flag L).
///
/// Two shapes because a keyring has two failure modes and they are **not** the same state: an
/// entry that is not there, and a store that cannot be opened at all. A box with a session bus
/// and no unlocked collection — a minimal window manager, a container, most CI images — is the
/// second, and the whole of this fix is that the two never collapse into each other.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub(crate) enum Fake {
    /// A readable keyring holding a DSN, or holding nothing.
    Slot(Option<String>),
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

/// The fake slot, poison-tolerant: a test that panicked mid-assertion must not poison every later
/// one.
#[cfg(feature = "test-support")]
pub(crate) fn fake() -> std::sync::MutexGuard<'static, Option<Fake>> {
    FAKE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The error an installed [`Fake::Broken`] answers `verb` with.
#[cfg(feature = "test-support")]
fn fake_failure(verb: &str, why: &str) -> StoreError {
    backend(verb, &format!("{SERVICE}/{USER}"), &why)
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
        return match fake {
            Fake::Slot(slot) => Ok(slot.clone().filter(|dsn| !dsn.trim().is_empty())),
            Fake::Broken(why) => Err(fake_failure("read", why)),
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
            Fake::Slot(slot) => {
                *slot = Some(dsn.to_owned());
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("store", why)),
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
            Fake::Slot(slot) => {
                *slot = None;
                Ok(())
            }
            Fake::Broken(why) => Err(fake_failure("remove", why)),
        };
    }
    Slot::open(SERVICE, USER)?.clear()
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
