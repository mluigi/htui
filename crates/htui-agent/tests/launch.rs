//! `agent.launch` / `agent.settings` deserialisation, `${tool}` resolution and the supervised
//! spawn (plan MOD-2 T7, `docs/ANA-4.md` §5.1, §5.2, §4.6).
//!
//! The two launch documents below are the ANA-4 §5.3 seed rows, held here as literals. Task 8
//! lands them as `htui_core::model::agent::seed_rows`, and task 9's `tests/extensibility.rs`
//! asserts that *those* rows deserialise into the same types — this file pins the shape before
//! either exists, which is what makes the seed a test subject rather than a source of truth.
//!
//! Since MOD-21 (plan D14) the file also carries the stderr tap's cases at the bottom: a login flow
//! needs the child's stderr *as it happens*, and what the tap must not cost is the bounded tail two
//! review gates hardened. Those cases are `cfg(unix)` per item, as `tests/acp_driver.rs` gates its
//! own, because their fixture is `/bin/sh`.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::time::{Duration, Instant};

use htui_agent::DriverError;
use htui_agent::launch::{
    AgentLaunch, AgentSettings, Discovery, Install, InstallSource, QuotaSource, ToolMap, ToolProbe,
    resolve,
};
#[cfg(unix)]
use htui_agent::launch::{ResolvedLaunch, Spawned, StopSignal};
#[cfg(unix)]
use tokio::sync::mpsc::UnboundedReceiver;

/// `claude`'s launch row (ANA-4 §5.3), byte for byte.
const CLAUDE_LAUNCH: &str = r#"{
  "command": "${node}",
  "args": ["${claude_agent_acp}"],
  "env": { "CLAUDE_CODE_EXECUTABLE": "${claude}" },
  "discovery": {
    "tools": {
      "node":   { "kind": "path", "names": ["node"],
                  "version": { "args": ["--version"], "pattern": "^v(\\d+\\.\\d+\\.\\d+)$",
                               "min": "22.0.0" } },
      "claude": { "kind": "path", "names": ["claude"],
                  "version": { "args": ["--version"],
                               "pattern": "^(\\d+\\.\\d+\\.\\d+) \\(Claude Code\\)$" } },
      "npx":    { "kind": "path", "names": ["npx"],
                  "version": { "args": ["--version"], "pattern": "^(\\d+\\.\\d+\\.\\d+)$" } },
      "claude_agent_acp": { "kind": "node_package",
                            "package": "@agentclientprotocol/claude-agent-acp",
                            "entry": "dist/index.js",
                            "pinned": "0.75.0",
                            "fallback": { "command": "${npx}",
                                          "args": ["-y",
                                            "@agentclientprotocol/claude-agent-acp@0.75.0"] } }
    },
    "handshake": true
  }
}"#;

/// `agy`'s launch row (ANA-4 §5.3), byte for byte — the `glob` probe with per-platform `args`.
const AGY_LAUNCH: &str = r#"{
  "command": "${agy_acp_server}",
  "args": [],
  "env": {},
  "discovery": {
    "tools": {
      "agy": { "kind": "path", "names": ["agy"],
               "version": { "args": ["--version"], "pattern": "^v?(\\d+\\.\\d+\\.\\d+)" } },
      "agy_acp_server": {
        "kind": "glob",
        "patterns": [],
        "platform": {
          "windows-x86_64": { "patterns": [
            "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe",
            "%LOCALAPPDATA%/htui/agents/antigravity-acp/*/agy_acp_server.exe" ] },
          "windows-aarch64": { "patterns": [
            "%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe" ] },
          "linux-x86_64":   { "patterns": ["~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"],
                              "args": ["--uid="] },
          "linux-aarch64":  { "patterns": ["~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"],
                              "args": ["--uid="] },
          "darwin-aarch64": { "patterns": ["~/Library/Application Support/htui/agents/antigravity-acp/*/agy_acp_server.par"] }
        }
      }
    },
    "handshake": true,
    "credential": {
      "env":   ["GEMINI_API_KEY"],
      "files": ["%GEMINI_HOME%/antigravity-acp/acp_token.json",
                "~/.gemini/antigravity-acp/acp_token.json"]
    }
  }
}"#;

