# Plan: MOD-65 - Every thread and task the agent runtime starts opens the contain window

**Status: CONFIRMED by the maintainer 2026-09-30; implemented and closed out 2026-09-30 (`docs/decisions/mod/mod-65.md`). Amendments after review at the end.**

**Source**: `HANDOFF.md:288-295` (MOD-65, from MOD-53, found 2026-09-29). Requirements `R-NF-3`,
`R-TUI-8`. Design authority: MOD-53 (`docs/decisions/mod/mod-53.md`, "Not done", bullet 2) and
MOD-56's hook question (`crates/htui/src/terminal.rs:71-97`).

**Routing**: routed as plan by `/handoff-run MOD-65` (1 of C1-C4 fired: C3, the item named two
fixes). Ultracode not needed. **Scope widened by the maintainer at planning (2026-09-30):** the fix
covers all async tasks as well as blocking threads (D2), and a lint guard is added (D5). That makes
the breadth about 24 files, which would fire C4 if the item were routed again. It stays on plan:
every call-site change is a one-token swap behind one helper, and a PRD would add no decision.
Staffing: session model (Opus 5.5) for every agent, no `model:` override.

**Base**: `hr/MOD-65` @ `7ce642a`. No migration, no `.sqlx` entry, no new dependency.

**Numbering**: MOD-65's own plan. Decisions **D1…D7**, tasks **T1…T4**.

---

## The defect

MOD-53 made the runtime poll each task inside `htui_agent::excerpt::contain` + `catch_unwind`
(`answering`, `crates/htui/src/agent_worker.rs:3274-3300`). The `CONTAINED` flag is a
**thread-local** (`excerpt.rs:690-711`). So the window covers only the thread that is polling the
runtime task, and only during its poll. Anything that task starts is polled or run on **another**
thread, where `panic_is_contained()` is false. A panic there makes the process hook
(`terminal.rs:71-79`) call `ratatui::restore()` under the live event loop, even though tokio catches
the panic and hands it back as a `JoinError` that the caller survives. The result is the same
wedged-UI shape MOD-56 and MOD-53 closed.

There are two classes of such threads in `htui-agent/src`, 21 sites in all (the clippy probe below
lists them all):

- **Blocking threads (12 `tokio::task::spawn_blocking`)**: `launch.rs:1031` (`which`),
  `box_probe/hardware.rs:68` (`system_facts`, named in the item), `:235` (PCI walk),
  `install/run.rs:368` (unpack, named in the item), `install/plan.rs:274` (`available_space`),
  `acp/fs.rs:86`, `:179`, `:210`, `probe.rs:497`, `:861`, `excerpt.rs:1104`, `:1158`.
- **Async tasks (9: 7 `tokio::spawn`, 2 `JoinSet::spawn`)**, not named in the item but the same
  hole: `launch.rs:1053` (stderr reader), `acp/handshake.rs:106`, `acp/mod.rs:774` (the ACP
  `run_session`), `acp/auth.rs:199`, `auth/browser.rs:126` (reaper), `cli/mod.rs:705` (the CLI
  `run_session`), `cli/mod.rs:781` (`read_lines`), `box_probe/mod.rs:119`, `:223`. An ACP session
  task that panics while decoding tears the terminal down today.

---

## Design decisions

