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

use std::collections::BTreeMap;

use htui_core::model::{
    Agent, AgentBox, AgentId, AgentSummary, BoundSkill, BoxId, BoxInfo, BoxProfile, BoxRow,
    BoxTool, CommandQueue, CoverageRow, Document, DocumentHead, DocumentId, Gate, GateOutcome,
    Isolation, Item, ItemCitation, ItemFilter, ItemId, ItemKind, ItemKindId, LinkEdge, LinkGraph,
    LinkKind, Note, PhaseAgent, PhaseId, Project, ProjectId, ProjectRef, PromptScope,
    PromptTemplate, Repo, RepoBoxPath, RepoId, Requirement, RequirementArea, RequirementFilter,
    RequirementId, RequirementRevision, RequirementSpec, ResolvedGraph, ResolvedInput,
    ResolvedPhase, Run, RunId, RunKind, RunMode, RunStatus, RunStep, RunStepCommit, RunStepSummary,
    RunStepTree, RunSummary, Scope, SessionEvent, SkillId, SkillVersion, StepGraph, StepGraphId,
    StepGraphPhase, StepId, StepStatus, UpstreamEntry, UserId, VerifyOutcome, Workspace,
    WorkspaceBoxPath, WorkspaceId, WorkspaceProject, WorkspaceSummary,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::store::{ReadStore, Result, SettingRung, StoreError, StoredSetting};
use serde_json::Value;
use uuid::Uuid;

use crate::error::map_sqlx;
use crate::pg::PgStore;
use crate::pg::rows::{LinkNodeRow, RunRow, SkillBindingRow, StepRow, UpstreamRow};
use crate::{MAX_UPSTREAM_HOPS, order_documents};

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
                   i.updated_at,
                   i.touched_paths
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
                   closed_at,
                   resolution    AS "resolution: htui_core::model::Resolution"
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
                   --
                   -- Guarded casts rather than casts (T68, F-52): `trim_record` is an untyped
                   -- `JSONB` column, and a bare `(…->>'estimated_after')::int` *raises* on a
                   -- string, a float or a number outside `i32`, as `jsonb_array_elements` does on
                   -- a `sections` that is not an array and `::bool` on a `trimmed` that is not
                   -- one. Any of those would fail the whole Runs read over one bad row, where
                   -- `prompt_summary` answers `(None, false)`; a `CASE` whose guard fails yields
                   -- SQL `NULL`, which is the same answer.
                   --
                   -- The guard is a **text** test and not a numeric one, because `Value::as_i64`
                   -- rejects a float by its *type* and no numeric predicate can see that: `35988.0`
                   -- is integral, is in `i32` range, and satisfies `@.floor() == @`, so the
                   -- jsonpath this replaces admitted it and then handed `::int` the text
                   -- `35988.0`, which raises (T68, F-52 review, H1). `jsonb` preserves the trailing
                   -- `.0` a float was written with, so the regex sees the difference the numeric
                   -- predicates cannot, and it also bounds the digits so the `::bigint` that
                   -- range-checks cannot itself overflow. (A record whose wire text says `1e3`
                   -- would normalise to `1000` and project as an integer where Rust reads a float;
                   -- `set_step_prompt` takes a `serde_json::Value`, which never writes that form.)
                   CASE WHEN jsonb_typeof(s.trim_record->'estimated_after') = 'number'
                         AND (s.trim_record->>'estimated_after') ~ '^-?[0-9]{1,10}$'
                         AND (s.trim_record->>'estimated_after')::bigint
                             BETWEEN -2147483648 AND 2147483647
                        THEN (s.trim_record->>'estimated_after')::int
                   END                                              AS "prompt_tokens?",
                   -- An `EXISTS` over the elements and not `jsonb_path_exists`, for the same
                   -- reason: SQL/JSON's **lax** mode (the default) auto-wraps a non-array before
                   -- `[*]` and unwraps a nested one, so `$.sections[*] ? (@.trimmed == true)`
                   -- answered `true` for a `sections` that is one section rather than a list of
                   -- them, for one nested an array too deep, and — because `==` unwraps its
                   -- operand too — for a `trimmed` of `[true]`. Rust reads all three as `false`
                   -- (`Value::as_array`, then `as_bool`), and so does the mirror (T68, F-52
                   -- review, H2 and F-90).
                   --
                   -- The `jsonb_typeof` guard keeps this total where a bare
                   -- `jsonb_array_elements` would raise on a non-array, and `e->'trimmed'` is
                   -- `NULL` on a scalar element rather than an error. `jsonb` equality against
                   -- `'true'` is exact — it rejects `1`, `"true"` and `[true]` — which is
                   -- `Value::as_bool() == Some(true)` exactly.
                   CASE WHEN jsonb_typeof(s.trim_record->'sections') = 'array'
                        THEN EXISTS (SELECT 1
                                       FROM jsonb_array_elements(s.trim_record->'sections') e
                                      WHERE e->'trimmed' = 'true'::jsonb)
                        ELSE false
                   END                                              AS "trimmed!",
                   -- MOD-4 milestone 1: the six `RunStepSummary` fields ANA-2 added, appended in
                   -- the struct's order because `query_as!` binds positionally. `agent_name` is
                   -- what `MemStore::run_steps` reads out of its agent map, so the projection
                   -- joins the registry rather than leaving the Runs pane an id.
                   s.usage,
                   s.selected,
                   s.exit_code,
                   s.verify_outcome AS "verify_outcome: htui_core::model::VerifyOutcome",
                   s.promoted_at,
                   a.name                                           AS "agent_name?"
              FROM run_step s
              LEFT JOIN agent a ON a.id = s.agent_id
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
    // MOD-2 milestone 9's four reads (`docs/ANA-5.md` §8). Every table they touch is mirrored, so
    // `CacheStore` answers them too and `store::conformance::READ_CASES` runs over both.
    // -------------------------------------------------------------------------------------------

    /// One document **with its body**, or `None` when no row has that id.
    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        sqlx::query_as!(
            Document,
            r#"
            SELECT id                  AS "id: DocumentId",
                   item_id             AS "item_id: ItemId",
                   kind,
                   version,
                   title,
                   body,
                   produced_by_step_id AS "produced_by_step_id: StepId",
                   created_by          AS "created_by: htui_core::model::UserId",
                   created_at
              FROM document WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The latest version of each `kind`, **in `kinds` order**; an empty `kinds` is every kind the
    /// item has, in kind byte order.
    ///
    /// `DISTINCT ON (kind) ... ORDER BY kind, version DESC` is the latest-per-kind pick; the
    /// caller's order is then applied in Rust rather than in SQL, because it is an arbitrary
    /// permutation no `ORDER BY` expresses and because Postgres would sort the empty case by
    /// collation where `MemStore` and the mirror sort by bytes
    /// ([`UpstreamEntry::sort_canonical`] gives the argument in full).
    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        let latest = sqlx::query_as!(
            Document,
            r#"
            SELECT DISTINCT ON (kind)
                   id                  AS "id: DocumentId",
                   item_id             AS "item_id: ItemId",
                   kind,
                   version,
                   title,
                   body,
                   produced_by_step_id AS "produced_by_step_id: StepId",
                   created_by          AS "created_by: htui_core::model::UserId",
                   created_at
              FROM document
             WHERE item_id = $1 AND (cardinality($2::text[]) = 0 OR kind = ANY($2))
             ORDER BY kind, version DESC
            "#,
            item.as_uuid(),
            kinds,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(order_documents(latest, kinds))
    }

    /// `docs/ANA-9.md` §7.3 as amended by `docs/ANA-5.md` §4.3, in one round trip.
    ///
    /// `best` is the amendment that matters: the shipped `UNION` dedups whole rows, that is
    /// `(item_id, depth)` pairs, so a diamond emitted its apex twice and the prompt digest became
    /// a function of edge insertion order. `MIN(depth)` collapses it to one row at the nearest
    /// hop. The scope CTE is the workspace's projects, or — with no workspace (`R-ENT-2`) — the
    /// one project the caller named, and it gates only the `summary` projection, never the walk:
    /// an out-of-scope item is still reached and rendered as the `R-PRM-2` stub.
    ///
    /// The `ORDER BY` is there for readability and is **not** the contract:
    /// [`UpstreamEntry::sort_canonical`] re-sorts in Rust, because Postgres orders
    /// `qualified_key` by the database collation and the SQLite mirror by byte value.
    ///
    /// Three of the `!` overrides are load-bearing and were measured one at a time:
    /// `qualified_key` (a `||` expression), `depth` (a `MIN` aggregate) and `in_scope` (an
    /// `IS NOT NULL` predicate). `item_id`, `title` and `status` come through an ordinary inner
    /// join and sqlx types them `NOT NULL` unaided, so the blueprint's "a recursive CTE makes
    /// every column nullable" is true of the computed columns only; theirs are kept for the reason
    /// [`links`](ReadStore::links) keeps its whole set — the inference is the macro's, not the
    /// schema's, and a `!` on a column that is already non-null costs nothing.
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        // `hops == 0` is no upstream at all - unlike `links(id, 0)`, the root is the step's own
        // item and is never an entry - and the anchor term of the CTE has no depth guard of its
        // own, so the zero case has to be answered before the round trip rather than by the SQL.
        let hops = hops.min(MAX_UPSTREAM_HOPS);
        if hops == 0 {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as!(
            UpstreamRow,
            r#"
            WITH RECURSIVE up AS (
                    SELECT l.to_item_id AS item_id, 1 AS depth
                      FROM item_link l
                     WHERE l.from_item_id = $1
                       AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
                UNION
                    SELECT l.to_item_id, up.depth + 1
                      FROM item_link l JOIN up ON l.from_item_id = up.item_id
                     WHERE up.depth < $2::int
                       AND l.kind IN ('blocked_by','origin') AND l.deleted_at IS NULL
            ),
            best AS (SELECT item_id, MIN(depth) AS depth FROM up GROUP BY item_id),
            scope AS (
                    SELECT project_id FROM workspace_project WHERE workspace_id = $3::uuid
                UNION
                    SELECT $4::uuid WHERE $3::uuid IS NULL
            )
            SELECT i.id                       AS "item_id!: ItemId",
                   p.slug || ':' || i.key     AS "qualified_key!",
                   i.title                    AS "title!",
                   i.status                   AS "status!: htui_core::model::Status",
                   best.depth                 AS "depth!",
                   (s.project_id IS NOT NULL) AS "in_scope!",
                   CASE WHEN s.project_id IS NOT NULL THEN d.body END AS "summary?"
              FROM best
              JOIN item i    ON i.id = best.item_id
              JOIN project p ON p.id = i.project_id
              LEFT JOIN scope s ON s.project_id = i.project_id
              LEFT JOIN LATERAL (SELECT body FROM document
                                  WHERE item_id = i.id AND kind = 'summary'
                                  ORDER BY version DESC LIMIT 1) d ON TRUE
             -- The root is the step's own item and is never an entry, and the walk can arrive back
             -- at it: the schema forbids a self-loop only, so `R blocked_by A, A blocked_by R` is
             -- storable and reaches `R` at depth 2. `MemStore` seeds `seen` with the root and so
             -- cannot emit it; the anchor term here starts at the root's *neighbours* and nothing
             -- downstream excludes it, so without this the assembler would be handed the item's own
             -- summary as upstream context (T68, F-52 review, H3).
             WHERE best.item_id <> $1
             -- The key is spelled out rather than referenced as `qualified_key`: the `!` of the
             -- nullability override is part of the quoted output name Postgres sees, so the alias
             -- an `ORDER BY` could use is `"qualified_key!"` and not `qualified_key`.
             ORDER BY best.depth, p.slug || ':' || i.key, i.id
            "#,
            id.as_uuid(),
            i32::from(hops),
            scope.workspace.map(WorkspaceId::as_uuid),
            scope.project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut entries: Vec<UpstreamEntry> =
            rows.into_iter().map(UpstreamRow::into_entry).collect();
        UpstreamEntry::sort_canonical(&mut entries);
        Ok(entries)
    }

    /// One project row with its `settings`, or `None` when no row has that id.
    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        sqlx::query_as!(
            Project,
            r#"
            SELECT id              AS "id: ProjectId",
                   slug,
                   name,
                   description,
                   secret_provider,
                   secret_scope,
                   settings,
                   created_by      AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM project WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    // ---- ANA-2 §8's five run reads (MOD-4 milestone 1, plan D1) --------------------------------

    /// The whole `run` row, lease columns and all — not [`RunSummary`], which the Runs pane reads
    /// and which carries no lease (blueprint F-S).
    async fn run(&self, id: RunId) -> Result<Option<Run>> {
        sqlx::query_as!(
            Run,
            r#"
            SELECT id               AS "id: RunId",
                   project_id       AS "project_id: ProjectId",
                   item_id          AS "item_id: ItemId",
                   kind             AS "kind: RunKind",
                   mode             AS "mode: RunMode",
                   status           AS "status: RunStatus",
                   target_box_id    AS "target_box_id: BoxId",
                   executing_box_id AS "executing_box_id: BoxId",
                   graph_snapshot,
                   started_by       AS "started_by: UserId",
                   queued_at,
                   started_at,
                   finished_at,
                   failure,
                   repo_scope       AS "repo_scope: Vec<RepoId>",
                   lease_box_id     AS "lease_box_id: BoxId",
                   lease_expires_at,
                   updated_at
              FROM run WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Every step of the run, judge first within its position and attempt: `fanout_index` ascending
    /// puts `-1` before `0` without a special case (ANA-2 §4.5).
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>> {
        sqlx::query_as!(
            RunStep,
            r#"
            SELECT id                  AS "id: StepId",
                   run_id              AS "run_id: RunId",
                   position,
                   attempt,
                   fanout_index,
                   phase_name,
                   agent_id            AS "agent_id: AgentId",
                   model,
                   status              AS "status: StepStatus",
                   gate_outcome        AS "gate_outcome: GateOutcome",
                   gate_note,
                   selected,
                   exit_code,
                   prompt_digest,
                   trim_record,
                   usage,
                   isolation_path,
                   started_at,
                   finished_at,
                   verify_outcome      AS "verify_outcome: VerifyOutcome",
                   verify_exit_code,
                   promoted_at,
                   updated_at
              FROM run_step
             WHERE run_id = $1
             ORDER BY position, attempt, fanout_index
            "#,
            run.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>> {
        sqlx::query_as!(
            RunStepTree,
            r#"
            SELECT run_step_id AS "run_step_id: StepId",
                   repo_id     AS "repo_id: RepoId",
                   mode        AS "mode: Isolation",
                   path,
                   base_ref,
                   dirty
              FROM run_step_tree WHERE run_step_id = $1 ORDER BY repo_id
            "#,
            step.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>> {
        sqlx::query_as!(
            RunStepCommit,
            r#"
            SELECT run_step_id AS "run_step_id: StepId",
                   repo_id     AS "repo_id: RepoId",
                   before_hash,
                   after_hash
              FROM run_step_commit WHERE run_step_id = $1 ORDER BY repo_id
            "#,
            step.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// ANA-2 §4.2's input resolver: latest eligible document per kind, this run's own output
    /// preferred, fan-out losers excluded, one entry per requested kind in request order.
    ///
    /// The rank is written as an explicit `CASE` rather than the `(s.run_id = $2) DESC NULLS LAST`
    /// of §4.2's sketch, and the mirror's statement carries the same three arms: `DESC NULLS LAST`
    /// over a boolean is not spelled or sorted alike on SQLite, and the two backends have to agree
    /// (blueprint H-14). `ROW_NUMBER() OVER (PARTITION BY kind ...)` rather than `DISTINCT ON` for
    /// the same reason — it is one statement both engines run.
    ///
    /// The caller's order is applied in Rust, exactly as
    /// [`documents_of_kinds`](ReadStore::documents_of_kinds) applies it and for the same two
    /// reasons: it is an arbitrary permutation no `ORDER BY` expresses, and the empty-`kinds` case
    /// must be byte order rather than the database collation.
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>> {
        let wanted: Vec<String> = if kinds.is_empty() {
            let mut all = sqlx::query_scalar!(
                "SELECT DISTINCT kind FROM document WHERE item_id = $1",
                item.as_uuid(),
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
            all.sort_unstable();
            all
        } else {
            kinds.to_vec()
        };

        let eligible = sqlx::query_as!(
            Document,
            r#"
            SELECT id                  AS "id!: DocumentId",
                   item_id             AS "item_id!: ItemId",
                   kind                AS "kind!",
                   version             AS "version!",
                   title               AS "title!",
                   body                AS "body!",
                   produced_by_step_id AS "produced_by_step_id: StepId",
                   created_by          AS "created_by!: UserId",
                   created_at          AS "created_at!"
              FROM (SELECT d.id,
                           d.item_id,
                           d.kind,
                           d.version,
                           d.title,
                           d.body,
                           d.produced_by_step_id,
                           d.created_by,
                           d.created_at,
                           ROW_NUMBER() OVER (
                               PARTITION BY d.kind
                               ORDER BY CASE WHEN s.id IS NULL      THEN 2
                                             WHEN s.run_id = $2     THEN 0
                                             ELSE 1 END,
                                        d.version DESC) AS rank_in_kind
                      FROM document d
                      LEFT JOIN run_step s ON s.id = d.produced_by_step_id
                     WHERE d.item_id = $1
                       AND d.kind = ANY($3::text[])
                       AND (s.id IS NULL OR s.selected IS NOT FALSE)) ranked
             WHERE rank_in_kind = 1
            "#,
            item.as_uuid(),
            run.as_uuid(),
            &wanted[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let by_kind: BTreeMap<&str, &Document> = eligible
            .iter()
            .map(|document| (document.kind.as_str(), document))
            .collect();
        Ok(wanted
            .iter()
            .map(|kind| ResolvedInput {
                kind: kind.clone(),
                document: by_kind.get(kind.as_str()).map(|&found| found.clone()),
            })
            .collect())
    }

    // ---- ANA-11 §5.1 (MOD-38): stubs until T8 makes them real ----

    async fn requirement_spec(&self, _project: ProjectId) -> Result<Option<RequirementSpec>> {
        unimplemented!("MOD-38 T8")
    }

    async fn requirement_areas(&self, _project: ProjectId) -> Result<Vec<RequirementArea>> {
        unimplemented!("MOD-38 T8")
    }

    async fn requirements(
        &self,
        _project: ProjectId,
        _filter: &RequirementFilter,
    ) -> Result<Vec<Requirement>> {
        unimplemented!("MOD-38 T8")
    }

    async fn requirement(&self, _id: RequirementId) -> Result<Option<Requirement>> {
        unimplemented!("MOD-38 T8")
    }

    async fn requirement_revisions(
        &self,
        _id: RequirementId,
    ) -> Result<Option<Vec<RequirementRevision>>> {
        unimplemented!("MOD-38 T8")
    }

    async fn item_requirements(&self, _item: ItemId) -> Result<Vec<ItemCitation>> {
        unimplemented!("MOD-38 T8")
    }

    async fn requirement_coverage(&self, _requirement: RequirementId) -> Result<Vec<CoverageRow>> {
        unimplemented!("MOD-38 T8")
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
            -- Positional binding, so the order here is `BoxInfo`'s, not the table's. The two tag
            -- arrays are both `Vec<String>` and adjacent: transposing them type-checks and binds
            -- silently, which is why `pg_criteria.rs::the_0003_columns_reach_the_projections`
            -- seeds two *different* sets and names which is which (blueprint §3.5, T1 audit A-5).
            SELECT id            AS "box_id: htui_core::model::BoxId",
                   hostname,
                   os_family     AS "os_family: htui_core::model::OsFamily",
                   probed_tags,
                   declared_tags,
                   settings
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

    // -------------------------------------------------------------------------------------------
    // MOD-2 milestone 9's five prompt reads (`docs/ANA-5.md` §8, blueprint B.13).
    //
    // Inherent for the reason `agents` is, and a stronger one: `prompt_template`, `skill`,
    // `skill_version`, `skill_binding` and `box_tool` are absent from the mirrored table list
    // (`docs/ANA-9.md` §4.4), so no offline backend could answer them and `Backend`'s `Offline`
    // arm refuses with `PROMPT_ON_SERVER_ONLY` (plan D109). Each signature is `MemStore`'s to the
    // byte, which is what lets `Backend` dispatch over both without a third trait.
    // -------------------------------------------------------------------------------------------

    /// A project's `prompt_template` rows, ordered by `(name, version)` (`docs/ANA-5.md` §4.6).
    ///
    /// Every version, not the latest per name: a phase may pin `template_version`, and picking
    /// here would hide the pin. `COLLATE "C"` rather than a bare `ORDER BY name`, so the order is
    /// the byte order `MemStore` sorts by and not the database's collation — the same argument
    /// [`UpstreamEntry::sort_canonical`] makes, applied where a re-sort in Rust would be the only
    /// other answer.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn prompt_templates(&self, project: ProjectId) -> Result<Vec<PromptTemplate>> {
        sqlx::query_as!(
            PromptTemplate,
            r#"
            SELECT id         AS "id: htui_core::model::PromptTemplateId",
                   project_id AS "project_id: ProjectId",
                   name,
                   version,
                   body,
                   created_by AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM prompt_template
             WHERE project_id = $1
             ORDER BY name COLLATE "C", version
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The skills in force for a project, or for one phase of it: `R-SKL-2`'s collapse
    /// (`docs/ANA-5.md` §4.2), already resolved to a version and a body.
    ///
    /// Two `SELECT`s — the project level (`phase_id IS NULL`) and the phase level — and then
    /// [`BoundSkill::collapse`] in Rust, exactly as `MemStore` does. The override rule and the
    /// render order therefore have **one** definition rather than one per backend; expressing
    /// `R-SKL-2` a second time in SQL would be a copy to keep in step.
    /// [`SkillBinding::version_in_force`](htui_core::model::SkillBinding::version_in_force)
    /// resolves `pinned_version` against the same rows for the
    /// same reason, and a pin that names no `skill_version` drops the binding rather than
    /// rendering it bodiless.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn bound_skills(
        &self,
        project: ProjectId,
        phase: Option<PhaseId>,
    ) -> Result<Vec<BoundSkill>> {
        // Every version of every skill this project binds, once: `version_in_force` ignores
        // another skill's rows, so one statement answers every binding of both levels.
        let versions = sqlx::query_as!(
            SkillVersion,
            r#"
            SELECT v.skill_id  AS "skill_id: SkillId",
                   v.version,
                   v.body,
                   v.created_by AS "created_by: htui_core::model::UserId",
                   v.created_at
              FROM skill_version v
             WHERE v.skill_id IN (SELECT skill_id FROM skill_binding WHERE project_id = $1)
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let project_level = sqlx::query_as!(
            SkillBindingRow,
            r#"
            SELECT b.id         AS "id: htui_core::model::SkillBindingId",
                   b.skill_id   AS "skill_id: SkillId",
                   b.project_id AS "project_id: ProjectId",
                   b.phase_id   AS "phase_id: PhaseId",
                   b.pinned_version,
                   b.position,
                   b.updated_at,
                   s.name
              FROM skill_binding b JOIN skill s ON s.id = b.skill_id
             WHERE b.project_id = $1 AND b.phase_id IS NULL
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let phase_level = match phase {
            None => Vec::new(),
            Some(phase) => sqlx::query_as!(
                SkillBindingRow,
                r#"
                SELECT b.id         AS "id: htui_core::model::SkillBindingId",
                       b.skill_id   AS "skill_id: SkillId",
                       b.project_id AS "project_id: ProjectId",
                       b.phase_id   AS "phase_id: PhaseId",
                       b.pinned_version,
                       b.position,
                       b.updated_at,
                       s.name
                  FROM skill_binding b JOIN skill s ON s.id = b.skill_id
                 WHERE b.project_id = $1 AND b.phase_id = $2
                "#,
                project.as_uuid(),
                phase.as_uuid(),
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?,
        };

        let bind = |rows: Vec<SkillBindingRow>| -> Vec<BoundSkill> {
            rows.into_iter()
                .filter_map(|row| row.bind(&versions))
                .collect()
        };
        Ok(BoundSkill::collapse(bind(project_level), bind(phase_level)))
    }

    /// One box projected for the prompt's `box` section, or `None` when no row has that id.
    ///
    /// The `box_tool` join is [`BoxProfile::project`]'s: name byte order, capped at
    /// [`BoxProfile::MAX_TOOLS`], `path` dropped because §4.2 rule 5 forbids an absolute
    /// filesystem path anywhere in a prompt. The statement's `ORDER BY name` is readability only —
    /// the projection re-sorts by bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn box_profile(&self, id: BoxId) -> Result<Option<BoxProfile>> {
        let Some(row) = sqlx::query_as!(
            BoxRow,
            r#"
            SELECT id             AS "id: BoxId",
                   user_id        AS "user_id: htui_core::model::UserId",
                   hostname,
                   os_family      AS "os_family: htui_core::model::OsFamily",
                   os_version,
                   arch,
                   cpu,
                   ram_mb,
                   gpu_present,
                   gpu_vendor,
                   htui_version,
                   probed_tags,
                   declared_tags,
                   quirks,
                   settings,
                   registered_at,
                   last_seen_at,
                   last_probed_at,
                   updated_at
              FROM box WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        else {
            return Ok(None);
        };

        let tools = sqlx::query_as!(
            BoxTool,
            r#"
            SELECT box_id AS "box_id: BoxId", name, version, path, probed_at
              FROM box_tool WHERE box_id = $1 ORDER BY name
            "#,
            id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(Some(BoxProfile::project(&row, tools)))
    }

    /// Every `app_setting` row, keyed by name: the last rung of the prompt's settings chain
    /// (`docs/ANA-5.md` §4.4).
    ///
    /// A [`BTreeMap`] and not a [`HashMap`](std::collections::HashMap): the assembler records
    /// which rung answered, and a map iterated in hash order would make that record depend on the
    /// process's random state.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn app_settings(&self) -> Result<BTreeMap<String, Value>> {
        let rows = sqlx::query!(r#"SELECT key, value FROM app_setting ORDER BY key"#)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(|row| (row.key, row.value)).collect())
    }

    /// One `item_kind` row, or `None` when no row has that id.
    ///
    /// The prompt's `{{item}}` section names the kind and `item` carries only `kind_id`; §6.1
    /// returns the kind nowhere, so the assembler's caller reads it here (blueprint E-6).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn item_kind(&self, id: ItemKindId) -> Result<Option<ItemKind>> {
        sqlx::query_as!(
            ItemKind,
            r#"
            SELECT id               AS "id: ItemKindId",
                   project_id       AS "project_id: ProjectId",
                   prefix,
                   name,
                   description,
                   default_graph_id AS "default_graph_id: htui_core::model::StepGraphId",
                   position,
                   updated_at
              FROM item_kind WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    // -------------------------------------------------------------------------------------------
    // MOD-4 milestone 1: the eleven orchestration reads of plan D1 (blueprint F-N).
    //
    // Inherent and not [`ReadStore`] methods for the same reason MOD-2's five prompt reads are:
    // each one's table - `phase_agent`, `agent_box`, `repo_box_path`, `box` whole rather than
    // projected - is absent from the mirrored list of `docs/ANA-9.md` §4.4, so `CacheStore` has
    // nothing to answer from and [`Backend::Offline`] refuses them. `MemStore` carries a
    // like-named inherent for each so that `Backend`'s `match self` has three arms to dispatch
    // over, and `pg_criteria::inherent_orchestration_reads_answer_the_fixture` is what pins the
    // two answers together - there is no conformance case, because the suite reaches only the
    // traits.
    //
    // ANA-2 §8 names sixteen reads; five of them (`agents`, `app_settings`, `item_kind`,
    // `phases`, `repos`) already existed when MOD-4 began and two of those the shipped tree keeps
    // elsewhere. The shipped placement wins (plan D1), which is why this block is eleven long.
    // -------------------------------------------------------------------------------------------

    /// One `step_graph` row, or `None`.
    ///
    /// The statement is `step_graph_row`'s - MOD-15 already needed a
    /// graph by id for its compare-and-set follow-ups. This is the public name `Backend` dispatches
    /// to, so the two crates do not each grow a copy of the select list.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn step_graph(&self, id: StepGraphId) -> Result<Option<StepGraph>> {
        self.step_graph_row(id).await
    }

    /// A phase's candidate agents in `position` order (`R-ORCH-1`, `R-AGT-8`).
    ///
    /// `(phase_id, position)` is the table's primary key, so the order is total without a
    /// tie-break. `MemStore` answers empty here whatever the phase: it holds no `phase_agent`
    /// table, and the snapshot builder falls back to `project.settings.default_agent_id`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>> {
        sqlx::query_as!(
            PhaseAgent,
            r#"
            SELECT phase_id AS "phase_id: PhaseId",
                   position,
                   agent_id AS "agent_id: htui_core::model::AgentId",
                   model
              FROM phase_agent
             WHERE phase_id = $1
             ORDER BY position
            "#,
            phase.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One `prompt_template` by `(project, name)`: the pinned `version` when `version` is `Some`,
    /// else the highest (`docs/ANA-5.md` §4.6).
    ///
    /// A pin that names no row is `None` rather than the highest version - the same rule
    /// [`SkillBinding::version_in_force`](htui_core::model::SkillBinding::version_in_force) applies
    /// to the same shape of question - which is why the pin is a `WHERE` conjunct and not an
    /// `ORDER BY` preference.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn prompt_template(
        &self,
        project: ProjectId,
        name: &str,
        version: Option<i32>,
    ) -> Result<Option<PromptTemplate>> {
        sqlx::query_as!(
            PromptTemplate,
            r#"
            SELECT id         AS "id: htui_core::model::PromptTemplateId",
                   project_id AS "project_id: ProjectId",
                   name,
                   version,
                   body,
                   created_by AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM prompt_template
             WHERE project_id = $1 AND name = $2 AND ($3::int IS NULL OR version = $3)
             ORDER BY version DESC
             LIMIT 1
            "#,
            project.as_uuid(),
            name,
            version,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The graph an item runs under - its own `step_graph_id`, else its kind's default - with the
    /// phases in `position` order and each phase's candidate agents (ANA-2 §8).
    ///
    /// Three round trips plus one per phase rather than one join, because [`ResolvedGraph`] is a
    /// tree and `query_as!` builds rows: the alternative is a `jsonb_agg` the two backends would
    /// then have to decode alike. `None` when the item, its kind or the graph is gone, which is
    /// `MemStore`'s `?`-chain in SQL - the `COALESCE` resolves the graph id and the row read after
    /// it decides whether such a graph exists.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
        let Some(graph_id) = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(i.step_graph_id, k.default_graph_id)
                       AS "graph_id?: htui_core::model::StepGraphId"
              FROM item i JOIN item_kind k ON k.id = i.kind_id
             WHERE i.id = $1
            "#,
            item.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .flatten() else {
            return Ok(None);
        };

        let Some(graph) = self.step_graph_row(graph_id).await? else {
            return Ok(None);
        };

        let mut phases = Vec::new();
        for phase in self.phase_rows(graph_id).await? {
            let agents = self.phase_agents(phase.id).await?;
            phases.push(ResolvedPhase { phase, agents });
        }
        Ok(Some(ResolvedGraph { graph, phases }))
    }

    /// Every `agent_box` row of one box, in `agent_id` byte order.
    ///
    /// Postgres compares `uuid` as its sixteen bytes, which is `Uuid`'s own `Ord`, so the bare
    /// `ORDER BY` is `MemStore`'s `sort_by_key` and needs no collation.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>> {
        sqlx::query_as!(
            AgentBox,
            r#"
            SELECT agent_id AS "agent_id: htui_core::model::AgentId",
                   box_id   AS "box_id: BoxId",
                   enabled,
                   version,
                   path,
                   probed_at,
                   quota,
                   quota_at,
                   updated_at,
                   probe
              FROM agent_box
             WHERE box_id = $1
             ORDER BY agent_id
            "#,
            box_id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One whole `box` row, unlike [`box_info`](PgStore::box_info)'s three-column top-bar
    /// projection: §4.7's admission reads `settings` and `R-ORCH-10`'s matching reads both tag
    /// lists.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn box_row(&self, id: BoxId) -> Result<Option<BoxRow>> {
        sqlx::query_as!(
            BoxRow,
            r#"
            SELECT id             AS "id: BoxId",
                   user_id        AS "user_id: htui_core::model::UserId",
                   hostname,
                   os_family      AS "os_family: htui_core::model::OsFamily",
                   os_version,
                   arch,
                   cpu,
                   ram_mb,
                   gpu_present,
                   gpu_vendor,
                   htui_version,
                   probed_tags,
                   declared_tags,
                   quirks,
                   settings,
                   registered_at,
                   last_seen_at,
                   last_probed_at,
                   updated_at
              FROM box WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Every repo checkout path on one box, in `repo_id` byte order (`R-BOX-4`).
    ///
    /// The other half of `repo_box_path_rows`, which reads the same
    /// table by `repo_id` for the repo editor; the orchestrator asks the opposite question.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>> {
        sqlx::query_as!(
            RepoBoxPath,
            r#"
            SELECT repo_id AS "repo_id: RepoId",
                   box_id  AS "box_id: BoxId",
                   local_path,
                   updated_at
              FROM repo_box_path
             WHERE box_id = $1
             ORDER BY repo_id
            "#,
            box_id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The scope's ready items this box can actually take: §7.4's readiness, then `R-ORCH-10`'s
    /// capability half - `required_tags` a subset of the box's `probed_tags ∪ declared_tags`.
    ///
    /// The readiness conjunct and the ordering are [`items`](ReadStore::items)' own, copied rather
    /// than shared because `ItemFilter` has no field for a box and this predicate is the machine's
    /// vocabulary, not the caller's. The capability half is `NOT EXISTS (... t <> ALL (...))`,
    /// which is `<@` spelled so that the tag list drives it.
    ///
    /// The two `COALESCE`s are the unknown-box case and are **not** decoration: the `LEFT JOIN`
    /// leaves both arrays `NULL` when $2 names no box, `t <> ALL (NULL)` is `NULL` rather than
    /// true, the `WHERE` inside `EXISTS` does not hold, and every item - tagged or not - would come
    /// back ready. With `'{}'` in their place `t <> ALL ('{}')` is true, so a tagged item is
    /// excluded and an untagged one has nothing to unnest. That is `MemStore`'s empty
    /// `box_capabilities` for an unknown box, in SQL, and it is what the parity case caught.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn ready_items(
        &self,
        scope: &Scope,
        box_id: BoxId,
    ) -> Result<Vec<htui_core::model::ItemSummary>> {
        if scope.is_empty() {
            return Ok(Vec::new()); // `= ANY('{}')` is false for every row; skip the round trip.
        }
        let projects = project_uuids(scope);

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
                   i.updated_at,
                   i.touched_paths
              FROM item i LEFT JOIN box b ON b.id = $2
             WHERE i.project_id = ANY($1)
               AND i.status = 'open'
               AND NOT EXISTS (
                        SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id
                         WHERE l.from_item_id = i.id AND l.kind = 'blocked_by'
                           AND l.deleted_at IS NULL AND t.status NOT IN ('done','closed'))
               AND NOT EXISTS (
                        SELECT 1 FROM UNNEST(i.required_tags) t
                         WHERE t <> ALL (COALESCE(b.probed_tags, '{}')
                                      || COALESCE(b.declared_tags, '{}')))
             ORDER BY array_position($1, i.project_id), i.key_prefix, i.key_number
            "#,
            &projects[..],
            box_id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// The `required_tags` of one item the box has neither probed nor declared, in byte order:
    /// what the Backlog renders beside an item it cannot start here (`R-ORCH-10`).
    ///
    /// Unlike [`ready_items`](PgStore::ready_items) this one **refuses** an unknown id, and the
    /// item is looked up first: an empty answer has to mean "this box can take it", so "no such
    /// box" cannot be spelled the same way. The two `SELECT 1` probes run only when the main
    /// statement returns nothing, which is also the answer when every tag is covered - so the cost
    /// falls on the boring path, and `CROSS JOIN` needs no outer-join semantics.
    ///
    /// `COLLATE "C"` because the documented order is bytes: `MemStore` sorts `as_bytes()`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] for an unknown item (`"item"`) or box (`"box"`), the item first.
    pub async fn missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>> {
        let missing = sqlx::query_scalar!(
            r#"
            SELECT DISTINCT t COLLATE "C" AS "tag!"
              FROM item i CROSS JOIN box b, UNNEST(i.required_tags) t
             WHERE i.id = $1 AND b.id = $2
               AND t <> ALL (b.probed_tags || b.declared_tags)
             ORDER BY 1
            "#,
            item.as_uuid(),
            box_id.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if !missing.is_empty() {
            return Ok(missing);
        }

        if sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", item.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_none()
        {
            return Err(StoreError::NotFound {
                entity: "item",
                id: item.to_string(),
            });
        }
        if sqlx::query_scalar!("SELECT 1 FROM box WHERE id = $1", box_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .is_none()
        {
            return Err(StoreError::NotFound {
                entity: "box",
                id: box_id.to_string(),
            });
        }
        Ok(missing)
    }

    /// How many runs hold a slot on one box: §4.7's admission count, which is `running` **and**
    /// `awaiting_approval` and not `queued` - a queued run occupies nothing yet.
    ///
    /// Distinct from [`active_runs`](PgStore::active_runs), which counts a scope's live runs for
    /// the top bar and does count `queued`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn active_runs_on_box(&self, box_id: BoxId) -> Result<usize> {
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM run
             WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval')
            "#,
            box_id.as_uuid(),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Every active run whose `repo_scope` intersects `scope`, in `(queued_at, id)` order: what
    /// §4.7's overlap refusal names.
    ///
    /// An empty `scope` intersects nothing (hazard H-10): `'{}' && anything` is false, which is
    /// `MemStore`'s `any(|repo| scope.contains(repo))` over an empty haystack, so the caller that
    /// queues a scopeless run gets no refusal from here.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn overlapping_runs(&self, scope: &[RepoId]) -> Result<Vec<Run>> {
        let repos: Vec<Uuid> = scope.iter().map(|id| id.as_uuid()).collect();
        sqlx::query_as!(
            Run,
            r#"
            SELECT id               AS "id: RunId",
                   project_id       AS "project_id: ProjectId",
                   item_id          AS "item_id: ItemId",
                   kind             AS "kind: RunKind",
                   mode             AS "mode: RunMode",
                   status           AS "status: RunStatus",
                   target_box_id    AS "target_box_id: BoxId",
                   executing_box_id AS "executing_box_id: BoxId",
                   graph_snapshot,
                   started_by       AS "started_by: UserId",
                   queued_at,
                   started_at,
                   finished_at,
                   failure,
                   repo_scope       AS "repo_scope: Vec<RepoId>",
                   lease_box_id     AS "lease_box_id: BoxId",
                   lease_expires_at,
                   updated_at
              FROM run
             WHERE status IN ('queued','running','awaiting_approval')
               AND repo_scope && $1::uuid[]
             ORDER BY queued_at, id
            "#,
            &repos[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    // -------------------------------------------------------------------------------------------
    // MOD-15 milestone 1: the nine hierarchy reads of plan D1.
    //
    // They are [`WriteStore`](htui_core::store::WriteStore) methods, not inherent ones, so the
    // conformance suite can read back what it wrote on both backends — but the trait arm lives in
    // `pg/write.rs` with the other thirty, because one trait has one `impl` block. What is here is
    // the statement each arm delegates to, named as `MemStore`'s `State` readers are
    // (`workspace_project_rows`, `repo_rows`, ...) so the two sides read alike.
    //
    // `COLLATE "C"` wherever the documented order is bytes, for the reason
    // [`prompt_templates`](PgStore::prompt_templates) gives: `MemStore` sorts `name.as_bytes()` and
    // a bare `ORDER BY name` would follow the database's collation instead. `ORDER BY box_id` and
    // `project_id` need no collation — Postgres compares `uuid` as its sixteen bytes, which is
    // `Uuid`'s own `Ord`.
    // -------------------------------------------------------------------------------------------

    /// One `workspace` row, or `None` when no row has that id.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn workspace_row(&self, id: WorkspaceId) -> Result<Option<Workspace>> {
        sqlx::query_as!(
            Workspace,
            r#"
            SELECT id         AS "id: WorkspaceId",
                   slug,
                   name,
                   description,
                   created_by AS "created_by: htui_core::model::UserId",
                   created_at,
                   updated_at
              FROM workspace WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// A workspace's `workspace_project` links, ordered by `position` then `project_id` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn workspace_project_rows(
        &self,
        workspace: WorkspaceId,
    ) -> Result<Vec<WorkspaceProject>> {
        sqlx::query_as!(
            WorkspaceProject,
            r#"
            SELECT workspace_id AS "workspace_id: WorkspaceId",
                   project_id   AS "project_id: ProjectId",
                   position
              FROM workspace_project
             WHERE workspace_id = $1
             ORDER BY position, project_id
            "#,
            workspace.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Every box's root path for a workspace, ordered by `box_id` bytes (`R-BOX-4`).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn workspace_box_path_rows(
        &self,
        workspace: WorkspaceId,
    ) -> Result<Vec<WorkspaceBoxPath>> {
        sqlx::query_as!(
            WorkspaceBoxPath,
            r#"
            SELECT workspace_id AS "workspace_id: WorkspaceId",
                   box_id       AS "box_id: BoxId",
                   root_path,
                   updated_at
              FROM workspace_box_path
             WHERE workspace_id = $1
             ORDER BY box_id
            "#,
            workspace.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One `repo` row by id: the follow-up read that tells a spent token from an absent row after
    /// a compare-and-set touched nothing (plan D3).
    ///
    /// Not on the trait — nothing reads a repo by id but the writer that just failed to write it,
    /// and [`repo_rows`](PgStore::repo_rows) is what the editor lists.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn repo_row(&self, id: RepoId) -> Result<Option<Repo>> {
        sqlx::query_as!(
            Repo,
            r#"
            SELECT id         AS "id: RepoId",
                   project_id AS "project_id: ProjectId",
                   name,
                   remote_url,
                   default_branch,
                   is_primary,
                   created_at,
                   updated_at
              FROM repo WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One `step_graph` row by id, for [`repo_row`](PgStore::repo_row)'s reason.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn step_graph_row(&self, id: StepGraphId) -> Result<Option<StepGraph>> {
        sqlx::query_as!(
            StepGraph,
            r#"
            SELECT id         AS "id: StepGraphId",
                   project_id AS "project_id: ProjectId",
                   name,
                   description,
                   -- Inserted between `description` and `created_at`, never appended:
                   -- `is_override` sits mid-struct and `query_as!` binds positionally, so an
                   -- appended column would map `created_at` onto it and type-check (T1 audit A-4).
                   is_override,
                   created_at,
                   updated_at
              FROM step_graph WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One `step_graph_phase` row by id, for [`repo_row`](PgStore::repo_row)'s reason.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn phase_row(&self, id: PhaseId) -> Result<Option<StepGraphPhase>> {
        sqlx::query_as!(
            StepGraphPhase,
            r#"
            SELECT id               AS "id: PhaseId",
                   graph_id         AS "graph_id: StepGraphId",
                   position,
                   name,
                   fan_out,
                   gate             AS "gate: Gate",
                   gate_hard,
                   retry_limit,
                   input_kinds,
                   output_kind,
                   isolation        AS "isolation: Isolation",
                   command_queue    AS "command_queue: CommandQueue",
                   verify_command,
                   template_name,
                   template_version,
                   token_budget,
                   updated_at
              FROM step_graph_phase WHERE id = $1
            "#,
            id.as_uuid(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// A project's repos, ordered by `name` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn repo_rows(&self, project: ProjectId) -> Result<Vec<Repo>> {
        sqlx::query_as!(
            Repo,
            r#"
            SELECT id         AS "id: RepoId",
                   project_id AS "project_id: ProjectId",
                   name,
                   remote_url,
                   default_branch,
                   is_primary,
                   created_at,
                   updated_at
              FROM repo
             WHERE project_id = $1
             ORDER BY name COLLATE "C"
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// Every box's checkout path for a repo, ordered by `box_id` bytes (`R-BOX-4`).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn repo_box_path_rows(&self, repo: RepoId) -> Result<Vec<RepoBoxPath>> {
        sqlx::query_as!(
            RepoBoxPath,
            r#"
            SELECT repo_id AS "repo_id: RepoId",
                   box_id  AS "box_id: BoxId",
                   local_path,
                   updated_at
              FROM repo_box_path
             WHERE repo_id = $1
             ORDER BY box_id
            "#,
            repo.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// A project's kinds, ordered by `position` then `prefix` bytes; `position` is not unique.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn item_kind_rows(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
        sqlx::query_as!(
            ItemKind,
            r#"
            SELECT id               AS "id: ItemKindId",
                   project_id       AS "project_id: ProjectId",
                   prefix,
                   name,
                   description,
                   default_graph_id AS "default_graph_id: StepGraphId",
                   position,
                   updated_at
              FROM item_kind
             WHERE project_id = $1
             ORDER BY position, prefix COLLATE "C"
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// A project's graphs, ordered by `name` bytes.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn step_graph_rows(&self, project: ProjectId) -> Result<Vec<StepGraph>> {
        sqlx::query_as!(
            StepGraph,
            r#"
            SELECT id         AS "id: StepGraphId",
                   project_id AS "project_id: ProjectId",
                   name,
                   description,
                   -- Inserted between `description` and `created_at`, never appended:
                   -- `is_override` sits mid-struct and `query_as!` binds positionally, so an
                   -- appended column would map `created_at` onto it and type-check (T1 audit A-4).
                   is_override,
                   created_at,
                   updated_at
              FROM step_graph
             WHERE project_id = $1
             ORDER BY name COLLATE "C"
            "#,
            project.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// A graph's phases, ordered by `position`, which is unique per graph.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn phase_rows(&self, graph: StepGraphId) -> Result<Vec<StepGraphPhase>> {
        sqlx::query_as!(
            StepGraphPhase,
            r#"
            SELECT id               AS "id: PhaseId",
                   graph_id         AS "graph_id: StepGraphId",
                   position,
                   name,
                   fan_out,
                   gate             AS "gate: Gate",
                   gate_hard,
                   retry_limit,
                   input_kinds,
                   output_kind,
                   isolation        AS "isolation: Isolation",
                   command_queue    AS "command_queue: CommandQueue",
                   verify_command,
                   template_name,
                   template_version,
                   token_budget,
                   updated_at
              FROM step_graph_phase
             WHERE graph_id = $1
             ORDER BY position
            "#,
            graph.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    /// One setting on one rung with the rung row's CAS token, or `None` when that row is absent
    /// (plan D8).
    ///
    /// The three rungs are three tables: an `app_setting` row keyed by
    /// [`SettingKey::key`](htui_core::prompt::settings::SettingKey::key), one key of
    /// `project.settings` named by
    /// [`SettingSpec::project_key`](htui_core::prompt::settings::SettingSpec::project_key) (flag
    /// A), and `step_graph_phase.token_budget`. A project or phase row that exists without a value
    /// answers `Some` with `value: None`, which is a different fact from the row not existing:
    /// the first means the rung below answers, the second that there is no rung here at all.
    ///
    /// The rung check is **not** here — [`WriteStore::setting`](htui_core::store::WriteStore::setting)
    /// makes it, so a writer that has already made it does not pay for it twice.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub(crate) async fn stored_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
    ) -> Result<Option<StoredSetting>> {
        match rung {
            SettingRung::App => {
                let row = sqlx::query!(
                    "SELECT value, updated_at FROM app_setting WHERE key = $1",
                    key.key(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                Ok(row.map(|row| StoredSetting {
                    value: Some(row.value),
                    updated_at: row.updated_at,
                }))
            }
            SettingRung::Project(id) => {
                let row = sqlx::query!(
                    "SELECT settings, updated_at FROM project WHERE id = $1",
                    id.as_uuid(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                Ok(row.map(|row| StoredSetting {
                    value: key
                        .spec()
                        .project_key
                        .and_then(|name| row.settings.get(name).cloned()),
                    updated_at: row.updated_at,
                }))
            }
            SettingRung::Phase(id) => {
                let row = sqlx::query!(
                    "SELECT token_budget, updated_at FROM step_graph_phase WHERE id = $1",
                    id.as_uuid(),
                )
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
                Ok(row.map(|row| StoredSetting {
                    value: row.token_budget.map(Value::from),
                    updated_at: row.updated_at,
                }))
            }
        }
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
