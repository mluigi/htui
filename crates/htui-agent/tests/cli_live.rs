//! The `claude` CLI stream, probed live: the nine empirical questions MOD-2's CLI transport is
//! written from (plan T55, T56; blueprint §F-T55, §F-T56).
//!
//! `#[ignore]` by default, exactly as `tests/agy_live.rs`, `tests/acp_live.rs` and
//! `tests/probe_live.rs` are. Unlike those three, **every case here spends real model tokens** on
//! whatever credential this box holds, so nothing in it may ever run from a plain `cargo test`. Run
//! it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test cli_live -- --ignored --nocapture
//! ```
//!
//! What it spawns: the `claude` binary the **seed row's own tier-1 probe** resolves, through
//! [`htui_agent::tools::resolve`] → [`htui_agent::launch::resolve`] → [`htui_agent::launch::spawn`]
//! — never through `src/cli/`. That module does not exist when this file is written, and it must
//! not be what a probe proves: the whole point of running before the transport is built is that the
//! answers arrive as *facts about the CLI* rather than as facts about `htui`'s reading of it.
//! `HTUI_TOOL_CLAUDE` overrides the resolution the same way it does for any other tool.
//!
//! Every case: resolve, spawn into a throwaway working directory, feed NDJSON user messages on
//! stdin, read stdout to EOF on a bounded deadline, record the transcript, and assert **by pid**
//! that nothing survives (`Spawned::pid`, then `/proc/<pid>` absent or state `Z`/`X` — the
//! `tests/acp_driver.rs` helper's parse, copied rather than shared as this repo copies its process
//! helpers per file). Then it prints the answer it exists for.
//!
//! **Prompts are one-word-answer short wherever the question allows.** The exceptions are case 3
//! (which needs a turn long enough to interrupt), case 6 (which needs a turn worth thinking about)
//! and case 7 (which needs a subagent), and each says so.
//!
//! # The fixtures
//!
//! Each case writes `claude_stream_json_<case>.jsonl` into `$HTUI_CLI_FIXTURE_DIR` — **read** from
//! the environment, never written to it (`std::env::set_var` is `unsafe` and the workspace is
//! `unsafe_code = "forbid"`), defaulting to this crate's `tests/fixtures/`. One JSON record per
//! line:
//!
//! ```text
//! {"direction":"stdin"|"stdout","line":<json|text>}   …
//! {"direction":"exit","status":<int|null>,"signal":<int|null>}
//! ```
//!
//! `line` is the parsed JSON when the line parsed and the raw string when it did not, so a
//! transport that emits a non-JSON line is recorded as what it emitted rather than dropped. These
//! transcripts are **T51's inputs** — `src/cli/claude.rs` is mapped against them — so a fixture
//! that does not faithfully record what arrived is worse than no fixture.
//!
//! **The redaction rule** (blueprint A.18, file-set row 18), applied just before the transcript is
//! written and asserted afterwards, because these transcripts are committed:
//!
//! - every session id, ours and the CLI's, → `<session>`;
//! - the run's working directory → `/scratch`;
//! - this box's home directory → `~`;
//! - and, belt and braces, the literal **value** of every environment variable whose name looks
//!   like a credential ([`SECRET_NAMES`]) → `<redacted>`, so a hook that echoed one into the stream
//!   cannot ride into git on the strength of nobody having thought of it.
//!
//! Each needle is applied **twice**: once to the serialised file text, and once to the reassembled
//! run of streaming deltas, which is a different string. [`write_fixture`] says why at length; the
//! short version is that the model chunks its reply wherever the tokeniser did, so a quoted path or
//! token arrives split across six JSON strings and a substitution over the file cannot see it. The
//! first version of this file redacted only the file text and asserted only the file text, and a
//! home directory rode into git through that gap on 2026-09-10.
//!
//! Message `uuid`s and `tool_use_id`s are **not** redacted: they are the wire's own join keys, they
//! name nothing outside the transcript, and T51 maps `tool_use_id` back to its call.
//!
//! # The nine cases and the question each one answers
//!
//! 1. [`case_1_auth_and_the_first_stdin_message`] — **E-0 / D92**. Two spawns, one with `--bare`
//!    and one without. Which of them authenticates on this box decides whether `--bare` stays in
//!    the seed's `extra_args`, and the ordering it prints (hook lines before `system/init`?) is
//!    what D84's banner buffer is sized for (H-2). Also: is `init` emitted before the first stdin
//!    line is read, and does `-p` with no positional prompt accept the stdin form at all (P-2).
//! 2. [`case_2_session_id_is_ours`] — **D84, H-12, H-17**. A UUIDv7 on `--session-id`, then a
//!    `--resume` of it.
//! 3. [`case_3_cancellation_semantics`] — **D81** and `docs/ANA-4.md` §11.14's cancellation item.
//!    Three spawns on a long turn: stdin close alone, stdin close then SIGINT, SIGTERM.
//! 4. [`case_4_usage_across_two_turns`] — **E-2**. Are `modelUsage[*]` counts per-turn or
//!    cumulative? Sets B.4's `tokens_are_cumulative`.
//! 5. [`case_5_stdin_eof_between_turns_exits_clean`] — D-2's "not a cancel" path.
//! 6. [`case_6_thinking_blocks`] — **D82**. §6.2 could not say what a `thinking` block looks like
//!    because thinking was off in the runs ANA-4 was written from.
//! 7. [`case_7_subagent_makes_model_usage_and_usage_disagree`] — **D86**. `result.usage` counts
//!    only the top-level loop; a subagent is what makes the two figures visibly differ.
//! 8. [`case_8_budget_trips_server_side`] — **D83, H-16**. `--max-budget-usd` at a near-zero value
//!    and at zero.
//! 9. [`case_9_a_denied_tool`] — **D85, E-1**. What a policy denial looks like on the wire, which
//!    is what `DriverEvent::PermissionAnswer` is mapped from.
//!
//! # The file's own rule
//!
//! **A probe that guesses is worse than one that fails.** No case asserts the hypothesis it was
//! written to test: `docs/ANA-4.md` §4.4's SIGINT/SIGTERM sentence is explicitly *unverified*, and
//! a test written to confirm it would confirm it by construction. What each case asserts is either
//! a precondition (the binary resolved, the process died) or a relation it **measured** — case 4's
//! two candidate arithmetics are the pattern: exactly one of them must hold, and a run where
//! neither does fails with both figures printed, because that is a genuine unknown and the
//! maintainer needs to see it.
//!
//! Reading the seed row's `claude` tool probe is data, not a code path keyed on an agent name
//! (`R-AGT-5`); `tests/probe_live.rs` and `tests/agy_live.rs` do the same. Test files are exempt
//! from `tests/extensibility.rs`'s vendor sweeps, which scan `src/` only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::Utc;
use htui_agent::launch::{AgentLaunch, Discovery, Spawned, StopSignal, ToolProbe};
use htui_core::model::StepId;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};

// ---------------------------------------------------------------------------------------------
// Shared machinery
// ---------------------------------------------------------------------------------------------

/// One live `claude` at a time, across all nine cases.
///
/// Three reasons, any one of which would be enough: the cases assert on process survival, the box's
/// subscription has a five-hour window that two concurrent turns burn twice as fast, and a
/// `--nocapture` run whose nine transcripts interleave is unreadable — and reading them is the
/// point of the file. A `tokio::sync::Mutex` because each `#[tokio::test]` builds its own runtime
/// and the guard is held across `.await`s.
static ONE_AT_A_TIME: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Every `rate_limit_event` any case in this process has seen, newest last.
///
/// Case 9 prints it, which is blueprint §F-T56's last sentence: B.4 has to know whether the quota
/// blob is nested under `rate_limit_info` or sits at the top level, and the only way to find out is
/// to notice one going past. A `std::sync::Mutex` because it is never held across an `.await`.
static RATE_LIMITS: LazyLock<std::sync::Mutex<Vec<Value>>> =
    LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// Environment variable names whose **values** are scrubbed out of every fixture.
///
/// Substring matches on the upper-cased name, not an exact list: the point is to catch the variable
/// nobody thought of. A short value is skipped by [`secret_values`] — replacing every occurrence of
/// a two-character `KEY` would shred the transcript and prove nothing.
const SECRET_NAMES: [&str; 6] = [
    "TOKEN",
    "KEY",
    "SECRET",
    "CREDENTIAL",
    "PASSWORD",
    "HEADERS",
];

/// How long a one-word turn is given to produce its `result`.
///
/// Generous rather than tuned, for `tests/agy_live.rs`'s reason: a slow box must report as slow,
/// not as broken. A cold `claude` start on this box pays for its hooks and its plugin cache before
/// it sends a byte.
const TURN_WINDOW: Duration = Duration::from_secs(180);

/// How long a *cancelled* turn is given to reach EOF (case 3, b and c).
///
/// This is the figure D81's grace window is chosen against, so a case that timed out at 5 s would
/// answer "the CLI does not exit" when the truth might be "it takes seven seconds". Long enough to
/// be evidence, finite so a regression is a named failure rather than a hung suite.
const CANCEL_WINDOW: Duration = Duration::from_secs(90);

