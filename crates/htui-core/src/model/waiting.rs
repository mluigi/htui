//! The waiting-on-you list's store rows (MOD-69 plan D2, D4; blueprint A-1, A-2, A-8): what
//! `ReadStore::waiting_candidates` and `WriteStore::open_permissions` answer. The list itself is
//! derived in `htui-worker` (`views::waiting`) and keeps no state of its own.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{ItemId, ProjectId, RunId};
use crate::model::item::Item;
use crate::model::relay::StepPermission;
use crate::model::run::{Run, RunStep};
use crate::model::scope::Scope;

/// One item of [`crate::store::ReadStore::waiting_candidates`] (MOD-69 plan D2): an item in scope
/// that is `blocked` or `awaiting_approval`, or that owns a run at `awaiting_approval`, with its
/// **active** runs (`RunStatus::is_active`) and each such run's steps.
///
/// Derives no `Eq`: `Item`, `Run` and `RunStep` derive none (they hold `serde_json::Value`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitingCandidate {
    /// The item row, body included: `verdicts` and `unblock_enabled` read it whole.
    pub item: Item,
    /// Its active runs, newest first (`queued_at DESC, id DESC`: `ReadStore::runs`' order, which
    /// `unblock_enabled` is sensitive to, blueprint H-1), each with its steps in
    /// `(position, attempt, fanout_index)` order.
    pub runs: Vec<(Run, Vec<RunStep>)>,
}

impl WaitingCandidate {
    /// Groups three flat reads into candidates and sorts them ([`Self::sort_canonical`]): every
    /// backend reads items, runs and steps separately and calls this, so the grouping and the order
    /// cannot differ between them (blueprint A-8). A run whose item is not in `items`, and a step
    /// whose run is not in `runs`, is dropped; an item with no run keeps `runs: []`.
    #[must_use]
    pub fn assemble(
        scope: &Scope,
        items: Vec<Item>,
        runs: Vec<Run>,
        steps: Vec<RunStep>,
    ) -> Vec<Self> {
        let mut steps_of: BTreeMap<RunId, Vec<RunStep>> = BTreeMap::new();
        for step in steps {
            steps_of.entry(step.run_id).or_default().push(step);
        }
        let mut runs_of: BTreeMap<ItemId, Vec<(Run, Vec<RunStep>)>> = BTreeMap::new();
        for run in runs {
            let Some(item) = run.item_id else { continue };
            let steps = steps_of.remove(&run.id).unwrap_or_default();
            runs_of.entry(item).or_default().push((run, steps));
        }
        let mut rows: Vec<Self> = items
            .into_iter()
            .map(|item| {
                let runs = runs_of.remove(&item.id).unwrap_or_default();
                Self { item, runs }
            })
            .collect();
        Self::sort_canonical(&mut rows, scope);
        rows
    }

    /// Items by `(scope position of project_id, key_prefix bytes, key_number, id)` (a project not
    /// in `scope` last); each item's runs by `(queued_at, id)` **descending**; each run's steps by
    /// `(position, attempt, fanout_index, id)`. Rust, not SQL: Postgres would order text by
    /// collation and the mirror by bytes (the `UpstreamEntry::sort_canonical` reason).
    pub fn sort_canonical(rows: &mut [Self], scope: &Scope) {
        let position = |project: ProjectId| {
            scope
                .project_ids
                .iter()
                .position(|id| *id == project)
                .unwrap_or(usize::MAX)
        };
        for row in rows.iter_mut() {
            for (_, steps) in &mut row.runs {
                steps.sort_by_key(|step| (step.position, step.attempt, step.fanout_index, step.id));
            }
            row.runs
                .sort_by_key(|(run, _)| Reverse((run.queued_at, run.id)));
        }
        rows.sort_by(|a, b| {
            (
                position(a.item.project_id),
                a.item.key_prefix.as_bytes(),
                a.item.key_number,
                a.item.id,
            )
                .cmp(&(
                    position(b.item.project_id),
                    b.item.key_prefix.as_bytes(),
                    b.item.key_number,
                    b.item.id,
                ))
        });
    }
}

