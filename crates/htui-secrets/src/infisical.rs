//! The Infisical provider (MOD-10 D5, D6, D8, D9): base URL policy, the HTTP client, the login
//! with its token reuse and single-flight latch, the secrets list, its merge and its validation.
//!
//! **Nothing here prints a secret.** `InfisicalProvider`'s `Debug` is hand-written; the token
//! state and the wire shapes have no `Debug` at all (H-2); every error is built from a status, a
//! known message, a cleaned non-login server message, a scope field, a key name or a cause chain.

use std::collections::BTreeMap;
use std::sync::Once;
use std::time::{Duration, Instant};

use htui_core::secret::{
    MachineIdentity, ProviderHealth, ResolvedSecrets, SecretError, SecretFuture, SecretProvider,
    SecretScope,
};
use tokio::sync::Mutex;
use url::{Host, Position, Url};
use zeroize::Zeroizing;

use crate::wire::{ErrorBody, ListResponse, LoginRequest, LoginResponse};

/// D9: the connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// D9: the whole-request timeout (`install/http.rs`'s `short` client shape).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// `htui/<version>`, so a server log line names the client.
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));
/// Universal Auth login.
const LOGIN_PATH: &str = "/api/v1/auth/universal-auth/login";
/// The secrets list (v0.150+).
const SECRETS_PATH: &str = "/api/v4/secrets";
/// The unauthenticated status endpoint (health only).
const STATUS_PATH: &str = "/api/status";

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
    client: reqwest::Client,
    /// D5: held across the login request (single flight), never across a data request.
    state: Mutex<TokenState>,
}

/// D5 token state. **No `Debug`** (it holds the token, H-2).
enum TokenState {
    /// No token yet, or the last login failed without a refusal.
    Empty,
    /// A token, reused while `Instant::now() < reuse_until`.
    Valid {
        /// The access token, wiped on drop.
        token: Zeroizing<String>,
        /// When to stop reusing it.
        reuse_until: Instant,
    },
    /// A login was refused: every later call is `LoginRefusedEarlier`, with no request.
    Refused,
}

