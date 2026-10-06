# MOD-12 — Auto mode queue runner

> Routed as **PRD** by `/handoff-run MOD-12` (C2 and C4 fired, low confidence at the threshold;
> accepted by the maintainer 2026-10-06, sandbox run `hr/MOD-12`). Ultracode recommended and
> accepted for the implement phase (C4) and the review phase (store, orchestrator, worker and TUI
> all touched). `docs/ANA-2.md` §4.10, §7, §8 ("MOD-12 build order") and §12 criteria 22–28 are the
> design; this PRD records where the tree has moved past it (MOD-4, MOD-7, MOD-41, MOD-69 have
> landed most of the machinery auto mode reuses) and the four decisions taken on top of it.
> Requirements: `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2`, `R-ORCH-10`, `R-ORCH-12` (v1 half),
> `R-ORCH-13` (key stored only), `R-AGT-7..8`, `R-TUI-1` (queue overlay), `R-TUI-2` (`queue`
> action), `R-TUI-8` (caps, scheduler window).

## Problem

Every run in `htui` starts by hand from an item's Runs pane, one at a time. When a run finishes or
parks, nothing starts the next one, so a box sits idle whenever the maintainer is away, and every
configured gate stops a run even on bounded work the maintainer would approve without reading.
There is also no spend guard across runs, so leaving a box working unattended has no ceiling.
Left unsolved, `htui`'s throughput is capped by how often the maintainer looks at it, and the
parallel, unattended pattern the maintainer already runs by hand outside `htui` cannot move inside.

## Evidence

Paths are relative to the repo root, read on `hr/MOD-12` (base `94c434de`).

