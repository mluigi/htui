//! ANA-9 §11 criteria 2, 3 and the Postgres half of 4, against a real server (blueprint E.3).
//!
//! Criterion 2 (§11.2) is the key counter under concurrency: two independent pools minting the
//! same `(project, prefix)` produce consecutive numbers, a rolled-back mint leaves no gap, and the
//! importer variant keeps `last_value` at or above the highest explicit number.
//! Criterion 3 (§11.3) is the compare-and-set race: two edits from one version, exactly one
//! `Updated` and one `Diverged`.
//! Criterion 4 (§11.4), Postgres half: 5 000 `session_event` rows replay in `seq` order with
//! identical payloads.
//!
//! Every case creates and drops its own database; with `HTUI_TEST_DATABASE_URL` unset each one
//! prints `common::SKIP` and passes (plan D13).
#![cfg(feature = "demo")]

use htui_store::testkit as common;

use chrono::{DateTime, Utc};
use futures::future::join_all;
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, ItemFilter, ItemId, ItemKindId, ItemPatch, NewItem,
    ProjectId, Status, StepId, Transport,
};
use htui_core::store::{ReadStore as _, UpdateOutcome, WriteStore as _};
use htui_store::PgStore;
use sqlx::Row as _;
use sqlx::postgres::PgPool;

/// The prefix the concurrency cases mint under: its own `item_kind`, so the counter starts empty
/// and the expected numbers are `1..=n` rather than "whatever the fixture left".
const RACE_PREFIX: &str = "RACE";

/// Inserts an `item_kind` with [`RACE_PREFIX`] into the fixture's `htui` project.
///
/// A fresh kind means a fresh `(project, prefix)` counter: no `item_key_counter` row exists for it
/// until the first mint, which is the branch of the §7.1 `ON CONFLICT` the fixture's kinds never
/// take.
async fn race_kind(pool: &PgPool) -> ItemKindId {
    let id = ItemKindId::new();
    sqlx::query!(
        "INSERT INTO item_kind (id, project_id, prefix, name, description, default_graph_id, \
         position) VALUES ($1, $2, $3, 'race', '', $4, 99)",
        id.as_uuid(),
        ids::PROJECT_HTUI.as_uuid(),
        RACE_PREFIX,
        ids::GRAPH_HTUI_FEAT.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("insert the race item_kind");
    id
}

/// A mint request for [`race_kind`]'s kind, authored by the fixture user.
fn race_item(kind_id: ItemKindId, title: &str) -> NewItem {
    NewItem {
        id: ItemId::new(),
        project_id: ids::PROJECT_HTUI,
        kind_id,
        title: title.to_owned(),
        body: String::new(),
        required_tags: Vec::new(),
        touched_paths: Vec::new(),
        priority: 0,
        step_graph_id: None,
        created_by: ids::USER,
        box_id: Some(ids::BOX),
    }
}

/// `item_key_counter.last_value` for `(project, prefix)`, or `None` when no row exists yet.
async fn counter(pool: &PgPool, project: ProjectId, prefix: &str) -> Option<i32> {
    sqlx::query_scalar!(
        "SELECT last_value FROM item_key_counter WHERE project_id = $1 AND prefix = $2",
        project.as_uuid(),
        prefix,
    )
    .fetch_optional(pool)
    .await
    .expect("read item_key_counter")
}

/// How many `item_revision` rows one item has.
async fn revision_count(pool: &PgPool, item: ItemId) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM item_revision WHERE item_id = $1"#,
        item.as_uuid(),
    )
    .fetch_one(pool)
    .await
    .expect("count item_revision")
}

/// `run_step.prompt_digest` and `run_step.usage`, the two columns §6.1 does not return.
///
/// Runtime-checked rather than `query!`, like the other reads this file adds: a new `query!` string
/// would need a `cargo sqlx prepare` pass, and `.sqlx/` belongs to the crate, not to a test.
async fn step_usage(pool: &PgPool, step: StepId) -> (Option<String>, Option<serde_json::Value>) {
    let row = sqlx::query("SELECT prompt_digest, usage FROM run_step WHERE id = $1")
        .bind(step.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read run_step");
    (row.get("prompt_digest"), row.get("usage"))
}

/// One registry row by id, out of the inherent `agents()` read (MOD-2 plan D3).
async fn agent_row(store: &PgStore, id: AgentId) -> Agent {
    store
        .agents()
        .await
        .expect("agents must not fail")
        .into_iter()
        .find(|row| row.agent.id == id)
        .expect("the agent is in the registry")
        .agent
}

/// §11.2, first clause: two independent pools minting the same prefix produce consecutive numbers.
///
/// Two `PgStore::connect` calls against the same database are two independent pools (plan D13's
/// "a second connection in the race tests"), so the only thing serialising the hundred mints is
/// the row lock the `ON CONFLICT DO UPDATE` of §7.1 takes on the counter row.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_mints_produce_consecutive_numbers() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let kind = race_kind(&db.pool).await;

    let left = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let right = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("third pool")
        .store;

    let mints = join_all((0..100).map(|n| {
        let store = if n % 2 == 0 { &left } else { &right };
        async move { store.mint_item(race_item(kind, &format!("race {n}"))).await }
    }))
    .await;

    let mut numbers: Vec<i32> = mints
        .into_iter()
        .map(|minted| minted.expect("every mint lands").key_number)
        .collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (1..=100).collect::<Vec<i32>>(),
        "a hundred concurrent mints are exactly 1..=100: no duplicate, no gap (§11.2)"
    );
    assert_eq!(
        counter(&db.pool, ids::PROJECT_HTUI, RACE_PREFIX).await,
        Some(100),
        "the counter ends at the highest number minted"
    );

    db.drop_db().await;
}

/// §11.2, second clause: a mint inside a rolled-back transaction leaves no gap.
///
/// The §7.1 statement is one CTE, so the counter upsert and the `item` insert share the enclosing
/// transaction: rolling it back takes the counter back with it and the next mint reuses the
/// number. The statement text below is `write.rs`'s, spelled out here so the test pins the
/// property of the SQL rather than of the Rust wrapper.
#[tokio::test(flavor = "multi_thread")]
async fn a_rolled_back_mint_leaves_no_gap() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let kind = race_kind(&db.pool).await;
    let rolled_back = ItemId::new();

    let mut tx = db.pool.begin().await.expect("begin");
    let number = sqlx::query_scalar!(
        r#"
        WITH c AS (
            INSERT INTO item_key_counter (project_id, prefix, last_value)
            SELECT $2, k.prefix, 1 FROM item_kind k WHERE k.id = $3 AND k.project_id = $2
            ON CONFLICT (project_id, prefix)
            DO UPDATE SET last_value = item_key_counter.last_value + 1
            RETURNING prefix, last_value
        ), i AS (
            INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body,
                              priority, required_tags, touched_paths, step_graph_id, created_by)
            SELECT $1, $2, $3, c.prefix, c.last_value, 'rolled back', '', 0,
                   '{}'::text[], '{}'::text[], NULL::uuid, $4 FROM c
            RETURNING id, version, title, body, required_tags, key_number
        ), r AS (
            INSERT INTO item_revision (item_id, version, title, body, required_tags,
                                       author_id, box_id, reason)
            SELECT id, version, title, body, required_tags, $4, $5, 'created' FROM i
            RETURNING item_id
        )
        SELECT i.key_number AS "key_number!" FROM i, r
        "#,
        rolled_back.as_uuid(),
        ids::PROJECT_HTUI.as_uuid(),
        kind.as_uuid(),
        ids::USER.as_uuid(),
        ids::BOX.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("the in-transaction mint lands");
    assert_eq!(number, 1, "the first mint of a fresh prefix is number 1");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        counter(&db.pool, ids::PROJECT_HTUI, RACE_PREFIX).await,
        None,
        "the rollback took the counter row with it"
    );
    assert!(
        db.store
            .item(rolled_back)
            .await
            .expect("read must not fail")
            .is_none(),
        "the rollback left no item row behind"
    );

    let landed = db
        .store
        .mint_item(race_item(kind, "after the rollback"))
        .await
        .expect("the next mint must land");
    assert_eq!(
        landed.key_number, 1,
        "the next mint reuses the rolled-back number: no gap (§11.2)"
    );
    assert_eq!(landed.key, "RACE-1", "and the key is assembled from it");

    db.drop_db().await;
}

