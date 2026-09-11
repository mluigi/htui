//! The live proof of a login (plan MOD-21 T8, D23): the **production** path, over the
//! **unmodified** `agy` seed row, with a human at a browser.
//!
//! `#[ignore]` by default, exactly as `tests/agy_live.rs`, `tests/probe_live.rs` and
//! `tests/install_live.rs` are — and more so than any of them, because the other three need only a
//! box with something installed on it and this one needs somebody to press a button in a browser.
//! A box where nobody does is not a failing build. Run it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test auth_live -- --ignored --nocapture
//! ```
//!
//! `--nocapture` is load-bearing rather than a nicety: the link the maintainer has to open is
//! printed by this test and by nothing else, so a captured run is a run that waits ten minutes for
//! a human it never told what to do.
//!
//! **It burns no model tokens.** The only calls it makes are `initialize`, then one `authenticate`
//! (or one `logout`), then the probe's tier 2 — which completes `initialize` and stops. No session
//! is opened and no prompt is ever sent, exactly as in `tests/agy_live.rs`.
//!
//! # What it is for
//!
//! The seed declares a credential file at `%GEMINI_HOME%/antigravity-acp/acp_token.json` (falling
//! back to `~/.gemini/antigravity-acp/acp_token.json`), and until this test is run by somebody,
//! **nobody has ever completed this flow** — so whether that filename is the one the vendor writes
//! is unknown, and the PRD's risk table says so. The probe reads `ready` from that file's existence
//! and from nothing else, so a login that succeeds against a differently-named file leaves the row
//! reading `unauthenticated` for ever. That is why
//! [`agy_logs_in_through_the_app_and_the_probe_reads_ready`] prints a **listing of the directory**
//! the seed points at after the login: the listing is the only way to learn the answer, and the
//! assertion that follows it either confirms the seed or names the mismatch.
//!
//! # The environment it reads (and never sets)
//!
//! Every one of these is read with `std::env::var` and written by nobody: `std::env::set_var` is
//! `unsafe`, this workspace forbids `unsafe`, and a process-wide mutation would leak into every
//! other case in the binary besides (the reason `tests/install_live.rs` gives).
//!
//! - `HTUI_LIVE_METHOD` — which advertised method id to send, defaulting to `oauth-personal`.
//!   It exists because the four this adapter advertises are not interchangeable: measured on this
//!   box, `gemini-api-key` fails fast with JSON-RPC `-32602` and "The GEMINI_API_KEY environment
//!   variable must be set in the environment the ACP server is launched from for Gemini API Key
//!   authentication" — a perfectly good answer, and not the one this proof is about. A maintainer
//!   with a business account or an API key can point this test at their own method without editing
//!   it.
//! - `HTUI_LIVE_OPEN` — `1` also hands the link to [`open_url`], which is what the pane's `o` key
//!   reaches (MOD-21 D17). **The default is print-only**: a test that opened a browser tab merely
//!   because it was listed would be a surprise, and the link is printed prominently either way.
//! - `HTUI_LIVE_LOGOUT` — `1` runs the second case, [`agy_logs_out_when_asked`]. It is off by
//!   default because a suite that logged the maintainer's box out on the way past would cost them
//!   a browser round trip every time they ran the first case.
//!
//! # What it asserts (plan D23)
//!
//! 1. The flow reached [`AuthOutcome::Completed`] — the agent answered the call. A refusal, an idle
//!    cap, a cancel and a decline each fail **by name** with the agent's own words where there are
//!    any, because each of them means something different to the person reading the output.
//! 2. No child process survived it. Same `/proc` read by pid as `tests/agy_live.rs` — never a
//!    `pgrep` pattern, which matches the shell that started `cargo test`.
//! 3. The re-probed status is `ready`, **or** the failure names the seed: it prints what
//!    `discovery.credential.files` declares, what the directory actually holds, and that correcting
//!    the seed is the finding this test exists to produce.
//!
//! Nothing the vendor may change at will is asserted: not the method ids (the chosen one is an
//! input, and the rest are printed), not their names or descriptions, not a stderr line, not the
//! shape of the link. That is `tests/agy_live.rs`'s rule and it applies here in full.
//!
//! # What it must never print (`R-ID-7`, `R-SEC-2`)
//!
//! **No credential value, ever.** The directory listing is a `read_dir` and a `metadata` — names,
//! sizes and modes — and this file opens no file under a credential directory, for any reason:
//! there is no `read_to_string` of one anywhere below. The environment tier is reported as *set* or
//! *unset* and its value is never read into a string. What *is* printed is what the pane already
//! shows a user: the method list, the adapter's own stderr lines, and the authorisation link the
//! adapter itself wrote to stderr for the human to open.
//!
//! # Preconditions, and what a miss does
//!
//! Both are named and both **stop rather than fail**, printing `SKIPPED:` — this is an interactive
//! proof, and a box that is not in the state it examines has not found a defect:
//!
//! - The seed row resolves through the production probe on this box. A `missing` status means the
//!   adapter is not installed, and the message says how to install it from inside `htui`.
//! - The status is `unauthenticated`. A box that is already `ready` stops by name, exactly as
//!   `tests/agy_live.rs`'s case 4 does: there is no login to watch on a box that has one, and the
//!   maintainer who wants to see this run again can log out with the second case first.
//!
//! `R-AGT-5`: the agent name, the method ids and the vendor's host live in **this file** and in the
//! seed document, which is data. The sweep in `tests/extensibility.rs` covers `crates/*/src`, and
//! nothing here is asking for a line there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use chrono::Utc;
use htui_agent::auth::{
    AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome,
    BrowserPolicy, OpenerCommand, open_url,
};
use htui_agent::driver::AgentDriver;
use htui_agent::error::DriverError;
use htui_agent::launch::{AgentLaunch, CredentialProbe};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeStatus, SpawnTier2, platform_key,
    probe_agent,
};
use htui_agent::registry::DriverFactory;
use htui_core::model::{Agent as AgentRow, AgentBox, BoxId};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// One live adapter at a time, across both cases.
///
/// The same reason `tests/agy_live.rs` gives for its own: the survivor check reads
/// `/proc/self/task/*/children`, which is **process-wide**, and `cargo test` runs the cases as
/// threads of one process, so two overlapping flows would each call the other's healthy child a
/// survivor. A `tokio::sync::Mutex` because each `#[tokio::test]` builds its own runtime and the
/// guard is held across `.await`s.
static ONE_AT_A_TIME: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Which advertised method id to send. Read, never written.
const METHOD_VAR: &str = "HTUI_LIVE_METHOD";

