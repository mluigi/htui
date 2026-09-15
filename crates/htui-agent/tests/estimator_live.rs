//! The estimator differential: `chars-v2`'s constants re-measured against the live `claude` CLI
//! (plan T69, D99, **D108**; blueprint §T69).
//!
//! `#[ignore]` by default, exactly as `tests/cli_live.rs`, `tests/agy_live.rs` and
//! `tests/probe_live.rs` are, and for `cli_live.rs`'s reason: **it spends real model tokens** on
//! whatever credential this box holds, so nothing in it may ever run from a plain `cargo test`. Run
//! it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test estimator_live -- --ignored --nocapture
//! ```
//!
//! # What it measures, and why it is a *differential*
//!
//! [`htui_core::prompt::estimate::TokenEstimator::DEFAULT`] says a character of Claude-family prose
//! costs 1/2.5 of a token and a character of code 1/2.4. Those two numbers decide every trim in the
//! assembler, and **no assertion inside `htui-core` can check them**: the trim tests all assert
//! *internal* arithmetic — that `tokens_after` sums to `estimated_after`, that `estimated_after <=
//! target` — and a wrong constant satisfies every one of them perfectly. That is exactly how ANA-5
//! §4.4's published 3.5 / 3.0 nearly shipped while reality was 2.51 / 2.44 (finding F-16). The only
//! thing that can contradict a constant is the model, so this file asks the model.
//!
//! Four turns, the shape the 2026-09-11 probe ran:
//!
//! | run | corpus | what it establishes |
//! |---|---|---|
//! | baseline | — | what the CLI's own system prompt costs before a corpus is added |
//! | prose | 40 000 chars | the prose rate |
//! | prose, half | 20 000 chars | that the relation is *linear*, i.e. the baseline subtraction is sound |
//! | code | 40 000 chars | the code rate |
//!
//! What it measured when it was committed, against `claude` **2.1.272**, model `claude-opus-5[1m]`
//! — the first run of this file as a file, and the confirmation D108 asked for:
//!
//! | run | chars | total input tokens | delta | chars/token | F-16, 2026-09-11 |
//! |---|---|---|---|---|---|
//! | baseline | — | 17 327 | — | — | 17 150 |
//! | prose | 40 002 | 33 672 | 16 345 | **2.447** | 2.507 |
//! | prose, half | 20 002 | 25 494 | 8 167 | **2.449** | 2.494 |
//! | code | 40 002 | 33 492 | 16 165 | **2.475** | 2.440 |
//!
//! `chars-v2` holds: prose 2.1% under its 2.5 and code 3.1% over its 2.4, both far inside the
//! ±[`TOLERANCE`] this file allows, and the two prose sizes 0.07% apart. The prose figure sits
//! below F-16's because the corpus is a different sample of ANA-5 (see F-61 below), not because
//! anything moved; the code corpus is unchanged from the probe's and 2.475 against 2.440 is 1.4%.
//! The baseline rose 17 150 → 17 327 with the CLI's own five patch versions, which is exactly what
//! the subtraction exists to absorb: it moves the baseline and not one ratio.
//!
//! The reply is pinned to one word ([`BASELINE_PROMPT`]) so the model's own production cannot move
//! an input figure, and each turn runs in its own throwaway working directory and its own process,
//! so no turn inherits another's context.
//!
//! ## The method note that is the whole point (F-16)
//!
//! Total input for a turn is
//!
//! ```text
//! result.usage.input_tokens
//!   + result.usage.cache_creation_input_tokens
//!   + result.usage.cache_read_input_tokens
//! ```
//!
//! — **all three**. The CLI's own system prompt lands in the cache fields, so `input_tokens` alone
//! reports **2** on a turn that really sent seventeen thousand tokens (see
//! `tests/fixtures/claude_stream_json_plain.jsonl`, whose `result` says exactly that). Reading the
//! first field alone is why the milestone-8 fixtures could not calibrate anything, and it is the
//! trap this file exists to keep shut: [`total_input`] is the one place the sum is spelled, and
//! [`ONLY_INPUT_TOKENS_IS_THE_TRAP`] pins the shape of the mistake so a future edit that "simplifies"
//! the sum fails here rather than in a trim six months later.
//!
//! ## The linearity assertion
//!
//! Two prose sizes, not one, and [`PROSE_LINEARITY`] demands they agree to 2%. A ratio is
//! `added_chars / (total - baseline)`, so a baseline that was measured wrong — a different system
//! prompt, a cached turn counted twice, a `result` from the wrong turn — biases the two sizes by
//! *different* amounts and the disagreement shows. Without it a broken subtraction would still
//! produce a plausible single number, and a plausible single number is how a constant gets
//! re-derived from nothing.
//!
//! ## The corpora are **committed**, not generated
//!
//! `tests/fixtures/estimator_prose.txt` (`docs/ANA-5.md` with its fenced blocks stripped) and
//! `tests/fixtures/estimator_code.txt` (`crates/htui-core/src/store/conformance.rs`), 40 000
//! characters each, frozen and read by `include_str!`. Two rejected alternatives, and why:
//!
//! * **reading those two source files live**, which is what the plan's probe did. The corpus then
//!   changes whenever a doc is edited or `conformance.rs` gains a case — and when this file later
//!   fails, nobody can say whether the tokenizer moved or the corpus did. A test that defends a
//!   constant has to hold everything but the model still;
//! * **generating text in the test** from a seeded word list. Deterministic, but synthetic: the
//!   ratio of a Markov-ish word salad is not the ratio of English technical prose or of Rust, and
//!   `chars-v2` is a claim about the second kind. The measurement would be repeatable and about the
//!   wrong thing.
//!
//! ### The prose corpus is a sample, doubled — and that is what makes the linearity check mean
//! something (F-61)
//!
//! [`PROSE`] is **20 000 characters written twice**: its two halves are byte-identical, and
//! [`assert_corpora_are_intact`] fails the run before a token is spent if a
//! regeneration ever breaks that. The 20 000-character unit is itself 20 blocks of 1 000 characters
//! spread evenly across the whole fence-stripped document, so it carries the document's real mix of
//! paragraphs, tables, `R-PRM-4`-style ids and section numbers rather than one stretch of it.
//!
//! The first version of this file used the plain **prefix** of a contiguous 40 000-character slice,
//! and the live run rejected it: 2.501 chars/token over the full slice against 2.393 over its first
//! half, 4.4% apart and over the limit. Nothing was wrong with the baseline — the first 20 000
//! characters of ANA-5 are simply denser than the rest (771 digits and 625 backticks against 247
//! and 472; the header, the scope note and the requirement tables), and a denser half tokenizes to
//! more tokens per character. A prefix is not a sample.
//!
//! That matters because of what the check is *for*. It asks "do two sizes of the same text give the
//! same ratio", and answers "no" when the baseline subtraction is wrong. With a prefix it answers
//! "no" when the baseline is wrong **or** when the two stretches of text differ, and the failure
//! message cannot tell the reader which — a check with two causes and one message diagnoses
//! neither. Doubling removes the second cause by construction: the halves *are* the same text, so a
//! disagreement can only be arithmetic.
//!
//! The cost is that the prose rate is measured over 20 000 unique characters rather than 40 000,
//! which is no cost at all: characters per token is a per-character property, the 2026-09-11 probe
//! got 2.494 from 20 000 characters and 2.507 from 40 000, and the doubled corpus still puts 40 000
//! characters on the wire.
//!
//! ## What it does *not* do
//!
//! It pins no model id. The constants are a property of the Claude tokenizer, not of one snapshot
//! of one model, and a test that hard-coded `claude-opus-5[1m]` would start failing for a reason
//! that has nothing to do with the estimator. The models the turns actually ran on are **printed**,
//! from `result.modelUsage`, so the run's own output says what was measured.
//!
//! It also does not pass `--tools ""`, though that would make a tool call impossible. Disabling the
//! tools would shrink the system prompt and make this baseline incomparable with F-16's 17 150 — and
//! F-16's figures are the numbers a drift here has to be argued against. Instead the risk is
//! *asserted away*: a turn that called a tool would run a second API request, count its context
//! twice under `cache_read_input_tokens`, and wreck the delta — so every run must report
//! `num_turns == 1`, and a run that does not fails by name and asks for a rerun rather than quietly
//! producing a ratio.
//!
//! Reading the seed row's `claude` tool probe is data, not a code path keyed on an agent name
//! (`R-AGT-5`); `tests/cli_live.rs`, `tests/probe_live.rs` and `tests/agy_live.rs` do the same. Test
//! files are exempt from `tests/extensibility.rs`'s vendor sweeps, which scan `src/` only.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Utc;
use htui_agent::launch::{AgentLaunch, Discovery, ResolvedLaunch, Spawned, ToolProbe};
use htui_core::prompt::estimate::TokenEstimator;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::ChildStdout;

