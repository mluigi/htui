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
    BoxTool, Document, DocumentHead, DocumentId, Item, ItemFilter, ItemId, ItemKind, ItemKindId,
    LinkEdge, LinkGraph, LinkKind, Note, PhaseId, Project, ProjectId, ProjectRef, PromptScope,
    PromptTemplate, RunId, RunStepSummary, RunSummary, Scope, SessionEvent, SkillId, SkillVersion,
    StepId, UpstreamEntry, WorkspaceSummary,
};
use htui_core::store::{ReadStore, Result, StoreError};
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
                   END                                              AS "trimmed!"
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
            scope.workspace.map(htui_core::model::WorkspaceId::as_uuid),
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