/// `claude`'s settings row (ANA-4 §5.3): the only seed carrying a `cli` block.
const CLAUDE_SETTINGS: &str = r#"{
  "acp": { "protocol_version": 1,
           "client_capabilities": { "fs_read": true, "fs_write": true, "terminal": false,
                                    "elicitation": false },
           "model_config_id": null,
           "session": { "load": true, "resume": true } },
  "cli": { "stream": "claude_stream_json", "permission_mode": "acceptEdits",
           "extra_args": ["--bare"] },
  "permission": { "default": "ask", "rules": [], "remembered": [] },
  "quota": { "source": "acp_meta_rate_limit" },
  "usage": { "scope": "model_usage" }
}"#;

fn tool_map(pairs: &[(&str, &str)]) -> ToolMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn claude_launch_round_trips() {
    let launch: AgentLaunch = serde_json::from_str(CLAUDE_LAUNCH).expect("claude launch parses");

    assert_eq!(launch.command, "${node}");
    assert_eq!(launch.args, ["${claude_agent_acp}"]);
    assert_eq!(
        launch.env.get("CLAUDE_CODE_EXECUTABLE").map(String::as_str),
        Some("${claude}")
    );

    let discovery = launch
        .discovery
        .as_ref()
        .expect("claude declares discovery");
    assert!(discovery.handshake);
    assert_eq!(discovery.tools.len(), 4);

    match discovery.tools.get("node").expect("node probe") {
        ToolProbe::Path { names, version } => {
            assert_eq!(names, &["node"]);
            let version = version.as_ref().expect("node has a version probe");
            assert_eq!(version.args, ["--version"]);
            assert_eq!(version.pattern, r"^v(\d+\.\d+\.\d+)$");
            assert_eq!(version.min.as_deref(), Some("22.0.0"));
        }
        other => panic!("node is a path probe, got {other:?}"),
    }

    match discovery
        .tools
        .get("claude_agent_acp")
        .expect("claude_agent_acp probe")
    {
        ToolProbe::NodePackage {
            package,
            entry,
            pinned,
            fallback,
        } => {
            assert_eq!(package, "@agentclientprotocol/claude-agent-acp");
            assert_eq!(entry, "dist/index.js");
            assert_eq!(pinned, "0.75.0");
            let fallback = fallback.as_ref().expect("a fallback is declared");
            assert_eq!(fallback.command, "${npx}");
            assert_eq!(
                fallback.args,
                ["-y", "@agentclientprotocol/claude-agent-acp@0.75.0"]
            );
        }
        other => panic!("claude_agent_acp is a node_package probe, got {other:?}"),
    }

    assert_eq!(
        discovery.credential, None,
        "the claude adapter demands no auth, so its row declares no credential block and keeps \
         milestone 5's rule (plan D59)"
    );

    // Round trip: the value we serialise back is the value we parsed, field for field.
    let text = serde_json::to_string(&launch).expect("launch serialises");
    assert!(
        !text.contains("credential"),
        "a row without the block re-serialises without a `\"credential\": null` key, so a probe \
         does not rewrite every milestone-5 registry row it touches: {text}"
    );
    let reparsed: AgentLaunch = serde_json::from_str(&text).expect("serialised launch parses");
    assert_eq!(reparsed, launch);
}

