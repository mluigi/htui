# MOD-69 — Waiting-on-you list across items

> Routed as **PRD** by `/handoff-run MOD-69` (criteria C2, C3 and C4 fired; accepted by the
> maintainer 2026-10-03, sandbox run `hr/MOD-69`). Ultracode accepted for the implement phase.
> Origin: `docs/ANA-27.md` §5.1 T8, §7 (`R-TUI-11` proposal), §8 (scope question);
> `docs/decisions/ana/ana-27.md`. Requirements: `R-TUI-1`, `R-TUI-4`, `R-ORCH-2`, `R-ORCH-4`,
> `R-NF-3`, proposed `R-TUI-11`. Spawned: MOD-75 (agent question tool, blocked on MOD-11).

## Problem

The maintainer runs several items at once (the in-process engine, `htui worker`), and any of them
can stop for a person: a parked gate, a fan-out awaiting selection, a failed judge, a blocked run,
an open permission request. Each of these shows only in its own item's Runs pane, and the top bar's
run count lumps waiting runs in with working ones, so the only way to find what needs a person is
to visit items one by one. A run left unseen holds its place in the walk — and a pending permission
request holds a live session and a compute slot — for as long as nobody happens to look.

## Evidence

Read at `187ca50e` (branch `hr/MOD-69`). Paths relative to the repo root.

- **Maintainer report (2026-10-03).** Items stop to ask the person something, or need an opinion
  because they found something they did not expect; and the top bar does not tell items that are
  working from items that are waiting. The second half (an agent-raised question) has no mechanism
  today — see the last bullet — and is carried by MOD-75.
- **Each reason is visible only per item.** The Runs pane derives waiting states through the
  engine's own predicates (`ItemActions`/`StepActions`, `crates/htui-worker/src/views.rs:174-216`,
  `actions()` :256) for the one selected item, and requests `RelayView{item}` for permissions
  (`crates/htui/src/ui/tabs/backlog/detail/runs.rs:446-473`). No read spans items.
- **The top bar does not separate waiting from working.** It renders `TopBarState.active_runs`
  (`crates/htui/src/ui/top_bar.rs:29-41`), fed by `StoreRequest::ActiveRuns{scope}`
  (`crates/htui/src/store_worker.rs:133`, served :1709) about once a second
  (`crates/htui/src/app/update.rs:19`, :126-136). `RunStatus::AwaitingApproval` counts as active
  (`crates/htui-core/src/model/run.rs:52`), so a parked run is counted as running.
- **No single "waits on a person" field exists; reasons are derived.**
  - Gate: step, run and item `awaiting_approval`, reason in `run_step.gate_note`
    (`park_step`, `crates/htui-orch/src/gate.rs:505`).
  - Selection: run and item `awaiting_approval` with **no parked step**; reason only in an
    `item_note` (`park_selection`, `crates/htui-orch/src/engine.rs:4462-4510`).
  - Judge failure: a selection park whose `HumanReason::JudgeFailed` (`crates/htui-orch/src/fanout.rs:215-226`)
    survives **only as note text** — no column records it.
  - Unblock: `UnblockCase` Reopen / FollowRun / Resume (`crates/htui-orch/src/command.rs:291-302`,
    `unblock_enabled` :1173); telling a park from a resumable crash needs `status::cursor` and the
    run's graph snapshot (`crates/htui-orch/src/status.rs:151,272,315,356`), not plain SQL.
  - Permission: `step_permission` (migration `0011_permission_relay`), open = `pending` under a
    live lease of the current owner (`crates/htui-store/src/pg/relay.rs:501-525`), with a scrubbed
    `summary` ("<tool_kind>: <title>"). The existing read is per item and on `WriteStore`
    (`crates/htui-core/src/store/traits.rs:1795`); relay tables are **not mirrored**, so offline the
    view is empty (`docs/decisions/mod/mod-42.md` OQ-4). MOD-42 is done, so these rows are in scope.
- **Nothing opens a Runs pane on a given step.** `RevealTarget` carries only `Item` and
  `Requirement` (`crates/htui/src/app/action.rs:101`); Backlog's `reveal` selects the item only
  (`crates/htui/src/ui/tabs/backlog/mod.rs:786`); step selection is private to the Runs pane
  (`runs.rs:334,412`).
- **Overlays and keys.** Global overlays register in `register_all`
  (`crates/htui/src/app/mod.rs:58-110`; Ctrl+F concepts search, `w` switcher, `KeyScope::Global`);
  dispatch is `App::on_key` (`crates/htui/src/app/state.rs:575`). MOD-67 (named actions,
  `keys.toml`) has not landed — no code, only the ANA-26 docs commit `bc9a3503`.
- **Scope.** `Scope` is always one workspace (`crates/htui-core/src/model/scope.rs:10`); the TUI is
  always inside one.
- **Agents cannot ask.** No tool or step exit lets an agent raise a question to a person; a question
  written in an agent's reply stays in the transcript and an ungated step still ends `done`. The MCP
  server (MOD-11) is not on this tree. Filed as **MOD-75**.
