//! What [`seed`](super::seed) and [`collect`](super::collect) share (MOD-132 R1b L8): the
//! scratch guard, the target-side argv, and the line a failed target verb is reported with.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use htui_agent::target_channel::ExecOutput;

/// A call's scratch on this box: a file and, for a collect, a directory, both removed on drop, so
/// on every exit: a refusal, a panic, or a future dropped by a cancel or a timeout.
///
/// The directory is held through an [`Arc`] (R1b verifier L1): a blocking step that works in it
/// (the pack migration) takes a clone, so a future dropped mid-step cannot remove the directory
/// under a `spawn_blocking` task that a dropped `JoinHandle` detaches rather than aborts. The
/// directory goes when the last holder does.
pub(crate) struct Quarantine {
    file: PathBuf,
    dir: Option<Arc<ScratchDir>>,
}

/// A scratch directory removed when its last holder drops it.
#[derive(Debug)]
pub(crate) struct ScratchDir(PathBuf);

impl ScratchDir {
    /// The directory.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // NotFound is the normal case after a refusal before the directory was made.
        if let Err(err) = std::fs::remove_dir_all(&self.0)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(dir = %self.0.display(), %err, "a collect quarantine could not be removed");
        }
    }
}

impl Quarantine {
    /// A scratch of one file.
    pub(crate) fn file(file: PathBuf) -> Self {
        Self { file, dir: None }
    }

    /// A scratch of one file and one directory.
    pub(crate) fn with_dir(file: PathBuf, dir: PathBuf) -> Self {
        Self {
            file,
            dir: Some(Arc::new(ScratchDir(dir))),
        }
    }

    /// The file.
    pub(crate) fn path(&self) -> &Path {
        &self.file
    }

    /// The directory, if there is one.
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref().map(ScratchDir::path)
    }

    /// A holder of the directory, for a blocking step that must finish before it is removed.
    pub(crate) fn dir_hold(&self) -> Option<Arc<ScratchDir>> {
        self.dir.clone()
    }
}

impl Drop for Quarantine {
    fn drop(&mut self) {
        // NotFound is the normal case after a refusal before the file was written. The
        // directory is `ScratchDir`'s, removed when its last holder drops.
        let _ = std::fs::remove_file(&self.file);
    }
}

/// `git -C <dir> <args>…` as an argv.
pub(crate) fn git_in(dir: &str, args: &[&str]) -> Vec<String> {
    ["git", "-C", dir]
        .iter()
        .chain(args)
        .map(|arg| (*arg).to_owned())
        .collect()
}

/// `out`'s last non-empty stderr line, trimmed, or its exit status when it wrote none. Raw: the
/// sentence that shows it escapes it ([`escape_controls`]).
pub(crate) fn last_stderr_line(out: &ExecOutput) -> String {
    out.stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(
            || match out.code {
                Some(code) => format!("exit status {code} with no stderr"),
                None => "killed by a signal".to_owned(),
            },
            str::to_owned,
        )
}

/// `text` with every control character (C0, DEL, C1) and the two Unicode line separators
/// (U+2028, U+2029) written as `\u{..}`, so target-supplied text can neither move a terminal's
/// cursor nor start a line of its own in a sentence a person reads (MOD-132 R1b L2).
pub(crate) fn escape_controls(text: &str) -> String {
    use std::fmt::Write as _;

    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
            let _ = write!(escaped, "\\u{{{:x}}}", u32::from(c));
        } else {
            escaped.push(c);
        }
    }
    escaped
}
