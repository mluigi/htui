# MOD-24 - Crash recovery of runs under the headless worker (done, 2026-10-02)

**Requirements:** `R-HIS-1`, `R-ORCH-11`.
**Origin:** the original ask (checkpoint agent memory, re-hydrate, resume mid-step) was dropped by
maintainer decision on 2026-09-25: a run survives a crash through ANA-2 §4.9's reset-and-retry, as
built by MOD-4 M6. What was left was the proof under `htui worker` (MOD-41), ANA-27 §5.1 T11's kill
points, and MOD-53's leftover (a chat that panics mid-turn).
**Artifacts:** plan
[`.claude/plans/mod-24-worker-crash-recovery.plan.md`](../../../.claude/plans/mod-24-worker-crash-recovery.plan.md)
(fact-checked: 27 claims, 7 falsified and amended before CONFIRM) and blueprint
`.claude/plans/mod-24-worker-crash-recovery.blueprint.md` (hazards H-1..H-23, amendments A-1..A-6).
Routed as a plan (0 of C1-C4 fired), run in the TOOL-7 sandbox on `hr/MOD-24`.
**Decisions:** maintainer, 2026-10-01: route accepted; plan confirmed with OQ-1..OQ-4 as recommended
(a real process kill; a cancel survives a crash; a chat orphaned by a TUI crash stays out of scope;
the sweep stops leasing chat runs here). Review dispositions as recommended.
**Commits:** `541e455`..`1617428` on `hr/MOD-24` (plan, blueprint, five tasks, the review round),
plus this close-out. No migration: the next one is still `0012`.

## What the fact-check changed

- **ANA-27's kill points were not all reachable.** "After capture and before the output document"
  does not exist: `walk_live_step` writes the document (`sink.after_done`) before it captures. The
  points became K1 session started, nothing flushed; K2 after a flush; K3 after the document, before
  capture; K4 after capture, before `finish_step`; K5 a pending cancel read and not applied (MOD-42
  has no `NOTIFY`, so the poll's pickup is the only "notification").
- **A pre-crash cancel could lose to the recovery.** The restarted process swept (adopting and
  recovering the run, sometimes finishing it) before its command poll applied the pending cancel,
  which then resolved `refused`. Deterministic when the kill is K4 on the last ungated phase.
- **Every sweep leased live chat runs.** `adopt_runs` had no `kind` filter: a chat run is `running`
  with a `NULL` lease, so each sweep took its lease, failed to recover it (no graph snapshot) and gave
  it back, and the chat's own writes (`StepFence::Unleased`) were refused `Fenced` meanwhile.

## What shipped

- **Kill points** (`htui_orch::kill_point`). `reached(point, site)` is an empty `#[inline]` fn unless
  `htui-orch/test-support` is on. Armed, it reads `HTUI_TEST_KILL_POINT`
  (`<point>[@<phase>][#<attempt>]`) once, writes the marker named by `HTUI_TEST_KILL_MARK` (temp,
  sync, rename) and parks the thread until the parent's `SIGKILL` (exit 86 after 300 s, 87 on a bad
  spec or a failed marker). Call sites: `Documented` and `Captured` in `walk_live_step`,
  `CommandPicked` in the worker runtime's `poll_once`. K1 and K2 need no hook: the test's own
  session parks.
