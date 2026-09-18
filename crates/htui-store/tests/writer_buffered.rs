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
    AgentBox, ChatRunSpec, DocumentId, EventKind, EventRole, GateOutcome, GraphSnapshot, Isolation,
    ItemId, ItemKindId, ItemKindPatch, ItemPatch, NewDocument, NewItem, NewItemKind, NewNote,
    NewProject, NewRepo, NewRun, NewRunStep, NewStepGraph, NewWorkspace, NoteId, PhaseId,
    PhasePatch, ProjectId, ProjectPatch, RepoBoxPath, RepoId, RepoPatch, RunId, RunMode, RunStatus,
    RunStepCommit, RunStepTree, SessionEvent, SnapshotGraph, SnapshotSettings, Status, StepGraphId,
    StepGraphPatch, StepId, StepOutcome, StepStatus, WorkspaceBoxPath, WorkspaceId, WorkspacePatch,
    WorkspaceProject,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::store::{DeleteTarget, ReadStore as _, SettingRung, StoreError, WriteStore as _};
use htui_store::cache::pending::OPEN_SUFFIX;
use htui_store::{Backend, BufferedWriter, CacheStore, DATABASE_UNREACHABLE, PgStore};
use serde_json::json;
use uuid::Uuid;

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

/// MOD-15 milestone 1, plan D2: the hierarchy is written - and read - on the server only.
///
/// All **31** new [`htui_core::store::WriteStore`] methods are listed here, readers included, and
/// every one answers [`DATABASE_UNREACHABLE`]. Two facts make that one sentence the right one
/// rather than a new constant: `htui` has been online-only since MOD-25, so a hierarchy write off
/// the server is the same event `R-STO-4` already words; and none of the six tables is mirrored
/// (`cache/mod.rs`'s `MIRRORED_TABLES`), so offline a read of them is unreachable in the literal
/// sense too. `REGISTRY_ON_SERVER_ONLY` and `PROMPT_ON_SERVER_ONLY` name other subsystems and
/// would send a reader looking in the wrong place.
///
/// The assertion is on the sentence, not just the variant, because the refusal being *one*
/// sentence is the thing that could silently regress: a second constant for the same fact is what
/// this case exists to fail on.
#[tokio::test]
async fn every_hierarchy_method_is_unreachable_offline() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());

    let refused = |what: &str, err: StoreError| match err {
        StoreError::Unreachable(sentence) => assert_eq!(
            sentence, DATABASE_UNREACHABLE,
            "{what} answers MOD-25's one offline sentence, not a second one"
        ),
        other => panic!("{what} must refuse as Unreachable, got {other:?}"),
    };

    let demo = fixtures::demo_data();
    let at = fixtures::demo_at(3, 0);
    let workspace = WorkspaceId::new();
    let project = ProjectId::new();
    let repo = RepoId::new();
    let kind = ItemKindId::new();
    let graph = StepGraphId::new();
    let phase = PhaseId::new();

    // workspace
    refused(
        "create_workspace",
        writer
            .create_workspace(NewWorkspace {
                id: workspace,
                slug: "offline".to_owned(),
                name: "Offline".to_owned(),
                description: String::new(),
                created_by: ids::USER,
            })
            .await
            .expect_err("no workspace is created offline"),
    );
    refused(
        "update_workspace",
        writer
            .update_workspace(workspace, at, WorkspacePatch::default())
            .await
            .expect_err("no workspace is edited offline"),
    );
    refused(
        "workspace",
        writer
            .workspace(workspace)
            .await
            .expect_err("the workspace table is not mirrored"),
    );

    // workspace links and box paths
    refused(
        "upsert_workspace_project",
        writer
            .upsert_workspace_project(&WorkspaceProject {
                workspace_id: workspace,
                project_id: project,
                position: 0,
            })
            .await
            .expect_err("no link is written offline"),
    );
    refused(
        "remove_workspace_project",
        writer
            .remove_workspace_project(workspace, project)
            .await
            .expect_err("no link is removed offline"),
    );
    refused(
        "workspace_projects",
        writer
            .workspace_projects(workspace)
            .await
            .expect_err("workspace_project is not mirrored"),
    );
    refused(
        "upsert_workspace_box_path",
        writer
            .upsert_workspace_box_path(&WorkspaceBoxPath {
                workspace_id: workspace,
                box_id: ids::BOX,
                root_path: "/tmp/offline".to_owned(),
                updated_at: at,
            })
            .await
            .expect_err("no root path is written offline"),
    );
    refused(
        "workspace_box_paths",
        writer
            .workspace_box_paths(workspace)
            .await
            .expect_err("workspace_box_path is not mirrored"),
    );

    // project
    refused(
        "create_project",
        writer
            .create_project(NewProject {
                id: project,
                slug: "offline".to_owned(),
                name: "Offline".to_owned(),
                description: String::new(),
                created_by: ids::USER,
            })
            .await
            .expect_err("no project is created offline"),
    );
    refused(
        "update_project",
        writer
            .update_project(project, at, ProjectPatch::default())
            .await
            .expect_err("no project is edited offline"),
    );

    // repo and repo box paths
    refused(
        "create_repo",
        writer
            .create_repo(NewRepo {
                id: repo,
                project_id: project,
                name: "htui".to_owned(),
                remote_url: None,
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect_err("no repo is created offline"),
    );
    refused(
        "update_repo",
        writer
            .update_repo(repo, at, RepoPatch::default())
            .await
            .expect_err("no repo is edited offline"),
    );
    refused(
        "repos",
        writer
            .repos(project)
            .await
            .expect_err("repo has no mirror reader"),
    );
    refused(
        "upsert_repo_box_path",
        writer
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: repo,
                box_id: ids::BOX,
                local_path: "/tmp/offline/htui".to_owned(),
                updated_at: at,
            })
            .await
            .expect_err("no checkout path is written offline"),
    );
    refused(
        "repo_box_paths",
        writer
            .repo_box_paths(repo)
            .await
            .expect_err("repo_box_path is not mirrored"),
    );

    // item_kind
    refused(
        "create_item_kind",
        writer
            .create_item_kind(NewItemKind {
                id: kind,
                project_id: project,
                prefix: "OFF".to_owned(),
                name: "Offline".to_owned(),
                description: String::new(),
                default_graph_id: graph,
                position: 0,
            })
            .await
            .expect_err("no kind is created offline"),
    );
    refused(
        "update_item_kind",
        writer
            .update_item_kind(kind, at, ItemKindPatch::default())
            .await
            .expect_err("no kind is edited offline"),
    );
    refused(
        "item_kinds",
        writer
            .item_kinds(project)
            .await
            .expect_err("item_kind has no mirror reader on this trait"),
    );
    refused(
        "delete_item_kind",
        writer
            .delete_item_kind(kind)
            .await
            .expect_err("no kind is deleted offline"),
    );

    // step_graph and phase
    refused(
        "create_step_graph",
        writer
            .create_step_graph(NewStepGraph {
                id: graph,
                project_id: project,
                name: "offline".to_owned(),
                description: String::new(),
            })
            .await
            .expect_err("no graph is created offline"),
    );
    refused(
        "update_step_graph",
        writer
            .update_step_graph(graph, at, StepGraphPatch::default())
            .await
            .expect_err("no graph is edited offline"),
    );
    refused(
        "step_graphs",
        writer
            .step_graphs(project)
            .await
            .expect_err("step_graph is not mirrored"),
    );
    refused(
        "create_phase",
        writer
            .create_phase(&demo.phases[0])
            .await
            .expect_err("no phase is created offline"),
    );
    refused(
        "update_phase",
        writer
            .update_phase(phase, at, PhasePatch::default())
            .await
            .expect_err("no phase is edited offline"),
    );
    refused(
        "phases",
        writer
            .phases(graph)
            .await
            .expect_err("step_graph_phase is not mirrored"),
    );

    // settings (D7, D8)
    refused(
        "set_setting",
        writer
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                json!(64_000),
                None,
            )
            .await
            .expect_err("no setting is written offline"),
    );
    refused(
        "clear_setting",
        writer
            .clear_setting(SettingRung::App, SettingKey::TokenBudget, at)
            .await
            .expect_err("no setting is cleared offline"),
    );
    refused(
        "setting",
        writer
            .setting(SettingRung::Project(project), SettingKey::UpstreamHops)
            .await
            .expect_err("app_setting is not mirrored"),
    );

    // deletes (D4)
    refused(
        "delete_reach",
        writer
            .delete_reach(DeleteTarget::Workspace(workspace))
            .await
            .expect_err("the reach is counted on the server"),
    );
    refused(
        "delete_workspace",
        writer
            .delete_workspace(workspace)
            .await
            .expect_err("no workspace is deleted offline"),
    );
    refused(
        "delete_project",
        writer
            .delete_project(project)
            .await
            .expect_err("no project is deleted offline"),
    );

    assert_eq!(
        pending_files(writer.dir()),
        0,
        "31 refusals write nothing to the buffer either"
    );
    cache.close().await;
}