| # | Decision | Why |
|---|---|---|
| D1 | **Contain at the spawn, unconditionally.** Every thread and task that `htui-agent` starts runs its closure, or each poll of its future, inside `excerpt::contain`. | Opening the window is sound wherever tokio is the catcher, and tokio always catches: a `spawn_blocking` closure or a spawned future that panics becomes a `JoinError`, and the process lives. If an awaiter re-raises with `.expect`/`unwrap`, that is a **new** panic: the hook runs again on the awaiter's thread and asks that thread's own window, so no fatal panic loses its restore. The one re-raise that skips the hook is `resume_unwind`, and there is none in the tree (verified). So no call site needs judging one at a time. |
| D2 | **Both classes, blocking threads and async tasks** (maintainer, 2026-09-30). | The same thread-local hole with the same cure. Fixing only blocking threads would leave the ACP/CLI session tasks, the likeliest place for a decode panic, still tearing the terminal down. |
| D3 | **One new public module, `htui_agent::contained`**, with three functions that mirror tokio's signatures and return tokio's own handles: `spawn_blocking(f) -> JoinHandle<R>` (runs `contain(f)`), `spawn(fut) -> JoinHandle<T>` (polls `fut` inside `contain` on every poll), and `spawn_in(&mut JoinSet<T>, fut) -> AbortHandle` (the same, on a set). | Call sites change by one path segment, and the `.await` / `JoinError` handling at each site stays byte-for-byte the same, so every site keeps its current error behaviour. The helpers are public so `htui` could use them later. |
| D4 | **The async wrapper opens the window per poll, not per task**: `Box::pin(fut)` driven by `std::future::poll_fn(\|cx\| contain(\|\| fut.as_mut().poll(cx)))`, the same shape as `answering`. | A future is polled on whichever worker picks it up, and a window left open across a `Pending` would vouch for a different task's panic on that thread. `contain` restores the previous value, so the window nests under `answering` when both apply. `Box::pin` rather than a pin-projected struct: `unsafe_code = "forbid"` and no `pin-project` dependency; one allocation per spawn is negligible next to a process or an ACP session. |
| D5 | **Guard with a crate-local `crates/htui-agent/clippy.toml`**: `msrv = "1.98"` (copied from the root file, because a crate-level `clippy.toml` **replaces** the root one rather than merging) plus `disallowed-methods` for `tokio::task::spawn`, `tokio::task::spawn_blocking`, `tokio::task::JoinSet::spawn`, `tokio::task::JoinSet::spawn_blocking`, `tokio::runtime::Handle::spawn` and `tokio::runtime::Handle::spawn_blocking`, each with a reason naming `crate::contained`. Workspace clippy runs with `-D warnings`, so a new raw spawn in this crate fails the gate. | Twelve sites grew with nobody noticing, and the next one would reopen the hole silently. The lint is scoped to this crate on purpose: the other crates' spawns (`htui`'s store worker, `htui-store`, `htui-orch`) are not started by the agent runtime and are outside MOD-65. |
| D6 | **The helpers carry `#[expect(clippy::disallowed_methods, reason = …)]` on their one raw call each; the 7 integration-test files that spawn scripted fake agents carry a file-level `#![expect(clippy::disallowed_methods, reason = "test fakes run under no TUI panic hook")]`.** | The test fakes play the *agent's* side and never run under the hook, so converting them would claim a meaning they lack. `expect`, not `allow`: once a file stops spawning, the unfulfilled expectation fails clippy, so the attribute cannot outlive its reason. The repo already uses `#[expect(…, reason = …)]` (`tests/recorder.rs:263`). |
| D7 | **`std::thread` is out**: the one raw OS thread in the crate, the excerpt provider thread (`excerpt.rs:807`), already runs `propose_caught`, which opens the window itself. It is not added to the lint. | It is already contained, and a lint on `std::thread::Builder::spawn` would flag the one correct site. |

---

## Files to change

| File | Task | Action |
|---|---|---|
| `crates/htui-agent/src/contained.rs` | T1 | new: `spawn_blocking`, `spawn`, `spawn_in`, the module doc (D1 argument) |
| `crates/htui-agent/src/lib.rs` | T1 | `pub mod contained;` + one crate-doc paragraph |
| `crates/htui-agent/tests/contained_spawn.rs` | T1 | new test binary, one `#[test]` (process hook, see Test plan) |
| `crates/htui-agent/src/launch.rs` | T2 | `:1031` blocking, `:1053` async |
| `crates/htui-agent/src/box_probe/hardware.rs` | T2 | `:68`, `:235` |
| `crates/htui-agent/src/box_probe/mod.rs` | T2 | `:119`, `:223` (`spawn_in`) |
| `crates/htui-agent/src/install/run.rs` | T2 | `:368` (+ the `:47` module-doc line) |
| `crates/htui-agent/src/install/plan.rs` | T2 | `:274` |
| `crates/htui-agent/src/acp/fs.rs` | T2 | `:86`, `:179`, `:210` |
| `crates/htui-agent/src/acp/mod.rs` | T2 | `:774` |
| `crates/htui-agent/src/acp/handshake.rs` | T2 | `:106` |
| `crates/htui-agent/src/acp/auth.rs` | T2 | `:199` |
| `crates/htui-agent/src/auth/browser.rs` | T2 | `:126` |
| `crates/htui-agent/src/cli/mod.rs` | T2 | `:705`, `:781` |
| `crates/htui-agent/src/probe.rs` | T2 | `:497`, `:861` |
| `crates/htui-agent/src/excerpt.rs` | T2 | `:1104`, `:1158` |
| `crates/htui-agent/clippy.toml` | T3 | new (D5) |
| `crates/htui-agent/tests/{acp_conformance,acp_driver,auth,cli_conformance,driver_contract,install,probe}.rs` | T3 | file-level `#![expect]` (D6) |
| `crates/htui/src/agent_worker.rs` | T4 | doc only: `answering`'s last paragraph (`:3272-3273`) says a thread the task starts "is not caught here"; it now says those threads open their own window through `htui_agent::contained` |