- **Commands before recovery (D3).** `adopt` first applies every pending cancel of a free `running`
  graph run on the box (`cancels_first`, the poll's own `cancel_run`), waiting under MOD-42's B-5
  guard for a row the poll already holds; only then does `sweep_fenced` recover. A failed read skips
  that tick's recovery rather than recovering without the cancels. Such a run ends `cancelled`,
  never walked on.
- **Graph runs only (D3b).** `adopt_runs` adds `kind = 'graph'` (Postgres and MemStore), with store
  conformance case `adopt_runs_never_leases_a_chat_run`. One `.sqlx` entry swapped.
- **A panicked chat closes its run (D4).** The fresh chat's panic answer closes the `run`/`run_step`
  pair `failed` before it sends `Failed` then `Ended`, bounded by `PANICKED_CHAT_CLOSE` (5 s), and
  never after a normal close (a shared closed flag). A promoted step's chat closes nothing.
- **The kill test** (`crates/htui/tests/worker_crash_pg.rs`). The test re-executes its own binary
  (`--exact crash_child`) as the worker, connected as the fixture box over the case's throwaway
  database, running `htui_worker::worker::run` with the real `GixIsolator` over a temporary repository
  (hermetic git config), a fake verifier and scripted sessions that commit. The parent waits for the
  marker or the row, `SIGKILL`s the child (signal 9 asserted), and a second child (a new lease owner)
  recovers after the 5 s test TTL. K1-K3 retry the step (attempt 1 `failed`/`interrupted`, the
  "retrying as attempt 2" note, attempt 2 `done`, the stray write never lands; K2 keeps the flushed
  log, K3 the first document); K4 settles `done` with no attempt 2 and no new session; K5 ends
  `cancelled` with the command `applied`. Each case fails with its crash removed; K5 fails 3 of 3
  with `cancels_first` disabled. The guide gained "If the worker crashes"
  (`docs/htui-worker.md`).

## Review

`rust-reviewer` approved with changes (3 MEDIUM, 8 LOW, 4 NIT). All applied except N2 (one extra
run read per row, kept). M1: `cancels_first` skipped, instead of awaiting, a row the poll held,
because the poll mints the walk child before it takes the run lock; M2 pins that wait; M3 raised the
test TTL to 5 s and moved the sessions' git calls to `spawn_blocking`. L1-L3 pin the chat close's
status and ordering, bound it and stop a second close; L4 keeps the sweep's cancels off chat runs;
L5 gates recovery on the cancels' read; L6 makes the children's git hermetic.

Fixing the review found a **deadlock from MOD-42**: `Shared::applying` built its `Applying` guard
eagerly (`then_some`), so a refused second claim dropped that guard while the set's lock was still
held, and its `Drop` locked the set again. D3 made a second claim of a held row reachable (K5 hung
about one run in four). Fixed in `b119d31`; test
`a_second_claim_of_a_held_row_is_refused_and_frees_nothing`.

## Decisions worth keeping

**A crash test needs a crash.** Aborting the loop task is not one: `RunRuntime` has no `Drop`, the
supervisors detach and the walks go on heartbeating. Only a separate process killed with `SIGKILL`
holds a lease no destructor gives back.

**Commands outrank recovery.** A user's cancel written before a crash is a decision; the recovery is
a mechanism. So the adopter reads commands first.

**The hook lives where test parts cannot reach.** K1, K2 and the document are the test's own (its
session and author); only the two engine seams after the document and after capture, and the poll's
pickup, need a production call site, and that call is empty without `test-support`.

## Left open

- **A chat orphaned by a TUI crash** (not a panic) stays `running`: a chat run has no lease or owner
  to sweep by, and after D3b no sweep touches it (OQ-3).
- A command already queued to a dead chat stays unanswered (MOD-53's note).
- The command poll itself may still lease a chat run that has a pending cancel (pre-existing; the
  sweep's own cancels are graph-only since L4).
- `cargo test`/`--all-targets` builds of the `htui` binary carry the armed hook (dev-dependency
  feature unification under resolver 3); it is inert without `HTUI_TEST_KILL_POINT`. Release builds
  and `cargo install` do not carry it.
- Kill points inside fan-out candidates and judges were not added.

## Pins after this item

Store conformance `CASES` 119 (was 118: `mod-42.md`'s 116 was already stale), `READ_CASES` 14;
`htui-orch` `CASES` unchanged. Migrations 11; `crates/htui-store/.sqlx` 307 files (one swapped);
`crates/htui/tests/snapshots` 127, unchanged. The full workspace suite
(`--all-features --no-fail-fast --test-threads=1`) passed 3376 and failed 0, with 26 ignored, against
the sandbox Postgres (baseline before the item: 3352); `worker_crash_pg` ran its 7 cases in 27.8 s.
Clippy (`--workspace --all-targets --all-features -D warnings`) and `cargo fmt --check` are clean.
