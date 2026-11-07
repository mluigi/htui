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
        171,
        "73 before MOD-41; CLEAN-4 makes it 74, T1's fenced capture (plan D1) 75, T9's eleven hand-back \
         cases (plan D12, OQ-6) 86, MOD-37 review L1's crashed rejection handed back 87, MOD-26's \
         five persona cases (plan D9, D12, D13) 92, MOD-37 M4's R-49 admission pin 93, MOD-73's \
         gate edit read by the next phase 94, MOD-11 T8's six command-queue cases (plan D16, \
         D17) 100, MOD-12 M1's three (criterion 23's two halves, criterion 24) 103, and MOD-12 \
         M2's four (the batch walk refusal, the batch remainder, the manual run, the run cap \
         across steps, plan D5, D6) 107, and review R1's snapshotted run cap against a lowered live \
         one 108, and CLEAN-9 T2's malformed cap refused at start 109, and MOD-138 M1's five \
         pinned-resume cases (plan D5) 114, and blueprint A6's \
         refused-v1 half split in two 115, and MOD-144's three (a queued cancel writing no item, a \
         stopped run holding its item, a rework run on a finished item) 118, and MOD-138 M2 T4's \
         nine loops over edges (plan D7, D10, Q-D: a `changes_requested` edge on a node not called \
         `review`, a `review` with none passing unread, a `fails` loop retiring its region, on a \
         human's reject, replayed after a crash and spent, a diamond's region, no progress on an \
         edge loop, and a graph with authored edges resolving) 127, and MOD-138 M2 T5's thirteen \
         `if` and `asks_you` cases (plan D6, D7, D8; blueprint A2, A3, A9: each `if` predicate, a \
         declined branch and its join, an undecided branch parked and resumed, a forward \
         `asks_you` approved and rejected, auto mode's stop, a back `asks_you` loop, its crash \
         replay and its spent budget, a decision read from its row after a restart, accept \
         declining, a verdict-reading `asks_you` node's reject looping, and a failed settle's park \
         rejected as ever) 140, and MOD-138 M3 T3's seventeen concurrency cases (plan D1-D5, \
         blueprint A2-A6, A11: a linear graph's call order, two branches live at once, a cap of two, \
         a fan-out heavier than the cap, a join waiting for both landings, two worktree landings, \
         shared_serialized never overlapping, in-place beside worktree, an owed selection before an \
         in-place node, a park draining its sibling, a sibling parking on a parked run, one answer of \
         two parks re-parking, a cut sibling, a park then an end, deferred cleanup, a loop on one branch, an owed loop \
         before its successor) 157, and T3 repair 1's two (cleanup deferred until a cut sibling \
         settled, a terminal end cancelling a sibling row created beside it) 159, and T3 repair \
         2's one (a rejection ending the run cancelling the other parked step) 160, and MOD-138 M3 \
         T4's ten recovery cases (plan D6, blueprint A7-A9: two unlanded branches, a reordered \
         chain, a done branch beside a running one, an already landed branch re-landed, the later of two \
         in-place landings, two running branches adjudicated, running branches parked each, a \
         re-run loop target beside an older side successor, a crash-cut unpark on a branching \
         run, an answer landing the other crashed answer's winner first) 170, and \
         T4's hand-back twin of T3 repair 2 (a rejection handed back cancelling the other parked \
         step) 171"
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
