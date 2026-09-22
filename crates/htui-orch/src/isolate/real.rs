//! `GixIsolator`: the production [`Isolator`], four modes over `gix` reads and
//! five `git` verbs (plan D22–D47).
//!
//! This is the first place ANA-2 §4.6's four isolation modes exist as *behaviour* rather than as
//! pieces: `isolate/git.rs` knows what a worktree is, `isolate/copy.rs` knows what a copy is, and
//! neither knows which of them a step wants. Every mode is one private `async fn` per verb, and
//! every call into the two sync modules runs through [`git::blocking`] (`spawn_blocking`) with
//! owned paths — no `gix::Repository` crosses an `.await` (blueprint H-17), and no `.await` happens
//! under a `std` lock (`crates/htui-core/src/store/mem.rs:3-7` is the precedent). The `git` verbs
//! are the one exception by design: they are `async` process supervision, and the `gix`
//! post-conditions inside them go through the same helper.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use htui_core::model::{BoxId, Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId};
use tokio::sync::OwnedMutexGuard;

use super::copy;
use super::git::{self, Cli, blocking};
use super::{IsolateError, Isolator, IsolatorFuture, Prepared, PreparedTree};

// One named free function per refusal sentence, the house style of `htui_core`'s store refusals
// (`crates/htui-core/src/store/traits.rs:1053-1160`). The two `copy` refusals and the unborn-HEAD
// one are *not* here: they live in `isolate/copy.rs` and `isolate/git.rs` beside the code that
// detects them, and this module passes them through rather than spelling them a second time.

/// The scope names a repository this box has no checkout of (plan D34).
///
/// The text `engine.rs:2606` pins for the fake, so the real isolator's refusal reads identically.
#[must_use]
pub fn no_checkout_for_repo() -> String {
    "no checkout for this repo on this box".to_owned()
}

/// ANA-2 invariant 4: trees live outside every managed repository, and a scratch root *under* one
/// would put every tree inside the repository it is isolating from.
#[must_use]
pub fn root_inside_repo(root: &Path, repo: &Path) -> String {
    format!(
        "scratch root {} is inside a managed repository ({})",
        root.display(),
        repo.display()
    )
}

/// The same invariant from the other side: a checkout under the scratch root would be deleted by
/// the run-terminal `remove_dir_all` of its own parent.
#[must_use]
pub fn repo_inside_root(repo: &Path, root: &Path) -> String {
    format!(
        "managed repository {} is inside the scratch root {}",
        repo.display(),
        root.display()
    )
}

/// Two repositories of one scope share a directory name, so their trees would be one directory.
#[must_use]
pub fn duplicate_repo_name(name: &str) -> String {
    format!("duplicate repo name {name}")
}

/// The `worktree` mode over a repository with submodules (the plan's mode table).
///
/// `git worktree add` checks the gitlink out and leaves the submodule uninitialised, so the agent
/// would be handed a tree that does not build.
#[must_use]
pub fn submodules_refused() -> String {
    "submodules: worktree isolation is not supported".to_owned()
}

/// D46: an administrative entry that survived `worktree remove`, named rather than pruned.
///
/// Not a refusal — the payload of an [`IsolateError::Git`] — because cleanup is run-terminal and
/// nothing is left to refuse; it is a sentence for the operator, who owns the `prune` we never run.
#[must_use]
pub fn stale_worktree(path: &Path) -> String {
    format!(
        "stale worktree entry {}; run `git worktree prune` yourself",
        path.display()
    )
}

/// Blueprint H-4 for the `copy` mode: the tree a row names is not there any more.
///
/// A copy is never removed before the run is terminal, so — unlike a `worktree` row, whose branch
/// survives every removal and answers for it — there is nothing left to read the step's work from.
#[must_use]
pub fn copy_tree_vanished(path: &Path) -> String {
    format!("the copy at {} is gone", path.display())
}

/// `<root>/<run_id>/<step_id>/`: the common parent of a step's trees and the `cwd` of its session
/// under the two isolating modes (plan D28).
fn session_dir(root: &Path, run: RunId, step: StepId) -> PathBuf {
    root.join(run.to_string()).join(step.to_string())
}

/// `path` resolved through the filesystem, or `path` itself when it does not exist yet.
///
/// Every path this module compares or persists goes through here (blueprint H-11): a symlinked
/// checkout, a `TempDir` under a symlinked `TMPDIR` and a relative `local_path` all spell the same
/// directory differently, and `RunStepTree.path` is a `String` that later gets compared.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// One repository's checkout on this box, resolved by the caller (plan D34).
///
/// `htui-orch` never depends on `htui-store` (ANA-2 invariant 10), so the `repo` and
/// `repo_box_path` rows are read by the caller and handed over as this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCheckout {
    /// `repo.name`: the directory name a tree of this repository takes under `<root>/<run>/<step>/`.
    pub name: String,
    /// `repo_box_path.local_path`, canonicalised by [`GixIsolator::new`].
    pub local_path: PathBuf,
    /// `repo.is_primary`: the checkout a `local` session runs in and the one `reconcile` merges
    /// into.
    pub is_primary: bool,
}

/// Everything [`GixIsolator`] needs that is not a `git` fact (plan D34).
#[derive(Debug, Clone)]
pub struct IsolatorConfig {
    /// Every repository this box can isolate, by id.
    pub repos: BTreeMap<RepoId, RepoCheckout>,
    /// `identity::config_root()/trees` (ANA-2 `:905`). Passed, not read: this crate never depends
    /// on `htui-store`, which is where `identity.rs` lives.
    pub scratch_root: PathBuf,
    /// `ProjectSettings.copy_exclude`; empty falls back to [`super::copy::DEFAULT_COPY_EXCLUDE`].
    pub copy_exclude: Vec<String>,
    /// `app_setting.copy_max_total_bytes` (`0003:134`), the cap `copy` measures against.
    pub copy_max_total_bytes: u64,
    /// `box.id`, half of D43's guard key.
    pub box_id: BoxId,
}

/// D43's guard table: one `tokio` mutex per `(box, repo)`, minted on first use and never removed.
type Guards = Mutex<BTreeMap<(BoxId, RepoId), Arc<tokio::sync::Mutex<()>>>>;

/// The guards a step is holding between its own `prepare` and its own `capture` (blueprint A-1),
/// keyed by the run too so the run's `cleanup` can reach a guard no row names.
type Held = Mutex<BTreeMap<(RunId, StepId), Vec<OwnedMutexGuard<()>>>>;

/// The production [`Isolator`]: four modes over `gix` reads and five `git` verbs (plan D22–D47).
#[derive(Debug)]
pub struct GixIsolator {
    /// Validated and canonicalised once by [`GixIsolator::new`].
    config: IsolatorConfig,
    /// [`Cli::locate`]'s answer, taken once (plan D40): the refusal sentence is what the two modes
    /// that shell out answer with, so a test can never pass on a box where production would refuse.
    git: Result<Cli, String>,
    /// D43: one guard per `(box, repo)`, minted on first use and never removed — a `BTreeMap` of
    /// `Arc`s, not of guards.
    locks: Guards,
    /// Blueprint A-1: the guards a step is holding, from its `prepare` to its own `capture`.
    ///
    /// The guard is the *step*'s, released at its own `capture`: every phase of a run resolves to
    /// the project's `default_isolation` (`model/kind.rs:256`), so a guard held to the run's
    /// `cleanup` would deadlock the second step of any multi-step `shared_serialized` run against
    /// the first. The key carries the run as well because [`cleanup`](Isolator::cleanup) must
    /// release what a crashed step left **without** a row to name it: the engine's
    /// `upsert_step_tree` can fail after `prepare` took the guard, and then no row exists.
    held: Held,
}

impl GixIsolator {
    /// Validates the configuration against ANA-2 invariant 4 and probes `git` once.
    ///
    /// Synchronous, because it spawns `git --version`: milestone 6 calls it at worker start, once
    /// per process. A box with no usable `git` is **not** an error here — `local` and
    /// `shared_serialized` never shell out, so the isolator is built and the two modes that do
    /// refuse with the sentence the probe produced.
    ///
    /// # Errors
    /// [`IsolateError::Refused`] with [`root_inside_repo`] or [`repo_inside_root`];
    /// [`IsolateError::Io`] when the scratch root cannot be created.
    pub fn new(config: IsolatorConfig) -> Result<Self, IsolateError> {
        let git = Cli::locate().map_err(|err| match err {
            IsolateError::Refused(reason) => reason,
            other => other.to_string(),
        });
        Self::assembled(config, git)
    }

    /// [`new`](GixIsolator::new) with the `git` decision made by the caller.
    ///
    /// The seam D40's "refused without git" case needs: a test cannot remove `git` from the box it
    /// runs on, and the refusal has to be provable on a box that has one.
    ///
    /// # Errors
    /// As [`new`](GixIsolator::new).
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_git(
        config: IsolatorConfig,
        git: Result<Cli, String>,
    ) -> Result<Self, IsolateError> {
        Self::assembled(config, git)
    }

