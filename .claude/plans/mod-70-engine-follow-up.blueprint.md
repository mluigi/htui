# Blueprint: MOD-70 — Follow-up command rows for engine steps, T0–T6

**Status**: proposed (2026-10-06, code-architect). Implements
`.claude/plans/mod-70-engine-follow-up.plan.md` (CONFIRMED 2026-10-06, OQ-1…OQ-6 as recommended,
fact-checked) under `.claude/prds/mod-70-engine-follow-up.prd.md`. The plan's D1–D16, D-model, D-stack,
I-1…I-9, the task order `T0 → {T1 ∥ T2 ∥ T5} → T3 → T4 → T6`, the file sets and the "Verified claims"
amendments are binding. Where this blueprint had to choose, the choice is a **B-n**, driven by a
finding **F-n**. Three findings would change a confirmed decision's text; they are **DV-1…DV-3**
below and need the maintainer's nod before T0 starts.

**Verified at**: `e05a86af` (`hr/MOD-70`, sandbox). `git diff 395100f7 e05a86af` touches only the plan
and the PRD, so the plan's line numbers hold. Every anchor below was re-read through Gortex at HEAD
(the "INACTIVE" banner is wrong in this sandbox; symbol search, `read` with `offset`/`limit` and
`search text` with `regexp:true` all answer). The Gortex hook blocks `Read` on indexed files: an
implementer reads with `read(operation:"file", options:{offset,limit})` and edits with Gortex `edit`
or an anchored scripted replace. Paths are relative to `crates/` unless they start with `docs/`,
`.claude/` or name a root file.

**House style (carried from MOD-42)**: `unsafe_code = "forbid"`; `missing_debug_implementations`,
`unused_qualifications` warn and clippy runs `-D warnings` (`clippy::all`, pedantic **off**); every new
`pub` item is documented and `Debug`; **no default body on a store trait**; `max_width = 100`;
edition 2024; every commit compiles; red first, then green, committed incrementally (uncommitted work
dies with the session; never stash on a shared tree). **E0034 hygiene**: no module `use`s
`RecorderStore`, `RelayStore`, `WorkerStore` or `WorkerHost`; bounds name them by path; every
forwarding body and every call on a type implementing two families is UFCS
(`WriteStore::request_follow_up(&writer, …)`, `htui_core::store::RelayStore::open_follow_ups(relay.store, …)`).
`every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs`): a backticked
snake_case name with ≥ 4 underscores in that file's docs must be a fn there or in `mem.rs`; T0's docs
never name a T1 test. **Never the word `zeta`** in an identifier or fixture string. **No plaintext
follow-up text in any `Debug`, `Display`, log line, status line or `RelayView`** (I-5).

**Layout**: Plan deviations · §0 findings · §0a decisions · §1 build order and lanes · §2 shared shapes
(2.1–2.9) · §3 T0 · §4 T1 · §5 T2 · §6 T5 · §7 T3 · §8 T4 · §9 T6 · §10 wave schedule and lane rules ·
§11 pins · §12 gate reference.

---

## Plan deviations (take these to the maintainer before T0)

**Answered: DV-1, DV-2 and DV-3 accepted as recommended (maintainer, 2026-10-06).** Every
`[DV-n declined: …]` fallback below is void; implement the recommended text.

Each is small, each has a fallback that keeps the plan's text, and this blueprint is written for the
recommended answer. Where code differs between the two answers, §2 marks it `[DV-n declined: …]`.

- **DV-1 · D4's executor fence is "owner AND live lease, as `apply_permission` is"; `apply_permission`
  is owner-only.** `pg/relay.rs:227-249` fences `apply_permission` on `r.lease_owner = $2` alone, and
  so does MemStore (`mem.rs:6713-6738`, `lease_owners.get(&run) == Some(&owner)`); `open_permission`
  likewise (`pg/relay.rs:125-177`, MOD-42 B-9, "owner only, no expiry: `StepFence` semantics"). The
  recorder's own writes (`StepFence::Lease(owner)`) are owner-only too. A live-lease fence on
  `settle_follow_up` would turn a lease that lapsed by the store's clock but that nobody took (a store
  blip the heartbeat rides out, D123) into `Fenced` → `LeaseLost` at the next turn end, killing a walk
  every other write of it would let live. **Recommended (B-3)**: `open_follow_ups`,
  `settle_follow_up` and `close_dropped_follow_ups` fence on `run.lease_owner = owner` only, as every
  other executor-side write does; the **enqueue** keeps the live-lease guard (D3 step 7, the answer
  side, as `answer_permission`). Fallback: add `AND r.lease_expires_at > clock_timestamp()` to the
  three (MemStore: `live_owner(run, now) == Some(owner)`).
- **DV-2 · `follow_up_window` needs an `owner` column for D3 step 7 to mean what its sentence says.**
  D1's window has no owner, so step 7 can only test "the run's lease is live" — any owner's. After a
  crash (OQ-5), a fresh process's command `take_lease` (`engine.rs:2012`) makes the lease live again
  while the dead walk's window is still open: an enqueue is then admitted ("queued") into a window no
  process will ever read, and its plaintext waits for a sweep or a cancel. With
  `owner UUID NOT NULL`, step 7 is `r.lease_owner = w.owner AND r.lease_expires_at >
  clock_timestamp()` — exactly the `relay_view`/`answer_permission` predicate for `step_permission`
  (`r.lease_owner = p.owner AND …`), and it refuses with "the process walking the step no longer
  holds the run", which is then true. Additive: one column, written by `open_follow_ups` (which takes
  `owner` already). **Recommended (B-4).** Fallback: no column; step 7 is `r.lease_owner IS NOT NULL
  AND r.lease_expires_at > clock_timestamp()` (MemStore `live_owner(run, now).is_some()`), and OQ-5's
  gap stays as written.
- **DV-3 · D14's line geometry truncates PRD Q7's verbatim sentence.** D14 draws each follow-up line
  "mirroring `permission_lines` (`blank(INDENT)` + `cells::fit(text, PANE - INDENT)`)", i.e. 35
  columns (`INDENT = 8`, `PANE = 43`, `runs.rs:150`, `:131`). "queued — sent when the current turn
  ends" (PRD Q7, D12, verbatim) is **40** columns, so it would render "queued — sent when the current
  tur…". The refused line's resolution "fitted to 43 columns" truncates three of the four resolutions
  D12 defines (52, 62 and ~64 columns). **Recommended (B-7)**: follow-up lines start at column 2
  (`FOLLOW_UP_INDENT = 2`, 41 columns of text, still exactly `PANE` wide); a refused row is the label
  line plus its resolution **wrapped** by `cells::wrap` to at most two lines (every D12 resolution fits
  two, pinned by a unit test). A step with a refused follow-up therefore takes up to three more
  lines, not two. Fallback: keep `INDENT`, shorten the pending label (contradicts PRD Q7).

Minor departures that need no nod (they settle what the plan leaves open): B-8 (`Mode::FollowUp`
without `run`), B-14 (a cancel refuses on `AlreadyPending` too), B-15 (T4's fourth bullet replaced by
deterministic tests in T1/T2).

---

## 0. Findings

| # | Severity | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | Major (T0→T2, T3, T5) | T2/T3/T5 tests assert "the row `applied` with `text = NULL`", "the window closed". | No trait method reads a resolved row's text or a window (`next_follow_up` answers only pending rows of an open window; `relay_view` has no text by OQ-6). W1 lanes may not edit `htui-core` (§10 rule 3). | **B-1**: T0 ships two `#[cfg(feature = "test-support")]` MemStore readers, `follow_up_rows()` and `follow_up_windows()` (§2.8), beside `relay_rows()`/`command_rows()` (`mem.rs:913-920`). |
| **F-2** | Major (T0) | D5: `RunCommand` is **not** changed. | MemStore stores `run_command` as `BTreeMap<RunCommandId, RunCommand>` (`mem.rs:271`), which cannot hold `run_step_id` or `text`. | **B-2**: a side map `follow_up_payloads: BTreeMap<RunCommandId, FollowUpPayload { run_step_id, text: Option<String> }>`, the `lease_owners`-beside-`runs` precedent (`mem.rs:262-264`); the `RunCommand` row (kind `FollowUp`) stays in `run_commands`, so `command_rows()`, `run_worker.rs:3365` `commands_of` and every existing path are untouched. |
| **F-3** | Deviation (T0, T1) | D4: executor methods fenced on owner **and** live lease "as `apply_permission` is". | `apply_permission`/`open_permission` are owner-only (B-9). | **DV-1 / B-3.** |
| **F-4** | Deviation (T0, T1) | D3 step 7 "the run's lease is not live → `ExecutorGone`"; D1's window has no owner. | A command take after a crash makes the lease live under a stranger while the dead window stays open. | **DV-2 / B-4.** |
| **F-5** | Major (T0, T1) | D4 `close_follow_ups(step, session, reason)`: "UPDATE the window's `closed_at` where it is this session's, then refuse the step's pending rows". | `run_command` has no `session` column. Read literally, the second statement refuses the step's pending row **whatever window it was queued under**: a superseded walk (a zombie whose lease moved and whose step a new session re-opened, D4 `open_follow_ups`) would refuse the new session's follow-up at its own exit. | **B-5**: the refusal runs only while the step's window row is this session's (`EXISTS (… w.session = $2)`); a close by a superseded session changes nothing. Conformance case 4 pins it. |
| **F-6** | Major (T1) | D4 `open_follow_ups`: "upserts the window … and refuses … any row still pending on the step from an older window". | Order matters under READ COMMITTED: refuse-then-upsert lets an enqueue that held the **old** window's `FOR SHARE` commit between the two statements and survive as a pending row of an old window. | **B-6**: one transaction: fence (`FOR KEY SHARE OF s FOR SHARE OF r`, `open_permission`'s query, shared by a helper so `.sqlx` reuses its entry), **then upsert** (takes the window row lock; an enqueue that already holds `FOR SHARE` commits first, one that arrives later waits and re-checks), **then refuse** (fresh snapshot sees every row committed before the lock). |
| **F-7** | Deviation (T5) | D14: follow-up lines `blank(INDENT)` + `cells::fit(…, PANE - INDENT)`; refused resolution fitted to 43. | 35 columns < 40 (Q7); three resolutions > 43. | **DV-3 / B-7.** |
| **F-8** | Minor (T5) | D14: `Mode::FollowUp { run, step, field }`. | `StoreRequest::FollowUp` needs only the step; a never-read enum field is `dead_code` under `-D warnings`. | **B-8**: `Mode::FollowUp { step, field }`. |
| **F-9** | Major (T2, T3) | T2: "a pending row at the first `done`"; T3: "a follow-up queued through MemStore by a second client". | `FakeSession::next_event` resolves at its first poll and MemStore never yields, so `drive` plays a scripted turn to its `done` in one poll: a client joined with it never runs before the turn ends (window opened and closed with nothing to see). The MOD-42 tests had the same problem and used a parked request as the rendezvous. | **B-9**: the happy-path tests park turn 0 on a permission request; the client enqueues **then** answers (`parked_row` → `request_follow_up` → `answer`), so the row is pending at the turn's `done`. Edge cases use two small hook doubles (§5.4): `CancelAtDone` (a session wrapper that signals cancel as it hands out a `done`) and `Hooked` (a `RelayStore` over MemStore with per-call hooks and counters). |
| **F-10** | Minor (T2) | D6 step 1: "A transient error is a `warn`, and the session runs without a window." | An error answer may still have committed (lost reply): a window left open with nobody to close it. | **B-10**: on any open error but a fence, `drive` skips every turn-end check and still tries **one** close (no retry) at exit; idempotent and session-scoped, so a no-op when nothing was written. |
| **F-11** | Minor (T2) | D6 steps 3–5 do not say what a failed `next_follow_up` or `settle_follow_up` does. | A follow-up is an add-on: the step's own session result must not turn into a failure because a turn-end read failed. | **B-11**: `next_follow_up` rides out `TRANSIENT_READS` consecutive `Unreachable`/`Backend` failures at `relay.poll`, selected against `control.changed()`; then (or on any other error) a `warn` and the session ends with its `done`; the close refuses what is left. A non-fence error of `settle_follow_up` is a `warn` and the session ends likewise. Only `Fenced` is ever the answer. |
| **F-12** | Minor (T2) | D6 step 3 reads the control once, before `next_follow_up`. | A cancel that lands during the read and the pre-scrub would still claim the row and send a turn the cancel then cuts. | **B-12**: the control is read again right before the claim (`settle_follow_up(Applied)`). |
| **F-13** | Minor (T0, T1) | D5: `relay_view.follow_ups` = "the newest follow-up row of each step". | MemStore's frozen test clocks give equal `issued_at`; `RunCommandId` order then decides, and the caller mints ids. | **B-13**: newest = the pending row if there is one (at most one, D1), else greatest `(issued_at, id)`; output in `(issued_at, id)` order. |
| **F-14** | Minor (T0, T1) | D5: `request_cancel` "also refuses every pending follow-up of the run … in the same statement or transaction as its insert". | Silent on `AlreadyPending`. Under READ COMMITTED a follow-up can slip in between a first cancel's two statements (R-5); a second `c` is the user saying "cancel" again. | **B-14**: both `Inserted` and `AlreadyPending` refuse the run's pending follow-ups (`FOLLOW_UP_RUN_CANCELLED`); `Inserted` does it inside the insert's transaction. Idempotent. |
| **F-15** | Minor (T4) | T4 bullet 4: "a follow-up enqueued after the walk's last check is refused `SessionEnded`" end to end. | Between the last check and the close is a few store calls; no end-to-end harness can enqueue there deterministically. | **B-15**: pinned deterministically at the two layers that own it — T1 `pg_criteria` (lock order, §4.3) and T2 (`Hooked` enqueues inside the last `next_follow_up`, §5.4). T4 asserts the end-to-end invariant instead: once the step is `done`, no follow-up row of it is `pending` and an enqueue is refused. |
| **F-16** | Minor (T1) | T1: "an enqueue racing a close … a `pg_sleep`-free interleave through the lock order". | Two store calls joined on a runtime do not interleave on demand. | **B-16**: the test holds one side open in a raw `sqlx` transaction on `db.pool` (the other side is the real store method) and checks the store call is **blocked** before committing (§4.3). |
| **F-17** | Major (lanes) | Task independence: "T1 ∩ T5 = ∅". | Files are disjoint, builds are not: `cargo test -p htui` compiles `htui-store` (T1 edits `query!` macros against a `.sqlx` it regenerates last) and `htui-agent` (T2 edits `drive`). A primary-tree T5 gate would test a tree that never existed. | **B-17**: T5 runs in a worktree (`hr/MOD-70-t5`) branched from T0's green commit, with its own `target/` (≈ 10 GB; 2.8 TB free at HEAD). T1 and T2 share the primary tree: `htui-store` and `htui-agent` depend only on `htui-core` (`Cargo.toml`s). |
| **F-18** | Minor (T1) | — | `pg/write.rs:444` has `begin_repeatable_read`. Under REPEATABLE READ the close's second statement reads the transaction's **first** snapshot — the single-snapshot bug the fact-check's CTE probe showed. | The two-statement bodies use `store.pool.begin()` (READ COMMITTED, the pool default; no other `SET TRANSACTION` in `pg/`). A comment says why. |
| **F-19** | Minor (T0) | D4 `settle_follow_up`: "`NotPending` and `Fenced` … told apart by a re-read". | Unknown id and a `cancel` row's id are unspecified. | **B-19**: re-read order `NotFound { entity: "run_command" }` → `Constraint` (not a follow-up) → `NotPending` → `Fenced`. |
| **F-20** | Note (T2) | D-stack: "If `turn`'s state machine sits inside the loop's, `turn` is boxed too." | `drive` already holds `turn` inline today; the loop adds no second slot, but `drive`'s new arms (`record_follow_up`, the close's retry) widen its state. | **B-20**: `Box::pin(turn(…))` once per turn (one allocation per turn, not per event); `drive` stays boxed by both callers. Gate: `every_case_name_dispatches` with `--no-fail-fast`, `grep SIGABRT`. |
| **F-21** | Minor (T0) | D12 names `ExecutorGone`'s sentence. | `model/relay.rs` already exports `EXECUTOR_GONE` (MOD-42, a different sentence). | Every MOD-70 sentence is `FOLLOW_UP_*` (§2.1). |
| **F-22** | Minor (T0) | D4 `close_dropped_follow_ups(run, owner, reason) -> u64`. | `StoreError::Fenced` carries a `StepId`; a run-level fence has none. | `Ok(0)` and nothing written when `owner` is not the run's lease owner; the conformance case asserts nothing moved. D9's call site is best-effort anyway. |
| **F-23** | Note (T5) | — | Nothing publishes a frame when a follow-up is applied mid-session; the pane re-reads the relay view on `SessionDone`/`Changed`/`Rested` frames and on its own requests. A pane may show "queued" for the length of the follow-up turn. | Accepted; T6 says "the pane updates when the session ends". |
| **F-24** | Note (T2, T3) | D6 step 1 runs `open_follow_ups` inside `drive`. | `drive` starts after `driver.start` (`engine.rs:6114`), so a fenced open has already spawned the agent and sent the prompt; the session is then dropped unpulled. Same outcome class as today's fenced recorder (`LeaseLost`, nothing more written). | No change; noted for the reviewer. |
| **F-25** | Note (R-3) | R-3: a failed close is "Low"; D9 closes it at the next re-take. | A run that completes normally after a failed close is never re-taken: the row keeps its plaintext until a cancel. | Accepted (R-3); T6's worker doc names it; the close's bounded retry (B-11 shape) makes it a sustained-outage case only. |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-1): MemStore test-support readers `follow_up_rows()` → `Vec<(FollowUpView, bool)>` (`bool` = text still stored) and `follow_up_windows()` → `Vec<FollowUpWindow>`.
- **B-2** (F-2): MemStore keeps follow-up payloads in a side map; `run_commands` still holds `RunCommand`.
- **B-3** (F-3, DV-1): executor-side follow-up writes are fenced on the owner only; the enqueue needs a live lease.
- **B-4** (F-4, DV-2): `follow_up_window.owner UUID NOT NULL`; D3 step 7 compares it with the live lease owner.
- **B-5** (F-5): `close_follow_ups` refuses only while the step's window row is this session's.
- **B-6** (F-6): `open_follow_ups` = fence, upsert, refuse — one transaction, that order.
- **B-7** (F-7, DV-3): follow-up lines from column 2; a refused resolution wraps to ≤ 2 lines.
- **B-8** (F-8): `Mode::FollowUp { step, field }`.
- **B-9** (F-9): parked-request rendezvous for happy paths; `CancelAtDone` and `Hooked` doubles for edges.
- **B-10** (F-10): a failed open → no checks, one close attempt.
- **B-11** (F-11): turn-end reads ride out `TRANSIENT_READS`; any other failure ends the session with its `done`.
- **B-12** (F-12): the control is read again right before the claim.
- **B-13** (F-13): newest follow-up per step = the pending one, else greatest `(issued_at, id)`.
- **B-14** (F-14): `request_cancel` refuses the run's pending follow-ups on `Inserted` and `AlreadyPending`.
- **B-15** (F-15): the "after the last check" race is pinned in T1 and T2; T4 pins the end-to-end invariant.
- **B-16** (F-16): Postgres race tests hold one side in a raw transaction.
- **B-17** (F-17): T5 in a worktree; T1 and T2 share the primary tree.
- **B-18**: the D9 engine tests reuse MOD-42's `dropped_while_parked` shape (`engine.rs:17619-17635`) with a follow-up enqueued before the drop.
- **B-19** (F-19): `settle_follow_up`'s miss order: `NotFound` → `Constraint` → `NotPending` → `Fenced`.
- **B-20** (F-20): `turn` is boxed per call.
- **B-21**: the D12 display labels (`FOLLOW_UP_QUEUED`, `FOLLOW_UP_SENT`, `FOLLOW_UP_REFUSED`) live in `model/relay.rs` with the other D12 sentences, so T5 and T6 quote one source.
- **B-22**: every reason a close or refusal writes is a `&str` the caller passes (`FOLLOW_UP_*` constants or `executor_scrub_refusal(rule)`); the store never invents a sentence except `open_follow_ups`' (`FOLLOW_UP_SESSION_ENDED`) and `request_cancel`'s (`FOLLOW_UP_RUN_CANCELLED`), which D4/D5 fix.

