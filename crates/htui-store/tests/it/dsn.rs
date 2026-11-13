//! The DSN newtype's contract (MOD-15 milestone 6, D1–D3; `docs/ANA-10.md` §4.9; blueprint §1.6).
//!
//! Three properties, and nothing else in this file matters as much as they do:
//!
//! 1. **`Dsn::parse` refuses with a fixed vocabulary.** Five sentences, none of which carries any
//!    of what was typed — because a refusal is rendered in the section and could otherwise echo a
//!    password back at the user (ANA-10 §4.9 (5)).
//! 2. **The refusal happens before sqlx sees the string.** sqlx-postgres 0.9.0 does not reject an
//!    unrecognised query parameter; it runs `tracing::warn!(%key, %value, ..)`
//!    (`options/parse.rs:107`), which puts the *value* the user typed into the `--log` file. The
//!    pre-scan is the guard, so `an_unknown_parameter_is_refused_before_sqlx_sees_it` is a
//!    security test, not a style one.
//! 3. **Nothing prints the text.** `Debug` is `Dsn(<redacted>)`, `summary()` is reconstructed from
//!    getters `PgConnectOptions` has — and it has no password getter at all.
//!
//! None of these cases is Postgres-gated: `apply_dsn` writes a keyring entry and opens a SQLite
//! mirror, and never dials.
#![cfg(feature = "test-support")]

use std::path::Path;
use std::time::Duration;

use htui_store::connect::{self, ConnectContext};
use htui_store::{CacheStore, Dsn, DsnError, PgStore, identity, secret, testkit};

/// A DSN carrying every field the summary reports, and a password no assertion may find.
const FULL: &str = "postgres://htui:s3cret@db.example:5432/htui?sslmode=require";

/// The query keys sqlx-postgres 0.9.0 handles (`options/parse.rs:51-105`), each with a value that
/// crate accepts, so "recognised" is asserted through a real parse rather than against our own
/// table.
const KNOWN: [(&str, &str); 18] = [
    ("sslmode", "require"),
    ("ssl-mode", "prefer"),
    ("sslrootcert", "/tmp/root.crt"),
    ("ssl-root-cert", "/tmp/root.crt"),
    ("ssl-ca", "/tmp/root.crt"),
    ("sslcert", "/tmp/client.crt"),
    ("ssl-cert", "/tmp/client.crt"),
    ("sslkey", "/tmp/client.key"),
    ("ssl-key", "/tmp/client.key"),
    ("statement-cache-capacity", "64"),
    ("host", "other.example"),
    ("hostaddr", "127.0.0.1"),
    ("port", "5433"),
    ("dbname", "htui"),
    ("user", "htui"),
    ("password", "s3cret"),
    ("application_name", "htui"),
    ("options", "-c_geqo"),
];

/// The context `apply_dsn` needs, rooted at a throwaway directory.
fn context(root: &Path) -> ConnectContext {
    ConnectContext {
        config_root: root.to_owned(),
        connect_timeout: Duration::from_millis(50),
        offline: false,
        registered: connect::Registered::default(),
    }
}

fn refusal(text: &str) -> DsnError {
    Dsn::parse(text).expect_err("this DSN must be refused")
}

#[test]
fn a_plain_string_is_not_a_url() {
    assert_eq!(refusal("nope"), DsnError::NotAUrl);
    assert_eq!(refusal("nope").to_string(), "not a URL");
    assert_eq!(refusal("mysql://h/d"), DsnError::NotAUrl);
    assert_eq!(refusal(""), DsnError::NotAUrl);
}

#[test]
fn a_url_without_a_host_is_refused() {
    assert_eq!(refusal("postgres:///htui"), DsnError::NoHost);
    assert_eq!(refusal("postgres://user:pw@/htui"), DsnError::NoHost);
    assert_eq!(refusal("postgres://user:pw@/htui").to_string(), "no host");
}

#[test]
fn an_unknown_sslmode_is_refused() {
    assert_eq!(
        refusal("postgres://h/d?sslmode=maybe"),
        DsnError::UnsupportedSslMode
    );
    assert_eq!(
        refusal("postgres://h/d?ssl-mode=maybe").to_string(),
        "unsupported sslmode"
    );
    // sqlx lower-cases before matching (`ssl_mode.rs:38-45`), so the scan must too.
    assert!(Dsn::parse("postgres://h/d?sslmode=REQUIRE").is_ok());
    for mode in [
        "disable",
        "allow",
        "prefer",
        "require",
        "verify-ca",
        "verify-full",
    ] {
        assert!(
            Dsn::parse(&format!("postgres://h/d?sslmode={mode}")).is_ok(),
            "{mode} is one of the six"
        );
    }
}

