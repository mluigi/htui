//! MOD-122 milestone 3 (plan D2-D5, D7; blueprint F2-F4): the Postgres-only half of the activity
//! feed. The rules both stores share run as `ACTIVITY_CASES` in `pg_conformance.rs`; what a
//! generic case cannot plant is here, with raw SQL: explicit transactions that move a row more
//! than once before commit (the deferred triggers), raw deletes of an item and of a free chat,
//! the workspace attribution, the kind `CHECK`, another user's rows, a run that finishes over a
//! `closed` item, a workspace delete, and (review R1 L1) a project or workspace delete beside a
//! transition still open on a second connection.
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes.

use htui_store::testkit as common;

use htui_core::fixtures::ids;
use htui_core::model::{
    ACTIVITY_PAGE, ActivityKind, ItemId, ItemKindId, ProjectId, RunId, StepId, UserId, WorkspaceId,
};
use htui_core::store::{ActivityStore as _, WriteStore as _};
use sqlx::PgPool;
use uuid::Uuid;

/// The planted items' key numbers start here, far above the fixture's.
const KEY_BASE: i32 = 9000;

/// Plants an `open` item of `project` under `kind` (`key_prefix` `prefix`), created by `user`.
async fn plant_item(
    pool: &PgPool,
    user: UserId,
    (project, kind, prefix): (ProjectId, ItemKindId, &str),
    number: i32,
) -> ItemId {
    let id = ItemId::new();
    sqlx::query(
        "INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, created_by) \
         VALUES ($1, $2, $3, $4, $5, 'planted', $6)",
    )
    .bind(id.as_uuid())
    .bind(project.as_uuid())
    .bind(kind.as_uuid())
    .bind(prefix)
    .bind(KEY_BASE + number)
    .bind(user.as_uuid())
    .execute(pool)
    .await
    .expect("plant an item");
    id
}

/// What a planted run is: its kind, item and recorded workspace.
struct RunSpec {
    kind: &'static str,
    project: ProjectId,
    item: Option<ItemId>,
    workspace: Option<WorkspaceId>,
    user: UserId,
}

impl RunSpec {
    fn graph(item: ItemId, user: UserId) -> Self {
        Self {
            kind: "graph",
            project: ids::PROJECT_HTUI,
            item: Some(item),
            workspace: None,
            user,
        }
    }

    fn chat(user: UserId) -> Self {
        Self {
            kind: "chat",
            project: ids::PROJECT_HTUI,
            item: None,
            workspace: None,
            user,
        }
    }
}

/// Plants a run at `status`. An insert writes no event (plan D2: only `step_permission` carries
/// an insert trigger).
async fn plant_run(pool: &PgPool, spec: &RunSpec, status: &str) -> RunId {
    let id = RunId::new();
    sqlx::query(
        "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                          graph_snapshot, started_by, workspace_id) \
         VALUES ($1, $2, $3, $4, 'manual', $5, $6, \
                 CASE WHEN $4 = 'graph' THEN '{}'::jsonb END, $7, $8)",
    )
    .bind(id.as_uuid())
    .bind(spec.project.as_uuid())
    .bind(spec.item.map(|item| item.as_uuid()))
    .bind(spec.kind)
    .bind(status)
    .bind(ids::BOX.as_uuid())
    .bind(spec.user.as_uuid())
    .bind(spec.workspace.map(|workspace| workspace.as_uuid()))
    .execute(pool)
    .await
    .expect("plant a run");
    id
}

/// Plants a step of `run` at `position`, named `name`, at `status`.
async fn plant_step(pool: &PgPool, run: RunId, position: i32, name: &str, status: &str) -> StepId {
    let id = StepId::new();
    sqlx::query(
        "INSERT INTO run_step (id, run_id, position, phase_name, status) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id.as_uuid())
    .bind(run.as_uuid())
    .bind(position)
    .bind(name)
    .bind(status)
    .execute(pool)
    .await
    .expect("plant a step");
    id
}

/// One statement, in autocommit: its own transaction, so its events are written at once.
async fn exec(pool: &PgPool, sql: &'static str, id: Uuid) {
    sqlx::query(sql)
        .bind(id)
        .execute(pool)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
}

/// Plants an `app_user` that is not this store's.
async fn plant_stranger(pool: &PgPool) -> UserId {
    let stranger = UserId::new();
    sqlx::query("INSERT INTO app_user (id, name) VALUES ($1, $2)")
        .bind(stranger.as_uuid())
        .bind(format!("stranger-{}", Uuid::now_v7().simple()))
        .execute(pool)
        .await
        .expect("plant another user");
    stranger
}

/// One stored row, any user's: `kind`, then the references and their copies.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    kind: String,
    workspace: Option<Uuid>,
    project: Option<Uuid>,
    project_slug: String,
    item: Option<Uuid>,
    item_key: Option<String>,
    run: Option<Uuid>,
    chat: bool,
    step: Option<Uuid>,
    step_name: Option<String>,
    text: String,
}

