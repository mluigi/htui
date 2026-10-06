//! `InfisicalProvider` against a loopback stand-in for Infisical (MOD-10 M2, blueprint §D.3).
//! Never a real server: every base URL is the stub's or a closed loopback port.
//!
//! Sentinel rule (H-8): no assert message interpolates `CLIENT_SECRET`, `TOKEN`, `TOKEN_2`,
//! `VALUE` or an error that might carry one. Messages name the case only.
//! So errors are compared with `assert_err` and unwrapped with `Quiet::must`/`must_fail`, which
//! name the variant, never with `assert_eq!`, `expect` or `unwrap_err`, which print it.

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use htui_core::secret::{MachineIdentity, SecretError, SecretProvider, SecretScope};
use htui_secrets::{InfisicalConfig, InfisicalProvider};
use serde_json::{Value, json};
use support::{ClosedPort, Reply, Stub};

const CLIENT_ID: &str = "cid-sentinel-1";
const CLIENT_SECRET: &str = "csecret-sentinel-1";
const TOKEN: &str = "tok-sentinel-1";
const TOKEN_2: &str = "tok-sentinel-2";
const VALUE: &str = "value-sentinel-1";

const LOGIN: &str = "/api/v1/auth/universal-auth/login";
const SECRETS: &str = "/api/v4/secrets";
const STATUS: &str = "/api/status";

/// H-8: `expect` and `unwrap_err` print the other side of the `Result` on failure, which for a
/// `SecretError` could be a leak. These name the case and the error variant only.
trait Quiet<T> {
    fn must(self, case: &str) -> T;
    fn must_fail(self, case: &str) -> SecretError;
}

impl<T> Quiet<T> for Result<T, SecretError> {
    #[track_caller]
    fn must(self, case: &str) -> T {
        match self {
            Ok(v) => v,
            Err(e) => panic!("{case}: failed with {}", variant(&e)),
        }
    }

    #[track_caller]
    fn must_fail(self, case: &str) -> SecretError {
        match self {
            Ok(_) => panic!("{case}: succeeded"),
            Err(e) => e,
        }
    }
}

/// H-8: `assert_eq!` on errors prints both on failure; this names the two variants only.
#[track_caller]
fn assert_err(got: &SecretError, want: &SecretError) {
    assert!(
        got == want,
        "wrong error: got {} where {} was expected",
        variant(got),
        variant(want)
    );
}

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

/// Fastify's default 404 for an unknown route.
fn fastify_404(method: &str, path: &str) -> Reply {
    Reply::json(
        404,
        &json!({"message": format!("Route {method}:{path} not found"), "error": "Not Found",
                "statusCode": 404}),
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
        .must("a provider on the stub")
}

fn scope() -> SecretScope {
    SecretScope::new("proj-1", "dev", "/app").must("a valid scope")
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
        .must("the provider builds with ring installed by htui-secrets");
    }
}

#[test]
fn an_http_base_url_on_a_lan_host_is_refused_before_any_request() {
    match InfisicalProvider::new(InfisicalConfig::new("http://192.168.1.10"), identity()) {
        Err(SecretError::Config(why)) => assert!(why.contains("https"), "the reason omits https"),
        Err(_) => panic!("expected Config"),
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
            Err(SecretError::Config(why)) => {
                assert!(why == sentence, "wrong Config reason for {sentence:?}")
            }
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
    .must("a provider on the stub");
    p.resolve(&scope()).await.must("resolves");
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
    .must("builds");
    assert_eq!(p.kind(), "infisical");
    assert_eq!(InfisicalProvider::KIND, "infisical");
}

// ---------------------------------------------------------------------------------------------
// Login + list
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn resolve_logs_in_then_lists_the_scope() {
    let stub = serving(json!([secret("B", VALUE), secret("A", "plain")]));
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
    provider(&stub).resolve(&scope()).await.must("resolves");
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
    let keys = p.list_keys(&scope()).await.must("lists");
    assert_eq!(keys, ["ALPHA", "MID", "ZED"]);
    let resolved = p.resolve(&scope()).await.must("resolves");
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
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
        let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
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
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
    assert_eq!(pairs(resolved.as_map()), owned(&[("K", "folder")]));
}

#[tokio::test]
async fn a_bad_key_name_is_refused_naming_it() {
    let stub = serving(json!([secret("my-key", VALUE)]));
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert!(
        err == SecretError::InvalidKey {
            key: "my-key".into()
        },
        "expected InvalidKey for my-key"
    );
}

#[tokio::test]
async fn a_nul_in_a_value_is_refused_naming_the_key() {
    let stub = serving(json!([secret("NUL_KEY", &format!("{VALUE}a\u{0}b"))]));
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
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
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert!(
        matches!(err, SecretError::PermissionDenied { .. }),
        "hidden was not reported first"
    );
    let stub = serving(json!([secret("B", "a\u{0}"), secret("Z-BAD", "x")]));
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert_err(
        &err,
        &SecretError::InvalidKey {
            key: "Z-BAD".into(),
        },
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
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::BadCredentials,
    );
    let before = stub.requests().len();
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
    );
    assert_err(
        &p.list_keys(&scope()).await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
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
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::IdentityLocked,
    );
    let before = stub.requests().len();
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
}

