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
use htui::app::{Action, Handled};
use htui::queue_settings::{self, QueueSettingsSnapshot, REQUEST_NAMES};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::SectionBench;
use htui::ui::tabs::settings::SettingsSection;
use htui::ui::tabs::settings::queue::{NOTHING_SET, QueueSection, UNKNOWN_COST};
use htui_core::fixtures::ids;
use htui_core::model::{
    DEFAULT_MAX_CONCURRENT_ITEMS, QueueSetting, Scope, USD_TOO_PRECISE, WorkspaceId,
};
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

// -------------------------------------------------------------------------------------------
// ---- section ----
//
// The same settings from the other end, through a `SectionBench`: the section holds no store
// (`R-NF-3`), so a test that needs a stored value writes it through `store_worker::serve` and hands
// the section the snapshot that came back, exactly the path the shell takes.
// -------------------------------------------------------------------------------------------

/// Row indices over the `htui` scope: the project group, the `all boxes` group, the box group.
mod row {
    /// `run cap`.
    pub const RUN_CAP: usize = 1;
    /// `batch cap`.
    pub const BATCH_CAP: usize = 2;
    /// The `all boxes` header.
    pub const APP_HEADER: usize = 3;
    /// `min budget`.
    pub const MIN_BUDGET: usize = 4;
    /// The app default `max concurrent`.
    pub const APP_LIMIT: usize = 5;
    /// `window`.
    pub const WINDOW: usize = 6;
    /// This box's `max concurrent`.
    pub const BOX_LIMIT: usize = 8;
}

/// A bench and a section with `snapshot` already delivered as a read's reply.
async fn bench_from(snapshot: &QueueSettingsSnapshot) -> (SectionBench, QueueSection) {
    let bench = SectionBench::new().await;
    let mut section = QueueSection::new();
    feed(&bench, &mut section, snapshot);
    (bench, section)
}

/// Hands the section one read's reply and drops whatever it emitted.
fn feed(bench: &SectionBench, section: &mut QueueSection, snapshot: &QueueSettingsSnapshot) {
    bench.reply(
        section,
        &StoreReply::QueueSettings(Box::new(snapshot.clone())),
    );
    let _ = bench.drained();
}

/// Puts the cursor on `row`, from the top.
fn move_to(bench: &SectionBench, section: &mut QueueSection, row: usize) {
    for _ in 0..row {
        bench.key(section, "j");
    }
}

/// Types one key per char, as a user would.
fn type_at(bench: &SectionBench, section: &mut QueueSection, text: &str) {
    for c in text.chars() {
        bench.key(section, &c.to_string());
    }
}

/// Empties the open field, whatever it was prefilled with.
fn clear_field(bench: &SectionBench, section: &mut QueueSection) {
    for _ in 0..40 {
        bench.key(section, "backspace");
    }
}

/// Opens the editor on `row`, empties it and types `text`, without pressing `Enter`.
fn edit(bench: &SectionBench, section: &mut QueueSection, row: usize, text: &str) {
    move_to(bench, section, row);
    bench.key(section, "e");
    clear_field(bench, section);
    type_at(bench, section, text);
}

/// The one store request a key emitted, or a panic naming what it emitted instead.
#[track_caller]
fn only_request(bench: &SectionBench) -> StoreRequest {
    let asked = bench.drained();
    match asked.as_slice() {
        [Action::Store(request)] => request.clone(),
        other => panic!("expected one store request: {other:?}"),
    }
}

/// The frame line that starts with `label` after its indent, or a panic naming the frame.
#[track_caller]
fn line_of<'a>(frame: &'a str, label: &str, nth: usize) -> &'a str {
    frame
        .lines()
        .filter(|line| line.trim_start().starts_with(label))
        .nth(nth)
        .unwrap_or_else(|| panic!("no line {nth} for `{label}`: {frame}"))
}

/// D8's three groups, each with its effective value: the project's caps unbounded, the app
/// default of `max_concurrent_items` shown as the default it falls back to, the window flagged as
/// stored-only, and the demo box's own limit of 2.
#[tokio::test]
async fn the_section_renders_three_groups_with_effective_values() {
    let snapshot = bench_settings(&demo()).await;
    let (bench, section) = bench_from(&snapshot).await;

    let frame = bench.render_section(&section, 100);

    assert!(frame.contains("project vulkan-tutorials"), "{frame}");
    assert!(frame.contains("all boxes"), "{frame}");
    assert!(frame.contains("this box (DESKTOP-HTUI)"), "{frame}");
    assert!(
        line_of(&frame, "run cap", 0).contains("unbounded"),
        "{frame}"
    );
    assert!(
        line_of(&frame, "batch cap", 0).contains("unbounded"),
        "{frame}"
    );
    assert!(line_of(&frame, "min budget", 0).contains("none"), "{frame}");
    assert!(
        line_of(&frame, "max concurrent", 0)
            .contains(&format!("{DEFAULT_MAX_CONCURRENT_ITEMS} (default)")),
        "{frame}"
    );
    assert!(
        line_of(&frame, "max concurrent", 1).ends_with(" 2"),
        "{frame}"
    );
    assert!(frame.contains(UNKNOWN_COST), "{frame}");
    insta::assert_snapshot!("demo", frame);
}

