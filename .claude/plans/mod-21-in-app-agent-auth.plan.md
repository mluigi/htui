# Plan: MOD-21 in-app agent authentication (milestones 1–4)

**Source PRD**: `.claude/prds/mod-21-in-app-agent-auth.prd.md` (status PLANNING; gate decisions
D1–D8 are the maintainer's, confirmed 2026-09-09, and are implemented here, not re-opened).
**Selected Milestones**: all four. Milestones 1–2 are pure `htui-agent` work that milestone 3
consumes whole, exactly as MOD-20's were, and milestone 4 is one live test plus documentation;
one dependency chain, one file.
**Numbering**: the PRD owns **D1–D8**. This plan's decisions start at **D9**; tasks at **T1**. A
plan decision that follows from a gate decision cites it by name (`per D2`), never re-uses its
number. Source comments cite these as `plan MOD-21 D9`.

**Design authority**: the PRD's Evidence section (verified live on 2026-09-09 against
`agy_acp_server` 1.1.1 and the vendored schema; relied on here as given); `docs/REQUIREMENTS.md`
`R-AGT-9` (the contract), `R-AGT-1`, `R-AGT-4`, `R-AGT-5`, `R-AGT-6`, `R-TUI-8`, `R-NF-3`,
`R-SEC-2`, `R-ID-7`; `HANDOFF.md:433-469` (the item text and the constraints MOD-2 left: D59's
existence-only credential rule, D60's claim, D61's `ChildGuard`); `docs/ANA-4.md` §4.5
(`:705-713`, the sentence `R-AGT-9` reverses) and §11.14 (`:1384-1388`, the three questions a
logged-in server answers). Prior plans: MOD-2 D59/D60/D61/D63 (`mod-2-agy-acp.plan.md:104-108`),
MOD-20 D18 (`Served::Deferred` with an owned `Writer`, the `LiveInstall` claim), D19 (the
in-section pane, the hint line), D21 (Windows written here, verified by MOD-16), and the MOD-20
blueprint's hazards H-9, H-10 and H-17, which this item extends rather than duplicates.

**Requirements**: `R-AGT-9` (authenticated from the app through the agent's own protocol, the
method chosen from what the agent advertises, logout where advertised, the credential never read,
held, transmitted or stored, a fact about a box), `R-AGT-1` (an operation on the one driver seam),
`R-AGT-4..6` (the outcome reaches `agent_box` through the probe, nothing writes `agent`, no agent
name in code), `R-TUI-8` (the action in the Settings agent section), `R-NF-3` (never on the UI
task, never in the worker's `select!` arm), `R-SEC-2`/`R-ID-7` (no credential value anywhere:
not on a frame, not in a log, not in a row).

**Complexity**: Large — a new trait operation with a capability predicate, a second ACP call site
outside `open_session`, an interactive flow whose child lives for minutes with an open loopback
listener, a stderr line stream that does not exist today, a browser the flow has to neutralise,
four requests and one streamed reply, and a Settings pane with a chooser.

**Routing**: PRD path (C2, C3, C4 fired). Ultracode recommended for implement and review; this
plan is shaped for it — two independent starts in `htui-agent` (T1 and T4), a serial chain through
T5, then a serial `htui` chain T6 → T7 with T8 alongside, each task with its file set stated for
the mechanical intersection gate. `rust-reviewer` gate per `.claude/workflow-config.json`.

## Summary

Today `unauthenticated` is a verdict with no verb. The probe records it (`probe.rs:1197-1205`
maps a non-empty `authMethods` plus no credential tier to `Unauthenticated`), the Settings cell
renders it (`agents.rs:349-351`), the chat refuses on it, and the cure is a vendor login `htui`
neither performs nor explains. The protocol already carries the verb: `authenticate` takes one
field, answers nothing but `_meta`, and the SDK this tree pins already sends it
(`agent-client-protocol-2.1.0/src/schema/client_to_agent/requests.rs:18`,
`impl_jsonrpc_request!(AuthenticateRequest, AuthenticateResponse, "authenticate")`; `:33` the
same for `logout`).

The plan adds exactly the verb, in the crates that already own each half:

- **`htui-agent`** gains a `DriverCaps::authenticate` predicate and an `AgentDriver::authenticate`
  operation with a default body that refuses (D10); `acp/auth.rs`, the wire flow — `initialize`,
  the live method list, the user's choice, `authenticate` or `logout`, the child killed on every
  exit — in the `handshake.rs` frame shape D1 adopts (D12); a transport-neutral `auth` module
  beside `install/` holding the flow's types, URL detection, the browser policy, the opener and the
  end-to-end run (D9); and a stderr **tap** on `Spawned`, because the tail that exists today is a
  bounded, unsignalled snapshot and the hand-off needs lines as they arrive (D14).
- **`htui`** gains four requests and one streamed reply served through `AgentRuntime::serve` as
  `Served::Deferred` off an owned `Writer`, a `LiveAuth` the runtime holds one of, the re-probe
  that decides the outcome, and a Settings action `a` beside `r` and `i` with a chooser, a stream
  pane, `o` to open the link and `x` to cancel (D18–D20).
- **`htui-core`** changes only if milestone 4 finds the seed's declared credential path is not what
  the vendor writes (D23). **`htui-store`** does not change at all; MOD-4's `0003_orchestration.sql`
  stays the next migration.

Nothing is keyed on an agent's name. The chooser is fed by the agent's own `initialize`; the
outcome is the probe's; the credential is a file `htui` checks the existence of and never opens.

## Prerequisite: what the tree already settles

Checked on this box at `5a16669` on 2026-09-09, before a task was written. Facts the PRD's
Evidence supplies are used as given; the ones below are what this plan needed beyond them.

- **The seam has room and a shape to copy.** `DriverCaps` (`driver.rs:317-333`) derives `Default`
  as all-false, so a transport that forgets a predicate advertises nothing — the property D10 leans
  on. `AgentDriver` (`driver.rs:344-362`) returns `DriverFuture` (`:36`) from every operation and
  its doc says "five operations" (`:34-35`), which T1 amends. `DriverCaps` is built as a struct
  literal in exactly **three** places — `registry.rs:144-151` and `:158-166` (`caps_from`),
  `fake.rs:104-111` (`full_caps`) — plus one inside a `#[cfg(test)]` module of the chat tab,
  `crates/htui/src/ui/tabs/chat/mod.rs:688-695`; `tests/driver_contract.rs:49` uses
  `DriverCaps::default()` and is untouched. The chat banner reads three predicates only
  (`chat/mod.rs:326-332`), so no chat snapshot moves.
- **`DriverError` has no "unsupported" variant** (`error.rs:13-44`), and every `match` on it in
  `htui` is an `if let` or carries a wildcard (`agent_worker.rs:1591` matches `Spawn` alone;
  `probe.rs:1484-1487` has `other =>`), so a new variant compiles everywhere.
- **The probe's process shape is one function**: `acp::handshake()` (`handshake.rs:96-159`) —
  `ChildGuard` in the caller's frame (`:102`), the connection task owning only the streams
  (`:106-126`), `block_task()` on the one `send_request` in the foreground future (`:116`), a
  `select!`-shaped `timeout` over a `oneshot` (`:128-148`), then `kill_and_reap` and `task.abort()`
  unconditionally (`:152-157`). `Handshake::from_response` keeps `authMethods[].id` only
  (`:70-74`) and records `agentCapabilities` verbatim as a `Value` (`:69`), so the stored snapshot
  already carries `auth.logout` — the seed fixture confirms `agy` advertises it
  (`tests/fixtures/agy_acp_handshake.json:22`, `"logout": {}`).
- **The schema types the flow needs, by line** (`agent-client-protocol-schema-1.7.0/src/v1/`):
  `AuthenticateRequest::new(method_id)` (`agent.rs:295-318`), `AuthenticateResponse` with `_meta`
  only (`:341`), `LogoutRequest` (`:385`), `AgentAuthCapabilities.logout: Option<LogoutCapabilities>`
  (`:464-472`) reached as `initialize.agent_capabilities.auth` (`:3814`), `AuthMethod` a
  `type`-tagged enum whose untagged arm is `Agent` (`:575-583`) with `id()`/`name()`/`description()`
  accessors (`:586-620`), `AuthMethodTerminal` whose doc says "The client MUST NOT pass this method
  to `authenticate`" (`:696`), `ClientCapabilities::auth(AuthCapabilities)` (`client.rs:2096`) and
  `AuthCapabilities::terminal(bool)` (`:2389`, default `false`), `ClientCapabilities::elicitation`
  (`:2104`). **`client_capabilities()` (`acp/client.rs:32-38`) sets neither `auth` nor
  `elicitation`, so both are already the D4/D21 values by default**; T2 pins that with a test
  rather than a change.
- **An inbound request nobody registered a handler for is answered "method not found" by the SDK
  itself** (`agent-client-protocol-2.1.0/src/jsonrpc/incoming_actor.rs:620`; the doc at
  `jsonrpc.rs:850` says so). An `elicitation/create` the agent sends during `authenticate` is
  therefore an error the agent hears, not a hang (D21).
- **The stderr channel is a tail, not a stream.** `launch::spawn` starts one reader task per child
  (`launch.rs:798-810`) that pushes each line into a `VecDeque` capped at `STDERR_TAIL_LINES = 64`
  (`:31`); `Spawned::stderr_tail()` (`:563-568`) clones the whole deque on demand and nothing
  signals a new line. The PRD's "already plumbed" is true of the pipe and the reader, not of a
  line stream — D14 adds the tap and it is the one change to `launch.rs`.
- **The runtime already has every piece the delivery needs.** `LiveInstall { agent_id, phase,
  cancel, task }` (`agent_worker.rs:152-161`), `claim_is_free` covering the install and a probe in
  flight (`:689-708`), `probe()` refusing while an install runs (`:519-524`), `install_cancel`
  tripping the token and answering at once (`:670-679`), `shutdown` cancelling before aborting
  (`:485-497`), `finish_background` awaiting the install under a limit (`:335-342`),
  `ReprobeClaims::claim((agent_id, box_id)) -> Option<ReprobeClaim>` with a releasing `Drop`
  (`:1061-1090`), `run_reprobe`'s tier-2-only re-probe with `ProbeEnv::host(cwd).without_versions()`
  (`:1151-1200`), and `Frames::reply(addr, ..)` (`:1520-1522`). `LiveChat.commands` plus
  `ChatCommand` (`:89-121`) is the shape a command *into* a live task takes.
- **The worker loop and the harness each name the runtime-served requests explicitly**:
  `store_worker.rs:574-591` (the interception before `try_serve`), `:426-436` (`try_serve`'s
  "no agent runtime in this build" refusal), and `testkit.rs:184-194` (`Harness::drive`), whose
  doc says a request the runtime owns "has to be added to it deliberately". Four new variants mean
  three lists.
