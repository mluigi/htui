# ANA-16 - Agent execution environments (Docker, remote shell)

> **Scope note:** "Research how to implement ways to run an agent in a Docker container (local and
> remote) and in a remote shell. Note this would require a central server with htui as just the
> interface." (`HANDOFF.md:62`)
>
> **Requirements addressed:** `R-ID-2`, `R-ID-3`, `R-ID-7`, `R-STO-1`, `R-STO-4`, `R-BOX-1..4`, `R-AGT-1`,
> `R-AGT-5`, `R-AGT-6`, `R-AGT-9`, `R-ORCH-8`, `R-ORCH-10..12`, `R-HIS-1`, `R-SEC-2`, `R-MCP-1`,
> `R-NF-1..3`.
>
> **Status (2026-09-24): concluded, amended the same day after a maintainer challenge (§10).**
> Verdict: feasible, and phased. Running away from the TUI needs a process that outlives it: a
> **headless `htui` worker per executing box**. Phase 1 has those workers talk only to Postgres
> (O3), which already coordinates N concurrent writers (§6.1). Phase 2 puts a **self-hosted
> control-plane server** (O5a) between workers and Postgres, owning dispatch, enrolment,
> config/secret distribution, relay and version skew, while the TUI stays a direct Postgres client.
> A full server in front of every client (O5b) is rejected. Docker is an execution environment *of*
> a box; a remote shell is how a remote box's worker is provisioned, not the agent transport.

Code citations are against HEAD `ba68682`; §6-§10 were re-verified against `ac6c2bc`.

---

## 1. Context and problem statement

Today every agent is a local child process of the TUI, over stdin/stdout pipes, in one tokio
process (`crates/htui/src/main.rs:10`, `crates/htui/src/lib.rs:65-99`). The item asks for three
further placements of the agent:

1. **Docker, local:** agent in a container on the machine running `htui`.
2. **Docker, remote:** agent in a container on another machine.
3. **Remote shell:** agent on another machine reached over SSH.

The item asserts that this "would require a central server with htui as just the interface". That
assertion is tested in §6, not assumed.

What the three placements share: the agent's stdio, working tree, credentials, process-tree kill and
lifetime all move off the TUI's host or out of its process. §2 lists every place the code assumes
they do not.

## 2. Current launch path in htui

**Topology.**
- `htui::run` (`crates/htui/src/lib.rs:65`) starts the store connection (`:80`), spawns the store
  worker (`:91`) and runs the event loop (`:99`). The UI talks to the worker only over two unbounded
  mpsc channels.
- `store_worker::spawn` (`crates/htui/src/store_worker.rs:1052`) calls `spawn_with` over
  `AgentRuntime::production()`. That task is the only owner of `Backend`, and the `AgentRuntime`
  lives inside it (`:1060-1063`).
- Chat, probe, install and auth requests go to `runtime.serve` (`:1338-1351`). `Served::Start` does
  `runtime.attach(step_id, tokio::spawn(task))` (`:1355-1357`). **A live chat is a task inside the
  TUI process, so closing the TUI ends it.**
- `crates/htui/Cargo.toml` depends on `htui-agent`, `htui-core` and `htui-store`, **not
  `htui-orch`**. The only mention of `htui_orch` in `crates/htui/src` is Sentry's `in_app_include`
  (`crates/htui/src/main.rs:18`). The step-graph engine has no production caller yet.
- The planned caller is MOD-4 milestone 6's `crates/htui/src/run_worker.rs`: "one per box, owns the
  lease refresh, spawns one engine task per active run" (`docs/ANA-2.md:1681-1682`). It is placed in
  the **TUI crate** and emits progress on the TUI's reply channel (`docs/ANA-2.md:1687-1692`).
- `htui-orch` itself is headless by construction. It depends on `htui-core` and `htui-agent`, never
  on `htui-store`, and it is generic over `S: WriteStore` (`crates/htui-orch/src/lib.rs:3-6`). It
  exists as a separate crate so that "a headless `htui` worker per box polling Postgres" can link it
  without `ratatui` or `crossterm` (`docs/ANA-2.md:1650-1654`).
- The CLI has `--demo`, `--log`, `--set-dsn`, `--clear-dsn` and `--offline`
  (`crates/htui/src/cli.rs:12-31`). There is no headless or worker mode.

**Driver seam.**
- `trait AgentDriver` (`crates/htui-agent/src/driver.rs:363`) and `trait AgentSession` (`:402`).
- `SessionSpec` (`driver.rs:250`) carries `cwd: PathBuf` (`:256`), `extra_dirs` (`:258`), `env`
  (`:260`) and `mcp: Vec<McpServerSpec>` (`:266`). `McpServerSpec` is a local command plus env
  (`:220-229`). **Every path in the spec is a path on the host that runs the driver.**
- `trait TransportBuilder` (`crates/htui-agent/src/registry.rs:29`).
  `DriverFactory::production()` (`:76`) registers the ACP and CLI stream-json adapters.
- The orchestrator's seam is `pub type DriverFor` (`crates/htui-orch/src/engine.rs:253`), held in
  `EngineParts` (`:321`). The engine builds its `SessionSpec` at `engine.rs:4350-4366`: `cwd` and
  `extra_dirs` come from the isolator, `env: BTreeMap::new()` (`:4359`), `mcp: Vec::new()`,
  `permission: PermissionPolicy::default()` (`:4363`). It then calls `driver.start` and
  `pump(...)` (`:4368-4369`).

**Launch and spawn.**
- `launch_from` (`crates/htui-agent/src/launch.rs:516`) reuses the probe-recorded command only if
  `probe::is_file` says it exists **on the local disk** (`:522`). Otherwise it resolves the tool now.
- Tool resolution:
  - Tier 1 is the environment variable `HTUI_TOOL_<NAME>` (`crates/htui-agent/src/tools.rs:42`,
    read at `:74`).
  - Tier 2 is a probe against `ProbeEnv::host(cwd)` (`tools.rs:95`). `ProbeEnv::host`
    (`crates/htui-agent/src/probe.rs:99`) snapshots the process's own environment and home.
  - `platform_key()` (`probe.rs:171`) is the host's OS and architecture.
  - `HTUI_AGENTS_ROOT` (`probe.rs:191`) defaults to `data_local_dir()/htui/agents` (`:205`).
- `spawn(launch, cwd)` (`launch.rs:1019`) calls `spawn_supervised` (`:1106`). That builds a
  `tokio::process::Command` with `.args`, `.envs(&launch.env)` and `.current_dir(cwd)`, pipes all
  three stdio streams (`:1114-1121`), and makes the child a process-group leader on unix
  (`:1125-1130`). **There is no `env_clear`**: the child inherits htui's whole environment,
  including `HOME` and `PATH`.
- `Spawned` (`launch.rs:638`): `signal` (`:792`) and `kill_tree` (`:816`) act on local pids and
  process groups. `ChildIo` (`:861`) is the transport-neutral pair of byte streams.

**Transports.**
- ACP: `io()` spawns locally (`crates/htui-agent/src/acp/mod.rs:366`). `session/new` sends
  `spec.cwd` and `extra_dirs` as paths that the agent interprets on its own filesystem (`:1096`).
- ACP `fs/read_text_file` and `fs/write_text_file` are served **by htui against its own disk**,
  behind `fs::guard(path, cwd, extra_dirs)` (`acp/mod.rs:1476`, `:1501`; guard at
  `crates/htui-agent/src/acp/fs.rs:43`). The seeds advertise
  `fs_read: true, fs_write: true, terminal: false` (`crates/htui-core/seeds/agent_agy.json:56`,
  `crates/htui-core/seeds/agent_claude.json:34`).
- `IoSource::Prepared` (`acp/mod.rs:148`, `:289`, `:356`) already runs a driver over an injected stream.
  It is compiled only under the `test-support` feature (`:147`), but it is the proof that a driver
  does not care where its bytes come from.
- CLI: `argv` (`crates/htui-agent/src/cli/mod.rs:112`) passes each extra dir as `--add-dir <host
  path>` (`:153`). It spawns at `:363`. Cancel is `spawned.signal(StopSignal::Interrupt)`
  (`:1138`). The seed runs `permission_mode: "acceptEdits"`
  (`crates/htui-core/seeds/agent_claude_cli.json:21`).

**Working directory.**
- Chat `cwd` is `std::env::current_dir()` of the TUI (`crates/htui/src/agent_worker.rs:1174`), with
  `extra_dirs` empty, "until MOD-13 and MOD-7 give a project a repo path per box" (`:1237`).
  Probe, plan, install and auth use the same `current_dir()` (`:753`, `:854`, `:899`, `:1006`).
- Orchestrator trees: `Isolator::prepare` (`crates/htui-orch/src/isolate.rs:141`, `:157`) returns
  `Prepared` (`:83`). The real isolator puts trees under `session_dir(scratch_root, run, step)`
  (`crates/htui-orch/src/isolate/real.rs:203`) and creates them with a **local** `git worktree add`
  under an admin lock (`real.rs:251`, `:278`, `:542`).
- The verifier runs `("sh", "-c")` locally with `.current_dir(cwd)`
  (`crates/htui-orch/src/verify.rs:52`, `:330`).

