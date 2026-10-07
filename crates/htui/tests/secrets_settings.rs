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
//!
//! The **section half** (T3) drives `SecretsSection` through `SectionBench`: keys in, `Action`s
//! and frames out, with no store behind it (`R-NF-3`). Its replies are built by hand, or served
//! by `serve` over a `MemStore` where the case is about a write's own answer.
#![cfg(feature = "testkit")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use htui::agent_worker::{AgentRuntime, Served};
use htui::app::{Action, Handled};
use htui::hierarchy::{self, HierarchySnapshot, ScopeWrite};
use htui::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use htui::secrets_settings::{
    CHECK_SECRET_PROVIDER, CHECK_SECRET_SCOPE, DEMO_SESSION, IDENTITY_INCOMPLETE, IdentityEntry,
    IdentityState, NO_PROVIDER_TO_CHECK, NO_SOURCE_TO_CHECK, Redacted, SET_PROJECT_SECRET_SCOPE,
    SecretCheck, SecretsSnapshot, UrlState,
};
use htui::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::{HierarchySection, QdrantSection, SecretsSection, SettingsSection};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{NewProject, Project, ProjectId, Scope};
use htui_core::secret::fake::{FakeSecretProvider, FakeSecretSource};
use htui_core::secret::{
    INFISICAL, MachineIdentity, ProviderHealth, ResolvedSecrets, SecretError, SecretFuture,
    SecretProvider, SecretScope, SecretSource, project_scope,
};
use htui_core::store::{MemStore, ReadStore, StoreError, WriteStore};
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

/// A keyring write's own answer, named `request` (R1 M-1), or a panic naming what came instead.
#[track_caller]
fn keyring_written_of(reply: StoreReply, request: &str) -> SecretsSnapshot {
    match reply {
        StoreReply::SecretsWritten {
            request: named,
            snapshot,
            ..
        } if named == request => snapshot,
        other => panic!("expected `{request}`'s own reply, got {other:?}"),
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
        INFISICAL
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

/// Under a broken fake keyring (R1 L-2): a `Memory` backend must not read a keyring, and a
/// regression that does reads the fake, never the real OS keyring, and answers `Unreadable`
/// instead of `NotApplicable`.
#[tokio::test]
async fn demo_rows_are_not_applicable_and_read_no_keyring() {
    let _keyring = common::mock_keyring_broken().await;
    let snapshot = info(&demo()).await;
    assert_eq!(
        snapshot,
        SecretsSnapshot {
            url: UrlState::NotApplicable,
            identity: IdentityState::NotApplicable,
        }
    );
}

/// Under a broken fake keyring, as above: every write is refused before the keyring is reached,
/// and a regression that reaches it is refused with the keyring's sentence, not the demo one.
#[tokio::test]
async fn demo_refuses_every_keyring_write() {
    let _keyring = common::mock_keyring_broken().await;
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
        keyring_written_of(reply, "set_infisical_url").url,
        UrlState::Stored("https://infisical.example.com".to_owned())
    );
    assert_eq!(
        secret::get_infisical_url().expect("the fake reads"),
        Some("https://infisical.example.com".to_owned()),
        "the normalised form is what is stored"
    );
}

/// The generation a keyring write's own answer carries, or a panic.
#[track_caller]
fn generation_of(reply: &StoreReply) -> u64 {
    match reply {
        StoreReply::SecretsWritten { generation, .. } => *generation,
        other => panic!("expected a keyring write's answer, got {other:?}"),
    }
}

/// Whether a keyring write's own answer says its write mark was stored (MOD-90 D3, R1 M-1), or a
/// panic.
#[track_caller]
fn mark_stored_of(reply: &StoreReply) -> bool {
    match reply {
        StoreReply::SecretsWritten { mark_stored, .. } => *mark_stored,
        other => panic!("expected a keyring write's answer, got {other:?}"),
    }
}

#[tokio::test]
async fn every_landed_keyring_write_answers_a_higher_generation() {
    // R1 L-1: the section compares a provider check's generation with these.
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-generation").await;
    let mut last = 0;
    for request in [
        StoreRequest::SetInfisicalUrl(STORED_URL.to_owned()),
        StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)),
        StoreRequest::ClearMachineIdentity,
        StoreRequest::ClearInfisicalUrl,
    ] {
        let generation = generation_of(&serve(&backend, &request).await);
        assert!(
            generation > last,
            "{}: {generation} after {last}",
            request.name()
        );
        last = generation;
    }
}

/// The four Settings keyring writes.
fn the_four_writes() -> [StoreRequest; 4] {
    [
        StoreRequest::SetInfisicalUrl(STORED_URL.to_owned()),
        StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET)),
        StoreRequest::ClearMachineIdentity,
        StoreRequest::ClearInfisicalUrl,
    ]
}

/// The write mark the fake holds now (MOD-90 D1).
fn mark_now() -> Option<String> {
    secret::get_infisical_write_mark().expect("the fake reads")
}

#[tokio::test]
async fn every_landed_keyring_write_stores_a_new_mark() {
    // MOD-90 D1: another process (`htui worker`) sees a landed write by its new mark alone.
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-mark").await;
    let mut last = mark_now();
    assert_eq!(last, None, "an empty keyring has no mark");
    for request in the_four_writes() {
        let name = request.name();
        let reply = serve(&backend, &request).await;
        assert!(
            mark_stored_of(&reply),
            "{name}: the reply says the mark landed (R1 M-1)"
        );
        keyring_written_of(reply, name);
        let mark = mark_now();
        assert!(mark.is_some(), "{name} stores a mark");
        assert_ne!(mark, last, "{name} stores a new mark");
        last = mark;
    }

    // A refused write stores no mark: a blank half, a URL normalisation refuses, a demo session.
    let blank = StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, "  "));
    failed_of(serve(&backend, &blank).await);
    assert_eq!(mark_now(), last, "a blank half stores no mark");
    let lan = StoreRequest::SetInfisicalUrl("http://192.168.1.10".to_owned());
    failed_of(serve(&backend, &lan).await);
    assert_eq!(mark_now(), last, "a refused URL stores no mark");
    for request in the_four_writes() {
        failed_of(serve(&demo(), &request).await);
        assert_eq!(mark_now(), last, "a demo {} stores no mark", request.name());
    }

    // MOD-90 D2: the mark is written last, so a write the keyring refuses leaves it as it was.
    common::refuse_fake_store(secret::INFISICAL_CLIENT_SECRET_USER);
    let refused = StoreRequest::SetMachineIdentity(identity_entry(CLIENT_ID, SECRET));
    failed_of(serve(&backend, &refused).await);
    assert_eq!(mark_now(), last, "a keyring-refused write stores no mark");
}

#[tokio::test]
async fn a_refused_mark_write_still_answers_written() {
    // MOD-90 D3: the write landed; only the mark is lost, so the reply is still the write's own.
    let _keyring = common::mock_keyring().await;
    common::refuse_fake_store(secret::INFISICAL_WRITE_MARK_USER);
    let (_root, backend) = offline("secrets-mark-refused").await;
    let mut last = 0;
    for request in the_four_writes() {
        let name = request.name();
        let reply = serve(&backend, &request).await;
        let generation = generation_of(&reply);
        assert!(
            !mark_stored_of(&reply),
            "{name}: the reply says the mark was refused (R1 M-1)"
        );
        keyring_written_of(reply, name);
        assert!(generation > last, "{name}: {generation} after {last}");
        last = generation;
        if matches!(request, StoreRequest::SetMachineIdentity(_)) {
            assert!(
                common::fake_machine_identity()
                    == (Some(CLIENT_ID.to_owned()), Some(SECRET.to_owned())),
                "the identity landed"
            );
        }
    }
    assert_eq!(mark_now(), None, "the refused mark was never stored");
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
    assert_eq!(
        keyring_written_of(reply, "set_machine_identity").identity,
        IdentityState::Stored
    );
    assert_eq!(
        common::fake_machine_identity(),
        (Some(CLIENT_ID.to_owned()), Some(SECRET.to_owned()))
    );
}

