# MOD-12 - Auto mode queue runner (done, 2026-10-08)

**Requirements:** `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard gates, `R-AGT-7..8` caps, `R-TUI-8`, the
`queue` action of `R-TUI-2`.
**Origin:** ANA-2 (`docs/ANA-2.md` §4.10, §9).
**Artifacts:**
- PRD `.claude/prds/mod-12-auto-mode-queue-runner.prd.md` (decisions D1-D4, three milestones);
- M1 plan `.claude/plans/mod-12-m1-unattended-runs.plan.md` and its blueprint;
- M2 plan `.claude/plans/mod-12-m2-spend-guard.plan.md` and its blueprint;
- M3 plan `.claude/plans/mod-12-m3-queue-overlay.plan.md` and its blueprint (amendments F-1..F-25).

Each plan carries a verified-claims table, and decision numbers are local to each plan. The item was
routed as **PRD**. Each milestone was then planned on that PRD. M3 ran with ultracode for the
implementers only: one workflow per task, implement → three read-only verifiers → repair.

## The problem

`htui` ran an item only when the maintainer started it. ANA-2 concluded the design for an unattended
mode:
- runs on ready, explicitly queued items in priority order, admitted through the same `claim_run`
  as manual runs;
- non-hard gates skipped and recorded, hard gates still parked;
- spend bounded per batch;
- one view of what the queue is doing and what needs a person.

## What was built

### M1 - unattended runs (`48ff0811`..`d19767df`, 2026-10-07)
- **Migration `0016_auto_queue`:**
  - `queue_entry`: one item on one box, with an explicit `position`;
  - `queue_batch`: one activation, at most one open per box (partial unique index);
  - `run.batch_id`.
- **`ready_items`** in `priority DESC, created_at, id` order (ANA-2 criterion 22).
- **Gate downgrade at snapshot time only.** `effective_gate` runs only in the snapshot builder: an
  auto run's soft gates become `never` (`gate_outcome = skipped`), and hard gates park (criteria 23,
  24).
- **The runner's admission** in `sweep_once`, run only by the box's executing process. It counts
  slots on running and queued runs, admits in queue order through `Engine::enqueue_in_batch`, makes
  a cancel stick for its batch, and drains a finished batch. A walk's end and a resume wake the sweep.
- **Backlog keys.** `Q` queues or dequeues; `P` resumes (opens a batch) or pauses (closes it and
  cancels its still-queued runs).

### M2 - spend guard (`961e4f40`..`8b3aed9f`, 2026-10-07)
- **Batch spend.** It is the sum of the batch's runs' integer `cost_micros` and is never stored.
- **The cap.** Each run is held to its own project's `per_token_cap_batch` against the whole
  batch's spend. One rule, `batch_budget`, applies at three points:
  - the runner's admission;
  - the candidate walk (`BatchCapReached` / `BatchBudget`, the item goes `blocked`);
  - the session allowance, `min(run cap - run spend, batch cap - batch spend)`. This also fixed the
    run cap being applied per step.
- **Settings > Queue** edits:
  - the two caps, in USD;
  - `min_budget_for_new_attempt`;
  - the app-wide and per-box `max_concurrent_items`;
  - the stored-only `scheduler_window`.
- **PRD metric.** The overshoot metric was amended to one attempt per open session.
- **Review.** R1 was applied. Its residuals are CLEAN-9.

### M3 - queue overlay (`925c9e9f`..`84cfb161`, 2026-10-08)
No migration.
- **The overlay** (`crates/htui/src/ui/overlay/queue.rs`) opens on global `ctrl-q` (`Act::Queue`).
  It lists the box's queue in one order everywhere, `position NULLS LAST, priority DESC,
  created_at, id`. Each row's state comes from the pure `classify_entry` (`model/queue.rs`):
  - running here, or live on another box;
  - next;
  - held by the batch rule (budget stop, malformed cap, absent project);
  - waiting (open `blocked_by`, cancelled in this batch);
  - escalated: review loop exhausted, judge undecided, hard gate parked, blocked, failed, or missing
    tags.

  The header reads either "running · batch since · spent · slots" or "paused", with why (by a user,
  or the last batch drained).
- **Overlay keys:**

  | Key | Action |
  |---|---|
  | `j` / `k` | Cursor |
  | `J` / `K` | Reorder |
  | `P` | Pause or resume |
  | `Q` | Dequeue |
  | `Enter` | Reveal the step or item |

  The overlay re-reads every third refresh tick and backs off after a failed read. `Overlay::refresh`
  is the shell's new hook for this.
- **Store:**
  - `queue_rows(box)`: one joined read of the latest graph run, parked step, latest note and open
    blockers, deduplicated;
  - `move_queue_entry`: atomic, positions `1..n` on the first move, locks in `item_id` order;
  - `missing_tags_of` (batched);
  - `last_closed_batch`.

  All four are on Mem, Pg and `Backend`, with an offline refusal and Pg/Mem parity tests.
- **Requests and the serve module.** `StoreRequest::QueueOverview` and `MoveQueueEntry` are served
  by `crates/htui/src/queue_overview.rs`, which composes the runner's own reads.
- **M1 review L4: a stalled batch closes** (maintainer, 2026-10-07). `admit` closes the batch
  `drained` once no run of it is live and nothing is admissible. A tick with no free slot never
  closes, and neither does an enqueue refusal. `close_drained_batch(batch, seen, at)`:
  - locks the batch row first, so it waits for an in-flight admission (a race the T2 verifiers
    reproduced, fixed in `4232d50c`);
  - is skipped while the box holds an entry for an item the runner did not read.

  So a later `Q` never starts spending; `P` does.

## Review R1 (M3)

`rust-reviewer`: approve with fixes. The maintainer chose to apply every finding:
- **H1 - `ctrl-q` refused existing keys files.** A `[global]` `ctrl-q` binding, which was the
  README's own example, refused startup. The keys rule is now **a user's entry beats an action left
  at its default**, within one table: that action loses the chord. htui prints a stderr notice, and
  `--print-keys` marks the line `(unbound by quit)`. Two user entries on one chord are still an
  error. The README example now binds `ctrl-x`, and ANA-26 §7.4 is amended. Commits `3c499bd8`..
  `5a7c9642`.
- **M1 - stale `P`.** `PauseQueue { expect }` and `ResumeQueue { expect_paused }` carry the state the
  sender saw. A mismatch answers `QueueWrite::Stale` with the queue as it now is, and both senders
  (the overlay and the Backlog) use it. `674cd08f`.
- **M2 - refresh cost.** The overlay re-reads every third refresh tick, and one `missing_tags_of`
  call replaces the per-row reads.
- **L1 - lock order.** Pg locks queue entries in a deterministic order in the move and the prune
  (`b8f40932`).
- **L2 - the drain's freshness guard.** It is by item, not by client timestamp (`92f51911`).
- **L4 - the paused header says why.** `L3` (an enqueue refusal that changes no state shows as
  "next") is documented as a known limit on `EntryState::Next` and in ANA-2.
- **N1-N3.** Cached row sentences, a write's result on the status line, and the doc count.

**Process note.** M1's Stale tests landed in the same commit as their fix, not red first. The repair
agent proved them after the fact by disabling the checks: four tests failed, and the file was then
restored.

## Gate

The final tree `84cfb161` passed in the sandbox:
- `cargo fmt --check`;
- clippy `-D warnings`, with all features and without;
- `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` (count in the run's
  report): 5608 passed and 1 failed. The failure was
  `qdrant_worker`'s "create collection" timeout under load, which passed 3 out of 3 when re-run alone; M3
  touches nothing in Qdrant;
- `cargo sqlx prepare --check` against a migrated scratch DB;
- `cargo insta test`: new queue-overlay snapshots only;
- `validate-workflow-docs.sh`.

`cargo doc -D warnings` still fails on pre-existing private intra-doc links (CLEAN-10); M3 added none.

## Owed on the host

- **The Postgres gate on the merged tree.**
- **The PRD's prototype batch:** at least 3 queued `FIX`/`CLEAN`/`TOOL` items, for the "unattended
  outcome quality" metric. It is still an open question in the PRD.
- **CLEAN-9** (M2 residuals) stays open.

**Relates to MOD-43:** target-box selection in auto mode. MOD-12 only reports a run that is live on
another box.
