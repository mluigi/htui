//! ANA-2 §4.10's close-out summary (`docs/ANA-2.md:1380-1392`), pure: rows in, one `NewDocument`
//! out. The engine reads and writes; this module formats (plan D167, blueprint D208).
//!
//! [`preview`] and [`summary`] build their commit table through the same private row builder, so
//! the count the first confirmation shows is by construction the length of the table the second
//! one writes.

use chrono::{DateTime, Utc};
use htui_core::model::{
    DocumentHead, DocumentId, Integration, IntegrationState, Item, ItemId, LinkGraph, LinkKind,
    NewDocument, Repo, RepoId, Resolution, RunId, RunStepCommit, RunSummary, Status, StepId,
    UserId,
};

use crate::infer::redact_remote;
use crate::land::short_hash;

/// `document.kind` of the close-out summary (ANA-2 §4.10).
const KIND: &str = "summary";

/// What the first confirmation shows (plan D167).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// `item.key`: the text the second confirmation asks to be typed back.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status` now: `open` or `blocked`.
    pub status: Status,
    /// Whether the item's latest graph run finished `done` (MOD-144): the second input of
    /// `Resolution::closes_from`.
    pub finished: bool,
    /// The resolution the confirmation starts on: `close_out_enabled`'s answer. The Runs pane
    /// may send another that `closes_from(status, finished)` (MOD-39 plan P13).
    pub resolution: Resolution,
    /// Runs of the item the summary lists.
    pub runs: usize,
    /// `(repo, step)` rows with an `after_hash`: the commit table's length.
    pub rows: usize,
    /// The `summary` document's version once written: `max(existing) + 1`, or 1.
    pub version: i32,
    /// MOD-150 M2 D7: the item's live `duplicates` canonical, labelled `KEY` in the item's
    /// project and `slug:KEY` outside it; `None` without one.
    pub duplicate_of: Option<String>,
    /// MOD-150 M2 D7: the items with a live `supersedes` link to this one, labelled as
    /// [`Self::duplicate_of`] is, sorted.
    pub superseded_by: Vec<String>,
}

/// The first confirmation's figures, counted over exactly what [`summary`] would write.
///
/// `version` reads only `heads` of this item whose kind is `summary`; `resolution` and `finished`
/// are the caller's, carried through so the confirmation and the command name the same one.
/// `links` is the item's one-hop link graph: its live `duplicates` edge from the item and its
/// live `supersedes` edges to it are the standing close-out links (MOD-150 M2 D7).
#[must_use]
pub fn preview(
    item: &Item,
    resolution: Resolution,
    finished: bool,
    runs: &[RunSummary],
    commits: &[(StepId, Vec<RunStepCommit>)],
    heads: &[DocumentHead],
    links: &LinkGraph,
) -> Preview {
    let version = heads
        .iter()
        .filter(|head| head.item_id == item.id && head.kind == KIND)
        .map(|head| head.version)
        .max()
        .map_or(1, |latest| latest.saturating_add(1));
    Preview {
        key: item.key.clone(),
        title: item.title.clone(),
        status: item.status,
        finished,
        resolution,
        runs: runs.len(),
        rows: rows(runs, commits, &[]).len(),
        version,
        duplicate_of: links
            .edges
            .iter()
            .find(|edge| edge.kind == LinkKind::Duplicates && edge.from_item_id == item.id)
            .map(|edge| label(item, links, edge.to_item_id)),
        superseded_by: {
            let mut by: Vec<String> = links
                .edges
                .iter()
                .filter(|edge| edge.kind == LinkKind::Supersedes && edge.to_item_id == item.id)
                .map(|edge| label(item, links, edge.from_item_id))
                .collect();
            by.sort();
            by
        },
    }
}

/// MOD-150 M2 D7: how the preview names a linked item: `KEY` in `item`'s project, `slug:KEY`
/// outside it, and the id when the traversal did not reach it.
fn label(item: &Item, links: &LinkGraph, id: ItemId) -> String {
    links.node(id).map_or_else(
        || id.to_string(),
        |node| {
            if node.project_id == item.project_id {
                node.key.clone()
            } else {
                format!("{}:{}", node.project_slug, node.key)
            }
        },
    )
}