/// §11.2, third clause: the importer variant leaves `last_value >= max(key_number)`.
///
/// The importer statement itself is MOD-8's; what this pins is the counter rule it relies on, so
/// the ordinary mint that follows an import cannot collide with an imported number.
#[tokio::test(flavor = "multi_thread")]
async fn the_importer_variant_keeps_the_counter_above_max() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let kind = race_kind(&db.pool).await;

    let imported = sqlx::query_scalar!(
        r#"
        WITH c AS (
            INSERT INTO item_key_counter (project_id, prefix, last_value)
            SELECT $2, k.prefix, $4 FROM item_kind k WHERE k.id = $3 AND k.project_id = $2
            ON CONFLICT (project_id, prefix)
            DO UPDATE SET last_value = GREATEST(item_key_counter.last_value, $4)
            RETURNING prefix, last_value
        ), i AS (
            INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, title, body,
                              priority, required_tags, touched_paths, step_graph_id, created_by)
            SELECT $1, $2, $3, c.prefix, $4, 'imported', '', 0,
                   '{}'::text[], '{}'::text[], NULL::uuid, $5 FROM c
            RETURNING key_number
        )
        SELECT i.key_number AS "key_number!" FROM i
        "#,
        ItemId::new().as_uuid(),
        ids::PROJECT_HTUI.as_uuid(),
        kind.as_uuid(),
        500_i32,
        ids::USER.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("the importer mint lands");
    assert_eq!(imported, 500, "the importer supplies the number");
    assert_eq!(
        counter(&db.pool, ids::PROJECT_HTUI, RACE_PREFIX).await,
        Some(500),
        "GREATEST keeps the counter at or above the imported number (§11.2)"
    );

    let next = db
        .store
        .mint_item(race_item(kind, "after the import"))
        .await
        .expect("the next ordinary mint must land");
    assert_eq!(
        next.key_number, 501,
        "the next ordinary mint continues above the imported number"
    );

    db.drop_db().await;
}

/// §11.3: two compare-and-set edits from one version yield exactly one `Updated` and one
/// `Diverged`, and the loser is handed the head and the ancestor it edited from (§4.2).
#[tokio::test(flavor = "multi_thread")]
async fn two_edits_from_one_version_diverge_exactly_once() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let before = db
        .store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    let start = before.version;

    let left = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let right = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("third pool")
        .store;

    let patch = |title: &str| ItemPatch {
        title: Some(title.to_owned()),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "edited".to_owned(),
        ..ItemPatch::default()
    };
    let (one, two) = tokio::join!(
        left.update_item(before.id, start, patch("Left")),
        right.update_item(before.id, start, patch("Right")),
    );

    let outcomes = [
        one.expect("the first edit must not fail"),
        two.expect("the second edit must not fail"),
    ];
    let updated: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UpdateOutcome::Updated(head) => Some(head),
            UpdateOutcome::Diverged { .. } => None,
        })
        .collect();
    let diverged: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UpdateOutcome::Diverged { head, ancestor } => Some((head, ancestor)),
            UpdateOutcome::Updated(_) => None,
        })
        .collect();

    assert_eq!(updated.len(), 1, "exactly one edit lands (§11.3)");
    assert_eq!(diverged.len(), 1, "exactly one edit diverges (§11.3)");
    assert_eq!(
        updated[0].version,
        start + 1,
        "the winner bumped the version once"
    );

    let (head, ancestor) = diverged[0];
    assert_eq!(
        head.version,
        start + 1,
        "the loser is shown the committed head (§11.3)"
    );
    assert_eq!(
        head.title, updated[0].title,
        "the head the loser sees is the winner's row"
    );
    assert_eq!(
        ancestor.version, start,
        "the ancestor is the revision the loser edited from (§11.3)"
    );
    assert_eq!(
        ancestor.item_id, before.id,
        "the ancestor belongs to the edited item"
    );
    assert_eq!(
        ancestor.title, before.title,
        "the ancestor carries the pre-race title"
    );

    db.drop_db().await;
}

/// A diverged edit writes nothing: after the race the item has one revision per landed version.
#[tokio::test(flavor = "multi_thread")]
async fn a_diverged_edit_writes_no_revision() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let before = db
        .store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    let start = before.version;
    assert_eq!(
        revision_count(&db.pool, before.id).await,
        i64::from(start),
        "the fixture wrote one revision per version"
    );

    let other = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let patch = |title: &str| ItemPatch {
        title: Some(title.to_owned()),
        author_id: ids::USER,
        box_id: Some(ids::BOX),
        reason: "edited".to_owned(),
        ..ItemPatch::default()
    };
    let (one, two) = tokio::join!(
        db.store.update_item(before.id, start, patch("Left")),
        other.update_item(before.id, start, patch("Right")),
    );
    one.expect("the first edit must not fail");
    two.expect("the second edit must not fail");

    assert_eq!(
        revision_count(&db.pool, before.id).await,
        i64::from(start) + 1,
        "the winner wrote one revision and the loser wrote none"
    );

    db.drop_db().await;
}

/// The status compare-and-set never touches `version`, and `closed_at` follows the current status
/// in both directions (§4.2, blueprint H.12).
#[tokio::test(flavor = "multi_thread")]
async fn status_cas_never_bumps_version() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let before = db
        .store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    assert_eq!(before.status, Status::Open, "fixture precondition");
    assert_eq!(before.closed_at, None, "fixture precondition");

    assert!(
        db.store
            .transition(before.id, Status::Open, Status::Done)
            .await
            .expect("transition must not fail"),
        "open -> done matches"
    );
    let done = db
        .store
        .item(before.id)
        .await
        .expect("read must not fail")
        .expect("the item still exists");
    assert_eq!(done.status, Status::Done, "the status moved");
    assert_eq!(done.version, before.version, "version untouched");
    assert!(done.closed_at.is_some(), "a terminal move sets closed_at");

    assert!(
        db.store
            .transition(before.id, Status::Done, Status::Open)
            .await
            .expect("transition must not fail"),
        "done -> open matches"
    );
    let reopened = db
        .store
        .item(before.id)
        .await
        .expect("read must not fail")
        .expect("the item still exists");
    assert_eq!(reopened.closed_at, None, "a reopen clears closed_at");
    assert_eq!(
        reopened.version, before.version,
        "neither move bumped the version"
    );
    assert_eq!(
        revision_count(&db.pool, before.id).await,
        i64::from(before.version),
        "no transition wrote a revision"
    );

    assert!(
        !db.store
            .transition(before.id, Status::Done, Status::Closed)
            .await
            .expect("transition must not fail"),
        "a stale `from` is refused rather than applied"
    );
    let missing = db
        .store
        .transition(ItemId::new(), Status::Open, Status::Done)
        .await;
    assert!(
        matches!(missing, Err(htui_core::store::StoreError::NotFound { .. })),
        "an unknown item is NotFound, not `false`, got {missing:?}"
    );

    db.drop_db().await;
}

