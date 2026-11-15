//! MOD-133 M1 (D3, D9): landing through the walk, over `MemStore` and the `FakeIsolator`.
//!
//! A repo whose mode resolves to `push` is frozen into the run's snapshot at `StartRun`; the
//! run's `finish_run(Done)` writes its integration record, and the walk lands it before it rests:
//! the record settles, and the item carries a note. A `local` run writes and lands nothing.
//! Refusals at run start are `ResolveError::IntegrateRefused`, before any run row exists.

use htui_core::fixtures::ids;
use htui_core::model::{
    Gate, IntegrateMode, IntegrationState, Isolation, ItemId, ItemPatch, LandOutcome, NewRepo,
    NewStepGraph, PhaseId, RepoId, RepoPatch, RunId, RunMode, RunStatus, Split, StepGraphId,
    StepGraphPhase,
};
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, WriteStore as _};
use htui_orch::command::{Command, CommandOutcome, EngineError, GateAnswer};
use htui_orch::fake::{FakeOrchestrator, IsolatorCall, ScriptedStep};
use htui_orch::graph::ResolveError;
use htui_orch::isolate::{Clock as _, IsolateError};

/// The remote every `push` repo of this file names, with a credential in it.
const REMOTE: &str = "https://bot:secret@forge.example/o/htui.git";

/// A second remote, for a case that needs two.
const OTHER_REMOTE: &str = "https://forge.example/o/other.git";

/// The demo fixture with `FEAT-3` free to start, every phase of its graph ungated (`gate` on
/// `gated` phases only) and `review` approving.
async fn harness(gated: &[&str]) -> FakeOrchestrator {
    let orch = FakeOrchestrator::demo();
    orch.store
        .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock.now())
        .await
        .expect("the seeded run is queued and cancellable");
    repoint(&orch.store, ids::HTUI_FEAT_3, |phase| {
        phase.gate = if gated.contains(&phase.name.as_str()) {
            Gate::Always
        } else {
            Gate::Never
        };
    })
    .await;
    orch.script("review", 1, ScriptedStep::review("approve", "fine"));
    orch
}

/// The HTUI project's primary repo `htui`, with `remote` and `integrate`.
async fn repo(
    orch: &FakeOrchestrator,
    remote: Option<&str>,
    integrate: Option<IntegrateMode>,
) -> RepoId {
    let id = RepoId::new();
    orch.store
        .create_repo(NewRepo {
            id,
            project_id: ids::PROJECT_HTUI,
            name: "htui".to_owned(),
            remote_url: remote.map(str::to_owned),
            default_branch: "main".to_owned(),
            is_primary: true,
        })
        .await
        .expect("the demo project has no repo yet");
    set_mode(orch, id, integrate).await;
    id
}

/// `repo.integrate` written through the store's own writer.
async fn set_mode(orch: &FakeOrchestrator, repo: RepoId, integrate: Option<IntegrateMode>) {
    let expected = orch
        .store
        .repos(ids::PROJECT_HTUI)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|row| row.id == repo)
        .expect("the repo is there")
        .updated_at;
    let outcome = orch
        .store
        .update_repo(
            repo,
            expected,
            RepoPatch {
                integrate: Some(integrate),
                ..RepoPatch::default()
            },
        )
        .await
        .expect("the repo is there");
    assert!(
        matches!(outcome, CasOutcome::Applied(_)),
        "the repo moved under the case: {outcome:?}"
    );
}