type RawRow = (
    String,
    Option<Uuid>,
    Option<Uuid>,
    String,
    Option<Uuid>,
    Option<String>,
    Option<Uuid>,
    bool,
    Option<Uuid>,
    Option<String>,
    String,
);

/// Every row, any user's, in id order.
async fn every_row(pool: &PgPool) -> Vec<Row> {
    let raw: Vec<RawRow> = sqlx::query_as(
        "SELECT kind, workspace_id, project_id, project_slug, item_id, item_key, run_id, chat, \
                step_id, step_name, text \
           FROM activity_event ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read activity_event");
    raw.into_iter()
        .map(
            |(
                kind,
                workspace,
                project,
                project_slug,
                item,
                item_key,
                run,
                chat,
                step,
                step_name,
                text,
            )| Row {
                kind,
                workspace,
                project,
                project_slug,
                item,
                item_key,
                run,
                chat,
                step,
                step_name,
                text,
            },
        )
        .collect()
}

/// The kinds of every row, in id order.
async fn kinds(pool: &PgPool) -> Vec<String> {
    every_row(pool)
        .await
        .into_iter()
        .map(|row| row.kind)
        .collect()
}

/// D2: in one explicit transaction the run parks *before* its step. The run's deferred call
/// re-reads the run at commit, finds a parked step and writes nothing; the step writes its gate.
/// One `gate`, no `interrupted`, whatever the order inside the transaction.
#[tokio::test]
async fn a_run_parked_before_its_step_writes_one_gate() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        1,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "running").await;
    let step = plant_step(&db.pool, run, 0, "implement", "running").await;

    let mut tx = db.pool.begin().await.expect("begin");
    sqlx::query("UPDATE run SET status = 'awaiting_approval' WHERE id = $1")
        .bind(run.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("park the run");
    sqlx::query("UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1")
        .bind(step.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("park the step");
    assert_eq!(
        kinds(&db.pool).await,
        Vec::<String>::new(),
        "deferred: nothing is written before commit"
    );
    tx.commit().await.expect("commit");

    assert_eq!(kinds(&db.pool).await, ["gate"], "one gate, no interrupted");
    let page = db
        .store
        .activity(None, ACTIVITY_PAGE)
        .await
        .expect("a read");
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].kind, ActivityKind::Gate);
    assert_eq!(page.events[0].step, Some(step));
    assert_eq!(page.pinned, page.events, "the waiting gate is pinned");

    db.drop_db().await;
}

/// D2: a run and its step park and unpark before commit. Every deferred call re-reads a row that
/// left the waiting state, so no needs-you event is written (and the run's `running` is not a
/// start: it did not come from `queued`).
#[tokio::test]
async fn a_park_undone_before_commit_writes_nothing() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        2,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "running").await;
    let step = plant_step(&db.pool, run, 0, "implement", "running").await;

    let mut tx = db.pool.begin().await.expect("begin");
    for (sql, id) in [
        (
            "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
            step.as_uuid(),
        ),
        (
            "UPDATE run SET status = 'awaiting_approval' WHERE id = $1",
            run.as_uuid(),
        ),
        (
            "UPDATE run_step SET status = 'running' WHERE id = $1",
            step.as_uuid(),
        ),
        (
            "UPDATE run SET status = 'running' WHERE id = $1",
            run.as_uuid(),
        ),
    ] {
        sqlx::query(sql)
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|err| panic!("{sql}: {err}"));
    }
    tx.commit().await.expect("commit");

    assert_eq!(
        kinds(&db.pool).await,
        Vec::<String>::new(),
        "a park undone before commit writes nothing"
    );

    db.drop_db().await;
}

