//! `Settings > Queue`, from the worker side out (MOD-12 milestone 2, D8-D10).
//!
//! The worker half drives `htui::store_worker::serve` directly over a `Backend`, exactly as
//! `tests/prompt_settings.rs` does: one request in, one reply out, no channels and no shell. The
//! section half follows it below through a `SectionBench`, which hands the section the snapshots
//! the worker answered and reads back the requests it emitted.
//!
//! The demo world behind a memory backend starts with no `app_setting` row (Postgres seeds some,
//! blueprint H-8), so every `App` entry here is unset until a test writes one, and its token is
//! `Stamp(None)`.
#![cfg(feature = "testkit")]

use std::collections::BTreeSet;

use chrono::Utc;
use htui::queue_settings::{self, QueueSettingsSnapshot, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{DEFAULT_MAX_CONCURRENT_ITEMS, QueueSetting, Scope, WorkspaceId};
use htui_core::store::{MemStore, QueueTarget, QueueToken, StoreError};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use serde_json::{Value, json};

/// The demo world behind a memory backend.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The scope the serve half reads: the Platform workspace narrowed to the `htui` project.
fn htui_scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI],
    }
}

/// The empty scope every name-only request is built against.
fn nil_scope() -> Scope {
    Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: Vec::new(),
    }
}

/// The snapshot an applied reply carries, or a panic naming what came back instead.
#[track_caller]
fn settings(reply: StoreReply) -> QueueSettingsSnapshot {
    match reply {
        StoreReply::QueueSettings(snapshot) => *snapshot,
        other => panic!("expected queue settings: {other:?}"),
    }
}

/// The snapshot a compare-and-set miss carries, or a panic naming what came back instead.
#[track_caller]
fn stale(reply: StoreReply) -> QueueSettingsSnapshot {
    match reply {
        StoreReply::QueueSettingsStale(snapshot) => *snapshot,
        other => panic!("expected a stale reply: {other:?}"),
    }
}

/// The `Failed` reply's `(request, message)`, or a panic naming what came back instead.
#[track_caller]
fn refusal(reply: StoreReply) -> (&'static str, String) {
    match reply {
        StoreReply::Failed { request, message } => (request, message),
        other => panic!("expected a refusal: {other:?}"),
    }
}

/// One read of the `htui` scope.
async fn demo_settings(backend: &Backend) -> QueueSettingsSnapshot {
    settings(serve(backend, &StoreRequest::QueueSettings(htui_scope())).await)
}

/// The `htui` project's token as the snapshot carries it.
#[track_caller]
fn project_token(snapshot: &QueueSettingsSnapshot) -> QueueToken {
    QueueToken::Stamp(Some(snapshot.projects[0].project.updated_at))
}

/// The token of one `App` entry.
#[track_caller]
fn app_token(snapshot: &QueueSettingsSnapshot, key: QueueSetting) -> QueueToken {
    snapshot
        .app
        .iter()
        .find(|entry| entry.key == key)
        .expect("the snapshot carries all three app keys")
        .token
}

/// One write through the worker, against the `htui` scope.
fn set(target: QueueTarget, key: QueueSetting, value: Value, expected: QueueToken) -> StoreRequest {
    StoreRequest::SetQueueSetting {
        scope: htui_scope(),
        target,
        key,
        value,
        expected,
    }
}

/// One clear through the worker, against the `htui` scope.
fn clear(target: QueueTarget, key: QueueSetting, expected: QueueToken) -> StoreRequest {
    StoreRequest::ClearQueueSetting {
        scope: htui_scope(),
        target,
        key,
        expected,
    }
}

/// One of each of the three, in [`REQUEST_NAMES`] order. The scope is nil where the request never
/// reaches a store.
fn queue_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::QueueSettings(nil_scope()),
        StoreRequest::SetQueueSetting {
            scope: nil_scope(),
            target: QueueTarget::App,
            key: QueueSetting::MaxConcurrentItems,
            value: json!(1),
            expected: QueueToken::Stamp(None),
        },
        StoreRequest::ClearQueueSetting {
            scope: nil_scope(),
            target: QueueTarget::App,
            key: QueueSetting::MaxConcurrentItems,
            expected: QueueToken::Stamp(Some(Utc::now())),
        },
    ]
}

