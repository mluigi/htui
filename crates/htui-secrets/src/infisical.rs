//! The Infisical provider (MOD-10 D5, D6, D8, D9): base URL policy, the HTTP client, the login
//! with its token reuse and single-flight latch, the secrets list, its merge and its validation.
//!
//! **Nothing here prints a secret.** `InfisicalProvider`'s `Debug` is hand-written; the token
//! state and the wire shapes have no `Debug` at all (H-2); every error is built from a status, a
//! known message, a cleaned non-login server message, a scope field, a key name or a cause chain.

use std::collections::BTreeMap;
use std::sync::{Arc, Once};
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
/// D5: after a login that was sent but got no answer, how long no new login is tried. Infisical's
/// default lockout counter-reset window, so the unknown attempt has expired from the count.
pub const DEFAULT_LOGIN_COOL_DOWN: Duration = Duration::from_secs(30);

/// `htui/<version>`, so a server log line names the client.
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));
/// Universal Auth login.
const LOGIN_PATH: &str = "/api/v1/auth/universal-auth/login";
/// The secrets list (v0.150+).
const SECRETS_PATH: &str = "/api/v4/secrets";
/// The unauthenticated status endpoint (health only).
const STATUS_PATH: &str = "/api/status";

/// A cap on a response body, and how a sentence names it.
#[derive(Clone, Copy)]
struct BodyCap {
    bytes: usize,
    name: &'static str,
}

/// The login answer and every error body: a few hundred bytes in practice.
const SMALL_BODY: BodyCap = BodyCap {
    bytes: 64 * 1024,
    name: "64 KiB",
};
/// A list answer: every value of a folder and its imports.
const LIST_BODY: BodyCap = BodyCap {
    bytes: 8 * 1024 * 1024,
    name: "8 MiB",
};

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
    /// D5: no login is tried for this long after a login that got no answer
    /// ([`SecretError::LoginCoolingDown`]).
    pub login_cool_down: Duration,
}

impl InfisicalConfig {
    /// `base_url` with the D9 timeouts and the D5 cool-down.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            timeout: DEFAULT_TIMEOUT,
            login_cool_down: DEFAULT_LOGIN_COOL_DOWN,
        }
    }
}

/// The Infisical client (D5, D6, D9). `Debug` is hand-written: base URL and client ID only.
pub struct InfisicalProvider {
    /// What the login task needs; shared with it so a dropped caller cannot cancel a login.
    inner: Arc<Inner>,
    /// D5: held across the login (single flight) by the login task itself, so it stays held
    /// until the outcome is recorded even when every caller gave up. Never held across a data
    /// request.
    state: Arc<Mutex<TokenState>>,
}

/// The provider's fixed parts. **No `Debug`** (it holds the identity, H-2).
struct Inner {
    /// Normalised: origin plus optional prefix, no trailing `/`.
    base: String,
    identity: MachineIdentity,
    client: reqwest::Client,
    /// D5: how long no login is tried after one that got no answer.
    login_cool_down: Duration,
}

/// D5 token state. **No `Debug`** (it holds the token, H-2).
enum TokenState {
    /// No token yet, or the last login failed in a way Infisical did not count against the
    /// identity (nothing sent, or an answer that is not a refusal).
    Empty,
    /// A token, reused while `Instant::now() < reuse_until`.
    Valid {
        /// The access token, wiped on drop.
        token: Zeroizing<String>,
        /// When to stop reusing it.
        reuse_until: Instant,
    },
    /// A login was sent and got no answer, so it may have counted as a failed attempt: until
    /// `until` every call is `LoginCoolingDown`, with no request; then one new login.
    CoolingDown {
        /// When the next login may be tried.
        until: Instant,
    },
    /// A login was refused: every later call is `LoginRefusedEarlier`, with no request.
    Refused,
}

/// Why [`Inner::login`] failed, which decides the next [`TokenState`].
enum LoginFailure {
    /// `BadCredentials` or `IdentityLocked`: latches (`Refused`).
    Refused(SecretError),
    /// Not counted against the identity: nothing was sent (connect error), or the server answered
    /// with something other than a refusal. Back to `Empty`; the next call logs in again.
    Other(SecretError),
    /// Sent but unanswered (a timeout or a reset after the send): the server may have counted a
    /// failed attempt. `CoolingDown`.
    Unanswered(SecretError),
}