/// D5 rests on the status alone: a 401 whose body is cut short is still a refusal and latches,
/// so a truncated answer can never turn a rejected login into a retried one. An unread body
/// cannot prove the lockout text, so the refusal is `BadCredentials`.
#[tokio::test]
async fn a_refused_login_with_a_cut_body_still_latches() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(
            401,
            "UnauthorizedError",
            "Identity is Temporarily Locked due to too many failed login attempts",
        )
        .truncated(4096),
    );
    let p = provider(&stub);
    assert!(
        matches!(p.resolve(&scope()).await, Err(SecretError::BadCredentials)),
        "a cut 401 login body is not BadCredentials"
    );
    let before = stub.requests().len();
    assert!(
        matches!(
            p.resolve(&scope()).await,
            Err(SecretError::LoginRefusedEarlier)
        ),
        "a cut 401 login body did not latch"
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
    assert_eq!(stub.count("POST", LOGIN), 1);
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
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::BadCredentials,
    );
    let before = stub.requests().len();
    assert_err(
        &p.health().await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
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
    let mut errors = [
        a.must_fail("expected an error").to_string(),
        b.must_fail("expected an error").to_string(),
    ];
    errors.sort();
    let mut want = [
        SecretError::BadCredentials.to_string(),
        SecretError::LoginRefusedEarlier.to_string(),
    ];
    want.sort();
    assert!(
        errors == want,
        "the two calls are not one BadCredentials and one LoginRefusedEarlier"
    );
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
// Cancel safety and the cool-down (D5, R1 #1)
// ---------------------------------------------------------------------------------------------

/// How long the stub sits on a login in the cancel and cool-down tests.
const SLOW: Duration = Duration::from_millis(300);
/// A caller's patience, or the request timeout, below `SLOW`.
const IMPATIENT: Duration = Duration::from_millis(100);

/// A provider on `stub` whose requests time out after `IMPATIENT` and whose cool-down after a
/// login without an answer lasts `cool_down`.
fn impatient(stub: &Stub, cool_down: Duration) -> InfisicalProvider {
    let mut config = InfisicalConfig::new(stub.base());
    config.timeout = IMPATIENT;
    config.login_cool_down = cool_down;
    InfisicalProvider::new(config, identity()).must("an impatient provider on the stub")
}

/// The reviewer's reproduction: three callers that give up (`tokio::time::timeout`) before a slow
/// 401 arrives. The login runs on its own task, so the dropped callers never lose its outcome:
/// one login, and the refusal still latches.
#[tokio::test]
async fn cancelled_callers_around_a_slow_refusal_cost_one_login() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(401, "UnauthorizedError", "Invalid credentials").delayed(SLOW),
    );
    let p = provider(&stub);
    // The first two give up for sure; the third may already see the latch as the 401 lands.
    for _ in 0..3 {
        let _ = tokio::time::timeout(IMPATIENT, p.resolve(&scope())).await;
    }
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
    );
    assert_eq!(
        stub.count("POST", LOGIN),
        1,
        "a cancelled caller cost a login"
    );
}

/// A caller that gives up during a good login does not waste it: the token is cached.
#[tokio::test]
async fn a_cancelled_caller_s_login_still_caches_the_token() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000).delayed(SLOW))
        .on(
            "GET",
            SECRETS,
            list_ok(json!([secret("A", VALUE)]), json!([])),
        );
    let p = provider(&stub);
    let gave_up = tokio::time::timeout(IMPATIENT, p.resolve(&scope())).await;
    assert!(gave_up.is_err(), "the caller did not give up");
    p.resolve(&scope())
        .await
        .must("resolves with the cached token");
    assert_eq!(stub.count("POST", LOGIN), 1);
    assert_eq!(stub.count("GET", SECRETS), 1);
}

