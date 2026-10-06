//! `Settings > Secrets`, from the worker side out (MOD-10 milestone 4).
//!
//! The **worker half** (T2a, then T2b) drives `htui::store_worker::serve` directly: the keyring
//! read and the four keyring writes need no loop state, so `try_serve` answers them for the loop,
//! the harness and `--demo` alike (blueprint B.4). The provider check is the agent runtime's, so
//! its cases call `AgentRuntime::serve` over a fake source and wait on the reply channel.
//!
//! **Every case that can reach a keyring takes `common::mock_keyring()` (or
//! `mock_keyring_broken()`) as its first statement**: the guard installs a process-wide fake and
//! holds a lock while it lives, so the developer's OS store is never opened. The cases that take no
//! guard are the `Backend::Memory` ones, which prove that no keyring is read at all (D10), where a
//! guard would hide the bug they exist to catch, and the `Debug` and runtime cases, which reach no
//! keyring.
#![cfg(feature = "testkit")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use htui::agent_worker::{AgentRuntime, Served};
use htui::app::{Action, Handled};
use htui::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use htui::secrets_settings::{
    CHECK_SECRET_PROVIDER, DEMO_SESSION, IDENTITY_INCOMPLETE, IdentityEntry, IdentityState,
    NO_SOURCE_TO_CHECK, Redacted, SecretCheck, SecretsSnapshot, UrlState,
};
use htui::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::QdrantSection;
use htui_agent::registry::DriverFactory;
use htui_core::secret::fake::{FakeSecretProvider, FakeSecretSource};
use htui_core::secret::{
    MachineIdentity, ProviderHealth, ResolvedSecrets, SecretError, SecretFuture, SecretProvider,
    SecretScope, SecretSource,
};
use htui_core::store::MemStore;
use htui_store::testkit as common;
use htui_store::{Backend, CacheStore, PgStore, secret};
use tokio::sync::mpsc;

/// A client secret: long and not pattern-shaped, so a leak is unmistakable.
const SECRET: &str = "zq7-client-secret-0123456789";
/// A client ID as typed.
const CLIENT_ID: &str = "cid-typed-1";
/// A Qdrant API key as typed.
const QDRANT_KEY: &str = "qk-typed-123";
/// A URL with a password in it, which normalisation refuses.
const URL_WITH_PASSWORD: &str = "https://user:hunter2@x.example";

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// A throwaway config root with an opened mirror under `fingerprint`.
async fn mirror(fingerprint: &str) -> (tempfile::TempDir, CacheStore) {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), fingerprint, PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    (root, cache)
}

/// An offline backend over a fresh mirror: anything but `Memory`, so the keyring is consulted.
async fn offline(fingerprint: &str) -> (tempfile::TempDir, Backend) {
    let (root, cache) = mirror(fingerprint).await;
    (
        root,
        Backend::Offline {
            cache,
            since: Some(Utc::now()),
        },
    )
}

fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The snapshot a reply carries, or a panic naming what came instead.
fn snapshot_of(reply: StoreReply) -> SecretsSnapshot {
    match reply {
        StoreReply::Secrets(snapshot) => snapshot,
        other => panic!("expected a secrets snapshot, got {other:?}"),
    }
}

/// `SecretsInfo` through `serve`.
async fn info(backend: &Backend) -> SecretsSnapshot {
    snapshot_of(serve(backend, &StoreRequest::SecretsInfo).await)
}