/// One row of [`crate::store::WriteStore::open_permissions`] (MOD-69 plan D4, blueprint A-1, A-2):
/// a pending permission request of an item run whose owner holds the run's lease live, with what
/// the list sorts and labels it by (plan D9), so the classifier needs no second read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitingPermission {
    /// `run.item_id`; never a chat run's (plan D4).
    pub item: ItemId,
    /// `item.project_id`: plan D9's project position.
    pub project: ProjectId,
    /// `item.key`, e.g. `FEAT-2`.
    pub item_key: String,
    /// `item.key_prefix`: the key's sort parts, so `FEAT-2` sorts before `FEAT-10`.
    pub key_prefix: String,
    /// `item.key_number`.
    pub key_number: i32,
    /// `run.queued_at`: plan D9's run creation.
    pub run_queued_at: DateTime<Utc>,
    /// `run_step.position` of `permission.run_step_id`.
    pub step_position: i32,
    /// `run_step.attempt`.
    pub step_attempt: i32,
    /// `run_step.fanout_index` (`-1` = the slot's judge).
    pub step_fanout_index: i32,
    /// `run_step.phase_name`.
    pub phase_name: String,
    /// The request, as `relay_view` lists it.
    pub permission: StepPermission,
}

impl WaitingPermission {
    /// `(permission.created_at, permission.id)`: `relay_view`'s order, applied in Rust on every
    /// backend.
    pub fn sort_canonical(rows: &mut [Self]) {
        rows.sort_by_key(|row| (row.permission.created_at, row.permission.id));
    }
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use chrono::Duration;
    use uuid::Uuid;

    use super::*;
    use crate::fixtures::{demo_at, demo_data};
    use crate::model::ids::{PermissionId, RelaySessionId, StepId, WorkspaceId};
    use crate::model::relay::PermissionStatus;

    /// A demo item re-keyed as `prefix-number` on `project`, with a fresh id.
    fn item(project: ProjectId, prefix: &str, number: i32) -> Item {
        let mut item = demo_data().items.remove(0);
        item.id = ItemId::new();
        item.project_id = project;
        item.key_prefix = prefix.to_owned();
        item.key_number = number;
        item.key = format!("{prefix}-{number}");
        item
    }

    /// A demo run of `item` (or a chat run, `None`), queued at `queued_at`, with id `id`.
    fn run(id: RunId, item: Option<ItemId>, queued_at: DateTime<Utc>) -> Run {
        let mut run = demo_data().runs.remove(0);
        run.id = id;
        run.item_id = item;
        run.queued_at = queued_at;
        run
    }

    /// A demo step of `run` at `(position, attempt, fanout_index)`, with a fresh id.
    fn step(run: RunId, position: i32, attempt: i32, fanout_index: i32) -> RunStep {
        let mut step = demo_data().steps.remove(0);
        step.id = StepId::new();
        step.run_id = run;
        step.position = position;
        step.attempt = attempt;
        step.fanout_index = fanout_index;
        step
    }

    fn scope(project_ids: Vec<ProjectId>) -> Scope {
        Scope {
            workspace_id: WorkspaceId::new(),
            project_ids,
        }
    }

    fn run_id(n: u128) -> RunId {
        RunId::from_uuid(Uuid::from_u128(n))
    }

    #[test]
    fn waiting_candidates_assemble_by_item_and_drop_orphans() {
        let project = ProjectId::new();
        let a = item(project, "FEAT", 1);
        let b = item(project, "FEAT", 2);
        let c = item(project, "FEAT", 3);
        let t = demo_at(1, 0);
        let a_old = run(run_id(1), Some(a.id), t);
        let a_new = run(run_id(2), Some(a.id), t + Duration::hours(1));
        let orphan_run = run(run_id(3), Some(c.id), t);
        let chat_run = run(run_id(4), None, t);
        let old_step = step(a_old.id, 0, 1, 0);
        let new_steps = vec![step(a_new.id, 0, 1, 0), step(a_new.id, 1, 1, 0)];
        let orphan_step = step(orphan_run.id, 0, 1, 0);
        let lost_step = step(run_id(99), 0, 1, 0);

        let rows = WaitingCandidate::assemble(
            &scope(vec![project]),
            vec![b.clone(), a.clone()],
            vec![a_old.clone(), orphan_run, a_new.clone(), chat_run],
            vec![
                new_steps[1].clone(),
                orphan_step,
                old_step.clone(),
                lost_step,
                new_steps[0].clone(),
            ],
        );

        assert_eq!(
            rows,
            vec![
                WaitingCandidate {
                    item: a,
                    runs: vec![(a_new, new_steps), (a_old, vec![old_step])],
                },
                WaitingCandidate {
                    item: b,
                    runs: vec![],
                },
            ],
        );
    }