/// §11.4, Postgres half: 5 000 events inserted out of order replay in `seq` order, content equal.
#[tokio::test(flavor = "multi_thread")]
async fn five_thousand_events_replay_in_seq_order() {
    const COUNT: i32 = 5_000;
    /// Coprime with [`COUNT`] (5 000 = 2^3 * 5^4), so `n * STRIDE mod COUNT` is a permutation.
    const STRIDE: i64 = 2_777;

    let Some(db) = common::demo_db().await else {
        return;
    };
    let step = ids::STEP_IMPL;
    assert!(
        db.store
            .step_events(step)
            .await
            .expect("read must not fail")
            .is_none(),
        "the fixture caches no log for the implement step"
    );

    let shuffled: Vec<i32> = (0..i64::from(COUNT))
        .map(|n| i32::try_from(n * STRIDE % i64::from(COUNT)).expect("in range"))
        .collect();
    sqlx::query!(
        "INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, payload, at) \
         SELECT $1, s, s / 100, 'assistant_text', 'agent', NULL, \
                jsonb_build_object('seq', s, 'text', 'event ' || s), now() \
           FROM UNNEST($2::int[]) AS s",
        step.as_uuid(),
        &shuffled[..],
    )
    .execute(&db.pool)
    .await
    .expect("insert 5 000 session_event rows");

    let events = db
        .store
        .step_events(step)
        .await
        .expect("read must not fail")
        .expect("the step now has a cached log");
    assert_eq!(
        events.len(),
        usize::try_from(COUNT).expect("in range"),
        "every inserted event comes back (§11.4)"
    );
    assert_eq!(
        events.iter().map(|event| event.seq).collect::<Vec<i32>>(),
        (0..COUNT).collect::<Vec<i32>>(),
        "events replay in seq order regardless of insert order (§11.4)"
    );
    for event in &events {
        assert_eq!(
            event.payload,
            serde_json::json!({ "seq": event.seq, "text": format!("event {}", event.seq) }),
            "payload content survives the round trip (§11.4)"
        );
        assert_eq!(event.run_step_id, step, "only the step's own events");
        assert_eq!(event.turn, event.seq / 100, "turn survives too");
        assert!(event.raw.is_none(), "raw is off");
    }

    db.drop_db().await;
}

