//! The box probe, tier 1 and tier 2 (`docs/ANA-4.md` §4.6, plan MOD-2 D46–D51, T24 and T25).
//!
//! Every case here runs against a throwaway directory tree and an **injected** [`ProbeEnv`]: the
//! workspace forbids `unsafe`, so `std::env::set_var` is not available and the environment a tier
//! reads is a value, not a global. Nothing in this file probes an unmodified seed *row* end to end
//! — this box has `node`, `claude` and the ACP adapter installed, and a probe of the seeded
//! `claude` row would spawn the real adapter inside `cargo test` (blueprint H-7); the live probe
//! lives in `tests/probe_live.rs`, `#[ignore]`d.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{TimeDelta, Utc};
use htui_agent::acp::{AcpIo, Handshake, handshake};
use htui_agent::driver::DriverFuture;
use htui_agent::error::DriverError;
use htui_agent::launch::{
    AcpSettings, AgentLaunch, Discovery, ResolvedLaunch, ToolProbe, VersionProbe,
};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeSource, ProbeStatus, Tier2,
    below_min, capture_version, expand, extract_version, glob_first, platform_key, probe_agent,
    probe_tools, resolve_tool, segment_matches, status_for, walk,
};
use htui_core::model::{Agent, AgentBox, AgentId, Billing, BoxId, Transport};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A box made of directories under `tmp`: `cwd`, `bin` (the injected `PATH`), `home` (what `~`
/// expands to) and `lad` (what `%LOCALAPPDATA%` expands to).
fn env(tmp: &Path) -> ProbeEnv {
    for dir in ["cwd", "bin", "home", "lad"] {
        std::fs::create_dir_all(tmp.join(dir)).expect("fixture directory");
    }
    let mut vars = BTreeMap::new();
    vars.insert(
        "PATH".to_owned(),
        tmp.join("bin").to_string_lossy().into_owned(),
    );
    vars.insert(
        "LOCALAPPDATA".to_owned(),
        tmp.join("lad").to_string_lossy().into_owned(),
    );
    ProbeEnv {
        cwd: tmp.join("cwd"),
        platform: "linux-x86_64".to_owned(),
        home: Some(tmp.join("home")),
        vars,
        versions: true,
        version_timeout: Duration::from_secs(5),
    }
}

/// Writes `contents` at `path`, creating parents, and makes it executable where that is a thing.
///
/// The write handle is closed before this returns, but that is not enough on Linux: the test
/// harness runs these tests on threads of one process, and a `fork` in **another** test's spawn
/// inherits every fd open at that instant. A child holding a write fd to this file makes `execve`
/// answer `ETXTBSY` until it execs or exits, which is why every fixture spawn goes through
/// [`spawn_fixture`] and its retry rather than calling `launch::spawn` directly.
fn executable(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// [`htui_agent::launch::spawn`], retrying the `ETXTBSY` window [`executable`] describes.
///
/// The window is short — it closes as soon as the racing child execs — so a bounded retry is the
/// whole fix. Anything else that fails still fails on the first attempt's error.
#[cfg(unix)]
async fn spawn_fixture(
    launch: &ResolvedLaunch,
    cwd: &Path,
) -> htui_agent::error::Result<htui_agent::launch::Spawned> {
    for _ in 0..20u32 {
        match htui_agent::launch::spawn(launch, cwd).await {
            Err(err) if err.to_string().contains("Text file busy") => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            other => return other,
        }
    }
    htui_agent::launch::spawn(launch, cwd).await
}

/// The pid a fixture recorded at `path`, once it has recorded one.
///
/// The one way back to a process `capture_version` and `npm_root_global` own privately: they spawn
/// internally, so no test can hold their [`htui_agent::launch::Spawned`] and ask it for a pid. The
/// fixture writes `$$` — the shell that is the direct child, and on unix the process-group leader
/// whose group `kill_tree` targets.
#[cfg(unix)]
fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// [`read_pid`], waited for.
#[cfg(unix)]
async fn await_pid(path: &Path) -> u32 {
    for _ in 0..200u32 {
        if let Some(pid) = read_pid(path) {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the fixture never recorded its pid at {}", path.display());
}

/// Windows needs an extension `PATHEXT` names before `which` will answer for a bare name.
fn tool_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

fn touch_at(path: &Path, at: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(at)
        .expect("set mtime");
}

fn discovery(name: &str, probe: ToolProbe) -> Discovery {
    let mut tools = BTreeMap::new();
    tools.insert(name.to_owned(), probe);
    Discovery {
        tools,
        handshake: false,
    }
}

fn path_probe(names: &[&str], version: Option<VersionProbe>) -> ToolProbe {
    ToolProbe::Path {
        names: names.iter().map(|n| (*n).to_owned()).collect(),
        version,
    }
}

/// The `agy` seed row's own `discovery`, not a copy of it: the glob tier is tested against the
/// document that ships (`crates/htui-core/seeds/agent_agy.json`).
fn agy_discovery() -> Discovery {
    let rows = htui_core::model::agent::seed_rows(Utc::now());
    let agy = rows
        .into_iter()
        .find(|row| row.name == "agy")
        .expect("the agy seed row");
    let launch: AgentLaunch = serde_json::from_value(agy.launch).expect("the seed launch parses");
    launch.discovery.expect("agy declares a discovery block")
}

// ---------------------------------------------------------------------------------------------
// 1. The platform key
// ---------------------------------------------------------------------------------------------

/// The registry's five keys spell macOS `darwin`; Rust spells it `macos`. That one rename is the
/// whole difference, and a box whose key is not one of the five would silently match no glob.
#[test]
fn the_platform_key_spells_macos_as_darwin() {
    let expected = format!(
        "{}-{}",
        std::env::consts::OS.replace("macos", "darwin"),
        std::env::consts::ARCH
    );
    assert_eq!(platform_key(), expected);
    assert!(
        [
            "darwin-aarch64",
            "linux-x86_64",
            "linux-aarch64",
            "windows-x86_64",
            "windows-aarch64",
        ]
        .contains(&platform_key().as_str()),
        "this box's key is one the ACP registry knows: {}",
        platform_key()
    );
}

// ---------------------------------------------------------------------------------------------
// 2–3. The `Path` and `NodePackage` tiers
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_path_probe_resolves_through_which_in_over_the_injected_path() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");

    let found = resolve_tool(&path_probe(&["nope", "htui-fake-tool"], None), &env)
        .await
        .expect("the lookup ran")
        .expect("the second name is on the injected PATH");
    assert_eq!(found.path, tool);
    assert_eq!(found.version, None, "this probe declares no version tier");
    assert!(found.args.is_empty());

    assert_eq!(
        resolve_tool(&path_probe(&["nope"], None), &env)
            .await
            .expect("the lookup ran"),
        None,
        "a name that is on no PATH entry resolves nowhere"
    );
}

#[tokio::test]
async fn a_node_package_prefers_the_local_tree_and_reads_its_package_json_version() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let package = env.cwd.join("node_modules").join("@scope/pkg");
    let entry = package.join("dist/index.js");
    std::fs::create_dir_all(entry.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&entry, "// entry").expect("write");
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"@scope/pkg","version":"0.48.0"}"#,
    )
    .expect("write");

    let probe = ToolProbe::NodePackage {
        package: "@scope/pkg".to_owned(),
        entry: "dist/index.js".to_owned(),
        pinned: "0.0.0".to_owned(),
        fallback: None,
    };
    let found = resolve_tool(&probe, &env)
        .await
        .expect("the lookup ran")
        .expect("the local tree holds it");
    assert_eq!(found.path, entry);
    assert_eq!(
        found.version.as_deref(),
        Some("0.48.0"),
        "the package's own package.json answers, with no spawn"
    );

    let quiet = ProbeEnv {
        versions: false,
        ..env
    };
    let found = resolve_tool(&probe, &quiet)
        .await
        .expect("the lookup ran")
        .expect("the local tree holds it");
    assert_eq!(
        found.version, None,
        "`versions: false` skips the version tier entirely"
    );
}