/// A `Failed` reply's `(request, message)`, or a panic.
fn failed_of(reply: StoreReply) -> (&'static str, String) {
    match reply {
        StoreReply::Failed { request, message } => (request, message),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn identity_entry(client_id: &str, client_secret: &str) -> IdentityEntry {
    IdentityEntry::new(
        client_id.to_owned(),
        Redacted::new(client_secret.to_owned()),
    )
}

fn envelope(seq: u64, request: StoreRequest) -> RequestEnvelope {
    RequestEnvelope {
        seq,
        origin: Origin::App,
        request,
    }
}

/// A runtime holding `source`, its reply channel, and the answer to one `CheckSecretProvider`.
async fn check_through(source: Option<Arc<dyn SecretSource>>) -> (Served, Option<ReplyEnvelope>) {
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    if let Some(source) = source {
        runtime = runtime.with_secret_source(source);
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let served = runtime
        .serve(
            &demo(),
            &tx,
            &envelope(7, StoreRequest::CheckSecretProvider),
        )
        .await;
    let reply = match served {
        Served::Deferred => Some(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("the check answers within five seconds")
                .expect("the runtime keeps the channel open"),
        ),
        _ => None,
    };
    runtime.finish_background(Duration::from_secs(5)).await;
    (served, reply)
}

/// The provider check's outcome out of a deferred reply at `seq` 7.
fn provider_outcome(reply: Option<ReplyEnvelope>) -> Result<ProviderHealth, SecretError> {
    let envelope = reply.expect("a deferred check answers through the channel");
    assert_eq!(envelope.seq, 7, "the answer goes to the request's address");
    match envelope.reply {
        StoreReply::SecretCheck(SecretCheck::Provider { outcome, .. }) => outcome,
        other => panic!("expected a provider check, got {other:?}"),
    }
}

/// A provider whose `health` is a refused login, like Infisical's 401.
#[derive(Debug)]
struct RefusingHealth;

impl SecretProvider for RefusingHealth {
    fn kind(&self) -> &'static str {
        htui_core::secret::INFISICAL
    }

    fn health(&self) -> SecretFuture<'_, ProviderHealth> {
        Box::pin(async { Err(SecretError::BadCredentials) })
    }

    fn list_keys<'a>(&'a self, _scope: &'a SecretScope) -> SecretFuture<'a, Vec<String>> {
        Box::pin(async { Err(SecretError::BadCredentials) })
    }

    fn resolve<'a>(&'a self, _scope: &'a SecretScope) -> SecretFuture<'a, ResolvedSecrets> {
        Box::pin(async { Err(SecretError::BadCredentials) })
    }
}

/// A Qdrant snapshot with the URL stored and no key.
fn qdrant_stored() -> QdrantSnapshot {
    QdrantSnapshot {
        url_state: QdrantState::Stored,
        key_state: QdrantState::NotStored,
        url_summary: Some("https://qdrant.example:6334".to_owned()),
    }
}

// ---------------------------------------------------------------------------------------------
// Demo: no keyring at all (D10)
// ---------------------------------------------------------------------------------------------

/// No guard on purpose: a `Memory` backend must not read a keyring, and a guard would hide one.
#[tokio::test]
async fn demo_rows_are_not_applicable_and_read_no_keyring() {
    let snapshot = info(&demo()).await;
    assert_eq!(
        snapshot,
        SecretsSnapshot {
            url: UrlState::NotApplicable,
            identity: IdentityState::NotApplicable,
        }
    );
}

/// No guard on purpose, as above: every write is refused before the keyring is reached.
#[tokio::test]
async fn demo_refuses_every_keyring_write() {
    for request in [
        StoreRequest::SetInfisicalUrl("https://x.example".to_owned()),
        StoreRequest::ClearInfisicalUrl,
        StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)),
        StoreRequest::ClearMachineIdentity,
    ] {
        let name = request.name();
        let (request, message) = failed_of(serve(&demo(), &request).await);
        assert_eq!(request, name);
        assert_eq!(message, DEMO_SESSION);
    }
}

// ---------------------------------------------------------------------------------------------
// The keyring rows (D2, A-7)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_keyring_is_not_stored_on_both_rows() {
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-empty").await;
    assert_eq!(
        info(&backend).await,
        SecretsSnapshot {
            url: UrlState::NotStored,
            identity: IdentityState::NotStored,
        }
    );
}

#[tokio::test]
async fn a_stored_url_is_shown_normalised() {
    let _keyring = common::mock_keyring().await;
    secret::set_infisical_url("https://Infisical.Example.com/api/").expect("the fake stores");
    let (_root, backend) = offline("secrets-url").await;
    assert_eq!(
        info(&backend).await.url,
        UrlState::Stored("https://infisical.example.com".to_owned())
    );
}

#[tokio::test]
async fn a_stored_url_that_does_not_normalise_is_unusable_and_never_echoed() {
    let _keyring = common::mock_keyring().await;
    secret::set_infisical_url(URL_WITH_PASSWORD).expect("the fake stores");
    let (_root, backend) = offline("secrets-unusable").await;
    let reply = serve(&backend, &StoreRequest::SecretsInfo).await;
    let shown = format!("{reply:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(
        matches!(snapshot_of(reply).url, UrlState::Unusable(_)),
        "{shown}"
    );
}

