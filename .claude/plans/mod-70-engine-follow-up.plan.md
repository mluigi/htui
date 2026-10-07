# Plan: MOD-70 — Follow-up command rows for engine steps

**Status: IMPLEMENTED 2026-10-07 (T0-T6, review rounds R1/R2; `docs/decisions/mod/mod-70.md`). CONFIRMED by the maintainer 2026-10-06, OQ-1…OQ-6 as recommended (`i` key, `follow_up_window`
table, follow-up turns count toward the step deadline and the run cap, refuse before the session
starts, a dead walk's row waits for recovery or cancel, text never echoed). Fact-checked at step 3.5
(141 claims, 123 true, 13 partial, 5 false (4 distinct); all amended in place, see "Verified claims").**

**Source PRD**: `.claude/prds/mod-70-engine-follow-up.prd.md`, both milestones (M1 follow-up row and
engine verb; M2 Runs-tab follow-up), with its gate decisions (maintainer, 2026-10-06, Q1-Q6 as
recommended, Q7 yes, Q8 closed; cited as **PRD Q1-Q8**). Origin: MOD-42 PRD Q9
(`docs/decisions/mod/mod-42.md:86-87`). Contracts: `docs/ANA-2.md` §4.8 (`:1217-1328`; the
"first path" row `:1265` and the MOD-4 M6 amendment `:1269-1275` that calls it unreachable),
invariants 1, 5 and 7 (`:103-108`, `:124-127`, `:132-134`).

**Requirements**: `R-AGT-1` (`docs/REQUIREMENTS.md:199-202`, "send a follow-up"), `R-HIS-1`
(`:284-286`, "every follow-up … is stored … after scrubbing"); constrained by `R-SEC-3` (`:317-320`)
and `R-ID-7` (`:80-81`, scrubbed on the host box, fails closed). `REQUIREMENTS.md` is not edited.

**Complexity**: Large. One migration (`0016`) reshaping `run_command` and adding one table, six new
`WriteStore` methods (five forwarded by `RelayStore`), three changed `WriteStore` semantics, a
follow-up loop inside `htui_agent::record::drive`, an engine flag and a dropped-walk close, a
Runs-pane input and display, `.sqlx` regeneration, and docs.

**Routing**: `/handoff-run MOD-70`, PRD path (C2, C3, C4). Ultracode accepted for the implement
phase; architect and reviewer stay plain agents. Reviewer: `rust-reviewer`.

**Numbering**: decisions **D1…**, tasks **T0…**, risks **R-1…**, open questions **OQ-1…**,
invariants **I-1…**. MOD-42's are cited as "MOD-42 D…/I-…/OQ-…".

**Tree reading**: HEAD `395100f7` (branch `hr/MOD-70`, sandbox). Three survey readers (store pins,
TUI, docs/MOD-10) plus direct reads of the seams through Gortex. Paths are relative to the repo
root. **MOD-10 M3 has not landed on this branch**: its PRD row is `pending`
(`.claude/prds/mod-10-secret-provider.prd.md:142`), `HANDOFF.md`'s MOD-10 phase note ends
"Remaining: M3 run-start injection" (`HANDOFF.md:166-175`), and the engine's scrubber is
`MinimalScrubber::new(std::iter::empty::<String>())` (`crates/htui-worker/src/runtime.rs:969`, field
`:892`), i.e. pattern rules only. MOD-67 (configurable keys) is open (`HANDOFF.md:395-409`): no key
catalogue exists on this branch.

---

## Open questions for the maintainer (read these first)

- **OQ-1 · `f` is taken by the Backlog tab; bind `i` instead.** PRD Q6 chose `f` before this
  conflict was known. The Runs pane lives inside the Backlog tab
  (`crates/htui/src/ui/tabs/backlog/detail/runs.rs`), and `BacklogTab::on_key` handles `f` itself —
  "`f` opens the filter form" (`crates/htui/src/ui/tabs/backlog/mod.rs:666-670`) — before any
  unmatched key reaches `self.detail.on_key` (`:687`); the pane sees every key first only while
  `captures_input()` is true (`:645-647`). Pinned by `f_and_shift_f_are_on_the_backlog_help_line`
  (`crates/htui/tests/backlog.rs:1808`). Routing `f` to the Runs pane first when the cursor's step
  is running would make `f` mean "filter" or "follow-up" depending on the selection. `i` ("instruct")
  is bound nowhere: not in the Backlog match (`backlog/mod.rs:655-686`), not in the Runs `action()`
  letters (`runs.rs:645-763`), not global (`crates/htui/src/keymap.rs:206-250`,
  `crates/htui/src/app/mod.rs:82-109`). **Recommended: `i`.** Alternative: `f` routed to the pane
  first only on a running engine step.
- **OQ-2 · A durable "session open" marker: one new table, `follow_up_window`.** Q5's
  compare-and-set needs something that changes when the walk's session ends. Today nothing does:
  the step stays `running` through verify, capture and settle (`crates/htui-orch/src/engine.rs:3586-3660`,
  `finish_step` only after them), and the recorder's last write is `run_step.usage`
  (`crates/htui-agent/src/record.rs:1301-1312`). A column on `run_step` would bump its
  `updated_at` through `trg_run_step_updated_at` (`crates/htui-store/migrations/0001_init.sql:574-579`;
  MemStore stamps it by hand as the trigger's stand-in, `mem.rs:1766`, `:1796`), so every open and
  close would read as a step change to the mirror's sync cursors, and the write would have to be
  fenced and stamped on both backends. A separate table avoids that churn and gives a natural
  `closed_at`. The table holds one row per step whose
  engine session accepts follow-ups (`closed_at` set when the session ends) and is not mirrored.
  Without it, a follow-up queued during verify would wait minutes and then be refused at step end.
  **Recommended: the table (D1, D4).**
- **OQ-3 · Follow-up turns do run under limits: the step deadline and the run cap.** PRD Q8 closed
  as "no step timeout or usage cap exists". Both exist on the engine path: `deadline_seconds`
  defaults to 7200 s through the project's `step_deadline_seconds` and the app setting
  (`crates/htui-orch/src/graph.rs:29-30`, `:740-745`), and `drive_once` wraps the whole boxed `drive`
  future in `drive_with_deadline` (`engine.rs:6140-6151`); the per-run token cap is
  `SessionSpec.budget_micros = settings.per_token_cap_run` (`engine.rs:6108`) and the recorder's cap
  (`record.rs:616`, `:630`, `enforce_breach` `:2086-2106`). Q8 is not re-opened: nothing new limits a
  follow-up. **Recommended: accept and document** — a follow-up turn counts toward the step's
  deadline and the run cap like any turn; a cut turn settles `DeadlineElapsed`, a breach ends the
  session, and no follow-up is applied after either (D7).
- **OQ-4 · A follow-up typed before the step's session opens is refused, not queued.** The window
  opens when `drive` starts (D6); a `running` step still preparing its tree or prompt has none. The
  refusal says "the step's session has not started yet". **Recommended: accept.**
- **OQ-5 · A follow-up left behind by a dead walk stays pending until the run is re-taken or
  cancelled.** A crash, a hard drop at shutdown (MOD-42 OQ-5) or a lost fence leaves the window
  open and the row `pending`. It is refused when this process re-takes a run it dropped
  (`renew_lease` on a `DeadWalks` run, `abandoned` with a live lease), when the sweep adopts the run
  (crash recovery, `engine.rs:2329`, `:2345-2348`), or when the run is cancelled (D10). A command
  `take_lease` by a fresh process on a crashed run's lapsed lease (`engine.rs:2012`) closes nothing;
  the sweep or a cancel does. New follow-ups are refused once the
  lease is no longer live (D3). This mirrors MOD-42 OQ-2 (a cancel whose executor is gone stays
  pending). **Recommended: accept.**
- **OQ-6 · The text is never echoed back.** The relay view shows "queued — sent when the current
  turn ends" and the resolution, never the text: it leaves the database only to the executing
  process. **Recommended: accept.**

---

## Summary

A user watching an engine-walked `running` step types a follow-up in any TUI. The typing box
refuses a text that still matches a credential pattern and otherwise writes a pending
`follow_up` `run_command` row for the step. The walking process — in the TUI, a same-box worker or
another box's worker — checks for that row at each turn end of the step's main session, before it
releases the session. It claims the row with a lease-fenced compare-and-set that clears the text,
records a scrubbed `follow_up` at `turn + 1`, sends the text into the live session, and drives to
the next `done`. Then verify, capture, settle and the gate run as today. When the session ends, the
walk closes the step's window and refuses whatever is still pending ("the step finished its
session; promote it to continue"). A cancel refuses the run's pending follow-ups. No step status is
added.

## Invariants (every task keeps these)

- **I-1 · Only the walking process writes the transcript.** The enqueuer writes only `run_command`;
  the `follow_up` `session_event` is written by the executor's fenced recorder (MOD-42 I-1;
  `record_follow_up`, `record.rs:772-795`).
- **I-2 · A follow-up is applied only by the live lease holder of its run, into its own open
  window's session.** `settle_follow_up` moves `pending → applied` only while
  `run.lease_owner = owner` and the lease is live (MOD-42 D3/D4 shape).
- **I-3 · Every status move is a compare-and-set; zero rows is reported, never retried blindly**
  (ANA-2 inv. 1, `docs/ANA-2.md:103-108`). A follow-up row's text is non-`NULL` exactly while it is
  `pending` (a `CHECK`, D1).
- **I-4 · Times come from `clock_timestamp()`** on Postgres and the handle's clock on MemStore
  (MOD-42 I-4).
- **I-5 · No plaintext beyond the pending window.** The text is pattern-checked before any write
  (D2), is never in `RelayView`, never in a `Debug` output or a log, and is nulled when the row
  resolves. The only lasting copy is the transcript row, scrubbed by the executor's scrubber
  (`R-SEC-3`, `R-ID-7`).
- **I-6 · No new step status; follow-ups are applied only while the session is live** — before
  `recorder.finish` (`engine.rs:5941` main session, `:4273` candidate) and before verify, capture,
  `finish_step` and `gate::apply` (`engine.rs:3608-3663`).
  ANA-2 inv. 5 (`:124-127`) holds.
- **I-7 · At most one pending follow-up per step, and every row resolves.** It resolves through the
  walk's take, the walk's close, a newer window's open, a cancel, or a lease re-take's close of a
  dropped window (D4, D9, D11; OQ-5).
- **I-8 · `pump`, its 26 call sites, judge sessions and chat are untouched.** Follow-ups need a
  relay with `follow_ups: true` (D6), which `pump` never passes (`record.rs:2034-2054`) and the judge
  never gets (D8).
- **I-9 · `pending_commands` returns cancels only.** `poll_once` and the sweep spawn `cancel_run`
  for every row they read without looking at `kind` (`runtime.rs:2713-2731`; `cancels_first`
  `:2060-2100`); a `follow_up` row must never reach them (D5).

## Design decisions (settled here, not in code review)

### Store (M1)

- **D1 · Migration `0016_follow_up.sql`.** Next free number: the last on disk is
  `0015_command_queue.sql`. Header in the house style (`-- 0016_follow_up.sql - MOD-70 (plan D1).` /
  `-- Forward-only (R-STO-5).`, as `0011_permission_relay.sql:1-2`). Contents:
  - `run_command` gains `run_step_id UUID NULL REFERENCES run_step(id) ON DELETE CASCADE` and
    `text TEXT NULL`.
  - `chk_run_command_kind` is dropped and re-added as `kind IN ('cancel', 'follow_up')`
    (`0011:58`).
  - A new `chk_run_command_follow_up`:
    `(kind = 'cancel' AND run_step_id IS NULL AND text IS NULL) OR (kind = 'follow_up' AND
    run_step_id IS NOT NULL AND (status = 'pending') = (text IS NOT NULL))`. Existing cancel rows
    satisfy it unchanged.
  - The unique index `uq_run_command_pending ON run_command (run_id, kind) WHERE status = 'pending'`
    (`0011:61-62`) is replaced by two: `uq_run_command_pending_cancel (run_id) WHERE status =
    'pending' AND kind = 'cancel'` (one pending cancel per run, as before) and
    `uq_run_command_pending_follow_up (run_step_id) WHERE status = 'pending' AND kind = 'follow_up'`
    (PRD Q4). Two plain partial indexes rather than one expression index: each `ON CONFLICT`
    target then infers a plain column list (probed at the fact-check).
  - New table `follow_up_window` (OQ-2): `run_step_id UUID PRIMARY KEY REFERENCES run_step(id) ON
    DELETE CASCADE`, `run_id UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE` (`delete_project`
    relies on cascades, MOD-42 D1), `session UUID NOT NULL` (the `RelaySessionId` `drive_once`
    mints, `engine.rs:6121`), `opened_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()`,
    `closed_at TIMESTAMPTZ NULL`; index `(run_id) WHERE closed_at IS NULL`.
  - Not mirrored: neither table reaches `cache_migrations/` (the relay module doc says so,
    `crates/htui-core/src/model/relay.rs:1-2`), and `refresh_run_step` selects an explicit column
    list (`crates/htui-store/src/cache/refresh.rs:1213-1219`). `schema_version` becomes 16, so each
    box rebuilds its mirror once (`crates/htui-store/src/cache/mod.rs:131-152`). A headless worker
    never migrates; migrate from a TUI first (`docs/htui-worker.md:270`).
  - `run_step.status`'s `CHECK` is unchanged (PRD metric "No new step state").
- **D2 · The text type: `FollowUpText`, checked at construction.** A newtype in
  `crates/htui-core/src/model/relay.rs`. `FollowUpText::new(String) -> Result<Self,
  FollowUpTextError>` refuses a text that is empty after trimming (`Empty`) or that a pattern-only
  `MinimalScrubber::new(Vec::<String>::new())` (`crates/htui-core/src/scrub.rs:196-202`) refuses
  when scrubbing `{"text": …}` (`Residue(Unmasked)`; `Unmasked` carries a pointer and a rule name,
  never the text, `scrub.rs:157-174`). The text is stored as typed (PRD Q3: the agent gets it as
  typed). `Debug` prints the length only. No serde unless a derive on its container demands it; if
  so, `#[serde(try_from = "String")]` so a deserialised text is checked too. Every enqueue path takes
  a `FollowUpText`, so the typing box refuses a residue before any request is sent or any row is
  written (PRD Q3).
- **D3 · Enqueue: `WriteStore::request_follow_up(NewFollowUp) -> FollowUpRequest`.** It is on
  `WriteStore` only, the answerer side, like `answer_permission` (`traits.rs:1843`; MOD-42 D2), and
  is not forwarded by `WorkerStore`: a headless worker never enqueues. `NewFollowUp { id:
  RunCommandId, run_step_id, text: FollowUpText, issued_by, issued_box }`. `FollowUpRequest =
  Queued(RunCommandId) | Refused(FollowUpRefusal)`. One insert of the row is admitted only if,
  under a `FOR SHARE OF w` lock on the step's window row only, every guard below holds. The window
  is an inner join (`FOR SHARE` cannot target the nullable side of an outer join), and the lock is
  qualified: an unqualified `FOR SHARE` locks every `FROM` relation and stalled a concurrent lease
  heartbeat for 1.5 s in the fact-check probe. On zero rows a re-read
  classifies the refusal in this order (the `request_cancel` re-read shape,
  `crates/htui-store/src/pg/relay.rs:366-431`):
  1. unknown step → `NotFound { run_step }`; unknown actor → `Constraint` (`require_actor`);
  2. chat run → `ChatRun`;
  3. `fanout_index < 0` (the judge step, `crates/htui-core/src/model/run.rs:236-238`) → `Judge`;
  4. step status ≠ `running` → `NotRunning`;
  5. a pending cancel on the run → `Cancelling`;
  6. a pending follow-up on the step → `AlreadyQueued`;
  7. the run's lease is not live → `ExecutorGone`;
  8. no window row → `NotStarted`; a closed window (`closed_at` set) → `SessionEnded`.

  The `FOR SHARE OF w` on the window row against D4's `UPDATE … SET closed_at` is what makes Q5 a
  compare-and-set (probed under READ COMMITTED): an enqueue that commits first is refused by the close, and one that waits behind
  the close reads it closed.
- **D4 · The executor's methods: `RelayStore` grows from four to nine.** Each is also a same-named
  `WriteStore` method, because the forwarding convention needs a same-named target (MOD-42 D2;
  `RelayStore` `crates/htui-core/src/store/worker.rs:74-97`). `open_follow_ups`, `settle_follow_up`
  and `close_dropped_follow_ups` take the executor's `owner` and are fenced on
  `run.lease_owner = owner AND lease_expires_at > clock_timestamp()`, as `apply_permission` is;
  `next_follow_up` and `close_follow_ups` are scoped by the window's `session` instead (a stale
  session's window was already replaced or closed):
  - `open_follow_ups(run, step, session, owner) -> bool`: upserts the window (`session`,
    `closed_at = NULL`) and refuses, with `SessionEnded`'s sentence, any row still pending on the
    step from an older window. This mirrors `open_permission` staling older sessions' rows (MOD-42
    D5). `false` = not the lease owner.
  - `next_follow_up(step, session) -> Option<QueuedFollowUp>`: the step's pending row while the
    window is this session's and open. `QueuedFollowUp { id, text: String }` has a `Debug` that
    prints the length only.
  - `settle_follow_up(id, owner, to: FollowUpSettle) -> SettleOutcome`: a CAS `pending → applied`
    (resolution `NULL`) or `pending → refused` (resolution = the sentence), always `text = NULL`,
    `resolved_at = clock_timestamp()`. `SettleOutcome = Settled | NotPending | Fenced`; the last two
    are told apart by a re-read.
  - `close_follow_ups(step, session, reason) -> u64`: in one READ COMMITTED transaction of **two
    separate statements** — `UPDATE` the window's `closed_at` where it is this session's, then
    refuse the step's pending rows with `reason`. Never one data-modifying CTE: its single snapshot
    predates a racing enqueue's commit and left the row `pending` in the fact-check probe.
  - `close_dropped_follow_ups(run, owner, reason) -> u64`: closes every open window of the run and
    refuses every pending follow-up of the run, only while `owner` holds the live lease (D9); the
    same two-statement shape. Idempotent: the sweep can reach it twice (`engine.rs:2329` via
    `renew_lease`, then `:2347`).

  Implementors, from the survey: `RelayStore` — MemStore (`store/worker.rs:528-549`), PgStore
  (`crates/htui-store/src/worker.rs:57`), Writer (`:394`), `NoRelay`
  (`crates/htui-agent/src/record/relay.rs:142-169`, `match *self {}`), and the test doubles
  `FailingReads`/`FailingSettles` (`crates/htui-agent/tests/relay.rs:467`, `:510`). `WriteStore` —
  MemStore, PgStore (`pg/write.rs` forwarding near `:6591-6614`), Writer
  (`crates/htui-store/src/writer.rs:1416-1452` region), `UsageSpy`
  (`crates/htui-agent/src/conformance.rs:751`, forwards `:1431-1450`), `SpyStore`
  (`crates/htui-agent/tests/recorder.rs:437`, forwards `:1155-1174`). `WorkerStore` keeps its 58
  own methods; its module doc (`store/worker.rs:15-18`) is updated to say `RelayStore` is nine.
