//! The permission and control relay on Postgres (MOD-42 plan D1-D5, D12-D14; blueprint §2.10;
//! `0011_permission_relay.sql`).
//!
//! `impl WriteStore for PgStore` (`write.rs`) delegates its relay methods here. Every status
//! move is one compare-and-set statement (I-3): under `READ COMMITTED` the loser of a race blocks
//! on the row lock, re-evaluates its `status` predicate against the committed row and matches
//! nothing. Every miss is told apart by one re-read, the `take_lease` shape. Every instant is
//! `clock_timestamp()`, never a box clock (I-4). MOD-42's only transaction is `open_permission`'s,
//! which key-share-locks the step and then share-locks its run (`FOR KEY SHARE OF s FOR SHARE OF
//! r`, MOD-77 plan D4, review L1: `park_step`'s order, and `KEY SHARE` is the lock the insert's
//! foreign key takes anyway) so an adoption cannot commit between its fence and its insert
//! (MOD-41's `step_fence` shape).
//!
//! An answer or a cancel request names an actor (`answered_by`/`answered_box`,
//! `issued_by`/`issued_box`, blueprint B-7). `MemStore` checks the actor **before** the row's
//! status, so the re-read of a miss checks it first too: an unknown user or box is a
//! [`StoreError::Constraint`] whether or not the compare-and-set would have matched, never
//! `Refused(Answered)` or `AlreadyPending`.
//!
//! MOD-70 (plan D1-D5, D9; blueprint §2.7; `0016_follow_up.sql`) adds follow-ups for engine steps.
//! The enqueue is the one `INSERT … SELECT … FOR SHARE OF w` on the step's `follow_up_window`
//! row, so it and a close's `UPDATE` of that row serialise (I-3): an enqueue that holds the lock
//! commits before the close reads, one that arrives later waits and reads the window closed.
//! MOD-70 has four multi-statement transactions: `open_follow_ups` (fence, upsert, refuse; B-6),
//! the two closes (close the window, then refuse what is still pending), and `request_cancel`
//! (insert the cancel, then refuse the run's pending follow-ups; B-14). Each relies on `READ
//! COMMITTED` (F-18), so each pins it as its first statement ([`begin_read_committed`]) rather than
//! inherit the server's default: a later statement takes a fresh snapshot, which sees an enqueue
//! that committed while an earlier one waited; under `REPEATABLE READ`, or as one CTE, it would
//! not. Every resolution nulls the text (I-5).

use chrono::{DateTime, Utc};
use htui_core::model::{
    AnswerOutcome, AnswerRefusal, BoxId, CancelRequest, FOLLOW_UP_RUN_CANCELLED,
    FOLLOW_UP_SESSION_ENDED, FollowUpRefusal, FollowUpRequest, FollowUpSettle, FollowUpView,
    ItemId, NewFollowUp, OpenPermission, PermissionChoice, PermissionId, PermissionStatus,
    ProjectId, QueuedFollowUp, RelayOption, RelaySessionId, RelayView, RunCommand, RunCommandId,
    RunCommandKind, RunCommandStatus, RunId, RunKind, Scope, SettleOutcome, StepId, StepPermission,
    StepStatus, UserId, WaitingPermission,
};
use htui_core::store::{Result, StoreError, references_no_row};
use sqlx::types::Json;
use uuid::Uuid;

use crate::error::map_sqlx;
use crate::pg::PgStore;

/// How many times [`request_cancel`] retries a pending row that was resolved between its insert
/// and its re-read before it gives up.
const CANCEL_ATTEMPTS: usize = 3;

/// How many times [`request_follow_up`] retries when its re-read finds every guard passing (the
/// pending row it collided with resolved in between) before it gives up (MOD-70 D3).
const FOLLOW_UP_ATTEMPTS: usize = 3;

/// MOD-70 review M-2 (F-18): a transaction pinned at `READ COMMITTED`, whatever the server's or the
/// role's `default_transaction_isolation`. Every multi-statement transaction of this module relies
/// on each statement taking a fresh snapshot (see the module doc); `SET TRANSACTION` must be the
/// transaction's first statement, which is why this is a helper (`write.rs`'
/// `begin_repeatable_read` is the same shape).
async fn begin_read_committed(
    pool: &sqlx::PgPool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    Ok(tx)
}

/// The step fence [`open_permission`] and [`open_follow_ups`] share: the step's run and that run's
/// lease owner, read under `FOR KEY SHARE OF s FOR SHARE OF r`.
#[derive(Debug)]
struct StepFenceRow {
    /// `run_step.run_id`.
    run_id: Uuid,
    /// `run.lease_owner`.
    lease_owner: Option<Uuid>,
}

/// MOD-77 plan D4, review L1: the step is key-share-locked (the insert's foreign-key lock) and
/// then its run share-locked, `park_step`'s order, so an adoption cannot commit between the
/// caller's check and its write, and a park holding the step cannot deadlock against it. One query
/// text for both callers, so `.sqlx` keeps one entry (MOD-70 blueprint §2.7).
async fn lock_step_fence(
    tx: &mut sqlx::PgConnection,
    step: StepId,
) -> Result<Option<StepFenceRow>> {
    sqlx::query_as!(
        StepFenceRow,
        r#"SELECT s.run_id, r.lease_owner AS "lease_owner?"
             FROM run_step s JOIN run r ON r.id = s.run_id
            WHERE s.id = $1
              FOR KEY SHARE OF s FOR SHARE OF r"#,
        step.as_uuid(),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)
}

