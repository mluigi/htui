//! The box probe against the agent this box really has installed (`docs/ANA-4.md` §11 criterion 9,
//! plan MOD-2 T26).
//!
//! `#[ignore]` by default, exactly as `tests/acp_live.rs` is: it spawns whatever is installed here,
//! and a box without `node` or without the ACP adapter is not a failing build. Run it by hand:
//!
//! ```text
//! cargo test -p htui-agent --features test-support --test probe_live -- --ignored --nocapture
//! ```
//!
//! It **burns no model tokens**: tier 1 runs `--version` children and tier 2 completes `initialize`,
//! both protocol or CLI traffic, and no prompt is ever sent. `acp_live.rs` proves that the seeded
//! `claude` row *resolves and handshakes* when a test drives the SDK by hand; this file proves the
//! half milestone 5 owns — that [`probe_agent`] turns the same row into an `agent_box` row saying
//! `status = "ready"` with a `protocol_version` of 1, and that pointing `HTUI_TOOL_NODE` at nothing
//! turns it into `missing` **without tier 2 ever being reached**.
//!
//! Not "without spawning anything": with `versions: true` tier 1 still runs `claude --version`,
//! `npx --version` and `npm root -g` before it discovers that `node` is not there. What D50
//! promises, and what these tests prove, is that nothing is spawned *after* a required tool
//! resolves nowhere — the adapter itself is never started.
//!
//! This is the only file in the workspace allowed to probe an unmodified seed row (blueprint H-7):
//! every other suite uses a registry whose tools cannot resolve, so `cargo test` never starts the
//! real adapter.

use std::path::PathBuf;
// Only the survivor check reads it, and an unconditional import is an unused one everywhere else.
#[cfg(target_os = "linux")]
use std::time::Duration;

use chrono::Utc;
use htui_agent::acp::Handshake;
use htui_agent::driver::DriverFuture;
use htui_agent::launch::{AcpSettings, ResolvedLaunch};
use htui_agent::probe::{
    ProbeContext, ProbeEnv, ProbeOutcome, ProbeStatus, SpawnTier2, Tier2, probe_agent,
};
use htui_core::model::{Agent as AgentRow, AgentBox, BoxId};
use serde_json::json;

/// The seeded `claude` row, exactly as `PgStore::seed_if_empty_as` inserts it.
fn claude_row() -> AgentRow {
    htui_core::model::agent::seed_rows(Utc::now())
        .into_iter()
        .find(|agent| agent.name == "claude")
        .expect("the seed rows carry `claude`")
}

/// The row a probe decided to write, or the reason there is none.
fn row_of(outcome: ProbeOutcome) -> AgentBox {
    match outcome {
        ProbeOutcome::Row(row) => row,
        ProbeOutcome::Kept { reason } => panic!("the probe wrote nothing: {reason}"),
    }
}

/// A tier 2 that cannot run: being called at all is the failure.
///
/// This is how "nothing is spawned" is *proved* rather than hoped for (plan D50). A test that
/// merely looked for a surviving process would pass just as well if the adapter had been started
/// and had exited on its own.
struct NeverTier2;

impl Tier2 for NeverTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        _settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move { panic!("tier 2 ran for a row whose `node` resolved nowhere") })
    }
}

