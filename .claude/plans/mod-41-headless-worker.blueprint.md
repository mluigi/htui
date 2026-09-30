# Blueprint: MOD-41 — Headless worker (`htui worker`), T1–T14

**Status**: proposed (2026-09-30, code-architect). Implements
`.claude/plans/mod-41-headless-worker.plan.md` (CONFIRMED 2026-09-30, OQ-1 to OQ-6 as recommended)
under `.claude/prds/mod-41-headless-worker.prd.md` (PRD D1-D7). Its D1-D19, I-1, Tasks table and
Fact-check record are binding. Where this blueprint had to choose, the choice is a **B-n**, driven
by a finding **F-n**. Anything that would move a confirmed decision is an **E-n** (§0b), unresolved
here.

**Verified at**: `85d397f` (`hr/MOD-41`). `git diff --stat 955de97 85d397f` touches only the plan
and the PRD, so every line number in the plan still holds, and every number below is at HEAD.
Paths are relative to `crates/` unless they start with `docs/`, `.claude/` or name a root file.
Gortex answered symbol reads on this checkout although its banner said "untracked"; line ranges
were read with `sed`. **Worktree lanes are not indexed: they read and edit with native tools.**

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn, clippy `-D warnings`; every new
`pub` item documented and `Debug`; no default body on a store trait (`RecorderStore`,
`WorkerStore`, `WorkerHost`); `max_width = 100`; every commit compiles; red first, then green,
committed incrementally (team memory). `every_cross_referenced_test_name_exists`
(`htui-core/src/store/conformance.rs`): a backticked snake_case name with ≥ 4 underscores in that
file's docs must be a fn there or in `mem.rs`; a test elsewhere is spelled `pg_criteria.rs::name`.
**D5 hygiene, restated because it bites everywhere**: no module `use`s `WorkerStore`,
`RecorderStore` or `WorkerHost`; bounds name them by path; a type parameter is never bounded on
both `WriteStore` and a new trait unless every shared-name call on it is UFCS; every forwarding
body is UFCS (`WriteStore::claim_run(self, …).await`, `PgStore::box_row(self, id).await`).

**Layout**: §0 findings · §0a decisions · §0b escalations · §1 build order · §2 shared shapes ·
§3-§16 T1…T14 (shapes, commits, tests, gate) · §17 wave schedule and lane rules · §18 pins ·
§19 gate reference.

---

## 0. Findings

