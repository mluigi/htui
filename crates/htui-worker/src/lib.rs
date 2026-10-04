//! `htui-worker`: run supervision without a UI (MOD-41 plan D6), the runtime the TUI and
//! `htui worker` share. It was `htui/src/run_worker.rs` until MOD-41 T6 (MOD-4 milestone 6, plan
//! D153, drove it from the store worker loop).
//!
//! `htui-orch` never names `htui-store` (ANA-2 invariant 10), so everything that joins the two
//! lives here:
//!
//! - [`HostGraphs`], the graph source over any [`WorkerHost`](htui_core::store::WorkerHost)
//!   (MOD-4 D155; MOD-41 plan D7);
//! - the request and reply shapes the views speak — [`OrchRequest`], [`OrchReply`], [`RunFrame`],
//!   [`ItemActions`] — and [`actions`], every verdict from the engine's own admission functions
//!   (D182, D184);
//! - [`RunRuntime`], which runs every command on a task of its own that answers its request once,
//!   at the request's address (`R-NF-3`, R-41). One [`RunLocks`] entry per run serialises a run's
//!   commands, walks and recoveries (R-27, D157); `CancelRun` and `PromoteStep` preempt a live
//!   walk — gracefully first, through its run's control (MOD-42 plan D11), then through its
//!   cancellation token — and give its lease and guards back (D187, D188); a cancel is a durable
//!   `run_command` row its executor applies, read by a command poll (MOD-42 plan D12, D13); the
//!   sweep runs at start, at every `Online` and on a ticker, fenced by the same locks (D158,
//!   D189, D190); every task is supervised, so a panicked walk is adopted by the next sweep
//!   (R-12) and a refused claim is retried once a walk rests (M5 D84). Progress reaches each
//!   subscriber as [`RunFrame`]s at its own address (D172, blueprint §0a point 3).
//!
//! MOD-41 plan D7: [`RunRuntime`] is generic over its host (a
//! [`WorkerHost`](htui_core::store::WorkerHost)) and where its answers go (a [`ReplySink`]). The
//! TUI's host is its `Backend` and its sink the store loop's channel; the worker's sink is
//! [`Unaddressed`].
//!
//! Off the server every command is refused with MOD-25's sentence and nothing is spawned (D174).
//!
//! The crate links no terminal crate: never `ratatui`, never `crossterm` (`tests/deps.rs`). The
//! `test-support` feature compiles `testing`, the runtime's white-box surface for `htui`'s
//! `run_worker` tests (MOD-41 plan D8); it is named in plain text, not linked, because a link to a
//! feature-gated module is a `broken_intra_doc_links` error in any build without the feature.
//!
//! [`worker`] is `htui worker`'s loop (MOD-41 plan D14): a [`RunRuntime`] over a `PgStore` with
//! [`Role::Worker`] and the [`Unaddressed`] sink, polled, with the box heartbeat beside it.
#![warn(missing_docs)]

mod address;
mod graphs;
mod runtime;
mod views;
pub mod worker;

pub use address::{ChatEnd, Promoted, ReplySink, RunReply, RunRequest, RunServed, Unaddressed};
pub use graphs::HostGraphs;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use runtime::testing;
pub use runtime::{
    CANCEL_ALREADY_REQUESTED, CANCEL_GRACE, CANCEL_REQUESTED, PREEMPTED, REPOS_MOVED, Role,
    RunLocks, RunRuntime, UNBLOCK_MOVED, WALK_PANICKED, promote_needs_the_walker, switched_off,
    unknown_executor,
};
pub use views::{
    Enabled, FrameKind, ItemActions, LiveChats, ORCH_NAMES, OrchReply, OrchRequest, ProgressSink,
    RunActions, RunFrame, StepActions, StepAuthor, Via, WaitingReason, WaitingRow, WaitingView,
    actions, waiting,
};