/// Every process this test process is the direct parent of, as pid strings.
///
/// By **pid**, never by a `pgrep -f` pattern: `pgrep -f node` matches the shell that launched
/// `cargo test`, the editor, and any other `node` on the box, so a pattern check fails for reasons
/// that have nothing to do with this code (`tests/acp_live.rs` learned that the hard way). The
/// per-thread `children` file is the kernel's own answer to "what did *I* start", and a child that
/// was killed but not reaped still appears in it — which is the point: `acp::handshake` promises
/// the tree is killed *and* reaped before it returns.
///
/// Read for every thread, not just the main one: a `tokio::spawn`ed task can fork from any worker,
/// and `children` is attributed to the thread that forked.
///
/// `children` exists only with `CONFIG_PROC_CHILDREN=y`. On a kernel built without it every read
/// fails, and an empty answer would make the survivor assertion pass **vacuously** — a green test
/// that checked nothing. So a run where not one `children` file was readable falls back to
/// [`children_by_ppid`], which reads a field every kernel has.
#[cfg(target_os = "linux")]
fn children_of_this_process() -> Vec<String> {
    let threads = std::fs::read_dir("/proc/self/task").expect("this process's own thread list");
    let mut pids = Vec::new();
    let mut readable = false;
    for thread in threads.flatten() {
        // A thread that exited between the listing and the read is not an error: it has no
        // children left either.
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
/// The `ppid` field is fourth, after `pid` and a `comm` that may itself hold spaces and parens —
/// hence the split at the **last** `)`, the same parse `tests/probe.rs`'s `assert_not_running` does
/// for the state field. Slower than `children` and racier (a pid can be reused between the listing
/// and the read), which is why it is the fallback and not the first answer.
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

/// Criterion 9: this box answers for itself.
#[tokio::test]
#[ignore = "spawns the agent installed on this box"]
async fn the_seeded_claude_row_probes_ready_with_a_v1_handshake() {
    let ctx = ProbeContext {
        env: ProbeEnv::host(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
        now: Utc::now(),
    };

    let row = row_of(
        probe_agent(
            &claude_row(),
            BoxId::new(),
            None,
            &ctx,
            &SpawnTier2::default(),
        )
        .await,
    );

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    // Quoted in the milestone write-up: this is `agent_box.probe` as it lands on this box.
    println!(
        "agent_box.probe = {}",
        serde_json::to_string_pretty(&probe).expect("the snapshot re-serialises")
    );
    println!(
        "agent_box columnar: enabled={} version={:?} path={:?} probed_at={:?}",
        row.enabled, row.version, row.path, row.probed_at
    );

    assert_eq!(
        probe["status"],
        json!(ProbeStatus::Ready.as_str()),
        "the claude row is runnable on this box: {probe}"
    );
    assert_eq!(
        probe["handshake"]["protocol_version"],
        json!(1),
        "MOD-2 speaks wire protocol 1 (ANA-4 §3)"
    );
    assert!(
        !probe["tools"]["node"].is_null(),
        "tier 1 captured node's version: {probe}"
    );
    assert!(
        !probe["resolved"].is_null(),
        "a ready row resolved: {probe}"
    );
    assert!(row.enabled, "a `ready` box is enabled (plan D50)");
    assert_eq!(row.probed_at, Some(ctx.now));

    // The child is owned by `acp::handshake`, so it is not reachable from here; what is checkable
    // is that this process has none left. `handshake` kills *and reaps* on every exit path, so
    // even a zombie would fail this.
    #[cfg(target_os = "linux")]
    {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let survivors = children_of_this_process();
        assert!(
            survivors.is_empty(),
            "the probe left children behind: {survivors:?}"
        );
    }
}

/// Criterion 9's other half, and plan D50's first rule: a required tool that resolves nowhere is
/// `missing`, the box is not enabled, and **tier 2 is never reached** — the adapter is not started.
///
/// Tier 1 does spawn: `versions: true` runs `claude --version`, `npx --version` and `npm root -g`
/// before the missing `node` is discovered, which is what the `#[ignore]` reason below says. The
/// [`NeverTier2`] that panics when it is called at all is what turns "nothing is spawned after the
/// miss" from a hope into a proof.
#[tokio::test]
#[ignore = "runs the version children installed on this box"]
async fn the_same_row_without_node_is_missing_and_spawns_nothing() {
    let mut env = ProbeEnv::host(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    // The checked override tier: a `HTUI_TOOL_NODE` pointing at a path that does not exist is
    // missing, not accepted. Injected as a value — the workspace forbids `unsafe`, so
    // `std::env::set_var` is not available and would not be thread-safe here anyway.
    env.vars.insert(
        "HTUI_TOOL_NODE".to_owned(),
        "/nonexistent/htui-probe-live/node".to_owned(),
    );
    let ctx = ProbeContext {
        env,
        now: Utc::now(),
    };

    let row = row_of(probe_agent(&claude_row(), BoxId::new(), None, &ctx, &NeverTier2).await);

    let probe = row
        .probe
        .clone()
        .expect("a probed row carries its snapshot");
    println!(
        "agent_box.probe without node = {}",
        serde_json::to_string_pretty(&probe).expect("the snapshot re-serialises")
    );

    assert_eq!(
        probe["status"],
        json!(ProbeStatus::Missing.as_str()),
        "a box without `node` cannot run the claude adapter: {probe}"
    );
    assert!(!row.enabled, "a `missing` box is not enabled (plan D50)");
    assert!(
        probe["resolved"].is_null(),
        "nothing resolved, so nothing could be spawned: {probe}"
    );
    assert!(probe["handshake"].is_null(), "tier 2 did not run: {probe}");
    assert!(
        probe["tools"]["node"].is_null(),
        "the override resolved nowhere, so `node` has no version: {probe}"
    );
    assert_eq!(row.path, None);
}