/// How long the *uncancelled* long turn of case 3 (a) is given to finish and exit.
const LONG_TURN_WINDOW: Duration = Duration::from_secs(300);

/// D-2's promise: stdin closed after a `result` ends the process promptly (case 5 measures it).
const EOF_WINDOW: Duration = Duration::from_secs(5);

/// The `claude` tool probe the seed rows declare, whichever row declares it.
///
/// By probe rather than by row name so the parallel seed work (T49 splits the CLI path onto its own
/// row) cannot break this file: any row that declares a `claude` tool declares the same tier-1
/// `path` probe, and this file needs the binary, not the row.
fn claude_probe() -> ToolProbe {
    for row in htui_core::model::agent::seed_rows(Utc::now()) {
        let Ok(launch) = serde_json::from_value::<AgentLaunch>(row.launch.clone()) else {
            continue;
        };
        if let Some(probe) = launch
            .discovery
            .as_ref()
            .and_then(|discovery| discovery.tools.get("claude"))
        {
            return probe.clone();
        }
    }
    panic!(
        "precondition: no seed row declares a `claude` tool probe, so this file cannot find the \
         binary the way production does. Point HTUI_TOOL_CLAUDE at it, or restore the row."
    )
}

/// The `${claude}` launch this file spawns: the §4.4 argv minus whatever the case has under test.
fn claude_launch(args: &[String]) -> AgentLaunch {
    let mut tools = BTreeMap::new();
    tools.insert("claude".to_owned(), claude_probe());
    AgentLaunch {
        command: "${claude}".to_owned(),
        args: args.to_vec(),
        env: BTreeMap::new(),
        discovery: Some(Discovery {
            tools,
            // Tier 2 is the ACP handshake; this transport has none.
            handshake: false,
            credential: None,
            install: None,
        }),
    }
}

/// `docs/ANA-4.md` §4.4's invocation, minus every flag a case decides for itself.
///
/// `-p` with **no positional prompt**: the prompt arrives as the first stdin NDJSON message, which
/// is P-2's question and which case 1 records the answer to.
fn base_args() -> Vec<String> {
    [
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect()
}

/// [`base_args`] with `extra` appended.
fn args_with(extra: &[&str]) -> Vec<String> {
    let mut args = base_args();
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    args
}

/// One NDJSON user message, the shape `--input-format stream-json` reads.
fn user_message(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    })
}

/// One stdout line as this file records it: JSON when it parsed, the raw text when it did not.
#[derive(Debug, Clone)]
enum Line {
    /// A line that parsed as JSON — everything the CLI documents.
    Json(Value),
    /// A line that did not. Recorded verbatim, because §6.2's wildcard row exists for it.
    Text(String),
}

impl Line {
    /// `type`, or `<non-json>` for an unparsed line.
    fn kind(&self) -> String {
        match self {
            Self::Json(value) => match (value["type"].as_str(), value["subtype"].as_str()) {
                (Some(kind), Some(sub)) => format!("{kind}/{sub}"),
                (Some(kind), None) => kind.to_owned(),
                (None, _) => "<no type>".to_owned(),
            },
            Self::Text(_) => "<non-json>".to_owned(),
        }
    }

    /// The parsed value, when there was one.
    fn json(&self) -> Option<&Value> {
        match self {
            Self::Json(value) => Some(value),
            Self::Text(_) => None,
        }
    }
}

/// What one bounded read of stdout produced.
#[derive(Debug)]
enum Next {
    /// A line arrived.
    Line(Line),
    /// The child closed stdout.
    Eof,
    /// The deadline passed with no line and no EOF.
    TimedOut,
}

/// A running `claude`, its recorded transcript, and the handles this file talks to it through.
#[derive(Debug)]
struct Run {
    spawned: Spawned,
    stdout: Lines<BufReader<ChildStdout>>,
    /// `None` once stdin has been closed — dropping the handle is what closes it.
    stdin: Option<ChildStdin>,
    /// The `{"direction":…}` records, in arrival order.
    records: Vec<Value>,
    /// Every stdout line, for the assertions the case makes afterwards.
    lines: Vec<Line>,
    pid: u32,
    /// The working directory, kept for the redaction pass.
    cwd: PathBuf,
    started: Instant,
}

impl Run {
    /// Resolves and spawns `claude` with `args`, in `cwd`.
    async fn start(cwd: &Path, args: &[String]) -> Self {
        let launch = claude_launch(args);
        let tools = htui_agent::tools::resolve(launch.discovery.as_ref(), cwd)
            .await
            .expect(
                "precondition: `claude` did not resolve on this box. It is a `path` probe on the \
                 seed row; put the binary on PATH or set HTUI_TOOL_CLAUDE.",
            );
        let resolved =
            htui_agent::launch::resolve(&launch, &tools).expect("`${claude}` substitutes");
        println!("$ {} {:?}", resolved.command, resolved.args);
        let mut spawned = htui_agent::launch::spawn(&resolved, cwd)
            .await
            .expect("the CLI starts");
        let pid = spawned.pid().expect("a live child reports a pid");
        let stdin = spawned.take_stdin().expect("stdin is piped").into_inner();
        let stdout = BufReader::new(spawned.take_stdout().expect("stdout is piped").into_inner());
        Self {
            spawned,
            stdout: stdout.lines(),
            stdin: Some(stdin),
            records: Vec::new(),
            lines: Vec::new(),
            pid,
            cwd: cwd.to_path_buf(),
            started: Instant::now(),
        }
    }

    /// Writes one NDJSON user message and records it.
    async fn send(&mut self, text: &str) {
        let message = user_message(text);
        let mut payload = serde_json::to_string(&message).expect("a user message serialises");
        payload.push('\n');
        let stdin = self
            .stdin
            .as_mut()
            .expect("stdin is still open when a message is sent");
        stdin
            .write_all(payload.as_bytes())
            .await
            .expect("the CLI accepts a user message on stdin");
        stdin.flush().await.expect("stdin flushes");
        self.records
            .push(json!({ "direction": "stdin", "line": message }));
        println!(
            "  [{:>6.2}s] -> {text}",
            self.started.elapsed().as_secs_f64()
        );
    }

    /// Closes stdin: the documented end-of-input for `--input-format stream-json`.
    fn close_stdin(&mut self) {
        drop(self.stdin.take());
        println!(
            "  [{:>6.2}s] -- stdin closed",
            self.started.elapsed().as_secs_f64()
        );
    }

    /// Signals the child's process **group**, through the production method (`Spawned::signal`).
    fn signal(&self, signal: StopSignal) {
        println!(
            "  [{:>6.2}s] -- {signal} to the group",
            self.started.elapsed().as_secs_f64()
        );
        // Not an assertion: a group that has already exited refuses the signal, and "the CLI died
        // before we could interrupt it" is an answer this case reports rather than fails on.
        if let Err(error) = self.spawned.signal(signal) {
            println!("        the signal was refused: {error}");
        }
    }

    /// One bounded read of stdout, recorded.
    async fn next(&mut self, within: Duration) -> Next {
        match tokio::time::timeout(within, self.stdout.next_line()).await {
            Err(_) => Next::TimedOut,
            Ok(Err(error)) => {
                // A read error is EOF for this file's purposes, but it is never silent.
                println!("  !! reading stdout: {error}");
                Next::Eof
            }
            Ok(Ok(None)) => Next::Eof,
            Ok(Ok(Some(text))) => {
                let line = match serde_json::from_str::<Value>(&text) {
                    Ok(value) => Line::Json(value),
                    Err(_) => Line::Text(text),
                };
                if let Some(value) = line.json()
                    && value["type"] == json!("rate_limit_event")
                {
                    RATE_LIMITS
                        .lock()
                        .expect("the rate-limit log is not poisoned")
                        .push(value.clone());
                }
                self.records.push(json!({
                    "direction": "stdout",
                    "line": match &line {
                        Line::Json(value) => value.clone(),
                        Line::Text(text) => Value::String(text.clone()),
                    },
                }));
                self.lines.push(line.clone());
                Next::Line(line)
            }
        }
    }

