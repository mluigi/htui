//! Worker log lines (MOD-122 milestone 2, plan D1, D6, D7; `0032_worker_log.sql`).
//!
//! A headless `htui worker` ships its own log lines into `worker_log`, so the TUI can read them
//! from the Logs tab. [`NewWorkerLog`] is what the worker hands the store, [`WorkerLogLine`] is a
//! stored row as the TUI reads it, and [`LogShipLevel`] is the per-box level the worker ships at
//! (`box.settings.log_level`).

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::BoxId;

str_enum! {
    /// `worker_log.level`, in `chk_worker_log_level`'s order: the most severe first.
    WorkerLogLevel {
        /// A failure. The only level the Logs tab counts as an error (plan D5), and the only one
        /// that outlives its box (plan D6).
        Error => "error",
        /// A warning, an ordinary line.
        Warn => "warn",
        /// Progress.
        Info => "info",
        /// Detail.
        Debug => "debug",
        /// Finest detail. The worker never ships it (no [`LogShipLevel`] reaches it), but a row
        /// may carry it.
        Trace => "trace",
    }
}

str_enum! {
    /// How much of its log a headless worker ships to the database (plan D7), read from its own
    /// `box.settings.log_level`. Not a column: the text is a key of the settings blob, read
    /// leniently through [`LogShipLevel::of`].
    #[derive(Default)]
    LogShipLevel {
        /// Ships nothing. The worker's stderr and file log are unchanged.
        Off => "off",
        /// `error` lines only.
        Error => "error",
        /// `warn` and above: the default.
        #[default]
        Warn => "warn",
        /// `info` and above.
        Info => "info",
        /// `debug` and above.
        Debug => "debug",
    }
}

impl LogShipLevel {
    /// The `box.settings` key.
    pub const KEY: &'static str = "log_level";

    /// The lenient mapping of one JSON value: a known string is its level, anything else is the
    /// default, [`LogShipLevel::Warn`].
    #[must_use]
    pub fn from_value(value: &Value) -> Self {
        value
            .as_str()
            .and_then(|text| text.parse().ok())
            .unwrap_or_default()
    }

    /// The level a settings blob names (blueprint F6): the `log_level` key alone, so a malformed
    /// sibling key cannot change it. A missing or unknown value, or a blob that is not an object,
    /// is the default.
    #[must_use]
    pub fn of(settings: &Value) -> Self {
        settings
            .as_object()
            .and_then(|map| map.get(Self::KEY))
            .map_or_else(Self::default, Self::from_value)
    }

    /// Whether a line at `level` ships at this setting. [`LogShipLevel::Off`] ships nothing.
    #[must_use]
    pub const fn ships(self, level: WorkerLogLevel) -> bool {
        let threshold = match self {
            Self::Off => return false,
            Self::Error => WorkerLogLevel::Error,
            Self::Warn => WorkerLogLevel::Warn,
            Self::Info => WorkerLogLevel::Info,
            Self::Debug => WorkerLogLevel::Debug,
        };
        // `ALL` order is severity order, the most severe first.
        (level as u8) <= (threshold as u8)
    }
}

/// The most bytes one stored line's text keeps (plan T1): a longer text is cut at a char
/// boundary and ends in `…`, the whole within this many bytes.
pub const WORKER_LOG_TEXT_MAX: usize = 8 * 1024;

/// One line a worker hands [`WorkerLogStore::append_worker_logs`](crate::store::WorkerLogStore).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewWorkerLog {
    /// The worker's clock when the line was logged: display only, retention keys on the
    /// database's `received_at`.
    pub logged_at: DateTime<Utc>,
    /// The line's level.
    pub level: WorkerLogLevel,
    /// The `tracing` target.
    pub target: String,
    /// The formatted line, already scrubbed (plan D9).
    pub text: String,
}

impl NewWorkerLog {
    /// The target as both stores write it: a NUL, which Postgres text refuses, becomes U+FFFD.
    #[must_use]
    pub fn stored_target(&self) -> Cow<'_, str> {
        without_nul(&self.target)
    }

    /// The text as both stores write it: NULs replaced as in [`NewWorkerLog::stored_target`],
    /// then capped at [`WORKER_LOG_TEXT_MAX`] bytes, cut at a char boundary with a `…` suffix.
    #[must_use]
    pub fn stored_text(&self) -> Cow<'_, str> {
        let text = without_nul(&self.text);
        if text.len() <= WORKER_LOG_TEXT_MAX {
            return text;
        }
        let mut end = WORKER_LOG_TEXT_MAX - '…'.len_utf8();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut capped = String::with_capacity(end + '…'.len_utf8());
        capped.push_str(&text[..end]);
        capped.push('…');
        Cow::Owned(capped)
    }
}