/// One `step_permission` row as the two readers select it: [`StepPermission`] with its options
/// still in their JSON wrapper.
#[derive(Debug)]
struct PermissionRecord {
    /// `step_permission.id`.
    id: PermissionId,
    /// `step_permission.run_id`.
    run_id: RunId,
    /// `step_permission.run_step_id`.
    run_step_id: StepId,
    /// `step_permission.session`.
    session: RelaySessionId,
    /// `step_permission.request_id`.
    request_id: String,
    /// `step_permission.tool_call_id`.
    tool_call_id: Option<String>,
    /// `step_permission.summary`.
    summary: Option<String>,
    /// `step_permission.options`.
    options: Json<Vec<RelayOption>>,
    /// `step_permission.status`.
    status: PermissionStatus,
    /// `step_permission.option_id`.
    option_id: Option<String>,
    /// `step_permission.answered_by`.
    answered_by: Option<UserId>,
    /// `step_permission.answered_box`.
    answered_box: Option<BoxId>,
    /// `step_permission.created_at`.
    created_at: DateTime<Utc>,
    /// `step_permission.answered_at`.
    answered_at: Option<DateTime<Utc>>,
    /// `step_permission.resolved_at`.
    resolved_at: Option<DateTime<Utc>>,
}

impl From<PermissionRecord> for StepPermission {
    fn from(row: PermissionRecord) -> Self {
        Self {
            id: row.id,
            run_id: row.run_id,
            run_step_id: row.run_step_id,
            session: row.session,
            request_id: row.request_id,
            tool_call_id: row.tool_call_id,
            summary: row.summary,
            options: row.options.0,
            status: row.status,
            option_id: row.option_id,
            answered_by: row.answered_by,
            answered_box: row.answered_box,
            created_at: row.created_at,
            answered_at: row.answered_at,
            resolved_at: row.resolved_at,
        }
    }
}

/// The actor check `MemStore` runs before the row's status (blueprint B-7): `table.{by}_by` must
/// name an `app_user`, `table.{by}_box` a `box`.
fn require_actor(
    known: (bool, bool),
    user: UserId,
    box_id: BoxId,
    table: &str,
    by: &str,
) -> Result<()> {
    let (user_known, box_known) = known;
    if !user_known {
        return Err(StoreError::Constraint(references_no_row(
            &format!("{table}.{by}_by"),
            user,
            "app_user",
        )));
    }
    if !box_known {
        return Err(StoreError::Constraint(references_no_row(
            &format!("{table}.{by}_box"),
            box_id,
            "box",
        )));
    }
    Ok(())
}

/// [`WriteStore::open_permission`](htui_core::store::WriteStore::open_permission): the fence,
/// D5's staling of older sessions and the insert, in one transaction.
pub(super) async fn open_permission(store: &PgStore, open: OpenPermission) -> Result<PermissionId> {
    let options = serde_json::to_value(&open.options).map_err(|err| {
        StoreError::Constraint(format!("step_permission.options does not serialise: {err}"))
    })?;
    let mut tx = store.pool.begin().await.map_err(map_sqlx)?;

    // MOD-77 plan D4, review L1: the fence locks the step, then its run (`lock_step_fence`).
    let fence = lock_step_fence(&mut tx, open.run_step_id)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: open.run_step_id.to_string(),
        })?;
    if fence.run_id != open.run_id.as_uuid() {
        return Err(StoreError::Constraint(format!(
            "step_permission.run_step_id `{}` is not a step of run `{}`",
            open.run_step_id, open.run_id
        )));
    }
    if fence.lease_owner != Some(open.owner) {
        return Err(StoreError::Fenced {
            step: open.run_step_id,
        });
    }

    // D5: an older session's open rows on the same step go stale. A refused insert below rolls
    // this back with it.
    sqlx::query!(
        "UPDATE step_permission SET status = 'stale', resolved_at = clock_timestamp() \
          WHERE run_step_id = $1 AND session <> $2 AND status IN ('pending', 'answered')",
        open.run_step_id.as_uuid(),
        open.session.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    sqlx::query!(
        "INSERT INTO step_permission (id, run_id, run_step_id, session, request_id, tool_call_id, \
                                      summary, options, owner) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        open.id.as_uuid(),
        open.run_id.as_uuid(),
        open.run_step_id.as_uuid(),
        open.session.as_uuid(),
        open.request_id,
        open.tool_call_id,
        open.summary,
        options,
        open.owner,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    tx.commit().await.map_err(map_sqlx)?;
    Ok(open.id)
}

/// [`WriteStore::permission`](htui_core::store::WriteStore::permission).
pub(super) async fn permission(
    store: &PgStore,
    id: PermissionId,
) -> Result<Option<StepPermission>> {
    let row = sqlx::query_as!(
        PermissionRecord,
        r#"SELECT id           AS "id: PermissionId",
                  run_id       AS "run_id: RunId",
                  run_step_id  AS "run_step_id: StepId",
                  session      AS "session: RelaySessionId",
                  request_id,
                  tool_call_id,
                  summary,
                  options      AS "options: Json<Vec<RelayOption>>",
                  status       AS "status: PermissionStatus",
                  option_id,
                  answered_by  AS "answered_by: UserId",
                  answered_box AS "answered_box: BoxId",
                  created_at,
                  answered_at,
                  resolved_at
             FROM step_permission WHERE id = $1"#,
        id.as_uuid(),
    )
    .fetch_optional(&store.pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row.map(StepPermission::from))
}

/// [`WriteStore::apply_permission`](htui_core::store::WriteStore::apply_permission): fenced on
/// the owner only (B-9).
pub(super) async fn apply_permission(
    store: &PgStore,
    id: PermissionId,
    owner: Uuid,
) -> Result<Option<PermissionChoice>> {
    let applied = sqlx::query_scalar!(
        r#"UPDATE step_permission p
              SET status = 'applied', resolved_at = clock_timestamp()
             FROM run r
            WHERE p.id = $1 AND p.status = 'answered' AND p.owner = $2
              AND r.id = p.run_id AND r.lease_owner = $2
        RETURNING p.option_id AS "option_id!""#,
        id.as_uuid(),
        owner,
    )
    .fetch_optional(&store.pool)
    .await
    .map_err(map_sqlx)?;
    if let Some(option_id) = applied {
        return Ok(Some(PermissionChoice { option_id }));
    }
    permission_exists(store, id).await?;
    Ok(None)
}

/// `NotFound { entity: "step_permission" }` for an id no row has: the second half of a miss.
async fn permission_exists(store: &PgStore, id: PermissionId) -> Result<()> {
    sqlx::query_scalar!("SELECT 1 FROM step_permission WHERE id = $1", id.as_uuid())
        .fetch_optional(&store.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "step_permission",
            id: id.to_string(),
        })?;
    Ok(())
}

