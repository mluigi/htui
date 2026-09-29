# Plan: MOD-41 — Headless worker (`htui worker`)

**Status: DRAFT — awaiting fact-check and maintainer CONFIRM.**

**Source**: `.claude/prds/mod-41-headless-worker.prd.md`, all four milestones, with its binding gate
decisions PRD D1-D7 (maintainer, 2026-09-29, "all recomm"). Design: `docs/ANA-16.md` §5.5, §7,
§8 item 2, §9 hazards 1, 10, 13 and 15. Carried in from `docs/decisions/mod/mod-40.md` ("Left
open") and `docs/decisions/mod/mod-4.md` (T6, "What a later item must know"). Boundaries:
`HANDOFF.md` MOD-24 (`:634-653`), MOD-42 (`:681-690`) and MOD-43 (`:691-695`).

**Requirements**: `R-ORCH-12` (promoted to must, PRD D2), `R-ID-2` (amended, PRD D1), `R-STO-1`
(amended, PRD D3), `R-NF-1`, `R-NF-2`, `R-NF-3`, `R-STO-5` (as amended by MOD-40), `R-STO-8`
(index sync). The amendments are already in `docs/REQUIREMENTS.md` (`:25-27`, `:52-55`,
`:145-152`, `:259-261`, commit `955de97`).

**Complexity**: Large. One new crate, `htui-worker`. Three new `htui-core` traits:
- `RecorderStore`, 3 methods;
- `WorkerStore`, 42 methods;
- `WorkerHost`, 22 methods.

A fence on three more `WriteStore` methods and a monotonic heartbeat. One new `box.settings` key,
`executor`, and one new host read, `queued_runs_on_box`. One clap subcommand. A background job.
**No migration**, so the next one is still `0008`.

**Routing**: `/handoff-run MOD-41`, PRD path. Ultracode is recommended for implement (C4) and
review. Reviewer: `rust-reviewer` (`.claude/workflow-config.json`).

**Numbering**: decisions **D1…** are this plan's; the PRD's are cited as **PRD D1-D7**. Tasks are
**T1…**, risks **R-1…**, open questions **OQ-1…**, compile probes **P-1…** and invariants
**I-1…**.

**Tree reading**: HEAD `955de97`, branch `hr/MOD-41`. No `graphify-out/`. The MCP banner said
Gortex was not tracking this directory, but its `search` and `relations` answered anyway. Line
numbers were confirmed with `sed`/`grep`. Code paths are relative to `crates/`; docs paths are
relative to the repo root.

---

## Open questions for the maintainer (read these first)

- **OQ-1: what the TUI's run commands do on a box whose executor is the worker.**
  **Recommended: hand back** (D12). Every command that walks today keeps its guard and its leased
  window: `a`, `x`, `r`, `s`, `A` and `u` in its resume case. The window is the part that records
  the human's decision under a lease. Instead of walking the tail, the command releases the lease.
  That leaves the run `running` with no lease, and the box's worker adopts it at its next poll:
  - `adopt_runs` takes `running` runs whose lease is NULL or lapsed (`htui-store/src/pg/write.rs:3867-3873`);
  - `release_lease` stamps the expiry to now (`:3989`).

  The worker's `recover_run` then finishes the tail exactly as it finishes a crash at that point:
  - the frontier reconcile for an approval or a selection (`htui-orch/src/engine.rs:2141-2156`,
    plan D97);
  - D131's `settle_failed` for a rejection (`:2187-2200`);
  - the cursor for the rest.

  The run's lease is free when the human acts, because a parked run holds no lease (walks release on
  rest, `release_after_walk`, `engine.rs:1769-1778`). The TUI's take therefore never steals, and a worker still walking the
  run refuses the take with `LeaseHeld` (`engine.rs:1790-1814`). What changes is only who walks the
  tail. It costs one engine switch (`EngineParts::tails`) and one conformance case per command. Gated
  graphs stay usable, and a TUI exit after the answer no longer strands the walk.
  **Fallback**: grey `a/x/r/s/A/u` on a worker box with "the worker on this box walks this run;
  answer it after MOD-42", making worker mode useless for gated graphs until MOD-42. MOD-42 inherits
  the hand-back as the in-box half of its command rows. Note that production `a` and `A` stay greyed
  until MOD-11 anyway (MOD-4 R-50, `HANDOFF.md:54-56`), so the hand-back of those two is exercised
  in tests only until then.

- **OQ-2: what `htui worker` does on a box whose executor is not `worker`.**
  **Recommended: idle and re-check at every poll.** It still beats `box.last_seen_at`, logs the
  state once per change, and exits 0 on a signal. Reasons:
  1. PRD D4 makes the executor a setting that can flip without moving a live lease, so the process
     must survive a flip in either direction.
  2. A user unit with `Restart=on-failure` (the documented sample, D18) would crash-loop on a
     non-zero exit.
  3. MOD-45 installs the service before anyone flips the setting.

  Non-zero exits are kept for startup refusals: schema, no DSN, and connect failure (D14).
  **Alternative**: exit 2 with "box executor is `tui`; set it to `worker` in Settings > Boxes". This
  is more discoverable, but it forces a restart after every flip.

- **OQ-3: the Runs pane's view of a run it does not walk.** Today it re-reads only on its own
  process's frames, commands and item changes (`htui/src/ui/tabs/backlog/detail/runs.rs:330-349`,
  `:568-578`, `:1175-1208`). Nothing polls, and the crates contain no `LISTEN`.
  **Recommended: a poll backstop** (D16). While the selected item has an active run
  (`queued | running | awaiting_approval`), the Backlog tab re-sends `Runs(item)` and
  `RunActions(item)` every `RUNS_POLL` = 5 s (20 of the existing 250 ms ticks,
  `htui/src/app/update.rs:18`, `:98-108`). That is two indexed reads per 5 s per visible item, and
  there is no visual change for a run the TUI walks. MOD-43 adds `LISTEN`/`NOTIFY` hints and keeps
  this poll as its backstop (ANA-16 §5.5). This is not liveness: the pane cannot say whether a
  worker is running (MOD-43).
  **Alternative**: defer everything to MOD-43. The user then sees a worker's progress only by
  re-selecting the item.

- **OQ-4: cancelling a run the worker is walking.** Today `c` on a `running` run whose live lease is
  another process's is refused with `LeaseHeld` and writes nothing (`engine.rs:1603-1616`,
  MOD-4 D179). There is no in-box way to ask a worker to stop a walk without a new column, which
  would be migration `0008`, or MOD-42's command rows.
  **Recommended: keep the refusal, and word it.** On a worker box the sentence says "the worker on
  this box is walking run …; cancelling a live run needs MOD-42's cancel command". `c` on a
  `queued` or parked run works as today: queued is a direct `finish_run`, parked takes the free
  lease. MOD-42 inherits cancel-of-a-live-walk as a command row the worker polls.
  **Alternative**: a `run.cancel_requested_at` column (migration `0008`) the worker checks each
  poll. That is out of MOD-41's no-migration scope and duplicates MOD-42.

- **OQ-5: should a TUI on an executor-`tui` box claim the box's queued rows it did not queue
  itself?** Today the TUI claims only runs in its in-memory queue (`htui/src/run_worker.rs:795`,
  `:825`, `:1887`). A `queued` row from an earlier process is never claimed again. Nothing scans
  the database for queued runs (verified claims). After a flip from `worker` back to `tui`, any
  runs the TUI queued for the worker would be stranded.
  **Recommended: yes.** The TUI's sweep, gated on `executor = tui`, feeds `queued_runs_on_box`
  (D11) into its claim queue, which is the worker's own claim path (D14). This bends "TUI behaviour
  unchanged when no worker runs" for one case only: a queued row a previous TUI left behind now
  starts on the next sweep. **Alternative**: leave stranded rows stranded, so the user cancels and
  re-runs after a flip.

## Summary

Four milestones on one branch, each ending green and committed, pushed only with the maintainer's
OK:
- **M1** fences the engine's last three unfenced step writes and makes the heartbeat's self-fence
  step-proof.
- **M2** moves run supervision into a UI-free crate, `htui-worker`. The crate reaches the store
  only through three named `htui-core` traits and replies through a sink trait that knows nothing
  of tabs. The TUI keeps every path it uses through re-exports and behaves exactly as before.
- **M3** adds `box.settings.executor` and the rule that only the matching process claims, adopts
  or sweeps on a box (I-1). It adds `htui worker`, with headless connect, the PRD D3 DSN sources, a
  bounded pool, a poll that claims and adopts, a box heartbeat and signal shutdown. It also adds the
  TUI's hand-back and the Runs poll.
- **M4** runs the concepts-index sync as the worker's background job.

## Design decisions (settled here, not in code review)

### M1 — fence completion

- **D1: `StepFence` on `set_step_prompt`, `upsert_step_tree` and `record_commits`** (PRD D5,
  MOD-40 "Left open").
  - **Signatures.** `set_step_prompt(fence, step, digest, trim)`,
    `upsert_step_tree(fence, step, trees)` and `record_commits(fence, step, commits)`, with the fence
    first after `&self`, as MOD-40 D1 placed it (`htui-core/src/store/traits.rs:301`, `:316`,
    `:1140`). Declarations are at `traits.rs:523`, `:1219` and `:1228`.
  - **Postgres, `set_step_prompt`** (`htui-store/src/pg/write.rs:1648-1667`): gains `set_step_usage`'s
    predicate verbatim (`:1052-1068`), namely
    `AND EXISTS (SELECT 1 FROM run r WHERE r.id = run_step.run_id AND r.lease_owner IS NOT DISTINCT FROM $4 FOR SHARE)`.
    A miss goes to `fenced_or_missing` (`:187-196`).
  - **Postgres, the two transactional writes** (`:4560-4625`, `:4639-4671`): replace their
    `step_exists(&mut tx, step)` with one new private helper, `step_fence(&mut tx, step, fence)`. It
    runs `SELECT r.lease_owner FROM run_step s JOIN run r ON r.id = s.run_id WHERE s.id = $1 FOR
    SHARE OF r`. No row is `NotFound { entity: "run_step" }`; an owner that is not `fence.owner()`
    is `Fenced { step }`.
  - **Order of refusals.** Existence, then the fence, then the batch checks. This keeps today's
    precedence for a missing step and `append_events`' "existence first, then the fence"
    (`htui-core/src/store/mem.rs:1511-1513`).
  - **MemStore.** `State::set_step_prompt` (`mem.rs:1580`), `upsert_step_tree` (`:4550`) and
    `record_commits` (`:4585`) check `fence_holds` (`:1495-1505`) after the step's existence and
    before `check_step_batch`.
  - **Delegates.** `Writer` (`htui-store/src/writer.rs:444-445`, `:953-954`, `:960-961`), `UsageSpy`
    (`htui-agent/src/conformance.rs:711`) and `SpyStore` (`htui-agent/tests/recorder.rs:428`) pass
    the fence through.
  - **Queries.** One `query!` text changes (`set_step_prompt`, its file replaced) and one is new
    (`step_fence`); `step_exists` keeps its other callers. So `.sqlx` goes from 288 to 289.
