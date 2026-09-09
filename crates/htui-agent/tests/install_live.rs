//! A third agent, installed from a registry row alone (plan MOD-20 T9, `R-AGT-5`, `R-AGT-10`).
//!
//! `#[ignore]` by default, exactly as `tests/probe_live.rs` and `tests/agy_live.rs` are: it reads
//! the real ACP registry and downloads a ~36 MB archive from the vendor's CDN, and a box with no
//! network is not a failing build. Run it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test install_live -- --ignored --nocapture
//! ```
//!
//! **What it proves.** The PRD's success metric is *a third agent installed with no code change*.
//! `agy` and `claude` are seeded rows the workspace has been carrying since MOD-2; the agent this
//! file installs is one **nobody wrote code for**. Its registry row is written as JSON in
//! [`amp_row`] below — no seed document declares it, no source file names it, no code path is
//! keyed on it. If [`plan`] and [`install`] can turn that JSON into a running adapter, then the
//! row is genuinely the whole interface, and `R-AGT-5` ("nothing under `src/` knows an agent's
//! name") is a property of the design rather than a claim about the current tree. The companion
//! sweep `the_installer_names_no_vendor` in `tests/extensibility.rs` enforces the other half: the
//! vendor names and URLs in this file may appear **here** and nowhere under `src/`.
//!
//! **It burns no model tokens.** The pipeline ends in [`probe_agent`], whose tier 2 completes
//! `initialize` and nothing else (`crate::acp::handshake`). No session is opened and no prompt is
//! ever sent, so this test costs bandwidth and disk, never quota.
//!
//! **It never touches the maintainer's install root.** `HTUI_AGENTS_ROOT` is pointed at a
//! `tempdir` through [`InstallConfig::root_override`], which [`InstallConfig::apply_to`] turns
//! into a [`ProbeEnv::vars`] entry — `std::env::set_var` is `unsafe`, the workspace forbids it,
//! and a process-wide mutation would leak into every other test in the binary besides. The
//! override is asserted to have landed *before* a byte is fetched, so a bug in the injection
//! fails as itself rather than as 36 MB written into `~/.local/share/htui/agents`.
//!
//! **Preconditions, and what a miss does.** There is exactly one, and it is skipped **by name**
//! rather than failed, because a box behind a firewall must not have a red build:
//!
//! - The ACP registry and the vendor's CDN are reachable. A transport failure arrives as
//!   [`PlanError::Network`] or [`InstallError::Network`] and prints `SKIPPED:` with the
//!   transport's own words.
//! - The registry publishes this entry for this platform. It publishes all five as of 0.9.0, but
//!   a sixth platform running this suite gets [`PlanError::NotAvailable`] and the same `SKIPPED:`
//!   treatment: "the vendor does not ship here" is not a defect in `htui`.
//!
//! **What is asserted, and what is only observed.** The `agy_live.rs` rule applies in full:
//! nothing the vendor may change from one release to the next is asserted. So the plan's digest
//! is asserted to be *present* (the registry publishes `sha256` on every platform of this entry,
//! which is what makes it the right entry for this proof) and the licence to be the one the
//! consent pane will show; the download is asserted to have been verified against that published
//! digest; the tree is asserted to be where the row's own glob looks; the handshake is asserted to
//! have answered at wire protocol 1. But the **status** is asserted only to be one of the two the
//! pipeline calls success — `ready` or `unauthenticated` — and which one it actually is, is
//! printed. Whether this adapter advertises auth methods on a box holding no credential for it is
//! an observation about the vendor, not an assumption `htui` is entitled to make.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use htui_agent::install::{
    InstallConfig, InstallError, InstallJob, InstallOutcome, InstallPhase, InstallProgress,
    Installer, Manifest, PlanError, REGISTRY_BASE, install, plan,
};
use htui_agent::probe::{
    INSTALL_ROOT_VAR, ProbeContext, ProbeEnv, ProbeStatus, SpawnTier2, install_root, platform_key,
};
use htui_core::model::{Agent, AgentId, Billing, BoxId, Transport};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// The registry entry this proof installs, and the licence its terms are published under.
///
/// Written here as constants rather than inline so the two things the test asserts about the
/// registry's *promise* — that it publishes a digest for this entry, and that the terms are the
/// ones the consent pane will show — read as the pinned facts they are. Both were verified
/// against the live document on 2026-09-09, at entry version 0.9.0.
const REGISTRY_ID: &str = "amp-acp";
/// The `discovery.tools` key whose glob must resolve whatever the installer writes (plan D5).
const TOOL: &str = "amp-acp";
/// The SPDX identifier the entry publishes.
const LICENSE: &str = "Apache-2.0";

