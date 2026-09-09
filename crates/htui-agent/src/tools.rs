//! Resolving `agent.launch`'s `${tool}` placeholders on this box (plan MOD-2 D25, D46).
//!
//! This is the **chat-start caller of the probe's resolver**, not a second copy of it. A session
//! cannot start before `${node}`, `${claude_agent_acp}` and `${claude}` are strings, and it must
//! not pay for a full probe to get them: this module answers that one question and **stores
//! nothing**. [`crate::probe::resolve_tool`] owns the tier walk itself — one resolver, two
//! callers, so the `PATH`/`node_modules`/glob rules cannot drift apart (plan D46).
//!
//! Two tiers per tool, in order:
//!
//! 1. `HTUI_TOOL_<NAME>` in the environment, taken **on trust**: it is the escape hatch for a box
//!    whose layout no tier understands, and refusing a path the user named would close it. (The
//!    probe checks the same override *does* exist, because a snapshot that claimed a nonexistent
//!    path was `ready` would be a lie the Settings tab repeats.)
//! 2. [`crate::probe::resolve_tool`]: [`ToolProbe::Path`] through `which`, which is
//!    `PATH`/`PATHEXT`-correct where a bare `Command::new` is not (§4.4's Windows finding);
//!    [`ToolProbe::NodePackage`] as a path to the package's entry point, never the shim on `PATH`;
//!    [`ToolProbe::Glob`] through the probe's walker.
//!
//! Version capture is deliberately off here ([`crate::probe::ProbeEnv::without_versions`]): a chat
//! start would otherwise spawn a `--version` child per tool before its first token.
//!
//! **Known limit** (blueprint H-3): a [`ToolMap`] value is one string, so a glob tool's
//! per-platform `args` — `agy`'s Linux-only `--uid=` — are *not* applied on this path. Since
//! milestone 6 (plan D58) this path is the **fallback** of `AcpDriver::launch_for`: a usable
//! `agent_box.probe.resolved` is spawned as recorded, arguments and all, and only a row with no
//! usable snapshot — or one whose recorded command is gone from disk — resolves here, without the
//! append. A row whose tools carry no per-platform arguments, which is every seed row but `agy`,
//! resolves to the same launch either way.

use std::path::Path;

use crate::error::{DriverError, Result};
use crate::launch::{Discovery, ToolMap, ToolProbe};

/// The environment variable that overrides one tool's resolution.
///
/// `${claude_agent_acp}` is `HTUI_TOOL_CLAUDE_AGENT_ACP`: the placeholder name uppercased, with
/// every character that is not ASCII alphanumeric folded to `_` so a row cannot name a variable
/// the shell refuses to set.
#[must_use]
pub fn env_override_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len() + 10);
    key.push_str("HTUI_TOOL_");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_uppercase());
        } else {
            key.push('_');
        }
    }
    key
}

/// Resolves every tool `discovery` declares, in name order.
///
/// Every entry is resolved, not only the ones the command line happens to reference: a row's
/// `env` block names tools too (`CLAUDE_CODE_EXECUTABLE = "${claude}"`), and resolving the whole
/// map once is cheaper than discovering a missing one after the child has started. A row with no
/// `discovery` yields an empty map, which is what a placeholder-free launch resolves against
/// (`crate::launch::resolve`).
///
/// # Errors
///
/// [`DriverError::Unresolved`] naming the first tool that resolves nowhere.
/// [`DriverError::Transport`] when the resolution machinery itself fails: a `which` lookup or a
/// glob walk that could not be scheduled.
pub async fn resolve(discovery: Option<&Discovery>, cwd: &Path) -> Result<ToolMap> {
    resolve_with(discovery, cwd, &env_override).await
}

/// The value of `HTUI_TOOL_<NAME>`, if the environment sets it.
fn env_override(name: &str) -> Option<String> {
    std::env::var_os(env_override_key(name)).map(|value| value.to_string_lossy().into_owned())
}

