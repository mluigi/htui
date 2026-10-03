//! The CLI supervisor: the invocation it assembles, and the session it runs over a child
//! (plan T50, blueprint B.2 and F-T50).
//!
//! Two halves, and the split is the point. The first is **pure** — [`argv`] and [`usd`] are a list
//! and a string, and every claim §4.4 makes about the command line is checked here without a
//! process, which is what lets the cases be exhaustive about flag order and about the two
//! combinations the CLI refuses. The second drives a real child, and the child is a `/bin/sh`
//! script this file writes into a temporary directory (`acp_driver.rs`'s technique): the real
//! `claude` binary lives in `tests/cli_live.rs` behind `#[ignore]`, because a supervisor test that
//! needed a subscription login would be a test nobody runs and a CI nobody trusts.
//!
//! Blueprint H-7's fixture rule: no case here touches a seed row. Every row is synthetic and
//! declares its command directly, so a case can say exactly what the kernel was asked to start.
//! Nothing here is keyed on the row's **name** either (`R-AGT-5`) — the rows are called
//! `scripted-cli` precisely so that a supervisor that had started reading names would fail.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use chrono::Utc;
#[cfg(unix)]
use htui_agent::cli::ClaudeStreamAdapter;
#[cfg(unix)]
use htui_agent::cli::{SessionOptions, open_session};
use htui_agent::cli::{argv, usd};
#[cfg(unix)]
use htui_agent::driver::{AgentSession, PermissionAnswer, PermissionRequestId};
use htui_agent::driver::{
    AgentSessionRef, McpServerSpec, PermissionPolicy, SessionSpec, ToolExposure,
};
#[cfg(unix)]
use htui_agent::error::DriverError;
use htui_agent::event::ToolKind;
#[cfg(unix)]
use htui_agent::event::{
    DriverEvent, PermissionOption, PermissionOptionKind, PermissionRequestEvent, Stamp, StopReason,
    TerminalReason, ToolResultStatus,
};
use htui_agent::launch::CliSettings;
#[cfg(unix)]
use htui_agent::launch::{AgentSettings, ChildIo};
use htui_agent::prompt_bridge::{PROMPT_TOOL, bridge};
#[cfg(unix)]
use htui_agent::prompt_bridge::{PromptAsk, PromptCall, PromptClosed, PromptVerdict};
#[cfg(unix)]
use htui_agent::registry::DriverFactory;
#[cfg(unix)]
use htui_core::model::{Agent as AgentRow, Billing, Transport};
use htui_core::model::{AgentId, StepId};
#[cfg(unix)]
use serde_json::json;

/// A spec with nothing set: every case turns on exactly the fields it names.
fn spec(cwd: PathBuf) -> SessionSpec {
    SessionSpec {
        agent_id: AgentId::new(),
        step_id: StepId::new(),
        cwd,
        extra_dirs: Vec::new(),
        env: BTreeMap::new(),
        model: None,
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        retain_raw: false,
        resume: None,
        budget_micros: None,
        prompt: None,
    }
}

/// The `settings.cli` block a row declares (§5.2).
fn cli_settings(permission_mode: &str, extra_args: &[&str]) -> CliSettings {
    CliSettings {
        stream: htui_agent::cli::STREAM.to_owned(),
        permission_mode: permission_mode.to_owned(),
        extra_args: extra_args.iter().map(|arg| (*arg).to_owned()).collect(),
    }
}

// ---------------------------------------------------------------------------------------------
// The invocation (§4.4), with no process in sight
// ---------------------------------------------------------------------------------------------

/// Every flag §4.4 verified, in the order blueprint B.2 fixes, with the row's own arguments in
/// front and the operator's `extra_args` behind — so a repeated flag is settled by the operator's
/// copy, which is the one the CLI keeps.
#[test]
fn argv_is_the_ana4_line_in_order() {
    let mut spec = spec(PathBuf::from("/scratch"));
    spec.model = Some("sonnet".to_owned());
    spec.extra_dirs = vec![PathBuf::from("/a"), PathBuf::from("/b")];
    spec.budget_micros = Some(300);

    let args = argv(
        &["--row-arg".to_owned()],
        &cli_settings("acceptEdits", &["--x"]),
        &spec,
        "minted-id",
    );

    assert_eq!(
        args,
        vec![
            "--row-arg",
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "acceptEdits",
            "--session-id",
            "minted-id",
            "--model",
            "sonnet",
            "--add-dir",
            "/a",
            "--add-dir",
            "/b",
            "--max-budget-usd",
            "0.000300",
            "--x",
        ],
    );
    assert!(
        !args.contains(&"--bare".to_owned()),
        "`--bare` is the seed's business and D92 dropped it there; the supervisor never adds one"
    );
}

/// The prompt is not on the command line, and the case says why rather than only that.
///
/// `-p` takes a positional prompt and the temptation is to put it there. An argv is world-readable
/// through `ps` on a shared box, so the prompt travels on stdin as the first user message instead
/// (blueprint P-2). Nothing in [`argv`]'s signature can carry it, and this asserts that the flag
/// that would have is bare.
#[test]
fn the_prompt_has_no_place_on_the_command_line() {
    let args = argv(
        &[],
        &cli_settings("", &[]),
        &spec(PathBuf::from("/scratch")),
        "minted-id",
    );
    let after_p = args
        .iter()
        .position(|arg| arg == "-p")
        .map(|at| args[at + 1].clone())
        .expect("`-p` is always passed");
    assert_eq!(
        after_p, "--output-format",
        "`-p` is followed by the next flag, never by a prompt: {args:?}"
    );
}

/// `--session-id` and `--resume` are exclusive: the CLI refuses the pair (blueprint H-18), so the
/// spec's `resume` picks one and the minted id is simply unused.
#[test]
fn a_resuming_session_names_the_old_id_and_mints_nothing() {
    let mut spec = spec(PathBuf::from("/scratch"));
    spec.resume = Some(AgentSessionRef::new("older-session"));

    let args = argv(&[], &cli_settings("", &[]), &spec, "minted-id");

    assert!(
        args.windows(2)
            .any(|pair| pair == ["--resume", "older-session"]),
        "{args:?}"
    );
    assert!(
        !args.contains(&"--session-id".to_owned()) && !args.contains(&"minted-id".to_owned()),
        "the two flags are exclusive, so the mint does not travel beside the resume: {args:?}"
    );
}

/// A row that names no permission mode passes no flag: the CLI's own default is a better answer
/// than a mode `htui` guessed.
#[test]
fn an_empty_permission_mode_passes_no_flag() {
    let args = argv(
        &[],
        &cli_settings("", &[]),
        &spec(PathBuf::from("/scratch")),
        "minted-id",
    );
    assert!(!args.contains(&"--permission-mode".to_owned()), "{args:?}");
}

/// **F-10, and the reason this case exists at all.** `--max-budget-usd 0` is refused *before* the
/// CLI reads a byte of stdin — the recorded `claude_stream_json_budget_zero.jsonl` transcript is
/// two lines long, exit 1, no stdout whatsoever. A supervisor that passed the flag for an absent
/// cap would therefore turn "this project sets no per-run cap" into "this project cannot hold a
/// turn", and would do it silently.
#[test]
fn a_budget_that_is_absent_or_zero_or_negative_passes_no_flag() {
    for budget in [None, Some(0), Some(-1)] {
        let mut spec = spec(PathBuf::from("/scratch"));
        spec.budget_micros = budget;
        let args = argv(&[], &cli_settings("", &[]), &spec, "minted-id");
        assert!(
            !args.contains(&"--max-budget-usd".to_owned()),
            "budget {budget:?} must not reach the command line: {args:?}"
        );
    }
}