/// Why [`InfisicalProvider::login`] failed: `Refused` latches (D5), `Other` does not.
enum LoginFailure {
    /// `BadCredentials` or `IdentityLocked`.
    Refused(SecretError),
    /// Anything else: unreachable, rate-limited, an unexpected answer.
    Other(SecretError),
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
            state: Mutex::new(TokenState::Empty),
        })
    }

    /// The normalised base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// A token for a data request: the cached one while it is reusable, else a login with the
    /// lock held (single flight). A refused login latches (§B.4.3).
    async fn token(&self) -> Result<Zeroizing<String>, SecretError> {
        let mut state = self.state.lock().await;
        match &*state {
            TokenState::Refused => return Err(SecretError::LoginRefusedEarlier),
            TokenState::Valid { token, reuse_until } if Instant::now() < *reuse_until => {
                return Ok(token.clone());
            }
            TokenState::Valid { .. } | TokenState::Empty => {}
        }
        self.login_into(&mut state).await
    }

    /// After a data 401 / 403 `TokenError` with `used`: one more login, unless another caller
    /// already replaced the token (H-18: not a security comparison, never logged).
    async fn refresh(&self, used: &str) -> Result<Zeroizing<String>, SecretError> {
        let mut state = self.state.lock().await;
        match &*state {
            TokenState::Refused => return Err(SecretError::LoginRefusedEarlier),
            TokenState::Valid { token, reuse_until }
                if token.as_str() != used && Instant::now() < *reuse_until =>
            {
                return Ok(token.clone());
            }
            TokenState::Valid { .. } | TokenState::Empty => {}
        }
        self.login_into(&mut state).await
    }

    /// Health always logs in afresh (D2); the new token replaces any cached one.
    async fn fresh_login(&self) -> Result<Zeroizing<String>, SecretError> {
        let mut state = self.state.lock().await;
        if matches!(*state, TokenState::Refused) {
            return Err(SecretError::LoginRefusedEarlier);
        }
        self.login_into(&mut state).await
    }

    /// Logs in and records the outcome in `state`, whose lock the caller holds.
    async fn login_into(&self, state: &mut TokenState) -> Result<Zeroizing<String>, SecretError> {
        match self.login().await {
            Ok((token, reuse_until)) => {
                *state = TokenState::Valid {
                    token: token.clone(),
                    reuse_until,
                };
                Ok(token)
            }
            Err(LoginFailure::Refused(e)) => {
                *state = TokenState::Refused;
                Err(e)
            }
            Err(LoginFailure::Other(e)) => {
                *state = TokenState::Empty;
                Err(e)
            }
        }
    }

    /// The login POST and its mapping (§B.4.2). The body of a failed login is never quoted.
    async fn login(&self) -> Result<(Zeroizing<String>, Instant), LoginFailure> {
        let url = self.endpoint_url(LOGIN_PATH).map_err(LoginFailure::Other)?;
        let request = LoginRequest {
            client_id: self.identity.client_id(),
            client_secret: self.identity.client_secret(),
        };
        let response = self
            .client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|e| LoginFailure::Other(unreachable(LOGIN_PATH, e)))?;
        let status = response.status();
        let retry_after_secs = retry_after(response.headers());
        let body = response
            .bytes()
            .await
            .map_err(|e| LoginFailure::Other(unreachable(LOGIN_PATH, e)))?;
        if status.is_success() {
            let answer: LoginResponse = decode(LOGIN_PATH, &body).map_err(LoginFailure::Other)?;
            let token = Zeroizing::new(answer.access_token);
            if token.is_empty() {
                return Err(LoginFailure::Other(protocol(
                    LOGIN_PATH,
                    "the login answer carried no access token".to_owned(),
                )));
            }
            return Ok((token, reuse_until(Instant::now(), answer.expires_in)));
        }
        if status.is_redirection() {
            return Err(LoginFailure::Other(redirect(LOGIN_PATH, status.as_u16())));
        }
        let error = ErrorBody::from_bytes(&body);
        Err(match status.as_u16() {
            401 if is_lockout(&error) => LoginFailure::Refused(SecretError::IdentityLocked),
            401 => LoginFailure::Refused(SecretError::BadCredentials),
            404 if is_fastify_not_found(&error) => {
                LoginFailure::Other(SecretError::UnsupportedServer {
                    endpoint: LOGIN_PATH,
                })
            }
            429 => LoginFailure::Other(SecretError::RateLimited { retry_after_secs }),
            code => LoginFailure::Other(protocol(LOGIN_PATH, format!("status {code}"))),
        })
    }

    /// Latch peek, the status GET, then a fresh login (§B.4.3).
    async fn health_inner(&self) -> Result<ProviderHealth, SecretError> {
        if matches!(*self.state.lock().await, TokenState::Refused) {
            return Err(SecretError::LoginRefusedEarlier);
        }
        let url = self.endpoint_url(STATUS_PATH)?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| unreachable(STATUS_PATH, e))?;
        let status = response.status();
        drop(response);
        if status.is_redirection() {
            return Err(redirect(STATUS_PATH, status.as_u16()));
        }
        let server_ok = status.is_success();
        self.fresh_login().await?;
        Ok(ProviderHealth {
            base_url: self.base.clone(),
            server_ok,
        })
    }

    /// Token, list, one refresh on a refused token, then merge and validate (§B.4.3).
    async fn resolve_inner(&self, scope: &SecretScope) -> Result<ResolvedSecrets, SecretError> {
        let token = self.token().await?;
        let list = match self.list(scope, &token).await? {
            Some(list) => list,
            None => {
                let token = self.refresh(&token).await?;
                self.list(scope, &token).await?.ok_or_else(|| {
                    protocol(
                        SECRETS_PATH,
                        "the access token was refused right after a fresh login".to_owned(),
                    )
                })?
            }
        };
        validate(merge(list))
    }

    /// One GET of the list endpoint with `token`; `Ok(None)` means "token refused" (401, or 403
    /// `TokenError`). §B.4.2.
    async fn list(
        &self,
        scope: &SecretScope,
        token: &str,
    ) -> Result<Option<ListResponse>, SecretError> {
        let url = self.list_url(scope)?;
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| unreachable(SECRETS_PATH, e))?;
        let status = response.status();
        let retry_after_secs = retry_after(response.headers());
        let body = response
            .bytes()
            .await
            .map_err(|e| unreachable(SECRETS_PATH, e))?;
        if status.is_success() {
            return decode(SECRETS_PATH, &body).map(Some);
        }
        if status.is_redirection() {
            return Err(redirect(SECRETS_PATH, status.as_u16()));
        }
        let error = ErrorBody::from_bytes(&body);
        let message = clean_message(&error);
        match status.as_u16() {
            401 => Ok(None),
            403 if error.error() == "TokenError" => Ok(None),
            403 => Err(SecretError::PermissionDenied {
                detail: if message.is_empty() {
                    format!("status 403 {}", error.error())
                } else {
                    message
                },
            }),
            404 if is_fastify_not_found(&error) => Err(SecretError::UnsupportedServer {
                endpoint: SECRETS_PATH,
            }),
            404 if error.error() == "NotFound" => Err(SecretError::ProjectNotFound),
            404 if error.error() == "SecretPathNotFound" => Err(SecretError::PathNotFound {
                environment: scope.environment().to_owned(),
                path: scope.path().to_owned(),
            }),
            404 => Err(protocol(SECRETS_PATH, format!("status 404: {message}"))),
            429 => Err(SecretError::RateLimited { retry_after_secs }),
            code => Err(protocol(SECRETS_PATH, status_detail(code, &message))),
        }
    }

    /// `{base}/api/v4/secrets?…` with the D6 flags, in order (§B.4.4).
    fn list_url(&self, scope: &SecretScope) -> Result<Url, SecretError> {
        let mut url = self.endpoint_url(SECRETS_PATH)?;
        url.query_pairs_mut()
            .append_pair("projectId", scope.project_id())
            .append_pair("environment", scope.environment())
            .append_pair("secretPath", scope.path())
            .append_pair("viewSecretValue", "true")
            .append_pair("expandSecretReferences", "true")
            .append_pair("includeImports", "true")
            .append_pair("include_imports", "true")
            .append_pair("recursive", "false")
            .append_pair("includePersonalOverrides", "false");
        Ok(url)
    }

    /// `base + path` as a URL.
    fn endpoint_url(&self, path: &'static str) -> Result<Url, SecretError> {
        Url::parse(&format!("{}{path}", self.base))
            .map_err(|e| SecretError::Config(format!("cannot build the URL of {path}: {e}")))
    }
}

