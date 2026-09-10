//! `agy` over ACP against the adapter this box really has installed (`docs/ANA-4.md` §11
//! criterion 10, plan MOD-2 T33).
//!
//! `#[ignore]` by default, exactly as `tests/probe_live.rs` and `tests/acp_live.rs` are: it spawns
//! `agy_acp_server.par` and `agy --version`, and a box where the maintainer has not installed the
//! adapter (`R-AGT-10` reversed plan D57: `htui` installs it now, but only when asked) is not a
//! failing build. Run it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture
//! ```
//!
//! It **burns no model tokens**: case 1 completes `initialize` through the production probe, case 2
//! completes (or fails to complete) `initialize` a second time with the platform args withheld,
//! case 3 adds `session/new`, and case 4 runs the production driver only where `session/new` is
//! known to refuse. All of it is protocol traffic; no prompt is ever sent, and nothing here opens a
//! turn. Case 4 is the one that *could* — `AgentDriver::start` sends the first prompt as soon as
//! the handshake succeeds — so it probes first and stops on any box that is not
//! `unauthenticated`, which is stated again on the case itself.
//!
//! What each case is for:
//!
//! 1. [`the_seeded_agy_row_resolves_through_the_glob_appends_uid_and_answers_v1`] — criterion 10
//!    itself. The **unmodified** seed row resolves `agy_acp_server` through the glob, the Linux
//!    platform `args` reach the command line, `initialize` answers at `protocolVersion 1`, and the
//!    recorded status is the one this box's credential state calls for. The `handshake` object it
//!    prints is what `tests/fixtures/agy_acp_handshake.json` holds.
//! 2. [`the_par_mechanics_are_recorded`] — evidence for the two `.par` questions ANA-4 §11.14 lists
//!    as open (`docs/ANA-4.md:1391-1392`): what sits beside the server, and what the literal empty
//!    `--uid=` does. It **never fails on the adapter's behaviour**, only on a missing precondition.
//! 3. [`session_new_reports_its_config_options`] — the model-list question (`:1389-1390`, plan
//!    D64), driven by hand the way `tests/acp_live.rs` drives `initialize`. On a box the maintainer
//!    has not logged in, `session/new`'s error **is** the answer: it is printed, never asserted
//!    against.
//! 4. [`an_unauthenticated_box_is_told_what_the_agent_said`] — the same refusal, but through
//!    `AcpDriver::start` rather than by hand, so what is printed is the string a chat tab shows.
//!    Case 3 measures the SDK; this one measures `htui`, and it is where the milestone-6 review's
//!    headline finding is checked against a real adapter.
//!
//! **Preconditions**, stated so a miss fails by name rather than mysteriously:
//!
//! - The adapter is installed (plan T29, D57): an executable
//!   `~/.local/share/htui/agents/antigravity-acp/<version>/agy_acp_server.par` on Linux, the
//!   platform's equivalent elsewhere. Every case resolves the seed's glob first and panics with the
//!   in-app install instruction when it finds nothing.
//! - `agy` is on `PATH`. The seed's `agy` tool is a `path` probe and `probe_tools` requires **every**
//!   declared tool, so a box with the server but no CLI probes `missing` and never reaches tier 2
//!   (blueprint H-11). The precondition check names that too.
//! - The box's credential state is **observed, never assumed** (blueprint P-9). Hardcoding
//!   `unauthenticated` would turn this suite red the moment the maintainer completes the vendor's
//!   own login, so case 1 asserts the *consistency* — `ready` iff a credential tier answered — and
//!   the `unauthenticated` half only when the test's own independent check finds no candidate.
//!
//! Together with `tests/probe_live.rs` and `tests/acp_live.rs` this file is one of the few allowed
//! to probe an **unmodified seed row** (blueprint H-18): every other suite uses a registry whose
//! tools cannot resolve, so a plain `cargo test` never starts a real adapter. Reading the `agy` row
//! is data, not a code path keyed on an agent name (`R-AGT-5`) — the same thing `probe_live.rs` does
//! with `claude`.
//!
//! The four cases take [`ONE_AT_A_TIME`] so they never overlap, which is what lets each of them
//! assert on a process-wide survivor list without needing `--test-threads=1` on the command line.
//!
//! `tests/fixtures/agy_acp_handshake.json` is the `probe.handshake` document case 1 printed on
//! 2026-09-08, from `antigravity-acp` 1.1.1 on `linux-x86_64`, unauthenticated. It is a
//! **recording**, not an oracle: nothing here asserts against it, because the vendor's
//! `agentInfo.version` moves with every `agy_acp_server` release and a fixture-equality assertion
//! would go red on an upgrade that broke nothing. It carries no path, no session id and no
//! credential, so there was nothing to redact.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, NewSessionRequest};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use chrono::Utc;
use htui_agent::acp::{AcpDriver, client};
use htui_agent::driver::{AgentDriver, PermissionPolicy, SessionSpec, ToolExposure};
use htui_agent::error::DriverError;
use htui_agent::launch::{AgentLaunch, AgentSettings, CredentialProbe, ResolvedLaunch};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeStatus, SpawnTier2, ToolReport, platform_key,
    probe_agent, probe_tools,
};
use htui_agent::registry::caps_for;
use htui_core::model::{Agent as AgentRow, AgentBox, BoxId, StepId};
use serde_json::{Value, json};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// One live adapter at a time, across all three cases.
///
/// Not politeness about a 1.9 GB process: the survivor check reads `/proc/self/task/*/children`,
/// which is **process-wide**, and `cargo test` runs the cases as threads of one process. Two cases
/// overlapping would make case 1 see case 3's perfectly healthy child and call it a survivor. A
/// `tokio::sync::Mutex` rather than a `std` one because each `#[tokio::test]` builds its own
/// runtime and the guard is held across `.await`s; the lock itself is runtime-agnostic.
static ONE_AT_A_TIME: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Where the maintainer is told to get the adapter when a precondition fails.
///
/// Since `R-AGT-10` that is `htui` itself, so this names the action rather than a URL: the old
/// wording spelled one version and one platform, and it is exactly the copy that goes stale
/// without anything going red.
const INSTALL_HINT: &str = "\
install the adapter from inside htui (`R-AGT-10` — it installs it now, when asked):
  1. open Settings > Agents and put the cursor on the `agy` row with `j`/`k`
  2. press `i`, read the consent pane and answer `y`; nothing is fetched before that, and the
     archive, its size, the licence, the install directory and the digest rule are all on it
  3. the section re-probes when it finishes, so the row itself says whether this suite can run
     (a box that cannot reach the registry is given manual steps derived from the same row)