/// Review gate HIGH-1: `npm root -g` is a spawned child like every other, and a hung `npm` must be
/// killed at `version_timeout` rather than stalling the caller forever.
///
/// This one is not only on `Settings > r`: `tools::resolve` walks the same tier at every
/// `ChatStart`, where no `HANDSHAKE_TIMEOUT` covers it, so an unbounded wait here is a chat that
/// never starts.
#[cfg(unix)]
#[tokio::test]
async fn a_hung_npm_root_is_killed_at_the_version_timeout() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = ProbeEnv {
        version_timeout: Duration::from_millis(300),
        ..env(tmp.path())
    };
    executable(
        &tmp.path().join("bin/npm"),
        "#!/bin/sh\necho $$ > npm.pid\nsleep 10\n",
    );
    let probe = ToolProbe::NodePackage {
        package: "@scope/pkg".to_owned(),
        entry: "dist/index.js".to_owned(),
        pinned: "0.0.0".to_owned(),
        fallback: None,
    };
    let pidfile = env.cwd.join("npm.pid");

    // `npm_root_global` spawns internally, so this cannot go through `spawn_fixture`'s `ETXTBSY`
    // retry. An `ETXTBSY` attempt is a spawn that never ran, which shows up here as "no pid file";
    // anything else is answered on the first pass.
    for _ in 0..20u32 {
        let _ = std::fs::remove_file(&pidfile);
        let started = std::time::Instant::now();
        assert_eq!(
            resolve_tool(&probe, &env).await.expect("the lookup ran"),
            None,
            "a broken `npm` is `not found`, never an error"
        );
        let Some(pid) = read_pid(&pidfile) else {
            tokio::time::sleep(Duration::from_millis(25)).await;
            continue;
        };
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "a hung `npm root -g` is killed at `version_timeout`, not waited out: {:?}",
            started.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_not_running(pid, "a hung `npm root -g`");
        return;
    }
    panic!("the fixture `npm` never ran");
}

// ---------------------------------------------------------------------------------------------
// 4–8. The glob walker
// ---------------------------------------------------------------------------------------------

#[test]
fn expand_replaces_percent_vars_and_tilde_and_skips_an_unset_var() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());

    let (root, rest) = expand(
        "~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par",
        &env,
    )
    .expect("`~` expands");
    assert_eq!(
        root,
        tmp.path()
            .join("home/.local/share/htui/agents/antigravity-acp")
    );
    assert_eq!(rest, vec!["*".to_owned(), "agy_acp_server.par".to_owned()]);

    let (root, rest) = expand(
        "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe",
        &env,
    )
    .expect("`%LOCALAPPDATA%` expands");
    assert_eq!(root, tmp.path().join("lad/JetBrains"));
    assert_eq!(rest.len(), 5, "everything from the first `*` on: {rest:?}");

    assert_eq!(
        expand("%NOPE%/x", &env),
        None,
        "an unset variable skips the pattern rather than failing the probe"
    );
    let homeless = ProbeEnv {
        home: None,
        ..env.clone()
    };
    assert_eq!(
        expand("~/x", &homeless),
        None,
        "no home, no `~` pattern — a box without one is not an error"
    );

    // A wildcard-first pattern has no directory to start the walk from. Answering `None` is what
    // makes the skip visible; an empty root would send `walk` at `read_dir("")` and report "no
    // match" for a pattern that was never tried.
    assert_eq!(expand("*/bin/tool", &env), None);
    assert_eq!(expand("**/agy_acp_server*", &env), None);
}

#[test]
fn segment_matches_is_star_within_one_name_only() {
    assert!(segment_matches("*", "IntelliJIdea2026.1"));
    assert!(segment_matches("agy*.exe", "agy_acp_server.exe"));
    assert!(segment_matches("*server*", "agy_acp_server.exe"));
    assert!(
        !segment_matches("*", "a/b"),
        "`*` never crosses a path separator"
    );
    assert!(
        segment_matches("**", "IntelliJIdea2026.1"),
        "`**` is not recursion here; it is two stars in one name"
    );
    assert!(segment_matches("agy_acp_server.par", "agy_acp_server.par"));
    assert!(!segment_matches("agy_acp_server.par", "agy_acp_server.exe"));
}

