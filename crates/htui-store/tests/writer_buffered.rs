//! The offline session sink (MOD-2 milestone 4, plan D34/D35, blueprint F-2).
//!
//! No server: every case opens a throwaway mirror over a `tempfile::tempdir()` and asserts against
//! the files [`BufferedWriter`] leaves under `<dir>/pending/`. That is the whole point of the arm -
//! the rows an online chat sends to Postgres, an offline one writes to disk - so the proof is the
//! file, read back as [`SessionEvent`]s, not a store read.
//!
//! `demo` is the feature `htui_core::fixtures::ids` lives behind; nothing here needs a database.
#![cfg(feature = "demo")]

use std::path::{Path, PathBuf};

use htui_core::fixtures::{self, ids};
use htui_core::model::{
    AgentBox, ChatRunSpec, EventKind, EventRole, ItemId, ItemPatch, NewItem, ProjectId, RunId,
    RunStatus, SessionEvent, Status, StepId,
};
use htui_core::store::{ReadStore as _, StoreError, WriteStore as _};
use htui_store::cache::pending::OPEN_SUFFIX;
use htui_store::{Backend, BufferedWriter, CacheStore, PgStore};
use serde_json::json;

/// A mirror in a throwaway directory: no server, no `%APPDATA%`.
async fn cache(root: &Path) -> CacheStore {
    CacheStore::open(root, "buffered-writer-test", PgStore::schema_version())
        .await
        .expect("open a throwaway mirror")
}

/// A chat of the fixture project, as `agent_worker` mints one.
fn chat() -> ChatRunSpec {
    ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        None,
    )
}

/// One synthetic row of a step's log.
fn event(step: StepId, seq: i32) -> SessionEvent {
    SessionEvent {
        run_step_id: step,
        seq,
        turn: 0,
        kind: EventKind::AssistantText,
        role: EventRole::Agent,
        tool_call_id: None,
        payload: json!({ "text": format!("buffered line {seq}") }),
        raw: None,
        at: fixtures::demo_at(3, i64::from(seq)),
    }
}

/// The live buffer's name, spelled out here rather than asked of the writer.
fn open_path(dir: &Path, project: ProjectId, run: RunId) -> PathBuf {
    dir.join("pending")
        .join(format!("{project}.{run}.jsonl.{OPEN_SUFFIX}"))
}

/// The sealed buffer's name - the only one `upload_pending` looks at.
fn sealed_path(dir: &Path, project: ProjectId, run: RunId) -> PathBuf {
    dir.join("pending").join(format!("{project}.{run}.jsonl"))
}

/// Every line of a buffer file, back as the events that were appended.
fn lines(path: &Path) -> Vec<SessionEvent> {
    let text = std::fs::read_to_string(path).expect("read the buffer");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every line is a session_event"))
        .collect()
}

/// How many files `pending/` holds, of any extension.
fn pending_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("pending"))
        .expect("pending/ exists: CacheStore::open creates it")
        .count()
}

#[tokio::test]
async fn start_chat_run_registers_and_append_events_buffers_in_seq_order() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();

    writer
        .start_chat_run(&chat)
        .await
        .expect("registering a chat writes no row and cannot fail");
    assert_eq!(
        writer.run_of(chat.step_id),
        Some((chat.project_id, chat.run_id)),
        "the step is registered under the pair the file name carries"
    );

    let rows: Vec<SessionEvent> = (0..5).map(|seq| event(chat.step_id, seq)).collect();
    assert_eq!(
        writer.append_events(&rows[..3]).await.expect("first flush"),
        3
    );
    assert_eq!(
        writer
            .append_events(&rows[3..])
            .await
            .expect("second flush"),
        2
    );

    let path = open_path(writer.dir(), chat.project_id, chat.run_id);
    assert_eq!(
        lines(&path),
        rows,
        "five lines, in the order they were appended"
    );
    assert!(
        !sealed_path(writer.dir(), chat.project_id, chat.run_id).exists(),
        "a live buffer is not sealed until the chat ends (H-1)"
    );

    cache.close().await;
}

#[tokio::test]
async fn an_event_for_an_unregistered_step_is_not_found_and_writes_nothing() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();
    writer.start_chat_run(&chat).await.expect("register");

    let stranger = StepId::new();
    let err = writer
        .append_events(&[event(stranger, 0)])
        .await
        .expect_err("an unregistered step has no file to append to");
    assert!(
        matches!(&err, StoreError::NotFound { entity, .. } if *entity == "run_step"),
        "an unregistered step is NotFound, never a silent drop, got {err:?}"
    );

    // The same batch also carrying a registered step's rows: still nothing is written, because the
    // trait's contract is "either every new row lands or none does".
    let batch = vec![event(chat.step_id, 0), event(stranger, 1)];
    let err = writer
        .append_events(&batch)
        .await
        .expect_err("one unknown step refuses the whole batch");
    assert!(matches!(err, StoreError::NotFound { .. }), "{err:?}");
    assert_eq!(
        pending_files(writer.dir()),
        0,
        "a refused batch leaves no file behind"
    );

    cache.close().await;
}

