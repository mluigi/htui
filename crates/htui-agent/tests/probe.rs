//! The box probe, tier 1 and tier 2 (`docs/ANA-4.md` §4.6, plan MOD-2 D46–D51, T24 and T25).
//!
//! Every case here runs against a throwaway directory tree and an **injected** [`ProbeEnv`]: the
//! workspace forbids `unsafe`, so `std::env::set_var` is not available and the environment a tier
//! reads is a value, not a global. Nothing in this file probes an unmodified seed *row* end to end
//! — this box has `node`, `claude` and the ACP adapter installed, and a probe of the seeded
//! `claude` row would spawn the real adapter inside `cargo test` (blueprint H-7); the live probe
//! lives in `tests/probe_live.rs`, `#[ignore]`d.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{TimeDelta, Utc};
use htui_agent::acp::{AcpIo, Handshake, handshake};
use htui_agent::driver::DriverFuture;
use htui_agent::error::DriverError;
use htui_agent::launch::{
    AcpSettings, AgentLaunch, CredentialProbe, Discovery, ResolvedLaunch, ToolProbe, VersionProbe,
};
use htui_agent::probe::{
    CredentialTier, GlobMatch, INSTALL_ROOT_VAR, ProbeContext, ProbeEnv, ProbeOutcome,
    ProbeSnapshot, ProbeSource, ProbeStatus, Tier2, below_min, capture_version,
    default_install_root, expand, extract_version, glob_first, install_root, platform_key,
    probe_agent, probe_tools, resolve_credential, resolve_tool, segment_matches, status_for, walk,
};
use htui_core::model::{Agent, AgentBox, AgentId, Billing, BoxId, Transport};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A box made of directories under `tmp`: `cwd`, `bin` (the injected `PATH`), `home` (what `~`
/// expands to), `lad` (what `%LOCALAPPDATA%` expands to) and `agents` (what
/// `%HTUI_AGENTS_ROOT%` expands to).
///
/// The install root is injected here for the reason every other variable is: `ProbeEnv::host`
/// would seed it from the *maintainer's* `dirs::data_local_dir()`, and `std::env::set_var` is
/// `unsafe` and forbidden, so the only way a seed-row case can stay off the real disk is to hand
/// the tier a value (plan MOD-20 D15).
fn env(tmp: &Path) -> ProbeEnv {
    for dir in ["cwd", "bin", "home", "lad", "agents"] {
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
    vars.insert(
        INSTALL_ROOT_VAR.to_owned(),
        tmp.join("agents").to_string_lossy().into_owned(),
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
        credential: None,
        install: None,
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

/// The shape `htui` never installs into, kept exactly as it shipped: a JetBrains-managed copy is
/// laid out `<ide>/acp-agents/antigravity-acp/<build>/`, and a build number (`20260501`) is not a
/// semver. So both captures here fail `Version::parse`, both assertions stay on D14's fallback,
/// and this case now pins that fallback — the mtime-then-path-descending rule of MOD-2 D48 —
/// rather than the whole of `newest()`.
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
        .join("agents/antigravity-acp/1.1.1/agy_acp_server.par");
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
    let server = tmp
        .path()
        .join("agents/antigravity-acp/1.1.1/agy_acp_server.par");
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

/// The `agy` seed row's own patterns for one platform key.
fn agy_patterns(platform: &str) -> Vec<String> {
    match agy_discovery()
        .tools
        .remove("agy_acp_server")
        .expect("the seed declares the adapter tool")
    {
        ToolProbe::Glob { platform: map, .. } => map
            .get(platform)
            .unwrap_or_else(|| panic!("the seed declares patterns for {platform}"))
            .patterns
            .clone(),
        other => panic!("the adapter tool is a glob probe, not {other:?}"),
    }
}

/// The Windows half of D15's token, split where the platform splits it.
///
/// Neither half can be run whole on this box: [`ProbeEnv::var`] folds case only under
/// `cfg!(windows)`, and a `C:\…` value names no directory a Linux `read_dir` can open. So the
/// expansion of a backslashed value — pure, and the reason `%HTUI_AGENTS_ROOT%` can replace a
/// hand-written `%LOCALAPPDATA%/…` at all — is asserted everywhere, and the resolve is driven with
/// the spelling the host can actually find: a deliberately mis-cased key on Windows, where the
/// environment block really does carry one, and the canonical key elsewhere.
#[tokio::test]
async fn the_windows_seed_pattern_expands_the_root_token_case_insensitively() {
    let tmp = tempfile::tempdir().expect("temp box");
    let base = env(tmp.path());
    let patterns = agy_patterns("windows-x86_64");
    let managed = patterns
        .iter()
        .find(|pattern| pattern.contains(INSTALL_ROOT_VAR))
        .expect("the windows-x86_64 entry reaches the install root through the token");

    // 1. A Windows environment block's value: separators are backslashes, and `expand` must push
    //    the whole thing as one segment rather than splitting it into six.
    let backslashed = r"C:\Users\x\AppData\Local\htui\agents";
    let mut vars = base.vars.clone();
    vars.insert(INSTALL_ROOT_VAR.to_owned(), backslashed.to_owned());
    let windows_box = ProbeEnv {
        platform: "windows-x86_64".to_owned(),
        vars,
        ..base.clone()
    };
    let (root, rest) = expand(managed, &windows_box).expect("the token expands");
    assert_eq!(
        root,
        PathBuf::from(backslashed).join("antigravity-acp"),
        "a backslashed value stays one segment, so the literal root ends at the registry id"
    );
    assert_eq!(rest, vec!["*".to_owned(), "agy_acp_server.exe".to_owned()]);

    // 2. The resolve, through the shipped row, with the key spelled the way this host looks it up.
    let root = tmp.path().join("agents");
    let mut vars = base.vars.clone();
    vars.remove(INSTALL_ROOT_VAR);
    let mis_cased = if cfg!(windows) {
        "HtUi_AgEnTs_RoOt"
    } else {
        INSTALL_ROOT_VAR
    };
    vars.insert(mis_cased.to_owned(), root.to_string_lossy().into_owned());
    let env = ProbeEnv {
        platform: "windows-x86_64".to_owned(),
        vars,
        ..base
    };
    let server = root.join("antigravity-acp/1.1.1/agy_acp_server.exe");
    executable(&server, "binary");

    let report = probe_tools(Some(&agy_discovery()), &env)
        .await
        .expect("the tiers ran");
    let found = report
        .found
        .get("agy_acp_server")
        .expect("the windows-x86_64 patterns found the server");
    assert_eq!(
        found.path, server,
        "the JetBrains pattern is tried first and matches nothing; the token pattern answers"
    );
}

/// D15's whole point: with the root behind a token there is nothing left in the document to
/// disagree with the installer about, on any platform. This greps the shipped files rather than
/// the parsed rows so a root re-introduced under *any* tool, in a comment or in a pattern, is
/// caught by the same test that pins the three spellings the seeds used to carry.
#[test]
fn no_seed_pattern_spells_an_install_root() {
    let seeds = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory is one level above this crate")
        .join("htui-core/seeds");
    let mut read = 0usize;

    for name in ["agent_agy.json", "agent_claude.json"] {
        let path = seeds.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} is readable: {err}", path.display()));
        read += 1;
        for spelling in [
            ".local/share/htui",
            "Application Support/htui",
            "LOCALAPPDATA%/htui",
        ] {
            assert!(
                !text.contains(spelling),
                "{name} spells an install root (`{spelling}`); the root is \
                 `%{INSTALL_ROOT_VAR}%`'s to say, and a document that repeats it is the copy \
                 that goes stale"
            );
        }
    }
    assert_eq!(read, 2, "both shipped documents were read, not one");
}

/// The other end of the injection: production has no one to inject for it, so [`ProbeEnv::host`]
/// seeds the token itself — and only when the box has not already said where its adapters live.
///
/// The "unless" half cannot be driven over `host`: writing the process environment needs
/// `std::env::set_var`, which is `unsafe` and forbidden here. It is read, never written, and the
/// override is proven one layer down, over [`install_root`] on a hand-built env — which is the
/// same lookup `host`'s own seeding consults before it inserts anything.
#[test]
fn host_seeds_the_root_token_from_the_helper_unless_the_environment_sets_it() {
    let host = ProbeEnv::host(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    match std::env::var_os(INSTALL_ROOT_VAR) {
        None => {
            assert_eq!(
                host.vars.get(INSTALL_ROOT_VAR).map(PathBuf::from),
                default_install_root(),
                "an environment that says nothing gets the helper's answer, under the exact key"
            );
            assert_eq!(install_root(&host), default_install_root());
        }
        Some(set) => assert_eq!(
            install_root(&host),
            Some(PathBuf::from(set)),
            "this box sets the variable, so the variable wins"
        ),
    }

    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    let elsewhere = tmp.path().join("a-bigger-disk/agents");
    env.vars.insert(
        INSTALL_ROOT_VAR.to_owned(),
        elsewhere.to_string_lossy().into_owned(),
    );
    assert_eq!(install_root(&env), Some(elsewhere.clone()));
    assert_ne!(
        install_root(&env),
        default_install_root(),
        "a box that names a root is never answered from `dirs::data_local_dir()`"
    );
}

#[test]
fn install_root_is_none_when_the_token_is_absent() {
    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    env.vars.remove(INSTALL_ROOT_VAR);

    assert_eq!(
        install_root(&env),
        None,
        "there is no fallback to `default_install_root()` here: a hand-built env that did not \
         inject the token must not be answered from the maintainer's real disk"
    );
    assert_eq!(
        expand(&agy_patterns("linux-x86_64")[0], &env),
        None,
        "and the seed pattern that names it is skipped, not resolved somewhere else"
    );
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
// 8b. Version-aware selection (plan MOD-20 D14, amending MOD-2 D48)
// ---------------------------------------------------------------------------------------------

/// The install root's shape, one version directory at a time: `<root>/<id>/<version>/<file>`, so
/// the pattern below has exactly one `*` and the version is the capture `newest()` keys on.
fn versioned(tmp: &Path, version: &str) -> PathBuf {
    let file = tmp
        .join("agents/acp-adapter")
        .join(version)
        .join("adapter-server");
    executable(&file, "binary");
    file
}

/// What the row's glob answers over the tree [`versioned`] built.
async fn resolve_versioned(env: &ProbeEnv) -> PathBuf {
    glob_first(
        &["%HTUI_AGENTS_ROOT%/acp-adapter/*/adapter-server".to_owned()],
        env,
    )
    .await
    .expect("the walk ran")
    .expect("the tree holds at least one install")
}

/// D4's first failure, made a test: unpacking preserves the vendor's build date, so the version
/// installed *last* routinely carries the *older* mtime. Keying on the mtime picked the version
/// the user had just replaced.
#[tokio::test]
async fn a_semver_directory_beats_a_newer_mtime() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let higher = versioned(tmp.path(), "1.10.0");
    let lower = versioned(tmp.path(), "1.9.0");

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    touch_at(&higher, epoch);
    touch_at(&lower, epoch + Duration::from_secs(3_600));

    assert_eq!(
        resolve_versioned(&env).await,
        higher,
        "the version in the directory name decides; the mtime is only what breaks a tie between \
         captures that are not versions at all"
    );
}

/// D4's second failure: with the mtimes equal the old rule fell through to the path, descending.
#[tokio::test]
async fn semver_beats_path_order_where_the_old_rule_was_wrong() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let higher = versioned(tmp.path(), "1.10.0");
    let lower = versioned(tmp.path(), "1.9.0");

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    touch_at(&higher, epoch);
    touch_at(&lower, epoch);

    assert_eq!(
        resolve_versioned(&env).await,
        higher,
        "`1.10.0` outranks `1.9.0` as a version. The descending-path tie-break this replaces \
         answered `1.9.0`, because `9` sorts above `1` lexicographically — the trap D4 names"
    );
}

/// `semver`'s own rule, stated here so it is a property of `newest()` and not of a dependency:
/// `2.0.0-rc.1` is below `2.0.0`, and no mtime lifts it back above.
#[tokio::test]
async fn a_prerelease_sorts_below_its_release() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let release = versioned(tmp.path(), "2.0.0");
    let prerelease = versioned(tmp.path(), "2.0.0-rc.1");

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    touch_at(&release, epoch);
    touch_at(&prerelease, epoch + Duration::from_secs(3_600));

    assert_eq!(
        resolve_versioned(&env).await,
        release,
        "a prerelease is below its release however recently it was written"
    );
}

/// D14's `None < Some`, driven end to end: a directory named like a version *is* an install, and
/// a sibling that is not named like one loses to it whatever its timestamp says.
#[tokio::test]
async fn a_version_directory_beats_a_non_version_sibling() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let version = versioned(tmp.path(), "1.1.1");
    let rolling = versioned(tmp.path(), "current");

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    touch_at(&version, epoch);
    touch_at(&rolling, epoch + Duration::from_secs(3_600));

    assert_eq!(
        resolve_versioned(&env).await,
        version,
        "a capture that parses as a version outranks every capture that does not, and `current` \
         does not — being newer only decides among the ones that do not"
    );
}

/// Registries and release tags spell the same version both ways; the resolver reads one thing.
///
/// The mtimes are what make this a test of the `v` and not of the path order: the tagged directory
/// is the *older* file, so it can only win by parsing — an untolerated `v` would leave it
/// unparsable, and an unparsable capture loses to a numbered sibling however old it is.
#[tokio::test]
async fn a_leading_v_is_tolerated() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    let tagged = versioned(tmp.path(), "v1.10.0");
    let plain = versioned(tmp.path(), "1.9.0");

    let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    touch_at(&tagged, epoch);
    touch_at(&plain, epoch + Duration::from_secs(3_600));

    assert_eq!(
        resolve_versioned(&env).await,
        tagged,
        "the leading `v` is stripped before parsing, so `v1.10.0` is the version 1.10.0 and not an \
         unparsable sibling that loses to every numbered one"
    );
}