/// The method this proof is about when [`METHOD_VAR`] says nothing: the browser round trip.
const DEFAULT_METHOD: &str = "oauth-personal";

/// `1` also hands the link to [`open_url`]; anything else, including unset, is print-only.
const OPEN_VAR: &str = "HTUI_LIVE_OPEN";

/// `1` runs [`agy_logs_out_when_asked`]; anything else, including unset, skips it.
const LOGOUT_VAR: &str = "HTUI_LIVE_LOGOUT";

/// The seed document whose `credential.files` this test either confirms or names as wrong.
const SEED_PATH: &str = "crates/htui-core/seeds/agent_agy.json";

/// How long a login that did not turn the box `ready` is given before its failure is composed.
///
/// Not a retry and not part of the verdict — the reading production would have written is taken
/// first and is the one asserted on. This buys the *diagnosis*: a token file that appears in the
/// second listing and not the first says the seed's filename is right and the re-probe is early,
/// which is a completely different correction from the one this test usually reports, and telling
/// them apart afterwards would cost another browser round trip.
const SETTLE: Duration = Duration::from_secs(5);

/// What the maintainer is told when the adapter is not on this box.
///
/// Since `R-AGT-10` the answer is `htui` itself, so this names the action rather than a version and
/// a URL that go stale without anything going red — `tests/agy_live.rs`'s `INSTALL_HINT` verbatim
/// in intent, kept local because this repo duplicates its live-suite helpers per file.
const INSTALL_HINT: &str = "\
install the adapter from inside htui (`R-AGT-10`):
  1. open Settings > Agents and put the cursor on the `agy` row with `j`/`k`
  2. press `i`, read the consent pane and answer `y`
  3. the section re-probes when it finishes, so the row itself says whether this suite can run
or point HTUI_TOOL_AGY_ACP_SERVER at the server this box already has";

// ---------------------------------------------------------------------------------------------
// The box, read the way production reads it
// ---------------------------------------------------------------------------------------------

/// The seeded `agy` row, exactly as `PgStore::seed_if_empty_as` inserts it. Unmodified: D23's whole
/// point is that the production path works over the document the workspace ships.
fn agy_row() -> AgentRow {
    htui_core::model::agent::seed_rows(Utc::now())
        .into_iter()
        .find(|agent| agent.name == "agy")
        .expect("the seed rows carry `agy`")
}

/// Where a login runs: the box's directory, never a session's (`AuthFlow::cwd`).
///
/// This crate's manifest directory, as every live suite in this file's neighbourhood uses — the
/// worker's own choice is `std::env::current_dir`, which under `cargo test` is the same tree.
fn cwd() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `true` when `name` is exactly `1` in this process's environment.
///
/// Exactly `1` rather than "set to anything": `HTUI_LIVE_OPEN=0` must not open a browser, and a
/// truthiness grammar with more than one member is a grammar somebody has to remember.
fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

