# ANA-34 - Coordinator at project and workspace level

> **Scope note:** Settle what `R-ORCH-14`'s coordinator is: an agent session, a deterministic process
> or both, and where `R-ID-6` draws the line between them; its lifetime and the box it runs on (the
> worker, `R-ORCH-12`); how it relates to ANA-33's planner and integrate step; the push and pull-request
> configuration and its credentials (`R-BOX-4`, `R-SEC-2`); the close-out checklist; and the final text
> of `R-TUI-9`. Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md`,
> `docs/requirements-review-2026-10-15.md` (the item's origin) and `docs/ui-redesign-findings-2026-10-15.md`
> §4.3 (Chat groups). Best read beside ANA-33, which is not yet written.
>
> **Requirements addressed:** `R-ORCH-14`, `R-TUI-9`, `R-ENT-8`, `R-ID-6`, `R-ORCH-7`, `R-ENT-15`.
> Touched at their seams: `R-BOX-4` and `R-STO-1` (push and forge credentials), `R-SEC-2`/`-3`/`-4`
> (credentials never reach a session, scrub, no fallback), `R-ORCH-11` (what a run records),
> `R-ORCH-12`/`R-ORCH-15` (where it runs, collect), `R-TUI-6` (Chat groups), `R-MCP-2` (`item_create`),
> `R-ENT-13` (memory, ANA-38), `R-ORCH-1`/`R-ORCH-6` (planned graphs, auto mode).
>
> **Status (2026-10-18): drafted, awaiting maintainer acceptance of §8.** No implementation code is
> written here. Verdict (§7): a coordinator is **a role with durable state in the store, not a
> long-lived process or session**. Its **integration and close-out writes are deterministic code**: an
> integrator that, when a child's run completes, lands the run's work as the project configures
> (`local` merge, which is today's behaviour and stays the default; `push`, fast-forward only; or
> `pull_request` through a forge adapter, OneDev first because the maintainer's forge is OneDev). It runs
> as one more sweep arm in the executor that already holds the run's box, checkout and credentials. Its
> **agent part is optional and short-lived**: turns opened by a person's message or by an event, seeded
> from store state, which only propose (follow-up drafts, a resolution, citation re-confirmations, a PR
> description, a queue request) for a person to accept. The coordinator **never resolves a conflict**;
> ANA-33's integrate step combines a run's own parts, the coordinator lands finished runs. Close-out stays
> a person's confirmation, now over a checklist the coordinator prepares; resolution `done` requires the
> work to have landed. The workspace coordinator follows and relays in version one and integrates
> nothing of its own. Push and forge credentials live in the executing box's OS keyring and never enter
> an agent's environment. Proposed spawns: **P1**, **P2**, **P3**, **P4**.

---

## 1. Context and problem statement

During the 2026-10-15 requirements review the maintainer asked where merges and pull requests happen
(`docs/requirements-review-2026-10-15.md:234-242`). The answer today: the orchestrator merges each done
step's branch into the local primary checkout (ANA-2 §4.6), close-out records hashes and writes a summary
(`crates/htui-orch/src/closeout.rs`), and nothing pushes or opens a pull request. The review added a new
concept, the **coordinator**, one per project and one per workspace, above item runs. It "only manages":
it follows what happens below it, talks with its children and its parent, integrates a child's work
when the child's run completes ("merging, pushing or opening a pull request as the project configures"),
and runs close-out with its checklist (follow-up items, re-confirming suspect citations, choosing the
resolution). It does no item work itself (`docs/REQUIREMENTS.md:426-431`). `R-TUI-9` was amended to name
the coordinator and defer the split to this item (`docs/REQUIREMENTS.md:601-604`).

The UI redesign then gave the coordinator a face: an always-writable chat per workspace and per project,
listed in the Chat tab above item runs (`R-TUI-6`, `docs/REQUIREMENTS.md:575-578`; findings
`:333-342`), modelled in the skeleton as LLM sessions with an agent and a model
(`spikes/ui-skeleton/src/data.rs:298-325,720-756`). So the requirement says "only manages", and the UI
assumes "an agent you talk to". This item reconciles the two and fixes the machinery.

Method: six readers in parallel (integration and close-out code; sessions, MCP and the `R-ID-6` line in
code; requirement text; sibling items and docs; forge prior art; coordinator prior art), each returning
facts with `path:line` or URL evidence. Facts below are theirs; §6 and §7 are the synthesis. Caveats on
the evidence: the bodies of `adopt`, `claim_scan`, the `GixIsolator::reconcile` dispatcher and the
`shared_serialized` branch-move path were not read, so facts about them come from doc comments and trait
contracts; `docs/htui-mcp.md` is cited by section heading because it is not indexed for line search;
ANA-2's line references quoted in code comments have drifted (the reconciliation table now sits near
`docs/ANA-2.md:971-974`, the lease text near `:1382-1388`); the OneDev version on the maintainer's forge
could not be read anonymously; a few framework facts are recalled, not fetched, and are labelled so.

## 2. Surface as read

### 2.1 Integration today: per step, local, deterministic

- **Integration happens once per step, during the walk.** After a step reaches `done`,
  `Engine::reconcile_done_step` reads the step's `run_step_tree` rows, calls `isolator.reconcile`, and
  records the result through `store.record_commits(StepFence::Lease(owner), …)`; the winner's
  `after_hash` becomes the merge commit (`crates/htui-orch/src/engine.rs:5541-5599`). Callers: gate
  approval, fan-out selection and the judge's pick, and the walk/resume paths (`engine.rs:965, 1279,
  2586, 2770, 3277, 3800, 4765, 5017`).
- **Per mode:** for `worktree` and `copy` it is a real `git merge --no-ff` into the primary checkout; for
  `shared_serialized` and `local` it is the identity, because the step committed into the primary tree as
  it went. Only the fan-out winner is merged; the losers' `htui/<step>` labels stay. The isolator touches
  no store (`crates/htui-orch/src/isolate.rs:137-149, 225-242`).
- **`reconcile_isolated`** reads the step tip, takes the per-repo admin lock (D70/D136), refuses
  `dirty_primary_tree`, detects an already-landed merge with `merge_of`, refuses `primary_moved` when the
  base is not an ancestor of HEAD, copies objects for copy mode, then merges with retry. It merges into
  **whatever branch the primary has checked out**, not `repo.default_branch`
  (`crates/htui-orch/src/isolate/real.rs:1127-1208`).
- **Git mechanics:** a git CLI child, `git -c merge.log=false merge --no-ff --no-edit -m "htui: reconcile
  <step>" <after>`, with a fixed identity htui <htui@localhost>, `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`,
  `GIT_OPTIONAL_LOCKS=0`, stdin null and a scrubbed `GIT_*` environment; exit 0 is checked against a gix
  post-condition that HEAD's parents are `[before, after]`; gix is used only for reads
  (`crates/htui-orch/src/isolate/git.rs:109-119, 308-332, 712-795, 1586-1589`).
- **Conflicts are never resolved.** On exit 1 the conflicted paths are read, the half-merge is always
  aborted, and the result is `Refused("merge_conflict: <paths>")`; a held `index.lock` is retried 3 times
  at 200/400/800 ms; the merge runs on a detached task so a dropped caller cannot leave the primary
  half-merged (`git.rs:484-486, 690-830, 951-958`). ANA-2 §4.6 states the rule: "The orchestrator never resolves a
  conflict" (`docs/ANA-2.md:1034-1047`).
- **Every refusal parks, never fails.** The run moves `running → awaiting_approval`, the item likewise,
  an `item_note` records "reconcile refused for step <id> … the step stays `done` and the run is parked",
  and `Unblock` (`u`) retries (`engine.rs:5546-5560, 5583-5636`).
