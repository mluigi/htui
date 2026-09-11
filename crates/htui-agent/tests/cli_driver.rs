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
use std::path::PathBuf;

use htui_agent::cli::{argv, usd};
use htui_agent::driver::{AgentSessionRef, PermissionPolicy, SessionSpec, ToolExposure};
use htui_agent::launch::CliSettings;
use htui_core::model::{AgentId, StepId};

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
