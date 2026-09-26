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
use htui::agent_worker::BoxProbeReport;
use htui::app::{Action, Handled};
use htui::box_settings::{self, BoxesSnapshot, REQUEST_NAMES, spec_view};
use htui::store_worker::{StoreReply, StoreRequest, serve};
use htui::testkit::{Harness, SectionBench};
use htui::ui::tabs::settings::{BoxesSection, SettingsSection, SettingsTab};
use htui_agent::box_probe::spec;
use htui_core::fixtures::{demo_data, ids};
use htui_core::model::{BoxEdit, BoxId, BoxRecord, Scope, canonical_declared_tags};
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

/// D49: `p` on this box sends milestone 1's `ProbeBox`, and the hint says a probe is running.
#[tokio::test]
async fn p_on_this_box_sends_probe_box() {
    let (bench, mut section) = bench_with(&snap_of(two_boxes()).await).await;

    assert_eq!(bench.key(&mut section, "p"), Handled::Consumed);

    assert_eq!(names(&requests(&bench)), ["probe_box"]);
    let frame = bench.render_section(&section, 100);
    assert!(frame.contains("probing\u{2026}"), "{frame}");
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

/// OQ-16: a `CONTROL` chord passes through an open quirks editor. Nothing in the shell quits on
/// `ctrl-c` today (MOD-52), so this pins the pass-through only.
#[tokio::test]
async fn ctrl_c_passes_through_an_open_quirks_editor() {
    let (bench, mut section) = bench_with(&snap_of(MemStore::demo()).await).await;

    bench.key(&mut section, "e");
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
    assert!(frame.contains("ctrl-s saves"), "{frame}");

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
