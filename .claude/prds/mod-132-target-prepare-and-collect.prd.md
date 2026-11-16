# MOD-132 — Target preparation and branch collection by bundle

> Routed as **PRD** by `/handoff-run MOD-132` (C2, C3 and C4 fired). Ultracode recommended for
> implement; accepted. The contract is `R-ORCH-15`, `R-SEC-5` and `R-ENT-17`; the design is
> `docs/ANA-42.md` §7.5 and §7.6 (all six §8 decisions accepted 2026-11-14), amended by
> `docs/ANA-33.md` §7.11 (stage rows) and ANA-34 Q9 (after collect, the repo's integration mode
> applies). Neighbours: MOD-130 (done: target record, cascade, refusals), MOD-44 (container
> decorator, open), MOD-131 (ssh decorator, open), MOD-117 (workspace components, open), MOD-133
> (done: integration modes), MOD-139 (done: `stage` role reserved), MOD-164 (done: the `capture`
> stage, the one stage writer today).

## Problem

An item can now be *bound* to a container or an ssh machine (MOD-130), but nothing gets the code
there or the work back. `R-ORCH-15` says htui prepares the target and "the work comes back when the
person collects the branch, as `scripts/hr collect` does"; that discipline lives only in a shell
script for the maintainer's own sandbox. Without a prepare stage and a hostile-checkout-safe collect,
MOD-44 and MOD-131 would each have to invent their own clone and their own way of bringing a branch
home, and the first one to ship would set the security bar by accident.

## Evidence

Facts read from the tree on 2026-11-15 (`hr/MOD-132`); each is re-checked by the plan fact-check.

- **Stage rows are reserved, and only `capture` writes one.** `0029_divided_fanout.sql:39-55` adds
  `run_step.role` with `'stage'`; `0033_memory_capture.sql:63-70` widens `chk_run_step_stage` to
  positions `-4..-1` with `node_key IN ('capture','pick','prepare','collect')` and binds no key to a
  position. `StepRole::Stage` and `STAGE_KEYS` (`crates/htui-core/src/model/run.rs:1192-1212`); only
  `CAPTURE_POSITION = -4` has a constant. The store mirror is `run_step_refusal`
  (`crates/htui-core/src/store/traits.rs:3379-3410`). The only writer is `Engine::capture_row`
  (`crates/htui-orch/src/engine/capture.rs:264-289`), called from `end_run`. Recovery, cancel, the
  walk, `node_of` and operator verbs already skip or refuse stage rows (`engine.rs:4112`, `:5145`,
  `:11078`; `command.rs:765-770`, `:1204`, `:1224-1232`; conformance
  `stage_rows_are_ignored_by_the_walk`).
- **The run graph shows stage rows as plain layers.** `RunStepSummary` has no role or key
  (`run.rs:1696-1747`); `execution_graph::layers` groups by `(position, attempt)`
  (`crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs:88-127`), so a stage sorts before
  position 0 unmarked.
- **Only `local` runs.** `RUNNABLE_KINDS = &[TargetKind::Local]` and `run_start_refusal`
  (`crates/htui-core/src/model/target.rs:50`, `:237-271`) refuse docker (MOD-44), ssh and
  ssh_docker (MOD-131), enrolled (MOD-47). `Engine.runnable` is a field set from `RUNNABLE_KINDS` in
  `Engine::new` (`engine.rs:625`, `:1303`). The locator is untyped JSON with names and addresses only
  (`locator_refusal`, `target.rs:466-479`). The engine reads the binding only at enqueue
  (`enqueue_with`, `engine.rs:1460-1557`); the walk holds ids.
- **No channel to a target exists.** Nothing in `crates/` runs a command anywhere but this box:
  `TransportBuilder` (`crates/htui-agent/src/registry.rs:29-42`) and `ResolvedLaunch`
  (`launch.rs:508-520`) have no host or wrapper; `run_shell` (`crates/htui-orch/src/verify.rs:557-590`)
  and the git `Cli` are local.