- **D5 · Changed `WriteStore` semantics:**
  - `pending_commands` returns `kind = 'cancel'` rows only. MemStore's `applies` filter today has
    no kind test (`mem.rs:6915-6937`), and neither does the Pg SQL (`pg/relay.rs:441-458`). The
    runtime is unchanged (I-9).
  - `request_cancel` also refuses every pending follow-up of the run, with "the run was
    cancelled", in the same statement or transaction as its insert. Its
    `ON CONFLICT (run_id, kind) WHERE status = 'pending'` target (`pg/relay.rs:376`) becomes
    `ON CONFLICT (run_id) WHERE status = 'pending' AND kind = 'cancel'`, matching D1's index.
  - `resolve_command` always sets `text = NULL` (a no-op for cancels), so no caller can break D1's
    `CHECK`.
  - `relay_view` gains `follow_ups: Vec<FollowUpView>`: the newest follow-up row of each step of
    the item's non-terminal runs (the `cancels` scope, `relay.rs:221-226`). `FollowUpView { id,
    run_id, run_step_id, status, resolution, issued_at, resolved_at }` has no text (OQ-6).
  - `RunCommand` (`relay.rs:195-216`) is **not** changed: it stays the cancel row
    `pending_commands` returns. Adding a field would break the Pg `query_as!(RunCommand, …)`
    (`pg/relay.rs:439-458`) before T1 regenerates `.sqlx`.
- **D-model · Types.** In `model/relay.rs`: `RunCommandKind::FollowUp => "follow_up"` (its doc
  "a follow-up command adds a kind later" is fulfilled), `FollowUpText`, `FollowUpTextError`,
  `NewFollowUp`, `FollowUpRequest`, `FollowUpRefusal` (`thiserror`, `Display` = the sentences in
  D12), `QueuedFollowUp`, `FollowUpSettle`, `SettleOutcome`, `FollowUpView`, and
  `RelayView.follow_ups`. No new id type: the window is keyed by `StepId` and carries a
  `RelaySessionId` (`model/ids.rs`). The CHECK-list test `run_command_kind_matches_check_list`
  (`crates/htui-core/src/model/mod.rs:372-376`) becomes `["cancel", "follow_up"]`.

### Agent loop (M1)

- **D6 · The loop lives in `drive`, behind `Relay::follow_ups`.** `Relay` (`record/relay.rs:101-120`)
  gains `follow_ups: bool`. With a relay and `follow_ups: true`, `drive` (`:200-224`) does this:
  1. **Open.** `open_follow_ups` runs before the first pull. `false` → `Err(Store(Fenced { step }))`.
     A transient error is a `warn`, and the session runs without a window: a follow-up is then
     refused `NotStarted`, and the step itself is unaffected.
  2. **Turn.** It loops over `turn` (`:233`). `turn` returns `Ok(done)` at the turn's `done`
     (`:273`), already recorded.
  3. **Turn end (PRD Q1).** It returns `Ok(done)` without looking for a follow-up when any of these
     holds:
     - `done.stop_reason == StopReason::Cancelled` — `enforce_breach` returns exactly that after a
       cap breach (`record.rs:2101-2105`), and a cut or cancelled session never reaches here;
     - `recorder.cap_breach()` is `Some` (`record.rs:630`);
     - `control.signal().is_cancel()`.

     Otherwise it calls `next_follow_up(step, session)`; `None` → return `Ok(done)`.
  4. **Pre-scrub.** It scrubs a copy of `{"text": text}` with `recorder.scrubber` — the same scrubber
     and payload shape `record_follow_up` uses (`record.rs:780-787`). This is visible from the child
     module, as `scrubbed()` already uses it (`relay.rs:348`). On `Unmasked`, it calls
     `settle_follow_up(Refused(executor-scrub sentence))` and goes back to step 3. The text is never
     recorded, so `record_follow_up`'s `refuse` path (`record.rs:1686-1691`) — a `scrub_residue` row
     that makes `finish` fail the step (`:1131-1137`) — is not reachable from a follow-up. Before
     MOD-10 M3 this scrubber is pattern-only, the same rules as D2, so the branch only fires once M3
     adds masks. The seam is the recorder's scrubber (`EngineParts::scrubber`, `engine.rs:470`), so
     M3 needs no MOD-70 change.
  5. **Claim.** `settle_follow_up(Applied)`: `Settled` → go on; `NotPending` (a cancel or a newer
     window refused it) → step 3; `Fenced` → `Err(Store(Fenced))`, which `drive_once` surfaces as
     `EngineError::Driver(Store(Fenced))` (`engine.rs:6173-6177`) and `heartbeaten` turns into
     `LeaseLost` (`engine.rs:1955-1964`, `is_fenced` `:6630`).
  6. **Record.** `recorder.record_follow_up(&text, (relay.now)())`: `turn + 1`, scrubbed (PRD Q1,
     `R-HIS-1`). This mirrors the judge's record-then-drive order (`engine.rs:5151-5153`).
  7. **Send.** `session.send_follow_up(text)` (`driver.rs:435-439`); an error ends the session as a
     failed turn would. Then back to step 2.
  8. **Close** on every exit except a fenced one (MOD-40 D1: a fenced writer writes nothing more),
     beside the existing `settles_stale` post-step (`relay.rs:206-221`):
     `close_follow_ups(step, session, reason)`, with `reason` = `SessionEnded` for `Ok`, the
     cancelled-session sentence for `Err(Cancelled)`, and `SessionEnded` otherwise. `drive` cannot
     tell a deadline cut from a run cancel (both arrive as `Signal::Cancel`; `cut` is known only in
     `drive_once`, `engine.rs:6150-6162`), so a deadline cut also closes with the cancelled-session
     sentence. It is best-effort with the parked poll's bounded transient retry
     (`TRANSIENT_READS`, `relay.rs:33`; at most 30 × 1 s after the session ends); a final failure
     is a `warn` (R-3).

  `applied` therefore means "taken by the executor for its next turn"; the transcript row is the
  evidence it was sent. Without a relay, or with `follow_ups: false`, `drive` is byte-for-byte
  today's (I-8).
- **D7 · Limits (OQ-3).** A follow-up turn runs inside `drive_once`'s deadline composite and under
  the run cap, unchanged. After a cut (`Err(Cancelled)` with `cut`, `engine.rs:6157-6167`) or a
  breach, no follow-up is applied; the close refuses any pending row.
- **D-stack · No new future nesting on the walk path.** The loop runs inside `drive`'s own future,
  which `drive_once` already boxes (`Box::pin(drive(…))`, `engine.rs:6147`) and `pump` boxes
  (`record.rs:2047-2053`). If `turn`'s state machine sits inside the loop's, `turn` is boxed too.
  The gate runs `every_case_name_dispatches` (`crates/htui-orch/src/conformance.rs:7794`) with
  `--no-fail-fast` and greps for SIGABRT (mod-42.md:77-79).

### Engine (M1)

- **D8 · `drive_once` sets `follow_ups: step.fanout_index >= 0`.** The `Relay` literal is at
  `engine.rs:6116-6126`. `drive_once` drives three kinds of session:
  - the main step, via `session` (`engine.rs:5899-5943`);
  - each fan-out candidate, via `candidate_live` (`:4252`);
  - each judge call, via `judge_calls` (`:5138`, `drive_once` at `:5160`) on the judge step, whose
    `fanout_index` is `-1` (`model/run.rs:236-238`).

  So main and candidate sessions open a window and judge calls never do (PRD Q2). `EngineParts`
  is unchanged.
- **D9 · A dropped walk's windows close where its permission rows go stale.**
  `stale_dropped_requests` (`engine.rs:2142`) first calls `close_dropped_follow_ups(run, owner,
  SessionEnded)`. Its three call sites already guarantee that this process holds the lease and that
  no walk of the run is live here:
  - `renew_lease` on a dead walk (`:2061`, through `stale_dropped_requests_of`);
  - `abandoned` after a successful `refresh_lease` (`:2122`, the same);
  - sweep recovery under the fence (`:2347`).

  `renew_lease` reaches it only for this process's own `DeadWalks`; a crashed process's walk is
  reached only by the sweep (OQ-5).

  A chat run never reaches it (`:2169-2176`). This closes OQ-5's rows on every re-take.

### Cancel (M1)

- **D10 · Cancel interplay (PRD Q4).**
  - `request_cancel` refuses the run's pending follow-ups (D5).
  - `request_follow_up` refuses while a cancel is pending (D3 step 5).
  - A follow-up that slips in between the two statements, under READ COMMITTED, is refused by the
    walk's close when the cancel reaches it (`Err(Cancelled)` → D6 step 8), or by D9 when the walk
    was dropped. R-5 covers this.
  - The queued-run cancel path writes no row (`runtime.rs:2585-2603`), and a queued run has no
    running step.
- **D11 · The runtime is untouched.** `cancel_run`, `poll_once` and `cancels_first`
  (`runtime.rs:2540`, `:2698`, `~:2048`) keep reading `pending_commands`, which D5 restricts to
  cancels. Follow-ups are consumed by the walk itself, never by the 1 s command poll.

### TUI (M2)

- **D12 · Sentences** (constants in `model/relay.rs`; the `Display` of `FollowUpRefusal` and the
  resolutions written to `run_command.resolution`):
  - `NotRunning`: "only a running step takes a follow-up; p promotes a parked or failed step to a
    chat" (PRD Q2: the reason points at `p`);
  - `Judge`: "a judge session takes no follow-up";
  - `ChatRun`: "a chat takes follow-ups in its own view";
  - `AlreadyQueued`: "a follow-up is already queued" (PRD Q4, verbatim);
  - `SessionEnded`: "the step finished its session; promote it to continue" (PRD Q5, verbatim);
  - `NotStarted`: "the step's session has not started yet" (OQ-4);
  - `Cancelling`: "the run is being cancelled"; the resolution a cancel writes: "the run was
    cancelled";
  - `ExecutorGone`: "the process walking the step no longer holds the run";
  - the executor-scrub resolution: "the executing box's scrubber refused the text ({rule})";
  - the cancelled-session resolution: "the step's session was cancelled before the follow-up was
    sent";
  - typing side (D2, nothing written): `Empty` → "a follow-up needs text"; `Residue` → "not sent:
    the text looks like it holds a credential ({rule})";
  - display: pending → "queued — sent when the current turn ends" (PRD Q7, verbatim, whether or not
    a permission request is parked); applied → "follow-up sent"; refused → "follow-up refused" and
    the resolution.
- **D13 · The request: `StoreRequest::FollowUp { step: StepId, text: FollowUpText }` → `StoreReply::FollowUpQueued
  { step }`.** It is served by the store worker in `try_serve`, mirroring the `AnswerPermission`
  arm (`crates/htui/src/store_worker.rs:1988-2015`):
  - offline it is refused with `DATABASE_UNREACHABLE`;
  - it takes the user and box, then calls `WriteStore::request_follow_up`;
  - `Refused(why)` → `Failed { request: "follow_up", message: why.to_string() }`.

  The variant goes after `AnswerPermission` (`:844-849`), the name arm after `:1123`, and the reply
  after `PermissionAnswered` (`:1392-1397`). It is not an `Orch` command: like an answer, it never
  touches the run runtime (`crates/htui/src/run_worker.rs:126-136` routes only `Orch`, `RunStream`
  and `RunActions`). `FollowUpText`'s redacting `Debug` keeps the text out of any `StoreRequest`
  log line.
- **D14 · The Runs pane.**
  - **Key** (OQ-1: `i`): `i` joins `RunsTab::on_key`'s letter pattern (`runs.rs:1468-1470`, so the
    end-gesture-on-capture path runs) and is handled before `action()`'s `NOT_LOADED` early return
    (`:649-652`) and its verdict lookup (`:705-708`): a follow-up has no `RunActions` verdict.
    `modal_key`'s exhaustive `Mode` match (`:579-623`) gains the `FollowUp` arm; a `FOLLOW_UP`
    request-name const sits beside `ANSWER_PERMISSION` (`:113`). On a step whose `RunStepSummary.status` is `Running`, whose
    `fanout_index >= 0` and whose run's `kind` is `Graph` (`model/run.rs:751-789`, `:835-865`), it
    opens `Mode::FollowUp { run, step, field: TextField::new() }`. Otherwise it puts D12's sentence
    on the status line and sends nothing; the store re-checks every guard.
  - **Mirror** the `x` reject-note flow: `Mode::RejectNote` (`runs.rs:277-285`), opened at
    `:713-719`, `reject_key` (`:838-863`), `captures_input` (`:1484`), `on_paste` (`:1490-1498`), and
    the footer with title, `field.line` and hint (`:1790-1798`). The hint is "Enter send · Esc
    cancel".
  - **Enter** → `FollowUpText::new(field.text().unwrap_or_default())` (`TextField::text()` is
    `Option<&str>`, `ui/text_field.rs:240`; mirrors `reject_key`, `runs.rs:847`):
    - `Empty` → status line, stay in the mode (the `NOTE_NEEDED` shape, `:94`);
    - `Residue` → status line, stay in the mode so the text can be edited;
    - `Ok` → send `StoreRequest::FollowUp`, back to Browse.
  - **Replies:** `FollowUpQueued` re-reads the relay view, as `PermissionAnswered` does
    (`:1640`); `Failed { request: "follow_up" }` re-reads as well (`:1641-1645` shape).
  - **Display:** a step with a `FollowUpView` takes one more line (pending, applied) or two
    (refused: the label, then the resolution fitted to 43 columns). The view's cache is the
    existing `relay` field (`:201`); it is drawn in `list_lines` (`:1751`) and `flow_head`
    (`~:1734`) beside `permission_lines` (`:1274`), each line exactly `PANE` wide, mirroring
    `permission_lines` (`blank(INDENT)` + `cells::fit(text, PANE - INDENT)`, `:1276-1289`).
  - The module doc's key table (`:16-30`) gains the row.
- **D15 · Counts.** These move and are restated at close-out from a fresh count:
  - `StoreRequest` 109 → 110 and `StoreReply` 66 → 67 (the survey's mechanical count; mod-42.md's
    93/54 is stale). The compile-time breakers are the exhaustive `name()` (`store_worker.rs:1015-1147`)
    and `try_serve`.
  - Store conformance `CASES` 148 → 157 (`conformance.rs:57-206`; pins
    `crates/htui-core/tests/mem_store.rs:37` and `crates/htui-store/tests/pg_conformance.rs:31`, `:38`).
  - Migrations 15 → 16; Postgres `TABLES` 42 → 43.
  - `.sqlx` 348 → N; snapshots 147 → 149; `RelayStore` 4 → 9.
  - Unchanged: `WorkerStore`'s 58 own methods and `EngineParts`' fields.

### Scope guards

- **D16** No mid-turn steering, no `LISTEN`/`NOTIFY`, no promote of worker-walked steps, no chat
  entry point, no payload encryption (PRD out of scope). Chat's `run_turn` is unchanged (MOD-42 D7).
  ANA-2 inv. 7 ("every refusal persists", `:132-134`) is about orchestration refusals: an enqueue
  refusal writes nothing and goes to its sender (the MOD-42 OQ-1 precedent), while a resolved row
  keeps its sentence.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store trait | `crates/htui-core/src/store/worker.rs:74-97` (`RelayStore`) | No default bodies; `-> impl Future + Send`; UFCS forwarding to a same-named `WriteStore` method |
| CAS + re-read | `crates/htui-store/src/pg/relay.rs:366-431` (`request_cancel`) | One insert or update, zero rows → one re-read that classifies, bounded retries |
| Fenced apply | `htui-agent/src/record/relay.rs:390-414` (`apply_permission` use) | CAS on the live lease owner; `None` → `Fenced { step }` |
| Older-session staling | MOD-42 D5 `open_permission` | A new session's open refuses the step's older rows |
| Dropped-walk cleanup | `crates/htui-orch/src/engine.rs:2131-2166` (`stale_dropped_requests`) | Only under this owner's lease with no live walk |
| Migration | `crates/htui-store/migrations/0011_permission_relay.sql:1-12` | Header, then rationale; `ON DELETE CASCADE`; `clock_timestamp()` |
| Record then drive | `engine.rs:5151-5153` (`judge_calls`) | `record_follow_up` before the next turn |
| Turn-level policy | `crates/htui/src/agent_worker.rs:4345-4372` (chat's follow-up) | `send_follow_up` + `record_follow_up` between turns |
| Typed-note input | `crates/htui/src/ui/tabs/backlog/detail/runs.rs:713-719`, `:838-863`, `:1790-1798` | `Mode` variant + `TextField` + footer |
| Offline-refused write | `crates/htui/src/store_worker.rs:1988-2015` | `writer().ok_or_else(Unreachable(DATABASE_UNREACHABLE))` |
| Conformance | `crates/htui-core/src/store/conformance.rs` (`CASES` `:57-206`) | One case per behaviour, run on MemStore and PgStore |
| Postgres e2e | `crates/htui/tests/worker_pg.rs:845` (`a_worker_parked_step_resumes_on_an_answer_from_another_box`), `crates/htui/tests/runs_pg.rs:1599` | A second `PgStore` with another box id plays the other TUI |
| Snapshot | `crates/htui/tests/backlog.rs:1913` (`runs_pane_shows_a_pending_permission`) | `Harness::over` MemStore, row seeded through the store |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/model/relay.rs` | UPDATE | D2, D-model, D12 | T0 |
| `crates/htui-core/src/model/mod.rs` | UPDATE | re-exports; kind CHECK-list test | T0 |
| `crates/htui-core/src/store/traits.rs` | UPDATE | six methods, D5 docs, module doc `:35-38` | T0 |
| `crates/htui-core/src/store/worker.rs` | UPDATE | `RelayStore` +5, MemStore impl, module doc | T0 |
| `crates/htui-core/src/store/mem.rs` | UPDATE | reference semantics, cascade (`:4206-4210`) | T0 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | +9 cases | T0 |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | 148 → 157 | T0 |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | 148 → 157 | T0 |
| `crates/htui-store/src/pg/write.rs`, `writer.rs`, `worker.rs` | UPDATE | forwarding | T0 |
| `crates/htui-store/src/pg/relay.rs` | UPDATE | T0 placeholders; T1 SQL | T0, T1 |
| `crates/htui-agent/src/conformance.rs`, `tests/recorder.rs` | UPDATE | spy forwarding only | T0 |
| `crates/htui-agent/src/record/relay.rs` | UPDATE | T0 `follow_ups` field + `NoRelay` stubs; T2 loop | T0, T2 |
| `crates/htui-agent/tests/relay.rs` | UPDATE | T0 literals/doubles; T2 tests | T0, T2 |
| `crates/htui-orch/src/engine.rs` | UPDATE | T0 literal `follow_ups: false`; T3 D8, D9, tests | T0, T3 |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T0 test literal (`:5238`); T5 D14 | T0, T5 |
| `crates/htui-store/migrations/0016_follow_up.sql` | CREATE | D1 | T1 |
| `crates/htui-store/.sqlx/*` | UPDATE | regenerated | T1 |
| `crates/htui-store/tests/pg_criteria.rs`, `migrations.rs`, `connect.rs` | UPDATE | Postgres cases; pins | T1 |
| `crates/htui/src/store_worker.rs` | UPDATE | D13 | T5 |
| `crates/htui/tests/backlog.rs`, `tests/snapshots/*` | UPDATE/CREATE | D14 tests and snapshots | T5 |
| `crates/htui/tests/worker_pg.rs`, `runs_pg.rs` | UPDATE | end-to-end | T4 |
| `docs/ANA-2.md`, `docs/htui-worker.md`, `README.md` | UPDATE | D17 | T6 |