/// The method id to send: [`METHOD_VAR`], or [`DEFAULT_METHOD`].
///
/// An empty value reads as unset, because that is how a shell profile clears a variable it once
/// exported.
fn chosen_method() -> String {
    std::env::var(METHOD_VAR)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_METHOD.to_owned())
}

/// The row a probe decided to write, or the reason there is none.
fn row_of(outcome: ProbeOutcome) -> Option<AgentBox> {
    match outcome {
        ProbeOutcome::Row(row) => Some(row),
        ProbeOutcome::Kept { reason } => {
            println!("SKIPPED: the probe wrote nothing and kept the row as it was: {reason}");
            None
        }
    }
}

/// This box's `agent_box` row for `agy`, probed through the production entry point.
///
/// Tier 1 **with** its version children, as `Settings > r` runs it: the seed declares `agy` as a
/// required `path` tool, and a box without the CLI probes `missing` — which this test would rather
/// report as itself than as a mystery three assertions later (blueprint H-11).
async fn probe_the_box() -> Option<AgentBox> {
    let ctx = ProbeContext {
        env: ProbeEnv::host(cwd()),
        now: Utc::now(),
    };
    row_of(probe_agent(&agy_row(), BoxId::new(), None, &ctx, &SpawnTier2::default()).await)
}

/// The re-probe a login is judged by, in production's own spelling.
///
/// `ProbeEnv::without_versions` because a login changes no version string — that is the worker's
/// reasoning at `run_auth`'s `Completed` arm, and this test asserts on what that arm would have
/// written, so it has to ask the same question.
async fn reprobe(existing: Option<&AgentBox>) -> Option<AgentBox> {
    let ctx = ProbeContext {
        env: ProbeEnv::host(cwd()).without_versions(),
        now: Utc::now(),
    };
    row_of(
        probe_agent(
            &agy_row(),
            BoxId::new(),
            existing,
            &ctx,
            &SpawnTier2::default(),
        )
        .await,
    )
}

/// The status a probed row records, or [`ProbeStatus::Failed`] when the column will not parse —
/// which is how the worker itself reads it back.
fn status_of(row: &AgentBox) -> ProbeStatus {
    ProbeSnapshot::from_row(row).map_or(ProbeStatus::Failed, |snapshot| snapshot.status)
}

/// What the snapshot says about the credential tier, as text, for the report.
fn credential_tier_of(row: &AgentBox) -> String {
    ProbeSnapshot::from_row(row)
        .and_then(|snapshot| snapshot.credential)
        .map_or_else(|| "none recorded".to_owned(), |tier| tier.to_string())
}

/// The probed row, printed, and stopped on by name unless it is in the state `want` describes.
///
/// Both misses print `SKIPPED:` and return `None` rather than panicking: this is an interactive
/// proof, and a box that is not in the state a case examines has found no defect in `htui`.
async fn box_in_state(want: ProbeStatus) -> Option<AgentBox> {
    let row = probe_the_box().await?;
    let status = status_of(&row);
    println!("platform = {}", platform_key());
    if let Some(launch) = ProbeSnapshot::from_row(&row).and_then(|snapshot| snapshot.resolved) {
        // What the flow will actually spawn (D58's recording, `--uid=` and all).
        println!(
            "agent_box.probe.resolved = {} {:?}",
            launch.command, launch.args
        );
    }
    println!(
        "agent_box.probe.status = {status}, credential tier = {}",
        credential_tier_of(&row)
    );

    if status == ProbeStatus::Missing || status == ProbeStatus::Failed {
        println!(
            "SKIPPED: the seed row probes `{status}` on this box, so there is no adapter to log \
             in to.\n{INSTALL_HINT}"
        );
        return None;
    }
    if status != want {
        println!(
            "SKIPPED: this case needs a box that reads `{want}` and this one reads `{status}`. \
             (`ready` means the vendor's credential is already there: run the logout case with \
             {LOGOUT_VAR}=1 first if you want to watch a login again.)"
        );
        return None;
    }
    Some(row)
}

/// Production's own refusal, mirrored: a row whose recorded handshake advertised no method has
/// nothing to offer a chooser, and the worker turns its `AuthStart` down before spawning anything.
///
/// Implied by `unauthenticated` today — `probe::status_for` reaches that status only through a
/// non-empty `authMethods` — and spelled here anyway, because the gate a maintainer reading this
/// output is looking at is the worker's, not this file's, and a status mapping that changed would
/// otherwise turn a refused `AuthStart` into a mystery.
fn advertises_auth(row: &AgentBox) -> bool {
    ProbeSnapshot::from_row(row)
        .and_then(|snapshot| snapshot.handshake)
        .is_some_and(|handshake| !handshake.auth_methods.is_empty())
}

