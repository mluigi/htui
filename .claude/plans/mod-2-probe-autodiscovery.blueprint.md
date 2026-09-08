# Blueprint: MOD-2 milestone 5 — autodiscovery and box probe

**Plan**: `.claude/plans/mod-2-probe-autodiscovery.plan.md` (D43–D56, T22–T28 binding; its "Verified claims" table is taken as read).
**Design authority**: `docs/ANA-4.md` §4.6 (`:718-805`), §5.1, §5.3, §9 step 5 (`:1267-1273`), §11 criteria 9–10 (`:1360-1364`); `docs/ANA-5.md` §9 (`:2140-2189`); `docs/ANA-2.md` §7 (`:1609-1615`), §9 (`:1848-1866`). Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**. Anything not verified in source is marked **UNVERIFIED — implementer must check**.

Conventions inherited unchanged: `#![warn(missing_docs)]` on every lib, `unsafe_code = "forbid"` (so no `std::env::set_var` in tests — every environment-dependent unit takes an injected `ProbeEnv`), `clippy::all` not `pedantic`, MSRV 1.98, one `thiserror` type per crate (`DriverError`, `StoreError` — no new error type), no lock across an `.await`, `htui-store` never depends on `htui-agent`, no store handle on the render side (`R-NF-3`), and **no process is ever spawned on the UI task or inside the worker's `select!` arm** (`R-NF-3`, plan D53).

## Plan ≠ tree, resolved in the tree's favour

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | D46: `resolve_tool(&ToolProbe, &Env) -> Option<ToolResolution>` | The tier machinery already fails with `DriverError::Transport` when the `spawn_blocking` lookup cannot run (`tools.rs:113-123`, `:154-158`) | `resolve_tool` returns `Result<Option<ToolResolution>>`; swallowing a runtime fault as "missing" would mark a healthy box `missing` |
| P-2 | T23: the conformance case "asserts `probe` round-trips" | `conformance.rs` cases are `S: WriteStore` and `ReadStore` has no registry read (`traits.rs:30-45`); `agents()` is inherent on each store | The conformance case asserts the **writes** (insert with `probe`, update replacing it, `None` clearing it); read-back lives in `mem.rs`'s `agents_join_this_box_only` and a new `pg_criteria.rs` case |
| P-3 | D53: "the loop … takes an owned `Writer` … spawns the task … and `continue`s" | Every long operation the loop already defers goes through `AgentRuntime::serve` → `Served` (`store_worker.rs:477-486`), and the test harness polls those futures inline (`testkit.rs:171-190`) | The probe is served by `AgentRuntime::serve` too; it `tokio::spawn`s inside the runtime, keeps the `JoinHandle`, and answers `Served::Deferred`. The loop body's only change is one more variant in the arm at `:473-476`. Same ownership, same `continue`, one place that knows how to spawn |
| P-4 | D48 relies on `dirs`, listed as "already a workspace dependency" | `crates/htui-agent/Cargo.toml` does not declare it | `dirs = { workspace = true }` joins `regex` and `semver` in the agent crate's manifest (T24) |
| P-5 | File table: `acp/mod.rs` UPDATE, "handshake entry point beside `open_session`" | `acp/mod.rs` is 1454 lines and the handshake owns a child differently from `run_session` (below) | New file `crates/htui-agent/src/acp/handshake.rs`; `acp/mod.rs` gains `pub mod handshake; pub use handshake::{Handshake, handshake};` and `AcpIo::from_spawned` — that is the UPDATE |
| P-6 | D45: `probe.resolved {command, args, env}` | `ResolvedLaunch` (`launch.rs:340-348`) has no serde derives | `ResolvedLaunch` gains `Serialize, Deserialize` — **file-set addition** to T24 (`launch.rs`, one derive line). It is by definition "what `AgentDriver::start` actually spawns" (ANA-4:786), and milestone 6 reads it back as that type |
| P-7 | D52: refuse "with that same message" as `Writer::Buffered` | The message is a private fn (`writer.rs:329-331`) | `pub const REGISTRY_ON_SERVER_ONLY: &str` in `writer.rs`, used by `registry_writes_need_the_server()` and by the refusal — **file-set addition** to T27 (`crates/htui-store/src/writer.rs`; T23 does not touch it) |
| P-8 | T27 file set has no `testkit.rs` | `Harness::drive` enumerates the runtime-served requests by hand (`testkit.rs:161-165`) | `ProbeAgents` joins that list; `drive_to_end` awaits the runtime's background tasks before `shutdown` — **file-set addition** to T27 |
| P-9 | T25 file set has no `launch.rs` | Killing a probe child on task abort needs a synchronous kill | `Spawned::start_kill(&mut self) -> Result<()>` — **file-set addition** to T25 (`launch.rs`) |
| P-10 | Silent | A tool that resolves **below** `VersionProbe.min` | Recorded as found (`tools.node = "18.0.0"`), counted as missing: `status = "missing"`, nothing spawned. Stated in B.5 and H-5 |
| P-11 | D50: "non-empty `authMethods` **and no configured credential**" | Nothing in the tree knows a credential: `SessionSpec.env` is empty until MOD-10 (`agent_worker.rs:396-398`), and ANA-4:711's "no token" is `agy`'s own file layout | `unauthenticated` = non-empty `auth_methods`, full stop, in this milestone. H-4 names the consequence for milestone 6 |

---

## A. Module map (build order)

| # | File | Action | Task | Thesis (one line) and what must **not** go in it |
|---|---|---|---|---|
| 1 | `crates/htui-store/migrations/0002_agent_probe.sql` | CREATE | T22 | *The probe column and the two ANAs' folded contracts, forward-only.* Section D verbatim. **Not**: any edit to `0001`, any `agent_box` mirror change, MOD-4's rows |
| 2 | `crates/htui-store/tests/migrations.rs` | UPDATE | T22 | G-T22's four cases; `:300-304` `app_setting` count `2` → `12` |
| 3 | `crates/htui-core/src/model/agent.rs` | UPDATE | T23 | `AgentBox.probe: Option<Value>` with `#[serde(default)]`; `AgentSummary.on_box` doc (`:157`) loses "until the box is probed (MOD-2 milestone 5)" and says "when a probe has run on this box" |
| 4 | `crates/htui-core/src/store/mem.rs` | UPDATE | T23 | `upsert_agent_box` already clones the whole row (`:826-847`), so the field rides along; the `agents_join_this_box_only` literal (`:1210`) gains `probe` and asserts it back |
| 5 | `crates/htui-core/src/store/conformance.rs` | UPDATE | T23 | `upsert_agent_box_by_pk` (`:1255-1293`): insert with `probe: Some(json!({"status":"ready","source":"probe"}))`, update with `probe: None`, orphan unchanged |
| 6 | `crates/htui-store/src/pg/write.rs` | UPDATE | T23 | `upsert_agent_box` (`:416-442`) binds `$10 = row.probe.as_ref()`, `probe = EXCLUDED.probe` |
| 7 | `crates/htui-store/src/pg/read.rs` | UPDATE | T23 | `agents()` selects `ab.probe AS "box_probe?"` and fills `probe: row.box_probe` at `:587` |
| 8 | `crates/htui-store/tests/pg_criteria.rs`, `tests/writer_buffered.rs` | UPDATE | T23 | New Postgres case (G-T23); the literal at `writer_buffered.rs:261` gains `probe: None` |
| 9 | `crates/htui-store/.sqlx/` | UPDATE | T23 | Two query files regenerate (the upsert, the join) |
| 10 | `Cargo.toml`, `crates/htui-agent/Cargo.toml` | UPDATE | T24 | workspace `regex = "1.13"`, `semver = "1.0"`; agent crate adds `regex`, `semver`, `dirs` (P-4) |
| 11 | `crates/htui-agent/src/probe.rs` | CREATE | T24, T25 | *ANA-4 §4.6's two-tier probe: what this box can run, as a snapshot `agent_box.probe` holds.* Owns `ProbeEnv`, the three tiers, the glob walker, version capture, `ProbeSnapshot`, `probe_agent`. **Not**: any store handle or write (it returns an `AgentBox`; the caller writes), any `Backend`, any UI text, the `HTUI_TOOL_*` **unchecked** override semantics of `tools.rs`, and no `session/new` |
| 12 | `crates/htui-agent/src/launch.rs` | UPDATE | T24, T25 | T24: `#[derive(Serialize, Deserialize)]` on `ResolvedLaunch` (P-6). T25: `Spawned::start_kill` (P-9). Module doc `:6-7` ("filling it from `agent_box.probe.tools` is milestone 5's probe") stays true and is left |
| 13 | `crates/htui-agent/src/tools.rs` | UPDATE | T24 | Keeps `env_override_key`, `resolve`, `resolve_with`, `unresolved`; the Path / NodePackage / Glob arms become one call to `probe::resolve_tool`; `on_path`, `node_package`, `npm_root_global`, `exists` **move** to `probe.rs`; the module doc's "this is not the probe" paragraph is rewritten to "this is the chat-start caller of the probe's resolver"; `:195`'s milestone-5 message goes |
| 14 | `crates/htui-agent/src/lib.rs` | UPDATE | T24, T25 | `pub mod probe;` + re-exports (B.1); the doc line "Deliberately absent … probes" (`:17`) drops "probes" |
| 15 | `crates/htui-agent/tests/probe.rs` | CREATE | T24, T25 | Tier 1 over temp trees, version parsing, the walker, `handshake` over a duplex and over `sh`, status mapping, D51 |
| 16 | `crates/htui-agent/src/acp/handshake.rs` | CREATE | T25 | *`initialize` and nothing else, with the child owned by the caller's frame so every exit path kills it.* **Not**: `session/new`, a prompt, a mapper, a recorder, any `Inbound` handler |
| 17 | `crates/htui-agent/src/acp/mod.rs` | UPDATE | T25 | `pub mod handshake;` + re-export; `AcpIo::from_spawned`, used by `AcpDriver::io` (`:250-261`) |
| 18 | `crates/htui-agent/tests/probe_live.rs` | CREATE | T26 | `#[ignore]` criterion 9 |
| 19 | `crates/htui-store/src/writer.rs` | UPDATE | T27 | `pub const REGISTRY_ON_SERVER_ONLY` (P-7) |
| 20 | `crates/htui/src/store_worker.rs` | UPDATE | T27 | `StoreRequest::ProbeAgents`, `name()` → `"probe_agents"`, `try_serve` arm in the "no runtime" group (`:334-340`), loop arm in the runtime group (`:473-476`); the `Served::Deferred` doc (`agent_worker.rs:123`) becomes "a task the runtime owns answers this request itself, once" |
| 21 | `crates/htui/src/agent_worker.rs` | UPDATE | T27, T28 | T27: `AgentRuntime::probe`, `run_probe`, `background: Vec<JoinHandle<()>>`, `finish_background`, `shutdown` aborts background. T28: `PROBE_TTL`, `needs_reprobe`, `run_reprobe`, the spawn inside `start()` |
| 22 | `crates/htui/src/testkit.rs` | UPDATE | T27 | `ProbeAgents` in `drive`'s runtime list; `drive_to_end` calls `runtime.finish_background()` before `shutdown` (P-8) |
| 23 | `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T27 | `r`, `probing`, the status column; module doc `:3-5` and `NOT_PROBED` doc `:18` updated |
| 24 | `crates/htui/tests/settings.rs`, `crates/htui/tests/probe.rs` | UPDATE / CREATE | T27 | G-T27 |
| 25 | `crates/htui/tests/chat.rs` | UPDATE | T28 | G-T28 |
| 26 | `README.md` (Settings keys), HANDOFF phase note | UPDATE (docs) | close-out | `r refresh agents`; the ANA-4 §9 and §4.6 amendments (D43, D45); H-1's mirror-rebuild note; milestone 6's list from section I |

---

## B. Interfaces, exactly

### B.1 `crates/htui-agent/src/probe.rs` — types

```rust
//! ANA-4 §4.6's two-tier probe (plan MOD-2 D45–D51): what this box can run, as the snapshot
//! `agent_box.probe` holds. Tier 1 resolves every `discovery.tools` entry and captures versions;
//! tier 2 spawns the resolved launch and completes `initialize`. Nothing here writes a store row:
//! [`probe_agent`] returns the `AgentBox` and the caller decides where it lands (`R-NF-3` keeps
//! that caller off the UI task).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{Agent, AgentBox, BoxId, Transport};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::acp::{Handshake, HANDSHAKE_TIMEOUT};
use crate::driver::DriverFuture;
use crate::error::{DriverError, Result};
use crate::launch::{AcpSettings, AgentLaunch, AgentSettings, Discovery, ResolvedLaunch, ToolMap,
                    ToolProbe, VersionProbe};