**Not touched, on purpose:** the `htui`, `htui-store`, `htui-orch` and `htui-core` spawn sites (D5
scope); `excerpt::contain` / `panic_is_contained` / `Contained` (the new module calls `contain` and
does not move it, because `htui::terminal` asks `excerpt::panic_is_contained`); `terminal.rs`; every
call site's `JoinError` handling (D3).

---

## Tasks (serial)

The file sets are pairwise disjoint (table above), but the order is fixed: T2 needs T1's API, and
T3's lint is red until T2 lands. There is one worktree, one `target/` and one `.git/index`, so one
implementer runs T1→T4 in order and commits after each task (project memory: commit
incrementally, no stash).

**T1 - the helpers, test first.**
1. Write `tests/contained_spawn.rs` (Test plan), plus `contained.rs` with the three functions as
   **plain pass-throughs** to tokio (no `contain`). Run it: the three "contained" assertions fail and
   the control passes. That is the red.
2. Add `contain` (D3, D4). The test passes. Commit.

**T2 - migrate the 21 sites.** Replace each raw call with `crate::contained::{spawn_blocking, spawn,
spawn_in}` and change nothing else at the site. Update the `install/run.rs:47` doc line that names
`spawn_blocking`. The existing suites are the regression net: `cargo test -p htui-agent
--all-features`. Commit.

**T3 - the guard.** Add `crates/htui-agent/clippy.toml` (D5) and the 7 test-file `#![expect]`s
(D6), plus the helpers' own `#[expect]`s if T1 did not add them already. Then run
`cargo clippy -p htui-agent --all-features --all-targets -- -D warnings`, which must be clean.
**Mutation check:** temporarily revert one T2 site to `tokio::task::spawn_blocking`; clippy must
fail naming it. Revert the mutation. Commit.

**T4 - the doc line in `agent_worker.rs`** (above). Commit.

**Gates (run by the main thread on the real tree, not trusted from the implementer):**
`cargo fmt --all -- --check`; `cargo clippy --workspace --all-features --all-targets -- -D warnings`;
`cargo test --workspace --all-features -- --test-threads=1` (project memory: the strict gate).

---

## Test plan

`crates/htui-agent/tests/contained_spawn.rs` is its own binary with **one** `#[test]`, because
`set_hook` is process state (the precedent is `crates/htui/tests/panic_hook.rs`, same reason). It
installs a hook that records `panic_is_contained()` for every panic in a `Mutex<Vec<(String, bool)>>`
keyed by the panic message, and it builds its own tokio runtimes. Cases:

1. `contained::spawn_blocking(|| panic!("blocking"))`: the `JoinError` is a panic, and the hook
   recorded `true`.
2. `contained::spawn(async { yield_now().await; panic!("task") })`: the panic comes on the
   **second** poll, so the case proves the window reopens on each poll and was not opened only
   once. `JoinError::is_panic`, and the hook recorded `true`.
3. `contained::spawn_in(&mut set, …)` panicking: `join_next` gives a panic, and the hook recorded
   `true`.
4. **Control**: raw `tokio::task::spawn_blocking(|| panic!("raw"))`, which needs its own
   `#[expect(clippy::disallowed_methods)]`, records `false`. It proves the recorder can see a
   `false`, so cases 1-3 cannot pass vacuously.
5. **Window scope**: on a `current_thread` runtime, a contained future parks on a `oneshot`. A raw
   task that runs while the first one is `Pending` observes `panic_is_contained() == false` and then
   releases it. This pins D4: no window stays open across a `Pending`.
6. Values pass through: each helper returns its closure's or future's output unchanged.

A runtime-level case through `AgentRuntime` is **not** added. There is no seam that makes
`system_facts` or an unpack panic on demand (the `HardwareSource` fake replaces `SystemHardware`
entirely), and adding one only for this would be a production seam for a test. The two proofs that
the sites use the helper are structural: the clippy guard (D5) and its mutation check (T3).

---

## Out of scope (for the close-out's "Not done")