#[test]
fn agy_launch_round_trips_with_per_platform_args() {
    let launch: AgentLaunch = serde_json::from_str(AGY_LAUNCH).expect("agy launch parses");

    assert_eq!(launch.command, "${agy_acp_server}");
    assert!(launch.args.is_empty());
    assert!(launch.env.is_empty());

    let discovery = launch.discovery.as_ref().expect("agy declares discovery");
    match discovery
        .tools
        .get("agy_acp_server")
        .expect("agy_acp_server probe")
    {
        ToolProbe::Glob { patterns, platform } => {
            assert!(patterns.is_empty(), "the top-level pattern list is empty");
            assert_eq!(platform.len(), 5);

            let linux = platform.get("linux-x86_64").expect("linux-x86_64 entry");
            assert_eq!(
                linux.patterns,
                ["~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"]
            );
            // The Linux-only `--uid=` is the reason a glob probe carries per-platform args at all
            // (ANA-4 §5.1): one registry row, two platforms, different argv.
            assert_eq!(linux.args, ["--uid="]);

            let windows = platform
                .get("windows-x86_64")
                .expect("windows-x86_64 entry");
            assert_eq!(windows.patterns.len(), 2);
            assert!(
                windows.args.is_empty(),
                "windows takes no extra args, so the key is absent and the default is empty"
            );
        }
        other => panic!("agy_acp_server is a glob probe, got {other:?}"),
    }

    // Plan D59/D63: `agy`'s ACP server advertises four auth methods whether or not this box has
    // logged in, so the row says where its own login leaves a credential. Two file candidates in
    // the glob tier's grammar — a `%VAR%` one for a box that sets `GEMINI_HOME`, a `~` one for the
    // default — then the API-key variable.
    let credential = discovery
        .credential
        .as_ref()
        .expect("agy declares a credential block");
    assert_eq!(credential.env, ["GEMINI_API_KEY"]);
    assert_eq!(credential.files.len(), 2);
    assert_eq!(
        credential.files[0],
        "%GEMINI_HOME%/antigravity-acp/acp_token.json"
    );
    assert_eq!(
        credential.files[1],
        "~/.gemini/antigravity-acp/acp_token.json"
    );

    let reparsed: AgentLaunch =
        serde_json::from_str(&serde_json::to_string(&launch).expect("launch serialises"))
            .expect("serialised launch parses");
    assert_eq!(reparsed, launch);
}

/// A discovery block that declares where its adapter comes from (plan MOD-20 D12), anonymous on
/// purpose: what is pinned here is the shape, and the row that carries a real registry id is
/// `tests/extensibility.rs`'s subject.
const INSTALLABLE_DISCOVERY: &str = r#"{
  "tools": { "y": { "kind": "glob", "patterns": ["/opt/y/*/y"] } },
  "handshake": true,
  "install": { "source": "acp_registry", "id": "x", "tool": "y" }
}"#;

#[test]
fn a_declared_install_source_round_trips() {
    let discovery: Discovery =
        serde_json::from_str(INSTALLABLE_DISCOVERY).expect("the discovery block parses");

    assert_eq!(
        discovery.install,
        Some(Install {
            source: InstallSource::AcpRegistry,
            id: "x".to_owned(),
            tool: "y".to_owned(),
        })
    );

    let text = serde_json::to_string(&discovery).expect("discovery serialises");
    assert!(
        text.contains(r#""source":"acp_registry""#),
        "the source is a closed vocabulary on the wire, not a free string a `match` would have to \
         guess at: {text}"
    );

    let reparsed: Discovery = serde_json::from_str(&text).expect("serialised discovery parses");
    assert_eq!(reparsed, discovery);
}

#[test]
fn a_row_without_an_install_block_re_serialises_unchanged() {
    let launch: AgentLaunch = serde_json::from_str(CLAUDE_LAUNCH).expect("claude launch parses");
    let discovery = launch
        .discovery
        .as_ref()
        .expect("claude declares discovery");

    assert_eq!(
        discovery.install, None,
        "the claude adapter is served by a package manager of its own, so its row declares no \
         source and there is nothing for the app to install (plan MOD-20 D12, PRD \"Not for\")"
    );

    // The `credential` rule of milestone 6, one key later: a document written before MOD-20 comes
    // back out of this type byte for byte.
    let text = serde_json::to_string(&launch).expect("launch serialises");
    assert!(
        !text.contains("install"),
        "a row without the block re-serialises without an `\"install\": null` key, so an install \
         does not rewrite every registry row it merely read: {text}"
    );
    let reparsed: AgentLaunch = serde_json::from_str(&text).expect("serialised launch parses");
    assert_eq!(reparsed, launch);
}

#[test]
fn an_unknown_install_source_names_the_value_it_refused() {
    const FROM_A_LATER_HTUI: &str = r#"{
      "tools": {},
      "install": { "source": "github_release", "id": "x", "tool": "y" }
    }"#;

    let error = serde_json::from_str::<Discovery>(FROM_A_LATER_HTUI)
        .expect_err("a second source is a variant of this crate, not a string a row may invent");

    let message = error.to_string();
    assert!(
        message.contains("github_release"),
        "the refusal has to name the value: a row written against a newer htui otherwise reads as \
         a parse failure with no clue which key is at fault: {message}"
    );
}

