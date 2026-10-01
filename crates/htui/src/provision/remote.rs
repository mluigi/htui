//! The ssh seam (MOD-45 D298). [`SshRemote`] is the only `ssh` spawner in htui (PRD "SSH not
//! reused"). The payload is written from a future joined with the two output drains, never before
//! them: a 100 MiB write into a full pipe whose reader is not draining would deadlock (V-6).

use std::borrow::Cow;
use std::ffi::OsString;
use std::process::Stdio;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

/// Each captured stream keeps its first 64 KiB; the rest is drained and dropped.
pub const OUTPUT_CAP: usize = 64 * 1024;

/// How long a dropped session's child has between SIGTERM and SIGKILL (verify round 1).
pub const TERM_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// What one remote session left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteOutput {
    /// The exit code; `None` when a signal ended it (ssh's own failures are 255).
    pub code: Option<i32>,
    /// At most [`OUTPUT_CAP`] bytes.
    pub stdout: Vec<u8>,
    /// At most [`OUTPUT_CAP`] bytes.
    pub stderr: Vec<u8>,
}

impl RemoteOutput {
    /// Lossy UTF-8 of `stdout`.
    #[must_use]
    pub fn stdout_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    /// The last non-empty stderr line, trimmed and [`sanitise`]d; `""` when there is none.
    #[must_use]
    pub fn last_stderr_line(&self) -> String {
        sanitise(
            String::from_utf8_lossy(&self.stderr)
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .unwrap_or_default(),
        )
    }

    /// The last `n` non-empty stderr lines, joined by `\n` and [`sanitise`]d.
    #[must_use]
    pub fn stderr_tail(&self, n: usize) -> String {
        let text = String::from_utf8_lossy(&self.stderr);
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .collect();
        sanitise(&lines[lines.len().saturating_sub(n)..].join("\n"))
    }
}

/// Remote text made safe to print (review finding 9): every control character but `\n` and `\t`
/// becomes its [`char::escape_default`] form, so an ESC or OSC sequence from the remote host is
/// shown as text instead of driving this terminal. Idempotent.
#[must_use]
pub fn sanitise(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            shown.extend(c.escape_default());
        } else {
            shown.push(c);
        }
    }
    shown
}

/// One remote session: run `command` under the remote login shell, feed it `stdin`, then close it.
#[allow(
    async_fn_in_trait,
    reason = "no dyn Remote is formed: run_with is generic (E-6)"
)]
pub trait Remote {
    /// Runs one session.
    ///
    /// # Errors
    ///
    /// The spawn's, or a write's other than a broken pipe (a script that exits without reading).
    async fn run(&self, command: &str, stdin: &[u8]) -> std::io::Result<RemoteOutput>;
}

/// The user's own `ssh` (PRD D4): `~/.ssh/config`, `ProxyJump` and the agent apply; its prompts go
/// to `/dev/tty`.
#[derive(Debug, Clone)]
pub struct SshRemote {
    destination: String,
}

impl SshRemote {
    /// `ssh` to `destination`, as the user typed it.
    #[must_use]
    pub fn new(destination: &str) -> Self {
        Self {
            destination: destination.to_owned(),
        }
    }
}

impl Remote for SshRemote {
    async fn run(&self, command: &str, stdin: &[u8]) -> std::io::Result<RemoteOutput> {
        let mut ssh = tokio::process::Command::new("ssh");
        ssh.args(ssh_argv(&self.destination, command));
        run_piped(ssh, stdin).await
    }
}

