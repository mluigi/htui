//! MOD-133 M1 (D3, D9): landing through the walk, over `MemStore` and the `FakeIsolator`.
//!
//! A repo whose mode resolves to `push` is frozen into the run's snapshot at `StartRun`; the
//! run's `finish_run(Done)` writes its integration record, and the walk lands it before it rests:
//! the record settles, and the item carries a note. A `local` run writes and lands nothing.
//! Refusals at run start are `ResolveError::IntegrateRefused`, before any run row exists.

use std::collections::BTreeMap;

use htui_core::fixtures::ids;
use htui_core::model::link::ProposeLink;
use htui_core::model::{
    Gate, GraphBody, GraphSnapshot, IntegrateMode, IntegrationState, Isolation, ItemFilter, ItemId,
    ItemPatch, LandOutcome, LinkKind, NewItem, NewRepo, NewRun, NewRunStep, NewStepGraph, PhaseId,
    PhasePart, RepoId, RepoPatch, RunId, RunMode, RunStatus, Scope, Split, StepGraphId,
    StepGraphPhase, StepId, StepRole,
};
use htui_core::store::{CasOutcome, MemStore, ReadStore as _, StepFence, WriteStore as _};
use htui_orch::command::{Command, CommandOutcome, EngineError, GateAnswer};
use htui_orch::fake::{FakeOrchestrator, IsolatorCall, ScriptedStep};
use htui_orch::graph::ResolveError;
use htui_orch::isolate::real::no_run_branch;
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

// -- MOD-133 M3 D2, D3, D5: a `pull_request` run's run branch through the walk ------------------

/// The run branch every `pull_request` case of `FEAT-3` names.
const RUN_BRANCH: &str = "htui/item/FEAT-3";

/// The isolator's call log, each step named by its phase and the run's own `cleanup`/`release`
/// by `run`.
async fn call_log_of(orch: &FakeOrchestrator, run: RunId) -> Vec<String> {
    let steps = orch.steps(run).await;
    let name = |step: StepId| {
        steps
            .iter()
            .find(|row| row.id == step)
            .map_or_else(|| "?".to_owned(), |row| row.phase_name.clone())
    };
    orch.isolator
        .call_log()
        .into_iter()
        .map(|call| match call {
            IsolatorCall::Prepare(step) => format!("prepare {}", name(step)),
            IsolatorCall::Capture(step) => format!("capture {}", name(step)),
            IsolatorCall::Reconcile(step) => format!("reconcile {}", name(step)),
            IsolatorCall::Reset(step) => format!("reset {}", name(step)),
            IsolatorCall::Premerge(step) => format!("premerge {}", name(step)),
            IsolatorCall::Unmerged(step) => format!("unmerged {}", name(step)),
            IsolatorCall::Reacquire(step) => format!("reacquire {}", name(step)),
            IsolatorCall::Land(_) => "land".to_owned(),
            IsolatorCall::RunBranches => "run_branches".to_owned(),
            IsolatorCall::Cleanup(id) => format!("cleanup {}", if id == run { "run" } else { "?" }),
            IsolatorCall::Release(id) => format!("release {}", if id == run { "run" } else { "?" }),
        })
        .collect()
}

/// The `prepare` and `run_branches` entries of [`call_log_of`], in order.
async fn bases_of(orch: &FakeOrchestrator, run: RunId) -> Vec<String> {
    call_log_of(orch, run)
        .await
        .into_iter()
        .filter(|call| call.starts_with("prepare ") || call == "run_branches")
        .collect()
}

/// The fake's tip of `repo`'s run branch ([`htui_orch::isolate::Isolator::run_branches`]),
/// apart from its `base` (`fake:group-base:<repo>`), so a step that starts from it started from
/// the run branch.
fn fake_tip(repo: RepoId) -> String {
    format!("fake:run-branch:{repo}")
}

