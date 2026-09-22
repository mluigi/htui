//! Every git fact this crate knows, and the only place it spawns a process.
//!
//! Two halves, one file, because they answer the same question from two directions (plan D42):
//!
//! - **The `gix` half** — every *read* and every ref *write*. Synchronous functions over `&Path`
//!   that return owned values, each called by `isolate/real.rs` under
//!   [`tokio::task::spawn_blocking`]; no `gix::Repository` ever crosses an `.await`, because it is
//!   `Send` and *not* `Sync` (`gix-0.87.1/src/types.rs:148`) and `IsolatorFuture` is `Send`.
//! - **The `git` half** — a `Cli` that spawns the binary for the five verbs `gix` 0.87.1 does not
//!   implement: `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort` and
//!   `reset --hard` (plan OQ-1, resolved by the maintainer on 2026-09-22; D23, D25, D47;
//!   `docs/ANA-2.md:1777` as amended). Nothing is ever parsed from a verb's stdout: success is
//!   exit 0 *plus* a `gix` post-condition, and failure is one line of stderr ([`Exited::message`]).
//!
//! `git worktree prune` is never spawned (plan D46): our entries are created `--lock`ed and prune
//! refuses locked entries, so the verb's only reachable effect is on worktrees this orchestrator
//! did not create. A stale entry is reported instead.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use htui_core::model::{RunId, StepId};
use tokio::io::AsyncReadExt as _;

use crate::isolate::IsolateError;

/// The oldest `git` that has everything the five verbs use: `worktree add --lock --reason` and
/// `worktree remove --force --force` (plan D23, the ledger's tag walk). 2.33.0, August 2021.
pub const MIN_GIT: (u32, u32, u32) = (2, 33, 0);

/// One verb's wall-clock budget; on expiry the process group is killed and nothing is retried.
pub const VERB_TIMEOUT: Duration = Duration::from_secs(120);

/// Bytes kept of each of stdout and stderr: the **last** 64 KiB (plan D30's figure).
///
/// The tail, not `launch.rs`'s head cap (`crates/htui-agent/src/launch.rs:736-740`), because the
/// line that classifies a `git` failure is the last one it wrote, not the first.
pub const CAPTURE_TAIL: usize = 64 * 1024;

/// `CREATE_NO_WINDOW`: a TUI must not flash a console window when it spawns a child.
///
/// Named here rather than imported because `htui_agent::launch`'s copy is `pub(crate)`
/// (`crates/htui-agent/src/launch.rs:54`) and because the constant then reads the same on every
/// platform. `verify.rs` shares this one (blueprint F-I, H-16).
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Every environment variable that redirects repository discovery, or the location of the index,
/// the refs or the objects — removed from every `git` child this file spawns.
///
/// `GIT_DIR=/nonexistent` was verified to break `worktree add` outright; the rest are the same
/// class, and an `htui` launched from inside a `git` hook or a `git rebase -x` inherits them all.
/// Everything else is inherited on purpose: `PATH`, `HOME` and the user's `~/.gitconfig` apply by
/// design, which is the inherited-behaviour risk the plan names under Risks.
pub const SCRUBBED_ENV: [&str; 10] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_PREFIX",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
];

/// The identity `merge --no-ff` commits under, and only it (plan D25).
///
/// Through the environment rather than `-c user.name=…` because a managed repository may carry no
/// `user.*` at all: this form was verified to commit as `htui <htui@localhost>` with
/// `HOME=/nonexistent` and no configuration anywhere.
pub const IDENTITY_ENV: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "htui"),
    ("GIT_AUTHOR_EMAIL", "htui@localhost"),
    ("GIT_COMMITTER_NAME", "htui"),
    ("GIT_COMMITTER_EMAIL", "htui@localhost"),
];

// The ten failure classes of blueprint §3.3, one named function per sentence, the house style of
// `htui_core::store::traits`'s refusals (`crates/htui-core/src/store/traits.rs:1053-1160`). A
// rewording is a one-line change here and nowhere else.

/// `which::which("git")` found nothing.
pub fn not_on_path() -> String {
    "git not on PATH".to_owned()
}

/// The binary answered `--version` with something older than [`MIN_GIT`].
pub fn too_old(version: (u32, u32, u32)) -> String {
    let (major, minor, patch) = version;
    let (min_major, min_minor, min_patch) = MIN_GIT;
    format!("git {major}.{minor}.{patch} is older than {min_major}.{min_minor}.{min_patch}")
}

/// The `--version` probe itself could not be spawned.
fn version_could_not_run(err: &std::io::Error) -> String {
    format!("git --version could not run: {err}")
}

/// The `--version` probe ran and printed something [`Cli::parse_version`] does not recognise.
fn version_not_understood(line: &str) -> String {
    format!("git --version output is not understood: {line}")
}

/// The operating system refused the spawn of a verb.
fn cannot_spawn(verb: &str, binary: &Path, err: &std::io::Error) -> String {
    format!("git {verb}: cannot spawn {}: {err}", binary.display())
}

/// A verb outlived its budget and its process group was killed.
fn timed_out(verb: &str, budget: Duration) -> String {
    format!("git {verb} timed out after {}", budget_text(budget))
}

/// A verb exited with no code at all, which on Unix means a signal killed it.
fn killed_by_signal(verb: &str) -> String {
    format!("git {verb}: killed by signal")
}

/// `120s` for the production budget, `200ms` for a test's; never `0s` for a sub-second one.
fn budget_text(budget: Duration) -> String {
    if budget.subsec_millis() == 0 {
        format!("{}s", budget.as_secs())
    } else {
        format!("{}ms", budget.as_millis())
    }
}

/// The `git` binary this process spawns for the five verbs `gix` 0.87.1 lacks.
///
/// Located once and version-checked once by [`Cli::locate`], then cloned freely: no verb
/// re-probes. Plan OQ-1, D23, D25, D47; `docs/ANA-2.md:1777` as amended.
#[derive(Debug, Clone)]
pub struct Cli {
    binary: PathBuf,
    version: (u32, u32, u32),
    /// [`VERB_TIMEOUT`] in production; a test injects a short one through `Cli::with_budget`.
    budget: Duration,
}

impl Cli {
    /// Finds `git` on `PATH`, reads its version and checks it against [`MIN_GIT`].
    ///
    /// Synchronous — it spawns `git --version` — because `GixIsolator::new` is, and because the
    /// answer is what decides whether a test runs or prints its skip (plan D40): a test can never
    /// pass on a box where production would refuse.
    ///
    /// # Errors
    /// [`IsolateError::Refused`] with [`not_on_path`], [`too_old`], or the sentence for a
    /// `--version` that could not run or could not be parsed.
    pub fn locate() -> Result<Self, IsolateError> {
        let binary = which::which("git").map_err(|_| IsolateError::Refused(not_on_path()))?;
        Self::probe(binary)
    }

    /// [`locate`](Cli::locate) with the binary already chosen; the version check is the same.
    ///
    /// # Errors
    /// As [`locate`](Cli::locate), minus the `PATH` lookup.
    pub fn probe(binary: PathBuf) -> Result<Self, IsolateError> {
        let mut probe = std::process::Command::new(&binary);
        probe.arg("--version").stdin(Stdio::null());
        for key in SCRUBBED_ENV {
            probe.env_remove(key);
        }
        probe.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");

        let output = probe
            .output()
            .map_err(|err| IsolateError::Refused(version_could_not_run(&err)))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().next().unwrap_or_default().trim();
        let version = Self::parse_version(line)
            .ok_or_else(|| IsolateError::Refused(version_not_understood(line)))?;
        if version < MIN_GIT {
            return Err(IsolateError::Refused(too_old(version)));
        }
        Ok(Self {
            binary,
            version,
            budget: VERB_TIMEOUT,
        })
    }

    /// The three leading integers of a `git version …` line, or `None`.
    ///
    /// `git version 2.43.0`, `git version 2.47.1.windows.1` and `git version 2.39.5 (Apple
    /// Git-154)` all parse; everything past the third component is ignored, because every
    /// distributor appends something different there and none of it orders.
    #[must_use]
    pub fn parse_version(line: &str) -> Option<(u32, u32, u32)> {
        let rest = line.trim().strip_prefix("git version ")?;
        let mut parts = rest.split('.');
        let major = leading_u32(parts.next()?)?;
        let minor = leading_u32(parts.next()?)?;
        let patch = leading_u32(parts.next()?)?;
        Some((major, minor, patch))
    }

    /// The version this binary reported, for a caller that wants to say so in a message.
    #[must_use]
    pub fn version(&self) -> (u32, u32, u32) {
        self.version
    }

    /// The binary itself, for the test support's own `git worktree list --porcelain` oracle.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// The same `git`, with a shorter per-verb budget — the only way a test can reach the timeout
    /// class without waiting two minutes for it.
    #[cfg(test)]
    fn with_budget(&self, budget: Duration) -> Self {
        Self {
            budget,
            ..self.clone()
        }
    }