/// What a caller of [`InfisicalProvider::token`] needs.
#[derive(Clone, Copy)]
enum Need<'a> {
    /// Any reusable token (a data request).
    Cached,
    /// A token other than this refused one (H-18: not a security comparison, never logged).
    Replacing(&'a str),
    /// A fresh login whatever is cached (health, D2).
    Fresh,
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
            inner: Arc::new(Inner {
                base,
                identity,
                client,
                login_cool_down: config.login_cool_down,
            }),
            state: Arc::new(Mutex::new(TokenState::Empty)),
        })
    }

    /// The normalised base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.inner.base
    }

    /// A token as `need` asks (§B.4.3): the cached one when it serves, else one login, single
    /// flight. The login runs on its own task, which owns the lock guard and records the outcome
    /// before releasing it: a caller dropped mid-login (a timeout, a `select!`) loses nothing, and
    /// the callers queued behind it see the outcome instead of logging in again (D5).
    async fn token(&self, need: Need<'_>) -> Result<Zeroizing<String>, SecretError> {
        let state = Arc::clone(&self.state).lock_owned().await;
        let now = Instant::now();
        if let Some(e) = no_login(&state, now) {
            return Err(e);
        }
        if let TokenState::Valid { token, reuse_until } = &*state
            && now < *reuse_until
        {
            match need {
                Need::Cached => return Ok(token.clone()),
                Need::Replacing(used) if token.as_str() != used => return Ok(token.clone()),
                Need::Replacing(_) | Need::Fresh => {}
            }
        }
        let inner = Arc::clone(&self.inner);
        let login = tokio::spawn(async move {
            let mut state = state;
            let outcome = inner.login().await;
            record(&mut state, outcome, inner.login_cool_down)
        });
        match login.await {
            Ok(result) => result,
            Err(e) => match e.try_into_panic() {
                Ok(panic) => std::panic::resume_unwind(panic),
                // Only a runtime shutting down cancels the task; its outcome is unknown.
                Err(_) => Err(protocol(
                    LOGIN_PATH,
                    "the login was cancelled before it answered".to_owned(),
                )),
            },
        }
    }

    /// Latch and cool-down peek, the status GET, then a fresh login (§B.4.3).
    async fn health_inner(&self) -> Result<ProviderHealth, SecretError> {
        if let Some(e) = no_login(&*self.state.lock().await, Instant::now()) {
            return Err(e);
        }
        let url = self.inner.endpoint_url(STATUS_PATH)?;
        let response = self
            .inner
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
        self.token(Need::Fresh).await?;
        Ok(ProviderHealth {
            base_url: self.inner.base.clone(),
            server_ok,
        })
    }

    /// Token, list, one refresh on a refused token, then merge and validate (§B.4.3).
    async fn resolve_inner(&self, scope: &SecretScope) -> Result<ResolvedSecrets, SecretError> {
        let token = self.token(Need::Cached).await?;
        let list = match self.list(scope, &token).await? {
            Some(list) => list,
            None => {
                let token = self.token(Need::Replacing(&token)).await?;
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
            .inner
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| unreachable(SECRETS_PATH, e))?;
        let status = response.status();
        // A 401 refuses the token whatever its body: decided before the read, so a body cut
        // short still earns the one re-login instead of an `Unreachable` that keeps the refused
        // token cached. A 403 needs its `error` field, so it still waits for the body.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }
        let retry_after_secs = retry_after(response.headers());
        let cap = if status.is_success() {
            LIST_BODY
        } else {
            SMALL_BODY
        };
        let body = read_body(SECRETS_PATH, response, cap).await?;
        if status.is_success() {
            return decode(SECRETS_PATH, &body).map(Some);
        }
        map_list_status(status, &body, scope, retry_after_secs).map_or(Ok(None), Err)
    }

    /// `{base}/api/v4/secrets?…` with the D6 flags, in order (§B.4.4).
    fn list_url(&self, scope: &SecretScope) -> Result<Url, SecretError> {
        let mut url = self.inner.endpoint_url(SECRETS_PATH)?;
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
}

impl Inner {
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
            .map_err(|e| {
                // D5: a connect error (DNS, refused, TLS, connect timeout) or a request that
                // never built sent no credentials. Anything else failed after the send: the
                // server may have counted a failed login.
                let nothing_sent = e.is_connect() || e.is_builder();
                let err = unreachable(LOGIN_PATH, e);
                if nothing_sent {
                    LoginFailure::Other(err)
                } else {
                    LoginFailure::Unanswered(err)
                }
            })?;
        let status = response.status();
        let retry_after_secs = retry_after(response.headers());
        let body = read_body(LOGIN_PATH, response, SMALL_BODY).await;
        // D5: a 401 refuses the identity whatever happens to its body, so a body cut short or
        // over the cap can never turn a rejected login into a retried one.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(LoginFailure::Refused(login_refusal(body.as_deref().ok())));
        }
        let body = body.map_err(LoginFailure::Other)?;
        if status.is_success() {
            return accept_login(&body, Instant::now()).map_err(LoginFailure::Other);
        }
        Err(LoginFailure::Other(map_login_status(
            status,
            &body,
            retry_after_secs,
        )))
    }

    /// `base + path` as a URL.
    fn endpoint_url(&self, path: &'static str) -> Result<Url, SecretError> {
        Url::parse(&format!("{}{path}", self.base))
            .map_err(|e| SecretError::Config(format!("cannot build the URL of {path}: {e}")))
    }
}