/// `text` with every NUL replaced by U+FFFD; borrowed when there is none.
fn without_nul(text: &str) -> Cow<'_, str> {
    if text.contains('\0') {
        Cow::Owned(text.replace('\0', "\u{FFFD}"))
    } else {
        Cow::Borrowed(text)
    }
}

/// One stored `worker_log` row, as the TUI reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLogLine {
    /// `worker_log.id`: rises with every insert, the read order and the TUI's cursor.
    pub id: i64,
    /// The box that wrote it; `None` once the box is deleted (plan D6).
    pub box_id: Option<BoxId>,
    /// The box's hostname when the line was written; kept after the box is deleted.
    pub box_name: String,
    /// The worker's clock (display only).
    pub logged_at: DateTime<Utc>,
    /// The database's clock; retention keys on it.
    pub received_at: DateTime<Utc>,
    /// The line's level.
    pub level: WorkerLogLevel,
    /// The `tracing` target.
    pub target: String,
    /// The line's text.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(text: &str) -> NewWorkerLog {
        NewWorkerLog {
            logged_at: DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp"),
            level: WorkerLogLevel::Info,
            target: "htui::worker".to_owned(),
            text: text.to_owned(),
        }
    }

    /// `chk_worker_log_level`'s five texts, in its order, each round-tripping.
    #[test]
    fn worker_log_level_texts_are_the_check_constraint_s() {
        let texts: Vec<&str> = WorkerLogLevel::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(texts, ["error", "warn", "info", "debug", "trace"]);
        for level in WorkerLogLevel::ALL {
            assert_eq!(level.as_str().parse::<WorkerLogLevel>(), Ok(*level));
            assert_eq!(
                serde_json::to_value(level).expect("a level serialises"),
                json!(level.as_str())
            );
        }
        assert!("ERROR".parse::<WorkerLogLevel>().is_err());
    }

    /// Plan D7: `off` ships nothing, each other setting ships its level and everything more severe.
    #[test]
    fn worker_log_ship_level_ships_its_level_and_above() {
        use WorkerLogLevel as L;
        for level in WorkerLogLevel::ALL {
            assert!(!LogShipLevel::Off.ships(*level), "off ships no {level}");
        }
        assert!(LogShipLevel::Error.ships(L::Error));
        assert!(!LogShipLevel::Error.ships(L::Warn));
        assert!(LogShipLevel::Warn.ships(L::Warn));
        assert!(!LogShipLevel::Warn.ships(L::Info));
        assert!(LogShipLevel::Info.ships(L::Info));
        assert!(!LogShipLevel::Info.ships(L::Debug));
        assert!(LogShipLevel::Debug.ships(L::Debug));
        assert!(!LogShipLevel::Debug.ships(L::Trace));
        let texts: Vec<&str> = LogShipLevel::ALL.iter().map(|l| l.as_str()).collect();
        assert_eq!(texts, ["off", "error", "warn", "info", "debug"]);
    }

    /// A text within the cap is kept as written; one over it is cut to the cap with a `…`.
    #[test]
    fn worker_log_text_is_capped_at_8_kib_with_an_ellipsis() {
        let exact = "a".repeat(WORKER_LOG_TEXT_MAX);
        assert!(matches!(line(&exact).stored_text(), Cow::Borrowed(_)));
        assert_eq!(line(&exact).stored_text(), exact);

        let long = "a".repeat(WORKER_LOG_TEXT_MAX + 1);
        let stored = line(&long).stored_text().into_owned();
        assert_eq!(stored.len(), WORKER_LOG_TEXT_MAX);
        assert!(stored.ends_with('…'));
        assert!(stored.starts_with("aaa"));
    }

    /// The cut never splits a char: a run of 3-byte chars is cut at a boundary below the cap.
    #[test]
    fn worker_log_text_is_cut_at_a_char_boundary() {
        // 'é' is two bytes; an odd prefix byte puts every boundary at an odd offset.
        let long = format!("a{}", "é".repeat(WORKER_LOG_TEXT_MAX));
        let stored = line(&long).stored_text().into_owned();
        assert!(stored.len() <= WORKER_LOG_TEXT_MAX, "{}", stored.len());
        assert!(stored.len() >= WORKER_LOG_TEXT_MAX - 1, "{}", stored.len());
        assert!(stored.ends_with('…'));
        let body = stored.trim_end_matches('…');
        assert!(body.chars().skip(1).all(|c| c == 'é'));
    }

    /// Postgres text refuses NUL, so both stores write U+FFFD instead.
    #[test]
    fn worker_log_nul_becomes_a_replacement_char() {
        let mut nul = line("a\0b");
        nul.target = "t\0".to_owned();
        assert_eq!(nul.stored_text(), "a\u{FFFD}b");
        assert_eq!(nul.stored_target(), "t\u{FFFD}");
        assert!(matches!(line("ab").stored_target(), Cow::Borrowed(_)));
    }
}
