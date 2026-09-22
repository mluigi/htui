//! `GixIsolator` and `ShellVerifier` under the real `Engine` (plan D41).
//!
//! The fake suite pins the walk's *decisions* over a double that touches no filesystem; this
//! binary pins the three validation criteria that are about a filesystem — ANA-2 §12's 11, 12 and
//! 13 (`docs/ANA-2.md:2115-2121`) — by assembling `EngineParts` over the production isolator and
//! the production verifier and driving them through the same commands.
//!
//! Every git-backed case opens with [`skip_without_git`] and **passes** on a box without a `git`
//! at or above 2.33.0 (plan D40): the sentence is printed and the case returns. Criterion 12 needs
//! no `git` at all — `local` isolation reads a checkout and creates nothing — so it is not skipped
//! anywhere.
//!
//! The oracle for what `git` believes is `git worktree list --porcelain`, run through the same
//! binary that wrote the administrative entries (`isolate::git::testkit::worktree_list`). The
//! `gix` side of the same question is asserted inside `isolate/real.rs`'s own tests, which can
//! reach the crate-private readers this binary cannot.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use htui_core::fixtures::ids;
use htui_core::model::{
    Isolation, ItemId, ItemPatch, NewRepo, NewStepGraph, PhaseId, RepoBoxPath, RepoId, RunId,
    RunMode, RunStatus, RunStep, SnapshotCandidate, SnapshotPhase, StepGraphId, StepGraphPhase,
    StepStatus, VerifyOutcome,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{MemStore, ReadStore as _, StoreError, WriteStore as _};
use htui_orch::command::{Command, CommandOutcome, EngineError, GateAnswer};
use htui_orch::engine::{Engine, EngineParts, FirstCandidate, SessionSink};
use htui_orch::fake::FakeOrchestrator;
use htui_orch::isolate::Clock as _;
use htui_orch::isolate::git::Cli;
use htui_orch::isolate::git::testkit::{commit_file, repo_with_one_commit, worktree_list};
use htui_orch::isolate::{GixIsolator, IsolatorConfig, RepoCheckout};
use htui_orch::skip_without_git;
use htui_orch::verify::{ShellVerifier, VERIFY_CLASS};

/// Everything one case drives: the store and the doubles a `FakeOrchestrator` already owns, plus
/// the two production pieces this binary exists to exercise.
struct Fixture {
    orch: FakeOrchestrator,
    /// Held for the case's lifetime: dropping it removes the repositories and the scratch root.
    _dir: tempfile::TempDir,
    root: PathBuf,
    core: Checkout,
    docs: Checkout,
    isolator: GixIsolator,
    verifier: ShellVerifier,
}

/// One repository as both halves name it: the store's id and the isolator's path.
struct Checkout {
    id: RepoId,
    path: PathBuf,
    /// `HEAD` at the moment the fixture was built, which is every step's `before_hash`.
    head: String,
}

impl Fixture {
    /// Two repositories, `core` (primary) and `docs`, a scratch root beside them, and every phase
    /// of `FEAT-3`'s graph pinned to `isolation` with `verify` as its `verify_command`.
    ///
    /// The isolation is set on the **phase** and not on the project: `ProjectPatch` has no
    /// `settings` field (`crates/htui-core/src/model/kind.rs`), so the project rung is not
    /// reachable through a writer, and `phase.isolation` is the rung above it anyway
    /// (`graph.rs`'s `phase.isolation.unwrap_or(settings.default_isolation)`).
    async fn new(isolation: Isolation, verify: Option<&str>) -> Self {
        let orch = FakeOrchestrator::demo();
        orch.store
            .finish_run(ids::RUN_2, RunStatus::Cancelled, None, orch.clock.now())
            .await
            .expect("the seeded run is queued and cancellable");

        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = make_repo(&orch.store, dir.path(), "core", true).await;
        let docs = make_repo(&orch.store, dir.path(), "docs", false).await;

        let verify = verify.map(str::to_owned);
        repoint(&orch.store, ids::HTUI_FEAT_3, |phase| {
            phase.isolation = Some(isolation);
            phase.verify_command = verify.clone();
        })
        .await;

        let root = dir.path().join("trees");
        let isolator = GixIsolator::new(IsolatorConfig {
            repos: BTreeMap::from([
                (
                    core.id,
                    RepoCheckout {
                        name: "core".to_owned(),
                        local_path: core.path.clone(),
                        is_primary: true,
                    },
                ),
                (
                    docs.id,
                    RepoCheckout {
                        name: "docs".to_owned(),
                        local_path: docs.path.clone(),
                        is_primary: false,
                    },
                ),
            ]),
            scratch_root: root.clone(),
            copy_exclude: Vec::new(),
            copy_max_total_bytes: 1 << 30,
            box_id: orch.box_id(),
        })
        .expect("the scratch root is outside both checkouts");

        let verifier = ShellVerifier::new(
            &BTreeMap::from([(VERIFY_CLASS.to_owned(), 1)]),
            Arc::new(MinimalScrubber::new([])),
            Arc::new(htui_orch::isolate::SystemClock),
        );

        Self {
            orch,
            _dir: dir,
            root,
            core,
            docs,
            isolator,
            verifier,
        }
    }

    /// One command through an `Engine` built over the production isolator and verifier.
    ///
    /// `EngineParts`' fields are all public, which is what plan D41 relies on: a test assembles the
    /// walk out of whichever halves it wants to be real, and nothing in `engine.rs` is generic over
    /// a harness.
    async fn dispatch<K: SessionSink>(
        &self,
        sink: &K,
        command: Command,
    ) -> Result<CommandOutcome, EngineError> {
        let graphs = self.orch.graphs();
        let driver = |_candidate: &SnapshotCandidate, phase: &str, attempt: i32| {
            self.orch.driver_for(phase, attempt)
        };
        let scrubber = MinimalScrubber::new([]);
        let app = self
            .orch
            .store
            .app_settings()
            .await
            .expect("MemStore never fails a read");
        let box_profile = self
            .orch
            .store
            .box_profile(self.orch.box_id())
            .await
            .expect("MemStore never fails a read")
            .expect("the demo fixture seeds this box");

        let engine = Engine::new(EngineParts {
            store: &self.orch.store,
            graphs: &graphs,
            isolator: &self.isolator,
            verifier: &self.verifier,
            clock: &self.orch.clock,
            selector: &FirstCandidate,
            sink,
            driver: &driver,
            scrubber: &scrubber,
            app,
            box_profile,
            box_id: self.orch.box_id(),
            owner: self.orch.owner(),
            user: self.orch.user(),
        });
        engine.dispatch(command).await
    }

    /// `StartRun` over both repositories, in `core, docs` order.
    async fn start<K: SessionSink>(&self, sink: &K) -> RunId {
        let outcome = self
            .dispatch(
                sink,
                Command::StartRun {
                    item: ids::HTUI_FEAT_3,
                    mode: RunMode::Manual,
                    repo_scope: Some(vec![self.core.id, self.docs.id]),
                },
            )
            .await
            .expect("the graph resolves and the box has a slot");
        let CommandOutcome::Started { run, .. } = outcome else {
            panic!("`StartRun` answers `Started`, not {outcome:?}");
        };
        run
    }

    async fn steps(&self, run: RunId) -> Vec<RunStep> {
        self.orch
            .store
            .run_steps(run)
            .await
            .expect("MemStore never fails a read")
    }

    /// The scratch root as `git` and `std::fs` both spell it, which is what a `starts_with`
    /// against a porcelain path has to be compared to (blueprint H-11).
    fn canonical_root(&self) -> PathBuf {
        std::fs::canonicalize(&self.root).expect("`GixIsolator::new` created the scratch root")
    }

    /// The step the walk parked at, which every case asserts it reached.
    async fn parked(&self, run: RunId) -> RunStep {
        self.steps(run)
            .await
            .into_iter()
            .find(|step| step.status == StepStatus::AwaitingApproval)
            .expect("the walk parked at a gate")
    }
}

/// A repository with one commit, registered with the store as `name` on this box.
async fn make_repo(store: &MemStore, dir: &Path, name: &str, is_primary: bool) -> Checkout {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).expect("the repository directory is made");
    let head = repo_with_one_commit(&path);

    let id = RepoId::new();
    store
        .create_repo(NewRepo {
            id,
            project_id: ids::PROJECT_HTUI,
            name: name.to_owned(),
            remote_url: None,
            default_branch: "main".to_owned(),
            is_primary,
        })
        .await
        .expect("the demo project holds no repo of this name");
    // Not read by the walk — the isolator is handed its paths (plan D34) — but this is the row
    // milestone 6's worker builds `IsolatorConfig` out of, and writing it keeps the two halves of
    // the fixture saying the same thing.
    store
        .upsert_repo_box_path(&RepoBoxPath {
            repo_id: id,
            box_id: ids::BOX,
            local_path: path.to_string_lossy().into_owned(),
            updated_at: htui_orch::engine::truncated(chrono::Utc::now()),
        })
        .await
        .expect("both ids name rows");

    Checkout {
        id,
        path: std::fs::canonicalize(&path).expect("the repository directory exists"),
        head,
    }
}

