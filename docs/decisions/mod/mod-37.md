# MOD-37 - Orchestrator hardening follow-ups (done, 2026-10-04)

**Requirements:** `R-ORCH-3`, `R-ORCH-5`, `R-ORCH-8`, `R-ORCH-9`, `R-TUI-4`, `R-HIS-1`, `R-NF-3`.
**Origin:** MOD-4 (`docs/decisions/mod/mod-4.md`, "Carried"): the orchestrator risks MOD-4 closed
with and that no other item owned. ANA-27 §5.1 added T4 (the agent-session deadline) and T5
("context not carried").
**Artifacts:**
- PRD [`.claude/prds/mod-37-orchestrator-hardening.prd.md`](../../../.claude/prds/mod-37-orchestrator-hardening.prd.md), five milestones;
- plans and blueprints, one pair per milestone, under `.claude/plans/`: `mod-37-run-state`,
  `mod-37-store-engine`, `mod-37-git-cost`, `mod-37-deadline-sessions`, `mod-37-acp-resume`
  (each plan carries its verified-claims table, and each blueprint its hazards).

Decision numbers are local to each milestone's plan and blueprint.

Routed as **PRD**. Every milestone was then planned on its own, in a TOOL-7 sandbox (`hr/MOD-37`).
Ultracode was used for implement and review in M1, for implement in M2, and not at all in M3-M5.

**Success metric (PRD):** every listed risk is closed or re-deferred with a stated reason, and each
closed risk has a regression test that fails on the old behaviour. Met. Four risks are re-deferred
with reasons, and **MOD-76** now owns them (see Carried).

---

## Milestones

### M1 - Run state and visibility (`19ca229`..`151c87e`, 2026-10-02)
Closed **R-3**: `RunStepSummary` carries `gate_note` from all three builders (Mem, Pg, cache
mirror), and the Runs pane shows a parked step's reason on a third line. Closed **R-40**:
`SessionSink::started` runs after each `pending → running` move, and `ProgressSink` publishes
`Changed`. Closed **R-41**: a `RunStream` `Error` frame puts its sentence on the status line even
when the reply was dropped as stale. Closed **R-51**: a command queued behind a live walk publishes
`Waiting`. Closed **T7's residual window**: the Chat tab sends `ChatFollow` as soon as `Promoted`
arrives, and the worker keeps a follow served before the bind. Re-deferred **R-44** (layout
decision) and **R-53** (mitigated by D184/D171). One LOW was left: the Chat tab's `followed` stays
set after a failed bind, which is harmless.

### M2 - Store and engine correctness (`1d1118f3`..`7df69d33`, 2026-10-02)
Closed **R-5** (the gate parks through one fenced `park_step`, and a skipped pass goes through
`pass_step`), **R-6** (`WriteStore::create_phase_agents`; `override_graph` copies phase agents),
**R-29** (MemStore truncates `queued_at` to the microsecond), **R-30** (`classify` adopts a
partly captured step) and the **R-31 remainder** (`status::resumable` covers a parked run over a
`failed` + `rejected` step). R-32 was half closed: D138's `part_way` keeps an `Io` error as `Io`.
Its D131 half is re-deferred. Accepted side effect: `Unblock` on a followed review-loop escalation
resumes it and escalates again, and a test pins this.

### M3 - Git cost (`7cf455a8`..`a387249c`, 2026-10-02)
Closed **R-37**: `reconcile_parent` opens the checkout once (6 opens before, 1 after; a test-only
counter pins it). Not changed: `RealIsolator`'s reconcile still opens the checkout once per
`blocking` hop under the admin lock.