or point HTUI_TOOL_AGY_ACP_SERVER at the server this box already has";

/// How long the hand-driven cases give the adapter.
///
/// Generous rather than tuned: the `.par` is a ~1.9 GB CPython launcher, so a cold first start pays
/// for its own extraction. Measured on this box it answers `initialize` in about a second warm, and
/// the production probe's own tier-2 window is `HANDSHAKE_TIMEOUT` (60 s, `acp/mod.rs`) — this is
/// the same order, not a shorter one that would report a slow box as a broken one.
const LIVE_TIMEOUT: Duration = Duration::from_secs(120);

/// The seeded `agy` row, exactly as `PgStore::seed_if_empty_as` inserts it.
fn agy_row() -> AgentRow {
    htui_core::model::agent::seed_rows(Utc::now())
        .into_iter()
        .find(|agent| agent.name == "agy")
        .expect("the seed rows carry `agy`")
}

/// The row a probe decided to write, or the reason there is none.
fn row_of(outcome: ProbeOutcome) -> AgentBox {
    match outcome {
        ProbeOutcome::Row(row) => row,
        ProbeOutcome::Kept { reason } => panic!("the probe wrote nothing: {reason}"),
    }
}

/// The platform `args` the seed declares for **this** box: `["--uid="]` on the two Linux entries,
/// nothing anywhere else (ANA-4 §4.5 `:690-693`).
fn expected_platform_args() -> Vec<String> {
    match platform_key().as_str() {
        "linux-x86_64" | "linux-aarch64" => vec!["--uid=".to_owned()],
        _ => Vec::new(),
    }
}