/// The capture list is what `newest()` keys on, so its shape is asserted directly: one entry per
/// `*` segment, in pattern order, and nothing at all for a pattern that has none.
#[test]
fn walk_reports_one_capture_per_star() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    for (ide, build) in [("ide-a", "20260501"), ("ide-b", "20260818")] {
        executable(
            &tmp.path()
                .join("home/ides")
                .join(ide)
                .join("builds")
                .join(build)
                .join("adapter-server"),
            "binary",
        );
    }

    let (root, segments) = expand("~/ides/*/builds/*/adapter-server", &env).expect("`~` expands");
    let mut found = walk(&root, &segments);
    found.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(
        found,
        vec![
            GlobMatch {
                path: tmp
                    .path()
                    .join("home/ides/ide-a/builds/20260501/adapter-server"),
                captures: vec!["ide-a".to_owned(), "20260501".to_owned()],
            },
            GlobMatch {
                path: tmp
                    .path()
                    .join("home/ides/ide-b/builds/20260818/adapter-server"),
                captures: vec!["ide-b".to_owned(), "20260818".to_owned()],
            },
        ],
        "two `*` segments, two captures each, in the order the pattern spells them — the last one \
         is the version segment `newest()` reads"
    );

    let (root, segments) =
        expand("~/ides/ide-a/builds/20260501/adapter-server", &env).expect("`~` expands");
    let literal = walk(&root, &segments);
    assert_eq!(
        literal.len(),
        1,
        "a pattern with no `*` is one `stat`, and it still finds its file"
    );
    assert!(
        literal[0].captures.is_empty(),
        "a pattern with no `*` captures nothing, which is what sends it to D14's fallback"
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
    assert_eq!(status_for(&found, None), ProbeStatus::Ready);
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
        status_for(&found, None),
        ProbeStatus::Unauthenticated,
        "an agent that offers a login is not ready to run (ANA-4 §11 criterion 10), and a row \
         that declares no credential block is judged by that rule alone"
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

/// MOD-2 plan D74: the projection does not carry `quota` / `quota_at` forward from the stored row.
///
/// It used to, and that was the lost update D74 removes: the row was read at chat start and handed
/// back seconds later to a statement whose `SET` list wrote both columns, so every latch that
/// landed in between was discarded. Since `upsert_agent_box` can no longer write them at all,
/// there is nothing to carry them *to* - `set_agent_box_quota` is the only writer, and the values
/// it wrote stay in the row the probe is about to update.
///
/// Every other projected field is asserted here too: what changed is two fields, not the
/// projection.
#[tokio::test]
async fn agent_box_row_does_not_carry_quota_forward() {
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
        row.quota, None,
        "the probe owns neither quota field, and since D74 it does not carry them either"
    );
    assert_eq!(row.quota_at, None, "nor their timestamp");

    // The rest of the projection is unchanged by D74.
    assert_eq!(row.agent_id, agent.id);
    assert_eq!(row.box_id, box_id);
    assert!(row.enabled, "a ready handshake enables the box");
    assert_eq!(
        row.version.as_deref(),
        Some("0.48.0"),
        "`version` is the recorded handshake's `agent_version`, and it never came from `existing`"
    );
    assert_eq!(
        row.path.as_deref(),
        Some(tool.to_string_lossy().as_ref()),
        "`path` is the resolved command"
    );
    assert_eq!(row.probed_at, Some(ctx.now));
    assert_eq!(row.updated_at, ctx.now);
    assert!(row.probe.is_some(), "the §4.6 snapshot rides the row");
}

