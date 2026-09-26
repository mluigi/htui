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

use chrono::{DateTime, SubsecRound as _, TimeDelta, Utc};
use futures::future::join_all;
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, AgentId, Billing, BoxId, Claim, CommandRunId, CommandRunStatus, GraphSnapshot,
    Isolation, ItemFilter, ItemId, ItemKindId, ItemKindPatch, ItemPatch, NewCommandRun, NewItem,
    NewProject, NewRepo, NewRequirement, NewRun, NewWorkspace, Priority, ProjectId, RepoId,
    RequirementId, RunId, RunMode, RunStatus, RunStepTree, SnapshotGraph, SnapshotSettings, Status,
    StepId, TIMESTAMPTZ_DIGITS, Transport, WorkspaceBoxPath, WorkspaceId, WorkspacePatch,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::prompt::{DEFAULT_TEMPLATES, body_of};
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore as _, SettingRung, UpdateOutcome,
    WriteStore as _,
};
use htui_store::PgStore;
use sqlx::Row as _;
use sqlx::postgres::PgPool;
use sqlx::{AssertSqlSafe, Connection as _, PgConnection};

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

/// A fresh project authored by the fixture user, the one `app_user` the demo database keeps.
///
/// `demo_db` deletes the seeded user (`testkit.rs:170-193`), so `ids::USER` is the only value
/// `project.created_by` can take here without a `23503`.
fn fresh_project(slug: &str) -> NewProject {
    NewProject {
        id: ProjectId::new(),
        slug: slug.to_owned(),
        name: slug.to_uppercase(),
        description: String::new(),
        created_by: ids::USER,
    }
}

/// How many rows of `table` carry `project_id = project`.
///
/// Runtime-checked rather than `query!`, like the other reads this file adds: a new `query!`
/// string would need a `cargo sqlx prepare` pass, and `.sqlx/` belongs to the crate, not to a
/// test. `table` is one of four literals from the bodies below and never comes from input.
async fn rows_of(pool: &PgPool, table: &str, project: ProjectId) -> i64 {
    let sql = format!("SELECT count(*) FROM \"{table}\" WHERE project_id = $1");
    sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
        .bind(project.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap_or_else(|error| panic!("count {table}: {error}"))
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

/// §11.2's first clause for `requirement_key_counter` (MOD-38 plan D8, blueprint F9): two
/// independent pools minting in one area produce consecutive numbers.
///
/// The fixture has minted `R-ENT-1` and `R-ENT-2`, so the counter row exists and every mint here
/// takes the `DO UPDATE` branch of `mint_requirement`'s CTE; its row lock is the only thing
/// serialising them, exactly as it is for [`concurrent_mints_produce_consecutive_numbers`].
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_requirement_mints_produce_consecutive_numbers() {
    let Some(db) = common::demo_db().await else {
        return;
    };

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
        let new = NewRequirement {
            id: RequirementId::new(),
            body: format!("race {n}"),
            rationale: String::new(),
            priority: Priority::Must,
            created_by: ids::USER,
            box_id: Some(ids::BOX),
        };
        async move { store.mint_requirement(ids::AREA_ENT, new).await }
    }))
    .await;

    let mut numbers: Vec<i32> = mints
        .into_iter()
        .map(|minted| minted.expect("every mint lands").number)
        .collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        (3..103).collect::<Vec<i32>>(),
        "a hundred concurrent mints after the fixture's two are exactly 3..=102: no duplicate, \
         no gap"
    );
    // Runtime-checked, as `rows_of` is: a test's `query!` would need its own `.sqlx` entry.
    let last = sqlx::query_scalar::<_, i32>(
        "SELECT last_value FROM requirement_key_counter WHERE area_id = $1",
    )
    .bind(ids::AREA_ENT.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("read requirement_key_counter");
    assert_eq!(last, 102, "the counter ends at the highest number minted");

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
///
/// Plan D4's fourth call site (blueprint §3.9(a)): `open -> done` is **not** in the ANA-2 §4.3
/// table, so since MOD-4 T2 it is [`StoreError::Constraint`] rather than a landed move. The legs
/// that follow need the row actually sitting at `done` with `closed_at` set, so the item is driven
/// there along the sanctioned path `open -> queued -> in_progress -> done` — the same shape
/// `conformance.rs::status_cas_keeps_version` now uses. The two legs after it stay exactly as they
/// were: `done -> open` is a sanctioned reopen and `done -> closed` from a row that is at `open`
/// is a *stale* `from` on a *legal* pair, which is the `Ok(false)` this case exists to tell apart
/// from the new `Constraint`.
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

    let illegal = db
        .store
        .transition(before.id, Status::Open, Status::Done)
        .await;
    assert!(
        matches!(illegal, Err(htui_core::store::StoreError::Constraint(_))),
        "open -> done is outside §4.3 and is refused before the UPDATE, got {illegal:?}"
    );
    for (from, to) in [
        (Status::Open, Status::Queued),
        (Status::Queued, Status::InProgress),
        (Status::InProgress, Status::Done),
    ] {
        assert!(
            db.store
                .transition(before.id, from, to)
                .await
                .expect("transition must not fail"),
            "{from} -> {to} matches"
        );
    }
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
            .transition(before.id, Status::Done, Status::Open)
            .await
            .expect("transition must not fail"),
        "a stale `from` is refused rather than applied"
    );
    // Plan D14's precedence, and the reason this leg keeps the pair the first one now refuses: the
    // id names nothing, so the answer is `NotFound` **even though** `open -> done` is also illegal.
    let missing = db
        .store
        .transition(ItemId::new(), Status::Open, Status::Done)
        .await;
    assert!(
        matches!(missing, Err(htui_core::store::StoreError::NotFound { .. })),
        "an unknown item is NotFound, not `false` and not Constraint, got {missing:?}"
    );

    db.drop_db().await;
}

/// ANA-2 §4.7's critical section, and the one thing about it no `MemStore` case can show: two
/// concurrent `claim_run`s against a box with a single free slot, and exactly one `true`.
///
/// The rules of admission are decided by `conformance.rs::claim_run_admits_one_and_refuses_the_second`
/// and `mem.rs::claim_run_refuses_an_overlapping_scope_and_a_full_box`. What is decided *here* is
/// that the decision is **serialised**: the slot count is a read followed by a write, and under
/// `READ COMMITTED` without the `SELECT ... FOR UPDATE` on the `box` row (§4.7,
/// `docs/ANA-2.md:1094`) both claimers count zero `running` runs — neither has committed one yet —
/// and both take the last slot. The lock is what makes the second claimer block until the first
/// commits and then count the run the first started.
///
/// Two independent pools, as §11.3's race cases use: one pool would only serialise the claims if
/// it happened to be size 1, which is not a property this case may lean on.
#[tokio::test(flavor = "multi_thread")]
async fn admission_is_serialised_by_the_box_row_lock() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    // One slot, so "both won" and "one won" are answers that differ. The fixture box seeds two.
    sqlx::query!(
        r#"UPDATE box SET settings = '{"max_concurrent_items": 1}'::jsonb WHERE id = $1"#,
        ids::BOX.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect("narrow the box to one slot");

    // Empty `repo_scope` on both, so the only thing that can refuse the second claim is the slot
    // count: a scope overlap would make the case pass for the wrong reason (hazard H-10).
    let mut queued = Vec::new();
    for item in [ids::HTUI_ANA_2, ids::HTUI_CLEAN_1] {
        queued.push(
            db.store
                .create_run(race_run(item))
                .await
                .expect("queue a graph run")
                .id,
        );
    }
    let [first, second] = queued[..] else {
        panic!("two runs were queued")
    };

    let other = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let at = Utc::now();
    let until = at + TimeDelta::minutes(5);
    let (one, two) = tokio::join!(
        db.store
            .claim_run(first, ids::BOX, uuid::Uuid::now_v7(), at, until),
        other.claim_run(second, ids::BOX, uuid::Uuid::now_v7(), at, until),
    );
    let one = one.expect("the first claim must not fail");
    let two = two.expect("the second claim must not fail");

    assert_eq!(
        usize::from(one.is_admitted()) + usize::from(two.is_admitted()),
        1,
        "exactly one claim may take the last slot, got ({one}, {two})"
    );
    let refusal = if one.is_admitted() { &two } else { &one };
    assert_eq!(
        refusal,
        &Claim::SlotFull {
            running: 1,
            limit: 1
        },
        "the loser counted the winner's run against the one slot"
    );
    assert_eq!(
        count_running_on_box(&db.pool, ids::BOX).await,
        1,
        "the box that was allowed one concurrent run holds one"
    );

    let loser = if one.is_admitted() { second } else { first };
    let refused = db
        .store
        .run(loser)
        .await
        .expect("read must not fail")
        .expect("the refused run still exists");
    assert_eq!(
        refused.status,
        RunStatus::Queued,
        "the refused claim left its run queued"
    );
    assert_eq!(
        (
            refused.executing_box_id,
            refused.started_at,
            refused.lease_expires_at
        ),
        (None, None, None),
        "and wrote neither the box, the start nor the lease"
    );

    db.drop_db().await;
}

/// ANA-2 §4.9's sweep, raced: two processes sweeping one box at once adopt every expired run
/// exactly once between them.
///
/// `adopt_runs` locks its candidates in `(queued_at, id)` order with `FOR UPDATE SKIP LOCKED`
/// (blueprint A-5), so neither sweep waits on the other's row locks, and a row the other already
/// took is re-checked against its new, live lease rather than adopted a second time. Two pools, as
/// in [`admission_is_serialised_by_the_box_row_lock`].
#[tokio::test(flavor = "multi_thread")]
async fn two_sweeps_adopt_each_expired_run_once() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    let at = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
    let crashed = uuid::Uuid::now_v7();
    let mut expired = Vec::new();
    for item in [ids::HTUI_ANA_2, ids::HTUI_CLEAN_1] {
        let run = db
            .store
            .create_run(race_run(item))
            .await
            .expect("queue a graph run")
            .id;
        assert_eq!(
            db.store
                .claim_run(run, ids::BOX, crashed, at, at - TimeDelta::minutes(1))
                .await
                .expect("the claim must not fail"),
            Claim::Admitted,
            "both runs fit the fixture box's two slots, under a lease already expired"
        );
        expired.push(run);
    }

    let other = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let until = at + TimeDelta::minutes(5);
    let (a, b) = tokio::join!(
        db.store
            .adopt_runs(ids::BOX, uuid::Uuid::now_v7(), at, until),
        other.adopt_runs(ids::BOX, uuid::Uuid::now_v7(), at, until),
    );
    let a: Vec<RunId> = a
        .expect("the first sweep must not fail")
        .iter()
        .map(|row| row.id)
        .collect();
    let b: Vec<RunId> = b
        .expect("the second sweep must not fail")
        .iter()
        .map(|row| row.id)
        .collect();

    assert_eq!(
        a.len() + b.len(),
        2,
        "two expired runs are adopted twice in all, got {a:?} and {b:?}"
    );
    assert!(
        a.iter().all(|run| !b.contains(run)),
        "no run is adopted by both sweeps, got {a:?} and {b:?}"
    );
    let mut union: Vec<RunId> = a.into_iter().chain(b).collect();
    union.sort_unstable();
    expired.sort_unstable();
    assert_eq!(
        union, expired,
        "and between them every expired run is adopted"
    );

    db.drop_db().await;
}