---

## 1. Build order

| Task | Crates | Commits (each compiles) | Lane | Gate |
|---|---|---|---|---|
| T0 | core, store, agent, orch, htui (literals only) | (1) **red**: `model/relay.rs` types, six `WriteStore` methods, `RelayStore` +5, every implementor and literal (§2.3), MemStore bodies `Err(Backend("MOD-70 T0: red"))`, PgStore placeholders, nine cases + pins, model tests; (2) **green**: MemStore state and bodies (§2.8), cascade, readers, D5 changes, `mem.rs` unit tests | primary, alone | G-T0 |
| T1 | store | (1) red: migration pins, `pg_criteria` cases; (2) green: `0016`, `pg/relay.rs` bodies, `.sqlx` regenerated and counted | primary (shared with T2) | G-T1 |
| T2 | agent | (1) red: `tests/relay.rs` doubles and cases (fail: no loop); (2) green: `drive`'s loop, close, docs | primary (shared with T1) | G-T2 |
| T5 | htui | (1) red: `StoreRequest::FollowUp`/`StoreReply::FollowUpQueued`, `name()`, `try_serve` arm, pane tests, harness tests; (2) green: `Mode::FollowUp`, `i`, lines, snapshots | **worktree** `hr/MOD-70-t5` | G-T5 |
| — | — | merge T5 (`--no-ff`) after T1 and T2 committed; wave gate | primary, no lane running | G-W1 |
| T3 | orch | (1) red: engine tests; (2) green: D8 flag, D9 close, docs | primary | G-T3 |
| T4 | htui (tests) | (1) Postgres end-to-end cases (red only if a bug is found) | primary | G-T4 |
| T6 | docs | (1) ANA-2, worker doc, README | primary | G-Final |

---

## 2. Shared code shapes

Everything here crosses a task boundary and is settled here. A lane that believes a shape is wrong
stops and reports; it does not edit another lane's crate.

### 2.1 `htui-core/src/model/relay.rs` (T0)

Module doc (`:1-2`) becomes: "The permission and control relay's rows and outcomes (MOD-42 plan
D1-D5, D12, D13; `0011_permission_relay.sql`) and MOD-70's follow-ups (plan D1-D5, D12;
`0016_follow_up.sql`). None of the three tables is mirrored (MOD-42 OQ-4, MOD-70 D1)." New imports:
`crate::scrub::{MinimalScrubber, Scrubber, Unmasked}`, `serde_json::json`.

```rust
str_enum!(
    /// `run_command.kind` (MOD-42 plan D1; MOD-70 plan D1).
    RunCommandKind {
        /// Cancel the run (MOD-42 D12).
        Cancel => "cancel",
        /// One user follow-up for a running engine step's live session (MOD-70).
        FollowUp => "follow_up",
    }
);

/// MOD-70 D2: a follow-up's text, checked when built: not empty after trimming, and nothing a
/// pattern-only scrubber refuses. Stored and sent as typed (PRD Q3). `Debug` prints the length
/// only; there is no `Display` and no serde (I-5).
#[derive(Clone, PartialEq, Eq)]
pub struct FollowUpText(String);

impl FollowUpText {
    /// # Errors
    /// [`FollowUpTextError::Empty`] for an empty or whitespace-only text;
    /// [`FollowUpTextError::Residue`] when `MinimalScrubber::new(Vec::<String>::new())` refuses
    /// `{"text": text}` (the payload shape `record_follow_up` scrubs).
    pub fn new(text: String) -> Result<Self, FollowUpTextError>;
    /// The text as typed.
    #[must_use]
    pub fn as_str(&self) -> &str;
    /// The text as typed, moved out.
    #[must_use]
    pub fn into_string(self) -> String;
}
impl core::fmt::Debug for FollowUpText { /* debug_struct("FollowUpText").field("len", &len) */ }

/// Why a typed follow-up was not sent (D2, D12; nothing was written).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FollowUpTextError {
    #[error("{}", FOLLOW_UP_EMPTY)]
    Empty,
    /// Carries the pointer and the rule, never the text (`scrub.rs:150-167`).
    #[error("not sent: the text looks like it holds a credential ({})", .0.rule)]
    Residue(Unmasked),
}

/// Arguments of `WriteStore::request_follow_up` (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFollowUp {
    pub id: RunCommandId,          // minted by the sender
    pub run_step_id: StepId,
    pub text: FollowUpText,
    pub issued_by: UserId,
    pub issued_box: BoxId,
}

/// `WriteStore::request_follow_up`'s outcome (D3; a refusal writes nothing, MOD-42 OQ-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpRequest { Queued(RunCommandId), Refused(FollowUpRefusal) }

/// Why an enqueue wrote nothing, in D3's classification order. `Display` is D12's sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FollowUpRefusal {
    #[error("{}", FOLLOW_UP_CHAT_RUN)]       ChatRun,
    #[error("{}", FOLLOW_UP_JUDGE)]          Judge,
    #[error("{}", FOLLOW_UP_NOT_RUNNING)]    NotRunning,
    #[error("{}", FOLLOW_UP_CANCELLING)]     Cancelling,
    #[error("{}", FOLLOW_UP_ALREADY_QUEUED)] AlreadyQueued,
    #[error("{}", FOLLOW_UP_EXECUTOR_GONE)]  ExecutorGone,
    #[error("{}", FOLLOW_UP_NOT_STARTED)]    NotStarted,
    #[error("{}", FOLLOW_UP_SESSION_ENDED)]  SessionEnded,
}

/// The step's pending follow-up, as the executor reads it (`next_follow_up`, D4). `Debug` prints
/// the id and the text's length only.
#[derive(Clone, PartialEq, Eq)]
pub struct QueuedFollowUp { pub id: RunCommandId, pub text: String }

/// What `settle_follow_up` moves a pending row to (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowUpSettle {
    /// Taken for the next turn: `applied`, `resolution` NULL.
    Applied,
    /// `refused`, with this sentence as `resolution` (`executor_scrub_refusal`).
    Refused(String),
}

/// `settle_follow_up`'s outcome (D4, B-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    /// This call moved it; `text` is now NULL.
    Settled,
    /// Not pending any more (a cancel, a newer window or a close refused it).
    NotPending,
    /// Pending, but `owner` is not the run's lease owner.
    Fenced,
}

/// What the Runs pane shows for one step's newest follow-up (D5, D14). No text (OQ-6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUpView {
    pub id: RunCommandId,
    pub run_id: RunId,
    pub run_step_id: StepId,
    pub status: RunCommandStatus,
    pub resolution: Option<String>,
    pub issued_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// A row of `follow_up_window` (D1, OQ-2): one per step whose engine session takes follow-ups.
/// MemStore's state and its test-support reader (B-1); no trait method returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpWindow {
    pub run_step_id: StepId,
    pub run_id: RunId,
    pub session: RelaySessionId,
    pub owner: Uuid,                     // B-4 [DV-2 declined: drop]
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

// RelayView gains, after `cancels`:
    /// MOD-70 D5: the newest follow-up of each step of the item's non-terminal runs (B-13),
    /// `(issued_at, id)` order. Never the text (OQ-6).
    pub follow_ups: Vec<FollowUpView>,
```

D12 sentences (doc each with its variant or use):

```rust
pub const FOLLOW_UP_NOT_RUNNING: &str =
    "only a running step takes a follow-up; p promotes a parked or failed step to a chat";
pub const FOLLOW_UP_JUDGE: &str = "a judge session takes no follow-up";
pub const FOLLOW_UP_CHAT_RUN: &str = "a chat takes follow-ups in its own view";
pub const FOLLOW_UP_ALREADY_QUEUED: &str = "a follow-up is already queued";
pub const FOLLOW_UP_SESSION_ENDED: &str = "the step finished its session; promote it to continue";
pub const FOLLOW_UP_NOT_STARTED: &str = "the step's session has not started yet";
pub const FOLLOW_UP_CANCELLING: &str = "the run is being cancelled";
pub const FOLLOW_UP_RUN_CANCELLED: &str = "the run was cancelled";
pub const FOLLOW_UP_EXECUTOR_GONE: &str = "the process walking the step no longer holds the run";
pub const FOLLOW_UP_SESSION_CANCELLED: &str =
    "the step's session was cancelled before the follow-up was sent";
pub const FOLLOW_UP_EMPTY: &str = "a follow-up needs text";
pub const FOLLOW_UP_QUEUED: &str = "queued \u{2014} sent when the current turn ends"; // PRD Q7
pub const FOLLOW_UP_SENT: &str = "follow-up sent";
pub const FOLLOW_UP_REFUSED: &str = "follow-up refused";
/// D12: the executor's scrubber refused the text (D6 step 4).
#[must_use]
pub fn executor_scrub_refusal(rule: &str) -> String {
    format!("the executing box's scrubber refused the text ({rule})")
}
```

