//! The secret-provider seam (MOD-10 D1–D4): the trait a provider implements, the scope it reads,
//! the map it returns and the one error it fails with.
//!
//! Lives in `htui-core` so `htui-worker`, `htui-orch` and `htui` can hold an
//! `Arc<dyn SecretProvider>` without depending on HTTP. The Infisical implementation is the
//! `htui-secrets` crate. The machine identity's keyring entries are `htui-store::secret`'s.
//!
//! **No type here prints a secret.** `ResolvedSecrets` and `MachineIdentity` have hand-written
//! `Debug`s, and no `SecretError` variant carries a value, a client secret or a token.

use serde::{Deserialize, Serialize};

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
    /// [`SecretError::Config`] when `project_id` or `environment` is empty or whitespace, or
    /// `path` does not start with `/`.
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
        // The column holds no secret, so serde's message may be quoted here.
        let parsed: ScopeColumn = serde_json::from_str(column)
            .map_err(|err| SecretError::Config(format!("the secret scope is not valid: {err}")))?;
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

    #[test]
    fn scope_refuses_text_that_is_not_json() {
        let sentence = config_sentence(SecretScope::parse("p1/dev"));
        assert!(
            sentence.starts_with("the secret scope is not valid: "),
            "unexpected sentence: {sentence}"
        );
    }
}
