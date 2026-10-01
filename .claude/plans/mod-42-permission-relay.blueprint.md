# Blueprint: MOD-42 — Permission and control relay through Postgres, T0–T6

**Status**: proposed (2026-10-01, code-architect). Implements
`.claude/plans/mod-42-permission-relay.plan.md` (CONFIRMED 2026-10-01, OQ-1 to OQ-5 as
recommended, fact-checked) under `.claude/prds/mod-42-permission-relay.prd.md`. The plan's D1-D17,
I-1..I-8, task order `T0 → {T1 ∥ T2 ∥ T5} → T3 → T4 → T6`, file sets and "Verified claims" are
binding. Where this blueprint had to choose, the choice is a **B-n**, driven by a finding **F-n**.
Anything that would move a confirmed decision is an **E-n** (§0b).

**Verified at**: `0d6a73d` (`hr/MOD-42`, sandbox). `git diff 476b4e8 0d6a73d` touches only the plan
and the PRD, so the plan's line numbers hold; every number below was re-read at HEAD. Paths are
relative to `crates/` unless they start with `docs/`, `.claude/` or name a root file. Gortex answers
symbol reads on this checkout despite its "INACTIVE" banner; it has no field-usage edges, so field
searches were done with `grep '\bname\b'`. **Worktree lanes are not indexed: they read and edit with
native tools.**

**House style (carried from MOD-41)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`; every
new `pub` item is documented and `Debug`; **no default body on a store trait** (`RecorderStore`,
`RelayStore`, `WorkerStore`, `WorkerHost`); `max_width = 100`; every commit compiles; red first,
then green, committed incrementally (uncommitted work dies with the session; never stash on a shared
tree). **E0034 hygiene**: no module `use`s `RecorderStore`, `RelayStore`, `WorkerStore` or
`WorkerHost`; bounds name them by path; every forwarding body and every call on a type that
implements two of the families is UFCS (`WriteStore::answer_permission(&writer, …)`,
`htui_core::store::WorkerStore::request_cancel(&kit.writer, …)`,
`htui_core::store::RelayStore::open_permission(relay.store, …)`).
`every_cross_referenced_test_name_exists` (`htui-core/src/store/conformance.rs`): a backticked
snake_case name with ≥ 4 underscores in that file's docs must be a fn there or in `mem.rs`; a test
elsewhere is spelled `pg_criteria.rs::name` **and must already exist** — so T0's docs never name a
T1 test. **Never the word `zeta`** in an identifier or fixture string
(`htui-agent/tests/extensibility.rs` greps the workspace). **No test walks production parts against
the real home**: every runtime in a test is built with fake parts (`RunRuntime::with_parts`, the
`run_worker.rs` `Fixture`, `worker_pg.rs`'s `Parts`); an offline harness takes
`htui_store::testkit::mock_keyring()` first. Any process a test spawns is reaped (kill-on-drop
guard); this item spawns none.

**Layout**: §0 findings · §0a decisions · §0b escalations · §1 build order and lanes · §2 shared
shapes (2.1-2.11) · §3 T0 · §4 T1 · §5 T2 · §6 T5 · §7 T3 · §8 T4 · §9 T6 · §10 wave schedule and
lane rules · §11 pins · §12 gate reference.

---

## 0. Findings

| # | Severity | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | Minor (T0, T1) | "seven new `WriteStore` methods" (Complexity, T0, T1 placeholders). | D2's own list is **nine**: `open_permission`, `permission`, `apply_permission`, `settle_permissions` (forwarded by `RelayStore`), `request_cancel`, `pending_commands`, `resolve_command` (forwarded by `WorkerStore`), `relay_view`, `answer_permission` (`WriteStore` only). | Nine methods, nine PgStore placeholders in T0, nine bodies in T1. No decision moves. |
| **F-2** | Major (T4) | D13: `RunRuntime::poll_commands(host, sink)` and `TuiRuns::poll_commands`. | MOD-41 blueprint F-7: an **inherent** method shadows a trait method of the same name; `runs.poll_commands(&backend, &tx)` in the store loop would resolve to the inherent `(host: &H, sink: &P)` and fail on `&UnboundedSender`. | **B-1**: library `RunRuntime::poll_commands_with`, TUI `TuiRuns::poll_commands` (the `sweep_with`/`sweep` precedent). |
| **F-3** | Major (T2, T4) | D6: the control is checked before parking, after every wake, and selected against the poll interval. | A cancel that reaches a walk **between** permission requests finds `drive` awaiting `session.next_event()`; with the control read only while parked, D11's preempt waits `grace + 1 s` and hard-drops the walk — no `session.cancel(grace)`, contrary to D11/ANA-2 `CancelRun`. ACP's `next_event` is an mpsc `recv` (`htui-agent/src/acp/mod.rs:498-520`), the fake's resolves at first poll: both are cancel-safe. | **B-2**: `drive` also selects `control.changed()` against `next_event()` (biased, control first) and reads the signal at the top of every iteration; a cancel at any point of the turn takes the same graceful path (§2.5). |
| **F-4** | Note (T3) | D10 names five catch-alls, among them `judge_calls`' `SessionFailed` arm (`engine.rs:4862`). | `judge_calls` calls `drive_once(…).await?` (`:4849-4860`): after the lift, `Cancelled` and `Fenced` are **outer** errors and leave by `?`; the `:4862` arm sees only inner session errors. `judge_sessions` (`:4800-4805`) propagates the outer error with `calls?` after finishing the recorder. | Four arms take code (§7.3); `:4862` gets a one-line comment and is pinned by the judge-cancel test. |
| **F-5** | Minor (T4) | — | A walk the control ended answers `Some(Err(EngineError::Cancelled))` at the runtime's five `walked(&walk, …)` sites (`runtime.rs:1547`, `:1741`, `:1886`, `:1953`, `:2066`), whose generic arm would answer the walk's own requester with the error sentence. | **B-3**: each site treats `Cancelled` as today's preempted `None`: `engine.abandoned(run)` then `ctx.refuse(PREEMPTED)`. |
| **F-6** | Major (T0, T1, T4) | D13: `pending_commands` selects runs whose `lease_owner = owner`, or on `box` with a free/expired lease and status `running \| awaiting_approval`; "an already-terminal run → `refused`". | Those two clauses never select a terminal run (its lease is released, `pg/write.rs:4090-4096`), so a cancel row of a run that finished before its executor saw it stays `pending` for ever and the Runs pane says "cancel requested" on a finished run. | **B-4**: `pending_commands` also returns pending rows of **terminal** runs executing on `box`; `relay_view.cancels` lists only non-terminal runs. D13's own refusal sentence then has an input. |
| **F-7** | Major (T4) | D12 step 3 (inline apply) and D13 (poll). | The inline path writes the row and works under the lease this process holds, so the same process's next poll tick selects that row (`lease_owner = owner`) and applies it a second time; whichever loses the run lock then meets a terminal run. | **B-5**: `Shared.applying: StdMutex<HashSet<RunCommandId>>`; a task applying a row holds a drop-guard entry, and the poll skips ids in the set. |
| **F-8** | Major (T3) | D9/D10: `PolicyFor`/`ControlFor` closures on `EngineParts`; `Kit::engine` has nine call sites. | `Kit::engine(&'a self, driver: DriverFor<'a>)` returns an engine borrowing its parts; a closure built inside `engine` cannot outlive it, and the nine call sites build only the driver closure. Same for `fake_parts` (`engine.rs:6347`). | **B-6**: `Kit` owns `policy: Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync>` and `control: Box<dyn Fn(RunId) -> Control + Send + Sync>`, built in `Kit::read`; `Kit::engine` lends `&*self.policy`, `&*self.control` (dropping `Send` is an allowed auto-trait coercion). The nine call sites are unchanged. `FakeOrchestrator` owns the same pair (§7.4). Test literals use two `fn` items (`ask_policy`, `never_cancelled`). |
| **F-9** | Minor (T0, T1) | D1 lists `answered_by`, `answered_box`, `issued_by`, `issued_box` as `UUID`. | Every other actor column references its table without cascade (`run.started_by REFERENCES app_user(id)`, `command_run.box_id REFERENCES box(id)`, `0001_init.sql:458`, `:540`). No production path deletes a box or user; `testkit::demo_db` deletes the seeded pair before any relay row exists (`testkit.rs:177-186`). | **B-7**: FKs to `app_user(id)` / `box(id)`, no cascade. MemStore checks the same (`references_no_row` `Constraint`) after the row's own checks. |
| **F-10** | Minor (T0) | — | `Run` omits `lease_owner` deliberately (`model/run.rs:203-205`): a reader has no use for another process's liveness token. | **B-8**: `StepPermission` omits `owner`; `OpenPermission` carries it. MemStore keeps it beside the row. |
| **F-11** | Minor (T0, T1) | D3 fences the answer, D4 the apply. `open_permission` is unspecified. | A walk whose lease another process adopted could still open a row (unanswerable by D3's guard, but visible to nobody and never settled by the adopter). | **B-9**: `open_permission` is fenced like a step write: `NotFound { run_step }` first, then `Constraint` (step not of the run), then `Fenced { step }` when `run.lease_owner IS DISTINCT FROM owner` (owner only, no expiry: `StepFence` semantics, `traits.rs:2167-2185`). `apply_permission` likewise fences on owner only; `answer_permission` and `relay_view` require the lease **live** by the store's clock (D3). |
| **F-12** | Minor (T4) | D13: each pending row runs "as an internal `CancelRun`". | `on_run` refuses a verb that moves a run under a live chat (D212, `runtime.rs:1919-1928`). A TUI poll that ignored live chats would cancel a step a chat is driving; one that honoured them through `ctx.refuse` would publish an error frame every second. | **B-10**: `poll_commands_with(host, sink, live: LiveChats)`: the TUI passes the loop's `live_chats(&runtime)`, the worker `LiveChats::default()`. A polled cancel the chat check refuses stays `pending`, logged at `debug`, nothing published. |
| **F-13** | Minor (T2, T4) | D6: after `session.cancel(grace)`, "drain to the stream's end". | A transport that never ends its stream would hang the walk past D11's window. The `run_worker.rs` `Play::Stall` session waits on `release` in `next_event` (`run_worker.rs:279-287`), so a drain could stall a test by `grace + 1 s`. | **B-11**: the drain is bounded by `grace + DRAIN_SLACK` (1 s); on timeout `drive` warns and still answers `Cancelled`. If an existing `Stall` case slows, the stall waits only until its session is cancelled. |
| **F-14** | Minor (T4, T5) | D12 step 6: a pending row "is answered 'cancel requested: …'". | No `OrchReply` carries a sentence, and `ctx.refuse` publishes `FrameKind::Error` (`runtime.rs:1307-1316`). | **B-12**: `TaskCtx::requested(already)` publishes `FrameKind::Changed` for the run (every watching pane re-reads, so `RelayView.cancels` shows it) and answers `RunReply::Failed { request: "cancel_run", message }`, which lands on the status line (`app/update.rs:285-287`). |
| **F-15** | Minor (T5) | D14: `StoreRequest::RelayView { item }` and its reply. | The pane drops a `RunActions` reply for another item by `actions.item` (`runs.rs:1203`); a bare `RelayView` has no item. A refusal must reach the status line, which only `Failed` does. | **B-13**: `StoreReply::RelayView { item, view: Box<RelayView> }`; `StoreReply::PermissionAnswered { permission }` on success; a refusal is `StoreReply::Failed { request: "answer_permission", message: refusal.to_string() }`. |
| **F-16** | Note (T3) | D4/D10: a lifted `Fenced` becomes `LeaseLost` through `is_fenced`. | On its way up it passes `walk_step`'s `fail_hard` (`engine.rs:3255-3262`), whose `transition_step` is not fenced. Today's fenced recorder takes the same road (`session` → `recorder.finish()?` → `Record(Fenced)` → `fail_hard`), so the lift widens nothing. | No change (I-6 is about `Cancelled`); recorded for the reviewer. |
| **F-17** | Note (T0) | D11: "the control lookup hands out `receiver.clone()` of the channel's own receiver". | A cloned receiver's `changed()` resolves at once for a value sent before the clone; `subscribe()` would not. `drive` reads `signal()` before every wait, so both are correct. | Kept as the plan says: `Parent` stores the receiver and the lookup clones it. |
| **F-18** | Minor (T0) | T0 tests "delete project cascades". | `delete_project_leaves_no_row_in_any_map` (`mem.rs:8114`) walks every `State` map; two new maps must join it. | T0 adds both asserts; MemStore prunes in `State::delete_project` (`mem.rs:3756-3771`). |
| **F-19** | Note (T5) | D14 "Digits". | `PermissionStrip`'s module is already `pub mod permission` (`ui/tabs/chat/mod.rs:28`), so `runs.rs` reaches it with no visibility change. `PermissionOption` is htui-agent's; core's `RelayOption` converts in `runs.rs` (orphan rule forbids a `From` between two foreign types). | `fn strip_options(&[RelayOption]) -> Vec<PermissionOption>` in `runs.rs`. |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-2): `RunRuntime::poll_commands_with(host, sink, live)`; `TuiRuns::poll_commands(backend, replies, live)`.
- **B-2** (F-3): `drive` selects the control against every pull, not only while parked.
- **B-3** (F-5): `Cancelled` at a `walked` site is answered as a preempt (`abandoned` + `PREEMPTED`).
- **B-4** (F-6): `pending_commands` returns terminal runs' pending rows on the box; `relay_view` hides them.
- **B-5** (F-7): `Shared.applying` de-duplicates the inline path and the poll.
- **B-6** (F-8): owned boxed lookups on `Kit` and `FakeOrchestrator`; `ask_policy`/`never_cancelled` for literals.
- **B-7** (F-9): actor columns are FKs (no cascade); MemStore checks them.
- **B-8** (F-10): `StepPermission` has no `owner` field.
- **B-9** (F-11): `open_permission` and `apply_permission` fence on the owner; answers and views need a live lease.
- **B-10** (F-12): the poll honours live chats and stays silent on that refusal.
- **B-11** (F-13): the post-cancel drain is bounded by `grace + 1 s`.
- **B-12** (F-14): "cancel requested" is a `Failed` reply plus a `Changed` frame.
- **B-13** (F-15): `RelayView { item, view }`, `PermissionAnswered { permission }`, refusals as `Failed`.
- **B-14**: the relay row's summary is `"<tool_kind>: <title>"` of the remembered `ToolCallEvent`; summary and labels are scrubbed together as one JSON document by the recorder's scrubber. On `Unmasked` the row keeps no summary and each label becomes its kind's text (`allow_once`, …): nothing unscrubbable is persisted (I-5, fail-closed), the request stays answerable, and the step's own `permission_request` row already carries the recorder's `scrub_residue` verdict. Option **ids** are never scrubbed: they are sent back verbatim.
- **B-15**: `drive` settles its session's leftover rows on **every** exit that opened one: `Cancelled` inside the cancel sequence (I-7 order), `Stale` best-effort on every other exit (a failure there is a `warn`, never the returned error).
- **B-16**: a `stale`/`cancelled` row read back while parked is a cancel with `Relay.grace` (`RELAY_GRACE`, 2 s); `applied` read back, or `apply_permission` answering `None` right after the row read `answered`, is `Fenced { step }` (D4).
- **B-17**: `drive_once` reads the control **before** `driver.start`: a cancel that reached the walk between sessions spawns nothing and answers `EngineError::Cancelled`.
- **B-18**: the engine's poll is the const `RELAY_POLL` (1 s, D8) — no `EngineParts` field; engine tests run `start_paused`. `Relay.poll` is a field so `drive`'s own tests shorten it.
- **B-19**: `RunRuntime` gains `cancel_grace` (const `CANCEL_GRACE` = 2 s, the TUI's `agent_worker::CANCEL_GRACE` value) and a `with_cancel_grace` builder for tests; `Signal::Cancel { grace }` carries it to `session.cancel`.
- **B-20**: `CancelRun` leaves `on_run`: a dedicated `cancel_run(ctx, run, live, existing)` runs D12. A `queued` or terminal run (read before anything is written) takes today's `on_run(…, Preempt::Never)` path — the queued CAS, or `cancel_enabled`'s refusal — and writes no row. `Preempt::Always` is removed.
- **B-21**: MemStore gains two `#[cfg(feature = "test-support")]` inherent readers, `relay_rows()` and `command_rows()`, for tests that must see rows no trait method lists.

### 0b. Escalations for the maintainer

None. Every finding above is settled inside the confirmed decisions: F-1 is a count, F-6 gives
D13's own "already-terminal → refused" sentence the input its selection clause omitted, and the
rest choose between shapes the plan leaves open.

---

## 1. Build order