/// Plan D87's take, raced (blueprint F-F): two processes answering one parked run whose lease was
/// released both try to take it, and exactly one does.
///
/// `take_lease` is one compare-and-set `UPDATE`. The loser blocks on the winner's row lock, and
/// READ COMMITTED re-checks its `WHERE` against the committed row, which by then carries the
/// winner's live lease.
#[tokio::test(flavor = "multi_thread")]
async fn two_takes_of_one_released_lease_admit_one() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    let at = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);
    let first_owner = uuid::Uuid::now_v7();
    let run = db
        .store
        .create_run(race_run(ids::HTUI_ANA_2))
        .await
        .expect("queue a graph run")
        .id;
    assert_eq!(
        db.store
            .claim_run(run, ids::BOX, first_owner, at, at + TimeDelta::minutes(5))
            .await
            .expect("the claim must not fail"),
        Claim::Admitted,
        "the run is admitted"
    );
    assert!(
        db.store
            .transition_run(run, RunStatus::Running, RunStatus::AwaitingApproval, at)
            .await
            .expect("the park must not fail"),
        "the run parks at a gate"
    );
    assert!(
        db.store
            .release_lease(run, first_owner, at)
            .await
            .expect("the release must not fail"),
        "the walk that parked it releases its lease (owner cleared, lease_expires_at = now: plan D139)"
    );

    let other = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let (one_until, two_until) = (at + TimeDelta::minutes(10), at + TimeDelta::minutes(20));
    let (one, two) = tokio::join!(
        db.store
            .take_lease(run, ids::BOX, uuid::Uuid::now_v7(), at, one_until),
        other.take_lease(run, ids::BOX, uuid::Uuid::now_v7(), at, two_until),
    );
    let one = one.expect("the first take must not fail");
    let two = two.expect("the second take must not fail");

    assert_eq!(
        usize::from(one) + usize::from(two),
        1,
        "exactly one take may win a released lease, got ({one}, {two})"
    );
    let winner = if one { one_until } else { two_until };
    assert_eq!(
        db.store
            .run(run)
            .await
            .expect("read must not fail")
            .expect("the run exists")
            .lease_expires_at,
        Some(winner),
        "the stored expiry is the winner's"
    );

    db.drop_db().await;
}

/// The `running` runs of one box, counted straight from the table so the assertion does not rest
/// on the reader under test.
async fn count_running_on_box(pool: &PgPool, box_id: BoxId) -> i64 {
    sqlx::query_scalar!(
        "SELECT COUNT(*) FROM run WHERE executing_box_id = $1 AND status = 'running'",
        box_id.as_uuid(),
    )
    .fetch_one(pool)
    .await
    .expect("count the running runs")
    .unwrap_or(0)
}