/// MOD-2 plan D4, **online first**: the two rows `start_chat_run` mints are the two rows the
/// offline buffer's upload would address for the same chat, so the two paths converge on one pair
/// instead of colliding.
///
/// The chat is started **online**, then the very same events are buffered to a `pending/` file and
/// uploaded, which is the offline path arriving late. Both `ON CONFLICT (id) DO NOTHING` clauses
/// must make that upload a no-op on the `run` and `run_step` rows and the
/// `PRIMARY KEY (run_step_id, seq)` a no-op on every event, so the database still holds exactly one
/// run, one step and one copy of each event, with the online path's column values untouched.
///
/// What converges is the row *count*; the columns belong to whichever path landed first. The
/// mirror image - upload first, online start second - is
/// [`an_offline_first_chat_keeps_the_uploaded_columns`].
///
/// The column values `store::conformance` cannot see - `run_step.usage`, `run_step.prompt_digest`
/// and every column of the minted pair - are asserted here in SQL, because §6.1 returns none of
/// them (plan D15(a)).
#[cfg(feature = "demo")]
#[tokio::test(flavor = "multi_thread")]
async fn chat_run_rows_converge_with_the_offline_mint() {
    use htui_core::model::{ChatRunSpec, EventKind, EventRole, RunStatus, SessionEvent};
    use htui_store::cache::pending::{append_pending, seal_pending, upload_pending};

    let Some(db) = common::demo_db().await else {
        return;
    };
    let workspaces = db.store.workspaces().await.expect("workspaces");
    let platform = workspaces
        .iter()
        .find(|ws| ws.workspace_id == ids::WORKSPACE_PLATFORM)
        .expect("the Platform workspace");
    let scope = htui_core::model::Scope::from_workspace(platform);
    let before = db.store.active_runs(&scope).await.expect("active_runs");

    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        db.store.this_box(),
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    );
    db.store
        .start_chat_run(&chat)
        .await
        .expect("the chat mint must land");

    let events: Vec<SessionEvent> = (0..3)
        .map(|seq| SessionEvent {
            run_step_id: chat.step_id,
            seq,
            turn: 0,
            kind: EventKind::AssistantText,
            role: EventRole::Agent,
            tool_call_id: None,
            payload: serde_json::json!({ "text": format!("chunk {seq}") }),
            raw: None,
            at: chat.started_at,
        })
        .collect();
    assert_eq!(
        db.store
            .append_events(&events)
            .await
            .expect("the events must land"),
        3,
        "three new rows are three inserts"
    );
    db.store
        .set_step_usage(
            chat.step_id,
            serde_json::json!({ "input_tokens": 11, "output_tokens": 22 }),
            Some("d1ge57".to_owned()),
        )
        .await
        .expect("the usage write must land");

    // The `run` row, column by column, against `cache/pending.rs`'s upload values.
    let run = sqlx::query!(
        r#"SELECT item_id, kind, mode, status, target_box_id, executing_box_id,
                  started_by, queued_at, started_at, finished_at
             FROM run WHERE id = $1"#,
        chat.run_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("the run row exists");
    assert_eq!(run.item_id, None, "run.item_id is NULL for a chat");
    assert_eq!(run.kind, "chat", "run.kind");
    assert_eq!(run.mode, "manual", "run.mode");
    assert_eq!(run.status, "running", "run.status, not the upload's 'done'");
    assert_eq!(run.finished_at, None, "run.finished_at is open");
    assert_eq!(
        run.executing_box_id,
        Some(db.store.this_box().as_uuid()),
        "a chat executes on the box it was started from"
    );
    assert_eq!(run.queued_at, chat.started_at, "run.queued_at");
    assert_eq!(run.started_at, Some(chat.started_at), "run.started_at");
    assert_eq!(run.started_by, ids::USER.as_uuid(), "run.started_by");

    let step = sqlx::query!(
        r#"SELECT run_id, position, attempt, fanout_index, phase_name, agent_id, model, status,
                  prompt_digest, usage, started_at, finished_at
             FROM run_step WHERE id = $1"#,
        chat.step_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("the run_step row exists");
    assert_eq!(step.run_id, chat.run_id.as_uuid(), "run_step.run_id");
    assert_eq!(step.position, 0, "run_step.position");
    assert_eq!(step.attempt, 1, "run_step.attempt");
    assert_eq!(step.fanout_index, 0, "run_step.fanout_index");
    assert_eq!(step.phase_name, "chat", "run_step.phase_name");
    assert_eq!(step.status, "running", "run_step.status");
    assert_eq!(
        step.agent_id,
        Some(ids::AGENT_CLAUDE.as_uuid()),
        "run_step.agent_id comes from the spec"
    );
    assert_eq!(step.model.as_deref(), Some("sonnet"), "run_step.model");
    assert_eq!(step.finished_at, None, "run_step.finished_at is open");
    assert_eq!(
        step.prompt_digest.as_deref(),
        Some("d1ge57"),
        "set_step_usage wrote the digest"
    );
    assert_eq!(
        step.usage,
        Some(serde_json::json!({ "input_tokens": 11, "output_tokens": 22 })),
        "set_step_usage wrote the usage"
    );

    assert_eq!(
        db.store.active_runs(&scope).await.expect("active_runs"),
        before + 1,
        "a running chat counts towards the active-run indicator"
    );

    // The offline path arriving late for the same chat: same ids, same events, own file.
    let root = tempfile::tempdir().expect("temp cache root");
    append_pending(root.path(), chat.project_id, chat.run_id, &events)
        .await
        .expect("the buffer must be written");
    seal_pending(root.path(), chat.project_id, chat.run_id)
        .await
        .expect("the chat ends, so its buffer is sealed (H-1)");
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("the upload must not fail"),
        1,
        "one buffer file landed"
    );

    let counts = sqlx::query!(
        r#"SELECT (SELECT COUNT(*) FROM run      WHERE id = $1)          AS "runs!",
                  (SELECT COUNT(*) FROM run_step WHERE run_id = $1)      AS "steps!",
                  (SELECT COUNT(*) FROM session_event WHERE run_step_id = $2) AS "events!""#,
        chat.run_id.as_uuid(),
        chat.step_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("count the converged rows");
    assert_eq!(counts.runs, 1, "the upload inserted no second run (D4)");
    assert_eq!(counts.steps, 1, "the upload inserted no second step (D4)");
    assert_eq!(
        counts.events, 3,
        "the primary key swallowed the replayed events (§4.3)"
    );

    let after_upload = sqlx::query!(
        "SELECT status, finished_at FROM run WHERE id = $1",
        chat.run_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("the run row survives");
    assert_eq!(
        after_upload.status, "running",
        "`DO NOTHING` left the online row's status alone"
    );
    assert_eq!(
        after_upload.finished_at, None,
        "and did not close a run that is still open"
    );

    // Closing it is what takes it back out of the active count (assumption A1).
    let closed_at = chat.started_at + chrono::TimeDelta::seconds(5);
    db.store
        .finish_chat_run(chat.run_id, chat.step_id, RunStatus::Done, closed_at)
        .await
        .expect("the close must land");
    assert_eq!(
        db.store.active_runs(&scope).await.expect("active_runs"),
        before,
        "a closed chat stops counting"
    );
    let done = sqlx::query!(
        r#"SELECT r.status AS "run_status!", r.finished_at AS "run_finished?",
                  s.status AS "step_status!", s.finished_at AS "step_finished?"
             FROM run r JOIN run_step s ON s.id = $2 WHERE r.id = $1"#,
        chat.run_id.as_uuid(),
        chat.step_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("both rows exist");
    assert_eq!(done.run_status, "done", "run.status");
    assert_eq!(done.step_status, "done", "run_step.status by the same name");
    assert_eq!(done.run_finished, Some(closed_at), "run.finished_at");
    assert_eq!(done.step_finished, Some(closed_at), "run_step.finished_at");

    db.drop_db().await;
}

/// The other half of MOD-2 plan D4's convergence claim: the same chat arriving **offline first**.
///
/// The buffer is uploaded while the chat is unknown to the server, and only then does the online
/// `start_chat_run` replay. Row-count convergence still holds - one `run`, one `run_step`, one copy
/// of each event - but every column is the *upload's*, because `ON CONFLICT (id) DO NOTHING` makes
/// the second path a no-op: `status = 'done'`, timestamps taken from the events' `at`, and
/// `agent_id` / `model` NULL, since the pending line format carries neither. So an offline-first
/// chat keeps a NULL `agent_id` even though the spec that replayed over it names an agent. Carrying
/// `agent_id` / `model` in the pending format belongs to the offline session path, MOD-2 milestone
/// 4 (plan D16); until then this asymmetry is the honest guarantee, and this test is what pins it.
#[tokio::test(flavor = "multi_thread")]
async fn an_offline_first_chat_keeps_the_uploaded_columns() {
    use htui_core::model::{ChatRunSpec, EventKind, EventRole, SessionEvent};
    use htui_store::cache::pending::{append_pending, seal_pending, upload_pending};

    let Some(db) = common::demo_db().await else {
        return;
    };

    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        db.store.this_box(),
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    );
    assert!(
        chat.agent_id.is_some() && chat.model.is_some(),
        "the spec that replays over the upload does name an agent and a model"
    );

    // Event stamps deliberately away from `chat.started_at`, so a row carrying the spec's stamp is
    // distinguishable from one carrying the buffer's.
    let first_at = chat.started_at + chrono::TimeDelta::seconds(30);
    let events: Vec<SessionEvent> = (0..3)
        .map(|seq| SessionEvent {
            run_step_id: chat.step_id,
            seq,
            turn: 0,
            kind: EventKind::AssistantText,
            role: EventRole::Agent,
            tool_call_id: None,
            payload: serde_json::json!({ "text": format!("offline chunk {seq}") }),
            raw: None,
            at: first_at + chrono::TimeDelta::seconds(i64::from(seq)),
        })
        .collect();
    let last_at = first_at + chrono::TimeDelta::seconds(2);

    let root = tempfile::tempdir().expect("temp cache root");
    append_pending(root.path(), chat.project_id, chat.run_id, &events)
        .await
        .expect("the buffer must be written");
    seal_pending(root.path(), chat.project_id, chat.run_id)
        .await
        .expect("the chat ends, so its buffer is sealed (H-1)");
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("the upload must not fail"),
        1,
        "one buffer file landed"
    );

    // The online path arriving late for the same chat: accepted, and a no-op on both rows.
    db.store
        .start_chat_run(&chat)
        .await
        .expect("a replayed online mint must not fail");

    let counts = sqlx::query!(
        r#"SELECT (SELECT COUNT(*) FROM run      WHERE id = $1)          AS "runs!",
                  (SELECT COUNT(*) FROM run_step WHERE run_id = $1)      AS "steps!",
                  (SELECT COUNT(*) FROM session_event WHERE run_step_id = $2) AS "events!""#,
        chat.run_id.as_uuid(),
        chat.step_id.as_uuid(),
    )
    .fetch_one(&db.pool)
    .await
    .expect("count the converged rows");
    assert_eq!(
        counts.runs, 1,
        "the online mint inserted no second run (D4)"
    );
    assert_eq!(
        counts.steps, 1,
        "the online mint inserted no second step (D4)"
    );
    assert_eq!(counts.events, 3, "and no event was duplicated (§4.3)");

    let run =
        sqlx::query("SELECT status, queued_at, started_at, finished_at FROM run WHERE id = $1")
            .bind(chat.run_id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("the run row exists");
    assert_eq!(
        run.get::<String, _>("status"),
        "done",
        "the upload's terminal status stands: the online mint's 'running' never landed"
    );
    assert_eq!(
        run.get::<Option<DateTime<Utc>>, _>("finished_at"),
        Some(last_at),
        "run.finished_at is the last event's `at`, not NULL as the online mint writes"
    );
    assert_eq!(
        run.get::<DateTime<Utc>, _>("queued_at"),
        first_at,
        "run.queued_at is the first event's `at`, not the spec's started_at"
    );
    assert_eq!(
        run.get::<Option<DateTime<Utc>>, _>("started_at"),
        Some(first_at),
        "run.started_at likewise"
    );

    let step = sqlx::query(
        "SELECT position, attempt, fanout_index, phase_name, agent_id, model, status, \
                started_at, finished_at FROM run_step WHERE id = $1",
    )
    .bind(chat.step_id.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the run_step row exists");
    // What the two paths do agree on: the shape of the pair.
    assert_eq!(step.get::<i32, _>("position"), 0, "run_step.position");
    assert_eq!(step.get::<i32, _>("attempt"), 1, "run_step.attempt");
    assert_eq!(
        step.get::<i32, _>("fanout_index"),
        0,
        "run_step.fanout_index"
    );
    assert_eq!(
        step.get::<String, _>("phase_name"),
        "chat",
        "run_step.phase_name"
    );
    // What they do not agree on, and what `DO NOTHING` therefore decides by arrival order.
    assert_eq!(
        step.get::<Option<uuid::Uuid>, _>("agent_id"),
        None,
        "run_step.agent_id stays NULL: the pending format carries no agent (MOD-2 milestone 4)"
    );
    assert_eq!(
        step.get::<Option<String>, _>("model"),
        None,
        "run_step.model stays NULL for the same reason"
    );
    assert_eq!(
        step.get::<String, _>("status"),
        "done",
        "run_step.status is the upload's"
    );
    assert_eq!(
        step.get::<Option<DateTime<Utc>>, _>("started_at"),
        Some(first_at),
        "run_step.started_at is the step's first event"
    );
    assert_eq!(
        step.get::<Option<DateTime<Utc>>, _>("finished_at"),
        Some(last_at),
        "run_step.finished_at is its last"
    );

    db.drop_db().await;
}

/// §11 criteria 3 and 7 for a chat that happened **offline** (MOD-2 plan D36).
///
/// The recorder writes `run_step.prompt_digest` and `run_step.usage` through `set_step_usage`
/// while a chat is online; offline it cannot, because the buffer's line format holds
/// `session_event` columns only. So the uploader derives both from the rows it is uploading: the
/// digest from the `prompt` row's payload, the usage from `UsageTotals::from_rows` - the same
/// summing rule the recorder ran, which is what makes an uploaded step indistinguishable from one
/// recorded online rather than one with an empty `usage`.
#[tokio::test(flavor = "multi_thread")]
async fn an_uploaded_offline_chat_carries_its_digest_and_usage() {
    use htui_core::model::{ChatRunSpec, EventKind, EventRole, SessionEvent, UsageTotals};
    use htui_store::cache::pending::{append_pending, seal_pending, upload_pending};

    /// The `sha256` shape milestone 9 writes; the uploader copies it verbatim.
    const DIGEST: &str = "9f2c1b7a4e5d6038c1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708";

    let Some(db) = common::demo_db().await else {
        return;
    };

    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        db.store.this_box(),
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        None,
    );
    let at = |seq: i64| chat.started_at + chrono::TimeDelta::seconds(seq);
    let row =
        |seq: i32, kind: EventKind, role: EventRole, payload: serde_json::Value| SessionEvent {
            run_step_id: chat.step_id,
            seq,
            turn: 0,
            kind,
            role,
            tool_call_id: None,
            payload,
            raw: None,
            at: at(i64::from(seq)),
        };

    let events = vec![
        row(
            0,
            EventKind::Prompt,
            EventRole::Htui,
            serde_json::json!({ "text": "offline?", "sections": [], "digest": DIGEST }),
        ),
        row(
            1,
            EventKind::AssistantText,
            EventRole::Agent,
            serde_json::json!({ "text": "yes" }),
        ),
        // Deltas, not cumulative totals: "take the last row" would answer 3 input tokens.
        row(
            2,
            EventKind::Usage,
            EventRole::Agent,
            serde_json::json!({ "input_tokens": 10, "output_tokens": 4, "cost_micros": 100 }),
        ),
        row(
            3,
            EventKind::AssistantText,
            EventRole::Agent,
            serde_json::json!({ "text": "and buffered" }),
        ),
        row(
            4,
            EventKind::Usage,
            EventRole::Agent,
            serde_json::json!({ "input_tokens": 20, "cache_read_tokens": 7, "cost_micros": 250 }),
        ),
        row(
            5,
            EventKind::Usage,
            EventRole::Agent,
            serde_json::json!({ "input_tokens": 3, "output_tokens": 1, "cost_micros": 1 }),
        ),
        row(
            6,
            EventKind::Done,
            EventRole::Agent,
            serde_json::json!({ "stop_reason": "end_turn" }),
        ),
    ];
    let expected_usage = UsageTotals::from_rows(&events).to_value();
    assert_eq!(
        expected_usage,
        serde_json::json!({
            "input_tokens": 33,
            "output_tokens": 5,
            "cache_read_tokens": 7,
            "cache_write_tokens": serde_json::Value::Null,
            "cost_micros": 351,
        }),
        "the fixture is the sum of the deltas, spelled out so the test cannot agree with a bug"
    );

    let root = tempfile::tempdir().expect("temp cache root");
    append_pending(root.path(), chat.project_id, chat.run_id, &events)
        .await
        .expect("the buffer must be written");
    seal_pending(root.path(), chat.project_id, chat.run_id)
        .await
        .expect("the chat ends (H-1)");
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("the upload must not fail"),
        1,
    );

    assert_eq!(
        step_usage(&db.pool, chat.step_id).await,
        (Some(DIGEST.to_owned()), Some(expected_usage.clone())),
        "both columns are the ones the online recorder would have written",
    );

    // The pass after the upload: the file is gone, so there is nothing to find and nothing to
    // change - the return value is a count of files that landed, not of buffers that ever existed.
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("a pass with no buffer is not an error"),
        0,
    );

    // The same buffer arriving twice - a crash between the commit and the delete - is the
    // `ON CONFLICT (id) DO NOTHING` case: the file goes, and neither column moves.
    append_pending(root.path(), chat.project_id, chat.run_id, &events)
        .await
        .expect("the buffer is written again");
    seal_pending(root.path(), chat.project_id, chat.run_id)
        .await
        .expect("and sealed again");
    let events_before = common::count(&db.pool, "session_event").await;
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("the second upload"),
        1,
    );
    assert_eq!(
        common::count(&db.pool, "session_event").await,
        events_before,
        "no event was duplicated",
    );
    assert_eq!(
        step_usage(&db.pool, chat.step_id).await,
        (Some(DIGEST.to_owned()), Some(expected_usage)),
        "and the step's two columns are exactly what the first pass wrote",
    );

    db.drop_db().await;
}