- **D2: every engine call site passes `StepFence::Lease(self.parts.owner)`.** All twelve run under
  the process's own lease; there is no `Unleased` site (verified claims, "call-site
  classification"):
  - `set_step_prompt`: `engine.rs:3181`, `:3765`, `:4590`;
  - `upsert_step_tree`: `:3129`, `:3745`;
  - `record_commits`: `:1402`, `:3140`, `:3211`, `:3691`, `:3756`, `:3812`, `:4747`.

  `:1402` is `accept_artifact`, inside the lease taken at `:1373` and the heartbeat at `:1455`.
  `:4747` is `reconcile_done_step`, reached from walks, from command tails after `take_lease`, and
  from the sweep's `recover_run` under the lease `adopt_runs` + `renew_lease` took (`:1984-2024`).
  The sweep's own unfenced write stays `interrupt_step` only (MOD-40 OQ-1).
  A `Fenced` from any of these goes through `heartbeaten`'s existing `is_fenced` arm
  (`engine.rs:1729-1741`): guards released, `LeaseLost`, nothing given back. `release_trees`'
  best-effort `record_commits` (`:3691`) keeps warning and swallowing.
- **D3: the heartbeat fences on tokio's monotonic clock** (MOD-40 blueprint F-38,
  `.claude/plans/mod-40-m4.blueprint.md:88`; `recover.rs:119-120`).
  - **`heartbeat`.** The new signature is `heartbeat(refresh, written: tokio::time::Instant, times)`,
    with **no clock parameter**. `fence = written - refresh`. Each beat reads
    `let sent = tokio::time::Instant::now()` before `refresh(times.ttl)`, and a success sets
    `fence = sent + ttl_std - refresh`. `ttl_std` is `times.ttl.to_std()`, which is never negative
    because `LeaseTimes::from_app` bounds it to 0..=1 year (`recover.rs:61-74`). Sleeps,
    `timeout` and `retry` are unchanged (`recover.rs:124-169`).
  - **Engine.** `renew_lease` (`engine.rs:1828-1841`) answers `Option<tokio::time::Instant>`
    (`sent + ttl` with `sent` read before the store call), and so does `take_lease` (`:1790`).
    `claim` (`:685-722`) reads `sent` before `claim_run`. `walk_leased` (`:1681`), `heartbeaten`
    (`:1710`), `accept_artifact`'s window (`:1455`), the sweep (`:2006`) and the test helper
    `fresh_until` (`:1876`) change the type of `until` only.
  - **What does not change.** `Clock` still stamps every row (MOD-4 D8). A wall-clock step can
    no longer move a fence, and a fence can no longer be computed from the wall clock at all.
    `recover.rs`'s private `PausedClock` (`:502-523`) is deleted, and its tests assert tokio
    elapsed time directly.

### M2 — supervision library

- **D4: three traits in a new `htui-core/src/store/worker.rs`, re-exported from `store/mod.rs`.**
  Each method is justified by a call site at HEAD (verified claims). **There is no default body
  anywhere** (PRD constraint). Every method is declared
  `fn m(&self, …) -> impl Future<Output = Result<T>> + Send`, and implementors write `async fn`
  (P-2).
  - **`RecorderStore: Send + Sync`** (3 methods): `append_events` (`record.rs:1077`, `:1090`),
    `set_step_usage` (`:1169`) and `set_agent_box_quota` (`:1235`, chats only; `Recorder`'s bound
    needs it). `Recorder`, `pump` and `enforce_breach` move from `S: WriteStore` to
    `S: RecorderStore` (`record.rs:373`, `:472`, `:1797`, `:1848`).
  - **`WorkerStore: RecorderStore`** (42 methods). These are exactly the engine's, `gate`'s and
    `graph::resolve`'s calls, plus `ProgressSink`'s `write_document` (`htui/src/run_worker.rs:646`,
    no production caller until MOD-11, PRD D5):
    - **13 `ReadStore` reads:** `item`, `documents`, `runs`, `step_events`, `document`,
      `documents_of_kinds`, `upstream_summaries`, `project`, `run`, `run_steps`, `step_trees`,
      `step_commits`, `resolve_inputs`.
    - **5 `WriteStore` reads:** `repos`, `repo_box_paths`, `item_kinds`, `phases`, `command_runs`.
    - **23 writes:** `transition`, `set_step_prompt`, `create_run`, `claim_run`, `refresh_lease`,
      `adopt_runs`, `take_lease`, `release_lease`, `create_step`, `transition_run`,
      `transition_step`, `finish_step`, `interrupt_step`, `answer_gate`, `select_fanout`,
      `supersede_step`, `upsert_step_tree`, `record_commits`, `record_command_run`, `promote_step`,
      `finish_run`, `close_out`, `add_note`.
    - **`write_document`.**

    The seven methods only `graph::override_graph` uses are **excluded**: `update_item`,
    `create_step_graph`, `create_phase`, `set_skill_binding`, `skills`, `skill_versions`,
    `skill_bindings`. `override_graph` has no production caller (`htui-orch/src/graph.rs:413`,
    callers only in its tests at `:1596`, `:1771`, `:1837`), so it keeps `S: WriteStore`. This is
    45 of `ReadStore` + `WriteStore`'s 109 methods (23 + 86, `traits.rs:78-254`, `:263-1478`).
  - **`WorkerHost: Clone + Send + Sync + 'static`**, the supervisor's source. It has
    `type Store: WorkerStore + Clone + Send + Sync + 'static` and `fn writer(&self) ->
    Option<Self::Store>` (sync, `backend.rs:148`), plus 21 reads, 22 in M3:
    - **9 of `Backend`'s inherent reads, called directly:** `box_info` (`run_worker.rs:1119`),
      `this_user` (`:1228`), `app_settings` (`:898`, `:1229`, `:1958`), `box_profile` (`:1231`),
      `agents` (`:1236`, `:2387`), `box_row` (`:1175`), `repo_paths` (`:1136`), `workspaces`
      (`:1142`), `active_runs_on_box` (`:1968`).
    - **5 `ReadStore` reads on the host,** for `actions()` and the runtime: `item`, `documents`,
      `runs`, `run`, `run_steps` (`:295-312`, `:1767`, `:1868`, `:1911`, `:2185`).
    - **6 of `Backend`'s inherent reads, through `GraphSource`:** `resolve_graph`, `phase_agents`,
      `prompt_template`, `agent_boxes`, `bound_skills`, `missing_tags` (`:2366-2410`).
    - **M3's `queued_runs_on_box`** (D11).

    Every type these return is in `htui-core`: `BoxInfo` (`model/box_.rs:221`), `BoxRow` (`:27`),
    `RepoBoxPath` and `WorkspaceSummary` (`model/hierarchy.rs:184`, `:224`), and so on.
  - **Why the host is a trait too.** The PRD's metric is that "the engine and supervisor compile
    against the worker-store trait only". The worker's host is a `PgStore`, not a `Backend`
    (D14), and MOD-47's client implements all three traits. That this is not a `WriteStore`
    mirror (hazard 15) is checked by the table above: no method without a HEAD call site.