- **Run completion integrates nothing.** When the cursor answers Finished, `finish_run(run, Done)` and
  `cleanup_run(run)` run; trees are removed at run end; the item moves `in_progress → done` only when no
  other run of the item is non-terminal (`engine.rs:3000-3036, 5644-5664`;
  `crates/htui-core/src/store/traits.rs:1631-1648`). By then every step is already merged locally.

### 2.2 Close-out today: a formatter and one transaction, person-triggered

- `closeout.rs` is pure: rows in, one `NewDocument` out. `preview()` and `summary()` share one row
  builder; the summary is kind `summary`, title "Close-out <key>", `produced_by_step_id = None`, with a
  `| repo | position | phase | attempt | commits |` table of `before..after` cells and one line per run
  (`crates/htui-orch/src/closeout.rs:1-121, 140-196`).
- It is triggered only by a person: the Runs pane key `RunsCloseOut`, a `CloseOutPreview`, then
  `Command::CloseOut{item, resolution}`; the UI asks for the item key to be typed back. No run lease is
  involved (`crates/htui-orch/src/engine.rs:657-658, 1772-1808, 2504-2540`;
  `crates/htui-orch/src/command.rs:1241-1263`; `crates/htui/src/keys/catalogue.rs:604`;
  `crates/htui/src/ui/tabs/backlog/detail/runs.rs:373-403`; `crates/htui-worker/src/views.rs:31-43`).
- `WriteStore::close_out` writes the summary at its next version, upserts commits and sets the item
  `closed` with resolution and `closed_at` in one transaction; it is the only way into `closed`, governed
  by `Resolution::closes_from`, and refused while any run of the item is queued, running or
  awaiting_approval (`crates/htui-core/src/store/traits.rs:1666-1691`;
  `crates/htui-store/src/pg/write.rs:6284`; `docs/decisions/mod/mod-38.md:19-26,54-63`).
- **There is no checklist.** Close-out creates no follow-up, never touches suspect citations, and its
  only choice is the resolution picker defaulting to `Resolution::default_for(status)`. Suspect
  citations exist (`crates/htui-core/src/model/requirement.rs:139-141`) and `WriteStore::reconfirm` exists
  (`traits.rs:1812`), but close-out does not call them. MOD-98 holds that "A model never goes inside
  close-out (ANA-2 invariant 3)" (`HANDOFF.md:518-526`).

### 2.3 Push, PR, forge and credentials: absent

- Nothing pushes, fetches into a remote, opens a PR or calls a forge API. The only remote-aware git code
  reads a remote's fetch URL to match checkouts to repos, credentials stripped
  (`crates/htui-orch/src/infer.rs:105-106, 322, 335-347`). `git push` appears only as a permission-rule
  test string (`crates/htui-agent/src/permission.rs:438-444`); GitHub and GitLab names only as scrubber
  patterns (`crates/htui-core/src/scrub.rs:41-55`).
- `Repo` has `remote_url: Option<String>`, `default_branch` and `is_primary`; `remote_url` serves only
  inference, `default_branch` is unused by reconcile (`crates/htui-core/src/model/hierarchy.rs:143-148,
  168-169, 185-186`). A project has zero repos or exactly one primary (MOD-106, `HANDOFF.md:604-613`).
- `RunSecrets` holds one walk's project secrets (session env plus scrubber, zeroized on drop); git
  children get `GIT_TERMINAL_PROMPT=0` and a scrubbed env; there is no credential helper or
  `GIT_ASKPASS` wiring (`crates/htui-orch/src/secrets.rs:1-40`; `isolate/git.rs:308-332`).
- MOD-108 (3) plans the only credential path: on a failed clone, name the remote and what it needs, mark
  the repo unavailable, and offer an SSH key (public half shown) or an HTTPS token in the OS keyring
  through a credential helper, never a file; "ANA-34's push/PR credentials will reuse this"
  (`HANDOFF.md:623-632`).

### 2.4 Worker, box and lease

- `htui worker` is a long-lived headless process, one per box, sharing the TUI's `box.toml`; it sweeps
  and scans every 5 s, reads `run_command` rows every 1 s and heartbeats the box; its reply sink is
  `Unaddressed`, so its run frames reach nobody (`crates/htui-worker/src/worker.rs:13-40, 50-83`;
  `crates/htui-worker/src/address.rs:76-91`).
- Rule I-1: on a box, only the process whose role matches `executor` (`tui|worker`) claims, adopts or
  sweeps; an unknown value fails closed. A TUI on a worker box hands the lease back and the worker adopts
  within about 5 s. The worker hosts the MCP tools on a private socket. Remote targeting waits on MOD-43
  (`docs/htui-worker.md:30-79, 89-102, 116-135`).
- `sweep_once`: read executor; refresh the repo map when no walk is live; adopt dead-lease runs; run the
  queue runner `admit`; then `claim_scan` (`crates/htui-worker/src/runtime.rs:2092-2147, 2409, 2555`). The
  queue runner lives inside this sweep, not in a process of its own (`runtime.rs:2148-2280`).
- The lease: `claim_run` admits a queued run whose `target_box_id` is this box and sets
  `executing_box_id`, `lease_box_id`, `lease_owner` and `lease_expires_at`; `refresh_lease` returning
  false means abandon; `adopt_runs` takes running graph runs with a dead lease, never chat runs;
  `take_lease` precedes a command write; selectors use `executing_box_id`
  (`crates/htui-core/src/store/traits.rs:1255-1375`; `crates/htui-store/migrations/0001_init.sql:455-456`;
  `0003_orchestration.sql:41-50`). TTL 120 s, refresh 60 s, owner minted per process
  (`docs/ANA-2.md:1382-1388`).

### 2.5 Sessions, MCP and the `R-ID-6` line in code

- The agent seam is `AgentDriver::start(spec, prompt)` and `AgentSession` with `next_event`,
  `send_follow_up`, `answer_permission` and `cancel` (`crates/htui-agent/src/driver.rs:397-472`).
  `SessionSpec` ties every session to a run step and carries `resume: Option<AgentSessionRef>`
  (`driver.rs:274-305`); `DriverCaps.resume` says whether a session can be resumed (`driver.rs:355-390`).
- A free-standing chat is `run.kind='chat'`, `item_id` NULL, one step; `run.project_id` is NOT NULL, so
  **no run can be owned by a workspace** (`crates/htui-core/src/model/run.rs:13-21, 182-188, 279-310`;
  `0001_init.sql:447-451`). No item, repo, document or run table is workspace-owned; only `workspace_project` and `workspace_box_path` key on a workspace (`0001_init.sql:129-197, 310-334, 389-401,
  447-466`).
- **Chats live in the TUI and die with it.** `AgentRuntime` ("Every live chat this process owns") is
  cancelled on shutdown; a fresh chat starts with `resume: None`; the worker hosts no chat
  (`crates/htui/src/agent_worker.rs:1-16, 428-431, 1449-1471, 2492, 4751`). Promotion resumes a parked
  step's own session when `caps.resume` holds, else opens a handoff session
  (`crates/htui-orch/src/promote.rs:16-60`).
- The real Chat tab holds one live session and an optional replay; no list, no groups, digits 1-9 for
  permissions (`crates/htui/src/ui/tabs/chat/mod.rs:1-25, 76-83, 110-124, 179-183`).
