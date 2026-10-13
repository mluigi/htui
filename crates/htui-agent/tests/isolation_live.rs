//! MOD-91's isolation levers, live: the **shipped** CLI argv and ACP `_meta` against the real
//! `claude` and `claude-agent-acp`, under a scratch Claude Code configuration (plan T6, blueprint
//! T6; the matrix it reproduces is the plan's §Probe results, T1 rows 0, 1 and 1a, and T2 (a)).
//!
//! `#[ignore]` by default, as `tests/cli_live.rs` is: every case but the D8 refusal **spends model
//! tokens** on the credential this box holds. Run it by hand, serially:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test isolation_live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! # What it drives
//!
//! - **CLI cases** go through the whole `CliDriver::start` (the factory and
//!   `ClaudeStreamAdapter`), so D8's refusal, `cli::ISOLATION_ARGS` on the argv, `HTUI_SESSION` in
//!   the launch environment and the MCP config file are all the shipped code. The row is
//!   synthetic: its command is `env -u … claude`, which keeps the parent session's `CLAUDE*`
//!   variables and every `ANTHROPIC_*` out of the child (argv rule 1 keeps a wrapper's leading
//!   arguments first), and its `launch.env` carries the scratch `CLAUDE_CONFIG_DIR`. The only flag
//!   the test adds is `--debug-file`, through `extra_args`.
//! - **ACP cases** resolve the **seeded `claude` row**'s launch, wrapped in the same `env -u`,
//!   through `AcpDriver::launch_for` (so `HTUI_SESSION` is the shipped insert), spawn it, and speak
//!   line JSON-RPC to it from a small client here. `session/new` carries
//!   `acp::session_meta(&settings)` with one key merged in: `extraArgs["debug-file"]`, the one thing
//!   the driver path cannot inject. One case resumes a session in a second adapter process with the
//!   same `_meta`, over `session/resume` (or `session/load` where only that is advertised). The
//!   driver's own `_meta` plumbing on new, resume and load is pinned by `tests/acp_driver.rs`.
//!
//!   Node and the adapter resolve exactly as a chat's do, through the `HTUI_TOOL_*` overrides or
//!   the discovery tiers. The sandbox has neither on `PATH`, so run with, for example:
//!
//!   ```text
//!   HTUI_TOOL_NODE=/tmp/node22/bin/node HTUI_TOOL_NPX=/tmp/node22/bin/npx \
//!   HTUI_TOOL_CLAUDE_AGENT_ACP=/tmp/acp-run/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js
//!   ```
//!
//!   A box where the row does not resolve **skips** the ACP cases with the reason printed.
//!
//! # The scratch configuration
//!
//! Nothing here writes under the real `~/.claude`: from the hr sandbox it is shared read-write with
//! the host (ANA-29 §3.3), and a test mod installed there would load in every host session. Each
//! case builds its own tempdir:
//!
//! - `CLAUDE_CONFIG_DIR`, holding a `.credentials.json` that is `{"claudeAiOauth": …}` alone, the
//!   box's object copied **without** its `refreshToken`, so a refresh can never rotate the host's
//!   token. An expired access token fails the auth assertion; it is never refreshed from here;
//! - `.claude.json` with onboarding done and the checkout trusted (Claude Code writes
//!   `oauthAccount` there itself; the T1 probe did not seed it);
//! - `settings.json` with a user `SessionStart` hook writing `${HTUI_SESSION-unset}` to a marker;
//! - a user skill, and a user-scope mod installed with `claude plugin marketplace add` and
//!   `claude plugin install` from `tests/fixtures/isolation/marketplace`;
//! - a git checkout, trusted, used as the session's cwd, with a skills-directory mod and a project
//!   skill under `.claude/skills/`;
//! - `tests/fixtures/isolation/stub_mcp.py` as the session's MCP server `htui` (blueprint B8): it
//!   serves `permission_prompt`, allows every call, and logs each method and its `HTUI_SESSION`.
//!
//! Which mods loaded is read from `--debug-file`: `hooks module <id> loaded` or `… not loaded`.
//!
//! Every observation is printed before anything is asserted, so a failing run is still a record.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::AcpDriver;
use htui_agent::cli::ClaudeStreamAdapter;
use htui_agent::driver::{
    AgentDriver, DriverCaps, McpServerSpec, PermissionPolicy, SessionSpec, ToolExposure,
};
use htui_agent::error::DriverError;
use htui_agent::event::{DriverEvent, StopReason};
use htui_agent::launch::{AgentSettings, ChildIo, SESSION_VAR};
use htui_agent::registry::DriverFactory;
use htui_core::model::{Agent as AgentRow, AgentId, Billing, StepId, Transport};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// One live session at a time: the cases read marker files and process state, and the
/// subscription's window is shared.
static ONE_AT_A_TIME: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// How long one turn may take before the case fails rather than hangs.
const TURN_WINDOW: Duration = Duration::from_secs(240);

/// The two mods the fixtures install, as the debug file names them.
const PROBE_MODS: [&str; 2] = ["probe-user@mod91-probe", "probe-proj@skills-dir"];

/// The two skills the fixtures install.
const PROBE_SKILLS: [&str; 2] = ["probe-skill", "probe-uskill"];

/// Variables the child must keep although they start with `CLAUDE`: the row sets them.
const KEPT: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_EXECUTABLE"];

/// Whether the child must not inherit `name`: the parent Claude Code session's `CLAUDE*` markers
/// (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, …), as `/tmp/mod91/cc` scrubbed them, and every
/// `ANTHROPIC_*`, so an API key or base URL cannot stand in for the subscription OAuth.
fn scrubbed(name: &str) -> bool {
    (name.starts_with("CLAUDE") && !KEPT.contains(&name)) || name.starts_with("ANTHROPIC_")
}

