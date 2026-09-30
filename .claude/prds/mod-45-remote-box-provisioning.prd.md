# MOD-45 — Remote box provisioning over SSH

> Routed as **PRD** by `/handoff-run MOD-45` (C2 and C4 fired, low confidence at the threshold;
> accepted by the maintainer 2026-10-01, sandbox run `hr/MOD-45`). Ultracode recommended for the
> implement phase (C4). `docs/ANA-16.md` §7 and §8 item 6 are the design. The dependencies are
> done: MOD-41 (`docs/decisions/mod/mod-41.md`, `htui worker`), MOD-7 (`docs/decisions/mod/mod-7.md`,
> id-keyed `register_box` and the box probe), MOD-22 (`docs/decisions/mod/mod-22.md`, loopback
> paste-back).
> Requirements: `R-BOX-1`, `R-BOX-4`, `R-AGT-9`, `R-STO-1` (as amended by MOD-41), `R-ORCH-12`.

## Problem

A remote machine can only become an executing box if the maintainer installs `htui` there by hand,
writes a systemd unit, and feeds the DSN into `systemd-creds encrypt` as root. The worker guide says
so outright: "This is a sample; nothing installs it for you (MOD-45)". Every step is manual and easy
to get wrong in a way that leaks the DSN into a file, `argv` or shell history, which `R-STO-1`
forbids. Until provisioning exists, MOD-43's remote dispatch has no remote worker to dispatch to.

## Evidence

Paths are relative to the repo root, read on `hr/MOD-45` (base `de177dd`).

- **The design settles the mechanism.** `docs/ANA-16.md` §7: "Remote shell" is the provisioning path
  for a remote box's worker, not an agent transport. §8 item 6 says to use system `ssh`, install the
  matching `htui` build and a service running `htui worker`, pass the credential on stdin (never
  `argv` or a file), and let the worker self-register. SSH is not used after that.
- **The worker it installs exists.** `htui worker [--pool-size N] [--dsn-stdin] [--log PATH]` is a
  clap subcommand (`crates/htui/src/cli.rs`, `Command::Worker`, MOD-41 plan D14). Its DSN comes
  from `--dsn-stdin`, the OS keyring, or on Linux the systemd credential `htui-dsn` read from
  `$CREDENTIALS_DIRECTORY` (`docs/htui-worker.md` "Where the DSN comes from").
- **The target service shape is documented but not automated.** `docs/htui-worker.md` "Running it
  as a systemd service" is a *system* unit with `User=` and
  `LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn`. The guide explains why:
  user units can load an encrypted credential only from systemd 256 on, and Ubuntu 24.04 ships 255.
- **`R-STO-1` already permits exactly this credential** (amended on MOD-41, 2026-09-29): on a
  headless Linux host the DSN may be a systemd credential encrypted at rest and bound to the host,
  and never in `argv`, the environment or a plaintext file.
- **Self-registration is in place.** The worker uses the box's `box.toml` and registers through
  MOD-7's id-keyed `register_box` and the box probe (`R-BOX-1`). A fresh host mints its own box id
  on first start.
- **Agent login on a box the browser can't reach** is MOD-22's paste-back (`R-AGT-9`, amended
  2026-09-30). `htui` never reads, holds or moves the agent credential itself.
- No user reports or metrics. The need comes from the design (ANA-16) and from the worker guide's
  explicit gap, not from observed failures.

## Users

- **Primary: the maintainer** (`R-USR-1`). They have one or more Linux machines on a trusted
  network that they reach with `ssh`. They want each machine to run `htui` runs headless, without
  hand-writing units or handling the DSN in a shell.
- **Downstream items:** MOD-43 (remote dispatch needs remote workers), MOD-44 (a container host is
  provisioned the same way), MOD-47 and MOD-48 (phase 2 swaps the installed DSN for an enrolment
  token).
- **Not for:** team or multi-user setups (`R-USR-3`, phase 2), untrusted hosts or hosts behind NAT
  (phase 2 triggers, ANA-16 §7), or Windows and macOS targets (MOD-16).

## Hypothesis

