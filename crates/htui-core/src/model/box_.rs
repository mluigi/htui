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

/// One tool the box probe found (MOD-7 plan D9): a future `box_tool` row without its box or instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbedTool {
    /// `box_tool.name`: the key of the probe spec's `tools` map, never a file name.
    pub name: String,
    /// `box_tool.version`; empty for a presence-only tool.
    pub version: String,
    /// `box_tool.path`: the file `which` resolved.
    pub path: String,
}

/// Everything one box probe learned, as `WriteStore::record_box_probe` writes it (MOD-7 D10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxProbe {
    /// The probed box.
    pub box_id: BoxId,
    /// `box.os_version`.
    pub os_version: String,
    /// `box.cpu`.
    pub cpu: String,
    /// `box.ram_mb`.
    pub ram_mb: Option<i32>,
    /// `box.gpu_present`.
    pub gpu_present: bool,
    /// `box.gpu_vendor`: the spec's vendor name, `None` without a GPU.
    pub gpu_vendor: Option<String>,
    /// The whole `box_tool` set; replaces the previous one. Names are unique.
    pub tools: Vec<ProbedTool>,
    /// `box.probed_tags`, sorted and deduplicated.
    pub probed_tags: Vec<String>,
    /// `box.htui_version`: the version that probed (D5).
    pub htui_version: String,
    /// `box.probe_spec_digest`: sha256 hex of the effective spec (D18).
    pub spec_digest: String,
    /// `box.last_probed_at` and every `box_tool.probed_at`.
    pub probed_at: DateTime<Utc>,
}

/// One box as `WriteStore::boxes` lists it (MOD-7 D10, D18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxRecord {
    /// The row.
    pub row: BoxRow,
    /// Its `box_tool` rows, name-byte-ordered.
    pub tools: Vec<BoxTool>,
    /// `box.probe_spec_digest`, which is not a `BoxRow` field (D14 keeps every constructor still).
    pub probe_spec_digest: Option<String>,
}

impl BoxRecord {
    /// Whether the next `Online` swap must probe this box (D5, D18): never probed, probed by another
    /// `htui`, or under another effective spec.
    #[must_use]
    pub fn needs_probe(&self, running: &str, spec_digest: &str) -> bool {
        self.row.last_probed_at.is_none()
            || self.row.htui_version != running
            || self.probe_spec_digest.as_deref() != Some(spec_digest)
    }
}

/// The longest declared tag, in chars (MOD-7 D42).
pub const DECLARED_TAG_MAX: usize = 64;

/// One human edit of a box row (MOD-7 milestone 2, D41): `Some` writes that column, `None` leaves
/// it alone. An edit with both `None` is legal and still bumps `edit_version`.
///
/// `Debug` is derived on purpose: quirks are not secret (they go into every prompt's box
/// section), and `htui`'s `StoreRequest` carries this type and derives `Debug`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoxEdit {
    /// `box.declared_tags`, whole; the store validates, sorts and deduplicates it.
    pub declared_tags: Option<Vec<String>>,
    /// `box.quirks`, whole, lines separated by `\n`; stored as given (D43).
    pub quirks: Option<String>,
}

/// Whether `tag` is a declared tag (MOD-7 D42): 1 to [`DECLARED_TAG_MAX`] chars of
/// `[a-z0-9_-]`, the first one a letter or a digit. Every tag the seed derives already is.
#[must_use]
pub fn is_declared_tag(tag: &str) -> bool {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
    let mut chars = tag.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(allowed)
        && tag.chars().count() <= DECLARED_TAG_MAX
}

/// The stored form of a declared-tag list (MOD-7 D42, D55): each element validated **as given**
/// (no trim, no split, no case change), then sorted by bytes and deduplicated. Both stores call
/// this before they write, so a refusal carries one sentence on `MemStore` and on Postgres.
///
/// # Errors
///
/// The refusal sentence of the first element [`is_declared_tag`] rejects, in input order.
pub fn canonical_declared_tags(tags: &[String]) -> Result<Vec<String>, String> {
    if let Some(bad) = tags.iter().find(|tag| !is_declared_tag(tag)) {
        return Err(declared_tag_refusal(bad));
    }
    let mut canonical = tags.to_vec();
    // `str`'s `Ord` is byte order, which is what "sorted by bytes" means.
    canonical.sort_unstable();
    canonical.dedup();
    Ok(canonical)
}

/// A declared-tag list as the user types it (MOD-7 D42, D50): split on `,`, each piece trimmed,
/// empty pieces dropped, then [`canonical_declared_tags`]. Empty text is no tags.
///
/// # Errors
///
/// [`canonical_declared_tags`]'s sentence.
pub fn declared_tags_from_text(text: &str) -> Result<Vec<String>, String> {
    let pieces: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(str::to_owned)
        .collect();
    canonical_declared_tags(&pieces)
}

