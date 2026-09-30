# MOD-41 - Headless worker (`htui worker`) (done, 2026-09-30)

**Requirements:** `R-ORCH-12` (promoted from later to must), `R-ID-2` and `R-STO-1` (both amended
2026-09-29 by maintainer decision), `R-NF-1`, `R-NF-2`, `R-NF-3`, `R-STO-5` (as amended by MOD-40),
`R-STO-8`.
**Origin:** ANA-16 (`docs/ANA-16.md` §7, §8 item 2). It carried MOD-40's unfenced writes and
monotonic fence, MOD-4 M6's library and worker-store asks, and MOD-34's background index sync.
**Artifacts:** PRD
[`.claude/prds/mod-41-headless-worker.prd.md`](../../../.claude/prds/mod-41-headless-worker.prd.md),
plan
[`.claude/plans/mod-41-headless-worker.plan.md`](../../../.claude/plans/mod-41-headless-worker.plan.md)
(fact-checked: 22 claims amended before CONFIRM) and blueprint
`.claude/plans/mod-41-headless-worker.blueprint.md` (hazards F-1..F-37). User guide
[`docs/htui-worker.md`](../../htui-worker.md). Routed as a PRD (C2, C3, C4 fired), run in the
TOOL-7 sandbox on `hr/MOD-41`.
**Decisions:** maintainer, 2026-09-29/30: PRD gate D1-D7 ("all recomm", requirement amendments
applied in `docs/REQUIREMENTS.md`), plan confirmed with OQ-1..OQ-6 as recommended, blueprint
escalations E-1 and E-2 accepted, the demo-only exception to OQ-5 (T9-C5) accepted, review
dispositions as recommended.
**Commits:** `955de97`..`2b4490a` on `hr/MOD-41` (four milestones, a review round, and the
close-out commit). No migration: the next one is still `0008`.

## What shipped

**Milestone 1, fence completion.** `set_step_prompt`, `upsert_step_tree` and `record_commits` take
a `StepFence` on both stores (Postgres: the `set_step_usage` predicate, and a `step_fence` read
`FOR SHARE OF r` inside the batch transactions); every engine call site passes the process's own
lease. The lease heartbeat fences on tokio's monotonic clock, so an NTP step or a suspend no longer
moves it (MOD-40 blueprint F-38).

**Milestone 2, supervision library.** Run supervision (`RunRuntime`, its tasks, claim, adopt,
sweep) moved out of the TUI crate into `crates/htui-worker`, which links no `ratatui` or
`crossterm` (pinned by `tests/deps.rs`). It reaches the store only through three `htui-core`
traits (`store/worker.rs`): `RecorderStore` (3 methods), `WorkerStore` (42, exactly the engine's
call set) and `WorkerHost` (22 reads plus `writer`), implemented explicitly for every store, never
by a blanket impl (its futures could not be proven `Send`). The engine, `gate` and `graph::resolve`
are generic over `WorkerStore`. Replies go through a `ReplySink`; the TUI keeps every path through
aliases and re-exports.

**Milestone 3, `htui worker`.**
- `box.settings.executor` (`tui` default, `worker`), written only by `edit_box` under its
  compare-and-set and edited in Settings > Boxes (`w`). Invariant I-1: only the process whose role
  matches claims, adopts or sweeps on a box. The gate reads the key alone (`Executor::of`) and fails
  closed on an unknown value or a non-object blob.
- On a worker box the TUI's walking commands hand back (`Tails::HandBack`): the decision is
  recorded under the free lease of the parked run, the lease is released, and the worker's sweep
  adopts and finishes the tail through crash recovery (OQ-1, OQ-6's topology gate, a per-run
  backoff). Retry admits the next attempt first; resume only unparks.
- A queued cancel is a compare-and-set on `Queued`, then the item returns to `open` (E-2: two
  statements). Cancelling a live worker walk is refused with a sentence naming the worker until
  MOD-42 (OQ-4). The TUI claims leftover queued rows on a `tui` box (OQ-5), except over the demo
  store (T9-C5).
