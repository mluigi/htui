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
//! in any build without the feature. Milestone 3 adds [`verify`], ANA-2 §4.2's three outcomes
//! (`docs/ANA-2.md:508-515`), beside the real isolator. Milestone 4 adds [`select`], `R-AGT-8`'s
//! walk over a phase's candidates (plan D60), and [`fanout`], the pure half of fan-out: the
//! prefilter, the group's route, the judge's phase and inputs, and its verdict (plan D49, D52,
//! D53). Milestone 5 adds `overlap`, ANA-2 §4.7's scope resolution, and [`recover`], §4.9's lease
//! heartbeat and the sweep's pure half (plan D85, D86, D90, D91, D97). Milestone 6 adds
//! [`closeout`] and [`promote`], the pure halves of close-out and promotion. `queue.rs`, also named
//! by ANA-2 §8, remains MOD-12's and is deliberately not created, not even empty (plan D1).
#![warn(missing_docs)]

pub mod closeout;
pub mod command;
#[cfg(feature = "test-support")]
pub mod conformance;
pub mod engine;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod fanout;
pub mod gate;
pub mod graph;
pub mod isolate;
pub mod overlap;
pub mod promote;
pub mod recover;
pub mod select;
pub mod status;
pub mod verify;

pub use command::{Command, CommandOutcome, EngineError, GateAnswer, Rest};
pub use engine::{
    Adopted, AgentSelector, DeadWalks, DriverFor, Engine, EngineParts, FirstCandidate, Next,
    NoSink, Resume, SessionKey, SessionSink, live_step_at, required_inputs,
};
#[cfg(feature = "test-support")]
pub use engine::{claim_fake, dispatch_fake, resume_fake, sweep_fake};
pub use fanout::{JudgeFailure, Route};
pub use gate::{
    Landing, LoopOutcome, LoopStop, Settle, SettleInput, StepFailure, Verdict, parse_verdict,
};
pub use graph::{GraphSource, ResolveError, Resolved, override_graph, resolve, topology};
pub use isolate::{
    Clock, FanoutSlot, GixIsolator, IsolateError, Isolator, IsolatorConfig, IsolatorFuture,
    Prepared, PreparedTree, RepoCheckout, ResetReport, SystemClock,
};
pub use recover::{Heartbeat, LeaseTimes};
pub use select::{SkipCause, Walk, walk};
pub use status::{Cursor, RunFailure, cursor, latest_at, may_attempt, next_attempt};
pub use verify::{ShellVerifier, Verifier, VerifierFuture, VerifyReport, VerifyRequest};

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
