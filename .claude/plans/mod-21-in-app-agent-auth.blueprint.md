# Blueprint: MOD-21 — in-app agent authentication (milestones 1–4)

**Plan**: `.claude/plans/mod-21-in-app-agent-auth.plan.md` (D9–D23, T1–T9, H-1…H-20 binding; its "Verified claims" table taken as read). **PRD**: `.claude/prds/mod-21-in-app-agent-auth.prd.md` (D1–D8, the maintainer's). Re-checked against the tree at `5a16669` on 2026-09-09. Where the plan and the tree disagree the tree wins and the point is marked **plan ≠ tree**; anything decided here that the plan left open is **decided here**; anything not verified in source is **UNVERIFIED — implementer must check**.

Conventions inherited unchanged: `#![warn(missing_docs)]`, `unsafe_code = "forbid"` (`Cargo.toml:80`; no `set_var`, no FFI opener), MSRV 1.98 (`Cargo.toml:7`), edition 2024, `missing_debug_implementations` (every `pub` type below derives or hand-writes `Debug`), no lock across an `.await`, nothing long on the UI task or in the worker's `select!` arm (`R-NF-3`), per-file test helpers, `R-AGT-5` (no agent name, method id or vendor host outside `crates/htui-core/seeds/` and `tests/`; verified today: **none** of the eight strings T1 sweeps for appears under any `crates/*/src`, so the sweep is vacuous until T2 and load-bearing from then). **No `cargo clippy --target x86_64-pc-windows-msvc` line anywhere in this document** (TOOL-3, D22).

---

## Plan ≠ tree, resolved

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | D9/T3: `auth/run.rs` "resolves the launch, applies the browser policy, spawns, taps stderr" and T3 tests a prepared transport through `AcpDriver::over` | `IoSource` (`acp/mod.rs:225`, `Prepared(Mutex<Option<Box<AcpIo>>>)`) is private to `acp/`; `launch_for` refuses a prepared transport by name (`:364-369`) | **Decided here**: `acp/mod.rs` gains `pub(crate) async fn auth_source(&self, cwd: &Path) -> Result<AuthSource>` answering `AuthSource::Launch(ResolvedLaunch)` (the `launch_in` result, the row's env) or `AuthSource::Prepared(AcpIo)` (the slot taken). `auth/run.rs` matches on it: `Launch` → `BrowserPolicy::apply` on that value, `launch::spawn`, `tap_stderr`, `AcpIo::from_spawned`; `Prepared` → no tap, no policy. The policy is still applied in `auth::run` only (H-11) and `acp/` is still the only module that knows `IoSource` (B.4) |
| P-2 | D13/H-13: `finish_background` "handles `auth` as it handles `install` (cancel then abort under the limit)" | `finish_background` (`agent_worker.rs:335-342`) **awaits** the install under `limit` first and cancels+aborts only on timeout | Mirror the tree's arm exactly (B.9): await under `limit`, on timeout `cancel.cancel()` then `abort()`. A flow parked on a human therefore costs `drive_to_end` the whole `CHAT_END` — which is why T7's harness cases use `until()`+`drive()` while a flow is live and `drive_to_end` only after `x` or after `Done` (H-13 as the plan words it for T7) |
| P-3 | D18: "the task switches its `Frames.addr` to the `AuthChoose` address" | `Frames { tx, addr }` (`:1474-1477`) has a private `addr`; `run_install` takes a local `let addr = frames.addr.clone()` and calls `frames.reply(&addr, ..)` (`:1396`, `:1470`) | The switch is a **local** `let mut addr: ReplyAddr` in `run_auth`, reassigned when `AuthCommand::Choose { reply, .. }` arrives; `Frames` is untouched (B.9) |
| P-4 | T1: the sweep is "the `the_codebase_has_never_heard_of_zeta` walk (`:111-131`) over `crates/*/src`" | The zeta walk reads every file whole and would not compile against in-module test blocks; the tree's closer precedent is `the_installer_names_no_vendor` (`tests/extensibility.rs:276-308`) over `tree_files()` filtered to `src/` through `production_half` (`:310-315`, MOD-20 blueprint H-12 correction) | `the_auth_flow_names_no_vendor_or_method` is written as a second `the_installer_names_no_vendor` with the eight strings (B.1, F) |
| P-5 | T5/T6: "the opener command is injectable for the test (`OpenerCommand`)", T6's `open_is_forwarded…` uses "the injected opener" | `AgentRuntime` (`:205-237`) has `with_installer` as its one injection seam; nothing carries an opener | **Decided here**: `AgentRuntime.opener: OpenerCommand` (default `Platform`) and `pub fn with_opener(self, OpenerCommand) -> Self` (B.9) |
| P-6 | T6: the fixture row's `launch.command` is "the script's path with no `discovery` so `probe_tools` completes" | (a) H-12/`tests/probe.rs:70-92`: a script written and executed in one process trips `ETXTBSY`, and the re-probe's `SpawnTier2` has no retry; (b) the `ready` verdict needs `discovery.credential.files` to name the file the fixture creates (`probe.rs:1197-1205`) | Row: `command: "/bin/sh"`, `args: ["<tmp>/agent.sh"]`, `discovery: { tools: {}, handshake: true, credential: { env: [], files: ["<tmp>/credential"] } }`. **UNVERIFIED — implementer must check** (`probe.rs:1444-1448`) that an empty `tools` map with a `credential` block completes step 3 and reaches tier 2; if not, drop `discovery` and assert `Unauthenticated` only in the T6 `Done` case, leaving `ready` to T7's harness row (C.2) |
| P-7 | D12/T2: `WireFlow` carries `handshake_timeout` "the subset of `AuthFlow` the wire needs" | `HANDSHAKE_TIMEOUT` is `acp::HANDSHAKE_TIMEOUT` (`acp/mod.rs:95`), not a field of `AcpSettings` (`launch.rs:283-292`) nor of `AuthFlow` | `auth::run::authenticate` fills `WireFlow.handshake_timeout` from the const; tests fill it with seconds. `AuthFlow` does not carry it (B.2, B.3) |
| P-8 | D18: `AuthOpen` answered `Opened` or "`Failed { request: "auth_open" }`"; `AuthFrame` also has `Failed { message }` | Two different `Failed`s exist in the tree: `StoreReply::Failed { request, message }` (`store_worker.rs:277-282`, the shell's status line) and a frame-local one (`InstallFrame::Failed`, `:320`) | Both are used, on purpose: a refusal *of a request* (`auth_start`, `auth_choose`, `auth_open`, `auth_cancel`) is `StoreReply::Failed { request, .. }` at that request's seq; a failure *of the running flow* (spawn, transport, the row write) is `StoreReply::Auth(AuthFrame::Failed { message })` at the stream's current seq (B.8, B.9) |
| P-9 | D14: the reader forwards to "an `Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>`" beside the deque, replay "under the same lock the reader holds" | `Spawned.stderr_tail: Arc<Mutex<VecDeque<String>>>` (`launch.rs:528`); two `Arc<Mutex>`s would be two locks | **Decided here**: one lock — `Arc<Mutex<StderrTail>>` with `StderrTail { lines: VecDeque<String>, tap: Option<mpsc::UnboundedSender<String>> }`; `stderr_tail()`'s behaviour unchanged (B.5) |
| P-10 | D18: refusals "in the probe's order": writer, box, claim, row, predicate, methods | `install_plan` (`:576-615`) is the tree's order: `recording_writer`, `registered_box`, `claim_is_free`, `row_for`, then the document check | The same six helpers, in that order, plus the `ReprobeClaims::claim` between the claim and the row (D19); the predicate and the method list are read off `row_for`'s `AgentSummary` (B.9) |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui-agent/src/driver.rs` | UPDATE | T1 | `DriverCaps.authenticate` after `usage` (`:332`); `AgentDriver::authenticate` with a default body after `start` (`:361`); doc `:34-35` "five operations" → "six". **Not**: `AgentSession`, `SessionSpec` |
| 2 | `crates/htui-agent/src/error.rs` | UPDATE | T1 | `DriverError::Unsupported(&'static str)` after `Closed` (`:35`) |
| 3 | `crates/htui-agent/src/registry.rs` | UPDATE | T1 | `caps_from` (`:144-151`, `:158-166`): `authenticate: true` / `false` |
| 4 | `crates/htui-agent/src/fake.rs` | UPDATE | T1 | `full_caps` (`:104-111`): `authenticate: false` |
| 5 | `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE | T1 | the `#[cfg(test)]` literal at `:688-695` gains `authenticate: true` (the banner at `:326-332` reads three predicates; no snapshot moves) |
| 6 | `crates/htui-agent/src/auth/mod.rs` | CREATE | T1, T5 | `AUTH_IDLE_CAP`, `AuthFlow`, `AuthEvent`, `AuthMethodInfo`, `AuthChoice`, `AuthCall`, `AuthOutcome`, `BrowserPolicy` (data only at T1), `OpenerCommand` (T5); `pub mod run` (T3), `pub mod url`, `pub mod browser` (T5) |
| 7 | `crates/htui-agent/src/lib.rs` | UPDATE | T1, T5 | `pub mod auth;` after `acp` (`:81`); re-export line (B.2) |
| 8 | `crates/htui-agent/tests/driver_contract.rs` | UPDATE | T1 | four cases; `StubDriver` (`:42-60`) untouched |
| 9 | `crates/htui-agent/tests/extensibility.rs` | UPDATE | T1 | `the_auth_flow_names_no_vendor_or_method` beside `the_installer_names_no_vendor` (`:276`) |
| 10 | `crates/htui-agent/src/acp/auth.rs` | CREATE | T2 | `WireFlow`, `run` (B.3). The only new file that imports `agent_client_protocol::schema` |
| 11 | `crates/htui-agent/src/acp/mod.rs` | UPDATE | T2, T3 | T2: `pub mod auth;` after `handshake` (`:22`), `pub use crate::acp::auth::{WireFlow, run as run_auth}` beside `:45`. T3: `launch_for` → `launch_in` + wrapper, `AuthSource`, `auth_source`, `AgentDriver::authenticate` for `AcpDriver` (B.4). **Not**: `open_session`, the mapper |
| 12 | `crates/htui-agent/src/acp/client.rs` | UPDATE | T2 | one inline test beside `:173-195`; `client_capabilities` (`:32-38`) **unchanged** |
| 13 | `crates/htui-agent/tests/auth.rs` | CREATE | T2, T3, T5 | scripted duplex agent, `sh` fixture, hijack fixture, every offline case (C, F) |
| 14 | `crates/htui-agent/src/auth/run.rs` | CREATE | T3, T5 | `authenticate` (B.4): T3 spawn + wire; T5 tap, URL, idle |
| 15 | `crates/htui-agent/src/launch.rs` | UPDATE | T4 | `StderrTail`, `Spawned::tap_stderr`, the reader (`:798-810`). **Not**: `spawn`'s signature, `ChildGuard`, `STDERR_TAIL_LINES` |
| 16 | `crates/htui-agent/tests/launch.rs` | UPDATE | T4 | five tap cases beside `:461` |
| 17 | `crates/htui-agent/src/auth/url.rs` | CREATE | T5 | `first_url` (B.6) |
| 18 | `crates/htui-agent/src/auth/browser.rs` | CREATE | T5 | `BrowserPolicy::apply`, `open_url`, the one `cfg(windows)` arm (B.7) |
| 19 | `crates/htui/src/store_worker.rs` | UPDATE | T6 | four variants after `InstallCancel` (`:159`), `name()` arms (`:189`), `StoreReply::Auth` after `Install` (`:240`), `AuthFrame` after `InstallFrame`, the `try_serve` arm (`:426-436`), the loop arm (`:574-591`), one loop-freedom test beside `:1358` |
| 20 | `crates/htui/src/agent_worker.rs` | UPDATE | T6 | `AuthCommand`, `LiveAuth`, `AgentRuntime.{auth, opener}`, `with_opener`, `auth_running`, `serve` sweep + four arms, `probe`/`claim_is_free`/`finish_background`/`shutdown` extended, `auth_start`/`auth_choose`/`auth_open`/`auth_cancel`, `AuthArgs`, `run_auth`, tests |
| 21 | `crates/htui/src/testkit.rs` | UPDATE | T6 | the runtime list in `drive` (`:186-194`) gains the four |
| 22 | `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T7 | `AuthState`, `auth` field, `a`/`o`/`Enter`/`n`/`Esc`/`x` arms, `on_auth_frame`, `auth_cell`, pane, hints, `HINT_IDLE` (B.10). **Not**: the column set, `on_box_cell`'s order after the in-flight arms, `on_reply(Agents)` |
| 23 | `crates/htui/tests/settings.rs`; `tests/snapshots/settings__agents_{demo,empty,probed,unknown_row}.snap`; `crates/htui/tests/auth.rs` | UPDATE / CREATE | T7 | section cases; four snapshots re-accepted for the idle hint; the harness file over the `sh` fixture (per-file helpers) |
| 24 | `crates/htui-agent/tests/auth_live.rs` | CREATE | T8 | D23, `#[ignore]` |
| 25 | `crates/htui-core/seeds/agent_agy.json` | UPDATE (conditional) | T8 | `credential.files` (`:38-40`) only if the live run names another file |
| 26 | `README.md:274-280`, `HANDOFF.md`, `crates/htui-agent/tests/agy_live.rs`, the PRD, the plan | UPDATE (docs) | T9 | D23 |

Not touched, on purpose (section H): `crates/htui-store/**`, `crates/htui-store/migrations/**`, `crates/htui/src/keymap.rs`, `crates/htui/src/ui/overlay/**`, `crates/htui-agent/src/probe.rs`, `crates/htui-agent/src/acp/handshake.rs`, `crates/htui-agent/src/install/**`, `crates/htui-core/src/**`. No manifest changes: `which`, `tokio-util` (`sync` by unification, plan Verified-claims), `tokio::process`, `serde` are already `htui-agent` dependencies; `htui`'s dev `tempfile` exists (`Cargo.toml:54`).

---

## B. Interfaces, exactly

### B.1 T1 — the seam (`driver.rs`, `error.rs`, `registry.rs`, `fake.rs`)

```rust
// driver.rs — DriverCaps (:317-333) gains, after `usage`:
    /// The transport can log the agent in through its own protocol (plan MOD-21 D10, `R-AGT-9`):
    /// `AgentDriver::authenticate` answers something other than `DriverError::Unsupported`. The
    /// Settings section refuses `a` on a row whose profile says `false` before a request is spent.
    pub authenticate: bool,

// driver.rs — AgentDriver (:344-362) gains, after `start`:
    /// Logs the agent in (or out) through its own protocol, off any session (plan MOD-21 D10, D11).
    ///
    /// One spawn serves the method list and the call: the flow's `events` carries the agent's own
    /// method names, the caller answers on `choice`, and the child is killed on every exit. The
    /// **default body refuses**: a transport that says nothing about authentication has none, which
    /// is the same structural reading `DriverCaps::default()` gives every other predicate.
    ///
    /// # Errors
    /// [`DriverError::Unsupported`]`("authenticate")` from the default body;
    /// [`DriverError::Spawn`] / [`DriverError::Transport`] from a transport that has one.
    fn authenticate<'a>(&'a self, flow: AuthFlow) -> DriverFuture<'a, AuthOutcome> {
        // Dropping `flow` drops its `events` sender: a caller's forwarding loop ends cleanly.
        drop(flow);
        Box::pin(async { Err(DriverError::Unsupported("authenticate")) })
    }
// imports added: `use crate::auth::{AuthFlow, AuthOutcome};`
// doc :34-35: "…the seam reads as six operations rather than as six copies of one type."

// error.rs — after `Closed` (:35):
    /// This transport has no such operation (plan MOD-21 D10). The name is the trait method's, so
    /// the Settings section and the runtime can branch on the variant and print the sentence.
    #[error("this transport has no `{0}` operation")]
    Unsupported(&'static str),

// registry.rs caps_from: `Transport::Acp => DriverCaps { .., usage: true, authenticate: true }`
//                        `Transport::Cli => DriverCaps { .., usage: true, authenticate: false }`
//   (doc on the Cli arm: "no CLI transport has a login verb; the vendor CLI's own is not ours")
// fake.rs full_caps: `authenticate: false` with the doc "the fake is the reference transport for
//   *sessions*; `authenticate` is proven through the default body (plan MOD-21 D10)".
```

The dyn-compatibility of a default body returning `DriverFuture<'a, T>` is the plan's compile-probed claim; `StubDriver` in `tests/driver_contract.rs:42-60` compiles unchanged and inherits the refusal.

### B.2 T1 — `crates/htui-agent/src/auth/mod.rs`

```rust
//! Plan MOD-21: the transport-neutral half of a login. Everything a *user* sees of a flow — a
//! method's name, a stderr line, a link, an outcome — is a type here; the wire (`acp/auth.rs`)
//! produces them and the runtime consumes them. Nothing here can carry a credential: every field
//! is an id, a sentence the agent already wrote to its own stderr, or a status (`R-SEC-2`,
//! `R-ID-7`). Nothing here names an agent, a method id or a host (`R-AGT-5`): the method list is
//! the agent's own `initialize` and the outcome is the probe's.

pub mod browser;   // T5
pub mod run;       // T3
pub mod url;       // T5

pub use browser::open_url;          // T5
pub use run::authenticate;          // T3
pub use url::first_url;             // T5

/// D13: a flow silent for this long — no stderr line, no event, no choice — is cancelled and
/// reported `Idle`. Measured from the last sign of life, never from the spawn: a human OAuth
/// round trip is minutes of nothing on stderr. Injected through `AuthFlow.idle` so tests use ms.
pub const AUTH_IDLE_CAP: Duration = Duration::from_secs(10 * 60);

/// Everything one login needs from its caller (D11).
#[derive(Debug)]
pub struct AuthFlow {
    /// Where the adapter runs; the probe's `cwd`, never a session's.
    pub cwd: PathBuf,
    /// The method list, then every stderr line and every new URL, as they happen.
    pub events: mpsc::UnboundedSender<AuthEvent>,
    /// The one answer to `AuthEvent::Methods`. A sender dropped unused is `AuthOutcome::Declined`.
    pub choice: oneshot::Receiver<AuthChoice>,
    /// Tripped by the pane, by shutdown, or (through a child token) by the idle clock.
    pub cancel: CancellationToken,
    /// [`AUTH_IDLE_CAP`] in production.
    pub idle: Duration,
    /// D16: `Neutralised` in production; `Inherit` exists for the regression pair.
    pub browser: BrowserPolicy,
}

/// What a flow tells its caller (D11). Text and ids only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// The agent's `initialize` answer, once, before anything else.
    Methods {
        /// Every `agent`-kind method, in the agent's order (an unknown `type` is one of these:
        /// the schema's untagged arm folds it here, plan Verified-claims).
        methods: Vec<AuthMethodInfo>,
        /// `agentCapabilities.auth.logout` was advertised.
        logout: bool,
        /// `terminal`-typed methods: named so the chooser can say why, never sent (D4, D21).
        hidden: Vec<AuthMethodInfo>,
    },
    /// One stderr line, as the adapter wrote it (T5).
    Line(String),
    /// A `http(s)` URL seen on stderr for the first time in this flow (D15; T5).
    Url(String),
}

/// One advertised method, the agent's own words. Crosses `StoreReply` (derives per plan T1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodInfo {
    /// `authMethods[].id` — the value `authenticate` is sent.
    pub id: String,
    /// `authMethods[].name`.
    pub name: String,
    /// `authMethods[].description`.
    pub description: Option<String>,
}

/// The user's answer to `Methods`. Crosses `StoreRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthChoice {
    /// `authenticate` with this method id.
    Method(String),
    /// `logout`.
    Logout,
}

/// Which call the flow made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCall {
    /// `authenticate` with this id.
    Authenticate(String),
    /// `logout`.
    Logout,
}

/// How a flow ended (D11). `Ok` covers everything the agent got to say, including "no".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// The call returned. Says nothing about the box: the probe decides (D6, `R-AGT-6`).
    Completed { call: AuthCall },
    /// The agent answered the call with a JSON-RPC error; `message` is its own `Display` text
    /// plus the stderr tail on a new line (D5, H-10).
    Refused { call: AuthCall, message: String },
    /// The token tripped (the pane, or shutdown).
    Cancelled,
    /// The idle clock tripped after this much silence (D13).
    Idle { after: Duration },
    /// The `choice` sender was dropped before choosing.
    Declined,
}

/// D16: what the auth spawn's environment says to the adapter's own browser opener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserPolicy {
    /// `BROWSER` = an existing no-op that exits 0. Production.
    #[default]
    Neutralised,
    /// The environment as the row resolved it. The regression pair's control.
    Inherit,
}

/// D17: what `open_url` spawns. Production is `Platform`; a test injects a script.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OpenerCommand {
    /// `xdg-open` / `open` / `powershell.exe Start-Process`, by `cfg`.
    #[default]
    Platform,
    /// This program, the URL as its one argument.
    Custom(PathBuf),
}
```

`lib.rs`: `pub use auth::{AUTH_IDLE_CAP, AuthCall, AuthChoice, AuthEvent, AuthFlow, AuthMethodInfo, AuthOutcome, BrowserPolicy, OpenerCommand, first_url, open_url, authenticate as authenticate_acp};` — `authenticate` alone at the crate root would shadow the trait method's name in reading; the `plan_install`/`resolve_tools` precedent (`lib.rs:114-116`, `:141-143`). T1 exports the types; T3 adds `authenticate_acp`; T5 adds `OpenerCommand`, `first_url`, `open_url`.

### B.3 T2 — `crates/htui-agent/src/acp/auth.rs` (D12)

```rust
//! The wire half of a login: `initialize`, the live method list, one `authenticate` or `logout`,
//! the child killed on every exit — `handshake.rs` with two more exits (the human's cancel and
//! choice), and the second `send_request` site outside `open_session`. Every SDK type the flow
//! needs is spoken here and nowhere else in `auth/`.