/// A graph-run request for `item` with an empty `repo_scope`, on the fixture's box and user.
///
/// The snapshot is the shortest value `ck_run_graph_snapshot` accepts and §5.1 decodes: this case
/// never reads it back, so there is nothing to gain from the fixture's fuller one.
fn race_run(item: ItemId) -> NewRun {
    NewRun {
        id: RunId::new(),
        project_id: ids::PROJECT_HTUI,
        item_id: item,
        mode: RunMode::Manual,
        target_box_id: ids::BOX,
        started_by: ids::USER,
        graph_snapshot: GraphSnapshot {
            v: GraphSnapshot::V,
            graph: SnapshotGraph {
                id: ids::GRAPH_HTUI_FEAT,
                name: "feature".to_owned(),
                is_override: false,
            },
            topology: "sha256:pg_criteria".to_owned(),
            mode: RunMode::Manual,
            phases: Vec::new(),
            settings: SnapshotSettings {
                default_isolation: Isolation::Worktree,
                per_token_cap_run: None,
                per_token_cap_batch: None,
                max_fan_out: 4,
                max_agents_per_run: 6,
            },
            scope: None,
        },
        repo_scope: Vec::new(),
        queued_at: Utc::now(),
    }
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

/// `set_step_prompt` writes `prompt_digest` and `trim_record` and **nothing else** on the row, the
/// `updated_at` trigger aside (`docs/ANA-5.md` §4.4).
///
/// `store::conformance::set_step_prompt_writes_digest_and_trim` observes the write through
/// `ReadStore::runs`, which §6.1 gives two derived figures and neither column, so it cannot say
/// what was *not* written; `store::mem`'s `set_step_prompt_writes_both_columns` is the memory half
/// of that claim and this is the Postgres half its doc comment names (T68, F-52 review, M2).
///
/// The whole row is compared as `jsonb` rather than a chosen column list, so a column added to
/// `run_step` later is covered without anyone remembering to extend this: the assertion is the
/// **set of keys that changed**, and a new column that `set_step_prompt` learns to write shows up
/// in it. The pre-write state is deliberately non-empty — a usage write and a first prompt write —
/// because a column that is `NULL` on both sides proves nothing about whether it was touched.
#[tokio::test(flavor = "multi_thread")]
async fn set_step_prompt_writes_only_the_digest_and_the_record() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let step = ids::STEP_IMPL;

    /// The `run_step` row as a JSON object, so two of them can be diffed key by key.
    async fn row(pool: &PgPool, step: StepId) -> serde_json::Map<String, serde_json::Value> {
        let row = sqlx::query("SELECT to_jsonb(s) AS row FROM run_step s WHERE s.id = $1")
            .bind(step.as_uuid())
            .fetch_one(pool)
            .await
            .expect("the fixture step is there");
        match row.get::<serde_json::Value, _>("row") {
            serde_json::Value::Object(map) => map,
            other => panic!("to_jsonb of a row is an object, got {other}"),
        }
    }

    // Something in every column the write must not disturb, so "unchanged" is a real claim.
    db.store
        .set_step_usage(step, serde_json::json!({ "input_tokens": 7 }), None)
        .await
        .expect("the usage write lands");
    db.store
        .set_step_prompt(step, "9f8e", &serde_json::json!({ "v": 1 }))
        .await
        .expect("the first prompt write lands");

    let before = row(&db.pool, step).await;
    let record = serde_json::json!({ "estimated_after": 34_000, "sections": [], "v": 1 });
    db.store
        .set_step_prompt(step, "0a1b", &record)
        .await
        .expect("the second prompt write lands");
    let after = row(&db.pool, step).await;

    let mut changed: Vec<&str> = before
        .keys()
        .chain(after.keys())
        .map(String::as_str)
        .filter(|key| before.get(*key) != after.get(*key))
        .collect();
    changed.sort_unstable();
    changed.dedup();
    assert_eq!(
        changed,
        vec!["prompt_digest", "trim_record", "updated_at"],
        "the two columns §4.4 names, plus the migration's `BEFORE UPDATE` trigger's own"
    );

    assert_eq!(
        after.get("prompt_digest"),
        Some(&serde_json::json!("0a1b")),
        "the digest is overwritten unconditionally, unlike `set_step_usage`'s optional one"
    );
    assert_eq!(
        after.get("trim_record"),
        Some(&record),
        "the record is stored whole, not a projection of it"
    );
    assert_eq!(
        after.get("usage"),
        Some(&serde_json::json!({ "input_tokens": 7 })),
        "the pre-flight audit does not touch the post-flight figure"
    );

    let unknown = db
        .store
        .set_step_prompt(StepId::new(), "9f8e", &serde_json::json!({}))
        .await;
    assert!(
        matches!(
            unknown,
            Err(htui_core::store::StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "a step no row has is NotFound, not a silent no-op, got {unknown:?}"
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
        4,
        "the fixture's agents plus this one, and the rename took no other row with it"
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
    // MOD-4 milestone 1, T1 audit A-5: `probed_tags` and `declared_tags` are adjacent
    // `Vec<String>` fields of `BoxInfo` and `query_as!` binds **positionally**, so transposing
    // them in the select list type-checks and binds silently. The fixture seeds two deliberately
    // different sets, and this names which is which.
    assert_eq!(
        (info.probed_tags.as_slice(), info.declared_tags.as_slice()),
        (
            ["rust".to_owned(), "msvc".to_owned(), "cmake".to_owned()].as_slice(),
            ["gpu".to_owned()].as_slice()
        ),
        "the probe wrote the toolchain tags and the human declared `gpu`; the two are not swapped"
    );
    assert_eq!(
        info.settings,
        serde_json::json!({ "max_concurrent_items": 2 }),
        "box.settings comes back whole (ANA-2 §4.7)"
    );

    // MOD-2 plan D3: the registry read is inherent too, because `agent` / `agent_box` are not
    // mirrored. The demo loader inserts the seed's `agent` rows and no `agent_box` row at all, so
    // every summary is unprobed on this box.
    let agents = db.store.agents().await.expect("agents must not fail");
    assert_eq!(
        agents
            .iter()
            .map(|row| row.agent.name.as_str())
            .collect::<Vec<_>>(),
        vec!["agy", "claude", "claude-cli"],
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
        box_id: BoxId,
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

/// MOD-2 milestone 9's five inherent prompt reads (blueprint B.13), against the fixture.
///
/// `prompt_template`, `skill`, `skill_version`, `skill_binding` and `box_tool` are the prompt
/// inputs `docs/ANA-9.md` §4.4 does **not** mirror, so these five never became `ReadStore`
/// methods and `Backend`'s offline arm refuses them (plan D109). What they cannot get from the
/// conformance suite they get here: the fixture's own rows, read through Postgres, compared
/// against the `MemStore` that the suite pins — which is only possible because `load_demo` now
/// inserts those four tables too (blueprint E-7).
#[tokio::test(flavor = "multi_thread")]
async fn inherent_prompt_reads_answer_the_fixture() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mem = htui_core::store::MemStore::demo();

    // Ten names, one version each, ordered by `(name, version)` in byte order (ANA-5 §4.6).
    let templates = db
        .store
        .prompt_templates(ids::PROJECT_HTUI)
        .await
        .expect("prompt_templates must not fail");
    assert_eq!(
        templates.len(),
        10,
        "the fixture seeds one version of each of the ten default templates per project"
    );
    assert_eq!(
        templates,
        mem.prompt_templates(ids::PROJECT_HTUI)
            .await
            .expect("MemStore::prompt_templates"),
        "Postgres and the reference store answer the same rows in the same order"
    );
    assert!(
        templates
            .iter()
            .all(|row| row.project_id == ids::PROJECT_HTUI),
        "a project's templates and no other project's"
    );

    // `R-SKL-2`: the phase binding overrides the project binding of the same skill, **once**, at
    // the phase binding's pinned version and position.
    let project_level = db
        .store
        .bound_skills(ids::PROJECT_HTUI, None)
        .await
        .expect("bound_skills must not fail");
    assert_eq!(
        project_level
            .iter()
            .map(|skill| (skill.name.as_str(), skill.version, skill.position))
            .collect::<Vec<_>>(),
        vec![("tests", Some(1), 0), ("rust-style", Some(2), 1)],
        "with no phase the project bindings stand, ordered by (position, name bytes), and an \
         unpinned binding follows the latest version"
    );

    let phase_level = db
        .store
        .bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
        .await
        .expect("bound_skills must not fail");
    assert_eq!(
        phase_level
            .iter()
            .map(|skill| (skill.name.as_str(), skill.version, skill.position))
            .collect::<Vec<_>>(),
        vec![("tests", Some(1), 0), ("rust-style", Some(1), 2)],
        "the `implement` phase pins rust-style at v1 and the collapse renders it once, at the \
         phase binding's position"
    );
    assert_eq!(
        phase_level
            .iter()
            .filter(|skill| skill.name == "rust-style")
            .count(),
        1,
        "a skill bound at both levels is rendered exactly once (R-SKL-2)"
    );
    assert_eq!(
        phase_level,
        mem.bound_skills(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT))
            .await
            .expect("MemStore::bound_skills"),
        "the collapse is `BoundSkill::collapse` on both backends, bodies included"
    );

    // The box section: the probe's tools in name byte order, versions kept, `path` dropped.
    let profile = db
        .store
        .box_profile(db.store.this_box())
        .await
        .expect("box_profile must not fail")
        .expect("the fixture registers this box");
    assert_eq!(
        profile.tools,
        vec![
            ("cargo".to_owned(), "1.98.0".to_owned()),
            ("cmake".to_owned(), String::new()),
            ("git".to_owned(), "2.51.0".to_owned()),
            ("rustc".to_owned(), "1.98.0".to_owned()),
        ],
        "the fixture's four `box_tool` rows, name-byte-sorted, the version-less one kept"
    );
    assert_eq!(
        profile.more_tools, 0,
        "four tools is under the cap, so nothing is reported as dropped"
    );
    assert_eq!(
        profile,
        mem.box_profile(db.store.this_box())
            .await
            .expect("MemStore::box_profile")
            .expect("the fixture registers this box"),
        "the projection is `BoxProfile::project` on both backends"
    );
    assert!(
        db.store
            .box_profile(BoxId::new())
            .await
            .expect("an unknown box is not an error")
            .is_none(),
        "an id nothing has is None, not a default profile"
    );

    // ANA-5 §9's ten `app_setting` defaults, as migration `0002` seeded them.
    let settings = db
        .store
        .app_settings()
        .await
        .expect("app_settings must not fail");
    for (key, expected) in [
        ("token_budget", 120_000_i64),
        ("prompt_upstream_hops", 2),
        ("max_skill_tokens", 20_000),
        ("excerpt_max_files", 12),
        ("excerpt_file_line_cap", 400),
        ("excerpt_head_lines", 200),
        ("excerpt_max_file_bytes", 524_288),
        ("excerpt_max_scan_files", 20_000),
        ("excerpt_provider_deadline_ms", 1_500),
    ] {
        assert_eq!(
            settings.get(key).and_then(serde_json::Value::as_i64),
            Some(expected),
            "app_setting.{key} is the ANA-5 §9 default the migration seeded"
        );
    }
    assert_eq!(
        settings
            .get("prompt_reserve_fraction")
            .and_then(serde_json::Value::as_f64),
        Some(0.10),
        "the tenth key is the one fractional default"
    );

    // `item_kind`, the read `{{item_kind}}` needs and §6.1 returns nowhere (blueprint E-6).
    let item = db
        .store
        .item(ids::HTUI_FEAT_1)
        .await
        .expect("item must not fail")
        .expect("the fixture holds FEAT-1");
    let kind = db
        .store
        .item_kind(item.kind_id)
        .await
        .expect("item_kind must not fail")
        .expect("every item's kind_id references a row");
    assert_eq!(
        (kind.prefix.as_str(), kind.name.as_str()),
        ("FEAT", "feature"),
        "the kind behind FEAT-1 is the one the key prefix names"
    );
    assert_eq!(
        Some(kind),
        mem.item_kind(item.kind_id)
            .await
            .expect("MemStore::item_kind"),
        "the two backends answer the same row"
    );

    db.drop_db().await;
}

/// D3's token is the migration's `BEFORE UPDATE` trigger's, and nothing a caller sends can become
/// one.
///
/// `store::conformance`'s `workspace_round_trip_and_cas` can say the token *advanced*; only SQL can
/// say **who advanced it**. Three claims Postgres alone can carry: the stamp a write leaves lies
/// strictly between two server-side `clock_timestamp()` readings taken around it, so it is the
/// server's clock rather than the client's; two writes in a row leave two distinct, strictly
/// increasing stamps; and an `updated_at` the caller puts in the row it hands to
/// `upsert_workspace_box_path` is discarded rather than stored - which is what "no write path may
/// set `updated_at` by hand" (`0001_init.sql:567`) means from the outside.
///
/// The App settings rung is included because its `UPDATE app_setting SET value = $2` names one
/// column and still moves the token: `app_setting` is in the trigger loop (`0001_init.sql:577`).
#[tokio::test(flavor = "multi_thread")]
async fn cas_tokens_advance_by_the_trigger_alone() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    /// The server's wall clock, read on a pooled connection of its own.
    async fn server_now(pool: &PgPool) -> DateTime<Utc> {
        sqlx::query_scalar::<_, DateTime<Utc>>("SELECT clock_timestamp()")
            .fetch_one(pool)
            .await
            .expect("the server answers its own clock")
    }

    /// `CasOutcome::Applied`'s row, or a panic naming what came back instead.
    fn applied<T: std::fmt::Debug>(outcome: CasOutcome<T>) -> T {
        match outcome {
            CasOutcome::Applied(row) => row,
            CasOutcome::Stale(row) => panic!("expected Applied, got Stale({row:?})"),
        }
    }

    let created = db
        .store
        .create_workspace(NewWorkspace {
            id: WorkspaceId::new(),
            slug: "trigger".to_owned(),
            name: "Trigger".to_owned(),
            description: String::new(),
            created_by: ids::USER,
        })
        .await
        .expect("the create lands");

    let rename = |name: &str| WorkspacePatch {
        name: Some(name.to_owned()),
        ..WorkspacePatch::default()
    };

    let opened = server_now(&db.pool).await;
    let first = applied(
        db.store
            .update_workspace(created.id, created.updated_at, rename("First"))
            .await
            .expect("the first edit lands"),
    );
    let closed = server_now(&db.pool).await;

    assert!(
        first.updated_at > opened && first.updated_at < closed,
        "the token is the server's clock_timestamp(), taken while the statement ran: \
         {opened} < {} < {closed}",
        first.updated_at
    );
    assert_ne!(
        first.updated_at, created.updated_at,
        "the token the caller edited from is not the token it edits into"
    );

    let second = applied(
        db.store
            .update_workspace(first.id, first.updated_at, rename("Second"))
            .await
            .expect("the second edit lands"),
    );
    assert!(
        second.updated_at > first.updated_at,
        "two writes in a row leave two distinct tokens, so neither can be replayed"
    );

    // A caller that fills `updated_at` in the row it hands over is writing into a column the
    // trigger and the column default own; the year 2000 is there to be conspicuous if it landed.
    let caller_stamp =
        DateTime::<Utc>::from_timestamp(946_684_800, 0).expect("2000-01-01 is a date");
    let before_path = server_now(&db.pool).await;
    db.store
        .upsert_workspace_box_path(&WorkspaceBoxPath {
            workspace_id: second.id,
            box_id: ids::BOX,
            root_path: "/srv/trigger".to_owned(),
            updated_at: caller_stamp,
        })
        .await
        .expect("the path insert lands");
    let stored = db
        .store
        .workspace_box_paths(second.id)
        .await
        .expect("the read-back")
        .pop()
        .expect("the row that was just written");
    assert!(
        stored.updated_at > before_path,
        "the caller's {caller_stamp} was discarded for the server's own stamp, got {}",
        stored.updated_at
    );

    let token = db
        .store
        .setting(SettingRung::App, SettingKey::TokenBudget)
        .await
        .expect("the App rung read")
        .expect("migration 0002 seeds token_budget");
    let opened = server_now(&db.pool).await;
    let written = applied(
        db.store
            .set_setting(
                SettingRung::App,
                SettingKey::TokenBudget,
                serde_json::json!(90_000),
                Some(token.updated_at),
            )
            .await
            .expect("the App rung write lands"),
    );
    let closed = server_now(&db.pool).await;
    assert!(
        written.updated_at > opened && written.updated_at < closed,
        "`UPDATE app_setting SET value = $2` names one column and the trigger still moves the \
         token: {opened} < {} < {closed}",
        written.updated_at
    );

    db.drop_db().await;
}

/// The `Project` rung merges **one key** and touches **one other column**: the whole `project` row
/// is diffed as `jsonb` before and after, and only `settings` and `updated_at` may differ (D7, D8).
///
/// `store::conformance`'s `settings_project_rung_merges_keys` asserts per-key equality of the
/// settings document, because JSONB's own normalisation makes byte identity unassertable across
/// `MemStore` and `PgStore`; `store::mem`'s
/// `set_setting_project_rung_leaves_unknown_keys_byte_identical` is the memory half. This is the
/// Postgres half, and it can say two things neither of those can: that no **other column** of
/// `project` moved - `slug`, `name`, `secret_provider`, `created_by` and the rest - and that a
/// clear puts the document back to the same JSONB it found, which is byte identity on this side of
/// the seam.
///
/// The whole row rather than a chosen column list, for the reason
/// `set_step_prompt_writes_only_the_digest_and_the_record` gives: a column a later migration adds
/// is covered without anyone remembering to extend this.
#[tokio::test(flavor = "multi_thread")]
async fn set_setting_project_rung_changes_only_settings_and_updated_at() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let id = ids::PROJECT_HTUI;
    let rung = SettingRung::Project(id);

    /// The `project` row as a JSON object, so two of them can be diffed key by key.
    async fn row(pool: &PgPool, id: ProjectId) -> serde_json::Map<String, serde_json::Value> {
        let row = sqlx::query("SELECT to_jsonb(p) AS row FROM project p WHERE p.id = $1")
            .bind(id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("the fixture project is there");
        match row.get::<serde_json::Value, _>("row") {
            serde_json::Value::Object(map) => map,
            other => panic!("to_jsonb of a row is an object, got {other}"),
        }
    }

    /// The `settings` column of such a row, which every fixture project seeds as an object.
    fn settings(row: &serde_json::Map<String, serde_json::Value>) -> &serde_json::Value {
        row.get("settings").expect("project.settings is a column")
    }

    let project = db
        .store
        .project(id)
        .await
        .expect("the project read")
        .expect("the fixture project");
    let before = row(&db.pool, id).await;
    assert!(
        settings(&before)
            .as_object()
            .is_some_and(|map| !map.is_empty()),
        "the fixture seeds keys this write must not disturb"
    );

    let written = match db
        .store
        .set_setting(
            rung,
            SettingKey::UpstreamHops,
            serde_json::json!(2),
            Some(project.updated_at),
        )
        .await
        .expect("the merge lands")
    {
        CasOutcome::Applied(row) => row,
        CasOutcome::Stale(row) => panic!("expected Applied, got Stale({row:?})"),
    };
    let after = row(&db.pool, id).await;

    let mut changed: Vec<&str> = before
        .keys()
        .chain(after.keys())
        .map(String::as_str)
        .filter(|key| before.get(*key) != after.get(*key))
        .collect();
    changed.sort_unstable();
    changed.dedup();
    assert_eq!(
        changed,
        vec!["settings", "updated_at"],
        "the one column D7 writes, plus the migration's `BEFORE UPDATE` trigger's own"
    );

    let before_keys = settings(&before).as_object().expect("an object").clone();
    let after_keys = settings(&after).as_object().expect("an object").clone();
    assert_eq!(
        after_keys.get("upstream_hops"),
        Some(&serde_json::json!(2)),
        "the project rung writes the key `resolve_hops` reads, not the App key (flag A)"
    );
    assert_eq!(
        after_keys.len(),
        before_keys.len() + 1,
        "one key was added and none was replaced"
    );
    for (key, value) in &before_keys {
        assert_eq!(
            after_keys.get(key),
            Some(value),
            "`{key}` survived the merge unchanged"
        );
    }

    db.store
        .clear_setting(rung, SettingKey::UpstreamHops, written.updated_at)
        .await
        .expect("the clear lands");
    let cleared = row(&db.pool, id).await;
    assert_eq!(
        settings(&cleared),
        settings(&before),
        "`settings - key` puts the document back to the JSONB it found, byte for byte"
    );

    db.drop_db().await;
}

/// Review M1: `delete_project` counts and deletes in one **snapshot**, not merely in one
/// transaction.
///
/// At `READ COMMITTED` every statement of a transaction takes its own snapshot and the counting
/// `SELECT` takes no row lock, so a child row committed between `project_reach`'s `count(*)` and
/// the `DELETE FROM project` is cascaded away without ever having been counted. That is PRD D13's
/// success metric - "the counts shown match what the cascade removes" - failing on the one
/// operation that destroys history, which is why it is worth a deterministic case rather than a
/// raced one.
///
/// Deterministic, and it does not need `delete_project` to pause anywhere: a second connection
/// inserts a `session_event` under one of the project's steps and **holds the transaction open**.
/// The count runs first and cannot see an uncommitted row, and the insert has taken a
/// `FOR KEY SHARE` lock on its `run_step` row, so the cascade blocks there until the commit -
/// which lands strictly between the count and the delete, every run.
///
/// Either answer is honest and the case accepts both: a report that names the extra row, or a
/// refusal that leaves the project standing. What it refuses is a report that is one short of what
/// the cascade took.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_committed_mid_delete_is_never_missing_from_the_count() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = ids::PROJECT_HTUI;

    /// How many `session_event` rows a project delete reaches, by `project_reach`'s own predicate.
    async fn events_of(pool: &PgPool, project: ProjectId) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM session_event e \
               JOIN run_step s ON s.id = e.run_step_id \
               JOIN run r ON r.id = s.run_id \
              WHERE r.project_id = $1 \
                 OR r.item_id IN (SELECT id FROM item WHERE project_id = $1)",
        )
        .bind(project.as_uuid())
        .fetch_one(pool)
        .await
        .expect("count session_event")
    }

    let step: uuid::Uuid = sqlx::query_scalar(
        "SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
          WHERE r.project_id = $1 OR r.item_id IN (SELECT id FROM item WHERE project_id = $1) \
          ORDER BY s.id LIMIT 1",
    )
    .bind(project.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the fixture gives the project a run_step");

    let before = events_of(&db.pool, project).await;
    assert!(
        before > 0,
        "the fixture seeds the project events, so a short count has something to be short of"
    );

    let mut racer = PgConnection::connect(&db.url)
        .await
        .expect("a second connection");
    sqlx::raw_sql("BEGIN")
        .execute(&mut racer)
        .await
        .expect("BEGIN on the racing connection");
    sqlx::query(
        "INSERT INTO session_event (run_step_id, seq, kind, role, payload, at) \
         VALUES ($1, 2147483647, 'other', 'htui', '{}'::jsonb, now())",
    )
    .bind(step)
    .execute(&mut racer)
    .await
    .expect("the racing insert");

    let store = db.store.clone();
    let deleting = tokio::spawn(async move { store.delete_project(project).await });
    // The counting statement is behind us and the cascade is parked on the `run_step` row lock.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    sqlx::raw_sql("COMMIT")
        .execute(&mut racer)
        .await
        .expect("COMMIT on the racing connection");
    racer.close().await.expect("close the racing connection");

    match deleting.await.expect("the delete task") {
        Ok(reach) => {
            assert_eq!(
                events_of(&db.pool, project).await,
                0,
                "the cascade ran, so the project keeps no event"
            );
            assert_eq!(
                reach.session_events,
                u64::try_from(before + 1).expect("a count is not negative"),
                "the report names the row committed mid-delete: the counts shown are the counts \
                 the act took (PRD D13)"
            );
        }
        Err(err) => {
            assert!(
                db.store
                    .project(project)
                    .await
                    .expect("the project read")
                    .is_some(),
                "a refused delete took nothing, so the project stands: {err}"
            );
            assert_eq!(
                events_of(&db.pool, project).await,
                before + 1,
                "and so do its events, the racing one included"
            );
        }
    }

    db.drop_db().await;
}

