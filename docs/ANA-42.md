# ANA-42 - Execution targets: where an item runs

> **Scope note:** Settle what an execution *target* is in the store and how it relates to boxes
> (`R-BOX-1..4`), MOD-43's target box and MOD-44's container box; where the agent process runs for
> each of the five kinds; how sessions, permissions and the MCP relay reach a remote target without
> the DSN leaving this box; how secrets reach it; how a branch is collected from a container or a
> remote checkout; and `R-NF-2` (docker as an opt-in per-target dependency). Governed by
> `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md` and
> `docs/ui-redesign-findings-2026-10-15.md` §4.2a. Read beside MOD-43, MOD-44 and MOD-47.
>
> **Requirements addressed:** `R-ORCH-15`, `R-ORCH-12`, `R-ENT-17`, `R-SEC-5`, `R-AGT-11`, `R-BOX-4`,
> `R-NF-2`. Touched at their seams: `R-SEC-2`/`-3`/`-4` (secret scope, scrub, no fallback), `R-AGT-5`
> (no agent-name keys), `R-AGT-9` (htui never holds an agent credential), `R-ID-2` (the DSN holder),
> `R-BOX-1..3` (probe and capability tags).
>
> **Status (2026-10-17): drafted, awaiting maintainer acceptance of §8.** No implementation code is
> written here. Verdict (§7): a **target is a record that points at a box and says how to reach it**;
> the engine, store writer, lease and MCP host **stay on this box** for every kind except an enrolled
> box; only the agent process is remote, driven over stdio by a launch decorator; secrets are resolved
> here and injected into that process's environment; the branch comes back as a **git bundle** through
> the same exec channel. Proposed spawns: **MOD-130**, **MOD-131**, **MOD-132**.

---

## 1. Context and problem statement

`R-ORCH-15` fixes five kinds of target and a cascade (Settings → workspace → project → workflow →
item): this box, docker on this box, a remote machine over SSH, docker on a remote machine, and a box
enrolled with an `htui` server. Wherever an item runs, `htui` prepares the code, components, environment
and secrets, and agents; the work returns when the person collects the branch. The review (findings
§4.2a) decided the product shape and the skeleton shows it. What it left open is the machinery, and
four earlier items each hold one corner of it: MOD-43 (choosing the target box), MOD-44 (the container
box), MOD-47 (the server) and TOOL-7 (`scripts/hr`, the proven local sandbox).

Method: seven readers in parallel (boxes, agent process, relay and permissions, secrets, isolation and
collection, requirements and skeleton, web prior art), each returning facts with `path:line` evidence.
Facts below are theirs; §6 and §7 are the synthesis. Two caveats on the evidence: the `R-BOX-1..4`
requirement text is not in an indexed file, so its meaning is read from code comments (§2.1); the
Docker-context and Nix rows of §4 are background knowledge, not fetched.

## 2. Surface as read

### 2.1 Boxes and the target already on a run

- A **box** is one development machine: a row keyed by a UUID in `box.toml`, with `hostname`,
  `os_family` (CHECK: windows|linux|macos), probed and declared tags, settings, and a machine
  fingerprint that detects a copied `box.toml` (`htui-core/src/model/box_.rs:22-62`,
  `migrations/0001_init.sql:51-72`, `0005_box_identity.sql`; `htui-store/src/identity.rs:1-30`).
- It has **no kind, parent, transport or address**. `parent_box` has zero hits; MOD-44's "child box of
  kind container" is still only planned (`HANDOFF.md` MOD-44).
- Everything about capability hangs off `box_id`: `box_tool`, `agent_box`, `workspace_box_path`,
  `repo_box_path` (`0001_init.sql:81-87,112-122,175-178,204-207`; `model/hierarchy.rs:194,207`).
- **A run already has a target.** `run.target_box_id` is a NOT NULL FK to `box`, beside
  `executing_box_id` and `lease_box_id`; the comment reads "R-ORCH-12 reserved: executes only when
  local", and the queue classifies a run on another box as `EntryState::Elsewhere`, reports it and never
  starts it (`0001_init.sql:455-456`, `model/run.rs:195-196`, `model/queue.rs:325,453-464`).