// ---------------------------------------------------------------------------------------------
// The corpora and the shape of the measurement
// ---------------------------------------------------------------------------------------------

/// 40 000 characters of technical English: `docs/ANA-5.md` with its fenced blocks stripped, frozen.
///
/// **20 000 characters, twice** — the module doc's F-61 says why at length. The unit is 20 blocks
/// of 1 000 characters spread evenly across the whole document, so it samples ANA-5's mix rather
/// than one stretch of it, and [`assert_corpora_are_intact`] holds the doubling.
const PROSE: &str = include_str!("fixtures/estimator_prose.txt");

/// 40 000 characters of Rust: `crates/htui-core/src/store/conformance.rs` from the top, frozen.
///
/// Contiguous, unlike [`PROSE`], because nothing differences it against a half of itself: it is
/// read once, at one size, for the code rate.
const CODE: &str = include_str!("fixtures/estimator_code.txt");

/// Both corpora are exactly this many **characters** — not bytes; the prose one holds multi-byte
/// punctuation and is 40 106 bytes long. Asserted at the top of the test, before a token is spent.
const CORPUS_CHARS: usize = 40_000;

/// The half-size prose run: the first half of [`PROSE`], which is the whole of [`PROSE`]'s text.
const HALF_CHARS: usize = 20_000;