/// A buffer holding the same `(run_step_id, seq)` line twice loads it **once** (M-1).
///
/// `session_event` is protected by `ON CONFLICT (run_step_id, seq) DO NOTHING`, so a duplicated
/// line never becomes a duplicated row - but `run_step.usage` is summed from the parsed lines
/// before any of that, and a `usage` row counted twice lands a step whose totals are double the
/// conversation's (criterion 7). The loader therefore drops a repeat itself, keeping the first
/// occurrence, so it is robust to a duplicate however it got onto the disk.
#[tokio::test(flavor = "multi_thread")]
async fn a_duplicated_line_in_a_buffer_is_counted_once() {
    use htui_core::model::{ChatRunSpec, EventKind, EventRole, SessionEvent, UsageTotals};
    use htui_store::cache::pending::{append_pending, seal_pending, upload_pending};

    let Some(db) = common::demo_db().await else {
        return;
    };

    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        db.store.this_box(),
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        None,
    );
    let row =
        |seq: i32, kind: EventKind, role: EventRole, payload: serde_json::Value| SessionEvent {
            run_step_id: chat.step_id,
            seq,
            turn: 0,
            kind,
            role,
            tool_call_id: None,
            payload,
            raw: None,
            at: chat.started_at + chrono::TimeDelta::seconds(i64::from(seq)),
        };
    let events = vec![
        row(
            0,
            EventKind::Prompt,
            EventRole::Htui,
            serde_json::json!({ "text": "twice?", "digest": "d" }),
        ),
        row(
            1,
            EventKind::Usage,
            EventRole::Agent,
            serde_json::json!({ "input_tokens": 10, "output_tokens": 4, "cost_micros": 100 }),
        ),
    ];
    let once = UsageTotals::from_rows(&events).to_value();

    let root = tempfile::tempdir().expect("temp cache root");
    // The same lines, twice, in one buffer: what a crash in the middle of the old appending seal
    // left behind, and what any second writer of the same rows would leave.
    for _ in 0..2 {
        append_pending(root.path(), chat.project_id, chat.run_id, &events)
            .await
            .expect("the buffer must be written");
    }
    seal_pending(root.path(), chat.project_id, chat.run_id)
        .await
        .expect("the chat ends (H-1)");
    assert_eq!(
        upload_pending(&db.pool, root.path(), db.store.this_box(), ids::USER)
            .await
            .expect("the upload must not fail"),
        1,
    );

    assert_eq!(
        step_usage(&db.pool, chat.step_id).await.1,
        Some(once),
        "the repeated usage row was summed once, not twice",
    );
    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM session_event WHERE run_step_id = $1 ORDER BY seq")
            .bind(chat.step_id.as_uuid())
            .fetch_all(&db.pool)
            .await
            .expect("the events");
    assert_eq!(seqs, vec![0, 1], "and the log holds each seq exactly once");

    db.drop_db().await;
}