/// A login sent but unanswered (the request timeout) may have counted as a failed attempt: the
/// caller sees `Unreachable`; for the cool-down every call, health included, is
/// `LoginCoolingDown` with no request; after it, concurrent callers share exactly one new login.
#[tokio::test]
async fn a_login_without_an_answer_cools_down_then_allows_one_attempt() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000).delayed(SLOW))
        .on("POST", LOGIN, login_ok(TOKEN_2, 2_592_000))
        .on(
            "GET",
            SECRETS,
            list_ok(json!([secret("A", VALUE)]), json!([])),
        )
        .on("GET", STATUS, Reply::json(200, &json!({})));
    let cool_down = Duration::from_millis(600);
    let p = impatient(&stub, cool_down);
    assert!(
        matches!(
            p.resolve(&scope()).await,
            Err(SecretError::Unreachable { endpoint, .. }) if endpoint == LOGIN
        ),
        "an unanswered login is not Unreachable at the login"
    );
    let cooling = SecretError::LoginCoolingDown {
        retry_after_secs: 1,
    };
    assert_err(
        &p.resolve(&scope())
            .await
            .must_fail("resolve while cooling down"),
        &cooling,
    );
    assert_err(
        &p.list_keys(&scope())
            .await
            .must_fail("list_keys while cooling down"),
        &cooling,
    );
    assert_err(
        &p.health().await.must_fail("health while cooling down"),
        &cooling,
    );
    assert_eq!(
        stub.requests().len(),
        1,
        "a cooling-down provider made a request"
    );
    tokio::time::sleep(cool_down + Duration::from_millis(200)).await;
    let s = scope();
    let (a, b) = tokio::join!(p.resolve(&s), p.resolve(&s));
    a.must("the first call after the cool-down");
    b.must("the second call after the cool-down");
    assert_eq!(
        stub.count("POST", LOGIN),
        2,
        "not exactly one login after the cool-down"
    );
}

/// The default cool-down is Infisical's lockout counter-reset window.
#[test]
fn the_default_cool_down_is_thirty_seconds() {
    assert_eq!(
        InfisicalConfig::new("https://app.infisical.com").login_cool_down,
        Duration::from_secs(30)
    );
    assert_eq!(
        htui_secrets::DEFAULT_LOGIN_COOL_DOWN,
        Duration::from_secs(30)
    );
}

// ---------------------------------------------------------------------------------------------
// Token (D5)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_live_token_is_reused_across_calls() {
    let stub = serving(json!([secret("A", VALUE)]));
    let p = provider(&stub);
    p.resolve(&scope()).await.must("resolves");
    p.list_keys(&scope()).await.must("lists");
    p.resolve(&scope()).await.must("resolves again");
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
    p.resolve(&scope()).await.must("resolves");
    p.resolve(&scope()).await.must("resolves again");
    assert_eq!(stub.count("POST", LOGIN), 2);
}

/// `Instant + Duration` panics on overflow: an absurd `expiresIn` must be an ordinary login whose
/// token is reused, never a panic in the caller's task.
#[tokio::test]
async fn a_huge_expires_in_is_reused_and_never_panics() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, u64::MAX)).on(
        "GET",
        SECRETS,
        list_ok(json!([secret("A", VALUE)]), json!([])),
    );
    let p = provider(&stub);
    p.resolve(&scope()).await.must("resolves");
    p.resolve(&scope()).await.must("resolves again");
    assert_eq!(stub.count("POST", LOGIN), 1);
    assert_eq!(stub.count("GET", SECRETS), 2);
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
        .must("resolves after a re-login");
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

/// A data 401 refuses the token whatever its body: a cut body still earns the one re-login,
/// instead of an `Unreachable` that keeps the refused token cached.
#[tokio::test]
async fn a_data_401_with_a_cut_body_logs_in_once_more_then_succeeds() {
    relogin_after(error(401, "UnauthorizedError", "Invalid token").truncated(4096)).await;
}

#[tokio::test]
async fn a_token_refused_twice_is_protocol_after_one_relogin() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000)).on(
        "GET",
        SECRETS,
        error(403, "TokenError", "Token expired"),
    );
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert_err(
        &err,
        &SecretError::Protocol {
            endpoint: SECRETS,
            detail: "the access token was refused right after a fresh login".into(),
        },
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
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::BadCredentials,
    );
    let before = stub.requests().len();
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::LoginRefusedEarlier,
    );
    assert_eq!(
        stub.requests().len(),
        before,
        "a latched provider made a request"
    );
}

