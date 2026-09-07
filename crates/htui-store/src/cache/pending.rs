//! The offline chat buffer of `docs/ANA-9.md` §4.3 (blueprint C.14, H.5).
//!
//! A free-standing chat started while Postgres is unreachable mints its `run.id` and `run_step.id`
//! client-side, scrubs every event on the box and appends them to
//! `<cache_dir>/pending/<project_id>.<run_id>.jsonl` - one JSON object per line, exactly the serde
//! form of [`SessionEvent`], carrying the `[H-1]` `.open` suffix below until the chat ends. On the
//! next successful connection [`upload_pending`] inserts the
//! `run`, its `run_step`s and every event in one transaction and then deletes the file.
//!
//! [`append_pending`] is the only writer of that name and [`upload_pending`] the only reader, so
//! the naming contract has exactly one owner on each side (MOD-2 plan D8). The appender **trusts
//! its input**: every event is expected to have been scrubbed by the recorder before it reaches
//! this function, because the `Scrubber` seam lives there - `htui-store` never inspects a payload.
//! The uploader reads two keys out of one payload, the `prompt` row's `digest` and the `usage`
//! rows' five integers, and that is a **schema** operation rather than an inspection: both are
//! `run_step` columns the recorder fills online through `set_step_usage`, and neither is text this
//! module looks at, masks or unmasks (MOD-2 plan D36).
//!
//! **A live buffer is written under a `.open` suffix** and sealed by [`seal_pending`] when its
//! chat ends (MOD-2 milestone 4, risk `[H-1]`): `upload_pending` reads sealed files only, so a
//! chat that outlives a reconnect is uploaded once, whole, on the pass after it ends rather than
//! half-way through with a partial `run_step.usage`. [`seal_orphaned`] at `CacheStore::open`
//! adopts what a crash left open. **Sealing is always a rename, never an append**: when the
//! sealed name is taken the buffer is renamed to `<project>.<run>.<n>.jsonl` instead, so no
//! sequence of crashes can duplicate a line and no second writer of the same run can have its file
//! deleted out from under the tail it just added. The one known limit is a second `htui` process on
//! the same box: it seals the first one's live buffer at start, so those rows land in two passes -
//! every row lands, and the *first* pass's `run_step.usage` is the partial sum, because the second
//! file's `run_step` insert is an `ON CONFLICT (id) DO NOTHING`. That is narrower than the race the
//! suffix removes, and ANA-9 §4.4 already treats a second process as just another reader.
//!
//! **The file name carries the project id**, which ANA-9 §4.3 does not: `run.project_id`,
//! `run.target_box_id` and `run.started_by` are all `NOT NULL` and the line format holds only
//! `session_event` columns, so the run could not otherwise be reconstructed. The box and the user
//! come from [`upload_pending`]'s arguments (blueprint H.5).

