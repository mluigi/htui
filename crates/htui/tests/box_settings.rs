//! `Settings > Boxes`, from the worker side out (MOD-7 milestone 2, D45, D46).
//!
//! The worker half drives `htui::store_worker::serve` directly over a `Backend`, exactly as
//! `tests/prompt_settings.rs` does: one request in, one reply out, no channels and no shell. The
//! section half is milestone 2's task 4 and follows it below, MOD-51's probe spec editor (D6)
//! included.
//!
//! Every case here runs over `MemStore`, or over an offline `CacheStore` for the refusal case: the
//! Postgres halves of `edit_box` are pinned by the store's own `box_identity` suite, and the
//! end-to-end path by `tests/box_probe_pg.rs`.
#![cfg(feature = "testkit")]

use chrono::{DateTime, Utc};
use htui::agent_worker::BoxProbeReport;
use htui::app::{Action, Handled};
use htui::box_settings::{self, BoxesSnapshot, REQUEST_NAMES, spec_view};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::{BoxesSection, PromptSection, SettingsSection, SettingsTab};
use htui_agent::box_probe::spec;
use htui_core::fixtures::{demo_data, ids};
use htui_core::model::{BoxEdit, BoxId, BoxRecord, Executor, Scope, canonical_declared_tags};
use htui_core::store::traits::BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN;
use htui_core::store::{MemStore, StoreError};
use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE, PgStore};
use serde_json::{Value, json};
use uuid::Uuid;

/// The demo world behind a memory backend: one box, `DESKTOP-HTUI`, which is this box.
fn demo() -> Backend {
    Backend::memory(MemStore::demo())
}

/// The demo world plus a second box of the demo user, `SECOND-BOX`, whose id sorts before the
/// fixture box's.
fn two_boxes() -> MemStore {
    let mut data = demo_data();
    let mut second = data
        .boxes
        .iter()
        .find(|row| row.id == ids::BOX)
        .expect("the fixture box")
        .clone();
    second.id = BoxId::from_uuid(Uuid::from_u128(1));
    second.hostname = "SECOND-BOX".to_owned();
    data.boxes.push(second);
    MemStore::from_demo(data)
}

/// [`two_boxes`] with `SECOND-BOX` handed to `htui worker` (MOD-41 plan D10): its settings blob
/// keeps the fixture's admission limit beside `executor`.
fn two_boxes_one_worker() -> MemStore {
    let mut data = demo_data();
    let mut second = data
        .boxes
        .iter()
        .find(|row| row.id == ids::BOX)
        .expect("the fixture box")
        .clone();
    second.id = BoxId::from_uuid(Uuid::from_u128(1));
    second.hostname = "SECOND-BOX".to_owned();
    second.settings = json!({ "max_concurrent_items": 2, "executor": "worker" });
    data.boxes.push(second);
    MemStore::from_demo(data)
}

/// The snapshot a `Boxes` reply carries, or a panic naming what came back instead.
#[track_caller]
fn boxes(reply: StoreReply) -> BoxesSnapshot {
    match reply {
        StoreReply::Boxes(snapshot) => *snapshot,
        other => panic!("expected boxes: {other:?}"),
    }
}