#[tokio::test]
async fn a_probe_snapshot_orders_keys_as_ana4_does() {
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
        // D59's key sits between the handshake that raised the question and the status that
        // answers it: a reader in `psql` sees "four auth methods, a token file, ready" in order.
        "\"credential\":",
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
    assert_eq!(
        snapshot.credential, None,
        "a milestone-5 document has no `credential` key, and its absence reads as the rule that \
         row was written under: the block was never declared"
    );
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

// ---------------------------------------------------------------------------------------------
// 14–19. The credential tier (plan D59/D63, T32)
// ---------------------------------------------------------------------------------------------

/// The token text the fixture file holds. Asserted **absent** from the serialised snapshot: the
/// file tier is a `stat` and the probe has no reason to ever open the file.
const FIXTURE_TOKEN: &str = "secret-9f8e";

/// A credential block no seed row declares, in the expander grammar the glob tier already speaks:
/// a `%VAR%` candidate whose variable this box does not set, a `~` candidate, and one variable.
///
/// Synthetic on purpose (blueprint H-18, and `R-AGT-5`): the rule under test is "a row that
/// declares candidates is judged by them", and a case built on `agy`'s own block would prove the
/// seed instead. Case 6 below is the one that reads the seed, and it reads it as *data*.
fn fake_credential() -> CredentialProbe {
    CredentialProbe {
        env: vec!["HTUI_FAKE_KEY".to_owned()],
        files: vec![
            "%HTUI_CRED_HOME%/token.json".to_owned(),
            "~/.htui-fake/token.json".to_owned(),
        ],
    }
}

/// A [`Handshake`] that demands `ids` — the tier-2 answer [`status_for`] maps. Built by hand rather
/// than driven over a duplex: the mapping is a pure function and this file already proves the
/// duplex path twice.
fn demanding(ids: &[&str]) -> Handshake {
    Handshake {
        at: Utc::now(),
        protocol_version: 1,
        agent_name: None,
        agent_version: None,
        capabilities: json!({}),
        auth_methods: ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// The `initialize` result of an agent that advertises a login whether or not one has happened —
/// the Antigravity shape ANA-4 §4.5 records, which is the whole reason D59 exists.
fn four_methods() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": {},
        "authMethods": [
            { "id": "oauth-personal", "name": "Log in with Google" },
            { "id": "oauth-business", "name": "Log in with a workspace account" },
            { "id": "gemini-api-key", "name": "Use a Gemini API key" },
            { "id": "agent-platform", "name": "Agent platform" },
        ],
    })
}

