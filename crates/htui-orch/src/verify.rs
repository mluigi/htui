//! Stage 5's other side effect: `verify_command`, and the three outcomes ANA-2 fixes for it.
//!
//! `verify_command` runs after the session and before the gate, in the step's own tree for the
//! project's primary repo (`docs/ANA-2.md:491-493`). What it produces is a three-valued fact —
//! `pass`, `fail`, `unavailable` — that lands in `run_step.verify_outcome` and
//! `run_step.verify_exit_code` (`:508-515`) and, in more detail, in a `command_run` row with
//! `class = 'verify'` (plan D31). This module produces the fact; the engine persists it (T6).
//!
//! **A phase with no `verify_command` is not `unavailable`.** It is the normal case — every
//! seeded phase is one (`crates/htui-core/src/seed.rs`) — and it produces no report at all, so
//! both columns stay `NULL` and milestone 2's settle rule (`ok` is an output document plus
//! `verify_outcome != fail`, `docs/ANA-2.md:438`) is untouched. `unavailable` is reserved for a
//! command that was asked for and could not be run, and it never fails a step (`:443`).
//!
//! The shell is plan D30's: `sh -c` on Unix, `cmd /C` on Windows, with the process environment
//! unchanged, stdout and stderr merged and tail-capped at 64 KiB, masked by the engine's
//! `Scrubber` before it is handed back, under a semaphore keyed `verify`, and with the step
//! deadline's remainder as the timeout. The process handling is `isolate/git.rs`'s
//! [`Cli`](crate::isolate::git::Cli) contract verb for verb — a supervised child whose whole group
//! a kill reaches, both pipes read concurrently so neither can fill and stall it, and the cap kept
//! at the **tail**, because the end of a build that failed is the part worth reading.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::model::{StepId, VerifyOutcome};
use htui_core::scrub::Scrubber;
use tokio::io::AsyncReadExt as _;

use crate::isolate::Clock;
use crate::isolate::git::{CAPTURE_TAIL, TailBuffer};

/// `command_run.class` for every row this milestone writes, and the `command_limits` key the
/// class semaphore is sized from (`docs/ANA-2.md:501-505`, `0003_orchestration.sql:129`).
pub const VERIFY_CLASS: &str = "verify";

/// The permit count used when `command_limits` names no `verify` — the shipped default.
pub const DEFAULT_VERIFY_LIMIT: u32 = 1;

/// The shell `verify_command` is handed to, and the flag that makes it read a command string.
///
/// Plan D30: ANA-2 never says how a `TEXT` becomes an argv, and `cargo test --workspace` — the
/// shape every seeded template assumes — only works under a reading where quoting does.
#[cfg(unix)]
const SHELL: (&str, &str) = ("sh", "-c");
/// The Windows shell, same contract (plan D30).
#[cfg(windows)]
const SHELL: (&str, &str) = ("cmd", "/C");

// The reasons an `unavailable` report carries, one named function per sentence — the house style
// of `htui_core::store::traits`'s refusals and of `isolate/git.rs`'s failure classes. The operator
// reads these out of `command_run.output`, so a rewording is a one-line change here.

/// The scope had no tree for the repo that `is_primary`, so there is nowhere to run (plan D30).
#[must_use]
pub fn no_primary_tree() -> String {
    "no primary tree".to_owned()
}

/// The step deadline's remainder ran out — in the class queue, or during the run itself
/// (`docs/ANA-2.md:515`).
///
/// The step still settles `failed` one stage later, and on the deadline rather than on this: the
/// remainder this verifier spent is exactly what was left of `deadline_seconds`, so `settle`'s own
/// elapsed-deadline rule has tripped by the time it reads the outcome. `unavailable` never fails a
/// step (`:443`) and does not have to here.
#[must_use]
pub fn deadline_elapsed() -> String {
    "deadline elapsed".to_owned()
}

/// The child died without an exit code, so there is no `verify_exit_code` to record.
#[must_use]
pub fn killed_by_signal() -> String {
    "killed by signal".to_owned()
}