/// D2, D3: a `pull_request` run ensures its run branch before every step that prepares the scope,
/// every such tree starts from the run branch's tip, and every landing goes onto the run branch.
#[tokio::test]
async fn a_pull_request_run_bases_steps_on_its_run_branch_and_reconciles_onto_it() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::PullRequest)).await;
    orch.isolator.script_land(LandOutcome::Pushed {
        hash: "abcdef0123456789abcdef0123456789abcdef01".to_owned(),
    });

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);

    let steps = orch.steps(run).await;
    let calls = bases_of(&orch, run).await;
    let mut expected = Vec::new();
    for step in &steps {
        expected.push("run_branches".to_owned());
        expected.push(format!("prepare {}", step.phase_name));
    }
    assert_eq!(calls, expected, "an ensure before every prepare");
    for step in &steps {
        let trees = orch
            .store
            .step_trees(step.id)
            .await
            .expect("MemStore never fails a read");
        assert_eq!(
            trees
                .iter()
                .map(|tree| (tree.repo_id, tree.base_ref.clone()))
                .collect::<Vec<_>>(),
            [(repo, fake_tip(repo))],
            "`{}` starts from the run branch",
            step.phase_name
        );
    }
    let onto = orch.isolator.reconcile_onto_log();
    assert_eq!(onto.len(), steps.len(), "every step lands: {onto:?}");
    for landing in onto {
        assert_eq!(
            landing.onto,
            Some((run, BTreeMap::from([(repo, RUN_BRANCH.to_owned())]))),
            "onto the run branch"
        );
    }
}

/// D2: a fan-out group's candidates start from the run branch (one ensure for the group, in
/// `group_base`), and a divided node's plan records the run branch as its `base_refs` (one ensure
/// at admission), which its parts and its integrate step start from without another.
#[tokio::test]
async fn fan_out_and_divided_parts_follow_the_run_branch() {
    let orch = harness(&[]).await;
    repoint(&orch.store, ids::HTUI_FEAT_3, |phase| {
        phase.input_kinds.clear();
        if phase.name == "prd" {
            phase.fan_out = 2;
        }
    })
    .await;
    edit_graph(&orch.store, ids::HTUI_FEAT_3, |body| {
        let node = body
            .nodes
            .iter_mut()
            .find(|node| node.phase.name == "implement")
            .expect("FEAT-3's graph has `implement`");
        node.phase.split = Split::Divide;
        node.phase.fan_out = 1;
        let phase = node.phase.id;
        node.parts = ["api", "ui"]
            .into_iter()
            .enumerate()
            .map(|(position, key)| PhasePart {
                phase_id: phase,
                position: i32::try_from(position).expect("two parts"),
                key: key.to_owned(),
                title: format!("The {key}"),
                brief: format!("Build the {key} part."),
                owned_paths: vec![format!("src/{key}/**")],
                acceptance: None,
                agent_id: None,
                model: None,
                persona_id: None,
            })
            .collect();
    })
    .await;
    orch.store.set_project_settings(
        ids::PROJECT_HTUI,
        serde_json::json!({ "judge_agent_id": ids::AGENT_AGY }),
    );
    for call in 0..2 {
        orch.script_candidate("prd:judge", 1, -1, call, ScriptedStep::judge(0, &[]));
    }
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::PullRequest)).await;
    orch.isolator.script_land(LandOutcome::Pushed {
        hash: "abcdef0123456789abcdef0123456789abcdef01".to_owned(),
    });

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);

    let plans = orch
        .store
        .run_plans(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        plans
            .iter()
            .map(|plan| plan.base_refs.clone())
            .collect::<Vec<_>>(),
        [Some(
            serde_json::json!({ repo.to_string(): fake_tip(repo) })
        )],
        "the plan's base is the run branch"
    );
    for step in orch.steps(run).await {
        for tree in orch
            .store
            .step_trees(step.id)
            .await
            .expect("MemStore never fails a read")
        {
            assert_eq!(
                tree.base_ref,
                fake_tip(repo),
                "`{}` starts from the run branch",
                step.phase_name
            );
        }
    }
    assert!(
        orch.isolator
            .reconcile_onto_log()
            .iter()
            .all(|landing| landing.onto
                == Some((run, BTreeMap::from([(repo, RUN_BRANCH.to_owned())])))),
        "every landing goes onto the run branch"
    );
    assert_eq!(
        bases_of(&orch, run).await,
        [
            "run_branches",
            "prepare prd",
            "prepare prd",
            // The judge prepares an empty scope: no base, no ensure.
            "prepare prd:judge",
            "run_branches",
            "prepare plan",
            // Admission's `base_refs`; the parts and integrate start from them.
            "run_branches",
            "prepare implement/api",
            "prepare implement/ui",
            "prepare implement",
            "run_branches",
            "prepare review",
        ]
    );
}