/// `REQUEST_NAMES` is what `StoreRequest::name` answers for the three, in variant order, and no
/// two share a name: the section's `Failed` routing is by name alone.
#[test]
fn queue_settings_names_are_stable() {
    let names: Vec<&'static str> = queue_requests().iter().map(StoreRequest::name).collect();

    assert_eq!(names, REQUEST_NAMES);
    assert_eq!(queue_settings::READ_NAME, "queue_settings");
    let unique: BTreeSet<&'static str> = names.iter().copied().collect();
    assert_eq!(unique.len(), REQUEST_NAMES.len(), "three distinct names");
}

/// The shape of one read (D8): the three `app_setting` keys in `APP_KEYS` order, all unset on a
/// fresh `MemStore`; the scope's one project with neither cap; and this box with its seeded
/// `max_concurrent_items` of 2, which is also what it admits.
#[tokio::test]
async fn the_demo_snapshot_has_three_app_keys_each_scope_project_and_this_box() {
    let snapshot = demo_settings(&demo()).await;

    let keys: Vec<QueueSetting> = snapshot.app.iter().map(|entry| entry.key).collect();
    assert_eq!(keys, QueueSetting::APP_KEYS);
    for entry in &snapshot.app {
        assert_eq!(entry.value, None, "`{}` holds nothing", entry.key);
        assert_eq!(
            entry.token,
            QueueToken::Stamp(None),
            "`{}` has no row",
            entry.key
        );
    }
    assert!(snapshot.app_map().is_empty());
    assert_eq!(snapshot.app_limit(), DEFAULT_MAX_CONCURRENT_ITEMS);

    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.project.id, ids::PROJECT_HTUI);
    assert_eq!(project.run_cap, None);
    assert_eq!(project.batch_cap, None);

    let this_box = snapshot
        .this_box
        .as_ref()
        .expect("the demo box is registered");
    assert_eq!(this_box.id, ids::BOX);
    assert_eq!(this_box.hostname, "DESKTOP-HTUI");
    assert_eq!(this_box.value, Some(json!(2)));
    assert_eq!(this_box.token, QueueToken::EditVersion(0));
    assert_eq!(this_box.effective, 2);
}

/// A project cap lands in `project.settings` as micros, under the project's `updated_at`, and the
/// blob's other keys survive the merge.
#[tokio::test]
async fn set_queue_setting_on_a_project_stores_micros_and_keeps_foreign_keys() {
    let backend = demo();
    let before = demo_settings(&backend).await;

    let after = settings(
        serve(
            &backend,
            &set(
                QueueTarget::Project(ids::PROJECT_HTUI),
                QueueSetting::PerTokenCapBatch,
                json!(1_500_000),
                project_token(&before),
            ),
        )
        .await,
    );

    let project = &after.projects[0];
    assert_eq!(project.batch_cap, Some(json!(1_500_000)));
    assert_eq!(project.run_cap, None);
    assert_eq!(
        project.project.settings.get("token_budget"),
        Some(&json!(120_000)),
        "the merge keeps every other key: {:?}",
        project.project.settings
    );
    assert_ne!(
        project_token(&after),
        project_token(&before),
        "the token moved"
    );
}

/// An `app_setting` key is upserted with `Stamp(None)` for "no row yet", and its clear takes the
/// entry back to unset with no token.
#[tokio::test]
async fn clear_queue_setting_on_app_returns_the_entry_to_none() {
    let backend = demo();
    let key = QueueSetting::MinBudgetForNewAttempt;

    let written = settings(
        serve(
            &backend,
            &set(
                QueueTarget::App,
                key,
                json!(500_000),
                QueueToken::Stamp(None),
            ),
        )
        .await,
    );
    assert_eq!(written.app[0].value, Some(json!(500_000)));
    let token = app_token(&written, key);
    assert!(matches!(token, QueueToken::Stamp(Some(_))), "{token:?}");

    let cleared = settings(serve(&backend, &clear(QueueTarget::App, key, token)).await);

    assert_eq!(cleared.app[0].value, None);
    assert_eq!(cleared.app[0].token, QueueToken::Stamp(None));
}