| Task | Crates | Commits (each compiles) | Lane | Gate |
|---|---|---|---|---|
| T0 | core, store, agent (spies) | (1) red: ids, `model/relay.rs`, nine `WriteStore` methods, `RelayStore`, three `WorkerStore` methods, every implementor (MemStore and PgStore bodies are placeholders), 12 conformance cases + pins; (2) green: MemStore state and bodies, `delete_project` pruning, test-support readers | primary, alone | §12 G-T0 |
| T1 | store | (1) red: migration pins + `pg_criteria` cases; (2) green: `0011`, `pg/relay.rs`, `write.rs` delegations, `.sqlx` | primary (shared with T2) | G-T1 |
| T2 | agent | (1) red: `DriverError::Cancelled`, `record/relay.rs` types, `drive` as today's pump loop (relay and control ignored), `pump` over it, `tests/relay.rs`; (2) green: the relay loop, cancel sequence, docs | primary (shared with T1) | G-T2 |
| T5 | htui | (1) red: `StoreRequest`/`StoreReply` variants, `name()`, `try_serve` arms, the strip refactor (behaviour-preserving), pane + harness tests; (2) green: `RunsTab` relay state, digits, lines, snapshot | **worktree** `hr/MOD-42-t5` | G-T5 |
| — | — | merge T5 (`--no-ff`) after T1 and T2 committed; wave gate | primary, no lane running | G-W1 |
| T3 | orch, worker (Kit only) | (1) red: `EngineParts` fields, `EngineError::Cancelled`, helpers, every literal, `Kit`/`FakeOrchestrator` lookups, engine tests; (2) green: `drive_once` over `drive`, lifts, four catch-alls, docs | primary | G-T3 |
| T4 | worker, htui | (1) red: constants, `Shared` fields, `poll_commands_with` (no-op), `TuiRuns::poll_commands`, runtime tests (pinned refusal test replaced); (2) green runtime: `Walks` signal, `preempt_gracefully`, `cancel_run`, `walked` arms, poll; (3) green loops: store-loop arm, worker arm, `worker_walks` removed | primary | G-T4 |
| T6 | htui, docs | (1) Postgres end-to-end cases; (2) docs | primary | G-Final |

**Lane decision for T5 (justified).** T1 and T2 can share the primary tree: their crate gates
compile disjoint crates (`htui-store` depends only on `htui-core`; `htui-agent` likewise,
`Cargo.toml`s), cargo's build-directory lock serialises their invocations, and neither runs a
workspace-wide command until the wave gate. T5 cannot: `cargo test -p htui` compiles `htui-store`,
`htui-agent`, `htui-orch` and `htui-worker`, so a half-edited `record.rs` (T2) or `pg/write.rs` (T1)
breaks its build mid-lane, and its green would be about a tree that never existed (team memory
"parallel unit is a crate that compiles"). T5 needs only T0's surface (MemStore relay methods,
model types), so it branches from T0's green commit into its own worktree with its own `target/`
(≈ 10 GB; 61 GB free at HEAD). Only T1 touches Postgres in the wave.

---

## 2. Shared code shapes

Everything in this section crosses a task boundary and is settled here. A lane that believes a
shape here is wrong stops and reports; it does not edit another lane's crate.

### 2.1 Ids (`htui-core/src/model/ids.rs`, T0)

Three entries appended to the `id_newtype!` list (`ids.rs:76-117`), after `RequirementId`:

```rust
    /// `step_permission.id` (MOD-42 plan D1): one parked stage-3 permission request.
    PermissionId,
    /// `run_command.id` (MOD-42 plan D1): one requested command on a run (today: cancel).
    RunCommandId,
    /// `step_permission.session` (MOD-42 plan D1): one driven agent session, minted per
    /// `drive_once` (a candidate's session, each judge call), so request ids that repeat across
    /// sessions never collide (PRD MVP item 3).
    RelaySessionId,
```

`model/mod.rs:115` `pub use ids::{…}` gains the three (rustfmt order).

### 2.2 `htui-core/src/model/relay.rs` (new, T0)

`model/mod.rs` declares `pub mod relay;` (after `pub mod quota;`, so `str_enum!` is in scope) and
re-exports every pub item below. Four `check_enum` tests join `model/mod.rs`'s `mod tests`
(`permission_status_matches_check_list`, `run_command_status_matches_check_list`,
`run_command_kind_matches_check_list`, `relay_option_kind_round_trips`).

```rust
//! The permission and control relay's rows and outcomes (MOD-42 plan D1-D5, D12, D13;
//! `0011_permission_relay.sql`). Neither table is mirrored (plan OQ-4).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{BoxId, PermissionId, RelaySessionId, RunCommandId, RunId, StepId, UserId};

str_enum!(
    /// `step_permission.status` (MOD-42 plan D1), in `CHECK` order.
    PermissionStatus {
        /// Parked; any store client may answer it (D3).
        Pending => "pending",
        /// A client answered it; the executor has not applied it yet (D4).
        Answered => "answered",
        /// The executor applied the answer to the live session and recorded it (D4).
        Applied => "applied",
        /// Its session was cancelled gracefully while it was open (I-7, D5).
        Cancelled => "cancelled",
        /// Its session ended, or a newer session of the step opened a row (D5).
        Stale => "stale",
    }
);

str_enum!(
    /// `run_command.kind` (MOD-42 plan D1). A follow-up command adds a kind later (PRD Q9).
    RunCommandKind {
        /// Cancel the run (D12).
        Cancel => "cancel",
    }
);

str_enum!(
    /// `run_command.status` (MOD-42 plan D1), in `CHECK` order.
    RunCommandStatus {
        /// Written; not yet applied by the run's executor (OQ-2: no timeout).
        Pending => "pending",
        /// The executor applied it.
        Applied => "applied",
        /// The executor could not apply it; `resolution` says why (a run already terminal).
        Refused => "refused",
    }
);

str_enum!(
    /// A relayed option's kind: `htui_agent::event::PermissionOptionKind`'s four values, as text
    /// (htui-core cannot depend on htui-agent; plan D6-ids). A UI hint only.
    RelayOptionKind {
        /// Allow this call only.
        AllowOnce => "allow_once",
        /// Allow this call and let the agent remember it (D16: htui persists nothing).
        AllowAlways => "allow_always",
        /// Reject this call only.
        RejectOnce => "reject_once",
        /// Reject this call and let the agent remember it.
        RejectAlways => "reject_always",
    }
);

/// One option of a relayed request, as `step_permission.options[]` stores it (`{id,label,kind}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayOption {
    /// The agent's option id, sent back verbatim; never scrubbed.
    pub id: String,
    /// The label, scrubbed by the executor's recorder before it is stored (I-5).
    pub label: String,
    /// The kind.
    pub kind: RelayOptionKind,
}

/// A row of `step_permission`, without `owner` (blueprint B-8: a reader has no use for another
/// process's liveness token, as `Run` has no `lease_owner`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepPermission {
    /// `step_permission.id`.
    pub id: PermissionId,
    /// `step_permission.run_id`.
    pub run_id: RunId,
    /// `step_permission.run_step_id`.
    pub run_step_id: StepId,
    /// `step_permission.session`.
    pub session: RelaySessionId,
    /// `step_permission.request_id`: the transport's id, unique per session.
    pub request_id: String,
    /// `step_permission.tool_call_id`.
    pub tool_call_id: Option<String>,
    /// `step_permission.summary`: `"<tool_kind>: <title>"`, scrubbed; `None` when the transport
    /// named no call or the scrubber refused it (B-14).
    pub summary: Option<String>,
    /// `step_permission.options`, in the agent's order.
    pub options: Vec<RelayOption>,
    /// `step_permission.status`.
    pub status: PermissionStatus,
    /// `step_permission.option_id`: set by the answer.
    pub option_id: Option<String>,
    /// `step_permission.answered_by`.
    pub answered_by: Option<UserId>,
    /// `step_permission.answered_box`.
    pub answered_box: Option<BoxId>,
    /// `step_permission.created_at`, the store's clock (I-4).
    pub created_at: DateTime<Utc>,
    /// `step_permission.answered_at`.
    pub answered_at: Option<DateTime<Utc>>,
    /// `step_permission.resolved_at`: set by `applied`, `cancelled` and `stale`.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Arguments of `WriteStore::open_permission`: the row as the executor parks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPermission {
    /// Minted by the executor (`PermissionId::new()`).
    pub id: PermissionId,
    /// The run.
    pub run_id: RunId,
    /// The step; must be a step of `run_id`.
    pub run_step_id: StepId,
    /// The driven session.
    pub session: RelaySessionId,
    /// The transport's request id.
    pub request_id: String,
    /// The gated tool call, when the transport named one.
    pub tool_call_id: Option<String>,
    /// Scrubbed summary (B-14).
    pub summary: Option<String>,
    /// Scrubbed options.
    pub options: Vec<RelayOption>,
    /// The executor's lease owner at park time (I-2).
    pub owner: uuid::Uuid,
}

/// What an applied answer chose (`WriteStore::apply_permission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionChoice {
    /// The option id the answer named.
    pub option_id: String,
}

/// `WriteStore::answer_permission`'s outcome (D3, OQ-1: a loser is told, not persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerOutcome {
    /// This answer won the compare-and-set.
    Answered,
    /// Nothing was written; the request's actual state.
    Refused(AnswerRefusal),
}

/// Why an answer wrote nothing (D3). The `Display` text is what the status line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnswerRefusal {
    /// Another answer won first.
    #[error("{}", ALREADY_ANSWERED)]
    Answered,
    /// Another answer won and the executor applied it.
    #[error("{}", ALREADY_APPLIED)]
    Applied,
    /// The request's session was cancelled.
    #[error("{}", REQUEST_CANCELLED)]
    Cancelled,
    /// The request's session ended or was superseded.
    #[error("{}", REQUEST_STALE)]
    Stale,
    /// The row's owner no longer holds a live lease on the run (adopted, released or expired).
    #[error("{}", EXECUTOR_GONE)]
    ExecutorGone,
    /// The option id is not one the request offered.
    #[error("{}", NOT_OFFERED)]
    NotOffered,
}

/// [`AnswerRefusal::Answered`].
pub const ALREADY_ANSWERED: &str = "this permission request was already answered";
/// [`AnswerRefusal::Applied`].
pub const ALREADY_APPLIED: &str = "this permission request was already answered and applied";
/// [`AnswerRefusal::Cancelled`].
pub const REQUEST_CANCELLED: &str = "this permission request was cancelled with its session";
/// [`AnswerRefusal::Stale`].
pub const REQUEST_STALE: &str = "this permission request belongs to a session that has ended";
/// [`AnswerRefusal::ExecutorGone`].
pub const EXECUTOR_GONE: &str =
    "the process that asked no longer holds the run; its request cannot be answered";
/// [`AnswerRefusal::NotOffered`].
pub const NOT_OFFERED: &str = "that option was not offered by this permission request";

/// `WriteStore::request_cancel`'s outcome: the partial unique index admits one pending cancel per
/// run (D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelRequest {
    /// A new row.
    Inserted(RunCommandId),
    /// A pending cancel already existed; nothing was written.
    AlreadyPending(RunCommandId),
}

/// A row of `run_command`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCommand {
    /// `run_command.id`.
    pub id: RunCommandId,
    /// `run_command.run_id`.
    pub run_id: RunId,
    /// `run_command.kind`.
    pub kind: RunCommandKind,
    /// `run_command.issued_by`.
    pub issued_by: UserId,
    /// `run_command.issued_box`.
    pub issued_box: BoxId,
    /// `run_command.status`.
    pub status: RunCommandStatus,
    /// `run_command.resolution`: the refusal's sentence, or `None`.
    pub resolution: Option<String>,
    /// `run_command.issued_at`, the store's clock.
    pub issued_at: DateTime<Utc>,
    /// `run_command.resolved_at`.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// What the Runs pane shows for one item (D14): its live pending requests and its runs with a
/// pending cancel. Empty offline (D14, OQ-4).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RelayView {
    /// `pending` rows whose `owner` is the run's live lease owner, `(created_at, id)` order.
    pub permissions: Vec<StepPermission>,
    /// Non-terminal runs of the item with a `pending` cancel, ascending.
    pub cancels: Vec<RunId>,
}
```

### 2.3 `WriteStore` additions (`htui-core/src/store/traits.rs`, T0)

Appended at the end of `pub trait WriteStore` (after `reconfirm`, `:1544-1548`), under a
`// -- MOD-42: the permission and control relay (plan D1-D5, D12-D14)` comment; the module doc gains
a **MOD-42** paragraph ("nine writer methods, two tables; `RelayStore` and `WorkerStore` forward
seven of them"). Import the new model types into the `use crate::model::{…}` list.

```rust
    /// D1, D5, B-9: parks one stage-3 request. In one transaction: the step and its run's lease
    /// (`NotFound { entity: "run_step" }`; `Constraint` when the step is not `open.run_id`'s;
    /// `Fenced { step }` when `run.lease_owner` is not `open.owner`), then every `pending` or
    /// `answered` row of the same step from **another session** moves to `stale` (D5), then the
    /// insert. Answers `open.id`. `created_at` is the store's clock (I-4).
    ///
    /// # Errors
    /// The three above; `Constraint` for a repeated id or a repeated `(session, request_id)`.
    async fn open_permission(&self, open: OpenPermission) -> Result<PermissionId>;

    /// One row by id; `None` for an id no row has.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn permission(&self, id: PermissionId) -> Result<Option<StepPermission>>;

    /// D4: compare-and-set `answered → applied`, only while `run.lease_owner = owner` (B-9: owner
    /// only, as a step fence). `Some(choice)` = applied now; `None` = not `answered`, not
    /// `owner`'s row, or the run's lease is not `owner`'s. `resolved_at` is the store's clock.
    ///
    /// # Errors
    /// `NotFound { entity: "step_permission" }` for an unknown id, told apart by one re-read.
    async fn apply_permission(&self, id: PermissionId, owner: Uuid)
    -> Result<Option<PermissionChoice>>;

    /// D5, I-7: every `pending` or `answered` row of `session` moves to `to` (`Cancelled` or
    /// `Stale`), `resolved_at` the store's clock. Answers how many moved.
    ///
    /// # Errors
    /// `Constraint` for any other `to`, before anything is written.
    async fn settle_permissions(&self, session: RelaySessionId, to: PermissionStatus)
    -> Result<u64>;

    /// D12: one `pending` cancel per run. Writes `(id, run, 'cancel', user, box_id)` unless a
    /// pending cancel exists. Never reads the run's status: the caller decides (B-20).
    ///
    /// # Errors
    /// `NotFound { entity: "run" }`; `Constraint` for an unknown user or box (B-7).
    async fn request_cancel(&self, run: RunId, user: UserId, box_id: BoxId)
    -> Result<CancelRequest>;

    /// D13, B-4: the `pending` commands this process applies, `(issued_at, id)` order: runs whose
    /// `lease_owner = owner`; runs executing on `box_id` whose lease is free or expired by the
    /// store's clock and whose status is `running` or `awaiting_approval`; and runs executing on
    /// `box_id` that are already terminal (the caller refuses those with their status).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn pending_commands(&self, owner: Uuid, box_id: BoxId) -> Result<Vec<RunCommand>>;

    /// D12, D13: compare-and-set `pending → to` (`Applied` or `Refused`), with `resolution`,
    /// `resolved_at` the store's clock. `Ok(false)` = not pending any more (I-3).
    ///
    /// # Errors
    /// `Constraint` for `to = Pending`, before anything is read; `NotFound { entity:
    /// "run_command" }` for an unknown id.
    async fn resolve_command(
        &self,
        id: RunCommandId,
        to: RunCommandStatus,
        resolution: Option<String>,
    ) -> Result<bool>;

    /// D14: the item's `pending` requests whose owner holds the run's lease live by the store's
    /// clock, and its non-terminal runs with a `pending` cancel. A read on `WriteStore` by the
    /// `command_runs` precedent (`:1318-1331`): neither table is mirrored (OQ-4).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn relay_view(&self, item: ItemId) -> Result<RelayView>;

    /// D3: compare-and-set `pending → answered`, only while `option_id` is one of the row's
    /// options and the row's `owner` holds the run's lease live by the store's clock. Never
    /// touches `run`, the lease or `session_event` (I-1). A loser is `Refused` with the actual
    /// state, decided by one re-read in this order: status (not `pending`), then `NotOffered`,
    /// then `ExecutorGone`.
    ///
    /// # Errors
    /// `NotFound { entity: "step_permission" }`; `Constraint` for an unknown user or box.
    async fn answer_permission(
        &self,
        id: PermissionId,
        option_id: &str,
        user: UserId,
        box_id: BoxId,
    ) -> Result<AnswerOutcome>;
```

### 2.4 `RelayStore` and the `WorkerStore` additions (`htui-core/src/store/worker.rs`, T0)

`store/mod.rs` re-exports `RelayStore` beside the other three. The module doc's "`WorkerStore` is
42 methods" (`worker.rs:15-17`) becomes "45 methods: … and MOD-42's three command methods;
`RelayStore` is four". `RelayStore` sits after `RecorderStore` (`:40-62`).

```rust
/// What `htui_agent::record::drive` writes and reads while a request is parked (MOD-42 plan D2,
/// D6). Separate from [`RecorderStore`] so `drive` can take a recorder store and a relay store as
/// two type parameters: the spies implement only the first (plan probe A1b).
pub trait RelayStore: Send + Sync {
    /// [`WriteStore::open_permission`].
    fn open_permission(&self, open: OpenPermission)
    -> impl Future<Output = Result<PermissionId>> + Send;
    /// [`WriteStore::permission`].
    fn permission(&self, id: PermissionId)
    -> impl Future<Output = Result<Option<StepPermission>>> + Send;
    /// [`WriteStore::apply_permission`].
    fn apply_permission(&self, id: PermissionId, owner: Uuid)
    -> impl Future<Output = Result<Option<PermissionChoice>>> + Send;
    /// [`WriteStore::settle_permissions`].
    fn settle_permissions(&self, session: RelaySessionId, to: PermissionStatus)
    -> impl Future<Output = Result<u64>> + Send;
}

pub trait WorkerStore: RecorderStore + RelayStore {
    // … the 42 of today, unchanged …
    // -- MOD-42 (plan D2, D12, D13): the command side
    /// [`WriteStore::request_cancel`].
    fn request_cancel(&self, run: RunId, user: UserId, box_id: BoxId)
    -> impl Future<Output = Result<CancelRequest>> + Send;
    /// [`WriteStore::pending_commands`].
    fn pending_commands(&self, owner: Uuid, box_id: BoxId)
    -> impl Future<Output = Result<Vec<RunCommand>>> + Send;
    /// [`WriteStore::resolve_command`].
    fn resolve_command(&self, id: RunCommandId, to: RunCommandStatus, resolution: Option<String>)
    -> impl Future<Output = Result<bool>> + Send;
}
```

Implementors (fact-checked complete, plan D2): `impl RelayStore` and the three `WorkerStore`
bodies for `MemStore` (this file), `PgStore` and `Writer` (`htui-store/src/worker.rs`); every body
UFCS, e.g. `WriteStore::open_permission(self, open).await`. `WriteStore` forwarding (nine methods)
for `Writer` (`htui-store/src/writer.rs:310`, `match self { Self::Memory(store) => store.x(…).await,
Self::Online(pg) => pg.x(…).await }`), `UsageSpy` (`htui-agent/src/conformance.rs:741`,
`self.inner.x(…).await`) and `SpyStore` (`htui-agent/tests/recorder.rs:429`, same). The spies get
no `RelayStore`. The compile-first module test gains `is_relay::<MemStore>()`.

### 2.5 `htui-agent`: `drive`, `Relay`, `Control`, `Signal`, `NoRelay` (T2; T0 needs none of it)

New file `htui-agent/src/record/relay.rs`, declared `mod relay;` inside `record.rs` (a child
module sees the recorder's private `flush` (`record.rs:1107`) and `scrubber` field (`:379`)),
re-exported `pub use relay::{…}` from `record.rs` and from `lib.rs:178`'s `pub use record::{…}`.

```rust
//! MOD-42 plan D6: one turn of a session, with the permission relay and the run's control.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{
    OpenPermission, PermissionChoice, PermissionId, PermissionStatus, RelayOption,
    RelayOptionKind, RelaySessionId, RunId, StepId, StepPermission,
};
use htui_core::store::{Result as StoreResult, StoreError};
use tokio::sync::watch;
use uuid::Uuid;

/// D8: how often a parked request's row is read. One primary-key read per tick, only while parked.
pub const RELAY_POLL: Duration = Duration::from_secs(1);
/// B-16: the grace a cancel `drive` discovers itself (a `stale`/`cancelled` row) is given.
pub const RELAY_GRACE: Duration = Duration::from_secs(2);
/// B-11: how long past the grace the post-cancel drain may run.
const DRAIN_SLACK: Duration = Duration::from_secs(1);

/// What a walk is asked to do (D10, D11). `Copy`, read with [`Control::signal`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Signal {
    /// Carry on.
    #[default]
    Run,
    /// Stop gracefully: answer parked requests `cancelled`, `session.cancel(grace)`, drain.
    Cancel {
        /// The window `AgentSession::cancel` is given.
        grace: Duration,
    },
}

