# Blueprint: MOD-65 - contained spawns

Implements `.claude/plans/mod-65-contained-spawns.plan.md` (CONFIRMED; D1-D7 are not reopened).
Base `hr/MOD-65` @ `acb0745`. Every file:line below was checked against the tree on 2026-09-30.
tokio is `1.53.1` (`Cargo.lock:6649`); the bounds are copied from its source.

## Design decisions (blueprint-level, within the plan)

- **B1 - call sites use the full path `crate::contained::…`, with no `use`.** It is the one-token swap,
  and it has to be: `launch.rs` defines `pub async fn spawn` (`launch.rs:1029`), so a
  `use crate::contained::spawn;` there would be E0255. Full paths also keep `unused_qualifications`
  (workspace `warn`) quiet, because no site imports the bare helper name.
- **B2 - every `#[expect(clippy::disallowed_methods)]` lands in T3, none in T1.** Before
  `clippy.toml` exists the lint never fires, so an `expect` would be an unfulfilled expectation, and
  clippy `-D warnings` would fail on the T1 and T2 commits (H-1). That covers the three helpers,
  the two raw spawns in `contained_spawn.rs` and the seven test files.
- **B3 - `#[track_caller]` on all three helpers**, the same as tokio's own. The "must be called from
  the context of a Tokio runtime" panic and the task spawn location then point at the call site, not
  at `contained.rs`.
- **B4 - no `#[must_use]`.** tokio's `spawn` has none either, and `launch.rs:1053` and
  `auth/browser.rs:126` drop the handle on purpose.
- **B5 - the per-poll wrapper is one private fn, `per_poll`**, shared by `spawn` and `spawn_in`.

## Files to create

| File | Purpose | Task |
|---|---|---|
| `crates/htui-agent/src/contained.rs` | `spawn_blocking`, `spawn`, `spawn_in`, private `per_poll`, module doc | T1 (expects T3) |
| `crates/htui-agent/tests/contained_spawn.rs` | one `#[test]`, process hook recorder, six cases | T1 (expects T3) |
| `crates/htui-agent/clippy.toml` | msrv copy + six `disallowed-methods` | T3 |

## Files to modify

| File | Change | Task |
|---|---|---|
| `crates/htui-agent/src/lib.rs` | `pub mod contained;` after `:102`; crate-doc paragraph after `:41` | T1 |
| 13 `src` files, 21 sites (table §4) | path swap; `install/run.rs:47-51` and `excerpt.rs:1014` docs | T2 |
| 7 `tests/*.rs` | file-level `#![expect]` | T3 |
| `crates/htui/src/agent_worker.rs` | `:3272-3273` doc paragraph | T4 |

---

## 1. `crates/htui-agent/src/contained.rs` (final form, after T3)

Lint constraints, checked against the tree:
- `missing_docs` (`lib.rs:45`, warn): the module and all three pub fns need docs. `per_poll` is
  private, so it does not need one, but it gets one anyway.
- `missing_debug_implementations`: this module adds no types, so it does not apply.
- rustdoc (`Cargo.toml` `[workspace.lints.rustdoc]`, all `deny`):
  - `redundant_explicit_links`: `contain` and `JoinSet` are imported, so link them **bare**
    (`[`contain`]`, `[`JoinSet::spawn`]`). Writing `[`contain`](crate::excerpt::contain)` here is a
    hard error.
  - `private_intra_doc_links`: pub docs must not link `per_poll` or `excerpt`'s private `CONTAINED`
    / `Contained`.
  - `htui` is not a dependency, so `htui::terminal` and `answering` appear in plain backticks only.
- `Future` is in the edition-2024 prelude (`agent_worker.rs` uses it unimported), so there is no
  `use std::future::Future`.