/// How long one `--version` child may run before it is killed and its version recorded unknown.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// The box as the resolver sees it: injected, never read from `std::env` inside a tier, so every
/// tier is testable without `set_var` (the workspace forbids `unsafe`, `tools.rs:69-73`).
#[derive(Debug, Clone)]
pub struct ProbeEnv {
    /// Where relative `node_modules` and `which` lookups start.
    pub cwd: PathBuf,
    /// `<os>-<arch>` in the ACP registry's vocabulary (`launch.rs:127-129`): [`platform_key`].
    pub platform: String,
    /// What a leading `~` expands to. `None` skips every `~` pattern.
    pub home: Option<PathBuf>,
    /// The environment: `HTUI_TOOL_<NAME>` overrides, `%VAR%` expansion, `PATH`.
    pub vars: BTreeMap<String, String>,
    /// Whether tier 1 runs `--version` children. `tools::resolve` and the `ChatStart` re-probe
    /// say `false` (D55 "tier 2 only"); `Settings > r` says `true`.
    pub versions: bool,
    /// Per `--version` child.
    pub version_timeout: Duration,
}

impl ProbeEnv {
    /// This process's box: `std::env::vars_os()` (lossy), `dirs::home_dir()`, [`platform_key`],
    /// `versions: true`, [`VERSION_TIMEOUT`].
    #[must_use] pub fn host(cwd: PathBuf) -> Self;
    /// The same env with `versions: false`.
    #[must_use] pub fn without_versions(self) -> Self;
    /// `vars[key]`, for the override and `%VAR%` tiers.
    #[must_use] pub fn var(&self, key: &str) -> Option<&str>;
}

/// `std::env::consts::{OS, ARCH}` as `<os>-<arch>`, with `macos` spelled `darwin` — the one
/// difference between Rust's names and the registry's five keys.
#[must_use] pub fn platform_key() -> String;

/// One tool, found (D46).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResolution {
    /// The file `${name}` substitutes to. Absolute after `which`; as expanded after a glob.
    pub path: PathBuf,
    /// Captured version, `None` = present, version unknown (D47).
    pub version: Option<String>,
    /// [`PlatformGlob::args`] of the matching platform, appended to `agent.launch.args`.
    pub args: Vec<String>,
    /// `version` parsed below `VersionProbe.min` (P-10). Counted as missing by [`probe_tools`].
    pub below_min: bool,
}

/// Every tool of one `discovery`, resolved or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolReport {
    /// Resolved tools by `${name}`.
    pub found: BTreeMap<String, ToolResolution>,
    /// Names that resolved nowhere or below their floor, in `BTreeMap` order.
    pub missing: Vec<String>,
}

impl ToolReport {
    /// `found` as the map `launch::resolve` substitutes from.
    #[must_use] pub fn tool_map(&self) -> ToolMap;
    /// The snapshot's `tools` key: every **found** name → its version (or `null`).
    #[must_use] pub fn versions(&self) -> BTreeMap<String, Option<String>>;
    /// Every found tool's `args`, name order — the platform append of ANA-4:698-699.
    #[must_use] pub fn extra_args(&self) -> Vec<String>;
    /// `missing.is_empty()`.
    #[must_use] pub fn is_complete(&self) -> bool;
}

wire_enum!(
    /// `probe.status` (ANA-4 §4.6, D50).
    ProbeStatus {
        /// Resolved, spawned, `initialize` answered, no auth demanded.
        Ready => "ready",
        /// `initialize` answered with a non-empty `authMethods` (P-11).
        Unauthenticated => "unauthenticated",
        /// A required tool resolved nowhere or below its floor. Nothing was spawned.
        Missing => "missing",
        /// The launch resolved but did not spawn, or `initialize` did not complete.
        Failed => "failed",
    }
);

wire_enum!(
    /// `probe.source` (D45): who wrote the row. A `manual` row survives a probe that finds nothing.
    #[derive(Default)]
    ProbeSource {
        /// Written by [`probe_agent`].
        #[default]
        Probe => "probe",
        /// Hand-written in Settings (the editor is a later milestone's; the rule is honoured now).
        Manual => "manual",
    }
);

/// `agent_box.probe` (ANA-4 §4.6, `:766-785`, plus D45's `source`). Field order **is** key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSnapshot {
    /// `agent.transport` at probe time.
    pub transport: Transport,
    /// What `AgentDriver::start` spawns on this box; `None` when `status` is `missing`.
    pub resolved: Option<ResolvedLaunch>,
    /// `${name}` → version, for every **found** tool. A missing tool has no key.
    pub tools: BTreeMap<String, Option<String>>,
    /// Tier 2's answer; `None` when tier 2 did not run.
    pub handshake: Option<Handshake>,
    /// D50.
    pub status: ProbeStatus,
    /// On `failed`: the failure text line by line — `handshake`'s error already carries the
    /// child's stderr after the reason (`acp/mod.rs:1362-1373`'s shape). `None` otherwise.
    pub stderr_tail: Option<Vec<String>>,
    /// D45. Absent in a hand-written row reads as `probe`, which is the safe direction: a row
    /// nobody marked `manual` may be refreshed.
    #[serde(default)]
    pub source: ProbeSource,
}

impl ProbeSnapshot {
    /// `row.probe` parsed; `None` when the column is `NULL` or does not parse (never an error:
    /// the Settings tab must still list the row).
    #[must_use] pub fn from_row(row: &AgentBox) -> Option<Self>;
    /// `serde_json::to_value(self)` — the column's document.
    #[must_use] pub fn to_value(&self) -> Value;
}

/// Tier 2's outcome mapped onto D50: empty `auth_methods` → `ready`, otherwise `unauthenticated`.
#[must_use] pub fn status_for(handshake: &Handshake) -> ProbeStatus;
```

`lib.rs` re-exports: `pub use probe::{ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeSource, ProbeStatus, SpawnTier2, Tier2, ToolReport, ToolResolution, platform_key, probe_agent, probe_tools};` and `pub use acp::{Handshake, handshake};` (the existing `acp::{…}` line grows by two names).

### B.2 `probe.rs` — resolution (tier 1)

```rust
/// One probe, one answer (D46). No override tier here: the two callers apply it themselves with
/// different rules (B.6), and this function is the tier walk they share.
///
/// - `Path`: the first of `names` that `which::which_in(name, env.var("PATH"), &env.cwd)` finds;
///   `version` captured when `env.versions` and the probe has one.
/// - `NodePackage`: `<cwd>/node_modules/<package>/<entry>`, else `<npm root -g>/<package>/<entry>`
///   (`npm` looked up by `which`, `npm root -g` run once; a missing or failing `npm` is "not
///   found", as `tools.rs:149-153` already ruled). `version` is the `"version"` of the package's
///   `package.json`, read from disk — no spawn — when `env.versions`. The `fallback` is **not**
///   consulted (a `ToolMap` entry is one string, `tools.rs:125-130`).
/// - `Glob`: [`glob_first`] over the matching platform's `patterns` then the platform-independent
///   `patterns`; `args` = the matching platform's `args` whenever `env.platform` has an entry,
///   whichever list matched. `version: None` always (handshake-only, ANA-4 probe table).
///
/// # Errors
/// [`DriverError::Transport`] when a blocking lookup could not be scheduled (P-1). "Found nothing"
/// is `Ok(None)`.
pub async fn resolve_tool(probe: &ToolProbe, env: &ProbeEnv) -> Result<Option<ToolResolution>>;

/// Every tool of `discovery`, override tier first (`tools::env_override_key`, **checked**: an
/// override whose path does not exist is missing, with a `warn!` naming the key — a probe that
/// recorded a nonexistent override as `ready` would lie), then [`resolve_tool`]. Walks **all**
/// tools rather than stopping at the first miss, so the snapshot lists every version it did find.
/// A found tool with `below_min` is moved to `missing`. `None` discovery → empty report.
///
/// # Errors
/// As [`resolve_tool`].
pub async fn probe_tools(discovery: Option<&Discovery>, env: &ProbeEnv) -> Result<ToolReport>;
```

### B.3 `probe.rs` — the glob walker (D48)

```rust
/// `%VAR%` (any position, from `env.vars`) and a leading `~` (from `env.home`) expanded, then
/// split on `/`. `None` when a named variable is unset or `~` has no home: the pattern is skipped,
/// not an error — a Linux box legitimately has no `%LOCALAPPDATA%`.
///
/// Returns the literal root (every leading segment without `*`, joined with `PathBuf::push`, so
/// an expanded `C:\Users\x\AppData\Local` stays one segment) and the remaining segments.
#[must_use] pub fn expand(pattern: &str, env: &ProbeEnv) -> Option<(PathBuf, Vec<String>)>;

