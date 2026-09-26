//! `Settings > Boxes`, from the worker side out (MOD-7 milestone 2, D45, D46).
//!
//! The worker half drives `htui::store_worker::serve` directly over a `Backend`, exactly as
//! `tests/prompt_settings.rs` does: one request in, one reply out, no channels and no shell. The
//! section half is milestone 2's task 4 and follows it below.
//!
//! Every case here runs over `MemStore`, or over an offline `CacheStore` for the refusal case: the
//! Postgres halves of `edit_box` are pinned by the store's own `box_identity` suite, and the
//! end-to-end path by `tests/box_probe_pg.rs`.
#![cfg(feature = "testkit")]

use chrono::Utc;
use htui::box_settings::{self, BoxesSnapshot, REQUEST_NAMES, spec_view};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui_agent::box_probe::spec;
use htui_core::fixtures::{demo_data, ids};
use htui_core::model::{BoxEdit, BoxId, BoxRecord, canonical_declared_tags};
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
        },
    }
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
}

/// Offline, both are refused by their own names with MOD-25's sentence: `Backend::writer()` is
/// `None`, so `serve` never reaches the seam, the read included.
#[tokio::test]
async fn offline_both_are_refused_with_the_database_sentence() {
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

    let requests = [StoreRequest::Boxes, edit(ids::BOX, 0, Some(&["gpu"]), None)];
    for (request, name) in requests.into_iter().zip(REQUEST_NAMES) {
        let (refused, message) = refusal(serve(&backend, &request).await);
        assert_eq!(refused, name);
        assert!(
            message.contains(DATABASE_UNREACHABLE),
            "`{name}` is refused with MOD-25's sentence: {message}"
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
