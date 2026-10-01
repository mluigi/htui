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
use core::net::IpAddr;
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

/// Where a DSN's server is, as another machine would read the DSN (MOD-45 D306).
///
/// Carries no text. `htui provision` refuses `Loopback` and `Socket`: the remote host would reach
/// itself, not this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsnHost {
    /// A host name or an address another machine can mean the same server by.
    Remote,
    /// `localhost` (any case, trailing dot, `*.localhost`), `127.0.0.0/8`, `::1`, an IPv4-mapped
    /// loopback, or the unspecified `0.0.0.0` / `::`, which also reach this machine.
    Loopback,
    /// A Unix-socket directory (`?host=/…` or a percent-encoded `/` host).
    Socket,
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

    /// The class of the host sqlx would dial (E-2): the URL authority's host, unless a later
    /// `host=` or `hostaddr=` query parameter replaces it, and `Socket` once any `host=/…` set a
    /// socket.
    ///
    /// Re-parses with `PgConnectOptions::from_str`, as [`Dsn::summary`] does. That is safe because
    /// `scan` already refused every parameter sqlx would log. It is never an environment default
    /// (`PGHOST`, a socket probe): `scan` refused a DSN without an authority host
    /// ([`DsnError::NoHost`]) before this value existed. The unreachable parse failure answers
    /// `Loopback`, the refusing direction.
    #[must_use]
    pub fn host_class(&self) -> DsnHost {
        let Ok(options) = PgConnectOptions::from_str(&self.0) else {
            return DsnHost::Loopback;
        };
        if options.get_socket().is_some() {
            return DsnHost::Socket;
        }
        classify_host(options.get_host())
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

/// `host` as sqlx holds it: brackets stripped, then the name and address rules of [`DsnHost`].
fn classify_host(host: &str) -> DsnHost {
    let host = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return DsnHost::Loopback;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) if v4.is_loopback() || v4.is_unspecified() => DsnHost::Loopback,
        Ok(IpAddr::V6(v6))
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()) =>
        {
            DsnHost::Loopback
        }
        _ => DsnHost::Remote,
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

#[cfg(test)]
mod tests {
    use super::{Dsn, DsnError, DsnHost, classify_host};

