//! The Infisical provider against a real server (MOD-10 D10). Gated on `HTUI_TEST_INFISICAL_URL`;
//! unset, it prints the skip line and passes (as `htui-store/tests/qdrant_live.rs`). With the URL
//! set, the other four variables are required, and a missing one panics by name.
//!
//! Use a throwaway Universal Auth identity with read access to one test environment holding at
//! least one secret.
//!
//! Output rule (H-8): nothing here prints a key name, a value, the client secret or the access
//! token, not even on failure. A missing variable panics with its name only; a failed call
//! panics with the `SecretError` variant's name only (`docs/htui-secrets.md` says what each one
//! means); the one success line prints the key count.

use htui_core::secret::{MachineIdentity, SecretError, SecretProvider as _, SecretScope};
use htui_secrets::{InfisicalConfig, InfisicalProvider};

const ENV_URL: &str = "HTUI_TEST_INFISICAL_URL";
const ENV_CLIENT_ID: &str = "HTUI_TEST_INFISICAL_CLIENT_ID";
const ENV_CLIENT_SECRET: &str = "HTUI_TEST_INFISICAL_CLIENT_SECRET";
const ENV_PROJECT_ID: &str = "HTUI_TEST_INFISICAL_PROJECT_ID";
const ENV_ENVIRONMENT: &str = "HTUI_TEST_INFISICAL_ENVIRONMENT";

/// The test server's base URL, or the skip line.
fn infisical_url() -> Option<String> {
    let url = std::env::var(ENV_URL).ok();
    if url.is_none() {
        eprintln!("skipped: {ENV_URL} not set");
    }
    url
}

/// A variable that must be set once the URL is. The panic names the variable, never its value.
fn required(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!("{name} must be set (and not blank) when {ENV_URL} is"),
    }
}

/// The variant's name. No wildcard arm, so a new variant fails to compile here.
fn variant(e: &SecretError) -> &'static str {
    match e {
        SecretError::NoIdentity => "NoIdentity",
        SecretError::Config(_) => "Config",
        SecretError::Unreachable { .. } => "Unreachable",
        SecretError::BadCredentials => "BadCredentials",
        SecretError::IdentityLocked => "IdentityLocked",
        SecretError::LoginRefusedEarlier => "LoginRefusedEarlier",
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

/// `Ok` or a panic naming the step and the variant. Never `expect`, which would print the error,
/// and for `resolve` the `Debug` of the other side.
#[track_caller]
fn must<T>(result: Result<T, SecretError>, step: &str) -> T {
    match result {
        Ok(value) => value,
        Err(e) => panic!(
            "{step} failed with SecretError::{}; see docs/htui-secrets.md, \"Errors and what to do\"",
            variant(&e)
        ),
    }
}

#[tokio::test]
async fn resolves_the_configured_scope() {
    let Some(url) = infisical_url() else {
        return;
    };
    let identity = MachineIdentity::new(required(ENV_CLIENT_ID), required(ENV_CLIENT_SECRET));
    let scope = must(
        SecretScope::new(required(ENV_PROJECT_ID), required(ENV_ENVIRONMENT), "/"),
        "the scope",
    );
    let provider = must(
        InfisicalProvider::new(InfisicalConfig::new(url), identity),
        "building the provider",
    );

    let health = must(provider.health().await, "health");
    assert!(
        health.server_ok,
        "the server's status endpoint did not answer 2xx"
    );

    let resolved = must(provider.resolve(&scope).await, "resolve");
    assert!(
        !resolved.is_empty(),
        "the scope resolved to no secrets; put at least one in the test environment's root folder"
    );

    // H-8: `assert!`, not `assert_eq!`, which would print both key lists on failure.
    let listed = must(provider.list_keys(&scope).await, "list_keys");
    assert!(
        listed == resolved.keys(),
        "list_keys and resolve disagree on the key names"
    );

    eprintln!("resolved {} keys", resolved.len());
}
