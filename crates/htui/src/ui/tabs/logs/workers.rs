//! The Logs tab's worker sources (MOD-122 M2 D10): one per headless worker box that shipped a
//! line, read from `worker_log` through [`StoreRequest::WorkerLogs`].
//!
//! Each source keeps a ring of [`CAPACITY`] lines, as the process's own buffer does, and counts
//! what it retains. A line is kept as a [`LogLine`] whose `seq` is the row's id, so the tab's
//! scroll, level filter and line drawing work on a worker source unchanged; the id space is
//! the database's, shared by every worker source and unrelated to the process's sequences.
//!
//! The tab re-reads from [`REREAD`] ids below the highest it holds (claim V10): a row a
//! concurrent transaction committed late carries a lower id than one already read, so reading
//! strictly past the highest id would miss it. The overlap is dropped by id, and a late row lands
//! in id order.
//!
//! [`StoreRequest::WorkerLogs`]: crate::store_worker::StoreRequest::WorkerLogs

use std::collections::{BTreeMap, VecDeque};

use htui_core::model::{BoxId, WorkerLogLevel, WorkerLogLine};
use tracing::Level;

use crate::logging::{CAPACITY, Counts, LogLine, Source};

/// How far below the highest held id the next read starts (plan D10, claim V10).
pub(super) const REREAD: i64 = 500;

/// Which worker source a row belongs to: its box, or, once the box is deleted (`box_id` is
/// `None`, plan D6), the box's name as the row kept it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum WorkerKey {
    /// A box that still exists.
    Box(BoxId),
    /// A deleted box, by the hostname its rows kept.
    Removed(String),
}

/// One worker source: its newest name and the lines it retains, oldest (lowest id) first.
#[derive(Debug, Default)]
pub(super) struct WorkerSource {
    /// The box's hostname as its newest row has it.
    name: String,
    /// Retained lines in id order.
    lines: VecDeque<LogLine>,
    /// Counts of the retained lines.
    counts: Counts,
}

impl WorkerSource {
    /// The retained lines, oldest first.
    pub(super) const fn lines(&self) -> &VecDeque<LogLine> {
        &self.lines
    }

    /// Lines and warnings retained.
    pub(super) const fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether a line with this `seq` is retained.
    fn holds(&self, seq: u64) -> bool {
        self.lines
            .binary_search_by_key(&seq, |line| line.seq)
            .is_ok()
    }

    /// Inserts `line` in id order unless its id is already held; drops the oldest past
    /// [`CAPACITY`].
    fn insert(&mut self, line: LogLine) {
        let Err(at) = self.lines.binary_search_by_key(&line.seq, |held| held.seq) else {
            return;
        };
        add(&mut self.counts, line.level);
        self.lines.insert(at, line);
        while self.lines.len() > CAPACITY {
            if let Some(dropped) = self.lines.pop_front() {
                remove(&mut self.counts, dropped.level);
            }
        }
    }
}

/// Every worker source the tab has read, and the highest id it has seen.
#[derive(Debug, Default)]
pub(super) struct Workers {
    /// The sources by key; listed through [`Workers::keys`].
    sources: BTreeMap<WorkerKey, WorkerSource>,
    /// The highest row id merged so far.
    max_id: Option<i64>,
}

impl Workers {
    /// The next read's cursor: `None` (the newest page) before any row, else [`REREAD`] below
    /// the highest id held.
    pub(super) fn after(&self) -> Option<i64> {
        self.max_id.map(|max| max.saturating_sub(REREAD).max(0))
    }

    /// The sequence one past the highest id held: where a paused view with nothing to anchor on
    /// waits.
    pub(super) fn next(&self) -> u64 {
        self.max_id.map_or(0, |max| seq_of(max).saturating_add(1))
    }

