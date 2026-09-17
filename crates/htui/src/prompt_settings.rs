//! The prompt settings behind `Settings > Prompt`: the ten `app_setting` keys of the registry and,
//! per project of the scope, the keys the `Project` rung accepts (MOD-15 milestone 5, D1/D3).
//!
//! One read per event and never one per keystroke (M3 D5's trade), one reply out, and every write
//! through milestone 1's compare-and-set seam. The section renders from the snapshot and never
//! patches a single row into it, so there is exactly one source of truth on the render side.
//!
//! Known residue, the same one [`crate::catalogue::serve`] carries and milestone 3 recorded: every
//! write arm answers `Failed` if the re-read *after* an applied write fails, so the section is told
//! nothing happened when the row has in fact changed — separating the two needs a seam method that
//! returns the write's own outcome without a read, and this milestone adds none.
//!
//! Nothing here resolves an identity: no row this module writes carries a `created_by` or a box, so
//! unlike [`crate::hierarchy`] there is no `this_user` and no `box_info` call in the file. Nothing
//! here reads the clock either — `updated_at` is the seam's to stamp, never this crate's.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use htui_core::model::{Project, Scope};
use htui_core::prompt::{Rungs, SettingKey};
use htui_core::store::{CasOutcome, ReadStore, Result, SettingRung, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::Value;

use crate::store_worker::{StoreReply, StoreRequest};

/// One read of the scope: the ten `App` rows, then every project of the scope that still names a
/// row, each with the keys the `Project` rung accepts.
///
/// `Eq` is absent for the reason [`CatalogueSnapshot`](crate::catalogue::CatalogueSnapshot) gives:
/// [`Project`] derives `PartialEq` only, and adding a derive to `htui-core` for this crate's
/// convenience is not this milestone's to do.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    /// The `App` rung, in [`SettingKey::ALL`] order — always ten entries.
    pub app: Vec<AppEntry>,
    /// The scope's projects, in `scope.project_ids` order; an id that names no row is skipped
    /// (D3), exactly as [`crate::catalogue::snapshot`] skips one.
    pub projects: Vec<ProjectEntry>,
}

/// One key on the `App` rung.
///
/// `updated_at` is `None` exactly when no `app_setting` row exists — the state a set must pass
/// `expected: None` for (D4, F-2) — and `value` is `Some` exactly when `updated_at` is, on both
/// stores: the `App` rung has no "row without a value" state, unlike a project that holds the key
/// nowhere in its blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Which key.
    pub key: SettingKey,
    /// The stored JSON, or `None` when the rung holds none.
    pub value: Option<Value>,
    /// The row's `updated_at`: the compare-and-set token, `None` when there is no row.
    pub updated_at: Option<DateTime<Utc>>,
}

/// One project with the keys the `Project` rung accepts.
///
/// `project.updated_at` is the rung's compare-and-set token, held once per project rather than per
/// key (D4): the write is a key-level merge into one JSONB column. `project.settings` is carried
/// whole because the raw blob is what the reader's own resolvers take (D5/D6).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEntry {
    /// The project row.
    pub project: Project,
    /// Its settings, in [`project_keys`] order.
    pub values: Vec<ProjectValue>,
}

/// One key's value on the `Project` rung.
///
/// Read through `setting()` so [`SettingSpec::project_key`](htui_core::prompt::SettingSpec) is
/// applied (D2); `None` when the blob holds no such key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValue {
    /// Which key.
    pub key: SettingKey,
    /// The stored JSON, or `None` when the project's blob holds none.
    pub value: Option<Value>,
}

impl SettingsSnapshot {
    /// The entries that carry a value, keyed by `spec().key`.
    ///
    /// The shape `Backend::app_settings()` hands the resolvers, so D6's effective numbers are
    /// computed over the reader's own input rather than over a second spelling of it.
    #[must_use]
    pub fn app_map(&self) -> BTreeMap<String, Value> {
        self.app
            .iter()
            .filter_map(|entry| {
                entry
                    .value
                    .clone()
                    .map(|value| (entry.key.key().to_owned(), value))
            })
            .collect()
    }

    /// The `App` entry for `key`; `None` only on a snapshot built by hand with fewer than ten.
    #[must_use]
    pub fn app_entry(&self, key: SettingKey) -> Option<&AppEntry> {
        self.app.iter().find(|entry| entry.key == key)
    }
}

/// The keys whose spec admits the `Project` rung, in [`SettingKey::ALL`] order.
///
/// One iterator for the snapshot and for the section's rows (B-4): the section indexes
/// `ProjectEntry.values` by position, so the two cannot disagree on which keys are there or in
/// what order.
pub fn project_keys() -> impl Iterator<Item = SettingKey> {
    SettingKey::ALL
        .into_iter()
        .filter(|key| key.spec().rungs.contains(Rungs::PROJECT))
}