    /// Reads until a line satisfies `wanted`, returning it; `None` on EOF or on the deadline.
    async fn read_until(
        &mut self,
        within: Duration,
        wanted: impl Fn(&Line) -> bool,
    ) -> Option<Line> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                println!("  .. gave up waiting after {within:?}");
                return None;
            }
            match self.next(left).await {
                Next::Line(line) => {
                    if wanted(&line) {
                        return Some(line);
                    }
                }
                Next::Eof => {
                    println!("  .. EOF before the line this read was waiting for");
                    return None;
                }
                Next::TimedOut => {
                    println!("  .. gave up waiting after {within:?}");
                    return None;
                }
            }
        }
    }

    /// Reads until the turn's terminal `result`, returning it.
    async fn read_result(&mut self, within: Duration) -> Option<Value> {
        self.read_until(within, |line| {
            line.json()
                .is_some_and(|value| value["type"] == json!("result"))
        })
        .await
        .and_then(|line| line.json().cloned())
    }

    /// Drains stdout to EOF, returning the elapsed time — or `None` if the deadline came first.
    async fn drain(&mut self, within: Duration) -> Option<Duration> {
        let started = Instant::now();
        let deadline = started + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            match self.next(left).await {
                Next::Line(_) => {}
                Next::Eof => return Some(started.elapsed()),
                Next::TimedOut => return None,
            }
        }
    }

    /// The exit status, waited for on a deadline; `None` when the child outlived it.
    async fn wait(&mut self, within: Duration) -> Option<ExitStatus> {
        match tokio::time::timeout(within, self.spawned.wait()).await {
            Ok(Ok(status)) => Some(status),
            Ok(Err(error)) => panic!("waiting for the CLI: {error}"),
            Err(_) => None,
        }
    }

    /// Every stdout line that parsed, in order.
    fn json_lines(&self) -> Vec<&Value> {
        self.lines.iter().filter_map(Line::json).collect()
    }

    /// The `kind()` of every line, for the orderings the cases print.
    fn kinds(&self) -> Vec<String> {
        self.lines.iter().map(Line::kind).collect()
    }

    /// Records the exit, writes `claude_stream_json_<name>.jsonl`, and asserts nothing survived.
    ///
    /// The kill is unconditional and comes **after** the wait: a case that reached EOF has already
    /// been reaped and this is a no-op, and a case whose child outlived its window must not leave
    /// it running for the next case to trip over. Blueprint §F-T55's survivor assertion is by pid.
    async fn finish(mut self, name: &str, waited: Option<ExitStatus>) -> ExitStatus {
        let status = match waited {
            Some(status) => status,
            None => {
                let _ = self.spawned.kill_tree().await;
                self.spawned
                    .wait()
                    .await
                    .expect("a killed child is waitable")
            }
        };
        println!(
            "  exit: code={:?} signal={:?} success={}",
            status.code(),
            signal_of(&status),
            status.success()
        );
        for line in self.spawned.stderr_tail() {
            println!("  stderr| {line}");
        }
        self.records.push(json!({
            "direction": "exit",
            "status": status.code(),
            "signal": signal_of(&status),
        }));
        write_fixture(name, &self.records, &self.cwd);
        assert_not_running(self.pid, name).await;
        status
    }
}

/// The signal that killed a process, where the platform has signals.
#[cfg(unix)]
fn signal_of(status: &ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(status)
}

/// The signal that killed a process, where the platform has signals.
#[cfg(not(unix))]
fn signal_of(_status: &ExitStatus) -> Option<i32> {
    None
}