/// F3: park → unpark → park inside one transaction queues two calls for `awaiting_approval`,
/// both re-reading the same parked row; the once-per-transaction index keeps one `gate`. A later
/// transaction parks again and writes a second one.
#[tokio::test]
async fn park_unpark_park_in_one_transaction_writes_one_gate() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        3,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "running").await;
    let step = plant_step(&db.pool, run, 0, "implement", "running").await;

    let mut tx = db.pool.begin().await.expect("begin");
    for status in ["awaiting_approval", "running", "awaiting_approval"] {
        sqlx::query("UPDATE run_step SET status = $2 WHERE id = $1")
            .bind(step.as_uuid())
            .bind(status)
            .execute(&mut *tx)
            .await
            .expect("move the step");
    }
    tx.commit().await.expect("commit");
    assert_eq!(kinds(&db.pool).await, ["gate"], "one gate per transaction");

    exec(
        &db.pool,
        "UPDATE run_step SET status = 'running' WHERE id = $1",
        step.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
        step.as_uuid(),
    )
    .await;
    assert_eq!(
        kinds(&db.pool).await,
        ["gate", "gate"],
        "a later transaction writes again"
    );

    db.drop_db().await;
}

/// D7: a raw `DELETE FROM item` (the cascade `delete_project` relies on) keeps only the
/// failures of its runs; their references go `NULL`, their copies stay.
#[tokio::test]
async fn deleting_an_item_keeps_only_its_failures() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        4,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "queued").await;
    let failing = plant_step(&db.pool, run, 0, "research", "running").await;
    let gated = plant_step(&db.pool, run, 1, "implement", "running").await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'running' WHERE id = $1",
        run.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'failed', gate_note = 'boom' WHERE id = $1",
        failing.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
        gated.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE item SET status = 'blocked' WHERE id = $1",
        item.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'failed', failure = 'gave up' WHERE id = $1",
        run.as_uuid(),
    )
    .await;
    assert_eq!(
        kinds(&db.pool).await,
        [
            "run_started",
            "step_failed",
            "gate",
            "unblock",
            "run_failed"
        ]
    );

    exec(&db.pool, "DELETE FROM item WHERE id = $1", item.as_uuid()).await;

    let key = format!("ANA-{}", KEY_BASE + 4);
    let failure = |kind: &str, step_name: Option<&str>, text: &str| Row {
        kind: kind.to_owned(),
        workspace: Some(ids::WORKSPACE_PLATFORM.as_uuid()),
        project: Some(ids::PROJECT_HTUI.as_uuid()),
        project_slug: "htui".to_owned(),
        item: None,
        item_key: Some(key.clone()),
        run: None,
        chat: false,
        step: None,
        step_name: step_name.map(str::to_owned),
        text: text.to_owned(),
    };
    assert_eq!(
        every_row(&db.pool).await,
        [
            failure("step_failed", Some("research"), "boom"),
            // `run_failed`'s step is the run's newest: the last one moved.
            failure("run_failed", Some("implement"), "gave up"),
        ],
        "only the failures stay, attributable by their copies"
    );

    db.drop_db().await;
}

/// D7: a raw `DELETE FROM run` of a free chat (no item) keeps only its failures, the run trigger's
/// half of the rule.
#[tokio::test]
async fn deleting_a_free_chat_keeps_only_its_failures() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let run = plant_run(&db.pool, &RunSpec::chat(me), "running").await;
    let failing = plant_step(&db.pool, run, 0, "chat", "running").await;
    let gated = plant_step(&db.pool, run, 1, "chat", "running").await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'failed' WHERE id = $1",
        failing.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
        gated.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'failed' WHERE id = $1",
        run.as_uuid(),
    )
    .await;
    assert_eq!(
        kinds(&db.pool).await,
        ["step_failed", "gate", "run_failed"],
        "a chat run writes no start"
    );

    exec(&db.pool, "DELETE FROM run WHERE id = $1", run.as_uuid()).await;

    let failure = |kind: &str| Row {
        kind: kind.to_owned(),
        workspace: Some(ids::WORKSPACE_PLATFORM.as_uuid()),
        project: Some(ids::PROJECT_HTUI.as_uuid()),
        project_slug: "htui".to_owned(),
        item: None,
        item_key: None,
        run: None,
        chat: true,
        step: None,
        step_name: Some("chat".to_owned()),
        text: String::new(),
    };
    assert_eq!(
        every_row(&db.pool).await,
        [failure("step_failed"), failure("run_failed")],
        "only the chat's failures stay"
    );

    db.drop_db().await;
}

