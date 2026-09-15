//! `impl ReadStore for CacheStore` plus the seven inherent reads (blueprint C.12; the last three
//! are MOD-2 milestone 4's).
//!
//! The SQL is the Postgres text of C.7 / C.8 with three mechanical substitutions, and nothing
//! else:
//!
//! 1. `= ANY($1)` becomes `IN (<n placeholders>)`, built at run time (SQLite has no array type),
//!    and `array_position($1, i.project_id)` becomes the equivalent `CASE` over the same ids;
//! 2. `i.required_tags @> $4` becomes one `EXISTS` over `json_each(i.required_tags)` per wanted
//!    tag;
//! 3. `AND l.deleted_at IS NULL` disappears - a tombstone is already gone from the mirror (C.11).
//!
//! Everything else is identical, ordering rules included: `ORDER BY kind, version`,
//! `ORDER BY created_at, id`, `ORDER BY queued_at DESC`, `ORDER BY position, attempt,
//! fanout_index` and `ORDER BY seq`, because ANA-9 §11.4 compares a mirror read with a Postgres
//! read row for row.
//!
//! Queries here are **runtime-checked** `sqlx::query` with `.bind(..)`, never `query!`: one crate
//! has one `DATABASE_URL` for the macros and it is the Postgres one (plan D2). Nothing in this
//! file contributes a `.sqlx/query-*.json`.
//!
//! # Decoding
//!
//! Every helper below exists because the sqlx 0.9 SQLite driver would otherwise get the column
//! silently wrong; none of them is optional (blueprint H.9, H.10). Enums need no helper: the
//! `str_enum!` derive emits a SQLite `Type`/`Decode` over `str`.

use chrono::{DateTime, Utc};
use htui_core::model::{
    Agent, AgentId, AgentSummary, Billing, BoxId, BoxInfo, Document, DocumentHead, DocumentId,
    EventKind, EventRole, GateOutcome, Item, ItemFilter, ItemId, ItemSummary, LinkEdge, LinkGraph,
    LinkKind, LinkNode, Note, NoteId, OsFamily, Project, ProjectId, ProjectRef, PromptScope, RunId,
    RunKind, RunMode, RunStatus, RunStepSummary, RunSummary, Scope, SessionEvent, Status,
    StepGraphId, StepId, StepStatus, Transport, UpstreamEntry, UserId, WorkspaceId,
    WorkspaceSummary,
};
use htui_core::store::{ReadStore, Result, StoreError};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, Row as _};
use uuid::Uuid;

use super::CacheStore;
use crate::error::map_sqlx;
use crate::{MAX_UPSTREAM_HOPS, order_documents};

// ------------------------------------------------------------------------------------------------
// Decoding helpers
// ------------------------------------------------------------------------------------------------

/// TEXT -> a UUID newtype.
///
/// `Uuid: Type<Sqlite>` is a **BLOB** in sqlx 0.9 (`sqlx-sqlite-0.9.0/src/types/uuid.rs`: `Decode`
/// calls `value.blob_borrowed()`), so a TEXT UUID column can never be decoded straight into `Uuid`
/// or into a transparent newtype over it (blueprint H.9).
pub(crate) fn uuid_col<T: From<Uuid>>(column: &str, text: &str) -> Result<T> {
    Uuid::parse_str(text)
        .map(T::from)
        .map_err(|_| bad(column, "a UUID"))
}

/// [`uuid_col`] over a nullable column.
pub(crate) fn opt_uuid_col<T: From<Uuid>>(column: &str, text: Option<&str>) -> Result<Option<T>> {
    text.map(|text| uuid_col(column, text)).transpose()
}

/// INTEGER microseconds -> `DateTime<Utc>`.
///
/// sqlx 0.9 decodes an INTEGER datetime as **seconds**
/// (`sqlx-sqlite-0.9.0/src/types/chrono.rs::decode_datetime_from_int` -> `timestamp_opt(v, 0)`)
/// and encodes a `DateTime` as RFC-3339 **text**, so both directions must be explicit. This one is
/// silent - a wrong-by-a-million timestamp decodes without an error - which makes it the single
/// most important helper here (blueprint H.10).
pub(crate) fn ts_col(column: &str, micros: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp_micros(micros).ok_or_else(|| bad(column, "a timestamp"))
}

/// [`ts_col`] over a nullable column.
pub(crate) fn opt_ts_col(column: &str, micros: Option<i64>) -> Result<Option<DateTime<Utc>>> {
    micros.map(|micros| ts_col(column, micros)).transpose()
}

/// `DateTime<Utc>` -> INTEGER microseconds, the only way a timestamp is written to the mirror.
#[must_use]
pub(crate) fn ts_bind(at: DateTime<Utc>) -> i64 {
    at.timestamp_micros()
}

/// TEXT holding a JSON array -> `Vec<String>` (`required_tags`, `touched_paths`, `probed_tags`).
pub(crate) fn strings_col(column: &str, text: &str) -> Result<Vec<String>> {
    serde_json::from_str(text).map_err(|_| bad(column, "a JSON array of strings"))
}