// ---------------------------------------------------------------------------------------------
// The credential candidates the seed declares — names and sizes only (`R-ID-7`)
// ---------------------------------------------------------------------------------------------

/// One candidate from `discovery.credential.files`, expanded for this box.
#[derive(Debug, Clone)]
struct Site {
    /// The pattern exactly as the seed spells it, so a failure can quote the document.
    pattern: String,
    /// The file the pattern names on this box.
    file: PathBuf,
    /// The directory that file would sit in: what the listing walks, and where a differently-named
    /// token would be hiding.
    dir: PathBuf,
}

/// One directory entry: what it is called and how big it is. **Never what it holds.**
#[derive(Debug, Clone)]
struct Entry {
    /// The file name, which is the finding this whole test exists to produce.
    name: String,
    /// Its size in bytes — enough to tell a written token from a zero-length placeholder, and not
    /// one byte of its content.
    bytes: u64,
    /// The permission bits, because a credential file's mode is worth seeing and reveals nothing.
    mode: String,
}

/// The seed's `credential` block, read as data.
fn credential_probe(launch: &AgentLaunch) -> CredentialProbe {
    launch
        .discovery
        .as_ref()
        .and_then(|discovery| discovery.credential.clone())
        .expect("the seeded agy row declares a credential block (plan D59)")
}

/// Every declared file candidate, expanded for this box.
///
/// A candidate whose `%VAR%` is unset is skipped, which is the probe's own rule — and on this box
/// it is why the `~/.gemini/...` fallback exists at all.
fn sites(probe: &CredentialProbe, env: &ProbeEnv) -> Vec<Site> {
    probe
        .files
        .iter()
        .filter_map(|pattern| {
            let file = expand_by_hand(pattern, env)?;
            Some(Site {
                pattern: pattern.clone(),
                dir: file
                    .parent()
                    .map_or_else(|| file.clone(), Path::to_path_buf),
                file,
            })
        })
        .collect()
}

/// `%VAR%` pairs substituted and a leading `~` expanded; `None` when a variable is unset or the
/// home directory is unknown, which is "skip this candidate".
///
/// A second, dumber implementation of the two grammar rules rather than a call to the probe's own
/// expander, for `tests/agy_live.rs`'s reason: asserting the probe against itself would prove
/// nothing about the box.
fn expand_by_hand(pattern: &str, env: &ProbeEnv) -> Option<PathBuf> {
    let mut text = pattern.to_owned();
    while let Some(open) = text.find('%') {
        let rest = &text[open + 1..];
        let close = rest.find('%')?;
        let value = env.var(&rest[..close])?.to_owned();
        text = format!("{}{value}{}", &text[..open], &rest[close + 1..]);
    }
    match text.strip_prefix("~/") {
        Some(tail) => Some(env.home.clone()?.join(tail)),
        None => Some(PathBuf::from(text)),
    }
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

/// What `dir` contains, by name and size; `None` when there is no such directory.
///
/// `read_dir` and `metadata`, and nothing else: no entry is opened, so nothing here can print a
/// credential (`R-ID-7`).
fn listing(dir: &Path) -> Option<Vec<Entry>> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| {
            let meta = entry.metadata().ok();
            Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                bytes: meta.as_ref().map_or(0, std::fs::Metadata::len),
                mode: meta.as_ref().map_or_else(
                    || "unreadable".to_owned(),
                    |meta| {
                        if meta.is_dir() {
                            format!("{} <dir>", mode_of(meta))
                        } else {
                            mode_of(meta)
                        }
                    },
                ),
            }
        })
        .collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Some(entries)
}

/// The whole credential question, as text ready to print or to put in a failure message.
///
/// Every candidate the seed declares, whether its file is there, and — the point of the exercise —
/// what its directory actually holds. Nobody has ever completed this flow, so the listing is the
/// only evidence there is that `acp_token.json` is or is not the vendor's filename.
fn credential_report(sites: &[Site], probe: &CredentialProbe, env: &ProbeEnv) -> String {
    let mut report = String::new();
    for site in sites {
        let _ = writeln!(
            report,
            "  declared `{}`\n    -> {} ({})",
            site.pattern,
            site.file.display(),
            if site.file.is_file() {
                "EXISTS"
            } else {
                "not there"
            }
        );
        match listing(&site.dir) {
            Some(entries) if entries.is_empty() => {
                let _ = writeln!(report, "    {} is empty", site.dir.display());
            }
            Some(entries) => {
                let _ = writeln!(report, "    {} holds:", site.dir.display());
                for entry in entries {
                    let _ = writeln!(
                        report,
                        "      {:<40} {:>12} bytes  mode {}",
                        entry.name, entry.bytes, entry.mode
                    );
                }
            }
            None => {
                let _ = writeln!(report, "    {} does not exist", site.dir.display());
            }
        }
    }
    for name in &probe.env {
        // Whether the name is set, never what it is set to (`R-ID-7`).
        let _ = writeln!(
            report,
            "  declared env `{name}`: {}",
            if env.var(name).is_some_and(|value| !value.is_empty()) {
                "set and non-empty"
            } else {
                "unset or empty"
            }
        );
    }
    report
}

