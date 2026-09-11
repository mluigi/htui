//! `impl ReadStore for PgStore` and the inherent reads beside it (blueprint C.7, C.8): the four
//! the milestone named, plus `agents` (MOD-2 plan D3) and `project_settings` (MOD-2 plan D70).
//!
//! Ordering is `MemStore`'s, because `htui_core::store::conformance` is the spec both backends are
//! written to: `items` by scope position then `(key_prefix, key_number)`, `links` breadth-first
//! over live edges in both directions, `documents` by `(kind, version)`, `notes` by `created_at`,
//! `runs` newest first with steps by `(position, attempt, fanout_index)`, `step_events` by `seq`.
//!
//! Every statement is static text with `$n` placeholders, and every parameter is bound as a
//! primitive - `Uuid`, `&str`, `&[String]`, `i32` - never as an ID newtype or a `str_enum!` enum
//! (blueprint A.8). Result columns come back through the type-override syntax, which only needs
//! `Decode`.

use htui_core::model::{
    Agent, AgentBox, AgentId, AgentSummary, BoxInfo, Document, DocumentHead, DocumentId, Item,
    ItemFilter, ItemId, LinkEdge, LinkGraph, LinkKind, Note, Project, ProjectId, ProjectRef,
    PromptScope, RunId, RunStepSummary, RunSummary, Scope, SessionEvent, StepId, UpstreamEntry,
    WorkspaceSummary,
};
use htui_core::store::{ReadStore, Result, StoreError};
use serde_json::Value;
use uuid::Uuid;

use crate::error::map_sqlx;
use crate::pg::PgStore;
use crate::pg::rows::{LinkNodeRow, RunRow, StepRow};

/// The `status IN (...)` list `active_runs` spells out, because `query!` needs a literal and
/// cannot read a Rust constant.
///
/// Test-only: the query text is the real definition, and
/// `active_runs_literal_matches_run_status` is what keeps it honest against
/// [`RunStatus::is_active`](htui_core::model::RunStatus::is_active).
#[cfg(test)]
const ACTIVE_RUN_STATUSES: [&str; 3] = ["queued", "running", "awaiting_approval"];

/// The `Uuid`s of a scope's projects, in scope order (blueprint H.17).
fn project_uuids(scope: &Scope) -> Vec<Uuid> {
    scope
        .project_ids
        .iter()
        .map(|id| id.as_uuid())
        .collect::<Vec<_>>()
}

/// A filter's optional project narrowing as `Uuid`s (blueprint H.16: it narrows, never widens).
fn filter_uuids(ids: Option<&Vec<ProjectId>>) -> Option<Vec<Uuid>> {
    ids.map(|ids| ids.iter().map(|id| id.as_uuid()).collect())
}