/// TEXT holding JSON -> `serde_json::Value` (`payload`, `raw`, `settings`, `graph_snapshot`,
/// `trim_record`, `usage`).
pub(crate) fn json_col(column: &str, text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|_| bad(column, "JSON"))
}

/// [`json_col`] over a nullable column.
pub(crate) fn opt_json_col(column: &str, text: Option<&str>) -> Result<Option<Value>> {
    text.map(|text| json_col(column, text)).transpose()
}

/// INTEGER 0/1 -> bool.
///
/// Written by MOD-6 ahead of its first reader, because `INTEGER` has no truthiness of its own and
/// re-deriving the conversion per call site is how `bool` on SQLite gets mirrored wrong.
/// [`CacheStore::agents`]'s `agent.enabled` is that first reader (MOD-2 milestone 4).
#[must_use]
pub(crate) const fn bool_col(v: i64) -> bool {
    v != 0
}

/// A corrupt mirror degrades to an error the TUI can show, never to a panic: the rebuild path of
/// [`CacheStore::rebuild`] exists for exactly this.
fn bad(column: &str, shape: &str) -> StoreError {
    StoreError::Backend(format!("cache: {column} is not {shape}"))
}

/// `?, ?, ?` for `n` bound values, or `NULL` when `n` is zero (`IN ()` is a syntax error and
/// `IN (NULL)` matches nothing, which is what an empty list means).
fn placeholders(n: usize) -> String {
    if n == 0 {
        return "NULL".to_owned();
    }
    let mut out = String::with_capacity(n * 3);
    for i in 0..n {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('?');
    }
    out
}

// ------------------------------------------------------------------------------------------------
// Row -> model
// ------------------------------------------------------------------------------------------------

/// One `item` row, every column of §5.5.
fn item_of(row: &SqliteRow) -> Result<Item> {
    Ok(Item {
        id: uuid_col("item.id", &text(row, "id")?)?,
        project_id: uuid_col("item.project_id", &text(row, "project_id")?)?,
        kind_id: uuid_col("item.kind_id", &text(row, "kind_id")?)?,
        key_prefix: text(row, "key_prefix")?,
        key_number: get(row, "key_number")?,
        key: text(row, "key")?,
        title: text(row, "title")?,
        body: text(row, "body")?,
        status: get::<Status>(row, "status")?,
        priority: get(row, "priority")?,
        required_tags: strings_col("item.required_tags", &text(row, "required_tags")?)?,
        touched_paths: strings_col("item.touched_paths", &text(row, "touched_paths")?)?,
        step_graph_id: opt_uuid_col::<StepGraphId>(
            "item.step_graph_id",
            opt_text(row, "step_graph_id")?.as_deref(),
        )?,
        version: get(row, "version")?,
        created_by: uuid_col::<UserId>("item.created_by", &text(row, "created_by")?)?,
        created_at: ts_col("item.created_at", get(row, "created_at")?)?,
        updated_at: ts_col("item.updated_at", get(row, "updated_at")?)?,
        closed_at: opt_ts_col("item.closed_at", get(row, "closed_at")?)?,
    })
}

/// The list projection of one `item` row.
fn item_summary_of(row: &SqliteRow) -> Result<ItemSummary> {
    Ok(ItemSummary {
        id: uuid_col("item.id", &text(row, "id")?)?,
        project_id: uuid_col("item.project_id", &text(row, "project_id")?)?,
        kind_id: uuid_col("item.kind_id", &text(row, "kind_id")?)?,
        key: text(row, "key")?,
        key_prefix: text(row, "key_prefix")?,
        key_number: get(row, "key_number")?,
        title: text(row, "title")?,
        status: get::<Status>(row, "status")?,
        priority: get(row, "priority")?,
        required_tags: strings_col("item.required_tags", &text(row, "required_tags")?)?,
        updated_at: ts_col("item.updated_at", get(row, "updated_at")?)?,
    })
}

/// One `document` row **with its body**, every column of §5.5's `document`.
fn document_of(row: &SqliteRow) -> Result<Document> {
    Ok(Document {
        id: uuid_col("document.id", &text(row, "id")?)?,
        item_id: uuid_col("document.item_id", &text(row, "item_id")?)?,
        kind: text(row, "kind")?,
        version: get(row, "version")?,
        title: text(row, "title")?,
        body: text(row, "body")?,
        produced_by_step_id: opt_uuid_col::<StepId>(
            "document.produced_by_step_id",
            opt_text(row, "produced_by_step_id")?.as_deref(),
        )?,
        created_by: uuid_col::<UserId>("document.created_by", &text(row, "created_by")?)?,
        created_at: ts_col("document.created_at", get(row, "created_at")?)?,
    })
}

/// A `TEXT NOT NULL` column.
fn text(row: &SqliteRow, column: &str) -> Result<String> {
    get(row, column)
}

/// A `TEXT` column that may be null.
fn opt_text(row: &SqliteRow, column: &str) -> Result<Option<String>> {
    get(row, column)
}

/// One column, with the driver error mapped.
fn get<'r, T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>>(
    row: &'r SqliteRow,
    column: &str,
) -> Result<T> {
    row.try_get(column).map_err(map_sqlx)
}