#[test]
fn empty_settings_document_yields_every_documented_default() {
    let settings: AgentSettings = serde_json::from_str("{}").expect("`{}` is a valid settings row");

    assert_eq!(settings.acp.protocol_version, 1);
    assert!(settings.acp.client_capabilities.fs_read);
    assert!(settings.acp.client_capabilities.fs_write);
    assert!(!settings.acp.client_capabilities.terminal);
    assert!(!settings.acp.client_capabilities.elicitation);
    assert_eq!(settings.acp.model_config_id, None);
    assert!(settings.acp.session.load);
    assert!(settings.acp.session.resume);

    assert!(
        settings.cli.is_none(),
        "no CLI block unless the row has one"
    );
    assert_eq!(settings.permission, htui_agent::PermissionPolicy::default());
    assert_eq!(settings.quota.source, QuotaSource::None);
    assert_eq!(settings, AgentSettings::default());
}

#[test]
fn claude_settings_round_trip() {
    let settings: AgentSettings =
        serde_json::from_str(CLAUDE_SETTINGS).expect("claude settings parse");

    let cli = settings.cli.as_ref().expect("claude carries a cli block");
    assert_eq!(cli.stream, "claude_stream_json");
    assert_eq!(cli.permission_mode, "acceptEdits");
    assert_eq!(cli.extra_args, ["--bare"]);
    assert_eq!(settings.quota.source, QuotaSource::AcpMetaRateLimit);

    let reparsed: AgentSettings =
        serde_json::from_str(&serde_json::to_string(&settings).expect("settings serialise"))
            .expect("serialised settings parse");
    assert_eq!(reparsed, settings);
}

#[test]
fn resolve_substitutes_command_args_and_env() {
    let launch: AgentLaunch = serde_json::from_str(CLAUDE_LAUNCH).expect("claude launch parses");
    let map = tool_map(&[
        ("node", "/usr/bin/node"),
        ("claude_agent_acp", "/opt/claude-agent-acp/dist/index.js"),
        ("claude", "/usr/local/bin/claude"),
    ]);

    let resolved = resolve(&launch, &map).expect("every placeholder has an entry");

    assert_eq!(resolved.command, "/usr/bin/node");
    assert_eq!(resolved.args, ["/opt/claude-agent-acp/dist/index.js"]);
    assert_eq!(
        resolved
            .env
            .get("CLAUDE_CODE_EXECUTABLE")
            .map(String::as_str),
        Some("/usr/local/bin/claude"),
        "env *values* are substituted too, not only the command line"
    );
}

#[test]
fn resolve_reports_the_missing_placeholder_by_name() {
    let launch: AgentLaunch = serde_json::from_str(CLAUDE_LAUNCH).expect("claude launch parses");
    let map = tool_map(&[
        ("claude_agent_acp", "/opt/claude-agent-acp/dist/index.js"),
        ("claude", "/usr/local/bin/claude"),
    ]);

    match resolve(&launch, &map) {
        Err(DriverError::Unresolved(name)) => assert_eq!(name, "node"),
        other => panic!("a missing tool is `Unresolved`, got {other:?}"),
    }
}

#[test]
fn a_row_without_placeholders_resolves_against_an_empty_map() {
    let launch = AgentLaunch {
        command: "agy".to_owned(),
        args: vec!["--serve".to_owned()],
        env: BTreeMap::from([("AGY_HOME".to_owned(), "/var/lib/agy".to_owned())]),
        discovery: None,
    };

    let resolved = resolve(&launch, &ToolMap::new()).expect("nothing to resolve");

    assert_eq!(resolved.command, "agy");
    assert_eq!(resolved.args, ["--serve"]);
    assert_eq!(
        resolved.env.get("AGY_HOME").map(String::as_str),
        Some("/var/lib/agy")
    );
}