| # | Severity | Plan says | Tree / evidence | Fix |
|---|---|---|---|---|
| **F-1** | **Blocker** (T1) | D1 MemStore: "the fence goes inside `check_step_batch`, right after its `require_step`". | `check_step_batch` (`htui-core/src/store/mem.rs:4510-4536`) is also `close_out`'s per-commit check (`mem.rs:4849-4855`), which has no fence and runs with no lease (`C` on a finished item). A fence inside it breaks close-out on MemStore. | **B-1**: `check_step_batch(&self, table, step, fence: Option<StepFence>, rows)`; order `require_step` → `fence_holds` when `Some` → batch. `upsert_step_tree`/`record_commits` pass `Some(fence)`, `close_out` passes `None`. Postgres `close_out` keeps `step_exists`. |
| **F-2** | Major (process, T1, T2) | "Tests first" for T1 and T2. | A test written against `set_step_prompt(fence, …)` or `heartbeat(refresh, Instant, times)` does not compile against today's tree, and "every commit compiles". | **B-2**: T1 commit 1 = new signatures + implementors that **accept and ignore** the fence + every call site + the new tests (red at run time); commit 2 = enforcement. T2's red commit carries only `a_stepped_wall_clock_moves_no_fence` (it compiles today). |
| **F-3** | Minor (T2) | `a_forward_step_does_not_expire_a_live_lease` is a red test. | It is green before T2 on MemStore: `tokio::time::timeout(ZERO, f)` polls `f` before the deadline, and `MemStore::refresh_lease` is ready at first poll (`mem.rs:6051-6057`), so today's wall-clock fence (`orch recover.rs:141-150`) never trips on a forward step there. | Committed in the green commit as a **regression pin**; the commit message says so. The red evidence for D3 is the stepped-clock case (+1 h fences early, −2 h fences late today). |
| **F-4** | Minor (T2) | D3: `fence = written - refresh`. | `std`/tokio `Instant - Duration` panics on underflow; `LeaseTimes` fields are `pub` (`recover.rs:47`, `:49`). | `until.checked_sub(times.refresh).unwrap_or_else(Instant::now)`: fail-safe, the first check reads `Expired`. |
| **F-5** | Minor (T3) | D4: "plus 21 reads, 22 in M3". | 9 + 5 + 6 = **20** reads; the plan's own "disagree" section (`plan:1019-1021`) says 20 + `writer`, 22 after D11. | T3 declares **21** methods (`writer` + 20 reads); T7 adds the 22nd. |
| **F-6** | Minor (T3) | P-4 spawns "a generic `RunRuntime<H, P>` future". | That type exists only from T5. | T3's P-4 covers `MemStore`, `PgStore`, `Writer`. T5 adds `a_generic_runtime_future_spawns` (compile-only). |
| **F-7** | Major (T5) | D7: a free `serve(runs, …)` shim, and `runs.sweep(&backend, &tx)` kept. | Call sites: `.serve(` ×12 in `htui/src/run_worker.rs`, ×2 `store_worker.rs`, ×2 `testkit.rs`; `.sweep(` ×4 + ×4. An **inherent** library method called `serve`/`sweep` with a host/sink signature shadows any TUI method of that name (inherent methods resolve first; the call then fails on its arguments). | **B-3**: library methods are `serve_request` and `sweep_with`. The TUI adapter exports `trait TuiRuns { serve, sweep }` with **today's** signatures, implemented for the alias. Call sites only add `use crate::run_worker::TuiRuns as _;`. |
| **F-8** | Minor (T5) | `RunServed = htui_worker::RunServed<ReplyAddr>`. | Its `Reply` carries `RunReply`; `store_worker.rs:1889` and `testkit.rs:324` bind it as a `StoreReply`; `run_worker.rs:4341` matches `RunServed::Reply(StoreReply::RunStream(..))`. | `.into()` at the two production sites (`impl From<RunReply> for StoreReply`, one-to-one); the test pattern becomes `RunServed::Reply(RunReply::Frame(..))`, same meaning. |
| **F-9** | Major (T5) | `RunServed<A> { …, Attach { addr: A, promoted, ended } }` with one parameter. | `ChatEnd` (`run_worker.rs:580-583`) and `ProgressSink` (`:628-632`) hold the `Publisher`, which becomes `Publisher<P>`; `ended: ChatEnd` and the engine's `K = ProgressSink` would drag `P` into both. | **B-4**: private object-safe `trait Publish: Send + Sync + Debug { fn publish(&self, frame: &RunFrame); }`, `impl<P: ReplySink> Publish for Publisher<P>`. `ChatEnd { publisher: Arc<dyn Publish>, tag }` stays non-generic; `ProgressSink<S> { publisher: Arc<dyn Publish>, writer: S, author }`. `Publisher<P>` gets a hand-written `Debug` (no `P: Debug` needed, the plan's `ReplySink` has none). |
| **F-10** | Minor (T5) | D7's `ReplySink` has `subscriber` and `send` only. | `Publisher::wire` keeps the first live sender and replaces only a closed one (`run_worker.rs:707-716`), which needs `is_closed`. Every caller wires one channel per runtime (the loop's `tx`; each test makes one `unbounded_channel` per runtime; checked). | `wire` replaces unconditionally ("last wire wins"), documented. No trait change. |
| **F-11** | Minor (T5) | D7: "removes `agent_worker::ChatTask`". | `ChatTask` is `Served::Start`'s field type (`agent_worker.rs:341`, `:355`) and `testkit.rs:24`'s. `testkit.rs:188` pushes `ended.after(task)` into `Vec<(StepId, ChatTask)>`. | `ChatTask` stays in `agent_worker`; only `run_worker` stops naming it. `testkit.rs` wraps `Box::pin(ended.after(task))`. |
| **F-12** | Minor (T6) | D8: `htui_worker::testing` with a `TaskCtx` constructor. | A type reachable from a `pub` module trips `missing_debug_implementations`; `TaskCtx` derives only `Clone` (`run_worker.rs:1687`). The T5 `TaskCtx` literals (`:3777`, `:3820`, `:3869`) must change **again** in T6 (cross-crate). | Hand-written `Debug` for `TaskCtx` and `Probe`; §8.3 lists the whole surface and every call-site rewrite. |
| **F-13** | Major (T7) | D10: a non-object blob is refused "before writing … through the existing pre-query refusal order". | Postgres reads the row before the `UPDATE` only for a bad tag list (`htui-store/src/pg/write.rs:1467-1484`); otherwise one `UPDATE`, whose `jsonb_set` on a scalar raises `22023` → `Backend`, not `Constraint`. | **B-5** (§9.3): the pre-query read path fires for a bad tag list **or** `Some(Executor::Other(_))`; the `UPDATE`'s `WHERE` gains `AND ($6::text IS NULL OR jsonb_typeof(settings) = 'object')`; a miss re-reads: none → `NotFound`, spent token → `Stale`, executor asked on a non-object blob → `Constraint`, else `cas_miss`. Same precedence as MemStore, race-safe. |
| **F-14** | Minor (T7, T9) | Tasks table file lists. | `Executor` needs `htui-core/src/model/mod.rs` (re-export list `:105`); `Tails` needs `htui-orch/src/lib.rs` (`pub use engine::{…}`, `:49-52`). | Both files are added to T7 and T9 respectively. |
| **F-15** | Minor (T6/T7) | — | `WorkerHost` has no default bodies; T7 adds `queued_runs_on_box`. | Exactly two implementors (`PgStore`, `Backend`) until MOD-47. T6's library tests use `Backend::memory(…)`, never a test host. |
| **F-16** | **Blocker** (T9, OQ-4) | The queued cancel "first wins `transition_run(Queued → Cancelled)`". | `transition_run` is run-only (`htui-core/src/store/traits.rs:1101-1117`); the item mirror `queued → open` is `finish_run`'s (`:1291-1305`). Without it the item stays `queued` with no active run, and `create_run` admits only `open | failed` (`htui-orch/src/engine.rs:584-587`): it could never run again. | **B-6** (§11.2): after the CAS wins, `transition(item, Status::Queued, Status::Open)` (legal, `htui-core/src/model/item.rs:55`; `Ok(false)` tolerated), then `cleanup_run`. The two-statement residual is **E-2**. |
| **F-17** | Major (T9, I-1) | D12: the executor is read per command and per sweep. | The in-memory claim queue also claims: `retry_claims` → `claim_queued` → `reclaim` (`run_worker.rs:1853-1953`) is fed by refused claims and fires whenever a task ends. After a `tui → worker` flip, the TUI would claim a run it queued before the flip: an I-1 breach. | **B-7**: `reclaim` checks `kit.role.executes(&kit.executor)` after `Kit::read` and drops the run otherwise (`debug!`); the worker's scan finds it. Test `flipping_to_worker_stops_the_in_memory_claim_retry`. |
| **F-18** | **Blocker** (T9, D14/OQ-5) | The claim scan runs after `runtime.sweep`. | `sweep_once` returns at once when there is no dead walk and `active_runs_on_box == 0` (`run_worker.rs:1967-1970`), and that count excludes `queued` (`htui-store/src/pg/read.rs:1965-1987`, `mem.rs:742-753`). On an idle box a scan placed after it never runs. | **B-8**: `sweep_once` = gate → adopt part (keeps its short-circuit) → claim scan, always (§11.4). |
| **F-19** | Minor (T9, OQ-5) | The TUI's sweep feeds its box's queued rows into its claim queue. | A scan that reads a row between `create_run`'s commit and `start_run` taking the run lock can win the lock; `start_run`'s `claim` then answers `ClaimRefused { NotClaimable }` to the user while the run walks. | **B-9**: `start_run` mints its walk token right after `enqueue` answers (moved up from `:2150`); the scan skips `walks.is_live(run)`. Residual: a wrong sentence, TUI boxes only, once per 120 s sweep; documented in the scan's doc. |
| **F-20** | Major (T9, OQ-6) | "A per-run backoff in the worker's adoption loop … 5 s doubling to 5 min". | `adopt_runs` leases every lapsed run to this owner, and a sweep never re-adopts a lapsed lease its own process owns (plan D88, `pg/write.rs:3867-3873`). Skipping the resume after adoption strands the run until the dead-walk pre-pass. | **B-10** (§11.6): worker role only. A run with a backoff entry is still adopted, but its `resumed` task first sleeps until `due`. The sleep is cancellable by the walk token; on cancel it releases the lease best-effort. `backs_off(err)` covers every `EngineError::Resolve` except `ResolveError::Store`. Each failure doubles the delay (5 s → 5 min) and logs once per step up; a resume that answers clears the entry. |
| **F-21** | Minor (T9) | `a_queued_cancel_loses_cleanly_to_a_concurrent_claim` among the orch cases. | The orch harness has no hook between `cancel_run`'s read (`engine.rs:1604`) and its write. | White-box `engine.rs` test over the factored `cancel_queued(&self, row: &Run)` fed a stale row after a second owner claimed. Not a `CASES` entry, consistent with the plan's +11 (10 adopter cases + `a_hand_back_writes_no_step_past_the_window`). |
| **F-22** | Minor (T9) | "A new field is a compile error at every constructor: `run_worker.rs:1282-1298`, `htui-orch/src/fake.rs`, `tests/gix_isolator.rs` and engine tests". | Nine `EngineParts` literals: `engine.rs:6083` (`fake_parts`), `:6415`, `:6502`, `:9095`, `:11474`, `:12382`; orch `conformance.rs:6614`; `tests/gix_isolator.rs:92`; `Kit::engine` (`run_worker.rs:1284`, in `htui-worker/src/runtime.rs` by T9). `fake.rs` has none. | `FakeOrchestrator` gains a `tails: Mutex<Tails>` read by `fake_parts`; `Orchestrate` gains `set_tails`; `restarted()` (`fake.rs:1291-1325`) starts at `Walk`. |
| **F-23** | Minor (T9) | D12: "Offline, the read goes through the cache, which mirrors `box.settings`". | `Backend::box_row` offline is `orchestration_offline()` (`htui-store/src/backend.rs:512-518`). | Harmless: `Kit::read` refuses with `DATABASE_UNREACHABLE` first (`run_worker.rs:1223-1225`) and a sweep returns on `writer().is_none()` (`:1430`). No code. |
| **F-24** | Note (T9, OQ-6) | "The adopter's tail is the crash path." | The adopter's `recover_run` reconciles the frontier (`engine.rs:2141-2156`) **before** its `resume` reaches the topology gate (`:2748-2771`); an in-process `u` checks topology first (`:2680-2690`). On a changed graph a handed-back approve, select or unblock merges the frontier, then parks with the topology note. | This is today's crash path, accepted by OQ-6. The changed-graph case pins it, and `docs/htui-worker.md` states it. |
| **F-25** | Minor (T11/T12) | D14: "`set_dsn_from_stdin` switches to the same `Zeroizing` line reader" (T11). | `set_dsn_from_stdin` is `htui/src/lib.rs:151-164`, a T12 file. | T11 adds `secret::read_dsn_line`; T12 switches `set_dsn_from_stdin` to it. |
| **F-26** | Minor (T12) | `main.rs` maps `WorkerExit` with `ExitCode::from(2)`. | `htui::run` returns `anyhow::Result<()>` (`htui/src/lib.rs:74`); `main.rs:23-30` maps every error to `FAILURE`. `htui` has no `thiserror`. | `WorkerExit: std::error::Error` (hand-written); `lib.rs`'s arm converts with `anyhow::Error::from`; `main` downcasts for the code (§14.4). |
| **F-27** | Major (T12) | Case 6: "refuses a wrong password". | The sandbox Postgres is trust-auth (`HTUI_TEST_DATABASE_URL` has no password; team memory), so every password is accepted. | The DSN names a database that does not exist, with the sentinel as its password: exit 2 under any auth mode. Renamed `the_worker_binary_refuses_a_dsn_it_cannot_use_without_echoing_it`. |
| **F-28** | Major (T12) | Case 5: "Wait for the box beat (`last_seen_at` moves)". | `BOX_HEARTBEAT` is 60 s (`htui-store/src/connect.rs:38`), and the store worker's first beat is one period out (`store_worker.rs:1584-1586`): a minute per run. | **B-11**: the worker beats once at start (`tokio::time::interval`, first tick immediate) and logs `htui worker ready` after connect. Case 5 polls the log for that line, then `last_seen_at`. `WorkerConfig { poll, box_beat, grace }` lets cases 1-3 shorten both. |
| **F-29** | Minor (T12) | Cases 1-3 use "a TUI-shaped `Stack`". | `Stack`/`run_runtime` are private to `htui/tests/runs_pg.rs` (`:230-292`); integration tests are separate crates. | `worker_pg.rs` carries a minimal copy (no `tests/common`, which would touch `runs_pg.rs`). |
| **F-30** | Major (T12) → **E-1** | R-10: `capture_anyhow` sees only typed `WorkerExit` sentences. | `main.rs:12-20` initialises Sentry with the production Glitchtip DSN before parsing. Case 6 (exit 2) sends one event per test run; case 5 exits 0 (breadcrumbs ride only with an event). | Escalated. §14.4 marks the one line E-1 decides. |
| **F-31** | Minor (T14) | D19: the Qdrant URL is read once at worker start. | In case 5's `env_clear()` environment (no `DBUS_SESSION_BUS_ADDRESS`) the `sync-secret-service` keyring must fail fast. | T14's gate re-runs T12 case 5. |
| **F-32** | Minor (T4) | `Recorder`/`pump`/`enforce_breach` → `RecorderStore`. | `htui-agent/src/record.rs:106` imports `WriteStore`, and the module docs link it (`:25`, `:52`, `:93`, `:1161`). | If `unused_imports` fires after the bound moves, rewrite those links as `htui_core::store::WriteStore` paths and drop the import. Never import `RecorderStore`. |
| **F-33** | Minor (T6) | Library tests over `Backend::memory`. | `Backend` implements `ReadStore` and `WorkerHost`; a test importing `ReadStore` beside `WorkerHost` is E0034 on `backend.run(..)` (P-3). | Library tests never import `WorkerHost`; they reach it through the generic runtime only. |
| **F-34** | Note (T9) | Retry hand-back: "`admit`, inside `leased_window`". | That window runs without the heartbeat. `admit` is stage-1 reads plus `create_step`s (`engine.rs:2828-2911`), bounded, and any error releases the lease (plan D149). | Accepted; stated in the code comment. |
| **F-35** | Minor (T8) | "`w` flips the executor". | From `Other(_)`, D10 lets the editor write only the two known values. | `w` proposes the other known value: `Tui → Worker`, `Worker → Tui`, `Other(_) → Tui` (back to the default). |
| **F-36** | Note (T9, R-6) | The worker re-scans every 5 s. | A `SlotFull`/`Overlaps` queued run is re-attempted every poll: `Kit::read` (≈ 8 reads) plus `claim_run`. | Accepted (R-6's per-user table), logged at `debug`. |
| **F-37** | Major (T7) | Store conformance: `edit_box_writes_the_executor_and_keeps_every_other_setting` (seed a three-key blob) and `edit_box_refuses_a_non_object_settings_blob` (both stores). | A conformance case is `run_case<S: WriteStore>(name, store)` over an already-loaded demo store (`conformance.rs:143-152`), with no seeding seam, and nothing on `WriteStore` writes arbitrary `box.settings` keys. The demo box's blob is `{"max_concurrent_items": 2}` (`htui-core/src/fixtures.rs:478`). | The generic case keeps the demo blob's keys; the three-key seed and the non-object refusal become `pg_criteria.rs` tests (raw `UPDATE box SET settings`) with MemStore twins in `mem.rs` (`MemStore::from_demo` over edited data). The third `CASES` entry is `an_executor_edit_is_a_compare_and_set`, so the pin stays at +3. |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B-1** (F-1): `check_step_batch` takes `Option<StepFence>`; `close_out` passes `None`.
- **B-2** (F-2): signature-first red commits: new signatures, implementors that accept and ignore the
  fence, rewritten call sites, red tests; enforcement is the green commit.
- **B-3** (F-7): library `serve_request`/`sweep_with`; TUI `trait TuiRuns { serve, sweep }`.
- **B-4** (F-9): `trait Publish` object behind `ChatEnd` and `ProgressSink<S>`.
- **B-5** (F-13): `edit_box` pre-read on bad tags or `Other`; `jsonb_typeof` guard; post-miss mapping.
- **B-6** (F-16): queued cancel = run CAS, then item `queued → open`, then cleanup.
- **B-7** (F-17): `reclaim` honours I-1.
- **B-8** (F-18): the claim scan runs on every gated sweep, after the adopt part.
- **B-9** (F-19): `start_run` mints its walk token before anything awaits after `enqueue`; the scan
  skips live runs.
- **B-10** (F-20): delayed-resume backoff, worker role only.
- **B-11** (F-28): the worker's first box beat is immediate; `htui worker ready` log line.
- **B-12** (F-37): T7's store `CASES` are the demo-blob executor write, the unknown-executor refusal
  and the executor CAS; the three-key seed and the non-object refusal are `pg_criteria.rs` tests
  with MemStore twins.

### 0b. Escalations for the maintainer

- **E-1 (Sentry in binary tests; F-30).** T12 cases 5 and 6 spawn the real `htui` binary, whose
  `main.rs:12-20` initialises Sentry with the production Glitchtip DSN. Case 6's exit 2 goes through
  `capture_anyhow` (`main.rs:26`) and sends one error event per test run from every machine that
  runs the suite. **Recommended:** `main.rs` does not capture a `WorkerExit::Refused` (exit 2).
  A refusal is a configuration state the user reads on stderr, not a crash, and `concepts.rs`'
  exit-1 refusals keep being captured as today. Case 6 then sends nothing, and case 5 exits 0.
  **Alternative:** capture every error as the plan's R-10 row implies, and accept test events in
  Glitchtip. The blueprint implements the recommendation behind one marked line (§14.4); a "no"
  deletes the `if`.
- **E-2 (queued-cancel crash window; F-16).** B-6 writes the run CAS and the item mirror in two
  statements, because `transition_run` is run-only and `finish_run` is not a CAS. A crash between
  them leaves the item `queued` with a cancelled run. No command recovers that: `R` needs
  `open | failed`, and `u` needs `blocked`. **Recommended:** accept it. It is a one-statement window
  in a human-triggered path, and it is named in `cancel_queued`'s doc and `docs/htui-worker.md`.
  **Alternative:** a 43rd `WorkerStore` method, `cancel_queued_run(run, at) -> bool`, doing both in
  one transaction on both stores (+1 `.sqlx`, +1 store conformance case). That needs an amendment
  to D4, which admits no method without a HEAD call site.

---

## 1. Build order

| Task | Crates | Commits (each compiles) | Gate |
|---|---|---|---|
| T1 | core, store, agent, orch | (1) red: three signatures, pass-through impls, 12 engine + 16 test call sites, `.sqlx` regenerated for the signature (text unchanged), 4 store + 1 orch conformance case + pins; (2) green: MemStore/Postgres enforcement, `step_fence`, `.sqlx` 289 | §19 G-T1 |
| T2 | orch | (1) red: `a_stepped_wall_clock_moves_no_fence`; (2) green: `heartbeat`, engine instants, `recover.rs` tests rewritten, the two new tests, docs | G-T2 |
| T3 | core, store, agent | (1) `store/worker.rs` traits + MemStore impls + core compile tests; (2) `htui-store/src/worker.rs` PgStore/Writer/Backend impls + P-4; (3) `UsageSpy`/`SpyStore` `RecorderStore` impls | G-T3 |
| T4 | agent, orch, htui (E0034 only) | (1) agent bounds + UFCS; (2) orch bounds + UFCS + comments | G-T4 |
| T5 | htui | (1) the new types and their two tests (`ReplySink`, `RunRequest`, `RunReply`, `TuiReplies`, `From`), green on arrival; (2) generic runtime in place, aliases, `TuiRuns`, call sites, test-module path edits | G-T5 |
| T6 | new htui-worker, htui | (1) crate skeleton and `tests/deps.rs` (a guard, green from day one); (2) the move, re-exports, `testing`, `Publisher` tests, test-module rewrites | G-T6 |
| T7 | core, store, orch (literals), htui (literals) | (1) red: `Executor` type + `BoxEdit.executor` + 18 literals + store cases + unit tests; (2) green: both `edit_box`s, `queued_runs_on_box` ×3 + host method, `.sqlx` 290 | G-T7 |
| T8 | htui | (1) red: three tests + new snapshot; (2) green: `Mode::Executor`, row line, footer, five snapshots re-accepted | G-T8 |
| T9 | orch, htui-worker, htui | (1) red: orch adopter cases + engine white-box + runtime + library tests; (2) green engine: `Tails`, `hand_back`, `cancel_queued`; (3) green runtime: `Role`, gate, scan, backoff, TUI hand-back | G-T9 |
| T10 | htui | (1) red: two backlog tests; (2) green: `on_refresh`, `has_active_run`, poll | G-T10 |
| T11 | store, htui (`concepts.rs`) | (1) red: pool + DSN tests; (2) green: `PoolSize`, `open_pool`, `connect_headless`, `headless_dsn`, `read_dsn_line`, `headless_refusal` | G-T11 |
| T12 | htui-worker, htui | (1) red: `cli.rs` tests; (2) CLI + `WorkerExit` + `main.rs`; (3) red: `worker_pg.rs`; (4) green: `worker.rs`, `worker_cmd.rs`, `lib.rs` arm, tracing, signals | G-T12 |
| T13 | docs | one commit | G-T13 |
| T14 | htui | (1) red: four `concepts.rs` tests + `qdrant_worker.rs`; (2) green: `sync_all`, `IndexSource`, `index_loop`, worker wiring | G-T14 |

---

## 2. Shared code shapes

### 2.1 `htui-core/src/store/worker.rs` (T3; `queued_runs_on_box` in T7)

`store/mod.rs` adds `mod worker;` and `pub use worker::{RecorderStore, WorkerHost, WorkerStore};`.
Imports mirror `traits.rs:1-54`. Every method is `fn … -> impl Future<Output = Result<T>> + Send`
(P-2); no default bodies. Signatures below are copied from `traits.rs` at HEAD, with T1's three
fenced signatures already applied.

```rust
//! The store surfaces the run supervisor and the engine compile against (MOD-41 plan D4).
//!
//! Three traits, each exactly its call sites at MOD-41's HEAD: [`RecorderStore`] for
//! `htui_agent::record`, [`WorkerStore`] for the engine, `gate` and `graph::resolve`, and
//! [`WorkerHost`] for the supervisor. None has a default body, so every implementor is found by
//! the compiler. They are **not** blanket-implemented over `WriteStore`: such an impl cannot prove
//! its futures `Send` (plan P-1), and the supervisor spawns them. Never `use` these traits in a
//! module that also sees `ReadStore`/`WriteStore` (E0034, plan D5): bound by path.

/// What a session recorder writes (`htui_agent::record`, plan D4).
pub trait RecorderStore: Send + Sync {
    /// [`WriteStore::append_events`].
    fn append_events(&self, fence: StepFence, events: &[SessionEvent])
    -> impl Future<Output = Result<usize>> + Send;
    /// [`WriteStore::set_step_usage`].
    fn set_step_usage(&self, fence: StepFence, step: StepId, usage: Value,
        prompt_digest: Option<String>) -> impl Future<Output = Result<()>> + Send;
    /// [`WriteStore::set_agent_box_quota`].
    fn set_agent_box_quota(&self, agent_id: AgentId, box_id: BoxId, quota: Value,
        quota_at: DateTime<Utc>) -> impl Future<Output = Result<bool>> + Send;
}

/// Every store call of the engine, `gate`, `graph::resolve` and the progress sink (plan D4).
pub trait WorkerStore: RecorderStore {
    // -- 13 ReadStore reads
    fn item(&self, id: ItemId) -> impl Future<Output = Result<Option<Item>>> + Send;
    fn documents(&self, id: ItemId) -> impl Future<Output = Result<Vec<DocumentHead>>> + Send;
    fn runs(&self, id: ItemId) -> impl Future<Output = Result<Vec<RunSummary>>> + Send;
    fn step_events(&self, step: StepId)
    -> impl Future<Output = Result<Option<Vec<SessionEvent>>>> + Send;
    fn document(&self, id: DocumentId) -> impl Future<Output = Result<Option<Document>>> + Send;
    fn documents_of_kinds(&self, item: ItemId, kinds: &[String])
    -> impl Future<Output = Result<Vec<Document>>> + Send;
    fn upstream_summaries(&self, id: ItemId, hops: u8, scope: &PromptScope)
    -> impl Future<Output = Result<Vec<UpstreamEntry>>> + Send;
    fn project(&self, id: ProjectId) -> impl Future<Output = Result<Option<Project>>> + Send;
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>>> + Send;
    fn run_steps(&self, run: RunId) -> impl Future<Output = Result<Vec<RunStep>>> + Send;
    fn step_trees(&self, step: StepId) -> impl Future<Output = Result<Vec<RunStepTree>>> + Send;
    fn step_commits(&self, step: StepId)
    -> impl Future<Output = Result<Vec<RunStepCommit>>> + Send;
    fn resolve_inputs(&self, item: ItemId, run: RunId, kinds: &[String])
    -> impl Future<Output = Result<Vec<ResolvedInput>>> + Send;
    // -- 5 WriteStore reads
    fn repos(&self, project: ProjectId) -> impl Future<Output = Result<Vec<Repo>>> + Send;
    fn repo_box_paths(&self, repo: RepoId) -> impl Future<Output = Result<Vec<RepoBoxPath>>> + Send;
    fn item_kinds(&self, project: ProjectId) -> impl Future<Output = Result<Vec<ItemKind>>> + Send;
    fn phases(&self, graph: StepGraphId)
    -> impl Future<Output = Result<Vec<StepGraphPhase>>> + Send;
    fn command_runs(&self, step: StepId) -> impl Future<Output = Result<Vec<CommandRun>>> + Send;
    // -- 23 writes
    fn transition(&self, id: ItemId, from: Status, to: Status)
    -> impl Future<Output = Result<bool>> + Send;
    fn set_step_prompt(&self, fence: StepFence, step: StepId, digest: &str, trim: &Value)
    -> impl Future<Output = Result<()>> + Send;
    fn create_run(&self, new: NewRun) -> impl Future<Output = Result<Run>> + Send;
    fn claim_run(&self, run: RunId, box_id: BoxId, owner: Uuid, at: DateTime<Utc>,
        ttl: TimeDelta) -> impl Future<Output = Result<Claim>> + Send;
    fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta)
    -> impl Future<Output = Result<bool>> + Send;
    fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta)
    -> impl Future<Output = Result<Vec<Run>>> + Send;
    fn take_lease(&self, run: RunId, box_id: BoxId, owner: Uuid, ttl: TimeDelta)
    -> impl Future<Output = Result<bool>> + Send;
    fn release_lease(&self, run: RunId, owner: Uuid) -> impl Future<Output = Result<bool>> + Send;
    fn create_step(&self, new: NewRunStep) -> impl Future<Output = Result<RunStep>> + Send;
    fn transition_run(&self, run: RunId, from: RunStatus, to: RunStatus, at: DateTime<Utc>)
    -> impl Future<Output = Result<bool>> + Send;
    fn transition_step(&self, step: StepId, from: StepStatus, to: StepStatus,
        at: DateTime<Utc>) -> impl Future<Output = Result<bool>> + Send;
    fn finish_step(&self, fence: StepFence, step: StepId, outcome: StepOutcome)
    -> impl Future<Output = Result<()>> + Send;
    fn interrupt_step(&self, step: StepId, note: &str, at: DateTime<Utc>)
    -> impl Future<Output = Result<bool>> + Send;
    fn answer_gate(&self, step: StepId, outcome: GateOutcome, note: Option<String>,
        at: DateTime<Utc>) -> impl Future<Output = Result<bool>> + Send;
    fn select_fanout(&self, run: RunId, position: i32, attempt: i32, winner: StepId,
        reason: Option<String>) -> impl Future<Output = Result<()>> + Send;
    fn supersede_step(&self, step: StepId) -> impl Future<Output = Result<()>> + Send;
    fn upsert_step_tree(&self, fence: StepFence, step: StepId, trees: &[RunStepTree])
    -> impl Future<Output = Result<()>> + Send;
    fn record_commits(&self, fence: StepFence, step: StepId, commits: &[RunStepCommit])
    -> impl Future<Output = Result<()>> + Send;
    fn record_command_run(&self, new: NewCommandRun)
    -> impl Future<Output = Result<CommandRun>> + Send;
    fn promote_step(&self, step: StepId, at: DateTime<Utc>)
    -> impl Future<Output = Result<()>> + Send;
    fn finish_run(&self, run: RunId, to: RunStatus, failure: Option<&str>, at: DateTime<Utc>)
    -> impl Future<Output = Result<()>> + Send;
    fn close_out(&self, item: ItemId, resolution: Resolution, summary: NewDocument,
        commits: &[RunStepCommit]) -> impl Future<Output = Result<Document>> + Send;
    fn add_note(&self, note: NewNote) -> impl Future<Output = Result<Note>> + Send;
    // -- the progress sink's document write (plan D4, PRD D5: no production caller until MOD-11)
    fn write_document(&self, new: NewDocument) -> impl Future<Output = Result<Document>> + Send;
}

/// The supervisor's source (plan D4): the process's store and every read the runtime makes.
pub trait WorkerHost: Clone + Send + Sync + 'static {
    /// The store engines write through.
    type Store: WorkerStore + Clone + Send + Sync + 'static;
    /// The store, or `None` off the server (`Backend::writer`, `backend.rs:148`).
    fn writer(&self) -> Option<Self::Store>;
    // -- 9 of Backend's inherent reads
    fn box_info(&self) -> impl Future<Output = Result<Option<BoxInfo>>> + Send;
    fn this_user(&self) -> impl Future<Output = Result<UserId>> + Send;
    fn app_settings(&self) -> impl Future<Output = Result<BTreeMap<String, Value>>> + Send;
    fn box_profile(&self, id: BoxId) -> impl Future<Output = Result<Option<BoxProfile>>> + Send;
    fn agents(&self) -> impl Future<Output = Result<Vec<AgentSummary>>> + Send;
    fn box_row(&self, id: BoxId) -> impl Future<Output = Result<Option<BoxRow>>> + Send;
    fn repo_paths(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<RepoBoxPath>>> + Send;
    fn workspaces(&self) -> impl Future<Output = Result<Vec<WorkspaceSummary>>> + Send;
    fn active_runs_on_box(&self, box_id: BoxId) -> impl Future<Output = Result<usize>> + Send;
    // -- 5 ReadStore reads on the host
    fn item(&self, id: ItemId) -> impl Future<Output = Result<Option<Item>>> + Send;
    fn documents(&self, id: ItemId) -> impl Future<Output = Result<Vec<DocumentHead>>> + Send;
    fn runs(&self, id: ItemId) -> impl Future<Output = Result<Vec<RunSummary>>> + Send;
    fn run(&self, id: RunId) -> impl Future<Output = Result<Option<Run>>> + Send;
    fn run_steps(&self, run: RunId) -> impl Future<Output = Result<Vec<RunStep>>> + Send;
    // -- 6 inherent reads behind GraphSource
    fn resolve_graph(&self, item: ItemId)
    -> impl Future<Output = Result<Option<ResolvedGraph>>> + Send;
    fn phase_agents(&self, phase: PhaseId) -> impl Future<Output = Result<Vec<PhaseAgent>>> + Send;
    fn prompt_template(&self, project: ProjectId, name: &str, version: Option<i32>)
    -> impl Future<Output = Result<Option<PromptTemplate>>> + Send;
    fn agent_boxes(&self, box_id: BoxId) -> impl Future<Output = Result<Vec<AgentBox>>> + Send;
    fn bound_skills(&self, project: ProjectId, phase: Option<PhaseId>)
    -> impl Future<Output = Result<Vec<BoundSkill>>> + Send;
    fn missing_tags(&self, item: ItemId, box_id: BoxId)
    -> impl Future<Output = Result<Vec<String>>> + Send;
    // -- T7 (plan D11)
    /// This box's `queued` runs, `(id, queued_at)` in `(queued_at, id)` order.
    fn queued_runs_on_box(&self, box_id: BoxId)
    -> impl Future<Output = Result<Vec<(RunId, DateTime<Utc>)>>> + Send;
}
```

Each method carries a one-line doc naming the method it forwards to (elided above). The
`MemStore` impls in the same file are all shaped like this:

```rust
impl WorkerStore for MemStore {
    async fn claim_run(&self, run: RunId, box_id: BoxId, owner: Uuid, at: DateTime<Utc>,
        ttl: TimeDelta) -> Result<Claim> {
        WriteStore::claim_run(self, run, box_id, owner, at, ttl).await
    }
    // … 41 more, each UFCS to ReadStore:: or WriteStore::
}
```

### 2.2 `htui-store/src/worker.rs` (T3; T7 adds `queued_runs_on_box`)

`lib.rs` adds `mod worker;` (impls only, nothing `pub`). `impl RecorderStore` + `impl WorkerStore`
for `PgStore` and `Writer` (UFCS to `WriteStore::`/`ReadStore::`), and `impl WorkerHost` for:

```rust
impl htui_core::store::WorkerHost for PgStore {
    type Store = PgStore;
    fn writer(&self) -> Option<PgStore> { Some(self.clone()) }
    async fn this_user(&self) -> Result<UserId> { Ok(PgStore::this_user(self)) } // const fn, pg/mod.rs:649
    async fn box_row(&self, id: BoxId) -> Result<Option<BoxRow>> { PgStore::box_row(self, id).await }
    async fn item(&self, id: ItemId) -> Result<Option<Item>> { ReadStore::item(self, id).await }
    // workspaces/box_info/agents/bound_skills/box_profile/app_settings/phase_agents/prompt_template/
    // resolve_graph/agent_boxes/box_row/repo_paths/missing_tags/active_runs_on_box are inherent on
    // PgStore (pg/read.rs:1137-1974): `PgStore::name(self, …)`.
}
impl htui_core::store::WorkerHost for Backend {
    type Store = Writer;
    fn writer(&self) -> Option<Writer> { Backend::writer(self) }
    // the nine: `Backend::name(self, …)`; the five: `ReadStore::name(self, …)`; the six: `Backend::name`.
}
```

### 2.3 `Executor` (T7, `htui-core/src/model/box_.rs`, re-exported from `model/mod.rs`)

```rust
/// Which process executes runs on a box: `box.settings.executor` (MOD-41 PRD D4, plan D9, I-1).
///
/// Decoding never fails: `"tui"`/`"worker"` are their variants, any other string or JSON value is
/// [`Executor::Other`], which neither role executes under (fail closed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum Executor {
    /// The TUI's run runtime claims, adopts and sweeps (the default).
    #[default]
    Tui,
    /// `htui worker` does; the TUI queues and hands back.
    Worker,
    /// A value this build does not know, kept as written (`7`, `"container"`, `null` → `"null"`).
    Other(String),
}

impl Executor {
    /// The `box.settings` key.
    pub const KEY: &'static str = "executor";

    /// As stored: `tui`, `worker`, or the unknown text.
    #[must_use]
    pub fn as_str(&self) -> &str { match self { Self::Tui => "tui", Self::Worker => "worker", Self::Other(text) => text } }

    fn from_value(value: &Value) -> Self {
        match value.as_str() {
            Some("tui") => Self::Tui,
            Some("worker") => Self::Worker,
            Some(other) => Self::Other(other.to_owned()),
            None => Self::Other(value.to_string()),
        }
    }

    /// I-1's reading (plan D9, fact-check F-M2): the `executor` key alone, never the whole
    /// `BoxSettings` decode, so a malformed sibling key cannot make a worker box read as a TUI
    /// box. Missing key → `Tui`; a non-object blob → `Other`, which fails closed.
    #[must_use]
    pub fn of(settings: &Value) -> Self {
        match settings {
            Value::Object(map) => map.get(Self::KEY).map_or(Self::Tui, Self::from_value),
            other => Self::Other(format!("<box.settings is not an object: {other}>")),
        }
    }
}

impl core::fmt::Display for Executor { /* as_str */ }
impl Serialize for Executor { /* serializer.serialize_str(self.as_str()) */ }

/// `BoxSettings.executor`'s decoder: whatever the value, an `Executor` (plan D9).
fn executor_lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Executor, D::Error> {
    Value::deserialize(d).map(|value| Executor::from_value(&value))
}
```

`BoxSettings` gains, after `command_limits`:

```rust
    /// `box.settings.executor` for display and round-trips. **I-1 never reads this field**: it
    /// reads [`Executor::of`] (a bad sibling key would make this struct fall back to default).
    #[serde(default, deserialize_with = "executor_lenient")]
    pub executor: Executor,
```

`box_.rs:237-239`'s doc is corrected (no `set_setting` rung writes it; `edit_box` does, plan D10).
`BoxEdit` gains `/// box.settings.executor; Some writes that key only. Other(_) is refused.`
`pub executor: Option<Executor>,`.

### 2.4 `Tails` (T9, `htui-orch/src/engine.rs`, re-exported from `lib.rs`)

```rust
/// Who walks a command's tail (MOD-41 plan D12, OQ-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tails {
    /// This engine walks every tail: the TUI on a `tui` box, and `htui worker` always.
    Walk,
    /// The command's leased window is written, then the lease is released and the run's rest is
    /// read: the box's worker adopts the run at its next poll (the TUI on a `worker` box).
    HandBack,
}
```

`EngineParts` gains, after `user`:
`/// Plan D12: walk command tails, or hand them back to the box's worker. pub tails: Tails,`
and the `Debug` impl adds `.field("tails", &self.tails)`.

### 2.5 The addressing family (T5 in `run_worker.rs`, T6 in `htui-worker/src/address.rs`)

```rust
/// Where the run runtime's answers and frames go (plan D7). The TUI's is `TuiReplies`; the
/// worker's is [`Unaddressed`].
pub trait ReplySink: Clone + Send + Sync + 'static {
    /// One request's address.
    type Addr: Clone + Send + Sync + core::fmt::Debug + 'static;
    /// Who a subscription belongs to: a later subscription of the same subscriber replaces it.
    type Subscriber: Clone + Eq + core::hash::Hash + Send + Sync + core::fmt::Debug + 'static;
    /// The subscriber `addr` belongs to.
    fn subscriber(addr: &Self::Addr) -> Self::Subscriber;
    /// One reply to `to`. Never fails: a gone receiver drops it, as today's `let _ = send`.
    fn send(&self, to: &Self::Addr, reply: RunReply);
}

/// One request the runtime serves (plan D7): the payload of `StoreRequest::{Orch, RunStream,
/// RunActions}`, without the TUI's envelope.
#[derive(Debug, Clone)]
pub enum RunRequest {
    /// A command or an orchestrator read.
    Orch(OrchRequest),
    /// Follow an item's runs.
    Stream { /** The item. */ item: ItemId },
    /// Every verdict for an item.
    Actions(ItemId),
}

impl RunRequest {
    /// Exactly `StoreRequest::name`'s string (`store_worker.rs:834-837`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Orch(request) => request.name(),
            Self::Stream { .. } => "run_stream",
            Self::Actions(_) => "run_actions",
        }
    }
}