/// [`WriteStore::settle_permissions`](htui_core::store::WriteStore::settle_permissions).
pub(super) async fn settle_permissions(
    store: &PgStore,
    session: RelaySessionId,
    to: PermissionStatus,
) -> Result<u64> {
    if !matches!(to, PermissionStatus::Cancelled | PermissionStatus::Stale) {
        return Err(StoreError::Constraint(format!(
            "step_permission rows settle to `cancelled` or `stale`, not `{to}`"
        )));
    }
    let moved = sqlx::query!(
        "UPDATE step_permission SET status = $2, resolved_at = clock_timestamp() \
          WHERE session = $1 AND status IN ('pending', 'answered')",
        session.as_uuid(),
        to.as_str(),
    )
    .execute(&store.pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(moved)
}

/// [`WriteStore::answer_permission`](htui_core::store::WriteStore::answer_permission): D3's
/// compare-and-set, then on a miss one re-read deciding, in `MemStore`'s order: `NotFound`, the
/// actor (`Constraint`), the status, `NotOffered`, `ExecutorGone`.
pub(super) async fn answer_permission(
    store: &PgStore,
    id: PermissionId,
    option_id: &str,
    user: UserId,
    box_id: BoxId,
) -> Result<AnswerOutcome> {
    // An unknown actor on a row that would win fails the foreign key here (`23503`).
    let won = sqlx::query!(
        "UPDATE step_permission p \
            SET status = 'answered', option_id = $2, answered_by = $3, answered_box = $4, \
                answered_at = clock_timestamp() \
          WHERE p.id = $1 \
            AND p.status = 'pending' \
            AND p.options @> jsonb_build_array(jsonb_build_object('id', $2::text)) \
            AND p.owner = (SELECT r.lease_owner FROM run r \
                            WHERE r.id = p.run_id AND r.lease_expires_at > clock_timestamp())",
        id.as_uuid(),
        option_id,
        user.as_uuid(),
        box_id.as_uuid(),
    )
    .execute(&store.pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    if won == 1 {
        return Ok(AnswerOutcome::Answered);
    }

    let state = sqlx::query!(
        r#"SELECT p.status AS "status: PermissionStatus",
                  p.options @> jsonb_build_array(jsonb_build_object('id', $2::text)) AS "offered!",
                  EXISTS (SELECT 1 FROM app_user WHERE id = $3) AS "user_known!",
                  EXISTS (SELECT 1 FROM box WHERE id = $4) AS "box_known!"
             FROM step_permission p JOIN run r ON r.id = p.run_id
            WHERE p.id = $1"#,
        id.as_uuid(),
        option_id,
        user.as_uuid(),
        box_id.as_uuid(),
    )
    .fetch_optional(&store.pool)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound {
        entity: "step_permission",
        id: id.to_string(),
    })?;
    require_actor(
        (state.user_known, state.box_known),
        user,
        box_id,
        "step_permission",
        "answered",
    )?;
    let refusal = match state.status {
        PermissionStatus::Pending if !state.offered => AnswerRefusal::NotOffered,
        // Pending and offered, so the lease is what failed the compare-and-set: its owner is
        // not the row's or it has expired. (Should it read live again by now, a concurrent change
        // was undone before the re-read; nothing was written either way.)
        PermissionStatus::Pending => AnswerRefusal::ExecutorGone,
        PermissionStatus::Answered => AnswerRefusal::Answered,
        PermissionStatus::Applied => AnswerRefusal::Applied,
        PermissionStatus::Cancelled => AnswerRefusal::Cancelled,
        PermissionStatus::Stale => AnswerRefusal::Stale,
    };
    Ok(AnswerOutcome::Refused(refusal))
}

/// [`WriteStore::request_cancel`](htui_core::store::WriteStore::request_cancel): an insert the
/// partial unique index admits once per run, then on a miss one re-read deciding, in `MemStore`'s
/// order: `NotFound { run }`, the actor (`Constraint`), `AlreadyPending`. A pending row resolved
/// between the two statements is retried, [`CANCEL_ATTEMPTS`] times in all.
///
/// MOD-70 D5, B-14: an `Inserted` cancel refuses the run's pending follow-ups with
/// [`FOLLOW_UP_RUN_CANCELLED`] in its insert's transaction; an `AlreadyPending` one refuses them
/// too (a follow-up that slipped in between a first cancel's two statements, R-5). Idempotent.
pub(super) async fn request_cancel(
    store: &PgStore,
    run: RunId,
    user: UserId,
    box_id: BoxId,
) -> Result<CancelRequest> {
    for _ in 0..CANCEL_ATTEMPTS {
        // F-18: the refusal after the insert reads a fresh snapshot.
        let mut tx = begin_read_committed(&store.pool).await?;
        let inserted = sqlx::query_scalar!(
            r#"INSERT INTO run_command (id, run_id, kind, issued_by, issued_box)
               VALUES ($1, $2, 'cancel', $3, $4)
               ON CONFLICT (run_id) WHERE status = 'pending' AND kind = 'cancel' DO NOTHING
               RETURNING id AS "id: RunCommandId""#,
            RunCommandId::new().as_uuid(),
            run.as_uuid(),
            user.as_uuid(),
            box_id.as_uuid(),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx);
        let refused = match inserted {
            Ok(Some(id)) => {
                refuse_run_follow_ups(&mut *tx, run).await?;
                tx.commit().await.map_err(map_sqlx)?;
                return Ok(CancelRequest::Inserted(id));
            }
            Ok(None) => None,
            Err(err @ StoreError::Constraint(_)) => Some(err),
            Err(err) => return Err(err),
        };
        // Nothing was written; the re-read runs on the pool.
        tx.rollback().await.map_err(map_sqlx)?;

        let state = sqlx::query!(
            r#"SELECT EXISTS (SELECT 1 FROM run WHERE id = $1) AS "run_known!",
                      EXISTS (SELECT 1 FROM app_user WHERE id = $2) AS "user_known!",
                      EXISTS (SELECT 1 FROM box WHERE id = $3) AS "box_known!",
                      (SELECT id FROM run_command
                        WHERE run_id = $1 AND kind = 'cancel' AND status = 'pending')
                          AS "pending?: RunCommandId""#,
            run.as_uuid(),
            user.as_uuid(),
            box_id.as_uuid(),
        )
        .fetch_one(&store.pool)
        .await
        .map_err(map_sqlx)?;
        if !state.run_known {
            return Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            });
        }
        require_actor(
            (state.user_known, state.box_known),
            user,
            box_id,
            "run_command",
            "issued",
        )?;
        if let Some(err) = refused {
            return Err(err);
        }
        if let Some(pending) = state.pending {
            refuse_run_follow_ups(&store.pool, run).await?;
            return Ok(CancelRequest::AlreadyPending(pending));
        }
    }
    Err(StoreError::Constraint(format!(
        "run_command: run `{run}`'s pending cancel was resolved between the insert and the \
         re-read {CANCEL_ATTEMPTS} times; nothing was written"
    )))
}