/// One read of the whole scope: the ten `App` rows with their tokens, then every project of
/// `scope.project_ids` that still names a row, with the keys the `Project` rung accepts.
///
/// N+1 reads on purpose (D3, M3 D5's trade, same words): ten `setting(App, _)` plus, per project,
/// one `project()` and one `setting(Project(id), _)` per project key. They happen per event —
/// activation, a scope change, after a write — never per keystroke.
///
/// A project that vanishes between its `project()` and its `setting()` calls yields `value: None`
/// rather than an error: a torn read is not a state to render, and the write that follows answers
/// the seam's own `NotFound` (H-2).
///
/// The bound is `ReadStore + WriteStore` rather than `ReadStore` alone because `setting` lives on
/// [`WriteStore`] (which extends [`ReadStore`]); only `project` is on the read half. Same shape as
/// [`crate::catalogue::snapshot`]'s bound.
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot<S: ReadStore + WriteStore + ?Sized>(
    store: &S,
    scope: &Scope,
) -> Result<SettingsSnapshot> {
    let mut app = Vec::with_capacity(SettingKey::ALL.len());
    for key in SettingKey::ALL {
        let stored = store.setting(SettingRung::App, key).await?;
        app.push(AppEntry {
            key,
            value: stored.as_ref().and_then(|row| row.value.clone()),
            updated_at: stored.map(|row| row.updated_at),
        });
    }

    let mut projects = Vec::with_capacity(scope.project_ids.len());
    for id in &scope.project_ids {
        let Some(project) = store.project(*id).await? else {
            continue;
        };
        let mut values = Vec::new();
        for key in project_keys() {
            let stored = store.setting(SettingRung::Project(*id), key).await?;
            values.push(ProjectValue {
                key,
                value: stored.and_then(|row| row.value),
            });
        }
        projects.push(ProjectEntry { project, values });
    }

    Ok(SettingsSnapshot { app, projects })
}

/// Serves one prompt settings request, off the UI task.
///
/// `Err(StoreError::Unreachable)` on [`Backend::Offline`], whose [`writer`](Backend::writer) is
/// `None` — including for the read, so the section's `unavailable` path is the same sentence every
/// other request gets there.
///
/// The module doc records the residue every write arm here shares with
/// [`crate::catalogue::serve`]: a re-read that fails after an applied write answers `Failed`.
///
/// # Errors
/// Whatever the seam reports, plus [`StoreError::Unreachable`] offline and
/// [`StoreError::Backend`] for a request that is not one of this module's three.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;

    match request {
        StoreRequest::PromptSettings(scope) => reread(&writer, scope).await,
        // `set` and `clear` are separate operations because clearing lets the rung below — and in
        // the end the compiled default — answer, rather than the editor guessing at a constant
        // (PRD D7, D10). `expected` keeps the seam's own asymmetry: `Option` on the set, where
        // `None` means "I expect no row" and is accepted on `App` alone (F-2), plain on the clear.
        StoreRequest::SetSetting {
            scope,
            rung,
            key,
            value,
            expected,
        } => {
            let outcome = writer
                .set_setting(*rung, *key, value.clone(), *expected)
                .await?;
            cas(&writer, scope, &outcome).await
        }
        StoreRequest::ClearSetting {
            scope,
            rung,
            key,
            expected,
        } => {
            let outcome = writer.clear_setting(*rung, *key, *expected).await?;
            cas(&writer, scope, &outcome).await
        }
        // `try_serve` routes exactly this module's three variants here, so the last arm is
        // unreachable from the shell; a caller that reached it anyway is better told which request
        // it sent than killed.
        other => Err(StoreError::Backend(format!(
            "not a prompt settings request: {}",
            other.name()
        ))),
    }
}

/// The settings as they are now, for a read or for a write that applied.
async fn reread(writer: &Writer, scope: &Scope) -> Result<StoreReply> {
    Ok(StoreReply::PromptSettings(Box::new(
        snapshot(writer, scope).await?,
    )))
}

/// A compare-and-set outcome as a reply: `Applied` answers the fresh settings, `Stale` answers the
/// same settings under [`StoreReply::PromptSettingsStale`] so the editor reloads and retries by
/// hand (D14, PRD D8).
///
/// The worker re-reads rather than handing the section the single row `Stale` carries: the section
/// renders a tree of rungs, and a row patched in locally would be a second source of truth (D3).
async fn cas<T>(writer: &Writer, scope: &Scope, outcome: &CasOutcome<T>) -> Result<StoreReply> {
    let fresh = Box::new(snapshot(writer, scope).await?);
    Ok(match outcome {
        CasOutcome::Applied(_) => StoreReply::PromptSettings(fresh),
        CasOutcome::Stale(_) => StoreReply::PromptSettingsStale(fresh),
    })
}

/// The three request names, in [`StoreRequest`] order.
///
/// [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here, so a fourth
/// request cannot be named in one place and matched in the other.
pub const REQUEST_NAMES: [&str; 3] = ["prompt_settings", "set_setting", "clear_setting"];

/// The **read**'s name, the one of the three a `Failed` is treated differently for: a refused read
/// leaves the section with no tree at all, where a refused write leaves the editor over its text.
///
/// Named rather than reached for as `REQUEST_NAMES[0]`, so the section's `Failed` arms say which
/// request they mean instead of relying on the order of the array above.
pub const READ_NAME: &str = REQUEST_NAMES[0];