/// D8: money is typed in USD and stored as micros. `1.5` on `batch cap` sends a set of
/// 1 500 000 micros against the project's `updated_at`; the applied reply closes the editor and
/// the row shows the dollars back.
#[tokio::test]
async fn a_usd_entry_stores_micros() {
    let backend = demo();
    let snapshot = bench_settings(&backend).await;
    let (bench, mut section) = bench_from(&snapshot).await;

    edit(&bench, &mut section, row::BATCH_CAP, "1.5");
    assert!(section.captures_input());
    insta::assert_snapshot!("editing", bench.render_section(&section, 100));
    bench.key(&mut section, "enter");

    let request = only_request(&bench);
    let StoreRequest::SetQueueSetting {
        target,
        key,
        value,
        expected,
        ..
    } = &request
    else {
        panic!("expected a set: {request:?}");
    };
    assert_eq!(*target, QueueTarget::Project(ids::PROJECT_VULKAN));
    assert_eq!(*key, QueueSetting::PerTokenCapBatch);
    assert_eq!(*value, json!(1_500_000));
    assert_eq!(*expected, project_token(&snapshot));

    // The worker applies it; the reply closes the editor and the row reads dollars.
    let applied = serve(&backend, &request).await;
    bench.reply(&mut section, &applied);
    assert!(
        !section.captures_input(),
        "the write landed, so the editor closed"
    );
    let frame = bench.render_section(&section, 100);
    assert!(line_of(&frame, "batch cap", 0).contains("$1.50"), "{frame}");
}

/// Empty clears: a key that holds a value sends a clear against its token; a key that holds
/// nothing says so and sends nothing.
#[tokio::test]
async fn an_empty_field_clears_the_key() {
    let backend = demo();
    let before = bench_settings(&backend).await;
    let held = settings(
        serve(
            &backend,
            &set_in_bench(
                QueueTarget::Project(ids::PROJECT_VULKAN),
                QueueSetting::PerTokenCapRun,
                json!(2_000_000),
                project_token(&before),
            ),
        )
        .await,
    );
    let (bench, mut section) = bench_from(&held).await;

    edit(&bench, &mut section, row::RUN_CAP, "");
    bench.key(&mut section, "enter");
    let request = only_request(&bench);
    let StoreRequest::ClearQueueSetting {
        scope,
        target,
        key,
        expected,
    } = &request
    else {
        panic!("expected a clear: {request:?}");
    };
    assert_eq!(
        *scope,
        section_scope().await,
        "the bench's scope is re-read"
    );
    assert_eq!(*scope, graphics_scope());
    assert_eq!(*target, QueueTarget::Project(ids::PROJECT_VULKAN));
    assert_eq!(*key, QueueSetting::PerTokenCapRun);
    assert_eq!(*expected, project_token(&held));

    // Nothing stored on `min budget`: no request, and the section says why.
    let (bench, mut section) = bench_from(&held).await;
    edit(&bench, &mut section, row::MIN_BUDGET, "");
    bench.key(&mut section, "enter");
    assert!(
        bench.drained().is_empty(),
        "nothing to clear, nothing asked"
    );
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(NOTHING_SET), "{frame}");
}

/// An invalid value shows the sentence that refuses it, keeps the editor open over the text, and
/// writes nothing: the parser's for too many decimals, the validator's for a limit of 0.
#[tokio::test]
async fn an_invalid_value_shows_the_validators_sentence_and_writes_nothing() {
    let snapshot = bench_settings(&demo()).await;
    let (bench, mut section) = bench_from(&snapshot).await;

    edit(&bench, &mut section, row::BATCH_CAP, "1.2345678");
    bench.key(&mut section, "enter");
    assert!(bench.drained().is_empty(), "a refused value asks nothing");
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(USD_TOO_PRECISE), "{frame}");
    insta::assert_snapshot!("invalid", frame);

    bench.key(&mut section, "esc");
    move_to(&bench, &mut section, row::BOX_LIMIT - row::BATCH_CAP);
    bench.key(&mut section, "e");
    clear_field(&bench, &mut section);
    type_at(&bench, &mut section, "0");
    bench.key(&mut section, "enter");
    assert!(bench.drained().is_empty(), "a refused value asks nothing");
    let sentence = QueueSetting::MaxConcurrentItems
        .validate(&json!(0))
        .expect_err("0 is refused");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&sentence), "{frame}");
}

