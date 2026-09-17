//! `Settings > Prompt`, from the worker side out (MOD-15 milestone 5, D16).
//!
//! The worker half drives `htui::store_worker::serve` directly over a `Backend`, exactly as
//! `tests/kinds.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 5's task 2 and lands below this one.
//!
//! The last case is Postgres-gated and returns without asserting when
//! `HTUI_TEST_DATABASE_URL` is unset, as every other Postgres-backed suite in this crate does: it
//! is the only store where the ten `app_setting` rows exist before anything writes one
//! (`0002_agent_probe.sql:68-79`), and therefore the only one where a set carries a token rather
//! than `expected: None`.
#![cfg(feature = "testkit")]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use htui::prompt_settings::{self, REQUEST_NAMES, SettingsSnapshot, project_keys};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_core::fixtures::ids;
use htui_core::model::{ProjectId, Scope, WorkspaceId};
use htui_core::prompt::{DEFAULTS, SettingKey};
use htui_core::store::{MemStore, SettingRung, StoreError};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use serde_json::{Value, json};

/// The demo world behind a memory backend: `MemStore` starts with an empty `app_settings` map and
/// the fixture seeds none (F-7), so every `App` entry is `None` until a test writes one.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The scope the section renders in the demo world: the Graphics workspace and its one project.
fn vulkan_scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    }
}

/// The empty scope every name-only request is built against: the three names are a property of
/// the enum, not of a store.
fn nil_scope() -> Scope {
    Scope {
        workspace_id: WorkspaceId::default(),
        project_ids: Vec::new(),
    }
}

/// The snapshot an applied reply carries, or a panic naming what came back instead.
#[track_caller]
fn settings(reply: StoreReply) -> SettingsSnapshot {
    match reply {
        StoreReply::PromptSettings(snapshot) => *snapshot,
        other => panic!("expected prompt settings: {other:?}"),
    }
}

