//! Did the new box reach Postgres, judged against a baseline taken before INSTALL (MOD-45 D305),
//! and its executor (D313). Both timestamps are the database's `clock_timestamp()`; no local clock
//! is used.
//!
//! The production verifier connects through [`worker_cmd::connect`] with this machine's own config
//! root, which registers or refreshes **this** box's row, as `htui --index-items` does (R-6). It
//! never connects with the remote identity.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use htui_core::model::{BoxEdit, BoxId, BoxRecord, Executor};
use htui_core::store::{CasOutcome, WriteStore as _};
use htui_store::PgStore;
use htui_store::pg::PoolSize;

use crate::worker_cmd;

/// `boxes()` as `{id → last_seen_at}`.
pub type Baseline = HashMap<BoxId, DateTime<Utc>>;

/// The local poll's cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poll {
    /// Between reads.
    pub interval: Duration,
    /// Overall.
    pub deadline: Duration,
}

/// The seam `run_with` verifies through (E-6: boxed futures, so `&dyn Verifier` is formable).
pub trait Verifier: core::fmt::Debug + Send + Sync {
    /// Connects (once) with `dsn` and lists the boxes. `Err` is the reason no baseline exists (the
    /// DSN reaches Postgres only from the remote network, a pending schema, …), never the DSN.
    fn baseline<'a>(&'a self, dsn: &'a str) -> BoxFuture<'a, Result<Baseline, String>>;

    /// Polls until `id` is new against `baseline` or its `last_seen_at` is later; `Err` is the
    /// reason it was not seen.
    fn box_seen<'a>(
        &'a self,
        id: BoxId,
        baseline: &'a Baseline,
        poll: Poll,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// D313: `executor = worker`, `Stale` retried once with the row it carries, skipped when
    /// already `worker` (E-23). `Ok(true)` when this call wrote it, `Ok(false)` when it already
    /// was (E-25 reports the difference); `Err` is the warning.
    fn set_executor(&self, id: BoxId) -> BoxFuture<'_, Result<bool, String>>;
}

/// The Postgres one.
#[derive(Debug)]
pub struct PgVerifier {
    root: PathBuf,
    store: tokio::sync::OnceCell<PgStore>,
}

impl PgVerifier {
    /// `root` is this machine's config root (`identity::config_root()`; a temp dir in tests).
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            store: tokio::sync::OnceCell::new(),
        }
    }

    /// The store [`Verifier::baseline`] connected, or the reason there is none.
    fn connected(&self) -> Result<&PgStore, String> {
        self.store
            .get()
            .ok_or_else(|| "no connection from here".to_owned())
    }
}

impl Verifier for PgVerifier {
    fn baseline<'a>(&'a self, dsn: &'a str) -> BoxFuture<'a, Result<Baseline, String>> {
        Box::pin(async move {
            let store = self
                .store
                .get_or_try_init(|| async {
                    worker_cmd::connect(dsn, &self.root, PoolSize::clamped(2))
                        .await
                        .map_err(|exit| exit.to_string())
                })
                .await?;
            let records = store.boxes().await.map_err(|err| err.to_string())?;
            Ok(records
                .into_iter()
                .map(|record| (record.row.id, record.row.last_seen_at))
                .collect())
        })
    }