/// Review L1: `command_run` cascades from `run_step` (`0001_init.sql:537-544`) and belongs in the
/// count.
///
/// Plan V11 pinned the cascade list and this table is not on it; nothing in the tree writes it yet,
/// so `store::conformance`'s case would report `0` and be right by accident for as long as that
/// holds. The row is therefore inserted here by hand - the only way to tell "counted and zero" from
/// "not counted at all" before MOD-16's queue exists.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_run_is_counted_and_taken_by_the_project_delete() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = ids::PROJECT_HTUI;

    let step: uuid::Uuid = sqlx::query_scalar(
        "SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
          WHERE r.project_id = $1 OR r.item_id IN (SELECT id FROM item WHERE project_id = $1) \
          ORDER BY s.id LIMIT 1",
    )
    .bind(project.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the fixture gives the project a run_step");

    sqlx::query(
        "INSERT INTO command_run (run_step_id, box_id, class, command, cwd) \
         VALUES ($1, $2, 'test', 'cargo test', '/srv/htui')",
    )
    .bind(step)
    .bind(ids::BOX.as_uuid())
    .execute(&db.pool)
    .await
    .expect("queue one command_run under the project");

    let reach = db
        .store
        .delete_reach(DeleteTarget::Project(project))
        .await
        .expect("the reach read")
        .expect("the fixture project has a reach");
    assert_eq!(
        reach.command_runs, 1,
        "the queued command is part of what the delete would take (review L1)"
    );

    let report = db
        .store
        .delete_project(project)
        .await
        .expect("the delete lands");
    assert_eq!(
        report.command_runs, 1,
        "and part of what it says it took (PRD D13)"
    );
    assert_eq!(
        common::count(&db.pool, "command_run").await,
        0,
        "the cascade from run_step took it, counted or not"
    );

    db.drop_db().await;
}