// ---------------------------------------------------------------------------------------------
// The scratch configuration
// ---------------------------------------------------------------------------------------------

/// One case's scratch world, removed when it drops.
struct Scratch {
    _root: tempfile::TempDir,
    /// The scratch `CLAUDE_CONFIG_DIR`.
    cfg: PathBuf,
    /// The trusted git checkout the session runs in.
    checkout: PathBuf,
    /// What the user `SessionStart` hook writes `${HTUI_SESSION-unset}` to.
    marker: PathBuf,
    /// The stub MCP server's log.
    mcp_log: PathBuf,
    /// Claude Code's `--debug-file`.
    debug: PathBuf,
    /// The file a gated `Bash` call creates.
    touched: PathBuf,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/isolation")
}

/// The first `name` on `PATH`, as an absolute path.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// `env -u NAME` for every variable this process carries that the child must not inherit
/// ([`scrubbed`]).
fn scrub_args() -> Vec<String> {
    let mut names: Vec<String> = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| scrubbed(name))
        .collect();
    names.sort();
    names
        .into_iter()
        .flat_map(|name| ["-u".to_owned(), name])
        .collect()
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a fixture directory");
    for entry in std::fs::read_dir(from).expect("read a fixture directory") {
        let entry = entry.expect("a fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy a fixture file");
        }
    }
}