/// MOD-70 D5, B-14: every `pending` follow-up of `run` moves to `refused` with
/// [`FOLLOW_UP_RUN_CANCELLED`], its text nulled. Answers how many moved.
async fn refuse_run_follow_ups<'e, E>(executor: E, run: RunId) -> Result<u64>
where
    E: sqlx::PgExecutor<'e>,
{
    let moved = sqlx::query!(
        "UPDATE run_command \
            SET status = 'refused', resolution = $2, text = NULL, resolved_at = clock_timestamp() \
          WHERE run_id = $1 AND kind = 'follow_up' AND status = 'pending'",
        run.as_uuid(),
        FOLLOW_UP_RUN_CANCELLED,
    )
    .execute(executor)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    Ok(moved)
}

/// [`WriteStore::pending_commands`](htui_core::store::WriteStore::pending_commands) (B-4):
/// **cancels only** (MOD-70 D5, I-9); a follow-up is read by the walk that owns its step.
pub(super) async fn pending_commands(
    store: &PgStore,
    owner: Uuid,
    box_id: BoxId,
) -> Result<Vec<RunCommand>> {
    sqlx::query_as!(
        RunCommand,
        r#"SELECT c.id         AS "id: RunCommandId",
                  c.run_id     AS "run_id: RunId",
                  c.kind       AS "kind: RunCommandKind",
                  c.issued_by  AS "issued_by: UserId",
                  c.issued_box AS "issued_box: BoxId",
                  c.status     AS "status: RunCommandStatus",
                  c.resolution,
                  c.issued_at,
                  c.resolved_at
             FROM run_command c JOIN run r ON r.id = c.run_id
            WHERE c.status = 'pending' AND c.kind = 'cancel'
              AND (r.lease_owner = $1
                   OR (r.executing_box_id = $2
                       AND (r.status IN ('done', 'failed', 'cancelled')
                            OR (r.status IN ('running', 'awaiting_approval')
                                AND (r.lease_owner IS NULL OR r.lease_expires_at IS NULL
                                     OR r.lease_expires_at <= clock_timestamp())))))
            ORDER BY c.issued_at, c.id"#,
        owner,
        box_id.as_uuid(),
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)
}