/// This box, the parsed seed launch and its resolved tools — or a loud, named panic.
///
/// Every case starts here so that "the adapter is not installed" and "`agy` is not on `PATH`" read
/// as themselves rather than as a mystery `missing` status three assertions later.
async fn preconditions() -> (AgentLaunch, AgentSettings, ToolReport, ProbeEnv) {
    let row = agy_row();
    let env = ProbeEnv::host(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let launch: AgentLaunch =
        serde_json::from_value(row.launch.clone()).expect("the seeded agy launch parses");
    let settings: AgentSettings =
        serde_json::from_value(row.settings.clone()).expect("the seeded agy settings parse");

    let report = probe_tools(launch.discovery.as_ref(), &env)
        .await
        .expect("the tool walk runs on this box");

    assert!(
        !report.missing.iter().any(|name| name == "agy_acp_server"),
        "precondition: the seed's `agy_acp_server` glob resolved nowhere on {} — {INSTALL_HINT}",
        platform_key()
    );
    assert!(
        !report.missing.iter().any(|name| name == "agy"),
        "precondition: `agy` is not on PATH. The seed declares it as a required `path` tool, so \
         `probe_tools` reports the row `missing` and tier 2 never runs (blueprint H-11). Install \
         the CLI, or run this suite on a box that has it."
    );
    assert!(
        report.is_complete(),
        "precondition: the seed's discovery did not resolve completely: missing {:?}",
        report.missing
    );

    (launch, settings, report, env)
}

/// The seed's `credential` block, which T32 added and this suite reads as data.
fn credential_probe(launch: &AgentLaunch) -> CredentialProbe {
    launch
        .discovery
        .as_ref()
        .and_then(|discovery| discovery.credential.clone())
        .expect("the seeded agy row declares a credential block (plan D59)")
}

/// This test's **own** answer to "does this box hold an `agy` credential", independent of the code
/// under test (blueprint P-9).
///
/// Deliberately a second, dumber implementation of the two grammar rules the seed's candidates use
/// — a `%VAR%` pair anywhere, a leading `~` — rather than a call to `probe::resolve_credential`:
/// asserting the probe against itself would prove nothing about the box. An unset variable skips
/// the candidate, which is the walker's rule too.
fn observed_credential(probe: &CredentialProbe, env: &ProbeEnv) -> Option<String> {
    for pattern in &probe.files {
        let Some(path) = expand_by_hand(pattern, env) else {
            continue;
        };
        if path.is_file() {
            return Some(format!("file: {}", path.display()));
        }
    }
    for name in &probe.env {
        if env.var(name).is_some_and(|value| !value.is_empty()) {
            return Some(format!("env: {name} is set and non-empty"));
        }
    }
    None
}

/// `%VAR%` pairs substituted and a leading `~` expanded; `None` when a variable is unset or the
/// home directory is unknown, which is "skip this candidate".
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

/// Every process this test process is the direct parent of, as pid strings.
///
/// Copied from `tests/probe_live.rs` rather than shared, as this repo duplicates its process
/// helpers per file. By **pid**, never by a `pgrep -f` pattern: a pattern matches the shell that
/// launched `cargo test` and any editor that has the word in its command line, so it fails for
/// reasons that have nothing to do with this code. A child that was killed but not reaped still
/// appears here — which is the point, since `acp::handshake` promises the tree is killed *and*
/// reaped before it returns.
///
/// `children` exists only with `CONFIG_PROC_CHILDREN=y`; a run where not one was readable falls
/// back to [`children_by_ppid`] so the survivor assertion can never pass vacuously.
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
        "{what} left children behind: {survivors:?}"
    );
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

/// The first four bytes, named when they are a magic this test knows — `file(1)` without spawning
/// `file(1)`, and without reading a 1.9 GB executable into memory.
fn magic_of(path: &Path) -> String {
    use std::io::Read;
    let mut head = [0_u8; 4];
    let Ok(mut file) = std::fs::File::open(path) else {
        return "unreadable".to_owned();
    };
    let Ok(read) = file.read(&mut head) else {
        return "unreadable".to_owned();
    };
    let bytes = &head[..read];
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    match bytes {
        [0x7f, b'E', b'L', b'F'] => format!("{hex} (ELF)"),
        [b'P', b'K', ..] => format!("{hex} (zip)"),
        [b'#', b'!', ..] => format!("{hex} (script)"),
        _ => hex,
    }
}