/// The shell itself could not be started: it is not on `PATH`, or the tree is gone.
fn cannot_spawn(shell: &Path, err: &std::io::Error) -> String {
    format!("cannot spawn {}: {err}", shell.display())
}

/// The child was started and then could not be waited for.
fn cannot_wait(err: &std::io::Error) -> String {
    format!("the shell could not be waited for: {err}")
}

/// `R-SEC-3` is fail-closed: an output the scrubber could not mask is not persistable, so the text
/// is dropped and its size is all that is reported. The exit code is a fact and is kept.
fn scrub_refused(bytes: usize) -> String {
    format!("<scrub refused: {bytes} bytes withheld>")
}

/// The boxed future every [`Verifier`] returns — [`IsolatorFuture`](crate::isolate::IsolatorFuture)'s
/// shape, and infallible: a verify that cannot run is a report, not an error (`docs/ANA-2.md:515`).
pub type VerifierFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What stage 5 hands the verifier (plan D30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRequest {
    /// `SnapshotPhase::verify_command`; `None` yields no report at all, which is the normal case.
    pub command: Option<String>,
    /// The primary repo's tree; `None` is `unavailable` with [`no_primary_tree`].
    pub cwd: Option<PathBuf>,
    /// The step deadline's remainder; `None` is no deadline, `Some(ZERO)` is already elapsed.
    pub remaining: Option<Duration>,
    /// The step this verify belongs to — `command_run.run_step_id`, and the only thing in a
    /// request that identifies it in a log line.
    pub step: StepId,
}

/// One run of the command, in `command_run`'s shape (plan D31 builds the row from this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// `run_step.verify_outcome`.
    pub outcome: VerifyOutcome,
    /// `run_step.verify_exit_code`: the code for [`VerifyOutcome::Pass`] and
    /// [`VerifyOutcome::Fail`], `None` for [`VerifyOutcome::Unavailable`], which never had one.
    pub exit_code: Option<i32>,
    /// Scrubbed and tail-capped: stdout and stderr merged, or the `unavailable` reason.
    pub output: String,
    /// When the command started — after the class permit was taken, so this is the instant the
    /// child existed and not the instant it was queued (`command_run.queued_at` is the engine's).
    pub started_at: DateTime<Utc>,
    /// When it ended, however it ended.
    pub finished_at: DateTime<Utc>,
}

/// Milestone 2 D6's pattern applied to the walk's other side effect: a seam now, implementations
/// by kind. [`ShellVerifier`] is the real one; `fake::FakeVerifier` scripts outcomes for the
/// conformance suite without a process (plan D41).
pub trait Verifier: Send + Sync + fmt::Debug {
    /// Runs `request`, or answers `None` when it names no command.
    ///
    /// Infallible by construction: every way a command can fail to run is one of the three
    /// outcomes, and `unavailable` is the one that carries the reason.
    fn run<'a>(&'a self, request: VerifyRequest) -> VerifierFuture<'a, Option<VerifyReport>>;
}

/// Plan D30's verifier: the platform shell in the primary tree, one permit of the `verify` class,
/// the deadline's remainder as the timeout, the output merged, capped, scrubbed.
///
/// Built once per process by the caller and shared: the semaphore is what makes a fan-out of four
/// `cargo test` runs on one box queue (`docs/ANA-2.md:502-506`), so a verifier per step would
/// defeat the only thing it is there for.
pub struct ShellVerifier {
    /// The `verify` class semaphore, sized from `command_limits` (`docs/ANA-2.md:504-506`).
    permits: Arc<tokio::sync::Semaphore>,
    /// `R-SEC-3`'s masker, the engine's own, so a verify output and a prompt are masked alike.
    scrubber: Arc<dyn Scrubber>,
    /// Plan D8's clock: every instant in a report comes from the caller's, never from `Utc::now`.
    clock: Arc<dyn Clock>,
    /// The shell binary. A field rather than a constant so a test can point it at something that
    /// does not exist and reach the spawn-refused class; `std::env::set_var` is `unsafe` in
    /// edition 2024 and this workspace forbids `unsafe`, so `PATH=""` is not available to a test.
    shell: PathBuf,
}