/// The error a call gets without a login (and so without any request), if the state forbids one:
/// a latched refusal, or a cool-down still running at `now`.
fn no_login(state: &TokenState, now: Instant) -> Option<SecretError> {
    match state {
        TokenState::Refused => Some(SecretError::LoginRefusedEarlier),
        TokenState::CoolingDown { until } if now < *until => {
            let left = *until - now;
            Some(SecretError::LoginCoolingDown {
                retry_after_secs: left.as_secs() + u64::from(left.subsec_nanos() > 0),
            })
        }
        TokenState::Empty | TokenState::Valid { .. } | TokenState::CoolingDown { .. } => None,
    }
}

/// Records a login's outcome in `state` (§B.4.3) and returns it to the caller.
fn record(
    state: &mut TokenState,
    outcome: Result<(Zeroizing<String>, Instant), LoginFailure>,
    cool_down: Duration,
) -> Result<Zeroizing<String>, SecretError> {
    let (next, result) = match outcome {
        Ok((token, reuse_until)) => (
            TokenState::Valid {
                token: token.clone(),
                reuse_until,
            },
            Ok(token),
        ),
        Err(LoginFailure::Refused(e)) => (TokenState::Refused, Err(e)),
        Err(LoginFailure::Other(e)) => (TokenState::Empty, Err(e)),
        Err(LoginFailure::Unanswered(e)) => {
            let now = Instant::now();
            // Never panics: a cool-down past `Instant`'s range is capped like a reuse window.
            let until = now.checked_add(cool_down.min(MAX_REUSE)).unwrap_or(now);
            (TokenState::CoolingDown { until }, Err(e))
        }
    };
    *state = next;
    result
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
            .field("base_url", &self.inner.base)
            .field("client_id", &self.inner.identity.client_id())
            .finish_non_exhaustive()
    }
}

/// A login 200's body as the token and its reuse deadline (§B.4.2). Never quotes the body.
fn accept_login(body: &[u8], now: Instant) -> Result<(Zeroizing<String>, Instant), SecretError> {
    let answer: LoginResponse = decode(LOGIN_PATH, body)?;
    let token = Zeroizing::new(answer.access_token);
    if token.is_empty() {
        return Err(protocol(
            LOGIN_PATH,
            "the login answer carried no access token".to_owned(),
        ));
    }
    // A token that cannot ride an `Authorization` header would fail every data call before it is
    // sent, as an `Unreachable` that never triggers a re-login.
    if !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(protocol(
            LOGIN_PATH,
            "the login answer carried an access token that is not a valid header value".to_owned(),
        ));
    }
    Ok((token, reuse_until(now, answer.expires_in)))
}

/// A login 401 (D5): `IdentityLocked` when a body read in full (`Some`) carries the lockout
/// text, else `BadCredentials`. A body that was cut short or not read is `None`.
fn login_refusal(body: Option<&[u8]>) -> SecretError {
    if body.is_some_and(|body| is_lockout(&ErrorBody::from_bytes(body))) {
        SecretError::IdentityLocked
    } else {
        SecretError::BadCredentials
    }
}

