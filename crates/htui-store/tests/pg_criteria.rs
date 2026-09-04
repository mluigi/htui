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

mod common;

use futures::future::join_all;
use htui_core::fixtures::ids;
use htui_core::model::{ItemFilter, ItemId, ItemKindId, ItemPatch, NewItem, ProjectId, Status};
use htui_core::store::{ReadStore as _, UpdateOutcome, WriteStore as _};
use htui_store::PgStore;
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