- **D5: explicit impls, never a blanket.**
  - **Why not a blanket.** P-1: `impl<T: WriteStore> WorkerStore for T` cannot prove its futures
    `Send` ("future cannot be sent between threads safely"), and the generic supervisor must
    `tokio::spawn` them. Without `+ Send` in the trait, the generic spawn fails the same way (P-1's
    first run).
  - **The impls:**
    - `MemStore` (in `worker.rs`, under `impl`);
    - `PgStore` and `Writer`, for `RecorderStore`/`WorkerStore`;
    - `PgStore` and `Backend`, for `WorkerHost`, in a new `htui-store/src/worker.rs`;
    - `UsageSpy` and `SpyStore`, for `RecorderStore` only.
  - **Bodies** forward to the `WriteStore`/`ReadStore`/inherent method by path
    (`WriteStore::claim_run(self, …).await`).
  - **Import hygiene.** A concrete receiver with both traits in scope is `E0034` (P-3). So **no
    module `use`s `WorkerStore` or `RecorderStore`**; bounds name them by path
    (`S: htui_core::store::WorkerStore`), which puts their methods in scope for the type parameter
    only. The compiler finds every violation. The fixes are UFCS or an import removed, never a
    rename.
  - **P-4** (T3's gate) re-runs P-2 on the real types: `tokio::spawn` of `Engine::<PgStore,…>::claim`,
    `Engine::<Writer,…>::claim` and a generic `RunRuntime<H, P>` future.
- **D6: the crate is `crates/htui-worker`** (workspace member 6, `Cargo.toml:2-3`).
  - **Dependencies:** `htui-core`, `htui-agent`, `htui-orch`, `htui-store` (for the `PgStore`
    host, `identity::config_root` at `run_worker.rs:903`, and `DATABASE_UNREACHABLE`), `chrono`,
    `serde_json`, `tokio` (`sync`, `rt`, `time`, `macros`), `tokio-util`, `tracing` and `uuid`.
    **Never `ratatui` or `crossterm`.** This is pinned by T6's test (D8).
  - **Contents**, moved from `htui/src/run_worker.rs:1-2411` with bodies unchanged except for D7's
    types:
    - `lib.rs`;
    - `runtime.rs`: `RunRuntime`, `Shared`, `Walks`, `RunLocks`, `Kit`, `TaskCtx`, the spawn and
      supervision helpers, `sweep_once` and the claim queue;
    - `views.rs`: `ORCH_NAMES`, `OrchRequest`, `OrchReply`, `Via`, `RunFrame`, `FrameKind`,
      `Enabled`, `ItemActions`, `RunActions`, `StepActions`, `LiveChats`, `actions`, and
      `StepAuthor`/`ProgressSink`/`RefusedDriver`;
    - `address.rs`: D7;
    - `graphs.rs`: `HostGraphs<H>`, which was `BackendGraphs`.
  - **The generic types.** `RunRuntime<H: WorkerHost, P: ReplySink>`. The engine type
    `WorkerEngine<'a>` (`run_worker.rs:1188-1197`) becomes generic over `H::Store`.
  - **`htui-orch/Cargo.toml:18-20`'s comment** is rewritten from "generic over `S: WriteStore`" to
    `WorkerStore`. The invariant itself (never `htui-store`) is unchanged.
- **D7: addressing without tabs.** `Origin` (`htui/src/store_worker.rs:78`) and `ReplyAddr`
  (`htui/src/agent_worker.rs:171`) stay in the TUI.
  - **`ReplySink` (library):**
    `trait ReplySink: Clone + Send + Sync + 'static { type Addr: Clone + Send + Sync + Debug + 'static; type Subscriber: Clone + Eq + Hash + Send + Sync + Debug + 'static; fn subscriber(addr: &Self::Addr) -> Self::Subscriber; fn send(&self, to: &Self::Addr, reply: RunReply); }`.
    `Publisher`'s map (`run_worker.rs:690-697`) becomes `HashMap<P::Subscriber, P::Addr>`.
    Re-subscribing replaces the entry, as the `Origin` key does today (`:719`).
  - **`RunRequest { Orch(OrchRequest), Stream { item }, Actions(ItemId) }`,** with `name()` answering
    exactly `StoreRequest::name`'s strings (`store_worker.rs:834-837`: the `ORCH_NAMES` entry,
    `"run_stream"`, `"run_actions"`).
  - **`RunReply { Orch(OrchReply), Frame(RunFrame), Actions(Box<ItemActions>), Failed { request, message } }`,**
    one-to-one with `StoreReply::{Orch, RunStream, RunActions, Failed}` (`store_worker.rs:1035-1062`).
  - **`RunServed<A> { Reply(RunReply), Deferred, Attach { addr: A, promoted, ended } }`.**
    `ChatEnd::after` becomes `fn after<F: Future<Output = ()> + Send + 'static>(self, task: F) ->
    impl Future<Output = ()> + Send + 'static`, which removes `agent_worker::ChatTask`
    (`run_worker.rs:591`).
  - **Chat promotion stays a TUI act.** The library returns `Attach`; the TUI's `on_run_served`
    (`store_worker.rs:1484-1518`) and `testkit.rs:173` hand it to `AgentRuntime::attach_promoted`,
    unchanged. The worker's sink is `Unaddressed` (`Addr = Subscriber = Infallible`). It serves no
    request, so it never promotes, and an `Attach` cannot be built for it.
  - **The TUI adapter** is `htui/src/run_worker.rs`, cut to the adapter plus its unchanged test
    module:
    - `TuiReplies(UnboundedSender<ReplyEnvelope>)` implements `ReplySink` (`Addr = ReplyAddr`,
      `Subscriber = Origin`) and maps `RunReply` to `StoreReply`;
    - `pub type RunRuntime = htui_worker::RunRuntime<Backend, TuiReplies>`;
    - a `serve(runs, backend, tx, envelope, live)` shim that turns a `RequestEnvelope` into
      `(ReplyAddr, RunRequest)`;
    - `pub use htui_worker::{…}` for every name the UI and tests import today
      (`app/update.rs:12`, `ui/tabs/chat/mod.rs:39`, `:66`, `ui/tabs/backlog/detail/runs.rs:48`,
      `:1336`, `agent_worker.rs:844`, `:858`, `:883`, `:1993`, `tests/backlog.rs:14`,
      `tests/chat.rs:19`, `tests/runs_pg.rs:38`).

    So `crate::run_worker::X` and `htui::run_worker::X` keep resolving. `StoreRequest` (85) and
    `StoreReply` (47) keep their variants.
- **D8: behaviour-preserving, proven by the existing tests.**
  - **The tests do not move.** `run_worker.rs`'s test module (`:2412-4621`) drives the runtime
    through the TUI's store loop (`Worker::spawn` → `spawn_with_runtimes`, `:2787-2794`) and names
    `crate::ui::tabs::TabId` (`:2454`). It therefore stays in `crates/htui`, and so does its
    `pub(crate)` fixture (`Fixture`, `Worker`, `start_run`, `step_at`, `only_run`, `parked`,
    `:2720-2931`). `store_worker.rs:3431-3433`'s `a_promotion_reaches_the_attach_hand_off` and
    `testkit.rs:710-761` keep importing it with no feature. Two lines change,
    `BackendGraphs(…)` → `HostGraphs(…)` (`:3467`, `:4445`).
  - **The library's own tests** (`runtime.rs`) run over `Backend::memory(MemStore::demo())` with
    `htui-store/demo` as a dev-dependency. They cover the parts only it owns: the `Publisher` over a
    recording `ReplySink`, `RunRequest::name`, and the executor gate (T9).
  - **`the_library_links_no_terminal_crate`** (`htui-worker/tests/deps.rs`) runs `cargo tree -p
    htui-worker -e normal --prefix none` through `env!("CARGO")` and asserts that no line starts
    with `ratatui` or `crossterm`. The same check is in Validation.

### M3 — `htui worker`

