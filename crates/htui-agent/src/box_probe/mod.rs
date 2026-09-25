//! The box probe (MOD-7 plan D6–D9): what this box is and what it can build.
//!
//! [`probe_box`] reads the hardware through a [`HardwareSource`] and walks every tool of an
//! [`EffectiveSpec`] through [`probe`](crate::probe)'s resolver — the same tier walk an agent row's
//! `discovery.tools` goes through (MOD-2 D46's "one resolver, two callers") — then derives the
//! tags. It never writes and never fails: the [`BoxProbe`] it returns is what
//! `WriteStore::record_box_probe` records, and an unreadable fact is simply empty.
//!
//! Every tool name lives in `spec.json` (plan D9, PRD D2), never in this code.

pub mod hardware;
pub mod spec;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, BoxProbe, ProbedTool};
use regex::Regex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::error::Result;
use crate::launch::{ResolvedLaunch, ToolProbe};
use crate::probe::{ProbeEnv, ToolResolution, resolve_tool, run_bounded};

use self::hardware::HardwareSource;
use self::spec::{EffectiveSpec, Fact, GpuVendor, TagRule};

/// At most this many tool lookups and version children at once (plan D9).
pub const MAX_CONCURRENT_TOOLS: usize = 8;

/// One box probe (plan D8): hardware through `hardware`, tools and tags through `spec`, on this
/// copy of `env` with `versions` forced on (OQ-11). Never fails; an unreadable fact is empty.
///
/// Tools resolve concurrently on a [`JoinSet`] behind a [`MAX_CONCURRENT_TOOLS`]-permit
/// [`Semaphore`], and the `ask` children of the tag rules (the MinGW `gcc` case) share the same
/// permits (blueprint D36). A versioned tool counts only when its version was captured (OQ-11): a
/// broken shim or a `--version` that hangs past `version_timeout` is absent. Dropping the future
/// aborts every task, and each child dies with its `ChildGuard`.
pub async fn probe_box(
    box_id: BoxId,
    env: &ProbeEnv,
    hardware: &dyn HardwareSource,
    spec: &EffectiveSpec,
    htui_version: &str,
    now: DateTime<Utc>,
) -> BoxProbe {
    let env = Arc::new(ProbeEnv {
        versions: true,
        ..env.clone()
    });
    let hw = hardware.read(&env).await;
    let gpu = pick_gpu_vendor(&hw.display_vendors, &spec.spec.gpu_vendors);
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_TOOLS));

    let found = probe_tools(&spec.spec.tools, &env, &permits).await;
    let asked = ask_tools(&spec.spec.tags, &found, &env, &permits).await;
    let present: BTreeSet<String> = found.keys().cloned().collect();
    let probed_tags = tags_from_presence(&spec.spec.tags, &present, gpu.is_some(), &asked);

    BoxProbe {
        box_id,
        os_version: hw.os_version,
        cpu: hw.cpu,
        ram_mb: hw.ram_mb,
        gpu_present: gpu.is_some(),
        gpu_vendor: gpu.map(str::to_owned),
        tools: found.into_values().collect(),
        probed_tags,
        htui_version: htui_version.to_owned(),
        spec_digest: spec.digest.clone(),
        probed_at: now,
    }
}

/// The first vendor of `map`, in its order, whose `pci` is among `found` (plan D7).
#[must_use]
pub fn pick_gpu_vendor<'a>(found: &[String], map: &'a [GpuVendor]) -> Option<&'a str> {
    map.iter()
        .find(|vendor| found.contains(&vendor.pci))
        .map(|vendor| vendor.name.as_str())
}

/// The tags `rules` derive (PRD D2), sorted and deduplicated. `asked` holds each `ask` rule's answer
/// by tag; a rule with an `ask` and no answer does not fire.
#[must_use]
pub fn tags_from_presence(
    rules: &[TagRule],
    present: &BTreeSet<String>,
    gpu_present: bool,
    asked: &BTreeMap<String, bool>,
) -> Vec<String> {
    let mut tags = BTreeSet::new();
    for rule in rules {
        let base = match rule.fact {
            Some(Fact::Gpu) => gpu_present,
            None => rule.any_tool.iter().any(|tool| present.contains(tool)),
        };
        let answered = rule.ask.is_none() || asked.get(&rule.tag).copied().unwrap_or(false);
        if base && answered {
            tags.insert(rule.tag.clone());
        }
    }
    tags.into_iter().collect()
}