`model/mod.rs` re-exports (`:151-156`) gain `FollowUpRefusal, FollowUpRequest, FollowUpSettle,
FollowUpText, FollowUpTextError, FollowUpView, FollowUpWindow, NewFollowUp, QueuedFollowUp,
SettleOutcome`, every `FOLLOW_UP_*` and `executor_scrub_refusal`. `run_command_kind_matches_check_list`
(`:372-376`) → `&["cancel", "follow_up"]`, doc naming `0016_follow_up.sql`'s `chk_run_command_kind`.

### 2.2 `WriteStore` additions (`htui-core/src/store/traits.rs`, T0)

A `// -- MOD-70: follow-ups for engine steps (plan D1-D5, D9)` block after `answer_permission`
(`:1843-1849`), before the MOD-11 block. Module doc `:35-38` gains a MOD-70 sentence.

```rust
/// D3: one pending follow-up per step, admitted only while the step's window is open, under a
/// `FOR SHARE OF w` on that window row (`READ COMMITTED`; I-3). On a miss one re-read decides, in
/// this order: `NotFound { entity: "run_step" }`; the actor (`Constraint`, MOD-42 A-1); `ChatRun`;
/// `Judge` (`fanout_index < 0`); `NotRunning`; `Cancelling`; `AlreadyQueued`; `ExecutorGone` (the
/// run's lease is not live under the window's owner, B-4); `NotStarted` (no window);
/// `SessionEnded` (window closed). Never touches `run`, the lease or `session_event` (I-1).
/// `issued_at` is the store's clock (I-4).
/// # Errors
/// The two above; `Constraint` for a repeated id; `Constraint` after three re-reads that each found
/// every guard passing (the pending row it collided with resolved in between).
async fn request_follow_up(&self, new: NewFollowUp) -> Result<FollowUpRequest>;

/// D4, B-6: opens (or re-opens for a new session) the step's window under `owner`, then refuses
/// with `FOLLOW_UP_SESSION_ENDED` every follow-up still pending on the step (an older window's).
/// `Ok(false)` = `run.lease_owner` is not `owner` (B-3, owner only); nothing written.
/// # Errors
/// `NotFound { entity: "run_step" }`; `Constraint` when the step is not `run`'s.
async fn open_follow_ups(&self, run: RunId, step: StepId, session: RelaySessionId, owner: Uuid)
    -> Result<bool>;

/// D4: the step's pending follow-up while its window is `session`'s and open; else `None`
/// (also for an unknown step). A read: never fenced.
/// # Errors
/// The backend's own failures only.
async fn next_follow_up(&self, step: StepId, session: RelaySessionId)
    -> Result<Option<QueuedFollowUp>>;

/// D4, B-3, B-19: compare-and-set `pending → applied | refused` while `run.lease_owner = owner`;
/// always `text = NULL`, `resolved_at` the store's clock. A miss is told apart by one re-read.
/// # Errors
/// `NotFound { entity: "run_command" }`; `Constraint` for a `cancel` row.
async fn settle_follow_up(&self, id: RunCommandId, owner: Uuid, to: FollowUpSettle)
    -> Result<SettleOutcome>;

/// D4, D6 step 8, B-5: closes the step's window if it is `session`'s, then — only while it is —
/// refuses the step's pending follow-ups with `reason`. Two statements in one `READ COMMITTED`
/// transaction on Postgres (never one CTE). Idempotent. Answers how many rows it refused.
/// # Errors
/// The backend's own failures only.
async fn close_follow_ups(&self, step: StepId, session: RelaySessionId, reason: &str)
    -> Result<u64>;

/// D9, B-3, F-22: while `owner` is the run's lease owner, closes every open window of `run` and
/// refuses every pending follow-up of `run` with `reason`; otherwise `Ok(0)`, nothing written.
/// Idempotent (the sweep reaches it twice, `engine.rs:2329`, `:2347`).
/// # Errors
/// The backend's own failures only.
async fn close_dropped_follow_ups(&self, run: RunId, owner: Uuid, reason: &str) -> Result<u64>;
```

D5's doc edits on existing methods: `request_cancel` (`:1793-1803`) gains "and, in the same
transaction, refuses every `pending` follow-up of the run with `FOLLOW_UP_RUN_CANCELLED` (MOD-70 D5,
B-14; on `AlreadyPending` too)"; `pending_commands` (`:1805-1812`) "**cancels only** (MOD-70 D5, I-9)";
`resolve_command` (`:1814-1826`) "always clears `text` (MOD-70 D5)"; `relay_view` (`:1828-1833`) names
`follow_ups`.

### 2.3 `RelayStore` +5, the forwarding convention, and every compile site of T0

`RelayStore` (`htui-core/src/store/worker.rs:74-97`) gains, in this order, each documented
"[`WriteStore::<name>`]." and declared `fn … -> impl Future<Output = Result<T>> + Send`:

```rust
fn open_follow_ups(&self, run: RunId, step: StepId, session: RelaySessionId, owner: Uuid)
    -> impl Future<Output = Result<bool>> + Send;
fn next_follow_up(&self, step: StepId, session: RelaySessionId)
    -> impl Future<Output = Result<Option<QueuedFollowUp>>> + Send;
fn settle_follow_up(&self, id: RunCommandId, owner: Uuid, to: FollowUpSettle)
    -> impl Future<Output = Result<SettleOutcome>> + Send;
fn close_follow_ups(&self, step: StepId, session: RelaySessionId, reason: &str)
    -> impl Future<Output = Result<u64>> + Send;
fn close_dropped_follow_ups(&self, run: RunId, owner: Uuid, reason: &str)
    -> impl Future<Output = Result<u64>> + Send;
```

