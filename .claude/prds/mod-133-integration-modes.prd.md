# MOD-133 — Integration records and modes (local, push, pull_request)

> Routed as **PRD** by `/handoff-run MOD-133` (C2 and C4 fired; C3 not fired because ANA-34 §7 already
> settles the design, a close call). Sandbox run `hr/MOD-133`, 2026-11-14. Ultracode accepted for the
> implement phase. Origin: ANA-34 P1 (`docs/ANA-34.md` §7.2-§7.4, §9), with ANA-35 §7.1-§7.2's landed
> check (`docs/ANA-35.md:788-795, 874, 1356`). Requirements: `R-ORCH-14`, `R-ORCH-11`, `R-TUI-9`,
> `R-ENT-4`. PRD gate: the maintainer answered Q1-Q3 below with the recommended options, 2026-11-14.

## Problem

A run that finishes `done` has merged each step into whatever branch the person's primary checkout has
out, and nothing more happens: htui never pushes, never opens a pull request and records nothing about
where the work landed. A person who wants the work on the remote does it by hand after every run, and a
dependent item is released the moment its blocker's run is `done`, even when that work exists only in a
local checkout. `R-ORCH-14` says the coordinator integrates "merging, pushing or opening a pull request
as the project configures", and nothing configures or performs the last two.

## Evidence

Read at `eeeae453` (branch `hr/MOD-133`). Paths relative to the repo root.

- **No push, fetch or remote write exists.** The only remote-aware git code reads fetch URLs
  (`crates/htui-orch/src/infer.rs:311-323`); the git child module lists its verbs as `worktree
  add/remove`, `merge --no-ff`, `merge --abort`, `reset --hard`, `diff` (`crates/htui-orch/src/isolate/git.rs:11-17`).
- **Step reconcile merges into the checked-out HEAD, never a named branch.** `reconcile_isolated` reads
  `git::head(checkout.local_path)` and merges there (`crates/htui-orch/src/isolate/real.rs:1198-1285`); a
  worktree step's base is the primary's HEAD (`real.rs:474-484, 1782-1790`).
- **Run completion writes nothing about landing.** `finish_run_on` locks the run, checks the move and
  updates status; `Done` has no branch of its own (`crates/htui-store/src/pg/write.rs:2607-2662`; Mem
  `crates/htui-core/src/store/mem.rs:8580-8619`). The engine's only `Done` site is the walk loop's
  `Pass::Finished` (`crates/htui-orch/src/engine.rs:4427-4436`).
- **"Released" means "latest graph run `done`" at seven sites, none of which can see landing:** Pg
  `items(ready)`, `ready_items`, `queue_rows`, `prune_finished_entries` (`crates/htui-store/src/pg/read.rs:156-166,
  2406-2415, 2693-2704`; `pg/write.rs:12917-12940`), Mem `is_released` and `prune_finished_entries`
  (`mem.rs:6503-6506, 1469-1478`), cache `ready_case` (`crates/htui-store/src/cache/read.rs:236-255`).
  `ItemState::is_finished` already says "MOD-133 adds \"and landed\"" (`crates/htui-core/src/model/item.rs:93-97`).
- **No per-repo configuration exists.** `repo` has `remote_url`, `default_branch`, `is_primary` and no
  settings (`crates/htui-store/migrations/0001_init.sql:185-196`); the settings registry has no string or
  choice kind (`crates/htui-core/src/prompt/settings.rs:357-369`).
- **Close-out records commits only.** The summary is a `| repo | position | phase | attempt | commits |`
  table and one line per run (`crates/htui-orch/src/closeout.rs:119-164`).
- **The need is the maintainer's own workflow** (assumption, needs validation by use): every finished
  `hr` run today is landed by hand (`scripts/hr collect`, then a manual merge and push), and the
  maintainer's forge is OneDev (`docs/ANA-34.md` §7.4).

## Users

- **Primary**: the maintainer (or any htui user) running items against a project whose repos live on a
  remote, who wants a finished run's work pushed to the target branch, or offered as a pull request,
  without a manual step; and anyone whose items depend on others, who wants a dependent to start only once
  its blocker's work has actually landed.
- **Not for**: plain-folder repos and repos without a remote (they stay `local`); off-box targets
  (MOD-130/132 bring a branch home, then this integrates); opening, updating or following pull requests
  on a forge, and htui's own push or forge credentials (MOD-134); the close-out checklist and the
  `done`-requires-landed close-out rule (MOD-135); coordinator agent turns (MOD-136).

## Hypothesis

We believe **a per-repo `integrate` mode (`local`, `push`, `pull_request`), snapshotted at run start and
carried out by the executing box after the run finishes, with one durable integration record per (run,
repo)** will **land finished work where the project wants it and hold dependents until it has landed**
for **users running items on repos with a remote**.
We'll know we're right when **a `push`-mode run that finishes `done` fast-forwards the remote's target
branch exactly once (also across a worker crash and restart), a non-fast-forward or an auth failure
leaves the run's work unpushed with a refusal a person can read and retry, a dependent item stays
unready until its blocker's records are landed, and `local`-mode projects behave byte-for-byte as
today**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| `local` unchanged | no record written, readiness and close-out output identical to today | existing conformance and engine suites pass unedited; a pinned case for a `local` run |
| `push` lands once | one fast-forward per (run, repo); a re-run of the arm after a crash pushes nothing new | worker/engine tests against a local bare remote, with a kill between push and record write |
| Never forced | no `--force`/`+ref` push; non-fast-forward parks with a typed refusal | tests on a remote that moved |
| Readiness sees landing | every released/ready site (Pg, Mem, cache) agrees for each record state | store conformance cases over MemStore and PgStore, plus the cache's `items(ready)` |
| Close-out reports it | the summary gains an integration section naming mode, target and outcome per (run, repo) | `closeout.rs` unit tests |
| Snapshot holds | a mode changed mid-run never moves where that run lands | engine test changing the setting between start and finish |