/// The close-out summary document (blueprint D208): `kind = "summary"`, `title = "Close-out
/// <key>"`, written by `user` at `at` and produced by no step.
///
/// The body is a heading, one table row per `(step, repo)` whose `after_hash` is `Some`, and one
/// line per run, oldest first. Rows are ordered by the run's `queued_at`, then the step's
/// `(position, attempt, fanout_index)`, then the repo name. A repo missing from `repos` renders
/// its id; a step no listed run carries renders its id as the phase and `-` as its position and
/// attempt, and sorts last.
///
/// MOD-133 D11: `integrations`, the `integration` records of the item's runs, append an
/// `## Integration` table after the run lines, one row per record: the repo's name, the run, the
/// mode, the remote (without userinfo) and target, and the outcome (`landed <short hash>`,
/// `refused: <reason>`, `pending`, or a PR state's wire name). Rows follow the run lines' order,
/// then the repo name. No record writes no section, so the body is byte-identical to a `local`
/// item's.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "MOD-133 D11: the integration records are the eighth input of one pure formatter, \
              each a separate read of the engine's"
)]
pub fn summary(
    item: &Item,
    runs: &[RunSummary],
    commits: &[(StepId, Vec<RunStepCommit>)],
    repos: &[Repo],
    id: DocumentId,
    user: UserId,
    at: DateTime<Utc>,
    integrations: &[Integration],
) -> NewDocument {
    let mut body = format!("# Close-out {} — {}\n\n", item.key, item.title);
    body.push_str("| repo | position | phase | attempt | commits |\n|---|---|---|---|---|\n");
    for row in rows(runs, commits, repos) {
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&row.repo),
            row.position,
            cell(&row.phase),
            row.attempt,
            cell(&row.commits),
        ));
    }
    let mut oldest_first: Vec<&RunSummary> = runs.iter().collect();
    oldest_first.sort_by_key(|run| (run.queued_at, run.id));
    if !oldest_first.is_empty() {
        body.push('\n');
    }
    for run in &oldest_first {
        body.push_str(&format!("- {} run {}: {}", run.kind, run.id, run.status));
        if let Some(finished) = run.finished_at {
            body.push_str(&format!(
                ", finished {}",
                finished.format("%Y-%m-%d %H:%M UTC")
            ));
        }
        body.push('\n');
    }
    integration_section(&mut body, &oldest_first, repos, integrations);
    NewDocument {
        id,
        item_id: item.id,
        kind: KIND.to_owned(),
        title: format!("Close-out {}", item.key),
        body,
        produced_by_step_id: None,
        created_by: user,
        created_at: at,
    }
}

/// MOD-133 D11: the `## Integration` table, appended to `body` when `integrations` is not empty.
fn integration_section(
    body: &mut String,
    oldest_first: &[&RunSummary],
    repos: &[Repo],
    integrations: &[Integration],
) {
    if integrations.is_empty() {
        return;
    }
    let repo_name = |record: &Integration| {
        repos
            .iter()
            .find(|repo| repo.id == record.repo_id)
            .map_or_else(|| record.repo_id.to_string(), |repo| repo.name.clone())
    };
    let mut rows: Vec<(usize, String, RepoId, &Integration)> = integrations
        .iter()
        .map(|record| {
            let run = oldest_first
                .iter()
                .position(|run| run.id == record.run_id)
                .unwrap_or(usize::MAX);
            (run, repo_name(record), record.repo_id, record)
        })
        .collect();
    rows.sort_by(|a, b| (a.0, a.3.run_id, &a.1, a.2).cmp(&(b.0, b.3.run_id, &b.1, b.2)));
    body.push_str(
        "\n## Integration\n\n| repo | run | mode | target | outcome |\n|---|---|---|---|---|\n",
    );
    for (_, repo, _, record) in rows {
        let outcome = match record.state {
            IntegrationState::Landed => format!(
                "landed {}",
                record.landed_hash.as_deref().map_or("?", short_hash)
            ),
            IntegrationState::Refused => {
                format!("refused: {}", record.refusal.as_deref().unwrap_or_default())
            }
            other => other.as_str().to_owned(),
        };
        body.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&repo),
            record.run_id,
            record.mode,
            cell(&format!(
                "{} {}",
                redact_remote(&record.remote),
                record.target_branch
            )),
            cell(&outcome),
        ));
    }
}