/// Micros → the decimal the flag takes, by integer arithmetic: the recorder's client-side cap and
/// the CLI's server-side one read one number, and a float round-trip is what would make them
/// disagree in the sixth place (D83, D90).
#[test]
fn usd_is_six_places_of_integer_arithmetic() {
    assert_eq!(usd(1_500_000), "1.500000");
    assert_eq!(usd(300), "0.000300");
    assert_eq!(usd(0), "0.000000");
    assert_eq!(usd(1), "0.000001", "the smallest cap the flag can express");
    assert_eq!(usd(1_000_000), "1.000000");
}

// ---------------------------------------------------------------------------------------------
// MOD-26 D11: a persona's narrowing on the command line
// ---------------------------------------------------------------------------------------------

fn tool_names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// An exposure naming `allow`, `deny` and `deny_kinds`, `command_run` off.
fn exposure(allow: &[&str], deny: &[&str], deny_kinds: &[ToolKind]) -> ToolExposure {
    ToolExposure {
        allow: tool_names(allow),
        deny: tool_names(deny),
        command_run: false,
        deny_kinds: deny_kinds.to_vec(),
    }
}

/// [`argv`] over a bare spec carrying `tools`, with `extra_args`.
fn narrowed_argv(tools: ToolExposure, extra_args: &[&str]) -> Vec<String> {
    let mut spec = spec(PathBuf::from("/scratch"));
    spec.tools = tools;
    argv(&[], &cli_settings("", extra_args), &spec, "minted-id")
}

/// D11's position: both flags come right after the last pair and before the operator's
/// `extra_args`, each as one `=`-joined argument, and `deny_kinds` is inverted onto the deny list
/// after the names it already carries.
#[test]
fn persona_flags_follow_the_pairs_and_precede_extra_args() {
    let args = narrowed_argv(
        exposure(&["Read", "Grep"], &["mcp__x__y"], &[ToolKind::Execute]),
        &["--foo"],
    );
    assert_eq!(
        args,
        vec![
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--session-id",
            "minted-id",
            "--tools=Read,Grep",
            "--disallowedTools=mcp__x__y,Bash,BashOutput,KillShell",
            "--foo",
        ],
    );
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--session-id", "minted-id"]),
        "the last pair is intact: {args:?}"
    );
}

/// I-1: `--allowedTools` auto-approves (probed), so no exposure ever produces it.
#[test]
fn allowed_tools_is_never_emitted() {
    let grid = [
        ToolExposure::default(),
        exposure(&["Read"], &[], &[]),
        exposure(&[], &["Bash"], &[]),
        exposure(&[], &[], &[ToolKind::Edit, ToolKind::Delete]),
        exposure(&["Read", "Grep"], &["mcp__x__y"], ToolKind::ALL),
    ];
    for tools in grid {
        let args = narrowed_argv(tools.clone(), &[]);
        assert!(
            !args.iter().any(|arg| arg.starts_with("--allowedTools")),
            "{tools:?} → {args:?}"
        );
    }
}

/// D11: `--tools=""` would disable every tool, so an empty allow-list passes no `--tools` at all.
#[test]
fn tools_is_absent_when_allow_is_empty() {
    let args = narrowed_argv(exposure(&[], &["Bash"], &[ToolKind::Read]), &[]);
    assert!(
        !args.iter().any(|arg| arg.starts_with("--tools")),
        "{args:?}"
    );
    assert!(
        args.iter().any(|arg| arg.starts_with("--disallowedTools=")),
        "the deny list still travels: {args:?}"
    );
}

/// D11: `deny_kinds: [edit]` is the four claude names that edit a file.
#[test]
fn deny_kinds_edit_yields_the_four_claude_names() {
    let args = narrowed_argv(exposure(&[], &[], &[ToolKind::Edit]), &[]);
    assert_eq!(
        args.last().map(String::as_str),
        Some("--disallowedTools=Edit,Write,MultiEdit,NotebookEdit"),
        "{args:?}"
    );
}

/// MOD-11 D17: an exposed `command_run` denies every OQ-5 prefix as `Bash(<prefix>:*)`, after
/// the names the exposure already denies, in the list's order.
#[test]
fn command_run_exposure_denies_the_heavy_bash_prefixes() {
    let mut tools = exposure(&[], &["mcp__x__y", "Bash(make:*)"], &[]);
    tools.command_run = true;
    let args = narrowed_argv(tools, &[]);
    let mut expected = vec!["mcp__x__y".to_owned(), "Bash(make:*)".to_owned()];
    for prefix in htui_core::model::kind::HEAVY_COMMAND_PREFIXES {
        let name = format!("Bash({prefix}:*)");
        if !expected.contains(&name) {
            expected.push(name);
        }
    }
    assert_eq!(
        expected.len(),
        2 + 14,
        "`make` was already denied: kept once"
    );
    assert_eq!(
        args.last().cloned(),
        Some(format!("--disallowedTools={}", expected.join(","))),
        "{args:?}"
    );
    assert!(
        args.iter()
            .any(|arg| arg.contains("Bash(cargo build:*)") && arg.contains("Bash(go test:*)")),
        "{args:?}"
    );
}

/// MOD-11 D17: no exposure, no Bash denial — a persona's deny list travels as it was.
#[test]
fn no_exposure_no_bash_denials() {
    let args = narrowed_argv(exposure(&[], &["mcp__x__y"], &[]), &[]);
    assert_eq!(
        args.last().map(String::as_str),
        Some("--disallowedTools=mcp__x__y"),
        "{args:?}"
    );
    let args = narrowed_argv(ToolExposure::default(), &[]);
    assert!(!args.iter().any(|arg| arg.contains("Bash(")), "{args:?}");
}

