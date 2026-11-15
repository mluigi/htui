//! MOD-133 M3 end to end: a `pull_request` run over the real `GixIsolator`, against a local bare
//! remote (no network).
//!
//! Every step of the run starts from the run branch `htui/item/<key>` and lands onto it in a
//! detached landing tree, so the primary's `HEAD`, index and files never move; once the walk is
//! `done` the box pushes the run branch, and the record waits `pending` with the pushed head for
//! MOD-134 to open the pull request.
//!
//! The case opens with [`skip_without_git`] and passes on a box without a usable `git`, as every
//! case of `gix_isolator` does.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use htui_core::fixtures::ids;
use htui_core::model::{
    Gate, IntegrateMode, IntegrationState, Isolation, ItemId, ItemPatch, NewRepo, NewStepGraph,
    PhaseId, RepoBoxPath, RepoId, RunId, RunMode, RunStatus, RunStep, SnapshotCandidate,
    SnapshotPhase, Split, StepGraphId, StepGraphPhase,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{MemStore, ReadStore as _, StoreError, WriteStore as _};
use htui_orch::command::{Command, CommandOutcome, EngineError};
use htui_orch::engine::{
    DeadWalks, Engine, EngineParts, FirstCandidate, NoSink, SessionKey, SessionSink,
};
use htui_orch::fake::{FakeOrchestrator, ScriptedStep};
use htui_orch::isolate::Clock as _;
use htui_orch::isolate::git::Cli;
use htui_orch::isolate::git::testkit::{commit_file, repo_with_one_commit};
use htui_orch::isolate::{GixIsolator, IsolatorConfig, RepoCheckout};
use htui_orch::secrets::RunSecrets;
use htui_orch::skip_without_git;
use htui_orch::verify::{ShellVerifier, VERIFY_CLASS};

/// An `Engine` over `$fix`'s store, graphs, driver, production isolator and verifier, with
/// `$sink`, bound to `$engine` while `$body` is evaluated (`gix_isolator`'s `engine_as`, cut down:
/// the parts it borrows are built here and must outlive the `Engine`).
macro_rules! with_engine {
    ($fix:expr, $sink:expr, |$engine:ident| $body:expr) => {{
        let fix: &Fixture = $fix;
        let graphs = fix.orch.graphs();
        let driver =
            |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| fix.orch.driver_for_key(key);
        let scrubber = MinimalScrubber::new([]);
        let secrets = RunSecrets::none();
        let dead_walks = DeadWalks::new();
        let app = fix
            .orch
            .store
            .app_settings()
            .await
            .expect("MemStore never fails a read");
        let box_profile = fix
            .orch
            .store
            .box_profile(fix.orch.box_id())
            .await
            .expect("MemStore never fails a read")
            .expect("the demo fixture seeds this box");
        let $engine = Engine::new(EngineParts {
            store: &fix.orch.store,
            graphs: &graphs,
            isolator: &fix.isolator,
            verifier: &fix.verifier,
            clock: &fix.orch.clock,
            selector: &FirstCandidate,
            sink: $sink,
            driver: &driver,
            policy: &htui_orch::ask_policy,
            control: &htui_orch::never_cancelled,
            scrubber: &scrubber,
            secrets: &secrets,
            app,
            box_profile,
            box_id: fix.orch.box_id(),
            owner: fix.orch.owner(),
            dead_walks: &dead_walks,
            user: fix.orch.user(),
            tails: htui_orch::Tails::Walk,
            tools: None,
        });
        $body
    }};
}

/// The run branch of `FEAT-3`.
const RUN_BRANCH: &str = "htui/item/FEAT-3";

/// The phases whose step commits a file; every other phase leaves its tree clean.
const COMMITTING: [&str; 2] = ["prd", "implement"];

/// One repository `core` (primary, `pull_request` to a bare remote), a scratch root beside it,
/// and `FEAT-3` ungated in `worktree` isolation.
struct Fixture {
    orch: FakeOrchestrator,
    /// Held for the case's lifetime: dropping it removes the repositories and the scratch root.
    _dir: tempfile::TempDir,
    core: RepoId,
    path: PathBuf,
    /// `core`'s `HEAD` when the fixture was built.
    head: String,
    remote: PathBuf,
    isolator: GixIsolator,
    verifier: ShellVerifier,
}

impl Fixture {
    async fn new(git: &Cli) -> Self {
        let orch = FakeOrchestrator::demo();
        orch.store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock.now())
            .await
            .expect("the seeded run is queued and cancellable");
        orch.script("review", 1, ScriptedStep::review("approve", "fine"));

        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("core");
        std::fs::create_dir_all(&path).expect("the repository directory is made");
        let head = repo_with_one_commit(&path);
        let path = std::fs::canonicalize(&path).expect("the repository directory exists");
        let remote = dir.path().join("core.git");
        git_out(
            git,
            dir.path(),
            &["init", "--quiet", "--bare", &remote.to_string_lossy()],
        )
        .await;
        git_out(
            git,
            &path,
            &[
                "push",
                "--quiet",
                "--",
                &remote.to_string_lossy(),
                "HEAD:refs/heads/main",
            ],
        )
        .await;

        let core = RepoId::new();
        orch.store
            .create_repo(NewRepo {
                id: core,
                project_id: ids::PROJECT_HTUI,
                name: "core".to_owned(),
                remote_url: Some(remote.to_string_lossy().into_owned()),
                default_branch: "main".to_owned(),
                is_primary: true,
            })
            .await
            .expect("the demo project has no repo yet");
        set_pull_request(&orch.store, core).await;
        orch.store
            .upsert_repo_box_path(&RepoBoxPath {
                repo_id: core,
                box_id: ids::BOX,
                local_path: path.to_string_lossy().into_owned(),
                updated_at: htui_orch::engine::truncated(chrono::Utc::now()),
            })
            .await
            .expect("both ids name rows");
        repoint(&orch.store, ids::HTUI_FEAT_3, |phase| {
            phase.isolation = Some(Isolation::Worktree);
            phase.gate = Gate::Never;
        })
        .await;

        let isolator = GixIsolator::new(IsolatorConfig {
            repos: BTreeMap::from([(
                core,
                RepoCheckout {
                    name: "core".to_owned(),
                    local_path: path.clone(),
                    is_primary: true,
                },
            )]),
            scratch_root: dir.path().join("trees"),
            copy_exclude: Vec::new(),
            copy_max_total_bytes: 1 << 30,
            box_id: orch.box_id(),
        })
        .expect("the scratch root is outside the checkout");
        let verifier = ShellVerifier::new(
            &BTreeMap::from([(VERIFY_CLASS.to_owned(), 1)]),
            Arc::new(MinimalScrubber::new([])),
            Arc::new(htui_orch::isolate::SystemClock),
        );
        Self {
            orch,
            _dir: dir,
            core,
            path,
            head,
            remote,
            isolator,
            verifier,
        }
    }

    /// One command through an `Engine` over the production isolator and verifier.
    async fn dispatch<K: SessionSink>(
        &self,
        sink: &K,
        command: Command,
    ) -> Result<CommandOutcome, EngineError> {
        with_engine!(self, sink, |engine| engine.dispatch(command).await)
    }

    /// A landing pass over `run`, as the sweep makes one.
    async fn land(&self, run: RunId) -> Result<usize, EngineError> {
        with_engine!(self, &NoSink, |engine| engine.land(Some(run)).await)
    }
}