use std::collections::{BTreeMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use htui_core::model::{
    BoxId, EventKind, ProjectId, RunId, SessionEvent, StepId, UsageTotals, UserId,
};
use htui_core::store::{Result, StoreError};
use serde_json::Value;
use sqlx::PgPool;

use super::PENDING_DIR;
use crate::error::map_sqlx;

/// The extension every buffer file carries.
const EXTENSION: &str = "jsonl";

/// `[H-1]` The extra suffix a buffer carries while its chat is still writing to it:
/// `<project>.<run>.jsonl.open`.
///
/// [`upload_pending`]'s listing filters on the `jsonl` extension, and an open buffer's extension
/// is this one, so a live chat's file is invisible to the uploader **by construction** rather than
/// by a check it could forget. [`seal_pending`] is what makes a buffer uploadable.
pub const OPEN_SUFFIX: &str = "open";

/// `[H-1]` How many `<project>.<run>.<n>.jsonl` names one run may be sealed under before the seal
/// gives up and reports it.
///
/// Each of them is one seal that found the previous name still on disk, i.e. one buffer no upload
/// pass has taken yet. Reaching this many means the uploader has not run - or has not succeeded -
/// a thousand times over one run, which is a state to report rather than to keep numbering past.
const SEALED_NAMES: u32 = 1_000;

/// Appends `events` to this run's **open** buffer file and returns how many lines were written.
///
/// The file is `<dir>/pending/<project>.<run>.jsonl.open`, the two-part name [`upload_pending`]
/// parses back plus the `[H-1]` suffix [`seal_pending`] removes; this is its only writer. It is
/// opened with `append(true).create(true)`, so the first call creates it and every later call
/// extends it without touching a byte already on disk - which is what makes a chat that outlives
/// several process restarts one buffer rather than several. The `pending/` directory is created
/// when missing, exactly as `CacheStore::open` creates it.
///
/// One `serde_json::to_string` line per event, in the order given, each closed by `\n`. The caller
/// owns `seq`: this function neither assigns nor checks it, and the loader sorts by `seq` anyway.
/// An empty slice writes nothing and creates no file, because an event-less buffer is one
/// [`upload_pending`] would only warn at.
///
/// The events are written verbatim. **Scrubbing is the recorder's job** and has already happened
/// by the time they get here; `htui-store` does not look inside a payload.
///
/// # Errors
///
/// [`StoreError::Backend`] when the directory cannot be created, the file cannot be opened or the
/// write fails - the path is in the message.
pub async fn append_pending(
    dir: &Path,
    project: ProjectId,
    run: RunId,
    events: &[SessionEvent],
) -> Result<usize> {
    if events.is_empty() {
        return Ok(0);
    }

    // Serialising is cheap and happens on the caller's thread; only the file write is blocking
    // work, so only it goes to the blocking pool.
    let mut lines = String::new();
    for event in events {
        let line = serde_json::to_string(event).map_err(|e| {
            StoreError::Backend(format!("cannot serialise a pending session_event: {e}"))
        })?;
        lines.push_str(&line);
        lines.push('\n');
    }

    let pending = dir.join(PENDING_DIR);
    // `[H-1]` the open name: sealed by `seal_pending` when the chat ends.
    let path = pending.join(format!("{project}.{run}.{EXTENSION}.{OPEN_SUFFIX}"));
    let written = events.len();

    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&pending).map_err(|e| {
            StoreError::Backend(format!("cannot create {}: {e}", pending.display()))
        })?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| StoreError::Backend(format!("cannot open {}: {e}", path.display())))?;
        file.write_all(lines.as_bytes()).map_err(|e| {
            StoreError::Backend(format!("cannot append to {}: {e}", path.display()))
        })?;
        Ok(written)
    })
    .await
    .map_err(|e| StoreError::Backend(format!("the pending append task failed: {e}")))?
}

/// `[H-1]` Seals this run's buffer, making it uploadable: `<project>.<run>.jsonl.open` becomes
/// `<project>.<run>.jsonl`.
///
/// Answers `false` when there is no open buffer - a chat that recorded nothing, or one already
/// sealed - because "seal what is not there" is the ordinary end of a cancelled chat, not a fault.
///
/// Called by `Writer::Buffered`'s `finish_chat_run`, which is the only thing that knows a chat has
/// ended. A flush that arrives *after* the seal creates a fresh open buffer rather than losing its
/// rows; the next seal (or the next [`seal_orphaned`]) gives it a sealed name of its own, so no
/// line is ever overwritten and none is ever written twice.
///
/// # Errors
///
/// [`StoreError::Backend`] with the path when the rename fails.
pub async fn seal_pending(dir: &Path, project: ProjectId, run: RunId) -> Result<bool> {
    let path = dir
        .join(PENDING_DIR)
        .join(format!("{project}.{run}.{EXTENSION}.{OPEN_SUFFIX}"));
    tokio::task::spawn_blocking(move || {
        if !path.exists() {
            return Ok(false);
        }
        seal_one(&path).map(|()| true)
    })
    .await
    .map_err(|e| StoreError::Backend(format!("the pending seal task failed: {e}")))?
}

/// `[H-1]` Seals every `pending/*.jsonl.open` and answers how many were sealed.
///
/// Called by `CacheStore::open`, at a moment when no chat of this process can be live: an open
/// buffer found there belongs to a run that crashed, and its rows are complete as far as they got,
/// which is what `R-HIS-1` asks for in the crash case. A missing `pending/` is `Ok(0)` - nothing
/// has ever been buffered on this box.
///
/// Synchronous because its one caller is already doing blocking directory work, and a mirror open
/// is not on the UI task.
///
/// **A file that cannot be sealed is a `warn!` and the sweep continues**, exactly as
/// [`upload_pending`] treats a file the server refuses. This is best-effort adoption of what a
/// crash left behind, and the caller is `CacheStore::open`: one unsealable buffer - a permissions
/// problem, a file a second process holds open on Windows - must not fail `start()` and cost the
/// user the whole application over a buffer nothing has read yet. The file stays on disk and the
/// next launch tries again.
///
/// # Errors
///
/// [`StoreError::Backend`] when `pending/` exists but cannot be listed. That is not a per-file
/// problem, and answering `0` would claim a sweep that never happened.
pub fn seal_orphaned(dir: &Path) -> Result<usize> {
    let pending = dir.join(PENDING_DIR);
    let entries = match std::fs::read_dir(&pending) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(StoreError::Backend(format!(
                "cannot list {}: {e}",
                pending.display()
            )));
        }
    };

    let mut sealed = 0usize;
    for path in entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == OPEN_SUFFIX))
    {
        match seal_one(&path) {
            Ok(()) => sealed += 1,
            // Warn and carry on: the buffer is still on disk, still complete, and still adoptable
            // by a later launch. Failing here would fail `CacheStore::open`, and with it `start()`.
            Err(err) => tracing::warn!(
                file = %path.display(),
                error = %err,
                "cache: cannot seal an offline chat buffer; leaving it open for the next launch",
            ),
        }
    }
    if sealed > 0 {
        tracing::info!(
            sealed,
            "cache: adopted offline chat buffers a previous run left open"
        );
    }
    Ok(sealed)
}