/// Fails unless `pid` is gone or reaped-pending within two seconds.
///
/// Copied from `tests/acp_driver.rs` rather than shared, as this repo copies its process helpers
/// per file: a case's own assertion living in its own file is what lets one of them change without
/// the others being re-read. By pid, never by a `pgrep` pattern — a pattern matches the shell that
/// launched `cargo test` and any editor with the word in its command line.
async fn assert_not_running(pid: u32, what: &str) {
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(2);
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
                Instant::now() < deadline,
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

/// Where the transcripts go: `$HTUI_CLI_FIXTURE_DIR`, or this crate's `tests/fixtures`.
fn fixture_dir() -> PathBuf {
    std::env::var_os("HTUI_CLI_FIXTURE_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        PathBuf::from,
    )
}

/// Every `session_id` / `sessionId` value anywhere in `records`.
fn session_ids(records: &[Value]) -> BTreeSet<String> {
    fn walk(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if (key == "session_id" || key == "sessionId")
                        && let Some(text) = child.as_str()
                        && !text.is_empty()
                    {
                        found.insert(text.to_owned());
                    }
                    walk(child, found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, found);
                }
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    for record in records {
        walk(record, &mut found);
    }
    found
}

/// The values of this process's credential-looking environment variables.
///
/// Long values only: a short one would match half the transcript and prove nothing.
fn secret_values() -> Vec<String> {
    std::env::vars()
        .filter(|(name, value)| {
            value.len() >= 12 && {
                let upper = name.to_uppercase();
                SECRET_NAMES.iter().any(|needle| upper.contains(needle))
            }
        })
        .map(|(_, value)| value)
        .collect()
}

/// Writes one transcript, redacted per the module doc, and checks the redaction held.
///
/// **Two passes, and the second one is not optional.** A plain text substitution over the
/// serialised file redacts every occurrence that is *contiguous in the file* — which is every
/// occurrence in a `system/init` or a `result`, and not every occurrence in the stream. The model
/// emits its reply as `text_delta`s split wherever the tokeniser split it, so a path or a token it
/// happens to quote arrives as `"…/"`, `"home/m"`, `"lu"`, `"ig"`, `"i/.claude…"` — six separate
/// JSON strings on six separate lines, none of which contains the needle. The substitution cannot
/// see it, and neither could this function's own assertion, because it re-checked the same
/// unreassembled text it had just rewritten: **a green redaction check proved nothing about the
/// stream**.
///
/// That is how `~` survived in the assembled `assistant` envelope of the thinking fixture while the
/// deltas that built it still spelled the home directory out (found 2026-09-11, by T51's
/// `text_streams_once_not_twice` comparing the two forms against each other). The leak that
/// actually happened is a username in a path. The hole is the same size for [`SECRET_NAMES`],
/// which is the half of the rule that exists so "a hook that echoed one into the stream cannot
/// ride into git on the strength of nobody having thought of it" — a token quoted by a tool's
/// output splits exactly as readily as a path.
///
/// So the needles are applied to the **reassembled** delta run as well
/// ([`redact_across_deltas`]), and the assertions below re-check that form too.
fn write_fixture(name: &str, records: &[Value], cwd: &Path) {
    let home = dirs::home_dir().map(|home| home.display().to_string());
    let cwd = cwd.display().to_string();

    // Every needle with its replacement, in the order they must be applied: the working directory
    // before the home directory, because the former lives inside the latter on a default box and
    // replacing the home first would leave a half-rewritten `~/…/scratch` path.
    let mut needles: Vec<(String, &str)> = Vec::new();
    for secret in secret_values() {
        needles.push((secret, "<redacted>"));
    }
    for id in session_ids(records) {
        needles.push((id, "<session>"));
    }
    if cwd.len() > 1 {
        needles.push((slug_of(&cwd), "-scratch"));
        needles.push((cwd, "/scratch"));
    }
    if let Some(home) = &home
        && home.len() > 1
    {
        needles.push((slug_of(home), "-home"));
        needles.push((home.clone(), "~"));
    }

    // Pass one, over the records: the split-aware rewrite, which is the only one that can reach a
    // needle the stream cut in half.
    let mut records = records.to_vec();
    for (needle, replacement) in &needles {
        redact_across_deltas(&mut records, needle, replacement);
    }

    // Pass two, over the serialised text: every contiguous occurrence, everywhere else in the
    // envelope — keys, ids, `cwd`, the assembled messages, the `result`.
    let mut text = String::new();
    for record in &records {
        text.push_str(&serde_json::to_string(record).expect("a transcript record serialises"));
        text.push('\n');
    }
    for (needle, replacement) in &needles {
        text = text.replace(needle, replacement);
    }

    // And the check, in both forms. The reassembled one is the one that would have caught this.
    let streamed = streamed_text(&records);
    for (needle, _) in &needles {
        assert!(
            !text.contains(needle),
            "{name}: a redacted value survived in the transcript text"
        );
        assert!(
            !streamed.contains(needle),
            "{name}: a redacted value survived *reassembled* across delta boundaries — the \
             transcript's own text is clean and the stream it encodes is not"
        );
    }

    let directory = fixture_dir();
    std::fs::create_dir_all(&directory).expect("the fixture directory is writable");
    let path = directory.join(format!("claude_stream_json_{name}.jsonl"));
    std::fs::write(&path, text).expect("the fixture is writable");
    println!("  fixture: {} ({} records)", path.display(), records.len());
}

/// A path as the CLI slugs it into a directory name: every character that is not alphanumeric,
/// `_` or `-` becomes `-`.
///
/// A second way a needle escapes a literal substitution, found the same way the first one was — by
/// a later case reading a committed fixture (case 10, 2026-09-11). The CLI derives a per-project
/// directory from the working directory and reports it on `system/init`, so
/// `/tmp/.tmpAbCdEf` arrives as `memory_paths.auto: "~/.claude/projects/-tmp--tmpAbCdEf/memory/"`.
/// The path *is* in the transcript; it is simply not spelled the way the filesystem spells it, and
/// a `text.replace(cwd, …)` cannot see it.
///
/// The value that leaked this way is a tempdir name, which is worth nothing. The rule it broke —
/// "the run's working directory → `/scratch`" — is worth stating truthfully, and the same
/// transformation applies to the home directory, which carries a username.
///
/// This is not a general defence. A needle can be base64'd, URL-encoded, or split *and* slugged,
/// and no substitution catches every form of a value the vendor is free to reshape. It covers the
/// one transformation that was observed, names the class in the assertion, and the fixture review
/// before a commit is what covers the rest.
fn slug_of(path: &str) -> String {
    path.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Every streamed text and thinking delta of a transcript, concatenated: the string the reader of
/// the stream actually sees, as opposed to the string the file happens to hold.
fn streamed_text(records: &[Value]) -> String {
    let mut joined = String::new();
    for_each_delta(&mut records.to_vec(), |text| joined.push_str(text));
    joined
}

/// Calls `visit` on every `text_delta` / `thinking_delta` payload, in arrival order.
fn for_each_delta(records: &mut [Value], mut visit: impl FnMut(&str)) {
    for record in records {
        if record["direction"] != json!("stdout") {
            continue;
        }
        let delta = &record["line"]["event"]["delta"];
        for key in ["text", "thinking"] {
            if let Some(text) = delta[key].as_str() {
                visit(text);
            }
        }
    }
}

/// Redacts a needle the stream split across delta boundaries, preserving the boundaries.
///
/// The deltas are the evidence — how a reply was chunked is a fact about the transport, and a
/// redaction that collapsed a run into one string would edit that fact while claiming to edit a
/// secret. So the occupied character range is blanked *in place*, across however many deltas it
/// spans, and the replacement is inserted at the start of the first of them. The delta count and
/// every other byte survive.
///
/// Loops until the needle is gone: one secret may be quoted twice in one reply.
fn redact_across_deltas(records: &mut [Value], needle: &str, replacement: &str) {
    if needle.is_empty() {
        return;
    }
    loop {
        let mut chunks: Vec<String> = Vec::new();
        for_each_delta(records, |text| chunks.push(text.to_owned()));
        let joined = chunks.concat();
        let Some(at) = joined.find(needle) else {
            return;
        };
        let end = at + needle.len();

        let mut cursor = 0usize;
        let mut inserted = false;
        for chunk in &mut chunks {
            let start = cursor;
            cursor += chunk.len();
            if cursor <= at || start >= end {
                continue;
            }
            let from = at.saturating_sub(start);
            let to = (end - start).min(chunk.len());
            let mut next = String::with_capacity(chunk.len());
            next.push_str(&chunk[..from]);
            if !inserted {
                next.push_str(replacement);
                inserted = true;
            }
            next.push_str(&chunk[to..]);
            *chunk = next;
        }

        let mut rewritten = chunks.into_iter();
        for record in &mut *records {
            if record["direction"] != json!("stdout") {
                continue;
            }
            for key in ["text", "thinking"] {
                if record["line"]["event"]["delta"][key].is_string() {
                    let next = rewritten.next().unwrap_or_default();
                    record["line"]["event"]["delta"][key] = Value::String(next);
                }
            }
        }
    }
}

/// A throwaway working directory, so the CLI reads no repository of the maintainer's and the
/// transcript's `cwd` redacts to one predictable `/scratch`.
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("a scratch working directory")
}

/// `Σ result.modelUsage[*][key]`, or `None` when the map is absent or empty.
///
/// The `None` matters: B.4's D86 table says an empty map answers `null` rather than `0`, and a
/// helper that returned `0` would hide the difference this file exists to measure.
fn model_usage_sum(result: &Value, key: &str) -> Option<i64> {
    let models = result["modelUsage"].as_object()?;
    if models.is_empty() {
        return None;
    }
    Some(
        models
            .values()
            .filter_map(|model| model[key].as_i64())
            .sum(),
    )
}

/// A compact one-line view of a `result`'s five figures, for the printouts.
fn usage_line(result: &Value) -> String {
    format!(
        "subtype={:?} is_error={} stop_reason={:?} terminal_reason={:?} num_turns={:?} \
         total_cost_usd={:?} usage.input={:?} usage.output={:?} ΣmodelUsage.in={:?} \
         ΣmodelUsage.out={:?} ΣmodelUsage.cacheRead={:?} ΣmodelUsage.cacheCreate={:?}",
        result["subtype"].as_str(),
        result["is_error"],
        result["stop_reason"].as_str(),
        result["terminal_reason"].as_str(),
        result["num_turns"].as_i64(),
        result["total_cost_usd"].as_f64(),
        result["usage"]["input_tokens"].as_i64(),
        result["usage"]["output_tokens"].as_i64(),
        model_usage_sum(result, "inputTokens"),
        model_usage_sum(result, "outputTokens"),
        model_usage_sum(result, "cacheReadInputTokens"),
        model_usage_sum(result, "cacheCreationInputTokens"),
    )
}

/// Whether a `result` reports a completed, unerrored turn.
fn is_success(result: &Value) -> bool {
    result["is_error"] != json!(true)
}

/// The `type` of every line up to and including `system/init`, or the whole list if it never came.
fn up_to_init(kinds: &[String]) -> Vec<String> {
    match kinds.iter().position(|kind| kind == "system/init") {
        Some(at) => kinds[..=at].to_vec(),
        None => kinds.to_vec(),
    }
}

// ---------------------------------------------------------------------------------------------
// Case 1 — E-0 / D92: which form of the invocation authenticates on this box
// ---------------------------------------------------------------------------------------------

/// The paired `--bare` / plain spawn D92 rests on, and the pre-`init` ordering D84 buffers for.
///
/// `--bare` is `docs/ANA-4.md` §4.4's "documented recommendation for scripted callers", and the
/// seed carried it. The plan's D92 drops it on the grounds that `--bare` makes Anthropic auth
/// *strictly* `ANTHROPIC_API_KEY` or `apiKeyHelper` — never OAuth, never the keychain — which on a
/// subscription box means every live turn fails auth before reading a byte of stdin. That is a
/// claim about **this box**, and this case is the only thing that can check it.
///
/// It does **not** assert the expectation. A box with an API key would authenticate both ways and a
/// test that demanded `--bare` fail would be wrong there. What it asserts is that *some* form
/// worked — a box where neither does cannot answer any other question in this file, and the failure
/// names both credentials so the reason is actionable — and it prints `D92` as `confirmed` or as a
/// loud contradiction, because if the expectation is wrong then D92 and the seed row are wrong with
/// it.
///
/// Three more answers fall out of the same two spawns:
///
/// - **P-2**: `-p` with no positional prompt, prompt on stdin. If the CLI refused that shape there
///   would be no `assistant` line at all.
/// - **is `init` emitted before the first stdin line is read?** The prompt is withheld for 500 ms
///   and what arrived in that window is printed. A supervisor that waited for `init` before writing
///   the prompt would deadlock if the answer were "no".
/// - **H-2 / D84**: whether hook lines precede `system/init` without `--bare`. The banner has to be
///   the step's first `other` row (`session_banner_is_first_other_row`), so the supervisor buffers
///   until `init` — and the size of what it buffers is printed here.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_1_auth_and_the_first_stdin_message() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let mut verdicts = Vec::new();

    for (name, extra) in [("bare", vec!["--bare"]), ("plain", Vec::new())] {
        println!("\n=== case 1 / {name} ===");
        let cwd = scratch();
        let mut run = Run::start(cwd.path(), &args_with(&extra)).await;

        // The prompt is deliberately late: what lands in this window is the answer to "is `init`
        // emitted before the first stdin line is read".
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut before_prompt = Vec::new();
        while let Next::Line(line) = run.next(Duration::from_millis(50)).await {
            before_prompt.push(line.kind());
        }
        println!("  before the prompt was written: {before_prompt:?}");

        run.send("reply with exactly the word ok").await;
        let result = run.read_result(TURN_WINDOW).await;
        run.close_stdin();
        let drained = run.drain(EOF_WINDOW).await;
        let status = run.wait(EOF_WINDOW).await;

        let kinds = run.kinds();
        let saw_init = kinds.iter().any(|kind| kind == "system/init");
        let assistant_text: Vec<String> = run
            .json_lines()
            .iter()
            .filter(|line| line["type"] == json!("assistant"))
            .filter_map(|line| line["message"]["content"][0]["text"].as_str())
            .map(ToOwned::to_owned)
            .collect();
        let before_init = up_to_init(&kinds);
        let authenticated = result.as_ref().is_some_and(is_success);

        println!("  line order up to `system/init`: {before_init:?}");
        println!("  all line kinds: {kinds:?}");
        println!("  assistant text: {assistant_text:?}");
        match &result {
            Some(result) => println!("  result: {}", usage_line(result)),
            None => println!("  result: none arrived"),
        }
        println!(
            "  system/init seen: {saw_init}; drained to EOF in {drained:?}; authenticated: \
             {authenticated}"
        );

        run.finish(name, status).await;
        verdicts.push((
            name,
            authenticated,
            saw_init,
            before_init.len(),
            assistant_text,
        ));
        drop(cwd);
    }

    println!("\n--- case 1 verdict ---");
    for (name, authenticated, saw_init, before_init, text) in &verdicts {
        println!(
            "  {name:<6}: authenticated={authenticated} system/init={saw_init} \
             lines before and including init={before_init} assistant={text:?}"
        );
    }
    let bare = verdicts[0].1;
    let plain = verdicts[1].1;
    assert!(
        bare || plain,
        "neither invocation authenticated on this box, so nothing else in this file can run. \
         `--bare` needs ANTHROPIC_API_KEY (or an `apiKeyHelper`); the plain form needs the vendor's \
         own OAuth login — run `claude` once interactively and complete `/login`."
    );
    if plain && !bare {
        println!(
            "  D92 CONFIRMED: `--bare` does not authenticate here and the plain form does, which \
             is exactly the contradiction D92 names — a `billing: subscription` row plus `--bare` \
             is a row that cannot run. `--bare` stays out of the seed's `extra_args`."
        );
    } else if bare && plain {
        println!(
            "  D92 IS NOT LOAD-BEARING ON THIS BOX: both forms authenticated, so this box holds an \
             API key as well as (or instead of) an OAuth login. D92's *reasoning* is untested here \
             and the drop is still right for a subscription box; report it as inconclusive."
        );
    } else {
        println!(
            "  D92 CONTRADICTED: `--bare` authenticated and the plain form did not. D92 and the \
             seed row are both wrong as written — this needs the maintainer before T49 lands."
        );
    }
    println!(
        "  H-2 / D84: without `--bare` there were {} lines up to and including `system/init`, so \
         the supervisor's pre-init buffer is not decorative.",
        verdicts[1].3
    );
    println!(
        "  P-2: `-p` with no positional prompt and the prompt on stdin {} an assistant reply.",
        if verdicts.iter().any(|v| !v.4.is_empty()) {
            "produced"
        } else {
            "did NOT produce"
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Case 2 — D84, H-12, H-17: the session id is ours
// ---------------------------------------------------------------------------------------------

/// A minted UUIDv7 on `--session-id`, echoed by `system/init`, and then resumed.
///
/// D84 makes the session id `htui`'s to mint rather than the agent's to report, which is what lets
/// `AgentSession::session_ref` answer before the first line arrives and what a later `--resume`
/// uses. Two things could break that and neither is documented: the CLI's validator might insist on
/// a v4 (H-12 — a v7 has version nibble `7`, and the workspace mints v7 everywhere), and the CLI
/// might reassign the id it was given (H-17).
///
/// The id is a [`StepId`], which is the workspace's own v7 mint (`ids.rs`) — deliberately, so that
/// this case tests the id shape production will actually pass rather than one written for the test.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_2_session_id_is_ours() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let minted = StepId::new().to_string();
    println!(
        "\n=== case 2 / minted UUIDv7: {minted} (version nibble {}) ===",
        {
            // Character 14 of a hyphenated UUID is the version nibble.
            minted.chars().nth(14).unwrap_or('?')
        }
    );

    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &args_with(&["--session-id", &minted])).await;
    run.send("reply with exactly the word violet").await;
    let first = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let init = run
        .json_lines()
        .iter()
        .find(|line| line["type"] == json!("system") && line["subtype"] == json!("init"))
        .map(|line| (*line).clone());
    let echoed = init
        .as_ref()
        .and_then(|line| line["session_id"].as_str())
        .map(ToOwned::to_owned);
    println!("  system/init.session_id = {echoed:?}");
    println!(
        "  system/init.capabilities = {:?}",
        init.as_ref().map(|line| line["capabilities"].clone())
    );
    match &first {
        Some(result) => println!("  result: {}", usage_line(result)),
        None => println!("  result: none arrived"),
    }
    run.finish("session_id", status).await;
    drop(cwd);

    assert_eq!(
        echoed.as_deref(),
        Some(minted.as_str()),
        "H-12/H-17: the CLI did not carry the minted UUIDv7 through to `system/init`. If it \
         *refused* it, `uuid`'s `v4` joins the workspace feature list and D84's mint changes one \
         line; if it *rewrote* it, `session_ref` has to follow the wire instead of the mint. Either \
         way this is a finding for the maintainer, not a flake."
    );

    // The resume half of D84. A separate process, the same id, and a follow-up that can only be
    // answered from the first turn's context.
    println!("\n=== case 2 / resume ===");
    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &args_with(&["--resume", &minted])).await;
    run.send("what colour did you just say? answer with one word")
        .await;
    let second = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let resumed_ids: BTreeSet<String> = run
        .json_lines()
        .iter()
        .filter_map(|line| line["session_id"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let replies: Vec<String> = run
        .json_lines()
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["content"][0]["text"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    println!("  session ids on the resumed stream: {resumed_ids:?}");
    println!("  the resumed reply: {replies:?}");
    match &second {
        Some(result) => println!("  result: {}", usage_line(result)),
        None => println!("  result: none arrived"),
    }
    let carried = resumed_ids.contains(&minted);
    println!(
        "  D84 resume: the follow-up {} under the minted id.",
        if carried { "arrived" } else { "did NOT arrive" }
    );
    println!(
        "  whether the model recalled the first turn is printed, never asserted: the reply above \
         is the evidence, and asserting on a model's wording would be asserting on the vendor."
    );
    run.finish("resume", status).await;
    drop(cwd);

    assert!(
        carried,
        "D84's resume half: `--resume {minted}` produced a stream under {resumed_ids:?}. If the \
         CLI reassigns the id on resume, `session_ref` must follow the wire and the plan needs the \
         amendment."
    );
}

// ---------------------------------------------------------------------------------------------
// Case 3 — D81 and ANA-4 §11.14: what actually ends a turn
// ---------------------------------------------------------------------------------------------

/// The three cancellations, measured: stdin close alone, stdin close then SIGINT, and SIGTERM.
///
/// `docs/ANA-4.md` §4.4 leaves this **Unverified — MOD-2 must confirm**, with a hypothesis it does
/// not stand behind: *"SIGINT ends the turn cleanly while SIGTERM is reported to leave the turn
/// unfinished with exit 143"*. D81's cancel sequence (stdin close → SIGINT → grace → kill the
/// group) is written on that hypothesis and this case is what decides whether it survives.
///
/// **Nothing here asserts the hypothesis.** Each spawn records its terminal envelope (or the
/// absence of one), its exit status, its signal and its wall time to EOF, and the three are printed
/// side by side; the only assertions are that the process died and that the transcript was
/// recorded. A test written to confirm §4.4 would confirm it whatever the CLI did.
///
/// The prompt has to run long enough to be interrupted mid-turn, which is why this is the one case
/// with a prompt that is not one-word-answer short. `--include-partial-messages` gives a
/// `text_delta` to trigger on, so the signal lands while the model is *generating* rather than
/// while it is still assembling its context.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_3_cancellation_semantics() {
    let _serial = ONE_AT_A_TIME.lock().await;
    const PROMPT: &str = "count slowly from 1 to 200, one number per line, nothing else";
    let mut findings = Vec::new();

    for how in ["stdin_close", "sigint", "sigterm"] {
        println!("\n=== case 3 / {how} ===");
        let cwd = scratch();
        let mut run = Run::start(cwd.path(), &args_with(&["--include-partial-messages"])).await;
        run.send(PROMPT).await;

        // Trigger on the first text the model produced, by either route: a `text_delta` when
        // partial messages are flowing, or the whole `assistant` message if they are not.
        let trigger = run
            .read_until(TURN_WINDOW, |line| {
                line.json().is_some_and(|value| {
                    (value["type"] == json!("stream_event")
                        && value["event"]["delta"]["type"] == json!("text_delta"))
                        || value["type"] == json!("assistant")
                })
            })
            .await;
        println!("  triggered on: {:?}", trigger.as_ref().map(Line::kind));
        assert!(
            trigger.is_some(),
            "{how}: the turn produced no text to interrupt, so there is nothing to measure. This \
             is a precondition, not a finding."
        );

        let window = match how {
            // (a) is not a cancel at all: the turn runs to its own end and the closed stdin is what
            // stops a second one being asked for. It gets the long window on purpose.
            "stdin_close" => LONG_TURN_WINDOW,
            _ => CANCEL_WINDOW,
        };
        let acted = Instant::now();
        match how {
            "stdin_close" => run.close_stdin(),
            "sigint" => {
                run.close_stdin();
                run.signal(StopSignal::Interrupt);
            }
            // SIGTERM alone, stdin left open: (b) already measures the pair, and mixing them here
            // would leave "which one ended it" unanswerable.
            _ => run.signal(StopSignal::Terminate),
        }

        let to_eof = run.drain(window).await;
        let status = run.wait(EOF_WINDOW).await;
        let elapsed = acted.elapsed();

        let terminal = run
            .json_lines()
            .iter()
            .rev()
            .find(|line| line["type"] == json!("result"))
            .map(|line| (*line).clone());
        let after_action: Vec<String> = run.kinds();
        println!(
            "  {} lines in all; the last five were {:?}",
            after_action.len(),
            after_action.iter().rev().take(5).rev().collect::<Vec<_>>()
        );
        match &terminal {
            Some(result) => println!("  terminal envelope: {}", usage_line(result)),
            None => println!("  terminal envelope: NONE — the stream ended without a `result`"),
        }
        println!(
            "  EOF {} after the action; wall time {:.2}s",
            if to_eof.is_some() {
                "reached"
            } else {
                "NOT reached within the window"
            },
            elapsed.as_secs_f64()
        );

        let status = run.finish(how, status).await;
        findings.push((
            how,
            terminal.clone(),
            status.code(),
            signal_of(&status),
            to_eof.is_some(),
            elapsed,
        ));
        drop(cwd);
    }

    println!("\n--- case 3 verdict (D81's middle step, ANA-4 §11.14) ---");
    for (how, terminal, code, signal, reached_eof, elapsed) in &findings {
        println!(
            "  {how:<12} exit={code:?} signal={signal:?} eof={reached_eof} \
             wall={:.2}s result.subtype={:?} result.is_error={:?} result.stop_reason={:?} \
             result.terminal_reason={:?} result.num_turns={:?}",
            elapsed.as_secs_f64(),
            terminal
                .as_ref()
                .and_then(|value| value["subtype"].as_str().map(ToOwned::to_owned)),
            terminal.as_ref().map(|value| value["is_error"].clone()),
            terminal
                .as_ref()
                .and_then(|value| value["stop_reason"].as_str().map(ToOwned::to_owned)),
            terminal
                .as_ref()
                .and_then(|value| value["terminal_reason"].as_str().map(ToOwned::to_owned)),
            terminal
                .as_ref()
                .and_then(|value| value["num_turns"].as_i64()),
        );
    }
    println!(
        "  These three rows are what fills B.4's `stop_reason` rows marked `?` and B.7's cancel \
         shape. ANA-4 §4.4's hypothesis (SIGINT clean, SIGTERM unfinished at 143) is *not* asserted \
         anywhere above: compare it against the rows and amend the document, never the other way."
    );
}

// ---------------------------------------------------------------------------------------------
// Case 4 — E-2: are `modelUsage[*]` counts per-turn or cumulative?
// ---------------------------------------------------------------------------------------------

/// Two turns in one process, and the arithmetic that tells the two conventions apart.
///
/// B.4's mapper subtracts the previous `result`'s sums when `tokens_are_cumulative` is set, and
/// gets `run_step.usage` wrong on turn 2 either way round if the flag is guessed (H-4): a
/// per-turn stream double-counts if it subtracts, a cumulative one double-counts if it does not.
/// Criterion 7 — the step's usage is the sum of its rows — is what breaks.
///
/// The discriminator is **output tokens**, not input: each `assistant` line carries its own
/// message's `usage.output_tokens`, so turn 2's own production is known independently of the
/// `result`. Then exactly one of two arithmetics can hold:
///
/// - per-turn: `Σ modelUsage.outputTokens` at result 2 == turn 2's own assistant output;
/// - cumulative: `Σ modelUsage.outputTokens` at result 2 == result 1's sum + turn 2's own.
///
/// A run where neither holds is a genuine unknown — a third convention, or a figure that counts
/// something else — and it **fails with both candidates printed**, because that is precisely the
/// case where a guess would be most expensive.
///
/// This is also the fixture T51 maps its two-turn snapshot from, which is why it runs with
/// `--include-partial-messages`: the deltas and the assembled `assistant` message both have to be
/// in the transcript for H-8's "text streams once, not twice" to be provable off it.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_4_usage_across_two_turns() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 4 / two turns, one process ===");
    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &args_with(&["--include-partial-messages"])).await;

    run.send("reply with exactly the word ok").await;
    let first = run
        .read_result(TURN_WINDOW)
        .await
        .expect("turn 1 produced a `result`");
    let after_first = run.json_lines().len();

    run.send("and one more word").await;
    let second = run
        .read_result(TURN_WINDOW)
        .await
        .expect("turn 2 produced a `result`");
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    println!("  turn 1 result: {}", usage_line(&first));
    println!("  turn 2 result: {}", usage_line(&second));
    println!(
        "  turn 1 modelUsage: {}",
        serde_json::to_string(&first["modelUsage"]).unwrap_or_default()
    );
    println!(
        "  turn 2 modelUsage: {}",
        serde_json::to_string(&second["modelUsage"]).unwrap_or_default()
    );

    // Turn 2's own production, from the assistant messages that arrived after result 1.
    let lines = run.json_lines();
    let turn_2_output: i64 = lines
        .iter()
        .skip(after_first)
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["usage"]["output_tokens"].as_i64())
        .sum();
    let sum_1 = model_usage_sum(&first, "outputTokens");
    let sum_2 = model_usage_sum(&second, "outputTokens");
    println!(
        "  Σ modelUsage.outputTokens: turn 1 = {sum_1:?}, turn 2 = {sum_2:?}; turn 2's own \
         assistant output = {turn_2_output}"
    );

    run.finish("turns", status).await;
    drop(cwd);

    let (Some(sum_1), Some(sum_2)) = (sum_1, sum_2) else {
        panic!(
            "E-2 is unanswerable from this run: `modelUsage` was absent or empty on at least one \
             turn ({sum_1:?} then {sum_2:?}). B.4's D86 table says an empty map answers `null`, so \
             this is a real shape — but it is not one this case can measure the convention from."
        );
    };
    let per_turn = sum_2 == turn_2_output;
    let cumulative = sum_2 == sum_1 + turn_2_output;
    assert!(
        per_turn ^ cumulative,
        "E-2: neither convention explains the figures, or both do. turn 1 Σ = {sum_1}, turn 2 Σ = \
         {sum_2}, turn 2's own assistant output = {turn_2_output}. per_turn candidate = \
         {turn_2_output}, cumulative candidate = {}. This is a finding for the maintainer: B.4's \
         `tokens_are_cumulative` cannot be set from a guess.",
        sum_1 + turn_2_output
    );
    println!(
        "  VERDICT: tokens_are_cumulative = {cumulative} (per-turn candidate {turn_2_output}, \
         cumulative candidate {}, observed {sum_2})",
        sum_1 + turn_2_output
    );
    println!(
        "  cost: total_cost_usd went {:?} -> {:?}, so `cost_micros_total` is cumulative and \
         `cost_micros` is the delta B.4 makes it.",
        first["total_cost_usd"].as_f64(),
        second["total_cost_usd"].as_f64()
    );
}