/// The snapshot a `BoxesStale` reply carries, or a panic naming what came back instead.
#[track_caller]
fn stale(reply: StoreReply) -> BoxesSnapshot {
    match reply {
        StoreReply::BoxesStale(snapshot) => *snapshot,
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

/// An `EditBox` request: only the fields given are `Some`.
fn edit(box_id: BoxId, expected: i32, tags: Option<&[&str]>, quirks: Option<&str>) -> StoreRequest {
    StoreRequest::EditBox {
        box_id,
        expected,
        edit: BoxEdit {
            declared_tags: tags.map(|tags| tags.iter().map(|&tag| tag.to_owned()).collect()),
            quirks: quirks.map(str::to_owned),
            executor: None,
        },
    }
}

/// A `SetProbeSpec` request (MOD-51 D4): `overlay: None` clears, `expected: None` expects no row.
fn set_spec(overlay: Option<Value>, expected: Option<DateTime<Utc>>) -> StoreRequest {
    StoreRequest::SetProbeSpec { overlay, expected }
}

/// The token of the stored overlay row a snapshot carries, or a panic when there is no row.
#[track_caller]
fn spec_token(snapshot: &BoxesSnapshot) -> DateTime<Utc> {
    snapshot
        .spec
        .stored
        .as_ref()
        .map(|row| row.updated_at)
        .unwrap_or_else(|| panic!("a stored overlay row: {:?}", snapshot.spec))
}

/// The record of `id` in a snapshot, or a panic listing what the snapshot holds.
#[track_caller]
fn record(snapshot: &BoxesSnapshot, id: BoxId) -> &BoxRecord {
    snapshot
        .boxes
        .iter()
        .find(|record| record.row.id == id)
        .unwrap_or_else(|| panic!("no box {id} in {:?}", snapshot.boxes))
}

/// A stored overlay adding `terraform`: the `terraform_spec()` document of `tests/box_probe_pg.rs`.
fn terraform_spec() -> Value {
    json!({
        "tools": {
            "terraform": {
                "kind": "path",
                "names": ["terraform"],
                "version": { "args": ["version"], "pattern": "^Terraform v(\\S+)" }
            }
        }
    })
}

/// D45: the read lists the demo box, marks it as this box, and carries the seed's spec view.
#[tokio::test]
async fn boxes_lists_the_demo_box_as_this_box() {
    let snapshot = boxes(serve(&demo(), &StoreRequest::Boxes).await);

    assert_eq!(snapshot.this_box, Some(ids::BOX));
    assert_eq!(snapshot.boxes.len(), 1, "one box: {:?}", snapshot.boxes);
    let record = &snapshot.boxes[0];
    assert_eq!(record.row.id, ids::BOX);
    assert_eq!(record.row.hostname, "DESKTOP-HTUI");
    assert_eq!(record.row.edit_version, 0);
    assert_eq!(
        record.tools.len(),
        4,
        "the fixture's four tools: {:?}",
        record.tools
    );

    assert_eq!(snapshot.spec, spec_view(None));
    assert!(!snapshot.spec.overlay);
    assert_eq!(snapshot.spec.error, None);
    assert_eq!(snapshot.spec.digest, spec::digest(spec::seed()));
}

/// D45: every box of this user, in id order, and `this_box` still names the fixture's.
#[tokio::test]
async fn boxes_lists_every_box_of_this_user_by_id() {
    let snapshot = boxes(serve(&Backend::memory(two_boxes()), &StoreRequest::Boxes).await);

    let hostnames: Vec<&str> = snapshot
        .boxes
        .iter()
        .map(|record| record.row.hostname.as_str())
        .collect();
    assert_eq!(hostnames, ["SECOND-BOX", "DESKTOP-HTUI"]);
    assert_eq!(snapshot.this_box, Some(ids::BOX));
}

/// D46: an applied edit answers the fresh snapshot, tags canonical, the token spent once.
#[tokio::test]
async fn edit_box_applied_answers_the_fresh_snapshot() {
    let snapshot = boxes(serve(&demo(), &edit(ids::BOX, 0, Some(&["vulkan", "gpu"]), None)).await);

    let row = &record(&snapshot, ids::BOX).row;
    assert_eq!(row.declared_tags, ["gpu", "vulkan"]);
    assert_eq!(row.edit_version, 1);
    assert_eq!(row.quirks, "", "only the edited field moves");
}

/// D46, D48: a spent token answers `BoxesStale` with the row as the first edit left it.
#[tokio::test]
async fn edit_box_with_a_spent_token_answers_boxes_stale_and_writes_nothing() {
    let backend = demo();
    boxes(serve(&backend, &edit(ids::BOX, 0, Some(&["gpu"]), None)).await);

    let snapshot = stale(serve(&backend, &edit(ids::BOX, 0, Some(&["cuda", "rocm"]), None)).await);

    let row = &record(&snapshot, ids::BOX).row;
    assert_eq!(row.declared_tags, ["gpu"], "the second edit wrote nothing");
    assert_eq!(row.edit_version, 1);
}

/// D45: a box that vanished under an open editor reaches the section as a snapshot without it.
#[tokio::test]
async fn edit_box_on_a_vanished_box_answers_boxes_stale_without_it() {
    let gone = BoxId::new();
    let snapshot = stale(serve(&demo(), &edit(gone, 0, Some(&["gpu"]), None)).await);

    assert!(
        snapshot.boxes.iter().all(|record| record.row.id != gone),
        "the vanished box is not listed: {:?}",
        snapshot.boxes
    );
    assert!(
        snapshot
            .boxes
            .iter()
            .any(|record| record.row.id == ids::BOX),
        "the demo box still is: {:?}",
        snapshot.boxes
    );
}

/// D42, D55: a refused tag is `Failed` under `edit_box` with the store's sentence, and nothing is
/// written.
#[tokio::test]
async fn edit_box_with_an_invalid_tag_is_failed_with_the_sentence() {
    let backend = demo();
    let (request, message) =
        refusal(serve(&backend, &edit(ids::BOX, 0, Some(&["GPU"]), None)).await);

    assert_eq!(request, "edit_box");
    let sentence = canonical_declared_tags(&["GPU".to_owned()]).unwrap_err();
    assert!(
        message.contains(&sentence),
        "the tag rule's own sentence: {message}"
    );

    let snapshot = boxes(serve(&backend, &StoreRequest::Boxes).await);
    assert_eq!(record(&snapshot, ids::BOX).row.edit_version, 0);
}

/// D45, D51: a stored, accepted overlay is in force and moves the digest; a stored, refused one is
/// named and leaves the seed's digest.
#[tokio::test]
async fn the_spec_view_names_a_stored_overlay_and_an_ignored_one() {
    let terraform = terraform_spec();
    let store = MemStore::demo();
    store.set_app_setting(spec::SETTING_KEY, terraform.clone());
    let accepted = boxes(serve(&Backend::memory(store), &StoreRequest::Boxes).await).spec;

    assert!(accepted.overlay);
    assert_eq!(accepted.error, None);
    assert_eq!(
        accepted.digest,
        spec::effective(spec::seed(), Some(&terraform)).digest
    );
    assert_ne!(accepted.digest, spec::digest(spec::seed()));
    assert_eq!(
        accepted.stored.map(|row| row.value),
        Some(Some(terraform)),
        "MOD-51 D5: the view carries the row it was computed from"
    );

    let store = MemStore::demo();
    store.set_app_setting(spec::SETTING_KEY, json!(42));
    let ignored = boxes(serve(&Backend::memory(store), &StoreRequest::Boxes).await).spec;

    assert!(!ignored.overlay);
    let error = ignored.error.expect("an ignored overlay says why");
    assert!(
        error.starts_with(spec::SPEC_IGNORED),
        "the probe's own prefix: {error}"
    );
    assert_eq!(ignored.digest, spec::digest(spec::seed()));
    assert_eq!(
        ignored.stored.map(|row| row.value),
        Some(Some(json!(42))),
        "MOD-51 D5, R-6: an ignored overlay still carries its row"
    );
}

/// MOD-51 D4, D5: an applied save answers `Boxes` with the overlay in force, its digest the
/// checker's, and the row with its token; a plain read afterwards carries the same row.
#[tokio::test]
async fn set_probe_spec_applied_answers_boxes_with_the_overlay_in_force() {
    let backend = demo();
    let snapshot = boxes(serve(&backend, &set_spec(Some(terraform_spec()), None)).await);

    assert!(snapshot.spec.overlay, "the overlay is in force");
    assert_eq!(snapshot.spec.error, None);
    assert_eq!(
        snapshot.spec.digest,
        spec::check(&terraform_spec()).expect("the terraform overlay is accepted")
    );
    assert_ne!(snapshot.spec.digest, spec::digest(spec::seed()));
    assert_eq!(
        snapshot.spec.stored.as_ref().map(|row| &row.value),
        Some(&Some(terraform_spec()))
    );

    let read = boxes(serve(&backend, &StoreRequest::Boxes).await);
    assert_eq!(
        read.spec.stored, snapshot.spec.stored,
        "the same row, the same token"
    );
}

/// MOD-51 D2, D4: a spent token answers `BoxesStale` with the row as the first save left it, and
/// so does an insert (`expected: None`) over a row that exists.
#[tokio::test]
async fn set_probe_spec_with_a_spent_token_answers_boxes_stale_and_writes_nothing() {
    let backend = demo();
    let cmake = json!({ "tools": { "cmake": { "disabled": true } } });
    let inserted = boxes(serve(&backend, &set_spec(Some(terraform_spec()), None)).await);
    let t1 = spec_token(&inserted);
    let updated = boxes(serve(&backend, &set_spec(Some(cmake.clone()), Some(t1))).await);
    let t2 = spec_token(&updated);

    let again = stale(serve(&backend, &set_spec(Some(json!({})), Some(t1))).await);
    assert_eq!(
        again.spec.stored.as_ref().map(|row| &row.value),
        Some(&Some(cmake.clone())),
        "the spent token wrote nothing"
    );
    assert_eq!(spec_token(&again), t2, "the token the first update left");

    let insert = stale(serve(&backend, &set_spec(Some(json!({})), None)).await);
    assert_eq!(
        insert.spec.stored.as_ref().map(|row| &row.value),
        Some(&Some(cmake)),
        "an insert over a row never overwrites it"
    );
}

/// MOD-51 D3, F-8: an overlay the probe would ignore is refused before the store, under the
/// request's own name, with the probe's own fault sentence after `SPEC_REFUSED`; nothing is
/// written. Through `store_worker::serve` the sentence follows the `Constraint` prefix; one layer
/// down, `box_settings::serve` answers the `Constraint` itself.
#[tokio::test]
async fn a_refused_overlay_is_failed_with_the_probe_s_sentence_and_nothing_is_written() {
    let backend = demo();
    let bad = json!({ "tools": { "x": { "kind": "path", "names": ["bin/x"] } } });
    let fault = spec::check(&bad).expect_err("a non-bare tool name is refused");

    let (request, message) = refusal(serve(&backend, &set_spec(Some(bad.clone()), None)).await);
    assert_eq!(request, "set_probe_spec");
    assert!(
        message.contains(&format!("{}: {fault}", spec::SPEC_REFUSED)),
        "the probe's own sentence after the refusal prefix: {message}"
    );

    let (request, message) = refusal(serve(&backend, &set_spec(Some(json!(42)), None)).await);
    assert_eq!(request, "set_probe_spec");
    assert!(
        message.contains("box_probe_spec refused: the value is not a JSON object"),
        "a non-object is the probe's refusal too: {message}"
    );

    let read = boxes(serve(&backend, &StoreRequest::Boxes).await);
    assert_eq!(read.spec.stored, None, "nothing was written");

    match box_settings::serve(&backend, &set_spec(Some(bad), None)).await {
        Err(StoreError::Constraint(sentence)) => assert!(
            sentence.starts_with("box_probe_spec refused: "),
            "the refusal starts with SPEC_REFUSED: {sentence}"
        ),
        other => panic!("expected a constraint: {other:?}"),
    }
}

/// MOD-51 D2, D4: a clear under the token answers `Boxes` with no stored row and the seed's spec.
#[tokio::test]
async fn clearing_under_the_token_answers_boxes_with_no_stored_overlay() {
    let backend = demo();
    let inserted = boxes(serve(&backend, &set_spec(Some(terraform_spec()), None)).await);

    let cleared = boxes(serve(&backend, &set_spec(None, Some(spec_token(&inserted)))).await);
    assert_eq!(cleared.spec.stored, None);
    assert!(!cleared.spec.overlay);
    assert_eq!(cleared.spec.error, None);
    assert_eq!(cleared.spec.digest, spec::digest(spec::seed()));
}

/// MOD-51 D2, D4, review LOW-2: an update under the token of a row cleared meanwhile answers
/// `BoxesStale` with no stored row, so the editor retries as an insert; nothing is written.
#[tokio::test]
async fn set_probe_spec_on_a_vanished_row_answers_boxes_stale_with_no_stored_overlay() {
    let backend = demo();
    let inserted = boxes(serve(&backend, &set_spec(Some(terraform_spec()), None)).await);
    let token = spec_token(&inserted);
    let cleared = boxes(serve(&backend, &set_spec(None, Some(token))).await);
    assert_eq!(cleared.spec.stored, None);

    let snapshot = stale(serve(&backend, &set_spec(Some(json!({})), Some(token))).await);
    assert_eq!(snapshot.spec.stored, None, "the row is gone");
    assert!(!snapshot.spec.overlay);

    let read = boxes(serve(&backend, &StoreRequest::Boxes).await);
    assert_eq!(read.spec.stored, None, "nothing was written");
}

/// MOD-51 D2: a clear with no token is the store's `Constraint`, bubbled to `Failed` under the
/// request's name (the checker has nothing to check in a clear).
#[tokio::test]
async fn a_clear_without_a_token_is_failed_by_the_store() {
    let (request, message) = refusal(serve(&demo(), &set_spec(None, None)).await);

    assert_eq!(request, "set_probe_spec");
    assert!(
        message.contains(BOX_PROBE_SPEC_CLEAR_NEEDS_A_TOKEN),
        "the store's own sentence: {message}"
    );
}

/// MOD-51 R-6: a stored overlay the probe ignores still carries its row and token, so the editor
/// can clear it; storing it again is refused.
#[tokio::test]
async fn a_stored_overlay_the_probe_ignores_carries_its_token_and_clears() {
    let store = MemStore::demo();
    store.set_app_setting(spec::SETTING_KEY, json!(42));
    let backend = Backend::memory(store);

    let snapshot = boxes(serve(&backend, &StoreRequest::Boxes).await);
    assert_eq!(
        snapshot.spec.stored.as_ref().map(|row| &row.value),
        Some(&Some(json!(42)))
    );
    assert!(!snapshot.spec.overlay);
    let error = snapshot
        .spec
        .error
        .clone()
        .expect("an ignored overlay says why");
    assert!(
        error.starts_with(spec::SPEC_IGNORED),
        "the probe's own prefix: {error}"
    );

    let cleared = boxes(serve(&backend, &set_spec(None, Some(spec_token(&snapshot)))).await);
    assert_eq!(
        cleared.spec.stored, None,
        "clearing an ignored overlay is accepted"
    );

    let (request, message) = refusal(serve(&backend, &set_spec(Some(json!(42)), None)).await);
    assert_eq!(request, "set_probe_spec");
    assert!(
        message.contains(spec::SPEC_REFUSED),
        "storing it back is refused: {message}"
    );
}

/// Offline, all three are refused by their own names with MOD-25's sentence: `Backend::writer()`
/// is `None`, so `serve` never reaches the seam (nor, for `SetProbeSpec`, the checker: the overlay
/// sent is one the checker refuses, review LOW-3), the read included. The length assertion keeps
/// the `zip` from dropping a request (MOD-51 F-7).
#[tokio::test]
async fn offline_every_box_request_is_refused_with_the_database_sentence() {
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(
        root.path(),
        "box-settings-offline",
        PgStore::schema_version(),
    )
    .await
    .expect("a fresh mirror");
    let backend = Backend::Offline {
        cache,
        since: Some(Utc::now()),
    };

    let requests = [
        StoreRequest::Boxes,
        edit(ids::BOX, 0, Some(&["gpu"]), None),
        set_spec(Some(json!({ "nope": 1 })), None),
    ];
    assert!(
        spec::check(&json!({ "nope": 1 })).is_err(),
        "the overlay sent offline is one the checker refuses"
    );
    assert_eq!(requests.len(), REQUEST_NAMES.len(), "one request per name");
    for (request, name) in requests.into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
        );
        assert!(
            !message.contains(spec::SPEC_REFUSED),
            "`{name}` is refused offline before the checker: {message}"
        );
    }
}

