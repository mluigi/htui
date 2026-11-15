//! The target channel: how htui runs one command on a target box (MOD-132 M1, plan P1-P3).
//!
//! A target is the box a run's repository clone lives on. htui reaches it through a
//! [`TargetChannel`], which runs one argv there and reports how it exited. The seam is argv only:
//! no working directory and no environment travel with a command. Every `git` command names its
//! repository with `-C <dir>`, and configuration travels as `-c key=value` argv, so the same
//! [`ExecSpec`] fits `docker exec -i <ctr> -- argv` and `ssh -T host -- argv` (the real channels
//! are MOD-131's). Stdin is nothing or a file on this box read from an offset; stdout is a tail or
//! a fresh private file with a byte cap; stderr is always a tail; a deadline kills the command.
//!
//! The only implementation in this item is [`StandInChannel`] (feature `test-support`), which
//! runs the argv on this box under a throwaway `home`.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

/// The future a [`TargetChannel`] returns: boxed, so the trait stays dyn-compatible, as
/// `DriverFuture` and `htui_orch`'s `IsolatorFuture` are.
pub type ChannelFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ChannelError>> + Send + 'a>>;

/// Set to `1` in every command the stand-in runs (plan P4).
///
/// Hostile fixtures create their sentinel only when it is **unset**: a hook that fires on the
/// target is the target's own business, one that fires on this box is a failure.
pub const STAND_IN_MARKER: &str = "HTUI_STAND_IN_TARGET";

/// Bytes kept of a tail-captured stream: the **last** 64 KiB, since the line that says why a
/// command failed is the last one it wrote.
pub const OUTPUT_TAIL: usize = 64 * 1024;

/// Where a command's stdin comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdinFrom {
    /// Nothing: the command reads end of file at once.
    Null,
    /// A file on this box, read from `offset` (a bundle's pack starts after its header).
    File {
        /// The file to read.
        path: PathBuf,
        /// The first byte handed to the command.
        offset: u64,
    },
}

/// Where a command's stdout goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdoutTo {
    /// The last [`OUTPUT_TAIL`] bytes, lossily decoded into [`ExecOutput::stdout`].
    Tail,
    /// A new file on this box: created with `create_new` (`0600` on Unix), holding at most `cap`
    /// bytes. Past the cap the command is killed, the file removed and
    /// [`ChannelError::OverCap`] returned. On any other error the file is removed too; after a
    /// non-zero exit it is kept, for the caller to judge and remove.
    File {
        /// The file to create; an existing one is refused before anything runs.
        path: PathBuf,
        /// The most bytes the file may hold.
        cap: u64,
    },
}

/// One command for a [`TargetChannel`] to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSpec {
    /// A short name for the command, used in errors (`"bundle create"`).
    pub verb: &'static str,
    /// The program and its arguments; `argv[0]` is looked up on the target's `PATH`.
    pub argv: Vec<String>,
    /// Where stdin comes from.
    pub stdin: StdinFrom,
    /// Where stdout goes.
    pub stdout: StdoutTo,
    /// How long the command may run before it is killed ([`ChannelError::TimedOut`]).
    pub timeout: Duration,
}

/// How a command exited. A non-zero exit is an answer, not a [`ChannelError`]: each caller
/// classifies its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    /// The exit code; `None` when a signal ended the command.
    pub code: Option<i32>,
    /// The stdout tail under [`StdoutTo::Tail`]; empty under [`StdoutTo::File`].
    pub stdout: String,
    /// The stderr tail, the last [`OUTPUT_TAIL`] bytes, lossily decoded.
    pub stderr: String,
    /// Every byte the command wrote to stdout, whichever way it went.
    pub stdout_bytes: u64,
}

impl ExecOutput {
    /// Whether the command exited 0.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// A command the channel could not run to an exit.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The target cannot be reached at all; the text names it.
    #[error("{0}")]
    Unreachable(String),
    /// The command could not be started (no such program, an empty argv).
    #[error("{0}")]
    Spawn(String),
    /// The deadline passed; the command and everything it started were killed.
    #[error("{verb} timed out after {} s on the target; it was killed", after.as_secs())]
    TimedOut {
        /// The command's [`ExecSpec::verb`].
        verb: &'static str,
        /// Its [`ExecSpec::timeout`].
        after: Duration,
    },
    /// Stdout passed its [`StdoutTo::File`] cap; the command was killed and the file removed.
    #[error("{verb} wrote more than {cap} bytes; it was killed and its output removed")]
    OverCap {
        /// The command's [`ExecSpec::verb`].
        verb: &'static str,
        /// The cap it passed.
        cap: u64,
    },
    /// Reading stdin, writing stdout's file or waiting for the command failed.
    #[error("target channel I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// How htui runs a command on a target (plan P1, P2).
pub trait TargetChannel: Send + Sync + fmt::Debug {
    /// The target in words, for sentences: `"stand-in at /tmp/x"`.
    fn describe(&self) -> String;