/// D5: the run's recorded workspace wins; a scope-row item takes its owner workspace; a project
/// in two workspaces with no recorded one is `NULL`.
#[tokio::test]
async fn workspace_attribution() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();

    // `htui` sits in Platform only; the run recorded Graphics.
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        5,
    )
    .await;
    let recorded = RunSpec {
        workspace: Some(ids::WORKSPACE_GRAPHICS),
        ..RunSpec::graph(item, me)
    };
    let run = plant_run(&db.pool, &recorded, "running").await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'failed' WHERE id = $1",
        run.as_uuid(),
    )
    .await;

    // A scope-row item: its project is Platform's own.
    let scoped = plant_item(
        &db.pool,
        me,
        (
            ids::PROJECT_SCOPE_PLATFORM,
            ids::KIND_SCOPE_PLATFORM_WS,
            "WS",
        ),
        6,
    )
    .await;
    exec(
        &db.pool,
        "UPDATE item SET status = 'blocked' WHERE id = $1",
        scoped.as_uuid(),
    )
    .await;

    // `htui` joins Graphics too: a chat with no recorded workspace has none to take.
    sqlx::query("INSERT INTO workspace_project (workspace_id, project_id) VALUES ($1, $2)")
        .bind(ids::WORKSPACE_GRAPHICS.as_uuid())
        .bind(ids::PROJECT_HTUI.as_uuid())
        .execute(&db.pool)
        .await
        .expect("a second workspace for htui");
    let chat = plant_run(&db.pool, &RunSpec::chat(me), "running").await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'failed' WHERE id = $1",
        chat.as_uuid(),
    )
    .await;

    let rows = every_row(&db.pool).await;
    let attributed: Vec<(&str, Option<Uuid>, Option<Uuid>)> = rows
        .iter()
        .map(|row| (row.kind.as_str(), row.workspace, row.project))
        .collect();
    assert_eq!(
        attributed,
        [
            (
                "run_failed",
                Some(ids::WORKSPACE_GRAPHICS.as_uuid()),
                Some(ids::PROJECT_HTUI.as_uuid())
            ),
            (
                "unblock",
                Some(ids::WORKSPACE_PLATFORM.as_uuid()),
                Some(ids::PROJECT_SCOPE_PLATFORM.as_uuid())
            ),
            ("run_failed", None, Some(ids::PROJECT_HTUI.as_uuid())),
        ]
    );
    assert_eq!(rows[1].item_key.as_deref(), Some("WS-9006"));
    assert!(rows[2].chat);

    db.drop_db().await;
}

/// D3: `chk_activity_event_kind` refuses a kind the table does not list.
#[tokio::test]
async fn the_kind_check_refuses_an_unknown_kind() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let err = sqlx::query(
        "INSERT INTO activity_event (user_id, kind, project_slug, subject) \
         VALUES ($1, 'bell', 'htui', gen_random_uuid())",
    )
    .bind(db.store.this_user().as_uuid())
    .execute(&db.pool)
    .await
    .expect_err("an unknown kind is refused");
    let refused = err.as_database_error().expect("a database error");
    assert_eq!(refused.code().as_deref(), Some("23514"));
    assert_eq!(refused.constraint(), Some("chk_activity_event_kind"));

    db.drop_db().await;
}