- MOD-43 holds that a non-local target stays `queued` until that box's worker claims it (the `htui
  worker` of MOD-41, polling Postgres under `R-ID-2`). MOD-47, the server, is trigger-gated phase 2.

### 2.2 Where the agent process runs, and what is local-only

- The agent is always a **local child**: `spawn_supervised` builds a `tokio::process::Command` in a
  process group (job object on Windows) from `ResolvedLaunch{command,args,env}`
  (`htui-agent/src/launch.rs:458,1186-1210`). ACP and the CLI transport both ride its piped stdio, so
  **stdio is the only thing a remote target must carry**.
- The seam for a decorator exists: `TransportBuilder::build(agent, on_box, caps)`, keyed by adapter,
  never by agent name (`registry.rs:29-40,80-96`; enforced by `tests/extensibility.rs`).
- **Local-only today:** the box probe (`box_probe/mod.rs:42`), install (a fetch, verify, unpack,
  one-rename into `install_root`; `install/mod.rs:1-12`) and login (`auth/mod.rs:1-60`, a 10-minute
  idle cap, URL out and code in), the 0600 `--mcp-config` file (`cli/mcp_file.rs`), and
  `this_binary()`, which is this process's `/proc/<pid>/exe` (`htui-mcp/src/host.rs`).
- Kill-tree is a process group locally (`launch.rs:895`); for docker it would be container removal; for
  ssh it is a remote process group the transport cannot observe.

### 2.3 Sessions, permissions, MCP relay

- The MCP host (tools, the store writer) lives **in the htui process that owns the session**. The `htui
  mcp` child is a byte relay that never parses MCP and holds no DSN (`mcp_cmd.rs:3-8`,
  `htui-mcp/src/channel.rs:1-9`; the writer is captured once in `Session.store`, `host.rs`).
- The child's host inputs are `HTUI_MCP_ADDR` (a Unix socket in a 0700 private dir, or a Windows pipe
  that refuses remote clients) and `HTUI_MCP_TOKEN`. The handshake is one JSON line `{token, version}`
  then NDJSON; the same-version rule refuses a child whose `RELAY_VERSION` differs
  (`channel.rs:~63-150,285`). `connection<S>` and `accept_loop<S>` already take any
  `AsyncRead+AsyncWrite`; only `platform::{bind,connect}` is transport-specific.
- **Permissions are store-mediated on the driving side:** `drive()` answers stages 1-2 by policy, and
  for stage 3 writes a scrubbed `step_permission` row then polls it at 1 s until answered
  (`record/relay.rs:22-30,520-620`; `RelayStore` in `htui-core/src/store/worker.rs:79`). Session events
  are durable rows the TUI reads from Postgres. So the TUI is already decoupled from where the agent
  runs; the **executor** (lease holder, recorder, MCP host, permission poller) is what must sit on a
  box that holds the DSN.
- The CLI transport's permission bridge (`PromptPort`/`PromptAsk`) is an in-process mpsc pair, correct
  only while the MCP host and the session driver share a process (`prompt_bridge.rs:1-35`).

### 2.4 Secrets and environment

- `R-SEC-2`: the project's secrets are resolved at run start into the **agent subprocess environment
  only**; no tool reads them; htui's own credentials are never exposed. `R-SEC-3`: a scrubber with
  exact-match masks runs before any row persists. `R-SEC-4`: provider unreachable means the run is
  refused, no plaintext fallback. `R-SEC-5`: the compose ports/URLs `.env` holds no secrets; passwords
  come from the provider (`docs/REQUIREMENTS.md:494-508`).
- `RunSecrets` resolves once per walk, is single-project, and builds env and scrubber from one map;
  the map reaches the agent only through `SessionSpec.env`, applied last over `launch.env`; `HTUI_*` keys
  are reserved (`htui-orch/src/secrets.rs:25-135`, `htui-agent/src/cli/mod.rs:587-609`,
  `htui-core/src/secret.rs:453,513-522`).
- The Infisical machine identity lives in the executing box's OS keyring; delivering one to a headless
  worker is MOD-48's job, and the trait has no bootstrap method yet (MOD-104). The provider has a login
  lockout and 429s that are never retried (`htui-secrets/src/infisical.rs:277-290,675-688`).

### 2.5 Isolation and collection

- `scripts/hr` clones with `git clone --no-hardlinks --no-checkout`, sets `pushurl` to a refusal,
  mounts the clone at the **same path** as the host repo, and shares no `~/.ssh` or git credentials
  (`scripts/hr:817-823`, `docker/hr/compose.hr.yaml`).
- `scripts/hr collect` treats the clone as hostile: only ref/object plumbing runs against it, with
  hooks and fsmonitor off; a status check that reads the worktree runs in a throwaway `--network none
  --cap-drop ALL` container; objects arrive in a temp ref `refs/hr-collect/<item>` under
  `transfer.fsckObjects`, rewritten history is refused without `--force`, and the branch moves by
  compare-and-swap `update-ref`. `--merge` confirms a diffstat of host-executing config paths, which
  `--yes` does not answer (`scripts/hr:193-204,247-262,1129-1230`).
- The `Isolator` trait has ten verbs, no store dependency, and `GixIsolator` is configured by one box's
  repo map and shells out to a **local** `git` (`htui-orch/src/isolate.rs:149`, `isolate/real.rs:215-320`).
  `RunStepTree.path` is persisted and compared later, so any path it reports must be valid on the engine's
  side or be keyed by box.

### 2.6 What the redesign already decided

`R-ORCH-15` and findings §4.2a settle the product: the five kinds and their meaning (local = worktree
"as you"; docker = fresh clone, private compose project, agent logins on a named volume; ssh = clone to
`~/htui-runs/<item>`, compose there; remote docker = a docker context over ssh; enrolled = shown, not
selectable until MOD-47), the cascade with an "inherit" row that names its source, a Settings → Running →
Targets list with a default, and a run graph that begins `prepare · <target>` and ends with a `collect
branch` step that waits for the person on every non-local target. `R-AGT-11`: the global htui agent
installs and logs in agents on a remote box, and asks before anything touching secrets, logins, remote
installs, deletions or a target change. The skeleton's `Target{name, kind, address, how, status,
planned}` and cascade resolver are throwaway (`spikes/ui-skeleton`).

## 3. What is open

1. Is a target a box, or does it point at one?
2. Who is the executor for a remote target, and what does `run.target_box_id` then mean?
3. How the MCP relay and the permission poll reach an agent that is not on this box.
4. How secret values cross the transport without argv, a file, or a stored identity on the target.
5. How the branch returns, given a hostile-checkout posture and no mount.
6. How an agent and a matching `htui` binary get onto a target, and who logs in.
7. `R-NF-2`, and which kinds work with only `git` and `ssh`.

## 4. Prior art

| Tool | What a target is | Agent/helper install | Credentials | Result return |
|---|---|---|---|---|
| VS Code Remote-SSH | an ssh `Host` entry | server auto-installed to `~/.vscode-server` on first connect | nothing copied; opt-in `ForwardAgent` | live filesystem |
| Zed remote | an ssh host | `~/.zed_server`, exact version match; client may upload the binary | ssh | live filesystem |
| devcontainer / Codespaces | a container described declaratively | features, lifecycle hooks | secrets scoped org/repo/user; recommended secret **names** only in the file | live |
| Coder | a Terraform template, run by a provisioner | agent via `CODER_AGENT_TOKEN` and init script; `coder_external_agent` adopts existing compute | provisioner holds infra creds, not the workspace | live |
| Claude Code cloud, Codex cloud | a named environment: setup script, network policy, env, secrets | cached setup | proxy-injected network secrets (agent never sees them), or setup-phase-only secrets | **a git branch** (`claude/…`, a PR) |
| Docker context over ssh | an endpoint `ssh://user@host` (background knowledge) | none beyond docker and sshd | ssh | bind mounts resolve on the **remote** side |
| Nix remote builders | a machine entry (systems, features) (background knowledge) | none | ssh, signed store paths | store paths copied back |