/// The snapshot a compare-and-set miss carries, or a panic naming what came back instead.
#[track_caller]
fn stale(reply: StoreReply) -> SettingsSnapshot {
    match reply {
        StoreReply::PromptSettingsStale(snapshot) => *snapshot,
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

/// One read of the demo scope.
async fn demo_settings(backend: &Backend) -> SettingsSnapshot {
    settings(serve(backend, &StoreRequest::PromptSettings(vulkan_scope())).await)
}

/// The `App` rung's compare-and-set token for one key, or a panic: a test that asks for it has
/// just applied a write, so the entry must carry one.
#[track_caller]
fn app_token(snapshot: &SettingsSnapshot, key: SettingKey) -> DateTime<Utc> {
    snapshot
        .app_entry(key)
        .expect("the snapshot carries all ten keys")
        .updated_at
        .expect("an applied write leaves the row a token")
}

/// A set of `token_budget` on the `App` rung against the token held (or `None` for "no row").
fn set_budget(expected: Option<DateTime<Utc>>) -> StoreRequest {
    StoreRequest::SetSetting {
        scope: vulkan_scope(),
        rung: SettingRung::App,
        key: SettingKey::TokenBudget,
        value: json!(5_000),
        expected,
    }
}

/// One of each of the three, in [`REQUEST_NAMES`] order. The scope is nil where the request never
/// reaches a store.
fn prompt_requests() -> Vec<StoreRequest> {
    vec![
        StoreRequest::PromptSettings(nil_scope()),
        StoreRequest::SetSetting {
            scope: nil_scope(),
            rung: SettingRung::App,
            key: SettingKey::TokenBudget,
            value: json!(1),
            expected: None,
        },
        StoreRequest::ClearSetting {
            scope: nil_scope(),
            rung: SettingRung::App,
            key: SettingKey::TokenBudget,
            expected: Utc::now(),
        },
    ]
}

/// `REQUEST_NAMES` is what `StoreRequest::name` answers for the three, in variant order, and no
/// two of them share a name — the `Failed` routing the section does is by name alone (D7).
#[test]
fn prompt_settings_names_are_stable() {
    let names: Vec<&'static str> = prompt_requests().iter().map(StoreRequest::name).collect();

    assert_eq!(names, REQUEST_NAMES);

    let unique: BTreeSet<&'static str> = names.iter().copied().collect();
    assert_eq!(unique.len(), REQUEST_NAMES.len(), "three distinct names");
}

/// The shape of one read (D3/D4): ten `App` entries in `SettingKey::ALL` order, all empty on a
/// fresh `MemStore` (F-7), and one project entry per scope project carrying exactly the keys the
/// `Project` rung accepts, in [`project_keys`] order.
///
/// The demo project's own blob already holds `token_budget`, so the second project value is a
/// stored number rather than `None` — which is what makes the `project_key` mapping of D2
/// observable from the first read.
#[tokio::test]
async fn the_demo_snapshot_has_ten_app_entries_and_two_keys_per_project() {
    let snapshot = demo_settings(&demo()).await;

    assert_eq!(snapshot.app.len(), 10);
    let keys: Vec<SettingKey> = snapshot.app.iter().map(|entry| entry.key).collect();
    assert_eq!(keys, SettingKey::ALL);
    for entry in &snapshot.app {
        assert_eq!(entry.value, None, "`{:?}` holds nothing", entry.key);
        assert_eq!(entry.updated_at, None, "`{:?}` has no row", entry.key);
    }

    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.project.id, ids::PROJECT_VULKAN);

    let value_keys: Vec<SettingKey> = project.values.iter().map(|value| value.key).collect();
    assert_eq!(value_keys, project_keys().collect::<Vec<_>>());
    assert_eq!(
        value_keys,
        vec![SettingKey::UpstreamHops, SettingKey::TokenBudget]
    );
    assert_eq!(project.values[0].value, None);
    assert_eq!(project.values[1].value, Some(json!(120_000)));
}

/// `app_map` is the shape `Backend::app_settings` hands the resolvers: only the entries that carry
/// a value, keyed by `SettingSpec::key` (D6).
#[tokio::test]
async fn app_map_holds_only_entries_with_a_value() {
    let backend = demo();

    let snapshot = settings(
        serve(
            &backend,
            &StoreRequest::SetSetting {
                scope: vulkan_scope(),
                rung: SettingRung::App,
                key: SettingKey::MaxSkillTokens,
                value: json!(4_000),
                expected: None,
            },
        )
        .await,
    );

    let map = snapshot.app_map();
    assert_eq!(map.len(), 1, "nine entries hold nothing: {map:?}");
    assert_eq!(map.get("max_skill_tokens"), Some(&json!(4_000)));
}

/// A scope id that names no project is skipped rather than failing the read (D3), the same
/// treatment `catalogue::snapshot` gives a torn read.
#[tokio::test]
async fn an_unknown_project_id_is_skipped() {
    let mut scope = vulkan_scope();
    scope.project_ids.push(ProjectId::new());

    let snapshot = settings(serve(&demo(), &StoreRequest::PromptSettings(scope)).await);

    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].project.id, ids::PROJECT_VULKAN);
}

/// `expected: None` is "I expect no row" and applies only while there is none (F-2): the second
/// set of the same shape is a compare-and-set miss, and so is a set re-using a token the applied
/// write has already replaced (PRD D8).
#[tokio::test]
async fn set_setting_on_app_with_no_row_applies_then_the_old_token_is_stale() {
    let backend = demo();

    let applied = settings(serve(&backend, &set_budget(None)).await);
    let entry = applied
        .app_entry(SettingKey::TokenBudget)
        .expect("the snapshot carries all ten keys");
    assert_eq!(entry.value, Some(json!(5_000)));
    let first = app_token(&applied, SettingKey::TokenBudget);

    let missed = stale(serve(&backend, &set_budget(None)).await);
    assert_eq!(
        app_token(&missed, SettingKey::TokenBudget),
        first,
        "a miss changes nothing"
    );

    let again = settings(serve(&backend, &set_budget(Some(first))).await);
    let second = app_token(&again, SettingKey::TokenBudget);
    assert_ne!(second, first, "an applied write moves the token");

    let dead = stale(serve(&backend, &set_budget(Some(first))).await);
    assert_eq!(app_token(&dead, SettingKey::TokenBudget), second);
}

