//! The Qdrant URL every box uses (MOD-158 D1, D2; `R-STO-1`): one `app_setting` row, validated by
//! [`QdrantUrl::parse`] wherever a URL enters htui (setter, env override, CLI, TUI field).

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::store::{Result, StoreError};

/// `app_setting.key` of the URL (D1). The value is a JSON string.
pub const QDRANT_URL_KEY: &str = "qdrant_url";

/// A Qdrant base URL that passed D2: trimmed; `http://` or `https://` (ASCII case-insensitive,
/// stored lowercase); a non-empty host, either a name of letters, digits, `-._~` and `%XX` escapes
/// or a bracketed IPv6 literal (with an optional `%25` zone id); an optional port 1..=65535; an
/// optional path; no userinfo, query, fragment, and no character outside printable ASCII
/// (whitespace, control and format characters included).
/// Not secret: `Debug`/`Display` print it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QdrantUrl(String);

impl QdrantUrl {
    /// Checks `text` against D2, in a fixed order so the refusal is deterministic: empty, space,
    /// scheme, query, fragment, credentials, host (present, then its characters), port.
    ///
    /// # Errors
    /// [`QdrantUrlError`]; never echoes `text`.
    pub fn parse(text: &str) -> core::result::Result<Self, QdrantUrlError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(QdrantUrlError::Empty);
        }
        // Printable ASCII only: `is_control` is just Cc, so a zero-width space or a bidi override
        // (Cf) would draw a URL on the terminal unlike the one the gRPC client is handed.
        if !text.chars().all(|c| c.is_ascii_graphic()) {
            return Err(QdrantUrlError::Space);
        }
        let Some((scheme, rest)) = text.split_once("://") else {
            return Err(QdrantUrlError::Scheme);
        };
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(QdrantUrlError::Scheme);
        }
        if rest.contains('?') {
            return Err(QdrantUrlError::Query);
        }
        if rest.contains('#') {
            return Err(QdrantUrlError::Fragment);
        }
        // An `@` anywhere, not only in the authority: a password holding a `/` would otherwise move
        // the rest of the userinfo into the path and pass.
        if rest.contains('@') {
            return Err(QdrantUrlError::Credentials);
        }
        let authority = rest
            .split_once('/')
            .map_or(rest, |(authority, _)| authority);
        check_authority(authority)?;
        Ok(Self(format!("{scheme}://{rest}")))
    }

    /// D3's override: `None` or a blank value is unset (`Ok(None)`); anything else must parse.
    ///
    /// # Errors
    /// [`QdrantUrlError`] for a non-blank value that is not a Qdrant URL.
    pub fn from_env(value: Option<&str>) -> core::result::Result<Option<Self>, QdrantUrlError> {
        match value {
            Some(value) if !value.trim().is_empty() => Self::parse(value).map(Some),
            _ => Ok(None),
        }
    }

    /// The URL as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The `app_setting.value` this URL is stored as.
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

impl core::fmt::Display for QdrantUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The host and the optional port: a bracketed IPv6 literal, or a name up to the last `:`.
fn check_authority(authority: &str) -> core::result::Result<(), QdrantUrlError> {
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((literal, after)) = bracketed.split_once(']') else {
            return Err(QdrantUrlError::NoHost);
        };
        let port = match after {
            "" => None,
            after => Some(after.strip_prefix(':').ok_or(QdrantUrlError::Port)?),
        };
        if !is_ipv6_literal(literal) {
            return Err(QdrantUrlError::NoHost);
        }
        (literal, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() || (!authority.starts_with('[') && host.contains(':')) {
        return Err(QdrantUrlError::NoHost);
    }
    if !authority.starts_with('[') && !is_reg_name(host) {
        return Err(QdrantUrlError::Host);
    }
    if let Some(port) = port {
        let valid = !port.is_empty()
            && port.bytes().all(|b| b.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|port| port != 0);
        if !valid {
            return Err(QdrantUrlError::Port);
        }
    }
    Ok(())
}

/// A host name as RFC 3986's reg-name, without its sub-delims (MOD-158 R1, H2): letters, digits,
/// `-._~` and `%XX` escapes. Sub-delims (`!$&'()*+,;=`) are legal in a URI's reg-name but in no
/// host a Qdrant client resolves (a DNS name or an IPv4 literal), so they are refused with the
/// characters no URI allows there (`"`, `\`, `[`, `]`, `<>`, `^`, `` ` ``, `{|}`).
fn is_reg_name(host: &str) -> bool {
    let bytes = host.as_bytes();
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%' {
            let escape = bytes.get(at + 1..at + 3);
            if !escape.is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) {
                return false;
            }
            at += 3;
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            at += 1;
        } else {
            return false;
        }
    }
    true
}

