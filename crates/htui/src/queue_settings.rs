//! The queue settings behind `Settings > Queue` (MOD-12 milestone 2, D8-D10): the three
//! `app_setting` keys, each scope project's two spend caps and this box's concurrency limit.
//!
//! One read per event, one reply out, every write through
//! [`WriteStore::set_queue_setting`]/[`WriteStore::clear_queue_setting`]. The section renders the
//! last snapshot and never patches a row into it, so there is one source of truth on the render
//! side, as [`crate::prompt_settings`] has it.
//!
//! Known residue, `prompt_settings`'s: a re-read that fails after an applied write answers
//! `Failed`, so the section is told nothing happened when the row has in fact changed.
//!
//! Nothing here reads the clock: `updated_at` and `edit_version` are the seam's to move.

use std::collections::BTreeMap;

use htui_core::model::{BoxId, Project, QueueSetting, Scope, admission_limit};
use htui_core::store::{
    CasOutcome, QueueStored, QueueTarget, QueueToken, ReadStore, Result, StoreError, WriteStore,
};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::{Value, json};

use crate::store_worker::{StoreReply, StoreRequest};

/// One read of the scope.
///
/// `PartialEq` only: [`Project`] derives no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueSettingsSnapshot {
    /// [`QueueSetting::APP_KEYS`], in that order: always three entries.
    pub app: Vec<QueueAppEntry>,
    /// Each scope project that still names a row, in scope order.
    pub projects: Vec<QueueProjectEntry>,
    /// This box; `None` before registration.
    pub this_box: Option<QueueBoxEntry>,
}

/// One `app_setting` key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAppEntry {
    /// The key.
    pub key: QueueSetting,
    /// The stored JSON, `None` when there is no row. Postgres seeds `scheduler_window` as JSON
    /// `null` (`0003_orchestration.sql`), which reads as not set, so a `null` is `None` here too.
    pub value: Option<Value>,
    /// The row's token; `Stamp(None)` when there is no row.
    pub token: QueueToken,
}

/// One project's two caps.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueProjectEntry {
    /// The row (name, slug, `updated_at`, the whole `settings` blob).
    pub project: Project,
    /// `settings.per_token_cap_run`, as stored; `None` (or JSON `null`) is unbounded.
    pub run_cap: Option<Value>,
    /// `settings.per_token_cap_batch`, as stored; `None` (or JSON `null`) is unbounded.
    pub batch_cap: Option<Value>,
}

impl QueueProjectEntry {
    /// The stored value of one of [`QueueSetting::PROJECT_KEYS`]; `None` for any other key.
    #[must_use]
    pub fn cap(&self, key: QueueSetting) -> Option<&Value> {
        match key {
            QueueSetting::PerTokenCapRun => self.run_cap.as_ref(),
            QueueSetting::PerTokenCapBatch => self.batch_cap.as_ref(),
            _ => None,
        }
    }

    /// The token a write to this project presents: its `updated_at`.
    #[must_use]
    pub const fn token(&self) -> QueueToken {
        QueueToken::Stamp(Some(self.project.updated_at))
    }
}

/// This box's `max_concurrent_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueBoxEntry {
    /// `box.id`.
    pub id: BoxId,
    /// `box.hostname`.
    pub hostname: String,
    /// `settings.max_concurrent_items`, as stored; `None` inherits the app default.
    pub value: Option<Value>,
    /// `EditVersion(box.edit_version)`.
    pub token: QueueToken,
    /// What `claim_run` admits on this box: [`admission_limit`] over the box's settings and the
    /// app rows (D8).
    pub effective: u32,
}

impl QueueSettingsSnapshot {
    /// The app rows with a value, keyed by [`QueueSetting::as_str`], as `admission_limit` reads
    /// them.
    #[must_use]
    pub fn app_map(&self) -> BTreeMap<String, Value> {
        self.app
            .iter()
            .filter_map(|entry| {
                entry
                    .value
                    .clone()
                    .map(|value| (entry.key.as_str().to_owned(), value))
            })
            .collect()
    }

    /// The app default a box with no limit of its own inherits.
    #[must_use]
    pub fn app_limit(&self) -> u32 {
        admission_limit(&json!({}), &self.app_map())
    }

    /// The `App` entry for `key`; `None` for a key the app target does not take.
    #[must_use]
    pub fn app_entry(&self, key: QueueSetting) -> Option<&QueueAppEntry> {
        self.app.iter().find(|entry| entry.key == key)
    }
}