/// [`WriteStore::resolve_command`](htui_core::store::WriteStore::resolve_command): always clears
/// `text` (MOD-70 D5, I-5).
pub(super) async fn resolve_command(
    store: &PgStore,
    id: RunCommandId,
    to: RunCommandStatus,
    resolution: Option<String>,
) -> Result<bool> {
    if to == RunCommandStatus::Pending {
        return Err(StoreError::Constraint(format!(
            "run_command `{id}` resolves to `applied` or `refused`, not `pending`"
        )));
    }
    let moved = sqlx::query!(
        "UPDATE run_command \
            SET status = $2, resolution = $3, text = NULL, resolved_at = clock_timestamp() \
          WHERE id = $1 AND status = 'pending'",
        id.as_uuid(),
        to.as_str(),
        resolution,
    )
    .execute(&store.pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    if moved == 1 {
        return Ok(true);
    }
    sqlx::query_scalar!("SELECT 1 FROM run_command WHERE id = $1", id.as_uuid())
        .fetch_optional(&store.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run_command",
            id: id.to_string(),
        })?;
    Ok(false)
}

/// [`WriteStore::relay_view`](htui_core::store::WriteStore::relay_view): three reads; a display
/// read, so three snapshots are harmless. MOD-70 D5, B-13: `follow_ups` is the newest follow-up
/// of each step of the item's non-terminal runs (the pending one, else the greatest
/// `(issued_at, id)`), in `(issued_at, id)` order, never its text (OQ-6).
pub(super) async fn relay_view(store: &PgStore, item: ItemId) -> Result<RelayView> {
    let permissions = sqlx::query_as!(
        PermissionRecord,
        r#"SELECT p.id           AS "id: PermissionId",
                  p.run_id       AS "run_id: RunId",
                  p.run_step_id  AS "run_step_id: StepId",
                  p.session      AS "session: RelaySessionId",
                  p.request_id,
                  p.tool_call_id,
                  p.summary,
                  p.options      AS "options: Json<Vec<RelayOption>>",
                  p.status       AS "status: PermissionStatus",
                  p.option_id,
                  p.answered_by  AS "answered_by: UserId",
                  p.answered_box AS "answered_box: BoxId",
                  p.created_at,
                  p.answered_at,
                  p.resolved_at
             FROM step_permission p JOIN run r ON r.id = p.run_id
            WHERE r.item_id = $1 AND p.status = 'pending'
              AND r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()
            ORDER BY p.created_at, p.id"#,
        item.as_uuid(),
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)?;

    let cancels = sqlx::query_scalar!(
        r#"SELECT DISTINCT c.run_id AS "run_id: RunId"
             FROM run_command c JOIN run r ON r.id = c.run_id
            WHERE r.item_id = $1 AND c.kind = 'cancel' AND c.status = 'pending'
              AND r.status NOT IN ('done', 'failed', 'cancelled')
            ORDER BY c.run_id"#,
        item.as_uuid(),
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)?;

    // The `!` overrides: sqlx cannot infer non-null through the subquery.
    let follow_ups = sqlx::query_as!(
        FollowUpView,
        r#"SELECT n.id AS "id!: RunCommandId", n.run_id AS "run_id!: RunId",
                  n.run_step_id AS "run_step_id!: StepId", n.status AS "status!: RunCommandStatus",
                  n.resolution, n.issued_at AS "issued_at!", n.resolved_at
             FROM (SELECT DISTINCT ON (c.run_step_id)
                          c.id, c.run_id, c.run_step_id, c.status, c.resolution, c.issued_at,
                          c.resolved_at
                     FROM run_command c JOIN run r ON r.id = c.run_id
                    WHERE r.item_id = $1 AND c.kind = 'follow_up'
                      AND r.status NOT IN ('done', 'failed', 'cancelled')
                    ORDER BY c.run_step_id, (c.status = 'pending') DESC, c.issued_at DESC,
                             c.id DESC) n
            ORDER BY n.issued_at, n.id"#,
        item.as_uuid(),
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)?;

    Ok(RelayView {
        permissions: permissions.into_iter().map(StepPermission::from).collect(),
        cancels,
        follow_ups,
    })
}