### M4 - Deadline and sessions (`68a7c7e9`..`1787fd69`, 2026-10-03)
Closed the **ANA-27 T4 deadline**: `drive_once` runs each step session and fan-out candidate under
a timer for the rest of `deadline_seconds`, cancels gracefully through MOD-42's control, and settles
`DeadlineElapsed` through `SettleInput::deadline_cut`. Closed **R-49** by admission rather than a
guard (maintainer amendment): `claim_run`'s rule I already keeps other runs off a promoted
`shared_serialized` repo, across processes, and a conformance pin holds it. Three LOW windows are
left. Closed **R-46**: when the refresher reports the server gone, `RunRuntime::preempt_walks`
abandons every live walk at once. A TUI read failure does not preempt, because it can be a pool
timeout.

### M5 - ACP resume (`1b31282e`..`736edef8`, 2026-10-03/04)
Closed **R-48** and **ANA-27 T5**. A promoted ACP step now resumes its own agent session. A resume
that fails, on either transport, is reported and then opens the handoff prompt, labelled as such,
so a fresh context is never mistaken for a resumed one.

**Maintainer decisions (2026-10-03):**
- full ACP resume plus the T5 note, not the note alone;
- `session/resume` first, then `session/load` with the replay discarded;
- a failed resume is reported, then falls back to a labelled handoff in the same bind;
- the note lives in a new `run_step.opening` column;
- **A-7:** a promotion resumes the step's **latest** `session_started` banner, not the first. This
  amends MOD-4 D192: after a fallback, a re-promotion resumes the handoff session, which holds the
  chat;
- review: **M-1** best effort (below); **L-2** only `Transport` and `Closed` fall back; L-1, L-4,
  L-5 + N-1 and N-4 + L-6 applied; L-3, N-2 and N-3 recorded here.

**What was built:**
- **Store (T1).** `run_step.opening TEXT CHECK (opening IN ('resumed','handoff','resume_failed'))`,
  nullable (PG migration `0014`, cache migration `0005`, no column comment, H-2). `StepOpening` is a
  `str_enum!`. `WriteStore::record_opening` exists in MemStore (a side map cleared by
  `delete_project`), Postgres (one unfenced `UPDATE`) and the mirror's refresh and read.
  `RunStepSummary.opening` only, not `RunStep`. Conformance cases 135 → 136, migration pins
  13 → 14, `.sqlx` two renames and one new file.