We believe **a single `htui` command that provisions a worker over the user's own `ssh`** will
**make a remote Linux machine a usable executing box without the DSN touching `argv`, a plaintext
file or shell history** for **the self-hosting maintainer**. We'll know we're right when **a fresh,
`ssh`-reachable Linux host becomes a registered box whose worker is running and heartbeating after
one command. Agent login (MOD-22) is the only manual step left.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| One-command provisioning | A reachable Linux host with systemd and sudo ends with the matching `htui`, an enabled and running `htui-worker` service, and a new box row whose `last_seen_at` advances | Integration test against a disposable SSH target (method TBD in the plan: container with sshd + systemd, or a scripted fake `ssh`); one manual run on a real host recorded in the write-up |
| DSN never exposed | The DSN never appears in any `argv` (local or remote), in the remote environment, in a plaintext file on either side, or in a log | Tests that capture the spawned `ssh` argv and remote command strings; a check of the files the provisioner writes; log scrub test |
| Build match | The remote `htui --version` equals the local build's version | Post-install check the provisioner runs and reports |
| Idempotent re-run | Running the command again on a provisioned host leaves one service, one credential and the same box id, and reports "already provisioned" | Integration test |
| Clear refusal | Wrong architecture, no systemd, systemd too old for the chosen unit shape, no sudo, or a loopback-only DSN fails before anything is written remotely, with the reason | Tests per preflight refusal |
| SSH not reused | After the command exits, no `htui` code path opens `ssh` to that host | Code-level: the provisioner is the only `ssh` spawner; test or review check |

## Scope

**MVP**

- A CLI subcommand, `htui provision <ssh-destination>` (name final in the plan). It runs the system
  `ssh`, so `~/.ssh/config`, `ProxyJump` and the SSH agent apply unchanged. The TUI is not involved.
- Preflight on the target before anything is written: OS is Linux, CPU architecture matches the
  local build, systemd is present, sudo is available (an interactive password prompt is fine), and
  an existing install is detected.
- Install the **local running `htui` binary** on the target when the architecture matches, sent
  over the SSH channel. A mismatch is a refusal naming the architecture.
- Install the **system unit with `User=`** from `docs/htui-worker.md`, running as the SSH login user
  with that user's config directory. The log directory is created and the unit enabled and started.
- Deliver the DSN **through stdin into `systemd-creds encrypt`** on the target, which produces
  `/etc/credstore.encrypted/htui-dsn`. The DSN never appears in `argv`, the environment or a
  plaintext file on either side. The source is the local keyring's DSN by default, or one line
  from local stdin when the remote host must reach Postgres by a different address. A DSN whose
  host is loopback is refused, because it cannot mean the same server from another machine.
- Verify: the service is active, the remote `htui --version` matches, and the box shows up in
  Postgres (a new box row heartbeating) within a bounded wait. Report the new box id.
- Re-run is safe. On a host that is already provisioned it reports the state and changes nothing,
  unless a flag asks to replace the credential.
- Documentation: `docs/htui-worker.md` gains the provisioning path. It also covers the agent-login
  step on the new box using MOD-22's paste-back, run by the maintainer in their own `ssh` session.

**Out of scope**

- **Windows and macOS targets.** These need a service and credential story of their own (MOD-16,
  launchd).
- **A TUI action** (for example Settings > Boxes). This is a later item if the CLI proves out.
- **Cross-architecture installs** (downloading a release artifact or building on the target). There
  is no release pipeline to download from. The plan may add a pointer item.
- **Upgrading an installed worker to a newer build.** Version skew (ANA-16 §9.14) is handled by the
  worker's headless refusal today. A deliberate upgrade path is a follow-up.
- **Uninstall and decommissioning** of a remote box.
- **A user-unit install without sudo.** User units can't load encrypted credentials before systemd
  256. A later item can add it once that systemd version is common.
- **Agent credentials.** `htui` never moves them (`R-AGT-9`). Login stays MOD-22's in-app flow on
  that box.
- **The phase-2 enrolment token** (MOD-47/48). The credential step is kept as one seam so it can be
  swapped later, but nothing more.
- **Docker child boxes** (MOD-44), remote targeting from the TUI (MOD-43) and permission relay
  (MOD-42).

## Constraints (fixed before planning)

- **ANA-16 §8 item 6 is the design.** Any deviation is recorded in the plan with a reason.
- **`R-STO-1` as amended by MOD-41** is the credential law. The only DSN form at rest is
  `systemd-creds` encrypted and host-bound. No requirement amendment is expected. If planning finds
  one is needed, it goes back to the maintainer.