/// [`WriteStore::open_permissions`](htui_core::store::WriteStore::open_permissions) (MOD-69 plan
/// D4): `relay_view`'s open predicate over the scope's item runs, joined to the item and the step.
/// `r.item_id IS NOT NULL` is implied by the item join and kept explicit: plan D4's chat-run rule.
/// `step_fanned` is the Runs pane's slot rule, a sibling at the step's slot with a non-zero index
/// (MOD-69 review L1).
pub(super) async fn open_permissions(
    store: &PgStore,
    scope: &Scope,
) -> Result<Vec<WaitingPermission>> {
    if scope.is_empty() {
        return Ok(Vec::new());
    }
    let projects: Vec<Uuid> = scope.project_ids.iter().map(|id| id.as_uuid()).collect();
    let rows = sqlx::query!(
        r#"SELECT p.id           AS "id: PermissionId",
                  p.run_id       AS "run_id: RunId",
                  p.run_step_id  AS "run_step_id: StepId",
                  p.session      AS "session: RelaySessionId",
                  p.request_id,
                  p.tool_call_id,
                  p.summary,
                  p.options      AS "options: Json<Vec<RelayOption>>",
                  p.status       AS "status: PermissionStatus",
                  p.option_id,
                  p.answered_by  AS "answered_by: UserId",
                  p.answered_box AS "answered_box: BoxId",
                  p.created_at,
                  p.answered_at,
                  p.resolved_at,
                  i.id           AS "item_id: ItemId",
                  i.project_id   AS "project_id: ProjectId",
                  i.key          AS "item_key!",
                  i.key_prefix,
                  i.key_number,
                  r.queued_at    AS run_queued_at,
                  s.position     AS step_position,
                  s.attempt      AS step_attempt,
                  s.fanout_index AS step_fanout_index,
                  EXISTS (SELECT 1 FROM run_step f
                           WHERE f.run_id = s.run_id AND f.position = s.position
                             AND f.attempt = s.attempt AND f.fanout_index <> 0) AS "step_fanned!",
                  s.phase_name
             FROM step_permission p
             JOIN run r      ON r.id = p.run_id
             JOIN item i     ON i.id = r.item_id
             JOIN run_step s ON s.id = p.run_step_id
            WHERE i.project_id = ANY($1) AND r.item_id IS NOT NULL
              AND p.status = 'pending'
              AND r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()
            ORDER BY p.created_at, p.id"#,
        &projects[..],
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)?;

    let mut out: Vec<WaitingPermission> = rows
        .into_iter()
        .map(|row| WaitingPermission {
            item: row.item_id,
            project: row.project_id,
            item_key: row.item_key,
            key_prefix: row.key_prefix,
            key_number: row.key_number,
            run_queued_at: row.run_queued_at,
            step_position: row.step_position,
            step_attempt: row.step_attempt,
            step_fanout_index: row.step_fanout_index,
            step_fanned: row.step_fanned,
            phase_name: row.phase_name,
            permission: StepPermission {
                id: row.id,
                run_id: row.run_id,
                run_step_id: row.run_step_id,
                session: row.session,
                request_id: row.request_id,
                tool_call_id: row.tool_call_id,
                summary: row.summary,
                options: row.options.0,
                status: row.status,
                option_id: row.option_id,
                answered_by: row.answered_by,
                answered_box: row.answered_box,
                created_at: row.created_at,
                answered_at: row.answered_at,
                resolved_at: row.resolved_at,
            },
        })
        .collect();
    WaitingPermission::sort_canonical(&mut out);
    Ok(out)
}

// ---- MOD-70 (plan D1-D5, D9; blueprint §2.7): follow-ups for engine steps. -----------------------

/// [`WriteStore::request_follow_up`](htui_core::store::WriteStore::request_follow_up): D3's one
/// `INSERT … SELECT … FOR SHARE OF w`, then on a miss one re-read deciding, in `MemStore`'s order:
/// `NotFound { run_step }`, the actor (`Constraint`), `ChatRun`, `Judge`, `NotRunning`,
/// `Cancelling`, `AlreadyQueued`, `ExecutorGone` (B-4), `NotStarted`, `SessionEnded`. An insert's
/// own `Constraint` (an unknown actor, a repeated id) is held until the re-read has checked the
/// actor and the guards. When every guard passes on the re-read the colliding row resolved in
/// between: retried, [`FOLLOW_UP_ATTEMPTS`] times in all.
pub(super) async fn request_follow_up(
    store: &PgStore,
    new: NewFollowUp,
) -> Result<FollowUpRequest> {
    let step = new.run_step_id;
    let text = new.text.into_string();
    for _ in 0..FOLLOW_UP_ATTEMPTS {
        // The casts: the parameters sit in a SELECT list, where Postgres cannot infer their type
        // from the INSERT's columns.
        let inserted = sqlx::query_scalar!(
            r#"INSERT INTO run_command (id, run_id, kind, run_step_id, text, issued_by, issued_box)
               SELECT $1::uuid, s.run_id, 'follow_up', s.id, $3::text, $4::uuid, $5::uuid
                 FROM follow_up_window w
                 JOIN run_step s ON s.id = w.run_step_id
                 JOIN run r      ON r.id = s.run_id
                WHERE w.run_step_id = $2
                  AND w.closed_at IS NULL
                  AND r.kind = 'graph'
                  AND s.fanout_index >= 0
                  AND s.status = 'running'
                  AND r.lease_owner = w.owner
                  AND r.lease_expires_at > clock_timestamp()
                  AND NOT EXISTS (SELECT 1 FROM run_command c
                                   WHERE c.run_id = s.run_id AND c.kind = 'cancel'
                                     AND c.status = 'pending')
                  FOR SHARE OF w
               ON CONFLICT (run_step_id) WHERE status = 'pending' AND kind = 'follow_up' DO NOTHING
               RETURNING id AS "id: RunCommandId""#,
            new.id.as_uuid(),
            step.as_uuid(),
            text,
            new.issued_by.as_uuid(),
            new.issued_box.as_uuid(),
        )
        .fetch_optional(&store.pool)
        .await
        .map_err(map_sqlx);
        let held = match inserted {
            Ok(Some(id)) => return Ok(FollowUpRequest::Queued(id)),
            Ok(None) => None,
            Err(err @ StoreError::Constraint(_)) => Some(err),
            Err(err) => return Err(err),
        };

        let state = sqlx::query!(
            r#"SELECT r.kind                AS "run_kind: RunKind",
                      s.fanout_index,
                      s.status              AS "step_status: StepStatus",
                      EXISTS (SELECT 1 FROM app_user WHERE id = $2) AS "user_known!",
                      EXISTS (SELECT 1 FROM box WHERE id = $3)      AS "box_known!",
                      EXISTS (SELECT 1 FROM run_command c
                               WHERE c.run_id = s.run_id AND c.kind = 'cancel'
                                 AND c.status = 'pending')          AS "cancelling!",
                      EXISTS (SELECT 1 FROM run_command c
                               WHERE c.run_step_id = s.id AND c.kind = 'follow_up'
                                 AND c.status = 'pending')          AS "queued!",
                      w.run_step_id IS NOT NULL                     AS "window!",
                      w.closed_at IS NOT NULL                       AS "window_closed!",
                      w.owner                                       AS "window_owner?",
                      r.lease_owner                                 AS "lease_owner?",
                      (r.lease_expires_at IS NOT NULL
                       AND r.lease_expires_at > clock_timestamp())  AS "lease_live!"
                 FROM run_step s
                 JOIN run r ON r.id = s.run_id
                 LEFT JOIN follow_up_window w ON w.run_step_id = s.id
                WHERE s.id = $1"#,
            step.as_uuid(),
            new.issued_by.as_uuid(),
            new.issued_box.as_uuid(),
        )
        .fetch_optional(&store.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: step.to_string(),
        })?;
        require_actor(
            (state.user_known, state.box_known),
            new.issued_by,
            new.issued_box,
            "run_command",
            "issued",
        )?;
        // B-4: the lease must be live under the window's owner; with no window, under anyone.
        let executor_live = state.lease_live
            && state.lease_owner.is_some()
            && (state.window_owner.is_none() || state.lease_owner == state.window_owner);
        let refusal = if state.run_kind == RunKind::Chat {
            Some(FollowUpRefusal::ChatRun)
        } else if state.fanout_index < 0 {
            Some(FollowUpRefusal::Judge)
        } else if state.step_status != StepStatus::Running {
            Some(FollowUpRefusal::NotRunning)
        } else if state.cancelling {
            Some(FollowUpRefusal::Cancelling)
        } else if state.queued {
            Some(FollowUpRefusal::AlreadyQueued)
        } else if !executor_live {
            Some(FollowUpRefusal::ExecutorGone)
        } else if !state.window {
            Some(FollowUpRefusal::NotStarted)
        } else if state.window_closed {
            Some(FollowUpRefusal::SessionEnded)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            return Ok(FollowUpRequest::Refused(refusal));
        }
        // Every guard passes now: a held insert error is a repeated id (MemStore checks the id
        // last too); otherwise the colliding row resolved, or a close was rolled back: retry.
        if let Some(err) = held {
            return Err(err);
        }
    }
    Err(StoreError::Constraint(format!(
        "run_command: step `{step}`'s follow-up guards changed between the insert and the \
         re-read {FOLLOW_UP_ATTEMPTS} times; nothing was written"
    )))
}