/// Another user's events are stored but never read (D5; `MemStore`'s twin is in `mem.rs`).
#[tokio::test]
async fn another_users_events_are_never_read() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let stranger = plant_stranger(&db.pool).await;
    let run = plant_run(&db.pool, &RunSpec::chat(stranger), "running").await;
    let step = plant_step(&db.pool, run, 0, "chat", "running").await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
        step.as_uuid(),
    )
    .await;
    assert_eq!(
        kinds(&db.pool).await,
        ["gate"],
        "the stranger's gate is written"
    );
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM activity_event")
        .fetch_one(&db.pool)
        .await
        .expect("read the owner");
    assert_eq!(owner, stranger.as_uuid(), "it is the run starter's");

    let page = db
        .store
        .activity(None, ACTIVITY_PAGE)
        .await
        .expect("a read");
    assert_eq!(page.events, Vec::new(), "not mine: not read");
    assert_eq!(page.pinned, Vec::new(), "not mine: not pinned");
    assert_eq!(
        db.store
            .activity(Some(0), ACTIVITY_PAGE)
            .await
            .expect("a read")
            .events,
        Vec::new()
    );

    db.drop_db().await;
}

/// D3: a graph run `done` over a `closed` item is `run_finished`; over an open one, `close_out`
/// (`MemStore`'s twin is in `mem.rs`: close-out refuses an item with a live run, so a generic
/// case cannot reach it).
#[tokio::test]
async fn a_run_done_over_a_closed_item_is_run_finished() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let closed = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        7,
    )
    .await;
    let open = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        8,
    )
    .await;
    let over_closed = plant_run(&db.pool, &RunSpec::graph(closed, me), "running").await;
    let over_open = plant_run(&db.pool, &RunSpec::graph(open, me), "running").await;
    exec(
        &db.pool,
        "UPDATE item SET status = 'closed', resolution = 'done' WHERE id = $1",
        closed.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'done' WHERE id = $1",
        over_closed.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'done' WHERE id = $1",
        over_open.as_uuid(),
    )
    .await;

    let rows = every_row(&db.pool).await;
    let shapes: Vec<(&str, Option<Uuid>, Option<Uuid>)> = rows
        .iter()
        .map(|row| (row.kind.as_str(), row.run, row.item))
        .collect();
    assert_eq!(
        shapes,
        [
            (
                "run_finished",
                Some(over_closed.as_uuid()),
                Some(closed.as_uuid())
            ),
            ("close_out", Some(over_open.as_uuid()), Some(open.as_uuid())),
        ]
    );

    db.drop_db().await;
}

/// D7: `delete_workspace` clears `workspace_id` on every surviving event of that workspace,
/// whatever its kind (`ON DELETE SET NULL`; `MemStore`'s twin is in `mem.rs`).
#[tokio::test]
async fn delete_workspace_clears_its_events_workspace() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        9,
    )
    .await;
    // A workspace of its own: the fixture's two hold live runs, which `delete_workspace` refuses.
    let scratch = WorkspaceId::new();
    sqlx::query(
        "INSERT INTO workspace (id, slug, name, created_by) VALUES ($1, 'scratch', 'Scratch', $2)",
    )
    .bind(scratch.as_uuid())
    .bind(me.as_uuid())
    .execute(&db.pool)
    .await
    .expect("plant a workspace");
    let recorded = RunSpec {
        workspace: Some(scratch),
        ..RunSpec::graph(item, me)
    };
    let run = plant_run(&db.pool, &recorded, "queued").await;
    let step = plant_step(&db.pool, run, 0, "implement", "running").await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'running' WHERE id = $1",
        run.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run_step SET status = 'failed' WHERE id = $1",
        step.as_uuid(),
    )
    .await;
    exec(
        &db.pool,
        "UPDATE run SET status = 'done' WHERE id = $1",
        run.as_uuid(),
    )
    .await;
    let recorded = Some(scratch.as_uuid());
    assert_eq!(
        every_row(&db.pool)
            .await
            .iter()
            .map(|row| (row.kind.as_str(), row.workspace))
            .collect::<Vec<_>>(),
        [
            ("run_started", recorded),
            ("step_failed", recorded),
            ("close_out", recorded)
        ]
    );

    db.store
        .delete_workspace(scratch)
        .await
        .expect("the planted workspace goes");

    let rows = every_row(&db.pool).await;
    assert_eq!(
        rows.iter()
            .map(|row| (row.kind.as_str(), row.workspace, row.run))
            .collect::<Vec<_>>(),
        [
            ("run_started", None, Some(run.as_uuid())),
            ("step_failed", None, Some(run.as_uuid())),
            ("close_out", None, Some(run.as_uuid())),
        ],
        "the events stay (their project stays), their workspace goes"
    );

    db.drop_db().await;
}