impl fmt::Debug for ShellVerifier {
    /// Hand written for one field: [`Clock`] carries no `Debug` supertrait
    /// (`crates/htui-orch/src/isolate.rs`), so `Arc<dyn Clock>` cannot be derived through. The
    /// same shape `EngineParts` uses for its driver factory.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellVerifier")
            .field("permits", &self.permits.available_permits())
            .field("scrubber", &self.scrubber)
            .field("shell", &self.shell)
            .finish_non_exhaustive()
    }
}

impl ShellVerifier {
    /// `limits` is the resolved `command_limits` map (`BoxSettings::command_limits`, falling
    /// through to the `app_setting` default `{"build":1,"test":4,"verify":1}`).
    ///
    /// A `verify` of `0` is read as `1`: a zero-permit semaphore never admits anyone, so a run
    /// with no deadline would wait on it forever and one with a deadline would spend its whole
    /// remainder to produce `unavailable`. ANA-2 gives the class limit no "off" meaning, and
    /// `R-MCP-3`'s limits are about queueing, not about disabling a phase's own command.
    #[must_use]
    pub fn new(
        limits: &BTreeMap<String, u32>,
        scrubber: Arc<dyn Scrubber>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let limit = limits
            .get(VERIFY_CLASS)
            .copied()
            .unwrap_or(DEFAULT_VERIFY_LIMIT)
            .max(1);
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(limit as usize)),
            scrubber,
            clock,
            shell: PathBuf::from(SHELL.0),
        }
    }

    /// The same verifier, spawning `shell` instead of the platform one — the only way a test can
    /// reach the spawn-refused class without touching the process environment.
    #[cfg(test)]
    fn with_shell(self, shell: impl Into<PathBuf>) -> Self {
        Self {
            shell: shell.into(),
            ..self
        }
    }

    /// One report, stamped at a single instant: every outcome that never spawned anything.
    fn without_running(&self, reason: String) -> VerifyReport {
        let at = self.clock.now();
        VerifyReport {
            outcome: VerifyOutcome::Unavailable,
            exit_code: None,
            output: self.scrubbed(reason),
            started_at: at,
            finished_at: at,
        }
    }

    /// One report for a command that did start, whatever became of it.
    fn finished(
        &self,
        outcome: VerifyOutcome,
        exit_code: Option<i32>,
        output: String,
        started_at: DateTime<Utc>,
    ) -> VerifyReport {
        VerifyReport {
            outcome,
            exit_code,
            output: self.scrubbed(output),
            started_at,
            finished_at: self.clock.now(),
        }
    }

    /// `text` masked by the engine's scrubber, or nothing at all when it refused (`R-SEC-3`).
    ///
    /// The refusal drops the text and keeps the outcome and the exit code: those are facts about
    /// the run, and neither can carry a secret.
    fn scrubbed(&self, text: String) -> String {
        let bytes = text.len();
        let mut value = serde_json::Value::String(text);
        match self.scrubber.scrub(&mut value) {
            Ok(()) => match value {
                serde_json::Value::String(text) => text,
                other => other.to_string(),
            },
            Err(refusal) => {
                tracing::warn!(%refusal, "verify output withheld: the scrubber refused it");
                scrub_refused(bytes)
            }
        }
    }

    /// The whole of plan D30, in the order its steps happen.
    async fn execute(&self, request: VerifyRequest) -> Option<VerifyReport> {
        let command = request.command?;
        let Some(cwd) = request.cwd else {
            return Some(self.without_running(no_primary_tree()));
        };
        if request.remaining == Some(Duration::ZERO) {
            return Some(self.without_running(deadline_elapsed()));
        }

        // One permit of the `verify` class, which is what makes a fan-out of four `cargo test`
        // runs on one box queue rather than thrash it (`docs/ANA-2.md:502-506`). Held until the
        // report is stamped, so two runs of a one-permit class never overlap.
        //
        // The wait counts against the remainder (plan D30): a step whose deadline expires while
        // it is queued behind three other `cargo test`s never gets to start one of its own.
        let queued_at = tokio::time::Instant::now();
        let _permit = match request.remaining {
            Some(remaining) => {
                match tokio::time::timeout(remaining, self.permits.acquire()).await {
                    Ok(permit) => permit.ok(),
                    Err(_elapsed) => return Some(self.without_running(deadline_elapsed())),
                }
            }
            None => self.permits.acquire().await.ok(),
        };
        let budget = match request.remaining {
            Some(remaining) => {
                let left = remaining.saturating_sub(queued_at.elapsed());
                if left.is_zero() {
                    return Some(self.without_running(deadline_elapsed()));
                }
                Some(left)
            }
            None => None,
        };

        Some(
            self.spawn_and_wait(&command, &cwd, budget, request.step)
                .await,
        )
    }

    /// Steps 3 to 5 of plan D30: the child, its two pipes, the budget, and how it ended.
    async fn spawn_and_wait(
        &self,
        command: &str,
        cwd: &Path,
        budget: Option<Duration>,
        step: StepId,
    ) -> VerifyReport {
        let started_at = self.clock.now();
        // The process environment is handed to the child unchanged (plan D30): ANA-2 `:493` asks
        // for "the agent's environment minus the secrets" and the walk's `SessionSpec.env` is
        // empty this milestone, so the agent's environment *is* this process's.
        let build = || {
            let mut shell = tokio::process::Command::new(&self.shell);
            shell
                .arg(SHELL.1)
                .arg(command)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            shell
        };

        let mut child = match spawn_supervised(build) {
            Ok(child) => child,
            Err(err) => {
                let reason = cannot_spawn(&self.shell, &err);
                return self.finished(VerifyOutcome::Unavailable, None, reason, started_at);
            }
        };
        tracing::debug!(%step, "verify command started");

        // One buffer for both pipes: D30 merges them, and merging at the chunk is the only place
        // the interleaving the command produced still exists. Both are drained concurrently so
        // neither can fill and stall the child, exactly as `Cli::run` does.
        let captured = Arc::new(Mutex::new(TailBuffer::new(CAPTURE_TAIL)));
        let stdout = child.stdout().take();
        let stderr = child.stderr().take();
        let run = async {
            tokio::join!(drain(stdout, &captured), drain(stderr, &captured), async {
                child.wait().await
            })
            .2
        };

        let ended = match budget {
            Some(budget) => tokio::time::timeout(budget, run).await,
            None => Ok(run.await),
        };

        match ended {
            Ok(Ok(status)) => {
                let output = take_output(&captured);
                match status.code() {
                    Some(0) => self.finished(VerifyOutcome::Pass, Some(0), output, started_at),
                    Some(code) => {
                        self.finished(VerifyOutcome::Fail, Some(code), output, started_at)
                    }
                    // A signal is not an exit code, so there is nothing to compare against zero
                    // and the run is one that could not be completed (`docs/ANA-2.md:515`).
                    None => self.finished(
                        VerifyOutcome::Unavailable,
                        None,
                        with_output(killed_by_signal(), output),
                        started_at,
                    ),
                }
            }
            Ok(Err(err)) => {
                let output = take_output(&captured);
                self.finished(
                    VerifyOutcome::Unavailable,
                    None,
                    with_output(cannot_wait(&err), output),
                    started_at,
                )
            }
            Err(_elapsed) => {
                // The group on Unix, the job object on Windows: a `cargo test` that spawned test
                // binaries must not outlive the verify that started it.
                let _ = Box::into_pin(child.kill()).await;
                let output = take_output(&captured);
                self.finished(
                    VerifyOutcome::Unavailable,
                    None,
                    with_output(deadline_elapsed(), output),
                    started_at,
                )
            }
        }
    }
}