// ------------------------------------------------------------------------------------------------
// The seven `ReadStore` methods
// ------------------------------------------------------------------------------------------------

impl ReadStore for CacheStore {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> Result<Vec<ItemSummary>> {
        if scope.project_ids.is_empty() {
            // An empty workspace reads as an empty backlog, not as "every project" (`Scope`).
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            "SELECT i.id, i.project_id, i.kind_id, i.key, i.key_prefix, i.key_number, i.title, \
             i.status, i.priority, i.required_tags, i.updated_at FROM item i WHERE i.project_id IN (",
        );
        sql.push_str(&placeholders(scope.project_ids.len()));
        sql.push(')');

        // `ItemFilter::project_ids` narrows the scope, it never widens it (blueprint H.16).
        if let Some(ids) = &filter.project_ids {
            sql.push_str(" AND i.project_id IN (");
            sql.push_str(&placeholders(ids.len()));
            sql.push(')');
        }
        if let Some(statuses) = &filter.statuses {
            sql.push_str(" AND i.status IN (");
            sql.push_str(&placeholders(statuses.len()));
            sql.push(')');
        }
        if let Some(tags) = &filter.tags {
            for _ in tags {
                sql.push_str(
                    " AND EXISTS (SELECT 1 FROM json_each(i.required_tags) t WHERE t.value = ?)",
                );
            }
        }
        if filter.text.is_some() {
            // SQLite has no ILIKE; `instr(lower(..), lower(..))` is the case-insensitive substring
            // match `MemStore` does with `to_lowercase().contains()`, without `%`/`_` escaping.
            sql.push_str(
                " AND (instr(lower(i.key), lower(?)) > 0 OR instr(lower(i.title), lower(?)) > 0)",
            );
        }
        if filter.ready.is_some() {
            // ANA-9 §7.4's store-side half: open, and no live `blocked_by` edge to a non-terminal
            // item. `deleted_at` is absent because a tombstone is already gone from the mirror.
            sql.push_str(
                " AND ? = (CASE WHEN i.status = 'open' AND NOT EXISTS (\
                 SELECT 1 FROM item_link l JOIN item t ON t.id = l.to_item_id \
                 WHERE l.from_item_id = i.id AND l.kind = 'blocked_by' \
                 AND t.status NOT IN ('done','closed')) THEN 1 ELSE 0 END)",
            );
        }

        // `array_position($1, i.project_id)`, spelled out: the scope order is the Backlog order.
        sql.push_str(" ORDER BY CASE i.project_id");
        for _ in &scope.project_ids {
            sql.push_str(" WHEN ? THEN ?");
        }
        sql.push_str(" ELSE ? END, i.key_prefix, i.key_number");