/// One answer (plan D7), one-to-one with `StoreReply::{Orch, RunStream, RunActions, Failed}`.
#[derive(Debug, Clone)]
pub enum RunReply {
    /// A command's or read's outcome.
    Orch(OrchReply),
    /// A frame of a subscribed item.
    Frame(RunFrame),
    /// Every verdict for an item.
    Actions(Box<ItemActions>),
    /// The request failed with the sentence.
    Failed { /** The request's name. */ request: &'static str, /** Why. */ message: String },
}

/// What `serve_request` (and the runtime's event channel) decided about one request.
#[derive(Debug)]
pub enum RunServed<A> {
    /// Answer with this reply, now.
    Reply(RunReply),
    /// A task of the runtime answers, exactly once.
    Deferred,
    /// A promotion's writes are done; the host hands `promoted` to its chat runtime (MOD-4 D181).
    Attach { /** The request's address. */ addr: A, /** The step. */ promoted: Box<Promoted>,
             /** What the chat's end publishes. */ ended: ChatEnd },
}

/// The worker's sink: it serves no request, so nothing is addressed (plan D7). `Attach` cannot be
/// built for it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unaddressed;

impl ReplySink for Unaddressed {
    type Addr = core::convert::Infallible;
    type Subscriber = core::convert::Infallible;
    fn subscriber(addr: &Self::Addr) -> Self::Subscriber { match *addr {} }
    fn send(&self, to: &Self::Addr, _reply: RunReply) { match *to {} }
}
```

`ChatEnd::after` (B-4):

```rust
impl ChatEnd {
    /// `task`, then a `Changed` frame for the promoted step's item (MOD-4 D212).
    pub fn after<F>(self, task: F) -> impl Future<Output = ()> + Send + 'static
    where F: Future<Output = ()> + Send + 'static {
        async move {
            task.await;
            if let Some(item) = self.tag.item.get() {
                self.publisher.publish(&RunFrame { item: *item, run: self.tag.run.get().copied(),
                                                   kind: FrameKind::Changed });
            }
        }
    }
}
```

`Publisher<P>`: `Arc<StdMutex<Subscribers<P>>>`, `Subscribers<P> { subs: HashMap<P::Subscriber,
(P::Addr, ItemId)>, sink: Option<P> }`; `wire(&self, sink: &P)` replaces unconditionally (F-10);
`subscribe(&self, addr: P::Addr, item)` inserts under `P::subscriber(&addr)`; `publish` sends
`RunReply::Frame(frame.clone())` to each `(addr, item)` whose item matches. Hand-written `Debug`
printing the subscriber count.

### 2.6 The TUI adapter (T5; stays in `htui/src/run_worker.rs` after T6)

```rust
/// The TUI's reply sink: the store loop's reply channel (plan D7).
#[derive(Debug, Clone)]
pub struct TuiReplies(pub mpsc::UnboundedSender<ReplyEnvelope>);

impl htui_worker::ReplySink for TuiReplies {
    type Addr = ReplyAddr;
    type Subscriber = Origin;
    fn subscriber(addr: &ReplyAddr) -> Origin { addr.origin.clone() }
    fn send(&self, to: &ReplyAddr, reply: RunReply) {
        let _ = self.0.send(ReplyEnvelope { seq: to.seq, origin: to.origin.clone(), reply: reply.into() });
    }
}

impl From<RunReply> for StoreReply {
    fn from(reply: RunReply) -> Self {
        match reply {
            RunReply::Orch(reply) => Self::Orch(reply),
            RunReply::Frame(frame) => Self::RunStream(frame),
            RunReply::Actions(actions) => Self::RunActions(actions),
            RunReply::Failed { request, message } => Self::Failed { request, message },
        }
    }
}

/// The TUI's run runtime (plan D7): the library's, over the `Backend` and the loop's channel.
pub type RunRuntime = htui_worker::RunRuntime<Backend, TuiReplies>;
/// What the TUI's runtime decided about one request.
pub type RunServed = htui_worker::RunServed<ReplyAddr>;

/// Today's `serve`/`sweep` signatures over the library's (B-3).
pub trait TuiRuns {
    /// One `Orch`, `RunStream` or `RunActions` envelope; anything else is refused.
    fn serve(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope, live: &LiveChats) -> impl Future<Output = RunServed> + Send;
    /// One sweep tick.
    fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>);
}

impl TuiRuns for RunRuntime {
    async fn serve(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope, live: &LiveChats) -> RunServed {
        let request = match &envelope.request {
            StoreRequest::Orch(request) => RunRequest::Orch(request.clone()),
            StoreRequest::RunStream { item } => RunRequest::Stream { item: *item },
            StoreRequest::RunActions(item) => RunRequest::Actions(*item),
            other => return RunServed::Reply(RunReply::Failed {
                request: other.name(), message: "not an orchestrator request".to_owned() }),
        };
        let addr = ReplyAddr { seq: envelope.seq, origin: envelope.origin.clone() };
        self.serve_request(backend, &TuiReplies(replies.clone()), addr, request, live).await
    }
    fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>) {
        self.sweep_with(backend, &TuiReplies(replies.clone()));
    }
}
```

Re-exports (T6): `pub use htui_worker::{ChatEnd, Enabled, FrameKind, HostGraphs, ItemActions,
LiveChats, ORCH_NAMES, OrchReply, OrchRequest, PREEMPTED, Promoted, ProgressSink, REPOS_MOVED,
RunActions, RunFrame, RunLocks, RunReply, RunRequest, StepActions, StepAuthor, UNBLOCK_MOVED, Via,
WALK_PANICKED, actions};` That covers every name at `app/update.rs:12`, `ui/tabs/chat/mod.rs:39`,
`:66`, `ui/tabs/backlog/detail/runs.rs:48`, `:1336`, `agent_worker.rs:844`, `:858`, `:883`,
`:1993`, `tests/backlog.rs:14`, `tests/chat.rs:19`, `tests/runs_pg.rs:38`, and `store_worker.rs`.

---

## 3. T1 — fence the three step writes (D1, D2)

### 3.1 Trait (`htui-core/src/store/traits.rs`)

- `:523` → `async fn set_step_prompt(&self, fence: StepFence, step: StepId, digest: &str, trim: &Value) -> Result<()>;`
- `:1219` → `async fn upsert_step_tree(&self, fence: StepFence, step: StepId, trees: &[RunStepTree]) -> Result<()>;`
- `:1228` → `async fn record_commits(&self, fence: StepFence, step: StepId, commits: &[RunStepCommit]) -> Result<()>;`

Each doc gains the `set_step_usage` paragraph (`:301-316`'s wording): writes only while the step's
run carries `fence`'s lease; `NotFound { entity: "run_step" }` for a missing step **before**
`Fenced { step }`; for the batch writes, then the batch's `Constraint`s. `StepFence`'s doc
(`:2076-2083`) lists six methods instead of three.

### 3.2 Postgres (`htui-store/src/pg/write.rs`)

`set_step_prompt` (`:1648-1667`):

```rust
async fn set_step_prompt(&self, fence: StepFence, step: StepId, digest: &str, trim: &Value)
-> Result<()> {
    let updated = sqlx::query!(
        "UPDATE run_step SET prompt_digest = $2, trim_record = $3 \
          WHERE id = $1 \
            AND EXISTS (SELECT 1 FROM run r \
                         WHERE r.id = run_step.run_id \
                           AND r.lease_owner IS NOT DISTINCT FROM $4 \
                           FOR SHARE)",
        step.as_uuid(), digest, trim, fence.owner(),
    )
    .execute(&self.pool).await.map_err(map_sqlx)?.rows_affected();
    if updated == 1 {
        return Ok(());
    }
    // Boxed as `set_step_usage`'s miss is (`:187-196`): a debug build's worker stack.
    Err(Box::pin(fenced_or_missing(&self.pool, step)).await)
}
```

New private helper beside `step_exists` (`:168`):

```rust
/// MOD-41 plan D1: `step` exists and its run carries `fence`'s lease, read `FOR SHARE OF r`
/// inside the caller's transaction, so an adoption cannot commit between this check and the
/// batch's writes. `NotFound` first, then `Fenced`: `append_events`' order.
async fn step_fence(conn: &mut PgConnection, step: StepId, fence: StepFence) -> Result<()> {
    let owner = sqlx::query_scalar!(
        r#"SELECT r.lease_owner AS "lease_owner?"
             FROM run_step s JOIN run r ON r.id = s.run_id
            WHERE s.id = $1
              FOR SHARE OF r"#,
        step.as_uuid(),
    )
    .fetch_optional(conn).await.map_err(map_sqlx)?
    .ok_or_else(|| StoreError::NotFound { entity: "run_step", id: step.to_string() })?;
    if owner == fence.owner() { Ok(()) } else { Err(StoreError::Fenced { step }) }
}
```

`upsert_step_tree` (`:4560`) and `record_commits` (`:4639`): the new `fence` parameter, and
`step_exists(&mut tx, step).await?;` (`:4562`, `:4641`) becomes `step_fence(&mut tx, step,
fence).await?;`. Nothing else in either body moves. `step_exists` keeps its other callers
(`record_command_run`, `close_out`, `fenced_or_missing`).

### 3.3 MemStore (`htui-core/src/store/mem.rs`)

- `State::set_step_prompt` (`:1580`) gains `fence: StepFence`. After the `NotFound`,
  `Self::fence_holds(&self.lease_owners, row, fence)?;`, before the writes. Take `row` with
  `self.steps.get_mut(&step)`; the borrow of `self.lease_owners` is disjoint.
- B-1: `check_step_batch(&self, table, step, fence: Option<StepFence>, rows)`:
  `let row = self.require_step(step)?; if let Some(fence) = fence { Self::fence_holds(&self.lease_owners, row, fence)?; }`,
  then today's loop. `upsert_step_tree` (`:4550`) and `record_commits` (`:4585`) gain
  `fence: StepFence` and pass `Some(fence)`; `close_out` (`:4850`) passes `None`.
- The `impl WriteStore for MemStore` wrappers (`:5766`, `:6154`, `:6159`) forward the fence.

### 3.4 Delegates and callers

- `Writer` (`htui-store/src/writer.rs:444-445`, `:953-954`, `:960-961`), `UsageSpy`
  (`htui-agent/src/conformance.rs:815`, `:1126`, `:1129`), `SpyStore`
  (`htui-agent/tests/recorder.rs:550`, `:865`, `:868`): forward `fence`.
- Engine (D2), all `StepFence::Lease(self.parts.owner)`: `set_step_prompt` `engine.rs:3181`,
  `:3765`, `:4590`; `upsert_step_tree` `:3129`, `:3745`; `record_commits` `:1402`, `:3140`,
  `:3211`, `:3691` (warn-and-swallow unchanged), `:3756`, `:3812`, `:4747`. `run_to_rest`'s doc
  (`:2552-2560`) gains "call only under `walk_leased`: its step writes carry this process's
  lease".
- Test call sites: `Lease(owner)` when the fixture holds a lease, `Unleased` only when it does not:
  `engine.rs:6728`, `gate.rs:1540`, orch `conformance.rs:4277`, `:4485`, `:6465`, `:6806`,
  `tests/gix_isolator.rs:2413`, `record.rs:2123`, `htui-store/tests/cache.rs:517`,
  `pg_criteria.rs:1289`, `:1296`, `:1333`, `:3292`, `:4406`, `:4429`, `:4439`, and the MemStore
  unit test `set_step_prompt_writes_both_columns` (`mem.rs:6538`). A fixture step of a run nobody
  claimed has `lease_owner NULL` → `Unleased`. Re-grep the method names; do not trust this list.

### 3.5 Commits

1. **red** (B-2): §3.1; implementors take the fence and **ignore it** (Pg bodies unchanged but
   for the parameter; MemStore passes `None` to `check_step_batch` and skips the check in
   `set_step_prompt`); every call site; the tests in §3.6; the pins; `cargo sqlx prepare -- --all-features --all-targets`
   (nothing changes yet). Red: the fenced assertions of cases 1 and 3, and the orch case.
2. **green**: §3.2 and §3.3 enforcement; `.sqlx` regenerated (`set_step_prompt`'s file replaced,
   `step_fence`'s new, **289**).

### 3.6 Tests

Store conformance (generic `<S: WriteStore>`, both stores; each appended to `CASES` after the
current last entry, arm before `other => panic!`, body after that case's body):

| Case | Assertion |
|---|---|
| `a_stale_owner_writes_no_prompt_tree_or_commits` | Lease a step to A (`leased_step`, `conformance.rs:5590`), release it, adopt as B (`adopt_runs(box, b, ttl)`). A's `set_step_prompt(Lease(a), …)`, `upsert_step_tree(Lease(a), …)` and `record_commits(Lease(a), …)` each answer `Fenced { step }`. The step's `prompt_digest`, `trim_record` and `isolation_path`, `step_trees(step)` and `step_commits(step)` equal their values before the three calls. |
| `the_new_owner_writes_prompt_tree_and_commits` | Same setup; B's three writes answer `Ok`; the digest and trim, one tree row and `isolation_path`, and one commit row read back as written. |
| `an_unleased_prompt_write_is_refused_on_a_leased_run` | On A's live lease, `Unleased` is `Fenced` for all three and writes nothing; on a chat run (`start_chat_run`, lease `NULL`) `set_step_prompt(Unleased, …)` is `Ok`. |
| `a_missing_step_is_not_found_before_the_fence` | An unknown step with `Lease(Uuid::now_v7())`: all three are `NotFound { entity: "run_step" }`, never `Fenced`; a batch with a row naming another step on an existing but fenced step is `Fenced` (fence before batch). |

MemStore unit test (not a `CASES` entry): `close_out_records_commits_without_a_fence` (B-1): a
finished run whose lease was released closes out with one commit row; the commit is stored.

Orch conformance: `a_walk_woken_after_done_records_no_commits`. A walks to a candidate's `Done` and
stalls there (`stall_after_done`). B (`restarted()`) adopts: A's lease is lapsed by B's clock. A
resumes, and its capture's `record_commits(Lease(a))` meets `Fenced`. A ends `LeaseLost` through
`heartbeaten`'s `is_fenced` arm (`engine.rs:1729-1741`). Then `step_commits(step)` and
`step_trees(step)` hold only what B wrote (or nothing), and A released no lease.

Pins: store `CASES` 96 → **100** (`conformance.rs` `CASES`, tally at `:12511`/`:12520`,
`htui-core/tests/mem_store.rs:36-37`, `htui-store/tests/pg_conformance.rs:19`); orch 73 → **74**
(`conformance.rs:5889`, `tests/fake_conformance.rs:16`). Each tally sentence names "MOD-41 T1's
four fence cases" / "T1's fenced capture".

### 3.7 Gate

G-T1 (§19): build, core + store + agent + orch gates, sqlx check (289), clippy, fmt.

---

## 4. T2 — monotonic self-fence (D3)

### 4.1 `htui-orch/src/recover.rs`

```rust
/// Sleep, then `refresh(times.ttl)` … (today's doc, `:103-118`, with every "clock" sentence
/// replaced): the fence is measured on **tokio's monotonic clock** (MOD-41 plan D3, MOD-40
/// blueprint F-38). `written` is the instant the caller's lease write was sent plus the TTL;
/// each successful refresh moves it to its own send instant plus the TTL. A wall-clock step
/// can neither extend nor cut it.
pub async fn heartbeat<F, Fut>(mut refresh: F, written: tokio::time::Instant, times: LeaseTimes)
-> Heartbeat
where
    F: FnMut(TimeDelta) -> Fut,
    Fut: Future<Output = Result<bool, StoreError>>,
{
    use tokio::time::Instant;
    let retry = (times.refresh / 4).max(MIN_RETRY);
    let ttl = times.ttl.to_std().unwrap_or(Duration::ZERO);
    // F-4: an instant cannot go below the platform's origin; failing to subtract fences now.
    let fence_of = |until: Instant| until.checked_sub(times.refresh).unwrap_or_else(Instant::now);
    let mut fence = fence_of(written);
    let mut interval = times.refresh.min(fence.saturating_duration_since(Instant::now()));
    loop {
        tokio::time::sleep(interval).await;
        let sent = Instant::now();
        let left = fence.saturating_duration_since(sent);
        let Ok(answer) = tokio::time::timeout(left, refresh(times.ttl)).await else {
            tracing::warn!("lease refresh still pending at the fence; the walk stops");
            return Heartbeat::Expired;
        };
        match answer {
            Ok(true) => { fence = fence_of(sent + ttl); interval = times.refresh; }
            Ok(false) => return Heartbeat::Abandoned,
            Err(error) => {
                let now = Instant::now();
                if now >= fence {
                    tracing::warn!(%error, "lease refresh failed at the fence; the walk stops");
                    return Heartbeat::Expired;
                }
                tracing::warn!(%error, "lease refresh failed; retrying sooner");
                interval = retry.min(fence - now);
            }
        }
    }
}
```

`Clock` is no longer imported by `recover.rs` production code. Tests: delete `PausedClock`
(`:502-523`) and rewrite `scripted` to record `(tokio::time::Instant, TimeDelta)`; `beats` becomes
`(at - start).as_secs()` from a `start = Instant::now()` taken first. The ten `heartbeat_*` cases
(`:569-800`) keep names and outcomes, and pass `start + default_times().ttl.to_std()` (or their
own `written`) instead of `clock.now() + ttl`.

### 4.2 `htui-orch/src/engine.rs`

```rust
/// The instant a lease sent at `sent` lapses by at the earliest, on tokio's clock (plan D3).
fn lease_until(&self, sent: tokio::time::Instant) -> tokio::time::Instant {
    sent + self.lease_times().ttl.to_std().unwrap_or(std::time::Duration::ZERO)
}
```

- `claim` (`:685-722`): `let sent = tokio::time::Instant::now();` beside `let now = self.now();`
  before `claim_run`; `walk_leased(run, self.lease_until(sent), …)`. `now` still stamps
  `started_at`.
- `renew_lease` (`:1828-1841`) → `Result<Option<tokio::time::Instant>, EngineError>`; `sent` is
  read before `take_lease`; `Ok(taken.then(|| self.lease_until(sent)))`. `take_lease`
  (`:1790`) → `Result<tokio::time::Instant, EngineError>`. Docs at `:1787` and `:1816-1822`
  ("the local instant … `now + ttl`") say "tokio's monotonic instant".
- `walk_leased` (`:1681`), `heartbeaten` (`:1710`): `until: tokio::time::Instant`;
  `heartbeaten` calls `recover::heartbeat(|ttl| store.refresh_lease(run, owner, ttl), until, times)`.
- `fresh_until` (`:1876`, `#[cfg(test)]`) → `self.lease_until(tokio::time::Instant::now())`.
- Callers that only pass `until` through compile unchanged: `:804`, `:971`, `:1045`, `:1089`,
  `:2681`, `:1373` (accept), `:2006` (sweep). `:1180` and `:1613` discard it.