/// Review R1: a minimum budget typed in dollars is refused in dollars. `0` parses (to 0 micros) and
/// the validator refuses it with a sentence that reads the amount back as typed, `$0.00`, beside
/// the stored micros.
#[tokio::test]
async fn a_zero_minimum_budget_is_refused_in_dollars() {
    let snapshot = bench_settings(&demo()).await;
    let (bench, mut section) = bench_from(&snapshot).await;

    edit(&bench, &mut section, row::MIN_BUDGET, "0");
    bench.key(&mut section, "enter");
    assert!(bench.drained().is_empty(), "a refused value asks nothing");
    let sentence = QueueSetting::MinBudgetForNewAttempt
        .validate(&json!(0))
        .expect_err("0 is refused");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&sentence), "{frame}");
    assert!(
        sentence.contains("at least $0.000001, got $0.00"),
        "{sentence}"
    );
}

/// D8: the box limit cleared inherits the app default, and the row says what that comes to.
#[tokio::test]
async fn an_empty_box_limit_shows_the_inherited_value() {
    let backend = demo();
    let cleared = settings(
        serve(
            &backend,
            &clear_in_bench(
                QueueTarget::Box(ids::BOX),
                QueueSetting::MaxConcurrentItems,
                QueueToken::EditVersion(0),
            ),
        )
        .await,
    );
    let (bench, section) = bench_from(&cleared).await;

    let frame = bench.render_section(&section, 100);

    assert!(
        line_of(&frame, "max concurrent", 1)
            .contains(&format!("inherit ({})", cleared.app_limit())),
        "{frame}"
    );
}

/// D10: the window is stored, not enforced, and the row says so whether it is set or not.
#[tokio::test]
async fn the_window_row_says_stored_not_enforced() {
    let backend = demo();
    let unset = bench_settings(&backend).await;
    let (bench, section) = bench_from(&unset).await;
    let frame = bench.render_section(&section, 100);
    let row = line_of(&frame, "window", 0);
    assert!(row.contains("not set"), "{row}");
    assert!(row.contains("stored, not enforced"), "{row}");

    // Typed as `22:00-06:00`, stored as D10's object, shown back as typed.
    let (bench, mut section) = bench_from(&unset).await;
    edit(&bench, &mut section, row::WINDOW, "22:00-06:00");
    bench.key(&mut section, "enter");
    let request = only_request(&bench);
    let StoreRequest::SetQueueSetting { value, .. } = &request else {
        panic!("expected a set: {request:?}");
    };
    assert_eq!(*value, json!({"start": "22:00", "end": "06:00"}));
    bench.reply(&mut section, &serve(&backend, &request).await);
    let frame = bench.render_section(&section, 100);
    let row = line_of(&frame, "window", 0);
    assert!(row.contains("22:00-06:00"), "{row}");
    assert!(row.contains("stored, not enforced"), "{row}");
}

/// Offline the read is refused with the sentence every orchestration request gets off the server,
/// and the section shows it rather than an empty list.
#[tokio::test]
async fn offline_the_section_says_the_server_is_needed() {
    let bench = SectionBench::new().await;
    let mut section = QueueSection::new();

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "queue_settings",
            message: DATABASE_UNREACHABLE.to_owned(),
        },
    );

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(DATABASE_UNREACHABLE), "{frame}");
    assert!(!frame.contains("all boxes"), "{frame}");
    insta::assert_snapshot!("offline", frame);
}

