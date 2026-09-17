//! `Settings > Prompt`, from the worker side out (MOD-15 milestone 5, D16).
//!
//! The worker half drives `htui::store_worker::serve` directly over a `Backend`, exactly as
//! `tests/kinds.rs` does: one request in, one reply out, no channels and no shell. The section
//! half is milestone 5's task 2 and follows it below, through a `Harness` for the frames a user
//! sees and a `SectionBench` for the keys and replies a frame cannot show.
//!
//! The worker half's last case is Postgres-gated and returns without asserting when
//! `HTUI_TEST_DATABASE_URL` is unset, as every other Postgres-backed suite in this crate does: it
//! is the only store where the ten `app_setting` rows exist before anything writes one
//! (`0002_agent_probe.sql:68-79`), and therefore the only one where a set carries a token rather
//! than `expected: None`.
#![cfg(feature = "testkit")]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use htui::app::{Action, Handled};
use htui::prompt_settings::{self, REQUEST_NAMES, SettingsSnapshot, project_keys};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::Theme;
use htui::ui::tabs::settings::{
    AgentsSection, HierarchySection, KindsSection, PromptSection, SettingsSection, SettingsTab,
};
use htui_core::fixtures::ids;
use htui_core::model::{ProjectId, Scope, WorkspaceId};
use htui_core::prompt::{DEFAULTS, Rungs, SettingKey};
use htui_core::store::{MemStore, SettingRung, StoreError};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::{Terminal, TerminalOptions, Viewport};
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

// -------------------------------------------------------------------------------------------
// ---- section (T2) ----
//
// The same settings from the other end: a `Harness` for the frames a user sees and a
// `SectionBench` for the keys and replies a frame cannot show (a request that was emitted, a
// scope that moved).
//
// A section holds no store (`R-NF-3`), so a test that needs a stored value seeds it through
// `store_worker::serve` and hands the section the snapshot that came back — exactly the path the
// shell takes.
// -------------------------------------------------------------------------------------------

/// A settled Settings tab over `store`, all four sections registered and the strip already cycled
/// onto `Prompt` — the product's own registration order (D15), so three `l` are what reach it.
async fn prompt_over(store: MemStore) -> Harness {
    let mut harness = Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
        Box::new(PromptSection::new()),
    ])));
    harness.settle().await;
    harness.key("l");
    harness.key("l");
    harness.key("l");
    harness.settle().await;
    harness
}

/// The scope a [`SectionBench`] issues against: the demo fixture's first workspace, exactly as the
/// bench builds it (the field itself is private).
async fn bench_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// A bench and a section with `snapshot` already delivered as a read's reply.
async fn bench_from(snapshot: &SettingsSnapshot) -> (SectionBench, PromptSection) {
    let bench = SectionBench::new().await;
    let mut section = PromptSection::new();
    feed(&bench, &mut section, snapshot);
    (bench, section)
}

/// A bench, a section and the demo settings already delivered to it.
async fn bench_with(backend: &Backend) -> (SectionBench, PromptSection, SettingsSnapshot) {
    let snapshot = demo_settings(backend).await;
    let (bench, section) = bench_from(&snapshot).await;
    (bench, section, snapshot)
}