/// Runs `program args` with the scrubbed environment and the scratch config, and requires success.
fn run(program: &Path, args: &[&str], cwd: &Path, cfg: &Path) {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env("CLAUDE_CONFIG_DIR", cfg);
    for (name, _) in std::env::vars_os() {
        if let Some(name) = name.to_str()
            && scrubbed(name)
        {
            command.env_remove(name);
        }
    }
    let out = command.output().expect("the command starts");
    assert!(
        out.status.success(),
        "{} {args:?} failed: {}\n{}",
        program.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

impl Scratch {
    /// Builds the whole scratch world. Panics, loudly, on a missing precondition: an ignored test
    /// that was asked for must not pass vacuously.
    fn new(claude: &Path) -> Self {
        let root = tempfile::tempdir().expect("a scratch root");
        let base = root.path().canonicalize().expect("canonical scratch root");
        let cfg = base.join("cfg");
        let checkout = base.join("checkout");
        std::fs::create_dir_all(&cfg).expect("cfg dir");
        std::fs::create_dir_all(&checkout).expect("checkout dir");

        // Credentials: read the real file, never write it; copy only the subscription OAuth
        // object, without its refresh token.
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        let real = home.join(".claude/.credentials.json");
        let text = std::fs::read_to_string(&real)
            .unwrap_or_else(|err| panic!("{} is unreadable: {err}", real.display()));
        let creds: Value = serde_json::from_str(&text).expect("credentials parse");
        let mut oauth = creds
            .get("claudeAiOauth")
            .and_then(Value::as_object)
            .cloned()
            .expect("the box holds a subscription OAuth credential");
        oauth.remove("refreshToken");
        let creds_path = cfg.join(".credentials.json");
        write_private(&creds_path, &json!({ "claudeAiOauth": oauth }).to_string());

        let checkout_key = checkout.to_string_lossy().into_owned();
        let claude_json = json!({
            "hasCompletedOnboarding": true,
            "projects": { checkout_key: { "hasTrustDialogAccepted": true } },
        });
        std::fs::write(cfg.join(".claude.json"), claude_json.to_string()).expect(".claude.json");

        let marker = base.join("settings-hook-fired");
        let hook = format!(
            "printf %s \"${{{SESSION_VAR}-unset}}\" > '{}'",
            marker.display()
        );
        let settings = json!({
            "hooks": { "SessionStart": [ { "hooks": [ { "type": "command", "command": hook } ] } ] },
        });
        std::fs::write(cfg.join("settings.json"), settings.to_string()).expect("settings.json");
        copy_tree(&fixtures().join("user-skills"), &cfg.join("skills"));

        // The user-scope mod, installed through Claude Code's own commands (no tokens spent).
        let market = base.join("marketplace");
        copy_tree(&fixtures().join("marketplace"), &market);
        let market_arg = market.to_string_lossy().into_owned();
        run(
            claude,
            &["plugin", "marketplace", "add", &market_arg],
            &checkout,
            &cfg,
        );
        run(
            claude,
            &["plugin", "install", "probe-user@mod91-probe"],
            &checkout,
            &cfg,
        );

        // The checkout: a git repository with a skills-directory mod and a project skill.
        let skills = checkout.join(".claude/skills");
        copy_tree(&fixtures().join("skills-dir"), &skills);
        copy_tree(&fixtures().join("skills"), &skills);
        let git = on_path("git").expect("`git` on PATH");
        run(&git, &["init", "-q"], &checkout, &cfg);
        run(&git, &["add", "-A"], &checkout, &cfg);
        run(
            &git,
            &[
                "-c",
                "user.name=htui",
                "-c",
                "user.email=htui@localhost",
                "commit",
                "-qm",
                "probe",
            ],
            &checkout,
            &cfg,
        );

        Self {
            mcp_log: base.join("mcp-calls.log"),
            debug: base.join("debug.log"),
            touched: checkout.join("touched"),
            _root: root,
            cfg,
            checkout,
            marker,
        }
    }

    /// The stub `htui` server, as `htui`'s own servers travel in a spec.
    fn stub_server(&self) -> McpServerSpec {
        McpServerSpec {
            name: "htui".to_owned(),
            command: "python3".to_owned(),
            args: vec![
                fixtures()
                    .join("stub_mcp.py")
                    .to_string_lossy()
                    .into_owned(),
                self.mcp_log.to_string_lossy().into_owned(),
            ],
            env: BTreeMap::new(),
        }
    }

    fn spec(&self) -> SessionSpec {
        SessionSpec {
            agent_id: AgentId::new(),
            step_id: StepId::new(),
            cwd: self.checkout.clone(),
            extra_dirs: Vec::new(),
            env: BTreeMap::new(),
            model: None,
            tools: ToolExposure::default(),
            mcp: vec![self.stub_server()],
            permission: PermissionPolicy::default(),
            retain_raw: true,
            resume: None,
            budget_micros: None,
            prompt: None,
        }
    }

    fn read(path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    /// `(mod id, loaded?)` for each probe mod the debug file mentions.
    fn mods(&self) -> BTreeMap<String, bool> {
        Self::mods_in(&self.debug)
    }

    /// [`Self::mods`] for the debug file at `path`.
    fn mods_in(path: &Path) -> BTreeMap<String, bool> {
        let debug = Self::read(path).unwrap_or_default();
        let mut mods = BTreeMap::new();
        for line in debug.lines() {
            for id in PROBE_MODS {
                if line.contains(&format!("hooks module {id} loaded")) {
                    mods.insert(id.to_owned(), true);
                } else if line.contains(&format!("hooks module {id} not loaded")) {
                    mods.entry(id.to_owned()).or_insert(false);
                }
            }
        }
        mods
    }

    fn mcp_log(&self) -> String {
        Self::read(&self.mcp_log).unwrap_or_default()
    }

    fn hook_marker(&self) -> Option<String> {
        Self::read(&self.marker)
    }
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("create the scratch credentials 0600");
    file.write_all(text.as_bytes()).expect("write credentials");
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) {
    std::fs::write(path, text).expect("write credentials");
}

/// `claude`, `python3` and `git` are preconditions: a missing one fails the case.
fn claude_on_path() -> PathBuf {
    assert!(
        on_path("python3").is_some(),
        "`python3` on PATH runs the stub MCP server"
    );
    on_path("claude").expect("`claude` on PATH")
}

// ---------------------------------------------------------------------------------------------
// CLI cases
// ---------------------------------------------------------------------------------------------

/// A synthetic `cli` row: `env -u … <claude>`, the scratch config, `--permission-mode default`
/// (so `Bash` is gated and asks the prompt tool), and `extra_args` as given.
fn cli_row(scratch: &Scratch, claude: &Path, user_extensions: bool, extra: &[&str]) -> AgentRow {
    let mut args = scrub_args();
    args.push(claude.to_string_lossy().into_owned());
    let now = Utc::now();
    let mut extra_args = vec![
        "--debug-file".to_owned(),
        scratch.debug.to_string_lossy().into_owned(),
    ];
    extra_args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    AgentRow {
        id: AgentId::new(),
        name: "isolation-live-cli".to_owned(),
        transport: Transport::Cli,
        launch: json!({
            "command": "env",
            "args": args,
            "env": { "CLAUDE_CONFIG_DIR": scratch.cfg.to_string_lossy() },
            "discovery": { "handshake": false, "tools": {} },
        }),
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({
            "cli": {
                "stream": htui_agent::cli::STREAM,
                "permission_mode": "default",
                "extra_args": extra_args,
            },
            "user_extensions": user_extensions,
        }),
        created_at: now,
        updated_at: now,
    }
}

fn cli_driver(row: &AgentRow) -> Box<dyn AgentDriver> {
    let mut factory = DriverFactory::new();
    factory.register(
        htui_agent::cli::ADAPTER_ID,
        Box::new(ClaudeStreamAdapter) as Box<dyn htui_agent::registry::TransportBuilder>,
    );
    factory
        .driver_for(row, None)
        .expect("the `cli/claude_stream_json` adapter builds this row")
}

/// What one CLI turn showed.
#[derive(Debug, Default)]
struct CliTurn {
    stop: Option<StopReason>,
    errors: Vec<String>,
    init: Option<Value>,
}

impl CliTurn {
    fn init_list(&self, key: &str) -> Vec<String> {
        self.init
            .as_ref()
            .and_then(|init| init.get(key))
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|item| {
                        item.as_str()
                            .map(ToOwned::to_owned)
                            .or_else(|| item.get("name").and_then(Value::as_str).map(Into::into))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn probe_skills(&self) -> Vec<String> {
        let mut names = self.init_list("skills");
        names.extend(self.init_list("slash_commands"));
        names.retain(|name| PROBE_SKILLS.iter().any(|skill| name.contains(skill)));
        names.sort();
        names.dedup();
        names
    }
}

const CLI_PROMPT: &str = "Run this exact Bash command: touch ./touched . Then reply with the single \
                          word DONE.";

/// Starts a session over `row` and reads it to its first `done`.
async fn cli_turn(row: &AgentRow, scratch: &Scratch) -> CliTurn {
    cli_turn_with(row, scratch.spec(), CLI_PROMPT).await
}

/// [`cli_turn`] with the spec and the prompt given.
async fn cli_turn_with(row: &AgentRow, mut spec: SessionSpec, prompt: &str) -> CliTurn {
    let driver = cli_driver(row);
    let (port, ask) = htui_agent::prompt_bridge::bridge();
    spec.prompt = Some(port);
    let mut session = driver
        .start(spec, prompt.to_owned())
        .await
        .unwrap_or_else(|err| panic!("the session starts (an expired token fails here): {err}"));
    let mut turn = CliTurn::default();
    let read = async {
        while let Some(envelope) = session.next_event().await.expect("the session streams") {
            if let Some(raw) = &envelope.raw
                && raw.get("type").and_then(Value::as_str) == Some("system")
                && raw.get("subtype").and_then(Value::as_str) == Some("init")
            {
                turn.init = Some(raw.clone());
            }
            match envelope.event {
                DriverEvent::Error(error) => turn.errors.push(format!("{error:?}")),
                DriverEvent::Done(done) => {
                    turn.stop = Some(done.stop_reason);
                    break;
                }
                _ => {}
            }
        }
    };
    tokio::time::timeout(TURN_WINDOW, read)
        .await
        .expect("the turn ends within the window");
    let _ = session.cancel(Duration::from_secs(2)).await;
    drop(session);
    drop(ask);
    tokio::time::sleep(Duration::from_millis(500)).await;
    turn
}

fn print_cli(label: &str, scratch: &Scratch, turn: &CliTurn) {
    let init = turn.init.as_ref();
    println!("== {label}");
    println!("  stop: {:?}  errors: {:?}", turn.stop, turn.errors);
    println!(
        "  mcp_servers: {:?}",
        init.and_then(|init| init.get("mcp_servers"))
    );
    println!("  plugins: {:?}", init.and_then(|init| init.get("plugins")));
    println!(
        "  skills: {} total, probe: {:?}",
        turn.init_list("skills").len(),
        turn.probe_skills()
    );
    println!("  mods: {:?}", scratch.mods());
    println!("  settings hook marker: {:?}", scratch.hook_marker());
    println!("  touched: {}", scratch.touched.exists());
    println!("  stub log:\n{}", indent(&scratch.mcp_log()));
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The mods as `(id, loaded)` pairs, sorted, for exact assertions.
fn both_mods(loaded: bool) -> BTreeMap<String, bool> {
    PROBE_MODS
        .iter()
        .map(|id| ((*id).to_owned(), loaded))
        .collect()
}

/// T1 row 0 through the shipped code: a row that opts back in loads everything, and the
/// extensions it lets in see `HTUI_SESSION=1`. The baseline must be positive, or the isolated case
/// proves nothing.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn cli_opted_in_is_the_positive_baseline() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let row = cli_row(&scratch, &claude, true, &[]);
    let turn = cli_turn(&row, &scratch).await;
    print_cli("cli opted in (user_extensions = true)", &scratch, &turn);

    assert_eq!(
        turn.stop,
        Some(StopReason::EndTurn),
        "auth and the turn work"
    );
    assert_eq!(scratch.mods(), both_mods(true), "both mods load");
    assert_eq!(
        scratch.hook_marker().as_deref(),
        Some("1"),
        "the settings hook fires and sees HTUI_SESSION=1"
    );
    assert_eq!(
        turn.probe_skills(),
        PROBE_SKILLS.map(String::from).to_vec(),
        "both probe skills are listed"
    );
    assert!(
        scratch.mcp_log().contains("CALL "),
        "permission_prompt was asked"
    );
    assert!(scratch.touched.exists(), "the allowed Bash call ran");
}

/// T1 row 1 through the shipped code (D1, D4): auth and htui's prompt tool work, no mod loads, the
/// settings hook does not fire, no skill is listed, and the stub server sees `HTUI_SESSION=1`.
/// Since MOD-96's `--setting-sources ""` no settings file enables the probe mods, so the debug file
/// no longer names them at all (it did under MOD-91's levers: both "not loaded"): the init
/// envelope's `plugins` is what shows neither was even enabled.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn cli_isolated_by_default() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let row = cli_row(&scratch, &claude, false, &[]);
    let turn = cli_turn(&row, &scratch).await;
    print_cli("cli isolated (user_extensions = false)", &scratch, &turn);

    assert_eq!(
        turn.stop,
        Some(StopReason::EndTurn),
        "auth and the turn work"
    );
    let log = scratch.mcp_log();
    assert!(
        log.contains("initialize") && log.contains("tools/list"),
        "htui's server connects"
    );
    assert!(log.contains("CALL "), "permission_prompt was asked");
    assert!(
        log.contains("env HTUI_SESSION=1"),
        "HTUI_SESSION reaches the session"
    );
    assert!(scratch.touched.exists(), "the allowed Bash call ran");
    assert!(
        scratch.mods().values().all(|loaded| !loaded),
        "no probe mod loads"
    );
    assert!(turn.init.is_some(), "the init envelope arrived");
    let plugins = turn.init_list("plugins");
    assert!(
        PROBE_MODS
            .iter()
            .filter_map(|id| id.split('@').next())
            .all(|name| !plugins.iter().any(|plugin| plugin == name)),
        "neither probe mod is enabled: {plugins:?}"
    );
    assert_eq!(
        scratch.hook_marker(),
        None,
        "the settings hook does not fire"
    );
    assert_eq!(
        turn.probe_skills(),
        Vec::<String>::new(),
        "no probe skill is listed"
    );
    assert!(
        turn.init_list("skills").is_empty(),
        "no skill is listed at all"
    );
}

/// T1 row 1a, refused by D8: an `extra_args` `--settings` that drops `disableAllHooks` would replace
/// htui's own, so `start` refuses before anything is spawned. Spends no tokens.
#[tokio::test]
#[ignore = "needs `claude` and this box's credential to build the scratch config"]
async fn cli_an_overriding_settings_is_refused() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let row = cli_row(&scratch, &claude, false, &["--settings", r#"{"env":{}}"#]);
    let result = cli_driver(&row)
        .start(scratch.spec(), CLI_PROMPT.to_owned())
        .await;
    let err = match result {
        Ok(_) => panic!("an isolated row with an overriding --settings must be refused"),
        Err(err) => err,
    };
    println!("== cli D8 refusal\n  {err}");
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        matches!(&err, DriverError::Transport(message) if message.contains("`--settings`")),
        "a Transport error naming the flag: {err:?}"
    );
    assert!(
        !err.to_string().contains(r#"{"env":{}}"#),
        "the value is never quoted"
    );
    assert!(!scratch.debug.exists(), "claude never started");
    assert!(!scratch.mcp_log.exists(), "no MCP server started");
}

/// MOD-96 probes A4 and B4: the command the gated `Bash` call runs. It writes a file, because a
/// read-only command such as `echo` is never asked about (probes A2, A3 and B3 proved nothing).
const PY_COMMAND: &str = r#"python3 -c "open('py_out.txt','w').write('x')""#;

/// The file [`PY_COMMAND`] writes, in the checkout.
const PY_OUT: &str = "py_out.txt";

/// `stub_mcp.py`'s one verdict line, and the deny that replaces it in [`denying_stub`].
const STUB_ALLOW: &str =
    r#"verdict = {"behavior": "allow", "updatedInput": args.get("input", {})}"#;
const STUB_DENY: &str =
    r#"verdict = {"behavior": "deny", "message": "htui denied this call. Do not retry it."}"#;

/// The stub `htui` server answering every `permission_prompt` with deny: `stub_mcp.py` with its one
/// verdict line swapped, written into the scratch root (never into the fixtures).
fn denying_stub(scratch: &Scratch) -> McpServerSpec {
    let source =
        std::fs::read_to_string(fixtures().join("stub_mcp.py")).expect("stub_mcp.py reads");
    assert_eq!(
        source.matches(STUB_ALLOW).count(),
        1,
        "stub_mcp.py's verdict line changed; update STUB_ALLOW"
    );
    let path = scratch.mcp_log.with_file_name("stub_mcp_deny.py");
    std::fs::write(&path, source.replace(STUB_ALLOW, STUB_DENY)).expect("write the denying stub");
    let mut server = scratch.stub_server();
    server.args[0] = path.to_string_lossy().into_owned();
    server
}

/// The checkout's project settings file allows `Bash` (probes A4/B4's setup). Written after
/// [`Scratch::new`]'s commit; the CLI reads it whether or not it is committed.
fn allow_bash_in_project(scratch: &Scratch) {
    let dir = scratch.checkout.join(".claude");
    std::fs::create_dir_all(&dir).expect("the checkout's .claude");
    std::fs::write(
        dir.join("settings.json"),
        r#"{"permissions":{"allow":["Bash"]}}"#,
    )
    .expect("the project settings.json");
}

/// One turn asking for [`PY_COMMAND`], with the denying stub as `htui`.
async fn py_turn(row: &AgentRow, scratch: &Scratch) -> CliTurn {
    let mut spec = scratch.spec();
    spec.mcp = vec![denying_stub(scratch)];
    let prompt = format!(
        "Use the Bash tool to run exactly this command, and nothing else: {PY_COMMAND} . Then \
         reply with the single word DONE."
    );
    cli_turn_with(row, spec, &prompt).await
}

/// The arguments of every `permission_prompt` call the stub logged.
fn prompt_calls(scratch: &Scratch) -> Vec<Value> {
    scratch
        .mcp_log()
        .lines()
        .filter_map(|line| line.strip_prefix("CALL "))
        .map(|args| serde_json::from_str(args).expect("the stub logs a call's arguments as JSON"))
        .collect()
}

fn print_py(label: &str, scratch: &Scratch, turn: &CliTurn) {
    print_cli(label, scratch, turn);
    println!("  init permissionMode: {:?}", init_mode(turn));
    println!(
        "  {PY_OUT} written: {}",
        scratch.checkout.join(PY_OUT).exists()
    );
    let calls = prompt_calls(scratch);
    println!(
        "  permission_prompt calls: {} {:?}",
        calls.len(),
        calls
            .iter()
            .map(|call| call.get("tool_name").and_then(Value::as_str).unwrap_or("?"))
            .collect::<Vec<_>>()
    );
}

fn init_mode(turn: &CliTurn) -> Option<&str> {
    turn.init
        .as_ref()
        .and_then(|init| init.get("permissionMode"))
        .and_then(Value::as_str)
}

/// MOD-96 A4/B4 and D3 through the shipped code: a project `settings.json` that allows `Bash`
/// settles the call on a row that opts back in (the baseline: written, htui never asked), and is
/// not read under isolation (`--setting-sources ""`): htui's prompt is asked, denies, and nothing is
/// written. The isolated row names no permission mode, so D3's `--permission-mode default` is what
/// the init envelope reports. The stub denies in both halves, so the baseline's file can only come
/// from the settings file's `allow`.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn cli_settings_file_allow_is_asked_under_isolation() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();

    let opted = Scratch::new(&claude);
    allow_bash_in_project(&opted);
    let row = cli_row(&opted, &claude, true, &[]);
    let opted_turn = py_turn(&row, &opted).await;
    print_py(
        "cli opted in, project allow [Bash] (A4, the baseline)",
        &opted,
        &opted_turn,
    );

    let isolated = Scratch::new(&claude);
    allow_bash_in_project(&isolated);
    let mut row = cli_row(&isolated, &claude, false, &[]);
    row.settings["cli"]["permission_mode"] = json!("");
    let isolated_turn = py_turn(&row, &isolated).await;
    print_py(
        "cli isolated, project allow [Bash], no row mode (B4, D3)",
        &isolated,
        &isolated_turn,
    );

    // The baseline must be positive, or the isolated half proves nothing.
    assert_eq!(
        opted_turn.stop,
        Some(StopReason::EndTurn),
        "baseline: auth and the turn work"
    );
    assert!(
        prompt_calls(&opted).is_empty(),
        "baseline: the project allow settles the call, so htui is never asked"
    );
    assert!(
        opted.checkout.join(PY_OUT).exists(),
        "baseline: the allowed Bash call wrote {PY_OUT}; if it did not, this case cannot prove \
         the isolated half"
    );

    assert_eq!(
        isolated_turn.stop,
        Some(StopReason::EndTurn),
        "isolated: auth and the turn work"
    );
    assert_eq!(
        init_mode(&isolated_turn),
        Some("default"),
        "isolated: D3 passes --permission-mode default for a row that names none"
    );
    let calls = prompt_calls(&isolated);
    let first = calls
        .first()
        .expect("isolated: the settings file is not read, so htui's prompt is asked");
    assert_eq!(
        first.get("tool_name").and_then(Value::as_str),
        Some("Bash"),
        "isolated: the first call asked about is the Bash call: {first}"
    );
    assert!(
        first
            .pointer("/input/command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(PY_OUT)),
        "isolated: the call asked about is the probe command: {first}"
    );
    assert!(
        !isolated.checkout.join(PY_OUT).exists(),
        "isolated: htui denied, so nothing wrote {PY_OUT}"
    );
}

// ---------------------------------------------------------------------------------------------
// ACP cases
// ---------------------------------------------------------------------------------------------

/// The seeded `claude` row with its launch wrapped in `env -u …` and the scratch config added.
fn acp_row(scratch: &Scratch, user_extensions: bool) -> AgentRow {
    let mut row = htui_core::model::agent::seed_rows(Utc::now())
        .into_iter()
        .find(|agent| agent.name == "claude")
        .expect("the seed rows carry `claude`");
    let launch = row
        .launch
        .as_object_mut()
        .expect("the seed launch is an object");
    let command = launch.get("command").cloned().expect("a command");
    let seed_args = launch
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut args: Vec<Value> = scrub_args().into_iter().map(Value::String).collect();
    args.push(command);
    args.extend(seed_args);
    launch.insert("command".to_owned(), json!("env"));
    launch.insert("args".to_owned(), Value::Array(args));
    launch
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("the seed env is an object")
        .insert(
            "CLAUDE_CONFIG_DIR".to_owned(),
            json!(scratch.cfg.to_string_lossy()),
        );
    row.settings
        .as_object_mut()
        .expect("the seed settings are an object")
        .insert("user_extensions".to_owned(), json!(user_extensions));
    row
}

/// `acp::session_meta` for the row's settings, plus `extraArgs["debug-file"]`.
fn acp_meta(row: &AgentRow, debug: &Path) -> Map<String, Value> {
    let settings: AgentSettings =
        serde_json::from_value(row.settings.clone()).expect("the row's settings parse");
    let mut meta = htui_agent::acp::session_meta(&settings).unwrap_or_default();
    let options = meta
        .entry("claudeCode")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("claudeCode is an object")
        .entry("options")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("options is an object");
    options
        .entry("extraArgs")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("extraArgs is an object")
        .insert("debug-file".to_owned(), json!(debug.to_string_lossy()));
    meta
}

/// A line JSON-RPC client over the adapter's stdio, mirroring `/tmp/mod91/acp_client.py`.
struct AcpClient {
    writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    inbox: mpsc::UnboundedReceiver<Value>,
    child: htui_agent::launch::Spawned,
    next_id: u64,
    updates: Vec<Value>,
}

impl AcpClient {
    fn new(io: ChildIo) -> Self {
        let (tx, inbox) = mpsc::unbounded_channel();
        let mut lines = BufReader::new(io.reader).lines();
        htui_agent::contained::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(message) = serde_json::from_str::<Value>(&line)
                    && tx.send(message).is_err()
                {
                    break;
                }
            }
        });
        Self {
            writer: io.writer,
            inbox,
            child: io.child.expect("a spawned child"),
            next_id: 0,
            updates: Vec::new(),
        }
    }

    async fn send(&mut self, message: &Value) {
        let line = format!("{message}\n");
        self.writer
            .write_all(line.as_bytes())
            .await
            .expect("write to the adapter");
        self.writer.flush().await.expect("flush to the adapter");
    }

    /// Handles one inbound message that is not the awaited response.
    async fn handle(&mut self, message: Value) {
        match message.get("method").and_then(Value::as_str) {
            Some("session/update") => {
                if let Some(update) = message.pointer("/params/update") {
                    self.updates.push(update.clone());
                }
            }
            Some("session/request_permission") => {
                let options = message
                    .pointer("/params/options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let pick = options
                    .iter()
                    .find(|option| {
                        option
                            .get("kind")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind.starts_with("allow"))
                    })
                    .or_else(|| options.first())
                    .and_then(|option| option.get("optionId"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": message.get("id"),
                    "result": { "outcome": { "outcome": "selected", "optionId": pick } },
                });
                self.send(&reply).await;
            }
            Some(_) if message.get("id").is_some() => {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": message.get("id"),
                    "error": { "code": -32601, "message": "unsupported" },
                });
                self.send(&reply).await;
            }
            _ => {}
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send(&request).await;
        let wait = async {
            loop {
                let message = self
                    .inbox
                    .recv()
                    .await
                    .expect("the adapter's stdout stays open");
                if message.get("id").and_then(Value::as_u64) == Some(id)
                    && message.get("method").is_none()
                {
                    return message;
                }
                self.handle(message).await;
            }
        };
        tokio::time::timeout(TURN_WINDOW, wait)
            .await
            .unwrap_or_else(|_| panic!("`{method}` is answered within the window"))
    }

    /// Takes whatever arrives within `window`.
    async fn drain(&mut self, window: Duration) {
        let deadline = tokio::time::Instant::now() + window;
        while let Ok(Some(message)) = tokio::time::timeout_at(deadline, self.inbox.recv()).await {
            self.handle(message).await;
        }
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
        let _ = self.child.kill_tree().await;
        let _ = self.child.wait().await;
    }
}

/// What one ACP turn showed.
#[derive(Debug, Default)]
struct AcpTurn {
    stop: Option<String>,
    commands: Vec<String>,
    modes: Vec<String>,
}

const ACP_PROMPT: &str = "Do exactly two things in order: 1) call the EnterPlanMode tool; 2) reply \
                          with the single word DONE. Do not do anything else.";

/// The cheapest turn: no tool, one word back.
const ACP_SHORT_PROMPT: &str = "Reply with the single word OK. Do not do anything else.";

/// Resolves and spawns the row's adapter and has it answer `initialize`, returning the client and
/// that answer; `None` (with the reason printed) when the seeded row does not resolve on this box.
async fn acp_start(scratch: &Scratch, row: &AgentRow) -> Option<(AcpClient, Value)> {
    let driver = AcpDriver::from_row(row, DriverCaps::default()).expect("the row builds");
    let spec = scratch.spec();
    let resolved = match driver.launch_for(&spec).await {
        Ok(resolved) => resolved,
        Err(DriverError::Unresolved(tool)) => {
            println!(
                "SKIPPED: the seeded `claude` row does not resolve `{tool}` on this box. Put node \
                 22+ and claude-agent-acp 0.75.0 where discovery finds them, or set \
                 HTUI_TOOL_NODE, HTUI_TOOL_NPX and HTUI_TOOL_CLAUDE_AGENT_ACP"
            );
            return None;
        }
        Err(err) => panic!("the seeded row's launch resolves: {err}"),
    };
    assert_eq!(
        resolved.env.get(SESSION_VAR).map(String::as_str),
        Some("1"),
        "launch_for marks the session"
    );
    let spawned = htui_agent::launch::spawn(&resolved, &scratch.checkout)
        .await
        .expect("the adapter starts");
    let mut client = AcpClient::new(ChildIo::from_spawned(spawned).expect("piped stdio"));

    let init = client
        .request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false,
                },
            }),
        )
        .await;
    assert!(init.get("result").is_some(), "initialize answers: {init}");
    Some((client, init))
}