// ---------------------------------------------------------------------------------------------
// Login errors
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_login_429_is_rate_limited_and_does_not_latch() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(429, "RateLimitExceeded", "slow down").header("Retry-After", "12"),
    )
    .on("POST", LOGIN, login_ok(TOKEN, 2_592_000))
    .on(
        "GET",
        SECRETS,
        list_ok(json!([secret("A", VALUE)]), json!([])),
    );
    let p = provider(&stub);
    assert_err(
        &p.resolve(&scope()).await.must_fail("expected an error"),
        &SecretError::RateLimited {
            retry_after_secs: Some(12),
        },
    );
    p.resolve(&scope()).await.must("the next call logs in");
    assert_eq!(stub.count("POST", LOGIN), 2);
}

#[tokio::test]
async fn a_login_network_failure_is_unreachable_and_does_not_latch() {
    let closed = ClosedPort::new();
    let p = InfisicalProvider::new(InfisicalConfig::new(closed.base()), identity())
        .must("a provider on a closed port");
    for _ in 0..2 {
        match p.resolve(&scope()).await {
            Err(SecretError::Unreachable { endpoint, cause }) => {
                assert_eq!(endpoint, LOGIN);
                assert!(!cause.is_empty(), "an empty cause");
                assert!(
                    !cause.contains("http://") && !cause.contains("/api/"),
                    "the cause carries the URL"
                );
            }
            Err(_) => panic!("expected Unreachable"),
            Ok(_) => panic!("a closed port resolved"),
        }
    }
}

#[tokio::test]
async fn a_login_route_404_is_unsupported_server() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, fastify_404("POST", LOGIN));
    let p = provider(&stub);
    for _ in 0..2 {
        assert_err(
            &p.resolve(&scope()).await.must_fail("expected an error"),
            &SecretError::UnsupportedServer { endpoint: LOGIN },
        );
    }
    assert_eq!(
        stub.count("POST", LOGIN),
        2,
        "an unsupported server latched"
    );
}

#[tokio::test]
async fn a_login_500_is_protocol_and_never_quotes_the_body() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        error(
            500,
            "InternalServerError",
            &format!("{CLIENT_SECRET} {TOKEN}"),
        ),
    );
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    let text = format!("{err} {err:?}");
    assert!(
        !text.contains(CLIENT_SECRET),
        "the login error quotes the client secret"
    );
    assert!(!text.contains(TOKEN), "the login error quotes the token");
    assert!(
        err == SecretError::Protocol {
            endpoint: LOGIN,
            detail: "status 500".into()
        },
        "expected Protocol status 500 at the login"
    );
}

// ---------------------------------------------------------------------------------------------
// Data errors
// ---------------------------------------------------------------------------------------------

/// A stub with a good login whose list answers `reply`; returns the first resolve's error.
async fn list_error(reply: Reply) -> SecretError {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000))
        .on("GET", SECRETS, reply);
    match provider(&stub).resolve(&scope()).await {
        Err(e) => e,
        Ok(_) => panic!("the list answer was accepted"),
    }
}

#[tokio::test]
async fn a_fastify_route_404_is_unsupported_server() {
    let err = list_error(fastify_404("GET", "/api/v4/secrets?projectId=proj-1")).await;
    assert_err(&err, &SecretError::UnsupportedServer { endpoint: SECRETS });
    assert!(err.to_string().contains("v0.150"));
}

#[tokio::test]
async fn a_not_found_404_is_project_not_found() {
    let err = list_error(error(404, "NotFound", "Project with ID proj-1 not found")).await;
    assert_err(&err, &SecretError::ProjectNotFound);
}

#[tokio::test]
async fn a_secret_path_404_is_path_not_found_naming_the_scope() {
    let err = list_error(error(404, "SecretPathNotFound", "Folder not found")).await;
    assert_err(
        &err,
        &SecretError::PathNotFound {
            environment: "dev".into(),
            path: "/app".into(),
        },
    );
}