impl Signal {
    /// Whether this is [`Signal::Cancel`].
    #[must_use]
    pub const fn is_cancel(self) -> bool { matches!(self, Self::Cancel { .. }) }
}

/// One walk's view of its run's [`Signal`] (D10): a `watch` receiver, or none ("never").
#[derive(Debug, Clone)]
pub struct Control(Option<watch::Receiver<Signal>>);

impl Control {
    /// A control no signal ever reaches: `pump`, and every engine no runtime cancels.
    #[must_use]
    pub const fn never() -> Self { Self(None) }

    /// Over a receiver of the run's channel.
    #[must_use]
    pub const fn new(receiver: watch::Receiver<Signal>) -> Self { Self(Some(receiver)) }

    /// The current signal. R-8: `changed()` is edge-triggered, so every wait reads this first.
    #[must_use]
    pub fn signal(&self) -> Signal { self.0.as_ref().map_or(Signal::Run, |rx| *rx.borrow()) }

    /// Resolves when the value changes. Pending for ever for [`Control::never`] and once the
    /// sender is gone (a dropped sender is not a cancel).
    pub async fn changed(&mut self) {
        match &mut self.0 {
            Some(rx) if rx.changed().await.is_ok() => {}
            _ => std::future::pending::<()>().await,
        }
    }
}

/// A fresh channel at [`Signal::Run`] and a control over it (tests, the fake orchestrator).
#[must_use]
pub fn control_channel() -> (watch::Sender<Signal>, Control) {
    let (sender, receiver) = watch::channel(Signal::Run);
    (sender, Control::new(receiver))
}

/// What `drive` needs to relay a stage-3 request (D6). Built per session by the engine.
pub struct Relay<'a, R: htui_core::store::RelayStore> {
    /// The executor's store.
    pub store: &'a R,
    /// The executor's lease owner (`EngineParts::owner`), the row's `owner` (I-2).
    pub owner: Uuid,
    /// The run.
    pub run: RunId,
    /// The step the session drives.
    pub step: StepId,
    /// This session (minted per `drive_once`).
    pub session: RelaySessionId,
    /// The agent's policy, stages 1-2 (D9).
    pub policy: &'a crate::driver::PermissionPolicy,
    /// Parked-row poll interval: [`RELAY_POLL`] in production (D8, B-18).
    pub poll: Duration,
    /// B-16's grace.
    pub grace: Duration,
    /// The instant source for the rows `htui` authors (`record_permission_answer`'s `at`).
    pub now: &'a (dyn Fn() -> DateTime<Utc> + Sync),
}
// `Debug` hand-written: every field but `store`, `policy` and `now`; `finish_non_exhaustive()`.

/// `pump`'s relay type (D6): uninhabited, so `None::<&Relay<'_, NoRelay>>` names a relay that
/// cannot exist. Every body is `match *self {}`.
#[derive(Debug, Clone, Copy)]
pub enum NoRelay {}
impl htui_core::store::RelayStore for NoRelay { /* four `async fn …(&self, …) { match *self {} }` */ }

