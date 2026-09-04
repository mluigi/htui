//! `impl WriteStore for PgStore` (blueprint C.9): the three write paths of ANA-9 §4.1 and §4.2.
//!
//! Each one is a **single statement**, so atomicity comes from Postgres rather than from an
//! explicit transaction: a `WITH ... RETURNING` CTE either lands whole or not at all, and under
//! `READ COMMITTED` the loser of a compare-and-set race blocks on the row lock, re-evaluates
//! `version = $2` against the committed value and matches nothing (ANA-9 §11.3).
//!
//! There is **no `DELETE FROM item`** in this file and none may be added: §4.1's "keys are never
//! reused" is enforced by the absence of the path, and the conformance case `no_delete_path` is
//! what pins it. Nor does any statement write `updated_at` - the `BEFORE UPDATE` trigger of the
//! migration owns it, and `RETURNING` sees the trigger-modified row.

use htui_core::model::{Item, ItemId, ItemPatch, ItemRevision, NewItem, Status};
use htui_core::store::{ReadStore as _, Result, StoreError, UpdateOutcome, WriteStore};

use crate::error::map_sqlx;
use crate::pg::PgStore;

/// The refusal text for a kind that is unknown or belongs to another project.
///
/// ANA-9 §5.5 has no composite `(project_id, id)` key on `item_kind`, so this rule is an explicit
/// guard rather than a foreign key, on both write paths (blueprint H.13).
fn kind_not_in_project(kind: impl core::fmt::Display, project: impl core::fmt::Display) -> String {
    format!("item_kind `{kind}` does not exist in project `{project}`")
}

impl WriteStore for PgStore {
    /// Mints an item: counter upsert, key assembly and revision 1 in one statement (ANA-9 §7.1).
    ///
    /// The kind-belongs-to-project guard is folded into the counter CTE's `SELECT`, so an invalid
    /// kind makes `c` empty and nothing is inserted anywhere - no key number is burned by a mint
    /// that never lands (`mint_unknown_kind_rejected`, `nil_author_rejected`). Zero rows out of the
    /// outer `SELECT` is therefore exactly "the guard fired".
    ///
    /// `key_prefix` comes from the kind, never from the caller ([`NewItem`] has no such field), and
    /// `r` is joined into the final `SELECT` so the dependency on the revision insert is written
    /// down rather than implied.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for an unknown or cross-project kind, for a duplicate `item.id`
    /// (`23505`) and for a `created_by` that names no `app_user` row (`23503`, which is what the
    /// nil UUID does).
    async fn mint_item(&self, new: NewItem) -> Result<Item> {
        let minted = sqlx::query_as!(
            Item,
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
                SELECT $1, $2, $3, c.prefix, c.last_value, $4, $5, $6, $7, $8, $9, $10 FROM c
                RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
                          priority, required_tags, touched_paths, step_graph_id, version,
                          created_by, created_at, updated_at, closed_at
            ), r AS (
                INSERT INTO item_revision (item_id, version, title, body, required_tags,
                                           author_id, box_id, reason)
                SELECT id, version, title, body, required_tags, $10, $11, 'created' FROM i
                RETURNING item_id
            )
            SELECT i.id            AS "id!: ItemId",
                   i.project_id    AS "project_id!: htui_core::model::ProjectId",
                   i.kind_id       AS "kind_id!: htui_core::model::ItemKindId",
                   i.key_prefix    AS "key_prefix!",
                   i.key_number    AS "key_number!",
                   i.key           AS "key!",
                   i.title         AS "title!",
                   i.body          AS "body!",
                   i.status        AS "status!: Status",
                   i.priority      AS "priority!",
                   i.required_tags AS "required_tags!",
                   i.touched_paths AS "touched_paths!",
                   i.step_graph_id AS "step_graph_id?: htui_core::model::StepGraphId",
                   i.version       AS "version!",
                   i.created_by    AS "created_by!: htui_core::model::UserId",
                   i.created_at    AS "created_at!",
                   i.updated_at    AS "updated_at!",
                   i.closed_at     AS "closed_at?"
              FROM i, r
            "#,
            new.id.as_uuid(),
            new.project_id.as_uuid(),
            new.kind_id.as_uuid(),
            new.title,
            new.body,
            new.priority,
            &new.required_tags[..],
            &new.touched_paths[..],
            new.step_graph_id.map(|id| id.as_uuid()),
            new.created_by.as_uuid(),
            new.box_id.map(|id| id.as_uuid()),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        minted
            .ok_or_else(|| StoreError::Constraint(kind_not_in_project(new.kind_id, new.project_id)))
    }