use agent_client_protocol::schema::v1::{AuthMethod, AuthenticateRequest, InitializeRequest, InitializeResponse, LogoutRequest};
// + ProtocolVersion, Agent, ByteStreams, Client, ConnectionTo, oneshot, select, compat — as handshake.rs:20-31

/// What the wire needs of an [`AuthFlow`]: no cwd (already spawned), no idle (run.rs's), no
/// browser (already applied). `cancel` is the flow's **child** token when run.rs owns the clock.
#[derive(Debug)]
pub struct WireFlow {
    pub events: mpsc::UnboundedSender<AuthEvent>,
    pub choice: oneshot::Receiver<AuthChoice>,
    pub cancel: CancellationToken,
    /// Bounds `initialize` only (`acp::HANDSHAKE_TIMEOUT` in production, P-7). Nothing bounds
    /// the call but the token (D3, D13).
    pub handshake_timeout: Duration,
}

/// The flow over `io`, killing `io.child` on **every** exit (D12, H-1).
///
/// Shape: `ChildGuard` in this frame (`handshake.rs:102`); the connection task owns the streams
/// only; the foreground future sends `initialize` with `block_task()`, sends `Methods`, then
/// `select!`s `choice` against `cancel.cancelled()`, and on a choice sends the call with
/// `block_task()` — the two `send_request`s and no other; the outcome goes back on a `oneshot`.
/// The outer frame `select!`s that `oneshot` against `cancel.cancelled()`, so a cancel reaches the
/// kill even while the foreground future is parked on a request the agent will never answer.
/// Then, whatever happened: `guard.kill_and_reap().await; task.abort();`.
///
/// # Errors
/// `Transport`: `initialize failed: {err}` (+ tail), `the agent did not complete its handshake
/// within {timeout:?}`, `the agent ended before answering {initialize|authenticate|logout}` (the
/// actor dropped the foreground future — the sender dropped unused). A JSON-RPC error **to the
/// call** is `Ok(Refused)`, not `Err` (D5).
pub async fn run(io: AcpIo, settings: &AcpSettings, flow: WireFlow) -> Result<AuthOutcome>;