#[test]
fn debug_never_prints_an_environment_value() {
    let launch = AgentLaunch {
        command: "agy".to_owned(),
        args: Vec::new(),
        env: BTreeMap::from([("AGY_TOKEN".to_owned(), "s3cr3t-value".to_owned())]),
        discovery: None,
    };
    let resolved = resolve(&launch, &ToolMap::new()).expect("nothing to resolve");

    for rendered in [format!("{launch:?}"), format!("{resolved:?}")] {
        assert!(
            !rendered.contains("s3cr3t-value"),
            "an environment value reached Debug: {rendered}"
        );
        assert!(
            rendered.contains("AGY_TOKEN"),
            "the key stays visible so a log still says which variables were set: {rendered}"
        );
        assert!(rendered.contains("[REDACTED]"), "{rendered}");
    }
}

#[test]
fn to_acp_config_carries_command_args_and_env() {
    let launch: AgentLaunch = serde_json::from_str(CLAUDE_LAUNCH).expect("claude launch parses");
    let map = tool_map(&[
        ("node", "/usr/bin/node"),
        ("claude_agent_acp", "/opt/claude-agent-acp/dist/index.js"),
        ("claude", "/usr/local/bin/claude"),
    ]);
    let resolved = resolve(&launch, &map).expect("resolves");

    let config = resolved.to_acp_config();

    assert_eq!(config.command(), std::path::Path::new("/usr/bin/node"));
    assert_eq!(config.arguments(), ["/opt/claude-agent-acp/dist/index.js"]);
    assert_eq!(
        config.environment().get("CLAUDE_CODE_EXECUTABLE"),
        Some(&"/usr/local/bin/claude".to_owned())
    );

    // The SDK type's own serialisation is `{command, args, env}` — the shape ANA-4 §5.1 says a
    // launch row is field-for-field. X6: we reach it through the builder because its fields are
    // private, not because the shape differs.
    let json = serde_json::to_value(&config).expect("config serialises");
    let object = json.as_object().expect("an object");
    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        ["command", "args", "env"]
    );
}

#[tokio::test]
async fn spawn_runs_the_resolved_command_under_supervision() {
    let cargo = std::env::var("CARGO").expect("cargo sets $CARGO for its test children");
    let launch = AgentLaunch {
        command: "${cargo}".to_owned(),
        args: vec!["--version".to_owned()],
        env: BTreeMap::new(),
        discovery: None,
    };
    let resolved = resolve(&launch, &tool_map(&[("cargo", &cargo)])).expect("resolves");

    let mut spawned =
        htui_agent::launch::spawn(&resolved, std::env::current_dir().unwrap().as_path())
            .await
            .expect("cargo --version spawns");

    let stdout = spawned.read_stdout_to_end().await.expect("stdout reads");
    let status = spawned.wait().await.expect("the child exits");

    assert!(
        status.success(),
        "`cargo --version` exits 0, got {status:?}"
    );
    assert!(
        stdout.starts_with("cargo "),
        "stdout starts with the program name, got {stdout:?}"
    );
    assert!(
        spawned.stderr_tail().is_empty(),
        "a successful `cargo --version` writes nothing to stderr, got {:?}",
        spawned.stderr_tail()
    );

    // `job_object` is a Windows fact. On unix the child is a process-group leader instead, and the
    // field is false by construction — asserting `true` here would assert the platform, not the
    // supervision (plan D11, risk "job-object assignment refused on this box").
    #[cfg(windows)]
    assert!(spawned.job_object, "the child is assigned to a job object");
    #[cfg(unix)]
    assert!(
        !spawned.job_object,
        "job objects are a Windows mechanism; unix supervises with a process group"
    );
}

// ---------------------------------------------------------------------------------------------
// The stderr tap (plan MOD-21 D14)
// ---------------------------------------------------------------------------------------------

/// `STDERR_TAIL_LINES` as `launch.rs` sets it. Named here because the constant is private and
/// these cases are the ones that pin it: a tap must not change what the tail keeps.
#[cfg(unix)]
const TAIL_LINES: usize = 64;

/// How long a case waits for a line that should already be on its way.
///
/// Never spent — the writer is a shell one process away — it is only the distance between "the tap
/// dropped a line" and a suite that hangs instead of saying so.
#[cfg(unix)]
const PATIENCE: Duration = Duration::from_secs(10);