/// The smallest `R-ORCH-11` snapshot [`NewRun`] takes: enough to type-check, never decoded here.
///
/// Offline the refusal comes before anything looks at it, which is the point — a snapshot the
/// seam would reject on the server must still be refused with the *offline* sentence.
fn offline_snapshot() -> GraphSnapshot {
    GraphSnapshot {
        v: GraphSnapshot::V,
        graph: SnapshotGraph {
            id: ids::GRAPH_HTUI_FEAT,
            name: "feature".to_owned(),
            is_override: false,
        },
        topology: "sha256:offline".to_owned(),
        mode: RunMode::Manual,
        phases: Vec::new(),
        settings: SnapshotSettings {
            default_isolation: Isolation::Worktree,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            max_fan_out: 4,
            max_agents_per_run: 6,
        },
    }
}

/// MOD-4 milestone 1, plan D5: ANA-2 §8's run seam is written - and read - on the server only.
///
/// All **23** new methods are listed here, the five [`htui_core::store::ReadStore`] reads
/// included, and every one answers [`DATABASE_UNREACHABLE`]. The sibling case above says why that
/// is the right sentence rather than a new constant, and it is the same reason twice over here:
/// `htui` has been online-only since MOD-25, so queueing a run off the server is exactly the event
/// `R-STO-4` words as "no item creation, no runs"; and a buffered `claim_run` would be a lease
/// taken against a database nobody can reach, which is not a sink, it is a lie.
///
/// The assertion is on the sentence, not just the variant, because the refusal being *one*
/// sentence is the thing that could silently regress: a second constant for the same fact is what
/// this case exists to fail on.
#[tokio::test]
async fn every_run_seam_method_is_unreachable_offline() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let writer = BufferedWriter::new(cache.clone());

    let refused = |what: &str, err: StoreError| match err {
        StoreError::Unreachable(sentence) => assert_eq!(
            sentence, DATABASE_UNREACHABLE,
            "{what} answers MOD-25's one offline sentence, not a second one"
        ),
        other => panic!("{what} must refuse as Unreachable, got {other:?}"),
    };

    let at = fixtures::demo_at(3, 0);
    let later = fixtures::demo_at(3, 1);
    let run = RunId::new();
    let step = StepId::new();
    let repo = RepoId::new();
    let owner = Uuid::now_v7();

    // ---- the five reads (plan D1) ----
    refused(
        "run",
        writer.run(run).await.expect_err("run has no mirror reader"),
    );
    refused(
        "run_steps",
        writer
            .run_steps(run)
            .await
            .expect_err("run_step has no mirror reader"),
    );
    refused(
        "step_trees",
        writer
            .step_trees(step)
            .await
            .expect_err("run_step_tree has no mirror reader"),
    );
    refused(
        "step_commits",
        writer
            .step_commits(step)
            .await
            .expect_err("run_step_commit has no mirror reader"),
    );
    refused(
        "resolve_inputs",
        writer
            .resolve_inputs(ids::HTUI_FEAT_1, run, &["plan".to_owned()])
            .await
            .expect_err("inputs are resolved on the server"),
    );

    // ---- run creation, admission and lease ----
    refused(
        "create_run",
        writer
            .create_run(NewRun {
                id: run,
                project_id: ids::PROJECT_HTUI,
                item_id: ids::HTUI_FEAT_1,
                mode: RunMode::Manual,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: offline_snapshot(),
                repo_scope: vec![repo],
                queued_at: at,
            })
            .await
            .expect_err("no run is queued offline"),
    );
    refused(
        "claim_run",
        writer
            .claim_run(run, ids::BOX, owner, at, later)
            .await
            .expect_err("no run is claimed offline"),
    );
    refused(
        "refresh_lease",
        writer
            .refresh_lease(run, owner, later)
            .await
            .expect_err("no lease is refreshed offline"),
    );
    refused(
        "adopt_runs",
        writer
            .adopt_runs(ids::BOX, owner, at, later)
            .await
            .expect_err("no run is adopted offline"),
    );

    // ---- steps and the §4.3 law ----
    refused(
        "create_step",
        writer
            .create_step(NewRunStep {
                id: step,
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "plan".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: None,
            })
            .await
            .expect_err("no step is created offline"),
    );
    refused(
        "transition_run",
        writer
            .transition_run(run, RunStatus::Queued, RunStatus::Running, at)
            .await
            .expect_err("no run moves offline"),
    );
    refused(
        "transition_step",
        writer
            .transition_step(step, StepStatus::Pending, StepStatus::Running, at)
            .await
            .expect_err("no step moves offline"),
    );
    refused(
        "finish_step",
        writer
            .finish_step(
                step,
                StepOutcome {
                    finished_at: at,
                    ..StepOutcome::default()
                },
            )
            .await
            .expect_err("no step settles offline"),
    );
    refused(
        "answer_gate",
        writer
            .answer_gate(step, GateOutcome::Approved, None, at)
            .await
            .expect_err("no gate is answered offline"),
    );
    refused(
        "select_fanout",
        writer
            .select_fanout(run, 0, 1, step, None)
            .await
            .expect_err("no winner is picked offline"),
    );
    refused(
        "supersede_step",
        writer
            .supersede_step(step)
            .await
            .expect_err("no step is superseded offline"),
    );

    // ---- trees and commits (ANA-2 §4.6) ----
    refused(
        "upsert_step_tree",
        writer
            .upsert_step_tree(
                step,
                &[RunStepTree {
                    run_step_id: step,
                    repo_id: repo,
                    mode: Isolation::Worktree,
                    path: "/tmp/offline/tree".to_owned(),
                    base_ref: "main".to_owned(),
                    dirty: false,
                }],
            )
            .await
            .expect_err("no tree is recorded offline"),
    );
    refused(
        "record_commits",
        writer
            .record_commits(
                step,
                &[RunStepCommit {
                    run_step_id: step,
                    repo_id: repo,
                    before_hash: "0".repeat(40),
                    after_hash: None,
                }],
            )
            .await
            .expect_err("no commit is recorded offline"),
    );

    // ---- documents, promotion, failure and close-out ----
    refused(
        "write_document",
        writer
            .write_document(NewDocument {
                id: DocumentId::new(),
                item_id: ids::HTUI_FEAT_1,
                kind: "plan".to_owned(),
                title: "Offline".to_owned(),
                body: "nothing".to_owned(),
                produced_by_step_id: Some(step),
                created_by: ids::USER,
                created_at: at,
            })
            .await
            .expect_err("no document is written offline"),
    );
    refused(
        "promote_step",
        writer
            .promote_step(step, at)
            .await
            .expect_err("no step is promoted offline"),
    );
    refused(
        "fail_run",
        writer
            .fail_run(run, "offline", at)
            .await
            .expect_err("no run is failed offline"),
    );
    refused(
        "close_out",
        writer
            .close_out(
                ids::HTUI_FEAT_1,
                NewDocument {
                    id: DocumentId::new(),
                    item_id: ids::HTUI_FEAT_1,
                    kind: "summary".to_owned(),
                    title: "Offline".to_owned(),
                    body: "nothing".to_owned(),
                    produced_by_step_id: Some(step),
                    created_by: ids::USER,
                    created_at: at,
                },
                &[],
            )
            .await
            .expect_err("no item is closed out offline"),
    );
    refused(
        "add_note",
        writer
            .add_note(NewNote {
                id: NoteId::new(),
                item_id: ids::HTUI_FEAT_1,
                body: "offline".to_owned(),
                created_by: ids::USER,
                box_id: Some(ids::BOX),
                via_step_id: Some(step),
                created_at: at,
            })
            .await
            .expect_err("no note is added offline"),
    );

    assert_eq!(
        pending_files(writer.dir()),
        0,
        "23 refusals write nothing to the buffer either"
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

/// The seam T21 walked through is **closed** by MOD-25: an offline backend hands out no writer at
/// all, so a chat off the server is refused rather than buffered. What has not changed is the
/// second answer - the server is still unreachable, which is what the re-dial ticker keys on.
#[tokio::test]
async fn an_offline_backend_hands_out_no_writer() {
    let root = tempfile::tempdir().expect("temp root");
    let cache = cache(root.path()).await;
    let backend = Backend::Offline {
        cache: cache.clone(),
        since: None,
    };

    assert!(
        backend.writer().is_none(),
        "no backend hands out Writer::Buffered since MOD-25 (`htui` is online-only)"
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