impl Verifier for ShellVerifier {
    fn run<'a>(&'a self, request: VerifyRequest) -> VerifierFuture<'a, Option<VerifyReport>> {
        Box::pin(self.execute(request))
    }
}

/// An `unavailable` reason with whatever the command managed to print before it ended.
///
/// The reason comes first: it is the sentence that explains the row, and the tail below it is
/// context. Keeping the tail is what makes a timed-out `cargo test` readable at all — the last
/// line a killed build wrote is rarely the informative one, so the whole tail is kept rather than
/// a line of it.
fn with_output(reason: String, output: String) -> String {
    if output.is_empty() {
        reason
    } else {
        format!("{reason}\n{output}")
    }
}

/// Everything the cap kept, leaving the buffer empty.
///
/// Lossily decoded: a command's output is bytes and `command_run.output` is `TEXT`.
fn take_output(captured: &Mutex<TailBuffer>) -> String {
    let mut guard = captured
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::replace(&mut *guard, TailBuffer::new(CAPTURE_TAIL)).into_string()
}

/// Drains `reader` into the shared tail, stopping at end of stream or at the first pipe error.
///
/// The lock is taken per chunk and never held across an `.await`, so the two drains interleave in
/// the order the bytes arrived.
async fn drain(reader: Option<impl tokio::io::AsyncRead + Unpin>, captured: &Mutex<TailBuffer>) {
    let Some(mut reader) = reader else {
        return;
    };
    let mut chunk = vec![0_u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => captured
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(&chunk[..read]),
        }
    }
}