## Tasks

Order: **T0 → {T1 ∥ T2 ∥ T5} → T3 → T4 → T6**. TDD throughout: each task's tests are written first
and fail for the stated reason. Each implementer commits incrementally (uncommitted work dies with
the session). Independence is decided by the file-set intersections under "Verified claims" and
"Task independence" below, not by this prose.

### T0: Contracts and the MemStore reference (M1; serial, first)
- **Action**:
  - the D-model types and D2's `FollowUpText`;
  - the six `WriteStore` methods and D5's changed semantics;
  - `RelayStore` +5;
  - MemStore implementations (`State` gains `follow_up_windows: HashMap<StepId, Window>` and
    follow-up rows in `run_commands`; the per-handle clock; the cascade);
  - forwarding for Writer, PgStore, `UsageSpy` and `SpyStore`; the `RelayStore` impls for MemStore,
    PgStore, Writer, `NoRelay`, `FailingReads` and `FailingSettles`;
  - `Relay::follow_ups` with `false` at all three literals (`engine.rs:6116`, `tests/relay.rs:177`,
    `:1452`), and `RelayView.follow_ups` at its literal sites (`mem.rs:6831`, `pg/relay.rs:546`,
    `runs.rs:5238`).

  **PgStore's new bodies return `Err(StoreError::Backend("MOD-70 T1: not yet implemented".into()))`**
  so the workspace compiles; T1 replaces every one. Pg `relay_view` returns `follow_ups:
  Vec::new()` until T1.
