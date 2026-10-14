# HANDOFF - Outstanding Work (htui)

> **Purpose:** Carry outstanding work between sessions so work resumes.
> `htui` is a Rust TUI that wraps coding agents (`claude`, `agy`) to run a backlog of work items
> through step graphs across projects and boxes. Product contract: `docs/REQUIREMENTS.md`
> (requirement IDs `R-<AREA>-<N>`). Standing invariants: `CONCEPTS.md`.

## How to use this file

- **Session start:** read this file first. Pick the next open item.
- **On completion:** delete the checklist line, write `docs/decisions/<prefix>/<prefix>-N.md`,
  prepend the index line to `DECISIONS.md`, update the summary table (per
  `.claude/rules/workflow-docs.md`; this repo is outside the `sync-workflow-surface` default
  target list, pass `-Targets` explicitly to receive the surface).
- Every item cites the requirement IDs it addresses (`R-NF-4`).

**Current status (2026-10-14):** **TOOL-8 is done** (`docs/decisions/tool/tool-8.md`): the `htui-dev` mod
(`.claude/skills/htui-dev/`, `docs/htui-dev.md`) refuses six kinds of costly call (M1), and keeps a status line in
interactive sessions (item and HANDOFF phase, merge/ahead/dirty, disk above 90%, Postgres not up, last gate) with a
toast when a gate finishes, plus turnless `/gate`, `/next` and `/validate` (M2); `scripts/dev-health.sh`/`.ps1` are
new. Owed on the host: a Windows run (`claude -p "/gate fmt"` returns at once, the `.ps1` commands), `dev-health`'s
docker path against the real `htui-postgres`, and the Postgres gate. Before it, **TOOL-9 is done**
(`docs/decisions/tool/tool-9.md`): `scripts/gate.sh` and `gate.ps1` run the one gate, detached on request, result in
`target/gate/last.json`; the workspace pre-commit is committed in `scripts/githooks/` (`core.hooksPath`);
`DECISIONS.md` merges by union. Owed on the host: a Windows run and the Postgres gate (both `.ps1` self-tests now
pass under PowerShell 7.5 on Linux, TOOL-8). Before it, **CLEAN-11 is done** (`docs/decisions/clean/clean-11.md`).
Still owed on the host from earlier items (see their write-ups): the Postgres gate on the merged tree for CLEAN-11,
MOD-67, MOD-96, CLEAN-10 and MOD-93, Windows runs for MOD-67 and MOD-93, MOD-93's live install and `tsc`. Open
from ANA-29: **TOOL-10**, **CLEAN-12**, and **MOD-94** behind a maintainer decision.
Live coordinates after MOD-96: migrations run through `0018_seed_permission_mode` (MOD-96: data only, pins
`acp.permission_mode` on the seeded `claude` row), before it `0017_follow_up` (`run_command.run_step_id`,
`text`; `follow_up_window`; MOD-12's `0016_auto_queue`: `queue_entry`, `queue_batch`, `run.batch_id`), so **the next
migration is `0019`** (cache: `0005`). MOD-37's `0014_run_step_opening` and MOD-11's queue migration both landed as
`0014`; MOD-11's was renumbered at the merge, and MOD-70's `0016` became `0017` at the merge with MOD-12. Pins: store
conformance `CASES` 159 (MOD-10 M4 adds the secret-column case), `READ_CASES` 15,
`htui-orch` `CASES` re-pin from the merged tree; Postgres tables 45.
---

## Open items

### Analyses


- [ ] **ANA-25 - Learn per-model weights from htui's own judge verdicts** (from ANA-21; ANA-21 is
  done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`, `R-ORCH-7`, `R-ID-6`.
  ANA-21 deferred learned weights behind a volume trigger but the maintainer asked for it to be
  tracked rather than remembered. **Trigger: do not start until MOD-36 is done and a project holds
  on the order of 100 judged cross-model groups.** MOD-11 is done (`docs/decisions/mod/mod-11.md`):
  production judges now resolve on agent-written `judge` documents, so verdicts accumulate. The data is already recorded —
  candidate `run_step` rows carry `agent_id`, `model`, `selected` and `verify_outcome`, and the
  judge row (`fanout_index = -1`) carries its own `agent_id`/`model` — so no new logging is needed.
  Fit a Bayesian Bradley-Terry model with a Plackett-Luce top-1 likelihood per verdict, taking
  ANA-21's tier as the prior and a same-family-as-judge covariate, since the documented self- and
  same-family preference (+10 to +25 points self, +3.4 to +8.4 same family) is as large as the
  signal being measured. Show the fit read-only beside the manual weights before anything is applied
  automatically. The fit is deterministic arithmetic over rows, so it is allowed under `R-ID-6`.
  Judge choice itself is out of scope: ANA-21 §2.2 established the judge is never asked of the
  selector, and MOD-36 owns the judge-identity hardening.
- [ ] **ANA-28 - `heavy_build`: queue switch vs required box capability** (from MOD-11,
  `docs/decisions/mod/mod-11.md`). `R-MCP-3`, `R-ORCH-10`. R-MCP-3 says an item carrying the
  `heavy_build` tag forces `command_run` on. MOD-11 reads that tag from `item.required_tags`, the only
  item tag field, and that field is also the item's required box capabilities (R-ORCH-10). So
  tagging an item `heavy_build` both exposes `command_run` and lets the item run only on a box that
  declares `heavy_build`; elsewhere the run is refused with `missing tags: heavy_build`
  (`docs/htui-mcp.md`, "When `command_run` is offered"). The other way to get the queue on a
  single-agent step, a phase's `command_queue = always`, is settable only in Postgres (Settings ›
  Kinds shows it read-only). Options to weigh: a non-routing item label, exempting `heavy_build`
  from the box-tag match, or a phase/queue setting in the TUI.
- [ ] **ANA-30 - Utility models: auxiliary cheap-model calls (session titles etc.)**. Survey every
  place htui could use a small, cheap model for background work: session and item titles,
  summaries, history digests, commit messages, judge or triage calls, and similar. Motivation: a
  $100 Claude API credit is available and needs uses. Per use, record the call site, the model tier
  (`docs/ANA-21.md`), the API route (direct Claude API vs the `claude` CLI that htui wraps), an
  estimated token cost, and the trigger. Then budget the uses against the credit and decide
  whether a `utility_model` setting is worth building. A MOD is spawned only if the verdict says
  yes. Not blocked.

### Next features
- [ ] **MOD-36 - Weighted agent assignment across fan-out candidates** (from MOD-4 milestone 4,
  OQ-7; ANA-21 is done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`,
  `R-ORCH-7`. Milestone 4 runs every candidate of a group on the
  one agent the walk selects (rival sampling). This item spreads candidates across the eligible
  agents by weight: a weighted `AgentSelector` (the seam milestone 4 leaves per-candidate) picks each
  `fanout_index`'s agent from the walk's eligible list using ANA-21's weights, so the judge compares
  different models on the same task. Slot 0 stays `eligible[0]` and only the rival slots are
  apportioned, so the priority order stands (`docs/ANA-21.md` §5.5, §7.2). Open points to settle in
  the plan: the prompt is assembled once
  per group, but token budgeting uses one agent's estimator (`TokenEstimator::for_agent`), so mixed
  families need either the tightest budget or per-candidate trimming, which breaks "identical prompt
  across siblings" (ANA-5 `:1870`); each candidate draws on its own agent's quota; the judge must not
  learn which model wrote which candidate (position-bias control extends to model-identity bias).
  **ANA-17 note (2026-09-25, `docs/decisions/ana/ana-17.md`):** the section framing is one frame for
  every model, so the estimator is the only thing a mixed-family group can diverge on. The function is
  `TokenEstimator::for_model` (keyed on a model id), not `for_agent`, and every production
  `PromptSpec` passes `TokenEstimator::DEFAULT` today (`htui-orch/src/engine.rs:4652`, `:5374`,
  `htui/src/preview.rs:306`), so choosing it per group or per candidate is new wiring.
  **MOD-23 note (2026-09-30, `docs/decisions/mod/mod-23.md`):** ANA-21 item 8's "MOD-23's editor
  surfaces `settings.weights`" moved here: the Settings > Agents edit form carries `settings`
  untouched, and a weights field is one more entry in its field list (plan D231).
  **ANA-24 note (2026-10-01, `docs/decisions/ana/ana-24.md`):** the seed `weights.sources` registry
  takes `docs/ANA-24.md` §5: `epoch-mirrorcode` (primary) and `arena-webdev`/`arena-agent` join the
  implement axis, `epoch-eci` is narrowed to `eci_scores.csv`, `tbench-4-0` stays blocked. Each
  dynamic source needs a parser under the fetch conditions of §6 (Arena from Hugging Face only, with
  a per-fetch licence check; Epoch keyed on `benchmark_metadata.csv`, never `*_external.csv`).
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees. `htui-orch`, its `Isolator` seam and `run_worker.rs` exist since MOD-4 (done, `docs/decisions/mod/mod-4.md`); the MCP half needs MOD-11.
  The named personas a `handoff` targets exist since MOD-26 (done, `docs/decisions/mod/mod-26.md`):
  `persona` registry rows, bound per phase, frozen into the run snapshot by name.
  **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T6-T7):** settle in the PRD, from OpenRig's queue
  and workflow runtime: (1) **The swarm baton is a typed exit.** An agent yields with one exit from
  a closed set the step declares (`handoff` to a named persona, `done`, `blocked` on an item,
  `failed`), validated by code. The orchestrator closes the step and creates the next step row in
  one transaction, and a per-run hop cap stops cycles. The agent proposes, the code records
  (`R-ID-6`, ANA-2 invariant 3). Each yield starts a new step with its own fresh session fed by
  documents (`R-PRM-1`). This overrides ANA-13 §3.3's "the orchestrator maintains a single shared
  context window" (`docs/ANA-13.md:59`), and the PRD must settle that conflict explicitly. (2)
  **`spawn_subagent` has a consult mode.** A read-only child, with no tree and no output document,
  returns its answer as the tool result. It is still its own `run_step` with `parent_step_id`,
  recorded and replayable (`R-HIS-1`, `R-HIS-2`), and draws on its own agent's quota (`R-AGT-7`).
  This is htui's form of OpenRig's `rig send`, without a bus (ANA-13 §3.3). (3) Open for the PRD:
  depth and breadth limits against `max_agents_per_run`; how a child fits `UNIQUE (run_id, position,
  attempt, fanout_index)`; crash recovery of a child under ANA-2 §4.9; and whether graph steps deny
  the harness's own subagent tool, whose sessions htui never sees. Cites `R-MCP-2`, which already
  lists the tool.