/// `box_settings::serve` is reachable only through `try_serve`'s or-ed pattern, so a request from
/// anywhere else is told which one it sent rather than panicking.
#[tokio::test]
async fn a_request_from_elsewhere_is_named_not_panicked() {
    let err = box_settings::serve(&demo(), &StoreRequest::BoxInfo)
        .await
        .expect_err("a box info request is not this module's");

    match err {
        StoreError::Backend(message) => assert!(
            message.contains("box_info"),
            "the refusal names the request: {message}"
        ),
        other => panic!("expected a backend refusal: {other:?}"),
    }
}

// -------------------------------------------------------------------------------------------
// ---- section (T4) ----
//
// The same boxes from the other end: a `SectionBench` for the keys and replies a frame cannot show
// (a request that was emitted, the token it carried), and a `Harness` or `render_section` for the
// frames a user sees. The section holds no store (`R-NF-3`), so every snapshot it is handed came
// out of `box_settings::serve`, or is one of those edited by hand to stand for a later read.
//
// Every frame snapshot runs under the digest filter (D57): the seed's digest moves whenever
// `spec.json` does, for a reason that has nothing to do with this section.
// -------------------------------------------------------------------------------------------

/// The `(filter, replacement)` every `box_settings__*` frame snapshot runs under (D57).
const DIGEST_FILTER: (&str, &str) = (r"\b[0-9a-f]{12}\b", "<digest>");

/// The snapshot a `Boxes` read answers over `store`.
async fn snap_of(store: MemStore) -> BoxesSnapshot {
    boxes(serve(&Backend::memory(store), &StoreRequest::Boxes).await)
}

/// Hands the section one read's reply and drops whatever it emitted.
fn feed(bench: &SectionBench, section: &mut BoxesSection, snapshot: &BoxesSnapshot) {
    bench.reply(section, &StoreReply::Boxes(Box::new(snapshot.clone())));
    let _ = bench.drained();
}

/// A bench and a section with `snapshot` already delivered as a read's reply.
async fn bench_with(snapshot: &BoxesSnapshot) -> (SectionBench, BoxesSection) {
    let bench = SectionBench::new().await;
    let mut section = BoxesSection::new();
    feed(&bench, &mut section, snapshot);
    (bench, section)
}

/// A settled Settings tab over `store` with the boxes section alone registered.
async fn boxes_over(store: MemStore) -> Harness {
    let mut harness =
        Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
            BoxesSection::new(),
        )])));
    harness.settle().await;
    harness
}

/// Every request the section emitted since the last drain.
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

/// The request names, for assertions that care only which requests went out.
fn names(requests: &[StoreRequest]) -> Vec<&'static str> {
    requests.iter().map(StoreRequest::name).collect()
}

/// The one `EditBox` in `requests`, or a panic naming what went out instead.
#[track_caller]
fn only_edit(requests: &[StoreRequest]) -> (BoxId, i32, BoxEdit) {
    match requests {
        [
            StoreRequest::EditBox {
                box_id,
                expected,
                edit,
            },
        ] => (*box_id, *expected, edit.clone()),
        other => panic!("expected exactly one edit_box: {other:?}"),
    }
}

/// Types one key per char, as a user would: a space is `space` and a newline is `enter`, because
/// the chord parser trims a bare `" "` to nothing.
fn type_at(bench: &SectionBench, section: &mut BoxesSection, text: &str) {
    for c in text.chars() {
        let chord = match c {
            ' ' => "space".to_owned(),
            '\n' => "enter".to_owned(),
            other => other.to_string(),
        };
        bench.key(section, &chord);
    }
}

/// The tags an edit sets, as `&str`s.
fn tags_of(edit: &BoxEdit) -> Option<Vec<&str>> {
    edit.declared_tags
        .as_ref()
        .map(|tags| tags.iter().map(String::as_str).collect())
}

/// The detail pane's `host` row for `hostname`: the 14-wide label column, then the value.
fn host_row(hostname: &str) -> String {
    format!("{:<14}{hostname}", "host")
}

/// The detail pane's `executor` row for `executor`: the 14-wide label column, then the value.
fn executor_row(executor: &str) -> String {
    format!("{:<14}{executor}", "executor")
}

/// A frame's words joined by single spaces, so a sentence the detail pane wrapped reads whole.
fn words(frame: &str) -> String {
    frame.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last line of a frame that is not blank: the notice when one shows, else the hint.
fn last_line(frame: &str) -> &str {
    frame
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
}

/// The demo snapshot with the fixture box's token set to `version`.
async fn demo_at_version(version: i32) -> BoxesSnapshot {
    let mut snapshot = snap_of(MemStore::demo()).await;
    snapshot.boxes[0].row.edit_version = version;
    snapshot
}

/// A snapshot whose fixture box moved everywhere a reconnect and a probe write, but whose token
/// did not: the read a reconnect issues after an editor opened.
fn reconnected(snapshot: &BoxesSnapshot) -> BoxesSnapshot {
    let mut later = snapshot.clone();
    let row = &mut later.boxes[0].row;
    let moved = row.updated_at + chrono::Duration::hours(3);
    row.updated_at = moved;
    row.last_seen_at = moved;
    row.last_probed_at = Some(moved);
    row.os_version = "10.0.26300".to_owned();
    later
}

/// D47: the one read, whatever the scope (boxes are the user's, not the workspace's).
#[tokio::test]
async fn activation_asks_for_boxes() {
    let workspaces = MemStore::demo()
        .workspaces()
        .await
        .expect("the memory store never fails");
    assert!(workspaces.len() >= 2, "the fixture has two workspaces");
    let section = BoxesSection::new();

    for workspace in &workspaces[..2] {
        let wanted = section.wants_requests(&Scope::from_workspace(workspace));
        assert!(
            matches!(wanted.as_slice(), [StoreRequest::Boxes]),
            "exactly one `boxes` read in {}: {wanted:?}",
            workspace.name
        );
    }
}

/// D47, D62: `j` and `k` move between the listed boxes and stop at the ends.
#[tokio::test]
async fn j_and_k_move_the_selection() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;
    let this_box = host_row("DESKTOP-HTUI (this box)");
    let second = host_row("SECOND-BOX");

    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&this_box),
        "the first read selects this box: {frame}"
    );

    assert_eq!(bench.key(&mut section, "j"), Handled::Consumed);
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&this_box),
        "`j` stops at the last box: {frame}"
    );

    bench.key(&mut section, "k");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&second), "`k` moves up: {frame}");

    bench.key(&mut section, "k");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&second),
        "`k` stops at the first box: {frame}"
    );

    bench.key(&mut section, "down");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&this_box), "`Down` moves down: {frame}");
    assert!(requests(&bench).is_empty(), "moving reads nothing");
}

/// D48: `t` opens the tag editor over the stored list, and `Enter` sends the tags alone.
#[tokio::test]
async fn t_opens_the_tag_editor_prefilled_and_enter_sends_only_the_tags() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    assert_eq!(bench.key(&mut section, "t"), Handled::Consumed);
    assert!(section.captures_input());
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&format!("{:<14}gpu", "declared tags")),
        "the field holds the stored list: {frame}"
    );

    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");

    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 0);
    assert_eq!(tags_of(&edit), Some(vec!["gpu", "vulkan"]));
    assert_eq!(edit.quirks, None, "only the edited field is sent");
}

/// D44, D48: `e` opens the quirks editor; `Enter` breaks the line and `ctrl-s` sends the quirks
/// alone.
#[tokio::test]
async fn e_opens_the_quirks_editor_and_ctrl_s_sends_only_the_quirks() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "e");
    assert!(section.captures_input());
    type_at(&bench, &mut section, "a\nb");
    assert!(requests(&bench).is_empty(), "`Enter` breaks the line");
    bench.key(&mut section, "ctrl-s");

    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 0);
    assert_eq!(edit.quirks.as_deref(), Some("a\nb"));
    assert_eq!(edit.declared_tags, None, "only the edited field is sent");
}

