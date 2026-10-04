//! The Infisical provider (MOD-10 D5, D6, D8, D9): base URL policy, the HTTP client, the login
//! with its token reuse and single-flight latch, the secrets list, its merge and its validation.
//!
//! **Nothing here prints a secret.** `InfisicalProvider`'s `Debug` is hand-written; the token
//! state and the wire shapes have no `Debug` at all (H-2); every error is built from a status, a
//! known message, a cleaned non-login server message, a scope field, a key name or a cause chain.

use std::sync::Once;
use std::time::Duration;

use htui_core::secret::{MachineIdentity, SecretError};
use url::{Host, Position, Url};

/// D9: the connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// D9: the whole-request timeout (`install/http.rs`'s `short` client shape).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// `htui/<version>`, so a server log line names the client.
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));

/// Guards [`install_crypto_provider`]: the process installs one default provider, once.
static PROVIDER: Once = Once::new();

/// How to reach an Infisical (D8, D9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfisicalConfig {
    /// The base URL as entered; [`InfisicalProvider::new`] normalises it.
    pub base_url: String,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl InfisicalConfig {
    /// `base_url` with the D9 timeouts.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// The Infisical client (D5, D6, D9). `Debug` is hand-written: base URL and client ID only.
pub struct InfisicalProvider {
    /// Normalised: origin plus optional prefix, no trailing `/`.
    base: String,
    identity: MachineIdentity,
    #[allow(dead_code)] // MOD-10 T3: read by the login and list requests (next commit).
    client: reqwest::Client,
}

impl InfisicalProvider {
    /// `project.secret_provider`'s value for this provider.
    pub const KIND: &'static str = "infisical";

    /// Builds the provider. Opens no socket.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`] when the base URL is refused (see [`normalise_base_url`]), a half
    /// of the identity is blank, or the HTTP client does not build.
    pub fn new(config: InfisicalConfig, identity: MachineIdentity) -> Result<Self, SecretError> {
        let (base, loopback) = parse_base(&config.base_url)?;
        if identity.client_id().trim().is_empty() {
            return Err(SecretError::Config(
                "the machine identity's client ID is empty".to_owned(),
            ));
        }
        if identity.client_secret().trim().is_empty() {
            return Err(SecretError::Config(
                "the machine identity's client secret is empty".to_owned(),
            ));
        }
        install_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(config.connect_timeout)
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none());
        if loopback {
            // D9 / H-5: `system-proxy` would route 127.0.0.1 through `HTTP_PROXY`.
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|e| {
            SecretError::Config(format!("cannot build the HTTP client: {}", cause_chain(e)))
        })?;
        Ok(Self {
            base,
            identity,
            client,
        })
    }

    /// The normalised base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base
    }
}

impl core::fmt::Debug for InfisicalProvider {
    /// `InfisicalProvider { base_url: "…", client_id: "…", .. }`. Never the secret, never the
    /// token, and no lock taken.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InfisicalProvider")
            .field("base_url", &self.base)
            .field("client_id", &self.identity.client_id())
            .finish_non_exhaustive()
    }
}

/// Normalises an Infisical base URL (D8): origin plus optional path prefix, no trailing `/`, a
/// trailing `/api` dropped. M4 calls it before storing a URL.
///
/// Only `https` is accepted, except for a loopback host (`localhost`, `127.0.0.0/8`, `::1`), where
/// `http` is allowed too. A user name, a password, a query or a fragment is refused.
///
/// # Errors
///
/// [`SecretError::Config`], naming the reason. Never echoes the raw input.
pub fn normalise_base_url(raw: &str) -> Result<String, SecretError> {
    parse_base(raw).map(|(base, _)| base)
}