/// Repoints `item` at a clone of its graph whose phases `mutate` has edited (the engine tests'
/// own helper, over the store).
async fn repoint(store: &MemStore, item: ItemId, mutate: impl Fn(&mut StepGraphPhase)) {
    let row = store
        .item(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the fixture holds the item");
    let graph = store
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph");
    let id = StepGraphId::new();
    let clone = store
        .create_step_graph(NewStepGraph {
            id,
            scope: htui_core::model::GraphScope::Project(row.project_id),
            name: format!("{}-landing-{id}", row.key),
            description: "a landing case's edit of the live graph".to_owned(),
            is_override: false,
        })
        .await
        .expect("the name is fresh");
    for phase in &graph.phases {
        let mut edited = StepGraphPhase {
            id: PhaseId::new(),
            graph_id: clone.id,
            ..phase.phase.clone()
        };
        mutate(&mut edited);
        if edited.split != Split::Divide {
            edited.split = Split::for_fan_out(edited.fan_out);
        }
        store
            .create_phase(&edited)
            .await
            .expect("the clone accepts its phases");
    }
    store
        .update_item(
            item,
            row.version,
            ItemPatch {
                step_graph_id: Some(Some(clone.id)),
                author_id: row.created_by,
                reason: "a landing case's edit of the live graph".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// `StartRun` on `FEAT-3` over `repo` alone.
async fn start(orch: &FakeOrchestrator, repo: RepoId) -> Result<CommandOutcome, EngineError> {
    orch.dispatch(Command::StartRun {
        item: ids::HTUI_FEAT_3,
        mode: RunMode::Manual,
        repo_scope: Some(vec![repo]),
        workspace: None,
    })
    .await
}

/// [`start`], which must answer `Started`: the run and its rest.
async fn started(orch: &FakeOrchestrator, repo: RepoId) -> (RunId, RunStatus) {
    match start(orch, repo).await.expect("the run starts") {
        CommandOutcome::Started { run, rest } => (run, rest.run),
        other => panic!("`StartRun` answers `Started`, not {other:?}"),
    }
}

/// The item's note bodies that mention an integration.
async fn integration_notes(orch: &FakeOrchestrator) -> Vec<String> {
    orch.store
        .notes(ids::HTUI_FEAT_3)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .map(|note| note.body)
        .filter(|body| body.starts_with("integration "))
        .collect()
}

/// D3, D9: a `push` repo is frozen into the snapshot; the finished walk lands its record with the
/// run's landed commits and notes it; a second pass finds nothing to take.
#[tokio::test]
async fn a_push_runs_finish_lands_it_and_notes_it() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);

    let snapshot = orch.run(run).await.graph_snapshot.expect("a graph run");
    assert_eq!(
        snapshot["integrate"][repo.to_string()],
        serde_json::json!({ "mode": "push", "remote": REMOTE, "target_branch": "main" }),
    );

    let requests = orch.isolator.land_requests();
    assert_eq!(requests.len(), 1, "one record, landed once: {requests:?}");
    assert_eq!(
        (
            requests[0].repo,
            requests[0].remote.as_str(),
            requests[0].target_branch.as_str()
        ),
        (repo, REMOTE, "main")
    );
    assert!(
        !requests[0].commits.is_empty(),
        "the run's landed commits are handed over"
    );
    let tip = requests[0].commits.last().cloned().expect("not empty");

    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(records.len(), 1);
    assert_eq!(
        (records[0].state, records[0].landed_hash.as_deref()),
        (IntegrationState::Landed, Some(tip.as_str()))
    );
    assert_eq!(
        integration_notes(&orch).await,
        [format!(
            "integration landed: repo `htui` main at {}",
            &tip[..7]
        )]
    );

    assert_eq!(
        orch.land(None).await.expect("the pass runs"),
        0,
        "a landed record is not taken again"
    );
    assert_eq!(orch.isolator.land_requests().len(), 1);
}

/// D9: a refusal parks the record with its reason and notes it; no sweep retries a `refused`
/// record.
#[tokio::test]
async fn a_refusal_parks_the_record_and_notes_it() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;
    let reason = "non_fast_forward: main on the remote has moved; merge it locally, then retry";
    orch.isolator.script_land(LandOutcome::Refused {
        reason: reason.to_owned(),
    });

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done, "a refusal never fails the walk");

    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        (records[0].state, records[0].refusal.as_deref()),
        (IntegrationState::Refused, Some(reason))
    );
    assert_eq!(
        integration_notes(&orch).await,
        [format!(
            "integration refused for repo `htui` (push to main): {reason}; the run's work has \
             not landed"
        )]
    );
    assert_eq!(orch.land(None).await.expect("the pass runs"), 0);
    assert_eq!(orch.isolator.land_requests().len(), 1, "not retried");
}

/// D9: an isolator error leaves the record `pending`, unnoted, and the walk still rests `done`;
/// the next pass (here after the lease lapsed) lands it.
#[tokio::test]
async fn an_isolator_error_leaves_the_record_pending() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;
    orch.isolator.fail_land(IsolateError::Git(
        "git push timed out after 120s".to_owned(),
    ));

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(records[0].state, IntegrationState::Pending);
    assert!(integration_notes(&orch).await.is_empty());

    // The same owner may take its own lease again at once.
    assert_eq!(orch.land(Some(run)).await.expect("the pass runs"), 1);
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(records[0].state, IntegrationState::Landed);
}