    /// Adds one read's rows: a row already held (by id, in any source) is skipped, a late lower
    /// id lands in order. An empty read (offline, blueprint F5) adds nothing and drops nothing.
    pub(super) fn merge(&mut self, rows: &[WorkerLogLine]) {
        for row in rows {
            let seq = seq_of(row.id);
            // Only a re-read row can be held already, and only one at or below the highest id.
            if self.max_id.is_some_and(|max| row.id <= max)
                && self.sources.values().any(|source| source.holds(seq))
            {
                continue;
            }
            let key = row
                .box_id
                .map_or_else(|| WorkerKey::Removed(row.box_name.clone()), WorkerKey::Box);
            let source = self.sources.entry(key).or_default();
            if source.lines.back().is_none_or(|newest| newest.seq < seq) {
                source.name.clone_from(&row.box_name);
            }
            source.insert(line_of(row, seq));
            self.max_id = Some(self.max_id.map_or(row.id, |max| max.max(row.id)));
        }
    }

    /// The listed sources: every one holding a line, live boxes before removed ones, each by
    /// name.
    pub(super) fn keys(&self) -> Vec<WorkerKey> {
        let mut keys: Vec<(&WorkerKey, &WorkerSource)> = self
            .sources
            .iter()
            .filter(|(_, source)| !source.lines.is_empty())
            .collect();
        keys.sort_by(|(a, one), (b, other)| {
            matches!(a, WorkerKey::Removed(_))
                .cmp(&matches!(b, WorkerKey::Removed(_)))
                .then_with(|| one.name.cmp(&other.name))
                .then_with(|| a.cmp(b))
        });
        keys.into_iter().map(|(key, _)| key.clone()).collect()
    }

    /// The source `key` names, if it holds a line.
    pub(super) fn get(&self, key: &WorkerKey) -> Option<&WorkerSource> {
        self.sources
            .get(key)
            .filter(|source| !source.lines.is_empty())
    }

    /// The label the sources list and the lines pane's border show: `worker · <name>`, and
    /// `worker · <name> (removed)` for a deleted box.
    pub(super) fn label(&self, key: &WorkerKey) -> String {
        match key {
            // A box the tab holds no row of has no name yet: its id's first group stands in.
            WorkerKey::Box(id) => match self.sources.get(key) {
                Some(source) => format!("worker \u{b7} {}", source.name),
                None => {
                    let id = id.to_string();
                    let short = id.split('-').next().unwrap_or(&id);
                    format!("worker \u{b7} {short}")
                }
            },
            WorkerKey::Removed(name) => format!("worker \u{b7} {name} (removed)"),
        }
    }
}

/// A row id as a [`LogLine`] sequence; ids rise from 1, so a negative one cannot occur.
fn seq_of(id: i64) -> u64 {
    u64::try_from(id).unwrap_or(0)
}

/// The tab's line for one row: the worker's clock, its level and target, and the target's
/// [`Source`] for the colour-free column under a worker row (plan D10).
fn line_of(row: &WorkerLogLine, seq: u64) -> LogLine {
    LogLine {
        seq,
        at: row.logged_at,
        level: level_of(row.level),
        target: row.target.clone(),
        source: Source::of(&row.target),
        text: row.text.clone(),
    }
}

/// A stored level as `tracing`'s.
const fn level_of(level: WorkerLogLevel) -> Level {
    match level {
        WorkerLogLevel::Error => Level::ERROR,
        WorkerLogLevel::Warn => Level::WARN,
        WorkerLogLevel::Info => Level::INFO,
        WorkerLogLevel::Debug => Level::DEBUG,
        WorkerLogLevel::Trace => Level::TRACE,
    }
}

/// `WARN` and `ERROR` count as warnings, as the process's own buffer counts them.
fn is_warning(level: Level) -> bool {
    level == Level::WARN || level == Level::ERROR
}

/// One more retained line at `level`.
fn add(counts: &mut Counts, level: Level) {
    counts.lines += 1;
    if is_warning(level) {
        counts.warnings += 1;
    }
}

