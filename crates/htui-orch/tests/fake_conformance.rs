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
fn cases_len_is_eleven() {
    assert_eq!(CASES.len(), 11);
}

#[test]
fn case_names_are_unique() {
    let mut sorted: Vec<&&str> = CASES.iter().collect();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), CASES.len());
}

#[tokio::test]
async fn fake_orchestrator_passes_every_case() {
    run_all(&Demo).await;
}