/// A clear returns the entry to the state D4 gives its own type: no value and no token, which is
/// the state the next set must pass `expected: None` for.
#[tokio::test]
async fn clear_setting_on_app_returns_the_entry_to_none() {
    let backend = demo();
    let set = StoreRequest::SetSetting {
        scope: vulkan_scope(),
        rung: SettingRung::App,
        key: SettingKey::MaxSkillTokens,
        value: json!(4_000),
        expected: None,
    };

    let applied = settings(serve(&backend, &set).await);
    let token = app_token(&applied, SettingKey::MaxSkillTokens);

    let cleared = settings(
        serve(
            &backend,
            &StoreRequest::ClearSetting {
                scope: vulkan_scope(),
                rung: SettingRung::App,
                key: SettingKey::MaxSkillTokens,
                expected: token,
            },
        )
        .await,
    );
    let entry = cleared
        .app_entry(SettingKey::MaxSkillTokens)
        .expect("the snapshot carries all ten keys");
    assert_eq!(entry.value, None);
    assert_eq!(entry.updated_at, None);
    assert!(cleared.app_map().is_empty());

    let reset = settings(serve(&backend, &set).await);
    assert_eq!(
        reset
            .app_entry(SettingKey::MaxSkillTokens)
            .expect("the snapshot carries all ten keys")
            .value,
        Some(json!(4_000)),
        "the rung is writable again with `expected: None`"
    );
}

/// H-1, pinned rather than discovered later: on the `App` rung a token for a row that was
/// **cleared** elsewhere is a `NotFound`, not a `Stale` — there is no row to hand back — so the
/// section's reload-and-retry habit never runs and the seam's own sentence is what it shows.
///
/// Open item O-3 names the seam change (a `Stale` able to carry "no row") that would close it.
#[tokio::test]
async fn a_token_over_a_cleared_app_row_answers_failed_not_found() {
    let backend = demo();

    let applied = settings(serve(&backend, &set_budget(None)).await);
    let token = app_token(&applied, SettingKey::TokenBudget);
    let _ = settings(
        serve(
            &backend,
            &StoreRequest::ClearSetting {
                scope: vulkan_scope(),
                rung: SettingRung::App,
                key: SettingKey::TokenBudget,
                expected: token,
            },
        )
        .await,
    );

    let (request, message) = refusal(serve(&backend, &set_budget(Some(token))).await);

    assert_eq!(request, "set_setting");
    assert!(
        message.contains("app_setting"),
        "the sentence names the table: {message}"
    );
    assert!(
        message.contains("not found"),
        "a dead token is a missing row, not a stale one: {message}"
    );
}

/// M1's live coordinate 2, from the request layer: `prompt_upstream_hops` is stored in a project
/// under `upstream_hops` because `setting()` applies `SettingSpec::project_key` (D2), and the
/// key-level merge leaves the blob's other keys alone (PRD `:370`).
#[tokio::test]
async fn set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys() {
    let backend = demo();

    let before = demo_settings(&backend).await;
    let project = &before.projects[0];
    let token = project.project.updated_at;
    // The fixture's blob already carries keys this milestone never writes, so the merge is
    // observable without seeding one first.
    assert_eq!(
        project.project.settings.get("retention_days"),
        Some(&Value::Null)
    );
    assert_eq!(
        project.project.settings.get("keep_raw_events"),
        Some(&json!(false))
    );

    let after = settings(
        serve(
            &backend,
            &StoreRequest::SetSetting {
                scope: vulkan_scope(),
                rung: SettingRung::Project(ids::PROJECT_VULKAN),
                key: SettingKey::UpstreamHops,
                value: json!(2),
                expected: Some(token),
            },
        )
        .await,
    );

    let entry = &after.projects[0];
    let blob = &entry.project.settings;
    assert_eq!(blob.get("upstream_hops"), Some(&json!(2)));
    assert_eq!(
        blob.get("prompt_upstream_hops"),
        None,
        "the App spelling is a key `resolve_hops` never looks at: {blob}"
    );
    assert_eq!(blob.get("retention_days"), Some(&Value::Null));
    assert_eq!(blob.get("keep_raw_events"), Some(&json!(false)));
    assert_eq!(blob.get("token_budget"), Some(&json!(120_000)));

    assert_eq!(entry.values[0].key, SettingKey::UpstreamHops);
    assert_eq!(entry.values[0].value, Some(json!(2)));
    assert_ne!(
        entry.project.updated_at, token,
        "the project's own token is what a project-rung write moves"
    );
}

