//! The DSN as a redacting newtype (MOD-15 milestone 6, D1–D3; `docs/ANA-10.md` §4.9).
//!
//! [`Dsn::parse`] is the only way to make one. It refuses with one of five fixed sentences
//! ([`DsnError`]) that never carry any of the text, and it refuses **before** sqlx sees the
//! string, because sqlx logs an unrecognised query parameter's value at `warn` rather than
//! rejecting it (sqlx-postgres 0.9.0 `options/parse.rs:107`,
//! `_ => tracing::warn!(%key, %value, "ignoring unrecognized connect parameter")`). A DSN typed
//! into the Settings field would otherwise put its parameter values into the `--log` file.
//!
//! The text never leaves this crate: `Dsn::as_str` is `pub(crate)` and there is no `Display`.
//! Everything an outside caller can ask for — the redacted [`Dsn::summary`], the mirror's
//! [`Dsn::fingerprint`] — is derived here.

use core::fmt;
use core::str::FromStr as _;

use sqlx::postgres::{PgConnectOptions, PgSslMode};
use zeroize::Zeroizing;

use crate::identity;

/// What [`Dsn::summary`] says when the stored text parsed once and no longer does.
///
/// Unreachable in practice — [`Dsn::parse`] ran `PgConnectOptions::from_str` on this very text —
/// and spelled out rather than `unwrap`ped, because a panic here would be a panic in a render.
const STORED: &str = "stored";

/// A connection string that passed [`Dsn::parse`].
///
/// Prints as `Dsn(<redacted>)`; the text is reachable only through `Dsn::as_str`, which is
/// `pub(crate)`. Cloned into the store worker's request and moved into the keyring write.
#[derive(Clone)]
pub struct Dsn(Zeroizing<String>);

impl fmt::Debug for Dsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dsn(<redacted>)")
    }
}

/// Why a string is not a DSN this build stores (D2).
///
/// Five sentences and nothing else: no sqlx text, no fragment of what was typed. sqlx's own
/// refusal quotes the `sslmode` value (`options/ssl_mode.rs:48`) and carries `ParseIntError` text
/// elsewhere, so it is never rendered — `Dsn::parse` maps every residual failure to
/// [`DsnError::NotAUrl`] without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DsnError {
    /// Not `postgres://` / `postgresql://`, or a shape sqlx's parser still refuses after the scan.
    #[error("not a URL")]
    NotAUrl,
    /// The authority has no host.
    #[error("no host")]
    NoHost,
    /// `sslmode=` is not one of the six names.
    #[error("unsupported sslmode")]
    UnsupportedSslMode,
    /// A `:port` or `port=` that is not a `u16`.
    #[error("port out of range")]
    PortOutOfRange,
    /// A query key sqlx would log rather than use.
    #[error("unrecognised parameter")]
    UnrecognisedParameter,
}

/// The query keys sqlx-postgres 0.9.0 handles (`options/parse.rs:51-105`), plus the
/// `options[<name>]` family checked separately. Anything else reaches the `warn!` arm with its
/// value, which is what the scan exists to prevent.
///
/// One key is refused without being a `warn!`: an **unclosed** `options[search_path` matches
/// sqlx's `k if k.starts_with("options[")` arm first, its `strip_suffix(']')` answers `None`, and
/// the key and its value are dropped in silence (`options/parse.rs:101-105`). The refusal stands
/// anyway — a parameter the user wrote and the driver ignored is a DSN that does not mean what it
/// says — it is simply not the logging hazard the rest of this list is about.
const KNOWN_PARAMETERS: [&str; 18] = [
    "sslmode",
    "ssl-mode",
    "sslrootcert",
    "ssl-root-cert",
    "ssl-ca",
    "sslcert",
    "ssl-cert",
    "sslkey",
    "ssl-key",
    "statement-cache-capacity",
    "host",
    "hostaddr",
    "port",
    "dbname",
    "user",
    "password",
    "application_name",
    "options",
];

/// `PgSslMode::from_str`'s six spellings (`options/ssl_mode.rs:38-45`), matched case-insensitively
/// as it does.
const SSL_MODES: [&str; 6] = [
    "disable",
    "allow",
    "prefer",
    "require",
    "verify-ca",
    "verify-full",
];

/// The two schemes `PgConnectOptions::from_str` accepts.
const SCHEMES: [&str; 2] = ["postgres://", "postgresql://"];

impl Dsn {
    /// Validates `text` and takes a zeroizing copy of it.
    ///
    /// The `scan` runs first and sqlx second: an unrecognised query parameter must be refused
    /// **before** `PgConnectOptions::from_str` is called, because that call is what logs it.
    ///
    /// # Errors
    ///
    /// One [`DsnError`]; see the type. Nothing is logged on any path.
    pub fn parse(text: &str) -> Result<Self, DsnError> {
        scan(text)?;
        PgConnectOptions::from_str(text).map_err(|_| DsnError::NotAUrl)?;
        Ok(Self(Zeroizing::new(text.to_owned())))
    }

