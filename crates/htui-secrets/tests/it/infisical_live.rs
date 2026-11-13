//! The Infisical provider against a real server (MOD-10 D10). Gated on `HTUI_TEST_INFISICAL_URL`;
//! unset, it prints the skip line and passes (as `htui-store/tests/qdrant_live.rs`). With the URL
//! set, the other four variables are required, and a missing one panics by name.
//!
//! Use a throwaway Universal Auth identity with access to one test environment. It needs read
//! access, and write access too if the environment's root folder is empty: the test then seeds
//! one placeholder secret (`SEED_KEY`) and leaves it there for the next run.
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

/// The placeholder secret the test creates when the root folder is empty.
const SEED_KEY: &str = "HTUI_TEST_SEED";

/// Creates `SEED_KEY` in the scope's root folder. A 4xx that says the key already exists is
/// fine (a concurrent run seeded it). Any other failure panics with the step and the HTTP status
/// only: no body, no token (H-8).
async fn seed(url: &str, client_id: &str, client_secret: &str, project: &str, environment: &str) {
    let base = url.trim_end_matches('/');
    let http = reqwest::Client::new();
    let login = http
        .post(format!("{base}/api/v1/auth/universal-auth/login"))
        .json(&serde_json::json!({ "clientId": client_id, "clientSecret": client_secret }))
        .send()
        .await
        .unwrap_or_else(|_| panic!("seeding: the login request did not complete"));
    assert!(
        login.status().is_success(),
        "seeding: login answered {}",
        login.status()
    );
    let body: serde_json::Value = login
        .json()
        .await
        .unwrap_or_else(|_| panic!("seeding: the login answer was not JSON"));
    let token = body["accessToken"]
        .as_str()
        .unwrap_or_else(|| panic!("seeding: the login answer had no accessToken"));

    let created = http
        .post(format!("{base}/api/v3/secrets/raw/{SEED_KEY}"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "workspaceId": project,
            "environment": environment,
            "secretPath": "/",
            "secretValue": "seed",
            "type": "shared",
        }))
        .send()
        .await
        .unwrap_or_else(|_| panic!("seeding: the create request did not complete"));
    let status = created.status();
    // Infisical answers 400 when the key exists already.
    assert!(
        status.is_success() || status == reqwest::StatusCode::BAD_REQUEST,
        "seeding: creating the placeholder secret answered {status}; the identity needs write \
         access to the environment, or add one secret by hand"
    );
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
        SecretError::LoginCoolingDown { .. } => "LoginCoolingDown",
        SecretError::ProjectNotFound => "ProjectNotFound",
        SecretError::PathNotFound { .. } => "PathNotFound",
        SecretError::PermissionDenied { .. } => "PermissionDenied",
        SecretError::RateLimited { .. } => "RateLimited",
        SecretError::UnsupportedServer { .. } => "UnsupportedServer",
        SecretError::InvalidKey { .. } => "InvalidKey",
        SecretError::InvalidValue { .. } => "InvalidValue",
        // Raised by `htui_core::secret::check_env`, never by the provider.
        SecretError::ReservedKey { .. } => "ReservedKey",
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
    let client_id = required(ENV_CLIENT_ID);
    let client_secret = required(ENV_CLIENT_SECRET);
    let project_id = required(ENV_PROJECT_ID);
    let environment = required(ENV_ENVIRONMENT);
    let provider_url = url.clone();
    let identity = MachineIdentity::new(client_id.clone(), client_secret.clone());
    let scope = must(
        SecretScope::new(project_id.clone(), environment.clone(), "/"),
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

    let mut resolved = must(provider.resolve(&scope).await, "resolve");
    if resolved.is_empty() {
        seed(
            provider_url.as_str(),
            &client_id,
            &client_secret,
            &project_id,
            &environment,
        )
        .await;
        resolved = must(provider.resolve(&scope).await, "resolve after seeding");
    }
    assert!(
        !resolved.is_empty(),
        "the scope resolved to no secrets; seeding did not take effect"
    );

    // H-8: `assert!`, not `assert_eq!`, which would print both key lists on failure.
    let listed = must(provider.list_keys(&scope).await, "list_keys");
    assert!(
        listed == resolved.keys(),
        "list_keys and resolve disagree on the key names"
    );

    eprintln!("resolved {} keys", resolved.len());
}