/// Hands the section one read's reply and drops whatever it emitted.
fn feed(bench: &SectionBench, section: &mut PromptSection, snapshot: &SettingsSnapshot) {
    bench.reply(
        section,
        &StoreReply::PromptSettings(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
}

/// One write through the worker, and the settings it left behind.
async fn applied(backend: &Backend, request: StoreRequest) -> SettingsSnapshot {
    settings(serve(backend, &request).await)
}

/// A set on the `App` rung of the demo scope.
fn set_app(key: SettingKey, value: Value, expected: Option<DateTime<Utc>>) -> StoreRequest {
    StoreRequest::SetSetting {
        scope: vulkan_scope(),
        rung: SettingRung::App,
        key,
        value,
        expected,
    }
}

/// A set on the demo project's rung.
fn set_project(key: SettingKey, value: Value, expected: DateTime<Utc>) -> StoreRequest {
    StoreRequest::SetSetting {
        scope: vulkan_scope(),
        rung: SettingRung::Project(ids::PROJECT_VULKAN),
        key,
        value,
        expected: Some(expected),
    }
}

/// A clear on the demo project's rung.
fn clear_project(key: SettingKey, expected: DateTime<Utc>) -> StoreRequest {
    StoreRequest::ClearSetting {
        scope: vulkan_scope(),
        rung: SettingRung::Project(ids::PROJECT_VULKAN),
        key,
        expected,
    }
}

/// The demo project's compare-and-set token as the snapshot carries it.
fn project_token(snapshot: &SettingsSnapshot) -> DateTime<Utc> {
    snapshot.projects[0].project.updated_at
}

/// Which row of the tree one `App` key is: the `app` header is row 0 and the ten keys follow in
/// `SettingKey::ALL` order (D9).
fn app_row(key: SettingKey) -> usize {
    1 + SettingKey::ALL
        .iter()
        .position(|candidate| *candidate == key)
        .expect("one of the ten")
}

/// Which row of the tree one `Project`-rung key of the scope's single project is: the ten `App`
/// rows and their header, then the `project` header.
fn project_row(key: SettingKey) -> usize {
    SettingKey::ALL.len()
        + 2
        + project_keys()
            .position(|candidate| candidate == key)
            .expect("a key the project rung accepts")
}

/// Puts the cursor on `row`, from the top.
fn move_to(bench: &SectionBench, section: &mut PromptSection, row: usize) {
    for _ in 0..row {
        bench.key(section, "j");
    }
}

/// Types one key per char into a section, as a user would.
fn type_at(bench: &SectionBench, section: &mut dyn SettingsSection, text: &str) {
    for c in text.chars() {
        bench.key(section, &c.to_string());
    }
}

/// Empties the focused field, whatever it was prefilled with.
fn clear_field(bench: &SectionBench, section: &mut PromptSection) {
    for _ in 0..40 {
        bench.key(section, "backspace");
    }
}

/// One section drawn into a `width`x30 buffer.
///
/// [`SectionBench::render_section`] answers text, and the thing below is a *style*: a clamp line
/// that is not in `theme.error` reads as a row of the tree, and a snapshot records symbols only.
fn drawn(bench: &SectionBench, section: &dyn SettingsSection, width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, 30);
    let mut terminal = Terminal::with_options(
        TestBackend::new(width, 30),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )
    .expect("a test terminal");
    let ctx = bench.ctx();
    terminal
        .draw(|frame| section.render(frame, frame.area(), &ctx))
        .expect("the section draws");
    terminal.backend().buffer().clone()
}

/// What the section drew in the theme's error colour, one entry per row that has any.
fn error_text(bench: &SectionBench, section: &dyn SettingsSection, width: u16) -> Vec<String> {
    error_lines(&drawn(bench, section, width))
}

/// The cells of a drawn buffer that carry the theme's error foreground, one entry per row that has
/// any — whoever drew it. Shared by the bench-level [`error_text`] and the shell-level
/// [`shell_error_text`], so the two cannot disagree on what "in the error colour" means.
fn error_lines(buffer: &Buffer) -> Vec<String> {
    let error = Theme::default().error.fg.unwrap_or(Color::Reset);
    (0..buffer.area.height)
        .filter_map(|y| {
            let text: String = (0..buffer.area.width)
                .filter(|x| buffer[(*x, y)].fg == error)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        })
        .collect()
}

/// What the **shell** drew in the theme's error colour: the whole app, not one section on a bench.
///
/// [`Harness::render`] answers text alone, and a sentence that reached the screen in the ordinary
/// colour is a sentence nobody is being asked to act on. Drawn at the harness's own 100x30 so the
/// layout is the one every other `Harness` case photographs.
fn shell_error_text(harness: &mut Harness) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("a test terminal");
    terminal
        .draw(|frame| harness.app().render(frame))
        .expect("the shell draws");
    error_lines(terminal.backend().buffer())
}

/// One frame's lines with the Settings block's border stripped, so a `Harness` frame and a
/// `SectionBench` frame read the same.
fn body(frame: &str) -> Vec<&str> {
    frame
        .lines()
        .map(|line| line.strip_prefix('\u{2502}').unwrap_or(line))
        .map(|line| line.strip_suffix('\u{2502}').unwrap_or(line))
        .collect()
}

/// The value rows of the frame: every line that starts with two spaces and then a registry key.
fn value_rows(frame: &str) -> Vec<&str> {
    body(frame)
        .into_iter()
        .filter(|line| {
            line.starts_with("  ")
                && SettingKey::from_key(line.split_whitespace().next().unwrap_or("")).is_some()
        })
        .collect()
}

/// The row of one key, or a panic naming the frame that has none.
#[track_caller]
fn row_of(frame: &str, key: SettingKey, nth: usize) -> &str {
    let rows: Vec<&str> = value_rows(frame)
        .into_iter()
        .filter(|line| line.split_whitespace().next() == Some(key.key()))
        .collect();
    rows.get(nth)
        .unwrap_or_else(|| panic!("no row {nth} for `{key}`: {frame}"))
}