    /// `postgres://user@host:port/db · sslmode=mode` — never the password (there is no getter for
    /// it on `PgConnectOptions`), never the query string.
    ///
    /// `/db` is omitted when the DSN names no database, and a socket DSN renders the socket path
    /// instead of `host:port`. Re-parses rather than caching (B-2), so the type stays one field
    /// and there is no second copy of anything.
    #[must_use]
    pub fn summary(&self) -> String {
        let Ok(options) = PgConnectOptions::from_str(&self.0) else {
            return STORED.to_owned();
        };
        let user = options.get_username();
        let mode = ssl_mode_name(options.get_ssl_mode());
        let target = options.get_socket().map_or_else(
            || format!("{}:{}", options.get_host(), options.get_port()),
            |socket| socket.display().to_string(),
        );
        let database = options
            .get_database()
            .map_or_else(String::new, |name| format!("/{name}"));
        format!("postgres://{user}@{target}{database} · sslmode={mode}")
    }

    /// [`identity::db_fingerprint`] of the text: the mirror directory name.
    ///
    /// Credentials and query parameters are not hashed (`identity.rs:126-144`), so rotating a
    /// password keeps the same mirror.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        identity::db_fingerprint(&self.0)
    }

    /// The text, for this crate's keyring write, dial and fingerprint. Deliberately not `pub`.
    ///
    /// Four callers, all inside `htui-store`: [`crate::connect::apply_dsn`]'s keyring write,
    /// [`crate::connect::reconnect_for`]'s per-dial copy, [`Dsn::fingerprint`] and
    /// [`Dsn::summary`]. A test in another crate cannot call it, which is the point (D1, flag B).
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pre-scan (flag H): everything sqlx would log, quote or default silently is caught here.
///
/// Hand-split rather than routed through a `url` crate this workspace does not declare: the four
/// steps are scheme, authority, host/port and the query's keys, and each returns the one variant
/// the caller renders. A percent-encoded key is refused as unrecognised — sqlx decodes before
/// matching, so refusing is the safe direction.
fn scan(text: &str) -> Result<(), DsnError> {
    // 1. The field already swallows control characters (`text_field.rs`); this is the belt.
    if text.chars().any(char::is_control) {
        return Err(DsnError::NotAUrl);
    }

    // 2. Scheme.
    let rest = SCHEMES
        .iter()
        .find_map(|scheme| text.strip_prefix(scheme))
        .ok_or(DsnError::NotAUrl)?;

    // 3. Authority, then host and port. The userinfo ends at the **last** `@`, so a password
    //    containing one does not move the host.
    let (before, query) = rest.split_once('?').map_or((rest, ""), |(b, q)| (b, q));
    let authority = before.split_once('/').map_or(before, |(a, _)| a);
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = split_host_port(hostport)?;
    if host.is_empty() {
        return Err(DsnError::NoHost);
    }
    if let Some(port) = port
        && port.parse::<u16>().is_err()
    {
        return Err(DsnError::PortOutOfRange);
    }

    // 4. Every query key against the allow-list, before sqlx's `warn!` can see one.
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !is_known(key) {
            return Err(DsnError::UnrecognisedParameter);
        }
        match key {
            "sslmode" | "ssl-mode" => {
                if !SSL_MODES.contains(&value.to_ascii_lowercase().as_str()) {
                    return Err(DsnError::UnsupportedSslMode);
                }
            }
            "port" if value.parse::<u16>().is_err() => return Err(DsnError::PortOutOfRange),
            _ => {}
        }
    }
    Ok(())
}

/// Splits `host[:port]`, handling an IPv6 literal's `[..]` brackets.
///
/// A trailing `:tail` that is not all digits is left inside the host: sqlx's URL parser is the
/// authority on that shape and refuses it as [`DsnError::NotAUrl`].
fn split_host_port(hostport: &str) -> Result<(&str, Option<&str>), DsnError> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let end = rest.find(']').ok_or(DsnError::NotAUrl)?;
        let tail = &rest[end + 1..];
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port),
            None if tail.is_empty() => None,
            None => return Err(DsnError::NotAUrl),
        };
        return Ok((&rest[..end], port));
    }
    match hostport.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            Ok((host, Some(port)))
        }
        _ => Ok((hostport, None)),
    }
}

/// Whether sqlx's `match` has an arm for `key`, including the `options[<name>]` family.
fn is_known(key: &str) -> bool {
    KNOWN_PARAMETERS.contains(&key) || (key.starts_with("options[") && key.ends_with(']'))
}

/// The six spellings, because `PgSslMode` implements `Debug` and no `Display`
/// (`options/ssl_mode.rs:7`). Exhaustive: the enum is not `#[non_exhaustive]` on 0.9.0.
fn ssl_mode_name(mode: PgSslMode) -> &'static str {
    match mode {
        PgSslMode::Disable => "disable",
        PgSslMode::Allow => "allow",
        PgSslMode::Prefer => "prefer",
        PgSslMode::Require => "require",
        PgSslMode::VerifyCa => "verify-ca",
        PgSslMode::VerifyFull => "verify-full",
    }
}