- `recover.rs:118-120`'s "a monotonic clock is MOD-41's" is replaced by the doc above.
  `htui/src/run_worker.rs:2671-2672` (`TokioClock` "moves the lease fence") lands with T5.

### 4.3 Commits and tests

1. **red**: `engine.rs` test `a_stepped_wall_clock_moves_no_fence` (`#[tokio::test(start_paused = true)]`).
   A leased run with `MemFault::RefreshLease` on. The walk loops `sleep(1 s)`, and the test
   `advance`s the harness `TestClock` by +1 h at paused 10 s and by −2 h at paused 20 s. Assert
   `LeaseLost` at paused elapsed == `ttl − refresh` (120 − 40 = 80 s) after the take. Today it is
   red: +1 h expires at 10 s.
2. **green**: §4.1 and §4.2; the ten rewritten `heartbeat_*` cases; new
   `recover.rs::heartbeat_fences_on_tokio_time_alone`: every refresh fails; `Expired` at exactly
   `written − refresh` of paused time, whatever a `TestClock` handed nowhere says. New
   `engine.rs::a_forward_step_does_not_expire_a_live_lease`: refreshes succeed, +1 day step, the
   walk is still pending after three intervals (a regression pin, F-3; the commit message says
   so).

### 4.4 Gate

G-T2: orch gate, then by name at `--test-threads=1`:
`a_walk_whose_refresh_keeps_failing_stops_before_the_lease_lapses` (`engine.rs:8520`), the cases
at `:8601`, `:8653`, `:11426`, plus the `htui` runtime tests (`cargo test -p htui run_worker`), all
unchanged in outcome.

---

## 5. T3 — the three traits and their impls (D4, D5)

- §2.1 in full (21 `WorkerHost` methods, F-5), with `MemStore`'s `RecorderStore` + `WorkerStore`
  impls in the same file.
- §2.2 in full, without `queued_runs_on_box`.
- `UsageSpy`: `impl<S: WriteStore + htui_core::store::RecorderStore> htui_core::store::RecorderStore for UsageSpy<'_, S>`,
  whose three bodies are `htui_core::store::RecorderStore::append_events(self.inner, fence, events).await`
  (fact-check P-1b). Check the field name at `htui-agent/src/conformance.rs:576`.
- `SpyStore`: `impl htui_core::store::RecorderStore for SpyStore`, forwarding to its own
  `WriteStore` methods by UFCS (it records the calls; keep that).
- No existing bound changes (the workspace compiles exactly as before).

**Commits**: (1) core traits + MemStore impls + core tests; (2) store impls + store tests;
(3) agent spy impls.

**Tests** (compile-only unless noted):
- `htui-core/src/store/worker.rs`: `worker_store_is_object_of_the_engines_calls`
  (`fn is_recorder<S: RecorderStore>() {}`, `fn is_worker<S: WorkerStore>() {}`, both for
  `MemStore`), and `a_mem_store_worker_future_spawns` (P-4, `#[tokio::test]`: `tokio::spawn` of a
  generic `async fn probe<S: WorkerStore + Clone + 'static>(s: S) -> Result<Option<Run>>`, which
  awaits `s.run(RunId::new())`, over `MemStore::demo()`; asserts `Ok(None)`).
- `htui-store/src/worker.rs`: `pg_and_writer_are_worker_stores`,
  `backend_and_pg_are_worker_hosts`, and `pg_and_writer_worker_futures_spawn` (a never-called
  `fn spawnable<S: WorkerStore + Clone + 'static>(s: S) { drop(tokio::spawn(probe(s))); }`
  instantiated as `spawnable::<PgStore>` and `spawnable::<Writer>` via
  `let _: fn(PgStore) = spawnable;`).
- Test modules here use `use super::*` and so see the traits: call **no methods** on concrete
  types in them except through the generic `probe` (E0034).

**Gate** G-T3.

---

## 6. T4 — bounds onto the traits (D4, D5)

- `htui-agent/src/record.rs`: `Recorder<'a, S: htui_core::store::RecorderStore>` (`:373`), its
  `Debug` impl (`:445`), `impl` (`:472`), `pump` (`:1797`), `enforce_breach` (`:1848`). F-32 for
  the import. `htui-agent/src/conformance.rs`: the ~20 generic cases that build a `Recorder`
  (`:205`, `:247`, `:1268`, `:1350`, `:2738`, `:3167`, …) gain `+ htui_core::store::RecorderStore`,
  and each `append_events`/`set_step_usage`/`set_agent_box_quota` call on that parameter becomes
  UFCS `WriteStore::append_events(store, …)` (P-3b).
