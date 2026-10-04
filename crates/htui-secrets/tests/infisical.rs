//! `InfisicalProvider` against a loopback stand-in for Infisical (MOD-10 M2, blueprint §D.3).
//! Never a real server: every base URL is the stub's or a closed loopback port.
//!
//! Sentinel rule (H-8): no assert message interpolates `CLIENT_SECRET`, `TOKEN`, `TOKEN_2`,
//! `VALUE` or an error that might carry one. Messages name the case only.

mod support;

use std::collections::BTreeMap;

use htui_core::secret::{MachineIdentity, SecretError, SecretProvider, SecretScope};
use htui_secrets::{InfisicalConfig, InfisicalProvider};
use serde_json::{Value, json};
use support::{Reply, Stub};

const CLIENT_ID: &str = "cid-sentinel-1";
const CLIENT_SECRET: &str = "csecret-sentinel-1";
const TOKEN: &str = "tok-sentinel-1";
const TOKEN_2: &str = "tok-sentinel-2";
const VALUE: &str = "value-sentinel-1";

const LOGIN: &str = "/api/v1/auth/universal-auth/login";
const SECRETS: &str = "/api/v4/secrets";

fn identity() -> MachineIdentity {
    MachineIdentity::new(CLIENT_ID, CLIENT_SECRET)
}

fn login_ok(token: &str, expires_in: u64) -> Reply {
    Reply::json(
        200,
        &json!({"accessToken": token, "expiresIn": expires_in, "accessTokenMaxTTL": 2_592_000,
                "tokenType": "Bearer"}),
    )
}

fn list_ok(secrets: Value, imports: Value) -> Reply {
    Reply::json(200, &json!({"secrets": secrets, "imports": imports}))
}

fn error(status: u16, name: &str, message: &str) -> Reply {
    Reply::json(
        status,
        &json!({"reqId": "req-1", "statusCode": status, "message": message, "error": name}),
    )
}

fn secret(key: &str, value: &str) -> Value {
    json!({"id": "s-1", "version": 1, "secretKey": key, "secretValue": value,
           "secretValueHidden": false, "type": "shared"})
}

fn hidden(key: &str) -> Value {
    json!({"secretKey": key, "secretValue": "<hidden-by-infisical>", "secretValueHidden": true,
           "type": "shared"})
}

fn personal(key: &str, value: &str) -> Value {
    json!({"secretKey": key, "secretValue": value, "secretValueHidden": false,
           "type": "personal"})
}

fn provider(stub: &Stub) -> InfisicalProvider {
    InfisicalProvider::new(InfisicalConfig::new(stub.base()), identity())
        .expect("a provider on the stub")
}

fn scope() -> SecretScope {
    SecretScope::new("proj-1", "dev", "/app").expect("a valid scope")
}

/// A stub whose login answers `TOKEN` for 30 days and whose list answers `secrets`, no imports.
fn serving(secrets: Value) -> Stub {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        list_ok(secrets, json!([])),
    );
    stub
}

