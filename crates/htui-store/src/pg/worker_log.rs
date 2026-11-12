//! Worker log lines on Postgres (MOD-122 milestone 2, plan D1, D2, D4, D6, D7; blueprint F3, F14;
//! `0031_worker_log.sql`).
//!
//! The `WorkerLogStore` impl lives here, a child of `pg`, so it reads the pool directly; nothing
//! in `write.rs` forwards to it. A batch is one transaction of `INSERT … SELECT … FROM UNNEST(…)`
//! statements, at most [`INSERT_CHUNK`] rows each: one statement text for every chunk, so one
//! offline entry. The box is joined in the statement, which is what stamps the user and the
//! hostname and what refuses another user's box (no row written, then `NotFound`).

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, LogShipLevel, NewWorkerLog, WorkerLogLevel, WorkerLogLine};
use htui_core::store::traits::BOX_SETTINGS_NOT_AN_OBJECT;
use htui_core::store::{Result, StoreError, WorkerLogStore};

use super::PgStore;
use crate::error::map_sqlx;

/// The most rows one `INSERT` writes (plan D2): a larger batch loops, in one transaction.
const INSERT_CHUNK: usize = 500;

/// How often [`PgStore::set_box_log_level`] retries when its `UPDATE` missed a row whose blob the
/// re-read then finds an object (another writer replaced the blob in between).
const LEVEL_ATTEMPTS: usize = 3;

impl WorkerLogStore for PgStore {
    async fn append_worker_logs(&self, box_id: BoxId, lines: &[NewWorkerLog]) -> Result<u64> {
        if lines.is_empty() {
            return Ok(0);
        }
        let me = self.this_user();
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let mut written = 0;
        for chunk in lines.chunks(INSERT_CHUNK) {
            let logged: Vec<DateTime<Utc>> = chunk.iter().map(|line| line.logged_at).collect();
            let levels: Vec<String> = chunk
                .iter()
                .map(|line| line.level.as_str().to_owned())
                .collect();
            let targets: Vec<String> = chunk
                .iter()
                .map(|line| line.stored_target().into_owned())
                .collect();
            let texts: Vec<String> = chunk
                .iter()
                .map(|line| line.stored_text().into_owned())
                .collect();
            let inserted = sqlx::query!(
                r#"
                INSERT INTO worker_log (user_id, box_id, box_name, logged_at, level, target, text)
                SELECT b.user_id, b.id, b.hostname, t.logged_at, t.level, t.target, t.text
                  FROM box b
                 CROSS JOIN UNNEST($3::timestamptz[], $4::text[], $5::text[], $6::text[])
                       WITH ORDINALITY AS t(logged_at, level, target, text, n)
                 WHERE b.id = $1 AND b.user_id = $2
                 ORDER BY t.n
                "#,
                box_id.as_uuid(),
                me.as_uuid(),
                &logged,
                &levels,
                &targets,
                &texts,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
            if inserted == 0 {
                // The chunk is not empty, so the join found no box of mine. The transaction
                // rolls back on drop: the batch is written whole or not at all.
                return Err(StoreError::NotFound {
                    entity: "box",
                    id: box_id.to_string(),
                });
            }
            written += inserted;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(written)
    }

    async fn worker_logs(&self, after: Option<i64>, limit: u32) -> Result<Vec<WorkerLogLine>> {
        let me = self.this_user();
        let limit = i64::from(limit);
        let rows = match after {
            Some(after) => {
                sqlx::query_as!(
                    WorkerLogLine,
                    r#"
                    SELECT id          AS "id!",
                           box_id      AS "box_id?: BoxId",
                           box_name    AS "box_name!",
                           logged_at   AS "logged_at!",
                           received_at AS "received_at!",
                           level       AS "level!: WorkerLogLevel",
                           target      AS "target!",
                           text        AS "text!"
                      FROM worker_log
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
                    WorkerLogLine,
                    r#"
                    SELECT id          AS "id!",
                           box_id      AS "box_id?: BoxId",
                           box_name    AS "box_name!",
                           logged_at   AS "logged_at!",
                           received_at AS "received_at!",
                           level       AS "level!: WorkerLogLevel",
                           target      AS "target!",
                           text        AS "text!"
                      FROM (SELECT id, box_id, box_name, logged_at, received_at, level, target, text
                              FROM worker_log
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
        };
        rows.map_err(map_sqlx)
    }

    async fn prune_worker_logs(&self, older_than: DateTime<Utc>, per_box_cap: u32) -> Result<u64> {
        let aged = sqlx::query!("DELETE FROM worker_log WHERE received_at < $1", older_than)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        // `box_id IS NOT NULL`: a removed box's error rows are not one capped partition; they
        // stay until the age cut (plan D6, blueprint F14).
        let capped = sqlx::query!(
            r#"
            DELETE FROM worker_log
             WHERE id IN (SELECT id
                            FROM (SELECT id,
                                         row_number() OVER (PARTITION BY box_id ORDER BY id DESC)
                                             AS newest_first
                                    FROM worker_log
                                   WHERE box_id IS NOT NULL) AS ranked
                           WHERE newest_first > $1)
            "#,
            i64::from(per_box_cap),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(aged + capped)
    }

    async fn set_box_log_level(&self, box_id: BoxId, level: Option<LogShipLevel>) -> Result<bool> {
        let me = self.this_user();
        for _ in 0..LEVEL_ATTEMPTS {
            // `jsonb_set` errors (`22023`) on a blob that is not an object, so the `WHERE` refuses
            // one instead and the re-read below names it: `edit_box`'s shape (blueprint F3). No
            // `edit_version` move: a level is last write wins (plan D7).
            let hit = sqlx::query!(
                r#"
                UPDATE box
                   SET settings = CASE WHEN $2::text IS NULL THEN settings - 'log_level'
                                       ELSE jsonb_set(settings, '{log_level}', to_jsonb($2::text))
                                  END
                 WHERE id = $1 AND user_id = $3 AND jsonb_typeof(settings) = 'object'
                "#,
                box_id.as_uuid(),
                level.map(LogShipLevel::as_str),
                me.as_uuid(),
            )
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
            if hit > 0 {
                return Ok(true);
            }
            match self.box_row(box_id).await?.filter(|row| row.user_id == me) {
                None => return Ok(false),
                Some(row) if !row.settings.is_object() => {
                    return Err(StoreError::Constraint(
                        BOX_SETTINGS_NOT_AN_OBJECT.to_owned(),
                    ));
                }
                // An object again since the `UPDATE`: another writer replaced the blob.
                Some(_) => {}
            }
        }
        Err(StoreError::Backend(format!(
            "box `{box_id}`'s settings kept changing while its log level was written"
        )))
    }
}