/// The demo world as the section draws it (D9): the `app` group with the ten registry keys, then
/// one group per project of the scope with the keys the `Project` rung accepts.
///
/// The demo project's blob already holds `token_budget` (`fixtures.rs:644-648`), so that row reads
/// `(project)` from the first frame and `prompt_upstream_hops` is the one genuinely unset
/// `Project`-rung row.
#[tokio::test]
async fn the_demo_snapshot_renders_the_tree() {
    let mut harness = prompt_over(MemStore::demo()).await;
    let frame = harness.render();

    assert!(
        frame.contains(" Agents  Hierarchy  Kinds  Prompt "),
        "the strip carries the fourth section: {frame}"
    );
    assert!(
        body(&frame).iter().any(|line| line.trim_end() == "app"),
        "the `App` group is headed: {frame}"
    );
    assert!(
        frame.contains("project Vulkan Tutorials"),
        "and the scope's one project: {frame}"
    );

    for key in SettingKey::ALL {
        let row = row_of(&frame, key, 0);
        assert!(
            row.contains("unset |"),
            "`{key}` holds nothing on a fresh MemStore: {row}"
        );
        assert!(
            row.contains("(app_setting_default)"),
            "so the compiled table answers for it: {row}"
        );
        let expected = DEFAULTS.value_of(key).to_string();
        assert!(
            row.contains(&format!("| {expected}")),
            "`{key}` shows the number the reader would use ({expected}): {row}"
        );
    }

    assert!(
        row_of(&frame, SettingKey::UpstreamHops, 1).contains("unset | 2 (app_setting_default)"),
        "the project's hops are genuinely unset: {frame}"
    );
    assert!(
        row_of(&frame, SettingKey::TokenBudget, 1).contains("120000 | 120000 (project)"),
        "the project's blob already holds the budget: {frame}"
    );

    insta::assert_snapshot!("demo", frame);
}

/// A scope with no project is the `app` group alone: there is no rung below it to list, and the
/// ten `App` rows are never empty.
#[tokio::test]
async fn no_workspace_lists_the_app_group_alone() {
    let mut harness = prompt_over(MemStore::new()).await;
    let frame = harness.render();

    assert!(
        body(&frame).iter().any(|line| line.trim_end() == "app"),
        "{frame}"
    );
    assert!(
        !frame.contains("project "),
        "no project rung is listed: {frame}"
    );
    assert_eq!(value_rows(&frame).len(), SettingKey::ALL.len(), "{frame}");

    insta::assert_snapshot!("app_only", frame);
}

/// Offline the read is refused like every write (the worker's `writer()` is `None`), so the
/// section says what is missing, in `theme.error`, instead of a tree nothing confirmed.
#[tokio::test]
async fn offline_is_unavailable_with_the_worker_sentence() {
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "prompt-section", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(SettingsTab::with_sections(vec![
        Box::new(AgentsSection::new()),
        Box::new(HierarchySection::new()),
        Box::new(KindsSection::new()),
        Box::new(PromptSection::new()),
    ])))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;
    harness.key("l");
    harness.key("l");
    harness.key("l");
    harness.settle().await;

    let frame = harness.render();
    assert!(
        frame.contains("settings unavailable"),
        "a refused read says what is missing: {frame}"
    );
    assert!(frame.contains(DATABASE_UNREACHABLE), "{frame}");

    insta::assert_snapshot!("offline", frame);
}

/// D9: a key is listed on a rung only when its spec admits it. The `App` group is the ten of
/// `SettingKey::ALL`; a project group is `project_keys()` and nothing else.
#[tokio::test]
async fn no_key_is_listed_on_a_rung_its_spec_refuses() {
    let (bench, section, _) = bench_with(&demo()).await;
    let frame = bench.render_section(&section, 100);
    let rows = value_rows(&frame);

    assert_eq!(
        rows.len(),
        SettingKey::ALL.len() + project_keys().count(),
        "ten app rows and one group of project rows: {frame}"
    );

    let project_rows = &rows[SettingKey::ALL.len()..];
    assert_eq!(project_rows.len(), project_keys().count());
    for row in project_rows {
        let key = SettingKey::from_key(row.split_whitespace().next().unwrap_or(""))
            .expect("every row names a registry key");
        assert!(
            key.spec().rungs.contains(Rungs::PROJECT),
            "`{key}` is offered on a rung its spec refuses: {row}"
        );
    }
}

/// Acceptance line 4: every row, label, unit, range and doc line is read from `SPECS`, so no key
/// name is spelled in the section at all.
#[test]
fn no_key_name_is_spelled_in_the_section() {
    let source = include_str!("../src/ui/tabs/settings/prompt.rs");

    for key in SettingKey::ALL {
        let literal = format!("\"{}\"", key.key());
        assert!(
            !source.contains(&literal),
            "`{key}` is spelled in the section; rows come from the registry"
        );
    }
}