/// The one refusal sentence (private).
fn declared_tag_refusal(tag: &str) -> String {
    format!(
        "declared tag `{tag}` is not 1-{DECLARED_TAG_MAX} characters of a-z, 0-9, `_` and `-` \
         starting with a letter or a digit"
    )
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

    const DIGEST_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// A box probed at `row()`'s version (`0.4.1`) under [`DIGEST_A`].
    fn record() -> BoxRecord {
        BoxRecord {
            row: row(),
            tools: Vec::new(),
            probe_spec_digest: Some(DIGEST_A.to_owned()),
        }
    }

    /// MOD-7 D5: a box that was never probed probes, whatever its version and digest say.
    #[test]
    fn a_box_never_probed_needs_a_probe() {
        let never = BoxRecord {
            row: BoxRow {
                last_probed_at: None,
                ..row()
            },
            ..record()
        };
        assert!(
            never.needs_probe("0.4.1", DIGEST_A),
            "same version and digest, but never probed"
        );
    }

    /// MOD-7 D5: an `htui` upgrade re-probes (`R-BOX-2`).
    #[test]
    fn a_box_probed_by_another_version_needs_a_probe() {
        assert!(record().needs_probe("0.4.2", DIGEST_A));
    }

    /// MOD-7 D5, D18: same version, same effective spec, nothing to do.
    #[test]
    fn a_box_probed_at_this_version_needs_none() {
        assert!(!record().needs_probe("0.4.1", DIGEST_A));
    }

    /// MOD-7 D18: a changed effective spec re-probes.
    #[test]
    fn a_changed_spec_digest_needs_a_probe() {
        assert!(record().needs_probe("0.4.1", DIGEST_B));
    }

    /// MOD-7 D18: a row probed before `probe_spec_digest` existed re-probes.
    #[test]
    fn a_box_with_no_recorded_digest_needs_a_probe() {
        let undigested = BoxRecord {
            probe_spec_digest: None,
            ..record()
        };
        assert!(undigested.needs_probe("0.4.1", DIGEST_A));
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

    fn owned(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|&tag| tag.to_owned()).collect()
    }

    /// MOD-7 D42: `[a-z0-9_-]`, 1 to 64 chars, the first a letter or a digit.
    #[test]
    fn a_declared_tag_is_lowercase_digits_underscore_and_dash() {
        for tag in ["gpu", "heavy_build", "x86-64", "9p", "a"] {
            assert!(is_declared_tag(tag), "`{tag}` is a declared tag");
        }
        for tag in ["", "_x", "-x", "GPU", "gpu tag", "gpü", "a,b"] {
            assert!(!is_declared_tag(tag), "`{tag}` is not a declared tag");
        }
    }

    /// MOD-7 D42, D50: the text the user types is split on `,`, trimmed, sorted and deduplicated.
    #[test]
    fn a_declared_tag_list_is_split_on_commas_trimmed_sorted_and_deduplicated() {
        assert_eq!(
            declared_tags_from_text(" vulkan, gpu ,,gpu, heavy_build "),
            Ok(owned(&["gpu", "heavy_build", "vulkan"]))
        );
    }

    /// MOD-7 D42: empty text, blanks and bare commas are no tags, not a refusal.
    #[test]
    fn an_empty_tag_list_is_no_tags() {
        for text in ["", "  ", " , , "] {
            assert_eq!(declared_tags_from_text(text), Ok(Vec::new()), "{text:?}");
        }
    }

    /// MOD-7 D42: nothing is lower-cased on the user's behalf; the refusal names the tag.
    #[test]
    fn an_uppercase_or_spaced_tag_is_refused_by_name() {
        let upper = declared_tags_from_text("gpu, Vulkan").expect_err("`Vulkan` is refused");
        assert!(upper.contains("`Vulkan`"), "{upper}");
        assert_eq!(upper, declared_tag_refusal("Vulkan"));

        let spaced =
            declared_tags_from_text("gpu, heavy build").expect_err("`heavy build` is refused");
        assert!(spaced.contains("`heavy build`"), "{spaced}");
    }

    /// MOD-7 D42: the ten seeded tags (`R-BOX-3`, `pg/mod.rs`) and the five milestone 1 derives
    /// (OQ-12) all pass the rule, so no stored list starts out invalid.
    #[test]
    fn every_seeded_and_derived_tag_is_a_valid_declared_tag() {
        let seeded = [
            "gpu",
            "vulkan",
            "msvc",
            "mingw",
            "clang",
            "cmake",
            "vcpkg",
            "docker",
            "rust",
            "heavy_build",
        ];
        let derived = ["go", "node", "python", "java", "dotnet"];
        for tag in seeded.into_iter().chain(derived) {
            assert!(is_declared_tag(tag), "`{tag}` is a valid declared tag");
        }
    }

    /// MOD-7 D42: the cap is [`DECLARED_TAG_MAX`] chars.
    #[test]
    fn a_sixty_five_char_tag_is_refused() {
        assert!(is_declared_tag(&"a".repeat(DECLARED_TAG_MAX)));
        assert!(!is_declared_tag(&"a".repeat(DECLARED_TAG_MAX + 1)));
    }

    /// MOD-7 D55: the store-side rule takes each element as given, so a stray space or comma is
    /// a refusal rather than a silent rewrite.
    #[test]
    fn the_store_list_is_validated_strictly_not_trimmed_or_split() {
        for tags in [owned(&[" gpu"]), owned(&["a,b"]), owned(&[""])] {
            assert!(
                canonical_declared_tags(&tags).is_err(),
                "{tags:?} is refused"
            );
        }
        assert_eq!(
            canonical_declared_tags(&owned(&["vulkan", "gpu", "gpu"])),
            Ok(owned(&["gpu", "vulkan"]))
        );
    }
}
