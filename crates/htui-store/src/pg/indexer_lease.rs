//! Indexer leases on Postgres (MOD-159 M1, plan D2, D3; `0038_indexer_lease.sql`).
//!
//! The `IndexerLeaseStore` impl lives here, a child of `pg`, as `worker_log.rs`'s does, so it
//! reads the pool directly. The SQL is the run lease's (`write.rs`'s `take_lease`,
//! `refresh_lease`, `release_lease`; a release of one collection is the release of all narrowed
//! to its row): every expiry is `clock_timestamp()` plus the TTL in
//! microseconds, judged by the same clock, and a compare-and-set that touches no row means "not
//! ours". The owner token is written and compared, never selected.
//!
//! A pass request (MOD-159 M3 plan D1, D2) is `pass_requested_at`, stamped by `clock_timestamp()`
//! under a live holder and cleared by the holder's claim. The take's upsert and both releases
//! never name the column, so they keep a pending request.

use chrono::TimeDelta;
use htui_core::model::{BoxId, IndexedCollection, IndexerLease, LeaseTake};
use htui_core::store::{IndexerLeaseStore, Result, StoreError, lease_ttl_micros};
use uuid::Uuid;

use super::PgStore;
use crate::error::map_sqlx;

/// How often [`PgStore::take_indexer_lease`] tries when its take was refused and the follow-up
/// read then found no live holder (it released or lapsed in between); after the last try the
/// answer is `Held` with what that read saw.
const TAKE_ATTEMPTS: usize = 3;

impl IndexerLeaseStore for PgStore {
    async fn take_indexer_lease(
        &self,
        collection: &IndexedCollection,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<LeaseTake> {
        let ttl = lease_ttl_micros(ttl)?;
        let name = collection.name();
        let mut attempt = 0;
        loop {
            attempt += 1;
            // `taken` writes only when the lease is free, lapsed or already `owner`'s, and creates
            // the row on a first take; a concurrent take's row is waited for and judged as
            // committed. `prior` reads the owner from the statement's snapshot, before `taken`:
            // it only tells `Ours` from `Taken`. No `FOR UPDATE` there: a row this statement's
            // own upsert modifies would then be skipped, and every take would read `Taken`.
            let answer = sqlx::query!(
                r#"
                WITH prior AS (
                    SELECT lease_owner FROM indexer_lease WHERE collection = $1
                ), taken AS (
                    INSERT INTO indexer_lease (collection, lease_owner, lease_box_id,
                                               lease_expires_at)
                    VALUES ($1, $3, $2, clock_timestamp() + $4::bigint * interval '1 microsecond')
                    ON CONFLICT (collection) DO UPDATE
                       SET lease_owner      = EXCLUDED.lease_owner,
                           lease_box_id     = EXCLUDED.lease_box_id,
                           lease_expires_at = EXCLUDED.lease_expires_at
                     WHERE indexer_lease.lease_owner IS NULL
                        OR indexer_lease.lease_owner = EXCLUDED.lease_owner
                        OR indexer_lease.lease_expires_at IS NULL
                        OR indexer_lease.lease_expires_at <= clock_timestamp()
                    RETURNING collection
                )
                SELECT EXISTS (SELECT 1 FROM taken) AS "taken!",
                       EXISTS (SELECT 1 FROM prior WHERE lease_owner = $3) AS "ours!"
                "#,
                name,
                box_id.as_uuid(),
                owner,
                ttl,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

            if answer.taken {
                return Ok(if answer.ours {
                    LeaseTake::Ours
                } else {
                    LeaseTake::Taken
                });
            }
            match self.indexer_lease(collection).await? {
                Some(holder) if holder.live || attempt >= TAKE_ATTEMPTS => {
                    return Ok(LeaseTake::Held(holder));
                }
                // No product path deletes a row; only a hand-made delete gets here.
                None if attempt >= TAKE_ATTEMPTS => {
                    return Err(StoreError::NotFound {
                        entity: "indexer_lease",
                        id: name,
                    });
                }
                // The holder let go between the take and the read: take again.
                _ => {}
            }
        }
    }

    async fn renew_indexer_lease(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool> {
        let ttl = lease_ttl_micros(ttl)?;
        let moved = sqlx::query!(
            "UPDATE indexer_lease \
                SET lease_expires_at = clock_timestamp() + $3::bigint * interval '1 microsecond' \
              WHERE collection = $1 AND lease_owner = $2",
            collection.name(),
            owner,
            ttl,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(moved == 1)
    }

    async fn release_indexer_leases(&self, owner: Uuid) -> Result<u64> {
        let freed = sqlx::query!(
            "UPDATE indexer_lease SET lease_owner = NULL, lease_expires_at = clock_timestamp() \
              WHERE lease_owner = $1",
            owner,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(freed)
    }

    async fn release_indexer_lease(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
    ) -> Result<bool> {
        let freed = sqlx::query!(
            "UPDATE indexer_lease SET lease_owner = NULL, lease_expires_at = clock_timestamp() \
              WHERE collection = $1 AND lease_owner = $2",
            collection.name(),
            owner,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(freed == 1)
    }

    async fn indexer_lease(&self, collection: &IndexedCollection) -> Result<Option<IndexerLease>> {
        let row = sqlx::query!(
            r#"
            SELECT l.collection,
                   l.lease_box_id,
                   b.hostname AS "box_name?",
                   l.lease_expires_at,
                   COALESCE(l.lease_owner IS NOT NULL
                            AND l.lease_expires_at > clock_timestamp(), false) AS "live!"
              FROM indexer_lease l
              LEFT JOIN box b ON b.id = l.lease_box_id
             WHERE l.collection = $1
            "#,
            collection.name(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(|row| IndexerLease {
            collection: row.collection,
            box_id: row.lease_box_id.map(BoxId::from_uuid),
            box_name: row.box_name,
            expires_at: row.lease_expires_at,
            live: row.live,
        }))
    }

    async fn request_indexer_pass(
        &self,
        collection: &IndexedCollection,
    ) -> Result<Option<IndexerLease>> {
        // One statement: the stamp and the holder it answers are the same row version, so a
        // holder that let go concurrently is either asked (and keeps the request) or not stamped.
        let row = sqlx::query!(
            r#"
            WITH asked AS (
                UPDATE indexer_lease SET pass_requested_at = clock_timestamp()
                 WHERE collection = $1 AND lease_owner IS NOT NULL
                   AND lease_expires_at > clock_timestamp()
                RETURNING collection, lease_box_id, lease_expires_at)
            SELECT a.collection AS "collection!", a.lease_box_id, b.hostname AS "box_name?",
                   a.lease_expires_at
              FROM asked a LEFT JOIN box b ON b.id = a.lease_box_id
            "#,
            collection.name(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(|row| IndexerLease {
            collection: row.collection,
            box_id: row.lease_box_id.map(BoxId::from_uuid),
            box_name: row.box_name,
            expires_at: row.lease_expires_at,
            live: true,
        }))
    }

    async fn claim_indexer_pass(
        &self,
        collection: &IndexedCollection,
        owner: Uuid,
    ) -> Result<bool> {
        let claimed = sqlx::query!(
            "UPDATE indexer_lease SET pass_requested_at = NULL \
              WHERE collection = $1 AND lease_owner = $2 AND pass_requested_at IS NOT NULL",
            collection.name(),
            owner,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        Ok(claimed == 1)
    }
}
