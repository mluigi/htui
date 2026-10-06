//! The secret-provider seam (MOD-10 D1–D4): the trait a provider implements, the scope it reads,
//! the map it returns and the one error it fails with.
//!
//! Lives in `htui-core` so `htui-worker`, `htui-orch` and `htui` can hold an
//! `Arc<dyn SecretProvider>` without depending on HTTP. The Infisical implementation is the
//! `htui-secrets` crate. The machine identity's keyring entries are `htui-store::secret`'s.
//!
//! **No type here prints a secret.** `ResolvedSecrets` and `MachineIdentity` have hand-written
//! `Debug`s, and no `SecretError` variant carries a value, a client secret or a token.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::model::Project;

/// The boxed future every [`SecretProvider`] method returns: the shape of `IsolatorFuture`
/// (`htui-orch`) and `DriverFuture` (`htui-agent`), for the same reason. A provider is held as
/// `Arc<dyn SecretProvider>`, and a plain `async fn` in a trait is not dyn-compatible.
pub type SecretFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SecretError>> + Send + 'a>>;

/// A secret provider (MOD-10 D2). One instance per process and identity (D5: the login latch is
/// per instance, so M3 must share it).
pub trait SecretProvider: Send + Sync + core::fmt::Debug {
    /// Stable provider name, the value `project.secret_provider` holds (`"infisical"`).
    fn kind(&self) -> &'static str;
    /// Reachability plus a fresh login with the configured identity.
    fn health(&self) -> SecretFuture<'_, ProviderHealth>;
    /// Key names visible in `scope`, sorted; never values.
    fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>>;
    /// Every key → value in `scope`, imports merged, validated for an environment block.
    fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets>;
}

/// Which secrets a project reads: an Infisical project, environment slug and folder path (D3).
/// Stored in `project.secret_scope` as compact JSON (`to_column`); M4 writes it, M3 reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretScope {
    project_id: String,
    environment: String,
    path: String,
}

/// The column's JSON, field for field. Private: only `parse`/`to_column` touch it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeColumn {
    project_id: String,
    environment: String,
    #[serde(default = "root_path")]
    path: String,
}

fn root_path() -> String {
    "/".to_owned()
}