// ---------------------------------------------------------------------------------------------
// Case 1 — ANA-4 §11 criterion 10
// ---------------------------------------------------------------------------------------------

/// Criterion 10: the unmodified seeded `agy` row answers for itself on this box.
///
/// The glob resolves, the platform `args` are appended, `initialize` comes back at protocol 1, and
/// the recorded `status` agrees with the box's *observed* credential state rather than with a
/// hardcoded expectation (blueprint P-9). Then the survivor check: nothing is left running.
#[tokio::test]
#[ignore = "spawns the agy ACP adapter installed on this box"]
async fn the_seeded_agy_row_resolves_through_the_glob_appends_uid_and_answers_v1() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let (launch, _, report, env) = preconditions().await;
    println!(
        "glob resolved agy_acp_server -> {}",
        report.found["agy_acp_server"].path.display()
    );
    println!("platform = {}", platform_key());

    let credential = credential_probe(&launch);
    let observed = observed_credential(&credential, &env);
    println!(
        "this test's own credential check: {}",
        observed
            .clone()
            .unwrap_or_else(|| "no candidate answered".to_owned())
    );

    let ctx = ProbeContext {
        env,
        now: Utc::now(),
    };
    let started = std::time::Instant::now();
    let row =
        row_of(probe_agent(&agy_row(), BoxId::new(), None, &ctx, &SpawnTier2::default()).await);
    println!("probe_agent took {:?} (tier 1 + tier 2)", started.elapsed());

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    // Quoted in the milestone write-up, and `probe["handshake"]` is what
    // `tests/fixtures/agy_acp_handshake.json` records.
    println!(
        "agent_box.probe = {}",
        serde_json::to_string_pretty(&probe).expect("the snapshot re-serialises")
    );
    println!(
        "agent_box columnar: enabled={} version={:?} path={:?} probed_at={:?}",
        row.enabled, row.version, row.path, row.probed_at
    );

    assert_eq!(
        probe["resolved"]["args"],
        json!(expected_platform_args()),
        "ANA-4 §4.6's per-platform append is the only place `--uid=` exists: {probe}"
    );
    assert!(
        probe["resolved"]["command"]
            .as_str()
            .is_some_and(|command| command.contains("agy_acp_server")),
        "the row's `${{agy_acp_server}}` placeholder substituted to the globbed server: {probe}"
    );
    assert_eq!(
        probe["handshake"]["protocol_version"],
        json!(1),
        "MOD-2 speaks wire protocol 1 (ANA-4 §3)"
    );
    assert!(
        !probe["tools"]["agy"].is_null(),
        "tier 1 captured the CLI's version: {probe}"
    );

    let auth_methods = probe["handshake"]["auth_methods"]
        .as_array()
        .expect("the handshake records the auth methods it was offered")
        .clone();
    println!(
        "authMethods = {auth_methods:?} ({} of them)",
        auth_methods.len()
    );
    assert!(
        !auth_methods.is_empty(),
        "ANA-4 §4.5 records that this server offers auth methods whether or not a credential \
         exists — which is exactly why D59's credential tier had to be built: {probe}"
    );

    // P-9's consistency rule, in both directions. `ready` iff a declared tier answered; the plan's
    // literal "records `unauthenticated`, `enabled = false`" is asserted only when this test's own
    // check agrees there is nothing to find, so a maintainer login turns this green rather than red.
    let tier = probe["credential"]
        .as_str()
        .expect("the snapshot records which credential tier answered (plan D59)")
        .to_owned();
    let status = probe["status"]
        .as_str()
        .expect("every snapshot carries a status")
        .to_owned();
    assert!(
        matches!(tier.as_str(), "file" | "env" | "absent"),
        "a row that declares candidates records one of the three tiers: {probe}"
    );
    let has_credential = matches!(tier.as_str(), "file" | "env");
    assert_eq!(
        status == ProbeStatus::Ready.as_str(),
        has_credential,
        "with a non-empty authMethods, `ready` means exactly `a credential tier answered` (D59): \
         status={status} credential={tier}"
    );
    assert_eq!(
        row.enabled,
        status == ProbeStatus::Ready.as_str(),
        "a `ready` box is enabled and nothing else is (plan D50)"
    );
    assert_eq!(
        has_credential,
        observed.is_some(),
        "the probe and this test disagree about the box: probe says {tier}, the test's own check \
         says {observed:?}"
    );
    if observed.is_none() {
        assert_eq!(
            status,
            ProbeStatus::Unauthenticated.as_str(),
            "installed but not logged in is `unauthenticated` (ANA-4 §4.5): {probe}"
        );
        assert!(!row.enabled, "an unauthenticated box is left disabled");
        println!(
            "OBSERVED CASE: installed-but-unauthenticated. Criterion 10's stated half holds here."
        );
    } else {
        println!("OBSERVED CASE: authenticated. `ready`, and the box is enabled.");
    }
    assert_eq!(row.probed_at, Some(ctx.now));

    // The child belongs to `acp::handshake`, which kills *and reaps* on every exit path, so even a
    // zombie fails this.
    #[cfg(target_os = "linux")]
    assert_no_survivors("the probe").await;
}