    /// The common body: canonicalise every path, then check invariant 4 both ways.
    fn assembled(
        mut config: IsolatorConfig,
        git: Result<Cli, String>,
    ) -> Result<Self, IsolateError> {
        std::fs::create_dir_all(&config.scratch_root)?;
        config.scratch_root = canonical(&config.scratch_root);
        for checkout in config.repos.values_mut() {
            checkout.local_path = canonical(&checkout.local_path);
        }
        for checkout in config.repos.values() {
            if config.scratch_root.starts_with(&checkout.local_path) {
                return Err(IsolateError::Refused(root_inside_repo(
                    &config.scratch_root,
                    &checkout.local_path,
                )));
            }
            if checkout.local_path.starts_with(&config.scratch_root) {
                return Err(IsolateError::Refused(repo_inside_root(
                    &checkout.local_path,
                    &config.scratch_root,
                )));
            }
        }
        Ok(Self {
            config,
            git,
            locks: Mutex::default(),
            held: Mutex::default(),
        })
    }

    /// The located `git`, or the refusal the probe produced (plan D40).
    fn cli(&self) -> Result<&Cli, IsolateError> {
        self.git
            .as_ref()
            .map_err(|reason| IsolateError::Refused(reason.clone()))
    }

    /// The scope as checkouts, in scope order.
    ///
    /// The prologue of every mode (blueprint §6.3): a repository with no checkout on this box and
    /// two repositories with one name are both refused before anything is created.
    fn resolve_scope(&self, scope: &[RepoId]) -> Result<Vec<(RepoId, RepoCheckout)>, IsolateError> {
        let mut resolved = Vec::with_capacity(scope.len());
        let mut names: Vec<&str> = Vec::with_capacity(scope.len());
        for repo in scope {
            let checkout = self
                .config
                .repos
                .get(repo)
                .ok_or_else(|| IsolateError::Refused(no_checkout_for_repo()))?;
            if names.contains(&checkout.name.as_str()) {
                return Err(IsolateError::Refused(duplicate_repo_name(&checkout.name)));
            }
            names.push(&checkout.name);
            resolved.push((*repo, checkout.clone()));
        }
        Ok(resolved)
    }

    /// The checkout a row was made from, by `repo_id`.
    ///
    /// `capture`, `reconcile` and `cleanup` are handed `step_trees` rows in `repo_id` order rather
    /// than the `Prepared` they came from (blueprint H-13), so every per-row action keys on the id.
    fn checkout_of(&self, tree: &RunStepTree) -> Result<&RepoCheckout, IsolateError> {
        self.config
            .repos
            .get(&tree.repo_id)
            .ok_or_else(|| IsolateError::Refused(no_checkout_for_repo()))
    }

    /// `HEAD` of a checkout, with A-4's refusal reworded in the repository's own name.
    ///
    /// [`git::head`] names the path it was given, because it has nothing else; a step's refusal
    /// reads better with `core` in it than with `/home/…/core`.
    async fn head_of(&self, checkout: &RepoCheckout) -> Result<String, IsolateError> {
        let path = checkout.local_path.clone();
        match blocking(move || git::head(&path)).await {
            // `head`'s only refusal is the unborn one.
            Err(IsolateError::Refused(_)) => {
                Err(IsolateError::Refused(git::unborn_head(&checkout.name)))
            }
            other => other,
        }
    }

    /// D43's guard for every repository of the scope, taken **before** the mode's reads (A-1).
    ///
    /// The `std` lock is released before the `tokio` one is awaited: an `.await` under a `std`
    /// guard is what `mem.rs:3-7` forbids, and here it would also deadlock the next `prepare`.
    async fn acquire(&self, run: RunId, step: StepId, checkouts: &[(RepoId, RepoCheckout)]) {
        for (repo, _) in checkouts {
            let lock = {
                let mut locks = self
                    .locks
                    .lock()
                    .expect("no panic holds the isolator's locks");
                Arc::clone(locks.entry((self.config.box_id, *repo)).or_default())
            };
            let guard = lock.lock_owned().await;
            self.held
                .lock()
                .expect("no panic holds the isolator's held guards")
                .entry((run, step))
                .or_default()
                .push(guard);
        }
    }

    /// Drops every guard `step` is holding; a step that holds none is not an error.
    fn release(&self, step: StepId) {
        let mut held = self
            .held
            .lock()
            .expect("no panic holds the isolator's held guards");
        let mut released = Vec::new();
        held.retain(|(_, held_step), guards| {
            if *held_step == step {
                released.append(guards);
                false
            } else {
                true
            }
        });
        drop(held);
        drop(released);
    }

    /// Drops every guard any step of `run` is holding, whether or not a row names that step.
    fn release_run(&self, run: RunId) {
        let mut held = self
            .held
            .lock()
            .expect("no panic holds the isolator's held guards");
        let mut released = Vec::new();
        held.retain(|(held_run, _), guards| {
            if *held_run == run {
                released.append(guards);
                false
            } else {
                true
            }
        });
        drop(held);
        drop(released);
    }

    /// `local` and `shared_serialized`: the checkout itself is the tree (plan D24, D29).
    ///
    /// The two modes differ by exactly one thing at `prepare` — the guard — so they share a body
    /// rather than a copy of it.
    async fn prepare_in_place(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
        mode: Isolation,
    ) -> Result<Prepared, IsolateError> {
        if mode == Isolation::SharedSerialized {
            self.acquire(run, step, checkouts).await;
        }

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            let before = self.head_of(checkout).await?;
            let path = checkout.local_path.clone();
            let dirty = blocking(move || git::is_dirty(&path)).await?;
            trees.push(PreparedTree {
                tree: RunStepTree {
                    run_step_id: step,
                    repo_id: *repo,
                    mode,
                    path: checkout.local_path.to_string_lossy().into_owned(),
                    base_ref: before.clone(),
                    dirty,
                },
                before_hash: before,
            });
        }