/// `ring`, once; "already installed" ignored (`install/http.rs`, `model.rs`). Must run before
/// `Client::build`, which panics without a provider under `rustls-no-provider` (H-1).
fn install_crypto_provider() {
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// `(normalised, is_loopback)`; [`normalise_base_url`] returns `.0` (§B.4.1).
fn parse_base(raw: &str) -> Result<(String, bool), SecretError> {
    let config = |why: String| SecretError::Config(why);
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(config("the Infisical base URL is empty".to_owned()));
    }
    let url = Url::parse(raw).map_err(|e| {
        config(format!(
            "the Infisical base URL is not an absolute URL ({e})"
        ))
    })?;
    let scheme = url.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(config(format!(
            "the Infisical base URL must use https (got {scheme})"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(config(
            "the Infisical base URL must not carry a user name or password".to_owned(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(config(
            "the Infisical base URL must not carry a query or a fragment".to_owned(),
        ));
    }
    let loopback = url.host().is_some_and(|h| is_loopback(&h));
    if scheme == "http" && !loopback {
        let host = url.host_str().unwrap_or_default();
        return Err(config(format!(
            "the Infisical base URL must use https unless its host is loopback: the client \
             secret would cross the network in plain text to {host}"
        )));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = path
        .strip_suffix("/api")
        .unwrap_or(path)
        .trim_end_matches('/');
    Ok((
        format!("{}{prefix}", &url[..Position::BeforePath]),
        loopback,
    ))
}

/// `localhost`, `127.0.0.0/8` or `::1`; nothing else (not `localhost.`, not `0.0.0.0`).
fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => *name == "localhost",
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// The cause chain of `err.without_url()`: top-level `Display` then every `source()`, joined
/// with `": "`. reqwest's own `Display` appends the URL with its query (H-4), hence `without_url`.
fn cause_chain(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut parts = vec![err.to_string()];
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_are_normalised() {
        let rows: [(&str, &str, bool); 13] = [
            (
                "https://app.infisical.com",
                "https://app.infisical.com",
                false,
            ),
            (
                "https://app.infisical.com/",
                "https://app.infisical.com",
                false,
            ),
            (
                "https://app.infisical.com/api",
                "https://app.infisical.com",
                false,
            ),
            (
                "https://app.infisical.com/api/",
                "https://app.infisical.com",
                false,
            ),
            (
                "  https://eu.infisical.com  ",
                "https://eu.infisical.com",
                false,
            ),
            (
                "HTTPS://Infisical.Example.COM",
                "https://infisical.example.com",
                false,
            ),
            (
                "https://example.com:8443",
                "https://example.com:8443",
                false,
            ),
            (
                "https://example.com/infisical/api/",
                "https://example.com/infisical",
                false,
            ),
            (
                "https://example.com/api/v4",
                "https://example.com/api/v4",
                false,
            ),
            ("http://localhost:8080", "http://localhost:8080", true),
            ("http://127.0.0.1:1234/api", "http://127.0.0.1:1234", true),
            ("http://127.8.9.10", "http://127.8.9.10", true),
            ("http://[::1]:8080", "http://[::1]:8080", true),
        ];
        for (input, want, loopback) in rows {
            assert_eq!(
                parse_base(input),
                Ok((want.to_owned(), loopback)),
                "input {input:?}"
            );
            assert_eq!(
                normalise_base_url(input),
                Ok(want.to_owned()),
                "input {input:?}"
            );
        }
    }

    #[test]
    fn base_urls_are_refused_with_a_reason() {
        let rows = [
            ("", "empty"),
            ("   ", "empty"),
            ("app.infisical.com", "absolute"),
            ("ftp://example.com", "https (got ftp)"),
            ("https://user:pw@example.com", "user name"),
            ("https://example.com/?x=1", "query"),
            ("https://example.com/#f", "query"),
            ("http://192.168.1.10", "plain text to 192.168.1.10"),
            ("http://infisical.lan", "plain text to infisical.lan"),
            ("http://localhost.", "plain text"),
            ("http://0.0.0.0", "plain text"),
        ];
        for (input, reason) in rows {
            match normalise_base_url(input) {
                Err(SecretError::Config(why)) => {
                    assert!(why.contains(reason), "input {input:?}: {why}");
                }
                other => panic!("input {input:?}: expected Config, got {other:?}"),
            }
        }
        let Err(SecretError::Config(why)) = normalise_base_url("https://user:pw@example.com")
        else {
            panic!("userinfo must be refused");
        };
        assert!(!why.contains("pw"), "the refusal echoes the password");
    }

    #[test]
    fn loopback_is_localhost_127_slash_8_and_ipv6_one_only() {
        let host = |s: &str| Url::parse(&format!("http://{s}/")).expect("a test URL");
        for yes in ["localhost", "127.0.0.1", "127.255.0.9", "[::1]"] {
            let url = host(yes);
            assert!(is_loopback(&url.host().expect("a host")), "{yes}");
        }
        for no in [
            "localhost.",
            "0.0.0.0",
            "10.0.0.1",
            "[::2]",
            "example.com",
            "128.0.0.1",
        ] {
            let url = host(no);
            assert!(!is_loopback(&url.host().expect("a host")), "{no}");
        }
    }
}