    /// The one place a `git` child's environment and stdio are shaped.
    ///
    /// `SCRUBBED_ENV` is removed; `LC_ALL=C` pins the English the classifier and the conflict
    /// branch match; `GIT_TERMINAL_PROMPT=0` keeps a verb from blocking on a tty;
    /// `GIT_OPTIONAL_LOCKS=0` stops `git` refreshing the index opportunistically and taking
    /// `index.lock` under a concurrent `gix` read; `GIT_ADVICE=0` suppresses the `hint:` lines
    /// 2.42+ would otherwise print after the `fatal:` one. The last two are simply ignored by a
    /// `git` that predates them and neither changes an exit status (blueprint H-21).
    fn command(&self, cwd: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.binary);
        command.current_dir(cwd);
        for key in SCRUBBED_ENV {
            command.env_remove(key);
        }
        command
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_ADVICE", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    /// Spawns one verb and returns how it exited, whatever that was.
    ///
    /// A non-zero exit is **not** an error here: each verb classifies its own before calling
    /// [`Exited::failure`], because exit 1 means two different things to `git merge` (blueprint
    /// H-8). Both pipes are read concurrently into tail buffers so neither can fill and stall the
    /// child, and the whole of read-both-then-wait runs under the budget.
    ///
    /// # Errors
    /// [`IsolateError::Git`] when the spawn is refused, when the wait fails, or when the budget
    /// elapses — in the last case the process group is killed first.
    pub async fn run(
        &self,
        verb: &'static str,
        cwd: &Path,
        args: &[&OsStr],
        extra_env: &[(&str, &str)],
    ) -> Result<Exited, IsolateError> {
        let build = || {
            let mut command = self.command(cwd);
            command.args(args);
            for (key, value) in extra_env {
                command.env(key, value);
            }
            command
        };

        let mut child = spawn_supervised(build)
            .map_err(|err| IsolateError::Git(cannot_spawn(verb, &self.binary, &err)))?;
        let stdout = child.stdout().take();
        let stderr = child.stderr().take();

        let collected = tokio::time::timeout(self.budget, async {
            let (out, err) = tokio::join!(
                read_tail(stdout, CAPTURE_TAIL),
                read_tail(stderr, CAPTURE_TAIL)
            );
            (out, err, child.wait().await)
        })
        .await;

        match collected {
            Ok((out, err, status)) => {
                let status = status.map_err(|err| {
                    IsolateError::Git(format!("git {verb}: waiting failed: {err}"))
                })?;
                Ok(Exited {
                    code: status.code(),
                    stdout: out.into_string(),
                    stderr: err.into_string(),
                })
            }
            Err(_elapsed) => {
                // The group on Unix, the job object on Windows: a `git` that spawned a hook or a
                // pager must not outlive the verb that started it.
                let _ = Box::into_pin(child.kill()).await;
                Err(IsolateError::Git(timed_out(verb, self.budget)))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The five verbs. Each classifies its own exit before building an error, because exit 1 means two
// different things to `git merge` (blueprint H-8) and exit 128 two to `worktree remove`.
// ---------------------------------------------------------------------------------------------

/// What a successful `merge --no-ff` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The merge commit, which becomes the winner's `after_hash` (ANA-2 `:987-988`).
    pub commit: String,
}

/// D25's conflict refusal: the sentence ANA-2 `:983` fixes, with the path list.
///
/// Named here rather than in `isolate/real.rs` because [`Cli::merge_no_ff`] is what detects the
/// conflict and reads the paths; `real.rs` passes the sentence straight through.
pub fn merge_conflict(paths: &[String]) -> String {
    format!("merge_conflict: {}", paths.join(", "))
}

/// The post-condition of a verb held, but the tree it made does not answer for it.
fn broken_post_condition(verb: &str, what: &str) -> IsolateError {
    IsolateError::Git(format!("git {verb}: {what}"))
}

impl Cli {
    /// D23: `git worktree add --lock --reason "htui run <run>" -b htui/<step> <path> <before>`,
    /// run in the managed checkout.
    ///
    /// Nothing is parsed from its output. Success is exit 0 **plus** two `gix` post-conditions:
    /// the new tree's `HEAD` is `before`, and the main repository lists a locked entry at `path`.
    ///
    /// # Errors
    /// [`IsolateError::Git`] with the last classifying stderr line — a branch that already exists
    /// (exit 255, unreachable after D38's reuse), a path that is already there (exit 128), an
    /// invalid start point (exit 128), or a held ref lock, which [`with_retry`] sleeps over.
    pub async fn add_worktree(
        &self,
        repo: &Path,
        path: &Path,
        step: StepId,
        run: RunId,
        before: &str,
    ) -> Result<(), IsolateError> {
        let branch = format!("htui/{step}");
        let reason = format!("htui run {run}");
        let exited = self
            .run(
                "worktree add",
                repo,
                &[
                    OsStr::new("worktree"),
                    OsStr::new("add"),
                    OsStr::new("--lock"),
                    OsStr::new("--reason"),
                    OsStr::new(&reason),
                    OsStr::new("-b"),
                    OsStr::new(&branch),
                    path.as_os_str(),
                    OsStr::new(before),
                ],
                &[],
            )
            .await?;
        if !exited.ok() {
            return Err(exited.failure("worktree add"));
        }

        if head(path)? != before {
            return Err(broken_post_condition(
                "worktree add",
                "created a tree that does not check out",
            ));
        }
        if !worktree_by_path(repo, path)?.is_some_and(|entry| entry.locked) {
            return Err(broken_post_condition(
                "worktree add",
                "created a tree that is not listed as locked",
            ));
        }
        Ok(())
    }

    /// D23 and D46: `git worktree remove --force --force <path>`, run in the managed checkout.
    ///
    /// One `--force` for a dirty tree and the second for a locked one — ours are always both. A
    /// directory that is already gone removes cleanly (the crash-recovery case) and a path `git`
    /// has never heard of reads as already removed.
    ///
    /// # Errors
    /// [`IsolateError::Git`] for any other non-zero exit.
    pub async fn remove_worktree(&self, repo: &Path, path: &Path) -> Result<(), IsolateError> {
        let exited = self
            .run(
                "worktree remove",
                repo,
                &[
                    OsStr::new("worktree"),
                    OsStr::new("remove"),
                    OsStr::new("--force"),
                    OsStr::new("--force"),
                    path.as_os_str(),
                ],
                &[],
            )
            .await?;
        if exited.ok()
            || (exited.code == Some(128) && exited.stderr.contains("is not a working tree"))
        {
            return Ok(());
        }
        Err(exited.failure("worktree remove"))
    }

    /// D47: `git reset --hard <target>` in `tree`, the verb that replaced a hand-composed `gix`
    /// checkout over a populated directory.
    ///
    /// Post-condition: the tree's `HEAD` is `target` and [`is_dirty`] is false. Untracked files
    /// survive, which is exactly why D24 does not count them.
    ///
    /// # Errors
    /// [`IsolateError::Git`] for a non-zero exit — a held `index.lock` among them, which
    /// [`with_retry`] sleeps over — or for a post-condition that does not hold.
    pub async fn reset_hard(&self, tree: &Path, target: &str) -> Result<(), IsolateError> {
        let exited = self
            .run(
                "reset --hard",
                tree,
                &[
                    OsStr::new("reset"),
                    OsStr::new("--hard"),
                    OsStr::new(target),
                ],
                &[],
            )
            .await?;
        if !exited.ok() {
            return Err(exited.failure("reset --hard"));
        }
        if head(tree)? != target || is_dirty(tree)? {
            return Err(broken_post_condition(
                "reset --hard",
                "left a tree that is not at the target or not clean",
            ));
        }
        Ok(())
    }

    /// D25: `git merge --no-ff --no-edit -m "htui: reconcile <step>" <after>` in the primary
    /// checkout, under the four identity variables.
    ///
    /// Exit 0 is checked against `gix`: the new `HEAD`'s parents must be exactly
    /// `[before, after]`. Exit 1 is where the care goes — it is both "conflict" and "somebody
    /// holds `index.lock`" (blueprint H-8), and both leave `MERGE_HEAD` behind — so the lock
    /// signature is consulted *first* and only the remainder is a conflict. Every failing path
    /// aborts the half-merge before it returns, and the abort is itself retried, so a caller that
    /// sleeps and tries again does not meet a `MERGE_HEAD` it left there (H-7).
    ///
    /// # Errors
    /// [`IsolateError::Refused`] with [`merge_conflict`] for a real conflict;
    /// [`IsolateError::Git`] for a held lock (retried) or any other non-zero exit; whichever error
    /// `merge --abort` produced, if the abort is what failed — the primary is then mid-merge and
    /// the operator has to be told so.
    pub async fn merge_no_ff(
        &self,
        primary: &Path,
        step: StepId,
        before: &str,
        after: &str,
    ) -> Result<Merged, IsolateError> {
        let message = format!("htui: reconcile {step}");
        let exited = self
            .run(
                "merge",
                primary,
                &[
                    OsStr::new("merge"),
                    OsStr::new("--no-ff"),
                    OsStr::new("--no-edit"),
                    OsStr::new("-m"),
                    OsStr::new(&message),
                    OsStr::new(after),
                ],
                &IDENTITY_ENV,
            )
            .await?;

        if exited.ok() {
            let parents = head_parents(primary)?;
            if parents != [before, after] {
                return Err(broken_post_condition(
                    "merge",
                    "the merge commit's parents are not [before_hash, after_hash]",
                ));
            }
            return Ok(Merged {
                commit: head(primary)?,
            });
        }

        // A lock, a conflict or anything else: read the paths first, because `--abort` throws the
        // stage entries away, then restore the primary, then classify.
        let conflicted = if exited.code == Some(1) && lock_signature(&exited.stderr).is_none() {
            conflicted_paths(primary)?
        } else {
            Vec::new()
        };
        with_retry("merge --abort", || self.abort_merge(primary)).await?;

        if conflicted.is_empty() {
            Err(exited.failure("merge"))
        } else {
            Err(IsolateError::Refused(merge_conflict(&conflicted)))
        }
    }

    /// `git merge --abort` in `primary`; "There is no merge to abort" is success.
    ///
    /// # Errors
    /// [`IsolateError::Git`] for any other non-zero exit, which leaves the primary mid-merge and
    /// is never swallowed (blueprint H-7).
    pub async fn abort_merge(&self, primary: &Path) -> Result<(), IsolateError> {
        let exited = self
            .run(
                "merge --abort",
                primary,
                &[OsStr::new("merge"), OsStr::new("--abort")],
                &[],
            )
            .await?;
        if exited.ok()
            || (exited.code == Some(128) && exited.stderr.contains("There is no merge to abort"))
        {
            return Ok(());
        }
        Err(exited.failure("merge --abort"))
    }
}

/// The sleeps between retries (plan D39): 200, 400 and 800 ms, so four attempts in all.
///
/// Entirely ours. `gix::lock` offers `Fail::AfterDurationWithBackoff`, whose backoff is quadratic
/// rather than this, and `gix` itself acquires with `Fail::Immediately` and never retries for us.
pub const RETRY_BACKOFF: [Duration; 3] = [
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
];

/// Runs `op`, retrying it on [`RETRY_BACKOFF`]'s schedule while [`is_lock_error`] says the failure
/// was somebody else holding a `.lock` file (plan D39).
///
/// Every git **write** goes through this — the five verbs, [`create_branch`] and [`copy_range`];
/// reads do not. Each failed attempt is logged at `warn` with its classification, so a `git`
/// whose wording drifts shows up as a run of warnings rather than as silence.
///
/// # Errors
/// Whatever `op` returned on its last attempt.
pub async fn with_retry<T, F, Fut>(verb: &'static str, mut op: F) -> Result<T, IsolateError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, IsolateError>>,
{
    let mut attempt = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) if attempt < RETRY_BACKOFF.len() && is_lock_error(&err) => {
                tracing::warn!(
                    verb,
                    attempt,
                    %err,
                    "a git lock was held; sleeping before the next attempt"
                );
                tokio::time::sleep(RETRY_BACKOFF[attempt]).await;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// The leading ASCII digits of `text` as a number, or `None` when it starts with none.
fn leading_u32(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// How one verb ended: the exit code, and the tails of both pipes.
///
/// Returned for **every** exit status, zero or not. `code` is `None` when a signal killed the
/// child, which cannot happen on Windows (blueprint §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exited {
    /// `None` when the child was killed by a signal.
    pub code: Option<i32>,
    /// The last [`CAPTURE_TAIL`] bytes of stdout, lossily decoded. Never parsed: every success has
    /// a `gix` post-condition instead.
    pub stdout: String,
    /// The last [`CAPTURE_TAIL`] bytes of stderr, lossily decoded.
    pub stderr: String,
}

impl Exited {
    /// Exit status zero.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// The stderr line a human should be shown.
    ///
    /// The **lock line if there is one**, otherwise the last non-empty line. That order is not
    /// cosmetic: `git merge` under a held `index.lock` prints `error: Unable to write index.` and
    /// *then* `Automatic merge failed; fix conflicts and then commit the result.`, and
    /// `git reset --hard` under one prints the `fatal: Unable to create '…/index.lock': File
    /// exists.` four lines before `remove the file manually to continue.` (both verified on
    /// 2.43.0 under `LC_ALL=C`). Taking the last line would hand [`is_lock_error`] a sentence with
    /// no signature in it and turn every retryable lock collision into a permanent failure.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        lock_signature(&self.stderr).or_else(|| {
            self.stderr
                .lines()
                .map(str::trim_end)
                .rfind(|line| !line.is_empty())
        })
    }

    /// This exit as a typed error: `git <verb>: <the line [`message`](Exited::message) chose>`.
    #[must_use]
    pub fn failure(&self, verb: &str) -> IsolateError {
        let Some(code) = self.code else {
            return IsolateError::Git(killed_by_signal(verb));
        };
        match self.message() {
            Some(line) => IsolateError::Git(format!("git {verb}: {line}")),
            None => IsolateError::Git(format!("git {verb}: exit status {code} with no stderr")),
        }
    }
}

/// The line of `stderr` that says a `.lock` file was in the way, if any (plan D39, source 2).
///
/// Three wordings, all produced on this box under `LC_ALL=C`: a ref lock on `worktree add`
/// (`fatal: cannot lock ref 'refs/heads/htui/<step>': Unable to create '<git_dir>/refs/heads/…
/// .lock': File exists.`, exit 255), an `index.lock` on `reset --hard` (`fatal: Unable to create
/// '<git_dir>/index.lock': File exists.`, exit 128) and one inside `merge` (`error: Unable to
/// write index.`, exit **1** — the conflict status, blueprint H-8).
#[must_use]
pub fn lock_signature(stderr: &str) -> Option<&str> {
    stderr.lines().map(str::trim_end).find(|line| {
        (line.contains("Unable to create '") && line.contains(".lock': File exists"))
            || line.contains("cannot lock ref")
            || line.contains("Unable to write index")
    })
}

/// Whether `err` is worth sleeping over (plan D39).
///
/// Two sources, as D39 names them: `gix`'s own lock errors, whose `Display` carries `.lock` or
/// `cannot lock` (`gix-lock-24.0.0/src/acquire.rs:44-52`; `gix` acquires with `Fail::Immediately`
/// and never retries for us), and the `git` CLI's, which [`lock_signature`] recognises.
///
/// A [`IsolateError::Refused`] is **never** one, even though a conflict path list can perfectly
/// well contain `Cargo.lock`: a refusal is a decision, not a collision. A timed-out verb is never
/// one either — the budget already elapsed once and a retry would spend it again.
#[must_use]
pub fn is_lock_error(err: &IsolateError) -> bool {
    match err {
        IsolateError::Refused(_) => false,
        IsolateError::Git(text) => {
            !text.contains("timed out")
                && (text.contains(".lock")
                    || text.contains("cannot lock")
                    || text.contains("Unable to write index"))
        }
        // `gix::lock` surfaces a held lock as `AlreadyExists` when it surfaces it as `io` at all.
        IsolateError::Io(err) => err.kind() == std::io::ErrorKind::AlreadyExists,
    }
}

/// The last `cap` bytes pushed through it, and nothing else.
///
/// A `VecDeque` drained from the front past the cap: the cheap shape for "keep the end", where
/// `Vec` would need either a copy per overflow or a second pass at the end.
#[derive(Debug)]
pub struct TailBuffer {
    bytes: VecDeque<u8>,
    cap: usize,
}

impl TailBuffer {
    /// An empty buffer that will keep at most `cap` bytes.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            cap,
        }
    }