/// `core`'s row turned `pull_request`.
async fn set_pull_request(store: &MemStore, repo: RepoId) {
    let row = store
        .repos(ids::PROJECT_HTUI)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|row| row.id == repo)
        .expect("the fixture made the repo");
    let outcome = store
        .update_repo(
            repo,
            row.updated_at,
            htui_core::model::RepoPatch {
                integrate: Some(Some(IntegrateMode::PullRequest)),
                ..htui_core::model::RepoPatch::default()
            },
        )
        .await
        .expect("the repo is there");
    assert!(
        matches!(outcome, htui_core::store::CasOutcome::Applied(_)),
        "{outcome:?}"
    );
}

/// Repoints `item` at a clone of its graph whose phases `mutate` has edited (`gix_isolator`'s
/// helper, copied: that module's is private to it).
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
            name: format!("{}-run-branch-{id}", row.key),
            description: "a run_branch case's edit of the live graph".to_owned(),
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
                reason: "a run_branch case's edit of the live graph".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// `git <args>` in `repo`, its stdout trimmed; a non-zero exit fails the case.
async fn git_out(git: &Cli, repo: &Path, args: &[&str]) -> String {
    let argv: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let exited = git.run("oracle", repo, &argv, &[]).await.expect("git runs");
    assert!(exited.ok(), "git {args:?} failed: {}", exited.stderr);
    exited.stdout.trim().to_owned()
}

/// A [`SessionSink`] that commits `<phase>.txt` into the step's tree for each [`COMMITTING`]
/// phase, then lets the fake orchestrator write the output document.
struct CommittingSink<'a> {
    orch: &'a FakeOrchestrator,
}