// ---------------------------------------------------------------------------------------------
// Case 5 — D-2's "not a cancel" path
// ---------------------------------------------------------------------------------------------

/// Stdin closed after a `result`: EOF and a clean exit, promptly.
///
/// The supervisor's drain (blueprint D-2) treats an EOF **with a turn still open** as an error plus
/// a synthesized `done{cancelled}`, and an EOF *between* turns as the ordinary end of a session.
/// The second half only holds if closing stdin after a `result` really does end the process, and
/// promptly: a CLI that lingered would make every clean shutdown look like a hang.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_5_stdin_eof_between_turns_exits_clean() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 5 / stdin EOF between turns ===");
    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &base_args()).await;
    run.send("reply with exactly the word ok").await;
    let result = run
        .read_result(TURN_WINDOW)
        .await
        .expect("the turn produced a `result`");
    println!("  result: {}", usage_line(&result));

    let closed = Instant::now();
    run.close_stdin();
    let to_eof = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;
    println!(
        "  EOF {:?} after the close; exit {:?}",
        to_eof,
        status.map(|status| (status.code(), signal_of(&status)))
    );
    let status = run.finish("eof", status).await;
    drop(cwd);

    assert!(
        to_eof.is_some(),
        "D-2: stdin closed after a `result` did not reach EOF within {EOF_WINDOW:?}. The \
         supervisor's clean-shutdown path assumes it does; if it does not, the shutdown needs its \
         own kill after the grace."
    );
    assert_eq!(
        status.code(),
        Some(0),
        "a session that ended between turns exited {status:?} rather than 0 (closed after \
         {:.2}s)",
        closed.elapsed().as_secs_f64()
    );
}

