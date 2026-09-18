//! The three operator commands of ANA-2 §6.2 (`docs/ANA-2.md:1560-1562`) and their enabling
//! guards: `StartRun`, `AnswerGate` and `RetryStep`.
//!
//! **Empty on purpose.** T3 fills this file. The shape is blueprint §4.4: a `Command` enum, a
//! `CommandOutcome`, a `GateAnswer`, and one `enabled` predicate per command so the Runs tab can
//! grey an action out (milestone 6) using the same rule the engine enforces.