Lessons that bind: install the helper over the channel you connect with and **version it to the client**
(VS Code, Zed); results return as a **git branch** in every cloud agent tool, which is what `scripts/hr
collect` does; a committed target description holds names, never values; ssh-agent forwarding is the
norm for git credentials but hands the remote use of the keys while connected, so it is opt-in; remote
docker bind mounts are remote-side, so code must be **shipped**, not mounted.

## 5. Constraints

1. The DSN and htui's own credentials never reach a target (MOD-44, `R-SEC-2`).
2. `R-AGT-5`: target kind enters as a launch decorator over `ResolvedLaunch`, never as agent-name code.
3. `R-BOX-4`: identity never depends on a path; a checkout on a target is a `repo_box_path` row.
4. `R-SEC-2/-3/-4`: secrets only in the agent's environment, scrubbed on the writing side, refused (not
   cached or defaulted) when the provider is down.
5. The token is a per-session secret: never argv, never logged. `htui mcp` stdout is the protocol, so a
   tunnel must be byte-clean (`ssh -T`, no banner, stderr separate).
6. Nothing is pushed from a target; the branch returns only by an explicit collect step, with `scripts/hr
   collect`'s temp-ref, fsck, compare-and-swap and no-forced-rewrite discipline.
7. `R-NF-2`: docker and ssh are never mandatory; an unavailable target is marked, not fatal. A local or
   ssh target must work with only `git` and `ssh`.