#[tokio::test]
async fn the_jetbrains_two_star_shape_resolves_to_the_newest_install() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let pattern = "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe";

    let base = tmp.path().join("lad/JetBrains");
    let mut installs = Vec::new();
    for ide in ["IntelliJIdea2025.3", "IntelliJIdea2026.1"] {
        for build in ["20260501", "20260818"] {
            let file = base
                .join(ide)
                .join("acp-agents/antigravity-acp")
                .join(build)
                .join("agy_acp_server.exe");
            executable(&file, "binary");
            installs.push(file);
        }
    }

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for file in &installs {
        touch_at(file, epoch);
    }
    let oldest_by_name =
        base.join("IntelliJIdea2025.3/acp-agents/antigravity-acp/20260501/agy_acp_server.exe");
    touch_at(&oldest_by_name, epoch + Duration::from_secs(600));

    let found = glob_first(&[pattern.to_owned()], &env)
        .await
        .expect("the walk ran")
        .expect("four files match");
    assert_eq!(
        found, oldest_by_name,
        "mtime decides, not the version in the directory name"
    );

    for file in &installs {
        touch_at(file, epoch);
    }
    let found = glob_first(&[pattern.to_owned()], &env)
        .await
        .expect("the walk ran")
        .expect("four files match");
    assert_eq!(
        found,
        base.join("IntelliJIdea2026.1/acp-agents/antigravity-acp/20260818/agy_acp_server.exe"),
        "equal mtimes fall back to the descending path, so the answer is stable"
    );
}

#[tokio::test]
async fn the_seeded_agy_row_resolves_by_glob_on_linux_with_the_uid_arg() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let server = tmp
        .path()
        .join("home/.local/share/htui/agents/antigravity-acp/1.1.1/agy_acp_server.par");
    executable(&server, "binary");

    let report = probe_tools(Some(&agy_discovery()), &env)
        .await
        .expect("the tiers ran");

    let found = report
        .found
        .get("agy_acp_server")
        .expect("the glob tier found the server");
    assert_eq!(found.path, server);
    assert_eq!(
        found.args,
        vec!["--uid=".to_owned()],
        "the linux-x86_64 entry's args are the ACP registry's Linux-only flag"
    );
    assert_eq!(
        found.version, None,
        "a glob probe declares no version tier: the handshake answers instead"
    );
    assert_eq!(
        report.missing,
        vec!["agy".to_owned()],
        "`agy` itself is on no injected PATH"
    );
    assert!(!report.is_complete());
}

#[tokio::test]
async fn the_same_row_on_darwin_appends_no_args() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = ProbeEnv {
        platform: "darwin-aarch64".to_owned(),
        ..env(tmp.path())
    };
    let server = tmp.path().join(
        "home/Library/Application Support/htui/agents/antigravity-acp/1.1.1/agy_acp_server.par",
    );
    executable(&server, "binary");

    let report = probe_tools(Some(&agy_discovery()), &env)
        .await
        .expect("the tiers ran");
    let found = report
        .found
        .get("agy_acp_server")
        .expect("the darwin patterns found the server");
    assert_eq!(found.path, server);
    assert!(
        found.args.is_empty(),
        "darwin-aarch64 declares no extra args"
    );
    assert!(report.extra_args().is_empty());
}

/// Review gate LOW-4: a user-authored `~/*/*/*` over a large home fans out multiplicatively on the
/// blocking pool, for a walk whose answer is one file. The cap is what stops it.
#[test]
fn the_glob_walk_caps_its_candidate_fan_out() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let wide = tmp.path().join("home/wide");
    std::fs::create_dir_all(&wide).expect("mkdir");
    for index in 0..4_200u32 {
        std::fs::write(wide.join(format!("f{index:05}")), "x").expect("write");
    }

    let (root, segments) = expand("~/wide/*", &env).expect("`~` expands");
    let matches = walk(&root, &segments);
    assert_eq!(
        matches.len(),
        4096,
        "the walk carries at most 4096 candidates between segments"
    );
}

// ---------------------------------------------------------------------------------------------
// 9–11. Version capture
// ---------------------------------------------------------------------------------------------

#[test]
fn extract_version_handles_the_three_seed_patterns() {
    assert_eq!(
        extract_version("v22.19.0", r"^v(\d+\.\d+\.\d+)$").as_deref(),
        Some("22.19.0")
    );
    assert_eq!(
        extract_version(
            "2.1.263 (Claude Code)",
            r"^(\d+\.\d+\.\d+) \(Claude Code\)$"
        )
        .as_deref(),
        Some("2.1.263")
    );
    assert_eq!(
        extract_version("11.7.0", r"^(\d+\.\d+\.\d+)$").as_deref(),
        Some("11.7.0")
    );
    assert_eq!(
        extract_version("1.1.26", r"^v?(\d+\.\d+\.\d+)").as_deref(),
        Some("1.1.26")
    );
    assert_eq!(
        extract_version("v1.1.26-rc1", r"^v?(\d+\.\d+\.\d+)").as_deref(),
        Some("1.1.26")
    );
    assert_eq!(
        extract_version("garbage", r"^v(\d+\.\d+\.\d+)$"),
        None,
        "no match is `present, version unknown`"
    );
    assert_eq!(
        extract_version("v22.19.0", "("),
        None,
        "a pattern that does not compile is version-unknown, never a probe failure"
    );
    assert_eq!(
        extract_version("banner line\nv22.19.0\n", r"^v(\d+\.\d+\.\d+)$").as_deref(),
        Some("22.19.0"),
        "the first line that matches answers, not the first line"
    );
}

#[test]
fn below_min_is_semver_and_tolerant() {
    assert!(below_min("18.20.0", "22.0.0"));
    assert!(!below_min("22.19.0", "22.0.0"));
    assert!(
        !below_min("weird", "22.0.0"),
        "an unparsable version is tolerated, not failed (ANA-4:762)"
    );
    assert!(!below_min("1.0.0", "also-weird"));
}