/// The turn whose only job is to establish what the CLI's own system prompt costs.
///
/// One word of reply, so `output_tokens` cannot move an input figure, and the same instruction
/// prefixes every corpus turn — so the difference between two turns is the corpus and the two
/// newlines that separate it, and nothing else.
const BASELINE_PROMPT: &str = "Reply with exactly the word: ok";

/// How far a measured ratio may sit from the constant it defends, as a fraction.
///
/// 10%, and the number is chosen from both sides. Measurement noise is far below it: the two prose
/// runs of 2026-09-11 landed 0.5% apart, and the baseline subtraction removes every fixed cost the
/// CLI adds. The drift worth catching is far above it: F-16's published-versus-measured gap was
/// 28.4%, and the vendor note it came from ("approximately 30 percent more tokens than on earlier
/// models") describes the size of a real tokenizer change. A tolerance between the two fails on the
/// thing that matters and not on the weather.
///
/// **If a run breaches this, the tolerance is not the thing to change.** Report the numbers; the
/// constants in `htui-core`'s `estimate.rs` and the `chars-v2` id are the maintainer's decision
/// (D108), and widening this until it passes would turn a measurement back into an assumption.
const TOLERANCE: f64 = 0.10;

/// How far the two prose sizes may disagree with each other, as a fraction.
///
/// Tighter than [`TOLERANCE`] on purpose: this is not a claim about the tokenizer but about the
/// *arithmetic*. Two sizes of the same text differenced against the same baseline must give the
/// same ratio, and 2026-09-11's 2.507 / 2.494 differ by 0.5%. A breach here means the baseline
/// subtraction is wrong, which makes both ratios meaningless — so it is reported as its own failure
/// rather than as a tolerance miss.
const PROSE_LINEARITY: f64 = 0.02;

/// The three `result.usage` fields that make up a turn's input, named once (F-16).
///
/// `input_tokens` **alone reports 2**. Kept as a constant so [`total_input`] and the assertion that
/// guards it cannot drift apart, and so the mistake has a name in the failure message.
const INPUT_FIELDS: [&str; 3] = [
    "input_tokens",
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
];

/// The wrong answer, kept so a run prints it next to the right one.
///
/// F-16's method note in one number: this is what a reader of `input_tokens` alone would have
/// differenced, and a differential over it is noise around zero.
const ONLY_INPUT_TOKENS_IS_THE_TRAP: &str = INPUT_FIELDS[0];

/// How long a turn is given to produce its `result`.
///
/// Generous rather than tuned, for `tests/cli_live.rs`'s reason — a slow box must report as slow,
/// not as broken — and longer than that file's 180 s because three of these four turns carry a
/// 40 000-character prompt.
const TURN_WINDOW: Duration = Duration::from_secs(300);