    /// Appends `chunk`, dropping whatever no longer fits from the front.
    pub fn push(&mut self, chunk: &[u8]) {
        // Only the last `cap` bytes of a chunk can possibly survive, so a 100 MiB write costs one
        // pass and 64 KiB of copying, not 100 MiB of pushing and popping.
        let keep = chunk.len().min(self.cap);
        self.bytes.extend(&chunk[chunk.len() - keep..]);
        while self.bytes.len() > self.cap {
            self.bytes.pop_front();
        }
    }

    /// The buffer as text, lossily.
    ///
    /// A cap that lands mid-codepoint yields a leading replacement character; that is the price of
    /// keeping the end of the output rather than the start.
    #[must_use]
    pub fn into_string(self) -> String {
        let bytes: Vec<u8> = self.bytes.into();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Drains `reader` into a [`TailBuffer`], stopping at end of stream or at the first pipe error.
async fn read_tail(reader: Option<impl tokio::io::AsyncRead + Unpin>, cap: usize) -> TailBuffer {
    let mut tail = TailBuffer::new(cap);
    let Some(mut reader) = reader else {
        return tail;
    };
    let mut chunk = vec![0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => tail.push(&chunk[..read]),
        }
    }
    tail
}

/// Builds the wrapped command and spawns it, mirroring `launch.rs:1106-1164` verb for verb.
///
/// Unix: a process group, so a kill reaches a hook `git` started. Windows: `CREATE_NO_WINDOW` and
/// a job object, with the same refused-job-object downgrade `launch.rs` takes — a supervision
/// mode, not a dead verb.
fn spawn_supervised(
    build: impl Fn() -> tokio::process::Command,
) -> std::io::Result<Box<dyn process_wrap::tokio::ChildWrapper>> {
    use process_wrap::tokio::CommandWrap;

    #[cfg(unix)]
    {
        let mut wrapped = CommandWrap::from(build());
        wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
        wrapped.spawn()
    }

    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::PROCESS_CREATION_FLAGS;

        let mut wrapped = CommandWrap::from(build());
        wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)));
        wrapped.wrap(JobObject);
        match wrapped.spawn() {
            Ok(child) => Ok(child),
            Err(error) => {
                tracing::warn!(%error, "job object assignment refused; spawning the verb without it");
                let mut wrapped = CommandWrap::from(build());
                wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)));
                wrapped.spawn()
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The `gix` half: every read and every ref write.
//
// Synchronous, over `&Path`, returning owned values. `isolate/real.rs` calls each of these under
// `tokio::task::spawn_blocking` with an owned `PathBuf`, so no `gix::Repository` — `Send` but not
// `Sync` (`gix-0.87.1/src/types.rs:148`) — is ever held across an `.await` (blueprint H-17).
// ---------------------------------------------------------------------------------------------

