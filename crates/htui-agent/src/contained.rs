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
//! **Kept that way by the lint.** This crate's `clippy.toml` refuses the raw tokio spawns and the
//! raw `std::thread` spawns (`disallowed-methods`), so a new one fails the workspace clippy gate
//! instead of reopening the hole silently. The three raw calls below are the only ones in `src`
//! it allows, apart from the excerpt provider thread in
//! [`run_providers`](crate::excerpt::run_providers), which opens its own window in
//! `propose_caught`. The integration tests' fake agents and `tests/contained_spawn.rs`'s control
//! calls are allowed too, each by an `expect` that names why. One door the lint cannot see:
//! `JoinSet`'s `Extend` and `FromIterator` impls spawn from inside tokio, so collecting or
//! extending futures into a `JoinSet` starts raw tasks. Do not do that in this crate; call
//! [`spawn_in`] once per future instead.
//!
//! **What is not covered.** The window covers what the closure or future does while it runs.
//! - A task that tokio cancels (an abort, or the runtime shutting down) has its future dropped
//!   outside any poll, so outside the window. A `Drop` that panics there is still caught by tokio
//!   and the process survives it, but the hook finds no window and gives the terminal back under a
//!   running event loop. Deferred by the maintainer, not fixed here.
//! - A `Drop` that panics **during the unwind** of a contained panic makes the process abort,
//!   and the terminal is left raw. MOD-53 had this case for `answering`'s tasks, and MOD-65 widens
//!   it to every spawn in this crate. Before MOD-65 a panic in, say, the ACP `run_session` was
//!   outside any window, so the hook's first call restored the terminal before the abort. Now the
//!   first call skips the restore because tokio is expected to catch the panic, and the abort
//!   that follows leaves the terminal raw.

// D1's soundness argument is that tokio catches every panic in a spawned closure or future. Under
// `panic = "abort"` nothing unwinds and nothing is caught, so an open window would only suppress
// the one restore the process ever gets.
#[cfg(panic = "abort")]
compile_error!(
    "htui_agent::contained needs `panic = \"unwind\"`: the contain window is only sound when tokio \
     can catch the panic it vouches for (MOD-65 D1)"
);

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
