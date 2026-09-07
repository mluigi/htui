//! Serving `fs/read_text_file` and intercepting `fs/write_text_file` (`docs/ANA-4.md` §4.3).
//!
//! §4.3 adopted "advertise, intercept": `htui` tells the agent it can write files, and every write
//! is read-then-diffed-then-performed, which is what buys an `edit_proposal` row with a real
//! unified diff instead of a post-hoc tool call with none. The diff synthesis is shared with the
//! `ToolCallContent::Diff` path (§6.1), which has old and new text but never touches the disk.
//!
//! Pure filesystem and text: no channel, no SDK type, no protocol knowledge. The session task
//! calls in, turns the outcome into events, and answers the wire request.

use std::path::{Component, Path, PathBuf};

/// Why a path was refused: it resolves outside `SessionSpec.cwd` and `extra_dirs`.
///
/// Not in ANA-4. An agent asking the *client* to write is asking `htui` to write, and a session
/// whose directories are its declared scope has no business writing outside them (`R-ID-4`'s
/// read-only posture is the neighbouring rule). The refusal is recorded as
/// `error { code: "path_outside_session" }` and answered on the wire as an invalid-params error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{path}` is outside the session directories")]
pub struct PathOutside {
    /// The path as the agent asked for it.
    pub path: String,
}

/// Normalises `path` and admits it only under `cwd` or one of `extra_dirs`.
///
/// Normalisation is **lexical**: a relative path is joined onto `cwd`, `.` is dropped and `..`
/// pops the previous component. Symlinks are not resolved, so a symlink *inside* the session
/// directories pointing outside them is admitted — the guard stops path traversal (`../../etc`),
/// not a filesystem the agent could already reach through its own tools. Resolving links would
/// mean canonicalising a path that does not exist yet, which is the common case for a new file.
///
/// # Errors
///
/// [`PathOutside`] when the normalised path is under none of the session's directories.
pub fn guard(path: &Path, cwd: &Path, extra_dirs: &[PathBuf]) -> Result<PathBuf, PathOutside> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let normalised = normalise(&joined);
    let admitted = std::iter::once(cwd)
        .chain(extra_dirs.iter().map(PathBuf::as_path))
        .any(|root| normalised.starts_with(normalise(root)));
    if admitted {
        Ok(normalised)
    } else {
        Err(PathOutside {
            path: path.to_string_lossy().into_owned(),
        })
    }
}

/// Folds `.` and `..` without touching the filesystem.
///
/// A leading `..` that would escape the root is dropped rather than kept: `/a/../../b` normalises
/// to `/b`, which is what every admitted-prefix comparison below expects.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The current text of `path`, or `""` when it does not exist.
///
/// ANA-4 §4.3: the client MUST create a file it is asked to write, so "absent" is an empty
/// document and not an error — that is what makes the synthesized diff of a new file a run of
/// pure `+` lines.
///
/// # Errors
///
/// Any I/O error other than `NotFound`, including a path that is a directory.
pub async fn read_current(path: &Path) -> std::io::Result<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

/// The unified diff `edit_proposal.diff` carries.
///
/// `similar`'s line diff with three lines of context and `a/<path>` / `b/<path>` headers — the
/// shape `git diff` prints, so the chat tab and any downstream reader need no format of their own.
/// ACP carries old and new *full text* (§3), never a diff, so this is the only place one exists.
#[must_use]
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

/// Writes `content` to `path`, creating parent directories first.
///
/// # Errors
///
/// Any I/O error from creating the parents or writing the file.
pub async fn write_text(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, content).await
}