/// Where a row sorts: the run's `queued_at` and id, then the step's `(position, attempt,
/// fanout_index)`.
type RowKey = (DateTime<Utc>, RunId, i32, i32, i32);

/// One line of the commit table, before it is rendered.
struct Row {
    /// `None` for a step no listed run carries, which sorts after every known one.
    key: Option<RowKey>,
    /// The step, which keeps an unknown step's rows together.
    step: StepId,
    /// `repo.name`, or the id when `repos` does not carry it.
    repo: String,
    /// The repo id, the last tie-break.
    repo_id: RepoId,
    /// `run_step.position`, or `-`.
    position: String,
    /// `run_step.phase_name`, or the step id.
    phase: String,
    /// `run_step.attempt`, or `-`.
    attempt: String,
    /// `before_hash..after_hash`, as stored.
    commits: String,
}

/// Every `(step, repo)` with an `after_hash`, in blueprint D208's order. `repos` only names the
/// rows, so [`preview`] passes none and counts the same rows [`summary`] renders.
fn rows(runs: &[RunSummary], commits: &[(StepId, Vec<RunStepCommit>)], repos: &[Repo]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (step_id, list) in commits {
        let step = runs.iter().find_map(|run| {
            run.steps
                .iter()
                .find(|step| step.id == *step_id)
                .map(|step| (run, step))
        });
        for commit in list {
            let Some(after) = &commit.after_hash else {
                continue;
            };
            let repo = repos
                .iter()
                .find(|repo| repo.id == commit.repo_id)
                .map_or_else(|| commit.repo_id.to_string(), |repo| repo.name.clone());
            rows.push(Row {
                key: step.map(|(run, step)| {
                    (
                        run.queued_at,
                        run.id,
                        step.position,
                        step.attempt,
                        step.fanout_index,
                    )
                }),
                step: *step_id,
                repo,
                repo_id: commit.repo_id,
                position: step
                    .map_or_else(|| "-".to_owned(), |(_, step)| step.position.to_string()),
                phase: step
                    .map_or_else(|| step_id.to_string(), |(_, step)| step.phase_name.clone()),
                attempt: step.map_or_else(|| "-".to_owned(), |(_, step)| step.attempt.to_string()),
                commits: format!("{}..{after}", commit.before_hash),
            });
        }
    }
    rows.sort_by(|a, b| {
        (a.key.is_none(), a.key, a.step, &a.repo, a.repo_id).cmp(&(
            b.key.is_none(),
            b.key,
            b.step,
            &b.repo,
            b.repo_id,
        ))
    });
    rows
}