#[tokio::test]
async fn a_batch_spanning_two_runs_lands_in_two_files() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let first = chat();
    let second = ChatRunSpec::mint(ids::PROJECT_AGY, ids::BOX, ids::USER, None, None);
    writer.start_chat_run(&first).await.expect("register one");
    writer.start_chat_run(&second).await.expect("register two");

    // Interleaved on purpose: the grouping is by `run_step_id`, not by arrival order.
    let batch = vec![
        event(first.step_id, 0),
        event(second.step_id, 0),
        event(first.step_id, 1),
        event(second.step_id, 1),
        event(first.step_id, 2),
    ];
    assert_eq!(
        writer.append_events(&batch).await.expect("the split flush"),
        5,
        "the count is every line written, across both files"
    );

    assert_eq!(
        lines(&open_path(writer.dir(), first.project_id, first.run_id))
            .iter()
            .map(|row| row.seq)
            .collect::<Vec<i32>>(),
        vec![0, 1, 2],
    );
    assert_eq!(
        lines(&open_path(writer.dir(), second.project_id, second.run_id))
            .iter()
            .map(|row| row.seq)
            .collect::<Vec<i32>>(),
        vec![0, 1],
    );

    cache.close().await;
}

#[tokio::test]
async fn item_and_registry_writes_are_unreachable() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());

    let unreachable = |what: &str, err: StoreError| {
        assert!(
            matches!(err, StoreError::Unreachable(_)),
            "{what} needs the server, got {err:?}"
        );
    };

    let demo = fixtures::demo_data();
    unreachable(
        "mint_item",
        writer
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: ids::PROJECT_HTUI,
                kind_id: demo.kinds[0].id,
                title: "offline".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect_err("no item mint offline"),
    );
    unreachable(
        "update_item",
        writer
            .update_item(demo.items[0].id, 1, ItemPatch::default())
            .await
            .expect_err("no item edit offline"),
    );
    unreachable(
        "transition",
        writer
            .transition(demo.items[0].id, Status::Open, Status::InProgress)
            .await
            .expect_err("no status move offline"),
    );
    unreachable(
        "upsert_agent",
        writer
            .upsert_agent(&demo.agents[0])
            .await
            .expect_err("no registry write offline"),
    );
    unreachable(
        "upsert_agent_box",
        writer
            .upsert_agent_box(&AgentBox {
                agent_id: demo.agents[0].id,
                box_id: ids::BOX,
                enabled: true,
                version: None,
                path: None,
                probed_at: None,
                quota: None,
                quota_at: None,
                updated_at: fixtures::demo_at(3, 0),
                probe: None,
            })
            .await
            .expect_err("no probe write offline"),
    );
    // MOD-2 plan D68: the quota latch is a registry write like the other two. The mirror holds no
    // `agent_box` table, so an offline chat leaves the last server-side value standing and its
    // buffered `usage` rows re-derive the spend after upload.
    unreachable(
        "set_agent_box_quota",
        writer
            .set_agent_box_quota(
                demo.agents[0].id,
                ids::BOX,
                json!({ "source": "none" }),
                fixtures::demo_at(3, 0),
            )
            .await
            .expect_err("no quota latch offline"),
    );

    assert_eq!(
        pending_files(writer.dir()),
        0,
        "a refusal writes nothing to the buffer either"
    );
    cache.close().await;
}

#[tokio::test]
async fn set_step_usage_is_a_no_op() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();
    writer.start_chat_run(&chat).await.expect("register");

    writer
        .set_step_usage(
            chat.step_id,
            json!({ "input_tokens": 11 }),
            Some("d".into()),
        )
        .await
        .expect("the buffer holds session_event columns only, so this cannot fail");
    // Even for a step nobody registered: there is no row to refuse against.
    writer
        .set_step_usage(StepId::new(), json!({}), None)
        .await
        .expect("still a no-op");

    assert_eq!(
        pending_files(writer.dir()),
        0,
        "no run_step column reaches the buffer (D35); the uploader recomputes it (D36)"
    );
    cache.close().await;
}