/// `authMethods` split by kind: `Terminal(_)` → hidden; `Agent(_)` → methods; the `#[non_exhaustive]`
/// wildcard is written `_ => methods.push(..)` with the comment: "unreachable for wire data —
/// `Agent` is `#[serde(untagged)]` and swallows every unknown `type`; a future *Rust* variant
/// lands here, and offering it is the schema's own default (plan D11)". `logout` =
/// `init.agent_capabilities.auth.logout.is_some()` (`AgentAuthCapabilities` is not an `Option`,
/// schema `agent.rs:3814`, `:464-472`).
fn methods_of(init: &InitializeResponse) -> (Vec<AuthMethodInfo>, bool, Vec<AuthMethodInfo>);

/// Foreground-future result, sent on the `oneshot`.
enum Wire { Answered(AuthOutcome), Cancelled }
```

Foreground future, in order: `initialize` (`InitializeRequest::new(ProtocolVersion::V1).client_capabilities(client::client_capabilities(&settings.client_capabilities)).client_info(client::client_info())`, `block_task()`, under `timeout(handshake_timeout, ..)` **inside** the future — `Err` on timeout is `Transport("the agent did not complete its handshake within ..")`); `events.send(Methods { .. })` (a closed receiver is not an error: the caller is gone and the cancel will follow); `select! { c = &mut choice => .., () = cancel.cancelled() => Wire::Cancelled }`; `Ok(AuthChoice::Method(id))` → `cx.send_request(AuthenticateRequest::new(id.as_str())).block_task().await` — `Ok(_)` → `Completed { call: Authenticate(id) }`, `Err(err)` → `Refused { call, message }` where `message = format!("{err}")` plus the stderr tail — **the tail is read in the outer frame** from the guard (the future has no guard), so the future sends `Refused { message: format!("{err}") }` and the outer frame appends `\n{tail}` when non-empty (the `handshake_error` shape, `handshake.rs:163-170`); `Ok(AuthChoice::Logout)` → `LogoutRequest::new()` likewise; `Err(RecvError)` → `Declined`. Outer: `select! { r = answer_rx => match r { Ok(Answered(o)) => Ok(o), Ok(Cancelled) => Ok(Cancelled), Err(_) => Err(Transport("the agent ended before answering ..")) }, () = cancel.cancelled() => Ok(Cancelled) }` — `AuthMethodId: From<&str>` (schema `AuthenticateRequest::new(impl Into<AuthMethodId>)`, `agent.rs:312`) — **UNVERIFIED — implementer must check** the exact `Into` impls; `AuthMethodId(id.into())` otherwise.

`acp/client.rs` T2 test (inline, beside `:173-195`): `the_advertised_capabilities_claim_neither_terminal_auth_nor_elicitation` — `serde_json::to_value(client_capabilities(&ClientCapabilities { terminal: true, elicitation: true, .. }))`: `["auth"]["terminal"]` is absent or `false`, and `["elicitation"]` is absent or `null` — asserted on the **serialised** value so the test does not depend on the schema's field visibility.

### B.4 T3 — `acp/mod.rs` additions and `auth/run.rs`

```rust
// acp/mod.rs
/// What this session would spawn, before spawning it (the body of today's `launch_for`,
/// `:359-386`, minus the `spec.env` extension; behaviour otherwise unchanged).
pub async fn launch_in(&self, cwd: &Path) -> Result<ResolvedLaunch>;
/// `launch_in(&spec.cwd)` then `resolved.env.extend(spec.env.clone())` — as today.
pub async fn launch_for(&self, spec: &SessionSpec) -> Result<ResolvedLaunch>;

/// Where a login's bytes come from (P-1). `pub(crate)`: the one thing outside `acp/` that reads
/// `IoSource` is `auth::run`, and it reads it through this.
pub(crate) enum AuthSource {
    /// The row's launch, resolved for `cwd`, env as the row and the probe recorded it.
    Launch(ResolvedLaunch),
    /// A prepared duplex (`test-support`): no process, no policy, no tap.
    #[cfg(feature = "test-support")]
    Prepared(AcpIo),
}
impl core::fmt::Debug for AuthSource { /* "Launch" / "Prepared", never the env */ }
impl AcpDriver {
    pub(crate) async fn auth_source(&self, cwd: &Path) -> Result<AuthSource>;   // Spawn → launch_in; Prepared → the slot taken (":410-419"'s two errors)
    pub(crate) fn acp_settings(&self) -> &AcpSettings { &self.settings.acp }
}
impl AgentDriver for AcpDriver {
    fn authenticate<'a>(&'a self, flow: AuthFlow) -> DriverFuture<'a, AuthOutcome> {
        Box::pin(crate::auth::run::authenticate(self, flow))
    }
}

// auth/run.rs
/// The end-to-end operation `AcpDriver::authenticate` delegates to (D9, D13, D14, D15, D16).
///
/// 1. `driver.auth_source(&flow.cwd)`.
/// 2. `Launch(mut launch)` → `flow.browser.apply(&mut launch)` (T5; a no-op at T3) →
///    `launch::spawn(&launch, &flow.cwd)` → `stderr = Some(spawned.tap_stderr())` (T5, **before**
///    `from_spawned`, which moves the child) → `AcpIo::from_spawned(spawned)`.
///    `Prepared(io)` → `(io, None)`.
/// 3. `let clock = flow.cancel.child_token();` an inner `oneshot` for the choice.
/// 4. `wire = acp::auth::run(io, driver.acp_settings(), WireFlow { events: flow.events.clone(),
///    choice: inner_rx, cancel: clock.clone(), handshake_timeout: HANDSHAKE_TIMEOUT })`, pinned.
/// 5. Loop `select!` (biased towards `wire`): `wire` done → break; a stderr line → `Line`, then
///    `first_url` → `Url` if new (`HashSet<String>` seen); reset the idle sleep; `flow.choice` →
///    forward on `inner_tx` (once), reset; `sleep(flow.idle)` elapsed → `clock.cancel()`,
///    `idle_fired = Some(flow.idle)`; the stderr receiver ending → stop polling it (not an exit:
///    a duplex has none).
/// 6. `Ok(Cancelled)` with `idle_fired` → `Ok(Idle { after })`; everything else unchanged.
///
/// The `ChildGuard` lives in `acp::auth::run`'s frame, so dropping *this* future drops that one
/// and the guard's `Drop` signals the tree (H-1). No guard here: one owner.
///
/// # Errors
/// `launch_in`'s (`Unresolved`, `Transport`), `launch::spawn`'s (`Spawn` naming the command),
/// `acp::auth::run`'s.
pub async fn authenticate(driver: &AcpDriver, flow: AuthFlow) -> Result<AuthOutcome>;
```

At T3 `run.rs` has steps 1, 2 (without `apply`/tap), 4 with `flow.cancel.clone()` as the token, and returns the wire's outcome; T5 rewrites step 2–6 as above. `AuthFlow.browser` is carried and unread at T3 (a `let _ = flow.browser;` with the comment "T5 reads it").

### B.5 T4 — `launch.rs` (D14, P-9)

```rust
/// The child's stderr as `spawn` keeps it: a bounded tail and, when a caller asked, a live tap.
#[derive(Debug, Default)]
struct StderrTail { lines: VecDeque<String>, tap: Option<mpsc::UnboundedSender<String>> }

pub struct Spawned { child, stdin, stdout, stderr_tail: Arc<Mutex<StderrTail>>, pub job_object }