/// `*` matches any run of characters **within** one name; there is no `**`, no `?`, no `[..]`
/// (the seeds use `*` per segment and nothing else, `agent_agy.json:19-28`). Case-insensitive on
/// Windows (`cfg!(windows)`), exact elsewhere.
#[must_use] pub fn segment_matches(pattern: &str, name: &str) -> bool;

/// Synchronous: from `root`, one segment at a time — a literal segment is pushed, a `*` segment
/// reads the directory and forks on every matching entry. Every leaf that `is_file()`.
#[must_use] pub fn walk(root: &Path, segments: &[String]) -> Vec<PathBuf>;

/// Newest `mtime` first (`metadata().modified()`, unreadable = `UNIX_EPOCH`), ties broken by path
/// **descending** so a JetBrains install with three versioned directories answers the same one on
/// every run.
#[must_use] pub fn newest(matches: Vec<PathBuf>) -> Option<PathBuf>;

/// `patterns` in order; the first pattern with any match wins, `newest` of its matches. The walk
/// runs on `spawn_blocking` (it is `read_dir` and `stat`, `launch.rs:539-544`'s rule).
///
/// # Errors
/// [`DriverError::Transport`] when the blocking task did not run.
pub async fn glob_first(patterns: &[String], env: &ProbeEnv) -> Result<Option<PathBuf>>;
```

### B.4 `probe.rs` — version capture (D47)

```rust
/// The first capture group of `pattern` on the first line of `output` that matches (stdout lines,
/// then stderr lines, each `trim`med). A pattern that does not compile, or matches nothing, is
/// `None` — "present, version unknown". A pattern with no group yields the whole match.
#[must_use] pub fn extract_version(output: &str, pattern: &str) -> Option<String>;

/// `semver::Version::parse(version) < parse(min)`. Either side unparsable → `false` (ANA-4:762's
/// tolerance: `agy`'s string is undocumented and must not fail the probe).
#[must_use] pub fn below_min(version: &str, min: &str) -> bool;

/// Runs `<path> <probe.args>` through `launch::spawn` (so Windows gets `CREATE_NO_WINDOW` and a
/// job object on a `--version` too), `read_stdout_to_end` + `wait` under `env.version_timeout`;
/// on timeout `kill_tree` and `None`. Feeds stdout then `stderr_tail()` to [`extract_version`].
pub async fn capture_version(path: &Path, probe: &VersionProbe, env: &ProbeEnv) -> Option<String>;

/// `"version"` of `<package_dir>/package.json`, or `None`.
pub async fn package_version(package_dir: &Path) -> Option<String>;
```

### B.5 `probe.rs` — `probe_agent` (D49–D51)

```rust
/// Tier 2, as a seam: production spawns the launch; a test answers from a duplex or a canned value.
pub trait Tier2: Send + Sync {
    /// Spawn `launch` in `env.cwd`, complete `initialize`, kill the child, answer.
    fn handshake<'a>(
        &'a self,
        launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake>;
}

/// The production tier 2: `launch::spawn` → `AcpIo::from_spawned` → `acp::handshake`.
#[derive(Debug, Clone, Copy)]
pub struct SpawnTier2 {
    /// [`HANDSHAKE_TIMEOUT`] by default.
    pub timeout: Duration,
}
impl Default for SpawnTier2 { /* timeout: HANDSHAKE_TIMEOUT */ }
impl Tier2 for SpawnTier2 { /* as documented */ }

/// What one probe run is told about the box and the clock.
#[derive(Debug, Clone)]
pub struct ProbeContext {
    /// The box.
    pub env: ProbeEnv,
    /// `probed_at`, `updated_at`, and `handshake.at`'s fallback.
    pub now: DateTime<Utc>,
}

/// What [`probe_agent`] decided.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    /// Write this row (`upsert_agent_box`).
    Row(AgentBox),
    /// D51: the stored row is `source: manual` and this probe found nothing. Write nothing — not
    /// even `probed_at`.
    Kept {
        /// For the log: `"manual entry kept: the probe resolved nothing"`.
        reason: &'static str,
    },
}

/// One registry row on this box, start to finish (D50 in this order):
///
/// 1. `agent.launch` → `AgentLaunch` (parse failure → `failed`, `stderr_tail = [the serde text]`,
///    nothing spawned); `agent.settings` → `AgentSettings` (`unwrap_or_default`, the registry's
///    rule).
/// 2. [`probe_tools`]. A `Transport` error → `failed` with its text.
/// 3. Incomplete report → `missing`, `resolved: None`, **nothing spawned**.
/// 4. `launch::resolve(&launch, &report.tool_map())` — `Unresolved` (a placeholder with no
///    `discovery` entry) → `missing` too. Then `resolved.args.extend(report.extra_args())`.
/// 5. Tier 2 runs iff `agent.transport == Transport::Acp` and
///    `launch.discovery.as_ref().is_none_or(|d| d.handshake)`; otherwise `ready` on resolution,
///    `handshake: None` (a `cli` row has no `initialize` to complete — milestone 8's problem).
/// 6. `tier2.handshake(..)`: `Ok(h)` → [`status_for`], `handshake: Some(h)`; `Err(e)` →
///    `failed`, `stderr_tail: Some(e.to_string().lines()…)`.
/// 7. `status == Missing` and `existing`'s snapshot has `source == Manual` → `Kept` (D51).
///    Anything the probe *did* find refreshes a manual row like any other, and the refreshed row
///    is `source: probe`.
/// 8. Otherwise [`agent_box_row`].
pub async fn probe_agent(
    agent: &Agent,
    box_id: BoxId,
    existing: Option<&AgentBox>,
    ctx: &ProbeContext,
    tier2: &dyn Tier2,
) -> ProbeOutcome;

/// The columnar projection ANA-4:764-766 names, over `snapshot`:
/// `enabled = status == Ready`; `version` = `handshake.agent_version`, else
/// `tools[agent.name]` (the tool that shares the agent's name), else `None`;
/// `path = resolved.command`; `probed_at = Some(now)`; `quota` / `quota_at` **carried over from
/// `existing`** (the probe owns neither; MOD-7 / milestone 7 write them); `updated_at = now`;
/// `probe = Some(snapshot.to_value())`.
#[must_use]
pub fn agent_box_row(
    agent: &Agent,
    box_id: BoxId,
    existing: Option<&AgentBox>,
    snapshot: &ProbeSnapshot,
    now: DateTime<Utc>,
) -> AgentBox;
```

### B.6 `crates/htui-agent/src/tools.rs` — the delegation (D46)

`resolve_with` keeps its signature and its first tier verbatim (`:85-89`: override → insert, **unchecked** — `the_override_wins_over_every_tier` pins a nonexistent path as accepted, and that is the escape hatch for a box whose layout no tier understands). The three arms at `:90-98` become:

```rust
let env = crate::probe::ProbeEnv::host(cwd.to_path_buf()).without_versions();
// … per tool, after the override check:
match crate::probe::resolve_tool(probe, &env).await? {
    Some(found) => { map.insert(name.clone(), found.path.to_string_lossy().into_owned()); }
    None => return Err(unresolved(name, probe)),
}
```

`unresolved`'s Glob arm: `"no file matches any glob pattern for this platform"`. `on_path`, `node_package`, `npm_root_global`, `exists` are deleted here and live in `probe.rs`. `ProbeEnv::host` is built once per `resolve_with` call, outside the loop. Known limit, stated in the module doc: a `ToolMap` value is one string, so a glob tool's `args` are **not** applied on this path — milestone 6 reads `agent_box.probe.resolved` instead of resolving again (H-3).

### B.7 `crates/htui-agent/src/acp/handshake.rs` (D49)

```rust
//! Tier 2 of the probe: `initialize` and nothing else (plan D49). One task owns the whole
//! `connect_with` future, `block_task()` is called exactly once, and the child is owned by the
//! **caller's** frame — not by the task — so a timeout, an actor failure and a success all reach
//! the same `kill_tree`. `run_session` keeps its child in the task (`acp/mod.rs:600-606`) because
//! a session has commands to serve after the handshake; a probe has none.

use agent_client_protocol::schema::v1::InitializeResponse;

/// What `initialize` answered — the snapshot's `handshake` object, keys as ANA-4:771-776.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    /// When the response arrived (`Utc::now().trunc_subsecs(6)`, the recorder's precision).
    pub at: DateTime<Utc>,
    /// `protocolVersion` as the agent echoed it. ANA-4:707 warns the echo is not proof of
    /// support; `htui` pins 1 and records what came back.
    pub protocol_version: u16,
    /// `agentInfo.name`.
    pub agent_name: Option<String>,
    /// `agentInfo.version` — the version ANA-4:766 says the row records when tier 2 ran.
    pub agent_version: Option<String>,
    /// `agentCapabilities` in the SDK's serde form (camelCase keys), verbatim. Reshaping it into
    /// ANA-4's illustrative `{load_session, session: [..]}` would be a second schema for a value
    /// no consumer reads yet (ANA-2 §7 and ANA-5 §4.2 read `status` only).
    pub capabilities: Value,
    /// `authMethods[].id` as strings (`oauth-personal`, … for `agy`; `[]` for the `claude`
    /// adapter, `tests/fixtures/claude_acp_handshake.jsonl` line 1).
    pub auth_methods: Vec<String>,
}

impl Handshake {
    /// `protocol_version.as_u16()`, `agent_info` split, `serde_json::to_value(&agent_capabilities)`
    /// (`Value::Null` if that fails), `auth_methods.iter().map(|m| m.id().0.to_string())`.
    #[must_use] pub fn from_response(init: &InitializeResponse, at: DateTime<Utc>) -> Self;
}