## Scope

**MVP** — per-repo `integrate` mode with a project default; snapshot at run start; an `integration`
record per (run, repo) written in `finish_run(Done)`'s transaction for non-`local` repos; an
integration pass on the run's executing box (the process the box's executor role names, I-1), leased
and idempotent; `push` as a plain fast-forward `git push` of the target branch; refusals park the
record with a typed reason, an `item_note` and a waiting row, and `Unblock` retries; the landed check
at every readiness site with the cache mirroring the records; an integration section in the close-out
summary; Settings fields for the repo mode and the project default; `pull_request`'s run branch
`htui/item/<key>` (worktree and copy isolation only) and its push.

Decisions taken at the PRD gate (maintainer, 2026-11-14, recommended answers):

- **Q1 — `pull_request` goes as far as branch and push.** MOD-133 builds the run branch (step
  reconcile merges into `htui/item/<key>` instead of the checked-out HEAD) and pushes it; the record then
  waits for MOD-134 to open the PR. Settings keeps `pull_request` unselectable until MOD-134 lands, so no
  real run waits on it. It is MOD-133's last milestone.
- **Q2 — `push` uses the executing box's own git credentials, for now.** The push child runs with what
  git already has on the box (SSH keys or agent, a configured credential helper), never prompting
  (`GIT_TERMINAL_PROMPT=0`, ssh `BatchMode=yes`). An authentication failure parks the record with a typed
  refusal naming the remote. MOD-134 replaces this with htui's keyring helper and decides whether the
  ambient path stays (`R-SEC-4`'s no-fallback posture is MOD-134's to settle).
- **Q3 — requirement edits.** This item applies ANA-34 §10.2 (`R-ORCH-14`) and §10.9 (`R-ORCH-11`) to
  `docs/REQUIREMENTS.md`; §10.1 (`R-TUI-9`) stays owed until MOD-135.

**Out of scope**
- Forge adapter, opening/updating PRs, auto-merge, PR state polling, forge kind/base URL/draft/
  reviewers/merge-strategy fields — MOD-134 (ANA-34 §7.4's remaining fields arrive with the adapter).
- htui's credential helper and keyring entries for push — MOD-134 (with MOD-108).
- The close-out checklist, a `CloseOut` waiting reason and `done` requiring landed work at close-out —
  MOD-135 (ANA-34 §7.6). This item makes readiness see landing; close-out's law is MOD-135's.
- `repo.vcs` and folder repos — MOD-145. Until it lands, "`local` only" is enforced for a repo without a
  `remote_url`; whichever of MOD-133/MOD-145 lands second adds the folder clause.
- Off-box targets and collect — MOD-130/MOD-132.
- A cross-process push lock: the record's lease serialises integration of one record; two runs pushing
  one target branch are serialised by the remote's fast-forward rule (the loser parks).

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Records, `push` and the landed check | a repo set to `push` (store/API level) has a finished run's target branch fast-forwarded on its remote by the executing box, once; a refusal parks the record with its reason and a note; dependents wait for landing at every readiness site; close-out lists the integration | complete | `.claude/plans/mod-133-m1-records-push-landed.plan.md` |
| 2 | Settings, visibility and retry | the repo mode and project default are edited in Settings with refusal reasons (no remote → `local` only; `pull_request` until MOD-134); a refused integration shows its own waiting row and `Unblock` retries it; the item's display state says landing/refused | complete | `.claude/plans/mod-133-m2-settings-visibility-retry.plan.md` |
| 3 | `pull_request` run branch | a `pull_request` run's steps land on `htui/item/<key>`, which is pushed after the run; the record waits for MOD-134 | complete | `.claude/plans/mod-133-m3-pull-request-run-branch.plan.md` |

## Open Questions

- [x] Q1-Q3 — decided at the PRD gate (above).
- [ ] Should the TUI executor (sweep every 120 s) run the integration pass at once after a run finishes,
  rather than on its next sweep? — the plan decides (default: an immediate pass on the finishing process,
  the sweep as the recovery path).
- [ ] What the item's display state is while its records are pending or refused (`done` today) — M2's
  plan decides within ANA-35 §7.1's table.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A crash between `git push` and the record write pushes twice or loses the outcome | medium | low (a second fast-forward to the same commit is a no-op) | idempotence: before pushing, compare the remote ref with the run's commit (`ls-remote`); record states are compare-and-set |
| `finish_run_on` also runs inside `close_batch`, so records appear on that path too | medium | medium | the plan checks every `finish_run_on` caller and pins the batch case in conformance |
| Seven spellings of "released" drift apart | high | high (Pg/Mem/cache disagree on readiness) | one conformance case per record state, run over MemStore and PgStore, plus a cache case |
| The ambient-credential push leaks a prompt or hangs | low | medium | `GIT_TERMINAL_PROMPT=0`, `GIT_SSH_COMMAND` with `BatchMode=yes`, the git child's 120 s verb budget |
| `pull_request`'s run branch changes step bases and reconcile, the walk's most delicate path | medium | high | last milestone, own plan; worktree/copy only; the snapshot refuses other modes |
| The new migration renumbers at merge (parallel branches) | high | low | the next free number at plan time, repinned at collect (HANDOFF live coordinates) |

---
*Status: ACCEPTED at the PRD gate 2026-11-14 — milestone 1 complete 2026-11-15 (Unblock retry moved to M2 at planning: it touches `unblock_enabled` and the waiting views, which M2 owns); milestone 2 complete 2026-11-15; milestone 3 complete 2026-11-15 (MOD-133 done, `docs/decisions/mod/mod-133.md`).*
