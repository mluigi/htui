//! `htui-orch`: the step-graph orchestrator (`docs/ANA-2.md` §4.2), milestone 2 — a graph walks.
//!
//! Depends on `htui-core` and `htui-agent`, **never** on `htui-store` (ANA-2 invariant 10,
//! `docs/ANA-2.md:143-146`): the engine is generic over `S: WriteStore` and holds no store handle
//! of its own, which is what keeps it headless (`R-ORCH-12`) and what keeps `htui-store` from
//! ever learning that an orchestrator exists.
//!
//! Milestone 2 lands [`graph`] and [`status`]. [`command`], [`isolate`], [`engine`], [`gate`],
//! [`fake`] and [`conformance`] are declared here with their contracts and are filled by
//! milestone 2's later tasks. `fanout.rs`, `select.rs`, `verify.rs`, `overlap.rs`, `recover.rs`
//! and `queue.rs` are named by ANA-2 §8 and belong to later milestones; they are deliberately not
//! created, not even empty (plan D1).
#![warn(missing_docs)]

pub mod command;
#[cfg(feature = "test-support")]
pub mod conformance;
pub mod engine;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod gate;
pub mod graph;
pub mod isolate;
pub mod status;

pub use graph::{
    GraphSource, ResolveError, Resolved, override_graph, resolve, resolve_scope, topology,
};
pub use status::{Cursor, RunFailure, cursor, latest_at, may_attempt, next_attempt};

// The crate's public face is re-exported from here, and the list grows with the modules that
// define it: `command::{Command, CommandOutcome, GateAnswer}` and `isolate::{Isolator,
// IsolatorFuture, IsolateError, Prepared, PreparedTree}` with T3, and `engine::{AgentSelector,
// Clock, Engine, EngineError, FirstCandidate, NoSink, Rest, SessionSink, SystemClock}` and
// `gate::{Settle, Verdict, parse_verdict}` with T4. Re-exporting a name before its module defines
// it does not compile, so the list arrives in pieces rather than whole.
