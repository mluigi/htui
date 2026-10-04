//! The permission and control relay on Postgres (MOD-42 plan D1-D5, D12-D14; blueprint §2.10;
//! `0011_permission_relay.sql`).
//!
//! `impl WriteStore for PgStore` (`write.rs`) delegates its nine relay methods here. Every status
//! move is one compare-and-set statement (I-3): under `READ COMMITTED` the loser of a race blocks
//! on the row lock, re-evaluates its `status` predicate against the committed row and matches
//! nothing. Every miss is told apart by one re-read, the `take_lease` shape. Every instant is
//! `clock_timestamp()`, never a box clock (I-4). The only transaction is `open_permission`'s,
//! which locks the run `FOR SHARE` so an adoption cannot commit between its fence and its insert
//! (MOD-41's `step_fence` shape).
//!
//! An answer or a cancel request names an actor (`answered_by`/`answered_box`,
//! `issued_by`/`issued_box`, blueprint B-7). `MemStore` checks the actor **before** the row's
//! status, so the re-read of a miss checks it first too: an unknown user or box is a
//! [`StoreError::Constraint`] whether or not the compare-and-set would have matched, never
//! `Refused(Answered)` or `AlreadyPending`.

use chrono::{DateTime, Utc};
use htui_core::model::{
    AnswerOutcome, AnswerRefusal, BoxId, CancelRequest, ItemId, OpenPermission, PermissionChoice,
    PermissionId, PermissionStatus, RelayOption, RelaySessionId, RelayView, RunCommand,
    RunCommandId, RunCommandKind, RunCommandStatus, RunId, Scope, StepId, StepPermission, UserId,
    WaitingPermission,
};
use htui_core::store::{Result, StoreError, references_no_row};
use sqlx::types::Json;
use uuid::Uuid;

use crate::error::map_sqlx;
use crate::pg::PgStore;

/// How many times [`request_cancel`] retries a pending row that was resolved between its insert
/// and its re-read before it gives up.
const CANCEL_ATTEMPTS: usize = 3;

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

    // The run is row-locked so an adoption cannot commit between this check and the insert.
    let fence = sqlx::query!(
        r#"SELECT s.run_id, r.lease_owner AS "lease_owner?"
             FROM run_step s JOIN run r ON r.id = s.run_id
            WHERE s.id = $1
              FOR SHARE OF r"#,
        open.run_step_id.as_uuid(),
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?
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
pub(super) async fn request_cancel(
    store: &PgStore,
    run: RunId,
    user: UserId,
    box_id: BoxId,
) -> Result<CancelRequest> {
    for _ in 0..CANCEL_ATTEMPTS {
        let inserted = sqlx::query_scalar!(
            r#"INSERT INTO run_command (id, run_id, kind, issued_by, issued_box)
               VALUES ($1, $2, 'cancel', $3, $4)
               ON CONFLICT (run_id, kind) WHERE status = 'pending' DO NOTHING
               RETURNING id AS "id: RunCommandId""#,
            RunCommandId::new().as_uuid(),
            run.as_uuid(),
            user.as_uuid(),
            box_id.as_uuid(),
        )
        .fetch_optional(&store.pool)
        .await
        .map_err(map_sqlx);
        let refused = match inserted {
            Ok(Some(id)) => return Ok(CancelRequest::Inserted(id)),
            Ok(None) => None,
            Err(err @ StoreError::Constraint(_)) => Some(err),
            Err(err) => return Err(err),
        };

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
            return Ok(CancelRequest::AlreadyPending(pending));
        }
    }
    Err(StoreError::Constraint(format!(
        "run_command: run `{run}`'s pending cancel was resolved between the insert and the \
         re-read {CANCEL_ATTEMPTS} times; nothing was written"
    )))
}

/// [`WriteStore::pending_commands`](htui_core::store::WriteStore::pending_commands) (B-4).
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
            WHERE c.status = 'pending'
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

/// [`WriteStore::resolve_command`](htui_core::store::WriteStore::resolve_command).
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
        "UPDATE run_command SET status = $2, resolution = $3, resolved_at = clock_timestamp() \
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

/// [`WriteStore::relay_view`](htui_core::store::WriteStore::relay_view): two reads; a display
/// read, so two snapshots are harmless.
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

    Ok(RelayView {
        permissions: permissions.into_iter().map(StepPermission::from).collect(),
        cancels,
    })
}

/// [`WriteStore::open_permissions`](htui_core::store::WriteStore::open_permissions) (MOD-69 plan
/// D4).
pub(super) async fn open_permissions(
    _store: &PgStore,
    _scope: &Scope,
) -> Result<Vec<WaitingPermission>> {
    todo!("MOD-69 T1")
}