/// The stub `htui` server as an ACP `mcpServers` list.
fn acp_servers(scratch: &Scratch) -> Value {
    let stub = scratch.stub_server();
    json!([{
        "name": stub.name,
        "command": stub.command,
        "args": stub.args,
        "env": [],
    }])
}

/// `session/new` with the row's `_meta` and `debug` as the debug file; the new session's id.
async fn acp_new(
    client: &mut AcpClient,
    scratch: &Scratch,
    row: &AgentRow,
    debug: &Path,
) -> String {
    let new = client
        .request(
            "session/new",
            json!({
                "cwd": scratch.checkout,
                "mcpServers": acp_servers(scratch),
                "_meta": acp_meta(row, debug),
            }),
        )
        .await;
    let session_id = new
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("session/new answers a session (an expired token fails here): {new}")
        })
        .to_owned();
    client.drain(Duration::from_secs(2)).await;
    session_id
}

/// One `session/prompt` and its stop reason.
async fn acp_prompt(client: &mut AcpClient, session_id: &str, text: &str) -> Option<String> {
    let prompt = client
        .request(
            "session/prompt",
            json!({ "sessionId": session_id, "prompt": [{ "type": "text", "text": text }] }),
        )
        .await;
    client.drain(Duration::from_secs(2)).await;
    let stop = prompt
        .pointer("/result/stopReason")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if stop.is_none() {
        println!("  session/prompt answered: {prompt}");
    }
    stop
}