#[test]
fn a_port_above_u16_is_refused() {
    assert_eq!(refusal("postgres://h:70000/d"), DsnError::PortOutOfRange);
    assert_eq!(
        refusal("postgres://h/d?port=70000"),
        DsnError::PortOutOfRange
    );
    assert_eq!(
        refusal("postgres://h:70000/d").to_string(),
        "port out of range"
    );
    assert!(Dsn::parse("postgres://h:65535/d").is_ok());
}

#[test]
fn an_unknown_parameter_is_refused_before_sqlx_sees_it() {
    // `parse.rs:107` would log `foo=bar` at warn — with the value — instead of refusing.
    assert_eq!(
        refusal("postgres://h/d?foo=bar"),
        DsnError::UnrecognisedParameter
    );
    assert_eq!(
        refusal("postgres://h/d?sslmode=require&foo=bar"),
        DsnError::UnrecognisedParameter
    );
    assert_eq!(
        refusal("postgres://h/d?foo=bar").to_string(),
        "unrecognised parameter"
    );
    for (key, value) in KNOWN {
        let dsn = format!("postgres://h/d?{key}={value}");
        assert!(Dsn::parse(&dsn).is_ok(), "{key} is a key sqlx handles");
    }
    assert!(Dsn::parse("postgres://h/d?options[search_path]=htui").is_ok());
    // `options[` without its `]` does **not** reach sqlx's `warn!` arm: the
    // `k if k.starts_with("options[")` arm matches first and its `strip_suffix(']')` answers
    // `None`, so the key and its value are dropped in silence (`options/parse.rs:101-105`).
    // Refused all the same - a parameter the driver ignores is a DSN that does not mean what it
    // says - which is why this assertion is here and must not change.
    assert_eq!(
        refusal("postgres://h/d?options[search_path=htui"),
        DsnError::UnrecognisedParameter
    );
}

#[test]
fn the_error_never_carries_the_text() {
    for text in [
        "postgres://htui:s3cret@db.example:5432/htui?foo=bar",
        "postgres://htui:s3cret@db.example:5432/htui?sslmode=maybe",
        "postgres://htui:s3cret@db.example:70000/htui",
        "postgres://htui:s3cret@/htui",
        "htui:s3cret@db.example",
    ] {
        let err = refusal(text);
        for rendered in [format!("{err}"), format!("{err:?}")] {
            assert!(
                !rendered.contains("s3cret"),
                "{rendered} leaks the password"
            );
            assert!(
                !rendered.contains("db.example"),
                "{rendered} leaks the host"
            );
            assert!(!rendered.contains("htui"), "{rendered} leaks the user");
            assert!(!rendered.contains("maybe"), "{rendered} leaks the value");
            assert!(!rendered.contains("foo"), "{rendered} leaks the key");
        }
    }
}

#[test]
fn debug_is_redacted() {
    let dsn = Dsn::parse("postgres://u:s3cret@h:5432/d").expect("parse");
    assert_eq!(format!("{dsn:?}"), "Dsn(<redacted>)");
    // `StoreRequest` and `RequestEnvelope` both derive `Debug`, so a nested print must be redacted
    // too (ANA-10 §4.9 (3)).
    assert_eq!(format!("{:?}", Some(dsn)), "Some(Dsn(<redacted>))");
}

#[test]
fn the_summary_names_everything_but_the_password() {
    let dsn = Dsn::parse(FULL).expect("parse");
    let summary = dsn.summary();
    assert_eq!(
        summary,
        "postgres://htui@db.example:5432/htui · sslmode=require"
    );
    assert!(
        !summary.contains("s3cret"),
        "{summary} carries the password"
    );
}

#[test]
fn the_summary_omits_a_missing_database() {
    let dsn = Dsn::parse("postgres://u@h:1/").expect("parse");
    assert_eq!(dsn.summary(), "postgres://u@h:1 · sslmode=prefer");
}

#[test]
fn credentials_do_not_change_the_fingerprint() {
    let one = Dsn::parse("postgres://a:x@h:5432/d").expect("parse");
    let two = Dsn::parse("postgres://b:y@h:5432/d").expect("parse");
    assert_eq!(one.fingerprint(), two.fingerprint());
    assert_eq!(
        one.fingerprint(),
        identity::db_fingerprint("postgres://h:5432/d"),
        "the newtype restates `identity.rs`'s rule rather than a second one"
    );
}