/// D5: the walk lands a `pull_request` record as its run branch: the request names it, the
/// record settles `pending` with the pushed head, the item is noted, and the next sweep does not
/// take it again.
#[tokio::test]
async fn a_pull_request_run_settles_pushed_and_the_sweep_leaves_it() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::PullRequest)).await;
    let hash = "abcdef0123456789abcdef0123456789abcdef01";
    orch.isolator.script_land(LandOutcome::Pushed {
        hash: hash.to_owned(),
    });

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);

    let requests = orch.isolator.land_requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| (request.repo, request.run_branch.as_deref()))
            .collect::<Vec<_>>(),
        [(repo, Some(RUN_BRANCH))],
        "one landing, of the run branch"
    );
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        records
            .iter()
            .map(|record| (record.mode, record.state, record.landed_hash.as_deref()))
            .collect::<Vec<_>>(),
        [(
            IntegrateMode::PullRequest,
            IntegrationState::Pending,
            Some(hash)
        )]
    );
    assert_eq!(
        integration_notes(&orch).await,
        [format!(
            "integration pushed: repo `htui` run branch {RUN_BRANCH} at abcdef0; its pull \
             request against main waits for MOD-134"
        )]
    );
    assert_eq!(
        orch.land(None).await.expect("the pass runs"),
        0,
        "a pushed record is not taken again"
    );
    assert_eq!(orch.isolator.land_requests().len(), 1);
}

/// D5: a `pending` `pull_request` record whose run's snapshot names no run branch (here edited
/// out after a landing failed) is never pushed as M1's target branch: the pass settles it
/// `refused` (`no_run_branch`) and notes it, so it waits in Unblock rather than silently.
#[tokio::test]
async fn a_pull_request_record_without_a_run_branch_is_refused() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::PullRequest)).await;
    orch.isolator.fail_land(IsolateError::Git(
        "git push timed out after 120s".to_owned(),
    ));
    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);
    assert_eq!(orch.isolator.land_requests().len(), 1);

    let mut snapshot = orch.run(run).await.graph_snapshot.expect("a graph run");
    let entry = snapshot["integrate"][repo.to_string()]
        .as_object_mut()
        .expect("the snapshot freezes the repo's integrate entry");
    assert!(entry.remove("run_branch").is_some(), "{entry:?}");
    assert!(orch.store.set_run_snapshot(run, snapshot));

    assert_eq!(orch.land(Some(run)).await.expect("the pass runs"), 1);
    assert_eq!(
        orch.isolator.land_requests().len(),
        1,
        "the isolator is not asked"
    );
    let reason = no_run_branch("(none in the run's snapshot)");
    let records = orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        (records[0].state, records[0].refusal.as_deref()),
        (IntegrationState::Refused, Some(reason.as_str()))
    );
    assert_eq!(
        integration_notes(&orch).await,
        [format!(
            "integration refused for repo `htui` (pull_request to main): {reason}; the run's \
             work has not landed"
        )]
    );
}

/// A fresh `FEAT` item of the HTUI project, `blocked_by` `FEAT-3`. The link is proposed by a step
/// of a run of its own, cancelled at once, as an agent would have proposed it (MOD-11 D13).
async fn dependent_of_feat_3(orch: &FakeOrchestrator, snapshot: GraphSnapshot) -> ItemId {
    let user = orch.user();
    let item = orch
        .store
        .mint_item(NewItem {
            id: ItemId::new(),
            project_id: ids::PROJECT_HTUI,
            kind_id: ids::KIND_HTUI_FEAT,
            title: "waits on FEAT-3".to_owned(),
            body: String::new(),
            required_tags: Vec::new(),
            touched_paths: Vec::new(),
            priority: 0,
            step_graph_id: None,
            target_id: None,
            created_by: user,
            box_id: Some(orch.box_id()),
        })
        .await
        .expect("the demo project mints")
        .id;
    let run = orch
        .store
        .create_run(NewRun {
            id: RunId::new(),
            project_id: ids::PROJECT_HTUI,
            item_id: item,
            mode: RunMode::Manual,
            target_box_id: orch.box_id(),
            started_by: user,
            graph_snapshot: snapshot,
            repo_scope: Vec::new(),
            queued_at: orch.clock.now(),
            batch_id: None,
            workspace_id: None,
            target: None,
        })
        .await
        .expect("the run is queued")
        .id;
    let step = orch
        .store
        .create_step(NewRunStep {
            id: StepId::new(),
            run_id: run,
            position: 0,
            attempt: 1,
            fanout_index: 0,
            phase_name: "implement".to_owned(),
            agent_id: Some(ids::AGENT_CLAUDE),
            model: Some("opus".to_owned()),
            role: StepRole::Step,
            parent_step_id: None,
            node_key: None,
        })
        .await
        .expect("the step is written")
        .id;
    orch.store
        .propose_link(
            StepFence::Unleased,
            ProposeLink {
                from: item,
                to: ids::HTUI_FEAT_3,
                kind: LinkKind::BlockedBy,
                step,
            },
        )
        .await
        .expect("the step proposes its own item's link");
    orch.store
        .finish_run(run, RunStatus::Cancelled, None, orch.clock.now())
        .await
        .expect("the queued run cancels");
    item
}