/// `-T -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -- <destination>
/// <command>` (V-7: `--` stops ssh from reading options after the destination). The keepalives end
/// a session whose peer went silent after about a minute (review finding 2). Pure.
#[must_use]
pub fn ssh_argv(destination: &str, command: &str) -> Vec<OsString> {
    [
        "-T",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
        "--",
        destination,
        command,
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

/// Spawns `command` with all three streams piped and `kill_on_drop(true)`, then joins
/// (`tokio::join!`, one task) the stdin write and close with both capped drains, then waits. A
/// `BrokenPipe` on the write is not an error. `pub` so the T4 `LocalShell` exercises the same code.
///
/// Dropped before the child ended (a session past its bound), the child gets SIGTERM, up to
/// [`TERM_GRACE`] to exit, and only then `kill_on_drop`'s SIGKILL: `ssh` restores the terminal on
/// SIGTERM when it was showing a password or passphrase prompt (verify round 1).
///
/// # Errors
///
/// The spawn's, the wait's, or a write's other than `BrokenPipe`.
pub async fn run_piped(
    mut command: tokio::process::Command,
    stdin: &[u8],
) -> std::io::Result<RemoteOutput> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = Terminating(command.spawn()?);
    let input = child.0.stdin.take();
    let out = child.0.stdout.take();
    let err = child.0.stderr.take();
    let write = async move {
        let Some(mut input) = input else {
            return Ok(());
        };
        let written = match input.write_all(stdin).await {
            Ok(()) => input.shutdown().await,
            Err(err) => Err(err),
        };
        drop(input);
        match written {
            Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => Err(err),
            _ => Ok(()),
        }
    };
    let (written, stdout, stderr) = tokio::join!(write, drain(out), drain(err));
    written?;
    let status = child.0.wait().await?;
    Ok(RemoteOutput {
        code: status.code(),
        stdout: stdout?,
        stderr: stderr?,
    })
}

/// A child that, dropped while it still runs, is asked to stop before `kill_on_drop` kills it.
struct Terminating(tokio::process::Child);

impl Drop for Terminating {
    fn drop(&mut self) {
        // `id()` is `None` once the child was waited to completion: nothing to signal, and no
        // recycled pid to hit.
        let Some(pid) = self.0.id() else {
            return;
        };
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        // No `unsafe` here (the workspace forbids it), so the shell's own `kill` sends SIGTERM.
        let asked = std::process::Command::new("sh")
            .args([
                "-c",
                "kill -TERM \"$1\"",
                "htui-provision",
                &pid.to_string(),
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !asked {
            return;
        }
        // A bounded, blocking wait: this runs only on the path that ends the run with an error.
        let until = std::time::Instant::now() + TERM_GRACE;
        while std::time::Instant::now() < until {
            if !matches!(self.0.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // `kill_on_drop` sends SIGKILL as the child is dropped.
    }
}

/// Reads `stream` to its end, keeping the first [`OUTPUT_CAP`] bytes.
async fn drain(stream: Option<impl AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
    let mut kept = Vec::new();
    let Some(mut stream) = stream else {
        return Ok(kept);
    };
    let mut chunk = vec![0_u8; 16 * 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(kept);
        }
        let room = OUTPUT_CAP - kept.len();
        kept.extend_from_slice(&chunk[..n.min(room)]);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn ssh_argv_puts_the_separator_before_the_destination() {
        let argv = ssh_argv("u@box1", "sh -c true");
        assert_eq!(
            argv,
            [
                "-T",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=4",
                "--",
                "u@box1",
                "sh -c true"
            ]
            .map(OsString::from)
            .to_vec()
        );
    }

    // Review finding 8: the sentinel check over `ssh_argv` runs a whole `run_with` now, in
    // `provision::tests::ssh_argv_of_every_session_carries_no_sentinel`.

    /// Review finding 9.
    #[test]
    fn sanitise_escapes_every_control_character_but_newline_and_tab() {
        let remote = "ok\u{1b}]0;owned\u{7} \u{1b}[31mred\u{1b}[0m\r\n\tdone\u{7f}\u{9b}";
        let shown = sanitise(remote);
        assert_eq!(
            shown,
            "ok\\u{1b}]0;owned\\u{7} \\u{1b}[31mred\\u{1b}[0m\\r\n\tdone\\u{7f}\\u{9b}"
        );
        assert_eq!(sanitise(&shown), shown, "idempotent");
        assert_eq!(sanitise("pläin › text"), "pläin › text");

        let output = RemoteOutput {
            code: Some(1),
            stdout: Vec::new(),
            stderr: b"one\n\x1b[2Jtwo\x1b]8;;x\x07\n".to_vec(),
        };
        assert_eq!(output.last_stderr_line(), "\\u{1b}[2Jtwo\\u{1b}]8;;x\\u{7}");
        assert!(!output.stderr_tail(2).contains('\u{1b}'));
    }

    #[test]
    fn last_stderr_line_and_tail_skip_blank_lines() {
        let output = RemoteOutput {
            code: Some(1),
            stdout: Vec::new(),
            stderr: b"one\n\ntwo\nthree  \n\n".to_vec(),
        };
        assert_eq!(output.last_stderr_line(), "three");
        assert_eq!(output.stderr_tail(2), "two\nthree");
        assert_eq!(output.stderr_tail(9), "one\ntwo\nthree");
        assert_eq!(RemoteOutput::default().last_stderr_line(), "");
    }

    fn sh(script: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[tokio::test]
    async fn run_piped_writes_stdin_while_draining_output() {
        let input = vec![b'x'; 4 * 1024 * 1024];
        let output = tokio::time::timeout(Duration::from_secs(10), run_piped(sh("cat"), &input))
            .await
            .expect("a sequential write would deadlock")
            .expect("cat runs");
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout.len(), OUTPUT_CAP);
    }

    /// `sh -c <script> htui-provision <marker>`: the script sees the marker path as `$1`.
    fn sh_with(script: &str, marker: &std::path::Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", script, "htui-provision"]).arg(marker);
        command
    }

    /// Verify round 1: a dropped session first gets SIGTERM, which `ssh` handles by giving the
    /// terminal back (echo on) when it was showing a password prompt; SIGKILL cannot be caught.
    #[tokio::test]
    async fn a_dropped_session_is_terminated_before_it_is_killed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let marker = dir.path().join("term");
        let script =
            "mark=$1\ntrap \"echo term > $mark; exit 0\" TERM\nwhile :; do sleep 0.05; done\n";
        let dropped = tokio::time::timeout(
            Duration::from_millis(300),
            run_piped(sh_with(script, &marker), b""),
        )
        .await;
        assert!(dropped.is_err(), "the loop never ends by itself");
        assert_eq!(
            std::fs::read_to_string(&marker).ok().as_deref(),
            Some("term\n"),
            "the child saw SIGTERM before the drop returned"
        );
    }

    /// Verify round 1: a child that ignores SIGTERM is still killed once the grace has passed.
    #[tokio::test]
    async fn a_dropped_session_that_ignores_sigterm_is_killed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let marker = dir.path().join("pid");
        let script = "trap \"\" TERM\necho $$ > \"$1\"\nwhile :; do sleep 0.05; done\n";
        let started = std::time::Instant::now();
        let dropped = tokio::time::timeout(
            Duration::from_millis(300),
            run_piped(sh_with(script, &marker), b""),
        )
        .await;
        assert!(dropped.is_err(), "the loop never ends by itself");
        assert!(
            started.elapsed() < TERM_GRACE + Duration::from_secs(2),
            "the grace is bounded: {:?}",
            started.elapsed()
        );
        let pid = std::fs::read_to_string(&marker).expect("the pid");
        let stat = std::path::PathBuf::from(format!("/proc/{}/stat", pid.trim()));
        let gone = std::time::Instant::now();
        loop {
            // Absent once reaped, `Z` until then; never running again.
            match std::fs::read_to_string(&stat) {
                Err(_) => break,
                Ok(text) if text.rsplit(") ").next().is_some_and(|s| s.starts_with('Z')) => break,
                Ok(text) => assert!(
                    gone.elapsed() < Duration::from_secs(5),
                    "still running: {text}"
                ),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn run_piped_tolerates_a_child_that_never_reads() {
        let input = vec![b'x'; 4 * 1024 * 1024];
        let output = tokio::time::timeout(Duration::from_secs(10), run_piped(sh("exit 4"), &input))
            .await
            .expect("it ends")
            .expect("a broken pipe is not an error");
        assert_eq!(output.code, Some(4));
    }
}