// ---------------------------------------------------------------------------------------------
// Case 2 — the `.par` mechanics ANA-4 §11.14 lists as open (`docs/ANA-4.md:1391-1392`)
// ---------------------------------------------------------------------------------------------

/// Evidence, not a verdict: what ships beside the server, and what the literal empty `--uid=` does.
///
/// This case **must never fail on the adapter's behaviour**. Both outcomes of the no-`--uid=` run
/// are printed and neither is asserted against, because either one answers the question — the flag
/// is load-bearing, or it is not. The only failures here are the preconditions
/// [`preconditions`] states.
#[tokio::test]
#[ignore = "spawns the agy ACP adapter installed on this box"]
async fn the_par_mechanics_are_recorded() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let (launch, settings, report, env) = preconditions().await;
    let server = report.found["agy_acp_server"].path.clone();

    // (a) What sits beside the server — ANA-4 §4.6 says to keep the whole unzipped directory
    // because the Windows bundle ships `localharness_external.exe`; `:1392` asks whether the
    // Linux/darwin build has an equivalent and whether it must stay.
    //
    // The listing answers the first half. The second half was answered **by hand** on 2026-09-08
    // rather than here: with `localharness_external` renamed aside, `agy_acp_server.par --uid=`
    // still completed `initialize` byte for byte, so the sibling is not needed for the handshake.
    // Whether a live *turn* needs it is T34's to find out, and the experiment is not automated
    // because it mutates the maintainer's install directory — a test that moved a 129 MB file and
    // then panicked would leave that install broken.
    println!("--- the install directory, as it is on disk ---");
    let dir = server
        .parent()
        .expect("the globbed server has a parent directory");
    println!("{}", dir.display());
    let mut siblings: Vec<_> = std::fs::read_dir(dir)
        .expect("the install directory is readable")
        .flatten()
        .collect();
    siblings.sort_by_key(std::fs::DirEntry::file_name);
    for entry in siblings {
        let path = entry.path();
        let meta = entry.metadata().expect("an entry has metadata");
        println!(
            "  {:<28} {:>13} bytes  mode {}  magic {}",
            entry.file_name().to_string_lossy(),
            meta.len(),
            mode_of(&meta),
            if meta.is_file() {
                magic_of(&path)
            } else {
                "<dir>".to_owned()
            }
        );
    }

    // (b) What `--uid=` does. The production path appends it (case 1 asserts that); here the same
    // resolution runs with the platform args **withheld**, which is exactly the launch
    // `tools::resolve` would have produced before D58 (blueprint H-3).
    println!("--- the same handshake with the platform args withheld ---");
    let without = htui_agent::launch::resolve(&launch, &report.tool_map())
        .expect("the seeded row's placeholders substitute");
    println!(
        "withheld args = {:?}; running: {} {:?}",
        expected_platform_args(),
        without.command,
        without.args
    );

    match handshake_over(&without, &settings, &env).await {
        Ok(handshake) => println!(
            "WITHOUT the platform args the adapter still answered: {}",
            serde_json::to_string_pretty(&handshake).expect("the handshake serialises")
        ),
        Err(err) => println!("WITHOUT the platform args the adapter did not answer: {err}"),
    }

    #[cfg(target_os = "linux")]
    assert_no_survivors("the no-args handshake").await;
}