/// D48, D59: unchanged text closes the editor without a write; "unchanged" is the parsed list, so
/// `gpu, gpu` is the list the editor opened on.
#[tokio::test]
async fn unchanged_text_closes_the_editor_without_a_request() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    bench.key(&mut section, "enter");
    assert!(requests(&bench).is_empty(), "nothing was typed");
    assert!(!section.captures_input(), "back to browse");

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", gpu");
    bench.key(&mut section, "enter");
    assert!(
        requests(&bench).is_empty(),
        "the same list: {:?}",
        names(&requests(&bench))
    );
    assert!(!section.captures_input());

    bench.key(&mut section, "e");
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "the same quirks");
    assert!(!section.captures_input());
}

/// D42, D48: a tag the rule refuses keeps the editor open over the text and sends nothing.
#[tokio::test]
async fn an_invalid_tag_keeps_the_editor_open_and_sends_nothing() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", Bad Tag");
    bench.key(&mut section, "enter");

    assert!(requests(&bench).is_empty());
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("`Bad Tag`"),
        "the rule's sentence names the tag: {frame}"
    );
}

/// PRD metric "a reconnect does not stale an open editor" (D39, D48), the section half: a plain
/// read that lands under an open editor replaces the list and leaves the text and the token alone.
#[tokio::test]
async fn a_reload_between_open_and_save_keeps_the_editor_token() {
    let opened = demo_at_version(3).await;
    let (bench, mut section) = bench_with(&opened).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    feed(&bench, &mut section, &reconnected(&opened));

    assert!(section.captures_input(), "a read closes no editor");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("gpu, vulkan"),
        "the typed text is kept: {frame}"
    );
    assert!(
        frame.contains("10.0.26300"),
        "the list is the new one: {frame}"
    );

    bench.key(&mut section, "enter");
    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 3, "the token the editor opened on");
    assert_eq!(tags_of(&edit), Some(vec!["gpu", "vulkan"]));
}

/// D48: a spent token keeps the typed text, takes the current row's token and says so; `Enter`
/// retries against it.
#[tokio::test]
async fn boxes_stale_keeps_the_text_takes_the_new_token_and_says_changed_elsewhere() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));

    let now = demo_at_version(5).await;
    bench.reply(&mut section, &StoreReply::BoxesStale(Box::new(now)));
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("gpu, vulkan"),
        "the typed text is kept: {frame}"
    );
    assert!(
        frame.contains("changed elsewhere since you opened it"),
        "the notice says so: {frame}"
    );

    bench.key(&mut section, "enter");
    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 5, "the current row's token");
    assert_eq!(tags_of(&edit), Some(vec!["gpu", "vulkan"]));
}

/// D48: a stale snapshot without the edited box closes the editor as deleted elsewhere; with no
/// editor open, a stale snapshot says nothing was written.
#[tokio::test]
async fn a_stale_snapshot_without_the_box_closes_the_editor() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;
    let mut without = snap_of(two_boxes()).await;
    without.boxes.retain(|record| record.row.id != ids::BOX);

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(without.clone())),
    );

    assert!(!section.captures_input(), "the editor closed");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("deleted elsewhere"), "{frame}");

    bench.reply(&mut section, &StoreReply::BoxesStale(Box::new(without)));
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("changed elsewhere; nothing was written"),
        "no editor to retry from: {frame}"
    );
}

/// D56: the `Boxes` that answers the section's own save closes the editor and clears the notice.
#[tokio::test]
async fn the_reply_to_a_save_closes_the_editor() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    let browse = last_line(&bench.render_section(&section, 100)).to_owned();

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));
    feed(&bench, &mut section, &demo_at_version(1).await);

    assert!(
        !section.captures_input(),
        "the reply to a save closes the editor"
    );
    let frame = bench.render_section(&section, 100);
    assert_eq!(
        last_line(&frame),
        browse,
        "no notice under the hint: {frame}"
    );
}

/// D56: a second save while the first is in flight sends nothing and says so.
#[tokio::test]
async fn a_second_save_while_saving_sends_nothing() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    bench.key(&mut section, "enter");

    only_edit(&requests(&bench));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("edit_box in flight"), "{frame}");
}

/// §6.4: a refused save keeps the editor over its text, says why, and frees the next save; an
/// offline refusal is never followed by a `Boxes` that would clear `busy` instead.
#[tokio::test]
async fn a_refused_save_keeps_the_editor_and_frees_the_next_save() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "edit_box",
            message: "store unreachable".to_owned(),
        },
    );

    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("store unreachable"), "{frame}");
    assert!(!frame.contains("saving\u{2026}"), "{frame}");
    assert!(frame.contains("gpu, vulkan"), "{frame}");

    bench.key(&mut section, "enter");
    let (_, expected, _) = only_edit(&requests(&bench));
    assert_eq!(expected, 0);
    let frame = bench.render_section(&section, 100);
    assert!(!frame.contains("edit_box in flight"), "{frame}");
}

/// D56 (the kinds section's `blocked`): no editor opens while a save is in flight, because that
/// save's reply closes whatever editor is open and would take the new one's text with it.
#[tokio::test]
async fn no_editor_opens_while_a_save_is_in_flight() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));
    bench.key(&mut section, "esc");
    bench.key(&mut section, "e");

    assert!(!section.captures_input(), "no editor opened");
    assert!(requests(&bench).is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("edit_box in flight"), "{frame}");
}

/// OQ-16: over the quirks editor `Enter` breaks the line, so the stale notice names `ctrl-s` as
/// the retry, and it is still drawn as an error.
#[tokio::test]
async fn a_stale_reply_over_the_quirks_editor_names_ctrl_s() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "no admin rights");
    bench.key(&mut section, "ctrl-s");
    only_edit(&requests(&bench));
    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(demo_at_version(5).await)),
    );

    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("changed elsewhere since you opened it"),
        "{frame}"
    );
    assert!(frame.contains("ctrl-s retries"), "{frame}");
    assert!(!frame.contains("Enter retries"), "{frame}");

    bench.key(&mut section, "ctrl-s");
    let (_, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(expected, 5, "the current row's token");
    assert_eq!(edit.quirks.as_deref(), Some("no admin rights"));
}

/// A refused read over an open editor leaves the editor on screen under the refusal, since it
/// still takes the keys; in Browse, `t`/`e`/`p`/`w`/`s` do nothing over a list the read did
/// not confirm (MOD-51 F-3 for `s`).
#[tokio::test]
async fn a_refused_read_keeps_an_open_editor_visible_and_blocks_new_ones() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    let refused = StoreReply::Failed {
        request: "boxes",
        message: "store unreachable".to_owned(),
    };

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.reply(&mut section, &refused);
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("boxes unavailable: store unreachable"),
        "{frame}"
    );
    assert!(
        frame.contains("gpu, vulkan"),
        "the editor is on screen: {frame}"
    );

    bench.key(&mut section, "esc");
    for key in ["t", "e", "p", "w", "s"] {
        bench.key(&mut section, key);
        assert!(!section.captures_input(), "{key} opened nothing");
    }
    assert!(requests(&bench).is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("boxes unavailable: store unreachable"),
        "{frame}"
    );
    assert!(!frame.contains("gpu, vulkan"), "{frame}");
}

/// D49: `p` on this box sends milestone 1's `ProbeBox`, and the hint says a probe is running.
#[tokio::test]
async fn p_on_this_box_sends_probe_box() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);

    assert_eq!(names(&requests(&bench)), ["probe_box"]);
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("probing\u{2026}"), "{frame}");
}

/// One `ProbeBox` in flight: a second `p` sends nothing, because a second request would
/// supersede the first's seq and drop its `BoxProbed` (and so the re-read) at the shell.
#[tokio::test]
async fn p_while_probing_sends_nothing() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;
    bench.key(&mut section, "p");
    assert_eq!(names(&requests(&bench)), ["probe_box"]);

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);

    assert!(requests(&bench).is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("probing\u{2026}"), "{frame}");
    assert!(
        frame.contains("a box probe is running on this box"),
        "{frame}"
    );

    bench.reply(
        &mut section,
        &StoreReply::BoxProbed(BoxProbeReport::default()),
    );
    assert_eq!(names(&requests(&bench)), ["boxes"]);
}

/// PRD D4: the probe runs on this box only; on another box `p` sends nothing and says why.
#[tokio::test]
async fn p_on_another_box_sends_nothing_and_says_why() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;

    bench.key(&mut section, "k");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&host_row("SECOND-BOX")), "{frame}");
    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);

    assert!(requests(&bench).is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("the probe runs on this box only"), "{frame}");
}

/// D49: a probe's report carries counts, not rows, so the section reads the boxes again.
#[tokio::test]
async fn a_box_probed_reply_asks_for_boxes_again() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "p");
    let _ = requests(&bench);

    bench.reply(
        &mut section,
        &StoreReply::BoxProbed(BoxProbeReport::default()),
    );

    assert_eq!(names(&requests(&bench)), ["boxes"]);
    let frame = bench.render_section(&section, 100);
    assert!(
        !frame.contains("probing\u{2026}"),
        "the probe is over: {frame}"
    );
}

/// D49: a refused probe lands on the section's notice line.
#[tokio::test]
async fn a_failed_probe_box_lands_on_the_notice_line() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "p");
    let _ = requests(&bench);

    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "probe_box",
            message: "a box probe is already running".to_owned(),
        },
    );

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("a box probe is already running"), "{frame}");
    assert!(!frame.contains("probing\u{2026}"), "{frame}");
}