impl SecretScope {
    /// A validated scope.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`] when `project_id` or `environment` is empty or whitespace, `path`
    /// does not start with `/`, or any of the three holds a control character (`char::is_control`).
    pub fn new(
        project_id: impl Into<String>,
        environment: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, SecretError> {
        let (project_id, environment, path) = (project_id.into(), environment.into(), path.into());
        if project_id.trim().is_empty() {
            return Err(SecretError::Config(
                "the secret scope has an empty project_id".to_owned(),
            ));
        }
        if environment.trim().is_empty() {
            return Err(SecretError::Config(
                "the secret scope has an empty environment".to_owned(),
            ));
        }
        if !path.starts_with('/') {
            return Err(SecretError::Config(
                "the secret scope path must start with `/`".to_owned(),
            ));
        }
        // `PathNotFound`'s `Display` prints the environment and path raw: a control character
        // would reach the terminal. The refusal names the field, never echoes the value.
        for (field, value) in [
            ("project_id", &project_id),
            ("environment", &environment),
            ("path", &path),
        ] {
            if value.chars().any(char::is_control) {
                return Err(SecretError::Config(format!(
                    "the secret scope's {field} holds a control character"
                )));
            }
        }
        Ok(Self {
            project_id,
            environment,
            path,
        })
    }

    /// Parses `project.secret_scope`. Strict: unknown fields refused; `path` defaults to `/`.
    ///
    /// # Errors
    ///
    /// [`SecretError::Config`]: not JSON of this shape, or a [`Self::new`] refusal.
    pub fn parse(column: &str) -> Result<Self, SecretError> {
        // The column holds no secret, so serde's message may be quoted here. serde prints an
        // unknown key with `Display`, so control characters are escaped before the sentence
        // can reach a terminal.
        let parsed: ScopeColumn = serde_json::from_str(column).map_err(|err| {
            let detail: String = err
                .to_string()
                .chars()
                .map(|c| {
                    if c.is_control() {
                        c.escape_debug().to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            SecretError::Config(format!("the secret scope is not valid: {detail}"))
        })?;
        Self::new(parsed.project_id, parsed.environment, parsed.path)
    }

    /// The column text: `{"project_id":"…","environment":"…","path":"…"}`, compact, in that key
    /// order, `path` always written.
    #[must_use]
    pub fn to_column(&self) -> String {
        // A derived struct keeps its field order whatever `serde_json/preserve_order`
        // unification does; a `json!` map would not.
        serde_json::to_string(&ScopeColumn {
            project_id: self.project_id.clone(),
            environment: self.environment.clone(),
            path: self.path.clone(),
        })
        .expect("a struct of three strings always serialises")
    }

    /// The Infisical project ID.
    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// The environment slug, e.g. `dev`.
    #[must_use]
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// The folder path, starting with `/`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A resolved `key → value` map (D3). Lends `&BTreeMap<String, String>` to
/// `MinimalScrubber::from_resolved` and `SessionSpec.env` (M3). Its `Debug` prints key names only,
/// and dropping it zeroizes every value. Not `Clone`, `Serialize` or `Display`.
///
/// M3 note: `from_resolved` and `SessionSpec.env` take unzeroized copies; the zeroize covers this
/// wrapper only.
pub struct ResolvedSecrets {
    map: BTreeMap<String, String>,
}

impl ResolvedSecrets {
    /// Wraps an already validated map (providers call this after their own validation).
    #[must_use]
    pub fn new(map: BTreeMap<String, String>) -> Self {
        Self { map }
    }

    /// The map, for `from_resolved` and `SessionSpec.env`.
    #[must_use]
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.map
    }

    /// The key names, sorted (map order).
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    /// How many keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl core::fmt::Debug for ResolvedSecrets {
    /// `ResolvedSecrets { keys: ["A", "B"] }`: names only.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResolvedSecrets")
            .field("keys", &self.map.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Drop for ResolvedSecrets {
    fn drop(&mut self) {
        for value in self.map.values_mut() {
            value.zeroize();
        }
    }
}

/// An Infisical Universal Auth machine identity (D3). Both halves are held wiped-on-drop.
/// `Debug` is hand-written and never prints the secret; there is no `Display`, `Serialize`,
/// `Clone` or `PartialEq`.
pub struct MachineIdentity {
    client_id: Zeroizing<String>,
    client_secret: Zeroizing<String>,
}

impl MachineIdentity {
    /// Wraps both halves at once. No validation here: blank halves are refused by the provider's
    /// constructor (`Config`) and read as absent by the keyring (`htui-store`'s `Slot::get`).
    #[must_use]
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: Zeroizing::new(client_id.into()),
            client_secret: Zeroizing::new(client_secret.into()),
        }
    }

    /// The client ID. Not secret (it is shown in `Debug`).
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The client secret. **Secret**: for the login body and the keyring write only; never log,
    /// format or compare it in an assert message.
    #[must_use]
    pub fn client_secret(&self) -> &str {
        &self.client_secret
    }
}

impl core::fmt::Debug for MachineIdentity {
    /// `MachineIdentity { client_id: "…", client_secret: "<redacted>" }`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MachineIdentity")
            .field("client_id", &self.client_id.as_str())
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

/// What [`SecretProvider::health`] reports. No token and no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealth {
    /// The normalised base URL the provider talks to.
    pub base_url: String,
    /// Whether the server's unauthenticated status endpoint answered 2xx.
    pub server_ok: bool,
}

/// Why a secret-provider call failed (D4). **No variant ever carries a secret value, the client
/// secret or the access token.** `endpoint` is a path (`/api/v4/secrets`), never a URL with query
/// values. Login failures never quote the server's body; other endpoints may quote the server's
/// `message`, cleaned and cut to 200 characters. Key names are not secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The keyring holds no machine identity. Built by the caller that reads the keyring (M3).
    #[error("no Infisical machine identity is stored in the OS keyring")]
    NoIdentity,
    /// A bad base URL, a blank identity half, a client that would not build, or a scope that
    /// does not parse.
    #[error("secret provider configuration: {0}")]
    Config(String),
    /// The request did not complete: DNS, connect, TLS, timeout, or a body cut short. `cause` is
    /// the `source()` chain of `reqwest::Error::without_url()`, joined with `": "`.
    #[error("cannot reach Infisical at {endpoint}: {cause}")]
    Unreachable {
        /// The endpoint path.
        endpoint: &'static str,
        /// The error chain, no URL.
        cause: String,
    },
    /// Login 401 without the lockout text: wrong, expired or exhausted credentials. Latches (D5).
    #[error(
        "Infisical refused the machine identity's login: the client ID or client secret is \
         wrong, expired or used up"
    )]
    BadCredentials,
    /// Login 401 with the lockout text. Latches (D5).
    #[error(
        "Infisical has temporarily locked the machine identity after repeated failed logins; \
         wait for the lockout to end before trying again"
    )]
    IdentityLocked,
    /// An earlier login by this provider was refused; no request was made (D5).
    #[error(
        "an earlier login with this machine identity was refused; no new login is tried until \
         the identity is entered again"
    )]
    LoginRefusedEarlier,
    /// An earlier login was sent but got no answer (a timeout or a reset after the request went
    /// out), so Infisical may have counted it as a failed attempt. For a short window no login is
    /// tried and no request is made (D5); then exactly one new attempt is allowed.
    #[error(
        "an earlier login with this machine identity got no answer and may have counted as a \
         failed attempt; no new login is tried for another {retry_after_secs} s"
    )]
    LoginCoolingDown {
        /// Whole seconds (rounded up) until the next login may be tried.
        retry_after_secs: u64,
    },
    /// The list endpoint answered 404 `NotFound`.
    #[error("Infisical has no project with the configured project ID")]
    ProjectNotFound,
    /// The list endpoint answered 404 `SecretPathNotFound`. Both fields come from the scope.
    #[error("Infisical has no environment `{environment}` or folder `{path}` in the project")]
    PathNotFound {
        /// The scope's environment slug.
        environment: String,
        /// The scope's folder path.
        path: String,
    },
    /// 403 `PermissionDenied`, `ProjectMembershipNotFound` or any other non-token 403, or a
    /// merged entry with `secretValueHidden: true`.
    #[error("the machine identity may not read these secrets: {detail}")]
    PermissionDenied {
        /// The cleaned server message, or which key is hidden.
        detail: String,
    },
    /// 429 from either endpoint (Infisical Cloud only). Never retried here; M3 decides.
    #[error("Infisical rate-limited the request{}", retry_hint(.retry_after_secs))]
    RateLimited {
        /// `Retry-After` in whole seconds, when the server sent a number.
        retry_after_secs: Option<u64>,
    },
    /// Fastify's "Route … not found" 404: the server predates the endpoint.
    #[error("this Infisical predates v0.150 ({endpoint} does not exist); upgrade it")]
    UnsupportedServer {
        /// The missing endpoint path.
        endpoint: &'static str,
    },
    /// A merged key that is not `^[A-Za-z_][A-Za-z0-9_]*$`. Printed with `{:?}` because the
    /// name may hold control characters.
    #[error(
        "the secret name {key:?} is not a valid environment variable name; rename it in Infisical"
    )]
    InvalidKey {
        /// The key as received.
        key: String,
    },
    /// A merged value holding NUL. The key is already a valid name (validation order: names
    /// first, then values).
    #[error(
        "the secret {key} holds a NUL byte, which an environment variable cannot carry; fix its \
         value in Infisical"
    )]
    InvalidValue {
        /// The key.
        key: String,
    },
    /// A resolved key carries htui's reserved prefix (MOD-10 D16, OQ-B). Raised by
    /// [`check_env`], never by a provider. Key names are not secret; a provider has already
    /// refused a key that is not a valid environment name (`InvalidKey`).
    #[error("the secret name `{key}` is reserved for htui; rename it in Infisical")]
    ReservedKey {
        /// The key.
        key: String,
    },
    /// Anything else: a non-JSON or ill-shaped body, an unexpected status, a redirect, or a
    /// token refused right after a fresh login.
    #[error("unexpected answer from Infisical at {endpoint}: {detail}")]
    Protocol {
        /// The endpoint path.
        endpoint: &'static str,
        /// What was wrong; never a body quote on login, never serde's message.
        detail: String,
    },
}

impl SecretError {
    /// MOD-10 D13: the one refusal sentence a run (`RunFailure::SecretsRefused`), an engine error
    /// (`EngineError::Secrets`) and a chat print: `secrets_refused: <Display>`. Names the cause,
    /// never a value (M2 D4).
    #[must_use]
    pub fn refusal(&self) -> String {
        format!("secrets_refused: {self}")
    }
}

/// `"; retry after {n} s"`, or nothing.
fn retry_hint(secs: &Option<u64>) -> String {
    secs.map(|s| format!("; retry after {s} s"))
        .unwrap_or_default()
}

/// `project.secret_provider`'s value for Infisical (the only provider of this build). Equal to
/// `htui_secrets::InfisicalProvider::KIND`, which `htui-core` cannot name.
pub const INFISICAL: &str = "infisical";

/// MOD-10 D16: a resolved key with this prefix (case-sensitive) is refused: htui's own variables
/// (`HTUI_MCP_*`, `HTUI_LOG*`, `HTUI_TOOL_*`) must never be shadowed by `SessionSpec.env`, which
/// is applied last.
pub const RESERVED_PREFIX: &str = "HTUI_";

/// [`project_scope`]'s [`SecretError::Config`] sentence for a project that names a provider and
/// no scope.
pub const PROVIDER_WITHOUT_SCOPE: &str =
    "the project names a secret provider but has no secret_scope";