// the reader task (:800-809) becomes, per line, under the one lock:
//   if tail.lines.len() == STDERR_TAIL_LINES { tail.lines.pop_front(); }
//   tail.lines.push_back(line.clone());
//   if let Some(tx) = &tail.tap && tx.send(line).is_err() { tail.tap = None; }
// `stderr_tail()` (:563-568): `.map(|tail| tail.lines.iter().cloned().collect())`.

impl Spawned {
    /// Every stderr line from now on — and first, **replayed under the same lock the reader
    /// holds**, every line already in the tail, so tapping after `spawn` loses and duplicates
    /// nothing (plan D14: the URL can arrive during `initialize`). Lines only (`lines()`), lossy
    /// UTF-8, as the tail. One tap per child: a second call replaces the first, whose receiver
    /// then ends. The tail keeps its 64-line bound whatever the tap holds.
    pub fn tap_stderr(&mut self) -> mpsc::UnboundedReceiver<String>;
}
```

`ChildGuard` gains nothing: the tap is taken on the `Spawned` before `AcpIo::from_spawned` moves it (B.4 step 2).

### B.6 T5 — `auth/url.rs` (D15)

```rust
/// The first `http://` or `https://` substring, extended to the first whitespace or one of
/// `<` `>` `"` `'` `` ` ``, then trailing `.` `,` `;` `:` `)` `]` `}` stripped. Never parsed,
/// never validated beyond the scheme: `open_url` refuses anything else again before spawning.
#[must_use] pub fn first_url(line: &str) -> Option<String>;
```

Implementation note: `line.find("http://").into_iter().chain(line.find("https://")).min()` — `https://` contains no `http://` prefix, so the two finds are independent and the smaller index wins; the scan is `char_indices` from there.

### B.7 T5 — `auth/browser.rs` (D16, D17, D22)

```rust
/// The variable an adapter's opener reads before `xdg-open`/`open` (the observed hijack, D16).
const BROWSER_VAR: &str = "BROWSER";
/// D17's Windows spelling: the URL travels in the child's environment, never on a command line.
#[cfg(windows)] const OPEN_URL_VAR: &str = "HTUI_OPEN_URL";

impl BrowserPolicy {
    /// `Neutralised`: `launch.env.insert("BROWSER", neutraliser())`. `Inherit`: nothing.
    /// `DISPLAY`/`WAYLAND_DISPLAY` untouched either way. Applied to the auth spawn's own clone of
    /// the launch and never recorded (H-11).
    pub fn apply(self, launch: &mut ResolvedLaunch);
}
/// unix: `which::which("true")` as an absolute path (a handful of `stat`s, on the auth task, not
/// the loop), else the literal `true`. windows: `cmd.exe /c exit 0` — written here, MOD-16's.
fn neutraliser() -> String;

/// D17: refuses any scheme but `http`/`https` **before** spawning; spawns a plain
/// `tokio::process::Command` (never `launch::spawn`: the opener must not be a process-group
/// leader `htui` kills, H-4) with `stdin/stdout/stderr = Stdio::null()` (H-3); hands the child
/// to `tokio::spawn(async move { let _ = child.wait().await; })` so it is reaped; returns at once.
///
/// `Platform`: linux `xdg-open <url>`; macos `open <url>`; windows `powershell.exe -NoProfile
/// -NonInteractive -Command "Start-Process -FilePath $env:HTUI_OPEN_URL"` with
/// `.env(OPEN_URL_VAR, url)` and `.creation_flags(CREATE_NO_WINDOW)` (the `open 5.4.3`
/// spelling, borrowed; **UNVERIFIED — implementer must check** whether `launch.rs`'s
/// `CREATE_NO_WINDOW` is reachable — reuse it if `pub(crate)`, else `const CREATE_NO_WINDOW: u32
/// = 0x0800_0000;` locally). `Custom(program)`: `<program> <url>`.
///
/// # Errors
/// `DriverError::Transport("only http and https links are opened; refused `{scheme}`")` before
/// any spawn; `DriverError::Spawn(format!("`{program}`: {err}"))` from the OS.
pub async fn open_url(url: &str, opener: &OpenerCommand) -> Result<()>;
/// `url.split_once("://").map(|(s, _)| s.to_ascii_lowercase())`, accepted for `http`/`https`.
fn scheme_of(url: &str) -> Option<String>;
```

The `cfg(unix)`/`cfg(windows)` split lives in `open_url`'s `Platform` arm and `neutraliser()` only.

### B.8 T6 — `crates/htui/src/store_worker.rs` (D18)

```rust
pub enum StoreRequest {
    /* existing; after InstallCancel (:159) */
    /// Start a login for one registry row (MOD-21 D18). Served by the agent runtime's own task:
    /// answered with [`AuthFrame::Methods`] at this `seq`, then every frame until the choice, or
    /// with [`StoreReply::Failed`] before anything is spawned.
    AuthStart { agent_id: AgentId },
    /// The user chose from the live method list. Every later frame — `Line`, `Url`, `Done`,
    /// `Refused`, `Cancelled`, `Failed` — carries **this** `seq` (the MOD-20 plan/confirm split:
    /// `App::is_fresh` keys on request kind, `app/state.rs:290-294`).
    AuthChoose { choice: AuthChoice },
    /// Open the link the pane shows, through `htui`'s own opener (D17). Answered once at its own
    /// `seq`: [`AuthFrame::Opened`] or [`StoreReply::Failed`].
    AuthOpen { url: String },
    /// Stop the running login. Answered [`AuthFrame::Cancelling`] at once; the stream ends
    /// [`AuthFrame::Cancelled`] at its own `seq`.
    AuthCancel,
}
// name(): "auth_start" | "auth_choose" | "auth_open" | "auth_cancel"

pub enum StoreReply { /* existing; after Install (:240) */
    /// One frame of a login (MOD-21 D18), the [`InstallFrame`] shape.
    Auth(AuthFrame),
}

/// One frame of a login stream. Text, ids and a status: nothing here can hold a credential.
#[derive(Debug, Clone)]
pub enum AuthFrame {
    Methods { methods: Vec<AuthMethodInfo>, logout: bool, hidden: Vec<AuthMethodInfo> },
    Line(String),
    Url(String),
    /// The opener was spawned (not: the browser opened).
    Opened,
    /// The call returned and the row was **re-probed and written first**; this is the probe's
    /// verdict, never the flow's (D6, `R-AGT-6`).
    Done { call: AuthCall, status: ProbeStatus },
    /// The agent refused the call in its own words (D5).
    Refused { message: String },
    Cancelling,
    Cancelled,
    /// The flow went silent for this long and was killed (D13).
    Idle { after: Duration },
    /// The flow died (spawn, transport, or the row write).
    Failed { message: String },
}
```

`Done` carries `call` as well as `status` (**decided here**): the section's notice says `logged in: ready` vs `logged out: unauthenticated` (D20) and cannot otherwise know which. `Idle` is its own frame (**decided here**) so the notice can say "no activity for 10m; login cancelled" rather than "cancelled" — H-9. `try_serve` (`:426-436`) and the loop arm (`:574-591`): the four join both lists; `testkit.rs:186-194` the same four.

### B.9 T6 — `crates/htui/src/agent_worker.rs` (D18, D19)

```rust
/// What the worker asks a live login to do (the `ChatCommand` shape, `:89-111`).
#[derive(Debug)]
pub enum AuthCommand {
    Choose { choice: AuthChoice, reply: ReplyAddr },
    Open { url: String, reply: ReplyAddr },
}

/// The one login this runtime allows at a time (MOD-21 D18, D19).
pub struct LiveAuth {
    agent_id: AgentId,
    /// Tripped by `AuthCancel`, by shutdown, and (as its child) by the idle clock.
    cancel: CancellationToken,
    /// Into `run_auth`. Closed = the flow has ended (H-6).
    commands: mpsc::UnboundedSender<AuthCommand>,
    task: JoinHandle<()>,
    /// D19: the row's `(agent_id, box_id)` re-probe claim, held for the flow's whole life so a chat
    /// started mid-login cannot re-probe the row under it (H-16). Released by `Drop` at the sweep.
    _claim: ReprobeClaim,
}
// hand-written Debug: agent_id, finished, closed — never a line or a URL (H-5)

pub struct AgentRuntime { /* existing */ auth: Option<LiveAuth>, opener: OpenerCommand }
impl AgentRuntime {
    // new(): auth: None, opener: OpenerCommand::Platform
    /// P-5: the injected opener; a test's script records the URL and sleeps.
    #[must_use] pub fn with_opener(mut self, opener: OpenerCommand) -> Self;
    /// Whether a login is running; tests.
    #[must_use] pub fn auth_running(&self) -> bool;

    // serve(): after `self.install.take_if(..)` (:380): `self.auth.take_if(|live| live.task.is_finished());`
    //   AuthStart { agent_id } => match self.auth_start(backend, replies, addr, *agent_id).await { Ok(s) => s, Err(e) => Served::Reply(failed("auth_start", &e)) }
    //   AuthChoose { choice }  => self.auth_command("auth_choose", AuthCommand::Choose { choice: choice.clone(), reply: addr })
    //   AuthOpen { url }       => self.auth_command("auth_open", AuthCommand::Open { url: url.clone(), reply: addr })
    //   AuthCancel             => self.auth_cancel()