- **The section's key budget.** The tab consumes `h`/`l`/`[`/`]`/arrows before a section is
  offered a key (`settings/mod.rs:197-213`); the global table binds `q`, `Tab`, `BackTab`, the
  digits and `?` (`keymap.rs:201-228`) plus `ctrl-c` and `-` (`agents.rs:512-514`); the section
  itself binds `j`, `k`, `i`, `r`, `x`, `y`, `n`, `Esc` (`agents.rs:507-570`). `a`, `o` and
  `Enter` are free. The MOD-20 review rule stands: a pane consumes its own keys and passes the rest
  (`agents.rs:259-268`), so a chooser may not swallow the digits.
- **`App::is_fresh` keys on `(origin, request kind)`** (`app/state.rs:290-294`): a frame is
  delivered if its `seq` is the newest of its *kind*, so frames at an `AuthStart`'s seq and frames
  at an `AuthChoose`'s seq are both fresh at once — the plan-then-confirm split MOD-20 relies on.
- **Process fixtures and kill checks exist and are Linux-shaped.** `tests/probe.rs` writes shell
  scripts with `executable(path, contents)` (`:77`) and answers `initialize` in-process through
  `scripted_initialize` over a duplex (`:1251-1270`), with `DuplexTier2` (`:1304-1327`) driving the
  real `acp::handshake`; `tests/acp_driver.rs:128-190` checks a kill through `/proc/<pid>/stat`
  and a bounded `KILL_WINDOW`, pid taken from `Spawned::pid()`. `tests/probe.rs:1042` records why a
  freshly written fixture is run through `sh` (`ETXTBSY`). Per-file helper rule: each test file
  carries its own copies (`crates/htui/tests/install.rs` module doc).
- **The harness polls, it does not sleep.** `Harness::drive` (`testkit.rs:173-262`) serves queued
  requests, polls chats once, drains the reply channel and returns when nothing moved; a task the
  runtime spawned makes progress only between drives. `drive_to_end` (`:271-297`) calls
  `finish_background(CHAT_END)` **then** `shutdown`. `x_mid_download_ends_cancelled_and_the_cell_goes_back`
  (`tests/install.rs:530-566`) is the precedent for a key pressed against a task still running.
- **URL opening without `unsafe` and without a crate.** `ShellExecuteW` is an `unsafe` FFI call
  the workspace forbids. `open 5.4.3` (MSRV 1.62, MIT) does not call it by default either: its
  `src/windows.rs` (read today) spawns `powershell.exe -NoProfile -NonInteractive -Command
  "Start-Process -FilePath $env:OPEN_RS_TARGET"` with the target in the child's **environment**
  — no quoting of the URL at all — under `CREATE_NO_WINDOW`, then `explorer.exe` as a fallback.
  D17 borrows the spelling and not the crate. On this box `/usr/bin/xdg-open` and `/usr/bin/open`
  exist; `true` is `/bin/true` and `/usr/bin/true`; `DISPLAY` and `BROWSER` are unset in the
  shell the tests run from — the second live run's conditions.
- **`url 2.5.8` is in `Cargo.lock`** only as a transitive dependency of `reqwest`; D15 needs no
  parser, so it is not promoted to a direct dependency.
- **The box has never completed the flow**: `~/.gemini/antigravity-acp/` does not exist here, so
  the seed's `credential.files` (`agent_agy.json:38-40`) is still the unverified declaration the
  PRD's risk table names. Milestone 4 is the only way to learn the filename (D23).
- **The README paragraph this item replaces** is `README.md:274-280` ("Authentication is the
  vendor's, not `htui`'s … `htui` never sees the credential and cannot perform the login"), inside
  "Installing an agent's adapter" (`:234`), before "Platforms" (`:282`). MOD-20's D22 left it
  untouched by name.
- **TOOL-3** (`HANDOFF.md:504-523`): `cargo clippy --target x86_64-pc-windows-msvc` dies in
  `ring`'s build script on this box for both `htui-agent` and `htui`. No task below names that
  line as a gate.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D9 | **Module layout: two halves, one per crate boundary that already exists.** The **wire** half is `crates/htui-agent/src/acp/auth.rs` — `initialize`, the live `AuthMethod` list, `authenticate`/`logout`, the JSON-RPC error, over an `AcpIo`, in `handshake.rs`'s frame shape. The **policy** half is a new `crates/htui-agent/src/auth/` beside `install/`: `mod.rs` (the flow's types and constants), `url.rs` (D15), `browser.rs` (D16, D17), `run.rs` (the end-to-end operation `AcpDriver::authenticate` delegates to: resolve the launch, apply the browser policy, spawn, tap stderr, drive `acp::auth`, forward lines and URLs, own the idle clock). `acp/mod.rs` gains `pub mod auth` and the `AgentDriver::authenticate` impl; `lib.rs` re-exports the `auth` types. | `acp/` is where every SDK type is spoken today (`handshake.rs`, `client.rs`, `mod.rs`) and nowhere else in the crate imports `agent_client_protocol::schema`; keeping `AuthenticateRequest` there keeps that true. Everything the *user* sees — a method's name, a stderr line, a URL, an outcome — is transport-neutral, and a CLI transport that one day *does* have a login must be able to produce the same `AuthEvent`s without touching `acp/`. Growing `acp/mod.rs` (1753 lines already) with a second interactive flow would put the browser policy beside the session mapper; growing `handshake.rs` would turn the probe's one-call file into two. `install/` set the precedent that a user-facing pipeline gets its own directory with a `run.rs` and that the crate root re-exports it (`lib.rs:107-113`). |