/// A compare-and-set miss: the editor keeps its text, `CHANGED_ELSEWHERE` shows, and the next
/// `Enter` carries the token the reload brought.
#[tokio::test]
async fn a_cas_conflict_shows_changed_elsewhere() {
    let backend = demo();
    let before = bench_settings(&backend).await;
    let (bench, mut section) = bench_from(&before).await;

    edit(&bench, &mut section, row::BOX_LIMIT, "4");
    bench.key(&mut section, "enter");
    let request = only_request(&bench);

    // Someone else writes the box first, so the section's write misses its token.
    let _ = settings(
        serve(
            &backend,
            &set_in_bench(
                QueueTarget::Box(ids::BOX),
                QueueSetting::MaxConcurrentItems,
                json!(3),
                QueueToken::EditVersion(0),
            ),
        )
        .await,
    );
    let missed = serve(&backend, &request).await;
    assert!(
        matches!(missed, StoreReply::QueueSettingsStale(_)),
        "{missed:?}"
    );
    bench.reply(&mut section, &missed);

    assert!(
        section.captures_input(),
        "the editor stays open over its text"
    );
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(CHANGED_ELSEWHERE_HEAD), "{frame}");
    insta::assert_snapshot!("changed_elsewhere", frame);

    bench.key(&mut section, "enter");
    let StoreRequest::SetQueueSetting {
        value, expected, ..
    } = only_request(&bench)
    else {
        panic!("expected the retry");
    };
    assert_eq!(value, json!(4), "the text was kept");
    assert_eq!(expected, QueueToken::EditVersion(1), "the reload's token");
}

/// The editor captures input while open, and only then.
#[tokio::test]
async fn editing_captures_input() {
    let snapshot = bench_settings(&demo()).await;
    let (bench, mut section) = bench_from(&snapshot).await;

    assert!(!section.captures_input());
    move_to(&bench, &mut section, row::APP_LIMIT);
    bench.key(&mut section, "e");
    assert!(section.captures_input());
    bench.key(&mut section, "esc");
    assert!(!section.captures_input());

    // A header opens nothing.
    let (bench, mut section) = bench_from(&snapshot).await;
    move_to(&bench, &mut section, row::APP_HEADER);
    bench.key(&mut section, "e");
    assert!(!section.captures_input());
    assert!(bench.drained().is_empty());
}

/// The scope a [`SectionBench`] issues against: the demo fixture's first workspace.
async fn section_scope() -> Scope {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    Scope::from_workspace(workspaces.first().expect("the fixture has a workspace"))
}

/// The opening of the shared compare-and-set sentence (`settings/mod.rs`), which the section shows
/// verbatim.
const CHANGED_ELSEWHERE_HEAD: &str = "changed elsewhere since you opened it";

/// One read of the bench's scope (Graphics, so the one project is `vulkan-tutorials`): the scope
/// every request the section emits re-reads, so a frame before and after a write shows one world.
async fn bench_settings(backend: &Backend) -> QueueSettingsSnapshot {
    settings(serve(backend, &StoreRequest::QueueSettings(graphics_scope())).await)
}

/// [`set`] against the bench's scope.
fn set_in_bench(
    target: QueueTarget,
    key: QueueSetting,
    value: Value,
    expected: QueueToken,
) -> StoreRequest {
    StoreRequest::SetQueueSetting {
        scope: graphics_scope(),
        target,
        key,
        value,
        expected,
    }
}

/// [`clear`] against the bench's scope.
fn clear_in_bench(target: QueueTarget, key: QueueSetting, expected: QueueToken) -> StoreRequest {
    StoreRequest::ClearQueueSetting {
        scope: graphics_scope(),
        target,
        key,
        expected,
    }
}

/// The bench's scope, spelled out: the Graphics workspace and its one project.
fn graphics_scope() -> Scope {
    Scope {
        workspace_id: ids::WORKSPACE_GRAPHICS,
        project_ids: vec![ids::PROJECT_VULKAN],
    }
}

/// MOD-67 M3 (D12): `Enter` opens the editor as `e` does, through the queue's view default on
/// `common.edit`; a browse key is a chord, so `ctrl-e`, `ctrl-enter` and `ctrl-r` do nothing
/// (ANA-26 §2.6 defect 1); the open editor passes `F1` and `ctrl-c` and keeps `Tab`.
#[tokio::test]
async fn enter_edits_and_modifier_chords_are_not_their_letters_in_queue() {
    let snapshot = bench_settings(&demo()).await;
    let (bench, mut section) = bench_from(&snapshot).await;
    move_to(&bench, &mut section, row::BATCH_CAP);

    for chord in ["ctrl-e", "ctrl-enter", "ctrl-r"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Pass, "`{chord}`");
        assert!(!section.captures_input(), "`{chord}` opens no editor");
    }
    assert!(bench.drained().is_empty(), "nothing read");

    assert_eq!(bench.key(&mut section, "enter"), Handled::Consumed);
    assert!(section.captures_input(), "`Enter` opens the editor");
    assert_eq!(bench.key(&mut section, "f1"), Handled::Pass, "help is F1");
    assert_eq!(bench.key(&mut section, "ctrl-c"), Handled::Pass);
    assert_eq!(
        bench.key(&mut section, "tab"),
        Handled::Consumed,
        "the tab stays"
    );
    assert!(section.captures_input(), "the editor is still open");
}