impl SecretProvider for InfisicalProvider {
    fn kind(&self) -> &'static str {
        Self::KIND
    }

    fn health(&self) -> SecretFuture<'_, ProviderHealth> {
        Box::pin(self.health_inner())
    }

    fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>> {
        Box::pin(async move { Ok(self.resolve_inner(scope).await?.keys()) })
    }

    fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets> {
        Box::pin(self.resolve_inner(scope))
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

/// `expires_in - max(60, expires_in / 10)`, saturating at zero (§B.4.3).
fn reuse_window(expires_in_secs: u64) -> Duration {
    Duration::from_secs(expires_in_secs.saturating_sub((expires_in_secs / 10).max(60)))
}

/// The longest a token is reused, whatever `expiresIn` says. `expiresIn` is server input and
/// `Instant + Duration` panics on overflow; ten years is far beyond any Infisical token TTL.
const MAX_REUSE: Duration = Duration::from_secs(10 * 365 * 24 * 60 * 60);

/// `now + reuse_window(expires_in_secs)`, the window capped at [`MAX_REUSE`]. Never panics: should
/// the addition still overflow, the deadline is `now` and the token is simply not reused.
fn reuse_until(now: Instant, expires_in_secs: u64) -> Instant {
    now.checked_add(reuse_window(expires_in_secs).min(MAX_REUSE))
        .unwrap_or(now)
}

/// `Unreachable` for a transport error, its URL stripped.
fn unreachable(endpoint: &'static str, err: reqwest::Error) -> SecretError {
    SecretError::Unreachable {
        endpoint,
        cause: cause_chain(err),
    }
}

/// `Protocol` at `endpoint`.
fn protocol(endpoint: &'static str, detail: String) -> SecretError {
    SecretError::Protocol { endpoint, detail }
}

/// A 3xx: redirects are never followed (D9), so the answer is unexpected.
fn redirect(endpoint: &'static str, code: u16) -> SecretError {
    protocol(
        endpoint,
        format!("the server answered a redirect ({code}); redirects are not followed"),
    )
}

/// `"status {code}: {message}"`, or `"status {code}"` without a message.
fn status_detail(code: u16, message: &str) -> String {
    if message.is_empty() {
        format!("status {code}")
    } else {
        format!("status {code}: {message}")
    }
}

/// The body as `T`; on failure `Protocol` with the category and position only, never serde's
/// message, which quotes mistyped values (A-5, H-4).
fn decode<T: serde::de::DeserializeOwned>(
    endpoint: &'static str,
    body: &[u8],
) -> Result<T, SecretError> {
    serde_json::from_slice(body).map_err(|e| {
        protocol(
            endpoint,
            format!(
                "the body is not the expected JSON ({:?} error at line {}, column {})",
                e.classify(),
                e.line(),
                e.column()
            ),
        )
    })
}

/// The server `message`: non-string → `""`; control characters removed; the first 200 `char`s
/// (H-17: never a byte slice).
fn clean_message(body: &ErrorBody) -> String {
    body.message()
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}

/// Fastify's default 404 for an unknown route: `Route GET:/… not found`. Infisical's own
/// `NotFound` has another message, so this check runs first.
fn is_fastify_not_found(body: &ErrorBody) -> bool {
    let message = body.message();
    message.starts_with("Route ") && message.ends_with(" not found")
}

/// `Retry-After` in whole seconds; an HTTP-date or anything else is `None`.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// A login 401 whose message says the identity is temporarily locked.
fn is_lockout(body: &ErrorBody) -> bool {
    body.message().to_lowercase().contains("temporarily locked")
}

/// `^[A-Za-z_][A-Za-z0-9_]*$`.
fn is_env_name(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// One merged entry (D6). **No `Debug`** (it holds a value, H-2).
pub(crate) struct Merged {
    /// The value, wiped on drop.
    pub(crate) value: Zeroizing<String>,
    /// `secretValueHidden`.
    pub(crate) hidden: bool,
}

/// D6 merge (§B.4.5): the folder first, then the imports last to first; a key already present is
/// kept; `personal` entries are skipped. Values are moved into `Zeroizing` as they are read, so
/// shadowed entries drop wiped.
pub(crate) fn merge(list: ListResponse) -> BTreeMap<String, Merged> {
    let mut out = BTreeMap::new();
    let imports = list.imports.unwrap_or_default();
    let entries = list
        .secrets
        .into_iter()
        .chain(imports.into_iter().rev().flat_map(|import| import.secrets));
    for entry in entries {
        if entry.kind.as_deref().is_some_and(|kind| kind != "shared") {
            continue;
        }
        let merged = Merged {
            value: Zeroizing::new(entry.secret_value),
            hidden: entry.secret_value_hidden,
        };
        out.entry(entry.secret_key).or_insert(merged);
    }
    out
}

/// D6 validation (§B.4.6): hidden, then names, then NUL, each the first in key order; then the
/// values are moved out with `std::mem::take`, byte for byte.
pub(crate) fn validate(merged: BTreeMap<String, Merged>) -> Result<ResolvedSecrets, SecretError> {
    if let Some(key) = merged.iter().find(|(_, m)| m.hidden).map(|(k, _)| k) {
        return Err(SecretError::PermissionDenied {
            detail: format!("the value of {key:?} is hidden from this identity"),
        });
    }
    if let Some(key) = merged.keys().find(|k| !is_env_name(k)) {
        return Err(SecretError::InvalidKey { key: key.clone() });
    }
    if let Some(key) = merged
        .iter()
        .find(|(_, m)| m.value.contains('\0'))
        .map(|(k, _)| k)
    {
        return Err(SecretError::InvalidValue { key: key.clone() });
    }
    Ok(ResolvedSecrets::new(
        merged
            .into_iter()
            .map(|(key, mut m)| (key, std::mem::take(&mut *m.value)))
            .collect(),
    ))
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
    fn the_reuse_deadline_never_overflows() {
        let now = Instant::now();
        assert_eq!(
            reuse_until(now, 600),
            now + Duration::from_secs(540),
            "an ordinary expiresIn"
        );
        for expires_in in [u64::MAX, u64::MAX / 2, 1 << 63] {
            assert_eq!(
                reuse_until(now, expires_in),
                now + MAX_REUSE,
                "expiresIn {expires_in}"
            );
        }
    }

    #[test]
    fn the_reuse_margin_is_the_larger_of_a_minute_and_a_tenth() {
        let rows = [
            (2_592_000, 2_332_800),
            (600, 540),
            (300, 240),
            (60, 0),
            (30, 0),
        ];
        for (expires_in, window) in rows {
            assert_eq!(
                reuse_window(expires_in),
                Duration::from_secs(window),
                "expiresIn {expires_in}"
            );
        }
    }

    #[test]
    fn env_names_match_the_posix_shape() {
        for yes in ["A", "_A1", "a_b"] {
            assert!(is_env_name(yes), "{yes:?}");
        }
        for no in ["", "1A", "A-B", "A.B", "É", "A B", "A\n"] {
            assert!(!is_env_name(no), "{no:?}");
        }
    }

    #[test]
    fn server_messages_lose_control_characters_and_stop_at_200_characters() {
        let message = format!("\r\n\x1b{}", "é".repeat(300));
        let body = ErrorBody::from_bytes(
            serde_json::to_vec(&serde_json::json!({ "message": message }))
                .expect("serialises")
                .as_slice(),
        );
        let cleaned = clean_message(&body);
        assert_eq!(cleaned.chars().count(), 200);
        assert!(!cleaned.chars().any(char::is_control));
        assert_eq!(clean_message(&ErrorBody::default()), "");
    }

    #[test]
    fn the_fastify_not_found_body_is_recognised() {
        let fastify = ErrorBody::from_bytes(
            br#"{"message":"Route GET:/api/v4/secrets?projectId=p not found","error":"Not Found","statusCode":404}"#,
        );
        assert!(is_fastify_not_found(&fastify));
        let infisical = ErrorBody::from_bytes(
            br#"{"message":"Project with ID p not found","error":"NotFound","statusCode":404}"#,
        );
        assert!(!is_fastify_not_found(&infisical));
        assert!(!is_fastify_not_found(&ErrorBody::default()));
    }

    #[test]
    fn retry_after_reads_whole_seconds_only() {
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
        let with = |v: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_static(v));
            headers
        };
        assert_eq!(retry_after(&with("30")), Some(30));
        assert_eq!(retry_after(&with(" 7 ")), Some(7));
        assert_eq!(retry_after(&with("Wed, 21 Oct 2026 07:28:00 GMT")), None);
        assert_eq!(retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn the_lockout_message_is_recognised_case_insensitively() {
        let body = |m: &str| ErrorBody::from_bytes(format!(r#"{{"message":"{m}"}}"#).as_bytes());
        assert!(is_lockout(&body("Identity is temporarily locked")));
        assert!(is_lockout(&body("IDENTITY TEMPORARILY LOCKED, try later")));
        assert!(!is_lockout(&body("Invalid credentials")));
        assert!(!is_lockout(&ErrorBody::default()));
    }

    fn merged(rows: &[(&str, &str, bool)]) -> BTreeMap<String, Merged> {
        rows.iter()
            .map(|(k, v, hidden)| {
                (
                    (*k).to_owned(),
                    Merged {
                        value: Zeroizing::new((*v).to_owned()),
                        hidden: *hidden,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn validation_reports_hidden_then_name_then_nul() {
        let all = merged(&[("1A", "x", false), ("B", "", true), ("C", "a\0b", false)]);
        assert!(matches!(
            validate(all),
            Err(SecretError::PermissionDenied { detail }) if detail.contains("\"B\"")
        ));
        let name_and_nul = merged(&[("A", "a\0b", false), ("Z-Z", "x", false)]);
        assert!(matches!(
            validate(name_and_nul),
            Err(SecretError::InvalidKey { key }) if key == "Z-Z"
        ));
        let nul = merged(&[("A", "ok", false), ("B", "a\0b", false)]);
        assert!(matches!(
            validate(nul),
            Err(SecretError::InvalidValue { key }) if key == "B"
        ));
        let good = validate(merged(&[("B", " b\n", false), ("A", "a", false)])).expect("valid");
        assert_eq!(good.keys(), ["A", "B"]);
        assert_eq!(good.as_map()["B"], " b\n");
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