/// `launch::spawn` → `AcpIo::from_spawned` → `acp::handshake`, the production tier 2 by hand.
///
/// Written out rather than reusing [`SpawnTier2`] so the caller can hand it a launch the probe
/// would never build — which is the whole point of case 2.
async fn handshake_over(
    resolved: &ResolvedLaunch,
    settings: &AgentSettings,
    env: &ProbeEnv,
) -> Result<htui_agent::acp::Handshake, DriverError> {
    let spawned = htui_agent::launch::spawn(resolved, &env.cwd).await?;
    let io = htui_agent::acp::AcpIo::from_spawned(spawned)?;
    htui_agent::acp::handshake(io, &settings.acp, LIVE_TIMEOUT).await
}

// ---------------------------------------------------------------------------------------------
// Case 3 — the model list and `configOptions` (`docs/ANA-4.md:1389-1390`, plan D64)
// ---------------------------------------------------------------------------------------------

/// `initialize` + `session/new`, driven by hand, printing `config_options()` verbatim.
///
/// Nothing in production surfaces `session/new`'s `configOptions` (the chat banner projects them
/// through `model_values`, blueprint P-8), so the only way to read them is to ask. No prompt is
/// sent: the session is opened, described and dropped.
///
/// On a box the maintainer has not logged in, `session/new` answers `Authentication required` —
/// that **is** the answer to `:1389-1390` for this box, and it is printed rather than asserted
/// against. The only assertion is `initialize`'s protocol version, which case 1 already earned.
///
/// The failure arrives on a path worth naming, because a reader of this output will otherwise
/// wonder where the message came from. `start_session` sends its request from a task it **spawns
/// on the connection** (`session.rs:885-905`), so a refused `session/new` fails an actor, and an
/// actor that fails first drops the foreground future (the ownership comment in `run_session` says
/// so). The closure therefore never returns and never reports; the agent's own text comes back as
/// the **connection future's** error. Both are read here, and whichever exists is printed.
#[tokio::test]
#[ignore = "spawns the agy ACP adapter installed on this box"]
async fn session_new_reports_its_config_options() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let (launch, settings, report, env) = preconditions().await;

    // With the platform args, unlike case 2: on Linux the server does not start without them, and
    // this case is about `session/new`, not about `--uid=`.
    let mut resolved = htui_agent::launch::resolve(&launch, &report.tool_map())
        .expect("the seeded row's placeholders substitute");
    resolved.args.extend(report.extra_args());
    println!("running: {} {:?}", resolved.command, resolved.args);

    let mut spawned = htui_agent::launch::spawn(&resolved, &env.cwd)
        .await
        .expect("the adapter starts");
    // Only the unix assertion below reads it, and an unconditional binding is an unused variable on
    // Windows — which `cargo clippy --target x86_64-pc-windows-msvc` is what catches.
    #[cfg(unix)]
    let pid = spawned.pid().expect("the child reports a pid");
    let writer = spawned.take_stdin().expect("stdin is piped");
    let reader = spawned.take_stdout().expect("stdout is piped");

    let transport = ByteStreams::new(
        writer.into_inner().compat_write(),
        reader.into_inner().compat(),
    );
    let capabilities = client::client_capabilities(&settings.acp.client_capabilities);
    let cwd = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // The report leaves through a `oneshot`, as `acp_live.rs` does it, rather than as the
    // `connect_with` future's own value: a refused `session/new` makes that future resolve to the
    // agent's error, which would throw the closure's value away — and the closure's value is the
    // evidence this case exists to collect.
    let (report_tx, report_rx) = tokio::sync::oneshot::channel();
    let driven = Client.builder().name("htui-live-probe").connect_with(
        transport,
        async move |cx: ConnectionTo<Agent>| {
            let init = match cx
                .send_request(
                    InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(capabilities)
                        .client_info(client::client_info()),
                )
                .block_task()
                .await
            {
                Ok(init) => init,
                Err(err) => {
                    let _ = report_tx.send(json!({ "initialize": { "error": err.to_string() } }));
                    return Ok(());
                }
            };
            let opened = cx
                .build_session_from(NewSessionRequest::new(cwd))
                .block_task()
                .start_session()
                .await;
            let session = match opened {
                Ok(session) => json!({
                    "outcome": "ok",
                    "session_id": session.session_id().0.to_string(),
                    "config_options": serde_json::to_value(session.config_options())
                        .unwrap_or(Value::Null),
                    "modes": serde_json::to_value(session.modes()).unwrap_or(Value::Null),
                    "meta": serde_json::to_value(session.meta()).unwrap_or(Value::Null),
                }),
                Err(err) => json!({
                    "outcome": "error",
                    "as_the_caller_sees_it": err.to_string(),
                    "document": serde_json::to_value(&err).unwrap_or(Value::Null),
                }),
            };
            let _ = report_tx.send(json!({
                "initialize": serde_json::to_value(&init).unwrap_or(Value::Null),
                "session_new": session,
            }));
            Ok(())
        },
    );

    let (connection, report) =
        tokio::time::timeout(LIVE_TIMEOUT, async { tokio::join!(driven, report_rx) })
            .await
            .expect("the adapter answered within the live timeout");

    match &connection {
        Ok(()) => println!("the connection ended cleanly"),
        // Not a failure of this case: on a refused `session/new` this *is* where the agent's own
        // message lands (see the doc above).
        Err(err) => println!("the connection ended with: {err}"),
    }
    let answer = match (report, &connection) {
        // The closure got to report: `session/new` was answered, one way or the other.
        (Ok(answer), _) => answer,
        // The closure was dropped mid-`start_session`, which is what a refused `session/new` does.
        // The connection's error is then the whole of the agent's answer.
        (Err(_), Err(err)) => json!({
            "initialize": Value::Null,
            "session_new": {
                "outcome": "error",
                "reported_by": "the connection future; the client closure was dropped before \
                                `start_session` returned",
                "message": err.to_string(),
                "document": serde_json::to_value(err).unwrap_or(Value::Null),
            },
        }),
        (Err(err), Ok(())) => {
            panic!("the connection ended cleanly and the client reported nothing: {err}")
        }
    };
    println!(
        "initialize = {}",
        serde_json::to_string_pretty(&answer["initialize"]).expect("the response re-serialises")
    );
    println!(
        "session/new = {}",
        serde_json::to_string_pretty(&answer["session_new"]).expect("the answer re-serialises")
    );
    match answer["session_new"]["outcome"].as_str() {
        Some("ok") => println!(
            "D64: `config_options` above is what fills `agent.models` / `settings.acp.model_config_id`."
        ),
        _ => println!(
            "D64: this box cannot open a session, so it learned no model ids; the seed's \
             `models: []` and `model_config_id: null` stay as they are and the reason is recorded."
        ),
    }
    // Only when `initialize`'s own response survived: a `session/new` refusal takes the closure —
    // and with it the recorded response — down with it, and case 1 has already proven protocol 1
    // through the production probe. Asserting anything else here would be asserting against the
    // adapter's behaviour, which this case does not do.
    if !answer["initialize"].is_null() {
        assert_eq!(
            answer["initialize"]["protocolVersion"],
            json!(1),
            "the adapter echoed the pinned protocol version"
        );
    }

    spawned.kill_tree().await.expect("the process tree dies");
    let _ = spawned.wait().await;

    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the agent process {pid} outlived the kill"
        );
    }
    #[cfg(target_os = "linux")]
    assert_no_survivors("the hand-driven session").await;
}