/// Review M3: the cascade is **measured**, not re-derived.
///
/// `store::conformance`'s `project_delete_takes_everything_and_says_so` compares `delete_reach`
/// with `delete_project` - two outputs of the same counting query - so a table missing from both
/// the counter and [`DeleteReach`](htui_core::store::DeleteReach) passes it unnoticed. That is how
/// `command_run` survived plan V11's own fact-check (review L1), and it is why this twin exists:
/// it never asks the counter what the cascade removed. It `count(*)`s every table of the chain
/// before and after and asserts the difference equals the report, field by field.
///
/// The fixture leaves five of those tables empty for this project - `repo`, `repo_box_path`,
/// `phase_agent`, `run_step_commit`, `command_run` - and `0 == 0` would pass for any of them, so
/// each is seeded with one row first. The rows go in through SQL rather than through the seam:
/// a case that measured `WriteStore` with `WriteStore` would be self-confirming in exactly the way
/// this one is written to stop being.
///
/// `workspace_box_path` is in the list and must **not** move: a project is not a workspace.
#[tokio::test(flavor = "multi_thread")]
async fn every_cascade_table_loses_exactly_what_the_report_names() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = ids::PROJECT_HTUI;

    let step: uuid::Uuid = sqlx::query_scalar(
        "SELECT s.id FROM run_step s JOIN run r ON r.id = s.run_id \
          WHERE r.project_id = $1 OR r.item_id IN (SELECT id FROM item WHERE project_id = $1) \
          ORDER BY s.id LIMIT 1",
    )
    .bind(project.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the fixture gives the project a run_step");
    let phase: uuid::Uuid = sqlx::query_scalar(
        "SELECT p.id FROM step_graph_phase p JOIN step_graph g ON g.id = p.graph_id \
          WHERE g.project_id = $1 ORDER BY p.id LIMIT 1",
    )
    .bind(project.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the fixture gives the project a phase");

    let repo: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO repo (project_id, name) VALUES ($1, 'measured') RETURNING id",
    )
    .bind(project.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("seed a repo");
    sqlx::query("INSERT INTO repo_box_path (repo_id, box_id, local_path) VALUES ($1, $2, '/src')")
        .bind(repo)
        .bind(ids::BOX.as_uuid())
        .execute(&db.pool)
        .await
        .expect("seed a repo_box_path");
    sqlx::query(
        "INSERT INTO phase_agent (phase_id, position, agent_id, model) VALUES ($1, 99, $2, 'm')",
    )
    .bind(phase)
    .bind(ids::AGENT_CLAUDE.as_uuid())
    .execute(&db.pool)
    .await
    .expect("seed a phase_agent");
    // The commit's repo is the **sibling** project's, and deliberately so: `run_step_commit.repo_id`
    // has no cascade (`0001_init.sql:503`), and Postgres runs that check while the `repo` row is
    // being cascaded away rather than at the end of the statement, so a project holding a commit
    // against one of its *own* repos cannot be deleted at all - a raw `23503` where PRD D13
    // promises a delete. Nothing in the tree writes `run_step_commit` yet, so the case is latent
    // and is reported rather than papered over here; this case is about the counting, and it takes
    // the one shape that isolates it.
    let sibling_repo: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO repo (project_id, name) VALUES ($1, 'sibling') RETURNING id",
    )
    .bind(ids::PROJECT_AGY.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("seed the sibling project's repo");
    sqlx::query(
        "INSERT INTO run_step_commit (run_step_id, repo_id, before_hash) VALUES ($1, $2, 'abc')",
    )
    .bind(step)
    .bind(sibling_repo)
    .execute(&db.pool)
    .await
    .expect("seed a run_step_commit");
    // `run_step_tree.repo_id` has no cascade either (`0003_orchestration.sql:94`), so it takes the
    // sibling project's repo for the reason spelled out above. Seeded rather than left at zero so
    // the `run_step_trees` entry below cannot pass on `0 == 0` (blueprint §3.9(b)).
    sqlx::query(
        "INSERT INTO run_step_tree (run_step_id, repo_id, mode, path, base_ref, dirty) \
         VALUES ($1, $2, 'worktree', '/trees/abc', 'main', true)",
    )
    .bind(step)
    .bind(sibling_repo)
    .execute(&db.pool)
    .await
    .expect("seed a run_step_tree");
    sqlx::query(
        "INSERT INTO command_run (run_step_id, box_id, class, command, cwd) \
         VALUES ($1, $2, 'test', 'cargo test', '/src')",
    )
    .bind(step)
    .bind(ids::BOX.as_uuid())
    .execute(&db.pool)
    .await
    .expect("seed a command_run");

    /// Every table of PRD D13's chain, paired with the [`DeleteReach`] field that claims it.
    ///
    /// Twenty-eight entries for twenty-eight fields: a field added without an entry leaves the
    /// struct literal below incomplete and the crate does not compile. MOD-38's six requirement
    /// tables close the list, in `DeleteReach` field order.
    ///
    /// `TABLES` and `claimed` are zipped **positionally**, so a new name goes where its field sits
    /// in `DeleteReach` and never simply at the end: `run_step_trees` is struct index 16, between
    /// `run_step_commits` and `command_runs`, and appending it to both arrays would compile while
    /// comparing every later field against the wrong table (T1 audit A-3).
    const TABLES: [&str; 28] = [
        "workspace_project",
        "workspace_box_path",
        "item",
        "item_key_counter",
        "item_kind",
        "step_graph",
        "step_graph_phase",
        "phase_agent",
        "prompt_template",
        "repo",
        "repo_box_path",
        "skill_binding",
        "run",
        "run_step",
        "session_event",
        "run_step_commit",
        "run_step_tree",
        "command_run",
        "item_note",
        "item_revision",
        "item_link",
        "document",
        "requirement_spec",
        "requirement_area",
        "requirement_key_counter",
        "requirement",
        "requirement_revision",
        "item_requirement",
    ];

    let mut before = Vec::with_capacity(TABLES.len());
    for table in TABLES {
        before.push(common::count(&db.pool, table).await);
    }

    let report = db
        .store
        .delete_project(project)
        .await
        .expect("the delete lands");

    // Destructured rather than read through `report.field`, so a field added to `DeleteReach`
    // without a line here is a compile error rather than a silent gap - which is the whole
    // complaint this case answers.
    let DeleteReach {
        workspace_links,
        workspace_box_paths,
        items,
        item_key_counters,
        item_kinds,
        step_graphs,
        phases,
        phase_agents,
        prompt_templates,
        repos,
        repo_box_paths,
        skill_bindings,
        runs,
        run_steps,
        session_events,
        run_step_commits,
        run_step_trees,
        command_runs,
        notes,
        revisions,
        links,
        documents,
        requirement_specs,
        requirement_areas,
        requirement_key_counters,
        requirements,
        requirement_revisions,
        item_requirements,
    } = report;
    let claimed: [u64; 28] = [
        workspace_links,
        workspace_box_paths,
        items,
        item_key_counters,
        item_kinds,
        step_graphs,
        phases,
        phase_agents,
        prompt_templates,
        repos,
        repo_box_paths,
        skill_bindings,
        runs,
        run_steps,
        session_events,
        run_step_commits,
        run_step_trees,
        command_runs,
        notes,
        revisions,
        links,
        documents,
        requirement_specs,
        requirement_areas,
        requirement_key_counters,
        requirements,
        requirement_revisions,
        item_requirements,
    ];

    for ((table, was), says) in TABLES.into_iter().zip(before).zip(claimed) {
        let now = common::count(&db.pool, table).await;
        let took = u64::try_from(was - now).expect("a cascade removes rows, it does not add them");
        assert_eq!(
            took, says,
            "`{table}` lost {took} rows and the report claims {says}: the counts shown are the \
             counts the act took, measured rather than re-counted (PRD D13, review M3)"
        );
    }
    assert_eq!(
        workspace_box_paths, 0,
        "a project is not a workspace, so no box path of one moves"
    );
    assert!(
        (
            phase_agents,
            run_step_commits,
            run_step_trees,
            command_runs,
            repo_box_paths
        ) == (1, 1, 1, 1, 1),
        "the six seeded tables are non-zero, so none of them passed on `0 == 0`"
    );

    db.drop_db().await;
}

/// Review L2: a mint that lands while `delete_item_kind` is deciding must still get D6's sentence.
///
/// The two statements the delete used to be - `count(*)` the holders, then `DELETE` - are two
/// autocommit round trips, so an item minted between them is invisible to the count and present by
/// the time the `item.kind_id` foreign key is checked. PRD D6 asks the refusal to "name what holds
/// it" and what came back instead was the constraint name and Postgres's own wording.
///
/// Deterministic by the same device `a_child_committed_mid_delete_is_never_missing_from_the_count`
/// uses: the racing insert takes a `FOR KEY SHARE` lock on the `item_kind` row, so the delete
/// parks on it until the commit.
#[tokio::test(flavor = "multi_thread")]
async fn a_mint_racing_a_kind_delete_still_names_what_holds_it() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let kind = race_kind(&db.pool).await;

    let mut racer = PgConnection::connect(&db.url)
        .await
        .expect("a second connection");
    sqlx::raw_sql("BEGIN")
        .execute(&mut racer)
        .await
        .expect("BEGIN on the racing connection");
    sqlx::query(
        "INSERT INTO item (project_id, kind_id, key_prefix, key_number, title, created_by) \
         VALUES ($1, $2, $3, 1, 'raced in', $4)",
    )
    .bind(ids::PROJECT_HTUI.as_uuid())
    .bind(kind.as_uuid())
    .bind(RACE_PREFIX)
    .bind(ids::USER.as_uuid())
    .execute(&mut racer)
    .await
    .expect("the racing mint");

    let store = db.store.clone();
    let deleting = tokio::spawn(async move { store.delete_item_kind(kind).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    sqlx::raw_sql("COMMIT")
        .execute(&mut racer)
        .await
        .expect("COMMIT on the racing connection");
    racer.close().await.expect("close the racing connection");

    match deleting.await.expect("the delete task") {
        Err(htui_core::store::StoreError::Constraint(text)) => assert!(
            text.contains("is held by 1 items"),
            "the refusal names what holds it (PRD D6), got `{text}`"
        ),
        other => panic!("a kind an item holds is Constraint, got {other:?}"),
    }
    let still_there: i64 = sqlx::query_scalar("SELECT count(*) FROM item_kind WHERE id = $1")
        .bind(kind.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("count the kind");
    assert_eq!(
        still_there, 1,
        "the refused delete removed nothing, so the item that holds it still has its kind"
    );

    db.drop_db().await;
}

/// MOD-15 D4's template rows, which no `WriteStore` reader returns: ten per project, named by
/// `DEFAULT_TEMPLATES`, body `body_of(name)`, version 1, `created_by` the creator's.
///
/// The Postgres twin of `store::mem`'s test of the same name. `project_create_seeds_the_catalogue`
/// counts the ten through `delete_reach`; what it cannot see is the *content*, and on this backend
/// it also cannot see that `created_at` and `updated_at` are the server's - both default in one
/// statement, so an equal pair is what says the seeder bound neither.
#[tokio::test(flavor = "multi_thread")]
async fn seeded_templates_carry_the_shipped_bodies() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = db
        .store
        .create_project(fresh_project("seeded"))
        .await
        .expect("the create lands");

    // `step_graph_phase` has no `project_id`; its fifteen are the conformance case's (e).
    assert_eq!(
        rows_of(&db.pool, "prompt_template", project.id).await,
        10,
        "ten template rows on the table itself"
    );
    assert_eq!(rows_of(&db.pool, "step_graph", project.id).await, 5);
    assert_eq!(rows_of(&db.pool, "item_kind", project.id).await, 5);

    let rows = db
        .store
        .prompt_templates(project.id)
        .await
        .expect("the inherent reader answers");
    let mut expected: Vec<&str> = DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect();
    expected.sort_unstable();
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        expected,
        "ten rows, one per default template, in the reader's name-byte order"
    );
    for row in &rows {
        assert_eq!(
            Some(row.body.as_str()),
            body_of(&row.name),
            "`{}` body",
            row.name
        );
        assert_eq!(row.version, 1, "`{}` is version 1", row.name);
        assert_eq!(row.created_by, ids::USER, "`{}` is the creator's", row.name);
        assert_eq!(row.project_id, project.id);
        assert_eq!(
            row.created_at, row.updated_at,
            "`{}` untouched since the insert, both columns the server's",
            row.name
        );
    }

    db.drop_db().await;
}

/// MOD-15 D5 on the table itself: `item_key_counter` has no row for a project until its first
/// mint, and then exactly one, for the prefix that minted.
///
/// The Postgres twin of `store::mem`'s test of the same name. `FEAT-1` rather than `FEAT-2` is the
/// user-visible half and the conformance case pins it; the row count is what says the seeder never
/// took §7.1's `ON CONFLICT` path on its own.
#[tokio::test(flavor = "multi_thread")]
async fn seed_never_writes_a_counter_row() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = db
        .store
        .create_project(fresh_project("lazy"))
        .await
        .expect("the create lands");

    assert_eq!(
        rows_of(&db.pool, "item_key_counter", project.id).await,
        0,
        "no counter row of any prefix after the create"
    );
    assert_eq!(counter(&db.pool, project.id, "FEAT").await, None);

    let feat = db
        .store
        .item_kinds(project.id)
        .await
        .expect("kinds read")
        .into_iter()
        .find(|kind| kind.prefix == "FEAT")
        .expect("the seeded FEAT kind");
    let minted = db
        .store
        .mint_item(NewItem {
            project_id: project.id,
            kind_id: feat.id,
            ..race_item(feat.id, "first")
        })
        .await
        .expect("the first mint lands");
    assert_eq!(minted.key, "FEAT-1");
    assert_eq!(
        counter(&db.pool, project.id, "FEAT").await,
        Some(1),
        "the row exists only after the mint"
    );
    assert_eq!(
        rows_of(&db.pool, "item_key_counter", project.id).await,
        1,
        "and only for the prefix that minted"
    );

    db.drop_db().await;
}

