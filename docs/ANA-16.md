# ANA-16 - Agent execution environments (Docker, remote shell)

> **Scope note:** "Research how to implement ways to run an agent in a Docker container (local and
> remote) and in a remote shell. Note this would require a central server with htui as just the
> interface." (`HANDOFF.md:62`)
>
> **Requirements addressed:** `R-ID-2`, `R-ID-3`, `R-ID-7`, `R-STO-1`, `R-STO-4`, `R-BOX-1..4`, `R-AGT-1`,
> `R-AGT-5`, `R-AGT-6`, `R-AGT-9`, `R-ORCH-8`, `R-ORCH-10..12`, `R-HIS-1`, `R-SEC-2`, `R-MCP-1`,
> `R-NF-1..3`.
>
> **Status (2026-09-24): concluded.** Verdict: feasible; the premise is half right. Running away from
> the TUI needs a process that outlives it, but that process is a **headless `htui` worker per
> executing box** talking only to Postgres (`R-ORCH-12`), not a new central server. Docker is an
> execution environment *of* a box, driven by that box's worker; a remote shell is how a remote box's
> worker is provisioned, not the agent transport.

Code citations are against HEAD `ba68682`.

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
| `R-ID-2` (`:38-39`) | "`htui` is not an IDE, not a code editor, not a terminal multiplexer, and not a cloud-hosted service. It is a local-first, developer-guided harness." | A central server contradicts this. A per-box worker is the amendment `R-ORCH-12` already names |
| `R-ID-3` (`:40-42`) | "Postgres is the single source of truth." | Postgres is already the hub. A second server would be a second place state lives |
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
| `R-NF-2` (`:325`) | "No dependency on any external daemon other than Postgres and the agents." | `dockerd` must be opt-in per box. A central htui server is a new daemon |
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
| O5 | **Central htui server** with its own API (REST/SSE or JSON-RPC/WebSocket), TUI as thin client, in the style of opencode or Codex | Server | Server's disk, or runners | A new always-on server, auth, TLS, schema for API clients | Yes | Contradicts `R-ID-2`, adds a daemon htui depends on (`R-NF-2`), duplicates `R-ID-3`'s hub, invites a web front end (out of scope). Still needs per-box runners for multi-machine work | **Reject** |
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
  droppable and corrected by the durable rows (OpenHands' `Delta` frames). **Do not** add a worker
  listener socket (the Coder relay), which reintroduces a server.

## 6. The "central server" claim, evaluated

The item says remote or container execution "would require a central server with htui as just the
interface". Against the actual architecture:

1. **The centre already exists, and it is Postgres.** `R-ID-3` puts runs, transcripts, box
   profiles, the agent registry and settings there. Leases, claims, `target_box_id` and
   `executing_box_id` are already in the schema (§2). Every process that executes must write
   Postgres directly (MOD-25: no writable local life; CLEAN-2: no pending buffer). A central htui
   server would sit *in front of* the single source of truth and duplicate it. ANA-14 already
   rejected a second hub for the same reason (`docs/ANA-14.md:43-53`).
2. **What is actually required is an executor that outlives the TUI.** The prior art (§4.3, §4.4)
   uniformly puts a long-lived process next to the agent. That is `R-ORCH-12`'s "headless `htui`
   worker per box polling Postgres", one per **executing box**, dialing out only to Postgres (the
   Coder workspace-daemon shape). It is not central. Two workers on two boxes never talk to each
   other.
