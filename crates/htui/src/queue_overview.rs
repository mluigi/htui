//! The queue overlay's read (MOD-12 M3 D7): one `QueueOverview` composed from the runner's own
//! reads (`ready_items`, `batch_cancelled_items`, `batch_spend`, `batch_budget`, `admission_limit`),
//! so the overlay says what `admit` would do. Classification is `htui_core::model::classify_entry`.

use std::collections::{HashMap, HashSet};

use htui_core::model::{
    BatchFigures, Hold, ItemId, LiveFacts, ProjectCaps, ProjectId, QueueOverview, Scope, Status,
    WorkspaceId, admission_limit, batch_budget, classify_entry, min_budget_micros,
};
use htui_core::store::{Result, StoreError};
use htui_store::Backend;

/// [`StoreRequest::QueueOverview`](crate::store_worker::StoreRequest::QueueOverview)'s answer.
///
/// The reads follow `admit` (`htui-worker`'s runtime): the ready set is `ready_items` over the
/// rows' projects less the batch's cancelled items, and a ready row's project is held when its
/// caps read absent, do not parse, or `batch_budget` refuses the batch's spend. `missing_tags` is
/// read once per `open` row that is not ready (D6's accepted N+1); a `NotFound` there (a delete
/// racing this read) reads as no tags.
///
/// # Errors
/// `NotFound` "this box" before registration; offline, `Unreachable(DATABASE_UNREACHABLE)` from
/// `queue_rows`, the first queue read; otherwise whatever a read reports.
pub async fn overview(backend: &Backend) -> Result<QueueOverview> {
    let info = backend
        .box_info()
        .await?
        .ok_or_else(|| StoreError::NotFound {
            entity: "box",
            id: "this box".to_owned(),
        })?;
    let box_id = info.box_id;
    let rows = backend.queue_rows(box_id).await?;
    let batch = backend.open_batch_of(box_id).await?;
    let cancelled: HashSet<ItemId> = match &batch {
        Some(open) => backend
            .batch_cancelled_items(open.id)
            .await?
            .into_iter()
            .collect(),
        None => HashSet::new(),
    };

    // The runner's placeholder scope: the entries' projects, first-seen order.
    let mut project_ids: Vec<ProjectId> = Vec::new();
    for row in &rows {
        if !project_ids.contains(&row.entry.project_id) {
            project_ids.push(row.entry.project_id);
        }
    }
    let scope = Scope {
        workspace_id: WorkspaceId::default(),
        project_ids,
    };
    let ready: HashSet<ItemId> = backend
        .ready_items(&scope, box_id)
        .await?
        .into_iter()
        .map(|item| item.id)
        .filter(|item| !cancelled.contains(item))
        .collect();

    let mut missing_tags = HashMap::new();
    for row in &rows {
        let item = row.entry.item_id;
        if row.status != Status::Open || ready.contains(&item) || cancelled.contains(&item) {
            continue;
        }
        let tags = match backend.missing_tags(item, box_id).await {
            Ok(tags) => tags,
            Err(StoreError::NotFound { .. }) => Vec::new(),
            Err(err) => return Err(err),
        };
        if !tags.is_empty() {
            missing_tags.insert(item, tags);
        }
    }

    let app = backend.app_settings().await?;
    let slots_used = backend.running_runs_on_box(box_id).await?
        + backend.queued_runs_on_box(box_id).await?.len();
    let slots_limit = admission_limit(&info.settings, &app);

    let mut holds = HashMap::new();
    let figures = match &batch {
        Some(open) => {
            let spent = backend.batch_spend(open.id).await?;
            let min = min_budget_micros(&app);
            let mut seen: HashSet<ProjectId> = HashSet::new();
            for row in rows.iter().filter(|row| ready.contains(&row.entry.item_id)) {
                let project = row.entry.project_id;
                if !seen.insert(project) {
                    continue;
                }
                let hold = match backend.project_settings(project).await? {
                    None => Some(Hold::ProjectGone),
                    Some(settings) => match ProjectCaps::from_settings(&settings) {
                        Err(err) => Some(Hold::BadCap(err)),
                        Ok(caps) => batch_budget(spent, caps.batch_micros, min)
                            .err()
                            .map(Hold::Budget),
                    },
                };
                if let Some(hold) = hold {
                    holds.insert(project, hold);
                }
            }
            Some(BatchFigures {
                id: open.id,
                opened_at: open.opened_at,
                spent,
            })
        }
        None => None,
    };

    let live = LiveFacts {
        paused: batch.is_none(),
        ready,
        cancelled,
        missing_tags,
        holds,
    };
    let rows = rows
        .into_iter()
        .map(|row| {
            let state = classify_entry(&row, box_id, &live);
            (row, state)
        })
        .collect();
    Ok(QueueOverview {
        box_id,
        batch: figures,
        slots_used,
        slots_limit,
        rows,
        demo: matches!(backend, Backend::Memory(_)),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::Utc;
    use htui_core::fixtures::{DemoData, demo_at, demo_data, ids};
    use htui_core::model::{
        BatchStop, BoxId, EntryState, Escalation, GraphSnapshot, Hold, ItemId, ItemLink, LinkKind,
        NewRun, NewRunStep, Note, NoteId, PER_TOKEN_CAP_BATCH, ProjectId, QueueMove, QueueOverview,
        RunId, RunMode, RunStatus, Scope, Status, StepId, StepStatus, Wait, WorkspaceId,
        admission_order,
    };
    use htui_core::store::{MemStore, StepFence, WriteStore as _};
    use htui_store::{Backend, CacheStore, DATABASE_UNREACHABLE};
    use serde_json::json;

    use crate::store_worker::{QUEUE_REQUEST_NAMES, QueueWrite, StoreReply, StoreRequest, serve};

    /// `LAPTOP-B`, the fixture's second box.
    const LAPTOP: &str = "LAPTOP-B";

    /// A fresh item cloned from `template` (its project and kind), keyed `ANA-{n}`, created at
    /// `demo_at(0, n)` so the D2 order is the fixture's to choose.
    fn push_item(
        data: &mut DemoData,
        template: ItemId,
        n: i32,
        status: Status,
        tags: &[&str],
    ) -> ItemId {
        let mut item = data
            .items
            .iter()
            .find(|row| row.id == template)
            .expect("the template item")
            .clone();
        item.id = ItemId::new();
        item.key_prefix = "QX".to_owned();
        item.key_number = n;
        item.key = format!("QX-{n}");
        item.title = format!("queue state {n}");
        item.status = status;
        item.priority = 0;
        item.required_tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        item.created_at = demo_at(0, i64::from(n));
        item.updated_at = item.created_at;
        item.closed_at = None;
        item.resolution = None;
        let id = item.id;
        data.items.push(item);
        id
    }

    /// A graph run of `item` cloned from `RUN_1`, with `status` on `target` (and executing there
    /// unless it is still `queued`).
    fn push_run(
        data: &mut DemoData,
        item: ItemId,
        status: RunStatus,
        target: BoxId,
        failure: Option<&str>,
    ) -> RunId {
        let mut run = data
            .runs
            .iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("RUN_1")
            .clone();
        let project = data
            .items
            .iter()
            .find(|row| row.id == item)
            .expect("the run's item")
            .project_id;
        run.id = RunId::new();
        run.project_id = project;
        run.item_id = Some(item);
        run.mode = RunMode::Auto;
        run.status = status;
        run.target_box_id = target;
        run.executing_box_id = (status != RunStatus::Queued).then_some(target);
        run.queued_at = demo_at(3, 0);
        run.started_at = (status != RunStatus::Queued).then(|| demo_at(3, 0));
        run.finished_at = status.is_terminal().then(|| demo_at(3, 1));
        run.failure = failure.map(str::to_owned);
        run.updated_at = demo_at(3, 1);
        let id = run.id;
        data.runs.push(run);
        id
    }

    /// A step of `run` parked `awaiting_approval` (a hard gate).
    fn push_parked_step(data: &mut DemoData, run: RunId) -> StepId {
        let mut step = data.steps.first().expect("a fixture step").clone();
        step.id = StepId::new();
        step.run_id = run;
        step.position = 0;
        step.attempt = 1;
        step.fanout_index = 0;
        step.status = StepStatus::AwaitingApproval;
        step.finished_at = None;
        let id = step.id;
        data.steps.push(step);
        id
    }

    /// A note on `item`, later than every fixture note.
    fn push_note(data: &mut DemoData, item: ItemId, body: &str) {
        data.notes.push(Note {
            id: NoteId::new(),
            item_id: item,
            body: body.to_owned(),
            created_by: ids::USER,
            box_id: Some(ids::BOX),
            via_step_id: None,
            created_at: demo_at(4, 0),
        });
    }

    /// `project`'s `settings[PER_TOKEN_CAP_BATCH]`.
    fn set_batch_cap(data: &mut DemoData, project: ProjectId, cap: serde_json::Value) {
        data.projects
            .iter_mut()
            .find(|row| row.id == project)
            .expect("the project")
            .settings[PER_TOKEN_CAP_BATCH] = cap;
    }

    /// Queues `items` on the demo box.
    async fn queue(store: &MemStore, items: &[ItemId]) {
        for item in items {
            store
                .queue_item(*item, ids::BOX, ids::USER, Utc::now())
                .await
                .expect("the item queues");
        }
    }

    /// Opens a batch on the demo box.
    async fn resume(store: &MemStore) -> htui_core::model::BatchId {
        store
            .open_batch(ids::BOX, ids::USER, Utc::now())
            .await
            .expect("the batch opens")
            .id
    }

    /// The fixture's snapshot, for a run created through the store.
    fn snapshot() -> GraphSnapshot {
        let value = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .and_then(|row| row.graph_snapshot)
            .expect("RUN_1 carries a snapshot");
        serde_json::from_value(value).expect("the fixture snapshot decodes")
    }

    /// An auto run of `item` admitted under `batch`, `queued` on the demo box.
    async fn batch_run(
        store: &MemStore,
        item: ItemId,
        project: ProjectId,
        batch: htui_core::model::BatchId,
    ) -> RunId {
        store
            .create_run(NewRun {
                id: RunId::new(),
                project_id: project,
                item_id: item,
                mode: RunMode::Auto,
                target_box_id: ids::BOX,
                started_by: ids::USER,
                graph_snapshot: snapshot(),
                repo_scope: Vec::new(),
                queued_at: Utc::now(),
                batch_id: Some(batch),
            })
            .await
            .expect("the batch run lands")
            .id
    }

    /// One step of `run` reporting `cost` USD micros.
    async fn cost(store: &MemStore, run: RunId, cost: i64) {
        let step = store
            .create_step(NewRunStep {
                id: StepId::new(),
                run_id: run,
                position: 0,
                attempt: 1,
                fanout_index: 0,
                phase_name: "research".to_owned(),
                agent_id: Some(ids::AGENT_CLAUDE),
                model: None,
            })
            .await
            .expect("the step lands")
            .id;
        store
            .set_step_usage(
                StepFence::Unleased,
                step,
                json!({ "cost_micros": cost }),
                None,
            )
            .await
            .expect("the usage lands");
    }

    /// The overview served for `backend`, through `serve`.
    async fn served(backend: &Backend) -> QueueOverview {
        match serve(backend, &StoreRequest::QueueOverview).await {
            StoreReply::QueueOverview(overview) => *overview,
            other => panic!("`QueueOverview` answers an overview: {other:?}"),
        }
    }

    /// Each row's item and state.
    fn states(overview: &QueueOverview) -> Vec<(ItemId, EntryState)> {
        overview
            .rows
            .iter()
            .map(|(row, state)| (row.entry.item_id, state.clone()))
            .collect()
    }

    /// The twelve-state fixture: what [`twelve_states`] queued, in D2 order, with the state each
    /// entry must read.
    struct TwelveStates {
        store: MemStore,
        expected: Vec<(ItemId, EntryState)>,
    }

    /// One queued entry per state, a running batch, `LAPTOP-B` beside the demo box, and the demo
    /// box's limit at 5. The demo `queued` `RUN_2` (FEAT-3) is the admitted one.
    async fn twelve_states() -> TwelveStates {
        let mut data = demo_data();
        let mut laptop = data
            .boxes
            .iter()
            .find(|row| row.id == ids::BOX)
            .expect("the demo box")
            .clone();
        laptop.id = BoxId::new();
        laptop.hostname = LAPTOP.to_owned();
        let laptop_id = laptop.id;
        data.boxes.push(laptop);
        data.boxes
            .iter_mut()
            .find(|row| row.id == ids::BOX)
            .expect("the demo box")
            .settings = json!({ "max_concurrent_items": 5 });

        // Fixture items: FEAT-1 (priority 2, in progress, its run done), FEAT-3 (priority 1,
        // RUN_2 queued here), ANA-2 (open, ready), FEAT-2 (blocked, no run), TOOL-1 (awaiting
        // approval), CLEAN-1 (failed).
        let tool_run = push_run(
            &mut data,
            ids::HTUI_TOOL_1,
            RunStatus::AwaitingApproval,
            ids::BOX,
            None,
        );
        push_note(
            &mut data,
            ids::HTUI_TOOL_1,
            "pick a fan-out branch\nthe rest",
        );
        let clean_run = push_run(
            &mut data,
            ids::HTUI_CLEAN_1,
            RunStatus::Failed,
            ids::BOX,
            Some("exit 1"),
        );
        push_note(&mut data, ids::HTUI_FEAT_2, "blocked by hand");

        let running = push_item(&mut data, ids::HTUI_ANA_2, 20, Status::InProgress, &[]);
        let running_run = push_run(&mut data, running, RunStatus::Running, ids::BOX, None);
        let elsewhere = push_item(&mut data, ids::HTUI_ANA_2, 21, Status::InProgress, &[]);
        let elsewhere_run = push_run(&mut data, elsewhere, RunStatus::Running, laptop_id, None);
        let waiting = push_item(&mut data, ids::HTUI_ANA_2, 22, Status::Open, &[]);
        data.links.push(ItemLink {
            from_item_id: waiting,
            to_item_id: ids::HTUI_ANA_2,
            kind: LinkKind::BlockedBy,
            proposed_by_step_id: None,
            created_at: demo_at(0, 22),
            updated_at: demo_at(0, 22),
            deleted_at: None,
        });
        let untagged = push_item(&mut data, ids::HTUI_ANA_2, 23, Status::Open, &["docker"]);
        let exhausted = push_item(&mut data, ids::HTUI_ANA_2, 24, Status::Blocked, &[]);
        let exhausted_run = push_run(
            &mut data,
            exhausted,
            RunStatus::AwaitingApproval,
            ids::BOX,
            None,
        );
        push_note(&mut data, exhausted, "review loop gave up after 3 rounds");
        let gated = push_item(
            &mut data,
            ids::HTUI_ANA_2,
            25,
            Status::AwaitingApproval,
            &[],
        );
        let gated_run = push_run(
            &mut data,
            gated,
            RunStatus::AwaitingApproval,
            ids::BOX,
            None,
        );
        let gated_step = push_parked_step(&mut data, gated_run);

        let store = MemStore::from_demo(data);
        resume(&store).await;
        // Queued out of D2 order on purpose: the overview reads the queue's order, not this one.
        queue(
            &store,
            &[
                gated,
                exhausted,
                untagged,
                waiting,
                elsewhere,
                running,
                ids::HTUI_CLEAN_1,
                ids::HTUI_TOOL_1,
                ids::HTUI_FEAT_2,
                ids::HTUI_ANA_2,
                ids::HTUI_FEAT_3,
                ids::HTUI_FEAT_1,
            ],
        )
        .await;

        let expected = vec![
            (
                ids::HTUI_FEAT_1,
                EntryState::Waiting(Wait::NotReady(Status::InProgress)),
            ),
            (
                ids::HTUI_FEAT_3,
                EntryState::Running {
                    run: ids::RUN_2,
                    status: RunStatus::Queued,
                },
            ),
            (ids::HTUI_ANA_2, EntryState::Next),
            (
                ids::HTUI_FEAT_2,
                EntryState::Escalated(Escalation::Blocked {
                    run: None,
                    note: Some("blocked by hand".to_owned()),
                }),
            ),
            (
                ids::HTUI_TOOL_1,
                EntryState::Escalated(Escalation::JudgeUndecided {
                    run: tool_run,
                    note: Some("pick a fan-out branch\nthe rest".to_owned()),
                }),
            ),
            (
                ids::HTUI_CLEAN_1,
                EntryState::Escalated(Escalation::Failed {
                    run: Some(clean_run),
                    failure: Some("exit 1".to_owned()),
                }),
            ),
            (
                running,
                EntryState::Running {
                    run: running_run,
                    status: RunStatus::Running,
                },
            ),
            (
                elsewhere,
                EntryState::Elsewhere {
                    run: elsewhere_run,
                    hostname: Some(LAPTOP.to_owned()),
                    status: RunStatus::Running,
                },
            ),
            (
                waiting,
                EntryState::Waiting(Wait::BlockedBy(vec!["ANA-2".to_owned()])),
            ),
            (
                untagged,
                EntryState::Escalated(Escalation::MissingTags(vec!["docker".to_owned()])),
            ),
            (
                exhausted,
                EntryState::Escalated(Escalation::ReviewLoopExhausted {
                    run: exhausted_run,
                    note: Some("review loop gave up after 3 rounds".to_owned()),
                }),
            ),
            (
                gated,
                EntryState::Escalated(Escalation::HardGateParked {
                    run: gated_run,
                    step: gated_step,
                }),
            ),
        ];
        TwelveStates { store, expected }
    }

    /// MOD-12 M3 D5, D7: one entry per state answers its state, the rows in the queue's (D2)
    /// order: priority, then `created_at`, whatever order they were queued in.
    #[tokio::test]
    async fn the_overview_lists_the_queue_in_order_with_every_state() {
        let fixture = twelve_states().await;
        let backend = Backend::memory(fixture.store.clone());

        let overview = served(&backend).await;

        assert_eq!(overview.box_id, ids::BOX);
        let order: Vec<ItemId> = fixture
            .store
            .queue_entries(ids::BOX)
            .await
            .expect("read")
            .into_iter()
            .map(|entry| entry.item_id)
            .collect();
        assert_eq!(
            overview
                .rows
                .iter()
                .map(|(row, _)| row.entry.item_id)
                .collect::<Vec<_>>(),
            order,
            "the queue's order (D2)"
        );
        let actual = states(&overview);
        assert_eq!(actual.len(), fixture.expected.len());
        for (got, want) in actual.iter().zip(&fixture.expected) {
            assert_eq!(got, want);
        }
    }

    /// MOD-12 M3 D7 (M2 D4): a batch whose spend reached a project's cap holds that project's
    /// ready rows, as `admit` refuses them; another project's ready row is next.
    #[tokio::test]
    async fn a_batch_over_its_cap_marks_ready_rows_held() {
        let mut data = demo_data();
        set_batch_cap(&mut data, ids::PROJECT_HTUI, json!(500));
        let store = MemStore::from_demo(data);
        let batch = resume(&store).await;
        let run = batch_run(&store, ids::AGY_FEAT_1, ids::PROJECT_AGY, batch).await;
        cost(&store, run, 600).await;
        queue(&store, &[ids::HTUI_ANA_2, ids::AGY_FIX_1]).await;

        let overview = served(&Backend::memory(store)).await;

        assert_eq!(
            states(&overview),
            [
                (
                    ids::HTUI_ANA_2,
                    EntryState::Held(Hold::Budget(BatchStop::CapReached {
                        spent: 600,
                        cap: 500
                    }))
                ),
                (ids::AGY_FIX_1, EntryState::Next),
            ]
        );
        assert_eq!(
            overview.batch.map(|figures| (figures.id, figures.spent)),
            Some((batch, Some(600)))
        );
    }

    /// MOD-12 M3 D10: a cap that does not parse holds the project's ready rows (fails closed, as
    /// the runner does).
    #[tokio::test]
    async fn a_malformed_cap_reads_held() {
        let mut data = demo_data();
        set_batch_cap(&mut data, ids::PROJECT_HTUI, json!("lots"));
        let store = MemStore::from_demo(data);
        resume(&store).await;
        queue(&store, &[ids::HTUI_ANA_2]).await;

        let overview = served(&Backend::memory(store)).await;

        assert_eq!(overview.rows.len(), 1);
        assert!(
            matches!(overview.rows[0].1, EntryState::Held(Hold::BadCap(_))),
            "{:?}",
            overview.rows[0].1
        );
    }

    /// MOD-12 M3 D5: with no batch open the ready rows wait on the pause, and the header has no
    /// batch.
    #[tokio::test]
    async fn a_paused_queue_marks_ready_rows_paused_and_has_no_batch() {
        let store = MemStore::demo();
        queue(&store, &[ids::HTUI_ANA_2, ids::AGY_FIX_1]).await;

        let overview = served(&Backend::memory(store)).await;

        assert_eq!(overview.batch, None);
        assert_eq!(
            states(&overview),
            [
                (ids::HTUI_ANA_2, EntryState::Waiting(Wait::Paused)),
                (ids::AGY_FIX_1, EntryState::Waiting(Wait::Paused)),
            ]
        );
    }

    /// MOD-12 M3 D7: the `Next` rows are exactly `admission_order` over the ready, uncancelled
    /// entries, less the held project: what `admit` would admit.
    #[tokio::test]
    async fn next_is_exactly_what_admission_would_admit() {
        let mut data = demo_data();
        set_batch_cap(&mut data, ids::PROJECT_HTUI, json!(500));
        let agy_open = push_item(&mut data, ids::AGY_FIX_1, 30, Status::Open, &[]);
        let agy_later = push_item(&mut data, ids::AGY_FIX_1, 31, Status::Open, &[]);
        let store = MemStore::from_demo(data);
        let batch = resume(&store).await;
        let spender = batch_run(&store, ids::AGY_FEAT_1, ids::PROJECT_AGY, batch).await;
        cost(&store, spender, 600).await;
        let cancelled = batch_run(&store, ids::AGY_FIX_1, ids::PROJECT_AGY, batch).await;
        store
            .finish_run(cancelled, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the run cancels");
        queue(
            &store,
            &[
                ids::HTUI_ANA_2,
                ids::AGY_FIX_1,
                agy_later,
                ids::HTUI_FEAT_1,
                agy_open,
                ids::AGY_FEAT_1,
            ],
        )
        .await;
        let backend = Backend::memory(store.clone());

        let overview = served(&backend).await;

        let entries = store.queue_entries(ids::BOX).await.expect("read");
        let scope = Scope {
            workspace_id: WorkspaceId::default(),
            project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
        };
        let cancelled_items: HashSet<ItemId> = store
            .batch_cancelled_items(batch)
            .await
            .expect("read")
            .into_iter()
            .collect();
        let ready: Vec<_> = store
            .ready_items(&scope, ids::BOX)
            .await
            .expect("read")
            .into_iter()
            .filter(|item| !cancelled_items.contains(&item.id))
            .collect();
        let admitted: Vec<ItemId> = admission_order(&entries, &ready)
            .into_iter()
            .filter(|item| {
                entries
                    .iter()
                    .any(|entry| entry.item_id == *item && entry.project_id != ids::PROJECT_HTUI)
            })
            .collect();
        let next: Vec<ItemId> = overview
            .rows
            .iter()
            .filter(|(_, state)| *state == EntryState::Next)
            .map(|(row, _)| row.entry.item_id)
            .collect();
        assert_eq!(next, admitted);
        assert_eq!(next, [agy_open, agy_later], "the fixture is not vacuous");
        let state_of = |item: ItemId| {
            overview
                .rows
                .iter()
                .find(|(row, _)| row.entry.item_id == item)
                .map(|(_, state)| state.clone())
                .expect("the item is listed")
        };
        assert_eq!(
            state_of(ids::AGY_FIX_1),
            EntryState::Waiting(Wait::CancelledInBatch)
        );
        assert!(matches!(
            state_of(ids::HTUI_ANA_2),
            EntryState::Held(Hold::Budget(_))
        ));
        assert_eq!(
            state_of(ids::HTUI_FEAT_1),
            EntryState::Waiting(Wait::NotReady(Status::InProgress))
        );
    }

    /// MOD-12 M3 D7 (B.4.2 step 5): a row cancelled in this batch still has its tags read, so
    /// one that now lacks a tag escalates (rule 8) rather than waiting on the next batch (rule
    /// 10), which would not admit it either.
    #[tokio::test]
    async fn a_cancelled_row_missing_tags_reads_escalated() {
        let mut data = demo_data();
        let untagged = push_item(&mut data, ids::AGY_FIX_1, 32, Status::Open, &["docker"]);
        let store = MemStore::from_demo(data);
        let batch = resume(&store).await;
        let run = batch_run(&store, untagged, ids::PROJECT_AGY, batch).await;
        store
            .finish_run(run, RunStatus::Cancelled, None, Utc::now())
            .await
            .expect("the run cancels");
        queue(&store, &[untagged]).await;
        assert_eq!(
            store.batch_cancelled_items(batch).await.expect("read"),
            [untagged],
            "the fixture is not vacuous"
        );

        let overview = served(&Backend::memory(store)).await;

        assert_eq!(overview.rows.len(), 1);
        assert_eq!(overview.rows[0].0.status, Status::Open);
        assert_eq!(
            overview.rows[0].1,
            EntryState::Escalated(Escalation::MissingTags(vec!["docker".to_owned()]))
        );
    }

    /// MOD-12 M3 D7: `slots_used` is the box's running runs plus its queued ones (`free_slots`'
    /// inputs), `slots_limit` the box's `max_concurrent_items`, and the batch its own spend.
    #[tokio::test]
    async fn the_header_figures_count_slots_and_spend() {
        let fixture = twelve_states().await;
        let backend = Backend::memory(fixture.store.clone());

        let overview = served(&backend).await;

        let running = fixture
            .store
            .running_runs_on_box(ids::BOX)
            .await
            .expect("read");
        let queued = fixture
            .store
            .queued_runs_on_box(ids::BOX)
            .await
            .expect("read")
            .len();
        assert_eq!((running, queued), (1, 1), "the fixture is not vacuous");
        assert_eq!(overview.slots_used, running + queued);
        assert_eq!(overview.slots_limit, 5);
        let batch = fixture
            .store
            .open_batch_of(ids::BOX)
            .await
            .expect("read")
            .expect("the batch is open");
        let figures = overview.batch.expect("a running queue has a batch");
        assert_eq!(
            (figures.id, figures.opened_at, figures.spent),
            (batch.id, batch.opened_at, None),
            "no step reported a cost"
        );
    }

    /// MOD-12 M3 D7 (M1 review L3): the memory backend, `htui --demo`, says so.
    #[tokio::test]
    async fn memory_answers_demo() {
        let overview = served(&Backend::memory(MemStore::demo())).await;

        assert!(overview.demo);
        assert!(overview.rows.is_empty());
        assert_eq!(overview.slots_limit, 2, "the demo box's limit");
    }

    /// MOD-12 M3 D3, D7: a move answers `QueueWritten { Moved }` with the queue in its new order.
    #[tokio::test]
    async fn move_queue_entry_answers_moved_with_the_new_order() {
        let store = MemStore::demo();
        queue(&store, &[ids::HTUI_ANA_2, ids::AGY_FIX_1]).await;
        let backend = Backend::memory(store);

        let reply = serve(
            &backend,
            &StoreRequest::MoveQueueEntry {
                item: ids::AGY_FIX_1,
                to: QueueMove::Up,
            },
        )
        .await;

        match reply {
            StoreReply::QueueWritten { write, view } => {
                assert_eq!(write, QueueWrite::Moved { moved: true });
                assert_eq!(view.entries, [ids::AGY_FIX_1, ids::HTUI_ANA_2]);
            }
            other => panic!("a move answers `QueueWritten`: {other:?}"),
        }
        assert_eq!(
            served(&backend)
                .await
                .rows
                .iter()
                .map(|(row, _)| row.entry.item_id)
                .collect::<Vec<_>>(),
            [ids::AGY_FIX_1, ids::HTUI_ANA_2],
            "the overview reads the moved order"
        );
    }

    /// MOD-12 M3 D3: the head moved up writes nothing and says so.
    #[tokio::test]
    async fn moving_the_head_up_answers_moved_false() {
        let store = MemStore::demo();
        queue(&store, &[ids::HTUI_ANA_2, ids::AGY_FIX_1]).await;
        let backend = Backend::memory(store);

        let reply = serve(
            &backend,
            &StoreRequest::MoveQueueEntry {
                item: ids::HTUI_ANA_2,
                to: QueueMove::Up,
            },
        )
        .await;

        match reply {
            StoreReply::QueueWritten { write, view } => {
                assert_eq!(write, QueueWrite::Moved { moved: false });
                assert_eq!(view.entries, [ids::HTUI_ANA_2, ids::AGY_FIX_1]);
            }
            other => panic!("a move answers `QueueWritten`: {other:?}"),
        }
    }

    /// MOD-12 M3 D7: offline, over a mirror that holds this box, both requests fail by their own
    /// names, which the queue's `Failed` matches list, with `DATABASE_UNREACHABLE`.
    #[tokio::test]
    async fn offline_both_requests_fail_by_their_names() {
        let root = tempfile::tempdir().expect("temp root");
        let cache = CacheStore::open(root.path(), "queue-overview-offline", 1)
            .await
            .expect("open a throwaway mirror");
        sqlx::query(
            "INSERT INTO box (id, user_id, hostname, os_family, os_version, arch, htui_version, \
                              registered_at, last_seen_at, updated_at) \
             VALUES (?, ?, 'offline-box', 'linux', '', 'x86_64', '0.0.0', 0, 0, 0)",
        )
        .bind(ids::BOX.to_string())
        .bind(ids::USER.to_string())
        .execute(cache.pool())
        .await
        .expect("plant this box in the mirror");
        let backend = Backend::Offline {
            cache: cache.clone(),
            since: None,
        };
        assert!(
            backend.box_info().await.expect("read").is_some(),
            "the box is known, so the queue read is what refuses"
        );

        for (request, name) in [
            (StoreRequest::QueueOverview, "queue_overview"),
            (
                StoreRequest::MoveQueueEntry {
                    item: ids::HTUI_ANA_2,
                    to: QueueMove::Down,
                },
                "move_queue_entry",
            ),
        ] {
            match serve(&backend, &request).await {
                StoreReply::Failed {
                    request: failed,
                    message,
                } => {
                    assert_eq!(failed, name);
                    assert!(QUEUE_REQUEST_NAMES.contains(&failed), "{failed}");
                    assert!(message.contains(DATABASE_UNREACHABLE), "{message}");
                }
                other => panic!("offline `{name}` fails: {other:?}"),
            }
        }
        cache.close().await;
    }
}