#[tokio::test]
async fn set_machine_identity_stores_the_trimmed_halves() {
    // CLEAN-8 #4: spaces around either half are dropped before the keyring sees them.
    let _keyring = common::mock_keyring().await;
    let (_root, backend) = offline("secrets-trimmed").await;
    let reply = serve(
        &backend,
        &StoreRequest::SetMachineIdentity(identity_entry(
            " cid-typed-1\t",
            "  zq7-client-secret-0123456789 ",
        )),
    )
    .await;
    assert_eq!(
        keyring_written_of(reply, "set_machine_identity").identity,
        IdentityState::Stored
    );
    assert!(
        common::fake_machine_identity() == (Some(CLIENT_ID.to_owned()), Some(SECRET.to_owned())),
        "both halves are stored trimmed"
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
    assert_eq!(
        keyring_written_of(reply, "clear_machine_identity").identity,
        IdentityState::NotStored
    );
    assert_eq!(common::fake_machine_identity(), (None, None));
}

#[tokio::test]
async fn clear_infisical_url_removes_the_url() {
    let _keyring = common::mock_keyring().await;
    secret::set_infisical_url("https://x.example").expect("stores");
    let (_root, backend) = offline("secrets-clear-url").await;
    let reply = serve(&backend, &StoreRequest::ClearInfisicalUrl).await;
    assert_eq!(
        keyring_written_of(reply, "clear_infisical_url").url,
        UrlState::NotStored
    );
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

// ---------------------------------------------------------------------------------------------
// The project scope write (D6, D7; A-1, A-11)
// ---------------------------------------------------------------------------------------------

/// A value an Infisical key holds, which no reply may carry.
const V1: &str = "v1-secret-value-0001";
/// Another.
const V2: &str = "v2-secret-value-0002";

/// A scope as the section would build it.
fn a_scope() -> SecretScope {
    SecretScope::new("p-graphics", "dev", "/app").expect("a valid scope")
}

/// The demo tree of `Graphics`, read from the store as the section's token would be.
async fn graphics(store: &MemStore) -> HierarchySnapshot {
    hierarchy::snapshot(store, ids::WORKSPACE_GRAPHICS, None)
        .await
        .expect("the store answers")
        .expect("the demo has `Graphics`")
}

/// `project`'s row inside `tree`, or a panic.
#[track_caller]
fn row_in(tree: &HierarchySnapshot, project: ProjectId) -> Project {
    tree.projects
        .iter()
        .find(|entry| entry.project.id == project)
        .map(|entry| entry.project.clone())
        .expect("the project is in the tree")
}

/// The project row as the store holds it now.
async fn stored(store: &MemStore, project: ProjectId) -> Project {
    store
        .project(project)
        .await
        .expect("the store answers")
        .expect("the project is there")
}

/// `SetProjectSecretScope` through `serve`.
async fn write_scope(
    backend: &Backend,
    id: ProjectId,
    expected: chrono::DateTime<Utc>,
    scope: Option<SecretScope>,
) -> StoreReply {
    serve(
        backend,
        &StoreRequest::SetProjectSecretScope {
            id,
            expected,
            scope,
        },
    )
    .await
}

/// A `SecretScopeWritten` reply's `(project, tree, outcome)`, or a panic.
#[track_caller]
fn written(reply: StoreReply) -> (ProjectId, HierarchySnapshot, ScopeWrite) {
    match reply {
        StoreReply::SecretScopeWritten {
            project,
            tree,
            outcome,
        } => (project, *tree, outcome),
        other => panic!("expected a scope write's own reply, got {other:?}"),
    }
}

#[tokio::test]
async fn set_project_secret_scope_writes_both_columns_and_answers_its_own_reply() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let token = row_in(&graphics(&store).await, ids::PROJECT_VULKAN).updated_at;

    let (project, tree, outcome) =
        written(write_scope(&backend, ids::PROJECT_VULKAN, token, Some(a_scope())).await);

    assert_eq!(project, ids::PROJECT_VULKAN);
    assert_eq!(outcome, ScopeWrite::Applied);
    assert_eq!(tree.workspace.id, ids::WORKSPACE_GRAPHICS);
    let column = Some(a_scope().to_column());
    for row in [
        row_in(&tree, ids::PROJECT_VULKAN),
        stored(&store, ids::PROJECT_VULKAN).await,
    ] {
        assert_eq!(row.secret_provider.as_deref(), Some(INFISICAL));
        assert_eq!(row.secret_scope, column);
    }
}

#[tokio::test]
async fn a_written_scope_is_what_the_next_walk_reads() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let token = row_in(&graphics(&store).await, ids::PROJECT_VULKAN).updated_at;
    let _ = written(write_scope(&backend, ids::PROJECT_VULKAN, token, Some(a_scope())).await);

    assert_eq!(
        project_scope(&stored(&store, ids::PROJECT_VULKAN).await),
        Ok(Some(a_scope()))
    );
}

#[tokio::test]
async fn clearing_a_scope_nulls_both_columns() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let token = row_in(&graphics(&store).await, ids::PROJECT_VULKAN).updated_at;
    let (_, tree, _) =
        written(write_scope(&backend, ids::PROJECT_VULKAN, token, Some(a_scope())).await);
    let token = row_in(&tree, ids::PROJECT_VULKAN).updated_at;

    let (_, tree, outcome) = written(write_scope(&backend, ids::PROJECT_VULKAN, token, None).await);

    assert_eq!(outcome, ScopeWrite::Applied);
    for row in [
        row_in(&tree, ids::PROJECT_VULKAN),
        stored(&store, ids::PROJECT_VULKAN).await,
    ] {
        assert_eq!(row.secret_provider, None);
        assert_eq!(row.secret_scope, None);
    }
    assert_eq!(
        project_scope(&stored(&store, ids::PROJECT_VULKAN).await),
        Ok(None)
    );
}

#[tokio::test]
async fn a_spent_token_answers_stale_and_writes_nothing() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let spent = row_in(&graphics(&store).await, ids::PROJECT_VULKAN).updated_at;
    let _ = written(write_scope(&backend, ids::PROJECT_VULKAN, spent, Some(a_scope())).await);
    let other = SecretScope::new("p-other", "prod", "/").expect("a valid scope");

    let (project, tree, outcome) =
        written(write_scope(&backend, ids::PROJECT_VULKAN, spent, Some(other)).await);

    assert_eq!(project, ids::PROJECT_VULKAN);
    assert_eq!(outcome, ScopeWrite::Stale);
    let now = stored(&store, ids::PROJECT_VULKAN).await;
    assert_eq!(now.secret_scope, Some(a_scope().to_column()), "unchanged");
    let current = match serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await {
        StoreReply::Hierarchy(Some(tree)) => *tree,
        other => panic!("expected a tree: {other:?}"),
    };
    assert_eq!(tree, current, "a stale write answers the tree as it is now");
    assert_eq!(
        row_in(&tree, ids::PROJECT_VULKAN).updated_at,
        now.updated_at
    );
}

/// The `tests/hierarchy.rs` shape: the workspace is resolved before the write, so an unlinked
/// project is refused by the write's own name and nothing is written.
#[tokio::test]
async fn an_unlinked_project_is_refused_before_anything_is_written() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let orphan = store
        .create_project(NewProject {
            id: ProjectId::new(),
            slug: "orphan".to_owned(),
            name: "Orphan".to_owned(),
            description: String::new(),
            created_by: ids::USER,
        })
        .await
        .expect("the store creates it");

    let (request, message) =
        failed_of(write_scope(&backend, orphan.id, orphan.updated_at, Some(a_scope())).await);

    assert_eq!(request, SET_PROJECT_SECRET_SCOPE);
    assert!(message.contains("workspace_project"), "{message}");
    let row = stored(&store, orphan.id).await;
    assert_eq!(row.secret_provider, None);
    assert_eq!(row.secret_scope, None);
}

/// A-11: the Hierarchy section matches its `Failed` replies by `hierarchy::REQUEST_NAMES`, so the
/// scope write is not one of them: its refusals are the Secrets section's.
#[test]
fn the_scope_write_is_not_a_hierarchy_request_name() {
    assert!(!hierarchy::REQUEST_NAMES.contains(&SET_PROJECT_SECRET_SCOPE));
    let request = StoreRequest::SetProjectSecretScope {
        id: ids::PROJECT_VULKAN,
        expected: Utc::now(),
        scope: None,
    };
    assert_eq!(request.name(), SET_PROJECT_SECRET_SCOPE);
}

// ---------------------------------------------------------------------------------------------
// The scope check (D8)
// ---------------------------------------------------------------------------------------------

/// R1 L-3: the Secrets section's tree read is `Hierarchy`'s read under its own name, so its
/// answer can never be taken for a Hierarchy write's.
#[tokio::test]
async fn the_secrets_tree_read_answers_the_hierarchy_tree_under_its_own_name() {
    let backend = demo();
    let request = StoreRequest::SecretsTree(ids::WORKSPACE_GRAPHICS);
    assert_eq!(request.name(), "secrets_tree");
    assert!(!hierarchy::REQUEST_NAMES.contains(&request.name()));
    let tree = match serve(&backend, &request).await {
        StoreReply::SecretsTree(Some(tree)) => *tree,
        other => panic!("expected the Secrets tree, got {other:?}"),
    };
    let read = match serve(&backend, &StoreRequest::Hierarchy(ids::WORKSPACE_GRAPHICS)).await {
        StoreReply::Hierarchy(Some(tree)) => *tree,
        other => panic!("expected the hierarchy tree, got {other:?}"),
    };
    assert_eq!(tree, read);
}

