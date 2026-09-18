//! The walk's transport-neutral conformance suite (plan D18), behind `test-support`.
//!
//! The shape is the one both existing suites already use
//! (`crates/htui-agent/src/conformance.rs:146-238`,
//! `crates/htui-core/src/store/conformance.rs:36-239`): a `CASES` list that is the suite's API, a
//! `run_case` dispatcher whose `match` panics on a name it does not know, a `run_all` that walks
//! the list, and a unit test that runs every name through the dispatcher so the two cannot drift.
//!
//! T3 lands the list, the harness traits and the case stubs; T5 replaces each stub body with the
//! validation criterion it is named for.

/// Case names in run order. A name never changes: every binding reports per case.
///
/// Eleven, and the count is pinned in two places on purpose — here by
/// `cases_are_unique_and_eleven` and out of crate by `tests/fake_conformance.rs` (T5) — because a
/// binding that silently ran ten of them would still be green.
///
/// Six are `docs/ANA-2.md` §12's validation criteria (1, 2, 3, 5, 6, 7); four are contract lines
/// §12 does not number but §4.2 states outright; the last is the `finish_run` seam T1 shipped for
/// this walk to use.
pub const CASES: &[&str] = &[
    // ANA-2 §12 criterion 1 (`docs/ANA-2.md:2085`): a FEAT graph walks its four phases.
    "feat_walks_end_to_end",
    // Criterion 2 (`:2088`): invariant 2 — a live run reads its snapshot, never the graph.
    "live_run_ignores_a_gate_edit",
    // Criterion 3 (`:2090`): a graph edited under a parked run is a topology mismatch on resume.
    "topology_mismatch_parks_on_resume",
    // Criterion 5 (`:2096`): approving a step that produced no output document is refused.
    "approve_needs_the_output_document",
    // Criterion 6 (`:2098`): the review loop runs its budget out and escalates to a human.
    "review_rejection_loops_then_escalates",
    // Criterion 7 (`:2103`): two identical `after_hash` values mean the loop stopped making
    // progress, and it is stopped before its retry budget says so.
    "identical_after_hash_stops_the_loop",
    // `docs/ANA-2.md:414`: a missing required input fails the run before a token is spent.
    "missing_input_fails_before_a_token",
    // `:430`: a step that produced no document of its `output_kind` settles `failed`.
    "missing_output_settles_failed",
    // `:482`: a gated phase whose only candidate cannot answer a permission request is refused
    // with `missing_capability: inline_approval`.
    "cli_agent_is_refused_at_a_gated_phase",
    // `:439` and plan D8: a step that outlives its deadline settles `failed`.
    "step_deadline_settles_failed",
    // Plan D7: the last write of a finished walk is `finish_run`, and the item mirrors it.
    "finish_run_is_the_last_write",
];

#[cfg(test)]
mod tests {
    use super::CASES;

    /// The list is the suite's API, and its length is a claim a binding is allowed to check.
    #[test]
    fn cases_are_unique_and_eleven() {
        let mut sorted: Vec<&&str> = CASES.iter().collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
        assert_eq!(
            CASES.len(),
            11,
            "six ANA-2 §12 criteria, four §4.2 contract lines and the `finish_run` seam"
        );
    }
}
