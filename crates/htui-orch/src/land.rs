//! MOD-133 D8, D9: landing's pure half. Which commits of a finished run the primary holds
//! ([`landed_commits`]), and the sentences a landing refuses or is noted with, so the real
//! isolator, the fake and the engine spell them once.
//!
//! A run's **landed** commits in a repo are the `after_hash` of exactly the rows `reconcile`
//! merged into the primary: the engine's `reconcile_done_step` is called only on a node's done
//! winner (a `fan_out = 1` row, or the `selected` candidate of a fanned-out slot) and on a divided
//! node's integrate row, and after a `worktree`/`copy` merge it rewrote that row's `after_hash` to
//! the merge commit. Fan-out losers (`selected = false`), judge rows (`fanout_index = -1`), a
//! divided node's planner and parts (their work reaches the primary through integrate's
//! pre-merge, then integrate's own landing), stage rows and every row not `done` are not landed
//! commits. A predicate that is too narrow only checks less (the target's tip is what is pushed),
//! and one too wide is a visible `not_on_target` refusal, never a lost push.

use std::collections::BTreeMap;

use htui_core::model::{
    GraphSnapshot, RepoId, RunStep, RunStepCommit, StepId, StepRole, StepStatus,
};

/// How many hex digits a hash is shown with in a reason or a note.
pub const SHORT_HASH: usize = 7;

/// `hash` cut to [`SHORT_HASH`] characters (whole, when it is shorter).
#[must_use]
pub fn short_hash(hash: &str) -> &str {
    hash.char_indices()
        .nth(SHORT_HASH)
        .map_or(hash, |(end, _)| &hash[..end])
}

/// MOD-133 D8: the run's landed commits in `repo`, in `steps` order, deduplicated: the
/// `after_hash` of each `done` [`StepRole::Step`] or [`StepRole::Integrate`] row that is not a
/// judge (`fanout_index >= 0`) and not a fan-out loser (`selected != Some(false)`). `commits` are
/// the rows' `run_step_commit` rows, by step, as `close_out_reads` reads them.
#[must_use]
pub fn landed_commits(
    steps: &[RunStep],
    commits: &[(StepId, Vec<RunStepCommit>)],
    repo: RepoId,
) -> Vec<String> {
    let mut landed: Vec<String> = Vec::new();
    for step in steps.iter().filter(|step| merged_into_primary(step)) {
        let Some((_, rows)) = commits.iter().find(|(id, _)| *id == step.id) else {
            continue;
        };
        for after in rows
            .iter()
            .filter(|row| row.repo_id == repo)
            .filter_map(|row| row.after_hash.as_ref())
        {
            if !landed.contains(after) {
                landed.push(after.clone());
            }
        }
    }
    landed
}

/// MOD-133 M3 D2, D5: the run branch of each `pull_request` repo of the run, as frozen in its
/// snapshot (`IntegrateSnapshot::run_branch`); empty for a `local` or `push` run, which then makes
/// no run-branch call at all.
#[must_use]
pub fn run_branches(snapshot: &GraphSnapshot) -> BTreeMap<RepoId, String> {
    snapshot
        .integrate
        .iter()
        .filter_map(|(repo, entry)| Some((*repo, entry.run_branch.clone()?)))
        .collect()
}

/// Whether `reconcile` merged `step` into the primary (the module's predicate).
fn merged_into_primary(step: &RunStep) -> bool {
    step.status == StepStatus::Done
        && step.fanout_index >= 0
        && step.selected != Some(false)
        && matches!(step.role, StepRole::Step | StepRole::Integrate)
}

/// The record's repo has no checkout on the landing box.
#[must_use]
pub fn repo_unavailable(detail: &str) -> String {
    format!("repo_unavailable: {detail}")
}

/// The landing box's checkout has no local `refs/heads/<target>`.
#[must_use]
pub fn no_target_branch(target: &str) -> String {
    format!("no_target_branch: {target}")
}