    /// Runs `spec` on the target and reports how it exited.
    fn exec<'a>(&'a self, spec: &'a ExecSpec) -> ChannelFuture<'a, ExecOutput>;
}

#[cfg(feature = "test-support")]
pub use stand_in::{Canned, StandInChannel};

/// The stand-in target (plan P3): every command runs on this box, under `home`.
#[cfg(feature = "test-support")]
mod stand_in {
    use std::collections::VecDeque;
    use std::io::{Seek as _, SeekFrom, Write as _};
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::{
        ChannelError, ChannelFuture, ExecOutput, ExecSpec, OUTPUT_TAIL, STAND_IN_MARKER, StdinFrom,
        StdoutTo, TargetChannel,
    };

    /// How long the pipes are still read after the command itself exited: a daemonised
    /// grandchild that inherited them must not turn an exit into a timeout.
    const PIPE_GRACE: Duration = Duration::from_secs(1);

    /// How long a killed command is waited for before it is abandoned.
    const KILL_GRACE: Duration = Duration::from_secs(5);

    /// The variables that redirect `git`'s repository discovery, removed from every command.
    ///
    /// A copy of `htui_orch::isolate::git::SCRUBBED_ENV`, since `htui-agent` cannot depend on
    /// `htui-orch`; the two lists name the same ten variables.
    const SCRUBBED_ENV: [&str; 10] = [
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

    /// A canned answer: the stdout bytes and exit code an argv gets instead of running.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Canned {
        /// What the command "wrote" to stdout, through the spec's [`StdoutTo`], cap included.
        pub stdout: Vec<u8>,
        /// Its exit code.
        pub code: i32,
    }

    type Matcher = dyn Fn(&[String]) -> Option<Canned> + Send + Sync;

    /// A [`TargetChannel`] that runs every argv on this box, with `current_dir` and `HOME` both
    /// `home` (so a relative target path lands under it), [`STAND_IN_MARKER`] set, and
    /// `htui_core::secret::HTUI_OWN_ENV` and the `git` location variables removed.
    ///
    /// Test knobs: [`unreachable`](Self::unreachable) and [`canned`](Self::canned).
    #[derive(Clone)]
    pub struct StandInChannel {
        home: PathBuf,
        unreachable: bool,
        canned: Vec<Arc<Matcher>>,
    }