#[tokio::test]
async fn apply_dsn_stores_then_opens_the_new_mirror() {
    let _keyring = testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("temp root");
    let dsn = Dsn::parse(FULL).expect("parse");

    let applied = connect::apply_dsn(dsn.clone(), &context(root.path()))
        .await
        .expect("apply");

    assert_eq!(applied.fingerprint, dsn.fingerprint());
    assert_eq!(applied.summary, dsn.summary());
    assert!(
        applied.cache.dir().ends_with(dsn.fingerprint()),
        "the mirror is opened under the new DSN's fingerprint: {}",
        applied.cache.dir().display()
    );
    assert_eq!(
        testkit::fake_dsn(),
        Some(FULL.to_owned()),
        "the keyring is written before the mirror is opened (D11)"
    );
    assert!(!format!("{applied:?}").contains("s3cret"));
    applied.cache.close().await;
}

#[tokio::test]
async fn apply_dsn_leaves_the_old_mirror_on_disk() {
    let _keyring = testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("temp root");

    let old = Dsn::parse("postgres://htui@old.example:5432/htui").expect("parse");
    let old_cache = CacheStore::open(root.path(), &old.fingerprint(), PgStore::schema_version())
        .await
        .expect("open the old mirror");
    let old_dir = old_cache.dir().to_owned();
    old_cache.close().await;

    let new = Dsn::parse("postgres://htui@new.example:5432/htui").expect("parse");
    let applied = connect::apply_dsn(new, &context(root.path()))
        .await
        .expect("apply");

    assert_ne!(applied.cache.dir(), old_dir);
    assert!(
        old_dir.join("cache.sqlite").exists(),
        "the old file survives"
    );
    assert!(applied.cache.dir().join("cache.sqlite").exists());
    applied.cache.close().await;
}

#[tokio::test]
async fn forget_dsn_on_an_empty_keyring_is_ok() {
    let _keyring = testkit::mock_keyring().await;
    let root = tempfile::tempdir().expect("temp root");

    connect::forget_dsn().await.expect("an empty keyring");
    assert_eq!(testkit::fake_dsn(), None);

    let applied = connect::apply_dsn(Dsn::parse(FULL).expect("parse"), &context(root.path()))
        .await
        .expect("apply");
    applied.cache.close().await;
    assert_eq!(testkit::fake_dsn(), Some(FULL.to_owned()));

    connect::forget_dsn().await.expect("clear");
    assert_eq!(testkit::fake_dsn(), None);
}

#[tokio::test]
async fn the_fake_keyring_round_trips() {
    // keyring's own mock keeps the secret in the `Entry`, and `secret.rs` opens a fresh `Slot` per
    // call, so a `set_dsn` there is invisible to the next `get_dsn` (blueprint flag L). This is the
    // property the fake slot exists for.
    let _keyring = testkit::mock_keyring().await;

    assert_eq!(secret::get_dsn().expect("read"), None);
    secret::set_dsn("postgres://u@h/d").expect("store");
    assert_eq!(
        secret::get_dsn().expect("read"),
        Some("postgres://u@h/d".to_owned())
    );
    secret::set_dsn("   ").expect("store blank");
    assert_eq!(
        secret::get_dsn().expect("read"),
        None,
        "a blank entry reads as no DSN, exactly as the real slot does"
    );
    secret::set_dsn("postgres://u@h/d").expect("store");
    secret::clear_dsn().expect("clear");
    assert_eq!(secret::get_dsn().expect("read"), None);
    secret::clear_dsn().expect("clearing twice is not an error");
}

/// A keyring that cannot be opened is **not** a keyring with nothing in it (the M6 follow-up fix).
///
/// Mapping the failure to `Ok(None)` here would tell a user whose collection is merely locked that
/// they have no DSN, and invite them to retype a credential into a store that cannot hold it. The
/// distinction is the asset; it is `connect::start` that decides to launch anyway.
#[tokio::test]
async fn a_broken_keyring_is_an_error_and_not_an_empty_one() {
    let _keyring = testkit::mock_keyring_broken().await;

    let err = secret::get_dsn().expect_err("an unreadable keyring is not an empty keyring");
    let sentence = err.to_string();
    assert!(
        sentence.contains("cannot read the keyring entry (htui/postgres-dsn)")
            && sentence.contains(testkit::BROKEN_KEYRING),
        "the seam names the entry and quotes the platform: {sentence}"
    );

    secret::set_dsn("postgres://u@h/d").expect_err("a write fails too");
    secret::clear_dsn().expect_err("and so does a delete");
}