**Credentials and login.**
- The agy seed's credential probe checks `GEMINI_API_KEY` and token files under `%GEMINI_HOME%` and
  `~` (`crates/htui-core/seeds/agent_agy.json:46-50`). They are expanded against the local host by
  `resolve_credential` (`probe.rs:894`).
- Claude relies on the inherited `HOME`. No code in `crates/*/src` names `.claude` or
  `CLAUDE_CONFIG_DIR`.
- Login spawns the ACP launch locally (`crates/htui-agent/src/auth/run.rs:73`, `:86`). The browser
  opener runs `xdg-open` on the TUI host (`crates/htui-agent/src/auth/browser.rs:102`, `:165`).
- The Postgres DSN lives in the OS keyring, service `htui` / user `postgres-dsn`
  (`crates/htui-store/src/secret.rs:20-23`). The Linux keyring backend is `sync-secret-service`
  only (`Cargo.toml:45-46`), which needs a D-Bus session with a secret-service daemon.

**Box identity.** `box.toml` carries the minted box id (`crates/htui-store/src/identity.rs:18`).
`register_box` (`crates/htui-store/src/pg/mod.rs:353`, called at `:420`) records the TUI host.
`agent_box` holds per-(agent, box) enablement, probe and quota
(`crates/htui-store/migrations/0001_init.sql:112`).

**Queue primitives already in the schema.**
- `run.target_box_id NOT NULL` ("R-ORCH-12 reserved") and `executing_box_id`
  (`0001_init.sql:455-456`).
- `lease_box_id`, `lease_owner` and `lease_expires_at` (`0003_orchestration.sql:41-43`), with TTL
  120 s and refresh 60 s (`:135-136`).
- `claim_run` does `SELECT ... FOR UPDATE` on the run and on the box
  (`crates/htui-store/src/pg/write.rs:2468-2497`). Also `refresh_lease` (`:2622`) and
  `adopt_runs` (`:2668`).
- `session_event` has `PRIMARY KEY (run_step_id, seq)` and includes the kinds
  `permission_request` and `permission_answer` (`0001_init.sql:513-526`).
- Nothing in `crates/` uses `LISTEN`, `NOTIFY` or `PgListener`.

**A gap that matters to any headless path.** The engine drives sessions through `pump`
(`crates/htui-agent/src/record.rs:1684-1703`), which has no permission handling. ACP `next_event`
returns an error while a request is parked (`acp/mod.rs:507-510`). `record.rs:1711-1713` says so
itself: production chat uses `agent_worker::run_turn` instead "because `next_event` refuses while a
permission request is parked". `PermissionPolicy::default()` is `Ask` (`driver.rs:121-125`,
`:186-189`), and the policy is evaluated only in `agent_worker`
(`crates/htui/src/agent_worker.rs:2654`), never on the engine path. So an engine-driven ACP step
whose agent asks for permission ends in a transport error today. This holds whether the engine runs
in the TUI or headless.

## 3. Constraints from requirements

Requirement text is quoted verbatim from `docs/REQUIREMENTS.md`.

| ID | Text (verbatim, abridged by ellipsis only) | Bearing |
|---|---|---|
| `R-ID-2` (`:38-39`) | "`htui` is not an IDE, not a code editor, not a terminal multiplexer, and not a cloud-hosted service. It is a local-first, developer-guided harness." | A per-box worker is the amendment `R-ORCH-12` already names. An always-on control plane needs a further amendment (§6.3) |
| `R-ID-3` (`:40-42`) | "Postgres is the single source of truth." | Postgres is already the hub. A server holds with this only if it keeps no durable state of its own (§6.3) |
| `R-ID-7` (`:51-52`) | "Any transcript or tool output is scrubbed of secrets on the host box before it is persisted or transmitted. Scrubbing fails closed." | The scrubber must run where the agent's output first lands, i.e. in the executing worker |
| `R-STO-1` (`:122-125`) | "Connection string and provider identities live in the OS keyring ... never in a file" | A headless Linux host or container has no secret-service session (§2). This is a hard blocker for a headless worker |
| `R-STO-4` (`:130-132`) | "When Postgres is unreachable ... No item creation, no runs." | The pending buffer was deleted (`docs/decisions/clean/clean-2.md:7-8`), so a worker that loses Postgres cannot keep recording a live run |
| `R-BOX-1` (`:62-63`) | "First launch on a machine registers it as a `box`" | Is a container a box? §5.3 |
| `R-BOX-2` (`:64-67`) | "... shells, container runtime, and installed agents." | The probe already owes "container runtime" |
| `R-BOX-3` (`:68-70`) | "Seeded vocabulary: ... `docker` ..." | Capability tag for routing container work |
| `R-BOX-4` (`:71-72`) | "Each box holds per-box paths for every repo and workspace root it has checked out." | Remote trees are that box's paths, never the TUI's |
| `R-AGT-1` (`:144-147`) | "start a session (prompt, working directory, environment, tool exposure) ..." | Unchanged. Environments sit under the driver |
| `R-AGT-5` (`:157-158`) | "Adding an agent requires a registry row and, at most, one stream adapter. No orchestrator or prompt code changes." | Docker must not become a per-agent code path |
| `R-AGT-6` (`:159-160`) | "probe `PATH` for known agent binaries" | The probe must run inside the target environment |
| `R-AGT-9` (`:166-173`) | "it does not read, hold, transmit or store the credential ... Authentication is a fact about a box" | Container credentials stay in the container's own volume. Remote login inherits MOD-22's loopback problem |
| `R-ORCH-8` (`:207-210`) | "`worktree` (default ...), `copy` ..., `shared_serialized` ..., `local`" | Isolation is orthogonal to environment. No new mode is needed (§5.2) |
| `R-ORCH-10` (`:214-215`) | "a run is refused when the item's required tags are not a subset of the box's tags" | Works unchanged if a container environment is a box |
| `R-ORCH-11` (`:216-217`) | "Every run records: item, target box, executing box ..." | Already in the schema |
| `R-ORCH-12` (`:218-220`) | "Remote dispatch ... Requires a headless `htui` worker per box polling Postgres, which amends R-ID-2. Version one stores the target box and executes only when it is the local box." | The requirement this item actually is |
| `R-HIS-1` (`:225-227`) | "Nothing about a run exists only on one box." | Durable log is `session_event`, written by the executing worker |
| `R-SEC-2` (`:254-256`) | "resolved at run start into the agent subprocess environment only ... `htui`'s own credentials (R-STO-1) are never exposed to a session." | `docker exec -e` and SSH `SetEnv` must be checked for exposure. The no-`env_clear` inheritance (§2) already breaks this in spirit |
| `R-MCP-1` (`:265-266`) | "`htui` exposes an MCP server to every session it launches." | A stdio `McpServerSpec` must be launchable where the agent runs |
| `R-NF-1` (`:324`) | "Windows 10+, Linux, macOS." | Same-path bind mounts do not exist on Windows hosts |
| `R-NF-2` (`:325`) | "No dependency on any external daemon other than Postgres and the agents." | `dockerd` must be opt-in per box. A control-plane server is a new daemon; Qdrant already is one without an amendment (`compose.yaml:51-53`) |
| `R-NF-3` (`:326-327`) | "the TUI never blocks on network or subprocess I/O." | Unchanged |
| Out of scope (`:332-335`) | "Any web or GUI front end." | Pushes against an HTTP API server whose natural second client is a web UI |

Architectural invariants also bind: one writer per status, as compare-and-set (ANA-2 inv. 1);
resume from the store, with the lease as the only liveness marker (`docs/ANA-2.md:139-142`); the
recovery sweep keyed on `executing_box_id = $this_box` (`docs/ANA-2.md:1285-1292`); a queued run
with a non-local target stays `queued` and is reported (`docs/ANA-2.md:603`, `:2165-2166`). ANA-14
rejected Redis in favour of Postgres `LISTEN`/`NOTIFY` and `FOR UPDATE SKIP LOCKED`
(`docs/ANA-14.md:43-53`).

## 4. Prior art

### 4.1 Docker, local

- **Docker Sandboxes (`sbx`).** One microVM per sandbox with its own kernel and dockerd. The
  workspace is either a filesystem passthrough **at the same absolute path** as on the host, or a private clone
  reached through a `sandbox-<name>` git remote. Credentials never enter the VM: env vars hold
  sentinels, and a host proxy swaps in the real value. CLI only, no programmatic API, local only.
  https://github.com/docker/docs/blob/main/content/manuals/ai/sandboxes/architecture.md,
  .../configuration/credentials.md, https://code.claude.com/docs/en/sandbox-environments
- **Anthropic's Claude Code dev container.** Workspace bind-mounted at `/workspace`,
  `CLAUDE_CONFIG_DIR=/home/node/.claude` on a named volume, `NET_ADMIN` plus an
  iptables/ipset egress allowlist that is resolved once at start.
  https://github.com/anthropics/claude-code/tree/main/.devcontainer,
  https://code.claude.com/docs/en/devcontainer
- **Hardened pattern.** `--cap-drop ALL --security-opt no-new-privileges --read-only
  --network none --user 1000:1000`, with egress only through a mounted Unix-socket proxy that
  injects credentials (`ANTHROPIC_BASE_URL=http://proxy`).
  https://code.claude.com/docs/en/agent-sdk/secure-deployment
