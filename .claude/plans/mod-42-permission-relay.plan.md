# Plan: MOD-42 — Permission and control relay through Postgres

**Status: IMPLEMENTED 2026-10-01 (all three milestones, T0-T6 and the review round; write-up `docs/decisions/mod/mod-42.md`; implementation amendments in the blueprint's §13). CONFIRMED by the maintainer 2026-10-01, OQ-1 to OQ-5 as recommended. Fact-checked
2026-10-01 (handoff-run step 3.5: five independent verification passes, workflow
`wf_362e6c61-139`; falsified and partly-true claims amended in place, see "Verified claims").**

**Source PRD**: `.claude/prds/mod-42-permission-relay.prd.md`, all three milestones, with its gate
decisions (maintainer, 2026-10-01, "proceed" = every recommended default; cited as **PRD Q1-Q10**).
Design origin: `docs/ANA-16.md` §5.5, §8 item 3, §9 risk 2. Contracts: `docs/ANA-4.md:489-515`
(permission pipeline, cancel MUST), `docs/ANA-2.md:103-142` (invariants), `:1610` (`CancelRun`).
Carried in: MOD-37's **R-38** (`HANDOFF.md:189-191`, absorbed per PRD Q6); MOD-41's refusal
(`crates/htui-worker/src/runtime.rs:95-101`, `:2019-2023`; `docs/htui-worker.md:56-58`, `:114`).

**Requirements**: `R-AGT-1`, `R-HIS-1`, `R-TUI-6`; constrained by `R-SEC-3`/`R-ID-7` (scrub on the
executing box), `R-TUI-4` (Runs cancel), `R-NF-2` (no daemon but Postgres).

**Complexity**: Large. One migration (`0011`), two tables, seven new `WriteStore` methods forwarded
through one new narrow trait (`RelayStore`) and `WorkerStore`, a new session loop in `htui-agent`,
engine changes across every session call site, a graceful-preempt and command-poll layer in
`htui-worker`, a Runs-pane surface in `htui`, and the `.sqlx` regeneration.

**Routing**: `/handoff-run MOD-42`, PRD path (C2, C4 — at threshold). Ultracode accepted for the
implement and review phases. Reviewer: `rust-reviewer` (`.claude/workflow-config.json`).

**Numbering**: decisions **D1…**, tasks **T0…**, risks **R-1…**, open questions **OQ-1…**,
invariants **I-1…**. The PRD's are **PRD Q1…**.

**Tree reading**: HEAD `476b4e8` (branch `hr/MOD-42`, sandbox). Six-reader survey plus a synthesis
(workflow `wf_eff69361-449`), direct reads of the seams, then the fact-check. Gortex answers in
this sandbox despite its "INACTIVE" banner. Paths are relative to `crates/` for code, the repo root
for docs.

---

## Open questions for the maintainer (read these first)

- **OQ-1 · A losing answer is refused to its sender, not persisted.** PRD MVP item 3 says a late
  answer is "persisted as refused". With one request row and a compare-and-set answer (D3), the
  loser changes nothing and is told the actual state — ANA-2 inv. 1 ("zero rows is a race,
  reported as such"). Inv. 7 is about orchestration refusals (capability, budget, judge), not
  about a second key press. Persisting losers needs a third, append-only table for no reader.
  **Recommended: refuse to sender, do not persist**; the PRD line is amended at CONFIRM.
- **OQ-2 · A cancel whose executor is gone stays pending.** A cancel of a leased run on another box
  (or a worker that is down) is a pending `run_command`; the Runs pane shows "cancel requested".
  It is applied by whichever process next holds or adopts the run on that box (D13). No timeout.
  **Recommended: accept.**
- **OQ-3 · `p` (promote) on a worker-walked step stays refused.** Promote hands a live session to
  *this* TUI's chat, which cannot cross processes. Its refusal gets a sentence naming the worker.
  **Recommended: accept; the in-process promote preempt still becomes graceful (R-38).**
- **OQ-4 · The two tables are not mirrored into the SQLite cache.** Offline there is no writer
  (`Backend` implements no `WriteStore`, `htui-store/src/backend.rs:6`), so nothing could be
  answered; the Runs pane shows no pending strip offline and no error either (D14).
  **Recommended: accept.**
- **OQ-5 · Shutdown stays a hard drop.** `RunRuntime::shutdown` and `forget_server` drop every walk
  at once (`runtime.rs:1235-1258`, `:1107-1115`); a walk parked on a permission is dropped without
  answering `cancelled` or marking its row. The agent process dies with the walk and the row is
  unanswerable by D3's owner guard (`Engine::abandoned` clears the lease). Routing shutdown through
  D11's graceful preempt is possible inside the existing 2×grace window but changes shutdown
  timing. **Recommended: keep the hard drop; scope I-7 and D5 to cancel/promote and normal
  returns.**

---

## Summary

Engine-driven ACP steps stop failing at their first permission request. The engine evaluates the
agent's own policy first (stages 1–2); a request that still needs a human is written as a
**pending `step_permission` row** by the executing process, which then waits — polling that row at
~1 s — until any store client answers it with a compare-and-set, or a cancel arrives. The executor
applies the answer to the live session and records the `permission_answer` event itself, so
`session_event` stays single-writer and lease-fenced. **Cancel becomes a durable `run_command`
row**, applied gracefully (parked requests answered `cancelled`, `session.cancel(grace)`, then the
walk is dropped) by whichever process holds the run — the local runtime, the box's worker, or the
next adopter — replacing MOD-41's refusal and closing R-38. The Runs pane shows a step's pending
request and answers it with the chat's digit keys, from any box holding the DSN.

## Invariants (every task keeps these)

- **I-1 · Answerers never write `session_event` and never call `take_lease`.** Only the executor
  echoes answers, through its fenced recorder (ANA-16 §8 item 3, C2).
- **I-2 · An answer is applied only by the current lease holder, to the live session.** A row's
  `owner` is the executor's lease owner at park time; the answer CAS and the apply CAS both require
  `run.lease_owner = row.owner` (D3, D4). A new session of the same step makes older rows stale
  (D5).
- **I-3 · Every status move on the new tables is a compare-and-set**; zero rows is reported, never
  retried blindly (ANA-2 inv. 1).
- **I-4 · Times on the new tables come from SQL `clock_timestamp()`** (C2), never a box clock.
  MemStore's reference uses its per-handle clock (`MemStore::now`, `htui-core/src/store/mem.rs:849`).
- **I-5 · Everything the relay persists from an agent is scrubbed first** (`R-SEC-3`, `R-ID-7`):
  the summary and option labels pass through the recorder's own scrubber, fail-closed on
  `Unmasked`.
- **I-6 · A graceful cancel settles nothing.** After a cancel reaches a walk, no candidate, judge or
  step writes `finish_step`/settle/fail; `cancel_leased` owns every terminal status, as today.
- **I-7 · ANA-4 §510-515 holds on every graceful cancel path** (cancel, promote preempt; not
  shutdown — OQ-5): every parked responder is answered `cancelled` before `session/cancel`
  completes, and each is recorded as `permission_answer {option_id:null, by:"policy",
  cancelled:true}`. Neither ACP nor the fake writes that row (`fake.rs:564-603`), so `drive` does.
- **I-8 · CLI agents and `pump`'s 26 callers are untouched** (`registry.rs:179`; D6).

## Design decisions (settled here, not in code review)

### Store (M1)

- **D1 · Migration `0011_permission_relay.sql`, two tables.** Header in the `0010` style
  (`-- 0011_permission_relay.sql - MOD-42 (plan D1).` / `-- Forward-only (R-STO-5).`); table
  comments optional. Not mirrored (OQ-4); `schema_version` becomes 11 and each box rebuilds its
  mirror once (`cache/mod.rs:141-153`). A headless worker never migrates
  (`migrations.rs:1012`), so a worker built with 0011 waits for a TUI to migrate — T6 documents the
  order.
  - **`step_permission`** — one row per parked stage-3 request. `id UUID PK` (Rust-minted
    `PermissionId`), `run_id` and `run_step_id` FKs **`ON DELETE CASCADE`** (`delete_project`
    relies on cascades, `pg/write.rs:3482-3484`), `session UUID` (`RelaySessionId`, one per driven
    session — a candidate's, each judge call's), `request_id TEXT`, `tool_call_id TEXT NULL`,
    `summary TEXT NULL` (scrubbed title/kind of the remembered `ToolCallEvent`; `None` when the
    transport named no call — `PermissionRequestEvent` has no title, `event.rs:363-371`),
    `options JSONB` (`[{id,label,kind}]`, labels scrubbed), `owner UUID`, `status TEXT CHECK
    (pending | answered | applied | cancelled | stale)`, `option_id TEXT NULL`, `answered_by UUID
    NULL`, `answered_box UUID NULL`, `created_at`, `answered_at`, `resolved_at`
    (`clock_timestamp()`). `UNIQUE (session, request_id)`; index `(run_id) WHERE status IN
    ('pending','answered')`.
  - **`run_command`** — one row per requested command. `id UUID PK` (`RunCommandId`), `run_id` FK
    `ON DELETE CASCADE`, `kind TEXT CHECK (kind IN ('cancel'))` (follow-up later adds a kind — PRD
    Q9), `issued_by UUID`, `issued_box UUID`, `status TEXT CHECK (pending | applied | refused)`,
    `resolution TEXT NULL`, `issued_at`, `resolved_at` (`clock_timestamp()`). Partial unique index:
    one `pending` row per `(run_id, kind)`.
- **D2 · Methods live on `WriteStore`; `RelayStore` and `WorkerStore` forward.** The forwarding
  convention requires a same-named target (`htui-store/src/worker.rs:3-6`), and the shared
  conformance suite is written against `WriteStore` alone (`htui-core/src/store/conformance.rs:1-7`),
  so every new method is a `WriteStore` method (writer-only and online, like `repos`/`command_runs`,
  `traits.rs:709`, `:1331`, `:1318-1328`):
  - executor side, forwarded by a new **`RelayStore`** trait in `htui-core/src/store/worker.rs`
    (beside `RecorderStore` at `:40`; no default bodies; `-> impl Future + Send`; bound by path, never
    `use`d beside `WriteStore`, E0034): `open_permission(OpenPermission) -> PermissionId`
    (also marks `stale` every `pending|answered` row of the same step from an **older session**,
    closing the same-owner gap — D5), `permission(id) -> Option<StepPermission>`,
    `apply_permission(id, owner) -> Option<PermissionChoice>` (CAS `answered → applied` fenced on
    `run.lease_owner = owner`), `settle_permissions(session, to) -> u64`;
  - `WorkerStore: RecorderStore + RelayStore` (compile-probed, A1) gains the command side:
    `request_cancel(run, user, box) -> CancelRequest` (`Inserted(id) | AlreadyPending(id)`),
    `pending_commands(owner, box) -> Vec<RunCommand>`, `resolve_command(id, applied|refused,
    resolution) -> bool` (CAS on `pending`);
  - answerer side, `WriteStore` only (D3): `relay_view(item) -> RelayView { permissions:
    Vec<StepPermission>, cancels: Vec<RunId> }` and `answer_permission(id, option_id, user, box)
    -> AnswerOutcome`.
  - Implementors (fact-checked, complete): `WriteStore` — MemStore `mem.rs:5801`, PgStore
    `pg/write.rs:714`, Writer `htui-store/src/writer.rs:310`, and the spies `UsageSpy`
    (`htui-agent/src/conformance.rs:741`) and `SpyStore` (`htui-agent/tests/recorder.rs:429`), which
    forward. `RelayStore` — MemStore, PgStore, Writer only (the spies do not need it, D6).
    `WorkerStore` — MemStore `worker.rs:389`, PgStore `htui-store/src/worker.rs:53`, Writer `:287`.
    `Backend`/`CacheStore` implement no `WriteStore`; untouched. Module docs pinning "WorkerStore is
    42 methods" (`htui-core/src/store/worker.rs:15-17`) are updated.
- **D3 · The answer is a compare-and-set, never a lease take.** `answer_permission`: `UPDATE
  step_permission SET status='answered', option_id=$2, answered_by=$3, answered_box=$4,
  answered_at=clock_timestamp() WHERE id=$1 AND status='pending' AND owner = (SELECT lease_owner
  FROM run WHERE id = run_id AND lease_expires_at > clock_timestamp())`, with `$2 ∈ options[].id`
  checked. Zero rows → re-read (the `take_lease` shape, `pg/write.rs:4035-4081`) and return
  `Refused { actual }`: `answered`, `applied`, `cancelled`, `stale`, "its executor is gone", or
  "not an offered option". MemStore reads `State.lease_owners` (`mem.rs:216-219`) — the Rust `Run`
  has no `lease_owner` (`model/run.rs:203-205`); PgStore does the check in SQL, never via `Run`.
  `relay_view` returns `pending` rows whose `owner` equals the live lease owner, and the run ids
  with a `pending` cancel.
- **D4 · Apply is fenced; the echo is the recorder's.** The executor: `apply_permission` CAS →
  `session.answer_permission` → `recorder.record_permission_answer(&id, Some(opt), AnsweredBy::User,
  false, at)` (`record.rs:803-833`). A `None` from the CAS while the row reads `answered` means the
  lease is gone: `drive` returns `DriverError::Store(StoreError::Fenced{step})`, and `drive_once`
  lifts it to its outer `Err` (D10) so `is_fenced` (`engine.rs:6025-6034`) makes it `LeaseLost`.
- **D5 · Stale rows.** `drive` marks its session's leftover rows `stale` on every normal return and
  `cancelled` on a graceful cancel. A hard drop (OQ-5) leaves them `pending`, unanswerable by D3's
  owner guard once `abandoned` clears the lease; the same process re-taking its own run with the same
  owner is covered by `open_permission` staling older sessions' rows. `adopt_runs` is unchanged
  (`pg/write.rs:3969`, `mem.rs:6210`).
- **D6-ids · New ids in `model/ids.rs`.** `PermissionId`, `RunCommandId`, `RelaySessionId` join
  the `id_newtype!` list (`htui-core/src/model/ids.rs:75-116`, private macro) and the
  `pub use ids::{…}` at `model/mod.rs:115`; `model/relay.rs` holds only the row and outcome types.
  Options are a core type `RelayOption {id, label, kind}` (htui-core cannot depend on htui-agent,
  whose `PermissionOption` is at `event.rs:352`); `htui` converts for the strip (D14).

### Agent loop (M1)

- **D6 · `htui_agent::record::drive`, generic over a separate relay store.** Signature shape:
  `drive<S: RecorderStore, R: RelayStore>(session, &mut Recorder<'_, S>, relay: Option<&Relay<'_,
  R>>, control: &mut Control) -> Result<DoneEvent, DriverError>` (the separate `R` compile-probed,
  A1b; a single `S: RecorderStore + RelayStore` fails E0277 for the spies). It lives in `record.rs`
  or a child module `record/relay.rs`, where the recorder's private `flush` (`record.rs:1107`) and
  `scrubber` field (`:379`) are visible. `Relay` carries the store, the owner, the run/step ids, the
  session id, the policy, the poll interval (D8) and an instant source (`record_permission_answer`
  takes a caller-supplied `at`; the engine's instants come from its injected clock). Behaviour:
  - `ToolCall` events are remembered by id (as `run_turn` does) for policy rules and the summary;
  - on `PermissionRequest` **with a relay**: `permission::evaluate(policy, call, options)`
    (`permission.rs:52-102`); `Some` → answer, record `AnsweredBy::Policy`; `None` → flush the
    recorder, `open_permission` (scrubbed, I-5), then wait in a `tokio::select!` over the poll
    interval and `control.changed()`, checking `*control.borrow()` **before parking and after every
    wake** (`changed()` is edge-triggered; a cancel sent before the clone or the park is otherwise
    missed — probe A3b);
  - answered row → D4; `stale`/`cancelled` read back → treated as a cancel;
  - cancel → `session.cancel(grace)` (ACP answers every parked responder `Cancelled` itself,
    `acp/mod.rs:1567-1614`), `record_permission_answer(&id, None, AnsweredBy::Policy, true, at)` for
    each parked id (I-7), `settle_permissions(session, Cancelled)`, drain to the stream's end, return
    `Err(DriverError::Cancelled)` — a **new** variant (`htui-agent/src/error.rs`; none exists today,
    and no exhaustive match outside the crate breaks).
  - **`pump` is `drive` with `relay: None`** (`None::<&Relay<'_, NoRelay>>`, `NoRelay` an
    uninhabited type implementing `RelayStore` by `match *self {}`): without a relay `drive` skips
    evaluation and keeps pulling exactly as today, so the transport still raises "is parked" after
    its queue drains (`fake.rs:393-405`) and all **26** `pump` call sites are byte-identical (21 in
    `htui-agent/src/conformance.rs`, `tests/extensibility.rs:570`, `tests/recorder.rs:2361, :3506,
    :4057`, and `drive_once` until T3). Two loops remain (`drive`, chat's `run_turn`), as today.
- **D7 · Chat is not moved onto `drive` in this item.** Noted at close-out as a possible CLEAN item.
- **D8 · Poll interval.** Production 1 s (PRD Q5); tests shorten it. One primary-key read per tick,
  only while a request is parked.

### Engine and runtime (M1, M3)

- **D9 · The engine uses the agent's policy.** `EngineParts` gains `policy: PolicyFor<'a>` with
  `pub type PolicyFor<'a> = &'a (dyn Fn(AgentId) -> PermissionPolicy + Sync);` (`+ Sync` is
  load-bearing: walks are spawned and `a_dispatch_future_is_send`, `engine.rs:12782`, pins Send —
  probe A2). `Kit` answers it from `self.agents` exactly as chat does (`agent_worker.rs:968-969`).
  `drive_once` (`engine.rs:5659`) passes `policy(candidate.agent_id)` into the `SessionSpec` and the
  `Relay`; judge calls use the judge agent's.
- **D10 · Cancel and fence leave the session result.** `EngineParts` gains `control: ControlFor<'a>`
  = `&'a (dyn Fn(RunId) -> Control + Sync)`, a per-run lookup served from `Walks`, because
  `start_run` builds its engine before the run's walk token exists (`runtime.rs:1849` vs `:1860`)
  and `Kit::engine` has nine call sites (`:1546, :1639, :1740, :1781, :1808, :1849, :1951, :2038,
  :2101`); non-walking engines get a never-signalled control. `drive_once` clones one receiver per
  session (fan-out candidates and judge calls run concurrently under `join_all`, `engine.rs:3682`).
  `drive_once` **lifts two results out of its inner `SessionResult`** before `recorder.finish`/settle:
  `DriverError::Store(Fenced)` → outer `Err(EngineError::Driver(..))` (→ `LeaseLost`), and
  `DriverError::Cancelled` → new `EngineError::Cancelled { run }` (`command.rs:311`). That variant
  passes unchanged through every catch-all that would otherwise write a failure (I-6):
  `walk_step`'s `fail_hard` arm (`engine.rs:3253-3262`), `run_candidate`'s `fail_candidate` arm
  (`:3846-3853`) and the `join_all` first-error handling (`~:3690`), and both judge
  `SessionFailed` conversions (`:4859-4862` in `judge_calls`, `~:4396-4399` in `run_judge`). The
  walk then ends; `walk_leased`'s Err arm releases the lease (`:1814-1822`), and the cancel task
  retakes it in `cancel_leased` (same process holds the `RunLocks` entry FIFO).
- **D11 · Graceful preempt (R-38).** `Walks`' `Parent` gains the `watch::Sender<Signal>`; the
  control lookup hands out `receiver.clone()` of the channel's own receiver. `preempt_gracefully(run,
  grace)`: send `Cancel`, wait until every task of the run dropped its `WalkToken` or `grace + 1 s`,
  then cancel the token as today. `CancelRun` (`Preempt::Always`) and `PromoteStep`
  (`Preempt::IfLive`, `runtime.rs:1788-1789`) use it. Shutdown and `forget_server` stay hard drops
  (OQ-5).
- **D12 · Durable cancel.** `CancelRun` from any TUI:
  1. queued run → today's CAS path (`cancel_from_queued`), no row;
  2. otherwise `request_cancel` writes the `run_command` row first (PRD Q7);
  3. this process walks the run → D11, then `cancel_leased`, `resolve_command(applied)`;
  4. the run executes on **another box** (decided from `run.executing_box_id` **before**
     `cancel_leased`, whose refusal there is `EngineError::RunStatus`, `engine.rs:1926-1931`, not
     `LeaseHeld`) → leave `pending`;
  5. else `cancel_leased`: success → `applied`; `LeaseHeld` → leave `pending`;
  6. a pending row is answered **"cancel requested: the run's executor applies it"**, replacing
     `worker_walks` (`runtime.rs:95-101`, re-exported `htui-worker/src/lib.rs:51`, imported
     `crates/htui/src/run_worker.rs:176`) and its pinned test (`run_worker.rs:2740-2776`).
- **D13 · The runtime polls commands.** `RunRuntime::poll_commands(host, sink)`, the `sweep_with`
  shape (sync, one tracked task under an in-flight CAS flag so ticks never overlap,
  `runtime.rs:991-1027`): read `pending_commands(owner, box)` — rows for runs whose `lease_owner =
  owner`, or that execute on `box` with a free or expired lease and status `running |
  awaiting_approval` — and run each as an internal `CancelRun` (D12 steps 3–5, no second row). An
  already-terminal run → `refused` with the actual status. Ticked every `COMMAND_POLL = 1 s` (a
  `const`, so `WorkerConfig` literals at `crates/htui/tests/worker_pg.rs:52, :60` are untouched) by
  a new `worker::run` arm (`htui-worker/src/worker.rs:44-71`) and by the TUI store loop through a
  new `TuiRuns::poll_commands` (`crates/htui/src/run_worker.rs:92-139`) in a new arm beside the
  sweeper (`store_worker.rs:2199-2201`; the TUI sweeper period is the 120 s lease TTL, not 5 s).

### TUI (M2)

- **D14 · The Runs pane shows and answers pending requests.**
  - `RunsTab::on_runs` (`runs.rs:330-349`) sends `StoreRequest::RelayView { item }` beside
    `RunActions`, so every Runs reply (selection, frame, Orch answer, the 5 s poll) refreshes it;
    `backlog/mod.rs` is unchanged. The pinned request list in
    `the_first_runs_reply_subscribes_once_and_asks_for_the_actions` (`runs.rs:2983-3027`) is
    updated.
  - Offline (`backend.writer()` is `None`) `RelayView` is served an **empty** reply, never
    `Failed`/`Unreachable` (a `Failed` always lands on the status line, `app/update.rs:285-287`, and
    `Unreachable` calls `go_offline`, `store_worker.rs:2121-2127`). `AnswerPermission` refuses
    offline with `DATABASE_UNREACHABLE` (the `requirements.rs:710-717` pattern).
  - A step with a pending request takes extra lines: the scrubbed summary and the strip, each
    fitted to the pane's 43 columns (`runs.rs:86`). `PermissionStrip` (`ui/tabs/chat/permission.rs`)
    gains `line(options, theme) -> Line<'static>` and `pick_from(options, digit)`; its existing
    `render`/`pick` delegate, so chat (`chat/mod.rs:323`, `:727`) is unchanged. The existing pins
    `every_step_takes_two_lines` (`runs.rs:1604`) and `every_step_row_fits_forty_three_columns`
    (`:1692`) keep holding for steps with no request; new pins cover the extra lines.
  - **Digits**: while the step under the cursor has a cached pending request the pane consumes
    **every** digit `1`-`9` (one past the offered options sends nothing), mirroring
    `chat/mod.rs:490-499`; otherwise it returns `Pass` so the global tab-select (`keymap.rs:225-232`)
    still works. A digit sends `StoreRequest::AnswerPermission { permission, option_id }`; a refusal
    goes to the status line.
  - The run line shows "cancel requested" while the run id is in `RelayView.cancels`.
- **D15 · Request/reply counts.** No test pins them (the 85/47 in `mod-41.md:106-107` is stale;
  HEAD is 91/52). The compile-time breakers are the exhaustive `StoreRequest::name()`
  (`store_worker.rs:826-935`) and `try_serve` (`:1422ff`). New counts are restated at close-out.

### Scope guards

- **D16 · "Allow always" is forwarded to the agent only** (PRD Q8), as chat does —
  `remembered[]` has no writer (`ui/tabs/chat/permission.rs:32`).
- **D17 · No `LISTEN`/`NOTIFY`** (PRD Q5); no follow-up command (PRD Q9); gate verbs and hand-back
  unchanged; `adopt_runs` unchanged (D5).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store trait | `htui-core/src/store/worker.rs:40` (`RecorderStore`) | No default bodies; `-> impl Future + Send`; forwards by UFCS to a same-named `WriteStore` method; bound by path |
| CAS write | `htui-store/src/pg/write.rs:4035-4081` (`take_lease`) | Single statement, `WHERE status = $expected`, `clock_timestamp()`, re-read on zero rows |
| Migration | `htui-store/migrations/0010_prompt_digest_undigested.sql:1-2` | `-- 00NN_name.sql - MOD-N (plan D…).` / `-- Forward-only (R-STO-5).` |
| Ids | `htui-core/src/model/ids.rs:75-116` | `id_newtype!` list, `Uuid::now_v7()` minted in Rust, bound at insert |
| Store conformance | `htui-core/src/store/conformance.rs` (`CASES` `:44`, `run_case` `:160`) | One case per new method, run by `htui-core/tests/mem_store.rs` and `htui-store/tests/pg_conformance.rs` |
| Session loop | `htui/src/agent_worker.rs:3814-3959` (`run_turn`) | Policy stage, parked wait, cancel ordering |
| Fake parking agent | `htui-agent/src/fake.rs:450` (`ParkPermission`), `:521` (`answer_permission`) | Parks like ACP |
| Engine factory closure | `htui-orch/src/engine.rs:271-272` (`DriverFor`) | `&'a (dyn Fn(..) -> .. + Sync)` |
| Tick without overlap | `htui-worker/src/runtime.rs:991-1027` (`sweep_with`) | Sync entry, one tracked task, in-flight CAS flag |
| Offline-safe read | `htui/src/…/requirements.rs:710-717` | `backend.writer()` → refuse offline for writes; empty reply for display reads (D14) |
| Digit capture | `htui/src/ui/tabs/chat/mod.rs:490-499` | Consume every digit only while parked |
| Tests (Postgres) | `htui/tests/worker_pg.rs`, `htui-store/tests/pg_criteria.rs` | `--features testkit`, one scratch **database** per test (`htui-store/src/testkit.rs:118`) |

## Tasks

Order: **T0 → {T1 ∥ T2 ∥ T5} → T3 → T4 → T6**. Independence is decided by the file-set
intersections recorded under "Verified claims", not by this prose. TDD throughout: each task's
tests are written first and fail for the stated reason. Each implementer commits incrementally.

### T0: Contracts and the MemStore reference (serial, first)
- **Action**: ids (D6-ids); `model/relay.rs` row/outcome types and `RelayOption`; the seven
  `WriteStore` methods (D2, D3); `RelayStore`; `WorkerStore` supertrait and command methods;
  MemStore implementations (reference semantics, `State.lease_owners`, per-handle clock);
  forwarding for `Writer`, `UsageSpy`, `SpyStore`, and the `RelayStore`/`WorkerStore` impls for
  MemStore/PgStore/Writer. **PgStore's seven `WriteStore` bodies return
  `Err(StoreError::Backend("MOD-42 T1: not yet implemented".into()))`** so the workspace compiles;
  T1 replaces every one (its validate greps the marker away). Shared conformance cases.
- **Tests first**: conformance cases — answer `pending→answered`; second answer refused
  `answered`; answer after the owner changed refused "executor is gone"; option not offered
  refused; apply fenced; `open_permission` stales older sessions' rows; `settle_permissions` moves
  only `pending|answered`; one pending cancel per run; `resolve_command` CAS; `relay_view` hides
  stale rows and lists pending cancels; delete project cascades.
- **Files**: `htui-core/src/model/ids.rs`, `model/mod.rs`, `model/relay.rs` (new),
  `htui-core/src/store/traits.rs`, `store/worker.rs`, `store/mem.rs`, `store/conformance.rs`,
  `htui-core/tests/mem_store.rs` (104 → N), `htui-store/tests/pg_conformance.rs` (`EXPECTED_CASES`
  `:21` and the stale "103" at `:28`), `htui-store/src/pg/write.rs` (placeholder bodies only),
  `htui-store/src/writer.rs`, `htui-store/src/worker.rs`, `htui-agent/src/conformance.rs`
  (`UsageSpy` forwarding only), `htui-agent/tests/recorder.rs` (`SpyStore` forwarding only).
- **Validate**: `cargo test -p htui-core --all-features`; `SQLX_OFFLINE=true cargo check --workspace
  --all-targets --all-features`.

### T1: Postgres (parallel with T2 and T5)
- **Action**: `0011`; PgStore bodies (replacing T0's placeholders) in `pg/write.rs` or a new
  `pg/relay.rs`; `.sqlx` via a migrated scratch database (`docs/hr-sandbox.md:196-205`); migration
  pins.
- **Tests first**: `pg_criteria` cases — two `PgStore` clients with different box ids: B answers
  A's pending row, A applies; B cannot answer after A's lease is adopted or expired; times are the
  database's; deleting a project holding relay rows succeeds.
- **Files**: `htui-store/migrations/0011_permission_relay.sql`, `htui-store/src/pg/write.rs`,
  `htui-store/src/pg/relay.rs` (new, optional), `htui-store/src/pg/mod.rs` (if a module is added),
  `htui-store/.sqlx/*`, `htui-store/tests/pg_criteria.rs`, `htui-store/tests/migrations.rs`
  (`TABLES` + 2 and `39 → 41`; `10 → 11` at `:89`, `:917`, `:1016`, `:1021`, `:1040`, `:1190`),
  `htui-store/tests/connect.rs` (`10 → 11` at `:140`, `:156`, `:242`).
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1`;
  `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; no
  "MOD-42 T1: not yet implemented" left in the tree.

### T2: `drive` (parallel with T1 and T5)
- **Action**: D6, D8, `DriverError::Cancelled`; `pump` over `drive` with no relay; the stale
  "eighteen call sites" sentence in `enforce_breach`'s doc (`record.rs:~1893`).
- **Tests first** (over `FakeSession` + MemStore): policy-allowed request never writes a row and
  records `by: policy`; stage-3 request writes one scrubbed row (a secret in the tool title is
  masked) and resumes on an answer, echo `by: user`; cancel while parked answers `cancelled` and
  records I-7's row exactly once; cancel sent **before** the park still ends as a cancel; stale row
  read back ends the turn as a cancel; apply fenced → `Store(Fenced)`; every existing `pump` test
  unchanged.
- **Files**: `htui-agent/src/record.rs`, `htui-agent/src/record/relay.rs` (new, optional),
  `htui-agent/src/lib.rs`, `htui-agent/src/error.rs`, `htui-agent/tests/relay.rs` (new).
- **Validate**: `cargo test -p htui-agent --all-features`.

### T5: Runs-pane answering (M2; parallel with T1 and T2)
- **Action**: D14; `StoreRequest::{RelayView, AnswerPermission}` and replies, `name()` and
  `try_serve` arms (D15); the strip refactor.
- **Tests first**: in `runs.rs`'s `mod tests` — digit sends `AnswerPermission`; digit past the
  options consumed, nothing sent; digit with no pending request passes (tab-select); refusal on the
  status line; "cancel requested" on the run line; extra-line and 43-column pins; the updated
  request-list pin. In `tests/backlog.rs` — the snapshot with a pending strip (MemStore
  `Harness::over`, row seeded through T0's MemStore); offline Runs reply leaves `status == None`.
- **Files**: `crates/htui/src/store_worker.rs` (variants and serve arms only),
  `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/src/ui/tabs/chat/permission.rs`,
  `crates/htui/tests/backlog.rs`, `crates/htui/tests/snapshots/*`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`.

### T3: Engine (serial, after T0 and T2)
- **Action**: D9, D10: `PolicyFor`/`ControlFor` fields; `drive_once` over `drive`, lifting
  `Fenced`/`Cancelled`; `EngineError::Cancelled` passed through the five catch-alls; `Kit` supplies
  the policy lookup and a never-signalled control (T4 wires `Walks`); `fake_parts` gets a
  control from a `FakeOrchestrator`-held sender; "sixteen fields" doc text (`engine.rs:390`,
  `:6340`).
- **Tests first**: an engine walk whose fake agent parks, answered through MemStore by a second
  "client" → step `done`; policy `Allow` → no row, step `done`; cancel via control on a single step,
  on a fan-out candidate and on a judge call → no `finish_step`, parked answered `cancelled`, run
  left for `cancel_leased`; apply fenced → `LeaseLost`.
- **Files**: `htui-orch/src/engine.rs` (EngineParts literals at `:6374` `fake_parts`, `:6707`,
  `:6795`, `:9492`, `:11872`, `:12782`), `htui-orch/src/command.rs` (`EngineError`),
  `htui-orch/src/conformance.rs` (literal `:7525`), `htui-orch/src/fake.rs` (control sender),
  `htui-orch/tests/gix_isolator.rs` (`engine_as!` literal `:92`), `htui-worker/src/runtime.rs`
  (`Kit::engine` literal `:829`, policy lookup).
- **Validate**: `cargo test -p htui-orch --all-features`; `cargo test -p htui-worker
  --all-features`.

### T4: Durable graceful cancel (M3; serial, after T3 and T5)
- **Action**: D11-D13 in `htui-worker`: `Walks` sender, control lookup, `preempt_gracefully`,
  durable `CancelRun`, `poll_commands`, `worker::run`'s 1 s arm; the TUI half:
  `TuiRuns::poll_commands` and the store-loop arm; replace `worker_walks` and its test.
- **Tests first**: in-process cancel is graceful (parked answered `cancelled`, grace honoured, no
  settle); promote preempt is graceful; TUI on a `worker` box: `c` writes a pending row and says
  "cancel requested"; the worker's `poll_commands` applies it within one poll; cancel of a parked
  run on this box applies inline; cross-box run → pending (not a `RunStatus` refusal); terminal run
  → `refused`; second `c` → "already requested"; overlapping ticks run one poll.
- **Files**: `htui-worker/src/runtime.rs`, `htui-worker/src/worker.rs`, `htui-worker/src/lib.rs`
  (`:51` re-export), `crates/htui/src/run_worker.rs` (`TuiRuns`, `:176` import, tests),
  `crates/htui/src/store_worker.rs` (the poll arm beside `:2199`).
- **Validate**: `cargo test -p htui-worker --all-features`; `cargo test -p htui --all-features
  run_worker -- --test-threads=1`.

### T6: End-to-end and docs (serial, last)
- **Action**: Postgres end-to-end: a worker walks a step whose fake agent parks; a second `PgStore`
  client with another box id answers; the step completes within 3 s of the answer. A cancel from
  the second client ends a live worker walk within grace + one poll. Docs: `docs/htui-worker.md`
  (permission, cancel, migration order with a headless worker), `crates/htui/src/cli.rs:75`.
- **Files**: `crates/htui/tests/worker_pg.rs`, `crates/htui/tests/runs_pg.rs`,
  `docs/htui-worker.md`, `crates/htui/src/cli.rs`.
- **Validate**: the full gate below.

## Test plan

| PRD metric | Test | Task |
|---|---|---|
| 0 engine ACP steps failing on a permission | engine walk over a parking fake, answered and policy paths | T3, T6 |
| Answer → resume ≤ ~2 s | end-to-end timing at the 1 s production interval (assert < 3 s) | T6 |
| Cross-box answer | two `PgStore` clients, different box ids | T1, T6 |
| Worker-walk cancel within grace + poll | `worker_pg` cancel from a second client | T4, T6 |
| Parked answered `cancelled` on cancel (I-7) | `drive` cancel-while-parked; runtime graceful preempt | T2, T4 |
| Stale / late answers never applied | owner guard, older-session staling, apply fence | T0, T1, T2 |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-1 · A catch-all turns `Cancelled` into a failure write | Medium | D10 names all five; T3 tests single step, fan-out candidate and judge |
| R-2 · The trait change ripples | High | T0 does it once, serially, over the fact-checked implementor list; PgStore placeholders removed by T1 |
| R-3 · A parked step holds its slot and lease indefinitely | High | Accepted (PRD Q3); visible in the Runs pane; cancel always works |
| R-4 · 1 s polling load | Low | One PK read per parked request per second; one command query per second per process |
| R-5 · Scrubber not applied to the relay row | Medium | I-5; the masked-title test |
| R-6 · `.sqlx` drift; scheduling-dependent suite | High | Scratch-DB `prepare`; gates with `--test-threads=1` and `--all-features` |
| R-7 · Engine behaviour change: policy now decides on the engine path | Medium | Same evaluator as chat; tests for each stage |
| R-8 · Edge-triggered `watch` misses an early cancel | Medium | `borrow()` before parking and after each wake; T2 test |
| R-9 · Mixed-version boxes: a worker with 0011 against an unmigrated database | Medium | Headless connect refuses and reports (MOD-40 C5); T6 documents migrating from a TUI first |

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features -- --test-threads=1   # HTUI_TEST_DATABASE_URL set (sandbox)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Acceptance

- [ ] PRD success metrics each pinned by a test (table above)
- [ ] I-1…I-8 hold, each with a test naming it
- [ ] The MOD-41 refusal and its pinned test are gone; `docs/htui-worker.md` updated
- [ ] R-38 closed (struck from MOD-37's HANDOFF text at close-out)
- [ ] Validation passes; reviewer gate (`rust-reviewer`) findings applied or deferred with the
      maintainer
- [ ] Close-out restates the moved counts from a fresh count: store `CASES` (104 → N), htui-orch
      `CASES`, `StoreRequest`/`StoreReply` (91/52 → …), `.sqlx`, snapshots, migrations (10 → 11)

## Verified claims

Fact-check 2026-10-01, workflow `wf_362e6c61-139`, five verifiers (store, agent, engine/runtime,
TUI, compile probes + independence). Compile probes ran on the repo's toolchain in a scratch crate
under `/tmp` (removed). Amendments are already applied above.

| Claim | Verdict | Evidence / amendment |
|---|---|---|
| `RecorderStore` at `store/worker.rs:39`, no default bodies, UFCS forwarding | PARTLY | Declared at `:40`; forwarding needs a same-named target → D2 puts every method on `WriteStore` |
| Implementor list "test spies, Backend" | PARTLY | Complete list in D2; `Backend`/`CacheStore` have no `WriteStore` |
| `repos`/`command_runs` are `WriteStore` reads | CONFIRMED | `traits.rs:709`, `:1331`, rationale `:1318-1328` |
| Migration convention; 0011 bumps `schema_version` | PARTLY | "COMMENT ON every column" is not a convention (dropped); counts pinned in `migrations.rs`/`connect.rs` → T1 files |
| `take_lease` SQL, lease columns | PARTLY | Confirmed; Rust `Run` has no `lease_owner` → MemStore uses `State.lease_owners` (D3) |
| Ids minted in Rust | PARTLY | `id_newtype!` is private to `ids.rs` → new ids there (D6-ids) |
| Shared conformance run by `pg_conformance.rs` | PARTLY | Suite is `htui-core/src/store/conformance.rs`; pinned 104 twice → T0 |
| `.sqlx` recipe `docs/hr-sandbox.md:196-205` | CONFIRMED | — |
| `Fenced` already maps to `LeaseLost` | PARTLY | Only via a later fenced write; `drive_once` lifts it (D4, D10) |
| `adopt_runs` can stay unchanged | PARTLY | Yes, but same-owner re-take gap → `open_permission` stales older sessions (D5); FKs cascade (D1) |
| `pump` has 20 conformance callers | FALSIFIED | 26 call sites (D6); single-bound `drive` fails E0277 for the spies → separate `R` (probe A1b) |
| `DriverError::Cancelled` may exist | CONFIRMED absent | New in T2 (`htui-agent/src/error.rs`) |
| Fake parks like ACP; cancel answers everything | PARTLY | Cancel writes no `permission_answer` → `drive` writes I-7's row |
| ACP cancel answers parked first (`acp/mod.rs:1567-1614`) | CONFIRMED | — |
| Recorder exposes flush and scrubber | FALSIFIED | Both private (`record.rs:1107`, `:379`) → `drive` in `record.rs` or `record/relay.rs` |
| `permission::evaluate` signature | CONFIRMED | `permission.rs:52-102` |
| `run_turn` behaviour as D6 | CONFIRMED | `agent_worker.rs:3814-3959` |
| `AgentSettings` usable by `Kit` | CONFIRMED | — |
| Two-loops comment `record.rs:1878-1884` | CONFIRMED | — |
| Chat never writes `remembered[]` | CONFIRMED | `chat/permission.rs:32` |
| `EngineParts` literal sites | PARTLY | Nine, incl. `htui-orch/tests/gix_isolator.rs:92` → T3 files |
| `PolicyFor` "the `DriverFor` shape" | PARTLY | Needs `+ Sync` (probe A2: without it the future is not Send) |
| `drive_once` / `session` / judge call sites | CONFIRMED | `engine.rs:5630-5667`, `:5535`, `:4826`, `:3880` |
| A returned Err bypasses settle | FALSIFIED | Five catch-alls write failures → D10 names them; no in-walk abandoned path |
| `cancel_run`/`cancel_leased`; cross-box is `LeaseHeld` | PARTLY | Cross-box is `RunStatus` (`engine.rs:1926-1931`) → D12 step 4 |
| `Walks`/`preempt`/`on_run`/refusal lines | CONFIRMED | `runtime.rs:480-580`, `:1381`, `:1912-2027`, `:95-101` |
| Pinned refusal test `run_worker.rs:2744-2777` | PARTLY | `:2740-2776`; `worker_walks` re-exported `lib.rs:51` → T4 files |
| Command-poll arm locations | PARTLY | TUI loop reaches runs via `TuiRuns` (`run_worker.rs:92-139`) → T4 owns the arm |
| `Kit.agents` holds settings | CONFIRMED | — |
| Shutdown gives walks grace | FALSIFIED | Hard drop (`runtime.rs:1235-1258`) → OQ-5, I-7/D5 scoped |
| `StoreRequest`/`StoreReply` counts pinned 85/47 | FALSIFIED | No pinning test; HEAD is 91/52 (D15) |
| `PermissionStrip` reusable as-is | PARTLY | Takes `TranscriptRow` → `line`/`pick_from` refactor (D14) |
| Digits free on the Runs pane | PARTLY | Globally bound to tab-select → consume only while pending (D14) |
| 5 s Runs poll sends `Runs` | PARTLY | Send `RelayView` from `on_runs`, not `backlog/mod.rs` |
| Offline serves no `WriteStore` | PARTLY | Serve `RelayView` empty, never `Failed` (D14) |
| Snapshot and key-test homes | CONFIRMED / PARTLY | Keys in `runs.rs` `mod tests`; snapshots in `tests/backlog.rs` |
| RPITIT supertrait `WorkerStore: RecorderStore + RelayStore` | CONFIRMED | Probe A1 |
| `select!` over interval + `watch::changed()` holding `&mut dyn AgentSession` | CONFIRMED | Probe A3; edge-triggered → `borrow()` check (A3b) |
| T1 ∥ T2 disjoint; T2 needs nothing from T1 | CONFIRMED | B1, B2 |
| T3 and T4 share `runtime.rs` | CONFIRMED | Serial T3 → T4 |
| T4 ∥ T5 | FALSIFIED | T5 needed T4's `poll_commands`; both edit `store_worker.rs` → the arm moves to T4, T4 after T5 |
| T1's conformance cases in T1's files | PARTLY | Cases are htui-core → T0 |
| Test DB isolation is a scratch schema | FALSIFIED | Scratch database per test (`testkit.rs:118`) |

**File-set intersections for the final order** (`T0 → {T1 ∥ T2 ∥ T5} → T3 → T4 → T6`):
T1 ∩ T2 = ∅ (htui-store vs htui-agent); T1 ∩ T5 = ∅ (htui-store vs crates/htui); T2 ∩ T5 = ∅.
T0 touches files of T1 (`pg/write.rs` placeholders, `pg_conformance.rs`) and T2 (`conformance.rs`
spy, `tests/recorder.rs` spy) but is strictly earlier. T3 ∩ T4 = `runtime.rs` (serial). T4 ∩ T5 =
`store_worker.rs` (serial, T5 first).
