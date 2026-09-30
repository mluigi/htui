# MOD-65 - Every thread and task the agent runtime starts opens the contain window (done, 2026-09-30)

**Requirements:** `R-NF-3`, `R-TUI-8`.
**Origin:** MOD-53's "Not done" (`docs/decisions/mod/mod-53.md`). Found 2026-09-29.
**Artifacts:** plan with its verified-claims table and post-review amendments:
[`.claude/plans/mod-65-contained-spawns.plan.md`](../../../.claude/plans/mod-65-contained-spawns.plan.md);
blueprint:
[`.claude/plans/mod-65-contained-spawns.blueprint.md`](../../../.claude/plans/mod-65-contained-spawns.blueprint.md).
No PRD. Routed as a plan on 2026-09-30 with 1 of C1-C4 fired (C3). The maintainer widened the scope
at planning, from the two named blocking threads to every thread and task the crate starts, plus a
lint guard.
**Commits:** `acb0745` plan, `bc2c0d6` blueprint, `607f1dc` helpers (T1), `f0e186f` the 21 sites
(T2), `65d2da5` the lint (T3), `6b957e5` the `answering` doc (T4), then review fixes `8f6fe20`,
`799ba13`, `baf13ac`, `600e508`, `e04db99`, `46345e2`, then this write-up.

## The defect, as it really was

MOD-53 polls each runtime task inside `excerpt::contain` + `catch_unwind`, so the panic hook
(`terminal::install_panic_hook`) leaves the terminal alone for a panic the runtime survives. The
`CONTAINED` flag is a thread-local, though, so the window covered only the thread polling the task
and only during that poll. Anything the task started ran on another thread with the flag false:

- **12 blocking threads**: the item's two (the install unpack, `SystemHardware::read`'s
  `system_facts`) and ten more: the PCI walk, `which` in `launch::spawn`, `available_space`, three
  in `acp/fs`, two in `probe`, and the two excerpt passes.
- **9 async tasks** the item did not name: the ACP and CLI `run_session`s, the handshake, the auth
  flow, the stderr reader, `read_lines`, the browser reaper, and the two box-probe `JoinSet`s.

tokio catches a panic on any of them and hands it back as a `JoinError` that every caller survives.
The hook still ran first, uncontained, and gave the terminal back under the live event loop.

## What shipped

- **`htui_agent::contained`**, a public module with `spawn_blocking`, `spawn` and
  `spawn_in(&mut JoinSet, fut)`. They mirror tokio's signatures and return tokio's handles.
  - `spawn_blocking` runs its closure inside `contain`.
  - The async helpers poll the future inside `contain` **on every poll** (`Box::pin` +
    `poll_fn`, the same shape as `answering`), so no window stays open across a `Pending`.
  - `#[track_caller]` keeps tokio's spawn location at the real call site.
  - `#[cfg(panic = "abort")] compile_error!` protects the soundness argument below.
- **All 21 sites** changed by one path each, and their `JoinError` handling is byte-identical.
- **The guard: `crates/htui-agent/clippy.toml`**, with `msrv` copied from the root file, because a
  crate-level file replaces the root one; the root file now points back at it.
  - The file sets `disallowed-methods` on every tokio spawn form: `task::spawn`/`spawn_blocking`/
    `spawn_local`, `JoinSet::spawn`/`spawn_blocking`/`spawn_on`/`spawn_blocking_on`/`spawn_local`/
    `spawn_local_on`, `LocalSet::spawn_local`, and `Handle::` and `Runtime::spawn`/`spawn_blocking`.
  - It also covers every `std::thread` spawn form.
  - The helpers' own raw calls and the excerpt provider thread, which is contained by
    `propose_caught`, carry reasoned `#[expect]`s.
  - Six test files whose fake agents spawn carry a file-level `#![expect]`. `acp_driver.rs` carries
    one on each of its two `#[cfg(unix)]` tests instead.
  - Workspace clippy runs with `-D warnings`, so a new raw spawn in this crate fails the gate.
- The `answering` doc in `crates/htui/src/agent_worker.rs` now names the contained spawns.

## Decisions worth keeping

