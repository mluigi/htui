//! The walk's transport-neutral conformance suite (plan D18), behind `test-support`.
//!
//! **Empty on purpose.** T3 lands the skeleton — `pub const CASES: &[&str]`, `run_case`,
//! `run_all`, a dispatcher whose `match` panics on an unknown name and the unit test that runs
//! every `CASES` entry so the list and the dispatcher cannot drift — and T5 lands the case
//! bodies. The shape is the one both existing suites already use
//! (`crates/htui-agent/src/conformance.rs:146-238`,
//! `crates/htui-core/src/store/conformance.rs:36-239`).