#[tokio::test]
async fn a_permission_denied_403_carries_the_cleaned_message() {
    let err = list_error(error(
        403,
        "PermissionDenied",
        "You are not allowed to\r\nread secrets in dev",
    ))
    .await;
    assert_err(
        &err,
        &SecretError::PermissionDenied {
            detail: "You are not allowed toread secrets in dev".into(),
        },
    );
    let err = list_error(error(403, "Forbidden", "")).await;
    assert_err(
        &err,
        &SecretError::PermissionDenied {
            detail: "status 403 Forbidden".into(),
        },
    );
}

#[tokio::test]
async fn a_membership_403_is_permission_denied() {
    let err = list_error(error(
        403,
        "ProjectMembershipNotFound",
        "Identity is not a member of the project",
    ))
    .await;
    assert_err(
        &err,
        &SecretError::PermissionDenied {
            detail: "Identity is not a member of the project".into(),
        },
    );
}

#[tokio::test]
async fn a_429_is_rate_limited_with_or_without_retry_after() {
    let err = list_error(error(429, "RateLimitExceeded", "slow").header("Retry-After", "30")).await;
    assert_err(
        &err,
        &SecretError::RateLimited {
            retry_after_secs: Some(30),
        },
    );
    let err = list_error(error(429, "RateLimitExceeded", "slow")).await;
    assert_err(
        &err,
        &SecretError::RateLimited {
            retry_after_secs: None,
        },
    );
}

#[tokio::test]
async fn a_long_server_message_is_cleaned_and_cut() {
    let message = format!("{}\r\n{}", "x".repeat(150), "y".repeat(150));
    let SecretError::PermissionDenied { detail } =
        list_error(error(403, "PermissionDenied", &message)).await
    else {
        panic!("expected PermissionDenied");
    };
    assert!(detail.chars().count() <= 200, "the detail is not cut");
    assert!(
        !detail.chars().any(char::is_control),
        "control characters kept"
    );
}

/// A-4 covers the `error` field too: with no `message`, the 403 detail quotes it, so a server
/// cannot reach the terminal with control characters or flood it through that field either.
#[tokio::test]
async fn a_403_error_name_is_cleaned_and_cut_too() {
    let name = format!("X\u{1b}[2J\r\nY{}", "z".repeat(300));
    let SecretError::PermissionDenied { detail } = list_error(error(403, &name, "")).await else {
        panic!("expected PermissionDenied");
    };
    assert!(
        detail.starts_with("status 403 X[2JYz"),
        "the name is not quoted"
    );
    assert!(
        !detail.chars().any(char::is_control),
        "control characters kept"
    );
    assert!(
        detail.chars().count() <= "status 403 ".len() + 200,
        "the name is not cut"
    );
}

#[tokio::test]
async fn an_unexpected_status_is_protocol_naming_the_endpoint() {
    let err = list_error(error(500, "InternalServerError", "boom")).await;
    assert_err(
        &err,
        &SecretError::Protocol {
            endpoint: SECRETS,
            detail: "status 500: boom".into(),
        },
    );
}

#[tokio::test]
async fn a_non_json_list_body_is_protocol() {
    match list_error(Reply::text(200, "<html>")).await {
        SecretError::Protocol { endpoint, detail } => {
            assert_eq!(endpoint, SECRETS);
            assert!(
                detail.contains("not the expected JSON"),
                "not the decode detail"
            );
        }
        _ => panic!("expected Protocol"),
    }
}

#[tokio::test]
async fn a_data_redirect_is_protocol_and_never_followed() {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login_ok(TOKEN, 2_592_000))
        .on(
            "GET",
            SECRETS,
            Reply::redirect(302, &format!("{}/elsewhere", stub.base())),
        )
        .on("GET", "/elsewhere", list_ok(json!([]), json!([])));
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert_err(
        &err,
        &SecretError::Protocol {
            endpoint: SECRETS,
            detail: "the server answered a redirect (302); redirects are not followed".into(),
        },
    );
    assert_eq!(stub.count("GET", "/elsewhere"), 0);
}

#[tokio::test]
async fn a_login_redirect_is_protocol_and_the_body_never_follows() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        Reply::redirect(307, &format!("{}/steal", stub.base())),
    )
    .on("POST", "/steal", login_ok(TOKEN, 2_592_000));
    let p = provider(&stub);
    for _ in 0..2 {
        assert!(
            p.resolve(&scope()).await.must_fail("expected an error")
                == SecretError::Protocol {
                    endpoint: LOGIN,
                    detail: "the server answered a redirect (307); redirects are not followed"
                        .into()
                },
            "expected the redirect Protocol at the login"
        );
    }
    assert_eq!(stub.count("POST", "/steal"), 0);
    assert_eq!(stub.count("POST", LOGIN), 2, "a login redirect latched");
}