/// [`resolve_project`]'s [`SecretError::Config`] sentence for a provider project in a process
/// that was given no [`SecretSource`].
pub const NO_SECRET_SOURCE: &str =
    "this process has no secret source, so the project's secrets cannot be resolved";

/// [`project_scope`]'s [`SecretError::Config`] sentence for a provider this build does not know.
/// `{:?}`, so a control character in the column is escaped.
fn unknown_provider(provider: &str) -> String {
    format!(
        "project.secret_provider {provider:?} is not a provider this build knows (expected \
         \"{INFISICAL}\")"
    )
}

/// [`resolve_project`]'s [`SecretError::Config`] sentence for a source whose provider is not the
/// kind the project's column names.
fn kind_mismatch(kind: &str, column: &str) -> String {
    format!("the secret source answered a `{kind}` provider for a `{column}` project")
}

/// MOD-10 D15: where a process gets its one provider per identity. Production is `htui`'s
/// keyring-backed Infisical source; tests use `fake::FakeSecretSource`.
pub trait SecretSource: Send + Sync + core::fmt::Debug {
    /// The provider for the identity stored now. Called once per walk and per chat; an
    /// implementation returns the **same** `Arc` while nothing it reads has changed, so the
    /// provider's login latch and cool-down persist across walks (M2 D5).
    fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>>;
}

/// MOD-10 D12: the project's scope, or `None` when `secret_provider` is unset (the scope column is
/// then ignored: the provider column is the switch).
///
/// # Errors
///
/// [`SecretError::Config`]: an unknown provider (named with `{:?}`, so a control character is
/// escaped), a provider with no scope, or a scope [`SecretScope::parse`] refuses.
pub fn project_scope(project: &Project) -> Result<Option<SecretScope>, SecretError> {
    let Some(provider) = project.secret_provider.as_deref() else {
        return Ok(None);
    };
    if provider != INFISICAL {
        return Err(SecretError::Config(unknown_provider(provider)));
    }
    let Some(column) = project.secret_scope.as_deref() else {
        return Err(SecretError::Config(PROVIDER_WITHOUT_SCOPE.to_owned()));
    };
    SecretScope::parse(column).map(Some)
}

/// MOD-10 D16: refuses the first key (map order) that starts with [`RESERVED_PREFIX`].
///
/// # Errors
///
/// [`SecretError::ReservedKey`].
pub fn check_env(resolved: &ResolvedSecrets) -> Result<(), SecretError> {
    match resolved
        .as_map()
        .keys()
        .find(|key| key.starts_with(RESERVED_PREFIX))
    {
        Some(key) => Err(SecretError::ReservedKey { key: key.clone() }),
        None => Ok(()),
    }
}

/// MOD-10 D12, D16 (blueprint A-7): the one resolution sequence the walk (`RunSecrets`) and the
/// chat share. `Ok(None)` for a provider-less project, **without touching `source`**. Otherwise,
/// in this order: [`project_scope`] (a column error never reads the keyring); a missing `source`
/// is `Config`; `source.provider()`; a provider whose `kind()` is not the column's is `Config`;
/// `provider.resolve(&scope)`; [`check_env`] (a refused map is dropped, so zeroized).
///
/// # Errors
///
/// [`SecretError::Config`] for a column fault, no source or a provider of another kind; the
/// source's or the provider's own error; [`SecretError::ReservedKey`].
pub fn resolve_project<'a>(
    source: Option<&'a dyn SecretSource>,
    project: &'a Project,
) -> SecretFuture<'a, Option<ResolvedSecrets>> {
    Box::pin(async move {
        let Some(scope) = project_scope(project)? else {
            return Ok(None);
        };
        let Some(source) = source else {
            return Err(SecretError::Config(NO_SECRET_SOURCE.to_owned()));
        };
        let provider = source.provider().await?;
        // `project_scope` answered a scope, so the column is set (and is `INFISICAL`).
        let column = project.secret_provider.as_deref().unwrap_or(INFISICAL);
        if provider.kind() != column {
            return Err(SecretError::Config(kind_mismatch(provider.kind(), column)));
        }
        let resolved = provider.resolve(&scope).await?;
        check_env(&resolved)?;
        Ok(Some(resolved))
    })
}

/// Test doubles for the seam (MOD-10 M3 D16): a scripted provider and a fixed source, both
/// counting their calls. No tokio, no keyring, no HTTP. Built for `htui-core`'s own tests and,
/// behind `test-support`, for `htui-orch`, `htui-worker` and `htui`.
#[cfg(any(test, feature = "test-support"))]
pub mod fake {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

    use zeroize::Zeroize;

    use super::{
        INFISICAL, ProviderHealth, ResolvedSecrets, SecretError, SecretFuture, SecretProvider,
        SecretScope, SecretSource,
    };

    /// One scripted answer: a map to wrap in [`ResolvedSecrets`], or the error to fail with.
    type Answer = Result<BTreeMap<String, String>, SecretError>;

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wipe(answer: &mut Answer) {
        if let Ok(map) = answer {
            for value in map.values_mut() {
                value.zeroize();
            }
        }
    }

    /// The script and the answer it repeats once spent, under one lock.
    struct Script {
        queue: VecDeque<Answer>,
        last: Option<Answer>,
    }

    impl Script {
        /// The answer the next `resolve` gives, without spending it.
        fn peek(&self) -> Answer {
            self.queue
                .front()
                .or(self.last.as_ref())
                .cloned()
                .unwrap_or_else(|| Err(no_scripted_answer()))
        }

        /// The next answer; the last one repeats once the queue is spent.
        fn next(&mut self) -> Answer {
            match self.queue.pop_front() {
                Some(answer) => {
                    if let Some(mut old) = self.last.replace(answer.clone()) {
                        wipe(&mut old);
                    }
                    answer
                }
                None => self
                    .last
                    .clone()
                    .unwrap_or_else(|| Err(no_scripted_answer())),
            }
        }
    }

    impl Drop for Script {
        fn drop(&mut self) {
            self.queue.iter_mut().for_each(wipe);
            if let Some(last) = self.last.as_mut() {
                wipe(last);
            }
        }
    }

    fn no_scripted_answer() -> SecretError {
        SecretError::Protocol {
            endpoint: "fake",
            detail: "no scripted answer".to_owned(),
        }
    }

    /// A provider answering a script, one answer per `resolve`; the last answer repeats once the
    /// script is spent; an empty script answers
    /// `Protocol { endpoint: "fake", detail: "no scripted answer" }`. `list_keys` is the next
    /// `resolve`'s keys (it neither spends nor counts an answer); `health` is `Ok`
    /// (`base_url: "fake://"`, `server_ok: true`). `Debug` prints the kind and the counters, never
    /// a value.
    pub struct FakeSecretProvider {
        kind: &'static str,
        script: Mutex<Script>,
        resolves: AtomicUsize,
        scopes: Mutex<Vec<SecretScope>>,
    }