/// An IPv6 address, optionally followed by a non-empty `%25` zone id (RFC 6874).
fn is_ipv6_literal(literal: &str) -> bool {
    let (address, zone) = match literal.split_once("%25") {
        Some((address, zone)) => (address, Some(zone)),
        None => (literal, None),
    };
    zone.is_none_or(|zone| !zone.is_empty()) && address.parse::<core::net::Ipv6Addr>().is_ok()
}

/// Why a text is not a Qdrant URL. No variant carries the text: a refused URL may hold a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QdrantUrlError {
    /// Nothing but whitespace.
    #[error("the Qdrant URL is empty")]
    Empty,
    /// A character outside printable ASCII inside: whitespace, a control or a format character.
    #[error(
        "a Qdrant URL may only contain printable ASCII characters (no spaces or control characters)"
    )]
    Space,
    /// Not `http://` or `https://`.
    #[error("a Qdrant URL starts with http:// or https://")]
    Scheme,
    /// No host after the scheme.
    #[error("a Qdrant URL needs a host")]
    NoHost,
    /// A host name holding a character outside letters, digits, `-._~` and `%XX` (MOD-158 R1).
    #[error(
        "the host of a Qdrant URL may only contain letters, digits, `-`, `.`, `_`, `~` and `%XX` \
         escapes, or be an IPv6 address in brackets"
    )]
    Host,
    /// A port that is not 1..=65535.
    #[error("the port of a Qdrant URL is a number from 1 to 65535")]
    Port,
    /// A user name or password (`R-STO-1`: a URL that carries credentials is refused).
    #[error(
        "a Qdrant URL may not carry a user name or password; store the API key in Settings > \
         Qdrant or with `htui --set-qdrant-key`"
    )]
    Credentials,
    /// A query string.
    #[error("a Qdrant URL may not carry a query (`?...`)")]
    Query,
    /// A fragment.
    #[error("a Qdrant URL may not carry a fragment (`#...`)")]
    Fragment,
    /// A stored value that is not a JSON string.
    #[error("the stored value is not text")]
    NotAString,
}

/// The `qdrant_url` row: its value as D2 reads it, and its compare-and-set token
/// (`app_setting.updated_at`). A malformed value is kept with its token (blueprint P-3), so it can
/// be overwritten or cleared; [`Self::usable`] is how every resolver reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQdrantUrl {
    /// The URL, or why the stored value is not one.
    pub url: core::result::Result<QdrantUrl, QdrantUrlError>,
    /// `app_setting.updated_at`: the token a write presents.
    pub token: DateTime<Utc>,
}

impl StoredQdrantUrl {
    /// From a stored `(value, updated_at)`: a JSON string that parses, else `Err(why)`.
    #[must_use]
    pub fn from_row(value: &Value, token: DateTime<Utc>) -> Self {
        let url = value
            .as_str()
            .ok_or(QdrantUrlError::NotAString)
            .and_then(QdrantUrl::parse);
        Self { url, token }
    }

    /// D1: the URL, or an error naming the key, never "no URL".
    ///
    /// # Errors
    /// `StoreError::Backend(stored_url_unusable(why))`.
    pub fn usable(&self) -> Result<&QdrantUrl> {
        self.url
            .as_ref()
            .map_err(|why| StoreError::Backend(stored_url_unusable(*why)))
    }
}