```rust
//! Spawning with the contain window open: every thread and task this crate starts (MOD-65).
//!
//! [`contain`] tells `htui`'s panic hook that a panic on **this** thread is one somebody is going
//! to catch, so the hook leaves the terminal alone. The flag behind it is a thread-local, and that
//! was the hole. `htui`'s agent runtime polls each of its tasks inside the window (MOD-53), but a
//! blocking thread or a task that the task starts runs on **another** thread, where the flag is
//! false. A panic there made the hook give the terminal back under a running event loop, even
//! though tokio caught it and the caller carried on with a `JoinError`. That is the wedged-UI shape
//! MOD-56 and MOD-53 closed, reached through a door neither of them covered.
//!
//! So the three functions here are tokio's, with the window opened on the thread that runs the
//! work: [`spawn_blocking`] runs its closure inside [`contain`], and [`spawn`] and [`spawn_in`]
//! poll their future inside it. Each keeps tokio's bounds and returns tokio's own handle, so a call
//! site changes by one path and its `JoinError` handling does not change at all.
//!
//! **Why every spawn, and not a judged subset** (plan D1). Opening the window is sound wherever
//! tokio is the catcher, and for a spawned closure or future tokio always is: a panic becomes a
//! `JoinError` and the process lives. An awaiter that re-raises it with `expect` or `unwrap` raises
//! a **new** panic on its own thread, and the hook asks that thread's own window, so no panic the
//! process does not survive loses its restore. The one re-raise that would skip the hook is
//! [`std::panic::resume_unwind`], and this workspace has none.
//!
//! **Per poll, not per task** (D4). A future is polled on whichever worker picks it up, and a
//! window left open across a `Pending` would vouch for a different task's panic on that thread. So
//! the window opens and closes around each poll, the same shape as `htui`'s `answering`. Because
//! [`contain`] restores the previous value, the window nests under `answering` when both apply.
//! The future is boxed rather than pin-projected (`unsafe_code` is forbidden and there is no
//! `pin-project`); one allocation is nothing next to a process or an ACP session.
//!
//! **Kept that way by the lint.** This crate's `clippy.toml` refuses the raw tokio spawns
//! (`disallowed-methods`), so a new one fails the workspace clippy gate instead of reopening the
//! hole silently. The three raw calls below are the only ones it allows. The excerpt provider
//! thread is a plain `std::thread` outside the lint, because it opens its own window in
//! [`run_providers`](crate::excerpt::run_providers) (D7).
//!
//! What is covered is what the closure or future does while it runs. Dropping a future that tokio
//! cancels happens outside any poll, so a panicking `Drop` there is not covered (MOD-53's
//! "Not done").

use tokio::task::{AbortHandle, JoinHandle, JoinSet};

use crate::excerpt::contain;

/// [`tokio::task::spawn_blocking`], with `f` run inside the [`contain`] window.
///
/// A panic in `f` is still the `JoinError` the handle resolves to. What changes is that `htui`'s
/// panic hook, which runs on the blocking thread during the unwind, now knows tokio is about to
/// catch it.
///
/// # Panics
/// As tokio's: when called outside a Tokio runtime.
#[track_caller]
#[expect(
    clippy::disallowed_methods,
    reason = "the wrapper the lint points at: the closure it hands tokio is `contain(f)`"
)]
pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(move || contain(f))
}

/// [`tokio::spawn`], with every poll of `future` inside the [`contain`] window.
///
/// Per poll and not once for the task (module doc, D4): between polls the worker runs other tasks,
/// and none of them is vouched for.
///
/// # Panics
/// As tokio's: when called outside a Tokio runtime.
#[track_caller]
#[expect(
    clippy::disallowed_methods,
    reason = "the wrapper the lint points at: the future it hands tokio opens the window per poll"
)]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(per_poll(future))
}

/// [`JoinSet::spawn`] on `set`, with every poll of `future` inside the [`contain`] window.
///
/// A free function, not a method, because `JoinSet` is tokio's: `set.spawn(fut)` becomes
/// `spawn_in(&mut set, fut)`, and the set's `join_next` answers exactly as before.
///
/// # Panics
/// As tokio's: when called outside a Tokio runtime.
#[track_caller]
#[expect(
    clippy::disallowed_methods,
    reason = "the wrapper the lint points at: the future it hands tokio opens the window per poll"
)]
pub fn spawn_in<T, F>(set: &mut JoinSet<T>, future: F) -> AbortHandle
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    set.spawn(per_poll(future))
}

/// `future`, each poll of it inside [`contain`]: the window closes again before `Pending` goes
/// back to the scheduler.
fn per_poll<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let mut future = Box::pin(future);
    std::future::poll_fn(move |cx| contain(|| future.as_mut().poll(cx)))
}
```