/// `e` then a number then `Enter` is one `SetSetting` carrying the entry's own token — `None` on
/// an `App` rung that holds no row (D4, F-2).
#[tokio::test]
async fn e_then_a_number_then_enter_sends_set_setting_with_the_entrys_token() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "4000");
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::SetSetting {
            scope,
            rung,
            key,
            value,
            expected,
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` writes once: {asked:?}");
    };
    assert_eq!(*scope, bench_scope().await, "every write carries the scope");
    assert_eq!(*rung, SettingRung::App);
    assert_eq!(*key, SettingKey::TokenBudget);
    assert_eq!(*value, json!(4_000));
    assert_eq!(*expected, None, "no row: `expected: None` is the token");
}

/// On a project row the compare-and-set token is the project row's own `updated_at`, held once
/// per project rather than per key (D4).
#[tokio::test]
async fn e_on_a_project_row_carries_the_projects_updated_at() {
    let (bench, mut section, snapshot) = bench_with(&demo()).await;
    move_to(&bench, &mut section, project_row(SettingKey::UpstreamHops));

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "1");
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::SetSetting {
            rung,
            key,
            expected,
            ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` writes once: {asked:?}");
    };
    assert_eq!(*rung, SettingRung::Project(ids::PROJECT_VULKAN));
    assert_eq!(*key, SettingKey::UpstreamHops);
    assert_eq!(*expected, Some(project_token(&snapshot)));
}

/// D10: an empty field **clears** rather than writing a guessed constant, so the rung below — and
/// in the end the compiled table — answers.
#[tokio::test]
async fn an_empty_field_clears_when_the_rung_holds_a_value() {
    let backend = demo();
    let seeded = applied(
        &backend,
        set_app(SettingKey::MaxSkillTokens, json!(4_000), None),
    )
    .await;
    let token = app_token(&seeded, SettingKey::MaxSkillTokens);
    let (bench, mut section) = bench_from(&seeded).await;
    move_to(&bench, &mut section, app_row(SettingKey::MaxSkillTokens));

    bench.key(&mut section, "e");
    clear_field(&bench, &mut section);
    bench.key(&mut section, "enter");

    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::ClearSetting {
            rung,
            key,
            expected,
            ..
        }),
    ] = asked.as_slice()
    else {
        panic!("an empty field clears, once: {asked:?}");
    };
    assert_eq!(*rung, SettingRung::App);
    assert_eq!(*key, SettingKey::MaxSkillTokens);
    assert_eq!(*expected, token);
}

/// The other half of D10: clearing a rung that holds nothing is not a write at all, and saying so
/// beats a refusal from a store that was never asked.
#[tokio::test]
async fn an_empty_field_on_an_unset_rung_sends_nothing_and_says_so() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    bench.key(&mut section, "e");
    bench.key(&mut section, "enter");

    assert!(bench.drained().is_empty(), "nothing was asked of the store");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("nothing is set on this rung"), "{frame}");
    assert!(
        !error_text(&bench, &section, 100)
            .iter()
            .any(|line| line.contains("nothing is set")),
        "it is a report, not something to act on"
    );
}

/// D11: the section parses shape and nothing else, and its two sentences name the key the registry
/// spells.
#[tokio::test]
async fn a_shape_refusal_names_the_key() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "abc");
    bench.key(&mut section, "enter");

    assert!(bench.drained().is_empty(), "a shape refusal asks nothing");
    assert!(
        error_text(&bench, &section, 100)
            .iter()
            .any(|line| line.contains("`token_budget` is a whole number, or empty to clear")),
        "{:?}",
        error_text(&bench, &section, 100)
    );

    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(
        &bench,
        &mut section,
        app_row(SettingKey::PromptReserveFraction),
    );
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "abc");
    bench.key(&mut section, "enter");

    assert!(bench.drained().is_empty());
    assert!(
        error_text(&bench, &section, 100).iter().any(|line| line
            .contains("`prompt_reserve_fraction` is a decimal fraction, or empty to clear")),
        "{:?}",
        error_text(&bench, &section, 100)
    );
}