#[tokio::test]
async fn capture_version_runs_the_tool_and_times_out_a_hung_one() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());

    // Every test box runs `cargo`, and cargo names its own binary for the test process.
    let probe = VersionProbe {
        args: vec!["--version".to_owned()],
        pattern: r"^cargo (\d+\.\d+\.\d+)".to_owned(),
        min: None,
    };
    let version = capture_version(Path::new(env!("CARGO")), &probe, &env).await;
    assert!(
        version.is_some(),
        "`cargo --version` matches its own pattern: {version:?}"
    );

    #[cfg(unix)]
    {
        let hang = tmp.path().join("bin/hang");
        executable(&hang, "#!/bin/sh\nsleep 30\n");
        let env = ProbeEnv {
            version_timeout: Duration::from_millis(300),
            ..env
        };
        let probe = VersionProbe {
            args: Vec::new(),
            pattern: r"(\d+)".to_owned(),
            min: None,
        };
        let started = std::time::Instant::now();
        assert_eq!(capture_version(&hang, &probe, &env).await, None);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the timeout kills the child rather than waiting it out: {:?}",
            started.elapsed()
        );
    }
}

/// Review gate HIGH-2: the `--version` child must not outlive an **aborted** probe task.
///
/// `AgentRuntime::shutdown` and the background limit both `abort()` the probe task, and tier 1
/// spends most of its wall time inside `capture_version`; the shape is
/// `a_dropped_handshake_future_still_kills_the_child`'s, one tier down.
#[cfg(unix)]
#[tokio::test]
async fn a_dropped_version_capture_still_kills_the_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    // Run the fixture *through* `/bin/sh` rather than as an executable of its own: `capture_version`
    // spawns internally, so it cannot go through `spawn_fixture`'s `ETXTBSY` retry, and a script an
    // interpreter only ever *reads* is never "text file busy".
    let script = tmp.path().join("hang.sh");
    std::fs::write(&script, "echo $$ > version.pid\nsleep 1000\n").expect("write");
    let probe = VersionProbe {
        args: vec![script.to_string_lossy().into_owned()],
        pattern: r"(\d+)".to_owned(),
        min: None,
    };
    let pidfile = env.cwd.join("version.pid");

    let task = tokio::spawn(async move {
        let _ = capture_version(Path::new("/bin/sh"), &probe, &env).await;
    });
    let pid = await_pid(&pidfile).await;
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_not_running(pid, "an aborted version capture");
}

/// Review gate MEDIUM-1: a tool that streams on `--version` is bounded by the cap, not by
/// `version_timeout` × throughput, and the child that would not stop is killed.
#[cfg(unix)]
#[tokio::test]
async fn read_stdout_to_end_caps_a_tool_that_streams_forever() {
    let tmp = tempfile::tempdir().expect("temp box");
    let launch = ResolvedLaunch {
        // `sh -c`, so there is no fixture file to be `ETXTBSY` on.
        command: "/bin/sh".to_owned(),
        args: vec![
            "-c".to_owned(),
            "while :; do echo streaming-forever-and-ever; done".to_owned(),
        ],
        env: BTreeMap::new(),
    };
    let mut spawned = spawn_fixture(&launch, tmp.path())
        .await
        .expect("the fixture command starts");
    let pid = spawned.pid().expect("a live child has a pid");

    let stdout = tokio::time::timeout(Duration::from_secs(10), spawned.read_stdout_to_end())
        .await
        .expect("the read is bounded by a cap, not by the child's lifetime")
        .expect("a capped read is an answer, not an error");
    assert!(
        stdout.len() <= 64 * 1024,
        "the read stops at the cap: {} bytes",
        stdout.len()
    );
    assert!(
        stdout.contains("streaming-forever-and-ever"),
        "what it did read is the child's output"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_not_running(pid, "a tool that streamed past the cap");
}

// ---------------------------------------------------------------------------------------------
// 12–13. `probe_tools`: the checked override, and the version floor
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_checked_override_pointing_nowhere_is_missing() {
    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    env.vars
        .insert("HTUI_TOOL_NODE".to_owned(), "/nonexistent/node".to_string());
    let discovery = discovery("node", path_probe(&["node"], None));

    let report = probe_tools(Some(&discovery), &env)
        .await
        .expect("the tiers ran");
    assert_eq!(
        report.missing,
        vec!["node".to_owned()],
        "a probe that recorded a nonexistent override as ready would lie"
    );
    assert!(report.found.is_empty());

    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    env.vars.insert(
        "HTUI_TOOL_NODE".to_owned(),
        tool.to_string_lossy().into_owned(),
    );
    let report = probe_tools(Some(&discovery), &env)
        .await
        .expect("the tiers ran");
    assert_eq!(
        report.found.get("node").map(|found| found.path.clone()),
        Some(tool),
        "an override that exists is the answer, whatever the tiers would have said"
    );
    assert!(report.missing.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_tool_below_its_floor_is_found_and_missing() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    executable(&tmp.path().join("bin/node"), "#!/bin/sh\necho v18.20.0\n");
    let discovery = discovery(
        "node",
        path_probe(
            &["node"],
            Some(VersionProbe {
                args: vec!["--version".to_owned()],
                pattern: r"^v(\d+\.\d+\.\d+)$".to_owned(),
                min: Some("22.0.0".to_owned()),
            }),
        ),
    );

    let report = probe_tools(Some(&discovery), &env)
        .await
        .expect("the tiers ran");
    assert!(
        report.found["node"].below_min,
        "node 18 is below the row's floor"
    );
    assert_eq!(
        report.versions().get("node"),
        Some(&Some("18.20.0".to_owned())),
        "the version it does have is still recorded"
    );
    assert_eq!(
        report.missing,
        vec!["node".to_owned()],
        "and it is counted missing, so nothing is spawned"
    );
    assert!(!report.is_complete());
}

/// A path resolution is the map `launch::resolve` substitutes from, and `PathBuf` is not it.
#[tokio::test]
async fn a_tool_map_is_built_from_what_resolved() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");

    let report = probe_tools(
        Some(&discovery("tool", path_probe(&["htui-fake-tool"], None))),
        &env,
    )
    .await
    .expect("the tiers ran");
    assert!(report.is_complete());
    assert_eq!(
        report.tool_map().get("tool").map(String::as_str),
        Some(tool.to_string_lossy().as_ref())
    );
    assert_eq!(report.versions().get("tool"), Some(&None));
}

/// A row with no `discovery` has nothing to probe, and that is a complete report — the same
/// answer `tools::resolve` gives for the same row.
#[tokio::test]
async fn a_row_without_discovery_reports_nothing_missing() {
    let tmp = tempfile::tempdir().expect("temp box");
    let report = probe_tools(None, &env(tmp.path()))
        .await
        .expect("nothing to do");
    assert!(report.found.is_empty());
    assert!(report.is_complete());
    assert_eq!(report, Default::default());
}

/// `tools::resolve` and the `ChatStart` re-probe both want resolution without the `--version`
/// children, and they get it by value rather than by remembering to pass a flag.
#[test]
fn a_probe_env_can_drop_its_version_tier() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path()).without_versions();
    assert!(!env.versions);
    assert_eq!(
        env.var("LOCALAPPDATA"),
        Some(tmp.path().join("lad").to_string_lossy().as_ref())
    );
    assert_eq!(env.var("NOPE"), None);
}