/// The `line` / `limit` window of `fs/read_text_file`.
///
/// Both are optional and 1-based on the wire. An out-of-range `line` yields an empty string rather
/// than an error: the agent asked for a window that holds nothing, which is a fact about the file,
/// not a failure of the read.
#[must_use]
pub fn slice_lines(text: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return text.to_owned();
    }
    let skip = line.map_or(0, |one_based| one_based.saturating_sub(1) as usize);
    let take = limit.map_or(usize::MAX, |limit| limit as usize);
    let mut out: String = text
        .split_inclusive('\n')
        .skip(skip)
        .take(take)
        .collect::<String>();
    // `split_inclusive` keeps the newline of every line that had one, so a window that ends at the
    // file's last line reproduces it exactly; nothing is appended.
    if out.is_empty() {
        out = String::new();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_diffs_as_a_run_of_plus_lines_that_reproduces_the_content() {
        let content = "one\ntwo\nthree\n";
        let diff = unified_diff("src/new.rs", "", content);
        assert!(diff.contains("--- a/src/new.rs"), "{diff}");
        assert!(diff.contains("+++ b/src/new.rs"), "{diff}");

        // The hunk's `+` lines, stripped of their marker, are the written content: the diff
        // applies to an empty file and yields it (ANA-4 §11 criterion 6).
        let applied: String = diff
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .map(|line| format!("{}\n", &line[1..]))
            .collect();
        assert_eq!(applied, content);
    }

    #[test]
    fn an_edit_diff_carries_both_sides() {
        let diff = unified_diff("a.txt", "one\ntwo\n", "one\ntwo point five\n");
        assert!(diff.contains("-two"), "{diff}");
        assert!(diff.contains("+two point five"), "{diff}");
    }

    #[test]
    fn the_guard_admits_the_cwd_and_the_extra_dirs_and_refuses_everything_else() {
        let cwd = PathBuf::from("/repo/htui");
        let extra = vec![PathBuf::from("/repo/shared")];

        assert_eq!(
            guard(Path::new("src/main.rs"), &cwd, &extra).expect("relative to cwd"),
            PathBuf::from("/repo/htui/src/main.rs")
        );
        assert_eq!(
            guard(Path::new("/repo/shared/notes.md"), &cwd, &extra).expect("an extra dir"),
            PathBuf::from("/repo/shared/notes.md")
        );
        assert_eq!(
            guard(Path::new("/etc/passwd"), &cwd, &extra),
            Err(PathOutside {
                path: "/etc/passwd".to_owned()
            })
        );
    }

    #[test]
    fn the_guard_folds_dot_and_dotdot_before_deciding() {
        let cwd = PathBuf::from("/repo/htui");
        assert_eq!(
            guard(Path::new("./src/./main.rs"), &cwd, &[]).expect("`.` folds away"),
            PathBuf::from("/repo/htui/src/main.rs")
        );
        assert_eq!(
            guard(Path::new("src/../Cargo.toml"), &cwd, &[]).expect("`..` inside stays inside"),
            PathBuf::from("/repo/htui/Cargo.toml")
        );
        assert!(
            guard(Path::new("../../etc/passwd"), &cwd, &[]).is_err(),
            "traversal out of the session directories is refused"
        );
    }

    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_not_admitted() {
        let cwd = PathBuf::from("/repo/htui");
        // `/repo/htui-secrets` starts with the *string* `/repo/htui`, but not with the *path*.
        assert!(guard(Path::new("/repo/htui-secrets/key"), &cwd, &[]).is_err());
    }

    #[tokio::test]
    async fn a_missing_file_reads_as_the_empty_document_and_a_write_creates_its_parents() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested/deeper/new.txt");

        assert_eq!(read_current(&path).await.expect("absent is empty"), "");

        write_text(&path, "hello\n").await.expect("write");
        assert_eq!(read_current(&path).await.expect("read back"), "hello\n");

        write_text(&path, "replaced\n")
            .await
            .expect("truncating write");
        assert_eq!(read_current(&path).await.expect("read back"), "replaced\n");
    }

    #[tokio::test]
    async fn a_refused_path_writes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let outside = dir.path().join("outside.txt");
        let cwd = dir.path().join("session");
        tokio::fs::create_dir_all(&cwd).await.expect("mkdir");

        let refused = guard(Path::new("../outside.txt"), &cwd, &[]);
        assert!(
            refused.is_err(),
            "the guard refuses before anything is written"
        );
        assert!(
            !outside.exists(),
            "nothing on the refused path may be created"
        );
    }

    #[test]
    fn the_line_window_is_one_based_and_clamps() {
        let text = "a\nb\nc\nd\n";
        assert_eq!(slice_lines(text, None, None), text);
        assert_eq!(slice_lines(text, Some(2), None), "b\nc\nd\n");
        assert_eq!(slice_lines(text, Some(2), Some(2)), "b\nc\n");
        assert_eq!(slice_lines(text, None, Some(1)), "a\n");
        assert_eq!(
            slice_lines(text, Some(9), Some(2)),
            "",
            "past the end is empty"
        );
        assert_eq!(
            slice_lines("no trailing newline", Some(1), Some(1)),
            "no trailing newline",
            "the last line is reproduced as it is"
        );
    }
}
