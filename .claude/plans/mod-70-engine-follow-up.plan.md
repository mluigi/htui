# Plan: MOD-70 — Follow-up command rows for engine steps

**Status: DRAFT, awaiting the step 3.5 fact-check and the maintainer's CONFIRM.**

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
  `updated_at` through `trg_run_step_updated_at` (`crates/htui-store/migrations/0001_init.sql:574-579`),
  which MemStore would not mirror, a backend divergence. The table holds one row per step whose
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
  open and the row `pending`. It is refused when a process on that box next holds the run (D9's
  three lease-take sites) or when the run is cancelled (D11). New follow-ups are refused once the
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
  `recorder.finish`, verify, capture, `finish_step` and `gate::apply` (`engine.rs:3586-3660`).
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
    (PRD Q4). Two plain partial indexes rather than an expression index; no migration uses one
    today.
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
  under a `FOR SHARE` lock on the step's window row, every guard below holds. On zero rows a re-read
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

  The `FOR SHARE` on the window row against D4's `UPDATE … SET closed_at` is what makes Q5 a
  compare-and-set: an enqueue that commits first is refused by the close, and one that waits behind
  the close reads it closed.
- **D4 · The executor's methods: `RelayStore` grows from four to nine.** Each is also a same-named
  `WriteStore` method, because the forwarding convention needs a same-named target (MOD-42 D2;
  `RelayStore` `crates/htui-core/src/store/worker.rs:74-97`). Every one takes the executor's `owner`
  and is fenced on `run.lease_owner = owner AND lease_expires_at > clock_timestamp()`, as
  `apply_permission` is:
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
  - `close_follow_ups(step, session, reason) -> u64`: in one transaction, sets `closed_at` where the
    window is this session's, and only then refuses the step's pending rows with `reason`.
  - `close_dropped_follow_ups(run, owner, reason) -> u64`: closes every open window of the run and
    refuses every pending follow-up of the run, only while `owner` holds the live lease (D9).

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
     window refused it) → step 3; `Fenced` → `Err(Store(Fenced))`, which `drive_once` already lifts
     to `LeaseLost` (`engine.rs:6170-6175`).
  6. **Record.** `recorder.record_follow_up(&text, (relay.now)())`: `turn + 1`, scrubbed (PRD Q1,
     `R-HIS-1`). This mirrors the judge's record-then-drive order (`engine.rs:5151-5153`).
  7. **Send.** `session.send_follow_up(text)` (`driver.rs:435-439`); an error ends the session as a
     failed turn would. Then back to step 2.
  8. **Close** on every exit except a fenced one (MOD-40 D1: a fenced writer writes nothing more),
     beside the existing `settles_stale` post-step (`relay.rs:206-221`):
     `close_follow_ups(step, session, reason)`, with `reason` = `SessionEnded` for `Ok`, the
     cancelled-session sentence for `Err(Cancelled)`, and `SessionEnded` otherwise. It is
     best-effort with the parked poll's bounded transient retry (`TRANSIENT_READS`, `relay.rs:33`);
     a final failure is a `warn` (R-3).

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
  - `renew_lease` on a dead walk (`:2061`);
  - `abandoned` after a successful `refresh_lease` (`:2122`);
  - sweep recovery under the fence (`:2347`).

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
  - **Key** (OQ-1: `i`): on a step whose `RunStepSummary.status` is `Running`, whose
    `fanout_index >= 0` and whose run's `kind` is `Graph` (`model/run.rs:751-789`, `:835-865`), it
    opens `Mode::FollowUp { run, step, field: TextField::new() }`. Otherwise it puts D12's sentence
    on the status line and sends nothing; the store re-checks every guard.
  - **Mirror** the `x` reject-note flow: `Mode::RejectNote` (`runs.rs:277-285`), opened at
    `:713-719`, `reject_key` (`:838-863`), `captures_input` (`:1484`), `on_paste` (`:1490-1498`), and
    the footer with title, `field.line` and hint (`:1790-1798`). The hint is "Enter send · Esc
    cancel".
  - **Enter** → `FollowUpText::new(field.text())`:
    - `Empty` → status line, stay in the mode (the `NOTE_NEEDED` shape, `:94`);
    - `Residue` → status line, stay in the mode so the text can be edited;
    - `Ok` → send `StoreRequest::FollowUp`, back to Browse.
  - **Replies:** `FollowUpQueued` re-reads the relay view, as `PermissionAnswered` does
    (`:1640`); `Failed { request: "follow_up" }` re-reads as well (`:1641-1645` shape).
  - **Display:** a step with a `FollowUpView` takes one more line (pending, applied) or two
    (refused: the label, then the resolution fitted to 43 columns). The view's cache is the
    existing `relay` field (`:201`); it is drawn in `list_lines` (`:1751`) and `flow_head`
    (`~:1734`) beside `permission_lines` (`:1274`), each line fitted with `cells::fit(…, PANE)`.
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
| `crates/htui/tests/backlog.rs`, `tests/connection.rs`, `tests/snapshots/*` | UPDATE/CREATE | D14 tests and snapshots | T5 |
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
  --all-targets --all-features`; `cargo clippy --workspace -- -D warnings`.

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
  - `follow_ups: false` and `pump` → no store call at all (a counting double);
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
      (`:3776`) gains the row;
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
  - in `tests/connection.rs`: `try_serve_refuses_every_writer_by_name` (`:291`) gains `follow_up`;
  - in `tests/backlog.rs`: snapshots `runs_follow_up_input` and `runs_follow_up_queued`
    (MemStore `Harness::over`, rows seeded through T0's MemStore).

  If OQ-1 keeps `f`, `f_and_shift_f_are_on_the_backlog_help_line` (`tests/backlog.rs:1808`) and the
  Backlog routing change join this task.
- **Files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/backlog/detail/runs.rs`,
  `crates/htui/tests/backlog.rs`, `crates/htui/tests/connection.rs`,
  `crates/htui/tests/snapshots/*` (new files only). With `f`: also
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
    the deadline and run cap (OQ-3), what the user sees, OQ-5, and that `0016` must be migrated
    from a TUI first.
  - `README.md`'s Runs key table (`:204-221`) gains the key row.