/// A row whose `discovery` carries [`fake_credential`] and `tools`, with a literal command.
fn credential_row(command: &str, tools: Value) -> Agent {
    Agent {
        launch: json!({
            "command": command,
            "args": [],
            "env": {},
            "discovery": {
                "tools": tools,
                "handshake": true,
                "credential": {
                    "env": ["HTUI_FAKE_KEY"],
                    "files": ["%HTUI_CRED_HOME%/token.json", "~/.htui-fake/token.json"],
                },
            },
        }),
        ..synthetic_row("probed", command, Transport::Acp)
    }
}

/// Writes the fixture token at `path`. Its *contents* exist so a later assertion can prove the
/// probe never read them.
fn write_token(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, FIXTURE_TOKEN).expect("write");
}

#[tokio::test]
async fn no_candidate_is_absent_and_a_demanding_handshake_stays_unauthenticated() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());

    assert_eq!(
        resolve_credential(Some(&fake_credential()), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::Absent),
        "the row declared candidates and none of them answered — which is a different fact from \
         `None`, where the row declared nothing at all"
    );
    assert_eq!(
        status_for(
            &demanding(&["oauth-personal"]),
            Some(CredentialTier::Absent)
        ),
        ProbeStatus::Unauthenticated
    );
}

#[tokio::test]
async fn a_declared_file_makes_it_ready_and_the_home_candidate_answers_when_the_var_is_unset() {
    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    let probe = fake_credential();
    let home_token = tmp.path().join("home/.htui-fake/token.json");
    write_token(&home_token);

    // `%HTUI_CRED_HOME%` is unset, so `expand` skips that candidate and the walk never sees it —
    // the same rule that lets one row carry a Windows `%LOCALAPPDATA%` pattern and a unix `~` one.
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::File)
    );
    assert_eq!(
        status_for(
            &demanding(&["oauth-personal", "gemini-api-key"]),
            Some(CredentialTier::File)
        ),
        ProbeStatus::Ready,
        "the box holds what the agent's own login flow leaves; the advertised methods are an \
         offer, not a demand (D59)"
    );

    let cred_home = tmp.path().join("cred");
    write_token(&cred_home.join("token.json"));
    env.vars.insert(
        "HTUI_CRED_HOME".to_owned(),
        cred_home.to_string_lossy().into_owned(),
    );
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::File),
        "both candidates exist; the answer is still `file` and the order is the row's"
    );

    std::fs::remove_file(&home_token).expect("remove the home candidate");
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::File),
        "the first pattern answers on its own once the second has nothing"
    );
}