Notes:
- The bounds are tokio's exactly: `spawn_blocking` is `task/blocking.rs:220-223`, `spawn` is
  `task/spawn.rs:174-177`, and `JoinSet::spawn` is `task/join_set.rs:99` (`impl<T: 'static>`) plus
  `:142-146` (`F: Future<Output = T> + Send + 'static, T: Send`), merged here into
  `T: Send + 'static`.
- `per_poll` returns `impl Future`, and auto traits leak through it, so it is `Send` whenever `F`
  is. Edition 2024 captures `F` implicitly, so the result is `'static` because `F: 'static`.
- `tokio::task::spawn(…)` inside `fn spawn` does **not** trip `unused_qualifications`, because the
  bare `spawn` resolves to this function, a different item. The same holds for `spawn_blocking`.
- The `#[expect]` block sits after `#[track_caller]` and directly on the fn. It cannot go on the
  call, because attributes on a tail expression are unstable. It is **absent in T1** (B2).
- **T1 red stubs** (step 1): the same signatures and docs, with the bodies
  `tokio::task::spawn_blocking(f)`, `tokio::task::spawn(future)` and `set.spawn(future)`, and no
  `per_poll`.

## 2. `crates/htui-agent/src/lib.rs`

- Module line: insert `pub mod contained;` **between `:102` (`pub mod conformance;`) and `:103`
  (`pub mod driver;`)**. The order is alphabetical, `conformance` < `contained`, and it is not
  feature-gated. Add no root re-export: `launch::spawn` already owns `spawn` at the root.
- Crate doc: insert after `:41` (the end of the `box_probe` paragraph), followed by a `//!` blank
  line, before `:43` (`Deliberately absent…`):

```rust
//! [`contained`] is MOD-65's answer to "and what if a thread it starts panics": every blocking
//! thread and task this crate starts runs inside [`excerpt::contain`]'s window, so a panic tokio
//! catches and hands back as a `JoinError` is one `htui`'s panic hook leaves the terminal alone
//! for. The raw tokio spawns are refused in this crate by its `clippy.toml`.
//!
```

## 3. `crates/htui-agent/tests/contained_spawn.rs`

Tokio: dev-deps give `macros, rt, rt-multi-thread, test-util, net` (`Cargo.toml` `[dev-dependencies]`)
and normal deps give `sync` (features unify). Both runtime flavours and `oneshot` are therefore
available. The test uses a plain `#[test]`, not `#[tokio::test]`, because it builds two runtimes of
its own. It needs no `test-support` feature, and there are no `[[test]]` entries, so the binary is
auto-discovered.

**Recorder** (from the precedent `crates/htui/tests/panic_hook.rs:71-83`, part 1):
`set_hook` **replaces** the default hook and pushes `(message, panic_is_contained())` into
`Arc<Mutex<Vec<(String, bool)>>>`. The message is the payload downcast to `&str`, then `String`,
then `"<non-text>"`. **Nothing inside the hook window asserts or `expect`s on an outcome.** Every
case reduces to plain `bool`s or values, and all `assert!`s run after `let _ = std::panic::take_hook();`.
Otherwise a failing assertion's own message would be swallowed by the recorder (H-6).

**Panic bodies are statements**, `|| { panic!("…"); }` and `async { …; panic!("…"); }`, so the
output type is `()`. A tail `panic!()` would leave `R` to edition-2024 never-type fallback (H-8).

Runtimes:
- `pool`: `Builder::new_multi_thread().worker_threads(2).build()`. It runs cases 1-4 and 6, so every
  panic happens on a thread other than the test thread.
- `current`: `Builder::new_current_thread().build()`, for case 5 only. It must be current-thread, so
  the parked task and the raw task share one OS thread. On a multi-thread runtime the raw task could
  read a different worker's flag, and the case would pass vacuously (H-7).
- Neither needs `enable_all()`, because there is no time or IO.

Cases (panic messages are the lookup keys):