/// A runtime holding `source` (when given) and the answer to one `CheckSecretScope` of `project`
/// over `store`: the immediate `Served`, and the deferred reply when there is one.
async fn scope_check_through(
    store: &MemStore,
    source: Option<Arc<dyn SecretSource>>,
    project: ProjectId,
) -> (Served, Option<ReplyEnvelope>) {
    let mut runtime = AgentRuntime::new(DriverFactory::new());
    if let Some(source) = source {
        runtime = runtime.with_secret_source(source);
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let served = runtime
        .serve(
            &Backend::memory(store.clone()),
            &tx,
            &envelope(9, StoreRequest::CheckSecretScope { project }),
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

/// A scope check's `(project, outcome)` out of a reply, or a panic.
#[track_caller]
fn scope_outcome(reply: &StoreReply) -> (ProjectId, Result<usize, SecretError>) {
    match reply {
        StoreReply::SecretCheck(SecretCheck::Scope {
            project, outcome, ..
        }) => (*project, outcome.clone()),
        other => panic!("expected a scope check, got {other:?}"),
    }
}

/// The demo store with `Vulkan` on Infisical at [`a_scope`].
fn scoped_store() -> MemStore {
    let store = MemStore::demo();
    store.set_project_secret_columns(
        ids::PROJECT_VULKAN,
        Some(INFISICAL),
        Some(&a_scope().to_column()),
    );
    store
}

#[tokio::test]
async fn check_secret_scope_answers_a_key_count_and_no_names() {
    let store = scoped_store();
    let provider = FakeSecretProvider::resolving(&[("API_KEY", V1), ("DB_URL", V2)]);
    let source = Arc::new(FakeSecretSource::new(Arc::new(provider)));

    let (served, reply) =
        scope_check_through(&store, Some(source.clone()), ids::PROJECT_VULKAN).await;

    assert!(matches!(served, Served::Deferred), "{served:?}");
    let envelope = reply.expect("a deferred check answers through the channel");
    assert_eq!(envelope.seq, 9, "the answer goes to the request's address");
    assert_eq!(scope_outcome(&envelope.reply), (ids::PROJECT_VULKAN, Ok(2)));
    let shown = format!("{:?}", envelope.reply);
    for name_or_value in ["API_KEY", "DB_URL", V1, V2] {
        assert!(!shown.contains(name_or_value), "{shown}");
    }
    assert_eq!(source.calls(), 1);
}

#[tokio::test]
async fn check_secret_scope_refuses_a_provider_less_project_without_the_source() {
    let store = MemStore::demo();
    let source = Arc::new(FakeSecretSource::new(Arc::new(
        FakeSecretProvider::resolving(&[]),
    )));

    let (served, reply) =
        scope_check_through(&store, Some(source.clone()), ids::PROJECT_VULKAN).await;

    assert!(reply.is_none(), "nothing deferred");
    match served {
        Served::Reply(reply) => assert_eq!(
            scope_outcome(&reply),
            (
                ids::PROJECT_VULKAN,
                Err(SecretError::Config(NO_PROVIDER_TO_CHECK.to_owned()))
            )
        ),
        other => panic!("answered at once: {other:?}"),
    }
    assert_eq!(source.calls(), 0, "the source is not asked");
}

#[tokio::test]
async fn check_secret_scope_refuses_a_column_fault_before_the_source() {
    let store = MemStore::demo();
    store.set_project_secret_columns(
        ids::PROJECT_VULKAN,
        Some("vault"),
        Some(&a_scope().to_column()),
    );
    let source = Arc::new(FakeSecretSource::new(Arc::new(
        FakeSecretProvider::resolving(&[]),
    )));

    let (served, reply) =
        scope_check_through(&store, Some(source.clone()), ids::PROJECT_VULKAN).await;

    assert!(reply.is_none(), "nothing deferred");
    match served {
        Served::Reply(reply) => {
            let (project, outcome) = scope_outcome(&reply);
            assert_eq!(project, ids::PROJECT_VULKAN);
            assert!(
                matches!(outcome, Err(SecretError::Config(_))),
                "{outcome:?}"
            );
        }
        other => panic!("answered at once: {other:?}"),
    }
    assert_eq!(source.calls(), 0, "the source is not asked");
}

#[tokio::test]
async fn check_secret_scope_passes_a_provider_error_through() {
    let store = scoped_store();
    let refusal = SecretError::PermissionDenied {
        detail: "the identity may not read /app".to_owned(),
    };
    let source = Arc::new(FakeSecretSource::new(Arc::new(
        FakeSecretProvider::failing(refusal.clone()),
    )));

    let (_, reply) = scope_check_through(&store, Some(source), ids::PROJECT_VULKAN).await;

    let envelope = reply.expect("a deferred check answers through the channel");
    assert_eq!(
        scope_outcome(&envelope.reply),
        (ids::PROJECT_VULKAN, Err(refusal))
    );
}

#[tokio::test]
async fn check_secret_scope_of_an_unknown_project_is_failed_by_name_without_the_source() {
    let unknown = ProjectId::new();
    let source = Arc::new(FakeSecretSource::new(Arc::new(
        FakeSecretProvider::resolving(&[]),
    )));

    let (served, reply) = scope_check_through(&scoped_store(), Some(source.clone()), unknown).await;

    assert!(reply.is_none(), "nothing deferred");
    match served {
        Served::Reply(StoreReply::Failed { request, message }) => {
            assert_eq!(request, CHECK_SECRET_SCOPE);
            let missing = StoreError::NotFound {
                entity: "project",
                id: unknown.to_string(),
            };
            assert_eq!(message, missing.to_string());
        }
        other => panic!("a missing project is failed at once: {other:?}"),
    }
    assert_eq!(source.calls(), 0, "the source is not asked");
}

#[tokio::test]
async fn check_secret_scope_without_a_source_is_failed_with_one_sentence() {
    let (served, reply) = scope_check_through(&scoped_store(), None, ids::PROJECT_VULKAN).await;
    assert!(reply.is_none(), "nothing deferred");
    match served {
        Served::Reply(StoreReply::Failed { request, message }) => {
            assert_eq!(request, CHECK_SECRET_SCOPE);
            assert_eq!(message, NO_SOURCE_TO_CHECK);
        }
        other => panic!("a runtime with no source refuses: {other:?}"),
    }
}

#[tokio::test]
async fn a_harness_without_an_agent_runtime_refuses_the_scope_check_by_name() {
    let mut harness = Harness::demo();
    harness.drive().await;
    harness
        .app()
        .update(Action::Store(StoreRequest::CheckSecretScope {
            project: ids::PROJECT_VULKAN,
        }));
    harness.drive().await;
    assert_eq!(
        harness.app().status.as_deref(),
        Some("check_secret_scope: no agent runtime in this harness")
    );
}

// ---------------------------------------------------------------------------------------------
// The section (T3): `SecretsSection` through `SectionBench` (D1, D3–D6, D8; A-1)
// ---------------------------------------------------------------------------------------------

/// The normalised URL the keyring rows show when one is stored.
const STORED_URL: &str = "https://infisical.example.com";

/// Row indices in cursor order: the four fixed rows, then the projects.
const ROW_URL: usize = 1;
const ROW_IDENTITY: usize = 2;
const ROW_HEALTH: usize = 3;
const ROW_FIRST_PROJECT: usize = 4;

/// A keyring snapshot reply.
fn keyring(url: UrlState, identity: IdentityState) -> StoreReply {
    StoreReply::Secrets(SecretsSnapshot { url, identity })
}

/// Both rows stored.
fn configured() -> StoreReply {
    keyring(
        UrlState::Stored(STORED_URL.to_owned()),
        IdentityState::Stored,
    )
}

/// Neither row stored.
fn not_configured() -> StoreReply {
    keyring(UrlState::NotStored, IdentityState::NotStored)
}

/// `--demo`: no keyring consulted.
fn demo_keyring() -> StoreReply {
    keyring(UrlState::NotApplicable, IdentityState::NotApplicable)
}

/// A tree as the read answers it.
fn tree_reply(tree: &HierarchySnapshot) -> StoreReply {
    StoreReply::Hierarchy(Some(Box::new(tree.clone())))
}

/// A tree as the Secrets section's own read answers it (R1 L-3).
fn secrets_tree_reply(tree: &HierarchySnapshot) -> StoreReply {
    StoreReply::SecretsTree(Some(Box::new(tree.clone())))
}

/// `keyring_reply`'s rows as the keyring write `request`'s own answer (R1 M-1), the session's
/// first keyring write: generation 1 (R1 L-1).
#[track_caller]
fn keyring_landed(request: &'static str, keyring_reply: StoreReply) -> StoreReply {
    match keyring_reply {
        StoreReply::Secrets(snapshot) => StoreReply::SecretsWritten {
            request,
            generation: 1,
            mark_stored: true,
            snapshot,
        },
        other => panic!("expected a keyring snapshot, got {other:?}"),
    }
}

/// `tree` with `project` gone, as a re-read after a delete elsewhere answers it.
fn without(tree: &HierarchySnapshot, project: ProjectId) -> HierarchySnapshot {
    let mut tree = tree.clone();
    tree.projects.retain(|entry| entry.project.id != project);
    tree
}

/// A fixed check time, so the rows and snapshots are stable.
fn at(hour: u32, minute: u32, second: u32) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    Utc.with_ymd_and_hms(2026, 10, 7, hour, minute, second)
        .single()
        .expect("a valid instant")
}

/// The `Action::Store`s the section emitted since the last drain.
fn requests(bench: &SectionBench) -> Vec<StoreRequest> {
    bench
        .drained()
        .into_iter()
        .filter_map(|action| match action {
            Action::Store(request) => Some(request),
            _ => None,
        })
        .collect()
}

/// Types `text` one key at a time.
fn type_text(bench: &SectionBench, section: &mut SecretsSection, text: &str) {
    for c in text.chars() {
        let chord = if c == ' ' {
            "space".to_owned()
        } else {
            c.to_string()
        };
        bench.key(section, &chord);
    }
}

/// Moves the cursor to `row` from the top.
fn go_to(bench: &SectionBench, section: &mut SecretsSection, row: usize) {
    for _ in 0..8 {
        bench.key(section, "k");
    }
    for _ in 0..row {
        bench.key(section, "j");
    }
}

/// A section over `keyring_reply` and `store`'s `Graphics` tree, with nothing left to drain.
async fn loaded_over(
    store: &MemStore,
    keyring_reply: StoreReply,
) -> (SectionBench, SecretsSection, HierarchySnapshot) {
    let bench = SectionBench::new().await;
    let mut section = SecretsSection::new();
    let tree = graphics(store).await;
    bench.reply(&mut section, &keyring_reply);
    bench.reply(&mut section, &tree_reply(&tree));
    let _ = bench.drained();
    (bench, section, tree)
}

/// [`loaded_over`] the plain demo store.
async fn loaded(keyring_reply: StoreReply) -> (SectionBench, SecretsSection, HierarchySnapshot) {
    loaded_over(&MemStore::demo(), keyring_reply).await
}

/// The rendered section at 100 columns.
fn frame(bench: &SectionBench, section: &SecretsSection) -> String {
    bench.render_section(section, 100)
}

/// `tree` with `Vulkan`'s row replaced by the store's current one.
async fn refreshed(store: &MemStore) -> HierarchySnapshot {
    graphics(store).await
}

/// A scope write's own reply.
fn scope_written(project: ProjectId, tree: &HierarchySnapshot, outcome: ScopeWrite) -> StoreReply {
    StoreReply::SecretScopeWritten {
        project,
        tree: Box::new(tree.clone()),
        outcome,
    }
}

/// The scope-form request out of a drain, or a panic.
#[track_caller]
fn the_scope_write(
    requests: &[StoreRequest],
) -> (ProjectId, chrono::DateTime<Utc>, Option<SecretScope>) {
    match requests {
        [
            StoreRequest::SetProjectSecretScope {
                id,
                expected,
                scope,
            },
        ] => (*id, *expected, scope.clone()),
        other => panic!("exactly one scope write, got {other:?}"),
    }
}

#[tokio::test]
async fn wants_requests_names_the_keyring_read_and_the_tree() {
    let bench = SectionBench::new().await;
    let section = SecretsSection::new();
    let scope = Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    };
    let wanted = section.wants_requests(&scope);
    assert!(
        matches!(
            wanted.as_slice(),
            [StoreRequest::SecretsInfo, StoreRequest::SecretsTree(ws)] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "{wanted:?}"
    );
    assert_eq!(section.title(), "Secrets");
    assert_eq!(section.id(), SecretsSection::ID);
    let _ = bench.drained();
}

#[tokio::test]
async fn rows_come_from_the_snapshot_and_the_tree() {
    let (bench, section, _) = loaded(keyring(
        UrlState::Stored(STORED_URL.to_owned()),
        IdentityState::NotStored,
    ))
    .await;
    let shown = frame(&bench, &section);
    for expected in [
        "infisical",
        "stored \u{b7} https://infisical.example.com",
        "not stored",
        "not checked this session",
        "Projects",
        "vulkan-tutorials",
        "no secret provider",
    ] {
        assert!(shown.contains(expected), "`{expected}` in {shown}");
    }
}

#[tokio::test]
async fn e_on_url_sends_the_normalised_url() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    assert!(section.captures_input());
    type_text(&bench, &mut section, " https://Infisical.Example.com/api/ ");
    bench.key(&mut section, "Enter");

    match requests(&bench).as_slice() {
        [StoreRequest::SetInfisicalUrl(url)] => assert_eq!(url, STORED_URL),
        other => panic!("exactly one URL write, got {other:?}"),
    }
    assert!(!section.captures_input(), "back to the rows");
}

#[tokio::test]
async fn a_refused_url_emits_nothing_and_the_notice_never_repeats_it() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, URL_WITH_PASSWORD);
    bench.key(&mut section, "Enter");

    assert!(bench.drained().is_empty(), "a refusal emits nothing (D3)");
    assert!(section.captures_input(), "the field stays open");
    let refusal = htui_secrets::normalise_base_url(URL_WITH_PASSWORD)
        .expect_err("a URL with a password is refused")
        .to_string();
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains(&refusal), "{refusal} in {shown}");
    // The field itself shows what was typed (it is a plain field); the notice does not repeat it.
    assert!(!refusal.contains("hunter2"), "{refusal}");
    let printed = format!("{section:?}");
    assert!(!printed.contains("hunter2"), "{printed}");
}