    impl FakeSecretProvider {
        /// A provider of kind [`INFISICAL`] answering `script` in order.
        pub fn new(script: impl IntoIterator<Item = Answer>) -> Self {
            Self {
                kind: INFISICAL,
                script: Mutex::new(Script {
                    queue: script.into_iter().collect(),
                    last: None,
                }),
                resolves: AtomicUsize::new(0),
                scopes: Mutex::new(Vec::new()),
            }
        }

        /// One `Ok` answer holding `pairs` (repeated for every `resolve`).
        pub fn resolving(pairs: &[(&str, &str)]) -> Self {
            Self::new([Ok(pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect())])
        }

        /// One `Err` answer (repeated for every `resolve`).
        pub fn failing(error: SecretError) -> Self {
            Self::new([Err(error)])
        }

        /// The same provider, reporting `kind` (default [`INFISICAL`]).
        #[must_use]
        pub fn with_kind(mut self, kind: &'static str) -> Self {
            self.kind = kind;
            self
        }

        /// How many times `resolve` was called.
        pub fn resolves(&self) -> usize {
            self.resolves.load(Ordering::SeqCst)
        }

        /// The scopes `resolve` was called with, in call order.
        pub fn scopes(&self) -> Vec<SecretScope> {
            lock(&self.scopes).clone()
        }
    }