/// Starts `/bin/sh -c <script>` under [`htui_agent::launch::spawn`], with piped stdio.
///
/// `sh -c` rather than a fixture file: a shell script written and executed by the same test process
/// races the `ETXTBSY` window `tests/probe.rs:70-92` documents, and the way that file's own
/// streaming case sidesteps it is to have no file at all (`tests/probe.rs:1042`).
#[cfg(unix)]
async fn spawn_sh(script: &str, cwd: &std::path::Path) -> Spawned {
    let launch = ResolvedLaunch {
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), script.to_owned()],
        env: BTreeMap::new(),
    };
    htui_agent::launch::spawn(&launch, cwd)
        .await
        .expect("the shell fixture starts")
}

/// A shell condition that blocks until `path` exists, so a case is late by *fact* rather than by
/// hoping a sleep was long enough.
#[cfg(unix)]
fn wait_for(path: &std::path::Path) -> String {
    format!("while [ ! -f {} ]; do sleep 0.01; done; ", path.display())
}

/// Writes the file [`wait_for`] blocks on.
#[cfg(unix)]
fn go_ahead(path: &std::path::Path) {
    std::fs::write(path, "").expect("the fixture's go-ahead is written");
}

/// Blocks until the tail holds `lines` lines, or fails.
///
/// The reader is a task, so "the child wrote it" and "the tail has it" are two moments; every
/// assertion about the tail's *contents* needs the second one, and a child that has exited is not
/// on its own evidence the task has drained the pipe.
#[cfg(unix)]
async fn tail_reaches(spawned: &Spawned, lines: usize) {
    let deadline = Instant::now() + PATIENCE;
    while spawned.stderr_tail().len() < lines {
        assert!(
            Instant::now() < deadline,
            "the tail never reached {lines} lines; it holds {:?}",
            spawned.stderr_tail()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The tap's next line, or `None` when the tap has ended — under [`PATIENCE`], so a lost line is a
/// named failure rather than a hung suite.
#[cfg(unix)]
async fn next_line(tap: &mut UnboundedReceiver<String>) -> Option<String> {
    tokio::time::timeout(PATIENCE, tap.recv())
        .await
        .expect("the tap answered inside the patience window")
}

/// Every line the tap yields until it ends.
#[cfg(unix)]
async fn drain(tap: &mut UnboundedReceiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(line) = next_line(tap).await {
        lines.push(line);
    }
    lines
}

/// Plan D14: the tap is a *stream*, and `stderr_tail()` is not — 64 lines is a bound, not a cursor,
/// so a flow that polled it would have no way to say which lines it had already seen.
#[cfg(unix)]
#[tokio::test]
async fn a_tap_receives_every_stderr_line_in_order() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let mut spawned = spawn_sh(
        "echo a >&2; sleep 0.05; echo b >&2; sleep 0.05; echo c >&2",
        tmp.path(),
    )
    .await;

    let mut tap = spawned.tap_stderr();
    let lines = drain(&mut tap).await;
    let status = spawned.wait().await.expect("the child exits");

    assert!(status.success(), "the fixture exits 0, got {status:?}");
    assert_eq!(
        lines,
        ["a", "b", "c"],
        "the tap carries every line in the order the child wrote it"
    );
}

/// Plan D14, the reason the replay happens under the reader's own lock: a URL can be printed during
/// `initialize`, before the caller has finished wiring the flow, and a tap that started at "now"
/// would have lost it.
#[cfg(unix)]
#[tokio::test]
async fn a_tap_installed_late_replays_the_tail_first_then_streams() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let go = tmp.path().join("go");
    let script = format!(
        "echo one >&2; echo two >&2; {}echo three >&2",
        wait_for(&go)
    );
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    // Late by fact: both lines are in the tail before the tap exists.
    tail_reaches(&spawned, 2).await;
    let mut tap = spawned.tap_stderr();
    go_ahead(&go);

    let lines = drain(&mut tap).await;
    spawned.wait().await.expect("the child exits");

    assert_eq!(
        lines,
        ["one", "two", "three"],
        "the lines already in the tail are replayed first and the stream continues from there, \
         each line exactly once — nothing lost to tapping late, nothing delivered twice"
    );
}

/// The tail is what `probe.stderr_tail` records and what a failed handshake is explained with; a
/// caller that taps must not be draining it (`launch.rs`'s two review gates).
#[cfg(unix)]
#[tokio::test]
async fn the_tail_is_unchanged_by_a_tap() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    const SCRIPT: &str = "seq 1 70 >&2";
    let expected: Vec<String> = (7..=70).map(|line| line.to_string()).collect();

    let mut tapped = spawn_sh(SCRIPT, tmp.path()).await;
    let mut tap = tapped.tap_stderr();
    drop(drain(&mut tap).await);
    tapped.wait().await.expect("the tapped child exits");

    let mut untapped = spawn_sh(SCRIPT, tmp.path()).await;
    untapped.wait().await.expect("the untapped child exits");
    tail_reaches(&untapped, TAIL_LINES).await;

    assert_eq!(
        untapped.stderr_tail(),
        expected,
        "the control: 70 lines in, the last {TAIL_LINES} kept"
    );
    assert_eq!(
        tapped.stderr_tail(),
        untapped.stderr_tail(),
        "a tap neither drains the tail nor changes its cap: the same child observed twice leaves \
         the same tail"
    );
}