/// The neighbour of `set_step_usage_is_a_no_op` above, and the contrast is the point (MOD-2
/// milestone 9, blueprint E-8).
///
/// `run_step.usage` may be dropped silently because `upload_pending` recomputes it from the
/// uploaded `session_event` rows (D36). Nothing recomputes a **trim record**: the buffer's line
/// format is `session_event` columns only, and the `prompt` event's payload carries an abridged
/// `sections[]` that is a lossy projection of the record. A no-op would therefore leave a step
/// with a `prompt` event and no audit row, which is what `R-PRM-3`'s "recorded on the step"
/// forbids — so this one refuses, loudly, for a registered step and an unknown one alike.
#[tokio::test]
async fn set_step_prompt_is_refused_offline() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();
    writer.start_chat_run(&chat).await.expect("register");

    for (what, step) in [
        ("a registered step", chat.step_id),
        ("an unknown step", StepId::new()),
    ] {
        let err = writer
            .set_step_prompt(step, "9f8e", &json!({ "estimated_after": 34_000 }))
            .await
            .expect_err("the prompt audit has no offline home");
        assert!(
            matches!(err, StoreError::Unreachable(_)),
            "{what}: a refusal, not a no-op and not a NotFound, got {err:?}"
        );
        assert!(
            err.to_string().contains("trim record"),
            "{what}: the sentence names the row that has nowhere to go, got {err}"
        );
    }

    assert_eq!(
        pending_files(writer.dir()),
        0,
        "a refusal writes nothing to the buffer either"
    );
    cache.close().await;
}

/// `[H-1]` The seal is what keeps `upload_pending` off a buffer whose chat is still running.
#[tokio::test]
async fn finish_chat_run_seals_the_buffer() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();
    writer.start_chat_run(&chat).await.expect("register");
    let rows: Vec<SessionEvent> = (0..3).map(|seq| event(chat.step_id, seq)).collect();
    writer.append_events(&rows).await.expect("append");

    writer
        .finish_chat_run(
            chat.run_id,
            chat.step_id,
            RunStatus::Done,
            fixtures::demo_at(3, 9),
        )
        .await
        .expect("the seal");

    assert!(
        !open_path(writer.dir(), chat.project_id, chat.run_id).exists(),
        "the open name is gone"
    );
    assert_eq!(
        lines(&sealed_path(writer.dir(), chat.project_id, chat.run_id)),
        rows,
        "and the same lines are under the uploadable name"
    );

    // A chat that recorded nothing has no buffer to seal, and that is not a failure.
    let empty = ChatRunSpec::mint(ids::PROJECT_AGY, ids::BOX, ids::USER, None, None);
    writer.start_chat_run(&empty).await.expect("register");
    writer
        .finish_chat_run(
            empty.run_id,
            empty.step_id,
            RunStatus::Cancelled,
            fixtures::demo_at(3, 9),
        )
        .await
        .expect("nothing to seal is not an error");
    assert!(!sealed_path(writer.dir(), empty.project_id, empty.run_id).exists());
    assert!(!open_path(writer.dir(), empty.project_id, empty.run_id).exists());

    // A step this writer never registered names no file, so it is the same `NotFound`
    // `append_events` answers rather than a silent success.
    let err = writer
        .finish_chat_run(
            RunId::new(),
            StepId::new(),
            RunStatus::Done,
            fixtures::demo_at(3, 9),
        )
        .await
        .expect_err("an unregistered step cannot be sealed");
    assert!(
        matches!(&err, StoreError::NotFound { entity, .. } if *entity == "run_step"),
        "{err:?}"
    );

    cache.close().await;
}

/// The recorder borrows a clone of the writer `ChatArgs` owns; both must see one registration map.
#[tokio::test]
async fn a_clone_shares_the_registration() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());
    let chat = chat();
    writer.start_chat_run(&chat).await.expect("register");

    let clone = writer.clone();
    assert_eq!(
        clone.run_of(chat.step_id),
        Some((chat.project_id, chat.run_id))
    );
    assert_eq!(
        clone
            .append_events(&[event(chat.step_id, 0)])
            .await
            .expect("the clone appends through the shared map"),
        1
    );
    assert_eq!(
        lines(&open_path(writer.dir(), chat.project_id, chat.run_id)).len(),
        1
    );

    cache.close().await;
}

#[tokio::test]
async fn reads_delegate_to_the_mirror() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());

    assert_eq!(
        writer
            .step_events(StepId::new())
            .await
            .expect("a mirror read"),
        None,
        "a freshly built mirror has cached no step, which is not the same as an empty one"
    );
    assert!(
        writer
            .runs(ids::HTUI_FEAT_1)
            .await
            .expect("a mirror read")
            .is_empty()
    );

    cache.close().await;
}

/// The seam T21 walks through: an offline backend now hands out a writer, and still says the
/// server is unreachable - the re-dial ticker keys on the second answer, not the first.
#[tokio::test]
async fn an_offline_backend_hands_out_a_buffered_writer() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let backend = Backend::Offline {
        cache: cache.clone(),
        since: None,
    };

    let writer = backend
        .writer()
        .expect("an offline backend records to its buffer (D34)");
    assert_eq!(
        writer.label(),
        "buffered",
        "what the chat header states (D42)"
    );
    assert!(
        !backend.is_writable(),
        "the server is still unreachable, which is what the re-dial ticker keys on"
    );
    assert!(
        backend.writable().is_none(),
        "and there is still no borrowed PgStore"
    );

    cache.close().await;
}
