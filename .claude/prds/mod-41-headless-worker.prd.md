# MOD-41 — Headless worker (`htui worker`)

> Routed as **PRD** by `/handoff-run MOD-41` (criteria C2, C3 and C4 fired; accepted by the
> maintainer 2026-09-29, sandbox run `hr/MOD-41`). Ultracode recommended for the implement phase
> (C4, ≥3 criteria) and the review phase. `docs/ANA-16.md` §7-§8 item 2 is the design; §9 hazards 1,
> 10 and 13 are the open forks. MOD-40 (`docs/decisions/mod/mod-40.md`) left four items here, MOD-4
> M6 (`docs/decisions/mod/mod-4.md`) left two, and MOD-34 left one.
> Requirements: `R-ORCH-12`, `R-ID-2`, `R-STO-1`, `R-NF-2`, `R-NF-3`; `R-STO-5` as amended by MOD-40;
> `R-STO-8` for the background index sync.

## Problem

Runs are supervised only inside the TUI process. `RunRuntime` (lease refresh, sweep, one engine
task per claimed run) lives in the TUI crate and is hosted by the store worker's `select!` loop, so
a run lives exactly as long as a terminal session: closing or crashing the TUI interrupts every
step it drives, and a box without a terminal cannot execute anything. Every later item on the
remote path (MOD-24's crash recovery, MOD-43 remote dispatch, MOD-44 containers, MOD-45 SSH
provisioning, MOD-47's control plane) presupposes a process that runs runs without a UI. Its DSN is
also the one piece `R-STO-1` currently forbids on a headless host: the Linux keyring backend is
`sync-secret-service`, which needs a desktop session.

## Evidence

Read at `b5e481f` (main). Paths are relative to the repo root.

- **The supervisor is TUI-bound by addressing, not by rendering.**
  `crates/htui/src/run_worker.rs` (4621 lines; code 1-2410, tests 2412-4621) imports neither
  `ratatui` nor `crossterm`, and every workspace crate it uses is UI-free. What binds it is
  `crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, Seq, StoreReply, StoreRequest}`
  (`run_worker.rs:60`) and `crate::agent_worker::ReplyAddr` (`:59`, `:591`): `Origin`
  (`store_worker.rs:78`) has `Tab(TabId)`/`Overlay(OverlayId)` variants imported from `crate::ui`
  (`store_worker.rs:52-53`), which import `ratatui` and `crossterm::event::KeyEvent`
  (`ui/tabs/registry.rs:8-12`, `ui/overlay/registry.rs:7-8`). `store_worker.rs:39-53` also pulls a
  dozen TUI modules into the request/reply enums.
- **Hosting.** `lib.rs:114` → `store_worker::spawn_with` → `spawn_with_runtimes(…,
  RunRuntime::production())` (`store_worker.rs:1469`, `:1526`). The loop sweeps on `Online`
  (`:1936`) and after migrations, ticks the sweeper while a writer exists, routes
  `StoreRequest::Orch | RunStream | RunActions` to `runs.serve` (`:1884-1896`), and joins
  `runs.shutdown(CANCEL_GRACE)` on exit (`:2015-2018`). The lease owner is minted once per process
  (`run_worker.rs:1387`, `Uuid::now_v7()`).
- **No narrow store trait exists.** The engine is generic over `S: WriteStore`
  (`crates/htui-orch/src/engine.rs:381-382`); `htui-orch` must never depend on `htui-store`
  (`crates/htui-orch/Cargo.toml:18-20`, ANA-2 inv. 10). The concrete store is `htui_store::Writer`
  (`Memory | Online`, `writer.rs:52`); `run_worker` also calls ~15 `Backend` inherent reads
  (`backend.rs`: `writer`, `run`, `run_steps`, `app_settings`, `box_info`, `repo_paths`, `box_row`,
  `box_profile`, `agents`, `active_runs_on_box`, …). The engine's own calls span leases
  (`create_run`, `claim_run`, `refresh_lease`, `take_lease`, `release_lease`, `adopt_runs`), step
  writes, gates and ~20 reads; the seams it already owns are `GraphSource` (7 methods),
  `SessionSink`, `RunFence`, `AgentSelector`, `Isolator`, `Verifier`.
- **Crate graph.** `htui-core` ← `htui-agent` ← `htui-orch`; `htui-store` ← `htui-core`; only
  `crates/htui` declares `ratatui`/`crossterm` (`crates/htui/Cargo.toml:35,37`). One `[[bin]]`
  (`crates/htui/Cargo.toml:15`).
- **CLI.** clap derive, flat flags only (`crates/htui/src/cli.rs:13-60`: `--demo`, `--log`,
  `--set-dsn`, `--clear-dsn`, `--offline`, `--index-items`, `--search-items`, …); dispatched by an
  early-return chain in `lib.rs:75-97`. No subcommands.
- **Headless connect exists.** `PgStore::connect_headless` (`crates/htui-store/src/pg/mod.rs:250-290`)
  refuses pending/newer/dirty schema and a build below `htui_target_version` (`HeadlessError`,
  `:118-143`), then bootstraps and registers the box. Only caller: `concepts::open`
  (`crates/htui/src/concepts.rs:62-84`).
- **DSN.** Keyring entry `("htui", "postgres-dsn")` (`crates/htui-store/src/secret.rs:20-23`); the
  module forbids any env-var fallback (`secret.rs:1-7`). keyring 3.6.3 is built with
  `windows-native`, `apple-native`, `sync-secret-service`, `crypto-rust` (`Cargo.toml:46-47`); the
  crate also offers `linux-native` (kernel keyutils) and `linux-native-sync-persistent`
  (keyutils in front of secret-service). `connect::StartOptions.dsn` (`connect.rs:202`) already
  accepts an injected DSN.
- **Pool.** `open_pool` hardcodes `max_connections(8)` (`pg/mod.rs:727-736`) for both
  `connect_with` and `connect_headless`. ANA-16 §6.1 puts the budget at about 12 processes at
  defaults.
- **Unfenced step writes left by MOD-40.** `set_step_prompt` (`traits.rs:523`, Pg `write.rs:1648-1667`,
  `WHERE id = $1`), `upsert_step_tree` (`traits.rs:1219`, Pg `write.rs:4560-4625`) and
  `record_commits` (`traits.rs:1228`, Pg `write.rs:4639-4671`) take no `StepFence`; the engine calls
  them at 3, 2 and 7 sites. The sink's document write (`write_document`, item-keyed, `traits.rs:1269`)
  has **no production caller**: `ProgressSink::after_done` writes only when a `StepAuthor` is set and
  production's is `None` (`run_worker.rs:618-653`). The fenced trio uses `StepFence`
  (`traits.rs:2086-2093`) and `StoreError::Fenced` (`error.rs:23-31`).
- **Wall-clock self-fence.** `recover::heartbeat` (`crates/htui-orch/src/recover.rs:124-169`) sleeps
  on tokio's monotonic clock but computes `fence = until - margin` from `Clock::now()`
  (`SystemClock` = `Utc::now()`, `crates/htui-core/src/clock.rs:37-44`). MOD-40 blueprint F-38
  (`.claude/plans/mod-40-m4.blueprint.md:88`) and the doc at `recover.rs:119-120` hand the monotonic
  fix to this item.
- **Claiming is already box-scoped.** `claim_run` returns `NotClaimable` unless
  `target_box_id = box_id` (`pg/write.rs:3647`) under a box-row lock; `adopt_runs` keys on
  `executing_box_id` with `SKIP LOCKED` (`:3862-3911`). The box heartbeat (`touch_box`, 60 s) runs
  from the store worker (`store_worker.rs:1584-1586`, `:2050-2059`).
- **Index sync.** `vector_sync::Indexer::sync(read, scope, vector_store)`
  (`crates/htui-store/src/vector_sync.rs:67`) has one production caller, `htui --index-items`
  (`concepts.rs:128`); `concepts.rs:7-8` says the automatic sync belongs to this item.
- **Permission gap (MOD-42, not this item).** The engine's session uses `PermissionPolicy::default()`
  (`engine.rs:5381`); a parked ACP permission request makes `next_event` return a transport error
  (`crates/htui-agent/src/acp/mod.rs:506-510`) that `pump` (`record.rs:1797-1816`) propagates. A
  worker inherits this exactly as the TUI's engine path has it today.

## Users

- **The maintainer**, who wants a run to outlive the terminal it was started from, and a box with no
  desktop session to execute runs.
- **MOD-24** (worker crash recovery), **MOD-43** (remote dispatch), **MOD-44** (containers need a
  worker to survive TUI exit), **MOD-45** (installs `htui worker` as a service), **MOD-47** (implements
  the worker-store trait as a client).
- **Not for:** remote-box targeting from the TUI (MOD-43), provisioning (MOD-45), permission relay
  (MOD-42).

## Hypothesis

If run supervision lives in a UI-free library that both `htui` and `htui worker` link, and reaches the
store only through a named worker-store trait, then a worker process on a box can claim, drive,
heartbeat and recover that box's runs with no terminal attached, a TUI exit stops interrupting runs
the worker owns, and MOD-47 can later swap Postgres for a client without touching the engine.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Headless run | `htui worker` with no TTY and no `TERM` claims a queued run targeted at its box and drives it to a settled state | Integration test against Postgres (`HTUI_TEST_DATABASE_URL`), fake agent |
| UI-free library | The crate hosting run supervision has no `ratatui`/`crossterm` in its dependency tree | `cargo tree -p <crate> -e normal` check in a test or `xtask`-style script |
| TUI exit does not interrupt | A run owned by the worker keeps its lease and settles after the TUI process exits | Integration test: TUI-shaped client queues, exits; worker settles |
| Store surface is named | The engine and supervisor compile against the worker-store trait only; `WriteStore` is not in their bounds | Compile-time: bounds on `Engine` and the supervisor; a doc-test or trait-object check |
| Schema refusal | Pending, newer or dirty schema, or a build below `htui_target_version`, exits non-zero with the reason and writes nothing | Test over `connect_headless` errors through the worker entry |
| DSN never exposed | The worker's DSN never appears in `argv`, the process environment, logs, or a plaintext file | Tests over each accepted source (per D3) and the log scrubber |
| Pool bounded | The worker's pool honours its configured size (default below 8) | `pg_criteria`-style case reading `pool.options().get_max_connections()` |
| Stale holder fully fenced | After adoption, the old owner's `set_step_prompt`, `upsert_step_tree` and `record_commits` write nothing and return `Fenced` | Conformance cases on MemStore and PgStore |
| Step-proof self-fence | A wall-clock jump forward or back during a lease neither extends nor cuts the heartbeat's self-fence | `recover` test with a stepped `TestClock` under paused tokio time |
| Background index sync | With Qdrant configured, the worker re-syncs the concepts index on an interval; with it absent or failing, runs are unaffected | Unit test with a fake `VectorStore`; `qdrant_live` case |

## Scope

**MVP**

- A UI-free library crate holding run supervision (`RunRuntime`, its tasks, lease/sweep/claim), with
  request/reply addressing that does not name TUI tabs or overlays; the TUI links it unchanged in
  behaviour.
- A worker-store trait in `htui-core` covering exactly what the engine and supervisor call; the
  engine's bound moves from `WriteStore` to it.
- `htui worker`: headless connect, DSN from the sources D3 allows, configurable pool size, sweep on
  start, periodic claim of queued runs targeted at this box, lease refresh, box heartbeat, graceful
  shutdown on SIGINT/SIGTERM (ctrl-c on Windows), logs to a file or stderr.
- Executor handover on a box that runs a worker (D4), so a TUI-started run is worker-owned.
- MOD-40 leftovers: fence `set_step_prompt`, `upsert_step_tree`, `record_commits`; monotonic
  heartbeat self-fence.
- Background concepts-index sync (D6).
- Requirement amendments D1-D3 recorded in `docs/REQUIREMENTS.md`.

**Out of scope**

- Choosing a non-local target box in the TUI, `LISTEN`/`NOTIFY` wake-ups, worker liveness in the
  Runs view (MOD-43).
- Answering parked permission requests (MOD-42); engine-driven ACP steps still fail on their first
  request, exactly as in the TUI today.
- Installing the worker as a service (systemd unit, launchd plist, Windows service) or over SSH
  (MOD-45); this item documents a sample user unit only.
- The kill-and-restart end-to-end recovery test (MOD-24), beyond the worker being restartable.
- Fencing `write_document` from a step sink: no production caller until a `StepAuthor` exists
  (MOD-11), see D5.
- A control-plane client (MOD-47); container environments (MOD-44).

## Constraints (fixed before planning)

- **ANA-16 §7-§8 item 2 is the design.** Deviations are recorded in the plan with a reason.
- **`htui-orch` never depends on `htui-store`** (ANA-2 inv. 10); the worker-store trait lives in
  `htui-core`.
- **The worker never migrates** (`R-STO-5` as amended by MOD-40) and connects only through
  `connect_headless`.
- **No migration expected.** If planning finds one, it is `0008`, rechecked against main (and the
  host lease) before it is minted.
- **No default trait bodies**, so the compiler finds every implementor; sqlx offline data
  regenerated for changed `query!` calls (`.sqlx` against a migrated scratch DB).
- TUI behaviour unchanged when no worker runs: every existing orchestrator, store-worker and
  snapshot test stays green.
- `unsafe_code = "forbid"`, workspace lints unchanged, TDD per repo convention; reviewer is
  `rust-reviewer` (`.claude/workflow-config.json`).
- **Do not co-run** with MOD-42, MOD-37 or MOD-12 (engine, `pump`, lease and run start).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Fence completion | A stale lease holder writes nothing to its old step through any engine step write; the heartbeat's self-fence ignores wall-clock steps. | complete | [plan](../plans/mod-41-headless-worker.plan.md) |
| 2 | Supervision library | Run supervision lives in a UI-free crate behind the worker-store trait; the TUI behaves exactly as before. | in-progress | [plan](../plans/mod-41-headless-worker.plan.md) |
| 3 | `htui worker` | A headless process on a box claims, drives and recovers that box's runs; a TUI-started run on a worker box survives TUI exit. | in-progress | [plan](../plans/mod-41-headless-worker.plan.md) |
| 4 | Background index sync | The worker keeps the concepts index current without `htui --index-items`. | in-progress | [plan](../plans/mod-41-headless-worker.plan.md) |

Milestone 1 is store/orch-only and makes a second writer process safe before one exists. Milestone 2
is a behaviour-preserving move with the widest file churn. Milestone 3 is the new entry point and the
requirement amendments. Milestone 4 is separable and can be split off without stranding the others.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-29 ("all recomm"), before planning. They are decisions, not
proposals: the plan implements them.

- **D1 — `R-ID-2` amended.** Appended: "A headless `htui worker` process per box, started by the user
  and talking only to that user's Postgres, is part of `htui`; it is not a hosted service
  (`R-ORCH-12`)."
- **D2 — `R-ORCH-12` promoted from later to must now.** It keeps "requires a headless `htui` worker
  per box polling Postgres" and drops "Version one … executes only when it is the local box";
  delivery spans MOD-41 (worker) and MOD-43 (targeting). `R-LATER-4` keeps scheduling only.
- **D3 — Headless DSN: OS keyring first, then a systemd credential, plus `--dsn-stdin`.** The worker
  reads the TUI's keyring entry when a keyring is available; on a Linux host without secret-service
  it reads `$CREDENTIALS_DIRECTORY/htui-dsn` (provisioned with `systemd-creds encrypt`, encrypted at
  rest, host-bound, visible only to the unit); `htui worker --dsn-stdin` reads one line for
  foreground use. Never argv, env or a plaintext file. `R-STO-1` amended accordingly.
- **D4 — `executor: tui | worker` box setting.** In `box.settings` JSON (no migration), edited in
  Settings > Boxes, default `tui`. With `worker`, the TUI creates runs `queued` and never claims,
  adopts or sweeps on that box; the worker does all three. Flipping it never moves a live lease.
- **D5 — `write_document` from a step sink stays unfenced until MOD-11**, which introduces the first
  production `StepAuthor`; a pointer is added to MOD-11. The three engine step writes are fenced here.
- **D6 — Background index sync is milestone 4.** When a Qdrant URL is in the keyring, the worker runs
  `Indexer::sync` over every project at start and on an interval (default 15 min, `app_setting`
  key); failures are logged and never touch runs (`R-STO-8`).
- **D7 — `htui worker --pool-size N`**, default 4, clamped to 2..=8; the pool is built before any
  setting can be read.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The worker-store trait grows into a full `WriteStore` mirror (ANA-16 hazard 15) | Medium | Medium | The trait is exactly the engine's and supervisor's call set, enumerated in the plan fact-check; adding a method needs a reason in the plan |
| Moving `RunRuntime` out of the TUI crate breaks store-worker tests that reuse its fixture (`store_worker.rs:3432`) | High | Low | Milestone 2 is behaviour-preserving; the fixture moves with a `test-support` feature |
| Two executors race when `executor` flips while runs are live | Low | Medium | The flip only changes who claims new runs; live runs keep their lease owner and adoption rules |
| A systemd credential is a file on disk, against `R-STO-1`'s spirit | Medium | Medium | Encrypted at rest and host-bound; only if D3 (a) is accepted as an amendment |
| fastembed makes the worker binary heavy for boxes that never index | Medium | Low | Index sync gated on a configured Qdrant URL; the dependency already ships in `htui` |
| Engine-driven ACP steps fail on the first permission request under the worker | High | Medium | Same as today in the TUI; MOD-42 closes it; documented in the worker's help and the sample unit |

---
*Status: APPROVED at the PRD gate 2026-09-29 (D1-D7). Implementation planning follows via `plan`.*