#[tokio::test]
async fn a_half_stored_identity_is_half_stored_not_unreadable() {
    let _keyring = common::mock_keyring().await;
    // A blank half reads as absent: a half identity.
    secret::set_machine_identity(&MachineIdentity::new("   ", SECRET)).expect("the fake stores");
    let (_root, backend) = offline("secrets-half").await;
    let reply = serve(&backend, &StoreRequest::SecretsInfo).await;
    let shown = format!("{reply:?}");
    assert!(!shown.contains(SECRET), "{shown}");
    match snapshot_of(reply).identity {
        IdentityState::HalfStored(message) => {
            assert!(message.contains("infisical-client-id"), "{message}");
            assert!(
                message.starts_with(secret::HALF_STORED_IDENTITY),
                "{message}"
            );
        }
        other => panic!("expected a half-stored identity, got {other:?}"),
    }
}

#[tokio::test]
async fn a_broken_keyring_is_unreadable_never_not_stored() {
    let _keyring = common::mock_keyring_broken().await;
    let (_root, backend) = offline("secrets-broken").await;
    let snapshot = info(&backend).await;
    match (&snapshot.url, &snapshot.identity) {
        (UrlState::Unreadable(url), IdentityState::Unreadable(identity)) => {
            assert!(url.contains(common::BROKEN_KEYRING), "{url}");
            assert!(identity.contains(common::BROKEN_KEYRING), "{identity}");
            assert!(
                !url.starts_with("store backend error"),
                "the seam's sentence, without the shell's prefix: {url}"
            );
        }
        other => panic!("both rows are unreadable, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The keyring writes (D2, D3, D4)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn set_infisical_url_stores_the_normalised_form_and_answers_a_fresh_snapshot() {
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-set-url").await;
    let reply = serve(
        &backend,
        &StoreRequest::SetInfisicalUrl("https://Infisical.Example.com/api/".to_owned()),
    )
    .await;
    assert_eq!(
        snapshot_of(reply).url,
        UrlState::Stored("https://infisical.example.com".to_owned())
    );
    assert_eq!(
        secret::get_infisical_url().expect("the fake reads"),
        Some("https://infisical.example.com".to_owned()),
        "the normalised form is what is stored"
    );
}

#[tokio::test]
async fn set_infisical_url_refuses_what_does_not_normalise_and_stores_nothing() {
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-bad-url").await;
    let (request, message) = failed_of(
        serve(
            &backend,
            &StoreRequest::SetInfisicalUrl(URL_WITH_PASSWORD.to_owned()),
        )
        .await,
    );
    assert_eq!(request, "set_infisical_url");
    assert!(!message.contains("hunter2"), "{message}");
    assert_eq!(secret::get_infisical_url().expect("the fake reads"), None);
}

#[tokio::test]
async fn set_machine_identity_stores_both_halves_and_answers_a_fresh_snapshot() {
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-set-identity").await;
    let reply = serve(
        &backend,
        &StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)),
    )
    .await;
    let shown = format!("{reply:?}");
    assert!(
        !shown.contains(SECRET) && !shown.contains(CLIENT_ID),
        "{shown}"
    );
    assert_eq!(snapshot_of(reply).identity, IdentityState::Stored);
    assert_eq!(
        common::fake_machine_identity(),
        (Some(CLIENT_ID.to_owned()), Some(SECRET.to_owned()))
    );
}

#[tokio::test]
async fn set_machine_identity_refuses_a_blank_half_and_stores_nothing() {
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-blank-half").await;
    for (client_id, client_secret) in [("   ", SECRET), (CLIENT_ID, " \t "), ("", "")] {
        let (request, message) = failed_of(
            serve(
                &backend,
                &StoreRequest::SetMachineIdentity(identity_entry(client_id, client_secret)),
            )
            .await,
        );
        assert_eq!(request, "set_machine_identity");
        assert_eq!(message, IDENTITY_INCOMPLETE);
    }
    assert_eq!(common::fake_machine_identity(), (None, None));
}

#[tokio::test]
async fn a_failed_secret_write_is_failed_and_leaves_no_half() {
    let _keyring = common::mock_keyring().await;
    common::refuse_fake_store(secret::INFISICAL_CLIENT_SECRET_USER);
    let (_root, backend) = offline("secrets-refused").await;
    let (request, message) = failed_of(
        serve(
            &backend,
            &StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)),
        )
        .await,
    );
    assert_eq!(request, "set_machine_identity");
    assert!(message.contains("infisical-client-secret"), "{message}");
    assert!(!message.contains(SECRET), "{message}");
    assert_eq!(common::fake_machine_identity(), (None, None));
}

#[tokio::test]
async fn clear_machine_identity_removes_both_halves() {
    let _keyring = common::mock_keyring().await;
    secret::set_machine_identity(&MachineIdentity::new(CLIENT_ID, SECRET)).expect("stores");
    let (_root, backend) = offline("secrets-clear-identity").await;
    let reply = serve(&backend, &StoreRequest::ClearMachineIdentity).await;
    assert_eq!(snapshot_of(reply).identity, IdentityState::NotStored);
    assert_eq!(common::fake_machine_identity(), (None, None));
}