/// Repoints `item` at a clone of its graph whose phases `mutate` has edited.
///
/// The conformance suite's own helper, copied rather than shared because `conformance.rs`'s is
/// written against its `Orchestrate` trait and this binary holds a store.
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
    let clone = store
        .create_step_graph(NewStepGraph {
            id: StepGraphId::new(),
            project_id: row.project_id,
            name: format!("{}-isolated", row.key),
            description: "a gix_isolator case's edit of the live graph".to_owned(),
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
                reason: "a gix_isolator case's edit of the live graph".to_owned(),
                ..ItemPatch::default()
            },
        )
        .await
        .expect("the item's version is current");
}

/// `git status --porcelain` in `repo`, which is the only reading of "dirty" this binary can make:
/// `isolate::git::is_dirty` is crate-private and `real.rs`'s own tests are where `gix`'s answer is
/// asserted.
async fn porcelain_status(git: &Cli, repo: &Path) -> String {
    let exited = git
        .run(
            "status",
            repo,
            &[OsStr::new("status"), OsStr::new("--porcelain")],
            &[],
        )
        .await
        .expect("git status runs");
    assert!(exited.ok(), "git status failed: {}", exited.stderr);
    exited.stdout
}

/// `HEAD` of `repo`, read through `git` rather than through the crate's private `gix` half.
async fn head_of(git: &Cli, repo: &Path) -> String {
    let exited = git
        .run(
            "rev-parse",
            repo,
            &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
            &[],
        )
        .await
        .expect("git rev-parse runs");
    assert!(exited.ok(), "git rev-parse failed: {}", exited.stderr);
    exited.stdout.trim().to_owned()
}