/// I-7: a step with no persona gets exactly today's command line.
#[test]
fn a_default_exposure_adds_no_flag() {
    let args = narrowed_argv(ToolExposure::default(), &[]);
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("--tools") || arg.starts_with("--disallowedTools")),
        "{args:?}"
    );
    assert_eq!(
        args.last().map(String::as_str),
        Some("minted-id"),
        "{args:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// MOD-11 D8: `htui`'s MCP server on the command line
// ---------------------------------------------------------------------------------------------

/// The server `htui` registers for a session: the binary, `mcp`, the address and the token.
fn htui_server() -> McpServerSpec {
    McpServerSpec {
        name: "htui".to_owned(),
        command: "/abs/htui".to_owned(),
        args: vec!["mcp".to_owned()],
        env: BTreeMap::from([
            ("HTUI_MCP_TOKEN".to_owned(), "token-value".to_owned()),
            (
                "HTUI_MCP_ADDR".to_owned(),
                "/run/htui-mcp-1-abcd/s".to_owned(),
            ),
        ]),
    }
}

/// A spec that sets every field `argv` reads: model, directories, budget and a narrowing.
fn full_spec() -> SessionSpec {
    let mut spec = spec(PathBuf::from("/scratch"));
    spec.model = Some("sonnet".to_owned());
    spec.extra_dirs = vec![PathBuf::from("/a")];
    spec.budget_micros = Some(300);
    spec.tools = exposure(&["Read", "Grep"], &["mcp__x__y"], &[]);
    spec
}

/// The `--mcp-config=` arguments of `args`.
fn mcp_configs(args: &[String]) -> Vec<&String> {
    args.iter()
        .filter(|arg| arg.starts_with("--mcp-config"))
        .collect()
}

/// I-7: a spec with no server gets exactly the pre-MOD-11 command line, flag for flag.
#[test]
fn argv_without_mcp_is_unchanged() {
    let args = argv(
        &["--row-arg".to_owned()],
        &cli_settings("acceptEdits", &["--x"]),
        &full_spec(),
        "minted-id",
    );
    assert_eq!(
        args,
        vec![
            "--row-arg",
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "acceptEdits",
            "--session-id",
            "minted-id",
            "--model",
            "sonnet",
            "--add-dir",
            "/a",
            "--max-budget-usd",
            "0.000300",
            "--tools=Read,Grep",
            "--disallowedTools=mcp__x__y",
            "--x",
        ],
    );
    assert!(mcp_configs(&args).is_empty(), "{args:?}");
    assert_eq!(htui_agent::cli::mcp_config(&[]), None);
}

/// D8: one `=`-joined `--mcp-config` argument (the CLI's variadic parse cannot swallow the next
/// token), after the last pair and before `--tools`, which is left as it was.
#[test]
fn argv_with_mcp_has_one_joined_config_before_tools() {
    let mut with = full_spec();
    with.mcp = vec![htui_server()];
    let args = argv(&[], &cli_settings("", &[]), &with, "minted-id");
    let without = argv(&[], &cli_settings("", &[]), &full_spec(), "minted-id");

    let configs = mcp_configs(&args);
    assert_eq!(configs.len(), 1, "{args:?}");
    assert!(configs[0].starts_with("--mcp-config="), "{args:?}");
    let config_at = args
        .iter()
        .position(|arg| arg.starts_with("--mcp-config="))
        .expect("the config is there");
    let tools_at = args
        .iter()
        .position(|arg| arg.starts_with("--tools="))
        .expect("the allow-list is there");
    assert!(config_at < tools_at, "{args:?}");
    assert_eq!(
        args[config_at - 2..config_at],
        ["--max-budget-usd", "0.000300"],
        "right after the last pair: {args:?}"
    );
    assert_eq!(args[tools_at], "--tools=Read,Grep", "--tools is untouched");
    let rest: Vec<&String> = args
        .iter()
        .filter(|arg| !arg.starts_with("--mcp-config"))
        .collect();
    assert_eq!(
        rest,
        without.iter().collect::<Vec<_>>(),
        "the config is the only addition"
    );
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("--strict-mcp-config")),
        "the operator's own servers stay: {args:?}"
    );
}

/// D8: the JSON is the CLI's own `mcpServers` shape — keyed by name, `type: "stdio"`, `env` an
/// object — and serialises compactly in key order.
#[test]
fn the_mcp_config_is_the_clis_stdio_shape() {
    let config = htui_agent::cli::mcp_config(&[htui_server()]).expect("one server, one config");
    let parsed: serde_json::Value = serde_json::from_str(&config).expect("the config is JSON");
    assert_eq!(
        parsed,
        serde_json::json!({
            "mcpServers": {
                "htui": {
                    "type": "stdio",
                    "command": "/abs/htui",
                    "args": ["mcp"],
                    "env": {
                        "HTUI_MCP_ADDR": "/run/htui-mcp-1-abcd/s",
                        "HTUI_MCP_TOKEN": "token-value",
                    },
                },
            },
        }),
    );
    assert!(
        !config.contains('\n') && !config.contains(": "),
        "compact: {config}"
    );
    assert!(
        config.find("HTUI_MCP_ADDR") < config.find("HTUI_MCP_TOKEN"),
        "env in key order: {config}"
    );

    let mut spec = spec(PathBuf::from("/scratch"));
    spec.mcp = vec![htui_server()];
    let args = argv(&[], &cli_settings("", &[]), &spec, "minted-id");
    assert_eq!(
        mcp_configs(&args),
        [&format!("--mcp-config={config}")],
        "argv carries exactly that JSON"
    );
}

/// D8: the operator's `extra_args` still come last, so a repeated flag is theirs.
#[test]
fn extra_args_stay_last_with_mcp() {
    let mut spec = full_spec();
    spec.mcp = vec![htui_server()];
    let args = argv(
        &[],
        &cli_settings("", &["--mcp-config=operator.json", "--y"]),
        &spec,
        "minted-id",
    );
    assert_eq!(
        args[args.len() - 2..],
        ["--mcp-config=operator.json", "--y"],
        "{args:?}"
    );
    assert!(
        args.iter()
            .position(|arg| arg.starts_with("--disallowedTools="))
            .expect("the deny list travels")
            < args.len() - 2,
        "{args:?}"
    );
}

/// MOD-11 D18: a spec with a prompt port names `htui`'s prompt tool as one flag pair right after
/// the `--mcp-config=` argument; a spec without one passes no such flag at all.
#[test]
fn argv_names_the_prompt_tool_only_with_a_port() {
    let mut with = full_spec();
    with.mcp = vec![htui_server()];
    let without = argv(&[], &cli_settings("", &["--y"]), &with, "minted-id");
    assert!(
        !without.contains(&"--permission-prompt-tool".to_owned()),
        "no port, no prompt tool: {without:?}"
    );

    let (port, _ask) = bridge();
    with.prompt = Some(port);
    let args = argv(&[], &cli_settings("", &["--y"]), &with, "minted-id");
    let config_at = args
        .iter()
        .position(|arg| arg.starts_with("--mcp-config="))
        .expect("the config is there");
    assert_eq!(
        args[config_at + 1..config_at + 3],
        ["--permission-prompt-tool", "mcp__htui__permission_prompt"],
        "the pair follows the config: {args:?}"
    );
    assert_eq!(PROMPT_TOOL, "mcp__htui__permission_prompt");
    let rest: Vec<&String> = args
        .iter()
        .filter(|arg| *arg != "--permission-prompt-tool" && *arg != PROMPT_TOOL)
        .collect();
    assert_eq!(
        rest,
        without.iter().collect::<Vec<_>>(),
        "the pair is the only addition"
    );
    assert_eq!(
        args.last().map(String::as_str),
        Some("--y"),
        "extra_args last"
    );
}

// ---------------------------------------------------------------------------------------------
// A session over a scripted agent
// ---------------------------------------------------------------------------------------------

/// How long a signalled process is given to stop being one.
///
/// `acp_driver.rs`'s window and its reasoning: a kill takes single-digit milliseconds on this box,
/// and a bound three orders of magnitude larger turns a regression into a named failure in two
/// seconds rather than a hung suite.
#[cfg(unix)]
const KILL_WINDOW: Duration = Duration::from_secs(2);

/// How long a case waits for an event before calling the supervisor stuck.
///
/// Every script here answers in milliseconds; this exists so a supervisor that stopped forwarding
/// fails with the name of the event it never produced instead of hanging the suite.
#[cfg(unix)]
const EVENT_WINDOW: Duration = Duration::from_secs(10);