/// [`resolve`] over an injected override lookup.
///
/// The seam exists for one reason: `std::env::set_var` is `unsafe` on this edition and the
/// workspace is `unsafe_code = "forbid"`, so the override tier cannot be exercised by setting a
/// variable. It is private, and production has exactly one caller passing [`env_override`].
async fn resolve_with(
    discovery: Option<&Discovery>,
    cwd: &Path,
    // `Sync` as well as `Fn`: this future is awaited inside the session task, which `tokio::spawn`
    // requires to be `Send`, and a `&dyn Fn` alone is not.
    overrides: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Result<ToolMap> {
    let mut map = ToolMap::new();
    let Some(discovery) = discovery else {
        return Ok(map);
    };
    // Built once, outside the loop: it snapshots the whole process environment, and every tool of
    // one row resolves against the same box.
    let env = crate::probe::ProbeEnv::host(cwd.to_path_buf()).without_versions();
    for (name, probe) in &discovery.tools {
        if let Some(value) = overrides(name) {
            map.insert(name.clone(), value);
            continue;
        }
        match crate::probe::resolve_tool(probe, &env).await? {
            Some(found) => {
                map.insert(name.clone(), found.path.to_string_lossy().into_owned());
            }
            None => return Err(unresolved(name, probe)),
        }
    }
    Ok(map)
}

/// The error a tool that resolved nowhere produces.
///
/// The variant is [`DriverError::Unresolved`] and carries the placeholder name alone, because that
/// is what a caller can act on and what `crate::launch::resolve` returns for the same condition —
/// one failure mode, one variant. Which tier was tried, and the override that would fix it, go to
/// the log: they are diagnosis, not control flow.
fn unresolved(name: &str, probe: &ToolProbe) -> DriverError {
    let detail = match probe {
        ToolProbe::Path { names, .. } => format!("none of {names:?} is on PATH"),
        ToolProbe::NodePackage { package, entry, .. } => {
            format!("`{package}/{entry}` is in neither the local nor the global node_modules")
        }
        ToolProbe::Glob { .. } => "no file matches any glob pattern for this platform".to_owned(),
    };
    tracing::warn!(
        tool = name,
        override_key = env_override_key(name),
        %detail,
        "tool did not resolve"
    );
    DriverError::Unresolved(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A `Discovery` holding one probe under `name`.
    fn discovery(name: &str, probe: ToolProbe) -> Discovery {
        let mut tools = BTreeMap::new();
        tools.insert(name.to_owned(), probe);
        Discovery {
            tools,
            handshake: false,
            // Resolution never reads either: the credential tier is the probe's (plan D59), the
            // install block is the installer's (plan MOD-20 D12), and `tools::resolve` answers the
            // same for a row that declares them and a row that does not.
            credential: None,
            install: None,
        }
    }

    fn path_probe(names: &[&str]) -> ToolProbe {
        ToolProbe::Path {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            version: None,
        }
    }

    #[test]
    fn the_override_key_is_the_name_uppercased_with_separators_folded() {
        assert_eq!(env_override_key("node"), "HTUI_TOOL_NODE");
        assert_eq!(
            env_override_key("claude_agent_acp"),
            "HTUI_TOOL_CLAUDE_AGENT_ACP"
        );
        assert_eq!(
            env_override_key("agy-acp.server"),
            "HTUI_TOOL_AGY_ACP_SERVER"
        );
    }

    #[tokio::test]
    async fn a_row_without_discovery_resolves_to_an_empty_map() {
        let map = resolve(None, Path::new(".")).await.expect("no probes");
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn a_path_probe_resolves_through_which() {
        // `cargo` is on PATH for every test run, and `tests/launch.rs` already leans on it.
        let map = resolve(
            Some(&discovery("cargo", path_probe(&["cargo"]))),
            Path::new("."),
        )
        .await
        .expect("cargo is on PATH");
        let resolved = map.get("cargo").expect("one entry");
        assert!(
            resolved.contains("cargo"),
            "the resolved path names the tool: {resolved}"
        );
        assert!(
            Path::new(resolved).is_absolute(),
            "`which` answers an absolute path: {resolved}"
        );
    }

    /// The same variant `crate::launch::resolve` returns for the same condition, carrying the
    /// placeholder name and nothing else.
    #[tokio::test]
    async fn a_tool_on_no_path_is_unresolved_under_its_own_name() {
        let probe = discovery("nope", path_probe(&["htui-no-such-binary-2f8e"]));
        let err = resolve(Some(&probe), Path::new("."))
            .await
            .expect_err("nothing resolves");
        assert_eq!(err, DriverError::Unresolved("nope".to_owned()));
    }

    #[tokio::test]
    async fn a_glob_probe_that_matches_nothing_is_unresolved() {
        let probe = discovery(
            "agy_acp_server",
            ToolProbe::Glob {
                patterns: vec!["**/agy_acp_server*".to_owned()],
                platform: BTreeMap::new(),
            },
        );
        let err = resolve(Some(&probe), Path::new("."))
            .await
            .expect_err("no such file exists under this pattern's root");
        assert_eq!(err, DriverError::Unresolved("agy_acp_server".to_owned()));
    }

    #[tokio::test]
    async fn a_node_package_prefers_the_local_tree_over_the_global_root() {
        let dir = tempfile::tempdir().expect("temp cwd");
        let entry = dir
            .path()
            .join("node_modules")
            .join("@scope/pkg")
            .join("dist/index.js");
        std::fs::create_dir_all(entry.parent().expect("parent")).expect("mkdir");
        std::fs::write(&entry, "// entry").expect("write");

        let probe = discovery(
            "pkg",
            ToolProbe::NodePackage {
                package: "@scope/pkg".to_owned(),
                entry: "dist/index.js".to_owned(),
                pinned: "0.0.0".to_owned(),
                fallback: None,
            },
        );
        let map = resolve(Some(&probe), dir.path())
            .await
            .expect("the local tree holds it");
        assert_eq!(
            map.get("pkg").map(String::as_str),
            Some(entry.to_string_lossy().as_ref()),
            "the local node_modules wins"
        );
    }

    /// The override is the first tier: it answers even for a tier this milestone cannot resolve
    /// at all, which is what makes it the escape hatch for an `agy` box before milestone 5.
    #[tokio::test]
    async fn the_override_wins_over_every_tier() {
        let probe = discovery(
            "glob_only",
            ToolProbe::Glob {
                patterns: Vec::new(),
                platform: BTreeMap::new(),
            },
        );
        let map = resolve_with(Some(&probe), Path::new("."), &|name| {
            (name == "glob_only").then(|| "/opt/agy/agy_acp_server".to_owned())
        })
        .await
        .expect("the override answers");
        assert_eq!(
            map.get("glob_only"),
            Some(&"/opt/agy/agy_acp_server".to_owned())
        );
    }
}