// ---------------------------------------------------------------------------------------------
// Case 6 — D82: what a thinking block looks like
// ---------------------------------------------------------------------------------------------

/// A turn invited to reason, with `--include-partial-messages`, recorded whatever it does.
///
/// §6.2's `thought` row is marked **Unverified — MOD-2 must confirm** for one reason: thinking was
/// off in the runs `docs/ANA-4.md` was written from, and `usage.output_tokens_details.thinking_tokens`
/// proved only that the channel exists. D82 says the fixture wins over the document if they
/// disagree.
///
/// A run where thinking stays off is **not a failure** — the fixture is the answer either way, and
/// this box's model, effort setting and account can all turn it off without anything being wrong.
/// What is printed is every `content_block_start` type and every delta type seen, which is exactly
/// the vocabulary B.4's `stream_event` rows are written against.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_6_thinking_blocks() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 6 / thinking ===");
    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &args_with(&["--include-partial-messages"])).await;
    run.send("think step by step, then answer: what is 17 × 23?")
        .await;
    let result = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let lines = run.json_lines();
    let block_types: BTreeSet<String> = lines
        .iter()
        .filter(|line| line["event"]["type"] == json!("content_block_start"))
        .filter_map(|line| line["event"]["content_block"]["type"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let delta_types: BTreeSet<String> = lines
        .iter()
        .filter(|line| line["event"]["type"] == json!("content_block_delta"))
        .filter_map(|line| line["event"]["delta"]["type"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let assistant_block_types: BTreeSet<String> = lines
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter_map(|block| block["type"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let thinking_tokens = result
        .as_ref()
        .and_then(|value| value["usage"]["output_tokens_details"]["thinking_tokens"].as_i64());

    println!("  stream_event content_block_start types: {block_types:?}");
    println!("  stream_event content_block_delta types: {delta_types:?}");
    println!("  assistant message content[] types: {assistant_block_types:?}");
    println!("  usage.output_tokens_details.thinking_tokens = {thinking_tokens:?}");
    if let Some(result) = &result {
        println!("  result: {}", usage_line(result));
    }

    let thought = block_types.contains("thinking")
        || delta_types.contains("thinking_delta")
        || assistant_block_types.contains("thinking");
    if thought {
        println!(
            "  D82: thinking arrived. The types above are what B.4 maps to `ThoughtChunk`; if they \
             disagree with §6.2, the fixture wins and §6.2 is amended."
        );
    } else {
        println!(
            "  D82: thinking was off in this run. Not a failure — the fixture records what the CLI \
             sent, and the mapper's `thinking` arms stay written against §6.2 until a run produces \
             one. thinking_tokens = {thinking_tokens:?} says whether the channel was used at all."
        );
    }

    run.finish("thinking", status).await;
    drop(cwd);
}

// ---------------------------------------------------------------------------------------------
// Case 7 — D86: `result.usage` and `Σ modelUsage` disagree once a subagent runs
// ---------------------------------------------------------------------------------------------

/// A turn with one subagent in it, so the two token figures visibly differ.
///
/// D86 sums `modelUsage[*]` rather than reading `result.usage`, on §7's grounds that `result.usage`
/// "counts only the top-level loop and undercounts as soon as a subagent runs". That sentence is
/// the whole basis of the choice and nothing in the repository has ever watched it happen. A
/// subagent is what makes the gap appear; without one the two figures agree and the decision looks
/// arbitrary.
///
/// The prompt is not one-word-answer short because it cannot be — it has to spawn a second model
/// loop. It is still as small as the question allows: one subagent, one word back.
///
/// A run where no subagent starts prints that and asserts nothing, for this file's rule: the
/// tool-availability of a `-p` run is the box's business, and a case that demanded one would fail
/// on a policy rather than on a finding.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_7_subagent_makes_model_usage_and_usage_disagree() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 7 / subagent ===");
    let cwd = scratch();
    let mut run = Run::start(
        cwd.path(),
        &args_with(&["--permission-mode", "acceptEdits"]),
    )
    .await;
    run.send(
        "use the Task tool exactly once to launch a general-purpose subagent whose entire job is \
         to reply with the single word ok; then reply with that word and nothing else",
    )
    .await;
    let result = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let lines = run.json_lines();
    let tool_names: Vec<String> = lines
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == json!("tool_use"))
        .filter_map(|block| block["name"].as_str())
        .map(ToOwned::to_owned)
        .collect();
    let with_parent = lines
        .iter()
        .filter(|line| !line["parent_tool_use_id"].is_null())
        .count();
    println!("  tool calls: {tool_names:?}");
    println!("  lines carrying a `parent_tool_use_id`: {with_parent}");
    if let Some(result) = &result {
        println!("  result: {}", usage_line(result));
        println!(
            "  result.subagent_stats = {}",
            serde_json::to_string(&result["subagent_stats"]).unwrap_or_default()
        );
        println!(
            "  result.modelUsage = {}",
            serde_json::to_string(&result["modelUsage"]).unwrap_or_default()
        );
    }

    let spawned = result
        .as_ref()
        .and_then(|value| value["subagent_stats"]["spawned"].as_i64())
        .unwrap_or(0);
    let top_level = result
        .as_ref()
        .and_then(|value| value["usage"]["input_tokens"].as_i64());
    let summed = result
        .as_ref()
        .and_then(|value| model_usage_sum(value, "inputTokens"));
    println!("  subagents spawned = {spawned}");
    println!(
        "  result.usage.input_tokens = {top_level:?} vs Σ modelUsage.inputTokens = {summed:?}"
    );

    match (spawned > 0, top_level, summed) {
        (true, Some(top_level), Some(summed)) if top_level != summed => println!(
            "  D86 CONFIRMED: a subagent ran and the two figures differ by {}. `result.usage` \
             undercounts, which is why the mapper sums `modelUsage[*]`.",
            summed - top_level
        ),
        (true, Some(top_level), Some(summed)) => println!(
            "  D86 INCONCLUSIVE: a subagent ran and the two figures agree ({top_level} == \
             {summed}). §7's claim is not reproduced by this run; report it, and note that summing \
             `modelUsage` is still correct — it is simply not *more* correct here."
        ),
        _ => println!(
            "  D86 UNMEASURED: no subagent ran in this turn (spawned = {spawned}), so there is \
             nothing for the two figures to disagree about. The fixture still records the shape."
        ),
    }

    run.finish("subagent", status).await;
    drop(cwd);
}

// ---------------------------------------------------------------------------------------------
// Case 8 — D83, H-16: `--max-budget-usd` at the bottom of its range
// ---------------------------------------------------------------------------------------------

/// A budget small enough to trip immediately, and a budget of zero.
///
/// D83 passes `project.settings.per_token_cap_run` as a second, server-side cap. Two things had to
/// be measured before the flag could be wired: what a breach *looks like* (B.4's `result` row needs
/// a `subtype` and an `is_error` to map), and what the CLI does with `0` (H-16 — a caller that
/// meant "no spending" and got "no budget" would have the cap silently removed).
///
/// The zero run is recorded as a transcript like any other even though it may never reach the
/// model: an argv the CLI **refuses** is a finding, and the exit status and stderr are the whole of
/// it.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_8_budget_trips_server_side() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let mut findings = Vec::new();

    for (name, budget) in [("budget", "0.000001"), ("budget_zero", "0.000000")] {
        println!("\n=== case 8 / --max-budget-usd {budget} ===");
        let cwd = scratch();
        let mut run = Run::start(cwd.path(), &args_with(&["--max-budget-usd", budget])).await;
        // A refused argv exits before it reads stdin, and writing to a closed pipe is an error
        // this case reports rather than panics on.
        let message = user_message("reply with exactly the word ok");
        let payload = format!(
            "{}\n",
            serde_json::to_string(&message).expect("a user message serialises")
        );
        let accepted = match run.stdin.as_mut() {
            Some(stdin) => stdin.write_all(payload.as_bytes()).await.is_ok(),
            None => false,
        };
        if accepted {
            run.records
                .push(json!({ "direction": "stdin", "line": message }));
        }
        println!(
            "  the CLI {} the prompt on stdin",
            if accepted { "accepted" } else { "refused" }
        );

        let result = run.read_result(TURN_WINDOW).await;
        run.close_stdin();
        let _ = run.drain(EOF_WINDOW).await;
        let status = run.wait(EOF_WINDOW).await;
        match &result {
            Some(result) => println!("  result: {}", usage_line(result)),
            None => println!("  result: none — the run produced no terminal envelope"),
        }
        println!("  line kinds: {:?}", run.kinds());
        let status = run.finish(name, status).await;
        findings.push((budget, result, status.code(), accepted));
        drop(cwd);
    }

    println!("\n--- case 8 verdict (D83, H-16) ---");
    for (budget, result, code, accepted) in &findings {
        println!(
            "  --max-budget-usd {budget:<9} exit={code:?} stdin_accepted={accepted} \
             subtype={:?} is_error={:?} result={:?}",
            result
                .as_ref()
                .and_then(|value| value["subtype"].as_str().map(ToOwned::to_owned)),
            result.as_ref().map(|value| value["is_error"].clone()),
            result
                .as_ref()
                .and_then(|value| value["result"].as_str().map(ToOwned::to_owned)),
        );
    }
    let zero_ran = findings[1].1.is_some();
    println!(
        "  H-16: at `0.000000` the CLI {}. If it refused the argv, D83's call site must omit the \
         flag at zero and the recorder's own cap (milestone 7 H-5) is the whole guard; if it ran \
         unbudgeted, the same omission is required for the opposite reason.",
        if zero_ran {
            "started a turn"
        } else {
            "produced no turn at all"
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Case 9 — D85, E-1: what a policy denial looks like
// ---------------------------------------------------------------------------------------------

/// `--permission-mode default` and a prompt that needs `Bash`, so the CLI denies it itself.
///
/// This is the fixture `DriverEvent::PermissionAnswer` is mapped from (D93's twelfth variant). B.4's
/// `denials_of` renames `result.permission_denials[]`'s vendor keys onto the row's column names,
/// and B.7's `PolicyDenied` line is written off the same transcript — both need to see one.
///
/// It also prints every `rate_limit_event` any case in this process has seen and its nesting, which
/// is blueprint §F-T56's last sentence: B.4 holds the blob until the turn's one `usage` row, and
/// whether the blob is the whole line's body or the `rate_limit_info` inside it decides what
/// `quota.rs`'s E-3 arm normalises.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn case_9_a_denied_tool() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 9 / a denied tool ===");
    let cwd = scratch();
    let mut run = Run::start(cwd.path(), &args_with(&["--permission-mode", "default"])).await;
    run.send("run `ls` with the Bash tool, then reply with the single word done")
        .await;
    let result = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let lines = run.json_lines();
    let calls: Vec<(String, String)> = lines
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == json!("tool_use"))
        .map(|block| {
            (
                block["name"].as_str().unwrap_or_default().to_owned(),
                block["id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    println!("  tool calls: {calls:?}");
    for line in lines
        .iter()
        .filter(|line| line["type"] == json!("user"))
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == json!("tool_result"))
    {
        println!(
            "  tool_result: {}",
            serde_json::to_string(line).unwrap_or_default()
        );
    }
    let denials = result
        .as_ref()
        .map(|value| value["permission_denials"].clone())
        .unwrap_or(Value::Null);
    println!(
        "  result.permission_denials = {}",
        serde_json::to_string_pretty(&denials).unwrap_or_default()
    );
    let system_denials: Vec<&&Value> = lines
        .iter()
        .filter(|line| line["subtype"] == json!("permission_denied"))
        .collect();
    println!(
        "  `system/permission_denied` lines: {}",
        system_denials.len()
    );
    if let Some(result) = &result {
        println!("  result: {}", usage_line(result));
    }

    let denied = denials.as_array().is_some_and(|items| !items.is_empty());
    if denied {
        println!(
            "  D85/E-1: the denial arrives on `result.permission_denials[]`, with the keys above. \
             B.4's `denials_of` renames `tool_use_id` -> `tool_call_id` and keeps the rest, and the \
             recorder stamps the row `by: policy, denied: true`."
        );
    } else {
        println!(
            "  D85/E-1 UNMEASURED: nothing was denied in this run — the model may not have reached \
             for `Bash`, or this box's settings pre-allow it. The transcript records what happened; \
             the mapper's `denials_of` arm stays written against §6.2."
        );
    }

    run.finish("denied", status).await;
    drop(cwd);

    println!("\n--- the quota blob (B.4's `pending_quota`) ---");
    let seen = RATE_LIMITS
        .lock()
        .expect("the rate-limit log is not poisoned")
        .clone();
    println!(
        "  `rate_limit_event` lines seen by the cases that ran in this process: {}",
        seen.len()
    );
    if let Some(last) = seen.last() {
        println!(
            "  the last one, verbatim: {}",
            serde_json::to_string_pretty(last).unwrap_or_default()
        );
        println!(
            "  nesting: top-level keys = {:?}; `rate_limit_info` present = {}",
            last.as_object()
                .map(|map| map.keys().cloned().collect::<Vec<_>>()),
            !last["rate_limit_info"].is_null()
        );
    } else {
        println!(
            "  none arrived. B.4's `pending_quota` stays `None` on such a turn, which is the shape \
             milestone 7's H-3 already covers: the first turn without a blob publishes nothing."
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Case 10 — the denial case 9 asked for and did not get
// ---------------------------------------------------------------------------------------------

/// **D85, measured.** What a policy denial looks like on the wire, which is what
/// `cli/claude.rs`'s `denials_of` is mapped from.
///
/// # Why there is a tenth case
///
/// Case 9 was written to provoke a denial and did not. Its transcript
/// (`claude_stream_json_denied.jsonl`) shows the `Bash` call **succeeding** —
/// `"(Bash completed with no output)"` — and `result.permission_denials` empty, so D85 shipped
/// implemented and unproven (plan F-12) on one hand-written line in `tests/cli_map.rs`.
///
/// The reason it failed is worth more than the case: case 9 asked for **`ls`**, and this box's
/// `~/.claude/settings.json` carries `Bash(ls *)` in its allow list. It picked the one command the
/// box pre-approves. That is not a flaw in the box — it is the hazard in probing a *policy*, which
/// is per-machine state by definition, and any case that depends on a particular box's settings
/// proves something about that box rather than about the dialect.
///
/// So this case does not depend on them. **`--permission-prompts none`** is documented by
/// `claude --help` as "nobody: anything that would prompt is denied automatically; the permission
/// mode still decides everything else", which makes the denial a property of the invocation rather
/// than of the allow list. `--permission-mode default` keeps the mode from pre-approving edits the
/// way the seed's `acceptEdits` would, and the tool asked for is one no plausible allow list
/// carries.
///
/// # What it is allowed to conclude
///
/// A **non-empty** `result.permission_denials[]` proves the shape `denials_of` reads. An empty one
/// after this invocation is itself a finding — it would mean the flag does not do what its help
/// text says, and that `permission_denials` is not the channel §6.2 assigns it — so the case fails
/// loudly rather than printing a shrug the way case 9 did. A probe that cannot fail cannot prove.
#[tokio::test]
#[ignore = "spawns the real claude CLI and spends model tokens"]
async fn case_10_a_denial_that_does_not_depend_on_this_box() {
    let _serial = ONE_AT_A_TIME.lock().await;
    println!("\n=== case 10 / a denial that does not depend on this box ===");
    let cwd = scratch();
    let mut run = Run::start(
        cwd.path(),
        &args_with(&[
            "--permission-mode",
            "default",
            // The lever. Without it the CLI waits for a "host" to answer the prompt — which over
            // this transport is nobody, because §4.3 fixes `permission_requests: false` — and the
            // turn's outcome would be a timeout rather than a denial.
            "--permission-prompts",
            "none",
        ]),
    )
    .await;
    run.send(
        "use the Write tool to create a file called note.txt containing the word hello, then \
         reply with the single word done",
    )
    .await;
    let result = run.read_result(TURN_WINDOW).await;
    run.close_stdin();
    let _ = run.drain(EOF_WINDOW).await;
    let status = run.wait(EOF_WINDOW).await;

    let lines = run.json_lines();
    let calls: Vec<(String, String)> = lines
        .iter()
        .filter(|line| line["type"] == json!("assistant"))
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == json!("tool_use"))
        .map(|block| {
            (
                block["name"].as_str().unwrap_or_default().to_owned(),
                block["id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    println!("  tool calls: {calls:?}");

    let denials = result
        .as_ref()
        .map(|result| result["permission_denials"].clone())
        .unwrap_or(Value::Null);
    println!(
        "  result.permission_denials = {}",
        serde_json::to_string_pretty(&denials).unwrap_or_default()
    );
    println!(
        "  the file the tool was refused: exists = {}",
        cwd.path().join("note.txt").exists()
    );

    run.finish("policy_denied", status).await;

    let entries = denials.as_array().cloned().unwrap_or_default();
    assert!(
        !entries.is_empty(),
        "D85 is unproven and this case exists to prove it. `--permission-prompts none` says \
         anything that would prompt is denied automatically, and a `Write` under \
         `--permission-mode default` is such a thing. An empty array here means the denial is \
         **not** reported through `result.permission_denials[]`, which is the channel \
         `docs/ANA-4.md` §6.2 assigns it and the one `cli/claude.rs::denials_of` reads — a finding \
         for the maintainer, not a flake. The tool calls this turn made were: {calls:?}"
    );

    // The keys `denials_of` reads, asserted by name so a rename is a failure here rather than a
    // silently empty column in the transcript.
    let first = &entries[0];
    println!(
        "  the first denial, verbatim: {}",
        serde_json::to_string_pretty(first).unwrap_or_default()
    );
    assert!(
        first["tool_use_id"].is_string(),
        "`tool_use_id` is what joins the answer to the call it settled, and is the only key \
         `denials_of` cannot do without: {first}"
    );
    assert!(
        first["tool_name"].is_string(),
        "`tool_name` names what was refused: {first}"
    );

    let ids: BTreeSet<&str> = calls.iter().map(|(_, id)| id.as_str()).collect();
    let denied_id = first["tool_use_id"].as_str().unwrap_or_default();
    assert!(
        ids.contains(denied_id),
        "the denied id is one of the turn's own tool calls, which is what makes the row joinable \
         at all — {denied_id} is not in {ids:?}"
    );

    println!(
        "\n  D85 MEASURED: the denial is reported on the terminal `result`, in \
         `permission_denials[]`, keyed by the call's own `tool_use_id`. `cli/claude.rs`'s \
         `denials_of` reads exactly these keys, and `tests/cli_map.rs` can stop calling its \
         coverage synthetic."
    );
}