/// `[H-1]` `<name>.jsonl.open` → a sealed name, always by **rename** and never by appending.
///
/// `<project>.<run>.jsonl` when that is free, which is every ordinary case. When it is not - a
/// flush that landed after an earlier seal, or a buffer a second process sealed while this one was
/// still writing - the open file takes the next free `<project>.<run>.<n>.jsonl` instead
/// ([`next_sealed_name`]), which [`parse`] reads back as the same run.
///
/// Appending the open file's lines to the sealed one and removing the open file was the earlier
/// answer, and it had two windows a rename does not have. A crash between the write and the
/// removal leaves the open file in place, so the next [`seal_orphaned`] appends the very same
/// lines again - and while `session_event` survives that on `ON CONFLICT (run_step_id, seq)`,
/// [`UsageTotals::from_rows`] would count every duplicated `usage` row twice and land a
/// `run_step.usage` that is not the conversation's (§11 criterion 7). And a second `htui` that
/// seals, uploads and deletes this run's sealed file takes the appended tail with it. A rename is
/// atomic, so neither window exists, and the extra file simply uploads on a pass of its own.
fn seal_one(open: &Path) -> Result<()> {
    // `with_extension("")` drops exactly the `.open` suffix, leaving the `.jsonl` name.
    let sealed = open.with_extension("");
    let target = if sealed.exists() {
        next_sealed_name(&sealed)?
    } else {
        sealed
    };
    std::fs::rename(open, &target).map_err(|e| {
        StoreError::Backend(format!(
            "cannot seal {} as {}: {e}",
            open.display(),
            target.display()
        ))
    })
}

/// `[H-1]` `<project>.<run>.jsonl` → the first free `<project>.<run>.<n>.jsonl`, `n` from 1.
///
/// There is exactly one `.open` name per `(project, run)`, so two sealers racing here are racing
/// over the *same* source file: at most one rename can succeed and the loser reports that its
/// source is gone rather than overwriting anything.
///
/// # Errors
///
/// [`StoreError::Backend`] when the name has no stem this can extend, or when [`SEALED_NAMES`]
/// numbers are all taken - a run whose buffer has been sealed that many times without a single
/// upload has something wrong with it that a `+1` would not fix.
fn next_sealed_name(sealed: &Path) -> Result<PathBuf> {
    let parent = sealed.parent().unwrap_or_else(|| Path::new(""));
    let stem = sealed
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| {
            StoreError::Backend(format!("cannot name a second seal of {}", sealed.display()))
        })?;
    (1..=SEALED_NAMES)
        .map(|n| parent.join(format!("{stem}.{n}.{EXTENSION}")))
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| {
            StoreError::Backend(format!(
                "{} has {SEALED_NAMES} unuploaded seals already",
                sealed.display()
            ))
        })
}

/// Uploads every sealed `pending/*.jsonl` chat buffer and returns how many files landed.
///
/// One transaction per file: the `run` (`kind = 'chat'`, `mode = 'manual'`, `item_id NULL`,
/// `status = 'done'`), its `run_step`s (`phase_name = 'chat'`, with the `prompt_digest` and the
/// `usage` derived from the rows) and every event, then the file is deleted. A buffer still being
/// written carries the `[H-1]` [`OPEN_SUFFIX`] and is therefore not a `*.jsonl` file at all, so a
/// chat that is still running is never uploaded half-way.
///
/// Idempotency is the three `ON CONFLICT DO NOTHING` clauses on top of
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
///
/// The extension filter is also what `[H-1]` rests on: `<name>.jsonl.open`'s extension is
/// [`OPEN_SUFFIX`], so a live buffer never reaches this list.
///
/// `[H-1]` The sort key is the **stem**, not the whole name, which is what puts
/// `<project>.<run>.jsonl` ahead of the `<project>.<run>.<n>.jsonl` a collided seal produced:
/// `<project>.<run>` is a prefix of every numbered stem and so sorts before all of them, whereas
/// the full names would order `…​.1.jsonl` first. The two halves of one run are then uploaded in
/// the order they were sealed, which is the order they were written - so the `run_step` row the
/// first of them inserts is the one carrying the chat's opening `prompt`, and `prompt_digest` is
/// the digest of the prompt the conversation actually started with.
fn list(pending: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(pending)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == EXTENSION))
        .collect();
    files.sort_by_key(|path| path.file_stem().map(std::ffi::OsStr::to_os_string));
    Ok(files)
}