/// Review gate MEDIUM-2: `ProbeEnv::host` snapshots the **whole** process environment, so one
/// `?env` in a log line would print every secret on the box (`R-SEC-2`) — the footgun
/// `ResolvedLaunch` got `RedactedEnv` for. Nothing logs it today; the `Debug` closes it anyway.
#[test]
fn a_probe_env_debug_does_not_print_the_environment() {
    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    env.vars.insert(
        "ANTHROPIC_API_KEY".to_owned(),
        "sk-ant-planted-secret".to_owned(),
    );

    let rendered = format!("{env:?}");
    assert!(
        !rendered.contains("sk-ant-planted-secret"),
        "the environment's contents never reach a log line: {rendered}"
    );
    assert!(
        rendered.contains("linux-x86_64"),
        "the fields that are not the environment still render: {rendered}"
    );

    let rendered = format!("{:?}", context(env));
    assert!(
        !rendered.contains("sk-ant-planted-secret"),
        "`ProbeContext` inherits the redaction, because it inherits the env: {rendered}"
    );
}

// ---------------------------------------------------------------------------------------------
// T25 fixtures: a scripted `initialize`, and the two `Tier2` seams
// ---------------------------------------------------------------------------------------------

/// The buffer each half of the in-process pipe gets, as `acp_conformance.rs` sizes it.
const DUPLEX_BYTES: usize = 64 * 1024;

/// Line 1 of the recorded `claude` transcript: the `initialize` **result** the real adapter sent.
fn fixture_initialize_result() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_acp_handshake.jsonl");
    let text = std::fs::read_to_string(path).expect("the recorded handshake fixture");
    let line = text.lines().next().expect("the fixture has a first line");
    let document: Value = serde_json::from_str(line).expect("the first line is JSON");
    document
        .get("result")
        .cloned()
        .expect("line 1 records the initialize result")
}

/// An agent that answers exactly one request — `initialize` — with `result`, then reads until the
/// client goes away. Raw newline-delimited JSON-RPC, importing no SDK type, for the reason
/// `acp_conformance.rs` gives: two ends sharing a library prove the library, not the protocol.
async fn scripted_initialize(stream: tokio::io::DuplexStream, result: Value) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let Ok(Some(line)) = lines.next_line().await else {
        return;
    };
    let request: Value = serde_json::from_str(&line).expect("the client speaks JSON-RPC");
    let response = json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "result": result,
    });
    let mut text = serde_json::to_string(&response).expect("the response serialises");
    text.push('\n');
    if writer.write_all(text.as_bytes()).await.is_err() {
        return;
    }
    let _ = writer.flush().await;
    while let Ok(Some(_)) = lines.next_line().await {}
}

/// What a canned tier 2 answers.
enum Canned {
    /// [`DriverError::Transport`] with this text.
    Failed(String),
    /// Nothing: being called at all is the failure (plan D50's "nothing is spawned").
    Never,
}

/// A tier 2 that answers without a process — the seam that lets a status-mapping case assert
/// **that nothing was spawned** rather than hope so.
struct CannedTier2(Canned);

impl Tier2 for CannedTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        _settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            match &self.0 {
                Canned::Failed(text) => Err(DriverError::Transport(text.clone())),
                Canned::Never => panic!("tier 2 ran for a row that resolved nothing"),
            }
        })
    }
}

/// A tier 2 that runs the **real** `acp::handshake` over a duplex whose far end answers `result`:
/// everything but the spawn.
struct DuplexTier2(Value);

impl Tier2 for DuplexTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
            let (reader, writer) = tokio::io::split(client_end);
            tokio::spawn(scripted_initialize(agent_end, self.0.clone()));
            handshake(
                AcpIo {
                    reader: Box::new(reader),
                    writer: Box::new(writer),
                    child: None,
                },
                settings,
                Duration::from_secs(5),
            )
            .await
        })
    }
}

/// The seeded `claude` row.
fn claude_row() -> Agent {
    htui_core::model::agent::seed_rows(Utc::now())
        .into_iter()
        .find(|row| row.name == "claude")
        .expect("the seed rows carry `claude`")
}

/// A registry row with a literal command and no `discovery`: resolution is trivially complete, so
/// a case can be about tier 2 and nothing else.
fn synthetic_row(name: &str, command: &str, transport: Transport) -> Agent {
    let now = Utc::now();
    Agent {
        id: AgentId::new(),
        name: name.to_owned(),
        transport,
        launch: json!({ "command": command, "args": [], "env": {} }),
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({}),
        created_at: now,
        updated_at: now,
    }
}