(Edition 2024: the return-position `impl Future` captures `reason`'s lifetime with no annotation.)
Trait doc (`:71-73`): "…while a request is parked, and its follow-up window (MOD-70 D4)". Module doc
(`:15-18`): "[`RelayStore`] is nine (MOD-42 plan D2, MOD-70 plan D4)". `WorkerStore` keeps its 58 own
methods; the engine reaches `close_dropped_follow_ups` through the supertrait.

**Forwarding convention** (unchanged from MOD-42): an implementor of a narrow trait forwards to the
**same-named `WriteStore` method by path** — `WriteStore::open_follow_ups(self, run, step, session,
owner).await` — because the module sees both families (E0034). `Writer`'s `WriteStore` impl matches
`Memory`/`Online` and calls the inner store with method syntax (its module imports only
`WriteStore`). The spies forward `self.inner.<name>(…)`.

**Every site T0 must touch for the workspace to compile** (verified: `impl … for` grep over
`(WriteStore|RelayStore|WorkerStore|RecorderStore)(<…>)? for` finds exactly these; `Relay {` and
`cancels:` literal greps find exactly the literal sites):

| # | File | Symbol / anchor | Change |
|---|---|---|---|
| 1 | `htui-core/src/model/relay.rs` | whole | §2.1 |
| 2 | `htui-core/src/model/mod.rs` | re-exports `:151-156`; test `:372-376` | §2.1 |
| 3 | `htui-core/src/store/traits.rs` | `WriteStore` `:1752-1849`; module doc `:35-38`; imports | §2.2 |
| 4 | `htui-core/src/store/worker.rs` | `RelayStore` `:74-97`; `impl RelayStore for MemStore` `:528-549`; doc `:15-18`; imports `:29-39` | +5 trait fns; +5 UFCS forwards |
| 5 | `htui-core/src/store/mem.rs` | `State` `:269-271`; `from_demo` `:375-376`; readers `:913-920`; `delete_project` `:4206-4210`; relay `impl State` `:6607-6963`; `impl WriteStore for MemStore` relay block `:7899-7972`; tests | §2.8 |
| 6 | `htui-core/src/store/conformance.rs` | `CASES` `:57-206`; dispatch arms; imports `:17-41`; `deleting_a_project_takes_its_relay_rows` `:15885` | §3.3 |
| 7 | `htui-core/tests/mem_store.rs` | `:37` `148,` and its sentence | → 157, "MOD-70 T0's nine follow-up cases (plan D1-D5, D9)" |
| 8 | `htui-store/tests/pg_conformance.rs` | `EXPECTED_CASES` `:31`, doc `:17-30`, message `:38` | → 157 |
| 9 | `htui-store/src/pg/write.rs` | relay delegations `:6567-6620` | +6 `relay::<name>(self, …)` |
| 10 | `htui-store/src/pg/relay.rs` | new `pub(super)` fns; `relay_view` literal `:546` | +6 placeholders `Err(StoreError::Backend("MOD-70 T1: not yet implemented".into()))` (params `_`-prefixed); `follow_ups: Vec::new()` |
| 11 | `htui-store/src/writer.rs` | relay block `:1378-1472` | +6 `match self { Memory(s) => s.<name>(…), Online(pg) => pg.<name>(…) }` |
| 12 | `htui-store/src/worker.rs` | `impl RelayStore for PgStore` `:57-78`; `for Writer` `:394-415` | +5 each, UFCS to `WriteStore` |
| 13 | `htui-agent/src/conformance.rs` | `impl<S: WriteStore> WriteStore for UsageSpy` relay block `:1405-1471` | +6 `self.inner.<name>(…)` (never logged, as MOD-42's) |
| 14 | `htui-agent/tests/recorder.rs` | `impl WriteStore for SpyStore` `:1131-1196` | +6 forwards |
| 15 | `htui-agent/src/record/relay.rs` | `Relay` `:101-120`; `Debug` `:122-135`; `NoRelay` `:142-169` | `pub follow_ups: bool` (doc: "MOD-70 D6: whether this session opens a follow-up window; `drive_once` sets it for main and candidate sessions, D8"), `.field("follow_ups", …)`, +5 `match *self {}` stubs |
| 16 | `htui-agent/tests/relay.rs` | `relay_over` literal `:177-187`; second literal `~:1441-1451`; `FailingReads` `:467-506`; `FailingSettles` `:510-540` | `follow_ups: false` ×2; +5 UFCS forwards each to `&self.store` / `&self.0` |
| 17 | `htui-orch/src/engine.rs` | `drive_once`'s `Relay` literal `:6116-6126` | `follow_ups: false` (T3 flips it) |
| 18 | `htui/src/ui/tabs/backlog/detail/runs.rs` | test helper `fn relay` `:5235-5241` | `follow_ups: Vec::new()` |

Nothing else names `RelayView { … }` or a `Relay { … }` literal, and no type outside the table
implements any store family. `StoreRequest`/`StoreReply` are T5's (§2.5), not T0's.

### 2.4 `htui-agent` (T0 field, T2 loop)

`Relay` gains `pub follow_ups: bool` (T0). `drive`'s signature is unchanged (`pump`'s 26 call sites
and `drive_once` compile as they are). T2's body is §5.2.

### 2.5 `htui` store request and reply (T5)

```rust
// StoreRequest, after AnswerPermission (store_worker.rs:842-849):
    /// MOD-70 plan D13: one follow-up for a running engine step (D3). Refused offline with
    /// `DATABASE_UNREACHABLE`. `text`'s `Debug` prints its length only (I-5).
    FollowUp {
        /// The step whose session takes it.
        step: StepId,
        /// Checked at construction (D2): the typing box refused a residue already.
        text: FollowUpText,
    },
// name(), after :1123:
            Self::FollowUp { .. } => "follow_up",
// StoreReply, after PermissionAnswered (:1391-1397):
    /// [`StoreRequest::FollowUp`] queued its row; a refusal is [`StoreReply::Failed`] with the
    /// refusal's sentence (MOD-70 D13).
    FollowUpQueued {
        /// The step it was queued for.
        step: StepId,
    },
```

`try_serve` arm, after `AnswerPermission`'s (`:1988-2015`), same shape:

```rust
        // MOD-70 D3, D13: refused offline before anything is sent; a refusal is its sentence.
        StoreRequest::FollowUp { step, text } => {
            let writer = backend.writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let user = backend.this_user().await?;
            let box_id = /* as AnswerPermission: box_info()…box_id, NotFound { entity: "box" } */;
            let new = NewFollowUp { id: RunCommandId::new(), run_step_id: *step, text: text.clone(),
                                    issued_by: user, issued_box: box_id };
            match WriteStore::request_follow_up(&writer, new).await? {
                FollowUpRequest::Queued(_) => StoreReply::FollowUpQueued { step: *step },
                FollowUpRequest::Refused(why) => StoreReply::Failed {
                    request: request.name(), message: why.to_string() },
            }
        }
```

Not an `Orch` request: `run_worker.rs:126-136` is untouched.

### 2.6 Migration `htui-store/migrations/0016_follow_up.sql` (T1), in full

```sql
-- 0016_follow_up.sql - MOD-70 (plan D1, D3, D4, D5).
-- Forward-only (R-STO-5).
--
-- Follow-ups for engine steps. A run_command row of kind follow_up carries one user follow-up for
-- one running step's live engine session: any store client queues it, the process walking the
-- step takes it at the session's next turn end and records the scrubbed follow_up event itself
-- (session_event stays single-writer). Its text is plaintext only while the row is pending and is
-- nulled when it resolves (chk_run_command_follow_up). follow_up_window holds one row per step whose
-- engine session takes follow-ups; closed_at is set when that session ends, and the enqueue's
-- FOR SHARE on the window row against the close's UPDATE is what makes a follow-up that misses the
-- last turn end a refusal rather than a stranded row. A step's status is unchanged: there is no new
-- state. Every time is clock_timestamp(). Both tables go with their run and step (ON DELETE
-- CASCADE), which delete_project relies on. Neither is mirrored, but schema_version becomes 16, so
-- each box rebuilds its mirror once. A headless worker never migrates: migrate from a TUI first, and
-- upgrade every box together - a pre-0016 binary's cancel no longer infers an index here.

ALTER TABLE run_command
    ADD COLUMN run_step_id UUID REFERENCES run_step(id) ON DELETE CASCADE,
    ADD COLUMN text        TEXT;

ALTER TABLE run_command DROP CONSTRAINT chk_run_command_kind;
ALTER TABLE run_command
    ADD CONSTRAINT chk_run_command_kind CHECK (kind IN ('cancel', 'follow_up'));
ALTER TABLE run_command
    ADD CONSTRAINT chk_run_command_follow_up CHECK (
        (kind = 'cancel' AND run_step_id IS NULL AND text IS NULL)
        OR (kind = 'follow_up' AND run_step_id IS NOT NULL
            AND (status = 'pending') = (text IS NOT NULL)));

-- One pending cancel per run, as before; one pending follow-up per step (PRD Q4). Two plain partial
-- indexes so each ON CONFLICT target infers a plain column list.
DROP INDEX uq_run_command_pending;
CREATE UNIQUE INDEX uq_run_command_pending_cancel ON run_command (run_id)
    WHERE status = 'pending' AND kind = 'cancel';
CREATE UNIQUE INDEX uq_run_command_pending_follow_up ON run_command (run_step_id)
    WHERE status = 'pending' AND kind = 'follow_up';

CREATE TABLE follow_up_window (
    run_step_id UUID        PRIMARY KEY REFERENCES run_step(id) ON DELETE CASCADE,
    run_id      UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    session     UUID        NOT NULL,
    owner       UUID        NOT NULL,                     -- B-4 [DV-2 declined: drop]
    opened_at   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    closed_at   TIMESTAMPTZ
);
CREATE INDEX idx_follow_up_window_open ON follow_up_window (run_id) WHERE closed_at IS NULL;
```

`run_step.status`'s CHECK is not touched (PRD metric "No new step state"); T1's migration test
asserts it byte-for-byte unchanged.

### 2.7 PgStore SQL (T1; bodies in `htui-store/src/pg/relay.rs`, delegated from `write.rs`)

Module doc (`:1-18`) gains a MOD-70 paragraph: the enqueue is the one `INSERT … SELECT … FOR SHARE
OF w`; the closes are the only two-statement transactions, READ COMMITTED on purpose (F-18).
Constants: `FOLLOW_UP_ATTEMPTS: usize = 3` (as `CANCEL_ATTEMPTS`). A private helper
`lock_step_fence(tx, step) -> Result<Option<StepFenceRow>>` holds `open_permission`'s fence query
**verbatim** (`:139-147`), and both `open_permission` and `open_follow_ups` call it, so `.sqlx` keeps
one entry for it.

**`request_follow_up`** (D3; loop `FOLLOW_UP_ATTEMPTS` times):

```sql
INSERT INTO run_command (id, run_id, kind, run_step_id, text, issued_by, issued_box)
SELECT $1, s.run_id, 'follow_up', s.id, $3, $4, $5
  FROM follow_up_window w
  JOIN run_step s ON s.id = w.run_step_id
  JOIN run r      ON r.id = s.run_id
 WHERE w.run_step_id = $2
   AND w.closed_at IS NULL
   AND r.kind = 'graph'
   AND s.fanout_index >= 0
   AND s.status = 'running'
   AND r.lease_owner = w.owner                       -- B-4 [DV-2 declined: r.lease_owner IS NOT NULL]
   AND r.lease_expires_at > clock_timestamp()
   AND NOT EXISTS (SELECT 1 FROM run_command c
                    WHERE c.run_id = s.run_id AND c.kind = 'cancel' AND c.status = 'pending')
   FOR SHARE OF w
ON CONFLICT (run_step_id) WHERE status = 'pending' AND kind = 'follow_up' DO NOTHING
RETURNING id AS "id: RunCommandId"
```

`Ok(Some(id))` → `Queued(id)`. `Err(Constraint)` (unknown actor FK, repeated id) is held and the
re-read decides first (MOD-42 A-1). `Ok(None)` → the re-read:

```sql
SELECT r.kind                AS "run_kind: RunKind",
       s.fanout_index,
       s.status              AS "step_status: StepStatus",
       EXISTS (SELECT 1 FROM app_user WHERE id = $2) AS "user_known!",
       EXISTS (SELECT 1 FROM box WHERE id = $3)      AS "box_known!",
       EXISTS (SELECT 1 FROM run_command c
                WHERE c.run_id = s.run_id AND c.kind = 'cancel' AND c.status = 'pending')
                                                     AS "cancelling!",
       EXISTS (SELECT 1 FROM run_command c
                WHERE c.run_step_id = s.id AND c.kind = 'follow_up' AND c.status = 'pending')
                                                     AS "queued!",
       w.run_step_id IS NOT NULL                     AS "window!",
       w.closed_at IS NOT NULL                       AS "window_closed!",
       w.owner                                       AS "window_owner?",
       r.lease_owner                                 AS "lease_owner?",
       (r.lease_expires_at IS NOT NULL AND r.lease_expires_at > clock_timestamp())
                                                     AS "lease_live!"
  FROM run_step s
  JOIN run r ON r.id = s.run_id
  LEFT JOIN follow_up_window w ON w.run_step_id = s.id
 WHERE s.id = $1
```

Classification in Rust, exactly MemStore's order (§2.8): `None` → `NotFound { entity: "run_step" }`;
`require_actor(…, "run_command", "issued")`; `Chat` → `ChatRun`; `fanout_index < 0` → `Judge`; status ≠ `Running` → `NotRunning`; `cancelling` →
`Cancelling`; `queued` → `AlreadyQueued`; `!(lease_live && lease_owner.is_some() && (window_owner
is None || lease_owner == window_owner))` → `ExecutorGone`; `!window` → `NotStarted`;
`window_closed` → `SessionEnded`; otherwise every guard passes now: a held insert `Constraint` (a
repeated id; MemStore checks the id last too) is returned, else (the colliding row resolved, or the
lock released a close that was rolled back) retry. After `FOLLOW_UP_ATTEMPTS`, `Constraint`
("run_command: step `{step}`'s follow-up guards changed between the insert and the re-read {n}
times; nothing was written").

**`open_follow_ups`** (D4, B-3, B-6): one transaction.

```sql
-- 1: lock_step_fence(&mut tx, step)  (open_permission's query, verbatim)
--    None → NotFound{run_step}; run_id ≠ run → Constraint (open_permission's sentence, "follow_up_window.run_step_id");
--    lease_owner ≠ Some(owner) → rollback, Ok(false)
-- 2:
INSERT INTO follow_up_window (run_step_id, run_id, session, owner)
VALUES ($1, $2, $3, $4)
ON CONFLICT (run_step_id) DO UPDATE
   SET session = EXCLUDED.session, owner = EXCLUDED.owner,
       opened_at = clock_timestamp(), closed_at = NULL
-- 3: ($2 = FOLLOW_UP_SESSION_ENDED)
UPDATE run_command
   SET status = 'refused', resolution = $2, text = NULL, resolved_at = clock_timestamp()
 WHERE run_step_id = $1 AND kind = 'follow_up' AND status = 'pending'
-- commit; Ok(true)
```

**`next_follow_up`**:

```sql
SELECT c.id AS "id: RunCommandId", c.text AS "text!"
  FROM run_command c
  JOIN follow_up_window w ON w.run_step_id = c.run_step_id
 WHERE c.run_step_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending'
   AND w.session = $2 AND w.closed_at IS NULL
```

**`settle_follow_up`** (B-3 owner only [DV-1 declined: add `AND r.lease_expires_at >
clock_timestamp()`]):

```sql
UPDATE run_command c
   SET status = $3, resolution = $4, text = NULL, resolved_at = clock_timestamp()
  FROM run r
 WHERE c.id = $1 AND c.kind = 'follow_up' AND c.status = 'pending'
   AND r.id = c.run_id AND r.lease_owner = $2
```

`$3` = `"applied"` / `"refused"`, `$4` = `None` / the sentence. One row → `Settled`. Zero → re-read
`SELECT kind AS "kind: RunCommandKind", status AS "status: RunCommandStatus" FROM run_command WHERE
id = $1`: `None` → `NotFound { entity: "run_command" }`; `Cancel` → `Constraint("run_command `{id}`
is not a follow-up")`; not `Pending` → `NotPending`; else `Fenced` (B-19).

**`close_follow_ups`** (B-5; `store.pool.begin()`, **two statements**, never a CTE, never
`begin_repeatable_read`):

```sql
-- 1
UPDATE follow_up_window SET closed_at = clock_timestamp()
 WHERE run_step_id = $1 AND session = $2 AND closed_at IS NULL
-- 2 (a fresh READ COMMITTED snapshot: sees an enqueue that held FOR SHARE and committed while 1 waited)
UPDATE run_command c
   SET status = 'refused', resolution = $3, text = NULL, resolved_at = clock_timestamp()
 WHERE c.run_step_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending'
   AND EXISTS (SELECT 1 FROM follow_up_window w WHERE w.run_step_id = $1 AND w.session = $2)
-- commit; answer statement 2's rows_affected
```

**`close_dropped_follow_ups`** (D9; same transaction shape [DV-1 declined: add the live predicate]):

```sql
-- 1
UPDATE follow_up_window w SET closed_at = clock_timestamp()
  FROM run r
 WHERE w.run_id = $1 AND w.closed_at IS NULL AND r.id = w.run_id AND r.lease_owner = $2
-- 2
UPDATE run_command c
   SET status = 'refused', resolution = $3, text = NULL, resolved_at = clock_timestamp()
  FROM run r
 WHERE c.run_id = $1 AND c.kind = 'follow_up' AND c.status = 'pending'
   AND r.id = c.run_id AND r.lease_owner = $2
```

**`request_cancel`** (D5, B-14; `:366-431`): each attempt opens `store.pool.begin()`; the insert's
target becomes

```sql
INSERT INTO run_command (id, run_id, kind, issued_by, issued_box)
VALUES ($1, $2, 'cancel', $3, $4)
ON CONFLICT (run_id) WHERE status = 'pending' AND kind = 'cancel' DO NOTHING
RETURNING id AS "id: RunCommandId"
```

`Some(id)` → `refuse_run_follow_ups(&mut *tx, run)` → commit → `Inserted(id)`. Otherwise the
transaction is dropped (rolled back) and the existing re-read runs on the pool; `AlreadyPending(x)`
calls `refuse_run_follow_ups(&store.pool, run)` first. The helper:

```sql
UPDATE run_command
   SET status = 'refused', resolution = $2, text = NULL, resolved_at = clock_timestamp()
 WHERE run_id = $1 AND kind = 'follow_up' AND status = 'pending'
```

(`$2 = FOLLOW_UP_RUN_CANCELLED`). Note: after `0016` the **old** target errors
("no unique or exclusion constraint matching the ON CONFLICT specification", fact-check probe), so
the migration and this edit land in one commit.

**`pending_commands`** (`:434-465`, I-9): `WHERE c.status = 'pending' AND c.kind = 'cancel' AND (…)`.
**`resolve_command`** (`:468-504`): `SET status = $2, resolution = $3, text = NULL, resolved_at =
clock_timestamp()`. **`relay_view`** (`:506-552`) gains a third read:

```sql
SELECT n.id AS "id!: RunCommandId", n.run_id AS "run_id!: RunId",
       n.run_step_id AS "run_step_id!: StepId", n.status AS "status!: RunCommandStatus",
       n.resolution, n.issued_at AS "issued_at!", n.resolved_at
  FROM (SELECT DISTINCT ON (c.run_step_id)
               c.id, c.run_id, c.run_step_id, c.status, c.resolution, c.issued_at, c.resolved_at
          FROM run_command c JOIN run r ON r.id = c.run_id
         WHERE r.item_id = $1 AND c.kind = 'follow_up'
           AND r.status NOT IN ('done', 'failed', 'cancelled')
         ORDER BY c.run_step_id, (c.status = 'pending') DESC, c.issued_at DESC, c.id DESC) n
 ORDER BY n.issued_at, n.id
```

(The `!` overrides are needed: sqlx cannot infer non-null through the subquery.)

### 2.8 MemStore reference semantics (`htui-core/src/store/mem.rs`, T0)

`State` gains (beside `run_commands`, `:271`), and `from_demo` initialises both empty:

```rust
    /// `run_command`'s MOD-70 columns for `follow_up` rows, by id (B-2): `run_step_id` and the
    /// text, `None` once the row resolves. The row itself is in `run_commands`.
    follow_up_payloads: BTreeMap<RunCommandId, FollowUpPayload>,
    /// `follow_up_window` (MOD-70 D1), by step.
    follow_up_windows: BTreeMap<StepId, FollowUpWindow>,
```

with a private `#[derive(Debug, Clone)] struct FollowUpPayload { run_step_id: StepId, text:
Option<String> }` whose `Debug` is hand-written (length only). Every method takes the handle's
`now` (I-4) and is one closure under the write lock, so the Postgres races collapse to "first one in".
In a new `impl State` block after MOD-42's (`:6963`), "MOD-70 (plan D1-D5, D9; blueprint §2.8)":

- `fn refuse_follow_ups(&mut self, pick: impl Fn(&RunCommand, &FollowUpPayload) -> bool, reason:
  &str, now) -> u64` — every `pending` `FollowUp` row `pick` selects: `status = Refused`,
  `resolution = Some(reason)`, `resolved_at = Some(now)`, payload `text = None`. Shared by every
  path below.
- `request_follow_up(new, now)` — **D3's order**: (1) `steps.get(new.run_step_id)` else
  `NotFound { entity: "run_step" }`; `require_actor(issued_by, issued_box, "run_command", "issued")`;
  (2) `runs[step.run_id].kind == RunKind::Chat` → `ChatRun`; (3) `step.fanout_index < 0` → `Judge`;
  (4) `step.status != StepStatus::Running` → `NotRunning`; (5) a pending `Cancel` of the run →
  `Cancelling`; (6) a pending `FollowUp` of the step → `AlreadyQueued`; (7) `let window =
  follow_up_windows.get(step)`; `match window { Some(w) => live_owner(run, now) != Some(w.owner),
  None => live_owner(run, now).is_none() }` → `ExecutorGone` [DV-2 declined: `live_owner(run,
  now).is_none()` in both arms]; (8) `None` → `NotStarted`; `closed_at.is_some()` → `SessionEnded`;
  then `run_commands.contains_key(&new.id)` → `Constraint(already_exists("run_command", new.id))`;
  insert the `RunCommand { kind: FollowUp, status: Pending, resolution: None, issued_at: now,
  resolved_at: None, … }` and the payload `{ run_step_id, text: Some(new.text.into_string()) }` →
  `Queued(new.id)`.
- `open_follow_ups(run, step, session, owner, now)` — `NotFound { entity: "run_step" }`;
  `step.run_id != run` → `Constraint` (`"follow_up_window.run_step_id `{step}` is not a step of run
  `{run}`"`); `lease_owners.get(&run) != Some(&owner)` → `Ok(false)` [DV-1 declined:
  `live_owner(run, now) != Some(owner)`]; insert/replace the window `{ run_step_id: step, run_id: run,
  session, owner, opened_at: now, closed_at: None }`; `refuse_follow_ups(step's pending,
  FOLLOW_UP_SESSION_ENDED)`; `Ok(true)`.
- `next_follow_up(step, session)` — the window is `session`'s with `closed_at == None`, then the
  step's pending `FollowUp` row with its payload text → `QueuedFollowUp { id, text }`; else `None`.
- `settle_follow_up(id, owner, to, now)` — B-19: unknown → `NotFound { entity: "run_command" }`;
  `kind != FollowUp` → `Constraint`; `status != Pending` → `NotPending`;
  `lease_owners.get(&row.run_id) != Some(&owner)` → `Fenced` [DV-1 declined: `live_owner`]; else
  move it (`Applied` → `resolution: None`; `Refused(s)` → `Some(s)`), `resolved_at = now`, text
  `None` → `Settled`.
- `close_follow_ups(step, session, reason, now)` — B-5: if the window is `session`'s, set
  `closed_at = Some(now)` when `None`, then `refuse_follow_ups(step's pending, reason)`; else `0`.
- `close_dropped_follow_ups(run, owner, reason, now)` — `lease_owners.get(&run) != Some(&owner)` →
  `0`; else close every window of `run` with `closed_at == None`, then refuse every pending
  `FollowUp` of `run`.
- `request_cancel` (`:6881-6913`, B-14): on both `Inserted` and `AlreadyPending`, `refuse_follow_ups(
  run's pending, FOLLOW_UP_RUN_CANCELLED)` before answering.
- `pending_commands` (`:6915-6937`, D5): the filter gains `c.kind == RunCommandKind::Cancel`.
- `resolve_command` (`:6939-6962`, D5): also sets the payload's `text = None` when one exists.
- `relay_view` (`:6808-6835`, B-13): `follow_ups` = for each `FollowUp` row of the item's
  non-terminal runs, grouped by payload `run_step_id`, the pending row if any else the greatest
  `(issued_at, id)`; sorted by `(issued_at, id)`; mapped to `FollowUpView`.
- `delete_project` (`:4206-4210`): `follow_up_windows.retain(|_, w| !gone.runs.contains(&w.run_id))`;
  after `run_commands` retains, `follow_up_payloads.retain(|id, _| run_commands.contains_key(id))`.

`impl WriteStore for MemStore` gains six wrappers after `open_permissions` (`:7969-7972`), each `let now
= self.now(); self.write(|state| state.<name>(…, now))` (`next_follow_up` uses `self.read`).
`impl RelayStore for MemStore` (`worker.rs:528-549`) gains five UFCS forwards.

Test-support readers (B-1), after `command_rows` (`:918-920`):

```rust
    /// Every `follow_up` row as the Runs pane would see it, with whether its text is still stored
    /// (MOD-70 blueprint B-1): `true` exactly while it is pending (I-5).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn follow_up_rows(&self) -> Vec<(FollowUpView, bool)>;
    /// Every `follow_up_window` row, in step-id order (MOD-70 blueprint B-1).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn follow_up_windows(&self) -> Vec<FollowUpWindow>;
```

### 2.9 Constants the pane reads (T5)

`runs.rs`: `const FOLLOW_UP: &str = "follow_up";` beside `ANSWER_PERMISSION` (`:113`);
`const FOLLOW_UP_INDENT: usize = 2;` beside `INDENT` (B-7). Every sentence is imported from
`htui_core::model` (B-21).

---

## 3. T0 — contracts and the MemStore reference (serial, first)

### 3.1 Files

Exactly §2.3's eighteen rows. Nothing in `htui-store/migrations`, `.sqlx`, `store_worker.rs` or
`drive`'s body.

### 3.2 Commits

1. **red** — §2.1, §2.2, §2.3 rows 3–18; MemStore's six `WriteStore` bodies are
   `Err(StoreError::Backend("MOD-70 T0: red".into()))` and the D5 changes are not made yet; the nine
   cases and both pins land; model tests land (green on arrival). Compiles workspace-wide
   (`SQLX_OFFLINE=true`): the new cases fail at their first `.expect`.
2. **green** — §2.8 complete, the readers, the `mem.rs` unit tests. `grep -rn "MOD-70 T0: red"
   crates` prints nothing; `"MOD-70 T1: not yet implemented"` stays (six fns, T1 removes them).

### 3.3 Tests (written first)

Store conformance (generic `<S: WriteStore>`; `mem_store.rs` runs them now, `pg_conformance.rs` once
T1 lands). Names appended to `CASES` after `open_permissions_list_live_pending_item_requests`, arms
before `other => panic!`. New helpers in the MOD-70 section: `follow_up_step(case, store, a, at) ->
(RunId, StepId)` (`leased_step` `:6627` then `transition_step(Pending → Running)`), `window_of(case,
store, run, step, a) -> RelaySessionId` (asserts `open_follow_ups` → `true`), `follow_up(step, text)
-> NewFollowUp` (`ids::USER`, `ids::BOX`, `FollowUpText::new(text.to_owned()).expect(…)`),
`queued(case, store, new) -> RunCommandId`, `view_of(case, store, step) -> Option<FollowUpView>`
(`relay_view(ids::HTUI_ANA_2)`). **Box limit**: a box runs at most two claimed runs at once
(`claimed`'s doc, H-6); a case uses at most two leased runs and several steps per run (distinct
`(position, attempt, fanout_index)` slots). Red reason for all nine: `Backend("MOD-70 T0: red")`.

| Case | Asserts |
|---|---|
| `a_follow_up_is_refused_in_the_documented_order` | Run A (owner `a`): steps `pending` (0,1,0), judge (0,1,-1) running, `nowin` (1,1,0) running, `main` (2,1,0) running + window, `closed` (3,1,0) running + window closed by `close_follow_ups`. Unknown step → `NotFound { entity: "run_step" }`; unknown user on `pending` → `Constraint` (actor before status); `pending` → `NotRunning`; judge → `Judge`; `nowin` → `NotStarted`; `closed` → `SessionEnded`; `main` → `Queued`, again → `AlreadyQueued`. `request_cancel(A)`; judge → `Judge` (3 before 5); `nowin` → `Cancelling` (5 before 7, 8). Run B (owner `a`): running step + window; zero-TTL `refresh_lease` → `ExecutorGone` (7 before 8, using a no-window sibling step: `ExecutorGone`, not `NotStarted`); `take_lease` by owner `b` → the windowed step still `ExecutorGone` (B-4; **[DV-2 declined: `Queued`]**). A chat run (`start_chat_run`) step → `ChatRun`. Nothing a refusal answered was written (`view_of` unchanged). |
| `a_step_has_at_most_one_pending_follow_up` | Two running windowed steps of one run: queue on `s1` → `Queued(x)`, again → `AlreadyQueued` and `next_follow_up(s1)` still `x` with the first text; `s2` → `Queued` (per step, not per run); `settle_follow_up(x, a, Applied)` → `Settled`, then `s1` → `Queued(y)`, `y != x`. `request_cancel` twice → `Inserted(c)`, `AlreadyPending(c)` (cancels stay one per run under the reshaped index). |
| `settling_a_follow_up_is_a_fenced_compare_and_set` | Queue `x`: `settle(x, b, Applied)` → `Fenced`, `next_follow_up` still answers `x`; `settle(x, a, Applied)` → `Settled`; `next_follow_up` → `None`; again → `NotPending`; unknown id → `NotFound { entity: "run_command" }`; a cancel's id → `Constraint`. Queue `y`; `settle(y, a, Refused(executor_scrub_refusal("anthropic_api_key")))` → `Settled`; `view_of` = refused with that resolution. Queue `z`; zero-TTL lapse of `a`'s lease; `settle(z, a, Applied)` → `Settled` (B-3; **[DV-1 declined: `Fenced`]**). |
| `closing_the_window_refuses_what_is_still_queued` | Window `s1`, queue `x`; `close_follow_ups(step, s1, FOLLOW_UP_SESSION_ENDED)` → `1`; `view_of` refused/`FOLLOW_UP_SESSION_ENDED`; enqueue → `SessionEnded`; close again → `0`. B-5: re-open with `s2` (`window_of`), queue `y`; `close_follow_ups(step, s1, …)` (the superseded session) → `0` and `y` still pending; `close_follow_ups(step, s2, …)` → `1`. |
| `opening_a_window_refuses_an_older_windows_follow_up` | `s1` open, queue `x`; `open_follow_ups(…, s2, a)` → `true`; `x` refused `FOLLOW_UP_SESSION_ENDED`; `next_follow_up(step, s1)` → `None`; queue `y` → `Queued` (window `s2` open); `open_follow_ups(…, s3, b)` → `false` and `y` still pending, `next_follow_up(step, s2)` still `y`; unknown step → `NotFound { entity: "run_step" }`; a step of another run → `Constraint`. |
| `a_cancel_refuses_the_runs_pending_follow_ups` | Run A: two windowed steps with pending rows; run B: one. `request_cancel(A)` → `Inserted`; A's two rows refused `FOLLOW_UP_RUN_CANCELLED`, B's untouched; enqueue on A → `Cancelling`. |
| `pending_commands_list_cancels_only` | A pending follow-up: `pending_commands(a, BOX)` empty; after `request_cancel` (which refuses the follow-up) only the cancel id is listed; a follow-up on run B (pending) is never listed under B's owner either (I-9). |
| `relay_view_lists_the_newest_follow_up_per_step_without_text` | Run A, step 1: `x` refused by a close, re-opened, `y` pending → listed `y` only; step 2: `z` applied → listed applied; run B: a row, then `finish_run(B, …, Done)` → B's row absent; order `(issued_at, id)`. `FollowUpView` has no text by type. |
| `closing_a_dropped_walks_windows_needs_the_lease` | Two windowed steps of run A with pending rows: `close_dropped_follow_ups(A, b, …)` → `0`, rows still pending, windows open (enqueue on a third windowed step admitted); `close_dropped_follow_ups(A, a, FOLLOW_UP_SESSION_ENDED)` → `3`; every window closed (enqueue → `SessionEnded`); again → `0`; run B's row untouched. |

`deleting_a_project_takes_its_relay_rows` (`:15885`, extended, not a new case): the step also gets
`running`, a window and a pending follow-up `f`; after `delete_project`: `next_follow_up(step,
session)` → `None`; `settle_follow_up(f, a, Applied)` → `NotFound { entity: "run_command" }`.

MemStore unit tests (`mem.rs` `mod tests`): `delete_project_leaves_no_row_in_any_map` gains
`follow_up_windows`/`follow_up_payloads` asserts (red: the maps survive); `relay_times_are_the_handles_clock`
(`~:9742`) gains a window open/close and a follow-up queue/settle (`issued_at`, `resolved_at`,
`opened_at`, `closed_at` all equal `t`); new `a_follow_ups_text_is_cleared_on_every_resolution` —
one row resolved by each of `settle(Applied)`, `settle(Refused)`, `close_follow_ups`, a superseding
`open_follow_ups`, `request_cancel`, `close_dropped_follow_ups`, `resolve_command`: every
`follow_up_rows()` flag `false`, and `true` for the one left pending.

Model unit tests (`model/relay.rs` or `model/mod.rs` `mod tests`):
`follow_up_text_refuses_empty_and_whitespace_only_text` (`Empty`, Display `FOLLOW_UP_EMPTY`);
`follow_up_text_refuses_a_credential_and_names_only_its_rule` (`"use sk-ant-api03-…"` → `Residue`,
Display contains `anthropic_api_key`, neither Display nor `Debug` contains `sk-ant`);
`follow_up_text_keeps_prose_as_typed` (`"  use the smaller fixture\t"` round-trips byte for byte);
`follow_up_texts_debug_prints_its_length_only`; `a_queued_follow_ups_debug_prints_its_length_only`;
`run_command_kind_matches_check_list` (`["cancel", "follow_up"]`).

### 3.4 Gate

G-T0 (§12). Every `htui-store` test run in T0 is `env -u HTUI_TEST_DATABASE_URL` (the placeholders
would fail `pg_conformance`'s nine new cases; they are T1's acceptance).

---

## 4. T1 — Postgres (parallel with T2 and T5; the wave's only Postgres lane)

### 4.1 Files

| File | Change |
|---|---|
| `htui-store/migrations/0016_follow_up.sql` (new) | §2.6 verbatim |
| `htui-store/src/pg/relay.rs` | §2.7: six bodies replace T0's placeholders; `lock_step_fence` helper (and `open_permission` calls it); `request_cancel`, `pending_commands`, `resolve_command`, `relay_view`; module doc |
| `htui-store/.sqlx/*` | regenerated through a scratch database (§12 `regen`); count restated |
| `htui-store/tests/pg_criteria.rs` | §4.3 |
| `htui-store/tests/migrations.rs` | `TABLES` (`:29-76`) gains a `// 0016_follow_up.sql (MOD-70)` group with `"follow_up_window"`; `42 → 43` and its messages (`:120-127`); `vec![1..=15]` → `…16` and its sentence (`:96-104`, "and MOD-70's 0016_follow_up.sql"); `Pending(15)` (`:1088-1089`, `:1189-1190`), `MigrationsPending(15)` (`:1198-1199`, `:1221-1222`), `_sqlx_migrations` 15 (`:1373-1375`) → 16; new `migration_0016_reshapes_run_command_and_adds_the_follow_up_window` beside the `0015` tests (`:511`, `:526`) |
| `htui-store/tests/connect.rs` | `15 → 16` at `:140-142`, `:156-158`, `:242-244`, sentences naming `0016_follow_up.sql` |

T1 never edits `htui-core`: a MemStore/Postgres disagreement is reported to the main thread.

### 4.2 Commits

1. **red** — the pins and the `pg_criteria` cases. Red: 15 migrations / 42 tables; the cases meet
   the placeholders.
2. **green** — `0016` and every SQL change in one commit (the old `ON CONFLICT` target errors on a
   migrated database), `.sqlx` regenerated. `grep -rn "MOD-70 T1: not yet implemented" crates`
   empty; `ls crates/htui-store/.sqlx | wc -l` restated in the message.

### 4.3 Tests (written first, `pg_criteria.rs`; `common::demo_db()`, one scratch database per test, each ends with `db.drop_db().await`)

A second client is `PgStore::connect(&db.url, &identity::load_or_mint(tmp.path())?)` over a
`tempfile::tempdir()` (a second box), as MOD-42's `a_second_box_answers_and_the_executor_applies`
does. A has the lease (`claim_run` with owner `a`), the step `running`, the window open.

| Test | Asserts |
|---|---|
| `a_follow_up_from_another_box_is_applied_by_the_executor` | B enqueues → `Queued(x)`; A's `next_follow_up` → `x` with B's text; A's `settle(x, a, Applied)` → `Settled`; `SELECT text IS NULL, issued_box FROM run_command WHERE id = x` → `(true, B's box)`; `SELECT count(*) FROM run_command WHERE kind = 'follow_up' AND text IS NOT NULL AND status <> 'pending'` → 0 (PRD metric 3); B wrote no `session_event` (I-1). |
| `a_close_waits_for_an_enqueue_holding_the_window_and_refuses_it` | B-16, enqueue-first: a raw `db.pool.begin()` runs `SELECT 1 FROM follow_up_window WHERE run_step_id = $1 FOR SHARE` and inserts a pending follow-up row by hand; A's `close_follow_ups` spawned; `timeout(300 ms)` on it elapses (blocked); raw tx commits; close → `1`; the row `refused`/`FOLLOW_UP_SESSION_ENDED`, `text IS NULL`. Pins the two-statement close (a CTE answers 0 and leaves it pending). |
| `an_enqueue_behind_a_close_reads_the_window_closed` | B-16, close-first: raw tx runs `UPDATE follow_up_window SET closed_at = clock_timestamp() WHERE run_step_id = $1` and holds; B's `request_follow_up` spawned, blocked (timeout elapses); commit; B → `Refused(SessionEnded)`; no row. Pins `FOR SHARE OF w` (without it B is `Queued` at once). |
| `an_enqueue_without_a_live_executor_is_refused_executor_gone` | Zero-TTL `refresh_lease(a)` → B `ExecutorGone`; `take_lease` by owner `x` → still `ExecutorGone` (B-4; **[DV-2 declined: drop this half]**); no row. |
| `the_check_keeps_text_only_while_pending` | Raw SQL: `UPDATE … SET status = 'applied', resolved_at = clock_timestamp()` on a pending follow-up keeping text → `23514`; a `follow_up` insert without `run_step_id` → `23514`; a `cancel` with text → `23514`; `resolve_command` on a cancel still works (text NULL). |
| `two_concurrent_follow_ups_on_one_step_queue_one` | `join!` of two enqueues, 20 rounds: one `Queued`, one `AlreadyQueued`; one pending row per step. |
| `follow_up_times_are_the_databases` | `SELECT clock_timestamp()` before and after open/enqueue/settle/close: `opened_at`, `issued_at`, `resolved_at`, `closed_at` between them (I-4). |
| `deleting_a_project_holding_follow_ups_succeeds` | A window and a pending follow-up; `delete_project(PROJECT_HTUI)` → `Ok`; `common::count` of `follow_up_window` and `run_command` → 0. |
| `an_unknown_actor_on_a_follow_up_is_refused_before_the_status` | MOD-42 A-1's shape: an unknown user on a non-running step → `Constraint`, not `NotRunning`. |
| `migration_0016_reshapes_run_command_and_adds_the_follow_up_window` (`migrations.rs`) | `pg_indexes` has `uq_run_command_pending_cancel`, `uq_run_command_pending_follow_up`, `idx_follow_up_window_open`, not `uq_run_command_pending`; `run_command` has `run_step_id`, `text`; `follow_up_window` has its six columns; `chk_run_step_status`'s definition equals the 0015 one. |

Existing `two_concurrent_cancel_requests_insert_one` (`:5843`) and
`deleting_a_project_holding_relay_rows_succeeds` (`:5965`) stay green unchanged (the reshaped target).
`pg_conformance` green including T0's nine.

### 4.4 Gate

G-T1. Every Postgres command under `flock /tmp/mod70-pg.lock`.

---

## 5. T2 — the follow-up loop in `drive` (parallel with T1 and T5)

### 5.1 Files

`htui-agent/src/record/relay.rs` (loop, helpers, `drive`'s doc), `htui-agent/tests/relay.rs`
(doubles, cases). Nothing else: `drive`'s signature, `pump` and `turn`'s body are unchanged.

### 5.2 `drive`'s body (names are binding, layout is not)

`drive` (`relay.rs:200-221`) obtains everything from the `Relay` it is handed: `relay.run`,
`relay.step` (`step.id` in `drive_once`), `relay.session` (the `RelaySessionId::new()` `drive_once`
mints per call, `engine.rs:6121`; each judge call and each retry gets its own), `relay.owner`,
`relay.now`, `relay.poll`. New imports: `htui_core::model::{FollowUpSettle, QueuedFollowUp,
SettleOutcome, FOLLOW_UP_SESSION_CANCELLED, FOLLOW_UP_SESSION_ENDED, executor_scrub_refusal}`,
`htui_core::scrub::Unmasked`, `crate::event::StopReason`.

```rust
/// MOD-70 D6, B-10: this session's follow-up window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Window {
    /// No relay, or `follow_ups: false`: no follow-up store call at all (I-8).
    Off,
    /// `open_follow_ups` answered `true`: checked at every turn end, closed with retries.
    Open,
    /// `open_follow_ups` failed (not fenced): never checked, one close attempted (B-10).
    Unknown,
}