/// Completes `initialize` over `io` and kills `io.child` on **every** exit path.
///
/// `io.child` is taken out before the task is spawned and held in a guard whose `Drop` calls
/// `Spawned::start_kill`, so even a caller that drops this future mid-await (an aborted probe
/// task at shutdown) leaves no process behind. The reader/writer move into the task with the
/// `InitializeRequest` built exactly as `session_main` builds it (`acp/mod.rs:799-803`:
/// `ProtocolVersion::V1`, `client::client_capabilities(&settings.client_capabilities)`,
/// `client::client_info()`); the foreground future sends the outcome on a `oneshot` and returns,
/// which ends `connect_with`.
///
/// # Errors
/// [`DriverError::Transport`]: `"initialize failed: {err}"` plus the child's stderr tail on a new
/// line when there is one (the `handshake_error` shape); `"the agent did not complete its
/// handshake within {n}s"` on timeout; `"the agent ended before answering initialize"` when the
/// connection actor failed first and dropped the foreground future (the milestone-3 CRITICAL,
/// `682a423`). In every case the child has been killed **and reaped** before this returns.
pub async fn handshake(io: AcpIo, settings: &AcpSettings, timeout: Duration) -> Result<Handshake>;
```

`acp/mod.rs` additions:

```rust
impl AcpIo {
    /// The pair `AcpDriver::start` builds (`:250-261`), factored so the probe spawns the same way.
    /// # Errors
    /// [`DriverError::Spawn`] when stdin or stdout was not piped.
    pub fn from_spawned(mut spawned: Spawned) -> Result<Self>;
}
```

`launch.rs` addition (P-9): `impl Spawned { /// Signals the tree without waiting; the sync half of kill_tree, for a Drop. pub fn start_kill(&mut self) -> Result<()>; }` — over `process_wrap::tokio::ChildWrapper::start_kill` (**UNVERIFIED — implementer must check** the trait method name in `process-wrap 10.0.0`; if absent, `Box::into_pin(self.child.kill())` polled once in a `futures::executor::block_on` is **not** acceptable inside a runtime — fall back to `tokio::process`'s `start_kill` through `inner()`).

### B.8 `crates/htui-core/src/model/agent.rs`

```rust
pub struct AgentBox {
    /* existing nine fields, unchanged order */
    /// `agent_box.probe` (`JSONB`, migration `0002`): the ANA-4 §4.6 snapshot, typed by
    /// `htui_agent::probe::ProbeSnapshot` (MOD-2 D44). Held as a [`Value`] for the reason
    /// [`Agent::launch`] gives; MOD-4's skip predicate reads `probe->>'status'` in SQL.
    #[serde(default)]
    pub probe: Option<Value>,
}
```

### B.9 `crates/htui-store/src/pg/{write,read}.rs`

`upsert_agent_box` SQL becomes:

```sql
INSERT INTO agent_box (agent_id, box_id, enabled, version, path, probed_at, quota, quota_at,
                       updated_at, probe)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
ON CONFLICT (agent_id, box_id) DO UPDATE SET
    enabled = EXCLUDED.enabled, version = EXCLUDED.version, path = EXCLUDED.path,
    probed_at = EXCLUDED.probed_at, quota = EXCLUDED.quota, quota_at = EXCLUDED.quota_at,
    probe = EXCLUDED.probe
```

with `$10 = row.probe.as_ref()` (`Option<&Value>`, as `quota` already binds). `agents()` adds `ab.probe AS "box_probe?"` to the select (`read.rs:540-562`) and `probe: row.box_probe` to the literal at `:587`. `.sqlx/` regenerates from inside `crates/htui-store` (README `:303-316`).

### B.10 `crates/htui-store/src/writer.rs` (P-7)

```rust
/// D35's refusal for the two registry writes, and D52's for a probe that has no server to write
/// to: the same sentence in both places, on purpose.
pub const REGISTRY_ON_SERVER_ONLY: &str = "the agent registry is written on the server only";
fn registry_writes_need_the_server() -> StoreError { StoreError::Unreachable(REGISTRY_ON_SERVER_ONLY.to_owned()) }
```

### B.11 `crates/htui/src/store_worker.rs`

```rust
pub enum StoreRequest {
    /* existing */
    /// Probe every enabled registry row on this box and write `agent_box` (MOD-2 D53, `R-AGT-6`).
    /// Served by the agent runtime's own task, never inside the loop: a probe spawns processes
    /// and may wait `HANDSHAKE_TIMEOUT` on each. Answered once, with [`StoreReply::Agents`] —
    /// the same reply the Settings section already renders — or [`StoreReply::Failed`].
    ProbeAgents,
}
// name(): Self::ProbeAgents => "probe_agents"
```

`try_serve`: `ProbeAgents` joins the four chat variants in the `"no agent runtime in this build"` arm (`:334-340`). Loop: `ProbeAgents` joins the arm at `:473-476`; `Served::Deferred => continue` already does the right thing. No new `StoreReply` variant.

### B.12 `crates/htui/src/agent_worker.rs`

```rust
/// A `agent_box` row older than this is re-probed (tier 2) when a chat starts on it (D55,
/// ANA-4:791-793: "before the first session of the day").
pub const PROBE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub struct AgentRuntime {
    /* existing */
    /// Probe tasks this runtime spawned and still owns: swept when finished, awaited by
    /// [`finish_background`](Self::finish_background), aborted by [`shutdown`](Self::shutdown).
    background: Vec<JoinHandle<()>>,
}

impl AgentRuntime {
    /// How many background tasks are still running (tests).
    #[must_use] pub fn background_len(&self) -> usize;

    /// Awaits every background task, each under `limit`; a task past it is aborted and named in a
    /// `warn!`. The harness's deterministic end (`Harness::drive_to_end`).
    pub async fn finish_background(&mut self, limit: Duration);

    /// **Now also** aborts every background task after the chats are down. A probe child dies
    /// with its guard (B.7); its reap is skipped, which at process exit is correct.
    pub async fn shutdown(&mut self, grace: Duration);

    /// `StoreRequest::ProbeAgents` (D52, D53), in this order and **before anything is spawned**:
    /// 1. `backend.writer()`; `None` → `Unreachable("this backend hands out no writer")`;
    ///    `Writer::Buffered(_)` → `Unreachable(REGISTRY_ON_SERVER_ONLY)` (D52).
    /// 2. `backend.box_info()?` → `NotFound { entity: "box", id: "this box is not registered" }`
    ///    when `None` (the `start()` message, `:345-349`).
    /// 3. `backend.agents()?`, `std::env::current_dir()`.
    /// 4. `tokio::spawn(run_probe(ProbeArgs { .. }))`, handle pushed to `background`,
    ///    `Ok(Served::Deferred)`.
    /// The `Err` is rendered by the caller as `Failed { request: "probe_agents", .. }`.
    async fn probe(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        addr: ReplyAddr,
    ) -> Result<Served, StoreError>;
}

/// Everything the probe task owns. Private, like `ChatArgs`.
struct ProbeArgs {
    writer: Writer,
    box_id: BoxId,
    agents: Vec<AgentSummary>,
    cwd: PathBuf,
    frames: Frames,          // the reply sender + the request's address, `:471-474`
}

/// The probe task: for every summary with `agent.enabled`, in `agents()` order,
/// `probe_agent(&agent, box_id, on_box.as_ref(), &ctx, &SpawnTier2::default())` with
/// `ctx = ProbeContext { env: ProbeEnv::host(cwd), now: Utc::now() }` — **one agent at a time**
/// (two adapters spawning at once on a laptop buys nothing and blurs whose stderr is whose).
/// `Row(row)` → `writer.upsert_agent_box(&row)`, and `summary.on_box = Some(row)` on success;
/// `Kept { reason }` → `info!`, `on_box` unchanged; a **write error** → one
/// `Failed { request: "probe_agents", message }` at `frames.addr` and return (the section
/// clears its in-flight state, the status line says what failed). Disabled registry rows are
/// skipped and their `on_box` left as read. Finally **one** `StoreReply::Agents(agents)` at
/// `frames.addr`, assembled from what the task itself wrote — not re-read — so the reply states
/// exactly what this probe did.
async fn run_probe(args: ProbeArgs);

/// D55: `on_box` is `None`, or `probed_at` is `None`, or `now - probed_at > PROBE_TTL`.
#[must_use] pub fn needs_reprobe(on_box: Option<&AgentBox>, now: DateTime<Utc>) -> bool;

/// The `ChatStart` re-probe (T28): `probe_agent` with `ProbeEnv::host(cwd).without_versions()`
/// ("tier 2 only": resolution is unavoidable, the `--version` children are skipped) and
/// `SpawnTier2::default()`; `Row` → `upsert_agent_box` (error → `warn!`), `Kept` → `debug!`.
/// Nothing here can fail the chat: it shares no channel with it.
async fn run_reprobe(writer: Writer, box_id: BoxId, agent: Agent, existing: Option<AgentBox>, cwd: PathBuf);
```

`serve` (`:215-283`): sweeps `background.retain(|h| !h.is_finished())` beside the `live` sweep; gains the arm `StoreRequest::ProbeAgents => match self.probe(backend, replies, addr).await { Ok(served) => served, Err(err) => Served::Reply(failed("probe_agents", &err)) }`.

`start()` (`:323-440`), after `self.started.push(chat.step_id)` and only when `summary.agent.transport == Transport::Acp && !matches!(writer, Writer::Buffered(_)) && needs_reprobe(summary.on_box.as_ref(), Utc::now())`: `self.background.push(tokio::spawn(run_reprobe(writer.clone(), box_id, summary.agent.clone(), summary.on_box.clone(), cwd.clone())))` — before `writer` moves into `ChatArgs`.

### B.13 `crates/htui/src/ui/tabs/settings/agents.rs` (D54)

```rust
/// What the `on this box` column reads while a probe this section asked for is in flight.
const PROBING: &str = "probing\u{2026}";

pub struct AgentsSection {
    agents: Vec<AgentSummary>,
    unavailable: Option<String>,
    /// `r` was pressed and no reply has come back. A second `r` is refused (D54).
    probing: bool,
}

// on_key:
//   KeyCode::Char('r') if !self.probing => { self.probing = true; ctx.request(StoreRequest::ProbeAgents); Handled::Consumed }
//   KeyCode::Char('r')                  => { ctx.emit(Action::Error("a probe is already running".to_owned())); Handled::Consumed }
//   _ => Handled::Pass
// on_reply:
//   Agents(rows)                                   => agents = rows, unavailable = None, probing = false
//   Failed { request: "agents", message }          => as today
//   Failed { request: "probe_agents", .. }         => probing = false   (the shell has already put the message on the status line, update.rs:132-134)
```

The `on this box` cell, in this order: `probing` → `probing…`; `on_box == None` → `not probed`; `probe["status"]` is `"missing"` | `"unauthenticated"` | `"failed"` → that word; otherwise (`"ready"`, or a pre-`0002` row with no `probe`) → `<version | —>` plus ` (off)` when `!enabled`. `r` is free: the global table binds `q`, `?`, `1`–`9`, `ctrl-c`, `-` (`keymap.rs:202-338`) and the tab consumes `h`/`l`/`[`/`]`/arrows (`settings/mod.rs:200-211`).

### B.14 `crates/htui/src/testkit.rs`

`drive` (`:161-165`): `StoreRequest::ProbeAgents` joins the four chat variants routed to `runtime.serve`. `drive_to_end` (`:266-278`): after `self.drive().await`, `runtime.finish_background(CHAT_END).await` **before** `runtime.shutdown(..)`, then the chat awaits, then the final `drive` — which is what delivers the probe's `Agents` reply.

---

## C. The `agent_box.probe` document, worked

The seeded `claude` row (`crates/htui-core/seeds/agent_claude.json`) probed on this box (Linux x86_64, `node` under volta, `claude` under mise), as `ProbeSnapshot::to_value()` writes it. Key order is struct order; `tools` and `resolved.env` are `BTreeMap`s, so alphabetical:

```json
{
  "transport": "acp",
  "resolved": {
    "command": "/home/mluigi/.volta/bin/node",
    "args": ["/home/mluigi/.volta/tools/image/node/22.19.0/lib/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js"],
    "env": { "CLAUDE_CODE_EXECUTABLE": "/home/mluigi/.local/share/mise/installs/claude/latest/claude" }
  },
  "tools": {
    "claude": "2.1.263",
    "claude_agent_acp": "0.48.0",
    "node": "22.19.0",
    "npx": "11.7.0"
  },
  "handshake": {
    "at": "2026-09-08T14:03:11.482913Z",
    "protocol_version": 1,
    "agent_name": "@agentclientprotocol/claude-agent-acp",
    "agent_version": "0.48.0",
    "capabilities": {
      "loadSession": true,
      "promptCapabilities": { "image": true, "audio": false, "embeddedContext": true },
      "mcpCapabilities": { "http": true, "sse": true },
      "sessionCapabilities": { "additionalDirectories": {}, "close": {}, "delete": {}, "fork": {}, "list": {}, "resume": {} },
      "auth": {},
      "_meta": { "claudeCode": { "promptQueueing": true } }
    },
    "auth_methods": []
  },
  "status": "ready",
  "stderr_tail": null,
  "source": "probe"
}
```

Where each value comes from: `resolved` = `launch::resolve` over `probe_tools().tool_map()` (`node` via `which_in`, `claude_agent_acp` via `<npm root -g>/@agentclientprotocol/claude-agent-acp/dist/index.js`, `claude` via `which_in`); `tools.node` = `extract_version("v22.19.0", "^v(\\d+\\.\\d+\\.\\d+)$")`, not below `22.0.0`; `tools.claude` = `extract_version("2.1.263 (Claude Code)", "^(\\d+\\.\\d+\\.\\d+) \\(Claude Code\\)$")`; `tools.npx` = `extract_version("11.7.0", "^(\\d+\\.\\d+\\.\\d+)$")`; `tools.claude_agent_acp` = `package_version(<npm root -g>/@agentclientprotocol/claude-agent-acp)`; `handshake` = `Handshake::from_response` over the response the fixture `tests/fixtures/claude_acp_handshake.jsonl` line 1 records (`capabilities` is the SDK's serialisation of `agentCapabilities` — tests assert `capabilities["loadSession"] == true` and nothing about the other keys, which the SDK owns); `auth_methods: []` → `status_for` → `ready`; `stderr_tail: null` because nothing failed.

The columnar row written beside it: `enabled = true`, `version = "0.48.0"` (the handshake value, ANA-4:765 — see H-6), `path = "/home/mluigi/.volta/bin/node"`, `probed_at = now`, `quota`/`quota_at` carried from the existing row (none today), `updated_at = now`.

The same row with `HTUI_TOOL_NODE=/nonexistent` (T26's second half): `resolved: null`, `tools: { "claude": "2.1.263", "claude_agent_acp": "0.48.0", "npx": "11.7.0" }` (no `node` key), `handshake: null`, `status: "missing"`, `stderr_tail: null`, `source: "probe"`; row `enabled = false`, `version = Some("2.1.263")` (no handshake, so the tool that shares the agent's name answers), `path = None`.

Serde that produces exactly this: `#[derive(Serialize, Deserialize)]` on `ProbeSnapshot` and `Handshake` with no `rename_all` (the keys are already snake_case and ANA-4's); `#[serde(default)]` on `source` only; `wire_enum!`'s per-variant `#[serde(rename = "...")]` for `status` / `source`; `Transport`'s `str_enum!` serde for `transport`; `ResolvedLaunch`'s new derive with its existing field order `command, args, env`; `Option<T>` fields serialise as `null` when `None` (no `skip_serializing_if` — ANA-4's example shows `"stderr_tail": null`, and MOD-4's `probe->>'status'` needs no key to be optional). `DateTime<Utc>` serialises RFC 3339 through `chrono`'s serde feature (already on).

---

## D. `crates/htui-store/migrations/0002_agent_probe.sql`, in full

```sql
-- 0002_agent_probe.sql - MOD-2 milestone 5 (plan D43): the ANA-4 §4.6 probe column, and ANA-5 §9's
-- column contracts and app_setting defaults folded in - "MOD-2 authors 0002 and MOD-2 is this
-- document's consumer, so there is zero sequencing risk" (docs/ANA-5.md 9). The file name stops
-- being a complete description of its contents; this header is the discoverability half of that.
--
-- Forward-only (R-STO-5): 0001_init.sql is never edited. ANA-2's 0003_orchestration.sql depends on
-- this file (docs/ANA-2.md 9); its cache-mirror migration is 0003 too, because
-- cache_migrations/0002_agent_mirror.sql is MOD-2 milestone 4's.

-- --------------------------------------------------------------------------------------------
-- 1. ANA-4 §9 (docs/ANA-4.md:1267-1273), as amended by MOD-2 plan D43.
--
-- agent_box.probe holds the §4.6 snapshot: {transport, resolved {command, args, env}, tools
-- {name: version}, handshake {at, protocol_version, agent_name, agent_version, capabilities,
-- auth_methods}, status (ready | unauthenticated | missing | failed), stderr_tail, source
-- (probe | manual)}. Typed by htui_agent::probe::ProbeSnapshot; htui-core and htui-store hold
-- it as JSON. MOD-4 reads probe->>'status' (docs/ANA-2.md 7).
--
-- ANA-4 wrote `COMMENT ON COLUMN agent.name IS NULL` to "fix the stale inline comment in 0001".
-- 0001 carries no database comment, so that statement clears nothing; the stale text is a
-- source-file comment (0001_init.sql:96) the forward-only rule forbids editing. This is the
-- comment ANA-4 meant, for the reader of `\d+ agent`.
-- --------------------------------------------------------------------------------------------

ALTER TABLE agent_box ADD COLUMN probe JSONB;

COMMENT ON COLUMN agent.name IS
  'unique registry name, e.g. claude or agy. Rows are seeded by htui, not by 0001_init.sql: '
  'htui_core::model::agent::seed_rows (crates/htui-core/seeds/*.json) inserted by '
  'PgStore::seed_if_empty_as when the table is empty. The inline comment in 0001_init.sql '
  'predates that and is stale (ANA-4 9 as amended by MOD-2 plan D43).';

-- --------------------------------------------------------------------------------------------
-- 2. ANA-5 §9: the five column contracts, verbatim (docs/ANA-5.md:2153-2181).
--
-- ANA-5 (prompt assembly). No DDL: run_step.prompt_digest and run_step.trim_record already
-- exist in 0001_init.sql. These are the documented contracts and the app_setting defaults.
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN prompt_template.body IS
  'ANA-5 4.1: {{name}} placeholders over a closed per-role set; {{{{ escapes a literal {{; no '
  'conditionals and no loops, because a section whose data is absent renders empty. Role is '
  'derived from name: judge and handoff are reserved, everything else is a phase template.';

COMMENT ON COLUMN prompt_template.name IS
  'phase name, or one of the reserved names judge and handoff (ANA-5 4.6); defaults are copied '
  'into each new project by the ANA-9 5.10 seed as amended by ANA-5';

COMMENT ON COLUMN run_step.prompt_digest IS
  'ANA-5 4.7: sha256, lowercase hex, over the canonical assembled prompt TEXT as sent - LF '
  'normalised, BOM stripped, one trailing LF, scrubbed before hashing. Not over the payload and '
  'not over sections[]. An audit field, never a replay key (ANA-2 4.9).';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1: {v, template, budget, budget_source, reserve, target, estimator, estimated_before, '
  'estimated_after, sections[], excerpts, notes}. Canonical; the prompt payload sections[] array '
  'is its abridged projection. Written at stage 3 by set_step_prompt, before the session starts.';

COMMENT ON COLUMN step_graph_phase.token_budget IS
  'ANA-5 4.4: phase, then project.settings.token_budget, then app_setting.token_budget; the '
  'assembler targets budget * (1 - app_setting.prompt_reserve_fraction)';

-- --------------------------------------------------------------------------------------------
-- 3. ANA-5 §9: the ten 5.3 defaults, verbatim (docs/ANA-5.md:2183-2189). Idempotent, so a re-run
--    and the MOD-6 seed agree.
-- --------------------------------------------------------------------------------------------

INSERT INTO app_setting (key, value) VALUES
  ('token_budget',                 '120000'::jsonb),
  ('prompt_reserve_fraction',      '0.10'::jsonb),
  ('prompt_upstream_hops',         '2'::jsonb),
  ('max_skill_tokens',             '20000'::jsonb),
  ('excerpt_max_files',            '12'::jsonb),
  ('excerpt_file_line_cap',        '400'::jsonb),
  ('excerpt_head_lines',           '200'::jsonb),
  ('excerpt_max_file_bytes',       '524288'::jsonb),
  ('excerpt_max_scan_files',       '20000'::jsonb),
  ('excerpt_provider_deadline_ms', '1500'::jsonb)
ON CONFLICT (key) DO NOTHING;
```

Two facts the implementer relies on: adjacent string constants separated by a newline are concatenated by PostgreSQL (the ANA-5 block depends on it, and so does the `agent.name` comment), and `'0.10'::jsonb` decodes through sqlx as `serde_json::Value::Number(0.1)`.

---

## E. Data flow and ownership

### E-1. `Settings > r` → `ProbeAgents` → task → `upsert_agent_box` × n → one `Agents`

```
AgentsSection::on_key('r')  probing = true; ctx.request(ProbeAgents)              [UI task; no spawn]
  └ App::drain(Origin::Tab(settings)) → App::dispatch → latest[(Tab(settings), ProbeAgents)] = seq
store_worker loop: envelope.request matches the runtime arm (:473-476)
  └ AgentRuntime::serve → AgentRuntime::probe(&backend, &tx, addr)                [worker task; still no spawn]
       1 backend.writer()  → Writer::Online(pg) | Writer::Memory(store)   ; Buffered → Err(Unreachable(REGISTRY_ON_SERVER_ONLY)) → Served::Reply(Failed{"probe_agents"})   (D52)
       2 backend.box_info()? → box_id                                        ; None → Err(NotFound{"box"})
       3 backend.agents()?  → Vec<AgentSummary>   ; std::env::current_dir()
       4 tokio::spawn(run_probe(ProbeArgs{ writer, box_id, agents, cwd, frames: Frames{ tx.clone(), addr } }))  → background.push(handle)
  └ Served::Deferred → continue                                                    [the loop is free: the next envelope is served immediately]
run_probe                                                                          [its own task]
  for summary in agents where agent.enabled:
    probe_agent(&agent, box_id, on_box.as_ref(), &ProbeContext{ env: ProbeEnv::host(cwd), now }, &SpawnTier2::default())
      ├ probe_tools: override(checked) | which_in (spawn_blocking) | node_modules + npm root -g | glob walk (spawn_blocking); capture_version per Path tool (launch::spawn, ≤15 s each)
      ├ incomplete → ProbeSnapshot{ status: missing, resolved: None }  (nothing spawned)     (D50)
      ├ launch::resolve + extra_args → ResolvedLaunch
      ├ acp && handshake → SpawnTier2: launch::spawn(&resolved, &cwd) → AcpIo::from_spawned → acp::handshake(io, &settings.acp, 60 s)
      │     └ Ok(h) → status_for(h): ready | unauthenticated ; Err(e) → failed, stderr_tail = e.lines()
      ├ missing && existing.source == manual → Kept                                                (D51)
      └ Row(agent_box_row(..))
    Row  → writer.upsert_agent_box(&row)  ok → summary.on_box = Some(row) ; Err → frames.reply(Failed{"probe_agents"}) ; return
    Kept → info!
  frames.reply(addr, StoreReply::Agents(agents))                                   [exactly one reply, at the request's seq/origin]
event_loop → App::update(Reply) → is_fresh(Tab(settings), seq) ✓ → SettingsTab::on_reply → AgentsSection::on_reply(Agents) → probing = false, rows replaced
render: `on this box` = 0.48.0 | missing | unauthenticated | failed | <version> (off)
```

Ownership at each hop, and why the task holds a `Writer` and not a `Backend`: the loop owns the one `Backend` (`store_worker.rs:1`, plan D4) and it is replaced wholesale in `go_online` / `go_offline` (`:579`, `:204`), so a task holding a clone would keep writing to a server the loop has already declared gone, and holding a `&Backend` across `tokio::spawn` is not `'static`. `Writer` is the owned handle built for exactly this (`writer.rs:1-10`: "a clone of a handle, never a copy of a store"), implements `WriteStore` (`:392`), and is what `run_chat` already takes for a session's life (`agent_worker.rs:338`, `:422`). `Vec<AgentSummary>` and `BoxId` move into the task; `Frames` clones the unbounded reply sender. Nothing in the task touches `AgentRuntime` or the loop afterwards; the `JoinHandle` is the runtime's only link, for `finish_background` / `shutdown`.

Locks: `ProbeEnv.vars` is a plain map; the child guard in `handshake` is a local; the only `Mutex` involved is `Spawned.stderr_tail`, taken and released inside `stderr_tail()` (`launch.rs:474-479`). No lock across an `.await`.

### E-2. `ChatStart` → staleness re-probe (T28)

```
AgentRuntime::start (existing) … summary = agents().find(agent_id) … self.live.insert … self.started.push
  if agent.transport == Acp && writer is not Buffered && needs_reprobe(summary.on_box.as_ref(), now):
     background.push(tokio::spawn(run_reprobe(writer.clone(), box_id, agent.clone(), on_box.clone(), cwd.clone())))
  Served::Start { step_id, task: run_chat(args) }        ← unchanged; the chat proceeds on tools::resolve as before
run_reprobe (its own task, its own child):  probe_agent(.., ProbeEnv::host(cwd).without_versions(), SpawnTier2::default()) → Row → writer.upsert_agent_box ; Kept → debug!
```

The chat never awaits the re-probe and the re-probe never answers a request: a `failed` re-probe writes `enabled = false, status = failed` and the running chat is untouched (D55). It is a second adapter process for up to 60 s beside the chat's own; the guard kills it on shutdown.

---

## F. Build order (T22 ∥ T24; T23 after T22; T25 after T24; T26 after T25; T27 after T23 **and** T25; T28 after T27)

| Pair | File-set intersection | Parallel? |
|---|---|---|
| T22 × T24, T22 × T25, T22 × T26 | ∅ (store crate × agent crate) | **yes** |
| T23 × T24, T23 × T25, T23 × T26 | ∅ — T23 touches `htui-core`, `htui-store`; T24–26 touch `htui-agent` and the root `Cargo.toml`/`Cargo.lock` (T24 only) | **yes** |
| T22 × T23 | ∅ by files; T23's `sqlx::query!` needs the column in `htui_sqlx` | serial (DB state, not files) |
| T24 × T25 | `probe.rs`, `tests/probe.rs`, `lib.rs`, `launch.rs` | serial |
| T25 × T26 | ∅ by files (`probe_live.rs` only) | serial by need (T26 calls `probe_agent`) |
| T26 × T27, T26 × T28 | ∅ | **yes** |
| T27 × T28 | `agent_worker.rs` | serial |
| T27 × T23 | ∅ — T27's `writer.rs` const vs T23's `tests/writer_buffered.rs` | n/a (T27 after T23 anyway) |

Every checkpoint: `cargo fmt --all -- --check` first; the Postgres line from the plan's Validation block for T22, T23, T28; `cd crates/htui-store && DATABASE_URL=…/htui_sqlx cargo sqlx migrate run --source migrations && cargo sqlx prepare -- --all-targets --all-features` after T22 lands and again after T23's query edits; `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings` after T24 and T25 (the walker's `cfg!(windows)` branch and `platform_key` compile there).

Commits, one per task: `feat(store): 0002_agent_probe — agent_box.probe, ANA-5 contracts and defaults` · `feat(core,store): AgentBox.probe through the three stores` · `feat(agent): probe tier 1 — resolve_tool, glob walker, version capture` · `feat(agent): probe tier 2 — acp::handshake and probe_agent` · `test(agent): criterion 9 live probe` · `feat(tui): ProbeAgents, the probe task and Settings > r` · `feat(tui): tier-2 re-probe of a stale agent_box at ChatStart`.

---

## G. Test plan, per task, TDD order (every case is written red before its code)

### G-T22 — `crates/htui-store/tests/migrations.rs` (Postgres-gated via `common::fresh_db()`)

1. `agent_box_gains_a_jsonb_probe_column` — `SELECT data_type FROM information_schema.columns WHERE table_name = 'agent_box' AND column_name = 'probe'` → `"jsonb"`; `is_nullable = 'YES'`.
2. `the_six_ana_comments_are_present_and_verbatim` — for each `(table, column, expected)` of the six (`agent.name`, `prompt_template.body`, `prompt_template.name`, `run_step.prompt_digest`, `run_step.trim_record`, `step_graph_phase.token_budget`): `SELECT pg_catalog.col_description(c.oid, a.attnum) FROM pg_class c JOIN pg_attribute a ON a.attrelid = c.oid WHERE c.relname = $1 AND a.attname = $2` equals the expected text **byte for byte** (the five ANA-5 strings copied from `docs/ANA-5.md:2153-2181` into the test as constants — the test is the guard against paraphrase); every other column of those tables has `NULL`.
3. `the_ten_ana5_defaults_land_with_their_values` — `SELECT key, value FROM app_setting WHERE key IN (…) ORDER BY key` → ten rows; `token_budget = 120000`, `prompt_reserve_fraction = 0.1` (`as_f64`), `prompt_upstream_hops = 2`, `max_skill_tokens = 20000`, `excerpt_max_files = 12`, `excerpt_file_line_cap = 400`, `excerpt_head_lines = 200`, `excerpt_max_file_bytes = 524288`, `excerpt_max_scan_files = 20000`, `excerpt_provider_deadline_ms = 1500`.
4. `a_second_run_is_a_no_op` (existing, `:100-113`) — now also asserts `count(app_setting)` unchanged across the second `apply_migrations` (the `ON CONFLICT DO NOTHING` half).
5. Existing `migrations_apply_on_a_clean_database` (`:44-98`): `applied == embedded` becomes `[1, 2]` by itself; `present.len() == TABLES.len() + 1` still holds (no new table). Existing seed test at `:300-304`: `2` → `12`, message "cache_refresh_seconds, cache_overlap_seconds and ANA-5's ten".

### G-T23 — `htui-core` unit, conformance, `pg_criteria.rs`, `writer_buffered.rs`

1. `conformance.rs::upsert_agent_box_by_pk` — insert with `probe: Some(json!({"status":"ready","source":"probe"}))`; update with `probe: None` (clears); orphan unchanged. Runs against `MemStore` (`htui-core` tests) and `PgStore` (`pg_conformance.rs`, `EXPECTED_CASES` unchanged).
2. `mem.rs::agents_join_this_box_only` (`:1192`) — the literal gains `probe: Some(json!({"status":"missing"}))`; asserts `claude.on_box.probe["status"] == "missing"` after the join; a second upsert with `probe: None` reads back `None`.
3. `pg_criteria.rs::a_probe_snapshot_round_trips_through_agent_box` — `demo_db()`; `upsert_agent_box` with a full `ProbeSnapshot`-shaped `json!` (hand-written — `htui-store` cannot name the type); `db.store.agents()` → `claude.on_box.unwrap().probe == Some(that value)`; `sqlx::query_scalar("SELECT probe->>'status' FROM agent_box WHERE agent_id = $1")` → `"ready"` (MOD-4's predicate, proven from SQL); second upsert with `probe: None` → `NULL`.
4. `writer_buffered.rs:261` — `probe: None`; the refusal assertion unchanged.
5. `cargo sqlx prepare --check` green.

### G-T24 — `crates/htui-agent/tests/probe.rs` (no Postgres, no real agent), plus `tools.rs` in-module

Fixture builders in the test file: `fn env(tmp: &Path) -> ProbeEnv` = `ProbeEnv { cwd: tmp.join("cwd"), platform: "linux-x86_64".into(), home: Some(tmp.join("home")), vars: BTreeMap from [("PATH", tmp/bin), ("LOCALAPPDATA", tmp/lad)], versions: true, version_timeout: 5 s }`; `fn executable(path, contents)` writes a file and, on unix, sets `0o755`; `fn touch_at(path, mtime)` uses `std::fs::File::set_modified` (stable since 1.75).

1. `the_platform_key_spells_macos_as_darwin` — `platform_key()` equals `format!("{}-{}", OS.replace("macos","darwin"), ARCH)` and is one of the five registry keys on this box.
2. `a_path_probe_resolves_through_which_in_over_the_injected_path` — `tmp/bin/htui-fake-tool` (`.exe` on Windows) → `resolve_tool(Path{names:["nope","htui-fake-tool"]}, &env)` → `Some` with that absolute path; `names: ["nope"]` → `None`.
3. `a_node_package_prefers_the_local_tree_and_reads_its_package_json_version` — moved from `tools.rs:294-321`, plus `package.json` `{"version":"0.48.0"}` → `version == Some("0.48.0")`; with `versions: false` → `None`.
4. `expand_replaces_percent_vars_and_tilde_and_skips_an_unset_var` — `"~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par"` → root `home/.local/share/htui/agents/antigravity-acp`, rest `["*", "agy_acp_server.par"]`; `"%LOCALAPPDATA%/JetBrains/*/acp-agents/antigravity-acp/*/agy_acp_server.exe"` → root `lad/JetBrains`; `"%NOPE%/x"` → `None`; `home: None` with `~` → `None`.
5. `segment_matches_is_star_within_one_name_only` — `*` vs `IntelliJIdea2026.1` ✓, `agy*.exe` vs `agy_acp_server.exe` ✓, `*` vs `a/b` ✗, `**` behaves as `*` (no recursion), exact match without `*`.
6. `the_jetbrains_two_star_shape_resolves_to_the_newest_install` — temp tree `lad/JetBrains/{IntelliJIdea2025.3,IntelliJIdea2026.1}/acp-agents/antigravity-acp/{20260501,20260818}/agy_acp_server.exe` with the 2025.3 file's mtime set **newer** → `glob_first` answers the 2025.3 path (mtime, not name, decides); equal mtimes → path-descending tiebreak → 2026.1.
7. `the_seeded_agy_row_resolves_by_glob_on_linux_with_the_uid_arg` — the real `agent_agy.json` `Discovery`; temp `home/.local/share/htui/agents/antigravity-acp/1.1.1/agy_acp_server.par`; `probe_tools` → `found["agy_acp_server"].args == ["--uid="]`, `version == None`; `agy` itself missing (not on the injected PATH) → `missing == ["agy"]`, `is_complete() == false` (criterion 10's non-live half, D56).
8. `the_same_row_on_darwin_appends_no_args` — `platform: "darwin-aarch64"`, tree under `home/Library/Application Support/…` → `args == []`.
9. `extract_version_handles_the_three_seed_patterns` — node `v22.19.0` → `22.19.0`; claude `2.1.263 (Claude Code)` → `2.1.263`; npx `11.7.0` → `11.7.0`; agy `^v?(\d+\.\d+\.\d+)` over `1.1.26` and `v1.1.26-rc1` → `1.1.26`; `garbage` → `None`; `(` (does not compile) → `None`; a second line matching after a first that does not → the second.
10. `below_min_is_semver_and_tolerant` — `("18.20.0","22.0.0") == true`, `("22.19.0","22.0.0") == false`, `("weird","22.0.0") == false`, `("1.0.0","also-weird") == false`.
11. `capture_version_runs_the_tool_and_times_out_a_hung_one` — `cargo` with `VersionProbe { args: ["--version"], pattern: "^cargo (\\d+\\.\\d+\\.\\d+)", min: None }` → `Some(_)` (every test box has `cargo`); `#[cfg(unix)]`: `tmp/bin/hang` = `#!/bin/sh\nsleep 30` with `version_timeout: 300 ms` → `None`, and the test returns in under 2 s.
12. `a_checked_override_pointing_nowhere_is_missing` — `vars["HTUI_TOOL_NODE"] = "/nonexistent/node"` → `probe_tools` → `missing == ["node"]`; pointing at `tmp/bin/htui-fake-tool` → found with that path.
13. `a_tool_below_its_floor_is_found_and_missing` — `tmp/bin/node` printing `v18.20.0`, `min: "22.0.0"` → `found["node"].below_min == true`, `versions()["node"] == Some("18.20.0")`, `missing == ["node"]`.
14. `tools.rs` in-module: `a_glob_probe_that_matches_nothing_is_unresolved` (renamed from `:279-291`, same assertion), `the_override_wins_over_every_tier` unchanged, `a_node_package_prefers_the_local_tree_over_the_global_root` unchanged (it now exercises the delegation).

### G-T25 — `tests/probe.rs` continued

Test-only helpers in the file: `async fn scripted_initialize(agent_end: DuplexStream, result: Value)` (reads one JSON-RPC line, answers `{"jsonrpc":"2.0","id":<id>,"result":result}`, then reads until EOF — 25 lines, raw JSON as `acp_conformance.rs` does); `struct CannedTier2(Result<Handshake>)` and `struct DuplexTier2(Value)` implementing `Tier2`.

1. `handshake_completes_initialize_and_reports_the_response` — duplex + `scripted_initialize` with the fixture's line-1 `result` → `Ok(h)`: `protocol_version == 1`, `agent_name == Some("@agentclientprotocol/claude-agent-acp")`, `agent_version == Some("0.48.0")`, `capabilities["loadSession"] == true`, `auth_methods.is_empty()`.
2. `handshake_reports_auth_methods_by_id` — `result` with `"authMethods": [{"id":"oauth-personal","name":"…"},{"id":"gemini-api-key","name":"…"}]` (ANA-4:710) → `auth_methods == ["oauth-personal","gemini-api-key"]`; `status_for(&h) == Unauthenticated`.
3. `handshake_times_out_and_kills_its_child` (`#[cfg(unix)]`) — `launch::spawn(ResolvedLaunch { command: "sleep", args: ["1000"], env: {} })` → `AcpIo::from_spawned` → `handshake(io, &settings, 300 ms)` → `Err(Transport)` containing `"within 0s"`, and `!Path::new(&format!("/proc/{pid}")).exists()` after 200 ms.
4. `handshake_kills_its_child_when_the_agent_speaks_garbage` (`#[cfg(unix)]`) — `sh -c 'echo not-json; sleep 1000'` → `Err(Transport)` naming `initialize`; pid gone.
5. `handshake_kills_its_child_on_success` (`#[cfg(unix)]`) — a `sh` script that reads one line, answers a `protocolVersion: 1` result for its id, then sleeps → `Ok(h)` with `protocol_version == 1`; pid gone.
6. `a_dropped_handshake_future_still_kills_the_child` (`#[cfg(unix)]`) — spawn `sleep 1000`, `tokio::spawn(handshake(..))`, `abort()` the handle after 100 ms → pid gone (the guard).
7. `probe_agent_is_missing_and_spawns_nothing_when_a_tool_is_absent` — seeded `claude` row, `env` with an empty `PATH` dir; `tier2 = CannedTier2(Err(..))` whose `handshake` **panics** if called → `Row` with `status == Missing`, `enabled == false`, `path == None`, `probe["resolved"].is_null()`, `probe["handshake"].is_null()`, `probed_at == Some(ctx.now)`.
8. `probe_agent_is_ready_on_an_empty_auth_list_and_unauthenticated_otherwise` — a synthetic `acp` row `{"command":"tmp/bin/htui-fake-tool"}` with no `discovery` (resolution trivially complete, `handshake` defaults true); `DuplexTier2(fixture result)` → `Ready`, `enabled == true`, `version == Some("0.48.0")`, `path == Some(<the fake tool path>)`; with the two-method result → `Unauthenticated`, `enabled == false`.
9. `probe_agent_is_failed_with_the_error_text_when_tier_2_errs` — `CannedTier2(Err(Transport("initialize failed: boom\nline 1\nline 2")))` → `Failed`, `probe["stderr_tail"] == ["initialize failed: boom","line 1","line 2"]`, `enabled == false`, `path` still `Some` (it resolved).
10. `a_cli_row_stops_at_tier_1` — the same synthetic row with `transport: cli` and a `Tier2` that panics → `Ready`, `handshake` null.
11. `a_manual_entry_survives_a_probe_that_finds_nothing_and_is_refreshed_by_one_that_does` — `existing = AgentBox { probe: Some(json!({"status":"ready","source":"manual", …})), probed_at: Some(old) }`; with the empty-PATH env → `Kept { .. }`; with the fake tool on PATH and the fixture tier 2 → `Row` whose `probe["source"] == "probe"` and `probed_at == Some(now)`.
12. `a_probe_carries_quota_over_and_orders_keys_as_ana4_does` — `existing.quota = Some(json!({"remaining":1}))`, `quota_at = Some(t)` → the row keeps both; `serde_json::to_string(&snapshot)` starts with `{"transport":` and its top-level key sequence is exactly `transport, resolved, tools, handshake, status, stderr_tail, source`.
13. `a_snapshot_round_trips_and_defaults_source_to_probe` — `from_row` over a row whose `probe` lacks `source` → `source == Probe`; over garbage → `None`.

### G-T26 — `crates/htui-agent/tests/probe_live.rs` (`#[ignore]`, run by hand)

1. `the_seeded_claude_row_probes_ready_with_a_v1_handshake` — `probe_agent(&claude_row(), BoxId::new(), None, &ProbeContext { env: ProbeEnv::host(manifest_dir), now }, &SpawnTier2::default())` → `Row` with `status == Ready`, `probe["handshake"]["protocol_version"] == 1`, `probe["tools"]["node"]` non-null, `enabled == true`; prints the snapshot (`--nocapture`, quoted in the milestone write-up); then a pid-safe survivor check — the child is not reachable from outside `handshake`, so this test asserts that no child of the **test process** survives 500 ms later (`ps -o pid= --ppid <test pid>`), never a pattern-only `pgrep` (**UNVERIFIED** — implementer picks the pid-safe form; `acp_live.rs:117-120` warns against the pattern form).
2. `the_same_row_without_node_is_missing_and_spawns_nothing` — `env.vars["HTUI_TOOL_NODE"] = "/nonexistent/node"`, `env.versions = true` → `Missing`, `enabled == false`, `probe["resolved"].is_null()`; `Tier2` is a panicking canned one (proof nothing was spawned).

### G-T27 — `crates/htui/tests/probe.rs` (CREATE), `tests/settings.rs`, `store_worker.rs`/`agent_worker.rs` in-module

**Fixture rule, stated at the top of every file**: no test outside `probe_live.rs` probes an unmodified seed row — this box has `node` and the adapter installed, and an unmodified `claude` row would spawn the real adapter inside `cargo test` (H-7). `fn unresolvable_registry() -> MemStore`: `MemStore::demo()`, then for each of its two agents `upsert_agent` with `launch.discovery.tools = { "gone": Path { names: ["htui-no-such-binary-2f8e"] } }` and `command: "${gone}"` (same ids and names, so the update lands).

1. `store_worker.rs`: `probe_agents_is_named_and_refused_without_a_runtime` — `StoreRequest::ProbeAgents.name() == "probe_agents"`; `serve(&Backend::memory(..), &ProbeAgents)` → `Failed { request: "probe_agents", message: "no agent runtime in this build" }`.
2. `agent_worker.rs`: `an_offline_backend_refuses_the_probe_before_spawning_anything` — `Backend::Offline` over a throwaway `CacheStore` → `runtime.serve(.., ProbeAgents)` → `Served::Reply(Failed { request: "probe_agents", message })` with `message.contains(REGISTRY_ON_SERVER_ONLY)`; `runtime.background_len() == 0`. Also `Backend::memory(MemStore::new())` (no box) → `Failed` containing `"not registered"`, `background_len() == 0`.
3. `agent_worker.rs`: `the_probe_task_answers_once_at_the_requests_address_with_agents` — `unresolvable_registry()`; `serve(ProbeAgents)` → `Deferred`, `background_len() == 1`; `finish_background(5 s)`; drain `rx` → exactly one envelope, `seq == 7`, `origin == Tab("settings")`, `Agents(rows)` where both rows have `on_box.probe["status"] == "missing"` and `enabled == false`; `store.agents()` shows the same two `agent_box` rows.
4. `store_worker.rs`: `the_loop_answers_other_requests_while_a_probe_is_in_flight` — spawned worker (`detached()` + `round_trip`) over `Started::detached(Backend::memory(unresolvable_registry()))` with `spawn_with(.., AgentRuntime::production())`; send `ProbeAgents` (seq 1) then `Workspaces` (seq 2); the **first** reply received is `Workspaces`; then `ProbeAgents`'s `Agents` arrives.
5. `tests/probe.rs` (harness): `r_in_the_agents_section_probes_and_the_column_reads_the_status` — `Harness::over(unresolvable_registry()).with_tab(SettingsTab::with_sections([AgentsSection]))`, `.with_agent_runtime(AgentRuntime::production())`; `settle`; `key("r")`; `render()` contains `probing…` **twice** (in-flight state, before any drive); `drive_to_end()`; `render()` contains `missing` twice and no `probing`; snapshot `settings__agents_probed_missing`.
6. `tests/probe.rs`: `a_second_r_while_probing_is_refused_on_the_status_line` — after `key("r")` (no drive), `key("r")` again → `harness.app().status` contains `"already running"`; `drive_to_end()` → exactly one `agent_box` row per agent (no double probe).
7. `tests/probe.rs`: `a_harness_without_a_runtime_clears_the_probing_state_on_the_refusal` — no runtime; `key("r")`, `settle()` → column back to `not probed`, status line `probe_agents: no agent runtime in this harness`.
8. `tests/settings.rs`: `the_status_column_renders_each_probe_outcome` — inject `Agents` with hand-built rows: `ready` + version `0.48.0` + enabled → `0.48.0`; `ready` + `enabled: false` → `0.48.0 (off)`; `missing` → `missing`; `unauthenticated` → `unauthenticated`; `failed` → `failed`; `on_box` with no `probe` key (pre-`0002` row) → old rule; snapshot `agents_probed`. Existing `agents_demo` / `agents_unknown_row` / `agents_empty` snapshots unchanged (no probe rows in the fixture; the test text `"nothing is probed until milestone 5"` at `:79` becomes `"no probe has run in the fixture"`).

### G-T28 — `crates/htui/tests/chat.rs`, `agent_worker.rs` in-module

1. `agent_worker.rs`: `needs_reprobe_is_absent_or_older_than_the_ttl` — `None` → true; `probed_at: None` → true; `now - 25 h` → true; `now - 23 h` → false.
2. `tests/chat.rs`: `a_chat_on_an_unprobed_row_refreshes_agent_box_in_the_background` — the existing streamed-turn script; after `drive_to_end()`, `store.agents()`'s scripted row has `on_box: Some` with `probe["status"] == "failed"` (the fake row's `command: "unused"` does not spawn — that is the point: a probe failure never touched the chat), `enabled == false`, `probed_at` within the last minute; the chat snapshot `chat__streamed_turn` **unchanged**.
3. `tests/chat.rs`: `a_fresh_row_is_not_re_probed_and_a_stale_one_is` — pre-insert `agent_box` for the scripted row with `probed_at = now`, `probe.status = ready`, `enabled = true` → after the chat, `probed_at` equal to what was inserted and `enabled` still true; repeat with `probed_at = now - 25 h` → `probed_at` moved, `status == failed`.
4. `tests/chat.rs`: `a_buffered_writer_never_re_probes` — reuse `chat_offline.rs`'s `Backend::Offline` construction: after the offline chat, `background_len() == 0` and the mirror holds no `agent_box` (the mirror never does — assert nothing panicked and `pending/` holds only the chat's file).

---

## H. Risks and findings the plan does not name

| # | Finding | Where | Consequence / mitigation |
|---|---|---|---|
| H-1 | `PgStore::schema_version()` is the **max embedded migration version** (`pg/mod.rs:387-389`), so landing `0002` makes it `2`; `CacheStore::open` compares it with `cache_meta.schema_version` and **rebuilds the mirror** on mismatch (MOD-6 plan D8, `cache/mod.rs:122-125`) | every box, first launch after this lands | Not a bug — the design's own rule — but a visible first-launch stall and a `pending/` directory that must survive the rebuild (**UNVERIFIED — implementer must check** that `rebuild()` deletes tables, not the directory; `cache/mod.rs:164` deletes rows of `MIRRORED_TABLES`). Recorded in HANDOFF |
| H-2 | `open_session`'s timeout arm (`acp/mod.rs:551-558`) calls `task.abort()`; the child lives in the task's `Arc<Mutex<Option<Spawned>>>` and `Spawned` has no `Drop` that kills, so a **handshake timeout orphans the adapter** — the very failure `682a423` fixed for the actor-failure path | pre-existing, milestone 3 | The probe's `handshake` owns the child in a `Drop`-killing guard (B.7). The same guard fixes `open_session` in four lines; **out of this plan's file set**, flagged for the `rust-reviewer` gate and milestone 6 |
| H-3 | `ToolMap` is `BTreeMap<String, String>` (`launch.rs:44`), so `tools::resolve` **cannot carry a glob tool's `args`**, and `AcpAdapter::build` ignores `on_box` (`acp/mod.rs:313`, `_on_box`) | `agy` on Linux via chat | The chat path would launch `agy_acp_server.par` without `--uid=`. Milestone 6 must build the driver from `probe.resolved` when `on_box` has a `ready` snapshot; stated in `tools.rs`'s doc and section I |
| H-4 | D50's "no configured credential" has no source (P-11): `SessionSpec.env` is empty until MOD-10 (`agent_worker.rs:396-398`); ANA-4:711's check is `<GEMINI_HOME>/antigravity-acp/acp_token.json`, an agent-specific fact | `agy` | An **authenticated** `agy` box reads `unauthenticated`, `enabled = false`, and MOD-4's predicate (`docs/ANA-2.md:1612`) skips it. Correct for criterion 10's stated half; milestone 6 owns the credential check. `claude`'s adapter reports `authMethods: []` so criterion 9 is unaffected |
| H-5 | A tool **below** `VersionProbe.min` (P-10) | `node 18` boxes | `missing`, nothing spawned, the found version still in `tools`. The reason is in the log only; ANA-4's shape has no `reason` key and D45 fixes the shape |
| H-6 | ANA-4:765 makes `agent_box.version` the **handshake** value, so the Settings column reads `0.48.0` (the adapter) for `claude`, while `tools.claude` holds `2.1.263` | Settings UX | Followed as written (design authority); if the maintainer prefers the agent's own CLI version in the column, it is a one-line change in `agent_box_row` and an ANA-4 amendment |
| H-7 | This box has `node`, `claude` and the adapter installed, so **any** test that probes an unmodified seed row spawns the real adapter for up to 60 s inside `cargo test` | every `htui` / `htui-agent` test | The fixture rule in G-T27 (`unresolvable_registry()`); `probe_live.rs` is the only file allowed to probe a seed row |
| H-8 | `which node` here answers a **volta shim** (`~/.volta/bin/node`), so `resolved.command` records the shim, not the binary | snapshot content | Harmless: the shim execs `node` and `--version` works through it; stated so a reviewer does not read it as a bug |
| H-9 | Existing assertion moves: `migrations.rs:300-304` (`app_setting == 2`); `agent.rs:157` doc; `settings.rs:79` test text | T22, T23, T27 | Listed in A and G |
| H-10 | Any `Agents` reply — a re-activation of the Settings tab, a scope change — clears `probing…` before the probe answers (D53's "no second reply arm" has this cost) | Settings UX | Accepted: the column reverts to the pre-probe rows for a moment and the probe's own reply supersedes them; noted in `agents.rs`'s doc |
| H-11 | `ProbeEnv::host` snapshots the whole environment on every `tools::resolve` call (every `ChatStart`) | perf | Microseconds; a `OnceLock` would freeze a `PATH` the user changed mid-session, which is worse |
| H-12 | `ResolvedLaunch`'s `env` lands in `JSONB` (P-6) | `R-SEC-2` | It is the row's `agent.launch.env` with placeholders substituted — paths, already in the database — never `SessionSpec.env` (`acp/mod.rs:246-249` extends it **after** resolution, and the probe never has a `SessionSpec`). The `RedactedEnv` `Debug` stays |
| H-13 | `process_wrap::tokio::ChildWrapper::start_kill` existence | B.7, P-9 | **UNVERIFIED — implementer must check**; fallback named in B.7 |
| H-14 | A seventh comment, `COMMENT ON COLUMN agent_box.probe`, would help the `\d+` reader ANA-4 wanted; D43 fixes six | D | Not added (settled decision); the file's section-1 header carries the shape instead. One line if the maintainer wants it |
| H-15 | Each `Settings > r` on this box spawns `node --version`, `claude --version`, `npx --version`, `npm root -g`, then the adapter: 3–5 s per agent, sequential | latency | Off the loop by construction (E-1); the `probing…` state is what the user sees |

---

## I. What this milestone does NOT touch

- **Milestone 6** (`agy` over ACP): the live half of criterion 10; the driver reading `agent_box.probe.resolved` instead of re-resolving (H-3); the `cli` fallback and the re-probe **on spawn failure** (D55's last sentence); the credential check behind `unauthenticated` (H-4).
- **MOD-16**: the real `%LOCALAPPDATA%\JetBrains\…` layout and the `.cmd` shim message (D56). The walker's `%VAR%` logic is proven on Linux with injected vars (G-T24 §6); the Windows **runtime** fact is not claimed.
- **MOD-7**: the on-registration probe hook (ANA-4:790) — `probe_agent` is the function it will call; no hook is added here.
- **Milestone 7 / MOD-7**: `agent_box.quota` / `quota_at` are carried through `agent_box_row`, never written.
- **MOD-10**: credentials; `SessionSpec` is never built by the probe.
- **The manual-entry editor**: nothing writes `source: manual`; D51's rule is honoured for rows that already carry it.
- **The mirror**: `agent_box` stays unmirrored (milestone 4 D31); offline `on_box` stays `None`; `Backend::agents()` is unchanged.
- **The store seam**: no `ReadStore` / `WriteStore` method is added (`pg_conformance.rs` `EXPECTED_CASES` unchanged); no `StoreReply` variant; no keymap entry; `event_loop.rs` unchanged.
- **`run_session` / `open_session`**: untouched, H-2 notwithstanding — the fix is proposed, not made.