/// How long a finished turn is given to reach EOF and exit once stdin is closed.
const EOF_WINDOW: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------------------------
// Finding the binary, the way production finds it
// ---------------------------------------------------------------------------------------------

/// The `claude` tool probe the seed rows declare, whichever row declares it.
///
/// By probe rather than by row name, as `tests/cli_live.rs` does it: any row that declares a
/// `claude` tool declares the same tier-1 `path` probe, and this file needs the binary, not the row.
/// A tree with **no** such row is a repo bug rather than a box without the CLI, so it panics —
/// the absent-CLI case is [`resolve_cli`]'s, and it skips.
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

/// `docs/ANA-4.md` §4.4's invocation: `-p` with **no positional prompt**, the prompt arriving as the
/// first stdin NDJSON message.
///
/// Identical for all four turns — the differential is only meaningful if every fixed cost is fixed.
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

/// The `${claude}` launch this file spawns.
fn claude_launch() -> AgentLaunch {
    let mut tools = BTreeMap::new();
    tools.insert("claude".to_owned(), claude_probe());
    AgentLaunch {
        command: "${claude}".to_owned(),
        args: base_args(),
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

/// Resolves `${claude}` to a binary, or `None` when this box has no `claude`.
///
/// **`None` is a skip, never a failure** — `tests/install_live.rs` and `tests/auth_live.rs` set that
/// rule and this file follows it: a box without the CLI is not a broken build, and the one thing
/// worse than not measuring the constants is failing a suite for a reason the suite cannot fix.
/// Resolution happens **once, before the first spawn**, so an absent CLI costs nothing; the four
/// turns then reuse the resolved launch in four different working directories.
async fn resolve_cli(cwd: &Path) -> Option<ResolvedLaunch> {
    let launch = claude_launch();
    let tools = htui_agent::tools::resolve(launch.discovery.as_ref(), cwd)
        .await
        .ok()?;
    htui_agent::launch::resolve(&launch, &tools).ok()
}

// ---------------------------------------------------------------------------------------------
// One turn
// ---------------------------------------------------------------------------------------------

/// What one turn reported.
#[derive(Debug, Clone)]
struct Turn {
    /// The row label, for the printed table.
    label: &'static str,
    /// Characters this prompt added over [`BASELINE_PROMPT`] — the corpus and its two separators.
    added_chars: usize,
    /// `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` (F-16).
    total_input: i64,
    /// `input_tokens` alone, printed beside the total so the trap is visible in the output.
    bare_input: i64,
    /// The terminal `result` envelope, kept for the assertions on `num_turns` and `is_error`.
    result: Value,
}

/// `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` — **all three** (F-16).
///
/// A missing field is `0` rather than a panic: the vendor is free to stop emitting one, and a turn
/// that reported two of the three is a measurement this file should fail on *arithmetically*, with
/// the numbers printed, rather than at the unwrap.
fn total_input(result: &Value) -> i64 {
    INPUT_FIELDS
        .iter()
        .map(|field| result["usage"][*field].as_i64().unwrap_or(0))
        .sum()
}

/// The model ids the turn billed against, from `result.modelUsage` — printed, never asserted.
fn models_of(result: &Value) -> Vec<String> {
    result["modelUsage"]
        .as_object()
        .map(|models| models.keys().cloned().collect())
        .unwrap_or_default()
}

/// One NDJSON user message, the shape `--input-format stream-json` reads.
fn user_message(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    })
}

/// Reads stdout until the turn's terminal `result`, on a deadline.
async fn read_result(lines: &mut Lines<BufReader<ChildStdout>>, within: Duration) -> Option<Value> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match tokio::time::timeout(left, lines.next_line()).await {
            Err(_) => return None,
            Ok(Err(error)) => {
                println!("  !! reading stdout: {error}");
                return None;
            }
            Ok(Ok(None)) => return None,
            Ok(Ok(Some(text))) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text)
                    && value["type"] == json!("result")
                {
                    return Some(value);
                }
            }
        }
    }
}

/// Fails unless `pid` is gone or reaped-pending within two seconds.
///
/// Copied from `tests/cli_live.rs` rather than shared, as this repo copies its process helpers per
/// file. By pid, never by a `pgrep` pattern — a pattern matches the shell that launched
/// `cargo test` and any editor with the word in its command line.
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

