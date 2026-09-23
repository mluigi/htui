//! Every git fact this crate knows, and the only place it spawns a process.
//!
//! Two halves, one file, because they answer the same question from two directions (plan D42):
//!
//! - **The `gix` half** — every *read* and every ref *write*. Synchronous functions over `&Path`
//!   that return owned values, and every async caller — `isolate/real.rs`, and the verbs' own
//!   post-conditions below — reaches them through `blocking`, i.e. under
//!   [`tokio::task::spawn_blocking`]; no `gix::Repository` ever crosses an `.await`, because it is
//!   `Send` and *not* `Sync` (`gix-0.87.1/src/types.rs:148`) and `IsolatorFuture` is `Send`, and
//!   no status walk ever runs on a runtime worker.
//! - **The `git` half** — a `Cli` that spawns the binary for the six verbs `gix` 0.87.1 does not
//!   implement: `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort`,
//!   `reset --hard` (plan OQ-1, resolved by the maintainer on 2026-09-22; D23, D25, D47;
//!   `docs/ANA-2.md:1777` as amended) and `diff` (MOD-4 milestone 4 D55). Nothing is ever parsed
//!   from the first five verbs' stdout: success is exit 0 *plus* a `gix` post-condition, and
//!   failure is one line of stderr ([`Exited::message`]). `diff` is the one verb whose stdout
//!   *is* the product: it is handed on verbatim, head-capped, and never parsed either.
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

/// The oldest `git` that has everything the six verbs use: `worktree add --lock --reason` and
/// `worktree remove --force --force` (plan D23, the ledger's tag walk). 2.33.0, August 2021.
pub const MIN_GIT: (u32, u32, u32) = (2, 33, 0);

/// One verb's wall-clock budget; on expiry the process group is killed and nothing is retried.
pub const VERB_TIMEOUT: Duration = Duration::from_secs(120);

/// The bound on `git --version` inside [`Cli::probe`], which runs synchronously at worker start.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a killed child gets to be reaped before it is abandoned.
///
/// `kill()` is `start_kill()` then `wait()`, and a `wait()` on a child stuck in uninterruptible
/// sleep (a hung NFS mount) does not return; the caller has already decided the child is over.
pub(crate) const KILL_GRACE: Duration = Duration::from_secs(5);

/// How long the pipes are still read after the verb itself exited.
///
/// A hook that daemonises inherits both pipes and holds them open long after `git` returned; the
/// verb's exit is the answer, and whatever the pipes had not delivered by then is not waited for.
const PIPE_GRACE: Duration = Duration::from_secs(1);

/// Bytes kept of each of stdout and stderr: the **last** 64 KiB (plan D30's figure).
///
/// The tail, not `launch.rs`'s head cap (`crates/htui-agent/src/launch.rs:736-740`), because the
/// line that classifies a `git` failure is the last one it wrote, not the first.
pub const CAPTURE_TAIL: usize = 64 * 1024;

/// Bytes kept of [`Cli::diff`]'s stdout: the **first** 64 KiB (MOD-4 milestone 4 D55).
///
/// The head, not the tail [`CAPTURE_TAIL`] keeps: a patch cut at its start loses its first
/// `diff --git`/`---`/`+++` headers and begins mid-hunk, which no reader can place.
pub const DIFF_CAP: usize = 64 * 1024;

/// What [`HeadBuffer::into_string`] appends to a capture that overflowed its cap (D55).
const DIFF_TRUNCATED: &str = "\n[diff truncated at 64 KiB]";

/// `git diff`'s `--stat` graph width (MOD-4 milestone 4 A-4, plan D75).
///
/// `git diff --stat` sizes itself from an inherited `COLUMNS` even when its stdout is a pipe, so
/// without this the judge prompt's bytes, and its digest, would depend on the terminal that
/// launched `htui`.
const DIFF_COLUMNS: (&str, &str) = ("COLUMNS", "80");

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

/// The `--version` probe did not exit within its bound and was killed.
fn version_timed_out(bound: Duration) -> String {
    format!("git --version did not answer within {}", budget_text(bound))
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

/// The `git` binary this process spawns for the six verbs `gix` 0.87.1 lacks.
///
/// Located once and version-checked once by [`Cli::locate`], then cloned freely: no verb
/// re-probes. Plan OQ-1, D23, D25, D47; `docs/ANA-2.md:1777` as amended; `diff` is MOD-4
/// milestone 4's D55, and the one verb whose stdout is the product.
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
        Self::probe_within(binary, PROBE_TIMEOUT)
    }

    /// [`probe`](Cli::probe) under an explicit bound: a `git` that never answers `--version` is
    /// killed and refused rather than hanging the synchronous `GixIsolator::new` with it.
    fn probe_within(binary: PathBuf, bound: Duration) -> Result<Self, IsolateError> {
        use std::io::Read as _;

        let mut probe = std::process::Command::new(&binary);
        probe
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for key in SCRUBBED_ENV {
            probe.env_remove(key);
        }
        probe.env("LC_ALL", "C").env("GIT_TERMINAL_PROMPT", "0");

        let mut child = probe
            .spawn()
            .map_err(|err| IsolateError::Refused(version_could_not_run(&err)))?;
        // The pipe is drained on its own thread, so neither a chatty binary nor a grandchild that
        // holds the pipe open can stall the bounded wait below.
        let (sender, receiver) = std::sync::mpsc::channel();
        if let Some(mut stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stdout.read_to_end(&mut bytes);
                let _ = sender.send(bytes);
            });
        }
        let deadline = std::time::Instant::now() + bound;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(IsolateError::Refused(version_timed_out(bound)));
                }
                Err(err) => return Err(IsolateError::Refused(version_could_not_run(&err))),
            }
        }
        let stdout = receiver.recv_timeout(PIPE_GRACE).unwrap_or_default();
        let text = String::from_utf8_lossy(&stdout);
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
    /// child. The budget bounds the verb's own exit; once it has exited, the pipes get
    /// `PIPE_GRACE` more and are then dropped, so a daemonising hook that inherited them cannot
    /// turn a verb that exited 0 into a timeout.
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
        self.run_capturing(verb, cwd, args, extra_env, Capture::Tail)
            .await
    }

    /// [`run`](Cli::run), with stdout kept as `stdout` says: the last [`CAPTURE_TAIL`] bytes for
    /// the five verbs whose stdout is never read, the first [`DIFF_CAP`] for [`diff`](Cli::diff),
    /// whose stdout is the product. Stderr is always tail-captured.
    async fn run_capturing(
        &self,
        verb: &'static str,
        cwd: &Path,
        args: &[&OsStr],
        extra_env: &[(&str, &str)],
        stdout: Capture,
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
        let out = std::sync::Mutex::new(match stdout {
            Capture::Tail => Sink::Tail(TailBuffer::new(CAPTURE_TAIL)),
            Capture::Head => Sink::Head(HeadBuffer::new(DIFF_CAP)),
        });
        let err = std::sync::Mutex::new(Sink::Tail(TailBuffer::new(CAPTURE_TAIL)));
        let stdout = child.stdout().take();
        let stderr = child.stderr().take();
        // Boxed rather than pinned on the stack so that dropping it drops the pipes with it.
        let mut reading = Box::pin(async {
            tokio::join!(read_tail(stdout, &out), read_tail(stderr, &err));
        });

        let waited = tokio::time::timeout(self.budget, async {
            tokio::select! {
                status = child.wait() => (status, false),
                () = &mut reading => (child.wait().await, true),
            }
        })
        .await;

        match waited {
            Ok((status, drained)) => {
                if !drained {
                    let _ = tokio::time::timeout(PIPE_GRACE, &mut reading).await;
                }
                drop(reading);
                let status = status.map_err(|err| {
                    IsolateError::Git(format!("git {verb}: waiting failed: {err}"))
                })?;
                Ok(Exited {
                    code: status.code(),
                    stdout: into_string(out),
                    stderr: into_string(err),
                })
            }
            Err(_elapsed) => {
                // The group on Unix, the job object on Windows: a `git` that spawned a hook or a
                // pager must not outlive the verb that started it.
                kill_within_grace(child.as_mut(), verb).await;
                Err(IsolateError::Git(timed_out(verb, self.budget)))
            }
        }
    }
}