/// Fails unless `pid` has been **reaped** within [`KILL_WINDOW`]: no `/proc/{pid}` at all, which a
/// zombie still has.
///
/// The strong form, and every case here is entitled to it: the cancel path awaits
/// `ChildGuard::kill_and_reap` before it acknowledges, and the handle joins the task before
/// `cancel` returns. `acp_driver.rs` carries the same helper for the same reason it is copied
/// rather than shared — a case's own assertion in its own file is what lets one change without
/// the others being re-read.
#[cfg(unix)]
async fn assert_reaped(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + KILL_WINDOW;
        while Path::new(&format!("/proc/{pid}")).exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: pid {pid} was not reaped"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

/// Fails unless `pid` is gone **or** reaped-pending within [`KILL_WINDOW`].
///
/// The weaker form, for the one exit that runs no code of ours: an aborted task's `ChildGuard`
/// `Drop` can only *signal*, so a zombie is the honest expectation there.
#[cfg(unix)]
async fn assert_not_running(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + KILL_WINDOW;
        loop {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return;
            };
            // `pid (comm) state …`, and `comm` may hold spaces and parens: the state is the first
            // field after the **last** `)`.
            let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
            let state = after.trim().chars().next().unwrap_or('Z');
            if state == 'Z' || state == 'X' {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: pid {pid} is still running (state {state})"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, what);
    }
}

/// Writes `contents` as an executable file, creating its directory.
#[cfg(unix)]
fn executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A synthetic `cli` row naming `command` directly.
///
/// Directly, and not through a `${tool}` placeholder: this file is about the supervisor, and a
/// discovery document would put `tools::resolve` between the case and the thing it is asserting.
/// The name is **not** an agent's brand on purpose (`R-AGT-5`): everything that selects this
/// transport comes out of `transport` and `settings.cli.stream`, and a supervisor that had started
/// reading `agent.name` would still pass if the row were called what the dialect is called.
#[cfg(unix)]
fn scripted_row(command: &Path, env: serde_json::Map<String, serde_json::Value>) -> AgentRow {
    let now = Utc::now();
    AgentRow {
        id: AgentId::new(),
        name: "scripted-cli".to_owned(),
        transport: Transport::Cli,
        launch: json!({
            "command": command.to_string_lossy(),
            "args": [],
            "env": env,
            "discovery": { "handshake": false, "tools": {} },
        }),
        models: vec!["row-model".to_owned()],
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({
            "cli": {
                "stream": htui_agent::cli::STREAM,
                "permission_mode": "acceptEdits",
                "extra_args": [],
            },
            "usage": { "scope": "model_usage" },
        }),
        created_at: now,
        updated_at: now,
    }
}

/// A session over `row`, through the factory — so the registry's own `cli/<stream>` derivation and
/// the capability profile it computes are part of every case rather than bypassed by them.
#[cfg(unix)]
async fn start(row: &AgentRow, cwd: &Path) -> Box<dyn AgentSession> {
    start_with(row, spec(cwd.to_path_buf())).await
}

/// [`start`] with a spec the case built (MOD-11 D18: one carrying a prompt port).
#[cfg(unix)]
async fn start_with(row: &AgentRow, spec: SessionSpec) -> Box<dyn AgentSession> {
    let mut factory = DriverFactory::new();
    factory.register(
        htui_agent::cli::ADAPTER_ID,
        Box::new(ClaudeStreamAdapter) as Box<dyn htui_agent::registry::TransportBuilder>,
    );
    let driver = factory
        .driver_for(row, None)
        .expect("the `cli/claude_stream_json` adapter builds this row");
    assert!(
        !driver.caps().permission_requests && !driver.caps().edit_proposals && !driver.caps().plans,
        "§4.3's triple is what the chat tab draws its banner from: {:?}",
        driver.caps()
    );
    driver
        .start(spec, "hello".to_owned())
        .await
        .expect("the scripted agent sends its `system/init`")
}

/// The next event, or a named failure rather than a hung suite.
#[cfg(unix)]
async fn next(session: &mut Box<dyn AgentSession>, what: &str) -> DriverEvent {
    match tokio::time::timeout(EVENT_WINDOW, session.next_event()).await {
        Ok(Ok(Some(envelope))) => envelope.event,
        Ok(Ok(None)) => panic!("the session ended while waiting for {what}"),
        Ok(Err(err)) => panic!("waiting for {what}: {err}"),
        Err(_) => panic!("nothing arrived within the window while waiting for {what}"),
    }
}

/// Every remaining event until the session ends.
#[cfg(unix)]
async fn drain(session: &mut Box<dyn AgentSession>) -> Vec<DriverEvent> {
    let mut events = Vec::new();
    loop {
        match tokio::time::timeout(EVENT_WINDOW, session.next_event()).await {
            Ok(Ok(Some(envelope))) => events.push(envelope.event),
            Ok(Ok(None)) => return events,
            Ok(Err(err)) => panic!("draining the session: {err}"),
            Err(_) => panic!("the session never ended; drained {events:?}"),
        }
    }
}

/// The banner's body, or a failure naming what arrived instead.
#[cfg(unix)]
fn banner_body(event: &DriverEvent) -> &serde_json::Value {
    match event {
        DriverEvent::Other(other) if other.update == "session_started" => &other.body,
        other => panic!("the session banner is the step's first row, got {other:?}"),
    }
}

/// The `system/init` line, with the keys the recorded transcripts carry — `claude_code_version`
/// and no `version`, which is the sort of thing only a fixture can settle.
#[cfg(unix)]
const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"the-cli-picked-this","claude_code_version":"9.9.9","model":"scripted-model"}"#;

/// A terminal `result`, successful, with a cost and one model's tokens.
#[cfg(unix)]
const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","total_cost_usd":0.000002,"modelUsage":{"m":{"inputTokens":1,"outputTokens":2,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}}}"#;

/// One assistant message carrying the word `ok`.
#[cfg(unix)]
const REPLY: &str =
    r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"ok"}]}}"#;

/// The tool-use id of the recorded probe's gated call (blueprint §2.7).
#[cfg(unix)]
const TOOL_USE_ID: &str = "toolu_01HuMymhasmoPmGaLNFxJ4GV";

/// The assistant line announcing that call, as the CLI streams it before it asks.
#[cfg(unix)]
const TOOL_USE: &str = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_01HuMymhasmoPmGaLNFxJ4GV","name":"mcp__htui__hello","input":{}}]}}"#;

/// [`RESULT`] carrying the probe's `permission_denials[]` entry for that call (blueprint §2.7):
/// what the CLI repeats at the end of a turn whose prompt was answered `deny`.
#[cfg(unix)]
const DENIED_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"terminal_reason":"completed","total_cost_usd":0.000002,"modelUsage":{"m":{"inputTokens":1,"outputTokens":2,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}},"permission_denials":[{"tool_name":"mcp__htui__hello","tool_use_id":"toolu_01HuMymhasmoPmGaLNFxJ4GV","tool_input":{}}]}"#;

/// What every script records about itself before it says a word: its argv, and its own pid.
///
/// The pid is the group leader's, and it is the one identifier a kill can be checked against that
/// nothing else can accidentally answer to — a `pgrep` pattern matches any command line that
/// merely contains the text, including the shell that ran the check (`launch.rs`'s `pid()` doc).
#[cfg(unix)]
const RECORD: &str = concat!(
    "printf '%s\n' \"$@\" > \"$HTUI_ARGV_FILE\"\n",
    "echo $$ > \"$HTUI_PID_FILE\"\n",
);

/// A template with its placeholders filled in.
///
/// The envelopes and the self-recording preamble live in one place so a case reads as a *script* —
/// what the agent does, in order — rather than as three hundred columns of JSON with a `while` loop
/// hidden in the middle.
#[cfg(unix)]
fn script(template: &str) -> String {
    template
        .replace("<RECORD>", RECORD)
        .replace("<INIT>", INIT)
        .replace("<RESULT>", RESULT)
        .replace("<REPLY>", REPLY)
        .replace("<TOOL_USE>", TOOL_USE)
        .replace("<DENIED_RESULT>", DENIED_RESULT)
        .replace("<FLOOD>", &FLOOD.to_string())
}

/// Answers every stdin line with a whole turn, and exits when stdin closes.
///
/// It records its own argv first, so one case can assert what the kernel was asked to start
/// without a second fixture. The hook envelope precedes `system/init` because that is what the
/// real CLI does without `--bare` (plan F-9), and the supervisor's pre-`init` buffer is what has to
/// put it *behind* the banner.
#[cfg(unix)]
const TURN_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '{"type":"system","subtype":"hook_started","hook_name":"SessionStart"}'
printf '%s\n' '<INIT>'
while IFS= read -r line; do
  printf '%s\n' '<REPLY>'
  printf '%s\n' '<RESULT>'