/// Builds the wrapped command and spawns it, mirroring `isolate/git.rs`'s `spawn_supervised` and
/// `crates/htui-agent/src/launch.rs:1106-1164` verb for verb.
///
/// Unix: a process group, so a kill reaches every process the verify command started. Windows:
/// `CREATE_NO_WINDOW` — `isolate/git.rs`'s constant, shared (blueprint F-I) — and a job object,
/// with the same refused-job-object downgrade `launch.rs` takes.
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
        wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(
            crate::isolate::git::CREATE_NO_WINDOW,
        )));
        wrapped.wrap(JobObject);
        match wrapped.spawn() {
            Ok(child) => Ok(child),
            Err(error) => {
                tracing::warn!(%error, "job object assignment refused; spawning the verify without it");
                let mut wrapped = CommandWrap::from(build());
                wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(
                    crate::isolate::git::CREATE_NO_WINDOW,
                )));
                wrapped.spawn()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use htui_core::model::{StepId, VerifyOutcome};
    use htui_core::scrub::MinimalScrubber;

    use super::{ShellVerifier, Verifier, VerifyRequest};
    use crate::isolate::SystemClock;

    /// A verifier with no secrets to mask and the shipped `verify = 1`.
    fn verifier() -> ShellVerifier {
        ShellVerifier::new(
            &BTreeMap::new(),
            Arc::new(MinimalScrubber::new([])),
            Arc::new(SystemClock),
        )
    }

    /// A verifier that masks `secrets`, `verify = 1` as shipped.
    fn verifier_masking(secrets: &[&str]) -> ShellVerifier {
        ShellVerifier::new(
            &BTreeMap::new(),
            Arc::new(MinimalScrubber::new(
                secrets.iter().copied().map(str::to_owned),
            )),
            Arc::new(SystemClock),
        )
    }

    /// A request for `command` in this test's own directory, with no deadline.
    fn request(command: &str, cwd: PathBuf) -> VerifyRequest {
        VerifyRequest {
            command: Some(command.to_owned()),
            cwd: Some(cwd),
            remaining: None,
            step: StepId::new(),
        }
    }

    /// The normal case, and the one that is **not** `unavailable`: a phase with no
    /// `verify_command` produces no report, so both columns stay `NULL` and milestone 2's settle
    /// rule is untouched (`docs/ANA-2.md:438`, `:514`).
    #[tokio::test]
    async fn no_verify_command_yields_no_report() {
        let report = verifier()
            .run(VerifyRequest {
                command: None,
                cwd: Some(PathBuf::from(".")),
                remaining: None,
                step: StepId::new(),
            })
            .await;
        assert_eq!(report, None, "a phase with no verify_command is not a run");
    }

    /// `docs/ANA-2.md:512`: exit 0 is `pass`, and the code that says so is recorded.
    #[cfg(unix)]
    #[tokio::test]
    async fn exit_zero_is_pass_with_code_zero() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier()
            .run(request("true", dir.path().to_path_buf()))
            .await
            .expect("a command was named, so there is a report");
        assert_eq!(report.outcome, VerifyOutcome::Pass);
        assert_eq!(report.exit_code, Some(0));
        assert!(report.started_at <= report.finished_at);
    }

    /// `:513`: a non-zero exit is `fail` with *the* code, which is what settles the step `failed`
    /// one stage later (milestone 2 D2).
    #[cfg(unix)]
    #[tokio::test]
    async fn nonzero_exit_is_fail_with_the_code() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier()
            .run(request("exit 3", dir.path().to_path_buf()))
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Fail);
        assert_eq!(report.exit_code, Some(3));
    }

    /// The command runs **in the primary tree**, not in this process's directory
    /// (`docs/ANA-2.md:491-492`).
    #[cfg(unix)]
    #[tokio::test]
    async fn the_command_runs_in_the_tree_it_was_given() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(dir.path().join("marker"), "here\n").expect("the marker is written");
        let report = verifier()
            .run(request("cat marker", dir.path().to_path_buf()))
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Pass);
        assert!(report.output.contains("here"), "output: {}", report.output);
    }

    /// A command whose binary is missing is **not** `unavailable`: the shell ran, reported it and
    /// exited 127, so the outcome is `fail` with that code and the shell's own sentence is what
    /// the operator reads (the plan's reading, "the shell reports it").
    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_binary_is_the_shells_own_failure() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier()
            .run(request(
                "definitely-not-a-binary-htui",
                dir.path().to_path_buf(),
            ))
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Fail);
        assert_eq!(report.exit_code, Some(127));
        assert!(
            report.output.contains("not found"),
            "the shell's sentence is the output: {}",
            report.output
        );
    }

    /// Plan D30: a scope with no tree for the primary repo has nowhere to run, and that is one of
    /// `unavailable`'s causes — no exit code, the reason in the output.
    #[tokio::test]
    async fn no_primary_tree_is_unavailable() {
        let report = verifier()
            .run(VerifyRequest {
                command: Some("true".to_owned()),
                cwd: None,
                remaining: None,
                step: StepId::new(),
            })
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Unavailable);
        assert_eq!(report.exit_code, None);
        assert_eq!(report.output, "no primary tree");
    }

    /// `:515`'s "the command could not run": no shell, no child, no exit code — `unavailable`,
    /// with the reason naming what could not be spawned.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_verifier_without_a_shell_is_unavailable() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier()
            .with_shell("/nonexistent/htui-no-shell-here")
            .run(request("true", dir.path().to_path_buf()))
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Unavailable);
        assert_eq!(report.exit_code, None);
        assert!(
            report
                .output
                .starts_with("cannot spawn /nonexistent/htui-no-shell-here: "),
            "unexpected reason: {}",
            report.output
        );
    }

    /// `docs/ANA-2.md:515` lists an elapsed deadline among `unavailable`'s causes, and a remainder
    /// of zero has already elapsed: there is nothing left to run the command in, so nothing is
    /// spawned at all. The step still settles `failed` one stage later, on the deadline itself.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_zero_remainder_never_spawns() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let marker = dir.path().join("spawned");
        let report = verifier()
            .run(VerifyRequest {
                command: Some(format!("touch {}", marker.display())),
                cwd: Some(dir.path().to_path_buf()),
                remaining: Some(Duration::ZERO),
                step: StepId::new(),
            })
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Unavailable);
        assert_eq!(report.exit_code, None);
        assert_eq!(report.output, "deadline elapsed");
        assert!(!marker.exists(), "an elapsed deadline spawns nothing");
    }

    /// Plan D30's timeout is the step deadline's remainder, and on expiry the **group** is killed:
    /// a `cargo test` that spawned test binaries must not outlive the verify that started it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timeout_is_unavailable_and_kills_the_group() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let pidfile = dir.path().join("pid");
        let report = verifier()
            .run(VerifyRequest {
                command: Some(format!("sleep 30 & echo $! > {}; wait", pidfile.display())),
                cwd: Some(dir.path().to_path_buf()),
                remaining: Some(Duration::from_millis(500)),
                step: StepId::new(),
            })
            .await
            .expect("a report");
        assert_eq!(report.outcome, VerifyOutcome::Unavailable);
        assert_eq!(report.exit_code, None);
        assert!(
            report.output.starts_with("deadline elapsed"),
            "unexpected reason: {}",
            report.output
        );

        // The grandchild is the one the group kill has to reach: the shell would have died with
        // its own kill either way, and `sleep 30` is what a runaway build looks like.
        let pid = std::fs::read_to_string(&pidfile).expect("the shell recorded its child's pid");
        let pid: u32 = pid.trim().parse().expect("a pid");
        #[cfg(target_os = "linux")]
        {
            let alive = PathBuf::from(format!("/proc/{pid}"));
            for _ in 0..40 {
                if !alive.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(!alive.exists(), "the group survived the timeout: {pid}");
        }
    }

    /// Plan D30 again: the wait for a class permit counts against the remainder, so a step whose
    /// deadline expires while it is queued behind another `cargo test` never starts one of its own.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_queue_wait_counts_against_the_remainder() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let verifier = verifier();
        let marker = dir.path().join("second");

        let holding = verifier.run(request("sleep 2", dir.path().to_path_buf()));
        let queued = async {
            // Long enough for the first request to be holding the one permit.
            tokio::time::sleep(Duration::from_millis(200)).await;
            verifier
                .run(VerifyRequest {
                    command: Some(format!("touch {}", marker.display())),
                    cwd: Some(dir.path().to_path_buf()),
                    remaining: Some(Duration::from_millis(300)),
                    step: StepId::new(),
                })
                .await
        };
        let (_first, second) = tokio::join!(holding, queued);

        let second = second.expect("a report");
        assert_eq!(second.outcome, VerifyOutcome::Unavailable);
        assert_eq!(second.output, "deadline elapsed");
        assert!(!marker.exists(), "the queued command never ran");
    }

    /// `docs/ANA-2.md:502-506`: the class limit is the whole point of running verify through the
    /// queue, so two requests of a one-permit class do not overlap.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_verify_semaphore_admits_one() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let verifier = verifier();
        let (first, second) = tokio::join!(
            verifier.run(request("sleep 0.3", dir.path().to_path_buf())),
            verifier.run(request("sleep 0.3", dir.path().to_path_buf()))
        );
        let first = first.expect("a report");
        let second = second.expect("a report");
        assert_eq!(first.outcome, VerifyOutcome::Pass);
        assert_eq!(second.outcome, VerifyOutcome::Pass);
        assert!(
            first.finished_at <= second.started_at || second.finished_at <= first.started_at,
            "the two runs overlapped: {first:?} and {second:?}"
        );
    }

    /// Plan D30's 64 KiB is a **tail** cap — the end of a failing build is the part worth
    /// keeping — and what survives it is masked before anyone can persist it (`R-SEC-3`).
    #[cfg(unix)]
    #[tokio::test]
    async fn output_is_tail_capped_and_masked() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier_masking(&["s3cret"])
            .run(request(
                "echo START-MARKER; yes filler-line | head -c 100000; echo; echo s3cret",
                dir.path().to_path_buf(),
            ))
            .await
            .expect("a report");

        assert_eq!(report.outcome, VerifyOutcome::Pass);
        assert!(
            !report.output.contains("START-MARKER"),
            "the head was kept, so the cap is not a tail cap"
        );
        assert!(
            !report.output.contains("s3cret"),
            "an unmasked secret reached the report"
        );
        assert!(
            report.output.contains("[REDACTED]"),
            "the secret was not masked, it was lost"
        );
    }

    /// `R-SEC-3` is fail-closed: an output the scrubber refuses is not persistable at all, so the
    /// text is dropped. The outcome and the exit code are facts about the run and survive.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_scrub_refusal_withholds_the_output_and_keeps_the_outcome() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let report = verifier()
            .run(request(
                "echo token=sk-ant-notarealkey; exit 2",
                dir.path().to_path_buf(),
            ))
            .await
            .expect("a report");

        assert_eq!(report.outcome, VerifyOutcome::Fail);
        assert_eq!(report.exit_code, Some(2));
        assert!(
            report.output.starts_with("<scrub refused: ")
                && report.output.ends_with(" bytes withheld>"),
            "unexpected output: {}",
            report.output
        );
    }
}