| # | Code (inside `pool.block_on` unless noted) | Collected | Asserted after `take_hook` |
|---|---|---|---|
| 1 | `contained::spawn_blocking(\|\| { panic!("contained: blocking"); }).await` | `is_err_and(\|e\| e.is_panic())` | true; decision `"contained: blocking"` == `true` |
| 2 | `contained::spawn(async { tokio::task::yield_now().await; panic!("contained: task, second poll"); }).await` | same | true; decision == `true` |
| 3 | `let mut set = JoinSet::new(); contained::spawn_in(&mut set, async { yield_now().await; panic!("contained: set"); }); set.join_next().await` | `matches!(r, Some(Err(ref e)) if e.is_panic())` | true; decision == `true` |
| 4 | `#[expect(clippy::disallowed_methods, reason = "the control: a raw spawn must record false, or cases 1-3 prove nothing")] let raw = tokio::task::spawn_blocking(\|\| { panic!("raw: blocking"); }).await;` | `is_err_and(is_panic)` | true; decision `"raw: blocking"` == `false` |
| 5 | On `current.block_on`: two `oneshot`s, `entered` (bool) and `release` (()). `let parked = contained::spawn(async move { let _ = entered_tx.send(panic_is_contained()); release_rx.await.ok(); panic_is_contained() });` then `#[expect(clippy::disallowed_methods, reason = "the raw task that checks no window stays open across the parked task's Pending")] let raw = tokio::spawn(async move { let inside = entered_rx.await.unwrap_or(false); let outside = panic_is_contained(); let _ = release_tx.send(()); (inside, outside) });` then `(raw.await, parked.await)` | `(inside, outside)`, `after` | `inside == true` (window open in poll 1), `outside == false` (closed across `Pending`, D4), `after == true` (reopened in poll 2) |
| 6 | `contained::spawn_blocking(\|\| 7_u8).await.ok()`, `contained::spawn(async { yield_now().await; "task" }).await.ok()`, `spawn_in(&mut values, async { 11_u32 })` + `join_next` | values | `Some(7)`, `Some("task")`, `Some(Ok(11))` |

Final assertion: `seen.len() == 4`, meaning exactly the four panics of cases 1-4. Each expected key
must appear once. Look them up by message, never by position, because they arrive from different
threads. End on `take_hook()`, the same as the precedent, so no closure outlives the test.

Test name: `every_contained_spawn_opens_the_window_and_a_raw_one_does_not`. The file's `//!` says
why this is its own binary (`set_hook` is process state; cite `crates/htui/tests/panic_hook.rs`),
and why it asks `excerpt::panic_is_contained` rather than `htui::terminal::restores_the_terminal`:
`htui` is not a dependency, and one is the negation of the other.

**Red step (T1.1):** with the pass-through stubs, cases 1-3 record `false`, and case 5's `inside` and
`after` are `false`, so those asserts fail. Case 4, case 5's `outside` and case 6 pass.
**Green (T1.2):** add `contain` / `per_poll`, and everything passes. In T1 the two raw-spawn `let`s
have **no** `#[expect]` yet (B2). T3 adds them.

## 4. The 21 call sites (T2) - `tokio::task::` / `tokio::` → `crate::contained::`

Every "before" was confirmed verbatim with `sed -n`. Leading whitespace is unchanged.

