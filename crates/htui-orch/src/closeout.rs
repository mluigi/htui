//! ANA-2 §4.10's close-out summary (`docs/ANA-2.md:1380-1392`), pure: rows in, one `NewDocument`
//! out. The engine reads and writes; this module formats (plan D167, blueprint D208).
//!
//! [`preview`] and [`summary`] build their commit table through the same private row builder, so
//! the count the first confirmation shows is by construction the length of the table the second
//! one writes.

use chrono::{DateTime, Utc};
use htui_core::model::{
    DocumentHead, DocumentId, Item, NewDocument, Repo, RunStepCommit, RunSummary, Status, StepId,
    UserId,
};

/// `document.kind` of the close-out summary (ANA-2 §4.10).
const KIND: &str = "summary";

/// What the first confirmation shows (plan D167).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// `item.key`: the text the second confirmation asks to be typed back.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status` now.
    pub status: Status,
    /// Runs of the item the summary lists.
    pub runs: usize,
    /// `(repo, step)` rows with an `after_hash`: the commit table's length.
    pub rows: usize,
    /// The `summary` document's version once written: `max(existing) + 1`, or 1.
    pub version: i32,
}

/// The first confirmation's figures, counted over exactly what [`summary`] would write.
///
/// `version` reads only `heads` of this item whose kind is `summary`.
#[must_use]
pub fn preview(
    item: &Item,
    runs: &[RunSummary],
    commits: &[(StepId, Vec<RunStepCommit>)],
    heads: &[DocumentHead],
) -> Preview {
    todo!("closeout::preview({item:?}, {runs:?}, {commits:?}, {heads:?})")
}

