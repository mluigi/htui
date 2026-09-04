//! The offline chat buffer of `docs/ANA-9.md` §4.3 (blueprint C.14, H.5).
//!
//! A free-standing chat started while Postgres is unreachable mints its `run.id` and `run_step.id`
//! client-side, scrubs every event on the box and appends them to
//! `<cache_dir>/pending/<project_id>.<run_id>.jsonl` - one JSON object per line, exactly the serde
//! form of [`SessionEvent`]. On the next successful connection [`upload_pending`] inserts the
//! `run`, its `run_step`s and every event in one transaction and then deletes the file.
//!
//! **The file name carries the project id**, which ANA-9 §4.3 does not: `run.project_id`,
//! `run.target_box_id` and `run.started_by` are all `NOT NULL` and the line format holds only
//! `session_event` columns, so the run could not otherwise be reconstructed. The box and the user
//! come from [`upload_pending`]'s arguments (blueprint H.5).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, ProjectId, RunId, SessionEvent, StepId, UserId};
use htui_core::store::{Result, StoreError};
use sqlx::PgPool;

use super::PENDING_DIR;
use crate::error::map_sqlx;

/// The extension every buffer file carries.
const EXTENSION: &str = "jsonl";

/// Uploads every `pending/*.jsonl` chat buffer and returns how many files landed.
///
/// One transaction per file: the `run` (`kind = 'chat'`, `mode = 'manual'`, `item_id NULL`,
/// `status = 'done'`), its `run_step`s (`phase_name = 'chat'`) and every event, then the file is
/// deleted. Idempotency is the three `ON CONFLICT DO NOTHING` clauses on top of
/// `session_event`'s `PRIMARY KEY (run_step_id, seq)`: running the upload twice inserts nothing
/// the second time and still deletes the file (§11.7). The delete happens **after** the commit, so
/// a crash in between re-uploads on the next pass, which is a no-op.
///
/// A file that cannot be parsed - a bad name, a bad line, or no events at all - is left in place
/// and logged at `warn`. It is never deleted and never fatal: a corrupt buffer must not cost the
/// user every other buffered chat, and it stays on disk for inspection. A file the *server*
/// refuses ([`StoreError::Constraint`]: a project deleted while this box was offline, a `kind`
/// outside its `CHECK`) is treated the same way, because no later pass will change the answer and
/// one poisoned buffer must not block every buffer queued behind it.
///
/// # Errors
///
/// Whatever else the driver reports, through [`map_sqlx`] - an unreachable server aborts the
/// upload and the next pass retries it - and [`StoreError::Backend`] when `pending/` exists but
/// cannot be listed.
pub async fn upload_pending(
    pool: &PgPool,
    dir: &Path,
    this_box: BoxId,
    this_user: UserId,
) -> Result<usize> {
    let pending = dir.join(PENDING_DIR);
    let files = match list(&pending) {
        Ok(files) => files,
        // Nothing has ever been buffered on this box; that is not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(StoreError::Backend(format!(
                "cannot list {}: {e}",
                pending.display()
            )));
        }
    };

    let mut uploaded = 0usize;
    for path in files {
        let Some(buffer) = parse(&path) else { continue };
        match upload_one(pool, &buffer, this_box, this_user).await {
            Ok(()) => {}
            // The server parsed it and said no - a project deleted while this box was offline, a
            // `kind` outside its `CHECK`. Nothing about the next pass will change its mind, so
            // propagating it would abort the upload half of *every* later pass and cost the user
            // every buffer queued behind this one. It stays on disk, like a malformed file.
            Err(StoreError::Constraint(why)) => {
                tracing::warn!(
                    file = %path.display(),
                    reason = %why,
                    "cache: the server refused a pending file; leaving it in place",
                );
                continue;
            }
            // Anything else is the server, not the file: stop and let the next pass retry.
            Err(err) => return Err(err),
        }
        match std::fs::remove_file(&path) {
            Ok(()) => uploaded += 1,
            Err(e) => {
                // The rows are committed; the next pass re-uploads them as a no-op and tries the
                // delete again, so this is a warning rather than a failed pass.
                tracing::warn!(file = %path.display(), error = %e, "cache: cannot remove an uploaded pending file");
            }
        }
    }
    Ok(uploaded)
}

/// One parsed buffer file.
struct Buffer {
    /// `run.project_id`, from the file name.
    project_id: ProjectId,
    /// `run.id`, from the file name.
    run_id: RunId,
    /// Every event of the file, ascending by `seq`.
    events: Vec<SessionEvent>,
    /// The distinct `run_step_id`s in `position` order (smallest `seq`, then earliest `at`).
    steps: Vec<StepId>,
}

/// `pending/*.jsonl`, sorted by name so two runs land in a stable order.
fn list(pending: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(pending)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .collect();
    files.sort();
    Ok(files)
}