#[tokio::test]
async fn the_identity_form_sends_one_redacted_entry() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);

    let before = frame(&bench, &section);
    assert!(!before.contains(SECRET), "{before}");
    assert!(before.contains('\u{2022}'), "{before}");
    assert!(before.contains("(28)"), "{before}");
    assert!(bench.drained().is_empty(), "nothing before Enter");

    bench.key(&mut section, "Enter");
    match requests(&bench).as_slice() {
        [StoreRequest::SetMachineIdentity(entry)] => {
            assert_eq!(entry.client_id(), CLIENT_ID);
            assert_eq!(entry.expose_client_secret(), SECRET);
            let shown = format!("{entry:?}");
            assert!(
                !shown.contains(SECRET) && !shown.contains(CLIENT_ID),
                "{shown}"
            );
        }
        other => panic!("exactly one identity write, got {other:?}"),
    }
    assert!(!section.captures_input());
}

#[tokio::test]
async fn a_blank_identity_half_emits_nothing() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);

    // No secret.
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Enter");
    assert!(bench.drained().is_empty());
    assert!(section.captures_input(), "the form stays open");
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("both the client ID and the client secret are required"),
        "{shown}"
    );
    bench.key(&mut section, "Esc");

    // No client ID.
    bench.key(&mut section, "e");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    assert!(bench.drained().is_empty());
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("(0)"),
        "the secret field was replaced: {shown}"
    );
}

#[tokio::test]
async fn tab_and_backtab_move_between_the_identity_fields() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "s");
    bench.key(&mut section, "BackTab");
    type_text(&bench, &mut section, "i");
    bench.key(&mut section, "Down");
    type_text(&bench, &mut section, "t");
    bench.key(&mut section, "Up");
    type_text(&bench, &mut section, "d");
    bench.key(&mut section, "Enter");

    match requests(&bench).as_slice() {
        [StoreRequest::SetMachineIdentity(entry)] => {
            assert_eq!(entry.client_id(), "id");
            assert_eq!(entry.expose_client_secret(), "st");
        }
        other => panic!("exactly one identity write, got {other:?}"),
    }
}

#[tokio::test]
async fn esc_drops_the_identity_form() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    assert_eq!(bench.key(&mut section, "Esc"), Handled::Consumed);

    assert!(!section.captures_input());
    assert!(bench.drained().is_empty());
    let shown = frame(&bench, &section);
    assert!(!shown.contains("client secret:"), "{shown}");
    // Reopening starts empty.
    bench.key(&mut section, "e");
    let shown = frame(&bench, &section);
    assert!(
        !shown.contains(CLIENT_ID) && shown.contains("(0)"),
        "{shown}"
    );
}

#[tokio::test]
async fn c_on_identity_asks_then_clears() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "c");
    let shown = frame(&bench, &section);
    assert!(shown.contains("Remove the machine identity"), "{shown}");
    bench.key(&mut section, "n");
    assert!(bench.drained().is_empty(), "`n` sends nothing");
    assert!(!section.captures_input());

    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::ClearMachineIdentity]
    ));
    bench.reply(
        &mut section,
        &keyring_landed(
            "clear_machine_identity",
            keyring(
                UrlState::Stored(STORED_URL.to_owned()),
                IdentityState::NotStored,
            ),
        ),
    );
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("the machine identity is gone from the keyring"),
        "{shown}"
    );
}

#[tokio::test]
async fn c_on_url_asks_then_clears() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "c");
    let shown = frame(&bench, &section);
    assert!(shown.contains("Remove the Infisical URL"), "{shown}");
    bench.key(&mut section, "Esc");
    assert!(bench.drained().is_empty());

    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::ClearInfisicalUrl]
    ));
    bench.reply(
        &mut section,
        &keyring_landed(
            "clear_infisical_url",
            keyring(UrlState::NotStored, IdentityState::Stored),
        ),
    );
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("the Infisical URL is gone from the keyring"),
        "{shown}"
    );
}

#[tokio::test]
async fn c_on_a_row_with_nothing_stored_is_refused_by_its_row_text() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    for row in [ROW_URL, ROW_IDENTITY] {
        go_to(&bench, &mut section, row);
        bench.key(&mut section, "c");
        assert!(!section.captures_input(), "no question over nothing");
        assert!(bench.drained().is_empty());
        let shown = frame(&bench, &section);
        let hint = shown.lines().last().unwrap_or_default().to_owned();
        assert!(hint.contains("not stored"), "{hint}");
        bench.key(&mut section, "Esc");
    }

    let unreadable = "the keyring is locked";
    let (bench, mut section, _) = loaded(keyring(
        UrlState::Unreadable(unreadable.to_owned()),
        IdentityState::NotStored,
    ))
    .await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "c");
    assert!(
        !section.captures_input(),
        "an unreadable URL is not offered"
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains(unreadable), "{shown}");
}