/// Plan D15(b) on Postgres: `set_step_usage` overwrites `run_step.usage` on every call, and
/// `COALESCE($3, prompt_digest)` keeps the stored digest when the caller supplies `None`.
///
/// `store::conformance`'s `set_step_usage_writes_usage_and_digest` can assert only the `Ok` /
/// `NotFound` shape, because §6.1 returns neither column (plan D15(a)); `store::mem`'s
/// `set_step_usage_writes_usage_every_time_and_the_digest_only_when_supplied` is the memory half of
/// the rule and this is the Postgres half. The third write is what keeps the second honest: the
/// column *is* writable, so surviving a `None` is a property of `COALESCE`, not of a frozen column.
#[tokio::test(flavor = "multi_thread")]
async fn set_step_usage_keeps_the_digest_a_none_call_does_not_supply() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let step = ids::STEP_IMPL;

    db.store
        .set_step_usage(
            step,
            serde_json::json!({ "input_tokens": 11 }),
            Some("d1ge57".to_owned()),
        )
        .await
        .expect("the first write must land");
    assert_eq!(
        step_usage(&db.pool, step).await,
        (
            Some("d1ge57".to_owned()),
            Some(serde_json::json!({ "input_tokens": 11 })),
        ),
        "a write that supplies a digest stores both columns"
    );

    db.store
        .set_step_usage(
            step,
            serde_json::json!({ "input_tokens": 30, "output_tokens": 40 }),
            None,
        )
        .await
        .expect("a digest-free write must land");
    assert_eq!(
        step_usage(&db.pool, step).await,
        (
            Some("d1ge57".to_owned()),
            Some(serde_json::json!({ "input_tokens": 30, "output_tokens": 40 })),
        ),
        "`COALESCE($3, prompt_digest)`: usage is replaced, the digest a `None` does not supply \
         survives (D15(b))"
    );

    db.store
        .set_step_usage(step, serde_json::json!({}), Some("f00d".to_owned()))
        .await
        .expect("a second digest write must land");
    assert_eq!(
        step_usage(&db.pool, step).await,
        (Some("f00d".to_owned()), Some(serde_json::json!({}))),
        "a supplied digest does overwrite: the column is not merely immutable"
    );

    db.drop_db().await;
}

/// `PgStore::upsert_agent` read back through the inherent `agents()` (MOD-2 plan D3): a second
/// write of the same `agent.id` updates the row **in place** - one row, the new column values - and
/// `created_at` survives it, because the insert supplies it and the `DO UPDATE SET` list does not
/// (§5.7). `updated_at` is the migration's `BEFORE UPDATE` trigger's, not the caller's.
///
/// `store::conformance`'s `upsert_agent_by_id_name_unique` can only assert that the second write is
/// *accepted*: `upsert_agent` is a [`htui_core::store::WriteStore`] method while the registry read
/// is inherent, so no conformance case can read the row back. This is that read-back.
#[tokio::test(flavor = "multi_thread")]
async fn upsert_agent_updates_in_place_and_keeps_created_at() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let id = AgentId::new();
    let created: DateTime<Utc> = "2020-01-02T03:04:05.000006Z"
        .parse()
        .expect("a microsecond-precision literal, which is `timestamptz`'s resolution");
    let inserted = Agent {
        id,
        name: "tester".to_owned(),
        transport: Transport::Cli,
        launch: serde_json::json!({ "argv": ["tester"], "env": {} }),
        models: vec!["small".to_owned()],
        default_model: Some("small".to_owned()),
        billing: Billing::PerToken,
        enabled: true,
        settings: serde_json::json!({ "cli": { "stream": "fake" } }),
        created_at: created,
        updated_at: created,
    };
    db.store
        .upsert_agent(&inserted)
        .await
        .expect("the insert must land");
    assert_eq!(
        agent_row(&db.store, id).await,
        inserted,
        "the insert round-trips column for column, both stamps included: the trigger is \
         `BEFORE UPDATE` only"
    );

    // Postgres' own clock, so the trigger's stamp is compared against the server that set it.
    let before: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&db.pool)
        .await
        .expect("read the server clock");

    let far_future: DateTime<Utc> = "2030-11-12T13:14:15.000016Z"
        .parse()
        .expect("a literal timestamp");
    let updated = Agent {
        name: "tester-renamed".to_owned(),
        transport: Transport::Acp,
        launch: serde_json::json!({ "argv": ["tester", "--acp"], "env": { "HTUI": "1" } }),
        models: vec!["small".to_owned(), "large".to_owned()],
        default_model: Some("large".to_owned()),
        billing: Billing::Subscription,
        enabled: false,
        settings: serde_json::json!({ "acp": { "permission_modes": true } }),
        created_at: far_future,
        updated_at: far_future,
        ..inserted.clone()
    };
    db.store
        .upsert_agent(&updated)
        .await
        .expect("the update must land");

    let registry = db.store.agents().await.expect("agents must not fail");
    assert_eq!(
        registry.iter().filter(|row| row.agent.id == id).count(),
        1,
        "the second write updated the row in place: `ON CONFLICT (id)`, no second row"
    );
    assert_eq!(
        registry.len(),
        3,
        "the fixture's two agents plus this one, and the rename took no other row with it"
    );
    let after = registry
        .into_iter()
        .find(|row| row.agent.id == id)
        .expect("the agent is in the registry");
    assert!(
        after.on_box.is_none(),
        "upsert_agent writes no agent_box row"
    );

    assert_eq!(
        after.agent,
        Agent {
            created_at: created,
            updated_at: after.agent.updated_at,
            ..updated.clone()
        },
        "every column in the `DO UPDATE SET` list took the second write's value, and `created_at` \
         is still the insert's"
    );
    assert_ne!(
        after.agent.created_at, far_future,
        "`created_at` is not in the `SET` list, so the update could not move it"
    );
    assert!(
        after.agent.updated_at > inserted.updated_at
            && after.agent.updated_at >= before
            && after.agent.updated_at != far_future,
        "the `BEFORE UPDATE` trigger owns `updated_at`: it is this write's server clock, not the \
         {far_future} the caller supplied, got {}",
        after.agent.updated_at
    );

    db.drop_db().await;
}

