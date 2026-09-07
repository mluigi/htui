//! `impl WriteStore for PgStore` (blueprint C.9): the three write paths of ANA-9 §4.1 and §4.2,
//! plus MOD-2's six (plan D3, `docs/ANA-4.md` §4.1).
//!
//! Each one is a **single statement**, so atomicity comes from Postgres rather than from an
//! explicit transaction: a `WITH ... RETURNING` CTE either lands whole or not at all, and under
//! `READ COMMITTED` the loser of a compare-and-set race blocks on the row lock, re-evaluates
//! `version = $2` against the committed value and matches nothing (ANA-9 §11.3). The two
//! exceptions are MOD-2's chat-run pair, which writes one `run` and one `run_step` and therefore
//! takes an explicit transaction, exactly as the pending-buffer upload does
//! (`crate::cache::pending`).
//!
//! There is **no `DELETE FROM item`** in this file and none may be added: §4.1's "keys are never
//! reused" is enforced by the absence of the path, and the conformance case `no_delete_path` is
//! what pins it. Nor does any statement write `updated_at` on an update path - the `BEFORE UPDATE`
//! trigger of the migration owns it, and `RETURNING` sees the trigger-modified row.

use chrono::{DateTime, Utc};
use htui_core::model::{
    Agent, AgentBox, ChatRunSpec, Item, ItemId, ItemPatch, ItemRevision, NewItem, RunId, RunStatus,
    SessionEvent, Status, StepId,
};
use htui_core::store::{
    ReadStore as _, Result, StoreError, UpdateOutcome, WriteStore, chat_step_status,
    not_a_terminal_status,
};
use serde_json::Value;

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

    /// Appends session events in one `INSERT`, skipping the `(run_step_id, seq)` pairs already
    /// stored (ANA-9 §4.3, plan D3).
    ///
    /// The batch is carried as **one** `jsonb` parameter and expanded by `jsonb_to_recordset`,
    /// rather than as nine parallel arrays: `SessionEvent`'s serde form already *is* the row -
    /// field names are the column names verbatim - which is the same fact the pending buffer's
    /// line format rests on (`crate::cache::pending`), and nullable `text[]` / `jsonb[]` parameters
    /// are avoided entirely.
    ///
    /// `rows_affected()` counts inserts only, because `ON CONFLICT ... DO NOTHING` reports the
    /// skipped rows as unaffected: that is the "how many landed" answer §4.1 asks for, and it is
    /// what makes a replayed offline buffer distinguishable from a fresh one.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when an event names a `run_step` that does not exist (`23503`)
    /// or a `kind` / `role` outside the §4.3 `CHECK` lists (`23514`). One statement, so a refused
    /// batch writes none of its rows.
    async fn append_events(&self, events: &[SessionEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let rows = serde_json::to_value(events).map_err(|err| {
            StoreError::Backend(format!("session_event does not serialise: {err}"))
        })?;

        let inserted = sqlx::query!(
            r#"
            INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id,
                                       payload, raw, at)
            SELECT e.run_step_id, e.seq, e.turn, e.kind, e.role, e.tool_call_id, e.payload,
                   e.raw, e.at
              FROM jsonb_to_recordset($1::jsonb)
                   AS e(run_step_id uuid, seq int, turn int, kind text, role text,
                        tool_call_id text, payload jsonb, raw jsonb, at timestamptz)
            ON CONFLICT (run_step_id, seq) DO NOTHING
            "#,
            rows,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        Ok(usize::try_from(inserted).unwrap_or(0))
    }

    /// `run_step.usage` on every call, `run_step.prompt_digest` only when one is supplied
    /// (`docs/ANA-4.md` §4.1, plan D15(b)).
    ///
    /// `COALESCE($3, prompt_digest)` is what makes `None` mean "leave it": the recorder computes
    /// the digest once, at the prompt, and every later usage write for the same step passes `None`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when the step does not exist - zero rows updated is the only thing
    /// this statement can mean.
    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> Result<()> {
        let updated = sqlx::query!(
            "UPDATE run_step \
                SET usage = $2, prompt_digest = COALESCE($3, prompt_digest) \
              WHERE id = $1",
            step.as_uuid(),
            usage,
            prompt_digest.as_deref(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();

        if updated == 0 {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        }
        Ok(())
    }

    /// Inserts or updates one `agent` row, keyed by `agent.id` (`docs/ANA-4.md` §4.1, ANA-9 §5.7).
    ///
    /// `created_at` and `updated_at` are supplied on the insert and neither is in the `DO UPDATE`
    /// `SET` list: the row keeps the creation stamp it was first written with, and the migration's
    /// `BEFORE UPDATE` trigger owns `updated_at` on every later write.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when another id already holds the name (`23505` on
    /// `agent_name_key`).
    async fn upsert_agent(&self, agent: &Agent) -> Result<()> {
        sqlx::query!(
            "INSERT INTO agent (id, name, transport, launch, models, default_model, billing, \
                                enabled, settings, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (id) DO UPDATE SET \
                 name          = EXCLUDED.name, \
                 transport     = EXCLUDED.transport, \
                 launch        = EXCLUDED.launch, \
                 models        = EXCLUDED.models, \
                 default_model = EXCLUDED.default_model, \
                 billing       = EXCLUDED.billing, \
                 enabled       = EXCLUDED.enabled, \
                 settings      = EXCLUDED.settings",
            agent.id.as_uuid(),
            agent.name,
            agent.transport.as_str(),
            &agent.launch,
            &agent.models[..],
            agent.default_model.as_deref(),
            agent.billing.as_str(),
            agent.enabled,
            &agent.settings,
            agent.created_at,
            agent.updated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// Inserts or updates one `agent_box` row on its composite primary key (`docs/ANA-4.md` §4.1,
    /// ANA-9 §5.7).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the agent or the box does not exist (`23503`).
    async fn upsert_agent_box(&self, row: &AgentBox) -> Result<()> {
        sqlx::query!(
            "INSERT INTO agent_box (agent_id, box_id, enabled, version, path, probed_at, quota, \
                                    quota_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (agent_id, box_id) DO UPDATE SET \
                 enabled   = EXCLUDED.enabled, \
                 version   = EXCLUDED.version, \
                 path      = EXCLUDED.path, \
                 probed_at = EXCLUDED.probed_at, \
                 quota     = EXCLUDED.quota, \
                 quota_at  = EXCLUDED.quota_at",
            row.agent_id.as_uuid(),
            row.box_id.as_uuid(),
            row.enabled,
            row.version.as_deref(),
            row.path.as_deref(),
            row.probed_at,
            row.quota.as_ref(),
            row.quota_at,
            row.updated_at,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// The `run` / `run_step` pair of a free-standing chat, in one transaction, both
    /// `ON CONFLICT (id) DO NOTHING` (plan D4).
    ///
    /// The pair has the shape `crate::cache::pending`'s upload gives the same chat - `kind 'chat'`,
    /// `mode 'manual'`, `item_id NULL`, `phase_name 'chat'`, `position 0`, `attempt 1`,
    /// `fanout_index 0`, `executing_box_id` = the target box - and, because the ids are minted
    /// client-side and both paths insert `ON CONFLICT (id) DO NOTHING`, an online start followed by
    /// a replayed upload (or the reverse) converges on **one** such pair rather than colliding
    /// (ANA-9 §4.3).
    ///
    /// It is the row count that converges, not every column: `DO NOTHING` hands the values to
    /// whichever path lands first, and this one differs from the upload in four groups. `status` is
    /// `running` rather than the upload's terminal `done` and `finished_at` is NULL, which
    /// [`WriteStore::finish_chat_run`] closes; `agent_id` and `model` come from the spec, where the
    /// upload leaves both NULL because the pending line format carries neither; and every stamp is
    /// `chat.started_at`, where the upload derives them from the buffered events' `at`. An
    /// offline-first chat therefore keeps a NULL `agent_id` after a later online start - see
    /// [`ChatRunSpec`] for the whole asymmetry, and `tests/pg_criteria.rs`
    /// (`chat_run_rows_converge_with_the_offline_mint`,
    /// `an_offline_first_chat_keeps_the_uploaded_columns`) for both directions in SQL. Carrying
    /// `agent_id` / `model` in the pending format is the offline session path's, MOD-2 milestone 4
    /// (plan D16).
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when the project, box, user or agent does not exist (`23503`).
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                              executing_box_id, started_by, queued_at, started_at, finished_at) \
             VALUES ($1, $2, NULL, 'chat', 'manual', 'running', $3, $3, $4, $5, $5, NULL) \
             ON CONFLICT (id) DO NOTHING",
            chat.run_id.as_uuid(),
            chat.project_id.as_uuid(),
            chat.target_box_id.as_uuid(),
            chat.started_by.as_uuid(),
            chat.started_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query!(
            "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, \
                                   agent_id, model, status, started_at, finished_at) \
             VALUES ($1, $2, 0, 1, 0, 'chat', $3, $4, 'running', $5, NULL) \
             ON CONFLICT (id) DO NOTHING",
            chat.step_id.as_uuid(),
            chat.run_id.as_uuid(),
            chat.agent_id.map(htui_core::model::AgentId::as_uuid),
            chat.model.as_deref(),
            chat.started_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        tx.commit().await.map_err(map_sqlx)
    }

    /// Closes both rows of a chat run in one transaction (plan D4, assumption A1).
    ///
    /// Without it `active_runs` would count a finished chat forever. The step takes the status of
    /// the same name; the three values `RunStatus` and `StepStatus` share are the only ones
    /// accepted, so a live status is refused before either row is touched rather than leaving the
    /// pair half-closed.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] for a non-terminal `status`; [`StoreError::NotFound`] when the
    /// run or the step does not exist.
    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> Result<()> {
        let step_status = chat_step_status(status)
            .ok_or_else(|| StoreError::Constraint(not_a_terminal_status(status)))?;

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let closed_run = sqlx::query!(
            "UPDATE run SET status = $2, finished_at = $3 WHERE id = $1",
            run.as_uuid(),
            status.as_str(),
            finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if closed_run == 0 {
            return Err(StoreError::NotFound {
                entity: "run",
                id: run.to_string(),
            });
        }

        let closed_step = sqlx::query!(
            "UPDATE run_step SET status = $2, finished_at = $3 WHERE id = $1",
            step.as_uuid(),
            step_status.as_str(),
            finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if closed_step == 0 {
            return Err(StoreError::NotFound {
                entity: "run_step",
                id: step.to_string(),
            });
        }

        tx.commit().await.map_err(map_sqlx)
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