/// The resolved map as owned pairs, for comparisons.
fn pairs(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn owned(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// `(method, path)` of every request so far.
fn routes(stub: &Stub) -> Vec<(String, String)> {
    stub.requests()
        .into_iter()
        .map(|r| (r.method, r.path))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Build and config
// ---------------------------------------------------------------------------------------------

/// Meaningful only under `cargo test -p htui-secrets`, whose dependency graph has no
/// `sentry`/aws-lc (H-1): there, `Client::build` panics unless this crate installed `ring`
/// first. A workspace-wide build gets a provider from elsewhere and proves nothing.
#[test]
fn a_provider_builds_with_only_this_crate_installing_ring() {
    for _ in 0..2 {
        InfisicalProvider::new(
            InfisicalConfig::new("https://app.infisical.com"),
            identity(),
        )
        .expect("the provider builds with ring installed by htui-secrets");
    }
}

#[test]
fn an_http_base_url_on_a_lan_host_is_refused_before_any_request() {
    match InfisicalProvider::new(InfisicalConfig::new("http://192.168.1.10"), identity()) {
        Err(SecretError::Config(why)) => assert!(why.contains("https"), "{why}"),
        Err(other) => panic!("expected Config, got {other}"),
        Ok(_) => panic!("a plain-text LAN URL was accepted"),
    }
}

#[test]
fn a_blank_identity_half_is_refused_at_build() {
    let cases = [
        (
            MachineIdentity::new("  ", CLIENT_SECRET),
            "the machine identity's client ID is empty",
        ),
        (
            MachineIdentity::new(CLIENT_ID, "  "),
            "the machine identity's client secret is empty",
        ),
    ];
    for (id, sentence) in cases {
        match InfisicalProvider::new(InfisicalConfig::new("https://app.infisical.com"), id) {
            Err(SecretError::Config(why)) => assert_eq!(why, sentence),
            Err(_) => panic!("expected Config for {sentence:?}"),
            Ok(_) => panic!("a blank half was accepted ({sentence})"),
        }
    }
}

#[tokio::test]
async fn the_base_url_is_normalised_before_use() {
    let stub = serving(json!([secret("A", VALUE)]));
    let p = InfisicalProvider::new(
        InfisicalConfig::new(format!("{}/api/", stub.base())),
        identity(),
    )
    .expect("a provider on the stub");
    p.resolve(&scope()).await.expect("resolves");
    assert_eq!(p.base_url(), stub.base());
    let first = &stub.requests()[0];
    assert_eq!(first.path, LOGIN);
}

#[test]
fn kind_is_infisical() {
    let p = InfisicalProvider::new(
        InfisicalConfig::new("https://app.infisical.com"),
        identity(),
    )
    .expect("builds");
    assert_eq!(p.kind(), "infisical");
    assert_eq!(InfisicalProvider::KIND, "infisical");
}

// ---------------------------------------------------------------------------------------------
// Login + list
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn resolve_logs_in_then_lists_the_scope() {
    let stub = serving(json!([secret("B", VALUE), secret("A", "plain")]));
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    let seen = stub.requests();
    assert_eq!(
        routes(&stub),
        [
            ("POST".to_owned(), LOGIN.to_owned()),
            ("GET".to_owned(), SECRETS.to_owned())
        ]
    );
    let login = &seen[0];
    assert!(
        login.json() == json!({"clientId": CLIENT_ID, "clientSecret": CLIENT_SECRET}),
        "the login body is not the identity in camelCase"
    );
    assert!(
        login
            .header("Content-Type")
            .is_some_and(|v| v.contains("application/json")),
        "the login is not sent as JSON"
    );
    let list = &seen[1];
    assert!(
        list.header("authorization") == Some(format!("Bearer {TOKEN}").as_str()),
        "the list does not carry the login's token as a bearer"
    );
    for request in &seen {
        assert!(
            request
                .header("user-agent")
                .is_some_and(|v| v.starts_with("htui/")),
            "no htui user agent on {}",
            request.path
        );
    }
    assert!(
        pairs(resolved.as_map()) == owned(&[("A", "plain"), ("B", VALUE)]),
        "the resolved map is not the scripted one"
    );
}

#[tokio::test]
async fn the_list_query_carries_every_flag_and_both_imports_spellings() {
    let stub = serving(json!([]));
    provider(&stub).resolve(&scope()).await.expect("resolves");
    let list = &stub.requests()[1];
    let want = [
        ("projectId", "proj-1"),
        ("environment", "dev"),
        ("secretPath", "/app"),
        ("viewSecretValue", "true"),
        ("expandSecretReferences", "true"),
        ("includeImports", "true"),
        ("include_imports", "true"),
        ("recursive", "false"),
        ("includePersonalOverrides", "false"),
    ];
    for (key, value) in want {
        assert_eq!(list.query(key), [value], "query key {key}");
    }
    let keys: Vec<&str> = list.query.iter().map(|(k, _)| k.as_str()).collect();
    let want_keys: Vec<&str> = want.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, want_keys, "the query order");
}

#[tokio::test]
async fn list_keys_returns_sorted_names_matching_resolve() {
    let stub = serving(json!([
        secret("ZED", VALUE),
        secret("ALPHA", VALUE),
        secret("MID", VALUE)
    ]));
    let p = provider(&stub);
    let keys = p.list_keys(&scope()).await.expect("lists");
    assert_eq!(keys, ["ALPHA", "MID", "ZED"]);
    let resolved = p.resolve(&scope()).await.expect("resolves");
    assert_eq!(keys, resolved.keys());
}

// ---------------------------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_folder_beats_every_import() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        list_ok(
            json!([secret("A", "folder")]),
            json!([{"secrets": [secret("A", "import-0")]}, {"secrets": [secret("A", "import-1")]}]),
        ),
    );
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    assert_eq!(pairs(resolved.as_map()), owned(&[("A", "folder")]));
}

#[tokio::test]
async fn the_last_import_beats_earlier_ones() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        list_ok(
            json!([secret("F", "folder")]),
            json!([
                {"secrets": [secret("C", "i0c"), secret("D", "i0d")]},
                {"secrets": [secret("C", "i1c")]},
            ]),
        ),
    );
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    assert_eq!(
        pairs(resolved.as_map()),
        owned(&[("C", "i1c"), ("D", "i0d"), ("F", "folder")])
    );
}