- **Tests first**:
  - model unit tests: `FollowUpText` refuses empty and whitespace-only text and a
    `sk-ant-api03-…` text (rule named, text absent from `Display`/`Debug`), and accepts plain prose;
    `run_command_kind_matches_check_list` reads `["cancel","follow_up"]`;
  - conformance cases (+9):
    1. `request_follow_up` refusals in D3's order (chat run, judge, not running, cancelling,
       already queued, executor gone, not started, session ended);
    2. one pending follow-up per step, while cancels stay one per run;
    3. `settle_follow_up` is a fenced CAS that nulls the text (`NotPending` vs `Fenced`);
    4. `close_follow_ups` refuses the pending row with `SessionEnded`, and a later enqueue is
       refused `SessionEnded`;
    5. `open_follow_ups` refuses an older window's row and is fenced;
    6. `request_cancel` refuses the run's pending follow-ups, and an enqueue during a pending
       cancel is refused;
    7. `pending_commands` lists cancels only;
    8. `relay_view` lists the newest follow-up per step of non-terminal runs, without text;
    9. `close_dropped_follow_ups` needs the live lease;

    plus the existing `deleting_a_project_takes_its_relay_rows` (`conformance.rs:15885`), extended
    to windows and follow-up rows.
- **Files**: `crates/htui-core/src/model/relay.rs`, `model/mod.rs`, `store/traits.rs`,
  `store/worker.rs`, `store/mem.rs`, `store/conformance.rs`, `crates/htui-core/tests/mem_store.rs`,
  `crates/htui-store/src/pg/relay.rs` (placeholders and the empty `follow_ups` only),
  `crates/htui-store/src/pg/write.rs`, `crates/htui-store/src/writer.rs`,
  `crates/htui-store/src/worker.rs`, `crates/htui-store/tests/pg_conformance.rs`,
  `crates/htui-agent/src/record/relay.rs` (field and `NoRelay` stubs only),
  `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`,
  `crates/htui-agent/tests/relay.rs` (literals and doubles only), `crates/htui-orch/src/engine.rs`
  (literal only), `crates/htui/src/ui/tabs/backlog/detail/runs.rs` (test literal only).
- **Validate**: `cargo test -p htui-core --all-features`; `SQLX_OFFLINE=true cargo check --workspace
  --all-targets --all-features`; `cargo clippy --workspace -- -D warnings`. `pg_conformance`'s nine
  new cases stay red against T0's placeholders; they are T1's acceptance.

### T1: Postgres (M1; parallel with T2 and T5)
- **Action**:
  - D1's migration;
  - the PgStore bodies in `pg/relay.rs`, replacing T0's placeholders: `FOR SHARE` on the window in
    `request_follow_up`; the close's transaction; the reshaped `ON CONFLICT`; `pending_commands`'
    kind filter; `relay_view`'s follow-ups; `resolve_command`'s `text = NULL`;
  - `.sqlx` regenerated through a migrated scratch database (`docs/hr-sandbox.md:194-210`:
    `CREATE DATABASE htui_sqlx`, `cargo sqlx migrate run --source migrations`,
    `cargo sqlx prepare -- --all-targets --all-features`);
  - the migration pins.
- **Tests first** (`pg_criteria`, two `PgStore` clients with different box ids):
  - B enqueues and A's settle applies the row; `text IS NULL` on every resolved row (a direct
    `SELECT` probe, PRD metric 3);
  - an enqueue racing a close is refused or refused-by-close, never left pending (two tasks, a
    `pg_sleep`-free interleave through the lock order);
  - B cannot enqueue after A's lease expires;
  - the CHECK rejects a follow-up row with text after resolution (a raw `UPDATE`);
  - deleting a project with windows and follow-up rows succeeds;
  - a migration test for `0016`'s content, beside the `0015` ones (`migrations.rs:511`, `:526`).
- **Files**: `crates/htui-store/migrations/0016_follow_up.sql`, `crates/htui-store/src/pg/relay.rs`,
  `crates/htui-store/.sqlx/*`, `crates/htui-store/tests/pg_criteria.rs`,
  `crates/htui-store/tests/migrations.rs`:
  - `TABLES` gains `follow_up_window` (`:29-76`) and its count 42 → 43 (`:120-127`);
  - `1..=15` → `1..=16` (`:96`, prose `:97-104`);
  - `Pending(15)` (`:1088-1089`, `:1189-1190`), `MigrationsPending(15)` (`:1198-1199`, `:1221-1222`)
    and `_sqlx_migrations` 15 (`:1373-1375`) → 16;

  and `crates/htui-store/tests/connect.rs` (`15` → `16` at `:140-142`, `:156-158`, `:242-244`).
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1`;
  `cargo sqlx prepare --check -- --all-targets --all-features` (from `crates/htui-store`);
  `pg_conformance` green, including T0's nine new cases;
  `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; no "MOD-70 T1: not yet
  implemented" left in the tree.

### T2: The follow-up loop in `drive` (M1; parallel with T1 and T5)
- **Action**: D6, D7, D-stack in `record/relay.rs`; `drive`'s doc gains the follow-up paragraph.
- **Tests first** (over `FakeSession` with a two-turn script + MemStore, `tests/relay.rs`):
  - a pending row at the first `done` → one `follow_up` row at `turn + 1` holding the scrubbed
    text, the second turn's events, the row `applied` with `text = NULL`, the window closed;
  - no row → today's single turn, the window closed;
  - a row queued after the last check → refused `SessionEnded` by the close;
  - a cap breach or a `Cancelled` stop → no follow-up applied, the row refused;
  - a cancel signalled at turn end → not applied;
  - a fenced settle → `Store(Fenced)` and no close;
  - `drive` with `follow_ups: false` → no follow-up store call (a new counting `RelayStore` double);
    for `pump` the property holds by construction (`None::<&Relay<NoRelay>>`, `record.rs:2047-2052`;
    `NoRelay` is uninhabited), pinned by the existing `drive_without_a_relay_is_pump`
    (`tests/relay.rs:1490`);
  - an executor-side refusal through a scrubber with a mask that turns a non-token-start into a
    token start, refused with the rule;
  - every existing relay and `pump` test unchanged.
- **Files**: `crates/htui-agent/src/record/relay.rs`, `crates/htui-agent/tests/relay.rs`.
- **Validate**: `cargo test -p htui-agent --all-features`.