/// D47: the section takes typed text exactly while an editor is open, and a save in flight keeps
/// it open until the reply.
#[tokio::test]
async fn the_section_captures_input_only_while_an_editor_is_open() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    assert!(!section.captures_input(), "browse");
    bench.key(&mut section, "t");
    assert!(section.captures_input(), "the tag editor");
    bench.key(&mut section, "esc");
    assert!(!section.captures_input(), "`Esc` cancels");
    bench.key(&mut section, "e");
    assert!(section.captures_input(), "the quirks editor");
    type_at(&bench, &mut section, "x");
    bench.key(&mut section, "ctrl-s");
    only_edit(&requests(&bench));
    assert!(section.captures_input(), "open until the reply");
}

/// OQ-16: a `CONTROL` chord passes through an open quirks editor, so the global `ctrl-c` binding
/// (MOD-52) quits from it. The quit itself is pinned in `tests/hierarchy.rs`.
#[tokio::test]
async fn ctrl_c_passes_through_an_open_quirks_editor() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "e");
    assert_eq!(bench.key(&mut section, "ctrl-c"), Handled::Pass);
    assert!(section.captures_input(), "the editor is still open");
}

/// MOD-67 M3 (D10, D14): `[settings.boxes] reload = "f5"` is a narrower override of the shared
/// `common.reload`. Boxes reloads on `F5`, not on `r`, and its hint says so; the Prompt section
/// beside it still reloads on `r`, because the override row lives in boxes' stack alone.
#[tokio::test]
async fn a_boxes_reload_override_moves_boxes_alone() {
    let keys = htui::keys::load_str("version = 1\n[settings.boxes]\nreload = \"f5\"\n")
        .expect("the key file loads");
    let mut harness = Harness::over(MemStore::demo())
        .with_keys(keys)
        .with_tab(Box::new(SettingsTab::with_sections(vec![
            Box::new(BoxesSection::new()),
            Box::new(PromptSection::new()),
        ])));
    harness.settle().await;

    let frame = harness.render();
    assert!(frame.contains(" Boxes "), "{frame}");
    assert!(
        frame.contains("\u{b7} F5 reload"),
        "the hint names F5: {frame}"
    );
    assert!(!frame.contains("r reload"), "{frame}");

    harness.key("r");
    assert_eq!(harness.queued(), 0, "`r` is not boxes' reload any more");
    harness.key("f5");
    assert_eq!(harness.queued(), 1, "`F5` reads the boxes");
    harness.settle().await;

    harness.key("l");
    harness.settle().await;
    let frame = harness.render();
    assert!(
        frame.contains("e edit \u{b7} r reload"),
        "Prompt's hint is unchanged: {frame}"
    );
    harness.key("f5");
    assert_eq!(harness.queued(), 0, "`F5` is boxes' alone");
    harness.key("r");
    assert_eq!(harness.queued(), 1, "Prompt still reloads on `r`");
}

/// MOD-67 M3 (D13), M4 (D6): a rebound `form.save` saves the quirks editor under its new chord
/// and the hint names it; the `TextArea` passes `ctrl-s` like every chord, so `ctrl-s` saves only
/// while it is `form.save`'s chord (the defaults).
#[tokio::test]
async fn a_rebound_save_saves_the_quirks_editor_and_ctrl_s_no_longer_does() {
    let keys = htui::keys::load_str("version = 1\n[form]\nsave = \"ctrl-x\"\n")
        .expect("the key file loads");
    let bench = SectionBench::new().await.with_keys(keys);
    let mut section = BoxesSection::new();
    feed(&bench, &mut section, &snap_of(MemStore::demo()).await);

    bench.key(&mut section, "e");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("Ctrl+x saves \u{b7} Esc cancels \u{b7} Enter breaks the line"),
        "{frame}"
    );
    type_at(&bench, &mut section, "x");
    let typed = bench.render_section(&section, 100);
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "`ctrl-s` sends nothing");
    assert_eq!(
        bench.render_section(&section, 100),
        typed,
        "`ctrl-s` typed nothing and the draft is kept"
    );
    assert_eq!(bench.key(&mut section, "ctrl-x"), Handled::Consumed);
    only_edit(&requests(&bench));

    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "x");
    bench.key(&mut section, "ctrl-s");
    only_edit(&requests(&bench));
}

/// MOD-67 M3 (ANA-26 §2.6 defect 1): a browse key is a chord, modifiers included, so `ctrl-s`
/// opens no spec editor and `ctrl-t`, `ctrl-e`, `ctrl-p`, `ctrl-r` do nothing; at the executor
/// question `alt-y` writes nothing. A modal mode passes CONTROL, ALT and function keys to the
/// shell (`F1` is help) and swallows every other unused key.
#[tokio::test]
async fn modifier_chords_are_not_their_letters_in_boxes() {
    let (bench, mut section) = bench_with(&demo_at_version(3).await).await;

    for chord in ["ctrl-s", "ctrl-t", "ctrl-e", "alt-w", "ctrl-p", "ctrl-r"] {
        assert_eq!(bench.key(&mut section, chord), Handled::Pass, "`{chord}`");
        assert!(!section.captures_input(), "`{chord}` opens nothing");
    }
    assert!(bench.drained().is_empty(), "nothing probed or read");

    assert_eq!(bench.key(&mut section, "w"), Handled::Consumed);
    assert_eq!(bench.key(&mut section, "alt-y"), Handled::Pass);
    assert!(section.captures_input(), "`alt-y` answers nothing");
    assert!(requests(&bench).is_empty(), "and writes nothing");
    assert_eq!(bench.key(&mut section, "f1"), Handled::Pass, "help is F1");
    assert_eq!(
        bench.key(&mut section, "q"),
        Handled::Consumed,
        "`q` is swallowed"
    );
    assert_eq!(bench.key(&mut section, "n"), Handled::Consumed);
    assert!(!section.captures_input(), "`n` closed the question");

    bench.key(&mut section, "e");
    assert_eq!(bench.key(&mut section, "f1"), Handled::Pass, "help is F1");
    assert!(section.captures_input(), "the quirks editor is still open");
    bench.key(&mut section, "esc");

    bench.key(&mut section, "t");
    assert_eq!(
        bench.key(&mut section, "tab"),
        Handled::Consumed,
        "the tab stays"
    );
    assert_eq!(bench.key(&mut section, "f1"), Handled::Pass, "help is F1");
    assert!(section.captures_input(), "the tags editor is still open");
}

/// MOD-41 plan D10: the detail pane shows the selected box's executor: `tui` when the key is
/// missing, `worker` as written, and an unknown value as its text.
#[tokio::test]
async fn the_boxes_section_shows_each_box_s_executor() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes_one_worker()).await).await;

    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&host_row("DESKTOP-HTUI (this box)")),
        "{frame}"
    );
    assert!(
        frame.contains(&executor_row("tui")),
        "a box without the key is a TUI box: {frame}"
    );

    bench.key(&mut section, "k");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains(&host_row("SECOND-BOX")), "{frame}");
    assert!(
        frame.contains(&executor_row("worker")),
        "the planted worker box: {frame}"
    );

    let mut snapshot = snap_of(MemStore::demo()).await;
    snapshot.boxes[0].row.settings = json!({ "executor": "container" });
    let (bench, section) = bench_with(&snapshot).await;
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(&executor_row("container")),
        "an unknown executor shows its text: {frame}"
    );
}

/// MOD-41 plan D10: `w` asks first and writes nothing; `y` sends the executor alone against the
/// row's token, and the reply to it closes the confirmation; `n` and `Esc` send nothing.
#[tokio::test]
async fn w_flips_the_executor_after_confirmation() {
    let (bench, mut section) = bench_with(&demo_at_version(3).await).await;

    assert_eq!(bench.key(&mut section, "w"), Handled::Consumed);
    assert!(section.captures_input(), "the confirmation takes the keys");
    assert!(requests(&bench).is_empty(), "`w` alone writes nothing");
    let frame = bench.render_section(&section, 100);
    assert!(
        words(&frame).contains("executor of `DESKTOP-HTUI`: `tui` \u{2192} `worker`?"),
        "{frame}"
    );
    assert!(frame.contains("y write \u{b7} n/Esc cancel"), "{frame}");

    assert_eq!(bench.key(&mut section, "y"), Handled::Consumed);
    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 3, "the row's edit_version");
    assert_eq!(
        edit,
        BoxEdit {
            executor: Some(Executor::Worker),
            ..BoxEdit::default()
        },
        "the executor alone"
    );
    feed(&bench, &mut section, &demo_at_version(4).await);
    assert!(
        !section.captures_input(),
        "the reply to the write closes the confirmation"
    );

    for cancel in ["n", "esc"] {
        let (bench, mut section) = bench_with(&demo_at_version(3).await).await;
        bench.key(&mut section, "w");
        assert_eq!(bench.key(&mut section, cancel), Handled::Consumed);
        assert!(!section.captures_input(), "`{cancel}` returns to browse");
        assert!(requests(&bench).is_empty(), "`{cancel}` sends nothing");
    }
}