        let mut query = sqlx::query(AssertSqlSafe(sql));
        for id in &scope.project_ids {
            query = query.bind(id.to_string());
        }
        if let Some(ids) = &filter.project_ids {
            for id in ids {
                query = query.bind(id.to_string());
            }
        }
        if let Some(statuses) = &filter.statuses {
            for status in statuses {
                query = query.bind(status.as_str());
            }
        }
        if let Some(tags) = &filter.tags {
            for tag in tags {
                query = query.bind(tag.clone());
            }
        }
        if let Some(needle) = &filter.text {
            query = query.bind(needle.clone()).bind(needle.clone());
        }
        if let Some(ready) = filter.ready {
            query = query.bind(i64::from(ready));
        }
        for (position, id) in scope.project_ids.iter().enumerate() {
            query = query.bind(id.to_string()).bind(position as i64);
        }
        query = query.bind(scope.project_ids.len() as i64);

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx)?;
        rows.iter().map(item_summary_of).collect()
    }

    async fn item(&self, id: ItemId) -> Result<Option<Item>> {
        let row = sqlx::query(
            "SELECT id, project_id, kind_id, key_prefix, key_number, key, title, body, status, \
             priority, required_tags, touched_paths, step_graph_id, version, created_by, \
             created_at, updated_at, closed_at FROM item WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(item_of).transpose()
    }

    async fn links(&self, id: ItemId, hops: u8) -> Result<LinkGraph> {
        // `UNION` (not `UNION ALL`) plus the `depth < ?` guard is what terminates the walk on a
        // cycle; `MIN(depth)` per node is the breadth-first depth `MemStore` produces.
        let nodes = sqlx::query(
            "WITH RECURSIVE walk(item_id, depth) AS ( \
                 SELECT ?, 0 \
               UNION \
                 SELECT CASE WHEN l.from_item_id = w.item_id THEN l.to_item_id \
                             ELSE l.from_item_id END, w.depth + 1 \
                   FROM walk w \
                   JOIN item_link l ON (l.from_item_id = w.item_id OR l.to_item_id = w.item_id) \
                  WHERE w.depth < ? \
             ), \
             node AS (SELECT item_id, MIN(depth) AS depth FROM walk GROUP BY item_id) \
             SELECT n.item_id, i.project_id, p.slug AS project_slug, i.key, i.title, i.status, \
                    n.depth \
               FROM node n \
               JOIN item i    ON i.id = n.item_id \
               JOIN project p ON p.id = i.project_id \
              ORDER BY n.depth, i.key",
        )
        .bind(id.to_string())
        .bind(i64::from(hops))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if nodes.is_empty() {
            // Either the root does not exist, or its project is not mirrored; both are `NotFound`
            // for a read that must return the root at depth 0.
            return Err(StoreError::NotFound {
                entity: "item",
                id: id.to_string(),
            });
        }

        let mut reached = Vec::with_capacity(nodes.len());
        let mut out = Vec::with_capacity(nodes.len());
        for row in &nodes {
            let item_id = text(row, "item_id")?;
            reached.push(item_id.clone());
            out.push(LinkNode {
                item_id: uuid_col("item_link.item_id", &item_id)?,
                project_id: uuid_col("item.project_id", &text(row, "project_id")?)?,
                project_slug: text(row, "project_slug")?,
                key: text(row, "key")?,
                title: text(row, "title")?,
                status: get::<Status>(row, "status")?,
                depth: u8::try_from(get::<i64>(row, "depth")?).unwrap_or(u8::MAX),
            });
        }

        let sql = format!(
            "SELECT from_item_id, to_item_id, kind FROM item_link \
              WHERE from_item_id IN ({placeholders}) AND to_item_id IN ({placeholders}) \
              ORDER BY from_item_id, to_item_id, kind",
            placeholders = placeholders(reached.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for item_id in reached.iter().chain(reached.iter()) {
            query = query.bind(item_id.clone());
        }
        let edge_rows = query.fetch_all(&self.pool).await.map_err(map_sqlx)?;

        let mut edges = Vec::with_capacity(edge_rows.len());
        for row in &edge_rows {
            edges.push(LinkEdge {
                from_item_id: uuid_col("item_link.from_item_id", &text(row, "from_item_id")?)?,
                to_item_id: uuid_col("item_link.to_item_id", &text(row, "to_item_id")?)?,
                kind: get::<LinkKind>(row, "kind")?,
            });
        }

        Ok(LinkGraph {
            root: id,
            nodes: out,
            edges,
        })
    }

    async fn documents(&self, id: ItemId) -> Result<Vec<DocumentHead>> {
        let rows = sqlx::query(
            "SELECT id, item_id, kind, version, title, produced_by_step_id, created_by, \
             created_at FROM document WHERE item_id = ? ORDER BY kind, version",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        rows.iter()
            .map(|row| {
                Ok(DocumentHead {
                    id: uuid_col::<DocumentId>("document.id", &text(row, "id")?)?,
                    item_id: uuid_col("document.item_id", &text(row, "item_id")?)?,
                    kind: text(row, "kind")?,
                    version: get(row, "version")?,
                    title: text(row, "title")?,
                    produced_by_step_id: opt_uuid_col::<StepId>(
                        "document.produced_by_step_id",
                        opt_text(row, "produced_by_step_id")?.as_deref(),
                    )?,
                    created_by: uuid_col::<UserId>(
                        "document.created_by",
                        &text(row, "created_by")?,
                    )?,
                    created_at: ts_col("document.created_at", get(row, "created_at")?)?,
                })
            })
            .collect()
    }

    async fn notes(&self, id: ItemId) -> Result<Vec<Note>> {
        let rows = sqlx::query(
            "SELECT id, item_id, body, created_by, box_id, via_step_id, created_at \
               FROM item_note WHERE item_id = ? ORDER BY created_at, id",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        rows.iter()
            .map(|row| {
                Ok(Note {
                    id: uuid_col::<NoteId>("item_note.id", &text(row, "id")?)?,
                    item_id: uuid_col("item_note.item_id", &text(row, "item_id")?)?,
                    body: text(row, "body")?,
                    created_by: uuid_col::<UserId>(
                        "item_note.created_by",
                        &text(row, "created_by")?,
                    )?,
                    box_id: opt_uuid_col::<BoxId>(
                        "item_note.box_id",
                        opt_text(row, "box_id")?.as_deref(),
                    )?,
                    via_step_id: opt_uuid_col::<StepId>(
                        "item_note.via_step_id",
                        opt_text(row, "via_step_id")?.as_deref(),
                    )?,
                    created_at: ts_col("item_note.created_at", get(row, "created_at")?)?,
                })
            })
            .collect()
    }

    async fn runs(&self, id: ItemId) -> Result<Vec<RunSummary>> {
        let run_rows = sqlx::query(
            "SELECT r.id, r.item_id, r.project_id, r.kind, r.mode, r.status, r.target_box_id, \
                    r.executing_box_id, COALESCE(b.hostname, '') AS box_hostname, \
                    r.queued_at, r.started_at, r.finished_at, r.failure \
               FROM run r \
               LEFT JOIN box b ON b.id = COALESCE(r.executing_box_id, r.target_box_id) \
              WHERE r.item_id = ? \
              ORDER BY r.queued_at DESC",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if run_rows.is_empty() {
            return Ok(Vec::new());
        }

        // Two statements, not a `LEFT JOIN` with a row per step: a run with no steps must still
        // appear, and the `RunSummary` shape is a nested one.
        let ids: Vec<String> = run_rows
            .iter()
            .map(|row| text(row, "id"))
            .collect::<Result<_>>()?;
        let sql = format!(
            // The last two columns are plan D106's figures, the same derivation the Postgres
            // statement makes and `htui_core::model::prompt_summary` makes in Rust. `max` over
            // `json_each` is SQLite's `bool_or`: there is no such aggregate, and the values are
            // the integers 0 and 1.
            //
            // The `json_type` guards are what make it total (T68, F-52), because `trim_record` is
            // untyped: a `"34000"` or a `35988.5` decodes as neither `i64` nor `NULL`, a
            // `sections` that is not an array makes `json_each` walk an object's members, and a
            // non-object member of one makes it raise `malformed JSON` — which would fail the
            // whole Runs read over a single bad row. `prompt_summary` answers `(None, false)` to
            // every one of them, and `'true'` rather than a truthy `json_extract` is its
            // `Value::as_bool`: the integer `1` is not the JSON `true`.
            "SELECT run_id, id, position, attempt, fanout_index, phase_name, agent_id, model, \
                    status, gate_outcome, started_at, finished_at, \
                    CASE WHEN json_type(trim_record, '$.estimated_after') = 'integer' \
                         THEN json_extract(trim_record, '$.estimated_after') END AS prompt_tokens, \
                    CASE WHEN json_type(trim_record, '$.sections') = 'array' \
                         THEN COALESCE((SELECT max(CASE WHEN e.type = 'object' \
                                                     AND json_type(e.value, '$.trimmed') = 'true' \
                                                    THEN 1 ELSE 0 END) \
                                          FROM json_each(run_step.trim_record, '$.sections') e), 0) \
                         ELSE 0 END AS trimmed \
               FROM run_step WHERE run_id IN ({}) \
              ORDER BY run_id, position, attempt, fanout_index",
            placeholders(ids.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for run_id in &ids {
            query = query.bind(run_id.clone());
        }
        let step_rows = query.fetch_all(&self.pool).await.map_err(map_sqlx)?;

        let mut steps: Vec<(String, RunStepSummary)> = Vec::with_capacity(step_rows.len());
        for row in &step_rows {
            steps.push((
                text(row, "run_id")?,
                RunStepSummary {
                    id: uuid_col("run_step.id", &text(row, "id")?)?,
                    position: get(row, "position")?,
                    attempt: get(row, "attempt")?,
                    fanout_index: get(row, "fanout_index")?,
                    phase_name: text(row, "phase_name")?,
                    agent_id: opt_uuid_col(
                        "run_step.agent_id",
                        opt_text(row, "agent_id")?.as_deref(),
                    )?,
                    model: opt_text(row, "model")?,
                    status: get::<StepStatus>(row, "status")?,
                    gate_outcome: get::<Option<GateOutcome>>(row, "gate_outcome")?,
                    started_at: opt_ts_col("run_step.started_at", get(row, "started_at")?)?,
                    finished_at: opt_ts_col("run_step.finished_at", get(row, "finished_at")?)?,
                    // A figure outside `i32` is no figure rather than a truncated one, which is
                    // what `prompt_summary` does with the same value in Rust.
                    prompt_tokens: get::<Option<i64>>(row, "prompt_tokens")?
                        .and_then(|tokens| i32::try_from(tokens).ok()),
                    trimmed: bool_col(get::<i64>(row, "trimmed")?),
                },
            ));
        }

        run_rows
            .iter()
            .zip(ids.iter())
            .map(|(row, run_id)| {
                Ok(RunSummary {
                    id: uuid_col::<RunId>("run.id", run_id)?,
                    item_id: opt_uuid_col("run.item_id", opt_text(row, "item_id")?.as_deref())?,
                    project_id: uuid_col("run.project_id", &text(row, "project_id")?)?,
                    kind: get::<RunKind>(row, "kind")?,
                    mode: get::<RunMode>(row, "mode")?,
                    status: get::<RunStatus>(row, "status")?,
                    target_box_id: uuid_col("run.target_box_id", &text(row, "target_box_id")?)?,
                    executing_box_id: opt_uuid_col(
                        "run.executing_box_id",
                        opt_text(row, "executing_box_id")?.as_deref(),
                    )?,
                    box_hostname: text(row, "box_hostname")?,
                    queued_at: ts_col("run.queued_at", get(row, "queued_at")?)?,
                    started_at: opt_ts_col("run.started_at", get(row, "started_at")?)?,
                    finished_at: opt_ts_col("run.finished_at", get(row, "finished_at")?)?,
                    failure: opt_text(row, "failure")?,
                    steps: steps
                        .iter()
                        .filter(|(owner, _)| owner == run_id)
                        .map(|(_, step)| step.clone())
                        .collect(),
                })
            })
            .collect()
    }

    async fn step_events(&self, step: StepId) -> Result<Option<Vec<SessionEvent>>> {
        let rows = sqlx::query(
            "SELECT run_step_id, seq, turn, kind, role, tool_call_id, payload, raw, at \
               FROM session_event WHERE run_step_id = ? ORDER BY seq",
        )
        .bind(step.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        // No rows is "not cached", which is the §6.1 contract (`None` = not cached): a step
        // outside the last N of §4.4 is never an error and never an empty `Vec`.
        if rows.is_empty() {
            return Ok(None);
        }

        rows.iter()
            .map(|row| {
                Ok(SessionEvent {
                    run_step_id: uuid_col("session_event.run_step_id", &text(row, "run_step_id")?)?,
                    seq: get(row, "seq")?,
                    turn: get(row, "turn")?,
                    kind: get::<EventKind>(row, "kind")?,
                    role: get::<EventRole>(row, "role")?,
                    tool_call_id: opt_text(row, "tool_call_id")?,
                    payload: json_col("session_event.payload", &text(row, "payload")?)?,
                    raw: opt_json_col("session_event.raw", opt_text(row, "raw")?.as_deref())?,
                    at: ts_col("session_event.at", get(row, "at")?)?,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    // -------------------------------------------------------------------------------------------
    // MOD-2 milestone 9's four reads. Every table they touch **is** mirrored — `document` body and
    // all (`0001_mirror.sql:97-101`), `project.settings` (`:57-61`), `item_link`,
    // `workspace_project` — which is why they are trait methods rather than inherent ones
    // (plan D96), and it is what lets `store::conformance::READ_CASES` run over the mirror.
    // -------------------------------------------------------------------------------------------

    async fn document(&self, id: DocumentId) -> Result<Option<Document>> {
        let row = sqlx::query(
            "SELECT id, item_id, kind, version, title, body, produced_by_step_id, created_by, \
             created_at FROM document WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.as_ref().map(document_of).transpose()
    }

    async fn documents_of_kinds(&self, item: ItemId, kinds: &[String]) -> Result<Vec<Document>> {
        // `DISTINCT ON` is Postgres-only; the mirror picks the latest version per kind with the
        // window function SQLite has had since 3.25, and `json_each` over a bound JSON array
        // stands in for `kind = ANY($2)` (SQLite has no array type - the file's rule 1).
        let rows = sqlx::query(
            "SELECT id, item_id, kind, version, title, body, produced_by_step_id, created_by, \
                    created_at \
               FROM (SELECT d.*, ROW_NUMBER() OVER (PARTITION BY d.kind ORDER BY d.version DESC) \
                                      AS rank_in_kind \
                       FROM document d \
                      WHERE d.item_id = ? \
                        AND (json_array_length(?) = 0 \
                             OR d.kind IN (SELECT k.value FROM json_each(?) k))) \
              WHERE rank_in_kind = 1",
        )
        .bind(item.to_string())
        .bind(serde_json::to_string(kinds).unwrap_or_else(|_| "[]".to_owned()))
        .bind(serde_json::to_string(kinds).unwrap_or_else(|_| "[]".to_owned()))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let latest: Vec<Document> = rows.iter().map(document_of).collect::<Result<_>>()?;
        Ok(order_documents(latest, kinds))
    }

    /// `docs/ANA-9.md` §7.3 as amended by `docs/ANA-5.md` §4.3, on the mirror.
    ///
    /// The Postgres text of `pg/read.rs` with this file's three substitutions plus one more:
    /// SQLite has no `LATERAL`, so the latest-summary lookup is a correlated subquery in the
    /// `SELECT` list. The workspace is bound **twice**, positionally, rather than reused through a
    /// `?NNN` placeholder, because every statement in this file binds plain `?` in order.
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> Result<Vec<UpstreamEntry>> {
        let hops = hops.min(MAX_UPSTREAM_HOPS);
        if hops == 0 {
            return Ok(Vec::new());
        }
        let workspace = scope.workspace.map(|id| id.to_string());

        let rows = sqlx::query(
            "WITH RECURSIVE up(item_id, depth) AS ( \
                     SELECT to_item_id, 1 FROM item_link \
                      WHERE from_item_id = ? AND kind IN ('blocked_by','origin') \
                 UNION \
                     SELECT l.to_item_id, up.depth + 1 \
                       FROM item_link l JOIN up ON l.from_item_id = up.item_id \
                      WHERE up.depth < ? AND l.kind IN ('blocked_by','origin') \
             ), \
             best AS (SELECT item_id, MIN(depth) AS depth FROM up GROUP BY item_id), \
             scope AS ( \
                     SELECT project_id FROM workspace_project WHERE workspace_id = ? \
                 UNION \
                     SELECT ? WHERE ? IS NULL \
             ) \
             SELECT i.id AS item_id, p.slug || ':' || i.key AS qualified_key, i.title, i.status, \
                    best.depth, \
                    (s.project_id IS NOT NULL) AS in_scope, \
                    CASE WHEN s.project_id IS NOT NULL THEN \
                         (SELECT body FROM document d WHERE d.item_id = i.id AND d.kind = 'summary' \
                           ORDER BY d.version DESC LIMIT 1) END AS summary \
               FROM best \
               JOIN item i    ON i.id = best.item_id \
               JOIN project p ON p.id = i.project_id \
               LEFT JOIN scope s ON s.project_id = i.project_id \
              ORDER BY best.depth, qualified_key, i.id",
        )
        .bind(id.to_string())
        .bind(i64::from(hops))
        .bind(workspace.clone())
        .bind(scope.project.to_string())
        .bind(workspace)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in &rows {
            entries.push(UpstreamEntry {
                item_id: uuid_col("item.id", &text(row, "item_id")?)?,
                qualified_key: text(row, "qualified_key")?,
                title: text(row, "title")?,
                status: get::<Status>(row, "status")?,
                depth: u8::try_from(get::<i64>(row, "depth")?).unwrap_or(u8::MAX),
                in_scope: bool_col(get::<i64>(row, "in_scope")?),
                summary: opt_text(row, "summary")?,
            });
        }
        // The mirror's `ORDER BY` is byte order and Postgres's is collation order, so neither is
        // the contract: this is.
        UpstreamEntry::sort_canonical(&mut entries);
        Ok(entries)
    }

    async fn project(&self, id: ProjectId) -> Result<Option<Project>> {
        let row = sqlx::query(
            "SELECT id, slug, name, description, secret_provider, secret_scope, settings, \
             created_by, created_at, updated_at FROM project WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(Project {
            id: uuid_col::<ProjectId>("project.id", &text(&row, "id")?)?,
            slug: text(&row, "slug")?,
            name: text(&row, "name")?,
            description: text(&row, "description")?,
            secret_provider: opt_text(&row, "secret_provider")?,
            secret_scope: opt_text(&row, "secret_scope")?,
            settings: json_col("project.settings", &text(&row, "settings")?)?,
            created_by: uuid_col::<UserId>("project.created_by", &text(&row, "created_by")?)?,
            created_at: ts_col("project.created_at", get(&row, "created_at")?)?,
            updated_at: ts_col("project.updated_at", get(&row, "updated_at")?)?,
        }))
    }
}

// ------------------------------------------------------------------------------------------------
// The seven inherent reads
// ------------------------------------------------------------------------------------------------

impl CacheStore {
    /// Every workspace with its projects, ordered by name then by position.
    ///
    /// A workspace whose projects are outside the refreshed scope lists none of them: the mirror
    /// only holds what a pass was asked to mirror (§4.4).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`], or [`StoreError::Backend`] from a
    /// decoding helper on a corrupt mirror.
    pub async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        let rows = sqlx::query(
            "SELECT w.id AS workspace_id, w.slug, w.name, \
                    p.id AS project_id, p.slug AS project_slug, p.name AS project_name, \
                    wp.position \
               FROM workspace w \
               LEFT JOIN workspace_project wp ON wp.workspace_id = w.id \
               LEFT JOIN project p            ON p.id = wp.project_id \
              ORDER BY w.name, wp.position",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut out: Vec<WorkspaceSummary> = Vec::new();
        for row in &rows {
            let workspace_id: WorkspaceId = uuid_col("workspace.id", &text(row, "workspace_id")?)?;
            if out
                .last()
                .is_none_or(|last| last.workspace_id != workspace_id)
            {
                out.push(WorkspaceSummary {
                    workspace_id,
                    slug: text(row, "slug")?,
                    name: text(row, "name")?,
                    projects: Vec::new(),
                });
            }
            // A `LEFT JOIN` miss means the workspace has no mirrored project on this row.
            let Some(project_id) = opt_text(row, "project_id")? else {
                continue;
            };
            if let Some(last) = out.last_mut() {
                last.projects.push(ProjectRef {
                    project_id: uuid_col("project.id", &project_id)?,
                    slug: text(row, "project_slug")?,
                    name: text(row, "project_name")?,
                    position: get(row, "position")?,
                });
            }
        }
        Ok(out)
    }

    /// This box's row, projected for the top bar.
    ///
    /// The mirror holds the own `box` row and no other (§4.4), so this needs no `this_box`
    /// argument: there is at most one row to find.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn box_info(&self) -> Result<Option<BoxInfo>> {
        let row = sqlx::query("SELECT id, hostname, os_family FROM box ORDER BY id LIMIT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(BoxInfo {
            box_id: uuid_col::<BoxId>("box.id", &text(&row, "id")?)?,
            hostname: text(&row, "hostname")?,
            os_family: get::<OsFamily>(&row, "os_family")?,
        }))
    }

    /// How many runs of the scope are active (`RunStatus::is_active`).
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`].
    pub async fn active_runs(&self, scope: &Scope) -> Result<usize> {
        if scope.project_ids.is_empty() {
            return Ok(0);
        }
        let sql = format!(
            "SELECT COUNT(*) AS count FROM run WHERE project_id IN ({}) \
              AND status IN ('queued','running','awaiting_approval')",
            placeholders(scope.project_ids.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for id in &scope.project_ids {
            query = query.bind(id.to_string());
        }
        let row = query.fetch_one(&self.pool).await.map_err(map_sqlx)?;
        Ok(usize::try_from(get::<i64>(&row, "count")?).unwrap_or(0))
    }

    /// The scope's projects, ordered by `workspace_project.position`.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`], or [`StoreError::Backend`] from a
    /// decoding helper on a corrupt mirror.
    pub async fn projects(&self, scope: &Scope) -> Result<Vec<ProjectRef>> {
        if scope.project_ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT p.id AS project_id, p.slug, p.name, wp.position \
               FROM workspace_project wp JOIN project p ON p.id = wp.project_id \
              WHERE wp.workspace_id = ? AND p.id IN ({}) \
              ORDER BY wp.position",
            placeholders(scope.project_ids.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql)).bind(scope.workspace_id.to_string());
        for id in &scope.project_ids {
            query = query.bind(id.to_string());
        }
        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx)?;

        rows.iter()
            .map(|row| {
                Ok(ProjectRef {
                    project_id: uuid_col::<ProjectId>("project.id", &text(row, "project_id")?)?,
                    slug: text(row, "slug")?,
                    name: text(row, "name")?,
                    position: get(row, "position")?,
                })
            })
            .collect()
    }

    /// The mirrored registry, ordered by `agent.name`, every row with `on_box: None` (MOD-2 plan
    /// D31).
    ///
    /// `on_box` is `None` by construction rather than because a probe found nothing: `agent_box` is
    /// not mirrored, its columns move in milestone 5, and offline "which box" is this box - so
    /// per-box enablement offline is `agent.enabled` and nothing else. The Settings tab already
    /// renders a `None` as "not probed", which is the honest reading here too.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`], or [`StoreError::Backend`] from a
    /// decoding helper on a corrupt mirror.
    pub async fn agents(&self) -> Result<Vec<AgentSummary>> {
        let rows = sqlx::query(
            "SELECT id, name, transport, launch, models, default_model, billing, enabled, \
                    settings, created_at, updated_at \
               FROM agent ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        rows.iter()
            .map(|row| {
                Ok(AgentSummary {
                    agent: Agent {
                        id: uuid_col::<AgentId>("agent.id", &text(row, "id")?)?,
                        name: text(row, "name")?,
                        transport: get::<Transport>(row, "transport")?,
                        launch: json_col("agent.launch", &text(row, "launch")?)?,
                        models: strings_col("agent.models", &text(row, "models")?)?,
                        default_model: opt_text(row, "default_model")?,
                        billing: get::<Billing>(row, "billing")?,
                        enabled: bool_col(get::<i64>(row, "enabled")?),
                        settings: json_col("agent.settings", &text(row, "settings")?)?,
                        created_at: ts_col("agent.created_at", get(row, "created_at")?)?,
                        updated_at: ts_col("agent.updated_at", get(row, "updated_at")?)?,
                    },
                    on_box: None,
                })
            })
            .collect()
    }

    /// `project.settings` of one mirrored project, or `None` when the mirror holds no such row
    /// (MOD-2 plan D70).
    ///
    /// The offline arm of the per-run cap, and it needs no new mirrored column: `project.settings`
    /// has been mirrored since `cache_migrations/0001_mirror.sql:57-61`. An offline chat therefore
    /// enforces the same cap as an online one, off the same document — which is the reason D70
    /// chose this column over an env knob or a new table.
    ///
    /// A runtime query rather than a macro, like every statement in this file: the mirror has no
    /// compile-time schema for `sqlx` to check against.
    ///
    /// # Errors
    ///
    /// Whatever the driver reports, through [`map_sqlx`], or [`StoreError::Backend`] when the
    /// mirrored column does not hold JSON.
    pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
        let row = sqlx::query("SELECT settings FROM project WHERE id = ?")
            .bind(project.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(|row| json_col("project.settings", &text(&row, "settings")?))
            .transpose()
    }

    /// Who this process is, for `run.started_by`, resolved against the mirror (MOD-2 plan D33).
    ///
    /// [`user_named`](CacheStore::user_named) under
    /// [`identity::os_user_name`](crate::identity::os_user_name) - the *same* name
    /// [`PgStore::seed_if_empty`](crate::PgStore::seed_if_empty) stamped the database with, which
    /// is what makes a run recorded offline and uploaded later indistinguishable from one recorded
    /// online.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when this box has never synced an `app_user` row under that name.
    pub async fn this_user(&self) -> Result<UserId> {
        self.user_named(&crate::identity::os_user_name()).await
    }

    /// The mirrored `app_user` row with this `name`: earliest `created_at`, then smallest `id`.
    ///
    /// The tiebreak is `MemStore::this_user`'s, so all three backends answer the same row when the
    /// mirror happens to hold two stamped at the same instant.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when no mirrored row carries that name. Refusing is the point: an
    /// invented `UserId` would make the uploader insert a stranger as the run's author.
    pub async fn user_named(&self, name: &str) -> Result<UserId> {
        let row =
            sqlx::query("SELECT id FROM app_user WHERE name = ? ORDER BY created_at, id LIMIT 1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
        let Some(row) = row else {
            return Err(StoreError::NotFound {
                entity: "app_user",
                id: name.to_owned(),
            });
        };
        uuid_col("app_user.id", &text(&row, "id")?)
    }
}

#[cfg(test)]
mod tests {
    use super::placeholders;

    #[test]
    fn an_empty_in_list_is_null_rather_than_a_syntax_error() {
        assert_eq!(placeholders(0), "NULL");
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?, ?, ?");
    }
}