- **Observed (maintainer's own workflow):** the maintainer already runs 7–10 items in flight per
  day as hand-started `hr` sandboxes, each selected with `/handoff-run next`, driven, and collected
  by hand (session history 2026-10-03/04). That is a queue runner operated manually.
- **Auto mode is unreachable in production.** `RunMode::Auto` is constructed only in fixtures and
  tests; every production `StartRun` is `RunMode::Manual` (`crates/htui/src/ui/tabs/backlog/detail/runs.rs`,
  `crates/htui/src/run_worker.rs`).
- **The gate downgrade is a stub.** The snapshot copies `gate_effective = gate` unconditionally,
  with a comment deferring the downgrade (`crates/htui-orch/src/graph.rs`); everything that reads
  `gate_effective` already honours it.
- **`ready_items` exists but is unused and mis-ordered.** MOD-4 shipped it for `PgStore`,
  `MemStore` and `Backend` with the status, `blocked_by` and tag-subset clauses, but it orders by
  display order, not `priority DESC, created_at` (`crates/htui-store/src/pg/read.rs`), and only
  tests call it.
- **The batch cap is a logged no-op.** `per_token_cap_batch` is seeded (migration `0003`) and read,
  but `crates/htui/src/agent_worker.rs` only logs that it is set; there is no batch identity and no
  usage sum. MOD-2 and ANA-4 hand its enforcement to this item; MOD-23 hands it the caps editor.
- **The rest of the path already exists:** `claim_run` admission with overlap, slot cap and
  `Claim::MissingTags` (MOD-4, MOD-7); the headless worker and the online TUI claiming this box's
  queued runs (MOD-41); the waiting-on-you view with reasons (MOD-69).
- **Assumption — needs validation via prototype:** skipping non-hard gates on `FIX` / `CLEAN` /
  `TOOL` graphs does not degrade outcomes. Validated by running a small unattended batch and
  counting runs that end `done` versus those needing a human.

## Users

- **Primary**: the maintainer, running `htui` on their own box (online TUI or headless worker) with
  a backlog of bounded items whose graphs carry no hard gate, who wants the box to keep working
  through a chosen set of items while they are away and to find, on return, what finished and what
  needs them.
- **Not for**: dispatching to another box (MOD-43 owns target-box selection in auto mode);
  time-windowed scheduling (`R-ORCH-13`, `later`); items whose graphs are gated `gate_hard`
  throughout — those still run, but park at every hard gate by design.

## Hypothesis

We believe **a per-box queue runner that takes the items the maintainer queued in queue order, runs
them with non-hard gates skipped, admits runs within `max_concurrent_items` and the per-batch spend
cap, and surfaces every parked or blocked item in a pausable queue overlay** will **turn idle box
time into finished bounded items** for **the maintainer**.
We'll know we're right when **a batch of queued `FIX` / `CLEAN` / `TOOL` items reaches a terminal or
escalated state with zero manual run starts, batch spend never exceeds `per_token_cap_batch` by more
than one attempt, and every escalation appears in the overlay with its reason**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Manual run starts in an unattended batch | 0 | Prototype batch of ≥3 queued `FIX`/`CLEAN`/`TOOL` items, every run row `mode = auto` |
| Batch spend overshoot | ≤ one attempt's estimate above `per_token_cap_batch` | Conformance test over `MemStore` + Postgres test over `SUM(run_step.usage)` |
| Escalations visible | 100% of parked / blocked / refused queued items listed with reason | Overlay snapshot tests per reason; ANA-2 criterion 27 |
| Manual/auto isolation | Overlapping manual and auto runs serialise in either order | ANA-2 criterion 26 as a test |
| ANA-2 validation criteria 22–28 | all pass | One named test (or test group) per criterion |
| Unattended outcome quality | TBD — needs validation via prototype | Share of auto runs ending `done` without a human in the prototype batch |

## Scope

**MVP** — the three milestones below: unattended runs of explicitly queued items (M1), the batch
spend guard and its Settings section (M2), and the queue overlay with escalations, pause and
reorder (M3).

**Decisions taken (maintainer, 2026-10-06, recommendations accepted)**

- **D1 — Queue membership is opt-in.** Auto mode runs only items the maintainer explicitly
  `queue`s (Backlog `queue` action, `R-TUI-2`), never every ready item on the box. An item that is
  queued but not ready (blocked, missing tags, `blocked_by` open) waits in the queue and says why.
- **D2 — A batch is one queue activation.** From resume (or first start) until pause or drain is
  one batch, given its own durable identity that each auto run records, so the cap, the overlay
  and the run record all mean the same "this batch". This needs a forward-only migration; ANA-2's
  "no aggregate column" still holds — batch spend is the sum of its runs' `run_step.usage`, never a
  stored total.
- **D3 — The runner lives in the shared worker runtime.** Both the online TUI and the headless
  worker run it; pause state is persisted per box so a pause set in the TUI stops a headless
  worker's admissions. `claim_run` stays the single admission path, so two runners on one box
  cannot double-admit.
- **D4 — Reorder sets a queue position.** Queue order is queue position, then `priority DESC,
  created_at` (ANA-2's ordering) to break ties; reordering never edits an item's backlog
  `priority`. This extends `R-ORCH-6`'s literal "dependency order and priority" with a
  maintainer-set position, recorded here as a deliberate requirement amendment.

**Out of scope**

- Enforcing the scheduler window — `R-ORCH-13` is `later`; the `scheduler_window` key is edited
  and stored only.
- Target-box selection in auto mode — MOD-43's; MOD-12 runs on the local box only and reports a
  queued run targeted elsewhere rather than claiming it.
- In-flight-first and dependent-count ordering (ANA-2 §4.10's R1/R2 prior art) — requirement
  changes not taken.
- Timer-based auto-approval and a judge substituting for a downgraded gate — both rejected in
  ANA-2 §4.10.
- `ready_items` on the SQLite cache — withdrawn with ANA-10's verdict (MOD-25); Pg and Mem only.
- Any billing statement — the cap is a guard rail (ANA-4 §7).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Unattended runs | The maintainer queues items from the Backlog; the box runs them one after another (up to `max_concurrent_items` at once) in queue order, with non-hard gates skipped and recorded as `skipped`, hard gates still parking; pausing stops new admissions without touching running runs | pending | — |
| 2 | Spend guard | Each queue activation is a batch; once its spend reaches `per_token_cap_batch`, or a run's remaining budget is below one attempt, nothing further is admitted; the Settings tab edits the caps, `max_concurrent_items` and the (unenforced) scheduler window | pending | — |
| 3 | Queue overlay | One overlay shows the queue in order, what is running, and every escalation with its reason (review loop exhausted, judge undecided, missing tags, hard gate parked, blocked, targeted at another box), with pause/resume and reorder | pending | — |

## Open Questions

- [ ] Unattended outcome quality for `FIX` / `CLEAN` / `TOOL` graphs — validated by the prototype
  batch after M1; a poor result narrows which kinds the maintainer queues, not the mechanism.
- [ ] Does pausing end the batch (so resume opens a new one with a fresh cap), or does a batch
  span pauses until the queue drains? D2's reading is "pause ends it"; `/plan` for M2 confirms
  against the overlay wording before the migration is written.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Two runners on one box (TUI + headless worker) race admission | Medium | Double runs, overlap breach | `claim_run` stays the only admission path; a two-runner test over Pg |
| Gate downgrade applied anywhere but snapshot time lets a mode change skip a passed gate | Low | Unreviewed work lands | Downgrade only in the snapshot builder; ANA-2 criterion 24 as a test |
| Batch spend read lags the recorder, overshooting the cap | Medium | Spend past the cap | Admission also applies `min_budget_for_new_attempt`; overshoot bounded to one attempt and tested |
| Migration for batch identity collides with a sibling run's migration number | Medium | Refused connect on merge | Number chosen at plan time against the host tree; flagged in the M2 plan |
| A queued item stuck not-ready is silently skipped forever | Medium | Maintainer thinks it ran | D1: queued-but-not-ready items listed with their reason in the overlay |
| Stack headroom in `htui-orch` conformance (`every_case_name_dispatches` near 2 MiB) | Medium | SIGABRT in the gate | Box large engine futures; gate with `--no-fail-fast` and grep for SIGABRT |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