// ---------------------------------------------------------------------------------------------
// One live flow
// ---------------------------------------------------------------------------------------------

/// Everything one flow let this test see, besides its outcome.
#[derive(Debug, Default)]
struct Seen {
    /// The `agent`-kind methods the adapter advertised, in its own order.
    methods: Vec<AuthMethodInfo>,
    /// Whether it advertised a logout capability.
    logout: bool,
    /// Every link it printed, first sighting only (the flow deduplicates, D15).
    urls: Vec<String>,
    /// How many stderr lines it wrote.
    lines: usize,
    /// The choice was sent.
    chose: bool,
    /// Why the choice was **not** sent, when it was not: the id asked for is not one the adapter
    /// offers. Reported after the loop rather than panicked mid-flow, so the child dies through the
    /// flow's own exit and not through a drop from a panicking frame.
    unadvertised: Option<String>,
}

/// One `authenticate` or `logout`, driven exactly as the worker loop drives it.
///
/// The `select!` is the worker's own (`agent_worker.rs`'s `run_auth`): biased towards the flow, the
/// event stream forwarded until it retires, and the queue drained afterwards because the last line
/// an adapter prints is often the one that says why. What this test does with the events is all
/// that differs — it prints them, chooses once, and offers the link to a human.
async fn drive(
    driver: &dyn AgentDriver,
    want: &AuthChoice,
) -> (Result<AuthOutcome, DriverError>, Seen) {
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (choice_tx, choice_rx) = oneshot::channel();
    let started = Instant::now();

    let running = driver.authenticate(AuthFlow {
        cwd: cwd(),
        events: events_tx,
        choice: choice_rx,
        // Nothing in this test trips it: the cases that would — a pane's `x`, a shutdown — are
        // `tests/auth.rs`'s, and here the human and the idle cap are the only clocks.
        cancel: CancellationToken::new(),
        // Production's cap, not a test's: D13 measures it from the last sign of life, which is what
        // makes ten minutes the right order for a browser round trip.
        idle: AUTH_IDLE_CAP,
        // Production, and it has to stay production here (D16): without it the adapter's own opener
        // falls through to a terminal browser on a display-less box and writes escape sequences
        // into the JSON-RPC stream this flow is speaking.
        browser: BrowserPolicy::Neutralised,
    });
    tokio::pin!(running);

    let mut choice_tx = Some(choice_tx);
    let mut seen = Seen::default();
    let mut listening = true;
    let outcome = loop {
        tokio::select! {
            biased;
            outcome = &mut running => break outcome,
            event = events_rx.recv(), if listening => match event {
                Some(event) => show(event, want, &mut choice_tx, &mut seen, started).await,
                None => listening = false,
            },
        }
    };
    // The senders lived inside the future that has just returned, so this drains what it wrote on
    // its way out and then ends.
    while let Some(event) = events_rx.recv().await {
        show(event, want, &mut choice_tx, &mut seen, started).await;
    }
    println!(
        "the flow ended after {:.1}s, {} stderr line(s), {} link(s)",
        started.elapsed().as_secs_f64(),
        seen.lines,
        seen.urls.len()
    );
    (outcome, seen)
}