/// D1's error sentence for a malformed row.
#[must_use]
pub fn stored_url_unusable(why: QdrantUrlError) -> String {
    format!(
        "the Qdrant URL setting (`app_setting` `{QDRANT_URL_KEY}`) is not usable: {why}; set it \
         again in Settings > Qdrant or with `htui --set-qdrant-url`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(text: &str) -> QdrantUrlError {
        QdrantUrl::parse(text).expect_err(text)
    }

    #[test]
    fn a_plain_http_url_with_a_port_parses() {
        let url = QdrantUrl::parse("http://host:6334").expect("a plain URL");
        assert_eq!(url.as_str(), "http://host:6334");
        assert_eq!(url.to_string(), "http://host:6334");
        assert_eq!(url.to_value(), Value::String("http://host:6334".into()));
    }

    #[test]
    fn https_a_path_and_surrounding_space_parse_trimmed() {
        let url = QdrantUrl::parse("  https://qdrant.example.com/base/path \n").expect("https");
        assert_eq!(url.as_str(), "https://qdrant.example.com/base/path");
        assert!(QdrantUrl::parse("https://q.example.com").is_ok());
    }

    #[test]
    fn the_scheme_is_lowercased_and_only_http_or_https() {
        assert_eq!(
            QdrantUrl::parse("HTTP://h").expect("upper").as_str(),
            "http://h"
        );
        assert_eq!(
            QdrantUrl::parse("HttpS://H").expect("mixed").as_str(),
            "https://H"
        );
        assert_eq!(refused("localhost:6334"), QdrantUrlError::Scheme);
        assert_eq!(refused("grpc://h"), QdrantUrlError::Scheme);
        assert_eq!(refused("http:/h"), QdrantUrlError::Scheme);
    }

    #[test]
    fn userinfo_is_refused_and_never_echoed() {
        assert_eq!(refused("http://u@h"), QdrantUrlError::Credentials);
        let err = refused("http://u:SENTINEL@h:6334");
        assert_eq!(err, QdrantUrlError::Credentials);
        assert!(!err.to_string().contains("SENTINEL"), "{err}");
        assert!(!format!("{err:?}").contains("SENTINEL"), "{err:?}");
        // A password holding a `/` cannot slip the rest of the userinfo into the path.
        assert_eq!(refused("http://u:a/b@h"), QdrantUrlError::Credentials);
    }

    #[test]
    fn a_query_is_refused() {
        assert_eq!(refused("http://h:6334/?api-key=x"), QdrantUrlError::Query);
        assert_eq!(refused("http://h?x"), QdrantUrlError::Query);
    }

    #[test]
    fn a_fragment_is_refused() {
        assert_eq!(refused("http://h:6334/#top"), QdrantUrlError::Fragment);
        assert_eq!(refused("http://h#"), QdrantUrlError::Fragment);
    }

    #[test]
    fn an_empty_host_is_refused() {
        assert_eq!(refused("http://"), QdrantUrlError::NoHost);
        assert_eq!(refused("http://:6334"), QdrantUrlError::NoHost);
        assert_eq!(refused("https:///p"), QdrantUrlError::NoHost);
        assert_eq!(refused("http://[]:6334"), QdrantUrlError::NoHost);
        assert_eq!(refused("http://a:b:6334"), QdrantUrlError::NoHost);
    }

    #[test]
    fn a_bad_port_is_refused() {
        for text in [
            "http://h:",
            "http://h:abc",
            "http://h:0",
            "http://h:70000",
            "http://h:+1",
            "http://[::1]x",
        ] {
            assert_eq!(refused(text), QdrantUrlError::Port, "{text}");
        }
        assert!(QdrantUrl::parse("http://h:65535").is_ok());
        assert!(QdrantUrl::parse("http://h:1/").is_ok());
    }

    #[test]
    fn whitespace_or_control_inside_is_refused() {
        assert_eq!(refused("http://h 1"), QdrantUrlError::Space);
        assert_eq!(refused("http://h\u{1b}[2J"), QdrantUrlError::Space);
        assert_eq!(refused("http://h\t:6334"), QdrantUrlError::Space);
        // Format characters (Cf) are not `is_control`, yet draw a URL unlike what it is.
        for text in [
            "http://h\u{200B}:6334",
            "http://h\u{202E}x",
            "http://u\u{FF20}h",
            "http://h\u{00A0}:6334",
            "http://h\u{FEFF}",
        ] {
            assert_eq!(refused(text), QdrantUrlError::Space, "{text:?}");
        }
        assert_eq!(refused(""), QdrantUrlError::Empty);
        assert_eq!(refused(" \n "), QdrantUrlError::Empty);
    }

    #[test]
    fn an_ipv6_literal_parses() {
        let url = QdrantUrl::parse("http://[::1]:6334").expect("IPv6 with a port");
        assert_eq!(url.as_str(), "http://[::1]:6334");
        assert!(QdrantUrl::parse("https://[2001:db8::1]/q").is_ok());
        assert_eq!(refused("http://[::1"), QdrantUrlError::NoHost);
        assert!(QdrantUrl::parse("http://[fe80::1%25eth0]:6334").is_ok());
        for text in [
            "http://[u:pw]:6334",
            "http://[h]",
            "http://[1.2.3.4]:6334",
            "http://[::1%25]",
        ] {
            assert_eq!(refused(text), QdrantUrlError::NoHost, "{text}");
        }
    }

    /// MOD-158 R1 (H2): a host that is not a bracketed IPv6 literal is an RFC 3986 reg-name
    /// without sub-delims: letters, digits, `-._~` and `%XX` escapes. Each refused class is `Host`,
    /// whose sentence never echoes the text.
    #[test]
    fn a_host_outside_the_name_characters_is_refused() {
        for text in [
            "http://h\"x:6334",
            "http://h\\x",
            "http://h]:6334",
            "http://h[:6334",
            "http://[::1]x]",
            "http://h<x>",
            "http://h^x",
            "http://h`x",
            "http://h{x}",
            "http://h|x",
            "http://h%",
            "http://h%2",
            "http://h%zz",
            "http://h!x",
            "http://h$x",
            "http://h&x",
            "http://h'x",
            "http://h(x)",
            "http://h*x",
            "http://h+x",
            "http://h,x",
            "http://h;x",
            "http://h=x:6334",
        ] {
            let err = refused(text);
            if text == "http://[::1]x]" {
                assert_eq!(err, QdrantUrlError::Port, "{text}");
                continue;
            }
            assert_eq!(err, QdrantUrlError::Host, "{text}");
            let sentence = err.to_string();
            assert!(
                !sentence.contains("h\"x") && !sentence.contains("h=x"),
                "{sentence}"
            );
        }
        assert_eq!(
            QdrantUrlError::Host.to_string(),
            "the host of a Qdrant URL may only contain letters, digits, `-`, `.`, `_`, `~` and \
             `%XX` escapes, or be an IPv6 address in brackets"
        );
    }

    #[test]
    fn plain_names_ipv4_and_ipv6_hosts_still_parse() {
        for text in [
            "http://localhost:6334",
            "http://qdrant-1.internal_net.example~x:6334/p",
            "http://127.0.0.1:6334",
            "https://10.0.0.7",
            "http://my%2Dhost:6334",
            "http://[::1]:6334",
            "https://[2001:db8::1]/q",
            "http://[fe80::1%25eth0]:6334",
            "http://h/a\"b",
        ] {
            assert!(QdrantUrl::parse(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn from_env_blank_is_unset_and_invalid_is_an_error() {
        assert_eq!(QdrantUrl::from_env(None), Ok(None));
        assert_eq!(QdrantUrl::from_env(Some("")), Ok(None));
        assert_eq!(QdrantUrl::from_env(Some("  \t")), Ok(None));
        assert_eq!(
            QdrantUrl::from_env(Some(" http://localhost:6334 ")),
            Ok(Some(
                QdrantUrl::parse("http://localhost:6334").expect("valid")
            ))
        );
        assert_eq!(
            QdrantUrl::from_env(Some("localhost:6334")),
            Err(QdrantUrlError::Scheme)
        );
    }

    #[test]
    fn a_stored_value_that_is_not_a_string_is_unusable_with_its_token() {
        let token = DateTime::<Utc>::UNIX_EPOCH;
        let row = StoredQdrantUrl::from_row(&serde_json::json!(42), token);
        assert_eq!(row.url, Err(QdrantUrlError::NotAString));
        assert_eq!(row.token, token);
        let row = StoredQdrantUrl::from_row(&serde_json::json!("ftp://h"), token);
        assert_eq!(row.url, Err(QdrantUrlError::Scheme));
        let row = StoredQdrantUrl::from_row(&serde_json::json!("http://h:6334"), token);
        assert_eq!(
            row.usable().expect("usable").as_str(),
            "http://h:6334",
            "a good row is usable"
        );
    }

    #[test]
    fn usable_names_the_key() {
        let row = StoredQdrantUrl::from_row(&serde_json::json!(42), DateTime::<Utc>::UNIX_EPOCH);
        let err = row
            .usable()
            .expect_err("a malformed row is an error, not no URL");
        let text = err.to_string();
        assert!(matches!(err, StoreError::Backend(_)), "{err:?}");
        assert!(text.contains("qdrant_url"), "{text}");
        assert!(text.contains("the stored value is not text"), "{text}");
        assert!(text.contains("htui --set-qdrant-url"), "{text}");
    }
}
