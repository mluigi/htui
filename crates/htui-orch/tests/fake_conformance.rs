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
fn cases_len_is_pinned() {
    assert_eq!(
        CASES.len(),
        107,
        "73 before MOD-41; CLEAN-4 makes it 74, T1's fenced capture (plan D1) 75, T9's eleven hand-back \
         cases (plan D12, OQ-6) 86, MOD-37 review L1's crashed rejection handed back 87, MOD-26's \
         five persona cases (plan D9, D12, D13) 92, MOD-37 M4's R-49 admission pin 93, MOD-73's \
         gate edit read by the next phase 94, MOD-11 T8's six command-queue cases (plan D16, \
         D17) 100, MOD-12 M1's three (criterion 23's two halves, criterion 24) 103, and MOD-12 \
         M2's four (the batch walk refusal, the batch remainder, the manual run, the run cap \
         across steps, plan D5, D6) 107"
    );
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