/// [`WriteStore::open_follow_ups`](htui_core::store::WriteStore::open_follow_ups) (D4, B-3, B-6):
/// one transaction, in this order: the fence ([`lock_step_fence`], owner only), the window's
/// upsert (which takes its row lock: an enqueue already holding `FOR SHARE` commits first, a later
/// one waits and re-checks), then the refusal of every follow-up still pending on the step, whose
/// fresh snapshot sees every row committed before the lock.
pub(super) async fn open_follow_ups(
    store: &PgStore,
    run: RunId,
    step: StepId,
    session: RelaySessionId,
    owner: Uuid,
) -> Result<bool> {
    // F-18: the refusal after the upsert reads a fresh snapshot.
    let mut tx = begin_read_committed(&store.pool).await?;
    let fence = lock_step_fence(&mut tx, step)
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "run_step",
            id: step.to_string(),
        })?;
    if fence.run_id != run.as_uuid() {
        return Err(StoreError::Constraint(format!(
            "follow_up_window.run_step_id `{step}` is not a step of run `{run}`"
        )));
    }
    // B-3: the owner alone, as every other executor-side write; nothing is written.
    if fence.lease_owner != Some(owner) {
        tx.rollback().await.map_err(map_sqlx)?;
        return Ok(false);
    }

    sqlx::query!(
        "INSERT INTO follow_up_window (run_step_id, run_id, session, owner) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (run_step_id) DO UPDATE \
            SET session = EXCLUDED.session, owner = EXCLUDED.owner, \
                opened_at = clock_timestamp(), closed_at = NULL",
        step.as_uuid(),
        run.as_uuid(),
        session.as_uuid(),
        owner,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    sqlx::query!(
        "UPDATE run_command \
            SET status = 'refused', resolution = $2, text = NULL, resolved_at = clock_timestamp() \
          WHERE run_step_id = $1 AND kind = 'follow_up' AND status = 'pending'",
        step.as_uuid(),
        FOLLOW_UP_SESSION_ENDED,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    tx.commit().await.map_err(map_sqlx)?;
    Ok(true)
}

/// [`WriteStore::next_follow_up`](htui_core::store::WriteStore::next_follow_up): the step's
/// pending follow-up while its window is `session`'s and open. A read: never fenced.
pub(super) async fn next_follow_up(
    store: &PgStore,
    step: StepId,
    session: RelaySessionId,
) -> Result<Option<QueuedFollowUp>> {
    let row = sqlx::query!(
        r#"SELECT c.id AS "id: RunCommandId", c.text AS "text!"
             FROM run_command c
             JOIN follow_up_window w ON w.run_step_id = c.run_step_id
            WHERE c.run_step_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending'
              AND w.session = $2 AND w.closed_at IS NULL"#,
        step.as_uuid(),
        session.as_uuid(),
    )
    .fetch_optional(&store.pool)
    .await
    .map_err(map_sqlx)?;
    Ok(row.map(|row| QueuedFollowUp {
        id: row.id,
        text: row.text,
    }))
}

/// [`WriteStore::settle_follow_up`](htui_core::store::WriteStore::settle_follow_up) (D4, B-3,
/// B-19): one compare-and-set fenced on the owner only, the text always nulled; a miss is told
/// apart by one re-read: `NotFound` → `Constraint` (a cancel) → `NotPending` → `Fenced`.
pub(super) async fn settle_follow_up(
    store: &PgStore,
    id: RunCommandId,
    owner: Uuid,
    to: FollowUpSettle,
) -> Result<SettleOutcome> {
    let (status, resolution) = match to {
        FollowUpSettle::Applied => (RunCommandStatus::Applied, None),
        FollowUpSettle::Refused(sentence) => (RunCommandStatus::Refused, Some(sentence)),
    };
    let moved = sqlx::query!(
        "UPDATE run_command c \
            SET status = $3, resolution = $4, text = NULL, resolved_at = clock_timestamp() \
           FROM run r \
          WHERE c.id = $1 AND c.kind = 'follow_up' AND c.status = 'pending' \
            AND r.id = c.run_id AND r.lease_owner = $2",
        id.as_uuid(),
        owner,
        status.as_str(),
        resolution,
    )
    .execute(&store.pool)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    if moved == 1 {
        return Ok(SettleOutcome::Settled);
    }

    let row = sqlx::query!(
        r#"SELECT kind AS "kind: RunCommandKind", status AS "status: RunCommandStatus"
             FROM run_command WHERE id = $1"#,
        id.as_uuid(),
    )
    .fetch_optional(&store.pool)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound {
        entity: "run_command",
        id: id.to_string(),
    })?;
    if row.kind != RunCommandKind::FollowUp {
        return Err(StoreError::Constraint(format!(
            "run_command `{id}` is a `{}`, not a `follow_up`",
            row.kind
        )));
    }
    if row.status != RunCommandStatus::Pending {
        return Ok(SettleOutcome::NotPending);
    }
    Ok(SettleOutcome::Fenced)
}