/// One fewer retained line at `level`.
fn remove(counts: &mut Counts, level: Level) {
    counts.lines = counts.lines.saturating_sub(1);
    if is_warning(level) {
        counts.warnings = counts.warnings.saturating_sub(1);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};
    use uuid::Uuid;

    use super::*;

    /// The first fixture box.
    pub(in crate::ui::tabs::logs) const BUILD: BoxId =
        BoxId::from_uuid(Uuid::from_u128(0xb0b1_u128 << 96));
    /// The second fixture box.
    pub(in crate::ui::tabs::logs) const GPU: BoxId =
        BoxId::from_uuid(Uuid::from_u128(0x6b02_u128 << 96));

    /// A fixed worker clock, so no snapshot moves with the clock.
    fn at(id: i64) -> DateTime<Utc> {
        let second = u32::try_from(id.rem_euclid(60)).expect("under sixty");
        Utc.with_ymd_and_hms(2026, 11, 5, 10, 15, second)
            .single()
            .expect("a valid time")
    }

    /// One stored row: `id`, from `box_id` (`None` once deleted) named `name`.
    pub(in crate::ui::tabs::logs) fn row(
        id: i64,
        box_id: Option<BoxId>,
        name: &str,
        level: WorkerLogLevel,
        target: &str,
        text: &str,
    ) -> WorkerLogLine {
        WorkerLogLine {
            id,
            box_id,
            box_name: name.to_owned(),
            logged_at: at(id),
            received_at: at(id),
            level,
            target: target.to_owned(),
            text: text.to_owned(),
        }
    }

    /// An `info` row from `BUILD-01`'s run runtime.
    fn build(id: i64) -> WorkerLogLine {
        row(
            id,
            Some(BUILD),
            "BUILD-01",
            WorkerLogLevel::Info,
            "htui_worker::walk",
            &format!("line {id}"),
        )
    }

    fn ids(workers: &Workers, key: &WorkerKey) -> Vec<u64> {
        workers
            .get(key)
            .map(|source| source.lines().iter().map(|line| line.seq).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_ring_keeps_capacity_lines_and_its_counts_follow_the_drop() {
        let mut workers = Workers::default();
        let mut rows: Vec<WorkerLogLine> = (1..=5)
            .map(|id| {
                let mut warn = build(id);
                warn.level = WorkerLogLevel::Warn;
                warn
            })
            .collect();
        let last = i64::try_from(CAPACITY).expect("fits") + 5;
        rows.extend((6..=last).map(build));
        workers.merge(&rows);
        let key = WorkerKey::Box(BUILD);
        let source = workers.get(&key).expect("listed");
        assert_eq!(source.lines().len(), CAPACITY);
        assert_eq!(
            source.counts(),
            Counts {
                lines: CAPACITY,
                warnings: 0
            },
            "the five warnings were the oldest and went first"
        );
        assert_eq!(source.lines().front().map(|line| line.seq), Some(6));
    }

    #[test]
    fn a_repeated_id_is_not_added_twice() {
        let mut workers = Workers::default();
        workers.merge(&[build(1), build(2), build(3)]);
        workers.merge(&[build(2), build(3), build(4)]);
        let key = WorkerKey::Box(BUILD);
        assert_eq!(ids(&workers, &key), [1, 2, 3, 4]);
        assert_eq!(workers.get(&key).expect("listed").counts().lines, 4);
    }

    /// Claim V10: a row a concurrent transaction committed late carries a lower id than one
    /// already read; the re-read finds it and it lands in id order.
    #[test]
    fn a_lower_id_arriving_late_lands_in_order() {
        let mut workers = Workers::default();
        workers.merge(&[build(10), build(12)]);
        assert_eq!(workers.after(), Some(0), "the margin never goes below zero");
        workers.merge(&[build(11), build(12), build(13)]);
        assert_eq!(ids(&workers, &WorkerKey::Box(BUILD)), [10, 11, 12, 13]);
    }

    #[test]
    fn the_cursor_re_reads_a_margin_below_the_highest_id() {
        let mut workers = Workers::default();
        assert_eq!(workers.after(), None, "the newest page first");
        workers.merge(&[build(2_000)]);
        assert_eq!(workers.after(), Some(2_000 - REREAD));
        workers.merge(&[]);
        assert_eq!(
            workers.after(),
            Some(2_000 - REREAD),
            "an empty read keeps it"
        );
        assert_eq!(ids(&workers, &WorkerKey::Box(BUILD)), [2_000]);
    }

    #[test]
    fn warnings_are_warn_and_error_lines() {
        let mut workers = Workers::default();
        let rows: Vec<WorkerLogLine> = WorkerLogLevel::ALL
            .iter()
            .zip(1..)
            .map(|(level, id)| {
                let mut line = build(id);
                line.level = *level;
                line
            })
            .collect();
        workers.merge(&rows);
        assert_eq!(
            workers
                .get(&WorkerKey::Box(BUILD))
                .expect("listed")
                .counts(),
            Counts {
                lines: 5,
                warnings: 2
            }
        );
    }

    #[test]
    fn a_box_with_no_lines_is_not_listed() {
        let mut workers = Workers::default();
        assert!(workers.keys().is_empty());
        workers.merge(&[]);
        assert!(workers.keys().is_empty());
        assert!(workers.get(&WorkerKey::Box(GPU)).is_none());
        workers.merge(&[build(1)]);
        assert_eq!(workers.keys(), [WorkerKey::Box(BUILD)]);
    }

    /// Plan D6: a deleted box's rows (`box_id` `None`) are their own source, keyed by the name
    /// they kept, listed after the live boxes.
    #[test]
    fn a_deleted_box_is_its_own_removed_source_keyed_by_name() {
        let mut workers = Workers::default();
        let gone = row(
            3,
            None,
            "OLD-RIG",
            WorkerLogLevel::Error,
            "htui_worker",
            "walk failed",
        );
        let gpu = row(
            4,
            Some(GPU),
            "AAA-GPU",
            WorkerLogLevel::Warn,
            "htui_store::pg",
            "slow",
        );
        workers.merge(&[build(1), gone, gpu]);
        let removed = WorkerKey::Removed("OLD-RIG".to_owned());
        assert_eq!(
            workers.keys(),
            [WorkerKey::Box(GPU), WorkerKey::Box(BUILD), removed.clone()],
            "live boxes by name, then the removed one"
        );
        assert_eq!(workers.label(&removed), "worker \u{b7} OLD-RIG (removed)");
        assert_eq!(workers.label(&WorkerKey::Box(GPU)), "worker \u{b7} AAA-GPU");
        assert_eq!(ids(&workers, &removed), [3]);
    }

    /// A box renamed between rows is labelled by its newest row's name.
    #[test]
    fn a_source_takes_its_newest_row_s_name() {
        let mut workers = Workers::default();
        let mut renamed = build(2);
        renamed.box_name = "BUILD-02".to_owned();
        workers.merge(&[renamed, build(1)]);
        assert_eq!(
            workers.label(&WorkerKey::Box(BUILD)),
            "worker \u{b7} BUILD-02"
        );
    }

    /// A worker line maps its target through `Source::of`, for the column only.
    #[test]
    fn a_worker_line_keeps_its_target_and_its_source() {
        let mut workers = Workers::default();
        workers.merge(&[row(
            1,
            Some(BUILD),
            "BUILD-01",
            WorkerLogLevel::Warn,
            "htui_store::pg",
            "slow",
        )]);
        let source = workers.get(&WorkerKey::Box(BUILD)).expect("listed");
        let line = source.lines().front().expect("one line");
        assert_eq!(line.source, Source::Store);
        assert_eq!(line.level, Level::WARN);
        assert_eq!(line.target, "htui_store::pg");
    }
}