8. `R-AGT-11`: install, login, secret and target changes need the person's permission.
9. A target in phase 2 (enrolled) must be representable as shown-but-unavailable.

## 6. Options

### 6.1 What a target is

- **A. Target = box** (add `box.kind`, `parent_box_id`, a locator). One concept and every per-box table
  works unchanged, but `box` means "a development machine" (fingerprint, `box.toml`, `os_family` CHECK) and
  a per-run sandbox would churn rows.
- **B. Separate `target` table** referenced by the cascade; resolves to a box at run time. Models per-kind
  configuration and lets two targets share one box, but run then needs both ids and every capability
  table still keys on a box, so it converges on A.
- **C. Hybrid:** `box` stays the machine (and MOD-44's container is a child box); a **`target` record**
  holds `{kind, locator, box_id, status}` and the cascade stores `target_id`; the target says how to
  reach and prepare, the box says what is there. A not-yet-registered remote becomes a box on first
  successful probe.

### 6.2 Who is the executor

- **A. Executor stays on this box; only the agent process is remote**, stdio over `ssh -T` or `docker
  exec -i`. Lease, recorder, MCP host, permission poll and the DSN stay put; `RelayStore` and the TUI are
  unchanged. Cost: one live link per session, and a drop is a failed turn.
- **B. A worker on the target claims the run** (MOD-43/47). Uniform, survives NAT, nothing local to tunnel;
  but it needs a DSN or a server, so ssh and docker targets would wait for MOD-47.
- **C. By kind:** A for the four driven kinds, B for the enrolled kind, behind one trait.

### 6.3 Relay and permissions (executor on this box)

- **A. Forwarded Unix socket** (`ssh -R remote.sock:local.sock`, or a bind-mounted socket directory for
  docker). Smallest change; the relay protocol and token are untouched.
- **B. A TCP/TLS `platform` backend** the child dials. Opens a listener on a laptop behind NAT; the token
  becomes a bearer credential on the wire. Rejected as default.
- **C. Multiplex the relay over the agent's own exec channel** (a second stream through the decorator).
  No listener and no forwarding, but needs framing in the decorator.

### 6.4 Secrets

- **A. Resolve here, inject into the exec's environment.** `R-SEC-2` untouched; one scrub path; the value
  lives only as long as the process (or per-run container).
- **B. The target resolves with its own identity.** Needs a stored identity on every target and the missing
  bootstrap (MOD-104); multiplies Infisical logins and lockout risk.
- **C. Short-lived scoped token pushed to the target.** Needs provider token minting the trait lacks and
  an `R-SEC-2` amendment.
- **D. Server-side resolution** for enrolled boxes (MOD-47/48), behind the open `R-SEC-2` amendment.

### 6.5 Collection

- **A. `git fetch` over ssh** from the target's clone. Incremental, but runs the target's `upload-pack`
  on a hostile checkout (repo-level `packObjectsHook`).
- **B. `git bundle` produced on the target, streamed through the exec channel**, fetched locally from the
  bundle into the temp ref. One mechanism for container, ssh and remote docker; `bundle verify` checks
  prerequisites; the host runs only plumbing on a file it received.
- **C. `format-patch`.** Loses merge commits and the hashes `capture` relies on. Last-resort fallback only.
- **D. Worker-mediated** upload through the server. The enrolled kind's path.

## 7. Verdict

**7.1 A target is a record that points at a box (6.1 C).** A new `target` row carries `kind`
(`local | docker | ssh | ssh_docker | enrolled`), a `locator` (JSONB: ssh `Host`/user, docker context,
image, run root), `box_id`, and a derived `status` (`ready | unprepared | unreachable | planned`). Only
the **driven** kinds create or reuse a box: docker-here is MOD-44's child box (`kind=container`,
`parent_box_id`), ssh and remote docker get a box row on first successful probe, so `agent_box`,
`box_tool`, capability tags and `repo_box_path` work unchanged. The cascade stores a nullable `target_id`
at workspace, project, workflow and item, with the Settings default; every level may say "inherit" and
the resolver reports its source. Target records carry **names and locators, never values**.