- **System `ssh` only**, spawned as a child. No SSH library that re-implements the user's SSH
  configuration (ANA-16 §4.3).
- **The worker is unchanged in behaviour.** It already reads the systemd credential. If the plan
  needs a worker change, it must say why.
- **No migration expected.** If one is needed, it is minted against main and the host lease first.
- `unsafe_code = "forbid"`, workspace lints unchanged, TDD per repo convention. The reviewer is
  `rust-reviewer` (`.claude/workflow-config.json`).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Provision a host | One command takes a reachable Linux host with sudo to a running, self-registered `htui worker`, with the DSN encrypted on the host and nowhere else. | in-progress | [plan](../plans/mod-45-remote-box-provisioning.plan.md) |
| 2 | Safe re-run and refusals | Re-running on a provisioned host changes nothing and says so. Every preflight failure refuses before a remote write, with the reason. The guide covers provisioning and agent login. | in-progress | [plan](../plans/mod-45-remote-box-provisioning.plan.md) |

Milestone 1 is the happy path end to end. Milestone 2 hardens it and can ship with it or right after
it.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-10-01 ("go with proposals"), before planning. They are
decisions, not proposals: the plan implements them.

- **D1 — System unit with `User=`, sudo required.** It matches `docs/htui-worker.md` and works on
  systemd 255. There is no user-unit fallback in the MVP.
- **D2 — Ship the local binary when the architecture matches.** Otherwise refuse. There is no
  release download and no build on the target.
- **D3 — DSN via stdin into `systemd-creds encrypt`**, which lands at
  `/etc/credstore.encrypted/htui-dsn` and is loaded with `LoadCredentialEncrypted=`. This is already
  permitted by `R-STO-1`. The worker's `--dsn-stdin` is not used by the service.
- **D4 — CLI first.** A TUI entry point is out of scope. Interactive SSH and sudo password prompts
  are acceptable.
- **D5 — First install only.** Upgrade and uninstall are follow-ups.

## Open Questions

- [ ] **Test target for the end-to-end metric.** The options are a container with `sshd` and
  systemd (no `docker` inside the `hr` sandbox), a fake `ssh` that runs a script locally, or both.
  TBD — needs validation via a plan fact-check of what the sandbox and CI can run.
- [ ] **libc compatibility of the shipped binary.** Matching the architecture does not guarantee
  that the target's glibc is new enough for the local build. TBD — needs validation via a preflight
  probe (running the copied binary's `--version` before installing the unit) in the plan.
- [ ] **Which DSN the remote host needs.** A keyring DSN pointing at a LAN address works as-is, but
  one pointing at `localhost` or a Docker-mapped port does not. The MVP refuses loopback and accepts
  an override on stdin. Whether that covers the maintainer's real hosts is TBD — needs validation
  via the first manual run.
- [ ] **Agent login step.** Paste-back needs the TUI running on that box, which here means the
  maintainer's own interactive `ssh` session. Whether that is acceptable as the documented path, or
  a headless login entry point is wanted, is TBD — needs validation via the first manual run.
  A headless entry point would be a new item.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The DSN leaks into a remote `argv` or shell history (for example `echo dsn \| systemd-creds …` in the command string) | Medium | High | The DSN only ever travels as bytes on the `ssh` child's stdin. Tests assert the command strings; the reviewer checks every `ssh` call site |
| The copied binary fails on the target (glibc, missing shared libs such as the ONNX runtime used by fastembed) | Medium | Medium | Run the copied binary's `--version` on the target as preflight, before the unit is installed. Report and refuse on failure |
| A partial install is left behind after a mid-way failure | Medium | Medium | Order the steps so the credential and unit come last. Re-run detects and completes or reports. Milestone 2 covers it |
| sudo prompts break the stdin-borne DSN (the prompt consumes it) | Medium | High | Establish sudo before the DSN is streamed, or use a separate `ssh` invocation for the credential step. Pinned in the plan fact-check |
| The worker registers a new box under the wrong user or config directory | Low | Medium | The unit's `User=` is the SSH login user. The provisioner reports the box id and hostname it expects and verifies the row |
| An end-to-end test that needs a systemd host can't run in the sandbox | High | Medium | Split the tests. Command construction and preflight logic are tested locally with a fake `ssh`; the real-host run is manual and recorded in the write-up |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