    /// D18's refusals, P-10's order, **before anything is spawned**: `recording_writer` (taken —
    /// the task writes the row) → `registered_box` → `claim_is_free` → `reprobe_claims.claim((agent_id, box_id))`
    /// or `Backend("a re-probe is running for agent {id}; try again in a moment")` → `row_for` →
    /// `caps_for(&summary.agent).authenticate` or `Backend(DriverError::Unsupported("authenticate").to_string())`
    /// → `ProbeSnapshot::from_row(on_box)` whose `handshake.auth_methods` is non-empty or
    /// `Backend(format!("`{name}` advertises no authentication methods"))` → `factory.driver_for(&agent, on_box.as_ref())`
    /// → `current_dir`. Then `tokio::spawn(run_auth(AuthArgs { .. }))`, `self.auth = Some(LiveAuth { .. })`, `Ok(Served::Deferred)`.
    async fn auth_start(&mut self, backend: &Backend, replies: &Sender, addr: ReplyAddr, agent_id: AgentId) -> Result<Served, StoreError>;
    /// The `command` shape (`:711-726`): no flow → `Failed { request, "no login is running" }`;
    /// a closed channel → `self.auth = None`, `Failed { request, "this login has ended" }`; else `Deferred`.
    fn auth_command(&mut self, request: &'static str, command: AuthCommand) -> Served;
    /// `install_cancel`'s shape (`:670-679`): `live.cancel.cancel()`, `Reply(Auth(Cancelling))`; none → `Failed { "auth_cancel", "no login is running" }`.
    fn auth_cancel(&mut self) -> Served;

    // probe(): before the install check (:519-524), the same for `self.auth`: "a login is running for agent {}; probe once it has finished".
    // claim_is_free(): before the install check (:690-695): `self.auth` → "a login is already running for agent {}".
    // finish_background(): after the install arm (:335-342), the identical arm over `self.auth.take()` (P-2).
    // shutdown(): after the install arm (:485-497), the identical arm: `live.cancel.cancel()`, `timeout(grace * 2, live.task)`, warn "a login did not end within the grace window"; then abort as today.
}

/// Everything the login task owns (the `InstallArgs` rule, `:1284-1298`).
struct AuthArgs {
    driver: Box<dyn AgentDriver>, agent: Agent, existing: Option<AgentBox>, box_id: BoxId,
    writer: Writer, cwd: PathBuf, cancel: CancellationToken,
    commands: mpsc::UnboundedReceiver<AuthCommand>, opener: OpenerCommand, frames: Frames,
}
// Debug: writer.label(), agent.name, finish_non_exhaustive — never the env (H-5)

/// The flow, in its own task. Frames before the choice go to the `AuthStart` address, every frame
/// after it to the `AuthChoose` address (P-3: a local `addr`). **The row is written before `Done`**
/// (`run_install`'s rule, `:1376-1380`). The re-probe is the **only** write, and only on `Completed`.
///
///  1. `(events_tx, events_rx)`, `(choice_tx, choice_rx)`; `let mut choice_tx = Some(choice_tx)`.
///  2. `flow = driver.authenticate(AuthFlow { cwd, events: events_tx, choice: choice_rx, cancel: cancel.clone(), idle: AUTH_IDLE_CAP, browser: Neutralised })`, pinned.
///  3. Loop `select!`: `out = &mut flow` → break; `Some(ev) = events_rx.recv()` → `Methods`/`Line`/`Url` at `addr`;
///     `Some(cmd) = commands.recv()` → `Choose { choice, reply }`: `choice_tx.take()` → `addr = reply; tx.send(choice)`;
///     already taken → `Failed { "auth_choose", "a method was already chosen" }` at `reply` (H-6);
///     `Open { url, reply }` → `open_url(&url, &opener).await` → `Auth(Opened)` or `Failed { "auth_open", err }` at `reply`.
///  4. Drain `events_rx` (the sender is gone once `flow` returned).
///  5. `Ok(Completed { call })` → `ProbeContext { env: ProbeEnv::host(cwd).without_versions(), now }`, `probe_agent(&agent, box_id, existing.as_ref(), &ctx, &SpawnTier2::default())`:
///     `Row(row)` → `writer.upsert_agent_box(&row)` → `Done { call, status: ProbeSnapshot::from_row(&row).status }` or `Failed { message }`;
///     `Kept { .. }` → `Done { call, status: existing's snapshot status or Unauthenticated }` (a hand-written row is left alone, D51).
///     `Ok(Refused { message, .. })` → `Refused { message }`; `Ok(Cancelled)` → `Cancelled`; `Ok(Idle { after })` → `Idle { after }`;
///     `Ok(Declined)` → `Cancelled` (the pane cancelled before choosing, or the runtime dropped the sender); `Err(e)` → `Failed { message: e.to_string() }`.
///  6. `commands` dropped with the task: the runtime's next `auth_command` hears "this login has ended".
async fn run_auth(args: AuthArgs);
```

`ProbeSnapshot::from_row` (`probe.rs:1087-1106`, `status` at `:1106`) is how the status is read back off the written row, never set here (`R-AGT-6`). `probe.resolved` on that row is the re-probe's own resolution and carries no `BROWSER` (H-11; T6 asserts it).

### B.10 T7 — `crates/htui/src/ui/tabs/settings/agents.rs` (D20)

```rust
const HINT_IDLE: &str = "j/k select \u{b7} r probe \u{b7} i install \u{b7} a authenticate";  // the four snapshots move
const HINT_CHOOSING: &str = "j/k choose \u{b7} Enter select \u{b7} Esc cancel";
const HINT_AUTH_RUNNING: &str = "o open link \u{b7} x cancel";
const STARTING: &str = "starting\u{2026}";  const CHOOSE: &str = "choose a method";
const LOGGING_IN: &str = "logging in\u{2026}"; const LOGGING_OUT: &str = "logging out\u{2026}";
const LOGOUT_ROW: &str = "log out";  const NO_LINK: &str = "no link yet";
/// D20: the last lines of the stream the pane shows.
const AUTH_PANE_LINES: usize = 6;

#[derive(Debug, Default)]
enum AuthState {
    #[default] Idle,
    Starting { agent_id: AgentId },
    Choosing { agent_id: AgentId, methods: Vec<AuthMethodInfo>, logout: bool, hidden: usize, cursor: usize },
    Running { agent_id: AgentId, call: AuthCall, lines: VecDeque<String>, url: Option<String>, cancelling: bool },
}
pub struct AgentsSection { /* existing */ auth: AuthState }
```

Keys (`on_key`), after the tab's `h`/`l`/`[`/`]`/arrows (`settings/mod.rs:197-213`) and the global `q ? digits ctrl-c -`; `Pending` (install) still answers first:

| State | Key | Effect |
|---|---|---|
| `Choosing` | `j`/`k` | cursor over `methods.len() + usize::from(logout)` rows, no wrap; `Consumed` |
| `Choosing` | `Enter` | `AuthChoice::Method(methods[cursor].id)` or `Logout` (the last row); `auth = Running { call, lines: empty, url: None, cancelling: false }`; `ctx.request(AuthChoose { choice })`; `Consumed` |
| `Choosing` | `n`/`Esc` | `ctx.request(AuthCancel)`; `auth` stays `Choosing` with the cell reading `cancelling…` until the frame (`cancelling` is on `Running` only, so `Choosing` moves to `Running { call: Authenticate(String::new()), cancelling: true, .. }` — **decided here**: one state carries the flag); `Consumed` |
| `Choosing` | `i`/`r`/`a`/`x` | `Consumed` (the section's own keys, the `answer_consent` rule, `:259-268`); everything else `Pass` |
| `Idle` | `a` | refusals as `Action::Error`, from the document, no request: install in flight / `probing` / `auth` not `Idle` ("a login is already running"); no row; `!caps_for(&summary.agent).authenticate` → ``"`{name}` is a `{transport}` agent; it has no `authenticate` call"``; `on_box` `None` or `probe.status` not `unauthenticated`/`ready` → ``"probe `{name}` first"``; `probe.handshake.auth_methods` empty → ``"`{name}` advertises no authentication methods"`` (read with `ProbeSnapshot::from_row`, as `on_box_cell` reads `status`); else `auth = Starting`, `notice = None`, `ctx.request(AuthStart { agent_id })` |
| `Starting`/`Running` | `x` | `cancelling = true` (Running); `ctx.request(AuthCancel)` |
| `Running` | `o` | `url` `Some` → `ctx.request(AuthOpen { url })`; `None` → `Action::Error(NO_LINK)` |
| `Starting`/`Choosing`/`Running` | `r`, `i` | `Action::Error("a login is running; …afterwards")` |

`on_reply(Auth(frame))` → `on_auth_frame`: `Methods` → `Choosing { hidden: hidden.len(), cursor: 0, .. }`; `Line(l)` → push, `truncate` front to `AUTH_PANE_LINES`; `Url(u)` → `url = Some(u)`; `Opened` → `notice = Some("link opened")`; `Done { call, status }` → `Idle`, `notice = Some(format!("{}: {status}", logged in|logged out))`, `ctx.request(StoreRequest::Agents)`; `Refused { message }` → `Idle`, notice = message; `Cancelling` → `cancelling = true`; `Cancelled` → `Idle`, `"login cancelled"`; `Idle { after }` → `Idle`, `format!("no activity for {after:?}; login cancelled")`; `Failed { message }` → `Idle`, notice. `Failed { request: "auth_start" | "auth_choose" | "auth_cancel" }` → `Idle` (the shell owns the sentence); `"auth_open"` → state unchanged. `Agents(..)` **unchanged** (H-7).

`on_box_cell`: after `install_cell` and before `probing`, `auth_cell(agent_id)`: `Starting` → `STARTING`; `Choosing` → `CHOOSE`; `Running { cancelling: true }` → `CANCELLING`; `Running { call: Authenticate(_) }` → `LOGGING_IN`; `Logout` → `LOGGING_OUT`. `pane()`: `Choosing` → blank line, one line per method `{name} — {description}` (cursor row in `theme.accent`), `log out` last when advertised, then dim `"{hidden} method(s) need a terminal htui does not provide"` when `hidden > 0`; `Running` → the last six `lines` and `link: {url}` when known. `hint()`: `Choosing` → `HINT_CHOOSING`; `Starting`/`Running` → `HINT_AUTH_RUNNING`; else as today. `install_in_flight` is joined by `auth_in_flight()` in `begin_install` and the `r` arm.

---

## C. Fixtures (all in the test file that uses them; `R-AGT-5`: made-up ids only)

### C.1 The scripted duplex agent (`crates/htui-agent/tests/auth.rs`, T2)

```rust
enum Answer { Ok, Error { code: i64, message: &'static str }, Never }
struct Script { initialize: Value, authenticate: Answer, logout: Answer, log: Arc<Mutex<Vec<Value>>> }
/// `scripted_initialize` (`tests/probe.rs:1251-1270`) generalised: raw JSON-RPC lines, no SDK type.
/// Every request is pushed to `log`; `initialize` is answered with `script.initialize`;
/// `authenticate`/`logout` per `Answer` (`Never` reads on without answering; `Error` sends
/// `{"jsonrpc":"2.0","id":..,"error":{"code":..,"message":..}}`); any other method → `-32601`.
async fn scripted_agent(stream: DuplexStream, script: Script);
/// `tests/fixtures/agy_acp_handshake.json`-shaped `initialize` result with `authMethods` carrying
/// `name`/`description`: `[{"id":"m-one","name":"One","description":"first"},{"id":"m-two","name":"Two"}]`,
/// plus `{"type":"terminal","id":"tty","name":"Terminal"}` or `{"type":"future-kind",..}` per case;
/// `agentCapabilities.auth.logout: {}` toggled per case.
fn init_result(methods: Value, logout: bool) -> Value;
/// `AcpIo` over `tokio::io::duplex(DUPLEX_BYTES)` with the agent spawned (`DuplexTier2`'s shape, `:1304-1327`).
fn duplex_io(script: Script) -> AcpIo;
/// `WireFlow` + the caller's ends: `(flow, events_rx, choice_tx, cancel)`.
fn wire_flow(timeout: Duration) -> (WireFlow, UnboundedReceiver<AuthEvent>, oneshot::Sender<AuthChoice>, CancellationToken);
```

### C.2 The `sh` fixture (T2 real-process cases, T3, T5, T6, T7)

Written by `executable(path, contents)` (copied from `tests/probe.rs:77-85`) and **run as `/bin/sh <path>`** (H-12). One script, behaviour by environment:

```sh
#!/bin/sh
# FIXTURE_DIR: where pid/argv/credential go.  FIXTURE_KEY: D5 — unset ⇒ authenticate is refused -32602.
# FIXTURE_HOLD=1 ⇒ never answer authenticate.  FIXTURE_URL: printed to stderr before answering.
# FIXTURE_CRED=1 ⇒ create $FIXTURE_DIR/credential on authenticate; delete it on logout.
# FIXTURE_HIJACK=1 ⇒ on authenticate: "$BROWSER" "$FIXTURE_URL" if BROWSER is set, else the alt-screen
#   sequence to STDOUT (the observed hijack, PRD Evidence).  FIXTURE_TWICE=1 ⇒ print the URL line twice.
echo $$ > "$FIXTURE_DIR/pid"; printf '%s\n' "$@" > "$FIXTURE_DIR/argv"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$FIXTURE_INIT" ;;
    *'"method":"authenticate"'*)
      [ -n "$FIXTURE_URL" ] && echo "Open the following link to log in: $FIXTURE_URL" >&2
      [ -n "$FIXTURE_TWICE" ] && echo "again: $FIXTURE_URL" >&2
      if [ -n "$FIXTURE_HIJACK" ]; then
        if [ -n "$BROWSER" ]; then "$BROWSER" "$FIXTURE_URL"; else printf '\033[?1049h\033[1;24r Acquisizione di %s\n' "$FIXTURE_URL"; fi; fi
      [ -n "$FIXTURE_HOLD" ] && sleep 3600
      if [ -z "$FIXTURE_KEY" ]; then printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"The FIXTURE_KEY environment variable must be set in the environment the ACP server is launched from."}}\n' "$id"
      else [ -n "$FIXTURE_CRED" ] && : > "$FIXTURE_DIR/credential"; printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"; fi ;;
    *'"method":"logout"'*) rm -f "$FIXTURE_DIR/credential"; printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
  esac