/// D6: one turn of `session` into `recorder`, ending at its `done`, a graceful cancel, or an error.
///
/// Without a relay it behaves exactly as `pump` always has: a `permission_request` is recorded
/// and the pull goes on, so the transport raises "is parked". With one: stage 1-2 policy answers
/// at once (`by: policy`); stage 3 flushes the recorder, opens a scrubbed row (I-5) and polls it
/// at `relay.poll` until it is answered (applied under the lease, D4, echoed `by: user`), read
/// back `stale`/`cancelled` (a cancel), or `control` signals a cancel. The control is read at the
/// top of every iteration and selected against every pull (B-2).
///
/// # Errors
/// [`DriverError::Cancelled`] after a graceful cancel (I-7); [`DriverError::Store`]
/// `(Fenced { step })` when an answer can no longer be applied under the lease (D4);
/// [`DriverError::Closed`] for a stream that ended without `done`; the recorder's failures.
pub async fn drive<S: htui_core::store::RecorderStore, R: htui_core::store::RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
) -> Result<DoneEvent, DriverError>;
```

`pump` (`record.rs:1854-1873`) keeps its signature and doc, and its body becomes
`drive(session, recorder, None::<&Relay<'_, NoRelay>>, &mut Control::never()).await`. All 26 call
sites are untouched (D6). The two stale "eighteen call sites" sentences (`record.rs:165`, `:1893`)
say "its call sites".

`DriverError` (`htui-agent/src/error.rs`) gains, after `Unsupported`:

```rust
    /// MOD-42 plan D6: the session was cancelled on request (a run cancel or a promote's
    /// preempt) and closed gracefully: every parked request was answered `cancelled` and
    /// recorded (I-7). Not a transport failure; the engine lifts it to `EngineError::Cancelled`
    /// and settles nothing (I-6).
    #[error("the session was cancelled on request")]
    Cancelled,
```

### 2.6 `htui-orch` (T3)

`engine.rs`, beside `DriverFor` (`:271-272`):

```rust
/// MOD-42 plan D9: the permission policy of an agent, for the session spec and the relay.
/// `+ Sync` is load-bearing: walks are spawned (`a_dispatch_future_is_send`, probe A2).
pub type PolicyFor<'a> = &'a (dyn Fn(AgentId) -> PermissionPolicy + Sync);

/// MOD-42 plan D10: a run's [`Control`], looked up per session (the run's walk token may not exist
/// when the engine is built, `runtime.rs:1849` vs `:1860`).
pub type ControlFor<'a> = &'a (dyn Fn(RunId) -> Control + Sync);

/// [`PolicyFor`] for an engine with no registry (tests, harness literals): every agent asks.
#[must_use]
pub fn ask_policy(_agent: AgentId) -> PermissionPolicy { PermissionPolicy::default() }

/// [`ControlFor`] for an engine no cancel reaches.
#[must_use]
pub fn never_cancelled(_run: RunId) -> Control { Control::never() }
```

`EngineParts` gains two fields after `driver` (and `Debug` stays `finish_non_exhaustive`); the
struct doc's "sixteen fields" (`:390`) and `fake_parts`' (`:6340`) say "eighteen":

```rust
    /// MOD-42 plan D9: each agent's own permission policy.
    pub policy: PolicyFor<'a>,
    /// MOD-42 plan D10: each run's control.
    pub control: ControlFor<'a>,
```

`command.rs` `EngineError` gains, after `LeaseLost` (`:414-417`):

```rust
    /// MOD-42 plan D10, I-6: the walk was asked to stop gracefully (a cancel, or a promote's
    /// preempt). Its session was cancelled with grace and every parked request answered
    /// `cancelled`; nothing was settled, so `cancel_leased` (or the promotion) owns every
    /// terminal status.
    #[error("run {run}: the walk was cancelled while it drove a session")]
    Cancelled {
        /// The run whose walk stopped.
        run: RunId,
    },
```

`lib.rs`'s `pub use engine::{…}` (`:49-52`) adds `ControlFor, PolicyFor, ask_policy,
never_cancelled`.

### 2.7 `htui-worker` (T4; T3 touches only `Kit`)

`runtime.rs`, replacing `worker_walks` (`:95-101`):

```rust
/// MOD-42 plan D12 step 6: a cancel this process cannot apply now; the run's executor does.
pub const CANCEL_REQUESTED: &str = "cancel requested: the run's executor applies it";
/// The same, when a cancel of the run was already pending.
pub const CANCEL_ALREADY_REQUESTED: &str =
    "a cancel is already requested: the run's executor applies it";

/// MOD-42 OQ-3: `p` on a step the box's worker walks. Promotion hands a live session to this
/// TUI's chat, which cannot cross processes.
#[must_use]
pub fn promote_needs_the_walker(run: RunId) -> String {
    format!("the worker on this box is walking run {run}; a live step is promoted only by the process that walks it")
}

/// MOD-42 plan D11, B-19: the window a graceful preempt gives a session's cancel.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);
```

`worker.rs`, beside `WORKER_POLL`:

```rust
/// MOD-42 plan D13: how often pending run commands are read (a `const`, so `WorkerConfig`
/// literals stay as they are).
pub const COMMAND_POLL: Duration = Duration::from_secs(1);
```

`RunRuntime` gains:

```rust
    /// B-19: the grace [`CANCEL_GRACE`] stands for, fixed by a test.
    #[must_use]
    pub fn with_cancel_grace(mut self, grace: Duration) -> Self;

    /// MOD-42 plan D13 (B-1, B-10): one command poll, the `sweep_with` shape: sync, one tracked
    /// task under an in-flight flag so ticks never overlap; nothing without a writer or after
    /// close. Each pending row this process may apply (`pending_commands(owner, box)`), not
    /// already being applied (B-5), runs as an internal cancel (D12 steps 3-5, no second row).
    pub fn poll_commands_with(&mut self, host: &H, sink: &P, live: LiveChats);
```

`lib.rs:49-52` re-exports `CANCEL_ALREADY_REQUESTED, CANCEL_GRACE, CANCEL_REQUESTED,
promote_needs_the_walker` and drops `worker_walks`; `worker::COMMAND_POLL` is reached as
`htui_worker::worker::COMMAND_POLL`.

`crates/htui/src/run_worker.rs` `trait TuiRuns` (`:92-139`) gains:

```rust
    /// MOD-42 plan D13: one command-poll tick over [`RunRuntime::poll_commands_with`].
    fn poll_commands(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        live: &LiveChats,
    );
// impl: self.poll_commands_with(backend, &TuiReplies(replies.clone()), live.clone());
```

### 2.8 `htui` store requests and replies (T5)

`StoreRequest` (`store_worker.rs:105`), after `RunActions` (`:724`):

```rust
    /// MOD-42 plan D14: the item's pending permission requests and pending cancels. Served from
    /// the writer; offline, an **empty** view — never `Failed` or `Unreachable` (OQ-4).
    RelayView {
        /// The item the Runs pane shows.
        item: ItemId,
    },
    /// MOD-42 plan D14: one answer to a pending request (D3). Refused offline with
    /// `DATABASE_UNREACHABLE`.
    AnswerPermission {
        /// The request.
        permission: PermissionId,
        /// The chosen option's id.
        option_id: String,
    },
```

`name()` (`:826-935`): `"relay_view"`, `"answer_permission"` after `"run_actions"`.
`StoreReply` (`:937`), after `RunActions` (`:1148`):

```rust
    /// Answer to [`StoreRequest::RelayView`] for `item` (B-13).
    RelayView {
        /// The item asked about.
        item: ItemId,
        /// What it holds.
        view: Box<RelayView>,
    },
    /// [`StoreRequest::AnswerPermission`] won its compare-and-set; a refusal is
    /// [`StoreReply::Failed`] with the refusal's sentence (B-13).
    PermissionAnswered {
        /// The answered request.
        permission: PermissionId,
    },
```

Counts: `StoreRequest` 91 → 93, `StoreReply` 52 → 54 (D15; no test pins them).

### 2.9 Migration `htui-store/migrations/0011_permission_relay.sql` (T1), in full

```sql
-- 0011_permission_relay.sql - MOD-42 (plan D1, D3, D4, D5, D12, D13).
-- Forward-only (R-STO-5).
--
-- The permission and control relay. step_permission holds one row per stage-3 permission request
-- an executor parked: any store client answers it with a compare-and-set, and the executor that
-- parked it applies the answer to its live session and records the permission_answer event
-- itself (session_event stays single-writer). run_command holds one row per requested command on
-- a run (today only cancel), applied by whichever process holds or adopts the run on its
-- executing box. Every time is clock_timestamp(), never a box clock. Both tables go with their
-- run (ON DELETE CASCADE), which delete_project relies on. Neither is mirrored (plan OQ-4), but
-- schema_version becomes 11, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

CREATE TABLE step_permission (
    id            UUID        PRIMARY KEY,
    run_id        UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    run_step_id   UUID        NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    session       UUID        NOT NULL,
    request_id    TEXT        NOT NULL,
    tool_call_id  TEXT,
    summary       TEXT,
    options       JSONB       NOT NULL,
    owner         UUID        NOT NULL,
    status        TEXT        NOT NULL DEFAULT 'pending',
    option_id     TEXT,
    answered_by   UUID        REFERENCES app_user(id),
    answered_box  UUID        REFERENCES box(id),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    answered_at   TIMESTAMPTZ,
    resolved_at   TIMESTAMPTZ,
    CONSTRAINT chk_step_permission_status
        CHECK (status IN ('pending', 'answered', 'applied', 'cancelled', 'stale')),
    CONSTRAINT chk_step_permission_options CHECK (jsonb_typeof(options) = 'array'),
    CONSTRAINT chk_step_permission_answer
        CHECK (status NOT IN ('answered', 'applied')
               OR (option_id IS NOT NULL AND answered_by IS NOT NULL
                   AND answered_box IS NOT NULL AND answered_at IS NOT NULL)),
    CONSTRAINT chk_step_permission_resolved
        CHECK ((status IN ('applied', 'cancelled', 'stale')) = (resolved_at IS NOT NULL)),
    CONSTRAINT uq_step_permission_request UNIQUE (session, request_id)
);
CREATE INDEX idx_step_permission_open ON step_permission (run_id)
    WHERE status IN ('pending', 'answered');
CREATE INDEX idx_step_permission_step ON step_permission (run_step_id)
    WHERE status IN ('pending', 'answered');

CREATE TABLE run_command (
    id           UUID        PRIMARY KEY,
    run_id       UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL,
    issued_by    UUID        NOT NULL REFERENCES app_user(id),
    issued_box   UUID        NOT NULL REFERENCES box(id),
    status       TEXT        NOT NULL DEFAULT 'pending',
    resolution   TEXT,
    issued_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    resolved_at  TIMESTAMPTZ,
    CONSTRAINT chk_run_command_kind CHECK (kind IN ('cancel')),
    CONSTRAINT chk_run_command_status CHECK (status IN ('pending', 'applied', 'refused')),
    CONSTRAINT chk_run_command_resolved CHECK ((status = 'pending') = (resolved_at IS NULL))
);
CREATE UNIQUE INDEX uq_run_command_pending ON run_command (run_id, kind)
    WHERE status = 'pending';
```

`idx_step_permission_step` serves `open_permission`'s staling `UPDATE` (by step); the plan's
`(run_id)` partial index serves `relay_view` and the answer's re-read joins.

### 2.10 PgStore SQL (T1; bodies in a new `htui-store/src/pg/relay.rs`)

`pg/mod.rs:7-10` gains `mod relay;`. `relay.rs` holds `pub(super) async fn x(store: &PgStore, …)`
bodies; `write.rs`'s `impl WriteStore for PgStore` gets nine one-line delegations (replacing T0's
placeholders). Every status/kind binds through the `str_enum` sqlx types
(`status AS "status: PermissionStatus"`), every option list through
`sqlx::types::Json<Vec<RelayOption>>`; every miss re-reads, the `take_lease` shape
(`write.rs:4035-4081`). Errors through `map_sqlx` (23xxx → `Constraint`, `error.rs:42-46`).

**`open_permission`** — one transaction:

```sql
-- 1. existence, membership and the fence, the run row-locked so an adoption cannot commit between
--    this check and the insert (MOD-41 `step_fence` shape)
SELECT s.run_id, r.lease_owner AS "lease_owner?"
  FROM run_step s JOIN run r ON r.id = s.run_id
 WHERE s.id = $1
   FOR SHARE OF r
-- none → NotFound { run_step }; s.run_id <> open.run_id → Constraint
--   ("step_permission.run_step_id `{step}` is not a step of run `{run}`");
-- lease_owner IS DISTINCT FROM open.owner → Fenced { step }
-- 2. D5: older sessions of the step go stale
UPDATE step_permission SET status = 'stale', resolved_at = clock_timestamp()
 WHERE run_step_id = $1 AND session <> $2 AND status IN ('pending', 'answered')
-- 3. the row (unique violations → Constraint)
INSERT INTO step_permission (id, run_id, run_step_id, session, request_id, tool_call_id,
                             summary, options, owner)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
```

**`permission`**:

```sql
SELECT id, run_id, run_step_id, session, request_id, tool_call_id, summary,
       options AS "options: Json<Vec<RelayOption>>", status AS "status: PermissionStatus",
       option_id, answered_by, answered_box, created_at, answered_at, resolved_at
  FROM step_permission WHERE id = $1
```

**`apply_permission`** (owner only, B-9):

```sql
UPDATE step_permission p
   SET status = 'applied', resolved_at = clock_timestamp()
  FROM run r
 WHERE p.id = $1 AND p.status = 'answered' AND p.owner = $2
   AND r.id = p.run_id AND r.lease_owner = $2
RETURNING p.option_id AS "option_id!"
-- zero rows: SELECT 1 FROM step_permission WHERE id = $1 → none: NotFound, else Ok(None)
```

**`settle_permissions`** (`to` checked in Rust first):

```sql
UPDATE step_permission SET status = $2, resolved_at = clock_timestamp()
 WHERE session = $1 AND status IN ('pending', 'answered')
-- rows_affected() is the answer
```

**`answer_permission`**:

```sql
UPDATE step_permission p
   SET status = 'answered', option_id = $2, answered_by = $3, answered_box = $4,
       answered_at = clock_timestamp()
 WHERE p.id = $1
   AND p.status = 'pending'
   AND p.options @> jsonb_build_array(jsonb_build_object('id', $2::text))
   AND p.owner = (SELECT r.lease_owner FROM run r
                   WHERE r.id = p.run_id AND r.lease_expires_at > clock_timestamp())
-- one row → Answered. Zero rows → re-read:
SELECT p.status AS "status: PermissionStatus",
       p.options @> jsonb_build_array(jsonb_build_object('id', $2::text)) AS "offered!",
       COALESCE(r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp(), false)
           AS "live!"
  FROM step_permission p JOIN run r ON r.id = p.run_id
 WHERE p.id = $1
-- none → NotFound { step_permission }; status ≠ pending → Refused(that status: Answered,
-- Applied, Cancelled, Stale); !offered → Refused(NotOffered); !live → Refused(ExecutorGone);
-- all three true (a concurrent change undone before the re-read) → Refused(ExecutorGone)
```

Two concurrent answers: the second waits on the row lock, re-evaluates `status = 'pending'`
against the committed row and matches nothing (READ COMMITTED), so exactly one wins (I-3).

**`request_cancel`** (up to three attempts, for a row resolved between the two statements):

```sql
INSERT INTO run_command (id, run_id, kind, issued_by, issued_box)
VALUES ($1, $2, 'cancel', $3, $4)
ON CONFLICT (run_id, kind) WHERE status = 'pending' DO NOTHING
RETURNING id
-- one row → Inserted(id). None →
SELECT id FROM run_command WHERE run_id = $1 AND kind = 'cancel' AND status = 'pending'
-- Some → AlreadyPending(id); None → retry; after three → Constraint(concurrent_write…).
-- A Constraint from the INSERT re-reads `SELECT 1 FROM run WHERE id = $1`: none → NotFound { run },
-- else the Constraint (unknown user or box) stands.
```

**`pending_commands`** (B-4):

```sql
SELECT c.id, c.run_id, c.kind AS "kind: RunCommandKind", c.issued_by, c.issued_box,
       c.status AS "status: RunCommandStatus", c.resolution, c.issued_at, c.resolved_at
  FROM run_command c JOIN run r ON r.id = c.run_id
 WHERE c.status = 'pending'
   AND (r.lease_owner = $1
        OR (r.executing_box_id = $2
            AND (r.status IN ('done', 'failed', 'cancelled')
                 OR (r.status IN ('running', 'awaiting_approval')
                     AND (r.lease_owner IS NULL OR r.lease_expires_at IS NULL
                          OR r.lease_expires_at <= clock_timestamp())))))
 ORDER BY c.issued_at, c.id
```

**`resolve_command`** (`to = Pending` refused in Rust first):

```sql
UPDATE run_command SET status = $2, resolution = $3, resolved_at = clock_timestamp()
 WHERE id = $1 AND status = 'pending'
-- zero rows: SELECT 1 FROM run_command WHERE id = $1 → none: NotFound, else Ok(false)
```

**`relay_view`** (two reads; a display read, so two snapshots are harmless):

```sql
SELECT p.… (as `permission`)
  FROM step_permission p JOIN run r ON r.id = p.run_id
 WHERE r.item_id = $1 AND p.status = 'pending'
   AND r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()
 ORDER BY p.created_at, p.id;

SELECT DISTINCT c.run_id
  FROM run_command c JOIN run r ON r.id = c.run_id
 WHERE r.item_id = $1 AND c.kind = 'cancel' AND c.status = 'pending'
   AND r.status NOT IN ('done', 'failed', 'cancelled')
 ORDER BY c.run_id;
```

`.sqlx`: 291 at HEAD; each **distinct** query text above adds one file (sqlx keys by text, so
`SELECT 1 FROM run WHERE id = $1` reuses `take_lease`'s). Regenerate, count, and restate.

### 2.11 MemStore reference semantics (`htui-core/src/store/mem.rs`, T0)

`State` (`mem.rs:124-237`) gains, after `command_runs`:

```rust
    /// `step_permission` (MOD-42 plan D1), by id; `owner` beside the row (B-8), as
    /// [`State::lease_owners`] sits beside `runs`.
    permissions: BTreeMap<PermissionId, PermissionRow>,
    /// `run_command` (MOD-42 plan D1), by id.
    run_commands: BTreeMap<RunCommandId, RunCommand>,
```

with a private `#[derive(Debug, Clone)] struct PermissionRow { row: StepPermission, owner: Uuid }`.
`from_demo`'s literal (`:276-330`) adds both as `BTreeMap::new()` (re-grep for other `State {`
literals). The `impl WriteStore for MemStore` wrappers (`:5801`) take `let now = self.now();`
(the handle's clock, I-4, `MemStore::now` `:849`) and call `self.write(|state| state.x(…, now))` /
`self.read(…)`, never `Utc::now()`. `State` methods:

- `fn live_owner(&self, run: RunId, now) -> Option<Uuid>` — `lease_owners.get(&run)` when
  `runs[run].lease_expires_at` is `Some(t)` with `t > now`. Postgres's
  `r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()`.
- `open_permission(open, now)` — order: `steps.get(run_step_id)` else `NotFound { entity:
  "run_step" }`; `step.run_id != open.run_id` → `Constraint`; `lease_owners.get(&run) !=
  Some(&open.owner)` → `Fenced { step }` (owner only, B-9); a held id →
  `Constraint(already_exists("step_permission", id))`; a held `(session, request_id)` →
  `Constraint`; then every row with `run_step_id == step && session != open.session && status ∈
  {Pending, Answered}` → `Stale`, `resolved_at = Some(now)`; then insert `Pending`, `created_at =
  now`, answer fields `None`.
- `permission(id)` — the row's clone.
- `apply_permission(id, owner, now)` — unknown → `NotFound { entity: "step_permission" }`;
  `status == Answered && row.owner == owner && lease_owners.get(&run) == Some(&owner)` → `Applied`,
  `resolved_at = now`, `Some(PermissionChoice { option_id })`; else `None`.
- `settle_permissions(session, to, now)` — `to ∉ {Cancelled, Stale}` → `Constraint` first; moves
  `Pending|Answered` rows of the session; answers the count.
- `answer_permission(id, option_id, user, box_id, now)` — unknown id → `NotFound`; unknown user /
  box → `Constraint(references_no_row(…))` (B-7); then the CAS predicate; on a miss the refusal
  order is status, `NotOffered`, `ExecutorGone` (`live_owner(run, now) != Some(row.owner)`). A
  win sets `Answered`, `option_id`, `answered_by`, `answered_box`, `answered_at = now`.
- `relay_view(item, now)` — permissions of runs with `run.item_id == Some(item)`, `Pending` and
  `live_owner == Some(owner)`, sorted `(created_at, id)`; cancels: distinct `run_id` of `Pending`
  `Cancel` rows whose run's status is not terminal (`RunStatus::is_terminal`, `run.rs:75`), sorted.
- `request_cancel(run, user, box_id, now)` — `require_run(run)` (`NotFound { run }`) first; then
  user/box `Constraint`; a `Pending` `Cancel` row of the run → `AlreadyPending(id)`; else insert
  (`RunCommandId::new()`, `issued_at = now`) → `Inserted(id)`.
- `pending_commands(owner, box_id, now)` — `Pending` rows whose run: `lease_owners.get(&run) ==
  Some(&owner)`; or `executing_box_id == Some(box_id)` and (terminal, or `Running |
  AwaitingApproval` with no `lease_owners` entry or `lease_expires_at.is_none_or(|t| t <= now)`);
  sorted `(issued_at, id)`.
- `resolve_command(id, to, resolution, now)` — `to == Pending` → `Constraint` first; unknown →
  `NotFound { entity: "run_command" }`; not `Pending` → `Ok(false)`; else move, `Ok(true)`.
- `delete_project` (`:3756-3771`): after `self.lease_owners.retain(…)`:
  `self.permissions.retain(|_, p| !gone.runs.contains(&p.row.run_id));`
  `self.run_commands.retain(|_, c| !gone.runs.contains(&c.run_id));` — the cascade (F-18).
- B-21: `#[cfg(feature = "test-support")] pub fn relay_rows(&self) -> Vec<StepPermission>` and
  `pub fn command_rows(&self) -> Vec<RunCommand>`, both in id order.

---

## 3. T0 — contracts and the MemStore reference (serial, first)

### 3.1 Files

| File | Change |
|---|---|
| `htui-core/src/model/ids.rs` | §2.1 |
| `htui-core/src/model/mod.rs` | `pub mod relay;`, re-exports, four `check_enum` tests |
| `htui-core/src/model/relay.rs` (new) | §2.2 |
| `htui-core/src/store/traits.rs` | §2.3, module doc paragraph, imports |
| `htui-core/src/store/worker.rs` | §2.4: `RelayStore`, three `WorkerStore` methods, MemStore impls, doc counts, `is_relay` probe |
| `htui-core/src/store/mod.rs` | `RelayStore` re-export |
| `htui-core/src/store/mem.rs` | §2.11 |
| `htui-core/src/store/conformance.rs` | 12 cases (§3.3), imports |
| `htui-core/tests/mem_store.rs` | `CASES.len()` 104 → 116, the sentence gains "MOD-42 T0's twelve relay cases (plan D1-D5, D12, D13)" |
| `htui-store/tests/pg_conformance.rs` | `EXPECTED_CASES` 104 → 116 (`:21`), its doc (`:19-20`) and the stale "103" message (`:28`) → 116 |
| `htui-store/src/pg/write.rs` | nine placeholder bodies: `Err(StoreError::Backend("MOD-42 T1: not yet implemented".into()))` |
| `htui-store/src/writer.rs` | nine `WriteStore` forwards |
| `htui-store/src/worker.rs` | `RelayStore` for `PgStore` and `Writer`; three `WorkerStore` methods each |
| `htui-agent/src/conformance.rs` | `UsageSpy`: nine `WriteStore` forwards only |
| `htui-agent/tests/recorder.rs` | `SpyStore`: nine `WriteStore` forwards only |

### 3.2 Commits

1. **red** — everything in 3.1 except MemStore's state: the nine MemStore `WriteStore` bodies are
   `Err(StoreError::Backend("MOD-42 T0: red".into()))`; the 12 cases and both pins land. Compiles
   workspace-wide; red: all 12 cases fail at their first `.expect`.
2. **green** — §2.11 (fields, `State` methods, wrappers, `delete_project` pruning, test-support
   readers), the `delete_project_leaves_no_row_in_any_map` asserts. `grep -rn "MOD-42 T0: red"
   crates` prints nothing; `"MOD-42 T1: not yet implemented"` stays (T1 removes it).

### 3.3 Tests (written first)

Store conformance (generic `<S: WriteStore>`, run by `htui-core/tests/mem_store.rs` now and
`htui-store/tests/pg_conformance.rs` once T1 lands). Each name appended to `CASES` after
`an_executor_edit_is_a_compare_and_set`, its arm before `other => panic!`. Fixture: `leased_step`
(`conformance.rs:5701`, owner `a`, `ids::BOX`, item `HTUI_ANA_2`) and `taken_by` (`:5730`); the
demo user and `ids::BOX` are the actor. Red reason for all twelve: the MemStore body answers
`Backend("MOD-42 T0: red")`.

| Case | Asserts |
|---|---|
| `a_pending_permission_is_answered_once` | Open a row (two options) under `a`; `relay_view(item)` lists it; answer option 2 → `Answered`; `permission(id)` reads `answered` with `option_id`, `answered_by`, `answered_box`, `answered_at` set; a second answer (option 1) → `Refused(Answered)` and the row still names option 2; `relay_view` no longer lists it. |
| `an_answer_after_the_owner_changed_is_refused_executor_gone` | Open under `a`; `taken_by(a → b)`; answer → `Refused(ExecutorGone)`, row still `pending`. A second row on a fresh leased step whose lease `a` lapses (zero-TTL `refresh_lease`) → `Refused(ExecutorGone)` too. |
| `an_option_the_request_did_not_offer_is_refused` | Answer `"nope"` → `Refused(NotOffered)`, row `pending`; unknown id → `NotFound { entity: "step_permission" }`. |
| `apply_is_fenced_on_the_lease_owner` | Answered row: `apply(id, b)` → `None`; after `taken_by(a → b)`, `apply(id, a)` → `None` and the row stays `answered`; on a second answered row under a live `a`, `apply(id, a)` → `Some(choice)` naming the option, row `applied` with `resolved_at`; a second apply → `None`. |
| `opening_a_permission_stales_older_sessions_of_the_step` | Session `s1` opens `r1` (pending) and `r2` (answered); session `s2` opens `r3` on the same step → `r1`, `r2` `stale` with `resolved_at`, `r3` `pending`; a row of `s1` on **another** step stays `pending`. Same `(s2, request_id)` again → `Constraint`. |
| `opening_a_permission_is_fenced_on_the_lease` | Unknown step → `NotFound { entity: "run_step" }`; a step of another run → `Constraint`; owner `b` on `a`'s lease → `Fenced { step }`; nothing was written (`permission(id)` → `None`). |
| `settle_moves_only_pending_and_answered_rows` | Session with one each of `pending`, `answered`, `applied`; `settle(s, Cancelled)` → `2`; the applied row unchanged; `settle(s, Applied)` → `Constraint`; another session's rows untouched. |
| `a_run_has_at_most_one_pending_cancel` | `request_cancel` → `Inserted(x)`; again → `AlreadyPending(x)`; after `resolve_command(x, Applied, None)` a third → `Inserted(y)`, `y != x`; unknown run → `NotFound { entity: "run" }`. |
| `resolve_command_is_a_compare_and_set` | `resolve(x, Refused, Some("…"))` → `true`; again (either status) → `false`; `resolve(x, Pending, None)` → `Constraint`; unknown id → `NotFound { entity: "run_command" }`. |
| `pending_commands_are_the_owners_and_the_boxs_free_runs` | Run R leased by `a` on `ids::BOX` with a pending cancel: `pending_commands(a, BOX)` lists it; `(c, BOX)` does not (live lease of another owner); after `a`'s zero-TTL refresh `(c, BOX)` lists it; `(c, BoxId::new())` does not; a terminal run (`finish_run(…, Cancelled)`) on `BOX` with a pending row is listed for `(c, BOX)` (B-4). |
| `relay_view_lists_live_pending_requests_and_pending_cancels` | A stale row and a row whose lease lapsed are absent; a pending live row present; a run with a pending cancel is in `cancels`; once that run is terminal it is not (B-4). |
| `deleting_a_project_takes_its_relay_rows` | Row + pending cancel on an `HTUI` run; `delete_project(PROJECT_HTUI)` succeeds; `permission(id)` → `None`; `pending_commands` no longer lists the command. |

MemStore unit tests (`mem.rs` `mod tests`, not `CASES`): `delete_project_leaves_no_row_in_any_map`
gains `permissions`/`run_commands` asserts (red: the maps survive the delete);
`relay_times_are_the_handles_clock` (I-4): a store `with_clock(TestClock::at(t))` opens, answers,
applies and requests a cancel; `created_at`, `answered_at`, `resolved_at`, `issued_at` all equal
`t` (red: placeholder). `model/mod.rs` gets the four enum tests (green on arrival: they pin the
`CHECK` text).

### 3.4 Gate

G-T0 (§12). T0 runs alone; `htui-store`'s Postgres conformance would fail on the placeholders, so
every `htui-store` test run in T0 is `env -u HTUI_TEST_DATABASE_URL`.

---

## 4. T1 — Postgres (parallel with T2 and T5; the wave's only Postgres lane)

### 4.1 Files

| File | Change |
|---|---|
| `htui-store/migrations/0011_permission_relay.sql` (new) | §2.9, verbatim |
| `htui-store/src/pg/relay.rs` (new) | §2.10 bodies |
| `htui-store/src/pg/mod.rs` | `mod relay;` |
| `htui-store/src/pg/write.rs` | nine delegations replacing T0's placeholders |
| `htui-store/.sqlx/*` | regenerated (scratch DB, §12) |
| `htui-store/tests/pg_criteria.rs` | §4.3 |
| `htui-store/tests/migrations.rs` | `TABLES` + `"step_permission"`, `"run_command"` (a `// 0011_permission_relay.sql (MOD-42)` group, last), `39 → 41` and its two messages (`:112-122`), `vec![1..=10]` → `…, 11` and its sentence (`:87-96`, "and MOD-42's 0011_permission_relay.sql"), `Pending(10)`/`MigrationsPending(10)` → 11 at `:917`, `:1016`, `:1021`, `:1040`, the `10` at `:1190` |
| `htui-store/tests/connect.rs` | `10 → 11` at `:140`, `:156`, `:242`, their sentences name `0011_permission_relay.sql` |

T1 never edits `htui-core`: a MemStore/Postgres disagreement found here is reported to the main
thread, not fixed in the lane (T2 and T5 build against `htui-core` meanwhile).

### 4.2 Commits

1. **red** — the pins and the `pg_criteria` cases. Red: the pins see ten migrations and 39 tables;
   the cases meet the placeholders (and `pg_conformance`'s twelve new cases fail the same way).
2. **green** — `0011`, `relay.rs`, delegations, `.sqlx` regenerated and counted. `grep -rn
   "MOD-42 T1: not yet implemented" crates` prints nothing.

### 4.3 Tests (written first)

`htui-store/tests/pg_criteria.rs` (`common::demo_db()`; one scratch **database** per test,
`testkit.rs:118`; each ends with `db.drop_db().await`). A second client is
`PgStore::connect(&db.url, &identity::load_or_mint(tmp.path())?)` over a `tempfile::tempdir()` —
never the real config root — which registers a second box (`b.this_box() != db.store.this_box()`).

| Test | Asserts |
|---|---|
| `a_second_box_answers_and_the_executor_applies` | A (`db.store`) claims a run and opens a row; B answers → `Answered`; A's `apply_permission(id, owner)` → `Some` with B's option; the row reads `applied`, `answered_box == B's box`. Executor-side writes only on A, answer-side only on B (I-1: B wrote no `session_event` — `step_events` count unchanged). |
| `a_second_box_cannot_answer_after_adoption_or_expiry` | Row under owner `a`; a zero-TTL `refresh_lease` by `a` → B's answer `Refused(ExecutorGone)`; another row, run taken by owner `x` (`take_lease` after the lapse) → `Refused(ExecutorGone)`; both rows still `pending`. |
| `two_concurrent_answers_admit_one` | `tokio::join!` of A's and B's answers on one row, 20 rounds: exactly one `Answered` per round, the other `Refused(Answered)` (I-3). |
| `two_concurrent_cancel_requests_insert_one` | `join!` of two `request_cancel`s: one `Inserted(x)`, one `AlreadyPending(x)`; one row in `run_command`. |
| `relay_times_are_the_databases` | `SELECT clock_timestamp()` before, then open/answer/apply/request, then after: every stored instant lies between the two database readings (I-4). |
| `deleting_a_project_holding_relay_rows_succeeds` | Rows in both tables under an `HTUI` run; `delete_project(PROJECT_HTUI)` → `Ok`; `common::count` of both tables is 0. |
| `a_terminal_runs_pending_cancel_is_listed_for_its_box` | B-4 on Postgres: after `finish_run(…, Done)` the row is in `pending_commands(any, box)` and absent from `relay_view(item).cancels`. |

Red reason: the placeholders answer `Backend("MOD-42 T1: not yet implemented")`.

### 4.4 Gate

G-T1. Every Postgres command under `flock /tmp/mod42-pg.lock`.

---

## 5. T2 — `drive` (parallel with T1 and T5)

### 5.1 Files

`htui-agent/src/record.rs` (`mod relay;`, re-exports, `pump`'s body, two doc sentences),
`htui-agent/src/record/relay.rs` (new), `htui-agent/src/lib.rs` (`pub use record::{…}` adds
`Control, NoRelay, RELAY_GRACE, RELAY_POLL, Relay, Signal, control_channel, drive`),
`htui-agent/src/error.rs` (`Cancelled`), `htui-agent/tests/relay.rs` (new,
`#![cfg(feature = "test-support")]`).

### 5.2 `drive`'s body (shape; names are binding, layout is not)

```rust
pub async fn drive<S: RecorderStore, R: RelayStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    relay: Option<&Relay<'_, R>>,
    control: &mut Control,
) -> Result<DoneEvent, DriverError> {
    let mut opened = false;
    let out = turn(session, recorder, relay, control, &mut opened).await;
    // B-15: a cancel settled its rows `cancelled` inside the sequence (I-7); every other exit
    // marks leftovers `stale` (D5), best-effort.
    if let Some(relay) = relay
        && opened
        && !matches!(out, Err(DriverError::Cancelled))
        && let Err(err) = htui_core::store::RelayStore::settle_permissions(
            relay.store, relay.session, PermissionStatus::Stale).await
    {
        tracing::warn!(%err, "the session's leftover permission rows were not marked stale");
    }
    out
}

async fn turn<S, R>(session, recorder, relay: Option<&Relay<'_, R>>, control, opened: &mut bool)
-> Result<DoneEvent, DriverError> {
    let mut calls: HashMap<String, ToolCallEvent> = HashMap::new();
    loop {
        // R-8, B-2: a cancel sent before this call, or while the last event was being recorded.
        if let Signal::Cancel { grace } = control.signal() {
            return cancel(session, recorder, relay, *opened, None, grace).await;
        }
        let pulled = tokio::select! {
            biased;
            () = control.changed() => None,          // the loop top reads the new value
            pulled = session.next_event() => Some(pulled),
        };
        let Some(pulled) = pulled else { continue };
        let Some(envelope) = pulled? else { return Err(DriverError::Closed) };
        let done = match &envelope.event { DriverEvent::Done(done) => Some(*done), _ => None };
        // Only with a relay: the call (for policy matching and the summary) and the request.
        let (call, request) = match (&envelope.event, relay.is_some()) {
            (DriverEvent::ToolCall(call), true) => (Some(call.clone()), None),
            (DriverEvent::PermissionRequest(request), true) => (None, Some(request.clone())),
            _ => (None, None),
        };
        if let Some(breach) = recorder.record(envelope).await? {
            return enforce_breach(session, recorder, breach).await;
        }
        if let Some(done) = done {
            return Ok(done);
        }
        let Some(relay) = relay else { continue };    // pump: pull on; the transport says "parked"
        if let Some(call) = call {
            calls.insert(call.tool_call_id.clone(), call);
        }
        if let Some(request) = request {
            match park(session, recorder, relay, control, &calls, &request, opened).await? {
                Parked::Resumed => {}
                Parked::Cancel { grace } => {
                    return cancel(session, recorder, Some(relay), *opened,
                                  Some(&request.request_id), grace).await;
                }
            }
        }
    }
}

enum Parked { Resumed, Cancel { grace: Duration } }

async fn park<S, R>(session, recorder, relay: &Relay<'_, R>, control, calls, request, opened)
-> Result<Parked, DriverError> {
    let call = request.tool_call_id.as_ref().and_then(|id| calls.get(id));
    // Stages 1-2 (D9): the agent's own policy, as chat's `run_turn` does (agent_worker.rs:3927).
    if let Some(answer) = crate::permission::evaluate(relay.policy, call, &request.options) {
        session.answer_permission(request.request_id.clone(),
                                  PermissionAnswer::Selected(answer.option_id.clone())).await?;
        recorder.record_permission_answer(&request.request_id, Some(&answer.option_id),
                                          AnsweredBy::Policy, false, (relay.now)()).await?;
        tracing::info!(stage = ?answer.stage, reason = %answer.reason,
                       "a permission request was answered by policy");
        return Ok(Parked::Resumed);
    }
    // Stage 3: the request row is durable before a relay row names it.
    recorder.flush().await?;
    let (summary, options) = scrubbed(recorder.scrubber, call, &request.options); // I-5, B-14
    let id = PermissionId::new();
    htui_core::store::RelayStore::open_permission(relay.store, OpenPermission {
        id, run_id: relay.run, run_step_id: relay.step, session: relay.session,
        request_id: request.request_id.as_str().to_owned(),
        tool_call_id: request.tool_call_id.clone(), summary, options, owner: relay.owner,
    }).await?;
    *opened = true;
    loop {
        if let Signal::Cancel { grace } = control.signal() {
            return Ok(Parked::Cancel { grace });
        }
        let row = htui_core::store::RelayStore::permission(relay.store, id).await?
            .ok_or_else(|| StoreError::NotFound { entity: "step_permission", id: id.to_string() })?;
        match row.status {
            PermissionStatus::Pending => {}
            PermissionStatus::Answered => {
                // D4: apply under the lease, answer the live session, echo through the recorder.
                let Some(choice) = htui_core::store::RelayStore::apply_permission(
                    relay.store, id, relay.owner).await? else {
                    return Err(StoreError::Fenced { step: relay.step }.into());   // B-16
                };
                session.answer_permission(request.request_id.clone(),
                                          PermissionAnswer::Selected(choice.option_id.clone())).await?;
                recorder.record_permission_answer(&request.request_id, Some(&choice.option_id),
                                                  AnsweredBy::User, false, (relay.now)()).await?;
                return Ok(Parked::Resumed);
            }
            PermissionStatus::Stale | PermissionStatus::Cancelled => {
                return Ok(Parked::Cancel { grace: relay.grace });                // B-16
            }
            PermissionStatus::Applied => return Err(StoreError::Fenced { step: relay.step }.into()),
        }
        tokio::select! {
            biased;
            () = control.changed() => {}
            () = tokio::time::sleep(relay.poll) => {}
        }
    }
}

/// The graceful cancel (D6, I-7): `session.cancel(grace)` first — ACP answers every parked
/// responder `Cancelled` itself (`acp/mod.rs:1567-1614`), the fake drops it — then the row the
/// transport does not write, then the relay rows, then the drain (B-11).
async fn cancel<S, R>(session, recorder, relay: Option<&Relay<'_, R>>, opened: bool,
                      parked: Option<&PermissionRequestId>, grace: Duration)
-> Result<DoneEvent, DriverError> {
    if let Err(err) = session.cancel(grace).await {
        tracing::warn!(%err, "the cancel took the kill path; the closing rows are written anyway");
    }
    if let Some(relay) = relay {
        if let Some(request_id) = parked {
            recorder.record_permission_answer(request_id, None, AnsweredBy::Policy, true,
                                              (relay.now)()).await?;
        }
        if opened {
            htui_core::store::RelayStore::settle_permissions(
                relay.store, relay.session, PermissionStatus::Cancelled).await?;
        }
    }
    let drained = async {
        while let Ok(Some(envelope)) = session.next_event().await {
            recorder.record(envelope).await?;   // a breach verdict here is spent; ignored
        }
        Ok::<(), RecordError>(())
    };
    match tokio::time::timeout(grace + DRAIN_SLACK, drained).await {
        Ok(recorded) => recorded?,
        Err(_) => tracing::warn!("the cancelled session did not end its stream within the grace"),
    }
    Err(DriverError::Cancelled)
}

/// B-14: `"<kind>: <title>"` and the labels, scrubbed together; fail-closed on `Unmasked`.
fn scrubbed(scrubber: &dyn Scrubber, call: Option<&ToolCallEvent>, options: &[PermissionOption])
-> (Option<String>, Vec<RelayOption>);
/// `PermissionOptionKind` → `RelayOptionKind`, one arm per variant.
fn relay_kind(kind: PermissionOptionKind) -> RelayOptionKind;
```

### 5.3 Commits

1. **red** — `DriverError::Cancelled`; `record/relay.rs` with every type of §2.5 and `drive` whose
   body is **today's pump loop** (relay and control ignored); `pump` over it; `tests/relay.rs`.
   Red: every relay case meets the transport's "is parked" (or never sees the cancel).
2. **green** — §5.2; the two doc sentences.

### 5.4 Tests (written first, `htui-agent/tests/relay.rs`)

Over `FakeSession` (scripts with `ScriptEvent::ParkPermission`, `fake.rs:450`) and
`MemStore::demo()`; the run is created and claimed with an owner, as the store conformance's
`leased_step` does; `Relay.poll` = 10 ms; `#[tokio::test(start_paused = true)]` where a wait is
involved; the scrubber is `MinimalScrubber::new(["fake-secret-9f8e7d"])`. A second "client" is a
spawned task on a clone of the store polling `relay_view(item)` until a row appears.

| Test | Asserts | Red because |
|---|---|---|
| `a_policy_answer_writes_no_relay_row_and_records_by_policy` | Policy `default: Allow`: `drive` → `Ok(done)`; `relay_rows()` empty; the log has `permission_answer {by: "policy", option_id: <allow>}`. | pump loop: "is parked" |
| `a_stage_three_request_is_relayed_scrubbed_and_resumes_on_an_answer` | The tool title carries `fake-secret-9f8e7d`; the row's `summary` masks it and is `"execute: …"`; the client answers option 1 → `Ok(done)`; row `applied`; log `permission_answer {by: "user", option_id}` after `permission_request` (I-1: the client wrote no event). | no row |
| `a_cancel_while_parked_answers_cancelled_once` | Signal `Cancel { grace: 3 s }` while parked → `Err(Cancelled)`; exactly one `permission_answer {option_id: null, by: "policy", cancelled: true}`; the row `cancelled`; the session's recorded `cancel` grace is 3 s (a recording wrapper over `FakeSession`); the log ends `done {stop_reason: cancelled}` (I-7). | no row, no cancel |
| `a_cancel_sent_before_the_park_still_ends_as_a_cancel` | `send_replace(Cancel)` before `drive` is called → `Err(Cancelled)` and no relay row (R-8, probe A3b). | the cancel is never read |
| `a_cancel_mid_turn_cancels_the_session_gracefully` | A session whose `next_event` pends (a wrapper) is cancelled through the control → `session.cancel` called once with the grace, `Err(Cancelled)` (B-2). | the pull is never interrupted |
| `a_stale_row_read_back_ends_the_turn_as_a_cancel` | The client opens a second session's row on the same step (`open_permission`, D5) → the parked row turns `stale`; `drive` → `Err(Cancelled)`, I-7's row once, `cancel` called with `RELAY_GRACE`. | "is parked" |
| `an_answer_whose_lease_is_gone_is_fenced` | The client answers, then `release_lease(run, owner)` before the next poll → `Err(Store(Fenced { step }))`; the session got no answer; the row stays `answered`. | "is parked" |
| `a_session_that_fails_while_parked_leaves_no_pending_row` | A wrapper whose `answer_permission` answers `Err(Closed)`: after an answer, `drive` → `Err(Closed)`; the row is `applied` (the CAS ran) and no row of the session is `pending`/`answered` (B-15). Second variant: the store read fails (`MemFault` if one fits, else a wrapper store) → the row ends `stale`. | "is parked" |
| `drive_without_a_relay_is_pump` | `drive(…, None::<&Relay<'_, NoRelay>>, &mut Control::never())` over a parking script answers the same `Err(Transport("… is parked …"))` and the same rows as `pump` (I-8). | green on arrival (a pin) |

Every existing `pump` test (21 in `conformance.rs`, `tests/extensibility.rs`, `tests/recorder.rs`)
passes unchanged.

### 5.5 Gate

G-T2, with `HTUI_TEST_DATABASE_URL` unset (`htui-agent` touches no Postgres; the rule is uniform).

---

## 6. T5 — Runs-pane answering (M2; parallel with T1 and T2, in a worktree)

### 6.1 Lane

`git worktree add -b hr/MOD-42-t5 /home/mluigi/projects/htui-wt/mod-42-t5 <T0 green sha>`; every
read and edit with native tools there (Gortex indexes, and its edit tools write, the primary
checkout only); every command with `HTUI_TEST_DATABASE_URL` unset; only `-p htui` commands.

### 6.2 Files

| File | Change |
|---|---|
| `crates/htui/src/store_worker.rs` | §2.8 variants, `name()` arms, two `try_serve` arms (§6.3), one `mod tests` case. **No** loop arm (T4's). |
| `crates/htui/src/ui/tabs/chat/permission.rs` | `line(options, theme) -> Line<'static>` and `pick_from(options, digit) -> Option<PermissionOption>`; `render` = `Paragraph::new(Self::line(options, theme))`, `pick` = `Self::pick_from(options, digit)`; chat (`chat/mod.rs:323`, `:727`) unchanged |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | §6.4 |
| `crates/htui/tests/backlog.rs` | §6.6 harness cases |
| `crates/htui/tests/snapshots/*` | one new `.snap` |

### 6.3 `try_serve` arms (after `RunActions`, `store_worker.rs:1590`)

```rust
        // MOD-42 D14: a display read; offline (no writer) the view is empty, never `Failed` or
        // `Unreachable` (OQ-4, `app/update.rs:285-287`, `go_offline` at `:2121-2127`).
        StoreRequest::RelayView { item } => StoreReply::RelayView {
            item: *item,
            view: Box::new(match backend.writer() {
                Some(writer) => WriteStore::relay_view(&writer, *item).await?,
                None => RelayView::default(),
            }),
        },
        // MOD-42 D3, D14: refused offline before anything is sent (`requirements.rs:710-717`).
        StoreRequest::AnswerPermission { permission, option_id } => {
            let writer = backend
                .writer()
                .ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
            let user = backend.this_user().await?;
            let box_id = backend
                .box_info()
                .await?
                .map(|info| info.box_id)
                .ok_or_else(|| StoreError::NotFound { entity: "box", id: "this box".to_owned() })?;
            match WriteStore::answer_permission(&writer, *permission, option_id, user, box_id).await? {
                AnswerOutcome::Answered => StoreReply::PermissionAnswered { permission: *permission },
                AnswerOutcome::Refused(why) => StoreReply::Failed {
                    request: request.name(),
                    message: why.to_string(),
                },
            }
        }
```

### 6.4 `RunsTab` (`runs.rs`)

- Field `relay: Option<RelayView>` (doc: "D14: the last `RelayView` for [`RunsTab::item`]");
  `on_item_change` clears it.
- `on_runs` (`:330-349`): after `ctx.request(StoreRequest::RunActions(item))`,
  `ctx.request(StoreRequest::RelayView { item })`.
- `on_reply` (`:1199-1228`): `StoreReply::RelayView { item, view } if Some(*item) == self.item =>
  self.relay = Some((**view).clone())`; `StoreReply::PermissionAnswered { .. } =>
  self.re_read(ctx)`; `StoreReply::Failed { request, .. } if *request == ANSWER_PERMISSION =>
  self.re_read(ctx)` (the sentence is on the status line already). `const ANSWER_PERMISSION: &str =
  "answer_permission";` beside `DOCUMENT` (`:79`).
- `fn pending_under_cursor(&self) -> Option<&StepPermission>`: the first `relay.permissions` row
  whose `run_step_id` is `self.selected_step()`.
- `on_key` (`:1138-1163`), in `Browse`, after the `CONTROL|ALT` pass and **before** the `match`:

```rust
        // MOD-42 D14: while the step under the cursor has a pending request, every digit is this
        // pane's (`chat/mod.rs:490-499`); otherwise digits pass to the tab select
        // (`keymap.rs:225-232`).
        if let KeyCode::Char(digit @ '1'..='9') = key.code
            && let Some(pending) = self.pending_under_cursor()
        {
            if let Some(option) = PermissionStrip::pick_from(&strip_options(&pending.options), digit) {
                ctx.request(StoreRequest::AnswerPermission {
                    permission: pending.id,
                    option_id: option.id,
                });
            }
            return Handled::Consumed;
        }
```

- `fn strip_options(options: &[RelayOption]) -> Vec<PermissionOption>` (F-19; kind by a
  four-arm match).
- `fn permission_lines(pending: &StepPermission, theme: &Theme) -> [Line<'static>; 2]`: line 1 is
  `INDENT` spaces then `fit(&format!("asks: {}", summary_or("a permission")), PANE - INDENT)` in
  `theme.accent`; line 2 is `PermissionStrip::line(&strip_options(..), theme)` cut or padded to
  exactly `PANE` columns (a `fit_line(line, PANE)` helper that walks spans, cutting with `CUT`).
  The render loop that pushes `step_lines` for each step entry pushes these two right after a step
  with a pending request; every height computation over entries (scroll, cursor visibility)
  counts them.
- `run_lines(run, cancel_requested: bool, theme)` (`:934-955`): when the run id is in
  `relay.cancels`, one more line `fit(CANCEL_REQUESTED_LINE, PANE)` in `theme.accent`, before the
  failure line; `const CANCEL_REQUESTED_LINE: &str = "cancel requested";` (the pane's label; the
  runtime's sentence is longer than 43 columns).

### 6.5 Commits

1. **red** — §2.8 variants, `name()` and §6.3 arms (complete: they are the store side), the strip
   refactor (behaviour-preserving; chat's three tests stay green), every test of §6.6, the updated
   request-list pin. Red: the pane neither asks for nor keeps a `RelayView`, so digits pass and no
   extra line is drawn.
2. **green** — §6.4; `cargo insta review` accepts exactly the one new snapshot.

### 6.6 Tests (written first)

`runs.rs` `mod tests` (the `Shell`/`pane` helpers, `PANE_WIDTH` 43):

| Test | Asserts |
|---|---|
| `the_first_runs_reply_subscribes_once_and_asks_for_the_actions` (updated, `:2983-3027`) | The request list is `[RunStream, RunActions, RelayView]` for the item. |
| `a_digit_answers_the_pending_request_under_the_cursor` | `on_reply(RelayView { item, view })` with a two-option row on the cursor's step; `'2'` → `Consumed`, one `AnswerPermission { permission, option_id: <second id> }` sent. |
| `a_digit_past_the_offered_options_is_consumed_and_sends_nothing` | `'5'` → `Consumed`, nothing sent. |
| `a_digit_with_no_pending_request_passes_to_tab_select` | No row (or a row on another step) → `Pass`. Green on arrival: a pin. |
| `a_relay_view_for_another_item_is_dropped` | A `RelayView` for another item changes nothing; `'1'` passes. |
| `a_step_with_a_pending_request_takes_two_more_lines_each_forty_three_wide` | `permission_lines` are each exactly `PANE` wide for a short, a 200-character and a `None` summary and for four options with long labels; the drawn pane shows the summary line right under the step's second line. `every_step_takes_two_lines` and `every_step_row_fits_forty_three_columns` hold unchanged for steps with no request. |
| `a_run_with_a_pending_cancel_says_cancel_requested` | `cancels` holds the run → its lines contain `cancel requested`; the run grid lines are unchanged. |
| `an_answer_reply_re_reads_the_runs` | `PermissionAnswered` and `Failed { request: "answer_permission" }` each send `Runs(item)`. |

`crates/htui/tests/backlog.rs` (`#![cfg(feature = "testkit")]`; run with `--all-features`):

| Test | Asserts |
|---|---|
| `runs_pane_shows_a_pending_permission` (snapshot) | `Harness::over(store)` where `store` is `MemStore::demo()` with a run on the selected item claimed for `MAX_LEASE_TTL`, one step, and `open_permission` (summary `"execute: cargo test"`, options Allow once / Reject once); the Runs pane frame shows the summary and `[1] Allow once  [2] Reject once`. |
| `a_digit_on_the_runs_pane_answers_through_the_store` | Same seed; `harness.key("1")`; `settle()`; `store.relay_rows()[0].status == Answered`, `option_id` the first; the pane re-read drops the strip. |
| `a_refused_answer_lands_on_the_status_line` | Seed, render (strip visible), then `release_lease(run, owner)` on the store; `key("1")` → `harness.app().status` holds `EXECUTOR_GONE`. |
| `offline_the_runs_pane_asks_for_no_error` | `let _keyring = htui_store::testkit::mock_keyring().await;` first; `Harness::over_backend(Backend::Offline { cache, since })` (the `tests/hierarchy.rs:1652-1667` pattern); select an item; `settle()`; `harness.app().status == None` (the empty `RelayView` raised nothing). |

`store_worker.rs` `mod tests`: `relay_reads_are_empty_offline_and_answers_are_refused` —
`try_serve(&Backend::Offline { … }, &StoreRequest::RelayView { item })` is
`StoreReply::RelayView` with an empty view; `AnswerPermission` is
`Err(StoreError::Unreachable(DATABASE_UNREACHABLE))` (keyring guard first).

### 6.7 Gate

G-T5 (worktree, no Postgres).

---

## 7. T3 — engine (serial, after the wave merge)

### 7.1 Files

| File | Change |
|---|---|
| `htui-orch/src/engine.rs` | §2.6 types, helpers and fields; `drive_once` (§7.2); four catch-alls and one comment (§7.3); doc counts; literals at `:6374` (`fake_parts`: `policy: orch.policy_for(), control: orch.control_for()`), `:6707`, `:6795`, `:9492`, `:11872`, `:12782` (`policy: &ask_policy, control: &never_cancelled` unless the test needs the fake's); tests (§7.5) |
| `htui-orch/src/command.rs` | `EngineError::Cancelled` |
| `htui-orch/src/lib.rs` | re-exports |
| `htui-orch/src/conformance.rs` | literal `:7525` |
| `htui-orch/src/fake.rs` | §7.4 |
| `htui-orch/tests/gix_isolator.rs` | `engine_as!` literal `:92`: `htui_orch::ask_policy`, `htui_orch::never_cancelled` |
| `htui-worker/src/runtime.rs` | `Kit`'s two lookups (§7.4); `Kit::engine` literal `:829-845` |

Re-grep `EngineParts {` before the red commit: the nine literals are the fact-checked set.

### 7.2 `drive_once` (`engine.rs:5630-5667`)

```rust
    async fn drive_once(&self, run: &Run, step: &RunStep, phase: &SnapshotPhase,
        key: &SessionKey<'_>, text: &str, cwd: PathBuf, extra_dirs: Vec<PathBuf>,
        recorder: &mut Recorder<'a, S>) -> Result<SessionResult, EngineError> {
        let project = self.project(run.project_id).await?;
        let settings = Self::project_settings(&project);
        let candidate = Self::candidate_of(step, phase)?;
        // MOD-42 D10, B-17: a cancel that reached the walk between sessions spawns nothing.
        let mut control = (self.parts.control)(run.id);
        if control.signal().is_cancel() {
            return Err(EngineError::Cancelled { run: run.id });
        }
        // MOD-42 D9: the agent's own policy (a judge call's is the judge agent's: `candidate_of`
        // over the judge phase).
        let policy = (self.parts.policy)(candidate.agent_id);
        let driver = (self.parts.driver)(&candidate, key);
        let spec = SessionSpec {
            // … every field as today, except:
            permission: policy.clone(),
        };
        let mut session = driver.start(spec, text.to_owned()).await?;
        let now = || self.now();
        let relay = Relay {
            store: self.parts.store,
            owner: self.parts.owner,
            run: run.id,
            step: step.id,
            session: RelaySessionId::new(),
            policy: &policy,
            poll: RELAY_POLL,
            grace: RELAY_GRACE,
            now: &now,
        };
        // MOD-42 D4, D10: two answers leave the session result before settle can read them.
        match drive(&mut *session, recorder, Some(&relay), &mut control).await {
            Err(DriverError::Cancelled) => Err(EngineError::Cancelled { run: run.id }),
            Err(fenced @ DriverError::Store(StoreError::Fenced { .. })) => {
                Err(EngineError::Driver(fenced)) // `is_fenced` → `LeaseLost` (`:6025-6034`)
            }
            result => Ok(result),
        }
    }
```

Its doc gains: "the outer `Err` is also a graceful cancel (`EngineError::Cancelled`) and a lost
fence; both leave before settle (D10)". `use htui_agent::record::{drive, Relay, RELAY_GRACE,
RELAY_POLL}` replaces `pump` in the imports if `pump` has no other user in the file.

### 7.3 I-6: the five catch-alls

`walk_step` (`:3255-3262`):

```rust
            Err(err) => {
                // MOD-42 I-6 (plan D10): a graceful cancel settles nothing; `cancel_leased` owns
                // every terminal status. The walk ends and `walk_leased` gives the lease back.
                if matches!(err, EngineError::Cancelled { .. }) {
                    return Err(err);
                }
                self.fail_hard(run, &step, &err.to_string()).await?;
                Err(err)
            }
```

`run_candidate` (`:3848-3853`):

```rust
            Err(err) => {
                // Plan D78's capture still runs: it releases a `shared_serialized` guard a sibling
                // in this `join_all` may be waiting on (it records commits; it settles nothing).
                if let (Some(trees), false) = (&trees, captured) {
                    self.release_trees(stage.step, trees).await;
                }
                // MOD-42 I-6: no `fail_candidate` for a cancel.
                if matches!(err, EngineError::Cancelled { .. }) {
                    return Err(err);
                }
                self.fail_candidate(stage.run, stage.phase, stage.step, &err.to_string())
                    .await
            }
```

The group's `join_all` (`:3682-3702`) — a cancel wins over any other candidate error, so the
caller's catch-all sees `Cancelled`:

```rust
        let mut first = None;
        let mut cancelled = None;
        for err in settled.into_iter().filter_map(Result::err) {
            // MOD-42 I-6: a cancel is raised in preference to any failure write's error.
            if matches!(err, EngineError::Cancelled { .. }) {
                cancelled.get_or_insert(err);
            } else if first.is_none() {
                first = Some(err);
            } else {
                tracing::warn!(run = %run.id, %err, "a further candidate's failure write failed; the first is raised");
            }
        }
        if let Some(cancelled) = cancelled {
            if let Some(err) = first {
                tracing::warn!(run = %run.id, %err, "a candidate's error is dropped for the run's cancel");
            }
            return Err(cancelled);
        }
        first.map_or(Ok(None), Err)
```

`run_judge` (`:4398-4404`):

```rust
        let verdict = match self
            .judge_sessions(run, phase, attempt, &judge, &candidate, &prompts)
            .await
        {
            Ok(Ok(documents)) => decide(&documents, &indices),
            Ok(Err(failure)) => Err(failure),
            // MOD-42 I-6: no `fail_judge` for a cancel; the judge stays `running` for
            // `cancel_leased`.
            Err(err @ EngineError::Cancelled { .. }) => return Err(err),
            Err(err) => Err(JudgeFailure::SessionFailed(err.to_string())),
        };
```

`judge_calls` (`:4862`), F-4 — no code, one comment above the arm:

```rust
                // MOD-42 I-6: `Cancelled` and a lost fence leave `drive_once` as its outer `Err`,
                // through the `?` above; this arm sees only the session's own errors.
                Err(err) => return Ok(Err(JudgeFailure::SessionFailed(err.to_string()))),
```

`session` (`:5535-5570`) and `candidate_live`'s `Err(refused)` arm (`:3972-3976`) already finish
the recorder and re-raise an outer `Err` unchanged: no edit.

### 7.4 Lookups: `Kit` and `FakeOrchestrator` (B-6)

`Kit` (`runtime.rs:690-712`) gains:

```rust
    /// MOD-42 plan D9: each agent's `agent.settings.permission`, parsed once per task as chat
    /// parses it (`agent_worker.rs:968-969`: a row that does not parse asks).
    policy: Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync>,
    /// MOD-42 plan D10: each run's control. T3: never signalled; T4: the run's `Walks` parent.
    control: Box<dyn Fn(RunId) -> Control + Send + Sync>,
```

built in `Kit::read` (`:716-783`):

```rust
        let policies: HashMap<AgentId, PermissionPolicy> = agents
            .values()
            .map(|summary: &AgentSummary| {
                let settings: AgentSettings =
                    serde_json::from_value(summary.agent.settings.clone()).unwrap_or_default();
                (summary.agent.id, settings.permission)
            })
            .collect();
        // …
            policy: Box::new(move |agent| policies.get(&agent).cloned().unwrap_or_default()),
            control: Box::new(htui_orch::never_cancelled),
```

and `Kit::engine` passes `policy: &*self.policy, control: &*self.control`. The nine call sites
are unchanged.

`FakeOrchestrator` (`fake.rs:1266-1301`) gains one field `relays: Relays`, a private struct with a
hand-written `Debug`:

```rust
/// MOD-42: the engines' policy and control lookups (plan D9, D10), owned so `fake_parts` can lend
/// them for `'a`.
struct Relays {
    signal: watch::Sender<Signal>,
    policies: Arc<Mutex<BTreeMap<AgentId, PermissionPolicy>>>,
    policy: Box<dyn Fn(AgentId) -> PermissionPolicy + Send + Sync>,
    control: Box<dyn Fn(RunId) -> Control + Send + Sync>,
}
// policy: clones `policies`' entry or `PermissionPolicy::default()`;
// control: `let receiver = signal.subscribe(); Box::new(move |_| Control::new(receiver.clone()))`.

impl FakeOrchestrator {
    /// MOD-42 D9: `agent`'s policy in every engine built over this harness.
    pub fn set_policy(&self, agent: AgentId, policy: PermissionPolicy);
    /// MOD-42 D10: every walk of this harness is asked to stop gracefully.
    pub fn cancel_walks(&self, grace: Duration);   // signal.send_replace(Signal::Cancel { grace })
    /// The policy lookup `fake_parts` lends.
    #[must_use]
    pub fn policy_for(&self) -> PolicyFor<'_>;
    /// The control lookup `fake_parts` lends.
    #[must_use]
    pub fn control_for(&self) -> ControlFor<'_>;
}
```

`restarted()` (`:1357-1390`) builds fresh `Relays` (a new process: no signal) carrying the
policies (they are the agents', not the process's). `ScriptedStep` gains
`pub fn parks(request: PermissionRequestEvent, body: &str) -> Self`: one turn of
`Emit(ToolCall)`, `ParkPermission(request)`, `Emit(Done { EndTurn })`, output `body`.

### 7.5 Commits

1. **red** — §2.6 (fields, aliases, helpers, `EngineError::Cancelled`, re-exports), every literal,
   `Kit` lookups, `Relays`, `ScriptedStep::parks`, the tests. `drive_once` unchanged (still `pump`
   with `PermissionPolicy::default()`). Red: a parked step fails "is parked"; a policy is ignored;
   a cancel is never read.
2. **green** — §7.2, §7.3, the "sixteen" → "eighteen" docs.

### 7.6 Tests (written first, `engine.rs` `mod tests`, `#[tokio::test(start_paused = true)]`)

A second "client" is a spawned task over `orch.store.clone()` (same rows) that polls
`relay_view(item)` every 100 ms of paused time and answers with the demo user and `ids::BOX`.

| Test | Asserts |
|---|---|
| `a_parked_step_resumes_on_an_answer_from_another_client` | `parks` scripted at the first phase; the client answers allow → the claim walks on, the step `done`; its log holds `permission_request` then `permission_answer {by: "user"}`; `relay_rows()` shows one `applied` row. |
| `an_allow_policy_answers_without_a_relay_row` | `set_policy(agent, default: Allow)`; same script → step `done`, `relay_rows()` empty, the answer row `by: "policy"` (R-7). |
| `a_cancel_reaches_a_parked_single_step_and_settles_nothing` | Once the row is pending, `cancel_walks(2 s)` → the walk answers `Err(EngineError::Cancelled { run })`; step and run still `running`, no `finish_step` (`finished_at` `None`, no failure note); I-7's row once; the row `cancelled`; the lease released; then `cancel_run(run)` → `CommandOutcome::Cancelled` and every step `cancelled`. |
| `a_cancel_reaches_a_parked_fanout_candidate_and_settles_nothing` | Fan-out 2: candidate 0 parks, candidate 1 is `done_with_output`; cancel → `Err(Cancelled)`; candidate 0 still `running` with no `fail_candidate` note; no `select_fanout`; then `cancel_run` settles both. |
| `a_cancel_reaches_a_parked_judge_call_and_settles_nothing` | Judge call 0 parks (`script_candidate(..., -1, 0, parks(..))`); cancel → `Err(Cancelled)`; the judge step `running`, no `fail_judge` park (run not `awaiting_approval`, no note). |
| `an_answer_applied_after_the_lease_moved_is_lease_lost` | The client answers, zero-TTL-refreshes `orch.owner()`'s lease and `take_lease`s it as a stranger, before the walk's next poll → `Err(EngineError::LeaseLost { run })`; the session got no answer; the row stays `answered`. |
| `a_cancel_before_the_session_spawns_nothing` | `cancel_walks` before `claim` → `Err(Cancelled)`, the scripted driver never started (B-17). |

`a_dispatch_future_is_send` (`:12782`) and `htui-orch`'s conformance stay green: they are the
`Send` and no-regression pins.

### 7.7 Gate

G-T3.

---

## 8. T4 — durable graceful cancel (M3; serial, after T3)

### 8.1 Files

| File | Change |
|---|---|
| `htui-worker/src/runtime.rs` | §2.7 constants and builder; `Walks` (§8.2); `Shared` fields (`polling: AtomicBool`, `applying: StdMutex<HashSet<RunCommandId>>`, `cancel_grace: Duration`); `TaskCtx::requested`; `cancel_run` (§8.3); `on_run`'s promote preempt and promote refusal; the five `walked` arms (B-3); `poll_commands_with` + `poll_once` (§8.4); `Kit`'s control from `Walks`; `worker_walks` removed |
| `htui-worker/src/worker.rs` | `COMMAND_POLL`; the loop arm (§8.5) |
| `htui-worker/src/lib.rs` | re-exports (`:49-52`) |
| `crates/htui/src/run_worker.rs` | `TuiRuns::poll_commands`; `:176` import; the pinned test replaced; §8.7 tests; `Play::Park` |
| `crates/htui/src/store_worker.rs` | the command-poll arm beside the sweeper (`:2199-2201`) |

### 8.2 `Walks` (`runtime.rs:503-578`)

```rust
/// A run's parent token, how many tasks work under it, and its signal (MOD-42 plan D11).
#[derive(Debug, Clone)]
struct Parent {
    token: CancellationToken,
    live: Arc<AtomicUsize>,
    /// D11: `Cancel` asks every walk of the run to stop gracefully before the token drops it.
    signal: watch::Sender<Signal>,
    /// The channel's own receiver; the control lookup hands out clones (F-17).
    control: watch::Receiver<Signal>,
}
// `child` mints `let (signal, control) = watch::channel(Signal::Run);` with the parent.
// If the pinned tokio's `watch::Sender` is not `Clone`, hold it as `Arc<watch::Sender<Signal>>`.

impl Walks {
    /// D10: the run's control: its live parent's receiver, else one no signal reaches.
    fn control(&self, run: RunId) -> Control {
        self.lock()
            .get(&run)
            .map_or_else(Control::never, |parent| Control::new(parent.control.clone()))
    }

    /// D11 (R-38): `Cancel { grace }` to every walk of `run`; wait until none is live or
    /// `grace + 1 s`; then [`Self::preempt`] as before (a walk still live is dropped hard). The
    /// parent stays in the map while this waits, so a walk that looks its control up meanwhile
    /// reads the cancel. Whether there was a parent.
    async fn preempt_gracefully(&self, run: RunId, grace: Duration) -> bool {
        let live = {
            let walks = self.lock();
            let Some(parent) = walks.get(&run) else { return false };
            parent.signal.send_replace(Signal::Cancel { grace });
            Arc::clone(&parent.live)
        };
        let deadline = tokio::time::Instant::now() + grace + Duration::from_secs(1);
        while live.load(Ordering::SeqCst) > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.preempt(run)
    }
}
```

`Kit::read` builds `control` from `shared.walks.clone()`: `Box::new(move |run| walks.control(run))`.
`preempt_all`, `close`, `forget_server` and `shutdown` are unchanged (OQ-5: hard drops).

`on_run` (`:1912-2027`): `enum Preempt { IfLive, Never }` (`Always` gone, B-20); the stop
becomes `if preempt == Preempt::IfLive && ctx.shared.walks.is_live(run) {
ctx.shared.walks.preempt_gracefully(run, ctx.shared.cancel_grace).await; }`. The `LeaseHeld`
arm that answered `worker_walks` becomes, for OQ-3:

```rust
        Some(Err(EngineError::LeaseHeld { run: held }))
            if promoting && kit.tails == Tails::HandBack =>
        {
            ctx.refuse(promote_needs_the_walker(held));
        }
```

(`let promoting = matches!(command, Command::PromoteStep { .. });` replaces `cancelling`). At each
`walked(&walk, …)` site (`:1547`, `:1741`, `:1886`, `:1953`, `:2066`), before the generic
`Some(Err(err))` arm (B-3):

```rust
        // MOD-42 B-3: a walk that stopped on its control was preempted, gracefully.
        Some(Err(EngineError::Cancelled { .. })) => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
```

### 8.3 `cancel_run` (D12; `run_request` routes `Command::CancelRun { run }` here)

```rust
/// MOD-42 plan D12: every cancel of a live run is a durable command its executor applies.
/// `existing` is a polled row (D13): no second row is written for it, and a refusal it meets is
/// not published (B-10).
async fn cancel_run<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    run: RunId,
    live: &LiveChats,
    existing: Option<RunCommandId>,
) {
    let row = ctx.tag(run).await;
    // D212's chat check first, as `on_run`'s (a polled cancel stays pending, logged at debug).
    // … (on_run's block, with `if existing.is_some() { debug; return; }` in the refusal arm)
    let Some(row) = row else { return ctx.refuse(/* NotFound { run } sentence */) };
    // B-20: a queued or terminal run takes today's path and writes no row.
    if existing.is_none() && (row.status == RunStatus::Queued || row.status.is_terminal()) {
        return on_run(ctx, run, Command::CancelRun { run }, Preempt::Never, live).await;
    }
    let Some(writer) = ctx.host.writer() else { return ctx.refuse(DATABASE_UNREACHABLE.to_owned()) };
    let (box_id, user) = match (registered_box(&ctx.host).await, ctx.host.this_user().await) {
        (Ok(box_id), Ok(user)) => (box_id, user),
        (Err(err), _) | (_, Err(err)) => return ctx.refuse(err.to_string()),
    };
    // D12 step 2: the row first.
    let (id, already) = match existing {
        Some(id) => (id, false),
        None => match htui_core::store::WorkerStore::request_cancel(&writer, run, user, box_id).await {
            Ok(CancelRequest::Inserted(id)) => (id, false),
            Ok(CancelRequest::AlreadyPending(id)) => (id, true),
            Err(err) => return ctx.refuse(err.to_string()),
        },
    };
    // B-5: one task per row at a time.
    let Some(_applying) = ctx.shared.applying(id) else { return ctx.requested(true) };
    if ctx.shared.walks.is_live(run) {
        // D12 step 3: this process walks it.
        ctx.shared.walks.preempt_gracefully(run, ctx.shared.cancel_grace).await;
    } else if row.executing_box_id != Some(box_id) {
        // D12 step 4, decided before `cancel_leased` (whose refusal there is `RunStatus`).
        return ctx.requested(already);
    }
    let walk = ctx.shared.walks.child(run);
    let Some(guard) = ctx.shared.lock_unless_cancelled(run, &walk).await else {
        return ctx.refuse(PREEMPTED.to_owned());
    };
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => return ctx.refuse(message),
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    let resolve = |to, why| htui_core::store::WorkerStore::resolve_command(&kit.writer, id, to, why);
    match walked(&walk, engine.dispatch(Command::CancelRun { run })).await {
        None => {
            engine.abandoned(run).await;
            ctx.refuse(PREEMPTED.to_owned());
        }
        Some(Ok(outcome)) => {
            if let Err(err) = resolve(RunCommandStatus::Applied, None).await {
                tracing::warn!(%run, %err, "the applied cancel's row was not resolved");
            }
            ctx.done(outcome);
        }
        // D12 step 5: a live lease elsewhere on this box (the worker): its poll applies it.
        Some(Err(EngineError::LeaseHeld { .. })) => ctx.requested(already),
        // D13: already terminal → refused with the actual status.
        Some(Err(err @ EngineError::RunStatus { status, .. })) if status.is_terminal() => {
            let sentence = err.to_string();
            if let Err(err) = resolve(RunCommandStatus::Refused, Some(sentence.clone())).await {
                tracing::warn!(%run, %err, "the refused cancel's row was not resolved");
            }
            ctx.refuse(sentence);
        }
        // Anything else is transient: the row stays pending and the next poll retries.
        Some(Err(err)) => ctx.refuse(err.to_string()),
    }
    drop(guard);
}
```

`TaskCtx::requested(&self, already: bool)` publishes `FrameKind::Changed` for the tagged run and
answers `RunReply::Failed { request: self.name, message: (if already { CANCEL_ALREADY_REQUESTED }
else { CANCEL_REQUESTED }).to_owned() }` (B-12). `Shared::applying(&self, id) ->
Option<ApplyingGuard>` inserts into the set, `None` when present; the guard removes on drop.

### 8.4 The poll

```rust
    pub fn poll_commands_with(&mut self, host: &H, sink: &P, live: LiveChats) {
        if host.writer().is_none() || self.shared.walks.closed() {
            return;
        }
        if self.shared.polling
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.shared.publisher.wire(sink);
        let ctx = TaskCtx { shared: Arc::clone(&self.shared), host: host.clone(),
            sink: sink.clone(), addr: None, name: "cancel_run", tag: Arc::default() };
        // spawned and tracked exactly as `sweep_with` (`:1013-1026`), with a drop guard that
        // clears `polling`; the task runs `poll_once(ctx, live)`.
    }

async fn poll_once<H, P>(ctx: TaskCtx<H, P>, live: LiveChats) {
    let Some(writer) = ctx.host.writer() else { return };
    let box_id = match registered_box(&ctx.host).await { Ok(id) => id, Err(err) => { warn; return } };
    match htui_core::store::WorkerStore::pending_commands(&writer, ctx.shared.owner, box_id).await {
        Ok(rows) => for row in rows {
            if ctx.shared.is_applying(row.id) { continue; }
            let task = ctx.unaddressed("cancel_run");
            let live = live.clone();
            spawn_supervised(task.clone(), async move {
                cancel_run(task, row.run_id, &live, Some(row.id)).await;
            });
        },
        Err(err) => tracing::warn!(%err, "reading the pending run commands failed"),
    }
}
```

### 8.5 The two loop arms

`htui-worker/src/worker.rs` `run` (`:44-71`), beside `poll`:

```rust
    let mut commands = tokio::time::interval(COMMAND_POLL);
    commands.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // in the select, after the poll arm:
            _ = commands.tick() => runtime.poll_commands_with(&host, &Unaddressed, LiveChats::default()),
```

`crates/htui/src/store_worker.rs`, beside the sweeper (`:2199-2201`), with the ticker declared
beside `sweeper` (`:1792`):

```rust
                _ = commands.tick(), if backend.writer().is_some() => {
                    let live = live_chats(&runtime);
                    runs.poll_commands(&backend, &tx, &live);
                }
```

### 8.6 Commits

1. **red** — constants, `Shared` fields, `with_cancel_grace`, `poll_commands_with` as a no-op,
   `TuiRuns::poll_commands`, `Play::Park`, the tests of §8.7 (the pinned
   `on_a_worker_box_a_live_walk_refuses_cancel_naming_the_worker`, `run_worker.rs:2740-2776`, is
   replaced by its successor). `worker_walks` still exists (the old arm still uses it). Red: the
   cancel is refused naming the worker, a parked walk is hard-dropped with no `cancelled` answer,
   the poll does nothing.
2. **green runtime** — §8.2, §8.3, §8.4, B-3 arms, `Kit`'s control, `worker_walks` and its
   re-export removed, `:176` import fixed.
3. **green loops** — §8.5.

### 8.7 Tests (written first)

`crates/htui/src/run_worker.rs` `mod tests` (the MemStore `Fixture`; fake parts only).
`Play::Park(PermissionRequestEvent)` plays `ToolCall`, `ParkPermission`, `Done`, and records the
grace its session's `cancel` received in a shared cell.

| Test | Asserts | Red because |
|---|---|---|
| `an_in_process_cancel_answers_the_parked_request_and_ends_gracefully` | `R` with `Play::Park`; wait for the pending row; `c` → `CommandOutcome::Cancelled`; the step `cancelled` by `cancel_leased`, never `failed`; I-7's row once; the relay row `cancelled`; the session's cancel grace = the runtime's (`with_cancel_grace(3 s)`); the command row `applied`; the walk's own requester answered `PREEMPTED`. | hard drop: no answer row, no grace |
| `a_promote_preempt_is_graceful` | `p` on a step whose sibling session is parked live → the parked request answered `cancelled` (I-7) before the promotion lands; promotion answers as today. | hard drop |
| `on_a_worker_box_cancel_writes_a_pending_command_and_says_requested` (replaces the pin) | Executor `worker`; a stranger's day-long lease; `c` → `Failed { request: "cancel_run", message: CANCEL_REQUESTED }`; one `pending` cancel row; the run untouched. | `worker_walks` sentence |
| `a_second_cancel_says_already_requested` | `c` twice → the second answers `CANCEL_ALREADY_REQUESTED`; still one row. | `worker_walks` sentence |
| `the_workers_command_poll_applies_a_pending_cancel` | A second runtime with role `Worker` whose owner holds the lease (the `testing` hooks MOD-41 added, or a `Fixture` runtime that took the lease) runs `poll_commands_with` once → the run `cancelled`, the row `applied`. | no-op poll |
| `a_cancel_of_a_run_on_another_box_stays_pending` | `executing_box_id` set to another box → `CANCEL_REQUESTED`, row `pending`, no `RunStatus` refusal (D12 step 4). | today: `RunStatus` |
| `a_pending_cancel_of_a_finished_run_is_refused_with_its_status` | Row pending, the run then `finish_run(Done)`; one poll → row `refused`, `resolution` names `done` (B-4). | no-op poll |
| `overlapping_command_polls_run_one_poll` | Two `poll_commands_with` back to back while the first's task is held (a held `RunLocks` entry, or a wrapper host whose `pending_commands` waits on a `Notify`) → one cancel task; the row applied once (B-5 and the flag). | no-op poll |
| `promote_of_a_worker_walked_step_names_the_worker` | OQ-3: `p` on a worker box under a stranger's live lease → `promote_needs_the_walker(run)`. | generic `LeaseHeld` sentence |
| `shutdown_still_drops_a_parked_walk` | OQ-5 pin: `shutdown(grace)` with a parked walk ends within `2 × grace`; the relay row is left `pending` and is absent from `relay_view` once the lease is released. | green on arrival (a pin) |

### 8.8 Gate

G-T4.

---

## 9. T6 — end-to-end and docs (serial, last)

### 9.1 Files

`crates/htui/tests/worker_pg.rs`, `crates/htui/tests/runs_pg.rs`, `docs/htui-worker.md`,
`crates/htui/src/cli.rs` (`:75`'s MOD-42 sentence).

### 9.2 Tests (Postgres; `testkit::demo_db`; in-process `worker::run`, never the binary)

`worker_pg.rs`'s `Parts` gains `runtime_with(builder)` so a case registers a transport that plays a
parking script (`Walks` stays the default). The second client is
`PgStore::connect(&db.url, &identity::load_or_mint(tmp.path())?)` over a `tempfile::tempdir()`.

| Test | Asserts |
|---|---|
| `worker_pg.rs::a_worker_parked_step_resumes_on_an_answer_from_another_box` | Executor `worker`; the worker loop walks a queued run whose session parks; the second client's `relay_view` shows the row; it answers; the step is `done` **within 3 s** of the answer at the production 1 s poll (PRD metric). |
| `worker_pg.rs::a_cancel_from_another_box_ends_a_live_worker_walk` | The parked worker walk; the second client calls `request_cancel` (as a TUI on another box would); the run is `cancelled` within `grace + COMMAND_POLL + 2 s`; the relay row `cancelled`; I-7's row in the step's log; the command `applied`. |
| `runs_pg.rs::an_in_process_walk_resumes_on_an_answer_from_another_box` | The TUI runtime (executor `tui`) walks a parking session; the second client answers; the step completes; the TUI's own `relay_view` is empty afterwards. |

They are expected green on arrival: a red one is a defect in T1-T5, fixed in place in its own
commit, named in the message.

### 9.3 Docs

`docs/htui-worker.md`: permission requests on worker steps (policy first, then the Runs pane of
any TUI holding the DSN; answers are applied within about a second); cancel of a worker-walked run
(durable, "cancel requested", applied within grace plus one poll; a cancel whose executor is down
waits, OQ-2); `p` on a worker-walked live step stays refused (OQ-3); shutdown is a hard drop
(OQ-5); **migration order**: `0011` is applied from a TUI first, a headless worker built with it
waits until then (MOD-40 C5). The MOD-41 refusal text (`:56-58`, `:114`) is removed.
`cli.rs:75` stops naming MOD-42 as a future fix.

### 9.4 Gate

G-Final.

---

## 10. Wave schedule and lane rules

| Wave | Tasks | Where | Postgres |
|---|---|---|---|
| W0 | T0 | primary | none (every `htui-store` run with the DSN unset: the placeholders would fail it) |
| **W1** | **T1 ∥ T2 ∥ T5** | T1, T2 primary; T5 worktree `hr/MOD-42-t5` | T1 only |
| W1-merge | merge T5; G-W1 | primary, no lane running | yes, under the lock |
| W2 | T3 | primary | none in its gate |
| W3 | T4 | primary | `htui` suite under the lock |
| W4 | T6 | primary | full gate |

1. **Branch first.** T5: `git worktree add -b hr/MOD-42-t5 /home/mluigi/projects/htui-wt/mod-42-t5
   <T0 green sha>`. T1 and T2 commit on `hr/MOD-42`.
2. **Crate-scoped commands only during W1.** T1 runs only `-p htui-store` commands (and `cargo
   sqlx` inside `crates/htui-store`); T2 only `-p htui-agent`; T5 only `-p htui`. No lane runs a
   `--workspace` command, `cargo fmt --all` or a workspace clippy until G-W1; `cargo fmt -p
   <crate> -- --check` instead.
3. **No lane edits `htui-core` in W1.** A shape in §2 that turns out wrong is reported to the main
   thread; a fix to `htui-core` waits for the merge.
4. **One Postgres lane.** Every command with `HTUI_TEST_DATABASE_URL` set, and every `cargo sqlx`
   command, runs under `flock /tmp/mod42-pg.lock`. T2 and T5 run with `env -u
   HTUI_TEST_DATABASE_URL` (the Postgres suites skip). `.sqlx` is T1's alone.
5. **`--all-features`** on every test command: without `testkit`, `crates/htui/tests/*.rs` compile
   to empty binaries and print `ok. 0 passed` — a wrong command, not a pass. `--test-threads=1`
   everywhere (the keyring fake is process-wide; the suite is scheduling-dependent).
6. **Reads and edits.** Primary lanes use Gortex; the T5 worktree uses native tools (Gortex does
   not index it and its edit tools write the primary checkout).
7. **Commit incrementally**, red then green, on the lane's branch. Never stash.
8. **Merge.** After T1 and T2 have committed and no build runs: `git merge --no-ff
   hr/MOD-42-t5`. Expected conflicts: none (disjoint files; T5's snapshot is new). Then G-W1 with
   **no lane running**; then `git worktree remove --force /home/mluigi/projects/htui-wt/mod-42-t5`
   and `git branch -d hr/MOD-42-t5`, in that order. Before `scripts/hr down --purge`, the branch is
   merged or fetched to the host (`git fetch <run_dir>/src hr/MOD-42-t5:hr/MOD-42-t5`).
9. **Hidden coupling to expect at the merge**: T2 changed `pump`'s body, which every
   `htui-orch`/`htui-worker`/`htui` session test drives — G-W1 runs all of them; T1's `0011` makes
   every Postgres harness migrate one more file.
10. **After every lane and gate**: `pgrep -af 'htui worker'` prints nothing; `ls ~/.config/htui`
    shows no `trees/<run>` a test made; `df -h /` before any Postgres-heavy gate (a crash loop is
    disk pressure first).

---

## 11. Pins

| Pin | Now | After | Where it moves |
|---|---|---|---|
| Store conformance `CASES` | 104 | 116 | T0 +12 (`mem_store.rs:36-37`, `pg_conformance.rs:21`, `:19-20`, `:28`) |
| `READ_CASES` | 14 | 14 | — |
| `htui-orch` `CASES` | unchanged | unchanged | T3's tests are `engine.rs` unit tests |
| Migrations | 10 | 11 | T1 (`migrations.rs`, `connect.rs`) |
| Postgres tables | 39 | 41 | T1 (`migrations.rs` `TABLES`) |
| `.sqlx` files | 291 | 291 + the distinct new query texts | T1 (counted and restated in its commit message) |
| `StoreRequest` / `StoreReply` | 91 / 52 | 93 / 54 | T5 (D15; not pinned by a test) |
| `crates/htui/tests/snapshots` | 121 | 122 | T5 |
| `WorkerStore` / `RecorderStore` / `RelayStore` / `WorkerHost` own methods | 42 / 3 / — / 22 | 45 / 3 / 4 / 22 | T0 (`store/worker.rs` doc) |
| `EngineParts` fields | 16 | 18 | T3 (docs `engine.rs:390`, `:6340`) |
| `WriteStore` methods added | — | 9 | T0 (F-1) |
| Unchanged | | | `MIRRORED_TABLES`, `GraphSource` 7, workspace members, `pump`'s 26 call sites, `WorkerConfig` literals |

Each moved pin's assertion message names its reason ("MOD-42 T0's twelve relay cases", "MOD-42's
0011_permission_relay.sql"). The implementer re-counts at each gate.

---

## 12. Gate reference

```bash
LOCK="flock /tmp/mod42-pg.lock"                    # anything touching Postgres
NOPG="env -u HTUI_TEST_DATABASE_URL"               # Postgres suites skip
pg()   { $LOCK cargo test -p "$1" --all-features -- --test-threads=1; }
nopg() { $NOPG cargo test -p "$1" --all-features -- --test-threads=1; }
lint() { cargo clippy -p "$1" --all-targets --all-features -- -D warnings; }
fmtp() { cargo fmt -p "$1" -- --check; }
# T1's scratch database for `cargo sqlx prepare` (docs/hr-sandbox.md:196-205); migrate it again
# after 0011 exists:
#   psql -h localhost -p 5439 -U postgres -c 'CREATE DATABASE htui_sqlx_mod42;'
SQLX_DB=postgres://postgres@localhost:5439/htui_sqlx_mod42
regen() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB sh -c \
  'cargo sqlx migrate run --source migrations && cargo sqlx prepare -- --all-targets --all-features'); }
check() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB \
  cargo sqlx prepare --check -- --all-targets --all-features); }
```

| Gate | Commands |
|---|---|
| G-T0 | `nopg htui-core`; `nopg htui-store` (the `case_list_matches_mem_store` pin runs; Postgres cases skip); `nopg htui-agent`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo fmt --all -- --check`; `grep -rn "MOD-42 T0: red" crates` empty |
| G-T1 | `regen` then `check`; `pg htui-store`; `lint htui-store`; `fmtp htui-store`; `grep -rn "MOD-42 T1: not yet implemented" crates` empty; `ls crates/htui-store/.sqlx \| wc -l` restated |
| G-T2 | `nopg htui-agent`; `lint htui-agent`; `fmtp htui-agent` |
| G-T5 (worktree) | `nopg htui`; `$NOPG cargo insta test -p htui --all-features -- --test-threads=1` with nothing pending; `lint htui`; `fmtp htui` |
| G-W1 (merged, no lane running) | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `check`; `$LOCK cargo test --workspace --all-features -- --test-threads=1` |
| G-T3 | `nopg htui-orch`; `nopg htui-worker`; `$NOPG cargo test -p htui --all-features run_worker -- --test-threads=1`; `lint htui-orch`; `lint htui-worker`; `cargo fmt --all -- --check` |
| G-T4 | `nopg htui-worker`; `pg htui` (includes `runs_pg`, `worker_pg`); `lint htui-worker`; `lint htui`; `cargo fmt --all -- --check` |
| G-Final | The plan's Validation list: `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `$LOCK cargo test --workspace --all-features -- --test-threads=1`; `check`; `cargo insta test --workspace --all-features` with nothing pending (DSN unset); `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; the §10 rule 10 checks |

---

## 13. Implementation amendments (recorded by the main thread)

Deviations the lanes made on evidence, accepted by the main thread; the code and its tests are
binding where they differ from §2-§9 above.

- **A-1 · Actor before status, both stores (T0 deferred → T1).** `answer_permission` and
  `request_cancel` check, on a miss: `NotFound` (row / run), then the actor (`Constraint`,
  B-7), then status / `NotOffered` / `ExecutorGone` (resp. `AlreadyPending`). MemStore's order
  (§2.11) is the spec; PgStore's re-reads follow it (`pg/relay.rs`), and §2.10's `live!` column is
  dropped (with the row pending and the option offered the answer is `ExecutorGone` either way).
  Pinned by `pg_criteria.rs::an_unknown_actor_is_refused_before_the_status`. (T1 `f066694`.)
- **A-2 · A fenced exit settles nothing (T2, amends B-15).** `drive` marks leftover rows `stale` on
  every exit except `Cancelled` **and `Store(Fenced)`**: a writer that lost its lease writes nothing
  more (MOD-40 D1). The row stays `answered`, is unanswerable (`ExecutorGone`), is hidden by
  `relay_view`, and is staled by the adopter's next `open_permission` on the step (D5). Pinned by
  `an_answer_whose_lease_is_gone_is_fenced`. (T2 `9e93de2`.)
- **A-3 · The post-cancel drain stops at the turn's `done` (T2, amends plan D6 "to the stream's
  end").** ACP's stream outlives a cancelled turn, so draining to `None` would make every cancel
  wait the full `grace + DRAIN_SLACK`. The cancel's synthesized `tool_result`s arrive before the
  `done`; the log still ends `done {stop_reason: cancelled}` (I-7). B-11's bound covers only the
  pulls, never a recording in flight (`the_drain_bound_never_drops_a_row_mid_flush`). (T2
  `9e93de2`, `13123e0`.)
- **A-4 · `Control::changed` as an if-let chain (T2).** §2.5's match-guard body does not compile
  (E0596); same meaning.
- **A-5 · One answer in flight per relayed request (T5).** A second digit before the first
  `AnswerPermission` reply is consumed and sends nothing: the app's staleness gate keys on
  `(origin, discriminant)` and would otherwise drop the winning reply and show the loser's
  `ALREADY_ANSWERED`. (T5 `cbbc061`.)
- **A-6 · Offline harness test dispatches `RelayView` directly (T5).** `seed_mirror` seeds no
  items, so the offline Runs pane is unreachable in the harness; the pane's empty-view path is
  pinned by a `runs.rs` unit case instead. (T5 `a339e4d`, `cbbc061`.)
- **A-7 · A lost fence settles nothing (T3, amends F-16).** F-16 recorded the fenced road through
  `fail_hard` as pre-existing; the relay's fenced apply (D4) is a **new** source of `Fenced`, and a
  probe showed the stale walk writing `failed` on a run another process held — and, for a fan-out
  candidate and a judge call, swallowing the error or parking the run. `walk_step`,
  `run_candidate` and `run_judge` now return before any failure write when `is_fenced(&err)`, as
  they do for `Cancelled`; the group fold `group_error` prefers cancel, then a lost fence, then the
  first error. Pinned by the single-step, candidate and judge "applied after the lease moved"
  cases. (T3 `b855ac6`, `46d12af`.) Kit's production policy lookup is `policy_lookup` with its own
  unit tests (`7ab5a8a`). Stack: `every_case_name_dispatches` still passes at 1.6 MiB (`9d65f73`).
- **A-8 · A queued/terminal cancel preempts live work gracefully (T4, amends B-20).** `cancel_run`'s
  fallback calls `on_run(…, Preempt::IfLive)`, not `Never`: with `Never`, a cancel of a run whose
  own start or claim task is live would wait behind that task's whole walk; MOD-41's
  `Preempt::Always` stopped it, and `IfLive` keeps that stop, now graceful. Still no row. Pinned by
  `a_cancel_of_a_queued_run_stops_its_live_claim_first`. (T4 `37c5884`, `f17aece`.)
- **A-9 · A lock wait honours the run's `Cancel` signal (T4, extends §8.2).** `WalkToken` carries a
  receiver of its parent's signal and `lock_unless_cancelled` returns on `Cancel`, so a command
  queued behind a cancelled walk leaves (refused `PREEMPTED`) instead of holding `live > 0` for
  `grace + 1 s` and then walking the run. Pinned by
  `a_command_queued_behind_a_cancelled_walk_leaves_at_the_signal`. (T4 `37c5884`, `f17aece`.)
- **A-10 · Polled cancels are silent; a cancelled run resolves `applied` (T4, extends B-5/B-10).**
  Every refusal of a cancel picked up by the poll (not only the live-chat one) is logged at `debug`
  and publishes nothing; inline cancels still answer their requester. Two processes on one box can
  both pick up a row (the poll's read can predate the other's claim); the loser finding the run
  already `cancelled` resolves the row `applied`, not `refused`. (T4 `f17aece`, `58d372a`,
  `7d377d6`.)

### Review gate (rust-reviewer, APPROVE WITH CHANGES; each finding adversarially verified)

Confirmed and fixed: M-1, M-2, M-3, L-1, L-4. Refuted by their verifiers: L-2 (no production
transport keeps the drain running past the preempt window) and L-3 (a 10 ms poll on a rare path,
no wrong behaviour).

- **A-11 · I-7 covers drained requests; cancel writes are best-effort (M-1, M-3).** The cancel's
  drain answers every `PermissionRequest` it pulls with I-7's row (ACP answers all responders it
  holds, queued ones included). Every store write in `cancel()` is best-effort (`warn`) except a
  fence, so a graceful cancel always ends `Cancelled` (I-6). The parked poll rides out up to 29
  consecutive `Unreachable`/`Backend` reads (`TRANSIENT_READS = 30`, about 30 s), then fails as
  before. (`cc502f0`.)
- **A-12 · A cancel or lost fence wins over a failed `recorder.finish()` (M-2).** `session` and
  `candidate_live` close the recorder best-effort on `Cancelled`/fenced and return the original
  error, as `judge_sessions` already did; a scrub residue no longer turns a cancel into a failure
  write. (`fa1235d`.)
- **A-13 · A dropped walk's requests are staled (L-1, extends D5 and OQ-5).** `Engine::abandoned`
  and a same-owner lease re-take mark the run's `pending|answered` rows of this owner `stale`, so a
  hard drop (shutdown, panic, preempt timeout) leaves no answerable ghost. Shutdown is still a hard
  drop (no graceful cancel, no `cancelled` answer row) — OQ-5 stands; only the row's status moves
  from `pending` to `stale`. (`2e5e790`; `shutdown_still_drops_a_parked_walk` updated.)
- **A-14 · A queued cancel that loses to a claim goes durable (L-4).** When `cancel_run`'s queued
  fallback meets `LeaseHeld` or a non-terminal `RunStatus`, it writes the `run_command` row and
  answers "cancel requested". (`04f6431`.)