/// A repository at `path` has no commit, so there is no `before_hash` to record (blueprint A-4).
///
/// ANA-2 `:962-963` makes `run_step_commit.before_hash` `NOT NULL`, so there is no row to write
/// and the mode refuses rather than inventing one. `isolate/real.rs` reaches for this one too,
/// with the repo's name rather than its path, so the sentence has a single home.
pub fn unborn_head(name: &str) -> String {
    format!("unborn HEAD: {name} has no commit to record as before_hash")
}

/// `gix::open` (`gix-0.87.1/src/lib.rs:418`).
///
/// # Errors
/// [`IsolateError::Git`] when there is no repository at `path` or it cannot be read.
pub fn open(path: &Path) -> Result<gix::Repository, IsolateError> {
    gix::open(path)
        .map_err(|err| IsolateError::Git(format!("cannot open {}: {err}", path.display())))
}

/// `HEAD`'s peeled commit as lowercase hex (`gix-0.87.1/src/repository/reference.rs:211`).
///
/// # Errors
/// [`IsolateError::Refused`] with [`unborn_head`] for a repository with no commit;
/// [`IsolateError::Git`] for anything else.
pub fn head(path: &Path) -> Result<String, IsolateError> {
    let repo = open(path)?;
    let head = repo.head().map_err(|err| {
        IsolateError::Git(format!("cannot read HEAD of {}: {err}", path.display()))
    })?;
    if head.is_unborn() {
        return Err(IsolateError::Refused(unborn_head(
            &path.display().to_string(),
        )));
    }
    let id = head.into_peeled_id().map_err(|err| {
        IsolateError::Git(format!("cannot peel HEAD of {}: {err}", path.display()))
    })?;
    Ok(id.detach().to_hex().to_string())
}

/// `HEAD`'s parents as hex, in order (`gix-0.87.1/src/object/commit.rs:154`).
///
/// D25's post-condition after `merge --no-ff`: exactly `[before, after]`.
///
/// # Errors
/// [`IsolateError::Git`] when `HEAD` has no commit or cannot be read.
pub fn head_parents(path: &Path) -> Result<Vec<String>, IsolateError> {
    let repo = open(path)?;
    let commit = repo.head_commit().map_err(|err| {
        IsolateError::Git(format!(
            "cannot read the HEAD commit of {}: {err}",
            path.display()
        ))
    })?;
    Ok(commit
        .parent_ids()
        .map(|id| id.detach().to_hex().to_string())
        .collect())
}

/// `Repository::is_dirty()` (`gix-0.87.1/src/status/mod.rs:168`), which is plan D24's predicate.
///
/// A change in the index against `HEAD` or in the working tree against the index, submodules
/// included, **untracked files excluded** — because the only consumer of `dirty` is milestone 5's
/// reset to `before_hash`, and a reset never deletes an untracked file anyway.
///
/// # Errors
/// [`IsolateError::Git`] when the status walk fails.
pub fn is_dirty(path: &Path) -> Result<bool, IsolateError> {
    let repo = open(path)?;
    repo.is_dirty().map_err(|err| {
        IsolateError::Git(format!("cannot read status of {}: {err}", path.display()))
    })
}

/// Whether the repository declares any submodule
/// (`gix-0.87.1/src/repository/submodule.rs:93`).
///
/// The `worktree` mode refuses one: `git worktree add` checks out the gitlink and leaves the
/// submodule uninitialised, which is a tree the agent cannot build in.
///
/// # Errors
/// [`IsolateError::Git`] when `.gitmodules` exists and cannot be parsed.
pub fn has_submodules(path: &Path) -> Result<bool, IsolateError> {
    let repo = open(path)?;
    let modules = repo.submodules().map_err(|err| {
        IsolateError::Git(format!(
            "cannot read the submodules of {}: {err}",
            path.display()
        ))
    })?;
    Ok(modules.is_some_and(|mut names| names.next().is_some()))
}

/// Creates `refs/heads/<name>` at `target`, refusing to move an existing one
/// (`gix-0.87.1/src/repository/reference.rs:79`, `PreviousValue::MustNotExist`).
///
/// D26's label for `shared_serialized` and A-2's for `copy`. Called through [`with_retry`],
/// because the ref store takes a `.lock` file and `gix` acquires it with `Fail::Immediately`.
///
/// # Errors
/// [`IsolateError::Git`] when the name is invalid, the target is not a hash, or the reference
/// already exists.
pub fn create_branch(path: &Path, name: &str, target: &str) -> Result<(), IsolateError> {
    let repo = open(path)?;
    let id = parse_oid(target)?;
    repo.reference(
        format!("refs/heads/{name}"),
        id,
        gix::refs::transaction::PreviousValue::MustNotExist,
        "htui: label",
    )
    .map(|_| ())
    .map_err(|err| IsolateError::Git(format!("cannot create refs/heads/{name}: {err}")))
}

/// What `refs/heads/<name>` points at, or `None` when it does not exist
/// (`gix-0.87.1/src/repository/reference.rs:323`).
///
/// D38 reads this to decide whether a tree already prepared for this step can be reused, and D25
/// reads it to find the winner's tip after the worktree itself was removed at capture.
///
/// # Errors
/// [`IsolateError::Git`] when the ref store cannot be read or the reference does not peel.
pub fn branch_target(path: &Path, name: &str) -> Result<Option<String>, IsolateError> {
    let repo = open(path)?;
    let full = format!("refs/heads/{name}");
    let Some(mut reference) = repo
        .try_find_reference(full.as_str())
        .map_err(|err| IsolateError::Git(format!("cannot look up {full}: {err}")))?
    else {
        return Ok(None);
    };
    let id = reference
        .peel_to_id()
        .map_err(|err| IsolateError::Git(format!("cannot peel {full}: {err}")))?;
    Ok(Some(id.detach().to_hex().to_string()))
}

/// One entry of `worktrees()`, flattened so it can outlive the `Repository` that read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// The checkout's base directory, canonicalised where it still exists.
    pub path: PathBuf,
    /// Whether a `locked` file stands beside the administrative entry.
    pub locked: bool,
    /// The text of that `locked` file — `htui run <run_id>` for one of ours.
    pub lock_reason: Option<String>,
}

/// The linked worktree of `main` whose base is `path`, or `None`
/// (`gix-0.87.1/src/repository/worktree.rs:46`, `src/worktree/proxy.rs:48-103`).
///
/// Both sides are canonicalised before the comparison (blueprint H-11): `git` writes the base into
/// its `gitdir` file in its own spelling, and a symlinked checkout — `/var` under `/private/var`
/// on a mac, a `TempDir` under a symlinked `TMPDIR` — differs from ours byte for byte otherwise.
///
/// # Errors
/// [`IsolateError::Git`] when the `worktrees/` directory cannot be read.
pub fn worktree_by_path(main: &Path, path: &Path) -> Result<Option<WorktreeEntry>, IsolateError> {
    let wanted = canonical(path);
    Ok(worktree_entries(main)?
        .into_iter()
        .find(|entry| entry.path == wanted))
}

/// Every linked worktree of `main` whose base is under `root`.
///
/// D46's report: cleanup runs `worktree remove` per tree and then reads this. A non-empty answer
/// is named in the cleanup error; `git worktree prune` is never run, because it would also clear
/// the maintainer's own stale entries and cannot be scoped to ours (blueprint F-B).
///
/// # Errors
/// [`IsolateError::Git`] when the `worktrees/` directory cannot be read.
pub fn worktrees_under(main: &Path, root: &Path) -> Result<Vec<PathBuf>, IsolateError> {
    let root = canonical(root);
    Ok(worktree_entries(main)?
        .into_iter()
        .filter(|entry| entry.path.starts_with(&root))
        .map(|entry| entry.path)
        .collect())
}

/// Every linked worktree of `main`, flattened and canonicalised once.
fn worktree_entries(main: &Path) -> Result<Vec<WorktreeEntry>, IsolateError> {
    let repo = open(main)?;
    let proxies = repo.worktrees().map_err(|err| {
        IsolateError::Git(format!(
            "cannot list the worktrees of {}: {err}",
            main.display()
        ))
    })?;
    let mut entries = Vec::with_capacity(proxies.len());
    for proxy in proxies {
        // A base that cannot be read belongs to an entry whose `gitdir` file is gone; it is still
        // an entry `git worktree list` shows, so D46's report must not drop it.
        let Ok(base) = proxy.base() else { continue };
        entries.push(WorktreeEntry {
            path: canonical(&base),
            locked: proxy.is_locked(),
            lock_reason: proxy.lock_reason().map(|reason| reason.to_string()),
        });
    }
    Ok(entries)
}