#[tokio::test]
async fn personal_entries_are_ignored() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        list_ok(
            json!([
                personal("A", "mine"),
                secret("B", "shared"),
                personal("P", "p1")
            ]),
            json!([{"secrets": [secret("A", "imported"), personal("E", "e1")]}]),
        ),
    );
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    assert_eq!(
        pairs(resolved.as_map()),
        owned(&[("A", "imported"), ("B", "shared")])
    );
}

#[tokio::test]
async fn values_pass_through_byte_for_byte() {
    let rows = [
        ("SPACE", "a b\n"),
        ("LEAD", "  x"),
        ("UNI", "é✓"),
        ("EMPTY", ""),
    ];
    let stub = serving(Value::Array(
        rows.iter().map(|(k, v)| secret(k, v)).collect(),
    ));
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    let mut want = owned(&rows);
    want.sort();
    assert_eq!(pairs(resolved.as_map()), want);
}

#[tokio::test]
async fn an_absent_or_null_imports_field_is_no_imports() {
    for body in [
        json!({"secrets": [secret("A", "x")]}),
        json!({"secrets": [secret("A", "x")], "imports": null}),
    ] {
        let stub = Stub::start();
        stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
            "GET",
            SECRETS,
            Reply::json(200, &body),
        );
        let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
        assert_eq!(pairs(resolved.as_map()), owned(&[("A", "x")]));
    }
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_hidden_value_is_refused_naming_the_key() {
    let stub = serving(json!([secret("OPEN", VALUE), hidden("HIDDEN_KEY")]));
    match provider(&stub).resolve(&scope()).await {
        Err(SecretError::PermissionDenied { detail }) => {
            assert!(
                detail.contains("HIDDEN_KEY"),
                "the refusal does not name the key"
            );
            assert!(!detail.contains(VALUE), "the refusal carries a value");
        }
        Err(_) => panic!("expected PermissionDenied, got another variant"),
        Ok(_) => panic!("a hidden value was accepted"),
    }
}

#[tokio::test]
async fn a_hidden_import_value_shadowed_by_the_folder_is_not_a_refusal() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        list_ok(
            json!([secret("K", "folder")]),
            json!([{"secrets": [hidden("K")]}]),
        ),
    );
    let resolved = provider(&stub).resolve(&scope()).await.expect("resolves");
    assert_eq!(pairs(resolved.as_map()), owned(&[("K", "folder")]));
}

#[tokio::test]
async fn a_bad_key_name_is_refused_naming_it() {
    let stub = serving(json!([secret("my-key", VALUE)]));
    let err = provider(&stub).resolve(&scope()).await.unwrap_err();
    assert_eq!(
        err,
        SecretError::InvalidKey {
            key: "my-key".into()
        }
    );
}

#[tokio::test]
async fn a_nul_in_a_value_is_refused_naming_the_key() {
    let stub = serving(json!([secret("NUL_KEY", &format!("{VALUE}a\u{0}b"))]));
    let err = provider(&stub).resolve(&scope()).await.unwrap_err();
    assert!(
        err == SecretError::InvalidValue {
            key: "NUL_KEY".into()
        },
        "expected InvalidValue for NUL_KEY"
    );
    assert!(
        !err.to_string().contains(VALUE),
        "the refusal carries the value"
    );
}

#[tokio::test]
async fn hidden_is_reported_before_a_bad_name_before_a_nul() {
    let stub = serving(json!([
        secret("1A", "x"),
        hidden("Z_HIDDEN"),
        secret("B", "a\u{0}")
    ]));
    let err = provider(&stub).resolve(&scope()).await.unwrap_err();
    assert!(
        matches!(err, SecretError::PermissionDenied { .. }),
        "hidden was not reported first"
    );
    let stub = serving(json!([secret("B", "a\u{0}"), secret("Z-BAD", "x")]));
    let err = provider(&stub).resolve(&scope()).await.unwrap_err();
    assert_eq!(
        err,
        SecretError::InvalidKey {
            key: "Z-BAD".into()
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Login refusal and lockout (D5)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_refused_login_is_bad_credentials_and_never_retried() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(401, "UnauthorizedError", "Invalid credentials"),
    );
    let p = provider(&stub);
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::BadCredentials
    );
    let before = stub.requests().len();
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::LoginRefusedEarlier
    );
    assert_eq!(
        p.list_keys(&scope()).await.unwrap_err(),
        SecretError::LoginRefusedEarlier
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
    assert_eq!(stub.count("POST", LOGIN), 1);
}

#[tokio::test]
async fn the_lockout_text_is_identity_locked_and_latches() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(
            401,
            "UnauthorizedError",
            "Identity is Temporarily Locked due to too many failed login attempts",
        ),
    );
    let p = provider(&stub);
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::IdentityLocked
    );
    let before = stub.requests().len();
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::LoginRefusedEarlier
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
}