**Contain at the spawn, unconditionally (D1).** Opening the window is sound wherever tokio is the
catcher, and tokio always catches a panic in a spawned task or blocking closure.
- An awaiter that re-raises with `.expect` starts a **new** panic, so the hook runs again on the
  awaiter's thread and asks that thread's own window.
- `resume_unwind` is the one re-raise that skips the hook, and the tree has none.

So no call site had to be judged on its own.

**The window opens per poll, not per task (D4).** A future moves between workers, and a window left
open across a `Pending` would vouch for another task's panic on that thread.

**A crate-local lint, not a workspace one (D5).** The other crates' spawns are not started by the
agent runtime, and their own tasks are not polled inside any window. Containing their threads alone
would change nothing.

**`std::thread` is linted too (D7, reversed at review).** A reasoned `#[expect]` on the one
contained site costs less than an unguarded gap.

**`#[expect]`, never `#[allow]`.** Once a file stops spawning, its unfulfilled expectation fails
clippy, so no exemption outlives its reason.

## Tests

`crates/htui-agent/tests/contained_spawn.rs` is its own binary with one `#[test]`, because the panic
hook is process state (the same reason as `crates/htui/tests/panic_hook.rs`). A recorder hook
stores each panic's message, its `panic_is_contained()` and its thread, and forwards any panic it
does not expect to the previous hook.

1. A contained blocking panic records `true`.
2. A contained task panics on its **second** poll and records `true`.
3. A contained `JoinSet` task records `true`.
4. A raw blocking panic records `false`. This is the control. The pool has one blocking thread,
   kept alive for an hour, and the test asserts case 4 ran on case 1's thread, which proves the
   window closed again on a reused thread after a panic.
5. On a current-thread runtime, a raw task that runs while a contained task is `Pending` sees the
   window closed.
6. Every helper passes its value through.

**Red first:** with pass-through stubs, cases 1-3 recorded `false` and the test failed.
**Mutation check:** putting one site (`hardware.rs:68`) back to raw `spawn_blocking` fails clippy
with the configured reason.
**Gates:** `cargo fmt --check`, workspace clippy `-D warnings`, and
`cargo test --workspace --all-features -- --test-threads=1` all passed, the suite at 2708 passed,
0 failed.

## Review

The configured `rust-reviewer` found 3 medium and 5 low findings, all applied; a re-review approved.
- Medium:
  - `acp_driver.rs`'s file-level expect would go unmet on non-unix targets.
  - The lint missed eight tokio forms and `std::thread`.
  - The module doc called the `Drop`-abort case "unchanged".
- Low:
  - Case 4's thread reuse was left to luck.
  - An unexpected panic's message was swallowed.
  - The root `clippy.toml` had no pointer to the crate copy.
  - Nothing guarded against `panic = "abort"`.
  - Two stale doc lines.

The re-review's two lows (the keep-alive, and recording the amendments in the plan) went in with
`46345e2`.

## Not done

- **Widened by this item:** a `Drop` that panics **during** an unwind aborts the process. The first
  hook now skips the restore at all 21 sites, not only in `answering` tasks, so the abort leaves the
  terminal raw. Before MOD-65 a panic at those sites restored the terminal first.
- **Deferred by the maintainer: the cancel drop.** tokio drops a cancelled task's future (abort,
  runtime shutdown) outside the window, so a `Drop` that panics on cancel is survived by tokio but
  still gives the terminal back.
  - A fix would be a `Drop` on the wrapper that drops the future inside `contain` unless
    `std::thread::panicking()`.
  - Getting that branch wrong would turn a poll panic into an abort with the terminal raw.
- `JoinSet`'s `Extend`/`FromIterator` spawn inside tokio, where no lint sees them. The module doc
  forbids them; nothing enforces it.
- Spawns in `htui`'s store, concepts and run workers, `htui-store` and `htui-orch` are untouched.
  Their tasks are not polled inside any window, so a panic there still gives the terminal back.
  That is a question for each worker; nothing has been filed for it.
- The chained default hook still prints a contained panic's message to stderr under the TUI, as it
  does for provider and runtime-task panics (MOD-56 behaviour).
- `acp_driver.rs`'s non-unix clippy path is correct by construction, but no non-unix target was
  built here.
