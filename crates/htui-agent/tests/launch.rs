//! `agent.launch` / `agent.settings` deserialisation, `${tool}` resolution and the supervised
//! spawn (plan MOD-2 T7, `docs/ANA-4.md` §5.1, §5.2, §4.6).
//!
//! The two launch documents below are the ANA-4 §5.3 seed rows, held here as literals. Task 8
//! lands them as `htui_core::model::agent::seed_rows`, and task 9's `tests/extensibility.rs`
//! asserts that *those* rows deserialise into the same types — this file pins the shape before
//! either exists, which is what makes the seed a test subject rather than a source of truth.

use std::collections::BTreeMap;

use htui_agent::DriverError;
use htui_agent::launch::{AgentLaunch, AgentSettings, QuotaSource, ToolMap, ToolProbe, resolve};

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