/// `EXPLAIN` of the inner scan of `PgStore::activity`'s pinned read (`pg/activity.rs`, a
/// `query_as!` literal this file cannot import): the rows it reads and the order it wants.
const PINNED_EXPLAIN: &str = "EXPLAIN \
    SELECT DISTINCT ON (subject) id, kind, subject \
      FROM activity_event \
     WHERE user_id = $1 \
       AND kind IN ('gate', 'judge_failed', 'selection', 'unblock', 'interrupted', 'permission') \
     ORDER BY subject, id DESC \
     LIMIT $2";

/// D4: the pinned read can be served by `idx_activity_event_needs_you`. Informative, not a pin
/// on plan shape: with sequential scans off the plan is printed, and it names the partial index.
#[tokio::test]
async fn the_pinned_query_can_use_the_partial_index() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mut conn = db.pool.acquire().await.expect("a connection");
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .expect("no sequential scans");
    let plan: Vec<String> = sqlx::query_scalar(PINNED_EXPLAIN)
        .bind(db.store.this_user().as_uuid())
        .bind(i64::from(htui_core::model::PINNED_LIMIT))
        .fetch_all(&mut *conn)
        .await
        .expect("explain the pinned read");
    let plan = plan.join("\n");
    println!("{plan}");
    assert!(
        plan.contains("idx_activity_event_needs_you"),
        "the pinned read names the partial index: {plan}"
    );
    drop(conn);

    db.drop_db().await;
}

/// The server's `deadlock_timeout` plus half a second: a transition that waits this long before
/// its commit lets the delete's own deadlock check pass first, so a cycle closed at the commit is
/// the transition's to detect, and the transition (not the retrying delete) is the one aborted.
async fn past_the_deadlock_check(pool: &PgPool) -> std::time::Duration {
    let millis: i32 =
        sqlx::query_scalar("SELECT setting::int FROM pg_settings WHERE name = 'deadlock_timeout'")
            .fetch_one(pool)
            .await
            .expect("read deadlock_timeout");
    let millis = u64::try_from(millis).expect("a positive timeout");
    std::time::Duration::from_millis(millis + 500)
}

/// Review R1 L1: `sql` (one status move of row `id`, whose deferred trigger writes an event about
/// a row `delete` removes) runs in an open transaction; `delete` starts beside it, and the
/// transition commits after [`past_the_deadlock_check`]. Both must succeed: the delete locks the
/// event sources before the parent, waits for the commit, and retries the `40001` it then meets.
async fn commit_beside<T>(
    pool: &PgPool,
    sql: &'static str,
    id: Uuid,
    delete: impl Future<Output = htui_core::store::Result<T>>,
) -> T {
    let pause = past_the_deadlock_check(pool).await;
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
    let (deleted, committed) = tokio::join!(delete, async {
        tokio::time::sleep(pause).await;
        tx.commit().await
    });
    committed.expect("the transition commits: its trigger never deadlocks against the delete");
    deleted.expect("the delete waits for the transition, then retries")
}

/// Review R1 L1: an item blocked in an open transaction while its project is deleted. Before the
/// fix the delete held the project row, its cascade waited on the item, and the item's deferred
/// trigger needed `FOR KEY SHARE` on the project at commit: a deadlock.
#[tokio::test]
async fn a_project_delete_beside_an_item_transition_does_not_deadlock() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        10,
    )
    .await;

    commit_beside(
        &db.pool,
        "UPDATE item SET status = 'blocked' WHERE id = $1",
        item.as_uuid(),
        db.store.delete_project(ids::PROJECT_HTUI),
    )
    .await;

    assert_eq!(
        kinds(&db.pool).await,
        Vec::<String>::new(),
        "the unblock went with its item"
    );

    db.drop_db().await;
}

/// Review R1 L1, the step's half: a step parked in an open transaction while its project is
/// deleted. The cascade reaches the step through the run, so the step is locked too.
#[tokio::test]
async fn a_project_delete_beside_a_step_transition_does_not_deadlock() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        11,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "running").await;
    let step = plant_step(&db.pool, run, 0, "implement", "running").await;

    commit_beside(
        &db.pool,
        "UPDATE run_step SET status = 'awaiting_approval' WHERE id = $1",
        step.as_uuid(),
        db.store.delete_project(ids::PROJECT_HTUI),
    )
    .await;

    assert_eq!(
        kinds(&db.pool).await,
        Vec::<String>::new(),
        "the gate went with its run"
    );

    db.drop_db().await;
}