- **Files**: `docs/ANA-2.md`, `docs/htui-worker.md`, `README.md`.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; the full gate
  below.

### Task independence

- **The three parallel tasks touch disjoint files.**
  - T1 ∩ T2 = ∅ (`htui-store` vs `htui-agent`).
  - T1 ∩ T5 = ∅ (`htui-store` vs `crates/htui`).
  - T2 ∩ T5 = ∅.
- **T0 is strictly earlier than the tasks whose files it touches.** It writes `pg/relay.rs` and
  `pg_conformance.rs` (T1's), `record/relay.rs` and `tests/relay.rs` (T2's), `engine.rs` (T3's) and
  `runs.rs` (T5's).
- **Hidden couplings:**
  - `.sqlx` is T1's alone, but `cargo sqlx prepare -- --all-targets --all-features` compiles the
    **whole workspace**. T1's prepare must run on a tree where T2's and T5's in-progress edits
    compile, or in its own worktree (memory: parallel fan-out hidden coupling).
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
| R-3 · A failed close leaves a live window on a step that moved on | Low | Bounded transient retry; D9 closes it at the next lease re-take; a cancel refuses the row |
| R-4 · `.sqlx` drift; the prepare compiles others' half-done edits | High | Scratch-DB prepare per `hr-sandbox.md:194-210`; worktree or wave-end prepare; `--check` in T1's validate |
| R-5 · Enqueue and cancel interleave under READ COMMITTED | Low | Walk's cancelled close and D9 refuse the survivor; e2e cancel test |
| R-6 · Plaintext in logs through `Debug` | Medium | `FollowUpText`/`QueuedFollowUp` redacting `Debug`; unit test |
| R-7 · Turn-end delay feels slow (a long turn) | Medium | Accepted (PRD out of scope: mid-turn steering); docs say "sent when the current turn ends" |
| R-8 · Mixed versions: a worker built with `0016` against an unmigrated database | Medium | Headless connect refuses (MOD-40 C5); T6 documents migrating from a TUI |
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

pending the step 3.5 fact-check
