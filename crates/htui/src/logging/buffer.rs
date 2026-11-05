//! The in-memory log (MOD-122 D2): a bounded ring of formatted lines with per-source counts, and
//! the [`MemoryLayer`] that fills it from every event that passed the global filter.

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use tracing::Level;
use tracing::field::Field;

use super::source::Source;

/// MOD-122 D2: lines kept; the oldest goes first.
pub const CAPACITY: usize = 10_000;

/// One log line as the Logs tab shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// From 0, never reused: the tab asks for lines from the one after the last it saw.
    pub seq: u64,
    /// When the event happened (wall clock).
    pub at: DateTime<Utc>,
    /// The event's level.
    pub level: Level,
    /// The event's target (a `log` record's own target, not `log`).
    pub target: String,
    /// The source the target maps to (D3).
    pub source: Source,
    /// The message, then ` key=value` for each field, on one line.
    pub text: String,
}

/// Lines and warnings of one source, or of all; warnings are `WARN` + `ERROR` lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Retained lines.
    pub lines: usize,
    /// Retained `WARN` and `ERROR` lines.
    pub warnings: usize,
}

impl Counts {
    fn add(&mut self, level: Level) {
        self.lines += 1;
        if is_warning(level) {
            self.warnings += 1;
        }
    }

    fn remove(&mut self, level: Level) {
        self.lines = self.lines.saturating_sub(1);
        if is_warning(level) {
            self.warnings = self.warnings.saturating_sub(1);
        }
    }
}

/// `WARN` and `ERROR` count as warnings.
fn is_warning(level: Level) -> bool {
    level == Level::WARN || level == Level::ERROR
}

/// The counts of every source (D2: of retained lines, so a count matches what the list can show).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceCounts([Counts; Source::COUNT]);

impl SourceCounts {
    /// The counts of `source`.
    #[must_use]
    pub const fn of(&self, source: Source) -> Counts {
        self.0[source.index()]
    }

    /// The counts of every source together.
    #[must_use]
    pub fn all(&self) -> Counts {
        self.0.iter().fold(Counts::default(), |sum, counts| Counts {
            lines: sum.lines + counts.lines,
            warnings: sum.warnings + counts.warnings,
        })
    }
}

/// What one [`LogBuffer::since`] read under one lock.
#[derive(Debug, Clone, Default)]
pub struct Since {
    /// Every retained line with a sequence at or after the one asked for, oldest first.
    pub lines: Vec<LogLine>,
    /// The sequence the next line will take: ask from here next time.
    pub next: u64,
    /// The oldest retained sequence (`next` when the buffer is empty).
    pub oldest: u64,
    /// The counts of the retained lines.
    pub counts: SourceCounts,
}

/// The ring behind a `Mutex`, held only to push or copy; no `tracing` call happens inside it
/// (plan risk "reentrancy"). Clone = the same ring.
#[derive(Clone)]
pub struct LogBuffer(Arc<Mutex<Ring>>);

struct Ring {
    lines: VecDeque<LogLine>,
    next: u64,
    counts: SourceCounts,
    capacity: usize,
}

impl LogBuffer {
    /// An empty buffer of [`CAPACITY`] lines.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Ring {
            lines: VecDeque::new(),
            next: 0,
            counts: SourceCounts::default(),
            capacity: CAPACITY,
        })))
    }

    /// Adds a line, dropping the oldest at capacity; returns the new line's sequence.
    pub fn push(&self, at: DateTime<Utc>, level: Level, target: &str, text: String) -> u64 {
        let source = Source::of(target);
        let target = target.to_owned();
        let mut ring = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if ring.lines.len() >= ring.capacity
            && let Some(dropped) = ring.lines.pop_front()
        {
            ring.counts.0[dropped.source.index()].remove(dropped.level);
        }
        let seq = ring.next;
        ring.next += 1;
        ring.counts.0[source.index()].add(level);
        ring.lines.push_back(LogLine {
            seq,
            at,
            level,
            target,
            source,
            text,
        });
        seq
    }

    /// Every retained line from `seq` on, with the next sequence, the oldest one and the counts.
    #[must_use]
    pub fn since(&self, seq: u64) -> Since {
        let ring = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let oldest = ring.lines.front().map_or(ring.next, |line| line.seq);
        // Sequences are contiguous, so the index of `seq` is its distance from the oldest.
        let skip = usize::try_from(seq.saturating_sub(oldest)).unwrap_or(usize::MAX);
        Since {
            lines: ring.lines.iter().skip(skip).cloned().collect(),
            next: ring.next,
            oldest,
            counts: ring.counts,
        }
    }

    /// How many lines are retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lines
            .len()
    }

    /// No line is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LogBuffer {
    /// The size only, never the lines.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogBuffer")
            .field("lines", &self.len())
            .finish()
    }
}