/// The cap is the tail's, not the tap's: a login that printed more than [`TAIL_LINES`] lines before
/// the one that mattered would otherwise lose them, which is the whole reason polling the tail is
/// not the design.
#[cfg(unix)]
#[tokio::test]
async fn past_sixty_four_lines_the_tap_has_them_all_and_the_tail_the_last_sixty_four() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let go = tmp.path().join("go");
    // The child writes nothing until the tap is installed, so "the tap has them all" is an
    // assertion about the tap and not about how fast this box scheduled the reader task.
    let script = format!("{}seq 1 100 >&2", wait_for(&go));
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    let mut tap = spawned.tap_stderr();
    go_ahead(&go);
    let lines = drain(&mut tap).await;
    spawned.wait().await.expect("the child exits");

    let all: Vec<String> = (1..=100).map(|line| line.to_string()).collect();
    assert_eq!(lines, all, "the tap drops nothing at the tail's bound");
    assert_eq!(
        spawned.stderr_tail(),
        all[all.len() - TAIL_LINES..],
        "and the tail keeps its {TAIL_LINES}-line bound whatever the tap holds"
    );
}

/// One tap per child (plan D14). A second `tap_stderr` is the newer caller's, and the older
/// receiver ends rather than silently going quiet.
#[cfg(unix)]
#[tokio::test]
async fn a_second_tap_replaces_the_first() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let go = tmp.path().join("go");
    let script = format!("echo first >&2; {}echo second >&2", wait_for(&go));
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    let mut first = spawned.tap_stderr();
    assert_eq!(
        next_line(&mut first).await.as_deref(),
        Some("first"),
        "the first tap is a tap like any other while it is the installed one"
    );

    let mut second = spawned.tap_stderr();
    assert_eq!(
        next_line(&mut first).await,
        None,
        "installing a tap ends the one it replaced, so the displaced reader is told rather than \
         left waiting on a channel nothing writes to"
    );

    go_ahead(&go);
    let lines = drain(&mut second).await;
    spawned.wait().await.expect("the child exits");

    assert_eq!(
        lines,
        ["first", "second"],
        "the replacement is a late tap like any other: the tail first, then the stream"
    );
}

