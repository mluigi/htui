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

**Current status (2026-10-15):** a **requirements review** with the maintainer
(`docs/requirements-review-2026-10-15.md`) went through every requirement and amended most of
`docs/REQUIREMENTS.md`. The main changes: `htui` serves any kind of project; fan-out now divides work
(best-of-N is later, `R-LATER-13`); a coordinator per project and per workspace (`R-ORCH-14`); key
bindings move to Postgres; `R-LATER-3` and `R-LATER-7` are promoted to must; macOS is best effort. Filed
**ANA-33..39** for the open designs and **MOD-107..114** for the settled work. **ANA-28 is concluded**
(`heavy_build` becomes a label, MOD-107). Before it, a **requirements audit** (2026-10-14) found 69 of 90 `must` requirements fully met; the
untracked gaps are filed as **MOD-99..106**, **ANA-31**, **ANA-32** and **TOOL-13**, and `R-NF-2`, `R-ORCH-6`,
`R-ORCH-12` and `R-LATER-6` were amended (`docs/REQUIREMENTS.md` header). **MOD-99** (hard-coded Sentry DSN in a
public repo) also owes key rotation and a history rewrite, after the live `hr` branches merge. **TOOL-14 is done**
(`docs/decisions/tool/tool-14.md`): the gate's new `postgres` step reads `skipped` when `HTUI_TEST_DATABASE_URL` is
unset, so a green no longer hides unexercised Postgres suites. Owed on the host: `gate.ps1 -SelfTest` on Windows.
Before it, **TOOL-8 is done** (`docs/decisions/tool/tool-8.md`): the `htui-dev` mod
(`.claude/skills/htui-dev/`, `docs/htui-dev.md`) refuses six kinds of costly call (M1), and keeps a status line in
interactive sessions (item and HANDOFF phase, merge/ahead/dirty, disk above 90%, Postgres not up, last gate) with a
toast when a gate finishes, plus turnless `/gate`, `/next` and `/validate` (M2); `scripts/dev-health.sh`/`.ps1` are
new. Owed on the host: a Windows run (`claude -p "/gate fmt"` returns at once, the `.ps1` commands), `dev-health`'s
docker path against the real `htui-postgres`, and the Postgres gate. Before it, **TOOL-10 is done** (`docs/decisions/tool/tool-10.md`): in an `hr` run
`~/.claude/plugins` and `skills` are read-only (`plugins/data` and `plugins/store` stay writable) and `dev-mods` is a
tmpfs, so a run cannot plant a mod that runs in the host's next session; `settings.json` is still writable, filed as
**TOOL-12**. Owed on the host: a real `scripts/hr up`, the Docker bats case. Before it, **ANA-30 is concluded** (`docs/ANA-30.md`, `docs/decisions/ana/ana-30.md`): no
`utility_model` setting now (every compliant cheap-model use is single-digit dollars a month and has no surface yet);
the $100 API credit is spent through a `per_token` registry row (data, no code), built into the first MOD that adds a
second drafting surface. Spawned **MOD-97** (an inherited `ANTHROPIC_API_KEY` reroutes subscription rows to API
billing) and **MOD-98** (opt-in `summary` phase); `R-ID-6` gained a one-line clarification and ANA-21 §5.7 a `haiku`
amendment. Owed on the host: checking which credit the $100 is (Console), and one dev-database query for stored ACP
`session_info_update` titles (`docs/ANA-30.md` §8 calls 2, 6). Before it, **CLEAN-12 is done** (`docs/decisions/clean/clean-12.md`): the committed
`.claude/.headroom_wrap_settings.lock` and the 20 `.claude/skills/generated/` Gortex files are untracked and
ignored (tool-owned, as `scripts/hr` already treated them); the installer headers name G6 as the `--no-verify`
block. Before it, **TOOL-9 is done** (`docs/decisions/tool/tool-9.md`): `scripts/gate.sh` and
`gate.ps1` run the one gate (fmt, both clippy runs, the suite single-threaded without fail-fast, an abort scan,
migration numbering, the workflow validator, the `htui-dev` mod when present), detached on request, result in
`target/gate/last.json`; the workspace pre-commit is committed in `scripts/githooks/` and the installers set
`core.hooksPath`; `DECISIONS.md` merges by union. Owed on the host: a Windows run of the `.ps1` files (`pwsh` was
not available in the sandbox) and the Postgres gate. Earlier completions (**CLEAN-11**, **MOD-67**, **MOD-96**, **CLEAN-10**, **MOD-93**, **MOD-89**, **MOD-95**, **TOOL-11**,
**MOD-91**, **MOD-92**) are in `DECISIONS.md`; their host Postgres gates on the merged tree, the Windows runs
and MOD-93's live install and `tsc` on the mod stay owed. Open from ANA-29: **MOD-94** behind a maintainer decision.
Live coordinates after MOD-96: migrations run through `0018_seed_permission_mode` (MOD-96: data only, pins
`acp.permission_mode` on the seeded `claude` row), before it `0017_follow_up` (`run_command.run_step_id`,
`text`; `follow_up_window`; MOD-12's `0016_auto_queue`: `queue_entry`, `queue_batch`, `run.batch_id`), so **the next
migration is `0019`** (cache: through `0005_run_step_opening`, next `0006`). MOD-37's `0014_run_step_opening` and MOD-11's queue migration both landed as
`0014`; MOD-11's was renumbered at the merge, and MOD-70's `0016` became `0017` at the merge with MOD-12. Pins: store
conformance `CASES` 164 (`htui-core/tests/mem_store.rs`), `READ_CASES` 15,
`htui-orch` `CASES` 109 (`htui-orch/tests/fake_conformance.rs`); Postgres tables 45.
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
  **Requirements review note (2026-10-15):** best-of-N fan-out is now the later-tier compete mode
  (`R-LATER-13`; `R-ORCH-7` means divided work), so this analysis and MOD-36 are later tier too.
- [ ] **ANA-31 - Can `agy` run with its own skills and context disabled** (from the 2026-10-14
  requirements audit). `R-ID-5`, `R-AGT-5`. Isolation is wired only for Claude: the ACP `claudeCode`
  meta (`crates/htui-agent/src/acp/mod.rs:1116`) and the `claude-cli` flags (`cli/mod.rs:258-272`,
  MOD-91, MOD-96). `agy` ignores the meta (doc at `acp/mod.rs:1110`), so its own skills and context
  files are never disabled, and a registry row's `settings.user_extensions` turns isolation off for
  any agent. Establish what `agy` (and an ACP agent in general) offers to disable its own instruction
  files, whether it can be expressed as row data (`R-AGT-5`), and whether `user_extensions` needs a
  guard or an `R-ID-5` amendment. **Requirements review note (2026-10-15):** `R-ID-5` now allows a
  row to opt out of isolation, provided Settings shows it (MOD-109 builds that part). What is left
  here is how `agy` is isolated.
- [ ] **ANA-32 - Measure warm start against the one-second budget** (from the 2026-10-14
  requirements audit). `R-STO-6`, `R-NF-3`. `R-STO-6` sets "under one second with a warm cache and a
  reachable Postgres on the reference workstation"; MOD-6 (`docs/decisions/mod/mod-6.md:127`) said to
  measure it and ANA-10 §12 criterion 13 keeps it as a recorded figure, but no figure, benchmark or
  item exists. Define the measurement (process start to first interactive frame, cache refresh
  excluded), record it on the reference workstation, and say whether a regression check belongs in
  the gate. **Requirements review note (2026-10-15):** `R-STO-6` now reads "first interactive frame
  from a warm cache under one second, with the Postgres connect allowed to finish later", and
  `R-NF-3` gains a key-press-to-screen bound (on the order of 50 ms under load) that this
  measurement sets.
- [ ] **ANA-33 - Step graphs v2 and divided fan-out** (from the 2026-10-15 requirements review,
  `docs/requirements-review-2026-10-15.md`). `R-ORCH-1`, `R-ORCH-3`, `R-ORCH-7`, `R-ORCH-8`,
  `R-MCP-2`, `R-ID-6`, `R-TUI-4`, `R-TUI-8`. Step graphs today are an ordered phase list per
  project, and fan-out runs N candidates on one prompt and keeps one result. The review changes both:
  graphs branch, run phases in parallel and loop, can be owned by a workspace, can run several items
  as subgraphs, and can be **planned at run time** by the agent that starts them, which then runs its
  plan directly within the project's bounds (phases, agents, caps, hard gates). Fan-out becomes
  **division of work**: a planner agent splits a step into parts, parallel sub-steps of the same run,
  each in its own tree, and an integrate step merges them before review (like Claude Code's
  ultracode). Loop-back on failure becomes a per-phase target. Settle: the graph model and its
  storage, and the snapshot a run freezes (`R-ORCH-11`); the planner's MCP tools; how a part maps
  onto `run_step` (`UNIQUE (run_id, position, attempt, fanout_index)`) and onto crash recovery
  (ANA-2 §4.9); how the integrate step relates to ANA-2 §4.6's reconciliation, where the orchestrator
  never resolves a conflict; the Settings graph editor; and what becomes of MOD-27 (swarm,
  `spawn_subagent`), MOD-36 and ANA-25 (now later tier, `R-LATER-13`). Its verdict ships in version
  one. Not blocked.
- [ ] **ANA-34 - Coordinator at project and workspace level** (from the 2026-10-15 requirements
  review). `R-ORCH-14`, `R-TUI-9`, `R-ENT-8`, `R-ID-6`, `R-ORCH-7`, `R-ENT-15`. The maintainer asked
  where merge and pull requests happen. Today the orchestrator merges a step's branch into the local
  primary checkout (ANA-2 §4.6), close-out only records hashes and writes the summary
  (`htui-orch/src/closeout.rs`), and nothing pushes or opens a PR. The review adds a **coordinator**,
  one per project and one per workspace. It only manages: it listens to what happens below it, talks
  with children and parent, integrates a child's work when the child's run completes (merge, push or
  PR, as the project configures), and runs close-out with a checklist (follow-up items, re-confirming
  suspect citations, choosing the resolution). Settle: whether a coordinator is an agent session, a
  deterministic process or both, and where `R-ID-6` draws the line; its lifetime and the box it runs
  on (the worker, `R-ORCH-12`); how it relates to ANA-33's planner and integrate step; the
  push/PR configuration and its credentials (`R-BOX-4`, `R-SEC-2`); and the final text of `R-TUI-9`.
  Not blocked; best read beside ANA-33.
- [ ] **ANA-35 - Entity model for any kind of project** (from the 2026-10-15 requirements review).
  `R-ID-1`, `R-ENT-1`, `R-ENT-2`, `R-ENT-3`, `R-ENT-4`, `R-ENT-5`, `R-ENT-6`, `R-ENT-8`, `R-ENT-9`,
  `R-ENT-12`, `R-ENT-14`, `R-ENT-16`, `R-BOX-4`, `R-STO-8`, `R-TUI-2`. `htui` now serves any kind of
  project, not only code. Settle the schema and migration for: a project **type** that seeds kinds and
  graphs (software, research, docs, ops; the non-software seeds); **repo-less** projects and repos
  that are **plain folders** (what refuses without git); workspaces with **no root path** that
  **own items, documents and requirements** (their key space and kinds, and `R-ENT-7`'s single
  counter); documents owned by a project or workspace; **attachments** in Postgres under a size cap;
  link kinds `parent`, `duplicates`, `implements` and links to documents or attachments; and the item
  status reduced to **open / blocked / closed**, with `queued`/`in_progress`/`awaiting_approval`/
  `failed` moving to the run. That last one touches the store, orchestrator, cache, Backlog filters,
  waiting-on-you and search's decisions filter. Also take over MOD-106's question of the primary repo.
  Not blocked.
- [ ] **ANA-36 - Further MCP tools for agents** (from the 2026-10-15 requirements review).
  `R-MCP-2`, `R-MCP-1`, `R-ID-6`. The review adds read tools, search, `item_create` drafts, the
  question tool (MOD-75) and the planner's tools (ANA-33), and asks for a survey of what else agents
  should get: what comparable harnesses expose to their agents, which tools would replace work an
  agent does badly today (from stored transcripts), and the scope and `R-ID-6` line for each. Not
  blocked.
- [ ] **ANA-37 - Is Qdrant worth requiring** (from the 2026-10-15 requirements review). `R-NF-2`,
  `R-STO-8`, `R-ENT-13`. The maintainer wants Qdrant required "if it is very beneficial". Today it is
  optional and fails open (MOD-34, MOD-50, MOD-68). Measure what semantic search adds once agents can
  search (MOD-112), triage and duplicate detection use it, and memory (ANA-38) sits on it. Weigh that
  against the cost of a second required daemon on every box (install, Windows, a headless worker) and
  against Postgres-only alternatives (`pgvector`, full-text search). Verdict: required or optional,
  and the `R-NF-2` text. Not blocked.
- [ ] **ANA-38 - Project and workspace memory** (from the 2026-10-15 requirements review; reopens
  `R-ENT-13`). `R-ENT-13`, `R-ID-6`, `R-STO-8`, `R-ORCH-14`. ANA-1's atomic facts with importance
  scores were dropped on 2026-09-03 as over-engineered ("summaries cover the need"). The maintainer
  reopened the question for the coordinator (ANA-34) and agents: memory of facts, decisions and
  gotchas that outlives one item. Settle who writes an entry (an agent, a person, close-out), how it
  stays fresh against the code it describes (commit pinning, expiry, confirmation), the `R-ID-6` line
  (a person accepts it, or not), how it reaches a prompt (`R-PRM-1`, `R-PRM-3`), and its
  **integration with Qdrant** (indexed, searched, deduplicated). Not blocked.
- [ ] **ANA-39 - Mandatory context tools for coding sessions** (from the 2026-10-15 requirements
  review; `R-LATER-7` promoted to must). `R-LATER-7`, `R-NF-2`, `R-AGT-10`, `R-ID-5`, `R-MCP-4`,
  `R-PRM-1`. Code-context tools (Gortex, Serena, Graphify, Headroom) become mandatory in coding
  sessions and replace the agents' plain search and read tools there. When one is missing, `htui`
  installs it or tells the user how. Settle: which tools, and which is the default; how each agent
  is denied grep/read in favour of them (permission rules, the agent's tool list, per row as data,
  `R-AGT-5`); what a coding session is (the project type, phase or kind); install and detection
  through R-AGT-10's tool managers; per-box indexing cost and freshness; and the excerpt-provider half
  of `R-PRM-1` (ANA-3 shipped none). Not blocked.

### Next features
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
  lists the tool. **Requirements review note (2026-10-15):** `R-ORCH-7` now means divided work
  (a planner, parallel parts, an integrate step), which overlaps the swarm. ANA-33 settles whether
  this item is subsumed, narrowed or kept; do not start before its verdict.
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
  server exists. `R-MCP-2` lists the tool since the 2026-10-15 requirements review, and MOD-114
  rings and notifies when a question parks a step.
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
- [ ] **MOD-97 - Strip inherited vendor API keys from subscription-billed agent spawns** (from
  ANA-30, `docs/ANA-30.md` §2.3, §8 call 7). `R-AGT-7`, `R-AGT-9`, `R-SEC-2`. `spawn_supervised`
  builds every agent child with `.envs(&launch.env)` and no `env_clear`
  (`crates/htui-agent/src/launch.rs:1184-1199`), and Claude Code always uses `ANTHROPIC_API_KEY` in
  `-p` mode when present. So an `htui` started where the key is exported (`hr attach --api-key` or
  `--shell`, `docs/hr-sandbox.md`), or a project secret of that name (`SessionSpec.env`,
  `driver.rs:283-284`), moves every `claude`/`claude-cli` run onto API billing while the row still
  says `subscription`: the quota latch watches the wrong thing and no per-token cap applies. Remove
  vendor auth variables (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`; which ones is the row's data,
  not a vendor name in code, `R-AGT-5`) from a `billing = subscription` row's spawn, or refuse the
  spawn naming the variable; settle which in the plan. Also in scope: Claude Code says `--bare` will
  become the `-p` default, which would stop the seeded `claude-cli` row (D92, no `--bare`) using the
  subscription login (`docs/ANA-30.md` §2.3); pin the behaviour with a test or an explicit flag. Not
  blocked.
- [ ] **MOD-98 - Opt-in `summary` phase on a cheap row to fill upstream summaries** (from ANA-30,
  `docs/ANA-30.md` §4 U5, §8 call 5). `R-PRM-1`, `R-ORCH-1`, `R-ID-6`. Upstream prompts read the
  latest `document.kind = 'summary'` (`pg/read.rs:588`), whose only writer is close-out's
  deterministic commit table (`htui-orch/src/closeout.rs:66-121`), so readers get no prose. Add an
  opt-in `summary` phase to the default step graphs (which ones, and whether "opt-in" is a seeded
  disabled phase or a documented recipe, is the plan's to settle): ordinary item work on a registry
  row, recorded as a `run_step`, writing the `summary` document. Sonnet 5.5 by default for quality
  (~$0.12 per item), Haiku 5.5 where cost matters. A model never goes inside close-out (ANA-2
  invariant 3) or prompt assembly (ANA-5 invariant 7). No `utility_model` setting (ANA-30 §7).
  Not blocked.
- [ ] **MOD-99 - Error-reporting endpoint from Settings; remove the hard-coded Sentry DSN** (from the
  2026-10-14 requirements audit). `R-NF-2`, `R-ID-7`, `R-ID-2`, `R-TUI-8`, `R-STO-1`. `body` in
  `crates/htui/src/main.rs:66-68` hard-codes a GlitchTip DSN and calls `sentry::init` on every start,
  with no opt-out, and `lib.rs`'s `sentry_tracing::layer()` forwards error events and breadcrumbs that
  never pass the `Scrubber` (`htui-core/src/scrub.rs`). The repository is public on GitHub. Make the
  endpoint a Settings value (Settings tab, entered and cleared like the Postgres DSN, `R-TUI-8`): unset
  means no client is built and nothing is sent; set means events and breadcrumbs pass the scrubber
  first, failing closed (`R-ID-7`). Where it is stored is the plan's to settle: Sentry starts before the
  store connects, so a store row needs a local copy or a late init, and a DSN with a key is a
  credential for `R-STO-1`. Also remove the host from `docs/htui-worker.md:493`. **Retroactive
  removal:** both DSNs ever committed (GlitchTip, and an older Bugsink one) are in commits `be47ad47`,
  `bf42e75f` and `53826fcb`; rotate both keys first (the public mirror is the leak, a rewrite cannot
  recall it), then rewrite history with `git filter-repo --replace-text` on every branch and force-push
  `origin` and `github`, **after** the live `hr` sandbox branches (TOOL-8, ANA-30, TOOL-10 at the
  audit) are merged or dropped. Maintainer decision 2026-10-14: file now, rewrite later. Not blocked.
  **Requirements review note (2026-10-15):** `R-ID-7` now names logs, telemetry and error reports
  explicitly. Check that `tracing` output written to a file passes the scrubber too, not only what
  reaches the endpoint.
- [ ] **MOD-100 - Run-history retention sweep per project** (from the 2026-10-14 requirements
  audit). `R-HIS-3`. `ProjectSettings.retention_days` (`htui-core/src/model/kind.rs:375`) is never
  read: MOD-2's PRD and MOD-6 (`docs/decisions/mod/mod-6.md:89`, plan D14) deferred the sweep and no
  item carried it. Add the setting to the Settings catalogue (`htui-core/src/prompt/settings.rs`) and
  a deterministic sweep (`R-ID-6`) that deletes run-step event rows older than the window, keeping
  run and step summaries and every document; `None` keeps everything (the default). Settle in the plan
  which process runs it (TUI on connect, the worker, or both under one lock), and what the cache
  (`R-STO-3`) does with rows it still holds. Not blocked.
- [ ] **MOD-101 - Record the agent exit code on `run_step`** (from the 2026-10-14 requirements
  audit). `R-ORCH-11`. Every engine `finish_step` passes `exit_code: None`
  (`crates/htui-orch/src/engine.rs:1636`, `:2743`, `:3777`, `:4570`, `:5038`) and Postgres overwrites
  the column (`htui-store/src/pg/write.rs:5341`), so `run_step.exit_code` is always NULL, though ANA-2
  (`docs/ANA-2.md:551`) defines it as the agent process's exit code and MOD-4 claims `R-ORCH-11`.
  Carry the child's exit status from the driver's `Done`/supervisor to `finish_step`; a recovered or
  promoted step with no process keeps NULL. Not blocked.
- [ ] **MOD-102 - Prompt upstream scope follows the active workspace** (from the 2026-10-14
  requirements audit). `R-PRM-1`, `R-PRM-2`. The engine builds every phase prompt with
  `PromptScope::project_only` (`crates/htui-orch/src/engine.rs:5967`, "the engine holds no
  workspace"), so upstream items in other projects of the active workspace render as one-line stubs,
  while the Backlog preview (`crates/htui/src/preview.rs:240`) uses the workspace: preview and real
  prompt disagree. No decision records the departure. Record the workspace on the run when it is
  queued (a run started with no workspace keeps project scope, `R-ENT-2`) and pass it to the engine;
  pin preview/engine equality with a test. Not blocked.
- [ ] **MOD-103 - Chat tab per session, and a project choice for a free-standing chat** (from the
  2026-10-14 requirements audit). `R-TUI-1`, `R-TUI-6`. `register_all` registers one `ChatTab`
  (`crates/htui/src/app/mod.rs:84`) holding one `session: Option<ChatSessionState>`
  (`crates/htui/src/ui/tabs/chat/mod.rs:182`); a new prompt replaces an ended chat, so `R-TUI-1`'s
  "Chat (one per session)" does not hold. A free-standing chat always starts against
  `ctx.projects.first()` (`chat/mod.rs:355`), so in a multi-project workspace no other project can be
  chosen. Also settle whether tool results show their content or only flip the call's status
  (`chat/transcript.rs:250-264`). Not blocked. **Requirements review (2026-10-15), `R-TUI-6`
  amended:** a free-standing chat runs against the **active workspace**, or the project when there is
  no workspace, whatever folder `htui` started in; there is no project picker. Tool results **show
  their content** (collapsible). The agent and model are chosen when the chat starts.
- [ ] **MOD-104 - Secret provider machine-identity bootstrap** (from the 2026-10-14 requirements
  audit). `R-SEC-1`. The `SecretProvider` trait (`crates/htui-core/src/secret.rs:28-37`) has kind,
  health, list_keys and resolve, but no "bootstrap a machine identity": the Infisical identity is
  pasted into Settings › Secrets by hand. Add the method and the Infisical implementation (create or
  register a machine identity for this box, store it in the keyring), or amend `R-SEC-1` if the
  provider cannot do it from a client. A keyring-less headless worker stays MOD-48's. Not blocked.
- [ ] **MOD-105 - In-app login and adapter install for `claude`** (from the 2026-10-14 requirements
  audit). `R-AGT-9`, `R-AGT-10`, `R-AGT-5`. `claude-agent-acp` advertises only a `terminal` auth
  method (`docs/ANA-4.md:599`), which MOD-21 refuses (`docs/decisions/mod/mod-21.md:151-153`), and
  `claude-cli` falls to the refusing default (`crates/htui-agent/src/driver.rs:427`), so neither claude
  row can be logged in from the app. In-app install handles only binary registry sources
  (`install/fetch.rs`), and only `agent_agy.json` declares `discovery.install`; the claude rows declare
  none, so a box without the npm adapter cannot install it. Run the terminal method in the embedded
  terminal pane (MOD-57) and add an npm-package install source, both as row data (`R-AGT-5`). Not
  blocked. **Requirements review (2026-10-15), `R-AGT-9` and `R-AGT-10` amended:** login is now
  **best effort**: where the agent offers no usable in-app method, showing the exact command to run
  is enough, so the embedded-terminal login is optional. Install grows: besides the npm source, install,
  detect and **update** agents through the box's tool managers (`mise`, `npm`/`volta`, `brew`,
  `winget`, `scoop`), preferring the one the box already uses (on the maintainer's box, `claude` and
  `agy` come from `mise`, the latter via `aqua:google-antigravity/antigravity-cli`), and show the
  installed version beside the latest. Autodiscovery must see mise and volta shims.
- [ ] **MOD-106 - Enforce one primary repo per project** (from the 2026-10-14 requirements audit).
  `R-ENT-3`. The schema stops two primaries (`uq_repo_primary`) but not zero: a project with repos
  and no primary is reachable (`crates/htui/src/ui/tabs/settings/hierarchy.rs:684`), and
  `item_spec.rs:367` copes by refusing bare paths. Enforce "at least one repo, exactly one primary"
  at the store write (promote another repo when the primary is removed, refuse removing the last),
  or amend `R-ENT-3` to allow a repo-less project. Not blocked. **Requirements review (2026-10-15):**
  `R-ENT-3` is amended, so a project may have **zero repos**, and when it has any, exactly one is
  primary. What remains: promote another repo when the primary is removed, and make "repos but no
  primary" unreachable. Removing the last repo is allowed. ANA-35 owns the wider repo-less model, so
  land this after it or with it.
- [ ] **MOD-107 - Item priority and labels; `heavy_build` as a label** (from ANA-28, concluded,
  `docs/decisions/ana/ana-28.md`, and the 2026-10-15 requirements review). `R-ENT-5`, `R-BOX-3`,
  `R-MCP-3`, `R-ORCH-6`, `R-ORCH-10`, `R-TUI-2`. Add an explicit item **priority** (auto mode already
  picks "by priority", `R-ORCH-6`) and free **labels**, kept apart from `required_tags`, which stay the
  routing capability set. `heavy_build` becomes a label: MOD-11's exposure check reads labels, and an
  item labelled `heavy_build` runs on any box. Migrate existing `required_tags` entries of
  `heavy_build` to the label, and drop `heavy_build` from the seeded box vocabulary. The Backlog filters
  and sorts by labels and priority; the item form edits them; the cache and search follow. Assignee and
  due date are later (`R-LATER-11`). Not blocked.
- [ ] **MOD-108 - Boxes and repos: wider probe, clone onto a box, git credentials** (from the
  2026-10-15 requirements review). `R-BOX-2`, `R-BOX-4`, `R-STO-1`, `R-SEC-2`, `R-AGT-6`. (1) The box
  probe also records language runtimes (`node`, `python`, `go`, `java`), tool and package managers
  (`mise`, `npm`, `volta`, `brew`, `winget`, `scoop`), free disk space and network reachability, as
  `box_probe_spec` data where possible. (2) A repo missing on a box can be **cloned** from its remote
  URL, and the per-box path recorded. (3) When the clone fails on authentication, name the remote and
  what it needs (SSH key, token, `gh auth`), mark the repo unavailable on that box, and offer an
  in-app setup: generate an SSH key and show its public half, or store an HTTPS token in the OS
  keyring for git to use (a credential helper), never in a file. ANA-34's push/PR credentials will
  reuse this. Not blocked.
- [ ] **MOD-109 - Settings and configuration surfaces** (from the 2026-10-15 requirements review).
  `R-TUI-10`, `R-TUI-8`, `R-STO-1`, `R-STO-3`, `R-STO-5`, `R-SEC-2`, `R-AGT-5`, `R-ID-5`, `R-MCP-3`.
  Five settled changes, landable as separate milestones:
  - **Key bindings in Postgres.** Edited in Settings and stored per user, shared by every `htui` on
    the store, cached for offline. `keys.toml` goes away. This reverses MOD-67's file (ANA-26), and
    MOD-67's catalogue, validation and `--default-keys` stay.
  - **Agent rows from Settings.** A user adds a registry row, beyond MOD-23's edit form. A row's
    isolation opt-out (`settings.user_extensions`, `R-ID-5`) is shown and editable.
  - **`command_run` controls.** A phase's exposure and the per-box command-class limits become
    editable; Settings › Kinds shows them read-only today.
  - **DSN from an environment variable**, for dev and CI. Read once, and stripped from every agent
    spawn (beside MOD-97's vendor-key strip).
  - **`htui migrate`.** An explicit, non-interactive CLI verb that applies pending migrations; the
    worker still never migrates.
  Not blocked.
- [ ] **MOD-110 - Agents and quota: event kinds, cost view, quota-aware auto mode** (from the
  2026-10-15 requirements review). `R-AGT-1`, `R-AGT-7`, `R-AGT-8`, `R-ORCH-6`, `R-HIS-1`,
  `R-TUI-6`. (1) The driver's event kinds gain plan/todo updates, subagent start and end, file read,
  mode change and questions to the user, mapped from ACP and the CLI stream as row data where the
  agent reports them. Check which already arrive as untyped events. (2) A cost and usage view over
  time, per agent, project and item, from the recorded usage events. (3) Auto mode **waits** for a
  subscription quota reset and resumes, rather than failing the item, with the wait visible in the
  queue overlay. Not blocked.
- [ ] **MOD-111 - Run control: advisory judge, step control, routing, budget raise** (from the
  2026-10-15 requirements review). `R-ORCH-2`, `R-ORCH-4`, `R-ORCH-10`, `R-ORCH-12`, `R-HIS-2`,
  `R-PRM-3`, `R-TUI-4`. (1) A gate can ask a configured judge agent for an **advisory** verdict,
  shown on the gate, and the person still decides. (2) Manual mode can **run one chosen step, skip a
  step, or restart from any step**. (3) **Fork from history**: start a new chat or run from a past
  step's context. (4) A run refused for missing capability tags **offers dispatch to a box that has
  them**; this part needs MOD-43. (5) In manual mode, a step about to start trimmed says so and lets the
  user **raise the budget** first (`R-PRM-3`'s trim stays as ANA-5 §4.4 built it). Not blocked except
  part (4).
- [ ] **MOD-112 - Agent context over MCP: read tools, item drafts, wider search** (from the
  2026-10-15 requirements review). `R-MCP-1`, `R-MCP-2`, `R-PRM-1`, `R-STO-8`, `R-ID-6`. (1) Read
  tools for items, documents and requirements, with **read scope wider than the run** (the active
  workspace) and writes still run-scoped. (2) A **search** tool over R-STO-8's index. (3)
  **`item_create`**: an agent proposes an item, held as a draft until a person accepts it. (4) The index
  also covers **notes** and **scrubbed run transcripts**, and searches can be scoped to a workspace
  (workspace-owned items once ANA-35 lands). The question tool is MOD-75's; the planner's tools are
  ANA-33's. Not blocked.
- [ ] **MOD-113 - Prompt library: templates and skills** (from the 2026-10-15 requirements review).
  `R-PRM-4`, `R-SKL-1`, `R-SKL-2`, `R-SKL-4`, `R-TUI-1`, `R-TUI-7`. (1) A **template library**
  shared across projects and workspaces, with per-project overrides, in its own **Templates tab**,
  where today Templates is a sub-tab of Skills. (2) Skill attachments gain **workspace** and
  **item-kind** levels (phase > kind > project > workspace > global), extending MOD-9's resolution
  (ANA-22). (3) Skills carry **resources** (scripts, references, files), and the prompt and session
  learn to deliver them. (4) **Import** whole skill folders and Claude Code plugins, resources
  included. Not blocked.
- [ ] **MOD-114 - TUI: notifications, requirement coverage, attachments, open in editor** (from the
  2026-10-15 requirements review). `R-TUI-11`, `R-ENT-15`, `R-TUI-3`, `R-TUI-5`, `R-ENT-12`,
  `R-ID-2`, `R-TUI-4`. (1) When something starts waiting on the user: a **terminal bell** and an
  **OS notification** (notify-send, macOS, Windows toast), switchable in Settings. (2) The citation
  kind **`verifies`**, plus a coverage view per requirement; the item's Reqs sub-tab flags suspect
  citations; the Graph shows cited requirements and linked documents as nodes (MOD-88's flow mode
  too). (3) **Attachments** on items and documents in Postgres under a size cap, with an Attachments
  sub-tab; who may own one waits on ANA-35. (4) **Open in `$EDITOR` at the line**: an artifact, or a
  file named by a tool call, edit proposal or diff, through MOD-9/MOD-57's editor handoff. Not
  blocked except part (3).
- [ ] **MOD-8 - Legacy markdown import.** `R-LATER-3`. Map old prefixes to kinds per project,
  preserve keys, build links. **Promoted to must by the 2026-10-15 requirements review:** this
  repository is its first user. ANA-9 §7.1 and ANA-11 §6 already cover its design, but it should
  follow ANA-35, whose status, link and ownership model it imports into. MOD-6 landed (`docs/decisions/mod/mod-6.md`, importer
  mint variant per ANA-9 §7.1 still to write). Widened by ANA-11 (`docs/ANA-11.md` §6 phase 3): also import
  `docs/REQUIREMENTS.md` into the requirement tables MOD-38 added (`docs/decisions/mod/mod-38.md`), map each `DECISIONS.md` status to
  `item.resolution` (`shipped` → `done`), write-ups to `summary` documents and each analysis doc to
  the `verdict` document, and scan body `R-` IDs once into `addresses` citations.

### Deferred backlog

- [ ] **MOD-36 - Weighted agent assignment across fan-out candidates** (from MOD-4 milestone 4,
  OQ-7; ANA-21 is done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`,
  `R-LATER-13`. **Deferred by the 2026-10-15 requirements review:** best-of-N fan-out is now the
  later-tier compete mode (`R-LATER-13`), and `R-ORCH-7` means divided work (ANA-33).
  Milestone 4 runs every candidate of a group on the
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
  **ANA-30 note (2026-10-14, `docs/decisions/ana/ana-30.md`):** the `haiku` alias now runs Haiku 5.5;
  seed the `claude`/`claude-cli` map with `claude-haiku-5-5` unscored (resolving through `*`) and no
  `haiku` key at 25 (`docs/ANA-21.md` §5.7 amendment).

- [ ] **MOD-3 - Diff tab + code explorer.** `R-LATER-1`. Later tier; needs its own ANA first.
- [ ] **MOD-5 - Issue tracker sync.** `R-LATER-2`. `IssueSync` trait, OneDev first. Since the
  2026-10-15 requirements review it is **two-way by field ownership** (each field has an owning side),
  no longer a downstream mirror. Later tier; needs its own ANA first.


### Tooling findings

- [ ] **TOOL-12 - hr sandbox: settings.json, agents, commands and hooks in ~/.claude stay writable from a run**
  (from TOOL-10, `docs/decisions/tool/tool-10.md`). TOOL-10 made `~/.claude/plugins` and `skills`
  read-only, so a run cannot install a plugin or write a mod. It can still write `settings.json` (hooks,
  `enabledPlugins`, `extraKnownMarketplaces` with a directory source on another shared path such as
  `~/.headroom`, `~/.serena`, `~/.cache/uv` or the run's project dir) and `agents/`, `commands/`, `hooks/`.
  Decide: a per-run copy of `settings.json` seeded at `up` (login and `/model` then stop flowing back), or a
  read-only bind plus a writable overlay for the few keys a run needs; and whether `agents/` and `commands/`
  go read-only too. Check what breaks in a run first (`/login` writes `.credentials.json`, which is a
  separate file). Not blocked.
- [ ] **TOOL-13 - Requirement-citation check in the workflow validator; backfill six write-ups** (from
  the 2026-10-14 requirements audit). `R-NF-4`. `validate-workflow-docs.sh`/`.ps1` reports 0/0 while
  `docs/decisions/ana/ana-13.md`, `docs/decisions/ana/ana-15.md`, `docs/decisions/ana/ana-19.md`,
  `docs/decisions/ana/ana-20.md`, `docs/decisions/mod/mod-29.md` and `docs/decisions/mod/mod-35.md`
  (and `docs/ANA-15.md`) cite no `R-` ID. Add a check: every open ANA/MOD line and every
  ANA/MOD write-up (ANA-1 and ANA-8 exempt, they predate REQUIREMENTS) cites at least one defined,
  non-withdrawn ID, and ranges such as `R-SEC-1..4` are written out so MOD-8's import scan can read
  them. Backfill the six write-ups and normalise the MOD-29, MOD-35 and TOOL-5 write-up headings
  to `# PREFIX-N - Title (status, date)`, which the validator also misses. Not blocked.
  **Requirements review (2026-10-15), `R-NF-4` amended:** the check covers **every prefix** (TOOL,
  CLEAN, NEXT, VAL as well as ANA and MOD), for open lines and write-ups alike. Backfill the write-ups
  that newly fall under it, or exempt items archived before 2026-10-15 if that list is long, and
  record which choice was made.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 10 (ANA-25 learned weights, ANA-31 agy isolation, ANA-32 warm-start measurement, ANA-33 step graphs v2 and divided fan-out, ANA-34 coordinator, ANA-35 entity model for any project, ANA-36 further MCP tools, ANA-37 Qdrant required?, ANA-38 project memory, ANA-39 mandatory context tools) |
| MOD-N   | 36 (MOD-16 Windows verification, MOD-27 swarm, MOD-75 agent question tool, MOD-43 remote dispatch, MOD-44 container env, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-81 terminal widths, MOD-82 shared pane chrome, MOD-83 display labels and errors, MOD-85 remaining accent uses, MOD-88 item-graph flow mode, MOD-94 permission answers outside the TUI, MOD-97 API-key env strip, MOD-98 summary phase, MOD-99 error-reporting endpoint, MOD-100 retention sweep, MOD-101 step exit code, MOD-102 workspace prompt scope, MOD-103 chat per session, MOD-104 identity bootstrap, MOD-105 claude login and install, MOD-106 primary repo, MOD-107 priority and labels, MOD-108 boxes and repos, MOD-109 settings surfaces, MOD-110 agents and quota, MOD-111 run control, MOD-112 agent context over MCP, MOD-113 prompt library, MOD-114 TUI notifications/coverage/attachments/editor, MOD-8 import; deferred MOD-36 weighted assignment, MOD-3 diff, MOD-5 tracker) |
| CLEAN-N | 0 |
| TOOL-N  | 2 (TOOL-12 sandbox settings hardening, TOOL-13 requirement-citation check) |