#[tokio::test]
async fn a_declared_variable_counts_only_when_set_and_non_empty() {
    let tmp = tempfile::tempdir().expect("temp box");
    let mut env = env(tmp.path());
    let probe = fake_credential();

    env.vars.insert("HTUI_FAKE_KEY".to_owned(), String::new());
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::Absent),
        "an exported-but-empty variable is how a shell profile unsets a key; it is not a credential"
    );

    env.vars.insert("HTUI_FAKE_KEY".to_owned(), "x".to_owned());
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::Env)
    );
    assert_eq!(
        status_for(&demanding(&["gemini-api-key"]), Some(CredentialTier::Env)),
        ProbeStatus::Ready
    );

    write_token(&tmp.path().join("home/.htui-fake/token.json"));
    assert_eq!(
        resolve_credential(Some(&probe), &env)
            .await
            .expect("the walk ran"),
        Some(CredentialTier::File),
        "files are checked first (D63): on a subscription box the token file is the tier that fires"
    );
}

#[tokio::test]
async fn a_row_without_a_credential_block_keeps_the_milestone_5_rule() {
    let tmp = tempfile::tempdir().expect("temp box");
    let env = env(tmp.path());
    // Even with a credential *on the box*, a row that never declared one cannot see it.
    write_token(&tmp.path().join("home/.htui-fake/token.json"));

    assert_eq!(
        resolve_credential(None, &env).await.expect("nothing to do"),
        None
    );
    assert_eq!(
        status_for(&demanding(&["oauth-personal"]), None),
        ProbeStatus::Unauthenticated,
        "milestone 5's rule verbatim: a non-empty `authMethods` is `unauthenticated`, full stop"
    );
    assert_eq!(status_for(&demanding(&[]), None), ProbeStatus::Ready);
    assert_eq!(
        status_for(&demanding(&[]), Some(CredentialTier::Absent)),
        ProbeStatus::Ready,
        "an agent that demands nothing is ready whatever the credential tier says: the block \
         explains an auth list, it does not add a requirement"
    );
}