fn context(env: ProbeEnv) -> ProbeContext {
    ProbeContext {
        env,
        now: Utc::now(),
    }
}

/// The row [`ProbeOutcome::Row`] carries, or a panic naming what came back instead.
fn row_of(outcome: ProbeOutcome) -> AgentBox {
    match outcome {
        ProbeOutcome::Row(row) => row,
        ProbeOutcome::Kept { reason } => {
            panic!("expected a written row, got `Kept {{ {reason} }}`")
        }
    }
}

/// `false` once the process is gone **or** reaped-pending: a killed child that nobody has waited
/// for is a zombie, which is dead by every measure this assertion is making.
///
/// Linux-only because `/proc` is: the pid is the one identifier a kill can be checked against that
/// nothing else can accidentally answer to (`launch.rs`'s `pid()` doc), and a `pgrep` pattern
/// matches the shell that ran the check (`acp_live.rs`'s warning).
#[cfg(unix)]
fn assert_not_running(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return;
        };
        // `pid (comm) state …`, and `comm` may hold spaces and parens: the state is the first
        // field after the **last** `)`.
        let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
        let state = after.trim().chars().next().unwrap_or('Z');
        assert!(
            state == 'Z' || state == 'X',
            "{what}: pid {pid} is still running (state {state})"
        );
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

// ---------------------------------------------------------------------------------------------
// 1–2. `acp::handshake` reports what `initialize` answered
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn handshake_completes_initialize_and_reports_the_response() {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(scripted_initialize(agent_end, fixture_initialize_result()));

    let found = handshake(
        AcpIo {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
        },
        &AcpSettings::default(),
        Duration::from_secs(5),
    )
    .await
    .expect("the scripted agent answers initialize");

    assert_eq!(found.protocol_version, 1);
    assert_eq!(
        found.agent_name.as_deref(),
        Some("@agentclientprotocol/claude-agent-acp")
    );
    assert_eq!(found.agent_version.as_deref(), Some("0.48.0"));
    assert_eq!(
        found.capabilities.get("loadSession"),
        Some(&json!(true)),
        "the SDK's own serialisation is recorded verbatim: {}",
        found.capabilities
    );
    assert!(
        found.auth_methods.is_empty(),
        "the claude adapter demands no auth"
    );
    assert_eq!(status_for(&found), ProbeStatus::Ready);
}

#[tokio::test]
async fn handshake_reports_auth_methods_by_id() {
    let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
    let (reader, writer) = tokio::io::split(client_end);
    tokio::spawn(scripted_initialize(
        agent_end,
        json!({
            "protocolVersion": 1,
            "agentCapabilities": {},
            "authMethods": [
                { "id": "oauth-personal", "name": "Log in with Google" },
                { "id": "gemini-api-key", "name": "Use a Gemini API key" },
            ],
        }),
    ));

    let found = handshake(
        AcpIo {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
        },
        &AcpSettings::default(),
        Duration::from_secs(5),
    )
    .await
    .expect("the scripted agent answers initialize");

    assert_eq!(
        found.auth_methods,
        vec!["oauth-personal".to_owned(), "gemini-api-key".to_owned()],
        "the ids, in the order the agent listed them"
    );
    assert_eq!(
        status_for(&found),
        ProbeStatus::Unauthenticated,
        "an agent that offers a login is not ready to run (ANA-4 §11 criterion 10)"
    );
}

// ---------------------------------------------------------------------------------------------
// 3–6. The child dies on every exit path (the milestone-3 CRITICAL, `682a423`)
// ---------------------------------------------------------------------------------------------

/// An agent that answers `initialize` and then stays alive, so the kill is the test's doing and
/// not the process's own exit.
///
/// The JSON-RPC id is a **string** (a UUID) on this SDK, so it is echoed back with its quotes
/// rather than parsed as a number.
#[cfg(unix)]
const ANSWERING_AGENT: &str = r#"#!/bin/sh
read -r line
id=$(printf '%s' "$line" | grep -o '"id":"[^"]*"' | head -n 1 | sed 's/^"id"://')
printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[]}}\n' "$id"
sleep 1000
"#;

/// The same agent, refusing `initialize` — and writing a stderr line first, which the error is
/// required to carry (the `handshake_error` shape). The `not-json` line is there to prove a frame
/// the client cannot parse does not derail the handshake.
#[cfg(unix)]
const REFUSING_AGENT: &str = r#"#!/bin/sh
echo "boot noise on stderr" >&2
read -r line
id=$(printf '%s' "$line" | grep -o '"id":"[^"]*"' | head -n 1 | sed 's/^"id"://')
printf 'not-json\n'
printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32601,"message":"initialize is not implemented"}}\n' "$id"
sleep 1000
"#;

/// Spawns `command` under supervision and hands back its pid and its streams.
#[cfg(unix)]
async fn spawned_io(cwd: &Path, command: &str, args: &[&str]) -> (u32, AcpIo) {
    let launch = ResolvedLaunch {
        command: command.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: BTreeMap::new(),
    };
    let spawned = spawn_fixture(&launch, cwd)
        .await
        .expect("the fixture command starts");
    let pid = spawned.pid().expect("a live child has a pid");
    (pid, AcpIo::from_spawned(spawned).expect("stdio is piped"))
}

#[cfg(unix)]
#[tokio::test]
async fn handshake_times_out_and_kills_its_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (pid, io) = spawned_io(tmp.path(), "sleep", &["1000"]).await;

    let error = handshake(io, &AcpSettings::default(), Duration::from_millis(300))
        .await
        .expect_err("a silent agent never answers initialize");
    assert!(
        error.to_string().contains("within 300ms"),
        "the timeout says how long it waited, at the resolution it was given — `as_secs()` \
         rendered this one as `within 0s`: {error}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_not_running(pid, "a timed-out handshake");
}