done
```

`FIXTURE_INIT` is the `initialize` result JSON (one line, no spaces the `case` glob could trip on); the `--uid=`-shaped argument T3 asserts is any made-up `--fixture-arg=1` in `args` after the script path. The `pid` file is what T6's "nothing spawned" cases read (absent ⇒ never ran); the `credential` file is what the row's `credential.files` names (P-6). The hijack fixture's `\033[?1049h` on **stdout** is what makes the `Inherit` control fail with `Err(Transport)` (a decode error from the SDK) and the `Neutralised` case pass — `BROWSER=/bin/true` swallows the URL and exits 0.

### C.3 Row and runtime shapes (T6, T7)

`Agent { transport: Acp, launch: json!({ "command": "/bin/sh", "args": [script], "env": { "FIXTURE_DIR": tmp, "FIXTURE_INIT": init, "FIXTURE_KEY": "set", "FIXTURE_CRED": "1", "FIXTURE_URL": "https://example.invalid/login?x=1&y=%2F" }, "discovery": { "tools": {}, "handshake": true, "credential": { "env": [], "files": [tmp/credential] } } }), settings: json!({}) }`; the box row seeded through `MemStore::upsert_agent_box` with `probe: json!({ "status": "unauthenticated", "handshake": { "auth_methods": ["m-one"], .. } })` (a `probed_row`-shaped snapshot, `tests/settings.rs:180-200`; the T6 `Done` case lets the flow's own re-probe replace it). Runtime: `AgentRuntime::new(DriverFactory::with_acp()).with_grace(Duration::ZERO).with_opener(OpenerCommand::Custom(recorder))` where `recorder` is a second `sh` script `echo "$1" >> "$FIXTURE_DIR/opened"; sleep 30`.

---

## D. Data flows and ownership

### D.1 A successful login

```
Settings (UI task)                         worker loop (select! arm)                         run_auth task                             htui-agent frames
a on row (Idle → Starting; cell "starting…")
  ctx.request(AuthStart{agent_id}) ─seq n─▶ runtime.serve → auth_start:
                                            writer taken · box · claim_is_free · ReprobeClaim · row
                                            caps_for.authenticate · snapshot.auth_methods ≠ [] · driver_for
                                            spawn(run_auth(AuthArgs)); LiveAuth; Deferred → continue
                                                                                          driver.authenticate(AuthFlow)  ──▶ auth::run::authenticate:
                                                                                                                              auth_source → Launch(l); l.env[BROWSER]=/bin/true
                                                                                                                              spawn(l) → tap_stderr → AcpIo ──▶ acp::auth::run:
                                                                                                                                                                  guard = ChildGuard(child)
                                                                                                                                                                  task{ initialize (block_task) }
                                                                                          ◀── events: Methods{..}          ◀── (idle reset)                  ◀── events.send(Methods)
on_reply Auth(Methods) → Choosing; pane     ◀─ frames.reply(addr n, Auth(Methods))
Enter → Running{Authenticate(id)}; cell "logging in…"
  ctx.request(AuthChoose{Method(id)}) ─seq m─▶ auth_command → commands.send(Choose{choice, reply m}); Deferred
                                                                                          addr = m; choice_tx.send ──▶ inner_tx ──▶ select!{choice} → send_request(authenticate) (block_task)
                                                                                          ◀── Line("Open the following link …") ◀── tap ◀── stderr
                                                                                          ◀── Url(https://…)                (first_url, dedup)
on_reply Auth(Line), Auth(Url) → pane shows lines + "link: …"   ◀─ frames.reply(addr m, ..)
o → ctx.request(AuthOpen{url}) ─seq k─▶ auth_command → Open{url, reply k}; Deferred
                                                                                          open_url(url, opener): scheme ok → Command(null stdio).spawn → tokio::spawn(wait)
on_reply Auth(Opened) @k → notice "link opened"                    ◀─ frames.reply(k, Auth(Opened))
        … human completes OAuth in the browser; the adapter's loopback listener consumes the redirect …
                                                                                                                                                                  ◀── result {} → answer_tx.send(Answered(Completed))
                                                                                                                                                                  guard.kill_and_reap(); task.abort()   ← child gone, reaped
                                                                                          flow → Ok(Completed{Authenticate(id)})
                                                                                          probe_agent(agent, box_id, existing, host(cwd).without_versions, SpawnTier2) → Row(row)   (credential file now exists → ready)
                                                                                          writer.upsert_agent_box(row)                 ← the only write
on_reply Auth(Done{call, status: ready}) → Idle; notice "logged in: ready"; ctx.request(Agents) ◀─ frames.reply(m, Auth(Done))
 ─seq j─▶ try_serve Agents → on_reply Agents → cell = the probe's status ("1.1.1", i.e. ready)
```

### D.2 A cancelled login (`x` during the round trip)

```
x → cancelling=true; cell "cancelling…"; ctx.request(AuthCancel) ─seq c─▶ auth_cancel: live.cancel.cancel(); Reply(Auth(Cancelling)@c)
   flow.cancel tripped ⇒ clock (child token) tripped ⇒ acp::auth::run's outer select! takes `cancel.cancelled()` while the foreground
   future is parked on `authenticate` ⇒ guard.kill_and_reap().await (SIGKILL to the group, then wait) ⇒ task.abort() ⇒ Ok(Cancelled)
   run.rs: idle_fired None ⇒ Cancelled ⇒ run_auth: no probe, no write ⇒ frames.reply(m, Auth(Cancelled)) ⇒ Idle; notice "login cancelled"
   the row's probed_at is untouched (D6); the next serve() sweeps LiveAuth (task finished) and its Drop releases the ReprobeClaim
```
Shutdown is the same path started by `AgentRuntime::shutdown` (cancel, `grace * 2`, abort); the idle clock is the same path started by `clock.cancel()` and reported `Idle { after }`.

### D.3 A refused login (the live `-32602`)

```
Enter on the API-key method → AuthChoose ─m─▶ … send_request(authenticate) → Err(Error{-32602, "The X environment variable must be set …"})
   foreground: answer_tx.send(Answered(Refused{call, message: "{err}"})) ; outer: append "\n{guard.stderr_tail()}" if any
   guard.kill_and_reap(); task.abort() → Ok(Refused) → run_auth: no probe, no write → frames.reply(m, Auth(Refused{message}))
   → Idle; notice = the agent's sentence verbatim (H-10); the README names MOD-10 as where the variable gets injected
```

### D.4 Ownership and cancellation (H-1)

| Thing | Owner | Lifetime |
|---|---|---|
| The child (`Spawned`) | `ChildGuard` in `acp::auth::run`'s frame — **the only owner** | until `kill_and_reap` on any returning exit, or `Drop` (`start_kill` on the group, `launch.rs:738-754`) when the frame is dropped |
| The streams | the `tokio::spawn`ed connection task (`task`) | `task.abort()` after the kill on every returning exit; the timeout arm aborts too |
| The stderr tap receiver | `auth::run::authenticate`'s frame | dropped with the frame; the reader task sees `send` fail and clears the tap |
| `AuthFlow.cancel` | `LiveAuth.cancel` (runtime) with a clone in `AuthArgs` → `AuthFlow` | tripped by `auth_cancel`, `shutdown`, `finish_background`'s timeout arm |
| The idle child token | `auth::run::authenticate`'s frame (`cancel.child_token()`) | tripped by the idle sleep; a parent trip trips it too, so the wire half watches one token |
| `run_auth`'s `JoinHandle` | `LiveAuth.task` | swept at `serve` when finished; awaited by `finish_background`; aborted by `shutdown` after the cancel |
| `ReprobeClaim` | `LiveAuth._claim` | released by `Drop` at the sweep / `take()` in `finish_background`/`shutdown` |
| The opener child | nobody (`tokio::spawn`ed `wait()`) | reaped when it exits; never killed (H-4) |

Exit paths of `acp::auth::run`, each ending in `kill_and_reap` + `abort` before returning: answered (`Completed`/`Refused`), `Declined` (choice sender dropped), user cancel (outer or inner `select!`), idle cancel (same token, child), `initialize` timeout, actor dropped the future (`Err(RecvError)`). The one non-returning exit — the outer future dropped or the task aborted — drops the guard, whose `Drop` signals the group; tokio's orphan reaper reaps (`launch.rs:743-749`). On Windows the job object's kill-on-close is the equivalent, **unverified here** (D22, H-19).

---

## E. Build order and red-on-purpose

```
T1 (seam, types) ──┬── T2 (acp/auth.rs) ── T3 (AcpDriver::authenticate, run.rs) ──┐
                   └── T4 (tap; ∥ T1)  ─────────────────────────────────────────────┴── T5 (url, browser, idle, tap wired) ── T6 (runtime) ── T7 (Settings) ── T9
                                                                                                                              └── T8 (live; ∥ T6/T7)
```

| Pair | Intersection | Parallel? |
|---|---|---|
| T1 × T4 | ∅ | **yes** |
| T2 × T4, T3 × T4 | ∅ | **yes** (T3 needs T2 landed) |
| T1 × T5 | `auth/mod.rs`, `lib.rs` | serial |
| T2 × T3 | `acp/mod.rs`, `tests/auth.rs` | serial |
| T3 × T5 | `auth/run.rs`, `auth/mod.rs`, `tests/auth.rs` | serial |
| T6 × T7 | ∅ by file; T7 needs T6's types | serial |
| T8 × T6/T7 | ∅ | **yes**, after T5 |

Red on purpose: `every_registered_adapter_agrees_with_its_predicate` (T1) is **red for `acp`** from T1 until T3's `a_prepared_transport_runs_the_flow_without_a_process` lands the impl — the sequencing pin. `the_auth_flow_names_no_vendor_or_method` is green and vacuous until T2 writes the first `authenticate`. T3's `a_url_is_reported_once_per_flow`-class cases do not exist until T5 (`Line`/`Url` arrive with the tap). One commit per task: `feat(agent): DriverCaps.authenticate and the refusing default (MOD-21 D10)` · `feat(agent): the ACP login wire flow (D12)` · `feat(agent): AcpDriver::authenticate end to end (D9)` · `feat(agent): Spawned::tap_stderr (D14)` · `feat(agent): stderr hand-off, URL, browser policy, idle cap (D13, D15–D17)` · `feat(tui): login requests served off the worker loop (D18, D19)` · `feat(tui): Settings > a, the chooser and the stream pane (D20)` · `test(agent): agy logged in live through the app (D23)` · `docs(mod-21): close-out`.

`cargo fmt --all -- --check` before each commit; `cargo clippy --workspace --all-targets --all-features -- -D warnings` after T1, T3, T5, T6, T7; the Postgres line (`USERNAME=htui-ci HTUI_TEST_DATABASE_URL=…`) after T6 and T7. No Windows target line (TOOL-3).

---

## F. Tests per task (names are the plan's; shapes are this section's)

**T1** `tests/driver_contract.rs`: `a_driver_that_says_nothing_about_authenticate_refuses_it` (`StubDriver.authenticate(flow).await` is `Err(Unsupported("authenticate"))`, `caps().authenticate == false`; `flow` from a local `flow()` helper: tempdir cwd, fresh channels, `AUTH_IDLE_CAP`, `Neutralised`); `caps_for_an_acp_row_advertises_authenticate_and_a_cli_row_does_not`; `the_fake_transport_neither_claims_nor_answers_authenticate` (`test-support`); `every_registered_adapter_agrees_with_its_predicate` (for each id in `DriverFactory::with_acp().adapter_ids()` build a row of that transport, `driver_for`, assert `caps().authenticate == !matches!(driver.authenticate(flow()).await, Err(Unsupported(_)))` — the `acp` driver spawns from a row whose command is `/bin/false` at T1 and still answers `Unsupported`, which is the red; at T3 it answers `Err(Spawn)` and the case goes green because the assertion only asks about `Unsupported`); `DriverCaps::default().authenticate` is `false`. `tests/extensibility.rs`: `the_auth_flow_names_no_vendor_or_method` — `the_installer_names_no_vendor`'s body (`:276-308`) with the eight strings.

**T2** `tests/auth.rs` over C.1: `methods_carry_the_agents_own_name_and_description_in_order`; `a_terminal_typed_method_is_hidden_and_named_but_never_sent` (choose the terminal id anyway → the log has no `authenticate`; **decided here**: the wire refuses a choice whose id is in `hidden` with `Ok(Refused { message: "`{id}` needs a terminal htui does not provide" })` — the id is the fixture's, not a vendor's); `an_unknown_method_type_is_offered_as_an_agent_method`; `logout_is_offered_exactly_when_the_agent_advertises_it`; `choosing_a_method_sends_authenticate_with_that_id_and_completes` (`log[1]["method"] == "authenticate"`, `["params"]["methodId"] == id`); `a_json_rpc_error_is_refused_in_the_agents_own_words` (`Answer::Error { code: -32602, message: "The FIXTURE_KEY environment variable must be set…" }`; `Ok(Refused { message })` and `message.contains("FIXTURE_KEY environment variable")`); `choosing_logout_sends_logout`; `cancel_before_the_choice_ends_the_connection_and_answers_cancelled`; `cancel_during_authenticate_answers_cancelled` (`Answer::Never`, `cancel.cancel()` after `Methods`); `the_choice_sender_dropped_is_declined`; `an_agent_that_dies_after_initialize_is_a_transport_error_with_its_stderr` (C.2 with `FIXTURE_HOLD` replaced by `exit 3` after `echo boom >&2` — the real process, so the tail exists; `Err(Transport(m))` with `m.contains("boom")`); `initialize_is_still_bounded_by_the_handshake_timeout` (a duplex agent that never answers, `handshake_timeout: 100 ms`). Real-process (`cfg(target_os = "linux")`, `assert_not_running`/`assert_reaped` copied from `tests/acp_driver.rs:128-190`, pid from `Spawned::pid()` read **before** `from_spawned`): `no_child_survives_a_cancelled_flow`, `no_child_survives_a_refusal`, `no_child_survives_a_dropped_flow_future` (`tokio::time::timeout(Duration::ZERO, run(..))` after `Methods` arrived; then `assert_not_running` within `KILL_WINDOW` — signalled, not reaped, is the claim). `acp/client.rs` inline: `the_advertised_capabilities_claim_neither_terminal_auth_nor_elicitation`.

**T3** `tests/auth.rs`: `the_acp_driver_resolves_the_recorded_launch_for_the_flow` (`AcpDriver::from_row_with_probe` with an `on_box` whose `probe.resolved = { command: "/bin/sh", args: [script, "--fixture-arg=1"] }`; the `argv` file holds `--fixture-arg=1`); `a_stale_recording_falls_back_to_resolution` (`resolved.command` = a deleted path; the row's own `launch` runs); `a_prepared_transport_runs_the_flow_without_a_process` (`AcpDriver::over(duplex_io(..), &row, caps, Stamp::Wall)`, `Completed`); `a_spawn_failure_is_a_spawn_error_naming_the_command`; `the_flow_reads_a_variable_already_in_the_spawn_environment` (`launch.env.FIXTURE_KEY` present → `Completed`; absent → `Refused` naming `FIXTURE_KEY`).

**T4** `tests/launch.rs` beside `:461`, each ending in `wait()`: `a_tap_receives_every_stderr_line_in_order` (`sh -c 'echo a >&2; sleep 0.05; echo b >&2; sleep 0.05; echo c >&2'`); `a_tap_installed_late_replays_the_tail_first_then_streams`; `the_tail_is_unchanged_by_a_tap`; `past_sixty_four_lines_the_tap_has_them_all_and_the_tail_the_last_sixty_four` (`seq 1 100 >&2`); `a_second_tap_replaces_the_first`.

**T5** `tests/auth.rs`: URL — `first_url_finds_the_scheme_anywhere_in_the_line` (`Open the following link to log in: https://h.invalid/o?a=1&b=%2F#frag` whole); `first_url_stops_at_whitespace_and_brackets_and_strips_trailing_punctuation` (`<https://h/p>.` → `https://h/p`; `(https://h/p),` → `https://h/p`); `first_url_ignores_non_http_schemes`; `first_url_is_none_for_a_line_without_one`; `a_url_is_reported_once_per_flow` (`FIXTURE_TWICE`: two `Line`s, one `Url`). Browser — `neutralised_sets_browser_to_an_existing_executable_that_exits_zero` (`apply` on an empty launch; `Path::new(value).is_file()`; `tokio::process::Command::new(value).status().await.success()`); `the_policy_touches_the_auth_launch_only` (`driver.launch_in(cwd)` after a flow has no `BROWSER`; the flow's fixture wrote its env — `env > "$FIXTURE_DIR/env"` added to C.2 — and *that* has it); `neutralised_leaves_display_alone`. The regression pair over C.2 with `FIXTURE_HIJACK=1`: `with_the_policy_the_protocol_stream_survives_the_agents_browser` (`Completed`, a `Url` event); `without_the_policy_the_same_agent_corrupts_the_stream` (`BrowserPolicy::Inherit` and `BROWSER` removed from the launch env → `Err(Transport(_))`). Idle — `an_idle_flow_is_killed_after_the_cap_and_reported_idle` (`idle: 200 ms`, `FIXTURE_HOLD`, `Idle { after: 200 ms }`, `assert_not_running(pid)`); `a_stderr_line_resets_the_idle_clock` (the fixture prints ten lines 100 ms apart before answering, under a 200 ms cap → `Completed`). Opener — `open_url_refuses_a_non_http_scheme_before_spawning` (`Custom(recorder)`; `file:///etc/passwd` → `Err`, no `opened` file); `open_url_spawns_with_null_stdio_and_does_not_wait` (the recorder script also writes `[ -t 1 ]` → `tty`/`notty` to a file; returns within 100 ms, `opened` holds the URL, `notty`).

**T6** `agent_worker.rs` `mod tests` (the C.3 row; `serve` called directly as `:3410-3470` does; `envelope(seq, req)`): `a_start_is_refused_before_any_spawn_on_a_buffered_writer` (no `pid` file, `background_len() == 0`, `!auth_running()`); `a_start_on_a_cli_row_is_refused_by_the_predicate` (message contains "has no `authenticate` operation"); `a_start_on_a_row_with_no_auth_methods_is_refused_by_name`; `a_start_while_an_install_runs_is_refused`; `an_install_plan_while_a_login_runs_is_refused`; `a_probe_while_a_login_runs_is_refused`; `a_second_start_while_one_runs_is_refused`; `methods_arrive_at_the_start_seq_and_the_rest_at_the_choose_seq` (`rx.recv()` seqs: `Methods` at 1, `Line`/`Done` at 2); `a_choose_with_no_flow_pending_is_refused`; `done_carries_the_probes_status_and_the_row_was_written_first` (`FIXTURE_CRED=1`; on `Done { status: Ready }` `stored_box(&store, id)` already says `ready` — the reply channel is drained *after* the store read); `success_into_a_box_with_no_credential_still_reads_unauthenticated` (no `FIXTURE_CRED`); `a_refusal_writes_nothing_and_keeps_probed_at` (`FIXTURE_KEY` unset; `stored_box` before == after); `cancel_answers_cancelling_then_the_stream_ends_cancelled_and_no_child_survives` (`FIXTURE_HOLD`; pid from the `pid` file; `assert_not_running`); `shutdown_cancels_then_aborts_a_running_login_and_no_child_survives`; `the_agent_row_is_byte_identical_before_and_after_a_login` (`serde_json::to_vec(&agent)` twice); `no_frame_carries_anything_but_ids_text_and_status` (`FIXTURE_URL` carries `state=SENTINEL-TOKEN-VALUE`; the credential file holds `SECRET-SENTINEL`; every `AuthFrame`'s `{:?}` is checked for `SECRET-SENTINEL` — the URL, which the user already sees, is allowed and the `state` is not asserted absent; `LiveAuth`'s `{:?}` contains neither); `open_is_forwarded_to_the_live_flow_and_answered_at_its_own_seq` (`Opened` at seq 3; the recorder's `opened` file holds the URL); `a_flow_holds_the_reprobe_claim_for_its_row` (`runtime.reprobe_claims.claim(key)` is `None` while running, `Some` after the sweep); `logout_reprobes_and_reads_unauthenticated` (H-17: the credential exists before; `AuthChoice::Logout`; `Done { call: Logout, status: Unauthenticated }`). `store_worker.rs`: `the_loop_answers_other_requests_while_a_login_waits_for_a_human` (`:1358-1400`'s shape with `AuthStart` at 1 over `FIXTURE_HOLD`, `Workspaces` at 2, first reply seq 2, then `AuthCancel`); `try_serve_without_a_runtime_refuses_all_four_by_name`; `name_arms_are_stable`.

**T7** `tests/settings.rs` (`probed_row`, `section_over`, `render_section`): the fifteen plan cases as named, plus the four snapshots re-accepted. `tests/auth.rs` (harness; `until(&mut harness, |frame| .., PATIENCE)` alternating `drive()` and `sleep(10 ms)`): `a_then_enter_logs_in_and_the_cell_reads_the_probes_verdict`; `a_then_enter_into_no_credential_still_reads_unauthenticated`; `x_mid_login_ends_cancelled_and_the_cell_goes_back` (`install.rs:530-566`'s shape; `drive_to_end` only after `x`); `o_opens_the_link_through_the_injected_opener`; `the_agent_table_is_unchanged_by_a_login`.

**T8** `tests/auth_live.rs`, `#[ignore]`: `agy_logs_in_through_the_app_and_the_probe_reads_ready` and `agy_logs_out_when_asked` (`HTUI_LIVE_LOGOUT=1`), per plan T8; `HTUI_LIVE_OPEN=1` read with `std::env::var`, never set.

---

## G. Hazards

H-1…H-20 are the plan's; their code shapes are sections B.3, B.4, D.4 (H-1), C.2 + F/T5 (H-2), B.7 (H-3, H-4), B.2/B.8/B.9 Debug rules + F/T6's sentinel case (H-5), B.9 `auth_command` + `choice_tx.take()` (H-6), B.10 `on_reply(Agents)` (H-7), C.2's `echo` newlines (H-8), B.8 `Idle` frame + notice (H-9), D.3 (H-10), B.4 step 2 + P-1 (H-11), C.2 `/bin/sh <path>` + P-6 (H-12), P-2 (H-13), A rows 3–5 (H-14), B.3's SDK "method not found" (H-15), B.9 `_claim` (H-16), F/T6 `logout_reprobes_and_reads_unauthenticated` (H-17), T8 (H-18), B.7's two `cfg(windows)` sites (H-19), P-4 (H-20). Added here:

| # | Hazard | Failure it prevents | Code shape |
|---|---|---|---|
| H-21 | **The tap taken after the child moved** | `AcpIo::from_spawned` takes `Spawned` by value (`acp/mod.rs:158`); a `tap_stderr` written after it has no `Spawned` to call on, and a URL printed during `initialize` is lost | B.4 step 2 orders `pid()`, `tap_stderr()`, then `from_spawned`; T5's `the_policy_touches…` and T2's pid cases read the pid the same way |
| H-22 | **Two `Failed`s** (P-8) | A refused `auth_open` rendered as the end of the flow, or a dead flow rendered as a status-line refusal with the pane still open | `StoreReply::Failed { request }` for a request's refusal (the section keeps its state on `"auth_open"`), `AuthFrame::Failed { message }` for the flow's death (the section goes `Idle`) |
| H-23 | **`Declined` from the runtime side** | `run_auth` drops `choice_tx` on its own exit (shutdown abort mid-`Choosing`) and the wire half answers `Declined` after the kill — a frame nobody wants | `Declined` maps to `Cancelled` in `run_auth` step 5; the pane's `n`/`Esc` sends `AuthCancel`, never drops a sender, so `Declined` never reaches a screen |
| H-24 | **The idle clock and a duplex** | A prepared transport has no stderr; a `select!` arm over `None` from a finished receiver would spin | The stderr receiver is polled only while `Some`; on `None` it is set to `None` and the arm is disabled (`select!` precondition `if stderr.is_some()`) |
| H-25 | **`ReprobeClaim` held past the task** | `LiveAuth` outlives its task until the next `serve`; a chat's re-probe of that row is refused in that window | Benign and bounded (one `serve`); the flow's own re-probe calls `probe_agent` directly, never `run_reprobe`, so it never contends with its own claim |
| H-26 | **The chooser's index and a `Methods` frame arriving twice** | A stale `AuthStart` from a previous flow answering into a new `Choosing` | `is_fresh` drops frames at an older `AuthStart` seq (`app/state.rs:290-294`); the section's `Methods` arm resets `cursor` to 0 regardless |

---

## H. What this item does NOT touch

`crates/htui-store/**` and its three migration directories (no column, no method; `0003_orchestration.sql` stays MOD-4's); `crates/htui-core/src/**` (`AgentBox`, `AgentSummary`, `Agent`, `WriteStore::upsert_agent` at `crates/htui-core/src/store/traits.rs:108` — T6's byte-identical case pins that nothing calls it); `crates/htui/src/keymap.rs` (no section scope, the hint line stands in); `crates/htui/src/ui/overlay/**`; `crates/htui-agent/src/probe.rs` (`probe_agent`, `status_for`, `ProbeSnapshot` consumed unchanged); `crates/htui-agent/src/acp/handshake.rs` (copied in shape, `Handshake::from_response` keeps ids only, D7); `crates/htui-agent/src/install/**`; `crates/htui-core/seeds/agent_claude.json`; every manifest.

---

## Findings (tree vs plan), by line

1. `crates/htui/src/agent_worker.rs:335-342` — `finish_background` awaits the install **before** cancelling; the plan's H-13 wording ("cancel then abort under the limit") describes the timeout arm only. Mirrored, not changed (P-2).
2. `crates/htui/src/agent_worker.rs:1474-1477` — `Frames.addr` is private and never reassigned; the D18 "switches its `Frames.addr`" is implemented as a local `addr` (P-3).
3. `crates/htui-agent/src/acp/mod.rs:225`, `:364-369` — `IoSource` is private and `launch_for` refuses a prepared transport; T3's prepared-transport case needs `AuthSource`/`auth_source` (P-1).
4. `crates/htui-agent/src/acp/mod.rs:95` — `HANDSHAKE_TIMEOUT` is a const, not an `AcpSettings` field (`launch.rs:283-292`); `WireFlow.handshake_timeout` is filled by `run.rs` (P-7).
5. `crates/htui-agent/tests/extensibility.rs:276-315` — the sweep precedent that compiles against in-module tests is `the_installer_names_no_vendor` + `production_half`, not the zeta walk the plan cites at `:111-131` (P-4).
6. `crates/htui-agent/src/launch.rs:528` — one `Arc<Mutex<VecDeque>>`; D14's second `Arc<Mutex<Option<Sender>>>` would be a second lock, so the two are folded into `StderrTail` (P-9).
7. `crates/htui/src/agent_worker.rs:205-237` — no opener seam exists; `with_opener` is added (P-5).
8. Schema `agent.rs:3814` — `AgentCapabilities.auth: AgentAuthCapabilities` is not an `Option`; `init.agent_capabilities.auth.logout.is_some()` is the read (consistent with D11's wording; recorded so nobody writes `.auth.as_ref()`).
9. `crates/htui-agent/tests/probe.rs:70-92` — the `ETXTBSY` rule applies to the **re-probe's** `SpawnTier2` too, which has no retry; the T6 row therefore runs the fixture through `/bin/sh` (P-6), and whether an empty `tools` map with a `credential` block reaches tier 2 is left **UNVERIFIED** at `probe.rs:1444-1448`.

Files most relevant to the implementer, absolute: `/home/mluigi/projects/htui/crates/htui-agent/src/acp/handshake.rs` (`:96-159`), `/home/mluigi/projects/htui/crates/htui-agent/src/acp/mod.rs` (`:139-179`, `:225-467`), `/home/mluigi/projects/htui/crates/htui-agent/src/launch.rs` (`:522-568`, `:691-754`, `:780-819`), `/home/mluigi/projects/htui/crates/htui-agent/src/driver.rs` (`:30-36`, `:312-362`), `/home/mluigi/projects/htui/crates/htui/src/agent_worker.rs` (`:85-237`, `:327-343`, `:366-501`, `:510-560`, `:576-726`, `:1058-1095`, `:1151-1196`, `:1284-1298`, `:1384-1530`), `/home/mluigi/projects/htui/crates/htui/src/store_worker.rs` (`:49-283`, `:426-436`, `:574-591`, `:1358-1400`), `/home/mluigi/projects/htui/crates/htui/src/testkit.rs` (`:173-290`), `/home/mluigi/projects/htui/crates/htui/src/ui/tabs/settings/agents.rs`, `/home/mluigi/projects/htui/crates/htui-agent/tests/probe.rs` (`:70-92`, `:1251-1327`), `/home/mluigi/projects/htui/crates/htui-agent/tests/acp_driver.rs` (`:120-195`), `/home/mluigi/projects/htui/crates/htui/tests/install.rs` (`:527-566`).