    impl core::fmt::Debug for FakeSecretProvider {
        /// `FakeSecretProvider { kind: "infisical", scripted: 1, resolves: 0 }`: no value.
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("FakeSecretProvider")
                .field("kind", &self.kind)
                .field("scripted", &lock(&self.script).queue.len())
                .field("resolves", &self.resolves())
                .finish()
        }
    }

    impl SecretProvider for FakeSecretProvider {
        fn kind(&self) -> &'static str {
            self.kind
        }

        fn health(&self) -> SecretFuture<'_, ProviderHealth> {
            Box::pin(async {
                Ok(ProviderHealth {
                    base_url: "fake://".to_owned(),
                    server_ok: true,
                })
            })
        }

        fn list_keys<'a>(&'a self, _scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>> {
            let answer = lock(&self.script).peek().map(ResolvedSecrets::new);
            Box::pin(async move { answer.map(|resolved| resolved.keys()) })
        }

        fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets> {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            lock(&self.scopes).push(scope.clone());
            let answer = lock(&self.script).next().map(ResolvedSecrets::new);
            Box::pin(async move { answer })
        }
    }

    /// A source answering one fixed provider (or one fixed error) and counting its calls.
    pub struct FakeSecretSource {
        answer: Result<Arc<dyn SecretProvider>, SecretError>,
        calls: AtomicUsize,
    }

    impl FakeSecretSource {
        /// A source answering `provider` on every call.
        pub fn new(provider: Arc<dyn SecretProvider>) -> Self {
            Self {
                answer: Ok(provider),
                calls: AtomicUsize::new(0),
            }
        }

        /// A source failing every call with `error` (a keyring with no identity, say).
        pub fn failing(error: SecretError) -> Self {
            Self {
                answer: Err(error),
                calls: AtomicUsize::new(0),
            }
        }

        /// How many times `provider` was called.
        pub fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl core::fmt::Debug for FakeSecretSource {
        /// The provider's kind or the error's variant sentence, and the call count: no value.
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            let mut out = f.debug_struct("FakeSecretSource");
            match &self.answer {
                Ok(provider) => out.field("provider", &provider.kind()),
                Err(error) => out.field("error", &error.to_string()),
            };
            out.field("calls", &self.calls()).finish()
        }
    }

    impl SecretSource for FakeSecretSource {
        fn provider(&self) -> SecretFuture<'_, Arc<dyn SecretProvider>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let answer = self.answer.clone();
            Box::pin(async move { answer })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn config_sentence(result: Result<SecretScope, SecretError>) -> String {
        match result {
            Err(SecretError::Config(sentence)) => sentence,
            Err(other) => panic!("expected Config, got {other:?}"),
            Ok(scope) => panic!("expected Config, got {scope:?}"),
        }
    }

    #[test]
    fn scope_parses_the_column_shape() {
        let scope = SecretScope::parse(r#"{"project_id":"p1","environment":"dev","path":"/app"}"#)
            .expect("a well-formed column parses");
        assert_eq!(scope.project_id(), "p1");
        assert_eq!(scope.environment(), "dev");
        assert_eq!(scope.path(), "/app");
    }

    #[test]
    fn scope_path_defaults_to_root() {
        let scope = SecretScope::parse(r#"{"project_id":"p1","environment":"dev"}"#)
            .expect("path is optional");
        assert_eq!(scope.path(), "/");
    }

    #[test]
    fn scope_round_trips_through_the_column() {
        let scope = SecretScope::new("p1", "dev", "/app/api").expect("valid scope");
        let back = SecretScope::parse(&scope.to_column()).expect("to_column output parses");
        assert_eq!(back, scope);
    }

    #[test]
    fn scope_column_is_compact_and_ordered() {
        let scope = SecretScope::new("p1", "dev", "/").expect("valid scope");
        assert_eq!(
            scope.to_column(),
            r#"{"project_id":"p1","environment":"dev","path":"/"}"#
        );
    }

    #[test]
    fn scope_refuses_unknown_fields() {
        let sentence = config_sentence(SecretScope::parse(
            r#"{"project_id":"p","environment":"dev","extra":1}"#,
        ));
        assert!(
            sentence.starts_with("the secret scope is not valid: "),
            "unexpected sentence: {sentence}"
        );
    }

    #[test]
    fn scope_refuses_an_empty_project_or_environment() {
        for blank in ["", "  "] {
            assert_eq!(
                config_sentence(SecretScope::new(blank, "dev", "/")),
                "the secret scope has an empty project_id"
            );
            assert_eq!(
                config_sentence(SecretScope::new("p1", blank, "/")),
                "the secret scope has an empty environment"
            );
        }
        // `parse` applies the same refusals.
        assert_eq!(
            config_sentence(SecretScope::parse(
                r#"{"project_id":" ","environment":"dev"}"#
            )),
            "the secret scope has an empty project_id"
        );
    }

    #[test]
    fn scope_refuses_a_path_without_a_leading_slash() {
        for path in ["app", ""] {
            assert_eq!(
                config_sentence(SecretScope::new("p1", "dev", path)),
                "the secret scope path must start with `/`"
            );
        }
    }

    /// A control character would reach a terminal raw through `PathNotFound`'s `Display`; the
    /// refusal names the field, never the value.
    #[test]
    fn scope_refuses_control_characters_naming_the_field_only() {
        for bad in ["a\nb", "\u{1b}[2J", "x\u{7f}", "a\u{85}", "a\tb", "\0"] {
            let path = format!("/{bad}");
            let rows = [
                (
                    SecretScope::new(bad, "dev", "/"),
                    "the secret scope's project_id holds a control character",
                ),
                (
                    SecretScope::new("p1", bad, "/"),
                    "the secret scope's environment holds a control character",
                ),
                (
                    SecretScope::new("p1", "dev", path.as_str()),
                    "the secret scope's path holds a control character",
                ),
            ];
            for (result, want) in rows {
                let sentence = config_sentence(result);
                assert_eq!(sentence, want, "input {bad:?}");
            }
        }
        // `parse` applies the same refusal.
        assert_eq!(
            config_sentence(SecretScope::parse(
                r#"{"project_id":"p1","environment":"dev","path":"/a\u001b"}"#
            )),
            "the secret scope's path holds a control character"
        );
        // Printable non-ASCII is not a control character.
        SecretScope::new("p1", "dév", "/é/✓").expect("printable Unicode is accepted");
    }

    /// serde quotes an unknown key with `Display`: a control character in it must reach the
    /// `Config` sentence escaped, never raw.
    #[test]
    fn scope_parse_escapes_control_characters_in_serde_messages() {
        let sentence = config_sentence(SecretScope::parse(
            r#"{"project_id":"p1","environment":"dev","x\u001b[2J\u0007":1}"#,
        ));
        assert!(
            sentence.starts_with("the secret scope is not valid: "),
            "unexpected sentence: {sentence:?}"
        );
        assert!(
            !sentence.chars().any(char::is_control),
            "a raw control character reached the sentence: {sentence:?}"
        );
        assert!(
            sentence.contains(r"x\u{1b}[2J\u{7}"),
            "the key is not shown escaped: {sentence:?}"
        );
    }

    #[test]
    fn scope_refuses_text_that_is_not_json() {
        let sentence = config_sentence(SecretScope::parse("p1/dev"));
        assert!(
            sentence.starts_with("the secret scope is not valid: "),
            "unexpected sentence: {sentence}"
        );
    }

    #[test]
    fn resolved_secrets_debug_prints_keys_only() {
        let resolved = ResolvedSecrets::new(BTreeMap::from([
            ("A".to_owned(), "s3cret-a".to_owned()),
            ("B".to_owned(), "s3cret-b".to_owned()),
        ]));
        let debug = format!("{resolved:?}");
        assert!(
            !debug.contains("s3cret-a") && !debug.contains("s3cret-b"),
            "ResolvedSecrets' Debug prints a value"
        );
        assert!(
            debug == r#"ResolvedSecrets { keys: ["A", "B"] }"#,
            "ResolvedSecrets' Debug is not the key list"
        );
    }

    #[test]
    fn resolved_secrets_lends_the_map_and_sorted_keys() {
        let mut input = BTreeMap::new();
        input.insert("B".to_owned(), "vb".to_owned());
        input.insert("A".to_owned(), "va".to_owned());
        let resolved = ResolvedSecrets::new(input.clone());
        assert!(resolved.as_map() == &input, "as_map differs from the input");
        assert_eq!(resolved.keys(), vec!["A".to_owned(), "B".to_owned()]);
        assert_eq!(resolved.len(), 2);
        assert!(!resolved.is_empty());

        let empty = ResolvedSecrets::new(BTreeMap::new());
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
        assert!(empty.keys().is_empty());
    }

    #[test]
    fn machine_identity_debug_redacts_the_secret() {
        let identity = MachineIdentity::new("cid-1", "csecret-xyz");
        let debug = format!("{identity:?}");
        assert!(
            debug.contains("cid-1"),
            "MachineIdentity's Debug hides the client ID"
        );
        assert!(
            debug.contains("<redacted>"),
            "MachineIdentity's Debug has no redaction marker"
        );
        assert!(
            !debug.contains("csecret-xyz"),
            "MachineIdentity's Debug prints the client secret"
        );
        assert_eq!(identity.client_id(), "cid-1");
        assert!(
            identity.client_secret() == "csecret-xyz",
            "client_secret() does not return the secret half"
        );
    }

    #[test]
    fn provider_health_is_plain_data() {
        let health = ProviderHealth {
            base_url: "https://infisical.example".to_owned(),
            server_ok: true,
        };
        assert_eq!(health.clone(), health);
        assert_eq!(
            format!("{health:?}"),
            r#"ProviderHealth { base_url: "https://infisical.example", server_ok: true }"#
        );
    }

    #[test]
    fn secret_error_display_is_exact() {
        assert_eq!(
            SecretError::NoIdentity.to_string(),
            "no Infisical machine identity is stored in the OS keyring"
        );
        assert_eq!(
            SecretError::Config("the Infisical base URL is empty".into()).to_string(),
            "secret provider configuration: the Infisical base URL is empty"
        );
        assert_eq!(
            SecretError::Unreachable {
                endpoint: "/api/status",
                cause: "error sending request: connection refused".into(),
            }
            .to_string(),
            "cannot reach Infisical at /api/status: error sending request: connection refused"
        );
        assert_eq!(
            SecretError::BadCredentials.to_string(),
            "Infisical refused the machine identity's login: the client ID or client secret is \
             wrong, expired or used up"
        );
        assert_eq!(
            SecretError::IdentityLocked.to_string(),
            "Infisical has temporarily locked the machine identity after repeated failed logins; \
             wait for the lockout to end before trying again"
        );
        assert_eq!(
            SecretError::LoginRefusedEarlier.to_string(),
            "an earlier login with this machine identity was refused; no new login is tried until \
             the identity is entered again"
        );
        assert_eq!(
            SecretError::LoginCoolingDown {
                retry_after_secs: 30
            }
            .to_string(),
            "an earlier login with this machine identity got no answer and may have counted as a \
             failed attempt; no new login is tried for another 30 s"
        );
        assert_eq!(
            SecretError::ProjectNotFound.to_string(),
            "Infisical has no project with the configured project ID"
        );
        assert_eq!(
            SecretError::PathNotFound {
                environment: "dev".into(),
                path: "/app".into(),
            }
            .to_string(),
            "Infisical has no environment `dev` or folder `/app` in the project"
        );
        assert_eq!(
            SecretError::PermissionDenied {
                detail: "no read on dev".into(),
            }
            .to_string(),
            "the machine identity may not read these secrets: no read on dev"
        );
        assert_eq!(
            SecretError::RateLimited {
                retry_after_secs: Some(30),
            }
            .to_string(),
            "Infisical rate-limited the request; retry after 30 s"
        );
        assert_eq!(
            SecretError::RateLimited {
                retry_after_secs: None,
            }
            .to_string(),
            "Infisical rate-limited the request"
        );
        assert_eq!(
            SecretError::UnsupportedServer {
                endpoint: "/api/v4/secrets",
            }
            .to_string(),
            "this Infisical predates v0.150 (/api/v4/secrets does not exist); upgrade it"
        );
        assert_eq!(
            SecretError::InvalidKey { key: "1BAD".into() }.to_string(),
            "the secret name \"1BAD\" is not a valid environment variable name; rename it in \
             Infisical"
        );
        assert_eq!(
            SecretError::InvalidValue {
                key: "TOKEN".into(),
            }
            .to_string(),
            "the secret TOKEN holds a NUL byte, which an environment variable cannot carry; fix \
             its value in Infisical"
        );
        assert_eq!(
            SecretError::Protocol {
                endpoint: "/api/v4/secrets",
                detail: "status 500".into(),
            }
            .to_string(),
            "unexpected answer from Infisical at /api/v4/secrets: status 500"
        );
    }

    /// Sentinels standing in for a secret value, a client secret and an access token.
    const VALUE: &str = "leak-value-1";
    const SECRET: &str = "leak-secret-1";
    const TOKEN: &str = "leak-token-1";

    /// The variant's name. No wildcard arm: a new variant fails to compile here until it is
    /// added to [`every_variant`] too.
    fn variant(e: &SecretError) -> &'static str {
        match e {
            SecretError::NoIdentity => "NoIdentity",
            SecretError::Config(_) => "Config",
            SecretError::Unreachable { .. } => "Unreachable",
            SecretError::BadCredentials => "BadCredentials",
            SecretError::IdentityLocked => "IdentityLocked",
            SecretError::LoginRefusedEarlier => "LoginRefusedEarlier",
            SecretError::LoginCoolingDown { .. } => "LoginCoolingDown",
            SecretError::ProjectNotFound => "ProjectNotFound",
            SecretError::PathNotFound { .. } => "PathNotFound",
            SecretError::PermissionDenied { .. } => "PermissionDenied",
            SecretError::RateLimited { .. } => "RateLimited",
            SecretError::UnsupportedServer { .. } => "UnsupportedServer",
            SecretError::InvalidKey { .. } => "InvalidKey",
            SecretError::InvalidValue { .. } => "InvalidValue",
            SecretError::ReservedKey { .. } => "ReservedKey",
            SecretError::Protocol { .. } => "Protocol",
        }
    }

    /// One of each variant. The sentinels go only into the fields that are free text by design
    /// (`Config`, `cause`, `detail`); every other field holds what it would in production.
    fn every_variant() -> Vec<SecretError> {
        let free_text = format!("{VALUE} {SECRET} {TOKEN}");
        vec![
            SecretError::NoIdentity,
            SecretError::Config(free_text.clone()),
            SecretError::Unreachable {
                endpoint: "/api/status",
                cause: free_text.clone(),
            },
            SecretError::BadCredentials,
            SecretError::IdentityLocked,
            SecretError::LoginRefusedEarlier,
            SecretError::LoginCoolingDown {
                retry_after_secs: 30,
            },
            SecretError::ProjectNotFound,
            SecretError::PathNotFound {
                environment: "dev".into(),
                path: "/app".into(),
            },
            SecretError::PermissionDenied {
                detail: free_text.clone(),
            },
            SecretError::RateLimited {
                retry_after_secs: Some(7),
            },
            SecretError::UnsupportedServer {
                endpoint: "/api/v4/secrets",
            },
            SecretError::InvalidKey {
                key: "BAD-KEY".into(),
            },
            SecretError::InvalidValue {
                key: "NUL_KEY".into(),
            },
            SecretError::ReservedKey {
                key: "HTUI_LOG".into(),
            },
            SecretError::Protocol {
                endpoint: "/api/v4/secrets",
                detail: free_text,
            },
        ]
    }

    fn carries_a_sentinel(text: &str) -> bool {
        [VALUE, SECRET, TOKEN].iter().any(|s| text.contains(s))
    }

    #[test]
    fn every_secret_error_variant_is_covered() {
        let all = every_variant();
        let names: BTreeSet<&'static str> = all.iter().map(variant).collect();
        assert_eq!(
            names.len(),
            16,
            "every_variant() misses or repeats a variant"
        );
        assert_eq!(all.len(), 16, "every_variant() repeats a variant");

        for e in &all {
            let name = variant(e);
            let (display, debug) = (e.to_string(), format!("{e:?}"));
            match e {
                // Free text by design: the structure is what this test pins.
                SecretError::Config(_)
                | SecretError::Unreachable { .. }
                | SecretError::PermissionDenied { .. }
                | SecretError::Protocol { .. } => {}
                // Key-carrying variants print their key and nothing else of the secret.
                SecretError::InvalidKey { key }
                | SecretError::InvalidValue { key }
                | SecretError::ReservedKey { key } => {
                    assert!(
                        display.contains(key.as_str()),
                        "{name} does not name its key"
                    );
                    assert!(
                        !carries_a_sentinel(&display),
                        "{name}'s Display carries a sentinel"
                    );
                    assert!(
                        !carries_a_sentinel(&debug),
                        "{name}'s Debug carries a sentinel"
                    );
                }
                SecretError::NoIdentity
                | SecretError::BadCredentials
                | SecretError::IdentityLocked
                | SecretError::LoginRefusedEarlier
                | SecretError::LoginCoolingDown { .. }
                | SecretError::ProjectNotFound
                | SecretError::PathNotFound { .. }
                | SecretError::RateLimited { .. }
                | SecretError::UnsupportedServer { .. } => {
                    assert!(
                        !carries_a_sentinel(&display),
                        "{name}'s Display carries a sentinel"
                    );
                    assert!(
                        !carries_a_sentinel(&debug),
                        "{name}'s Debug carries a sentinel"
                    );
                }
            }
        }
    }

    /// A provider answering fixed data, to prove the trait is dyn-compatible and `Send`.
    #[derive(Debug)]
    struct Fixed;

    impl SecretProvider for Fixed {
        fn kind(&self) -> &'static str {
            "fixed"
        }

        fn health(&self) -> SecretFuture<'_, ProviderHealth> {
            Box::pin(async {
                Ok(ProviderHealth {
                    base_url: "https://fixed.invalid".to_owned(),
                    server_ok: true,
                })
            })
        }

        fn list_keys<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>> {
            Box::pin(async move { Ok(vec![format!("{}_KEY", scope.environment().to_uppercase())]) })
        }

        fn resolve<'a>(&'a self, scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets> {
            Box::pin(async move {
                if scope.path() != "/" {
                    return Err(SecretError::PathNotFound {
                        environment: scope.environment().to_owned(),
                        path: scope.path().to_owned(),
                    });
                }
                Ok(ResolvedSecrets::new(BTreeMap::from([(
                    format!("{}_KEY", scope.environment().to_uppercase()),
                    "fixed-value".to_owned(),
                )])))
            })
        }
    }

    #[tokio::test]
    async fn a_provider_is_usable_as_arc_dyn() {
        let provider: Arc<dyn SecretProvider> = Arc::new(Fixed);
        let scope = SecretScope::new("p1", "dev", "/").expect("valid scope");

        assert_eq!(provider.kind(), "fixed");
        assert_eq!(
            provider.health().await.expect("health answers"),
            ProviderHealth {
                base_url: "https://fixed.invalid".to_owned(),
                server_ok: true,
            }
        );
        assert_eq!(
            provider.list_keys(&scope).await.expect("list_keys answers"),
            vec!["DEV_KEY".to_owned()]
        );
        let resolved = provider.resolve(&scope).await.expect("resolve answers");
        assert_eq!(resolved.keys(), vec!["DEV_KEY".to_owned()]);

        let elsewhere = SecretScope::new("p1", "dev", "/app").expect("valid scope");
        assert_eq!(
            provider.resolve(&elsewhere).await.map(|r| r.keys()),
            Err(SecretError::PathNotFound {
                environment: "dev".into(),
                path: "/app".into(),
            })
        );

        // The futures cross threads: a provider is shared by the worker's tasks.
        fn assert_send<T: Send>(_: &T) {}
        assert_send(&provider.resolve(&scope));
    }

    // ---- MOD-10 M3 T3 (D12, D15, D16; blueprint A-7) -------------------------------------

    use crate::model::{Project, ProjectId, UserId};
    use fake::{FakeSecretProvider, FakeSecretSource};

    const SCOPE_COLUMN: &str = r#"{"project_id":"p1","environment":"dev","path":"/app"}"#;

    fn project(provider: Option<&str>, scope: Option<&str>) -> Project {
        let now = chrono::Utc::now();
        Project {
            id: ProjectId::new(),
            slug: "p".to_owned(),
            name: "P".to_owned(),
            description: String::new(),
            secret_provider: provider.map(str::to_owned),
            secret_scope: scope.map(str::to_owned),
            settings: serde_json::json!({}),
            created_by: UserId::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn infisical(scope: Option<&str>) -> Project {
        project(Some(INFISICAL), scope)
    }

    fn the_scope() -> SecretScope {
        SecretScope::parse(SCOPE_COLUMN).expect("the test scope parses")
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn config<T: core::fmt::Debug>(result: Result<T, SecretError>) -> String {
        match result {
            Err(SecretError::Config(sentence)) => sentence,
            other => panic!("expected Config, got {other:?}"),
        }
    }

    #[test]
    fn project_scope_is_none_without_a_provider() {
        for scope in [None, Some("not json at all"), Some(SCOPE_COLUMN)] {
            assert_eq!(
                project_scope(&project(None, scope)),
                Ok(None),
                "the provider column is the switch (scope {scope:?})"
            );
        }
    }

    #[test]
    fn project_scope_parses_an_infisical_scope() {
        assert_eq!(
            project_scope(&infisical(Some(SCOPE_COLUMN))),
            Ok(Some(the_scope()))
        );
    }

    #[test]
    fn project_scope_refuses_an_unknown_provider_and_escapes_it() {
        assert_eq!(
            config(project_scope(&project(Some("vault"), Some(SCOPE_COLUMN)))),
            r#"project.secret_provider "vault" is not a provider this build knows (expected "infisical")"#
        );
        let sentence = config(project_scope(&project(
            Some("inf\u{1b}[31m"),
            Some(SCOPE_COLUMN),
        )));
        assert_eq!(
            sentence,
            r#"project.secret_provider "inf\u{1b}[31m" is not a provider this build knows (expected "infisical")"#
        );
        assert!(
            !sentence.chars().any(char::is_control),
            "a raw control character reached the sentence: {sentence:?}"
        );
        // Case matters: the column holds the provider's `kind()` verbatim.
        config(project_scope(&project(
            Some("Infisical"),
            Some(SCOPE_COLUMN),
        )));
    }

    #[test]
    fn project_scope_refuses_a_provider_without_a_scope() {
        assert_eq!(
            config(project_scope(&infisical(None))),
            PROVIDER_WITHOUT_SCOPE
        );
        assert_eq!(
            SecretError::Config(PROVIDER_WITHOUT_SCOPE.to_owned()).to_string(),
            "secret provider configuration: the project names a secret provider but has no \
             secret_scope"
        );
    }

    #[test]
    fn project_scope_refuses_an_unparsable_scope() {
        let sentence = config(project_scope(&infisical(Some("p1/dev"))));
        assert!(
            sentence.starts_with("the secret scope is not valid: "),
            "unexpected sentence: {sentence}"
        );
        assert_eq!(
            config(project_scope(&infisical(Some(
                r#"{"project_id":"p1","environment":"dev","path":"app"}"#
            )))),
            "the secret scope path must start with `/`"
        );
    }

    #[test]
    fn check_env_refuses_a_reserved_key_and_names_it() {
        let resolved = ResolvedSecrets::new(map(&[
            ("API_KEY", "v1"),
            ("HTUI_MCP_TOKEN", "v2"),
            ("HTUI_ZZZ", "v3"),
        ]));
        let err = check_env(&resolved).expect_err("a reserved key is refused");
        assert_eq!(
            err,
            SecretError::ReservedKey {
                key: "HTUI_MCP_TOKEN".into()
            },
            "the first reserved key in map order is named"
        );
        assert_eq!(
            err.to_string(),
            "the secret name `HTUI_MCP_TOKEN` is reserved for htui; rename it in Infisical"
        );
    }

    #[test]
    fn check_env_is_case_sensitive_and_needs_the_underscore() {
        let resolved = ResolvedSecrets::new(map(&[
            ("htui_x", "v"),
            ("HTUIX", "v"),
            ("MY_HTUI_X", "v"),
            ("HTUI", "v"),
        ]));
        assert_eq!(check_env(&resolved), Ok(()));
    }

    #[test]
    fn check_env_accepts_an_empty_map() {
        assert_eq!(check_env(&ResolvedSecrets::new(BTreeMap::new())), Ok(()));
    }

    #[test]
    fn secret_error_refusal_prefixes_the_display() {
        assert_eq!(
            SecretError::BadCredentials.refusal(),
            "secrets_refused: Infisical refused the machine identity's login: the client ID or \
             client secret is wrong, expired or used up"
        );
        for e in every_variant() {
            assert_eq!(e.refusal(), format!("secrets_refused: {e}"));
        }
    }

    fn source_of(provider: FakeSecretProvider) -> FakeSecretSource {
        FakeSecretSource::new(Arc::new(provider))
    }

    #[tokio::test]
    async fn resolve_project_never_touches_the_source_without_a_provider() {
        let source = source_of(FakeSecretProvider::resolving(&[("A", "va")]));
        for scope in [None, Some("garbage")] {
            let got = resolve_project(Some(&source), &project(None, scope))
                .await
                .expect("a provider-less project resolves to nothing");
            assert!(got.is_none(), "a provider-less project got a map");
        }
        let got = resolve_project(None, &project(None, None))
            .await
            .expect("no source is fine without a provider");
        assert!(got.is_none(), "a provider-less project got a map");
        assert_eq!(source.calls(), 0, "the source was asked for a provider");
    }

    #[tokio::test]
    async fn resolve_project_refuses_a_column_fault_before_the_source() {
        let source = source_of(FakeSecretProvider::resolving(&[("A", "va")]));
        for bad in [
            project(Some("vault"), Some(SCOPE_COLUMN)),
            infisical(None),
            infisical(Some("p1/dev")),
        ] {
            let want = config(project_scope(&bad));
            assert_eq!(
                config(resolve_project(Some(&source), &bad).await),
                want,
                "resolve_project refuses with project_scope's sentence"
            );
        }
        assert_eq!(source.calls(), 0, "a column fault read the source");
    }

    #[tokio::test]
    async fn resolve_project_refuses_without_a_source() {
        assert_eq!(
            config(resolve_project(None, &infisical(Some(SCOPE_COLUMN))).await),
            NO_SECRET_SOURCE
        );
        assert_eq!(
            NO_SECRET_SOURCE,
            "this process has no secret source, so the project's secrets cannot be resolved"
        );
    }

    #[tokio::test]
    async fn resolve_project_refuses_a_provider_of_another_kind() {
        let provider = Arc::new(FakeSecretProvider::resolving(&[("A", "va")]).with_kind("vault"));
        let source = FakeSecretSource::new(provider.clone());
        assert_eq!(
            config(resolve_project(Some(&source), &infisical(Some(SCOPE_COLUMN))).await),
            "the secret source answered a `vault` provider for a `infisical` project"
        );
        assert_eq!(source.calls(), 1);
        assert_eq!(
            provider.resolves(),
            0,
            "a foreign provider was asked to resolve"
        );
    }

    #[tokio::test]
    async fn resolve_project_passes_a_provider_error_through() {
        let source = source_of(FakeSecretProvider::failing(SecretError::BadCredentials));
        assert_eq!(
            resolve_project(Some(&source), &infisical(Some(SCOPE_COLUMN)))
                .await
                .map(|r| r.map(|r| r.keys())),
            Err(SecretError::BadCredentials)
        );
        // A source that cannot build its provider refuses the same way.
        let broken = FakeSecretSource::failing(SecretError::NoIdentity);
        assert_eq!(
            resolve_project(Some(&broken), &infisical(Some(SCOPE_COLUMN)))
                .await
                .map(|r| r.map(|r| r.keys())),
            Err(SecretError::NoIdentity)
        );
        assert_eq!(broken.calls(), 1);
    }

    #[tokio::test]
    async fn resolve_project_refuses_a_reserved_key_from_the_provider() {
        let source = source_of(FakeSecretProvider::resolving(&[
            ("API_KEY", "va"),
            ("HTUI_LOG", "vb"),
        ]));
        assert_eq!(
            resolve_project(Some(&source), &infisical(Some(SCOPE_COLUMN)))
                .await
                .map(|r| r.map(|r| r.keys())),
            Err(SecretError::ReservedKey {
                key: "HTUI_LOG".into()
            })
        );
    }

    #[tokio::test]
    async fn resolve_project_returns_the_map_and_the_scope_reached_the_provider() {
        let provider = Arc::new(FakeSecretProvider::resolving(&[("A", "va"), ("B", "vb")]));
        let source = FakeSecretSource::new(provider.clone());
        let resolved = resolve_project(Some(&source), &infisical(Some(SCOPE_COLUMN)))
            .await
            .expect("the provider answers")
            .expect("an Infisical project resolves");
        assert!(
            resolved.as_map() == &map(&[("A", "va"), ("B", "vb")]),
            "the resolved map is not the provider's"
        );
        assert_eq!(provider.scopes(), vec![the_scope()]);
        assert_eq!(provider.resolves(), 1);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn the_fakes_count_and_repeat_the_last_answer() {
        let provider = FakeSecretProvider::new([
            Ok(map(&[("A", "va")])),
            Err(SecretError::RateLimited {
                retry_after_secs: None,
            }),
        ]);
        assert_eq!(provider.kind(), INFISICAL);
        let scope = the_scope();
        let mut answers = Vec::new();
        for _ in 0..3 {
            answers.push(provider.resolve(&scope).await.map(|r| r.keys()));
        }
        let limited = Err(SecretError::RateLimited {
            retry_after_secs: None,
        });
        assert_eq!(
            answers,
            vec![Ok(vec!["A".to_owned()]), limited.clone(), limited]
        );
        assert_eq!(provider.resolves(), 3);
        assert_eq!(provider.scopes(), vec![scope.clone(); 3]);
        assert_eq!(
            provider.health().await,
            Ok(ProviderHealth {
                base_url: "fake://".to_owned(),
                server_ok: true,
            })
        );

        let empty = FakeSecretProvider::new([]);
        assert_eq!(
            empty.resolve(&scope).await.map(|r| r.keys()),
            Err(SecretError::Protocol {
                endpoint: "fake",
                detail: "no scripted answer".into(),
            })
        );

        // `list_keys` is the next `resolve`'s keys, and neither counts nor spends it.
        let listing = FakeSecretProvider::resolving(&[("B", "vb"), ("A", "va")]);
        assert_eq!(
            listing.list_keys(&scope).await,
            Ok(vec!["A".to_owned(), "B".to_owned()])
        );
        assert_eq!(listing.resolves(), 0);

        let source = source_of(FakeSecretProvider::resolving(&[]));
        for _ in 0..2 {
            let provider = source.provider().await.expect("the fixed provider");
            assert_eq!(provider.kind(), INFISICAL);
        }
        assert_eq!(source.calls(), 2);
        let failing = FakeSecretSource::failing(SecretError::NoIdentity);
        assert_eq!(
            failing.provider().await.map(|p| p.kind()),
            Err(SecretError::NoIdentity)
        );
        assert_eq!(failing.calls(), 1);
    }

    #[tokio::test]
    async fn the_fakes_never_print_a_value() {
        let provider = Arc::new(FakeSecretProvider::new([
            Ok(map(&[("A", VALUE)])),
            Ok(map(&[("B", SECRET)])),
        ]));
        let source = FakeSecretSource::new(provider.clone());
        let before = format!("{provider:?} {source:?}");
        provider
            .resolve(&the_scope())
            .await
            .expect("the scripted answer");
        let after = format!("{provider:?} {source:?}");
        for debug in [before, after] {
            assert!(
                !carries_a_sentinel(&debug),
                "a fake's Debug prints a value: {debug}"
            );
            assert!(
                debug.contains(INFISICAL),
                "a fake's Debug does not name its kind: {debug}"
            );
        }
    }

    #[tokio::test]
    async fn a_source_is_usable_as_arc_dyn_and_send() {
        let source: Arc<dyn SecretSource> =
            Arc::new(source_of(FakeSecretProvider::resolving(&[("A", "va")])));
        let project = infisical(Some(SCOPE_COLUMN));

        fn assert_send<T: Send>(_: &T) {}
        fn assert_send_sync<T: Send + Sync + ?Sized>(_: &T) {}
        assert_send_sync(&*source);
        let future = resolve_project(Some(source.as_ref()), &project);
        assert_send(&future);
        let resolved = future
            .await
            .expect("the provider answers")
            .expect("an Infisical project resolves");
        assert_eq!(resolved.keys(), vec!["A".to_owned()]);
        assert_send(&source.provider());
    }
}