/// A [`SessionSink`] that commits a file into the step's primary tree and then lets the fake
/// orchestrator write the output document.
///
/// This is the only way a case can make a step produce a *commit*: the walk's agent is a
/// `FakeDriver` and a fake agent edits nothing. Criterion 13 needs one, because a run whose steps
/// committed nothing has nothing for `reconcile` to merge and nothing for cleanup to be
/// interesting about.
struct CommittingSink<'a> {
    orch: &'a FakeOrchestrator,
    /// The repository directory names to commit in; a tree not named here is left clean, which is
    /// what D27 removes at `capture`.
    repos: &'a [&'a str],
}

impl SessionSink for CommittingSink<'_> {
    async fn after_done(
        &self,
        item: ItemId,
        step: &RunStep,
        phase: &SnapshotPhase,
        _done: &htui_agent::event::DoneEvent,
    ) -> Result<(), StoreError> {
        let trees = self.orch.store.step_trees(step.id).await?;
        for tree in &trees {
            if !self
                .repos
                .iter()
                .any(|name| tree.path.ends_with(&format!("/{name}")))
            {
                continue;
            }
            commit_file(
                Path::new(&tree.path),
                "agent.txt",
                &format!("{} attempt {}\n", phase.name, step.attempt),
                &format!("htui test: {}", phase.name),
            );
        }
        // `after_done` answers the document it wrote; the seam wants a unit.
        FakeOrchestrator::after_done(self.orch, item, step, phase).await?;
        Ok(())
    }
}