/// One event, printed — and, on the method list, answered.
///
/// The `Methods` arm is the chooser the pane draws, reduced to one decision taken from the
/// environment. The `Url` arm is the hand-off: printed prominently for a human always, opened only
/// when [`OPEN_VAR`] says so.
async fn show(
    event: AuthEvent,
    want: &AuthChoice,
    choice: &mut Option<oneshot::Sender<AuthChoice>>,
    seen: &mut Seen,
    started: Instant,
) {
    match event {
        AuthEvent::Methods {
            methods,
            logout,
            hidden,
        } => {
            println!("--- the adapter's own `initialize` answer ---");
            println!("advertised methods ({}):", methods.len());
            for method in &methods {
                println!("  id=`{}`  name=`{}`", method.id, method.name);
                println!(
                    "    description: {}",
                    method.description.as_deref().unwrap_or("<none>")
                );
            }
            println!("logout advertised: {logout}");
            if hidden.is_empty() {
                println!("terminal-typed methods (never sent, D4/D21): none");
            } else {
                for method in &hidden {
                    println!(
                        "terminal-typed (never sent, D4/D21): id=`{}` name=`{}`",
                        method.id, method.name
                    );
                }
            }
            seen.logout = logout;
            seen.methods = methods;

            let offered = match want {
                AuthChoice::Method(id) => seen.methods.iter().any(|method| &method.id == id),
                AuthChoice::Logout => logout,
            };
            match (offered, choice.take()) {
                (true, Some(sender)) => {
                    println!("choosing: {want:?}");
                    seen.chose = sender.send(want.clone()).is_ok();
                }
                (false, Some(sender)) => {
                    // Dropped rather than sent: the flow reads that as `Declined` and ends without
                    // asking the adapter for something it never offered.
                    drop(sender);
                    let offers = seen
                        .methods
                        .iter()
                        .map(|method| method.id.as_str())
                        .collect::<Vec<_>>();
                    seen.unadvertised = Some(match want {
                        AuthChoice::Method(id) => format!(
                            "this run asked for method `{id}`, which this adapter does not \
                             advertise: it offers {offers:?}. Set {METHOD_VAR} to one of those ids."
                        ),
                        AuthChoice::Logout => {
                            "this run asked for a logout and this adapter advertises no logout \
                             capability, so there is nothing to send it (plan D4)."
                                .to_owned()
                        }
                    });
                    println!("STOPPING: {}", seen.unadvertised.as_deref().unwrap_or(""));
                }
                (_, None) => println!("a second method list arrived; the choice was already sent"),
            }
        }
        AuthEvent::Line(line) => {
            seen.lines += 1;
            println!(
                "[{:>7.1}s] stderr | {line}",
                started.elapsed().as_secs_f64()
            );
        }
        AuthEvent::Url(url) => {
            seen.urls.push(url.clone());
            println!(
                "\n\
                 ==============================================================================\n\
                 OPEN THIS LINK IN A BROWSER TO FINISH THE LOGIN:\n\n\
                 {url}\n\n\
                 The loopback listener that link comes back to lives inside the adapter process,\n\
                 so it dies with this test: finish the flow before the idle cap of {}s of silence.\n\
                 ==============================================================================\n",
                AUTH_IDLE_CAP.as_secs()
            );
            if env_flag(OPEN_VAR) {
                match open_url(&url, &OpenerCommand::Platform).await {
                    Ok(()) => println!("{OPEN_VAR}=1: handed the link to this box's opener"),
                    Err(err) => println!("{OPEN_VAR}=1: the opener could not be started: {err}"),
                }
            } else {
                println!(
                    "{OPEN_VAR} is not `1`, so htui opened nothing. That is the default; the link \
                     above is yours to open."
                );
            }
        }
    }
}