#[tokio::test]
async fn health_after_a_refused_login_makes_no_request() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(401, "UnauthorizedError", "Invalid credentials"),
    );
    let p = provider(&stub);
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::BadCredentials
    );
    let before = stub.requests().len();
    assert_eq!(
        p.health().await.unwrap_err(),
        SecretError::LoginRefusedEarlier
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "health on a latched provider made a request"
    );
}

#[tokio::test]
async fn concurrent_first_calls_share_one_login() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(401, "UnauthorizedError", "Invalid credentials"),
    );
    let (p, s) = (provider(&stub), scope());
    let (a, b) = tokio::join!(p.resolve(&s), p.resolve(&s));
    let mut errors = [a.unwrap_err().to_string(), b.unwrap_err().to_string()];
    errors.sort();
    let mut want = [
        SecretError::BadCredentials.to_string(),
        SecretError::LoginRefusedEarlier.to_string(),
    ];
    want.sort();
    assert_eq!(errors, want);
    assert_eq!(stub.count("POST", LOGIN), 1);
}

#[tokio::test]
async fn concurrent_first_calls_with_a_good_login_share_one_login() {
    let stub = serving(json!([secret("A", VALUE)]));
    let (p, s) = (provider(&stub), scope());
    let (a, b) = tokio::join!(p.resolve(&s), p.resolve(&s));
    assert!(a.is_ok() && b.is_ok(), "both resolves succeed");
    assert_eq!(stub.count("POST", LOGIN), 1);
    assert_eq!(stub.count("GET", SECRETS), 2);
}

// ---------------------------------------------------------------------------------------------
// Token (D5)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_live_token_is_reused_across_calls() {
    let stub = serving(json!([secret("A", VALUE)]));
    let p = provider(&stub);
    p.resolve(&scope()).await.expect("resolves");
    p.list_keys(&scope()).await.expect("lists");
    p.resolve(&scope()).await.expect("resolves again");
    assert_eq!(stub.count("POST", LOGIN), 1);
    assert_eq!(stub.count("GET", SECRETS), 3);
}

#[tokio::test]
async fn a_token_inside_the_margin_is_replaced() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 30)).on(
        "GET",
        SECRETS,
        list_ok(json!([secret("A", VALUE)]), json!([])),
    );
    let p = provider(&stub);
    p.resolve(&scope()).await.expect("resolves");
    p.resolve(&scope()).await.expect("resolves again");
    assert_eq!(stub.count("POST", LOGIN), 2);
}

/// Logins `[TOKEN, TOKEN_2]`, the first list answer `first`, then a good one.
async fn relogin_after(first: Reply) {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "POST",
        LOGIN,
        login_ok(TOKEN_2, 2_592_000),
    );
    stub.on("GET", SECRETS, first).on(
        "GET",
        SECRETS,
        list_ok(json!([secret("A", VALUE)]), json!([])),
    );
    let resolved = provider(&stub)
        .resolve(&scope())
        .await
        .expect("resolves after a re-login");
    assert_eq!(resolved.keys(), ["A"]);
    assert_eq!(stub.count("POST", LOGIN), 2);
    let gets: Vec<_> = stub
        .requests()
        .into_iter()
        .filter(|r| r.path == SECRETS)
        .collect();
    assert_eq!(gets.len(), 2);
    assert!(
        gets[1].header("authorization") == Some(format!("Bearer {TOKEN_2}").as_str()),
        "the retried list does not carry the new token"
    );
}

#[tokio::test]
async fn a_token_error_403_logs_in_once_more_then_succeeds() {
    relogin_after(error(403, "TokenError", "Token expired")).await;
}

#[tokio::test]
async fn a_data_401_logs_in_once_more_then_succeeds() {
    relogin_after(error(401, "UnauthorizedError", "Invalid token")).await;
}

#[tokio::test]
async fn a_token_refused_twice_is_protocol_after_one_relogin() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        error(403, "TokenError", "Token expired"),
    );
    let err = provider(&stub).resolve(&scope()).await.unwrap_err();
    assert_eq!(
        err,
        SecretError::Protocol {
            endpoint: SECRETS,
            detail: "the access token was refused right after a fresh login".into()
        }
    );
    assert_eq!(stub.count("POST", LOGIN), 2);
    assert_eq!(stub.count("GET", SECRETS), 2);
}

#[tokio::test]
async fn a_refused_relogin_latches() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "POST",
        LOGIN,
        error(401, "UnauthorizedError", "Invalid credentials"),
    );
    stub.on("GET", SECRETS, error(403, "TokenError", "Token expired"));
    let p = provider(&stub);
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::BadCredentials
    );
    let before = stub.requests().len();
    assert_eq!(
        p.resolve(&scope()).await.unwrap_err(),
        SecretError::LoginRefusedEarlier
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
}