done
exit 0
"#;

/// Announces a tool call and then **dies mid-turn**, which is the milestone-3 defect class.
#[cfg(unix)]
const EOF_MID_TURN_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":"ls"}}]}}'
exit 0
"#;

/// Finishes its turn and *then* exits: the stream ends with nothing open (plan F-1).
#[cfg(unix)]
const EOF_BETWEEN_TURNS_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '<REPLY>'
printf '%s\n' '<RESULT>'
exit 0
"#;

/// Runs forever until it is interrupted, and answers the interrupt with a **late** terminal
/// `result` — the shape plan F-2 measured, minus the error dress, so a drained `done` is
/// distinguishable from a synthesized one by its stop reason alone.
#[cfg(unix)]
const SLOW_CANCEL_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
on_int() {
  sleep 0.2
  printf '%s\n' '<RESULT>'
  exit 0
}
trap on_int INT
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '<REPLY>'
while true; do sleep 0.1; done
"#;

/// Writes a line that is not JSON and a line that ends CRLF, and then finishes its turn
/// normally: neither may end the stream (blueprint H-23).
#[cfg(unix)]
const NOISY_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' 'npm warn: this is not an envelope'
printf '%s\r\n' '<REPLY>'
printf '%s\n' '<RESULT>'
exit 0
"#;

/// MOD-11 D18: announces [`TOOL_USE`] and then waits — as the CLI waits on its
/// `--permission-prompt-tool` — until the case creates `$HTUI_GO_FILE`; then ends the turn with
/// [`DENIED_RESULT`] and exits.
#[cfg(unix)]
const PROMPT_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '<TOOL_USE>'
while [ ! -f "$HTUI_GO_FILE" ]; do sleep 0.05; done
printf '%s\n' '<DENIED_RESULT>'
exit 0
"#;

/// [`PROMPT_SCRIPT`] answering the interrupt with [`DENIED_RESULT`] — F-2's real `result`, carrying
/// the `permission_denials[]` entry for the call the cancel denied.
#[cfg(unix)]
const PROMPT_CANCEL_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
on_int() {
  printf '%s\n' '<DENIED_RESULT>'
  exit 0
}
trap on_int INT
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '<TOOL_USE>'
while true; do sleep 0.05; done
"#;

/// How many assistant lines [`FLOOD_PROMPT_SCRIPT`] streams while its prompt is parked: well past
/// [`htui_agent::cli::EVENTS_CAPACITY`], so the session task blocks on a full event channel.
#[cfg(unix)]
const FLOOD: usize = 1000;

/// [`PROMPT_SCRIPT`], except that once `$HTUI_GO_FILE` exists it streams [`FLOOD`] replies — as a
/// CLI keeps streaming parallel work while one gated call waits — and only ends its turn once
/// `$HTUI_GO_FILE.end` exists too.
#[cfg(unix)]
const FLOOD_PROMPT_SCRIPT: &str = r#"#!/bin/sh
<RECORD>
printf '%s\n' '<INIT>'
IFS= read -r line
printf '%s\n' '<TOOL_USE>'
while [ ! -f "$HTUI_GO_FILE" ]; do sleep 0.05; done
i=0
while [ "$i" -lt <FLOOD> ]; do printf '%s\n' '<REPLY>'; i=$((i+1)); done
while [ ! -f "$HTUI_GO_FILE.end" ]; do sleep 0.05; done
printf '%s\n' '<RESULT>'
exit 0
"#;

/// A row over a script written into `dir`, plus the two files it records about itself.
#[cfg(unix)]
struct Scripted {
    row: AgentRow,
    /// Where the script writes its own argv, one argument per line.
    argv: PathBuf,
    /// Where the script writes its own pid.
    pid: PathBuf,
}

#[cfg(unix)]
fn scripted(dir: &Path, body: &str) -> Scripted {
    let command = dir.join("bin/scripted-agent");
    let argv = dir.join("argv");
    let pid = dir.join("pid");
    executable(&command, &script(body));
    let env = serde_json::Map::from_iter([
        ("HTUI_ARGV_FILE".to_owned(), json!(argv.to_string_lossy())),
        ("HTUI_PID_FILE".to_owned(), json!(pid.to_string_lossy())),
        (
            "HTUI_GO_FILE".to_owned(),
            json!(dir.join("go").to_string_lossy()),
        ),
    ]);
    Scripted {
        row: scripted_row(&command, env),
        argv,
        pid,
    }
}

/// The pid the script recorded for itself, once it has got that far.
#[cfg(unix)]
fn recorded_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .expect("the script recorded its pid")
        .trim()
        .parse()
        .expect("a pid is a number")
}

/// A turn, start to finish, in the order §4.4 and §6.2 fix: the banner first, then whatever came
/// before it, then the reply, then the turn's one `usage`, then exactly one `done`.
#[tokio::test]
#[cfg(unix)]
async fn a_session_over_a_scripted_agent_streams_a_turn_and_then_a_follow_up() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), TURN_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let banner = next(&mut session, "the banner").await;
    let body = banner_body(&banner);
    assert_eq!(
        body["session_id"],
        json!(
            session
                .session_ref()
                .expect("a CLI session has an id")
                .as_str()
        ),
        "the id is `htui`'s mint, not the one the script reported (D84, blueprint H-17): {body}"
    );
    assert_eq!(
        body["protocol_version"],
        json!(null),
        "the stream negotiates nothing, and a `1` copied from ACP would claim a handshake that \
         never happened: {body}"
    );
    assert_eq!(body["agent_name"], json!("scripted-cli"), "{body}");
    assert_eq!(
        body["agent_version"],
        json!("9.9.9"),
        "`system/init.claude_code_version`, which is the key the wire actually carries: {body}"
    );
    assert_eq!(
        body["models"],
        json!(["scripted-model"]),
        "the stream says which model it selected, and that beats the row's list: {body}"
    );

    assert!(
        matches!(
            next(&mut session, "the buffered hook line").await,
            DriverEvent::Other(other) if other.update == "system/hook_started"
        ),
        "F-9: a hook envelope that arrived before `init` is released *behind* the banner, so the \
         banner is still the step's first `other` row"
    );

    let follow_up_too_early = session.send_follow_up("again".to_owned()).await;
    assert!(
        matches!(
            follow_up_too_early,
            Err(DriverError::Transport(ref message))
                if message.contains("would interleave two turns")
        ),
        "a follow-up before the turn's `done` is refused: {follow_up_too_early:?}"
    );

    assert!(matches!(
        next(&mut session, "the reply").await,
        DriverEvent::AssistantChunk(chunk) if chunk.text == "ok"
    ));
    let usage = next(&mut session, "the turn's usage").await;
    assert!(
        matches!(&usage, DriverEvent::Usage(usage) if usage.cost_micros == Some(2)),
        "the turn's one `usage` row, cost as micros: {usage:?}"
    );
    assert!(matches!(
        next(&mut session, "the turn's done").await,
        DriverEvent::Done(done) if done.stop_reason == StopReason::EndTurn
    ));

    session
        .send_follow_up("again".to_owned())
        .await
        .expect("after the `done` a follow-up opens the next turn");
    assert!(matches!(
        next(&mut session, "the second turn's reply").await,
        DriverEvent::AssistantChunk(_)
    ));
    let _usage = next(&mut session, "the second turn's usage").await;
    assert!(matches!(
        next(&mut session, "the second turn's done").await,
        DriverEvent::Done(_)
    ));

    let argv = std::fs::read_to_string(&scripted.argv).expect("the script recorded its argv");
    let seen: Vec<&str> = argv.lines().collect();
    assert!(
        seen.windows(2)
            .any(|pair| pair == ["--output-format", "stream-json"]),
        "the argv the kernel saw is §4.4's line: {argv}"
    );
    let minted = session
        .session_ref()
        .expect("a CLI session has an id")
        .as_str()
        .to_owned();
    assert!(
        seen.windows(2)
            .any(|pair| pair == ["--session-id", minted.as_str()]),
        "D84 end to end: the id `session_ref` hands a later step's `--resume` is the one this \
         process was started with, not one the agent reported back: {argv}"
    );
    assert!(
        !argv.contains("hello"),
        "and the prompt is not on it — it went out on stdin (blueprint P-2): {argv}"
    );

    // Between turns, so there is no turn to cancel: this ends the session.
    session.cancel(Duration::ZERO).await.expect("cancel");
    assert!(
        matches!(
            tokio::time::timeout(EVENT_WINDOW, session.next_event()).await,
            Ok(Ok(None))
        ),
        "a cancel between turns ends the session rather than synthesizing a turn's `done`"
    );
}