    impl std::fmt::Debug for StandInChannel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StandInChannel")
                .field("home", &self.home)
                .field("unreachable", &self.unreachable)
                .field("canned", &self.canned.len())
                .finish()
        }
    }

    impl StandInChannel {
        /// A reachable stand-in rooted at `home`, with no canned answers.
        #[must_use]
        pub fn new(home: impl Into<PathBuf>) -> Self {
            Self {
                home: home.into(),
                unreachable: false,
                canned: Vec::new(),
            }
        }

        /// The directory every command runs in, and its `HOME`.
        #[must_use]
        pub fn home(&self) -> &Path {
            &self.home
        }

        /// Every exec fails with [`ChannelError::Unreachable`], and nothing runs.
        #[must_use]
        pub fn unreachable(mut self) -> Self {
            self.unreachable = true;
            self
        }

        /// An argv `matcher` accepts is answered with its [`Canned`] instead of running. Matchers
        /// are asked in the order they were added; the first answer wins.
        #[must_use]
        pub fn canned(
            mut self,
            matcher: impl Fn(&[String]) -> Option<Canned> + Send + Sync + 'static,
        ) -> Self {
            self.canned.push(Arc::new(matcher));
            self
        }

        async fn run(&self, spec: &ExecSpec) -> Result<ExecOutput, ChannelError> {
            if self.unreachable {
                return Err(ChannelError::Unreachable(format!(
                    "{} is unreachable: {} was not run",
                    self.describe(),
                    spec.verb
                )));
            }
            if let Some(answer) = self.canned.iter().find_map(|matcher| matcher(&spec.argv)) {
                return answer_canned(spec, answer);
            }
            let Some((program, args)) = spec.argv.split_first() else {
                return Err(ChannelError::Spawn(format!(
                    "{}: an empty argv names no program",
                    spec.verb
                )));
            };
            let stdin = open_stdin(&spec.stdin)?;
            let sink = Sink::open(&spec.stdout)?;
            let build = || -> std::io::Result<tokio::process::Command> {
                let mut command = tokio::process::Command::new(program);
                command.args(args).current_dir(&self.home);
                for key in SCRUBBED_ENV {
                    command.env_remove(key);
                }
                for key in htui_core::secret::HTUI_OWN_ENV {
                    command.env_remove(key);
                }
                command
                    .env("HOME", &self.home)
                    .env(STAND_IN_MARKER, "1")
                    .stdin(if stdin.is_some() {
                        Stdio::piped()
                    } else {
                        Stdio::null()
                    })
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                Ok(command)
            };
            let mut child = match spawn_supervised(build) {
                Ok(child) => child,
                Err(err) => {
                    sink.discard();
                    return Err(ChannelError::Spawn(format!(
                        "{}: cannot run {program} on {}: {err}",
                        spec.verb,
                        self.describe()
                    )));
                }
            };
            supervise(spec, child.as_mut(), stdin, sink).await
        }
    }

    impl TargetChannel for StandInChannel {
        fn describe(&self) -> String {
            format!("stand-in at {}", self.home.display())
        }

        fn exec<'a>(&'a self, spec: &'a ExecSpec) -> ChannelFuture<'a, ExecOutput> {
            Box::pin(self.run(spec))
        }
    }

    /// Opens a [`StdinFrom::File`] at its offset, before anything runs; `None` for
    /// [`StdinFrom::Null`].
    ///
    /// The file is fed through a pipe, not handed over as the child's descriptor: `git` sizes a
    /// regular-file stdin by `fstat` and ignores the offset (`hash-object --stdin` answers
    /// "short read"), and a remote channel's stdin is a stream anyway.
    fn open_stdin(stdin: &StdinFrom) -> Result<Option<tokio::fs::File>, ChannelError> {
        match stdin {
            StdinFrom::Null => Ok(None),
            StdinFrom::File { path, offset } => {
                let mut file = std::fs::File::open(path)?;
                file.seek(SeekFrom::Start(*offset))?;
                Ok(Some(tokio::fs::File::from_std(file)))
            }
        }
    }

    /// Copies `from` into the child's stdin and closes it. A child that exits, or closes its
    /// stdin, without reading everything is its own answer, so a write error just stops the
    /// copy.
    async fn feed(from: Option<tokio::fs::File>, to: Option<impl tokio::io::AsyncWrite + Unpin>) {
        if let (Some(mut from), Some(mut to)) = (from, to) {
            let _ = tokio::io::copy(&mut from, &mut to).await;
            let _ = to.shutdown().await;
        }
    }

    /// A canned answer's bytes through the spec's [`StdoutTo`], cap included.
    fn answer_canned(spec: &ExecSpec, answer: Canned) -> Result<ExecOutput, ChannelError> {
        let mut sink = Sink::open(&spec.stdout)?;
        if let Err(stop) = sink.push(&answer.stdout) {
            sink.discard();
            return Err(stop.into_error(spec.verb));
        }
        let (stdout, stdout_bytes) = sink.finish(spec.verb)?;
        Ok(ExecOutput {
            code: Some(answer.code),
            stdout,
            stderr: String::new(),
            stdout_bytes,
        })
    }

    /// Why reading stdout stopped before end of file.
    #[derive(Debug)]
    enum Stop {
        OverCap(u64),
        Io(std::io::Error),
    }

    impl Stop {
        fn into_error(self, verb: &'static str) -> ChannelError {
            match self {
                Self::OverCap(cap) => ChannelError::OverCap { verb, cap },
                Self::Io(err) => ChannelError::Io(err),
            }
        }
    }

    /// Where stdout goes while the command runs.
    #[derive(Debug)]
    enum Sink {
        Tail {
            tail: TailBuffer,
            seen: u64,
        },
        File {
            file: std::fs::File,
            path: PathBuf,
            cap: u64,
            written: u64,
        },
    }

    impl Sink {
        /// The sink [`StdoutTo`] asks for; a [`StdoutTo::File`] is created here, `create_new`
        /// and `0600` on Unix, so an existing file is refused before anything runs.
        fn open(stdout: &StdoutTo) -> Result<Self, ChannelError> {
            match stdout {
                StdoutTo::Tail => Ok(Self::Tail {
                    tail: TailBuffer::new(OUTPUT_TAIL),
                    seen: 0,
                }),
                StdoutTo::File { path, cap } => {
                    let mut options = std::fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt as _;
                        options.mode(0o600);
                    }
                    Ok(Self::File {
                        file: options.open(path)?,
                        path: path.clone(),
                        cap: *cap,
                        written: 0,
                    })
                }
            }
        }

        /// Takes one chunk; a file refuses the chunk that would pass its cap.
        fn push(&mut self, chunk: &[u8]) -> Result<(), Stop> {
            let len = chunk.len() as u64;
            match self {
                Self::Tail { tail, seen } => {
                    tail.push(chunk);
                    *seen += len;
                    Ok(())
                }
                Self::File {
                    file, cap, written, ..
                } => {
                    if *written + len > *cap {
                        return Err(Stop::OverCap(*cap));
                    }
                    file.write_all(chunk).map_err(Stop::Io)?;
                    *written += len;
                    Ok(())
                }
            }
        }

        /// The stdout text (empty for a file) and the byte count; a file is flushed and kept.
        fn finish(self, verb: &'static str) -> Result<(String, u64), ChannelError> {
            match self {
                Self::Tail { tail, seen } => Ok((tail.into_string(), seen)),
                Self::File {
                    mut file,
                    path,
                    written,
                    ..
                } => match file.flush() {
                    Ok(()) => Ok((String::new(), written)),
                    Err(err) => {
                        drop(file);
                        let _ = std::fs::remove_file(&path);
                        Err(Stop::Io(err).into_error(verb))
                    }
                },
            }
        }

        /// Drops the sink, removing a file it created.
        fn discard(self) {
            if let Self::File { file, path, .. } = self {
                drop(file);
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// Waits for `child` under the spec's deadline while both pipes drain, and turns the end
    /// into an answer. Every error path kills the process group and discards the sink.
    async fn supervise(
        spec: &ExecSpec,
        child: &mut dyn process_wrap::tokio::ChildWrapper,
        stdin_file: Option<tokio::fs::File>,
        sink: Sink,
    ) -> Result<ExecOutput, ChannelError> {
        let stdin = child.stdin().take();
        let stdout = child.stdout().take();
        let stderr = child.stderr().take();
        let sink = Mutex::new(sink);
        let err_tail = Mutex::new(TailBuffer::new(OUTPUT_TAIL));
        // Boxed rather than pinned on the stack so that dropping it drops the pipes with it.
        let mut reading = Box::pin(async {
            tokio::try_join!(
                pump(stdout, &sink),
                async {
                    read_tail(stderr, &err_tail).await;
                    Ok(())
                },
                async {
                    feed(stdin_file, stdin).await;
                    Ok(())
                }
            )
            .map(|((), (), ())| ())
        });

        let waited = tokio::time::timeout(spec.timeout, async {
            tokio::select! {
                status = child.wait() => Waited::Exited(status, false),
                read = &mut reading => match read {
                    Ok(()) => Waited::Exited(child.wait().await, true),
                    Err(stop) => Waited::Stopped(stop),
                },
            }
        })
        .await;

        let ended = match waited {
            Ok(Waited::Exited(status, drained)) => {
                let late = if drained {
                    Ok(())
                } else {
                    tokio::time::timeout(PIPE_GRACE, &mut reading)
                        .await
                        .unwrap_or(Ok(()))
                };
                late.map_err(|stop| stop.into_error(spec.verb))
                    .and_then(|()| status.map_err(ChannelError::Io))
            }
            Ok(Waited::Stopped(stop)) => {
                kill_within_grace(child).await;
                Err(stop.into_error(spec.verb))
            }
            Err(_elapsed) => {
                kill_within_grace(child).await;
                Err(ChannelError::TimedOut {
                    verb: spec.verb,
                    after: spec.timeout,
                })
            }
        };
        drop(reading);
        let sink = sink.into_inner().unwrap_or_else(PoisonError::into_inner);
        let status = match ended {
            Ok(status) => status,
            Err(err) => {
                sink.discard();
                return Err(err);
            }
        };
        let (stdout, stdout_bytes) = sink.finish(spec.verb)?;
        Ok(ExecOutput {
            code: status.code(),
            stdout,
            stderr: err_tail
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .into_string(),
            stdout_bytes,
        })
    }

    /// How the wait under the deadline ended.
    enum Waited {
        /// The command exited; `true` when both pipes had already reached end of file.
        Exited(std::io::Result<std::process::ExitStatus>, bool),
        /// Stdout stopped first: the cap, or a write that failed.
        Stopped(Stop),
    }

    /// Drains stdout into the sink, stopping at end of file or a pipe error, or failing at the
    /// cap. The `std` lock is held for one push and never across an `.await`.
    async fn pump(
        reader: Option<impl tokio::io::AsyncRead + Unpin>,
        sink: &Mutex<Sink>,
    ) -> Result<(), Stop> {
        let Some(mut reader) = reader else {
            return Ok(());
        };
        let mut chunk = vec![0_u8; 8 * 1024];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) | Err(_) => return Ok(()),
                Ok(read) => sink
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(&chunk[..read])?,
            }
        }
    }

    /// Drains stderr into its tail, stopping at end of file or the first pipe error.
    async fn read_tail(
        reader: Option<impl tokio::io::AsyncRead + Unpin>,
        tail: &Mutex<TailBuffer>,
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
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(&chunk[..read]),
            }
        }
    }

    /// Kills the child's whole group and waits for it, for at most [`KILL_GRACE`].
    async fn kill_within_grace(child: &mut dyn process_wrap::tokio::ChildWrapper) {
        if tokio::time::timeout(KILL_GRACE, Box::into_pin(child.kill()))
            .await
            .is_err()
        {
            tracing::warn!(
                "a killed stand-in command was not reaped within the grace; abandoning it"
            );
        }
    }

    /// The last `cap` bytes pushed through it.
    #[derive(Debug)]
    struct TailBuffer {
        bytes: VecDeque<u8>,
        cap: usize,
    }

    impl TailBuffer {
        fn new(cap: usize) -> Self {
            Self {
                bytes: VecDeque::new(),
                cap,
            }
        }

        fn push(&mut self, chunk: &[u8]) {
            let keep = chunk.len().min(self.cap);
            self.bytes.extend(&chunk[chunk.len() - keep..]);
            let excess = self.bytes.len().saturating_sub(self.cap);
            self.bytes.drain(..excess);
        }

        fn into_string(self) -> String {
            let mut bytes = Vec::with_capacity(self.bytes.len());
            let (front, back) = self.bytes.as_slices();
            bytes.extend_from_slice(front);
            bytes.extend_from_slice(back);
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    /// Builds the command and spawns it supervised, mirroring `htui_orch`'s
    /// `isolate::git::spawn_supervised`.
    ///
    /// Unix: a process group, so a kill reaches whatever the command started. Windows:
    /// `CREATE_NO_WINDOW` and a job object, with the downgrade to no job object when it is
    /// refused (a supervision mode, not a dead command).
    fn spawn_supervised(
        build: impl Fn() -> std::io::Result<tokio::process::Command>,
    ) -> std::io::Result<Box<dyn process_wrap::tokio::ChildWrapper>> {
        use process_wrap::tokio::CommandWrap;

        #[cfg(unix)]
        {
            let mut wrapped = CommandWrap::from(build()?);
            wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
            wrapped.spawn()
        }

        #[cfg(windows)]
        {
            use process_wrap::tokio::{CreationFlags, JobObject};
            use windows::Win32::System::Threading::PROCESS_CREATION_FLAGS;

            let mut wrapped = CommandWrap::from(build()?);
            wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(
                crate::launch::CREATE_NO_WINDOW,
            )));
            wrapped.wrap(JobObject);
            match wrapped.spawn() {
                Ok(child) => Ok(child),
                Err(error) => {
                    tracing::warn!(%error, "job object assignment refused; spawning the stand-in command without it");
                    let mut wrapped = CommandWrap::from(build()?);
                    wrapped.wrap(CreationFlags(PROCESS_CREATION_FLAGS(
                        crate::launch::CREATE_NO_WINDOW,
                    )));
                    wrapped.spawn()
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::TailBuffer;

        #[test]
        fn the_tail_keeps_the_last_cap_bytes() {
            let mut tail = TailBuffer::new(4);
            tail.push(b"ab");
            tail.push(b"cdef");
            assert_eq!(tail.into_string(), "cdef");
            let mut tail = TailBuffer::new(4);
            tail.push(b"0123456789");
            tail.push(b"x");
            assert_eq!(tail.into_string(), "789x");
        }
    }
}