impl SessionSink for CommittingSink<'_> {
    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        key: &SessionKey<'_>,
        _done: &htui_agent::event::DoneEvent,
    ) -> Result<(), StoreError> {
        if COMMITTING.contains(&phase.name.as_str()) {
            for tree in self.orch.store.step_trees(step.id).await? {
                commit_file(
                    Path::new(&tree.path),
                    &format!("{}.txt", phase.name),
                    &format!("{} attempt {}\n", phase.name, step.attempt),
                    &format!("htui test: {}", phase.name),
                );
            }
        }
        FakeOrchestrator::after_done(self.orch, item, step, phase, key).await?;
        Ok(())
    }
}

/// MOD-133 M3 D2, D3, D5: a `pull_request` run with two committing `worktree` phases. The second
/// starts from the first's merge on the run branch, both merges land on `htui/item/FEAT-3` and
/// nowhere else, the primary's `HEAD`, branch and tree never move, the run branch is pushed to
/// the remote (whose `main` stays put), and the record waits `pending` with the pushed head; a
/// second landing pass takes nothing.
#[tokio::test]
async fn a_pull_request_run_lands_on_its_run_branch_and_pushes_it() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let fix = Fixture::new(&git).await;
    let sink = CommittingSink { orch: &fix.orch };

    let outcome = fix
        .dispatch(
            &sink,
            Command::StartRun {
                item: ids::HTUI_FEAT_3,
                mode: RunMode::Manual,
                repo_scope: Some(vec![fix.core]),
                workspace: None,
            },
        )
        .await
        .expect("the graph resolves and the box has a slot");
    let CommandOutcome::Started { run, rest } = outcome else {
        panic!("`StartRun` answers `Started`, not {outcome:?}");
    };
    assert_eq!(rest.run, RunStatus::Done);

    // The primary never moved.
    assert_eq!(
        git_out(&git, &fix.path, &["rev-parse", "HEAD"]).await,
        fix.head
    );
    assert_eq!(
        git_out(&git, &fix.path, &["symbolic-ref", "HEAD"]).await,
        "refs/heads/main"
    );
    assert_eq!(
        git_out(&git, &fix.path, &["status", "--porcelain"]).await,
        ""
    );
    assert!(
        !fix.path.join("prd.txt").exists() && !fix.path.join("implement.txt").exists(),
        "no step's file reached the primary's tree"
    );

    // The run branch holds both merges, each a first-parent step over the one before.
    let tip = git_out(
        &git,
        &fix.path,
        &["rev-parse", &format!("refs/heads/{RUN_BRANCH}")],
    )
    .await;
    let merges = git_out(
        &git,
        &fix.path,
        &[
            "rev-list",
            "--merges",
            "--first-parent",
            &format!("{}..{RUN_BRANCH}", fix.head),
        ],
    )
    .await;
    assert_eq!(merges.lines().count(), 2, "two merges: {merges}");
    assert_eq!(
        merges.lines().next(),
        Some(tip.as_str()),
        "the tip is a merge"
    );
    // D2: `implement` started from `prd`'s merge on the run branch, not from the primary's HEAD.
    let first = merges.lines().nth(1).expect("the older merge is prd's");
    let mut implement = Vec::new();
    for step in fix.orch.steps(run).await {
        if step.phase_name == "implement" {
            implement.extend(
                fix.orch
                    .store
                    .step_trees(step.id)
                    .await
                    .expect("MemStore never fails a read")
                    .into_iter()
                    .map(|tree| tree.base_ref),
            );
        }
    }
    assert_eq!(implement, [first], "implement starts from prd's merge");
    let files = git_out(&git, &fix.path, &["ls-tree", "--name-only", RUN_BRANCH]).await;
    assert_eq!(
        files.lines().collect::<Vec<_>>(),
        ["f", "implement.txt", "prd.txt"],
        "both steps' files are on the run branch"
    );

    // The remote holds the run branch; its `main` is the pull request's base, untouched.
    assert_eq!(
        git_out(
            &git,
            &fix.remote,
            &["rev-parse", &format!("refs/heads/{RUN_BRANCH}")]
        )
        .await,
        tip
    );
    assert_eq!(
        git_out(&git, &fix.remote, &["rev-parse", "refs/heads/main"]).await,
        fix.head
    );

    let records = fix
        .orch
        .store
        .integrations(run)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record.repo_id,
                record.mode,
                record.state,
                record.landed_hash.clone()
            ))
            .collect::<Vec<_>>(),
        [(
            fix.core,
            IntegrateMode::PullRequest,
            IntegrationState::Pending,
            Some(tip.clone())
        )]
    );

    assert_eq!(
        fix.land(run).await.expect("the pass runs"),
        0,
        "a pushed record is not taken again"
    );
}
