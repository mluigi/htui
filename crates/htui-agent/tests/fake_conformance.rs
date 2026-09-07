//! Binds the transport-neutral conformance suite to the [`FakeDriver`] (plan MOD-2 T6, design row
//! D7).
//!
//! The suite itself names no transport: this file is the **only** binding in milestone 1, and
//! milestone 3's ACP transport adds a second file exactly like it. That is `docs/ANA-4.md` §11
//! criterion 1 made mechanical — one [`conformance::CASES`] list, one
//! [`conformance::CaseHarness`] per transport, and adding a transport adds no case.
#![cfg(feature = "test-support")]

use htui_agent::conformance::{self, CaseHarness, Script};
use htui_agent::driver::AgentDriver;
use htui_agent::fake::FakeDriver;
use htui_core::store::MemStore;

/// The fake's binding: a script becomes a [`FakeDriver`] and nothing else changes.
#[derive(Debug)]
struct FakeHarness;

impl CaseHarness for FakeHarness {
    fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
        Box::new(FakeDriver::scripted(script))
    }
}

/// The list is the suite's API and its length is asserted against a literal, the way
/// `crates/htui-core/tests/mem_store.rs:24` asserts the store suite's.
#[test]
fn cases_len_is_thirteen() {
    assert_eq!(
        conformance::CASES.len(),
        13,
        "D7's thirteen cases; a transport adds a harness, never a case (ANA-4 §11 criterion 1)"
    );
}

/// Case names are the suite's API: a second transport reports per case against the same names.
#[test]
fn case_names_are_unique() {
    let mut sorted = conformance::CASES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        conformance::CASES.len(),
        "case names are the suite's API"
    );
}

/// Every case of the one list, against the fake, recorded into `MemStore::demo()`.
#[tokio::test]
async fn fake_driver_passes_every_case() {
    conformance::run_all(&FakeHarness, || async { MemStore::demo() }).await;
}