pub async fn drive<S: RecorderStore, R: RelayStore>(session, recorder, relay, control)
    -> Result<DoneEvent, DriverError>
{
    let mut opened = false;
    // D6 step 1: before the first pull.
    let window = match relay {
        Some(relay) if relay.follow_ups => open_window(relay).await?,   // Err only: Fenced
        _ => Window::Off,
    };
    // D6 steps 2-7.
    let out = turns(session, recorder, relay, control, &mut opened, window).await;
    // B-15 (MOD-42): unchanged.
    if let Some(relay) = relay && opened && settles_stale(&out) && let Err(err) = … { warn }
    // D6 step 8: every exit but a fence (MOD-40 D1), after the session ended, before drive_once's
    // recorder.finish, verify, capture, finish_step and gate::apply (I-6).
    if let Some(relay) = relay && window != Window::Off && !is_fenced(&out) {
        let reason = match &out {
            Err(DriverError::Cancelled) => FOLLOW_UP_SESSION_CANCELLED, // a deadline cut too (D6 step 8)
            _ => FOLLOW_UP_SESSION_ENDED,
        };
        close_window(relay, window, reason).await;     // best-effort; never changes `out`
    }
    out
}

async fn open_window<R: RelayStore>(relay: &Relay<'_, R>) -> Result<Window, DriverError> {
    match RelayStore::open_follow_ups(relay.store, relay.run, relay.step, relay.session, relay.owner).await {
        Ok(true) => Ok(Window::Open),
        Ok(false) | Err(StoreError::Fenced { .. }) =>
            Err(StoreError::Fenced { step: relay.step }.into()),
        Err(err) => { tracing::warn!(%err, "the step's follow-up window did not open; \
                       this session takes no follow-up"); Ok(Window::Unknown) }
    }
}