/// F2 M1: an isolator error for a remote skips the pass's later records for that remote (they
/// stay `pending`, untried); a record for another remote is still tried. Two errors are scripted:
/// whichever record comes first, the pass asks the isolator twice, once per remote, and both
/// answers are errors. The next pass lands all three.
#[tokio::test]
async fn an_isolator_error_skips_the_passs_other_records_for_that_remote() {
    let orch = harness(&[]).await;
    let first = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;
    let mut scope = vec![first];
    for (name, remote) in [("twin", REMOTE), ("other", OTHER_REMOTE)] {
        let id = RepoId::new();
        orch.store
            .create_repo(NewRepo {
                id,
                project_id: ids::PROJECT_HTUI,
                name: name.to_owned(),
                remote_url: Some(remote.to_owned()),
                default_branch: "main".to_owned(),
                is_primary: false,
            })
            .await
            .expect("the repo is fresh");
        set_mode(&orch, id, Some(IntegrateMode::Push)).await;
        scope.push(id);
    }
    for _ in 0..2 {
        orch.isolator.fail_land(IsolateError::Git(
            "git ls-remote timed out after 30s".to_owned(),
        ));
    }

    let started = orch
        .dispatch(Command::StartRun {
            item: ids::HTUI_FEAT_3,
            mode: RunMode::Manual,
            repo_scope: Some(scope),
            workspace: None,
        })
        .await
        .expect("the run starts");
    let CommandOutcome::Started { run, rest } = started else {
        panic!("`StartRun` answers `Started`, not {started:?}");
    };
    assert_eq!(rest.run, RunStatus::Done);
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(records.len(), 3, "{records:?}");
    assert!(
        records
            .iter()
            .all(|record| record.state == IntegrationState::Pending),
        "{records:?}"
    );
    let requests = orch.isolator.land_requests();
    let mut asked: Vec<&str> = requests
        .iter()
        .map(|request| request.remote.as_str())
        .collect();
    asked.sort_unstable();
    assert_eq!(
        asked,
        [REMOTE, OTHER_REMOTE],
        "one try per remote: {requests:?}"
    );

    // The same owner takes its own leases again at once; nothing more is scripted.
    assert_eq!(orch.land(Some(run)).await.expect("the pass runs"), 3);
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert!(
        records
            .iter()
            .all(|record| record.state == IntegrationState::Landed),
        "{records:?}"
    );
}

/// D3: a `local` run (the repo's default, the project's default) writes no record and lands
/// nothing, and its snapshot carries no `integrate` key.
#[tokio::test]
async fn a_local_run_writes_and_lands_nothing() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), None).await;

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);
    let snapshot = orch.run(run).await.graph_snapshot.expect("a graph run");
    assert!(
        snapshot.get("integrate").is_none(),
        "a local run's snapshot JSON is unchanged: {snapshot}"
    );
    assert!(
        orch.store
            .integrations(run)
            .await
            .expect("MemStore never fails a read")
            .is_empty()
    );
    assert!(orch.isolator.land_requests().is_empty());
    assert!(
        !orch
            .isolator
            .call_log()
            .iter()
            .any(|call| matches!(call, IsolatorCall::Land(_)))
    );
    assert!(integration_notes(&orch).await.is_empty());
}

/// D3: the mode is the snapshot's. A repo switched to `local` after its run started still lands
/// that run; one switched to `push` mid-run does not.
#[tokio::test]
async fn a_mode_changed_mid_run_lands_per_the_snapshot() {
    let orch = harness(&["prd"]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;
    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::AwaitingApproval, "parked at `prd`");

    set_mode(&orch, repo, Some(IntegrateMode::Local)).await;
    let step = orch
        .steps(run)
        .await
        .into_iter()
        .find(|step| step.phase_name == "prd")
        .expect("prd ran");
    orch.dispatch(Command::AnswerGate {
        run,
        step: step.id,
        answer: GateAnswer::Approved,
    })
    .await
    .expect("the gate is answered");
    assert_eq!(orch.run(run).await.status, RunStatus::Done);
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        records
            .iter()
            .map(|record| (record.mode, record.state))
            .collect::<Vec<_>>(),
        [(IntegrateMode::Push, IntegrationState::Landed)],
        "frozen at StartRun"
    );
}

