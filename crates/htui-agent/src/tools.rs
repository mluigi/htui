//! Resolving `agent.launch`'s `${tool}` placeholders on this box (plan MOD-2 D25).
//!
//! This is **not** the probe. `docs/ANA-4.md` §4.6's tiered probe writes `agent_box.probe` and
//! belongs to milestone 5; a chat session, however, cannot start before `${node}`,
//! `${claude_agent_acp}` and `${claude}` are strings, so milestone 3 ships the smallest resolver
//! that turns the seeded rows into a working launch and **stores nothing**.
//!
//! Three tiers per tool, in order:
//!
//! 1. `HTUI_TOOL_<NAME>` in the environment, which is also the escape hatch for a box whose layout
//!    neither of the other two tiers understands;
//! 2. [`ToolProbe::Path`] through `which`, which is `PATH`/`PATHEXT`-correct where a bare
//!    `Command::new` is not (§4.4's Windows finding);
//! 3. [`ToolProbe::NodePackage`] as a path to the package's entry point — the local
//!    `node_modules` first, then the global root `npm root -g` reports — never the shim on `PATH`,
//!    which §4.4 measured to be unspawnable on Windows.
//!
//! [`ToolProbe::Glob`] resolves nowhere here: it is `agy`'s tier (milestone 6) and its patterns are
//! per platform, which is exactly the part of §4.6 milestone 5 owns. The error says so rather than
//! guessing a path.

use std::path::{Path, PathBuf};

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
/// [`DriverError::Unresolved`] naming the first tool that resolves nowhere — including every
/// [`ToolProbe::Glob`], whose message names milestone 5. [`DriverError::Transport`] when the
/// resolution machinery itself fails: a `which` lookup that could not be scheduled, or an `npm`
/// that could not be run.
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
    for (name, probe) in &discovery.tools {
        if let Some(value) = overrides(name) {
            map.insert(name.clone(), value);
            continue;
        }
        let resolved = match probe {
            ToolProbe::Path { names, .. } => on_path(names).await?,
            ToolProbe::NodePackage { package, entry, .. } => {
                node_package(package, entry, cwd).await?
            }
            // Milestone 6's `agy` row is the only one that uses this tier, and its patterns are
            // per `<os>-<arch>`: guessing one here would be the probe, badly.
            ToolProbe::Glob { .. } => None,
        };
        match resolved {
            Some(path) => {
                map.insert(name.clone(), path);
            }
            None => return Err(unresolved(name, probe)),
        }
    }
    Ok(map)
}

/// The first of `names` that `which` finds, or `None`.
///
/// `which` is synchronous and touches the filesystem, so it runs on `spawn_blocking` rather than
/// on the runtime's worker (the rule `crate::launch::spawn` already follows).
async fn on_path(names: &[String]) -> Result<Option<String>> {
    let names = names.to_vec();
    tokio::task::spawn_blocking(move || {
        names
            .iter()
            .find_map(|name| which::which(name).ok())
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|err| DriverError::Transport(format!("tool lookup did not run: {err}")))
}

/// `<cwd>/node_modules/<package>/<entry>`, else `<npm root -g>/<package>/<entry>`, else `None`.
///
/// The package's declared `fallback` (`npx -y <package>@<pinned>`) is deliberately **not** used
/// here: it is a command with arguments, and a [`ToolMap`] entry is one string that lands in
/// `command` or inside a single `args[i]`. Milestone 5's probe owns that tier, where the whole
/// `AgentLaunch` can be rewritten rather than one placeholder substituted.
async fn node_package(package: &str, entry: &str, cwd: &Path) -> Result<Option<String>> {
    let suffix = PathBuf::from(package).join(entry);

    let local = cwd.join("node_modules").join(&suffix);
    if exists(&local).await {
        return Ok(Some(local.to_string_lossy().into_owned()));
    }

    let Some(root) = npm_root_global().await? else {
        return Ok(None);
    };
    let global = root.join(&suffix);
    if exists(&global).await {
        return Ok(Some(global.to_string_lossy().into_owned()));
    }
    Ok(None)
}

/// The directory `npm root -g` prints, or `None` when `npm` is absent or fails.
///
/// A missing `npm` is not an error: it means this tier found nothing, and the caller's
/// [`DriverError::Unresolved`] is the honest report. A *broken* `npm` is treated the same way, on
/// purpose — the resolution answer is identical and a transport error would name the wrong problem.
async fn npm_root_global() -> Result<Option<PathBuf>> {
    let Ok(npm) = tokio::task::spawn_blocking(|| which::which("npm"))
        .await
        .map_err(|err| DriverError::Transport(format!("npm lookup did not run: {err}")))?
    else {
        return Ok(None);
    };
    let output = tokio::process::Command::new(npm)
        .args(["root", "-g"])
        .output()
        .await;
    let Ok(output) = output else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(root)))
}

/// Whether `path` exists, off the runtime's worker.
async fn exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
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
        ToolProbe::Glob { .. } => "glob probes arrive with MOD-2 milestone 5".to_owned(),
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
    async fn a_glob_probe_is_unresolved_rather_than_guessed() {
        let probe = discovery(
            "agy_acp_server",
            ToolProbe::Glob {
                patterns: vec!["**/agy_acp_server*".to_owned()],
                platform: BTreeMap::new(),
            },
        );
        let err = resolve(Some(&probe), Path::new("."))
            .await
            .expect_err("glob resolves nowhere in milestone 3");
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
