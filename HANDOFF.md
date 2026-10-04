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

**Current status (2026-10-04):** **MOD-76 is done** (`docs/decisions/mod/mod-76.md`): MOD-37's four
carried risks. R-32 and R-53 closed as accepted with no code. The Runs pane shortens `agent/model`
before it cuts it (a trailing `-YYYYMMDD` and a leading `{agent}-` go, so
`claude/claude-sonnet-4-5-20250929` reads `claude/sonnet-4-5`). A `command_limits` edit now reaches
the verifier with no restart: the TUI at its next walking `StartRun`, `htui worker` at its next sweep
with no run walking, never under a live walk. No migration.
Before it, **MOD-11 is done** (`docs/decisions/mod/mod-11.md`): every session
htui launches (engine steps in the TUI and in `htui worker`, fresh and promoted chats) gets the `htui`
MCP server: `box_profile`, `document_write`, `note_add`, `item_status` (a note, never a transition),
`item_link`, `search_concepts`, `command_run`, and `permission_prompt` for `claude-cli`. Scope comes
from a per-session token and every item write is fenced. Production judges and `approve`/`accept`
now work on agent-written documents. `command_run` queues builds and tests per `(box, class)`.
`claude-cli` permission prompts reach the Runs pane and the chat. New crate `htui-mcp`, migration
`0015_command_queue`, user guide `docs/htui-mcp.md`. Follow-ups MOD-77, MOD-78, MOD-79 and ANA-28.
Live coordinates after MOD-11: migrations run through `0015_command_queue` (`command_run.claimed_by`,
`heartbeat_at`), so **the next migration is `0016`** (cache: `0005`). MOD-37's `0014_run_step_opening` and
MOD-11's queue migration both landed as `0014`; MOD-11's was renumbered at the merge. Pins: store
conformance `CASES` 146, `READ_CASES` 15, `htui-orch` `CASES` 100.
Before it, **MOD-37 is done** (`docs/decisions/mod/mod-37.md`): orchestrator
hardening closed in five milestones (run state, store and engine correctness, git cost, deadline and
sessions, ACP resume). A promoted ACP step now resumes its own session (`session/resume`, else
`session/load` with the replay discarded), and a promotion resumes the step's latest banner. A
resume that fails is reported and opens the handoff prompt in the same bind. `run_step.opening`
(migration 0014) records which way the chat opened, and the Runs pane and the Chat tab say "context
not carried; handoff prompt only". The four re-deferred risks are MOD-76.
Before it, **MOD-73 is done** (`docs/decisions/mod/mod-73.md`): a hand-written
document version newer than a step's output is what the next step reads (ANA-2 §4.2 amended).

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
- [ ] **MOD-10 - Secret provider** (from ANA-7). `R-SEC-1..4`, `R-TUI-8`. `SecretProvider` trait,
  Infisical implementation, environment injection at run start, scrubber with exact-match and
  pattern masks, fail-closed persistence gate, Settings tab secret provider section. **No longer
  blocked** — MOD-2 is done (`docs/decisions/mod/mod-2.md`) and shipped the `Scrubber` seam with the
  fail-closed `MinimalScrubber` this item replaces *behind an unchanged trait*. Its call sites are
  already fail-closed on every digested byte (MOD-2 D100 as corrected by that milestone's CRITICAL);
  the one call site that was **missing** was **MOD-32**'s, now done
  (`docs/decisions/mod/mod-32.md`). **MOD-4 wired no secrets** (done,
  `docs/decisions/mod/mod-4.md`, plan D176): `htui-orch`'s `drive_once` builds every graph
  `SessionSpec` with an empty `env`, and its comment names this item as the one that fills it.
  **Relates to ANA-16** (`docs/ANA-16.md` §8, §9): open question on where secrets resolve, on the
  worker or on the server; MOD-48 owns it, with `R-SEC-2` amended only if the server resolves them.
  **Verifier boundary (MOD-62, 2026-09-29):** once this item fills the agent's `SessionSpec.env`
  with resolved secrets, that map must never reach `htui-orch/src/verify.rs`. The verifier keeps
  htui's own process environment (plan D30) and scrubs its output. A test pinning that the verify
  child does not see a resolved secret belongs to this item.
  **Run engine scrubber (MOD-61, folded in 2026-09-29):** the run engine's `MinimalScrubber` is
  built with an empty secret list (`crates/htui-worker/src/runtime.rs:929`, since MOD-41), so `scrub`'s masking half
  is inert on the run path and a `trim_record` string equal to a secret would be stored verbatim.
  When this item resolves a run's secrets, build that scrubber from the **same** map that fills the
  step's `SessionSpec.env` (`htui-orch` `drive_once`, `engine.rs:5987`), so the two cannot drift;
  the chat path already does this from `spec.env` (`agent_worker.rs:3961`) and needs the same map.
  The verifier's scrubber (`crates/htui-worker/src/runtime.rs:437`) stays pattern-only: handing it the map would put
  the resolved secrets inside `verify.rs`. A test pinning that a record string equal to a resolved
  secret is stored as `[REDACTED]` belongs to this item.
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

- [ ] **MOD-70 - Follow-up command rows for engine steps** (from MOD-42, PRD Q9;
  `docs/decisions/mod/mod-42.md`). `R-AGT-1`, `R-HIS-1`. MOD-42's `run_command` table carries only
  `kind = 'cancel'`; a follow-up typed in any TUI for a step an engine walks (in process or on a
  worker, on any box) would be a second kind, applied by the lease holder like a cancel. Open first:
  the engine has no follow-up verb and ANA-2 no state that accepts one (a walk's session ends at
  `done` before the step parks, `docs/ANA-2.md:1235-1238`), and the text is typed on one box but must
  be scrubbed on the executing box (`R-SEC-3`, `R-ID-7`). Not blocked.
- [ ] **MOD-77 - Pg step writers lock the step before the run (`park_step` deadlock order)** (from
  MOD-11, `docs/decisions/mod/mod-11.md`). `R-HIS-1`, `R-ORCH-11`. `park_step` locks
  `FOR UPDATE OF s, r` (step, then run), and MOD-11's fenced writes follow it through `step_scope`
  (`FOR SHARE OF s, r`, step → run → item). The older step writers go the other way. `append_events`,
  `set_step_usage`, `set_step_prompt`, `finish_step`, `pass_step`, `upsert_step_tree`,
  `record_commits`, and the relay's `open_permission`, fence through `step_fence` (`FOR SHARE OF r`, or
  an `EXISTS … FOR SHARE`). They lock the run first and only then touch `run_step`. Not reachable
  through MOD-11 (verified). The worst case is a detected `40P01` when a stale walk races the walk that
  adopted its run. Fix: give `step_fence` the `FOR SHARE OF s, r` shape; writers that update
  `run_step` take `FOR NO KEY UPDATE OF s FOR SHARE OF r`; regenerate `.sqlx`. MOD-76
  (done, `docs/decisions/mod/mod-76.md`) did not touch it. Not blocked.