/// H-6: `inf` and `nan` are the shape refusals `parse::<f64>()` does **not** make.
///
/// `"abc"` never reaches the guard — it fails the parse — so the `is_finite()` filter is what this
/// pins: on this toolchain `"inf".parse::<f64>()` is `Ok(f64::INFINITY)` and `"nan"` is `Ok(NaN)`,
/// and `Value::from` turns either into `Null`. Without the filter a `SetSetting` carrying `Null`
/// would leave for the seam and come back "not a finite JSON number" one round trip later; with it
/// the section says the fraction sentence and asks nothing at all.
#[tokio::test]
async fn inf_and_nan_are_refused_by_shape_and_never_leave_the_section() {
    for typed in ["inf", "nan"] {
        let (bench, mut section, _) = bench_with(&demo()).await;
        move_to(
            &bench,
            &mut section,
            app_row(SettingKey::PromptReserveFraction),
        );

        bench.key(&mut section, "e");
        clear_field(&bench, &mut section);
        type_at(&bench, &mut section, typed);
        bench.key(&mut section, "enter");

        assert!(
            bench.drained().is_empty(),
            "`{typed}` parses but is not a number to store, so nothing leaves the section"
        );
        let errors = error_text(&bench, &section, 100);
        assert!(
            errors.iter().any(|line| line
                .contains("`prompt_reserve_fraction` is a decimal fraction, or empty to clear")),
            "`{typed}` gets the fraction sentence: {errors:?}"
        );
        assert!(
            section.captures_input(),
            "`{typed}` leaves the editor open over its text, as any shape refusal does"
        );
    }
}

/// D14/PRD D8: a compare-and-set miss keeps the typed text, takes the reloaded token, and retries
/// only on a second `Enter`.
#[tokio::test]
async fn a_stale_reply_keeps_the_text_and_retakes_the_token() {
    let backend = demo();
    let seeded = applied(
        &backend,
        set_app(SettingKey::TokenBudget, json!(5_000), None),
    )
    .await;
    let first = app_token(&seeded, SettingKey::TokenBudget);
    let (bench, mut section) = bench_from(&seeded).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    bench.key(&mut section, "e");
    clear_field(&bench, &mut section);
    type_at(&bench, &mut section, "7000");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    // Somebody else writes the row while the first `Enter` is out.
    let moved = applied(
        &backend,
        set_app(SettingKey::TokenBudget, json!(6_000), Some(first)),
    )
    .await;
    let second = app_token(&moved, SettingKey::TokenBudget);
    bench.reply(
        &mut section,
        &StoreReply::PromptSettingsStale(Box::new(moved)),
    );
    let _ = bench.drained();

    assert!(section.captures_input(), "the editor is still open");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("7000"), "the typed text survives: {frame}");
    assert!(
        frame.contains("changed elsewhere since you opened it"),
        "{frame}"
    );
    insta::assert_snapshot!("stale", frame);

    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::SetSetting {
            value, expected, ..
        }),
    ] = asked.as_slice()
    else {
        panic!("the retry is one write: {asked:?}");
    };
    assert_eq!(*value, json!(7_000), "the text is what is retried");
    assert_eq!(*expected, Some(second), "against the reloaded token");
}

/// B-5: an `App` row that is gone from the reload leaves the editor with no token at all, so the
/// next set passes `expected: None` rather than a dead one.
#[tokio::test]
async fn a_stale_reply_over_a_cleared_app_row_passes_no_token() {
    let backend = demo();
    let seeded = applied(
        &backend,
        set_app(SettingKey::TokenBudget, json!(5_000), None),
    )
    .await;
    let token = app_token(&seeded, SettingKey::TokenBudget);
    let (bench, mut section) = bench_from(&seeded).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    bench.key(&mut section, "e");
    clear_field(&bench, &mut section);
    type_at(&bench, &mut section, "7000");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let cleared = applied(
        &backend,
        StoreRequest::ClearSetting {
            scope: vulkan_scope(),
            rung: SettingRung::App,
            key: SettingKey::TokenBudget,
            expected: token,
        },
    )
    .await;
    bench.reply(
        &mut section,
        &StoreReply::PromptSettingsStale(Box::new(cleared)),
    );
    let _ = bench.drained();

    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::SetSetting { expected, .. })] = asked.as_slice() else {
        panic!("the retry is one write: {asked:?}");
    };
    assert_eq!(*expected, None, "no row means `expected: None` (D4, F-2)");
}

/// A project that is gone from the reload has nothing left to retry against: the editor closes and
/// says so.
#[tokio::test]
async fn a_stale_reply_for_a_vanished_project_closes_the_editor() {
    let (bench, mut section, snapshot) = bench_with(&demo()).await;
    move_to(&bench, &mut section, project_row(SettingKey::UpstreamHops));
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "1");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let mut gone = snapshot.clone();
    gone.projects.clear();
    bench.reply(
        &mut section,
        &StoreReply::PromptSettingsStale(Box::new(gone)),
    );

    assert!(!section.captures_input(), "the editor went with the row");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("deleted elsewhere \u{2014} the editor was closed"),
        "{frame}"
    );
}

