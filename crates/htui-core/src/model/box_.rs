//! Boxes and their probed tooling (`docs/ANA-9.md` §5.2).
//!
//! `Box` is in the Rust prelude, so the row type is [`BoxRow`] (blueprint B.4 naming).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

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
    /// `box.probed_tags` (`R-ORCH-10`).
    pub probed_tags: Vec<String>,
    /// `box.declared_tags`.
    pub declared_tags: Vec<String>,
    /// `box.settings`, whole; decode with [`BoxSettings`].
    pub settings: Value,
}

/// `box.settings` as ANA-2 §4.7 reads it.
///
/// Read-only this milestone (plan D11): MOD-15's key-level `set_setting` is the only writer, so
/// unknown keys survive because nothing here is ever re-serialised onto the row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxSettings {
    /// `R-ORCH-9`; `None` = fall through to the `app_setting` rung, else
    /// [`DEFAULT_MAX_CONCURRENT_ITEMS`].
    ///
    /// `Option<u32>` rather than `u32` because `#[serde(default)]` on a `u32` is `0`, which
    /// admits nothing at all — a box whose settings blob does not name the key would stop
    /// accepting runs (blueprint F-T).
    pub max_concurrent_items: Option<u32>,
    /// `R-MCP-3`'s `{class: n}`; empty = the `app_setting` rung.
    pub command_limits: BTreeMap<String, u32>,
}

/// The value `0003_orchestration.sql` seeds under `app_setting.max_concurrent_items`, and what a
/// store answers when neither the box nor that table names one (`R-ORCH-9`).
pub const DEFAULT_MAX_CONCURRENT_ITEMS: u32 = 2;

/// The prompt's `box` section, projected from `box` and its `box_tool` rows
/// (`docs/ANA-5.md` §4.2). Not a table.
///
/// The projection is closed and deliberately narrower than the row: `box.settings` is orchestrator
/// policy with no place in a prompt, and `probed_tags` / `declared_tags` are `R-ORCH-10` matching
/// vocabulary rather than a machine description. `box_tool.path` is dropped outright — §4.2 rule 5
/// forbids an absolute filesystem path anywhere in a prompt, because a fan-out sibling's tree
/// differs only by its worktree path and the prompt digest must not.
///
/// The same list is what the `box_profile` read tool of `R-MCP-2` hands an agent, so the two
/// cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxProfile {
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
    /// `box.ram_mb`; `None` omits the `ram` line.
    pub ram_mb: Option<i32>,
    /// `box.gpu_vendor`, but only on a box that has a GPU; `None` omits the `gpu` line.
    pub gpu_vendor: Option<String>,
    /// `box.htui_version`.
    pub htui_version: String,
    /// `(box_tool.name, box_tool.version)`, name-byte-sorted and capped at
    /// [`BoxProfile::MAX_TOOLS`]. An empty version renders the name bare.
    pub tools: Vec<(String, String)>,
    /// How many probed tools the cap dropped: the `N` of the render's trailing `, +N more`, and
    /// `0` when nothing was dropped.
    pub more_tools: usize,
    /// `box.quirks`, verbatim; empty omits the `quirks` line, and the render collapses newlines
    /// to `; `.
    pub quirks: String,
}

impl BoxProfile {
    /// The cap on rendered tools (§4.2). A probe finds far more than a prompt can spend tokens on,
    /// and the overflow is reported as a count rather than silently lost.
    pub const MAX_TOOLS: usize = 24;