/// ANA-2 §12 criterion 11 (`docs/ANA-2.md:2115`): a `worktree` step over two repositories gets one
/// tree per repository, both outside every managed checkout, both recorded.
#[tokio::test]
async fn criterion_11_a_worktree_step_on_two_repos() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let fix = Fixture::new(Isolation::Worktree, None).await;
    // Both trees are committed to, which is what keeps both alive past `capture`: D27 removes a
    // `worktree` tree the step left clean and without a commit, and the criterion asks `git
    // worktree list` to show them.
    let sink = CommittingSink {
        orch: &fix.orch,
        repos: &["core", "docs"],
    };
    let run = fix.start(&sink).await;
    let step = fix.parked(run).await;

    let trees = fix
        .orch
        .store
        .step_trees(step.id)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(trees.len(), 2, "one tree per repo in scope: {trees:?}");
    for tree in &trees {
        assert_eq!(tree.mode, Isolation::Worktree);
        let path = PathBuf::from(&tree.path);
        assert!(
            path.starts_with(fix.canonical_root()),
            "every tree is under the scratch root: {path:?}"
        );
        assert!(
            !path.starts_with(&fix.core.path) && !path.starts_with(&fix.docs.path),
            "and inside neither managed checkout (invariant 4): {path:?}"
        );
        assert!(path.join(".git").exists(), "the tree checked out: {path:?}");
    }

    let commits = fix
        .orch
        .store
        .step_commits(step.id)
        .await
        .expect("MemStore never fails a read");
    let mut bases: Vec<(RepoId, String)> = commits
        .iter()
        .map(|row| (row.repo_id, row.before_hash.clone()))
        .collect();
    bases.sort();
    let mut expected = vec![
        (fix.core.id, fix.core.head.clone()),
        (fix.docs.id, fix.docs.head.clone()),
    ];
    expected.sort();
    assert_eq!(
        bases, expected,
        "each `before_hash` is its repository's own `HEAD`"
    );

    // Plan D33: `upsert_step_tree` writes the primary's path to `run_step.isolation_path`.
    let primary = trees
        .iter()
        .find(|tree| tree.repo_id == fix.core.id)
        .expect("the primary has a tree");
    assert_eq!(
        fix.steps(run)
            .await
            .into_iter()
            .find(|row| row.id == step.id)
            .and_then(|row| row.isolation_path),
        Some(primary.path.clone())
    );

    // The oracle: the binary that wrote the entries, asked what it believes.
    for (checkout, repo_name) in [(&fix.core, "core"), (&fix.docs, "docs")] {
        let listed = worktree_list(&git, &checkout.path).await;
        // The main checkout's own directory is also named `core`/`docs`, so the entry is found by
        // "under the scratch root" and not by its last component.
        let entry = listed
            .iter()
            .find(|entry| {
                entry.path.starts_with(fix.canonical_root()) && entry.path.ends_with(repo_name)
            })
            .unwrap_or_else(|| panic!("`git` lists the tree it made: {listed:?}"));
        assert_eq!(
            entry.branch.as_deref(),
            Some(format!("refs/heads/htui/{}", step.id).as_str()),
            "D23 branches the tree at `htui/<step_id>`"
        );
        assert_eq!(
            entry.locked.as_deref(),
            Some(format!("htui run {run}").as_str()),
            "D23 locks it with the run in the reason"
        );
    }
}

/// ANA-2 §12 criterion 12 (`docs/ANA-2.md:2118`), its record half: a `local` step over a dirty
/// checkout records `dirty = true` and the checkout's own `HEAD`.
///
/// **No `git` needed**: `local` reads the checkout and creates nothing, which is exactly what
/// makes it the mode a box without `git` can still walk. The sweep that acts on `dirty = true` is
/// milestone 5's and is not asserted here.
#[tokio::test]
async fn criterion_12_a_local_step_on_a_dirty_tree() {
    let fix = Fixture::new(Isolation::Local, None).await;
    std::fs::write(fix.core.path.join("f"), "edited by a human\n")
        .expect("the tracked file is writable");

    let run = fix.start(&fix.orch).await;
    let step = fix.parked(run).await;

    let trees = fix
        .orch
        .store
        .step_trees(step.id)
        .await
        .expect("MemStore never fails a read");
    let core = trees
        .iter()
        .find(|tree| tree.repo_id == fix.core.id)
        .expect("the primary has a row");
    assert_eq!(core.mode, Isolation::Local);
    assert_eq!(
        PathBuf::from(&core.path),
        fix.core.path,
        "`local` is the maintainer's own checkout, not a scratch tree"
    );
    assert!(
        core.dirty,
        "a modified tracked file is recorded, never reset"
    );
    assert_eq!(core.base_ref, fix.core.head);

    let docs = trees
        .iter()
        .find(|tree| tree.repo_id == fix.docs.id)
        .expect("the second repo has a row");
    assert!(!docs.dirty, "nothing touched `docs`");

    let commits = fix
        .orch
        .store
        .step_commits(step.id)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(
        commits
            .iter()
            .find(|row| row.repo_id == fix.core.id)
            .map(|row| row.before_hash.clone()),
        Some(fix.core.head.clone())
    );
    assert!(
        !fix.root.join(run.to_string()).exists(),
        "`local` creates nothing under the scratch root"
    );
}

