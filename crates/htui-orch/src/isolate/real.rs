//! `GixIsolator`: the production [`Isolator`], four modes over `gix` reads and
//! six `git` verbs (plan D22–D47; MOD-4 milestone 4 D54–D57 for fan-out).
//!
//! This is the first place ANA-2 §4.6's four isolation modes exist as *behaviour* rather than as
//! pieces: `isolate/git.rs` knows what a worktree is, `isolate/copy.rs` knows what a copy is, and
//! neither knows which of them a step wants. Every mode is one private `async fn` per verb, and
//! every call into the two sync modules runs through `git::blocking` (`spawn_blocking`) with
//! owned paths — no `gix::Repository` crosses an `.await` (blueprint H-17), and no `.await` happens
//! under a `std` lock (`crates/htui-core/src/store/mem.rs:3-7` is the precedent). The `git` verbs
//! are the one exception by design: they are `async` process supervision, and the `gix`
//! post-conditions inside them go through the same helper.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use htui_core::model::{BoxId, Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId};
use htui_core::prompt::DiffBlock;
use tokio::sync::OwnedMutexGuard;

use super::copy;
use super::git::{self, Cli, blocking};
use super::{
    FanoutSlot, IsolateError, Isolator, IsolatorFuture, Prepared, PreparedTree, ResetReport,
};

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

/// MOD-4 milestone 4 D54(a): the slot's base names no entry for a repository of the scope, so
/// there is no commit the candidate could start from.
#[must_use]
pub fn no_slot_base(name: &str) -> String {
    format!("no fan-out base for {name}")
}

/// `local` works in the user's own checkout with no guard, so its candidates could not be
/// compared; `fan_out > 1` is refused at snapshot time for it (`graph.rs`), and this is the
/// isolator's own answer should a slot reach it anyway.
#[must_use]
pub fn local_cannot_fan_out() -> String {
    "local isolation cannot fan out".to_owned()
}

/// D56, D72: a `shared_serialized` candidate would have to reset a checkout that holds
/// uncommitted work, which is either the user's or a sibling's (ANA-2 `:916`).
#[must_use]
pub fn dirty_tree_not_reset(path: &Path) -> String {
    format!("dirty_tree_not_reset: {}", path.display())
}

/// D114 (blueprint F-T): the step's `htui/<step>` label already names a commit other than the
/// checkout's `HEAD`, so labelling `HEAD` would move it and resetting would strand one of the two.
#[must_use]
pub fn label_conflict(label: &str, target: &str, head: &str) -> String {
    format!("label_conflict: {label} names {target}, HEAD is {head}")
}

/// Blueprint A-6: a `local` checkout is the maintainer's own tree and branch, so a `HEAD` that moved
/// during the run is not reset under them — even with the commits labelled, `reset --hard` would
/// move their branch backwards.
#[must_use]
pub fn local_moved(path: &Path, head: &str, base: &str) -> String {
    format!(
        "local_moved: {} at {head}, before_hash {base}",
        path.display()
    )
}

/// Plan D138 (review L1): `reset` failed on a later row after these rows were already labelled and
/// `reset --hard`, as `(repo, path, head, base)`. Appended to that failure, so the sweep's
/// `never_reset` reason says which trees *were* moved and where their work is.
#[must_use]
pub fn already_reset(rows: &[(RepoId, &Path, &str, &str)]) -> String {
    let rows = rows
        .iter()
        .map(|(repo, path, head, base)| format!("{repo} {} from {head} to {base}", path.display()))
        .collect::<Vec<_>>();
    format!("already reset: {}", rows.join(", "))
}

/// D138: `err` in its own variant with [`already_reset`] appended, when `done` names any row; an
/// `Io` failure has no text of its own to extend and is carried as `Git`.
fn part_way(err: IsolateError, done: &[(RepoId, PathBuf, String, String)]) -> IsolateError {
    if done.is_empty() {
        return err;
    }
    let rows = done
        .iter()
        .map(|(repo, path, head, base)| (*repo, path.as_path(), head.as_str(), base.as_str()))
        .collect::<Vec<_>>();
    let named = already_reset(&rows);
    match err {
        IsolateError::Refused(text) => IsolateError::Refused(format!("{text}; {named}")),
        IsolateError::Git(text) => IsolateError::Git(format!("{text}; {named}")),
        IsolateError::Io(io) => IsolateError::Git(format!("{io}; {named}")),
    }
}

/// Where a tree of this repository starts: the slot's base when there is a slot (D54(a)), the
/// checkout's `HEAD` otherwise.
fn slot_base(
    slot: Option<FanoutSlot<'_>>,
    repo: RepoId,
    checkout: &RepoCheckout,
) -> Option<Result<String, IsolateError>> {
    slot.map(|slot| {
        slot.base
            .get(&repo)
            .cloned()
            .ok_or_else(|| IsolateError::Refused(no_slot_base(&checkout.name)))
    })
}

/// D54(c): each repository's text under a `# repo <name>` line of its own.
///
/// A part that does not end in a newline — a patch D55 cut at its cap ends in the truncation
/// marker — is given one, or the next header would share its line.
fn under_repo_headers<'a>(parts: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    let mut joined = String::new();
    for (name, text) in parts {
        if !joined.is_empty() && !joined.ends_with('\n') {
            joined.push('\n');
        }
        joined.push_str("# repo ");
        joined.push_str(name);
        joined.push('\n');
        joined.push_str(text);
    }
    joined
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

/// D70 and D73's admin locks: one `tokio` mutex per repository, minted on first use and never
/// removed, held around one `git worktree add`/`remove` child at a time.
type AdminLocks = Mutex<BTreeMap<RepoId, Arc<tokio::sync::Mutex<()>>>>;

/// The guards a step is holding between its own `prepare` and its own `capture` (blueprint A-1),
/// keyed by the run too so the run's `cleanup` can reach a guard no row names.
type Held = Mutex<BTreeMap<(RunId, StepId), Vec<OwnedMutexGuard<()>>>>;

