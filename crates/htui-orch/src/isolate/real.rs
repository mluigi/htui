//! `GixIsolator`: the production [`Isolator`](super::Isolator), four modes over `gix` reads and
//! five `git` verbs (plan D22–D47).
//!
//! This is the first place ANA-2 §4.6's four isolation modes exist as *behaviour* rather than as
//! pieces: `isolate/git.rs` knows what a worktree is, `isolate/copy.rs` knows what a copy is, and
//! neither knows which of them a step wants. Every mode is one private `async fn` per verb, and
//! every call into the two sync modules runs under [`tokio::task::spawn_blocking`] with owned
//! paths — no `gix::Repository` crosses an `.await` (blueprint H-17), and no `.await` happens under
//! a `std` lock (`crates/htui-core/src/store/mem.rs:3-7` is the precedent).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use htui_core::model::{BoxId, Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId};
use tokio::sync::OwnedMutexGuard;

use super::git::{self, Cli};
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

/// The guards a step is holding between its own `prepare` and its own `capture` (blueprint A-1).
type Held = Mutex<BTreeMap<StepId, Vec<OwnedMutexGuard<()>>>>;

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
    /// Keyed by step and not by run because the guard is the *step*'s: every phase of a run
    /// resolves to the project's `default_isolation` (`model/kind.rs:256`), so a guard held to the
    /// run's `cleanup` would deadlock the second step of any multi-step `shared_serialized` run
    /// against the first. [`cleanup`](Isolator::cleanup) releases what a crashed step left.
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
    async fn acquire(&self, step: StepId, checkouts: &[(RepoId, RepoCheckout)]) {
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
                .entry(step)
                .or_default()
                .push(guard);
        }
    }

    /// Drops every guard `step` is holding; a step that holds none is not an error.
    fn release(&self, step: StepId) {
        let guards = self
            .held
            .lock()
            .expect("no panic holds the isolator's held guards")
            .remove(&step);
        drop(guards);
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
            self.acquire(step, checkouts).await;
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
                Isolation::Worktree | Isolation::Copy => {
                    return Err(not_landed_yet(tree.mode));
                }
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

/// A mode whose behaviour is not in the tree yet; removed as each lands (MOD-4 M3 T5).
fn not_landed_yet(mode: Isolation) -> IsolateError {
    IsolateError::Git(format!("the {} mode is not implemented yet", mode.as_str()))
}

/// Runs one synchronous `gix` or filesystem call on the blocking pool.
///
/// Every call into `isolate/git.rs`'s `gix` half and `isolate/copy.rs` goes through here with owned
/// paths, which is what keeps a `gix::Repository` — `Send` and not `Sync` — from ever being alive
/// across an `.await` (blueprint H-17).
async fn blocking<T, F>(task: F) -> Result<T, IsolateError>
where
    F: FnOnce() -> Result<T, IsolateError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result,
        Err(err) => Err(IsolateError::Git(format!(
            "a blocking git task did not finish: {err}"
        ))),
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
                    self.prepare_in_place(run, step, &checkouts, isolation)
                        .await
                }
                Isolation::Worktree | Isolation::Copy => {
                    // D40: the two modes that shell out answer with the probe's own sentence
                    // before they touch a filesystem.
                    self.cli()?;
                    Err(not_landed_yet(isolation))
                }
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
                    Isolation::Worktree | Isolation::Copy => return Err(not_landed_yet(tree.mode)),
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

    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            for tree in trees {
                match tree.mode {
                    // D43: whatever the step's own `capture` never released (blueprint H-15).
                    Isolation::SharedSerialized => self.release(tree.run_step_id),
                    // Nothing was created, so nothing is removed.
                    Isolation::Local => {}
                    Isolation::Worktree | Isolation::Copy => return Err(not_landed_yet(tree.mode)),
                }
            }
            // F-Q: `prepare` made `<root>/<run>/<step>/` and no per-tree verb names it.
            let run_dir = self.config.scratch_root.join(run.to_string());
            match std::fs::remove_dir_all(&run_dir) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(IsolateError::Io(err)),
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use htui_core::model::{BoxId, Isolation, RepoId, RunId, StepId};

    use crate::isolate::Isolator as _;
    use crate::isolate::git::testkit::{commit_file, empty_repo, repo_with_one_commit};

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
}