/// Waits for the child, killing its group if the deadline passes; then asserts nothing survived.
async fn finish(mut spawned: Spawned, pid: u32, label: &str) {
    if tokio::time::timeout(EOF_WINDOW, spawned.wait())
        .await
        .is_err()
    {
        println!("  .. the CLI outlived {EOF_WINDOW:?} after its `result`; killing the group");
        let _ = spawned.kill_tree().await;
        let _ = spawned.wait().await;
    }
    for line in spawned.stderr_tail() {
        println!("  stderr| {line}");
    }
    assert_not_running(pid, label).await;
}

/// Spawns one `claude`, sends one prompt, reads its `result`, and reaps the child.
///
/// A fresh [`tempfile::TempDir`] per turn so the CLI reads no repository of the maintainer's and no
/// turn inherits another's session state — the differential assumes each turn pays the system
/// prompt independently.
async fn one_turn(resolved: &ResolvedLaunch, label: &'static str, prompt: &str) -> Turn {
    let cwd = tempfile::tempdir().expect("a scratch working directory");
    let added_chars = prompt.chars().count() - BASELINE_PROMPT.chars().count();
    println!("\n=== {label} ({added_chars} characters over the baseline prompt) ===");

    let mut spawned = htui_agent::launch::spawn(resolved, cwd.path())
        .await
        .expect("the CLI starts");
    let pid = spawned.pid().expect("a live child reports a pid");
    let mut stdin = spawned.take_stdin().expect("stdin is piped").into_inner();
    let mut lines =
        BufReader::new(spawned.take_stdout().expect("stdout is piped").into_inner()).lines();

    let mut payload =
        serde_json::to_string(&user_message(prompt)).expect("a user message serialises");
    payload.push('\n');
    let started = Instant::now();
    stdin
        .write_all(payload.as_bytes())
        .await
        .expect("the CLI accepts a user message on stdin");
    stdin.flush().await.expect("stdin flushes");

    let result = read_result(&mut lines, TURN_WINDOW).await;
    // Dropping the handle closes stdin, which is the documented end-of-input.
    drop(stdin);
    finish(spawned, pid, label).await;
    drop(cwd);

    let result = result.unwrap_or_else(|| {
        panic!(
            "{label}: no `result` arrived within {TURN_WINDOW:?}. Nothing downstream can be \
             measured without it — this is a precondition, not a finding."
        )
    });
    let turn = Turn {
        label,
        added_chars,
        total_input: total_input(&result),
        bare_input: result["usage"][ONLY_INPUT_TOKENS_IS_THE_TRAP]
            .as_i64()
            .unwrap_or(0),
        result,
    };
    println!(
        "  {:.1}s  total input {} tokens  (input_tokens alone: {} — F-16's trap)  \
         num_turns={:?} subtype={:?} models={:?}",
        started.elapsed().as_secs_f64(),
        turn.total_input,
        turn.bare_input,
        turn.result["num_turns"].as_i64(),
        turn.result["subtype"].as_str(),
        models_of(&turn.result),
    );
    turn
}

/// Fails unless the turn completed, unerrored, in exactly one model round trip.
///
/// `num_turns != 1` is the one failure mode that would otherwise produce a *plausible* wrong
/// number: a tool call means a second request, whose whole context is billed again under
/// `cache_read_input_tokens`, so the delta roughly doubles and the ratio roughly halves. Named
/// explicitly because the right response is a rerun, not a constants change.
fn assert_measurable(turn: &Turn) {
    assert!(
        turn.result["is_error"] != json!(true),
        "{}: the turn errored, so its token figures measure nothing: {}",
        turn.label,
        turn.result
    );
    assert_eq!(
        turn.result["num_turns"].as_i64(),
        Some(1),
        "{}: the turn took {:?} model round trips rather than one — almost certainly a tool call, \
         whose second request bills the whole context again under cache_read_input_tokens and \
         roughly halves the measured ratio. Rerun; do not read a constant off this.",
        turn.label,
        turn.result["num_turns"].as_i64(),
    );
    assert!(
        turn.total_input > 0,
        "{}: the `result` carried none of {INPUT_FIELDS:?}, so there is no input figure to \
         difference: {}",
        turn.label,
        turn.result
    );
}

