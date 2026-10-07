# MOD-70 - Follow-up command rows for engine steps (done, 2026-10-07)

**Requirements:** `R-AGT-1` (send a follow-up), `R-HIS-1` (every follow-up stored after scrubbing);
constrained by `R-SEC-3`/`R-ID-7` (scrub on the executing box, fail closed).
**Origin:** MOD-42 PRD Q9 (`docs/decisions/mod/mod-42.md`, "Left open").
**Artifacts:** PRD
[`.claude/prds/mod-70-engine-follow-up.prd.md`](../../../.claude/prds/mod-70-engine-follow-up.prd.md),
plan
[`.claude/plans/mod-70-engine-follow-up.plan.md`](../../../.claude/plans/mod-70-engine-follow-up.plan.md)
(fact-checked: 141 claims, 13 partial and 5 false (4 distinct), all amended before CONFIRM) and
blueprint `.claude/plans/mod-70-engine-follow-up.blueprint.md` (findings F-1..F-25, decisions
B-1..B-22, deviations DV-1..DV-3). User guide: `docs/htui-worker.md`, "Follow-ups on worker steps".
Routed as a PRD (C2, C3, C4 fired; C4 low-confidence), ultracode for the implement phase, run in the
TOOL-7 sandbox on `hr/MOD-70`.
**Decisions:** maintainer, 2026-10-06: PRD gate Q1-Q6 as recommended, Q7 yes; Q8 closed on a wrong
premise and corrected at plan CONFIRM (OQ-3: follow-up turns count toward the step deadline and the
run cap). Plan CONFIRMED with OQ-1..OQ-6 as recommended (`i` not `f`; a `follow_up_window` table;
refuse before the session starts; a dead walk's row waits for recovery or cancel; the text is never
echoed). Blueprint DV-1..DV-3 accepted (executor fence on the owner only; `follow_up_window.owner`;
follow-up lines from column 2, a refusal wrapped to two lines).
**Commits:** `37659239`..`8e961419` on `hr/MOD-70` (PRD, plan, blueprint, tasks T0-T6 each with
implement/verify/repair, the review rounds R1 and R2) plus this close-out. Migration `0016`.

## What shipped

**The row and the window (M1, T0/T1).** Migration `0017_follow_up.sql`:
- `run_command` gains `run_step_id` and `text`. The kind `CHECK` adds `follow_up`, and a `CHECK`
  keeps `text` non-null exactly while a follow-up row is `pending`.
- The pending index splits in two: one pending cancel per run, one pending follow-up per step.
- A new table, `follow_up_window` (one row per step session that accepts follow-ups, with its
  `owner`, `opened_at` and `closed_at`), is not mirrored. It is the compare-and-set's target.
- Indexes back every foreign key (review M-1).

Six `WriteStore` methods (MemStore is the reference, PgStore in `pg/relay.rs`), five forwarded by
`RelayStore` (now nine), and nine shared conformance cases:
- `request_follow_up` refuses in a fixed order: chat run, judge, not running, cancelling, already
  queued, executor gone, not started, session ended. The enqueue locks only the window row
  (`FOR SHARE OF w`).
- `open_follow_ups`, `next_follow_up` and `settle_follow_up` serve the walk; settle is a fenced
  compare-and-set that nulls the text.
- `close_follow_ups` and `close_dropped_follow_ups` are two statements each under a pinned
  `READ COMMITTED` (review M-2).

Existing methods change too: `request_cancel` refuses the run's pending follow-ups,
`pending_commands` returns cancels only (a follow-up never reaches `cancel_run`), `resolve_command`
nulls the text, and `relay_view` lists the newest follow-up per step, without its text.

**The engine verb (M1, T2/T3).** `htui_agent::record::drive` gains a follow-up loop behind
`Relay::follow_ups`.
- **Open:** the walk opens the window before the first pull.
- **Turn end:** at each turn's `done` (not after a cancelled stop, a cap breach or a cancel) it reads
  the step's pending row, pre-scrubs it with the recorder's scrubber, and claims it. It then records
  `follow_up` at `turn + 1`, sends the text, and drives to the next `done`.