- **Dev containers spec and CLI.** A declarative image and lifecycle (`devcontainer up`,
  `devcontainer exec`). The CLI shells out to `docker`. https://containers.dev/implementors/reference/,
  https://github.com/devcontainers/cli
- **Dagger container-use.** The agent stays on the host. Its tools run in containers through an MCP
  server, and each environment is a git branch plus a worktree **copied** into `/workdir`.
  https://github.com/dagger/container-use
- **OpenHands PR #4883.** A trusted outer server keeps the loop and credentials. An execution-only
  container exposes tools on 127.0.0.1 with a capability token, and host mounts are forbidden.
  https://github.com/OpenHands/software-agent-sdk/pull/4883

**Takeaway.** With a local daemon, a bind mount at an identical path is the norm. It is the only
choice under which htui's host-side ACP `fs/*` service and the agent's view of `cwd` are the same
files. Credentials either live in a container volume or are injected by a host proxy.

### 4.2 Docker, remote

- **Remote daemon.** `DOCKER_HOST=ssh://user@host` runs `ssh host docker system dial-stdio` and
  tunnels the Engine API. TLS on `tcp://:2376` is the alternative; plaintext `:2375` is root.
  https://github.com/docker/docs/blob/main/content/manuals/engine/security/protect-access.md
- **Inference.** Against a remote daemon, a bind-mount source is a path on the remote host. Trees
  must therefore be cloned or copied there, and `-p` publishes on the remote.
- **bollard 0.21.1.** Its `ssh` feature spawns the system `ssh` and runs `dial-stdio` (`src/ssh.rs`).
  `start_exec` returns `Attached { output, input }`, a full-duplex pair (`src/exec.rs`).
  https://github.com/fussybeaver/bollard
