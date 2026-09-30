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
use std::sync::Arc;

use cap_std::fs::Dir;

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

/// Selects a session directory by lexical path. This alone does not authorize I/O:
/// [`SessionFs::guard`] binds the result to a directory handle, and every operation on
/// [`ScopedPath`] resolves beneath that handle through `cap_std`.
///
/// **Every root must be absolute.** A relative root normalises to a path that `starts_with`
/// answers `true` for on *everything* — `"."` normalises to the empty path — so a session whose
/// `cwd` is relative would admit `/etc/passwd`. A caller with no absolute directory has no session
/// scope to enforce, and this refuses rather than pretending to.
///
/// # Errors
///
/// [`PathOutside`] when the normalised path is under none of the session's directories, and when
/// no absolute root was supplied at all.
fn guard(path: &Path, cwd: &Path, extra_dirs: &[PathBuf]) -> Result<PathBuf, PathOutside> {
    let refused = || PathOutside {
        path: path.to_string_lossy().into_owned(),
    };
    let roots: Vec<PathBuf> = std::iter::once(cwd)
        .chain(extra_dirs.iter().map(PathBuf::as_path))
        .filter(|root| root.is_absolute())
        .map(normalise)
        .filter(|root| root.components().next().is_some())
        .collect();
    if roots.is_empty() {
        tracing::warn!("no absolute session directory to admit a path against; refusing");
        return Err(refused());
    }
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let normalised = normalise(&joined);
    let admitted = roots.iter().any(|root| normalised.starts_with(root));
    if admitted {
        Ok(normalised)
    } else {
        Err(refused())
    }
}

/// Directory capabilities pinned once, before the session handshake. Replacing a root with a
/// symlink later cannot redirect a request. Roots that cannot be opened are never authorized.
#[derive(Debug)]
pub struct SessionFs {
    cwd: PathBuf,
    roots: Vec<(PathBuf, Arc<Dir>)>,
}

impl SessionFs {
    /// Opens the declared roots once. Missing, inaccessible and relative roots stay unauthorized.
    ///
    /// # Errors
    /// Returns an I/O error if the blocking task cannot complete.
    pub async fn new(cwd: &Path, extra_dirs: &[PathBuf]) -> std::io::Result<Self> {
        let cwd = cwd.to_owned();
        let extra_dirs = extra_dirs.to_vec();
        crate::contained::spawn_blocking(move || {
            let roots = std::iter::once(&cwd)
                .chain(extra_dirs.iter())
                .filter(|root| root.is_absolute())
                .filter_map(|root| {
                    match Dir::open_ambient_dir(root, cap_std::ambient_authority()) {
                        Ok(dir) => Some((normalise(root), Arc::new(dir))),
                        Err(err) => {
                            tracing::warn!(path = %root.display(), %err, "session directory unavailable; refusing access");
                            None
                        }
                    }
                })
                .collect();
            Self { cwd, roots }
        })
        .await
        .map_err(std::io::Error::other)
    }