/// ANA-2 §12 criterion 13 (`docs/ANA-2.md:2120`): cancelling a run removes every tree it made and
/// leaves the managed checkouts exactly as it found them — merge included.
#[tokio::test]
async fn criterion_13_cancel_removes_every_tree() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let fix = Fixture::new(Isolation::Worktree, None).await;
    // Only the primary is committed to, so `docs`'s clean no-commit tree is removed at `capture`
    // (D27) and its reconcile is the identity — the shape the criterion's "leaves the managed
    // repo's own tree untouched" half is easiest to read on.
    let sink = CommittingSink {
        orch: &fix.orch,
        repos: &["core"],
    };

    let run = fix.start(&sink).await;
    let prd = fix.parked(run).await;
    assert_eq!(prd.position, 0);

    // Approving `prd` runs plan D25's reconcile: the step's branch is merged `--no-ff` into the
    // primary checkout, which is the one thing in this milestone that moves a managed repository.
    fix.dispatch(
        &sink,
        Command::AnswerGate {
            run,
            step: prd.id,
            answer: GateAnswer::Approved,
        },
    )
    .await
    .expect("`prd` produced its document");

    let merged = head_of(&git, &fix.core.path).await;
    assert_ne!(
        merged, fix.core.head,
        "the winner's commit reached the primary checkout (ANA-2 `:987-988`)"
    );
    assert_eq!(
        head_of(&git, &fix.docs.path).await,
        fix.docs.head,
        "`docs` had no commit, so its reconcile was the identity"
    );

    let parked = fix.parked(run).await;
    assert_eq!(parked.position, 1, "the walk advanced and parked at `plan`");

    let outcome = fix
        .dispatch(&sink, Command::CancelRun { run })
        .await
        .expect("a parked run is cancellable");
    assert!(
        matches!(outcome, CommandOutcome::Cancelled { .. }),
        "{outcome:?}"
    );

    assert!(
        !fix.root.join(run.to_string()).exists(),
        "the run's whole scratch directory is gone (blueprint F-Q)"
    );
    for checkout in [&fix.core, &fix.docs] {
        let listed = worktree_list(&git, &checkout.path).await;
        for entry in &listed {
            assert!(
                !entry.path.starts_with(fix.canonical_root()),
                "no entry under the scratch root survives: {entry:?}"
            );
            assert!(
                entry
                    .branch
                    .as_deref()
                    .is_none_or(|branch| !branch.starts_with("refs/heads/htui/")),
                "and no `htui/` branch is left checked out anywhere: {entry:?}"
            );
        }
    }
    assert_eq!(
        head_of(&git, &fix.core.path).await,
        merged,
        "cleanup moves no managed checkout"
    );
    assert_eq!(head_of(&git, &fix.docs.path).await, fix.docs.head);
    assert_eq!(
        porcelain_status(&git, &fix.core.path).await,
        "",
        "and leaves it clean"
    );
    assert_eq!(porcelain_status(&git, &fix.docs.path).await, "");
}

/// Plan D30 end to end: the phase's `verify_command` runs in the **primary repo's tree**, and the
/// `command_run` row names that tree.
#[tokio::test]
async fn a_verify_command_runs_in_the_primary_tree() {
    let Some(_git) = skip_without_git!() else {
        return;
    };
    // `f` is `repo_with_one_commit`'s file and exists only inside a checkout, so a `pass` proves
    // where the shell ran as well as that it ran.
    let fix = Fixture::new(Isolation::Worktree, Some("test -f f")).await;
    let run = fix.start(&fix.orch).await;
    let step = fix.parked(run).await;

    assert_eq!(step.verify_outcome, Some(VerifyOutcome::Pass));
    assert_eq!(step.verify_exit_code, Some(0));

    let rows = fix
        .orch
        .store
        .command_runs(step.id)
        .await
        .expect("MemStore never fails a read");
    assert_eq!(rows.len(), 1, "one verify, one row: {rows:?}");
    let primary = fix
        .orch
        .store
        .step_trees(step.id)
        .await
        .expect("MemStore never fails a read")
        .into_iter()
        .find(|tree| tree.repo_id == fix.core.id)
        .expect("the primary has a tree");
    assert_eq!(
        (
            rows[0].class.as_str(),
            rows[0].cwd.as_str(),
            rows[0].exit_code
        ),
        (VERIFY_CLASS, primary.path.as_str(), Some(0)),
        "ANA-2 `:491-493`: the step's own tree for the project's primary repo"
    );
}