/// M4's H-9 residue, inherited with the sentence: a miss with no editor open cannot say "Enter
/// retries", so it says what did not happen instead.
#[tokio::test]
async fn a_stale_reply_with_no_editor_open_says_nothing_was_written() {
    let (bench, mut section, snapshot) = bench_with(&demo()).await;

    bench.reply(
        &mut section,
        &StoreReply::PromptSettingsStale(Box::new(snapshot)),
    );

    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("changed elsewhere; nothing was written"),
        "{frame}"
    );
}

/// D11: every bound is the store's, and its sentence reaches the editor verbatim.
#[tokio::test]
async fn a_failed_write_shows_the_seams_sentence_verbatim() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "0");
    bench.key(&mut section, "enter");
    let _ = bench.drained();

    let message = "`token_budget` = 0 is outside 1..=9223372036854775807 tokens";
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_setting",
            message: message.to_owned(),
        },
    );

    assert!(
        section.captures_input(),
        "the editor stays open over its text"
    );
    let errors = error_text(&bench, &section, 100);
    assert!(
        errors.iter().any(|line| line.contains("is outside 1..=")),
        "the seam's own sentence: {errors:?}"
    );

    // `busy` was cleared with the refusal, so a corrected value goes out on the next `Enter`.
    clear_field(&bench, &mut section);
    type_at(&bench, &mut section, "1");
    bench.key(&mut section, "enter");
    assert_eq!(bench.drained().len(), 1, "the retry is not blocked");
}

/// The same claim as above, with nobody authoring the sentence.
///
/// The case above hands the section a hand-written `Failed`, so a drift in `validate`'s wording
/// would break no test at all: it pins the section's *handling*, not the seam's words. Here the
/// only thing typed is `0`, and the sentence on screen is whatever `validate` wrote, carried by
/// `set_setting` through `store_worker::serve` to the section and into the frame. Nothing in this
/// test spells any part of it but the fragment that must survive the trip.
#[tokio::test]
async fn the_seams_sentence_reaches_the_screen_with_nobody_writing_it_down() {
    let mut harness = prompt_over(MemStore::demo()).await;
    for _ in 0..app_row(SettingKey::TokenBudget) {
        harness.key("j");
    }

    harness.key("e");
    harness.key("0");
    harness.key("enter");
    harness.settle().await;

    let frame = harness.render();
    // The shell echoes `{request}: {message}` on the status line under the block, and that line
    // proves only that the worker answered. What this case is about is the line *inside* the
    // Settings block: the section's own hint row, carrying a sentence it never composed.
    let echo = format!("{}: ", REQUEST_NAMES[1]);
    let inside: Vec<String> = shell_error_text(&mut harness)
        .into_iter()
        .filter(|line| !line.starts_with(&echo))
        .collect();
    assert!(
        inside.iter().any(|line| line.contains("is outside 1..=")),
        "the section's own hint carries the store's range refusal, in the error colour: \
         {inside:?}\n{frame}"
    );
    assert!(
        frame.contains("token_budget: 0"),
        "the editor is still open over the text that was refused: {frame}"
    );
}

/// One write of a kind in flight at a time (D14): the staleness index keeps only the newest
/// request of a variant, so two racing writes would lose the reply about the one that landed.
#[tokio::test]
async fn a_second_write_while_one_is_in_flight_is_refused() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "4000");

    bench.key(&mut section, "enter");
    bench.key(&mut section, "enter");

    assert_eq!(bench.drained().len(), 1, "the second `Enter` asks nothing");
    let errors = error_text(&bench, &section, 100);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("`set_setting` is still in flight")),
        "{errors:?}"
    );
}