- `htui-orch/src/engine.rs`: `S: htui_core::store::WorkerStore` on `EngineParts` (`:382`), its
  `Debug` (`:430`), `Engine` (`:458`), `impl` (`:529`); every other `S: WriteStore` in the file
  (`GateContext` construction helpers, `fake_parts`'s return type needs none). `gate.rs`: every
  `S: WriteStore` (`:301`, `:381`, … `:986`, and `GateContext<'r, S, C>`). `graph.rs:298`:
  `resolve<S: htui_core::store::WorkerStore, G: GraphSource>`; `override_graph` keeps `WriteStore`.
  `graph.rs:45`'s stale `run_worker::BackendGraphs` becomes `htui_worker::HostGraphs`.
- `htui-orch/src/lib.rs:4` and `htui-orch/Cargo.toml:18-20`: "generic over `S: WorkerStore`".
- The acceptance grep: `grep -nE '\bWriteStore\b' htui-orch/src/{engine.rs,gate.rs}` shows only
  doc comments, test modules and nothing in production bounds; `graph.rs` shows `override_graph`.

**Commits**: (1) agent; (2) orch. **Tests**: none new. **Gate** G-T4 (agent, orch, htui).

---

## 7. T5 — generalise the runtime in place (D7)

Everything below lives in `htui/src/run_worker.rs` until T6 moves it.

### 7.1 Generic types

| Today | After T5 |
|---|---|
| `enum RunServed` (`:555`) | `RunServed<A>` (§2.5) + the TUI alias |
| `ChatEnd { publisher: Publisher, tag }` (`:580`) | `ChatEnd { publisher: Arc<dyn Publish>, tag }`, generic `after` (§2.5) |
| `ProgressSink { publisher, writer: Writer, author }` (`:628`) | `ProgressSink<S> { publisher: Arc<dyn Publish>, writer: S, author }`, `impl<S: htui_core::store::WorkerStore> SessionSink for ProgressSink<S>`, body `self.writer.write_document(document)` |
| `Publisher` (`:687`) | `Publisher<P: ReplySink>` (§2.5) |
| `Shared` (`:781`) | `Shared<P>`: `events: mpsc::UnboundedSender<RunServed<P::Addr>>`, `publisher: Publisher<P>`, the rest unchanged (T9 adds `role`, `backoff`, `last_executor`) |
| `registered_box(&Backend)`, `repo_map(&Backend, &Writer, …)`, `command_limits(&Backend, …)` | `<H: htui_core::store::WorkerHost>(host: &H …)`, `writer: &H::Store` |
| `WorkerEngine<'a>` (`:1188`) | `WorkerEngine<'a, H> = Engine<'a, H::Store, HostGraphs<H>, dyn Isolator, dyn Verifier, dyn Clock, FirstCandidate, ProgressSink<H::Store>>` |
| `Kit` (`:1201`) | `Kit<H>`: `writer: H::Store`, `graphs: HostGraphs<H>`, `sink: ProgressSink<H::Store>`; `Kit::read<P>(shared: &Shared<P>, host: &H, start_run)` |
| `RunRuntime` (`:1333`) | `RunRuntime<H, P> { shared: Arc<Shared<P>>, events: Option<…>, host: PhantomData<fn() -> H> }` |
| `TaskCtx` (`:1688`) | `TaskCtx<H, P> { shared, host: H, sink: P, addr: Option<P::Addr>, name, tag }`; `answer(reply: RunReply)` → `self.sink.send(addr, reply)` |
| `BackendGraphs(pub Backend)` (`:2364`) | `HostGraphs<H>(pub H)`, `impl<H: htui_core::store::WorkerHost> GraphSource for HostGraphs<H>` (bodies `self.0.name(…)`, `agent` filters `agents()`) |
| `actions(backend: &Backend, …)` (`:289`) | `actions<H: htui_core::store::WorkerHost>(host: &H, …)`; `backend.writer().is_some()` → `host.writer().is_some()` |

`RunRuntime<H, P>`'s inherent API keeps today's names **except** the two with host/sink
parameters (B-3): `new(drivers)`, `production()`, `with_parts(isolator, verifier, drivers)` (three
arguments, unchanged), `with_sweep_every`, `sweep_every`, `with_clock`, `with_author`,
`with_scratch_root`, `take_events() -> UnboundedReceiver<RunServed<P::Addr>>`, `forget_server`,
`isolator_builds`, `tasks_len`, `settle`, `shutdown`, and:

```rust
/// D158, D189, D190: one recovery sweep on a task of its own (today's `sweep`).
pub fn sweep_with(&mut self, host: &H, sink: &P);
/// §8.5: one request (today's `serve`), answered at `addr`. Awaits nothing.
pub async fn serve_request(&mut self, host: &H, sink: &P, addr: P::Addr, request: RunRequest,
    live: &LiveChats) -> RunServed<P::Addr>;
```

`serve_request`'s body is today's `serve` (`:1553-1627`) with `envelope.origin/seq` replaced by
`addr`, `StoreReply::X` by `RunReply::X`, and `StoreRequest::name` by `RunRequest::name`. The
`StoreRequest` fall-through arm moves into the TUI shim (§2.6).

### 7.2 The adapter and shims

§2.6 in full. `store_worker.rs`: `use crate::run_worker::{LiveChats, RunRuntime, RunServed, TuiRuns as _};`
(`:48`); `:1889` `RunServed::Reply(reply) => reply.into(),`; `on_run_served` (`:1484-1518`)
unchanged but for `RunServed::Reply(_)`'s type; `runtime.attach(step_id, tokio::spawn(ended.after(task)))`
compiles as is. `testkit.rs`: `TuiRuns as _` import, `:324` `.into()`, `:188`
`self.chats.push((step_id, Box::pin(ended.after(task))))` (F-11).

### 7.3 Test module (`run_worker.rs:2412-4621`)

Names and assertions unchanged. Path-level edits only:
- `use super::{…}`: `BackendGraphs` → `HostGraphs`, add `TuiRuns as _`, `RunReply`, `TuiReplies`.
- The three `TaskCtx { … }` literals (`:3777`, `:3820`, `:3869`): `backend:` → `host:`, `replies`
  → `sink: TuiReplies(replies)`.
- `runtime.shared.publisher.wire(&replies)` (`:3815`) → `.wire(&TuiReplies(replies.clone()))`;
  `.subscribe(backlog.clone(), 3, item)` (`:3816-3818`) →
  `.subscribe(ReplyAddr { seq: 3, origin: backlog.clone() }, item)`.
- `:4341`: `RunServed::Reply(RunReply::Frame(RunFrame { .. }))` (F-8).
- `:2671-2672`: the `TokioClock` doc no longer says it moves the lease fence (D3).

### 7.4 Tests

- `run_request_names_match_store_request_names`: for every `OrchRequest` of `every_orch_request()`
  (`:4504`) plus `Stream { item }` and `Actions(item)`, `RunRequest::name()` equals the matching
  `StoreRequest`'s `name()`.
- `tui_replies_map_every_run_reply`: one `RunReply` of each variant through
  `TuiReplies::send(&ReplyAddr { seq: 7, origin: Origin::App }, …)` arrives as the matching
  `StoreReply` at `seq` 7, origin `App`.
- `a_generic_runtime_future_spawns` (F-6): a never-called generic
  `fn _generic<H: WorkerHost, P: ReplySink>(runtime: &mut RunRuntime<H, P>, host: &H, sink: &P) { runtime.sweep_with(host, sink); }`
  named in a `#[test]` via `let _ = _generic::<Backend, TuiReplies>;`. It compiles only if the
  generic spawn in `spawn_supervised` is `Send`.
- Pins unchanged: `StoreRequest` 85, `StoreReply` 47.

### 7.5 Commits

(1) the new types and their two tests: `ReplySink`, `RunRequest`, `RunReply`, `TuiReplies`,
`From`, green on arrival (new code with no prior behaviour). (2) The generic runtime, the aliases,
the shims and the call sites, with the test-module edits (§7.3); the whole existing suite is the
proof.

**Gate** G-T5, then `cargo test -p htui --all-features run_worker -- --test-threads=1` alone.

---

## 8. T6 — move into `htui-worker` (D6, D8)

### 8.1 The crate

`Cargo.toml` (root): members += `"crates/htui-worker"`; `[workspace.dependencies] htui-worker = { path = "crates/htui-worker" }`.

```toml
[package]
name        = "htui-worker"
version     = "0.1.0"
description = "Run supervision without a UI: the runtime `htui` and `htui worker` share (MOD-41)"
edition.workspace = true   # and rust-version, license, publish, as the other crates

[features]
default = []
# `htui_worker::testing`: the run runtime's white-box surface, for `htui`'s run_worker tests (D8).
test-support = []

[dependencies]
htui-core  = { workspace = true }
htui-agent = { workspace = true }
htui-orch  = { workspace = true }
# PgStore (the worker's host), identity::config_root, DATABASE_UNREACHABLE, HTUI_VERSION, BOX_HEARTBEAT.
htui-store = { workspace = true }
chrono     = { workspace = true }
serde_json = { workspace = true }
tokio      = { workspace = true, features = ["sync", "rt", "time", "macros"] }
tokio-util = { workspace = true }
tracing    = { workspace = true }
uuid       = { workspace = true }
# Never ratatui or crossterm: tests/deps.rs.

[dev-dependencies]
htui-core  = { workspace = true, features = ["test-support"] }
htui-orch  = { workspace = true, features = ["test-support"] }
htui-agent = { workspace = true, features = ["test-support"] }
htui-store = { workspace = true, features = ["demo", "test-support"] }
tokio      = { workspace = true, features = ["test-util", "rt-multi-thread"] }

[lints]
workspace = true
```

Files: `src/lib.rs` (crate doc from `run_worker.rs:1-19`, modules, `pub use`), `src/views.rs`
(`ORCH_NAMES` … `newest_output`, `StepAuthor`, `ProgressSink`, `RefusedDriver`,
`run_worker.rs:62-545` and `:607-685`), `src/address.rs` (§2.5), `src/graphs.rs` (`HostGraphs`),
`src/runtime.rs` (the rest of `:546-2361`, plus `testing`). `htui/Cargo.toml`:
`htui-worker = { workspace = true }`, and in `[dev-dependencies]`
`htui-worker = { workspace = true, features = ["test-support"] }`.

### 8.2 `htui/src/run_worker.rs` after T6

The module doc names the adapter's job, then §2.6 (adapter, aliases, `TuiRuns`, re-exports), then
the unchanged test module with the §8.3 rewrites.

### 8.3 `htui_worker::testing` (in `runtime.rs`, `#[cfg(feature = "test-support")] #[doc(hidden)] pub mod testing`, re-exported from `lib.rs`)

```rust
/// A handle on a runtime's shared state, for the TUI's white-box tests (MOD-41 plan D8).
pub struct Probe<P: ReplySink>(Arc<Shared<P>>);          // hand-written Debug
pub fn probe<H: WorkerHost, P: ReplySink>(runtime: &RunRuntime<H, P>) -> Probe<P>;
impl<P: ReplySink> Probe<P> {
    pub fn try_lock(&self, run: RunId) -> Option<OwnedMutexGuard<()>>;   // shared.locks.try_lock
    pub fn has_lock_entry(&self, run: RunId) -> bool;                    // shared.locks.0 map
    pub fn has_parent(&self, run: RunId) -> bool;                        // shared.walks.lock()
    pub fn walk_child(&self, run: RunId) -> WalkToken;                   // shared.walks.child
    pub fn is_dead_walk(&self, run: RunId) -> bool;                      // shared.dead_walks.contains
    pub fn queue(&self, queued_at: DateTime<Utc>, run: RunId);           // shared.queue
    pub fn queued(&self) -> Vec<(DateTime<Utc>, RunId)>;                 // shared.queued
    pub fn all_tasks_finished(&self) -> bool;                            // shared.tasks
    pub fn wire(&self, sink: &P);                                        // shared.publisher.wire
    pub fn subscribe(&self, addr: P::Addr, item: ItemId);                // shared.publisher.subscribe
    pub fn task_ctx<H: WorkerHost>(&self, host: H, sink: P, name: &'static str) -> TaskCtx<H, P>;
}
pub use super::{TaskCtx, WalkToken};                     // hand-written Debug on TaskCtx (F-12)
pub fn set_task_run<H: WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>, run: RunId);
pub fn unaddressed<H: WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>, name: &'static str) -> TaskCtx<H, P>;
pub fn spawn_supervised<H: WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, work: impl Future<Output = ()> + Send + 'static);
pub async fn resumed<H: WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>, run: RunId);
pub async fn retry_claims<H: WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>);
pub async fn command_limits<H: WorkerHost>(host: &H, box_id: BoxId) -> StoreResult<BTreeMap<String, u32>>;
```

Test-module rewrites (names and assertions unchanged): `super::command_limits` (`:3416`, `:3434`,
`:3440`) → `testing::command_limits`; `runtime.shared.locks.try_lock` (`:3556`) →
`testing::probe(&runtime).try_lock`; `.dead_walks.contains` (`:3571`, `:3831`, and `shared.`
at `:4391`-`:4437`) → `is_dead_walk`; `.walks.lock().contains_key` (`:3717`, `:3759`) →
`has_parent`; `.locks.0.lock()…contains_key` (`:3722-3733`, `:3763`) → `has_lock_entry`;
`.walks.child` (`:3736`) → `walk_child`; `.tasks…is_finished` (`:3741-3748`) →
`all_tasks_finished`; the three `TaskCtx` literals → `probe.task_ctx(host, TuiReplies(replies), name)`;
`super::spawn_supervised(ctx.unaddressed(..), ..)` (`:3786`) →
`testing::spawn_supervised(testing::unaddressed(&ctx, ..), ..)`; `super::resumed` (`:3829`),
`super::retry_claims` (`:3881`) → `testing::`; `ctx.tag.run.set(ended)` (`:3876`) →
`testing::set_task_run`; `shared.queue`/`shared.queued` (`:3879`, `:3885`, `:3954`, `:4018`,
`:4042`) → `probe.queue`/`probe.queued()`; `let shared = Arc::clone(&runtime.shared)` (`:3929`,
`:4391`) → `let shared = testing::probe(&runtime)`; `BackendGraphs(…)` (`:3467`, `:4445`) →
`HostGraphs(…)`. `store_worker.rs:3431-3433` and `testkit.rs:710-761` keep importing the fixture.

### 8.4 Tests

- `htui-worker/tests/deps.rs::the_library_links_no_terminal_crate`:
  `Command::new(env!("CARGO")).args(["tree", "-p", "htui-worker", "-e", "normal", "--prefix", "none", "--all-features", "--offline"]).current_dir(env!("CARGO_MANIFEST_DIR"))`;
  assert success, that no line starts with `ratatui` or `crossterm`, and that some line starts
  with `htui-orch` (the tree was really read).
- `runtime.rs` tests (over `Backend::memory(MemStore::demo())`, never importing `WorkerHost`,
  F-33): `publisher_sends_each_frame_to_its_items_subscribers_only` over a `Recording` sink
  (`Arc<Mutex<Vec<(u64, RunReply)>>>`, `Addr = u64`, `Subscriber = u64`): two subscribers on item A
  and one on B; a frame for A reaches exactly the two A addresses. Plus
  `a_later_subscription_replaces_the_earlier_one`.

### 8.5 Commits

(1) crate skeleton (empty `lib.rs` with its doc) and `deps.rs`, green as a guard from day one.
(2) The move, re-exports, `testing`, the test-module rewrites and the `Publisher` tests.

**Gate** G-T6.

---

## 9. T7 — executor setting and the queued read (D9, D10, D11)

### 9.1 Model

§2.3. `model/mod.rs:105` re-exports `Executor` (F-14). The 18 `BoxEdit { … }` literals in 9 files
gain `executor: None` (D10's list; re-grep `BoxEdit {` and skip the two `-> BoxEdit {` returns).

### 9.2 MemStore `edit_box` (`htui-core/src/store/mem.rs:1815-1850`)

Order: `NotFound` → `Stale` → tag `Constraint` → `Constraint("executor must be tui or worker")`
for `Some(Other(_))` → `Constraint("box.settings is not a JSON object")` when
`edit.executor.is_some() && !row.settings.is_object()` → writes, where the executor write is
`row.settings.as_object_mut().expect("checked above").insert(Executor::KEY.to_owned(), Value::String(executor.as_str().to_owned()));`.
Both sentences are `pub const` in `traits.rs` beside `canonical_declared_tags`' sentence helpers
(`EXECUTOR_MUST_BE_KNOWN`, `BOX_SETTINGS_NOT_AN_OBJECT`) so the stores cannot drift.

### 9.3 Postgres `edit_box` (`htui-store/src/pg/write.rs:1461-1519`, B-5)

```rust
let executor = edit.executor.as_ref();
let refused = match (tags_result, executor) {                // tags_result = today's canonicalisation
    (Err(sentence), _) => Some(sentence),
    (Ok(_), Some(Executor::Other(_))) => Some(EXECUTOR_MUST_BE_KNOWN.to_owned()),
    _ => None,
};
if let Some(sentence) = refused { /* today's `:1476-1483` read: NotFound, Stale, Constraint(sentence) */ }
let written = sqlx::query_as!(BoxRow, r#"
    UPDATE box
       SET declared_tags = COALESCE($3, declared_tags),
           quirks        = COALESCE($4, quirks),
           settings      = CASE WHEN $6::text IS NULL THEN settings
                                ELSE jsonb_set(settings, '{executor}', to_jsonb($6::text)) END,
           edit_version  = edit_version + 1
     WHERE id = $1 AND user_id = $5 AND edit_version = $2
       AND ($6::text IS NULL OR jsonb_typeof(settings) = 'object')
    RETURNING … (today's list)
    "#, id.as_uuid(), expected, tags.as_deref(), edit.quirks, me.as_uuid(),
    executor.map(Executor::as_str))
    .fetch_optional(&self.pool).await.map_err(map_sqlx)?;
match written {
    Some(row) => Ok(CasOutcome::Applied(row)),
    None => {
        let current = self.box_row(id).await?.filter(|row| row.user_id == me);
        match current {
            Some(row) if row.edit_version == expected && executor.is_some()
                && !row.settings.is_object() =>
                Err(StoreError::Constraint(BOX_SETTINGS_NOT_AN_OBJECT.to_owned())),
            other => cas_miss(other, "box", id),
        }
    }
}
```

The trait doc (`traits.rs:448-470`) drops "has **no** writer at all today" and states: `edit_box`
is `box.settings`' only writer, key by key (`executor` only), under the same `edit_version` CAS;
every other key survives; precedence `NotFound`, `Stale`, `Constraint` (tags, executor, blob).
`0003_orchestration.sql:116-118` is **not** touched (DDL, plan D10).

### 9.4 `queued_runs_on_box` (D11)

```rust
// pg/read.rs, after active_runs_on_box (:1974-1987)
/// This box's `queued` runs, `(id, queued_at)` by `(queued_at, id)` (MOD-41 plan D11). No index
/// covers `(target_box_id, status)`; a per-user table scanned every 5 s is accepted (R-6).
pub async fn queued_runs_on_box(&self, box_id: BoxId) -> Result<Vec<(RunId, DateTime<Utc>)>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id: RunId", queued_at
             FROM run
            WHERE target_box_id = $1 AND status = 'queued'
            ORDER BY queued_at, id"#,
        box_id.as_uuid(),
    )
    .fetch_all(&self.pool).await.map_err(map_sqlx)?;
    Ok(rows.into_iter().map(|row| (row.id, row.queued_at)).collect())
}
// mem.rs, after active_runs_on_box (:742): same doc; filter target_box_id == box_id &&
// status == Queued, sort_by_key(|(id, at)| (*at, *id)) (RunId's Ord is uuid byte order, which is
// Postgres' uuid order).
// backend.rs, after active_runs_on_box (:569): Memory/Online arms, Offline => Err(orchestration_offline()).
```

`WorkerHost::queued_runs_on_box` (§2.1) plus the `PgStore`/`Backend` impls in
`htui-store/src/worker.rs`: `PgStore::queued_runs_on_box(self, box_id).await`,
`Backend::queued_runs_on_box(self, box_id).await`. `.sqlx` **290**.

### 9.5 Commits and tests

1. **red**: §2.3 (the type, `BoxSettings.executor`, `BoxEdit.executor`, the 18 literals), stores
   that **ignore** `edit.executor`, the tests below.
2. **green**: §9.2-§9.4, `.sqlx` regenerated.

| Test | Assertion |
|---|---|
| store conformance `edit_box_writes_the_executor_and_keeps_every_other_setting` | On the demo box (`boxes()` reads its blob, `{"max_concurrent_items": 2}`): write `Some(Worker)` → `Applied`, `executor == "worker"`, `max_concurrent_items` still 2, `edit_version` +1; write `Some(Tui)` with the new token → `"tui"`, still 2. |
| store conformance `an_unknown_executor_is_refused_and_writes_nothing` | `Some(Other("container"))`: `Constraint(EXECUTOR_MUST_BE_KNOWN)`, row unchanged (`boxes()` before = after); with a spent token as well: `Stale` first. |
| store conformance `an_executor_edit_is_a_compare_and_set` (F-37) | Two edits from one token: the first `Applied`, the second `Stale` carrying the first's row, and the blob holds the first's executor only. |
| `pg_criteria.rs::edit_box_keeps_every_other_settings_key` + MemStore twin | Seed `{"max_concurrent_items": 1, "command_limits": {"verify": 2}, "x": true}` (raw `UPDATE`; MemStore `from_demo` over edited data), write `Worker`: all four keys present. |
| `pg_criteria.rs::edit_box_refuses_a_non_object_settings_blob` + MemStore twin | Blob `[]`, then `"x"`: `Some(Tui)` is `Constraint(BOX_SETTINGS_NOT_AN_OBJECT)`, nothing written, `edit_version` unchanged; a quirks-only edit on the same row is `Applied`. |
| `box_.rs` `an_unknown_executor_keeps_the_admission_limit` | `{"executor": 7, "max_concurrent_items": 1}` and `{"executor": "container", …}` decode to `max_concurrent_items == Some(1)`, `executor` `Other("7")`/`Other("container")`. |
| `box_.rs` `a_missing_executor_is_tui` | `{}` and `{"max_concurrent_items": 1}`: `Executor::of == Tui`, struct field `Tui`. |
| `box_.rs` `a_bad_sibling_key_still_reads_the_worker_executor` | `{"executor": "worker", "max_concurrent_items": "2"}`: `Executor::of == Worker` (the struct decode falls back to default). |
| `box_.rs` `a_non_object_blob_is_other` | `[]`, `"x"`, `null`: `Executor::of` is `Other(_)`. |
| `box_.rs` `executor_round_trips_as_its_text` (added) | `Serialize` of each variant is the bare string; `Other("container")` survives decode → encode. |
| `pg_criteria.rs::claim_run_honours_the_limit_beside_an_unknown_executor` | `{"executor": 7, "max_concurrent_items": 1}`: a second claim on the box is `SlotFull`. |
| `pg_criteria.rs::queued_runs_on_box_lists_this_boxs_queued_runs_in_queue_order` | Three queued runs on box A (two sharing `queued_at`), one on box B, one running on A: exactly A's three, by `(queued_at, id)`. MemStore twin in `mem.rs` tests. |

Pins: store `CASES` 100 → **103**.

**Gate** G-T7.

---

## 10. T8 — Settings > Boxes executor editor (D10)

- `boxes.rs` `Mode` (`:110-117`) gains
  `/// The executor flip, awaiting y/n (MOD-41 plan D10). Executor(ExecutorFlip),` with
  `#[derive(Debug)] struct ExecutorFlip { box_id: BoxId, expected: i32, from: Executor, to: Executor }`.
- `w` in `Browse` (beside `t`/`e`/`p`, `:456-466`) opens it for the selected row:
  `to = match from { Tui => Worker, Worker | Other(_) => Tui }` (F-35).
- `y` submits `StoreRequest::EditBox { box_id, expected, edit: BoxEdit { executor: Some(to), ..BoxEdit::default() } }`,
  through `submit`'s busy/notice rules (`:368-415`); `n`/`Esc` returns to `Browse`. The stale arm
  (`:231-241`) gains `(Some(token), Mode::Executor(flip))` → `flip.expected = token` with
  today's stale sentence.
- The row line shows `executor: tui|worker|<text>` from `Executor::of(&record.row.settings)`.
- `HINT_BROWSE` (`:49`) → `j/k move · t tags · e quirks · w executor · p probe this box · r reload`;
  `HINT_EXECUTOR` = `y write · n/esc cancel`; the confirmation line:
  ``executor of `<hostname>`: `tui` → `worker`? The TUI stops walking runs here; `htui worker` must run on this box.``
  (for `→ tui`: ``… The TUI walks this box's runs again.``).
- `box_settings.rs`: an `executor_label(&BoxRecord) -> String` helper if the render wants one.

**Tests** (`htui/tests/box_settings.rs`): `the_boxes_section_shows_each_box_s_executor` (two boxes,
one `worker` planted in the demo data, both lines rendered);
`w_flips_the_executor_after_confirmation` (`w`, then `y`, sends
`EditBox { executor: Some(Worker), expected: <row's edit_version>, .. }`; `n` sends nothing);
`a_stale_executor_edit_says_so` (a `BoxesStale` reply keeps the flip open, with the stale notice).
New snapshot `box_settings__executor_confirm`. Re-accept five (demo, two_boxes, tag_editor, stale,
quirks_editor); the footer changes only in demo and two_boxes; `offline` unchanged. Review each
diff.

**Commits**: (1) red: tests and the new snapshot pending; (2) green. **Gate** G-T8.

---

## 11. T9 — executor gate and hand-back (I-1, D12, D13, OQ-1, OQ-4, OQ-5, OQ-6)

### 11.1 Engine switch (`htui-orch/src/engine.rs`)

§2.4, plus:

```rust
/// Plan D12: the command's window is written; the lease goes back, and the answer is the run's
/// rest as read now (the box's worker adopts it at its next poll).
async fn hand_back(&self, run: RunId) -> Result<Rest, EngineError> {
    self.release_lease(run).await;
    self.resting(&self.run(run).await?).await
}
```

Cut points: each inserts `if self.parts.tails == Tails::HandBack { … }` right after the window's
`.await?;` and before `let tail = async {`:

| Command | Window ends | Insert (before) | HandBack body |
|---|---|---|---|
| `a`, `x` (`answer_guarded`, `:794-857`) and `A` (`accept_artifact` → `answer_guarded` at `:1459`) | `:840` | `:844` | `return self.hand_back(run.id).await;` The adopter finishes an approval by the frontier reconcile and a rejection by D131's `settle_failed` → `rejection`. |
| `r` single (`retry_guarded`, `:964-1031`) | `:1016` | `:1021` | `let admitted = self.leased_window(run.id, async { let run = self.run(run.id).await?; let steps = self.parts.store.run_steps(run.id).await?; let attempt = next_attempt(&steps, row.position); self.admit(&run, snapshot, phase, attempt).await }).await?; let handed = self.hand_back(run.id).await?; return Ok(CommandOutcome::Retried { step: row.id, rest: admitted.unwrap_or(handed) });` |
| `r` group (`retry_group_guarded`, `:1036-1066`) | `:1055` | `:1057` | as above with `self.admit(&run, snapshot, phase, attempt + 1)` |
| `s` (`select_fanout`, `:1075-1132`) | `:1120` | `:1122` | `return Ok(CommandOutcome::Selected { rest: self.hand_back(run.id).await? });` The adopter's frontier reconcile passes the same siblings (`group_at(&steps, position, attempt)` minus the winner, `:2149`). |
| `u` Resume (`unblock`, `:1531-1537`) | — | replaces `:1534-1536` | `Some(match self.parts.tails { Tails::Walk => match self.resume(run).await? { Resume::Walked(rest) \| Resume::TopologyChanged { rest, .. } => rest }, Tails::HandBack => self.hand_back_resume(run).await? })` |

```rust
/// Plan D12's `u` on a worker box: the lease, then only `walk_resumed_from`'s unpark
/// (`:2791-2800`, MOD-4 plan D180's compare-and-set), then the lease back. The adopter's
/// `resume` runs `resume_window` once; running it here too would resolve twice (plan D12).
async fn hand_back_resume(&self, run: RunId) -> Result<Rest, EngineError> {
    self.take_lease(run).await?;
    self.leased_window(run, async {
        let row = self.run(run).await?;
        if row.status == RunStatus::AwaitingApproval {
            let snapshot = Self::snapshot_of(&row)?;
            let steps = self.parts.store.run_steps(run).await?;
            if resumable_park(&cursor(&snapshot, &steps)) && !self.unpark(&row, self.now()).await? {
                return Err(stale_run(run, RunStatus::AwaitingApproval, RunStatus::Running));
            }
        }
        Ok(())
    })
    .await?;
    self.hand_back(run).await
}
```

`resume` itself is **not** switched: the adopter (always `Walk`) calls it. `start_run` (engine)
is not switched either; the runtime's `R` path is (§11.5). `p`, `C`, `T`, `o` are unchanged.

### 11.2 Queued cancel (OQ-4, B-6, every box)

`cancel_run` (`:1603-1648`) becomes:

```rust
pub async fn cancel_run(&self, run: RunId) -> Result<CommandOutcome, EngineError> {
    let row = self.run(run).await?;
    crate::command::cancel_enabled(&row)?;
    if row.status == RunStatus::Queued {
        if let Some(outcome) = self.cancel_queued(&row).await? {
            return Ok(outcome);
        }
        // OQ-4: a claim won between the read and the compare-and-set. The run is leased now,
        // and the leased path refuses a live lease with `LeaseHeld`, writing nothing.
        let row = self.run(run).await?;
        crate::command::cancel_enabled(&row)?;
        return self.cancel_leased(&row).await;
    }
    self.cancel_leased(&row).await
}

/// OQ-4, B-6: `queued → cancelled` as a compare-and-set, then the item `queued → open` that
/// `finish_run` would have mirrored, then plan D36's cleanup. `None` when the run left `queued`
/// first. A crash between the two writes leaves the item `queued` (E-2).
async fn cancel_queued(&self, row: &Run) -> Result<Option<CommandOutcome>, EngineError> {
    let position = self.resting(row).await?.position;
    let now = self.now();
    if !self.parts.store.transition_run(row.id, RunStatus::Queued, RunStatus::Cancelled, now).await? {
        return Ok(None);
    }
    if let Some(item) = row.item_id {
        self.parts.store.transition(item, Status::Queued, Status::Open).await?;
    }
    self.cleanup_run(row.id).await?;
    Ok(Some(CommandOutcome::Cancelled { rest: Rest { run: RunStatus::Cancelled, position, failure: None } }))
}
```

`cancel_leased(&self, row: &Run)` is today's body from `:1606` with `leased` always true (a
`running` or parked run): `take_lease`, the window (steps → `cancelled`, `finish_run`,
`cleanup_run`), `release_lease`.

### 11.3 Harness (`fake.rs`, `conformance.rs`, `engine.rs`, `gix_isolator.rs`)

`FakeOrchestrator` gains `tails: Mutex<Tails>` (`Walk` in `demo` and in `restarted`), `pub fn
tails(&self) -> Tails`, `pub fn set_tails(&self, tails: Tails)`. `Orchestrate` gains
`fn set_tails(&self, tails: Tails);` (implemented in `conformance.rs`'s `impl Orchestrate for
FakeOrchestrator`). `fake_parts` sets `tails: orch.tails()`; every other literal (F-22) sets
`tails: Tails::Walk`.

### 11.4 Runtime gate (`htui-worker/src/runtime.rs`)

```rust
/// Which process a runtime is (I-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// The TUI's runtime.
    #[default]
    Tui,
    /// `htui worker`'s.
    Worker,
}

impl Role {
    /// I-1: whether this role claims, adopts and sweeps on a box whose executor is `executor`.
    #[must_use]
    pub fn executes(self, executor: &Executor) -> bool {
        matches!((self, executor), (Self::Tui, Executor::Tui) | (Self::Worker, Executor::Worker))
    }
    /// Plan D12: the TUI hands back on a worker box; everything else walks.
    #[must_use]
    pub fn tails(self, executor: &Executor) -> Tails {
        if self == Self::Tui && *executor == Executor::Worker { Tails::HandBack } else { Tails::Walk }
    }
}

/// Plan D9: the refusal on a box whose executor this build does not know.
#[must_use]
pub fn unknown_executor(executor: &Executor) -> String {
    format!("box executor `{executor}` is not known to htui {}", htui_store::pg::HTUI_VERSION)
}

/// OQ-4: `c` on a run the box's worker is walking.
#[must_use]
pub fn worker_walks(run: RunId) -> String {
    format!("the worker on this box is walking run {run}; cancelling a live run needs MOD-42's cancel command")
}
```

(Use the re-export path of `HTUI_VERSION` that exists; it is `htui-store/src/pg/mod.rs:56`.)

`Shared<P>` gains `role: Role`, `last_executor: StdMutex<Option<Executor>>`, and
`backoff: StdMutex<HashMap<RunId, Backoff>>`. `RunRuntime::with_role(self, role) -> Self`
configures `role` before serving (same `configure()` rule as `with_clock`).

`Kit<H>` gains `role`, `executor: Executor`, `tails: Tails`. `Kit::read` adds, after
`registered_box`:

```rust
let executor = host.box_row(box_id).await.map_err(sentence)?
    .map(|row| Executor::of(&row.settings))
    .ok_or_else(|| "this box has no row".to_owned())?;
let tails = shared.role.tails(&executor);
```

`Kit::engine` sets `tails: self.tails`. `Kit::refusal(&self, command: &Command) -> Option<String>`:
`Some(unknown_executor(..))` when `role == Tui`, the executor is `Other(_)` and the command is
`StartRun | AnswerGate | RetryStep | SelectFanout | AcceptArtifact`.

`sweep_once` (`:1955-2016`), B-8:

```rust
async fn sweep_once<H: WorkerHost, P: ReplySink>(ctx: TaskCtx<H, P>) {
    // today's sweep_every update (:1957-1963)
    let Ok(box_id) = registered_box(&ctx.host).await else { return };
    // I-1 (plan D12, D13): only the process whose role matches the box's executor adopts or claims.
    let executor = match ctx.host.box_row(box_id).await {
        Ok(Some(row)) => Executor::of(&row.settings),
        Ok(None) => return,
        Err(err) => { tracing::debug!(%err, "the sweep could not read this box's executor"); return; }
    };
    ctx.shared.note_executor(&executor);           // logs once per change; info for Worker, debug for Tui
    if !ctx.shared.role.executes(&executor) {
        return;
    }
    if !ctx.shared.dead_walks.runs().is_empty()
        || !matches!(ctx.host.active_runs_on_box(box_id).await, Ok(0))
    {
        adopt(&ctx).await;                          // today's :1971-2015, unchanged
    }
    claim_scan(&ctx, box_id).await;
}

/// Plan D14, OQ-5: this box's queued rows join the claim queue in `(queued_at, id)` order and are
/// claimed through `claim_queued`, each under its run lock. A run this process is already working
/// on (its own `StartRun` between `enqueue` and the lock) is skipped (B-9); what is left of that
/// race is a wrong refusal sentence on a TUI box, never a second claim.
async fn claim_scan<H: WorkerHost, P: ReplySink>(ctx: &TaskCtx<H, P>, box_id: BoxId) {
    let queued = match ctx.host.queued_runs_on_box(box_id).await {
        Ok(queued) => queued,
        Err(err) => { tracing::debug!(%err, "the claim scan could not list queued runs"); return; }
    };
    let mut fed = false;
    for (run, queued_at) in queued {
        if ctx.shared.walks.is_live(run) { continue; }
        ctx.shared.queue(queued_at, run);
        fed = true;
    }
    if fed { claim_queued(ctx).await; }
}
```

`reclaim` (`:1919-1953`), B-7: after `Kit::read`, `if !kit.role.executes(&kit.executor) {
tracing::debug!(%run, "not this process's to claim on this box (I-1)"); return; }` (the run leaves
the in-memory queue; the executing process's scan finds it).

### 11.5 TUI command paths (runtime, role `Tui`)

- `start_run` (`:2118-2180`): `if let Some(refusal) = kit.refusal(&StartRun{..}) { return ctx.refuse(refusal) }`
  before `enqueue`. After `enqueue` answers: `let walk = ctx.shared.walks.child(run);` first (B-9;
  moved from `:2150`), then tag and publish `Started`. Then
  `if kit.tails == Tails::HandBack { return ctx.done(CommandOutcome::Started { run, rest: Rest { run: RunStatus::Queued, position: None, failure: None } }); }`
  (no claim, nothing queued in memory). Otherwise today's code.
- `on_run` (`:2182-2280`): after `Kit::read`, `if let Some(refusal) = kit.refusal(&command) { return ctx.refuse(refusal) }`;
  `let cancelling = matches!(command, Command::CancelRun { .. });` before dispatch; one new arm
  before `Some(Err(err))`:
  `Some(Err(EngineError::LeaseHeld { run: held })) if cancelling && kit.tails == Tails::HandBack => ctx.refuse(worker_walks(held)),`.
- `unblock` (`:2283-2322`): after the case, `if matches!(case, UnblockCase::Resume(_)) && kit.role == Role::Tui && matches!(kit.executor, Executor::Other(_)) { return ctx.refuse(unknown_executor(&kit.executor)) }`.
  The engine's `Tails` does the rest.
- The TUI's `sweep` is the gated `sweep_once` above: on a worker box it neither adopts nor claims
  (D13 `tui → worker`: in-flight walks keep their leases, heartbeat and rest normally).

### 11.6 Per-run backoff (OQ-6, B-10; role `Worker` only)

```rust
const BACKOFF_FIRST: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
struct Backoff { due: tokio::time::Instant, delay: Duration }

/// OQ-6: a resume that failed to resolve the live graph for a reason other than the store
/// (`NoGraph`, `NoTemplate`, `NoAgentRow`, `EmptyScopeWithPrimary`, …; the handled list is
/// `engine.rs:2708-2713`) is retried later, not at every poll.
fn backs_off(err: &EngineError) -> bool {
    matches!(err, EngineError::Resolve(resolve) if !matches!(resolve, ResolveError::Store(_)))
}
```

`Shared::step_backoff(run)`: `delay = previous.map_or(BACKOFF_FIRST, |b| (b.delay * 2).min(BACKOFF_MAX))`;
`warn!(%run, delay_secs, "a run's resume keeps failing; the worker waits before resuming it again")`
only when the delay changed; `due = Instant::now() + delay`. `clear_backoff(run)`.
`backoff_due(run) -> Option<Instant>` answers only for `role == Worker`.

`resumed` (`:2018-2046`): after `let walk = ctx.shared.walks.child(run);`:

```rust
if let Some(due) = ctx.shared.backoff_due(run) {
    tokio::select! {
        biased;
        () = walk.token.cancelled() => {
            // Shutdown while waiting: the adopted lease is this owner's; give it back.
            if let Some(store) = ctx.host.writer()
                && let Err(err) = store.release_lease(run, ctx.shared.owner).await
            {
                tracing::warn!(%run, %err, "releasing a backed-off run's lease failed; it lapses");
            }
            return;
        }
        () = tokio::time::sleep_until(due) => {}
    }
}
```

and on the resume's answer: `Some(Ok(..))` → `clear_backoff(run)`; `Some(Err(err))` →
`if ctx.shared.role == Role::Worker && backs_off(&err) { ctx.shared.step_backoff(run) }`, then
today's refusal.

### 11.7 Commits

1. **red**: every test in §11.8 (orch cases through `set_tails`, which lands here as a knob that
   `fake_parts` does not read yet; engine white-box tests against `cancel_queued`, landing here
   as today's queued branch factored out, unchanged; runtime and library tests against `Role`,
   `with_role` and `unknown_executor`, which land here as types with no effect yet).
2. **green (engine)**: §2.4, §11.1, §11.2, §11.3, `lib.rs` re-export (F-14).
3. **green (runtime)**: §11.4-§11.6.

### 11.8 Tests

Orch conformance (each: a control run in-process on one `harness.fresh()` and the hand-back run
on another; A walks to the park with `Walk`, then `a.set_tails(Tails::HandBack)` and answers; the
answer's rest is `running` and A's lease is released; `let b = a.restarted(); b.sweep()` adopts
it (`Next::Walk`), then `b.resume(run)` → `Resume::Walked(rest)`. Assert `(rest.run,
rest.position, rest.failure)` and the step list `[(position, attempt, fanout_index, phase_name,
status)]` equal the control's):

1. `an_approval_handed_back_is_walked_by_the_adopter`
2. `a_rejection_handed_back_is_walked_by_the_adopter`: loopable (review loop resumes) and terminal (run `failed`, `Rejected`), both in one case
3. `a_selection_handed_back_is_walked_by_the_adopter`
4. `a_retry_of_a_failed_step_handed_back_is_walked_by_the_adopter`
5. `a_resume_handed_back_is_walked_by_the_adopter` (item `awaiting_approval` over a resumable park)
6. `an_accept_handed_back_is_walked_by_the_adopter` (a promoted step; the verify runs in A)
7. `a_retry_of_an_awaiting_step_handed_back_is_walked_by_the_adopter`: plus the previous winner's reconcile count is 1 (FakeIsolator's recorded merges)
8. `a_group_retry_handed_back_is_walked_by_the_adopter`: same extra assertion
9. `a_retry_past_the_budget_handed_back_is_walked_not_reparked`: rest equals the control's (a new attempt walks), never `AwaitingApproval` with `Interrupted`
10. `a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note`: after A's hand-back, add a phase to the item's live graph; B → `Resume::TopologyChanged`, run and item `awaiting_approval`, one `topology mismatch` note (F-24: the frontier merge happened first)
11. `a_hand_back_writes_no_step_past_the_window`: after A's answer, the steps are exactly the ones before, with the answered step moved; no `pending`/`running` row at a later position; the run is `running`, `lease_expires_at` ≤ A's now, and `refresh_lease(run, a.owner(), …)` answers `false` (`Run` has no owner field, `model/run.rs:203-206`; the existing restart case asserts the same way)

Engine white-box (`engine.rs` tests, not `CASES`):
- `a_queued_cancel_loses_cleanly_to_a_concurrent_claim` (F-21): enqueue; read the row (stale);
  a second owner `claim_run`s it with a one-day TTL; `cancel_queued(&stale)` is `Ok(None)` and
  writes nothing; `dispatch(CancelRun)` is `LeaseHeld`; the run is still `running` under the
  second owner and the item `in_progress`.
- `a_queued_cancel_moves_the_item_back_to_open` (B-6): enqueue then cancel: run `cancelled` with
  `finished_at`, item `open`, and a second `R` on the item succeeds.

Runtime (`htui/src/run_worker.rs` tests, over the TUI loop; executor planted with
`fixture.store.edit_box(.., BoxEdit { executor: Some(Worker), .. })`, or in demo data for `Other`):
- `on_a_worker_box_start_run_only_queues`: `R` answers `Done(Started { rest.run: Queued })`; the
  run is `queued`, `lease_expires_at` is `None`, no step; still so after `settle`.
- `on_a_worker_box_an_answer_hands_back` (added): `a` on a parked run answers `Answered` with
  `rest.run == Running`; lease released; no step past the gate.
- `on_a_worker_box_the_tui_never_sweeps`: a `stranded` run (lapsed foreign lease, `:3461-3500`);
  `runtime.sweep` + `settle`: still `running`, not this owner's, no step.
- `on_a_worker_box_a_live_walk_refuses_cancel_naming_the_worker`: a foreign one-day lease on a
  `running` run; `c` → `Failed { message: worker_walks(run) }`; nothing written.
- `an_unknown_executor_refuses_start_and_walking_commands`: `"executor": "container"` in demo
  data; `R` and `a` → `unknown_executor` sentence, nothing written; `c` on a parked run cancels.
- `a_tui_box_claims_a_queued_row_it_did_not_queue` (OQ-5): a queued row created by another
  engine (another owner); one TUI sweep; the run leaves `queued` under this owner and walks.
- `flipping_to_worker_keeps_the_tuis_live_walk`: a TUI walk stalls; flip to `worker`; the walk
  keeps heartbeating and rests normally when released; a sweep in between adopts nothing.
- `flipping_to_worker_stops_the_in_memory_claim_retry` (added, F-17): a refused claim waits in the
  in-memory queue; flip to `worker`; a walk rests; the queued run stays `queued`.
- `a_worker_box_with_a_bad_sibling_setting_is_still_a_worker_box`:
  `{"executor": "worker", "max_concurrent_items": "2"}`: `R` only queues.

Library (`htui-worker/src/runtime.rs`, `Backend::memory(MemStore::demo())`, role `Worker`,
`Recording` sink):
- `a_worker_role_idles_on_a_tui_box`: executor default; a queued run stays queued; a stranded run
  is not adopted.
- `an_unknown_executor_idles_the_worker` (added): `"container"`, same assertions.
- `a_worker_role_claims_queued_rows_in_order`: two queued runs with overlapping scopes; the earlier
  `queued_at` is claimed first, and the later one is claimed after the first rests.
- `a_run_whose_resume_keeps_failing_backs_off` (`start_paused`): a stranded run whose live graph
  no longer resolves (`NoGraph`); poll `sweep_with` every 1 s for 60 s of paused time; the
  resolve-refusal frames published for the item arrive at paused 0, 5, 15 and 35 s: 4, not 60.

Pins: orch `CASES` 74 → **85** (`conformance.rs:5889`, `fake_conformance.rs:16`).

**Gate** G-T9.

---

## 12. T10 — Runs poll (D16, OQ-3)

- `htui/src/ui/tabs/registry.rs` `Tab` (`:35-64`), after `on_external_edit`:

  ```rust
  /// The shell's once-a-second refresh reached the active tab (MOD-41 plan D16). Defaulted, the
  /// trait's third default after `focus_section` and `on_external_edit`, so no other tab changes.
  /// A view trait, not a store trait: the no-default rule does not apply.
  fn on_refresh(&mut self, _ctx: &mut Ctx<'_>) {}
  ```

- `htui/src/app/update.rs` `on_tick` (`:98-108`): inside the `TICKS_PER_REFRESH` branch, before
  `self.dirty = true`, `self.refresh_active_tab();`. That helper mirrors
  `finish_external_edit` (`app/state.rs:370-396`), whose fields are all visible from `app::update`:

  ```rust
  /// D16: the active tab's refresh hook, with a `Ctx` addressed as that tab.
  fn refresh_active_tab(&mut self) {
      let Some(id) = self.tabs.active_id() else { return };   // adapt to active_id()'s real type
      let origin = Origin::Tab(id);
      {
          let Self { scope, projects, top_bar, keymap, theme, emit, tabs, .. } = self;
          let Some(view) = tabs.active_mut() else { return };
          let mut ctx = Ctx::new(scope, projects, top_bar, keymap, theme, origin.clone(), emit);
          view.on_refresh(&mut ctx);
      }
      self.drain(&origin);
  }
  ```

- `ui/tabs/backlog/detail/mod.rs`: `DetailTab` gains
  `/// Whether this sub-tab shows a run that is still active (D16). fn has_active_run(&self) -> bool { false }`,
  and `DetailRegistry` gains `pub fn has_active_run(&self) -> bool { self.tabs.iter().any(|tab| tab.has_active_run()) }`.
- `detail/runs.rs`: `RunsTab` overrides it:
  `self.item.is_some() && self.runs.iter().any(|run| run.status.is_active())`.
- `backlog/mod.rs`: `BacklogTab` gains `refreshes: u32` and

  ```rust
  /// Refreshes between two `Runs` polls: five of the shell's one-second refreshes (OQ-3).
  const REFRESHES_PER_RUNS_POLL: u32 = 5;

  fn on_refresh(&mut self, ctx: &mut Ctx<'_>) {
      self.refreshes = self.refreshes.wrapping_add(1);
      if !self.refreshes.is_multiple_of(REFRESHES_PER_RUNS_POLL) {
          return;
      }
      // `Runs` only: `RunsTab::on_runs` asks for `RunActions` after every `Runs` reply (`runs.rs:348`).
      if let Some(item) = self.item().map(|item| item.id) && self.detail.has_active_run() {
          ctx.request(StoreRequest::Runs(item));
      }
  }
  ```

**Tests** (`htui/tests/backlog.rs`, `app.update(Action::Tick)` in a loop):
- `the_runs_pane_rereads_an_active_run_every_poll`: select an item with a `running` run; move the
  run to `awaiting_approval` through the store handle; 19 ticks, and the pane still shows
  `running`; the 20th tick plus a drive, and it shows the park, with no frame sent.
- `the_runs_pane_does_not_poll_a_finished_item`: every run terminal; 40 ticks send no `Runs`.
- `the_runs_pane_poll_keeps_the_cursor` (added, MOD-4 D198): the cursor on the second entry stays
  there across a poll.

No snapshot changes. **Commits**: red, green. **Gate** G-T10.

---

## 13. T11 — pool size and DSN sources (D14's pool and DSN, PRD D3, D7)

### 13.1 `htui-store/src/pg/mod.rs`

```rust
/// A Postgres pool size (PRD D7): `htui worker --pool-size`, clamped to `2..=8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSize(u32);

impl PoolSize {
    /// The smallest pool: one walk and one read never wait on each other.
    pub const MIN: u32 = 2;
    /// The largest: today's TUI pool.
    pub const MAX: u32 = 8;
    /// The TUI's pool, today's value (`open_pool`'s former constant).
    pub const TUI: Self = Self(8);
    /// `htui worker`'s default.
    pub const WORKER_DEFAULT: Self = Self(4);

    /// `requested` clamped to `MIN..=MAX`, with a `warn` when it moved.
    #[must_use]
    pub fn clamped(requested: u32) -> Self {
        let used = requested.clamp(Self::MIN, Self::MAX);
        if used != requested {
            tracing::warn!(requested, used, "the pool size is clamped to 2..=8");
        }
        Self(used)
    }

    /// The connection count.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }
}
```

`open_pool(dsn, connect_timeout, pool: PoolSize)` (`:727-736`): `.max_connections(pool.get())`;
doc "eight connections" → "`pool` connections". `connect_with` (`:217`) passes `PoolSize::TUI`.
`connect_headless(dsn, identity, connect_timeout, pool: PoolSize)` (`:250`) passes `pool`; doc:
"the pool is built before any setting is read (PRD D7)". Callers:
`htui-store/tests/migrations.rs:985`, `:1004`, `:1040`, `:1184`, `:1201`, `:1211`, `:1222` pass
`PoolSize::TUI`; `concepts.rs:68` passes `PoolSize::TUI`.

### 13.2 `htui-store/src/secret.rs`

Module doc (`:1-14`) names the three sources for `htui worker` (the keyring, then
`$CREDENTIALS_DIRECTORY/htui-dsn` on Linux, or `--dsn-stdin` alone) and keeps "never argv, the
environment or a plain file".

```rust
/// The systemd credential name `htui worker` reads (`LoadCredentialEncrypted=htui-dsn:…`).
pub const CREDENTIAL_NAME: &str = "htui-dsn";

/// Where `htui worker` may read its DSN (PRD D3, plan D14).
pub struct DsnSources<'a> {
    /// `Some` exactly when `--dsn-stdin` was given: one line from it, and no other source.
    pub stdin: Option<&'a mut dyn std::io::BufRead>,
    /// `$CREDENTIALS_DIRECTORY` as the caller read it (a directory, not the secret; passed in so
    /// tests need no `set_var`). Read on Linux only.
    pub credentials_dir: Option<&'a std::path::Path>,
}
// hand-written Debug: `stdin: bool`, `credentials_dir: Option<&Path>`

/// Why no DSN was found. No variant carries the DSN.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DsnSourceError {
    /// `--dsn-stdin` read a blank line or end of input.
    #[error("no DSN on stdin; nothing was read")]
    EmptyStdin,
    /// Stdin failed.
    #[error("stdin could not be read: {0}")]
    Stdin(String),
    /// The credential exists and could not be read.
    #[error("the systemd credential {path} could not be read: {why}")]
    Credential { /** Its path. */ path: String, /** The OS error. */ why: String },
    /// No source had one.
    #[error("no Postgres DSN: the OS keyring has none ({keyring}), and there is no \
             $CREDENTIALS_DIRECTORY/htui-dsn; run `htui --set-dsn`, provision the systemd \
             credential, or pass `--dsn-stdin`")]
    NoSource { /** What the keyring answered: "empty" or its error. */ keyring: String },
}

/// One DSN line, trimmed, in a wiped buffer; `None` for a blank line or end of input. `htui
/// --set-dsn` and `htui worker --dsn-stdin` share it.
pub fn read_dsn_line(reader: &mut dyn std::io::BufRead) -> std::io::Result<Option<Zeroizing<String>>> {
    let mut line = Zeroizing::new(String::new());
    reader.read_line(&mut line)?;
    let dsn = line.trim();
    Ok((!dsn.is_empty()).then(|| Zeroizing::new(dsn.to_owned())))
}

/// `htui worker`'s DSN (PRD D3): `--dsn-stdin` alone when given; else the keyring (an `Err` is
/// "no keyring", not a failure); else, on Linux, `credentials_dir/htui-dsn`.
///
/// # Errors
/// [`DsnSourceError`]; never the DSN.
pub fn headless_dsn(sources: DsnSources<'_>) -> Result<Zeroizing<String>, DsnSourceError> {
    if let Some(stdin) = sources.stdin {
        return read_dsn_line(stdin)
            .map_err(|err| DsnSourceError::Stdin(err.to_string()))?
            .ok_or(DsnSourceError::EmptyStdin);
    }
    let keyring = match get_dsn() {
        Ok(Some(dsn)) => return Ok(Zeroizing::new(dsn)),
        Ok(None) => "empty".to_owned(),
        Err(err) => err.to_string(),
    };
    #[cfg(target_os = "linux")]
    if let Some(dir) = sources.credentials_dir {
        let path = dir.join(CREDENTIAL_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let text = Zeroizing::new(text);
                let dsn = text.trim();
                if !dsn.is_empty() {
                    return Ok(Zeroizing::new(dsn.to_owned()));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(DsnSourceError::Credential {
                path: path.display().to_string(), why: err.to_string() }),
        }
    }
    Err(DsnSourceError::NoSource { keyring })
}
```

(`get_dsn` returns a plain `String`; wrap it at once. A later hardening of `Slot::get` is out of
scope.)

### 13.3 `htui/src/concepts.rs`

`connect_headless(…, PoolSize::TUI)` at `:68`, and the refusal sentences factored out for T12
(plan D14: "`concepts.rs:68-79`'s sentences are shared, but its exit stays 1"):

```rust
/// A headless connect's refusal as the line a user reads (MOD-40 plan D8's sentences, byte for
/// byte). `--index-items` exits 1 with it, `htui worker` exits 2.
pub fn headless_refusal(err: HeadlessError) -> anyhow::Error {
    match err {
        HeadlessError::MigrationsPending(n) => anyhow::anyhow!(
            "{n} schema migration(s) are pending; start `htui` once to apply them"),
        HeadlessError::Store(err) => {
            let context = store_context(&err);
            anyhow::Error::new(err).context(context)
        }
        below @ HeadlessError::BelowTarget { .. } => below.into(),
    }
}
```

`open` (`:62-84`) maps with `.map_err(headless_refusal)?`.

### 13.4 Tests

- `migrations.rs`: `a_headless_pool_honours_its_size` (`connect_headless(…, PoolSize::clamped(3))`
  → `pool().options().get_max_connections() == 3`); `a_headless_pool_is_clamped` (`clamped(1)` →
  2, `clamped(64)` → 8, over a real connect).
- `secret.rs` (async `#[tokio::test]`s, `crate::testkit::mock_keyring()` /
  `mock_keyring_broken()` guards, `tempfile::tempdir()` for the directory):
  - `stdin_wins_and_the_keyring_is_not_read`: keyring holds `A`, stdin `B\n` → `B`; with the
    broken keyring, stdin still answers (the keyring was never opened).
  - `the_keyring_is_read_before_the_credential`: keyring `A`, credential file `C` → `A`.
  - `a_missing_keyring_falls_back_to_the_credential`: broken keyring, credential `C\n` → `C`.
  - `no_source_is_an_error_naming_all_three`: empty keyring, no directory → `NoSource`, whose
    `Display` names the keyring, `CREDENTIALS_DIRECTORY` and `--dsn-stdin`.
  - `the_credential_is_read_only_from_the_given_directory`: a `htui-dsn` in another tempdir is
    ignored; `NoSource`.
  - `a_blank_line_is_no_dsn`: stdin `"   \n"` → `EmptyStdin` (explicit wins: no fall-through).
  - `a_credential_file_holding_only_whitespace_is_no_dsn` (added): → `NoSource`.
  - Credential cases are `#[cfg(target_os = "linux")]`.

**Commits**: (1) red: tests, with `PoolSize`/`DsnSources`/`headless_dsn` declared with the final
signatures, and `headless_dsn` answering `NoSource` unconditionally so the assertions run and fail;
(2) green. **Gate** G-T11.

---

## 14. T12 — `htui worker` (D14, D15)

### 14.1 `htui/src/cli.rs`

```rust
#[derive(Debug, Clone, Default, clap::Parser)]
#[command(group = clap::ArgGroup::new("concepts").args(["index_items", "search_items"]))]
#[command(name = "htui", version, about = "Terminal UI for the htui workflow store",
          args_conflicts_with_subcommands = true)]
pub struct Args {
    /// A subcommand instead of the TUI. With one, no flag of the TUI may be given.
    #[command(subcommand)]
    pub command: Option<Command>,
    // … demo unchanged …
    /// Write logs to this file. Never stdout: stdout is the TUI. After a subcommand
    /// (`htui worker --log PATH`), not before it; `HTUI_LOG` works with either.
    #[arg(long, env = "HTUI_LOG", value_name = "PATH", global = true)]
    pub log: Option<PathBuf>,
    // … the rest unchanged …
}

/// `htui`'s subcommands (MOD-41 plan D14).
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Run this box's runs with no terminal: claim queued runs targeted at this box, walk them,
    /// heartbeat, recover, and stop on SIGINT/SIGTERM. The DSN comes from the OS keyring, the
    /// systemd credential `htui-dsn`, or `--dsn-stdin`; never argv or the environment.
    /// Engine-driven ACP steps still fail on their first permission request (MOD-42).
    Worker(WorkerArgs),
}

/// `htui worker`'s flags. None reads the environment.
#[derive(Debug, Clone, clap::Args)]
pub struct WorkerArgs {
    /// Postgres connections, 2 to 8 (clamped).
    #[arg(long, value_name = "N", default_value_t = 4)]
    pub pool_size: u32,
    /// Read the DSN from one line of stdin instead of the keyring or the credential.
    #[arg(long)]
    pub dsn_stdin: bool,
}
```

### 14.2 `htui/src/lib.rs`

`pub mod worker_cmd;`. The first statement of `run` (`:74-76`), before `init_tracing`:

```rust
if let Some(cli::Command::Worker(worker)) = args.command.clone() {
    // The worker installs its own subscriber (stderr without `--log`), MOD-41 plan D14.
    return worker_cmd::run(worker, args.log.as_deref()).await.map_err(anyhow::Error::from);
}
```

`set_dsn_from_stdin` (`:151-164`) uses `secret::read_dsn_line(&mut std::io::stdin().lock())`
(F-25); its sentences are unchanged.

### 14.3 `htui/src/worker_cmd.rs` (new)

```rust
//! `htui worker` (MOD-41 plan D14, D15): the headless connect, then `htui_worker::worker::run`.

/// How `htui worker` ends (plan D14): `main` maps it to the exit code.
#[derive(Debug)]
pub enum WorkerExit {
    /// Exit 2: nothing ran (no DSN, connect or schema refused, signals not installable).
    Refused(String),
    /// Exit 1: the worker ran and failed.
    Failed(String),
}
impl WorkerExit {
    /// 2 for a refusal, 1 for a failure.
    #[must_use]
    pub const fn code(&self) -> u8 { match self { Self::Refused(_) => 2, Self::Failed(_) => 1 } }
}
// Display: the sentence; impl std::error::Error.

/// `htui worker`.
///
/// # Errors
/// [`WorkerExit`]; a clean signal shutdown is `Ok`.
pub async fn run(args: WorkerArgs, log: Option<&Path>) -> Result<(), WorkerExit> {
    init_worker_tracing(log).map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let shutdown = shutdown_signal().map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pool = PoolSize::clamped(args.pool_size);            // before any setting (PRD D7)
    let dsn = read_dsn(args.dsn_stdin)?;
    let root = identity::config_root().map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = connect(&dsn, &root, pool).await?;
    drop(dsn);                                                // zeroized
    let runtime = RunRuntime::<PgStore, Unaddressed>::production().with_role(Role::Worker);
    tracing::info!(box_id = %pg.this_box(), pool = pool.get(), "htui worker ready");
    htui_worker::worker::run(pg, runtime, WorkerConfig::PRODUCTION, shutdown).await;
    Ok(())
}

/// PRD D3's sources. The prompt goes to stderr only when stdin is a terminal.
fn read_dsn(dsn_stdin: bool) -> Result<Zeroizing<String>, WorkerExit> {
    let credentials = std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from);
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    if dsn_stdin && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!("paste the DSN and press Enter (it will be visible):");
    }
    secret::headless_dsn(DsnSources {
        stdin: dsn_stdin.then_some(&mut lock as &mut dyn std::io::BufRead),
        credentials_dir: credentials.as_deref(),
    })
    .map_err(|err| WorkerExit::Refused(err.to_string()))
}

/// The headless connect and the registration write-back (plan D14): every refusal before any
/// write, with `concepts::headless_refusal`'s sentences. `pub` for `worker_pg.rs` case 4.
///
/// # Errors
/// [`WorkerExit::Refused`].
pub async fn connect(dsn: &str, root: &Path, pool: PoolSize) -> Result<PgStore, WorkerExit> {
    let presented = identity::load_or_mint(root).map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = PgStore::connect_headless(dsn, &presented, CONNECT_TIMEOUT, pool)
        .await
        .map_err(|err| WorkerExit::Refused(format!("{:#}", concepts::headless_refusal(err))))?;
    connect::persist_registration(root, &presented, &pg)
        .map_err(|err| WorkerExit::Refused(err.to_string()))?;
    Ok(pg)
}

/// `--log PATH` as the TUI writes it (`lib.rs:170-192`); without it, stderr (ANSI only on a
/// terminal). Both add `sentry_tracing::layer()`. `HTUI_LOG_FILTER` as the TUI.
fn init_worker_tracing(log: Option<&Path>) -> anyhow::Result<()>;

/// SIGINT or SIGTERM on unix; Ctrl-C, Ctrl-Break, Ctrl-Close and Ctrl-Shutdown on Windows (R-8:
/// uncompiled here). Installed before connecting, so a refusal to install is a startup refusal.
fn shutdown_signal() -> std::io::Result<impl Future<Output = ()> + Send + 'static> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        Ok(async move { tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} } })
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
        let (mut c, mut b, mut cl, mut sd) = (ctrl_c()?, ctrl_break()?, ctrl_close()?, ctrl_shutdown()?);
        Ok(async move { tokio::select! { _ = c.recv() => {}, _ = b.recv() => {},
                                         _ = cl.recv() => {}, _ = sd.recv() => {} } })
    }
}
```

The DSN is never a field, a message or a `Debug` of anything logged. `Zeroizing<String>` is not
passed to `tracing`.

### 14.4 `htui/src/main.rs`

`in_app_include` gains `"htui_worker"` (`:18`). The error arm:

```rust
Err(error) => {
    let code = error
        .downcast_ref::<htui::worker_cmd::WorkerExit>()
        .map_or(1, htui::worker_cmd::WorkerExit::code);
    if code != 2 {                         // E-1 (recommended): a refusal is not a crash report
        sentry_anyhow::capture_anyhow(&error);
    }
    eprintln!("htui: {error:#}");
    ExitCode::from(code)
}
```

(E-1 answered "no" → delete the `if`, keep the capture.) Never `process::exit`: the `_sentry`
guard must flush.

### 14.5 `htui-worker/src/worker.rs` (new)

```rust
//! `htui worker`'s loop (MOD-41 plan D14): poll, box heartbeat, shutdown.

/// How often the worker sweeps and scans (plan D14): hand-back latency is bounded by this.
pub const WORKER_POLL: Duration = Duration::from_secs(5);
/// A cancelled walk's graceful window on shutdown, as the TUI's `CANCEL_GRACE`.
pub const WALK_GRACE: Duration = Duration::from_secs(2);

/// The worker's periods (B-11: tests shorten them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerConfig {
    /// Sweep and claim scan.
    pub poll: Duration,
    /// `box.last_seen_at`; the first beat is at start.
    pub box_beat: Duration,
    /// Walk grace on shutdown.
    pub grace: Duration,
}
impl WorkerConfig {
    /// Production: 5 s, `BOX_HEARTBEAT`, 2 s.
    pub const PRODUCTION: Self = Self { poll: WORKER_POLL,
        box_beat: htui_store::connect::BOX_HEARTBEAT, grace: WALK_GRACE };
}

/// The loop until `shutdown` resolves, then the runtime's shutdown (walks cancelled, leases
/// given back). A store outage is logged by each arm and the loop carries on (plan D14).
pub async fn run(host: PgStore, mut runtime: RunRuntime<PgStore, Unaddressed>,
    config: WorkerConfig, shutdown: impl Future<Output = ()>) {
    let mut poll = tokio::time::interval(config.poll);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut beat = tokio::time::interval(config.box_beat);      // first tick now (B-11)
    beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut beating: Option<JoinHandle<()>> = None;
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            _ = poll.tick() => runtime.sweep_with(&host, &Unaddressed),
            _ = beat.tick(), if beating.is_none() => {
                beating = Some(tokio::spawn(beat_once(host.clone())));
            }
            () = in_flight(&mut beating), if beating.is_some() => beating = None,
        }
    }
    if let Some(beat) = beating { beat.abort(); }
    runtime.shutdown(config.grace).await;
}
```

`beat_once` and `in_flight` copy `store_worker.rs:2036-2059` (`touch_box(this_box())`;
`Ok(false)`/`Err` → `warn!`, whatever the executor). `lib.rs` adds `pub mod worker;` and re-exports
`Role` (T9) and `Unaddressed`.

### 14.6 Tests

`cli.rs`:
- `worker_parses_its_two_flags`: `htui worker --pool-size 6 --dsn-stdin` →
  `Some(Command::Worker(WorkerArgs { pool_size: 6, dsn_stdin: true }))`; bare `htui worker` → 4, false.
- `flat_flags_still_parse_and_refuse_a_subcommand_beside_them`: `htui --offline` parses with
  `command: None`; `htui --demo worker` is an error.
- `no_worker_argument_reads_the_environment`: over `Args::command().find_subcommand("worker")`,
  every `Arg` except `log` has `get_env() == None`; `log`'s is `Some("HTUI_LOG")`.
- `log_goes_after_the_subcommand`: `htui worker --log x` parses (`args.log == Some("x")`);
  `htui --log x worker` is an error.
- `worker_takes_no_dsn_argument`: `htui worker --dsn x` is an error.
- `worker_exit_codes` (added, `worker_cmd.rs`): `Refused` → 2, `Failed` → 1.

`htui/tests/worker_pg.rs` (skipped with `testkit::SKIP` without `HTUI_TEST_DATABASE_URL`; a minimal
copy of `runs_pg.rs`'s stack, F-29; the worker runs as a spawned `worker::run` over `db.store`
with `RunRuntime::<PgStore, Unaddressed>::with_parts(FakeIsolator, FakeVerifier, factory)
.with_role(Role::Worker)` and `WorkerConfig { poll: 50 ms, box_beat: 1 s, grace: ZERO }`, stopped
by a `oneshot`):
1. `a_headless_worker_drives_a_queued_run_to_rest`: box executor `worker`; a run queued through
   the TUI-shaped stack; the worker drives it to `done`; the lease is released.
2. `a_tui_exit_does_not_interrupt_a_worker_run`: `R` answers `Started { rest.run: Queued }` and
   the row's `lease_expires_at` is `None`; the stack is dropped (the TUI's runtime shuts down); the worker settles
   the run to `done`.