/// Every tool of the spec, resolved under the shared permits: name → what was found.
async fn probe_tools(
    tools: &BTreeMap<String, ToolProbe>,
    env: &Arc<ProbeEnv>,
    permits: &Arc<Semaphore>,
) -> BTreeMap<String, ProbedTool> {
    let mut set = JoinSet::new();
    for (name, probe) in tools {
        let (name, probe) = (name.clone(), probe.clone());
        let (env, permits) = (Arc::clone(env), Arc::clone(permits));
        set.spawn(async move {
            // The semaphore is never closed, so `acquire` cannot fail; a failure would only mean
            // running unthrottled, never skipping the tool.
            let _permit = permits.acquire_owned().await.ok();
            probe_tool(name, &probe, &env).await
        });
    }

    let mut found = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Some(tool)) => {
                found.insert(tool.name.clone(), tool);
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "a box probe tool task did not finish"),
        }
    }
    found
}

/// One tool: present when found, and — for a versioned tool — only when its version was captured
/// (OQ-11).
///
/// A versioned `kind: "path"` tool tries its `names` one at a time, in order, and keeps the first
/// whose version was captured: the Windows Store `python3.exe` stub is found first but prints no
/// version, and the `python` after it is the real interpreter. A presence-only tool, or any other
/// kind, is resolved once and keeps the first file found.
async fn probe_tool(name: String, probe: &ToolProbe, env: &ProbeEnv) -> Option<ProbedTool> {
    let ToolProbe::Path {
        names,
        version: Some(version),
    } = probe
    else {
        return accept(name, resolve_tool(probe, env).await, false);
    };
    for file in names {
        let one = ToolProbe::Path {
            names: vec![file.clone()],
            version: Some(version.clone()),
        };
        if let Some(tool) = accept(name.clone(), resolve_tool(&one, env).await, true) {
            return Some(tool);
        }
    }
    None
}

/// What one resolution of tool `name` reports: the tool when found and — when `versioned` — its
/// version was captured, else nothing, logged.
fn accept(
    name: String,
    resolved: Result<Option<ToolResolution>>,
    versioned: bool,
) -> Option<ProbedTool> {
    match resolved {
        Ok(Some(found)) if !versioned || found.version.is_some() => Some(ProbedTool {
            version: found.version.unwrap_or_default(),
            path: found.path.to_string_lossy().into_owned(),
            name,
        }),
        Ok(Some(found)) => {
            debug!(
                tool = %name,
                path = %found.path.display(),
                "found, but no version was captured; counted absent"
            );
            None
        }
        Ok(None) => {
            debug!(tool = %name, "not found");
            None
        }
        Err(error) => {
            warn!(tool = %name, %error, "the box probe could not look for a tool; counted absent");
            None
        }
    }
}

/// Each `ask` rule's answer, by tag: its first present tool, run by its resolved path with the
/// rule's `args` under the shared permits (blueprint D36). A rule whose tools are all absent is
/// not asked.
async fn ask_tools(
    rules: &[TagRule],
    found: &BTreeMap<String, ProbedTool>,
    env: &Arc<ProbeEnv>,
    permits: &Arc<Semaphore>,
) -> BTreeMap<String, bool> {
    let mut set = JoinSet::new();
    for rule in rules {
        let Some(ask) = &rule.ask else {
            continue;
        };
        let Some(tool) = rule.any_tool.iter().find_map(|name| found.get(name)) else {
            continue;
        };
        let launch = ResolvedLaunch {
            command: tool.path.clone(),
            args: ask.args.clone(),
            env: BTreeMap::new(),
        };
        let (tag, matches) = (rule.tag.clone(), ask.matches.clone());
        let (env, permits) = (Arc::clone(env), Arc::clone(permits));
        set.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            let answer = ask_tool(&launch, &matches, &env).await;
            (tag, answer)
        });
    }

    let mut asked = BTreeMap::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((tag, answer)) => {
                asked.insert(tag, answer);
            }
            Err(error) => warn!(%error, "a box probe ask task did not finish"),
        }
    }
    asked
}

/// Whether any trimmed line of the child's stdout, then its stderr tail, matches `matches`. A child
/// that did not run, or a pattern that does not compile, answers no.
async fn ask_tool(launch: &ResolvedLaunch, matches: &str, env: &ProbeEnv) -> bool {
    let regex = match Regex::new(matches) {
        Ok(regex) => regex,
        Err(error) => {
            warn!(pattern = %matches, %error, "an ask pattern does not compile; answering no");
            return false;
        }
    };
    let Some(out) = run_bounded("ask", launch, env).await else {
        return false;
    };
    out.stdout
        .lines()
        .chain(out.stderr.iter().map(String::as_str))
        .map(str::trim)
        .any(|line| regex.is_match(line))
}