/// MOD-122 D1, D2: every event that passed the global filter, into the buffer.
#[derive(Debug, Clone)]
pub struct MemoryLayer {
    buffer: LogBuffer,
}

impl MemoryLayer {
    /// A layer that pushes into `buffer`.
    #[must_use]
    pub const fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for MemoryLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut line = LineVisitor::default();
        // Formatted before the ring's lock is taken.
        event.record(&mut line);
        let meta = event.metadata();
        let target = line
            .log_target
            .take()
            .unwrap_or_else(|| meta.target().to_owned());
        self.buffer
            .push(Utc::now(), *meta.level(), &target, line.text());
    }
}

/// Formats an event's fields as the `fmt` layer does: the message, then ` key=value`.
#[derive(Default)]
struct LineVisitor {
    message: String,
    fields: String,
    log_target: Option<String>,
}

impl tracing::field::Visit for LineVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message.push_str(value),
            // A `log` record bridged by tracing-log carries its real target here.
            "log.target" => self.log_target = Some(value.to_owned()),
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.fields, " {name}={value:?}");
            }
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => {
                let _ = write!(self.message, "{value:?}");
            }
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.fields, " {name}={value:?}");
            }
        }
    }
}

impl LineVisitor {
    /// The message then the fields; without a message, the fields alone; `\r` and `\n` folded to
    /// a space, so a line is one row. Most lines hold neither and are not copied again (MOD-122
    /// R1).
    fn text(&self) -> String {
        let text = if self.message.is_empty() {
            self.fields.trim_start().to_owned()
        } else {
            format!("{}{}", self.message, self.fields)
        };
        if text.contains(['\r', '\n']) {
            text.replace("\r\n", " ").replace(['\r', '\n'], " ")
        } else {
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt as _;

    use super::{CAPACITY, Counts, LogBuffer, MemoryLayer};
    use crate::logging::source::Source;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 5, 9, 30, 0)
            .single()
            .expect("a valid time")
    }

    /// Runs `f` with a scoped subscriber that holds only a [`MemoryLayer`] over a new buffer.
    fn captured(f: impl FnOnce()) -> LogBuffer {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(MemoryLayer::new(buffer.clone()));
        tracing::subscriber::with_default(subscriber, f);
        buffer
    }

    #[test]
    fn a_pushed_line_keeps_its_time_level_target_source_and_text() {
        let buffer = LogBuffer::new();
        assert!(buffer.is_empty());
        let first = buffer.push(
            at(),
            Level::WARN,
            "htui_orch::engine",
            "walk stalled".into(),
        );
        let second = buffer.push(at(), Level::INFO, "htui::app", "hello".into());
        assert_eq!((first, second), (0, 1));
        assert_eq!(buffer.len(), 2);
        let line = &buffer.since(0).lines[0];
        assert_eq!(line.seq, 0);
        assert_eq!(line.at, at());
        assert_eq!(line.level, Level::WARN);
        assert_eq!(line.target, "htui_orch::engine");
        assert_eq!(line.source, Source::Orchestrator);
        assert_eq!(line.text, "walk stalled");
        assert_eq!(format!("{buffer:?}"), "LogBuffer { lines: 2 }");
    }