/// The turns of one session: D6 steps 2-7.
async fn turns<S, R>(session, recorder, relay, control, opened: &mut bool, window: Window)
    -> Result<DoneEvent, DriverError>
{
    loop {
        // B-20: boxed per turn, so the loop adds no `turn`-sized slot to `drive`.
        let done = Box::pin(turn(session, recorder, relay, control, opened)).await?;
        let Some(relay) = relay.filter(|_| window == Window::Open) else { return Ok(done) };
        let Some(queued) = take_follow_up(recorder, relay, control, &done).await? else {
            return Ok(done);
        };
        // D6 step 6 (I-1, PRD Q1): `turn + 1`, scrubbed with the same scrubber the pre-scrub used,
        // so `record_follow_up`'s `refuse` path is unreachable here.
        recorder.record_follow_up(&queued.text, (relay.now)()).await?;
        // D6 step 7: an error ends the session as a failed turn would.
        session.send_follow_up(queued.text).await?;
    }
}

/// D6 steps 3-5 at one turn end: the row claimed for the next turn, or `None` to end the session.
async fn take_follow_up<S, R>(recorder: &Recorder<'_, S>, relay: &Relay<'_, R>,
                              control: &mut Control, done: &DoneEvent)
    -> Result<Option<QueuedFollowUp>, DriverError>
{
    loop {
        // D6 step 3, D7: a breach, a cut or a cancel applies nothing; the close refuses the row.
        if done.stop_reason == StopReason::Cancelled
            || recorder.cap_breach().is_some()
            || control.signal().is_cancel()
        {
            return Ok(None);
        }
        let Some(queued) = next_follow_up(relay, control).await else { return Ok(None) }; // B-11
        // D6 step 4: the executor's scrubber, the payload `record_follow_up` scrubs.
        let to = match prescrub(recorder.scrubber, &queued.text) {
            Ok(()) if control.signal().is_cancel() => return Ok(None),         // B-12
            Ok(()) => FollowUpSettle::Applied,
            Err(unmasked) => FollowUpSettle::Refused(executor_scrub_refusal(unmasked.rule)),
        };
        let applying = to == FollowUpSettle::Applied;
        // D6 step 5.
        match RelayStore::settle_follow_up(relay.store, queued.id, relay.owner, to).await {
            Ok(SettleOutcome::Settled) if applying => return Ok(Some(queued)),
            // Refused here (executor scrub), or by a cancel or a newer window: look again.
            Ok(SettleOutcome::Settled | SettleOutcome::NotPending) => {}
            Ok(SettleOutcome::Fenced) | Err(StoreError::Fenced { .. }) =>
                return Err(StoreError::Fenced { step: relay.step }.into()),
            Err(err) => { tracing::warn!(%err, "a follow-up was not claimed; the session ends \
                           and its window's close refuses it"); return Ok(None); }       // B-11
        }
    }
}