/// The commands and mode changes among the updates `client` received, with `stop`.
fn acp_turn_of(client: &AcpClient, stop: Option<String>) -> AcpTurn {
    let mut turn = AcpTurn {
        stop,
        ..AcpTurn::default()
    };
    for update in &client.updates {
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("available_commands_update") => {
                let names = update
                    .get("availableCommands")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|command| command.get("name").and_then(Value::as_str));
                turn.commands.extend(names.map(ToOwned::to_owned));
            }
            Some("current_mode_update") => {
                if let Some(mode) = update.get("currentModeId").and_then(Value::as_str) {
                    turn.modes.push(mode.to_owned());
                }
            }
            _ => {}
        }
    }
    turn.commands.sort();
    turn.commands.dedup();
    turn
}

/// Resolves, spawns and drives one ACP session; `None` (with the reason printed) when the seeded
/// row does not resolve on this box.
async fn acp_turn(scratch: &Scratch, user_extensions: bool) -> Option<AcpTurn> {
    let row = acp_row(scratch, user_extensions);
    let (mut client, _) = acp_start(scratch, &row).await?;
    let session_id = acp_new(&mut client, scratch, &row, &scratch.debug).await;
    let stop = acp_prompt(&mut client, &session_id, ACP_PROMPT).await;
    let turn = acp_turn_of(&client, stop);
    client.close().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    Some(turn)
}