/// Reads and validates one buffer, or `None` after a `warn!` that leaves the file in place.
fn parse(path: &Path) -> Option<Buffer> {
    let skip = |reason: &str| {
        tracing::warn!(file = %path.display(), reason, "cache: skipping a malformed pending file");
        None::<Buffer>
    };

    // `<project_id>.<run_id>.jsonl`, both UUIDs hyphenated, separated by a dot - or, `[H-1]`,
    // `<project_id>.<run_id>.<n>.jsonl`, the name [`seal_one`] falls back to when the two-part one
    // is taken. Only the first two components are read: `<n>` distinguishes the files on disk and
    // means nothing to the run they both belong to.
    let stem = path.file_stem()?.to_str()?;
    let mut parts = stem.splitn(3, '.');
    let (Some(project), Some(run)) = (parts.next(), parts.next()) else {
        return skip("the name is not <project_id>.<run_id>[.<n>].jsonl");
    };
    let (Ok(project_id), Ok(run_id)) = (project.parse::<ProjectId>(), run.parse::<RunId>()) else {
        return skip("the name does not hold two UUIDs");
    };

    let Ok(text) = std::fs::read_to_string(path) else {
        return skip("the file cannot be read");
    };

    // A `(run_step_id, seq)` this file has already carried is dropped, keeping the first line that
    // held it. `session_event`'s `ON CONFLICT (run_step_id, seq)` makes a duplicated *row*
    // harmless, but `run_step.usage` is summed from these events before any of that runs, and a
    // `usage` row counted twice lands totals that are not the conversation's (§11 criterion 7).
    // Nothing in this module produces a duplicate any more - sealing is a rename - so this is the
    // loader being robust to one that arrived some other way, not a mechanism anything relies on.
    let mut seen: HashSet<(StepId, i32)> = HashSet::new();
    let mut events: Vec<SessionEvent> = Vec::new();
    let mut duplicates = 0usize;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // Not re-derived from §4.3's table: the line *is* `SessionEvent`'s serde form.
        match serde_json::from_str::<SessionEvent>(line) {
            Ok(event) if !seen.insert((event.run_step_id, event.seq)) => duplicates += 1,
            Ok(event) => events.push(event),
            Err(_) => return skip("a line is not a session_event object"),
        }
    }
    if duplicates > 0 {
        tracing::warn!(
            file = %path.display(),
            duplicates,
            "cache: a pending file repeats a (run_step_id, seq); keeping the first of each",
        );
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
///
/// Each `run_step` carries the two columns the online recorder writes through `set_step_usage`
/// (MOD-2 plan D36), derived from that step's own rows: `prompt_digest` is the `prompt` row's
/// `payload.digest`, and `usage` is [`UsageTotals::from_rows`], the **one** summing rule both
/// writers share - the token fields are per-row deltas, so "take the last row" would be wrong and
/// a second hand-written sum here would drift from the recorder's. `usage` is written even when
/// it totals five `null`s, because the online path writes that same document at the prompt and an
/// uploaded step must be indistinguishable from one recorded online (§11 criteria 3 and 7).
///
/// `run_step.agent_id` and `model` stay `NULL`: the line format carries neither (risk H-3).
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
        let rows: Vec<SessionEvent> = buffer
            .events
            .iter()
            .filter(|event| event.run_step_id == *step)
            .cloned()
            .collect();
        let started_at = rows.iter().map(|row| row.at).min().unwrap_or(first_at);
        let finished_at = rows.iter().map(|row| row.at).max().unwrap_or(last_at);
        // Two `run_step` columns out of the step's own rows: a schema read, not an inspection.
        let prompt_digest = rows
            .iter()
            .find(|row| row.kind == EventKind::Prompt)
            .and_then(|row| row.payload.get("digest"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let usage = UsageTotals::from_rows(&rows).to_value();

        sqlx::query!(
            "INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, \
                                   status, started_at, finished_at, prompt_digest, usage) \
             VALUES ($1, $2, $3, 1, 0, 'chat', 'done', $4, $5, $6, $7) \
             ON CONFLICT (id) DO NOTHING",
            step.as_uuid(),
            buffer.run_id.as_uuid(),
            i32::try_from(position).unwrap_or(i32::MAX),
            started_at,
            finished_at,
            prompt_digest.as_deref(),
            usage,
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