/// A login answer that is neither 2xx nor 401, as its error (§B.4.2). The body only decides
/// between variants; it is never quoted.
fn map_login_status(
    status: reqwest::StatusCode,
    body: &[u8],
    retry_after_secs: Option<u64>,
) -> SecretError {
    if status.is_redirection() {
        return redirect(LOGIN_PATH, status.as_u16());
    }
    match status.as_u16() {
        404 if is_fastify_not_found(&ErrorBody::from_bytes(body)) => {
            SecretError::UnsupportedServer {
                endpoint: LOGIN_PATH,
            }
        }
        429 => SecretError::RateLimited { retry_after_secs },
        code => protocol(LOGIN_PATH, format!("status {code}")),
    }
}

/// A list answer that is not 2xx, as its error (§B.4.2); `None` means the token was refused
/// (401, or 403 `TokenError`), which earns one re-login.
fn map_list_status(
    status: reqwest::StatusCode,
    body: &[u8],
    scope: &SecretScope,
    retry_after_secs: Option<u64>,
) -> Option<SecretError> {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return None;
    }
    if status.is_redirection() {
        return Some(redirect(SECRETS_PATH, status.as_u16()));
    }
    let error = ErrorBody::from_bytes(body);
    let message = clean_message(&error);
    Some(match status.as_u16() {
        403 if error.error() == "TokenError" => return None,
        403 => SecretError::PermissionDenied {
            detail: if message.is_empty() {
                format!("status 403 {}", clean(error.error()))
            } else {
                message
            },
        },
        404 if is_fastify_not_found(&error) => SecretError::UnsupportedServer {
            endpoint: SECRETS_PATH,
        },
        404 if error.error() == "NotFound" => SecretError::ProjectNotFound,
        404 if error.error() == "SecretPathNotFound" => SecretError::PathNotFound {
            environment: scope.environment().to_owned(),
            path: scope.path().to_owned(),
        },
        404 => protocol(SECRETS_PATH, format!("status 404: {message}")),
        429 => SecretError::RateLimited { retry_after_secs },
        code => protocol(SECRETS_PATH, status_detail(code, &message)),
    })
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