| # | Site | Before | After |
|---|---|---|---|
| 1 | `launch.rs:1031` | `let program = tokio::task::spawn_blocking(move \|\| which::which(&command))` | `let program = crate::contained::spawn_blocking(move \|\| which::which(&command))` |
| 2 | `launch.rs:1053` | `tokio::spawn(async move {` | `crate::contained::spawn(async move {` |
| 3 | `box_probe/hardware.rs:68` | `let facts = tokio::task::spawn_blocking(system_facts)` | `let facts = crate::contained::spawn_blocking(system_facts)` |
| 4 | `box_probe/hardware.rs:235` | `tokio::task::spawn_blocking(move \|\| pci_display_vendors(&root))` | `crate::contained::spawn_blocking(move \|\| pci_display_vendors(&root))` |
| 5 | `box_probe/mod.rs:119` | `set.spawn(async move {` | `crate::contained::spawn_in(&mut set, async move {` **(method → free fn; the `});` at `:124` is unchanged)** |
| 6 | `box_probe/mod.rs:223` | `set.spawn(async move {` | `crate::contained::spawn_in(&mut set, async move {` **(the `});` at `:227` is unchanged)** |
| 7 | `install/run.rs:368` | `let mut handle = tokio::task::spawn_blocking(move \|\| {` | `let mut handle = crate::contained::spawn_blocking(move \|\| {` |
| 8 | `install/plan.rs:274` | `tokio::task::spawn_blocking(move \|\| {` | `crate::contained::spawn_blocking(move \|\| {` |
| 9 | `acp/fs.rs:86` | `tokio::task::spawn_blocking(move \|\| {` | `crate::contained::spawn_blocking(move \|\| {` |
| 10 | `acp/fs.rs:179` | `tokio::task::spawn_blocking(move \|\| match path.dir.read_to_string(&path.relative) {` | `crate::contained::spawn_blocking(move \|\| match path.dir.read_to_string(&path.relative) {` |
| 11 | `acp/fs.rs:210` | `tokio::task::spawn_blocking(move \|\| {` | `crate::contained::spawn_blocking(move \|\| {` |
| 12 | `acp/mod.rs:774` | `let task = tokio::spawn(run_session(` | `let task = crate::contained::spawn(run_session(` |
| 13 | `acp/handshake.rs:106` | `let task = tokio::spawn(async move {` | `let task = crate::contained::spawn(async move {` |
| 14 | `acp/auth.rs:199` | `let task = tokio::spawn(async move {` | `let task = crate::contained::spawn(async move {` |
| 15 | `auth/browser.rs:126` | `tokio::spawn(async move {` | `crate::contained::spawn(async move {` |
| 16 | `cli/mod.rs:705` | `let task = tokio::spawn(run_session(` | `let task = crate::contained::spawn(run_session(` |
| 17 | `cli/mod.rs:781` | `let reading = tokio::spawn(read_lines(reader, lines_tx));` | `let reading = crate::contained::spawn(read_lines(reader, lines_tx));` |
| 18 | `probe.rs:497` | `tokio::task::spawn_blocking(move \|\| {` | `crate::contained::spawn_blocking(move \|\| {` |
| 19 | `probe.rs:861` | `tokio::task::spawn_blocking(move \|\| {` | `crate::contained::spawn_blocking(move \|\| {` |
| 20 | `excerpt.rs:1104` | `let walked = tokio::task::spawn_blocking(move \|\| {` | `let walked = crate::contained::spawn_blocking(move \|\| {` |
| 21 | `excerpt.rs:1158` | `tokio::task::spawn_blocking(move \|\| excerpt_pass(&request, &listing, est))` | `crate::contained::spawn_blocking(move \|\| excerpt_pass(&request, &listing, est))` (99 cols, fits) |

Imports: no change anywhere. The existing imports stay used:
- `box_probe/mod.rs:21` `use tokio::task::JoinSet` is still used by `JoinSet::new()` at `:115` and
  `:208` and by the doc link at `:37`.
- `acp/mod.rs:41` and `cli/mod.rs:29` `use tokio::task::JoinHandle` are still used, because the
  helpers return tokio's `JoinHandle`.

**Docs that name the raw call:**
- `install/run.rs:47-51` (plan-mandated). Replace the five lines of step 3 with this (each line is
  ≤ 100 cols):
  ```rust
  /// 3. `Unpacking`: [`archive::unpack`] under [`crate::contained::spawn_blocking`], its byte
  ///    counter polled every `progress_every` while the handle is awaited — the only way to report
  ///    on work a [`JoinHandle::abort`](tokio::task::JoinHandle::abort) cannot even stop (blueprint
  ///    P-3). The `cmd` must be in the tree, and it is made executable **in staging** so the
  ///    promoted tree is never unrunnable for an instant (hazard H-5).
  ```
- `excerpt.rs:1014` (recommended, because it names the full tokio path, which is now false): change
  ``under `tokio::task::spawn_blocking` when one is`` to
  ``under [`crate::contained::spawn_blocking`] when one is`` (99 cols).
- **Left as they are**, because they name the concept and not the call, and stay true:
  - `excerpt.rs:972` and `:1133`
  - `launch.rs:1019` and `:1023`
  - `install/archive.rs:11` and `:68`
  - `probe.rs:492` and `:876`
  - `tools.rs:85`
  - `tests/probe_live.rs:85`

## 5. Guard (T3)

`crates/htui-agent/clippy.toml`:

```toml
# A crate-level clippy.toml REPLACES the workspace root one rather than merging with it (MOD-65
# D5), so every root setting is repeated here. Keep this in step with /clippy.toml.
msrv = "1.98"

# MOD-65 D5: every thread and task this crate starts opens the contain window, through
# `crate::contained`. A raw spawn panics outside the window, and htui's panic hook then gives the
# terminal back under a live event loop for a panic tokio was about to catch.
disallowed-methods = [
    { path = "tokio::task::spawn", reason = "use `crate::contained::spawn`: a raw task panics outside the contain window (MOD-65)" },
    { path = "tokio::task::spawn_blocking", reason = "use `crate::contained::spawn_blocking`: a raw blocking thread panics outside the contain window (MOD-65)" },
    { path = "tokio::task::JoinSet::spawn", reason = "use `crate::contained::spawn_in`: a raw set task panics outside the contain window (MOD-65)" },
    { path = "tokio::task::JoinSet::spawn_blocking", reason = "add a `crate::contained` helper first: a raw blocking thread panics outside the contain window (MOD-65)" },
    { path = "tokio::runtime::Handle::spawn", reason = "use `crate::contained::spawn`, or add a `crate::contained` helper that takes the handle: a raw task panics outside the contain window (MOD-65)" },
    { path = "tokio::runtime::Handle::spawn_blocking", reason = "use `crate::contained::spawn_blocking`, or add a `crate::contained` helper that takes the handle: a raw blocking thread panics outside the contain window (MOD-65)" },
]
```

The file-level expect is the same line in all seven files. In each file the `//!` block runs from
line 1 to the "last `//!`" line below, and the next line is blank. **Insert the attribute plus one
blank line after that blank line**, so the file reads `//!…` / blank / `#![expect…]` / blank /
`use…`:

```rust
#![expect(clippy::disallowed_methods, reason = "test fakes run under no TUI panic hook")]
```

| File | Last `//!` | `#![expect]` goes at | Raw spawns it covers |
|---|---|---|---|
| `tests/acp_conformance.rs` | 10 | 12 | 1 (`:60`) |
| `tests/acp_driver.rs` | 39 | 41 | 2 (`:356`, `:418`) |
| `tests/auth.rs` | 20 | 22 | 3 (`:189`, `:201`, `:223`) |
| `tests/cli_conformance.rs` | 44 | 46 | 1 (`:113`) |
| `tests/driver_contract.rs` | 21 | 23 | 1 (`:181`) |
| `tests/install.rs` | 18 | 20 | 5 (`:105`, `:108`, `:2705`, `:2778`, `:3530`) |
| `tests/probe.rs` | 8 | 10 | 5 (`:1024`, `:1314`, `:1409`, `:1446`, `:1597`) |

None of the seven has an existing `#![…]`. No other test file spawns: `probe_live.rs:85` is a doc
line.

T3 also adds:
- The three helper `#[expect]`s (§1).
- The two statement-level `#[expect]`s in `contained_spawn.rs` (cases 4 and 5, §3). Lint attributes
  on a `let` statement are stable and cover its initializer.

## 6. T4 - `crates/htui/src/agent_worker.rs:3272-3273`

Before:
```rust
/// A panic on a thread the task starts (`spawn_blocking`, a provider thread) is not caught here;
/// it comes back to the task as an error, as it always did.
```
After:
```rust
/// A panic on a thread the task starts (a blocking thread, a spawned task, a provider thread) is
/// not caught here; it comes back to the task as an error, as it always did. It does not reach the
/// terminal either: every blocking thread and task `htui_agent` starts opens its own window
/// through [`htui_agent::contained`] (MOD-65), and a provider thread opens one in `run_providers`.
```
This is accurate for `answering`'s tasks: every production spawn in `agent_worker.rs` is
`tokio::spawn(answering(…))` (`:628`, `:1273`, `:1309`, `:1360`, `:1420`, `:1469`, `:1579`, `:1904`).
The tasks start no raw thread of their own. The other hits (`:3950`, `:3953`, `:5341`, `:8753`,
`:8778`) are tests.

## 7. Build sequence