/// Which end of a verb's stdout [`Cli::run_capturing`] keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    /// The last [`CAPTURE_TAIL`] bytes: the line that classifies a failure is the last one.
    Tail,
    /// The first [`DIFF_CAP`] bytes: a patch is read from its first header (D55).
    Head,
}

// ---------------------------------------------------------------------------------------------
// The six verbs. Each classifies its own exit before building an error, because exit 1 means two
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

/// D23's two `gix` post-conditions of `worktree add`: the new tree's `HEAD` is `before`, and the
/// main repository lists a locked entry at `path`.
async fn worktree_post_condition(
    repo: &Path,
    path: &Path,
    before: &str,
) -> Result<(), IsolateError> {
    let tree = path.to_path_buf();
    if blocking(move || head(&tree)).await? != before {
        return Err(broken_post_condition(
            "worktree add",
            "created a tree that does not check out",
        ));
    }
    let (main, tree) = (repo.to_path_buf(), path.to_path_buf());
    if !blocking(move || worktree_by_path(&main, &tree))
        .await?
        .is_some_and(|entry| entry.locked)
    {
        return Err(broken_post_condition(
            "worktree add",
            "created a tree that is not listed as locked",
        ));
    }
    Ok(())
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
        worktree_post_condition(repo, path, before).await
    }

    /// `git worktree add --lock --reason "htui run <run>" <path> htui/<step>`: D23's verb onto a
    /// branch that **already exists**, so without `-b`, which refuses one.
    ///
    /// The re-make of a tree whose administrative entry was pruned from under it: the branch — and
    /// with it the step's `before_hash` — survived, the checkout did not. `base` is where the
    /// branch points; the post-conditions are [`add_worktree`](Cli::add_worktree)'s.
    ///
    /// # Errors
    /// As [`add_worktree`](Cli::add_worktree).
    pub async fn add_worktree_on_branch(
        &self,
        repo: &Path,
        path: &Path,
        step: StepId,
        run: RunId,
        base: &str,
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
                    path.as_os_str(),
                    OsStr::new(&branch),
                ],
                &[],
            )
            .await?;
        if !exited.ok() {
            return Err(exited.failure("worktree add"));
        }
        worktree_post_condition(repo, path, base).await
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
    /// survive unless `target` tracks their path, which is written over them without a word; a
    /// caller that must not lose one asks [`untracked_paths_base_tracks`] first (plan D121).
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
        let read = tree.to_path_buf();
        let (at, dirty) = blocking(move || Ok((head(&read)?, is_dirty(&read)?))).await?;
        if at != target || dirty {
            return Err(broken_post_condition(
                "reset --hard",
                "left a tree that is not at the target or not clean",
            ));
        }
        Ok(())
    }

    /// D25: `git -c merge.log=false merge --no-ff --no-edit -m "htui: reconcile <step>" <after>`
    /// in the primary checkout, under the four identity variables. Plan D148: `merge.log` is off
    /// so a primary that sets it still gets D25's exact message; `--no-verify` is not passed, so
    /// the user's hooks stay their policy (plan D147 reads past a body they add).
    ///
    /// Exit 0 is checked against `gix`: the new `HEAD`'s parents must be exactly
    /// `[before, after]`, `before` being the primary's `HEAD` at the call — the step's base, or a
    /// descendant of it once another run has merged (plan D136). Exit 1 is where the care goes — it is both "conflict" and "somebody
    /// holds `index.lock`" (blueprint H-8), and both leave `MERGE_HEAD` behind — so the lock
    /// signature is consulted *first* and only the remainder is a conflict. Every failing path
    /// aborts the half-merge before it returns, and the abort is itself retried, so a caller that
    /// sleeps and tries again does not meet a `MERGE_HEAD` it left there (H-7).
    ///
    /// # Errors
    /// [`IsolateError::Refused`] with [`merge_conflict`] for a real conflict;
    /// [`IsolateError::Git`] for a held lock (retried) or any other non-zero exit; whichever error
    /// `merge --abort` produced, if the abort is what failed — the primary is then mid-merge and
    /// the operator has to be told so; [`IsolateError::Git`] from the conflicted-paths read when
    /// that read failed and the abort after it succeeded.
    pub async fn merge_no_ff(
        &self,
        primary: &Path,
        step: StepId,
        before: &str,
        after: &str,
    ) -> Result<Merged, IsolateError> {
        self.merge_no_ff_reading(primary, step, before, after, conflicted_paths)
            .await
    }

    /// [`merge_no_ff`](Cli::merge_no_ff) with the conflicted-paths read passed in: the only seam
    /// through which a test can make that read fail while leaving the index `--abort` needs intact.
    async fn merge_no_ff_reading(
        &self,
        primary: &Path,
        step: StepId,
        before: &str,
        after: &str,
        read_conflicts: fn(&Path) -> Result<Vec<String>, IsolateError>,
    ) -> Result<Merged, IsolateError> {
        let message = reconcile_message(step);
        let exited = self
            .run(
                "merge",
                primary,
                &[
                    OsStr::new("-c"),
                    OsStr::new("merge.log=false"),
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
            let read = primary.to_path_buf();
            let (parents, commit) =
                blocking(move || Ok((head_parents(&read)?, head(&read)?))).await?;
            if parents != [before, after] {
                return Err(broken_post_condition(
                    "merge",
                    "the merge commit's parents are not [HEAD, after_hash]",
                ));
            }
            return Ok(Merged { commit });
        }

        // A lock, a conflict or anything else: read the paths first, because `--abort` throws the
        // stage entries away, then restore the primary, then classify. A read that fails must not
        // skip the abort — that would leave `MERGE_HEAD` in the user's tree — so it is held until
        // the primary is restored and surfaced only then.
        let mut read_failed = None;
        let conflicted = if exited.code == Some(1) && lock_signature(&exited.stderr).is_none() {
            let read = primary.to_path_buf();
            blocking(move || read_conflicts(&read))
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(%err, "the conflicted paths could not be read; aborting anyway");
                    read_failed = Some(err);
                    Vec::new()
                })
        } else {
            Vec::new()
        };
        with_retry("merge --abort", || self.abort_merge(primary)).await?;

        if let Some(err) = read_failed {
            Err(err)
        } else if conflicted.is_empty() {
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

    /// MOD-4 milestone 4 D55 and A-4: `git diff --no-color --no-ext-diff --no-textconv
    /// --src-prefix=a/ --dst-prefix=b/ [--stat] <before> <after> --` in `repo`, under
    /// `COLUMNS=80`, with stdout head-capped at [`DIFF_CAP`].
    ///
    /// The flags are `git diff --help`'s: `--no-color` because the text is read by an agent, not a
    /// terminal; `--no-ext-diff` and `--no-textconv` keep `diff.external` and textconv drivers
    /// out of an orchestrator path; the explicit `--src-prefix`/`--dst-prefix` defeat
    /// `diff.noprefix` and `diff.mnemonicPrefix`; the trailing `--` keeps a revision from ever
    /// being read as a path. The stdout is the product and is returned verbatim, with
    /// `\n[diff truncated at 64 KiB]` appended when it overflowed. Not retried: a read (M3 D39).
    ///
    /// # Errors
    /// [`IsolateError::Git`] for a spawn or budget failure, or for a non-zero exit (a revision the
    /// repository does not hold, among them).
    pub async fn diff(
        &self,
        repo: &Path,
        before: &str,
        after: &str,
        stat: bool,
    ) -> Result<String, IsolateError> {
        let mut args = vec![
            OsStr::new("diff"),
            OsStr::new("--no-color"),
            OsStr::new("--no-ext-diff"),
            OsStr::new("--no-textconv"),
            OsStr::new("--src-prefix=a/"),
            OsStr::new("--dst-prefix=b/"),
        ];
        if stat {
            args.push(OsStr::new("--stat"));
        }
        args.extend([OsStr::new(before), OsStr::new(after), OsStr::new("--")]);
        let exited = self
            .run_capturing("diff", repo, &args, &[DIFF_COLUMNS], Capture::Head)
            .await?;
        if !exited.ok() {
            return Err(exited.failure("diff"));
        }
        Ok(exited.stdout)
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
/// Every git **write** goes through this — the five writing verbs, [`create_branch`] and
/// [`copy_range`]; reads do not, and neither does `diff`, which is one. Each failed attempt is logged at `warn` with its classification, so a `git`
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

/// Runs one synchronous `gix` or filesystem call on the blocking pool.
///
/// Every async caller of this file's `gix` half and of `isolate/copy.rs` goes through here with
/// owned paths, which is what keeps a `gix::Repository` — `Send` and not `Sync` — from ever being
/// alive across an `.await` (blueprint H-17), and a status walk over a large tree from stalling a
/// runtime worker. A task that panicked or was cancelled is a [`IsolateError::Git`], never a
/// panic in the caller.
///
/// # Errors
/// Whatever `task` returned, or [`IsolateError::Git`] when it did not finish.
pub(crate) async fn blocking<T, F>(task: F) -> Result<T, IsolateError>
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
    /// Stdout, lossily decoded: the last [`CAPTURE_TAIL`] bytes, or the first [`DIFF_CAP`] for
    /// [`Cli::diff`]. Never parsed, except by [`Cli::diff`], whose stdout is the product: every
    /// other success has a `gix` post-condition instead.
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
///
/// The ref lock is recognised by its `Unable to create '….lock': File exists` clause, not by
/// `cannot lock ref`: git prints that prefix for a directory/file ref conflict too
/// (`cannot lock ref 'refs/heads/htui/x': 'refs/heads/htui' exists`), which no sleep resolves.
#[must_use]
pub fn lock_signature(stderr: &str) -> Option<&str> {
    stderr.lines().map(str::trim_end).find(|line| {
        (line.contains("Unable to create '") && line.contains(".lock': File exists"))
            || line.contains("Unable to write index")
    })
}

/// `gix`'s own lock contention, behind [`create_branch`]'s `cannot create refs/heads/…: ` prefix:
/// `gix-lock`'s `PermanentlyLocked` (`gix-lock-24.0.0/src/acquire.rs:49-52`) or `gix-ref`'s
/// `LockAcquire` (`gix-ref-0.67.1/src/store/file/transaction/prepare.rs:487`).
fn gix_lock_signature(text: &str) -> bool {
    text.starts_with("cannot create refs/heads/")
        && ((text.contains("The lock for resource '") && text.contains("could not be obtained"))
            || text.contains("A lock could not be obtained for reference"))
}

/// Whether `err` is worth sleeping over (plan D39).
///
/// Two sources, as D39 names them: `gix`'s own lock errors, which `gix_lock_signature`
/// recognises (`gix` acquires with `Fail::Immediately` and never retries for us), and the `git`
/// CLI's, which [`lock_signature`] recognises. Exact shapes only: a bare `.lock` or `cannot lock`
/// substring also matches a ref directory/file conflict or any line that names `Cargo.lock`, and
/// retrying those three times only delays a failure that is permanent.
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
                && (lock_signature(text).is_some() || gix_lock_signature(text))
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

/// The first `cap` bytes pushed through it, and whether anything was dropped after them (D55).
#[derive(Debug)]
pub struct HeadBuffer {
    kept: Vec<u8>,
    cap: usize,
    overflowed: bool,
}

impl HeadBuffer {
    /// An empty buffer that will keep at most `cap` bytes.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            kept: Vec::new(),
            cap,
            overflowed: false,
        }
    }

    /// Appends as much of `chunk` as still fits, and records an overflow for the rest.
    pub fn push(&mut self, chunk: &[u8]) {
        let room = self.cap - self.kept.len();
        if chunk.len() > room {
            self.overflowed = true;
        }
        self.kept.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    /// The buffer as text, lossily, with `\n[diff truncated at 64 KiB]` appended when it
    /// overflowed.
    ///
    /// A cap that lands mid-codepoint yields a trailing replacement character before the marker.
    #[must_use]
    pub fn into_string(self) -> String {
        let mut text = String::from_utf8_lossy(&self.kept).into_owned();
        if self.overflowed {
            text.push_str(DIFF_TRUNCATED);
        }
        text
    }
}

/// One pipe's capture: a tail for everything but `diff`'s stdout, which keeps its head.
#[derive(Debug)]
enum Sink {
    Tail(TailBuffer),
    Head(HeadBuffer),
}

impl Sink {
    fn push(&mut self, chunk: &[u8]) {
        match self {
            Self::Tail(tail) => tail.push(chunk),
            Self::Head(head) => head.push(chunk),
        }
    }

    fn into_string(self) -> String {
        match self {
            Self::Tail(tail) => tail.into_string(),
            Self::Head(head) => head.into_string(),
        }
    }
}

/// Drains `reader` into `tail`, stopping at end of stream or at the first pipe error.
///
/// Into a shared buffer rather than a returned one, so that a read abandoned after the verb exited
/// ([`PIPE_GRACE`]) still leaves behind everything it had read. The `std` lock is held for one
/// `push` and never across an `.await`.
async fn read_tail(
    reader: Option<impl tokio::io::AsyncRead + Unpin>,
    tail: &std::sync::Mutex<Sink>,
) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = vec![0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => tail
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(&chunk[..read]),
        }
    }
}