/// A write against a token another write already spent answers `QueueSettingsStale`, carrying
/// the settings as they are now; nothing of the stale write lands.
#[tokio::test]
async fn a_spent_token_answers_queue_settings_stale() {
    let backend = demo();
    let before = demo_settings(&backend).await;
    let spent = project_token(&before);
    let target = QueueTarget::Project(ids::PROJECT_HTUI);

    let _ = settings(
        serve(
            &backend,
            &set(
                target,
                QueueSetting::PerTokenCapRun,
                json!(1_000_000),
                spent,
            ),
        )
        .await,
    );
    let now = stale(
        serve(
            &backend,
            &set(
                target,
                QueueSetting::PerTokenCapRun,
                json!(9_000_000),
                spent,
            ),
        )
        .await,
    );

    assert_eq!(
        now.projects[0].run_cap,
        Some(json!(1_000_000)),
        "the first write stands"
    );
    assert_ne!(
        project_token(&now),
        spent,
        "the reload carries the current token"
    );
}

/// The box limit is a key of `box.settings`, guarded by `edit_version`, which each write bumps;
/// cleared, the box inherits the app default and the snapshot says what that comes to.
#[tokio::test]
async fn the_box_limit_writes_box_settings_under_edit_version() {
    let backend = demo();
    let key = QueueSetting::MaxConcurrentItems;
    let target = QueueTarget::Box(ids::BOX);

    let raised = settings(
        serve(
            &backend,
            &set(target, key, json!(3), QueueToken::EditVersion(0)),
        )
        .await,
    );
    let this_box = raised.this_box.as_ref().expect("this box");
    assert_eq!(this_box.value, Some(json!(3)));
    assert_eq!(this_box.token, QueueToken::EditVersion(1));
    assert_eq!(this_box.effective, 3);

    let cleared = settings(serve(&backend, &clear(target, key, QueueToken::EditVersion(1))).await);
    let this_box = cleared.this_box.as_ref().expect("this box");
    assert_eq!(this_box.value, None);
    assert_eq!(this_box.token, QueueToken::EditVersion(2));
    assert_eq!(
        this_box.effective,
        cleared.app_limit(),
        "it inherits the app default"
    );

    // The app default moves, and the inheriting box follows it.
    let app = settings(
        serve(
            &backend,
            &set(QueueTarget::App, key, json!(5), QueueToken::Stamp(None)),
        )
        .await,
    );
    assert_eq!(app.app_limit(), 5);
    assert_eq!(app.this_box.as_ref().expect("this box").effective, 5);
}

/// The store's validator is the last word: a value the section would refuse is refused here too,
/// by its own sentence, and nothing is written.
#[tokio::test]
async fn an_invalid_value_is_refused_with_the_validators_sentence() {
    let backend = demo();

    let (request, message) = refusal(
        serve(
            &backend,
            &set(
                QueueTarget::Box(ids::BOX),
                QueueSetting::MaxConcurrentItems,
                json!(0),
                QueueToken::EditVersion(0),
            ),
        )
        .await,
    );

    assert_eq!(request, "set_queue_setting");
    let sentence = QueueSetting::MaxConcurrentItems
        .validate(&json!(0))
        .expect_err("0 is refused");
    assert!(message.contains(&sentence), "{message}");
    let after = demo_settings(&backend).await;
    assert_eq!(after.this_box.expect("this box").value, Some(json!(2)));
}

/// Offline, each of the three is refused by its own name with MOD-25's sentence, the one every
/// orchestration request gets off the server: `Backend::writer()` is `None`, the read included.
#[tokio::test]
async fn offline_refuses_all_three_by_name() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "queue-settings-offline",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    for (request, name) in queue_requests().into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
        );
    }
}

/// `queue_settings::serve` is reachable only through `try_serve`'s or-ed patterns, so a request
/// from anywhere else is told which one it sent rather than panicking.
#[tokio::test]
async fn serve_refuses_a_foreign_request_by_name() {
    let err = queue_settings::serve(&demo(), &StoreRequest::Catalogue(nil_scope()))
        .await
        .expect_err("a catalogue request is not this module's");

    assert_eq!(
        err,
        StoreError::Backend("not a queue settings request: catalogue".to_owned())
    );
}