/// Review R1 L1, the workspace's half: a run that recorded a workspace fails in an open
/// transaction while that workspace is deleted. The delete's `SET NULL` on `run.workspace_id`
/// waited on the run, and the run's trigger needed `FOR KEY SHARE` on the workspace at commit.
#[tokio::test]
async fn a_workspace_delete_beside_a_run_transition_does_not_deadlock() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        12,
    )
    .await;
    let scratch = WorkspaceId::new();
    sqlx::query(
        "INSERT INTO workspace (id, slug, name, created_by) VALUES ($1, 'scratch', 'Scratch', $2)",
    )
    .bind(scratch.as_uuid())
    .bind(me.as_uuid())
    .execute(&db.pool)
    .await
    .expect("plant a workspace");
    let recorded = RunSpec {
        workspace: Some(scratch),
        ..RunSpec::graph(item, me)
    };
    let run = plant_run(&db.pool, &recorded, "running").await;

    commit_beside(
        &db.pool,
        "UPDATE run SET status = 'failed' WHERE id = $1",
        run.as_uuid(),
        db.store.delete_workspace(scratch),
    )
    .await;

    assert_eq!(
        every_row(&db.pool)
            .await
            .iter()
            .map(|row| (row.kind.as_str(), row.workspace, row.run))
            .collect::<Vec<_>>(),
        [("run_failed", None, Some(run.as_uuid()))],
        "the failure stays, its workspace gone with the workspace"
    );

    db.drop_db().await;
}

/// Review R1 L1, second round: an item blocked in an open transaction while it has a live run.
/// The unblock event names that run, so the trigger's foreign-key check needs `FOR KEY SHARE` on
/// the run at commit: a delete that held the run `FOR UPDATE` while it waited on the item closed a
/// cycle there.
#[tokio::test]
async fn a_project_delete_beside_an_item_transition_with_a_live_run_does_not_deadlock() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        13,
    )
    .await;
    plant_run(&db.pool, &RunSpec::graph(item, me), "awaiting_approval").await;

    commit_beside(
        &db.pool,
        "UPDATE item SET status = 'blocked' WHERE id = $1",
        item.as_uuid(),
        db.store.delete_project(ids::PROJECT_HTUI),
    )
    .await;

    assert_eq!(
        kinds(&db.pool).await,
        Vec::<String>::new(),
        "the unblock went with its item and its run"
    );

    db.drop_db().await;
}

/// Review R1 L1, second round: a run failed in an open transaction over a step an earlier
/// transaction wrote. The `run_failed` event names that step, which the transition never locked,
/// so the trigger's foreign-key check needs `FOR KEY SHARE` on it at commit: a delete that held the
/// step `FOR UPDATE` while it waited on the run closed a cycle there.
#[tokio::test]
async fn a_project_delete_beside_a_run_failed_over_an_older_step_does_not_deadlock() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let me = db.store.this_user();
    let item = plant_item(
        &db.pool,
        me,
        (ids::PROJECT_HTUI, ids::KIND_HTUI_ANA, "ANA"),
        14,
    )
    .await;
    let run = plant_run(&db.pool, &RunSpec::graph(item, me), "running").await;
    plant_step(&db.pool, run, 0, "implement", "done").await;

    commit_beside(
        &db.pool,
        "UPDATE run SET status = 'failed' WHERE id = $1",
        run.as_uuid(),
        db.store.delete_project(ids::PROJECT_HTUI),
    )
    .await;

    assert_eq!(
        every_row(&db.pool)
            .await
            .iter()
            .map(|row| (
                row.kind.as_str(),
                row.project,
                row.run,
                row.step,
                row.step_name.as_deref()
            ))
            .collect::<Vec<_>>(),
        [("run_failed", None, None, None, Some("implement"))],
        "the failure stays, named by its copies, its sources gone with the project"
    );

    db.drop_db().await;
}
