# MOD-139 - Divided fan-out runtime: planner, parts, integrate

Routed PRD path by `/handoff-run` (C2 new engine roles and seams, C4 breadth; maintainer accepted 2026-11-10,
ultracode for implement and review). Source: `HANDOFF.md` MOD-139; design: `docs/ANA-33.md` §7.4, §7.5, §7.7,
§7.8, §7.11, §10 (all ten §8 decisions accepted 2026-10-18).

## Problem
A graph author can save a step with `split = divide` (MOD-137 stores it, MOD-138 expands it into
`<name>:plan` and `<name>`), but no run can start on it: resolve refuses the node as not yet runnable. The only
runnable fan-out is compete (N agents, one prompt, one survivor), and parallel plain nodes each land alone and
park on any conflict, so nothing can split one step into different parallel parts and have an agent merge them.
Large items therefore run as one long session in one tree, or are split into separate items by hand.

## Evidence
- The refusal is in code: resolve answers `NotYetRunnable { reason: Divide }` naming MOD-139
  (`crates/htui-orch/src/graph.rs`, pinned by `a_divide_node_is_expanded_then_refused`).
- Five open items are blocked on it: MOD-140 (planner tools), MOD-142 (Runs pane parts tree), MOD-27
  (consult-mode `spawn_subagent`), MOD-126 and MOD-132 (the `stage` rows).
- `R-ORCH-7` was redefined as division of work in the 2026-10-15 requirements review; ANA-33 §7.14 puts divided
  fan-out in version one.
- Benefit over one long session: **Assumption — needs validation via a trial on a real item** (for example an
  `implement` divided into an `api` and a `ui` part) once M3 lands.

## Users
- **Primary**: the maintainer as graph author and run operator, who wants one step split into parallel parts
  (each in its own branch, with its own brief and agent) and one merged, checked branch back.
- **Not for**: best-of-N users (compete fan-out, `R-LATER-13`, unchanged); multi-item orchestration
  (subgraphs, MOD-143; the coordinator, ANA-34).

## Hypothesis
We believe **running a `divide` node as planner, parallel parts and an integrate agent** will **let one large
step be split into parallel work and come back as one merged, verified branch** for **the graph author**.
We'll know we're right when **a run on a graph with a two-part divided `implement` lands exactly one integrate
branch whose history contains both parts' commits, the orchestrator itself resolves no conflict, and a crash at
any role boundary resumes without a twin session**.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Divided run lands one branch | 1 integrate landing per divided node; every done part's `after_hash` an ancestor of it | Engine end-to-end test on the fake isolator and a real-git test |
| Conflicting parts reach the agent, not the orchestrator | Pre-merge stops before the first conflicting part and reports it; the orchestrator writes no resolution | Isolator test with two parts editing one path |
| A missing part is never routed around | An exhausted part fails the divided step; integrate never starts on a partial set | Engine test |
| Recovery per role | A crash after admission, mid-part, mid-pre-merge and after integrate resumes with no twin session and no re-planning | Recovery tests per role, MemStore and PgStore conformance |
| Linear and compete runs unchanged | Existing suites green, no snapshot drift | The gate |
| Real-world benefit | TBD | Trial on a real item after M3 (assumption above) |

## Scope
**MVP** — default-parts divided nodes run end to end: revision-0 admission, parts in their own branches from
shared base refs, integrate with a deterministic pre-merge and an ancestry post-check, only the integrate branch
landing, recovery per role, caps at admission.

Decisions taken at the PRD gate (2026-11-10):
- **Q1 (planner agent before MOD-140):** a divided node with a planner *agent* configured is refused at resolve
  with its own reason naming MOD-140. The admission transaction is still general enough to admit a proposed
  revision ≥ 1, exercised through a direct store write in tests.
- **Q2 (revision 0 approval):** default parts are admitted at once, with no approval park; the plan-approval
  park is MOD-140's, its view MOD-142's.
- **Q3 (box-level session cap):** not in this item. `max_parallel_sessions` per run and the per-part quota wait
  only; a box-wide cap becomes a follow-up if a trial shows it is needed.

**Out of scope**
- `plan_bounds` / `plan_submit` / `plan_status`, the shared plan validator, the plan-approval park,
  `runtime_changes`, `successors` plans, read-only planner exposure — MOD-140.
- The Runs pane parts tree and plan-approval view — MOD-142.
- Writing `pick`, `prepare`, `collect` stage rows — MOD-126 and MOD-132; this item reserves the role and positions
  only.
- Subgraph nodes — MOD-143. Consult-mode subagents — MOD-27.
- A per-part diff check against `owned_paths` — ANA-33 §9 leaves it unknown.
- "Integrate what finished" — rejected by ANA-33 §8 decision 10.

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Records | Step rows carry role, parent step and node key; the append-only plan record exists and is mirrored to the box cache; the `stage` role and positions -3..-1 are reserved; a phase named with the `:plan` suffix is refused at save. Runs behave as before. | pending | — |
| 2 | Planner admission and parts | Default parts are admitted as revision 0 in one transaction with base refs; each part runs in its own branch from them with its own part prompt and agent; part documents are hidden from generic inputs; per-part retry and whole-step failure; caps at every admission; `fan_out_only` means live siblings; planner and part recovery. Divided graphs are still refused at resolve. | pending | — |
| 3 | Integrate and landing | Integrate prepares from base refs (with the copy-mode fetch), pre-merges the parts and hands the report and the parts' documents to its agent; the ancestry post-check and verify gate it; only its branch lands; integrate recovery and the landing frontier. The resolve refusal is lifted for default-parts divided nodes. | pending | — |

## Open Questions
- [x] Q1 planner agent before MOD-140 — refuse at resolve (decided 2026-11-10).
- [x] Q2 revision-0 approval — none (decided 2026-11-10).
- [x] Q3 box-level session cap — deferred (decided 2026-11-10).
- [ ] Does `merge_no_ff`'s two-parent post-condition hold unchanged in a `copy` tree after the part fetch
  (ANA-33 §9, unverified)? — TBD, needs validation via a real-git test in M3's plan.
- [ ] Migration numbers: the tree's next is `0028` (cache `0010`), but parallel sandboxes (MOD-97) also take
  `0028`; renumbered at collect as before.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Concurrent part futures overflow the 2 MiB debug stack | Medium | High | Box every per-node future; gate `--no-fail-fast` and grep SIGABRT (ANA-33 §9) |
| Ancestry proves commits, not changes (an integrate `-s ours` merge passes) | Low | Medium | Per-part diffstat to integrate and review; review stays after integrate |
| One run with N parts draws N sessions and quotas | Medium | Medium | `max_parallel_sessions` and per-part quota wait; box cap deferred (Q3) |
| Migration moves count pins and needs `sqlx prepare` on a migrated scratch DB | High | Low | Repin `tests/migrations.rs`, conformance counts; scratch DB per `docs/hr-sandbox.md` |
| Recovery bug classes seen elsewhere (stop-then-retry on fanned-out DAGs) | Medium | High | Recovery tests per role; MOD-125's matrix gains a divided step |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