/// How long the whole pipeline is given.
///
/// Generous rather than tuned: it covers a ~36 MB transfer from a CDN redirect plus an unpack, and
/// then a cold first start of the adapter for the handshake. A shorter bound would report a slow
/// line as a broken installer.
const LIVE_TIMEOUT: Duration = Duration::from_secs(600);

/// The row nobody wrote code for.
///
/// Every coordinate an installer needs is in this JSON and nowhere else: a `Glob` tool whose
/// pattern reaches the install root through `%HTUI_AGENTS_ROOT%` (so the row spells no path on any
/// platform), an `install` block naming the registry and the entry id, and `handshake: true` so
/// the re-probe goes all the way to `initialize` rather than stopping at "the file exists".
///
/// The `.exe` pattern is the same tool, not a second one: this entry's `cmd` is `./amp-acp` on
/// unix and `amp-acp.exe` on Windows, and a row that could only be proven on one of them would be
/// proving something narrower than "a row is the whole interface".
fn amp_row() -> Agent {
    let now = Utc::now();
    Agent {
        id: AgentId::new(),
        name: "amp".to_owned(),
        transport: Transport::Acp,
        launch: json!({
            "command": format!("${{{TOOL}}}"),
            "args": [],
            "env": {},
            "discovery": {
                "tools": {
                    TOOL: {
                        "kind": "glob",
                        "patterns": [
                            format!("%{INSTALL_ROOT_VAR}%/{REGISTRY_ID}/*/amp-acp"),
                            format!("%{INSTALL_ROOT_VAR}%/{REGISTRY_ID}/*/amp-acp.exe"),
                        ],
                    },
                },
                "handshake": true,
                "install": {
                    "source": "acp_registry",
                    "id": REGISTRY_ID,
                    "tool": TOOL,
                },
            },
        }),
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// Every process this test process is the direct parent of, as pid strings.
///
/// Copied from `tests/probe_live.rs` rather than shared, as this repo duplicates its process
/// helpers per file. By **pid**, never by a `pgrep -f` pattern: a pattern matches the shell that
/// launched `cargo test` and any editor with the word in its command line, so it fails for reasons
/// that have nothing to do with this code. A child that was killed but not reaped still appears
/// here — which is the point, since `acp::handshake` promises the tree is killed *and* reaped
/// before it returns.
///
/// `children` exists only with `CONFIG_PROC_CHILDREN=y`; a run where not one was readable falls
/// back to [`children_by_ppid`] so the survivor assertion can never pass vacuously.
#[cfg(target_os = "linux")]
fn children_of_this_process() -> Vec<String> {
    let threads = std::fs::read_dir("/proc/self/task").expect("this process's own thread list");
    let mut pids = Vec::new();
    let mut readable = false;
    for thread in threads.flatten() {
        if let Ok(text) = std::fs::read_to_string(thread.path().join("children")) {
            readable = true;
            pids.extend(text.split_ascii_whitespace().map(ToOwned::to_owned));
        }
    }
    if readable {
        return pids;
    }
    eprintln!("/proc/self/task/*/children is unreadable here; scanning /proc for ppid instead");
    children_by_ppid()
}

/// Every process whose parent is this one, by scanning `/proc/<pid>/stat`.
///
/// The `ppid` field is fourth, after a `comm` that may itself hold spaces and parens — hence the
/// split at the **last** `)`.
#[cfg(target_os = "linux")]
fn children_by_ppid() -> Vec<String> {
    let me = std::process::id().to_string();
    let mut pids = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str() else { continue };
        if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
        if after.split_ascii_whitespace().nth(1) == Some(me.as_str()) {
            pids.push(pid.to_owned());
        }
    }
    pids
}

/// `mode`, or the closest thing the platform has.
#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:o}", meta.permissions().mode())
}