/// Whether `item` is in the `Platform` workspace's ready list.
async fn is_ready(orch: &FakeOrchestrator, item: ItemId) -> bool {
    orch.store
        .items(
            &Scope {
                workspace_id: ids::WORKSPACE_PLATFORM,
                project_ids: vec![ids::PROJECT_HTUI, ids::PROJECT_AGY],
            },
            &ItemFilter {
                ready: Some(true),
                ..ItemFilter::default()
            },
        )
        .await
        .expect("MemStore never fails a read")
        .iter()
        .any(|row| row.id == item)
}

/// D5: a pushed `pull_request` record is not landed, so an item `blocked_by` the run's item keeps
/// waiting after the run is `done`; the same walk under `push`, landed, releases it.
#[tokio::test]
async fn its_dependent_stays_unready() {
    for (mode, outcome, released) in [
        (
            IntegrateMode::PullRequest,
            LandOutcome::Pushed {
                hash: "abcdef0123456789abcdef0123456789abcdef01".to_owned(),
            },
            false,
        ),
        (
            IntegrateMode::Push,
            LandOutcome::Landed {
                hash: "abcdef0123456789abcdef0123456789abcdef01".to_owned(),
            },
            true,
        ),
    ] {
        let orch = harness(&[]).await;
        let repo = repo(&orch, Some(REMOTE), Some(mode)).await;
        orch.isolator.script_land(outcome);

        let (run, rest) = started(&orch, repo).await;
        assert_eq!(rest, RunStatus::Done, "{mode:?}");
        let snapshot =
            serde_json::from_value(orch.run(run).await.graph_snapshot.expect("a graph run"))
                .expect("the snapshot decodes");
        let dependent = dependent_of_feat_3(&orch, snapshot).await;
        assert_eq!(
            is_ready(&orch, dependent).await,
            released,
            "{mode:?}: the dependent is released only once the work landed"
        );
    }
}

/// D2: a `push` run makes no `run_branches` call: its call log is the one it made before M3.
#[tokio::test]
async fn a_push_run_makes_no_run_branches_call() {
    let orch = harness(&[]).await;
    let repo = repo(&orch, Some(REMOTE), Some(IntegrateMode::Push)).await;

    let (run, rest) = started(&orch, repo).await;
    assert_eq!(rest, RunStatus::Done);
    assert_eq!(
        call_log_of(&orch, run).await,
        [
            "prepare prd",
            "capture prd",
            "reconcile prd",
            "prepare plan",
            "capture plan",
            "reconcile plan",
            "prepare implement",
            "capture implement",
            "reconcile implement",
            "prepare review",
            "capture review",
            "reconcile review",
            "cleanup run",
            "land",
        ]
    );
    assert!(
        orch.isolator
            .reconcile_onto_log()
            .iter()
            .all(|landing| landing.onto.is_none()),
        "a `push` run reconciles into the primary"
    );
}

/// Rewrites `item`'s live graph through `replace_graph` after `edit` (the conformance suite's
/// helper, over the store).
async fn edit_graph(store: &MemStore, item: ItemId, edit: impl FnOnce(&mut GraphBody)) {
    let graph = store
        .resolve_graph(item)
        .await
        .expect("MemStore never fails a read")
        .expect("the item resolves to a graph")
        .graph
        .id;
    let mut body = store
        .graph_body(graph)
        .await
        .expect("MemStore never fails a read")
        .expect("the item's graph is there");
    edit(&mut body);
    let outcome = store
        .replace_graph(graph, body.graph.revision, body)
        .await
        .expect("the store takes the edited graph");
    assert!(matches!(outcome, CasOutcome::Applied(_)), "{outcome:?}");
}