3. `an_answer_handed_back_is_finished_by_the_worker`: a gated graph parks; the stack answers `x`
   (reject, loopable); the answer's rest is `running` with no lease; the worker walks the review
   loop to the next park or rest, equal to the in-process outcome.
4. `the_worker_refuses_a_pending_schema_and_writes_nothing`: `testkit::bare_db()`;
   `worker_cmd::connect(url, tempdir, PoolSize::WORKER_DEFAULT)` is `Refused` (code 2) whose
   sentence contains `pending`; `_sqlx_migrations` does not exist and no `box` table either.
5. `the_worker_binary_never_exposes_its_dsn` (`cfg(target_os = "linux")`):
   `Command::new(env!("CARGO_BIN_EXE_htui")).args(["worker", "--dsn-stdin", "--log", <tmp>/w.log])`,
   `.env_clear()`, `HOME` and `XDG_CONFIG_HOME` set to a tempdir, `USERNAME=htui-ci`, stdin piped
   with the demo DB's DSN rewritten to carry the password `sentinel-<uuid>` (trust auth accepts it).
   Poll the log for `htui worker ready` (≤ 60 s), then read `/proc/<pid>/cmdline` and
   `/proc/<pid>/environ`: neither contains the sentinel. `kill -TERM <pid>` gives exit 0 within 10 s;
   the log does not contain the sentinel.