    #[test]
    fn waiting_candidates_sort_by_scope_key_then_runs_newest_first() {
        let (project_a, project_b, outside) =
            (ProjectId::new(), ProjectId::new(), ProjectId::new());
        let t = demo_at(1, 0);
        let later = t + Duration::hours(1);
        let (oldest, tie_low, tie_high) = (run_id(40), run_id(20), run_id(30));
        let judge = step(tie_high, 0, 1, -1);
        let fan_one = step(tie_high, 0, 1, 1);
        let retry = step(tie_high, 0, 2, 0);
        let next = step(tie_high, 1, 1, 0);
        let mut runs_item = item(project_a, "FEAT", 2);
        runs_item.title = "runs".to_owned();
        let mut rows: Vec<WaitingCandidate> = [
            item(outside, "AAA", 1),
            item(project_a, "edit", 1),
            item(project_a, "Zed", 5),
            item(project_a, "FEAT", 10),
            runs_item,
            item(project_b, "FEAT", 9),
        ]
        .into_iter()
        .map(|item| WaitingCandidate { item, runs: vec![] })
        .collect();
        if let Some(row) = rows.iter_mut().find(|row| row.item.title == "runs") {
            // Shuffled on purpose: neither the input reversed nor a stable
            // sort on `queued_at` alone yields the expected order.
            row.runs = vec![
                (run(tie_low, Some(row.item.id), later), vec![]),
                (run(oldest, Some(row.item.id), t), vec![]),
                (
                    run(tie_high, Some(row.item.id), later),
                    vec![next.clone(), retry.clone(), judge.clone(), fan_one.clone()],
                ),
            ];
        }

        WaitingCandidate::sort_canonical(&mut rows, &scope(vec![project_b, project_a]));

        let keys: Vec<&str> = rows.iter().map(|row| row.item.key.as_str()).collect();
        assert_eq!(
            keys,
            ["FEAT-9", "FEAT-2", "FEAT-10", "Zed-5", "edit-1", "AAA-1"]
        );
        let runs = &rows[1].runs;
        let run_ids: Vec<RunId> = runs.iter().map(|(run, _)| run.id).collect();
        assert_eq!(run_ids, [tie_high, tie_low, oldest]);
        let step_ids: Vec<StepId> = runs[0].1.iter().map(|step| step.id).collect();
        assert_eq!(step_ids, [judge.id, fan_one.id, retry.id, next.id]);
    }

    #[test]
    fn waiting_permissions_sort_by_creation_then_id() {
        let t = demo_at(1, 0);
        let row = |id: u128, created_at: DateTime<Utc>| WaitingPermission {
            item: ItemId::new(),
            project: ProjectId::new(),
            item_key: "FEAT-1".to_owned(),
            key_prefix: "FEAT".to_owned(),
            key_number: 1,
            run_queued_at: t,
            step_position: 0,
            step_attempt: 1,
            step_fanout_index: 0,
            phase_name: "implement".to_owned(),
            permission: StepPermission {
                id: PermissionId::from_uuid(Uuid::from_u128(id)),
                run_id: RunId::new(),
                run_step_id: StepId::new(),
                session: RelaySessionId::new(),
                request_id: format!("req-{id}"),
                tool_call_id: None,
                summary: None,
                options: vec![],
                status: PermissionStatus::Pending,
                option_id: None,
                answered_by: None,
                answered_box: None,
                created_at,
                answered_at: None,
                resolved_at: None,
            },
        };
        let mut rows = vec![row(1, t + Duration::minutes(1)), row(3, t), row(2, t)];

        WaitingPermission::sort_canonical(&mut rows);

        let ids: Vec<u128> = rows
            .iter()
            .map(|row| row.permission.id.as_uuid().as_u128())
            .collect();
        assert_eq!(ids, [2, 3, 1]);
    }
}