/// The text a shared capture holds.
fn into_string(tail: std::sync::Mutex<Sink>) -> String {
    tail.into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .into_string()
}

/// Kills a supervised child's whole group and waits for it — for at most [`KILL_GRACE`].
///
/// Shared with `verify.rs`. A child that is not reaped in time is abandoned with a warning: the
/// caller has already decided it is over, and a `wait()` that never returns must not take the
/// verb, or the run, with it.
pub(crate) async fn kill_within_grace(
    child: &mut dyn process_wrap::tokio::ChildWrapper,
    what: &'static str,
) {
    if tokio::time::timeout(KILL_GRACE, Box::into_pin(child.kill()))
        .await
        .is_err()
    {
        tracing::warn!(
            what,
            "a killed child was not reaped within the grace; abandoning it"
        );
    }
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
// Synchronous, over `&Path`, returning owned values. Every async caller — `isolate/real.rs` and
// the verbs above — calls each of these through [`blocking`] with an owned `PathBuf`, so no
// `gix::Repository` — `Send` but not `Sync` (`gix-0.87.1/src/types.rs:148`) — is ever held across
// an `.await` (blueprint H-17).
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

/// Whether `path`'s object database holds `hex` as a commit (MOD-4 milestone 4 A-3, plan D74).
///
/// A `gix` read, not a verb: [`Cli::diff`] must run in a repository that holds the range's `after`,
/// and after a `copy` reconcile that is the merge commit, which only the primary holds.
///
/// # Errors
/// [`IsolateError::Git`] when the repository cannot be opened, `hex` is not a hash, or the lookup
/// fails.
pub fn has_commit(path: &Path, hex: &str) -> Result<bool, IsolateError> {
    let repo = open(path)?;
    let id = parse_oid(hex)?;
    let object = repo
        .try_find_object(id)
        .map_err(|err| IsolateError::Git(format!("cannot look up {hex}: {err}")))?;
    Ok(object.is_some_and(|object| object.kind == gix::object::Kind::Commit))
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

/// Whether `ancestor` is `descendant` itself or one of its ancestors (plan D136): some commit the
/// walk from `descendant` yields names it as a parent.
///
/// A `gix` read (`gix-0.87.1/src/repository/revision.rs:174`); `merge_base` would answer the
/// same question but sits behind the `revision` feature this workspace does not enable. The
/// walk hides `ancestor` (plan D142), because it runs under the repository's admin lock: it
/// yields only the commits above it, or above the point where the two lines meet. `gix` still
/// reads until the two histories meet, so an `ancestor` that shares no history with
/// `descendant` is read to its root. An `ancestor` this repository does not hold is not one.
///
/// # Errors
/// [`IsolateError::Git`] when either hash is not a hash, or the walk cannot read a commit.
pub fn is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> Result<bool, IsolateError> {
    ancestor_walk(path, ancestor, descendant).map(|(found, _)| found)
}

/// [`is_ancestor`]'s answer and the number of commits its walk yielded (plan D142's pin).
fn ancestor_walk(
    path: &Path,
    ancestor: &str,
    descendant: &str,
) -> Result<(bool, usize), IsolateError> {
    let (wanted, tip) = (parse_oid(ancestor)?, parse_oid(descendant)?);
    if wanted == tip {
        return Ok((true, 0));
    }
    if !has_commit(path, ancestor)? {
        return Ok((false, 0));
    }
    let repo = open(path)?;
    let fail = |err: &dyn std::fmt::Display| {
        IsolateError::Git(format!("cannot walk from {descendant}: {err}"))
    };
    // D142: the hidden `ancestor` is never yielded, but the commit above it on the way down is,
    // and that commit names it as a parent.
    let walk = repo
        .rev_walk([tip])
        .with_hidden([wanted])
        .all()
        .map_err(|err| fail(&err))?;
    let mut yielded = 0;
    for info in walk {
        let info = info.map_err(|err| fail(&err))?;
        yielded += 1;
        if info.parent_ids.contains(&wanted) {
            return Ok((true, yielded));
        }
    }
    Ok((false, yielded))
}

/// The merge on `head`'s first-parent history whose second parent is `after`, walking from
/// `head` back to `base` (which is not itself read), or `None` when there is none (plan D136).
///
/// That is what "this step is already merged" means once the primary may move past a merge:
/// another run's later merge sits on top of it, so `HEAD`'s own parents no longer name it. The
/// walk hides `base` (plan D142): when `base` is not on the first-parent line it stops where the
/// line meets `base`'s history, not at a root commit, because it runs under the repository's
/// admin lock. `gix` reads until the two histories meet, so a `base` that shares no history with
/// `head` is still read to its root.
///
/// # Errors
/// [`IsolateError::Git`] when a hash is not a hash or a commit on the walk cannot be read.
pub fn merge_of(
    path: &Path,
    head: &str,
    base: &str,
    after: &str,
) -> Result<Option<String>, IsolateError> {
    merge_walk(path, head, base, after).map(|(merge, _)| merge)
}

/// [`merge_of`]'s answer and the number of commits its walk read (plan D142's pin).
fn merge_walk(
    path: &Path,
    head: &str,
    base: &str,
    after: &str,
) -> Result<(Option<String>, usize), IsolateError> {
    let (tip, base, after) = (parse_oid(head)?, parse_oid(base)?, parse_oid(after)?);
    let repo = open(path)?;
    let fail =
        |err: &dyn std::fmt::Display| IsolateError::Git(format!("cannot walk from {head}: {err}"));
    let walk = repo
        .rev_walk([tip])
        .first_parent_only()
        .with_hidden([base])
        .all()
        .map_err(|err| fail(&err))?;
    let mut read = 0;
    for info in walk {
        let at = info.map_err(|err| fail(&err))?.id;
        // A first-parent walk lists the first parent only; the second is read off the commit.
        let commit = repo
            .find_commit(at)
            .map_err(|err| IsolateError::Git(format!("cannot find commit {at}: {err}")))?;
        read += 1;
        if commit
            .parent_ids()
            .nth(1)
            .is_some_and(|parent| parent == after)
        {
            return Ok((Some(at.to_hex().to_string()), read));
        }
    }
    Ok((None, read))
}

/// D25's merge message for `step`: what names a merge commit as that step's reconcile.
fn reconcile_message(step: StepId) -> String {
    format!("htui: reconcile {step}")
}

/// The first parent of `after` when `after` is `step`'s own reconcile merge on the primary
/// `checkout`, or `None` for any other commit (plan D141, D146).
///
/// D136 merges a step onto a primary that another run moved, so the row reads `(base, merge)`
/// and `base..merge` holds the other run's work too. The step's own change is the merge against
/// its first parent. Plan D146 ties the merge to the primary, git-only, and all four must hold:
/// `after` has exactly two parents `[first, second]`; its subject is D25's
/// `htui: reconcile <step>` (plan D147); `before` is `first` or an ancestor of it; and the merge
/// on the checkout's first-parent line, from its `HEAD` read now down to `before`, whose second
/// parent is `second` ([`merge_of`]) is `after` itself. The first two are cheap and checked first.
///
/// An agent's own two-parent commit with that message is never on the primary's first-parent
/// line (`merge --no-ff` puts the agent's tip on the second-parent side), so it answers `None`,
/// as does a step's own commit, an agent's merge, or a merge for another step. So does a commit
/// `checkout` does not hold: the merge exists only there, so this is always asked of the
/// checkout, never of a `copy` tree. A primary whose `HEAD` a user reset below the merge answers
/// `None` too, and the caller then diffs `before..after`, a superset (R-33's residual).
///
/// # Errors
/// [`IsolateError::Git`] when a hash is not a hash, or a commit or `HEAD` cannot be read.
pub fn reconcile_parent(
    checkout: &Path,
    before: &str,
    after: &str,
    step: StepId,
) -> Result<Option<String>, IsolateError> {
    let id = parse_oid(after)?;
    if !has_commit(checkout, after)? {
        return Ok(None);
    }
    let repo = open(checkout)?;
    let commit = repo
        .find_commit(id)
        .map_err(|err| IsolateError::Git(format!("cannot find commit {after}: {err}")))?;
    let parents: Vec<gix::ObjectId> = commit.parent_ids().map(gix::Id::detach).collect();
    let [first, second] = parents.as_slice() else {
        return Ok(None);
    };
    if subject(commit.message_raw_sloppy()) != reconcile_message(step).as_bytes() {
        return Ok(None);
    }
    let (first, second) = (first.to_hex().to_string(), second.to_hex().to_string());
    if !is_ancestor(checkout, before, &first)? {
        return Ok(None);
    }
    let head = head(checkout)?;
    let merge = merge_of(checkout, &head, before, &second)?;
    Ok((merge == Some(id.to_hex().to_string())).then_some(first))
}

/// Plan D147: a raw commit message's subject, up to its first `\n` and ASCII-trimmed, so a body
/// a `merge.log` shortlog or a commit-msg hook appends never hides D25's message.
fn subject(message: &[u8]) -> &[u8] {
    message
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default()
        .trim_ascii()
}

/// `Repository::is_dirty()` (`gix-0.87.1/src/status/mod.rs:168`), which is plan D24's predicate.
///
/// A change in the index against `HEAD` or in the working tree against the index, submodules
/// included, **untracked files excluded**. A reset leaves most untracked files alone, but not one
/// whose path the target tracks: `reset --hard` writes the tracked blob over it. So milestone 5's
/// reset to `before_hash` asks [`untracked_paths_base_tracks`] as well (plan D121).
///
/// # Errors
/// [`IsolateError::Git`] when the status walk fails.
pub fn is_dirty(path: &Path) -> Result<bool, IsolateError> {
    let repo = open(path)?;
    repo.is_dirty().map_err(|err| {
        IsolateError::Git(format!("cannot read status of {}: {err}", path.display()))
    })
}

/// Whether the working tree holds any untracked, not-ignored file — the half of `git status` that
/// [`is_dirty`] leaves out on purpose.
///
/// `Repository::is_dirty` runs its status walk with `dirwalk_options = None`
/// (`gix-0.87.1/src/status/mod.rs:196-198`), so a tree whose only change is a brand-new file reads
/// as clean. That is right for D24 and wrong for D27: `worktree remove --force --force` deletes an
/// untracked file for good (ANA-2 `:2065`, risk 2), so the removal guard asks this question too.
/// An ignored file is not counted — it is build output, and the ignore rules say so.
///
/// # Errors
/// [`IsolateError::Git`] when the status walk cannot be configured or fails part-way.
pub fn has_untracked_files(path: &Path) -> Result<bool, IsolateError> {
    let fail = |err: &dyn std::fmt::Display| {
        IsolateError::Git(format!(
            "cannot read untracked files of {}: {err}",
            path.display()
        ))
    };
    let repo = open(path)?;
    let items = repo
        .status(gix::progress::Discard)
        .map_err(|err| fail(&err))?
        // Explicit, so a `status.showUntrackedFiles=no` in the user's config cannot hide one.
        .untracked_files(gix::status::UntrackedFiles::Collapsed)
        .index_worktree_rewrites(None)
        .index_worktree_submodules(None)
        .into_index_worktree_iter(Vec::new())
        .map_err(|err| fail(&err))?;
    for item in items {
        if let gix::status::index_worktree::Item::DirectoryContents { entry, .. } =
            item.map_err(|err| fail(&err))?
            && entry.status == gix::dir::entry::Status::Untracked
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Every untracked, not-ignored file in `path`'s working tree that `reset --hard <base>` would
/// overwrite or delete, as repo-relative paths in walk order (plan D121).
///
/// [`is_dirty`] leaves untracked files out, and most of them survive a reset. One whose path
/// `base`'s tree tracks does not: `git reset --hard` writes the tracked blob over it without a
/// word. So does one below a path `base` tracks as a file, because the directory holding it is
/// replaced. Each untracked file (the walk is not collapsed, so a new directory yields its files)
/// is looked up in `base`'s tree; a path `base` does not track is not reported.
///
/// # Errors
/// [`IsolateError::Git`] when `base` is not a commit of the repository, or the status walk or a
/// tree lookup fails.
pub fn untracked_paths_base_tracks(path: &Path, base: &str) -> Result<Vec<String>, IsolateError> {
    let fail = |err: &dyn std::fmt::Display| {
        IsolateError::Git(format!(
            "cannot read untracked files of {}: {err}",
            path.display()
        ))
    };
    let repo = open(path)?;
    let tree = repo
        .find_commit(parse_oid(base)?)
        .map_err(|err| IsolateError::Git(format!("cannot find commit {base}: {err}")))?
        .tree()
        .map_err(|err| IsolateError::Git(format!("cannot read the tree of {base}: {err}")))?;
    let items = repo
        .status(gix::progress::Discard)
        .map_err(|err| fail(&err))?
        // Every file, not one entry per new directory: the lookup below is per path.
        .untracked_files(gix::status::UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .index_worktree_submodules(None)
        .into_index_worktree_iter(Vec::new())
        .map_err(|err| fail(&err))?;
    let mut found = Vec::new();
    for item in items {
        let gix::status::index_worktree::Item::DirectoryContents { entry, .. } =
            item.map_err(|err| fail(&err))?
        else {
            continue;
        };
        if entry.status != gix::dir::entry::Status::Untracked {
            continue;
        }
        let parts: Vec<&[u8]> = entry.rela_path.split(|byte| *byte == b'/').collect();
        // The full path tracked as anything, or a leading directory tracked as a file.
        for depth in 1..=parts.len() {
            let Some(tracked) = tree
                .lookup_entry(parts[..depth].iter().copied())
                .map_err(|err| fail(&err))?
            else {
                break;
            };
            if depth == parts.len() || !tracked.mode().is_tree() {
                found.push(entry.rela_path.to_string());
                break;
            }
        }
    }
    Ok(found)
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

    /// Removes `name` from the tree of `HEAD`'s commit, commits that on whatever `HEAD` names,
    /// deletes the working-tree file and leaves the index matching the new commit.
    ///
    /// # Panics
    /// When `HEAD` has no commit, or any step of the commit fails.
    pub fn commit_removal(repo: &Path, name: &str, message: &str) -> String {
        let repository = gix::open(repo).expect("the repository opens");
        let parent = repository.head_commit().expect("HEAD has a commit");
        let entries = parent
            .tree()
            .expect("the parent commit has a tree")
            .iter()
            .map(|entry| {
                let entry = entry.expect("the parent tree decodes");
                gix::objs::tree::Entry {
                    mode: entry.mode(),
                    filename: entry.filename().to_owned(),
                    oid: entry.oid().to_owned(),
                }
            })
            .filter(|entry| entry.filename != name)
            .collect::<Vec<_>>();

        let tree = repository
            .write_object(gix::objs::Tree { entries })
            .expect("the tree is written")
            .detach();
        let commit = repository
            .commit_as(who(), who(), "HEAD", message, tree, vec![parent.id])
            .expect("the commit is written")
            .detach();

        std::fs::remove_file(repo.join(name)).expect("the working-tree file is deleted");
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

    use super::testkit::{
        commit_file, commit_removal, empty_repo, has_object, repo_with_one_commit,
    };
    use super::{
        CAPTURE_TAIL, Cli, DIFF_CAP, Exited, HeadBuffer, MIN_GIT, SCRUBBED_ENV, TailBuffer,
        ancestor_walk, is_lock_error, merge_walk,
    };
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

    /// Plan D136's ancestor read: a commit is its own ancestor, a parent is one, and a child is
    /// not; neither is a commit on a line that never met this one.
    #[test]
    fn is_ancestor_follows_the_parents_only() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let first = repo_with_one_commit(dir.path());
        let second = commit_file(dir.path(), "g", "second\n", "two");
        let other = tempfile::tempdir().expect("a temporary directory");
        empty_repo(other.path());
        let unrelated = commit_file(other.path(), "x", "elsewhere\n", "a line of its own");

        let ancestor = |a: &str, d: &str| super::is_ancestor(dir.path(), a, d).expect("the walk");
        assert!(ancestor(&first, &second));
        assert!(ancestor(&second, &second));
        assert!(ancestor(&first, &first));
        assert!(!ancestor(&second, &first));
        // `unrelated` is not an object of this repository; the walk from `second` never meets it.
        assert!(!ancestor(&unrelated, &second));
    }

    /// An empty-tree commit on `parents` at `1600000000 + minute * 60`, written without moving any
    /// reference: history shaped by hand, with commit times that grow the way real ones do.
    fn commit_at(dir: &std::path::Path, parents: &[&str], minute: i64) -> String {
        let repo = gix::open(dir).expect("the repository opens");
        let time = format!("{} +0000", 1_600_000_000 + minute * 60);
        let who = gix::actor::SignatureRef {
            name: gix::bstr::BStr::new(b"htui test"),
            email: gix::bstr::BStr::new(b"test@localhost"),
            time: &time,
        };
        let parents: Vec<gix::ObjectId> = parents
            .iter()
            .map(|hex| gix::ObjectId::from_hex(hex.as_bytes()).expect("a hex object id"))
            .collect();
        let tree = gix::ObjectId::empty_tree(gix::hash::Kind::Sha1);
        repo.new_commit_as(who, who, format!("minute {minute}"), tree, parents)
            .expect("the commit is written")
            .id
            .to_hex()
            .to_string()
    }

    /// Forty commits in a line, each a minute after the last; the last one is returned.
    fn long_history(dir: &std::path::Path) -> String {
        empty_repo(dir);
        let mut tip = commit_at(dir, &[], 0);
        for minute in 1..40 {
            tip = commit_at(dir, &[&tip], minute);
        }
        tip
    }

    /// Plan D142 (review M-B): `is_ancestor` runs under the repository's admin lock, so it must
    /// not walk the history below the ancestor it looks for. Two commits sit above `base`, forty
    /// below; the walk yields the two, whether `base` is on the line or beside it. The side
    /// commit is the case that walked to the root before; on the line, the old walk stopped on
    /// meeting `base`. The count is of commits the walk yields: gix's paint of the hidden
    /// frontier reads a little further, and stops once every queued commit is stale.
    #[test]
    fn is_ancestor_never_walks_below_the_ancestor() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let base = long_history(dir.path());
        let side = commit_at(dir.path(), &[&base], 41);
        let above = commit_at(dir.path(), &[&base], 42);
        let head = commit_at(dir.path(), &[&above], 43);

        let (found, yielded) = ancestor_walk(dir.path(), &base, &head).expect("the walk");
        assert!(found, "the base is under HEAD");
        assert!(
            yielded <= 2,
            "the walk stopped at the base: {yielded} commits"
        );
        let (found, yielded) = ancestor_walk(dir.path(), &side, &head).expect("the walk");
        assert!(!found, "a commit beside HEAD's line is not under it");
        assert!(
            yielded <= 2,
            "the walk stopped at the fork: {yielded} commits"
        );
    }

    /// Plan D142 (review M-B): `merge_of` walks `HEAD`'s first-parent line down to `base`, and a
    /// `base` that is not on that line — `HEAD` went a way of its own from an older commit — must
    /// stop the walk where the two lines meet, not at the root forty commits further down.
    #[test]
    fn merge_of_never_walks_below_the_base() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let fork = long_history(dir.path());
        let base = commit_at(dir.path(), &[&fork], 41);
        let tip = commit_at(dir.path(), &[&base], 42);
        let head = commit_at(dir.path(), &[&fork], 43);
        let merge = commit_at(dir.path(), &[&base, &tip], 44);

        let (found, read) = merge_walk(dir.path(), &head, &base, &tip).expect("the walk");
        assert_eq!(found, None, "HEAD's line holds no merge of the tip");
        assert!(read <= 1, "the walk stopped at the fork: {read} commits");
        let (found, read) = merge_walk(dir.path(), &merge, &base, &tip).expect("the walk");
        assert_eq!(found.as_deref(), Some(merge.as_str()), "the merge is found");
        assert!(read <= 1, "and nothing under it is read: {read} commits");
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
    /// are not a change; the one a reset would overwrite is D121's question, not this one.
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

    /// D27's other half: an untracked file is what `is_dirty` leaves out, and exactly what
    /// `has_untracked_files` sees; an ignored one is build output and counts for neither.
    #[test]
    fn has_untracked_files_sees_a_new_file_and_not_an_ignored_one() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        commit_file(dir.path(), ".gitignore", "target/\n", "ignore target");
        assert!(!super::has_untracked_files(dir.path()).expect("status reads"));

        std::fs::create_dir(dir.path().join("target")).expect("the directory is made");
        std::fs::write(dir.path().join("target").join("out"), "built\n").expect("written");
        assert!(
            !super::has_untracked_files(dir.path()).expect("status reads"),
            "an ignored file is not work"
        );

        std::fs::create_dir(dir.path().join("new")).expect("the directory is made");
        std::fs::write(dir.path().join("new").join("notes"), "work\n").expect("written");
        assert!(
            super::has_untracked_files(dir.path()).expect("status reads"),
            "an untracked file in an untracked directory is"
        );
        assert!(!super::is_dirty(dir.path()).expect("status reads"), "D24");
    }

    /// D121: an untracked file whose path the base tracks is what `reset --hard <base>` would
    /// overwrite, and exactly what the helper reports; `is_dirty` reads the tree as clean.
    #[test]
    fn untracked_paths_base_tracks_sees_a_file_at_a_path_the_base_tracks() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let base = repo_with_one_commit(dir.path());
        commit_removal(dir.path(), "f", "the agent deletes f");
        assert!(
            super::untracked_paths_base_tracks(dir.path(), &base)
                .expect("status reads")
                .is_empty(),
            "a deleted file is not an untracked one"
        );

        std::fs::write(dir.path().join("f"), "the maintainer's notes\n").expect("written");
        assert_eq!(
            super::untracked_paths_base_tracks(dir.path(), &base).expect("status reads"),
            vec!["f".to_owned()]
        );
        assert!(!super::is_dirty(dir.path()).expect("status reads"), "D24");
    }

    /// D121: an untracked file the base does not track survives the reset and is not reported;
    /// `HEAD`'s own tree does not decide, the base's does.
    #[test]
    fn untracked_paths_base_tracks_ignores_a_path_the_base_does_not_track() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        let base = commit_file(dir.path(), ".gitignore", "target\n", "ignore target");
        std::fs::write(dir.path().join("untracked"), "noise\n").expect("written");
        std::fs::create_dir(dir.path().join("new")).expect("the directory is made");
        std::fs::write(dir.path().join("new").join("notes"), "work\n").expect("written");
        std::fs::write(dir.path().join("target"), "built\n").expect("written");

        assert!(
            super::untracked_paths_base_tracks(dir.path(), &base)
                .expect("status reads")
                .is_empty()
        );
    }

    /// D121: a file inside an untracked directory whose name the base tracks as a file is lost
    /// too, because the reset replaces the directory with the base's blob.
    #[test]
    fn untracked_paths_base_tracks_sees_a_file_below_a_path_the_base_tracks_as_a_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let base = repo_with_one_commit(dir.path());
        commit_removal(dir.path(), "f", "the agent deletes f");
        std::fs::create_dir(dir.path().join("f")).expect("the directory is made");
        std::fs::write(dir.path().join("f").join("x"), "work\n").expect("written");

        assert_eq!(
            super::untracked_paths_base_tracks(dir.path(), &base).expect("status reads"),
            vec!["f/x".to_owned()]
        );
    }

    /// D121: an untracked file where the base tracks a directory is lost too, because the reset
    /// replaces the file with the base's directory; the full path is reported whatever its kind.
    #[test]
    fn untracked_paths_base_tracks_sees_a_file_at_a_path_the_base_tracks_as_a_directory() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        let repository = gix::open(dir.path()).expect("the repository opens");
        let head = repository.head_commit().expect("HEAD has a commit");
        let agent_tree = head.tree().expect("the commit has a tree").id;
        let blob = repository
            .write_blob(b"inside\n")
            .expect("the blob is written")
            .detach();
        let sub = repository
            .write_object(gix::objs::Tree {
                entries: vec![gix::objs::tree::Entry {
                    mode: gix::objs::tree::EntryKind::Blob.into(),
                    filename: "x".into(),
                    oid: blob,
                }],
            })
            .expect("the subtree is written")
            .detach();
        let mut entries = head
            .tree()
            .expect("the commit has a tree")
            .iter()
            .map(|entry| {
                let entry = entry.expect("the tree decodes");
                gix::objs::tree::Entry {
                    mode: entry.mode(),
                    filename: entry.filename().to_owned(),
                    oid: entry.oid().to_owned(),
                }
            })
            .collect::<Vec<_>>();
        entries.push(gix::objs::tree::Entry {
            mode: gix::objs::tree::EntryKind::Tree.into(),
            filename: "d".into(),
            oid: sub,
        });
        entries.sort();
        let base_tree = repository
            .write_object(gix::objs::Tree { entries })
            .expect("the tree is written")
            .detach();
        let who = gix::actor::SignatureRef {
            name: gix::bstr::BStr::new(b"test"),
            email: gix::bstr::BStr::new(b"test@localhost"),
            time: "1600000000 +0000",
        };
        let base = repository
            .commit_as(
                who,
                who,
                "HEAD",
                "base tracks d/x",
                base_tree,
                vec![head.id],
            )
            .expect("the commit is written")
            .detach();
        // The agent deletes `d/`: HEAD's tree and the index go back to the one without it.
        repository
            .commit_as(
                who,
                who,
                "HEAD",
                "the agent deletes d",
                agent_tree,
                vec![base],
            )
            .expect("the commit is written");
        repository
            .index_from_tree(&agent_tree)
            .expect("an index is built from the tree")
            .write(gix::index::write::Options::default())
            .expect("the index is written");

        std::fs::write(dir.path().join("d"), "the maintainer's notes\n").expect("written");
        assert_eq!(
            super::untracked_paths_base_tracks(dir.path(), &base.to_hex().to_string())
                .expect("status reads"),
            vec!["d".to_owned()]
        );
    }

    /// D121: an ignored file is not reported even at a path the base tracks; only untracked,
    /// not-ignored files are the maintainer's work a reset must not overwrite.
    #[test]
    fn untracked_paths_base_tracks_skips_an_ignored_file_at_a_path_the_base_tracks() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        let base = commit_file(dir.path(), "target", "tracked\n", "base tracks target");
        commit_removal(dir.path(), "target", "the agent deletes target");
        commit_file(
            dir.path(),
            ".gitignore",
            "target\n",
            "the agent ignores target",
        );
        std::fs::write(dir.path().join("target"), "built\n").expect("written");

        assert!(
            super::untracked_paths_base_tracks(dir.path(), &base)
                .expect("status reads")
                .is_empty(),
            "an ignored file is not reported"
        );
    }

    /// A base that is not a commit of the repository is an error, not an empty answer.
    #[test]
    fn untracked_paths_base_tracks_refuses_an_unknown_base() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        repo_with_one_commit(dir.path());
        let missing = "0123456789012345678901234567890123456789";
        assert!(matches!(
            super::untracked_paths_base_tracks(dir.path(), missing),
            Err(IsolateError::Git(_))
        ));
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

    /// D39 retries a held lock and nothing else. `.lock` or `cannot lock` as bare substrings
    /// also match a directory/file ref conflict and any message that names `Cargo.lock`, and
    /// neither goes away by sleeping; only the exact shapes do.
    #[test]
    fn only_the_exact_lock_shapes_are_retried() {
        for (label, text) in [
            (
                "git's ref lock",
                "git worktree add: fatal: cannot lock ref 'refs/heads/htui/x': Unable to create \
                 '/r/.git/refs/heads/htui/x.lock': File exists.",
            ),
            (
                "git's index lock",
                "git reset --hard: fatal: Unable to create '/r/.git/index.lock': File exists.",
            ),
            (
                "merge's index lock",
                "git merge: error: Unable to write index.",
            ),
            (
                "gix's resource lock",
                "cannot create refs/heads/htui/x: The lock for resource \
                 '/r/.git/refs/heads/htui/x' could not be obtained immediately after 1 attempt(s).",
            ),
            (
                "gix-ref's reference lock",
                "cannot create refs/heads/htui/x: A lock could not be obtained for reference \
                 \"refs/heads/htui/x\"",
            ),
        ] {
            assert!(
                is_lock_error(&IsolateError::Git(text.to_owned())),
                "{label} is a lock"
            );
        }
        for (label, text) in [
            (
                "a directory/file ref conflict",
                "git worktree add: fatal: cannot lock ref 'refs/heads/htui/x': 'refs/heads/htui' \
                 exists; cannot create 'refs/heads/htui/x'",
            ),
            (
                "a message naming Cargo.lock",
                "git merge: error: Your local changes to the following files would be \
                 overwritten by merge: Cargo.lock",
            ),
            (
                "a checkout naming a lock file",
                "git reset --hard: error: unable to unlink old 'Cargo.lock': Permission denied",
            ),
            (
                "gix, but not about a lock",
                "cannot create refs/heads/htui/x: The reference \"refs/heads/htui/x\" should \
                 not exist, the lockfile is fine",
            ),
        ] {
            assert!(
                !is_lock_error(&IsolateError::Git(text.to_owned())),
                "{label} is not a lock"
            );
        }
    }

    /// A child that daemonises keeps the inherited pipes open after the verb itself exited 0. The
    /// budget covers the verb, not whatever it left behind: the exit is reported, not a timeout.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_verb_whose_grandchild_holds_the_pipe_still_reports_its_exit() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let sleep = which::which("sleep").expect("a box that can run a test suite has `sleep`");
        let script = format!(
            "if [ \"$1\" = \"--version\" ]; then echo 'git version 2.43.0'; exit 0; fi\n\
             '{}' 6 &\n\
             echo done\n\
             exit 0\n",
            sleep.display()
        );
        let binary = fake_git(dir.path(), "daemon-git", &script);
        let cli = Cli::probe(binary)
            .expect("the script answers --version")
            .with_budget(Duration::from_secs(3));

        let started = std::time::Instant::now();
        let exited = cli
            .run("worktree add", dir.path(), &[OsStr::new("go")], &[])
            .await
            .expect("the verb exited 0 and says so");
        assert!(exited.ok(), "{exited:?}");
        assert_eq!(
            exited.stdout, "done\n",
            "what it wrote before exiting is kept"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "it did not wait for the grandchild: {:?}",
            started.elapsed()
        );
    }

    /// `GixIsolator::new` runs the probe synchronously at worker start, so a `git` that hangs on
    /// `--version` must be refused within a bound rather than hang the worker with it.
    #[cfg(unix)]
    #[test]
    fn a_probe_that_hangs_is_refused_within_its_bound() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let sleep = which::which("sleep").expect("a box that can run a test suite has `sleep`");
        let hung = fake_git(
            dir.path(),
            "hung-git",
            &format!("exec '{}' 30", sleep.display()),
        );
        let started = std::time::Instant::now();
        let err = Cli::probe_within(hung, Duration::from_millis(300))
            .expect_err("the probe outlived its bound");
        assert_eq!(
            err.to_string(),
            "isolation refused: git --version did not answer within 300ms"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    /// D39's schedule, both of its sources, and the two classes it must not touch.
    #[tokio::test(start_paused = true)]
    async fn with_retry_retries_three_times_on_a_lock_error_and_not_on_others() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let attempts = AtomicU32::new(0);
        let err = super::with_retry("worktree add", || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(IsolateError::Git(
                "git worktree add: fatal: cannot lock ref 'refs/heads/htui/x': Unable to create \
                 '/r/.git/refs/heads/htui/x.lock': File exists."
                    .to_owned(),
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

    /// The blocking helper hands back the task's own answer, and a task that panicked is a git
    /// error for the caller rather than a panic in it. On the `current_thread` runtime
    /// `#[tokio::test]` uses, which is where a sync status walk would stall everything.
    #[tokio::test]
    async fn blocking_returns_the_answer_and_maps_a_panic_to_an_error() {
        assert_eq!(
            super::blocking(|| Ok(7)).await.expect("the task answers"),
            7
        );
        let err = super::blocking::<(), _>(|| panic!("the walk fell over"))
            .await
            .expect_err("a panicked task is an error");
        assert!(
            matches!(&err, IsolateError::Git(text) if text.starts_with("a blocking git task did not finish")),
            "{err}"
        );
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

    /// The doc of `merge_no_ff` promises every failing path aborts the half-merge. A conflicted-
    /// paths read that fails must not skip the abort and leave `MERGE_HEAD` and the stage entries
    /// in the user's primary tree; its error surfaces only after the primary is restored.
    #[tokio::test]
    async fn a_failed_conflict_read_still_aborts_the_merge() {
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
            .merge_no_ff_reading(&repo, step, &before, &after, |_| {
                Err(IsolateError::Git("the index could not be read".to_owned()))
            })
            .await
            .expect_err("the read failed");
        assert_eq!(err.to_string(), "git: the index could not be read");
        assert!(
            !repo.join(".git").join("MERGE_HEAD").exists(),
            "--abort ran although the read failed"
        );
        assert_eq!(super::head(&repo).expect("HEAD reads"), before);
        assert!(!super::is_dirty(&repo).expect("status reads"), "and clean");
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

    /// The oracle for D55's verb: the very argv, spawned by the test itself under the pinned
    /// `COLUMNS` (A-4), with M3's scrubbed environment.
    fn oracle_diff(
        git: &Cli,
        repo: &std::path::Path,
        before: &str,
        after: &str,
        stat: bool,
    ) -> String {
        let mut oracle = std::process::Command::new(git.binary());
        oracle.current_dir(repo).args([
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ]);
        if stat {
            oracle.arg("--stat");
        }
        oracle.args([before, after, "--"]);
        for key in SCRUBBED_ENV {
            oracle.env_remove(key);
        }
        let out = oracle
            .env("LC_ALL", "C")
            .env("COLUMNS", "80")
            .output()
            .expect("the oracle runs");
        assert!(out.status.success(), "the oracle failed: {out:?}");
        String::from_utf8(out.stdout).expect("the oracle's output is text")
    }

    /// D55's verb is the one whose stdout is the product, so its output is checked byte for byte
    /// against `git`'s own, patch and stat both.
    #[tokio::test]
    async fn diff_and_diff_stat_of_a_range_match_git_s_own_output() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let before = repo_with_one_commit(dir.path());
        commit_file(dir.path(), "f", "first\nsecond\n", "edit f");
        let after = commit_file(
            dir.path(),
            "a-rather-long-file-name-that-a-narrow-terminal-would-cut.txt",
            "new\n",
            "add",
        );

        let patch = git
            .diff(dir.path(), &before, &after, false)
            .await
            .expect("the patch reads");
        let stat = git
            .diff(dir.path(), &before, &after, true)
            .await
            .expect("the stat reads");
        assert_eq!(patch, oracle_diff(&git, dir.path(), &before, &after, false));
        assert_eq!(stat, oracle_diff(&git, dir.path(), &before, &after, true));
        assert!(patch.starts_with("diff --git a/"), "{patch}");
        assert!(stat.contains("2 files changed"), "{stat}");
        assert!(
            stat.contains("a-rather-long-file-name-that-a-narrow-terminal-would-cut.txt"),
            "COLUMNS=80 keeps the name whole: {stat}"
        );
    }

    /// D55: `--no-ext-diff` keeps `diff.external` out and the explicit prefixes beat
    /// `diff.noprefix`, whatever the repository's config says.
    #[cfg(unix)]
    #[tokio::test]
    async fn diff_ignores_an_external_diff_driver_and_noprefix() {
        use std::io::Write as _;

        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).expect("the repository directory is made");
        let before = repo_with_one_commit(&repo);
        let after = commit_file(&repo, "f", "edited\n", "edit f");

        let driver = fake_git(dir.path(), "external-diff", "echo EXTERNAL; exit 0");
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(repo.join(".git").join("config"))
            .expect("the config opens");
        write!(
            config,
            "[diff]\n\texternal = {}\n\tnoprefix = true\n",
            driver.display()
        )
        .expect("the config is written");
        drop(config);

        let patch = git
            .diff(&repo, &before, &after, false)
            .await
            .expect("the patch reads");
        assert!(!patch.contains("EXTERNAL"), "{patch}");
        assert!(patch.starts_with("diff --git a/f b/f\n"), "{patch}");
        assert!(patch.contains("\n--- a/f\n+++ b/f\n"), "{patch}");
    }

    /// An empty range is an empty answer, not an error.
    #[tokio::test]
    async fn diff_of_an_empty_range_is_empty() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let head = repo_with_one_commit(dir.path());
        for stat in [false, true] {
            assert_eq!(
                git.diff(dir.path(), &head, &head, stat)
                    .await
                    .expect("the empty range reads"),
                ""
            );
        }
    }

    /// D55: a head cap, not a tail one — a long patch keeps its first `diff --git` header and says
    /// it was cut.
    #[tokio::test]
    async fn a_diff_over_the_cap_keeps_its_head_and_says_it_was_truncated() {
        let Some(git) = crate::skip_without_git!() else {
            return;
        };
        let dir = tempfile::tempdir().expect("a temporary directory");
        let before = repo_with_one_commit(dir.path());
        let body: String = (0..4_000_u32)
            .map(|line| format!("{line:0>40}\n"))
            .collect();
        let after = commit_file(dir.path(), "big", &body, "big");

        let patch = git
            .diff(dir.path(), &before, &after, false)
            .await
            .expect("the patch reads");
        assert!(
            patch.starts_with("diff --git a/big b/big\n"),
            "{}",
            &patch[..80]
        );
        assert!(patch.ends_with("\n[diff truncated at 64 KiB]"));
        assert!(patch.len() <= DIFF_CAP + "\n[diff truncated at 64 KiB]".len() + 3);
    }

    /// The head buffer keeps the first bytes and records that it overflowed.
    #[test]
    fn head_buffer_keeps_the_first_64_kib() {
        let mut head = HeadBuffer::new(DIFF_CAP);
        for index in 0..200_u32 {
            head.push(format!("{index:0>1024}\n").as_bytes());
        }
        let kept = head.into_string();
        assert!(kept.starts_with("0000"), "the start survives");
        assert!(!kept.contains("0199\n"), "the end does not");
        assert_eq!(kept.len(), DIFF_CAP + "\n[diff truncated at 64 KiB]".len());
        assert!(kept.ends_with("\n[diff truncated at 64 KiB]"));

        let mut head = HeadBuffer::new(8);
        head.push(b"abcdefghijklm");
        assert_eq!(head.into_string(), "abcdefgh\n[diff truncated at 64 KiB]");

        let mut exact = HeadBuffer::new(4);
        exact.push(b"ab");
        exact.push(b"cd");
        assert_eq!(
            exact.into_string(),
            "abcd",
            "exactly the cap is not an overflow"
        );
    }

    /// A-3's read: whether an object database holds a commit, with no subprocess.
    #[test]
    fn has_commit_answers_for_this_odb_only() {
        let first = tempfile::tempdir().expect("a temporary directory");
        let second = tempfile::tempdir().expect("a temporary directory");
        let base = repo_with_one_commit(first.path());
        let tip = commit_file(first.path(), "g", "second\n", "two");
        repo_with_one_commit(second.path());
        assert!(super::has_commit(first.path(), &tip).expect("the read succeeds"));
        assert!(super::has_commit(first.path(), &base).expect("the read succeeds"));
        assert!(!super::has_commit(second.path(), &tip).expect("the read succeeds"));
        assert!(super::has_commit(first.path(), "not-hex").is_err());
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
