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

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

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

/// `"; retry after {n} s"`, or nothing.
fn retry_hint(secs: &Option<u64>) -> String {
    secs.map(|s| format!("; retry after {s} s"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

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
            15,
            "every_variant() misses or repeats a variant"
        );
        assert_eq!(all.len(), 15, "every_variant() repeats a variant");

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
                SecretError::InvalidKey { key } | SecretError::InvalidValue { key } => {
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
}
