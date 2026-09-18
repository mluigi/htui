//! `htui-orch`: the step-graph orchestrator (`docs/ANA-2.md` §4.2), milestone 2 — a graph walks.
//!
//! Depends on `htui-core` and `htui-agent`, **never** on `htui-store` (ANA-2 invariant 10,
//! `docs/ANA-2.md:143-146`): the engine is generic over `S: WriteStore` and holds no store handle
//! of its own, which is what keeps it headless (`R-ORCH-12`) and what keeps `htui-store` from
//! ever learning that an orchestrator exists.
//!
//! Milestone 2 lands [`graph`], [`status`], [`command`], [`isolate`], [`engine`] and [`gate`], plus
//! `fake` and `conformance` — those two are named in plain text, not linked, because they are
//! `#[cfg(feature = "test-support")]` and a doc link to them is a `broken_intra_doc_links` error
//! in any build without the feature. `fanout.rs`, `select.rs`, `verify.rs`, `overlap.rs`, `recover.rs`
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

pub use command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
pub use engine::{
    AgentSelector, DriverFor, Engine, EngineParts, FirstCandidate, NoSink, Resume, SessionSink,
    required_inputs,
};
pub use gate::{
    Landing, LoopOutcome, LoopStop, Settle, SettleInput, StepFailure, Verdict, parse_verdict,
};
pub use graph::{
    GraphSource, ResolveError, Resolved, override_graph, resolve, resolve_scope, topology,
};
pub use isolate::{
    Clock, IsolateError, Isolator, IsolatorFuture, Prepared, PreparedTree, SystemClock,
};
pub use status::{Cursor, RunFailure, cursor, latest_at, may_attempt, next_attempt};

// Four of the blueprint's `engine::` names are exported from elsewhere, and the crate-root paths
// are unchanged by it: `Clock` and `SystemClock` are `isolate`'s (their only implementor this
// milestone is `fake::TestClock`, and T3 needed the trait before `engine.rs` existed), and
// `EngineError` and `Rest` are `command`'s (the enabling guards return the first and
// `CommandOutcome` carries the second). Milestone 6 reads all four from here either way.
//
// Three names the blueprint does not list are exported because they are part of the surface a
// caller has to match on: `gate::Landing` (what stage 6 decided), `engine::Resume` (whether a
// resumed run found its own topology) and `gate::LoopStop` (why the review loop stopped, which is
// `StopReason` in blueprint §5.6 and is renamed because this crate already re-exports
// `htui_agent::event::StopReason`'s vocabulary through `gate::settle`).