/// The paths of every index entry left at a conflicted stage, sorted and deduplicated
/// (`gix-0.87.1/src/repository/index.rs:25`; `gix-index-0.55.0/src/entry/mod.rs:3-11`).
///
/// D25's conflict branch reads this after `git merge` exits 1 and before `git merge --abort`
/// throws the stage entries away.
///
/// # Errors
/// [`IsolateError::Git`] when the index cannot be opened.
pub fn conflicted_paths(path: &Path) -> Result<Vec<String>, IsolateError> {
    let repo = open(path)?;
    let index = match repo.open_index() {
        Ok(index) => index,
        // A repository whose index file was never written has no conflicts to report.
        Err(gix::worktree::open_index::Error::IndexFile(_)) => return Ok(Vec::new()),
        Err(err) => {
            return Err(IsolateError::Git(format!(
                "cannot open the index of {}: {err}",
                path.display()
            )));
        }
    };
    let mut paths: Vec<String> = index
        .entries()
        .iter()
        .filter(|entry| entry.stage() != gix::index::entry::Stage::Unconflicted)
        .map(|entry| entry.path(&index).to_string())
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Copies every object of `base..tip` that `to` lacks out of `from`'s object database, and reports
/// how many it wrote (plan OQ-5).
///
/// `rev_walk([tip]).with_hidden([base])` (`gix-0.87.1/src/repository/revision.rs:174`) is the
/// range; per commit the commit itself, its tree and every tree and blob reachable from it that
/// `to` does not already have. A `copy` reconcile runs this before `merge_no_ff`, so `<after_hash>`
/// resolves in the primary. Writes take loose-object locks, which is why this too goes through
/// [`with_retry`].
///
/// # Errors
/// [`IsolateError::Git`] when either repository cannot be read or an object cannot be written.
pub fn copy_range(from: &Path, to: &Path, base: &str, tip: &str) -> Result<u32, IsolateError> {
    let source = open(from)?;
    let target = open(to)?;
    let tip_id = parse_oid(tip)?;
    let base_id = parse_oid(base)?;

    let walk = source
        .rev_walk([tip_id])
        .with_hidden([base_id])
        .all()
        .map_err(|err| IsolateError::Git(format!("cannot walk {base}..{tip}: {err}")))?;

    let mut copied = 0;
    for info in walk {
        let info =
            info.map_err(|err| IsolateError::Git(format!("cannot walk {base}..{tip}: {err}")))?;
        let commit = source
            .find_object(info.id)
            .map_err(|err| IsolateError::Git(format!("cannot read commit {}: {err}", info.id)))?
            .into_commit();
        let tree_id = commit
            .tree_id()
            .map_err(|err| IsolateError::Git(format!("commit {} has no tree: {err}", info.id)))?
            .detach();
        copy_tree_objects(&source, &target, tree_id, &mut copied)?;
        copy_object(&source, &target, info.id, &mut copied)?;
    }
    Ok(copied)
}

/// `id` and everything reachable from it, skipping a subtree the target already has.
///
/// A git object store that holds a tree holds everything under it, so a present tree ends the
/// recursion — which is what makes a second `copy_range` of the same range cost one lookup.
fn copy_tree_objects(
    source: &gix::Repository,
    target: &gix::Repository,
    id: gix::ObjectId,
    copied: &mut u32,
) -> Result<(), IsolateError> {
    if !copy_object(source, target, id, copied)? {
        return Ok(());
    }
    let tree = source
        .find_object(id)
        .map_err(|err| IsolateError::Git(format!("cannot read tree {id}: {err}")))?
        .into_tree();
    let entries = tree
        .iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| IsolateError::Git(format!("cannot decode tree {id}: {err}")))?;
    for entry in entries {
        let child = entry.oid().to_owned();
        match entry.mode().kind() {
            gix::objs::tree::EntryKind::Tree => {
                copy_tree_objects(source, target, child, copied)?;
            }
            // A gitlink names a commit in *another* repository; the `worktree` mode refuses
            // submodules outright and `copy` copies the directory wholesale, so there is nothing
            // of ours on the other side of one.
            gix::objs::tree::EntryKind::Commit => {}
            _ => {
                copy_object(source, target, child, copied)?;
            }
        }
    }
    Ok(())
}

/// Writes one object into `target` if it is not already there; reports whether it wrote.
fn copy_object(
    source: &gix::Repository,
    target: &gix::Repository,
    id: gix::ObjectId,
    copied: &mut u32,
) -> Result<bool, IsolateError> {
    use gix::objs::Write as _;

    if target.has_object(id) {
        return Ok(false);
    }
    let object = source
        .find_object(id)
        .map_err(|err| IsolateError::Git(format!("cannot read object {id}: {err}")))?;
    // The bytes verbatim, not a re-encode: the hash is the contract and a round trip through a
    // decoded type is one more place it could change.
    target
        .objects
        .write_buf(object.kind, &object.data)
        .map_err(|err| IsolateError::Git(format!("cannot write object {id}: {err}")))?;
    *copied += 1;
    Ok(true)
}

/// A hex hash as an [`gix::ObjectId`].
fn parse_oid(hex: &str) -> Result<gix::ObjectId, IsolateError> {
    gix::ObjectId::from_hex(hex.as_bytes())
        .map_err(|err| IsolateError::Git(format!("{hex} is not an object id: {err}")))
}

/// `path` resolved through the filesystem, or `path` itself when it does not exist.
///
/// A vanished worktree still has an administrative entry to report, and that entry's base cannot
/// be canonicalised; falling back to the literal path keeps it comparable with the one we asked
/// for, which is how it was spelled when we created it.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Printed byte for byte by every git-backed test on a box with no `git` on `PATH` (plan D40).
///
/// The convention is `htui_store::testkit::SKIP`'s (`crates/htui-store/src/testkit.rs:35`): a
/// missing external is a skip the suite prints and passes, not a failure.
pub const SKIP_GIT: &str = "skipped: git not on PATH";

/// Real repositories for tests, built with `gix` alone so they need no `git` on the box (D40).
#[cfg(any(test, feature = "test-support"))]
pub mod testkit {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use super::{Cli, IsolateError, SKIP_GIT, too_old};

    /// The `git` this box can run, or the sentence a test should print and pass on.
    ///
    /// Decided once per process, through the very probe `GixIsolator::new` runs, so a test can
    /// never pass on a box where production would refuse (plan D40). A `git` below the floor
    /// yields `skipped: git <version> is older than 2.33.0`.
    pub fn usable_git() -> Result<Cli, String> {
        static DECIDED: OnceLock<Result<Cli, String>> = OnceLock::new();
        DECIDED
            .get_or_init(|| {
                Cli::locate().map_err(|err| match err {
                    // The floor sentence is the refusal's own, with the skip prefix in front of
                    // it, so the two never drift apart.
                    IsolateError::Refused(reason) if reason == too_old(super::MIN_GIT) => {
                        format!("skipped: {reason}")
                    }
                    IsolateError::Refused(reason) if reason.starts_with("git ") => {
                        format!("skipped: {reason}")
                    }
                    _ => SKIP_GIT.to_owned(),
                })
            })
            .clone()
    }

    /// `let Some(git) = skip_without_git!() else { return };` — prints the skip sentence and
    /// yields `None` on a box without a usable `git`.
    #[macro_export]
    macro_rules! skip_without_git {
        () => {
            match $crate::isolate::git::testkit::usable_git() {
                Ok(git) => Some(git),
                Err(reason) => {
                    println!("{reason}");
                    None
                }
            }
        };
    }