- **In-container server pattern.** SWE-ReX: `swerex-remote` on :8000 with `X-API-Key`, started by
  `subprocess docker run` (https://github.com/SWE-agent/SWE-ReX). OpenHands agent-server: REST
  plus WebSocket with `X-Session-API-Key` (https://github.com/OpenHands/software-agent-sdk). E2B
  `envd` over Connect RPC (https://github.com/e2b-dev/infra). Daytona toolbox daemon
  (https://www.daytona.io/docs/en/architecture/). All of these need a port or tunnel plus a token.
- **Remote-SSH plus Dev Containers in VS Code.** The container is opened **on the SSH host**, with
  no local Docker. https://code.visualstudio.com/docs/remote/ssh (source
  https://raw.githubusercontent.com/microsoft/vscode-docs/main/docs/remote/ssh.md)

**Takeaway.** Every product that runs containers remotely puts a component next to the remote
daemon, whether a server, a runner or an extension host. None drives a remote daemon from the UI
machine while keeping trees on the UI machine.

### 4.3 Remote shell

- **VS Code Remote-SSH.** Installs a server in `~/.vscode-server`, started and stopped by the
  client. Workspace extensions run on the host. `ControlMaster` is recommended, and
  `ForwardAgent` keeps git keys local. https://code.visualstudio.com/docs/remote/ssh
- **Zed remote.** Uses the system `ssh` with one ControlMaster per project. Its server runs in
  "proxy mode", which starts the daemon if it is not running and reconnects to it if it is, so work
  survives an SSH drop. The UI and LLM calls stay local.
  https://github.com/zed-industries/zed/blob/main/docs/src/remote-development.md
- **JetBrains Gateway.** A headless backend on the host and a thin client locally; the backend
  survives the client closing. https://www.jetbrains.com/help/idea/remote-development-overview.html
- **Claude Code over SSH.** The official advice is tmux or screen
  (https://code.claude.com/docs/en/remote-control). Headless `-p --output-format stream-json`,
  with resume by `--resume <id>` **on the same host**, because transcripts are stored there
  (https://code.claude.com/docs/en/headless). Credentials are in
  `~/.claude/.credentials.json`, `CLAUDE_CONFIG_DIR`, `ANTHROPIC_API_KEY` or `apiKeyHelper`
  (https://code.claude.com/docs/en/authentication).
- **ACP.** stdio is the only v1 transport, so `ssh -T host agent` is protocol-legal.
  `session/load` and `session/resume` take an absolute `cwd` on the agent's host. `fs/*` and
  `terminal/*` are served by the client. The HTTP/WebSocket RFD defers resumable streams to v2.
  https://github.com/agentclientprotocol/agent-client-protocol (`docs/protocol/v1/transports.mdx`,
  `session-setup.mdx`, `file-system.mdx`, `docs/rfds/streamable-http-websocket-transport.mdx`),
  and the stdio-only limit is recorded in `docs/ANA-4.md:99-103`.
- **tmux and mosh.** A PTY adds echo and line discipline, which is wrong for JSON-RPC or NDJSON.
  mosh cannot carry non-interactive streams. https://man7.org/linux/man-pages/man1/tmux.1.html,
  https://github.com/mobile-shell/mosh
- **Rust crates.**
  - `openssh` 0.11.6 wraps the system `ssh` through a ControlMaster, is unix only, and passes
    `~/.ssh/config` and ProxyJump through unchanged. https://github.com/openssh-rust/openssh
  - `russh` 0.63.3 is pure Rust with `AsyncRead`/`AsyncWrite` channels, but you re-implement the
    user's SSH configuration. https://github.com/Eugeny/russh

**Takeaway.** Every mature remote-dev tool runs a **long-lived process on the remote host** that
owns the child processes and the tree. SSH is the pipe used to install it and attach to it, never
the thing that owns the agent's stdio.

### 4.4 Client/server split

- **Claude Code self-hosted environments.** A control plane queues sessions. A runner in your
  network polls over outbound HTTPS only, with a lease and heartbeat and requeue after about 60 s.
  It spawns one child `claude` per session. https://code.claude.com/docs/en/self-hosted-environments
- **Claude Code Remote Control.** Outbound-only connections, `--spawn worktree`,
  `--capacity N`, and a transcript held centrally. https://code.claude.com/docs/en/remote-control
- **Coder Agents / chatd.** The closest precedent built on Postgres.
  - Every replica has a worker that claims chats with no owner or an expired lease. Heartbeats go
    to an `UNLOGGED` table every 9 s, with a 30 s lease.
  - `LISTEN`/`NOTIFY` payloads are only version hints, delivered post-commit and best effort. A
    10 s poller is the backstop.
  - Live tokens are *not* in Postgres; they go through a replica relay.
  - https://github.com/coder/coder/blob/main/coderd/x/chatd/ARCHITECTURE.md,
    https://github.com/coder/coder/blob/main/docs/ai-coder/agents/architecture.md
- **opencode.** The TUI is a client of an HTTP server that has an OpenAPI spec, an SSE `/event`
  stream and REST commands; `opencode serve` plus `opencode attach`.
  https://github.com/anomalyco/opencode/blob/dev/packages/web/src/content/docs/server.mdx
- **Codex app-server.** JSON-RPC over stdio, or over WebSocket as an experimental option with
  bearer or JWT auth. A non-loopback listener is unauthenticated by default.
  https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md,
  https://github.com/openai/codex/pull/14847
- **OpenHands session socket.** `DurableFrame{seq}` versus `TransientFrame` and `Delta`, with
  reconnect by `after_seq`; progress frames are never replayed.
  https://raw.githubusercontent.com/OpenHands/software-agent-sdk/main/openhands-agent-server/openhands/agent_server/session_protocol.py
- **Postgres `NOTIFY`.** Delivered on commit, payload under 8000 bytes, lost while a listener is
  disconnected. https://github.com/postgres/postgres/blob/master/doc/src/sgml/ref/notify.sgml
- **sqlx `PgListener`.** Reconnects silently on `recv()`; `try_recv()` returns `None` so the caller
  can resync. https://github.com/launchbadge/sqlx/blob/main/sqlx-postgres/src/listener.rs

**Common shape.** Each system has a durable ordered log, a live channel that may lose data, and a
client that reattaches by cursor. The executor owns the agent process and outlives the client.
htui already has the durable log (`session_event`, gapless `seq`: `record.rs:14-20`) and the lease
(`0003_orchestration.sql:41-43`). It lacks the executor process and the live channel.

## 5. Options compared

### 5.1 Placement options

| # | Option | Who owns the agent process | Tree location | New process / daemon | Survives TUI exit | Requirement friction | Assessment |
|---|---|---|---|---|---|---|---|
| O1 | **Launch wrapper from the TUI**: registry launch rewritten to `docker exec -i <ctr> ...` or `ssh -T host ...` | TUI process, via a local `docker` or `ssh` child | Must exist at the same path on the far side | None | No. `ssh` drop kills the session, and killing the local `docker exec`/`ssh` client does not reliably kill the far process (to verify) | Breaks `fs/*` locality (§2), local `is_file` check (`launch.rs:522`), local probe (`probe.rs:99`), local `git worktree` (`real.rs:278`), local kill (`launch.rs:816`) | **Reject** as the architecture. Acceptable only as the *mechanism* inside O2 |
| O2 | **Container environment of a box**: the box's worker starts one container per session and execs the agent inside it; trees bind-mounted at **identical absolute paths** | Worker on the same host as `dockerd` | Host scratch root, mounted | `dockerd`, opt-in per box | Only if the worker is headless (O3) | `R-NF-2` (opt-in daemon), `R-NF-1` (no same-path mounts on Windows), `R-SEC-2` (exec env exposure, to verify) | **Adopt** for Docker, local and remote |
| O3 | **Headless worker per box** (`R-ORCH-12`): a `htui` process with no TUI claims runs whose `target_box_id` is its box, drives the engine and writes `session_event`; the TUI is a Postgres client for that box's runs | Worker on the executing box | That box's `repo_box_path` and scratch root | One `htui` process per executing box (not a daemon htui depends on) | Yes | Amends `R-ID-2` (already foreseen). `R-STO-1` keyring on headless hosts. Permission relay needed (§2 gap) | **Adopt**: the core of remote execution |
| O4 | **Remote Docker daemon from the local worker** (`DOCKER_HOST=ssh://`, bollard `ssh`) | Local worker | Must be copied to the remote host | Remote `dockerd` | If the local worker is headless | Bind mounts are remote paths, so trees need tar or rsync each step; local `git worktree` is pointless; `before_hash`/`after_hash` and the merge must cross the network | **Reject**. O3 plus O2 on the remote box does the same thing with none of the copying |
| O5a | **Control-plane server** (`htui server`): workers dial out to it; it alone holds the worker-facing DSN and owns dispatch, enrolment, config/secret distribution, permission relay and event fan-out. The TUI keeps its direct DSN | Worker on the executing box (as O3) | That box's paths (as O3) | One self-hosted server, plus O3's workers | Yes | Amends `R-NF-2`, `R-ID-2`, `R-ORCH-12` ("polling Postgres"), `R-STO-1`/`R-STO-5` for workers (§6.3) | **Adopt as phase 2** (§7) |
| O5b | **Full server**: TUI and workers both talk only to it; Postgres private behind it (opencode, Codex, Coder style) | Worker on the executing box | That box's paths | As O5a, and every TUI depends on it | Yes | O5a's amendments plus `R-STO-3/4/6`, `R-TUI-1/8`, out-of-scope "web or GUI front end" (§6.3) | **Reject** |
| O6 | **In-container agent server** (SWE-ReX, OpenHands, E2B style: HTTP server in the image, htui connects by port and token) | In-container server | Container | A server per container, plus port exposure and tokens | Yes, while the container lives | New transport per agent (breaks `R-AGT-5`); ACP has no stable network transport (`docs/ANA-4.md:99-103`) | **Reject**. Reconsider if the ACP Streamable-HTTP RFD stabilises |

### 5.2 Why Docker is an environment, not an isolation mode

`R-ORCH-8` modes answer "which tree", and Docker answers "which process sandbox". A `worktree` step
can run inside a container just as a `local` step can. Adding `container` as a fifth isolation mode
would make "worktree in a container" unrepresentable. So O2 composes with the existing `Isolator`
(`isolate.rs:141`): the isolator still prepares the tree on the host, and the environment decides
where the agent process runs.

One correctness detail is forced by git. A linked worktree's `.git` is a file pointing at
`<repo>/.git/worktrees/<name>` by absolute path, and commits write objects into the main repo's
common dir. The container therefore needs **both** the scratch root and the repo's `local_path`
mounted at identical paths, and writable. Run the container as the host UID and GID, or the tree
fills with root-owned files that the host-side `git` and cleanup cannot remove.

### 5.3 Is a container a box?

Two models:

- **(a) Environment is a box setting.** The host box's `agent_box` rows describe the host, not the
  image. Probe, auth and quota for the image have no row. Nothing lets `R-ORCH-10` route work
  there.
- **(b) Environment is a child box.** A `box` row of kind `container`, with a parent box and an image
  reference, served by the parent's worker. Probe (`R-AGT-6`), auth (`R-AGT-9`: "a fact about a
  box"), install (`R-AGT-10`), capability tags (`R-ORCH-10`) and `target_box_id` all work
  unchanged. Its `box.toml` identity is minted by the parent, not by a container's ephemeral
  filesystem. Its hostname is the row's name, not the container's churning hostname, which matters
  for the prompt digest (MOD-33).

**Recommend (b).** The cost is a schema addition to `box` (a kind and a parent) and a probe that
runs inside the image. The child row also needs a hostname distinct from its parent's, because `box`
is `UNIQUE (user_id, hostname)` (`0001_init.sql:73`) and `register_box` upserts on that key
(`pg/mod.rs:358`).

### 5.4 Transport under O2 and O3

- Keep the agent transport stdio-shaped. Under O2 the worker runs `docker exec -i` (or bollard
  `start_exec` attached) and hands the stream to the existing adapters as a `ChildIo`-like pair.
  `IoSource::Prepared` (`acp/mod.rs:356`, test-support only) shows that a driver already accepts an
  injected stream; production needs an equivalent non-test variant.
- `Spawned`'s `signal` and `kill_tree` (`launch.rs:792`, `:816`) need an environment-aware
  equivalent. For a container, cancel is a signal via `docker exec kill` or `docker kill -s INT`,
  and kill-tree is stopping and removing the per-session container.
- The environment goes below `TransportBuilder` (`registry.rs:29`), as a launch decorator selected
  by the box. It is **not** a new adapter id. That keeps `R-AGT-5`: no agent row changes.
- Under O3, SSH is not in the data path at all. The worker is local to its agent, so every
  local-host assumption in §2 holds again, but on the executing box.

### 5.5 TUI ↔ worker channel under O3

- **Durable path.** `session_event` rows with cursor `seq`. The TUI reads `WHERE seq > cursor`,
  then follows.
- **Wake-ups.** `LISTEN`/`NOTIFY` with version or seq hints only, plus a poll as backstop. This is
  the Coder model and ANA-14's choice. sqlx already has the postgres feature (`Cargo.toml:42-44`),
  so it adds no new crate.
- **Commands.** Follow-up, cancel, gate approve and permission answer become rows written by the TUI
  and applied by the worker. `permission_answer` is already an event kind (`0001_init.sql:526`).
- **Live text.** The recorder flushes only on variant change, message id change, `Done`, 16 KiB or
  session end, with no idle flush (`record.rs:7-12`, `:118`). Reading Postgres alone, a remote run
  shows text in bursts. Options: accept that; or add transient delta `NOTIFY`s under 8000 bytes,
  droppable and corrected by the durable rows (OpenHands' `Delta` frames). Under O3 do not add a
  worker listener socket; under O5a the server is the relay (the Coder shape, §4.4).

## 6. The "central server" claim, evaluated

The item says remote or container execution "would require a central server with htui as just the
interface". The maintainer then asked two sharper questions: whether a central server would simply
be better, including as a **configuration manager** for workers, and what happens when several
agents write Postgres at the same time. §6.1 answers the second, §6.2 lists the configuration a
manager would own, §6.3 evaluates the server variants, §6.4 compares them and §6.5 draws the line.

### 6.1 N concurrent writers on one Postgres

**Short answer.** Concurrent writes are already handled, and a server would not make them more
correct. Agents never write Postgres; the htui process that hosts them does (recorder, R-MCP-1's
step-scoped tools: `docs/REQUIREMENTS.md:265-266`). The store was built for several writer
processes, and Postgres does the coordinating. Under MVCC "reading never blocks writing and writing
never blocks reading", and row locks "block only writers and lockers to the same row"
(https://raw.githubusercontent.com/postgres/postgres/master/doc/src/sgml/mvcc.sgml). A server would
run the same transactions against the same database; its only extra power, serialising in memory,
holds for one instance only, i.e. a single point of failure.

**What already guarantees it.**
- **Isolation.** READ COMMITTED throughout; each write path is one statement, so the loser of a
  compare-and-set race blocks on the row lock, re-evaluates and matches nothing
  (`crates/htui-store/src/pg/write.rs:4-11`). Exceptions: the chat-run pair takes an explicit
  transaction, and MOD-15's two deletes run at REPEATABLE READ (same comment).
- **Status CAS** (ANA-2 inv. 1, `docs/ANA-2.md:103-108`): `transition` updates
  `WHERE id = $1 AND status = $2` (`write.rs:595-609`); zero rows is a lost race, told apart from
  "not found" by a re-read. `update_item` is a CAS on `version` (`write.rs:501`, R-ENT-10).
- **Admission.** `claim_run` locks the run row, then the box row, `FOR UPDATE`
  (`write.rs:2468-2500`); the box lock is the critical section for the slot count and overlap check
  (`write.rs:2443-2448`, `:2527-2537`), and the final `UPDATE ... WHERE status = 'queued'`
  (`:2579-2586`) closes it. Postgres-only pin: `admission_is_serialised_by_the_box_row_lock`
  (`crates/htui-store/tests/pg_criteria.rs:586`). Two processes on one box cannot both take the last
  slot; hazard 10's former "admission still races" was wrong at the store level (§9.10). There is no
  cross-box limit, by design.
- **Leases.** `refresh_lease` is `WHERE id = $1 AND lease_owner = $2` (`write.rs:2622-2628`);
  `take_lease` succeeds only if the lease is ours, free or expired **and** `executing_box_id = $2`
  (`:2734-2751`); `adopt_runs` is one CTE over `FOR UPDATE SKIP LOCKED` with
  `lease_owner IS DISTINCT FROM $2` (`:2668-2700`), so concurrent sweeps neither deadlock nor
  double-adopt. `lease_owner` is minted per process (`docs/ANA-2.md:1276-1282`). Defaults: TTL 120 s,
  refresh 60 s (`crates/htui-orch/src/recover.rs:25-35`). The heartbeat self-fences one refresh
  before the lease lapses (`recover.rs:97-108`; used at `crates/htui-orch/src/engine.rs:1117-1152`).
- **Transcript.** One recorder per step; `seq` is gapless with one writer, so
  `PRIMARY KEY (run_step_id, seq)` is a backstop, not the allocator
  (`crates/htui-agent/src/record.rs:14-20`), applied as `ON CONFLICT (run_step_id, seq) DO NOTHING`
  in one statement per batch (`write.rs:657-685`). Different steps never collide.
- **Settings** are a CAS on `updated_at`; an insert that conflicts is `Stale`, never an overwrite
  (`write.rs:1960-1975` and the rungs below it).
- **Seeding** takes `LOCK TABLE app_user IN SHARE ROW EXCLUSIVE MODE` first
  (`crates/htui-store/src/pg/mod.rs:257-262`) and runs on every connect (`pg/mod.rs:418-424`).
- **Migrations.** sqlx `Migrator::run` takes `pg_advisory_lock` by default, so concurrent migrators
  serialise (https://raw.githubusercontent.com/launchbadge/sqlx/main/sqlx-postgres/src/migrate.rs;
  sqlx 0.9 at `Cargo.toml:42`). htui itself takes no advisory lock.
- The conformance suite already pins cross-process cases: "a_live_lease_blocks_an_answer_from_another_process"
  and "the_sweep_never_touches_a_parked_run_or_a_live_lease"
  (`crates/htui-orch/src/conformance.rs:386-408`).

**What runs today.** No crate depends on `htui-orch` (`crates/*/Cargo.toml`), so claim, lease, sweep
and heartbeat run only in the conformance and Postgres tests; production writers are chats, the
recorder, the probe, settings and CRUD. The multi-writer machinery is designed and tested, not yet
exercised by a second process.

**Connections, the real ceiling.** Each process opens one pool of up to 8 connections with a 10 s
acquire timeout (`pg/mod.rs:44`, `:137-139`). Postgres 16 `max_connections` defaults to 100 with 3
superuser slots (https://raw.githubusercontent.com/postgres/postgres/REL_16_STABLE/doc/src/sgml/config.sgml),
so about 12 processes (workers plus TUIs) with every pool full. Pools grow lazily, and a worker's
write rate is small (a flush per 16 KiB or variant change, a lease refresh per run per 60 s), so the
fix below that scale is a smaller worker pool or a higher `max_connections`, not PgBouncer.
PgBouncer's transaction mode never supports `LISTEN`, SQL-level `PREPARE` or session advisory locks,
and protocol-level prepared statements only with `max_prepared_statements` set
(https://raw.githubusercontent.com/pgbouncer/pgbouncer.github.io/master/features.md), i.e. it breaks
sqlx's migration lock and any `LISTEN`-based wake-up. A server collapses the worker side to one pool; that is its
one concrete gain on this axis.

**Spots that need work regardless of server choice.**

| # | Spot | Evidence | Fix | Server removes it? |
|---|---|---|---|---|
| C1 | **No DB-side fence for a stale lease holder.** `append_events`, `set_step_usage` and `finish_step` write `WHERE id = $1` only | `write.rs:657-685`, `:696-705`, `:3022-3050` | A process that wakes from suspend after its lease was adopted can still append rows and overwrite `exit_code`/`usage`/`finished_at`. Add a `lease_owner` (or `status`) predicate joined through `run` to step writes | No; the server would need the same predicate |
| C2 | **Lease times come from the worker's clock.** `SystemClock` is `Utc::now()` | `crates/htui-orch/src/isolate.rs:265-270`; `engine.rs:571-572` | Harmless while every lease check is same-box (`take_lease`'s `executing_box_id = $2`). Once another box answers or relays, use `clock_timestamp()` in SQL | Partly: a server can stamp times, but only if it computes them in SQL |
| C3 | **Quota write is last-writer-wins.** | `UPDATE agent_box SET quota = $3, quota_at = $4` with no ordering guard, `write.rs:832` | `AND (quota_at IS NULL OR quota_at <= $4)` | No |
| C4 | **Box identity is the hostname.** `register_box` upserts `ON CONFLICT (user_id, hostname)` | `pg/mod.rs:353-372`; `0001_init.sql:73` | A container or remote host reporting a duplicate hostname merges into another box's row: shared slots, quota and adoption sweep, which resets trees the other box cannot see. Key workers on the `box.toml` id; add a box heartbeat (`last_seen_at` is bumped only at connect) | Yes, if the server mints box ids at enrolment |
| C5 | **Schema skew.** An older binary is refused only at its next connect | `pg/mod.rs:449-452`; `htui_version` recorded but not enforced, `pg/mod.rs:360`, `:370`; migrations applied only on a TUI confirmation, `crates/htui/src/store_worker.rs:1128-1135` | Headless rule: a worker never migrates; it refuses and reports. Open pools keep running old code against a new schema until reconnect | Yes: the server holds the only schema pin and speaks a versioned protocol to workers |
| C6 | **Agent registry edits are last-writer-wins.** `upsert_agent` overwrites every column `ON CONFLICT (id)` | `write.rs:734-762`; only test callers (`crates/htui/src/agent_worker.rs:3233` and later) | `updated_at` CAS before the Settings tab edits agents from several TUIs. Seeding re-adds a deleted seed agent by design (`pg/mod.rs:226-233`) | Partly: a server can serialise, but the CAS is cheaper |
| C7 | **Box settings have no write path.** No `UPDATE box` in `write.rs`; admission reads `box.settings` under the lock (`write.rs:2496-2512`) | grep of `write.rs` | Any future writer is a CAS | No |
| C8 | **A duplicate `seq` is silently dropped.** `DO NOTHING` hides a second writer; `rows_affected` short of the batch is the only signal | `write.rs:674-684`; `record.rs:270-271` | Error when `inserted < len` outside replay, so C1 cannot lose rows silently | No |

**Conclusion.** N workers writing one Postgres is a solved problem in this schema: every shared write
is a CAS or a short row lock, and different boxes contend only on shared rows (item, key counter,
`app_setting`) and only briefly. C1-C8 are the work, and they are the same work with or without a
server; a server naturally absorbs C4 and C5 and the connection count, not the rest. **Concurrency
is therefore not an argument for a server either way.** The arguments for one are §6.3's.

### 6.2 Configuration a manager would own, and where it lives today

| Config | Lives in | Distributed today by | Notes |
|---|---|---|---|
| Agent registry: launch, models, settings, install source | `agent` (`0001_init.sql:94-106`); seeds, e.g. `crates/htui-core/seeds/agent_agy.json` | Postgres read (`R-ID-3`) | Last-writer-wins (C6) |
| Per-box enablement, version, path, probe, quota | `agent_box` (`0001_init.sql:112-124`) | Written by each box's own probe | Per-box facts; quota is C3 |
| Box profile: tags, `settings` (`max_concurrent_items`, `command_limits`), `htui_version` | `box` (`0001_init.sql:64-68`) | Postgres | No write path for settings (C7) |
| App / project / phase settings | `app_setting` (`0001_init.sql:557-561`), `project.settings` | Postgres, CAS | Safe with N writers |
| Box identity | `box.toml` (`crates/htui-store/src/identity.rs:18`) | Local file | Should be the key (C4) |
| Postgres DSN, Qdrant URL and key | OS keyring (`crates/htui-store/src/secret.rs:19-28`) | Typed per box | The `R-STO-1` headless blocker |
| Tool overrides, agents root | env `HTUI_TOOL_<NAME>`, `HTUI_AGENTS_ROOT` (`crates/htui-agent/src/tools.rs:42`; `crates/htui-agent/src/probe.rs:191-205`) | Per-box environment | Drift surface |
| Installed adapter binaries | Under `HTUI_AGENTS_ROOT` | `R-AGT-10` install, per box | Drift surface |
| Agent credentials | Wherever the agent keeps them | Never, by `R-AGT-9` (`docs/REQUIREMENTS.md:166-173`) | A manager **may not** distribute these |
| Project secrets | `SecretProvider` (`R-SEC-1`, `REQUIREMENTS.md:252-253`) | Not built (no `SecretProvider` in `crates/`) | `R-SEC-2`: into the agent's env only |
| Planned personas | `~/.config/htui/agents.d/` (MOD-26, `HANDOFF.md:141`) | Per-box files | Works against `R-ID-3`; could be registry rows instead |
| Container images and digests, child-box profiles | Not yet (MOD-41, §5.3) | - | New config either way |
| The `htui` build itself | Per box | Manual | C5 |

**Reading.** Everything durable a manager would distribute is already in Postgres, so under O3
Postgres *is* the config manager for those rows; what it lacks is change push (workers poll or
`LISTEN`), ordering guards (C3, C6, C7) and version enforcement (C5). What Postgres cannot do is
the off-database surface: enrolment, the build and images, and targeted secrets without handing each
box the whole database.

### 6.3 Server variants

**O3 - workers direct to Postgres.** Each worker holds a DSN, claims by `target_box_id` and lease,
and writes `session_event` after scrubbing (§7). Config is read from the tables of §6.2. Costs:
connection fan-out (§6.1); a full-database credential on every box, which an agent running as the
same user with htui's inherited environment (`crates/htui-agent/src/launch.rs:1114-1121`, no
`env_clear`) can plausibly reach (inference); Postgres reachable from every box, TLS optional
(`R-STO-2`, `REQUIREMENTS.md:126`); lockstep upgrades (C5); `NOTIFY` as the only push, under 8000
bytes and lost while disconnected (§4.4). Gain: no new daemon, and `htui-orch` already runs over any
`S: WriteStore` (`crates/htui-orch/src/lib.rs:3-6`).

**O5a - control-plane server, TUI still direct.** One `htui server` process next to Postgres,
reusing `htui-store` unchanged, the only holder of a **worker-facing** DSN. The TUI keeps its own DSN
and its offline cache, so `R-STO-3/4/6` and `R-TUI-8` are untouched. Responsibilities:
1. **Enrolment and identity.** A single-use token of about 1 h mints a worker keypair or client
   certificate; the server creates the box row, so ids are server-minted (fixes C4). Precedent:
   GitHub runner registration token "expires after one hour"
   (https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json)
   and a runner-held RSA key thereafter
   (https://raw.githubusercontent.com/actions/runner/main/src/Runner.Listener/Configuration/ConfigurationManager.cs);
   Boundary activation tokens
   (https://raw.githubusercontent.com/hashicorp/web-unified-docs/main/content/boundary/v0.20.x/content/docs/workers/registration.mdx);
   kubelet bootstrap token to CSR to rotating certificate.
2. **Authorisation.** Every RPC is scoped to the caller's box: claim only `target_box_id = self`,
   write events only for steps it executes, read only config targeted at it. That is least privilege
   for workers without per-worker Postgres roles; the migrations have no `GRANT`, role or RLS today.
   `R-USR-3`'s "per-user Postgres credentials" still apply to TUIs, which keep their own DSN.
3. **Config manager.** `GetManifest(box)` returns a versioned snapshot of §6.2's rows for that box:
   agents, `agent_box` enablement, `box.settings`, relevant `app_setting`, repo paths, child-box and
   image refs with digests, target `htui` version and download digest. `WatchManifest(from)` streams
   deltas derived from the `updated_at` triggers plus server-side `PgListener`; a stale cursor gets a
   full snapshot (the Kubernetes list-then-watch and `410 Gone` rule,
   https://raw.githubusercontent.com/kubernetes/website/main/content/en/docs/reference/using-api/api-concepts.md).
   The worker caches the last manifest. Precedent: Coder's agent pulls `GetManifest` with
   `environment_variables`, `scripts` and `repeated WorkspaceSecret secrets`
   (https://raw.githubusercontent.com/coder/coder/main/agent/proto/agent.proto); Buildkite returns
   tunables in the registration response
   (https://raw.githubusercontent.com/buildkite/agent/main/api/agents.go).
4. **Secrets.** The server resolves the `SecretProvider` and sends only a run's secrets inside its
   job message (Salt pillar "only accessible by the minion for which it is targeted",
   https://raw.githubusercontent.com/saltstack/salt/master/doc/topics/pillar/index.rst). The worker
   still injects them into the agent's env only and still scrubs (`R-ID-7`, `R-SEC-3`). Agent
   credentials stay on the box (`R-AGT-9`). Alternative that avoids a `R-SEC-2` amendment: each
   worker resolves secrets itself via `R-SEC-1`'s machine identity.
5. **Dispatch.** One worker-initiated WebSocket (proxy-friendly, the Jenkins JEP-222 lesson,
   https://raw.githubusercontent.com/jenkinsci/jep/master/jep/222/README.adoc) with a poll fallback.
   The server wraps the existing `claim_run`, `refresh_lease`, `adopt_runs`; the lease stays in
   Postgres as the only liveness marker (`docs/ANA-2.md:139-142`), and the heartbeat also bumps
   `box.last_seen_at`.
6. **Event ingest.** Batches idempotent on `(run_step_id, seq)`, acknowledged by highest `seq`
   (GitLab `PatchTrace`, Buildkite ordered chunks).
7. **Live relay and permission relay.** Transient deltas and `permission_request` go worker to
   server to subscribed TUIs, never persisted; the answer is written as a row and pushed to the
   worker. This is MOD-39 with push instead of polling; it still needs `pump`'s parked-request fix
   (`record.rs:1684-1713`). The TUI subscribes to the server for live views only; durable reads stay
   on its DSN.
8. **Versions.** The server holds the one schema pin (`R-STO-5`) and accepts protocol N-1 from
   workers; the manifest carries the target build (GitHub runners self-update within a week,
   https://raw.githubusercontent.com/github/docs/main/content/actions/reference/runners/self-hosted-runners.md).

Server down: workers keep executing, buffer events in memory, and lose their lease after 120 s
unless they reconnect; either keep ANA-2's reset-and-retry (the worker aborts when it cannot
refresh), or add a Nomad-style `unknown` state with `lost_after`
(https://raw.githubusercontent.com/hashicorp/web-unified-docs/main/content/nomad/v1.11.x/content/docs/job-specification/disconnect.mdx),
which amends ANA-2's "lease as the only liveness marker". Any on-disk spool re-opens CLEAN-2's
deleted pending buffer (`docs/decisions/clean/clean-2.md:5-11`) and needs a decision. TUIs are
unaffected except for live views.

Store surface. The worker needs a remote store. If that is a full `WriteStore` (66 methods,
`crates/htui-core/src/store/traits.rs:195-1058`), it is the third implementation MOD-25 removed
(`HANDOFF.md:155-158`, `:165-166`). It must instead be the narrow verb set the worker actually calls (claim,
lease, adopt, step status, events, usage, finish), which is a constraint on MOD-38 now (§8).

**O5b - full server, Postgres private.** As O5a, and the TUI too talks only to the server. Adds:
Postgres never leaves a private network; one pool total. Costs: an RPC mirror of `ReadStore` (16),
`WriteStore` (66) and the TUI's 58-variant `StoreRequest` (`crates/htui/src/store_worker.rs:79`);
the cache refresher reads through the API; offline mode fires on either of two hops in series;
`R-STO-6`'s sub-second start is re-measured; the connection settings of `R-TUI-8` become an endpoint
and token; chats, which run as tasks inside the TUI (`store_worker.rs:1355-1357`), record through the
server. An HTTP API with a second client in reach pushes against "Any web or GUI front end" being
out of scope (`REQUIREMENTS.md:333`). This is the opencode/Codex shape, and it is the version of the
item's "htui as just the interface" claim that the requirements resist most.

### 6.4 Comparison

| Axis | O3 workers → Postgres | O5a control plane, TUI direct | O5b full server |
|---|---|---|---|
| Concurrency correctness | Postgres CAS/locks (§6.1); C1-C8 to fix | Same transactions; same C1-C3, C6-C8; C4, C5 absorbed | Same as O5a |
| Connections | ~8 per process, ~12 processes at PG16 defaults | 1 server pool + TUIs | 1 pool |
| Security, DB exposure | Full-DB DSN on every box; 5432 reachable from every box | Box-scoped revocable key on workers; 5432 reachable from TUIs and server only | Postgres private |
| Headless credential (`R-STO-1`) | DSN on a headless host: hard blocker (§9.1) | Moved, not solved: a worker key in a file, but box-scoped and revocable | As O5a, plus TUI tokens |
| Permission relay | Rows plus poll or `NOTIFY` (MOD-39) | Push through server; same `pump` fix | Same |
| Live streaming | Flush bursts; `NOTIFY` deltas < 8000 B, lossy (MOD-43) | Native relay | Native relay |
| Worker liveness | Lease per run; box heartbeat to add | Connection plus lease | Same |
| Config drift, version skew | DB config shared; off-DB surface unmanaged; schema lockstep (C5) | Manifest + watch, targeted secrets, target build, N-1 protocol | Same, TUIs included |
| Ops cost, SPOF | Postgres only | Adds one daemon; worker dispatch stops while it is down, TUIs continue | Adds a daemon in series with Postgres for everything |
| Requirement amendments | `R-ID-2` per `R-ORCH-12`, `R-ORCH-12` to must, `R-STO-1`, `R-STO-5` (worker never migrates), `R-NF-2` (dockerd) | O3's plus `R-NF-2` (server), `R-ID-2` (self-hosted control plane), `R-ORCH-12` wording, `R-STO-1`/`R-STO-5` for workers, `R-SEC-2` if server resolves secrets | O5a's plus `R-STO-3/4/6`, `R-TUI-1/8`, out-of-scope web front end |
| Build effort | Worker entry point, relay rows, C1-C8 | O3's worker and fixes, plus server, protocol, enrolment, CA, manifest | O5a plus the full RPC mirror and a cache refresher over it |
| Offline behaviour | Worker without Postgres cannot record; lease lapses, reset-and-retry | Worker without server: same, unless a spool or `unknown` state is added | TUI offline when either hop fails |

### 6.5 What the evidence says

1. **Concurrency does not decide it.** §6.1: correctness is Postgres's, and a server would reuse it.
2. **The worker is needed in every variant.** The executor must be next to the tree and agent
   (§4.3, §4.4); every server design is O3's worker plus something. So O3's worker is the first
   step whichever way phase 2 goes, and nothing built for O3 is thrown away by O5a.
3. **O5a's gains are real and are all about workers on boxes the user does not fully trust or
   reach**: a revocable box-scoped credential instead of the database, no inbound 5432, N-1 version
   skew, a real live channel, and pushing the off-database config of §6.2. For one developer whose
   boxes share a trusted network (`R-USR-1`) and whose containers are child boxes driven by the
   host's worker (so a container never holds the DSN, §5.3 and MOD-41 in §8), those gains are small next to a new
   daemon, a protocol and an enrolment CA.
4. **O5b buys little over O5a** (Postgres fully private, one pool) at the price of the RPC mirror,
   a second offline hop and the most requirement friction. Rejected.
5. **The old rejection of any server was too strong.** It rested on `R-ID-2`, `R-NF-2` and `R-ID-3`
   alone. A self-hosted server holding no durable state keeps `R-ID-3` (it is a gateway, like
   Temporal's Frontend, not a second store), ANA-14 rejected Redis rather than any hub
   (`docs/ANA-14.md:43-53`), and `R-NF-2` is already bent by Qdrant. What remains is a real but
   amendable cost, not a contradiction.

## 7. Verdict

**Phased: O3 + O2 now, O5a as phase 2, O5b rejected.** Remote execution is `R-ORCH-12`: a headless
`htui` worker on each executing box, claiming runs by `target_box_id` and lease, driving the existing
engine and writing `session_event` after scrubbing on that box. In phase 1 the worker talks directly
to Postgres (O3), which already coordinates N concurrent writers through CAS, row locks and
`SKIP LOCKED` (§6.1); the multi-writer gaps C1-C8 are fixed first because they bind any design. The
TUI stays a Postgres client throughout. Phase 2 adds a self-hosted **control-plane server** (O5a)
that workers dial out to: it enrols boxes and mints their ids, holds the only worker-facing DSN, owns
dispatch, targeted secrets, a versioned config manifest, the target build, the live and permission
relay, and the schema pin. It holds no durable state, so `R-ID-3` stands. Phase 2 is opened when the
first of these holds: a worker on a box outside the user's trusted network or behind NAT, team use
(`R-USR-3`), more worker processes than the Postgres connection budget allows, or MOD-43's `NOTIFY`
deltas proving inadequate. Phase 1 is built so phase 2 is additive: the worker talks to a narrow
store surface, never a full `WriteStore` mirror. A full server in front of the TUI (O5b) is rejected:
it adds a second offline hop, an RPC mirror of the whole store and the most requirement friction for
little over O5a.

Docker, local or remote, is an execution environment modelled as a **child box** of the host that
runs `dockerd`. It is driven by that host's worker, one container per session, with trees
bind-mounted at identical absolute paths and the agent's stdio carried over `docker exec -i`, so the
ACP and CLI adapters are unchanged. "Docker remote" is O3 (or O5a) on the remote host plus O2 there,
never a remote daemon driven from the TUI's machine (O4 rejected).

"Remote shell" is the provisioning path for a remote box's worker, not an agent transport. A raw
`ssh -T` or `docker exec` wrapper around the TUI's own launch (O1) is rejected because it breaks
host-side `fs/*`, probe, git isolation and kill.

Four things block the first remote run and have to be settled first:
- the DSN on a headless host (`R-STO-1` against a keyring that is `sync-secret-service` only);
- a permission relay (the engine's `pump` cannot answer a parked ACP request);
- keeping MOD-4 M6's `run_worker` out of TUI-only code, behind a narrow store surface;
- the store-level multi-writer fixes C1, C4, C5 and C8 (§6.1).

Requirement amendments to put to the maintainer:
- Phase 1: `R-ID-2` (per `R-ORCH-12`); `R-ORCH-12` from later to must; `R-NF-2` (`dockerd` as an
  opt-in, per-box dependency); `R-STO-1` (headless credential store for the worker's DSN); `R-STO-5`
  (a headless worker never migrates; it refuses and reports).
- Phase 2: `R-NF-2` (an optional self-hosted `htui server`; Qdrant's existing status should be
  recorded in the same amendment); `R-ID-2` (a self-hosted control plane is not a cloud-hosted
  service); `R-ORCH-12` ("polling Postgres" becomes "polling Postgres or the control plane");
  `R-STO-1` (a worker holds a box-scoped, revocable key in a file, not a DSN); `R-STO-5` (the server
  owns migrations for workers); `R-SEC-2` only if the server, rather than the worker, resolves
  project secrets. `R-USR-3` needs no amendment: its per-user Postgres credentials stay with TUIs,
  and worker authorisation lives in the server.
- Unchanged in every phase: `R-AGT-9` (the config manager never distributes agent credentials),
  `R-ID-7` (scrubbing stays on the executing box), `R-ID-3`.

## 8. Phasing

IDs minted 2026-09-24 when the items were opened (placeholders G, A-F, H, I became MOD-37..45 in that order). Order is dependency order.

**Phase 1 - no server (O3 + O2).**

1. **MOD-37 - Multi-writer store hardening.** `R-ID-3`, `R-HIS-1`, ANA-2 inv. 1.
   - C1: fence step writes (`append_events`, `set_step_usage`, `finish_step`) on the run's
     `lease_owner`. C8: error on a short insert outside replay.
   - C2: lease times from `clock_timestamp()` in SQL. C3: ordered quota write. C6: `updated_at` CAS
     on `upsert_agent`. C7: any box-settings writer is a CAS.
   - C4: box keyed on the `box.toml` id, not the hostname; a box heartbeat bumping `last_seen_at`.
   - C5: a headless connect never migrates and reports "schema is newer" as a worker state;
     `htui_version` compared against a target in `app_setting`.
   - Depends on nothing; MOD-38 depends on it.
2. **MOD-38 - Headless worker (`htui worker`).** `R-ORCH-12`, `R-ID-2` (amended), `R-STO-1`,
   `R-NF-2`, `R-NF-3`.
   - A ratatui-free worker entry point hosting MOD-4 M6's run supervision: lease refresh, sweep, one
     engine task per claimed run, claims only `target_box_id = self`.
   - The worker reaches the store only through a narrow worker-store trait (claim, lease, adopt,
     step status, events, usage, finish), implemented by `PgStore` now and by an O5a client later.
   - A headless DSN source compliant with an amended `R-STO-1` (keyring `linux-native` or a systemd
     credential). Maintainer decision.
   - A per-worker pool size setting (default below 8) for the connection budget (§6.1).
   - Includes the ask to MOD-4 M6: `run_worker` in a library both binaries link.
   - Depends on MOD-37, MOD-4 (M6) and MOD-7.
3. **MOD-39 - Permission and control relay through Postgres.** `R-AGT-1`, `R-HIS-1`, `R-TUI-6`.
   - Replaces `pump`'s silent failure on a parked ACP request: the worker records
     `permission_request`, waits on a `permission_answer` row, then answers the session. Cancel and
     follow-up become command rows. Also fixes the engine's in-process path.
   - Answers from another box go through the relay, never `take_lease` (C2).
   - Depends on MOD-4 (M6). MOD-38 consumes it. Its row protocol is the payload O5a later pushes.
4. **MOD-40 - Remote dispatch in the TUI.** `R-ORCH-11`, `R-ORCH-12`, `R-TUI-1`, `R-NF-3`.
   - Target box on run start and in auto mode; a non-local target stays `queued` until its worker
     claims it (`docs/ANA-2.md:2165`); the Runs view follows `session_event` by `seq` with
     `LISTEN`/`NOTIFY` hints and a poll backstop; worker liveness from the box heartbeat.
   - Depends on MOD-38 and MOD-39, and on MOD-12 for auto mode.
5. **MOD-41 - Container execution environment.** `R-BOX-1..3`, `R-AGT-5`, `R-AGT-6`, `R-AGT-9`,
   `R-SEC-2`, `R-MCP-1`, `R-NF-1`, `R-NF-2` (amended).
   - A child `box` of kind `container` (parent, image and digest, mounts, limits, egress policy),
     with its own id and a hostname distinct from its parent's (C4).
   - Probe, install and auth inside the image; per-session container as host UID/GID; scratch root
     and repo `local_path` bind-mounted at identical paths; launch decorator under
     `TransportBuilder`; cancel as a signal inside the container, kill-tree as container removal;
     secrets via exec env after checking `docker inspect`; agent credentials on a named volume htui
     never reads. The container never holds the DSN.
   - Linux and macOS first; Windows deferred to MOD-16. Depends on MOD-7; needs MOD-38 to survive TUI
     exit.
6. **MOD-42 - Remote box provisioning over SSH.** `R-BOX-1`, `R-BOX-4`, `R-AGT-9`, `R-STO-1`.
   - System `ssh` to a host; install the matching `htui` build and a user service running
     `htui worker`; set the credential through the worker's stdin (never argv or a file); the worker
     self-registers. Agent login uses MOD-22's paste-back. SSH is not used after provisioning.
   - Under phase 2 the credential it installs is an enrolment token, not a DSN.
   - Depends on MOD-38 and MOD-22.
7. **MOD-43 - Live streaming via `NOTIFY` (optional).** `R-HIS-1`, `R-NF-3`.
   - Transient `NOTIFY` deltas under 8000 bytes between recorder flushes, droppable, superseded by
     durable rows. Open only if flush bursts prove unusable; if phase 2 is already open, the relay
     replaces it.
   - Depends on MOD-40.

**Phase 2 - control plane (O5a), opened on a §7 trigger.**

8. **MOD-44 - `htui server` control plane.** `R-NF-2`, `R-ID-2`, `R-ORCH-12`, `R-STO-1`, `R-STO-5`
   (all as amended), `R-USR-3`, `R-SEC-1..4`, `R-ID-7`.
   - A subcommand or binary reusing `htui-store`, the only worker-facing DSN holder; stateless
     except in-memory subscriptions and an enrolment CA key.
   - Enrolment (single-use token, worker keypair, server-minted box id), box-scoped authorisation,
     a versioned worker protocol accepting N-1.
   - Worker-initiated WebSocket: dispatch wrapping `claim_run`/`refresh_lease`/`adopt_runs`, event
     ingest idempotent on `(run_step_id, seq)`, live and permission relay to TUI subscribers.
   - A decision on server-down behaviour: reset-and-retry kept, or an `unknown`/`lost_after` state
     (amends ANA-2).
   - Depends on MOD-38 (narrow store surface), MOD-39, MOD-37.
9. **MOD-45 - Config manager and secret distribution.** `R-ID-3`, `R-AGT-9`, `R-AGT-10`, `R-SEC-1`,
   `R-SEC-2`.
   - `GetManifest`/`WatchManifest` over §6.2's rows plus images, target build and download digest;
     full resync on a stale cursor; worker-side cache of the last manifest.
   - Targeted per-run secrets, or worker-side resolution if `R-SEC-2` is not amended. Never agent
     credentials.
   - Worker self-update to the manifest's target build.
   - Depends on MOD-44 and MOD-10 (secret provider).

Existing items affected (for the maintainer, not edited here):
- MOD-4 M6: library placement of `run_worker` and the narrow worker-store surface.
- MOD-7: box registration keyed on the box id (C4); under phase 2, registration by enrolment.
- MOD-10: where secrets resolve (worker or server).
- MOD-11: an MCP server reachable inside a container or on a remote box; a stdio `McpServerSpec`
  must be launchable there.
- MOD-12: target box selection.
- MOD-15/MOD-23: the connection section; unchanged for the TUI under O5a.
- MOD-24: presupposes a daemon and re-hydration, which conflicts with ANA-2 §4.9's reset-and-retry
  resume; O3 gives it the daemon but not the re-attach.
- MOD-26: personas as registry rows rather than a per-box directory, so they are distributed like
  the rest of §6.2.
- MOD-33: child-box hostnames.

## 9. Risks and open questions

1. **Headless DSN (`R-STO-1`).** The Linux keyring backend is `sync-secret-service` only
   (`Cargo.toml:45-46`), and a headless server or container has no secret-service daemon. Options:
   keyring `linux-native` (kernel keyutils, lost on reboot) or a systemd credential (a file, which
   `R-STO-1` forbids). Needs a maintainer amendment before MOD-38. Phase 2 does not remove the
   problem; it shrinks what is stored to a revocable box-scoped key.
2. **Permission gap is present today.** `pump` (`record.rs:1684-1703`) plus ACP parking
   (`acp/mod.rs:507-510`) plus no policy evaluation on the engine path means engine-driven ACP steps
   fail on the first permission request. MOD-4 M6 hits this before any remote work does.
3. **Environment inheritance.** `spawn_supervised` has no `env_clear` (`launch.rs:1114-1121`).
   Locally the child sees htui's whole environment, at odds with `R-SEC-2`'s "never exposed to a
   session", and under O3 a same-user agent may be able to reach the worker's DSN (inference). Under
   O2 `docker exec` passes nothing implicitly, but explicit `-e` values may be visible through
   `docker inspect` and `/proc`. Verify before MOD-41.
4. **Kill semantics across a wrapper.** Killing a local `docker exec` or `ssh -T` client is not
   known to terminate the far process (unverified). For O2, per-session containers make removal the
   kill-tree. `docs/ANA-4.md:1365-1366` requires that no agent process is left behind.
5. **Worktree mounts.** Mounting only the tree breaks `git` inside the container (§5.2). Mounting the
   repo's `.git` writable gives the agent the same power over the main repo as local `worktree` mode.
   Container mode is not a security boundary for the repo.
6. **Windows (`R-NF-1`).** Same-path bind mounts from `C:\...` into a Linux container do not exist.
   Deferred. Under phase 2 the server also needs a Windows story or is Linux-only.
7. **Offline executor.** With the pending buffer deleted (`docs/decisions/clean/clean-2.md:5-11`), a
   worker that loses Postgres (or, in phase 2, the server) mid-run cannot record. Its lease expires
   and the sweep resets the tree (`docs/ANA-2.md:1276-1299`). Acceptable; a spool would re-open
   CLEAN-2. `docs/ANA-2.md:1325-1332` still describes the removed buffer and is stale.
8. **Credential placement.** Per-box login for a container means a named volume per child box.
   Remote login inherits MOD-22's loopback problem (`HANDOFF.md:490-519`). API-key agents rely on
   MOD-10 injection, not built.
9. **Quota rows.** `agent_box` quota is keyed `(agent_id, box_id)`; a host and its container child
   sharing one subscription split into two rows. Probably acceptable; confirm with MOD-36/ANA-21. The
   write itself is last-writer-wins until C3.
10. **Two executors on one box.** Per-process `lease_owner` keeps an in-process `run_worker` and a
    headless worker from adopting each other's runs (`docs/ANA-2.md:1277-1279`), and admission is
    serialised by the box-row lock (`write.rs:2443-2448`; `pg_criteria.rs:586`), so the per-box
    limit holds. The remaining hazard is C1 (a suspended process writing after adoption). Still
    decide whether a headless worker on a box disables in-process execution there, for clarity
    rather than safety.
11. **Stale lease holder (C1) and clock skew (C2).** Until MOD-37, a process resumed from suspend can
    overwrite an adopted step; once a relay answers across boxes, worker clocks matter.
12. **Box identity by hostname (C4).** A container or cloned VM with a duplicate hostname silently
    merges into another box's row, sharing its sweep. Whether Docker host networking passes the host
    hostname through is unverified. Fix before MOD-41 and MOD-42.
13. **Postgres exposure and connection budget.** Under O3 every worker needs a reachable DSN, TLS
    optional (`R-STO-2`), and about 8 connections (§6.1). Acceptable for `R-USR-1` on a trusted
    network; team use (`R-USR-3`), untrusted hosts or more worker processes than the connection
    budget allows (about 12 processes at defaults, §6.1) are phase 2 triggers (§7).
14. **Version skew (C5).** Until MOD-37, one box migrating locks out every older worker at its next
    connect, while open pools keep running old code. Phase 2 moves the pin to the server.
15. **Phase 2 costs.** A new daemon on the dispatch path (a second SPOF beside Postgres for workers),
    an enrolment CA to protect, a protocol to version, and the risk of the worker-store surface
    creeping toward a full `WriteStore` mirror (MOD-25's third store). The narrow surface in MOD-38 is
    the guard.
16. **Server-down semantics.** Reset-and-retry after 120 s versus an `unknown`/`lost_after` state
    that amends ANA-2's single liveness marker. Decide in MOD-44.
17. **ACP network transport.** If the Streamable-HTTP/WebSocket RFD stabilises, O6 becomes cheaper.
    It still would not remove the need for an executor that owns the tree.

## 10. Amendment record

| Date | Change |
|---|---|
| 2026-09-24 | Original conclusion: no central server; a headless worker per box talking only to Postgres (O3 + O2). O5 rejected on `R-ID-2`, `R-NF-2` and `R-ID-3` alone |
| 2026-09-24 | After the maintainer's challenge ("wouldn't a central server be better? what if multiple agents write at the same time on postgres?"): added §6.1 showing concurrent writers are already coordinated by CAS, row locks and `SKIP LOCKED`, and listing eight multi-writer gaps (C1-C8) that bind any design; corrected hazard 10 (admission is serialised by the box-row lock); added §6.2's config inventory; split O5 into O5a (control plane, TUI direct) and O5b (full server) and compared them with O3 on twelve axes. Verdict revised from "no server" to **phased**: O3 + O2 first, with MOD-37 (store hardening) and a narrow worker-store surface; O5a as phase 2 (MOD-44 server, MOD-45 config manager) on stated triggers; O5b rejected. Phase 2 requirement amendments added |
