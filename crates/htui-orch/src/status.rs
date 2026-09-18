//! Where the walk is, and whether it may take another step.
//!
//! Three things milestone 2 needs before anything can walk: ANA-2 §4.2's retry admission
//! predicate (plan D3), the typed failure vocabulary whose `Display` renders ANA-2's exact bytes
//! (plan D12), and the cursor the engine re-derives from `run_steps` on every call rather than
//! remembering across one (plan D16).
//!
//! **Empty on purpose until T2's second commit.**