        // D28: the session runs in the primary's own checkout, and every other tree of the scope is
        // somewhere else on the box entirely.
        let cwd = match checkouts
            .iter()
            .find(|(_, checkout)| checkout.is_primary)
            .or_else(|| checkouts.first())
        {
            Some((_, checkout)) => checkout.local_path.clone(),
            None => {
                let cwd = session_dir(&self.config.scratch_root, run, step);
                std::fs::create_dir_all(&cwd)?;
                cwd
            }
        };
        let extra_dirs = trees
            .iter()
            .map(|prepared| PathBuf::from(&prepared.tree.path))
            .filter(|path| !path.starts_with(&cwd))
            .collect();
        Ok(Prepared {
            trees,
            cwd,
            extra_dirs,
        })
    }

    /// `worktree` (plan D23, D38): one locked linked worktree per repository under
    /// `<root>/<run>/<step>/`, each on its own `htui/<step>` branch at the source's `HEAD`.
    async fn prepare_worktree(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
    ) -> Result<Prepared, IsolateError> {
        let git = self.cli()?.clone();
        let cwd = session_dir(&self.config.scratch_root, run, step);
        std::fs::create_dir_all(&cwd)?;
        let branch = format!("htui/{step}");

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            let local = checkout.local_path.clone();
            if blocking(move || git::has_submodules(&local)).await? {
                return Err(IsolateError::Refused(submodules_refused()));
            }
            let source_head = self.head_of(checkout).await?;
            let path = cwd.join(&checkout.name);
            let before = self
                .worktree_at(&git, checkout, &path, &branch, &source_head, step, run)
                .await?;
            trees.push(PreparedTree {
                tree: RunStepTree {
                    run_step_id: step,
                    repo_id: *repo,
                    mode: Isolation::Worktree,
                    path: path.to_string_lossy().into_owned(),
                    base_ref: before.clone(),
                    dirty: false,
                },
                before_hash: before,
            });
        }
        // D28: every tree of this mode is under one common parent, so the session needs no
        // `extra_dirs` to reach any of them.
        Ok(Prepared {
            trees,
            cwd,
            extra_dirs: Vec::new(),
        })
    }

    /// The tree at `path`, made or found, and the `before_hash` that goes with it.
    ///
    /// D38's idempotence, and blueprint H-1's crash window with it: a second `prepare` for the same
    /// `(run, step)` finds the tree it made, and the branch — not the source's `HEAD`, which may
    /// have moved in between — is what says where the step started.
    ///
    /// H-5's half-made tree is **repaired** with D47's `reset --hard` rather than removed and
    /// re-added, which is a departure from blueprint §6.3 with a reason: `git worktree add -b`
    /// refuses a branch that already exists (`git.rs`'s own
    /// `add_worktree_on_an_existing_branch_is_a_git_error_naming_the_branch` pins that), and the
    /// remove-then-add path leaves exactly that branch behind. A tree whose `HEAD` cannot even be
    /// read — a `.git` file pointing at a `worktrees/<id>` entry somebody pruned — is not a
    /// repository any more, so `reset --hard` inside it could only fail, forever: it is removed and
    /// re-added onto the existing branch **without** `-b` ([`Cli::add_worktree_on_branch`]). The
    /// remove-and-re-add with `-b` survives for the one case where `-b` is legal: a tree with no
    /// branch at all.
    #[expect(
        clippy::too_many_arguments,
        reason = "every one of them is a fact of the tree being made; a struct for them would be \
                  named once and read once"
    )]
    async fn worktree_at(
        &self,
        git: &Cli,
        checkout: &RepoCheckout,
        path: &Path,
        branch: &str,
        source_head: &str,
        step: StepId,
        run: RunId,
    ) -> Result<String, IsolateError> {
        let local = checkout.local_path.clone();
        if path.join(".git").exists() {
            let (repo, name) = (local.clone(), branch.to_owned());
            match blocking(move || git::branch_target(&repo, &name)).await? {
                Some(base) => {
                    let tree = path.to_path_buf();
                    let Ok(head) = blocking(move || git::head(&tree)).await else {
                        git::with_retry("worktree remove", || git.remove_worktree(&local, path))
                            .await?;
                        remove_directory(path)?;
                        git::with_retry("worktree add", || {
                            git.add_worktree_on_branch(&local, path, step, run, &base)
                        })
                        .await?;
                        return Ok(base);
                    };
                    let at_base = head == base;
                    let tree = path.to_path_buf();
                    let clean = at_base
                        && matches!(blocking(move || git::is_dirty(&tree)).await, Ok(false));
                    if !clean {
                        git::with_retry("reset --hard", || git.reset_hard(path, &base)).await?;
                    }
                    return Ok(base);
                }
                None => {
                    git::with_retry("worktree remove", || git.remove_worktree(&local, path))
                        .await?;
                    remove_directory(path)?;
                }
            }
        }
        git::with_retry("worktree add", || {
            git.add_worktree(&local, path, step, run, source_head)
        })
        .await?;
        Ok(source_head.to_owned())
    }

    /// `copy` (plan D35, D47; blueprint A-2): a whole filesystem copy of the checkout per
    /// repository, reset to the source's `HEAD` and labelled with the base it was taken from.
    async fn prepare_copy(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
    ) -> Result<Prepared, IsolateError> {
        let git = self.cli()?.clone();
        let cwd = session_dir(&self.config.scratch_root, run, step);
        std::fs::create_dir_all(&cwd)?;
        let branch = format!("htui/{step}");
        let excludes = copy::excludes(&self.config.copy_exclude);

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            // Blueprint §6.3's order: D35's two `.git` refusals answer before anything is
            // measured and before the reuse branch is even consulted.
            let source = checkout.local_path.clone();
            blocking(move || copy::check_source(&source)).await?;
            let source_head = self.head_of(checkout).await?;
            let path = cwd.join(&checkout.name);
            let before = self
                .copy_at(&git, checkout, &path, &branch, &source_head, &excludes)
                .await?;
            trees.push(PreparedTree {
                tree: RunStepTree {
                    run_step_id: step,
                    repo_id: *repo,
                    mode: Isolation::Copy,
                    path: path.to_string_lossy().into_owned(),
                    base_ref: before.clone(),
                    dirty: false,
                },
                before_hash: before,
            });
        }
        Ok(Prepared {
            trees,
            cwd,
            extra_dirs: Vec::new(),
        })
    }

    /// The copy at `path`, made or found, and the `before_hash` that goes with it.
    ///
    /// D38's reuse reads the `htui/<step>` label **inside the copy** (blueprint A-2): `copy` never
    /// writes a ref into the source, so the label in the scratch tree is the only durable record of
    /// the base once the agent has committed over the copy's `HEAD`.
    ///
    /// The order of the last four steps is this module's, and it is load-bearing (T4's third
    /// finding): the copy is built as `<name>.partial`, then reset, then labelled, and only then
    /// renamed. An interruption anywhere before the rename leaves nothing the reuse branch above
    /// will look at, so the next `prepare` starts the copy again rather than handing an agent a
    /// tree that was never reset.
    async fn copy_at(
        &self,
        git: &Cli,
        checkout: &RepoCheckout,
        path: &Path,
        branch: &str,
        source_head: &str,
        excludes: &[copy::Exclude],
    ) -> Result<String, IsolateError> {
        if path.exists() {
            let (tree, name) = (path.to_path_buf(), branch.to_owned());
            if path.join(".git").is_dir()
                && let Some(base) = blocking(move || git::branch_target(&tree, &name)).await?
            {
                return Ok(base);
            }
            // Anything else standing where the copy goes was not finished by us — the rename is
            // what makes a copy visible, and it runs after the label.
            remove_directory(path)?;
        }

        let (source, owned) = (checkout.local_path.clone(), excludes.to_vec());
        let cap = self.config.copy_max_total_bytes;
        blocking(move || copy::measure_within_cap(&source, &owned, cap)).await?;

        let (source, owned, destination) = (
            checkout.local_path.clone(),
            excludes.to_vec(),
            path.to_path_buf(),
        );
        let partial =
            blocking(move || copy::copy_into_partial(&source, &destination, &owned)).await?;

        // D47: the reset is `git reset --hard` and nothing is composed out of `gix` primitives.
        git::with_retry("reset --hard", || git.reset_hard(&partial, source_head)).await?;
        let (tree, name, target) = (partial.clone(), branch.to_owned(), source_head.to_owned());
        git::with_retry("create branch", move || {
            let (tree, name, target) = (tree.clone(), name.clone(), target.clone());
            async move { blocking(move || git::create_branch(&tree, &name, &target)).await }
        })
        .await?;

        let destination = path.to_path_buf();
        blocking(move || copy::finish_partial(&destination)).await?;
        Ok(source_head.to_owned())
    }

    /// `capture` for one row, without the guard release the caller owns.
    async fn capture_rows(
        &self,
        step: StepId,
        trees: &[RunStepTree],
    ) -> Result<Vec<RunStepCommit>, IsolateError> {
        let mut commits = Vec::with_capacity(trees.len());
        for tree in trees {
            let checkout = self.checkout_of(tree)?.clone();
            let after = match tree.mode {
                Isolation::Local | Isolation::SharedSerialized => {
                    self.capture_in_place(step, tree, &checkout).await?
                }
                Isolation::Worktree => self.capture_worktree(step, tree, &checkout).await?,
                Isolation::Copy => self.capture_copy(tree).await?,
            };
            commits.push(RunStepCommit {
                run_step_id: step,
                repo_id: tree.repo_id,
                before_hash: tree.base_ref.clone(),
                after_hash: after,
            });
        }
        Ok(commits)
    }

    /// `capture` for a tree that is the checkout itself: `HEAD`, and D26's label for the serialised
    /// mode.
    async fn capture_in_place(
        &self,
        step: StepId,
        tree: &RunStepTree,
        checkout: &RepoCheckout,
    ) -> Result<Option<String>, IsolateError> {
        let after = self.head_of(checkout).await?;
        if after == tree.base_ref {
            return Ok(None);
        }
        if tree.mode == Isolation::SharedSerialized {
            // D26: the label is the only durable record that this step is what moved the shared
            // checkout. `PreviousValue::MustNotExist` tolerates a label already at exactly this
            // target and refuses only a move, which is what makes a `capture` that runs twice safe.
            let path = checkout.local_path.clone();
            let name = format!("htui/{step}");
            let target = after.clone();
            git::with_retry("create branch", move || {
                let (path, name, target) = (path.clone(), name.clone(), target.clone());
                async move { blocking(move || git::create_branch(&path, &name, &target)).await }
            })
            .await?;
        }
        Ok(Some(after))
    }

    /// `capture` for a linked worktree: its `HEAD`, and D27's removal of the tree that earned
    /// nothing.
    ///
    /// A tree that is no longer there is blueprint H-4: an earlier `capture` removed it and the
    /// crash happened before `record_commits`. The branch survives every removal, so it — not the
    /// vanished tree — is what answers the second time.
    async fn capture_worktree(
        &self,
        step: StepId,
        tree: &RunStepTree,
        checkout: &RepoCheckout,
    ) -> Result<Option<String>, IsolateError> {
        let path = PathBuf::from(&tree.path);
        if !path.join(".git").exists() {
            let (repo, name) = (checkout.local_path.clone(), format!("htui/{step}"));
            let label = blocking(move || git::branch_target(&repo, &name)).await?;
            return Ok(label.filter(|target| *target != tree.base_ref));
        }

        let read = path.clone();
        let after = blocking(move || git::head(&read)).await?;
        if after != tree.base_ref {
            return Ok(Some(after));
        }

        // D27: a clean tree the step committed nothing to is litter, and the run may hold dozens
        // of them. A dirty one is the agent's unfinished work and stays for milestone 5's sweep.
        // "Clean" here is stricter than D24's: an untracked file is work too, and the double
        // `--force` below would delete it for good (ANA-2 `:2065`, risk 2).
        let read = path.clone();
        let litter =
            blocking(move || Ok(!git::is_dirty(&read)? && !git::has_untracked_files(&read)?))
                .await?;
        if litter {
            let git = self.cli()?.clone();
            let local = checkout.local_path.clone();
            git::with_retry("worktree remove", || git.remove_worktree(&local, &path)).await?;
        }
        Ok(None)
    }

    /// `capture` for a copy: its `HEAD`, and nothing else.
    ///
    /// Cleanup is run-terminal, so a copy is never removed at capture (D27 is the `worktree`
    /// mode's rule alone) and a copy that is gone is a tree somebody else deleted — blueprint H-4's
    /// refusal, because there is no branch in the source to read the answer from.
    async fn capture_copy(&self, tree: &RunStepTree) -> Result<Option<String>, IsolateError> {
        let path = PathBuf::from(&tree.path);
        if !path.join(".git").is_dir() {
            return Err(IsolateError::Refused(copy_tree_vanished(&path)));
        }
        let after = blocking(move || git::head(&path)).await?;
        Ok((after != tree.base_ref).then_some(after))
    }

    /// `git worktree remove --force --force` for one row, through D39's retry.
    async fn remove_worktree_of(&self, tree: &RunStepTree) -> Result<(), IsolateError> {
        let local = self.checkout_of(tree)?.local_path.clone();
        let git = self.cli()?.clone();
        let path = PathBuf::from(&tree.path);
        git::with_retry("worktree remove", || git.remove_worktree(&local, &path)).await
    }

    /// D46's report: every entry a `worktrees()` read still lists under `run_dir`, as errors.
    async fn stale_entries(&self, repo: RepoId, run_dir: &Path) -> Vec<IsolateError> {
        let Some(checkout) = self.config.repos.get(&repo) else {
            return vec![IsolateError::Refused(no_checkout_for_repo())];
        };
        let (local, root) = (checkout.local_path.clone(), run_dir.to_path_buf());
        match blocking(move || git::worktrees_under(&local, &root)).await {
            Ok(paths) => paths
                .iter()
                .map(|path| IsolateError::Git(stale_worktree(path)))
                .collect(),
            Err(err) => vec![err],
        }
    }

    /// `reconcile` for a tree that is *not* the checkout: D25's `--no-ff` merge of the winner into
    /// the primary tree.
    ///
    /// The winner's tip is read from the branch for a `worktree` (the tree itself may be gone —
    /// D27 removed it, or milestone 5 will) and from the copy's own `HEAD` for a `copy`, whose
    /// label never moved past the base (blueprint A-2).
    ///
    /// Three refusals stand between the tip and the merge: no usable `git`, a dirty primary
    /// (ANA-2 `:978-979`), and a primary that moved. The moved case is read twice before it is
    /// refused, because blueprint H-3's crash between the merge and `record_commits` leaves a
    /// primary whose `HEAD` is exactly the merge this call would make — its parents say so, and
    /// the answer is that commit rather than a refusal.
    async fn reconcile_isolated(
        &self,
        step: StepId,
        tree: &RunStepTree,
        checkout: &RepoCheckout,
    ) -> Result<Option<String>, IsolateError> {
        let local = checkout.local_path.clone();
        let tip = match tree.mode {
            Isolation::Copy => {
                let path = PathBuf::from(&tree.path);
                if !path.join(".git").is_dir() {
                    return Err(IsolateError::Refused(copy_tree_vanished(&path)));
                }
                Some(blocking(move || git::head(&path)).await?)
            }
            _ => {
                let (repo, name) = (local.clone(), format!("htui/{step}"));
                blocking(move || git::branch_target(&repo, &name)).await?
            }
        };
        let Some(after) = tip.filter(|tip| *tip != tree.base_ref) else {
            return Ok(None);
        };

        let git = self.cli()?.clone();
        let read = local.clone();
        if blocking(move || git::is_dirty(&read)).await? {
            return Err(IsolateError::Refused(dirty_primary_tree()));
        }
        let read = local.clone();
        let head = blocking(move || git::head(&read)).await?;
        if head != tree.base_ref {
            let read = local.clone();
            let parents = blocking(move || git::head_parents(&read)).await?;
            if parents == [tree.base_ref.as_str(), after.as_str()] {
                return Ok(Some(head));
            }
            return Err(IsolateError::Refused(primary_moved(&head)));
        }

        if tree.mode == Isolation::Copy {
            // OQ-5: a copy has its own object database, so `<after>` does not resolve in the
            // primary until the range has been written there.
            let (from, to) = (PathBuf::from(&tree.path), local.clone());
            let (base, tip) = (tree.base_ref.clone(), after.clone());
            git::with_retry("copy objects", move || {
                let (from, to, base, tip) = (from.clone(), to.clone(), base.clone(), tip.clone());
                async move { blocking(move || git::copy_range(&from, &to, &base, &tip)).await }
            })
            .await?;
        }

        let merged = git::with_retry("merge", || {
            git.merge_no_ff(&local, step, &tree.base_ref, &after)
        })
        .await?;
        Ok(Some(merged.commit))
    }

    /// `reconcile` for a tree that is the checkout itself: nothing to merge, because the step
    /// committed into the primary tree as it went.
    async fn reconcile_in_place(
        &self,
        step: StepId,
        tree: &RunStepTree,
        checkout: &RepoCheckout,
    ) -> Result<Option<String>, IsolateError> {
        let head = self.head_of(checkout).await?;
        if tree.mode == Isolation::Local {
            return Ok((head != tree.base_ref).then_some(head));
        }

        // `shared_serialized`: the label D26 wrote at capture is what says the checkout is still
        // where this step left it. Anything else moved it, and this milestone has no verb that
        // could put it back.
        let path = checkout.local_path.clone();
        let name = format!("htui/{step}");
        let label = blocking(move || git::branch_target(&path, &name)).await?;
        match label {
            Some(target) if target == head => Ok((head != tree.base_ref).then_some(head)),
            None if head == tree.base_ref => Ok(None),
            _ => Err(IsolateError::Refused(primary_moved(&head))),
        }
    }
}