/// [`WriteStore::close_follow_ups`](htui_core::store::WriteStore::close_follow_ups) (D4, D6 step
/// 8, B-5): two statements in one `READ COMMITTED` transaction, never one CTE (F-18). The first
/// closes the window if it is `session`'s, waiting on an enqueue that holds its row; the second,
/// on a fresh snapshot that sees that enqueue, refuses the step's pending follow-ups, but only
/// while the window is this session's (a superseded walk's close refuses nothing). Answers the
/// second's count.
pub(super) async fn close_follow_ups(
    store: &PgStore,
    step: StepId,
    session: RelaySessionId,
    reason: &str,
) -> Result<u64> {
    // F-18: the refusal reads a snapshot fresher than the close's wait.
    let mut tx = begin_read_committed(&store.pool).await?;
    sqlx::query!(
        "UPDATE follow_up_window SET closed_at = clock_timestamp() \
          WHERE run_step_id = $1 AND session = $2 AND closed_at IS NULL",
        step.as_uuid(),
        session.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let refused = sqlx::query!(
        "UPDATE run_command c \
            SET status = 'refused', resolution = $3, text = NULL, resolved_at = clock_timestamp() \
          WHERE c.run_step_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending' \
            AND EXISTS (SELECT 1 FROM follow_up_window w \
                         WHERE w.run_step_id = $1 AND w.session = $2)",
        step.as_uuid(),
        session.as_uuid(),
        reason,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    tx.commit().await.map_err(map_sqlx)?;
    Ok(refused)
}

/// [`WriteStore::close_dropped_follow_ups`](htui_core::store::WriteStore::close_dropped_follow_ups)
/// (D9, B-3, F-22): [`close_follow_ups`]' transaction shape over the whole run, both statements
/// fenced on `owner` being the run's lease owner; otherwise nothing matches and it answers 0.
pub(super) async fn close_dropped_follow_ups(
    store: &PgStore,
    run: RunId,
    owner: Uuid,
    reason: &str,
) -> Result<u64> {
    // F-18: the refusal reads a snapshot fresher than the close's wait.
    let mut tx = begin_read_committed(&store.pool).await?;
    sqlx::query!(
        "UPDATE follow_up_window w SET closed_at = clock_timestamp() \
           FROM run r \
          WHERE w.run_id = $1 AND w.closed_at IS NULL AND r.id = w.run_id AND r.lease_owner = $2",
        run.as_uuid(),
        owner,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let refused = sqlx::query!(
        "UPDATE run_command c \
            SET status = 'refused', resolution = $3, text = NULL, resolved_at = clock_timestamp() \
           FROM run r \
          WHERE c.run_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending' \
            AND r.id = c.run_id AND r.lease_owner = $2",
        run.as_uuid(),
        owner,
        reason,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?
    .rows_affected();
    tx.commit().await.map_err(map_sqlx)?;
    Ok(refused)
}