/// `added_chars / (total_input - baseline)`: characters per token, the figure `chars-v2` is.
fn ratio(turn: &Turn, baseline: &Turn) -> f64 {
    let delta = turn.total_input - baseline.total_input;
    assert!(
        delta > 0,
        "{}: adding {} characters did not increase the input token count ({} vs the baseline's \
         {}). Either the baseline turn is not the baseline, or all three of {INPUT_FIELDS:?} are \
         being read from the wrong envelope.",
        turn.label,
        turn.added_chars,
        turn.total_input,
        baseline.total_input,
    );
    #[expect(
        clippy::cast_precision_loss,
        reason = "token counts are tens of thousands; f64 is exact well past 2^53"
    )]
    let ratio = turn.added_chars as f64 / delta as f64;
    ratio
}

/// `|measured - want| / want`.
fn drift(measured: f64, want: f64) -> f64 {
    (measured - want).abs() / want
}

// ---------------------------------------------------------------------------------------------
// The measurement
// ---------------------------------------------------------------------------------------------

/// Everything about the corpora the ratios below depend on, checked before a token is spent.
///
/// Both are the length they claim to be, and the prose one is its own first half twice over (F-61).
/// A fixture regenerated at a different length, or from a contiguous slice, would move every ratio
/// in this file and would do it **silently** — the run would still print three plausible numbers.
/// Not a `#[test]` of its own: this file spends money, so it stays one `#[ignore]`d case and a
/// plain `cargo test` over it runs nothing at all.
fn assert_corpora_are_intact() {
    assert_eq!(
        PROSE.chars().count(),
        CORPUS_CHARS,
        "the prose corpus is not {CORPUS_CHARS} characters; every ratio below is derived from its \
         length"
    );
    assert_eq!(
        CODE.chars().count(),
        CORPUS_CHARS,
        "the code corpus is not {CORPUS_CHARS} characters"
    );
    let first: String = PROSE.chars().take(HALF_CHARS).collect();
    let second: String = PROSE.chars().skip(HALF_CHARS).collect();
    assert_eq!(
        first, second,
        "the prose corpus is no longer its own first half repeated, so the two prose runs are no \
         longer two sizes of the *same* text and the linearity check below would fire on a \
         difference in the text rather than on a broken baseline subtraction — which is exactly \
         the ambiguity F-61 removed. Regenerate it as 20 000 characters written twice."
    );
}