/// B-11: `next_follow_up` riding out `TRANSIENT_READS` consecutive transient failures at
/// `relay.poll`, selected against the control; `None` on a cancel, on exhaustion or on any other
/// error (each a `warn`).
async fn next_follow_up<R>(relay: &Relay<'_, R>, control: &mut Control) -> Option<QueuedFollowUp>;

/// D6 step 4: `{"text": text}` through `scrubber` — `record_follow_up`'s payload (`record.rs:780`).
fn prescrub(scrubber: &dyn Scrubber, text: &str) -> Result<(), Unmasked>;

/// D6 step 8, B-10: `close_follow_ups` with `reason`; `Open` retries `Unreachable`/`Backend` up to
/// `TRANSIENT_READS` times at `relay.poll`, `Unknown` tries once. A final failure is a `warn`
/// ("…; a lease re-take or a cancel refuses what it holds", R-3).
async fn close_window<R>(relay: &Relay<'_, R>, window: Window, reason: &str);

/// `Err(Store(Fenced))`: the one exit that writes nothing more (MOD-40 D1).
const fn is_fenced(out: &Result<DoneEvent, DriverError>) -> bool;
```

Where this sits against the real code: `drive` keeps its two post-steps in the order shown (the
MOD-42 B-15 settle first, then the close; neither depends on the other). `turn` (`:233-306`) is
untouched — `Ok(done)` there is still "the turn's `done`, already recorded" (`:269-273`), including
`enforce_breach`'s `Ok(Done { Cancelled })`, which `take_follow_up` turns into "no follow-up". The
parked poll (`park`, `:309-445`) is unchanged and runs inside any turn, follow-up turns included;
`opened` accumulates across turns, so B-15's staling covers every turn's rows. `cancel()`
(`:448-529`) is unchanged: a cancel during a follow-up turn leaves through it as `Err(Cancelled)`,
and `drive` then closes with `FOLLOW_UP_SESSION_CANCELLED`. `recorder.scrubber` (`record.rs:386`,
private) is visible here as `scrubbed()` already shows (`:348`).

`drive`'s doc (`:171-199`) gains a paragraph: "**Follow-ups (MOD-70 D6).** With `relay.follow_ups`,
the step's window opens before the first pull (`false` is a fence) and, at each turn's `done` that
is not a cancel or a breach, the step's pending follow-up is pre-scrubbed, claimed under the lease,
recorded at `turn + 1` and sent; the next turn is driven the same way. Every exit but a fence closes
the window, refusing what is still queued. Without it, `drive` makes no follow-up call (I-8)."

### 5.3 Commits

1. **red** — §5.4's doubles and cases. Red: with no loop, `follow_ups: true` opens nothing, so every
   enqueue answers `NotStarted` (the cases `expect` `Queued`).
2. **green** — §5.2, the doc.

### 5.4 Tests (written first, `htui-agent/tests/relay.rs`, `#[tokio::test(start_paused = true)]`)

New fixtures: `running(fx)` (moves `fx.step` `Pending → Running`, which `leased_at` leaves
`pending`); `relay_with_follow_ups(fx, policy)` (`relay` with `follow_ups: true`); `enqueue(store,
step, text) -> FollowUpRequest`; `two_turns(turn1)` = `Script::turns(vec![answered_script's turn 0,
turn1])` where turn 0 parks (`call` + `park("Allow")` + `finished()`); `follow_ups_in(log)`
(`EventKind::FollowUp` rows with `(turn, payload)`). Doubles (B-9):

- `CancelAtDone { inner: Box<dyn AgentSession>, sender: watch::Sender<Signal>, grace }` — forwards
  everything; when `next_event` hands out a `Done`, it first sends `Signal::Cancel { grace }`.
- `Hooked { store: MemStore, calls: [AtomicUsize; 5], on_next: Mutex<Option<(When, Hook)>>, open_fails: bool }`
  with `type Hook = Box<dyn FnOnce(MemStore) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>`
  and `enum When { BeforeRead, AfterRead }` — a `RelayStore` forwarding to MemStore (UFCS
  `WriteStore::…`), counting the five follow-up methods, awaiting `on_next` once inside the first
  `next_follow_up` — `BeforeRead`: hook, then the read (the caller sees what the hook wrote);
  `AfterRead`: the read, then the hook, then the read's answer (the caller misses what the hook
  wrote) — and answering `Unreachable` from `open_follow_ups` when `open_fails`.

The client for happy paths is MOD-42's: `parked_row(&fx.store)` → `enqueue(…)` (must be `Queued`) →
`answer(&fx.store, &row, ALLOW)`, joined with `drive` under `within`.

| Test | Asserts |
|---|---|
| `a_follow_up_queued_during_the_turn_is_sent_at_its_end` | Text `"also run {SECRET} checks"`. `out == Ok(Done { EndTurn })` (turn 1's); `follow_ups_in(log) == [(1, {"text": "also run [REDACTED] checks"})]` (executor-masked, R-HIS-1); turn 1's events after it at `turn = 1`; `follow_up_rows()` = `[(Applied, false)]`; `follow_up_windows()[0].closed_at.is_some()`. |
| `two_follow_ups_run_two_more_turns` | Turn 1 parks again; the client queues a second follow-up during that park. Two `follow_up` rows at turns 1 and 2; three `done`s; both rows applied (PRD Q1 "repeating until none is pending"). |
| `without_a_follow_up_the_session_ends_at_its_first_done` | `Hooked`: one `open`, one `next`, zero `settle`, one `close`; one turn; window closed; the session's log is row-for-row the log of the same script driven with `follow_ups: false` (a window writes no `session_event`). |
| `a_follow_up_queued_after_the_last_check_is_refused_by_the_close` | B-15: `Hooked.on_next = (AfterRead, enqueue through WriteStore::request_follow_up)`, so the check answers `None` and the row lands after it. `out == Ok(..)` after one turn; the row `(Refused, false)` with `FOLLOW_UP_SESSION_ENDED`; no `follow_up` event. |
| `a_cap_breach_applies_no_follow_up` | A recorder with a run cap a scripted `usage` crosses in turn 0 (the `enforce_breach` path, `record.rs:2086-2106`); the row queued while parked is `(Refused, false)` with `FOLLOW_UP_SESSION_ENDED`; `out` is `Done { Cancelled }`; no `follow_up` event. |
| `a_cancelled_stop_applies_no_follow_up` | Turn 0's scripted `done` is `StopReason::Cancelled`; same assertions. |
| `a_cancel_signalled_at_the_turn_end_applies_no_follow_up` | `CancelAtDone` around the fake: `out == Ok(Done { EndTurn })` (turn 0's), the row refused `FOLLOW_UP_SESSION_ENDED`, no `follow_up` event, no second turn. |
| `a_cancel_during_a_follow_up_turn_closes_with_the_cancelled_sentence` | Turn 1 = `[ExpectCancel]`; after the first follow-up is applied the client queues a second, then sends `Signal::Cancel`. `out == Err(Cancelled)`; second row refused `FOLLOW_UP_SESSION_CANCELLED`; window closed. |
| `a_fenced_claim_ends_the_walk_and_closes_nothing` | `Hooked.on_next = (BeforeRead, …)`: enqueue, then zero-TTL `refresh_lease(owner)` and `take_lease` by a stranger. `out == Err(Store(Fenced { step }))`; row `(Pending, true)`; window open (`closed_at` `None`): a fenced writer writes nothing more. |
| `drive_without_follow_ups_makes_no_follow_up_call` | `Hooked` with `follow_ups: false` over `answered_script()`: all five counters `0`; the log equals today's. (`pump`: by construction, pinned by the existing `drive_without_a_relay_is_pump`, `:1490`.) |
| `an_executor_scrub_refusal_refuses_the_row_with_its_rule` | Text `format!("{SECRET}{RESIDUE}")`: `FollowUpText::new` accepts it (`sk-ant-…` follows `d`, no token start), the executor masks `SECRET` → `[REDACTED]sk-ant-…` (a token start) → `Unmasked(anthropic_api_key)`. Row `(Refused, false)` with `executor_scrub_refusal("anthropic_api_key")`; no `follow_up` and **no `scrub_residue`** row; `out == Ok(Done { EndTurn })` after one turn; `recorder.finish()` is `Ok` (the step is not failed by a refused follow-up). |
| `a_failed_open_runs_the_session_without_a_window` | `Hooked { open_fails: true }`: `out == Ok(..)`; an enqueue during the park → `NotStarted`; one close attempted; zero `next`. |
| `a_fenced_open_drives_nothing` | The lease taken by a stranger before `drive`: `out == Err(Store(Fenced { step }))`; the log has no row past what the recorder held before; no window. |

Every existing case in the file passes unchanged (`follow_ups: false` everywhere T0 set it).

### 5.5 Gate

G-T2.

---

## 6. T5 — Runs-pane follow-up (M2; parallel with T1 and T2, in a worktree)

### 6.1 Lane

`git worktree add -b hr/MOD-70-t5 /home/mluigi/projects/htui-wt/mod-70-t5 <T0 green sha>` (B-17).
Native tools only in the worktree (Gortex indexes and edits the primary checkout).

### 6.2 Files

`htui/src/store_worker.rs` (§2.5, test), `htui/src/ui/tabs/backlog/detail/runs.rs` (§6.3),
`htui/tests/backlog.rs`, `htui/tests/snapshots/` (two new files). `ui/tabs/backlog/mod.rs` is **not**
touched (OQ-1: `i`).

### 6.3 `RunsTab` (`runs.rs`)

- Module doc key table (`:16-30`): `| \`i\` | running engine step | a typed follow-up (\`FollowUp\`, MOD-70 D14) |`;
  the MOD-42 paragraph (`:32-35`) gains "A step's newest follow-up takes one line (queued, sent) or
  up to three (refused: the label and its reason)".
- `Mode::FollowUp { step: StepId, field: TextField }` after `RejectNote` (`:277-285`), doc "`i`: the
  follow-up typed for a running step's session (MOD-70 D14)" (B-8).
- `on_key` letter pattern (`:1468-1470`) gains `'i'` (so the end-gesture-on-capture path runs).
- `action` (`:645`): after `let Some(item) = self.item else { return Handled::Pass }` and **before**
  the `NOT_LOADED` early return (`:649-652`):
  `if key == 'i' { self.open_follow_up(ctx); return Handled::Consumed; }`.
- `fn open_follow_up(&mut self, ctx: &Ctx<'_>)`: `entry_step()` → `None` → `NO_STEP`; the run's
  `kind == RunKind::Chat` → `FOLLOW_UP_CHAT_RUN`; `step.fanout_index < 0` → `FOLLOW_UP_JUDGE`;
  `step.status != StepStatus::Running` → `FOLLOW_UP_NOT_RUNNING` (each `ctx.emit(Action::Error(..))`,
  nothing sent; the store re-checks every guard); else `self.mode = Mode::FollowUp { step, field:
  TextField::new() }`.
- `modal_key` (`:579-623`): `Mode::FollowUp { step, field } => follow_up_key(step, field, key, ctx)`.
- `fn follow_up_key(step, mut field, key, ctx) -> Mode` beside `reject_key` (`:836-863`):
  `Submit` → `match FollowUpText::new(field.text().unwrap_or_default().to_owned())`: `Ok(text)` →
  `ctx.request(StoreRequest::FollowUp { step, text })`, `Mode::Browse`; `Err(why)` →
  `ctx.emit(Action::Error(why.to_string()))`, stay (`Empty` and `Residue` alike, so the text can be
  edited); `Cancel` → `Browse`; else stay.
- `on_paste` (`:1490-1498`): `Mode::FollowUp { field, .. }` joins the field arm.
- `on_reply` (`:1588-1646`): `StoreReply::FollowUpQueued { .. } => self.re_read(ctx)`;
  `StoreReply::Failed { request, .. } if *request == FOLLOW_UP => self.re_read(ctx)`.
- `fn follow_up_on(&self, step: StepId) -> Option<&FollowUpView>` beside `pending_on`.
- `fn follow_up_lines(view: &FollowUpView, theme: &Theme) -> Vec<Line<'static>>` beside
  `permission_lines` (`:1272-1289`), B-7: each line `blank(FOLLOW_UP_INDENT)` +
  `cells::fit(text, PANE - FOLLOW_UP_INDENT)` (exactly `PANE` wide): `Pending` →
  `[FOLLOW_UP_QUEUED]` (`theme.warning`); `Applied` → `[FOLLOW_UP_SENT]` (`theme.dim`); `Refused` →
  `[FOLLOW_UP_REFUSED]` then `cells::wrap(resolution, PANE - FOLLOW_UP_INDENT).take(2)` (`theme.dim`).
- `list_lines` (`:1739-1777`) and `flow_head` (`:1720-1735`): after `permission_lines`,
  `if let Some(view) = self.follow_up_on(step.id) { lines.extend(follow_up_lines(view, theme)); }`
  (the cursor's end counts them, as the request's).
- `footer` (`:1780-1800`): `Mode::FollowUp { field, .. } => vec![Line::styled("follow-up for the
  running step:", theme.title), field.line(width, true, theme), hint("Enter send · Esc cancel")]`.

### 6.4 Commits

1. **red** — §2.5 (variant, `name()` arm, `try_serve` arm, reply), `Mode::FollowUp` with
   `modal_key`/`footer`/`captures_input` arms but `i` unbound, the tests. Red: `i` passes.
2. **green** — `i`, `open_follow_up`, `follow_up_key`, lines, replies, snapshots accepted.

### 6.5 Tests (written first)

`runs.rs` `mod tests` (the `driven()` fixture `:2943` has only `done` steps, so each case first sets
the cursor step's `status = Running`, `fanout_index = 0` on the pane's runs):

| Test | Asserts |
|---|---|
| `i_on_a_running_main_step_opens_the_follow_up_input` | Mode `FollowUp` for the cursor step; `captures_input()`; nothing sent. |
| `i_says_why_on_a_judge_a_parked_or_a_done_step` | `fanout_index = -1` → `FOLLOW_UP_JUDGE`; `AwaitingApproval` → `FOLLOW_UP_NOT_RUNNING`; `Done` → same; a chat run → `FOLLOW_UP_CHAT_RUN`; no request sent, mode `Browse`. Also with no `RunActions` loaded (no `NOT_LOADED`). |
| `enter_sends_a_follow_up_and_returns_to_browse` | Typed `"use the smaller fixture"`: one `StoreRequest::FollowUp { step, text }` with `text.as_str()` equal; mode `Browse`. |
| `an_empty_follow_up_stays_open_with_its_sentence` | `Enter` on `"   "` → status `FOLLOW_UP_EMPTY`, still `FollowUp`, nothing sent. |
| `a_credential_shaped_follow_up_stays_open_and_sends_nothing` | `sk-ant-api03-…`: status names `anthropic_api_key` and not the key; still `FollowUp`; nothing sent. |
| `esc_leaves_the_follow_up_input` / `a_paste_reaches_the_follow_up_field` | As the `RejectNote` siblings. |
| `a_follow_up_reply_re_reads_the_runs` | `FollowUpQueued` and `Failed { request: "follow_up" }` each request `Runs` (and the relay view). |
| `a_step_with_a_follow_up_takes_its_lines_each_forty_three_wide` | Pending: 1 more line reading `FOLLOW_UP_QUEUED` whole (B-7); applied: 1; refused with `FOLLOW_UP_SESSION_ENDED`: 3; every line exactly 43 cells (mirrors `:5448`). |
| `every_follow_up_resolution_fits_two_lines` | Each D12 resolution (`SESSION_ENDED`, `SESSION_CANCELLED`, `RUN_CANCELLED`, `executor_scrub_refusal` of the longest rule name in `scrub.rs`'s table) wraps to ≤ 2 lines at 41. |
| `every_step_takes_two_lines` (`:2187`) | Unchanged, green. |

`store_worker.rs` tests: `relay_reads_are_empty_offline_and_answers_are_refused` (`:5148-5182`) gains
`StoreRequest::FollowUp { step, text }`: `name() == "follow_up"`, offline →
`Err(Unreachable(DATABASE_UNREACHABLE))`.

`tests/backlog.rs` (MemStore `Harness::over`; seed = `relayed_store`'s claim of `RUN_2` by `owner`,
then `transition_step(STEP_R2_PRD, Pending, Running)`, `open_follow_ups(RUN_2, STEP_R2_PRD,
RelaySessionId::new(), owner)`; the default cursor is on `STEP_R2_PRD`, as
`a_digit_on_the_runs_pane_answers_through_the_store` shows):

| Test | Asserts |
|---|---|
| `offline_the_runs_pane_asks_for_no_error` (`~:1985-2032`, extended) | A dispatched `StoreRequest::FollowUp`: status starts `follow_up: ` and ends with `DATABASE_UNREACHABLE`. |
| `i_on_the_runs_pane_queues_a_follow_up_through_the_store` | `key("i")`, `paste("use the smaller fixture")`, `key("Enter")`, `drive()`: `store.follow_up_rows() == [(Pending view of STEP_R2_PRD, true)]`; status `None`; the frame shows `FOLLOW_UP_QUEUED`. |
| `a_refused_follow_up_lands_on_the_status_line` | The window closed before `Enter` (`close_follow_ups`): status `"follow_up: " + FOLLOW_UP_SESSION_ENDED`; no row. |
| snapshot `runs_follow_up_input` | After `i` and the paste: the footer with title, field and hint. |
| snapshot `runs_follow_up_queued` | A row queued through the store before the harness starts: the step and its queued line. |

### 6.6 Gate

G-T5 (worktree).

---

## 7. T3 — engine (serial, after the wave merge)

### 7.1 Files

`htui-orch/src/engine.rs` only. (A `ScriptedStep::parks_then(request, turn1, body)` helper in
`htui-orch/src/fake.rs` is allowed if the literal `ScriptedStep { script: Script::turns(…), output,
spawn_failure: None }` — its fields are `pub`, `fake.rs:1082-1091` — reads worse.)

### 7.2 Code

- **D8** (`drive_once`'s literal, `:6116-6126`): `follow_ups: step.fanout_index >= 0,` with the
  comment "MOD-70 D8: the main step's and each candidate's session take follow-ups; a judge call
  (`fanout_index = -1`, `judge_calls` `:5138`) never opens a window (PRD Q2)."
- **D9** (`stale_dropped_requests`, `:2142`), first statement of the body:

```rust
        // MOD-70 D9: the dropped walk's follow-up windows close first, under the same lease: a
        // follow-up queued for a session nobody drives is refused, not stranded (OQ-5).
        if let Err(err) = self
            .parts
            .store
            .close_dropped_follow_ups(run, self.parts.owner, FOLLOW_UP_SESSION_ENDED)
            .await
        {
            tracing::warn!(%run, %err, "closing a dropped walk's follow-up windows failed; a cancel or the next re-take refuses what they hold");
        }
```

  Its doc (`:2131-2141`) gains the sentence. The three call sites (`:2061` via
  `stale_dropped_requests_of`, `:2122` the same, `:2347` directly) are unchanged; the sweep reaching it
  twice (`:2329`, `:2347`) is the idempotence T0 pinned. `self.parts.store` is `S: WorkerStore`;
  `close_dropped_follow_ups` resolves through the `RelayStore` supertrait, as `settle_permissions`
  does two lines below.

### 7.3 Commits

1. **red** — §7.4's tests. Red: `follow_ups: false` opens no window, so the client's enqueue answers
   `NotStarted`; the D9 cases find the row pending.
2. **green** — D8, D9, docs.

### 7.4 Tests (written first, `engine.rs` `mod tests::relay` `:16199`, `#[tokio::test(start_paused = true)]`)

Helpers: `parks_then(turn1: Vec<ScriptEvent>, body) -> ScriptedStep` (turn 0 = `parks`' events,
turn 1 = `turn1`); `following_client(store, item, text)` = `answering_client` (`:16297`) that calls
`request_follow_up` for the parked row's step (`Queued` expected) **before** answering (B-9).

| Test | Asserts |
|---|---|
| `a_follow_up_queued_while_parked_runs_a_second_turn_before_the_gate` | FEAT-3, `prd` ungated (`feat_3_with_prd_ungated`), `prd` = `parks_then([text, Done{EndTurn}])`. Walk → `Started`, rest at `plan`'s gate; `prd` `done`; its log has one `follow_up` at `turn = 1` and two `done`s; the row `(Applied, false)`; the window closed. |
| `a_fan_out_candidate_takes_a_follow_up` | ANA-2 `research` fanned to 2 (the `ana_2_judged_with_a_parked_call` setup, judge call 0 **not** parked), candidate 0 = `parks_then`; the follow-up lands on candidate 0's log only; the judge still picks a winner. |
| `a_judge_step_opens_no_window` | `ana_2_judged_with_a_parked_call`: while the judge call parks, `request_follow_up` on the judge step → `Refused(Judge)`; `follow_up_windows()` has no entry for it after the walk. |
| `a_follow_up_turn_cut_by_the_deadline_settles_deadline_elapsed` | `step_deadline_seconds` small (as `a_hung_session_is_cut_at_the_step_deadline_and_settles_deadline_elapsed`, `~:17000-17080`), turn 1 = `[ExpectCancel]`: the step settles `DeadlineElapsed`; the follow-up was applied (turn 1 ran); a second row queued during turn 1 is refused `FOLLOW_UP_SESSION_CANCELLED` (D6 step 8: the cut closes with the cancelled sentence). |
| `a_cancel_during_a_follow_up_turn_finishes_no_step` | Turn 1 = `[ExpectCancel]`; once the first row is applied, `orch.cancel_walks(GRACE)`: walk → `EngineError::Cancelled`; `prd` not finished (I-6); a second queued row refused `FOLLOW_UP_SESSION_CANCELLED`. |
| `a_dead_walks_follow_up_is_refused_when_its_sweep_readopts_the_run` | B-18: `dropped_with_a_follow_up` = `dropped_while_parked` (`:17619`) with a follow-up queued before the drop; `dead_walks.mark`; `engine.sweep()` → the window closed, the row refused `FOLLOW_UP_SESSION_ENDED`, `text` gone; `assert_ghost_staled` still holds. |
| `a_dead_walks_follow_up_is_refused_when_a_command_retakes_the_lease` | Same, through `engine.take_lease`. |
| `an_abandoned_walks_follow_up_is_refused_before_the_lease_goes_back` | Same, through `engine.abandoned`; then `take_lease` succeeds. |
| `a_dispatch_future_is_send` (`:14375`) | Still compiles. |
| `every_case_name_dispatches` (`conformance.rs:7794`) | Passes (no SIGABRT). |

### 7.5 Gate

G-T3.

---

## 8. T4 — end to end over Postgres (serial, after T1 and T3)

### 8.1 Files

`htui/tests/worker_pg.rs`, `htui/tests/runs_pg.rs`.

### 8.2 Tests

Mirror `worker_pg.rs:845` (`a_worker_parked_step_resumes_on_an_answer_from_another_box`) and
`runs_pg.rs:1599` (`an_in_process_walk_resumes_on_an_answer_from_another_box`, helpers `another_box`
`:1562`, `pending_request` `:1578`): the step's fake script parks turn 0 and has a turn 1; box B
enqueues while parked, then answers.

| Test | Asserts |
|---|---|
| `a_worker_walked_step_takes_a_follow_up_from_another_box` (`worker_pg`) | A worker on box A walks; B's `request_follow_up` → `Queued`; the step completes `done`; its log has a scrubbed `follow_up` at `turn = 1`; `SELECT status, text IS NULL, issued_box FROM run_command WHERE kind = 'follow_up'` → `('applied', true, B)`. |
| `an_in_process_walk_takes_a_follow_up_from_another_box` (`runs_pg`) | The same through the TUI's in-process runtime. |
| `a_cancel_from_another_box_refuses_its_pending_follow_up` (`worker_pg`) | B enqueues while parked, then `request_cancel` (`WorkerStore`/`WriteStore`): the row `('refused', true)` with `FOLLOW_UP_RUN_CANCELLED`; the run `cancelled`; no `follow_up` event. |
| `no_follow_up_is_left_pending_after_the_step_ends` (`worker_pg`, B-15) | After the walk's step is `done`: zero `pending` follow-up rows; a further enqueue → `Refused(NotRunning)`; zero windows with `closed_at IS NULL` for the step. |

### 8.3 Gate

G-T4.

---

## 9. T6 — docs (serial, last)

As the plan's T6, plus: `docs/htui-worker.md`'s section says the pane updates when the session ends
(F-23), that a failed close keeps a row pending until a cancel or a re-take (F-25, R-3), that the
deadline and the run cap include follow-up turns (OQ-3), migrate `0016` from a TUI first, and
**upgrade every box together** (R-8). `README.md` Runs key table (`:204-221`): `| \`i\` | running
step: a follow-up sent when its current turn ends |`. ANA-2 §4.8 amendment after `:1269-1275`.
Gate: G-Final.

---

## 10. Wave schedule and lane rules

| Wave | Tasks | Where | Postgres |
|---|---|---|---|
| W0 | T0 | primary | none (`htui-store` runs with the DSN unset) |
| **W1** | **T1 ∥ T2 ∥ T5** | T1, T2 primary; T5 worktree `hr/MOD-70-t5` | T1 only |
| W1-merge | merge T5; G-W1 | primary, no lane running | yes, under the lock |
| W2 | T3 | primary | none in its gate |
| W3 | T4 | primary | `htui` Postgres suites under the lock |
| W4 | T6 | primary | full gate |

1. **Branch first.** T5: `git worktree add -b hr/MOD-70-t5 /home/mluigi/projects/htui-wt/mod-70-t5
   <T0 green sha>`. T1 and T2 commit on `hr/MOD-70`.
2. **Crate-scoped commands only during W1.** T1: `-p htui-store` and `cargo sqlx` inside
   `crates/htui-store`; T2: `-p htui-agent`; T5: `-p htui`. No `--workspace`, no `cargo fmt --all`,
   no workspace clippy until G-W1; `cargo fmt -p <crate> -- --check` instead.
3. **No lane edits `htui-core` in W1.** A §2 shape found wrong is reported; the fix waits for the
   merge.
4. **One Postgres lane.** Every command with `HTUI_TEST_DATABASE_URL` set and every `cargo sqlx`
   command runs under `flock /tmp/mod70-pg.lock`. T2 and T5 run with `env -u HTUI_TEST_DATABASE_URL`.
   `.sqlx` is T1's alone (its prepare compiles `htui-store` + `htui-core` only).
5. **`--all-features`** on every test command (`htui` without `testkit` runs 0 tests and says ok);
   `--test-threads=1` everywhere (process-wide keyring fake; scheduling-dependent suite).
6. **Reads and edits.** Primary lanes use Gortex (`read` with `offset`/`limit`; `edit`, or an anchored
   scripted replace where the hook blocks `Read`); the T5 worktree uses native tools.
7. **Commit incrementally**, red then green, on the lane's branch. Never stash.
8. **Merge.** After T1 and T2 committed and no build runs: `git merge --no-ff hr/MOD-70-t5`.
   Expected conflicts: none (disjoint files; T5's snapshots are new). Then G-W1 with no lane
   running; then `git worktree remove --force /home/mluigi/projects/htui-wt/mod-70-t5`, then `git
   branch -d hr/MOD-70-t5`.
9. **Coupling to expect at the merge**: nothing calls the loop until T3 flips D8, so G-W1 exercises
   T2 only through `htui-agent`'s own tests; T1's `0016` makes every Postgres harness migrate one more
   file and changes `request_cancel`'s statement (the `htui` cancel suites run in G-W1).
10. **After every lane and gate**: `pgrep -af 'htui worker'` prints nothing; no stray
    `~/.config/htui/trees/<run>`; `df -h .` before a Postgres-heavy gate (a crash loop is disk
    pressure first); re-run `qdrant_live` serially before calling a timeout a regression.

---

## 11. Pins

| Pin | Now | After | Where it moves |
|---|---|---|---|
| Store conformance `CASES` | 148 | 157 | T0 +9 (`mem_store.rs:37`, `pg_conformance.rs:31`, `:17-30`, `:38`) |
| Migrations | 15 | 16 | T1 (`migrations.rs`, `connect.rs`) |
| Postgres tables | 42 | 43 | T1 (`migrations.rs` `TABLES`) |
| `.sqlx` files | 348 | 348 − 3 changed texts (`request_cancel` insert, `pending_commands`, `resolve_command`) + the new distinct texts (counted, restated in T1's commit) | T1 |
| `StoreRequest` / `StoreReply` | 109 / 66 | 110 / 67 | T5 (not pinned by a test) |
| `crates/htui/tests/snapshots` | 147 | 149 | T5 |
| `RelayStore` own methods | 4 | 9 | T0 (`store/worker.rs` doc) |
| `WriteStore` methods added | — | 6 | T0 |
| Unchanged | | | `WorkerStore` 58, `EngineParts` fields, `pump`'s 26 call sites, `run_step.status` CHECK, `MIRRORED_TABLES`, `htui-orch` `CASES` |

Each moved pin's message names its reason ("MOD-70 T0's nine follow-up cases (plan D1-D5, D9)",
"MOD-70's 0016_follow_up.sql"). The implementer re-counts at each gate.

---

## 12. Gate reference

```bash
LOCK="flock /tmp/mod70-pg.lock"                    # anything touching Postgres
NOPG="env -u HTUI_TEST_DATABASE_URL"               # Postgres suites skip
pg()   { $LOCK cargo test -p "$1" --all-features -- --test-threads=1; }
nopg() { $NOPG cargo test -p "$1" --all-features -- --test-threads=1; }
lint() { cargo clippy -p "$1" --all-targets --all-features -- -D warnings; }
fmtp() { cargo fmt -p "$1" -- --check; }
# T1's scratch database (docs/hr-sandbox.md:194-210); migrate it again once 0016 exists:
#   $LOCK psql -h localhost -p 5439 -U postgres -c 'CREATE DATABASE htui_sqlx_mod70;'
SQLX_DB=postgres://postgres@localhost:5439/htui_sqlx_mod70
regen() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB sh -c \
  'cargo sqlx migrate run --source migrations && cargo sqlx prepare -- --all-targets --all-features'); }
check() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB \
  cargo sqlx prepare --check -- --all-targets --all-features); }
orch() { $NOPG cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \
  | tee /tmp/orch.log; ! grep -q SIGABRT /tmp/orch.log; }
```

| Gate | Commands |
|---|---|
| G-T0 | `nopg htui-core`; `nopg htui-store` (the `case_list_matches_mem_store` pin runs; Postgres cases skip); `nopg htui-agent`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo clippy --workspace -- -D warnings`; `cargo fmt --all -- --check`; `grep -rn "MOD-70 T0: red" crates` empty |
| G-T1 | `regen` then `check`; `pg htui-store` (`pg_conformance` incl. T0's nine, `pg_criteria`, `migrations`, `connect`); `lint htui-store`; `fmtp htui-store`; `grep -rn "MOD-70 T1: not yet implemented" crates` empty; `ls crates/htui-store/.sqlx \| wc -l` restated |
| G-T2 | `nopg htui-agent`; `lint htui-agent`; `fmtp htui-agent` |
| G-T5 (worktree) | `nopg htui`; `$NOPG cargo insta test -p htui --all-features -- --test-threads=1` (a full run: exactly the two new snapshots, nothing else pending); `lint htui`; `fmtp htui` |
| G-W1 (merged, no lane running) | `cargo fmt --all -- --check`; both workspace clippies; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `check`; `$LOCK cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 \| tee /tmp/w1.log; ! grep -q SIGABRT /tmp/w1.log` |
| G-T3 | `orch`; `nopg htui-worker`; `$NOPG cargo test -p htui --all-features run_worker -- --test-threads=1`; `lint htui-orch`; `cargo fmt --all -- --check` |
| G-T4 | `$LOCK cargo test -p htui --all-features --test worker_pg --test runs_pg -- --test-threads=1`; `lint htui` |
| G-Final | The plan's Validation list (`plan.md` "Validation"), `check`, a full `cargo insta test --workspace --all-features` with nothing pending (DSN unset), `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, the §10 rule 10 checks; the close-out restates §11 from a fresh count |