- **I-1: executor gate (safety invariant).** On a box, only the process whose role matches
  `box.settings.executor` claims queued runs, adopts, or sweeps. `tui` means the TUI's `RunRuntime`;
  `worker` means `htui worker`.
  - **Why it is an invariant.** Without it a TUI sweep adopts any `running` run on the box whose
    lease lapsed, whichever process owned it (`adopt_runs`, `pg/write.rs:3862-3911`; `sweep_once`
    counts every process's runs, `run_worker.rs:1967-1970`). The run then walks in the TUI, and a
    TUI exit interrupts it, which is exactly what PRD D4 and the TUI-exit metric rule out.
  - **Correctness does not rest on I-1.** If I-1 is broken (the flip window, D13, or a
    misconfigured box), correctness still rests on the lease CAS and the fences (MOD-40 plus D1). I-1
    makes ownership single. It does not make the store safe.
- **D9: `box.settings.executor`** (PRD D4).
  - **The type.** `BoxSettings` (`htui-core/src/model/box_.rs:240-252`) gains
    `executor: Executor`, with `enum Executor { Tui (default), Worker, Other(String) }`.
  - **Decoding.** It decodes through `#[serde(default, deserialize_with = "executor_lenient")]`:
    `"tui"` or `"worker"` map to their variants, any other string maps to `Other`, and any other JSON
    type (number, object, `null`) maps to `Other` too. The field **can never fail the struct's
    decode**. That matters because both of `BoxSettings`' readers decode the whole blob with
    `.ok()` (`pg/write.rs:3699-3701`, `mem.rs:3788-3800`), so a failing field would silently drop
    `max_concurrent_items` to its fallback.
  - **`Other(_)` fails closed.** Neither role executes. The TUI refuses `R` and the walking commands
    with "box executor `…` is not known to htui <version>".
  - **Doc fix.** `box_.rs:237-239`'s doc says `set_setting` writes the blob, which is wrong
    (`SettingRung` has no box rung, `traits.rs:2196-2204`); it is corrected.
- **D10: the first `box.settings` writer is `edit_box`** (`traits.rs:454-458` reserves exactly
  this). `BoxEdit` (`box_.rs:157-162`) gains `executor: Option<Executor>`. Postgres
  (`pg/write.rs:1461-1519`) adds
  `settings = CASE WHEN $6::text IS NULL THEN settings ELSE jsonb_set(settings, '{executor}', to_jsonb($6::text)) END`
  under the same `edit_version` CAS, so every other key (`max_concurrent_items`, `command_limits`,
  unknown ones) survives. MemStore mirrors it on its `Value`. `Other(_)` is refused with
  `Constraint("executor must be tui or worker")`, so the editor can only write the two known values.
  The trait doc drops "has **no** writer" and states the new invariant. All 21 exhaustive
  `BoxEdit { … }` literals in 10 files gain `executor: None` (mechanical sweep, T7).
- **D11: `WorkerHost::queued_runs_on_box(box) -> Vec<(RunId, DateTime<Utc>)>`.** It answers
  `SELECT id, queued_at FROM run WHERE target_box_id = $1 AND status = 'queued' ORDER BY queued_at,
  id`. There is `PgStore` inherent code (`pg/read.rs`, beside `active_runs_on_box` at `:1965-1987`),
  `MemStore` inherent code, and a `Backend` arm (offline → `orchestration_offline()`, as
  `backend.rs:569-575` does). No index covers `(target_box_id, status)` (`0001_init.sql:465-466`,
  `0003_orchestration.sql:50-52`). A per-user `run` table is small, so a sequential scan every 5 s
  is accepted (R-6) and **no migration** is added. `.sqlx` goes to 290.
- **D12: the TUI on an executor-`worker` box.** `Kit::read` (`run_worker.rs:1221-1260`) reads the box's
  `BoxSettings` once, from the `box_row` it already fetches for `command_limits` (`:1172-1186`). The
  table below is the whole behaviour (OQ-1, OQ-4):

  | key | today | executor `worker` |
  |---|---|---|
  | `R` start | `enqueue` + `claim` + walk (`run_worker.rs:2135`, `:2156`) | `enqueue` only: the run stays `queued`, `target_box_id` = this box (`engine.rs:638-651`). Reply `Started { run, rest: Rest { run: Queued, position: None, failure: None } }`; no new `CommandOutcome` variant |
  | `a`/`x` answer | take (free on a park), `answer_gate` + `unpark`, walk the tail (`engine.rs:794-857`) | same window, then `release_lease`, no tail. Recovered by the frontier reconcile (approve) or D131 (reject) |
  | `r` retry | take, answer/retire + `unpark`, `admit` + walk (`:964-1064`) | window **plus `admit`**, then release. Without `admit`, a retried `failed` step would read as D131's unfinished failure (`:2187-2200`) |
  | `s` select | take, `select_fanout` + note + `unpark`, reconcile + walk (`:1075-1131`) | window, then release. Frontier reconcile |
  | `A` accept | take, verify + capture + `finish_step` under the heartbeat (`:1373-1455`), then `a`'s path | verify window in the TUI (same box, bounded by the phase deadline, MOD-4 D211), then `a`'s hand-back |
  | `u` unblock | `Reopen`/`FollowRun`: no lease; `Resume`: take, `resume_window`, `walk_resumed` (`:2680-2690`, `:2786-2808`) | `Resume`: window, then `unpark` when `resumable_park` (`status.rs:313-318`), then release. Frontier reconcile |
  | `p` promote | take, promote, release (`:1165-1219`) | unchanged. A step the worker is walking refuses with `LeaseHeld`; a parked step promotes and its chat runs unleased in the TUI |
  | `c` cancel | queued: `finish_run`; parked: take + cancel; live foreign lease: `LeaseHeld` (`:1603-1640`) | unchanged; the `LeaseHeld` sentence names the worker (OQ-4) |
  | `C`, `T`, `o` | no lease | unchanged |

  - **The engine switch.** `EngineParts` (`engine.rs:380-391`) gains `pub tails: Tails` (`Walk` |
    `HandBack`). A new field is a compile error at every constructor: `run_worker.rs:1282-1298`,
    `htui-orch/src/fake.rs`, `tests/gix_isolator.rs` and engine tests. `HandBack` replaces
    `walk_leased(run, until, tail)` in the five commands above with
    `release_lease(run); resting(run)`. The window is untouched.
  - **Who sets it.** `Tails::HandBack` is set by the runtime when role = `Tui` and executor =
    `Worker`. The worker's own engine is always `Walk`.
  - **Pane feedback.** The pane sees the command's `Changed` frame and then OQ-3's poll.
- **D13: the flip window** (PRD D4: "flipping never moves a live lease").
  - **`tui` → `worker`.** The TUI reads the executor at each sweep and each command. Its in-flight
    walks keep their leases and heartbeats and rest normally; its next sweep is skipped, and its
    next command hands back. The worker adopts only released or lapsed leases, and a TUI that exits
    gives its walks' leases back (`shutdown` → `abandoned`, `run_worker.rs:1657-1683`). So the
    worker adopts them at its next poll, as `running` runs whose steps the sweep resets and retries
    (ANA-2 §4.9, MOD-24's semantics).
  - **`worker` → `tui`.** The worker stops claiming and sweeping at its next poll, keeps
    heartbeating its live walks until they rest, then idles (OQ-2). The TUI starts sweeping and
    adopts only lapsed or released leases, and with OQ-5 it claims the queued rows.

  Tests pin both directions (T9).
- **D14: `htui worker`.**
  - **CLI.** `Args` (`htui/src/cli.rs:6-60`) gains `#[command(subcommand)] command:
    Option<Command>` with `#[command(args_conflicts_with_subcommands = true)]`, so every flat flag
    keeps parsing exactly as today (`cli.rs:69`, `:83`, `:121` stay green). `--log`/`HTUI_LOG`
    becomes `global = true`. `Command::Worker(WorkerArgs { pool_size: u32 (default 4),
    dsn_stdin: bool })` has no `env =` on any worker argument (tested structurally).
    `lib.rs:75-97`'s early-return chain gains a first arm, `Some(Command::Worker(args)) =>
    worker_cmd::run(args)`.
  - **Pool** (PRD D7). `PoolSize::clamped(n)` clamps to `2..=8` and warns when it moved. `open_pool`
    (`htui-store/src/pg/mod.rs:727-736`) takes `max_connections`. `connect_headless` (`:250-290`)
    gains a `pool: PoolSize` parameter; `concepts.rs:68` passes `PoolSize::TUI` (8, today's value).
    `connect_with` keeps 8. The pool is built before any setting is read.
  - **DSN** (PRD D3). `htui_store::secret::headless_dsn(sources) -> Result<Zeroizing<String>,
    DsnSourceError>` tries its sources in this order:
    1. with `--dsn-stdin`, one line from stdin and nothing else (explicit wins; the prompt goes to
       stderr only when stdin is a terminal);
    2. otherwise `secret::get_dsn()` (`secret.rs:95`), where an `Err` means "no keyring", not a
       failure;
    3. otherwise, on `cfg(target_os = "linux")`, `$CREDENTIALS_DIRECTORY/htui-dsn`.

    `CREDENTIALS_DIRECTORY` is a directory the unit's manager sets, not the secret; its path is
    passed in by the caller so tests need no `set_var`, which is `unsafe` in edition 2024 and
    `unsafe_code = "forbid"`. The DSN never comes from any other environment variable, `argv` or
    file. It is dropped (zeroized) right after `connect_headless` answers. `secret.rs:1-14`'s doc
    names the three sources. `set_dsn_from_stdin` (`htui/src/lib.rs:151-164`) switches to the same
    `Zeroizing` line reader.
  - **Connect.** `identity::load_or_mint(config_root)`, the same `box.toml` as the TUI and so the
    same box id, then `connect_headless`, then `connect::persist_registration` (`connect.rs:589`)
    for a copied `box.toml`. Every `HeadlessError` exits 2 with its sentence (`concepts.rs:68-79`'s
    mapping, shared). Nothing is written before the refusal (MOD-40 D8).
  - **Loop** (`htui_worker::worker::run(host: PgStore, runtime, config, shutdown)`). It is one
    `select!`:
    - the shutdown future;
    - `poll` every `WORKER_POLL` = 5 s. It reads the executor (`box_row(this_box)`); if it is
      `worker`, it runs `runtime.sweep` and then the claim scan (`queued_runs_on_box` → the
      runtime's existing claim queue, `Shared::queue` + `claim_queued`, `run_worker.rs:825`,
      `:1887-1904`). Otherwise it idles;
    - `box_beat` every `BOX_HEARTBEAT` (`connect.rs:38`) calls `touch_box` (`pg/mod.rs:604-614`),
      whatever the executor.

    A 5 s sweep is what bounds hand-back latency (D12). `sweep_once` still returns early with
    nothing to do (`run_worker.rs:1967-1970`). `adopt_runs` is one `SKIP LOCKED` statement.
  - **Store outage.** Every arm logs its error and carries on. Walks fence themselves through the
    heartbeat, and the next successful poll adopts.
  - **Shutdown.** SIGINT or SIGTERM on unix, and `ctrl_c`/`ctrl_close`/`ctrl_shutdown` on Windows
    (`tokio::signal`, whose feature is already on `htui`, `htui/Cargo.toml:39`; the cfg split
    mirrors `editor.rs:339-356`). Then `runtime.shutdown(CANCEL_GRACE)`, which cancels walks and
    gives their leases back (`run_worker.rs:1657-1683`), then exit 0. An interrupted step is reset
    and retried by the next worker's sweep (MOD-24).
  - **Logs.** `--log PATH` writes the file exactly as the TUI does (`lib.rs:170-192`). Without it,
    the worker, unlike the TUI, logs to stderr (`fmt` layer, ANSI only on a terminal), so a user
    unit's journal has it. The DSN is never a field or message; a test greps for it (T12).
  - **No TTY is needed.** The worker never calls `terminal::init` (`lib.rs:126`).
- **D15: the worker's `RunRuntime`.** Built with `WorkerHost = PgStore`, `ReplySink = Unaddressed`,
  role `Worker` and production parts (`GixIsolator`, `ShellVerifier`, `DriverFactory`). It is
  `RunRuntime::production`'s parts with the new host. The engine-driven ACP permission gap is
  inherited unchanged (MOD-42, PRD Risks), and the worker's `--help` says so.
- **D16: Runs poll** (OQ-3). `Tab` (`htui/src/ui/tabs/registry.rs:35-64`) gains
  `fn on_refresh(&mut self, _ctx: &mut Ctx<'_>) {}`. It has a default body, as `focus_section`
  and `on_external_edit` do; the no-default rule is for store traits. `App::on_tick` calls it on
  the active tab each refresh (`update.rs:98-108`). `BacklogTab` counts refreshes and, every 5th,
  re-requests `Runs` and `RunActions` for the selected item when its detail shows an active run.
  The cursor survives the re-read (MOD-4 D198).
- **D17: `CONCEPTS.md`** (R-ID-2, R-STO-1 amended 2026-09-29).
  - Lines 15-16 keep "not a cloud service" and add: "A headless `htui worker` per box, started by
    you and talking only to your Postgres, is part of `htui` [R-ID-2]."
  - Lines 20-21 end: "Credentials live in the OS keyring; a headless worker without one reads its
    DSN from a systemd credential or its stdin, never argv, the environment or a plain file
    [R-STO-1]."
  - Owner line and "Reviewed:" date are bumped.
- **D18: `docs/htui-worker.md`** (new) covers:
  - what the worker does and does not do (no permission answers until MOD-42, no remote targeting
    until MOD-43);
  - the executor setting and I-1;
  - the three DSN sources;
  - `--pool-size`;
  - exit codes;
  - a **sample** systemd user unit, not installed: `ExecStart=%h/.cargo/bin/htui worker --log
    %h/.local/state/htui/worker.log`, `LoadCredentialEncrypted=htui-dsn:…`, `Restart=on-failure`,
    `WantedBy=default.target`, plus the `systemd-creds encrypt --user` step, with a note to check
    that the host's systemd supports user-scoped encrypted credentials (R-9).

  `README.md`'s option table (`:73-87`) gains a `htui worker` row that points there.

### M4 — background index sync

- **D19: concepts-index sync as a worker job** (PRD D6, `R-STO-8`).
  - **Startup.** At worker start, `secret::get_qdrant_url()` (`secret.rs:145`) is read once. `None`
    or `Err` means no job, with an `info` saying why. On a keyring-less host that is always the case
    (see "disagree").
  - **The job.** One task. It builds `FastEmbedder::new()` on a blocking thread and
    `QdrantStore::connect` (`concepts.rs:80-83`), then loops: `concepts::sync_all(pg, qdrant)`,
    which is `index_items`' loop over `scopes(pg, None)` (`concepts.rs:98-145`) factored out, then
    sleeps `concepts_sync_minutes`.
  - **The setting.** `concepts_sync_minutes` is an `app_setting` key, a positive JSON integer; it is
    absent, non-positive or not an integer → 15. It is re-read each cycle, following
    `recover.rs:80-82`'s rule. An unknown key is ignored by every other reader.
  - **Failures.** Any failure is a `warn` and the next cycle. The job never touches `RunRuntime`,
    and on shutdown it is aborted.
  - **Pool.** It shares the worker's pool and runs one query at a time (R-7).

## Patterns to Mirror

- Step fence SQL and miss: `set_step_usage` / `finish_step` (`pg/write.rs:1045-1073`,
  `:4217-4252`), `fenced_or_missing` (`:187-196`), MemStore `fence_holds` (`mem.rs:1495-1505`).
- Paused-time tests: `recover.rs:569-800` (`#[tokio::test(start_paused = true)]`).
- A second process in the orch suite: `FakeOrchestrator::restarted` (`htui-orch/src/fake.rs`,
  used at `conformance.rs:4006`, `:4036`, `:4108`).
- Box CAS editor: Settings > Boxes `Mode::Tags`/`Mode::Quirks` and `submit` (`htui/src/ui/tabs/
  settings/boxes.rs:110-137`, `:368-415`, `:441-480`); the closest enum picker is the Runs pane's
  ←/→ resolution cycle (`runs.rs:714-756`).
- Tickers: `sweep_ticker` (`store_worker.rs:2030-2034`), `box_beat` (`:1584-1587`, `:1983-1993`).
- Headless connect error mapping: `concepts.rs:68-79`.
- Signals: `editor.rs:312-361`.
- Postgres integration harness: `htui/tests/runs_pg.rs` (`Stack`, `:243-292`; `run_runtime`,
  `:230-239`; skip via `testkit::demo_db`, `htui-store/src/testkit.rs:68-77`, `:169`).

## Tasks

Milestones run in order and each ends green and committed. The groups are:
- **M1:** T1 → T2.
- **M2:** T3 → T4 → T5 → T6.
- **M3:** T7 ∥ T10 ∥ T11, then T8 ∥ T9, then T12 ∥ T13.
- **M4:** T14.

A task is marked parallel only when its file set is disjoint from every task it runs beside
(intersected mechanically in the fact-check). Worktree fan-out follows team memory
`workflow-worktree-implementers`: named branch first, Gortex edits hit the primary checkout, about
10 GB of `target/` per worktree, and worktrees are removed before `branch -d`.
**Mechanically sweep-able:** the call sites of T1 and T4, and T7's `BoxEdit` literals.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `htui-core/src/store/{traits.rs, mem.rs, conformance.rs}`, `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `htui-store/.sqlx/`, `htui-store/tests/{pg_criteria.rs, cache.rs}`, `htui-agent/src/{conformance.rs, record.rs}`, `htui-agent/tests/recorder.rs`, `htui-orch/src/{engine.rs, conformance.rs, gate.rs}`, `htui-orch/tests/gix_isolator.rs` | first |
| T2 | `htui-orch/src/{recover.rs, engine.rs}` | after T1 (`engine.rs`) |
| T3 | `htui-core/src/store/{worker.rs (new), mod.rs}`, `htui-store/src/{worker.rs (new), lib.rs}`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs` | after T2 |
| T4 | `htui-agent/src/record.rs`, `htui-orch/src/{engine.rs, gate.rs, graph.rs, conformance.rs, fake.rs, lib.rs}`, `htui-orch/tests/gix_isolator.rs`, `htui-orch/Cargo.toml` (comment), `htui/src/agent_worker.rs` (only an `E0034` fix, if one appears) | after T3 |
| T5 | `htui/src/{run_worker.rs, store_worker.rs, testkit.rs, agent_worker.rs}` | after T4 |
| T6 | `Cargo.toml`, `Cargo.lock`, `htui-worker/{Cargo.toml, src/lib.rs, src/runtime.rs, src/views.rs, src/address.rs, src/graphs.rs, tests/deps.rs}` (all new), `htui/Cargo.toml`, `htui/src/{run_worker.rs, store_worker.rs, testkit.rs, agent_worker.rs}` | after T5 |
| T7 | `htui-core/src/model/box_.rs`, `htui-core/src/store/{traits.rs, mem.rs, conformance.rs, worker.rs}`, `htui-store/src/{pg/write.rs, pg/read.rs, backend.rs, worker.rs}`, `htui-store/.sqlx/`, `htui-store/tests/{pg_criteria.rs, box_identity.rs}`, `htui-orch/src/{conformance.rs, engine.rs}` (`BoxEdit` literals), `htui/src/ui/tabs/settings/boxes.rs` (literals), `htui/tests/{box_probe_pg.rs, box_settings.rs, runs_pg.rs}` (literals) | after T6; ∥ T10, T11 |
| T8 | `htui/src/ui/tabs/settings/boxes.rs`, `htui/src/box_settings.rs`, `htui/tests/box_settings.rs`, `htui/tests/snapshots/box_settings__*.snap` (six re-accepted, one new) | after T7; ∥ T9 |
| T9 | `htui-orch/src/{engine.rs, conformance.rs, fake.rs}`, `htui-orch/tests/gix_isolator.rs`, `htui-worker/src/{runtime.rs, lib.rs}`, `htui/src/run_worker.rs` | after T7; ∥ T8 |
| T10 | `htui/src/app/update.rs`, `htui/src/ui/tabs/registry.rs`, `htui/src/ui/tabs/backlog/{mod.rs, detail/runs.rs}`, `htui/tests/backlog.rs` | after T6; ∥ T7, T11 |
| T11 | `htui-store/src/{pg/mod.rs, secret.rs}`, `htui-store/tests/migrations.rs`, `htui/src/concepts.rs` | after T6; ∥ T7, T10 |
| T12 | `htui-worker/src/{worker.rs (new), lib.rs}`, `htui/src/{cli.rs, lib.rs, worker_cmd.rs (new)}`, `htui/tests/worker_pg.rs` (new) | after T9, T11; ∥ T13 |
| T13 | `CONCEPTS.md`, `docs/htui-worker.md` (new), `README.md` | after T11; ∥ T12 |
| T14 | `htui/src/{concepts.rs, worker_cmd.rs}`, `htui-store/tests/qdrant_live.rs` | after T12, T13 |

Every implementer prompt carries these rules:
- PRD D1-D7, this plan's D1-D19 and I-1, and the maintainer's OQ answers win over prose.
- Read the tree with Gortex first, then `sed`/`grep`. `htui-orch` never names `htui-store`.
- No test is skipped or loosened. A moved pin names its reason in the assertion message.
- Every new `pub` item has a doc comment and `Debug`. No default body on a store trait.
- Red tests are committed first, then green. Commit incrementally (team memory).
- Gate: `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test -p <crate>
  --all-features -- --test-threads=2`. Re-run a failure alone at `--test-threads=1` before
  believing it (team memory: dev Postgres recovery and the process-wide keyring fake).

### T1: fence the three step writes (D1, D2)
- **Tests first.** Store conformance, both stores:
  - `a_stale_owner_writes_no_prompt_tree_or_commits`: claim as A, adopt as B, then A's three writes
    are `Fenced` and the prompt digest, trim record, trees, `isolation_path` and commits are
    unchanged;
  - `the_new_owner_writes_prompt_tree_and_commits`;
  - `an_unleased_prompt_write_is_refused_on_a_leased_run`;
  - `a_missing_step_is_not_found_before_the_fence` (all three).

  Orch conformance: `a_walk_woken_after_done_records_no_commits`. This is blueprint F-7's shape:
  the walk stalls after the session's `Done`, B adopts, A's capture meets `Fenced`, the walk ends
  `LeaseLost`, and B's rows are intact. Existing `set_step_prompt_writes_both_columns`
  (`mem.rs:6538`) and every `pg_criteria` use keep their assertions with the fence added.
- **Action.** D1, D2. Run `cargo sqlx prepare`.
- **Validate.** The `htui-core`, `htui-store` and `htui-orch` gates, then
  `cargo build --workspace --all-features --all-targets`.

### T2: monotonic self-fence (D3)
- **Tests first.**
  - `recover.rs`: `heartbeat_fences_on_tokio_time_alone`, where every refresh fails and it returns
    `Expired` at exactly `ttl - refresh` of paused time after `written`.
  - `engine.rs`: `a_stepped_wall_clock_moves_no_fence`. Under `start_paused`, the engine's
    `TestClock` is `advance`d +1 h and then −2 h while a walk holds its lease with failing
    refreshes; the walk ends `LeaseLost` at the same paused instant as with no step.
  - `engine.rs`: `a_forward_step_does_not_expire_a_live_lease`. With refreshes succeeding and a
    +1 day step, the walk is still running after three intervals.
  - The ten existing `heartbeat_*` cases (`recover.rs:569-800`) are rewritten onto tokio instants,
    names and outcomes unchanged.
- **Action.** D3.
- **Validate.** The `htui-orch` gate.

### T3: the three traits and their impls (D4, D5)
- **Tests first.**
  - `htui-core`: `worker_store_is_object_of_the_engines_calls`, a compile-only test instantiating a
    generic `fn` bound on each trait for `MemStore`.
  - `htui-store`: `pg_and_writer_are_worker_stores` and `backend_and_pg_are_worker_hosts`, same
    shape.
  - **P-4**, as a `#[test]` that only has to compile: `tokio::spawn` of a generic
    `async fn f<S: WorkerStore>(s: S)` future for `MemStore`, `PgStore` and `Writer`.
- **Action.** D4 and D5, with nothing's bound changed yet, so the whole workspace still compiles
  as today.
- **Validate.** `cargo build --workspace --all-features --all-targets`, plus the `htui-core` and
  `htui-store` gates.

### T4: bounds onto the traits (D4, D5)
- **Tests first.** None new. The whole orch and agent suites are the proof, together with the
  compile-time fact that `WriteStore` no longer appears in `engine.rs`, `gate.rs` or
  `graph::resolve`: `grep -n 'WriteStore' htui-orch/src/{engine.rs,gate.rs}` shows only doc
  comments and `override_graph`.
- **Action.** Switch the bounds:
  - `EngineParts`/`Engine`/`impl Engine` → `S: htui_core::store::WorkerStore`
    (`engine.rs:382`, `:431`, `:457`, `:527`);
  - `gate.rs` functions (`:301`, `:381`, … `:986`);
  - `graph::resolve` (`:298`);
  - `Recorder`/`pump`/`enforce_breach` → `RecorderStore`.

  Fix `E0034`s by UFCS (D5).
- **Validate.** The `htui-agent`, `htui-orch` and `htui` gates.

### T5: generalise the runtime in place (D7)
- **Tests first.**
  - `run_worker.rs`: `run_request_names_match_store_request_names`, over `every_orch_request`
    (`:4504`) plus the stream and actions requests;
  - `tui_replies_map_every_run_reply`, one `StoreReply` per `RunReply` variant.
  - All existing tests are unchanged.
- **Action.** Still inside `htui/src/run_worker.rs`:
  - introduce `WorkerHost` use, `ReplySink`, `RunRequest`, `RunReply`, `RunServed<A>`,
    `HostGraphs<H>` and the generic `ChatEnd::after`;
  - make `RunRuntime<H, P>` generic;
  - add the `TuiReplies` adapter and the `serve` shim.

  `store_worker.rs:1884-1896`, `:1484-1518` and `testkit.rs:160-345` call the shim.
- **Validate.** The `htui` gate. `StoreRequest`/`StoreReply` variant counts are unchanged at 85
  and 47.

### T6: move into `htui-worker` (D6, D8)
- **Tests first.** `htui-worker/tests/deps.rs::the_library_links_no_terminal_crate`, and the
  library's own `Publisher` test over a recording sink.
- **Action.** A mechanical move of T5's generic code to the crate. `run_worker.rs` keeps the
  adapter, the re-exports and its test module. Update the workspace members and `htui`'s
  dependency.
- **Validate.** The `htui-worker` and `htui` gates; `cargo tree -p htui-worker -e normal | grep -E
  'ratatui|crossterm'` is empty.

### T7: executor setting and the queued read (D9, D10, D11)
- **Tests first.**
  - Store conformance: `edit_box_writes_the_executor_and_keeps_every_other_setting` (seed
    `{"max_concurrent_items": 1, "command_limits": {"verify": 2}, "x": true}`, write `worker`, all
    four keys present) and `an_unknown_executor_is_refused_and_writes_nothing`.
  - `box_.rs` unit tests: `an_unknown_executor_keeps_the_admission_limit`, for
    `{"executor": 7, "max_concurrent_items": 1}` and `{"executor": "container", …}` →
    `max_concurrent_items == Some(1)` and `executor` is `Other`; `a_missing_executor_is_tui`.
  - `pg_criteria`: `claim_run_honours_the_limit_beside_an_unknown_executor` and
    `queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order`. The MemStore twin of the
    latter goes in `mem.rs` tests.
- **Action.** D9-D11, and sweep the 21 `BoxEdit` literals.
- **Validate.** The `htui-core`, `htui-store` and `htui` gates; `cargo sqlx prepare --check`.

### T8: Settings > Boxes executor editor (D10)
- **Tests first** (`htui/tests/box_settings.rs`):
  - `the_boxes_section_shows_each_box_s_executor`;
  - `w_flips_the_executor_after_confirmation` (`w`, then `y`, sends `EditBox { executor:
    Some(Worker) }` with the row's `edit_version`);
  - `a_stale_executor_edit_says_so` (the existing stale path).

  New snapshot `box_settings__executor_confirm`. The six existing snapshots are re-accepted for the
  new column and footer hint (`j/k move · t tags · e quirks · w executor · p probe this box · r reload`),
  each diff reviewed.
- **Action.** A new `Mode::Executor` confirmation beside `Tags`/`Quirks` (`boxes.rs:110-137`). The
  row shows `executor: tui|worker|<other>`.
- **Validate.** The `htui` gate and `cargo insta test` with nothing pending.

### T9: executor gate and hand-back (I-1, D12, D13, OQ-1, OQ-4, OQ-5)
- **Tests first.**
  - **Orch conformance, one per command.** In each, engine A (`Tails::HandBack`) answers and
    `restarted()` engine B sweeps, and the run rests where an in-process command would have put it:
    - `an_approval_handed_back_is_walked_by_the_adopter`;
    - `a_rejection_handed_back_is_walked_by_the_adopter`, both loopable and terminal;
    - `a_selection_handed_back_is_walked_by_the_adopter`;
    - `a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter`;
    - `a_resume_handed_back_is_walked_by_the_adopter`;
    - `an_accept_handed_back_is_walked_by_the_adopter`.
  - Also `a_hand_back_writes_no_step_past_the_window`.
  - **Runtime tests** (`run_worker.rs`, over the TUI loop):
    - `on_a_worker_box_start_run_only_queues`;
    - `on_a_worker_box_the_tui_never_sweeps` (a lapsed foreign lease stays unadopted);
    - `on_a_worker_box_a_live_walk_refuses_cancel_naming_the_worker`;
    - `an_unknown_executor_refuses_start_and_walking_commands`;
    - `a_tui_box_claims_a_queued_row_it_did_not_queue` (OQ-5);
    - `flipping_to_worker_keeps_the_tuis_live_walk`.
  - **Library:** `a_worker_role_idles_on_a_tui_box` and `a_worker_role_claims_queued_rows_in_order`.
- **Action.** `Tails` in `engine.rs`, executor reads in the runtime, `Role`, and the claim scan
  over `queued_runs_on_box`.
- **Validate.** The `htui-orch`, `htui-worker` and `htui` gates.

### T10: Runs poll (D16, OQ-3)
- **Tests first** (`htui/tests/backlog.rs`):
  - `the_runs_pane_rereads_an_active_run_every_poll` (a run moved in the store by another handle
    appears after 20 ticks with no frame);
  - `the_runs_pane_does_not_poll_a_finished_item`.
- **Action.** D16.
- **Validate.** The `htui` gate, and no snapshot changes.

### T11: pool size and DSN sources (D14's pool and DSN)
- **Tests first.**
  - `migrations.rs`: `a_headless_pool_honours_its_size` (`pool().options().get_max_connections()
    == 3`) and `a_headless_pool_is_clamped`.
  - `secret.rs` tests:
    - `stdin_wins_and_the_keyring_is_not_read`;
    - `the_keyring_is_read_before_the_credential`;
    - `a_missing_keyring_falls_back_to_the_credential`;
    - `no_source_is_an_error_naming_all_three`;
    - `the_credential_is_read_only_from_the_given_directory`;
    - `a_blank_line_is_no_dsn`.

    These use `testkit::mock_keyring` / `mock_keyring_broken` and a `tempdir` for the credentials
    directory.
- **Action.** `PoolSize`, `open_pool`'s parameter, `connect_headless`'s parameter (7 test call
  sites in `migrations.rs`, and `concepts.rs:68`), `headless_dsn`, and the module doc.
- **Validate.** The `htui-store` gate.

### T12: `htui worker` (D14, D15)
- **Tests first.**
  - `cli.rs`:
    - `worker_parses_its_two_flags`;
    - `flat_flags_still_parse_and_refuse_a_subcommand_beside_them`;
    - `no_worker_argument_reads_the_environment` (every worker `Arg::get_env()` is `None`);
    - `worker_takes_no_dsn_argument` (`htui worker --dsn x` is an error).
  - `htui/tests/worker_pg.rs`, against Postgres, skipped without `HTUI_TEST_DATABASE_URL`:
    1. `a_headless_worker_drives_a_queued_run_to_rest`: an in-process `worker::run` over a fake
       driver factory (`runs_pg.rs:230-239`'s `run_runtime` parts), a box with
       `executor = worker`, and a run queued by a TUI-shaped `Stack` → `done`, lease released.
    2. `a_tui_exit_does_not_interrupt_a_worker_run`: the `Stack` queues, `R` answers
       `Started { rest.run: Queued }`, the harness shuts down, the worker settles the run, and the
       TUI wrote no lease.
    3. `an_answer_handed_back_is_finished_by_the_worker`: a gated graph, the TUI answers `x`, and the
       worker walks the review loop.
    4. `the_worker_refuses_a_pending_schema_and_writes_nothing`: `testkit::bare_db` gives exit 2 with
       `MigrationsPending`, and `_sqlx_migrations` does not exist.
    5. `the_worker_binary_never_exposes_its_dsn` (`cfg(target_os = "linux")`). Spawn
       `env!("CARGO_BIN_EXE_htui") worker --dsn-stdin --log <tmp>` with `env_clear()`, no `TERM`,
       stdin piped with the DSN and a sentinel password. Wait for the box beat (`last_seen_at`
       moves). Assert that `/proc/<pid>/cmdline` and `/proc/<pid>/environ` do not contain the
       sentinel, SIGTERM through `kill -TERM` gives exit 0, and the log file does not contain it.
    6. `the_worker_binary_refuses_a_wrong_password_without_echoing_it`: exit 2 and the sentinel is
       absent from stderr.
- **Action.** D14 and D15: `worker.rs`, the CLI and the glue.
- **Validate.** The `htui-worker` and `htui` gates.

### T13: docs (D17, D18)
- **Action.** D17 and D18. `README.md` row. No test. `bash
  .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` stays green.

### T14: background index sync (D19)
- **Tests first** (`concepts.rs` tests, `MemVectorStore` and `HashEmbedder` from
  `htui-store/test-support`, `vector.rs:780`, `embed.rs:80-82`):
  - `the_index_job_syncs_at_start_and_every_interval` (paused time, a counting `VectorStore`);
  - `a_failing_sync_is_logged_and_retried_next_cycle`;
  - `the_interval_reads_concepts_sync_minutes_with_a_15_minute_default`;
  - `no_qdrant_url_means_no_job`.

  `qdrant_live.rs`: `the_worker_job_indexes_a_new_item` (gated on `HTUI_TEST_QDRANT_URL`).
- **Action.** D19.
- **Validate.** The `htui` gate, plus `qdrant_live` when a Qdrant is available.

## Test plan

- **Store behaviour** is pinned by the conformance suite on MemStore and PgStore: the fence (T1)
  and the executor edit (T7). `pg_criteria` covers the Postgres-only facts: the queued read, the
  admission limit beside an unknown executor, and the pool.
- **The engine** is covered by the orch conformance suite on MemStore: the fence's walk case, and
  hand-back's adopter cases through `FakeOrchestrator::restarted`.
- **The heartbeat** is tested under paused tokio time with a stepped `TestClock`.
- **The supervisor** is covered by the unchanged `run_worker.rs` suite (behaviour preservation), the
  library's own tests, and the dependency-tree test.
- **The worker** is tested in-process against Postgres with fake drivers, and the binary as a
  child process for the argv, environment, stdin, signal and exit-code facts.
- **The TUI** is covered by `box_settings.rs` snapshots and `backlog.rs` for the poll.

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-1 | A hand-back's recovery differs from the tail it replaces | Medium | High | One adopter case per command (T9) compares with the in-process rest; retry includes `admit` (D12); the tails reuse crash recovery that MOD-4 already pins (D97, D131) |
| R-2 | `E0034` churn when a module sees both traits | High | Low | D5's hygiene rule; compile errors, never silent; UFCS fixes |
| R-3 | T6's move breaks a test through a path | Medium | Low | Re-exports keep every `run_worker::X` path; the test module stays put (D8) |
| R-4 | Two isolators on one box (TUI accept window, worker walk) touch one repository | Low | Medium | `claim_run` refuses an overlapping `repo_scope` while any active run holds it (`Claim::Overlaps`, `engine.rs:713`), and the accept acts on its own parked run's trees; `T` cleanup is terminal-only (`command.rs:1216`). This is inference, flagged for the blueprint |
| R-5 | The flip window leaves a run nobody walks | Low | Medium | D13's two directions are tested; OQ-5 claims stranded queued rows; a hand-back into a box with no worker waits for the worker's startup sweep |
| R-6 | `queued_runs_on_box` scans `run` every 5 s with no index | Low | Low | Per-user table; an index would be migration `0008`, raised only if measured |
| R-7 | The index job starves the pool (4) and delays lease refreshes | Low | High | The job is sequential, one connection at a time; `acquire_timeout` is the connect timeout; the heartbeat's retry and fence absorb a delay |
| R-8 | Windows cannot be compiled here (`rustup target list --installed` shows only `x86_64-unknown-linux-gnu`) | Certain | Low | The `cfg(windows)` signal arm mirrors `editor.rs:352-356`; the credential source is `cfg(target_os = "linux")`; MOD-16 verifies Windows (`R-NF-1`) |
| R-9 | A systemd user manager may not support `LoadCredentialEncrypted` on older versions | Medium | Low | The doc says to verify on the host; `--dsn-stdin` and the keyring remain |
| R-10 | Sentry (`main.rs:12-20`) captures a worker error carrying the DSN | Low | High | No message or field built from the DSN; T12 cases 5 and 6 grep stderr and the log |
| R-11 | Engine-driven ACP steps fail on the first permission request under the worker | Certain | Medium | Unchanged from the TUI today (PRD Risks); documented in `--help` and `docs/htui-worker.md`; MOD-42 |
| R-12 | Churn in `.sqlx`, snapshots and pins | Certain | Low | `cargo sqlx prepare --check` and `cargo insta` at every gate |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres
  cargo test --workspace --all-features --no-fail-fast -- --test-threads=2`. A failure is re-run
  alone at `--test-threads=1`. The pre-existing failure on main is
  `every_provider_failure_leaves_a_valid_prompt`.
- `cargo sqlx prepare --check` from `crates/htui-store`, against a scratch database migrated to
  `0007` (team memory `sqlx-prepare-needs-migrated-scratch-db`: the compose `htui` database is
  empty).
- `cargo tree -p htui-worker -e normal | grep -E 'ratatui|crossterm'` prints nothing.
- `cargo doc --workspace --no-deps --keep-going` shows only the six baseline errors
  (`HANDOFF.md:49-54`).
- `cargo insta test --workspace` with nothing pending.
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`

**Pins to move** (HANDOFF "Live coordinates", `HANDOFF.md:36-48`):

| Pin | Now | After |
|---|---|---|
| Store conformance `CASES` | 96 | 102 (T1 +4, T7 +2) |
| `READ_CASES` | 14 | 14 |
| `htui-orch` `CASES` | 73 | 80 (T1 +1, T9 +6) |
| `.sqlx` files | 288 | 290 (`step_fence`, `queued_runs_on_box`; the `set_step_prompt` and `edit_box` texts replaced in place) |
| `crates/htui/tests/snapshots` | 107 | 108 (six `box_settings__*` re-accepted) |
| Workspace members | 5 | 6 |
| Everything else | — | unchanged: `StoreRequest` 85, `StoreReply` 47, `GraphSource` 7, `hierarchy::REQUEST_NAMES` 13, `MIRRORED_TABLES` 21, seven Settings sections, 34 commented columns; migrations `0001`-`0007`, next `0008` |

The implementer re-counts each pin at its gate; the numbers above are this plan's prediction.

## Acceptance

- After adoption, a stale holder's `set_step_prompt`, `upsert_step_tree` and `record_commits`
  write nothing and answer `Fenced`, on both stores.
- A wall-clock step during a lease neither extends nor cuts the heartbeat's fence.
- `htui-worker` hosts run supervision, links no terminal crate, and reaches the store only through
  `WorkerHost`/`WorkerStore`/`RecorderStore`. `WriteStore` is in none of the engine's, `gate`'s,
  `resolve`'s or the runtime's bounds. With no worker running, the TUI's suites pass unchanged.
- On a box whose executor is `worker`:
  - the TUI queues and never claims, adopts or sweeps;
  - gated answers hand back and the worker walks on;
  - a TUI exit interrupts nothing the worker owns.
- `htui worker` with no TTY and no `TERM` connects headless, claims queued runs targeted at its box,
  heartbeats, and shuts down cleanly on a signal. It refuses bad schema non-zero, writing nothing.
  Its pool honours `--pool-size`, and its DSN never appears in argv, the environment, the log or a
  plain file.
- With a Qdrant URL in the keyring, the worker re-syncs the concepts index at start and on the
  interval. Without one, or when a sync fails, runs are unaffected.

## Where the PRD, ANA or tree disagree

- **PRD Risks row 2** says "the fixture moves with a `test-support` feature". The fixture drives the
  runtime through the TUI's store loop and names `TabId` (`run_worker.rs:2454`, `:2787-2794`), so it
  stays in `crates/htui` and no feature is needed (D8).
- **The steering's blanket impl** (`impl<T: WriteStore> WorkerStore for T`) cannot prove its futures
  `Send`, which is P-1. The impls are explicit (D5).
- **PRD Evidence: "~15 `Backend` inherent reads"** is, precisely, 9 called directly, 6 more through
  `GraphSource`, and 5 `ReadStore` reads on `Backend`: 20 host reads plus `writer`, and 22 after
  D11's new read (D4).
- **ANA-16 §8 item 2's surface** "(claim, lease, adopt, step status, events, usage, finish)" is
  narrower than the engine's real call set: 45 methods over three traits, plus the host's 22. The
  plan keeps exactly the call set and names each call site (hazard 15), rather than shrinking the
  engine.
- **MOD-40 plan D6** says `edit_box` "is the only `box.settings` writer". It writes
  `declared_tags`, `quirks` and `edit_version` only (`pg/write.rs:1490-1494`), and the trait doc says
  `box.settings` "has **no** writer at all today" (`traits.rs:454-458`). `box_.rs:237-239` says
  `set_setting` writes it, which it cannot. D10 makes `edit_box` the first writer, and PRD D4's
  "edited in Settings > Boxes" becomes T8's new editor.
- **The PRD's scope** has no DB scan of queued runs. None exists (`claim_queued` works an in-memory
  set, `run_worker.rs:795`, `:825`, `:1887`), so D11 adds one read, and OQ-5 extends it to the TUI.
- **PRD D3's order** "keyring first … plus `--dsn-stdin`" is read as: `--dsn-stdin` is explicit and
  exclusive, and otherwise the keyring comes before the credential (D14).
- **PRD D6** gates the sync on "a Qdrant URL in the keyring". On a keyring-less host, which is D3's
  case, there is none, so that worker never indexes. A credential-sourced Qdrant URL is not in scope.
- **ANA-16 hazard 10** calls disabling in-process execution "for clarity rather than safety". With
  cross-process adoption and free-lease command takes on parked runs, the plan makes it an invariant
  (I-1) for single ownership, while correctness stays with the lease CAS and the fences.
- **Stale citations:**
  - `HANDOFF.md:662` cites the keyring features at `Cargo.toml:45-46`; they are at `:46-47`.
  - The PRD cites `secret.rs:1-7` for the no-env rule; the module doc is `:1-14`, with the rule at
    `:4-7`.
  - MOD-4's doc names `run_worker::BackendGraphs` as the `GraphSource` implementor
    (`htui-orch/src/graph.rs:45` is stale per MOD-4 C-3); after T6 it is `htui_worker::HostGraphs`,
    and T4 fixes that comment while it edits `graph.rs`.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| The engine calls `set_step_prompt` 3 times, `upsert_step_tree` 2 times, `record_commits` 7 times, all under this process's lease | true | `engine.rs:3181`, `:3765`, `:4590`; `:3129`, `:3745`; `:1402`, `:3140`, `:3211`, `:3691`, `:3756`, `:3812`, `:4747`. Walk sites are under `walk_leased` (`:1681`); `:1402` is under `take_lease` `:1373` and `heartbeaten` `:1455`; `:4747` is reached from walks, from command tails after `take_lease`, and from `recover_run` under `adopt_runs` + `renew_lease` (`:1984-2024`) |
| No production caller passes an unleased fence to these three | true | Non-engine hits are tests: `record.rs:2123` (tests from `:1876`), `gate.rs:1540`, `cache.rs:517`, `gix_isolator.rs:2413`, orch `conformance.rs:4277`, `:4485`, `:6465`, `:6806`; `writer.rs` delegates |
| The two batch writes check existence in their own transaction | true | `step_exists(&mut tx, step)` at `pg/write.rs:4562`, `:4641` |
| MOD-40's fence predicate and miss helper exist | true | `pg/write.rs:1052-1068`, `:4224-4240`, `:187-196`; `mem.rs:1495-1505` |
| Seven `ReadStore`/`WriteStore` implementors | true | `mem.rs:5544`/`:5666`; `writer.rs:120`/`:307`; `pg/read.rs:63`, `pg/write.rs:684`; `backend.rs:611` (read only); `cache/read.rs:268` (read only); `htui-agent/src/conformance.rs:608`/`:711`; `htui-agent/tests/recorder.rs:324`/`:428` |
| The heartbeat's fence reads the wall clock | true | `recover.rs:137-157` (`clock.now()`); `SystemClock` is `Utc::now()` (`htui-core/src/clock.rs:37-44`) |
| `TestClock` can step backwards | true | `advance(TimeDelta)` accepts a negative delta (`clock.rs:104-110`) |
| The engine's store surface is 48 methods, 7 only `override_graph`'s, plus 3 through `Recorder` | true | Enumeration of `self.parts.store.*`, `gate.rs` and `graph.rs` calls (D4 table); `override_graph` callers only `graph.rs:1596`, `:1771`, `:1837` (tests); `Recorder` calls `record.rs:1077`, `:1090`, `:1169`, `:1235` |
| `ReadStore` has 23 methods and `WriteStore` 86 | true | `grep -c '^\s*async fn'` over `traits.rs:78-254` and `:263-1478` |
| The supervisor's store calls are 9 inherent `Backend` reads, 5 `ReadStore` reads, 6 inherent reads through `GraphSource`, plus `writer()` | true | `run_worker.rs:294-312`, `:898`, `:1119`, `:1136`, `:1142`, `:1175`, `:1223-1236`, `:1958`, `:1968`, `:2366-2410`; definitions `backend.rs:148`, `:170`, `:239`, `:252`, `:299`, `:360`, `:378`, `:393`, `:451`, `:465`, `:484`, `:498`, `:512`, `:526`, `:555`, `:569` |
| Every type the host trait returns lives in `htui-core` | true | `BoxInfo` `model/box_.rs:221`, `BoxRow` `:27`, `RepoBoxPath` `model/hierarchy.rs:184`, `WorkspaceSummary` `:224` |
| Real store futures are `Send` at a concrete instantiation today | true | `run_worker.rs` spawns engine futures over `Writer` with `tokio::spawn` (`spawn_task` `:1804`, `spawn_supervised` `:1817`) |
| **P-1**: a blanket `impl<T: WriteStore> WorkerStore for T` cannot be spawned generically | true | Scratch probe (two crates under `/tmp`, deleted after the run; T3's P-4 repeats it on the real types), rustc 1.98.1. Without `Send` in the trait, `tokio::spawn` of a generic engine future fails with E0277 ("`async fn` in trait … does not automatically imply that its future is `Send`"). With `-> impl Future + Send` declared, the blanket impl fails with "future cannot be sent between threads safely" (×3) |
| **P-2**: explicit impls written as `async fn` satisfy `-> impl Future + Send` declarations; a generic supervisor spawns them; a downstream crate implements the traits for a local type | true | Same probe: `core_a` (traits, `Mem`, `Engine`, `Recorder`, generic `Runtime<H>`), `down_b` (`Client`, a `Backend` host, `tokio::spawn`): `cargo check --tests` clean |
| **P-3**: a concrete receiver with both traits in scope is ambiguous | true | Same probe with `use core_a::{WriteStore, WorkerStore}` in a test: `error[E0034]: multiple applicable items in scope` |
| **P-4** (the real types) | to run | T3's compile test |
| `htui-orch` never depends on `htui-store` | true | `htui-orch/Cargo.toml:18-20` and its `[dependencies]` |
| `run_worker.rs` production code imports no UI module; its coupling is the envelope types and `ChatTask` | true | `run_worker.rs:22-60`; `:591`. `Origin` wraps `TabId`/`OverlayId` (`store_worker.rs:78`, `:51-52`), whose modules import ratatui/crossterm (`ui/tabs/registry.rs:8-12`, `:19`; `ui/overlay/registry.rs:7-8`, `:12`) |
| `run_worker.rs`'s tests use the TUI loop and `TabId` | true | `:2450-2454`, `Worker::spawn` `:2787-2794` |
| `store_worker.rs`'s promotion test reuses the run_worker fixture | true | `store_worker.rs:3431-3433` |
| Nothing scans the database for queued runs; `claim_queued` works an in-memory set | true | `Shared.queued` `run_worker.rs:795`, `queue` `:825` (fed only by refused claims `:1929`, `:1945`, `:2163`), `claim_queued` `:1887-1904`; the only `status = 'queued'` SQL is in `create_run`/`claim_run` (`pg/write.rs:3538`, `:3677`, `:3689`, `:3773`, `:3788`) |
| A TUI sweep adopts any `running` run on its box whose lease lapsed, whichever process owned it | true | `adopt_runs` `pg/write.rs:3867-3873` (`executing_box_id`, NULL or lapsed lease, owner distinct from the sweeper); `sweep_once` counts all of the box's runs (`run_worker.rs:1967-1970`, `pg/read.rs:1974-1986`) |
| A released lease is adoptable at once | true | `release_lease` sets `lease_expires_at = clock_timestamp()` (`pg/write.rs:3989`); `adopt_runs` takes `<= clock_timestamp()` (`:3872`) |
| A parked run holds no lease, so a TUI command's take is not a steal | true | `release_after_walk` releases on any rest but `running` (`engine.rs:1769-1778`); `take_lease` refuses a live foreign lease with `LeaseHeld` (`:1790-1814`) |
| Every walking command records its decision in a leased window before its tail | true | `answer_guarded` `:794-857`, `retry_guarded` `:964-1030`, `retry_group_guarded` `:1036-1064`, `select_fanout` `:1075-1131`, `accept_artifact` `:1357-1473`, `resume` `:2680-2690` |
| `recover_run` finishes an unfinished approval, selection or rejection tail | true | frontier reconcile `engine.rs:2141-2156` (D97); `running` + waiting step re-parks `:2167-2186` (D96); `failed` latest step → `settle_failed` `:2187-2200` (D131) |
| A retried `failed` step without `admit` would read as D131's unfinished failure | true | `retry_guarded` leaves a `failed` step as is (`:992-995`); `recover_run` sends `Cursor::Rest { Failed }` to `settle_failed` (`:2187-2200`) |
| No Runs-pane key is greyed by lease ownership | true | `verdicts` `run_worker.rs:355-488` and `command.rs` guards read no `lease_*`/`executing_box_id`; `Run` carries no `lease_owner` (`model/run.rs:203-206`) |
| The Runs pane has no timer; the app's tick re-reads only the top bar | true | `update.rs:98-108`; `BacklogTab::wants_requests` asks for `Items` only (`backlog/mod.rs:194-199`); the `Tab` trait has no tick hook (`registry.rs:35-64`) |
| `box.settings` has no writer; `BoxSettings` decodes with `.ok()` and denies no unknown field | true | `traits.rs:454-458`; `pg/write.rs:1490-1494`; `box_.rs:240-252`; readers `pg/write.rs:3699-3701`, `mem.rs:3788-3800` |
| Every `BoxEdit` literal is exhaustive: 21 in 10 files | true | `grep 'BoxEdit {'` lists 21 sites; none uses `..Default::default()` |
| Settings > Boxes edits only tags and quirks | true | `boxes.rs:110-137`, `:386-401`, `:441-480` |
| No clap subcommand exists; flat flags dispatch by early return | true | `cli.rs:6-60`; `lib.rs:75-97` |
| `open_pool` hard-codes 8, shared by both connects | true | `pg/mod.rs:727-736`, callers `:217`, `:255` |
| `connect_headless`' callers | true | `concepts.rs:68`; `htui-store/tests/migrations.rs:985`, `:1004`, `:1040`, `:1184`, `:1201`, `:1211`, `:1222` |
| Tokio's `signal` feature is on `htui` only | true | root `Cargo.toml:33-34`; `htui/Cargo.toml:39`; use `editor.rs:312-361` |
| `persist_registration` is public | true | `connect.rs:589` |
| `shutdown` gives the leases of the walks it cancels back | true | doc and body `run_worker.rs:1657-1683` |
| No test spawns the `htui` binary today | true | no `CARGO_BIN_EXE`/`assert_cmd` hit in the workspace |
| The Qdrant URL is a keyring entry; `Indexer::sync` has one caller | true | `secret.rs:27`, `:145`; `concepts.rs:132` |
| No migration is needed | true | D1-D19 add no DDL; `executor` is a JSON key (D9, D10), the queued read uses existing columns (D11), the interval is an `app_setting` key (D19) |
| T1-T14's parallel marks are disjoint | to fact-check | Tasks table; T7 ∥ T10 ∥ T11, T8 ∥ T9, T12 ∥ T13 |