/// D5's `--demo` smoke, pinned: a project row's source flips to `project` when the rung is written
/// and back when it is cleared.
///
/// The flip is shown on `prompt_upstream_hops`, which the demo blob genuinely lacks; the mirror
/// image is shown on `token_budget`, which it already holds (`fixtures.rs:644-648`).
#[tokio::test]
async fn a_project_row_flips_to_project_and_back() {
    let backend = demo();
    let (bench, mut section, base) = bench_with(&backend).await;
    assert!(
        row_of(
            &bench.render_section(&section, 100),
            SettingKey::UpstreamHops,
            1
        )
        .contains("unset | 2 (app_setting_default)")
    );

    let seeded = applied(
        &backend,
        set_project(SettingKey::UpstreamHops, json!(1), project_token(&base)),
    )
    .await;
    feed(&bench, &mut section, &seeded);
    assert!(
        row_of(
            &bench.render_section(&section, 100),
            SettingKey::UpstreamHops,
            1
        )
        .contains("1 | 1 (project)"),
        "{}",
        bench.render_section(&section, 100)
    );

    let cleared = applied(
        &backend,
        clear_project(SettingKey::UpstreamHops, project_token(&seeded)),
    )
    .await;
    feed(&bench, &mut section, &cleared);
    assert!(
        row_of(
            &bench.render_section(&section, 100),
            SettingKey::UpstreamHops,
            1
        )
        .contains("unset | 2 (app_setting_default)"),
        "cleared, the rung below answers again"
    );

    // The mirror image, on the key the demo project already holds.
    let dropped = applied(
        &backend,
        clear_project(SettingKey::TokenBudget, project_token(&cleared)),
    )
    .await;
    feed(&bench, &mut section, &dropped);
    assert!(
        row_of(
            &bench.render_section(&section, 100),
            SettingKey::TokenBudget,
            1
        )
        .contains("unset | 120000 (app_setting_default)"),
        "{}",
        bench.render_section(&section, 100)
    );

    let back = applied(
        &backend,
        set_project(
            SettingKey::TokenBudget,
            json!(90_000),
            project_token(&dropped),
        ),
    )
    .await;
    feed(&bench, &mut section, &back);
    assert!(
        row_of(
            &bench.render_section(&section, 100),
            SettingKey::TokenBudget,
            1
        )
        .contains("90000 | 90000 (project)"),
        "{}",
        bench.render_section(&section, 100)
    );
}

/// D13: the `not_above` rule is one-directional, so a cap lowered under a stored head leaves the
/// row describing something the reader will not use. The pane says what it will use instead.
#[tokio::test]
async fn a_head_above_the_cap_renders_the_clamp_line() {
    let backend = demo();
    let _ = applied(
        &backend,
        set_app(SettingKey::ExcerptHeadLines, json!(50), None),
    )
    .await;
    let seeded = applied(
        &backend,
        set_app(SettingKey::ExcerptFileLineCap, json!(10), None),
    )
    .await;
    let (bench, mut section) = bench_from(&seeded).await;
    move_to(&bench, &mut section, app_row(SettingKey::ExcerptHeadLines));

    let frame = bench.render_section(&section, 100);
    assert!(
        row_of(&frame, SettingKey::ExcerptHeadLines, 0).contains("50 | 10 (app_setting)"),
        "{frame}"
    );
    assert!(
        frame.contains("clamped to excerpt_file_line_cap = 10"),
        "{frame}"
    );
    let errors = error_text(&bench, &section, 100);
    assert!(
        errors
            .iter()
            .any(|line| line.contains("clamped to excerpt_file_line_cap = 10")),
        "the consequence is something to act on: {errors:?}"
    );

    insta::assert_snapshot!("clamped", frame);
}

/// D12: a fraction renders as what it is and what it means, and the editor round-trips the stored
/// text rather than the basis points its range is written in.
#[tokio::test]
async fn the_fraction_row_renders_both_units_and_round_trips() {
    let backend = demo();
    let seeded = applied(
        &backend,
        set_app(SettingKey::PromptReserveFraction, json!(0.1), None),
    )
    .await;
    let token = app_token(&seeded, SettingKey::PromptReserveFraction);
    let (bench, mut section) = bench_from(&seeded).await;
    move_to(
        &bench,
        &mut section,
        app_row(SettingKey::PromptReserveFraction),
    );

    let frame = bench.render_section(&section, 100);
    assert!(
        row_of(&frame, SettingKey::PromptReserveFraction, 0)
            .contains("0.1 | 0.1 (1000 bp) (app_setting)"),
        "{frame}"
    );

    bench.key(&mut section, "e");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("prompt_reserve_fraction: 0.1"),
        "the field opens on the stored text: {frame}"
    );
    insta::assert_snapshot!("editor_fraction", frame);

    bench.key(&mut section, "enter");
    let asked = bench.drained();
    let [
        Action::Store(StoreRequest::SetSetting {
            value, expected, ..
        }),
    ] = asked.as_slice()
    else {
        panic!("`Enter` writes once: {asked:?}");
    };
    assert_eq!(*value, json!(0.1), "the float, never the basis points");
    assert_eq!(*expected, Some(token));
}

/// The pane under the cursor is the registry's own row: the doc line, the range in its unit, and
/// the rungs the key accepts.
#[tokio::test]
async fn the_pane_prints_doc_range_and_rungs() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    let frame = bench.render_section(&section, 100);
    let spec = SettingKey::TokenBudget.spec();
    assert!(
        frame.contains(spec.doc.split(';').next().unwrap_or(spec.doc)),
        "the registry's doc line: {frame}"
    );
    assert!(
        frame.contains("range 1..=9223372036854775807 tokens"),
        "{frame}"
    );
    assert!(frame.contains("rungs app|project|phase"), "{frame}");
}