- **Close:** it closes the window on every exit except a fenced one. A deadline cut or a cancel
  closes with the cancelled-session sentence.
- **Stack:** `turn` is boxed once per turn.
- **Wiring:** `drive_once` sets the flag for main and fan-out candidate sessions, never for judge
  calls.
- **Dead walks:** `stale_dropped_requests` closes a dropped walk's windows.
- **Run given back:** `release_after_walk` closes the windows of any run it gives back, parked or
  finished (review M-3, R2 P-1), so a stranded row never keeps its text past the walk.

`pump`, its call sites, judges and chat are unchanged.

**Runs-pane follow-up (M2, T5).** `i` on a running engine step opens a one-line input.
- The typing box refuses empty text, and text that still matches a credential pattern, before it
  writes anything.
- The text travels as `StoreRequest::FollowUp`.
- The pane shows "queued — sent when the current turn ends", "follow-up sent" or "follow-up
  refused" with the reason, never the text.

**Docs (T6).** The ANA-2 §4.8 amendment: the "first path" is reachable between turns. Also the
worker guide's follow-up section, upgrade note (R-8) and key row, and the README Runs key row.

**Review gate.** `rust-reviewer` approved with changes: 4 MEDIUM, 3 LOW, all applied in R1.
- M-1: indexes.
- M-2: pinned isolation.
- M-3: a finished run refuses a stranded follow-up.
- M-4: the loop's failure and retry paths under test.
- L-1: a doc sentence (the log, not the label, is the evidence of delivery).
- L-2: MemStore cleanup.
- L-3: an open-versus-enqueue race test.

R1's verifiers found that a *parked* run kept a stranded row. R2 fixed that and pinned the
closed-window re-open race.

## Decisions worth keeping

**`applied` means "taken for the next turn", not "delivered" (D6).** The claim precedes record and
send, so the step's log is the evidence of delivery, and a failed send fails the step.

**A close is two statements under pinned `READ COMMITTED` (F-18, probed).** A single data-modifying
CTE's snapshot predates a racing enqueue's commit and leaves the row `pending`. An unqualified
`FOR SHARE` locks `run` too and stalled a lease heartbeat for 1.5 s in the probe.

**Every give-back closes the run's windows (M-3, P-1).** The walk's own close is best-effort, so the
backstop is the release of the lease, parked or finished, plus the D9 close for dropped walks.

**Branches cut before `0d69e0c5` overflow the 2 MiB stack on any engine-path change.** W1's
merged gate aborted `htui --test chat`, `runs_pg` and `worker_pg`. The fix was a cherry-pick of
MOD-10 M3's dispatch-arm boxing (`4351d497`), which is the same change already on `main`.

## Left open

- No mid-turn steering: a follow-up waits for the turn's `done`. A parked permission request
  delays it until answered.
- A follow-up typed before the step's session opens is refused, not queued (OQ-4).
- Mixed versions (R-8): once `0016` is applied, a pre-MOD-70 TUI or worker still running cannot
  cancel, and its cancel poll fails on a follow-up row. Upgrade every box together; an older binary
  started later is refused at connect.
- No `LISTEN`/`NOTIFY`: the walk reads at the turn end, the pane re-reads every 5 s (MOD-43/MOD-46).
- MOD-75 (agent question tool) may reuse the window and turn-end seam for its answer turn.

## Pins after this item

- Store conformance `CASES` is 157 (was 148).
- Migrations: 16 (was 15).
- Postgres tables: 43 (was 42).
- `crates/htui-store/.sqlx`: 361 files (was 348).
- `crates/htui/tests/snapshots`: 149 (was 147).
- `StoreRequest` / `StoreReply`: 110 / 67 (were 109 / 66).
- `RelayStore`: 9 own methods (was 4); `WriteStore` gained 6.
- Unchanged: `WorkerStore` 58, `EngineParts` fields, `run_step.status` `CHECK`.

The full workspace suite passed 4857 and failed 0, with 30 ignored, against the sandbox Postgres
(`--no-fail-fast`, `--test-threads=1`, no aborts).
