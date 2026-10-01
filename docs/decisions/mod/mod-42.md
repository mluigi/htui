# MOD-42 - Permission and control relay through Postgres (done, 2026-10-01)

**Requirements:** `R-AGT-1`, `R-HIS-1`, `R-TUI-6`; constrained by `R-SEC-3`/`R-ID-7` (scrub on the
executing box), `R-TUI-4` (Runs cancel), `R-NF-2` (no daemon but Postgres).
**Origin:** ANA-16 (`docs/ANA-16.md` §5.5, §8 item 3, §9 risk 2). It absorbed MOD-37's **R-38**
(graceful preempt) and replaced MOD-41's "cancelling a live run needs MOD-42" refusal.
**Artifacts:** PRD
[`.claude/prds/mod-42-permission-relay.prd.md`](../../../.claude/prds/mod-42-permission-relay.prd.md),
plan
[`.claude/plans/mod-42-permission-relay.plan.md`](../../../.claude/plans/mod-42-permission-relay.plan.md)
(fact-checked: 39 claims, 8 falsified and amended before CONFIRM) and blueprint
`.claude/plans/mod-42-permission-relay.blueprint.md` (findings F-1..F-19, decisions B-1..B-21,
implementation amendments A-1..A-14 in §13). User guide [`docs/htui-worker.md`](../../htui-worker.md).
Routed as a PRD (C2, C4 fired, at threshold), ultracode for implement and review, run in the TOOL-7
sandbox on `hr/MOD-42`.
**Decisions:** maintainer, 2026-10-01: PRD gate Q1-Q10 as recommended; plan confirmed with
OQ-1..OQ-5 as recommended (a losing answer is refused to its sender, not persisted; a cancel whose
executor is gone stays pending; promote of a worker-walked step stays refused; the relay tables are
not mirrored; shutdown stays a hard drop).
**Commits:** `476b4e8`..`bbd40c5` on `hr/MOD-42` (PRD, plan, blueprint, seven tasks T0-T6 with
their verify/repair rounds, the review round) plus this close-out. Migration `0011`.

## What shipped

**The relay (M1).** Engine-driven ACP steps no longer fail at their first permission request.
- `drive_once` evaluates the agent's own `agent.settings.permission` policy (stages 1-2, the same
  evaluator as chat) instead of `PermissionPolicy::default()`.
- A request that still needs a human becomes a pending `step_permission` row (migration
  `0011_permission_relay.sql`), written by the executing process with the summary and option labels
  scrubbed by the recorder's scrubber (fail-closed).
- The executor waits, polling that row every 1 s (`RELAY_POLL`), until any store client answers it
  with a compare-and-set that never takes the lease: `pending → answered` only while the row's owner
  still holds a live lease (`clock_timestamp()`). It then applies the answer under a fence on its own
  lease, answers the session and records `permission_answer` itself, so `session_event` stays
  single-writer and fenced.
- One loop, `htui_agent::record::drive`, carries the relay and a per-run cancel control; `pump` is
  `drive` with no relay (all 26 call sites unchanged), boxed so its state machine stays out of
  every caller's future.
- Nine `WriteStore` methods (MemStore is the reference semantics, PgStore in `pg/relay.rs`), forwarded
  by the new narrow `RelayStore` and by `WorkerStore`; 12 new shared conformance cases.

**Runs-pane answering (M2).** `RunsTab` asks for `RelayView { item }` beside `RunActions`; a step
with a pending request shows its summary and the chat's `PermissionStrip`, and digits `1`-`9` answer
it (consumed only while that step has a request, so tab-select still works elsewhere). Refusals
(already answered, executor gone, option not offered) land on the status line. Offline the view is
empty, never an error. One answer per request is in flight.

**Durable graceful cancel (M3, R-38).**
- Every cancel of a leased run writes a `run_command` row first. The process walking the run applies
  it gracefully: parked requests are answered `cancelled` (I-7's `permission_answer {option_id:null,
  by:"policy", cancelled:true}`, also for requests drained after the cancel),
  `session.cancel(grace)`, then the walk is dropped. No cancelled walk settles or fails anything
  (I-6); `cancel_leased` owns the terminal statuses.
- A run another process holds, on this box or another, answers "cancel requested: the run's executor
  applies it"; `RunRuntime::poll_commands_with` (every 1 s, worker loop and TUI store loop) applies it.
  A cancel of a run that finished first is resolved `refused` with its status. MOD-41's
  `worker_walks` refusal is gone.
- Promote's in-process preempt is graceful too; a command queued behind a cancelled walk leaves at
  the signal.

**Review gate.** `rust-reviewer` approved with changes (3 MEDIUM, 4 LOW); one verifier per finding
refuted L-2 and L-3. Fixed: drained requests get I-7's row (M-1); a failed `recorder.finish()` no
longer masks a cancel or a lost fence (M-2); cancel's writes are best-effort and the parked poll rides
out transient store errors (M-3); a dropped walk's requests are staled when its owner holds the run
again (L-1); a queued cancel that loses to a claim goes durable (L-4).

## Decisions worth keeping

**Answerers never write `session_event` and never take the lease (I-1).** Every answer and command is
a row in its own table; only the lease holder echoes it into the transcript. This is the row protocol
MOD-47's control plane will push.

**A lost fence settles nothing (A-7).** A relay apply that finds its lease gone is a new source of
`Fenced`; `walk_step`, `run_candidate` and `run_judge` return before any failure write on it, as on a
cancel, so a stale walk never writes `failed` on a run another process holds.

**Box large futures on the walk path.** `htui-orch`'s `every_case_name_dispatches` runs within about
0.4 MiB of the 2 MiB debug stack; `drive` inlined into `pump` overflowed it (SIGABRT, which stops a
workspace run silently without `--no-fail-fast`). `pump` and `drive_once` box `drive`.

**Shutdown stays a hard drop (OQ-5, A-13).** A walk parked at shutdown is dropped without a
`cancelled` answer; its row is marked `stale` and can never be answered.

## Left open

- **MOD-70** (new, from this item, PRD Q9): follow-up on engine steps as a `run_command` kind. The
  engine has no follow-up verb and ANA-2 no state for it.
- `p` (promote) on a step the worker walks stays refused, with a sentence naming the worker (OQ-3).
- No `LISTEN`/`NOTIFY`: an answer resumes within one 1 s poll; MOD-43/MOD-46 own wake-ups.
- "Allow always" is forwarded to the agent only; `remembered[]` still has no writer (D16).
- Chat keeps its own loop (`run_turn`); folding it onto `drive` is a possible CLEAN item (D7), not
  filed.
- A parked step holds its slot and lease until answered or cancelled (PRD Q3, no timeout).
- Mixed versions: a worker built with `0011` refuses an unmigrated database; migrate from a TUI first
  (`docs/htui-worker.md`).

## Pins after this item

Store conformance `CASES` 116 (was 104), `READ_CASES` 14; `htui-orch` `CASES` unchanged. Migrations
11 (was 10), Postgres tables 41 (was 39). `crates/htui-store/.sqlx` 307 files (was 291);
`crates/htui/tests/snapshots` 122 (was 121). `StoreRequest` 93 / `StoreReply` 54 (were 91 / 52;
the 85 / 47 in `mod-41.md` was already stale). `WorkerStore` 45 own methods, `RelayStore` 4,
`EngineParts` 18 fields. The full workspace suite passed 3157 and failed 0, with 26 ignored, against
the sandbox Postgres (`--no-fail-fast`, no aborts).