/// The body, read chunk by chunk up to `cap`: a declared length over it is refused before the
/// read, an undeclared one as soon as the bytes read pass it. Over the cap is `Protocol` naming
/// the status and the cap, never the body; a transport error is `Unreachable`.
async fn read_body(
    endpoint: &'static str,
    mut response: reqwest::Response,
    cap: BodyCap,
) -> Result<Vec<u8>, SecretError> {
    let too_large = |status: reqwest::StatusCode| {
        protocol(
            endpoint,
            format!(
                "status {} with a body larger than the {} limit",
                status.as_u16(),
                cap.name
            ),
        )
    };
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|declared| declared > cap.bytes as u64)
    {
        return Err(too_large(status));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| unreachable(endpoint, e))?
    {
        if chunk.len() > cap.bytes - body.len() {
            return Err(too_large(status));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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

/// The server `message`, cleaned by [`clean`]; non-string → `""`.
fn clean_message(body: &ErrorBody) -> String {
    clean(body.message())
}

/// Server text fit for a terminal (A-4): control characters removed, the first 200 `char`s
/// (H-17: never a byte slice).
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
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
/// kept; `personal` entries are skipped. Values arrive already in `Zeroizing` (`wire.rs`), so
/// skipped and shadowed entries drop wiped.
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
            value: entry.secret_value,
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
                    assert!(why.contains(reason), "input {input:?}: another reason");
                }
                _ => panic!("input {input:?}: expected Config"),
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
    fn a_cool_down_forbids_a_login_until_it_ends_rounding_seconds_up() {
        let now = Instant::now();
        let cooling = |left: Duration| TokenState::CoolingDown { until: now + left };
        let rows = [
            (Duration::from_secs(30), Some(30)),
            (Duration::from_millis(29_100), Some(30)),
            (Duration::from_millis(1), Some(1)),
            (Duration::ZERO, None),
        ];
        for (left, want) in rows {
            let got = no_login(&cooling(left), now);
            assert_eq!(
                got,
                want.map(|retry_after_secs| SecretError::LoginCoolingDown { retry_after_secs }),
                "{left:?} left"
            );
        }
        assert_eq!(
            no_login(&TokenState::Refused, now),
            Some(SecretError::LoginRefusedEarlier)
        );
        assert_eq!(no_login(&TokenState::Empty, now), None);
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

    fn status(code: u16) -> reqwest::StatusCode {
        reqwest::StatusCode::from_u16(code).expect("a valid status")
    }

    fn error_body(error: &str, message: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"error": error, "message": message}))
            .expect("serialises")
    }

    fn fastify_body(path: &str) -> Vec<u8> {
        error_body("Not Found", &format!("Route GET:{path} not found"))
    }

    #[test]
    fn login_statuses_map_to_their_errors() {
        let rows = [
            (307, Vec::new(), None, redirect(LOGIN_PATH, 307)),
            (
                404,
                fastify_body(LOGIN_PATH),
                None,
                SecretError::UnsupportedServer {
                    endpoint: LOGIN_PATH,
                },
            ),
            (
                404,
                error_body("NotFound", "no such route here"),
                None,
                protocol(LOGIN_PATH, "status 404".to_owned()),
            ),
            (
                429,
                error_body("RateLimitExceeded", "slow down"),
                Some(12),
                SecretError::RateLimited {
                    retry_after_secs: Some(12),
                },
            ),
            (
                403,
                error_body("Forbidden", "quoted nowhere"),
                None,
                protocol(LOGIN_PATH, "status 403".to_owned()),
            ),
            (
                500,
                error_body("InternalServerError", "quoted nowhere"),
                None,
                protocol(LOGIN_PATH, "status 500".to_owned()),
            ),
        ];
        for (code, body, retry, want) in rows {
            assert_eq!(
                map_login_status(status(code), &body, retry),
                want,
                "status {code}"
            );
        }
    }

    #[test]
    fn a_login_401_is_locked_only_when_a_whole_body_says_so() {
        let locked = error_body("UnauthorizedError", "Identity is temporarily locked");
        assert_eq!(login_refusal(Some(&locked)), SecretError::IdentityLocked);
        let wrong = error_body("UnauthorizedError", "Invalid credentials");
        assert_eq!(login_refusal(Some(&wrong)), SecretError::BadCredentials);
        assert_eq!(login_refusal(Some(b"<html>")), SecretError::BadCredentials);
        assert_eq!(login_refusal(None), SecretError::BadCredentials);
    }

    #[test]
    fn list_statuses_map_to_their_errors() {
        let scope = SecretScope::new("p1", "dev", "/app").expect("a valid scope");
        let denied = |detail: &str| SecretError::PermissionDenied {
            detail: detail.to_owned(),
        };
        let rows = [
            (
                401,
                error_body("UnauthorizedError", "Invalid token"),
                None,
                None,
            ),
            (403, error_body("TokenError", "Token expired"), None, None),
            (
                403,
                error_body("PermissionDenied", "no read\r\non dev"),
                None,
                Some(denied("no readon dev")),
            ),
            (
                403,
                error_body("Forbidden", ""),
                None,
                Some(denied("status 403 Forbidden")),
            ),
            (
                404,
                fastify_body(SECRETS_PATH),
                None,
                Some(SecretError::UnsupportedServer {
                    endpoint: SECRETS_PATH,
                }),
            ),
            (
                404,
                error_body("NotFound", "Project not found"),
                None,
                Some(SecretError::ProjectNotFound),
            ),
            (
                404,
                error_body("SecretPathNotFound", "Folder not found"),
                None,
                Some(SecretError::PathNotFound {
                    environment: "dev".to_owned(),
                    path: "/app".to_owned(),
                }),
            ),
            (
                404,
                error_body("Other", "gone"),
                None,
                Some(protocol(SECRETS_PATH, "status 404: gone".to_owned())),
            ),
            (
                429,
                Vec::new(),
                Some(3),
                Some(SecretError::RateLimited {
                    retry_after_secs: Some(3),
                }),
            ),
            (302, Vec::new(), None, Some(redirect(SECRETS_PATH, 302))),
            (
                500,
                error_body("InternalServerError", "boom"),
                None,
                Some(protocol(SECRETS_PATH, "status 500: boom".to_owned())),
            ),
            (
                502,
                b"<html>".to_vec(),
                None,
                Some(protocol(SECRETS_PATH, "status 502".to_owned())),
            ),
        ];
        for (code, body, retry, want) in rows {
            assert_eq!(
                map_list_status(status(code), &body, &scope, retry),
                want,
                "status {code}"
            );
        }
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