/// A table cell: a `|` or a line break in a name would otherwise split the row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use htui_core::model::{
        BoxId, IntegrateMode, IntegrationState, ItemKindId, LinkEdge, LinkNode, ProjectId, RunKind,
        RunMode, RunStatus, RunStepSummary, StepStatus,
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
            status: Status::Open,
            priority: 0,
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            step_graph_id: None,
            target_id: None,
            version: 1,
            created_by: user(),
            created_at: at(1, 9),
            updated_at: at(1, 9),
            closed_at: None,
            resolution: None,
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
            gate_note: None,
            opening: None,
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
            integrate: None,
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
        write_with(runs, commits, repos, &[])
    }

    fn write_with(
        runs: &[RunSummary],
        commits: &[(StepId, Vec<RunStepCommit>)],
        repos: &[Repo],
        integrations: &[Integration],
    ) -> NewDocument {
        summary(
            &item(),
            runs,
            commits,
            repos,
            DocumentId::from_uuid(Uuid::from_u128(0x600)),
            user(),
            at(3, 8),
            integrations,
        )
    }

    /// One `integration` record of run `run` in repo `repo`, in `state`.
    fn record(run: u128, repo: u128, state: IntegrationState) -> Integration {
        Integration {
            run_id: run_id(run),
            repo_id: repo_id(repo),
            project_id: project(),
            mode: IntegrateMode::Push,
            remote: "https://bot:secret@forge.example/o/htui.git".to_owned(),
            target_branch: "main".to_owned(),
            state,
            refusal: (state == IntegrationState::Refused)
                .then(|| "non_fast_forward: main on the remote has moved".to_owned()),
            landed_hash: (state == IntegrationState::Landed)
                .then(|| "0123456789abcdef0123456789abcdef01234567".to_owned()),
            pr_ref: None,
            pr_url: None,
            created_at: at(2, 12),
            updated_at: at(2, 12),
        }
    }

    /// MOD-133 D11: the integration section follows the run lines, one row per record, oldest
    /// run first and by repo name; the remote is shown without its userinfo.
    #[test]
    fn the_summary_lists_each_integration_record_after_the_runs() {
        let runs = [
            run(
                2,
                at(2, 14),
                RunStatus::Done,
                Some(at(2, 16)),
                vec![step(2, 1, 1, 0, "implement")],
            ),
            run(
                1,
                at(2, 10),
                RunStatus::Done,
                Some(at(2, 12)),
                vec![step(1, 1, 1, 0, "implement")],
            ),
        ];
        let commits = [(step_id(1), vec![commit(1, 1, "0a1b2c3", Some("9f8e7d6"))])];
        let repos = [repo(1, "htui"), repo(2, "docs")];
        let integrations = [
            record(2, 1, IntegrationState::Pending),
            record(1, 1, IntegrationState::Landed),
            record(1, 2, IntegrationState::Refused),
            record(2, 2, IntegrationState::PrOpen),
        ];

        let doc = write_with(&runs, &commits, &repos, &integrations);

        assert_eq!(
            doc.body,
            format!(
                "# Close-out FEAT-1 — Close me\n\
                 \n\
                 | repo | position | phase | attempt | commits |\n\
                 |---|---|---|---|---|\n\
                 | htui | 1 | implement | 1 | 0a1b2c3..9f8e7d6 |\n\
                 \n\
                 - graph run {r1}: done, finished 2026-09-02 12:00 UTC\n\
                 - graph run {r2}: done, finished 2026-09-02 16:00 UTC\n\
                 \n\
                 ## Integration\n\
                 \n\
                 | repo | run | mode | target | outcome |\n\
                 |---|---|---|---|---|\n\
                 | docs | {r1} | push | https://forge.example/o/htui.git main | refused: \
                 non_fast_forward: main on the remote has moved |\n\
                 | htui | {r1} | push | https://forge.example/o/htui.git main | landed 0123456 |\n\
                 | docs | {r2} | push | https://forge.example/o/htui.git main | pr_open |\n\
                 | htui | {r2} | push | https://forge.example/o/htui.git main | pending |\n",
                r1 = run_id(1),
                r2 = run_id(2),
            ),
        );
        assert!(!doc.body.contains("secret"), "no userinfo is written");
    }

    /// MOD-133 D11: no record, today's bytes exactly.
    #[test]
    fn no_integration_record_keeps_the_summary_byte_identical() {
        let runs = [run(
            1,
            at(2, 10),
            RunStatus::Done,
            Some(at(2, 12)),
            vec![step(1, 1, 1, 0, "implement")],
        )];
        let commits = [(step_id(1), vec![commit(1, 1, "0a1b2c3", Some("9f8e7d6"))])];
        let repos = [repo(1, "htui")];
        assert_eq!(
            write_with(&runs, &commits, &repos, &[]).body,
            format!(
                "# Close-out FEAT-1 — Close me\n\
                 \n\
                 | repo | position | phase | attempt | commits |\n\
                 |---|---|---|---|---|\n\
                 | htui | 1 | implement | 1 | 0a1b2c3..9f8e7d6 |\n\
                 \n\
                 - graph run {}: done, finished 2026-09-02 12:00 UTC\n",
                run_id(1)
            )
        );
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
        let repos = [repo(1, "omega"), repo(2, "alpha")];

        let doc = write(&runs, &commits, &repos);

        assert_eq!(
            table_rows(&doc.body),
            [
                "| alpha | 1 | plan | 1 | p1a..p1b |".to_owned(),
                "| alpha | 1 | plan | 2 | p2c..p2d |".to_owned(),
                "| omega | 1 | plan | 2 | p2a..p2b |".to_owned(),
                format!("| {} | 2 | implement | 1 | f0a..f0b |", repo_id(3)),
                "| omega | 2 | implement | 1 | f1a..f1b |".to_owned(),
                "| alpha | 1 | implement | 1 | r2c..r2d |".to_owned(),
                "| omega | 1 | implement | 1 | r2a..r2b |".to_owned(),
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

        let first = preview(
            &item(),
            Resolution::Done,
            true,
            &runs,
            &commits,
            &[],
            &unlinked(),
        );
        assert_eq!(
            first,
            Preview {
                key: "FEAT-1".to_owned(),
                title: "Close me".to_owned(),
                status: Status::Open,
                finished: true,
                resolution: Resolution::Done,
                runs: 1,
                rows: table_rows(&doc.body).len(),
                version: 1,
                duplicate_of: None,
                superseded_by: Vec::new(),
            }
        );
        assert_eq!(first.rows, 2);

        // Another kind's versions are not the summary's.
        let heads = [head(1, "summary"), head(2, "summary"), head(7, "plan")];
        assert_eq!(
            preview(
                &item(),
                Resolution::Done,
                true,
                &runs,
                &commits,
                &heads,
                &unlinked()
            )
            .version,
            3
        );
    }

    /// The item's own graph node, alone: no link stands.
    fn unlinked() -> LinkGraph {
        LinkGraph {
            root: item().id,
            nodes: vec![node(item().id, project(), "htui", "FEAT-1", 0)],
            edges: Vec::new(),
            documents: vec![],
        }
    }

    fn node(item_id: ItemId, project_id: ProjectId, slug: &str, key: &str, depth: u8) -> LinkNode {
        LinkNode {
            item_id,
            project_id,
            project_slug: slug.to_owned(),
            key: key.to_owned(),
            title: format!("{key} title"),
            status: Status::Open,
            depth,
            latest_run_status: None,
            latest_run_landing: None,
        }
    }

    fn edge(from_item_id: ItemId, kind: LinkKind, to_item_id: ItemId) -> LinkEdge {
        LinkEdge {
            from_item_id,
            to_item_id,
            kind,
        }
    }

    /// MOD-150 M2 D7: the preview carries the standing `duplicates` link from the item and every
    /// `supersedes` link to it, sorted, as `KEY` in the item's project and `slug:KEY` outside it.
    /// Other kinds, and links of the item's neighbours, are not the item's close-out links.
    #[test]
    fn the_preview_carries_the_standing_links() {
        let me = item().id;
        let other = ProjectId::from_uuid(Uuid::from_u128(0x11));
        let canonical = ItemId::from_uuid(Uuid::from_u128(0x50));
        let (late, early, relates) = (
            ItemId::from_uuid(Uuid::from_u128(0x51)),
            ItemId::from_uuid(Uuid::from_u128(0x52)),
            ItemId::from_uuid(Uuid::from_u128(0x53)),
        );
        let graph = LinkGraph {
            root: me,
            nodes: vec![
                node(me, project(), "htui", "FEAT-1", 0),
                node(canonical, other, "agy", "FIX-3", 1),
                node(late, project(), "htui", "TOOL-9", 1),
                node(early, project(), "htui", "ANA-2", 1),
                node(relates, project(), "htui", "FEAT-7", 1),
            ],
            edges: vec![
                edge(me, LinkKind::Duplicates, canonical),
                edge(late, LinkKind::Supersedes, me),
                edge(early, LinkKind::Supersedes, me),
                edge(me, LinkKind::Supersedes, relates),
                edge(relates, LinkKind::Duplicates, me),
                edge(me, LinkKind::Relates, relates),
                edge(relates, LinkKind::Supersedes, canonical),
            ],
            documents: vec![],
        };
        let shown = preview(&item(), Resolution::Duplicate, false, &[], &[], &[], &graph);
        assert_eq!(
            (shown.duplicate_of.as_deref(), shown.superseded_by),
            (
                Some("agy:FIX-3"),
                vec!["ANA-2".to_owned(), "TOOL-9".to_owned()]
            ),
            "the canonical outside the project is qualified; the supersessors sort"
        );
        assert_eq!(
            preview(
                &item(),
                Resolution::Withdrawn,
                false,
                &[],
                &[],
                &[],
                &unlinked()
            ),
            Preview {
                key: "FEAT-1".to_owned(),
                title: "Close me".to_owned(),
                status: Status::Open,
                finished: false,
                resolution: Resolution::Withdrawn,
                runs: 0,
                rows: 0,
                version: 1,
                duplicate_of: None,
                superseded_by: Vec::new(),
            },
            "no link stands"
        );
    }
}