/// Every bound is the store's (D11): the section checks shape only, and the range sentence reaches
/// the editor with the key and the range in it.
#[tokio::test]
async fn an_out_of_range_value_is_refused_with_the_seams_sentence() {
    let (request, message) = refusal(
        serve(
            &demo(),
            &StoreRequest::SetSetting {
                scope: vulkan_scope(),
                rung: SettingRung::App,
                key: SettingKey::TokenBudget,
                value: json!(0),
                expected: None,
            },
        )
        .await,
    );

    assert_eq!(request, "set_setting");
    assert!(
        message.contains("`token_budget` = 0 is outside 1..="),
        "the key and the floor: {message}"
    );
    assert!(message.contains("tokens"), "and the unit: {message}");
}

/// H-10: a fraction typed as the basis points its range is written in is refused by the rounding
/// the reader does, and the refusal names both numbers — which is why D12 renders both units.
#[tokio::test]
async fn a_fraction_typed_as_basis_points_is_refused_by_rounding() {
    let (request, message) = refusal(
        serve(
            &demo(),
            &StoreRequest::SetSetting {
                scope: vulkan_scope(),
                rung: SettingRung::App,
                key: SettingKey::PromptReserveFraction,
                value: json!(1_000.0),
                expected: None,
            },
        )
        .await,
    );

    assert_eq!(request, "set_setting");
    assert!(
        message.contains("rounds to 10000000 bp, outside 0..=5000 bp"),
        "the rounding is what refuses it: {message}"
    );
}

/// `rung_refusal` is the seam's own gate (D9): a key offered on a rung its spec refuses answers
/// that sentence, whatever the token.
#[tokio::test]
async fn a_key_on_a_rung_its_spec_refuses_answers_the_rung_sentence() {
    let backend = demo();
    let token = demo_settings(&backend).await.projects[0].project.updated_at;

    let (request, message) = refusal(
        serve(
            &backend,
            &StoreRequest::SetSetting {
                scope: vulkan_scope(),
                rung: SettingRung::Project(ids::PROJECT_VULKAN),
                key: SettingKey::MaxSkillTokens,
                value: json!(1),
                expected: Some(token),
            },
        )
        .await,
    );

    assert_eq!(request, "set_setting");
    assert!(
        message.contains("`max_skill_tokens` is not accepted on the project rung"),
        "the seam's own refusal: {message}"
    );
}

/// Offline, each of the three is refused by its own name with MOD-25's one sentence:
/// `Backend::writer()` is `None`, so `serve` never reaches the seam — the read included.
#[tokio::test]
async fn offline_refuses_all_three_by_name() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "prompt-settings-offline",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    for (request, name) in prompt_requests().into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
        );
    }
}

/// `prompt_settings::serve` is reachable only through `try_serve`'s three or-ed patterns, so a
/// request from anywhere else is told which one it sent rather than panicking.
#[tokio::test]
async fn serve_refuses_a_foreign_request_by_name() {
    let err = prompt_settings::serve(&demo(), &StoreRequest::Catalogue(nil_scope()))
        .await
        .expect_err("a catalogue request is not this module's");

    assert_eq!(
        err,
        StoreError::Backend("not a prompt settings request: catalogue".to_owned())
    );
}

/// The store difference D16 exists for: a migrated Postgres holds all ten `app_setting` rows
/// before anything writes one, so every entry carries a value **and** a token, and the map the
/// resolvers read is the compiled default table verbatim.
///
/// Skipped, not failed, with `HTUI_TEST_DATABASE_URL` unset.
#[tokio::test(flavor = "multi_thread")]
async fn a_migrated_postgres_presents_ten_app_rows_with_tokens() {
    let Some(db) = htui_store::testkit::demo_db().await else {
        return;
    };

    let snapshot = prompt_settings::snapshot(&db.store, &vulkan_scope())
        .await
        .expect("one read of the migrated scope");

    assert_eq!(snapshot.app.len(), 10);
    for entry in &snapshot.app {
        assert!(
            entry.value.is_some(),
            "`{:?}` is seeded by migration 0002",
            entry.key
        );
        assert!(
            entry.updated_at.is_some(),
            "`{:?}` therefore has a token, so a set carries one",
            entry.key
        );
    }

    let expected: BTreeMap<String, Value> = DEFAULTS
        .as_rows()
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    assert_eq!(snapshot.app_map(), expected);

    db.drop_db().await;
}