/// MOD-41 blueprint F-35: `edit_box` refuses an unknown executor, so from `worker` or from any
/// value this build does not know, `w` proposes `tui`, the default.
#[tokio::test]
async fn w_from_worker_or_an_unknown_executor_proposes_tui() {
    for settings in [
        json!({ "executor": "worker" }),
        json!({ "executor": "container" }),
        json!({ "executor": 7 }),
    ] {
        let mut snapshot = snap_of(MemStore::demo()).await;
        snapshot.boxes[0].row.settings = settings.clone();
        let (bench, mut section) = bench_with(&snapshot).await;

        bench.key(&mut section, "w");
        let frame = bench.render_section(&section, 100);
        assert!(
            words(&frame).contains("\u{2192} `tui`? The TUI walks this box's runs again."),
            "{settings}: {frame}"
        );
        bench.key(&mut section, "y");
        let (_, _, edit) = only_edit(&requests(&bench));
        assert_eq!(
            edit.executor,
            Some(Executor::Tui),
            "{settings} flips to tui"
        );
    }
}

/// MOD-41 plan D10, D48: a spent token keeps the confirmation open with the current row's token
/// and the stale notice, which names `y` as the retry; `y` retries against the new token.
#[tokio::test]
async fn a_stale_executor_edit_says_so() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "w");
    bench.key(&mut section, "y");
    only_edit(&requests(&bench));
    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(demo_at_version(5).await)),
    );

    assert!(section.captures_input(), "the confirmation stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("changed elsewhere since you opened it"),
        "{frame}"
    );
    assert!(frame.contains("y retries"), "{frame}");
    assert!(!frame.contains("Enter retries"), "{frame}");

    bench.key(&mut section, "y");
    let (box_id, expected, edit) = only_edit(&requests(&bench));
    assert_eq!(box_id, ids::BOX);
    assert_eq!(expected, 5, "the current row's token");
    assert_eq!(edit.executor, Some(Executor::Worker));
}

/// MOD-41 plan D10, D48: a plain read that drops the flip's own box moves the selection, but the
/// confirmation still names the box `y` writes to, never the newly selected one.
#[tokio::test]
async fn the_executor_confirmation_names_its_own_box_after_it_leaves_the_list() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes_one_worker()).await).await;

    bench.key(&mut section, "k");
    bench.key(&mut section, "w");
    feed(&bench, &mut section, &snap_of(MemStore::demo()).await);

    let frame = words(&bench.render_section(&section, 100));
    assert!(
        frame.contains("executor of `SECOND-BOX`: `worker` \u{2192} `tui`?"),
        "{frame}"
    );
    assert!(!frame.contains("executor of `DESKTOP-HTUI`"), "{frame}");

    bench.key(&mut section, "y");
    let (box_id, _, edit) = only_edit(&requests(&bench));
    assert_eq!(
        box_id,
        BoxId::from_uuid(Uuid::from_u128(1)),
        "the flip's own box"
    );
    assert_eq!(edit.executor, Some(Executor::Tui));
}

/// MOD-41 plan D10: `w` over a save in flight opens nothing and says so, like `t` and `e`.
#[tokio::test]
async fn w_while_a_save_is_in_flight_opens_nothing() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));
    bench.key(&mut section, "esc");
    assert_eq!(bench.key(&mut section, "w"), Handled::Consumed);

    assert!(!section.captures_input(), "no confirmation opened");
    assert!(requests(&bench).is_empty());
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("edit_box in flight"), "{frame}");
}

/// MOD-41 plan D10: `w` shadows the global workspace switcher only over a listed box. Before the
/// first read, over a refused read and over an empty list it passes, so the switcher still opens.
#[tokio::test]
async fn w_passes_to_the_workspace_switcher_with_no_box_to_act_on() {
    let bench = SectionBench::new().await;
    let mut section = BoxesSection::new();
    assert_eq!(bench.key(&mut section, "w"), Handled::Pass, "not read yet");

    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "boxes",
            message: "store unreachable".to_owned(),
        },
    );
    assert_eq!(
        bench.key(&mut section, "w"),
        Handled::Pass,
        "a refused read"
    );
    assert!(!section.captures_input());

    let mut empty = snap_of(MemStore::demo()).await;
    empty.boxes.clear();
    let (bench, mut section) = bench_with(&empty).await;
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no box is registered for this user yet"),
        "{frame}"
    );
    assert_eq!(bench.key(&mut section, "w"), Handled::Pass, "an empty list");
    assert!(!section.captures_input());
    assert!(requests(&bench).is_empty());
}

/// MOD-41 plan D10: a `CONTROL` chord passes through the confirmation, so `ctrl-c` still quits.
#[tokio::test]
async fn ctrl_c_passes_through_the_executor_confirmation() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "w");
    assert_eq!(bench.key(&mut section, "ctrl-c"), Handled::Pass);
    assert!(section.captures_input(), "the confirmation is still open");
}

// ---- the probe spec editor (MOD-51 D6) ----

/// The demo world with the terraform overlay stored, so the spec editor opens over a row.
fn terraform_store() -> MemStore {
    let store = MemStore::demo();
    store.set_app_setting(spec::SETTING_KEY, terraform_spec());
    store
}

/// The demo read with its box list emptied: the spec is app-wide, so `s` still works over it.
async fn no_boxes() -> BoxesSnapshot {
    let mut snapshot = snap_of(MemStore::demo()).await;
    snapshot.boxes.clear();
    snapshot
}

/// `snapshot` with its stored overlay row's token moved to `at`: the read after another writer.
fn spec_at(snapshot: &BoxesSnapshot, at: DateTime<Utc>) -> BoxesSnapshot {
    let mut later = snapshot.clone();
    later
        .spec
        .stored
        .as_mut()
        .expect("a stored overlay row")
        .updated_at = at;
    later
}

/// `snapshot` with no stored overlay row: the read after another writer cleared it.
fn spec_gone(snapshot: &BoxesSnapshot) -> BoxesSnapshot {
    let mut later = snapshot.clone();
    later.spec.stored = None;
    later
}

/// The one `SetProbeSpec` in `requests` as `(overlay, expected)`, or a panic naming what went out.
#[track_caller]
fn only_spec(requests: &[StoreRequest]) -> (Option<Value>, Option<DateTime<Utc>>) {
    match requests {
        [StoreRequest::SetProbeSpec { overlay, expected }] => (overlay.clone(), *expected),
        other => panic!("expected exactly one set_probe_spec: {other:?}"),
    }
}

/// Empties an editor holding `text` with the cursor at its end, one `backspace` per char.
fn clear_editor(bench: &SectionBench, section: &mut BoxesSection, text: &str) {
    for _ in text.chars() {
        bench.key(section, "backspace");
    }
}

/// The terraform overlay as the editor shows it.
fn terraform_text() -> String {
    serde_json::to_string_pretty(&terraform_spec()).expect("a JSON value prints")
}

/// MOD-51 D6, F-6(c): `s` opens the spec editor over the stored overlay pretty-printed, under the
/// effective spec's line and the title; nothing is sent.
#[tokio::test]
async fn s_opens_the_spec_editor_over_the_pretty_printed_overlay() {
    let (bench, mut section) = bench_with(&snap_of(terraform_store()).await).await;

    assert_eq!(bench.key(&mut section, "s"), Handled::Consumed);
    assert!(section.captures_input(), "the spec editor takes the keys");
    let frame = bench.render_section(&section, 100);
    for expected in [
        "\"terraform\": {",
        "\"kind\": \"path\"",
        "stored overlay box_probe_spec, merged into the seed by name; blank clears it",
        "probe spec: seed + stored overlay \u{b7} ",
    ] {
        assert!(frame.contains(expected), "`{expected}` in {frame}");
    }
    assert_eq!(
        last_line(&frame).trim_end_matches(' '),
        "Ctrl+s saves \u{b7} Esc cancels \u{b7} Enter breaks the line \u{b7} blank clears",
        "{frame}"
    );
    assert!(requests(&bench).is_empty(), "opening sends nothing");
}

/// MOD-51 D6, F-3: the spec is app-wide, so over a read list with no box in it the hint offers
/// `s` and the editor opens; its save expects no row.
#[tokio::test]
async fn s_with_no_box_listed_still_opens_the_spec_editor() {
    let (bench, mut section) = bench_with(&no_boxes().await).await;
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no box is registered for this user yet"),
        "{frame}"
    );
    assert_eq!(
        last_line(&frame).trim_end_matches(' '),
        "s spec \u{b7} r reload"
    );

    bench.key(&mut section, "s");
    assert!(
        section.captures_input(),
        "the spec editor opens over no box"
    );
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("probe spec: seed \u{b7} "),
        "the spec in force is on screen with no box listed (F-6(c)): {frame}"
    );
    // The spec line is drawn over an empty list in Browse too (review LOW-6), so the editor's own
    // title is what proves the editor is the thing drawn.
    assert!(
        frame.contains("stored overlay box_probe_spec, merged into the seed by name"),
        "the spec editor is drawn over no box: {frame}"
    );

    bench.paste(&mut section, r#"{"tools": {}}"#);
    bench.key(&mut section, "ctrl-s");
    assert_eq!(
        only_spec(&requests(&bench)),
        (Some(json!({ "tools": {} })), None)
    );
}