#[cfg(unix)]
#[tokio::test]
async fn handshake_kills_its_child_when_the_agent_refuses_initialize() {
    let tmp = tempfile::tempdir().expect("temp box");
    let script = tmp.path().join("bin/agent");
    executable(&script, REFUSING_AGENT);
    let (pid, io) = spawned_io(tmp.path(), &script.to_string_lossy(), &[]).await;

    let error = handshake(io, &AcpSettings::default(), Duration::from_secs(10))
        .await
        .expect_err("an agent that refuses initialize fails the handshake");
    let text = error.to_string();
    assert!(
        text.contains("initialize failed:"),
        "the error names the step that failed: {text}"
    );
    assert!(
        text.contains("initialize is not implemented"),
        "the agent's own reason is carried through: {text}"
    );
    assert!(
        text.contains("boot noise"),
        "the child's stderr tail is appended after the reason: {text}"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_not_running(pid, "a failed handshake");
}

#[cfg(unix)]
#[tokio::test]
async fn handshake_kills_its_child_on_success() {
    let tmp = tempfile::tempdir().expect("temp box");
    let script = tmp.path().join("bin/agent");
    executable(&script, ANSWERING_AGENT);
    let (pid, io) = spawned_io(tmp.path(), &script.to_string_lossy(), &[]).await;

    let found = handshake(io, &AcpSettings::default(), Duration::from_secs(10))
        .await
        .expect("the script answers initialize");
    assert_eq!(found.protocol_version, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_not_running(pid, "a successful handshake");
}

#[cfg(unix)]
#[tokio::test]
async fn a_dropped_handshake_future_still_kills_the_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (pid, io) = spawned_io(tmp.path(), "sleep", &["1000"]).await;

    let settings = AcpSettings::default();
    let task = tokio::spawn(async move {
        let _ = handshake(io, &settings, Duration::from_secs(60)).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    task.abort();
    let _ = task.await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_not_running(pid, "an aborted probe task");
}

// ---------------------------------------------------------------------------------------------
// 7–13. `probe_agent`: the status mapping (D50), the manual rule (D51), the snapshot (D45)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn probe_agent_is_missing_and_spawns_nothing_when_a_tool_is_absent() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));

    let row = row_of(
        probe_agent(
            &claude_row(),
            BoxId::new(),
            None,
            &ctx,
            &CannedTier2(Canned::Never),
        )
        .await,
    );

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("missing"));
    assert!(!row.enabled, "a box that cannot run it is not enabled");
    assert_eq!(row.path, None);
    assert!(probe["resolved"].is_null(), "nothing resolved: {probe}");
    assert!(probe["handshake"].is_null(), "nothing spawned: {probe}");
    assert_eq!(probe["source"], json!("probe"));
    assert_eq!(row.probed_at, Some(ctx.now));
    assert_eq!(row.updated_at, ctx.now);
}

#[tokio::test]
async fn probe_agent_is_ready_on_an_empty_auth_list_and_unauthenticated_otherwise() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = synthetic_row("claude", &tool.to_string_lossy(), Transport::Acp);

    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &DuplexTier2(fixture_initialize_result()),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("ready"));
    assert!(row.enabled);
    assert_eq!(
        row.version.as_deref(),
        Some("0.48.0"),
        "the handshake's own version is what the row records (ANA-4 §4.6)"
    );
    assert_eq!(row.path.as_deref(), Some(tool.to_string_lossy().as_ref()));
    assert_eq!(probe["handshake"]["protocol_version"], json!(1));

    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &DuplexTier2(json!({
                "protocolVersion": 1,
                "agentCapabilities": {},
                "authMethods": [{ "id": "oauth-personal", "name": "Log in" }],
            })),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("unauthenticated"));
    assert!(
        !row.enabled,
        "the vendor's own auth flow is not htui's to drive (ANA-4 §4.6)"
    );
}

#[tokio::test]
async fn probe_agent_is_failed_with_the_error_text_when_tier_2_errs() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = synthetic_row("claude", &tool.to_string_lossy(), Transport::Acp);

    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &CannedTier2(Canned::Failed(
                "initialize failed: boom\nline 1\nline 2".to_owned(),
            )),
        )
        .await,
    );

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("failed"));
    assert!(!row.enabled);
    assert!(
        probe["resolved"]["command"] == json!(tool.to_string_lossy().as_ref()),
        "it resolved; it did not answer: {probe}"
    );
    assert_eq!(row.path.as_deref(), Some(tool.to_string_lossy().as_ref()));
    let tail: Vec<String> = serde_json::from_value(probe["stderr_tail"].clone())
        .expect("a failed probe carries its lines");
    assert_eq!(
        tail,
        vec![
            // Literal, not `format!("{}", DriverError::Transport(..))`: an expectation built from
            // the implementation's own `Display` can never catch the wrapper leaking back in.
            "initialize failed: boom".to_owned(),
            "line 1".to_owned(),
            "line 2".to_owned(),
        ],
        "the failure text as the child wrote it, line by line, with no error-enum prefix"
    );
}

#[tokio::test]
async fn a_cli_row_stops_at_tier_1() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = synthetic_row("agy", &tool.to_string_lossy(), Transport::Cli);

    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &CannedTier2(Canned::Never),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(
        probe["status"],
        json!("ready"),
        "a cli row has no `initialize` to complete: resolution is the whole probe"
    );
    assert!(probe["handshake"].is_null());
    assert_eq!(probe["transport"], json!("cli"));
    assert!(row.enabled);
}