    /// Compare-and-set edit over the seven spec columns of ANA-9 §4.2, with `COALESCE` patch
    /// semantics (ANA-9 §7.2).
    ///
    /// `step_graph_id` needs **two** parameters rather than a `COALESCE`: the field is a
    /// double-[`Option`], and `None` ("leave it") and `Some(None)` ("clear the override back to the
    /// kind default") would both arrive as SQL `NULL`. `CASE WHEN $9 THEN $10 END` splits them.
    ///
    /// `key_prefix`, `key_number` and `key` are never in the `SET` list, even across a kind change
    /// (§4.1), and `updated_at` is left to the trigger.
    ///
    /// Zero rows means one of three things, disambiguated by one follow-up read rather than by
    /// three statements up front: no such item, a lost race, or the kind guard. The version still
    /// matching after a zero-row compare-and-set can only be the guard, which is also `MemStore`'s
    /// order of judgements (compare-and-set first, kind check second).
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the item does not exist, or when the ancestor revision a
    /// divergence needs is missing; [`StoreError::Constraint`] when the patched kind belongs to
    /// another project, or when `author_id` names no `app_user` row.
    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> Result<UpdateOutcome> {
        let updated = sqlx::query_as!(
            Item,
            r#"
            WITH u AS (
                UPDATE item SET
                    title         = COALESCE($3, title),
                    body          = COALESCE($4, body),
                    kind_id       = COALESCE($5, kind_id),
                    required_tags = COALESCE($6, required_tags),
                    priority      = COALESCE($7, priority),
                    touched_paths = COALESCE($8, touched_paths),
                    step_graph_id = CASE WHEN $9 THEN $10 ELSE step_graph_id END,
                    version       = version + 1
                 WHERE id = $1
                   AND version = $2
                   AND ($5::uuid IS NULL OR EXISTS (
                           SELECT 1 FROM item_kind k
                            WHERE k.id = $5 AND k.project_id = item.project_id))
                RETURNING id, project_id, kind_id, key_prefix, key_number, key, title, body, status,
                          priority, required_tags, touched_paths, step_graph_id, version,
                          created_by, created_at, updated_at, closed_at
            ), r AS (
                INSERT INTO item_revision (item_id, version, title, body, required_tags,
                                           author_id, box_id, reason)
                SELECT id, version, title, body, required_tags, $11, $12, $13 FROM u
                RETURNING item_id
            )
            SELECT u.id            AS "id!: ItemId",
                   u.project_id    AS "project_id!: htui_core::model::ProjectId",
                   u.kind_id       AS "kind_id!: htui_core::model::ItemKindId",
                   u.key_prefix    AS "key_prefix!",
                   u.key_number    AS "key_number!",
                   u.key           AS "key!",
                   u.title         AS "title!",
                   u.body          AS "body!",
                   u.status        AS "status!: Status",
                   u.priority      AS "priority!",
                   u.required_tags AS "required_tags!",
                   u.touched_paths AS "touched_paths!",
                   u.step_graph_id AS "step_graph_id?: htui_core::model::StepGraphId",
                   u.version       AS "version!",
                   u.created_by    AS "created_by!: htui_core::model::UserId",
                   u.created_at    AS "created_at!",
                   u.updated_at    AS "updated_at!",
                   u.closed_at     AS "closed_at?"
              FROM u, r
            "#,
            id.as_uuid(),
            expected_version,
            patch.title,
            patch.body,
            patch.kind_id.map(|id| id.as_uuid()),
            patch.required_tags.as_deref(),
            patch.priority,
            patch.touched_paths.as_deref(),
            patch.step_graph_id.is_some(),
            patch.step_graph_id.flatten().map(|id| id.as_uuid()),
            patch.author_id.as_uuid(),
            patch.box_id.map(|id| id.as_uuid()),
            patch.reason,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if let Some(head) = updated {
            return Ok(UpdateOutcome::Updated(head));
        }

        let Some(head) = self.item(id).await? else {
            return Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            });
        };
        if head.version == expected_version {
            // The compare-and-set matched, so the only other conjunct of the `WHERE` - the kind
            // guard - is what refused the row. It cannot have fired without a patched kind, and
            // `version` never decreases, so the `None` arm is unreachable rather than a second
            // rule: it is reported as a backend fault instead of being dressed up as a kind error.
            return Err(match patch.kind_id {
                Some(kind) => StoreError::Constraint(kind_not_in_project(kind, head.project_id)),
                None => StoreError::Backend(format!(
                    "item `{id}` refused a compare-and-set at version {expected_version} that \
                     matched: no rule of §4.2 can produce this"
                )),
            });
        }

        let ancestor = self.revision(id, expected_version).await?;
        Ok(UpdateOutcome::Diverged { head, ancestor })
    }

    /// The status compare-and-set of ANA-9 §4.2: never bumps `version`, never writes a revision.
    ///
    /// `closed_at` follows the *current* status in both directions - set on a move to a terminal
    /// status, cleared on a move back to a live one (blueprint H.12), which is the MOD-1 errata
    /// rule `closed_at = to.is_terminal().then_some(now)` written in SQL.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the item does not exist. A `from` that does not match the
    /// current status is `Ok(false)`, not an error, exactly as `MemStore` answers.
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> Result<bool> {
        let moved = sqlx::query!(
            "UPDATE item \
                SET status    = $3, \
                    closed_at = CASE WHEN $3 IN ('done','closed') THEN clock_timestamp() ELSE NULL END \
              WHERE id = $1 AND status = $2",
            id.as_uuid(),
            from.as_str(),
            to.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if moved == 1 {
            return Ok(true);
        }

        let exists = sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_some();

        if exists {
            Ok(false)
        } else {
            Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            })
        }
    }
}

impl PgStore {
    /// One `item_revision` row: the ancestor a [`UpdateOutcome::Diverged`] answer is rendered
    /// against (ANA-9 §4.2).
    ///
    /// Private because §6.1 exposes revisions only through the divergence answer; MOD-11's history
    /// view will promote it.
    async fn revision(&self, id: ItemId, version: i32) -> Result<ItemRevision> {
        sqlx::query_as!(
            ItemRevision,
            r#"
            SELECT item_id   AS "item_id: ItemId",
                   version,
                   title,
                   body,
                   required_tags,
                   author_id AS "author_id: htui_core::model::UserId",
                   box_id    AS "box_id: htui_core::model::BoxId",
                   reason,
                   created_at
              FROM item_revision WHERE item_id = $1 AND version = $2
            "#,
            id.as_uuid(),
            version,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or_else(|| StoreError::NotFound {
            entity: "item_revision",
            id: format!("{id}@{version}"),
        })
    }
}