#[tokio::test]
async fn c_on_a_half_stored_identity_is_offered() {
    let half = format!(
        "{}: infisical-client-secret is missing",
        secret::HALF_STORED_IDENTITY
    );
    let (bench, mut section, _) = loaded(keyring(
        UrlState::NotStored,
        IdentityState::HalfStored(half.clone()),
    ))
    .await;
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(&half),
        "the seam's sentence verbatim: {shown}"
    );
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "c");
    assert!(section.captures_input(), "clearing a half is the fix");
    bench.key(&mut section, "y");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::ClearMachineIdentity]
    ));
}

#[tokio::test]
async fn one_write_in_flight_refuses_e_and_c_but_not_r_or_t() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert_eq!(requests(&bench).len(), 1);

    for key in ["e", "c"] {
        bench.key(&mut section, key);
        assert!(!section.captures_input(), "`{key}` opens nothing");
        assert!(bench.drained().is_empty(), "`{key}` sends nothing");
        let shown = frame(&bench, &section);
        assert!(
            shown.contains("`clear_infisical_url` is still in flight"),
            "{shown}"
        );
    }
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    assert!(!section.captures_input(), "a project edit waits too");

    bench.key(&mut section, "r");
    let reads = requests(&bench);
    assert!(
        matches!(
            reads.as_slice(),
            [StoreRequest::SecretsInfo, StoreRequest::SecretsTree(ws)] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "{reads:?}"
    );
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::CheckSecretProvider]
    ));
}

#[tokio::test]
async fn t_on_a_keyring_row_sends_one_provider_check() {
    let (bench, mut section, _) = loaded(configured()).await;
    for row in [0, ROW_URL, ROW_IDENTITY, ROW_HEALTH] {
        go_to(&bench, &mut section, row);
        bench.key(&mut section, "t");
        assert!(
            matches!(
                requests(&bench).as_slice(),
                [StoreRequest::CheckSecretProvider]
            ),
            "row {row}"
        );
        assert!(frame(&bench, &section).contains("checking\u{2026}"));
        bench.reply(
            &mut section,
            &StoreReply::SecretCheck(SecretCheck::Provider {
                at: at(14, 2, 11),
                generation: 0,
                outcome: Err(SecretError::NoIdentity),
            }),
        );
    }
}

#[tokio::test]
async fn a_second_t_while_checking_is_refused() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    assert_eq!(requests(&bench).len(), 1);
    bench.key(&mut section, "t");
    assert!(bench.drained().is_empty());
    assert!(frame(&bench, &section).contains("a check is still running"));

    // The answer frees it.
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 0,
            outcome: Ok(ProviderHealth {
                base_url: STORED_URL.to_owned(),
                server_ok: true,
            }),
        }),
    );
    bench.key(&mut section, "t");
    assert_eq!(requests(&bench).len(), 1);
}

#[tokio::test]
async fn t_on_a_provider_less_project_emits_nothing() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "t");
    assert!(bench.drained().is_empty());
    assert!(frame(&bench, &section).contains("this project has no secret scope to check"));
}

#[tokio::test]
async fn t_on_a_scoped_project_sends_a_scope_check() {
    let (bench, mut section, _) = loaded_over(&scoped_store(), configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "t");
    match requests(&bench).as_slice() {
        [StoreRequest::CheckSecretScope { project }] => assert_eq!(*project, ids::PROJECT_VULKAN),
        other => panic!("exactly one scope check, got {other:?}"),
    }
    assert!(frame(&bench, &section).contains("\u{b7} checking\u{2026}"));
}

#[tokio::test]
async fn a_provider_check_shows_on_the_health_row() {
    let (bench, mut section, _) = loaded(configured()).await;
    for (outcome, expected) in [
        (
            Ok(ProviderHealth {
                base_url: STORED_URL.to_owned(),
                server_ok: true,
            }),
            "last check 14:02:11: server ok \u{b7} login ok".to_owned(),
        ),
        (
            Ok(ProviderHealth {
                base_url: STORED_URL.to_owned(),
                server_ok: false,
            }),
            "last check 14:02:11: server status not ok \u{b7} login ok".to_owned(),
        ),
        (
            Err(SecretError::NoIdentity),
            format!("last check 14:02:11: {}", SecretError::NoIdentity),
        ),
    ] {
        bench.reply(
            &mut section,
            &StoreReply::SecretCheck(SecretCheck::Provider {
                at: at(14, 2, 11),
                generation: 0,
                outcome,
            }),
        );
        let shown = frame(&bench, &section);
        let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(&expected), "`{expected}` in {shown}");
    }
}

#[tokio::test]
async fn a_latching_refusal_shows_the_latch_line() {
    let latch = "the last login was refused";
    for (error, latches) in [
        (SecretError::BadCredentials, true),
        (SecretError::IdentityLocked, true),
        (SecretError::LoginRefusedEarlier, true),
        (
            SecretError::Unreachable {
                endpoint: "/api/status",
                cause: "connection refused".to_owned(),
            },
            false,
        ),
    ] {
        let (bench, mut section, _) = loaded(configured()).await;
        bench.reply(
            &mut section,
            &StoreReply::SecretCheck(SecretCheck::Provider {
                at: at(14, 2, 11),
                generation: 0,
                outcome: Err(error.clone()),
            }),
        );
        let shown = frame(&bench, &section);
        assert_eq!(shown.contains(latch), latches, "{error:?}: {shown}");
    }
}

#[tokio::test]
async fn a_landed_identity_write_lifts_the_latch_line_until_the_next_check() {
    // A-4: every landed keyring write rebuilds the provider, so the latch the last check saw is
    // gone; the Health row keeps the check's own outcome, and the next check speaks again.
    let latch = "the last login was refused";
    let refused = |generation| {
        StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation,
            outcome: Err(SecretError::BadCredentials),
        })
    };
    let (bench, mut section, _) = loaded(configured()).await;
    bench.reply(&mut section, &refused(0));
    assert!(frame(&bench, &section).contains(latch));

    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::SetMachineIdentity(_)]
    ));
    assert!(
        frame(&bench, &section).contains(latch),
        "the write has not landed yet"
    );
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    let shown = frame(&bench, &section);
    assert!(!shown.contains(latch), "{shown}");
    assert!(shown.contains("last check 14:02:11"), "{shown}");

    bench.reply(&mut section, &refused(1));
    assert!(frame(&bench, &section).contains(latch));
}

#[tokio::test]
async fn a_refused_write_mark_shows_until_a_write_stores_one() {
    // MOD-90 D3, R1 M-1: the write landed, so its own line says so and nothing is refused; the
    // refused mark is a line under the rows, as the latch line is. A re-read keeps it (it cannot
    // tell whether another process has seen the write); the next landed write that stores a mark
    // drops it, since that mark carries every write before it.
    let notice = "the keyring refused htui/infisical-write-mark";
    let landed = |generation, mark_stored| match configured() {
        StoreReply::Secrets(snapshot) => StoreReply::SecretsWritten {
            request: "set_machine_identity",
            generation,
            mark_stored,
            snapshot,
        },
        other => panic!("expected a keyring snapshot, got {other:?}"),
    };
    let store_identity = |bench: &SectionBench, section: &mut SecretsSection| {
        go_to(bench, section, ROW_IDENTITY);
        bench.key(section, "e");
        type_text(bench, section, CLIENT_ID);
        bench.key(section, "Tab");
        type_text(bench, section, SECRET);
        bench.key(section, "Enter");
        assert!(matches!(
            requests(bench).as_slice(),
            [StoreRequest::SetMachineIdentity(_)]
        ));
    };
    let (bench, mut section, _) = loaded(configured()).await;
    assert!(!frame(&bench, &section).contains(notice));

    store_identity(&bench, &mut section);
    bench.reply(&mut section, &landed(1, false));
    let shown = frame(&bench, &section);
    assert!(shown.contains(notice), "{shown}");
    assert!(
        shown.contains("identity stored"),
        "the write landed: {shown}"
    );
    assert!(bench.errors().is_empty(), "a refused mark is not a failure");

    bench.key(&mut section, "r");
    let _ = bench.drained();
    bench.reply(&mut section, &configured());
    let shown = frame(&bench, &section);
    assert!(shown.contains(notice), "a re-read keeps it: {shown}");

    store_identity(&bench, &mut section);
    bench.reply(&mut section, &landed(2, true));
    let shown = frame(&bench, &section);
    assert!(!shown.contains(notice), "{shown}");
    assert!(shown.contains("identity stored"), "{shown}");
}

#[tokio::test]
async fn a_check_answered_after_a_later_identity_write_does_not_bring_the_latch_back() {
    // The check was sent first, so the loop served it against the identity the write replaced;
    // the write lands before the answer, and A-4 rebuilds on the next `provider()`. Its refusal
    // stays on the Health row as history, and is no latch.
    let latch = "the last login was refused";
    let refused = |generation| {
        StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation,
            outcome: Err(SecretError::BadCredentials),
        })
    };
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::CheckSecretProvider]
    ));
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::SetMachineIdentity(_)]
    ));
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    // Built before the write: generation 0.
    bench.reply(&mut section, &refused(0));
    let shown = frame(&bench, &section);
    assert!(!shown.contains(latch), "{shown}");
    assert!(shown.contains("last check 14:02:11"), "{shown}");

    // The next check speaks again.
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    bench.reply(&mut section, &refused(1));
    assert!(frame(&bench, &section).contains(latch));
}