/// D3: the other direction: a `local` run whose repo turned `push` mid-run writes no record.
#[tokio::test]
async fn a_repo_turned_push_mid_run_does_not_land_that_run() {
    let orch = harness(&["prd"]).await;
    let repo = repo(&orch, Some(REMOTE), None).await;
    let (run, _) = started(&orch, repo).await;

    set_mode(&orch, repo, Some(IntegrateMode::Push)).await;
    let step = orch
        .steps(run)
        .await
        .into_iter()
        .find(|step| step.phase_name == "prd")
        .expect("prd ran");
    orch.dispatch(Command::AnswerGate {
        run,
        step: step.id,
        answer: GateAnswer::Approved,
    })
    .await
    .expect("the gate is answered");
    assert_eq!(orch.run(run).await.status, RunStatus::Done);
    assert!(
        orch.store
            .integrations(run)
            .await
            .expect("MemStore never fails a read")
            .is_empty()
    );
    assert!(orch.isolator.land_requests().is_empty());
}

/// D3: the run-start refusals, each before a run row exists. M3 D4: a `pull_request` repo
/// refuses a phase that runs in place (`local` here), naming it.
#[tokio::test]
async fn an_unlandable_mode_refuses_the_run_at_start() {
    for (remote, mode, settings, in_place, sentence) in [
        (
            None,
            Some(IntegrateMode::Push),
            None,
            None,
            "`htui` integrates by `push` but has no remote URL: set one, or set it to `local`",
        ),
        (
            Some(REMOTE),
            Some(IntegrateMode::PullRequest),
            None,
            Some("implement"),
            "`htui` integrates by `pull_request`, which needs `worktree` or `copy` isolation, \
             but phase `implement` runs `local`: set the phase's isolation, or set the repo to \
             `local` or `push`",
        ),
        (
            Some(REMOTE),
            None,
            Some(serde_json::json!({ "integrate": "bogus" })),
            None,
            "project setting `integrate` is \"bogus\", not one of local, push, pull_request",
        ),
        (
            None,
            None,
            Some(serde_json::json!({ "integrate": "push" })),
            None,
            "`htui` integrates by `push` but has no remote URL: set one, or set it to `local`",
        ),
    ] {
        let orch = harness(&[]).await;
        if let Some(settings) = settings {
            orch.store.set_project_settings(ids::PROJECT_HTUI, settings);
        }
        if let Some(name) = in_place {
            repoint(&orch.store, ids::HTUI_FEAT_3, |phase| {
                if phase.name == name {
                    phase.isolation = Some(Isolation::Local);
                }
            })
            .await;
        }
        let repo = repo(&orch, remote, mode).await;
        let before = orch
            .store
            .runs(ids::HTUI_FEAT_3)
            .await
            .expect("MemStore never fails a read")
            .len();

        let err = start(&orch, repo).await.expect_err("refused at start");
        assert!(
            matches!(
                &err,
                EngineError::Resolve(ResolveError::IntegrateRefused { repo, .. }) if repo == "htui"
            ),
            "{err:?}"
        );
        assert_eq!(err.to_string(), sentence);
        assert_eq!(
            orch.store
                .runs(ids::HTUI_FEAT_3)
                .await
                .expect("MemStore never fails a read")
                .len(),
            before,
            "no run row is written"
        );
    }
}

/// D3: the project default reaches a repo that sets none, and a `local` override wins over it.
#[tokio::test]
async fn the_project_default_applies_and_a_repo_override_wins() {
    let orch = harness(&[]).await;
    orch.store.set_project_settings(
        ids::PROJECT_HTUI,
        serde_json::json!({ "integrate": "push" }),
    );
    let first = repo(&orch, Some(REMOTE), None).await;
    let (run, _) = started(&orch, first).await;
    assert_eq!(
        orch.store
            .integrations(run)
            .await
            .expect("MemStore never fails a read")
            .len(),
        1,
        "the project's `push` applies"
    );

    let orch = harness(&[]).await;
    orch.store.set_project_settings(
        ids::PROJECT_HTUI,
        serde_json::json!({ "integrate": "push" }),
    );
    let second = repo(&orch, None, Some(IntegrateMode::Local)).await;
    let (run, rest) = started(&orch, second).await;
    assert_eq!(
        rest,
        RunStatus::Done,
        "the repo's `local` wins: no remote is needed"
    );
    assert!(
        orch.store
            .integrations(run)
            .await
            .expect("MemStore never fails a read")
            .is_empty()
    );
}