/// Once the session has ended, every operation but a pull answers `Closed` — including the one this
/// transport does not have at all.
#[tokio::test]
#[cfg(unix)]
async fn answer_permission_is_unsupported_and_closed_first_once_the_session_has_ended() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), TURN_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let refused = session
        .answer_permission(
            PermissionRequestId::new("whatever"),
            PermissionAnswer::Cancelled,
        )
        .await;
    assert!(
        matches!(refused, Err(DriverError::Unsupported("answer_permission"))),
        "this transport has no permission channel at all, which is a statement about the \
         *operation* rather than about the id (§4.3): {refused:?}"
    );

    session.cancel(Duration::ZERO).await.expect("cancel");
    while let Ok(Ok(Some(_))) = tokio::time::timeout(EVENT_WINDOW, session.next_event()).await {}

    let closed = session
        .answer_permission(
            PermissionRequestId::new("whatever"),
            PermissionAnswer::Cancelled,
        )
        .await;
    assert!(
        matches!(closed, Err(DriverError::Closed)),
        "a session that is over should not be arguing about an operation it never had the chance \
         to refuse: {closed:?}"
    );
}

/// **The milestone-3 defect class, closed** (`682a423`: "a stream ending before its `done` was
/// recorded as a finished turn"). A child that dies with a turn open owes three things, in this
/// order: an `error` naming the transport, a synthesized `failed` result for every call still
/// open, and exactly one `done { cancelled }`.
///
/// The order is the assertion. A `done` before the `error` would let a replay read the turn as
/// having ended cleanly and then met some unrelated trouble; a missing `tool_result` leaves the
/// call spinning in the transcript forever; a second `done` breaks "exactly one per turn", which
/// is what `send_follow_up` gates on.
#[tokio::test]
#[cfg(unix)]
async fn an_eof_mid_turn_is_an_error_a_closed_call_and_one_cancelled_done() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), EOF_MID_TURN_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let _banner = next(&mut session, "the banner").await;
    assert!(matches!(
        next(&mut session, "the tool call").await,
        DriverEvent::ToolCall(call) if call.tool_call_id == "call-1"
    ));

    let rest = drain(&mut session).await;
    let kinds: Vec<&str> = rest
        .iter()
        .map(|event| match event {
            DriverEvent::Error(_) => "error",
            DriverEvent::ToolResult(_) => "tool_result",
            DriverEvent::Done(_) => "done",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["error", "tool_result", "done"],
        "an EOF mid-turn is an error, then the open call closed, then one `done`: {rest:?}"
    );
    assert!(matches!(
        &rest[0],
        DriverEvent::Error(error) if error.code == "transport_closed"
    ));
    assert!(matches!(
        &rest[1],
        DriverEvent::ToolResult(result)
            if result.tool_call_id == "call-1"
                && result.status == ToolResultStatus::Failed
                && result.terminal_reason == Some(TerminalReason::Cancelled)
    ));
    assert!(matches!(
        &rest[2],
        DriverEvent::Done(done) if done.stop_reason == StopReason::Cancelled
    ));
}

/// The other half of F-1: a stream that stops when **nothing is open** is a session that finished,
/// not a turn that broke. The CLI runs its open turn to completion and exits 0 once its stdin
/// closes, so an EOF here owes no `error` and no second `done` — only `Ok(None)`.
#[tokio::test]
#[cfg(unix)]
async fn an_eof_between_turns_ends_the_session_and_says_nothing_else() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), EOF_BETWEEN_TURNS_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let _banner = next(&mut session, "the banner").await;
    let rest = drain(&mut session).await;

    assert_eq!(
        rest.iter()
            .filter(|event| matches!(event, DriverEvent::Done(_)))
            .count(),
        1,
        "one turn, one `done`: {rest:?}"
    );
    assert!(
        !rest
            .iter()
            .any(|event| matches!(event, DriverEvent::Error(_))),
        "a clean exit between turns is not a transport failure: {rest:?}"
    );
}

/// **D-2's ordering, and the reason the drain comes before anything is synthesized.**
///
/// The cancel closes stdin, interrupts the group and then *reads* for the grace window. A `result`
/// that arrives in that window is the turn's real ending and is reported as such — here the script
/// answers its interrupt 200 ms late with a `completed` result, so the `done` the caller sees
/// carries **the script's** stop reason and could not have been invented on this side. And there is
/// exactly one: `close_turn` is a no-op on a turn the drain already closed.
///
/// The pid assertion is §11 criterion 11's CLI half: `cancel` returning means the tree is gone,
/// not that a kill is on its way.
#[tokio::test]
#[cfg(unix)]
async fn a_cancel_drains_a_late_result_and_reports_its_done_rather_than_a_second_one() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), SLOW_CANCEL_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let _banner = next(&mut session, "the banner").await;
    assert!(matches!(
        next(&mut session, "the first chunk").await,
        DriverEvent::AssistantChunk(_)
    ));
    let pid = recorded_pid(&scripted.pid);

    session
        .cancel(Duration::from_secs(5))
        .await
        .expect("cancel");
    assert_reaped(pid, "the cancelled session's child").await;

    let rest = drain(&mut session).await;
    let dones: Vec<StopReason> = rest
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Done(done) => Some(done.stop_reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        dones,
        vec![StopReason::EndTurn],
        "the drained `result` is the turn's real end, and nothing synthesized a second one: \
         {rest:?}"
    );
    assert!(
        rest.iter()
            .any(|event| matches!(event, DriverEvent::Usage(_))),
        "and its usage came with it: {rest:?}"
    );
}

/// The same cancel with **no** grace: there is nothing to drain, so the turn ends the only way it
/// can — one synthesized `done { cancelled }` — and the script's later `result` never lands,
/// because the kill took the choice away.
#[tokio::test]
#[cfg(unix)]
async fn a_cancel_with_no_grace_synthesizes_exactly_one_cancelled_done() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), SLOW_CANCEL_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let _banner = next(&mut session, "the banner").await;
    assert!(matches!(
        next(&mut session, "the first chunk").await,
        DriverEvent::AssistantChunk(_)
    ));
    let pid = recorded_pid(&scripted.pid);

    session.cancel(Duration::ZERO).await.expect("cancel");
    assert_reaped(pid, "the cancelled session's child").await;

    let rest = drain(&mut session).await;
    let dones: Vec<StopReason> = rest
        .iter()
        .filter_map(|event| match event {
            DriverEvent::Done(done) => Some(done.stop_reason),
            _ => None,
        })
        .collect();
    assert_eq!(dones, vec![StopReason::Cancelled], "{rest:?}");
    assert!(
        !rest
            .iter()
            .any(|event| matches!(event, DriverEvent::Usage(_))),
        "the script's late `result` never arrived, so nothing invented its cost: {rest:?}"
    );
}