#[tokio::test]
async fn a_check_sent_after_an_identity_write_latches_on_its_answer() {
    // The write was sent first, so the loop served the check against the new identity: the
    // write landing before the answer does not lift what that answer says.
    let latch = "the last login was refused";
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    assert!(matches!(
        requests(&bench).as_slice(),
        [
            StoreRequest::SetMachineIdentity(_),
            StoreRequest::CheckSecretProvider
        ]
    ));
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 1,
            outcome: Err(SecretError::BadCredentials),
        }),
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains(latch), "{shown}");
}

#[tokio::test]
async fn a_check_that_built_after_a_later_write_latches_on_its_answer() {
    // R1 L-1: the check is served first but runs in a spawned task, which can wait out a walk's
    // keyring read and build its provider after the write sent later has landed. Its answer
    // carries that provider's generation, the write's own: the shared provider is latched now.
    let latch = "the last login was refused";
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    assert!(matches!(
        requests(&bench).as_slice(),
        [
            StoreRequest::CheckSecretProvider,
            StoreRequest::SetMachineIdentity(_)
        ]
    ));
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 1,
            outcome: Err(SecretError::BadCredentials),
        }),
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains(latch), "{shown}");
}

#[tokio::test]
async fn a_write_answered_after_a_check_that_built_after_it_keeps_the_latch() {
    // R1 L-1: the write lands first, but its answer waits on the keyring read-back, so the
    // check's answer can come first. Both carry generation 1: the provider the check latched is
    // the write's own, and the write arriving after does not lift its latch line.
    let latch = "the last login was refused";
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 1,
            outcome: Err(SecretError::BadCredentials),
        }),
    );
    assert!(frame(&bench, &section).contains(latch));
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains("identity stored"), "{shown}");
    assert!(shown.contains(latch), "{shown}");
}

#[tokio::test]
async fn a_scope_check_shows_a_count() {
    let (bench, mut section, _) = loaded_over(&scoped_store(), configured()).await;
    for (count, expected) in [(12, "12 keys visible"), (1, "1 key visible")] {
        bench.reply(
            &mut section,
            &StoreReply::SecretCheck(SecretCheck::Scope {
                project: ids::PROJECT_VULKAN,
                at: at(14, 3, 5),
                outcome: Ok(count),
            }),
        );
        let shown = frame(&bench, &section);
        let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(&format!("checked 14:03:05: {expected}")),
            "{shown}"
        );
    }
    // A project not in the tree is not stored.
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Scope {
            project: ProjectId::new(),
            at: at(14, 4, 0),
            outcome: Ok(99),
        }),
    );
    assert!(!frame(&bench, &section).contains("99 keys"));
}

#[tokio::test]
async fn a_scope_check_answered_after_a_later_scope_write_is_not_kept() {
    // The check was sent first, so the loop read the scope the write replaced: its count is not
    // the new scope's, and the row says nothing until the next check.
    let store = scoped_store();
    let backend = Backend::memory(store.clone());
    let (bench, mut section, _) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "t");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::CheckSecretScope { .. }]
    ));
    bench.key(&mut section, "e");
    bench.key(&mut section, "Tab");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "/v2");
    bench.key(&mut section, "Enter");
    let request = requests(&bench).pop().expect("the write");
    let reply = serve(&backend, &request).await;
    assert!(matches!(
        reply,
        StoreReply::SecretScopeWritten {
            outcome: ScopeWrite::Applied,
            ..
        }
    ));
    bench.reply(&mut section, &reply);
    let check = |count| {
        StoreReply::SecretCheck(SecretCheck::Scope {
            project: ids::PROJECT_VULKAN,
            at: at(14, 3, 5),
            outcome: Ok(count),
        })
    };
    bench.reply(&mut section, &check(12));
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("/app/v2"), "the new scope: {shown}");
    assert!(!flat.contains("12 keys"), "{shown}");
    assert!(!flat.contains("checking"), "{shown}");

    // The next check is kept.
    bench.key(&mut section, "t");
    bench.reply(&mut section, &check(3));
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("checked 14:03:05: 3 keys visible"), "{shown}");
}

#[tokio::test]
async fn a_scope_check_sent_after_a_scope_write_is_kept() {
    // The clear was sent first, so the loop read the cleared row: the answer is the current one.
    let store = scoped_store();
    let backend = Backend::memory(store.clone());
    let (bench, mut section, _) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    bench.key(&mut section, "t");
    let sent = requests(&bench);
    assert!(
        matches!(
            sent.as_slice(),
            [
                StoreRequest::SetProjectSecretScope { scope: None, .. },
                StoreRequest::CheckSecretScope { .. }
            ]
        ),
        "{sent:?}"
    );
    let reply = serve(&backend, &sent[0]).await;
    bench.reply(&mut section, &reply);
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Scope {
            project: ids::PROJECT_VULKAN,
            at: at(14, 3, 5),
            outcome: Err(SecretError::Config("no provider".to_owned())),
        }),
    );
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("checked 14:03:05"), "{shown}");
}

#[tokio::test]
async fn e_on_a_project_opens_the_scope_form_prefilled() {
    // A scoped project: its column's three fields.
    let (bench, mut section, _) = loaded_over(&scoped_store(), configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    assert!(section.captures_input());
    let shown = frame(&bench, &section);
    for expected in ["project ID: p-graphics", "environment: dev", "path: /app"] {
        let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(expected), "`{expected}` in {shown}");
    }

    // An unscoped one: blank, path `/`.
    let (bench, mut section, tree) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p1");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Enter");
    let (id, expected, scope) = the_scope_write(&requests(&bench));
    assert_eq!(id, ids::PROJECT_VULKAN);
    assert_eq!(expected, row_in(&tree, ids::PROJECT_VULKAN).updated_at);
    assert_eq!(
        scope,
        Some(SecretScope::new("p1", "dev", "/").expect("valid"))
    );
}

#[tokio::test]
async fn a_refused_scope_emits_nothing_and_names_the_field() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p1");
    // Environment left empty.
    bench.key(&mut section, "Enter");
    assert!(bench.drained().is_empty());
    assert!(section.captures_input(), "the form is kept");
    let shown = frame(&bench, &section);
    assert!(shown.contains("empty environment"), "{shown}");
}

#[tokio::test]
async fn enter_on_the_scope_form_sends_the_write_with_the_token_and_stays_open() {
    let (bench, mut section, tree) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "app");
    bench.key(&mut section, "Enter");

    let (id, expected, scope) = the_scope_write(&requests(&bench));
    assert_eq!(id, ids::PROJECT_VULKAN);
    assert_eq!(expected, row_in(&tree, ids::PROJECT_VULKAN).updated_at);
    assert_eq!(scope, Some(a_scope()));
    assert!(
        section.captures_input(),
        "the form stays open until the reply"
    );

    bench.key(&mut section, "Enter");
    assert!(
        bench.drained().is_empty(),
        "a second Enter while busy is refused"
    );
}

#[tokio::test]
async fn its_own_applied_write_closes_the_form_and_says_so() {
    let store = MemStore::demo();
    let backend = Backend::memory(store.clone());
    let (bench, mut section, _) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "app");
    bench.key(&mut section, "Enter");
    let request = requests(&bench).pop().expect("the write");
    let reply = serve(&backend, &request).await;
    assert!(matches!(
        reply,
        StoreReply::SecretScopeWritten {
            outcome: ScopeWrite::Applied,
            ..
        }
    ));

    bench.reply(&mut section, &reply);
    assert!(!section.captures_input(), "closed");
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("scope saved"), "{shown}");
    assert!(
        flat.contains("infisical \u{b7} p-graphics \u{b7} dev \u{b7} /app"),
        "the row is the written tree's: {shown}"
    );
}

#[tokio::test]
async fn a_stale_scope_write_keeps_the_text_and_takes_the_new_token() {
    let store = MemStore::demo();
    let (bench, mut section, _) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Enter");
    let (_, first, _) = the_scope_write(&requests(&bench));

    // Someone else moves the row.
    let row = stored(&store, ids::PROJECT_VULKAN).await;
    store
        .update_project(
            ids::PROJECT_VULKAN,
            row.updated_at,
            htui_core::model::ProjectPatch {
                name: Some("Vulkan Elsewhere".to_owned()),
                ..htui_core::model::ProjectPatch::default()
            },
        )
        .await
        .expect("the store writes");
    let current = refreshed(&store).await;
    bench.reply(
        &mut section,
        &scope_written(ids::PROJECT_VULKAN, &current, ScopeWrite::Stale),
    );

    assert!(section.captures_input(), "the form is kept");
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("changed elsewhere since you opened it"),
        "{shown}"
    );
    assert!(flat.contains("p-graphics"), "the text is kept: {shown}");

    bench.key(&mut section, "Enter");
    let (_, second, _) = the_scope_write(&requests(&bench));
    assert_ne!(first, second);
    assert_eq!(second, row_in(&current, ids::PROJECT_VULKAN).updated_at);
}