fn print_acp(label: &str, scratch: &Scratch, turn: &AcpTurn) {
    let probe: Vec<&String> = turn
        .commands
        .iter()
        .filter(|name| PROBE_SKILLS.iter().any(|skill| name.contains(skill)))
        .collect();
    println!("== {label}");
    println!("  stop: {:?}", turn.stop);
    println!(
        "  commands: {} total, probe: {probe:?}",
        turn.commands.len()
    );
    println!("  current_mode_update: {:?}", turn.modes);
    println!("  mods: {:?}", scratch.mods());
    println!("  settings hook marker: {:?}", scratch.hook_marker());
    println!("  stub log:\n{}", indent(&scratch.mcp_log()));
}

/// T2's baseline through the shipped `_meta`: a row that opts back in sends no `claudeCode` key
/// beyond the debug file, and both mods, the hook and the probe commands are there.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn acp_opted_in_is_the_positive_baseline() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let Some(turn) = acp_turn(&scratch, true).await else {
        return;
    };
    print_acp("acp opted in (user_extensions = true)", &scratch, &turn);

    assert_eq!(
        turn.stop.as_deref(),
        Some("end_turn"),
        "auth and the turn work"
    );
    assert_eq!(scratch.mods(), both_mods(true), "both mods load");
    assert_eq!(
        scratch.hook_marker().as_deref(),
        Some("1"),
        "the hook fires, under HTUI_SESSION=1"
    );
    for skill in PROBE_SKILLS {
        assert!(
            turn.commands.iter().any(|name| name.contains(skill)),
            "`{skill}` is listed among the commands"
        );
    }
}