/// **H-23.** A line that is not JSON is an event, not an ending, and a line that arrives CRLF is
/// the same line.
///
/// `lines()` would have ended this stream twice over — once on the unparseable line if the reader
/// had insisted on JSON, and once on the first non-UTF-8 byte it answers `InvalidData` for. Either
/// would have been recorded as a transport that closed mid-turn, which is the one failure this
/// transport must never invent.
#[tokio::test]
#[cfg(unix)]
async fn noise_and_a_carriage_return_do_not_end_the_stream() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), NOISY_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;

    let _banner = next(&mut session, "the banner").await;
    let rest = drain(&mut session).await;

    assert!(
        rest.iter().any(|event| matches!(
            event,
            DriverEvent::Other(other)
                if other.update == "<unparsed>"
                    && other.body["line"] == json!("npm warn: this is not an envelope")
        )),
        "the unparseable line is kept whole, which is the shape most worth keeping: {rest:?}"
    );
    assert!(
        rest.iter().any(|event| matches!(
            event,
            DriverEvent::AssistantChunk(chunk) if chunk.text == "ok"
        )),
        "the CRLF line parsed, so the `\\r` was stripped rather than handed to the decoder: \
         {rest:?}"
    );
    assert!(
        rest.iter()
            .any(|event| matches!(event, DriverEvent::Done(_))),
        "and the turn still ended: {rest:?}"
    );
}

/// An agent that opens its streams and never says `system/init` must not hold the tab that asked
/// for the chat — and must not be left running when the wait gives up.
///
/// The child is `sleep` and the streams are an in-process duplex nobody writes to: a session's
/// child and a session's byte streams are separable in [`ChildIo`], and separating them is what
/// makes this assertion about the child alone. `assert_not_running` and not `assert_reaped`,
/// because this is the one exit that runs no code of ours — the abort drops the `ChildGuard`, whose
/// `Drop` can only signal.
#[tokio::test]
#[cfg(unix)]
async fn a_stream_with_no_init_times_out_and_kills_its_child() {
    let tmp = tempfile::tempdir().expect("temp box");
    let child = htui_agent::launch::spawn(
        &htui_agent::launch::ResolvedLaunch {
            command: "sleep".to_owned(),
            args: vec!["1000".to_owned()],
            env: BTreeMap::new(),
        },
        tmp.path(),
    )
    .await
    .expect("`sleep` is on this box");
    let pid = child.pid().expect("a freshly spawned child has a pid");

    // Held for the test's life on purpose: dropping the agent end is an EOF, and an EOF is the
    // *other* failing exit, not this one.
    let (client_end, _agent_end) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(client_end);
    let io = ChildIo {
        reader: Box::new(reader),
        writer: Box::new(writer),
        child: Some(child),
    };

    let options = SessionOptions {
        agent_name: "scripted-cli".to_owned(),
        models: Vec::new(),
        settings: AgentSettings::default(),
        stamp: Stamp::Wall,
        // Short enough that this is a test rather than a coffee break: the production minute is
        // what `SessionOptions` carries the window as a *field* for.
        init_timeout: Duration::from_millis(300),
        box_version: None,
        session_id: "minted-id".to_owned(),
    };
    let opened = open_session(io, spec(tmp.path().to_path_buf()), "hi".to_owned(), options).await;

    match &opened {
        Err(DriverError::Transport(message)) => assert!(
            message.contains("system/init"),
            "the error names what never arrived: {message}"
        ),
        Err(other) => panic!("expected a transport error, got {other:?}"),
        Ok(_) => panic!("a silent agent does not open a session"),
    }
    assert_not_running(pid, "the session's child").await;
}

// ---------------------------------------------------------------------------------------------
// MOD-11 D18: the permission bridge
// ---------------------------------------------------------------------------------------------

/// The probe's prompt call for [`TOOL_USE_ID`], or one with no id when `id` is `None`.
#[cfg(unix)]
fn prompt_call(id: Option<&str>) -> PromptCall {
    PromptCall {
        tool_name: "mcp__htui__hello".to_owned(),
        input: json!({}),
        tool_use_id: id.map(str::to_owned),
    }
}

/// The request the session announces for a prompt whose id is `request_id`.
#[cfg(unix)]
fn prompt_request(request_id: &str, tool_call_id: Option<&str>) -> DriverEvent {
    DriverEvent::PermissionRequest(PermissionRequestEvent {
        request_id: PermissionRequestId::new(request_id),
        tool_call_id: tool_call_id.map(str::to_owned),
        options: vec![
            PermissionOption {
                id: "allow".to_owned(),
                label: "Allow".to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject".to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ],
    })
}

/// A session over [`PROMPT_SCRIPT`] with a prompt port, pulled up to its announced tool call; the
/// tool end of the bridge and the script's go-file come back with it.
#[cfg(unix)]
async fn prompting(dir: &Path) -> (Box<dyn AgentSession>, PromptAsk, PathBuf) {
    prompting_over(dir, PROMPT_SCRIPT).await
}

/// [`prompting`] over `body` rather than [`PROMPT_SCRIPT`].
#[cfg(unix)]
async fn prompting_over(dir: &Path, body: &str) -> (Box<dyn AgentSession>, PromptAsk, PathBuf) {
    let scripted = scripted(dir, body);
    let (port, ask) = bridge();
    let mut spec = spec(dir.to_path_buf());
    spec.prompt = Some(port.clone());
    let mut session = start_with(&scripted.row, spec).await;
    assert!(
        port.take().is_none(),
        "H-17: the transport took the receiver; a clone gets no second stream"
    );
    let _banner = next(&mut session, "the banner").await;
    let call = next(&mut session, "the gated call").await;
    assert!(
        matches!(&call, DriverEvent::ToolCall(call) if call.tool_call_id == TOOL_USE_ID),
        "{call:?}"
    );
    (session, ask, dir.join("go"))
}

/// Asks `call` on its own task, as the MCP tool does while the session runs.
#[cfg(unix)]
fn asking(
    ask: &PromptAsk,
    call: PromptCall,
) -> tokio::task::JoinHandle<Result<PromptVerdict, PromptClosed>> {
    let ask = ask.clone();
    htui_agent::contained::spawn(async move { ask.ask(call).await })
}

/// The verdict an asking task got, within the window.
#[cfg(unix)]
async fn verdict(
    task: tokio::task::JoinHandle<Result<PromptVerdict, PromptClosed>>,
) -> Result<PromptVerdict, PromptClosed> {
    tokio::time::timeout(EVENT_WINDOW, task)
        .await
        .expect("the ask resolved within the window")
        .expect("the asking task did not panic")
}

#[tokio::test]
#[cfg(unix)]
async fn a_prompt_request_becomes_a_permission_request_event() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, _go) = prompting(tmp.path()).await;

    let first = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    assert_eq!(
        next(&mut session, "the first prompt").await,
        prompt_request(TOOL_USE_ID, Some(TOOL_USE_ID)),
        "the CLI's tool-use id is the request id, and the call it gates"
    );

    let second = asking(&ask, prompt_call(None));
    let minted = next(&mut session, "the second prompt").await;
    let DriverEvent::PermissionRequest(request) = &minted else {
        panic!("a prompt is a permission request: {minted:?}")
    };
    assert!(
        request.request_id.as_str().starts_with("prompt-"),
        "no tool-use id, a minted request id: {minted:?}"
    );
    assert_eq!(
        minted,
        prompt_request(request.request_id.as_str(), None),
        "the same two options, no call named"
    );
    assert!(
        !first.is_finished() && !second.is_finished(),
        "both asks wait for an answer"
    );

    session.cancel(Duration::ZERO).await.expect("cancel");
    for task in [first, second] {
        assert!(matches!(
            verdict(task).await,
            Ok(PromptVerdict::Deny { .. })
        ));
    }
}

