//! The activity feed on Postgres (MOD-122 milestone 3, plan D1, D4, D6; blueprint F3;
//! `0034_activity_event.sql`).
//!
//! The `ActivityStore` impl lives here, a child of `pg`, so it reads the pool directly. Nothing
//! here writes an event: the deferred triggers of `0034_activity_event.sql` do, inside the
//! transition they record (plan D2). The reads are the user's own rows; `pinned` is derived from
//! each subject's current state (plan D4), newest event per subject (`DISTINCT ON (subject)`,
//! blueprint F3), over the partial index `idx_activity_event_needs_you`.

use chrono::{DateTime, Utc};
use htui_core::model::{
    ActivityEvent, ActivityKind, ActivityPage, ItemId, PINNED_LIMIT, PermissionId, ProjectId,
    RunId, StepId, WorkspaceId,
};
use htui_core::store::{ActivityStore, Result};

use super::PgStore;
use crate::error::map_sqlx;

impl ActivityStore for PgStore {
    async fn activity(&self, after: Option<i64>, limit: u32) -> Result<ActivityPage> {
        let me = self.this_user();
        let limit = i64::from(limit);
        let events = match after {
            Some(after) => {
                sqlx::query_as!(
                    ActivityEvent,
                    r#"
                    SELECT id            AS "id!",
                           at            AS "at!",
                           kind          AS "kind!: ActivityKind",
                           workspace_id  AS "workspace?: WorkspaceId",
                           project_id    AS "project?: ProjectId",
                           project_slug  AS "project_slug!",
                           item_id       AS "item?: ItemId",
                           item_key,
                           run_id        AS "run?: RunId",
                           chat          AS "chat!",
                           step_id       AS "step?: StepId",
                           step_name,
                           permission_id AS "permission?: PermissionId",
                           text          AS "text!"
                      FROM activity_event
                     WHERE user_id = $1 AND id > $2
                     ORDER BY id
                     LIMIT $3
                    "#,
                    me.as_uuid(),
                    after,
                    limit,
                )
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as!(
                    ActivityEvent,
                    r#"
                    SELECT id            AS "id!",
                           at            AS "at!",
                           kind          AS "kind!: ActivityKind",
                           workspace_id  AS "workspace?: WorkspaceId",
                           project_id    AS "project?: ProjectId",
                           project_slug  AS "project_slug!",
                           item_id       AS "item?: ItemId",
                           item_key,
                           run_id        AS "run?: RunId",
                           chat          AS "chat!",
                           step_id       AS "step?: StepId",
                           step_name,
                           permission_id AS "permission?: PermissionId",
                           text          AS "text!"
                      FROM (SELECT id, at, kind, workspace_id, project_id, project_slug, item_id,
                                   item_key, run_id, chat, step_id, step_name, permission_id, text
                              FROM activity_event
                             WHERE user_id = $1
                             ORDER BY id DESC
                             LIMIT $2) AS newest
                     ORDER BY id
                    "#,
                    me.as_uuid(),
                    limit,
                )
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(map_sqlx)?;

        // Plan D4: the newest needs-you event per subject, kept while its subject still waits.
        // The kind list is `idx_activity_event_needs_you`'s predicate, so the index serves it.
        let pinned = sqlx::query_as!(
            ActivityEvent,
            r#"
            SELECT id            AS "id!",
                   at            AS "at!",
                   kind          AS "kind!: ActivityKind",
                   workspace_id  AS "workspace?: WorkspaceId",
                   project_id    AS "project?: ProjectId",
                   project_slug  AS "project_slug!",
                   item_id       AS "item?: ItemId",
                   item_key,
                   run_id        AS "run?: RunId",
                   chat          AS "chat!",
                   step_id       AS "step?: StepId",
                   step_name,
                   permission_id AS "permission?: PermissionId",
                   text          AS "text!"
              FROM (SELECT DISTINCT ON (subject)
                           id, at, kind, workspace_id, project_id, project_slug, item_id,
                           item_key, run_id, chat, step_id, step_name, permission_id, text
                      FROM activity_event
                     WHERE user_id = $1
                       AND kind IN ('gate', 'judge_failed', 'selection', 'unblock',
                                    'interrupted', 'permission')
                     ORDER BY subject, id DESC) AS newest
             WHERE CASE kind
                     WHEN 'gate' THEN EXISTS (
                         SELECT 1 FROM run_step s
                          WHERE s.id = newest.step_id AND s.status = 'awaiting_approval')
                     WHEN 'permission' THEN EXISTS (
                         SELECT 1 FROM step_permission p
                          WHERE p.id = newest.permission_id AND p.status = 'pending')
                     WHEN 'unblock' THEN EXISTS (
                         SELECT 1 FROM item i
                          WHERE i.id = newest.item_id AND i.status = 'blocked')
                     ELSE EXISTS (
                         SELECT 1 FROM run r
                          WHERE r.id = newest.run_id AND r.status = 'awaiting_approval'
                            AND NOT EXISTS (SELECT 1 FROM run_step s
                                             WHERE s.run_id = r.id
                                               AND s.status = 'awaiting_approval'))
                   END
             ORDER BY id DESC
             LIMIT $2
            "#,
            me.as_uuid(),
            i64::from(PINNED_LIMIT),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(ActivityPage { events, pinned })
    }

    async fn prune_activity(&self, older_than: DateTime<Utc>, per_user_cap: u32) -> Result<u64> {
        let aged = sqlx::query!("DELETE FROM activity_event WHERE at < $1", older_than)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        let capped = sqlx::query!(
            r#"
            DELETE FROM activity_event
             WHERE id IN (SELECT id
                            FROM (SELECT id,
                                         row_number() OVER (PARTITION BY user_id ORDER BY id DESC)
                                             AS newest_first
                                    FROM activity_event) AS ranked
                           WHERE newest_first > $1)
            "#,
            i64::from(per_user_cap),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(aged + capped)
    }
}