/// `mode`, or the closest thing the platform has.
#[cfg(not(unix))]
fn mode_of(meta: &std::fs::Metadata) -> String {
    format!("readonly={}", meta.permissions().readonly())
}

/// The tree as it landed, printed the way `agy_live.rs` prints an install directory.
fn print_tree(dir: &Path) {
    println!("--- {} ---", dir.display());
    let Ok(entries) = std::fs::read_dir(dir) else {
        println!("  <unreadable>");
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(meta) = entry.metadata() else { continue };
        println!(
            "  {:<24} {:>12} bytes  mode {}",
            entry.file_name().to_string_lossy(),
            meta.len(),
            mode_of(&meta)
        );
    }
}

/// Two paths compared through the filesystem's own idea of them.
///
/// `canonicalize` on both sides because a `tempdir` is under `/tmp`, which is a symlink on more
/// than one platform this suite runs on: comparing the strings would fail on a box where nothing
/// is wrong.
fn same_file(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// The proof: a row, a registry, and a box that can run what it did not have.
///
/// The whole pipeline in one case on purpose. Splitting it would mean either downloading 36 MB
/// twice or sharing state between cases, and the thing being proven is precisely that the steps
/// compose: a plan the user could have consented to, executed unchanged, ending in a probe that
/// says the box can run it.
#[tokio::test]
#[ignore = "downloads ~36 MB from the real ACP registry and spawns the adapter it installs"]
async fn a_third_agent_installs_from_a_registry_row_alone() {
    let tmp = tempfile::tempdir().expect("a temporary install root");
    let root = tmp.path().join("agents");
    let config = InstallConfig::new(REGISTRY_BASE, Some(root.clone()));
    let installer = Installer::new(config).expect("the HTTP clients build");

    // The maintainer's real root is never in reach: `ProbeEnv::host` seeds `HTUI_AGENTS_ROOT` from
    // `dirs::data_local_dir()`, and this is the value that overwrites it. Asserted here, before
    // `plan()` so much as writes its registry cache — the one moment at which a failed injection
    // is still free.
    let env = installer
        .config()
        .apply_to(ProbeEnv::host(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
    let injected = install_root(&env).expect("the override seeded the install root");
    assert!(
        injected.starts_with(tmp.path()),
        "{INSTALL_ROOT_VAR} must point inside the tempdir, not at {}",
        injected.display()
    );
    println!("platform      = {}", platform_key());
    println!("registry base = {REGISTRY_BASE}");
    println!("install root  = {} (temporary)", injected.display());

    let agent = amp_row();
    println!(
        "the row, in full (no seed, no source file and no code path knows it):\n{}",
        serde_json::to_string_pretty(&agent.launch).expect("the row's launch re-serialises")
    );

    // ---- The pre-flight, against the real registry -------------------------------------------

    let started = Instant::now();
    let planned = tokio::time::timeout(LIVE_TIMEOUT, plan(&installer, &agent, &env, Utc::now()))
        .await
        .expect("the registry answered within the live timeout");
    let plan_took = started.elapsed();

    let planned = match planned {
        Ok(planned) => planned,
        // Skipped by name, never failed: a box with no route to the registry has nothing to say
        // about this code (blueprint F, the live row).
        Err(PlanError::Network { message, manual }) => {
            println!("SKIPPED: the ACP registry is unreachable from this box: {message}");
            for line in manual.lines() {
                println!("  {line}");
            }
            return;
        }
        Err(PlanError::NotAvailable {
            id,
            version,
            platform,
        }) => {
            println!(
                "SKIPPED: the registry publishes no `{id}` {version} for {platform}. That is the \
                 vendor's decision, not a defect in the installer."
            );
            return;
        }
        Err(other) => panic!("the pre-flight refused for a reason this box can act on: {other}"),
    };

    println!("plan() took {plan_took:?}");
    println!(
        "the plan:\n{}",
        serde_json::to_string_pretty(&planned).expect("the plan re-serialises")
    );
    println!("--- what the consent pane would show ---");
    for line in planned.consent_lines() {
        println!("  {line}");
    }

    assert_eq!(planned.registry_id, REGISTRY_ID);
    assert_eq!(planned.platform, platform_key());
    let published = planned
        .sha256
        .clone()
        .expect("this entry publishes a sha256 on every platform, which is why it is this entry");
    assert_eq!(
        planned.license.as_deref(),
        Some(LICENSE),
        "the licence the consent pane shows is the registry's own word for it"
    );
    assert!(
        planned.license_url.is_some(),
        "terms with no URL would be a consent pane with nothing to point at: {planned:?}"
    );
    assert!(
        planned.root.starts_with(tmp.path()) && planned.install_dir.starts_with(tmp.path()),
        "the plan would write outside the tempdir: {}",
        planned.install_dir.display()
    );
    assert!(
        planned.existing_versions.is_empty(),
        "a fresh root has nothing installed under `{REGISTRY_ID}`: {:?}",
        planned.existing_versions
    );
    assert!(
        planned.consent.is_none() && planned.recorded.is_none(),
        "a fresh root has no manifest, so `y` is what accepts these terms: {planned:?}"
    );
    assert!(
        planned.registry_cached_age_secs.is_none(),
        "the document came off the network, not out of a cache this run created"
    );
    println!(
        "published sha256 = {published}\ndigest sentence  = {}",
        planned.digest_sentence()
    );

    // ---- The install, with the production tier 2 ---------------------------------------------

    let ctx = ProbeContext {
        env: env.clone(),
        now: Utc::now(),
    };
    let box_id = BoxId::new();
    let cancel = CancellationToken::new();
    let mut frames: Vec<InstallProgress> = Vec::new();
    let mut sink = |frame: InstallProgress| frames.push(frame);

    let started = Instant::now();
    let outcome = tokio::time::timeout(
        LIVE_TIMEOUT,
        install(
            &installer,
            InstallJob {
                plan: &planned,
                agent: &agent,
                box_id,
                existing: None,
                ctx: &ctx,
                tier2: &SpawnTier2::default(),
            },
            &mut sink,
            &cancel,
        ),
    )
    .await
    .expect("the download, the unpack and the handshake fit inside the live timeout");
    let install_took = started.elapsed();
    println!("install() took {install_took:?}");

    let outcome = match outcome {
        Ok(outcome) => outcome,
        // The same rule as the pre-flight's: the CDN is a second host, and it can be down while
        // the registry is up.
        Err(InstallError::Network { message, manual }) => {
            println!("SKIPPED: the archive host is unreachable from this box: {message}");
            for line in manual.lines() {
                println!("  {line}");
            }
            return;
        }
        Err(other) => panic!("the install failed: {other}"),
    };

    println!("--- progress frames, as the sink received them ---");
    for frame in &frames {
        println!(
            "  {:<12} done={} total={:?}",
            frame.phase.as_str(),
            frame.done,
            frame.total
        );
    }
    let downloaded = frames
        .iter()
        .filter(|frame| frame.phase == InstallPhase::Downloading)
        .map(|frame| frame.done)
        .max()
        .expect("the download reported at least its final frame");
    println!("bytes streamed = {downloaded}");
    if let Some(length) = planned.content_length {
        assert_eq!(
            downloaded, length,
            "the whole archive the `HEAD` promised arrived (the CDN redirect included)"
        );
    }

    let (record, version, dir, row, status, removed_versions, digest_changed) = match outcome {
        InstallOutcome::Installed {
            record,
            version,
            dir,
            row,
            status,
            removed_versions,
            digest_changed,
        } => (
            record,
            version,
            dir,
            row,
            status,
            removed_versions,
            digest_changed,
        ),
        // Not a soft landing: `Failed` is the probe refusing a tree that downloaded and promoted,
        // which is exactly the finding this test exists to surface. Everything it carries is
        // printed before the panic, because the panic message alone would not be diagnosable.
        InstallOutcome::Failed {
            record,
            version,
            status,
            stderr_tail,
            restored,
            probe,
        } => {
            println!("record  = {record:?}");
            println!("restored = {restored:?}");
            println!("probe   = {probe:?}");
            panic!(
                "the tree promoted and the probe refused it: {version} is `{}`, stderr {stderr_tail:?}",
                status.as_str()
            );
        }
    };

    // ---- What the pipeline is asserted to have done ------------------------------------------

    println!("installed {version} into {}", dir.display());
    print_tree(&dir);

    assert_eq!(version, planned.version);
    assert_eq!(dir, planned.install_dir);
    assert!(
        removed_versions.is_empty() && !digest_changed,
        "a first install replaces nothing: removed={removed_versions:?} changed={digest_changed}"
    );

    // The digest: verified against the *published* one, not merely computed. `install` would have
    // answered `DigestMismatch` before unpacking a byte otherwise, so this pins that the branch
    // taken was the verifying one.
    assert!(
        record.published,
        "the registry published a digest, so the download was checked against it: {record:?}"
    );
    assert_eq!(
        record.sha256, published,
        "the bytes on disk hash to what the registry promised"
    );
    assert_eq!(record.archive, planned.archive_url);
    assert_eq!(record.platform, planned.platform);

    // The tree: where the registry said the command would be, and where the row's glob looks.
    let cmd = dir.join(planned.cmd.trim_start_matches("./"));
    assert!(
        cmd.is_file(),
        "the entry's `cmd` names {} and the unpacked tree does not have it",
        cmd.display()
    );
    let resolved = row
        .path
        .clone()
        .expect("a row the probe resolved carries the command it resolved");
    assert!(
        same_file(Path::new(&resolved), &cmd),
        "the re-probe resolved {resolved}, the installer wrote {}",
        cmd.display()
    );
    println!("the row's glob resolved {resolved}");

    // The manifest: this box's own record of what it accepted and what it has.
    let manifest = Manifest::load(&root.join(REGISTRY_ID).join("manifest.json")).await;
    println!(
        "manifest.json = {}",
        serde_json::to_string_pretty(&manifest).expect("the manifest re-serialises")
    );
    assert!(
        manifest.consent_covers(planned.license.as_deref(), planned.license_url.as_deref()),
        "`y` accepted these terms and the manifest is where that survives: {manifest:?}"
    );
    assert_eq!(manifest.installs.get(&version), Some(&record));

    // ---- What the probe said, and what is only observed ---------------------------------------

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    println!(
        "agent_box.probe = {}",
        serde_json::to_string_pretty(&probe).expect("the snapshot re-serialises")
    );
    println!(
        "agent_box columnar: enabled={} version={:?} path={:?} probed_at={:?}",
        row.enabled, row.version, row.path, row.probed_at
    );

    assert_eq!(
        probe["handshake"]["protocol_version"],
        json!(1),
        "the adapter the installer wrote speaks wire protocol 1: {probe}"
    );
    assert_eq!(row.agent_id, agent.id);
    assert_eq!(row.box_id, box_id);
    assert_eq!(row.probed_at, Some(ctx.now));
    assert_eq!(
        row.enabled,
        status == ProbeStatus::Ready,
        "a `ready` box is enabled and nothing else is (plan D50)"
    );

    // The `agy_live.rs` rule. `Installed` is only ever returned for these two, so the pair is the
    // pipeline's own contract and is asserted; **which** of the two this box lands on depends on
    // whether the vendor advertises auth methods to a box holding no credential for it, and that
    // is an observation, printed.
    assert!(
        matches!(status, ProbeStatus::Ready | ProbeStatus::Unauthenticated),
        "`Installed` means the probe could run it: {status:?}"
    );
    let auth_methods = probe["handshake"]["auth_methods"].clone();
    println!("OBSERVED status  = {}", status.as_str());
    println!("OBSERVED authMethods = {auth_methods}");
    println!(
        "OBSERVED agentInfo   = name {} version {}",
        probe["handshake"]["agent_name"], probe["handshake"]["agent_version"]
    );

    // Nothing this process started is still around. The child belongs to `acp::handshake`, which
    // kills *and reaps* on every exit path, so even a zombie fails this.
    #[cfg(target_os = "linux")]
    {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let survivors = children_of_this_process();
        assert!(
            survivors.is_empty(),
            "the install's re-probe left children behind: {survivors:?}"
        );
    }
}