6. `the_worker_binary_refuses_a_dsn_it_cannot_use_without_echoing_it` (F-27): same environment; the
   DSN names database `htui_no_such_<uuid>` with password `sentinel-<uuid>`; exit code 2; neither
   stderr nor the log contains the sentinel.

**Commits**: (1) red `cli.rs` tests; (2) CLI, `WorkerExit`, `main.rs`, `lib.rs` arm; (3) red
`worker_pg.rs`; (4) green `worker.rs`, `worker_cmd.rs`, tracing, signals, `set_dsn_from_stdin`.
**Gate** G-T12.

---

## 15. T13 — docs (D17, D18)

- `CONCEPTS.md` (plan D17): lines 15-16 keep "not a cloud service" and add the R-ID-2 sentence;
  lines 20-21 end with the R-STO-1 sentence; the owner line and "Reviewed:" date are bumped.
- `docs/htui-worker.md` (new), sections in this order: what it does and does not do (no
  permission answers until MOD-42, no remote targeting until MOD-43); the executor setting, I-1,
  the flip window both ways (D13), and the hand-back table (D12, with F-24's changed-graph
  semantics and the lost-release case); the three DSN sources in PRD D3's order; `--pool-size`;
  logs (`--log` after `worker`, `HTUI_LOG`); exit codes 0/1/2; the per-run backoff (OQ-6); the
  queued-cancel residual (E-2, as answered); a **system** unit sample with `User=`,
  `ExecStart=/home/<you>/.cargo/bin/htui worker --log /home/<you>/.local/state/htui/worker.log`,
  `LoadCredentialEncrypted=htui-dsn:/etc/credstore.encrypted/htui-dsn`, `Restart=on-failure`,
  `WantedBy=multi-user.target`, the `systemd-creds encrypt --name=htui-dsn` step (root, host key
  or TPM), and the user-unit variant for systemd ≥ 256 only (R-9).