/// The production [`Isolator`]: four modes over `gix` reads and six `git` verbs (plan D22–D47).
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
    /// MOD-4 milestone 4 D70 and D73 (blueprint A-2): every `git worktree add` and
    /// `git worktree remove` on one repository runs under that repository's lock.
    ///
    /// Two concurrent `add`s on one repository fail about one time in 250 with `fatal: failed to
    /// read .git/worktrees/<id>/commondir` (the plan's probe on git 2.43.0), a wording no retry
    /// recognises, after already creating the branch; `remove` mutates the same directory. The
    /// lock is keyed by repository alone, independent of D43's `(box, repo)` step guard, and is
    /// held for the one child and nothing else — never across another lock or an agent session —
    /// so it cannot take part in a deadlock.
    admin: AdminLocks,
    /// How many admin-locked children are running right now; test instrumentation only.
    #[cfg(test)]
    in_flight: std::sync::atomic::AtomicU32,
    /// The most that ever ran at once; `max_admin_in_flight` reads it.
    #[cfg(test)]
    max_in_flight: std::sync::atomic::AtomicU32,
    /// How many of `reconcile_isolated`'s D136 probes found the repository's admin lock held;
    /// test instrumentation only.
    #[cfg(test)]
    reconcile_held: std::sync::atomic::AtomicU32,
    /// How many of those probes found it free.
    #[cfg(test)]
    reconcile_unheld: std::sync::atomic::AtomicU32,
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
            admin: Mutex::default(),
            #[cfg(test)]
            in_flight: std::sync::atomic::AtomicU32::new(0),
            #[cfg(test)]
            max_in_flight: std::sync::atomic::AtomicU32::new(0),
            #[cfg(test)]
            reconcile_held: std::sync::atomic::AtomicU32::new(0),
            #[cfg(test)]
            reconcile_unheld: std::sync::atomic::AtomicU32::new(0),
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

    /// The commit a tree of `repo` starts from: `slot.base[repo]`, or the checkout's `HEAD` when
    /// there is no slot.
    async fn start_of(
        &self,
        slot: Option<FanoutSlot<'_>>,
        repo: RepoId,
        checkout: &RepoCheckout,
    ) -> Result<String, IsolateError> {
        match slot_base(slot, repo, checkout) {
            Some(base) => base,
            None => self.head_of(checkout).await,
        }
    }

    /// D56 and D72: a `shared_serialized` candidate's checkouts, each put at the group base.
    ///
    /// Every checkout's dirtiness is read **before** anything is reset, so a refusal leaves the
    /// whole scope where it was: a dirty checkout is the user's work or a sibling's, and neither
    /// may be destroyed. A clean checkout off the base — where the previous sibling left it — is
    /// `reset --hard` to it (M3 D47's verb). "Clean" includes plan D121's half, which `is_dirty`
    /// leaves out (D137): no untracked file at a path the base tracks, which the reset would write
    /// over.
    async fn reset_to_slot_base(
        &self,
        checkouts: &[(RepoId, RepoCheckout)],
        slot: FanoutSlot<'_>,
    ) -> Result<(), IsolateError> {
        let mut targets = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            let base =
                slot_base(Some(slot), *repo, checkout).expect("a slot always yields an answer")?;
            let path = checkout.local_path.clone();
            if blocking(move || git::is_dirty(&path)).await? {
                return Err(IsolateError::Refused(dirty_tree_not_reset(
                    &checkout.local_path,
                )));
            }
            if self.head_of(checkout).await? == base {
                continue;
            }
            self.refuse_untracked_under(&checkout.local_path, &base)
                .await?;
            targets.push((checkout, base));
        }
        for (checkout, base) in targets {
            let git = self.cli()?.clone();
            let local = checkout.local_path.clone();
            git::with_retry("reset --hard", || git.reset_hard(&local, &base)).await?;
        }
        Ok(())
    }

    /// Plan D121's guard, as D137 applies it before every other `reset --hard`: `dirty_tree_not_reset`
    /// when `path` holds an untracked, not-ignored file at a path `target` tracks.
    async fn refuse_untracked_under(&self, path: &Path, target: &str) -> Result<(), IsolateError> {
        let (read, base) = (path.to_path_buf(), target.to_owned());
        if blocking(move || git::untracked_paths_base_tracks(&read, &base))
            .await?
            .is_empty()
        {
            Ok(())
        } else {
            Err(IsolateError::Refused(dirty_tree_not_reset(path)))
        }
    }

    /// D43's guard for every repository of the scope, taken **before** the mode's reads (A-1).
    ///
    /// The `std` lock is released before the `tokio` one is awaited: an `.await` under a `std`
    /// guard is what `mem.rs:3-7` forbids, and here it would also deadlock the next `prepare`.
    /// The guards are taken in `RepoId` order, never scope order: two steps whose scopes name the
    /// same repositories in opposite orders would otherwise each hold one and wait on the other.
    async fn acquire(&self, run: RunId, step: StepId, checkouts: &[(RepoId, RepoCheckout)]) {
        let mut repos: Vec<RepoId> = checkouts.iter().map(|(repo, _)| *repo).collect();
        repos.sort_unstable();
        repos.dedup();
        for repo in &repos {
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

    /// D70's lock for `repo`, awaited.
    ///
    /// The shape of [`acquire`](GixIsolator::acquire): the `Arc` is cloned under the `std` lock,
    /// that lock is dropped, and only then is the `tokio` one awaited.
    async fn admin_guard(&self, repo: RepoId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut admin = self
                .admin
                .lock()
                .expect("no panic holds the isolator's admin locks");
            Arc::clone(admin.entry(repo).or_default())
        };
        lock.lock_owned().await
    }

    /// Runs one `worktree add`/`remove` (with its D39 retries) under `repo`'s admin lock (D70,
    /// D73), and holds the lock for nothing else.
    async fn under_admin<T, Fut>(
        &self,
        repo: RepoId,
        verb: impl FnOnce() -> Fut,
    ) -> Result<T, IsolateError>
    where
        Fut: Future<Output = Result<T, IsolateError>>,
    {
        let guard = self.admin_guard(repo).await;
        #[cfg(test)]
        {
            use std::sync::atomic::Ordering;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        }
        let result = verb().await;
        #[cfg(test)]
        self.in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        drop(guard);
        result
    }

    /// The most admin-locked `git` children that were ever in flight at once.
    #[cfg(test)]
    fn max_admin_in_flight(&self) -> u32 {
        self.max_in_flight.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// D136's probe: records whether `repo`'s admin lock is held at this point of an isolated
    /// `reconcile`. Only meaningful while nothing else could hold that lock.
    #[cfg(test)]
    fn probe_reconcile_admin(&self, repo: RepoId) {
        use std::sync::atomic::Ordering;
        let held = self
            .admin
            .lock()
            .expect("no panic holds the isolator's admin locks")
            .get(&repo)
            .is_some_and(|lock| lock.try_lock().is_err());
        let counter = if held {
            &self.reconcile_held
        } else {
            &self.reconcile_unheld
        };
        counter.fetch_add(1, Ordering::SeqCst);
    }

    /// `(held, unheld)`: how often `reconcile_isolated`'s probes found the admin lock held, and
    /// free.
    #[cfg(test)]
    fn reconcile_admin_probes(&self) -> (u32, u32) {
        use std::sync::atomic::Ordering;
        (
            self.reconcile_held.load(Ordering::SeqCst),
            self.reconcile_unheld.load(Ordering::SeqCst),
        )
    }

    /// Drops every guard `step` is holding; a step that holds none is not an error.
    fn release_step(&self, step: StepId) {
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
    ///
    /// A fan-out slot changes the `shared_serialized` half only (D56, D72): after the guard, every
    /// checkout is refused if dirty and otherwise reset to the group base, so the body below
    /// records `before = base` and `dirty = false`. `local` refuses a slot outright.
    async fn prepare_in_place(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
        mode: Isolation,
        slot: Option<FanoutSlot<'_>>,
    ) -> Result<Prepared, IsolateError> {
        if mode == Isolation::SharedSerialized {
            self.acquire(run, step, checkouts).await;
        }
        if let Some(slot) = slot {
            if mode == Isolation::Local {
                return Err(IsolateError::Refused(local_cannot_fan_out()));
            }
            self.reset_to_slot_base(checkouts, slot).await?;
        }

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            let before = self.start_of(slot, *repo, checkout).await?;
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
                create_directory(&cwd).await?;
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
    /// `<root>/<run>/<step>/`, each on its own `htui/<step>` branch at the source's `HEAD` — or at
    /// the slot's base for a fan-out candidate (D54(a)).
    async fn prepare_worktree(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
        slot: Option<FanoutSlot<'_>>,
    ) -> Result<Prepared, IsolateError> {
        let git = self.cli()?.clone();
        let cwd = session_dir(&self.config.scratch_root, run, step);
        create_directory(&cwd).await?;
        let branch = format!("htui/{step}");

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            let local = checkout.local_path.clone();
            if blocking(move || git::has_submodules(&local)).await? {
                return Err(IsolateError::Refused(submodules_refused()));
            }
            let source_head = self.start_of(slot, *repo, checkout).await?;
            let path = cwd.join(&checkout.name);
            let before = self
                .worktree_at(
                    &git,
                    *repo,
                    checkout,
                    &path,
                    &branch,
                    &source_head,
                    step,
                    run,
                )
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
        repo_id: RepoId,
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
                        self.under_admin(repo_id, || {
                            git::with_retry("worktree remove", || git.remove_worktree(&local, path))
                        })
                        .await?;
                        remove_directory(path).await?;
                        self.under_admin(repo_id, || {
                            git::with_retry("worktree add", || {
                                git.add_worktree_on_branch(&local, path, step, run, &base)
                            })
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
                    self.under_admin(repo_id, || {
                        git::with_retry("worktree remove", || git.remove_worktree(&local, path))
                    })
                    .await?;
                    remove_directory(path).await?;
                }
            }
        }
        self.under_admin(repo_id, || {
            git::with_retry("worktree add", || {
                git.add_worktree(&local, path, step, run, source_head)
            })
        })
        .await?;
        Ok(source_head.to_owned())
    }

    /// `copy` (plan D35, D47; blueprint A-2): a whole filesystem copy of the checkout per
    /// repository, reset to the source's `HEAD` — or to the slot's base for a fan-out candidate
    /// (D54(a), D57) — and labelled with the base it was taken from.
    async fn prepare_copy(
        &self,
        run: RunId,
        step: StepId,
        checkouts: &[(RepoId, RepoCheckout)],
        slot: Option<FanoutSlot<'_>>,
    ) -> Result<Prepared, IsolateError> {
        let git = self.cli()?.clone();
        let cwd = session_dir(&self.config.scratch_root, run, step);
        create_directory(&cwd).await?;
        let branch = format!("htui/{step}");
        let excludes = copy::excludes(&self.config.copy_exclude);

        let mut trees = Vec::with_capacity(checkouts.len());
        for (repo, checkout) in checkouts {
            // Blueprint §6.3's order: D35's two `.git` refusals answer before anything is
            // measured and before the reuse branch is even consulted.
            let source = checkout.local_path.clone();
            blocking(move || copy::check_source(&source)).await?;
            let source_head = self.start_of(slot, *repo, checkout).await?;
            let path = cwd.join(&checkout.name);
            // D57: every candidate of the group makes its own copy of this tree.
            let copies = slot.map_or(1, |slot| u64::try_from(slot.width).unwrap_or(1).max(1));
            let before = self
                .copy_at(
                    &git,
                    checkout,
                    &path,
                    &branch,
                    &source_head,
                    &excludes,
                    copies,
                )
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
    #[expect(
        clippy::too_many_arguments,
        reason = "every one of them is a fact of the copy being made; a struct for them would be \
                  named once and read once"
    )]
    async fn copy_at(
        &self,
        git: &Cli,
        checkout: &RepoCheckout,
        path: &Path,
        branch: &str,
        source_head: &str,
        excludes: &[copy::Exclude],
        copies: u64,
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
            remove_directory(path).await?;
        }

        let (source, owned) = (checkout.local_path.clone(), excludes.to_vec());
        let cap = self.config.copy_max_total_bytes;
        blocking(move || copy::measure_within_cap(&source, &owned, cap, copies)).await?;

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
            self.under_admin(tree.repo_id, || {
                git::with_retry("worktree remove", || git.remove_worktree(&local, &path))
            })
            .await?;
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
        self.under_admin(tree.repo_id, || {
            git::with_retry("worktree remove", || git.remove_worktree(&local, &path))
        })
        .await
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
    /// (ANA-2 `:978-979`), and a primary that moved off the base's line. A primary that moved
    /// *ahead* — the base is an ancestor of `HEAD`, as when another run on disjoint paths merged
    /// first (plan D136, rule P) — is merged into on top of its `HEAD`; a real conflict is
    /// [`git::merge_conflict`]'s refusal like any other. Before that, blueprint H-3's crash
    /// between the merge and `record_commits` is looked for: a merge on `HEAD`'s first-parent
    /// history back to the base whose second parent is the tip is this step's, even under a later
    /// run's merge, and the answer is that commit rather than a second merge. Everything from the
    /// clean check to the merge's post-condition runs under the repository's admin lock (D70), so
    /// two runs of one process reconciling one repository take turns.
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
        // D136 under D70's lock: from the clean check and the `HEAD` read through the merge's
        // post-condition, so another run of this process cannot merge in between and leave this
        // merge on top of one the post-condition does not expect. Nothing below takes the lock
        // again — `tokio`'s mutex is not reentrant. Another *process* is not held off; its merge
        // in between fails the post-condition, the run parks, and the next reconcile finds the
        // landed merge through `merge_of`.
        let _admin = self.admin_guard(tree.repo_id).await;
        let read = local.clone();
        if blocking(move || git::is_dirty(&read)).await? {
            return Err(IsolateError::Refused(dirty_primary_tree()));
        }
        #[cfg(test)]
        self.probe_reconcile_admin(tree.repo_id);
        let read = local.clone();
        let head = blocking(move || git::head(&read)).await?;
        if head != tree.base_ref {
            // D136: the merge this step would make may be `HEAD`, or lie under a later run's.
            let (read, at, base, tip) = (
                local.clone(),
                head.clone(),
                tree.base_ref.clone(),
                after.clone(),
            );
            if let Some(merge) = blocking(move || git::merge_of(&read, &at, &base, &tip)).await? {
                return Ok(Some(merge));
            }
            let (read, at, base) = (local.clone(), head.clone(), tree.base_ref.clone());
            if !blocking(move || git::is_ancestor(&read, &base, &at)).await? {
                return Err(IsolateError::Refused(primary_moved(&head)));
            }
            // `HEAD` already holds `after` with no merge naming it (the branch was fast-forwarded
            // onto the step): `merge --no-ff` would make no commit, so this is refused as before.
            let (read, at, tip) = (local.clone(), head.clone(), after.clone());
            if blocking(move || git::is_ancestor(&read, &tip, &at)).await? {
                return Err(IsolateError::Refused(primary_moved(&head)));
            }
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

        // D136: onto `HEAD`, which is the base itself or a descendant of it.
        let merged =
            git::with_retry("merge", || git.merge_no_ff(&local, step, &head, &after)).await?;
        #[cfg(test)]
        self.probe_reconcile_admin(tree.repo_id);
        Ok(Some(merged.commit))
    }

    /// `reconcile` for a tree that is the checkout itself: nothing to merge, because the step
    /// committed into the primary tree as it went.
    ///
    /// For a fan-out winner under `shared_serialized` (D56) the checkout is wherever the *last*
    /// sibling left it, so the checked-out branch is moved to the winner's label — or to the base,
    /// for a winner that committed nothing — with `reset --hard`. That move is taken only when it
    /// destroys nothing: the checkout is clean, and its `HEAD` is the group base or a sibling's
    /// label. With no siblings (`fan_out = 1`) the M3 rule stands unchanged. Clean includes plan
    /// D121's half (D137): an untracked file at a path the target tracks refuses
    /// `dirty_tree_not_reset`.
    async fn reconcile_in_place(
        &self,
        step: StepId,
        tree: &RunStepTree,
        checkout: &RepoCheckout,
        siblings: &[StepId],
    ) -> Result<Option<String>, IsolateError> {
        let head = self.head_of(checkout).await?;
        if tree.mode == Isolation::Local {
            return Ok((head != tree.base_ref).then_some(head));
        }

        // `shared_serialized`: the label D26 wrote at capture is what says the checkout is still
        // where this step left it.
        let label = self.label_of(checkout, step).await?;
        let target = label.clone().unwrap_or_else(|| tree.base_ref.clone());
        if head == target {
            return Ok((head != tree.base_ref).then_some(head));
        }
        if siblings.is_empty() {
            // M3: anything else moved it, and a `fan_out = 1` step has no sibling whose work the
            // checkout could be holding.
            return Err(IsolateError::Refused(primary_moved(&head)));
        }

        let read = checkout.local_path.clone();
        if blocking(move || git::is_dirty(&read)).await? {
            return Err(IsolateError::Refused(dirty_primary_tree()));
        }
        let mut left_by_us = head == tree.base_ref;
        for sibling in siblings {
            if left_by_us {
                break;
            }
            left_by_us = self.label_of(checkout, *sibling).await?.as_deref() == Some(&*head);
        }
        if !left_by_us {
            return Err(IsolateError::Refused(primary_moved(&head)));
        }
        // D137: `is_dirty` above leaves untracked files out; the reset would not.
        self.refuse_untracked_under(&checkout.local_path, &target)
            .await?;
        let git = self.cli()?.clone();
        let local = checkout.local_path.clone();
        git::with_retry("reset --hard", || git.reset_hard(&local, &target)).await?;
        Ok(label.filter(|label| *label != tree.base_ref))
    }

    /// Where `htui/<step>` points in `checkout`, or `None` when the step never labelled it.
    async fn label_of(
        &self,
        checkout: &RepoCheckout,
        step: StepId,
    ) -> Result<Option<String>, IsolateError> {
        let path = checkout.local_path.clone();
        let name = format!("htui/{step}");
        blocking(move || git::branch_target(&path, &name)).await
    }

    /// One repository's slice of [`Isolator::diff`]: its name, its range and both texts, or `None`
    /// when the row committed nothing.
    ///
    /// D74 (blueprint A-3) chooses the repository that holds `after`: a `copy` tree's own object
    /// database until reconcile, and the checkout after it, because the merge commit exists only
    /// there; every other mode shares the checkout's object database.
    ///
    /// The range is the row's `before..after`, except for a step's own reconcile merge (plan
    /// D141, [`git::reconcile_parent`]): that is diffed from its first parent, which is the base
    /// itself unless another run's merge moved the primary first (D136).
    async fn diff_of(
        &self,
        git: &Cli,
        trees: &[RunStepTree],
        commit: &RunStepCommit,
    ) -> Result<Option<(String, String, String, String)>, IsolateError> {
        let before = commit.before_hash.as_str();
        let Some(after) = commit
            .after_hash
            .as_deref()
            .filter(|after| *after != before)
        else {
            return Ok(None);
        };
        let checkout = self
            .config
            .repos
            .get(&commit.repo_id)
            .ok_or_else(|| IsolateError::Refused(no_checkout_for_repo()))?;
        let mut repo = checkout.local_path.clone();
        if let Some(tree) = trees
            .iter()
            .find(|tree| tree.repo_id == commit.repo_id && tree.mode == Isolation::Copy)
        {
            let copy = PathBuf::from(&tree.path);
            // A copy that is gone, or does not hold the commit, falls back to the checkout; a
            // commit neither holds is `git diff`'s own readable failure.
            let (probe, hex) = (copy.clone(), after.to_owned());
            if blocking(move || git::has_commit(&probe, &hex))
                .await
                .unwrap_or(false)
            {
                repo = copy;
            }
        }
        // D141: a merge onto a primary another run moved is diffed against its first parent, so
        // the range holds this step's change and never the other run's.
        let (probe, hex, step) = (repo.clone(), after.to_owned(), commit.run_step_id);
        let merged_onto = blocking(move || git::reconcile_parent(&probe, &hex, step)).await?;
        let before = merged_onto.as_deref().unwrap_or(before);
        let stat = git.diff(&repo, before, after, true).await?;
        let patch = git.diff(&repo, before, after, false).await?;
        Ok(Some((
            checkout.name.clone(),
            format!("{before}..{after}"),
            stat,
            patch,
        )))
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
///
/// On the blocking pool, like every other filesystem walk here: a tree under the scratch root can
/// hold a whole `target/`, and deleting it is not work for a runtime worker.
async fn remove_directory(path: &Path) -> Result<(), IsolateError> {
    let path = path.to_path_buf();
    blocking(move || match std::fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(IsolateError::Io(err)),
    })
    .await
}

/// `create_dir_all` on the blocking pool, for the same reason as [`remove_directory`].
async fn create_directory(path: &Path) -> Result<(), IsolateError> {
    let path = path.to_path_buf();
    blocking(move || Ok(std::fs::create_dir_all(&path)?)).await
}

impl Isolator for GixIsolator {
    fn prepare<'a>(
        &'a self,
        run: RunId,
        step: StepId,
        scope: &'a [RepoId],
        isolation: Isolation,
        slot: Option<FanoutSlot<'a>>,
    ) -> IsolatorFuture<'a, Prepared> {
        Box::pin(async move {
            let checkouts = self.resolve_scope(scope)?;
            match isolation {
                Isolation::Local | Isolation::SharedSerialized => {
                    let prepared = self
                        .prepare_in_place(run, step, &checkouts, isolation, slot)
                        .await;
                    if prepared.is_err() {
                        // A refusal after the guard was taken has no `run_step_tree` row to its
                        // name, so no `cleanup` would ever be handed it (blueprint A-1 releases
                        // by row): the guard goes back here or it never goes back at all.
                        self.release_step(step);
                    }
                    prepared
                }
                // D40: the two modes that shell out answer with the probe's own sentence before
                // they touch a filesystem.
                Isolation::Worktree => self.prepare_worktree(run, step, &checkouts, slot).await,
                Isolation::Copy => self.prepare_copy(run, step, &checkouts, slot).await,
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
            self.release_step(step);
            captured
        })
    }

    /// D54(b): `resolve_scope`'s refusals, then each checkout's
    /// `HEAD`. A read: no guard, no `git` child.
    fn base<'a>(&'a self, scope: &'a [RepoId]) -> IsolatorFuture<'a, BTreeMap<RepoId, String>> {
        Box::pin(async move {
            let mut base = BTreeMap::new();
            for (repo, checkout) in self.resolve_scope(scope)? {
                base.insert(repo, self.head_of(&checkout).await?);
            }
            Ok(base)
        })
    }

    /// D55: two `git diff`s per committed row (stat, then patch), in the order of `commits`
    /// (blueprint F-J: `repo_id` order, which is what the engine hands). No usable `git` is
    /// `Ok(None)`: the diff is advisory, and its absence degrades a prompt rather than failing it.
    fn diff<'a>(
        &'a self,
        trees: &'a [RunStepTree],
        commits: &'a [RunStepCommit],
    ) -> IsolatorFuture<'a, Option<DiffBlock>> {
        Box::pin(async move {
            let Ok(git) = self.cli() else {
                return Ok(None);
            };
            let git = git.clone();
            let mut parts = Vec::new();
            for commit in commits {
                if let Some(part) = self.diff_of(&git, trees, commit).await? {
                    parts.push(part);
                }
            }
            Ok(match parts.as_slice() {
                [] => None,
                [(_, range, stat, patch)] => Some(DiffBlock {
                    range: range.clone(),
                    stat: stat.clone(),
                    diff: patch.clone(),
                }),
                several => Some(DiffBlock {
                    range: several
                        .iter()
                        .map(|(name, range, _, _)| format!("{name}:{range}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    stat: under_repo_headers(
                        several
                            .iter()
                            .map(|(name, _, stat, _)| (name.as_str(), stat.as_str())),
                    ),
                    diff: under_repo_headers(
                        several
                            .iter()
                            .map(|(name, _, _, patch)| (name.as_str(), patch.as_str())),
                    ),
                }),
            })
        })
    }

    fn reconcile<'a>(
        &'a self,
        winner: StepId,
        trees: &'a [RunStepTree],
        siblings: &'a [StepId],
    ) -> IsolatorFuture<'a, Vec<RunStepCommit>> {
        Box::pin(async move {
            let mut commits = Vec::with_capacity(trees.len());
            for tree in trees {
                let checkout = self.checkout_of(tree)?.clone();
                let after = match tree.mode {
                    Isolation::Local | Isolation::SharedSerialized => {
                        self.reconcile_in_place(winner, tree, &checkout, siblings)
                            .await?
                    }
                    // D54(d): a fan-out winner is merged alone; the losers' labels stay.
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
    /// removes; no sweep retries a terminal run's cleanup (milestone 6).
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
                        if let Err(err) = remove_directory(Path::new(&tree.path)).await {
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
            if let Err(err) = remove_directory(&run_dir).await {
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

    /// D92, D114, D121: every in-place row checked before any is written; `worktree`/`copy` rows
    /// are skipped without their path being read.
    fn reset<'a>(
        &'a self,
        step: StepId,
        trees: &'a [RunStepTree],
    ) -> IsolatorFuture<'a, ResetReport> {
        Box::pin(async move {
            let label = format!("htui/{step}");
            let mut report = ResetReport::default();
            let mut planned = Vec::new();
            // Every read first (D114): a refusal found at the last row must leave the first one
            // exactly where it was.
            for tree in trees {
                if !matches!(tree.mode, Isolation::SharedSerialized | Isolation::Local) {
                    // OQ-8: the path is not even read, so a vanished tree is not an error.
                    continue;
                }
                let path = PathBuf::from(&tree.path);
                let read = path.clone();
                if blocking(move || git::is_dirty(&read)).await? {
                    // OQ-7: dirty *now* may be the maintainer's own edit.
                    report
                        .refused
                        .push((tree.repo_id, dirty_tree_not_reset(&path)));
                    continue;
                }
                let read = path.clone();
                let head = blocking(move || git::head(&read)).await?;
                if head == tree.base_ref {
                    continue;
                }
                if tree.mode == Isolation::Local {
                    report
                        .refused
                        .push((tree.repo_id, local_moved(&path, &head, &tree.base_ref)));
                    continue;
                }
                // D121: `is_dirty` leaves untracked files out, and `reset --hard` writes the
                // base's blob over one whose path `base_ref` tracks.
                let (read, base) = (path.clone(), tree.base_ref.clone());
                if !blocking(move || git::untracked_paths_base_tracks(&read, &base))
                    .await?
                    .is_empty()
                {
                    report
                        .refused
                        .push((tree.repo_id, dirty_tree_not_reset(&path)));
                    continue;
                }
                let (read, name) = (path.clone(), label.clone());
                match blocking(move || git::branch_target(&read, &name)).await? {
                    Some(target) if target != head => report
                        .refused
                        .push((tree.repo_id, label_conflict(&label, &target, &head))),
                    found => planned.push((tree, path, head, found.is_none())),
                }
            }
            if !report.refused.is_empty() || planned.is_empty() {
                return Ok(report);
            }

            // D40: the probe's refusal answers before the first write, not between two.
            let git = self.cli()?.clone();
            // D138: what a later row's failure must still say was moved.
            let mut done = Vec::new();
            for (tree, path, head, create) in planned {
                if create {
                    let (at, name, target) = (path.clone(), label.clone(), head.clone());
                    git::with_retry("create branch", move || {
                        let (at, name, target) = (at.clone(), name.clone(), target.clone());
                        async move { blocking(move || git::create_branch(&at, &name, &target)).await }
                    })
                    .await
                    .map_err(|err| part_way(err, &done))?;
                }
                report.labelled.push((tree.repo_id, head.clone()));
                git::with_retry("reset --hard", || git.reset_hard(&path, &tree.base_ref))
                    .await
                    .map_err(|err| part_way(err, &done))?;
                done.push((tree.repo_id, path, head, tree.base_ref.clone()));
            }
            Ok(report)
        })
    }

    /// D99: `release_run` alone — no tree is touched.
    fn release<'a>(&'a self, run: RunId) -> IsolatorFuture<'a, ()> {
        Box::pin(async move {
            self.release_run(run);
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

    use htui_core::model::{BoxId, Isolation, RepoId, RunId, RunStepCommit, RunStepTree, StepId};

    use crate::isolate::git::testkit::{
        commit_file, commit_removal, empty_repo, repo_with_one_commit,
    };
    use crate::isolate::{FanoutSlot, Isolator as _, Prepared, ResetReport};

    use super::{
        GixIsolator, IsolatorConfig, RepoCheckout, already_reset, label_conflict, local_moved,
    };

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
                None,
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
            .prepare(
                RunId::new(),
                StepId::new(),
                &[one, two],
                Isolation::Local,
                None,
            )
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
            .prepare(RunId::new(), step, &[id], Isolation::Local, None)
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
            .prepare(
                RunId::new(),
                StepId::new(),
                &[docs, core],
                Isolation::Local,
                None,
            )
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
            .prepare(run, first, &[id], Isolation::SharedSerialized, None)
            .await
            .expect("the first step prepares");
        assert_eq!(prepared.trees[0].before_hash, head);

        let waiting = {
            let isolator = Arc::clone(&isolator);
            let scope = vec![id];
            tokio::spawn(async move {
                isolator
                    .prepare(
                        run,
                        StepId::new(),
                        &scope,
                        Isolation::SharedSerialized,
                        None,
                    )
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
                None,
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
                None,
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
            .prepare(run, crashed, &[id], Isolation::SharedSerialized, None)
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
            isolator.prepare(run, StepId::new(), &[id], Isolation::SharedSerialized, None),
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
            .prepare(run, StepId::new(), &[id], Isolation::SharedSerialized, None)
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
                None,
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
            .prepare(RunId::new(), StepId::new(), &[id], Isolation::Local, None)
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
            .prepare(
                RunId::new(),
                StepId::new(),
                &[id],
                Isolation::Worktree,
                None,
            )
            .await
            .expect_err("the worktree mode needs git");
        assert_eq!(err.to_string(), "isolation refused: git not on PATH");

        isolator
            .prepare(RunId::new(), StepId::new(), &[id], Isolation::Local, None)
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
            .prepare(run, step, &[], Isolation::Local, None)
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
            .prepare(run, step, &[core, docs], Isolation::Worktree, None)
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
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree, None)
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
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree, None)
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
            .prepare(RunId::new(), step, &[core], Isolation::Worktree, None)
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
            .prepare(run, step, &[core], Isolation::Worktree, None)
            .await
            .expect("the first call prepares");
        // The crash window of H-1: the source moves before the retry.
        commit_file(&core_path, "h", "moved\n", "the source moves on");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Worktree, None)
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
            .prepare(run, step, &[core], Isolation::Worktree, None)
            .await
            .expect("the first call prepares");
        let tree = PathBuf::from(&first.trees[0].tree.path);
        std::fs::remove_file(tree.join("f")).expect("a tracked file is deleted");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Worktree, None)
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
            .prepare(run, step, &[core], Isolation::Worktree, None)
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
            .prepare(run, step, &[core], Isolation::Worktree, None)
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
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::Worktree,
                None,
            )
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
            .prepare(run, step, &[core, docs], Isolation::Worktree, None)
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
            .prepare(run, StepId::new(), &[core], Isolation::Worktree, None)
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
            .prepare(run, step, &[core], Isolation::Copy, None)
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
            .prepare(RunId::new(), StepId::new(), &[core], Isolation::Copy, None)
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
            .prepare(RunId::new(), StepId::new(), &[core], Isolation::Copy, None)
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
            .prepare(run, step, &[core], Isolation::Copy, None)
            .await
            .expect("the first call prepares");
        let copy = PathBuf::from(&first.trees[0].tree.path);
        std::fs::write(copy.join("agent-work"), "in progress\n").expect("the agent writes");
        commit_file(&core_path, "h", "moved\n", "the source moves on");

        let second = isolator
            .prepare(run, step, &[core], Isolation::Copy, None)
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
            .prepare(RunId::new(), step, &[core], Isolation::Worktree, None)
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
            .reconcile(step, &rows, &[])
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
            .prepare(RunId::new(), step, &[core], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        let rows = rows(&prepared);
        isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");

        let commits = isolator
            .reconcile(step, &rows, &[])
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
            .reconcile(step, &rows, &[])
            .await
            .expect_err("a dirty primary is refused");
        assert_eq!(err.to_string(), "isolation refused: dirty_primary_tree");
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "and nothing was merged"
        );
    }

    /// Plan D136 (review H4): the primary moved *ahead* of the step's base — another run merged,
    /// or the maintainer committed — so the base is an ancestor of `HEAD` and the step is merged
    /// on top of it rather than refused. The merge's parents are the moved `HEAD` and the tip.
    #[tokio::test]
    async fn reconcile_merges_on_top_of_a_primary_that_moved_ahead() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, _core, core_path, head, after, step, rows) = stepped(dir.path()).await;
        let moved = commit_file(&core_path, "h", "somebody else\n", "the primary moves");

        let commits = isolator
            .reconcile(step, &rows, &[])
            .await
            .expect("a primary ahead of the base is merged into");
        let merge = crate::isolate::git::head(&core_path).expect("the checkout has a HEAD");
        assert_eq!(commits[0].before_hash, head, "the row's base is unchanged");
        assert_eq!(commits[0].after_hash.as_deref(), Some(&*merge));
        assert_eq!(
            crate::isolate::git::head_parents(&core_path).expect("the parents read"),
            vec![moved, after],
            "merged on top of the moved HEAD"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("h")).expect("the file reads"),
            "somebody else\n",
            "the commit it moved to survives"
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
            .reconcile(step, &rows, &[])
            .await
            .expect("the winner reconciles");
        let second = isolator
            .reconcile(step, &rows, &[])
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
            .prepare(RunId::new(), step, &[core], Isolation::Copy, None)
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
            .reconcile(step, &rows, &[])
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

    // ---------------------------------------------------------------------------------------------
    // MOD-4 milestone 4: the fan-out seam (D54, D55, D56, D57, D72, D74).
    // ---------------------------------------------------------------------------------------------

    /// A slot for candidate `index` of a `width`-wide group over `base`.
    fn slot(index: i32, width: i32, base: &BTreeMap<RepoId, String>) -> FanoutSlot<'_> {
        FanoutSlot { index, width, base }
    }

    /// D54(b): the group's base is each repository's `HEAD`, read once, and an empty scope reads
    /// nothing.
    #[tokio::test]
    async fn base_reads_each_repo_s_head() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, core_head) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let docs_head = commit_file(&docs_checkout.local_path, "g", "second\n", "two");
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let base = isolator.base(&[docs, core]).await.expect("both heads read");
        assert_eq!(
            base,
            BTreeMap::from([(core, core_head), (docs, docs_head)]),
            "one entry per repo of the scope, keyed by repo"
        );
        assert!(
            isolator
                .base(&[])
                .await
                .expect("an empty scope reads nothing")
                .is_empty()
        );
        let err = isolator
            .base(&[RepoId::new()])
            .await
            .expect_err("a repo with no checkout is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: no checkout for this repo on this box"
        );
    }

    /// D54(a): every `worktree` candidate starts from the group's base, even when the primary
    /// moved between the base read and the candidate's `prepare`; a scope repo the base does not
    /// name is refused rather than guessed.
    #[tokio::test]
    async fn worktree_candidates_all_start_from_the_slot_base_even_after_the_primary_moved() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let base = isolator.base(&[core]).await.expect("the base reads");
        let moved = commit_file(&core_path, "h", "meanwhile\n", "the primary moves");
        assert_ne!(moved, head);

        let run = RunId::new();
        for index in 0..3 {
            let prepared = isolator
                .prepare(
                    run,
                    StepId::new(),
                    &[core],
                    Isolation::Worktree,
                    Some(slot(index, 3, &base)),
                )
                .await
                .expect("the candidate prepares");
            let tree = PathBuf::from(&prepared.trees[0].tree.path);
            assert_eq!(prepared.trees[0].before_hash, head, "candidate {index}");
            assert_eq!(prepared.trees[0].tree.base_ref, head);
            assert_eq!(
                crate::isolate::git::head(&tree).expect("the tree has a HEAD"),
                head,
                "candidate {index} checked out the base, not the moved primary"
            );
        }

        let empty = BTreeMap::new();
        let err = isolator
            .prepare(
                run,
                StepId::new(),
                &[core],
                Isolation::Worktree,
                Some(slot(0, 3, &empty)),
            )
            .await
            .expect_err("a base with no entry for the repo is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: no fan-out base for core"
        );
    }

    /// D57: each `copy` candidate is its own copy, reset to the group base; the cap counts the
    /// size once per candidate.
    #[tokio::test]
    async fn copy_candidates_reset_to_the_slot_base() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let base = isolator.base(&[core]).await.expect("the base reads");
        commit_file(&core_path, "h", "meanwhile\n", "the primary moves");

        let run = RunId::new();
        let mut paths = Vec::new();
        for index in 0..2 {
            let prepared = isolator
                .prepare(
                    run,
                    StepId::new(),
                    &[core],
                    Isolation::Copy,
                    Some(slot(index, 2, &base)),
                )
                .await
                .expect("the candidate prepares");
            let copy = PathBuf::from(&prepared.trees[0].tree.path);
            assert_eq!(prepared.trees[0].before_hash, head);
            assert_eq!(
                crate::isolate::git::head(&copy).expect("the copy has a HEAD"),
                head,
                "candidate {index} was reset to the base"
            );
            assert!(
                !copy.join("h").exists(),
                "the moved primary's file is not in it"
            );
            paths.push(copy);
        }
        assert_ne!(paths[0], paths[1], "every candidate has its own copy");

        // One copy fits the cap, `width` of them do not.
        let need = crate::isolate::copy::measure(&core_path, &crate::isolate::copy::excludes(&[]))
            .expect("the source measures");
        let mut tight = config(&dir.path().join("trees"), &[]);
        tight.repos = isolator.config.repos.clone();
        tight.copy_max_total_bytes = need * 2;
        let tight = GixIsolator::new(tight).expect("the config validates");
        let err = tight
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::Copy,
                Some(slot(0, 3, &base)),
            )
            .await
            .expect_err("three copies are over the cap");
        assert_eq!(
            err.to_string(),
            format!(
                "isolation refused: copy would need {need} bytes × 3 copies = {}; cap is {}",
                need * 3,
                need * 2
            )
        );
    }

    /// D56: siblings run one after another in the checkout, and each one after the first finds
    /// the previous sibling's commit and is reset to the group base before it starts.
    #[tokio::test]
    async fn shared_serialized_siblings_reset_a_clean_checkout_to_base() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");
        let base = isolator.base(&[core]).await.expect("the base reads");
        let run = RunId::new();

        let first = StepId::new();
        let prepared = isolator
            .prepare(
                run,
                first,
                &[core],
                Isolation::SharedSerialized,
                Some(slot(0, 2, &base)),
            )
            .await
            .expect("sibling 0 prepares");
        assert_eq!(prepared.trees[0].before_hash, head);
        let first_tip = commit_file(&core_path, "g", "sibling 0\n", "sibling 0 commits");
        isolator
            .capture(first, &rows(&prepared))
            .await
            .expect("sibling 0 captures");

        let second = StepId::new();
        let prepared = isolator
            .prepare(
                run,
                second,
                &[core],
                Isolation::SharedSerialized,
                Some(slot(1, 2, &base)),
            )
            .await
            .expect("sibling 1 prepares");
        assert_eq!(prepared.trees[0].before_hash, head);
        assert_eq!(prepared.trees[0].tree.base_ref, head);
        assert!(!prepared.trees[0].tree.dirty);
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the checkout was reset to the base"
        );
        assert!(!core_path.join("g").exists(), "sibling 0's file is gone");
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{first}"))
                .expect("the ref store reads"),
            Some(first_tip),
            "and sibling 0's work survives under its label"
        );
        isolator
            .capture(second, &rows(&prepared))
            .await
            .expect("sibling 1 captures");
    }

    /// D72 (blueprint A-1): with a slot, a dirty checkout is refused for **every** sibling — the
    /// first one too, at `HEAD == base` — and the refusal gives the guard back.
    #[tokio::test]
    async fn shared_serialized_refuses_a_dirty_checkout_with_a_slot() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");
        let base = isolator.base(&[core]).await.expect("the base reads");
        std::fs::write(core_path.join("f"), "the maintainer is mid-edit\n")
            .expect("the checkout is dirtied");

        for index in 0..2 {
            let err = tokio::time::timeout(
                Duration::from_secs(5),
                isolator.prepare(
                    RunId::new(),
                    StepId::new(),
                    &[core],
                    Isolation::SharedSerialized,
                    Some(slot(index, 2, &base)),
                ),
            )
            .await
            .expect("the refused sibling before it held nothing")
            .expect_err("a dirty checkout is refused");
            assert_eq!(
                err.to_string(),
                format!(
                    "isolation refused: dirty_tree_not_reset: {}",
                    core_path.display()
                )
            );
        }
        assert_eq!(
            std::fs::read_to_string(core_path.join("f")).expect("the file reads"),
            "the maintainer is mid-edit\n",
            "nothing was reset"
        );
    }

    /// Plan D137 (review K1), D121's guard on the slot reset: a sibling removed `f` and left the
    /// checkout there, and the maintainer then created an untracked `f`. `is_dirty` does not see
    /// it, but `reset --hard <base>` would write the base's `f` over it, so the next sibling's
    /// `prepare` refuses `dirty_tree_not_reset` and leaves both the file and `HEAD` alone.
    #[tokio::test]
    async fn a_slot_reset_refuses_an_untracked_file_at_a_path_the_base_tracks() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");
        let base = isolator.base(&[core]).await.expect("the base reads");
        let moved = commit_removal(&core_path, "f", "a sibling removes f");
        std::fs::write(core_path.join("f"), "the maintainer's new f\n")
            .expect("an untracked f is written");

        let err = isolator
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::SharedSerialized,
                Some(slot(1, 2, &base)),
            )
            .await
            .expect_err("the untracked file is not overwritten");
        assert_eq!(
            err.to_string(),
            format!(
                "isolation refused: dirty_tree_not_reset: {}",
                core_path.display()
            )
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("f")).expect("the file reads"),
            "the maintainer's new f\n",
            "the untracked file survives"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            moved,
            "and nothing was reset"
        );
    }

    /// D72: every checkout of the scope is read for dirtiness before any is reset, so a dirty
    /// second repository leaves the first — clean, but off the base — exactly where it was.
    #[tokio::test]
    async fn a_dirty_repo_refuses_the_slot_before_any_repo_is_reset() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let (core_path, docs_path) = (
            core_checkout.local_path.clone(),
            docs_checkout.local_path.clone(),
        );
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");
        let base = isolator.base(&[core, docs]).await.expect("the base reads");
        let moved = commit_file(&core_path, "g", "a sibling's work\n", "moved off the base");
        std::fs::write(docs_path.join("f"), "the maintainer is mid-edit\n")
            .expect("the checkout is dirtied");

        let err = isolator
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core, docs],
                Isolation::SharedSerialized,
                Some(slot(1, 2, &base)),
            )
            .await
            .expect_err("a dirty checkout is refused");
        assert_eq!(
            err.to_string(),
            format!(
                "isolation refused: dirty_tree_not_reset: {}",
                docs_path.display()
            )
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            moved,
            "the clean repository ahead of the dirty one was not reset"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the file reads"),
            "a sibling's work\n"
        );
    }

    /// Two siblings that both committed, and a winner that is not the last one: the checked-out
    /// branch moves to the winner's label, and that label is the winner's `after_hash`.
    async fn two_shared_siblings(
        dir: &Path,
    ) -> (
        GixIsolator,
        PathBuf,
        String,
        [(StepId, String, Vec<RunStepTree>); 2],
    ) {
        shared_siblings(dir, [true, true]).await
    }

    /// Two `shared_serialized` siblings of one group, prepared, worked and captured in turn;
    /// sibling `i` commits only when `commits[i]`, and its tip is the base when it does not.
    async fn shared_siblings(
        dir: &Path,
        commits: [bool; 2],
    ) -> (
        GixIsolator,
        PathBuf,
        String,
        [(StepId, String, Vec<RunStepTree>); 2],
    ) {
        let (core, core_checkout, head) = repo(dir, "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator = GixIsolator::new(config(&dir.join("trees"), &[(core, core_checkout)]))
            .expect("the config validates");
        let base = isolator.base(&[core]).await.expect("the base reads");
        let run = RunId::new();
        let mut done = Vec::new();
        for index in 0..2 {
            let step = StepId::new();
            let prepared = isolator
                .prepare(
                    run,
                    step,
                    &[core],
                    Isolation::SharedSerialized,
                    Some(slot(index, 2, &base)),
                )
                .await
                .expect("the sibling prepares");
            let tip = if commits[usize::try_from(index).expect("a small index")] {
                commit_file(
                    &core_path,
                    "g",
                    &format!("sibling {index}\n"),
                    "the sibling commits",
                )
            } else {
                head.clone()
            };
            let rows = rows(&prepared);
            isolator
                .capture(step, &rows)
                .await
                .expect("the sibling captures");
            done.push((step, tip, rows));
        }
        let [zero, one]: [_; 2] = done.try_into().expect("two siblings");
        (isolator, core_path, head, [zero, one])
    }

    /// D56: `HEAD` at the last sibling's label → reset to the winner's label.
    #[tokio::test]
    async fn shared_serialized_reconcile_moves_the_branch_to_the_winner() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core_path, head, [zero, one]) = two_shared_siblings(dir.path()).await;
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            one.1,
            "the last sibling left the checkout at its own tip"
        );

        let commits = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect("the winner reconciles");
        assert_eq!(commits[0].before_hash, head);
        assert_eq!(commits[0].after_hash.as_deref(), Some(&*zero.1));
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            zero.1,
            "the checked-out branch moved to the winner's label"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the file reads"),
            "sibling 0\n"
        );

        // Idempotent: the checkout is now at the winner's label, which is M3's identity case.
        let again = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect("a second reconcile is the identity");
        assert_eq!(again, commits);
    }

    /// D56: the last sibling committed nothing, so `HEAD` is the group base — which no sibling
    /// label names — and the move to an earlier winner's label is still taken.
    #[tokio::test]
    async fn shared_serialized_reconcile_moves_from_the_base_to_an_earlier_winner() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core_path, head, [zero, one]) =
            shared_siblings(dir.path(), [true, false]).await;
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the last sibling left the checkout at the base"
        );

        let commits = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect("the winner reconciles");
        assert_eq!(commits[0].before_hash, head);
        assert_eq!(commits[0].after_hash.as_deref(), Some(&*zero.1));
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            zero.1,
            "the checked-out branch moved to the winner's label"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the file reads"),
            "sibling 0\n"
        );
    }

    /// D56: a winner that committed nothing has no label, so the checkout a later sibling moved
    /// goes back to the group base and the winner's answer is `None`.
    #[tokio::test]
    async fn shared_serialized_reconcile_of_an_empty_winner_resets_to_the_base() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core_path, head, [zero, one]) =
            shared_siblings(dir.path(), [false, true]).await;
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            one.1,
            "the last sibling left the checkout at its own tip"
        );

        let commits = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect("the winner reconciles");
        assert_eq!(commits[0].before_hash, head);
        assert_eq!(commits[0].after_hash, None);
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the checked-out branch went back to the base"
        );
        assert!(!core_path.join("g").exists(), "sibling 1's file is gone");
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{}", one.0))
                .expect("the ref store reads"),
            Some(one.1),
            "and the loser's work survives under its label"
        );
    }

    /// D56: a `HEAD` that is neither the base nor any sibling's label was moved by somebody else,
    /// and a dirty checkout is not reset either.
    #[tokio::test]
    async fn shared_serialized_reconcile_refuses_a_head_no_sibling_left() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core_path, _head, [zero, one]) = two_shared_siblings(dir.path()).await;

        std::fs::write(core_path.join("g"), "mid-edit\n").expect("the checkout is dirtied");
        let err = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect_err("a dirty checkout is not reset");
        assert_eq!(err.to_string(), "isolation refused: dirty_primary_tree");
        std::fs::write(core_path.join("g"), "sibling 1\n").expect("the edit is undone");

        let moved = commit_file(&core_path, "h", "somebody else\n", "the checkout moves");
        let err = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect_err("a head no sibling left is refused");
        assert_eq!(
            err.to_string(),
            format!("isolation refused: primary_moved: {moved}")
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            moved,
            "and nothing was reset"
        );
    }

    /// Plan D137 (review K1), D121's guard on the in-place reconcile: sibling 0 won with `g`,
    /// sibling 1 committed nothing and left the checkout at the base, and the maintainer then
    /// created an untracked `g`. Moving the branch to the winner's label would write the label's
    /// `g` over it, so the reconcile refuses `dirty_tree_not_reset` and moves nothing.
    #[tokio::test]
    async fn shared_serialized_reconcile_refuses_an_untracked_file_the_winners_label_tracks() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core_path, head, [zero, one]) =
            shared_siblings(dir.path(), [true, false]).await;
        std::fs::write(core_path.join("g"), "the maintainer's own g\n")
            .expect("an untracked g is written");

        let err = isolator
            .reconcile(zero.0, &zero.2, &[one.0])
            .await
            .expect_err("the untracked file is not overwritten");
        assert_eq!(
            err.to_string(),
            format!(
                "isolation refused: dirty_tree_not_reset: {}",
                core_path.display()
            )
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("g")).expect("the file reads"),
            "the maintainer's own g\n",
            "the untracked file survives"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "and the branch did not move"
        );
    }

    /// D54(c): several committed repos give one range per repo, `<name>:<before>..<after>`, and
    /// their stats and patches under a `# repo <name>` line each, in the order of the rows.
    #[tokio::test]
    async fn diff_spans_every_repo_under_a_header_line() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let paths = BTreeMap::from([
            (core, core_checkout.local_path.clone()),
            (docs, docs_checkout.local_path.clone()),
        ]);
        let names = BTreeMap::from([(core, "core"), (docs, "docs")]);
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        for tree in &prepared.trees {
            commit_file(Path::new(&tree.tree.path), "g", "the step's work\n", "work");
        }
        let rows = rows(&prepared);
        let commits = isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");

        let block = isolator
            .diff(&rows, &commits)
            .await
            .expect("the diff reads")
            .expect("both repos committed");
        let mut range = Vec::new();
        let (mut stat, mut patch) = (String::new(), String::new());
        for commit in &commits {
            let name = names[&commit.repo_id];
            let after = commit.after_hash.as_deref().expect("committed");
            let repo = &paths[&commit.repo_id];
            range.push(format!("{name}:{}..{after}", commit.before_hash));
            stat.push_str(&format!(
                "# repo {name}\n{}",
                git.diff(repo, &commit.before_hash, after, true)
                    .await
                    .expect("the stat reads")
            ));
            patch.push_str(&format!(
                "# repo {name}\n{}",
                git.diff(repo, &commit.before_hash, after, false)
                    .await
                    .expect("the patch reads")
            ));
        }
        assert_eq!(block.range, range.join(", "));
        assert_eq!(block.stat, stat);
        assert_eq!(block.diff, patch);
        assert!(block.diff.starts_with("# repo "), "{}", block.diff);

        // One committed repo is its bare range, with no header.
        let one = &commits[..1];
        let single = isolator
            .diff(&rows, one)
            .await
            .expect("the diff reads")
            .expect("one repo committed");
        assert_eq!(
            single.range,
            format!(
                "{}..{}",
                one[0].before_hash,
                one[0].after_hash.as_deref().expect("committed")
            )
        );
        assert!(
            single.diff.starts_with("diff --git a/g b/g\n"),
            "{}",
            single.diff
        );
    }

    /// D54(c) with D55's cap: a patch cut at 64 KiB ends in the truncation marker with no newline,
    /// and the next repository's `# repo <name>` header still starts a line of its own.
    #[tokio::test]
    async fn a_truncated_patch_leaves_the_next_repo_header_on_its_own_line() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core, docs], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        let big: String = (0..4096)
            .map(|line| format!("line {line} of a patch well past the cap\n"))
            .collect();
        assert!(big.len() > crate::isolate::git::DIFF_CAP);
        for tree in &prepared.trees {
            commit_file(Path::new(&tree.tree.path), "g", &big, "work");
        }
        let rows = rows(&prepared);
        let commits = isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");

        let block = isolator
            .diff(&rows, &commits)
            .await
            .expect("the diff reads")
            .expect("both repos committed");
        assert!(
            block.diff.contains("[diff truncated at 64 KiB]"),
            "the first patch overflowed the cap"
        );
        for text in [&block.stat, &block.diff] {
            let headers: Vec<usize> = text.match_indices("# repo ").map(|(at, _)| at).collect();
            assert_eq!(headers.len(), 2, "one header per repo");
            for at in headers {
                assert!(
                    at == 0 || text.as_bytes()[at - 1] == b'\n',
                    "a header shares a line with what precedes it: {:?}",
                    &text[at.saturating_sub(40)..at + 12]
                );
            }
        }
    }

    /// D55: no committed row, or no usable `git`, is `None` — the diff is advisory.
    #[tokio::test]
    async fn diff_is_none_without_git() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let rows = vec![RunStepTree {
            run_step_id: StepId::new(),
            repo_id: core,
            mode: Isolation::Worktree,
            path: dir.path().join("nowhere").to_string_lossy().into_owned(),
            base_ref: head.clone(),
            dirty: false,
        }];
        let committed = vec![RunStepCommit {
            run_step_id: rows[0].run_step_id,
            repo_id: core,
            before_hash: head.clone(),
            after_hash: Some(commit_file(&core_checkout.local_path, "g", "x\n", "x")),
        }];
        let without = GixIsolator::with_git(
            config(&dir.path().join("trees"), &[(core, core_checkout.clone())]),
            Err("git not on PATH".to_owned()),
        )
        .expect("the config validates");
        assert_eq!(
            without
                .diff(&rows, &committed)
                .await
                .expect("no git is not an error"),
            None
        );

        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let with = GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
            .expect("the config validates");
        let nothing = vec![RunStepCommit {
            after_hash: None,
            ..committed[0].clone()
        }];
        assert_eq!(with.diff(&rows, &nothing).await.expect("reads"), None);
        assert_eq!(with.diff(&rows, &[]).await.expect("reads"), None);
        let unmoved = vec![RunStepCommit {
            after_hash: Some(head),
            ..committed[0].clone()
        }];
        assert_eq!(with.diff(&rows, &unmoved).await.expect("reads"), None);
    }

    /// D74 (blueprint A-3): a `copy` winner's post-reconcile `after_hash` is the merge commit,
    /// which only the primary holds — the diff reads the copy before reconcile and the checkout
    /// after.
    #[tokio::test]
    async fn copy_diff_after_reconcile_reads_the_checkout() {
        let Some(git) = crate::skip_without_git!() else {
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
            .prepare(RunId::new(), step, &[core], Isolation::Copy, None)
            .await
            .expect("the copy mode prepares");
        let copy = PathBuf::from(&prepared.trees[0].tree.path);
        let after = commit_file(&copy, "g", "the step's work\n", "the step commits");
        let rows = rows(&prepared);
        let captured = isolator
            .capture(step, &rows)
            .await
            .expect("the step captures");

        let before_merge = isolator
            .diff(&rows, &captured)
            .await
            .expect("the copy holds the range")
            .expect("the step committed");
        assert_eq!(before_merge.range, format!("{head}..{after}"));
        assert_eq!(
            before_merge.diff,
            git.diff(&copy, &head, &after, false)
                .await
                .expect("the patch reads")
        );

        let reconciled = isolator
            .reconcile(step, &rows, &[])
            .await
            .expect("the copy reconciles");
        let merge = reconciled[0].after_hash.clone().expect("a merge commit");
        assert!(!crate::isolate::git::has_commit(&copy, &merge).expect("reads"));
        assert!(crate::isolate::git::has_commit(&core_path, &merge).expect("reads"));
        let after_merge = isolator
            .diff(&rows, &reconciled)
            .await
            .expect("the checkout holds the merge")
            .expect("the step committed");
        assert_eq!(after_merge.range, format!("{head}..{merge}"));
        assert_eq!(
            after_merge.diff, before_merge.diff,
            "before..merge carries the same patch as before..after"
        );
    }

    /// Every M3 path is what `None` reaches: a no-slot `shared_serialized` prepare still records a
    /// dirty checkout at its `HEAD` rather than refusing it, and `local` refuses a slot outright.
    #[tokio::test]
    async fn a_no_slot_prepare_is_unchanged() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");
        let base = isolator.base(&[core]).await.expect("the base reads");
        let moved = commit_file(&core_path, "g", "second\n", "two");
        std::fs::write(core_path.join("f"), "mid-edit\n").expect("the checkout is dirtied");

        let step = StepId::new();
        let prepared = isolator
            .prepare(
                RunId::new(),
                step,
                &[core],
                Isolation::SharedSerialized,
                None,
            )
            .await
            .expect("M3 records a dirty checkout rather than refusing it");
        assert_eq!(prepared.trees[0].before_hash, moved, "the HEAD, not a base");
        assert!(prepared.trees[0].tree.dirty);
        isolator
            .capture(step, &rows(&prepared))
            .await
            .expect("the step captures");

        let err = isolator
            .prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::Local,
                Some(slot(0, 2, &base)),
            )
            .await
            .expect_err("local never fans out");
        assert_eq!(
            err.to_string(),
            "isolation refused: local isolation cannot fan out"
        );
    }

    /// D70 and D73: 24 slotted `worktree` prepares of distinct steps on one repository, under a
    /// multi-thread runtime, all succeed — and the per-repository admin lock never lets two
    /// `worktree add`/`remove` children run at once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_worktree_prepares_on_one_repo_never_overlap_their_adds() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let isolator = Arc::new(
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates"),
        );
        let base = Arc::new(isolator.base(&[core]).await.expect("the base reads"));
        let run = RunId::new();

        let tasks: Vec<_> = (0..24)
            .map(|index| {
                let (isolator, base) = (Arc::clone(&isolator), Arc::clone(&base));
                tokio::spawn(async move {
                    let prepared = isolator
                        .prepare(
                            run,
                            StepId::new(),
                            &[core],
                            Isolation::Worktree,
                            Some(slot(index, 24, &base)),
                        )
                        .await?;
                    Ok::<_, crate::isolate::IsolateError>(prepared.trees[0].before_hash.clone())
                })
            })
            .collect();
        for task in tasks {
            let before = task
                .await
                .expect("the task did not panic")
                .expect("every candidate prepares");
            assert_eq!(before, head);
        }
        assert_eq!(
            isolator.max_admin_in_flight(),
            1,
            "the adds ran, one at a time"
        );
    }

    /// D70: the admin lock is per repository — a held lock on one repository does not hold up a
    /// `worktree add` on another, and does hold up one on itself.
    #[tokio::test]
    async fn adds_on_two_repositories_do_not_wait_for_each_other() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let isolator = Arc::new(
            GixIsolator::new(config(
                &dir.path().join("trees"),
                &[(core, core_checkout), (docs, docs_checkout)],
            ))
            .expect("the config validates"),
        );
        let run = RunId::new();

        for (held, free) in [(core, docs), (docs, core)] {
            let guard = isolator.admin_guard(held).await;
            tokio::time::timeout(
                Duration::from_secs(10),
                isolator.prepare(run, StepId::new(), &[free], Isolation::Worktree, None),
            )
            .await
            .expect("the other repository's add does not wait")
            .expect("the other repository prepares");

            let mut blocked = {
                let isolator = Arc::clone(&isolator);
                tokio::spawn(async move {
                    isolator
                        .prepare(run, StepId::new(), &[held], Isolation::Worktree, None)
                        .await
                        .map(|prepared| prepared.trees.len())
                })
            };
            assert!(
                tokio::time::timeout(Duration::from_millis(200), &mut blocked)
                    .await
                    .is_err(),
                "an add on the held repository waits for its lock"
            );
            drop(guard);
            let prepared = tokio::time::timeout(Duration::from_secs(10), blocked)
                .await
                .expect("the add proceeds once the lock is free")
                .expect("the task did not panic")
                .expect("the held repository prepares");
            assert_eq!(prepared, 1);
        }
    }

    /// D73 (blueprint A-2): every `git worktree remove` takes the admin lock too — D27's removal of
    /// a clean, empty tree at `capture`, and `cleanup`'s removal of every tree.
    #[tokio::test]
    async fn worktree_removals_wait_for_the_admin_lock() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let isolator = Arc::new(
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates"),
        );
        let run = RunId::new();

        // `capture` of a step that committed nothing removes its tree.
        let step = StepId::new();
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        let captured_rows = rows(&prepared);
        let tree = PathBuf::from(&captured_rows[0].path);
        let guard = isolator.admin_guard(core).await;
        let mut capture = {
            let (isolator, rows) = (Arc::clone(&isolator), captured_rows.clone());
            tokio::spawn(async move { isolator.capture(step, &rows).await })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut capture)
                .await
                .is_err(),
            "capture's removal waits for the lock"
        );
        assert!(tree.exists(), "and the tree is still there");
        drop(guard);
        let commits = tokio::time::timeout(Duration::from_secs(10), capture)
            .await
            .expect("the removal proceeds once the lock is free")
            .expect("the task did not panic")
            .expect("the step captures");
        assert_eq!(commits[0].after_hash, None);
        assert!(!tree.exists(), "the empty tree was removed");

        // `cleanup` removes a tree that is still there.
        let kept = isolator
            .prepare(run, StepId::new(), &[core], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        let kept_rows = rows(&kept);
        let tree = PathBuf::from(&kept_rows[0].path);
        let guard = isolator.admin_guard(core).await;
        let mut cleanup = {
            let isolator = Arc::clone(&isolator);
            tokio::spawn(async move { isolator.cleanup(run, &kept_rows).await })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut cleanup)
                .await
                .is_err(),
            "cleanup's removal waits for the lock"
        );
        assert!(tree.exists(), "and the tree is still there");
        drop(guard);
        tokio::time::timeout(Duration::from_secs(10), cleanup)
            .await
            .expect("the removal proceeds once the lock is free")
            .expect("the task did not panic")
            .expect("the run cleans up");
        assert!(!tree.exists(), "the tree was removed");
    }

    /// Plan D136: an isolated `reconcile` holds the repository's admin lock from its `HEAD` read
    /// through the merge and its post-condition, so two runs of one process reconciling the same
    /// repository cannot interleave a merge between the other's read and its merge.
    #[tokio::test]
    async fn an_isolated_reconcile_waits_for_the_admin_lock() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, core, core_path, head, _, step, rows) = stepped(dir.path()).await;
        let isolator = Arc::new(isolator);

        let guard = isolator.admin_guard(core).await;
        let mut reconcile = {
            let isolator = Arc::clone(&isolator);
            tokio::spawn(async move { isolator.reconcile(step, &rows, &[]).await })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut reconcile)
                .await
                .is_err(),
            "the reconcile waits for the lock"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "and nothing was merged"
        );
        drop(guard);
        let commits = tokio::time::timeout(Duration::from_secs(10), reconcile)
            .await
            .expect("the reconcile proceeds once the lock is free")
            .expect("the task did not panic")
            .expect("the winner reconciles");
        assert_eq!(
            commits[0].after_hash,
            Some(crate::isolate::git::head(&core_path).expect("the checkout has a HEAD")),
            "the merge landed"
        );
    }

    /// Plan D136: the admin lock is not only awaited but *held* by an isolated `reconcile` at its
    /// `HEAD` read and still after the merge's post-condition, so taking and dropping it at once,
    /// or taking it only after the read, is caught here and not left to a race.
    #[tokio::test]
    async fn an_isolated_reconcile_holds_the_admin_lock_from_its_head_read_through_its_merge() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (isolator, _core, core_path, _head, _, step, rows) = stepped(dir.path()).await;

        let commits = isolator
            .reconcile(step, &rows, &[])
            .await
            .expect("the winner reconciles");
        assert_eq!(
            commits[0].after_hash,
            Some(crate::isolate::git::head(&core_path).expect("the checkout has a HEAD")),
            "the merge landed"
        );
        assert_eq!(
            isolator.reconcile_admin_probes(),
            (2, 0),
            "the lock was held at the HEAD read and after the post-condition"
        );
    }

    /// Every `refs/heads/htui/*` of the repository at `path`, with its target, in name order.
    fn htui_refs(path: &Path) -> Vec<(String, String)> {
        let repository = gix::open(path).expect("the repository opens");
        let platform = repository.references().expect("the ref store reads");
        let mut refs = platform
            .local_branches()
            .expect("the branches list")
            .map(|reference| {
                let mut reference = reference.expect("a branch reads");
                let name = reference.name().as_bstr().to_string();
                let target = reference
                    .peel_to_id()
                    .expect("a branch peels")
                    .detach()
                    .to_hex()
                    .to_string();
                (name, target)
            })
            .filter(|(name, _)| name.starts_with("refs/heads/htui/"))
            .collect::<Vec<_>>();
        refs.sort();
        refs
    }

    /// An isolator that could never spawn `git`: what a git-free case runs against, so a pass
    /// proves the path it took needs no binary.
    fn without_git(root: &Path, checkouts: &[(RepoId, RepoCheckout)]) -> GixIsolator {
        GixIsolator::with_git(
            config(root, checkouts),
            Err("no git on this box".to_owned()),
        )
        .expect("the config validates")
    }

    /// OQ-8: a `worktree` or `copy` retry prepares a tree of its own, so `reset` touches neither —
    /// not the agent's commit, not the branch, not a file, not a ref.
    #[tokio::test]
    async fn reset_leaves_worktree_and_copy_trees_untouched() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
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

        let (run, step) = (RunId::new(), StepId::new());
        let worktree = isolator
            .prepare(run, step, &[core], Isolation::Worktree, None)
            .await
            .expect("the worktree mode prepares");
        let copy = isolator
            .prepare(run, step, &[docs], Isolation::Copy, None)
            .await
            .expect("the copy mode prepares");
        let mut trees = rows(&worktree);
        trees.extend(rows(&copy));
        let worktree_path = PathBuf::from(&trees[0].path);
        let copy_path = PathBuf::from(&trees[1].path);
        let worktree_tip = commit_file(&worktree_path, "g", "agent\n", "the agent commits");
        let copy_tip = commit_file(&copy_path, "g", "agent\n", "the agent commits");
        std::fs::write(worktree_path.join("f"), "unfinished\n").expect("the tree is edited");

        let before = [
            htui_refs(&core_path),
            htui_refs(&docs_path),
            htui_refs(&copy_path),
        ];
        let report = isolator.reset(step, &trees).await.expect("reset answers");

        assert_eq!(report, ResetReport::default());
        assert_eq!(
            crate::isolate::git::head(&worktree_path).expect("the worktree has a HEAD"),
            worktree_tip
        );
        assert_eq!(
            crate::isolate::git::head(&copy_path).expect("the copy has a HEAD"),
            copy_tip
        );
        assert_eq!(
            std::fs::read_to_string(worktree_path.join("f")).expect("the file reads"),
            "unfinished\n",
            "the agent's edit survives"
        );
        assert_eq!(
            std::fs::read_to_string(copy_path.join("g")).expect("the file reads"),
            "agent\n"
        );
        assert_eq!(
            [
                htui_refs(&core_path),
                htui_refs(&docs_path),
                htui_refs(&copy_path),
            ],
            before,
            "no `htui/` ref was added or moved"
        );
    }

    /// D92: a clean `shared_serialized` checkout the agent committed on is labelled at its `HEAD`
    /// first and only then reset to the step's `base_ref`, so the commit stays reachable.
    #[tokio::test]
    async fn reset_labels_then_resets_a_clean_shared_checkout() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");

        let report = isolator
            .reset(step, &rows(&prepared))
            .await
            .expect("reset answers");

        assert_eq!(
            report,
            ResetReport {
                labelled: vec![(core, commit.clone())],
                refused: Vec::new(),
            }
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the checkout is back at base_ref"
        );
        assert!(!core_path.join("g").exists(), "the agent's file is gone");
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{step}"))
                .expect("the ref store reads"),
            Some(commit.clone()),
            "the label names the agent's commit"
        );
        assert!(
            crate::isolate::git::testkit::has_object(&core_path, &commit),
            "and the commit is still there to be read"
        );
        isolator.release(run).await.expect("release answers");
    }

    /// D121: the agent deleted a file `base_ref` tracks and an untracked file now sits at that
    /// path. `is_dirty` reads the checkout as clean, but `reset --hard <base_ref>` would write the
    /// base's blob over the untracked file — so the row is refused and nothing is written.
    #[tokio::test]
    async fn reset_refuses_an_untracked_file_at_a_path_base_ref_tracks() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let trees = rows(&prepared);
        let commit = commit_removal(&core_path, "f", "the agent deletes f");
        std::fs::write(core_path.join("f"), "the maintainer's notes\n")
            .expect("an untracked file is written at the deleted path");
        assert!(
            !crate::isolate::git::is_dirty(&core_path).expect("the status reads"),
            "the checkout reads as clean to is_dirty"
        );

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        assert_eq!(
            report,
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(
                    core,
                    format!(
                        "dirty_tree_not_reset: {}",
                        Path::new(&trees[0].path).display()
                    )
                )],
            }
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("f")).expect("the file reads"),
            "the maintainer's notes\n",
            "the untracked file survives byte for byte"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            commit,
            "HEAD is unmoved"
        );
        assert!(htui_refs(&core_path).is_empty(), "no label was written");
        isolator.release(run).await.expect("release answers");
    }

    /// D121's other half: an untracked file at a path `base_ref` does not track survives
    /// `reset --hard`, so it refuses nothing and is still there afterwards.
    #[tokio::test]
    async fn reset_keeps_an_untracked_file_at_a_path_base_ref_does_not_track() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");
        std::fs::write(core_path.join("notes"), "the maintainer's notes\n")
            .expect("an untracked file is written");

        let report = isolator
            .reset(step, &rows(&prepared))
            .await
            .expect("reset answers");

        assert_eq!(
            report,
            ResetReport {
                labelled: vec![(core, commit)],
                refused: Vec::new(),
            }
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head,
            "the checkout is back at base_ref"
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("notes")).expect("the file reads"),
            "the maintainer's notes\n",
            "the untracked file survives the reset"
        );
        isolator.release(run).await.expect("release answers");
    }

    /// Blueprint F-T: `capture` already wrote `htui/<step>` at `HEAD` (D26), and `create_branch` is
    /// `MustNotExist` — `reset` finds the label rather than failing on it.
    #[tokio::test]
    async fn reset_finds_a_label_capture_already_wrote() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");
        let trees = rows(&prepared);
        isolator
            .capture(step, &trees)
            .await
            .expect("the step captures, writing the label");

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        assert_eq!(report.labelled, vec![(core, commit.clone())]);
        assert!(report.refused.is_empty(), "{:?}", report.refused);
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head
        );
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &format!("htui/{step}"))
                .expect("the ref store reads"),
            Some(commit)
        );
    }

    /// D114: a label that already names some other commit is a refusal, and nothing is written —
    /// not the label, not the reset.
    #[tokio::test]
    async fn reset_refuses_a_label_that_names_another_commit() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let label = format!("htui/{step}");
        crate::isolate::git::create_branch(&core_path, &label, &head)
            .expect("the label is pre-created at base_ref");
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");

        let report = isolator
            .reset(step, &rows(&prepared))
            .await
            .expect("reset answers");

        assert_eq!(
            report,
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(core, label_conflict(&label, &head, &commit))],
            }
        );
        assert_eq!(
            label_conflict(&label, &head, &commit),
            format!("label_conflict: {label} names {head}, HEAD is {commit}")
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            commit,
            "HEAD is unmoved"
        );
        assert_eq!(
            crate::isolate::git::branch_target(&core_path, &label).expect("the ref store reads"),
            Some(head),
            "and the label is where it was"
        );
        isolator.release(run).await.expect("release answers");
    }

    /// D92: a checkout still at its `base_ref` has nothing to put back and nothing to label.
    #[tokio::test]
    async fn reset_of_a_shared_checkout_at_before_hash_writes_no_label() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator =
            GixIsolator::new(config(&dir.path().join("trees"), &[(core, core_checkout)]))
                .expect("the config validates");

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");

        let report = isolator
            .reset(step, &rows(&prepared))
            .await
            .expect("reset answers");

        assert_eq!(report, ResetReport::default());
        assert!(
            htui_refs(&core_path).is_empty(),
            "no `htui/` ref was written"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head
        );
        isolator.release(run).await.expect("release answers");
    }

    /// OQ-7: a `local` checkout that is dirty **now** may hold the maintainer's edits, and is
    /// refused before anything is read further — no `git` binary needed to say so.
    #[tokio::test]
    async fn reset_refuses_a_live_dirty_local_checkout_and_touches_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator = without_git(&dir.path().join("trees"), &[(core, core_checkout)]);

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Local, None)
            .await
            .expect("local prepares");
        let trees = rows(&prepared);
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");
        std::fs::write(core_path.join("f"), "the maintainer is mid-edit\n")
            .expect("the checkout is dirtied");

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        assert_eq!(
            report,
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(
                    core,
                    format!(
                        "dirty_tree_not_reset: {}",
                        Path::new(&trees[0].path).display()
                    )
                )],
            }
        );
        assert_eq!(
            std::fs::read_to_string(core_path.join("f")).expect("the file reads"),
            "the maintainer is mid-edit\n",
            "the edit survives byte for byte"
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            commit,
            "HEAD is unmoved"
        );
        assert!(htui_refs(&core_path).is_empty(), "no label was written");
    }

    /// Blueprint A-6: a clean `local` checkout whose `HEAD` moved is the maintainer's own branch,
    /// and `reset --hard` would move it backwards under them — refused, and nothing written.
    #[tokio::test]
    async fn reset_refuses_a_local_checkout_whose_head_moved() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator = without_git(&dir.path().join("trees"), &[(core, core_checkout)]);

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Local, None)
            .await
            .expect("local prepares");
        let trees = rows(&prepared);
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        let path = PathBuf::from(&trees[0].path);
        assert_eq!(
            report,
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(core, local_moved(&path, &commit, &head))],
            }
        );
        assert_eq!(
            local_moved(&path, &commit, &head),
            format!(
                "local_moved: {} at {commit}, before_hash {head}",
                path.display()
            )
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            commit,
            "HEAD is unmoved"
        );
        assert!(htui_refs(&core_path).is_empty(), "no label was written");
    }

    /// A clean `local` checkout still at its `base_ref` is vacuously reset: nothing to do, and no
    /// binary needed to find that out.
    #[tokio::test]
    async fn reset_of_a_local_checkout_at_before_hash_is_empty() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, head) = repo(dir.path(), "core", true);
        let core_path = core_checkout.local_path.clone();
        let isolator = without_git(&dir.path().join("trees"), &[(core, core_checkout)]);

        let step = StepId::new();
        let prepared = isolator
            .prepare(RunId::new(), step, &[core], Isolation::Local, None)
            .await
            .expect("local prepares");

        let report = isolator
            .reset(step, &rows(&prepared))
            .await
            .expect("reset answers");

        assert_eq!(report, ResetReport::default());
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            head
        );
    }

    /// D92's all-or-nothing: one dirty repository refuses the whole reset, so the clean, moved one
    /// beside it is neither labelled nor reset.
    #[tokio::test]
    async fn reset_is_all_or_nothing_across_repos() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
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

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core, docs], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let trees = rows(&prepared);
        let commit = commit_file(&core_path, "g", "agent\n", "the agent commits");
        std::fs::write(docs_path.join("f"), "uncommitted\n").expect("docs is dirtied");

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        let docs_row = trees
            .iter()
            .find(|row| row.repo_id == docs)
            .expect("docs has a row");
        assert_eq!(
            report,
            ResetReport {
                labelled: Vec::new(),
                refused: vec![(
                    docs,
                    format!(
                        "dirty_tree_not_reset: {}",
                        Path::new(&docs_row.path).display()
                    )
                )],
            }
        );
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            commit,
            "the clean repository was not reset"
        );
        assert!(htui_refs(&core_path).is_empty(), "nor labelled");
        assert_eq!(
            std::fs::read_to_string(docs_path.join("f")).expect("the file reads"),
            "uncommitted\n"
        );
        isolator.release(run).await.expect("release answers");
    }

    /// Plan D138's sentence, byte for byte: one `(repo, path, head, base)` per row already reset.
    #[test]
    fn already_reset_names_every_row_and_where_it_was() {
        let (a, b) = (RepoId::new(), RepoId::new());
        assert_eq!(
            already_reset(&[
                (a, Path::new("/src/core"), "h1", "b1"),
                (b, Path::new("/src/docs"), "h2", "b2"),
            ]),
            format!("already reset: {a} /src/core from h1 to b1, {b} /src/docs from h2 to b2")
        );
    }

    /// Plan D138 (review L1): `core` is labelled and reset, then `docs`'s `reset --hard` fails on
    /// a held `index.lock`. The error still fails the call — the engine takes D93's path with its
    /// text — but it now names `core` as already reset, from its commit to its base, rather than
    /// letting the note say no tree was touched.
    #[tokio::test]
    async fn a_reset_that_fails_part_way_names_the_rows_already_reset() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
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

        let (run, step) = (RunId::new(), StepId::new());
        let prepared = isolator
            .prepare(run, step, &[core, docs], Isolation::SharedSerialized, None)
            .await
            .expect("the step prepares");
        let trees = rows(&prepared);
        let core_commit = commit_file(&core_path, "g", "agent\n", "the agent commits");
        let docs_commit = commit_file(&docs_path, "g", "agent\n", "the agent commits");
        assert_eq!(trees[0].repo_id, core, "core's row is reset first");
        let lock = docs_path.join(".git").join("index.lock");
        std::fs::write(&lock, "").expect("docs's index is held");

        let err = isolator
            .reset(step, &trees)
            .await
            .expect_err("docs's reset --hard fails");
        std::fs::remove_file(&lock).expect("the index is let go");

        let text = err.to_string();
        let named = already_reset(&[(
            core,
            Path::new(&trees[0].path),
            &core_commit,
            &trees[0].base_ref,
        )]);
        assert!(text.starts_with("git: "), "still a git failure: {text}");
        assert!(text.contains("index.lock"), "the cause is kept: {text}");
        assert!(text.ends_with(&format!("; {named}")), "{text}");
        assert_eq!(
            crate::isolate::git::head(&core_path).expect("the checkout has a HEAD"),
            trees[0].base_ref,
            "core was reset"
        );
        assert_eq!(
            crate::isolate::git::head(&docs_path).expect("the checkout has a HEAD"),
            docs_commit,
            "docs was not"
        );
        isolator.release(run).await.expect("release answers");
    }

    /// OQ-8, scoped: `reset` never reads a `worktree`/`copy` row's path, so a tree that vanished —
    /// or a repository this box has no checkout of — is not an error.
    #[tokio::test]
    async fn reset_never_reads_a_worktree_or_copy_path() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let isolator = without_git(&dir.path().join("trees"), &[]);
        let step = StepId::new();
        let trees = [Isolation::Worktree, Isolation::Copy].map(|mode| RunStepTree {
            run_step_id: step,
            repo_id: RepoId::new(),
            mode,
            path: dir
                .path()
                .join("vanished")
                .join(format!("{mode:?}"))
                .to_string_lossy()
                .into_owned(),
            base_ref: "0".repeat(40),
            dirty: false,
        });

        let report = isolator.reset(step, &trees).await.expect("reset answers");

        assert_eq!(report, ResetReport::default());
    }

    /// D99: `release(run)` gives back every guard the run's steps hold and removes nothing — the
    /// adopter reads those trees.
    #[tokio::test]
    async fn release_frees_a_shared_guard_without_removing_a_tree() {
        let Some(_git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let (core, core_checkout, _) = repo(dir.path(), "core", true);
        let (docs, docs_checkout, _) = repo(dir.path(), "docs", false);
        let core_path = core_checkout.local_path.clone();
        let isolator = GixIsolator::new(config(
            &dir.path().join("trees"),
            &[(core, core_checkout), (docs, docs_checkout)],
        ))
        .expect("the config validates");

        let first = RunId::new();
        isolator
            .prepare(
                first,
                StepId::new(),
                &[core],
                Isolation::SharedSerialized,
                None,
            )
            .await
            .expect("the shared step prepares");
        let worktree = isolator
            .prepare(first, StepId::new(), &[docs], Isolation::Worktree, None)
            .await
            .expect("the worktree step prepares");
        let worktree_path = PathBuf::from(&worktree.trees[0].tree.path);

        isolator.release(first).await.expect("release answers");

        tokio::time::timeout(
            Duration::from_secs(5),
            isolator.prepare(
                RunId::new(),
                StepId::new(),
                &[core],
                Isolation::SharedSerialized,
                None,
            ),
        )
        .await
        .expect("the released guard admits the next run")
        .expect("the next run prepares");
        assert!(core_path.join("f").exists(), "the checkout is untouched");
        assert!(
            worktree_path.join(".git").exists(),
            "the worktree tree is still there"
        );
    }
}