#[tokio::test]
async fn an_empty_or_unusable_login_token_is_protocol_and_never_cached() {
    let cases = [
        ("", "the login answer carried no access token"),
        (
            "tok-sentinel-1\nX",
            "the login answer carried an access token that is not a valid header value",
        ),
        (
            "tok sentinel-1",
            "the login answer carried an access token that is not a valid header value",
        ),
    ];
    for (i, (token, detail)) in cases.into_iter().enumerate() {
        let stub = Stub::start();
        stub.on("POST", LOGIN, login_ok(token, 2_592_000)).on(
            "GET",
            SECRETS,
            list_ok(json!([]), json!([])),
        );
        let p = provider(&stub);
        for _ in 0..2 {
            let err = p.resolve(&scope()).await.must_fail("expected an error");
            assert!(
                matches!(&err, SecretError::Protocol { endpoint, detail: got }
                    if *endpoint == LOGIN && got == detail),
                "case {i}: wrong error variant or detail"
            );
            assert!(!format!("{err} {err:?}").contains(TOKEN), "case {i}: leak");
        }
        assert_eq!(
            stub.count("POST", LOGIN),
            2,
            "case {i}: the token was cached"
        );
        assert_eq!(
            stub.count("GET", SECRETS),
            0,
            "case {i}: a data call went out"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Body caps (R1 #5)
// ---------------------------------------------------------------------------------------------

/// The login and every error body.
const SMALL_CAP: usize = 64 * 1024;
/// A list answer.
const LIST_CAP: usize = 8 * 1024 * 1024;

/// `status` with a body of `len` bytes made of `TOKEN` and `VALUE`, with or without a declared
/// length. Never valid JSON; only its size matters.
fn oversized(status: u16, len: usize, declared: bool) -> Reply {
    let filler = format!("{TOKEN} {VALUE} ");
    let body: String = filler.chars().cycle().take(len).collect();
    let reply = Reply::text(status, &body);
    if declared {
        reply
    } else {
        reply.without_length()
    }
}

/// `err` is `Protocol` at `endpoint` naming `cap`, and quotes nothing of the body.
#[track_caller]
fn assert_too_large(err: &SecretError, endpoint: &str, cap: &str, case: &str) {
    assert!(
        matches!(err, SecretError::Protocol { endpoint: e, detail }
            if *e == endpoint && detail.contains(&format!("larger than the {cap} limit"))),
        "{case}: not Protocol at {endpoint} naming the {cap} cap"
    );
    let text = format!("{err} {err:?}");
    assert!(!text.contains(TOKEN), "{case}: the error quotes the body");
    assert!(!text.contains(VALUE), "{case}: the error quotes the body");
}

#[tokio::test]
async fn an_oversized_login_answer_is_protocol_naming_the_cap_and_never_latches() {
    for declared in [true, false] {
        let case = format!("declared length {declared}");
        let stub = Stub::start();
        stub.on("POST", LOGIN, oversized(200, SMALL_CAP + 1, declared));
        let p = provider(&stub);
        for _ in 0..2 {
            let err = p.resolve(&scope()).await.must_fail(&case);
            assert_too_large(&err, LOGIN, "64 KiB", &case);
        }
        assert_eq!(stub.count("POST", LOGIN), 2, "{case}: latched");
    }
}

#[tokio::test]
async fn an_oversized_list_answer_is_protocol_naming_the_cap() {
    for declared in [true, false] {
        let case = format!("declared length {declared}");
        let err = list_error(oversized(200, LIST_CAP + 1, declared)).await;
        assert_too_large(&err, SECRETS, "8 MiB", &case);
    }
}

#[tokio::test]
async fn an_oversized_list_error_body_is_protocol_naming_the_cap() {
    for declared in [true, false] {
        let case = format!("declared length {declared}");
        let err = list_error(oversized(500, SMALL_CAP + 1, declared)).await;
        assert_too_large(&err, SECRETS, "64 KiB", &case);
    }
}

/// The list cap is the larger one: a list answer above the error-body cap still resolves.
#[tokio::test]
async fn a_list_answer_above_the_error_cap_resolves() {
    let big = "v".repeat(SMALL_CAP);
    let stub = serving(json!([secret("BIG", &big)]));
    let resolved = provider(&stub).resolve(&scope()).await.must("resolves");
    assert_eq!(resolved.keys(), ["BIG"]);
}

/// d8f4a7ca: a 401 is decided on its status before the body, so an oversized refusal still
/// latches; a body never read in full cannot prove the lockout, so it is `BadCredentials`.
#[tokio::test]
async fn an_oversized_login_401_still_latches() {
    for declared in [true, false] {
        let case = format!("declared length {declared}");
        let stub = Stub::start();
        stub.on("POST", LOGIN, oversized(401, SMALL_CAP + 1, declared));
        let p = provider(&stub);
        assert_err(
            &p.resolve(&scope()).await.must_fail(&case),
            &SecretError::BadCredentials,
        );
        assert_err(
            &p.resolve(&scope()).await.must_fail(&case),
            &SecretError::LoginRefusedEarlier,
        );
        assert_eq!(stub.count("POST", LOGIN), 1, "{case}: not latched");
    }
}

// ---------------------------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn health_reads_the_status_then_logs_in() {
    let stub = Stub::start();
    stub.on("GET", STATUS, Reply::json(200, &json!({"message": "Ok"})))
        .on("POST", LOGIN, login_ok(TOKEN, 2_592_000));
    let health = provider(&stub).health().await.must("healthy");
    assert_eq!(
        routes(&stub),
        [
            ("GET".to_owned(), STATUS.to_owned()),
            ("POST".to_owned(), LOGIN.to_owned())
        ]
    );
    assert_eq!(health.base_url, stub.base());
    assert!(health.server_ok);
}

#[tokio::test]
async fn health_reports_a_failing_status_and_still_logs_in() {
    let stub = Stub::start();
    stub.on("GET", STATUS, error(503, "ServiceUnavailable", "down"))
        .on("POST", LOGIN, login_ok(TOKEN, 2_592_000));
    let health = provider(&stub).health().await.must("health answers");
    assert!(!health.server_ok);
    assert_eq!(stub.count("POST", LOGIN), 1);
}

#[tokio::test]
async fn health_always_logs_in_afresh() {
    let stub = serving(json!([secret("A", VALUE)]));
    stub.on("GET", STATUS, Reply::json(200, &json!({})));
    let p = provider(&stub);
    p.resolve(&scope()).await.must("resolves");
    p.health().await.must("healthy");
    assert_eq!(stub.count("POST", LOGIN), 2);
}

#[tokio::test]
async fn health_on_an_unreachable_server_names_the_status_endpoint() {
    let closed = ClosedPort::new();
    let p = InfisicalProvider::new(InfisicalConfig::new(closed.base()), identity())
        .must("a provider on a closed port");
    assert!(
        matches!(
            p.health().await,
            Err(SecretError::Unreachable { endpoint, .. }) if endpoint == STATUS
        ),
        "expected Unreachable at the status endpoint"
    );
}

// ---------------------------------------------------------------------------------------------
// Leak (D10)
// ---------------------------------------------------------------------------------------------

/// Exhaustive, no wildcard: a new variant fails to compile here and needs a scenario.
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

/// The first resolve's error on a stub with `login` and `list` scripted.
async fn scenario(login: Reply, list: Reply) -> SecretError {
    let stub = Stub::start();
    stub.on("POST", LOGIN, login).on("GET", SECRETS, list);
    match provider(&stub).resolve(&scope()).await {
        Err(e) => e,
        Ok(_) => panic!("a leak scenario resolved"),
    }
}

#[tokio::test]
async fn no_error_or_debug_carries_a_value_the_client_secret_or_the_token() {
    let good = || login_ok(TOKEN, 2_592_000);
    let unused = || list_ok(json!([]), json!([]));
    let leaky_login = |status: u16, message: &str| {
        error(
            status,
            "UnauthorizedError",
            &format!("{message} {CLIENT_SECRET} {TOKEN}"),
        )
    };
    let mut errors = vec![SecretError::NoIdentity];

    errors.push(
        InfisicalProvider::new(InfisicalConfig::new("http://192.168.1.10"), identity())
            .must_fail("a LAN http URL is refused"),
    );
    let closed = ClosedPort::new();
    let unreachable = InfisicalProvider::new(InfisicalConfig::new(closed.base()), identity())
        .must("a provider on a closed port");
    errors.push(
        unreachable
            .resolve(&scope())
            .await
            .must_fail("a closed port fails"),
    );

    let stub = Stub::start();
    stub.on("POST", LOGIN, leaky_login(401, "Invalid credentials"));
    let latched = provider(&stub);
    errors.push(latched.resolve(&scope()).await.must_fail("refused"));
    errors.push(latched.resolve(&scope()).await.must_fail("latched"));

    errors.push(scenario(leaky_login(401, "Identity is temporarily locked"), unused()).await);
    errors.push(scenario(leaky_login(500, "Internal error"), unused()).await);
    errors.push(scenario(good(), error(404, "NotFound", "Project not found")).await);
    errors.push(scenario(good(), error(404, "SecretPathNotFound", "Folder not found")).await);
    errors.push(
        scenario(
            good(),
            list_ok(json!([secret("OPEN", VALUE), hidden("SHUT")]), json!([])),
        )
        .await,
    );
    errors.push(scenario(good(), error(429, "RateLimitExceeded", "slow")).await);
    errors.push(scenario(good(), fastify_404("GET", SECRETS)).await);
    errors.push(
        scenario(
            good(),
            list_ok(json!([secret("bad-key", VALUE)]), json!([])),
        )
        .await,
    );
    errors.push(
        scenario(
            good(),
            list_ok(json!([secret("NUL", &format!("{VALUE}\u{0}"))]), json!([])),
        )
        .await,
    );
    errors.push(
        scenario(
            good(),
            Reply::json(
                200,
                &json!({"secrets": [{"secretKey": "K", "secretValue": "x",
                                     "secretValueHidden": VALUE}]}),
            ),
        )
        .await,
    );
    errors.push(
        scenario(
            Reply::json(200, &json!({"accessToken": TOKEN, "expiresIn": TOKEN})),
            unused(),
        )
        .await,
    );

    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        leaky_login(401, "Invalid credentials").delayed(SLOW),
    );
    let cooling = impatient(&stub, Duration::from_secs(30));
    errors.push(cooling.resolve(&scope()).await.must_fail("unanswered"));
    errors.push(cooling.resolve(&scope()).await.must_fail("cooling down"));

    let names: std::collections::BTreeSet<&str> = errors.iter().map(variant).collect();
    assert_eq!(names.len(), 15, "not every variant was produced: {names:?}");
    for e in &errors {
        let name = variant(e);
        for text in [e.to_string(), format!("{e:?}")] {
            assert!(!text.contains(VALUE), "{name} carries a secret value");
            assert!(
                !text.contains(CLIENT_SECRET),
                "{name} carries the client secret"
            );
            assert!(!text.contains(TOKEN), "{name} carries the token");
            assert!(!text.contains(TOKEN_2), "{name} carries the second token");
        }
    }
}

#[tokio::test]
async fn the_provider_debug_shows_neither_the_secret_nor_the_token() {
    let stub = serving(json!([secret("A", VALUE)]));
    let p = provider(&stub);
    p.resolve(&scope()).await.must("resolves");
    let debug = format!("{p:?}");
    assert!(debug.contains(CLIENT_ID), "the Debug lacks the client ID");
    assert!(debug.contains(&stub.base()), "the Debug lacks the base URL");
    assert!(
        !debug.contains(CLIENT_SECRET),
        "the Debug carries the client secret"
    );
    assert!(!debug.contains(TOKEN), "the Debug carries the token");
}

#[tokio::test]
async fn a_malformed_login_answer_never_quotes_the_token() {
    let stub = Stub::start();
    stub.on(
        "POST",
        LOGIN,
        Reply::json(200, &json!({"accessToken": TOKEN, "expiresIn": TOKEN})),
    );
    let err = provider(&stub)
        .resolve(&scope())
        .await
        .must_fail("expected an error");
    assert!(
        matches!(err, SecretError::Protocol { endpoint, .. } if endpoint == LOGIN),
        "expected Protocol at the login"
    );
    assert!(
        !format!("{err} {err:?}").contains(TOKEN),
        "the error quotes the token"
    );
}

#[tokio::test]
async fn a_malformed_list_answer_never_quotes_a_value() {
    let err = list_error(Reply::json(
        200,
        &json!({"secrets": [{"secretKey": "K", "secretValue": "x", "secretValueHidden": VALUE}]}),
    ))
    .await;
    assert!(
        matches!(err, SecretError::Protocol { endpoint, .. } if endpoint == SECRETS),
        "expected Protocol at the list"
    );
    assert!(
        !format!("{err} {err:?}").contains(VALUE),
        "the error quotes a value"
    );
}