/// D99's differential, repeatable: the four turns, the three ratios, and the constants they defend.
#[tokio::test]
#[ignore = "spends model tokens against the credential this box holds"]
async fn chars_v2_constants_hold_against_the_live_cli() {
    assert_corpora_are_intact();

    let scratch = tempfile::tempdir().expect("a scratch working directory");
    let Some(resolved) = resolve_cli(scratch.path()).await else {
        println!(
            "SKIPPED: `claude` does not resolve on this box, so the estimator cannot be measured \
             here. It is a `path` probe on the seed row; put the binary on PATH or set \
             HTUI_TOOL_CLAUDE. A box without the CLI is not a failing build."
        );
        return;
    };
    println!("$ {} {:?}", resolved.command, resolved.args);

    let half: String = PROSE.chars().take(HALF_CHARS).collect();
    let prompt = |corpus: &str| format!("{BASELINE_PROMPT}\n\n{corpus}");

    // Sequential, and in this order, because every corpus turn is differenced against the first.
    let baseline = one_turn(&resolved, "baseline", BASELINE_PROMPT).await;
    let prose_full = one_turn(&resolved, "prose 40 000", &prompt(PROSE)).await;
    let prose_half = one_turn(&resolved, "prose 20 000", &prompt(&half)).await;
    let code_full = one_turn(&resolved, "code 40 000", &prompt(CODE)).await;

    let turns = [&baseline, &prose_full, &prose_half, &code_full];
    for turn in turns {
        assert_measurable(turn);
    }

    let prose_ratio = ratio(&prose_full, &baseline);
    let half_ratio = ratio(&prose_half, &baseline);
    let code_ratio = ratio(&code_full, &baseline);

    println!("\n--- T69 / D99: the estimator differential ---");
    println!("| run | chars | total input tokens | delta vs baseline | chars/token |");
    println!("|---|---|---|---|---|");
    println!(
        "| baseline (`{BASELINE_PROMPT}`) | — | {} | — | — |",
        baseline.total_input
    );
    for (turn, measured) in [
        (&prose_full, prose_ratio),
        (&prose_half, half_ratio),
        (&code_full, code_ratio),
    ] {
        println!(
            "| {} | {} | {} | {} | **{measured:.3}** |",
            turn.label,
            turn.added_chars,
            turn.total_input,
            turn.total_input - baseline.total_input,
        );
    }
    println!(
        "\nF-16, 2026-09-11, `claude` 2.1.267: baseline 17 150; prose 2.507 and 2.494; code 2.440.\n\
         The `input_tokens`-alone figures for the four turns were {:?} — that is the whole of \
         F-16's method note: differencing those would have measured nothing.",
        turns.iter().map(|turn| turn.bare_input).collect::<Vec<_>>(),
    );
    println!(
        "models billed: {:?}",
        turns
            .iter()
            .map(|turn| models_of(&turn.result))
            .collect::<Vec<_>>()
    );

    // The arithmetic first: if the two prose sizes disagree, neither ratio means anything and a
    // tolerance miss below would be the wrong diagnosis.
    let spread = (prose_ratio - half_ratio).abs() / ((prose_ratio + half_ratio) / 2.0);
    println!(
        "\nlinearity: {prose_ratio:.3} at {CORPUS_CHARS} chars vs {half_ratio:.3} at {HALF_CHARS}, \
         {:.2}% apart (limit {:.0}%)",
        spread * 100.0,
        PROSE_LINEARITY * 100.0,
    );
    assert!(
        spread <= PROSE_LINEARITY,
        "the two prose sizes disagree by {:.2}%, over the {:.0}% limit: {prose_ratio:.3} at \
         {CORPUS_CHARS} characters against {half_ratio:.3} at {HALF_CHARS}. The two runs are the \
         same 20 000 characters, once and twice over (F-61), so this is not the text and it is not \
         the tokenizer — it is the arithmetic. Either the baseline turn ({} tokens) is not what \
         the corpus turns paid on top of, or one of the four turns was billed for something the \
         others were not. Fix the differential before reading anything off it, and do not touch \
         the constants on the strength of a ratio this check has rejected.",
        spread * 100.0,
        PROSE_LINEARITY * 100.0,
        baseline.total_input,
    );

    // Then the constants. `estimate.rs`'s own numbers, ×10, so this file cannot drift from the
    // thing it defends: a constants edit that did not re-measure fails here.
    let want_prose = f64::from(TokenEstimator::DEFAULT.prose_cpt) / 10.0;
    let want_code = f64::from(TokenEstimator::DEFAULT.code_cpt) / 10.0;
    println!(
        "against {}: prose {want_prose:.1} measured {prose_ratio:.3} ({:+.1}%), \
         code {want_code:.1} measured {code_ratio:.3} ({:+.1}%), tolerance ±{:.0}%",
        TokenEstimator::DEFAULT.id,
        (prose_ratio / want_prose - 1.0) * 100.0,
        (code_ratio / want_code - 1.0) * 100.0,
        TOLERANCE * 100.0,
    );

    for (what, measured, want) in [
        ("prose", prose_ratio, want_prose),
        ("code", code_ratio, want_code),
    ] {
        assert!(
            drift(measured, want) <= TOLERANCE,
            "{what}: {}'s {want:.1} characters per token measured {measured:.3} here, {:.1}% away \
             — outside the ±{:.0}% this file allows. Do **not** widen the tolerance and do not \
             edit `estimate.rs` to match: a constants change is a new estimator id (D108), because \
             `trim_record.estimator` means \"every token figure in this record is by this \
             arithmetic and no other\". Report the numbers above, with F-16's beside them.",
            TokenEstimator::DEFAULT.id,
            drift(measured, want) * 100.0,
            TOLERANCE * 100.0,
        );
    }

    // F-18, re-checked for free: the Claude row is the default for an unknown model because it is
    // the conservative one, and a lower characters-per-token yields a *higher* estimate.
    assert!(
        prose_ratio < f64::from(TokenEstimator::WIDE.prose_cpt) / 10.0
            && code_ratio < f64::from(TokenEstimator::WIDE.code_cpt) / 10.0,
        "F-18: the measured Claude rates ({prose_ratio:.3} / {code_ratio:.3}) are no longer below \
         the unverified GPT/Gemini row ({}), so `TokenEstimator::DEFAULT` is no longer the \
         conservative default for an unknown model — which is the argument §4.4 makes for it.",
        TokenEstimator::WIDE.id,
    );
}