- Spawns outside `htui-agent` (`htui`'s store/concepts/run workers, `htui-store`, `htui-orch`).
  Those tasks are not polled inside any window, so a panic in them tears the terminal down whether
  or not their blocking threads are contained. That is a separate question for each worker and is
  not filed here.
- A panicking `Drop` during an unwind still aborts with the terminal left raw (MOD-53 "Not done",
  unchanged).
- The chained default hook still prints a contained panic's message to stderr under the TUI, as it
  does for provider and runtime-task panics today (MOD-56 behaviour, unchanged).

---

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| The two sites the item names exist and use `spawn_blocking` | true | `box_probe/hardware.rs:68`, `install/run.rs:368` |
| htui-agent has 12 `spawn_blocking` + 9 `tokio::spawn`/`JoinSet::spawn` sites in `src` | true | clippy probe with the D5 `disallowed-methods`: 21 warnings on `htui-agent (lib)`, sites as listed |
| `CONTAINED` is thread-local, so `answering`'s window does not reach other threads | true | `excerpt.rs:690-711`; `answering` `agent_worker.rs:3274-3300` |
| The hook restores on any panic that is not contained, then chains the previous hook | true | `terminal.rs:71-79`, `:95-97` |
| No `resume_unwind` / `into_panic` anywhere in `crates/` (D1's soundness argument) | true | grep: only `is_panic()` at `excerpt.rs:1216`, `run_worker.rs:1836` |
| Every one of the 12 blocking sites turns the `JoinError` into a value (none re-panics) | true | read at each site: `unwrap_or_default` / `.ok()` / `map_err(...)` / `match` |
| The excerpt provider thread is already contained | true | `excerpt.rs:807` runs `propose_caught`, which enters `Contained` (`:750-757`) |
| A crate-level `clippy.toml` is honoured by `cargo clippy -p htui-agent` from the workspace root | true | compile probe, 2026-09-30, clippy 0.1.98 |
| All six D5 paths resolve (silence means resolved) | true | probe: no config warning, while a bogus `tokio::task::no_such_fn` gives "does not refer to a reachable function" |
| The lint reaches integration tests: 18 sites in 7 files | true | probe: `acp_conformance` 1, `acp_driver` 2, `auth` 3, `cli_conformance` 1, `driver_contract` 1, `install` 5, `probe` 5 |
| No `src` `#[cfg(test)]` module spawns (lib test = the same 21 as lib) | true | probe: "lib test generated 21 warnings (21 duplicates)" |
| `#![expect(clippy::disallowed_methods)]` is silent under plain `cargo build -D warnings`, satisfied under clippy when used, and an error under clippy `-D warnings` when unused | true | scratch-crate probe on the pinned 1.98.1 toolchain |
| The crate's tokio features include `rt` (spawn, spawn_blocking, JoinSet) | true | `crates/htui-agent/Cargo.toml:24` |
| `#[expect(…, reason = …)]` has precedent in the repo | true | `crates/htui-agent/tests/recorder.rs:263` |
| The root `clippy.toml` holds only `msrv = "1.98"`, so copying that line keeps the crate's config complete | true | `clippy.toml` |
| No production seam makes `system_facts` or the unpack panic on demand | true | `HardwareSource` fake replaces `SystemHardware` whole (`hardware.rs`); `archive::unpack` takes real paths |

---

## Amendments after review (2026-09-30, maintainer-approved)

The `rust-reviewer` pass (3 MEDIUM, 5 LOW, all applied; re-review approved) changed two statements
above. They are recorded here rather than edited in place, so the confirmed text stays as it was
approved.

- **D7 reversed.** `std::thread::spawn`, `std::thread::Builder::spawn`,
  `std::thread::Builder::spawn_scoped` and `std::thread::Scope::spawn` are now in the lint. The
  excerpt provider thread carries a reasoned `#[expect]` naming `propose_caught`. The lint also
  covers `Runtime::spawn`/`spawn_blocking`, `JoinSet::spawn_on`/`spawn_blocking_on`, and the
  `spawn_local` family. `JoinSet`'s `Extend`/`FromIterator` spawn inside tokio, where no lint can
  see them, and the module doc forbids them.
- **"Unchanged" in Out of scope was wrong.** MOD-65 **widens** MOD-53's abort case from `answering`
  tasks to all 21 sites. Before, a panic in the ACP `run_session` or the unpack restored the
  terminal on its first hook call, even when a `Drop` then panicked and aborted. Now the first hook
  skips the restore, so the abort leaves the terminal raw.
- **Deferred by the maintainer: the cancel drop.** tokio drops a cancelled task's future (abort,
  runtime shutdown) outside the window, so a `Drop` that panics on cancel is survived by tokio but
  still restores the terminal.