#[tokio::test]
async fn probe_agent_records_the_tier_and_never_the_value() {
    let tmp = tempfile::tempdir().expect("temp box");
    let ctx = context(env(tmp.path()));
    let tool = tmp.path().join("bin").join(tool_name("htui-fake-tool"));
    executable(&tool, "#!/bin/sh\nexit 0\n");
    let agent = credential_row(&tool.to_string_lossy(), json!({}));
    let token = tmp.path().join("home/.htui-fake/token.json");
    write_token(&token);

    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &DuplexTier2(four_methods()),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["credential"], json!("file"));
    assert_eq!(
        probe["status"],
        json!("ready"),
        "four advertised methods and a token on disk is a box that can run the agent: {probe}"
    );
    assert!(row.enabled, "and the orchestrator will not skip it");
    assert_eq!(
        probe["handshake"]["auth_methods"].as_array().map(Vec::len),
        Some(4)
    );

    // The whole point of recording a *tier*: the column is rendered verbatim by the Settings tab
    // and read by hand in `psql`, so neither the secret nor the path to it may reach it.
    let text = serde_json::to_string(&probe).expect("the snapshot serialises");
    assert!(
        !text.contains(FIXTURE_TOKEN),
        "the token's own text reached the snapshot: {text}"
    );
    assert!(
        !text.contains("token.json"),
        "the path of the credential reached the snapshot: {text}"
    );

    std::fs::remove_file(&token).expect("remove the token");
    let row = row_of(
        probe_agent(
            &agent,
            BoxId::new(),
            None,
            &ctx,
            &DuplexTier2(four_methods()),
        )
        .await,
    );
    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    assert_eq!(probe["credential"], json!("absent"));
    assert_eq!(probe["status"], json!("unauthenticated"));
    assert!(!row.enabled);

    // Step 2 placement: the credential is resolved as soon as `agent.launch` parses, so a box that
    // never got as far as spawning anything still says whether it holds a token.
    write_token(&token);
    let absent_tool = credential_row(
        &tool.to_string_lossy(),
        json!({ "nope": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] } }),
    );
    let row = row_of(
        probe_agent(
            &absent_tool,
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
    assert_eq!(
        probe["credential"],
        json!("file"),
        "a `missing` box still reports its credential state: {probe}"
    );
}

/// The seed row is **data**: `R-AGT-5` says a new agent costs one registry row and at most one
/// adapter, so this reads `agent_agy.json`'s own block the way `probe_live.rs` reads `claude`'s.
/// Nothing in production branches on the name.
#[test]
fn the_seeded_agy_row_declares_the_token_candidates_and_the_api_key_variable() {
    let credential = agy_discovery()
        .credential
        .expect("the agy seed declares a credential block");
    assert_eq!(
        credential,
        CredentialProbe {
            env: vec!["GEMINI_API_KEY".to_owned()],
            files: vec![
                "%GEMINI_HOME%/antigravity-acp/acp_token.json".to_owned(),
                "~/.gemini/antigravity-acp/acp_token.json".to_owned(),
            ],
        },
        "the token file `agy_acp_server`'s own login writes (ANA-4 §4.5), then the API key"
    );

    let claude: AgentLaunch = serde_json::from_value(claude_row().launch).expect("it parses");
    assert_eq!(
        claude
            .discovery
            .expect("claude declares a discovery block")
            .credential,
        None,
        "the adapter that demands no auth declares no block, and keeps milestone 5's rule"
    );
}

// ---------------------------------------------------------------------------------------------
// 20. `recorded_launch`: D58's three row-side rules (T31)
// ---------------------------------------------------------------------------------------------

/// A snapshot carrying only what D58 reads. Everything else is what a probe that got no further
/// would have left, so a case here cannot pass for a reason it did not name.
fn snapshot(
    source: ProbeSource,
    status: ProbeStatus,
    resolved: Option<ResolvedLaunch>,
) -> ProbeSnapshot {
    ProbeSnapshot {
        transport: Transport::Acp,
        resolved,
        tools: BTreeMap::new(),
        handshake: None,
        credential: None,
        status,
        stderr_tail: None,
        source,
    }
}

/// A launch whose `args` no resolution can reproduce: `tools::resolve` yields one string per tool
/// and `launch::resolve` substitutes it, so a per-platform argument exists in a snapshot and
/// nowhere else (blueprint H-3).
fn recorded() -> ResolvedLaunch {
    ResolvedLaunch {
        command: "/opt/antigravity/agy_acp_server.par".to_owned(),
        args: vec!["--uid=".to_owned()],
        env: BTreeMap::new(),
    }
}

/// The pure half of D58: which snapshots may be spawned as recorded, decided from the document
/// alone. The fourth rule — the command still exists — is I/O and belongs to
/// `AcpDriver::launch_for`, which is why this case can be a `#[test]` and stay one.
#[test]
fn recorded_launch_applies_d58s_three_row_side_rules() {
    let ready = snapshot(ProbeSource::Probe, ProbeStatus::Ready, Some(recorded()));
    assert_eq!(
        ready.recorded_launch(),
        Some(&recorded()),
        "a probed, ready row is exactly the case D58 exists for"
    );

    let unauthenticated = snapshot(
        ProbeSource::Probe,
        ProbeStatus::Unauthenticated,
        Some(recorded()),
    );
    assert_eq!(
        unauthenticated.recorded_launch(),
        Some(&recorded()),
        "an unauthenticated box recorded the right launch; spawning it and letting the vendor say \
         `log in first` is more use than resolving a second time (blueprint H-4)"
    );

    let missing = snapshot(ProbeSource::Probe, ProbeStatus::Missing, None);
    assert_eq!(
        missing.recorded_launch(),
        None,
        "a `missing` probe resolved nothing, so there is nothing to spawn"
    );

    let failed = snapshot(ProbeSource::Probe, ProbeStatus::Failed, Some(recorded()));
    assert_eq!(
        failed.recorded_launch(),
        None,
        "a `failed` probe recorded a launch that did not answer; resolving again is the honest \
         second attempt"
    );

    let manual = snapshot(ProbeSource::Manual, ProbeStatus::Ready, Some(recorded()));
    assert_eq!(
        manual.recorded_launch(),
        None,
        "a `manual` row is a human's path, and `launch::resolve` already honours it through the \
         row itself (blueprint H-5)"
    );

    let empty = snapshot(ProbeSource::Probe, ProbeStatus::Ready, None);
    assert_eq!(
        empty.recorded_launch(),
        None,
        "`ready` with nothing resolved is not a launch, whatever the status says"
    );
}