**7.2 The executor stays on this box (6.2 C).** For the four driven kinds this box's engine holds the
lease, recorder, MCP host and permission poller; the agent is a remote child behind a **`TargetTransport`
launch decorator** under `TransportBuilder` (`docker exec -i`, `ssh -T host --`,
`docker --context X exec -i`), adapters unchanged. This resolves the two ids already on `run`:
**`target_box_id` is where the work runs, `executing_box_id` is who drives it.** For driven kinds they
differ and the run starts here; only the enrolled kind is `queued` until its worker claims it (MOD-43's
rule). The queue's `Elsewhere` classification must therefore key on `executing_box_id`, not
`target_box_id == here` (the change MOD-130 owns). Kill-tree is container removal for docker and the
decorator's remote process-group kill for ssh; a dropped link is a failed turn that resumes through the
driver's existing path.

**7.3 The relay rides a forwarded socket (6.3 A).** `HTUI_MCP_ADDR` names a per-session socket on the
target, forwarded to this box's listener (`ssh -R` with stream-local forwarding; a bind-mounted 0700
directory for docker-here, checked against `sun_path` length). The relay protocol, token and
`RELAY_VERSION` rule are unchanged. The token reaches the remote child by a **stdin preamble** to the
decorator, never argv or a remote file. `McpServerSpec.command` becomes a per-target path to an `htui`
binary of the same relay version, because `this_binary()` is meaningless there. The CLI transport's
`--mcp-config` is written on the target through the same channel (0600, removed at session end). Option
6.3 C stays the fallback if forwarding proves unreliable in a spike (§9).

**7.4 Secrets are resolved here and injected (6.4 A), `R-SEC-2` unchanged.** `RunSecrets` resolves once
here; the map travels as environment variables of the exec (`docker exec -e NAME` reading this process's
environment, or an ssh stdin preamble that a small shim turns into `exec env … agent`), never argv, never
a file. The scrubber stays on the writing side. The value lives for the process; docker targets use a
per-run container so `docker inspect` exposure ends with the run. Provider unreachable refuses the run;
**target unreachable** is a distinct, reported state (`R-SEC-4` is about the provider). Compose
passwords follow the same route as interpolation from the compose client's environment (`R-SEC-5`).
Options B-D stay with the enrolled kind (MOD-47/48). Agent logins are vendor-owned and persist on the
target by the vendor's design (docker: the named volume; ssh: the remote home); htui does not handle
them (`R-AGT-9`), and a login URL/code pair crosses the same exec channel.

**7.5 The branch returns by bundle (6.5 B), fetched under `scripts/hr collect`'s discipline.** The
`collect branch` step runs `git bundle create - <branch> ^<base_sha>` on the target through the
decorator, writes it to a quarantine file here, runs `git bundle verify`, fetches into
`refs/htui-collect/<item>` with `transfer.fsckObjects` and hooks off, refuses rewritten history without
an explicit force, and moves the branch by compare-and-swap. Merging stays the person's step, with the
host-executing-paths diffstat confirmation unchanged. The target's own `git` runs only inside the run it
belongs to; the host never executes anything from the checkout. Where the engine's `Isolator` reports
`RunStepTree.path`, the path is keyed by `box_id` and read through `repo_box_path`, so a target's
checkout path is never compared against a local one.