    #[test]
    fn since_returns_the_lines_from_the_sequence_asked_and_the_next() {
        let buffer = LogBuffer::new();
        let empty = buffer.since(0);
        assert!(empty.lines.is_empty());
        assert_eq!((empty.next, empty.oldest), (0, 0));
        for text in ["a", "b", "c"] {
            buffer.push(at(), Level::INFO, "htui", text.into());
        }
        let all = buffer.since(0);
        assert_eq!(all.lines.len(), 3);
        assert_eq!((all.next, all.oldest), (3, 0));
        assert!(buffer.since(all.next).lines.is_empty());
        let tail: Vec<String> = buffer.since(1).lines.into_iter().map(|l| l.text).collect();
        assert_eq!(tail, ["b", "c"]);
    }

    #[test]
    fn the_oldest_line_drops_at_ten_thousand_and_one() {
        let buffer = LogBuffer::new();
        for n in 0..=CAPACITY {
            buffer.push(at(), Level::INFO, "htui", format!("line {n}"));
        }
        assert_eq!(buffer.len(), CAPACITY);
        let since = buffer.since(0);
        assert_eq!(since.oldest, 1);
        assert_eq!(since.next, CAPACITY as u64 + 1);
        assert_eq!(since.lines.len(), CAPACITY);
        assert_eq!(since.lines[0].text, "line 1");
        assert_eq!(buffer.since(5).lines[0].seq, 5);
    }

    #[test]
    fn counts_follow_a_dropped_line() {
        let buffer = LogBuffer::new();
        buffer.push(at(), Level::WARN, "htui_store::cache", "w".into());
        assert_eq!(
            buffer.since(0).counts.of(Source::Store),
            Counts {
                lines: 1,
                warnings: 1
            }
        );
        for _ in 0..CAPACITY {
            buffer.push(at(), Level::INFO, "htui::app", "i".into());
        }
        let counts = buffer.since(0).counts;
        assert_eq!(counts.of(Source::Store), Counts::default());
        assert_eq!(
            counts.of(Source::Ui),
            Counts {
                lines: CAPACITY,
                warnings: 0
            }
        );
        assert_eq!(
            counts.all(),
            Counts {
                lines: CAPACITY,
                warnings: 0
            }
        );
    }

    #[test]
    fn only_warn_and_error_count_as_warnings() {
        let buffer = LogBuffer::new();
        for level in [
            Level::TRACE,
            Level::DEBUG,
            Level::INFO,
            Level::WARN,
            Level::ERROR,
        ] {
            buffer.push(at(), level, "htui", "x".into());
        }
        let counts = buffer.since(0).counts;
        assert_eq!(
            counts.of(Source::Ui),
            Counts {
                lines: 5,
                warnings: 2
            }
        );
        assert_eq!(counts.all(), counts.of(Source::Ui));
    }

    #[test]
    fn the_memory_layer_puts_the_message_and_fields_on_one_line() {
        let buffer = captured(|| {
            tracing::warn!(target: "htui_store::cache", rows = 3, path = "x", "cache refreshed");
        });
        let lines = buffer.since(0).lines;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, r#"cache refreshed rows=3 path="x""#);
        assert_eq!(lines[0].level, Level::WARN);
        assert_eq!(lines[0].target, "htui_store::cache");
        assert_eq!(lines[0].source, Source::Store);
    }

    #[test]
    fn an_event_without_a_message_keeps_its_fields_and_a_line_break_folds() {
        let buffer = captured(|| {
            tracing::info!(target: "htui", n = 1);
            tracing::info!(target: "htui", "a\nb");
            tracing::info!(target: "htui", "c\r\nd");
        });
        let texts: Vec<String> = buffer.since(0).lines.into_iter().map(|l| l.text).collect();
        assert_eq!(texts, ["n=1", "a b", "c d"]);
    }

    #[test]
    fn a_log_record_is_sourced_by_its_log_target() {
        let buffer = captured(|| {
            tracing::info!(target: "log", { "log.target" = "sqlx::query", "log.module_path" = "sqlx" }, "q");
        });
        let lines = buffer.since(0).lines;
        assert_eq!(lines[0].target, "sqlx::query");
        assert_eq!(lines[0].source, Source::Store);
        assert_eq!(lines[0].text, "q");
    }
}