/// Reads and validates one buffer, or `None` after a `warn!` that leaves the file in place.
fn parse(path: &Path) -> Option<Buffer> {
    let skip = |reason: &str| {
        tracing::warn!(file = %path.display(), reason, "cache: skipping a malformed pending file");
        None::<Buffer>
    };

    // `<project_id>.<run_id>.jsonl`, both UUIDs hyphenated, separated by a dot.
    let stem = path.file_stem()?.to_str()?;
    let Some((project, run)) = stem.split_once('.') else {
        return skip("the name is not <project_id>.<run_id>.jsonl");
    };
    let (Ok(project_id), Ok(run_id)) = (project.parse::<ProjectId>(), run.parse::<RunId>()) else {
        return skip("the name does not hold two UUIDs");
    };

    let Ok(text) = std::fs::read_to_string(path) else {
        return skip("the file cannot be read");
    };

    let mut events: Vec<SessionEvent> = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // Not re-derived from §4.3's table: the line *is* `SessionEvent`'s serde form.
        match serde_json::from_str::<SessionEvent>(line) {
            Ok(event) => events.push(event),
            Err(_) => return skip("a line is not a session_event object"),
        }
    }
    if events.is_empty() {
        return skip("the file holds no event");
    }

    // Lines are appended in `seq` order, but the loader sorts anyway.
    events.sort_by_key(|event| event.seq);

    // One `run_step` per distinct `run_step_id`, ordered by that step's smallest `seq` then its
    // earliest `at`, so the `UNIQUE (run_id, position, attempt, fanout_index)` of §5.8 cannot
    // collide.
    let mut first: BTreeMap<StepId, (i32, DateTime<Utc>)> = BTreeMap::new();
    for event in &events {
        let entry = first
            .entry(event.run_step_id)
            .or_insert((event.seq, event.at));
        if (event.seq, event.at) < *entry {
            *entry = (event.seq, event.at);
        }
    }
    let mut ordered: Vec<(StepId, (i32, DateTime<Utc>))> = first.into_iter().collect();
    ordered.sort_by_key(|(_, key)| *key);
    let steps = ordered.into_iter().map(|(id, _)| id).collect();

    Some(Buffer {
        project_id,
        run_id,
        events,
        steps,
    })
}

/// Inserts one buffer's `run`, `run_step`s and events in a single transaction.
async fn upload_one(
    pool: &PgPool,
    buffer: &Buffer,
    this_box: BoxId,
    this_user: UserId,
) -> Result<()> {
    let first_at = buffer
        .events
        .iter()
        .map(|event| event.at)
        .min()
        .ok_or_else(|| StoreError::Backend("cache: an empty pending buffer".to_owned()))?;
    let last_at = buffer
        .events
        .iter()
        .map(|event| event.at)
        .max()
        .unwrap_or(first_at);

    let mut tx = pool.begin().await.map_err(map_sqlx)?;

    sqlx::query!(
        "INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, \
                          executing_box_id, started_by, queued_at, started_at, finished_at) \
         VALUES ($1, $2, NULL, 'chat', 'manual', 'done', $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (id) DO NOTHING",
        buffer.run_id.as_uuid(),
        buffer.project_id.as_uuid(),
        this_box.as_uuid(),
        this_box.as_uuid(),
        this_user.as_uuid(),
        first_at,
        first_at,
        last_at,
    )
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    for (position, step) in buffer.steps.iter().enumerate() {
        let bounds: Vec<DateTime<Utc>> = buffer
            .events
            .iter()
            .filter(|event| event.run_step_id == *step)
            .map(|event| event.at)
            .collect();
        let started_at = bounds.iter().copied().min().unwrap_or(first_at);
        let finished_at = bounds.iter().copied().max().unwrap_or(last_at);

        sqlx::query!(
            "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, \
                                   status, started_at, finished_at) \
             VALUES ($1, $2, $3, 1, 0, 'chat', 'done', $4, $5) \
             ON CONFLICT (id) DO NOTHING",
            step.as_uuid(),
            buffer.run_id.as_uuid(),
            i32::try_from(position).unwrap_or(i32::MAX),
            started_at,
            finished_at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for event in &buffer.events {
        sqlx::query!(
            "INSERT INTO session_event (run_step_id, seq, turn, kind, role, tool_call_id, \
                                        payload, raw, at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (run_step_id, seq) DO NOTHING",
            event.run_step_id.as_uuid(),
            event.seq,
            event.turn,
            event.kind.as_str(),
            event.role.as_str(),
            event.tool_call_id.as_deref(),
            &event.payload,
            event.raw.as_ref(),
            event.at,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    tx.commit().await.map_err(map_sqlx)
}