/// PRD D12's third fact: the counter row of the **old** prefix survives a rename.
///
/// The Postgres twin of `store::mem`'s test of the same name.
/// `item_kind_round_trip_and_prefix_rules` pins the other two (old key text kept, `ANL-1` next) on
/// both stores; no trait reader sees `item_key_counter`, so this one is per backend. It runs on
/// the fixture's `htui` project, whose `ANA` counter stands at 2.
#[tokio::test(flavor = "multi_thread")]
async fn renamed_prefix_leaves_the_old_counter_row() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let project = ids::PROJECT_HTUI;
    assert_eq!(
        counter(&db.pool, project, "ANA").await,
        Some(2),
        "the fixture minted ANA-1 and ANA-2"
    );

    let ana = db
        .store
        .item_kinds(project)
        .await
        .expect("kinds read")
        .into_iter()
        .find(|kind| kind.id == ids::KIND_HTUI_ANA)
        .expect("the fixture kind");
    let renamed = db
        .store
        .update_item_kind(
            ana.id,
            ana.updated_at,
            ItemKindPatch {
                prefix: Some("ANL".to_owned()),
                ..ItemKindPatch::default()
            },
        )
        .await
        .expect("the rename lands");
    assert!(matches!(renamed, CasOutcome::Applied(_)));
    assert_eq!(
        counter(&db.pool, project, "ANA").await,
        Some(2),
        "the old row is history, not garbage"
    );
    assert_eq!(
        counter(&db.pool, project, "ANL").await,
        None,
        "nothing minted under the new prefix yet"
    );

    let minted = db
        .store
        .mint_item(race_item(ids::KIND_HTUI_ANA, "after the rename"))
        .await
        .expect("the mint lands");
    assert_eq!(minted.key, "ANL-1");
    assert_eq!(counter(&db.pool, project, "ANL").await, Some(1));
    assert_eq!(counter(&db.pool, project, "ANA").await, Some(2), "still");

    db.drop_db().await;
}

/// `run_step_tree` rows go with their step when the run above them is deleted (ANA-2 §4.6).
///
/// The Postgres-only twin the `store::conformance` case
/// `trees_and_commits_round_trip` names: `MemStore` keeps its trees in a map it clears by hand, so
/// nothing in `htui-core` can show that `0003_orchestration.sql:93`'s
/// `REFERENCES run_step(id) ON DELETE CASCADE` is what actually takes them here. The delete is the
/// **run**, not the step, so both links of the chain - `run_step.run_id` from `0001_init.sql` and
/// `run_step_tree.run_step_id` from `0003` - are exercised at once.
///
/// `run_step_tree.repo_id` deliberately has **no** cascade (a tree outlives a repo rename, never a
/// step), so the repo is expected to stand afterwards; the case asserts that too, because a
/// cascade that reached it would be a silent data loss the conformance suite cannot see.
#[tokio::test(flavor = "multi_thread")]
async fn step_tree_rows_cascade_with_their_step() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    let repo = db
        .store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "cascade".to_owned(),
            default_branch: "main".to_owned(),
            is_primary: false,
            remote_url: None,
        })
        .await
        .expect("the repo lands")
        .id;
    db.store
        .upsert_step_tree(
            ids::STEP_R2_PRD,
            &[RunStepTree {
                run_step_id: ids::STEP_R2_PRD,
                repo_id: repo,
                mode: Isolation::Worktree,
                path: "/srv/trees/prd".to_owned(),
                base_ref: "main".to_owned(),
                dirty: false,
            }],
        )
        .await
        .expect("the tree row lands");

    /// How many `run_step_tree` rows hang off the fixture's `RUN_2`.
    async fn trees_of(pool: &PgPool, run: RunId) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM run_step_tree t \
               JOIN run_step s ON s.id = t.run_step_id WHERE s.run_id = $1",
        )
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("count run_step_tree")
    }

    assert_eq!(
        trees_of(&db.pool, ids::RUN_2).await,
        1,
        "the writer wrote the row this case is about to lose"
    );

    sqlx::query("DELETE FROM run WHERE id = $1")
        .bind(ids::RUN_2.as_uuid())
        .execute(&db.pool)
        .await
        .expect("the run is deleted");

    assert_eq!(
        trees_of(&db.pool, ids::RUN_2).await,
        0,
        "the tree rows went with the run's steps (0003_orchestration.sql:93)"
    );
    assert_eq!(
        db.store
            .step_trees(ids::STEP_R2_PRD)
            .await
            .expect("the read stands"),
        Vec::new(),
        "and the reader agrees"
    );
    assert!(
        db.store
            .repos(ids::PROJECT_HTUI)
            .await
            .expect("the repo read")
            .iter()
            .any(|row| row.id == repo),
        "run_step_tree.repo_id has no cascade: the repo outlives the tree"
    );

    db.drop_db().await;
}