    /// What `htui provision` reads off a DSN (MOD-45 D306; E-1, E-2).
    #[test]
    fn host_class_reads_the_host_sqlx_would_dial() {
        let cases: [(&str, Result<DsnHost, DsnError>); 15] = [
            ("postgres://u:p@localhost/db", Ok(DsnHost::Loopback)),
            ("postgres://u:p@LOCALHOST./db", Ok(DsnHost::Loopback)),
            ("postgres://u:p@127.0.0.5/db", Ok(DsnHost::Loopback)),
            ("postgres://u:p@[::1]:5432/db", Ok(DsnHost::Loopback)),
            ("postgres://u:p@0.0.0.0/db", Ok(DsnHost::Loopback)),
            (
                "postgres://u:p@localhost/db?host=/var/run/postgresql",
                Ok(DsnHost::Socket),
            ),
            (
                "postgres://u:p@%2Fvar%2Frun%2Fpostgresql/db",
                Ok(DsnHost::Socket),
            ),
            (
                "postgres://u:p@/db?host=/var/run/postgresql",
                Err(DsnError::NoHost),
            ),
            ("postgres:///db", Err(DsnError::NoHost)),
            ("postgres://u:p@db.example/db", Ok(DsnHost::Remote)),
            ("postgres://u:p@10.0.0.3/db", Ok(DsnHost::Remote)),
            ("postgres://u:p@[fd00::3]/db", Ok(DsnHost::Remote)),
            (
                "postgres://u:p@db.example/db?hostaddr=127.0.0.1",
                Ok(DsnHost::Loopback),
            ),
            (
                "postgres://u:p@db.example/db?host=localhost",
                Ok(DsnHost::Loopback),
            ),
            (
                "postgres://u:p@localhost/db?host=db.example",
                Ok(DsnHost::Remote),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                Dsn::parse(text).map(|dsn| dsn.host_class()),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn nothing_prints_the_password() {
        const SENTINEL: &str = "SENTINEL-DSN-PW";
        let dsn = Dsn::parse("postgres://u:SENTINEL-DSN-PW@localhost/db").expect("a DSN");
        assert!(!format!("{dsn:?}").contains(SENTINEL));
        assert!(!format!("{:?}", dsn.host_class()).contains(SENTINEL));

        let err = Dsn::parse("postgres://u:SENTINEL-DSN-PW@/db").expect_err("no host");
        assert!(!err.to_string().contains(SENTINEL));
        assert!(!format!("{err:?}").contains(SENTINEL));
    }

    #[test]
    fn classify_host_strips_brackets_and_the_trailing_dot() {
        let cases = [
            ("localhost", DsnHost::Loopback),
            ("localhost.", DsnHost::Loopback),
            ("LocalHost", DsnHost::Loopback),
            ("db.localhost", DsnHost::Loopback),
            ("DB.LOCALHOST.", DsnHost::Loopback),
            ("notlocalhost", DsnHost::Remote),
            ("localhost.example", DsnHost::Remote),
            ("127.0.0.1", DsnHost::Loopback),
            ("127.255.255.254", DsnHost::Loopback),
            ("0.0.0.0", DsnHost::Loopback),
            ("[::1]", DsnHost::Loopback),
            ("::1", DsnHost::Loopback),
            ("[::]", DsnHost::Loopback),
            ("[::ffff:127.0.0.1]", DsnHost::Loopback),
            ("[::ffff:10.0.0.3]", DsnHost::Remote),
            ("[fd00::3]", DsnHost::Remote),
            ("10.0.0.3", DsnHost::Remote),
            ("db.example.", DsnHost::Remote),
            ("db.example", DsnHost::Remote),
        ];
        for (host, expected) in cases {
            assert_eq!(classify_host(host), expected, "{host}");
        }
    }

    /// The shorthand IPv4 spellings `inet_aton` accepts. sqlx hands the host to the resolver
    /// verbatim, and glibc's `getaddrinfo` reads `127.1`, `2130706433` and `0x7f.1` as
    /// `127.0.0.1` and `0` as `0.0.0.0`, so they reach this machine.
    #[test]
    fn classify_host_reads_the_inet_aton_spellings() {
        let cases = [
            ("127.1", DsnHost::Loopback),
            ("127.0.1", DsnHost::Loopback),
            ("2130706433", DsnHost::Loopback),
            ("0x7f000001", DsnHost::Loopback),
            ("0X7F.1", DsnHost::Loopback),
            ("0177.0.0.1", DsnHost::Loopback),
            ("127.000.000.001", DsnHost::Loopback),
            ("0", DsnHost::Loopback),
            ("0.0", DsnHost::Loopback),
            ("127.1.", DsnHost::Loopback),
            ("10.3", DsnHost::Remote),
            ("167772163", DsnHost::Remote),
            ("08.1", DsnHost::Remote),
            ("127.0.0.256", DsnHost::Remote),
            ("127.16777216", DsnHost::Remote),
            ("1.2.3.4.5", DsnHost::Remote),
            ("127..1", DsnHost::Remote),
            ("0x", DsnHost::Remote),
            ("deadbeef", DsnHost::Remote),
            ("4294967296", DsnHost::Remote),
        ];
        for (host, expected) in cases {
            assert_eq!(classify_host(host), expected, "{host}");
        }

        for text in [
            "postgres://u:p@127.1/db",
            "postgres://u:p@2130706433:5432/db",
            "postgres://u:p@db.example/db?host=0x7f.1",
        ] {
            assert_eq!(
                Dsn::parse(text).map(|dsn| dsn.host_class()),
                Ok(DsnHost::Loopback),
                "{text}"
            );
        }
    }
}