### T5: Runs-pane follow-up (M2; parallel with T1 and T2)
- **Action**: D13, D14 (key per OQ-1's answer).
- **Tests first**:
  - in `runs.rs`'s `mod tests`:
    - the key on a running main step opens the input, and `captures_input_follows_the_mode`
      (`:3776`) or a sibling gains the row — its `driven()` fixture (`:2943`) has only `done` steps
      (`fixtures.rs:1506-1516`), so the test first sets the cursor step `Running` with
      `fanout_index >= 0`;
    - on a judge, parked or done step, the key puts D12's sentence on the status line and sends
      nothing;
    - Enter sends `StoreRequest::FollowUp`; empty Enter stays with "a follow-up needs text";
    - a credential-shaped text stays in the mode, its sentence on the status line, nothing sent;
    - Esc returns to Browse;
    - paste reaches the field;
    - pending / applied / refused lines with a 43-column pin, mirroring
      `a_step_with_a_pending_request_takes_two_more_lines_each_forty_three_wide` (`:5448`);
    - `every_step_takes_two_lines` (`:2187`) still holds for a step without a follow-up;
  - in `store_worker.rs` tests: offline `FollowUp` is refused with `DATABASE_UNREACHABLE`
    (beside `relay_reads_are_empty_offline_and_answers_are_refused`, `~:5150-5175`);
  - in `tests/backlog.rs`: `offline_the_runs_pane_asks_for_no_error` (`~:1985-2032`) gains a
    `StoreRequest::FollowUp` whose status starts `follow_up: ` and ends with `DATABASE_UNREACHABLE`
    (`tests/connection.rs:291` pins only the connection section's four requests and is not touched);
  - in `tests/backlog.rs`: snapshots `runs_follow_up_input` and `runs_follow_up_queued`
    (MemStore `Harness::over`, rows seeded through T0's MemStore; `STEP_R2_PRD` is seeded `Pending`,
    `fixtures.rs:1542`, so the seed first moves the step to `running` and opens its window).

  If OQ-1 keeps `f`, `f_and_shift_f_are_on_the_backlog_help_line` (`tests/backlog.rs:1808`) and the
  Backlog routing change join this task.
- **Files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/backlog/detail/runs.rs`,
  `crates/htui/tests/backlog.rs`, `crates/htui/tests/snapshots/*` (new files only). With `f`: also
  `crates/htui/src/ui/tabs/backlog/mod.rs`.
- **Validate**: `cargo test -p htui --features testkit -- --test-threads=1`; `cargo insta test
  -p htui --features testkit` shows only the two new snapshots (a full insta run, not a grep of
  `.snap` files).

### T3: Engine (M1; serial, after T0 and T2)
- **Action**: D8 (the flag at `engine.rs:6116`), D9 (`stale_dropped_requests` closes dropped
  windows).
- **Tests first** (`engine.rs` `mod tests`, the MOD-42 relay harness):
  - a walked step whose fake agent runs two turns, with a follow-up queued through MemStore by a
    second "client" → `follow_up` at turn 1, then verify and the gate as today, step `done`;
  - a fan-out candidate takes one;
  - a judge step never opens a window (enqueue refused `Judge`, and no window row after the judge
    call);
  - a follow-up turn cut by the step deadline settles `DeadlineElapsed`;
  - cancel during a follow-up turn → no `finish_step`, row refused;
  - `abandoned` with a live lease closes the dead window and refuses its row;
  - `a_dispatch_future_is_send` (`:14375`) still compiles;
  - `every_case_name_dispatches` passes.
- **Files**: `crates/htui-orch/src/engine.rs`.
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/orch.log;
  ! grep -q SIGABRT /tmp/orch.log`; `cargo test -p htui-worker --all-features`.

### T4: End-to-end over Postgres (M1; serial, after T1 and T3)
- **Action**:
  - a worker walks a two-turn step on box A; a second `PgStore` client on box B enqueues a
    follow-up; the transcript holds a scrubbed `follow_up` at `turn + 1` and the step completes;
    the row is `applied` with `text IS NULL`;
  - the same for an in-process walk (`runs_pg.rs`);
  - a cancel from B refuses B's pending follow-up;
  - a follow-up enqueued after the walk's last check is refused `SessionEnded`.
- **Files**: `crates/htui/tests/worker_pg.rs`, `crates/htui/tests/runs_pg.rs`.
- **Validate**: `cargo test -p htui --features testkit --test worker_pg --test runs_pg --
  --test-threads=1`.

### T6: Docs (M2; serial, last)
- **Action** (D17):
  - `docs/ANA-2.md` §4.8: a MOD-70 amendment after `:1269-1275`. As built, the first path is
    reachable for an engine step's main session between turns: a `follow_up` `run_command` is sent
    at the turn end, `turn` increments, nothing respawns. Judges excluded; state table unchanged.
  - `docs/htui-worker.md`: a row in the key table (`:120-131`) and a section "Follow-ups on worker
    steps" after "Cancelling a run the worker walks" (`:243-268`). It covers the turn-end delay,
    the deadline and run cap (OQ-3), what the user sees, OQ-5, that `0016` must be migrated
    from a TUI first, and that every box upgrades together (R-8).
  - `README.md`'s Runs key table (`:204-221`) gains the key row.
- **Files**: `docs/ANA-2.md`, `docs/htui-worker.md`, `README.md`.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; the full gate
  below.

### Task independence

- **The three parallel tasks touch disjoint files.**
  - T1 ∩ T2 = ∅ (`htui-store` vs `htui-agent`).
  - T1 ∩ T5 = ∅ (`htui-store` vs `crates/htui`).
  - T2 ∩ T5 = ∅.
- **T0 is strictly earlier than the tasks whose files it touches.** T0∩T1 = {`pg/relay.rs`},
  T0∩T2 = {`record/relay.rs`, `tests/relay.rs`}, T0∩T3 = {`engine.rs`}, T0∩T5 = {`runs.rs`}.
  `pg_conformance.rs` is T0's file; its nine new Pg cases stay red until T1 and are T1's acceptance.
- **Hidden couplings:**
  - `.sqlx` is T1's alone. From `crates/htui-store`, `cargo sqlx prepare -- --all-targets
    --all-features` runs `cargo check` on `htui-store` and its dependency `htui-core` only (probed
    with cargo-sqlx 0.9.0), so T2's and T5's in-progress edits cannot break it; `htui-core` is
    settled by T0. The real coupling is the shared tree and `target/`.
  - `conformance.rs` and its two count pins are T0's alone; no later task adds a case.
  - Snapshots are T5's alone; T6 changes no rendered text.
  - The three tasks share one `target/` (check `df -h .` before the wave).
- **The remaining edges are serial:**
  - T3 needs T2's loop and T0's flag.
  - T4 needs T1 (Postgres) and T3 (the engine sets the flag).
  - T6 is last.

## Test plan

| PRD metric | Test | Task |
|---|---|---|
| Applied across executors (in-process, same-box worker, other-box worker) | conformance on both backends; `runs_pg` in-process; `worker_pg` cross-box | T0, T1, T4 |
| Sender always learns the outcome; none pending after its step ends | close race (MemStore + Pg lock order), cancel refusal, dropped-window close | T0, T1, T2, T3, T4 |
| No unscrubbed persistence | `FollowUpText` residue refusal; `text IS NULL` probe; transcript row scrubbed; no text in `RelayView`/`Debug` | T0, T1, T2 |
| No new step state | migration diff (no `run_step` change); ANA-2 amendment, not a row | T1, T6 |
| Q7 wording | pane line pin + snapshot | T5 |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-1 · A `follow_up` row reaches `cancel_run` | High without D5 | `pending_commands` kind filter; conformance case 7 |
| R-2 · The walk-path future grows past the 2 MiB test stack | Medium | Loop inside the already-boxed `drive`; box `turn` if needed; `--no-fail-fast` + SIGABRT grep |
| R-3 · A failed close leaves a live window on a step that moved on | Low | Bounded transient retry; D9 closes it at the next lease re-take; a cancel refuses the row; the engine, when the walk gives the run back (settled terminal or parked at a gate), closes the run's windows under the lease before releasing it (review M-3, R2 P-1, best-effort: a warn) |
| R-4 · `.sqlx` drift | Medium | Scratch-DB prepare per `hr-sandbox.md:194-210` (it compiles `htui-store` + `htui-core` only); `--check` in T1's validate |
| R-5 · Enqueue and cancel interleave under READ COMMITTED | Low | Walk's cancelled close and D9 refuse the survivor; e2e cancel test |
| R-6 · Plaintext in logs through `Debug` | Medium | `FollowUpText`/`QueuedFollowUp` redacting `Debug`; unit test |
| R-7 · Turn-end delay feels slow (a long turn) | Medium | Accepted (PRD out of scope: mid-turn steering); docs say "sent when the current turn ends" |
| R-8 · Mixed versions, both directions: a worker built with `0016` against an unmigrated database; and, once `0016` is applied, a pre-MOD-70 binary cannot cancel (its `ON CONFLICT (run_id, kind) WHERE status = 'pending'` no longer infers an index: probed error) and fails `pending_commands` once a `follow_up` row exists (no kind filter, unknown kind) | Medium | Headless connect refuses (MOD-40 C5); T6 documents migrating from a TUI **and upgrading every box together** |
| R-9 · Scheduling-dependent suite and the process-wide keyring fake | Medium | Gates with `--test-threads=1`; re-run `qdrant_live` serially before calling a regression |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless: catches test-support-only code
SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/gate.log
! grep -q SIGABRT /tmp/gate.log                              # HTUI_TEST_DATABASE_URL is set in the sandbox
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

`--all-features` includes `htui`'s `testkit`, without which `crates/htui/tests/*.rs` run zero tests
and report ok.

## Acceptance

- [ ] OQ-1…OQ-6 answered at CONFIRM; the plan amended to match
- [ ] Every PRD success metric pinned by a test (table above); I-1…I-9 each named by a test
- [ ] No "MOD-70 T1: not yet implemented" left; `pending_commands` returns cancels only
- [ ] ANA-2 §4.8 amended; `docs/htui-worker.md` and `README.md` updated
- [ ] Validation passes; reviewer gate (`rust-reviewer`) findings applied or deferred with the
      maintainer
- [ ] Close-out restates the moved counts from a fresh count: store `CASES` (148 → …), migrations
      (15 → 16), Postgres tables (42 → 43), `.sqlx` (348 → …), snapshots (147 → …),
      `StoreRequest`/`StoreReply` (109/66 → …), `RelayStore` (4 → 9), `WriteStore` (+6)

## Verified claims

Step 3.5, 2026-10-06: five read-only checkers (one per area) against HEAD `32d31d54`; Postgres
behaviour probed in a scratch database (partial-index `ON CONFLICT` inference, D1's `CHECK`, the
`FOR SHARE`/close race, project-delete cascades), `cargo sqlx prepare` scope probed with a logging
fake cargo. **141 claims: 123 TRUE, 13 PARTIAL, 5 FALSE** (4 distinct; D1's expression-index remark was reported by two checkers).
Every PARTIAL/FALSE is amended in place above; "Amendment" says how. Line pins within ~5 lines
count as TRUE.

### Store

| Anchor | Claim | Verdict | Evidence | Amendment |
|---|---|---|---|---|
| Header | Origin MOD-42 PRD Q9 at docs/decisions/mod/mod-42.md:86-87; ANA-2 §4.8 :1217-1328, first-path row :1265, MOD-4 M6 amendment :1269-1275, invariants 1/5/7 at :103-108/:124-127/:132-134 | TRUE | mod-42.md:86-87 MOD-70 left-open bullet; ANA-2.md:1217 '### 4.8', :1265 follow_up row, :1269-1275 amendment, §4.9 at :1330; inv 1 :103, 5 :125, 7 :132 | — |
| Header | REQUIREMENTS R-AGT-1 :199-202, R-HIS-1 :284-286, R-SEC-3 :317-320, R-ID-7 :80-81 | TRUE | REQUIREMENTS.md lines match each requirement text | — |
| Header | MOD-10 M3 pending (prd :142), HANDOFF :166-175 ends 'Remaining: M3 run-start injection', runtime.rs:969 MinimalScrubber::new(empty) field :892; MOD-67 open HANDOFF :395-409 | TRUE | prd row 3 'pending' at :142; HANDOFF.md:174-175; runtime.rs:892 field, :969 ctor; HANDOFF.md:395 '- [ ] **MOD-67' | — |
| Header | Tree reading at HEAD 395100f7 | TRUE | 395100f7 exists; current HEAD 32d31d54 adds only the plan doc | — |
| D1 | Last migration on disk is 0015_command_queue.sql; next is 0016; 15 migrations | TRUE | crates/htui-store/migrations: 0001..0015, 15 files, last 0015_command_queue.sql | — |
| D1 | House header style '-- 00NN_x.sql - MOD-NN (plan ...).' / '-- Forward-only (R-STO-5).' as 0011:1-2 | TRUE | 0011_permission_relay.sql:1-2 exactly that shape (0015 puts both on one line) | — |
| D1 | chk_run_command_kind at 0011:58; uq_run_command_pending ON run_command (run_id, kind) WHERE status='pending' at 0011:61-62; no later migration touches run_command | TRUE | 0011:58 CHECK (kind IN ('cancel')); :61-62 unique index; only 0011 mentions run_command | — |
| D1 | Proposed 0016 (new columns, CHECK swap, chk_run_command_follow_up, two partial unique indexes, follow_up_window) applies and existing cancel rows satisfy the new CHECK unchanged | TRUE | Probe: scratch DB, 0001-0015 via psql, seeded one applied + one pending cancel, applied draft 0016 -> COMMIT ok; follow_up with text NULL while pending rejected, resolved with text rejected, resolved with text=NULL accepted | — |
| D1 | 'Two plain partial indexes rather than an expression index; no migration uses one today' | FALSE | 0001_init.sql:75 CREATE INDEX idx_box_tags ON box USING GIN ((probed_tags \|\| declared_tags)) is an expression index | Drop 'no migration uses one today' or say 'no unique expression index exists' (the GIN on box tags is the only expression index). |
| D1 | follow_up_window FKs to run_step(id)/run(id) ON DELETE CASCADE suffice for delete_project | TRUE | Probe: DELETE FROM project cascaded run, run_command (incl. follow_up rows) and follow_up_window to 0 rows | — |
| D1 | session is the RelaySessionId drive_once mints at engine.rs:6121 | TRUE | engine.rs:6116 'let relay = Relay {', :6121 session: RelaySessionId::new() | — |
| D1 | Neither table reaches cache_migrations/ (relay.rs:1-2 says so); refresh_run_step selects an explicit column list refresh.rs:1213-1219; schema_version -> rebuild cache/mod.rs:131-152; htui-worker.md:270 migrate from TUI | TRUE | cache_migrations 0001-0005 no relay tables; model/relay.rs:1-2 'Neither table is mirrored'; refresh.rs:1213-1219 explicit SELECT; cache/mod.rs:131-152 rebuild on schema_version mismatch; schema_version() = max MIGRATOR version (pg/mod.rs:690); htui-worker.md:270 heading | — |
| D1/D5 | Reshaped request_cancel ON CONFLICT (run_id) WHERE status='pending' AND kind='cancel' infers the new partial index | TRUE | Probe: after 0016, new target -> INSERT 0 0 on duplicate pending cancel; old target ON CONFLICT (run_id, kind) WHERE status='pending' -> ERROR 'no unique or exclusion constraint matching the ON CONFLICT specification' | — |
| R-8 (store side) | Mixed-version risk is only a new worker against an unmigrated DB | PARTIAL | Probe shows the converse also breaks: a pre-MOD-70 binary's request_cancel (pg/relay.rs:376 old target) errors on a 0016-migrated DB, and its pending_commands (no kind filter, pg/relay.rs:439-458) would try to decode a 'follow_up' kind it lacks | Add to R-8: after 0016 is applied, older TUIs/workers cannot cancel (ON CONFLICT inference error) and fail pending_commands once a follow-up is pending; document 'upgrade every box' in T6. |
| D2 | MinimalScrubber::new at scrub.rs:196-202; Unmasked carries pointer + rule, never text, scrub.rs:157-174; pattern-only scrubber refuses sk-ant-api03 text | TRUE | scrub.rs:196 new(impl IntoIterator<Item=String>); Unmasked struct :150-167 (path, rule); rule anthropic_api_key :36; scrub() = mask_value then find_residue -> Err(Unmasked) (:340-343) | — |
| D3 | answer_permission is WriteStore-only at traits.rs:1843 | TRUE | traits.rs:1843 async fn answer_permission; not in RelayStore (worker.rs:74-97) | — |
| D3 | request_cancel re-read shape at pg/relay.rs:366-431 | TRUE | pg/relay.rs:366 fn, insert ON CONFLICT DO NOTHING, re-read classifies NotFound/actor/AlreadyPending, CANCEL_ATTEMPTS loop, ends :431 | — |
| D3 | fanout_index < 0 is the judge step, model/run.rs:236-238 | TRUE | run.rs:235-237 doc '-1 is the judge step', field fanout_index: i32 | — |
| D3/D4 | FOR SHARE on the window row vs UPDATE closed_at makes enqueue/close a CAS: enqueue committing first is refused by the close; one waiting behind the close reads it closed | TRUE | Probe (READ COMMITTED): close-first -> blocked INSERT…SELECT…FOR SHARE OF w … ON CONFLICT returns 0 rows; enqueue-first -> close UPDATE waits, then its second statement refuses the new row. Caveat: a single-statement data-modifying CTE close left the row pending (snapshot predates the enqueue commit), so the close must be two statements | In D4 close_follow_ups state explicitly: two separate statements in one READ COMMITTED transaction (UPDATE window, then UPDATE run_command), never one CTE. |
| D4 | RelayStore at store/worker.rs:74-97 has four methods (open_permission, permission, apply_permission, settle_permissions), no default bodies | TRUE | worker.rs:74-97, 4 fns returning impl Future + Send | — |
| D4 | Every one of the five new RelayStore methods takes the executor's owner and is lease-fenced | PARTIAL | Plan's own signatures: next_follow_up(step, session) and close_follow_ups(step, session, reason) take no owner; only open/settle/close_dropped do | Either add owner to next_follow_up/close_follow_ups or reword: 'open, settle and close_dropped are fenced on owner; next and close are scoped by session'. |
| D4 | RelayStore implementors: MemStore worker.rs:528-549, PgStore htui-store/src/worker.rs:57, Writer :394, NoRelay record/relay.rs:142-169 (match *self {}), FailingReads tests/relay.rs:467, FailingSettles :510; no others | TRUE | grep 'impl … RelayStore for' finds exactly these 6; NoRelay impl :142-165; Gortex extra hits (UsageSpy, SpyStore, WriteStore) are method-set inference, not impls | — |
| D4 | WriteStore implementors: MemStore, PgStore (pg/write.rs fwd ~6591-6614), Writer (writer.rs 1416-1452), UsageSpy conformance.rs:751 (fwd 1431-1450), SpyStore recorder.rs:437 (fwd 1155-1174); no others | TRUE | impls: mem.rs:7097, pg/write.rs:895, writer.rs:328, conformance.rs:751, recorder.rs:437 only; relay fwd pg/write.rs:6567-6620, writer.rs:1380-1460, conformance.rs:1407-1460, recorder.rs:1131-1185 | — |
| D4/D15 | WorkerStore has 58 own methods; module doc store/worker.rs:15-18 says RelayStore is four | TRUE | trait WorkerStore worker.rs:103-420, 58 fns; doc :15-18 '58 methods … [RelayStore] is four' | — |
| D5 | MemStore pending_commands applies filter has no kind test (mem.rs:6915-6937); Pg SQL neither (pg/relay.rs:441-458) | TRUE | mem.rs:6915-6937 filter on status+applies only; pg/relay.rs:439-458 WHERE c.status='pending' AND lease/box only | — |
| D5 | request_cancel ON CONFLICT (run_id, kind) WHERE status='pending' at pg/relay.rs:376 | TRUE | pg/relay.rs:376 | — |
| D5 | relay_view cancels scope at relay.rs:221-226; RunCommand relay.rs:195-216; Pg query_as!(RunCommand) pg/relay.rs:439-458 | TRUE | model/relay.rs:194-214 RunCommand, :220-226 RelayView.cancels; pg/relay.rs:439 query_as!(RunCommand | — |
| D-model | RunCommandKind doc says 'A follow-up command adds a kind later'; RelaySessionId in model/ids.rs; run_command_kind_matches_check_list at model/mod.rs:372-376 | TRUE | model/relay.rs:27 (str_enum at :26-31); ids.rs:125 RelaySessionId; mod.rs:374-376 check_enum(RunCommandKind::ALL, &["cancel"]); no exhaustive match on RunCommandKind anywhere | — |
| D15 | StoreRequest 109, StoreReply 66; name() at store_worker.rs:1015-1147 | TRUE | store_worker.rs:130-1010 109 variants; :1152-1493 66 variants; name() :1015, 109 arms, closes :1147 | — |
| D15 | Conformance CASES 148 at conformance.rs:57-206; pins mem_store.rs:37, pg_conformance.rs:31, :38 | TRUE | CASES :57, '];' :206, 148 entries; mem_store.rs:37 '148,'; pg_conformance.rs:31 EXPECTED_CASES = 148, :38 message '148 since MOD-69' | — |
| D15 | Migrations 15, Postgres TABLES 42, .sqlx 348, snapshots 147, RelayStore 4 | TRUE | 15 .sql files; scratch DB public tables = 42; migrations.rs TABLES 42 entries; .sqlx 348 files; crates/htui/tests/snapshots 147 .snap (191 workspace-wide); RelayStore 4 | — |
| Patterns | Store rows: RelayStore worker.rs:74-97 UFCS forwarding; CAS+re-read pg/relay.rs:366-431; migration 0011:1-12 header, cascade, clock_timestamp; conformance CASES :57-206; fenced apply record/relay.rs:390-414 | TRUE | MemStore impl uses WriteStore::x(self,..) UFCS (worker.rs:528-549); 0011:1-12 header+rationale; record/relay.rs:397-401 apply_permission None -> Fenced | — |
| Files/T0 | traits.rs module doc :35-38 (MOD-42 relay); mem.rs cascade :4206-4210; conformance.rs:15885 deleting_a_project_takes_its_relay_rows | TRUE | traits.rs:35-38 MOD-42 paragraph; mem.rs:4206-4210 permissions/run_commands retain; conformance.rs:15885 fn | — |
| T0 | Relay literals at engine.rs:6116, tests/relay.rs:177, :1452 (all three); RelayView literals at mem.rs:6831, pg/relay.rs:546, runs.rs:5238 | TRUE | grep 'Relay {' -> exactly those 3; 'RelayView {' struct literals exactly mem.rs:6831, pg/relay.rs:546, runs.rs:5238 (others are StoreRequest/StoreReply variants) | — |
| T1 | migrations.rs pins: TABLES :29-76, count 42 :120-127, 1..=15 at :96 prose :97-104, Pending(15) :1088-1089/:1189-1190, MigrationsPending(15) :1198-1199/:1221-1222, _sqlx_migrations 15 :1373-1375; 0015 tests :511, :526 | TRUE | migrations.rs:29 TABLES, :76 '];', :120 42, :96 vec![1..15], :1088, :1189, :1198, :1221, :1373; :511 and :526 0015 docs (fn :530) | — |
| T1 | connect.rs 15 -> 16 at :140-142, :156-158, :242-244; no other crate pins the migration count | TRUE | connect.rs:140 Pending(15), :156 pending, 15, :242 Pending(15); no other Pending(15)/MigrationsPending(15)/'fifteen migrations' in crates | — |
| T1 | docs/hr-sandbox.md:194-210 describes scratch-DB sqlx prepare (CREATE DATABASE htui_sqlx, migrate run --source migrations, prepare -- --all-targets --all-features) | TRUE | hr-sandbox.md:194 '### Changing SQL queries in a run' through :210 note on --all-targets --all-features | — |

### Agent loop

| Anchor | Claim | Verdict | Evidence | Amendment |
|---|---|---|---|---|
| I-1 | follow_up session_event written by recorder's record_follow_up, record.rs:772-795 | TRUE | record.rs:772 pub async fn record_follow_up(&mut self, text:&str, at) ... ends :794; flush, turn+=1, scrub {"text":text}, push FollowUp/User | — |
| I-8a | pump has 26 call sites | TRUE | git grep '\bpump(' minus def = 26 (conformance.rs 21, tests/recorder.rs 3, tests/relay.rs 1, tests/extensibility.rs 1) | — |
| I-8b | pump never passes a follow_ups relay (record.rs:2034-2054) | TRUE | record.rs:2034-2054: pump passes None::<&Relay<'_, NoRelay>> and Control::never(); NoRelay is uninhabited | — |
| I-8c | Only pump and drive_once call record::drive in production | TRUE | drive( callers: record.rs:2047 (pump), engine.rs:6149 (drive_once); chat's run_turn does not use drive | — |
| D6-Relay | Relay struct at record/relay.rs:101-120 | TRUE | relay.rs:101 pub struct Relay<'a,R>; fields store, owner, run, step, session, policy, poll, grace, now; closes :120. Manual Debug impl :122-135 lists fields (follow_ups should be added there too) | — |
| D6-literals | Relay literals are exactly engine.rs:6116, tests/relay.rs:177, tests/relay.rs:1452 | TRUE | git grep '(^\|[^A-Za-z_])Relay \{' across all *.rs: only these 3 (engine.rs:6116-6126, tests/relay.rs:177 in relay_over, :1452 FailingReads case) | — |
| D6-drive | drive at relay.rs:200-224; turn at :233; turn returns Ok(done) at :273 already recorded | TRUE | drive :200-221 (settles_stale :225-230); turn :233; recorder.record(envelope) at :269 before `return Ok(done)` :273 | — |
| D6-session-access | drive can call session.send_follow_up and knows step/session for next_follow_up(step, session) | TRUE | drive takes session: &mut dyn AgentSession (:201); relay.step and relay.session (RelaySessionId) available via Option<&Relay>; relay.now for record_follow_up's at | — |
| D6-3a | enforce_breach returns Ok(DoneEvent{stop_reason: Cancelled}) after a cap breach (record.rs:2101-2105) | TRUE | record.rs:2086 enforce_breach; Ok(DoneEvent{stop_reason: StopReason::Cancelled}) at ~:2107-2109 (within drift); turn returns it at relay.rs:270 | — |
| D6-3b | a cut or cancelled session never reaches turn end with Ok | TRUE | control Cancel at loop top/park -> cancel() always returns Err(DriverError::Cancelled) (relay.rs cancel tail); drive_with_deadline cut only sends Cancel to local control (engine.rs:6665-6700), never drops drive | — |
| D6-3c | recorder.cap_breach() exists (record.rs:630) | TRUE | record.rs:630 pub const fn cap_breach(&self) -> Option<CapBreach>; run_cap at :617 | — |
| D6-3d | control.signal().is_cancel() available in drive | TRUE | relay.rs:77 Control::signal -> Signal; :51 Signal::is_cancel const fn; drive has control: &mut Control. Under a deadline this is the step-local control fed concurrently by forward_or_cut | — |
| D6-4a | recorder.scrubber visible from child module record/relay.rs; scrubbed() uses it at relay.rs:348 | TRUE | record.rs:386 private field `scrubber: &'a dyn Scrubber` (private items visible to descendant module record::relay); relay.rs:348 scrubbed(recorder.scrubber, ...) | — |
| D6-4b | record_follow_up scrubs payload {"text": text} (record.rs:780-787) | TRUE | record.rs:780 json!({"text": text}); :784-787 scrubber.scrub -> Err => refuse | — |
| D6-4c | refuse (record.rs:1686-1691) writes scrub_residue row; finish fails (:1131-1137) and that fails the step | TRUE | record.rs:1686-1691 refuse: note_residue + residue_row; finish :1131, returns Err(RecordError::Unmasked) at :1135-1137; engine.rs:5941 recorder.finish().await? -> failure_text maps Record(Unmasked) to RunFailure::ScrubRefused | — |
| D6-4d | Engine recorder scrubber is EngineParts::scrubber (engine.rs:470) and is pattern-only before MOD-10 M3 | TRUE | engine.rs:470 pub scrubber: &'a dyn Scrubber; htui-worker/src/runtime.rs:969 MinimalScrubber::new(empty) wired at :1048 (no secrets/masks) | — |
| D6-5 | Fenced -> Err(Store(Fenced)), which drive_once already lifts to LeaseLost (engine.rs:6170-6175) | PARTIAL | drive_once engine.rs:6175-6177 passes it through as EngineError::Driver(fenced); the lift to LeaseLost happens in heartbeaten via is_fenced (engine.rs:1954, is_fenced :~6630) | Say: drive_once passes it through as EngineError::Driver(Fenced) (engine.rs:6175-6177) and heartbeaten lifts it to LeaseLost (engine.rs:1954). |
| D6-6 | judge's record-then-drive order at engine.rs:5151-5153 | TRUE | engine.rs:5151-5152 if call>0 { recorder.record_follow_up(text, self.now()) } then drive_once at :5160 (note: judge starts a new session per call, not send_follow_up) | — |
| D6-7 | session.send_follow_up(text) at driver.rs:435-439 | TRUE | driver.rs:439 fn send_follow_up(&mut self, text: String) -> DriverFuture<()>; doc :435-438 | — |
| D6-7-transports | All transports accept a follow-up after a turn's done | TRUE | acp/mod.rs:534, cli/mod.rs:621, fake.rs:541: Closed if ended, Transport if empty or turn_open; turn_open reset on Done (acp :628, cli :740, fake :348). Fake pops the next scripted turn, else ended+Closed | — |
| D6-8a | settles_stale post-step at relay.rs:206-221 | TRUE | relay.rs:206-219 opened/settles_stale/settle_permissions(Stale) warn; settles_stale fn :225-230 excludes Cancelled and Fenced | — |
| D6-8b | TRANSIENT_READS at relay.rs:33 is the parked poll's bounded transient retry | TRUE | relay.rs:34 const TRANSIENT_READS: u32 = 30 (private; reachable from drive's module); used in park with RELAY_POLL sleeps. The close retry needs its own sleep interval (relay.poll) or it spins 30 calls instantly | Optionally state the close retries at relay.poll intervals (30 x 1 s worst case after session end). |
| D6-8c | close reason: cancelled-session sentence for Err(Cancelled) | PARTIAL | drive cannot tell a deadline cut from a run cancel: both arrive as Signal::Cancel on the step-local control and end Err(Cancelled); `cut` is only known in drive_once (engine.rs:6150-6162) | Note that a deadline cut also closes with the cancelled-session sentence (or pass a cut-aware reason from drive_once). |
| D6-NoRelay | NoRelay implements RelayStore with uninhabited stubs; RelayStore impls are MemStore, PgStore, Writer, NoRelay, FailingReads (:467), FailingSettles (:510) | TRUE | relay.rs:140 pub enum NoRelay {}; :142 impl with `match *self {}`; git grep 'RelayStore for': core/store/worker.rs:528 MemStore, htui-store/worker.rs:57 PgStore, :394 Writer, tests/relay.rs:467 FailingReads (struct :445), :510 FailingSettles (struct :508) - exactly 6, no blanket impls | — |
| D7 | cut = Err(Cancelled) with cut, engine.rs:6157-6167 | TRUE | engine.rs:6162-6170 Err(Cancelled) if cut && !control.signal().is_cancel() -> Ok(Driven{Cancelled, cut:true}) | — |
| OQ-3/D7 | drive_once wraps the boxed drive in drive_with_deadline (engine.rs:6140-6151); budget_micros at :6108 | TRUE | engine.rs:6149 Box::pin(drive(..)), :6152 Box::pin(drive_with_deadline(..)); :6108 budget_micros: settings.per_token_cap_run | — |
| D-stack | drive_once boxes drive (engine.rs:6147), pump boxes it (record.rs:2047-2053); every_case_name_dispatches at conformance.rs:7794; mod-42.md:77-79 | TRUE | engine.rs:6149; record.rs:2047-2053; htui-orch/src/conformance.rs:7794; docs/decisions/mod/mod-42.md:77-79 'Box large futures on the walk path' | — |
| T2-fake | FakeSession supports a two-turn script | TRUE | fake.rs:248 turns: VecDeque<Turn>, :283 from script.turns; send_follow_up pops next turn (:541-565); tests/relay.rs:304 fake(script, step) helper | — |
| T2-counting | follow_ups:false and pump -> no store call at all (a counting double) | PARTIAL | pump hard-codes None::<&Relay<NoRelay>> (record.rs:2047-2052); a counting double cannot be passed to pump; no counting RelayStore double exists yet in tests/relay.rs | Counting double applies to drive with follow_ups:false; for pump the no-store-call property holds by construction (NoRelay uninhabited) - test via existing drive_without_a_relay_is_pump (tests/relay.rs:1490) instead. |

### Engine / runtime

| Anchor | Claim | Verdict | Evidence | Amendment |
|---|---|---|---|---|
| OQ-2a | Step stays running through verify, capture, settle; finish_step only after them (engine.rs:3586-3660) | TRUE | engine.rs:3586 session(), :3608 verify, :3621 capture, :3624 record_commits, :3640 finish_step, ~:3663 gate::apply | — |
| OQ-2b | Recorder's last write is run_step.usage (record.rs:1301-1312) | TRUE | record.rs:1301 sync_step -> set_step_usage; finish() :1131-1134 flush then sync_step last | — |
| OQ-2c | trg_run_step_updated_at in 0001_init.sql:574-579 bumps run_step.updated_at on any column update | TRUE | 0001_init.sql:574-580 DO-loop creates trg_%s_updated_at BEFORE UPDATE FOR EACH ROW (no column list). Probe: scratch DB with 0001 applied lists trg_run_step_updated_at; set_updated_at bumps on update of an unrelated column | — |
| OQ-2d | MemStore would not mirror the trigger's updated_at bump (backend divergence) | FALSE | MemStore stamps updated_at by hand on every run_step write as a trigger stand-in: mem.rs:1766 (set_step_usage), :1796 (set_step_prompt); doc comments mem.rs:1902-1905, :2081 say this mirrors the BEFORE UPDATE trigger | Drop the 'MemStore would not mirror' argument. If the table is still preferred, justify it differently: a run_step column write bumps updated_at, which cache/sync cursors read as a change, and it has to be fenced and stamped on both backends. A separate table avoids that churn and gives a natural closed_at. |
| OQ-3a | deadline_seconds defaults to 7200 via project step_deadline_seconds then app setting (graph.rs:29-30, 740-745) | TRUE | graph.rs:30 DEFAULT_DEADLINE_SECONDS=7200; :740-745 settings.step_deadline_seconds.or_else(app_u32(app,"step_deadline_seconds")).unwrap_or(DEFAULT) | — |
| OQ-3b | drive_once wraps the whole boxed drive future in drive_with_deadline (engine.rs:6140-6151); Box::pin(drive(...)) at :6147 | TRUE | engine.rs:6147 let driving = Box::pin(drive(...)); :6148-6152 Box::pin(drive_with_deadline(driving, ...)) when deadline is Some. Judge calls pass deadline None (engine.rs:5174-5176), so they are untimed | — |
| OQ-3c | SessionSpec.budget_micros = settings.per_token_cap_run (engine.rs:6108) | TRUE | engine.rs:6108 | — |
| OQ-3d | Recorder's run cap at record.rs:616, :630; enforce_breach :2086-2106 | TRUE | record.rs:616 run_cap(), :630 cap_breach(), :2086 enforce_breach (body to ~:2106) | — |
| D7 | Deadline cut arm Err(Cancelled) with cut at engine.rs:6157-6167 | TRUE | engine.rs:6159-6168 `Err(DriverError::Cancelled) if cut && !control...is_cancel()` -> Driven{cut:true} | — |
| OQ-5 | A dead walk's pending follow-up is refused when a process on that box next holds the run, at D9's three lease-take sites | PARTIAL | renew_lease (engine.rs:2049-2064) stales only when `taken && dead_walks.remove(run)`, i.e. only for this process's own DeadWalks. A command take_lease (:2012) on a run walked by a crashed process with another owner does not reach it. abandoned (:2110-2127) is a refresh_lease, not a take. A crashed walk is reached only through sweep recovery (:2329, :2345-2348) on a running graph run | Reword: 'refused when this process re-takes a run it dropped (renew_lease on a DeadWalks run, abandoned with a live lease) or when the sweep adopts the run (crash recovery)'. A command take by a fresh process on a crashed run's lapsed lease closes nothing; the sweep or a cancel does. |
| I-6 | Follow-ups applied only before recorder.finish, verify, capture, finish_step and gate::apply (engine.rs:3586-3660) | PARTIAL | verify, capture, finish_step and gate are at engine.rs:3608-3663, but recorder.finish is not in that range. It is inside session() at engine.rs:5941 (and :5937 on error), and in candidate_live at :4269/:4273 | Cite recorder.finish separately: engine.rs:5941 (main session) and :4273 (candidate). |
| I-6b | ANA-2 inv. 5 at docs/ANA-2.md:124-127 | TRUE | ANA-2.md:124-127 'A gate stops only at a durable boundary' | — |
| I-8 | pump has 26 call sites and passes no relay (record.rs:2034-2054) | TRUE | 26 non-comment pump( calls (21 in htui-agent/src/conformance.rs, 3 in tests/recorder.rs, 1 relay.rs, 1 extensibility.rs); record.rs:2034 pump, :2047-2052 Box::pin(drive(.., None::<&Relay>, &mut Control::never())) | — |
| I-9 | poll_once spawns cancel_run for every row without looking at kind (runtime.rs:2713-2731); cancels_first likewise (:2060-2100) | TRUE | htui-worker/src/runtime.rs:2713-2731 loop spawns cancel_run(row.run_id) with no kind check; cancels_first :2048, read :2060, loop to ~:2105 also ignores kind. pending_commands SQL (htui-store/src/pg/relay.rs:434-465) and MemStore (mem.rs:6915) do not filter kind either. CHECK kind IN ('cancel') at 0011_permission_relay.sql:57 | — |
| D8a | The Relay literal is at engine.rs:6116-6126 (T3: flag at :6116) | TRUE | engine.rs:6116 `let relay = Relay {` through :6126 `};` | — |
| D8b | drive_once has exactly three callers: main session (5899-5943), candidate_live (:4252), judge_calls (:5138, drive_once at :5160) | TRUE | Only three .drive_once( calls: engine.rs:5921 in session() (5899-5944), :4253 in candidate_live (fn :4167), :5161 in judge_calls (fn :5138). drive_once defined at :6005 | — |
| D8c | Judge step's fanout_index is -1 (model/run.rs:236-238), and judge_calls drives the judge step | TRUE | htui-core/src/model/run.rs:236-238 doc says '-1 is the judge step', field fanout_index:i32. judge_calls passes `judge: &RunStep` to drive_once (engine.rs:5163) | — |
| RelaySessionId | drive_once mints the RelaySessionId at engine.rs:6121, a new one on every call (each judge call or retry gets its own) | TRUE | engine.rs:6121 `session: RelaySessionId::new()`, which is Uuid::now_v7() (ids.rs:31-32). drive_once has no internal retry loop. Each judge call (2 per judge step, engine.rs:5150 loop) and each retry attempt (new run_step, new drive_once) gets a fresh id. relay.rs:110 doc says 'minted per drive_once' | — |
| D9a | stale_dropped_requests at engine.rs:2142; call sites renew_lease (:2061), abandoned after a successful refresh_lease (:2122), sweep recovery (:2347) | TRUE | fn at :2142. :2061 and :2122 call stale_dropped_requests_of (:2169), which reads the run and then calls it; :2347 calls it directly. Note: the sweep calls renew_lease (:2329) first, which can itself stale a DeadWalks run, so on that path the close runs twice and must be idempotent | Say 2061/2122 reach it through stale_dropped_requests_of. Require close_dropped_follow_ups to be idempotent, because the sweep can run it twice (:2329 via renew_lease, then :2347). |
| D9b | A chat run never reaches it (:2169-2176) | TRUE | engine.rs:2169-2177: item_id None -> Ok(_) => {} 'A chat run never parks'; the sweep also guards with `if let Some(item) = run.item_id` (:2346) | — |
| D10 | Queued-run cancel path writes no row (runtime.rs:2585-2603) | TRUE | runtime.rs:2582-2604: queued/terminal run with existing None goes through on_run_unless_claimed and returns without writing a row ('B-20: a queued or terminal run takes today's path and writes no row') | — |
| D11 | cancel_run :2540, poll_once :2698, cancels_first ~:2048 read pending_commands | TRUE | runtime.rs:2540 cancel_run, :2698 poll_once (reads at :2713), :2048 cancels_first (reads at :2060) | — |
| T3a | a_dispatch_future_is_send at engine.rs:14375 | TRUE | engine.rs:14375 | — |
| T3b | every_case_name_dispatches at conformance.rs:7794 | TRUE | crates/htui-orch/src/conformance.rs:7794 | — |
| D6/T3c | drive_once already lifts Err(Store(Fenced)) to LeaseLost (engine.rs:6170-6175) | PARTIAL | engine.rs:6173-6177 only lifts DriverError::Store(Fenced) out of the session result as EngineError::Driver(fenced). The LeaseLost conversion happens in heartbeaten (engine.rs:1955-1964) through is_fenced (:6630-6637), as the comment at :6172 says | Reword: 'drive_once surfaces it as EngineError::Driver(Store(Fenced)) (engine.rs:6173-6177), which heartbeaten turns into LeaseLost (engine.rs:1955-1964, is_fenced :6630)'. |

### TUI

| Anchor | Claim | Verdict | Evidence | Amendment |
|---|---|---|---|---|
| OQ-1 f filter | BacklogTab::on_key handles `f` (opens filter form) at backlog/mod.rs:666-670 | TRUE | backlog/mod.rs:666-667 comment, :668-670 `KeyCode::Char('f') => self.form = Some(FilterForm::open(..))` | — |
| OQ-1 routing | Unmatched keys reach self.detail.on_key at :687; pane sees every key first only while captures_input() (:645-647) | TRUE | mod.rs:645-647 captures guard; :648-653 CONTROL/ALT also go to detail; :690 `_ => return self.detail.on_key` (3 lines drift) | — |
| OQ-1 match range | Backlog match spans backlog/mod.rs:655-686 | TRUE | match key.code at :654, ends :691 | — |
| OQ-1 test pin | f_and_shift_f_are_on_the_backlog_help_line at tests/backlog.rs:1808 | TRUE | tests/backlog.rs:1808; help rows bound at app/mod.rs:144-155 (Tab(BacklogTab) scope) | — |
| OQ-1 i unbound | `i` is bound nowhere: Backlog match, Runs action() letters (runs.rs:645-763), global keymap.rs:206-250, app/mod.rs:82-109 | TRUE | No Char('i') in backlog/mod.rs, detail/mod.rs, runs.rs, runs/execution_graph.rs, keymap.rs, app/*.rs; Backlog-scope rows app/mod.rs:129-165 bind Enter,m,f,F,N,e only; overlays are modal. `i` exists only tab-locally elsewhere (chat/mod.rs:498 enters the composer - same 'type text' meaning; settings hierarchy.rs:1267, agents.rs:2428), unreachable from Backlog. Note: the Runs dispatch letter set is in on_key at runs.rs:1468-1470 (`key @ ('a'\|'x'\|...)`), not only action(). | Add to D14/T5: `i` must also be added to RunsTab::on_key's letter pattern (runs.rs:1468-1470) so the end_gesture-on-capture path runs; and handle it before action()'s `self.actions` NOT_LOADED early return (runs.rs:649-652) and the verdict lookup (:705-708), since follow-up has no RunActions verdict. Optionally cite chat/mod.rs:498 (`i` opens the chat composer) as precedent for the letter. |
| OQ-6 | The relay view shows queued text and resolution, never the follow-up text | TRUE | Design claim; consistent with RelayView today (permissions, cancels only, runs.rs:5236-5241 literal) and D12 display sentences | — |
| D12 PRD verbatim | AlreadyQueued, SessionEnded, pending display sentences are PRD Q4/Q5/Q7 verbatim; NotRunning points at p per Q2 | TRUE | prd.md:88-90 (Q2 p promote), :96 Q4, :98-99 Q5, :123-126 Q7 'queued — sent when the current turn ends' | — |
| D13 AnswerPermission arm | try_serve AnswerPermission arm at store_worker.rs:1988-2015 refuses offline with DATABASE_UNREACHABLE, takes user and box, Refused(why) -> Failed{request.name(), why.to_string()} | TRUE | store_worker.rs:1988-2015: writer().ok_or_else(Unreachable(DATABASE_UNREACHABLE)), this_user, box_info, AnswerOutcome::Refused -> Failed | — |
| D13 variant | StoreRequest::AnswerPermission variant at :844-849 | TRUE | store_worker.rs:842-849 | — |
| D13 name arm | name arm goes after :1123 | TRUE | store_worker.rs:1123 `Self::AnswerPermission { .. } => "answer_permission"` | — |
| D13 reply | StoreReply::PermissionAnswered at :1392-1397 | TRUE | store_worker.rs:1391-1397 | — |
| D13 run_worker | run_worker.rs:126-136 routes only Orch, RunStream and RunActions | TRUE | run_worker.rs:126-135 match; other => Failed 'not an orchestrator request' | — |
| D13 Debug | StoreRequest derives Debug so FollowUpText needs a redacting Debug | TRUE | store_worker.rs:122-130 doc rule + #[derive(Debug, Clone)]; FollowUpText must also be Clone | — |
| D14 key data | RunStepSummary has status and fanout_index (model/run.rs:751-789) and run kind is on RunSummary (:835-865); pane has these per step | TRUE | run.rs:749-801 RunStepSummary (fanout_index :758, status :766); RunSummary :835-865 kind :843; RunKind {Graph, Chat} run.rs:12-20; judge = fanout_index -1 (traits.rs:1387). Pane: entry_step() runs.rs:377-384 returns (RunId,&RunStepSummary); kind via entry_run() runs.rs:371-374. Item runs are graph runs in practice (chat runs have item_id null) | — |
| D14 Mode::RejectNote | Mode::RejectNote at runs.rs:277-285 | TRUE | runs.rs:276-284 | — |
| D14 opened | RejectNote opened at runs.rs:713-719 | TRUE | runs.rs:715-721 `'x' if allowed(&verdicts.reject, ctx)` sets Mode::RejectNote | — |
| D14 reject_key | reject_key at runs.rs:838-863 | TRUE | runs.rs:836-863; dispatched from modal_key :581 | Also note modal_key's exhaustive Mode match (runs.rs:579-623) gains the FollowUp arm. |
| D14 captures/on_paste | captures_input at :1484, on_paste at :1490-1498 | TRUE | runs.rs:1484-1486 (!Browse), :1490-1498 RejectNote\|CloseOut Typed field.on_paste | — |
| D14 footer | Footer with title, field.line and hint at :1790-1798 | TRUE | runs.rs:1794-1798 RejectNote footer 'reject with a note:' / field.line(width,true,theme) / 'Enter reject · Esc cancel' | — |
| D14 NOTE_NEEDED | NOTE_NEEDED at :94 | TRUE | runs.rs:94 | — |
| D14 Enter text | Enter -> FollowUpText::new(field.text()) | PARTIAL | TextField::text() returns Option<&str> (ui/text_field.rs:240, None when masked); reject_key uses field.text().unwrap_or_default() | Write FollowUpText::new(field.text().unwrap_or_default()) (or make new take Option), matching reject_key runs.rs:847. |
| D14 replies | PermissionAnswered re-reads at :1640; Failed answer re-reads at :1641-1645 | TRUE | runs.rs:1640 re_read; :1641-1645 Failed if request==ANSWER_PERMISSION (const :113) | Add a FOLLOW_UP const beside ANSWER_PERMISSION (runs.rs:113). |
| D14 relay field | relay cache field at :201 | TRUE | runs.rs:200-201 `relay: Option<RelayView>` | — |
| D14 draw sites | list_lines :1751, flow_head ~:1734, permission_lines :1274, lines fitted with cells::fit(…, PANE) | PARTIAL | runs.rs:1751, :1734, :1274 correct; but permission_lines does blank(INDENT) + cells::fit(.., PANE - INDENT) for the summary line and clip_spans+pad to PANE for the strip (:1276-1289), not a bare cells::fit(…, PANE) | Say: each line exactly PANE wide, mirroring permission_lines (blank(INDENT) + cells::fit(text, PANE - INDENT)). |
| D14 PANE | PANE width 43 | TRUE | runs.rs:131 `const PANE: usize = 43` | — |
| D14 module doc | Module doc key table at :16-30 | TRUE | runs.rs:15-29 | — |
| T5 captures_input test | captures_input_follows_the_mode (:3776) gains the row | PARTIAL | runs.rs:3776 exists, but it uses driven() (:2943) over feat_1_runs whose four steps are all done (fixtures.rs:1506-1516); D14's client-side Running guard would refuse `i` there, so the opener loop cannot simply gain an `i` row | The test (or a sibling) must first set the cursor step's status to Running (and fanout_index>=0) in the pane's runs before pressing `i`. |
| T5 every_step_takes_two_lines | every_step_takes_two_lines at :2187 | TRUE | runs.rs:2187 | — |
| T5 43-wide test | a_step_with_a_pending_request_takes_two_more_lines_each_forty_three_wide at :5448 | TRUE | runs.rs:5448 | — |
| T5 store_worker offline | relay_reads_are_empty_offline_and_answers_are_refused ~:5150-5175 | TRUE | store_worker.rs:5148-5182; AnswerPermission offline -> Unreachable(DATABASE_UNREACHABLE) at :5170-5178 | — |
| T5 connection.rs | tests/connection.rs try_serve_refuses_every_writer_by_name (:291) gains follow_up | FALSE | connection.rs:291 exists but iterates only connection_requests() (:92-99: ConnectionInfo, SetDsn, ClearDsn, RebuildCache) zipped with connection::REQUEST_NAMES [4] (src/connection.rs:259), asserting NO_WORKER; it is the connection section's own pin, not a list of every store writer. AnswerPermission is not in it; its app-level offline pin is tests/backlog.rs:2020-2031 (offline_the_runs_pane_asks_for_no_error) | Drop tests/connection.rs from T5 (tests + Files + Files-to-Change row). Instead extend tests/backlog.rs offline_the_runs_pane_asks_for_no_error (~:1985-2032) with a StoreRequest::FollowUp whose status starts with 'follow_up: ' and ends with DATABASE_UNREACHABLE, beside the store_worker.rs:5148 unit case. |
| T5 snapshot pattern | runs_pane_shows_a_pending_permission at tests/backlog.rs:1913 uses Harness::over MemStore, row seeded through the store | TRUE | backlog.rs:1913; on_feat_3_runs -> polled() -> Harness::over(store) (:1433-1434); relayed_store claims RUN_2 and opens a permission (:1864-1900) | Note: STEP_R2_PRD is seeded Pending (fixtures.rs:1542) and relayed_store only claims the run; the follow-up snapshots must also move the step to running (and open its window) before seeding the row. |
| D15/T5 snapshot count | snapshots 147 -> 149 | TRUE | crates/htui/tests/snapshots has 147 .snap; crate-wide 149 (+src/snapshots, ui/overlay/snapshots), workspace 191 | Optionally scope it: '147 in crates/htui/tests/snapshots'. |
| T6 htui-worker key table | docs/htui-worker.md key table at :120-131 | TRUE | header :120, rows to :131 (`C`, `T`, `o`) | — |
| T6 htui-worker cancel section | 'Cancelling a run the worker walks' at :243-268; new section before :270 | TRUE | ## Cancelling at :243, last line :268; ## Upgrading at :270. 'Permission requests on worker steps' already at :179 | — |
| T6 README | README.md Runs key table at :204-221 | TRUE | **Runs.** :204, table :207-222 (`+`/`-` row at :222) | — |
| Help hints | (implicit) no other Runs key help/hint text needs updating | TRUE | No runs-key help strings in crates/htui/src; Runs keys have no keymap rows (only Backlog-scope Enter/m/f/F/N/e at app/mod.rs:129-165) | — |

### Probes and task independence

| Anchor | Claim | Verdict | Evidence | Amendment |
|---|---|---|---|---|
| D1 index / D5 ON CONFLICT | After replacing uq_run_command_pending with two partial unique indexes, INSERT ... ON CONFLICT (run_id) WHERE status = 'pending' AND kind = 'cancel' DO NOTHING infers uq_run_command_pending_cancel | TRUE | Probe on scratch DB mod70_probe (PG 16.15). I mirrored 0011:47-62 and applied D1. A second pending cancel on the same run gave INSERT 0 0. A cancel on a run with only resolved cancels gave INSERT 0 1. Reversing the predicate order (kind first) also inferred the index (INSERT 0 0). The old target ON CONFLICT (run_id, kind) WHERE status='pending' gave ERROR: there is no unique or exclusion constraint matching the ON CONFLICT specification, so changing pg/relay.rs:376 is mandatory, as D5 says. ON CONFLICT (run_step_id) WHERE status='pending' AND kind='follow_up' gave INSERT 0 1, then 0 0. A pending cancel and a pending follow-up coexist on the same run. | — |
| D1 0011 cites | chk_run_command_kind at 0011:58; uq_run_command_pending at 0011:61-62; run_command table at 0011:47-62 | TRUE | 0011_permission_relay.sql:57 CHECK kind IN ('cancel') (within tolerance); :61-62 the unique index; :47 CREATE TABLE run_command. 0015 is the last migration on disk. | — |
| D1 CHECK | chk_run_command_follow_up as written: existing cancel rows satisfy it; a pending follow_up with text passes; a resolved follow_up with text fails; a resolved follow_up without text passes | TRUE | The ADD CONSTRAINT (validated, not NOT VALID) succeeded over existing cancel rows: pending/no resolution, applied/no resolution, refused/resolution, applied/resolution. Pending follow_up with text: INSERT 0 1. UPDATE to applied keeping text: ERROR violates chk_run_command_follow_up. UPDATE to refused with text=NULL: UPDATE 1. Also rejected by it: pending follow_up with NULL text, follow_up with NULL run_step_id, cancel with text. A resolve_command-style UPDATE ... text=NULL on cancels passed (UPDATE 2). | — |
| D1 expression index | Two plain partial indexes rather than an expression index; no migration uses one today | FALSE | crates/htui-store/migrations/0001_init.sql:75 `CREATE INDEX idx_box_tags ON box USING GIN ((probed_tags \|\| declared_tags));` is an expression index. | Drop 'no migration uses one today'. Justify the two partial indexes on their own: each ON CONFLICT target infers a plain column list (probed). |
| D3 CAS (enqueue first) | With FOR SHARE on the window row, an enqueue that commits first is refused by the close (D4: UPDATE closed_at, then refuse pending rows, in one transaction) | TRUE | Probe A: two background psql sessions with \timing. The enqueue (INSERT...SELECT FROM follow_up_window w ... WHERE closed_at IS NULL FOR SHARE OF w ON CONFLICT ... DO NOTHING) inserted 1 row and held for 2 s. The close's UPDATE follow_up_window SET closed_at blocked 1503 ms, then UPDATE 1. Its next statement, UPDATE run_command SET status='refused' ... WHERE status='pending', gave UPDATE 1. Final row: refused / text=NULL. This only works because the refusal is a separate statement with a fresh READ COMMITTED snapshot. Probe D: the same close written as ONE statement (WITH c AS (UPDATE window ... RETURNING) UPDATE run_command ... FROM c) blocked 1502 ms, then returned UPDATE 0 and left the row pending with its text. | D4 close_follow_ups and D9 close_dropped_follow_ups: the window UPDATE and the refusal UPDATE must be two separate statements in READ COMMITTED, never one CTE statement (a CTE close leaves a racing enqueue's row pending). Add this to T1's Action and pin it with T1's race test. |
| D3 CAS (close first, EvalPlanQual) | An enqueue that waits behind the close reads the window closed | TRUE | Probe B: the close's window UPDATE ran and the transaction stayed open 2 s. The enqueue's INSERT...SELECT ... FOR SHARE OF w blocked 1502 ms, then returned INSERT 0 0: EPQ re-checked closed_at IS NULL on the committed version. The close's refusal UPDATE found 0 rows; no row was left pending. The enqueue then needs D3's re-read to classify SessionEnded. Control probe E (no FOR SHARE): the enqueue did not block (INSERT 0 1 in 1 ms) against the uncommitted close. It was only caught because the close's refusal happened to run after the enqueue committed, so the lock is what makes this deterministic. | — |
| D3 FOR SHARE scope | FOR SHARE lock on the step's window row (implicitly the only row locked) | PARTIAL | Probe: the enqueue joined to run and run_step with an unqualified FOR SHARE and held 2 s. A concurrent lease heartbeat (UPDATE run SET lease_expires_at) blocked 1502 ms. With FOR SHARE OF w the heartbeat finished in 0.8 ms. An unqualified FOR SHARE locks every FROM relation. | In D3/T1, write `FOR SHARE OF <window alias>` so the enqueue locks only the window row and never stalls lease heartbeats or other run/run_step writers. The window must be an inner join: FOR SHARE cannot target the nullable side of an outer join. |
| T1 Files / hr-sandbox cite | Scratch-DB prepare per docs/hr-sandbox.md:194-210 (CREATE DATABASE htui_sqlx, cargo sqlx migrate run --source migrations, cargo sqlx prepare -- --all-targets --all-features) | TRUE | docs/hr-sandbox.md:194 heading 'Changing SQL queries in a run', :200 CREATE DATABASE htui_sqlx, :204 prepare -- --all-targets --all-features, :205 --check. | — |
| Task independence: prepare compiles whole workspace | cargo sqlx prepare -- --all-targets --all-features (from crates/htui-store) compiles the whole workspace, so T1's prepare must run on a tree where T2's and T5's in-progress edits compile | FALSE | Probe in a /tmp copy of the tree, with cargo-sqlx 0.9.0 run directly and CARGO pointed at a logging fake cargo. It invoked only `metadata`, `locate-project` and `check --all-targets --all-features` (no --workspace/-p) from crates/htui-store, so cargo selects only the htui-store package. A real run in the copy failed with errors only in `htui-store (lib)`. `cargo tree -p htui-store -e normal,dev,build` contains only htui-core and htui-store. T2 (htui-agent) and T5 (crates/htui) edits cannot break T1's prepare. Only htui-core edits could, and T0 has landed by then. | Replace the bullet: 'From crates/htui-store, prepare runs cargo check --all-targets --all-features on htui-store and its dependency htui-core only; T2/T5 edits do not affect it. The real coupling is the shared tree/target (and htui-core, settled by T0).' Soften R-4 ('the prepare compiles others' half-done edits') the same way. |
| Task independence: T1∩T2, T1∩T5, T2∩T5 | The three parallel tasks touch disjoint files | TRUE | T1 = {migrations/0016_follow_up.sql, src/pg/relay.rs, .sqlx/*, tests/pg_criteria.rs, tests/migrations.rs, tests/connect.rs} (all htui-store). T2 = {htui-agent src/record/relay.rs, tests/relay.rs}. T5 = {crates/htui/src/store_worker.rs, src/ui/tabs/backlog/detail/runs.rs, tests/backlog.rs, tests/connection.rs, tests/snapshots/* new, [+ src/ui/tabs/backlog/mod.rs with f]}. All pairwise intersections are empty. Other pairs: T3∩{T1,T2,T5,T4,T6}=∅; T4 {worker_pg.rs, runs_pg.rs} ∩ T5 = ∅ (same tests dir, different files); T6 disjoint. | — |
| Task independence: T0 overlaps | T0 writes pg/relay.rs and pg_conformance.rs (T1's), record/relay.rs and tests/relay.rs (T2's), engine.rs (T3's) and runs.rs (T5's) | PARTIAL | The intersections are T0∩T1={pg/relay.rs}, T0∩T2={record/relay.rs, tests/relay.rs}, T0∩T3={engine.rs}, T0∩T5={runs.rs}. pg_conformance.rs is in T0's Files list only, not in T1's, so it is not 'T1's' file. There is a behavioural coupling, though: T0 adds 9 conformance cases that pg_conformance runs against PgStore placeholders returning Backend errors. Those cases fail until T1, and T0's Validate omits `cargo test -p htui-store`, so T1 inherits the red Pg cases. No unstated file intersections were found. | Say 'pg_conformance.rs (T0's file; its 9 new Pg cases stay red until T1 and are T1's acceptance)'. Optionally add `pg_conformance` passing to T1's Validate wording. |
| T5 needs T0's MemStore | T5's snapshots seed rows through T0's MemStore (Harness::over) | TRUE | crates/htui/src/testkit.rs:116 `pub fn over(store: MemStore) -> Self`. Follow-up rows/windows exist in MemStore only after T0. | — |
| T5 store_worker needs no T1 | T5's store_worker.rs change (D13) does not require T1's Postgres bodies | TRUE | store_worker.rs:1988-2010: AnswerPermission dispatches via `WriteStore::answer_permission(&writer, …)` on the `Writer` trait object, refusing offline first with DATABASE_UNREACHABLE (:1993). FollowUp mirrors this, so it compiles against T0's Writer forwarding and PgStore placeholders. The offline test (relay_reads_are_empty_offline_and_answers_are_refused, :5148-5175) never reaches Pg. connection.rs:291 try_serve_refuses_every_writer_by_name confirmed. runs.rs:5238 RelayView literal confirmed (5235-5241). | — |