// ---------------------------------------------------------------------------------------------
// Case 4 — what the user is actually told (the milestone-6 review's headline finding)
// ---------------------------------------------------------------------------------------------

/// The **production** path end to end — [`AcpDriver::start`] on the probed row — and the string a
/// chat tab would print.
///
/// Case 3 asks `session/new` by hand and reads the refusal off whichever future survived to carry
/// it, which is a fact about the SDK. This case asks the driver, which is what a `ChatStart` does,
/// and the answer is a fact about `htui`. Until the milestone-6 review that answer was `the session
/// task ended before the handshake` — the fallback for a handshake **nobody answered** — for every
/// box that is installed and not logged in: `SessionBuilder::start_session` sends its request from
/// a task spawned on the connection, an actor that fails first drops the foreground future, and the
/// vendor's own text died in a `warn!`. The two assertions below are about `htui`'s half only. The
/// message itself is printed, never asserted against, for the reason case 2 gives — the wording is
/// the vendor's and moves with their releases.
///
/// **It burns no model tokens, and the guard is what guarantees that.** `AgentDriver::start` sends
/// the first prompt the moment the handshake succeeds, so on a box the maintainer *has* logged in
/// this case would open a turn. It probes first and runs the session only where the refusal exists
/// to be read: a box that cannot authenticate. On any other box it says why it stopped and asserts
/// nothing.
#[tokio::test]
#[ignore = "spawns the agy ACP adapter installed on this box"]
async fn an_unauthenticated_box_is_told_what_the_agent_said() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let (_launch, _settings, _report, env) = preconditions().await;

    // The real snapshot, not a hand-built one: D58's recorded launch is what `start` spawns, and
    // reading it from an actual probe is what makes this case the whole chain rather than the last
    // link of it.
    let row = agy_row();
    let ctx = ProbeContext {
        env,
        now: Utc::now(),
    };
    let on_box = row_of(probe_agent(&row, BoxId::new(), None, &ctx, &SpawnTier2::default()).await);
    let status = on_box
        .probe
        .as_ref()
        .and_then(|probe| probe["status"].as_str())
        .unwrap_or_default()
        .to_owned();
    println!("agent_box.probe.status = {status}");
    if status != ProbeStatus::Unauthenticated.as_str() {
        println!(
            "SKIPPED: this box records `{status}`, and only `unauthenticated` refuses \
             `session/new` without a turn. Opening a session on a `ready` box would send the \
             first prompt and spend the maintainer's quota, which no case in this file does."
        );
        return;
    }

    let driver = AcpDriver::from_row_with_probe(&row, Some(&on_box), caps_for(&row))
        .expect("the seeded agy launch parses");
    let spec = SessionSpec {
        agent_id: row.id,
        step_id: StepId::new(),
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        extra_dirs: Vec::new(),
        env: BTreeMap::new(),
        model: None,
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        retain_raw: false,
        resume: None,
        budget_micros: None,
    };
    let opened = tokio::time::timeout(
        LIVE_TIMEOUT,
        driver.start(spec, "this prompt is never sent".to_owned()),
    )
    .await
    .expect("the adapter answered within the live timeout");

    let message = match opened {
        Err(DriverError::Transport(message)) => message,
        Err(other) => panic!("a refused `session/new` is a transport error, got {other:?}"),
        // Not a panic: the file's rule is that no case fails on the adapter's behaviour, and a
        // session opening here is the adapter disagreeing with its own probe. Dropping the handle
        // starts the `DROP_GRACE` shutdown, which is what any abandoned session gets.
        Ok(session) => {
            drop(session);
            println!(
                "INCONCLUSIVE: the probe said `unauthenticated` and `session/new` succeeded \
                 anyway. Nothing to report about a refusal that did not happen."
            );
            return;
        }
    };

    println!("what a chat tab shows on this box:\n{message}");
    assert!(
        message.starts_with("session/new failed:"),
        "the driver names the step that was refused: {message}"
    );
    assert!(
        !message.contains("the session task ended before the handshake"),
        "that fallback means `nobody answered`, and this agent answered very clearly: {message}"
    );

    // Exit (2) of `open_session`: the failure is composed on the task's own timeline and the task
    // is awaited afterwards, so the tree is killed *and* reaped before `start` returned.
    #[cfg(target_os = "linux")]
    assert_no_survivors("the refused chat start").await;
}