/// Review L-5: one byte that is not UTF-8 does not end the reader.
///
/// `tap_stderr`'s doc has always promised "lossy UTF-8", and `BufReader::lines()` does not give it:
/// it answers `Err(InvalidData)` and **stops**, which closed the tap, froze the tail and left the
/// pipe undrained — so a child with more to say would eventually block on its own stderr. The line
/// after the stray byte is the one a login cares about, because that is where the link is.
#[cfg(unix)]
#[tokio::test]
async fn a_non_utf8_byte_neither_ends_the_stream_nor_hides_the_line_after_it() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let go = tmp.path().join("go");
    // `\376` is a byte no UTF-8 sequence may begin with: a spinner from a legacy encoding, written
    // mid-line, with the line that matters printed after it.
    let script = format!(
        "printf 'spin\\376ner\\n' >&2; {}printf 'open https://h.invalid/login\\n' >&2",
        wait_for(&go)
    );
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    let mut tap = spawned.tap_stderr();
    go_ahead(&go);
    let lines = drain(&mut tap).await;
    spawned.wait().await.expect("the child exits");

    assert_eq!(
        lines,
        ["spin\u{fffd}ner", "open https://h.invalid/login"],
        "the byte is replaced and the stream carries on: {lines:?}"
    );
    assert_eq!(
        spawned.stderr_tail(),
        lines,
        "and the tail is the same two lines, so a failed handshake is still explained"
    );
}

// ---------------------------------------------------------------------------------------------
// Signals short of a kill (plan MOD-2 D81)
// ---------------------------------------------------------------------------------------------

/// Whether a process id still names a live process, through `kill -0` rather than `/proc`: the
/// question is POSIX, and `/proc` is Linux's answer to it.
#[cfg(unix)]
async fn alive(pid: &str) -> bool {
    tokio::process::Command::new("kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Waits until `pid` is gone, or gives up after [`PATIENCE`].
#[cfg(unix)]
async fn gone(pid: &str) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if !alive(pid).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// An interrupt is a **request**, and that is the whole difference from
/// [`Spawned::kill_tree`](htui_agent::launch::Spawned::kill_tree): the child gets to run its own
/// handler and choose its exit status. A cancel that only knew how to `SIGKILL` could never let an
/// agent write the closing message that ends the turn cleanly.
///
/// `exit 42` is what makes this assertable: a status nothing but the child's own trap can produce,
/// and one no signal death carries.
#[cfg(unix)]
#[tokio::test]
async fn an_interrupt_lets_the_child_exit_on_its_own_terms() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let ready = tmp.path().join("ready");
    let script = format!(
        "trap 'exit 42' INT; printf ready > {}; while :; do sleep 0.05; done",
        ready.display()
    );
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    // Not a sleep: the trap must be installed before the signal, or the shell dies by default
    // disposition and this case would pass for the wrong reason.
    let deadline = Instant::now() + PATIENCE;
    while !ready.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready.exists(), "the fixture installed its trap");

    spawned
        .signal(StopSignal::Interrupt)
        .expect("the child's process group accepts a signal");

    let status = tokio::time::timeout(PATIENCE, spawned.wait())
        .await
        .expect("the interrupted child exits within the patience window")
        .expect("the wait itself succeeds");
    assert_eq!(
        status.code(),
        Some(42),
        "the child ran its own handler; a kill would have left a signal death instead: {status:?}"
    );
}

/// The signal goes to the **group**, not to the direct child.
///
/// The same reason `kill_tree` exists: an agent that spawned a helper is a tree, and a supervisor
/// that signalled only the process it can name would leave the rest running with nobody holding
/// them. The background `sleep` here is that helper, and it is never signalled by this test — only
/// its group is.
#[cfg(unix)]
#[tokio::test]
async fn a_signal_reaches_the_whole_process_group() {
    let tmp = tempfile::tempdir().expect("a temp working directory");
    let pidfile = tmp.path().join("helper.pid");
    let script = format!(
        "sleep 30 & printf %s $! > {}; while :; do sleep 0.05; done",
        pidfile.display()
    );
    let mut spawned = spawn_sh(&script, tmp.path()).await;

    let deadline = Instant::now() + PATIENCE;
    while !pidfile.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let helper = std::fs::read_to_string(&pidfile).expect("the fixture recorded its helper's pid");
    assert!(
        alive(&helper).await,
        "the helper is running before the signal"
    );

    spawned
        .signal(StopSignal::Terminate)
        .expect("the child's process group accepts a signal");

    tokio::time::timeout(PATIENCE, spawned.wait())
        .await
        .expect("the terminated child exits within the patience window")
        .expect("the wait itself succeeds");
    assert!(
        gone(&helper).await,
        "the helper the fixture spawned was never named by this test, and died with its group"
    );
}