#[tokio::test]
async fn clear_infisical_url_removes_the_url() {
    let _keyring = common::mock_keyring().await;
    secret::set_infisical_url("https://x.example").expect("stores");
    let (_root, backend) = offline("secrets-clear-url").await;
    let reply = serve(&backend, &StoreRequest::ClearInfisicalUrl).await;
    assert_eq!(snapshot_of(reply).url, UrlState::NotStored);
    assert_eq!(secret::get_infisical_url().expect("the fake reads"), None);
}

// ---------------------------------------------------------------------------------------------
// Debug never prints what was typed (D4, D9, A-6)
// ---------------------------------------------------------------------------------------------

#[test]
fn no_secret_request_prints_what_was_typed() {
    let identity = StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET));
    let qdrant = StoreRequest::SetQdrantApiKey(Redacted::new(QDRANT_KEY.to_owned()));
    for shown in [
        format!("{identity:?}"),
        format!("{qdrant:?}"),
        format!("{:?}", envelope(1, identity.clone())),
        format!("{:?}", envelope(2, qdrant.clone())),
    ] {
        for value in [SECRET, CLIENT_ID, QDRANT_KEY] {
            assert!(!shown.contains(value), "{shown}");
        }
        assert!(shown.contains("<redacted>"), "{shown}");
    }
}

/// A-6: the key field is masked, so `text()` is `None` and the old submit sent `""`, which the
/// worker reads as "clear". The typed key is now what is sent.
#[tokio::test]
async fn a_typed_qdrant_key_is_sent_whole_and_redacted() {
    let bench = SectionBench::new().await;
    let mut section = QdrantSection::new();
    bench.reply(&mut section, &StoreReply::Qdrant(qdrant_stored()));
    let _ = bench.drained();

    bench.key(&mut section, "j");
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    for c in QDRANT_KEY.chars() {
        bench.key(&mut section, &c.to_string());
    }
    bench.key(&mut section, "Enter");

    let requests: Vec<StoreRequest> = bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect();
    match requests.as_slice() {
        [StoreRequest::SetQdrantApiKey(key)] => {
            assert_eq!(key.expose(), QDRANT_KEY);
            let shown = format!("{key:?}");
            assert!(!shown.contains(QDRANT_KEY), "{shown}");
        }
        other => panic!("exactly one key write, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The provider check (D5, A-2, A-5)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn check_secret_provider_answers_server_ok() {
    let source = FakeSecretSource::new(Arc::new(FakeSecretProvider::resolving(&[])));
    let (served, reply) = check_through(Some(Arc::new(source))).await;
    assert!(matches!(served, Served::Deferred), "{served:?}");
    match provider_outcome(reply) {
        Ok(ProviderHealth { server_ok, .. }) => assert!(server_ok),
        Err(err) => panic!("the fake is healthy: {err}"),
    }
}

#[tokio::test]
async fn check_secret_provider_passes_a_refused_login_through() {
    let source = FakeSecretSource::new(Arc::new(RefusingHealth));
    let (_, reply) = check_through(Some(Arc::new(source))).await;
    assert_eq!(provider_outcome(reply), Err(SecretError::BadCredentials));
}

#[tokio::test]
async fn check_secret_provider_passes_a_source_refusal_through() {
    let source = FakeSecretSource::failing(SecretError::NoIdentity);
    let (_, reply) = check_through(Some(Arc::new(source))).await;
    assert_eq!(provider_outcome(reply), Err(SecretError::NoIdentity));
}

#[tokio::test]
async fn check_secret_provider_without_a_source_is_failed_with_one_sentence() {
    let (served, reply) = check_through(None).await;
    assert!(reply.is_none(), "nothing deferred");
    match served {
        Served::Reply(StoreReply::Failed { request, message }) => {
            assert_eq!(request, CHECK_SECRET_PROVIDER);
            assert_eq!(message, NO_SOURCE_TO_CHECK);
        }
        other => panic!("a runtime with no source refuses: {other:?}"),
    }
}

#[tokio::test]
async fn a_harness_without_an_agent_runtime_refuses_the_provider_check_by_name() {
    let mut harness = Harness::demo();
    harness.drive().await;
    harness
        .app()
        .update(Action::Store(StoreRequest::CheckSecretProvider));
    harness.drive().await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("check_secret_provider: no agent runtime in this harness")
    );
}
