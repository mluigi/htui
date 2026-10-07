# MOD-78 - `command_run` lifecycle: lease check and cancel on session end (done, 2026-10-07)

**Requirements:** `R-MCP-1`, `R-MCP-3`.
**Origin:** MOD-11 (`docs/decisions/mod/mod-11.md`), I-3 ("every item write", `command_run` unfenced) and review L5.
**Artifacts:**
- plan [`.claude/plans/mod-78-command-run-lease.plan.md`](../../../.claude/plans/mod-78-command-run-lease.plan.md):
  R1-R4, D1-D7, verified-claims table;
- blueprint `.claude/plans/mod-78-command-run-lease.blueprint.md`: signatures, SQL, test setups, hazards B-1..B-12.

Decision numbers are local to MOD-78 (the MOD-31 convention).

Routed as **plan** (C3 only, weak). Run in a TOOL-7 sandbox (`hr/MOD-78`). T1 (store read) ran on the branch and T2
(session token) in a worktree in parallel. T2 had to be isolated because it builds `htui-mcp`, which does not compile
while T1 is half-way through adding a trait method. T3 (`command_run`) followed T1, and T4 (docs) ran beside T3.

## The problem

`command_run` was the one htui tool whose writes took no fence. A session whose walk had lost its lease could still
queue a command, run it and record it until the walk noticed the loss at its next lease renewal (default TTL 120 s).
Dropping the session's `ToolLease` only set `Session.ended`, and that flag was read only at the start of a call. A
`command_run` already queued or running therefore outlived its session, as long as the agent kept the connection
open.

## What was built

- **`lease_holds(run, fence) -> Result<bool>`** on `WriteStore`, forwarded through `WorkerStore`, `Writer`,
  `UsageSpy` and `SpyStore`.
  - Postgres reads `SELECT lease_owner IS NOT DISTINCT FROM $2 AS "holds!" FROM run WHERE id = $1` on the pool. It
    takes no `FOR SHARE`, so it is lock-free.
  - Mem uses `State::fence_holds`'s predicate.
  - A missing run is `NotFound { entity: "run" }`.
  - It checks the owner only, never the expiry (D2): an expired lease that nobody has taken still writes, as every
    other fence allows.
  - New conformance case `lease_holds_reads_the_owner` (149 cases), plus `MemFault::LeaseHolds` for tests.
- **`command_run` reads the session's lease (D3, D4):**
  - (a) before it queues: a loss answers `fenced: lease lost` and writes nothing;
  - (d) while queued, at most once per `COMMAND_HEARTBEAT`;
  - (b) once at admission, before the shell starts;
  - (c) on every running heartbeat. A loss here stops `run_shell`, which kills the process group, then finishes the
    row `cancelled` with `[stopped: fenced: lease lost]` before the scrubbed tail, and answers `fenced: lease lost`.
  - A read that fails or takes longer than a heartbeat stops nothing at (b), (c) or (d) (D5, blueprint B-6, R1 L2).
    Only (a) refuses, answering `store unavailable: …`.
- **A session's end ends its in-flight calls (D6, D7):**
  - A `tokio_util::sync::CancellationToken` replaces `Session.ended`.
  - `Served::call` races `tools::dispatch` against it in a `biased` select.
  - The token is cancelled by the lease drop, by `McpHost::close`, and by a new `Drop for Inner` (the last `McpHost`
    is gone).
  - The dropped call cleans up after itself through the paths that already existed: the armed `Enqueued` guard
    cancels the row, the `command_slot` permit returns, and `run_shell`'s `kill_on_drop` and `GroupGuard` kill the
    group.
- `docs/htui-mcp.md`: the Scope, Liveness, prompt and troubleshooting sections describe the above. The docs no longer
  say "`command_run` is not fenced".
- One unrelated rustdoc fix: `channel.rs`'s public `relay` doc no longer links the private `DRAIN_WITHOUT_HALF_CLOSE`.

No migration. One new `.sqlx` entry. `htui-mcp` gains `tokio-util` (already a workspace dependency).

## Decisions (maintainer, 2026-10-06)

- D1-D7 as in the plan, confirmed at CONFIRM.
- **B-6:** only the read before the enqueue refuses on a store error. A queued or running command is never stopped by a
  failed read, because the claim's own heartbeat and reaper already cover a lasting outage.
- **Behaviour change accepted with R1 (M1, L1):**
  - A call still running when its session ends answers `session ended`, even if its write already committed. The docs
    now say that call "may or may not have written".
  - A `permission_prompt` still pending at the session's end gets that error rather than a `deny`. The CLI is being
    stopped at that point.

## Plan claims corrected by the blueprint

- **A second case-count pin.** `crates/htui-core/tests/mem_store.rs` pins the conformance count too; the plan's file
  table missed it (B-1).
- **T3's tests split in two (B-2).** Paused time cannot drive `run_shell`'s timers beside a real child: tokio advances
  the clock while the child runs, so the timeout fires at once. The end-to-end tests live in
  `tests/tools_command.rs` on real time, and the unit tests in `command.rs` use paused time.
- **T2 ∥ T3 is unsafe on one tree (B-4)** despite their disjoint file sets, since both build `htui-mcp`.

## Commits

- plan, blueprint: `aaf794a0`, `454fd0d9`;
- T1 `lease_holds`: `0a3ccf6f`, `4daa0473`;
- T2 session token: `63e989b1`, `71c50438`, `b02e1892`, merged `629b49e6`;
- T3 `command_run` checks: `53574fde`, `1b203c7d`;
- T4 docs: `c50ebe51`; rustdoc link: `1e6b4eb8`;
- review R1: M1, L1, L3 `744e13e8`; M2 `35f36cf0`; L2 and NITs `2266d016`.

## Verification

- **Full suite, serial.** `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` after T1-T4:
  5078 passed, 0 failed, no SIGABRT, no orphaned processes. It was re-run on the final tree after R1.
- **Postgres conformance.** `pg_conformance` passed against the sandbox Postgres, both 149-case runs included.
- **New tests:**
  - host: an in-flight call ends on lease drop, on `close()`, and with the last host. All three were red first: the
    call hung in `admit`.
  - `command.rs` (paused time): a waiter stops within two heartbeats and not before one; a lost lease stops `beat`; a
    failed read does not.
  - `tools_command.rs`: a lost lease refuses and queues nothing; a lease lost while running kills the child (checked
    with `pgrep`) and cancels the row with the note; a lease lost while queued cancels a row that never ran. These
    passed 3/3 runs, about 10 s each.
- **Lint.** `cargo clippy --workspace` passes with and without `--all-targets --all-features` (`-D warnings`);
  `cargo fmt` passes.
- **Docs.** `cargo doc -D warnings` passes for `htui-mcp` and `htui-core`.
- **Review.** `rust-reviewer` approved with fixes; R1 applied every finding.

## Carried

- **The R1 L2 timeout has no test.** `MemFault` cannot make a call hang, and adding a hanging fault kind was not worth
  it for one `timeout` wrapper.
- **`cargo doc --workspace -D warnings` fails on old `htui-store` debt.** There are 14 private or broken intra-doc
  links, `pg/write.rs:1310` through `:6791`, plus an unresolved `ensure_model`, all older than MOD-78. Not filed; the
  maintainer decides whether it becomes an item.
- **The host must still run the real Postgres gate on the merged tree.**