    fn box_seen<'a>(
        &'a self,
        id: BoxId,
        baseline: &'a Baseline,
        poll: Poll,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let store = self.connected()?;
            let started = tokio::time::Instant::now();
            let mut last_error = None;
            loop {
                match store.boxes().await {
                    Ok(records) if seen_against(&records, id, baseline) => return Ok(()),
                    Ok(_) => {}
                    Err(err) => last_error = Some(err.to_string()),
                }
                if started.elapsed() >= poll.deadline {
                    break;
                }
                tokio::time::sleep(poll.interval).await;
            }
            let mut reason = format!(
                "box {id} did not check in within {} s",
                poll.deadline.as_secs()
            );
            if let Some(err) = last_error {
                reason.push_str(&format!(" (the last read failed: {err})"));
            }
            Err(reason)
        })
    }

    fn set_executor(&self, id: BoxId) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(async move {
            let store = self.connected()?;
            let records = store.boxes().await.map_err(|err| err.to_string())?;
            let Some(record) = records.into_iter().find(|record| record.row.id == id) else {
                return Err(format!("box {id} is not in Postgres"));
            };
            let mut row = record.row;
            for _ in 0..2 {
                if matches!(Executor::of(&row.settings), Executor::Worker) {
                    return Ok(false);
                }
                let edit = BoxEdit {
                    executor: Some(Executor::Worker),
                    ..BoxEdit::default()
                };
                match store.edit_box(id, row.edit_version, edit).await {
                    Ok(CasOutcome::Applied(_)) => return Ok(true),
                    Ok(CasOutcome::Stale(current)) => row = current,
                    Err(err) => return Err(err.to_string()),
                }
            }
            Err("the box was edited elsewhere while its executor was being set".to_owned())
        })
    }
}

/// Whether `records` show `id` new against `baseline` or checked in since. Pure.
#[must_use]
pub fn seen_against(records: &[BoxRecord], id: BoxId, baseline: &Baseline) -> bool {
    records
        .iter()
        .find(|record| record.row.id == id)
        .is_some_and(|record| {
            baseline
                .get(&id)
                .is_none_or(|before| record.row.last_seen_at > *before)
        })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use htui_core::model::{BoxRow, OsFamily, UserId};

    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0)
            .single()
            .expect("a time")
    }

    fn record(id: BoxId, last_seen_at: DateTime<Utc>) -> BoxRecord {
        BoxRecord {
            row: BoxRow {
                id,
                user_id: UserId::from_uuid(uuid::Uuid::nil()),
                hostname: "box1".to_owned(),
                os_family: OsFamily::Linux,
                os_version: "6.8".to_owned(),
                arch: "x86_64".to_owned(),
                cpu: String::new(),
                ram_mb: None,
                gpu_present: false,
                gpu_vendor: None,
                htui_version: "0.1.0".to_owned(),
                probed_tags: Vec::new(),
                declared_tags: Vec::new(),
                quirks: String::new(),
                settings: serde_json::json!({}),
                registered_at: at(0),
                last_seen_at,
                last_probed_at: None,
                updated_at: at(0),
                edit_version: 0,
            },
            tools: Vec::new(),
            probe_spec_digest: None,
        }
    }

    #[test]
    fn seen_against_new_moved_and_unchanged() {
        let id = BoxId::from_uuid(uuid::Uuid::from_u128(1));
        let other = BoxId::from_uuid(uuid::Uuid::from_u128(2));
        let empty = Baseline::new();
        assert!(seen_against(&[record(id, at(5))], id, &empty), "new");

        let before: Baseline = [(id, at(5))].into_iter().collect();
        assert!(
            !seen_against(&[record(id, at(5))], id, &before),
            "unchanged"
        );
        assert!(seen_against(&[record(id, at(6))], id, &before), "later");
        assert!(!seen_against(&[record(other, at(9))], id, &empty), "absent");
    }

    #[tokio::test]
    async fn box_seen_and_set_executor_need_a_connection() {
        let verifier = PgVerifier::new(PathBuf::from("/nonexistent/htui-provision-test"));
        let id = BoxId::from_uuid(uuid::Uuid::from_u128(1));
        let poll = Poll {
            interval: Duration::from_millis(1),
            deadline: Duration::from_millis(1),
        };
        assert_eq!(
            verifier.box_seen(id, &Baseline::new(), poll).await,
            Err("no connection from here".to_owned())
        );
        assert_eq!(
            verifier.set_executor(id).await,
            Err("no connection from here".to_owned())
        );
    }
}