    /// Projects a `box` row and its `box_tool` rows into the prompt's `box` section.
    ///
    /// Tools are sorted by `name` byte order — not collation order, for the reason
    /// [`crate::model::link::UpstreamEntry::sort_canonical`] gives — then capped, keeping
    /// `(name, version)` and dropping `path`. `gpu_vendor` survives only on a box that reports a
    /// GPU: a vendor string on a box with `gpu_present = false` is stale probe data, not a GPU.
    #[must_use]
    pub fn project(row: &BoxRow, mut tools: Vec<BoxTool>) -> Self {
        tools.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let more_tools = tools.len().saturating_sub(Self::MAX_TOOLS);
        tools.truncate(Self::MAX_TOOLS);
        Self {
            hostname: row.hostname.clone(),
            os_family: row.os_family,
            os_version: row.os_version.clone(),
            arch: row.arch.clone(),
            cpu: row.cpu.clone(),
            ram_mb: row.ram_mb,
            gpu_vendor: if row.gpu_present {
                row.gpu_vendor.clone()
            } else {
                None
            },
            htui_version: row.htui_version.clone(),
            tools: tools
                .into_iter()
                .map(|tool| (tool.name, tool.version))
                .collect(),
            more_tools,
            quirks: row.quirks.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::BoxId;
    use serde_json::json;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    fn row() -> BoxRow {
        BoxRow {
            id: BoxId::new(),
            user_id: UserId::new(),
            hostname: "dev-win-01".to_owned(),
            os_family: OsFamily::Windows,
            os_version: "10.0.26200".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: "AMD Ryzen 9 7950X, 32 threads".to_owned(),
            ram_mb: Some(65_536),
            gpu_present: true,
            gpu_vendor: Some("nvidia".to_owned()),
            htui_version: "0.4.1".to_owned(),
            probed_tags: vec!["windows".to_owned()],
            declared_tags: vec!["gaming".to_owned()],
            quirks: "MSVC toolchain only; no WSL.".to_owned(),
            settings: json!({"max_concurrent_items": 2}),
            registered_at: at(),
            last_seen_at: at(),
            last_probed_at: Some(at()),
            updated_at: at(),
        }
    }

    fn tool(box_id: BoxId, name: &str, version: &str, path: &str) -> BoxTool {
        BoxTool {
            box_id,
            name: name.to_owned(),
            version: version.to_owned(),
            path: path.to_owned(),
            probed_at: at(),
        }
    }

    /// All three are nullable in `box` and all three are omitted rather than rendered empty
    /// (§4.2): `ram` when `ram_mb` is NULL, `gpu` when `gpu_present` is false **or** the vendor is
    /// NULL, `quirks` when empty.
    #[test]
    fn profile_omits_ram_gpu_and_quirks_when_absent() {
        let full = BoxProfile::project(&row(), Vec::new());
        assert_eq!(full.ram_mb, Some(65_536));
        assert_eq!(full.gpu_vendor.as_deref(), Some("nvidia"));
        assert_eq!(full.quirks, "MSVC toolchain only; no WSL.");

        let bare = BoxRow {
            ram_mb: None,
            gpu_present: false,
            quirks: String::new(),
            ..row()
        };
        let profile = BoxProfile::project(&bare, Vec::new());
        assert_eq!(profile.ram_mb, None, "ram is omitted when ram_mb is NULL");
        assert_eq!(
            profile.gpu_vendor, None,
            "a vendor on a box with no GPU is not a GPU"
        );
        assert!(profile.quirks.is_empty(), "an empty quirks renders nothing");

        let vendorless = BoxRow {
            gpu_present: true,
            gpu_vendor: None,
            ..row()
        };
        assert_eq!(
            BoxProfile::project(&vendorless, Vec::new()).gpu_vendor,
            None,
            "a present GPU with no vendor names nothing"
        );
    }

    /// `box_tool` rows are sorted by `name` byte order and capped at
    /// [`BoxProfile::MAX_TOOLS`], the overflow counted for the render's `, +N more`.
    #[test]
    fn tools_are_name_sorted_capped_at_24_with_more() {
        let id = BoxId::new();
        let mut tools: Vec<BoxTool> = (0..30)
            .rev()
            .map(|i| {
                tool(
                    id,
                    &format!("tool-{i:02}"),
                    "1.0",
                    &format!("/usr/bin/tool-{i:02}"),
                )
            })
            .collect();
        tools.push(tool(id, "Zig", "0.15", "/usr/bin/zig"));

        let profile = BoxProfile::project(&row(), tools);

        assert_eq!(profile.tools.len(), BoxProfile::MAX_TOOLS);
        assert_eq!(profile.more_tools, 7, "31 probed, 24 kept");
        assert_eq!(
            profile.tools[0],
            ("Zig".to_owned(), "0.15".to_owned()),
            "byte order puts `Z` before `t`"
        );
        assert_eq!(profile.tools[1].0, "tool-00");
        assert_eq!(profile.tools[23].0, "tool-22");
        assert!(
            profile.tools.windows(2).all(|w| w[0].0 <= w[1].0),
            "sorted by name"
        );

        let empty = BoxProfile::project(&row(), Vec::new());
        assert!(empty.tools.is_empty());
        assert_eq!(
            empty.more_tools, 0,
            "nothing over the cap, nothing to count"
        );
    }

    /// `box_tool.version` is `TEXT NOT NULL`, so there is no null case; an empty string is kept as
    /// an empty string and the render emits the name bare.
    #[test]
    fn a_bare_version_renders_the_name_alone() {
        let id = BoxId::new();
        let profile = BoxProfile::project(&row(), vec![tool(id, "shellcheck", "", "/usr/bin/sc")]);
        assert_eq!(
            profile.tools,
            vec![("shellcheck".to_owned(), String::new())]
        );
    }

    /// §4.2 rule 5: an absolute filesystem path never appears inside any section, and the agent
    /// invokes a tool by name. The projection is where `box_tool.path` stops — asserted over the
    /// whole serialised profile, so a later field cannot smuggle one back in.
    #[test]
    fn path_never_reaches_the_profile() {
        let id = BoxId::new();
        let profile = BoxProfile::project(
            &row(),
            vec![
                tool(
                    id,
                    "cargo",
                    "1.98.1",
                    "C:\\Users\\dev\\.cargo\\bin\\cargo.exe",
                ),
                tool(id, "git", "2.47.0", "/usr/bin/git"),
            ],
        );

        let json = serde_json::to_string(&profile).expect("a profile serializes");
        assert!(!json.contains(".cargo\\\\bin"), "no Windows path: {json}");
        assert!(!json.contains("/usr/bin"), "no POSIX path: {json}");
        assert_eq!(
            profile.tools,
            vec![
                ("cargo".to_owned(), "1.98.1".to_owned()),
                ("git".to_owned(), "2.47.0".to_owned()),
            ],
            "name and version, and nothing else"
        );
    }
}