/// MOD-51 LOW-6: over a read list with no box in it the body still says what the next probe runs
/// under, so a save over an empty list shows the new source, and an ignored overlay says why.
#[tokio::test]
async fn the_empty_list_shows_the_spec_in_force() {
    let (bench, mut section) = bench_with(&no_boxes().await).await;
    bench.key(&mut section, "s");
    bench.paste(&mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));
    let mut saved = snap_of(terraform_store()).await;
    saved.boxes.clear();
    bench.reply(&mut section, &StoreReply::Boxes(Box::new(saved)));

    assert!(!section.captures_input(), "the reply closes the editor");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no box is registered for this user yet"),
        "{frame}"
    );
    assert!(
        frame.contains("probe spec: seed + stored overlay \u{b7} "),
        "the saved overlay is in force: {frame}"
    );

    let store = MemStore::demo();
    store.set_app_setting(spec::SETTING_KEY, json!(42));
    let mut ignored = snap_of(store).await;
    ignored.boxes.clear();
    feed(&bench, &mut section, &ignored);
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("probe spec: seed \u{b7} "), "{frame}");
    assert!(
        frame.contains(spec::SPEC_IGNORED),
        "the ignored overlay's sentence: {frame}"
    );
}

/// MOD-51 D6, F-3: before the first read and over a refused one `s` opens nothing, and the hint
/// offers only `r`.
#[tokio::test]
async fn s_does_nothing_before_the_first_read_or_over_a_refused_read() {
    let bench = SectionBench::new().await;
    let mut section = BoxesSection::new();
    assert_eq!(bench.key(&mut section, "s"), Handled::Consumed);
    assert!(!section.captures_input(), "not read yet");
    let frame = bench.render_section(&section, 100);
    assert_eq!(
        last_line(&frame).trim_end_matches(' '),
        "r reload",
        "{frame}"
    );

    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "boxes",
            message: "store unreachable".to_owned(),
        },
    );
    assert_eq!(bench.key(&mut section, "s"), Handled::Consumed);
    assert!(!section.captures_input(), "a refused read");
    let frame = bench.render_section(&section, 100);
    assert_eq!(
        last_line(&frame).trim_end_matches(' '),
        "r reload",
        "{frame}"
    );
    assert!(requests(&bench).is_empty());
}

/// MOD-51 D6, F-15: `ctrl-s` sends the parsed overlay under the token it opened on, even when
/// the text is unchanged (R-6 needs an unchanged save sent), and the hint says it is saving.
#[tokio::test]
async fn ctrl_s_sends_set_probe_spec_with_the_parsed_overlay_and_the_token() {
    let snapshot = snap_of(terraform_store()).await;
    let token = spec_token(&snapshot);
    let (bench, mut section) = bench_with(&snapshot).await;

    bench.key(&mut section, "s");
    bench.key(&mut section, "ctrl-s");

    assert_eq!(
        only_spec(&requests(&bench)),
        (Some(terraform_spec()), Some(token))
    );
    assert!(section.captures_input(), "open until the reply");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("saving\u{2026}"), "{frame}");
}

/// MOD-51 D6: text that is not JSON is refused on the render side; the editor stays open over it
/// with `serde_json`'s sentence, and nothing is sent.
#[tokio::test]
async fn a_parse_error_keeps_the_spec_editor_open_and_sends_nothing() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "s");
    bench.paste(&mut section, r#"{"tools": "#);
    bench.key(&mut section, "ctrl-s");

    assert!(requests(&bench).is_empty());
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("the overlay is not JSON: "), "{frame}");
    assert!(frame.contains(r#"{"tools":"#), "the text is kept: {frame}");
    assert!(!frame.contains("saving\u{2026}"), "{frame}");
}

/// MOD-51 D6: blank text over no stored row has nothing to clear, so `ctrl-s` closes the editor
/// without a request; whitespace is blank.
#[tokio::test]
async fn blank_over_no_row_closes_the_spec_editor_without_a_request() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "s");
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "nothing to clear");
    assert!(!section.captures_input(), "back to browse");

    bench.key(&mut section, "s");
    bench.paste(&mut section, "   \n  ");
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "whitespace is blank");
    assert!(!section.captures_input());
}

/// MOD-51 D6: blank text over a stored row clears it under the token it opened on.
#[tokio::test]
async fn blank_over_a_row_sends_a_clear_under_the_token() {
    let snapshot = snap_of(terraform_store()).await;
    let token = spec_token(&snapshot);
    let (bench, mut section) = bench_with(&snapshot).await;

    bench.key(&mut section, "s");
    clear_editor(&bench, &mut section, &terraform_text());
    bench.key(&mut section, "ctrl-s");

    assert_eq!(only_spec(&requests(&bench)), (None, Some(token)));
}

/// MOD-51 D6, MOD-7 D48: a plain read under an open spec editor replaces the snapshot and leaves
/// the text and the token alone.
#[tokio::test]
async fn a_plain_read_leaves_the_spec_editor_and_its_token_alone() {
    let opened = snap_of(terraform_store()).await;
    let t1 = spec_token(&opened);
    let (bench, mut section) = bench_with(&opened).await;

    bench.key(&mut section, "s");
    clear_editor(&bench, &mut section, &terraform_text());
    bench.paste(&mut section, "{}");
    feed(
        &bench,
        &mut section,
        &spec_at(&opened, t1 + chrono::Duration::seconds(5)),
    );

    assert!(section.captures_input(), "a read closes no editor");
    bench.key(&mut section, "ctrl-s");
    assert_eq!(
        only_spec(&requests(&bench)),
        (Some(json!({})), Some(t1)),
        "the token the editor opened on"
    );
}

/// MOD-51 D6, F-6(a): a spent token keeps the typed text, takes the current row's token (none
/// when the row is gone), and says `ctrl-s` retries.
#[tokio::test]
async fn boxes_stale_over_the_spec_editor_keeps_the_text_and_takes_the_new_token() {
    let opened = snap_of(terraform_store()).await;
    let t2 = spec_token(&opened) + chrono::Duration::seconds(5);
    let (bench, mut section) = bench_with(&opened).await;

    bench.key(&mut section, "s");
    clear_editor(&bench, &mut section, &terraform_text());
    bench.paste(&mut section, r#"{"gpu_vendors": []}"#);
    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));

    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(spec_at(&opened, t2))),
    );
    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains(r#"{"gpu_vendors": []}"#),
        "the typed text is kept: {frame}"
    );
    assert!(
        frame.contains("changed elsewhere since you opened it"),
        "{frame}"
    );
    assert!(frame.contains("ctrl-s retries"), "{frame}");

    bench.key(&mut section, "ctrl-s");
    assert_eq!(
        only_spec(&requests(&bench)),
        (Some(json!({ "gpu_vendors": [] })), Some(t2)),
        "the current row's token"
    );

    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(spec_gone(&opened))),
    );
    assert!(section.captures_input(), "still open over a vanished row");
    bench.key(&mut section, "ctrl-s");
    assert_eq!(
        only_spec(&requests(&bench)),
        (Some(json!({ "gpu_vendors": [] })), None),
        "a gone row is retried as an insert"
    );
}

/// MOD-51 F-4: after the row vanished under the editor there is nothing left to clear, so blank
/// text closes it without a request (the store would refuse a clear with no token).
#[tokio::test]
async fn blank_after_the_row_vanished_closes_without_a_request() {
    let opened = snap_of(terraform_store()).await;
    let token = spec_token(&opened);
    let (bench, mut section) = bench_with(&opened).await;

    bench.key(&mut section, "s");
    clear_editor(&bench, &mut section, &terraform_text());
    bench.key(&mut section, "ctrl-s");
    assert_eq!(only_spec(&requests(&bench)), (None, Some(token)));

    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(spec_gone(&opened))),
    );
    assert!(section.captures_input(), "the stale reply keeps the editor");
    bench.key(&mut section, "ctrl-s");
    assert!(requests(&bench).is_empty(), "nothing left to clear");
    assert!(!section.captures_input(), "back to browse");
}

/// MOD-51 LOW-1 (amended at review: the live token decides the blank rule): an editor opened
/// over no row whose save met a row written meanwhile holds that row's token, so blank text
/// clears it under the token rather than closing.
#[tokio::test]
async fn blank_after_a_stale_reload_over_a_new_row_clears_under_its_token() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    let later = snap_of(terraform_store()).await;
    let token = spec_token(&later);

    bench.key(&mut section, "s");
    type_at(&bench, &mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    assert_eq!(only_spec(&requests(&bench)), (Some(json!({})), None));

    bench.reply(&mut section, &StoreReply::BoxesStale(Box::new(later)));
    assert!(section.captures_input(), "the stale reply keeps the editor");
    clear_editor(&bench, &mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    assert_eq!(
        only_spec(&requests(&bench)),
        (None, Some(token)),
        "the row the stale reply carried is cleared under its token"
    );
}

/// MOD-51 D6: a refused spec save keeps the editor over its text with the worker's sentence and
/// frees the next save.
#[tokio::test]
async fn a_refused_spec_save_keeps_the_editor_and_frees_the_next_save() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    let sentence =
        "constraint violated: box_probe_spec refused: tools: `bin/x` is not a bare tool name";

    bench.key(&mut section, "s");
    bench.paste(
        &mut section,
        r#"{"tools": {"x": {"kind": "path", "names": ["bin/x"]}}}"#,
    );
    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "set_probe_spec",
            message: sentence.to_owned(),
        },
    );

    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(words(&frame).contains(sentence), "{frame}");
    assert!(!frame.contains("saving\u{2026}"), "{frame}");
    assert!(frame.contains("bin/x"), "the text is kept: {frame}");

    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));
}