- **Prior art.** OpenRig's TUI attention view shows human requests apart from outcome and health
  updates (https://openrig.dev/docs/tui; `docs/ANA-27.md` T8).

## Users

- **Primary**: the maintainer as htui's operator, with several items in flight across the engine
  and `htui worker`, who needs to know — without visiting each item — what is waiting on them and
  why.
- **Not for**: auto mode's unattended escalation path (MOD-12 owns it and may reuse the query);
  cross-workspace oversight (the list follows the active workspace).

## Hypothesis

We believe **a waiting-on-you overlay reachable from every screen, plus a top bar that counts
working and waiting runs separately**, will **end the item-by-item hunt for parked runs** for **the
maintainer running several items**. We'll know we're right when **every waiting reason in the
active workspace appears in the list and the waiting count, each reaching its step in one key plus
`Enter`, and the counts agree with the store within one refresh tick (~1 s)**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Reason coverage | Every reason (gate, selection, judge failure, unblock, open permission) yields exactly one row per instance; a resumable crash or a finished run yields none | Conformance cases over MemStore and Postgres, one fixture per reason plus negatives |
| Count split | Waiting = rows in the list; working = active runs owning no row, so a run is never counted twice ("working + waiting = active" does not hold: a Reopen row has no run and a run may own several rows — plan D5) | Classifier test over mixed fixtures |
| Freshness | Top bar counts and an open overlay reflect a park or an answer within one refresh tick | Update-loop test on the existing tick cadence |
| Reach | `Enter` on any row lands on the item's Runs pane with that step (or the run, for selection) selected | TUI snapshot / navigation test per reason |
| No own state | The list has no table, cache or persisted field of its own | Review gate; no migration for the list itself |

## Scope

**MVP**
- The top bar shows two counts — runs working and runs waiting on a person — replacing the single
  active-run count (`R-TUI-1` line change).
- One overlay, opened from every screen by a fixed key, lists every run in the **active workspace**
  waiting on a person: one row per reason with the item, the step and the reason. Reasons: gate,
  fan-out selection, judge failure (its **own row**, distinct from selection), unblock, and each open
  permission request with its tool.
- `Enter` on a row opens that item's Runs pane on the step.
- The list is computed from the store at read time by one shared query that the top bar's waiting
  count also uses; it keeps no state of its own. MOD-12 can reuse the query.
- Offline, open permission requests are omitted and the overlay says "permissions unavailable
  offline" (relay tables are not mirrored).
- `docs/REQUIREMENTS.md`: `R-TUI-11` added and the `R-TUI-1` top-bar line amended (approved and
  applied 2026-10-03).

**Out of scope**
- Rebinding the opening key in `keys.toml` — MOD-67 registers it as a named action when it lands.
- Agent-raised questions — MOD-75 (blocked on MOD-11); its rows join this list when it lands.
- Auto-mode escalations — MOD-12.
- Every-workspace view — the list follows the active workspace (maintainer, 2026-10-03).
- Answering a gate, selection or permission from the overlay itself — the Runs pane stays the one
  place that answers; the overlay navigates.

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | See what waits | Top bar shows working vs waiting counts; the overlay lists every waiting reason in the active workspace (gate, selection, judge failure, unblock, open permission) from every screen | in-progress | `.claude/plans/mod-69-waiting-on-you.plan.md` |
| 2 | Jump to it | `Enter` on a row opens the item's Runs pane on that step; close-out | in-progress | `.claude/plans/mod-69-waiting-on-you.plan.md` |

## Open Questions

- [x] **`R-TUI-11` and `R-TUI-1` wording** — approved as proposed (maintainer, 2026-10-03) and applied
  to `docs/REQUIREMENTS.md`: `R-TUI-11` added (gate, fan-out selection, judge failure, blocked run,
  open permission request, agent's open question; active workspace; read-time derivation);
  `R-TUI-1`'s "active run count" became "working run count and waiting-on-you count (R-TUI-11)".
- [x] **Offline permission rows** — approved (maintainer, 2026-10-03): offline, the list omits open
  permission requests and the overlay shows "permissions unavailable offline".
- [x] **Judge failure as its own row** — decided (maintainer, 2026-10-03); *how* it is recorded
  (a structured field vs the note text) is `/plan`'s call, constrained by "no state of the list's
  own".

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The derived list drifts from the Runs pane's own predicates and shows a row the pane cannot act on (or misses one it can) | Medium | Medium | Derive from the same engine predicates the pane uses; conformance cases pin each reason against the pane's enabled action |
| A workspace-wide derivation per tick is too slow (snapshot reads for unblock classification) | Medium | Low | Measure on a seeded workspace; bound the per-tick work (count only for the top bar, full list only while the overlay is open) |
| A permission row outlives its lease and shows a dead request | Low | Low | Reuse the relay's own "open" predicate (pending under a live lease) |
| The fixed key collides with an existing global binding before MOD-67 lands | Low | Low | Pick a free global key; MOD-67 later makes it remappable |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
