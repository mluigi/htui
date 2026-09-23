use htui_orch::conformance::{CASES, CaseHarness, run_all};
use htui_orch::fake::FakeOrchestrator;

struct Demo;

impl CaseHarness for Demo {
    type Orch = FakeOrchestrator;

    fn fresh(&self) -> FakeOrchestrator {
        FakeOrchestrator::demo()
    }
}

#[test]
fn cases_len_is_forty_two() {
    assert_eq!(CASES.len(), 42);
}

#[test]
fn case_names_are_unique() {
    let unique: std::collections::HashSet<_> = CASES.iter().collect();
    assert_eq!(unique.len(), CASES.len());
}

#[tokio::test]
async fn fake_orchestrator_passes_every_case() {
    run_all(&Demo).await;
}