#[tokio::test]
async fn a_stale_clear_says_nothing_was_written() {
    let store = scoped_store();
    let (bench, mut section, tree) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &scope_written(ids::PROJECT_VULKAN, &tree, ScopeWrite::Stale),
    );
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("changed elsewhere; nothing was written"),
        "{shown}"
    );
}

#[tokio::test]
async fn c_on_a_scoped_project_asks_then_sends_scope_none() {
    let (bench, mut section, tree) = loaded_over(&scoped_store(), configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "c");
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("Remove `vulkan-tutorials`'s secret scope?"),
        "{shown}"
    );
    bench.key(&mut section, "n");
    assert!(bench.drained().is_empty());

    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let (id, expected, scope) = the_scope_write(&requests(&bench));
    assert_eq!(id, ids::PROJECT_VULKAN);
    assert_eq!(expected, row_in(&tree, ids::PROJECT_VULKAN).updated_at);
    assert_eq!(scope, None);

    // A project with no provider has nothing to clear.
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "c");
    assert!(bench.drained().is_empty());
    assert!(frame(&bench, &section).contains("this project has no secret scope to clear"));
}

#[tokio::test]
async fn a_hierarchy_reply_refreshes_rows_but_is_never_taken_as_the_scope_write() {
    let store = MemStore::demo();
    let (bench, mut section, _) = loaded_over(&store, configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Enter");
    let _ = requests(&bench);

    // Another writer's tree, through the plain read and the stale reply.
    store.set_project_secret_columns(
        ids::PROJECT_VULKAN,
        Some(INFISICAL),
        Some(
            &SecretScope::new("p-other", "prod", "/")
                .expect("valid")
                .to_column(),
        ),
    );
    let other = refreshed(&store).await;
    bench.reply(&mut section, &tree_reply(&other));
    bench.reply(
        &mut section,
        &StoreReply::HierarchyStale(Box::new(other.clone())),
    );

    assert!(
        section.captures_input(),
        "the form is still open: not its write's answer"
    );
    let shown = frame(&bench, &section);
    let flat = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("p-other"), "the rows were refreshed: {shown}");
    assert!(!flat.contains("scope saved"), "{shown}");
    bench.key(&mut section, "Enter");
    assert!(bench.drained().is_empty(), "still busy");
}

#[tokio::test]
async fn a_tree_of_another_workspace_is_not_adopted() {
    let (bench, mut section, tree) = loaded(configured()).await;
    let mut elsewhere = tree.clone();
    elsewhere.workspace.id = htui_core::model::WorkspaceId::new();
    elsewhere.projects[0].project.slug = "somewhere-else".to_owned();
    bench.reply(&mut section, &tree_reply(&elsewhere));
    let shown = frame(&bench, &section);
    assert!(shown.contains("vulkan-tutorials"), "{shown}");
    assert!(!shown.contains("somewhere-else"), "{shown}");
}

#[tokio::test]
async fn the_hierarchy_section_adopts_a_scope_write_tree_without_its_attribution() {
    let bench = SectionBench::new().await;
    let mut hierarchy_section = HierarchySection::new();
    let tree = graphics(&MemStore::demo()).await;
    bench.reply(&mut hierarchy_section, &tree_reply(&tree));
    let _ = bench.drained();

    let mut renamed = tree.clone();
    renamed.projects[0].project.name = "Vulkan Renamed".to_owned();
    for outcome in [ScopeWrite::Stale, ScopeWrite::Applied] {
        bench.reply(
            &mut hierarchy_section,
            &scope_written(ids::PROJECT_VULKAN, &renamed, outcome),
        );
        assert!(bench.drained().is_empty(), "{outcome:?}: nothing emitted");
        let shown = bench.render_section(&hierarchy_section, 100);
        assert!(!shown.contains("reloaded; press p again"), "{shown}");
        assert!(
            shown.contains("Vulkan Renamed"),
            "the tree is adopted: {shown}"
        );
    }
}

#[tokio::test]
async fn a_scope_change_drops_an_open_form_and_the_project_rows() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    assert!(section.captures_input());

    section.on_scope_change(&Scope {
        workspace_id: htui_core::model::WorkspaceId::new(),
        project_ids: Vec::new(),
    });
    assert!(!section.captures_input(), "the form went with the scope");
    let shown = frame(&bench, &section);
    assert!(!shown.contains("vulkan-tutorials"), "{shown}");
    assert!(
        shown.contains("no workspace: project scopes need one"),
        "{shown}"
    );
    assert!(
        shown.contains(STORED_URL),
        "the keyring rows survive: {shown}"
    );
}

#[tokio::test]
async fn the_section_debug_holds_no_typed_text() {
    let (bench, mut section, _) = loaded(not_configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    let printed = format!("{section:?}");
    assert!(
        !printed.contains(CLIENT_ID) && !printed.contains(SECRET),
        "{printed}"
    );
    bench.key(&mut section, "Esc");

    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, URL_WITH_PASSWORD);
    let printed = format!("{section:?}");
    assert!(!printed.contains("hunter2"), "{printed}");
    bench.key(&mut section, "Esc");

    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-typed-scope");
    let printed = format!("{section:?}");
    assert!(!printed.contains("p-typed-scope"), "{printed}");
}

#[tokio::test]
async fn a_refused_read_is_unavailable_and_r_recovers() {
    let bench = SectionBench::new().await;
    let mut section = SecretsSection::new();
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "secrets_info",
            message: "keyring task failed: boom".to_owned(),
        },
    );
    let shown = frame(&bench, &section);
    assert!(
        shown.contains("secret settings are unavailable: keyring task failed: boom"),
        "{shown}"
    );
    assert_eq!(shown.lines().last(), Some("r reload"));

    bench.key(&mut section, "r");
    assert_eq!(requests(&bench).len(), 2);
    bench.reply(&mut section, &configured());
    let shown = frame(&bench, &section);
    assert!(!shown.contains("unavailable"), "{shown}");
    assert!(shown.contains(STORED_URL), "{shown}");
}

#[tokio::test]
async fn a_refused_write_lands_on_the_section() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "clear_infisical_url",
            message: "store backend error: cannot remove the keyring entry".to_owned(),
        },
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains("cannot remove the keyring entry"), "{shown}");
    // Not busy any more: `c` asks again.
    bench.key(&mut section, "c");
    assert!(section.captures_input());
    bench.key(&mut section, "Esc");

    // A refused check clears its flight and is not a result.
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: CHECK_SECRET_PROVIDER,
            message: NO_SOURCE_TO_CHECK.to_owned(),
        },
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains(NO_SOURCE_TO_CHECK), "{shown}");
    assert!(shown.contains("not checked this session"), "{shown}");
    bench.key(&mut section, "t");
    assert_eq!(requests(&bench).len(), 1, "the check is free again");

    // Another section's refusal is not this one's.
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_dsn",
            message: "not ours".to_owned(),
        },
    );
    assert!(!frame(&bench, &section).contains("not ours"));
}

#[tokio::test]
async fn a_read_sent_before_a_write_is_not_its_answer() {
    // R1 M-1: the loop serves in order, so a `SecretsInfo` sent before the write answers first,
    // from the keyring the write is about to replace. It is fresh rows, never the write's answer.
    let latch = "the last login was refused";
    let (bench, mut section, _) = loaded(configured()).await;
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 0,
            outcome: Err(SecretError::BadCredentials),
        }),
    );
    bench.key(&mut section, "r");
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    bench.key(&mut section, "Enter");
    let sent = requests(&bench);
    assert!(
        matches!(sent.last(), Some(StoreRequest::SetMachineIdentity(_))),
        "{sent:?}"
    );

    // The read's answer.
    bench.reply(&mut section, &configured());
    let shown = frame(&bench, &section);
    assert!(!shown.contains("identity stored"), "{shown}");
    assert!(shown.contains("set_machine_identity in flight"), "{shown}");
    assert!(shown.contains(latch), "nothing was rebuilt yet: {shown}");
    bench.key(&mut section, "e");
    assert!(!section.captures_input(), "the write is still in flight");
    assert!(
        frame(&bench, &section).contains("`set_machine_identity` is still in flight"),
        "{}",
        frame(&bench, &section)
    );

    // The write's own answer.
    bench.reply(
        &mut section,
        &keyring_landed("set_machine_identity", configured()),
    );
    let shown = frame(&bench, &section);
    assert!(shown.contains("identity stored"), "{shown}");
    assert!(!shown.contains(latch), "{shown}");
    bench.key(&mut section, "e");
    assert!(section.captures_input(), "the write is free again");
}

#[tokio::test]
async fn a_refused_read_does_not_free_a_write_in_flight() {
    // R1 M-1: only a real `SecretsInfo` is refused as `secrets_info`; a write whose read-back
    // fails is refused under its own name.
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_URL);
    bench.key(&mut section, "c");
    bench.key(&mut section, "y");
    assert!(matches!(
        requests(&bench).as_slice(),
        [StoreRequest::ClearInfisicalUrl]
    ));
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "secrets_info",
            message: "keyring task failed: boom".to_owned(),
        },
    );
    // `r` recovers the rows; the write is still out.
    bench.reply(&mut section, &configured());
    for key in ["e", "c"] {
        bench.key(&mut section, key);
        assert!(!section.captures_input(), "`{key}` opens nothing");
        assert!(bench.drained().is_empty(), "`{key}` sends nothing");
        let shown = frame(&bench, &section);
        assert!(
            shown.contains("`clear_infisical_url` is still in flight"),
            "{shown}"
        );
    }
}