3. **"htui as just the interface" is true per run, not globally.** For a run whose executing box is
   not the TUI's box, the TUI only reads and writes rows. On the TUI's own box, the TUI either keeps
   executing in-process (MOD-4 M6's `run_worker`) or defers to a local headless worker. Both are
   valid. The requirement is "the executor is wherever the worker is", not "the TUI never
   executes".
4. **Local Docker needs no remote process at all.** O2 on the TUI's own box is the in-process
   `run_worker` plus a container launch. Only *surviving TUI exit* needs O3, and that is true
   without Docker too.
5. **Remote shell does not need a server either.** It needs the remote box to run its own worker.
   SSH is how that worker is installed and started (the VS Code and Zed pattern), not a transport
   htui keeps open.

**So the claim is refuted as stated and confirmed in a narrower form.** No central server is
needed. A second `htui` process kind, the headless worker, is needed. It amends `R-ID-2` exactly as
`R-ORCH-12` already says, and it makes `R-ORCH-12` a prerequisite rather than a later-tier nicety.

## 7. Verdict

**Adopt O3 + O2.** Remote execution is `R-ORCH-12`: a headless `htui` worker on each executing
box, claiming runs by `target_box_id` and lease from Postgres, driving the existing engine and
writing `session_event` after scrubbing on that box. The TUI becomes a Postgres client for runs it
does not execute.

Docker, local or remote, is an execution environment modelled as a **child box** of the host that
runs `dockerd`. It is driven by that host's worker, one container per session, with trees
bind-mounted at identical absolute paths and the agent's stdio carried over `docker exec -i`, so the
ACP and CLI adapters are unchanged. "Docker remote" is therefore O3 on the remote host plus O2 there,
never a remote daemon driven from the TUI's machine (O4 rejected).

"Remote shell" is the provisioning path for a remote box's worker, not an agent transport. A raw
`ssh -T` or `docker exec` wrapper around the TUI's own launch (O1) is rejected because it breaks
host-side `fs/*`, probe, git isolation and kill. A central htui server (O5) is rejected against
`R-ID-2`, `R-NF-2` and `R-ID-3`.

Three things block the first remote run and have to be settled first:
- the DSN on a headless host (`R-STO-1` against a keyring that is `sync-secret-service` only);
- a permission relay (the engine's `pump` cannot answer a parked ACP request);
- keeping MOD-4 M6's `run_worker` out of TUI-only code, so the headless binary can reuse it.

Requirement amendments to put to the maintainer: `R-ID-2` (per `R-ORCH-12`), `R-ORCH-12` from
later to must, `R-NF-2` (`dockerd` as an opt-in, per-box dependency), and `R-STO-1` (headless
credential store).

## 8. Phasing

Placeholder IDs. Real `MOD-N` IDs are minted when the items are opened. Order is dependency order.

1. **MOD-A - Headless worker (`htui worker`).** `R-ORCH-12`, `R-ID-2` (amended), `R-STO-1`,
   `R-NF-2`, `R-NF-3`.
   - A ratatui-free worker entry point: a subcommand, or a crate without `ratatui`/`crossterm`.
   - It hosts MOD-4 M6's run supervision: lease refresh, sweep, one engine task per claimed run,
     claims only `target_box_id = self`.
   - It adds a headless DSN source compliant with an amended `R-STO-1`: keyring `linux-native`
     (kernel keyutils), or a systemd credential. This needs a maintainer decision.
   - It adds a liveness signal per box: a heartbeat, or a periodic refresh of the existing
     `box.last_seen_at` (`0001_init.sql:70`), which today is bumped only by `register_box`.
   - Includes the ask to MOD-4 M6: put `run_worker` in a library both binaries link, not in TUI-only
     code.
   - Depends on MOD-4 (M6) and MOD-7 (box registration and `repo_box_path`).
2. **MOD-B - Permission and control relay through Postgres.** `R-AGT-1`, `R-HIS-1`, `R-TUI-6`.
   - Replaces `pump`'s silent failure on a parked ACP request: the worker records
     `permission_request`, waits on a `permission_answer` row written by the TUI, then answers the
     session.
   - Cancel and follow-up become command rows the same way.
   - Also fixes the engine's in-process path.
   - Depends on MOD-4 (M6). MOD-A consumes it.
3. **MOD-C - Remote dispatch in the TUI.** `R-ORCH-11`, `R-ORCH-12`, `R-TUI-1`, `R-NF-3`.
   - Choose the target box on run start and in auto mode.
   - A non-local target stays `queued` until that box's worker claims it (`docs/ANA-2.md:2165`).
   - The Runs view follows `session_event` by `seq` cursor with `LISTEN`/`NOTIFY` hints and a poll
     backstop, and shows worker liveness.
   - Depends on MOD-A and MOD-B, and on MOD-12 for auto mode targeting.
4. **MOD-D - Container execution environment.** `R-BOX-1..3`, `R-AGT-5`, `R-AGT-6`, `R-AGT-9`,
   `R-SEC-2`, `R-MCP-1`, `R-NF-1`, `R-NF-2` (amended).
   - A child `box` of kind `container`: parent box id, image, mounts, resource limits, optional
     egress policy.
   - The probe, install and auth run inside the image.
   - Per-session container, running as host UID/GID, with the scratch root and repo `local_path`
     bind-mounted at identical paths.
   - A launch decorator under `TransportBuilder` (`docker exec -i`, or bollard attach) that yields a
     `ChildIo`-like pair.
   - Cancel is a signal inside the container; kill-tree is container removal.
   - Secrets go in through exec env, after verifying they do not surface in `docker inspect`.
   - The agent's credentials live on a named volume that htui never reads.
   - Linux and macOS hosts first; Windows is deferred to MOD-16.
   - Depends on MOD-7. Works in-process without MOD-A; needs MOD-A to survive TUI exit.
5. **MOD-E - Remote box provisioning over SSH.** `R-BOX-1`, `R-BOX-4`, `R-AGT-9`, `R-STO-1`.
   - From the TUI: `ssh` (system binary, honouring the user's `~/.ssh/config`) to a host. Upload or
     download the matching `htui` build, install a user service running `htui worker`, set the DSN
     through the worker's own stdin (never argv and never a file), and let it self-register as a
     box.
   - Agent login on that box uses MOD-22's paste-back.
   - SSH is not used after provisioning.
   - Depends on MOD-A and MOD-22.
6. **MOD-F - Live streaming for runs on other boxes (optional).** `R-HIS-1`, `R-NF-3`.
   - Transient `NOTIFY` deltas under 8000 bytes for assistant text between recorder flushes. They
     may be dropped and are superseded by the durable `session_event` rows.
   - No worker listener socket.
   - Depends on MOD-C. Open it only if flush-granularity bursts prove unusable.

Existing items affected (for the maintainer, not edited here):
- MOD-4 M6: library placement of `run_worker`.
- MOD-11: an MCP server that the agent can reach inside a container or on a remote box; a stdio
  `McpServerSpec` must be launchable there.
- MOD-12: target box selection.
- MOD-24: presupposes a daemon and re-hydration, which conflicts with ANA-2 §4.9's
  reset-and-retry resume; O3 gives it the daemon but not the re-attach.
- MOD-33: child-box hostnames.

## 9. Risks and open questions

1. **Headless DSN (`R-STO-1`).** The Linux keyring backend is `sync-secret-service` only
   (`Cargo.toml:45-46`), and a headless server or container has no secret-service daemon. The
   options are enabling keyring's `linux-native` backend (kernel keyutils, which does not survive a
   reboot) or a systemd credential (a file, which `R-STO-1` forbids). This needs a maintainer
   amendment before MOD-A.
2. **Permission gap is present today.** `pump` (`record.rs:1684-1703`) plus ACP parking
   (`acp/mod.rs:507-510`) plus no policy evaluation on the engine path means engine-driven ACP steps fail on the first
   permission request. MOD-4 M6 hits this before any remote work does.
3. **Environment inheritance.** `spawn_supervised` has no `env_clear` (`launch.rs:1114-1121`).
   Locally the child sees htui's whole environment, which is at odds with `R-SEC-2`'s "never
   exposed to a session". Under O2 `docker exec` passes nothing implicitly, which is better, but
   explicit `-e` values may be visible through `docker inspect` and `/proc` inside the container.
   Verify before MOD-D.
4. **Kill semantics across a wrapper.** Killing a local `docker exec` or `ssh -T` client is not
   known to terminate the far process (unverified). This is the main reason O1 is rejected. For O2,
   use per-session containers so removal is the kill-tree. `docs/ANA-4.md:1365-1366` requires that
   no agent process is left behind.
5. **Worktree mounts.** Mounting only the tree breaks `git` inside the container (§5.2). Mounting the
   repo's `.git` writable gives the agent the same power over the main repo as today's local
   `worktree` mode, not more. Container mode is not a security boundary for the repo.
6. **Windows (`R-NF-1`).** Same-path bind mounts from `C:\...` into a Linux container do not exist.
   Container environments on Windows need WSL2 paths or copy mode. Deferred.
7. **Offline executor.** With the pending buffer deleted (`docs/decisions/clean/clean-2.md:7-8`), a
   remote worker that loses Postgres mid-run cannot record. Its lease expires and the sweep resets
   the tree (`docs/ANA-2.md:1276-1299`). This is acceptable, but `docs/ANA-2.md:1325-1332` still
   describes the removed buffer and is stale.
8. **Credential placement.** Per-box login for a container means a named volume per child box.
   Login from a remote box inherits the loopback OAuth problem (MOD-22; `ssh -L` did not work,
   paste-back did: `HANDOFF.md:490-519`). API-key agents (`GEMINI_API_KEY`) rely on MOD-10
   injection, which is not built.
9. **Quota rows.** `agent_box` quota is keyed `(agent_id, box_id)`. A host and its container child
   sharing one subscription split into two rows, both passive and both correct at reporting time.
   Probably acceptable. Confirm with MOD-36/ANA-21's weighting.
10. **Two executors on one box.** If the TUI's in-process `run_worker` and a headless worker run on
    the same box, the per-process `lease_owner` keeps them from adopting each other's runs
    (`docs/ANA-2.md:1277-1279`). Admission still races for the per-box concurrency limit. Decide
    whether a headless worker on a box disables in-process execution there.
11. **Live-view latency.** Recorder flush granularity (16 KiB, no idle flush) may make remote runs
    feel frozen. MOD-F covers this if it matters.
12. **Postgres exposure.** Every worker needs a DSN reachable from its host, and over a network
    that should mean TLS, which `R-STO-2` supports but leaves optional. For a single developer (`R-USR-1`) that is acceptable. Team use (`R-USR-3`) needs
    per-user roles before workers on shared hosts.
13. **ACP network transport.** If the Streamable-HTTP/WebSocket RFD stabilises, O6 becomes
    cheaper. It still would not remove the need for an executor that owns the tree, so the verdict
    is unaffected.