/// A stored value as the snapshot holds it: JSON `null` reads as not set (H-8).
fn present(value: Option<Value>) -> Option<Value> {
    value.filter(|value| !value.is_null())
}

/// One read of the whole scope: three `queue_setting(App, _)`, per scope project one `project()`,
/// and this box's `box_info()` plus one `queue_setting(Box(id), MaxConcurrentItems)`.
///
/// N+1 reads on purpose, `prompt_settings::snapshot`'s trade: they happen per event (activation, a
/// scope change, after a write), never per keystroke. `backend` answers `box_info` (the writer
/// holds no such read); `writer` answers every other read.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot(
    backend: &Backend,
    writer: &Writer,
    scope: &Scope,
) -> Result<QueueSettingsSnapshot> {
    let mut app = Vec::with_capacity(QueueSetting::APP_KEYS.len());
    for key in QueueSetting::APP_KEYS {
        let stored = writer.queue_setting(QueueTarget::App, key).await?;
        app.push(match stored {
            Some(QueueStored { value, token }) => QueueAppEntry {
                key,
                value: present(value),
                token,
            },
            None => QueueAppEntry {
                key,
                value: None,
                token: QueueToken::Stamp(None),
            },
        });
    }

    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for id in &scope.project_ids {
        let Some(project) = writer.project(*id).await? else {
            continue;
        };
        let cap = |key: QueueSetting| present(project.settings.get(key.as_str()).cloned());
        let (run_cap, batch_cap) = (
            cap(QueueSetting::PerTokenCapRun),
            cap(QueueSetting::PerTokenCapBatch),
        );
        projects.push(QueueProjectEntry {
            project,
            run_cap,
            batch_cap,
        });
    }

    let mut snapshot = QueueSettingsSnapshot {
        app,
        projects,
        this_box: None,
    };
    if let Some(info) = backend.box_info().await?
        && let Some(stored) = writer
            .queue_setting(
                QueueTarget::Box(info.box_id),
                QueueSetting::MaxConcurrentItems,
            )
            .await?
    {
        let effective = admission_limit(&info.settings, &snapshot.app_map());
        snapshot.this_box = Some(QueueBoxEntry {
            id: info.box_id,
            hostname: info.hostname,
            value: present(stored.value),
            token: stored.token,
            effective,
        });
    }
    Ok(snapshot)
}

/// Serves one queue-setting request, off the UI task.
///
/// `Err(StoreError::Unreachable(DATABASE_UNREACHABLE))` on [`Backend::Offline`], whose
/// [`writer`](Backend::writer) is `None`, the read included: the sentence every orchestration
/// request gets off the server, as `prompt_settings::serve` answers it.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline and
/// [`StoreError::Backend`] for a request that is not one of this module's three.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;

    match request {
        StoreRequest::QueueSettings(scope) => Ok(StoreReply::QueueSettings(Box::new(
            snapshot(backend, &writer, scope).await?,
        ))),
        StoreRequest::SetQueueSetting {
            scope,
            target,
            key,
            value,
            expected,
        } => {
            let outcome = writer
                .set_queue_setting(*target, *key, value.clone(), *expected)
                .await?;
            cas(backend, &writer, scope, &outcome).await
        }
        StoreRequest::ClearQueueSetting {
            scope,
            target,
            key,
            expected,
        } => {
            let outcome = writer.clear_queue_setting(*target, *key, *expected).await?;
            cas(backend, &writer, scope, &outcome).await
        }
        // `try_serve` routes exactly this module's three variants here; a caller that reached it
        // anyway is better told which request it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a queue settings request: {}",
            other.name()
        ))),
    }
}

/// A compare-and-set outcome as a reply: `Applied` answers the fresh settings, `Stale` the same
/// under [`StoreReply::QueueSettingsStale`], so the editor reloads and retries by hand.
async fn cas<T>(
    backend: &Backend,
    writer: &Writer,
    scope: &Scope,
    outcome: &CasOutcome<T>,
) -> Result<StoreReply> {
    let fresh = Box::new(snapshot(backend, writer, scope).await?);
    Ok(match outcome {
        CasOutcome::Applied(_) => StoreReply::QueueSettings(fresh),
        CasOutcome::Stale(_) => StoreReply::QueueSettingsStale(fresh),
    })
}

/// The three request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here.
pub const REQUEST_NAMES: [&str; 3] = ["queue_settings", "set_queue_setting", "clear_queue_setting"];

/// The read's name: a refused read leaves the section with nothing to show, where a refused write
/// leaves the editor over its text.
pub const READ_NAME: &str = REQUEST_NAMES[0];