| Step | Work | Validation | Commit |
|---|---|---|---|
| T1.1 | `contained.rs` stubs (no expects) + `lib.rs` (§2) + `tests/contained_spawn.rs` (no expects) | `cargo test -p htui-agent --test contained_spawn` must **fail**, on case 1-3 decisions and case 5 `inside`/`after` | no |
| T1.2 | `per_poll` + `contain` bodies (§1) | `cargo test -p htui-agent --test contained_spawn` passes; `cargo doc -p htui-agent --no-deps` is clean; `cargo clippy -p htui-agent --all-features --all-targets -- -D warnings` is clean | `feat(mod-65): contained spawn helpers` |
| T2 | 21 swaps + 2 doc edits (§4) | `grep -rnE 'tokio::(task::)?spawn(_blocking)?\(\|set\.spawn\(' crates/htui-agent/src` → only `contained.rs`; `cargo test -p htui-agent --all-features -- --test-threads=1`; `cargo doc -p htui-agent --no-deps`; clippy as in T1.2 | `fix(mod-65): every htui-agent spawn opens the contain window` |
| T3 | `clippy.toml` + 7 file expects + 5 local expects (§5) | `cargo clippy -p htui-agent --all-features --all-targets -- -D warnings` is clean. **Mutation:** revert site 3 (`hardware.rs:68`) to `tokio::task::spawn_blocking`, and clippy must fail naming it with the reason; restore it. Also delete one test file's `#![expect]` and clippy must fail there; restore it | `build(mod-65): lint raw tokio spawns in htui-agent` |
| T4 | doc (§6) | `cargo doc -p htui --no-deps --document-private-items` shows no *new* intra-doc error on `answering`, because the link is on a private fn and is only resolved with that flag | `docs(mod-65): answering names the contained spawns` |
| Gates (main thread) | - | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-features --all-targets -- -D warnings`; `cargo test --workspace --all-features -- --test-threads=1` | - |

Run `cargo fmt --all` before each commit. Rows 1, 2 and 17 grow by 5-11 cols, and rustfmt may
re-wrap them.

## 8. Hazards

- **H-1** Putting any `#[expect(clippy::disallowed_methods)]` in before `clippy.toml` makes clippy
  `-D warnings` fail with `unfulfilled_lint_expectations` on the T1/T2 commits. All of them go in
  T3 (B2). Under plain `cargo build`/`test` they are inert (plan, verified claims).
- **H-2** `redundant_explicit_links` is `deny`. In `contained.rs`, link `contain` and
  `JoinSet::spawn` bare, because both are imported. The error shows only under `cargo doc`, which
  the plan's gate list does not run, so T1/T2 run it explicitly.
- **H-3** `private_intra_doc_links` is `deny`. Pub docs in `contained.rs` must not link `per_poll`,
  `CONTAINED` or `Contained`.
- **H-4** E0255 / `unused_qualifications`: never `use crate::contained::spawn` or `spawn_blocking`
  at a site. `launch.rs` owns a `spawn`, and full paths are the whole convention (B1).
- **H-5** No `#[must_use]` on the helpers, because two sites drop the handle deliberately (B4).
- **H-6** The recorder replaces the default hook. Any `assert!`/`expect` that fails while it is
  installed reports no message. Collect `bool`s, `take_hook()`, then assert (precedent
  `panic_hook.rs:106-107`).
- **H-7** Case 5 must use the current-thread runtime, or `outside == false` is vacuous.
- **H-8** Write panicking closures and async blocks as statements (`panic!(…);`). A tail `panic!()`
  makes `R`/`Output` fall back to `!` under edition 2024 and fails the `Send` inference in odd ways.
- **H-9** The crate `clippy.toml` shadows the root one. A key later added to `/clippy.toml` will not
  reach `htui-agent` unless it is copied, and the file's header comment says so.
- **H-10** These are outside the guard's reach and stay uncontained (no action, for the close-out's
  "Not done"):
  - tokio's own internal blocking: `tokio::fs`, and DNS through `reqwest`/hyper.
  - Tasks that third-party crates spawn (the ACP SDK, hyper).
  - `tokio::runtime::Runtime::spawn`, `spawn_local` and `task::Builder`, which are not in the
    list. The crate has no current use of any of them.
  - A panicking `Drop` of a future that tokio cancels (module doc's last paragraph).
- **H-11** `cargo test` is scheduling-dependent in this repo (project memory). Validate T2 with
  `--test-threads=1`. `contained_spawn` is its own binary, so it is not exposed to the
  cross-test keyring fake.