- `README.md` option table (`:73-87`): one `htui worker` row pointing at `docs/htui-worker.md`.

No test. **Gate** G-T13.

---

## 16. T14 — background index sync (D19)

`htui/src/concepts.rs`:

```rust
/// The `app_setting` key of the worker's index interval (plan D19).
pub const SYNC_MINUTES_KEY: &str = "concepts_sync_minutes";
/// The interval when the key is absent, non-positive or not an integer.
pub const DEFAULT_SYNC_MINUTES: u64 = 15;

/// `index_items`' loop (`:128-145`) over given scopes (plan D19, fact-check F-C4).
///
/// # Errors
/// The first scope's store or index error.
pub async fn sync_all(read: &impl ReadStore, scopes: &[Scope], vectors: &impl VectorStore)
-> Result<SyncReport, StoreError> {
    let mut report = SyncReport::default();
    for scope in scopes {
        report += Indexer::sync(read, scope, vectors).await?;
    }
    Ok(report)
}

/// The interval `app` asks for, re-read each cycle (`recover.rs:80-82`'s rule).
#[must_use]
pub fn sync_interval(app: &BTreeMap<String, Value>) -> Duration;

/// What the index job reads besides `ReadStore` (inherent on both stores).
#[allow(async_fn_in_trait, reason = "no dyn IndexSource is formed")]
pub trait IndexSource: ReadStore {
    /// Every workspace's projects as scopes (`scopes(pg, None)`).
    async fn index_scopes(&self) -> anyhow::Result<Vec<Scope>>;
    /// The `app_setting` map.
    async fn index_settings(&self) -> Result<BTreeMap<String, Value>, StoreError>;
}
impl IndexSource for PgStore { /* scopes(self, None), PgStore::app_settings */ }
impl IndexSource for MemStore { /* MemStore::workspaces → Scope::from_workspace, MemStore::app_settings */ }

/// Sync at start, then every `sync_interval`, forever; a failure is a `warn` and the next cycle.
/// Never touches `RunRuntime`; the worker aborts it on shutdown.
pub async fn index_loop(source: &impl IndexSource, vectors: &impl VectorStore) -> ! {
    loop {
        match source.index_scopes().await {
            Ok(scopes) => match sync_all(source, &scopes, vectors).await {
                Ok(report) => tracing::info!(?report, "concepts index synced"),
                Err(err) => tracing::warn!(%err, "concepts index sync failed; next cycle"),
            },
            Err(err) => tracing::warn!(%err, "concepts index scopes failed; next cycle"),
        }
        let app = source.index_settings().await.unwrap_or_default();
        tokio::time::sleep(sync_interval(&app)).await;
    }
}

/// The worker's job, when the keyring holds a Qdrant URL (read once, PRD D6); `None` with an
/// `info` saying why otherwise (always `None` on a keyring-less host).
pub async fn spawn_index_job(pg: PgStore) -> Option<tokio::task::JoinHandle<()>>;
```

`spawn_index_job` reads `secret::get_qdrant_url()` and the API key once, builds `settings_from`,
`FastEmbedder::new()` on `spawn_blocking`, and `QdrantStore::connect`. Any failure there is an
`info`/`warn` and `None`: runs are unaffected. It then spawns `index_loop(&pg, &store)`.
`index_items` calls `sync_all(&pg, &scopes(&pg, project).await?, &store)`.
`worker_cmd::run` calls `let job = concepts::spawn_index_job(pg.clone()).await;` before the loop,
and `if let Some(job) = job { job.abort(); }` after it.

**Tests** (`concepts.rs`, `MemVectorStore` + `HashEmbedder` from `htui-store/test-support`,
`vector.rs:780`, `embed.rs:80-82`; a `Counting<V>` `VectorStore` wrapper counting syncs):
- `the_index_job_syncs_at_start_and_every_interval` (`start_paused`): `index_loop` over
  `MemStore::demo()`; one sync at 0; the second after 15 min of paused time; the third after 30.
- `a_failing_sync_is_logged_and_retried_next_cycle`: the wrapper fails its first upsert; the loop
  is still alive, and the next cycle syncs.
- `the_interval_reads_concepts_sync_minutes_with_a_15_minute_default`: `sync_interval` for
  absent, `0`, `-3`, `"5"`, `2.5` → 15 min; `5` → 5 min; and a `set_app_setting` between cycles
  moves the next sleep.
- `no_qdrant_url_means_no_job` (`mock_keyring()` guard, empty slots): `spawn_index_job` is
  `None`, over a lazy `PgStore` (`PgStore::lazy`-style constructor, feature `demo`) that is
  never queried.
- `htui/tests/qdrant_worker.rs::the_worker_job_indexes_a_new_item` (gated on
  `HTUI_TEST_QDRANT_URL` as `htui-store/tests/qdrant_live.rs:21`, plus the Postgres DSN): a new
  item appears in a search after one cycle.

**Gate** G-T14 (re-runs T12 case 5, F-31).

---

## 17. Wave schedule and lane rules

| Wave | Tasks | Where | Postgres |
|---|---|---|---|
| W1 | T1 | primary tree | yes (store conformance, `pg_criteria`, sqlx) |
| W2 | T2 | primary | no (orch; `htui` runtime tests use MemStore) |
| W3 | T3 | primary | yes (store gate) |
| W4 | T4 | primary | yes (`htui` gate includes `runs_pg`) |
| W5 | T5 | primary | yes (`htui` gate) |
| W6 | T6 | primary | yes (`htui` gate) |
| **W7** | **T7 ∥ T10 ∥ T11** | T7 primary; T10, T11 worktrees | T7 and T11 both need it: one at a time |
| **W8** | **T8 ∥ T9** | T9 primary; T8 worktree | both run the `htui` gate: one at a time |
| **W9** | **T12 ∥ T13** | T12 primary; T13 worktree (docs, no build) | T12 only |
| W10 | T14 | primary | yes |

The parallel marks are the plan's (disjoint file sets, re-intersected in its fact-check).
F-14's two additions (`htui-core/src/model/mod.rs` to T7, `htui-orch/src/lib.rs` to T9) keep
every pair disjoint.

**Lane rules**

1. **Branch first.** From the wave's base commit (the primary tree's HEAD after the previous wave's
   merged gate): `git worktree add -b hr/MOD-41-t10 ../htui-wt/mod-41-t10 <base>`, and the same
   for `-t11`, `-t8` and `-t13`. The primary lane stays on `hr/MOD-41`.
2. **Reads and edits.** The primary lane uses Gortex. Worktree lanes use native tools
   (`sed`, `grep`, Edit) for both reads and edits: Gortex does not index worktrees, and its edit
   tools would write the primary checkout.
3. **One Postgres lane at a time.** Any command that runs with `HTUI_TEST_DATABASE_URL` set, or
   that runs `cargo sqlx prepare`, is wrapped in `flock /tmp/mod41-pg.lock …`. While waiting for
   the lock, a lane may run its suites with `env -u HTUI_TEST_DATABASE_URL` (the Postgres cases
   skip). A lane's gate is not green until its full run has passed under the lock.
4. **`.sqlx` belongs to T7 alone** in W7. T11 must not run `cargo sqlx prepare`: `pg/mod.rs`'s pool
   change touches no `query!`.
5. **Commit incrementally** on the lane's branch: red, then green, per the task's commit list.
   Uncommitted work dies with the session. Never stash on a shared tree.
6. **Disk.** 103 GB free at HEAD; about 10 GB of `target/` per building worktree (T10, T11, T8).
   Clear `target/debug/incremental` before W7. T13 builds nothing.
7. **Merge.** The primary lane commits first. Then `git merge --no-ff` each worktree branch, never
   under a running build: W7 merges T11, then T10; W8 merges T8; W9 merges T13. Then run the
   merged-tree gate for **every** task of the wave with **no lane running**, at `--test-threads=1`
   for any failure. Then `git worktree remove --force ../htui-wt/mod-41-tN` and
   `git branch -d hr/MOD-41-tN`, in that order.
8. **Hidden coupling to expect at merge:** T7's `box_.rs` change rebuilds everything; T11's
   `connect_headless` signature and T7's `BoxEdit` literals touch no common file. T8 edits
   `boxes.rs`, where T7 added `executor: None`: T8 starts from W7's merged tree, so no conflict.
9. **Sandbox fetch.** Before `scripts/hr down --purge`, every lane branch is on the host
   (`git fetch <run_dir>/src <branch>:<branch>`), or the purge refuses.

---

## 18. Pins

| Pin | Now | After | Where it moves |
|---|---|---|---|
| Store conformance `CASES` | 96 | 103 | T1 +4 (100), T7 +3 |
| `READ_CASES` | 14 | 14 | — |
| `htui-orch` `CASES` | 73 | 85 | T1 +1 (74), T9 +11 |
| `.sqlx` files | 288 | 290 | T1 +1 (`step_fence`; `set_step_prompt` replaced), T7 +1 (`queued_runs_on_box`; `edit_box` replaced) |
| `crates/htui/tests/snapshots` | 107 | 108 | T8 (five re-accepted, one new) |
| Workspace members | 5 | 6 | T6 |
| `WorkerStore` / `RecorderStore` / `WorkerHost` methods | — | 42 / 3 / 22 | T3 (21 host), T7 (22) |
| Unchanged | | | `StoreRequest` 85, `StoreReply` 47, `GraphSource` 7, `hierarchy::REQUEST_NAMES` 13, `MIRRORED_TABLES` 21, seven Settings sections, 34 commented columns, migrations `0001`-`0007` (next `0008`) |

Each moved pin's assertion message names its reason ("MOD-41 T1's four fence cases", …). The
implementer re-counts at each gate.

---

## 19. Gate reference

```bash
PG="env USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres"
LOCK="flock /tmp/mod41-pg.lock"
gate() { $LOCK $PG cargo test -p "$1" --all-features -- --test-threads=2; }   # a failure: re-run it alone at --test-threads=1
SQLX_DB=postgres://postgres@localhost:5439/htui_prepare_check
# once per run (team memory sqlx-prepare-needs-migrated-scratch-db, sandbox variant):
#   psql postgres://postgres@localhost:5439/postgres -c 'CREATE DATABASE htui_prepare_check;'
#   (cd crates/htui-store && DATABASE_URL=$SQLX_DB sqlx migrate run --source migrations)
regen() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB cargo sqlx prepare -- --all-features --all-targets); }
check() { (cd crates/htui-store && $LOCK env DATABASE_URL=$SQLX_DB cargo sqlx prepare --check -- --all-features --all-targets); }
common() { cargo fmt --all -- --check && cargo clippy --workspace --all-features --all-targets -- -D warnings; }
```

| Gate | Commands |
|---|---|
| G-T1 | `cargo build --workspace --all-features --all-targets`; `gate htui-core`; `gate htui-store`; `gate htui-agent`; `gate htui-orch`; `regen` then `check`; `ls crates/htui-store/.sqlx \| wc -l` = 289; `common` |
| G-T2 | `gate htui-orch`; `$PG cargo test -p htui-orch --all-features -- a_walk_whose_refresh_keeps_failing_stops_before_the_lease_lapses --test-threads=1` and the three at `engine.rs:8601`, `:8653`, `:11426` by name; `env -u HTUI_TEST_DATABASE_URL cargo test -p htui --all-features run_worker -- --test-threads=2`; `common` |
| G-T3 | `cargo build --workspace --all-features --all-targets`; `gate htui-core`; `gate htui-store`; `gate htui-agent`; `common` |
| G-T4 | `gate htui-agent`; `gate htui-orch`; `gate htui`; the acceptance grep of §6; `common` |
| G-T5 | `gate htui`; `$LOCK $PG cargo test -p htui --all-features run_worker -- --test-threads=1`; `common` |
| G-T6 | `gate htui-worker`; `gate htui`; `cargo tree -p htui-worker -e normal --all-features \| grep -E 'ratatui\|crossterm'` prints nothing; `cargo doc --workspace --no-deps --keep-going` shows only the six baseline errors; `common` |
| G-T7 | `gate htui-core`; `gate htui-store`; `gate htui-orch`; `gate htui`; `regen` then `check`; `.sqlx` = 290; `common` |
| G-T8 | `gate htui`; `cargo insta test -p htui --all-features` with nothing pending; `common` |
| G-T9 | `gate htui-orch`; `gate htui-worker`; `gate htui`; `common` |
| G-T10 | `gate htui`; no snapshot pending; `common` |
| G-T11 | `gate htui-store`; `cargo build -p htui --all-features --all-targets`; `common` |
| G-T12 | `gate htui-worker`; `gate htui` (includes `worker_pg.rs` cases 1-6); `common` |
| G-T13 | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| G-T14 | `gate htui` (re-runs `worker_pg` case 5, F-31); `$LOCK $PG HTUI_TEST_QDRANT_URL=… cargo test -p htui --all-features --test qdrant_worker` when a Qdrant is available; `common` |
| Final (after W10, no lane running) | The plan's Validation list with the sandbox DSN: `cargo fmt --all -- --check`; clippy; `$LOCK $PG cargo test --workspace --all-features --no-fail-fast -- --test-threads=2` then any failure alone at `--test-threads=1` (the known pre-existing failure is `every_provider_failure_leaves_a_valid_prompt`); `check`; the `cargo tree` check; `cargo doc`; `cargo insta test --workspace`; `validate-workflow-docs.sh` |