- **ACP driver (T2).** `session_main` step 2 branches on `spec.resume`. With no resume it sends
  `session/new`, as before. Otherwise it sends `session/resume` when the agent advertises
  `sessionCapabilities.resume` and `settings.acp.session.resume` allows it. Failing that, it sends
  `session/load` when `loadSession` is advertised and allowed. Failing both, it refuses with a
  `Transport` error that names what is missing. Updates queued before the restore answer are
  drained, with one `Waker::noop()` poll per item, after both restore routes (review N-1). The SDK
  queues replay ahead of the ordered response. A failed restore kills and reaps its child before
  `start` answers: the arm leaves the error in `ReadyCell.owed`, and `run_session` reaps first
  (review L-4; `session/new` keeps D61's order). ACP `caps.resume` = `session.resume ||
  session.load`.
- **Engine (T3).** `promote::opening_kind(caps, events)` has no transport parameter and resumes
  either transport when `caps.resume` holds and a banner exists. `promote::banner` takes the latest
  banner (A-7). `OpeningPath::Resume { session_ref, text, fallback: Option<HandoffText> }`.
  `Engine::handoff_opening` (behind `Box::pin`, H-9) builds the handoff. It is strict for a
  `Handoff` opening and **best effort for a `Resume`** (review M-1): a handoff that cannot be built
  is logged, and the step resumes with no fallback, so a CLI resume that worked before M5 still
  does. `promote::CONTEXT_NOT_CARRIED` = "context not carried; handoff prompt only".
- **Worker (T4).** One recorder is built before the first `start` (H-6). On a `Resume` whose
  `start` fails with `Transport` or `Closed` (an exhaustive `match`, review L-2), and when a
  fallback exists, the worker does four things. It records an `other` row `resume_failed` as htui's
  own (`Recorder::record_notice`) with `{session_id, reason, note}`. It sends that row to the tab
  **as scrubbed** (review L-1). It writes `opening = resume_failed`. Then it starts again with no
  resume and the handoff text, recorded as the `follow_up`. Every other error fails the chat as
  before. Otherwise `opening` is `resumed` or `handoff`. A failed `record_opening` write is logged,
  never fatal (pinned, review N-4).
- **UI (T5).** The Runs pane shows two wrapped lines under a step whose `opening` is `handoff` or
  `resume_failed`, at both `note_line` sites (list and flow head), after R-3's reason. The Chat tab
  renders `resume_failed` as one transcript row (the reason, then the note). The live row flips the
  promoted header to `handoff`, and a replayed one does not. When both starts fail, the reason is
  shown above the refusal (review L-6).

**Commits:**
- plan, blueprint and A-7: `1b31282e`, `5c98f89b`, `59e1f323`, `57f61794`, `5689f232`, `c7fdb6f0`;
- T1 store: `354bc8bd` (red), `a3062708`, `4f8b974a`;
- T2 ACP driver: `d34b4130` (red), `bfd4d566`;
- T3 engine (worktree `hr/MOD-37-m5-t3`, merged `47657e5e`): `63881a77` (red), `178d3643`, `c8ae44be`;
- T4 worker: `1a3b2fdd`, `5fe8b0c1` (red), `1e6f7749`;
- T5 UI: `9dd0e815` (red), `b07d359f`, `8fa2b007`;
- review fixes: `cf39d015` + `736edef8` (M-1), `63c3a468` (L-2), `af397d87` (L-1), `604c4bcb`
  (L-5, N-1), `c6943fe0` (L-4), `7a2706e0` (N-4), `0c4aa966` (L-6).

**Gates (final tree):**
- fmt and `clippy --workspace --all-targets --all-features -D warnings` are clean;
- htui-agent: 556 passed;
- htui-orch: 581 passed, 0 SIGABRT;
- htui (`--test-threads=1`): 2173 passed, with the Postgres suites reached;
- htui-store: 312 passed;
- htui-core: 605 passed;
- `sqlx prepare --check` passes.

One load flake was seen once and passed on rerun: `htui-agent` `tests/auth.rs`
`a_stderr_line_resets_the_idle_clock`, which has a 400 ms idle window.

**Accepted and recorded (M5):**
- **H-5.** An agent that streams `session/load` replay *after* its response would leave duplicate
  rows: visible, never lost. `session/resume` is preferred.
- **H-8.** A failed handoff start, or a resume with no fallback, writes no `opening`.
- **H-12.** A restored session that offers no model options emits the existing `model_unavailable`
  row.
- **"Resumed" is a protocol claim.** It does not prove the model remembers: an agent that
  persisted nothing restores an empty context.
- **Review L-3.** A start failing for a reason unrelated to the resume (`initialize`, a handshake
  timeout) costs two handshakes, up to 2 × 60 s, and is labelled `resume_failed`.
- **Review N-2** (`run_chat` has grown; a `start_with_fallback` helper would read better) and
  **N-3** (a clone of the handoff text). Both left.

## Carried

**MOD-76** owns the risks re-deferred with reasons:
- **R-32**, the D131 half: a not-reset park loses its detail. Diagnostics only.
- **R-44**: step rows at 43 columns truncate `agent/model`. A layout decision, which waits for the
  Runs pane's next layout change.
- **R-53**: `ItemActions` is as of the last `Runs` reply. Mitigated by D184 and D171.
- **R-55**: `command_limits` is read once per process. "Whoever adds an editor".

Also left, in this write-up only:
- **R-49's three LOW windows** (M4): a same-run command between `Promoted` and the chat bind, a
  second promotion of the same step (D185), and an isolator rebuilt during the chat.
- **M1's LOW**: `followed` stays set after a failed bind.
- **M2's NITs**: `unblock_enabled`'s bare `bool`, and a `Vec<String>` copy in the `phase_agent`
  insert.
- **M3's NIT**: a hash parsed twice.
- **M4's NIT**: a `Debug`-string comparison in a pin test.
