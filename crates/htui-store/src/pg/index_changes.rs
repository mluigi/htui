//! The index change read on Postgres (MOD-159 M2, plan D3, D4).
//!
//! The `IndexChangesStore` impl lives here, a child of `pg`, as `indexer_lease.rs`'s does, so it
//! reads the pool directly. The predicates are the cache cursor's (`cache/refresh.rs`): a row
//! counts when its stamp is strictly after `since`, and `since` is the server's own clock
//! (`clock_timestamp()`), never the caller's. Item and requirement stamps are the server's
//! (`set_updated_at`); a document's `created_at` is its writer's, so a writer whose clock lags the
//! server's by more than the caller's overlap is left to the full reconcile.
//!
//! One statement of four arms joined by `UNION`, not four round trips: an item edited and given a
//! document version in the same window comes back once, and every arm reads the same snapshot.
//! An item document is listed under the item's project (it joins `item`), which
//! `fk_document_item_project` pins to the document's own anyway.

use chrono::{DateTime, Utc};
use htui_core::model::{
    DocumentId, IndexChanges, ItemId, ProjectChanges, ProjectId, RequirementId,
};
use htui_core::store::{IndexChangesStore, Result, StoreError};
use uuid::Uuid;

use super::PgStore;
use crate::error::map_sqlx;

impl IndexChangesStore for PgStore {
    async fn index_clock(&self) -> Result<DateTime<Utc>> {
        sqlx::query_scalar!(r#"SELECT clock_timestamp() AS "now!""#)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    async fn index_changes(
        &self,
        projects: &[ProjectId],
        since: DateTime<Utc>,
    ) -> Result<IndexChanges> {
        if projects.is_empty() {
            return Ok(IndexChanges::default());
        }
        let asked: Vec<Uuid> = projects.iter().map(|id| id.as_uuid()).collect();
        let rows = sqlx::query!(
            r#"
            SELECT 'item' AS "kind!", i.project_id AS "project_id!", i.id AS "id!"
              FROM item i WHERE i.project_id = ANY($1) AND i.updated_at > $2
            UNION
            SELECT 'item', i.project_id, i.id
              FROM document d JOIN item i ON i.id = d.item_id
             WHERE i.project_id = ANY($1) AND d.created_at > $2
            UNION
            SELECT 'requirement', r.project_id, r.id
              FROM requirement r WHERE r.project_id = ANY($1) AND r.updated_at > $2
            UNION
            SELECT 'owner_document', d.project_id, d.id
              FROM document d WHERE d.item_id IS NULL AND d.project_id = ANY($1) AND d.created_at > $2
            "#,
            &asked,
            since,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut changes = IndexChanges::default();
        for row in rows {
            let entry: &mut ProjectChanges = changes
                .projects
                .entry(ProjectId::from_uuid(row.project_id))
                .or_default();
            match row.kind.as_str() {
                "item" => {
                    entry.items.insert(ItemId::from_uuid(row.id));
                }
                "requirement" => {
                    entry.requirements.insert(RequirementId::from_uuid(row.id));
                }
                "owner_document" => {
                    entry.owner_documents.insert(DocumentId::from_uuid(row.id));
                }
                other => {
                    return Err(StoreError::Backend(format!(
                        "index change read answered an unknown kind `{other}`"
                    )));
                }
            }
        }
        Ok(changes)
    }
}