/// The `PgStore` inherent reads T4's `Backend` dispatches over, against the fixture.
#[tokio::test(flavor = "multi_thread")]
async fn inherent_reads_answer_the_fixture() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let workspaces = db
        .store
        .workspaces()
        .await
        .expect("workspaces must not fail");
    assert_eq!(
        workspaces
            .iter()
            .map(|ws| ws.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Graphics", "Platform"],
        "workspaces are ordered by name"
    );
    let platform = workspaces
        .iter()
        .find(|ws| ws.workspace_id == ids::WORKSPACE_PLATFORM)
        .expect("the Platform workspace is here");
    assert_eq!(
        platform
            .projects
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["htui", "agy"],
        "a workspace's projects are ordered by workspace_project.position"
    );

    let scope = htui_core::model::Scope::from_workspace(platform);
    assert_eq!(
        db.store
            .projects(&scope)
            .await
            .expect("projects must not fail")
            .iter()
            .map(|p| p.project_id)
            .collect::<Vec<_>>(),
        scope.project_ids,
        "the scope's projects come back in scope order"
    );
    assert_eq!(
        db.store
            .active_runs(&scope)
            .await
            .expect("active_runs must not fail"),
        1,
        "the fixture has exactly one active run"
    );

    let info = db
        .store
        .box_info()
        .await
        .expect("box_info must not fail")
        .expect("this box is registered");
    assert_eq!(
        info.box_id,
        db.store.this_box(),
        "box_info answers for this box"
    );

    // MOD-2 plan D3: the registry read is inherent too, because `agent` / `agent_box` are not
    // mirrored. The demo loader inserts the two `agent` rows and no `agent_box` row at all, so
    // every summary is unprobed on this box.
    let agents = db.store.agents().await.expect("agents must not fail");
    assert_eq!(
        agents
            .iter()
            .map(|row| row.agent.name.as_str())
            .collect::<Vec<_>>(),
        vec!["agy", "claude"],
        "the registry comes back ordered by agent.name"
    );
    assert!(
        agents.iter().all(|row| row.on_box.is_none()),
        "no agent_box row exists for this box, so every summary is unprobed"
    );
    assert_eq!(
        agents
            .iter()
            .find(|row| row.agent.name == "claude")
            .expect("claude is registered")
            .agent
            .id,
        ids::AGENT_CLAUDE,
        "the summary carries the whole agent row, id included"
    );

    // `ItemFilter::text` is a literal substring, not a pattern: `MemStore` uses `contains` and the
    // mirror uses `instr`, so `%` and `_` must not act as LIKE wildcards here either.
    let text_filter = |needle: &str| ItemFilter {
        text: Some(needle.to_owned()),
        ..ItemFilter::default()
    };
    assert!(
        !db.store
            .items(&scope, &text_filter("EAT-"))
            .await
            .expect("items must not fail")
            .is_empty(),
        "a literal substring of a key still matches"
    );
    for wildcard in ["_", "%", "%EAT%", "F_AT-1"] {
        assert!(
            db.store
                .items(&scope, &text_filter(wildcard))
                .await
                .expect("items must not fail")
                .is_empty(),
            "`{wildcard}` is a literal needle, and no fixture key or title contains it"
        );
    }

    db.drop_db().await;
}