- [ ] **MOD-78 - `command_run` lifecycle: lease check and cancel on session end** (from MOD-11,
  `docs/decisions/mod/mod-11.md`). `R-MCP-1`, `R-MCP-3`. `command_run` is not fenced (MOD-11's I-3
  reads "every item write"). A session whose walk lost its lease can queue and run commands until the
  walk notices at its next renewal (`docs/htui-mcp.md`, "Scope"). Add a lock-free `lease_owner` check
  before `enqueue_command` and on each heartbeat, and kill the child on a loss. Review L5: dropping
  the lease only sets the session's `ended`. Add a cancellation signal on `Session` (a `watch` or a
  `CancellationToken`) that `Served::call` selects against, so an in-flight call ends with its
  session. Not blocked.
- [ ] **MOD-79 - MCP token off claude's argv** (from MOD-11, `docs/decisions/mod/mod-11.md`,
  blueprint E-1 and review L2). `R-MCP-1`, `R-NF-1`. `claude-cli` gets the per-session
  `HTUI_MCP_TOKEN` inline in `--mcp-config=<json>`, so any local user can read it from
  `/proc/<pid>/cmdline`. It is useless without the `0700` socket directory, but it should not be
  there. Pass a `0600` config file in the session's private directory instead (the CLI accepts a
  path), and redact `--mcp-config` arguments in `ResolvedLaunch`'s `Debug`. A Windows equivalent for
  the file's ACL is part of the item. Not blocked.
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
  not the worker, resolves project secrets. Blocked on MOD-47, MOD-10.

- [ ] **MOD-55 - Ask an agent for help while editing a template or skill** (from MOD-9, PRD gate
  2026-09-25; `docs/decisions/mod/mod-9.md`). `R-SKL-3`, `R-PRM-4`. MOD-9's editor (Skills tab, `TextArea` and `$EDITOR`,
  `.claude/prds/mod-9-skill-library-templates.prd.md` D2) gains an action that sends the body being
  edited, the role's placeholder table and the maintainer's request to a configured agent and offers
  the reply as a proposed edit, shown as a diff and saved only through the same `parse` gate. Open:
  which agent and model answer (the chat driver or a one-shot CLI call), whether the exchange is
  recorded, and how secrets in a body are scrubbed before they leave. Not blocked: MOD-9 is done (`docs/decisions/mod/mod-9.md`); the editor this extends landed with
  its milestone 1 and the Skills view with milestone 3.

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
- [ ] **MOD-69 - Waiting-on-you list across items** (from ANA-27, `docs/ANA-27.md` §5.1 T8,
  `docs/decisions/ana/ana-27.md`). `R-TUI-1`, `R-TUI-4`, `R-ORCH-2`, `R-ORCH-4`, `R-NF-3`. The Runs
  pane shows what waits on a person only for the selected item, and the top bar counts active runs,
  so with several items running a parked gate, a fan-out awaiting selection, a judge park or a
  blocked run is found by visiting items one by one. Add an overlay, opened from every screen, that
  lists every run in the active workspace waiting on a person, one row per reason (gate, selection,
  judge failure, unblock, and, once MOD-42 lands, each open permission request with its tool), read
  by one store query. `Enter` opens the item's Runs pane on that step, and the top bar gains the
  count. The list is derived at read time and keeps no state of its own; MOD-12's planned escalation
  list can reuse the query, and the opening key is a MOD-67 action. **Open question for the
  maintainer:** it needs the proposed `R-TUI-11` and the matching `R-TUI-1` top-bar line (ANA-27
  §7), not yet applied. Not blocked; the permission rows are added after MOD-42.

### Deferred backlog

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
| MOD-N   | 21 (MOD-10 secrets, MOD-12 auto mode, MOD-16 Windows verification, MOD-27 swarm, MOD-36 weighted agent assignment, MOD-70 engine follow-up, MOD-77 step-before-run lock order, MOD-78 command_run lifecycle, MOD-79 MCP token off argv, MOD-43 remote dispatch, MOD-44 container env, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-55 agent help in the editor, MOD-57 embedded editor, MOD-67 configurable hotkeys, MOD-69 waiting-on-you list, deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0 |
| TOOL-N  | 0 |