/// MOD-51 D6, D8: the `Boxes` that answers a spec save closes the editor and says the next `p`
/// re-probes; the section sends no probe itself.
#[tokio::test]
async fn the_reply_to_a_spec_save_closes_the_editor_with_the_reprobe_notice() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "s");
    bench.paste(&mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));
    bench.reply(
        &mut section,
        &StoreReply::Boxes(Box::new(snap_of(terraform_store()).await)),
    );

    assert!(!section.captures_input(), "the reply closes the editor");
    assert!(requests(&bench).is_empty(), "no probe is sent (D8)");
    let frame = bench.render_section(&section, 100);
    let notice = last_line(&frame);
    assert!(notice.contains("probe spec saved"), "{frame}");
    assert!(notice.contains("re-probes under it"), "{frame}");
    assert!(!frame.contains("saving\u{2026}"), "{frame}");
}

/// MOD-51 D6, F-5: a second spec save while the first is in flight sends nothing, and the notice
/// names the spec request.
#[tokio::test]
async fn a_second_spec_save_while_saving_sends_nothing() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "s");
    bench.paste(&mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    bench.key(&mut section, "ctrl-s");

    only_spec(&requests(&bench));
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("set_probe_spec in flight"), "{frame}");
}

/// MOD-51 F-5, MOD-7 D56: no editor opens while either write is in flight, and the notice names
/// the write that is.
#[tokio::test]
async fn no_editor_opens_across_the_two_writes() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "s");
    bench.paste(&mut section, "{}");
    bench.key(&mut section, "ctrl-s");
    only_spec(&requests(&bench));
    bench.key(&mut section, "esc");
    for key in ["t", "e", "w", "s"] {
        bench.key(&mut section, key);
        assert!(!section.captures_input(), "{key} opened nothing");
        let frame = bench.render_section(&section, 100);
        assert!(frame.contains("set_probe_spec in flight"), "{key}: {frame}");
    }
    assert!(requests(&bench).is_empty());

    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    only_edit(&requests(&bench));
    bench.key(&mut section, "esc");
    assert_eq!(bench.key(&mut section, "s"), Handled::Consumed);
    assert!(!section.captures_input(), "s opened nothing");
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("edit_box in flight"), "{frame}");
}

/// MOD-51 F-6(b): a refused read over a spec editor opened over no box keeps the editor on
/// screen under the refusal, since it still takes the keys.
#[tokio::test]
async fn a_refused_read_keeps_the_spec_editor_visible() {
    let (bench, mut section) = bench_with(&no_boxes().await).await;

    bench.key(&mut section, "s");
    bench.paste(&mut section, r#"{"gpu_vendors": []}"#);
    bench.reply(
        &mut section,
        &StoreReply::Failed {
            request: "boxes",
            message: "store unreachable".to_owned(),
        },
    );

    assert!(section.captures_input(), "the editor stays open");
    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("boxes unavailable: store unreachable"),
        "{frame}"
    );
    assert!(
        frame.contains(r#"{"gpu_vendors": []}"#),
        "the editor is on screen: {frame}"
    );
}

/// MOD-51 D6: a `CONTROL` chord passes through an open spec editor, so `ctrl-c` still quits.
#[tokio::test]
async fn ctrl_c_passes_through_an_open_spec_editor() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "s");
    assert_eq!(bench.key(&mut section, "ctrl-c"), Handled::Pass);
    assert!(section.captures_input(), "the editor is still open");
}

/// D47, D51, D57: the demo box through the shell.
#[tokio::test]
async fn the_demo_box_renders() {
    let mut harness = boxes_over(MemStore::demo()).await;
    let frame = harness.render();

    for expected in [
        " Boxes ",
        "DESKTOP-HTUI (this box)",
        "windows 10.0.26200 \u{b7} x86_64",
        "last probe",
        "probed under the current spec: no",
        "cargo 1.98.0, cmake, git 2.51.0, rustc 1.98.0",
        "probe spec: seed \u{b7} ",
    ] {
        assert!(frame.contains(expected), "`{expected}` in {frame}");
    }

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("demo", frame);
    });
}

/// PRD `:262`: two listed boxes with one hostname both carry the last eight hex digits of their
/// id, and only this box is marked.
#[tokio::test]
async fn two_boxes_with_one_hostname_carry_their_id_suffix() {
    let mut snapshot = snap_of(MemStore::demo()).await;
    let mut other = snapshot.boxes[0].clone();
    other.row.id = BoxId::from_uuid(Uuid::from_u128(1));
    snapshot.boxes.insert(0, other);
    let (bench, section) = bench_with(&snapshot).await;

    let frame = bench.render_section(&section, 100);
    let other_suffix = "DESKTOP-HTUI \u{2026}00000001";
    let this_suffix = format!(
        "DESKTOP-HTUI \u{2026}{}",
        &ids::BOX.to_string().replace('-', "")[24..]
    );
    // The list pane only: the detail pane's `host` row also says `(this box)`.
    let list: Vec<String> = frame
        .lines()
        .map(|line| line.chars().take(28).collect())
        .collect();
    let rows_with = |needle: &str| list.iter().filter(|row| row.contains(needle)).count();
    assert_eq!(rows_with(other_suffix), 1, "{frame}");
    assert_eq!(
        rows_with(&this_suffix[this_suffix.len() - 8..]),
        1,
        "{frame}"
    );
    assert_eq!(rows_with("(this box)"), 1, "one box is this one: {frame}");
    assert!(
        list.iter()
            .any(|row| row.contains(&this_suffix[this_suffix.len() - 8..])
                && row.contains("(this box)")),
        "the marker sits on this box's row: {frame}"
    );

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("two_boxes", frame);
    });
}

/// F-G: offline, the read is refused with the worker's sentence, and the section says so.
#[tokio::test]
async fn offline_says_boxes_unavailable() {
    // `App::start` issues `ConnectionInfo`, and over a non-`Memory` backend that read reaches the
    // OS keyring without this guard.
    let _keyring = htui_store::testkit::mock_keyring().await;
    // The mirror outlives the harness: dropping the directory deletes it mid-test.
    let root = tempfile::tempdir().expect("a throwaway config root");
    let cache = CacheStore::open(root.path(), "box-section", PgStore::schema_version())
        .await
        .expect("a fresh mirror");
    let mut harness = Harness::over_backend(Backend::Offline {
        cache,
        since: Some(Utc::now()),
    })
    .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
        BoxesSection::new(),
    )])))
    // `offline · 3s` would age between the render and the next tick.
    .with_store_state("offline \u{b7} 0s", None);
    harness.settle().await;

    let frame = harness.render();
    assert!(frame.contains("boxes unavailable: "), "{frame}");
    assert!(frame.contains(DATABASE_UNREACHABLE), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("offline", frame);
    });
}

/// D50: the tag editor's field and the dim line of every tag seen on a listed box.
#[tokio::test]
async fn the_tag_editor_renders() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("gpu, vulkan"), "{frame}");
    assert!(frame.contains("seen: cmake, gpu, msvc, rust"), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("tag_editor", frame);
    });
}

/// D44: the quirks editor over two typed lines.
#[tokio::test]
async fn the_quirks_editor_renders() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "e");
    type_at(&bench, &mut section, "no admin rights\nuse the D: drive");

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("no admin rights"), "{frame}");
    assert!(frame.contains("use the D: drive"), "{frame}");
    assert!(frame.contains("Ctrl+s saves"), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("quirks_editor", frame);
    });
}

/// D48: a stale reply over an open editor: the kept text and the `changed elsewhere` notice.
#[tokio::test]
async fn a_stale_editor_renders() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "t");
    type_at(&bench, &mut section, ", vulkan");
    bench.key(&mut section, "enter");
    let _ = requests(&bench);
    bench.reply(
        &mut section,
        &StoreReply::BoxesStale(Box::new(demo_at_version(5).await)),
    );

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("gpu, vulkan"), "{frame}");
    assert!(frame.contains("changed elsewhere"), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("stale", frame);
    });
}

/// MOD-41 plan D10: the executor confirmation over the demo box.
#[tokio::test]
async fn the_executor_confirmation_renders() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;
    bench.key(&mut section, "w");

    let frame = bench.render_section(&section, 100);
    assert!(
        words(&frame).contains(
            "executor of `DESKTOP-HTUI`: `tui` \u{2192} `worker`? The TUI stops walking runs \
             here; `htui worker` must run on this box."
        ),
        "{frame}"
    );
    assert!(frame.contains("y write \u{b7} n/Esc cancel"), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("executor_confirm", frame);
    });
}

/// MOD-51 D6: the spec editor over the terraform overlay: the spec in force, the title, the
/// overlay pretty-printed, and the editor's keys.
#[tokio::test]
async fn the_spec_editor_renders() {
    let (bench, mut section) = bench_with(&snap_of(terraform_store()).await).await;
    bench.key(&mut section, "s");

    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("\"terraform\": {"), "{frame}");
    assert!(frame.contains("blank clears"), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("spec_editor", frame);
    });
}

/// MOD-51 LOW-6: Browse over a read list with no box in it: the message, the spec lines under
/// it, and the keys that still work.
#[tokio::test]
async fn the_empty_list_renders() {
    let (bench, section) = bench_with(&no_boxes().await).await;

    let frame = bench.render_section(&section, 100);
    assert!(
        frame.contains("no box is registered for this user yet"),
        "{frame}"
    );
    assert!(frame.contains("probe spec: seed \u{b7} "), "{frame}");

    insta::with_settings!({ filters => vec![DIGEST_FILTER] }, {
        insta::assert_snapshot!("no_boxes", frame);
    });
}