**7.6 Preparing a target is `prepare · <target>`, a stage htui owns.** In order: probe (the existing
box probe, run through the decorator, after shipping a version-matched `htui` helper over the same
channel, Zed-style; upload from this box rather than download on the target, so a target needs no
outbound network for htui itself), clone into a run root (`~/htui-runs/<item>`, or the container
volume) with `pushurl` disabled and no credentials copied, compose project on free ports, install and
log in missing agents. Every step that touches secrets, logins, installs on a remote or a target change
is a permission card under `R-AGT-11`. Git credentials on a remote are `MOD-108`'s subject (clone onto a
box, credential guidance) and are not duplicated here; ssh-agent forwarding is an opt-in per target.

**7.7 Phasing.** The kinds are one trait with five implementations, shipped in this order:

1. **This box and docker here:** the target record, cascade and `executing_box_id` semantics (MOD-130)
   on top of MOD-44's decorator, plus bundle collect (MOD-132). These match `scripts/hr`, which remains
   the maintainer's dev sandbox (TOOL-7) and is a behavioural reference, not code to reuse.
2. **ssh and remote docker:** the ssh/context decorator, relay forwarding, helper shipping, remote
   bootstrap (MOD-131).
3. **Enrolled:** worker claim over the server (MOD-47), reusing the same trait; shown, not selectable,
   until then.

**7.8 `R-NF-2`.** Amend to: no required dependency on any external daemon other than Postgres, the
agents and the code-context tools; **`dockerd` and `sshd` are opt-in per target**, gated by the `docker`
capability tag from the box probe and by a successful target probe. A project that never adds a target
needs neither. This is the amendment MOD-44 already lists as open; accepting it here closes both.

## 8. Maintainer decisions requested

1. **Target = record pointing at a box (7.1).** Accept, or choose target = box (6.1 A)?
2. **Executor on this box for driven kinds (7.2)**, with `target_box_id`/`executing_box_id` split. This
   changes MOD-43's wording ("a non-local target stays queued") to apply to the enrolled kind only.
3. **Bundle collect (7.5)** as the single mechanism, rather than fetch over ssh for ssh targets.
4. **Amend `R-NF-2` (7.8)** to make `dockerd` and `sshd` opt-in per target.
5. **Phasing (7.7):** is shipping this-box and docker-here before ssh acceptable, given the review lists
   all five kinds as the requirement?
6. **Secrets by injection (7.4)** needs no `R-SEC-2` amendment; confirm that the server-resolution
   amendment stays with MOD-47/48.

## 9. Risks and unknowns for the spawned items

- Stream-local forwarding of the relay socket over ssh, and `sun_path` length inside a container, need a
  spike before MOD-131 commits to 6.3 A. If either fails, fall back to 6.3 C.
- ACP session resume on a fresh process after a dropped link is not shown to exist; a drop is a failed
  turn until it is.
- A static relay-only helper binary would avoid the same-version rule's equality demand; today the helper
  is the full `htui` binary of the same relay version.
- Deleting a box with run history is refused by the plain FK on `run.target_box_id`; ephemeral sandbox
  boxes need a retention rule (kept for history, hidden from pickers).
- Cached preparation (Codex-style, invalidated on script or secret change) is deferred; v1 prepares each
  item.
- Windows: docker is Linux and macOS first (MOD-44), and ssh process-group signalling differs.

## 10. Spawned items

- **MOD-130 - Execution targets: target record, child boxes, target cascade.** Migration for `target`
  and `box.kind`/`parent_box_id`; relax the `os_family` CHECK for child boxes; `target_id` on workspace,
  project, workflow and item with the Settings default and an inherited-from resolver; `executing_box_id`
  vs `target_box_id` in the queue and `Elsewhere`; the `R-NF-2` amendment; wires MOD-44's container box.
- **MOD-131 - Driven targets: ssh and docker exec launch, MCP relay tunnel, remote bootstrap.** The
  `TargetTransport` decorator for ssh, docker context and docker exec; relay socket forwarding and token
  preamble; per-target helper path and shipping; remote probe, install and login through the channel.
  Blocked on MOD-130, MOD-44.
- **MOD-132 - Target preparation and branch collection by bundle.** The `prepare · <target>` stage and
  the `collect branch` step: clone, no-push, compose project, bundle create/verify/fetch under the
  `scripts/hr collect` discipline, the config-path diffstat confirmation. Blocked on MOD-130.

MOD-47 (enrolled boxes) keeps its trigger gate and gains a pointer to §7.2 and §7.4.
