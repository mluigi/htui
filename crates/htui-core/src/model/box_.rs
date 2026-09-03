//! Boxes and their probed tooling (`docs/ANA-9.md` §5.2).
//!
//! `Box` is in the Rust prelude, so the row type is [`BoxRow`] (blueprint B.4 naming).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::ids::{BoxId, UserId};

str_enum!(
    /// `box.os_family` (§5.2).
    OsFamily {
        /// Windows.
        Windows => "windows",
        /// Linux.
        Linux => "linux",
        /// macOS.
        Macos => "macos",
    }
);

/// A row of `box` (§5.2): one development machine, identified by a UUID kept in `box.toml` so the
/// identity survives a hostname change (`R-BOX-4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxRow {
    /// `box.id`.
    pub id: BoxId,
    /// `box.user_id`.
    pub user_id: UserId,
    /// `box.hostname`.
    pub hostname: String,
    /// `box.os_family`.
    pub os_family: OsFamily,
    /// `box.os_version`.
    pub os_version: String,
    /// `box.arch`.
    pub arch: String,
    /// `box.cpu`.
    pub cpu: String,
    /// `box.ram_mb`.
    pub ram_mb: Option<i32>,
    /// `box.gpu_present`.
    pub gpu_present: bool,
    /// `box.gpu_vendor`.
    pub gpu_vendor: Option<String>,
    /// `box.htui_version`: the re-probe trigger (`R-BOX-2`).
    pub htui_version: String,
    /// `box.probed_tags`.
    pub probed_tags: Vec<String>,
    /// `box.declared_tags`.
    pub declared_tags: Vec<String>,
    /// `box.quirks`.
    pub quirks: String,
    /// `box.settings` (`JSONB`): command limits, `max_concurrent_items`.
    pub settings: Value,
    /// `box.registered_at`.
    pub registered_at: DateTime<Utc>,
    /// `box.last_seen_at`.
    pub last_seen_at: DateTime<Utc>,
    /// `box.last_probed_at`.
    pub last_probed_at: Option<DateTime<Utc>>,
    /// `box.updated_at`.
    pub updated_at: DateTime<Utc>,
}

/// A row of `box_tool` (§5.2): one compiler, build tool, shell or container runtime found by the
/// probe (`R-BOX-2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxTool {
    /// `box_tool.box_id`.
    pub box_id: BoxId,
    /// `box_tool.name`, e.g. `rustc` or `cmake`.
    pub name: String,
    /// `box_tool.version`.
    pub version: String,
    /// `box_tool.path`.
    pub path: String,
    /// `box_tool.probed_at`.
    pub probed_at: DateTime<Utc>,
}

/// Top-bar projection of the current box (`R-TUI-1`). Not a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxInfo {
    /// `box.id` of this box.
    pub box_id: BoxId,
    /// `box.hostname` of this box.
    pub hostname: String,
    /// `box.os_family` of this box.
    pub os_family: OsFamily,
}