- `htui worker [--pool-size N] [--dsn-stdin] [--log PATH]`: headless connect, DSN from `--dsn-stdin`,
  else the keyring, else `$CREDENTIALS_DIRECTORY/htui-dsn`; pool 4 clamped to 2..=8; beats
  `box.last_seen_at` at start and logs `htui worker ready`; stops on SIGINT/SIGTERM at any point,
  startup included. Exit 0 clean, 2 startup refusal (not sent to Sentry, E-1), 101 panic; 1 is
  reserved. `main` bounds runtime teardown with `shutdown_timeout(5 s)`, on the panic path too.
- The Backlog tab re-reads an active run's `Runs` every 5 s (OQ-3; MOD-43 adds `NOTIFY`).

**Milestone 4, background index sync.** With a Qdrant URL in the keyring the worker re-syncs the
concepts index at start and every `concepts_sync_minutes` (default 15); failures are a `warn` and
never touch runs.

**Review gate.** `rust-reviewer` asked for changes (1 HIGH, 1 MEDIUM, 7 LOW); one verifier per
finding refuted R-7 and narrowed R-2 and R-9. Fixed: the worker refreshes its repo map and parts
once per sweep when no walk is live (R-1, HIGH: it had frozen them at its first sweep), a panicking
run steps its backoff (R-2), the TUI skips the isolator rebuild when it hands back (R-6), stale
backoff entries are pruned (R-8), bounded teardown (R-4), docs (R-3, R-5, R-9).

## Decisions worth keeping

**Explicit trait impls, bounds by path.** A blanket `impl<T: WriteStore> WorkerStore for T` cannot
prove its futures `Send` (probe P-1), and a type or type parameter that sees both trait families
hits `E0034`. So no module `use`s the new traits and no parameter is bounded on both.

**I-1 is a safety invariant, not a convenience.** A TUI sweep adopts any lapsed run on its box
whichever process owned it, and a parked run holds no lease, so without the gate the two processes
walk each other's runs. Correctness still rests on the lease compare-and-set and the fences; I-1
makes ownership single.

**A hand-back is a crash-path resume (OQ-6).** The adopter passes the live-graph topology gate, so a
run whose graph changed mid-run parks where the TUI would have walked on its snapshot.

**The systemd sample is a system unit with `User=`.** User-scoped encrypted credentials need
systemd 256; Ubuntu 24.04 ships 255 (fact-check F-21).

## Left open

- A worker's limit edits (`command_limits`, `copy_max_total_bytes`) alone reach it at its next
  restart; a repo or checkout change is picked up at the next sweep with no walk live. A claim that
  was refused and queued in the process can still be retried with the parts read before a map move
  while another walk is live (documented, `docs/htui-worker.md`).
- Git worktree admin locks are per process; on a worker box the TUI's `c`/`T` cleanup is not
  serialised against the worker's `worktree add` on the same repository (MOD-4 D70's residual,
  documented). A cross-process `flock` was deferred at the review gate.
- The sink's `write_document` stays unfenced until MOD-11 introduces the first production author
  (PRD D5); a pointer is on MOD-11.
- Engine-driven ACP steps still fail on their first permission request (MOD-42). Remote targeting
  and worker liveness in the Runs view are MOD-43; installing the service is MOD-45.
- Under load the sandbox's Qdrant times out (`qdrant_live`, `qdrant_worker`); each passed alone.

## Pins after this item

The store conformance suite has 103 `CASES` (was 96) and 14 `READ_CASES`. `htui-orch` has 85
`CASES` (was 73). `crates/htui-store/.sqlx` has 290 files (was 288); `crates/htui/tests/snapshots`
has 108 (was 107); the workspace has six members (`htui-worker` new). `StoreRequest` 85,
`StoreReply` 47 and `GraphSource` 7 are unchanged. `cargo doc --workspace --no-deps --keep-going
--all-features` shows ten pre-existing link errors (HANDOFF's "six" was stale). The full workspace
suite passed 2679 and failed 0, with 26 ignored, against the sandbox Postgres.