    /// One `worktree` block of `git worktree list --porcelain`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PorcelainWorktree {
        /// The `worktree` line's path.
        pub path: PathBuf,
        /// The `HEAD` line's hash, absent for a bare entry.
        pub head: Option<String>,
        /// The `branch` line's full reference name, absent when the tree is detached.
        pub branch: Option<String>,
        /// The `locked` line's reason, absent when the tree is not locked.
        pub locked: Option<String>,
    }

    /// `git worktree list --porcelain` in `repo`, parsed.
    ///
    /// Criteria 11 and 13's oracle. It is not a nicety: it runs the *same* binary that wrote the
    /// administrative entry, so it answers for what `git` believes rather than for what `gix`
    /// reconstructs.
    ///
    /// # Panics
    /// When the verb cannot run or exits non-zero.
    pub async fn worktree_list(git: &Cli, repo: &Path) -> Vec<PorcelainWorktree> {
        use std::ffi::OsStr;

        let exited = git
            .run(
                "worktree list",
                repo,
                &[
                    OsStr::new("worktree"),
                    OsStr::new("list"),
                    OsStr::new("--porcelain"),
                ],
                &[],
            )
            .await
            .expect("git worktree list runs");
        assert!(exited.ok(), "git worktree list failed: {}", exited.stderr);

        let mut entries: Vec<PorcelainWorktree> = Vec::new();
        for line in exited.stdout.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => entries.push(PorcelainWorktree {
                    path: PathBuf::from(value),
                    head: None,
                    branch: None,
                    locked: None,
                }),
                "HEAD" | "branch" | "locked" => {
                    let Some(entry) = entries.last_mut() else {
                        continue;
                    };
                    let value = (!value.is_empty()).then(|| value.to_owned());
                    match key {
                        "HEAD" => entry.head = value,
                        "branch" => entry.branch = value,
                        _ => entry.locked = value,
                    }
                }
                _ => {}
            }
        }
        entries
    }

    /// The signature every test commit carries; a fixed timestamp keeps the hashes of a fixture
    /// stable across runs.
    fn who() -> gix::actor::SignatureRef<'static> {
        gix::actor::SignatureRef {
            name: gix::bstr::BStr::new(b"htui test"),
            email: gix::bstr::BStr::new(b"test@localhost"),
            time: "1600000000 +0000",
        }
    }

    /// `gix::init` and nothing else: a repository whose `HEAD` is unborn.
    ///
    /// # Panics
    /// When the directory cannot be initialised.
    pub fn empty_repo(dir: &Path) {
        gix::init(dir).expect("the repository is initialised");
    }

    /// A repository with one file `f` and one commit; returns the commit's hash.
    ///
    /// # Panics
    /// When the repository cannot be created or committed to.
    pub fn repo_with_one_commit(dir: &Path) -> String {
        empty_repo(dir);
        commit_file(dir, "f", "first\n", "one")
    }

    /// Writes `name`, commits it on whatever `HEAD` names, and leaves the index matching the new
    /// commit so [`super::is_dirty`] reads `false` afterwards.
    ///
    /// # Panics
    /// When any step of the commit fails.
    pub fn commit_file(repo: &Path, name: &str, body: &str, message: &str) -> String {
        let repository = gix::open(repo).expect("the repository opens");
        let blob = repository
            .write_blob(body.as_bytes())
            .expect("the blob is written")
            .detach();

        let (parents, mut entries) = match repository.head_commit() {
            Ok(commit) => {
                let tree = commit.tree().expect("the parent commit has a tree");
                let entries = tree
                    .iter()
                    .map(|entry| {
                        let entry = entry.expect("the parent tree decodes");
                        gix::objs::tree::Entry {
                            mode: entry.mode(),
                            filename: entry.filename().to_owned(),
                            oid: entry.oid().to_owned(),
                        }
                    })
                    .collect::<Vec<_>>();
                (vec![commit.id], entries)
            }
            Err(_) => (Vec::new(), Vec::new()),
        };
        entries.retain(|entry| entry.filename != name);
        entries.push(gix::objs::tree::Entry {
            mode: gix::objs::tree::EntryKind::Blob.into(),
            filename: name.into(),
            oid: blob,
        });
        entries.sort();

        let tree = repository
            .write_object(gix::objs::Tree { entries })
            .expect("the tree is written")
            .detach();
        let commit = repository
            .commit_as(who(), who(), "HEAD", message, tree, parents)
            .expect("the commit is written")
            .detach();

        std::fs::write(repo.join(name), body).expect("the working-tree file is written");
        repository
            .index_from_tree(&tree)
            .expect("an index is built from the new tree")
            .write(gix::index::write::Options::default())
            .expect("the index is written");
        commit.to_hex().to_string()
    }

    /// Whether `repo`'s object database holds `hex`.
    ///
    /// # Panics
    /// When the repository cannot be opened or `hex` is not a hash.
    #[must_use]
    pub fn has_object(repo: &Path, hex: &str) -> bool {
        let repository = gix::open(repo).expect("the repository opens");
        let id = gix::ObjectId::from_hex(hex.as_bytes()).expect("a hex object id");
        repository.has_object(id)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::testkit::{commit_file, empty_repo, has_object, repo_with_one_commit};
    use super::{CAPTURE_TAIL, Cli, Exited, MIN_GIT, SCRUBBED_ENV, TailBuffer, is_lock_error};
    use crate::isolate::IsolateError;

    /// A repository this crate made with `gix` alone reads back through this crate's own `head`.
    #[test]
    fn init_commit_and_head_round_trip() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let first = repo_with_one_commit(dir.path());
        assert_eq!(first.len(), 40, "a hex sha1: {first}");
        assert_eq!(super::head(dir.path()).expect("HEAD reads"), first);

        let second = commit_file(dir.path(), "g", "second\n", "two");
        assert_ne!(second, first);
        assert_eq!(super::head(dir.path()).expect("HEAD reads"), second);
        assert_eq!(
            super::head_parents(dir.path()).expect("the parents read"),
            vec![first],
            "one parent, the commit before it"
        );
    }

    /// `gix::init` alone makes a repository with an unborn `HEAD`, and `before_hash` is `NOT NULL`
    /// (ANA-2 `:962-963`), so there is no row to write and the mode refuses (blueprint A-4).
    #[test]
    fn head_of_an_unborn_repo_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        empty_repo(dir.path());
        let err = super::head(dir.path()).expect_err("an unborn HEAD has no hash");
        assert!(
            err.to_string()
                .starts_with("isolation refused: unborn HEAD: ")
                && err
                    .to_string()
                    .ends_with(" has no commit to record as before_hash"),
            "unexpected sentence: {err}"
        );
    }

    /// Plan D24: the index against `HEAD` and the working tree against the index — untracked files
    /// are not a change, because milestone 5's reset would not have deleted them anyway.
    #[test]
    fn is_dirty_ignores_untracked_and_sees_a_modified_tracked_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        assert!(
            !super::is_dirty(dir.path()).expect("status reads"),
            "a fresh checkout is clean"
        );

        std::fs::write(dir.path().join("untracked"), "noise\n").expect("the file is written");
        assert!(
            !super::is_dirty(dir.path()).expect("status reads"),
            "an untracked file is not a change (D24)"
        );

        std::fs::write(dir.path().join("f"), "edited\n").expect("the file is written");
        assert!(
            super::is_dirty(dir.path()).expect("status reads"),
            "a modified tracked file is"
        );
    }

    /// D26's label: written once with `MustNotExist`, read back by name, `None` for one that is
    /// not there (D38's idempotence check reads exactly this).
    #[test]
    fn branch_label_is_created_once_and_read_back() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let base = repo_with_one_commit(dir.path());

        assert_eq!(
            super::branch_target(dir.path(), "htui/absent").expect("the read succeeds"),
            None,
            "a branch that was never made reads as None, not as an error"
        );

        super::create_branch(dir.path(), "htui/step", &base).expect("the label is written");
        assert_eq!(
            super::branch_target(dir.path(), "htui/step").expect("the read succeeds"),
            Some(base.clone())
        );

        // `gix-ref-0.67.1/src/store/file/transaction/prepare.rs:157-165`: `MustNotExist` tolerates
        // a reference that already points exactly where the write would have put it, and refuses
        // only a move. Capture relies on the first half — a `shared_serialized` step whose capture
        // runs twice writes the same label twice (blueprint §6.4).
        super::create_branch(dir.path(), "htui/step", &base)
            .expect("relabelling the same commit is not an error");

        let moved = commit_file(dir.path(), "g", "second\n", "two");
        let err = super::create_branch(dir.path(), "htui/step", &moved)
            .expect_err("MustNotExist refuses a move");
        assert!(
            matches!(err, IsolateError::Git(_)),
            "moving an existing label is a git error, not a refusal: {err}"
        );
        assert_eq!(
            super::branch_target(dir.path(), "htui/step").expect("the read succeeds"),
            Some(base),
            "and the label did not move"
        );
    }

    /// OQ-5: a `copy` reconcile needs `<after_hash>` to resolve in the primary before `git merge`
    /// runs, so the range's objects move first.
    #[test]
    fn copy_range_moves_every_object_between_odbs() {
        let source = tempfile::tempdir().expect("a temporary directory");
        let target = tempfile::tempdir().expect("a temporary directory");
        let base = repo_with_one_commit(source.path());
        let tip = commit_file(source.path(), "added", "by the agent\n", "the step's work");
        empty_repo(target.path());

        assert!(
            !has_object(target.path(), &tip),
            "the target starts without it"
        );
        let copied =
            super::copy_range(source.path(), target.path(), &base, &tip).expect("the range copies");
        assert!(
            copied >= 3,
            "the commit, its tree and the new blob at least: {copied}"
        );
        assert!(
            has_object(target.path(), &tip),
            "the tip resolves in the target"
        );
        assert!(
            !has_object(target.path(), &base),
            "the hidden base is not walked: it is already in the primary in production"
        );

        // Idempotent: a second copy of the same range writes nothing new.
        assert_eq!(
            super::copy_range(source.path(), target.path(), &base, &tip).expect("the range copies"),
            0,
            "every object of the range is already there"
        );
    }

    /// The three reads whose interesting half needs a real `git` still have to answer for a plain
    /// repository, which is the state every `prepare` starts from.
    #[test]
    fn a_plain_repo_has_no_submodules_no_conflicts_and_no_linked_worktrees() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());

        assert!(!super::has_submodules(dir.path()).expect("the read succeeds"));
        assert_eq!(
            super::conflicted_paths(dir.path()).expect("the index reads"),
            Vec::<String>::new()
        );
        assert_eq!(
            super::worktrees_under(dir.path(), dir.path()).expect("the read succeeds"),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            super::worktree_by_path(dir.path(), &dir.path().join("nowhere"))
                .expect("the read succeeds"),
            None
        );

        let err = super::open(&dir.path().join("not-a-repo")).expect_err("there is no repo there");
        assert!(
            err.to_string().starts_with("git: cannot open "),
            "unexpected sentence: {err}"
        );
    }

    /// Three shapes `git --version` is known to print, and the refusals below them (blueprint
    /// §3.3 steps 3 and 4).
    #[test]
    fn parses_git_version_and_refuses_below_2_33() {
        assert_eq!(Cli::parse_version("git version 2.43.0"), Some((2, 43, 0)));
        assert_eq!(
            Cli::parse_version("git version 2.47.1.windows.1"),
            Some((2, 47, 1)),
            "Git for Windows appends a fourth component"
        );
        assert_eq!(
            Cli::parse_version("git version 2.39.5 (Apple Git-154)"),
            Some((2, 39, 5)),
            "Apple's git appends a parenthesised build after the patch digits"
        );

        assert_eq!(Cli::parse_version("2.43.0"), None, "the prefix is required");
        assert_eq!(Cli::parse_version("git version 2.43"), None);
        assert_eq!(Cli::parse_version("git version nonsense"), None);

        assert!((2, 32, 1) < MIN_GIT);
        assert_eq!(
            super::too_old((2, 32, 1)),
            "git 2.32.1 is older than 2.33.0"
        );
        assert_eq!(super::not_on_path(), "git not on PATH");
        assert_eq!(
            super::version_not_understood("git version nonsense"),
            "git --version output is not understood: git version nonsense"
        );
    }

    /// A binary that answers `--version` with a version below the floor is refused by the probe
    /// itself, before any verb is ever spawned.
    #[cfg(unix)]
    #[test]
    fn a_git_below_the_floor_is_refused_by_the_probe() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let old = fake_git(dir.path(), "old-git", "echo 'git version 2.32.1'; exit 0");
        let err = Cli::probe(old).expect_err("2.32.1 is below the floor");
        assert_eq!(
            err.to_string(),
            "isolation refused: git 2.32.1 is older than 2.33.0"
        );

        let mute = fake_git(dir.path(), "mute-git", "echo 'not a version'; exit 0");
        let err = Cli::probe(mute).expect_err("an unparseable line is refused");
        assert_eq!(
            err.to_string(),
            "isolation refused: git --version output is not understood: not a version"
        );

        let missing = dir.path().join("no-such-git");
        let err = Cli::probe(missing).expect_err("a binary that is not there cannot be probed");
        assert!(
            err.to_string()
                .starts_with("isolation refused: git --version could not run: "),
            "unexpected sentence: {err}"
        );
    }

    /// The cap is a tail, not a head: the *last* 64 KiB survive a write of twice that.
    #[test]
    fn tail_buffer_keeps_the_last_64_kib() {
        let mut tail = TailBuffer::new(CAPTURE_TAIL);
        for index in 0..200_u32 {
            tail.push(format!("{index:0>1024}\n").as_bytes());
        }
        let kept = tail.into_string();
        assert_eq!(kept.len(), CAPTURE_TAIL, "exactly the cap is kept");
        assert!(
            kept.ends_with("0199\n"),
            "the end survives: {:?}",
            &kept[kept.len() - 8..]
        );
        assert!(!kept.contains("0000\n"), "the start does not");

        // One push larger than the cap on its own.
        let mut tail = TailBuffer::new(8);
        tail.push(b"abcdefghijklm");
        assert_eq!(tail.into_string(), "fghijklm");

        let mut empty = TailBuffer::new(CAPTURE_TAIL);
        empty.push(b"");
        assert_eq!(empty.into_string(), "");
    }

    /// The scrub list is the documented one, read back off the `Command` itself.
    #[cfg(unix)]
    #[test]
    fn env_scrub_list_is_the_documented_one() {
        let cli = usable_or_fake(&tempfile::tempdir().expect("a temporary directory"));
        let command = cli.command(std::path::Path::new("."));
        let envs: Vec<(String, Option<String>)> = command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();

        for key in SCRUBBED_ENV {
            assert!(
                envs.contains(&(key.to_owned(), None)),
                "{key} is not removed from the child environment"
            );
        }
        for (key, value) in [
            ("LC_ALL", "C"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_OPTIONAL_LOCKS", "0"),
            ("GIT_ADVICE", "0"),
        ] {
            assert!(
                envs.contains(&(key.to_owned(), Some(value.to_owned()))),
                "{key}={value} is not set on the child"
            );
        }
        assert_eq!(
            envs.len(),
            SCRUBBED_ENV.len() + 4,
            "the child environment is shaped in exactly one place: {envs:?}"
        );
    }

    /// A verb that outlives its budget is killed with its whole process group, and says so.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_verb_that_hangs_times_out() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let marker = dir.path().join("survived");
        // An absolute `sleep` and a redirection rather than `touch`: this case must not depend on
        // what is on `PATH`, because the suite is also run with `git` stripped out of it (C-8).
        let sleep = which::which("sleep").expect("a box that can run a test suite has `sleep`");
        let script = format!(
            "if [ \"$1\" = \"--version\" ]; then echo 'git version 2.43.0'; exit 0; fi\n\
             '{}' 2\n\
             : > '{}'\n",
            sleep.display(),
            marker.display()
        );
        let binary = fake_git(dir.path(), "slow-git", &script);
        let cli = Cli::probe(binary)
            .expect("the script answers --version")
            .with_budget(Duration::from_millis(200));

        let started = std::time::Instant::now();
        let err = cli
            .run("merge", dir.path(), &[OsStr::new("hang")], &[])
            .await
            .expect_err("the verb outlives its budget");
        assert_eq!(err.to_string(), "git: git merge timed out after 200ms");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "it did not wait for the child"
        );
        assert!(
            !is_lock_error(&err),
            "a timeout is never retried, however its text reads"
        );

        // The `sleep` is a child of the script: if only the shell had been killed, the marker
        // would appear. The process group is what makes it not.
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!marker.exists(), "the process group outlived the kill");
    }

    /// The exit an `index.lock` collision inside `git merge` produces, byte for byte from this
    /// box, classified: the lock line wins over the last line, and `with_retry` will see it.
    #[test]
    fn a_lock_collision_is_named_by_the_lock_line_not_the_last_one() {
        let merge = Exited {
            code: Some(1),
            stdout: String::new(),
            stderr: "error: Unable to write index.\n\
                     Automatic merge failed; fix conflicts and then commit the result.\n"
                .to_owned(),
        };
        assert_eq!(merge.message(), Some("error: Unable to write index."));
        let err = merge.failure("merge");
        assert_eq!(
            err.to_string(),
            "git: git merge: error: Unable to write index."
        );
        assert!(is_lock_error(&err));

        let reset = Exited {
            code: Some(128),
            stdout: String::new(),
            stderr: "fatal: Unable to create '/r/.git/index.lock': File exists.\n\n\
                     Another git process seems to be running in this repository.\n\
                     remove the file manually to continue.\n"
                .to_owned(),
        };
        assert!(is_lock_error(&reset.failure("reset --hard")));

        // A genuine conflict carries no signature, so it is classified as one and never slept on.
        let conflict = Exited {
            code: Some(1),
            stdout: String::new(),
            stderr: "Auto-merging f\nCONFLICT (content): Merge conflict in f\n\
                     Automatic merge failed; fix conflicts and then commit the result.\n"
                .to_owned(),
        };
        assert_eq!(
            conflict.message(),
            Some("Automatic merge failed; fix conflicts and then commit the result.")
        );
        assert!(!is_lock_error(&conflict.failure("merge")));

        // A refusal whose path list happens to name a lock file is still a decision.
        assert!(!is_lock_error(&IsolateError::Refused(
            "merge_conflict: Cargo.lock".to_owned()
        )));

        let silent = Exited {
            code: Some(3),
            stdout: "noise".to_owned(),
            stderr: "  \n".to_owned(),
        };
        assert_eq!(
            silent.failure("worktree remove").to_string(),
            "git: git worktree remove: exit status 3 with no stderr"
        );
        let signalled = Exited {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(
            signalled.failure("worktree add").to_string(),
            "git: git worktree add: killed by signal"
        );
        assert!(!signalled.ok());
    }

    /// D39's schedule, both of its sources, and the two classes it must not touch.
    #[tokio::test(start_paused = true)]
    async fn with_retry_retries_three_times_on_a_lock_error_and_not_on_others() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let attempts = AtomicU32::new(0);
        let err = super::with_retry("worktree add", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(IsolateError::Git(
                "git worktree add: fatal: cannot lock ref 'refs/heads/htui/x'".to_owned(),
            ))
        })
        .await
        .expect_err("every attempt failed");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            4,
            "the first plus three retries"
        );
        assert!(is_lock_error(&err));

        // `gix`'s own wording, D39's first source.
        let attempts = AtomicU32::new(0);
        let _ = super::with_retry("create branch", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(IsolateError::Git(
                "cannot create refs/heads/htui/x: The lock for resource \
                 '.git/refs/heads/htui/x.lock' could not be obtained"
                    .to_owned(),
            ))
        })
        .await;
        assert_eq!(attempts.load(Ordering::SeqCst), 4);

        // A lock that clears: the second attempt is the last.
        let attempts = AtomicU32::new(0);
        super::with_retry("reset --hard", || async {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(IsolateError::Git(
                    "git reset --hard: fatal: Unable to create '/r/.git/index.lock': File exists."
                        .to_owned(),
                ))
            } else {
                Ok(())
            }
        })
        .await
        .expect("the second attempt succeeded");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

        for (label, refused, text) in [
            ("a timeout", false, "git merge timed out after 120s"),
            ("a refusal", true, "merge_conflict: Cargo.lock"),
            (
                "any other git failure",
                false,
                "git merge: fatal: bad object",
            ),
        ] {
            let attempts = AtomicU32::new(0);
            let _ = super::with_retry("merge", || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    Err::<(), _>(if refused {
                        IsolateError::Refused(text.to_owned())
                    } else {
                        IsolateError::Git(text.to_owned())
                    })
                }
            })
            .await;
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                1,
                "{label} is never retried"
            );
        }
    }

    /// D23's post-conditions, checked by both oracles: `gix`'s `worktrees()` and the very binary
    /// that wrote the entry.
    #[tokio::test]
    async fn add_worktree_is_listed_by_gix_and_by_git() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);

        let run = htui_core::model::RunId::new();
        let step = htui_core::model::StepId::new();
        let tree = dir.path().join("trees").join("core");
        git.add_worktree(&repo, &tree, step, run, &before)
            .await
            .expect("the worktree is added");

        let entry = super::worktree_by_path(&repo, &tree)
            .expect("the read succeeds")
            .expect("gix lists the entry");
        assert!(entry.locked, "--lock wrote the locked file");
        assert_eq!(
            entry.lock_reason.as_deref(),
            Some(&*format!("htui run {run}"))
        );
        assert_eq!(super::head(&tree).expect("the tree has a HEAD"), before);
        assert_eq!(
            super::branch_target(&repo, &format!("htui/{step}")).expect("the read succeeds"),
            Some(before.clone()),
            "-b created the branch at the start point"
        );

        let listed = super::testkit::worktree_list(&git, &repo).await;
        let ours = listed
            .iter()
            .find(|entry| entry.path == tree)
            .expect("git worktree list names the tree it made");
        assert_eq!(ours.head.as_deref(), Some(&*before));
        assert_eq!(
            ours.branch.as_deref(),
            Some(&*format!("refs/heads/htui/{step}"))
        );
        assert_eq!(ours.locked.as_deref(), Some(&*format!("htui run {run}")));

        // D23's own failure mode, unreachable in production because of D38's reuse.
        let again = dir.path().join("trees").join("core-again");
        let err = git
            .add_worktree(&repo, &again, step, run, &before)
            .await
            .expect_err("the branch already exists");
        assert_eq!(
            err.to_string(),
            format!("git: git worktree add: fatal: a branch named 'htui/{step}' already exists")
        );
    }

    /// D23's remove: two `--force`es clear a locked *and* dirty tree, and a directory that is
    /// already gone — the crash-recovery case — is not an error either.
    #[tokio::test]
    async fn remove_clears_a_locked_dirty_worktree_and_a_vanished_one() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);
        let run = htui_core::model::RunId::new();

        let dirty = dir.path().join("dirty");
        git.add_worktree(&repo, &dirty, htui_core::model::StepId::new(), run, &before)
            .await
            .expect("the worktree is added");
        std::fs::write(dirty.join("f"), "edited by the agent\n").expect("the file is written");
        git.remove_worktree(&repo, &dirty)
            .await
            .expect("locked and dirty is still removable with two forces");
        assert!(!dirty.exists(), "the directory is gone");
        assert_eq!(
            super::worktree_by_path(&repo, &dirty).expect("the read succeeds"),
            None,
            "and so is the administrative entry"
        );

        let vanished = dir.path().join("vanished");
        git.add_worktree(
            &repo,
            &vanished,
            htui_core::model::StepId::new(),
            run,
            &before,
        )
        .await
        .expect("the worktree is added");
        std::fs::remove_dir_all(&vanished).expect("the directory is deleted behind git's back");
        git.remove_worktree(&repo, &vanished)
            .await
            .expect("a vanished tree removes cleanly");

        git.remove_worktree(&repo, &dir.path().join("never-existed"))
            .await
            .expect("`is not a working tree` reads as already removed");
    }

    /// D46: an entry that outlives its directory is **reported**, never pruned.
    #[tokio::test]
    async fn a_stale_entry_that_survives_remove_is_reported_and_never_pruned() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        let root = dir.path().join("trees");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);
        let run = htui_core::model::RunId::new();

        let removed = root.join("removed");
        let stale = root.join("stale");
        for tree in [&removed, &stale] {
            git.add_worktree(&repo, tree, htui_core::model::StepId::new(), run, &before)
                .await
                .expect("the worktree is added");
        }
        git.remove_worktree(&repo, &removed)
            .await
            .expect("the first is removed properly");
        std::fs::remove_dir_all(&stale).expect("the second's directory disappears");

        let surviving = super::worktrees_under(&repo, &root).expect("the read succeeds");
        assert_eq!(
            surviving,
            vec![super::canonical(&stale)],
            "exactly the entry whose directory went away, which is what cleanup reports"
        );
        assert!(
            super::testkit::worktree_list(&git, &repo)
                .await
                .iter()
                .any(|entry| entry.path == stale),
            "git itself still lists it: nothing pruned it"
        );
    }

    /// D47: the fifth verb, replacing the hand-composed `gix` checkout.
    #[tokio::test]
    async fn reset_hard_restores_a_deleted_and_a_modified_tracked_file() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        repo_with_one_commit(&repo);
        let target = commit_file(&repo, "g", "second\n", "two");

        std::fs::remove_file(repo.join("f")).expect("a tracked file is deleted");
        std::fs::write(repo.join("g"), "mangled\n").expect("another is modified");
        std::fs::write(repo.join("untracked"), "kept\n").expect("an untracked file is written");
        assert!(super::is_dirty(&repo).expect("status reads"));

        git.reset_hard(&repo, &target)
            .await
            .expect("the reset succeeds");
        assert_eq!(super::head(&repo).expect("HEAD reads"), target);
        assert!(
            !super::is_dirty(&repo).expect("status reads"),
            "the tree is clean again"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("g")).expect("g is back"),
            "second\n"
        );
        assert!(repo.join("f").exists(), "the deleted file is restored");
        assert!(
            repo.join("untracked").exists(),
            "reset --hard never removes an untracked file, which is why D24 ignores them"
        );
    }

    /// D25's happy path: always `--no-ff`, always two parents, always the `htui` identity.
    #[tokio::test]
    async fn merge_no_ff_of_a_descendant_makes_a_two_parent_commit() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);
        let step = htui_core::model::StepId::new();

        let tree = dir.path().join("tree");
        git.add_worktree(&repo, &tree, step, htui_core::model::RunId::new(), &before)
            .await
            .expect("the worktree is added");
        let after = commit_file(&tree, "written", "by the agent\n", "the step's work");

        let merged = git
            .merge_no_ff(&repo, step, &before, &after)
            .await
            .expect("a descendant merges");
        assert_eq!(super::head(&repo).expect("HEAD reads"), merged.commit);
        assert_eq!(
            super::head_parents(&repo).expect("the parents read"),
            vec![before, after],
            "--no-ff makes a merge commit even of a descendant (ANA-2 :980)"
        );
        assert!(
            repo.join("written").exists(),
            "the merge updated the primary working tree"
        );
        assert!(
            !repo.join(".git").join("MERGE_HEAD").exists(),
            "and concluded it"
        );
        assert!(!super::is_dirty(&repo).expect("status reads"));
    }

    /// D25's conflict branch: the path list, then `--abort`, then the refusal.
    #[tokio::test]
    async fn a_conflicting_merge_is_refused_with_the_path_and_aborted() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let base = repo_with_one_commit(&repo);
        let step = htui_core::model::StepId::new();

        let tree = dir.path().join("tree");
        git.add_worktree(&repo, &tree, step, htui_core::model::RunId::new(), &base)
            .await
            .expect("the worktree is added");
        let after = commit_file(&tree, "f", "the agent's line\n", "the step's work");
        let before = commit_file(&repo, "f", "the human's line\n", "meanwhile");

        let err = git
            .merge_no_ff(&repo, step, &before, &after)
            .await
            .expect_err("the two edits of `f` conflict");
        assert_eq!(err.to_string(), "isolation refused: merge_conflict: f");

        assert_eq!(
            super::head(&repo).expect("HEAD reads"),
            before,
            "the primary is where it was"
        );
        assert!(!super::is_dirty(&repo).expect("status reads"), "and clean");
        assert!(
            !repo.join(".git").join("MERGE_HEAD").exists(),
            "--abort ran before the refusal"
        );
    }

    /// Blueprint H-8: exit 1 is both "conflict" and "`index.lock` held", and only the second is
    /// worth sleeping over.
    #[tokio::test]
    async fn a_held_index_lock_inside_merge_is_aborted_and_retried() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);
        let step = htui_core::model::StepId::new();

        let tree = dir.path().join("tree");
        git.add_worktree(&repo, &tree, step, htui_core::model::RunId::new(), &before)
            .await
            .expect("the worktree is added");
        let after = commit_file(&tree, "written", "by the agent\n", "the step's work");

        // An index `gix` wrote from a tree carries no stat data, so `git` treats every path as
        // possibly modified and reaches for its stash path rather than for the index directly —
        // which produces `fatal: stash failed` instead of the lock wording this case is about.
        // One `reset --hard` gives the primary the fully stat-ed index a real checkout has.
        git.reset_hard(&repo, &before)
            .await
            .expect("the index is refreshed");

        let lock = repo.join(".git").join("index.lock");
        std::fs::write(&lock, "").expect("somebody else holds the index");
        let releaser = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            std::fs::remove_file(&lock).expect("the lock is released");
        });

        let merged = super::with_retry("merge", || git.merge_no_ff(&repo, step, &before, &after))
            .await
            .expect("a later attempt found the index free");
        releaser.await.expect("the releaser finished");

        assert_eq!(super::head(&repo).expect("HEAD reads"), merged.commit);
        assert_eq!(
            super::head_parents(&repo).expect("the parents read"),
            vec![before, after]
        );
        assert!(
            !repo.join(".git").join("MERGE_HEAD").exists(),
            "each failed attempt aborted its own half-merge"
        );
    }

    /// A `sh` script that answers `--version` like `git` does, marked executable.
    #[cfg(unix)]
    fn fake_git(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("the script is written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the script is executable");
        path
    }

    /// The real `git` when this box has one, and a script that claims to be 2.43.0 when it does
    /// not: this test reads the child environment, which does not depend on the binary.
    #[cfg(unix)]
    fn usable_or_fake(dir: &tempfile::TempDir) -> Cli {
        Cli::locate().unwrap_or_else(|_| {
            let binary = fake_git(dir.path(), "any-git", "echo 'git version 2.43.0'; exit 0");
            Cli::probe(binary).expect("the script answers --version")
        })
    }
}