#[tokio::test]
async fn a_secrets_reload_reads_its_own_tree() {
    // R1 L-3: `r` re-reads the tree under the section's own name, and the section adopts it.
    let (bench, mut section, tree) = loaded(configured()).await;
    bench.key(&mut section, "r");
    let reads = requests(&bench);
    assert!(
        matches!(
            reads.as_slice(),
            [StoreRequest::SecretsInfo, StoreRequest::SecretsTree(ws)] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "{reads:?}"
    );
    let mut renamed = tree.clone();
    renamed.projects[0].project.slug = "vulkan-renamed".to_owned();
    bench.reply(&mut section, &secrets_tree_reply(&renamed));
    let shown = frame(&bench, &section);
    assert!(shown.contains("vulkan-renamed"), "{shown}");
}

#[tokio::test]
async fn the_hierarchy_section_adopts_a_secrets_tree_without_taking_it_as_its_write() {
    // R1 L-3: a Secrets reload answered while a Hierarchy write is in flight is fresh rows for
    // the Hierarchy section, never the answer that closes its editor.
    let bench = SectionBench::new().await;
    let mut hierarchy_section = HierarchySection::new();
    let tree = graphics(&MemStore::demo()).await;
    bench.reply(&mut hierarchy_section, &tree_reply(&tree));
    let _ = bench.drained();

    bench.key(&mut hierarchy_section, "e");
    for c in "-2".chars() {
        bench.key(&mut hierarchy_section, &c.to_string());
    }
    bench.key(&mut hierarchy_section, "Enter");
    let sent = bench.drained();
    assert!(
        matches!(
            sent.as_slice(),
            [Action::Store(StoreRequest::UpdateWorkspace { .. })]
        ),
        "{sent:?}"
    );

    let mut renamed = tree.clone();
    renamed.workspace.name = "Graphics Renamed".to_owned();
    bench.reply(&mut hierarchy_section, &secrets_tree_reply(&renamed));
    assert!(bench.drained().is_empty(), "nothing emitted");
    assert!(
        hierarchy_section.captures_input(),
        "the editor waits for its own write's answer"
    );
    let shown = bench.render_section(&hierarchy_section, 100);
    assert!(
        shown.contains("Graphics Renamed"),
        "the tree is adopted: {shown}"
    );

    // Its own write's answer still closes it.
    bench.reply(&mut hierarchy_section, &tree_reply(&renamed));
    assert!(!hierarchy_section.captures_input());
}

#[tokio::test]
async fn a_scope_write_refused_for_a_deleted_project_closes_the_form_on_the_re_read() {
    // R1 L-4: a project deleted elsewhere is refused before the CAS write (`NotFound`), never
    // answered `Stale`. The refusal re-reads the tree; the tree without the project closes the
    // form, rather than leaving every `Enter` to be refused again.
    let (bench, mut section, tree) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, "dev");
    bench.key(&mut section, "Enter");
    let _ = the_scope_write(&requests(&bench));

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: SET_PROJECT_SECRET_SCOPE,
            message: StoreError::NotFound {
                entity: "workspace_project",
                id: ids::PROJECT_VULKAN.to_string(),
            }
            .to_string(),
        },
    );
    assert!(section.captures_input(), "a refusal alone keeps the text");
    let reread = requests(&bench);
    assert!(
        matches!(
            reread.as_slice(),
            [StoreRequest::SecretsTree(ws)] if *ws == ids::WORKSPACE_GRAPHICS
        ),
        "{reread:?}"
    );

    bench.reply(
        &mut section,
        &secrets_tree_reply(&without(&tree, ids::PROJECT_VULKAN)),
    );
    assert!(!section.captures_input(), "the form closed");
    let shown = frame(&bench, &section);
    assert!(shown.contains("deleted elsewhere"), "{shown}");
    assert!(!shown.contains("vulkan-tutorials"), "{shown}");
}

#[tokio::test]
async fn a_tree_without_the_open_project_closes_its_form_or_question() {
    // R1 L-4: any fresh tree of the scope's workspace that no longer holds the project ends the
    // form or the question; its token could only be refused.
    let (bench, mut section, tree) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, "p-graphics");
    bench.reply(
        &mut section,
        &tree_reply(&without(&tree, ids::PROJECT_VULKAN)),
    );
    assert!(!section.captures_input(), "the form closed");
    assert!(frame(&bench, &section).contains("deleted elsewhere"));
    assert!(bench.drained().is_empty());

    let (bench, mut section, tree) = loaded_over(&scoped_store(), configured()).await;
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "c");
    assert!(section.captures_input(), "the question is open");
    bench.reply(
        &mut section,
        &secrets_tree_reply(&without(&tree, ids::PROJECT_VULKAN)),
    );
    assert!(!section.captures_input(), "the question closed");
    bench.key(&mut section, "y");
    assert!(bench.drained().is_empty(), "nothing is cleared");
    assert!(frame(&bench, &section).contains("deleted elsewhere"));
}

#[tokio::test]
async fn demo_refuses_keyring_edits_and_checks_but_edits_scopes() {
    let (bench, mut section, _) = loaded(demo_keyring()).await;
    let shown = frame(&bench, &section);
    assert!(shown.contains("n/a in a demo session"), "{shown}");

    for row in [ROW_URL, ROW_IDENTITY] {
        go_to(&bench, &mut section, row);
        bench.key(&mut section, "e");
        assert!(!section.captures_input(), "row {row}");
        assert!(
            frame(&bench, &section).contains("a demo session never reads or writes the keyring")
        );
        bench.key(&mut section, "Esc");
    }
    go_to(&bench, &mut section, ROW_HEALTH);
    bench.key(&mut section, "t");
    assert!(bench.drained().is_empty(), "no check in a demo");
    assert!(frame(&bench, &section).contains("a demo session has no secret provider to check"));
    bench.key(&mut section, "Esc");

    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    bench.key(&mut section, "e");
    assert!(
        section.captures_input(),
        "scopes live in the store, not the keyring"
    );
}

#[tokio::test]
async fn the_product_registers_secrets_last() {
    let mut harness = Harness::demo();
    htui::app::register_all(harness.app());
    harness.settle().await;
    harness.key("4");
    harness.settle().await;
    for _ in 0..8 {
        harness.key("l");
    }
    harness.settle().await;

    let frame = harness.render();
    assert!(frame.contains(" Personas  Secrets "), "{frame}");
    assert!(
        frame.contains("e edit \u{b7} c clear \u{b7} t check \u{b7} r reload \u{b7} j/k rows"),
        "the active section is Secrets: {frame}"
    );
    assert!(frame.contains("n/a in a demo session"), "{frame}");
}

// ---------------------------------------------------------------------------------------------
// Snapshots (D.4 #31)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_not_configured() {
    let (bench, section, _) = loaded(not_configured()).await;
    insta::assert_snapshot!("not_configured", frame(&bench, &section));
}

#[tokio::test]
async fn snapshot_configured_health_ok() {
    let (bench, mut section, _) = loaded(configured()).await;
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 0,
            outcome: Ok(ProviderHealth {
                base_url: STORED_URL.to_owned(),
                server_ok: true,
            }),
        }),
    );
    go_to(&bench, &mut section, ROW_HEALTH);
    insta::assert_snapshot!("configured_health_ok", frame(&bench, &section));
}

#[tokio::test]
async fn snapshot_health_refused() {
    let (bench, mut section, _) = loaded(configured()).await;
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Provider {
            at: at(14, 2, 11),
            generation: 0,
            outcome: Err(SecretError::BadCredentials),
        }),
    );
    insta::assert_snapshot!("health_refused", frame(&bench, &section));
}

#[tokio::test]
async fn snapshot_project_scope_checked() {
    let (bench, mut section, _) = loaded_over(&scoped_store(), configured()).await;
    bench.reply(
        &mut section,
        &StoreReply::SecretCheck(SecretCheck::Scope {
            project: ids::PROJECT_VULKAN,
            at: at(14, 3, 5),
            outcome: Ok(12),
        }),
    );
    go_to(&bench, &mut section, ROW_FIRST_PROJECT);
    insta::assert_snapshot!("project_scope_checked", frame(&bench, &section));
}

#[tokio::test]
async fn snapshot_identity_form() {
    let (bench, mut section, _) = loaded(configured()).await;
    go_to(&bench, &mut section, ROW_IDENTITY);
    bench.key(&mut section, "e");
    type_text(&bench, &mut section, CLIENT_ID);
    bench.key(&mut section, "Tab");
    type_text(&bench, &mut section, SECRET);
    insta::assert_snapshot!("identity_form", frame(&bench, &section));
}

#[tokio::test]
async fn snapshot_demo() {
    let (bench, section, _) = loaded(demo_keyring()).await;
    insta::assert_snapshot!("demo", frame(&bench, &section));
}