| D10 | **The seam: `DriverCaps.authenticate: bool` and `AgentDriver::authenticate<'a>(&'a self, flow: AuthFlow) -> DriverFuture<'a, AuthOutcome>` with a default body that answers `Err(DriverError::Unsupported("authenticate"))`.** `DriverError` gains `Unsupported(&'static str)` (`"this transport has no `{0}` operation"`). `caps_from` (`registry.rs:142-168`) sets `authenticate: true` for `Transport::Acp` and `false` for `Transport::Cli`; `FakeDriver::full_caps` says `false`. A contract test asserts, for every adapter the factory registers, that `caps().authenticate` and "the operation does not answer `Unsupported`" agree. | `R-AGT-1` says the seam, and the PRD's metric is "a transport with no `authenticate` refuses rather than pretends": a **default body** makes refusal the thing a transport gets by saying nothing, which is the same structural reading `DriverCaps`'s all-false `Default` already has. The predicate is computed by the factory from the row, as every other predicate is (`registry.rs:132-135`: "so the chat tab's capability banner and the orchestrator's gate check read one profile per row"), which is what lets the Settings section refuse `a` on a `cli` row from the document alone (`caps_for`, `:137`) before a request is spent — the `declares_a_source` pattern of MOD-20 (`agents.rs:644-649`). `Unsupported` is a new variant rather than a `Transport("…")` string because the section and the runtime both branch on it, and a string is not something to branch on. The CLI transport is not registered in production (`registry.rs:65-72` registers `acp` only; `cli/fake` exists under `test-support`), so the refusal is proven through `caps_for` on a `cli` row and through the fake's default body, not through a production CLI driver. |
| D11 | **One spawn, one operation, an interactive middle.** `AuthFlow { cwd: PathBuf, events: mpsc::UnboundedSender<AuthEvent>, choice: oneshot::Receiver<AuthChoice>, cancel: CancellationToken, idle: Duration, browser: BrowserPolicy }`. `AuthEvent::{ Methods { methods: Vec<AuthMethodInfo>, logout: bool, hidden: Vec<AuthMethodInfo> }, Line(String), Url(String) }` with `AuthMethodInfo { id, name, description: Option<String> }`. `AuthChoice::{ Method(String), Logout }`. **The schema's `AuthMethod` is `#[non_exhaustive]`**
(`schema/v1/agent.rs:574`), so the match that builds these arms needs a wildcard to compile — but
**that arm is unreachable for wire data, and hiding unknown kinds there would be a lie**: the enum
declares `Agent` `#[serde(untagged)]`, so an entry with an unrecognised `type` deserialises **as
`Agent`**, not as something a match can catch. Measured against the real crate:
`{"type":"future-kind","id":"x","name":"X"}` → `Agent(AuthMethodAgent { id: "x", … })`. Only
`type: "terminal"` is distinguishable, which is the one the spec forbids sending. So `hidden`
holds terminal methods, the wildcard arm is written as `unreachable`-shaped defence with a comment
saying why, and a future kind is offered as an agent method — the schema's own choice, not
`htui`'s. `AuthOutcome::{ Completed { call: AuthCall }, Refused { call: AuthCall, message: String }, Cancelled, Idle { after: Duration }, Declined }` where `AuthCall::{ Authenticate(String), Logout }` and `Declined` is the sender of `choice` dropped before choosing. Spawn and transport failures are `Err(DriverError::Spawn | Transport)`. | D1 ("one spawn serves both the method list and the call") and D7 (the chooser reads the live response) force the choice *into* the operation: two operations would be two spawns, and the second would `initialize` again to send an id the first already knew. A `oneshot` is the narrowest thing that carries one answer once. D5 makes a JSON-RPC error an **answer** — the live `-32602` names the variable the user has to set — so `Refused` is an `Ok` outcome carrying the agent's own `Display` text (the same `{err}` `handshake_error` renders, `handshake.rs:163-170`), not an `Err` a caller might swallow. `Cancelled` and `Idle` are outcomes for the same reason: the flow ended the way it was told to, and D6 says each of them "leaves `agent_box` exactly as found", which the runtime can only enforce by matching on them. Every event is text or ids; nothing on this enum can carry a credential by construction (`R-SEC-2`). |
| D12 | **Lifetime and kill discipline: `acp::auth::run(io, settings, flow) -> Result<AuthOutcome>` is `handshake()` with two more exits.** The `ChildGuard` lives in `run`'s frame (`handshake.rs:102`); the connection task owns only the streams; the foreground future sends `initialize` with `block_task()`, sends `AuthEvent::Methods`, then `select!`s the `choice` receiver against `cancel.cancelled()`, and on a choice sends `authenticate` or `logout` with `block_task()` and reports through the `oneshot` (`:104-121`). The outer frame `select!`s the answer against `cancel.cancelled()`; on every exit — answered, refused, cancelled, the connection actor dropping the foreground future, the sender dropped — it runs `guard.kill_and_reap().await` and then `task.abort()` (`:152-157`). Aborting the *outer* future drops the guard, whose `Drop` signals the tree (`launch.rs:738-754`). `HANDSHAKE_TIMEOUT` bounds `initialize` only; nothing bounds `authenticate` but D13. | D1 names this lifetime and the reason: no session, no prompt, no cwd to invent. The two exits `handshake()` lacks are the human's — a cancel and a choice — and both are awaited *inside* the foreground future because that is the only place the SDK lets a request be sent from (ANA-4 §4.2 risk 11, `handshake.rs:115`). `block_task()` appears on the two `send_request`s and nowhere else. The choice is awaited under a `select!` with the token so a cancel during the chooser ends the connection future cleanly rather than leaving it parked on a `oneshot` nobody will fire. MOD-2 spent two review gates on this class (D61, the milestone-3 CRITICAL) and this child is longer-lived than any of them, with a loopback listener open (PRD Evidence: `redirect_uri=http://127.0.0.1:<port>/`), so the kill is unconditional and reaped, and T2's process cases prove each exit by pid. |
| D13 | **Cancellation, not a timeout (D3 made concrete).** A `CancellationToken` tripped by `AuthCancel`, by shutdown, and by the idle clock. `pub const AUTH_IDLE_CAP: Duration = 10 min` in `auth/mod.rs`, injected through `AuthFlow.idle` so tests use milliseconds. The clock lives in `auth/run.rs`, which sees every stderr line, the `Methods` event and the choice, and resets on each; when it fires it cancels a **child token** of the flow's and maps the resulting `Cancelled` to `Idle { after }`. `AgentRuntime::shutdown` cancels the live flow's token, waits `grace * 2`, then aborts (the `LiveInstall` arm, `agent_worker.rs:485-497`). | The PRD's D3 says "a long idle cap (order of ten minutes) kills a flow nobody is watching rather than leaking a child with an open listener". Idle is *silence*, not elapsed time: a human OAuth round trip is minutes of nothing on stderr, and a cap measured from the spawn would kill a slow but live login; a cap measured from the last sign of life kills only an abandoned one. It lives in `run.rs` rather than `acp/auth.rs` because the wire half sees no stderr, and a child token keeps "the user cancelled" and "the clock cancelled" distinguishable for the outcome. Cooperative cancellation through a token rather than `abort()` for MOD-20 D18's reason: the task owes its stream a last frame. |
| D14 | **A stderr tap on `Spawned`.** `launch.rs`'s reader task keeps its tail (`:798-810`, `STDERR_TAIL_LINES` unchanged) and additionally forwards every line to an `Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>`. `pub fn tap_stderr(&mut self) -> mpsc::UnboundedReceiver<String>` installs a sender and, **under the same lock the reader holds**, first replays the lines already in the tail, so no line is lost or duplicated by tapping late. One tap per child; a second call replaces the first. Lines only (`lines()`), lossy UTF-8. | Polling `stderr_tail()` every tick has no cursor: 64 lines is a bound, not a position, and a flow that printed more than 64 lines between two 250 ms ticks would silently lose some. A second reader of the pipe is impossible — the reader task owns the handle. Replaying the tail under the lock is what makes "tap after spawn" exact: the URL line can arrive during `initialize`, before `run.rs` has finished wiring. `tap_stderr` is the one addition to `launch.rs`, a file two review gates hardened, and it changes no existing behaviour (T4's cases pin the tail's). |
| D15 | **URL detection is a scan, not a parse.** `auth::url::first_url(line: &str) -> Option<String>`: the first substring starting with `http://` or `https://`, extended to the first whitespace, `<`, `>`, `"`, `'` or `` ` ``, with trailing `.`, `,`, `;`, `:`, `)`, `]`, `}` stripped. `run.rs` sends `AuthEvent::Line` for every line and `AuthEvent::Url` for every **new** URL (deduplicated by equality within the flow). No `url` crate; no vendor sentence matched. | The PRD's risk row: "surface the stream, do not parse a vendor's sentence; keep URL detection generic". The live line is `Open the following link to authenticate the ACP server: https://accounts.google.com/…`; another agent will write another sentence and the scheme is the one thing all of them share. The pane shows the line whether or not a URL was found, so a miss costs the `o` key and nothing else. A parser would add a dependency to validate a string `htui` only ever hands to an opener, and D17 already restricts that opener to the two schemes the scan admits. |
| D16 | **The browser-neutralising environment (D2 made concrete).** `BrowserPolicy::{ Neutralised, Inherit }`; production is `Neutralised`, `Inherit` exists for the regression test. `Neutralised` inserts **one** variable into the auth spawn's `ResolvedLaunch.env`: `BROWSER` = the absolute path `which::which("true")` answers on unix (`/bin/true` here; resolved, never spelled, because macOS keeps it under `/usr/bin`), falling back to the literal `true`. On Windows the value is `cmd.exe /c exit 0` — **written here, unverified here, MOD-16's** (D22). `DISPLAY` and `WAYLAND_DISPLAY` are left exactly as inherited. The variable is applied to the auth spawn only: never to a chat spawn, never to the probe's, never recorded in `probe.resolved`. | The PRD's live evidence is the decision: `BROWSER=/bin/true` produced a clean stream; unset, with no display, the adapter's opener fell through to a terminal browser that wrote alt-screen sequences into the JSON-RPC channel. A no-op that **exists and exits 0** is what stops a chain-style opener (Python's `webbrowser`, the likely implementation behind a `.par`, is *unverified* and this plan does not depend on it) from trying the next candidate — a nonexistent command would fail and fall through to exactly the terminal browser that hijacked stdout. Unsetting `DISPLAY` would make things worse for the same reason. The rejected alternative — a temporary directory holding no-op `xdg-open`/`open` shims prepended to the child's `PATH` — catches openers that ignore `BROWSER` (Node's `open` package calls `xdg-open` directly) but writes executables to disk per flow and rewrites the child's `PATH`; it is recorded as the fallback if a second agent (`amp-acp`'s `setup`, untested) proves to ignore `BROWSER`, and not built for the MVP. T5's regression fixture is the observed hijack: an agent that, on `authenticate`, runs `$BROWSER` when set and writes `\x1b[?1049h…` to **stdout** when not. |
| D17 | **The opener: `htui` opens the URL on `o` and only then, with a plain `tokio::process::Command` whose stdio is all `Stdio::null()`.** Linux `xdg-open <url>`; macOS `open <url>`; Windows `powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:HTUI_OPEN_URL"` with the URL in the child's environment and `CREATE_NO_WINDOW` (the `open 5.4.3` spelling, borrowed; the crate itself is not added). `open_url` refuses any scheme but `http`/`https` before spawning, spawns, and hands the child to a `tokio::spawn`ed `wait()` so it is reaped; it neither awaits the opener nor kills it. **Not** through `launch::spawn`: the opener must not become a process-group leader `htui` owns. | D2: "opens it only when the user presses the key — `xdg-open`/`open` here, `ShellExecute` on Windows (verification MOD-16's)". `ShellExecuteW` is `unsafe` FFI and forbidden by the workspace; PowerShell's `Start-Process` with the target in an environment variable is the one Windows spelling that involves no quoting of an OAuth URL full of `&` and `%` (`cmd /C start "" "<url>"` parses both; `rundll32 url.dll,FileProtocolHandler` is undocumented). Null stdio is the second half of D2's protection: on a box with no display `xdg-open` itself may fall through to a terminal browser, and one with no tty on any of its three streams dies at `initscr` instead of seizing `htui`'s screen. Not owning the tree is deliberate — `xdg-open` may exec the real browser as its own child, and a timeout-kill on that tree would close the user's browser ten seconds into their login. Refusing non-`http(s)` closes the one door the scan could otherwise open (`file:`, `javascript:`). |
| D18 | **Requests, replies, runtime.** `StoreRequest::AuthStart { agent_id }`, `AuthChoose { choice: AuthChoice }`, `AuthOpen { url: String }`, `AuthCancel`; `StoreReply::Auth(AuthFrame)` with `AuthFrame::{ Methods { methods, logout, hidden }, Line(String), Url(String), Opened, Done { status: ProbeStatus }, Refused { message }, Cancelling, Cancelled, Failed { message } }`. Served by `AgentRuntime::serve`: `AuthStart` → `Served::Deferred` with a task; `AuthChoose` and `AuthOpen` → forwarded into the live task through `LiveAuth.commands: mpsc::UnboundedSender<AuthCommand>` (`AuthCommand::{ Choose { choice, reply: ReplyAddr }, Open { url, reply: ReplyAddr } }`, the `LiveChat.commands` shape) and `Served::Deferred`; `AuthCancel` → the token, answered `Cancelling` at once. **Frames before the choice carry the `AuthStart` seq; the task switches its `Frames.addr` to the `AuthChoose` address when the choice arrives**, so every later frame carries that seq (the plan/confirm split of MOD-20 D18). `AuthOpen` is answered `Opened` or `Failed { request: "auth_open" }` at its own seq. **Refusals before anything is spawned**, in the probe's order (`agent_worker.rs:510-547`): no writer or `Writer::Buffered` → `REGISTRY_ON_SERVER_ONLY`; no box row; the claim held (D19); no such row; `caps_for(agent).authenticate == false` → `Unsupported`'s text; a stored snapshot whose `handshake.auth_methods` is empty → "`<name>` advertises no authentication methods". The task: `driver.authenticate(flow)` polled beside a forwarding loop; on `Completed` it re-probes the row — `probe_agent` with `ProbeEnv::host(cwd).without_versions()` and `SpawnTier2::default()`, the `run_reprobe` recipe (`:1151-1200`) — writes the row with the owned `Writer` **before** sending `Done { status }` (the ordering `run_install` documents at `:1376-1380`), and on every other outcome writes nothing. `LiveAuth { agent_id, cancel, commands, task, _claim: ReprobeClaim }`; `Debug` prints the id and whether the task finished — never a line or a URL. | The `ProbeAgents`/install shape is the PRD's stated model and `R-NF-3` is enforced by the same ownership: the `Writer` is taken before the spawn, the task owns it, the loop `continue`s (`store_worker.rs:582-590`). One request, many replies is the chat stream. Two request kinds with the address switch rather than one long stream at the start seq, because `is_fresh` (`app/state.rs:290`) keys on request kind, and the chooser's answer is a *new* request whose frames must supersede a stale one. A command channel into the task rather than a `oneshot` on the runtime, because there are two commands (choose, open) and the opener has to run somewhere the runtime owns — `background` would trip `claim_is_free`'s "a probe is running" (`:702-706`) and a bare `tokio::spawn` would be a task nobody can name. **D6 is the `Completed` arm and nothing else**: the re-probe is the only write, it goes through `probe_agent`, and a `Refused`/`Cancelled`/`Idle`/`Failed` leaves `probed_at` untouched because nothing runs. Tier 2 only (`without_versions`) because a login changes no version string. `Done` carries the probe's status and the section re-reads `Agents`, so there is no "logged in" state anywhere in the UI that the probe did not decide (`R-AGT-6`). Nothing here touches `WriteStore::upsert_agent` (`crates/htui-core/src/store/traits.rs:108`); T6's byte-identical test pins it. |
| D19 | **One claim, three holders.** `AgentRuntime.auth: Option<LiveAuth>` joins `install` and `background` in `claim_is_free` (`agent_worker.rs:689-708`): an auth flow refuses while an install or a probe runs; an install plan or confirm refuses while a flow runs; `probe()` refuses while a flow runs, with the same sentence shape as `:519-524`. `serve` sweeps a finished `LiveAuth` at the top exactly as it sweeps `install` (`:380`). `LiveAuth` additionally holds a `ReprobeClaim` for its `(agent_id, box_id)`, taken at `AuthStart`, so a D60 or staleness re-probe for the same row (`ReprobeClaims`, `:1061-1090`) is refused for the flow's whole life, and the flow's own re-probe never overlaps one. | The PRD's risk row: "the runtime's existing one-at-a-time claim (MOD-20 hazard H-10, MOD-2 D60's claim) covers all three, extended rather than duplicated". Two claims exist today for two different overlaps — `claim_is_free` for "who may write `agent_box` from a Settings action", `ReprobeClaims` for "who may re-probe this row from a chat" — and the flow sits in both: it will write the row at the end (so the first), and its write is a re-probe of one row (so the second). Holding the `ReprobeClaim` from the start rather than from the `Completed` arm closes the window in which a chat started during the browser round trip re-probes the row mid-login and records `unauthenticated` seconds before the flow's own probe records `ready` — harmless in outcome, confusing on screen. |
| D20 | **The Settings UI.** `a` on the highlighted row starts the flow; `AuthState::{ Idle, Starting { agent_id }, Choosing { agent_id, methods, logout, hidden, cursor }, Running { agent_id, call, lines: VecDeque<String>, url: Option<String>, cancelling } }`. Refusals, from the document alone and before any request: an install, probe or flow in flight; no row selected; `caps_for(&summary.agent).authenticate == false` ("`<name>` is a `cli` agent; it has no `authenticate` call"); no `on_box` snapshot or a status other than `unauthenticated`/`ready` ("probe `<name>` first"); a snapshot whose `handshake.auth_methods` is empty ("`<name>` advertises no authentication methods"). `Choosing`: the pane lists each method as `name — description`, plus `log out` last when advertised, plus one dim line "`n` method(s) need a terminal `htui` does not provide" when `hidden` is non-empty (D4); `j`/`k` move, `Enter` sends `AuthChoose`, `n`/`Esc` sends `AuthCancel`; every other key passes. `Running`: the pane is the last `AUTH_PANE_LINES = 6` stderr lines and, when one was detected, `link: <url>`; `o` sends `AuthOpen { url }` (refused with "no link yet" otherwise); `x` sends `AuthCancel`. The `on this box` cell reads `starting…`, `choose a method`, `logging in…` / `logging out…`, `cancelling…`. Hints: idle `j/k select · r probe · i install · a authenticate`; choosing `j/k choose · Enter select · Esc cancel`; running `o open link · x cancel`. Terminal frames: `Done { status }` → `Idle`, a notice `logged in: <status>` / `logged out: <status>`, and a `StoreRequest::Agents`; `Refused { message }` → `Idle` and the message as the notice; `Cancelled`, `Failed` → `Idle` and a notice. `on_reply(Agents)` does not touch `auth` (hazard H-17). `r` and `i` are refused while a flow runs. | MOD-20 D19's reasons apply unchanged: the overlay factory takes no argument, replies to a popped overlay are dropped, and the section holds the cursor. The chooser reads from the live `Methods` frame (D7) and not from the snapshot, but the snapshot decides whether `a` is *offered* (D7's stated cost). `Enter` rather than digits because the digits are global tab switches and the MOD-20 review finding (`agents.rs:259-268`) forbids a pane that kills them. Six lines because the pane is under a table that must stay readable on a 24-row terminal, and the live flow's useful stderr is one line. Logout in the chooser rather than on its own key because it is the same spawn and the same stream (D1), and a `ready` row with auth methods is exactly the row that can only log *out* — the chooser shows methods anyway, since re-authenticating is a thing a user does. |
| D21 | **What is not advertised, and what happens if an agent ignores that.** `client_capabilities()` keeps `auth.terminal` and `elicitation` at their defaults (`false`/`None`), pinned by a test rather than changed. A `terminal`-typed `AuthMethod` in the live list is never sent to `authenticate` (schema `agent.rs:696`); it goes into `AuthEvent::Methods.hidden` and the chooser says why. An `elicitation/create` the agent sends anyway is answered by the SDK's automatic "method not found" (`incoming_actor.rs:620`) — the agent hears a refusal, `htui` renders whatever it then writes to stderr, and the flow ends the way the agent ends it. | D4 verbatim for terminal methods. Elicitation is the schema's own answer to this problem (PRD Evidence, `elicitation.rs:1489-1496`) and `agy` did not use it; advertising a capability with no UI behind it "turns a working session into a hung one" (`launch.rs:308-309`), so it stays off and is recorded as the additive later step the PRD's risk row asks for. Naming the SDK's behaviour here is what makes "the agent hangs waiting for an elicitation answer" a claim the fact-check can falsify. |
| D22 | **Windows: written and reviewed here, verified by MOD-16; not lint-checked on this box.** Written: the PowerShell opener with `CREATE_NO_WINDOW` (D17), the `BROWSER` spelling (D16), `cfg(windows)`/`cfg(unix)` split in `browser.rs` only. **No `cargo clippy --target x86_64-pc-windows-msvc` line appears in any task's Validate or in the Validation block**: TOOL-3 records that the target cannot be built here at all (`ring`'s build script, no MSVC-capable C compiler) for `htui-agent` and `htui` both, and MOD-20 shipped under the same condition with its Windows lines reviewed by eye. **Deferred to MOD-16 by name**: whether any opener on Windows honours `BROWSER`, and whether `cmd.exe /c exit 0` is a value it accepts; that `Start-Process` opens the default browser from a non-interactive PowerShell under `CREATE_NO_WINDOW`; that the job object's kill-on-close reaps the auth child **with its loopback listener** (MOD-2's criterion 11, now with a socket); and every runtime fact about `agy_acp_server.exe`'s login. | The PRD's out-of-scope line and MOD-16's charter. Listing the deferred facts by name is what lets MOD-16 pick them up. Saying plainly that the lint gate is absent, where MOD-20 D21 promised one it could not run, is what TOOL-3 asked of the next item. |
| D23 | **The live proof and the documentation.** `crates/htui-agent/tests/auth_live.rs`, `#[ignore]`, interactive: it runs the production path over the **unmodified** seed row, prints the method list and the stderr stream, waits up to `AUTH_IDLE_CAP` for the maintainer to finish the browser flow, then re-probes and prints the status and **what `~/.gemini/antigravity-acp/` (or `$GEMINI_HOME`) now holds**. It asserts: the flow reached `Completed`; no child survives; the re-probed status is `ready` **or** the seed's `credential.files` names a file that does not exist while the directory holds one that does — in which case the test fails by name and T8 corrects the seed from the finding. MOD-2's T34 (`HANDOFF.md:233-243`) is then run as written: `agy_live.rs -- --ignored`. `README.md:274-280` becomes the in-app flow; `HANDOFF.md`'s T34 note is answered; MOD-21's line gains a phase note per milestone; the PRD's rows go `complete`. | Milestone 4 as the PRD states it: "`agy` on this box goes `unauthenticated` → `ready` through the app, the declared credential path is confirmed or corrected, MOD-2's T34 chat runs, and the README says what to do instead of what to install". The filename is learnable only by completing the flow (PRD risk table), so the test is where it is learned. Interactive and ignored, like every `*_live.rs` (`agy_live.rs:4-11`): a box where nobody presses the key is not a failing build. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Child in the caller's frame, killed on every exit | `crates/htui-agent/src/acp/handshake.rs:96-159` | `ChildGuard::new(child)`; streams into the task; `block_task()` in the foreground only; `select!` over a `oneshot`; `kill_and_reap` then `task.abort()` |
| Error text with the stderr tail | `handshake.rs:163-170` | `format!("initialize failed: {err}\n{stderr}")` |
| Command into a live task | `crates/htui/src/agent_worker.rs:89-121, 711-726` | `ChatCommand { .., reply: ReplyAddr }` over an unbounded sender; a closed channel is "this chat has ended" |
| Long op off the loop, owned `Writer` | `agent_worker.rs:510-560, 624-663` | refuse before spawning, take the `Writer`, `tokio::spawn`, `Served::Deferred` |
| One-at-a-time claim | `agent_worker.rs:689-708`; `:1061-1090` | `claim_is_free`; `ReprobeClaims::claim` with a releasing `Drop` |
| Row written before the terminal frame | `agent_worker.rs:1376-1380, 1436-1456` | `upsert_agent_box` then `Done` |
| Cancel, then abort | `agent_worker.rs:485-497` | token first, `grace * 2`, then the handle |
| Loop-freedom test | `crates/htui/src/store_worker.rs:1361-1400` | request at seq 1 deferred, seq 2 answered first |
| Refuse-before-spawn test | `agent_worker.rs:3410-3470` (`a_plan_is_refused_before_any_request_on_a_buffered_writer`) | `background_len() == 0`, the fixture saw nothing |
| Two writers never race one row | `agent_worker.rs:3540` (`a_probe_and_an_install_never_write_the_same_row_at_once`) | the claim, from both sides |
| Section with a pane, a cursor, a hint | `crates/htui/src/ui/tabs/settings/agents.rs:76-114, 236-269, 387-426, 507-611` | `InstallState`, `answer_consent`'s pass-the-rest rule, `pane`, `hint`, `on_reply` not clearing the wrong state |
| Section render test | `crates/htui/tests/settings.rs:59, 275-311, 540-560` | `render_section`, `probed_row`, `section_over`, `on_box_cell` |
| Harness end to end with a task still running | `crates/htui/tests/install.rs:530-566` | `key`, `drive`, assert the hint, `key("x")`, `drive_to_end` under `PATIENCE` |
| Scripted agent over a duplex | `crates/htui-agent/tests/probe.rs:1251-1270, 1304-1327` | raw JSON-RPC lines, no SDK type on the agent side |
| Real child, pid-checked kill | `crates/htui-agent/tests/acp_driver.rs:128-190`; `tests/probe.rs:77, 1042` | `/proc/<pid>/stat`, `KILL_WINDOW`; `executable()` then run through `sh` |
| Live suite | `crates/htui-agent/tests/agy_live.rs:1-40` | `#[ignore]`, the run line in the module doc, preconditions by name, nothing asserted that the vendor may change |
| `R-AGT-5` sweep | `crates/htui-agent/tests/extensibility.rs:76-131` | walk `crates/`, fail on a name outside seeds and tests |
| Redacted `Debug` | `crates/htui-agent/src/driver.rs:293-306`; `probe.rs:79-90` | counts and ids, never values |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-agent/src/driver.rs` | UPDATE | T1 | `DriverCaps.authenticate`; `AgentDriver::authenticate` with the default body; the "five operations" doc |
| `crates/htui-agent/src/error.rs` | UPDATE | T1 | `DriverError::Unsupported` |
| `crates/htui-agent/src/registry.rs` | UPDATE | T1 | `caps_from` arms |
| `crates/htui-agent/src/fake.rs` | UPDATE | T1 | `full_caps` |
| `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE | T1 | the `#[cfg(test)]` `DriverCaps` literal at `:688` |
| `crates/htui-agent/src/auth/mod.rs` | CREATE | T1 | `AuthFlow`, `AuthEvent`, `AuthChoice`, `AuthCall`, `AuthOutcome`, `AuthMethodInfo`, `BrowserPolicy`, `AUTH_IDLE_CAP` (D11, D13) |
| `crates/htui-agent/src/lib.rs` | UPDATE | T1, T5 | `pub mod auth` and re-exports |
| `crates/htui-agent/tests/driver_contract.rs` | UPDATE | T1 | the default body refuses; caps and operation agree |
| `crates/htui-agent/tests/extensibility.rs` | UPDATE | T1 | the sweep gains the auth vocabulary |
| `crates/htui-agent/src/acp/auth.rs` | CREATE | T2 | the wire flow (D12) |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | T2, T3 | `pub mod auth` + re-export (T2); `launch_in`, `AgentDriver::authenticate` for `AcpDriver` (T3) |
| `crates/htui-agent/src/acp/client.rs` | UPDATE | T2 | the test pinning `auth.terminal == false` and `elicitation == None` |
| `crates/htui-agent/tests/auth.rs` | CREATE | T2, T3, T5 | duplex cases, real-process cases, the hand-off cases, the hijack regression |
| `crates/htui-agent/src/auth/run.rs` | CREATE | T3, T5 | the end-to-end operation (T3); tap, URL and idle wiring (T5) |
| `crates/htui-agent/src/launch.rs` | UPDATE | T4 | `Spawned::tap_stderr` (D14) |
| `crates/htui-agent/tests/launch.rs` | UPDATE | T4 | the tap's cases |
| `crates/htui-agent/src/auth/url.rs` | CREATE | T5 | `first_url` (D15) |
| `crates/htui-agent/src/auth/browser.rs` | CREATE | T5 | `BrowserPolicy` application, `open_url` (D16, D17) |
| `crates/htui/src/store_worker.rs` | UPDATE | T6 | four variants, `name()` arms, `StoreReply::Auth`, `AuthFrame`, interception, `try_serve` refusal, loop-freedom test |
| `crates/htui/src/agent_worker.rs` | UPDATE | T6 | `LiveAuth`, `AuthCommand`, the four arms, `claim_is_free`/`probe`/`shutdown`/`finish_background` extended, `run_auth` |
| `crates/htui/src/testkit.rs` | UPDATE | T6 | the runtime-served request list in `drive` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T7 | `AuthState`, `a`, the chooser, the stream pane, `o`, `x`, hints, cell words |
| `crates/htui/tests/settings.rs` | UPDATE | T7 | section cases per frame and per refusal |
| `crates/htui/tests/auth.rs` | CREATE | T7 | harness end to end over a fixture agent |
| `crates/htui/tests/snapshots/settings__agents_demo.snap`, `settings__agents_empty.snap`, `settings__agents_probed.snap`, `settings__agents_unknown_row.snap` | UPDATE | T7 | the idle hint line gains `a authenticate` |
| `crates/htui-agent/tests/auth_live.rs` | CREATE | T8 | the live proof (D23) |
| `crates/htui-core/seeds/agent_agy.json` | UPDATE (conditional) | T8 | `credential.files` corrected **only** if the live run shows a different filename |
| `README.md`, `HANDOFF.md`, `crates/htui-agent/tests/agy_live.rs`, the PRD, this plan | UPDATE | T9 | D23 |