/// While an editor is open every printable key is text, `l` included, so the tab's own section
/// cycle is off for as long as something is being typed (M3 D2).
#[tokio::test]
async fn l_is_a_letter_while_editing() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    assert!(!section.captures_input(), "Browse captures nothing");
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);
    assert!(section.captures_input(), "an open editor takes every key");

    assert_eq!(bench.key(&mut section, "l"), Handled::Consumed);
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("token_budget: l"),
        "`l` went into the field, not to the strip: {frame}"
    );
    assert!(bench.drained().is_empty(), "typing asks the store nothing");
}

/// A refused read hides the tree, so `e` has nothing to offer an editor over: the hint already
/// says `r reload` and a write from behind the refusal could only be refused again.
///
/// The snapshot is deliberately still held — `unavailable` is set over a tree that was read
/// before the outage — so this is the case `snapshot.is_none()` alone does not cover.
#[tokio::test]
async fn e_is_refused_while_the_read_is_unavailable() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "prompt_settings",
            message: DATABASE_UNREACHABLE.to_owned(),
        },
    );
    let _ = bench.drained();

    assert_eq!(bench.key(&mut section, "e"), Handled::Consumed);

    assert!(
        !section.captures_input(),
        "no editor opened over a hidden tree"
    );
    assert!(
        bench.drained().is_empty(),
        "and nothing was asked of the store"
    );
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("settings unavailable"), "{frame}");
    assert!(
        frame.contains("r reload"),
        "the hint is the only offer: {frame}"
    );
}

/// B-1: a group header is a row, so the cursor index is the line index — and `e` on one says what
/// it edits instead of opening an editor over nothing.
#[tokio::test]
async fn e_on_a_header_edits_nothing() {
    let (bench, mut section, _) = bench_with(&demo()).await;

    bench.key(&mut section, "e");

    assert!(!section.captures_input(), "no editor opened");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("`e` edits a value row"), "{frame}");
}

/// `r` re-reads the scope and `Esc` clears whatever the last reply said (T2 keys).
#[tokio::test]
async fn r_reloads_and_esc_clears_the_notice() {
    let (bench, mut section, _) = bench_with(&demo()).await;

    bench.key(&mut section, "r");
    let asked = bench.drained();
    let [Action::Store(StoreRequest::PromptSettings(scope))] = asked.as_slice() else {
        panic!("`r` asks for the scope's settings, once: {asked:?}");
    };
    assert_eq!(*scope, bench_scope().await);

    assert_eq!(
        bench.key(&mut section, "esc"),
        Handled::Pass,
        "with nothing to clear, `Esc` belongs to the shell"
    );

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_setting",
            message: "nope".to_owned(),
        },
    );
    assert!(bench.render_section(&section, 100).contains("nope"));

    assert_eq!(bench.key(&mut section, "esc"), Handled::Consumed);
    assert!(!bench.render_section(&section, 100).contains("nope"));
}

/// A scope change drops what belongs to the other workspace — the editor included, because its
/// compare-and-set token is the other workspace's — and keeps the notice.
#[tokio::test]
async fn a_scope_change_drops_the_editor_and_keeps_the_notice() {
    let (bench, mut section, _) = bench_with(&demo()).await;
    move_to(&bench, &mut section, app_row(SettingKey::TokenBudget));
    bench.key(&mut section, "e");
    assert!(section.captures_input());
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_setting",
            message: "nope".to_owned(),
        },
    );
    let other = Scope {
        workspace_id: WorkspaceId::new(),
        project_ids: vec![ProjectId::new()],
    };

    section.on_scope_change(&other);

    assert!(!section.captures_input(), "the editor did not survive it");
    let wanted = section.wants_requests(&other);
    let [StoreRequest::PromptSettings(scope)] = wanted.as_slice() else {
        panic!("the read the section wants is the new scope's, whole: {wanted:?}");
    };
    assert_eq!(*scope, other);
    let frame = bench.render_section(&section, 100);
    assert!(!frame.contains("Vulkan Tutorials"), "{frame}");
    assert!(frame.contains("nope"), "the notice survives: {frame}");
}

/// Browse is not a mode: `captures_input` is false there, so the global table still owns `q`.
#[tokio::test]
async fn q_quits_from_browse() {
    let mut harness = prompt_over(MemStore::demo()).await;
    harness.key("q");
    harness.settle().await;
    assert!(
        harness.app().should_quit,
        "Browse binds no `q`, so the global binding takes it"
    );
}