- **Step worktrees come from this box's checkouts.** `GixIsolator` is built per process from this
  box's `repo_box_path` map (`crates/htui-worker/src/runtime.rs:533-563`); trees are
  `<scratch_root>/<run>/<step>/<repo>` on `htui/<step>` (`isolate/real.rs:241-243`, `:791-842`).
  `RunStepTree.path` is "absolute, on the executing box" and carries no box (`run.rs:707-721`).
- **Integration follows `done`.** `run_to_rest` calls `end_run(Done)` (capture, then `finish_run`,
  which writes one `Integration` record per non-local repo the run changed), then `cleanup_walk`, then
  `land` (`engine.rs:4783-4831`, `:3895-3911`; `model/integration.rs:6-9`). `land` reads
  `run_step_commit` rows and this box's checkouts. The `pull_request` run branch is
  `htui/item/<key>`, built in a detached landing worktree (`real.rs:1299-1435`).
- **Waiting reasons have no collect.** `WaitingReason` is Gate, JudgeFailed, Selection, Unblock,
  Landing, Interrupted, Permission (`crates/htui-worker/src/views.rs:528-549`); Landing is the
  precedent for "a done run's work did not arrive; retry with a key" (`retry_landing`,
  `integration.rs:93-96`).
- **The collect discipline to port** (`scripts/hr`): temp ref `refs/hr-collect/<item>` deleted on every
  exit (`:1082-1092`, `:1154`); fetch with `core.hooksPath=/dev/null`, `transfer.fsckObjects=true`,
  `--no-tags --no-write-fetch-head --no-recurse-submodules` (`:1155-1157`); host reads of the clone
  restricted to plumbing with fsmonitor and hooks off (`:187-203`); rewritten history refused unless
  `--force` (`:1144-1149`, `:1160-1164`); compare-and-swap `update-ref` (`:1185-1186`); a diffstat of
  host-executing config paths that `--yes` never answers (`:1075-1076`, `:1166-1182`). The clone is
  `--no-hardlinks --no-checkout` with `remote.origin.pushurl` set to a refusal (`:828-833`).
- **htui's git layer has none of it yet.** `isolate/git.rs`: gix for reads and CAS ref writes
  (`create_branch`, `move_branch`, `:2236-2266`), a `Cli` for worktree, merge, diff, `ls-remote`,
  `push`; no bundle, fetch, fsck or `update-ref`; hooks are not disabled in production; `Cli::command`
  scrubs `GIT_*` location variables and inherits `HOME`/`~/.gitconfig` on purpose (`:107-124`,
  `:336-364`).
- **No compose or free ports anywhere.** `ProjectSettings` (`crates/htui-core/src/model/kind.rs:656-679`)
  has no such key; "compose" occurs only in `scripts/hr` and dev infrastructure. Workspace components
  and the generated compose file are MOD-117 (open, blocked on MOD-116).

## Users

- **Primary**: the maintainer, who today collects sandbox work with `scripts/hr collect` by hand and
  wants htui runs on a container or ssh target to come home under the same rules, without a script.
- **Also served**: MOD-44 and MOD-131, which implement the target channel and then get prepare and
  collect for free; MOD-117, which fills prepare's components slot; MOD-133's integration, which runs
  from the collected branch.
- **Not for**: a run on a `local` target. It keeps its worktrees, writes no prepare or collect row,
  and integrates exactly as today.

## Decisions (maintainer, 2026-11-15)

- **D1 — Seam and stand-in.** MOD-132 defines a small target command channel (run a command on the
  target with streamed stdin and stdout, an exit status, a kill) and builds prepare and collect on it.
  It is proved end to end by a **test-only stand-in target**: a separate clone on this box behind the
  same channel. No user-selectable kind is added and `RUNNABLE_KINDS` is unchanged; MOD-44 and MOD-131
  implement the channel for docker and ssh. Users see no change until one of them lands.