Not touched, on purpose: anything under `crates/htui-store/` and `crates/htui-store/migrations/`
(no migration; MOD-4's `0003` stays next), `crates/htui/src/keymap.rs` (no section scope, MOD-20
D19), `crates/htui/src/ui/overlay/*`, `crates/htui-agent/src/probe.rs` (the probe is consumed,
not changed — the outcome reaches it through `probe_agent` exactly as MOD-20's did),
`crates/htui-agent/src/acp/handshake.rs` (copied in shape, not edited),
`crates/htui-agent/src/install/*`.

## Milestone → task map

| PRD milestone | Outcome | Delivered by |
|---|---|---|
| 1 — The call, capability-gated | off any UI, `htui` asks an agent which ways it can be logged in and starts one; spawn, `initialize`, `authenticate`, the child killed on every exit; a transport without the call refuses | **T1**, **T2**, **T3** |
| 2 — The hand-off a TUI can survive | stderr reaches the caller as it happens, the URL is detected and offered, the adapter's own browser cannot touch the stream, cancel kills the child, nothing leaks | **T4**, **T5** |
| 3 — The action in the app | `a` in Settings: choose a method, watch the flow, open the link, cancel it, see the row change by a re-probe, the TUI responsive throughout; logout where advertised | **T6**, **T7** |
| 4 — Proven on a real login | `agy` on this box `unauthenticated` → `ready` through the app; the credential path confirmed or corrected; T34's chat runs; the README says what to do | **T8**, **T9** |

## Tasks

TDD per repo convention: the test that fails for the stated reason comes first, and a task whose
only test is "it compiles" is not a task. **Independence is by file set** and the sets are listed
for the mechanical gate; a dependency on a *type* from an earlier task without a shared file is
stated as "needs Tn landed", a sequencing note and not a file intersection.

Two tasks can start at once: **T1** (the seam) and **T4** (the tap) share no file. Then T2 → T3
serially after T1; T5 after T3 and T4; T6 → T7 serially; T8 after T5 and in parallel with T6/T7;
T9 last.

### T1: The seam — predicate, operation, types (milestone 1; independent; starts first)
- **Touched files**: `crates/htui-agent/src/driver.rs`, `crates/htui-agent/src/error.rs`,
  `crates/htui-agent/src/registry.rs`, `crates/htui-agent/src/fake.rs`,
  `crates/htui-agent/src/auth/mod.rs`, `crates/htui-agent/src/lib.rs`,
  `crates/htui/src/ui/tabs/chat/mod.rs`, `crates/htui-agent/tests/driver_contract.rs`,
  `crates/htui-agent/tests/extensibility.rs`
- **Tests first** (`tests/driver_contract.rs`): `a_driver_that_says_nothing_about_authenticate_refuses_it`
  — the file's `StubDriver` (`:42-60`), untouched, answers `Err(DriverError::Unsupported("authenticate"))`
  and its `caps().authenticate` is `false`; `caps_for_an_acp_row_advertises_authenticate_and_a_cli_row_does_not`
  over two hand-built `Agent` rows through `registry::caps_for`;
  `the_fake_transport_neither_claims_nor_answers_authenticate` (`test-support`);
  `every_registered_adapter_agrees_with_its_predicate` — for each id in
  `DriverFactory::with_acp().adapter_ids()` plus the fake, build a driver from a row of that
  transport and assert `caps().authenticate == !matches!(authenticate(flow).await, Err(Unsupported(_)))`
  — for the ACP driver at T1 the operation is still the default body, so this case is written to
  **fail** on `acp` and stays red until T3 lands (the sequencing pin). `DriverCaps::default().authenticate`
  is `false`. (`tests/extensibility.rs`) `the_auth_flow_names_no_vendor_or_method`: the
  `the_codebase_has_never_heard_of_zeta` walk (`:111-131`) over `crates/*/src` for
  `oauth-personal`, `oauth-business`, `gemini-api-key`, `agent-platform`, `accounts.google.com`,
  `antigravity-acp`, `GEMINI_HOME`, `GEMINI_API_KEY` — allowed only under `seeds/` and `tests/`.
  Vacuous today, load-bearing from T2.
- **Action**: D10 (`DriverCaps.authenticate`, `Unsupported`, the default body, the three literal
  sites, `driver.rs:34-35`'s "five operations" → six); D11's types and `BrowserPolicy` as a data
  enum, `AUTH_IDLE_CAP`, in `auth/mod.rs` with `#![warn(missing_docs)]`-clean docs; `pub mod auth`
  and re-exports in `lib.rs`. `AuthMethodInfo` and `AuthChoice` derive `Debug`,
  `Clone`, `Serialize` and `Deserialize` — `Debug + Clone` because both cross the request/reply
  boundary and `StoreRequest`/`StoreReply` derive exactly those (`store_worker.rs:49`, `:197`);
  nothing else in the module is serialisable, on purpose.
- **Validate**: `cargo test -p htui-agent --features test-support --test driver_contract --test extensibility`;
  `cargo test -p htui --features testkit --lib` (the chat literal); `cargo clippy --workspace --all-targets --all-features -- -D warnings`

### T2: The wire flow — `acp/auth.rs` (milestone 1; serial after T1)
- **Touched files**: `crates/htui-agent/src/acp/auth.rs`, `crates/htui-agent/src/acp/mod.rs`,
  `crates/htui-agent/src/acp/client.rs`, `crates/htui-agent/tests/auth.rs`
- **Tests first** (`tests/auth.rs`, through a duplex with a local `scripted_agent(stream, answers)`
  generalising `tests/probe.rs:1251-1270` to answer `initialize` and then `authenticate`/`logout`
  by method name, raw JSON-RPC, no SDK type on the agent side; the `initialize` result is the
  fixture's shape, `tests/fixtures/agy_acp_handshake.json`, with `authMethods` extended to carry
  `name` and `description`): `methods_carry_the_agents_own_name_and_description_in_order`;
  `a_terminal_typed_method_is_hidden_and_named_but_never_sent` (a `{"type":"terminal",…}` entry —
  the case asserts the agent saw **no** `authenticate` for it and `hidden` holds it);
  `an_unknown_method_type_is_offered_as_an_agent_method` (a `{"type":"future-kind",…}` entry: the
  schema folds it into `Agent` through its untagged arm, so the case **documents** that it is
  offered and sent, and would go red the day the SDK starts distinguishing it);
  `logout_is_offered_exactly_when_the_agent_advertises_it` (`agentCapabilities.auth.logout: {}`
  present vs absent); `choosing_a_method_sends_authenticate_with_that_id_and_completes` (the
  agent's log shows one `authenticate` whose `params.methodId` is the chosen id; outcome
  `Completed { call: Authenticate(id) }`); `a_json_rpc_error_is_refused_in_the_agents_own_words`
  (the agent answers `{"code":-32602,"message":"The FIXTURE_KEY environment variable must be set…"}`;
  outcome `Refused { message }` **containing that sentence** and the flow returned `Ok`);
  `choosing_logout_sends_logout`; `cancel_before_the_choice_ends_the_connection_and_answers_cancelled`;
  `cancel_during_authenticate_answers_cancelled` (the agent never answers; the token trips);
  `the_choice_sender_dropped_is_declined`; `an_agent_that_dies_after_initialize_is_a_transport_error_with_its_stderr`;
  `initialize_is_still_bounded_by_the_handshake_timeout`. **Real-process cases** (Linux, a
  `sh` fixture written with a local `executable()` and run through `sh`, pid from `Spawned::pid()`,
  liveness through `/proc` with the `acp_driver.rs:128-190` helpers copied locally):
  `no_child_survives_a_cancelled_flow`, `no_child_survives_a_refusal`,
  `no_child_survives_a_dropped_flow_future` (the future dropped mid-`authenticate`, the guard's
  `Drop` signals; polled within `KILL_WINDOW` as `acp_driver.rs` does). (`acp/client.rs` inline)
  `the_advertised_capabilities_claim_neither_terminal_auth_nor_elicitation` beside the existing
  case at `:173-195` (D21).
- **Action**: D12 — `pub async fn run(io: AcpIo, settings: &AcpSettings, flow_half: WireFlow) -> Result<AuthOutcome>`
  where `WireFlow` is the subset of `AuthFlow` the wire needs (`events`, `choice`, `cancel`,
  `handshake_timeout`); `Handshake::from_response` is **not** changed (D7); the method list is
  built from `init.auth_methods` through the schema's `id()`/`name()`/`description()` and the
  `Terminal` arm goes to `hidden`; logout from `init.agent_capabilities.auth.logout.is_some()`;
  the refusal text is `format!("{err}")` plus the stderr tail on a new line, the `handshake_error`
  shape. `pub mod auth` in `acp/mod.rs`, re-exported at the crate root as `acp::auth::run`.
- **Validate**: `cargo test -p htui-agent --features test-support --test auth`;
  `cargo test -p htui-agent --lib acp::client`

### T3: `AcpDriver::authenticate` — the operation end to end (milestone 1; serial after T2)
- **Touched files**: `crates/htui-agent/src/acp/mod.rs`, `crates/htui-agent/src/auth/run.rs`,
  `crates/htui-agent/src/auth/mod.rs`, `crates/htui-agent/tests/auth.rs`
- **Tests first** (`tests/auth.rs`): `the_acp_driver_resolves_the_recorded_launch_for_the_flow`
  — `AcpDriver::from_row_with_probe` over a row whose `agent_box.probe` records a launch with
  platform `args` (the `--uid=` shape, spelled with a made-up argument) and a command that is a
  file: the flow's spawn used **that** argv (the fixture agent writes its `$@` to an `argv` file
  in the tempdir before answering `initialize`, and the case reads the file — stderr is not yet
  an event stream at T3);
  `a_stale_recording_falls_back_to_resolution` (the recorded command gone — the `launch_for` rule
  at `acp/mod.rs:371-383`, now shared); `a_prepared_transport_runs_the_flow_without_a_process`
  (`AcpDriver::over(io, ..)` under `test-support`, the duplex from T2 — this is what makes
  `every_registered_adapter_agrees_with_its_predicate` (T1) go green for `acp`);
  `a_spawn_failure_is_a_spawn_error_naming_the_command`;
  `the_flow_reads_a_variable_already_in_the_spawn_environment` (D5: the fixture agent refuses
  `authenticate` unless `FIXTURE_KEY` is in its environment; a row whose `launch.env` carries it
  completes, one without is `Refused` — no second mechanism).
- **Action**: refactor `launch_for(spec)` (`acp/mod.rs:359-386`) into
  `pub async fn launch_in(&self, cwd: &Path) -> Result<ResolvedLaunch>` plus the `spec.env`
  extension, behaviour unchanged (the existing `acp_driver.rs` cases pin it); implement
  `AgentDriver::authenticate` for `AcpDriver` as `auth::run::authenticate(self, flow)`: `launch_in`
  (or the prepared io), `launch::spawn` over a **clone** of the resolved launch (the clone is where
  T5's `BrowserPolicy::apply` will write; at T3 `browser.rs` does not exist and `flow.browser` is
  carried but not read), `AcpIo::from_spawned`, `acp::auth::run`, the outcome returned unchanged.
  At T3 the events channel carries `Methods` only; `Line`/`Url` arrive with T5.
- **Validate**: `cargo test -p htui-agent --features test-support --test auth --test acp_driver --test driver_contract`

### T4: The stderr tap (milestone 2; **independent** — no file shared with T1–T3)
- **Touched files**: `crates/htui-agent/src/launch.rs`, `crates/htui-agent/tests/launch.rs`
- **Tests first** (`tests/launch.rs`, beside `spawn_runs_the_resolved_command_under_supervision`
  at `:461`): `a_tap_receives_every_stderr_line_in_order` (`sh -c` writing three lines to stderr
  with a pause between them); `a_tap_installed_late_replays_the_tail_first_then_streams`
  (two lines written, then the tap, then a third: the receiver yields exactly three, in order,
  none twice); `the_tail_is_unchanged_by_a_tap` (`stderr_tail()` after the child exits still
  holds the lines, still capped); `past_sixty_four_lines_the_tap_has_them_all_and_the_tail_the_last_sixty_four`;
  `a_second_tap_replaces_the_first` (the first receiver ends). Each case ends with the child
  reaped (`wait()`).
- **Action**: D14 — the shared `Option<UnboundedSender<String>>` beside the deque, the reader
  forwarding under the one lock, `pub fn tap_stderr(&mut self) -> mpsc::UnboundedReceiver<String>`
  with the replay-under-lock rule in its doc; `STDERR_TAIL_LINES` unchanged; `ChildGuard` gains
  nothing (the tap is taken before the guard, on the `Spawned`).
- **Validate**: `cargo test -p htui-agent --features test-support --test launch`

### T5: The hand-off — lines, URL, browser, opener (milestone 2; serial after T3 and T4)
- **Touched files**: `crates/htui-agent/src/auth/url.rs`, `crates/htui-agent/src/auth/browser.rs`,
  `crates/htui-agent/src/auth/run.rs`, `crates/htui-agent/src/auth/mod.rs`,
  `crates/htui-agent/src/lib.rs`, `crates/htui-agent/tests/auth.rs`
- **Tests first** (`tests/auth.rs`): **URL** — `first_url_finds_the_scheme_anywhere_in_the_line`
  (the live sentence shape with a made-up host; the URL with `?`, `&`, `%2F`, `#` kept whole);
  `first_url_stops_at_whitespace_and_brackets_and_strips_trailing_punctuation`
  (`<https://h/p>.`, `(https://h/p),`); `first_url_ignores_non_http_schemes` (`file:///x`,
  `javascript:alert(1)`, `ftp://h`); `first_url_is_none_for_a_line_without_one`;
  `a_url_is_reported_once_per_flow` (the fixture prints the same link twice → one `Url`, two
  `Line`s). **Browser policy** — `neutralised_sets_browser_to_an_existing_executable_that_exits_zero`
  (unix: the value is a path, `is_file`, and `Command::new(value).status()` is success);
  `the_policy_touches_the_auth_launch_only` (a `launch_in` result after a flow carries no
  `BROWSER`; `ProbeSnapshot::resolved` on a re-probe never carries it — asserted in T6 too);
  `neutralised_leaves_display_alone`. **The hijack regression** (the PRD's success metric, the
  fixture agent from T2 extended: on `authenticate` it runs `"$BROWSER" <url>` when `BROWSER` is
  set and otherwise writes `\x1b[?1049h\x1b[1;24r Acquisizione di https://…` to **stdout** before
  answering): `with_the_policy_the_protocol_stream_survives_the_agents_browser` (outcome
  `Completed`, the `Url` event present); `without_the_policy_the_same_agent_corrupts_the_stream`
  (`BrowserPolicy::Inherit` → `Err(Transport)` — the case that proves the policy is what
  protects, and the one that would have caught the live hijack). **Idle** —
  `an_idle_flow_is_killed_after_the_cap_and_reported_idle` (`idle: 200 ms`, the fixture holds
  `authenticate` forever, no child survives, outcome `Idle { after }`);
  `a_stderr_line_resets_the_idle_clock` (the fixture prints a line every 100 ms for a second under
  a 200 ms cap, then answers: `Completed`). **Opener** — `open_url_refuses_a_non_http_scheme_before_spawning`;
  `open_url_spawns_with_null_stdio_and_does_not_wait` (an injected opener command — a `sh` script
  that records its argv and `sleep 30`s — returns within milliseconds and the recorded argv is
  the URL; the script's stdout/stderr are not a tty). The opener command is injectable for the
  test (`OpenerCommand::{ Platform, Custom(PathBuf) }`), production is `Platform`.
- **Action**: D15 (`url.rs`), D16 and D17 (`browser.rs`: `BrowserPolicy::apply(&mut ResolvedLaunch)`,
  `open_url(url, opener) -> Result<()>` with the `cfg(unix)`/`cfg(windows)` split in one function
  and the Windows arm written per D22), D13 (the idle clock in `run.rs`), D14 wired: `tap_stderr`
  before `AcpIo::from_spawned`, the forwarding loop turning lines into `Line`/`Url` events and
  resetting the clock; re-exports.
- **Validate**: `cargo test -p htui-agent --features test-support`;
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`. **No Windows clippy line** (TOOL-3, D22): `browser.rs`'s `cfg(windows)` arm is reviewed by eye and listed for MOD-16.

### T6: Requests, replies, runtime (milestone 3; serial after T5)
- **Touched files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`,
  `crates/htui/src/testkit.rs`
- **Tests first** (inline `mod tests` of both files, the fixture agent as a `sh` script written by
  the test into a tempdir, the registry row's `launch.command` the script's path with no
  `discovery` so `probe_tools` completes and tier 2 runs — `probe.rs:1444-1448`):
  `the_loop_answers_other_requests_while_a_login_waits_for_a_human` (`store_worker.rs`, the
  `:1361-1400` shape: `AuthStart` at seq 1 over a fixture that never answers `authenticate`,
  `Workspaces` at seq 2, the first reply is seq 2; then `AuthCancel`);
  `try_serve_without_a_runtime_refuses_all_four_by_name`; `name_arms_are_stable`.
  (`agent_worker.rs`) `a_start_is_refused_before_any_spawn_on_a_buffered_writer`
  (`REGISTRY_ON_SERVER_ONLY`, `background_len() == 0`, the fixture never ran — its script writes a
  pid file when it starts); `a_start_on_a_cli_row_is_refused_by_the_predicate` (the `Unsupported`
  sentence, nothing spawned); `a_start_on_a_row_with_no_auth_methods_is_refused_by_name`;
  `a_start_while_an_install_runs_is_refused` and `an_install_plan_while_a_login_runs_is_refused`
  and `a_probe_while_a_login_runs_is_refused` (D19, three sides); `a_second_start_while_one_runs_is_refused`;
  `methods_arrive_at_the_start_seq_and_the_rest_at_the_choose_seq`;
  `a_choose_with_no_flow_pending_is_refused`; `done_carries_the_probes_status_and_the_row_was_written_first`
  (the fixture creates the credential file the row's `credential.files` names when it answers
  `authenticate` → the re-probe reads `ready`, `stored_box()` shows it before `Done` was received);
  `success_into_a_box_with_no_credential_still_reads_unauthenticated` (the PRD's "probe is the
  authority" metric: the fixture answers success, creates nothing, `Done { status: Unauthenticated }`);
  `a_refusal_writes_nothing_and_keeps_probed_at` (D6: the row before and after, byte for byte);
  `cancel_answers_cancelling_then_the_stream_ends_cancelled_and_no_child_survives`;
  `shutdown_cancels_then_aborts_a_running_login_and_no_child_survives`;
  `the_agent_row_is_byte_identical_before_and_after_a_login` (`agents()` twice, the `Agent` halves
  equal); `no_frame_carries_anything_but_ids_text_and_status` (every `AuthFrame` produced by a
  full flow serialises with `Debug` to text that contains neither the fixture's credential value
  nor the token file's contents — the fixture's "credential" is a known sentinel string);
  `open_is_forwarded_to_the_live_flow_and_answered_at_its_own_seq` (the injected opener records
  the URL); `a_flow_holds_the_reprobe_claim_for_its_row` (the `:2695` shape:
  `claims.claim((agent_id, box_id))` is `None` while the flow runs and `Some` after).
  (`testkit.rs`) no new test; `drive`'s list gains the four variants, and T7's harness cases are
  what prove it.
- **Action**: D18, D19 — four variants, `name()` arms, `StoreReply::Auth`, `AuthFrame`, the
  interception at `:574-591`, the `try_serve` refusal at `:426-436`, `LiveAuth`, `AuthCommand`,
  `auth_start`/`auth_choose`/`auth_open`/`auth_cancel`, `claim_is_free`/`probe`/`serve`'s sweep/
  `shutdown`/`finish_background` extended, `run_auth` as a plain `async fn` over an owned
  `AuthArgs` (the `run_install` rule) with the forwarding loop and the address switch, the re-probe
  and the write on `Completed` only, `testkit.rs:184-194`.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui --features testkit --lib`

### T7: The Settings action (milestone 3; serial after T6; no file shared with T6)
- **Touched files**: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/auth.rs`, `crates/htui/tests/snapshots/settings__agents_demo.snap`,
  `crates/htui/tests/snapshots/settings__agents_empty.snap`,
  `crates/htui/tests/snapshots/settings__agents_probed.snap`,
  `crates/htui/tests/snapshots/settings__agents_unknown_row.snap`
- **Tests first** (`tests/settings.rs`, the `probed_row`/`section_over`/`render_section` style):
  `a_on_a_cli_row_is_refused_by_name_and_sends_nothing`; `a_on_an_unprobed_row_says_probe_first`;
  `a_on_a_row_with_no_auth_methods_is_refused_by_name` (a `probed_row` whose `handshake.auth_methods`
  is `[]`); `a_on_an_unauthenticated_row_sends_auth_start_and_the_cell_reads_starting`;
  `a_methods_frame_renders_the_chooser_with_names_descriptions_logout_and_the_hidden_count`;
  `j_k_and_enter_choose_and_send_the_choice`; `esc_in_the_chooser_cancels`;
  `digits_and_q_pass_through_the_chooser` (the MOD-20 review rule);
  `line_and_url_frames_render_the_last_six_lines_and_the_link`;
  `o_sends_auth_open_with_the_last_url_and_is_refused_without_one`; `x_sends_auth_cancel_and_the_cell_reads_cancelling`;
  `done_clears_the_state_notes_the_status_and_re_reads_the_registry`; `refused_shows_the_agents_sentence`;
  `an_agents_reply_during_a_login_does_not_clear_the_state` (hazard H-17's twin);
  `r_and_i_are_refused_while_a_login_runs`; the four snapshots re-accepted for the hint line.
  (`tests/auth.rs`, harness over a fixture agent as in T6, with a local `until(harness, pred, PATIENCE)`
  helper that alternates `drive()` and a 10 ms sleep because a runtime task makes progress only
  between drives): `a_then_enter_logs_in_and_the_cell_reads_the_probes_verdict` (the fixture
  creates the credential file → `ready`); `a_then_enter_into_no_credential_still_reads_unauthenticated`;
  `x_mid_login_ends_cancelled_and_the_cell_goes_back` (the `install.rs:530-566` shape; then
  `drive_to_end`, which cancels and aborts through `finish_background`); `o_opens_the_link_through_the_injected_opener`;
  `the_agent_table_is_unchanged_by_a_login` (the rendered `name`/`transport`/`billing` columns
  before and after).
- **Action**: D20.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui --features testkit`;
  `cargo insta review` of the four snapshots

### T8: The live proof (milestone 4; after T5; no file shared with T6/T7)
- **Touched files**: `crates/htui-agent/tests/auth_live.rs`, `crates/htui-core/seeds/agent_agy.json`
  (conditional, D23)
- **Test first**: `#[ignore]`, the run line in the module doc, preconditions by name (the seed row
  resolves through the production probe; the status is `unauthenticated` — a `ready` box stops by
  name, as `agy_live.rs` case 4 does). It drives `AcpDriver::authenticate` with production
  settings, prints every `Methods`/`Line`/`Url` event with `--nocapture`, prompts the maintainer
  on stderr to open the link (or opens it through `open_url` when `HTUI_LIVE_OPEN=1` is in the
  environment of the test — read with `std::env::var`, never set), waits up to `AUTH_IDLE_CAP`,
  and then re-probes through `probe_agent` with `SpawnTier2`. Asserts per D23; prints the
  directory listing so the seed's `credential.files` can be confirmed or corrected in the same
  task. A second `#[ignore]` case logs out and re-probes to `unauthenticated`, so the maintainer's
  box is not left in a state the suite cannot reproduce — run only when asked (`HTUI_LIVE_LOGOUT=1`).
- **Action**: the test; the seed edit **only if** the live run shows the vendor writes a
  different filename, recorded as a finding in the phase note either way.
- **Validate**: `cargo test -p htui-agent --features test-support --test auth_live -- --ignored --nocapture`;
  then MOD-2's T34 as written: `cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture`
  (case 4's precondition inverts once the box is `ready`; what it prints is the answer to the three
  §11.14 questions, recorded in the phase note, not asserted)

### T9: Close-out (serial, last)
- **Touched files**: `README.md`, `HANDOFF.md`, `crates/htui-agent/tests/agy_live.rs`,
  `.claude/prds/mod-21-in-app-agent-auth.prd.md`, this plan
- **Action**: D23 — `README.md:274-280` becomes "Logging an agent in": `Settings > Agents`, `j`/`k`,
  `a`, what the chooser shows, `o` and the link, `x`, the idle cap, what changes the row (a re-probe)
  and what never does (`htui` holds no credential), API-key methods and MOD-10; `agy_live.rs`'s
  module doc ("the maintainer authenticates the server by hand", `:4-11` and the case-4 note)
  amended to name the in-app flow; `HANDOFF.md`'s MOD-2 T34 note (`:233-243`) answered with what
  T8 found, MOD-21's line gaining a phase note per milestone (lifecycle step 4), the summary table
  untouched (MOD-21 stays open until the write-up); the PRD's four rows `complete`. Maintainer
  acceptance step recorded, not tested: press `a` on the `agy` row on this box and report the cell
  afterwards.
- **Validate**: the full block below

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features

# T8, explicitly (interactive; opens nothing unless HTUI_LIVE_OPEN=1; burns no model tokens)
cargo test -p htui-agent --features test-support --test auth_live -- --ignored --nocapture

# MOD-2 T34, after T8 (the adapter this box holds; case 4 stops by name on a `ready` box)
cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture
```

Per milestone: **1** — `cargo test -p htui-agent --features test-support --test driver_contract --test extensibility --test auth --test acp_driver`;
**2** — `cargo test -p htui-agent --features test-support` (adds `launch`); **3** — the `htui` line
with the Postgres variables and `cargo insta review`; **4** — the two `--ignored` lines, then the
full block.

`USERNAME=htui-ci` is TOOL-2's standing workaround. **There is no Windows clippy line** (D22): TOOL-3
records that `cargo clippy --target x86_64-pc-windows-msvc` cannot build on this box for either
crate, so `browser.rs`'s `cfg(windows)` arm is reviewed by eye at the `rust-reviewer` gate and its
runtime facts are MOD-16's, listed in D22 by name. No task adds a migration, so
`cargo sqlx prepare --check` is unaffected; it is run once at T9 to prove that.

## Hazards

| # | Hazard | Failure it prevents | Code shape |
|---|---|---|---|
| H-1 | **The orphan with a listener** (MOD-2's class, longer-lived) | An adapter mid-OAuth outlives its flow: a cancel, an idle cap, a shutdown or an aborted task leaves `agy_acp_server` running with `127.0.0.1:<port>` open, holding a redirect that will never be consumed | `ChildGuard` in `acp::auth::run`'s frame; `kill_and_reap` on every exit; `task.abort()` after; the outer `select!` on the token so a cancel reaches the kill even while the foreground future is parked; `shutdown` cancels then aborts; T2's three pid cases, T5's idle case, T6's cancel and shutdown cases |
| H-2 | **The stdout hijack** (observed live) | The adapter's own opener falls through to a terminal browser that writes alt-screen sequences into the JSON-RPC stream; the SDK decodes garbage; the flow dies with a decode error the user cannot act on | D16's `BROWSER` on the auth spawn; T5's regression pair — the same fixture with and without the policy — is the pin that turns the live finding into a red test |
| H-3 | **The opener seizing the terminal** | `xdg-open` on a display-less box falls through to `w3m`/`lynx` on `htui`'s own tty | D17: all three stdio `null()`; the opener child is not `htui`'s process-group leader; T5's injected-opener case asserts a non-tty stdout |
| H-4 | **Killing the user's browser** | An opener supervised through `launch::spawn` and timed out would kill the process group — and `xdg-open` may have exec'd the real browser as its child | D17: a plain `Command`, never `launch::spawn`, never a kill; reaped by a spawned `wait()` |
| H-5 | **A credential on a frame, in a log or in a row** (`R-SEC-2`, `R-ID-7`) | A stderr line carrying a token, a `Debug` of `LiveAuth` printing the last URL with its OAuth `state`, a `tracing::info!` with the line | `AuthEvent`/`AuthFrame` carry text the user already sees and nothing else; `LiveAuth`/`AuthArgs` `Debug` print ids and counts (`driver.rs:293-306`'s rule); stderr lines are rendered, never logged above `trace`, never persisted; the only write is `probe_agent`'s row, which records a **tier**; T6's sentinel case |
| H-6 | **The address switch and a late choice** | `AuthChoose` served after the flow ended, or twice; frames at a seq `is_fresh` no longer passes | `AuthCommand` over a channel whose closed state is the refusal ("this login has ended", `agent_worker.rs:718-724`'s shape); the wire half accepts one choice (a `oneshot`), a second `Choose` is answered `Failed` by the task |
| H-7 | **`Agents` clearing the login state** (MOD-20 H-17) | A scope change during the browser round trip resets the pane and the running flow has no key to cancel it with | `on_reply(Agents)` touches `agents`/`unavailable`/`probing`/cursor only; `auth` is cleared by auth frames alone; T7's case |
| H-8 | **A line without a newline** | A prompt written without `\n` never reaches the tap; the user sees nothing and the flow idles out | Accepted and stated: the live line ends in a newline, the tail has the same limit today, and the idle cap bounds the cost; recorded for the PRD's "a second agent surfaces the URL elsewhere" risk |
| H-9 | **The idle cap under a slow human** | Ten minutes of silence while the user finds their password kills a live login | `AUTH_IDLE_CAP` is measured from the last sign of life, the pane's hint says it, the URL stays on screen, and a second `a` restarts; the cap is a constant a later item can raise, not a timeout wired into `handshake()` |
| H-10 | **The API-key refusal read as a crash** | `-32602` for `gemini-api-key` renders as "login failed" and the user does not learn which variable to set | `Refused { message }` is its own frame and its own notice, the agent's sentence verbatim (D5, D11); the README names MOD-10 as the injection seam |
| H-11 | **`BROWSER` leaking into a chat or the probe** | A chat spawn inherits the neutraliser and a legitimate `xdg-open` from a tool call is silenced; or the re-probe records `BROWSER` in `probe.resolved` and every later spawn carries it | `BrowserPolicy::apply` on a clone of the launch inside `auth::run` only; `launch_in` is shared but returns the row's env; T5's `the_policy_touches_the_auth_launch_only`, T6's snapshot assertion |
| H-12 | **The fixture agent and `ETXTBSY`** | A script written and executed in the same test intermittently fails to spawn | Run through `sh <script>` as `tests/probe.rs:1042` records; the pid file the fixture writes is what T6's "nothing spawned" cases read |
| H-13 | **The harness and a task waiting for a human** | `drive_to_end` calls `finish_background(CHAT_END)` — a flow parked on the chooser would wait the whole limit, then be cancelled and aborted, and a test would pass for the wrong reason or hang | `finish_background` handles `auth` as it handles `install` (cancel then abort under the limit); T7 uses `until()` + `drive()` while a flow is live and `drive_to_end` only to finish; no `start_paused` in any file that spawns a process |
| H-14 | **The three `DriverCaps` literal sites** | T1 does not compile until every struct literal names the new field | Listed in the Prerequisite: `registry.rs:144,158`, `fake.rs:104`, `chat/mod.rs:688`; `driver_contract.rs:49` is `Default` and is untouched |
| H-15 | **`elicitation/create` from an agent that ignores the advertised capabilities** | The flow hangs on a request `htui` has no handler for | The SDK answers "method not found" itself (`incoming_actor.rs:620`); D21 records it; the fixture agent's `elicitation` case in T2 is optional and marked as such |
| H-16 | **Two claims, one row** | A chat started during the round trip re-probes the row mid-login (`ReprobeClaims`) while `claim_is_free` only knows about installs | D19: `LiveAuth` holds a `ReprobeClaim` from `AuthStart`; T6's `a_flow_holds_the_reprobe_claim_for_its_row` |
| H-17 | **Logout without a re-probe** | A successful `logout` leaves the row `ready` until `PROBE_TTL` | `Completed { call: Logout }` takes the same re-probe arm as `Authenticate` (D18); T6's logout case asserts `Done { status: Unauthenticated }` on a fixture that deletes its credential file |
| H-18 | **The seed's credential filename** | The live login succeeds and the row still reads `unauthenticated` because `acp_token.json` is not what the vendor writes | D23: `auth_live.rs` prints the directory and fails by name on the mismatch; T8 corrects the seed and the phase note records the finding |
| H-19 | **Windows written blind** | `cmd.exe /c exit 0` is not honoured, `Start-Process` needs an interactive session, `CREATE_NO_WINDOW` on PowerShell hides a prompt | D22 names each for MOD-16; nothing on Windows is claimed as verified anywhere in this plan |
| H-20 | **A vendor string in source** (`R-AGT-5`) | A method id or the Google host creeps into a match arm or a hint | T1's sweep over `crates/*/src` for the eight strings; every method name on screen comes from the wire |

## Verified claims

Filled by the `/handoff-run` step-3.5 fact-check pass on 2026-09-09, independently of the
Prerequisite section above. **104 `file.rs:line` citations were checked mechanically** (each: does
the claimed symbol or text appear within ±8 lines of the cited line), plus the toolchain probes
below. Three amendments were applied to this plan before the CONFIRM gate and are marked
**amended**; they are listed here rather than silently folded in.

| Claim | Verdict | Evidence |
|---|---|---|
| 104 line citations across `crates/`, the SDK and the schema resolve to what they claim | **99 verified, 5 investigated, 0 false** | `chk` sweep; the 5 were `agent_worker.rs:510`/`:624`, `agents.rs:507` (function heads, cited content 6–18 lines in — the citation is a range and correct), `jsonrpc.rs:850` ("Unknown requests receive Method not found automatically", matched case-sensitively by the sweep), and `traits.rs:108` below |
| `WriteStore::upsert_agent` is at `traits.rs:108` | **verified, path amended** | No `crates/htui-store/src/traits.rs` exists; the trait is `crates/htui-core/src/store/traits.rs:57`, `upsert_agent` at `:108`. D18 now names the qualified path |
| A default trait body on `AgentDriver` returning `DriverFuture<'a, T>` keeps the trait dyn-compatible (D10) | **verified by compile probe** | Standalone probe mirroring the seam (`DriverFuture` alias, `Box::pin(async move …)` default body, `Box<dyn AgentDriver>`, `&dyn AgentDriver`) compiles clean under `rustc 1.98.1`, the repo's toolchain |
| `AuthMethod`'s `Terminal` arm can be matched to build `hidden` (D11, T2) | **verified for `terminal`; a defect found and amended for anything else** | The enum is `#[non_exhaustive]` (`schema/v1/agent.rs:574`) so the match needs a wildcard, **and that wildcard is unreachable for wire data**: `Agent` is `#[serde(untagged)]`, so an unrecognised `type` deserialises as `Agent`. Measured against the real crate in a scratch project: `{"type":"future-kind","id":"x","name":"X"}` → `Agent(AuthMethodAgent { id: "x", … })`; `{"type":"terminal",…}` → `Terminal(…)`. The first amendment written here (hide unknown kinds) was itself false and was replaced: D11 and T2 now say a future kind is offered as an agent method, and T2's case documents that rather than asserting the impossible |
| `AuthChoice` / `AuthMethodInfo` can cross the request/reply boundary (D18) | **verified with a gap — amended** | `StoreRequest` and `StoreReply` both `#[derive(Debug, Clone)]` (`store_worker.rs:49`, `:197`); T1 named only `Serialize`/`Deserialize`. Derives amended |
| `CancellationToken` is available to `htui-agent` (D11, D13) | **verified, with a caveat recorded** | Already used by `install/{run,fetch,archive}.rs`, so it compiles today — but the workspace declares `tokio-util = { features = ["compat"] }` (`Cargo.toml:42`), so `sync` arrives by feature unification from another dependency. Not introduced by this item; if it ever breaks it breaks MOD-20's installer identically |
| `DriverFactory::adapter_ids()` exists for T1's contract test | **verified** | `registry.rs:83` |
| `ProbeEnv::host`, `without_versions`, `SpawnTier2` exist for the re-probe recipe (D18) | **verified** | `probe.rs:99`, `:139`, `:1234` |
| The four Settings snapshots T7 re-accepts exist under those names | **verified** | `crates/htui/tests/snapshots/settings__agents_{demo,empty,probed,unknown_row}.snap` |
| `open 5.4.3`'s Windows spelling is `powershell.exe … "Start-Process -FilePath $env:OPEN_RS_TARGET"` with the target in the environment (D17) | **verified verbatim** | `open-5.4.3/src/windows.rs:75-80`; the crate is in the local registry cache, so the planner's reading is reproducible |
| `url 2.5.8` is present transitively only (D15 needs no parser) | **verified** | `Cargo.lock:4370-4371`; no direct dependency in any manifest |
| `/bin/true` and `/usr/bin/xdg-open` exist on this box (D16, D17) | **verified, one phrasing corrected** | Both exist. `/usr/bin/open` also exists but is a Debian alternative, **not** macOS's `open`; D16/D17 use `open` on macOS only, so nothing depends on it — the Prerequisite's phrasing should not be read as "the macOS opener is testable here" |
| Task independence: T1 ∥ T4, and T8 ∥ T6/T7 | **verified by set intersection** | T1 {driver, error, registry, fake, auth/mod, lib, chat/mod, driver_contract, extensibility} ∩ T4 {launch.rs, tests/launch.rs} = ∅; T8 {auth_live.rs, agent_agy.json} ∩ (T6 ∪ T7) = ∅. Every other pair is declared serial, and T5 ∩ T1 = {lib.rs} is consistent with that |
| No task adds a migration | **verified** | No task touches `crates/htui-store/migrations/`, `cache_migrations/` or `local_migrations/`; MOD-4's `0003_orchestration.sql` stays next |

## Acceptance

1. `caps_for` says `authenticate` for an `acp` row and not for a `cli` one; a driver that says
   nothing refuses with `Unsupported`; over a duplex the flow lists the agent's methods with their
   names, hides a terminal-typed one, sends `authenticate` with the chosen id, relays a JSON-RPC
   refusal verbatim, and kills its child on every exit, pid-checked (milestone 1).
2. Stderr lines reach the caller as they arrive, a URL in them is detected once and only for
   `http(s)`, the adapter's own browser cannot write into the stream (the regression pair), an idle
   flow is killed and reported, and the opener runs with no tty and no process group of `htui`'s
   (milestone 2).
3. `a` in Settings shows the chooser fed by the live `initialize`; `Enter` starts the call; the
   pane streams the agent's lines and offers `o`; `x` cancels; the loop answers other requests
   meanwhile; the cell afterwards is the **probe's** status, and a success into a box with no
   credential still reads `unauthenticated`; the `agent` row is byte-identical before and after
   (milestone 3).
4. `agy` on this box goes `unauthenticated` → `ready` through the app; the seed's credential path
   is confirmed or corrected from what the flow left on disk; `agy_live.rs` runs against the
   logged-in server and the phase note records what it printed; the README says what to do
   (milestone 4).
5. No source file outside seeds and tests names an agent, a method id or a vendor host (`R-AGT-5`);
   no frame, log line or row carries a credential value (`R-SEC-2`, `R-ID-7`); `htui-store` and
   `migrations/` unchanged.
6. Full suite green on Linux with Postgres live. **No Windows lint claim** (TOOL-3, D22).
7. `rust-reviewer` gate clear, with `browser.rs`'s `cfg(windows)` arm reviewed by eye and said so.

## Close-out

Per `.claude/rules/workflow-docs.md` lifecycle step 4: MOD-21 stays one open `HANDOFF.md`
checklist line with a **phase note per milestone** (each naming the commit, the tests that prove
it, and any amendment to this plan), and archives as one write-up when milestone 4 lands. The
PRD's four rows move to `complete` as each phase note is written. The note for milestone 4 records
the credential-file finding (seed corrected or confirmed), what `agy_live.rs` printed for ANA-4
§11.14's three questions, and that `docs/ANA-4.md` §4.5's "`htui` cannot log in non-interactively"
is reversed in fact as well as in `R-AGT-9`. MOD-2's T34 is answered in MOD-2's own entry, not
restated. Push only when agreed.