/// The primary checkout is not where `reconcile` left it, so there is nothing safe to merge into.
#[must_use]
pub fn primary_moved(head: &str) -> String {
    format!("primary_moved: {head}")
}

/// ANA-2 `:978-979`: the primary tree carries uncommitted work, so a merge into it would mix the
/// step's changes with somebody else's.
#[must_use]
pub fn dirty_primary_tree() -> String {
    "dirty_primary_tree".to_owned()
}

/// `remove_dir_all` where an absent directory is the outcome asked for, not a failure.
fn remove_directory(path: &Path) -> Result<(), IsolateError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(IsolateError::Io(err)),
    }
}

impl Isolator for GixIsolator {
    fn prepare<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        scope: &'a [RepoId],
        isolation: Isolation,
    ) -> IsolatorFuture<'a, Prepared> {
        Box::pin(async move {
            let checkouts = self.resolve_scope(scope)?;
            match isolation {
                Isolation::Local | Isolation::SharedSerialized => {
                    let prepared = self
                        .prepare_in_place(run, step, &checkouts, isolation)
                        .await;
                    if prepared.is_err() {
                        // A refusal after the guard was taken has no `run_step_tree` row to its
                        // name, so no `cleanup` would ever be handed it (blueprint A-1 releases
                        // by row): the guard goes back here or it never goes back at all.
                        self.release(step);
                    }
                    prepared
                }
                // D40: the two modes that shell out answer with the probe's own sentence before
                // they touch a filesystem.
                Isolation::Worktree => self.prepare_worktree(run, step, &checkouts).await,
                Isolation::Copy => self.prepare_copy(run, step, &checkouts).await,
            }
        })
    }

    /// Blueprint A-1: this is where a `shared_serialized` step's guard is released — at the
    /// `capture` of the step that took it, not at the run's `cleanup`.
    ///
    /// Released on the error path too: a step whose capture failed is over, and the next step of
    /// the same run must not wait for a `cleanup` that only runs when the whole run is terminal.
    fn capture<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move {
            let captured = self.capture_rows(step, trees).await;
            self.release(step);
            captured
        })
    }

    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move {
            let mut commits = Vec::with_capacity(trees.len());
            for tree in trees {
                let checkout = self.checkout_of(tree)?.clone();
                let after = match tree.mode {
                    Isolation::Local | Isolation::SharedSerialized => {
                        self.reconcile_in_place(winner, tree, &checkout).await?
                    }
                    Isolation::Worktree | Isolation::Copy => {
                        self.reconcile_isolated(winner, tree, &checkout).await?
                    }
                };
                commits.push(RunStepCommit {
                    run_step_id: winner,
                    repo_id: tree.repo_id,
                    before_hash: tree.base_ref.clone(),
                    after_hash: after,
                });
            }
            Ok(commits)
        })
    }

    /// D36: run-terminal, never at step end, and every row is processed before the first failure
    /// is returned — a tree left behind because another one refused to go is a tree nobody
    /// removes, since milestone 5's sweep is the only retry there is.
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            let mut failures: Vec<IsolateError> = Vec::new();
            let mut worktree_repos: Vec<RepoId> = Vec::new();

            for tree in trees {
                match tree.mode {
                    Isolation::Worktree => {
                        if !worktree_repos.contains(&tree.repo_id) {
                            worktree_repos.push(tree.repo_id);
                        }
                        if let Err(err) = self.remove_worktree_of(tree).await {
                            failures.push(err);
                        }
                    }
                    // The copy is a plain directory under the scratch root; `git` knows nothing
                    // about it, so there is nothing to tell it.
                    Isolation::Copy => {
                        if let Err(err) = remove_directory(Path::new(&tree.path)) {
                            failures.push(err);
                        }
                    }
                    // D43's guards are released for the whole run below, rows or not.
                    // Nothing was created, so nothing is removed.
                    Isolation::SharedSerialized | Isolation::Local => {}
                }
            }

            // D43: whatever a step's own `capture` never released (blueprint H-15) — including
            // the guard of a step whose `run_step_tree` row was never written, which no loop over
            // the rows could reach.
            self.release_run(run);

            // F-Q: `prepare` made `<root>/<run>/<step>/` and no per-tree verb names it.
            let run_dir = self.config.scratch_root.join(run.to_string());
            if let Err(err) = remove_directory(&run_dir) {
                failures.push(err);
            }

            // D46, last: `git worktree prune` is never spawned, so an entry that survived its
            // `remove` is reported here and left exactly where it is.
            for repo in worktree_repos {
                failures.extend(self.stale_entries(repo, &run_dir).await);
            }

            let mut failures = failures.into_iter();
            let Some(first) = failures.next() else {
                return Ok(());
            };
            for rest in failures {
                tracing::warn!(%run, err = %rest, "a further cleanup failure of the same run");
            }
            Err(first)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use htui_core::model::{BoxId, Isolation, RepoId, RunId, RunStepTree, StepId};

    use crate::isolate::git::testkit::{commit_file, empty_repo, repo_with_one_commit};
    use crate::isolate::{Isolator as _, Prepared};

    use super::{GixIsolator, IsolatorConfig, RepoCheckout};

    /// A repository at `<dir>/<name>` with one commit, and the pieces a config map wants.
    fn repo(dir: &Path, name: &str, is_primary: bool) -> (RepoId, RepoCheckout, String) {
        let local_path = dir.join(name);
        std::fs::create_dir_all(&local_path).expect("the repository directory is made");
        let head = repo_with_one_commit(&local_path);
        (
            RepoId::new(),
            RepoCheckout {
                name: name.to_owned(),
                local_path,
                is_primary,
            },
            head,
        )
    }

    /// The `run_step_tree` rows `capture`, `reconcile` and `cleanup` are handed, in `repo_id`
    /// order — what `step_trees` answers, never the `Prepared`'s own scope order (blueprint H-13).
    fn rows(prepared: &Prepared) -> Vec<RunStepTree> {
        let mut rows: Vec<RunStepTree> = prepared
            .trees
            .iter()
            .map(|prepared| prepared.tree.clone())
            .collect();
        rows.sort_by_key(|row| row.repo_id);
        rows
    }

    /// A config over `checkouts` with a scratch root at `root` and a cap nothing reaches.
    fn config(root: &Path, checkouts: &[(RepoId, RepoCheckout)]) -> IsolatorConfig {
        IsolatorConfig {
            repos: checkouts.iter().cloned().collect::<BTreeMap<_, _>>(),
            scratch_root: root.to_path_buf(),
            copy_exclude: Vec::new(),
            copy_max_total_bytes: 1 << 30,
            box_id: BoxId::new(),
        }
    }

    /// Invariant 4, the first half: a scratch root under a managed checkout would put every tree
    /// inside the repository the trees are isolating from.
    #[test]
    fn a_scratch_root_inside_a_repo_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, _) = repo(dir.path(), "core", true);
        let root = checkout.local_path.join("trees");

        let err = GixIsolator::new(config(&root, &[(id, checkout)]))
            .expect_err("a root inside a repository is refused");
        let text = err.to_string();
        assert!(
            text.starts_with("isolation refused: scratch root ")
                && text.contains("is inside a managed repository"),
            "{text}"
        );
    }

    /// Invariant 4, the other half: a checkout under the scratch root would be removed by the
    /// run-terminal `remove_dir_all` of its own parent.
    #[test]
    fn a_repo_inside_the_scratch_root_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("trees");
        std::fs::create_dir_all(&root).expect("the scratch root is made");
        let (id, checkout, _) = repo(&root, "core", true);

        let err = GixIsolator::new(config(&root, &[(id, checkout)]))
            .expect_err("a repository inside the root is refused");
        let text = err.to_string();
        assert!(
            text.starts_with("isolation refused: managed repository ")
                && text.contains("is inside the scratch root"),
            "{text}"
        );
    }

    /// The sentence `engine.rs:2606` already pins, produced by the real isolator now.
    #[tokio::test]
    async fn a_repo_missing_from_the_map_is_refused_with_the_pinned_sentence() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, _) = repo(dir.path(), "core", true);
        let isolator = GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
            .expect("the config validates");

        let err = isolator
            .prepare(
                RunId::new(),
                StepId::new(),
                &[RepoId::new()],
                Isolation::Local,
            )
            .await
            .expect_err("a repo with no checkout is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: no checkout for this repo on this box"
        );
    }

    /// Two repositories of one project whose directory names collide would be one directory under
    /// `<root>/<run>/<step>/`, so the collision is refused before any tree is made.
    #[tokio::test]
    async fn two_repos_sharing_a_name_are_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (one, first, _) = repo(&dir.path().join("a"), "core", true);
        let (two, mut second, _) = repo(&dir.path().join("b"), "core", false);
        second.name = "core".to_owned();
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(one, first), (two, second)],
        ))
        .expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[one, two], Isolation::Local)
            .await
            .expect_err("the name collision is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: duplicate repo name core"
        );
    }

    /// Criterion 12's record half (D24): `local` works in the checkout itself, reports its `HEAD`
    /// as `before_hash` and records the dirtiness rather than acting on it.
    #[tokio::test]
    async fn local_records_dirty_true_and_head_as_before_hash() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, head) = repo(dir.path(), "core", true);
        std::fs::write(checkout.local_path.join("f"), "edited\n").expect("the file is edited");
        let local_path = checkout.local_path.clone();
        let isolator = GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
            .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[id], Isolation::Local)
            .await
            .expect("local prepares");

        assert_eq!(prepared.trees.len(), 1);
        let row = &prepared.trees[0];
        assert_eq!(row.before_hash, head);
        assert_eq!(row.tree.base_ref, head);
        assert_eq!(row.tree.mode, Isolation::Local);
        assert_eq!(row.tree.run_step_id, step);
        assert!(
            row.tree.dirty,
            "a modified tracked file is recorded, not reset"
        );
        assert_eq!(
            PathBuf::from(&row.tree.path),
            std::fs::canonicalize(&local_path).expect("the checkout canonicalises"),
            "the tree is the checkout itself"
        );
    }

    /// D28: a `local` step runs in the primary's own checkout, and the other repository of its
    /// scope is somewhere else on the box entirely — which is what `extra_dirs` is for.
    #[tokio::test]
    async fn local_cwd_is_the_primary_and_the_other_repo_is_an_extra_dir() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let core_path = core_checkout.local_path.clone();
        let docs_path = docs_checkout.local_path.clone();
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        // Scope order names `docs` first; the primary is still the `cwd` (blueprint H-13).
        let prepared = isolator
            .prepare(RunId::new(), StepId::new(), &[docs, core], Isolation::Local)
            .await
            .expect("local prepares");

        assert_eq!(
            prepared
                .trees
                .iter()
                .map(|t| t.tree.repo_id)
                .collect::<Vec<_>>(),
            vec![docs, core],
            "the rows are in scope order"
        );
        assert_eq!(
            prepared.cwd,
            std::fs::canonicalize(&core_path).expect("the checkout canonicalises")
        );
        assert_eq!(
            prepared.extra_dirs,
            vec![std::fs::canonicalize(&docs_path).expect("the checkout canonicalises")]
        );
    }

    /// D26 and D43 read through blueprint A-1: the `(box, repo)` guard is taken at `prepare` and
    /// released at the `capture` of the *same* step, and the label names what the step committed.
    #[tokio::test]
    async fn shared_serialized_labels_after_hash_and_holds_the_lock_until_capture() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, head) = repo(dir.path(), "core", true);
        let local_path = checkout.local_path.clone();
        let isolator = Arc::new(
            GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
                .expect("the config validates"),
        );

        let run = RunId::new();
        let first = StepId::new();
        let prepared = isolator
            .prepare(run, first, &[id], Isolation::SharedSerialized)
            .await
            .expect("the first step prepares");
        assert_eq!(prepared.trees[0].before_hash, head);

        let waiting = {
            let isolator = Arc::clone(&isolator);
            let scope = vec![id];
            tokio::spawn(async move {
                isolator
                    .prepare(run, StepId::new(), &scope, Isolation::SharedSerialized)
                    .await
                    .map(|prepared| prepared.trees.len())
            })
        };
        let mut waiting = waiting;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiting)
                .await
                .is_err(),
            "a second step on the same (box, repo) waits"
        );

        let after = commit_file(&local_path, "g", "second\n", "two");
        let rows = prepared
            .trees
            .iter()
            .map(|prepared| prepared.tree.clone())
            .collect::<Vec<_>>();
        let commits = isolator
            .capture(first, &rows)
            .await
            .expect("the first step captures");
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].before_hash, head);
        assert_eq!(commits[0].after_hash.as_deref(), Some(&*after));
        assert_eq!(
            crate::isolate::git::branch_target(&local_path, &format!("htui/{first}"))
                .expect("the ref store reads"),
            Some(after),
            "D26's label names the step's own tip"
        );

        let admitted = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the second step is admitted at the first one's capture")
            .expect("the task did not panic")
            .expect("the second step prepares");
        assert_eq!(admitted, 1);
    }

    /// A `prepare` that refuses **after** taking D43's guard has no `run_step_tree` row to its
    /// name, so no `cleanup` would ever be handed it: the guard has to go back where it was taken.
    #[tokio::test]
    async fn a_refused_shared_serialized_prepare_releases_what_it_took() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let unborn_path = dir.path().join("unborn");
        std::fs::create_dir_all(&unborn_path).expect("the repository directory is made");
        empty_repo(&unborn_path);
        let unborn = RepoId::new();
        let unborn_checkout = RepoCheckout {
            name: "unborn".to_owned(),
            local_path: unborn_path,
            is_primary: false,
        };
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (unborn, unborn_checkout)],
        ))
        .expect("the config validates");

        let err = isolator
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core, unborn],
                Isolation::SharedSerialized,
            )
            .await
            .expect_err("the unborn repository is refused");
        assert!(err.to_string().contains("unborn HEAD"), "{err}");

        tokio::time::timeout(
            Duration::from_secs(5),
            isolator.prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::SharedSerialized,
            ),
        )
        .await
        .expect("the refused step holds nothing")
        .expect("the next step prepares");
    }

    /// Blueprint H-15: a step killed between `prepare` and `capture` leaves its guard behind, and
    /// the run-terminal `cleanup` is what frees it.
    #[tokio::test]
    async fn cleanup_releases_a_lock_a_crashed_step_never_captured() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, _) = repo(dir.path(), "core", true);
        let isolator = Arc::new(
            GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
                .expect("the config validates"),
        );

        let run = RunId::new();
        let crashed = StepId::new();
        let prepared = isolator
            .prepare(run, crashed, &[id], Isolation::SharedSerialized)
            .await
            .expect("the crashed step prepared");
        let rows = prepared
            .trees
            .iter()
            .map(|prepared| prepared.tree.clone())
            .collect::<Vec<_>>();

        isolator
            .cleanup(run, &rows)
            .await
            .expect("the run cleans up");

        tokio::time::timeout(
            Duration::from_secs(5),
            isolator.prepare(run, StepId::new(), &[id], Isolation::SharedSerialized),
        )
        .await
        .expect("the guard was released by cleanup")
        .expect("the next step prepares");
    }

    /// The engine's `upsert_step_tree` can fail after `prepare` took the guard, and then the run's
    /// `cleanup` is handed no row for that step at all. Releasing by row alone would leak the guard
    /// until restart, so `cleanup(run, _)` drops every guard of the run whatever the rows say.
    #[tokio::test]
    async fn cleanup_releases_a_guard_whose_step_has_no_row() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, _) = repo(dir.path(), "core", true);
        let isolator = GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
            .expect("the config validates");

        let run = RunId::new();
        isolator
            .prepare(run, StepId::new(), &[id], Isolation::SharedSerialized)
            .await
            .expect("the step prepared, and its row was never written");

        isolator
            .cleanup(run, &[])
            .await
            .expect("the run cleans up with no rows");

        tokio::time::timeout(
            Duration::from_secs(5),
            isolator.prepare(
                RunId::new(),
                StepId::new(),
                &[id],
                Isolation::SharedSerialized,
            ),
        )
        .await
        .expect("the run's cleanup released the rowless guard")
        .expect("the next step prepares");
    }

    /// Blueprint A-4: `run_step_commit.before_hash` is `NOT NULL`, so a repository with no commit
    /// has no row to write and every mode refuses it by name.
    #[tokio::test]
    async fn an_unborn_repo_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let local_path = dir.path().join("core");
        std::fs::create_dir_all(&local_path).expect("the repository directory is made");
        empty_repo(&local_path);
        let id = RepoId::new();
        let checkout = RepoCheckout {
            name: "core".to_owned(),
            local_path,
            is_primary: true,
        };
        let isolator = GixIsolator::new(config(&dir.path().join("trees"), &[(id, checkout)]))
            .expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[id], Isolation::Local)
            .await
            .expect_err("an unborn HEAD is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: unborn HEAD: core has no commit to record as before_hash",
            "the refusal names the repo, not the path"
        );
    }

    /// D40: a box with no usable `git` still runs `local` steps, and the two modes that shell out
    /// refuse with the sentence the probe produced — never with a panic and never with a tree.
    #[tokio::test]
    async fn worktree_prepare_is_refused_without_git() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (id, checkout, _) = repo(dir.path(), "core", true);
        let isolator = GixIsolator::with_git(
            config(&dir.path().join("trees"), &[(id, checkout)]),
            Err(crate::isolate::git::not_on_path()),
        )
        .expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[id], Isolation::Worktree)
            .await
            .expect_err("the worktree mode needs git");
        assert_eq!(err.to_string(), "isolation refused: git not on PATH");

        isolator
            .prepare(RunId::new(), StepId::new(), &[id], Isolation::Local)
            .await
            .expect("local needs no git at all");
    }

    /// The demo fixture's own shape: a project with no repository still runs sessions, so an empty
    /// scope yields no trees and a `cwd` the session can start in.
    #[tokio::test]
    async fn an_empty_scope_prepares_a_cwd_and_no_trees() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("trees");
        let isolator = GixIsolator::new(config(&root, &[])).expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(run, step, &[], Isolation::Local)
            .await
            .expect("an empty scope prepares");

        assert!(prepared.trees.is_empty());
        assert!(prepared.extra_dirs.is_empty());
        assert!(prepared.cwd.is_dir(), "the session directory exists");
        assert!(
            prepared.cwd.ends_with(format!("{run}/{step}")),
            "{}",
            prepared.cwd.display()
        );
    }

    /// Criterion 11's isolator half (`docs/ANA-2.md:2115`): one tree per repository, all of them
    /// under the scratch root and none of them inside any managed checkout, both branches at the
    /// base, and `git` itself agreeing that the entries are locked.
    #[tokio::test]
    async fn worktree_prepare_writes_one_tree_per_repo_outside_every_repo() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, core_head) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, docs_head) = repo(dir.path(), "docs", false);
        let core_path = core_checkout.local_path.clone();
        let docs_path = docs_checkout.local_path.clone();
        let root = dir.path().join("trees");
        let isolator = GixIsolator::new(config(
            &root,
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(run, step, &[core, docs], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");

        assert_eq!(prepared.trees.len(), 2);
        assert_eq!(
            prepared.cwd,
            std::fs::canonicalize(&root)
                .expect("the root canonicalises")
                .join(run.to_string())
                .join(step.to_string())
        );
        assert!(
            prepared.extra_dirs.is_empty(),
            "every tree is under the session directory"
        );

        for (row, (source, head)) in prepared
            .trees
            .iter()
            .zip([(&core_path, &core_head), (&docs_path, &docs_head)])
        {
            let path = PathBuf::from(&row.tree.path);
            assert_eq!(&row.before_hash, head);
            assert_eq!(&row.tree.base_ref, head);
            assert_eq!(row.tree.mode, Isolation::Worktree);
            assert!(!row.tree.dirty, "a fresh worktree is clean");
            assert!(path.starts_with(&prepared.cwd), "{}", path.display());
            assert!(!path.starts_with(source), "{}", path.display());
            assert_eq!(
                crate::isolate::git::head(&path).expect("the tree has a HEAD"),
                **head
            );
            assert_eq!(
                crate::isolate::git::branch_target(source, &format!("htui/{step}"))
                    .expect("the ref store reads"),
                Some((*head).clone()),
                "D23's -b branched the step from the base"
            );
            let listed = crate::isolate::git::testkit::worktree_list(&git, source).await;
            let ours = listed
                .iter()
                .find(|entry| entry.path == path)
                .expect("git worktree list names the tree");
            assert_eq!(ours.locked.as_deref(), Some(&*format!("htui run {run}")));
            assert_eq!(
                ours.branch.as_deref(),
                Some(&*format!("refs/heads/htui/{step}"))
            );
        }
    }

    /// Stage 5's own question: what did the step commit? The tree it committed in answers
    /// `Some(head)`, the one it did not answers `None`.
    #[tokio::test]
    async fn worktree_capture_reports_the_new_head_or_none() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, core_head) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, docs_head) = repo(dir.path(), "docs", false);
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        // The agent commits in `core`'s tree and leaves `docs`'s alone, but dirties it so D27
        // does not remove it under this test's feet.
        let core_tree = PathBuf::from(&prepared.trees[0].tree.path);
        let docs_tree = PathBuf::from(&prepared.trees[1].tree.path);
        let after = commit_file(&core_tree, "g", "second\n", "two");
        std::fs::write(docs_tree.join("f"), "edited\n").expect("the file is edited");

        let commits = isolator
            .capture(step, &rows(&prepared))
            .await
            .expect("the step captures");

        let core_row = commits
            .iter()
            .find(|commit| commit.repo_id == core)
            .expect("core is captured");
        assert_eq!(core_row.before_hash, core_head);
        assert_eq!(core_row.after_hash.as_deref(), Some(&*after));
        let docs_row = commits
            .iter()
            .find(|commit| commit.repo_id == docs)
            .expect("docs is captured");
        assert_eq!(docs_row.before_hash, docs_head);
        assert_eq!(docs_row.after_hash, None, "nothing was committed there");
    }

    /// D27: the clean no-commit tree is litter and goes at capture; the dirty one is the agent's
    /// unfinished work and stays, because milestone 5's sweep is what reads it.
    #[tokio::test]
    async fn a_no_commit_clean_worktree_is_removed_at_capture_and_a_dirty_one_is_kept() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, core_head) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let core_path = core_checkout.local_path.clone();
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        let clean = PathBuf::from(&prepared.trees[0].tree.path);
        let dirty = PathBuf::from(&prepared.trees[1].tree.path);
        std::fs::write(dirty.join("f"), "edited\n").expect("the file is edited");

        let commits = isolator
            .capture(step, &rows(&prepared))
            .await
            .expect("the step captures");
        assert!(commits.iter().all(|commit| commit.after_hash.is_none()));

        assert!(!clean.exists(), "the clean no-commit tree is gone");
        assert_eq!(
            crate::isolate::git::worktree_by_path(&core_path, &clean).expect("the worktrees read"),
            None,
            "and so is its administrative entry"
        );
        assert!(dirty.is_dir(), "the dirty tree is kept for milestone 5");
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{step}"))
                .expect("the ref store reads"),
            Some(core_head),
            "the branch survives the removal, which is what reconcile reads"
        );
    }

    /// D27 read against ANA-2 `:2065` risk 2: a tree whose only change is a new, never-staged file
    /// is not "clean" in the sense that licenses `worktree remove --force --force`. D24's
    /// `is_dirty` excludes untracked files by design, so the removal guard asks both questions.
    #[tokio::test]
    async fn a_no_commit_worktree_holding_only_an_untracked_file_survives_capture() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        let tree = PathBuf::from(&prepared.trees[0].tree.path);
        std::fs::write(tree.join("notes.md"), "the agent's only work\n")
            .expect("an untracked file is written");
        assert!(
            !crate::isolate::git::is_dirty(&tree).expect("the status reads"),
            "the premise: D24 does not count it"
        );

        let commits = isolator
            .capture(step, &rows(&prepared))
            .await
            .expect("the step captures");
        assert_eq!(commits[0].after_hash, None);
        assert_eq!(
            std::fs::read_to_string(tree.join("notes.md")).expect("the file survives"),
            "the agent's only work\n",
            "an untracked file is work, and the tree holding it is kept"
        );
    }

    /// D38 and blueprint H-1: a second `prepare` for the same `(run, step)` finds the tree it made
    /// and reports the same `before_hash`, even after the source moved on underneath it.
    #[tokio::test]
    async fn prepare_is_idempotent_for_worktree() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let first = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the first call prepares");
        // The crash window of H-1: the source moves before the retry.
        commit_file(&core_path, "h", "moved\n", "the source moves on");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the second call reuses");

        assert_eq!(
            second.trees[0].before_hash, head,
            "the base is the branch's"
        );
        assert_eq!(second.trees[0].tree.path, first.trees[0].tree.path);
        assert_eq!(
            crate::isolate::git::worktrees_under(
                &core_path,
                &PathBuf::from(&first.trees[0].tree.path)
            )
            .expect("the worktrees read")
            .len(),
            1,
            "one tree, not two"
        );
    }

    /// Blueprint H-5: a `worktree add` killed mid-checkout leaves a tree that is not at its own
    /// branch. It is repaired in place with D47's `reset --hard`, because `git worktree add -b`
    /// cannot re-create a branch that already exists.
    #[tokio::test]
    async fn a_half_made_worktree_is_reset_and_reused() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let first = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the first call prepares");
        let tree = PathBuf::from(&first.trees[0].tree.path);
        std::fs::remove_file(tree.join("f")).expect("a tracked file is deleted");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the second call repairs and reuses");

        assert_eq!(second.trees[0].before_hash, head);
        assert_eq!(second.trees[0].tree.path, first.trees[0].tree.path);
        assert!(tree.join("f").is_file(), "the tree was reset to its base");
        assert!(
            !crate::isolate::git::is_dirty(&tree).expect("the status reads"),
            "and is clean again"
        );
        assert_eq!(
            crate::isolate::git::worktrees_under(&core_path, &tree)
                .expect("the worktrees read")
                .len(),
            1
        );
    }

    /// A tree whose `.git` file points at an administrative entry somebody pruned has a branch but
    /// no readable `HEAD`, and `reset --hard` inside it is a command run outside any repository.
    /// It is re-made instead — removed, and re-added onto the existing branch without `-b`.
    #[tokio::test]
    async fn a_worktree_whose_admin_entry_was_pruned_is_re_added_onto_its_branch() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let first = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the first call prepares");
        let tree = PathBuf::from(&first.trees[0].tree.path);

        // The prune: the administrative entry goes, the tree's `.git` file still points at it.
        let admin = core_path.join(".git").join("worktrees");
        for entry in std::fs::read_dir(&admin).expect("the worktrees directory reads") {
            std::fs::remove_dir_all(entry.expect("an entry").path()).expect("the entry is pruned");
        }
        assert!(
            tree.join(".git").is_file(),
            "the gitfile survives the prune"
        );
        assert!(crate::isolate::git::head(&tree).is_err(), "the premise");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Worktree)
            .await
            .expect("the second call re-makes the tree");

        assert_eq!(
            second.trees[0].before_hash, head,
            "the base is the branch's"
        );
        assert_eq!(second.trees[0].tree.path, first.trees[0].tree.path);
        assert_eq!(
            crate::isolate::git::head(&tree).expect("the tree has a HEAD again"),
            head
        );
        let listed = crate::isolate::git::testkit::worktree_list(&git, &core_path).await;
        let ours = listed
            .iter()
            .find(|entry| entry.path == tree)
            .expect("git lists the re-made tree");
        assert_eq!(
            ours.branch.as_deref(),
            Some(&*format!("refs/heads/htui/{step}")),
            "on the step's own branch"
        );
        assert_eq!(ours.locked.as_deref(), Some(&*format!("htui run {run}")));
    }

    /// The mode table's own refusal: `git worktree add` checks out the gitlink and leaves the
    /// submodule uninitialised, which is a tree the agent cannot build in.
    #[tokio::test]
    async fn a_repo_with_submodules_is_refused_for_worktree() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        commit_file(
            &core_checkout.local_path,
            ".gitmodules",
            "[submodule \"sub\"]\n\tpath = sub\n\turl = ./sub\n",
            "declare a submodule",
        );
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[core], Isolation::Worktree)
            .await
            .expect_err("a repository with submodules is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: submodules: worktree isolation is not supported"
        );
    }

    /// Criterion 13's isolator half (`:2120`): after the run-terminal cleanup neither `gix` nor
    /// `git` lists anything under the scratch root, the maintainer's own linked worktree is
    /// untouched, and every checkout is exactly where it was.
    #[tokio::test]
    async fn cleanup_removes_every_tree_and_leaves_the_repo_untouched() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, core_head) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, docs_head) = repo(dir.path(), "docs", false);
        let core_path = core_checkout.local_path.clone();
        let docs_path = docs_checkout.local_path.clone();
        let root = dir.path().join("trees");
        let isolator = GixIsolator::new(config(
            &root,
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        // The maintainer's own linked worktree, made outside the scratch root and never ours.
        let theirs = dir.path().join("their-own-tree");
        git.add_worktree(&core_path, &theirs, StepId::new(), RunId::new(), &core_head)
            .await
            .expect("their worktree is added");

        let run = RunId::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(run, step, &[core, docs], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        let trees: Vec<PathBuf> = prepared
            .trees
            .iter()
            .map(|prepared| PathBuf::from(&prepared.tree.path))
            .collect();

        isolator
            .cleanup(run, &rows(&prepared))
            .await
            .expect("the run cleans up");

        let canonical_root = std::fs::canonicalize(&root).expect("the root canonicalises");
        for source in [&core_path, &docs_path] {
            assert!(
                crate::isolate::git::worktrees_under(source, &canonical_root)
                    .expect("the worktrees read")
                    .is_empty(),
                "gix lists no entry under the scratch root"
            );
            let listed = crate::isolate::git::testkit::worktree_list(&git, source).await;
            assert!(
                listed
                    .iter()
                    .all(|entry| !entry.path.starts_with(&canonical_root)),
                "and neither does git"
            );
            assert!(
                listed
                    .iter()
                    .all(|entry| entry.branch.as_deref()
                        != Some(&*format!("refs/heads/htui/{step}"))),
                "no htui branch is left checked out anywhere"
            );
            assert!(
                crate::isolate::git::branch_target(source, &format!("htui/{step}"))
                    .expect("the ref store reads")
                    .is_some(),
                "the branches themselves survive"
            );
        }
        for tree in &trees {
            assert!(!tree.exists(), "{}", tree.display());
        }
        assert!(
            !canonical_root.join(run.to_string()).exists(),
            "the run's own directory is gone (F-Q)"
        );

        assert!(theirs.is_dir(), "the maintainer's worktree survives");
        assert!(
            crate::isolate::git::worktree_by_path(&core_path, &theirs)
                .expect("the worktrees read")
                .is_some()
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            core_head
        );
        assert_eq!(
            crate::isolate::git::head(&docs_path).expect("the checkout has a HEAD"),
            docs_head
        );
        for source in [&core_path, &docs_path] {
            assert!(!crate::isolate::git::is_dirty(source).expect("the status reads"));
        }
    }

    /// D46: `git worktree prune` is never spawned, so an entry that survives `remove` is named in
    /// the cleanup error and left for the operator.
    #[tokio::test]
    async fn a_stale_entry_is_reported_not_pruned() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let root = dir.path().join("trees");
        let isolator = GixIsolator::new(config(&root, &[(core, core_checkout)]))
            .expect("the config validates");

        let run = RunId::new();
        let prepared = isolator
            .prepare(run, StepId::new(), &[core], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");

        // Blueprint H-1's crash window, from the other end: a tree whose `run_step_tree` row was
        // never written is a tree `cleanup` is never handed, so no `remove` clears its entry. Its
        // directory goes with `<root>/<run>/`; its administrative entry is what D46 reports.
        let orphan = std::fs::canonicalize(&root)
            .expect("the root canonicalises")
            .join(run.to_string())
            .join(StepId::new().to_string())
            .join("core");
        git.add_worktree(
            &core_path,
            &orphan,
            StepId::new(),
            run,
            &crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
        )
        .await
        .expect("the orphaned worktree is added");
        let tree = orphan;

        let err = isolator
            .cleanup(run, &rows(&prepared))
            .await
            .expect_err("a surviving entry is reported");
        assert_eq!(
            err.to_string(),
            format!(
                "git: stale worktree entry {}; run `git worktree prune` yourself",
                tree.display()
            )
        );
        assert!(
            crate::isolate::git::testkit::worktree_list(&git, &core_path)
                .await
                .iter()
                .any(|entry| entry.path == tree),
            "and is still there afterwards: nothing pruned it"
        );
    }

    /// D35, D47 and blueprint A-2: the copy is a whole checkout, `git reset --hard` erases the
    /// source's dirtiness inside it, and the `htui/<step>` label that records the base is written
    /// **in the copy** — `copy` never writes a ref into the source.
    #[tokio::test]
    async fn copy_resets_a_dirty_source_and_labels_the_base() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        std::fs::write(core_path.join("f"), "edited\n").expect("the source is dirtied");
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::Copy)
            .await
            .expect("the copy mode prepares");

        let copy = PathBuf::from(&prepared.trees[0].tree.path);
        assert_eq!(prepared.trees[0].before_hash, head);
        assert!(
            !prepared.trees[0].tree.dirty,
            "the reset erased the source's dirtiness (the mode table)"
        );
        assert_eq!(copy, prepared.cwd.join("core"));
        assert_eq!(
            std::fs::read_to_string(copy.join("f")).expect("the copied file reads"),
            "first\n",
            "the copy is at the base, not at the source's edit"
        );
        assert_eq!(
            crate::isolate::git::branch_target(&copy, &format!("htui/{step}"))
                .expect("the ref store reads"),
            Some(head.clone()),
            "A-2: the label is inside the copy"
        );
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{step}"))
                .expect("the ref store reads"),
            None,
            "and never in the source"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("f")).expect("the source file reads"),
            "edited\n",
            "the source itself was not touched"
        );

        let after = commit_file(&copy, "g", "second\n", "two");
        let commits = isolator
            .capture(step, &rows(&prepared))
            .await
            .expect("the step captures");
        assert_eq!(commits[0].after_hash.as_deref(), Some(&*after));
        assert!(copy.is_dir(), "a copy is never removed at capture");

        isolator
            .cleanup(run, &rows(&prepared))
            .await
            .expect("the run cleans up");
        assert!(!copy.exists(), "the copy goes at run end");
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head
        );
    }

    /// ANA-2 `:930-932`: the tree is measured before the first byte is copied, and the refusal
    /// names both numbers. The sentence is `copy.rs`'s own — there is no second spelling of it.
    #[tokio::test]
    async fn copy_refuses_over_the_cap() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let mut config = config(&dir.path().join("trees"), &[(core, core_checkout)]);
        config.copy_max_total_bytes = 1;
        let isolator = GixIsolator::new(config).expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[core], Isolation::Copy)
            .await
            .expect_err("the copy is over the cap");
        let text = err.to_string();
        assert!(
            text.starts_with("isolation refused: copy would need ")
                && text.ends_with(" bytes; cap is 1"),
            "{text}"
        );
    }

    /// T4's second finding and blueprint §6.3's order: the two `.git` refusals run before the
    /// measure. A cap of one byte would refuse anything, and the sentence proves which check ran.
    #[tokio::test]
    async fn copy_refuses_a_source_that_is_not_a_checkout_before_it_measures() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let local_path = dir.path().join("core");
        std::fs::create_dir_all(&local_path).expect("the directory is made");
        std::fs::write(local_path.join("f"), "not a checkout\n").expect("the file is written");
        let core = RepoId::new();
        let checkout = RepoCheckout {
            name: "core".to_owned(),
            local_path,
            is_primary: true,
        };
        let mut config = config(&dir.path().join("trees"), &[(core, checkout)]);
        config.copy_max_total_bytes = 1;
        let isolator = GixIsolator::new(config).expect("the config validates");

        let err = isolator
            .prepare(RunId::new(), StepId::new(), &[core], Isolation::Copy)
            .await
            .expect_err("a directory that is not a checkout is refused");
        assert_eq!(err.to_string(), "isolation refused: not a git checkout");
    }

    /// D38 through A-2: a copy's base is read from the label inside it, so a second `prepare`
    /// reuses the copy and reports the same `before_hash` even after the source moved on.
    #[tokio::test]
    async fn prepare_is_idempotent_for_copy() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let run = RunId::new();
        let step = StepId::new();
        let first = isolator
            .prepare(run, step, &[core], Isolation::Copy)
            .await
            .expect("the first call prepares");
        let copy = PathBuf::from(&first.trees[0].tree.path);
        std::fs::write(copy.join("agent-work"), "in progress\n").expect("the agent writes");
        commit_file(&core_path, "h", "moved\n", "the source moves on");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Copy)
            .await
            .expect("the second call reuses");

        assert_eq!(second.trees[0].before_hash, head, "the base is the label's");
        assert_eq!(second.trees[0].tree.path, first.trees[0].tree.path);
        assert!(
            copy.join("agent-work").is_file(),
            "the copy was reused, not remade"
        );
    }

    /// A `worktree` step's `prepare`, its commit and its `capture`, so that a `reconcile` test has
    /// something to merge. Returns the isolator, the checkout, the base and the step's rows.
    async fn stepped(
        dir: &Path,
    ) -> (
        GixIsolator,
        RepoId,
        PathBuf,
        String,
        String,
        StepId,
        Vec<RunStepTree>,
    ) {
        let (core, core_checkout, head) = repo(dir, "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator = GixIsolator::new(config(&dir.join("trees"), &[(core, core_checkout)]))
            .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        let after = commit_file(
            &PathBuf::from(&prepared.trees[0].tree.path),
            "g",
            "the step's work\n",
            "the step commits",
        );
        let rows = rows(&prepared);
        isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");
        (isolator, core, core_path, head, after, step, rows)
    }

    /// D25: the winner is merged into the primary tree with `--no-ff`, and the merge commit is
    /// what the step's `after_hash` becomes (ANA-2 `:987-988`).
    #[tokio::test]
    async fn reconcile_merges_the_winner_no_ff_and_updates_the_primary_tree() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core, core_path, head, after, step, rows) = stepped(dir.path()).await;

        let commits = isolator
            .reconcile(step, &rows)
            .await
            .expect("the winner reconciles");

        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].repo_id, core);
        assert_eq!(commits[0].before_hash, head);
        let merge = commits[0]
            .after_hash
            .clone()
            .expect("the merge commit is the new after_hash");
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            merge
        );
        assert_eq!(
            crate::isolate::git::head_parents(&core_path).expect("the parents read"),
            vec![head, after],
            "--no-ff made a two-parent commit"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the merged file reads"),
            "the step's work\n",
            "the primary working tree carries the step's work"
        );
    }

    /// A step that committed nothing has nothing to merge, and `reconcile` must not invent a
    /// commit for it.
    #[tokio::test]
    async fn reconcile_is_the_identity_for_a_no_commit_step() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Worktree)
            .await
            .expect("the worktree mode prepares");
        let rows = rows(&prepared);
        isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");

        let commits = isolator
            .reconcile(step, &rows)
            .await
            .expect("the identity reconciles");
        assert_eq!(commits[0].after_hash, None);
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the primary tree was not touched"
        );
    }

    /// ANA-2 `:978-979`: a dirty primary tree is refused by name, and the run is parked with it.
    #[tokio::test]
    async fn reconcile_refuses_a_dirty_primary() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, _core, core_path, head, _after, step, rows) = stepped(dir.path()).await;
        std::fs::write(core_path.join("f"), "the maintainer is mid-edit\n")
            .expect("the primary is dirtied");

        let err = isolator
            .reconcile(step, &rows)
            .await
            .expect_err("a dirty primary is refused");
        assert_eq!(err.to_string(), "isolation refused: dirty_primary_tree");
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "and nothing was merged"
        );
    }

    /// The primary moved under us and its new `HEAD` is not a merge of ours, so there is nothing
    /// this milestone can safely do but say where it is.
    #[tokio::test]
    async fn reconcile_refuses_a_moved_primary() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, _core, core_path, _head, _after, step, rows) = stepped(dir.path()).await;
        let moved = commit_file(&core_path, "h", "somebody else\n", "the primary moves");

        let err = isolator
            .reconcile(step, &rows)
            .await
            .expect_err("a moved primary is refused");
        assert_eq!(
            err.to_string(),
            format!("isolation refused: primary_moved: {moved}")
        );
    }

    /// Blueprint H-3: the crash between `git merge` and `record_commits` leaves the primary at a
    /// merge commit the row does not know about; a re-run reads its parents and reports it rather
    /// than refusing `primary_moved`.
    #[tokio::test]
    async fn reconcile_after_a_crash_between_merge_and_the_row_is_idempotent() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, _core, core_path, _head, _after, step, rows) = stepped(dir.path()).await;

        let first = isolator
            .reconcile(step, &rows)
            .await
            .expect("the winner reconciles");
        let second = isolator
            .reconcile(step, &rows)
            .await
            .expect("the second reconcile reads the merge it already made");

        assert_eq!(first[0].after_hash, second[0].after_hash);
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            second[0]
                .after_hash
                .clone()
                .expect("the merge commit is reported"),
            "nothing was merged a second time"
        );
    }

    /// Plan OQ-5: a copy has its own object database, so the step's range is written into the
    /// primary's before the merge can resolve the hash at all.
    #[tokio::test]
    async fn copy_reconcile_copies_the_range_then_merges() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Copy)
            .await
            .expect("the copy mode prepares");
        let copy = PathBuf::from(&prepared.trees[0].tree.path);
        let after = commit_file(&copy, "g", "the step's work\n", "the step commits");
        let rows = rows(&prepared);
        isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");
        assert!(
            !crate::isolate::git::testkit::has_object(&core_path, &after),
            "the source has never seen the copy's commit"
        );

        let commits = isolator
            .reconcile(step, &rows)
            .await
            .expect("the copy reconciles");

        assert!(
            crate::isolate::git::testkit::has_object(&core_path, &after),
            "the range was copied into the primary's object database"
        );
        let merge = commits[0]
            .after_hash
            .clone()
            .expect("the merge commit is the new after_hash");
        assert_eq!(
            crate::isolate::git::head_parents(&core_path).expect("the parents read"),
            vec![head, after]
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            merge
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the merged file reads"),
            "the step's work\n"
        );
    }
}
