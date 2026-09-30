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