/// T2 (a) through the shipped `_meta` (D5): the turn ends, htui's server connects, neither mod
/// loads, the hook does not fire, the command list is empty, and the adapter's own `PostToolUse`
/// hook still reports `current_mode_update: plan`.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn acp_isolated_by_default() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let Some(turn) = acp_turn(&scratch, false).await else {
        return;
    };
    print_acp("acp isolated (user_extensions = false)", &scratch, &turn);

    assert_eq!(
        turn.stop.as_deref(),
        Some("end_turn"),
        "auth and the turn work"
    );
    let log = scratch.mcp_log();
    assert!(
        log.contains("initialize") && log.contains("tools/list"),
        "htui's server connects"
    );
    assert!(
        log.contains("env HTUI_SESSION=1"),
        "HTUI_SESSION reaches the session"
    );
    assert_eq!(
        scratch.mods(),
        both_mods(false),
        "the debug file names both mods, and neither loads"
    );
    assert_eq!(
        scratch.hook_marker(),
        None,
        "the settings hook does not fire"
    );
    assert_eq!(turn.commands, Vec::<String>::new(), "no command is listed");
    assert!(
        turn.modes.iter().any(|mode| mode == "plan"),
        "the adapter's own SDK hook still reports plan mode"
    );
}

/// The resume route live (D5, plan claim 8): a session made isolated in one adapter process and
/// resumed in a second, with the same `_meta`, still loads neither mod. The first process runs
/// `session/new` and one short turn and ends; the second calls `session/resume` (or
/// `session/load` when only that is advertised, as the driver chooses) with its own debug file,
/// and runs one short turn.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn acp_isolated_on_resume() {
    let _one = ONE_AT_A_TIME.lock().await;
    let claude = claude_on_path();
    let scratch = Scratch::new(&claude);
    let row = acp_row(&scratch, false);

    let Some((mut first, _)) = acp_start(&scratch, &row).await else {
        return;
    };
    let session_id = acp_new(&mut first, &scratch, &row, &scratch.debug).await;
    let first_stop = acp_prompt(&mut first, &session_id, ACP_SHORT_PROMPT).await;
    first.close().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let resumed_debug = scratch.debug.with_file_name("debug-resumed.log");
    let Some((mut second, init)) = acp_start(&scratch, &row).await else {
        return;
    };
    let caps = init
        .pointer("/result/agentCapabilities")
        .cloned()
        .unwrap_or_default();
    let method = if caps.pointer("/sessionCapabilities/resume").is_some() {
        "session/resume"
    } else if caps.get("loadSession").and_then(Value::as_bool) == Some(true) {
        "session/load"
    } else {
        panic!("the adapter advertises neither resume nor load: {caps}")
    };
    let resumed = second
        .request(
            method,
            json!({
                "sessionId": session_id,
                "cwd": scratch.checkout,
                "mcpServers": acp_servers(&scratch),
                "_meta": acp_meta(&row, &resumed_debug),
            }),
        )
        .await;
    second.drain(Duration::from_secs(2)).await;
    let resumed_stop = acp_prompt(&mut second, &session_id, ACP_SHORT_PROMPT).await;
    let turn = acp_turn_of(&second, resumed_stop);
    second.close().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let resumed_mods = Scratch::mods_in(&resumed_debug);
    println!("== acp isolated, resumed in a second process over {method}");
    println!("  first: stop {first_stop:?}, mods {:?}", scratch.mods());
    println!("  {method} answered: {resumed}");
    println!("  resumed: stop {:?}, mods {resumed_mods:?}", turn.stop);
    println!("  commands after resume: {:?}", turn.commands);
    println!("  settings hook marker: {:?}", scratch.hook_marker());

    assert_eq!(
        first_stop.as_deref(),
        Some("end_turn"),
        "auth and the first turn work"
    );
    assert!(
        resumed.get("result").is_some(),
        "`{method}` restores the session: {resumed}"
    );
    assert_eq!(
        turn.stop.as_deref(),
        Some("end_turn"),
        "the resumed session takes a turn"
    );
    assert_eq!(
        resumed_mods,
        both_mods(false),
        "the resumed session's debug file names both mods, and neither loads"
    );
    assert_eq!(
        scratch.hook_marker(),
        None,
        "the settings hook fires in neither process"
    );
    assert_eq!(turn.commands, Vec::<String>::new(), "no command is listed");
}