- **D2 — Compose waits for MOD-117.** Prepare has a components slot that does nothing in this item;
  MOD-117's HANDOFF entry gains the note that it starts an item's components on the target during
  prepare (`R-ENT-17`, `R-SEC-5`). No compose or port logic is built here.
- **D3 — Step worktrees on a target are each transport's.** Prepare registers its clone as the target
  box's checkout of the repo, and collect bundles the run branch from it. Making step worktrees inside
  that clone needs the channel, so it is MOD-44's and MOD-131's; their HANDOFF entries gain that note.
  The stand-in works because its clone is on this box.
- **D4 — The clone is seeded by bundle from here.** Prepare bundles the base commit from this box's
  checkout, sends it through the channel, and the target clones from it. The target needs no git
  credentials and no network; its origin names the real remote with push disabled.
- **D5 — The run waits at collect, not done.** After its last node, a target run waits at its
  `collect branch` stage and shows in Waiting with a key. Only a successful collect lets it finish
  `done`; integration (`local`, `push`, `pull_request`) then runs from the collected branch. A refused
  collect stays waiting with its reason and can be retried.

## Hypothesis

We believe **a target channel with a `prepare · <target>` stage that seeds a push-disabled clone by
bundle, and a `collect branch` stage that brings the run branch home by bundle under
`scripts/hr collect`'s rules and waits for the person** will **give every driven target one safe way
in and out** for **the maintainer and the transport items that follow**.

We'll know we're right when **a run on the stand-in target writes a `prepare` row at -2, works in the
stand-in's clone, waits at a `collect` row at -1, comes home on `htui/item/<key>` after the person
collects, then integrates by the repo's mode; and a hostile clone (hooks, `fsmonitor`,
`packObjectsHook`, a rewritten branch, a corrupt or incomplete bundle) executes nothing on this box and
moves no local ref** — with every existing local run unchanged.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Hostile clone | A clone carrying hooks, `core.fsmonitor`, `uploadpack.packObjectsHook` and a malicious `.gitattributes` filter runs none of them on this box during prepare or collect | Tests with sentinel-writing hooks/commands in the stand-in's clone; sentinel absent |
| Bundle integrity | A bundle that fails `verify`, lacks the base prerequisite, or carries an object `fsck` rejects moves no ref and leaves no temp ref | Collect tests with crafted bundles |
| Rewritten history | A collected tip that is not a descendant of the local branch is refused with a sentence; an explicit force by the person moves it; the move is compare-and-swap (a concurrent local move refuses) | Collect tests |
| Config paths | A collected change touching host-executing paths needs a confirmation that names them; no flag or setting answers it silently | Engine/TUI tests: Waiting row confirmation |
| Seeding | Prepare's clone holds exactly the base commit, origin is the real remote, any push from it fails, no credential is copied | Stand-in tests: `git push` refused, config read |
| Stage rows | A target run writes `prepare` at -2 before its first node and `collect` at -1 after its last; a local run writes neither | Engine tests over `MemStore`; conformance cases on both stores |
| Run state | A target run is not `done` until collected; after collect, integration records come from the collected branch per the repo's mode | Engine tests with the stand-in |
| Local unchanged | Every existing local run path behaves as before | The full gate, existing tests unedited except pinned counts |
| Visible | The run graph names the two stages; Waiting shows the collect row, its reason on refusal, and its keys (from `keys.toml`) | `insta` snapshots |

## Scope

**MVP** — the target channel with a test-only stand-in; prepare (bundle-seeded, push-disabled clone,
recorded base, registered as the target box's checkout, components slot left empty); collect (bundle
on the target through the channel, quarantine file here, `verify`, fetch into `refs/htui-collect/<item>`
with fsck and hooks off, rewritten-history refusal with a person-only force, compare-and-swap move of
`htui/item/<key>`, config-path confirmation); the two stage rows in the engine; the run waiting at
collect; integration from the collected branch; the run graph and Waiting surfaces; HANDOFF notes on
MOD-44, MOD-131 and MOD-117.

Also in scope:

- **Several repos.** A run whose scope spans several repos prepares and collects each; the collect
  row settles when every repo is home, and a refusal names the repo.
- **Failure states.** Target unreachable, bundle refused, history rewritten, no checkout of the repo on
  this box: each is a distinct, reported reason on the waiting row, never a silent `failed` run.
- **Stop and resume.** A stopped run at prepare or collect resumes at that stage (`R-ORCH-16`: one of
  htui's own steps halts at once and runs again on resume).

**Out of scope**

- **docker and ssh channels**, the `TargetTransport` launch decorator, relay forwarding, helper
  shipping, remote probe, install and login — **MOD-44**, **MOD-131**.
- **Step worktrees inside a target's clone** — **MOD-44**, **MOD-131** (D3).
- **Components and compose on the target** — **MOD-117** (D2).
- **Agents install and login during prepare**, and their permission cards (`R-AGT-11`) — the transport
  items; prepare leaves an ordered slot for them.
- **Git credentials on a target**, ssh-agent forwarding — **MOD-108**; not needed by D4.
- **Cached preparation** across items — deferred (ANA-42 §9); v1 prepares each run.
- **A user-selectable stand-in kind** — rejected (D1).
- **Enrolled boxes** — **MOD-47** (worker-mediated collection, ANA-42 §6.5 D).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Channel and bundle mechanics | The target channel and stand-in exist; prepare seeds a push-disabled clone by bundle; collect brings a branch home under every refusal, proved against hostile clones and crafted bundles. No engine change. | complete | `.claude/plans/mod-132-m1-channel-and-bundles.plan.md` |
| 2 | Target runs prepare and wait to be collected | A run on the stand-in writes `prepare` (-2) and `collect` (-1); it waits at collect, is collected by a store-backed verb with the config-path confirmation, then finishes `done` and integrates from the collected branch; stop/resume and refusals hold on both stores. | pending | — |
| 3 | Collect is visible and keyed | The run graph names the stages; Waiting shows the collect row with its reason, keys and confirmation; HANDOFF notes on MOD-44, MOD-131, MOD-117. | pending | — |

## Open Questions

- [x] **Host-executing path list.** Settled in the M1 plan (P9), widened to nested tool files by the maintainer on 2026-11-16. `scripts/hr` lists htui's own paths. The product default must cover
  agent instruction and config files, git and environment hooks, and build or toolchain config that runs
  code; whether a project can extend it is TBD — decided in the M1 plan from the agents htui drives
  (`docs/` agent notes), not invented.
- [ ] **Which local branch collect moves when the repo mode is `local`.** `htui/item/<key>` matches
  MOD-133's run branch; whether a `local`-mode repo then merges it into the checkout's branch
  automatically or leaves the merge to the person (ANA-42 §7.5 narrowed by ANA-34 Q9) — TBD in the M2
  plan against `land`'s current `local` behaviour.
- [ ] **How `land` consumes a collected branch.** `land` reads per-step commits and this box's step
  worktrees today; for a target run it must work from the collected branch. TBD in the M2 plan (fact
  check of `Isolator::land` and `run_step_commit`).

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The channel shape chosen here does not fit `docker exec -i` or `ssh -T` | Medium | MOD-44/131 rework the seam | Keep the channel to argv, stdin, stdout, stderr, status, kill; check it against both command lines in the M1 plan |
| Running local `git` on a received bundle still reads a hostile config | Low | Code runs on this box | Fetch runs in this box's repo, never the clone; the bundle is a plain file; hooks off, fsck on, no `--git-dir` into the clone |
| The stand-in hides a real transport's failure modes (drops, partial streams) | Medium | Collect misreports a dropped link | Stand-in can inject a truncated stream and a non-zero exit; both covered by tests |
| A target run that never gets collected holds a slot or lease | Medium | Queue stalls | Waiting at collect releases the driver's slot as other waiting reasons do; checked in the M2 plan |
| A new waiting reason or key moves many snapshots | High | Noisy diff | Full `insta` run per memory note; snapshot moves listed in the plan |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
