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
use htui::hierarchy::{self, HierarchySnapshot, ScopeWrite};
use htui::qdrant_settings_info::{QdrantSnapshot, QdrantState};
use htui::secrets_settings::{
    CHECK_SECRET_PROVIDER, CHECK_SECRET_SCOPE, DEMO_SESSION, IDENTITY_INCOMPLETE, IdentityEntry,
    IdentityState, NO_PROVIDER_TO_CHECK, NO_SOURCE_TO_CHECK, Redacted, SET_PROJECT_SECRET_SCOPE,
    SecretCheck, SecretsSnapshot, UrlState,
};
use htui::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::QdrantSection;
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{NewProject, Project, ProjectId};
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