- [ ] **MOD-16 - Windows runtime verification of the agent driver** (from MOD-2). `R-AGT-1`,
  `R-NF-3`, `R-HIS-1`. **This is now the only Windows check** (TOOL-3 decided 2026-09-28,
  `docs/decisions/tool/tool-3.md`): the maintainer accepted that
  `cargo clippy --target x86_64-pc-windows-msvc` cannot run on this box, so this item carries the
  runtime half *and* the compile-level branch coverage that was lost. The line was green when
  MOD-2 wrote this entry and had caught two defects a Linux build cannot see (`c4d65ba`); it is
  green for neither crate now, `ring`'s build script being the blocker on a path that predates
  MOD-20. Every Windows-only path MOD-2 and MOD-20 wrote was never ran. Beyond the four runtime
  facts below, this item also inherits the branch coverage `ring` took away — see TOOL-3's "What
  MOD-16 inherits". The four runtime facts are:
  the job object's kill-on-close guarantee leaves no `node`/`claude` process behind (`docs/ANA-4.md`
  §11 criterion 11's Windows half); `CreateProcess` refuses a `.cmd` shim, so `${claude}` resolving
  to one must fail with a message naming the shim rather than a bare `os error 193`;
  `CREATE_NO_WINDOW` actually suppresses the console; and milestone 4's `[H-1]` buffer sealing
  behaves under Windows rename semantics, where a sealed file held open by another process makes
  the rename fail rather than silently succeed (`seal_orphaned` must warn and carry on, and
  `seal_one`'s numbered fallback must not lose a tail). Also run the Postgres suites there: the
  cache is SQLite on a different filesystem, and `append_pending`'s `OpenOptions::append` and the
  `.jsonl.open` sealing have never met a Windows file lock. Needs a Windows box with `node` and
  `claude-agent-acp` installed. Not blocked, and **MOD-2 is now done across all
  nine milestones** (`docs/decisions/mod/mod-2.md`), so the full Windows surface is here rather than
  accumulating. Milestone 9 added one more runtime fact to the list: `htui_agent::excerpt::FsRepoReader`
  is the **first `htui-agent` code that traverses arbitrary repositories**, and its guarantees are
  `symlink_metadata` per component, `.git` pruned as a path component at any depth, and a scan cap
  counting every entry — none of which has met an NTFS junction, a reparse point or a case-insensitive
  path. Paths are repo-relative by contract (ANA-5 §4.2 rule 5) and the digest LF-normalises before
  any byte is counted (criterion 6), which is the Windows hazard that would otherwise change a
  `prompt_digest` between boxes. Criterion 11's CLI half also holds on **Linux only**.
  **MOD-11 added the MCP host's Windows half** (`docs/decisions/mod/mod-11.md`): the named-pipe
  listener (`\\.\pipe\htui-mcp-<uuid>`, first instance, remote clients refused), the relay's
  1 s drain in place of a half-close, and a failed `connect` replacing its pipe instance. It
  type-checks for `x86_64-pc-windows-gnu` and has never run; a Windows box should run
  `crates/htui/tests/mcp_stdio.rs`'s cases by hand.
  **MOD-20 added a second body of Windows-only code** (`docs/decisions/mod/mod-20.md`), and it is
  the first that could not be lint-checked from Linux at all (**TOOL-3**), so it was reviewed by eye
  only. The runtime facts it defers here, by name: that `Layout::promote`'s three-attempt
  `PermissionDenied` backoff actually clears a Defender lock on a freshly written `.exe`; long-path
  behaviour past 260 characters under `<install root>\<id>\<version>\`; that `create_link`'s
  Windows fallback (a small file naming the target, since a real symlink needs a privilege an
  ordinary user lacks) is never mistaken for the adapter; `fs4::available_space` on a junction;
  `reqwest`'s `system-proxy` reading the OS proxy configuration; and whether a partial
  `.staging/` entry can be removed while its file handle is open, which `fetch.rs`'s abandon path
  was restructured for but which no Linux test can distinguish.
  **MOD-21 added a third body of Windows-only code** (`docs/decisions/mod/mod-21.md`), unlinted for
  the same TOOL-3 reason and reviewed by eye. The runtime facts it defers here, by name: whether any
  Windows opener honours `BROWSER` at all and whether `cmd.exe /c exit 0` is a value it accepts —
  the neutraliser works only on an opener that *word-splits* the variable, and one that treats it as
  a single path falls through to exactly the stdout hijack the policy exists to stop, which was
  observed live on Linux; that `powershell.exe -NoProfile -NonInteractive -Command "Start-Process
  -FilePath $env:HTUI_OPEN_URL"` reaches the default browser under `CREATE_NO_WINDOW`
  (`ShellExecuteW` is unsafe FFI the workspace forbids, which is why the URL travels in the child's
  environment); and that the job object reaps the auth child **with its loopback listener** — MOD-2
  §11 criterion 11, now with a socket and a child that lives for minutes rather than seconds.
  **MOD-4 handed over two more** (`docs/decisions/mod/mod-4.md`). **R-10**: a SIGKILLed orchestrator's
  agent survives, because the child leads its own process group and `ChildGuard::drop` cannot run in
  a killed process; signalling a stale pid needs `unsafe` or a dependency, so the design (a pid in
  `session_started` plus a signal on adoption, or a death signal, with the reused-pid hazard stated)
  is this item's. **R-45**: `htui` now links `gix`, `process-wrap` and `walkdir` through `htui-orch`.
  MOD-4 also made the `git` CLI (≥ 2.33.0) a runtime dependency of the `worktree` mode and of
  reconciliation, and none of those `git` calls has run on Windows.
  **MOD-7 milestone 1 added a fourth body** (`.claude/plans/mod-7-box-identity-probe.blueprint.md`
  §11; MOD-7 is done, `docs/decisions/mod/mod-7.md`), unlinted for the TOOL-3 reason and reviewed
  by eye: the `MachineGuid` fingerprint reader
  (`htui-store/src/identity.rs`, `windows-registry`); `sysinfo`'s OS, CPU and RAM facts; the GPU
  scan through an absolute-path `powershell.exe` CIM query (`htui-agent/src/box_probe/hardware.rs`);
  and two box-probe behaviours only a Windows box shows — the seed's `bash` resolving to the WSL
  launcher `%SystemRoot%\System32\bash.exe`, which boots the WSL VM and records the distro's bash
  (documented, turned off by `{"tools":{"bash":{"disabled":true}}}` in `box_probe_spec`; a code fix
  that names no tool is this item's call), and the Microsoft Store `python3` stub, which the probe now
  skips by trying each name until one prints a version (verify it holds).
  **MOD-22 and MOD-41 added two more** (`docs/decisions/mod/mod-22.md`, `docs/decisions/mod/mod-41.md`):
  whether MOD-22's loopback paste-back completes a login from a Windows box, and whether
  `htui worker`'s Windows stop path (Ctrl-Break, closing the console, a system shutdown;
  `docs/htui-worker.md`) cancels its walks and gives their leases back as it does on SIGTERM.
  **ANA-27 note on R-10 (2026-10-01, `docs/ANA-27.md` §5.1 T10):** if R-10 picks the pid-and-signal
  design, record the agent child's pid **and its start time** in `session_started`, and signal on
  adoption only when both still match. A reused pid then fails the match instead of receiving the
  signal. This does not remove the need for `unsafe` or a dependency to send the signal, and the
  death-signal alternative avoids the reused-pid hazard entirely. OpenRig treats pid plus start time
  as a process's identity in the same way (an argv token match, then a lineage fingerprint including
  `lstart` compared across two `ps` observations).
  **MOD-68 added one more** (`docs/decisions/mod/mod-68.md`, OQ-1): the `rten` embedder was checked
  only by a cross `cargo check` for Windows and macOS. Run `htui --index-items` (or a Ctrl+F search)
  there once with an empty `<cache>/htui/model`, so that the pinned fetch, the locked `.part`, the
  rename onto a file another process may hold open, and the golden tests
  (`cargo test -p htui-store --features local-embed --lib embed -- --ignored`) run on that platform.
  **MOD-71 and MOD-74 added the mouse** (`docs/decisions/mod/mod-71.md`, `docs/decisions/mod/mod-74.md`).
  Exercise capture in the Runs flow view (`v`) on Windows Terminal and on the legacy console: a click,
  a drag pan and a wheel zoom, then an `$EDITOR` handoff before the flow view was ever opened (MOD-71
  review H1's path). `cargo check` the `#[cfg(windows)]` half of `EnableButtonMouseCapture`
  (`crates/htui/src/terminal.rs`, MOD-74 D5), which no Linux build compiles.
  **From MOD-55 (review L1, `docs/decisions/mod/mod-55.md`):** `edit_help::holds_mask` compares `[REDACTED]` counts
  in the sent body and the proposal. MOD-10 is done (`docs/decisions/mod/mod-10.md`), but a help turn still scrubs with
  an empty `env` (pattern rules only). Once help turns mask a project's resolved values, the help runtime should
  report how many masks it applied, so a body's literal `[REDACTED]` cannot hide a masked value from the double-accept.
  **MOD-57 M1 added the in-pane editor's Windows half** (`docs/htui-editor.md`, Windows): ConPTY
  through `portable-pty` (`cmd /S /C` with the temp directory as the working directory); `Ctrl+\`
  arrives as `ctrl-\` there, so only a physical `Ctrl+4` reaches `editor.focus`; the kill is
  `TerminateProcess` (no process-group kill); `ClosePseudoConsole` runs on its own thread; and a
  forwarded `Ctrl+C` must reach the editor without ending htui.
  **ANA-29 note (2026-10-08, `docs/ANA-29.md` §8):** once they land, run on Windows MOD-91's
  isolation flags against a subscription login, MOD-92's `--json` verbs (landed; keyring DSN, exit codes, `docs/htui-cli.md`),
  and MOD-93's `htui claude emit` with a `claude plugin marketplace add` of the emitted directory,
  with the mod reaching `htui.exe` through `$.process.run`. Mods themselves are documented with
  PowerShell setup and `;` path lists, but native Windows behaviour is unconfirmed. MOD-57 is done
  (`docs/decisions/mod/mod-57.md`); its M2 adds to this check: other tabs working while the ConPTY editor
  waits, and the quit and abort questions killing it.

- [ ] **MOD-43 - Remote dispatch in the TUI** (from ANA-16, §8 item 4). `R-ORCH-11`, `R-ORCH-12`,
  `R-TUI-1`, `R-NF-3`. Target box on run start and in auto mode; a non-local target stays `queued`
  until its worker claims it; the Runs view follows `session_event` by `seq` with `LISTEN`/`NOTIFY`
  hints and a poll backstop, and shows worker liveness. Its auto-mode half builds on MOD-12, now done
  (`docs/decisions/mod/mod-12.md`: the queue runs on the local box only and reports a run live elsewhere)
  (MOD-42 is done: `docs/decisions/mod/mod-42.md`; its relay answers within a 1 s poll, which a
  `NOTIFY` hint would shorten).
  **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T3):** the liveness the Runs view shows is
  derived from rows at read time, never reported by the agent, and "unknown" is a value. Per running
  step: *working* (a `session_event` within a window), *quiet* (none within it), *waiting on you*
  (MOD-42's open permission requests, with count and tool), *overdue* (past `deadline_seconds`;
  display only; MOD-37 phase 4 now cuts the session at the deadline), and *unknown* (the run's lease holder has not
  refreshed within the TTL). Per run: *queued, no worker* when the target box's worker has not
  checked in (`docs/htui-worker.md`, "the pane cannot tell you why"). The windows are settings, and
  each label shows the age of the evidence behind it.
- [ ] **MOD-44 - Container execution environment** (from ANA-16, §8 item 5). `R-BOX-1..3`,
  `R-AGT-5`, `R-AGT-6`, `R-AGT-9`, `R-SEC-2`, `R-MCP-1`, `R-NF-1`, `R-NF-2`. A child `box` of kind
  `container` with its own id and hostname; probe, install and auth inside the image; one container
  per session as host UID/GID; trees bind-mounted at identical paths; a launch decorator under
  `TransportBuilder` (`docker exec -i`), adapters unchanged; kill-tree is container removal; agent
  credentials on a named volume; the container never holds the DSN. Linux and macOS first (Windows
  via MOD-16). **Open question for the maintainer:** `R-NF-2` amendment (`dockerd` as an opt-in,
  per-box dependency). The MOD-7 dependency is met (done, `docs/decisions/mod/mod-7.md`: id-keyed
  registration, the box probe, capability tags); survives TUI exit through `htui worker` (MOD-41, done: `docs/decisions/mod/mod-41.md`).
  A child box's hostname never moves a digest, and the project's hostname switch covers it by
  construction (MOD-33 D275, `docs/decisions/mod/mod-33.md`).
  **MOD-11 note (2026-10-04, `docs/decisions/mod/mod-11.md`):** `R-MCP-1`'s server is a stdio
  `htui mcp` relay the agent starts, with `HTUI_MCP_ADDR` and `HTUI_MCP_TOKEN`. It connects to the
  hosting htui process's local channel: a Unix socket in a `0700` directory, or a named pipe on
  Windows. That channel is the seam a container bridges (`docs/htui-mcp.md`, "The socket" and
  "`htui mcp`: the relay"). Today the relay command is `/proc/<host pid>/exe` on Linux, which an
  agent in a container that does not share the host's `/proc` cannot start, and the socket must be
  bind-mounted into the container. A container launch needs a relay binary inside the image and the
  socket directory mounted, or a relay over the `docker exec -i` stream (PRD OQ-1).
- [ ] **MOD-46 - Live streaming via `NOTIFY` (optional)** (from ANA-16, §8 item 7). `R-HIS-1`,
  `R-NF-3`. Transient `NOTIFY` deltas under 8000 bytes between recorder flushes, droppable, superseded
  by durable `session_event` rows. Start only if 16 KiB flush bursts prove unusable; replaced by
  MOD-47's relay if phase 2 is already open. Blocked on MOD-43.
- [ ] **MOD-47 - `htui server` control plane (phase 2)** (from ANA-16, §7, §8 item 8). `R-NF-2`,
  `R-ID-2`, `R-ORCH-12`, `R-STO-1`, `R-STO-5`, `R-USR-3`, `R-SEC-1..4`, `R-ID-7`. **Trigger-gated:**
  start only when a worker runs outside the trusted network or behind NAT, team use (`R-USR-3`)
  starts, worker count exceeds the Postgres connection budget, or MOD-46 proves inadequate. The only
  worker-facing DSN holder, no durable state (`R-ID-3` stands); enrolment with server-minted box ids
  (replacing MOD-7's `box.toml`-keyed registration for such workers; MOD-7 done,
  `docs/decisions/mod/mod-7.md`),
  box-scoped auth, versioned worker protocol accepting N-1, worker-initiated WebSocket for dispatch,
  event ingest, live and permission relay; a decision on server-down behaviour. The TUI keeps
  talking to Postgres. **Open questions for the maintainer (requirement amendments):** `R-NF-2`
  (optional self-hosted server), `R-ID-2` (self-hosted control plane is not a cloud service),
  `R-ORCH-12` ("polling Postgres or the control plane"), `R-STO-1` (worker holds a box-scoped
  revocable key), `R-STO-5` (server owns migrations for workers), `R-USR-3` (roles enforced in the
  server). Not blocked: MOD-40, MOD-41 and MOD-42 are done (`docs/decisions/mod/mod-40.md`,
  `docs/decisions/mod/mod-41.md`, `docs/decisions/mod/mod-42.md`); the worker reaches the store only
  through `WorkerStore`/`WorkerHost`/`RelayStore`, which a control-plane client would implement, and
  MOD-42's `step_permission`/`run_command` rows are the payload the server relays.
- [ ] **MOD-48 - Config manager and secret distribution (phase 2)** (from ANA-16, §6.2, §8 item 9).
  `R-ID-3`, `R-AGT-9`, `R-AGT-10`, `R-SEC-1`, `R-SEC-2`. `GetManifest`/`WatchManifest` over agent
  registry, box profiles, settings, images, target build and digest; full resync on a stale cursor;
  worker-side cache; worker self-update. Targeted per-run secrets, never agent credentials
  (`R-AGT-9` unchanged). **Open question for the maintainer:** `R-SEC-2` amendment only if the server,
  not the worker, resolves project secrets. Blocked on MOD-47. MOD-10 is done (`docs/decisions/mod/mod-10.md`):
  the executing box resolves a project's secrets with the machine identity in its own keyring; a headless worker
  without one refuses, and delivering it is this item's.

- [ ] **MOD-85 - Remaining accent (cyan) uses that do not mean focus or selection** (from MOD-80,
  `docs/decisions/mod/mod-80.md` "Carried"; blueprint B-6, review M1/L5). `R-TUI-1`. MOD-80 made
  `accent` mean focus and selection, gave keys, running, warnings and diff-added their own theme roles,
  and moved the Runs pane's needs-a-person lines to `warning`. About twenty `accent` uses still mean
  something else: the Documents kind column (`detail/documents.rs`), flow edge labels
  (`runs/execution_graph.rs` `edge`), divergence Theirs/Mine (`backlog/divergence.rs`), chat headers,
  transcript `you` and notes, permission-strip digits, and the requirements form `[value]`. Decide
  each one (`key`, `dim`, `warning` or `base`), and pin the decisions with buffer-style tests. Also:
  the Skills attach activation field marks focus with `selected` rather than `accent`, and the
  concepts-search, workspace-switcher and path-picker overlays mark their cursor row with `accent`
  alone, which is bold-only under `NO_COLOR`. Style only. Not blocked; touches views that MOD-81
  and MOD-82 also rewrite, so run it after them or beside them with care (MOD-67 is done,
  `docs/decisions/mod/mod-67.md`).
- [ ] **MOD-81 - Narrow and wide terminal widths** (from the TUI design review of 2026-10-04,
  https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 4, 14). `R-TUI-1`, `R-TUI-2`,
  `R-TUI-8`. Below 100 columns the Settings > Agents table drops its name column, the only
  `Constraint::Fill` (`ui/tabs/settings/agents.rs:1842`), and rows lose what identifies them; the
  Backlog detail sub-tab strip clips `Notes Prompt Reqs` with no overflow mark; Backlog titles shrink
  to `Chapter…` at 60-80 columns because the status column is sized for `awaiting_approval`; the
  Prompt pane clips its right edge without wrapping. At 160 columns the fixed list/detail ratio gives
  the mostly empty list 86 columns and the reading pane 70. Give identifying columns a minimum
  width, shorten statuses below about 90 columns, mark a clipped strip with `›`, wrap or scroll the
  Prompt pane, and cap the list width so the detail pane takes the rest. Rows fit through
  `ui::cells` (MOD-60). MOD-67 is done (`docs/decisions/mod/mod-67.md`): hints now sit in one row above the status line.
- [ ] **MOD-82 - Shared pane chrome: sub-tabs, detail header, overlays, Settings layouts** (from
  the TUI design review of 2026-10-04, https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 9,
  13, 15, 16). `R-TUI-1`, `R-TUI-3`, `R-TUI-7`, `R-TUI-8`. Secondary navigation comes in three
  forms: Skills draws `Skills │ Templates` outside its box with the active tab white bold, while
  Backlog detail and Settings draw a row inside the box with the active tab cyan; make one sub-tab
  widget. The Backlog detail header repeats itself (block title `FEAT-1`, then `FEAT-1  FEAT
  in_progress`, the kind being the key's prefix); put key and title in the block title and status,
  tags and priority on the meta line. The concepts search overlay leaves a one-column strip of the
  panes below at its edges and at 60 columns covers the tab strip; clear a margin around overlays or
  dim the backdrop. The eight Settings sections use five layouts (table, tree, key-value,
  master-detail, one-line records) with different indents, and Connection says `press Enter or R`
  where its hint row says `R rebuild cache`; settle on table and key-value with shared label widths.
  MOD-67 is done (`docs/decisions/mod/mod-67.md`); carried here from it: the Prompt pane header's `n / p · N
  templates` and the Skills/Templates pane titles' ` J/K scroll ` still name keys as literal pane chrome.
- [ ] **MOD-83 - Display labels and actionable errors** (from the TUI design review of 2026-10-04,
  https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 10, 12). `R-TUI-1`, `R-TUI-4`,
  `R-TUI-8`. Internal values reach the screen: snake_case statuses (`awaiting_approval`), source tags
  (`(app_setting_default)`), units (`1000 bp`), raw scores (`3.000`), `hit(s)`, `template(s)`, `1
  options`, `gate soft · in none · budget inherit`, and step ids such as `0.1/0` in the Runs flow.
  Give each shown enum a display label and add one pluralising helper. Errors name no next step
  (`qdrant: query: connection refused`, `hierarchy needs Postgres`); each one says where to fix it
  (for example Settings > Connection or Settings > Qdrant). Not blocked; copy only, but it shares
  snapshots with MOD-81 and MOD-82, so it lands beside or after them.
- [ ] **MOD-75 - Agent question tool: an MCP tool that parks the step for a person** (from MOD-69,
  `.claude/prds/mod-69-waiting-on-you.prd.md`). `R-MCP-1..4`, `R-TUI-11`. A step's agent that
  finds something it did not expect, or needs an opinion, has no way to ask: the question lands in
  the transcript, and an ungated step still ends `done`, so nobody sees it. Add an MCP tool the
  agent calls with the question (and optional choices); the call is recorded durably, the step
  parks waiting on a person, and the answer goes back to the agent as the tool result or a
  follow-up turn. MOD-69's waiting-on-you list shows each open question as its own row. Whether a
  parked question holds its session and compute slot (like a permission request) or releases them
  and resumes later is this item's design call. Not blocked: MOD-11 is done (`docs/decisions/mod/mod-11.md`), so the MCP
  server exists.
  **From MOD-55 (`docs/decisions/mod/mod-55.md`):** milestone 4 (Skills) also takes the editors' `Ctrl+G` (ask an
  agent, `ui/tabs/skills/agent_help.rs`) and the `AgentHelp` panel's keys (`Enter`/`y` accept, `Esc`/`n` discard),
  hard-coded today like `Ctrl+S`/`Ctrl+E`.

- [ ] **MOD-88 - Flow-graph mode for the Backlog item graph (rataflow)** (from MOD-14 and MOD-28,
  `docs/decisions/mod/mod-14.md`, `docs/decisions/mod/mod-28.md`). `R-TUI-1`. The Backlog detail's Graph
  sub-tab (`ui/tabs/backlog/detail/graph.rs`, MOD-14) shows an item's link neighbourhood only as a
  `cargo tree`-style tree. Add a second mode that draws the same `LinkGraph` as a `rataflow` node-and-edge
  graph: one node per item, one edge per link (depends, origin, blocked-on), the arrow direction read the way the
  tree reads it. It reuses MOD-28's `ExecutionGraph` plumbing (pan, zoom, mouse, fit through `ui::cells`), with a
  key toggle like the Runs pane's `v`. Open in the plan: node content and size, layout of cycles and items with
  several parents, re-rooting from a node (the tree's `Enter` emits `Action::Reveal`), and how the hop depth
  (`+`/`-`) sits beside the flow's zoom. Not blocked; its toggle is a catalogue action (MOD-67, `docs/decisions/mod/mod-67.md`).

- [ ] **MOD-94 - Answer a relayed permission request from outside the TUI** (from ANA-29,
  `docs/ANA-29.md` §5.1 C3). `R-TUI-4`, `R-TUI-6`, `R-HIS-1`, `R-ID-6`. MOD-42's executor waits for
  "any store client" to answer a `step_permission` row by compare-and-set
  (`docs/decisions/mod/mod-42.md:31-32`), but only the TUI answers. Add `htui permission list
  --json` and `htui permission answer <request-id> <option>`, which run the same compare-and-set,
  name the request they answer, refuse a request already answered or whose session is gone, and
  record the answerer as a person on this box (ANA-27 T2). MOD-93's pane (`docs/decisions/mod/mod-93.md`) then gains one button per
  option, pressed by a person; no mod or agent answers on its own. **Blocked on a maintainer
  decision** (`docs/ANA-29.md` §12 Q2); MOD-92 is done (`docs/htui-cli.md`, its envelope and exit
  codes are the model for `permission list --json`); MOD-93 is done (`docs/decisions/mod/mod-93.md`), so the pane the buttons go in exists.

### Deferred backlog

- [ ] **CLEAN-12 - `.claude/` hygiene found by ANA-29** (from ANA-29, `docs/ANA-29.md` §4.1).
  Remove the committed `.claude/.headroom_wrap_settings.lock` and ignore it. Decide whether
  `.claude/skills/generated/` is committed or tool-owned: it is committed, yet `scripts/hr:692`
  lists it in `TOOL_EXCLUDES` as untracked. Correct `.claude/skills/handoff-run/references/lifecycle.md:98-99` and
  `install-workflow-hooks.sh:22-23` to what this repo has now that TOOL-9 landed (the commit hook via
  `core.hooksPath`, the `--no-verify` block from TOOL-8). No behaviour change. Nothing here is
  blocked: TOOL-8 is done (G6 in `docs/htui-dev.md`, `docs/decisions/tool/tool-8.md`). TOOL-9 already added the
  `core.hooksPath` paragraph to `.claude/skills/handoff-run/references/lifecycle.md` and the installer header, so check those first.
- [ ] **MOD-3 - Diff tab + code explorer.** `R-LATER-1`. Later tier; needs its own ANA first.
- [ ] **MOD-5 - Issue tracker mirror.** `R-LATER-2`. `IssueSync` trait, OneDev first, downstream
  only. Later tier; needs its own ANA first.

- [ ] **MOD-8 - Legacy markdown import.** `R-LATER-3`. Map old prefixes to kinds per project,
  preserve keys, build links. Later tier; MOD-6 landed (`docs/decisions/mod/mod-6.md`, importer
  mint variant per ANA-9 §7.1 still to write). Widened by ANA-11 (`docs/ANA-11.md` §6 phase 3): also import
  `docs/REQUIREMENTS.md` into the requirement tables MOD-38 added (`docs/decisions/mod/mod-38.md`), map each `DECISIONS.md` status to
  `item.resolution` (`shipped` → `done`), write-ups to `summary` documents and each analysis doc to
  the `verdict` document, and scan body `R-` IDs once into `addresses` citations.

### Tooling findings

- [ ] **TOOL-10 - hr sandbox: stop a run from planting a mod that runs on the host** (from
  ANA-29, `docs/ANA-29.md` §3.3). `~/.claude` is shared read-write with every run
  (`docker/hr/compose.hr.yaml:99-114`), and `docs/hr-sandbox.md` already lists "add hooks or
  settings that execute in your next host Claude session". Mods widen that: a plugin installed,
  or a `~/.claude/skills/<x>/.claude-plugin` folder written (it loads as `@skills-dir`), inside a
  sandbox runs **in process** in every later host session, unsandboxed. `~/.claude/dev-mods/` is
  a lesser risk: a mod there loads only in the session whose ID names its folder, after a
  hot-reload approval and in a trusted workspace (`mods/create`), so it reaches the host only if
  that host session is resumed; verify this before deciding whether to mount it read-only. Mount
  `~/.claude/plugins` (except `plugins/data` and `plugins/store`) and `~/.claude/skills`
  read-only, or masked, in runs; check that plugin auto-update and `$.store` degrade quietly; and
  extend the `docs/hr-sandbox.md` table. Not blocked.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 3 (ANA-25 learned weights, ANA-28 heavy_build routing, ANA-30 utility models) |
| MOD-N   | 18 (MOD-16 Windows verification, MOD-27 swarm, MOD-36 weighted agent assignment, MOD-75 agent question tool, MOD-43 remote dispatch, MOD-44 container env, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-81 terminal widths, MOD-82 shared pane chrome, MOD-83 display labels and errors, MOD-85 remaining accent uses, MOD-88 item-graph flow mode, MOD-94 permission answers outside the TUI, deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 1 (CLEAN-12 `.claude/` hygiene) |
| TOOL-N  | 1 (TOOL-10 sandbox mod hardening) |