/// The close-out summary document (blueprint D208): `kind = "summary"`, `title = "Close-out
/// <key>"`, written by `user` at `at` and produced by no step.
///
/// The body is a heading, one table row per `(step, repo)` whose `after_hash` is `Some`, and one
/// line per run, oldest first. Rows are ordered by the run's `queued_at`, then the step's
/// `(position, attempt, fanout_index)`, then the repo name. A repo missing from `repos` renders
/// its id; a step no listed run carries renders its id as the phase and `-` as its position and
/// attempt, and sorts last.
#[must_use]
pub fn summary(
    item: &Item,
    runs: &[RunSummary],
    commits: &[(StepId, Vec<RunStepCommit>)],
    repos: &[Repo],
    id: DocumentId,
    user: UserId,
    at: DateTime<Utc>,
) -> NewDocument {
    todo!("closeout::summary({item:?}, {runs:?}, {commits:?}, {repos:?}, {id}, {user}, {at})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use htui_core::model::{
        BoxId, ItemId, ItemKindId, ProjectId, RepoId, RunId, RunKind, RunMode, RunStatus,
        RunStepSummary, StepStatus,
    };
    use uuid::Uuid;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0)
            .single()
            .expect("a valid fixed instant")
    }

    fn project() -> ProjectId {
        ProjectId::from_uuid(Uuid::from_u128(0x10))
    }

    fn user() -> UserId {
        UserId::from_uuid(Uuid::from_u128(0x20))
    }

    fn item() -> Item {
        Item {
            id: ItemId::from_uuid(Uuid::from_u128(0x30)),
            project_id: project(),
            kind_id: ItemKindId::from_uuid(Uuid::from_u128(0x31)),
            key_prefix: "FEAT".to_owned(),
            key_number: 1,
            key: "FEAT-1".to_owned(),
            title: "Close me".to_owned(),
            body: String::new(),
            status: Status::Done,
            priority: 0,
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            step_graph_id: None,
            version: 1,
            created_by: user(),
            created_at: at(1, 9),
            updated_at: at(1, 9),
            closed_at: None,
        }
    }

    fn step_id(n: u128) -> StepId {
        StepId::from_uuid(Uuid::from_u128(0x100 + n))
    }

    fn repo_id(n: u128) -> RepoId {
        RepoId::from_uuid(Uuid::from_u128(0x200 + n))
    }

    fn run_id(n: u128) -> RunId {
        RunId::from_uuid(Uuid::from_u128(0x300 + n))
    }

    fn step(
        n: u128,
        position: i32,
        attempt: i32,
        fanout_index: i32,
        phase: &str,
    ) -> RunStepSummary {
        RunStepSummary {
            id: step_id(n),
            position,
            attempt,
            fanout_index,
            phase_name: phase.to_owned(),
            agent_id: None,
            model: None,
            status: StepStatus::Done,
            gate_outcome: None,
            started_at: None,
            finished_at: None,
            prompt_tokens: None,
            trimmed: false,
            usage: None,
            selected: None,
            exit_code: None,
            verify_outcome: None,
            promoted_at: None,
            agent_name: None,
        }
    }

    fn run(
        n: u128,
        queued_at: DateTime<Utc>,
        status: RunStatus,
        finished_at: Option<DateTime<Utc>>,
        steps: Vec<RunStepSummary>,
    ) -> RunSummary {
        RunSummary {
            id: run_id(n),
            item_id: Some(item().id),
            project_id: project(),
            kind: RunKind::Graph,
            mode: RunMode::Manual,
            status,
            target_box_id: BoxId::from_uuid(Uuid::from_u128(0x40)),
            executing_box_id: None,
            box_hostname: "dev-01".to_owned(),
            queued_at,
            started_at: None,
            finished_at,
            failure: None,
            steps,
        }
    }

    fn repo(n: u128, name: &str) -> Repo {
        Repo {
            id: repo_id(n),
            project_id: project(),
            name: name.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary: n == 1,
            created_at: at(1, 9),
            updated_at: at(1, 9),
        }
    }

    fn commit(step: u128, repo: u128, before: &str, after: Option<&str>) -> RunStepCommit {
        RunStepCommit {
            run_step_id: step_id(step),
            repo_id: repo_id(repo),
            before_hash: before.to_owned(),
            after_hash: after.map(str::to_owned),
        }
    }

    fn head(version: i32, kind: &str) -> DocumentHead {
        DocumentHead {
            id: DocumentId::from_uuid(Uuid::from_u128(0x500 + u128::from(version.unsigned_abs()))),
            item_id: item().id,
            kind: kind.to_owned(),
            version,
            title: "Close-out FEAT-1".to_owned(),
            produced_by_step_id: None,
            created_by: user(),
            created_at: at(1, 9),
        }
    }

    fn write(
        runs: &[RunSummary],
        commits: &[(StepId, Vec<RunStepCommit>)],
        repos: &[Repo],
    ) -> NewDocument {
        summary(
            &item(),
            runs,
            commits,
            repos,
            DocumentId::from_uuid(Uuid::from_u128(0x600)),
            user(),
            at(3, 8),
        )
    }

    /// The commit table's body rows: every `| …` line but the header and the separator.
    fn table_rows(body: &str) -> Vec<&str> {
        body.lines()
            .filter(|line| line.starts_with("| ") && !line.starts_with("| repo |"))
            .collect()
    }

    #[test]
    fn one_row_per_repo_and_step_with_an_after_hash() {
        let runs = [run(
            1,
            at(2, 10),
            RunStatus::Done,
            Some(at(2, 12)),
            vec![step(1, 1, 1, 0, "plan"), step(2, 2, 1, 0, "implement")],
        )];
        let commits = [
            (
                step_id(1),
                vec![
                    commit(1, 1, "aaa0001", Some("bbb0001")),
                    commit(1, 2, "ccc0001", Some("ddd0001")),
                ],
            ),
            (step_id(2), vec![commit(2, 1, "bbb0001", Some("eee0001"))]),
        ];
        let repos = [repo(1, "htui"), repo(2, "docs")];

        let doc = write(&runs, &commits, &repos);
        let rows = table_rows(&doc.body);

        assert_eq!(
            rows,
            [
                "| docs | 1 | plan | 1 | ccc0001..ddd0001 |",
                "| htui | 1 | plan | 1 | aaa0001..bbb0001 |",
                "| htui | 2 | implement | 1 | bbb0001..eee0001 |",
            ],
            "one row per (step, repo), hashes as stored:\n{}",
            doc.body
        );
    }

    #[test]
    fn a_step_that_committed_nothing_has_no_row() {
        let runs = [run(
            1,
            at(2, 10),
            RunStatus::Done,
            Some(at(2, 12)),
            vec![
                step(1, 1, 1, 0, "plan"),
                step(2, 2, 1, 0, "implement"),
                step(3, 3, 1, 0, "review"),
            ],
        )];
        let commits = [
            // A tree that was prepared but never committed: `after_hash` is `None`.
            (step_id(1), vec![commit(1, 1, "aaa0001", None)]),
            (step_id(2), vec![commit(2, 1, "aaa0001", Some("fff0002"))]),
            // A step with no `run_step_commit` row at all.
            (step_id(3), Vec::new()),
        ];
        let repos = [repo(1, "htui")];

        let doc = write(&runs, &commits, &repos);

        assert_eq!(
            table_rows(&doc.body),
            ["| htui | 2 | implement | 1 | aaa0001..fff0002 |"],
            "{}",
            doc.body
        );
    }

    #[test]
    fn rows_are_in_position_attempt_repo_order() {
        // The older run is listed second on purpose: `ReadStore::runs` answers newest first.
        let runs = [
            run(
                2,
                at(2, 14),
                RunStatus::Done,
                Some(at(2, 16)),
                vec![step(20, 1, 1, 0, "implement")],
            ),
            run(
                1,
                at(2, 10),
                RunStatus::Failed,
                Some(at(2, 11)),
                vec![
                    step(10, 2, 1, 1, "implement"),
                    step(11, 2, 1, 0, "implement"),
                    step(12, 1, 2, 0, "plan"),
                    step(13, 1, 1, 0, "plan"),
                ],
            ),
        ];
        // Shuffled, and each step committed to both repos: the order is the module's, not the
        // input's.
        let commits = [
            (
                step_id(20),
                vec![
                    commit(20, 1, "r2a", Some("r2b")),
                    commit(20, 2, "r2c", Some("r2d")),
                ],
            ),
            (step_id(10), vec![commit(10, 1, "f1a", Some("f1b"))]),
            (
                step_id(12),
                vec![
                    commit(12, 1, "p2a", Some("p2b")),
                    commit(12, 2, "p2c", Some("p2d")),
                ],
            ),
            (step_id(11), vec![commit(11, 3, "f0a", Some("f0b"))]),
            (step_id(13), vec![commit(13, 2, "p1a", Some("p1b"))]),
        ];
        // Repo 3 is not in the list, so its row renders the id.
        let repos = [repo(1, "zeta"), repo(2, "alpha")];

        let doc = write(&runs, &commits, &repos);

        assert_eq!(
            table_rows(&doc.body),
            [
                "| alpha | 1 | plan | 1 | p1a..p1b |".to_owned(),
                "| alpha | 1 | plan | 2 | p2c..p2d |".to_owned(),
                "| zeta | 1 | plan | 2 | p2a..p2b |".to_owned(),
                format!("| {} | 2 | implement | 1 | f0a..f0b |", repo_id(3)),
                "| zeta | 2 | implement | 1 | f1a..f1b |".to_owned(),
                "| alpha | 1 | implement | 1 | r2c..r2d |".to_owned(),
                "| zeta | 1 | implement | 1 | r2a..r2b |".to_owned(),
            ],
            "run queued_at, then (position, attempt, fanout_index), then repo name:\n{}",
            doc.body
        );
        let lines: Vec<&str> = doc.body.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(
            lines,
            [
                format!(
                    "- graph run {}: failed, finished 2026-09-02 11:00 UTC",
                    run_id(1)
                ),
                format!(
                    "- graph run {}: done, finished 2026-09-02 16:00 UTC",
                    run_id(2)
                ),
            ],
            "run lines are oldest first"
        );
    }

    #[test]
    fn the_summary_is_kind_summary_produced_by_nobody() {
        let runs = [
            run(
                2,
                at(2, 14),
                RunStatus::Cancelled,
                None,
                vec![step(2, 1, 1, 0, "implement")],
            ),
            run(
                1,
                at(2, 10),
                RunStatus::Done,
                Some(at(2, 12)),
                vec![step(1, 2, 1, 0, "implement")],
            ),
        ];
        let commits = [(step_id(1), vec![commit(1, 1, "0a1b2c3", Some("9f8e7d6"))])];
        let repos = [repo(1, "htui")];

        let doc = write(&runs, &commits, &repos);

        assert_eq!(doc.id, DocumentId::from_uuid(Uuid::from_u128(0x600)));
        assert_eq!(doc.item_id, item().id);
        assert_eq!(doc.kind, "summary");
        assert_eq!(doc.title, "Close-out FEAT-1");
        assert_eq!(
            doc.produced_by_step_id, None,
            "written by hand, not by a step"
        );
        assert_eq!(doc.created_by, user());
        assert_eq!(doc.created_at, at(3, 8));
        assert_eq!(
            doc.body,
            format!(
                "# Close-out FEAT-1 — Close me\n\
                 \n\
                 | repo | position | phase | attempt | commits |\n\
                 |---|---|---|---|---|\n\
                 | htui | 2 | implement | 1 | 0a1b2c3..9f8e7d6 |\n\
                 \n\
                 - graph run {}: done, finished 2026-09-02 12:00 UTC\n\
                 - graph run {}: cancelled\n",
                run_id(1),
                run_id(2)
            ),
            "blueprint D208's body"
        );
    }

    #[test]
    fn the_preview_counts_what_the_summary_holds() {
        let runs = [run(
            1,
            at(2, 10),
            RunStatus::Done,
            Some(at(2, 12)),
            vec![step(1, 1, 1, 0, "plan"), step(2, 2, 1, 0, "implement")],
        )];
        let commits = [
            (
                step_id(1),
                vec![commit(1, 1, "a", Some("b")), commit(1, 2, "c", None)],
            ),
            (step_id(2), vec![commit(2, 1, "b", Some("d"))]),
        ];
        let repos = [repo(1, "htui"), repo(2, "docs")];
        let doc = write(&runs, &commits, &repos);

        let first = preview(&item(), &runs, &commits, &[]);
        assert_eq!(
            first,
            Preview {
                key: "FEAT-1".to_owned(),
                title: "Close me".to_owned(),
                status: Status::Done,
                runs: 1,
                rows: table_rows(&doc.body).len(),
                version: 1,
            }
        );
        assert_eq!(first.rows, 2);

        // Another kind's versions are not the summary's.
        let heads = [head(1, "summary"), head(2, "summary"), head(7, "plan")];
        assert_eq!(preview(&item(), &runs, &commits, &heads).version, 3);
    }
}