/// MOD-2 D44: the ANA-4 §4.6 snapshot survives `agent_box.probe` unchanged, and MOD-4's skip
/// predicate can read `probe->>'status'` straight out of SQL (`docs/ANA-2.md` §7).
///
/// `htui-store` cannot name `htui_agent::probe::ProbeSnapshot` - the dependency runs the other way
/// (D44) - so the document is hand-written here in the shape that type serialises to. That is the
/// point of the case: the column is opaque to this crate, and anything the driver writes has to
/// come back byte for byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_probe_snapshot_round_trips_through_agent_box() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let snapshot = serde_json::json!({
        "transport": "acp",
        "resolved": {
            "command": "/usr/bin/node",
            "args": ["/opt/claude-code-acp/dist/index.js"],
            "env": { "CLAUDE_CODE_EXECUTABLE": "/usr/bin/claude" },
        },
        "tools": { "claude": "2.1.263", "claude_agent_acp": null, "node": "22.19.0" },
        "handshake": {
            "at": "2026-09-08T12:00:00Z",
            "protocol_version": 1,
            "agent_name": "claude-code-acp",
            "agent_version": "0.7.1",
            "capabilities": { "loadSession": false },
            "auth_methods": [],
        },
        "status": "ready",
        "stderr_tail": null,
        "source": "probe",
    });
    let row = AgentBox {
        agent_id: ids::AGENT_CLAUDE,
        box_id: db.store.this_box(),
        enabled: true,
        version: Some("0.7.1".to_owned()),
        path: Some("/usr/bin/node".to_owned()),
        probed_at: Some(Utc::now()),
        quota: None,
        quota_at: None,
        updated_at: Utc::now(),
        probe: Some(snapshot.clone()),
    };
    db.store
        .upsert_agent_box(&row)
        .await
        .expect("the probe row lands");

    let on_box = db
        .store
        .agents()
        .await
        .expect("agents must not fail")
        .into_iter()
        .find(|summary| summary.agent.id == ids::AGENT_CLAUDE)
        .expect("claude is registered")
        .on_box
        .expect("this box now has an agent_box row");
    assert_eq!(
        on_box.probe.as_ref(),
        Some(&snapshot),
        "the JSONB document comes back exactly as it went in"
    );

    let status: Option<String> =
        sqlx::query_scalar("SELECT probe->>'status' FROM agent_box WHERE agent_id = $1")
            .bind(ids::AGENT_CLAUDE.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read probe->>'status'");
    assert_eq!(
        status.as_deref(),
        Some("ready"),
        "MOD-4's skip condition reads the status out of SQL, not out of Rust"
    );

    db.store
        .upsert_agent_box(&AgentBox { probe: None, ..row })
        .await
        .expect("the cleared row lands");
    let cleared: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT probe FROM agent_box WHERE agent_id = $1")
            .bind(ids::AGENT_CLAUDE.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read probe");
    assert_eq!(cleared, None, "a `None` probe writes SQL NULL, not `null`");
    assert_eq!(
        db.store
            .agents()
            .await
            .expect("agents must not fail")
            .into_iter()
            .find(|summary| summary.agent.id == ids::AGENT_CLAUDE)
            .and_then(|summary| summary.on_box)
            .and_then(|on_box| on_box.probe),
        None,
        "and the read comes back `None`"
    );

    db.drop_db().await;
}

/// MOD-2 plan D67: `set_agent_box_quota` writes the two quota columns of an existing row and
/// leaves `agent_box.probe` byte-identical.
///
/// The claim the narrow setter exists for, asserted in SQL rather than assumed: a latch runs
/// inside a chat while a re-probe of the same row may be running beside it, so the statement that
/// writes the allowance must not be the statement that writes the §4.6 snapshot. `updated_at` is
/// read too, because the two backends have to agree on it - `MemStore` bumps it by hand and
/// Postgres has `agent_box` in the `BEFORE UPDATE` trigger loop (`0001_init.sql:577`), so no
/// `SET updated_at` is needed here.
#[tokio::test(flavor = "multi_thread")]
async fn set_agent_box_quota_leaves_probe_byte_identical() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    // A document big enough that "byte-identical" is a claim about a payload and not about a
    // two-key object JSONB might normalise either way.
    let snapshot = serde_json::json!({
        "transport": "acp",
        "resolved": {
            "command": "/usr/bin/node",
            "args": ["/opt/claude-code-acp/dist/index.js", "--stdio"],
            "env": { "CLAUDE_CODE_EXECUTABLE": "/usr/bin/claude" },
        },
        "tools": { "claude": "2.1.263", "claude_agent_acp": null, "node": "22.19.0" },
        "handshake": {
            "at": "2026-09-08T12:00:00Z",
            "protocol_version": 1,
            "agent_name": "claude-code-acp",
            "agent_version": "0.7.1",
            "capabilities": { "loadSession": false },
            "auth_methods": [],
        },
        "status": "ready",
        "stderr_tail": null,
        "source": "probe",
    });
    // Microsecond-precision literals, `timestamptz`'s resolution — the
    // `upsert_agent_updates_in_place_and_keeps_created_at` precedent above. `Utc::now()` carries
    // nanoseconds Postgres rounds away, and this case compares stamps for equality.
    let probed_at: DateTime<Utc> = "2026-09-10T06:00:00.000123Z"
        .parse()
        .expect("a microsecond-precision literal");
    db.store
        .upsert_agent_box(&AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: db.store.this_box(),
            enabled: true,
            version: Some("0.7.1".to_owned()),
            path: Some("/usr/bin/node".to_owned()),
            probed_at: Some(probed_at),
            quota: None,
            quota_at: None,
            updated_at: probed_at,
            probe: Some(snapshot.clone()),
        })
        .await
        .expect("the probe row lands");
    let before: DateTime<Utc> =
        sqlx::query_scalar("SELECT updated_at FROM agent_box WHERE agent_id = $1 AND box_id = $2")
            .bind(ids::AGENT_CLAUDE.as_uuid())
            .bind(db.store.this_box().as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read updated_at");

    let quota = serde_json::json!({
        "source": "acp_meta_rate_limit",
        "billing": "subscription",
        "status": "allowed",
        "exhausted": false,
        "windows": [{ "id": "five_hour", "utilization": 0.11 }],
        "spend": { "session_micros": 351, "currency": "USD" },
        "observed_at": "2026-09-10T09:00:00Z",
    });
    let quota_at: DateTime<Utc> = "2026-09-10T09:00:00.000456Z"
        .parse()
        .expect("a microsecond-precision literal");
    db.store
        .set_agent_box_quota(
            ids::AGENT_CLAUDE,
            db.store.this_box(),
            quota.clone(),
            quota_at,
        )
        .await
        .expect("the latch lands on the probed row");

    let row = sqlx::query(
        "SELECT probe, quota, quota_at, updated_at, enabled, version, path, probed_at \
         FROM agent_box WHERE agent_id = $1 AND box_id = $2",
    )
    .bind(ids::AGENT_CLAUDE.as_uuid())
    .bind(db.store.this_box().as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("read the latched row");
    assert_eq!(
        row.get::<Option<serde_json::Value>, _>("probe").as_ref(),
        Some(&snapshot),
        "the §4.6 snapshot survives the latch byte for byte (D67)"
    );
    assert_eq!(
        row.get::<Option<serde_json::Value>, _>("quota").as_ref(),
        Some(&quota),
        "the document is what the call passed"
    );
    assert_eq!(
        row.get::<Option<DateTime<Utc>>, _>("quota_at"),
        Some(quota_at),
        "and `quota_at` with it"
    );
    assert!(
        row.get::<bool, _>("enabled"),
        "`enabled` is not one of the two columns"
    );
    assert_eq!(
        row.get::<Option<String>, _>("version").as_deref(),
        Some("0.7.1"),
        "nor is `version`"
    );
    assert_eq!(
        row.get::<Option<String>, _>("path").as_deref(),
        Some("/usr/bin/node"),
        "nor is `path`"
    );
    assert_eq!(
        row.get::<Option<DateTime<Utc>>, _>("probed_at"),
        Some(probed_at),
        "nor is `probed_at`: a latch is not a probe"
    );
    assert!(
        row.get::<DateTime<Utc>, _>("updated_at") > before,
        "`updated_at` moved, and the `BEFORE UPDATE` trigger - not the statement - moved it"
    );

    let missing = db
        .store
        .set_agent_box_quota(AgentId::new(), db.store.this_box(), quota, quota_at)
        .await;
    assert!(
        matches!(
            missing,
            Err(htui_core::store::StoreError::NotFound {
                entity: "agent_box",
                ..
            })
        ),
        "`rows_affected() == 0` is the `NotFound`; there is no insert path, got {missing:?}"
    );

    db.drop_db().await;
}

/// MOD-2 plan D74: `upsert_agent_box` can neither **set** nor **clear** `agent_box.quota` /
/// `quota_at`. `set_agent_box_quota` is their only writer.
///
/// The proof has to be SQL-level, not trait-level: what is being removed is a
/// `quota = EXCLUDED.quota` line out of an `ON CONFLICT ... DO UPDATE SET` list and a column out
/// of an `INSERT` list, and no `WriteStore` call can see either. Both paths are read back with a
/// raw `SELECT`.
///
/// The bug this closes is not a race that needs unlucky timing. `probe::agent_box_row` used to
/// read `quota` off a row fetched at chat start and hand it to a statement whose `SET` list wrote
/// it back, so every latch that landed in between was discarded - a lost update by construction.
/// `COALESCE(EXCLUDED.quota, agent_box.quota)` was considered and rejected: it would still let an
/// upsert *set* the column, and it would make clearing it impossible.
#[tokio::test(flavor = "multi_thread")]
async fn an_upsert_can_neither_set_nor_clear_the_quota_columns() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    /// `quota`, `quota_at` and the one probe column, straight out of SQL.
    async fn columns(
        pool: &PgPool,
        box_id: htui_core::model::BoxId,
    ) -> (
        Option<serde_json::Value>,
        Option<DateTime<Utc>>,
        Option<String>,
    ) {
        let row = sqlx::query(
            "SELECT quota, quota_at, version FROM agent_box WHERE agent_id = $1 AND box_id = $2",
        )
        .bind(ids::AGENT_CLAUDE.as_uuid())
        .bind(box_id.as_uuid())
        .fetch_one(pool)
        .await
        .expect("read the agent_box row");
        (row.get("quota"), row.get("quota_at"), row.get("version"))
    }

    let stamp: DateTime<Utc> = "2026-09-10T06:00:00.000123Z"
        .parse()
        .expect("a microsecond-precision literal");
    let probed = AgentBox {
        agent_id: ids::AGENT_CLAUDE,
        box_id: db.store.this_box(),
        enabled: true,
        version: Some("1.2.3".to_owned()),
        path: Some("/usr/bin/claude".to_owned()),
        probed_at: Some(stamp),
        // The insert path.
        quota: Some(serde_json::json!({ "invented": "by the probe" })),
        quota_at: Some(stamp),
        updated_at: stamp,
        probe: Some(serde_json::json!({ "status": "ready", "source": "probe" })),
    };
    db.store
        .upsert_agent_box(&probed)
        .await
        .expect("an `AgentBox` carrying a quota is accepted, not refused");
    let (quota, quota_at, _) = columns(&db.pool, db.store.this_box()).await;
    assert_eq!(
        (quota, quota_at),
        (None, None),
        "the two columns left the INSERT list, so a fresh row is NULL/NULL - the column default \
         and the honest value: a row nobody has latched has no observed allowance"
    );

    let latched = serde_json::json!({
        "source": "acp_meta_rate_limit",
        "spend": { "session_micros": 351, "currency": "USD" },
    });
    let latched_at: DateTime<Utc> = "2026-09-10T09:00:00.000456Z"
        .parse()
        .expect("a microsecond-precision literal");
    db.store
        .set_agent_box_quota(
            ids::AGENT_CLAUDE,
            db.store.this_box(),
            latched.clone(),
            latched_at,
        )
        .await
        .expect("the only writer writes");

    // The conflict path, first half: a re-probe handing back the row it read before the latch.
    db.store
        .upsert_agent_box(&AgentBox {
            version: Some("1.3.0".to_owned()),
            quota: Some(serde_json::json!({ "stale": "read before the latch" })),
            quota_at: None,
            ..probed.clone()
        })
        .await
        .expect("the update lands");
    let (quota, quota_at, version) = columns(&db.pool, db.store.this_box()).await;
    assert_eq!(
        quota.as_ref(),
        Some(&latched),
        "`quota = EXCLUDED.quota` is gone from the SET list: the latch stands"
    );
    assert_eq!(quota_at, Some(latched_at), "and `quota_at` with it");
    assert_eq!(
        version.as_deref(),
        Some("1.3.0"),
        "the columns still in the SET list took the second write's value, so this is a row the \
         upsert really did update"
    );

    // Second half: a `None` cannot clear them either, the way a `None` probe clears its own.
    db.store
        .upsert_agent_box(&AgentBox {
            quota: None,
            quota_at: None,
            ..probed
        })
        .await
        .expect("the second update lands");
    let (quota, quota_at, _) = columns(&db.pool, db.store.this_box()).await;
    assert_eq!(
        quota.as_ref(),
        Some(&latched),
        "clearing a latch is `set_agent_box_quota`'s too; an upsert has no way to do it"
    );
    assert_eq!(quota_at, Some(latched_at));

    db.drop_db().await;
}