impl ReadStore for PgStore {
    /// Every [`ItemFilter`] field in SQL, ordered by scope position then `(key_prefix, key_number)`.
    ///
    /// The scope order is carried in as the same array the `WHERE` uses and recovered with
    /// `array_position`, so the Backlog list renders the statement's order directly. The readiness
    /// conjunct is ANA-9 §7.4's `NOT EXISTS` plus `status = 'open'`, compared against the wanted
    /// boolean so `ready: Some(false)` selects the *not*-ready items.
    async fn items(
        &self,
        scope: &Scope,
        filter: &ItemFilter,
    ) -> Result<Vec<htui_core::model::ItemSummary>> {
        if scope.is_empty() {
            return Ok(Vec::new()); // `= ANY('{}')` is false for every row; skip the round trip.
        }
        let projects = project_uuids(scope);
        let narrowed = filter_uuids(filter.project_ids.as_ref());
        let statuses = filter.statuses.as_ref().map(|list| {
            list.iter()
                .map(|s| s.as_str().to_owned())
                .collect::<Vec<_>>()
        });

        sqlx::query_as!(
            htui_core::model::ItemSummary,
            r#"
            SELECT i.id            AS "id: ItemId",
                   i.project_id    AS "project_id: ProjectId",
                   i.kind_id       AS "kind_id: htui_core::model::ItemKindId",
                   i.key           AS "key!",
                   i.key_prefix,
                   i.key_number,
                   i.title,
                   i.status        AS "status: htui_core::model::Status",
                   i.priority,
                   i.required_tags,
                   i.updated_at
              FROM item i
             WHERE i.project_id = ANY($1)
               AND ($2::uuid[] IS NULL OR i.project_id = ANY($2))
               AND ($3::text[] IS NULL OR i.status = ANY($3))
               AND ($4::text[] IS NULL OR i.required_tags @> $4)
               -- `position`, not ILIKE: the needle is a literal substring, so `%` and `_` in it
               -- must not act as wildcards. `MemStore` uses `contains` and the mirror uses
               -- `instr(lower(..), lower(..))`; all three agree.
               AND ($5::text IS NULL
                    OR position(lower($5::text) in lower(i.key)) > 0
                    OR position(lower($5::text) in lower(i.title)) > 0)
               AND ($6::bool IS NULL OR $6 = (
                        i.status = 'open'
                    AND NOT EXISTS (
                        SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id
                         WHERE l.from_item_id = i.id AND l.kind = 'blocked_by'
                           AND l.deleted_at IS NULL AND t.status NOT IN ('done','closed'))))
             ORDER BY array_position($1, i.project_id), i.key_prefix, i.key_number
            "#,
            &projects[..],
            narrowed.as_deref(),
            statuses.as_deref(),
            filter.tags.as_deref(),
            filter.text.as_deref(),
            filter.ready,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One item with every `item` column, body included.
    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        sqlx::query_as!(
            Item,
            r#"
            SELECT id            AS "id: ItemId",
                   project_id    AS "project_id: ProjectId",
                   kind_id       AS "kind_id: htui_core::model::ItemKindId",
                   key_prefix,
                   key_number,
                   key           AS "key!",
                   title,
                   body,
                   status        AS "status: htui_core::model::Status",
                   priority,
                   required_tags,
                   touched_paths,
                   step_graph_id AS "step_graph_id: htui_core::model::StepGraphId",
                   version,
                   created_by    AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at,
                   closed_at
              FROM item WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The link neighbourhood: a recursive walk over live edges in **both** directions, crossing
    /// projects by UUID, with `MIN(depth)` per node - which is the breadth-first depth `MemStore`
    /// produces.
    ///
    /// `UNION` (not `UNION ALL`) plus the `depth < $2` guard is what terminates the walk on a
    /// cycle. `hops = 0` returns the root alone; a tombstoned edge is neither traversed nor
    /// returned.
    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        let root = id.as_uuid();
        let nodes = sqlx::query_as!(
            LinkNodeRow,
            r#"
            WITH RECURSIVE walk(item_id, depth) AS (
                    SELECT $1::uuid, 0
                UNION
                    SELECT CASE WHEN l.from_item_id = w.item_id
                                THEN l.to_item_id ELSE l.from_item_id END,
                           w.depth + 1
                      FROM walk w
                      JOIN item_link l
                        ON (l.from_item_id = w.item_id OR l.to_item_id = w.item_id)
                     WHERE l.deleted_at IS NULL AND w.depth < $2::int
            ),
            node AS (SELECT item_id, MIN(depth) AS depth FROM walk GROUP BY item_id)
            SELECT n.item_id     AS "item_id!: ItemId",
                   i.project_id  AS "project_id!: ProjectId",
                   p.slug        AS "project_slug!",
                   i.key         AS "key!",
                   i.title       AS "title!",
                   i.status      AS "status!: htui_core::model::Status",
                   n.depth       AS "depth!"
              FROM node n
              JOIN item i    ON i.id = n.item_id
              JOIN project p ON p.id = i.project_id
             ORDER BY n.depth, i.key
            "#,
            root,
            i32::from(hops),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        // `node` always holds the root, and every other node came out of an `item_link` row whose
        // endpoints are foreign keys into `item`; an empty result therefore means one thing only.
        if nodes.is_empty() {
            return Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            });
        }

        let reached: Vec<Uuid> = nodes.iter().map(|node| node.item_id.as_uuid()).collect();
        let edges = sqlx::query!(
            r#"
            SELECT l.from_item_id AS "from_item_id: ItemId",
                   l.to_item_id   AS "to_item_id: ItemId",
                   l.kind         AS "kind: LinkKind"
              FROM item_link l
             WHERE l.deleted_at IS NULL
               AND l.from_item_id = ANY($1) AND l.to_item_id = ANY($1)
            "#,
            &reached[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(LinkGraph {
            root: id,
            nodes: nodes.into_iter().map(LinkNodeRow::into_node).collect(),
            edges: edges
                .into_iter()
                .map(|row| LinkEdge {
                    from_item_id: row.from_item_id,
                    to_item_id: row.to_item_id,
                    kind: row.kind,
                })
                .collect(),
        })
    }

    /// Document heads, `(kind, version)` ascending. `body` is deliberately not in the `SELECT`
    /// list: the sub-tab lists heads and fetches a body only when one is opened.
    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        sqlx::query_as!(
            DocumentHead,
            r#"
            SELECT id                  AS "id: htui_core::model::DocumentId",
                   item_id             AS "item_id: ItemId",
                   kind,
                   version,
                   title,
                   produced_by_step_id AS "produced_by_step_id: StepId",
                   created_by          AS "created_by: htui_core::model::UserId",
                   created_at
              FROM document WHERE item_id = $1 ORDER BY kind, version
            "#,
            id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Notes, `created_at` ascending with `id` as the tiebreaker so two notes written in the same
    /// transaction keep a stable order across reads.
    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        sqlx::query_as!(
            Note,
            r#"
            SELECT id          AS "id: htui_core::model::NoteId",
                   item_id     AS "item_id: ItemId",
                   body,
                   created_by  AS "created_by: htui_core::model::UserId",
                   box_id      AS "box_id: htui_core::model::BoxId",
                   via_step_id AS "via_step_id: StepId",
                   created_at
              FROM item_note WHERE item_id = $1 ORDER BY created_at, id
            "#,
            id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Runs newest first, each with its steps.
    ///
    /// Two statements rather than one `LEFT JOIN` with a row per step: a run with no steps must
    /// still appear, and [`RunSummary`] is a nested shape.
    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        let runs = sqlx::query_as!(
            RunRow,
            r#"
            SELECT r.id               AS "id: RunId",
                   r.item_id          AS "item_id: ItemId",
                   r.project_id       AS "project_id: ProjectId",
                   r.kind             AS "kind: htui_core::model::RunKind",
                   r.mode             AS "mode: htui_core::model::RunMode",
                   r.status           AS "status: htui_core::model::RunStatus",
                   r.target_box_id    AS "target_box_id: htui_core::model::BoxId",
                   r.executing_box_id AS "executing_box_id: htui_core::model::BoxId",
                   COALESCE(b.hostname, '') AS "box_hostname!",
                   r.queued_at,
                   r.started_at,
                   r.finished_at,
                   r.failure
              FROM run r
              LEFT JOIN box b ON b.id = COALESCE(r.executing_box_id, r.target_box_id)
             WHERE r.item_id = $1
             ORDER BY r.queued_at DESC
            "#,
            id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if runs.is_empty() {
            return Ok(Vec::new());
        }

        let run_ids: Vec<Uuid> = runs.iter().map(|run| run.id.as_uuid()).collect();
        let steps = sqlx::query_as!(
            StepRow,
            r#"
            SELECT s.run_id       AS "run_id: RunId",
                   s.id           AS "id: StepId",
                   s.position,
                   s.attempt,
                   s.fanout_index,
                   s.phase_name,
                   s.agent_id     AS "agent_id: htui_core::model::AgentId",
                   s.model,
                   s.status       AS "status: htui_core::model::StepStatus",
                   s.gate_outcome AS "gate_outcome: htui_core::model::GateOutcome",
                   s.started_at,
                   s.finished_at,
                   -- Plan D106's two figures, derived from `trim_record` in SQL rather than
                   -- returned whole: the record is a document the Runs pane has no room for, and
                   -- §6.1 exposes neither it nor `prompt_digest`. `htui_core::model::prompt_summary`
                   -- is the same derivation in Rust, and `MemStore` uses it, so the two agree by
                   -- the `set_step_prompt` conformance case rather than by luck.
                   (s.trim_record->>'estimated_after')::int          AS "prompt_tokens?",
                   COALESCE((SELECT bool_or((e->>'trimmed')::bool)
                               FROM jsonb_array_elements(s.trim_record->'sections') e),
                            false)                                   AS "trimmed!"
              FROM run_step s
             WHERE s.run_id = ANY($1)
             ORDER BY s.run_id, s.position, s.attempt, s.fanout_index
            "#,
            &run_ids[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(runs
            .into_iter()
            .map(|run| {
                let own: Vec<RunStepSummary> = steps
                    .iter()
                    .filter(|step| step.run_id == run.id)
                    .cloned()
                    .map(StepRow::into_summary)
                    .collect();
                run.into_summary(own)
            })
            .collect())
    }

    /// The step's replay log, `seq` ascending (ANA-9 §7.5). No rows at all is `Ok(None)`, which
    /// §6.1 reads as "not cached" rather than as an error.
    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        let events = sqlx::query_as!(
            SessionEvent,
            r#"
            SELECT run_step_id AS "run_step_id: StepId",
                   seq,
                   turn,
                   kind         AS "kind: htui_core::model::EventKind",
                   role         AS "role: htui_core::model::EventRole",
                   tool_call_id,
                   payload,
                   raw,
                   at
              FROM session_event WHERE run_step_id = $1 ORDER BY seq
            "#,
            step.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok((!events.is_empty()).then_some(events))
    }

    // -------------------------------------------------------------------------------------------
    // MOD-2 milestone 9's four reads. The signatures land with the seam (T62) so the workspace
    // compiles against one trait; the bodies — the amended §7.3 recursive CTE of blueprint C.1
    // among them — are T63's, with the `.sqlx` data they need.
    // -------------------------------------------------------------------------------------------

    async fn document(&self, _id: DocumentId) -> Result<Option<Document>> {
        todo!("T63: SELECT the document row with its body")
    }

    async fn documents_of_kinds(&self, _item: ItemId, _kinds: &[String]) -> Result<Vec<Document>> {
        todo!("T63: DISTINCT ON (kind) ... ORDER BY kind, version DESC, then the caller's order")
    }

    async fn upstream_summaries(
        &self,
        _id: ItemId,
        _hops: u8,
        _scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        todo!("T63: the amended §7.3 recursive CTE (blueprint C.1), then sort_canonical")
    }

    async fn project(&self, _id: ProjectId) -> Result<Option<Project>> {
        todo!("T63: SELECT the project row with its settings")
    }
}

/// The reads `Backend` dispatches over that are not [`ReadStore`] methods.
///
/// ANA-9 §6.1 is quoted verbatim in `htui_core::store::traits` and has none of them, so they stay
/// inherent - the same signatures `MemStore` and (from T3) `CacheStore` carry, which is what
/// lets `Backend` dispatch over three arms without a fourth trait (MOD-1 blueprint B.7).
///
/// MOD-2's `agents` is inherent for a second reason on top of that one: `agent` and `agent_box`
/// are absent from the mirrored table list (`docs/ANA-9.md` §4.4), so `CacheStore` could not
/// answer it at all and `Backend::agents` refuses while offline (MOD-2 plan D3, D14).
impl PgStore {
    /// Every workspace with its projects, ordered by name then by `workspace_project.position`.
    ///
    /// One statement with two `LEFT JOIN`s, grouped in Rust: a workspace with no projects still
    /// gets a row, and grouping by the id rather than by adjacency keeps the result correct even
    /// if two workspaces share a name.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        let rows = sqlx::query!(
            r#"
            SELECT w.id   AS "workspace_id: htui_core::model::WorkspaceId",
                   w.slug,
                   w.name,
                   p.id    AS "project_id?: ProjectId",
                   p.slug  AS "project_slug?",
                   p.name  AS "project_name?",
                   wp.position AS "position?"
              FROM workspace w
              LEFT JOIN workspace_project wp ON wp.workspace_id = w.id
              LEFT JOIN project p            ON p.id = wp.project_id
             ORDER BY w.name, wp.position
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut summaries: Vec<WorkspaceSummary> = Vec::new();
        for row in rows {
            let index = match summaries
                .iter()
                .position(|ws| ws.workspace_id == row.workspace_id)
            {
                Some(index) => index,
                None => {
                    summaries.push(WorkspaceSummary {
                        workspace_id: row.workspace_id,
                        slug: row.slug,
                        name: row.name,
                        projects: Vec::new(),
                    });
                    summaries.len() - 1
                }
            };
            if let (Some(project_id), Some(slug), Some(name), Some(position)) = (
                row.project_id,
                row.project_slug,
                row.project_name,
                row.position,
            ) && let Some(summary) = summaries.get_mut(index)
            {
                summary.projects.push(ProjectRef {
                    project_id,
                    slug,
                    name,
                    position,
                });
            }
        }
        Ok(summaries)
    }

    /// This box's row, projected for the top bar.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn box_info(&self) -> Result<Option<BoxInfo>> {
        sqlx::query_as!(
            BoxInfo,
            r#"
            SELECT id        AS "box_id: htui_core::model::BoxId",
                   hostname,
                   os_family AS "os_family: htui_core::model::OsFamily"
              FROM box WHERE id = $1
            "#,
            self.this_box.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// How many runs of the scope are active
    /// ([`RunStatus::is_active`](htui_core::model::RunStatus::is_active)).
    ///
    /// The status list is written out because `query!` needs a literal; the unit test below
    /// asserts it against `RunStatus::ALL`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize> {
        if scope.is_empty() {
            return Ok(0);
        }
        let projects = project_uuids(scope);
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!" FROM run
             WHERE project_id = ANY($1)
               AND status IN ('queued','running','awaiting_approval')
            "#,
            &projects[..],
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// The scope's projects, ordered by `workspace_project.position`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>> {
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let projects = project_uuids(scope);
        sqlx::query_as!(
            ProjectRef,
            r#"
            SELECT p.id AS "project_id: ProjectId", p.slug, p.name, wp.position
              FROM workspace_project wp JOIN project p ON p.id = wp.project_id
             WHERE wp.workspace_id = $1 AND p.id = ANY($2)
             ORDER BY wp.position
            "#,
            scope.workspace_id.as_uuid(),
            &projects[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The agent registry, ordered by `agent.name`, each row carrying **this box's** `agent_box`
    /// when there is one (MOD-2 plan D3, `docs/ANA-4.md` §4.1).
    ///
    /// One statement with a `LEFT JOIN` narrowed to `this_box` in the join condition rather than
    /// in the `WHERE`, so an agent that has never been probed here still gets a row with
    /// `on_box: None` - which is what the Settings tab renders as "not probed".
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn agents(&self) -> Result<Vec<AgentSummary>> {
        let rows = sqlx::query!(
            r#"
            SELECT a.id            AS "id: AgentId",
                   a.name,
                   a.transport     AS "transport: htui_core::model::Transport",
                   a.launch,
                   a.models,
                   a.default_model,
                   a.billing       AS "billing: htui_core::model::Billing",
                   a.enabled,
                   a.settings,
                   a.created_at,
                   a.updated_at,
                   ab.box_id       AS "box_id?: htui_core::model::BoxId",
                   ab.enabled      AS "box_enabled?",
                   ab.version      AS "box_version?",
                   ab.path         AS "box_path?",
                   ab.probed_at    AS "box_probed_at?",
                   ab.quota        AS "box_quota?",
                   ab.quota_at     AS "box_quota_at?",
                   ab.updated_at   AS "box_updated_at?",
                   ab.probe        AS "box_probe?"
              FROM agent a
              LEFT JOIN agent_box ab ON ab.agent_id = a.id AND ab.box_id = $1
             ORDER BY a.name
            "#,
            self.this_box.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let agent = Agent {
                    id: row.id,
                    name: row.name,
                    transport: row.transport,
                    launch: row.launch,
                    models: row.models,
                    default_model: row.default_model,
                    billing: row.billing,
                    enabled: row.enabled,
                    settings: row.settings,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                };
                let on_box = match (row.box_id, row.box_enabled, row.box_updated_at) {
                    (Some(box_id), Some(enabled), Some(updated_at)) => Some(AgentBox {
                        agent_id: agent.id,
                        box_id,
                        enabled,
                        version: row.box_version,
                        path: row.box_path,
                        probed_at: row.box_probed_at,
                        quota: row.box_quota,
                        quota_at: row.box_quota_at,
                        updated_at,
                        probe: row.box_probe,
                    }),
                    _ => None,
                };
                AgentSummary { agent, on_box }
            })
            .collect())
    }

    /// `project.settings` of one project, or `None` when no such row exists (MOD-2 plan D70).
    ///
    /// Inherent beside [`PgStore::agents`] rather than a [`ReadStore`]
    /// method, for the reason that read gives: `Backend` dispatches over three stores and the
    /// projection is one column. The per-run token cap lives in this document
    /// (`docs/ANA-4.md` §7 `:1143-1150`), which is why the column needed a reader at all — it has
    /// been `JSONB NOT NULL` since `0001_init.sql:150` and nothing read it.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
        sqlx::query_scalar!(
            "SELECT settings FROM project WHERE id = $1",
            project.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }
}

#[cfg(test)]
mod tests {
    use htui_core::model::RunStatus;

    use super::ACTIVE_RUN_STATUSES;

    #[test]
    fn active_runs_literal_matches_run_status() {
        let from_model: Vec<&str> = RunStatus::ALL
            .iter()
            .filter(|status| status.is_active())
            .map(|status| status.as_str())
            .collect();
        assert_eq!(
            from_model,
            ACTIVE_RUN_STATUSES.to_vec(),
            "the `status IN (...)` literal of `active_runs` and `RunStatus::is_active` must agree"
        );
    }
}
