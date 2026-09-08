# Blueprint: MOD-2 milestone 6 — `agy` over ACP

**Plan**: `.claude/plans/mod-2-agy-acp.plan.md` (D57–D64, T29–T36 binding; its "Verified claims" table taken as read, re-checked against the tree at `fb626a8` on 2026-09-08).
**Design authority**: `docs/ANA-4.md` §4.5 (`:698-714`: four `authMethods`, `<GEMINI_HOME>/antigravity-acp/acp_token.json`, "installed-but-unauthenticated"), §4.6 (`:790-798`: self-update, "spawn fails already triggers a re-probe"), §5.3 (`:955-967`), §11.14 (`:1384-1392`). Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**. Anything not verified in source is marked **UNVERIFIED — implementer must check**.

Conventions inherited unchanged from the milestone-5 blueprint: `#![warn(missing_docs)]`, `unsafe_code = "forbid"` (no `set_var`; every environment-dependent unit takes an injected `ProbeEnv`), MSRV 1.98 (`Cargo.toml:7`), `agent-client-protocol = "=2.1.0"` (`:41`), one `thiserror` type per crate (no new `DriverError` variant, no new `DriverEvent` variant — D62), no lock across an `.await`, no spawn on the UI task or inside the worker's `select!` arm (`R-NF-3`), the H-7 fixture rule (only `probe_live.rs`, and now `agy_live.rs` / `chat_live_agy.rs`, may touch an unmodified seed row), and nothing keyed on `agent.name` (`R-AGT-5`; `tests/extensibility.rs:108-130` sweeps for `zeta`, and every `"agy"` literal below is a *test reading a seed row*, the same thing `probe_live.rs:43-48` does with `claude`).

---

