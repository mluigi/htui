# MOD-40 — Multi-writer store hardening

> Routed as **PRD** by `/handoff-run MOD-40` (criteria C3 and C4 fired, at threshold; accepted by the
> maintainer 2026-09-26). Ultracode recommended for the implement phase (C4). `docs/ANA-16.md` §6.1
> is the design: its table names gaps C1-C8. This document records what the tree says about closing
> them at `edf19c0`, the forks §6.1 left open, and the milestones.
> Requirements: `R-ID-3`, `R-HIS-1`, `R-STO-5` (amended 2026-09-26 by maintainer decision: "a
> headless process never migrates; it refuses and reports").

## Problem

The store was built for several writer processes and most shared writes are already a CAS or a
short row lock (ANA-16 §6.1, "What already guarantees it"). Eight spots are not. The one that
matters first is C1: a process that wakes from suspend after its run was adopted can still append
transcript rows and overwrite a step's settle columns, because step writes check only the step id.
MOD-41 (`htui worker`) puts a second writer process on every box, so these gaps become live the
moment it lands; MOD-41 is blocked on this item.

MOD-7 already closed half of C4 (box keyed on the `box.toml` id, migration `0005_box_identity`) and
made its box writers compare-and-set. This item keeps C4's heartbeat half.

## Evidence

Read at `edf19c0` (main). Paths are relative to `crates/`. **ANA-16's line numbers are stale**
(e.g. `write.rs:657-685` for `append_events`, `pg/mod.rs:449-452` for the newer-schema check); the
ones below are current.

- **Seam.** `WriteStore` starts at `htui-core/src/store/traits.rs:266`. Implementors: `PgStore`
  (`htui-store/src/pg/write.rs:666`), `MemStore` (`htui-core/src/store/mem.rs:5559`), `Writer`
  (`htui-store/src/writer.rs:351`), and two delegating test doubles (`UsageSpy`,
  `htui-agent/src/conformance.rs:711`; `SpyStore`, `htui-agent/tests/recorder.rs:391`). No default
  bodies, so every signature change is five files plus callers.
- **`StoreError`** (`htui-core/src/store/error.rs:10-42`) has `NotFound`, `Constraint`, `ReadOnly`,
  `Unreachable`, `Backend(String)`, `ParseEnum`. No lease, stale or schema variant: lease loss is
  `Ok(false)` from `refresh_lease`/`take_lease`/`release_lease`; schema mismatches are
  `Backend(String)` (`htui-store/src/error.rs:87-109`).
- **C1 — step writes are unfenced.** `append_events` (`write.rs:946-973`, `INSERT … ON CONFLICT
  (run_step_id, seq) DO NOTHING`), `set_step_usage` (`:985-1011`, `WHERE id = $1`) and `finish_step`
  (`:4072-4102`, `WHERE id = $1`). Callers:
  - The recorder (`htui-agent/src/record.rs:1042` append, `:1114` usage) holds only `store` and
    `step`; the engine builds it without an owner (`htui-orch/src/engine.rs:5274`) although it has
    `parts.owner` (`:419`).
  - The engine's `finish_step` calls (`engine.rs:1403, 2282, 3189, 3790, 4231`) all have
    `self.parts.owner`.
  - Chats (`htui/src/agent_worker.rs:3102` new, `:3107` continuing a promoted step) have no owner:
    `start_chat_run` (`write.rs:1408-1441`) inserts a run with `lease_owner` NULL, and a promoted
    step's run is parked, which releases its lease (`htui-orch/src/conformance.rs:3964`).
  - The owner is minted once per process (`htui/src/run_worker.rs:1387`) and is not a `Run` field.
  - Schema: `run.lease_owner`, `lease_expires_at` in `migrations/0003_orchestration.sql:41-43`.
    No existing case tests a stale holder's step write.
- **C8 — a short insert is silent.** `append_events` returns `rows_affected`; the recorder adds it
  to `rows` (`record.rs:1044`). A failed batch is re-offered at the same `seq` ahead of fresh rows in
  **one** call (`record.rs:1024-1050`), so a short count cannot today tell "replayed rows already
  stored" from "a second writer took my `seq`". The offline pending-buffer upload the trait docs
  mention (`traits.rs:290`, `write.rs:939`, `writer.rs:15-24`) no longer exists; those docs are stale.
- **C2 — lease times come from the caller.** `claim_run`, `refresh_lease`, `adopt_runs`,
  `take_lease`, `release_lease` all take instants (`traits.rs:959-1037`); the engine computes them
  from `Clock` (`htui-orch/src/isolate.rs:253`, `SystemClock` `:265-270`; engine uses at `:686-695`,
  `:1802-1807`, `:1850`, `:1881`, `:1961-1966`). The heartbeat keeps a self-fence at
  `until - margin` (`htui-orch/src/recover.rs:135-146`), so a DB-stamped expiry must be returned to
  it. MemStore has no clock and uses the caller's instants (`mem.rs:3965-4084`). Tests fake time
  with `TestClock` (`htui-orch/src/fake.rs:791-841`), `PausedClock`, `TokioClock`
  (`run_worker.rs:2674-2688`), and the store conformance cases pass instants directly
  (`htui-core/src/store/conformance.rs:4334, 5088, 5233, 5392` and the claim cases). Precedent for
  SQL time: `register_box` sets `last_seen_at = clock_timestamp()` (`htui-store/src/pg/mod.rs:460`).
- **C3 — quota is last-writer-wins.** `set_agent_box_quota(agent, box, quota, quota_at)`
  (`traits.rs:372-378`; Pg `write.rs:1113-1136`, zero rows → `NotFound`; MemStore `mem.rs:1587-1606`
  overwrites). One production caller, `Recorder::latch_quota` (`record.rs:1152-1179`), stamping the
  envelope's time.
- **C4 heartbeat.** `last_seen_at` is written only by `register_box`'s known branch
  (`pg/mod.rs:456-471`) and the insert default. No reader in `htui/src`. Periodic hosts exist in
  `htui/src/store_worker.rs` (reconnect ticker `:1410-1415`, sweep ticker `:1808`/`:1845`).
  `REQUIREMENTS.md:76` already says "`last_seen` updated per session".
- **C5 — schema skew.** `connect_with` (`pg/mod.rs:168-193`) returns `Connected { store, migrations:
  MigrationState::{UpToDate, Pending(n)} }`; `schema_state` (`:584-627`) refuses a dirty version, a
  newer applied version (`:602-606`) and checksum drift. The TUI asks and applies
  (`htui/src/ui/overlay/migration_prompt.rs:65-99`, `store_worker.rs:1449-1494`). The one
  non-interactive caller today, `htui/src/concepts.rs:52-62` (`--index-items`/`--search-items`),
  already refuses on pending migrations with its own message. `HTUI_VERSION` is
  `CARGO_PKG_VERSION` (`pg/mod.rs:54`), written to `box.htui_version` at insert and by the probe,
  compared only as a re-probe trigger (`htui-core/src/model/box_.rs:141-145`). No target version
  exists anywhere. `semver` is already a workspace dependency (`Cargo.toml:83`). `app_setting` is
  seeded via `SEEDED_SETTINGS` (`pg/mod.rs:47-50`).
- **C6 — `upsert_agent` overwrites.** `upsert_agent(&Agent)` (`traits.rs:323`; Pg `write.rs:1023-1052`
  `ON CONFLICT (id) DO UPDATE`; MemStore `mem.rs:1515-1538`). **No production caller**; tests only.
  `agent.updated_at` is trigger-stamped (`0001_init.sql:577`). The CAS precedent is `set_setting`
  (`traits.rs:851`, token `expected: DateTime<Utc>`, outcome `CasOutcome<T>` `:1974-1979`, Pg helper
  `cas_miss` `write.rs:81`).
- **C7 — box writers.** `edit_box` is a CAS on `edit_version` (`write.rs:1350-1354`), the only writer
  of `box.settings`. `record_box_probe` (`:1156-1160`) and `register_box` write machine facts, not
  settings, and are the box's own. `claim_run` reads `settings` under `FOR UPDATE` (`:3499`).
  C7 is therefore already met; it needs a pin, not code.
- **Migrations.** Highest is `0007_skill_attachments`; next would be `0008`. Pins in
  `htui-store/tests/migrations.rs` (`:81`, `:102-103`, `:494-497`, `Pending(7)` at `:877` and
  `tests/connect.rs:102, :204`). sqlx is offline (281 files in `htui-store/.sqlx/`); changed `query!`
  calls need `cargo sqlx prepare`. Nothing below needs a schema change as proposed, so **no
  migration is expected**.

## Users

- **MOD-41 (`htui worker`)**, the first second-writer process on a box, and MOD-42/43 after it
  (relay and remote dispatch, where C2 stops being harmless).
- **MOD-23** (agent registry editing), the first production caller of the C6 CAS.
- **The maintainer**, who gets a stale lease holder that cannot corrupt a transcript, and a worker
  that refuses a schema or build it does not match instead of writing through it.

## Hypothesis

If every step write names the lease it writes under, every ordered write carries its order, and a
headless process refuses a schema or build it was not made for, then a second writer process on
the same database can only lose a race cleanly, never overwrite, and MOD-41 can be built without
touching the store.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Stale holder fenced | After `adopt_runs`/`take_lease` moves a run to owner B, owner A's `append_events`, `set_step_usage` and `finish_step` on that run's step write nothing and return the fence error | New conformance case on MemStore and PgStore; an orchestrator case driving a suspended engine |
| Chats unaffected | A chat step (lease NULL) and a promoted step continued by a chat still write | Existing chat tests plus a conformance case for the unleased fence |
| No silent loss | A fresh batch that inserts fewer rows than offered is an error the recorder does not retry; a replayed batch may still come up short | Recorder tests with a colliding second writer and with a replay |
| DB-stamped lease time (C2) | On Postgres every lease expiry and `started_at` written by the lease methods is `clock_timestamp()`-derived; the heartbeat's self-fence uses the returned expiry | `pg_criteria` case with a skewed caller clock; orch conformance unchanged in meaning |
| Quota ordering | An older `quota_at` never overwrites a newer one; it is not `NotFound` | Conformance case, both stores |
| Agent CAS | A stale `updated_at` token diverges as `Stale`, never overwrites | Conformance case, both stores |
| Box heartbeat | `last_seen_at` advances while a TUI stays connected | `pg_criteria` case over the heartbeat method; worker wiring test |
| Headless refusal | A headless connect with pending, newer or dirty schema, or an `htui` older than the target, returns an error and applies nothing | `migrations.rs`/`connect.rs` cases |
| C7 pinned | The only `box.settings` writer is a CAS | A case asserting `edit_box` with a stale `edit_version` diverges (if not already pinned) and a doc note on the trait |

## Scope

**In scope**

- C1: a step-write fence on `append_events`, `set_step_usage` and `finish_step`, threaded through
  `Recorder` (new/continuing) and the engine's `finish_step` calls; chats pass the unleased fence.
- C8: the recorder flushes re-offered rows as a replay call separate from fresh rows; a short
  fresh insert is a non-retried error. Stale "offline buffer" docs corrected.
- C2: lease methods take durations; Postgres stamps with `clock_timestamp()`; the stamped expiry
  comes back to the caller; MemStore gets an injectable clock (D2).
- C3: `quota_at` ordering guard on both stores, a lost ordering race told apart from `NotFound`.
- C6: `upsert_agent` split into create and an `updated_at` CAS update returning `CasOutcome`.
- C7: a pin and a trait-doc invariant ("every `box.settings` writer is a CAS").
- C4 heartbeat: a `touch_box` store method (`last_seen_at = clock_timestamp()`) called on a timer
  from the store worker while online.
- C5: a headless connect mode that never migrates and refuses pending/newer/dirty schema and an
  `htui` below the target version; the target version kept in `app_setting` (D3);
  `concepts.rs` moved onto it.
- `R-STO-5` amendment (done, maintainer's typed ok 2026-09-26).
- Conformance cases for every metric above on MemStore and PgStore.

**Out of scope**

- The `htui worker` binary and anything that runs headless (MOD-41).
- Using `last_seen_at` for liveness or showing it in the UI (ANA-2 rejects it for dead runs,
  `docs/ANA-2.md:1302`).
- Agent editing UI (MOD-23); the C6 CAS is the seam it will call.
- Connection-count limits and PgBouncer (§6.1 "Connections"); a server (MOD-47).
- The pre-existing MemStore project-delete gap (`item_link.proposed_by_step_id`).

## Constraints (fixed before planning)

- **ANA-16 §6.1 is the design.** Deviations are recorded in the plan with a reason.
- **One statement per write, READ COMMITTED**, as today (`write.rs:4-11`): the fence is a predicate
  joined through `run` in the same statement, not a separate read.
- **No migration expected.** If planning finds one, it is `0008`, rechecked against main before it
  is minted, and the pins in `migrations.rs`/`connect.rs` are updated, not deleted.
- **Forward-only, sqlx offline data regenerated.**
- **No default trait bodies**, so the compiler finds every implementor.
- `unsafe_code = "forbid"`, workspace lints unchanged, TDD per repo convention; reviewer is
  `rust-reviewer` (`.claude/workflow-config.json`).
- **Do not co-run** with MOD-37, MOD-42, MOD-12 or MOD-23 (shared step writes, lease, engine,
  `upsert_agent`).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Step fence | C1 and C8: a stale lease holder writes nothing to its old step, chats still write, and a short fresh insert is loud. | in-progress | [plan](../plans/mod-40-multi-writer-hardening.plan.md) |
| 2 | Ordered writes | C3 quota ordering, C6 agent CAS, C7 pinned. | pending | [plan](../plans/mod-40-multi-writer-hardening.plan.md) |
| 3 | Box and schema | C4 heartbeat and C5 headless connect with the target version. | pending | [plan](../plans/mod-40-multi-writer-hardening.plan.md) |
| 4 | Database time | C2: lease times stamped by Postgres, heartbeat fence on the returned expiry, MemStore clock. | pending | [plan](../plans/mod-40-multi-writer-hardening.plan.md) |

Milestone 1 is what MOD-41 most needs and touches the recorder and engine. Milestones 2 and 3 are
store-only plus one timer. Milestone 4 has the widest test churn (every lease conformance case),
so it goes last and can be split off without stranding the others.

## Decisions taken at the PRD gate

Answered by the maintainer on 2026-09-26 ("all recomm"), before planning. They are decisions, not
proposals: the plan implements them.

- **D1 — Step writes carry an owner fence.** A `StepFence` argument on `append_events`,
  `set_step_usage` and `finish_step`: `Lease(owner)` for the engine, `Unleased` for chats. The
  predicate is `run.lease_owner = $owner` or `run.lease_owner IS NULL`, joined through `run` in the
  same statement; a miss writes nothing and is a new `StoreError::Fenced`.
- **D2 — C2 lands now, as milestone 4.** Lease methods take durations, Postgres stamps with
  `clock_timestamp()` and returns what it wrote, and MemStore takes a clock; the `Clock` trait moves
  from `htui-orch` to `htui-core` so tests share one `TestClock`.
- **D3 — Below the target version, a TUI warns and a headless process refuses.** A TUI that applies
  migrations raises `app_setting` `htui.target_version` to its own version and never lowers it.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A chat continuing a promoted step is fenced once an answer takes the lease | Medium | Medium | Conformance case for the hand-over; whether a chat can still be writing when accept takes the lease is checked in the plan fact-check (inferred, not verified) |
| C2's clock injection churns every lease test | High | Medium | Milestone 4 last and separable (D2); fan-out by file set |
| The recorder's new non-retried error leaves a step half-recorded | Low | Medium | Only reachable with a second writer on the same `seq`, which C1 already prevents; the error names it |
| Wide mechanical change across five implementors and the sqlx data | Medium | Low | No default bodies; `cargo sqlx prepare` in the plan's final task |
| Luigi runs one of the do-not-co-run items off-project | Low | High | Re-check main and open PRs before each milestone and before pushing |