    /// Binds a lexical path to a pinned root. I/O still checks symlinks during resolution:
    /// relative links must stay beneath that root; absolute links are refused.
    ///
    /// # Errors
    /// Returns [`PathOutside`] if no available root contains the path.
    pub fn guard(&self, path: &Path) -> Result<ScopedPath, PathOutside> {
        let roots: Vec<_> = self.roots.iter().map(|(path, _)| path.clone()).collect();
        let absolute = guard(path, &self.cwd, &roots)?;
        // Prefer a declared nested root, which may itself be a link to an authorized directory.
        let (root, dir) = self
            .roots
            .iter()
            .filter(|(root, _)| absolute.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .ok_or_else(|| PathOutside {
                path: path.to_string_lossy().into_owned(),
            })?;
        Ok(ScopedPath {
            relative: absolute
                .strip_prefix(root)
                .expect("matched root")
                .to_owned(),
            absolute,
            dir: Arc::clone(dir),
        })
    }
}

/// An admitted name and the only directory capability through which it may be accessed.
/// The absolute name is for display only; it must never be reopened using ambient filesystem I/O.
#[derive(Debug, Clone)]
pub struct ScopedPath {
    absolute: PathBuf,
    relative: PathBuf,
    dir: Arc<Dir>,
}

impl ScopedPath {
    /// The lexical absolute name for edit proposals, never for filesystem access.
    pub fn display_path(&self) -> &Path {
        &self.absolute
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
pub async fn read_current(path: &ScopedPath) -> std::io::Result<String> {
    let path = path.clone();
    crate::contained::spawn_blocking(move || match path.dir.read_to_string(&path.relative) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    })
    .await
    .map_err(std::io::Error::other)?
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
pub async fn write_text(path: &ScopedPath, content: &str) -> std::io::Result<()> {
    let path = path.clone();
    let content = content.to_owned();
    crate::contained::spawn_blocking(move || {
        if let Some(parent) = path.relative.parent()
            && !parent.as_os_str().is_empty()
        {
            path.dir.create_dir_all(parent)?;
        }
        path.dir.write(&path.relative, content)
    })
    .await
    .map_err(std::io::Error::other)?
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
    // `split_inclusive` keeps the newline of every line that had one, so a window ending at the
    // file's last line reproduces it exactly and nothing is appended.
    text.split_inclusive('\n').skip(skip).take(take).collect()
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

    /// A relative root — `"."` is the one production nearly used — cannot scope anything: it
    /// normalises to the empty path, which is a prefix of every path there is.
    #[test]
    fn a_relative_root_admits_nothing_rather_than_everything() {
        assert_eq!(
            guard(Path::new("/etc/passwd"), Path::new("."), &[]),
            Err(PathOutside {
                path: "/etc/passwd".to_owned()
            })
        );
        assert!(guard(Path::new("src/main.rs"), Path::new("repo"), &[]).is_err());
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
        let scope = SessionFs::new(dir.path(), &[]).await.expect("scope");
        let path = scope
            .guard(Path::new("nested/deeper/new.txt"))
            .expect("guard");

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

    #[tokio::test]
    async fn extra_directories_are_scoped_and_unavailable_roots_stay_refused() {
        let cwd = tempfile::tempdir().expect("cwd");
        let extra = tempfile::tempdir().expect("extra");
        let missing = cwd.path().join("missing");
        let scope = SessionFs::new(&missing, &[extra.path().to_owned()])
            .await
            .expect("scope");
        std::fs::create_dir(&missing).expect("late directory");
        assert!(scope.guard(&missing.join("new.txt")).is_err());
        let path = scope
            .guard(&extra.path().join("new.txt"))
            .expect("extra admitted");
        write_text(&path, "shared").await.expect("extra write");
        assert_eq!(read_current(&path).await.expect("extra read"), "shared");
        assert!(scope.guard(&cwd.path().join("outside.txt")).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_explicit_extra_root_can_authorize_a_symlink_beneath_cwd() {
        let cwd = tempfile::tempdir().expect("cwd");
        let extra = tempfile::tempdir().expect("extra");
        let link = cwd.path().join("shared");
        std::os::unix::fs::symlink(extra.path(), &link).expect("link");
        let scope = SessionFs::new(cwd.path(), &[link]).await.expect("scope");
        let path = scope
            .guard(Path::new("shared/new.txt"))
            .expect("explicit root");
        write_text(&path, "shared")
            .await
            .expect("write to declared extra root");
        assert_eq!(read_current(&path).await.expect("read"), "shared");
        assert_eq!(
            std::fs::read_to_string(extra.path().join("new.txt")).unwrap(),
            "shared"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_directory_and_dangling_symlinks_cannot_escape() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().expect("base");
        let cwd = base.path().join("session");
        let outside = base.path().join("outside");
        std::fs::create_dir(&cwd).expect("cwd");
        std::fs::create_dir(&outside).expect("outside");
        std::fs::write(outside.join("secret"), "private").expect("secret");
        symlink(outside.join("secret"), cwd.join("file")).expect("file link");
        symlink("../outside", cwd.join("directory")).expect("directory link");
        symlink(outside.join("new"), cwd.join("dangling")).expect("dangling link");
        let scope = SessionFs::new(&cwd, &[]).await.expect("scope");

        for name in [
            "file",
            "directory/secret",
            "directory/nested/new",
            "dangling",
        ] {
            let path = scope.guard(Path::new(name)).expect("lexically inside");
            assert!(read_current(&path).await.is_err(), "read {name}");
            assert!(
                write_text(&path, "overwritten").await.is_err(),
                "write {name}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(outside.join("secret")).unwrap(),
            "private"
        );
        assert!(!outside.join("new").exists());
        assert!(!outside.join("nested").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn relative_symlinks_that_stay_inside_still_work() {
        use std::os::unix::fs::symlink;

        let cwd = tempfile::tempdir().expect("cwd");
        std::fs::create_dir(cwd.path().join("actual")).expect("directory");
        symlink("actual", cwd.path().join("link")).expect("link");
        let scope = SessionFs::new(cwd.path(), &[]).await.expect("scope");
        let path = scope
            .guard(Path::new("link/nested/new.txt"))
            .expect("guard");
        write_text(&path, "inside")
            .await
            .expect("write through internal link");
        assert_eq!(read_current(&path).await.expect("read"), "inside");
        assert_eq!(
            std::fs::read_to_string(cwd.path().join("actual/nested/new.txt")).unwrap(),
            "inside"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_a_validated_file_or_parent_cannot_redirect_io() {
        use std::os::unix::fs::symlink;

        let cwd = tempfile::tempdir().expect("cwd");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret"), "private").unwrap();
        std::fs::write(cwd.path().join("file"), "inside").unwrap();
        std::fs::create_dir(cwd.path().join("parent")).unwrap();
        let scope = SessionFs::new(cwd.path(), &[]).await.expect("scope");
        let file = scope.guard(Path::new("file")).expect("file guard");
        let new = scope.guard(Path::new("parent/new")).expect("parent guard");
        assert_eq!(read_current(&file).await.unwrap(), "inside");

        // Swap after admission and after the old-text read used to build an edit proposal.
        std::fs::remove_file(cwd.path().join("file")).unwrap();
        symlink(outside.path().join("secret"), cwd.path().join("file")).unwrap();
        std::fs::remove_dir(cwd.path().join("parent")).unwrap();
        symlink(outside.path(), cwd.path().join("parent")).unwrap();
        assert!(read_current(&file).await.is_err());
        assert!(write_text(&file, "changed").await.is_err());
        assert!(write_text(&new, "created").await.is_err());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "private"
        );
        assert!(!outside.path().join("new").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replacing_the_session_root_does_not_replace_its_capability() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().expect("base");
        let cwd = base.path().join("session");
        let moved = base.path().join("moved");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::write(cwd.join("file"), "inside").unwrap();
        std::fs::write(outside.path().join("file"), "private").unwrap();
        let scope = SessionFs::new(&cwd, &[]).await.expect("scope");
        std::fs::rename(&cwd, &moved).unwrap();
        symlink(outside.path(), &cwd).unwrap();
        let path = scope
            .guard(Path::new("file"))
            .expect("guard after replacement");
        assert_eq!(read_current(&path).await.unwrap(), "inside");
        write_text(&path, "changed").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(moved.join("file")).unwrap(),
            "changed"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("file")).unwrap(),
            "private"
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