#[tokio::test]
#[cfg(unix)]
async fn answering_allow_completes_the_prompt_with_allow() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, go) = prompting(tmp.path()).await;

    let task = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    let _request = next(&mut session, "the prompt").await;
    session
        .answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("allow".to_owned()),
        )
        .await
        .expect("a parked prompt is answerable");
    assert_eq!(verdict(task).await, Ok(PromptVerdict::Allow));

    let again = session
        .answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("allow".to_owned()),
        )
        .await;
    assert!(
        matches!(&again, Err(DriverError::Transport(why)) if why.contains("no parked")),
        "an answered prompt is no longer parked: {again:?}"
    );

    std::fs::write(&go, b"").expect("go");
    let rest = drain(&mut session).await;
    assert!(
        matches!(rest.last(), Some(DriverEvent::Done(done)) if done.stop_reason == StopReason::EndTurn),
        "{rest:?}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn answering_reject_completes_it_with_deny() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, _go) = prompting(tmp.path()).await;

    let rejected = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    let _request = next(&mut session, "the first prompt").await;
    session
        .answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("reject".to_owned()),
        )
        .await
        .expect("a parked prompt is answerable");
    assert_eq!(
        verdict(rejected).await,
        Ok(PromptVerdict::Deny {
            message: "denied in htui".to_owned()
        })
    );

    let cancelled = asking(&ask, prompt_call(Some("toolu_second")));
    let _request = next(&mut session, "the second prompt").await;
    session
        .answer_permission(
            PermissionRequestId::new("toolu_second"),
            PermissionAnswer::Cancelled,
        )
        .await
        .expect("a parked prompt is answerable");
    assert_eq!(
        verdict(cancelled).await,
        Ok(PromptVerdict::Deny {
            message: "the session was cancelled".to_owned()
        })
    );

    session.cancel(Duration::ZERO).await.expect("cancel");
}

#[tokio::test]
#[cfg(unix)]
async fn a_cancel_denies_every_pending_prompt() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, _go) = prompting(tmp.path()).await;

    let first = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    let _request = next(&mut session, "the first prompt").await;
    let second = asking(&ask, prompt_call(None));
    let _request = next(&mut session, "the second prompt").await;

    session.cancel(Duration::ZERO).await.expect("cancel");
    for task in [first, second] {
        assert_eq!(
            verdict(task).await,
            Ok(PromptVerdict::Deny {
                message: "the session was cancelled".to_owned()
            })
        );
    }
    assert_eq!(
        ask.ask(prompt_call(None)).await,
        Err(PromptClosed),
        "nobody is left to ask"
    );
}

/// D18 dedup: the terminal `result` repeats a prompt-denied call in `permission_denials[]`; the
/// prompt's own answer already settled it, so the replay adds no `permission_answer`.
#[tokio::test]
#[cfg(unix)]
async fn a_prompt_denial_is_reported_once() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, go) = prompting(tmp.path()).await;

    let task = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    let _request = next(&mut session, "the prompt").await;
    session
        .answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("reject".to_owned()),
        )
        .await
        .expect("a parked prompt is answerable");
    assert!(matches!(
        verdict(task).await,
        Ok(PromptVerdict::Deny { .. })
    ));

    std::fs::write(&go, b"").expect("go");
    let rest = drain(&mut session).await;
    assert!(
        !rest
            .iter()
            .any(|event| matches!(event, DriverEvent::PermissionAnswer(_))),
        "the answered prompt is not reported again: {rest:?}"
    );
    assert!(
        matches!(rest.last(), Some(DriverEvent::Done(_))),
        "{rest:?}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn without_a_port_answer_permission_stays_unsupported() {
    let tmp = tempfile::tempdir().expect("temp box");
    let scripted = scripted(tmp.path(), TURN_SCRIPT);
    let mut session = start(&scripted.row, tmp.path()).await;
    let refused = session
        .answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("allow".to_owned()),
        )
        .await;
    assert!(
        matches!(refused, Err(DriverError::Unsupported("answer_permission"))),
        "{refused:?}"
    );
    let argv = std::fs::read_to_string(&scripted.argv).expect("the script recorded its argv");
    assert!(!argv.contains("--permission-prompt-tool"), "{argv}");
    session.cancel(Duration::ZERO).await.expect("cancel");
}

/// Adversarial review T9-ADV-1: an answer must not depend on the caller pulling events. A CLI that
/// keeps streaming while one call waits fills the event channel; the session task then sits in
/// `emit`, and a handle that only awaited the task's reply would wait on a task waiting on it.
#[tokio::test]
#[cfg(unix)]
async fn an_answer_lands_while_the_event_channel_is_full() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, go) = prompting_over(tmp.path(), FLOOD_PROMPT_SCRIPT).await;

    let task = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    assert_eq!(
        next(&mut session, "the prompt").await,
        prompt_request(TOOL_USE_ID, Some(TOOL_USE_ID))
    );
    std::fs::write(&go, b"").expect("go");
    // Long enough for the flood to fill the channel behind the parked prompt.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let answered = tokio::time::timeout(
        EVENT_WINDOW,
        session.answer_permission(
            PermissionRequestId::new(TOOL_USE_ID),
            PermissionAnswer::Selected("allow".to_owned()),
        ),
    )
    .await;
    assert!(
        matches!(answered, Ok(Ok(()))),
        "the answer lands with nobody pulling: {answered:?}"
    );
    assert_eq!(verdict(task).await, Ok(PromptVerdict::Allow));

    std::fs::write(go.with_extension("end"), b"").expect("end");
    let rest = drain(&mut session).await;
    assert!(
        rest.len() >= FLOOD,
        "nothing the answer drained is lost: {} events",
        rest.len()
    );
    assert!(
        matches!(rest.last(), Some(DriverEvent::Done(done)) if done.stop_reason == StopReason::EndTurn),
        "{:?}",
        rest.last()
    );
}

/// Adversarial review T9-ADV-2: a cancel denies the parked prompt, and the caller records that
/// request's `cancelled` answer itself (the relay's `answer_cancelled`, chat's `Cancel` arm). The
/// `result` the interrupt buys repeats the denied call in `permission_denials[]`; it is the same
/// refusal and adds no second `permission_answer`.
#[tokio::test]
#[cfg(unix)]
async fn a_cancelled_prompt_is_not_reported_again() {
    let tmp = tempfile::tempdir().expect("temp box");
    let (mut session, ask, _go) = prompting_over(tmp.path(), PROMPT_CANCEL_SCRIPT).await;

    let task = asking(&ask, prompt_call(Some(TOOL_USE_ID)));
    let _request = next(&mut session, "the prompt").await;

    session.cancel(EVENT_WINDOW).await.expect("cancel");
    assert_eq!(
        verdict(task).await,
        Ok(PromptVerdict::Deny {
            message: "the session was cancelled".to_owned()
        })
    );
    let rest = drain(&mut session).await;
    assert!(
        matches!(rest.last(), Some(DriverEvent::Done(_))),
        "the interrupt's `result` was drained: {rest:?}"
    );
    assert!(
        !rest
            .iter()
            .any(|event| matches!(event, DriverEvent::PermissionAnswer(_))),
        "the cancelled prompt is not reported again: {rest:?}"
    );
}