/// A landed commit of the run is not an ancestor of the local target branch.
#[must_use]
pub fn not_on_target(commit: &str, target: &str) -> String {
    format!("not_on_target: {} is not on {target}", short_hash(commit))
}

/// The remote's target branch is not an ancestor of the local one: `git push` refused the
/// fast-forward (`non-fast-forward` or `fetch first`).
#[must_use]
pub fn non_fast_forward(target: &str) -> String {
    format!("non_fast_forward: {target} on the remote has moved; merge it locally, then retry")
}

/// MOD-133 M3 D5: the remote's run branch is not an ancestor of the local one: `git push`
/// refused the fast-forward, so the remote holds commits of the pull request this box lacks.
#[must_use]
pub fn run_branch_non_fast_forward(branch: &str) -> String {
    format!(
        "non_fast_forward: `{branch}` on the remote has commits this box lacks; merge them into \
         the local branch, then retry"
    )
}

/// Any other non-zero `git push`, with its scrubbed stderr line.
#[must_use]
pub fn push_failed(tail: &str) -> String {
    format!("push_failed: {tail}")
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};
    use htui_core::fixtures::ids;
    use htui_core::model::{RepoId, RunStep, RunStepCommit, StepId, StepRole, StepStatus};
    use uuid::Uuid;

    use super::{landed_commits, not_on_target, run_branches, short_hash};

    fn repo(n: u128) -> RepoId {
        RepoId::from_uuid(Uuid::from_u128(0x200 + n))
    }

    fn step(position: i32, fanout_index: i32, status: StepStatus, role: StepRole) -> RunStep {
        let at = Utc
            .with_ymd_and_hms(2026, 11, 1, 9, 0, 0)
            .single()
            .expect("a valid instant");
        RunStep {
            id: StepId::new(),
            run_id: ids::RUN_1,
            position,
            attempt: 1,
            fanout_index,
            phase_name: format!("p{position}"),
            agent_id: None,
            model: None,
            status,
            gate_outcome: None,
            gate_note: None,
            selected: None,
            exit_code: None,
            prompt_digest: None,
            trim_record: None,
            usage: None,
            isolation_path: None,
            started_at: Some(at),
            finished_at: Some(at),
            verify_outcome: None,
            verify_exit_code: None,
            promoted_at: None,
            branch: None,
            role,
            parent_step_id: None,
            node_key: None,
            updated_at: at,
        }
    }

    fn selected(mut row: RunStep, selected: bool) -> RunStep {
        row.selected = Some(selected);
        row
    }

    /// One commit row per `(repo, after)` for `step`.
    fn commits(step: &RunStep, rows: &[(RepoId, Option<&str>)]) -> (StepId, Vec<RunStepCommit>) {
        (
            step.id,
            rows.iter()
                .map(|(repo, after)| RunStepCommit {
                    run_step_id: step.id,
                    repo_id: *repo,
                    before_hash: "b".repeat(40),
                    after_hash: after.map(str::to_owned),
                })
                .collect(),
        )
    }

    /// A fanned-out slot lands its selected candidate only: not its losers, not its judge; a
    /// `fan_out = 1` row lands; a row that is not `done` does not; another repo's rows are not
    /// this repo's; a step that committed nothing adds nothing.
    #[test]
    fn a_fan_out_run_lands_its_winner_and_its_plain_rows() {
        let plan = step(0, 0, StepStatus::Done, StepRole::Step);
        let loser = selected(step(1, 0, StepStatus::Superseded, StepRole::Step), false);
        let winner = selected(step(1, 1, StepStatus::Done, StepRole::Step), true);
        let done_loser = selected(step(1, 2, StepStatus::Done, StepRole::Step), false);
        let judge = step(1, -1, StepStatus::Done, StepRole::Step);
        let failed = step(2, 0, StepStatus::Failed, StepRole::Step);
        let quiet = step(3, 0, StepStatus::Done, StepRole::Step);
        let steps = [
            plan.clone(),
            loser.clone(),
            winner.clone(),
            done_loser.clone(),
            judge.clone(),
            failed.clone(),
            quiet.clone(),
        ];
        let rows = [
            commits(&plan, &[(repo(1), Some("plan")), (repo(2), Some("docs"))]),
            commits(&loser, &[(repo(1), Some("loser"))]),
            commits(&winner, &[(repo(1), Some("merge"))]),
            commits(&done_loser, &[(repo(1), Some("other"))]),
            commits(&judge, &[(repo(1), Some("judge"))]),
            commits(&failed, &[(repo(1), Some("failed"))]),
            commits(&quiet, &[(repo(1), None)]),
        ];
        assert_eq!(landed_commits(&steps, &rows, repo(1)), ["plan", "merge"]);
        assert_eq!(landed_commits(&steps, &rows, repo(2)), ["docs"]);
        assert!(landed_commits(&steps, &rows, repo(3)).is_empty());
    }

    /// A divided node lands through its integrate row: the planner's and the parts' commits are
    /// not landed commits, and neither is a stage row's; a repeated hash is listed once.
    #[test]
    fn a_divided_run_lands_its_integrate_row_not_its_parts() {
        let plan = step(0, 0, StepStatus::Done, StepRole::Step);
        let planner = step(1, 0, StepStatus::Done, StepRole::Planner);
        let part_a = step(2, 0, StepStatus::Done, StepRole::Part);
        let part_b = step(3, 0, StepStatus::Done, StepRole::Part);
        let integrate = step(4, 0, StepStatus::Done, StepRole::Integrate);
        let review = step(5, 0, StepStatus::Done, StepRole::Step);
        let capture = step(-4, 0, StepStatus::Done, StepRole::Stage);
        let steps = [
            plan.clone(),
            planner.clone(),
            part_a.clone(),
            part_b.clone(),
            integrate.clone(),
            review.clone(),
            capture.clone(),
        ];
        let rows = [
            commits(&plan, &[(repo(1), Some("plan"))]),
            commits(&planner, &[(repo(1), Some("planner"))]),
            commits(&part_a, &[(repo(1), Some("part-a"))]),
            commits(&part_b, &[(repo(1), Some("part-b"))]),
            commits(&integrate, &[(repo(1), Some("integrate"))]),
            commits(&review, &[(repo(1), Some("integrate"))]),
            commits(&capture, &[(repo(1), Some("capture"))]),
        ];
        assert_eq!(
            landed_commits(&steps, &rows, repo(1)),
            ["plan", "integrate"]
        );
    }

    /// MOD-133 M3 D2: only `integrate` entries with a run branch are run branches; a run with
    /// none (`local`, `push`) answers an empty map.
    #[test]
    fn run_branches_are_the_pull_request_entries() {
        use htui_core::fixtures::demo_data;
        use htui_core::model::{GraphSnapshot, IntegrateMode, IntegrateSnapshot};

        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        let mut snapshot: GraphSnapshot =
            serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
                .expect("the fixture snapshot is a `GraphSnapshot`");
        assert!(run_branches(&snapshot).is_empty());

        let entry = |mode, run_branch: Option<&str>| IntegrateSnapshot {
            mode,
            remote: "git@example.com:o/r.git".to_owned(),
            target_branch: "main".to_owned(),
            run_branch: run_branch.map(str::to_owned),
        };
        snapshot
            .integrate
            .insert(repo(1), entry(IntegrateMode::Push, None));
        snapshot.integrate.insert(
            repo(2),
            entry(IntegrateMode::PullRequest, Some("htui/item/FEAT-3")),
        );
        assert_eq!(
            run_branches(&snapshot),
            [(repo(2), "htui/item/FEAT-3".to_owned())]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn a_hash_is_shown_short() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(short_hash(hash), "0123456");
        assert_eq!(short_hash("abc"), "abc");
        assert_eq!(
            not_on_target(hash, "main"),
            "not_on_target: 0123456 is not on main"
        );
    }
}
