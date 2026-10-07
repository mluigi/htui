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

**Current status (2026-10-07):** **MOD-86 and MOD-87 are done** (`docs/decisions/mod/mod-86.md`,
`docs/decisions/mod/mod-87.md`), run together on `hr/MOD-86`. Every chat text (opening, promoted opening, resume
handoff, follow-up, and a help opening on a provider project) is masked before a driver, row or tab frame sees it;
a refusal is `not sent: the <section> matches the <rule> rule`, a refused follow-up leaves the chat live. A chat now
serves commands while it streams: `Esc Esc` cuts the turn (`cancelled`), mid-turn sends are deferred, and every
queued command is answered once after the run is closed (D206 kept). No migration, no UI change.
Before them, **MOD-10 is done** (`docs/decisions/mod/mod-10.md`): a project's secrets
come from a self-hosted Infisical, reach the agent only through `SessionSpec.env`, and are masked by the same map in what a run stores.
M1 hardened the scrubber (whole-token rules, exact-match masks, a typed `ScrubRefused` run failure); M2 added the
`htui-secrets` crate (Universal Auth, a 401 latch, the machine identity in the executing box's keyring); M3 resolves
once per walk into one `RunSecrets` that is both the agent's env and the walk's scrubber, and refuses the run with
`secrets_refused: <cause>`; M4 added Settings > Secrets (URL, identity, health, per-project scope through
`ProjectPatch.secret`; one conformance case, no migration). It filed **MOD-89** (`.env` in a worktree), **MOD-90**
(`htui worker` keeps a refused login) and **CLEAN-8** (M4 review residuals).
Live coordinates after MOD-70 and MOD-12 M1: migrations run through `0017_follow_up` (`run_command.run_step_id`,
`text`; `follow_up_window`; MOD-12's `0016_auto_queue`: `queue_entry`, `queue_batch`, `run.batch_id`), so **the next
migration is `0018`** (cache: `0005`). MOD-37's `0014_run_step_opening` and MOD-11's queue migration both landed as
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
- [ ] **MOD-12 - Auto mode queue runner** (from ANA-2). `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard
  gates, `R-AGT-7..8` caps, `R-TUI-8`. Ready-item selection, capability filter, concurrency with
  overlap rule, queue overlay, escalation, Settings tab caps and scheduler window section. The
  `queue` action of `R-TUI-2`. Target box stored, local execution only. Design concluded in
  `docs/ANA-2.md` (§4.10, §9): `ready_items` per ANA-9 §7.4 in full, the same `claim_run`
  admission as MOD-4, batch caps as `SUM(run_step.usage)`, escalations in the queue overlay, the
  `scheduler_window` key stored but not enforced; a graph with a `gate_hard` phase is never fully
  unattended, so `FIX`/`CLEAN`/`TOOL` are the first targets. ANA-10's SQLite rewrite of
  `ready_items` (`docs/ANA-10.md` §4.8) is withdrawn with its verdict (MOD-25,
  `docs/decisions/mod/mod-25.md`): the SQLite cache is a read mirror, so `ready_items` is written for
  `PgStore` and `MemStore` only.
  **Not blocked**: MOD-4 is done (`docs/decisions/mod/mod-4.md`); `claim_run`'s admission, the
  overlap predicate, the lease, the sweep and `run_worker.rs` are there to reuse. **MOD-7 is done**
  (`docs/decisions/mod/mod-7.md`) and supplies what the capability filter needs: real
  `probed_tags`/`declared_tags`, `repo_box_path` rows by inference, and the `R-ORCH-10` refusal at
  enqueue and at claim (`Claim::MissingTags`), so auto mode reuses them rather than adding a check.
  **Relates to ANA-16** (`docs/ANA-16.md` §8): target-box selection in auto mode is MOD-43's, which
  depends on this item for its auto-mode half.
  **Phase 1 landed (`48ff0811`..`d19767df`, 2026-10-07):** unattended runs. Migration
  `0016_auto_queue` (`queue_entry`, one item on one box; `queue_batch`, at most one open per box;
  `run.batch_id`); `ready_items` in `priority DESC, created_at, id` order; the auto-mode gate
  downgrade at snapshot time (`effective_gate`); the queue store surface on `MemStore`/`PgStore`/
  `Backend` and the runner's `WorkerHost` methods; `Engine::enqueue_in_batch`; admission, prune and
  drain in `sweep_once` (only the box's executing process, slots counted on running runs, a cancel
  sticks for its batch, a pause cancels the batch's still-queued runs, the drain closes only the batch
  it checked) with walk-end and resume wake-ups; Backlog `Q` (queue/dequeue) and `P` (resume/pause).
  ANA-2 criteria 22, 23, 24 (by construction, resume pinned), 26 and 27's pause half have tests.
  PRD `.claude/prds/mod-12-auto-mode-queue-runner.prd.md`, plan
  `.claude/plans/mod-12-m1-unattended-runs.plan.md`.
  **Phase 2 landed (`961e4f40`..`8b3aed9f`, 2026-10-07):** spend guard.
  - The batch spend is the sum of its runs' integer `cost_micros`, never stored.
  - Each run is held to its own project's `per_token_cap_batch` against the whole batch's spend, at
    three points:
    - the runner's admission (a stopped batch stays open);
    - the candidate walk (`BatchCapReached` / `BatchBudget`, the item goes `blocked`);
    - the session allowance `min(run cap - run spend, batch cap - batch spend)`, from the snapshot.
      This also fixes the run cap being applied per step.
  - Settings > Queue edits the two caps (in USD), `min_budget_for_new_attempt`, the app-wide and
    per-box `max_concurrent_items`, and the stored-only `scheduler_window`.
  - The PRD overshoot metric is amended to one attempt per open session. Review R1 is applied
    (`docs/ANA-2.md` §4.10 and §5.4 as-built notes); its residuals are CLEAN-9.
  - Plan `.claude/plans/mod-12-m2-spend-guard.plan.md`.

  Remaining: M3 queue overlay (escalations including a capped batch, reorder, off-box runs, the
  open-batch-on-`Q` question of M1 review L4).
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

- [ ] **MOD-43 - Remote dispatch in the TUI** (from ANA-16, §8 item 4). `R-ORCH-11`, `R-ORCH-12`,
  `R-TUI-1`, `R-NF-3`. Target box on run start and in auto mode; a non-local target stays `queued`
  until its worker claims it; the Runs view follows `session_event` by `seq` with `LISTEN`/`NOTIFY`
  hints and a poll backstop, and shows worker liveness. Blocked on MOD-12 for auto mode only
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

- [ ] **MOD-57 - Run the external editor inside the TUI pane** (from MOD-9, `docs/decisions/mod/mod-9.md`;
  merge of MOD-7 milestone 2, maintainer-decided 2026-09-26). `R-TUI-1`, `R-TUI-7`, `R-NF-1`. Today `E`/`Ctrl+E`
  in the Templates editor (and any later user of the shared `ui::TextArea`) suspends the whole TUI
  and runs `$VISUAL`/`$EDITOR` full-screen (`crates/htui/src/editor.rs`). Offer a toggle that runs
  the editor inside the editing pane instead, with the rest of the TUI still drawn: a pseudo-terminal
  (`portable-pty`, ConPTY on Windows) feeding a VT parser (`vt100`) drawn by a ratatui widget
  (`tui-term`), keys forwarded to the child with one reserved chord to leave, resize forwarded,
  and the file read back through the same `parse` gate on exit. Works for any editor, nvim
  included. The alternative, nvim's `--embed` RPC UI (`nvim-rs`), is nvim-only and was not
  preferred. The three crates are not in `Cargo.lock`, so the plan owns that dependency decision.
  Not blocked. Its in-app widget draws through `ui::cells` (MOD-54, done:
  `docs/decisions/mod/mod-54.md`), and every other row fits through the same module (MOD-60,
  done: `docs/decisions/mod/mod-60.md`).
  **Keys (ANA-26, `R-TUI-10`):** the reserved leave chord is an action in MOD-67's catalogue, never a
  hard-coded key, and this pane is the one place `ctrl-c` is forwarded instead of quitting; the
  leave action must stay bound (`docs/ANA-26.md` §6.4).

- [ ] **MOD-67 - Configurable hotkeys: named actions, `keys.toml`, generated hints** (from ANA-26,
  `docs/ANA-26.md`, `docs/decisions/ana/ana-26.md`). `R-TUI-10`, `R-TUI-1`, `R-STO-1`, `R-NF-1`.
  Every key outside text entry becomes a named action in one compiled-in catalogue (about 100
  actions, named once per meaning: shared `list`/`pane`/`confirm`/`form`/`common` contexts, a
  narrower context overriding one for a single view), dispatched through composed context stacks
  whose resolver returns ordered candidates so a view can decline and fall through. A user
  overrides only what they change in `<config_root>/keys.toml` (per OS user and machine, never the
  store); an invalid file refuses to start with every error as `path:line`, exit status 2, kept out
  of error capture; `--keys`, `--default-keys`, `--print-keys`. `ctrl-c` is checked first and
  always quits (MOD-57's pane excepted). Every hint, key-naming message and the `?` box is
  generated; `?`/`f1` open help from every screen. Fixes two routing defects found by ANA-26:
  `ctrl-c` opens the clear confirm in Settings > Connection and Qdrant browse, and the Qdrant editor
  passes `Tab` to the tab bar. Six milestones (ANA-26 §8): M1 catalogue and resolver, M2 the file,
  M3 Settings and overlays, M4 Skills and Requirements, M5 Backlog and Chat, M6 close-out; M3-M5
  are independent but share snapshots. Not blocked.
  **TUI design review (2026-10-04, https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 1,
  3):** `?` lists only the global keys and the tab's action bindings, missing j/k, h/l, J/K and every
  detail-pane key, so the generated sheet must list every key of the focused tab and pane. Hints sit
  in four places and spell one action four ways (`j/k move`/`select`/`rows`, `J/K move`, `Up/Dn
  move`), and Backlog has no hint row; generated hints go in one fixed row for every tab. MOD-81
  and MOD-82 come after this item.
  **Phase M1 landed (`c3b3f7bd`..`add7fb39`, 2026-10-06):** the `keys/` module: `KeyChord` with a
  strict parser, the 41-action catalogue (`global`, `overlay`, shared contexts), the `Keys`
  resolver over context stacks, and generated status line and `?` box. `ctrl-c` quits first from
  every screen (fixes Connection/Qdrant browse), and `?`/`f1` open help over overlays. No snapshot
  changed. Deferred: the overlay-aware and capture-filtered status line to M3, and the Unix
  ctrl-collision rejects to M2 (`.claude/plans/mod-67-m1-catalogue-resolver.plan.md`, "Review gate").
  **Phase M2 landed (`d4be4861`..`97cad2aa`, 2026-10-07):** `<config_root>/keys.toml` (or
  `--keys PATH`) is loaded with line spans, checked per context and per declared stack, and either
  installed or refused with every error as `keys.toml:LINE:`, exit status 2, outside Sentry;
  `--default-keys`, `--print-keys` (round-trips). The Unix legacy ctrl rejects landed (some
  `cfg(unix)` only). Global and overlay keys are configurable end to end; a key a view or overlay
  still matches itself wins until M3-M5 (README "Changing keys"). The narrower-context override of a
  shared verb and the shadowing allow-list kind land with M3's first view context
  (`.claude/plans/mod-67-m2-keys-file.plan.md`, "Review gate").
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
  alone, which is bold-only under `NO_COLOR`. Style only. Not blocked; touches views that MOD-67,
  MOD-81 and MOD-82 also rewrite, so run it after them or beside them with care.
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
  `ui::cells` (MOD-60). After MOD-67, which rewrites the same views' hint rows and snapshots.
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
  After MOD-67.
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
  (`+`/`-`) sits beside the flow's zoom. Not blocked. After MOD-67, which owns the key catalogue.

- [ ] **MOD-89 - Warn or refuse when a run's worktree holds a `.env` file** (from MOD-10,
  `docs/decisions/mod/mod-10.md`; PRD open question decided 2026-10-03: "follow-up item, filed at close-out").
  `R-SEC-2`, `R-ID-7`. MOD-10 lets a project drop its `.env` by resolving secrets from Infisical into the agent's
  `SessionSpec.env` only, but nothing stops a run on a tree that still holds one: the agent can read it, and from
  there it reaches the LLM context and the stored transcript. Detect a `.env` (and similar files) in a run's
  worktree at run start and warn or refuse. Open: which file names count,
  warn vs refuse (and whether per project), whether a provider-less project is checked too, and where the warning
  shows. Out of MOD-10's MVP by PRD scope. Not blocked.

- [ ] **MOD-90 - Clear a refused Infisical login in `htui worker` when the same identity is re-entered** (from
  MOD-10, `docs/decisions/mod/mod-10.md`, blueprint A-4). `R-SEC-4`, `R-TUI-8`. A 401 latches the process's one
  `KeyringInfisical` provider. MOD-10 M4's keyring-write generation (`crates/htui/src/secrets.rs`) makes any
  Settings > Secrets write, even of the same identity, rebuild the TUI's provider. The generation is per process,
  so a separate `htui worker` rebuilds only when the stored URL, client ID or client secret changed, and keeps its
  latch after the same identity is re-entered until it is restarted (documented in `docs/htui-secrets.md`, "Logins,
  tokens and lockout safety"). Give the worker a way to see the write (a keyring-side marker, a store row, or a
  command) without retrying a refused login on its own. Not blocked.

### Deferred backlog

- [ ] **CLEAN-8 - MOD-10 M4 review residuals** (from MOD-10, `docs/decisions/mod/mod-10.md`). `R-SEC-3`, `R-TUI-8`.
  - **L-7** (refuted as a finding, kept as hardening): `read_body` in `crates/htui-secrets/src/infisical.rs`
    accumulates the response body, values included, in a growing `Vec<u8>` that is never wiped; read into a
    pre-sized `Zeroizing` buffer instead.
  - The 7 rust-reviewer NITs: `ProjectPatch`'s `Option<Option<SecretScope>>` loses `Some(None)` in a JSON round
    trip; `SecretScope::to_column` clones; `secrets_settings::serve` refuses a blank half by trimmed text but
    stores it untrimmed; `SecretsSection::rows()` allocates per call; `fresh_tree` repeats the re-read/CAS;
    `HalfStored` vs `Unreadable` are told apart by a message prefix (blueprint A-7) instead of a typed error; long
    `on_reply`/`on_scope_written`/`lines` functions.
  - A `--demo` guard on the Qdrant keyring arms: Settings > Qdrant still reads and writes the keyring in a demo
    session (`docs/htui-secrets.md`, Settings).
  Not blocked.
- [ ] **CLEAN-9 - MOD-12 M2 review residuals** (from MOD-12 M2, plan
  `.claude/plans/mod-12-m2-spend-guard.plan.md`). `R-AGT-7`, `R-TUI-8`.
  - **Recorder error-row wording.** Since M2's D6 the recorder's `RunCap.micros` is the session's
    remaining allowance, the smaller of run cap minus run spend and batch cap minus batch spend. The
    `cap_exceeded` row in `crates/htui-agent/src/record.rs` still says
    `project.settings.per_token_cap_run = <n> micros`, which is wrong whenever an earlier step spent
    anything or the batch term binds. The row shows in the transcript and may be carried forward as
    cached transcript. The item note and `run.failure` are correct. Fix: give `RunCap` a basis (run
    remainder, or batch `<id>` remainder) and word the row as "session allowance reached".
  - **One malformed project key uncaps the engine** (plan D11, review M3). `graph.rs` and
    `Engine::project_settings` decode `project.settings` with `unwrap_or_default()`, so one bad
    unrelated key (`"keep_raw_events": "yes"`) drops both caps from the snapshot and the recorder. The
    runner (`ProjectCaps::from_settings`) still admits the run. Read the caps through
    `ProjectCaps::from_settings` at snapshot time, and fail or note a malformed blob instead of
    defaulting it.
  - **Non-object `project.settings` gaps that only raw SQL can reach.** The Pg clear statements
    (`pg/write.rs`, `settings - key`) apply on an array blob and raise 22023 on a scalar where Mem
    refuses. `ProjectCaps::from_settings` reads a non-object blob as "no caps".
  - **Test hardening.** `a_batch_overshoots_its_cap_by_at_most_one_attempt_pg` should also assert that
    the batch is still open before the second sweep. The once-per-batch log levels (`info`, then
    `debug`) are not pinned.
  Not blocked.
- [ ] **CLEAN-10 - `cargo doc` fails on private intra-doc links** (found at MOD-12 M2's gate).
  `cargo doc --workspace --no-deps --all-features` stops with 17 errors in `htui-core` and
  `htui-store`. They are public docs linking to private items (`close_batch` →
  `State::finish_run`/`finish_run_on`, `upsert_agent`/`set_requirement_spec` → `cas_miss`,
  `write_step_document`/`propose_link`/`add_step_note`/`withdraw_link` → `step_scope`, …) and one
  unresolved `ensure_model`. All of them predate M2. Point those links at public items, or turn them
  into plain code spans. Not blocked.
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

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 2 (ANA-25 learned weights, ANA-28 heavy_build routing) |
| MOD-N   | 22 (MOD-12 auto mode, MOD-16 Windows verification, MOD-27 swarm, MOD-36 weighted agent assignment, MOD-75 agent question tool, MOD-43 remote dispatch, MOD-44 container env, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-57 embedded editor, MOD-67 configurable hotkeys, MOD-81 terminal widths, MOD-82 shared pane chrome, MOD-83 display labels and errors, MOD-85 remaining accent uses, MOD-88 item-graph flow mode, MOD-89 `.env` in a worktree, MOD-90 worker login latch, deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 3 (CLEAN-8 MOD-10 M4 review residuals, CLEAN-9 MOD-12 M2 review residuals, CLEAN-10 `cargo doc` private links) |
| TOOL-N  | 0 |