- MCP: eight tools; scope always names one run and one step, built by the engine from the session token,
  never from arguments; item writes are fenced by the walk's lease; a chat without an item gets only
  `box_profile`, `search_concepts` and possibly `permission_prompt` (`crates/htui-mcp/src/tools/mod.rs:1-6,
  45-54, 76-91`; `crates/htui-orch/src/tools.rs:24-48`; `docs/htui-mcp.md` "Which session gets which
  tools", "Scope: what a session can touch").
- **The code already splits "an agent asks, deterministic code or a person does".** `item_status` writes a
  note "status request: …" and moves nothing (`crates/htui-mcp/src/tools/status.rs:1-7, 49-55`); only a
  person closes an item (§2.2); requirement writes and re-confirmations are person-gated TUI requests,
  and `item_requirement.proposed_by_step_id` NULL marks a human (`crates/htui/src/store_worker.rs:989-1069`;
  `0006_requirements.sql:115-126`). `item_create`, `spawn_subagent`, the ask-the-user tool (MOD-75) and the
  planner tools of `R-MCP-2` have no code (`docs/REQUIREMENTS.md:514-519`).
- Cross-process talk exists through Postgres: `step_permission` rows parked by the lease holder and
  answered by any client (MOD-42), and `run_command` rows of kind `cancel` and `follow_up` (MOD-70)
  (`crates/htui-core/src/model/relay.rs:13-36`; `traits.rs:1819-1929`;
  `0011_permission_relay.sql:14,47`; `0017_follow_up.sql:21-23`).

### 2.6 Events a coordinator could listen to

- **There is no LISTEN/NOTIFY and no cross-process event bus**: zero hits for `PgListener` or `pg_notify`
  in `crates/`, and `LISTEN` only in one comment naming MOD-43's plan. Processes poll: worker sweep 5 s, command poll 1 s, MCP `command_run` 1 s, cache
  refresher 30 s (`crates/htui-store/src/cache/refresh.rs:1-60, 100-125, 190`;
  `crates/htui-mcp/src/tools/command.rs:40`). MOD-43 plans LISTEN/NOTIFY hints with a poll backstop
  (`HANDOFF.md:332-339`).
- `RunFrame{item, run, kind}` (`Started`, `SessionDone`, `Rested`, `Changed`, `Waiting`, `Adopted`,
  `Error`) is in-process only (`crates/htui-worker/src/views.rs:123-172, 938-984`). `DriverEvent`'s 12
  variants persist as `session_event` rows (`crates/htui-agent/src/event.rs:222-276`).
- Durable signals a poller can read: run status reaching a terminal state (`finish_run`), item status,
  `item_note` rows, queue `EntryState`/`Escalation` (`HardGateParked`, `JudgeUndecided`,
  `ReviewLoopExhausted`, `Blocked`, `Failed`, `MissingTags`; `crates/htui-core/src/model/queue.rs`) and
  `run_command` rows.
- The waiting list has six reasons (`Gate`, `JudgeFailed`, `Selection`, `Unblock`, `Interrupted`,
  `Permission`) and **every `WaitingRow` requires an item** (`crates/htui-worker/src/views.rs:482-562`);
  chat-run permissions are never listed there (`traits.rs:2131-2144`). The MOD-115 header shows "N working
  · M waiting" from it (`crates/htui/src/ui/header.rs:1-31, 151, 182-203`).
- The only grouping above runs is `queue_batch`, per box (`0016_auto_queue.sql:1-40`). `item_link.kind`
  has no `parent`, and no run has a parent run (`crates/htui-core/src/model/link.rs:10-22`).

### 2.7 Requirement text where it matters

- `R-ORCH-14`: "A coordinator only manages: it follows what happens below it, talks with its children and
  its parent, and integrates a child's work when the child's run completes — merging, pushing or opening a
  pull request as the project configures — and runs close-out (R-TUI-9). It does no item work itself."
  (`docs/REQUIREMENTS.md:426-431`).
- `R-TUI-9`: "Close-out writes the final summary document, sets status, records commit hashes from the
  run. No markdown files are produced. The coordinator (R-ORCH-14) runs close-out and its checklist —
  follow-up items, re-confirming suspect citations, choosing the resolution — and the merge, push or pull
  request; ANA-34 settles the split. Amended 2026-10-15." (`docs/REQUIREMENTS.md:601-604`).
- `R-ID-6`: "No LLM agent is ever in a sync, cache, import or bookkeeping path. … An agent choosing *which
  work to do next* (R-ORCH-1's planned graphs, R-ORCH-14's coordinator) is item work, not bookkeeping; the
  status transitions it causes stay deterministic." Drafts a person accepts are outside the ban "as long
  as no status transition, link, import, cache refresh or prompt ever reads the unaccepted model text"
  (`docs/REQUIREMENTS.md:113-123`).
- `R-ENT-8`: "Transitions are driven by the orchestrator, the coordinator (R-ORCH-14) and close-out, not
  edited by hand." Status is `open|blocked|closed` with run-owned progress (`docs/REQUIREMENTS.md:199-203`);
  the code still has eight statuses (`crates/htui-core/src/model/item.rs:8-66`; migration in ANA-35).
- `R-ORCH-7`: "an integrate step then merges the parts' branches and artifacts, resolving conflicts, before
  review" (`docs/REQUIREMENTS.md:404-409`); the review calls that step "an agent"
  (`docs/requirements-review-2026-10-15.md:147-155`).
- `R-ENT-15`: a citation records the requirement version; "a newer version marks it suspect until
  re-confirmed"; an agent may propose splits "as drafts a person accepts" (`docs/REQUIREMENTS.md:233-239`).
  `R-ENT-14`: "coordinators and agents propose changes as drafts a person accepts" (`:228-229`).
- `R-BOX-4`: credential guidance for clone (SSH key, or an HTTPS token in the OS keyring "for git to use,
  never in a file") (`:148-153`). `R-SEC-2`: "`htui`'s own credentials (R-STO-1) are never exposed to a
  session" (`:494-496`). `R-STO-1`: keyring only; headless worker exception is a systemd credential or
  `--dsn-stdin` (`:257-267`). `R-SEC-4`: no plaintext fallback (`:501-502`).
- The review asked this ANA to name the requirements it changes: "at least R-ID-6, R-ORCH-1/7, R-TUI-9 and
  R-ENT-8" (`docs/requirements-review-2026-10-15.md:241-242`); `R-ORCH-14` names R-ID-6, R-ORCH-7, R-ENT-8 and
  R-TUI-9.

## 3. Constraints from sibling items

1. **ANA-2 invariants.** Every status move is a compare-and-set with a single writer (invariant 1); no
   agent in a bookkeeping path, close-out included, the judge being the only exception (invariant 3);
   htui writes no files into a managed repo (invariant 4, `R-ID-4`); resume is decided from the store,
   with no cross-restart memory (invariant 9) (`docs/ANA-2.md:103-142`). §4.6: clean-primary check,
   `--no-ff`, park on conflict, never resolve (`:1034-1047`). §4.10: close-out is one transaction and
   refused while a run is live; "watch items for later modules" had no home (`:1486-1503`).
2. **ANA-33 (unwritten).** Owns the run graph, the planner's MCP tools, the part-to-`run_step` mapping
   and "how the integrate step relates to ANA-2 §4.6's reconciliation, where the orchestrator never
   resolves a conflict"; graphs may be workspace-owned and run several items as subgraphs; ships in
   version one (`HANDOFF.md:105-120`). ANA-40 already gave ANA-33 the run graph and the integrate step
   (`docs/ANA-40.md:149-159`).
3. **ANA-41** (project workflows that order a project's items) is blocked on this item and wanted later
   (`HANDOFF.md:178-183`). ANA-34 must not absorb "which item next".
4. **ANA-38** reopens project and workspace memory "for the coordinator" (`HANDOFF.md:160-166`;
   `R-ENT-13`, `docs/REQUIREMENTS.md:221-225`). ANA-34 leaves a hook, not a design.
5. **ANA-42 (drafted).** The executor stays on this box for driven targets; `target_box_id` is where the
   work runs, `executing_box_id` who drives it (§7.2, `docs/ANA-42.md:245-252`). Constraint 1: htui's own
   credentials never reach a target. Constraint 6: nothing is pushed from a target (`:165-179`). §7.5:
   collect by bundle, and "Merging stays the person's step" with the config-path diffstat confirmation
   (`:274-282`). §7.6: clones with `pushurl` disabled; git credentials on a remote are MOD-108's.
6. **MOD-121** builds the Chat groups and is blocked on MOD-115; "coordinators need ANA-34"
   (`HANDOFF.md:769-776`).
7. **MOD-41/43/47.** The worker exists (MOD-41); remote targeting and LISTEN/NOTIFY hints are MOD-43; the
   `htui` server is trigger-gated phase 2 (`HANDOFF.md:332-339, 370-386`).
8. **Secrets.** The Infisical identity lives in the OS keyring under service `htui`; secrets apply only to
   the agent's env and never to htui's own env (`docs/htui-secrets.md:53-76, 234-289`). A headless worker
   without a keyring cannot hold an identity today (MOD-48, MOD-104; `HANDOFF.md:387-394, 583-588`). The
   worker reads its DSN from `--dsn-stdin`, the keyring, or the systemd credential `htui-dsn`
   (`docs/htui-worker.md:380-398`).
9. **`scripts/hr collect`** is the proven human-driven integrator: temp ref, fsck, no hooks, refuses
   rewritten history, `--merge` is `--no-ff` with a validator and a config-path diffstat confirmation, and
   it "never pushes"; "Push when you are satisfied, as always." Mechanical HANDOFF conflicts may get a
   primed host session the person approves; code conflicts are left to the person
   (`docs/hr-sandbox.md:298-363`). Sandboxes mount no git credentials (`:12, 470, 558`).
10. **MOD-125** stops and resumes every run in a workspace (`HANDOFF.md:796-803`); a coordinator's state
    must survive that, which store-held state does.

## 4. Prior art

| System | What integrates | Agent or deterministic | Credentials | Source |
|---|---|---|---|---|
| GitHub merge queue | queued PRs, tested on `gh-readonly-queue/{base}` with those ahead | deterministic | platform | https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges/managing-a-merge-queue |
| GitLab merge trains | MRs on top of those ahead; failures dropped, later ones restarted | deterministic | platform | https://docs.gitlab.com/ci/pipelines/merge_trains/ |
| bors-ng (archived 2024) | batches on a staging branch, fast-forwards main to the tested commit | deterministic | GitHub App | https://github.com/bors-ng/bors-ng |
| Mergify | updates each PR on main, re-runs CI, merges | deterministic | GitHub App | https://docs.mergify.com/merge-queue/ |
| OneDev auto-merge, branch protection | PR merges when reviews and required builds pass; strict builds invalidate on target update | deterministic | user access token, restrictable per project and role, optional expiry | https://docs.onedev.io/tutorials/code/pullrequest-approval ; https://docs.onedev.io/tutorials/security/manage-access-tokens |
| OneDev TOD skills | `submit-issue-work` pushes and opens or updates the PR; "work" and "submit" skills kept apart so a person reviews before a push | agent invokes a CLI on a person's request | token in a 0600 INI or env vars | https://github.com/theonedev/tod ; https://onedev.io/blogs/onedev-cli-and-agent-skills |
| GitHub Copilot coding agent | pushes only to its own `copilot/` branch, opens a draft PR; cannot run `git push`, approve or merge; the requester cannot approve | agent produces, platform pushes, person merges | platform-held | https://docs.github.com/en/copilot/concepts/agents/coding-agent/risks-and-mitigations |
| Claude Code GitHub Action | commits to a new branch and posts a link; the person opens the PR | agent commits, person integrates | short-lived app token; static PATs warned against | https://code.claude.com/docs/en/github-actions |
| Codex cloud | person reviews the diff, then opens a PR | person | secrets removed before the agent phase | https://learn.chatgpt.com/docs/cloud |
| Jules | person clicks "Create branch", opens the PR | person (agent opens PRs only for labelled issues) | GitHub app | https://jules.google/docs/running-tasks/ |
| Devin | opens PRs; docs imply it can merge behind branch protection | agent | org-wide GitHub App | https://docs.devin.ai/integrations/gh |
| OpenHands resolver | draft PR on success, branch on failure | deterministic action around an agent | PAT or `github.token` | https://pypi.org/project/openhands-resolver/0.3.0 |
| Claude Code agent teams | nothing; advises splitting file ownership; a teammate cannot give consent for the user | LLM lead, fixed for its lifetime, not restored on `/resume` | n/a | https://code.claude.com/docs/en/agent-teams |
| LangGraph supervisor; Magentic-One | synthesises workers' results; ledgers of facts, plan and progress, replan on stall | LLM supervisor | n/a | https://docs.langchain.com/oss/python/langchain/supervisor ; https://www.microsoft.com/en-us/research/articles/magentic-one-a-generalist-multi-agent-system-for-solving-complex-tasks/ |
| Temporal agents | workflow code deterministic, LLM calls in Activities; signals wake a waiting workflow at no compute | hybrid | n/a | https://temporal.io/blog/of-course-you-can-build-dynamic-ai-agents-with-temporal ; https://docs.temporal.io/ai-cookbook/human-in-the-loop-python |
| Inngest AgentKit; OpenAI Agents SDK | code routers first, agent routing added; "orchestrating via code … more deterministic and predictable" | hybrid by recommendation | n/a | https://agentkit.inngest.com/concepts/routers ; https://openai.github.io/openai-agents-python/multi_agent/ |

Lessons that bind:

- **Every surveyed harness keeps the push and the merge out of the agent's hands.** The agent produces
  commits on its own branch; a deterministic actor holding the credential pushes, opens the PR and gates
  the merge (Copilot, Claude's action, Codex, Jules, OneDev's work/submit split). Merge queues are fully
  deterministic. No published design splits a coordinator into a deterministic integrator plus an LLM
  checklist; that split is this item's inference from the router and orchestration guidance.
- **Credentials stay out of the agent's reach**: Codex removes secrets before the agent phase, Claude's
  action warns that a static PAT "could be recovered via prompt injection".
- **Always-on LLM coordinators are costly and fragile.** Agent teams use about 7x a session's tokens, an
  idle wake after the cache TTL reprocesses the whole context, a lead "can stop early", and teams are not
  restored on `/resume` (https://code.claude.com/docs/en/costs ; agent-teams page). The robust pattern is
  "capture results as application state and pass them into a fresh session's prompt"
  (https://code.claude.com/docs/en/agent-sdk/sessions), with a deterministic process that sleeps between
  events (Temporal `wait_condition`, Inngest `waitForEvent`, Airflow's triggerer).
- **Forges differ in how a PR is opened.** The maintainer's forge, `git.mluigi.it`, runs **OneDev**
  (probed 2026-10-18: login page meta `Author=onedev.io`, API under `/~api/`), not Gitea or Forgejo. OneDev
  opens PRs by REST: `POST /pulls` with `PullRequestOpenData {targetProjectId, sourceProjectId,
  targetBranch, sourceBranch, title, workInProgress, description, mergeStrategy, reviewerIds, …}`, plus
  `POST /pulls/{id}/auto-merge` (https://github.com/theonedev/onedev/blob/main/server-core/src/main/java/io/onedev/server/rest/resource/PullRequestResource.java).
  No OneDev push-to-create was found (absence of evidence; the OneDev tracker was not searched). Forgejo
  and Gitea open PRs on push through AGit `refs/for/<branch>` with `-o topic=` (https://forgejo.org/docs/latest/user/agit-support/ ;
  https://docs.gitea.com/usage/agit); GitLab through `-o merge_request.create`
  (https://docs.gitlab.com/topics/git/commit/#push-options-for-merge-requests); GitHub only by REST or `gh`
  (https://docs.github.com/en/rest/pulls/pulls). Every forge can **merge on green server-side** (OneDev
  auto-merge, Forgejo `merge_when_checks_succeed`, GitLab `auto_merge`, GitHub auto-merge or queue), so a
  coordinator need not poll CI to merge.
- **Credential shapes.** GitHub App installation tokens expire in an hour and narrow per repo; OneDev,
  Forgejo and Gitea offer only user-bound tokens with optional expiry. Git credential helpers speak
  get/store/erase on stdin/stdout and scope per URL with `credential.<url>.helper` and `useHttpPath`
  (https://git-scm.com/docs/gitcredentials). On headless Linux, GCM's secret-service store needs a D-Bus
  session and an unlocked collection (https://github.com/git-ecosystem/git-credential-manager/blob/main/docs/credstores.md).
  OpenSSH warns that agent forwarding lets anyone reaching the socket use the keys
  (https://man.openbsd.org/ssh_config#ForwardAgent). Deploy keys can push but cannot open PRs.

## 5. Options per question

### 5.1 Agent, deterministic process, or both; where `R-ID-6` draws the line

- **A. Agent session.** The coordinator is a long-lived LLM session (the skeleton's mock) that calls tools
  to merge, push, open PRs and close out. Matches the UI's chat; but puts a model in front of status
  transitions and merges, against `R-ID-6`, ANA-2 invariant 3 and MOD-98, and the push credential would sit
  next to a prompt-injectable context.
- **B. Deterministic process.** No model at all: integration on child completion, a checklist assembled
  from rows (suspect citations, `default_for(status)`), messages unsupported. Safe and cheap; fails
  `R-TUI-6` ("Coordinators always take messages") and leaves follow-up drafting to the person.
- **C. Hybrid: deterministic core, optional agent turns that only propose.** The core does every write
  (merge, push, PR call, status, resolution, citation re-stamp, summary). The agent part answers messages
  and fills checklist proposals as drafts a person accepts, and may invoke only deterministic commands
  that already carry their own gate (an enqueue under a permission card). The line: **the agent chooses
  and drafts; deterministic code disposes; nothing irreversible runs on unaccepted model text.**

### 5.2 Lifetime and box

- **A. One always-on process per project and workspace**, with its own lease. A clean "coordinator box",
  but a project whose repos sit on several boxes needs a designated box, a new supervisor and a new
  lease, and the integration still needs the checkout the run used.
- **B. A role, carried out where the rows are.** No coordinator process: the deterministic part is one
  more arm of the existing executor sweep (I-1), acting on the run's executing box, which already holds
  the checkout, the repo map and the credentials; its state is rows (invariant 9). Agent turns are
  short sessions started on demand.
- **C. An always-on agent session per project and workspace, hosted by the worker.** What the skeleton's
  "the window closes, the work keeps running" suggests. Costly (§4), needs worker-hosted chats that do not
  exist, and a dropped session loses context unless it is reseeded anyway.

### 5.3 Relation to ANA-33's planner and integrate step, and to step reconcile

- **A. Keep the per-step local reconcile; the coordinator only publishes** (push the primary branch, or
  open a PR from it) after the run completes. Minimal; but a PR from the branch the steps were merged
  into is meaningless when that branch is the target (`main`).
- **B. Mode-dependent landing branch.** In `local` and `push` modes step reconcile is unchanged; in
  `pull_request` mode, `worktree` and `copy` steps reconcile into a **run branch** (`htui/item/<key>`) in
  the primary repo instead of the checked-out HEAD, and the coordinator pushes that branch and opens the
  PR when the run completes. ANA-33's integrate step stays inside the run and combines its parts.
- **C. Defer all reconcile to run end.** One merge per run; but later phases stop seeing earlier phases'
  commits (ANA-2 §4.6 branches from HEAD so they do) and it breaks the D136 `merge_of` recovery and the
  engine tests that pin it.

On conflicts, two sub-options: **(i)** the coordinator parks and a person resolves, possibly through a
follow-up item the coordinator drafts (item work by a child run); **(ii)** the coordinator runs an agent
to resolve. (ii) makes the coordinator do item work, which `R-ORCH-14` forbids.

### 5.4 Push and PR configuration

- **A. Project setting only**: one `integrate` mode for the whole project. Simple; wrong for a project
  whose repos live on different remotes or forges.
- **B. Per-repo setting with a project default**: `integrate` = `local | push | pull_request`, plus
  remote (default `repo.remote_url`), target branch (default `repo.default_branch`), forge kind and base
  URL, draft, merge strategy, auto-merge, reviewers. Fields map one-to-one onto OneDev's
  `PullRequestOpenData` and `AutoMergeData`, and onto GitHub, Forgejo and GitLab equivalents.
- **C. Push options only** (AGit, GitLab push options), no forge API. No token beyond git's; but OneDev and
  GitHub cannot open PRs this way, so it fails the maintainer's own forge.

### 5.5 Credentials

- **A. Agent-held**: the coordinator agent gets a token in its env and uses `tod`/`gh`. Simplest; breaks
  `R-SEC-2` ("`htui`'s own credentials … never exposed to a session") and the prior-art lesson.
- **B. Executing box's OS keyring, used only by deterministic code.** The git transport credential is
  MOD-108's (SSH key, or HTTPS token through an htui credential helper scoped per URL); the forge API token
  is a new keyring entry per forge base URL under service `htui`. Never in argv, a file, htui's own env, or
  any agent env; scrubbed if it ever appears. Missing credential: integration parks with a typed refusal
  naming the remote and the credential; no fallback.
- **C. ssh-agent forwarding** from the person's machine. Nothing stored; but it cannot serve unattended
  runs and hands the keys to anyone reaching the socket.

For a headless worker with no keyring: **(i)** extend `R-STO-1`'s systemd-credential exception to push and
forge credentials, or **(ii)** refuse integration on such boxes.

### 5.6 The close-out checklist

- **A. Coordinator closes automatically** when the last run is done and integrated. Fast; but choosing a
  resolution and re-confirming citations are judgements, and `R-ID-6` forbids writing unaccepted model
  text.
- **B. Person confirms over a prepared checklist.** When an item's runs are terminal and its work landed
  (or was refused), the core assembles a checklist: integration outcome, proposed resolution, suspect
  citations with re-confirm or amend proposals, follow-up drafts. A person works through it and confirms;
  the existing `close_out` transaction writes. A new waiting reason carries it.
- **C. Today's flow plus a separate follow-ups screen.** Least change; keeps the checklist outside the
  waiting list, so it is never "waiting on you".

### 5.7 The workspace coordinator

- **A. Full peer**: integrates project coordinators' work (cross-project, multi-repo landing). No
  mechanism exists; ANA-2's reconciliation is per (step, repo).
- **B. Follower and relay in version one**: aggregates its projects' waiting rows and integration states,
  takes messages, routes them to project coordinators, proposes cross-project drafts (workspace
  requirement splits under `R-ENT-15`); integrates nothing of its own. Cross-project landing waits for
  ANA-41.

### 5.8 The final text of `R-TUI-9`

- **A. Close-out keeps everything** (summary, status, hashes, merge, push, PR, checklist). Conflates
  "land the work", which happens per run, with "end the item", which happens once.
- **B. Split**: integration is the coordinator's per-run action under `R-ORCH-14`; close-out ends the item,
  records the integration outcome, and is a person's confirmation over the coordinator's checklist.

## 6. Analysis

**The code already draws the `R-ID-6` line the hybrid needs.** Every write that moves state is
deterministic and fenced: merges are a git child with post-conditions (§2.1), close-out is one
transaction a person triggers (§2.2), `item_status` from an agent is a note, not a move (§2.5),
requirement changes and re-confirmations are person-gated. Adding a coordinator does not need a new
principle; it needs the same split applied to two new writes (push, PR) and one new screen (the
checklist). Option 5.1 A would undo all of this, and 5.1 B cannot satisfy `R-TUI-6`. The prior art
agrees without exception (§4): the agent produces, the deterministic actor publishes, a person or the
forge's rules merge.

**Integration "when the child's run completes" is mostly a publish step.** Because each step already
lands locally while the run walks (§2.1), the only thing left at run completion is to push, or to open a
PR. That makes `local` mode exactly today's behaviour and lets it stay the default with zero migration.
The only mode that needs a different landing branch is `pull_request`, and only for isolating modes
(`worktree`, `copy`); `local` and `shared_serialized` commit straight into the person's checked-out tree,
so htui cannot put them on a separate branch without switching the person's branch, and they support
`local` and `push` only. Option 5.3 B is therefore the smallest change that makes all three modes
meaningful. It keeps reconcile's discipline (admin lock, clean-primary, `--no-ff`, abort on conflict,
park and note) and its tests; the run branch is just a different target ref, created from the primary's
HEAD at run start.

**Two levels of integration, never two owners of one merge.** ANA-33's integrate step works inside one
run: it combines the parts of a divided step, and `R-ORCH-7` says it resolves conflicts, which is item
work, so it may be an agent. The coordinator works across runs: it lands a finished run on the configured
branch, remote or forge, and it never resolves a conflict, both because `R-ORCH-14` says it does no item
work and because ANA-2 §4.6 never does. A conflict at the coordinator's level parks the run with the
existing `merge_conflict` refusal and becomes a waiting row; the coordinator may draft a follow-up item
("resolve the conflict between X and main"), which a person accepts and a child run then does. The planner
of ANA-33 decides what work a run contains; the coordinator does not plan.

**A process per coordinator buys nothing the executor does not already give.** The executor sweep
(§2.4) is long-lived, lease-fenced, I-1-governed, already hosts the queue runner and MCP tools, and sits
on the box that has the run's checkout. Integration needs exactly that box: the merge needs the primary
checkout locally, and pushing from a target is forbidden (ANA-42 constraint 6). Making the coordinator a
role over rows (5.2 B) means no new supervisor, no designated coordinator box, crash recovery by the
existing adopt pattern, and survival of TUI exit and MOD-125's workspace stop for free. A per-run
`integration` record with its own lease columns, written by `finish_run(Done)` in the same transaction,
gives the sweep its work queue and makes the step idempotent across crashes (check "already pushed" by
comparing the remote ref, "already opened" by the stored PR reference), which the prior art on
resumable side effects requires (LangGraph re-runs a node on resume; OpenAI warns on replayed approvals).
Polling at the sweep's 5 s cadence is enough; MOD-43's LISTEN/NOTIFY hints can shorten it later.

**The agent part should be turns, not a resident.** An always-on session that "listens" to every event
grows its context with each one and pays the full context on every idle wake (§4); agent teams are not
even restorable. The store already is the ledger Magentic-One keeps in context: run statuses,
escalations, notes, integration records, the checklist. So a coordinator turn is a fresh or resumed
session seeded from those rows, opened when a person writes to it or when the core wants proposals for a
checklist, and ended when it answers. "Always take messages" (`R-TUI-6`) then means a message always
opens a turn, and the conversation's transcript persists as `session_event` rows like any chat. Two
schema gaps follow and are owed regardless of hosting: a workspace coordinator has no row to record
against (`run.project_id` NOT NULL, §2.5), and neither `ToolScope` nor `WaitingRow` can express a scope
without an item (§2.5, §2.6).

**Credentials must stay with the integrator.** `R-SEC-2` already forbids exposing htui's own credentials
to a session, and a push or forge token is htui's own credential, not a project secret. Placing it in the
executing box's keyring, read only by the deterministic integrator, and handing it to git through a
credential helper over stdin (never `-c http.extraHeader=…` in argv, as TOD does) keeps it out of argv,
files and every agent environment. MOD-108 already plans the helper for clone; push reuses it. The forge
API token is new and needs `R-BOX-4` to name it. The headless worker is the hard case: without a keyring
it cannot hold a token, the same gap MOD-48 has for Infisical. Extending `R-STO-1`'s systemd-credential
exception is the consistent answer, since the DSN already uses it.

**OneDev decides the first forge.** A design built on AGit push options would fail the maintainer's own
remote. A small forge adapter with four verbs (push a branch, open or update a PR, set auto-merge, read PR
state) covers OneDev, GitHub, Forgejo/Gitea and GitLab; OneDev first, GitHub second, the others later.
Delegating "merge when green" to the forge's auto-merge keeps the coordinator from polling CI, and the
forge's own review rules (a submitter cannot approve their own PR) keep a person in the merge path where
the project wants one.

**Close-out is where judgement meets bookkeeping, so it stays a person's confirmation.** The checklist's
structure is deterministic (which citations are suspect, which runs landed, the default resolution);
its content may be proposed by an agent turn (follow-ups, a rationale for a resolution, a re-confirm or
amend proposal). Writing any of it without a person violates `R-ID-6`'s "a draft a person accepts".
Auto mode does not change this: auto mode runs work unattended, it does not close items. Requiring
`done` to wait for landed work makes the resolution honest in `pull_request` mode, where a run can be
done while its PR is still open.

**The workspace coordinator has nothing to integrate yet.** A workspace owns no repo, item or run
(§2.5); cross-project landing (several per-repo PRs merging together) has no mechanism and is the subject
of ANA-41's project workflows. Version one gives it the follower role the skeleton shows ("what should I
look at first today?", `spikes/ui-skeleton/src/data.rs:720-756`) and leaves integration of its own for
later.

## 7. Verdict

**7.1 Hybrid, with the line drawn at "propose versus dispose" (5.1 C).** A coordinator has a
deterministic core and an optional agent part. The core does every write: landing a run's work (merge,
push, PR call, auto-merge request), recording the outcome, assembling the close-out checklist, and,
on a person's confirmation, the existing `close_out` transaction, the follow-up items and the citation
re-confirmations. The agent part only proposes: follow-up item drafts, a resolution with its rationale,
re-confirm or amend proposals for suspect citations, a PR title and description, and queue or link
requests. Proposals are stored as drafts no transition, link, prompt or import reads until a person
accepts them. The agent may invoke a deterministic command only where that command already carries a
gate (an enqueue or a `blocked_by` link through a permission card); choosing which work to do next stays
"item work" under `R-ID-6`, and its autonomy beyond that is ANA-41's. The agent never holds a push or
forge credential, never merges, never resolves a conflict and never closes an item. A project with no
coordinator agent configured gets the deterministic checklist alone.

**7.2 A coordinator is a role over store rows; its core runs in the existing executor (5.2 B).**
There is no coordinator process and no coordinator box. When `finish_run(Done)` completes a run whose
repos are configured for `push` or `pull_request`, the same transaction writes one **integration record
per (run, repo)**: mode, remote, target branch and forge as snapshotted at run start (`R-ORCH-11`,
invariant 2), state (`pending`, `landed`, `pr_open`, `pr_merged`, `pr_closed`, `refused`), the pushed
commit or PR reference and URL, and lease columns on the `claim_run` pattern. A new **integrate arm of
`sweep_once`**, in the process whose role matches `executor` on the run's executing box (I-1), claims
pending records, performs them idempotently, and polls open PRs' state at the sweep cadence. A refusal
(conflict, non-fast-forward, missing credential, forge error) parks with a typed reason and an
`item_note`, exactly as reconcile does, and appears as a waiting row; `Unblock` retries. In `local` mode
no record is needed beyond today's reconcile; the record exists so the checklist can say "landed".
Agent turns are short: opened by a person's message (hosted by the TUI that sends it, today's chat path)
or by the core when a checklist wants proposals (hosted by the executor, under a lease, like a graph
step), seeded from the store's rows and, where `DriverCaps.resume` holds, resuming the previous turn's
session. Coordinator turns are recorded as runs of a new kind `coordinator` with no item; a workspace
coordinator's runs need a workspace owner, so `run` gains a workspace scope (nullable `project_id` with a
`workspace_id` and a check that exactly one is set, or an equivalent the plan chooses). ANA-38's memory,
when it lands, is added to the seed.

**7.3 Two levels of integration; the landing branch depends on the mode (5.3 B, conflicts (i)).**

| Mode | Step reconcile (ANA-2 §4.6) | When the run completes | Isolation modes allowed |
|---|---|---|---|
| `local` (default) | into the primary's checked-out branch, unchanged | nothing more | all |
| `push` | into the primary's checked-out branch, which must be the configured target branch, else `primary_moved`-style refusal | fast-forward push of the target branch to the remote; never force; non-fast-forward parks | all |
| `pull_request` | into a run branch `htui/item/<key>` cut from the primary's HEAD at run start | push the run branch, open or update the PR (draft and auto-merge as configured), then follow its state | `worktree`, `copy` only |

ANA-33's integrate step combines a divided step's parts inside one run and may resolve conflicts (item
work, an agent); its output enters step reconcile like any other step. The coordinator lands finished
runs and **never resolves a conflict**: a conflict or a rejected push parks the run and becomes a waiting
row, and the coordinator may draft a follow-up item to resolve it, done by a child run once a person
accepts it. Several runs landing on one target are serialised by the existing per-repo admin lock; a local
merge train is not built, and PR mode relies on the forge's required checks and auto-merge (OneDev's
strict pull-request builds, GitHub's queue). For an off-box target (`R-ORCH-15`, ANA-42 §7.5), collect
brings the branch home under its own discipline and its config-path diffstat confirmation is the
person's gate; the coordinator then integrates per mode. ANA-42's "merging stays the person's step"
holds for `local` mode and is narrowed accordingly (§10).

**7.4 Push and PR configuration is per repo with a project default (5.4 B).** Fields: `integrate`
(`local | push | pull_request`), remote (default `repo.remote_url`), target branch (default
`repo.default_branch`), forge kind (`onedev | github | forgejo | gitea | gitlab`) and base URL, draft,
merge strategy, auto-merge, reviewers. Values are snapshotted at run start, so a mid-run change never moves
where a run lands. A repo-less or plain-folder project, or a repo without a remote, accepts `local` only;
the Settings form refuses the others with the reason. The forge enters through a **forge adapter** with
four verbs: push a branch, open or update a PR, set auto-merge, read PR state. Implementations ship
OneDev REST first (`/~api/pulls`, `/~api/pulls/{id}/auto-merge`), GitHub REST second; Forgejo/Gitea
(REST or AGit) and GitLab (REST or push options) after, behind the same trait. Pushes are plain `git push`
children with the same scrubbed environment as reconcile; force is never used.

**7.5 Credentials live in the executing box's keyring and reach only deterministic code (5.5 B, (i)).**
The git transport credential is MOD-108's: an SSH key, or an HTTPS token used through an htui credential
helper scoped per remote URL (`credential.<url>.helper`, `useHttpPath`), passed over the helper's stdin.
The forge API token is a new keyring entry per forge base URL under service `htui` (`R-STO-1`). Neither
is ever placed in argv, a file, htui's own environment or any agent or target environment; both are
added to the scrubber's exact-match masks. A missing or refused credential parks the integration with a
typed refusal that names the remote, the credential it needs and how to provide it (`R-BOX-4`'s
guidance); there is no fallback (`R-SEC-4`'s posture). A headless worker without a keyring reads them
from systemd credentials, the same exception the DSN has; elsewhere it refuses. Recommended token scope
is least privilege (OneDev: owner permissions off, the project's code-write role, an expiry), recorded as
guidance, not enforced. ssh-agent forwarding is not used for integration.

**7.6 Close-out stays a person's confirmation over the coordinator's checklist (5.6 B).** When an item
has no active run and each of its runs' integration records is terminal, the core opens a **close-out
checklist** and a new waiting reason, `CloseOut`, for the item. The checklist shows: each run's
integration outcome (merge commit, pushed commit, or PR reference, URL and state); the proposed
resolution (the coordinator agent's, else `Resolution::default_for(status)`); every suspect citation with
re-confirm, amend-draft or leave choices (through the existing `reconfirm`); and follow-up item drafts.
The person works through it and confirms with today's typed-key confirmation; the existing
`close_out` transaction writes the summary, now with an integration section beside the commit table,
sets `closed` and the resolution, and in the same request the accepted follow-ups are created as items
and the accepted re-confirmations are written. Resolution `done` (and `concluded`) requires every
integration record to be `landed` or `pr_merged`; the other resolutions do not. Auto mode stops at this
waiting row; it never closes an item. ANA-2 §4.10's homeless "watch items for later modules" note gets
its home as a follow-up draft.

**7.7 The workspace coordinator follows and relays in version one (5.7 B).** It aggregates its
projects' waiting rows and integration states, takes messages and routes them to a project coordinator,
and proposes cross-project drafts (a workspace requirement split under `R-ENT-15`, a follow-up in a
sibling project). It integrates nothing of its own; landing several projects' work together is left to
ANA-41. Its "children" are the project coordinators; a project coordinator's "children" are the item
runs of its project, recorded by `run.project_id` today, with no `parent` link needed for version one.

**7.8 `R-TUI-9` is split from integration (5.8 B).** Integration is the coordinator's per-run action
under `R-ORCH-14`; close-out ends the item, records what was integrated, and is confirmed by a person over
the coordinator's checklist. The proposed final text is in §10.1.

## 8. Questions for the maintainer

1. **Hybrid coordinator (7.1):** deterministic core for every write, optional agent turns that only
   propose; the agent never holds a credential, merges, resolves a conflict or closes an item.
   **Recommended: accept.**
2. **A role, not a process (7.2):** the core is an integrate arm of the existing executor sweep on the
   run's executing box, driven by per-(run, repo) integration records with their own lease; no
   coordinator box. **Recommended: accept.**
3. **Agent turns on demand, not a resident session (7.2):** a message or a checklist opens a turn seeded
   from the store; the transcript persists; "always take messages" (`R-TUI-6`) means a message always
   opens a turn. **Recommended: accept.**
4. **Three modes, `local` the default (7.3):** `local` is today's behaviour unchanged; `push` is
   fast-forward only, never force; `pull_request` lands steps on `htui/item/<key>` and is allowed only for
   `worktree` and `copy`. **Recommended: accept.**
5. **Two levels of integration (7.3):** ANA-33's integrate step may resolve conflicts inside a run; the
   coordinator never does and drafts a follow-up instead. ANA-33 is asked to adopt this split.
   **Recommended: accept.**
6. **Per-repo configuration and a forge adapter, OneDev first, GitHub second (7.4).** **Recommended:
   accept.**
7. **Credentials (7.5):** git and forge credentials in the executing box's keyring, used only by
   deterministic code through a credential helper, scrubbed, no fallback; extend `R-STO-1`'s
   systemd-credential exception to them for headless workers. **Recommended: accept, including the
   exception.**
8. **Close-out (7.6):** a person always confirms, also in auto mode; a `CloseOut` waiting reason carries
   the checklist; `done` requires the work to have landed. **Recommended: accept.**
9. **Off-box targets (7.3):** after ANA-42's collect, whose diffstat confirmation stays the person's gate,
   the coordinator integrates per mode; ANA-42 §7.5's "merging stays the person's step" is narrowed to
   `local` mode. **Recommended: accept.**
10. **Workspace coordinator (7.7):** follows and relays in version one, integrates nothing; cross-project
    landing goes to ANA-41. **Recommended: accept.**
11. **Requirement edits (§10)**, including the final `R-TUI-9` text in §10.1. **Recommended: accept as
    written, the wording open to the maintainer's edits.**

## 9. Proposed spawns

IDs are minted after acceptance; ordering is by dependency.

| P | Subject | Scope | Blocked on | Requirements |
|---|---|---|---|---|
| P1 | Integration records and modes | Per-repo `integrate` configuration with project default and Settings form; snapshot at run start; run branch `htui/item/<key>` for `pull_request`; `integration` record written by `finish_run(Done)`; integrate arm in `sweep_once` with lease, idempotence and park-with-refusal; fast-forward `push`; integration section in the close-out summary. `local` mode unchanged. | none (PR mode completes with P2) | R-ORCH-14, R-ORCH-11, R-TUI-9, R-ENT-4 |
| P2 | Forge adapter and push/forge credentials | Forge adapter trait (push, open or update PR, auto-merge, read state); OneDev REST, then GitHub REST; htui credential helper for git children (reusing MOD-108's); forge token keyring entries and guidance; scrubber masks; systemd-credential exception for headless workers; PR state polling. | MOD-108 (helper), P1 | R-ORCH-14, R-BOX-4, R-STO-1, R-SEC-2, R-SEC-3, R-SEC-4 |
| P3 | Close-out checklist | `CloseOut` waiting reason; deterministic checklist (integration outcome, default resolution, suspect citations through `reconfirm`, follow-up drafts); a draft store for proposed items no reader consumes until accepted (shared with `R-MCP-2`'s `item_create`); one confirmation that closes, creates accepted follow-ups and writes accepted re-confirmations; `done` requires landed work. | P1; coordinates with ANA-35's status migration | R-TUI-9, R-ENT-8, R-ENT-15, R-ID-6, R-MCP-2 |
| P4 | Coordinator turns and the workspace scope | Run kind `coordinator`; workspace-owned runs; a `ToolScope` and `WaitingRow` variant without an item; coordinator MCP tools (read the runs and waiting rows below, write drafts into P3's store, propose a resolution or re-confirmation, request an enqueue or link through a permission card); message-opened turns in the TUI and checklist-opened turns in the executor; resume or reseed from the store. Supplies MOD-121's coordinator groups. | P3, MOD-115 | R-ORCH-14, R-TUI-6, R-MCP-2, R-ID-6, R-ENT-14 |

Effects on existing items, applied at acceptance: **MOD-121** drops "coordinators need ANA-34" and gains
P4 as its blocker for the coordinator groups; **ANA-41** is unblocked and inherits "which work next" and
cross-project landing; **ANA-33** gains the two-level split of 7.3 as a constraint; **ANA-42** §7.5's
merge sentence is narrowed (Q9); **MOD-108** gains P2 as a consumer of its credential helper; **ANA-38**
gains "seed of a coordinator turn" as a reader.

## 10. Requirement edits owed

All maintainer-gated; `docs/REQUIREMENTS.md` is not edited by this item.

**10.1 `R-TUI-9`, proposed final text:**

> **R-TUI-9 (must).** Close-out ends an item. In one transaction, refused while any run of the item is
> active, it writes the final summary document (the commit hashes from the item's runs and the outcome
> of their integration: the merge commit, the pushed commit, or the pull request and its state), sets
> the item `closed` and records its resolution. No markdown files are produced. The project's coordinator
> (R-ORCH-14) prepares the close-out checklist once the item's runs are finished and their work has been
> integrated or refused: a proposed resolution, follow-up items as drafts, and every suspect citation
> (R-ENT-15) with a proposal to re-confirm or amend it. A person works through the checklist and confirms
> the close-out; nothing on it is written until the person accepts it (R-ID-6), and auto mode never closes
> an item. Resolution `done` requires the work to have landed as the project configures. Integration
> itself (merge, push or pull request) is not part of close-out: it is the coordinator's deterministic
> action when each run completes (R-ORCH-14). Amended 2026-10-15; final text by ANA-34.

**10.2 `R-ORCH-14`**, replace "and integrates … and runs close-out (R-TUI-9)" with: "integrates a
child's work when the child's run completes, as the project configures per repo: `local` (merged into the
primary checkout, the default), `push` (fast-forward only, never forced) or `pull_request` (through the
project's forge); and prepares close-out (R-TUI-9). Integration and every status transition are
deterministic code; an agent part, if configured, only proposes drafts a person accepts (R-ID-6). A
coordinator never resolves a conflict: it parks the run and may propose a follow-up item. Version one's
workspace coordinator follows and relays and integrates nothing of its own (ANA-41)."

**10.3 `R-ID-6`**, add after the coordinator sentence: "A coordinator's integration (merge, push, pull
request) and close-out writes are deterministic; its agent part proposes follow-ups, a resolution and
citation re-confirmations as drafts a person accepts, and never holds a push or forge credential."

**10.4 `R-ENT-8`**, replace "Transitions are driven by the orchestrator, the coordinator (R-ORCH-14) and
close-out" with "Transitions are driven by deterministic code, the orchestrator, the coordinator's
integration (R-ORCH-14) and a person's close-out (R-TUI-9), never by unaccepted model text".

**10.5 `R-ORCH-7`**, add: "The integrate step combines one run's parts; landing the run's finished work on
a branch, a remote or a forge is the coordinator's integration (R-ORCH-14), which never resolves a
conflict."

**10.6 `R-ENT-15`**, add: "Close-out's checklist lists the item's suspect citations; re-confirming one is
a person's act, which a coordinator may propose (R-TUI-9)."

**10.7 `R-BOX-4`**, extend the credential sentence to pushing and to a forge's API token for opening pull
requests: "… and the same for pushing to a remote and for a forge's API token used to open pull requests
(R-ORCH-14); these are `htui`'s own credentials, used only by deterministic code and never exposed to a
session (R-SEC-2)."

**10.8 `R-STO-1`**, add push and forge credentials to the keyring list and to the headless worker's
systemd-credential exception.

**10.9 `R-ORCH-11`**, add "and, per repo, the integration mode, target and outcome (merge commit, pushed
commit or pull request reference)".

**10.10 `R-TUI-6`**, add: "A message to a coordinator always opens a turn; its conversation persists in
the store and resumes, or is reseeded from the store, on the next message."

**10.11 `R-MCP-2`**, add the coordinator's tools: read the runs and waiting rows below it, write
proposals into the close-out checklist, and request an enqueue or a link through a permission card.

**10.12 `CONCEPTS.md`**, refine "A coordinator per project and workspace only manages [R-ORCH-14]" to
name the split: deterministic integration and close-out, agent proposals a person accepts. The review's
list also named `R-ORCH-1`; no edit is owed there beyond `R-ID-6`'s existing sentence, because the
coordinator does not plan graphs.

## 11. Not verified

- The bodies of `adopt`, `claim_scan`, the `GixIsolator::reconcile` dispatcher and the
  `shared_serialized` branch-move path; facts about them are from doc comments and trait contracts. That
  `push` mode works for `shared_serialized` and `local` as 7.3 says rests on those contracts.
- Line numbers inside `docs/htui-mcp.md` (cited by section) and ANA-2's drifted line references.
- The OneDev version on `git.mluigi.it`, and therefore whether TOD (15.1+) and the `/~api/tod/` endpoints
  are available there; the REST `PullRequestResource` path is from OneDev's source on `main`, not checked
  against that server. Whether OneDev supports any push-to-create path (not found; tracker not searched),
  OneDev's REST authentication beyond the Bearer form TOD uses, and whether its token expiry is mandatory.
- Whether running a git credential helper owned by htui works with the OS keyring on a headless Linux box
  without D-Bus; MOD-108 has not built it.
- Recalled, not fetched: AG2's `speaker_selection_method` names, Magentic-One's `max_stalls` parameter,
  Temporal's Event History limits (51,200 events or 50 MB).
- Whether ANA-42's §8 has been accepted (`HANDOFF.md:197-200` says it stays open), which Q9 depends on.
- How ANA-33 will shape the integrate step and the planner tools; 7.3 states the constraint ANA-34 needs
  from it, not its design.
- The cost of a checklist-opened coordinator turn against a deterministic-only checklist; no source gives
  a number.