## Plan ≠ tree, resolved in the tree's favour

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | T30: "with `HANDSHAKE_TIMEOUT` shortened" | `HANDSHAKE_TIMEOUT` is a `const` (`acp/mod.rs:94`); the only `SessionOptions` literal in the workspace is `acp/mod.rs:311` | `SessionOptions` gains `pub handshake_timeout: Duration`; `AcpDriver::start` fills it with `HANDSHAKE_TIMEOUT`, which stays the documented default. **No `test-support` gate and no `IoSource::Prepared` involvement**: `open_session` is already `pub` (`lib.rs:94`), `AcpIo`'s fields are `pub`, `launch::spawn` is `pub`, so the test builds the `AcpIo` by hand and never needs a driver |
| P-2 | D61: "`kill`/`stderr_tail` go through the guard's own methods" | `ChildGuard::kill_and_reap` is `async fn(&mut self)` (`launch.rs:623`) — it cannot be awaited under the `Mutex` | `kill` swaps the guard out under the lock (`std::mem::replace(&mut *lock, ChildGuard::new(None))`) and awaits `kill_and_reap` on the taken value. No new `ChildGuard` method; the lock is released at the end of the `replace` statement |
| P-3 | `ChildGuard`'s doc says "`open_session` hands its child to a task … its own orphan is a separate, deferred fix" and "Two callers hold one" (`launch.rs:583-591`) | This milestone is that fix | Doc rewrite: three holders (`handshake`, the probe's one-shots, `run_session`); the `Spawned`-has-no-`Drop` rationale stays (`:583-585`, still true: the guard, not the process type, carries the policy) |
| P-4 | T32 "independent of T30/T31" | T31 adds `ProbeSnapshot::recorded_launch` to `probe.rs` and its cases to `tests/probe.rs`; T32 owns both files | **T30 ∥ T32** holds; **T31 runs after T32** (as well as after T30) |
| P-5 | Test file list: `tests/{probe.rs,launch.rs,acp_map.rs}` | `tests/probe.rs` is ~1500 lines and neither `open_session` nor `AcpDriver` is the probe | **New** `crates/htui-agent/tests/acp_driver.rs` holds T30's session-timeout case and T31's driver-level cases; `tests/probe.rs` keeps the pure `recorded_launch` and credential cases |
| P-6 | Validation: `cargo test -p htui --features demo,test-support --test chat_live_agy` | `crates/htui/Cargo.toml:19-23` declares one feature, `testkit`; `chat_live.rs:7` runs with `--features testkit` | `cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture`, with `HTUI_KEEP_RAW_EVENTS=1` in front when recording the fixture (H-12) |
| P-7 | T35: a `Spawn` at chat start "schedules the same `run_reprobe`" | The failure surfaces inside `run_chat` (`agent_worker.rs:842-857`), a task with no handle to `AgentRuntime.background`; `run_reprobe` takes five positional args (`:712-718`) | `ChatArgs` carries an `Option<ReprobeArgs>`; `run_chat` awaits `run_reprobe` **inline, after the failure frame and after dropping its command receiver**. Off the worker arm by construction (production spawns `run_chat`), deterministic in the harness (`drive_to_end` awaits the chat future, `testkit.rs:272-277`). `run_reprobe` takes the struct; the staleness call site passes the same struct |
| P-8 | D64: "`settings.acp.model_config_id` gains the option id" | Nothing surfaces `session/new`'s `configOptions`: the banner's `raw` is the `initialize` response (`acp/mod.rs:867`) and `models` is `model_values`'s projection (`:850`, `:1444-1470`) | T33 gains a third, token-free case that drives `initialize` + `session/new` by hand (the `acp_live.rs:36-120` pattern) and prints `config_options()` verbatim. No production change |
| P-9 | T33: the unmodified `agy` row "records `unauthenticated`, `enabled = false`" | After T32 + D63 that holds only until the maintainer logs in for T34; the same test would then go red | T33 asserts the **consistency** — `status == ready` iff `probe.credential ∈ {file, env}` — and prints which case it observed. The "unauthenticated" half is asserted when the test's own check finds no candidate |
| P-10 | D59: `Discovery.credential` "`#[serde(default)]`" | `tests/launch.rs:146-149` round-trips `AgentLaunch` through `to_string`; `AgentLaunch.discovery` already uses `skip_serializing_if` (`launch.rs:76`) | `#[serde(default, skip_serializing_if = "Option::is_none")]`, so a row without the block re-serialises without a `"credential": null` key |
| P-11 | File table: `tests/launch.rs` not listed | `AGY_LAUNCH` (`tests/launch.rs:42-69`) is "ANA-4 §5.3 byte for byte" and pins the seed's shape | T32 amends the literal and `agy_launch_round_trips_with_per_platform_args`; T36 amends ANA-4 §5.3 (`:955-967`) to match |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-agent/src/acp/mod.rs` | UPDATE | T30 | `SessionOptions.handshake_timeout` (`:181-189`); `AcpDriver::start` fills it (`:311-315`); `open_session` uses it in both `timeout` calls and awaits the aborted task (`:568-575`, `:591`); `run_session` builds `Arc<Mutex<ChildGuard>>` (`:623`); `session_main` takes `&Mutex<ChildGuard>` (`:811`); `kill` / `stderr_tail` / `handshake_error` re-typed (`:1357-1390`). **Not**: any change to the turn loop, the cancel sequence, or the mapper |
| 2 | `crates/htui-agent/src/acp/mod.rs` | UPDATE | T31 | `IoSource::Spawn { launch, recorded: Option<ResolvedLaunch> }` (`:196-201`); `AcpDriver::from_row_with_probe`; `from_row` becomes its `None` case (`:240-252`); new `pub async fn launch_for(&self, &SessionSpec) -> Result<ResolvedLaunch>` holding the disk check; `io()` calls it (`:268-279`); `AcpAdapter::build` passes `on_box` (`:326-335`) |
| 3 | `crates/htui-agent/src/launch.rs` | UPDATE | T31 | `ChildGuard` doc `:580-593` (P-3). No code. *(Assigned to T31 rather than T30 so wave 1's file sets stay disjoint.)* |
| 4 | `crates/htui-agent/src/launch.rs` | UPDATE | T32 | `Discovery.credential: Option<CredentialProbe>` after `handshake` (`:91-100`); `pub struct CredentialProbe { env, files }` |
| 5 | `crates/htui-agent/src/acp/handshake.rs` | UPDATE | T31 | Module doc `:6-7`: "`run_session` keeps its child inside the task … " gains "in a guard of its own since milestone 6". Doc only |
| 6 | `crates/htui-agent/src/probe.rs` | UPDATE | T32 | `CredentialTier` (`wire_enum!`), `resolve_credential`, `ProbeSnapshot.credential` (after `handshake`, `:878-880`), `status_for(&Handshake, Option<CredentialTier>)` (`:919-925`), `snapshot_for` step 2 and the `blank` closure (`:1072-1172`) |
| 7 | `crates/htui-agent/src/probe.rs` | UPDATE | T31 | `ProbeSnapshot::recorded_launch(&self) -> Option<&ResolvedLaunch>` beside `from_row` (`:891-911`); `exists` (`:485-487`) becomes `pub(crate)` for `launch_for` |
| 8 | `crates/htui-agent/src/lib.rs` | UPDATE | T32 | `pub use launch::{…, CredentialProbe, …}` (`:108-112`); `pub use probe::{…, CredentialTier, resolve_credential, …}` (`:114-117`) |
| 9 | `crates/htui-agent/src/tools.rs` | UPDATE | T31 | Doc `:23-26`: "Milestone 6 builds the driver from `agent_box.probe.resolved`" → "This path is the **fallback** of `AcpDriver::launch_for`: a usable snapshot is spawned as recorded, and only a row with none — or one whose command is gone — resolves here, without the append". Doc only |
| 10 | `crates/htui-core/seeds/agent_agy.json` | UPDATE | T32 | `discovery.credential` block after `"handshake": true` (`:32`), section C |
| 11 | `crates/htui-core/seeds/agent_agy.json` | UPDATE | T34 | D64: `models`, `default_model`, `settings.acp.model_config_id` **only if** T33's third case learned them; otherwise untouched and the fact recorded |
| 12 | `crates/htui/src/agent_worker.rs` | UPDATE | T35 | `struct ReprobeArgs`; `run_reprobe(ReprobeArgs)` (`:712-747`); `ChatArgs.reprobe: Option<ReprobeArgs>` (`:573-587`); `start()` builds it once (`:525-546`); `run_chat`'s `Err` arm (`:842-857`); in-module tests (`:1265+`) |
| 13 | `crates/htui-agent/tests/acp_driver.rs` | CREATE | T30, T31 | G-T30, G-T31 (driver level); its own `assert_not_running` copy (`tests/probe.rs:997-1016`'s parse — the repo duplicates process helpers per file rather than sharing, `probe_live.rs:93-140`) |
| 14 | `crates/htui-agent/tests/probe.rs` | UPDATE | T31, T32 | T31: `recorded_launch` pure cases. T32: credential cases; `status_for(&found)` → `status_for(&found, None)` at `:1056` and the `handshake_reports_auth_methods_by_id` call; `a_probe_carries_quota_over_and_orders_keys_as_ana4_does` gains `credential` between `handshake` and `status` |
| 15 | `crates/htui-agent/tests/launch.rs` | UPDATE | T32 | `AGY_LAUNCH` literal gains the block (P-11); its round-trip asserts `credential.files.len() == 2`, `env == ["GEMINI_API_KEY"]`; `CLAUDE_LAUNCH` asserts `credential.is_none()` |
| 16 | `crates/htui-agent/tests/agy_live.rs` | CREATE | T33 | G-T33 |
| 17 | `crates/htui/tests/chat_live_agy.rs` | CREATE | T34 | G-T34 |
| 18 | `crates/htui-agent/tests/fixtures/agy_acp_handshake.json`, `agy_acp_turn.jsonl` | CREATE | T33, T34 | Recorded: the `Handshake` document T33 prints; the raw `session/update` lines T34 captures (same `{method, params}` line shape as `claude_acp_turn.jsonl`, `acp_map.rs:18-32`). Redacted by hand before commit (session ids, paths under `$HOME`) |
| 19 | `crates/htui-agent/tests/acp_map.rs`, `tests/snapshots/` | UPDATE | T34 | `a_recorded_agy_turn_maps_to_the_rows_it_did_when_it_was_captured` over fixture 18, `insta` snapshot; a mapper edit only under D62's rule |
| 20 | `README.md`, `HANDOFF.md:74`, PRD `:155`, `docs/ANA-4.md` §5.3 + §11.14 | UPDATE (docs) | T36 | README: a subsection between the env-knob table (`:227-232`) and `## Platforms` (`:234`) — the D57 URL, unzip dir, `chmod +x`, the glob, the vendor login, `HTUI_TOOL_AGY_ACP_SERVER`; HANDOFF phase-6 note; PRD row `complete`; ANA-4 §5.3 gains the `credential` key, §11.14's six `agy` lines gain their answers |

No manifest changes: `tokio`'s `fs` feature is already on (`htui-agent/Cargo.toml:24`), and no new dependency is needed.

---

## B. Interfaces, exactly

### B.1 T30 — `acp/mod.rs` (D61)

```rust
pub struct SessionOptions {
    pub agent_name: String,
    pub settings: AgentSettings,
    pub stamp: Stamp,
    /// How long `initialize` + `session/new` may take before [`open_session`] gives up.
    /// [`HANDSHAKE_TIMEOUT`] in production; a test that proves the timeout arm kills its child
    /// sets milliseconds here rather than waiting a minute.
    pub handshake_timeout: Duration,
}
```

`open_session` (`:548-598`), the two arms that change:

```rust
let timeout = options.handshake_timeout;
let task = tokio::spawn(run_session(/* unchanged */));
match tokio::time::timeout(timeout, ready_rx).await {
    Err(_) => {
        task.abort();
        // D61: the aborted task drops its `ChildGuard`, whose `Drop` signals the kill. Awaiting
        // the cancelled handle is *not* waiting for the adapter — it resolves as soon as the
        // runtime has dropped the future — and it is what makes this `Err` mean "the kill has
        // been sent" rather than "the kill will be sent shortly". The reap is skipped: the
        // zombie is the same one the probe's abort path leaves (`launch.rs:651-652`).
        let _ = task.await;
        Err(DriverError::Transport(format!(
            "the agent did not complete its handshake within {}s", timeout.as_secs()
        )))
    }
    // …
    Ok(Ok(Err(err))) => { let _ = tokio::time::timeout(timeout, task).await; Err(err) }
    // …
}
```

`run_session` (`:623`): `let child = Arc::new(Mutex::new(ChildGuard::new(child)));` — the ownership comment at `:617-622` stays word for word and gains one sentence: "and an **aborted** task drops the guard, whose `Drop` signals the kill (D61)". `session_main` (`:811`): `child: &Mutex<ChildGuard>`. The three helpers:

```rust
/// Kills the process tree and reaps it, if there is one and nobody has yet. The guard is
/// **swapped out** under the lock and awaited outside it, so no lock is held across an `.await`;
/// a second call finds an empty guard and returns.
async fn kill(child: &Mutex<ChildGuard>) {
    let mut taken = std::mem::replace(
        &mut *child.lock().unwrap_or_else(PoisonError::into_inner),
        ChildGuard::new(None),
    );
    taken.kill_and_reap().await;   // logs its own two warnings; the `Drop` after it is a no-op
}

fn stderr_tail(child: &Mutex<ChildGuard>) -> String {
    child.lock().unwrap_or_else(PoisonError::into_inner).stderr_tail().join("\n")
}

fn handshake_error(step: &str, err: &agent_client_protocol::Error, child: &Mutex<ChildGuard>) -> DriverError; // body unchanged
```

`Arc<Mutex<ChildGuard>>` has exactly the holders the old `Arc<Mutex<Option<Spawned>>>` had — `run_session`'s local and the `connect_with` closure's clone (`:656`) — both inside the task's future, so an abort drops the last reference. `Send` bounds are unchanged (`ChildGuard` wraps the same `Spawned`).

### B.2 T31 — `acp/mod.rs` and `probe.rs` (D58)

```rust
enum IoSource {
    Spawn {
        launch: Box<AgentLaunch>,
        /// `agent_box.probe.resolved` when the snapshot passed D58's three row-side rules
        /// ([`ProbeSnapshot::recorded_launch`]). The fourth rule — the command still exists —
        /// is I/O and is applied per session by [`AcpDriver::launch_for`].
        recorded: Option<ResolvedLaunch>,
    },
    #[cfg(feature = "test-support")]
    Prepared(Mutex<Option<Box<AcpIo>>>),
}

impl AcpDriver {
    /// [`from_row`](Self::from_row) plus this box's row: a usable snapshot is what `start`
    /// spawns, `--uid=` and all (H-3 of the milestone-5 blueprint). A `probe` column that does
    /// not parse, a `manual` row, or a `missing`/`failed` status all mean "resolve as before".
    pub fn from_row_with_probe(agent: &AgentRow, on_box: Option<&AgentBox>, caps: DriverCaps) -> Result<Self>;
    // from_row(agent, caps) == from_row_with_probe(agent, None, caps)

    /// What this session would spawn, before spawning it.
    ///
    /// The recorded launch when there is one **and** its `command` still exists
    /// (`tokio::fs::metadata` — filesystem I/O on the session's own task, never the worker loop);
    /// otherwise the row's tools are resolved now (`tools::resolve` → `launch::resolve`), which is
    /// the pre-milestone-6 path and carries no platform `args`. Either way `spec.env` is applied
    /// **last** and wins (`R-SEC-2`): the row's environment holds paths, the spec's holds secrets.
    ///
    /// # Errors
    /// [`DriverError::Unresolved`] / [`DriverError::Transport`] as `tools::resolve` and
    /// `launch::resolve`; `Transport` for a prepared transport, which spawns nothing.
    pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch> {
        let IoSource::Spawn { launch, recorded } = &self.io else {
            return Err(DriverError::Transport("a prepared transport spawns nothing".to_owned()));
        };
        let mut resolved = match recorded {
            Some(recorded) if crate::probe::exists(Path::new(&recorded.command)).await => recorded.clone(),
            Some(recorded) => {
                tracing::info!(command = %recorded.command, "the probe's recorded command is gone; resolving again");
                resolve_now(launch, &spec.cwd).await?
            }
            None => resolve_now(launch, &spec.cwd).await?,
        };
        resolved.env.extend(spec.env.clone());
        Ok(resolved)
    }
}

/// `tools::resolve` then `launch::resolve`: the two lines `io()` held at `:271-272`.
async fn resolve_now(launch: &AgentLaunch, cwd: &Path) -> Result<ResolvedLaunch>;
```

`io()`'s `Spawn` arm becomes `let resolved = self.launch_for(spec).await?; AcpIo::from_spawned(crate::launch::spawn(&resolved, &spec.cwd).await?)`. `AcpAdapter::build`: `Ok(Box::new(AcpDriver::from_row_with_probe(agent, on_box, caps)?))`. `AcpDriver`'s `Debug` (`:221-229`) gains `.field("recorded", &matches!(self.io, IoSource::Spawn { recorded: Some(_), .. }))`.

`probe.rs`:

```rust
impl ProbeSnapshot {
    /// D58's three row-side rules: `resolved` is `Some`, `source` is `probe` (a `manual` row is a
    /// human's path and `launch::resolve` honours it through the row itself), and `status` is
    /// `ready` or `unauthenticated` (an unauthenticated box recorded the right launch; the
    /// vendor's own auth error is more use than a second resolution). `missing` has nothing to
    /// spawn and `failed` recorded a launch that did not answer. The fourth rule — the command
    /// still exists on disk — is I/O and belongs to the caller.
    #[must_use]
    pub fn recorded_launch(&self) -> Option<&ResolvedLaunch> {
        if self.source != ProbeSource::Probe { return None; }
        if !matches!(self.status, ProbeStatus::Ready | ProbeStatus::Unauthenticated) { return None; }
        self.resolved.as_ref()
    }
}
pub(crate) async fn exists(path: &Path) -> bool;   // `:485-487`, visibility only
```

What `AgentBox.probe` deserialises into and where: `ProbeSnapshot::from_row(&AgentBox)` (`probe.rs:898-900`, already `pub`, already tolerant — `None` on `NULL` or on a document that does not parse), called once in `from_row_with_probe`. `htui-core` is untouched.

### B.3 T32 — `launch.rs` and `probe.rs` (D59)

```rust
pub struct Discovery {
    #[serde(default)] pub tools: BTreeMap<String, ToolProbe>,
    #[serde(default = "default_true")] pub handshake: bool,
    /// Where a credential the agent's **own** auth flow leaves on this box may be found (plan
    /// D59). Names and paths only, never values; a row without it keeps milestone 5's rule
    /// (a non-empty `authMethods` is `unauthenticated`, full stop).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialProbe>,
}

/// `agent.launch.discovery.credential` (D59).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CredentialProbe {
    /// Environment variables whose presence — set and non-empty — counts, as the shell names them.
    pub env: Vec<String>,
    /// File candidates in the glob tier's expander grammar (`probe::expand`): `%VAR%` anywhere, a
    /// leading `~`, an unset variable skips the pattern. In order; the first that exists wins.
    /// Checked **before** `env` (plan D63: the token file is the tier that fires on a
    /// subscription box).
    pub files: Vec<String>,
}
```

`probe.rs`:

```rust
wire_enum!(
    /// `probe.credential` (D59): which tier of `discovery.credential` answered. The value — a
    /// token's text, a variable's contents — is never read into memory by the probe and never
    /// recorded; the file tier is a `stat`.
    CredentialTier {
        /// A declared file exists.
        File => "file",
        /// A declared variable is set and non-empty.
        Env => "env",
        /// The row declares candidates and none answered.
        Absent => "absent",
    }
);

/// D59: files first (through [`glob_first`], so the grammar, the `spawn_blocking` walk and the
/// "unset variable skips the pattern" rule are the walker's own — a literal path is a pattern
/// with no `*`), then variables. Spawns nothing. `Ok(None)` when `probe` is `None`.
///
/// # Errors
/// [`DriverError::Transport`] when the file walk could not be scheduled — the same rule as
/// `probe_tools` (milestone-5 P-1: a runtime fault must not read as "no credential").
pub async fn resolve_credential(probe: Option<&CredentialProbe>, env: &ProbeEnv) -> Result<Option<CredentialTier>>;

/// Tier 2's outcome mapped onto D50 as amended by D59: an empty `auth_methods` is `ready`
/// whatever `credential` says; a non-empty one is `ready` on `Some(File | Env)` and
/// `unauthenticated` on `None` (no block declared: milestone 5's rule) or `Some(Absent)`.
#[must_use]
pub fn status_for(handshake: &Handshake, credential: Option<CredentialTier>) -> ProbeStatus;

pub struct ProbeSnapshot {
    pub transport: Transport,
    pub resolved: Option<ResolvedLaunch>,
    pub tools: BTreeMap<String, Option<String>>,
    pub handshake: Option<Handshake>,
    /// D59: which tier answered, never the value. `None` when the row declares no block, or when
    /// the probe ended before `agent.launch` parsed. Absent in a pre-milestone-6 row reads as
    /// `None`, which is the milestone-5 rule that row was written under.
    #[serde(default)]
    pub credential: Option<CredentialTier>,
    pub status: ProbeStatus,
    pub stderr_tail: Option<Vec<String>>,
    #[serde(default)] pub source: ProbeSource,
}
```

Where the check runs in `probe_agent`'s ordered steps (doc `:1021-1041`): **step 2, immediately after `agent.launch` parses and before `probe_tools`** — it needs only `launch.discovery.credential` and `ctx.env`, it is a `stat` and a map lookup, and running it first means every snapshot that has a parsed launch records it (a `missing` box still says whether it holds a token). `snapshot_for`'s `blank` closure (`:1073-1082`) takes the value; a `Transport` error here is `failed` with its text, exactly as `probe_tools`'s is (`:1098-1107`). Step 6 becomes `status_for(&handshake, credential)`. The log line is `debug!(tier = %tier, "a credential tier answered")` — the tier, not the path.

### B.4 T35 — `agent_worker.rs` (D60)

```rust
/// What a re-probe needs, whichever trigger asks for it: the staleness check at `start` (plan
/// D55) and a spawn failure inside `run_chat` (plan D60).
#[derive(Clone)]
struct ReprobeArgs { writer: Writer, box_id: BoxId, agent: Agent, existing: Option<AgentBox>, cwd: PathBuf }
// hand-written `Debug` (writer.label(), agent.name, existing.is_some())

async fn run_reprobe(args: ReprobeArgs);   // body of `:712-747`, destructured

pub struct ChatArgs {
    /* existing */
    /// D60: `Some` when a spawn failure should refresh this box's row; `None` for a `cli` row
    /// (tier 2 is `initialize`), a buffered writer (`upsert_agent_box` is refused, D52), or a
    /// chat whose staleness re-probe is already running — a second one would race it for the
    /// same row.
    reprobe: Option<ReprobeArgs>,
}
```

`start()` (`:525-546`) builds `reprobe` once from the three existing conditions minus staleness, spawns `run_reprobe(reprobe.clone())` into `background` when `needs_reprobe` is true — exactly today — and sets `ChatArgs.reprobe = if stale { None } else { reprobe }`. `run_chat`'s `Err` arm (`:842-857`):

```rust
Err(err) => {
    let message = err.to_string();
    frames.reply(&start_addr, StoreReply::Failed { request: "chat_start", message: message.clone() });
    close_run(&writer, &chat, RunStatus::Failed).await;
    frames.failed(message);
    // D60. A spawn failure is a fact about this box's row, not about the conversation: the row
    // is refreshed here, in this task's remaining life, off the worker's arm (`R-NF-3`). The
    // command receiver goes first, so a command sent meanwhile is refused as "this chat has
    // ended" rather than queued for nobody. No transport fallback: a CLI agent is its own row.
    if let (DriverError::Spawn(_), Some(reprobe)) = (&err, reprobe) {
        drop(commands);
        run_reprobe(reprobe).await;
    }
    return;
}
```

The adapter's message is already surfaced twice (`Failed { request: "chat_start" }` and `ChatFrame::Failed`); the test pins it because D60 names it.

---

## C. The `agy` documents, worked

The seed edit (`agent_agy.json:11-33`, after `"handshake": true`):

```json
      "handshake": true,
      "credential": {
        "env":   ["GEMINI_API_KEY"],
        "files": ["%GEMINI_HOME%/antigravity-acp/acp_token.json",
                  "~/.gemini/antigravity-acp/acp_token.json"]
      }
```

`expand` (`probe.rs:507-537`) on the first candidate with `GEMINI_HOME` unset returns `None` → skipped; on the second, root = `<home>/.gemini/antigravity-acp/acp_token.json`, segments `[]`; `walk` (`:619-656`) answers `[root]` filtered by `is_file()`; `glob_first` (`:682-697`) returns it on the blocking pool. `%GEMINI_HOME%` set to a directory expands the same way. No `*` anywhere, and the walker does not care.

`agent_box.probe` for the seed `agy` row on this box after T29, **before** the login (the T33 case), key order = struct order:

```json
{
  "transport": "acp",
  "resolved": {
    "command": "/home/mluigi/.local/share/htui/agents/antigravity-acp/1.1.1/agy_acp_server.par",
    "args": ["--uid="],
    "env": {}
  },
  "tools": { "agy": "1.1.27", "agy_acp_server": null },
  "handshake": {
    "at": "…", "protocol_version": 1,
    "agent_name": "antigravity-acp", "agent_version": "agy_acp_server_…",
    "capabilities": { "…": "as the SDK serialises it" },
    "auth_methods": ["oauth-personal", "oauth-business", "gemini-api-key", "agent-platform"]
  },
  "credential": "absent",
  "status": "unauthenticated",
  "stderr_tail": null,
  "source": "probe"
}
```

Columnar: `enabled = false`, `version = "agy_acp_server_…"` (the handshake's, ANA-4:766), `path = …/agy_acp_server.par`. After the login the same probe reads `"credential": "file"`, `"status": "ready"`, `enabled = true` — and `recorded_launch()` answers `resolved` in both cases, so a chat started on either row spawns `agy_acp_server.par --uid=` through `launch_for` (E-1). `agent_name`/`agent_version`/`capabilities` above are ANA-4 §4.5's report (`:698-702`), **UNVERIFIED on this box until T33 runs** — the fixture `agy_acp_handshake.json` is what T33 prints.

---

## D. Data flow and ownership

### E-1. `ChatStart` on a probed `agy` row (T31)

```
AgentRuntime::start  (worker loop; no spawn, no fs I/O)
  backend.agents() → summary { agent, on_box: Some(row) }
  factory.driver_for(&agent, on_box)                                     registry.rs:93-108
    └ AcpAdapter::build(agent, on_box, caps)
        └ AcpDriver::from_row_with_probe: launch = agent.launch parsed;
          recorded = on_box.and_then(ProbeSnapshot::from_row).and_then(|s| s.recorded_launch().cloned())
  ChatArgs { driver, spec { env: {} until MOD-10 }, reprobe, … } → Served::Start
run_chat  (its own task)
  driver.start(spec, prompt) → AcpDriver::io(&spec)
    └ launch_for(&spec):  recorded Some + tokio::fs::metadata(command) ok  → recorded.clone()
                          recorded Some + gone                             → info!; tools::resolve → launch::resolve   (no --uid=)
                          recorded None                                    → tools::resolve → launch::resolve
       resolved.env.extend(spec.env)                                        (spec wins on a key both hold)
    └ launch::spawn(&resolved, &spec.cwd)  → which → process group → AcpIo::from_spawned
  open_session(io, spec, prompt, SessionOptions { handshake_timeout: 60 s, .. })
```

The disk check sits on the session's task because `tools::resolve` already puts its `which` and glob walks there (`spawn_blocking` inside `probe.rs:416-427`, `:690-696`), and because a stale path must degrade *into* resolution on the same code path, not fail a request on the loop.

### E-2. Handshake timeout (T30)

```
open_session: timeout(handshake_timeout, ready_rx) → Err
  task.abort()  → the runtime drops run_session's future
                   → drops `connected` (the connect_with future, holding the closure's Arc clone)
                   → drops the local Arc → refcount 0 → Mutex<ChildGuard> dropped
                   → ChildGuard::drop → Spawned::start_kill (SIGKILL to the group / job object)
  task.await    → Err(Cancelled) once the drop has happened
  Err(Transport("… within 60s"))                      the child is signalled; reaped at process exit
```

Every other exit of `run_session` still ends in `kill(&child).await` (`:671`), which empties the guard first, so the `Drop` that follows is a no-op — no double signal, no spurious "survived its guard" warning.

### E-3. Spawn failure at chat start (T35)

```
run_chat: driver.start → Err(DriverError::Spawn(msg))          e.g. "`…/agy_acp_server.par` is not executable: …"  (launch.rs:689)
  Failed { chat_start, msg } at the request's address  →  close_run(Failed)  →  ChatFrame::Failed { msg }
  reprobe.is_some() && err is Spawn:
    drop(commands)                                     the runtime's next `serve` sweeps the LiveChat entry (:265)
    run_reprobe(ReprobeArgs).await                     probe_agent(.., without_versions(), SpawnTier2) → Row → writer.upsert_agent_box
  return                                               the task ends; nothing waited on it
```

`Unresolved` and `Transport` do not trigger it (D60 names `Spawn`; H-8).

---

## E. Build order and file-set intersections

Order: **T29 ∥ T30 ∥ T32 ∥ T35 → T31 (after T30 and T32) → T33 (after T29, T32) → T34 (after T31, T33, and the maintainer's login) → T36.**

| Pair | File-set intersection | Parallel? |
|---|---|---|
| T30 × T32 | ∅ — T30: `acp/mod.rs`, `tests/acp_driver.rs`; T32: `launch.rs`, `probe.rs`, `lib.rs`, seed, `tests/probe.rs`, `tests/launch.rs`. *(The `launch.rs`/`handshake.rs` doc edits P-3 and A.5 moved to T31 to keep this empty.)* | **yes** |
| T30 × T35, T32 × T35 | ∅ (`agent_worker.rs` only) | **yes** |
| T30 × T31 | `acp/mod.rs`, `tests/acp_driver.rs` | serial |
| T31 × T32 | `probe.rs`, `tests/probe.rs` | serial (P-4) |
| T31 × T35 | ∅ | **yes** by files; T35's test uses a literal launch so its outcome is the same before and after T31 |
| T33 × T31, T33 × T35 | ∅ (`agy_live.rs` only) | **yes** by files; T33 needs T32's `credential` key |
| T34 × everything | `agent_agy.json` (D64, after T32's edit), `acp_map.rs`, fixtures | serial by need (D58 is what makes the live chat launch with `--uid=`) |

Checkpoints: `cargo fmt --all -- --check` first; the plan's clippy pair after T30, T31, T32 (`ChildGuard`, `CredentialProbe` and `launch_for` all compile on the Windows target — there is no `cfg` in any of them); the Postgres line after T35 (`agent_worker.rs` tests). Commits, one per task: `fix(agent): open_session kills its child on a handshake timeout (D61)` · `feat(agent): declarative credential tier behind unauthenticated (D59)` · `feat(agent): the driver spawns agent_box.probe.resolved (D58)` · `feat(tui): re-probe the row when a chat fails to spawn (D60)` · `test(agent): criterion 10 live agy probe` · `test(tui,agent): live agy chat, recorded transcript and the §11.14 answers` · `docs(mod-2): milestone 6 close-out`.

---

## F. Test plan, per task, TDD order

### G-T30 — `crates/htui-agent/tests/acp_driver.rs` (CREATE), `#[cfg(unix)]`

1. `open_session_times_out_and_kills_its_child` — `launch::spawn(&ResolvedLaunch { command: "sleep", args: ["1000"], env: {} }, tmp)` (a system binary: no `ETXTBSY` window, so no `spawn_fixture` retry is needed); `pid = spawned.pid()`. `let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES); let (reader, writer) = tokio::io::split(client_end);` and **hold `agent_end`** for the test's life so the client's reader never sees EOF (an EOF is the `Ok(Err(_))` arm, not the timeout). `AcpIo { reader, writer, child: Some(spawned) }`; `SessionSpec` as `tests/extensibility.rs:364-378` over `tmp`; `SessionOptions { agent_name: "fixture", settings: default, stamp: Stamp::Wall, handshake_timeout: 300 ms }`. `open_session(..).await` → `Err(DriverError::Transport(m))` with `m.contains("did not complete its handshake")`; then `assert_not_running(pid, "the session's child")`. **Red before D61 for the stated reason**: `sleep` is in state `S` — "pid … is still running (state S)". Green after: state `Z` (signalled, unreaped). Because `open_session` awaited the aborted task, no polling loop is needed; if the implementer keeps a pure `abort()`, the assertion polls ≤ 2 s.
2. `a_failed_handshake_still_reaps_its_child` — regression guard for the `Ok(Ok(Err))` arm (`:590-593`): `sh -c 'echo not-json; sleep 1000'` as the child **and** as the transport (`AcpIo::from_spawned`) → `Err(Transport)` naming `initialize`; `assert_not_running`. Passes before and after (the arm already waits on the task); it is here so the two arms are tested side by side.

### G-T32 — `crates/htui-agent/tests/probe.rs`, `tests/launch.rs`

Over `env(tmp)` (`:36-57`; `home = tmp/home`, `vars` injected) and a synthetic `CredentialProbe { env: ["HTUI_FAKE_KEY"], files: ["%HTUI_CRED_HOME%/token.json", "~/.htui-fake/token.json"] }` — an agent-agnostic block, so the rule is proven without the seed:

1. `no_candidate_is_absent_and_a_demanding_handshake_stays_unauthenticated` — `resolve_credential(Some(&probe), &env)` → `Ok(Some(Absent))`; `status_for(&two_methods, Some(Absent)) == Unauthenticated`. **Red first**: the function and the second argument do not exist.
2. `a_declared_file_makes_it_ready_and_the_home_candidate_answers_when_the_var_is_unset` — write `tmp/home/.htui-fake/token.json` with contents `"secret-9f8e"`; → `Some(File)`, `Ready`; then set `vars["HTUI_CRED_HOME"] = tmp/cred` with `tmp/cred/token.json` present → still `File` (first candidate); remove the home file, keep the var → `File` via the first pattern.
3. `a_declared_variable_counts_only_when_set_and_non_empty` — `vars["HTUI_FAKE_KEY"] = ""` → `Absent`; `= "x"` → `Some(Env)`; with the file present too → `File` (files first, D63).
4. `a_row_without_a_credential_block_keeps_the_milestone_5_rule` — `resolve_credential(None, &env)` → `Ok(None)`; `status_for(&two_methods, None) == Unauthenticated`; `status_for(&empty, None) == Ready`; `status_for(&empty, Some(Absent)) == Ready`.
5. `probe_agent_records_the_tier_and_never_the_value` — the `synthetic_row` + `DuplexTier2(two-method result)` shape of `:1290-1313`, with a discovery carrying the block and the token file present: `probe["credential"] == "file"`, `probe["status"] == "ready"`, `row.enabled`, and `serde_json::to_string(&probe)` does **not** contain `secret-9f8e`; without the file → `"absent"`, `"unauthenticated"`, `!row.enabled`. A `missing` outcome (tool absent) still carries `probe["credential"]` (step-2 placement).
6. `the_seeded_agy_row_declares_the_token_candidates_and_the_api_key_variable` — `agy_discovery()` (`:155-163`) → `credential == Some(CredentialProbe { env: ["GEMINI_API_KEY"], files: [the two strings verbatim] })`; `claude`'s seed → `None`.
7. Existing: `status_for(&found)` → `status_for(&found, None)` at `:1056` and in `handshake_reports_auth_methods_by_id`; the key-order assertion gains `credential` after `handshake`; `a_snapshot_round_trips_and_defaults_source_to_probe` also parses a document **without** `credential` → `None`.
8. `tests/launch.rs`: `AGY_LAUNCH` gains the block; `agy_launch_round_trips_with_per_platform_args` asserts it and that `to_string` → parse is equal; `claude_launch_round_trips` asserts `discovery.credential.is_none()` and that the serialised text has no `"credential"` key (P-10).

### G-T31 — `tests/probe.rs` (pure), `tests/acp_driver.rs` (driver level)

Fixture rule: the agent row's own discovery must **not** resolve on this box, so a launch that comes back with the marker can only have come from the snapshot. `synthetic_row("probed", "${tool}", Transport::Acp)` with `discovery.tools = { "tool": Path { names: ["htui-no-such-binary-2f8e"] } }` for (a) and (c); for (b), `NodePackage { package: "@htui/fake", entry: "dist/index.js" }` with the entry written under `spec.cwd/node_modules` (`tools.rs:219-246`'s deterministic tier). `on_box` built with `agent_box_row(&agent, BoxId::new(), None, &snapshot, now)` (`probe.rs:1188`).

1. `tests/probe.rs::recorded_launch_applies_d58s_three_row_side_rules` — `Ready`+`Probe`+`Some` → `Some`; `Unauthenticated` → `Some`; `Missing` (`resolved: None`) → `None`; `Failed` → `None`; `Manual` → `None`. **Red first**: no method.
2. `acp_driver.rs::a_usable_snapshot_is_what_the_driver_would_spawn_marker_included` — recorded `ResolvedLaunch { command: tmp/bin/htui-fake-adapter (a plain file), args: ["--marker=htui-d58"], env: {"ROW_VAR": "row", "SHARED": "row"} }`; `spec.env = {"SHARED": "spec", "SECRET": "s"}`; `AcpDriver::from_row_with_probe(&agent, Some(&on_box), caps_for(&agent))` → `launch_for(&spec)` → `command` = recorded, `args == ["--marker=htui-d58"]`, `env == {ROW_VAR: row, SHARED: spec, SECRET: s}`. **Red first**: today the same row is `Err(Unresolved("tool"))`.
3. `acp_driver.rs::through_the_factory_the_spawned_argv_carries_the_marker` (`#[cfg(unix)]`) — recorded command = `executable(tmp/bin/htui-fake-adapter, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HTUI_ARGV_FILE\"\nexit 0\n")`, recorded `env: {"HTUI_ARGV_FILE": tmp/argv}`; `DriverFactory::with_acp().driver_for(&agent, Some(&on_box))` → `start(spec, "hi")` → `Err` (the adapter answered nothing; the message names `initialize` or the handshake) and `tmp/argv` reads `--marker=htui-d58`. This is `AcpAdapter::build` no longer ignoring `on_box`, proven end to end.
4. `acp_driver.rs::a_recorded_command_that_is_gone_falls_back_to_resolution` — recorded `command: tmp/gone/agy_acp_server.par` (never created), `args: ["--uid="]`; the (b) row → `launch_for` → `command` = the `node_modules` entry, `args == []`. The `info!` is the trace; the assertion is the fallback.
5. `acp_driver.rs::a_manual_snapshot_is_not_used_as_a_launch` — the (b) row with `source: manual`, recorded command present, marker args → resolution's launch, no marker.
6. `acp_driver.rs::a_probe_column_that_does_not_parse_resolves_as_before` — `on_box.probe = Some(json!({"status": "ready"}))` → same as (b)'s resolution.
7. `acp_conformance.rs` unchanged (`AcpDriver::over` keeps its signature; `Prepared` is untouched).

### G-T35 — `crates/htui/src/agent_worker.rs` in-module tests

Row: `Agent { transport: Acp, launch: json!({ "command": "/nonexistent/htui-d60/agent" }), settings: json!({}), enabled: true, … }` in `MemStore::demo()`; runtime `AgentRuntime::production()` (the real ACP builder; `tools::resolve(None)` → empty map → `launch::resolve` ok → `launch::spawn` → `which` fails → `DriverError::Spawn`, no process). Pre-insert a **fresh** `agent_box` (`agent_box_row` over `ProbeSnapshot { status: Ready, source: Probe, resolved: Some(that command), .. }`, `probed_at = now`) so `needs_reprobe` is false and `background_len()` stays 0 — the only re-probe that can run is D60's.

1. `a_spawn_failure_reports_the_adapters_message_and_reprobes_the_row_off_the_arm` — `serve(ChatStart)` → `Served::Start { task, .. }`; **before polling the task**: `runtime.background_len() == 0` and the stored row still says `ready` with the inserted `probed_at` (nothing happened on the arm); `task.await`; replies hold `Failed { request: "chat_start", message }` with `message.contains("is not executable")` and `ChatFrame::Failed`; `store.agents()` → the row's `probe["status"] == "failed"`, `probe["stderr_tail"][0]` contains `not executable`, `!enabled`, `probed_at` moved. **Red first**: the row is untouched after the task.
2. `an_unresolved_placeholder_does_not_reprobe` — `unresolvable_registry()`'s shape (`:1847-1865`) with the same fresh pre-insert → `Failed { chat_start }` naming `gone`, row untouched (H-8 pinned).
3. `a_stale_row_reprobes_once_not_twice` — same as 1 with `probed_at = now - 25 h`: `background_len() == 1` after `serve`, then `finish_background`, then the task; the row was written **once** (count writes through a `MemStore` read of `updated_at` before/after, or a `Writer` wrapper if one exists — **UNVERIFIED — implementer must check** whether `MemStore` exposes a write count; otherwise assert only that `probed_at` moved and no panic).
4. `needs_reprobe_is_absent_or_older_than_the_ttl` unchanged.

### G-T33 — `crates/htui-agent/tests/agy_live.rs` (CREATE, `#[ignore]`)

Module doc, mirroring `probe_live.rs:1-25`: what it spawns (`agy_acp_server.par` through the seed glob and `agy --version` through tier 1), why it burns no tokens (`initialize`, and `session/new` in case 3 — protocol traffic, no prompt), the run line `cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture`, and its **preconditions, stated so a miss is loud and named**: (i) T29 — `~/.local/share/htui/agents/antigravity-acp/<ver>/agy_acp_server.par` executable (the test resolves the seed glob first and panics with the D57 URL and the `chmod +x` line if it finds nothing); (ii) `agy` on `PATH` — the seed's `agy` tool is a `Path` probe and `probe_tools` requires every tool (H-11); (iii) the box's credential state is *observed*, not assumed (P-9).

1. `the_seeded_agy_row_resolves_through_the_glob_appends_uid_and_answers_v1` — `probe_agent(&agy_row(), BoxId::new(), None, &ProbeContext { env: ProbeEnv::host(manifest_dir), now }, &SpawnTier2::default())`; prints `agent_box.probe` (the `agy_acp_handshake.json` fixture source); asserts `probe["resolved"]["args"] == ["--uid="]` on `linux-x86_64`/`linux-aarch64` and `[]` elsewhere, `probe["handshake"]["protocol_version"] == 1`, `auth_methods` non-empty (prints the ids; ANA-4 §4.5 says four), `probe["credential"] ∈ {"file","env","absent"}`, and `status == "ready"` iff `credential ∈ {"file","env"}` with `row.enabled == (status == "ready")`; then the survivor check (`probe_live.rs:200-208`'s 500 ms + `children_of_this_process`). **§11.14 `:1391-1392`, half one**: the `.par` launches and answers on Linux with `--uid=`.
2. `the_par_mechanics_are_recorded` — never fails on the adapter's behaviour, only on preconditions: prints `read_dir` of the `.par`'s parent (what sits beside it — the `localharness_external.exe` question, `:1392`); runs `acp::handshake` over `launch::spawn` of the resolved launch **without** `--uid=` and prints `Ok(h)` or the error with its stderr tail (what `--uid=` does, `:1391`); prints `file`-style facts the test can read itself (size, mode). Recorded in T36's table.
3. `session_new_reports_its_config_options` (P-8, D64) — `acp_live.rs:36-120`'s hand-driven client: `initialize`, then `NewSessionRequest::new(manifest_dir)` and `session.config_options()` printed verbatim (`serde_json::to_value`), then kill + survivor check. On an unauthenticated box the `session/new` error is itself the answer and is printed, not asserted against. Answers `:1389-1390` (model list and `configOptions`) without spending a token.

### G-T34 — `crates/htui/tests/chat_live_agy.rs` (CREATE, `#[ignore]`)

Modelled on `chat_live.rs:29-154`: `MemStore::demo()`, the `agy` summary, `Backend::memory`, `AgentRuntime::production().with_grace(1 s)`, `ChatStart` → frames until `Done` → `ChatCancel` → `Ended`, then the log and the survivor check (`pgrep -f agy_acp_server` before/after, the `matching_processes` pattern of `:158-167`). Two additions that are the point of the file:

- **Probe first, then chat.** Before `ChatStart`, `probe_agent` over the seed row with `SpawnTier2::default()` and `store.upsert_agent_box(&row)`; assert `status == ready` (the D63 login precondition — a miss panics with "run `agy_acp_server`'s own login first; `htui` cannot", ANA-4 §4.5 `:706-714`). Only a written `agent_box` row makes `AgentRuntime::start` → `driver_for(agent, Some(on_box))` → `launch_for` spawn the recorded launch **with `--uid=`** (E-1). Without it the chat would resolve through `tools::resolve` and launch without the flag — the very H-3 defect.
- **Three turns, each answering §11.14 lines**, listed in the module doc as the assertion → item table:

| Turn / assertion | Answers (`docs/ANA-4.md`) |
|---|---|
| `ChatAccepted.caps == caps_for(&agy_row)` and equals the `claude` row's profile (`registry.rs:144-152`) — no capability banner | criterion 11's "differing only by registry row and capability banner"; the plan's sixth item |
| Turn 1 "Reply with exactly the word ok": the `session_started` banner's `agent_name`/`agent_version`/`models`; every `DriverEvent::Usage` frame printed with its body, or the absence recorded | `:1384` (`usage_update` presence and field, §7) — plus `:1389` (`models` as the banner projects it; the ids come from T33 case 3) |
| Turn 2 (`ChatSend`) "Read README.md and reply with its first heading, using a tool": a `PermissionRequest` frame → printed `options[].{id, kind}` and answered with the first `AllowOnce` via `ChatAnswer`; or the absence recorded ("no request for a read in `default` mode") | `:1385-1386` (`session/request_permission` in `default` mode, option ids and kinds) |
| Turn 3 "Create a file named `htui-agy-probe.txt` in the working directory containing the word ok": the gating request answered allow; then **either** an `EditProposal` with a non-empty `diff` **or** a `ToolCall { tool_kind: Edit }` plus whatever `other` rows carried the vendor shape — the test asserts one of the two arrived and prints which; the file is removed by the test afterwards (H-10) | `:1387-1388` (standard `tool_call` + `diff`, or vendor shape); D62: the mapper is amended only if neither shape mapped |
| Every raw `session/update` (rows' `raw`, present only under `HTUI_KEEP_RAW_EVENTS=1`, `agent_worker.rs:509`, `:874`) written to `$HTUI_AGY_FIXTURE_OUT` when that variable is set (read with `std::env::var`, never written), else printed | the `agy_acp_turn.jsonl` fixture for `acp_map.rs` (A.19) |

Run line (P-6): `HTUI_KEEP_RAW_EVENTS=1 HTUI_AGY_FIXTURE_OUT=/tmp/agy_acp_turn.jsonl cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture`.

---

## G. Hazards and findings the plan does not name

| # | Finding | Where | Consequence / mitigation |
|---|---|---|---|
| H-1 | D61's timeout arm signals but does not reap (the same asymmetry the probe's abort path accepts, `launch.rs:651-652`) | T30 | **Not a leak, and no reaper task is owed.** Verified in tokio 1.53.1: dropping an unwaited `Child` pushes it onto the global orphan queue (`process/unix/reap.rs` `Reaper::drop`, `pidfd_reaper.rs` for the pidfd path) and the process driver drains that queue on every park once `SIGCHLD` has arrived (`runtime/process.rs:33`). The zombie lasts until the next park, not until process exit; Windows has no zombie state at all. The original "a `Drop` that hands the `Spawned` to a reaper task" idea is **withdrawn** — it would duplicate the runtime |
| H-2 | `SessionOptions` gains a field; it is `pub` with pub fields and no constructor | T30 | The only literal is `acp/mod.rs:311`; nothing outside the crate builds one (grep). A downstream that did would break at compile time, which is the right time |
| H-3 | A recorded `command` that is a bare name (an `HTUI_TOOL_*` override the probe accepted, `probe.rs:360-372`) fails `launch_for`'s `metadata` check relative to nothing and falls back — to the same override | T31 | Harmless and documented in `launch_for`; the override tier is the fallback's first tier anyway (`tools.rs:93-97`) |
| H-4 | `unauthenticated` snapshots are usable (D58), and `AgentRuntime::start` gates on `agent.enabled` — the **registry** row (`agent_worker.rs:470`) — not on `agent_box.enabled` | T31 | Spawning the recording is the only launch that *starts*: on Linux it is the only one carrying `--uid=`, and a second resolution would exit in `ChangeRootAndUser` before reading stdin (measured in T36 case 2). `unauthenticated` is exactly the status certifying the binary is right and the box is not logged in, so the chat fails at `session/new` with the vendor's own `Authentication required` — which reaches the caller since the review fix to `open_session` (the SDK sends `session/new` from a connection actor, so the refusal arrives as the connection future's error and is reported from there). The store-side gate is MOD-4's `probe->>'status'` predicate, not this milestone's |
| H-5 | A `manual` row's hand-written `probe.resolved` is *not* what the chat spawns (D58: `source == probe` only) — the chat spawns the row's `launch` through `tools::resolve` + `HTUI_TOOL_*` | T31 | Settled by D58 and left as is; stated here so the eventual manual-entry editor (milestone 7+) knows the two paths differ |
| H-6 | `ProbeSnapshot` gains a key; the milestone-5 blueprint's C section and its key-order test both change | T32 | One test assertion moves (A.14); `from_row` on a milestone-5 row still parses (`#[serde(default)]`); MOD-4's `probe->>'status'` is untouched |
| H-7 | The inline D60 re-probe extends a failed chat task by up to `HANDSHAKE_TIMEOUT`; `shutdown` waits `grace * 2` per chat (`agent_worker.rs:346`) and moves on **without aborting** | T35 | At process exit the runtime drops the task and the tier-2 `ChildGuard` signals its child. A one-line `task.abort()` after the timeout in `shutdown` would make it immediate; out of scope, flagged |
| H-8 | D60 triggers on `Spawn` only; `Unresolved` at chat start (a tool that vanished) does not re-probe | T35 | Deliberate per D60's wording; the 24 h staleness path covers it, and the `Unresolved` message already names the placeholder. Adding it is one arm in `run_chat`'s `if let` — recorded, not done |
| H-9 | A stale row and a spawn failure on the same chat would run two re-probes for one row | T35 | `ChatArgs.reprobe = None` when the staleness one was spawned (B.4); G-T35 case 3 pins it |
| H-10 | T34's turn 3 asks the agent to write a file in `spec.cwd`, which under `cargo test` is `crates/htui` | T34 | A named file the test deletes in a `defer`-style guard at the end whether or not the turn succeeded; the maintainer runs it with a clean tree so a stray `htui-agy-probe.txt` is visible in `git status` |
| H-11 | The seed's `agy` `Path` tool is **required**: a box with the server installed but no `agy` CLI on `PATH` probes `missing` and never reaches tier 2 | T33 | On this box the mise shim satisfies it (plan T29). Whether the CLI should be optional in the row is a seed/ANA-4 §5.3 question for the maintainer, not a code change here |
| H-12 | `retain_raw` is read from the process environment (`KEEP_RAW_ENV`), which a test cannot set | T34 | The variable goes on the command line (P-6); the fixture path is read, never written, by the test |
| H-13 | The `agy_acp_turn.jsonl` fixture will carry absolute paths under `$HOME` and a session id | T34 | Redact by hand before commit, as milestone 3's fixtures were; the `acp_map.rs` case asserts kinds, not paths |
| H-14 | `AGY_LAUNCH` in `tests/launch.rs` and ANA-4 §5.3 both claim to be the seed byte for byte | T32, T36 | All three move together (A.10, A.15, A.20) |
| H-15 | `tokio::test` defaults to `current_thread`; T30's `task.abort(); task.await` relies on the aborted task being dropped on the next scheduler tick | T30 | The test's own `await` on the `JoinHandle` yields to the scheduler; no `multi_thread` flavour needed. `spawn_blocking` inside `launch::spawn` still works on `current_thread` (the blocking pool is separate) |
| H-16 | `launch_for` on `IoSource::Prepared` returns `Err(Transport)` | T31 | Documented; the conformance harness never calls it, and `io()`'s `Prepared` arm is untouched |
| H-17 | The recorded `resolved.env` now reaches a child again (milestone-5 H-12 said it "never" would) | T31 | It is the row's `agent.launch.env` with placeholders substituted — paths already in the row — and `spec.env` still extends over it last (`B.2`); `RedactedEnv` covers the `Debug` of both |
| H-18 | Milestone-5 H-7 (fixture rule) | all | Every new non-live case here uses a synthetic row with an unresolvable or `node_modules`-local discovery; `agy_live.rs` and `chat_live_agy.rs` join `probe_live.rs` as the files allowed to probe a seed row, and each says so in its module doc |
| H-19 | Milestone-5 H-13 (`start_kill` existence) | T30 | Verified present: `launch.rs:569-573` over `process_wrap::tokio::ChildWrapper::start_kill` |

---

## H. What this milestone does NOT touch

- **`htui-core` and `htui-store`**: no model change (`AgentBox.probe` stays `Option<Value>`), no migration, no `ReadStore`/`WriteStore` method, no `.sqlx` regeneration; `pg_conformance.rs` `EXPECTED_CASES` unchanged.
- **The event model**: no `DriverEvent` variant (D62), no `ChatFrame`/`StoreReply`/`StoreRequest` variant, no `EventKind`.
- **The registry seam**: `TransportBuilder::build`'s signature (`registry.rs:38-43`) and `DriverFactory::driver_for` are unchanged — `on_box` was already threaded through (`agent_worker.rs:476-479`); nothing gains an arm on `agent.name`.
- **`acp::handshake` and the probe's tier 2**: the guard was already theirs; `SpawnTier2`, `Tier2`, `probe_tools`, the walker and version capture are untouched except for `exists`'s visibility.
- **The turn loop, cancel sequence and mapper** (`acp/mod.rs:924-1340`, `acp/map.rs`): re-typed helpers only; a mapper amendment happens only if T34's fixture demands it (D62), as its own reviewable diff.
- **The `cli` transport and any runtime fallback between transports** (D60, milestone 8): a CLI agent is its own registry row.
- **Downloading or updating the adapter** (D57): a maintainer step documented in the README.
- **Credential *values***: `SessionSpec.env` stays empty until MOD-10 (`agent_worker.rs:502-504`); the probe records a tier name and reads no file's contents.
- **Windows runtime facts** (MOD-16): the `%GEMINI_HOME%`/`~` candidates and the JetBrains globs compile and are proven with injected vars on Linux; whether the Windows server writes `acp_token.json` under `%USERPROFILE%\.gemini` is not claimed.
- **Settings UI**: the `on this box` column already renders `unauthenticated`/`ready` (`settings/agents.rs`); the new `credential` key has no reader in the tab yet.
- **`AgentRuntime::shutdown`'s straggler handling** (H-7): flagged, not made. A reaping `ChildGuard::drop` (H-1) is no longer flagged at all — the review established that tokio's orphan queue already does it, and duplicating it would be the actual mistake.