/// The outcome, or a failure that says which of the five it was in the words that outcome carries.
///
/// D23 asserts `Completed` and this is where that assertion lives, spelled once for both cases:
/// every other outcome means something different to the person reading the output, and a bare
/// `assert!(matches!(..))` would tell them none of it.
fn completed(outcome: Result<AuthOutcome, DriverError>, seen: &Seen) -> AuthCall {
    match outcome {
        Ok(AuthOutcome::Completed { call }) => call,
        Ok(AuthOutcome::Refused { call, message }) => panic!(
            "the agent refused {call:?} in its own words (plan D5), so nothing was proven about a \
             completed login:\n{message}"
        ),
        Ok(AuthOutcome::Idle { after }) => panic!(
            "nobody finished the flow within {after:?} of silence (D13). The cap is measured from \
             the adapter's last stderr line, not from the spawn, so this is an abandoned login \
             rather than a slow one."
        ),
        Ok(AuthOutcome::Cancelled) => {
            panic!("the flow was cancelled; nothing in this test trips its token")
        }
        Ok(AuthOutcome::Declined) => panic!(
            "no choice was ever sent. {}",
            seen.unadvertised
                .as_deref()
                .unwrap_or("The method list never arrived.")
        ),
        Err(err) => panic!("the flow failed before the agent could answer: {err}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Survivors — by pid, never by pattern (`tests/agy_live.rs`'s helper, kept local)
// ---------------------------------------------------------------------------------------------

/// Every process this test process is the direct parent of, as pid strings.
///
/// `children` exists only with `CONFIG_PROC_CHILDREN=y`; a run where not one was readable falls
/// back to [`children_by_ppid`] so the survivor assertion can never pass vacuously. A child that
/// was killed but not reaped still appears here — which is the point, since the flow promises the
/// tree is killed *and* reaped before it returns (blueprint H-1).
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

/// Nothing this process started is still around, half a second after the kill.
#[cfg(target_os = "linux")]
async fn assert_no_survivors(what: &str) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let survivors = children_of_this_process();
    assert!(
        survivors.is_empty(),
        "{what} left children behind: {survivors:?}. The OAuth loopback listener lives inside that \
         process, so a survivor is a port held open on this box."
    );
}

// ---------------------------------------------------------------------------------------------
// Case 1 — milestone 4: `unauthenticated` -> `ready`, and what the vendor actually wrote
// ---------------------------------------------------------------------------------------------

/// The proof (plan D23): a real login, through the production path, over the unmodified seed row.
///
/// **This case is run by a human**, and it will sit and wait for one. It prints the adapter's
/// method list, chooses [`chosen_method`], prints every stderr line the adapter writes, prints the
/// authorisation link prominently for the maintainer to open, waits out the browser round trip,
/// re-probes, and then prints the listing that answers the one open question in the seed: **what
/// file does the vendor actually write?**
///
/// The last assertion is the finding. `ready` confirms `credential.files`; anything else fails by
/// name, quotes what the seed declares and what the directory holds, and hands the maintainer the
/// one-line edit that closes MOD-21's milestone 4.
#[tokio::test]
#[ignore = "interactive: spawns the agy adapter and waits for a human to finish a browser login"]
async fn agy_logs_in_through_the_app_and_the_probe_reads_ready() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let Some(on_box) = box_in_state(ProbeStatus::Unauthenticated).await else {
        return;
    };
    if !advertises_auth(&on_box) {
        println!(
            "SKIPPED: this row's recorded handshake advertises no auth method, which is what the \
             worker refuses an `AuthStart` on before it spawns anything."
        );
        return;
    }

    let env = ProbeEnv::host(cwd());
    let launch: AgentLaunch =
        serde_json::from_value(agy_row().launch.clone()).expect("the seeded agy launch parses");
    let credential = credential_probe(&launch);
    let candidates = sites(&credential, &env);
    println!("--- the seed's credential candidates, before the login ---");
    print!("{}", credential_report(&candidates, &credential, &env));

    let method = chosen_method();
    println!(
        "--- logging in with `{method}` (from {METHOD_VAR}, default `{DEFAULT_METHOD}`) ---\n\
         The idle cap is {}s of silence; every stderr line the adapter writes restarts it.",
        AUTH_IDLE_CAP.as_secs()
    );

    // The production chain, not a hand-built driver: `DriverFactory::production` is what the worker
    // holds, `driver_for` is what it calls, and for an `acp` row the `authenticate` below is
    // `AcpDriver`'s. The probed row goes in so the flow spawns D58's recording — `--uid=` and all.
    let driver = DriverFactory::production()
        .driver_for(&agy_row(), Some(&on_box))
        .expect("the seeded agy row builds its production driver");
    let (outcome, seen) = drive(driver.as_ref(), &AuthChoice::Method(method.clone())).await;

    let call = completed(outcome, &seen);
    assert_eq!(
        call,
        AuthCall::Authenticate(method.clone()),
        "the outcome names the call this test made"
    );
    assert!(
        seen.chose,
        "a completed authenticate had a choice sent to it"
    );
    println!("the flow reached `Completed` for {call:?}");

    // Before the re-probe, because a survivor would still be holding the loopback port the browser
    // just came back to.
    #[cfg(target_os = "linux")]
    assert_no_survivors("the login").await;

    let reprobed = reprobe(Some(&on_box))
        .await
        .expect("a re-probe of a row this test just probed writes a row");
    let status = status_of(&reprobed);
    println!("--- after the login ---");
    println!(
        "re-probed agent_box.probe.status = {status}, credential tier = {}, enabled = {}",
        credential_tier_of(&reprobed),
        reprobed.enabled
    );
    let after = credential_report(&candidates, &credential, &env);
    println!("--- what the seed's credential candidates hold now ---");
    print!("{after}");

    #[cfg(target_os = "linux")]
    assert_no_survivors("the re-probe").await;

    // D23's verdict, and the reason this file exists. Everything needed to correct the seed is in
    // the message: no byte of any of those files was read to compose it.
    if status != ProbeStatus::Ready {
        // One more look before the verdict is written down — and **not** a second chance for
        // production, which is judged on the reading above: the worker re-probes the instant the
        // flow returns and writes whatever it finds. This is here because the two diagnoses look
        // identical from a single reading and cost the maintainer a whole browser round trip to
        // tell apart afterwards: "the vendor writes a file this seed does not name" is a one-line
        // seed edit, and "the vendor writes it a moment after `authenticate` returns" is a defect
        // in when `htui` re-probes.
        println!("--- not `ready`; looking again after {SETTLE:?} before deciding why ---");
        tokio::time::sleep(SETTLE).await;
        let settled = reprobe(Some(&reprobed))
            .await
            .map_or_else(|| "no row".to_owned(), |row| status_of(&row).to_string());
        let settled_report = credential_report(&candidates, &credential, &env);
        #[cfg(target_os = "linux")]
        assert_no_survivors("the settling re-probe").await;
        panic!(
            "the login reached `Completed` and the re-probe reads `{status}` (and `{settled}` \
             again {SETTLE:?} later).\n\n\
             This is the mismatch MOD-21's plan calls H-18: the vendor wrote its credential \
             somewhere other than where `{SEED_PATH}` declares it. What the seed declares, and \
             what this box held at the moment production would have judged it:\n{after}\n\
             And {SETTLE:?} later:\n{settled_report}\n\
             If a file listed above is the token under another name, correct \
             `discovery.credential.files` in that seed to it and re-run — that edit is T8's other \
             half and nothing else in the tree has to change. If no directory above exists at all, \
             the vendor writes under a different root and both patterns need replacing. If the \
             file is absent in the first listing and present in the second, the filename is right \
             and the finding is about **when** the login re-probes, not about the seed."
        );
    }
    println!(
        "CONFIRMED: `{}` is what the vendor writes, and `{SEED_PATH}` already declares it. \
         Milestone 4's `unauthenticated` -> `ready` holds on this box.",
        candidates
            .iter()
            .find(|site| site.file.is_file())
            .map_or_else(
                || "an env-tier credential".to_owned(),
                |site| site.file.display().to_string()
            )
    );
}

// ---------------------------------------------------------------------------------------------
// Case 2 — putting the box back (run only when asked)
// ---------------------------------------------------------------------------------------------

/// `logout`, and the probe reading `unauthenticated` again.
///
/// Off unless `HTUI_LIVE_LOGOUT=1`, for two reasons. It is destructive — it spends the
/// maintainer's own browser round trip to undo — and it is the *other* half of leaving a box in a
/// state the suite can reproduce: with it, a maintainer can run the login proof twice; without it,
/// the first run is the only one, because every run afterwards stops by name on a `ready` box.
///
/// The same rule as case 1 about the vendor: what is asserted is that the call completed, that
/// nothing survived it, and that the **probe** — the sole authority on what a box is (`R-AGT-6`,
/// D6) — went back to `unauthenticated`. What the adapter said on the way is printed, never
/// asserted against.
#[tokio::test]
#[ignore = "interactive and destructive: logs this box out of the agent (HTUI_LIVE_LOGOUT=1)"]
async fn agy_logs_out_when_asked() {
    let _serial = ONE_AT_A_TIME.lock().await;
    if !env_flag(LOGOUT_VAR) {
        println!(
            "SKIPPED: this case logs this box out, and would cost a browser round trip to undo. \
             Run it with {LOGOUT_VAR}=1 when you want the box back where the login proof can be \
             run again."
        );
        return;
    }
    let Some(on_box) = box_in_state(ProbeStatus::Ready).await else {
        return;
    };

    let env = ProbeEnv::host(cwd());
    let launch: AgentLaunch =
        serde_json::from_value(agy_row().launch.clone()).expect("the seeded agy launch parses");
    let credential = credential_probe(&launch);
    let candidates = sites(&credential, &env);
    println!("--- the seed's credential candidates, before the logout ---");
    print!("{}", credential_report(&candidates, &credential, &env));

    let driver = DriverFactory::production()
        .driver_for(&agy_row(), Some(&on_box))
        .expect("the seeded agy row builds its production driver");
    let (outcome, seen) = drive(driver.as_ref(), &AuthChoice::Logout).await;

    let call = completed(outcome, &seen);
    assert_eq!(call, AuthCall::Logout, "the outcome names the call made");
    assert!(
        seen.logout,
        "a completed logout was advertised by the adapter's own capabilities"
    );

    #[cfg(target_os = "linux")]
    assert_no_survivors("the logout").await;

    let reprobed = reprobe(Some(&on_box))
        .await
        .expect("a re-probe of a row this test just probed writes a row");
    let status = status_of(&reprobed);
    println!("--- after the logout ---");
    println!(
        "re-probed agent_box.probe.status = {status}, credential tier = {}, enabled = {}",
        credential_tier_of(&reprobed),
        reprobed.enabled
    );
    let after = credential_report(&candidates, &credential, &env);
    print!("{after}");

    assert_eq!(
        status,
        ProbeStatus::Unauthenticated,
        "the logout completed and the box still reads `{status}`. Either the vendor leaves its \
         credential file in place — in which case the file the seed watches is not the one a logout \
         removes — or a declared environment variable is still set. What this box holds now:\n{after}"
    );
    assert!(
        !reprobed.enabled,
        "an unauthenticated box is left disabled (plan D50)"
    );
    println!("the box is back where the login proof can be run again.");
}