#[tokio::test]
async fn a_manual_entry_survives_a_probe_that_finds_nothing_and_is_refreshed_by_one_that_does() {
    let tmp = tempfile::tempdir().expect("temp box");
    let agent = claude_row();
    let box_id = BoxId::new();
    let old = Utc::now() - TimeDelta::days(3);
    let existing = AgentBox {
        agent_id: agent.id,
        box_id,
        enabled: true,
        version: Some("hand-written".to_owned()),
        path: Some("/opt/claude".to_owned()),
        probed_at: Some(old),
        quota: None,
        quota_at: None,
        updated_at: old,
        probe: Some(json!({
            "transport": "acp",
            "resolved": null,
            "tools": {},
            "handshake": null,
            "status": "ready",
            "stderr_tail": null,
            "source": "manual",
        })),
    };

    let ctx = context(env(tmp.path()));
    match probe_agent(
        &agent,
        box_id,
        Some(&existing),
        &ctx,
        &CannedTier2(Canned::Never),
    )
    .await
    {
        ProbeOutcome::Kept { reason } => assert!(
            reason.contains("manual"),
            "the log line says why nothing was written: {reason}"
        ),
        ProbeOutcome::Row(row) => panic!("a manual entry was overwritten: {row:?}"),
    }

    // ANA-4:795-798's rule is "a probe that finds **nothing**", not "a probe whose status is
    // `missing`". An `agent.launch` that does not parse is `failed` with `resolved: null` — a
    // probe that learned nothing about this box either, and one that must not turn a hand-written
    // row into `enabled = false, source: probe`. (`probe_tools` answering with a transport fault is
    // the other route to the same arm: `failed`, `resolved: null`, nothing learned.)
    let unparsable = Agent {
        launch: json!("this is not a launch document"),
        ..claude_row()
    };
    match probe_agent(
        &unparsable,
        box_id,
        Some(&existing),
        &ctx,
        &CannedTier2(Canned::Never),
    )
    .await
    {
        ProbeOutcome::Kept { reason } => assert!(
            reason.contains("manual"),
            "the log line says why nothing was written: {reason}"
        ),
        ProbeOutcome::Row(row) => {
            panic!("a manual entry was overwritten by a probe that resolved nothing: {row:?}")
        }
    }

    // The same row, on a box where the probe *does* find something, is refreshed like any other.
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = synthetic_row("claude", &tool.to_string_lossy(), Transport::Acp);
    let row = row_of(
        probe_agent(
            &agent,
            box_id,
            Some(&existing),
            &ctx,
            &DuplexTier2(fixture_initialize_result()),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("ready"));
    assert_eq!(
        probe["source"],
        json!("probe"),
        "a refreshed row records who refreshed it"
    );
    assert_eq!(row.probed_at, Some(ctx.now));

    // And a `failed` that *did* resolve refreshes too: the probe found the binary on this box, it
    // just did not get an answer out of it. That is a fact about the box, not a probe that learned
    // nothing, so the manual row does not survive it.
    let row = row_of(
        probe_agent(
            &agent,
            box_id,
            Some(&existing),
            &ctx,
            &CannedTier2(Canned::Failed("initialize failed: boom".to_owned())),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["status"], json!("failed"));
    assert_eq!(
        probe["resolved"]["command"],
        json!(tool.to_string_lossy().as_ref()),
        "it resolved, so the probe did learn something: {probe}"
    );
}

#[tokio::test]
async fn a_probe_carries_quota_over_and_orders_keys_as_ana4_does() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = synthetic_row("claude", &tool.to_string_lossy(), Transport::Acp);
    let box_id = BoxId::new();
    let quota_at = Utc::now() - TimeDelta::hours(2);
    let existing = AgentBox {
        agent_id: agent.id,
        box_id,
        enabled: false,
        version: None,
        path: None,
        probed_at: None,
        quota: Some(json!({ "remaining": 1 })),
        quota_at: Some(quota_at),
        updated_at: quota_at,
        probe: None,
    };

    let row = row_of(
        probe_agent(
            &agent,
            box_id,
            Some(&existing),
            &ctx,
            &DuplexTier2(fixture_initialize_result()),
        )
        .await,
    );
    assert_eq!(
        row.quota,
        Some(json!({ "remaining": 1 })),
        "the probe owns neither quota field (MOD-7 writes them)"
    );
    assert_eq!(row.quota_at, Some(quota_at));

    let snapshot = ProbeSnapshot::from_row(&row).expect("the snapshot parses back");
    let text = serde_json::to_string(&snapshot).expect("the snapshot serialises");
    assert!(
        text.starts_with("{\"transport\":"),
        "field order is key order: {text}"
    );
    let mut at = 0;
    for key in [
        "\"transport\":",
        "\"resolved\":",
        "\"tools\":",
        "\"handshake\":",
        "\"status\":",
        "\"stderr_tail\":",
        "\"source\":",
    ] {
        let found = text[at..]
            .find(key)
            .unwrap_or_else(|| panic!("`{key}` is missing or out of order in {text}"));
        at += found + key.len();
    }
}

#[tokio::test]
async fn a_snapshot_round_trips_and_defaults_source_to_probe() {
    let agent = claude_row();
    let row = AgentBox {
        agent_id: agent.id,
        box_id: BoxId::new(),
        enabled: true,
        version: Some("0.48.0".to_owned()),
        path: Some("/usr/bin/node".to_owned()),
        probed_at: Some(Utc::now()),
        quota: None,
        quota_at: None,
        updated_at: Utc::now(),
        // No `source` key: a row nobody marked `manual` may be refreshed.
        probe: Some(json!({
            "transport": "acp",
            "resolved": { "command": "/usr/bin/node", "args": [], "env": {} },
            "tools": { "node": "22.19.0", "claude": null },
            "handshake": null,
            "status": "ready",
            "stderr_tail": null,
        })),
    };

    let snapshot = ProbeSnapshot::from_row(&row).expect("the snapshot parses");
    assert_eq!(snapshot.source, ProbeSource::Probe);
    assert_eq!(snapshot.status, ProbeStatus::Ready);
    assert_eq!(snapshot.transport, Transport::Acp);
    assert_eq!(
        snapshot.tools.get("node"),
        Some(&Some("22.19.0".to_owned()))
    );
    assert_eq!(snapshot.tools.get("claude"), Some(&None));
    assert_eq!(
        snapshot.resolved.as_ref().map(|r| r.command.as_str()),
        Some("/usr/bin/node")
    );

    let garbage = AgentBox {
        probe: Some(json!("nonsense")),
        ..row.clone()
    };
    assert_eq!(
        ProbeSnapshot::from_row(&garbage),
        None,
        "an unparseable column must not stop the Settings tab listing the row"
    );
    assert_eq!(
        ProbeSnapshot::from_row(&AgentBox { probe: None, ..row }),
        None
    );
}
