//! The Postgres DSN in the OS keyring (plan D7, blueprint C.3).
//!
//! One entry, `("htui", "postgres-dsn")`, written by `htui --set-dsn` and removed by
//! `htui --clear-dsn`. The binary reads the DSN from here and **nowhere else**: there is no
//! env-var fallback (`R-STO-1`), so a DSN never appears in `argv`, in a shell history or in a
//! dotfile. `HTUI_TEST_DATABASE_URL` is read by the integration-test harness only, never by this
//! crate at run time.
//!
//! TLS is whatever the DSN's `sslmode=` says; `PgConnectOptions::from_str` handles it (`R-STO-2`).

use htui_core::store::{Result, StoreError};
use keyring::{Entry, Error as KeyringError};

/// Keyring service name of the DSN entry.
pub const SERVICE: &str = "htui";

/// Keyring user name of the DSN entry.
pub const USER: &str = "postgres-dsn";

/// The stored DSN, or `None` when nothing is stored.
///
/// A first launch has no DSN yet, so `keyring::Error::NoEntry` is `Ok(None)` rather than an error;
/// so is an entry holding only whitespace, which is what a mis-typed `--set-dsn` leaves behind.
///
/// # Errors
///
/// [`StoreError::Backend`] for every other keyring failure — a locked keychain, a missing secret
/// service, a platform that has none.
pub fn get_dsn() -> Result<Option<String>> {
    get_at(SERVICE, USER)
}

/// Stores the DSN, replacing whatever was there.
///
/// # Errors
///
/// [`StoreError::Backend`] when the keyring refuses the write.
pub fn set_dsn(dsn: &str) -> Result<()> {
    set_at(SERVICE, USER, dsn)
}

/// Removes the entry. A missing entry is `Ok(())`.
///
/// # Errors
///
/// [`StoreError::Backend`] when the keyring refuses the delete.
pub fn clear_dsn() -> Result<()> {
    clear_at(SERVICE, USER)
}

/// [`get_dsn`] against a named entry, so the unit tests never touch the real one.
fn get_at(service: &str, user: &str) -> Result<Option<String>> {
    match entry(service, user)?.get_password() {
        Ok(dsn) if dsn.trim().is_empty() => Ok(None),
        Ok(dsn) => Ok(Some(dsn)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(backend("read", service, user, &err)),
    }
}

/// [`set_dsn`] against a named entry.
fn set_at(service: &str, user: &str, dsn: &str) -> Result<()> {
    entry(service, user)?
        .set_password(dsn)
        .map_err(|err| backend("store", service, user, &err))
}

/// [`clear_dsn`] against a named entry.
fn clear_at(service: &str, user: &str) -> Result<()> {
    match entry(service, user)?.delete_credential() {
        // `delete_credential`, not the 2.x `delete_password` (blueprint C.3).
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(backend("remove", service, user, &err)),
    }
}

/// One keyring entry, or the reason the platform would not build one.
fn entry(service: &str, user: &str) -> Result<Entry> {
    Entry::new(service, user).map_err(|err| backend("open", service, user, &err))
}

/// The one error shape this module produces.
fn backend(verb: &str, service: &str, user: &str, err: &KeyringError) -> StoreError {
    StoreError::Backend(format!(
        "cannot {verb} the keyring entry ({service}/{user}): {err}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{SERVICE, USER, clear_at, get_at, set_at};

    /// A service name no other process and no other test run shares.
    ///
    /// Never [`SERVICE`]: a test must not read, overwrite or delete the DSN the user stored with
    /// `htui --set-dsn`.
    fn test_service() -> String {
        format!("htui-test-{}", std::process::id())
    }

    #[test]
    fn a_dsn_round_trips_and_a_missing_entry_is_none() {
        let service = test_service();
        let user = "round-trip";
        assert_ne!(service, SERVICE, "the tests own their own service name");

        // Whatever a previous crashed run left behind.
        clear_at(&service, user).expect("clear");
        assert_eq!(get_at(&service, user).expect("read"), None);

        set_at(&service, user, "postgres://u:p@h:5433/db").expect("store");
        assert_eq!(
            get_at(&service, user).expect("read"),
            Some("postgres://u:p@h:5433/db".to_owned())
        );

        set_at(&service, user, "postgres://u:p@h:5433/other").expect("replace");
        assert_eq!(
            get_at(&service, user).expect("read"),
            Some("postgres://u:p@h:5433/other".to_owned()),
            "a second store replaces rather than appends"
        );

        clear_at(&service, user).expect("clear");
        assert_eq!(get_at(&service, user).expect("read"), None);
        clear_at(&service, user).expect("clearing twice is not an error");
    }

    #[test]
    fn a_blank_entry_reads_as_no_dsn() {
        let service = test_service();
        let user = "blank";
        set_at(&service, user, "   ").expect("store");
        assert_eq!(
            get_at(&service, user).expect("read"),
            None,
            "whitespace is what a mis-typed --set-dsn leaves; it is not a DSN"
        );
        clear_at(&service, user).expect("clear");
    }

    #[test]
    fn the_real_entry_is_named_exactly_as_the_plan_says() {
        assert_eq!((SERVICE, USER), ("htui", "postgres-dsn"));
    }
}