/// MOD-4 milestone 1's eleven inherent orchestration reads (blueprint §3.5, F-N), against the
/// fixture.
///
/// `phase_agent`, `agent_box`, `repo_box_path` and the whole `box` row are outside the mirrored
/// table list of `docs/ANA-9.md` §4.4, so these eleven never became `ReadStore` methods and
/// `Backend`'s offline arm refuses them (plan D1). The conformance suite reaches only the traits,
/// so this is the **only** thing in the tree that pins Postgres's answers against the `MemStore`
/// the suite pins — which is what makes them one seam rather than two implementations.
///
/// Three reads are compared against seeded rows rather than against `MemStore`, because the
/// fixture has no `phase_agent`, `agent_box` or `repo_box_path` row and equality on two empty
/// vectors pins nothing. `phase_agents` stays deliberately divergent: `MemStore` holds no such
/// table and answers empty whatever the phase, which is the blueprint's F-N note and the reason
/// the snapshot builder falls back to `project.settings.default_agent_id`.
#[tokio::test(flavor = "multi_thread")]
async fn inherent_orchestration_reads_answer_the_fixture() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mem = htui_core::store::MemStore::demo();
    let scope = htui_core::model::Scope {
        workspace_id: ids::WORKSPACE_PLATFORM,
        project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
    };

    // ---- step_graph, prompt_template, resolve_graph: the fixture's own rows -------------------
    assert_eq!(
        db.store
            .step_graph(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("step_graph must not fail"),
        mem.step_graph(ids::GRAPH_HTUI_FEAT)
            .await
            .expect("MemStore::step_graph"),
        "one step_graph row, byte for byte"
    );
    assert_eq!(
        db.store
            .step_graph(htui_core::model::StepGraphId::new())
            .await
            .expect("an unknown graph is None, not an error"),
        None
    );

    let highest = db
        .store
        .prompt_template(ids::PROJECT_HTUI, "implement", None)
        .await
        .expect("prompt_template must not fail");
    assert_eq!(
        highest,
        mem.prompt_template(ids::PROJECT_HTUI, "implement", None)
            .await
            .expect("MemStore::prompt_template"),
        "the highest version of a name, on both backends"
    );
    let version = highest
        .as_ref()
        .expect("the fixture seeds `implement`")
        .version;
    assert_eq!(
        db.store
            .prompt_template(ids::PROJECT_HTUI, "implement", Some(version))
            .await
            .expect("the pin resolves"),
        highest,
        "a pin that names the highest version is the same row"
    );
    assert_eq!(
        db.store
            .prompt_template(ids::PROJECT_HTUI, "implement", Some(version + 99))
            .await
            .expect("an unhonourable pin is None, not an error"),
        None,
        "a pin that names no row does not fall back to the highest"
    );

    let resolved = db
        .store
        .resolve_graph(ids::HTUI_FEAT_1)
        .await
        .expect("resolve_graph must not fail")
        .expect("FEAT-1's kind has a default graph");
    assert_eq!(
        Some(&resolved),
        mem.resolve_graph(ids::HTUI_FEAT_1)
            .await
            .expect("MemStore::resolve_graph")
            .as_ref(),
        "the graph and its phases, in position order, on both backends"
    );
    assert!(
        resolved
            .phases
            .windows(2)
            .all(|pair| pair[0].phase.position < pair[1].phase.position),
        "phases are in `position` order"
    );
    assert_eq!(
        db.store
            .resolve_graph(ItemId::new())
            .await
            .expect("an unknown item is None, not an error"),
        None
    );

    // ---- box_row, active_runs_on_box, overlapping_runs, ready_items, missing_tags -------------
    assert_eq!(
        db.store
            .box_row(ids::BOX)
            .await
            .expect("box_row must not fail"),
        mem.box_row(ids::BOX).await.expect("MemStore::box_row"),
        "the whole box row, both tag lists and `settings` included"
    );
    let this_box = db
        .store
        .box_row(ids::BOX)
        .await
        .expect("box_row")
        .expect("the fixture's one box");
    assert_eq!(
        (
            this_box.probed_tags.as_slice(),
            this_box.declared_tags.as_slice()
        ),
        (
            ["rust", "msvc", "cmake"].map(str::to_owned).as_slice(),
            ["gpu"].map(str::to_owned).as_slice()
        ),
        "the two `Vec<String>` columns are adjacent in the struct and a transposed select list \
         would type-check (T1 audit A-5)"
    );

    assert_eq!(
        db.store
            .active_runs_on_box(ids::BOX)
            .await
            .expect("active_runs_on_box must not fail"),
        mem.active_runs_on_box(ids::BOX)
            .await
            .expect("MemStore::active_runs_on_box"),
        "§4.7's slot count: `running` and `awaiting_approval`, never `queued`"
    );

    // Every fixture run has an empty `repo_scope` — the fixture has no `repo` row — so the
    // overlap read is given one by hand rather than asserted vacuously. `RUN_2` is `queued`,
    // which is active.
    let scoped = RepoId::new();
    let elsewhere = RepoId::new();
    assert_eq!(
        db.store
            .overlapping_runs(&[scoped])
            .await
            .expect("overlapping_runs must not fail"),
        mem.overlapping_runs(&[scoped])
            .await
            .expect("MemStore::overlapping_runs"),
        "with no run in the scope both backends answer the same nothing"
    );
    sqlx::query!(
        "UPDATE run SET repo_scope = $2::uuid[] WHERE id = $1",
        ids::RUN_2.as_uuid(),
        &[scoped.as_uuid()][..],
    )
    .execute(&db.pool)
    .await
    .expect("give the queued run a scope");
    assert_eq!(
        db.store
            .overlapping_runs(&[scoped])
            .await
            .expect("overlapping_runs")
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![ids::RUN_2],
        "an active run whose scope intersects is named"
    );
    assert!(
        db.store
            .overlapping_runs(&[elsewhere])
            .await
            .expect("overlapping_runs")
            .is_empty(),
        "a disjoint scope names nothing"
    );
    assert!(
        db.store
            .overlapping_runs(&[])
            .await
            .expect("an empty scope is not an error")
            .is_empty(),
        "an empty scope intersects nothing (hazard H-10)"
    );
    db.store
        .fail_run(ids::RUN_2, "no longer wanted", Utc::now())
        .await
        .expect("terminate the run");
    assert!(
        db.store
            .overlapping_runs(&[scoped])
            .await
            .expect("overlapping_runs")
            .is_empty(),
        "and a terminal run holds no scope at all"
    );

    let ready = db
        .store
        .ready_items(&scope, ids::BOX)
        .await
        .expect("ready_items must not fail");
    assert_eq!(
        ready,
        mem.ready_items(&scope, ids::BOX)
            .await
            .expect("MemStore::ready_items"),
        "§7.4's readiness and `R-ORCH-10`'s capability half agree on both backends"
    );
    assert!(
        ready.iter().all(|row| row
            .required_tags
            .iter()
            .all(|tag| this_box.probed_tags.contains(tag) || this_box.declared_tags.contains(tag))),
        "nothing this box cannot run is offered"
    );
    assert!(
        db.store
            .ready_items(&scope, BoxId::new())
            .await
            .expect("an unknown box is not an error")
            .iter()
            .all(|row| row.required_tags.is_empty()),
        "a box with no row has no capabilities, so only untagged items are ready"
    );

    // `docker` is neither probed nor declared; `cmake` is probed and `gpu` declared.
    assert_eq!(
        db.store
            .missing_tags(ids::HTUI_TOOL_1, ids::BOX)
            .await
            .expect("missing_tags must not fail"),
        mem.missing_tags(ids::HTUI_TOOL_1, ids::BOX)
            .await
            .expect("MemStore::missing_tags"),
        "the tags this box cannot cover, in byte order"
    );
    assert_eq!(
        db.store
            .missing_tags(ids::HTUI_TOOL_1, ids::BOX)
            .await
            .expect("missing_tags"),
        vec!["docker".to_owned()],
        "and they are the right ones"
    );
    assert!(
        db.store
            .missing_tags(ids::HTUI_FEAT_1, ids::BOX)
            .await
            .expect("missing_tags")
            .is_empty(),
        "an item this box can take is missing nothing"
    );
    assert!(
        matches!(
            db.store.missing_tags(ItemId::new(), ids::BOX).await,
            Err(htui_core::store::StoreError::NotFound { entity: "item", .. })
        ),
        "an empty answer means `this box can take it`, so an unknown item cannot be spelled alike"
    );
    assert!(
        matches!(
            db.store.missing_tags(ids::HTUI_FEAT_1, BoxId::new()).await,
            Err(htui_core::store::StoreError::NotFound { entity: "box", .. })
        ),
        "nor an unknown box"
    );

    // ---- phase_agents, agent_boxes, repo_paths: seeded, because the fixture has no such row ---
    sqlx::query!(
        "INSERT INTO phase_agent (phase_id, position, agent_id, model) \
         VALUES ($1, 1, $2, 'opus'), ($1, 0, $3, 'sonnet')",
        ids::PHASE_HTUI_IMPLEMENT.as_uuid(),
        ids::AGENT_AGY.as_uuid(),
        ids::AGENT_CLAUDE.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect("seed two phase_agent rows out of position order");

    let candidates = db
        .store
        .phase_agents(ids::PHASE_HTUI_IMPLEMENT)
        .await
        .expect("phase_agents must not fail");
    assert_eq!(
        candidates
            .iter()
            .map(|row| (row.position, row.agent_id, row.model.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, ids::AGENT_CLAUDE, "sonnet"),
            (1, ids::AGENT_AGY, "opus")
        ],
        "candidates come back in `position` order, not insertion order"
    );
    assert!(
        mem.phase_agents(ids::PHASE_HTUI_IMPLEMENT)
            .await
            .expect("MemStore::phase_agents")
            .is_empty(),
        "`MemStore` holds no `phase_agent` table and says so (blueprint F-N)"
    );
    assert_eq!(
        db.store
            .resolve_graph(ids::HTUI_FEAT_1)
            .await
            .expect("resolve_graph")
            .expect("the graph")
            .phases
            .iter()
            .find(|row| row.phase.id == ids::PHASE_HTUI_IMPLEMENT)
            .map(|row| row.agents.len()),
        Some(2),
        "and `resolve_graph` carries the candidates it now has"
    );

    sqlx::query!(
        "INSERT INTO agent_box (agent_id, box_id, enabled, version, path) \
         VALUES ($1, $2, true, '1.2.3', '/usr/bin/claude')",
        ids::AGENT_CLAUDE.as_uuid(),
        ids::BOX.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect("seed one agent_box row");
    let installed = db
        .store
        .agent_boxes(ids::BOX)
        .await
        .expect("agent_boxes must not fail");
    assert_eq!(
        installed
            .iter()
            .map(|row| (row.agent_id, row.enabled, row.version.as_deref()))
            .collect::<Vec<_>>(),
        vec![(ids::AGENT_CLAUDE, true, Some("1.2.3"))],
        "the box's own agent rows, and no other box's"
    );
    assert!(
        db.store
            .agent_boxes(BoxId::new())
            .await
            .expect("an unknown box is not an error")
            .is_empty()
    );

    let repo = db
        .store
        .create_repo(NewRepo {
            id: RepoId::new(),
            project_id: ids::PROJECT_HTUI,
            name: "core".to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("create the repo the path hangs off")
        .id;
    sqlx::query!(
        "INSERT INTO repo_box_path (repo_id, box_id, local_path) VALUES ($1, $2, 'C:/src/core')",
        repo.as_uuid(),
        ids::BOX.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect("seed one repo_box_path row");
    assert_eq!(
        db.store
            .repo_paths(ids::BOX)
            .await
            .expect("repo_paths must not fail")
            .iter()
            .map(|row| (row.repo_id, row.local_path.as_str()))
            .collect::<Vec<_>>(),
        vec![(repo, "C:/src/core")],
        "every checkout path on one box (`R-BOX-4`), the mirror image of `repo_box_path_rows`"
    );

    db.drop_db().await;
}

/// `0003`'s `ck_run_graph_snapshot` is `NOT VALID`: it is checked for every new row and for no
/// row that was already there (ANA-2 §5.1, plan D7).
///
/// `run_create_moves_the_item` in the conformance suite delegates the column's own guard here,
/// because `MemStore` has no `CHECK` to show. Two halves: the constraint is still unvalidated in
/// the catalogue — which is what lets a database with pre-MOD-4 `graph` runs take the migration at
/// all — and a fresh `kind = 'graph'` row with no snapshot is refused by it.
#[tokio::test(flavor = "multi_thread")]
async fn ck_run_graph_snapshot_is_not_valid_for_old_rows_and_checked_for_new() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    let validated = sqlx::query_scalar!(
        "SELECT convalidated FROM pg_constraint WHERE conname = 'ck_run_graph_snapshot'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the constraint exists");
    assert!(
        !validated,
        "`NOT VALID` is the whole point: a validated constraint would have had to scan `run` on \
         upgrade and would have refused a database holding pre-MOD-4 `graph` runs"
    );

    // The fixture's own rows loaded under it, which is the "new row" path already exercised.
    let graph_runs = sqlx::query_scalar!(
        "SELECT COUNT(*) AS \"count!\" FROM run WHERE kind = 'graph' AND graph_snapshot IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count the fixture's graph runs");
    assert!(
        graph_runs > 0,
        "the fixture carries graph runs and every one of them has a snapshot (D13)"
    );

    let refused = sqlx::query!(
        "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                          graph_snapshot, started_by, queued_at) \
         VALUES ($1, $2, NULL, 'graph', 'manual', 'queued', $3, NULL, $4, now())",
        RunId::new().as_uuid(),
        ids::PROJECT_HTUI.as_uuid(),
        ids::BOX.as_uuid(),
        ids::USER.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect_err("a graph run with no snapshot is refused");
    assert_eq!(
        refused
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23514"),
        "and refused by the `CHECK`, not by anything else: {refused}"
    );

    // The same insert as a `chat` run is accepted: the constraint is implication, not NOT NULL.
    sqlx::query!(
        "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                          graph_snapshot, started_by, queued_at) \
         VALUES ($1, $2, NULL, 'chat', 'manual', 'queued', $3, NULL, $4, now())",
        RunId::new().as_uuid(),
        ids::PROJECT_HTUI.as_uuid(),
        ids::BOX.as_uuid(),
        ids::USER.as_uuid(),
    )
    .execute(&db.pool)
    .await
    .expect("a chat run needs no snapshot");

    db.drop_db().await;
}

/// Plan D6: two writers arriving together at one `(item, kind)` get consecutive versions, never
/// the same one.
///
/// The conformance case `write_document_allocates_its_version` delegates this here: `MemStore`
/// holds a `RwLock` and cannot show it. What makes it hold on Postgres is the
/// `SELECT 1 FROM item ... FOR UPDATE` `write_document` takes before its
/// `COALESCE(MAX(version), 0) + 1` — the row the second writer would otherwise have to wait for
/// does not exist in `document` yet, so the parent is the only row both contend on. Without that
/// lock both statements read `MAX(version) = 2` and both write `3`.
#[tokio::test(flavor = "multi_thread")]
async fn document_versions_do_not_collide_under_contention() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let left = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("second pool")
        .store;
    let right = PgStore::connect(&db.url, &db.identity)
        .await
        .expect("third pool")
        .store;

    let plan = |title: &str| htui_core::model::NewDocument {
        id: htui_core::model::DocumentId::new(),
        item_id: ids::HTUI_FEAT_1,
        kind: "plan".to_owned(),
        title: title.to_owned(),
        body: String::new(),
        produced_by_step_id: None,
        created_by: ids::USER,
        created_at: Utc::now(),
    };

    let (one, two) = tokio::join!(
        left.write_document(plan("Left")),
        right.write_document(plan("Right")),
    );
    let mut versions = [
        one.expect("the first write must not fail").version,
        two.expect("the second write must not fail").version,
    ];
    versions.sort_unstable();
    assert_eq!(
        versions,
        [3, 4],
        "the fixture holds plan v1 and v2, so the two concurrent writers take 3 and 4 - never 3 \
         twice (plan D6)"
    );

    let stored = sqlx::query_scalar!(
        "SELECT version FROM document WHERE item_id = $1 AND kind = 'plan' ORDER BY version",
        ids::HTUI_FEAT_1.as_uuid(),
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the versions back");
    assert_eq!(
        stored,
        vec![1, 2, 3, 4],
        "and the table agrees: no gap, no repeat"
    );

    db.drop_db().await;
}

/// Plan M2 D7: `finish_run` derives the item from the item's *remaining* live runs, and on
/// Postgres it does so under the item's row lock.
///
/// The conformance case `finish_run_moves_run_and_item_together` delegates the column half here:
/// `item.closed_at` is not on [`htui_core::model::Item`]'s critical path for the `MemStore`
/// assertions, and this is where the SQL that writes it can be read back directly. The two-run leg
/// is repeated rather than referenced because it is the one the plan's Risks row names, and a
/// `PgStore` that counted the item's runs *without* the `FOR UPDATE` would still pass it
/// sequentially while stranding the item under concurrency.
///
/// Getting two live graph runs onto one item takes the same detour as the conformance case:
/// `create_run` admits only `open | failed` items and leaves the item at `queued`, so the item is
/// walked back `queued -> open` between the two creates.
#[tokio::test(flavor = "multi_thread")]
async fn finish_run_holds_the_item_while_another_run_is_live() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let owner = uuid::Uuid::now_v7();
    let at = Utc::now();
    let until = at + TimeDelta::minutes(5);

    let closed_at = async |item: ItemId| {
        sqlx::query_scalar!(
            r#"SELECT closed_at FROM item WHERE id = $1"#,
            item.as_uuid(),
        )
        .fetch_one(&db.pool)
        .await
        .expect("read the item's closed_at back")
    };

    let left = db
        .store
        .create_run(race_run(ids::HTUI_ANA_2))
        .await
        .expect("the first run is queued")
        .id;
    assert!(
        db.store
            .transition(ids::HTUI_ANA_2, Status::Queued, Status::Open)
            .await
            .expect("the walk back must not fail"),
        "queued -> open is sanctioned, and is the only way to queue a second run on one item"
    );
    let right = db
        .store
        .create_run(race_run(ids::HTUI_ANA_2))
        .await
        .expect("the second run is queued")
        .id;
    for run in [left, right] {
        assert_eq!(
            db.store
                .claim_run(run, ids::BOX, owner, at, until)
                .await
                .expect("the claim must not fail"),
            Claim::Admitted,
            "both runs fit the fixture box's two slots"
        );
    }

    db.store
        .finish_run(left, RunStatus::Done, None, at)
        .await
        .expect("the first finish must not fail");
    let held = db
        .store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    assert_eq!(
        held.status,
        Status::InProgress,
        "the item is held while its other run is still live"
    );
    assert_eq!(
        closed_at(ids::HTUI_ANA_2).await,
        None,
        "and nothing was closed off"
    );

    db.store
        .finish_run(right, RunStatus::Done, None, at)
        .await
        .expect("the second finish must not fail");
    let moved = db
        .store
        .item(ids::HTUI_ANA_2)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    assert_eq!(
        moved.status,
        Status::Done,
        "the last run out moves the item"
    );
    assert!(
        closed_at(ids::HTUI_ANA_2).await.is_some(),
        "`done` is terminal for an item, so the mirror sets `closed_at` - the same rule \
         `transition` writes, not a second one"
    );

    // The other direction of that rule, on its own item: a `cancelled` run hands the item back to
    // the backlog, and `open` is not terminal, so `closed_at` stays clear.
    let cancelled = db
        .store
        .create_run(race_run(ids::HTUI_CLEAN_1))
        .await
        .expect("a failed item can be re-queued")
        .id;
    db.store
        .finish_run(cancelled, RunStatus::Cancelled, None, at)
        .await
        .expect("the cancel must not fail");
    let released = db
        .store
        .item(ids::HTUI_CLEAN_1)
        .await
        .expect("read must not fail")
        .expect("the fixture item exists");
    assert_eq!(
        released.status,
        Status::Open,
        "queued -> open is the only row of the table whose `from` is `queued`"
    );
    assert_eq!(
        closed_at(ids::HTUI_CLEAN_1).await,
        None,
        "and `open` is not terminal, so `closed_at` is cleared rather than stamped"
    );

    db.drop_db().await;
}

/// Plan D31's row against a real server: what `store::conformance`'s `verify_run_is_recorded`
/// asserts of both stores, plus the two things only Postgres can answer.
///
/// The first is `command_run.output`. ANA-2 §4.2 caps a verify's captured output at 64 KiB per
/// stream before it reaches the seam, so a row can carry ~128 KiB plus the command's own text.
/// `TEXT` has no declared limit and the column is not `VARCHAR(n)`, but that is a claim about the
/// DDL rather than about the round trip: a driver that truncated, re-encoded or normalised line
/// endings would pass every `MemStore` assertion and lose the tail of the one artefact a human
/// reads after a failed verify. So this writes 70 KiB - the cap plus slack - of text chosen to
/// exercise the encoder rather than the allocator, and compares it byte for byte.
///
/// The second is the ordering. `MemStore` sorts in Rust; Postgres sorts in the statement, and
/// `ORDER BY queued_at, id` over a `timestamptz` and a `uuid` is a different comparison from
/// `chrono`'s and `Uuid`'s. Three rows queued out of insertion order, two of them sharing a
/// `queued_at` to the microsecond, pin that the tiebreak is the `id` and not the heap.
#[tokio::test(flavor = "multi_thread")]
async fn command_run_round_trips_and_orders_by_queued_at() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let t0 = Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS);

    // 70 KiB: the 64 KiB tail cap plus slack, and not one repeated byte — a `TEXT` round trip that
    // survives 70 000 `a`s says less about the encoder than one that survives multi-byte
    // characters, embedded newlines and a NUL-adjacent control character.
    let output: String = "error: `λ` unresolved\r\n\ttest \u{1}line\n"
        .repeat(2_000)
        .chars()
        .take(70 * 1024)
        .collect();

    let base = NewCommandRun {
        id: CommandRunId::new(),
        run_step_id: ids::STEP_R2_PRD,
        box_id: ids::BOX,
        class: "verify".to_owned(),
        command: "cargo test --all-features".to_owned(),
        cwd: "/srv/trees/prd/core".to_owned(),
        status: CommandRunStatus::Done,
        exit_code: Some(101),
        output: Some(output.clone()),
        queued_at: t0,
        started_at: Some(t0),
        finished_at: Some(t0 + TimeDelta::seconds(3)),
    };

    let written = db
        .store
        .record_command_run(base.clone())
        .await
        .expect("the row lands");
    assert_eq!(
        written.output.as_deref(),
        Some(output.as_str()),
        "the writer hands back exactly what it was given"
    );

    // Two more rows: one queued a second earlier, one sharing `queued_at` with the first. Written
    // last-first, so insertion order and `queued_at` order disagree.
    let earlier = NewCommandRun {
        id: CommandRunId::new(),
        status: CommandRunStatus::Failed,
        exit_code: None,
        output: Some("no `sh` on PATH".to_owned()),
        queued_at: t0 - TimeDelta::seconds(1),
        started_at: None,
        finished_at: None,
        ..base.clone()
    };
    db.store
        .record_command_run(earlier.clone())
        .await
        .expect("the earlier row lands");
    let tied = NewCommandRun {
        id: CommandRunId::new(),
        output: None,
        ..base.clone()
    };
    assert!(
        tied.id > base.id,
        "UUIDv7 ids are minted in order, so the tie must break towards the later row"
    );
    db.store
        .record_command_run(tied.clone())
        .await
        .expect("the tied row lands");

    let rows = db
        .store
        .command_runs(ids::STEP_R2_PRD)
        .await
        .expect("the read");
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![earlier.id, base.id, tied.id],
        "`ORDER BY queued_at, id`: the earlier row first, then the tie broken by id"
    );
    assert_eq!(
        rows.get(1).and_then(|row| row.output.as_deref()),
        Some(output.as_str()),
        "and 70 KiB of `TEXT` comes back byte for byte"
    );
    assert_eq!(
        rows.first().map(|row| row.status),
        Some(CommandRunStatus::Failed),
        "the `status` column decodes through the `CHECK` list's text"
    );
    assert_eq!(
        rows.first().and_then(|row| row.exit_code),
        None,
        "a command that never ran has no exit code"
    );
    assert_eq!(
        rows.get(2).map(|row| row.queued_at),
        Some(t0),
        "the caller's instant survives the round trip at microsecond precision"
    );

    // An unknown box is the foreign key's refusal, not the step's.
    let no_box = db
        .store
        .record_command_run(NewCommandRun {
            id: CommandRunId::new(),
            box_id: BoxId::new(),
            ..base.clone()
        })
        .await;
    assert!(
        matches!(&no_box, Err(htui_core::store::StoreError::Constraint(text)) if text.contains("command_run")),
        "an unknown box is a 23503 naming the table, got {no_box:?}"
    );
    let no_step = db
        .store
        .record_command_run(NewCommandRun {
            id: CommandRunId::new(),
            run_step_id: StepId::new(),
            box_id: BoxId::new(),
            ..base.clone()
        })
        .await;
    assert!(
        matches!(
            no_step,
            Err(htui_core::store::StoreError::NotFound {
                entity: "run_step",
                ..
            })
        ),
        "and the step is checked before the box, got {no_step:?}"
    );
    assert_eq!(
        common::count(&db.pool, "command_run").await,
        3,
        "neither refusal left a row behind — the insert is its own transaction"
    );

    db.drop_db().await;
}

/// Plan D33's column, read as a column rather than through the projection that carries it.
///
/// `store::conformance`'s `trees_and_commits_round_trip` asserts `run_step.isolation_path` through
/// `run_steps`, which is a `SELECT` this crate also owns: a batch that wrote the wrong path and a
/// projection that read the wrong column would agree with each other. This reads the column
/// directly, and it reads `updated_at` either side of the write - the one thing the seam cannot
/// show, because nothing sets it by hand. `trg_run_step_updated_at` (`0001_init.sql:574-580`) is
/// what has to move it, and the §4.4 cache cursor rides on its doing so.
#[tokio::test(flavor = "multi_thread")]
async fn upsert_step_tree_writes_the_primary_isolation_path() {
    let Some(db) = common::demo_db().await else {
        return;
    };

    let repo = async |name: &str, is_primary: bool| -> RepoId {
        db.store
            .create_repo(NewRepo {
                id: RepoId::new(),
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                default_branch: "main".to_owned(),
                is_primary,
                remote_url: None,
            })
            .await
            .expect("the repo lands")
            .id
    };
    let core = repo("core", true).await;
    let docs = repo("docs", false).await;

    let column = async || -> (Option<String>, DateTime<Utc>) {
        let row = sqlx::query!(
            "SELECT isolation_path, updated_at FROM run_step WHERE id = $1",
            ids::STEP_R2_PRD.as_uuid(),
        )
        .fetch_one(&db.pool)
        .await
        .expect("the fixture step exists");
        (row.isolation_path, row.updated_at)
    };

    let (before, stamped_before) = column().await;
    assert_eq!(before, None, "nothing has written the column yet");

    let tree = |repo_id: RepoId, path: &str| RunStepTree {
        run_step_id: ids::STEP_R2_PRD,
        repo_id,
        mode: Isolation::Worktree,
        path: path.to_owned(),
        base_ref: "main".to_owned(),
        dirty: false,
    };
    // `docs` first, so batch order and the rule disagree.
    db.store
        .upsert_step_tree(
            ids::STEP_R2_PRD,
            &[
                tree(docs, "/srv/trees/prd/docs"),
                tree(core, "/srv/trees/prd/core"),
            ],
        )
        .await
        .expect("the two-row batch lands");

    let (after, stamped_after) = column().await;
    assert_eq!(
        after.as_deref(),
        Some("/srv/trees/prd/core"),
        "the primary repo's path is the step's, whatever order the batch listed"
    );
    assert!(
        stamped_after > stamped_before,
        "the `UPDATE` fired `trg_run_step_updated_at`, which is what the cache cursor rides on"
    );

    // A batch with no primary in it falls back to the lowest `repo_id`, which is a batch of one.
    db.store
        .upsert_step_tree(ids::STEP_R2_PRD, &[tree(docs, "/srv/trees/prd/docs")])
        .await
        .expect("the one-row batch lands");
    assert_eq!(
        column().await.0.as_deref(),
        Some("/srv/trees/prd/docs"),
        "a batch with no primary chooses its lowest `repo_id`"
    );

    db.store
        .upsert_step_tree(ids::STEP_R2_PRD, &[])
        .await
        .expect("the empty batch is a check, not a write");
    assert_eq!(
        column().await.0.as_deref(),
        Some("/srv/trees/prd/docs"),
        "an empty batch names no tree, so it leaves the column alone"
    );

    db.drop_db().await;
}
